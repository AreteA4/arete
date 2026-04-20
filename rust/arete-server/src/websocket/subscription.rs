use serde::{Deserialize, Serialize};

use crate::websocket::auth::AuthDeny;

/// Client message types for subscription management
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    /// Subscribe to a view
    Subscribe(Subscription),
    /// Unsubscribe from a view
    Unsubscribe(Unsubscription),
    /// Keep-alive ping (no response needed)
    Ping,
    /// Refresh authentication token without reconnecting
    RefreshAuth(RefreshAuthRequest),
}

/// Request to refresh authentication token
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefreshAuthRequest {
    pub token: String,
}

/// Response to a refresh auth request
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshAuthResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
}

/// Server-sent socket issue payload for auth and quota failures after connect.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SocketIssueMessage {
    #[serde(rename = "type")]
    pub kind: String,
    pub error: String,
    pub message: String,
    pub code: String,
    pub retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggested_action: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docs_url: Option<String>,
    pub fatal: bool,
}

impl SocketIssueMessage {
    pub fn from_auth_deny(deny: &AuthDeny, fatal: bool) -> Self {
        let response = deny.to_error_response();
        Self {
            kind: "error".to_string(),
            error: response.error,
            message: response.message,
            code: response.code,
            retryable: response.retryable,
            retry_after: response.retry_after,
            suggested_action: response.suggested_action,
            docs_url: response.docs_url,
            fatal,
        }
    }
}

/// Client subscription to a specific view
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Subscription {
    pub view: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub partition: Option<String>,
    /// Number of items to return (for windowed subscriptions)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub take: Option<usize>,
    /// Number of items to skip (for windowed subscriptions)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skip: Option<usize>,
    /// Whether to include initial snapshot (defaults to true for backward compatibility)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub with_snapshot: Option<bool>,
    /// Cursor for resuming from a specific point (_seq value).
    /// Note: Ignored for State mode subscriptions (single entity, no pagination).
    /// Note: Not supported for derived views (windowed aggregations with sort). Derived views
    /// always emit `seq: None` in live update frames, so cursor-based reconnection is unavailable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
    /// Maximum number of entities to include in snapshot (pagination hint).
    /// Note: Ignored for State mode subscriptions (single entity).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_limit: Option<usize>,
}

/// Client unsubscription request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Unsubscription {
    pub view: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
}

impl Unsubscription {
    /// Generate the subscription key used for tracking
    pub fn sub_key(&self) -> String {
        match &self.key {
            Some(k) => format!("{}:{}", self.view, k),
            None => format!("{}:*", self.view),
        }
    }
}

impl Subscription {
    pub fn matches_view(&self, view_id: &str) -> bool {
        self.view == view_id
    }

    pub fn matches_key(&self, key: &str) -> bool {
        self.key.as_ref().is_none_or(|k| k == key)
    }

    pub fn matches(&self, view_id: &str, key: &str) -> bool {
        self.matches_view(view_id) && self.matches_key(key)
    }

    pub fn sub_key(&self) -> String {
        match &self.key {
            Some(k) => format!("{}:{}", self.view, k),
            None => format!("{}:*", self.view),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::websocket::auth::{AuthDeny, AuthErrorCode};
    use serde_json::json;

    #[test]
    fn test_subscription_parse() {
        let json = json!({
            "view": "SettlementGame/list",
            "key": "835"
        });

        let sub: Subscription = serde_json::from_value(json).unwrap();
        assert_eq!(sub.view, "SettlementGame/list");
        assert_eq!(sub.key, Some("835".to_string()));
    }

    #[test]
    fn test_subscription_no_key() {
        let json = json!({
            "view": "SettlementGame/list"
        });

        let sub: Subscription = serde_json::from_value(json).unwrap();
        assert_eq!(sub.view, "SettlementGame/list");
        assert!(sub.key.is_none());
    }

    #[test]
    fn test_subscription_matches() {
        let sub = Subscription {
            view: "SettlementGame/list".to_string(),
            key: Some("835".to_string()),
            partition: None,
            take: None,
            skip: None,
            with_snapshot: None,
            after: None,
            snapshot_limit: None,
        };

        assert!(sub.matches("SettlementGame/list", "835"));
        assert!(!sub.matches("SettlementGame/list", "836"));
        assert!(!sub.matches("SettlementGame/state", "835"));
    }

    #[test]
    fn test_subscription_matches_all_keys() {
        let sub = Subscription {
            view: "SettlementGame/list".to_string(),
            key: None,
            partition: None,
            take: None,
            skip: None,
            with_snapshot: None,
            after: None,
            snapshot_limit: None,
        };

        assert!(sub.matches("SettlementGame/list", "835"));
        assert!(sub.matches("SettlementGame/list", "836"));
        assert!(!sub.matches("SettlementGame/state", "835"));
    }

    #[test]
    fn test_client_message_subscribe_parse() {
        let json = json!({
            "type": "subscribe",
            "view": "SettlementGame/list",
            "key": "835"
        });

        let msg: ClientMessage = serde_json::from_value(json).unwrap();
        match msg {
            ClientMessage::Subscribe(sub) => {
                assert_eq!(sub.view, "SettlementGame/list");
                assert_eq!(sub.key, Some("835".to_string()));
            }
            _ => panic!("Expected Subscribe"),
        }
    }

    #[test]
    fn test_client_message_unsubscribe_parse() {
        let json = json!({
            "type": "unsubscribe",
            "view": "SettlementGame/list",
            "key": "835"
        });

        let msg: ClientMessage = serde_json::from_value(json).unwrap();
        match msg {
            ClientMessage::Unsubscribe(unsub) => {
                assert_eq!(unsub.view, "SettlementGame/list");
                assert_eq!(unsub.key, Some("835".to_string()));
            }
            _ => panic!("Expected Unsubscribe"),
        }
    }

    #[test]
    fn test_client_message_ping_parse() {
        let json = json!({ "type": "ping" });

        let msg: ClientMessage = serde_json::from_value(json).unwrap();
        assert!(matches!(msg, ClientMessage::Ping));
    }

    #[test]
    fn test_client_message_refresh_auth_parse() {
        let json = json!({
            "type": "refresh_auth",
            "token": "new_token_here"
        });

        let msg: ClientMessage = serde_json::from_value(json).unwrap();
        match msg {
            ClientMessage::RefreshAuth(req) => {
                assert_eq!(req.token, "new_token_here");
            }
            _ => panic!("Expected RefreshAuth"),
        }
    }

    #[test]
    fn test_legacy_subscription_parse_as_subscribe() {
        let json = json!({
            "view": "SettlementGame/list",
            "key": "835"
        });

        let sub: Subscription = serde_json::from_value(json).unwrap();
        assert_eq!(sub.view, "SettlementGame/list");
        assert_eq!(sub.key, Some("835".to_string()));
    }

    #[test]
    fn test_sub_key_with_key() {
        let sub = Subscription {
            view: "SettlementGame/list".to_string(),
            key: Some("835".to_string()),
            partition: None,
            take: None,
            skip: None,
            with_snapshot: None,
            after: None,
            snapshot_limit: None,
        };
        assert_eq!(sub.sub_key(), "SettlementGame/list:835");
    }

    #[test]
    fn test_sub_key_without_key() {
        let sub = Subscription {
            view: "SettlementGame/list".to_string(),
            key: None,
            partition: None,
            take: None,
            skip: None,
            with_snapshot: None,
            after: None,
            snapshot_limit: None,
        };
        assert_eq!(sub.sub_key(), "SettlementGame/list:*");
    }

    #[test]
    fn test_unsubscription_sub_key() {
        let unsub = Unsubscription {
            view: "SettlementGame/list".to_string(),
            key: Some("835".to_string()),
        };
        assert_eq!(unsub.sub_key(), "SettlementGame/list:835");

        let unsub_all = Unsubscription {
            view: "SettlementGame/list".to_string(),
            key: None,
        };
        assert_eq!(unsub_all.sub_key(), "SettlementGame/list:*");
    }

    #[test]
    fn test_subscription_with_take_skip() {
        let json = json!({
            "type": "subscribe",
            "view": "OreRound/latest",
            "take": 10,
            "skip": 0
        });

        let msg: ClientMessage = serde_json::from_value(json).unwrap();
        match msg {
            ClientMessage::Subscribe(sub) => {
                assert_eq!(sub.view, "OreRound/latest");
                assert_eq!(sub.take, Some(10));
                assert_eq!(sub.skip, Some(0));
            }
            _ => panic!("Expected Subscribe"),
        }
    }

    #[test]
    fn test_subscription_with_snapshot() {
        let json = json!({
            "type": "subscribe",
            "view": "SettlementGame/list",
            "withSnapshot": true
        });

        let msg: ClientMessage = serde_json::from_value(json).unwrap();
        match msg {
            ClientMessage::Subscribe(sub) => {
                assert_eq!(sub.with_snapshot, Some(true));
            }
            _ => panic!("Expected Subscribe"),
        }
    }

    #[test]
    fn test_subscription_with_partition() {
        let json = json!({
            "type": "subscribe",
            "view": "SettlementGame/list",
            "partition": "mainnet"
        });

        let msg: ClientMessage = serde_json::from_value(json).unwrap();
        match msg {
            ClientMessage::Subscribe(sub) => {
                assert_eq!(sub.partition, Some("mainnet".to_string()));
            }
            _ => panic!("Expected Subscribe"),
        }
    }

    #[test]
    fn test_subscription_with_after() {
        let json = json!({
            "type": "subscribe",
            "view": "SettlementGame/list",
            "after": "12345"
        });

        let msg: ClientMessage = serde_json::from_value(json).unwrap();
        match msg {
            ClientMessage::Subscribe(sub) => {
                assert_eq!(sub.after, Some("12345".to_string()));
            }
            _ => panic!("Expected Subscribe"),
        }
    }

    #[test]
    fn test_subscription_with_snapshot_limit() {
        let json = json!({
            "type": "subscribe",
            "view": "SettlementGame/list",
            "snapshotLimit": 100
        });

        let msg: ClientMessage = serde_json::from_value(json).unwrap();
        match msg {
            ClientMessage::Subscribe(sub) => {
                assert_eq!(sub.snapshot_limit, Some(100));
            }
            _ => panic!("Expected Subscribe"),
        }
    }

    #[test]
    fn test_socket_issue_message_from_auth_deny() {
        let deny = AuthDeny::new(
            AuthErrorCode::SubscriptionLimitExceeded,
            "subscription limit exceeded",
        )
        .with_suggested_action("unsubscribe first");

        let issue = SocketIssueMessage::from_auth_deny(&deny, false);
        assert_eq!(issue.kind, "error");
        assert_eq!(issue.code, "subscription-limit-exceeded");
        assert_eq!(issue.suggested_action.as_deref(), Some("unsubscribe first"));
        assert!(!issue.fatal);
    }
}
