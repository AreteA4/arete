//! View abstractions for unified access to views.
//!
//! All views return collections (Vec<T>). Use `.first()` on the result
//! if you need a single item.
//!
//! # Example
//!
//! ```ignore
//! use arete_sdk::prelude::*;
//! use my_stack::OreRoundViews;
//!
//! let a4 = Arete::connect("wss://example.com").await?;
//!
//! // Access views through the generated views struct
//! let views = OreRoundViews::new(&a4);
//!
//! // Get latest round - use .first() for single item
//! let latest = views.latest().get().await.first().cloned();
//!
//! // List all rounds
//! let rounds = views.list().get().await;
//!
//! // One-shot read with query options (TypeScript `list.get({ filters })`),
//! // failing as TypeScript rejects when the initial snapshot never arrives
//! let open = views
//!     .list()
//!     .get_with(GetOptions::new().filter("state.status", "open").take(10))
//!     .await?;
//!
//! // Get specific round by key
//! let round = views.state().get("round_key").await;
//!
//! // Watch for updates
//! let mut stream = views.latest().watch();
//! while let Some(update) = stream.next().await {
//!     println!("Latest round updated: {:?}", update);
//! }
//! ```

use crate::connection::{ConnectionManager, SubscriptionLease, SubscriptionOptions};
use crate::error::AreteError;
use crate::store::SharedStore;
use crate::stream::{EntityStream, KeyFilter, RichEntityStream, Update, UseStream};
use futures_util::Stream;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;
use thiserror::Error;

/// Query options for the one-shot reads [`ViewHandle::get_with`] and
/// [`StateView::get_with`]: the Rust form of TypeScript's `GetOptions`
/// (`list.get(options)`, `state.get(key, options)`).
///
/// Every field except `timeout` is sent on the read's subscription exactly as
/// the stream builders (`listen`, `watch`, `watch_rich`) send it: `filters`,
/// `take`, `skip`, `partition`, `after` and `snapshot_limit` in the protocol v2
/// query, `with_snapshot` as the subscription's `snapshot.enabled` (default
/// `true`). An empty `filters` map sends no filters. `timeout` bounds the wait
/// for the initial snapshot instead of the client's `initial_data_timeout`
/// (TypeScript `timeoutMs`). The default sends what [`ViewHandle::get`] sends.
///
/// ```ignore
/// let rows = a4
///     .views
///     .lookup_table
///     .list()
///     .get_with(GetOptions::new().filter("state.authority", authority))
///     .await?;
/// ```
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GetOptions {
    pub partition: Option<String>,
    pub filters: BTreeMap<String, Value>,
    pub take: Option<usize>,
    pub skip: Option<usize>,
    pub with_snapshot: Option<bool>,
    pub after: Option<String>,
    pub snapshot_limit: Option<usize>,
    pub timeout: Option<Duration>,
}

impl GetOptions {
    /// No options: the query [`ViewHandle::get`] sends.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a server-side filter (`filters[path] = value`).
    pub fn filter(mut self, path: impl Into<String>, value: impl Into<Value>) -> Self {
        self.filters.insert(path.into(), value.into());
        self
    }

    /// Limit the read to the top N items.
    pub fn take(mut self, n: usize) -> Self {
        self.take = Some(n);
        self
    }

    /// Skip the first N items.
    pub fn skip(mut self, n: usize) -> Self {
        self.skip = Some(n);
        self
    }

    pub fn partition(mut self, partition: impl Into<String>) -> Self {
        self.partition = Some(partition.into());
        self
    }

    /// Set whether to include the initial snapshot (defaults to true).
    pub fn with_snapshot(mut self, with_snapshot: bool) -> Self {
        self.with_snapshot = Some(with_snapshot);
        self
    }

    /// Resume after this `{epoch}:{offset}` cursor, exclusive.
    pub fn after(mut self, cursor: impl Into<String>) -> Self {
        self.after = Some(cursor.into());
        self
    }

    /// Set the maximum number of entities to include in the snapshot.
    pub fn with_snapshot_limit(mut self, limit: usize) -> Self {
        self.snapshot_limit = Some(limit);
        self
    }

    /// Wait at most `timeout` for the initial snapshot (instead of the
    /// client's `initial_data_timeout`).
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// The subscription options and the snapshot wait of this read.
    fn into_parts(self, default_timeout: Duration) -> (SubscriptionOptions, Duration) {
        let timeout = self.timeout.unwrap_or(default_timeout);
        (
            SubscriptionOptions {
                partition: self.partition,
                filters: self.filters,
                take: self.take,
                skip: self.skip,
                with_snapshot: self.with_snapshot,
                after: self.after,
                snapshot_limit: self.snapshot_limit,
            },
            timeout,
        )
    }
}

/// Why a one-shot read with [`ViewHandle::get_with`] or
/// [`StateView::get_with`] has no result: what TypeScript's
/// `list.get(options)` and `state.get(key, options)` reject with. The read's
/// subscription is released either way.
///
/// [`ViewHandle::get`], [`ViewHandle::get_one`] and [`StateView::get`] never
/// fail: they return what the read holds (no rows, or `None`) instead.
#[derive(Debug, Clone, Error)]
#[non_exhaustive]
pub enum ViewError {
    /// The read could not subscribe to the view: the client was connected
    /// with `Transport::Http`, its connection is closed, or the query is
    /// invalid. `source` is the subscription's error, as TypeScript rejects
    /// with the connection or query error.
    #[error("Could not subscribe to view '{view}': {source}")]
    Subscription {
        view: String,
        #[source]
        source: AreteError,
    },

    /// The view's initial snapshot did not arrive within the read's timeout
    /// ([`GetOptions::timeout`], by default the client's
    /// `initial_data_timeout`): TypeScript's `InitialDataTimeoutError`, code
    /// `INITIAL_DATA_TIMEOUT`, with the same message.
    #[error(
        "Timed out after {}ms waiting for the initial snapshot of view '{view}'",
        .timeout.as_millis()
    )]
    InitialDataTimeout { view: String, timeout: Duration },
}

/// A failed view read as the [`AreteError`] an extension's read returns, so
/// a bundle propagates it with `?` as the TypeScript read's rejection
/// propagates: a [`ViewError::Subscription`] is its `source`, and a
/// [`ViewError::InitialDataTimeout`] an [`AreteError::ConnectionFailed`]
/// carrying the TypeScript message.
impl From<ViewError> for AreteError {
    fn from(error: ViewError) -> Self {
        match error {
            ViewError::Subscription { source, .. } => source,
            timeout @ ViewError::InitialDataTimeout { .. } => {
                AreteError::ConnectionFailed(timeout.to_string())
            }
        }
    }
}

/// Subscribe to `view_path` (at `key`) with `options` and wait for the initial
/// snapshot. Returns the read's lease, which releases the subscription when
/// dropped, and the timeout when the snapshot did not arrive within it.
async fn subscribe_for_read(
    connection: &ConnectionManager,
    store: &SharedStore,
    view_path: &str,
    key: Option<&str>,
    options: GetOptions,
    default_timeout: Duration,
) -> Result<(SubscriptionLease, Option<Duration>), AreteError> {
    let (subscription, timeout) = options.into_parts(default_timeout);
    let lease = connection
        .ensure_subscription_with_opts(view_path, key, subscription)
        .await?;
    let ready = store
        .wait_for_subscription_ready(lease.subscription_id(), timeout)
        .await;
    Ok((lease, (!ready).then_some(timeout)))
}

/// A handle to a view that provides get/watch operations.
///
/// All views return collections (Vec<T>). Use `.first()` on the result
/// if you need a single item from views with a `take` limit.
pub struct ViewHandle<T> {
    connection: ConnectionManager,
    store: SharedStore,
    view_path: String,
    initial_data_timeout: Duration,
    _marker: PhantomData<T>,
}

impl<T> ViewHandle<T>
where
    T: Serialize + DeserializeOwned + Clone + Send + Sync + 'static,
{
    /// Get all items from this view.
    ///
    /// For views with a `take` limit defined in the stack, this returns
    /// up to that many items. Use `.first()` on the result if you need
    /// a single item.
    ///
    /// Never fails: when the read cannot subscribe it returns no rows, and
    /// when the initial snapshot does not arrive within the client's
    /// `initial_data_timeout` it returns the rows the read holds by then.
    /// [`ViewHandle::get_with`] reports both as a [`ViewError`] instead, as
    /// TypeScript's `list.get()` rejects.
    pub async fn get(&self) -> Vec<T> {
        let Ok((lease, _)) = self.subscribe(GetOptions::default()).await else {
            return Vec::new();
        };
        self.store
            .list_for_subscription::<T>(lease.subscription_id())
            .await
    }

    /// Get the items of this view that match `options` (TypeScript
    /// `list.get(options)`): subscribes with the options' query, waits for
    /// its initial snapshot and releases the subscription. Returns the rows
    /// the host sent for that query, in the view's order.
    ///
    /// Fails as TypeScript's read rejects: with
    /// [`ViewError::InitialDataTimeout`] when the snapshot does not arrive
    /// within [`GetOptions::timeout`] (by default the client's
    /// `initial_data_timeout`), and with [`ViewError::Subscription`] when the
    /// read cannot subscribe.
    ///
    /// ```ignore
    /// let pools = a4
    ///     .views
    ///     .pool
    ///     .list()
    ///     .get_with(
    ///         GetOptions::new()
    ///             .filter("tokens.base_mint", base_mint)
    ///             .filter("tokens.quote_mint", quote_mint),
    ///     )
    ///     .await?;
    /// ```
    pub async fn get_with(&self, options: GetOptions) -> Result<Vec<T>, ViewError> {
        let (lease, timed_out) =
            self.subscribe(options)
                .await
                .map_err(|source| ViewError::Subscription {
                    view: self.view_path.clone(),
                    source,
                })?;
        if let Some(timeout) = timed_out {
            return Err(ViewError::InitialDataTimeout {
                view: self.view_path.clone(),
                timeout,
            });
        }
        Ok(self
            .store
            .list_for_subscription::<T>(lease.subscription_id())
            .await)
    }

    async fn subscribe(
        &self,
        options: GetOptions,
    ) -> Result<(SubscriptionLease, Option<Duration>), AreteError> {
        subscribe_for_read(
            &self.connection,
            &self.store,
            &self.view_path,
            None,
            options,
            self.initial_data_timeout,
        )
        .await
    }

    /// Synchronously get all items from cached data.
    ///
    /// Returns cached data immediately without waiting for subscription.
    /// Returns empty vector if data not yet loaded or lock unavailable.
    pub fn get_sync(&self) -> Vec<T> {
        self.store
            .list_for_query_sync::<T>(&crate::SubscriptionQuery::new(&self.view_path))
    }

    /// Get the first item from this view, mirroring the TypeScript `useOne`
    /// convenience for single-row derived views like `latest`. Never fails,
    /// like [`ViewHandle::get`].
    pub async fn get_one(&self) -> Option<T> {
        self.get().await.into_iter().next()
    }

    /// Stream merged entities directly (simplest API - filters out removals and deletes).
    ///
    /// Emits `T` after each change. Patches are merged to give full entity state.
    /// Deletes are filtered out. Use `.watch()` if you need delete notifications.
    pub fn listen(&self) -> UseBuilder<T>
    where
        T: Unpin,
    {
        UseBuilder::new(
            self.connection.clone(),
            self.store.clone(),
            self.view_path.clone(),
            KeyFilter::None,
        )
    }

    /// Watch for updates to this view. Chain `.take(n)` to limit results.
    pub fn watch(&self) -> WatchBuilder<T>
    where
        T: Unpin,
    {
        WatchBuilder::new(
            self.connection.clone(),
            self.store.clone(),
            self.view_path.clone(),
            KeyFilter::None,
        )
    }

    /// Watch for updates with before/after diffs.
    pub fn watch_rich(&self) -> RichWatchBuilder<T>
    where
        T: Unpin,
    {
        RichWatchBuilder::new(
            self.connection.clone(),
            self.store.clone(),
            self.view_path.clone(),
            KeyFilter::None,
        )
    }

    /// Watch for updates filtered to specific keys.
    pub fn watch_keys(&self, keys: &[&str]) -> WatchBuilder<T>
    where
        T: Unpin,
    {
        WatchBuilder::new(
            self.connection.clone(),
            self.store.clone(),
            self.view_path.clone(),
            KeyFilter::Multiple(keys.iter().map(|s| s.to_string()).collect()),
        )
    }

    /// Stream merged entities filtered to specific keys (deletes filtered out).
    pub fn listen_keys(&self, keys: &[&str]) -> UseBuilder<T>
    where
        T: Unpin,
    {
        UseBuilder::new(
            self.connection.clone(),
            self.store.clone(),
            self.view_path.clone(),
            KeyFilter::Multiple(keys.iter().map(|s| s.to_string()).collect()),
        )
    }
}

/// Builder for `.use()` subscriptions that emit `T` directly. Implements `Stream`.
pub struct UseBuilder<T>
where
    T: Serialize + DeserializeOwned + Clone + Send + Sync + Unpin + 'static,
{
    connection: ConnectionManager,
    store: SharedStore,
    view_path: String,
    key_filter: KeyFilter,
    key: Option<String>,
    partition: Option<String>,
    take: Option<usize>,
    skip: Option<usize>,
    filters: BTreeMap<String, Value>,
    with_snapshot: Option<bool>,
    after: Option<String>,
    snapshot_limit: Option<usize>,
    stream: Option<UseStream<T>>,
}

impl<T> UseBuilder<T>
where
    T: Serialize + DeserializeOwned + Clone + Send + Sync + Unpin + 'static,
{
    fn new(
        connection: ConnectionManager,
        store: SharedStore,
        view_path: String,
        key_filter: KeyFilter,
    ) -> Self {
        Self::new_keyed(connection, store, view_path, key_filter, None)
    }

    fn new_keyed(
        connection: ConnectionManager,
        store: SharedStore,
        view_path: String,
        key_filter: KeyFilter,
        key: Option<String>,
    ) -> Self {
        Self {
            connection,
            store,
            view_path,
            key_filter,
            key,
            partition: None,
            take: None,
            skip: None,
            filters: BTreeMap::new(),
            with_snapshot: None,
            after: None,
            snapshot_limit: None,
            stream: None,
        }
    }

    /// Limit subscription to the top N items.
    pub fn take(mut self, n: usize) -> Self {
        self.take = Some(n);
        self
    }

    /// Skip the first N items.
    pub fn skip(mut self, n: usize) -> Self {
        self.skip = Some(n);
        self
    }

    /// Add a server-side filter.
    pub fn filter(mut self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        self.filters.insert(key.into(), value.into());
        self
    }

    pub fn partition(mut self, partition: impl Into<String>) -> Self {
        self.partition = Some(partition.into());
        self
    }

    /// Set whether to include the initial snapshot (defaults to true).
    pub fn with_snapshot(mut self, with_snapshot: bool) -> Self {
        self.with_snapshot = Some(with_snapshot);
        self
    }

    /// Set the cursor to resume from (for reconnecting and getting only newer data).
    pub fn after(mut self, cursor: impl Into<String>) -> Self {
        self.after = Some(cursor.into());
        self
    }

    /// Set the maximum number of entities to include in the snapshot.
    pub fn with_snapshot_limit(mut self, limit: usize) -> Self {
        self.snapshot_limit = Some(limit);
        self
    }
}

impl<T> Stream for UseBuilder<T>
where
    T: Serialize + DeserializeOwned + Clone + Send + Sync + Unpin + 'static,
{
    type Item = T;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        if this.stream.is_none() {
            this.stream = Some(UseStream::new_lazy_with_opts(
                this.connection.clone(),
                this.store.clone(),
                this.view_path.clone(),
                this.key_filter.clone(),
                this.key.clone(),
                this.partition.clone(),
                this.filters.clone(),
                this.take,
                this.skip,
                this.with_snapshot,
                this.after.clone(),
                this.snapshot_limit,
            ));
        }

        Pin::new(this.stream.as_mut().unwrap()).poll_next(cx)
    }
}

/// Builder for configuring watch subscriptions. Implements `Stream` directly.
pub struct WatchBuilder<T>
where
    T: Serialize + DeserializeOwned + Clone + Send + Sync + Unpin + 'static,
{
    connection: ConnectionManager,
    store: SharedStore,
    view_path: String,
    key_filter: KeyFilter,
    key: Option<String>,
    partition: Option<String>,
    take: Option<usize>,
    skip: Option<usize>,
    filters: BTreeMap<String, Value>,
    with_snapshot: Option<bool>,
    after: Option<String>,
    snapshot_limit: Option<usize>,
    stream: Option<EntityStream<T>>,
}

impl<T> WatchBuilder<T>
where
    T: Serialize + DeserializeOwned + Clone + Send + Sync + Unpin + 'static,
{
    fn new(
        connection: ConnectionManager,
        store: SharedStore,
        view_path: String,
        key_filter: KeyFilter,
    ) -> Self {
        Self::new_keyed(connection, store, view_path, key_filter, None)
    }

    fn new_keyed(
        connection: ConnectionManager,
        store: SharedStore,
        view_path: String,
        key_filter: KeyFilter,
        key: Option<String>,
    ) -> Self {
        Self {
            connection,
            store,
            view_path,
            key_filter,
            key,
            partition: None,
            take: None,
            skip: None,
            filters: BTreeMap::new(),
            with_snapshot: None,
            after: None,
            snapshot_limit: None,
            stream: None,
        }
    }

    /// Limit subscription to the top N items.
    pub fn take(mut self, n: usize) -> Self {
        self.take = Some(n);
        self
    }

    /// Skip the first N items.
    pub fn skip(mut self, n: usize) -> Self {
        self.skip = Some(n);
        self
    }

    /// Add a server-side filter.
    pub fn filter(mut self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        self.filters.insert(key.into(), value.into());
        self
    }

    pub fn partition(mut self, partition: impl Into<String>) -> Self {
        self.partition = Some(partition.into());
        self
    }

    /// Set whether to include the initial snapshot (defaults to true).
    pub fn with_snapshot(mut self, with_snapshot: bool) -> Self {
        self.with_snapshot = Some(with_snapshot);
        self
    }

    /// Set the cursor to resume from (for reconnecting and getting only newer data).
    pub fn after(mut self, cursor: impl Into<String>) -> Self {
        self.after = Some(cursor.into());
        self
    }

    /// Set the maximum number of entities to include in the snapshot.
    pub fn with_snapshot_limit(mut self, limit: usize) -> Self {
        self.snapshot_limit = Some(limit);
        self
    }

    /// Get a rich stream with before/after diffs instead.
    pub fn rich(self) -> RichEntityStream<T> {
        RichEntityStream::new_lazy_with_opts(
            self.connection,
            self.store,
            self.view_path,
            self.key_filter,
            self.key,
            self.partition,
            self.filters,
            self.take,
            self.skip,
            self.with_snapshot,
            self.after,
            self.snapshot_limit,
        )
    }
}

impl<T> Stream for WatchBuilder<T>
where
    T: Serialize + DeserializeOwned + Clone + Send + Sync + Unpin + 'static,
{
    type Item = Update<T>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        if this.stream.is_none() {
            this.stream = Some(EntityStream::new_lazy_with_opts(
                this.connection.clone(),
                this.store.clone(),
                this.view_path.clone(),
                this.key_filter.clone(),
                this.key.clone(),
                this.partition.clone(),
                this.filters.clone(),
                this.take,
                this.skip,
                this.with_snapshot,
                this.after.clone(),
                this.snapshot_limit,
            ));
        }

        Pin::new(this.stream.as_mut().unwrap()).poll_next(cx)
    }
}

/// Builder for rich watch subscriptions with before/after diffs.
pub struct RichWatchBuilder<T>
where
    T: Serialize + DeserializeOwned + Clone + Send + Sync + Unpin + 'static,
{
    connection: ConnectionManager,
    store: SharedStore,
    view_path: String,
    key_filter: KeyFilter,
    key: Option<String>,
    partition: Option<String>,
    take: Option<usize>,
    skip: Option<usize>,
    filters: BTreeMap<String, Value>,
    with_snapshot: Option<bool>,
    after: Option<String>,
    snapshot_limit: Option<usize>,
    stream: Option<RichEntityStream<T>>,
}

impl<T> RichWatchBuilder<T>
where
    T: Serialize + DeserializeOwned + Clone + Send + Sync + Unpin + 'static,
{
    fn new(
        connection: ConnectionManager,
        store: SharedStore,
        view_path: String,
        key_filter: KeyFilter,
    ) -> Self {
        Self::new_keyed(connection, store, view_path, key_filter, None)
    }

    fn new_keyed(
        connection: ConnectionManager,
        store: SharedStore,
        view_path: String,
        key_filter: KeyFilter,
        key: Option<String>,
    ) -> Self {
        Self {
            connection,
            store,
            view_path,
            key_filter,
            key,
            partition: None,
            take: None,
            skip: None,
            filters: BTreeMap::new(),
            with_snapshot: None,
            after: None,
            snapshot_limit: None,
            stream: None,
        }
    }

    pub fn take(mut self, n: usize) -> Self {
        self.take = Some(n);
        self
    }

    pub fn skip(mut self, n: usize) -> Self {
        self.skip = Some(n);
        self
    }

    pub fn filter(mut self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        self.filters.insert(key.into(), value.into());
        self
    }

    pub fn partition(mut self, partition: impl Into<String>) -> Self {
        self.partition = Some(partition.into());
        self
    }

    /// Set whether to include the initial snapshot (defaults to true).
    pub fn with_snapshot(mut self, with_snapshot: bool) -> Self {
        self.with_snapshot = Some(with_snapshot);
        self
    }

    /// Set the cursor to resume from (for reconnecting and getting only newer data).
    pub fn after(mut self, cursor: impl Into<String>) -> Self {
        self.after = Some(cursor.into());
        self
    }

    /// Set the maximum number of entities to include in the snapshot.
    pub fn with_snapshot_limit(mut self, limit: usize) -> Self {
        self.snapshot_limit = Some(limit);
        self
    }
}

impl<T> Stream for RichWatchBuilder<T>
where
    T: Serialize + DeserializeOwned + Clone + Send + Sync + Unpin + 'static,
{
    type Item = crate::stream::RichUpdate<T>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        if this.stream.is_none() {
            this.stream = Some(RichEntityStream::new_lazy_with_opts(
                this.connection.clone(),
                this.store.clone(),
                this.view_path.clone(),
                this.key_filter.clone(),
                this.key.clone(),
                this.partition.clone(),
                this.filters.clone(),
                this.take,
                this.skip,
                this.with_snapshot,
                this.after.clone(),
                this.snapshot_limit,
            ));
        }

        Pin::new(this.stream.as_mut().unwrap()).poll_next(cx)
    }
}

/// Builder for creating view handles.
///
/// This is used internally by generated code to create properly configured view handles.
#[derive(Clone)]
pub struct ViewBuilder {
    connection: ConnectionManager,
    store: SharedStore,
    initial_data_timeout: Duration,
}

impl ViewBuilder {
    pub fn new(
        connection: ConnectionManager,
        store: SharedStore,
        initial_data_timeout: Duration,
    ) -> Self {
        Self {
            connection,
            store,
            initial_data_timeout,
        }
    }

    pub fn connection(&self) -> &ConnectionManager {
        &self.connection
    }

    pub fn store(&self) -> &SharedStore {
        &self.store
    }

    pub fn initial_data_timeout(&self) -> Duration {
        self.initial_data_timeout
    }

    /// Create a view handle.
    pub fn view<T>(&self, view_path: &str) -> ViewHandle<T>
    where
        T: Serialize + DeserializeOwned + Clone + Send + Sync + 'static,
    {
        ViewHandle {
            connection: self.connection.clone(),
            store: self.store.clone(),
            view_path: view_path.to_string(),
            initial_data_timeout: self.initial_data_timeout,
            _marker: PhantomData,
        }
    }
}

/// Trait for generated view accessor structs.
pub trait Views: Sized + Send + Sync + 'static {
    fn from_builder(builder: ViewBuilder) -> Self;
}

/// Standalone program SDKs have no live views.
impl Views for () {
    fn from_builder(_builder: ViewBuilder) -> Self {}
}

/// A state view handle that requires a key for access.
pub struct StateView<T> {
    connection: ConnectionManager,
    store: SharedStore,
    view_path: String,
    initial_data_timeout: Duration,
    _marker: PhantomData<T>,
}

impl<T> StateView<T>
where
    T: Serialize + DeserializeOwned + Clone + Send + Sync + 'static,
{
    pub fn new(
        connection: ConnectionManager,
        store: SharedStore,
        view_path: String,
        initial_data_timeout: Duration,
    ) -> Self {
        Self {
            connection,
            store,
            view_path,
            initial_data_timeout,
            _marker: PhantomData,
        }
    }

    /// Get an entity by key.
    ///
    /// Never fails: `None` also when the read cannot subscribe, and when the
    /// initial snapshot does not arrive within the client's
    /// `initial_data_timeout` it returns what the read holds by then.
    /// [`StateView::get_with`] reports both as a [`ViewError`] instead, as
    /// TypeScript's `state.get(key)` rejects.
    pub async fn get(&self, key: &str) -> Option<T> {
        let (lease, _) = self.subscribe(key, GetOptions::default()).await.ok()?;
        self.store
            .get_for_subscription::<T>(lease.subscription_id(), key)
            .await
    }

    /// Get an entity by key with query options (TypeScript
    /// `state.get(key, options)`); see [`GetOptions`]. `Ok(None)` when the
    /// view holds no entity at `key`; fails like [`ViewHandle::get_with`].
    pub async fn get_with(&self, key: &str, options: GetOptions) -> Result<Option<T>, ViewError> {
        let (lease, timed_out) =
            self.subscribe(key, options)
                .await
                .map_err(|source| ViewError::Subscription {
                    view: self.view_path.clone(),
                    source,
                })?;
        if let Some(timeout) = timed_out {
            return Err(ViewError::InitialDataTimeout {
                view: self.view_path.clone(),
                timeout,
            });
        }
        Ok(self
            .store
            .get_for_subscription::<T>(lease.subscription_id(), key)
            .await)
    }

    async fn subscribe(
        &self,
        key: &str,
        options: GetOptions,
    ) -> Result<(SubscriptionLease, Option<Duration>), AreteError> {
        subscribe_for_read(
            &self.connection,
            &self.store,
            &self.view_path,
            Some(key),
            options,
            self.initial_data_timeout,
        )
        .await
    }

    /// Synchronously get an entity from cached data.
    pub fn get_sync(&self, key: &str) -> Option<T> {
        self.store.get_for_query_sync::<T>(
            &crate::SubscriptionQuery::new(&self.view_path).with_key(key),
            key,
        )
    }

    /// Stream merged entity values directly (simplest API - filters out removals and deletes).
    ///
    /// Returns a builder, so keyed subscriptions accept the same query options
    /// as list views: `.with_snapshot(false)`, `.after(cursor)`, `.partition(..)`, …
    pub fn listen(&self, key: &str) -> UseBuilder<T>
    where
        T: Unpin,
    {
        UseBuilder::new_keyed(
            self.connection.clone(),
            self.store.clone(),
            self.view_path.clone(),
            KeyFilter::Single(key.to_string()),
            Some(key.to_string()),
        )
    }

    /// Watch for updates to a specific key. Chain query options before polling.
    pub fn watch(&self, key: &str) -> WatchBuilder<T>
    where
        T: Unpin,
    {
        WatchBuilder::new_keyed(
            self.connection.clone(),
            self.store.clone(),
            self.view_path.clone(),
            KeyFilter::Single(key.to_string()),
            Some(key.to_string()),
        )
    }

    /// Watch for updates with before/after diffs. Chain query options before polling.
    pub fn watch_rich(&self, key: &str) -> RichWatchBuilder<T>
    where
        T: Unpin,
    {
        RichWatchBuilder::new_keyed(
            self.connection.clone(),
            self.store.clone(),
            self.view_path.clone(),
            KeyFilter::Single(key.to_string()),
            Some(key.to_string()),
        )
    }
}
