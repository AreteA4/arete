//! View abstractions for unified access to views.
//!
//! All views return collections (Vec<T>). Use `.first()` on the result
//! if you need a single item.
//!
//! # Example
//!
//! ```ignore
//! use hyperstack_sdk::prelude::*;
//! use my_stack::OreRoundViews;
//!
//! let hs = HyperStack::connect("wss://example.com").await?;
//!
//! // Access views through the generated views struct
//! let views = OreRoundViews::new(&hs);
//!
//! // Get latest round - use .first() for single item
//! let latest = views.latest().get().await.first().cloned();
//!
//! // List all rounds
//! let rounds = views.list().get().await;
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

use crate::connection::ConnectionManager;
use crate::store::SharedStore;
use crate::stream::{EntityStream, KeyFilter, RichEntityStream, Update, UseStream};
use futures_util::Stream;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

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
    pub async fn get(&self) -> Vec<T> {
        self.connection
            .ensure_subscription(&self.view_path, None)
            .await;
        self.store
            .wait_for_view_ready(&self.view_path, self.initial_data_timeout)
            .await;
        self.store.list::<T>(&self.view_path).await
    }

    /// Synchronously get all items from cached data.
    ///
    /// Returns cached data immediately without waiting for subscription.
    /// Returns empty vector if data not yet loaded or lock unavailable.
    pub fn get_sync(&self) -> Vec<T> {
        self.store.list_sync::<T>(&self.view_path)
    }

    /// Stream merged entities directly (simplest API - filters out deletes).
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
    take: Option<u32>,
    skip: Option<u32>,
    filters: Option<HashMap<String, String>>,
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
        Self {
            connection,
            store,
            view_path,
            key_filter,
            take: None,
            skip: None,
            filters: None,
            with_snapshot: None,
            after: None,
            snapshot_limit: None,
            stream: None,
        }
    }

    /// Limit subscription to the top N items.
    pub fn take(mut self, n: u32) -> Self {
        self.take = Some(n);
        self
    }

    /// Skip the first N items.
    pub fn skip(mut self, n: u32) -> Self {
        self.skip = Some(n);
        self
    }

    /// Add a server-side filter.
    pub fn filter(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.filters
            .get_or_insert_with(HashMap::new)
            .insert(key.into(), value.into());
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
                this.view_path.clone(),
                this.key_filter.clone(),
                None,
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
    take: Option<u32>,
    skip: Option<u32>,
    filters: Option<HashMap<String, String>>,
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
        Self {
            connection,
            store,
            view_path,
            key_filter,
            take: None,
            skip: None,
            filters: None,
            with_snapshot: None,
            after: None,
            snapshot_limit: None,
            stream: None,
        }
    }

    /// Limit subscription to the top N items.
    pub fn take(mut self, n: u32) -> Self {
        self.take = Some(n);
        self
    }

    /// Skip the first N items.
    pub fn skip(mut self, n: u32) -> Self {
        self.skip = Some(n);
        self
    }

    /// Add a server-side filter.
    pub fn filter(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.filters
            .get_or_insert_with(HashMap::new)
            .insert(key.into(), value.into());
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
            self.view_path.clone(),
            self.view_path,
            self.key_filter,
            None,
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
                this.view_path.clone(),
                this.key_filter.clone(),
                None,
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
    take: Option<u32>,
    skip: Option<u32>,
    filters: Option<HashMap<String, String>>,
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
        Self {
            connection,
            store,
            view_path,
            key_filter,
            take: None,
            skip: None,
            filters: None,
            with_snapshot: None,
            after: None,
            snapshot_limit: None,
            stream: None,
        }
    }

    pub fn take(mut self, n: u32) -> Self {
        self.take = Some(n);
        self
    }

    pub fn skip(mut self, n: u32) -> Self {
        self.skip = Some(n);
        self
    }

    pub fn filter(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.filters
            .get_or_insert_with(HashMap::new)
            .insert(key.into(), value.into());
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
                this.view_path.clone(),
                this.key_filter.clone(),
                None,
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
    pub async fn get(&self, key: &str) -> Option<T> {
        self.connection
            .ensure_subscription(&self.view_path, Some(key))
            .await;
        self.store
            .wait_for_view_ready(&self.view_path, self.initial_data_timeout)
            .await;
        self.store.get::<T>(&self.view_path, key).await
    }

    /// Synchronously get an entity from cached data.
    pub fn get_sync(&self, key: &str) -> Option<T> {
        self.store.get_sync::<T>(&self.view_path, key)
    }

    /// Stream merged entity values directly (simplest API - filters out deletes).
    pub fn listen(&self, key: &str) -> UseStream<T>
    where
        T: Unpin,
    {
        UseStream::new_lazy(
            self.connection.clone(),
            self.store.clone(),
            self.view_path.clone(),
            self.view_path.clone(),
            KeyFilter::Single(key.to_string()),
            Some(key.to_string()),
        )
    }

    /// Watch for updates to a specific key.
    pub fn watch(&self, key: &str) -> EntityStream<T> {
        EntityStream::new_lazy(
            self.connection.clone(),
            self.store.clone(),
            self.view_path.clone(),
            self.view_path.clone(),
            KeyFilter::Single(key.to_string()),
            Some(key.to_string()),
        )
    }

    /// Watch for updates with before/after diffs.
    pub fn watch_rich(&self, key: &str) -> RichEntityStream<T> {
        RichEntityStream::new_lazy(
            self.connection.clone(),
            self.store.clone(),
            self.view_path.clone(),
            self.view_path.clone(),
            KeyFilter::Single(key.to_string()),
            Some(key.to_string()),
        )
    }
}
