use crate::connection::{ConnectionManager, SubscriptionOptions};
use crate::frame::Operation;
use crate::store::{SharedStore, StoreUpdate};
use futures_util::Stream;
use pin_project_lite::pin_project;
use serde::de::DeserializeOwned;
use std::collections::HashSet;
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
        patch: Option<serde_json::Value>,
    },
    Deleted {
        key: String,
        last_known: Option<T>,
    },
}

impl<T> Update<T> {
    pub fn key(&self) -> &str {
        match self {
            Update::Upsert { key, .. } => key,
            Update::Patch { key, .. } => key,
            Update::Delete { key } => key,
        }
    }

    pub fn data(&self) -> Option<&T> {
        match self {
            Update::Upsert { data, .. } => Some(data),
            Update::Patch { data, .. } => Some(data),
            Update::Delete { .. } => None,
        }
    }

    pub fn is_delete(&self) -> bool {
        matches!(self, Update::Delete { .. })
    }

    pub fn into_data(self) -> Option<T> {
        match self {
            Update::Upsert { data, .. } => Some(data),
            Update::Patch { data, .. } => Some(data),
            Update::Delete { .. } => None,
        }
    }

    pub fn has_data(&self) -> bool {
        matches!(self, Update::Upsert { .. } | Update::Patch { .. })
    }

    pub fn into_key(self) -> String {
        match self {
            Update::Upsert { key, .. } => key,
            Update::Patch { key, .. } => key,
            Update::Delete { key } => key,
        }
    }

    pub fn map<U, F: FnOnce(T) -> U>(self, f: F) -> Update<U> {
        match self {
            Update::Upsert { key, data } => Update::Upsert { key, data: f(data) },
            Update::Patch { key, data } => Update::Patch { key, data: f(data) },
            Update::Delete { key } => Update::Delete { key },
        }
    }
}

impl<T> RichUpdate<T> {
    pub fn key(&self) -> &str {
        match self {
            RichUpdate::Created { key, .. } => key,
            RichUpdate::Updated { key, .. } => key,
            RichUpdate::Deleted { key, .. } => key,
        }
    }

    pub fn data(&self) -> Option<&T> {
        match self {
            RichUpdate::Created { data, .. } => Some(data),
            RichUpdate::Updated { after, .. } => Some(after),
            RichUpdate::Deleted { last_known, .. } => last_known.as_ref(),
        }
    }

    pub fn before(&self) -> Option<&T> {
        match self {
            RichUpdate::Created { .. } => None,
            RichUpdate::Updated { before, .. } => Some(before),
            RichUpdate::Deleted { last_known, .. } => last_known.as_ref(),
        }
    }

    pub fn into_data(self) -> Option<T> {
        match self {
            RichUpdate::Created { data, .. } => Some(data),
            RichUpdate::Updated { after, .. } => Some(after),
            RichUpdate::Deleted { last_known, .. } => last_known,
        }
    }

    pub fn is_created(&self) -> bool {
        matches!(self, RichUpdate::Created { .. })
    }

    pub fn is_updated(&self) -> bool {
        matches!(self, RichUpdate::Updated { .. })
    }

    pub fn is_deleted(&self) -> bool {
        matches!(self, RichUpdate::Deleted { .. })
    }

    pub fn patch(&self) -> Option<&serde_json::Value> {
        match self {
            RichUpdate::Updated { patch, .. } => patch.as_ref(),
            _ => None,
        }
    }

    pub fn has_patch_field(&self, field: &str) -> bool {
        self.patch()
            .and_then(|p| p.as_object())
            .map(|obj| obj.contains_key(field))
            .unwrap_or(false)
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
            KeyFilter::None => true,
            KeyFilter::Single(k) => k == key,
            KeyFilter::Multiple(keys) => keys.contains(key),
        }
    }
}

pub struct EntityStream<T> {
    state: EntityStreamState<T>,
    view: String,
    key_filter: KeyFilter,
    _marker: PhantomData<T>,
}

enum EntityStreamState<T> {
    Lazy {
        connection: ConnectionManager,
        store: SharedStore,
        subscription_view: String,
        subscription_key: Option<String>,
        take: Option<u32>,
        skip: Option<u32>,
        with_snapshot: Option<bool>,
        after: Option<String>,
        snapshot_limit: Option<usize>,
    },
    Active {
        inner: BroadcastStream<StoreUpdate>,
    },
    Subscribing {
        fut: Pin<Box<dyn Future<Output = ()> + Send>>,
        inner: BroadcastStream<StoreUpdate>,
    },
    Invalid,
    _Phantom(PhantomData<T>),
}

impl<T: DeserializeOwned + Clone + Send + 'static> EntityStream<T> {
    pub fn new(rx: broadcast::Receiver<StoreUpdate>, view: String) -> Self {
        Self {
            state: EntityStreamState::Active {
                inner: BroadcastStream::new(rx),
            },
            view,
            key_filter: KeyFilter::None,
            _marker: PhantomData,
        }
    }

    pub fn new_filtered(rx: broadcast::Receiver<StoreUpdate>, view: String, key: String) -> Self {
        Self {
            state: EntityStreamState::Active {
                inner: BroadcastStream::new(rx),
            },
            view,
            key_filter: KeyFilter::Single(key),
            _marker: PhantomData,
        }
    }

    pub fn new_multi_filtered(
        rx: broadcast::Receiver<StoreUpdate>,
        view: String,
        keys: HashSet<String>,
    ) -> Self {
        Self {
            state: EntityStreamState::Active {
                inner: BroadcastStream::new(rx),
            },
            view,
            key_filter: KeyFilter::Multiple(keys),
            _marker: PhantomData,
        }
    }

    pub fn new_lazy(
        connection: ConnectionManager,
        store: SharedStore,
        entity_name: String,
        subscription_view: String,
        key_filter: KeyFilter,
        subscription_key: Option<String>,
    ) -> Self {
        Self::new_lazy_with_opts(
            connection,
            store,
            entity_name,
            subscription_view,
            key_filter,
            subscription_key,
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
        entity_name: String,
        subscription_view: String,
        key_filter: KeyFilter,
        subscription_key: Option<String>,
        take: Option<u32>,
        skip: Option<u32>,
        with_snapshot: Option<bool>,
        after: Option<String>,
        snapshot_limit: Option<usize>,
    ) -> Self {
        Self {
            state: EntityStreamState::Lazy {
                connection,
                store,
                subscription_view,
                subscription_key,
                take,
                skip,
                with_snapshot,
                after,
                snapshot_limit,
            },
            view: entity_name,
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
            match &mut this.state {
                EntityStreamState::Lazy { .. } => {
                    let EntityStreamState::Lazy {
                        connection,
                        store,
                        subscription_view,
                        subscription_key,
                        take,
                        skip,
                        with_snapshot,
                        after,
                        snapshot_limit,
                    } = std::mem::replace(&mut this.state, EntityStreamState::Invalid)
                    else {
                        unreachable!()
                    };

                    // Subscribe to broadcast BEFORE sending subscription to server
                    // This ensures we don't miss any frames that arrive during setup
                    let inner = BroadcastStream::new(store.subscribe());

                    let conn = connection.clone();
                    let view = subscription_view.clone();
                    let key = subscription_key.clone();
                    let fut = Box::pin(async move {
                        let opts = SubscriptionOptions {
                            take,
                            skip,
                            with_snapshot,
                            after,
                            snapshot_limit,
                        };
                        conn.ensure_subscription_with_opts(&view, key.as_deref(), opts)
                            .await;
                    });

                    this.state = EntityStreamState::Subscribing { fut, inner };
                    continue;
                }
                EntityStreamState::Subscribing { fut, .. } => match fut.as_mut().poll(cx) {
                    Poll::Ready(()) => {
                        let EntityStreamState::Subscribing { inner, .. } =
                            std::mem::replace(&mut this.state, EntityStreamState::Invalid)
                        else {
                            unreachable!()
                        };
                        this.state = EntityStreamState::Active { inner };
                        continue;
                    }
                    Poll::Pending => return Poll::Pending,
                },
                EntityStreamState::Active { inner } => match Pin::new(inner).poll_next(cx) {
                    Poll::Ready(Some(Ok(update))) => {
                        if update.view != this.view {
                            continue;
                        }

                        if !this.key_filter.matches(&update.key) {
                            continue;
                        }

                        match update.operation {
                            Operation::Delete => {
                                return Poll::Ready(Some(Update::Delete { key: update.key }));
                            }
                            Operation::Upsert | Operation::Create | Operation::Snapshot => {
                                if let Some(data) = update.data {
                                    if let Ok(typed) = serde_json::from_value::<T>(data) {
                                        return Poll::Ready(Some(Update::Upsert {
                                            key: update.key,
                                            data: typed,
                                        }));
                                    }
                                }
                            }
                            Operation::Patch => {
                                if let Some(data) = update.data {
                                    match serde_json::from_value::<T>(data) {
                                        Ok(typed) => {
                                            return Poll::Ready(Some(Update::Patch {
                                                key: update.key,
                                                data: typed,
                                            }));
                                        }
                                        Err(e) => {
                                            tracing::warn!(
                                                key = %update.key,
                                                error = %e,
                                                "Patch failed to deserialize to full type, skipping"
                                            );
                                            continue;
                                        }
                                    }
                                }
                            }
                            Operation::Subscribed => {
                                continue;
                            }
                        }
                    }
                    Poll::Ready(Some(Err(_lagged))) => {
                        tracing::warn!("EntityStream lagged behind, some messages were dropped");
                        continue;
                    }
                    Poll::Ready(None) => {
                        return Poll::Ready(None);
                    }
                    Poll::Pending => {
                        return Poll::Pending;
                    }
                },
                EntityStreamState::Invalid => {
                    panic!("EntityStream in invalid state");
                }
                EntityStreamState::_Phantom(_) => unreachable!(),
            }
        }
    }
}

pub struct RichEntityStream<T> {
    state: RichEntityStreamState<T>,
    view: String,
    key_filter: KeyFilter,
    _marker: PhantomData<T>,
}

enum RichEntityStreamState<T> {
    Lazy {
        connection: ConnectionManager,
        store: SharedStore,
        subscription_view: String,
        subscription_key: Option<String>,
        take: Option<u32>,
        skip: Option<u32>,
        with_snapshot: Option<bool>,
        after: Option<String>,
        snapshot_limit: Option<usize>,
    },
    Active {
        inner: BroadcastStream<StoreUpdate>,
    },
    Subscribing {
        fut: Pin<Box<dyn Future<Output = ()> + Send>>,
        inner: BroadcastStream<StoreUpdate>,
    },
    Invalid,
    _Phantom(PhantomData<T>),
}

impl<T: DeserializeOwned + Clone + Send + 'static> RichEntityStream<T> {
    pub fn new(rx: broadcast::Receiver<StoreUpdate>, view: String) -> Self {
        Self {
            state: RichEntityStreamState::Active {
                inner: BroadcastStream::new(rx),
            },
            view,
            key_filter: KeyFilter::None,
            _marker: PhantomData,
        }
    }

    pub fn new_filtered(rx: broadcast::Receiver<StoreUpdate>, view: String, key: String) -> Self {
        Self {
            state: RichEntityStreamState::Active {
                inner: BroadcastStream::new(rx),
            },
            view,
            key_filter: KeyFilter::Single(key),
            _marker: PhantomData,
        }
    }

    pub fn new_lazy(
        connection: ConnectionManager,
        store: SharedStore,
        entity_name: String,
        subscription_view: String,
        key_filter: KeyFilter,
        subscription_key: Option<String>,
    ) -> Self {
        Self::new_lazy_with_opts(
            connection,
            store,
            entity_name,
            subscription_view,
            key_filter,
            subscription_key,
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
        entity_name: String,
        subscription_view: String,
        key_filter: KeyFilter,
        subscription_key: Option<String>,
        take: Option<u32>,
        skip: Option<u32>,
        with_snapshot: Option<bool>,
        after: Option<String>,
        snapshot_limit: Option<usize>,
    ) -> Self {
        Self {
            state: RichEntityStreamState::Lazy {
                connection,
                store,
                subscription_view,
                subscription_key,
                take,
                skip,
                with_snapshot,
                after,
                snapshot_limit,
            },
            view: entity_name,
            key_filter,
            _marker: PhantomData,
        }
    }
}

impl<T: DeserializeOwned + Clone + Send + Unpin + 'static> Stream for RichEntityStream<T> {
    type Item = RichUpdate<T>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        loop {
            match &mut this.state {
                RichEntityStreamState::Lazy { .. } => {
                    let RichEntityStreamState::Lazy {
                        connection,
                        store,
                        subscription_view,
                        subscription_key,
                        take,
                        skip,
                        with_snapshot,
                        after,
                        snapshot_limit,
                    } = std::mem::replace(&mut this.state, RichEntityStreamState::Invalid)
                    else {
                        unreachable!()
                    };

                    // Subscribe to broadcast BEFORE sending subscription to server
                    // This ensures we don't miss any frames that arrive during setup
                    let inner = BroadcastStream::new(store.subscribe());

                    let conn = connection.clone();
                    let view = subscription_view.clone();
                    let key = subscription_key.clone();
                    let fut = Box::pin(async move {
                        let opts = SubscriptionOptions {
                            take,
                            skip,
                            with_snapshot,
                            after,
                            snapshot_limit,
                        };
                        conn.ensure_subscription_with_opts(&view, key.as_deref(), opts)
                            .await;
                    });

                    this.state = RichEntityStreamState::Subscribing { fut, inner };
                    continue;
                }
                RichEntityStreamState::Subscribing { fut, .. } => match fut.as_mut().poll(cx) {
                    Poll::Ready(()) => {
                        let RichEntityStreamState::Subscribing { inner, .. } =
                            std::mem::replace(&mut this.state, RichEntityStreamState::Invalid)
                        else {
                            unreachable!()
                        };
                        this.state = RichEntityStreamState::Active { inner };
                        continue;
                    }
                    Poll::Pending => return Poll::Pending,
                },
                RichEntityStreamState::Active { inner } => match Pin::new(inner).poll_next(cx) {
                    Poll::Ready(Some(Ok(update))) => {
                        if update.view != this.view {
                            continue;
                        }

                        if !this.key_filter.matches(&update.key) {
                            continue;
                        }

                        let previous: Option<T> =
                            update.previous.and_then(|v| serde_json::from_value(v).ok());

                        match update.operation {
                            Operation::Delete => {
                                return Poll::Ready(Some(RichUpdate::Deleted {
                                    key: update.key,
                                    last_known: previous,
                                }));
                            }
                            Operation::Create | Operation::Snapshot => {
                                if let Some(data) = update.data {
                                    if let Ok(typed) = serde_json::from_value::<T>(data) {
                                        return Poll::Ready(Some(RichUpdate::Created {
                                            key: update.key,
                                            data: typed,
                                        }));
                                    }
                                }
                            }
                            Operation::Upsert | Operation::Patch => {
                                if let Some(data) = update.data {
                                    match serde_json::from_value::<T>(data.clone()) {
                                        Ok(after) => {
                                            if let Some(before) = previous {
                                                return Poll::Ready(Some(RichUpdate::Updated {
                                                    key: update.key,
                                                    before,
                                                    after,
                                                    patch: update.patch,
                                                }));
                                            } else {
                                                return Poll::Ready(Some(RichUpdate::Created {
                                                    key: update.key,
                                                    data: after,
                                                }));
                                            }
                                        }
                                        Err(e) => {
                                            tracing::warn!(
                                                key = %update.key,
                                                error = %e,
                                                "Update failed to deserialize, skipping"
                                            );
                                            continue;
                                        }
                                    }
                                }
                            }
                            Operation::Subscribed => {
                                continue;
                            }
                        }
                    }
                    Poll::Ready(Some(Err(_lagged))) => {
                        tracing::warn!(
                            "RichEntityStream lagged behind, some messages were dropped"
                        );
                        continue;
                    }
                    Poll::Ready(None) => {
                        return Poll::Ready(None);
                    }
                    Poll::Pending => {
                        return Poll::Pending;
                    }
                },
                RichEntityStreamState::Invalid => {
                    panic!("RichEntityStream in invalid state");
                }
                RichEntityStreamState::_Phantom(_) => unreachable!(),
            }
        }
    }
}

impl<T: DeserializeOwned + Clone + Send + 'static> RichEntityStream<T> {
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
                Poll::Ready(Some(item)) => {
                    if (this.predicate)(&item) {
                        return Poll::Ready(Some(item));
                    }
                }
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

/// A stream that emits merged entity values directly (filtering out deletes).
///
/// This is the simplest streaming interface - it just yields `T` after each change,
/// applying patches to give you the full merged entity state. Deletes are filtered out.
///
/// Corresponds to TypeScript SDK's `.use()` method.
pub struct UseStream<T> {
    state: UseStreamState<T>,
    view: String,
    key_filter: KeyFilter,
    _marker: PhantomData<T>,
}

enum UseStreamState<T> {
    Lazy {
        connection: ConnectionManager,
        store: SharedStore,
        subscription_view: String,
        subscription_key: Option<String>,
        take: Option<u32>,
        skip: Option<u32>,
        with_snapshot: Option<bool>,
        after: Option<String>,
        snapshot_limit: Option<usize>,
    },
    Active {
        inner: BroadcastStream<StoreUpdate>,
    },
    Subscribing {
        fut: Pin<Box<dyn Future<Output = ()> + Send>>,
        inner: BroadcastStream<StoreUpdate>,
    },
    Invalid,
    _Phantom(PhantomData<T>),
}

impl<T: DeserializeOwned + Clone + Send + 'static> UseStream<T> {
    pub fn new(rx: broadcast::Receiver<StoreUpdate>, view: String) -> Self {
        Self {
            state: UseStreamState::Active {
                inner: BroadcastStream::new(rx),
            },
            view,
            key_filter: KeyFilter::None,
            _marker: PhantomData,
        }
    }

    pub fn new_filtered(rx: broadcast::Receiver<StoreUpdate>, view: String, key: String) -> Self {
        Self {
            state: UseStreamState::Active {
                inner: BroadcastStream::new(rx),
            },
            view,
            key_filter: KeyFilter::Single(key),
            _marker: PhantomData,
        }
    }

    pub fn new_lazy(
        connection: ConnectionManager,
        store: SharedStore,
        entity_name: String,
        subscription_view: String,
        key_filter: KeyFilter,
        subscription_key: Option<String>,
    ) -> Self {
        Self::new_lazy_with_opts(
            connection,
            store,
            entity_name,
            subscription_view,
            key_filter,
            subscription_key,
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
        entity_name: String,
        subscription_view: String,
        key_filter: KeyFilter,
        subscription_key: Option<String>,
        take: Option<u32>,
        skip: Option<u32>,
        with_snapshot: Option<bool>,
        after: Option<String>,
        snapshot_limit: Option<usize>,
    ) -> Self {
        Self {
            state: UseStreamState::Lazy {
                connection,
                store,
                subscription_view,
                subscription_key,
                take,
                skip,
                with_snapshot,
                after,
                snapshot_limit,
            },
            view: entity_name,
            key_filter,
            _marker: PhantomData,
        }
    }

    /// Filter the stream to only emit items matching the predicate.
    pub fn filter<F>(self, predicate: F) -> FilteredStream<Self, T, F>
    where
        F: FnMut(&T) -> bool,
    {
        FilteredStream::new(self, predicate)
    }

    /// Filter and transform items in one step.
    pub fn filter_map<U, F>(self, f: F) -> FilterMapStream<Self, T, U, F>
    where
        F: FnMut(T) -> Option<U>,
    {
        FilterMapStream::new(self, f)
    }

    /// Transform each item.
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
            match &mut this.state {
                UseStreamState::Lazy { .. } => {
                    let UseStreamState::Lazy {
                        connection,
                        store,
                        subscription_view,
                        subscription_key,
                        take,
                        skip,
                        with_snapshot,
                        after,
                        snapshot_limit,
                    } = std::mem::replace(&mut this.state, UseStreamState::Invalid)
                    else {
                        unreachable!()
                    };

                    // Subscribe to broadcast BEFORE sending subscription to server
                    let inner = BroadcastStream::new(store.subscribe());

                    let conn = connection.clone();
                    let view = subscription_view.clone();
                    let key = subscription_key.clone();
                    let fut = Box::pin(async move {
                        let opts = SubscriptionOptions {
                            take,
                            skip,
                            with_snapshot,
                            after,
                            snapshot_limit,
                        };
                        conn.ensure_subscription_with_opts(&view, key.as_deref(), opts)
                            .await;
                    });

                    this.state = UseStreamState::Subscribing { fut, inner };
                    continue;
                }
                UseStreamState::Subscribing { fut, .. } => match fut.as_mut().poll(cx) {
                    Poll::Ready(()) => {
                        let UseStreamState::Subscribing { inner, .. } =
                            std::mem::replace(&mut this.state, UseStreamState::Invalid)
                        else {
                            unreachable!()
                        };
                        this.state = UseStreamState::Active { inner };
                        continue;
                    }
                    Poll::Pending => return Poll::Pending,
                },
                UseStreamState::Active { inner } => match Pin::new(inner).poll_next(cx) {
                    Poll::Ready(Some(Ok(update))) => {
                        if update.view != this.view {
                            continue;
                        }

                        if !this.key_filter.matches(&update.key) {
                            continue;
                        }

                        // Filter out deletes - UseStream only emits actual entity data
                        match update.operation {
                            Operation::Delete => {
                                // Skip deletes entirely
                                continue;
                            }
                            Operation::Upsert
                            | Operation::Create
                            | Operation::Snapshot
                            | Operation::Patch => {
                                if let Some(data) = update.data {
                                    match serde_json::from_value::<T>(data) {
                                        Ok(typed) => {
                                            return Poll::Ready(Some(typed));
                                        }
                                        Err(e) => {
                                            tracing::warn!(
                                                key = %update.key,
                                                error = %e,
                                                "UseStream: failed to deserialize entity, skipping"
                                            );
                                            continue;
                                        }
                                    }
                                }
                            }
                            Operation::Subscribed => {
                                continue;
                            }
                        }
                    }
                    Poll::Ready(Some(Err(_lagged))) => {
                        tracing::warn!("UseStream lagged behind, some messages were dropped");
                        continue;
                    }
                    Poll::Ready(None) => {
                        return Poll::Ready(None);
                    }
                    Poll::Pending => {
                        return Poll::Pending;
                    }
                },
                UseStreamState::Invalid => {
                    panic!("UseStream in invalid state");
                }
                UseStreamState::_Phantom(_) => unreachable!(),
            }
        }
    }
}
