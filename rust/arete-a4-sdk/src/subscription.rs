use crate::error::AreteError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};

pub const PROTOCOL_VERSION: u8 = 2;
pub const MAX_SUBSCRIPTION_ID_BYTES: usize = 128;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubscriptionQuery {
    pub view: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub partition: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub filters: BTreeMap<String, Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub take: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skip: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_limit: Option<usize>,
}

impl SubscriptionQuery {
    pub fn new(view: impl Into<String>) -> Self {
        Self {
            view: view.into(),
            key: None,
            partition: None,
            filters: BTreeMap::new(),
            take: None,
            skip: None,
            after: None,
            snapshot_limit: None,
        }
    }

    pub fn with_key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }

    pub fn with_partition(mut self, partition: impl Into<String>) -> Self {
        self.partition = Some(partition.into());
        self
    }

    pub fn with_filter(mut self, path: impl Into<String>, value: impl Into<Value>) -> Self {
        self.filters.insert(path.into(), value.into());
        self
    }

    pub fn with_filters(mut self, filters: BTreeMap<String, Value>) -> Self {
        self.filters = filters;
        self
    }

    pub fn with_take(mut self, take: usize) -> Self {
        self.take = Some(take);
        self
    }

    pub fn with_skip(mut self, skip: usize) -> Self {
        self.skip = Some(skip);
        self
    }

    pub fn after(mut self, cursor: impl Into<String>) -> Self {
        self.after = Some(cursor.into());
        self
    }

    pub fn with_snapshot_limit(mut self, limit: usize) -> Self {
        self.snapshot_limit = Some(limit);
        self
    }

    pub fn validate(&self) -> Result<(), AreteError> {
        if self.view.trim().is_empty() {
            return Err(AreteError::SubscriptionFailed(
                "query.view must not be empty".to_string(),
            ));
        }
        if self.take == Some(0) {
            return Err(AreteError::SubscriptionFailed(
                "query.take must be greater than zero".to_string(),
            ));
        }
        if self.snapshot_limit == Some(0) {
            return Err(AreteError::SubscriptionFailed(
                "query.snapshotLimit must be greater than zero".to_string(),
            ));
        }
        if self
            .filters
            .keys()
            .any(|path| path.is_empty() || path.split('.').any(str::is_empty))
        {
            return Err(AreteError::SubscriptionFailed(
                "query filter paths must contain non-empty dot-path segments".to_string(),
            ));
        }
        Ok(())
    }

    /// Canonical v2 query identity. Struct field order matches the server and
    /// filters use a BTreeMap so equivalent insertion orders serialize equally.
    pub fn canonical_identity(&self) -> Result<String, AreteError> {
        self.validate()?;
        serde_json::to_string(self).map_err(AreteError::from)
    }
}

#[derive(Serialize)]
struct CanonicalSubscriptionIdentity<'a> {
    query: &'a SubscriptionQuery,
    snapshot: &'a SnapshotOptions,
}

pub(crate) fn canonical_subscription_identity(
    query: &SubscriptionQuery,
    snapshot: &SnapshotOptions,
) -> Result<String, AreteError> {
    query.validate()?;
    serde_json::to_string(&CanonicalSubscriptionIdentity { query, snapshot })
        .map_err(AreteError::from)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SnapshotOptions {
    #[serde(default = "default_snapshot_enabled")]
    pub enabled: bool,
}

impl Default for SnapshotOptions {
    fn default() -> Self {
        Self { enabled: true }
    }
}

fn default_snapshot_enabled() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Subscription {
    pub protocol_version: u8,
    pub subscription_id: String,
    pub query: SubscriptionQuery,
    #[serde(default)]
    pub snapshot: SnapshotOptions,
}

impl Subscription {
    pub fn new(subscription_id: impl Into<String>, query: SubscriptionQuery) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            subscription_id: subscription_id.into(),
            query,
            snapshot: SnapshotOptions::default(),
        }
    }

    pub fn with_snapshot(mut self, enabled: bool) -> Self {
        self.snapshot.enabled = enabled;
        self
    }

    pub fn validate(&self) -> Result<(), AreteError> {
        validate_protocol(self.protocol_version)?;
        validate_subscription_id(&self.subscription_id)?;
        self.query.validate()
    }

    /// Canonical protocol v2 identity, including snapshot behavior.
    pub fn canonical_identity(&self) -> Result<String, AreteError> {
        canonical_subscription_identity(&self.query, &self.snapshot)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Unsubscription {
    pub protocol_version: u8,
    pub subscription_id: String,
}

impl Unsubscription {
    pub fn new(subscription_id: impl Into<String>) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            subscription_id: subscription_id.into(),
        }
    }

    pub fn validate(&self) -> Result<(), AreteError> {
        validate_protocol(self.protocol_version)?;
        validate_subscription_id(&self.subscription_id)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    Subscribe(Subscription),
    Unsubscribe(Unsubscription),
    Ping,
    RefreshAuth { token: String },
}

pub fn validate_protocol(version: u8) -> Result<(), AreteError> {
    if version == PROTOCOL_VERSION {
        Ok(())
    } else {
        Err(AreteError::Protocol {
            message: format!(
                "unsupported WebSocket protocol version {version}; the Rust SDK requires protocol v2"
            ),
            subscription_id: None,
        })
    }
}

pub fn validate_subscription_id(subscription_id: &str) -> Result<(), AreteError> {
    let message = if subscription_id.is_empty() {
        Some("subscriptionId must not be empty")
    } else if subscription_id.len() > MAX_SUBSCRIPTION_ID_BYTES {
        Some("subscriptionId exceeds 128 bytes")
    } else if subscription_id.trim() != subscription_id
        || subscription_id.chars().any(char::is_control)
    {
        Some("subscriptionId must have no surrounding whitespace or control characters")
    } else {
        None
    };

    match message {
        Some(message) => Err(AreteError::SubscriptionFailed(message.to_string())),
        None => Ok(()),
    }
}

#[derive(Debug, Clone)]
struct ActiveSubscription {
    request: Subscription,
    ref_count: usize,
}

#[derive(Debug, Default)]
pub(crate) struct SubscriptionRegistry {
    by_identity: HashMap<String, ActiveSubscription>,
    identity_by_id: HashMap<String, String>,
    next_id: u64,
}

impl SubscriptionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn acquire(
        &mut self,
        query: SubscriptionQuery,
        snapshot: SnapshotOptions,
    ) -> Result<(Subscription, bool), AreteError> {
        let identity = canonical_subscription_identity(&query, &snapshot)?;
        if let Some(active) = self.by_identity.get_mut(&identity) {
            active.ref_count += 1;
            return Ok((active.request.clone(), false));
        }

        let subscription_id = loop {
            self.next_id += 1;
            let candidate = format!("rust-sdk:{:016x}", self.next_id);
            if !self.identity_by_id.contains_key(&candidate) {
                break candidate;
            }
        };
        let request = Subscription {
            protocol_version: PROTOCOL_VERSION,
            subscription_id: subscription_id.clone(),
            query,
            snapshot,
        };
        request.validate()?;
        self.identity_by_id
            .insert(subscription_id, identity.clone());
        self.by_identity.insert(
            identity,
            ActiveSubscription {
                request: request.clone(),
                ref_count: 1,
            },
        );
        Ok((request, true))
    }

    pub fn acquire_explicit(
        &mut self,
        request: Subscription,
    ) -> Result<(Subscription, bool), AreteError> {
        request.validate()?;
        let identity = request.canonical_identity()?;
        if self
            .identity_by_id
            .get(&request.subscription_id)
            .is_some_and(|existing| existing != &identity)
        {
            return Err(AreteError::SubscriptionFailed(format!(
                "subscriptionId '{}' is already active",
                request.subscription_id
            )));
        }
        if let Some(active) = self.by_identity.get_mut(&identity) {
            active.ref_count += 1;
            return Ok((active.request.clone(), false));
        }
        if self.identity_by_id.contains_key(&request.subscription_id) {
            return Err(AreteError::SubscriptionFailed(format!(
                "subscriptionId '{}' is already active",
                request.subscription_id
            )));
        }
        self.identity_by_id
            .insert(request.subscription_id.clone(), identity.clone());
        self.by_identity.insert(
            identity,
            ActiveSubscription {
                request: request.clone(),
                ref_count: 1,
            },
        );
        Ok((request, true))
    }

    pub fn release(&mut self, subscription_id: &str) -> Option<Unsubscription> {
        let identity = self.identity_by_id.get(subscription_id)?.clone();
        let active = self.by_identity.get_mut(&identity)?;
        active.ref_count -= 1;
        if active.ref_count > 0 {
            return None;
        }
        self.by_identity.remove(&identity);
        self.identity_by_id.remove(subscription_id);
        Some(Unsubscription::new(subscription_id))
    }

    pub fn remove(&mut self, subscription_id: &str) -> Option<Unsubscription> {
        let identity = self.identity_by_id.remove(subscription_id)?;
        self.by_identity.remove(&identity);
        Some(Unsubscription::new(subscription_id))
    }

    pub fn all(&self) -> Vec<Subscription> {
        self.by_identity
            .values()
            .map(|active| active.request.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn canonical_identity_uses_protocol_field_and_filter_order() {
        let mut first = SubscriptionQuery::new("Order/list")
            .with_filter("state.status", "open")
            .with_filter("market.symbol", "SOL")
            .with_take(10)
            .with_skip(0);
        let mut second = SubscriptionQuery::new("Order/list")
            .with_filter("market.symbol", "SOL")
            .with_filter("state.status", "open")
            .with_take(10)
            .with_skip(0);

        assert_eq!(
            first.canonical_identity().unwrap(),
            second.canonical_identity().unwrap()
        );
        assert_eq!(
            serde_json::to_value(&first).unwrap(),
            json!({
                "view": "Order/list",
                "filters": {"market.symbol": "SOL", "state.status": "open"},
                "take": 10,
                "skip": 0
            })
        );

        first.partition = Some("a".to_string());
        second.partition = Some("b".to_string());
        assert_ne!(
            first.canonical_identity().unwrap(),
            second.canonical_identity().unwrap()
        );
    }

    #[test]
    fn registry_refcounts_equivalent_queries_and_keeps_id() {
        let mut registry = SubscriptionRegistry::new();
        let first_query = SubscriptionQuery::new("Order/list")
            .with_filter("b", "two")
            .with_filter("a", "one");
        let second_query = SubscriptionQuery::new("Order/list")
            .with_filter("a", "one")
            .with_filter("b", "two");

        let (first, first_is_new) = registry
            .acquire(first_query, SnapshotOptions::default())
            .unwrap();
        let (second, second_is_new) = registry
            .acquire(second_query, SnapshotOptions::default())
            .unwrap();

        assert!(first_is_new);
        assert!(!second_is_new);
        assert_eq!(first.subscription_id, second.subscription_id);
        assert!(registry.release(&first.subscription_id).is_none());
        assert!(registry.release(&first.subscription_id).is_some());
    }

    #[test]
    fn snapshot_behavior_is_part_of_subscription_identity() {
        let query = SubscriptionQuery::new("Order/list").with_take(10);
        let with_snapshot = Subscription::new("orders:snapshot", query.clone());
        let without_snapshot = Subscription::new("orders:live", query.clone()).with_snapshot(false);

        assert_ne!(
            with_snapshot.canonical_identity().unwrap(),
            without_snapshot.canonical_identity().unwrap()
        );

        let mut registry = SubscriptionRegistry::new();
        let (first, first_is_new) = registry
            .acquire(query.clone(), SnapshotOptions { enabled: true })
            .unwrap();
        let (second, second_is_new) = registry
            .acquire(query, SnapshotOptions { enabled: false })
            .unwrap();
        assert!(first_is_new);
        assert!(second_is_new);
        assert_ne!(first.subscription_id, second.subscription_id);
    }

    #[test]
    fn explicit_ids_cannot_alias_another_query_or_collide_with_generated_ids() {
        let mut registry = SubscriptionRegistry::new();
        let explicit = Subscription::new(
            "rust-sdk:0000000000000001",
            SubscriptionQuery::new("Order/list"),
        );
        registry.acquire_explicit(explicit).unwrap();

        let (generated, is_new) = registry
            .acquire(
                SubscriptionQuery::new("Trade/list"),
                SnapshotOptions::default(),
            )
            .unwrap();
        assert!(is_new);
        assert_eq!(generated.subscription_id, "rust-sdk:0000000000000002");

        let collision = Subscription::new(
            "rust-sdk:0000000000000001",
            SubscriptionQuery::new("Trade/list"),
        );
        assert!(registry.acquire_explicit(collision).is_err());
    }

    #[test]
    fn subscribe_and_unsubscribe_are_v2_envelopes() {
        let subscribe = ClientMessage::Subscribe(Subscription::new(
            "orders:open",
            SubscriptionQuery::new("Order/list").with_take(10),
        ));
        assert_eq!(
            serde_json::to_value(subscribe).unwrap(),
            json!({
                "type": "subscribe",
                "protocolVersion": 2,
                "subscriptionId": "orders:open",
                "query": {"view": "Order/list", "take": 10},
                "snapshot": {"enabled": true}
            })
        );
        assert_eq!(
            serde_json::to_value(ClientMessage::Unsubscribe(Unsubscription::new(
                "orders:open"
            )))
            .unwrap(),
            json!({
                "type": "unsubscribe",
                "protocolVersion": 2,
                "subscriptionId": "orders:open"
            })
        );
    }
}
