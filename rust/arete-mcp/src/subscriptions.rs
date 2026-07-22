//! Subscription registry: tracks which views each connection is subscribed to.
//!
//! The SDK's `ConnectionManager` multiplexes subscriptions over one WebSocket
//! and reference-counts equivalent protocol v2 queries. This registry keeps
//! each SDK lease alive and maps MCP subscription IDs to the effective wire
//! subscription ID used by exact query-membership reads.

use std::sync::Arc;

use arete_sdk::SubscriptionLease;
use dashmap::DashMap;
use uuid::Uuid;

use crate::connections::ConnectionId;

pub type SubscriptionId = String;

pub struct SubscriptionEntry {
    pub id: SubscriptionId,
    pub connection_id: ConnectionId,
    pub view: String,
    pub key: Option<String>,
    pub wire_subscription_id: String,
    _lease: SubscriptionLease,
}

#[derive(Clone, Default)]
pub struct SubscriptionRegistry {
    inner: Arc<DashMap<SubscriptionId, Arc<SubscriptionEntry>>>,
}

impl SubscriptionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn next_id(&self) -> SubscriptionId {
        Uuid::new_v4().simple().to_string()
    }

    /// Register a subscribed SDK lease under its MCP-facing ID.
    pub fn insert(
        &self,
        id: SubscriptionId,
        connection_id: ConnectionId,
        view: String,
        key: Option<String>,
        lease: SubscriptionLease,
    ) -> Arc<SubscriptionEntry> {
        let wire_subscription_id = lease.subscription_id().to_string();
        let entry = Arc::new(SubscriptionEntry {
            id: id.clone(),
            connection_id,
            view,
            key,
            wire_subscription_id,
            _lease: lease,
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
