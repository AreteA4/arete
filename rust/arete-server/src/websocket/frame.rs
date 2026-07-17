use serde::{Deserialize, Serialize};

/// Streaming mode for different data access patterns
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Latest value only (watch semantics)
    State,
    /// Append-only stream
    Append,
    /// Collection/list view (also used for key-value lookups)
    List,
}

/// Sort order for sorted views
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SortOrder {
    Asc,
    Desc,
}

/// Sort configuration for a view
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SortConfig {
    /// Field path to sort by (e.g., ["id", "roundId"])
    pub field: Vec<String>,
    /// Sort order
    pub order: SortOrder,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WireFormat {
    pub wide_int_paths: Vec<Vec<String>>,
}

/// Subscription acknowledgment frame sent when a client subscribes
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscribedFrame {
    /// Operation type - always "subscribed"
    pub op: &'static str,
    /// The view that was subscribed to
    pub view: String,
    /// Streaming mode for this view
    pub mode: Mode,
    /// Sort configuration if this is a sorted view
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort: Option<SortConfig>,
}

impl SubscribedFrame {
    pub fn new(view: String, mode: Mode, sort: Option<SortConfig>) -> Self {
        Self {
            op: "subscribed",
            view,
            mode,
            sort,
        }
    }
}

/// Data frame sent over WebSocket
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Frame {
    pub mode: Mode,
    #[serde(rename = "entity")]
    pub export: String,
    pub op: &'static str,
    pub key: String,
    pub data: serde_json::Value,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub append: Vec<String>,
    /// Sequence cursor for ordering and resume capability
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seq: Option<String>,
}

/// A single entity within a snapshot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotEntity {
    pub key: String,
    pub data: serde_json::Value,
}

/// Batch snapshot frame for initial data load
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotFrame {
    pub mode: Mode,
    #[serde(rename = "entity")]
    pub export: String,
    pub op: &'static str,
    /// Subscription key that requested this snapshot. Omitted for keyless subscriptions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    pub data: Vec<SnapshotEntity>,
    /// Indicates whether this is the final snapshot batch.
    /// When `false`, more snapshot batches will follow.
    /// When `true`, the snapshot is complete and live streaming begins.
    #[serde(default = "default_complete")]
    pub complete: bool,
}

fn default_complete() -> bool {
    true
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

impl Frame {
    pub fn entity(&self) -> &str {
        &self.export
    }

    pub fn key(&self) -> &str {
        &self.key
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_frame_entity_key_accessors() {
        let frame = Frame {
            mode: Mode::List,
            export: "SettlementGame/list".to_string(),
            op: "upsert",
            key: "123".to_string(),
            data: serde_json::json!({}),
            append: vec![],
            seq: None,
        };

        assert_eq!(frame.entity(), "SettlementGame/list");
        assert_eq!(frame.key(), "123");
    }

    #[test]
    fn test_frame_serialization() {
        let frame = Frame {
            mode: Mode::List,
            export: "SettlementGame/list".to_string(),
            op: "upsert",
            key: "123".to_string(),
            data: serde_json::json!({"gameId": "123"}),
            append: vec![],
            seq: None,
        };

        let json = serde_json::to_value(&frame).unwrap();
        assert_eq!(json["op"], "upsert");
        assert_eq!(json["mode"], "list");
        assert_eq!(json["entity"], "SettlementGame/list");
        assert_eq!(json["key"], "123");
    }

    #[test]
    fn test_frame_with_seq() {
        let frame = Frame {
            mode: Mode::List,
            export: "SettlementGame/list".to_string(),
            op: "upsert",
            key: "123".to_string(),
            data: serde_json::json!({"gameId": "123"}),
            append: vec![],
            seq: Some("123456789:000000000042".to_string()),
        };

        let json = serde_json::to_value(&frame).unwrap();
        assert_eq!(json["op"], "upsert");
        assert_eq!(json["seq"], "123456789:000000000042");
    }

    #[test]
    fn test_frame_seq_skipped_when_none() {
        let frame = Frame {
            mode: Mode::List,
            export: "SettlementGame/list".to_string(),
            op: "upsert",
            key: "123".to_string(),
            data: serde_json::json!({"gameId": "123"}),
            append: vec![],
            seq: None,
        };

        let json = serde_json::to_value(&frame).unwrap();
        assert!(json.get("seq").is_none());
    }

    #[test]
    fn test_snapshot_frame_complete_serialization() {
        let frame = SnapshotFrame {
            mode: Mode::List,
            export: "tokens/list".to_string(),
            op: "snapshot",
            key: Some("owner-1".to_string()),
            data: vec![SnapshotEntity {
                key: "abc".to_string(),
                data: serde_json::json!({"id": "abc"}),
            }],
            complete: false,
        };

        let json = serde_json::to_value(&frame).unwrap();
        assert_eq!(json["complete"], false);
        assert_eq!(json["op"], "snapshot");
        assert_eq!(json["key"], "owner-1");
    }

    #[test]
    fn test_snapshot_frame_complete_defaults_to_true_on_deserialize() {
        #[derive(Debug, Deserialize)]
        struct TestSnapshotFrame {
            #[allow(dead_code)]
            mode: Mode,
            #[allow(dead_code)]
            #[serde(rename = "entity")]
            export: String,
            #[allow(dead_code)]
            op: String,
            #[serde(default)]
            key: Option<String>,
            #[allow(dead_code)]
            data: Vec<SnapshotEntity>,
            #[serde(default = "super::default_complete")]
            complete: bool,
        }

        let json_without_complete = serde_json::json!({
            "mode": "list",
            "entity": "tokens/list",
            "op": "snapshot",
            "data": []
        });

        let frame: TestSnapshotFrame = serde_json::from_value(json_without_complete).unwrap();
        assert!(frame.complete);
        assert!(frame.key.is_none());
    }

    #[test]
    fn test_snapshot_frame_batching_fields() {
        let first_batch = SnapshotFrame {
            mode: Mode::List,
            export: "tokens/list".to_string(),
            op: "snapshot",
            key: None,
            data: vec![],
            complete: false,
        };

        let final_batch = SnapshotFrame {
            mode: Mode::List,
            export: "tokens/list".to_string(),
            op: "snapshot",
            key: None,
            data: vec![],
            complete: true,
        };

        assert!(!first_batch.complete);
        assert!(final_batch.complete);
        assert!(serde_json::to_value(final_batch)
            .unwrap()
            .get("key")
            .is_none());
    }

    #[test]
    fn wire_format_stringifies_marked_wide_int_paths() {
        let wire_format = WireFormat {
            wide_int_paths: vec![
                vec!["amount".to_string()],
                vec!["nested".to_string(), "timestamp".to_string()],
                vec!["values".to_string()],
                vec!["positions".to_string(), "liquidity".to_string()],
            ],
        };

        let mut value = serde_json::json!({
            "amount": 42,
            "nested": { "timestamp": -7 },
            "values": [1, 2, 3],
            "positions": [
                { "liquidity": 9, "label": "a" },
                { "liquidity": 11, "label": "b" }
            ],
            "small": 5,
        });

        apply_wire_format(&mut value, &wire_format);

        assert_eq!(value["amount"], serde_json::Value::String("42".to_string()));
        assert_eq!(
            value["nested"]["timestamp"],
            serde_json::Value::String("-7".to_string())
        );
        assert_eq!(
            value["values"][0],
            serde_json::Value::String("1".to_string())
        );
        assert_eq!(
            value["positions"][1]["liquidity"],
            serde_json::Value::String("11".to_string())
        );
        assert_eq!(value["small"], serde_json::json!(5));
    }
}
