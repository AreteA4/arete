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

/// Transform large u64 values to strings for JavaScript compatibility.
/// JavaScript's Number.MAX_SAFE_INTEGER is 2^53 - 1 (9007199254740991).
/// Values larger than this will lose precision in JavaScript.
pub fn transform_large_u64_to_strings(value: &mut serde_json::Value) {
    const MAX_SAFE_INTEGER: u64 = 9007199254740991; // 2^53 - 1

    match value {
        serde_json::Value::Object(map) => {
            for (_, v) in map.iter_mut() {
                transform_large_u64_to_strings(v);
            }
        }
        serde_json::Value::Array(arr) => {
            for v in arr.iter_mut() {
                transform_large_u64_to_strings(v);
            }
        }
        serde_json::Value::Number(n) => {
            if let Some(n_u64) = n.as_u64() {
                if n_u64 > MAX_SAFE_INTEGER {
                    *value = serde_json::Value::String(n_u64.to_string());
                }
            } else if let Some(n_i64) = n.as_i64() {
                const MIN_SAFE_INTEGER: i64 = -(MAX_SAFE_INTEGER as i64);
                if n_i64 < MIN_SAFE_INTEGER {
                    *value = serde_json::Value::String(n_i64.to_string());
                }
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
        };

        let json = serde_json::to_value(&frame).unwrap();
        assert_eq!(json["op"], "upsert");
        assert_eq!(json["mode"], "list");
        assert_eq!(json["entity"], "SettlementGame/list");
        assert_eq!(json["key"], "123");
    }

    #[test]
    fn test_snapshot_frame_complete_serialization() {
        let frame = SnapshotFrame {
            mode: Mode::List,
            export: "tokens/list".to_string(),
            op: "snapshot",
            data: vec![SnapshotEntity {
                key: "abc".to_string(),
                data: serde_json::json!({"id": "abc"}),
            }],
            complete: false,
        };

        let json = serde_json::to_value(&frame).unwrap();
        assert_eq!(json["complete"], false);
        assert_eq!(json["op"], "snapshot");
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
    }

    #[test]
    fn test_snapshot_frame_batching_fields() {
        let first_batch = SnapshotFrame {
            mode: Mode::List,
            export: "tokens/list".to_string(),
            op: "snapshot",
            data: vec![],
            complete: false,
        };

        let final_batch = SnapshotFrame {
            mode: Mode::List,
            export: "tokens/list".to_string(),
            op: "snapshot",
            data: vec![],
            complete: true,
        };

        assert!(!first_batch.complete);
        assert!(final_batch.complete);
    }
}
