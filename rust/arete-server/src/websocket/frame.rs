use serde::{Deserialize, Serialize};

use crate::websocket::subscription::{SubscriptionQuery, PROTOCOL_VERSION};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    State,
    Append,
    List,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SortOrder {
    Asc,
    Desc,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SortConfig {
    pub field: Vec<String>,
    pub order: SortOrder,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WireFormat {
    pub wide_int_paths: Vec<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubscribedFrame {
    pub protocol_version: u8,
    pub subscription_id: String,
    pub op: &'static str,
    pub query: SubscriptionQuery,
    pub mode: Mode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort: Option<SortConfig>,
}

impl SubscribedFrame {
    pub fn new(
        subscription_id: String,
        query: SubscriptionQuery,
        mode: Mode,
        sort: Option<SortConfig>,
    ) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            subscription_id,
            op: "subscribed",
            query,
            mode,
            sort,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnsubscribedFrame {
    pub protocol_version: u8,
    pub subscription_id: String,
    pub op: &'static str,
}

impl UnsubscribedFrame {
    pub fn new(subscription_id: String) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            subscription_id,
            op: "unsubscribed",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Frame {
    pub protocol_version: u8,
    pub subscription_id: String,
    pub mode: Mode,
    #[serde(rename = "entity")]
    pub export: String,
    pub op: String,
    pub key: String,
    pub data: serde_json::Value,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub append: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seq: Option<String>,
}

impl Frame {
    pub fn scoped(
        subscription_id: impl Into<String>,
        mode: Mode,
        export: impl Into<String>,
        op: impl Into<String>,
        key: impl Into<String>,
        data: serde_json::Value,
        seq: Option<String>,
    ) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            subscription_id: subscription_id.into(),
            mode,
            export: export.into(),
            op: op.into(),
            key: key.into(),
            data,
            append: vec![],
            seq,
        }
    }

    pub fn entity(&self) -> &str {
        &self.export
    }

    pub fn key(&self) -> &str {
        &self.key
    }
}

/// Unscoped payload published by the projector and never sent directly to clients.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct SourceFrame {
    pub mode: Mode,
    #[serde(rename = "entity")]
    pub export: String,
    pub op: &'static str,
    pub key: String,
    pub data: serde_json::Value,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub append: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seq: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SnapshotEntity {
    pub key: String,
    pub data: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotFrame {
    pub protocol_version: u8,
    pub subscription_id: String,
    pub snapshot_id: String,
    pub authoritative: bool,
    pub mode: Mode,
    #[serde(rename = "entity")]
    pub export: String,
    pub op: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    pub data: Vec<SnapshotEntity>,
    pub complete: bool,
}

pub fn apply_wire_format(value: &mut serde_json::Value, wire_format: &WireFormat) {
    for path in &wire_format.wide_int_paths {
        stringify_value_at_path(value, path);
    }
}

fn stringify_value_at_path(value: &mut serde_json::Value, path: &[String]) {
    if path.is_empty() {
        stringify_wide_int_value(value);
        return;
    }

    match value {
        serde_json::Value::Object(map) => {
            if let Some(child) = map.get_mut(&path[0]) {
                stringify_value_at_path(child, &path[1..]);
            }
        }
        serde_json::Value::Array(values) => {
            for child in values {
                stringify_value_at_path(child, path);
            }
        }
        _ => {}
    }
}

fn stringify_wide_int_value(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Number(number) => {
            if let Some(unsigned) = number.as_u64() {
                *value = serde_json::Value::String(unsigned.to_string());
            } else if let Some(signed) = number.as_i64() {
                *value = serde_json::Value::String(signed.to_string());
            }
        }
        serde_json::Value::Array(values) => {
            for child in values {
                stringify_wide_int_value(child);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn scoped_live_frames_carry_v2_identity() {
        let frame = Frame::scoped(
            "rounds:1",
            Mode::List,
            "OreRound/latest",
            "upsert",
            "123",
            json!({"id": 123}),
            Some("10:000000000001".to_string()),
        );
        let value = serde_json::to_value(frame).unwrap();
        assert_eq!(value["protocolVersion"], 2);
        assert_eq!(value["subscriptionId"], "rounds:1");
        assert_eq!(value["seq"], "10:000000000001");
    }

    #[test]
    fn snapshot_serializes_conformance_metadata() {
        let frame = SnapshotFrame {
            protocol_version: PROTOCOL_VERSION,
            subscription_id: "rounds:1".to_string(),
            snapshot_id: "snapshot-1".to_string(),
            authoritative: true,
            mode: Mode::List,
            export: "OreRound/latest".to_string(),
            op: "snapshot",
            key: None,
            data: vec![],
            complete: true,
        };
        let value = serde_json::to_value(frame).unwrap();
        assert_eq!(value["snapshotId"], "snapshot-1");
        assert_eq!(value["authoritative"], true);
        assert_eq!(value["complete"], true);
    }

    #[test]
    fn wire_format_stringifies_marked_wide_int_paths() {
        let wire_format = WireFormat {
            wide_int_paths: vec![
                vec!["amount".to_string()],
                vec!["positions".to_string(), "liquidity".to_string()],
            ],
        };
        let mut value = json!({
            "amount": 42,
            "positions": [{"liquidity": 9}, {"liquidity": 11}],
            "small": 5,
        });
        apply_wire_format(&mut value, &wire_format);
        assert_eq!(value["amount"], "42");
        assert_eq!(value["positions"][1]["liquidity"], "11");
        assert_eq!(value["small"], 5);
    }
}
