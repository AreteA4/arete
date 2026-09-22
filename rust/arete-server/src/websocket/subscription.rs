use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::websocket::auth::AuthDeny;

pub const PROTOCOL_VERSION: u8 = 2;
pub const MAX_SUBSCRIPTION_ID_BYTES: usize = 128;

/// Client message types for subscription management.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    Subscribe(Subscription),
    Unsubscribe(Unsubscription),
    Ping,
    RefreshAuth(RefreshAuthRequest),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RefreshAuthRequest {
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshAuthResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
}

/// Server-sent protocol, authorization, or quota error.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SocketIssueMessage {
    #[serde(rename = "type")]
    pub kind: String,
    pub protocol_version: u8,
    /// Null only when a request did not contain a usable subscription ID.
    pub subscription_id: Option<String>,
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
    /// Present on `cursor-expired` and `invalid-cursor`: the offsets the view
    /// can still serve.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replay_window: Option<crate::journal::ReplayWindow>,
    /// Present on `replay-lagged`: the cursor for the last record delivered
    /// *before* the gap. Resubscribing with `after` set to this replays the
    /// skipped records, provided they are still retained.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recover_from: Option<String>,
    pub fatal: bool,
}

impl SocketIssueMessage {
    pub fn from_auth_deny(deny: &AuthDeny, fatal: bool, subscription_id: Option<String>) -> Self {
        let response = deny.to_error_response();
        Self {
            kind: "error".to_string(),
            protocol_version: PROTOCOL_VERSION,
            subscription_id,
            error: response.error,
            message: response.message,
            code: response.code,
            retryable: response.retryable,
            retry_after: response.retry_after,
            suggested_action: response.suggested_action,
            docs_url: response.docs_url,
            replay_window: None,
            recover_from: None,
            fatal,
        }
    }

    pub fn protocol(
        subscription_id: Option<String>,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        let code = code.into();
        Self {
            kind: "error".to_string(),
            protocol_version: PROTOCOL_VERSION,
            subscription_id,
            error: code.clone(),
            message: message.into(),
            code,
            retryable: false,
            retry_after: None,
            suggested_action: None,
            docs_url: None,
            replay_window: None,
            recover_from: None,
            fatal: false,
        }
    }

    /// Delivery fell behind the server's fan-out buffer and records were
    /// skipped.
    ///
    /// `recover_from` is the cursor for the last record delivered *before*
    /// the gap, not the latest offset seen: resubscribing after the newest
    /// frame would step over the skipped records permanently. Delivery on
    /// this subscription stops here, so nothing arrives that could advance
    /// the consumer's checkpoint past the gap.
    pub fn replay_lagged(
        subscription_id: Option<String>,
        skipped: u64,
        recover_from: Option<crate::journal::Cursor>,
    ) -> Self {
        let mut issue = Self::protocol(
            subscription_id,
            "replay-lagged",
            format!("delivery fell behind by {skipped} records and this subscription has stopped"),
        );
        issue.suggested_action = Some(match &recover_from {
            Some(cursor) => format!(
                "unsubscribe, then resubscribe with after set to {cursor} to replay the skipped records"
            ),
            None => "unsubscribe, then resubscribe without `after` to replay the retained window"
                .to_string(),
        });
        issue.recover_from = recover_from.map(|cursor| cursor.to_string());
        issue
    }

    /// A replay the journal refused, rendered for the client.
    ///
    /// Each reason gets its own code: they have different remediations, and
    /// an epoch mismatch in particular means the consumer's whole notion of
    /// the stream is stale rather than merely out of date.
    pub fn replay_refused(
        subscription_id: Option<String>,
        error: &crate::journal::ReplayError,
    ) -> Self {
        use crate::journal::ReplayError;

        let window = error.window().clone();
        let (code, message, action) = match error {
            ReplayError::EpochMismatch(_) => (
                "cursor-epoch-changed",
                "cursor was issued by a previous journal lifetime and its offsets do not apply here"
                    .to_string(),
                "discard the cursor and resubscribe without `after`",
            ),
            ReplayError::CursorExpired(window) => (
                "cursor-expired",
                format!(
                    "cursor is older than the retained replay window; the oldest retained record is at offset {}",
                    window.earliest
                ),
                "resubscribe without `after` to replay the whole retained window",
            ),
            ReplayError::CursorBeyondWindow(window) => (
                "cursor-unknown",
                format!(
                    "cursor is beyond this view's latest offset; the next record will be at offset {}",
                    window.next
                ),
                "resubscribe without `after`, or with a cursor this view has issued",
            ),
            ReplayError::GapCrossed(window) => (
                "replay-gap",
                format!(
                    "records after offset {} were lost when the stream restarted live; replaying across the hole would present it as continuous",
                    window.gap_after.unwrap_or(window.earliest)
                ),
                "resubscribe without `after`, accepting the gap, or from a cursor after it",
            ),
        };

        let mut issue = Self::protocol(subscription_id, code, message);
        issue.suggested_action = Some(action.to_string());
        issue.replay_window = Some(window);
        issue
    }
}

/// Canonical query identity and behavior for protocol v2.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
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
    pub fn matches_key(&self, key: &str) -> bool {
        self.key.as_ref().is_none_or(|expected| expected == key)
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        if self.view.trim().is_empty() {
            return Err("query.view must not be empty");
        }
        if self.take == Some(0) {
            return Err("query.take must be greater than zero");
        }
        if self.snapshot_limit == Some(0) {
            return Err("query.snapshotLimit must be greater than zero");
        }
        if self
            .filters
            .keys()
            .any(|path| path.is_empty() || path.split('.').any(|segment| segment.is_empty()))
        {
            return Err("query filter paths must contain non-empty dot-path segments");
        }
        Ok(())
    }
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
    pub fn validate(&self) -> Result<(), &'static str> {
        validate_protocol(self.protocol_version)?;
        validate_subscription_id(&self.subscription_id)?;
        self.query.validate()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Unsubscription {
    pub protocol_version: u8,
    pub subscription_id: String,
}

impl Unsubscription {
    pub fn validate(&self) -> Result<(), &'static str> {
        validate_protocol(self.protocol_version)?;
        validate_subscription_id(&self.subscription_id)
    }
}

pub fn validate_protocol(version: u8) -> Result<(), &'static str> {
    if version == PROTOCOL_VERSION {
        Ok(())
    } else {
        Err("protocolVersion must be 2")
    }
}

pub fn validate_subscription_id(subscription_id: &str) -> Result<(), &'static str> {
    if subscription_id.is_empty() {
        return Err("subscriptionId must not be empty");
    }
    if subscription_id.len() > MAX_SUBSCRIPTION_ID_BYTES {
        return Err("subscriptionId exceeds 128 bytes");
    }
    if subscription_id.trim() != subscription_id || subscription_id.chars().any(char::is_control) {
        return Err("subscriptionId must be opaque non-whitespace text without control characters");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::websocket::auth::{AuthDeny, AuthErrorCode};
    use serde_json::json;

    fn valid_subscription() -> Value {
        json!({
            "type": "subscribe",
            "protocolVersion": 2,
            "subscriptionId": "rounds:page-1",
            "query": {
                "view": "OreRound/latest",
                "filters": {"state.status": "open"},
                "take": 10,
                "skip": 0,
                "after": "100:000000000001",
                "snapshotLimit": 10
            },
            "snapshot": {"enabled": true}
        })
    }

    #[test]
    fn parses_canonical_v2_subscription() {
        let message: ClientMessage = serde_json::from_value(valid_subscription()).unwrap();
        let ClientMessage::Subscribe(subscription) = message else {
            panic!("expected subscribe");
        };

        assert!(subscription.validate().is_ok());
        assert_eq!(subscription.subscription_id, "rounds:page-1");
        assert_eq!(subscription.query.filters["state.status"], "open");
        assert!(subscription.snapshot.enabled);
    }

    #[test]
    fn rejects_legacy_and_unknown_subscription_fields() {
        assert!(serde_json::from_value::<ClientMessage>(json!({
            "type": "subscribe",
            "view": "OreRound/latest"
        }))
        .is_err());

        let mut value = valid_subscription();
        value["withSnapshot"] = json!(true);
        assert!(serde_json::from_value::<ClientMessage>(value).is_err());
    }

    #[test]
    fn validates_protocol_and_opaque_id() {
        let mut value = valid_subscription();
        value["protocolVersion"] = json!(1);
        let ClientMessage::Subscribe(subscription) =
            serde_json::from_value::<ClientMessage>(value).unwrap()
        else {
            panic!("expected subscribe");
        };
        assert_eq!(subscription.validate(), Err("protocolVersion must be 2"));

        assert!(validate_subscription_id("client-selected.id:1").is_ok());
        assert!(validate_subscription_id("").is_err());
        assert!(validate_subscription_id(" leading").is_err());
        assert!(validate_subscription_id(&"x".repeat(129)).is_err());
    }

    #[test]
    fn parses_id_only_unsubscribe() {
        let message: ClientMessage = serde_json::from_value(json!({
            "type": "unsubscribe",
            "protocolVersion": 2,
            "subscriptionId": "rounds:page-1"
        }))
        .unwrap();
        let ClientMessage::Unsubscribe(unsubscription) = message else {
            panic!("expected unsubscribe");
        };
        assert!(unsubscription.validate().is_ok());
    }

    #[test]
    fn socket_issue_carries_protocol_and_subscription_identity() {
        let deny = AuthDeny::new(
            AuthErrorCode::SubscriptionLimitExceeded,
            "subscription limit exceeded",
        );
        let issue =
            SocketIssueMessage::from_auth_deny(&deny, false, Some("rounds:page-1".to_string()));
        assert_eq!(issue.protocol_version, PROTOCOL_VERSION);
        assert_eq!(issue.subscription_id.as_deref(), Some("rounds:page-1"));
        assert_eq!(issue.code, "subscription-limit-exceeded");
    }
}
