use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ClientMessage {
    Subscribe(Subscription),
    Unsubscribe(Unsubscription),
    Ping,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Subscription {
    pub view: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub partition: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filters: Option<HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub take: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skip: Option<u32>,
    /// Whether to include initial snapshot (defaults to true for backward compatibility)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub with_snapshot: Option<bool>,
    /// Cursor for resuming from a specific point (_seq value)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
    /// Maximum number of entities to include in snapshot (pagination hint)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Unsubscription {
    pub view: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
}

impl Unsubscription {
    pub fn new(view: impl Into<String>) -> Self {
        Self {
            view: view.into(),
            key: None,
        }
    }

    pub fn with_key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }

    pub fn sub_key(&self) -> String {
        format!("{}:{}", self.view, self.key.as_deref().unwrap_or("*"),)
    }
}

impl From<&Subscription> for Unsubscription {
    fn from(sub: &Subscription) -> Self {
        Self {
            view: sub.view.clone(),
            key: sub.key.clone(),
        }
    }
}

impl Subscription {
    pub fn new(view: impl Into<String>) -> Self {
        Self {
            view: view.into(),
            key: None,
            partition: None,
            filters: None,
            take: None,
            skip: None,
            with_snapshot: None,
            after: None,
            snapshot_limit: None,
        }
    }

    pub fn with_key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }

    pub fn with_filters(mut self, filters: HashMap<String, String>) -> Self {
        self.filters = Some(filters);
        self
    }

    pub fn with_take(mut self, take: u32) -> Self {
        self.take = Some(take);
        self
    }

    pub fn with_skip(mut self, skip: u32) -> Self {
        self.skip = Some(skip);
        self
    }

    /// Set whether to include the initial snapshot (defaults to true)
    pub fn with_snapshot(mut self, with_snapshot: bool) -> Self {
        self.with_snapshot = Some(with_snapshot);
        self
    }

    /// Set the cursor to resume from (for reconnecting and getting only newer data)
    pub fn after(mut self, cursor: impl Into<String>) -> Self {
        self.after = Some(cursor.into());
        self
    }

    /// Set the maximum number of entities to include in the snapshot
    pub fn with_snapshot_limit(mut self, limit: usize) -> Self {
        self.snapshot_limit = Some(limit);
        self
    }

    pub fn sub_key(&self) -> String {
        let filters_str = self
            .filters
            .as_ref()
            .map(|f| serde_json::to_string(f).unwrap_or_default())
            .unwrap_or_default();
        format!(
            "{}:{}:{}:{}",
            self.view,
            self.key.as_deref().unwrap_or("*"),
            self.partition.as_deref().unwrap_or(""),
            filters_str
        )
    }
}

#[derive(Debug, Default)]
pub struct SubscriptionRegistry {
    subscriptions: HashMap<String, Subscription>,
}

impl SubscriptionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, sub: Subscription) {
        let key = sub.sub_key();
        self.subscriptions.insert(key, sub);
    }

    pub fn remove(&mut self, sub: &Subscription) {
        let key = sub.sub_key();
        self.subscriptions.remove(&key);
    }

    pub fn contains(&self, sub: &Subscription) -> bool {
        let key = sub.sub_key();
        self.subscriptions.contains_key(&key)
    }

    pub fn all(&self) -> Vec<Subscription> {
        self.subscriptions.values().cloned().collect()
    }

    #[allow(dead_code)]
    pub fn clear(&mut self) {
        self.subscriptions.clear();
    }
}
