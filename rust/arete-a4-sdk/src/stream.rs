use crate::connection::{ConnectionManager, SubscriptionLease};
use crate::frame::Operation;
use crate::store::{SharedStore, StoreUpdate};
use crate::subscription::{SnapshotOptions, SubscriptionQuery};
use futures_util::Stream;
use pin_project_lite::pin_project;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;

#[derive(Debug, Clone)]
pub enum Update<T> {
    Upsert { key: String, data: T },
    Patch { key: String, data: T },
    Remove { key: String },
    Delete { key: String },
}

#[derive(Debug, Clone)]
pub enum RichUpdate<T> {
    Created {
        key: String,
        data: T,
    },
    Updated {
        key: String,
        before: T,
        after: T,
        patch: Option<Value>,
    },
    Removed {
        key: String,
        last_known: Option<T>,
    },
    Deleted {
        key: String,
        last_known: Option<T>,
    },
}

impl<T> Update<T> {
    pub fn key(&self) -> &str {
        match self {
            Self::Upsert { key, .. }
            | Self::Patch { key, .. }
            | Self::Remove { key }
            | Self::Delete { key } => key,
        }
    }

    pub fn data(&self) -> Option<&T> {
        match self {
            Self::Upsert { data, .. } | Self::Patch { data, .. } => Some(data),
            Self::Remove { .. } | Self::Delete { .. } => None,
        }
    }

    pub fn is_delete(&self) -> bool {
        matches!(self, Self::Delete { .. })
    }

    pub fn is_remove(&self) -> bool {
        matches!(self, Self::Remove { .. })
    }

    pub fn into_data(self) -> Option<T> {
        match self {
            Self::Upsert { data, .. } | Self::Patch { data, .. } => Some(data),
            Self::Remove { .. } | Self::Delete { .. } => None,
        }
    }

    pub fn has_data(&self) -> bool {
        matches!(self, Self::Upsert { .. } | Self::Patch { .. })
    }

    pub fn into_key(self) -> String {
        match self {
            Self::Upsert { key, .. }
            | Self::Patch { key, .. }
            | Self::Remove { key }
            | Self::Delete { key } => key,
        }
    }

    pub fn map<U, F: FnOnce(T) -> U>(self, f: F) -> Update<U> {
        match self {
            Self::Upsert { key, data } => Update::Upsert { key, data: f(data) },
            Self::Patch { key, data } => Update::Patch { key, data: f(data) },
            Self::Remove { key } => Update::Remove { key },
            Self::Delete { key } => Update::Delete { key },
        }
    }
}

impl<T> RichUpdate<T> {
    pub fn key(&self) -> &str {
        match self {
            Self::Created { key, .. }
            | Self::Updated { key, .. }
            | Self::Removed { key, .. }
            | Self::Deleted { key, .. } => key,
        }
    }

    pub fn data(&self) -> Option<&T> {
        match self {
            Self::Created { data, .. } => Some(data),
            Self::Updated { after, .. } => Some(after),
            Self::Removed { last_known, .. } | Self::Deleted { last_known, .. } => {
                last_known.as_ref()
            }
        }
    }

    pub fn before(&self) -> Option<&T> {
        match self {
            Self::Created { .. } => None,
            Self::Updated { before, .. } => Some(before),
            Self::Removed { last_known, .. } | Self::Deleted { last_known, .. } => {
                last_known.as_ref()
            }
        }
    }

    pub fn into_data(self) -> Option<T> {
        match self {
            Self::Created { data, .. } => Some(data),
            Self::Updated { after, .. } => Some(after),
            Self::Removed { last_known, .. } | Self::Deleted { last_known, .. } => last_known,
        }
    }

    pub fn is_created(&self) -> bool {
        matches!(self, Self::Created { .. })
    }

    pub fn is_updated(&self) -> bool {
        matches!(self, Self::Updated { .. })
    }

    pub fn is_removed(&self) -> bool {
        matches!(self, Self::Removed { .. })
    }

    pub fn is_deleted(&self) -> bool {
        matches!(self, Self::Deleted { .. })
    }

    pub fn patch(&self) -> Option<&Value> {
        match self {
            Self::Updated { patch, .. } => patch.as_ref(),
            _ => None,
        }
    }

    pub fn has_patch_field(&self, field: &str) -> bool {
        self.patch()
            .and_then(Value::as_object)
            .is_some_and(|object| object.contains_key(field))
    }
}

#[derive(Clone)]
pub enum KeyFilter {
    None,
    Single(String),
    Multiple(HashSet<String>),
}

impl KeyFilter {
    fn matches(&self, key: &str) -> bool {
        match self {
            Self::None => true,
            Self::Single(expected) => expected == key,
            Self::Multiple(keys) => keys.contains(key),
        }
    }
}

struct ScopedUpdateStream {
    state: ScopedState,
    view: String,
    subscription_id: Option<String>,
}

enum ScopedState {
    Lazy {
        connection: ConnectionManager,
        store: SharedStore,
        query: SubscriptionQuery,
        snapshot: SnapshotOptions,
    },
    Subscribing {
        future: Pin<
            Box<dyn Future<Output = Result<SubscriptionLease, crate::error::AreteError>> + Send>,
        >,
        inner: BroadcastStream<StoreUpdate>,
    },
    Active {
        inner: BroadcastStream<StoreUpdate>,
        _lease: Option<SubscriptionLease>,
    },
    Closed,
}

impl ScopedUpdateStream {
    fn active(receiver: broadcast::Receiver<StoreUpdate>, view: String) -> Self {
        Self {
            state: ScopedState::Active {
                inner: BroadcastStream::new(receiver),
                _lease: None,
            },
            view,
            subscription_id: None,
        }
    }

    fn lazy(
        connection: ConnectionManager,
        store: SharedStore,
        query: SubscriptionQuery,
        snapshot: SnapshotOptions,
    ) -> Self {
        Self {
            view: query.view.clone(),
            state: ScopedState::Lazy {
                connection,
                store,
                query,
                snapshot,
            },
            subscription_id: None,
        }
    }
}

impl Stream for ScopedUpdateStream {
    type Item = StoreUpdate;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        loop {
            match &mut this.state {
                ScopedState::Lazy { .. } => {
                    let ScopedState::Lazy {
                        connection,
                        store,
                        query,
                        snapshot,
                    } = std::mem::replace(&mut this.state, ScopedState::Closed)
                    else {
                        unreachable!()
                    };
                    let inner = BroadcastStream::new(store.subscribe());
                    let future =
                        Box::pin(async move { connection.acquire_query(query, snapshot).await });
                    this.state = ScopedState::Subscribing { future, inner };
                }
                ScopedState::Subscribing { future, .. } => match future.as_mut().poll(cx) {
                    Poll::Ready(Ok(lease)) => {
                        this.subscription_id = Some(lease.subscription_id().to_string());
                        let ScopedState::Subscribing { inner, .. } =
                            std::mem::replace(&mut this.state, ScopedState::Closed)
                        else {
                            unreachable!()
                        };
                        this.state = ScopedState::Active {
                            inner,
                            _lease: Some(lease),
                        };
                    }
                    Poll::Ready(Err(error)) => {
                        tracing::warn!(%error, "failed to acquire protocol v2 subscription");
                        this.state = ScopedState::Closed;
                        return Poll::Ready(None);
                    }
                    Poll::Pending => return Poll::Pending,
                },
                ScopedState::Active { inner, .. } => match Pin::new(inner).poll_next(cx) {
                    Poll::Ready(Some(Ok(update))) => {
                        let matches = this
                            .subscription_id
                            .as_ref()
                            .map_or(update.view == this.view, |id| id == &update.subscription_id);
                        if matches {
                            return Poll::Ready(Some(update));
                        }
                    }
                    Poll::Ready(Some(Err(_))) => {
                        tracing::warn!("entity stream lagged; protocol updates were dropped");
                    }
                    Poll::Ready(None) => return Poll::Ready(None),
                    Poll::Pending => return Poll::Pending,
                },
                ScopedState::Closed => return Poll::Ready(None),
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn query_with_options(
    view: String,
    key: Option<String>,
    partition: Option<String>,
    filters: BTreeMap<String, Value>,
    take: Option<usize>,
    skip: Option<usize>,
    after: Option<String>,
    snapshot_limit: Option<usize>,
) -> SubscriptionQuery {
    SubscriptionQuery {
        view,
        key,
        partition,
        filters,
        take,
        skip,
        after,
        snapshot_limit,
    }
}

pub struct EntityStream<T> {
    inner: ScopedUpdateStream,
    key_filter: KeyFilter,
    _marker: PhantomData<T>,
}

impl<T: DeserializeOwned + Clone + Send + 'static> EntityStream<T> {
    pub fn new(receiver: broadcast::Receiver<StoreUpdate>, view: String) -> Self {
        Self {
            inner: ScopedUpdateStream::active(receiver, view),
            key_filter: KeyFilter::None,
            _marker: PhantomData,
        }
    }

    pub fn new_filtered(
        receiver: broadcast::Receiver<StoreUpdate>,
        view: String,
        key: String,
    ) -> Self {
        Self {
            inner: ScopedUpdateStream::active(receiver, view),
            key_filter: KeyFilter::Single(key),
            _marker: PhantomData,
        }
    }

    pub fn new_multi_filtered(
        receiver: broadcast::Receiver<StoreUpdate>,
        view: String,
        keys: HashSet<String>,
    ) -> Self {
        Self {
            inner: ScopedUpdateStream::active(receiver, view),
            key_filter: KeyFilter::Multiple(keys),
            _marker: PhantomData,
        }
    }

    pub fn new_lazy(
        connection: ConnectionManager,
        store: SharedStore,
        entity_name: String,
        _subscription_view: String,
        key_filter: KeyFilter,
        subscription_key: Option<String>,
    ) -> Self {
        Self::new_lazy_with_opts(
            connection,
            store,
            entity_name,
            key_filter,
            subscription_key,
            None,
            BTreeMap::new(),
            None,
            None,
            None,
            None,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_lazy_with_opts(
        connection: ConnectionManager,
        store: SharedStore,
        view: String,
        key_filter: KeyFilter,
        key: Option<String>,
        partition: Option<String>,
        filters: BTreeMap<String, Value>,
        take: Option<usize>,
        skip: Option<usize>,
        with_snapshot: Option<bool>,
        after: Option<String>,
        snapshot_limit: Option<usize>,
    ) -> Self {
        let query = query_with_options(
            view,
            key,
            partition,
            filters,
            take,
            skip,
            after,
            snapshot_limit,
        );
        Self {
            inner: ScopedUpdateStream::lazy(
                connection,
                store,
                query,
                SnapshotOptions {
                    enabled: with_snapshot.unwrap_or(true),
                },
            ),
            key_filter,
            _marker: PhantomData,
        }
    }

    pub fn filter<F>(self, predicate: F) -> FilteredStream<Self, Update<T>, F>
    where
        F: FnMut(&Update<T>) -> bool,
    {
        FilteredStream::new(self, predicate)
    }

    pub fn filter_map<U, F>(self, f: F) -> FilterMapStream<Self, Update<T>, U, F>
    where
        F: FnMut(Update<T>) -> Option<U>,
    {
        FilterMapStream::new(self, f)
    }

    pub fn map<U, F>(self, f: F) -> MapStream<Self, Update<T>, U, F>
    where
        F: FnMut(Update<T>) -> U,
    {
        MapStream::new(self, f)
    }
}

impl<T: DeserializeOwned + Clone + Send + Unpin + 'static> Stream for EntityStream<T> {
    type Item = Update<T>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        loop {
            let update = match Pin::new(&mut this.inner).poll_next(cx) {
                Poll::Ready(Some(update)) => update,
                Poll::Ready(None) => return Poll::Ready(None),
                Poll::Pending => return Poll::Pending,
            };
            if !this.key_filter.matches(&update.key) {
                continue;
            }
            match update.operation {
                Operation::Remove => return Poll::Ready(Some(Update::Remove { key: update.key })),
                Operation::Delete => return Poll::Ready(Some(Update::Delete { key: update.key })),
                Operation::Upsert | Operation::Patch => {
                    let Some(data) = update.data else {
                        continue;
                    };
                    match serde_json::from_value(data) {
                        Ok(data) if update.operation == Operation::Patch => {
                            return Poll::Ready(Some(Update::Patch {
                                key: update.key,
                                data,
                            }))
                        }
                        Ok(data) => {
                            return Poll::Ready(Some(Update::Upsert {
                                key: update.key,
                                data,
                            }))
                        }
                        Err(error) => {
                            tracing::warn!(key = %update.key, %error, "failed to deserialize entity update")
                        }
                    }
                }
            }
        }
    }
}

pub struct RichEntityStream<T> {
    inner: ScopedUpdateStream,
    key_filter: KeyFilter,
    _marker: PhantomData<T>,
}

impl<T: DeserializeOwned + Clone + Send + 'static> RichEntityStream<T> {
    pub fn new(receiver: broadcast::Receiver<StoreUpdate>, view: String) -> Self {
        Self {
            inner: ScopedUpdateStream::active(receiver, view),
            key_filter: KeyFilter::None,
            _marker: PhantomData,
        }
    }

    pub fn new_filtered(
        receiver: broadcast::Receiver<StoreUpdate>,
        view: String,
        key: String,
    ) -> Self {
        Self {
            inner: ScopedUpdateStream::active(receiver, view),
            key_filter: KeyFilter::Single(key),
            _marker: PhantomData,
        }
    }

    pub fn new_lazy(
        connection: ConnectionManager,
        store: SharedStore,
        entity_name: String,
        _subscription_view: String,
        key_filter: KeyFilter,
        subscription_key: Option<String>,
    ) -> Self {
        Self::new_lazy_with_opts(
            connection,
            store,
            entity_name,
            key_filter,
            subscription_key,
            None,
            BTreeMap::new(),
            None,
            None,
            None,
            None,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_lazy_with_opts(
        connection: ConnectionManager,
        store: SharedStore,
        view: String,
        key_filter: KeyFilter,
        key: Option<String>,
        partition: Option<String>,
        filters: BTreeMap<String, Value>,
        take: Option<usize>,
        skip: Option<usize>,
        with_snapshot: Option<bool>,
        after: Option<String>,
        snapshot_limit: Option<usize>,
    ) -> Self {
        let query = query_with_options(
            view,
            key,
            partition,
            filters,
            take,
            skip,
            after,
            snapshot_limit,
        );
        Self {
            inner: ScopedUpdateStream::lazy(
                connection,
                store,
                query,
                SnapshotOptions {
                    enabled: with_snapshot.unwrap_or(true),
                },
            ),
            key_filter,
            _marker: PhantomData,
        }
    }

    pub fn filter<F>(self, predicate: F) -> FilteredStream<Self, RichUpdate<T>, F>
    where
        F: FnMut(&RichUpdate<T>) -> bool,
    {
        FilteredStream::new(self, predicate)
    }

    pub fn filter_map<U, F>(self, f: F) -> FilterMapStream<Self, RichUpdate<T>, U, F>
    where
        F: FnMut(RichUpdate<T>) -> Option<U>,
    {
        FilterMapStream::new(self, f)
    }

    pub fn map<U, F>(self, f: F) -> MapStream<Self, RichUpdate<T>, U, F>
    where
        F: FnMut(RichUpdate<T>) -> U,
    {
        MapStream::new(self, f)
    }
}

impl<T: DeserializeOwned + Clone + Send + Unpin + 'static> Stream for RichEntityStream<T> {
    type Item = RichUpdate<T>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        loop {
            let update = match Pin::new(&mut this.inner).poll_next(cx) {
                Poll::Ready(Some(update)) => update,
                Poll::Ready(None) => return Poll::Ready(None),
                Poll::Pending => return Poll::Pending,
            };
            if !this.key_filter.matches(&update.key) {
                continue;
            }
            let previous = update
                .previous
                .and_then(|value| serde_json::from_value(value).ok());
            match update.operation {
                Operation::Remove => {
                    return Poll::Ready(Some(RichUpdate::Removed {
                        key: update.key,
                        last_known: previous,
                    }))
                }
                Operation::Delete => {
                    return Poll::Ready(Some(RichUpdate::Deleted {
                        key: update.key,
                        last_known: previous,
                    }))
                }
                Operation::Upsert | Operation::Patch => {
                    let Some(data) = update.data else {
                        continue;
                    };
                    let Ok(after) = serde_json::from_value(data) else {
                        continue;
                    };
                    return Poll::Ready(Some(match previous {
                        Some(before) => RichUpdate::Updated {
                            key: update.key,
                            before,
                            after,
                            patch: update.patch,
                        },
                        None => RichUpdate::Created {
                            key: update.key,
                            data: after,
                        },
                    }));
                }
            }
        }
    }
}

pub struct UseStream<T> {
    inner: ScopedUpdateStream,
    key_filter: KeyFilter,
    _marker: PhantomData<T>,
}

impl<T: DeserializeOwned + Clone + Send + 'static> UseStream<T> {
    pub fn new(receiver: broadcast::Receiver<StoreUpdate>, view: String) -> Self {
        Self {
            inner: ScopedUpdateStream::active(receiver, view),
            key_filter: KeyFilter::None,
            _marker: PhantomData,
        }
    }

    pub fn new_filtered(
        receiver: broadcast::Receiver<StoreUpdate>,
        view: String,
        key: String,
    ) -> Self {
        Self {
            inner: ScopedUpdateStream::active(receiver, view),
            key_filter: KeyFilter::Single(key),
            _marker: PhantomData,
        }
    }

    pub fn new_lazy(
        connection: ConnectionManager,
        store: SharedStore,
        entity_name: String,
        _subscription_view: String,
        key_filter: KeyFilter,
        subscription_key: Option<String>,
    ) -> Self {
        Self::new_lazy_with_opts(
            connection,
            store,
            entity_name,
            key_filter,
            subscription_key,
            None,
            BTreeMap::new(),
            None,
            None,
            None,
            None,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_lazy_with_opts(
        connection: ConnectionManager,
        store: SharedStore,
        view: String,
        key_filter: KeyFilter,
        key: Option<String>,
        partition: Option<String>,
        filters: BTreeMap<String, Value>,
        take: Option<usize>,
        skip: Option<usize>,
        with_snapshot: Option<bool>,
        after: Option<String>,
        snapshot_limit: Option<usize>,
    ) -> Self {
        let query = query_with_options(
            view,
            key,
            partition,
            filters,
            take,
            skip,
            after,
            snapshot_limit,
        );
        Self {
            inner: ScopedUpdateStream::lazy(
                connection,
                store,
                query,
                SnapshotOptions {
                    enabled: with_snapshot.unwrap_or(true),
                },
            ),
            key_filter,
            _marker: PhantomData,
        }
    }

    pub fn filter<F>(self, predicate: F) -> FilteredStream<Self, T, F>
    where
        F: FnMut(&T) -> bool,
    {
        FilteredStream::new(self, predicate)
    }

    pub fn filter_map<U, F>(self, f: F) -> FilterMapStream<Self, T, U, F>
    where
        F: FnMut(T) -> Option<U>,
    {
        FilterMapStream::new(self, f)
    }

    pub fn map<U, F>(self, f: F) -> MapStream<Self, T, U, F>
    where
        F: FnMut(T) -> U,
    {
        MapStream::new(self, f)
    }
}

impl<T: DeserializeOwned + Clone + Send + Unpin + 'static> Stream for UseStream<T> {
    type Item = T;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        loop {
            let update = match Pin::new(&mut this.inner).poll_next(cx) {
                Poll::Ready(Some(update)) => update,
                Poll::Ready(None) => return Poll::Ready(None),
                Poll::Pending => return Poll::Pending,
            };
            if !this.key_filter.matches(&update.key)
                || matches!(update.operation, Operation::Remove | Operation::Delete)
            {
                continue;
            }
            if let Some(data) = update.data {
                match serde_json::from_value(data) {
                    Ok(data) => return Poll::Ready(Some(data)),
                    Err(error) => {
                        tracing::warn!(key = %update.key, %error, "failed to deserialize entity update")
                    }
                }
            }
        }
    }
}

pin_project! {
    pub struct FilteredStream<S, I, F> {
        #[pin]
        inner: S,
        predicate: F,
        _item: PhantomData<I>,
    }
}

impl<S, I, F> FilteredStream<S, I, F> {
    pub fn new(inner: S, predicate: F) -> Self {
        Self {
            inner,
            predicate,
            _item: PhantomData,
        }
    }
}

impl<S, I, F> Stream for FilteredStream<S, I, F>
where
    S: Stream<Item = I>,
    F: FnMut(&I) -> bool,
{
    type Item = I;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();
        loop {
            match this.inner.as_mut().poll_next(cx) {
                Poll::Ready(Some(item)) if (this.predicate)(&item) => {
                    return Poll::Ready(Some(item))
                }
                Poll::Ready(Some(_)) => {}
                Poll::Ready(None) => return Poll::Ready(None),
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

impl<S, I, F> FilteredStream<S, I, F>
where
    S: Stream<Item = I>,
    F: FnMut(&I) -> bool,
{
    pub fn filter<F2>(self, predicate: F2) -> FilteredStream<Self, I, F2>
    where
        F2: FnMut(&I) -> bool,
    {
        FilteredStream::new(self, predicate)
    }

    pub fn filter_map<U, F2>(self, f: F2) -> FilterMapStream<Self, I, U, F2>
    where
        F2: FnMut(I) -> Option<U>,
    {
        FilterMapStream::new(self, f)
    }

    pub fn map<U, F2>(self, f: F2) -> MapStream<Self, I, U, F2>
    where
        F2: FnMut(I) -> U,
    {
        MapStream::new(self, f)
    }
}

pin_project! {
    pub struct FilterMapStream<S, I, U, F> {
        #[pin]
        inner: S,
        f: F,
        _item: PhantomData<(I, U)>,
    }
}

impl<S, I, U, F> FilterMapStream<S, I, U, F> {
    pub fn new(inner: S, f: F) -> Self {
        Self {
            inner,
            f,
            _item: PhantomData,
        }
    }
}

impl<S, I, U, F> Stream for FilterMapStream<S, I, U, F>
where
    S: Stream<Item = I>,
    F: FnMut(I) -> Option<U>,
{
    type Item = U;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();
        loop {
            match this.inner.as_mut().poll_next(cx) {
                Poll::Ready(Some(item)) => {
                    if let Some(mapped) = (this.f)(item) {
                        return Poll::Ready(Some(mapped));
                    }
                }
                Poll::Ready(None) => return Poll::Ready(None),
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

impl<S, I, U, F> FilterMapStream<S, I, U, F>
where
    S: Stream<Item = I>,
    F: FnMut(I) -> Option<U>,
{
    pub fn filter<F2>(self, predicate: F2) -> FilteredStream<Self, U, F2>
    where
        F2: FnMut(&U) -> bool,
    {
        FilteredStream::new(self, predicate)
    }

    pub fn filter_map<V, F2>(self, f: F2) -> FilterMapStream<Self, U, V, F2>
    where
        F2: FnMut(U) -> Option<V>,
    {
        FilterMapStream::new(self, f)
    }

    pub fn map<V, F2>(self, f: F2) -> MapStream<Self, U, V, F2>
    where
        F2: FnMut(U) -> V,
    {
        MapStream::new(self, f)
    }
}

pin_project! {
    pub struct MapStream<S, I, U, F> {
        #[pin]
        inner: S,
        f: F,
        _item: PhantomData<(I, U)>,
    }
}

impl<S, I, U, F> MapStream<S, I, U, F> {
    pub fn new(inner: S, f: F) -> Self {
        Self {
            inner,
            f,
            _item: PhantomData,
        }
    }
}

impl<S, I, U, F> Stream for MapStream<S, I, U, F>
where
    S: Stream<Item = I>,
    F: FnMut(I) -> U,
{
    type Item = U;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.project();
        match this.inner.poll_next(cx) {
            Poll::Ready(Some(item)) => Poll::Ready(Some((this.f)(item))),
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl<S, I, U, F> MapStream<S, I, U, F>
where
    S: Stream<Item = I>,
    F: FnMut(I) -> U,
{
    pub fn filter<F2>(self, predicate: F2) -> FilteredStream<Self, U, F2>
    where
        F2: FnMut(&U) -> bool,
    {
        FilteredStream::new(self, predicate)
    }

    pub fn filter_map<V, F2>(self, f: F2) -> FilterMapStream<Self, U, V, F2>
    where
        F2: FnMut(U) -> Option<V>,
    {
        FilterMapStream::new(self, f)
    }

    pub fn map<V, F2>(self, f: F2) -> MapStream<Self, U, V, F2>
    where
        F2: FnMut(U) -> V,
    {
        MapStream::new(self, f)
    }
}
