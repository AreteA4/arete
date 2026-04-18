//! Subscription registry: tracks which views each connection is subscribed to.
//!
//! The SDK's `ConnectionManager` already multiplexes many subscriptions over
//! a single WebSocket and dedupes them internally. This registry exists only
//! to give the MCP layer stable opaque `subscription_id`s that map back to
//! `(connection_id, view, key)` so query tools can look up the right cache.
//!
//! Per the v1 plan: snapshot only, no filters yet. Filter wiring lands in step 5.

use std::sync::Arc;

use dashmap::DashMap;
use arete_sdk::{Subscription, Unsubscription};
use uuid::Uuid;

use crate::connections::ConnectionId;

pub type SubscriptionId = String;

#[derive(Clone)]
pub struct SubscriptionEntry {
    pub id: SubscriptionId,
    pub connection_id: ConnectionId,
    pub view: String,
    pub key: Option<String>,
}

impl SubscriptionEntry {
    /// Build the SDK Subscription this entry represents.
    pub fn to_sdk_subscription(&self) -> Subscription {
        let mut sub = Subscription::new(&self.view);
        if let Some(k) = &self.key {
            sub = sub.with_key(k);
        }
        sub
    }

    /// Build the SDK Unsubscription this entry represents.
    pub fn to_sdk_unsubscription(&self) -> Unsubscription {
        let mut un = Unsubscription::new(&self.view);
        if let Some(k) = &self.key {
            un = un.with_key(k);
        }
        un
    }
}

#[derive(Clone, Default)]
pub struct SubscriptionRegistry {
    inner: Arc<DashMap<SubscriptionId, Arc<SubscriptionEntry>>>,
}

impl SubscriptionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a new subscription. The caller is responsible for actually
    /// telling the `ConnectionManager` to subscribe — this only tracks the id
    /// → (connection, view, key) mapping.
    pub fn insert(
        &self,
        connection_id: ConnectionId,
        view: String,
        key: Option<String>,
    ) -> Arc<SubscriptionEntry> {
        let id = Uuid::new_v4().simple().to_string();
        let entry = Arc::new(SubscriptionEntry {
            id: id.clone(),
            connection_id,
            view,
            key,
        });
        self.inner.insert(id, entry.clone());
        entry
    }

    pub fn get(&self, id: &str) -> Option<Arc<SubscriptionEntry>> {
        self.inner.get(id).map(|e| e.clone())
    }

    /// Remove and return the subscription, if present.
    pub fn remove(&self, id: &str) -> Option<Arc<SubscriptionEntry>> {
        self.inner.remove(id).map(|(_, e)| e)
    }

    /// All subscriptions, optionally filtered to a single connection.
    pub fn list(&self, connection_id: Option<&str>) -> Vec<Arc<SubscriptionEntry>> {
        self.inner
            .iter()
            .filter(|e| connection_id.is_none_or(|cid| e.value().connection_id == cid))
            .map(|e| e.value().clone())
            .collect()
    }

    /// Drop every subscription for a given connection (used on disconnect).
    pub fn remove_for_connection(&self, connection_id: &str) {
        self.inner
            .retain(|_, entry| entry.connection_id != connection_id);
    }
}
