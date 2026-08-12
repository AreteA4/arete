use crate::error::AreteError;
use crate::subscription::{SubscriptionQuery, PROTOCOL_VERSION};
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::cmp::Ordering;
use std::io::Read;

const GZIP_MAGIC: [u8; 2] = [0x1f, 0x8b];

fn is_gzip(data: &[u8]) -> bool {
    data.starts_with(&GZIP_MAGIC)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    State,
    Append,
    List,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SortOrder {
    Asc,
    Desc,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SortConfig {
    pub field: Vec<String>,
    pub order: SortOrder,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Operation {
    Upsert,
    Patch,
    Remove,
    Delete,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SnapshotEntity {
    pub key: String,
    pub data: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(
    tag = "op",
    rename_all = "lowercase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ServerFrame {
    Subscribed {
        protocol_version: u8,
        subscription_id: String,
        query: SubscriptionQuery,
        mode: Mode,
        #[serde(skip_serializing_if = "Option::is_none")]
        sort: Option<SortConfig>,
    },
    Unsubscribed {
        protocol_version: u8,
        subscription_id: String,
    },
    Snapshot {
        protocol_version: u8,
        subscription_id: String,
        snapshot_id: String,
        authoritative: bool,
        mode: Mode,
        entity: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        key: Option<String>,
        data: Vec<SnapshotEntity>,
        complete: bool,
    },
    Upsert {
        protocol_version: u8,
        subscription_id: String,
        mode: Mode,
        entity: String,
        key: String,
        data: Value,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        append: Vec<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        seq: Option<String>,
    },
    Patch {
        protocol_version: u8,
        subscription_id: String,
        mode: Mode,
        entity: String,
        key: String,
        data: Value,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        append: Vec<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        seq: Option<String>,
    },
    Remove {
        protocol_version: u8,
        subscription_id: String,
        mode: Mode,
        entity: String,
        key: String,
        data: Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        seq: Option<String>,
    },
    Delete {
        protocol_version: u8,
        subscription_id: String,
        mode: Mode,
        entity: String,
        key: String,
        data: Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        seq: Option<String>,
    },
}

impl ServerFrame {
    pub fn protocol_version(&self) -> u8 {
        match self {
            Self::Subscribed {
                protocol_version, ..
            }
            | Self::Unsubscribed {
                protocol_version, ..
            }
            | Self::Snapshot {
                protocol_version, ..
            }
            | Self::Upsert {
                protocol_version, ..
            }
            | Self::Patch {
                protocol_version, ..
            }
            | Self::Remove {
                protocol_version, ..
            }
            | Self::Delete {
                protocol_version, ..
            } => *protocol_version,
        }
    }

    pub fn subscription_id(&self) -> &str {
        match self {
            Self::Subscribed {
                subscription_id, ..
            }
            | Self::Unsubscribed {
                subscription_id, ..
            }
            | Self::Snapshot {
                subscription_id, ..
            }
            | Self::Upsert {
                subscription_id, ..
            }
            | Self::Patch {
                subscription_id, ..
            }
            | Self::Remove {
                subscription_id, ..
            }
            | Self::Delete {
                subscription_id, ..
            } => subscription_id,
        }
    }
}

pub type Frame = ServerFrame;

/// Split a `slot:index` sequence string into its two leading segments.
///
/// `frame-processor.ts:313` uses `split(':', 2)` and `wire.py:83` takes
/// `parts[1]`: both keep only the *second* segment as the index, so anything
/// after a second colon is dropped rather than folded into the index.
fn split_seq(value: &str) -> (&str, &str) {
    let mut segments = value.split(':');
    let slot = segments.next().unwrap_or("");
    let index = segments.next().unwrap_or("");
    (slot, index)
}

fn is_ascii_digits(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
}

/// Order two protocol sequence strings (`"<slot>:<index>"`).
///
/// Mirrors `compareSeq` (`typescript/core/src/frame-processor.ts:312`) and
/// `compare_seq` (`python/arete-sdk/arete/wire.py:98`): the slot is compared
/// numerically when *both* slots are all ASCII digits, and the index is
/// compared by code point. A non-numeric slot on either side makes the slot
/// irrelevant and the index decides.
///
/// The slot is compared as a digit string rather than through an integer parse
/// because TypeScript uses `BigInt` and Python uses `int`: neither has a width
/// limit, so a `u64::parse` would overflow on long slots.
pub(crate) fn compare_seq(left: &str, right: &str) -> Ordering {
    let (left_slot, left_index) = split_seq(left);
    let (right_slot, right_index) = split_seq(right);
    if is_ascii_digits(left_slot) && is_ascii_digits(right_slot) {
        let left_digits = left_slot.trim_start_matches('0');
        let right_digits = right_slot.trim_start_matches('0');
        let slot_order = left_digits
            .len()
            .cmp(&right_digits.len())
            .then_with(|| left_digits.cmp(right_digits));
        if slot_order != Ordering::Equal {
            return slot_order;
        }
    }
    // Code-point order, matching Python's `<` on `str`. `collation.rs` models
    // JS `localeCompare` for sort keys and filter keys; seq indices are opaque
    // fixed-width counters and are not collated.
    left_index.cmp(right_index)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProtocolErrorFrame {
    #[serde(rename = "type")]
    pub kind: String,
    pub protocol_version: u8,
    pub subscription_id: Option<String>,
    #[serde(default)]
    pub error: String,
    #[serde(default)]
    pub message: String,
    pub code: String,
    #[serde(default)]
    pub retryable: bool,
    #[serde(default)]
    pub retry_after: Option<u64>,
    #[serde(default)]
    pub suggested_action: Option<String>,
    #[serde(default)]
    pub docs_url: Option<String>,
    #[serde(default)]
    pub fatal: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ServerMessage {
    Frame(ServerFrame),
    Error(ProtocolErrorFrame),
}

fn decompress(data: &[u8]) -> Result<Vec<u8>, AreteError> {
    if !is_gzip(data) {
        return Ok(data.to_vec());
    }
    let mut decoder = GzDecoder::new(data);
    let mut decompressed = Vec::new();
    decoder
        .read_to_end(&mut decompressed)
        .map_err(|error| AreteError::Protocol {
            message: format!("failed to decompress WebSocket frame: {error}"),
            subscription_id: None,
        })?;
    Ok(decompressed)
}

pub fn parse_server_message(bytes: &[u8]) -> Result<ServerMessage, AreteError> {
    let bytes = decompress(bytes)?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|error| AreteError::Protocol {
        message: format!("malformed WebSocket message: {error}"),
        subscription_id: None,
    })?;
    let subscription_id = value
        .get("subscriptionId")
        .and_then(Value::as_str)
        .map(str::to_string);

    let Some(version) = value.get("protocolVersion").and_then(Value::as_u64) else {
        return Err(AreteError::Protocol {
            message: "legacy WebSocket frame rejected: protocolVersion 2 and subscriptionId are required; migrate this client/server pair to protocol v2".to_string(),
            subscription_id,
        });
    };
    if version != u64::from(PROTOCOL_VERSION) {
        return Err(AreteError::Protocol {
            message: format!(
                "unsupported WebSocket protocol version {version}; the Rust SDK requires protocol v2"
            ),
            subscription_id,
        });
    }

    if value.get("type").and_then(Value::as_str) == Some("error") {
        let error: ProtocolErrorFrame =
            serde_json::from_value(value).map_err(|error| AreteError::Protocol {
                message: format!("invalid protocol v2 error envelope: {error}"),
                subscription_id: subscription_id.clone(),
            })?;
        if error.kind != "error" {
            return Err(AreteError::Protocol {
                message: format!(
                    "unknown WebSocket protocol v2 message type '{}'; expected 'error'",
                    error.kind
                ),
                subscription_id,
            });
        }
        return Ok(ServerMessage::Error(error));
    }

    let Some(operation) = value.get("op").and_then(Value::as_str).map(str::to_string) else {
        let kind = value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("<missing>");
        return Err(AreteError::Protocol {
            message: format!(
                "unknown WebSocket protocol v2 message type '{kind}'; expected a frame operation"
            ),
            subscription_id,
        });
    };
    if !matches!(
        operation.as_str(),
        "subscribed" | "unsubscribed" | "snapshot" | "upsert" | "patch" | "remove" | "delete"
    ) {
        return Err(AreteError::Protocol {
            message: format!(
                "unknown WebSocket protocol v2 operation '{operation}'; legacy operations are not supported"
            ),
            subscription_id,
        });
    }

    let frame: ServerFrame =
        serde_json::from_value(value).map_err(|error| AreteError::Protocol {
            message: format!("invalid protocol v2 '{operation}' frame: {error}"),
            subscription_id,
        })?;
    Ok(ServerMessage::Frame(frame))
}

pub fn parse_frame(bytes: &[u8]) -> Result<ServerFrame, AreteError> {
    match parse_server_message(bytes)? {
        ServerMessage::Frame(frame) => Ok(frame),
        ServerMessage::Error(error) => Err(AreteError::Protocol {
            message: format!(
                "server returned protocol error '{}': {}",
                error.code, error.message
            ),
            subscription_id: error.subscription_id,
        }),
    }
}

pub fn parse_snapshot_entities(data: &Value) -> Vec<SnapshotEntity> {
    data.as_array()
        .into_iter()
        .flatten()
        .filter_map(|value| serde_json::from_value(value.clone()).ok())
        .collect()
}

pub fn try_parse_subscribed_frame(bytes: &[u8]) -> Option<ServerFrame> {
    match parse_server_message(bytes).ok()? {
        ServerMessage::Frame(frame @ ServerFrame::Subscribed { .. }) => Some(frame),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{write::GzEncoder, Compression};
    use std::io::Write;

    const FRAME: &str = r#"{"protocolVersion":2,"subscriptionId":"things","mode":"list","entity":"Thing/list","op":"upsert","key":"1","data":{"id":1}}"#;

    #[test]
    fn parses_typed_v2_frame_and_gzip() {
        assert!(matches!(
            parse_frame(FRAME.as_bytes()).unwrap(),
            ServerFrame::Upsert { ref key, .. } if key == "1"
        ));

        let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
        encoder.write_all(FRAME.as_bytes()).unwrap();
        assert!(matches!(
            parse_frame(&encoder.finish().unwrap()).unwrap(),
            ServerFrame::Upsert { .. }
        ));
    }

    #[test]
    fn compare_seq_orders_slot_numerically() {
        // Numeric slots never compare as text: "9" > "10" lexicographically.
        assert_eq!(compare_seq("9:0001", "10:0001"), Ordering::Less);
        assert_eq!(compare_seq("50:0001", "49:9999"), Ordering::Greater);
        // Leading zeros and unbounded width (TS `BigInt` / Python `int`).
        assert_eq!(compare_seq("007:a", "7:a"), Ordering::Equal);
        assert_eq!(
            compare_seq("99999999999999999999999:a", "100000000000000000000000:a"),
            Ordering::Less
        );
    }

    #[test]
    fn compare_seq_falls_back_to_index_lexicographically() {
        assert_eq!(
            compare_seq("50:000000000001", "50:000000000002"),
            Ordering::Less
        );
        assert_eq!(compare_seq("50:0002", "50:0002"), Ordering::Equal);
        // Only the second segment is the index; anything past it is ignored.
        assert_eq!(compare_seq("50:0002:zzz", "50:0002:aaa"), Ordering::Equal);
        // A missing index is the empty string, which sorts first.
        assert_eq!(compare_seq("50", "50:0001"), Ordering::Less);
    }

    #[test]
    fn compare_seq_ignores_non_numeric_slots() {
        // One non-digit slot disables the numeric slot comparison entirely, so
        // the index decides even though "b" > "a".
        assert_eq!(compare_seq("b:0001", "a:0002"), Ordering::Less);
        assert_eq!(compare_seq("b:0002", "9:0001"), Ordering::Greater);
        // An empty slot is not "all digits" either.
        assert_eq!(compare_seq(":0002", "50:0001"), Ordering::Greater);
        // Non-ASCII digits are not digits (`/^\d+$/`, `^[0-9]+$`).
        assert_eq!(compare_seq("١٢:0001", "9:0002"), Ordering::Less);
    }

    #[test]
    fn rejects_unknown_and_legacy_operations() {
        let unknown = FRAME.replace("upsert", "create");
        let error = parse_frame(unknown.as_bytes()).unwrap_err();
        assert!(error
            .to_string()
            .contains("unknown WebSocket protocol v2 operation 'create'"));

        let legacy = r#"{"mode":"list","entity":"Thing/list","op":"upsert","key":"1","data":{}}"#;
        let error = parse_frame(legacy.as_bytes()).unwrap_err();
        assert!(error
            .to_string()
            .contains("migrate this client/server pair"));

        let unknown_field = FRAME.replace(
            r#""data":{"id":1}"#,
            r#""data":{"id":1},"legacyView":"Thing/list""#,
        );
        let error = parse_frame(unknown_field.as_bytes()).unwrap_err();
        assert!(error.to_string().contains("unknown field"));
    }
}
