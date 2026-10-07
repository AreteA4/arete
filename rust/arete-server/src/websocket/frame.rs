use serde::ser::{SerializeMap as _, SerializeSeq as _};
use serde::{Deserialize, Serialize, Serializer};
use smallvec::SmallVec;

use crate::shared_entity::SharedEntity;
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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
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
    /// Offsets this view can replay, for append views backed by the event
    /// journal. Absent when the view has no retained tape.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replay_window: Option<crate::journal::ReplayWindow>,
    /// The server sends a key whole (`upsert` or a snapshot row) before any
    /// `patch` for it, so a client may drop a patch for a key it does not
    /// hold. Older servers omit it, and clients then keep such a patch. Tape
    /// records (frames with an `offset`) are events and not covered.
    #[serde(default)]
    pub whole_entities: bool,
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
            replay_window: None,
            whole_entities: true,
        }
    }

    /// Advertise the offsets a resuming consumer can still ask for.
    pub fn with_replay_window(mut self, window: crate::journal::ReplayWindow) -> Self {
        self.replay_window = Some(window);
        self
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
    /// Journal offset for replayable append views. Absent for latest-state
    /// modes, which have no per-event identity to resume from.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<u64>,
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

/// An entity as a view puts it on the wire: serialized with the view's
/// [`WireFormat`] applied, byte for byte as [`apply_wire_format`] on a copy of
/// it would serialize, without making the copy.
pub(crate) struct WireEntity<'a> {
    pub entity: &'a SharedEntity,
    pub wire_format: &'a WireFormat,
}

impl Serialize for WireEntity<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let wire = WirePaths::new(self.wire_format.wide_int_paths.iter().map(Vec::as_slice));
        if wire.is_noop() {
            return self.entity.serialize(serializer);
        }
        self.entity.serialize_with(
            serializer,
            |key, value| FieldWire {
                value,
                wire: wire.field(key),
            },
            |value| WireValue { value, wire: &wire },
        )
    }
}

/// A top-level field and the paths that reach it.
struct FieldWire<'a> {
    value: &'a serde_json::Value,
    wire: WirePaths<'a>,
}

impl Serialize for FieldWire<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        WireValue {
            value: self.value,
            wire: &self.wire,
        }
        .serialize(serializer)
    }
}

/// The wide-int paths that reach one node of a value, as
/// [`stringify_value_at_path`] walks them: `terminal` when one ends here.
#[derive(Clone)]
struct WirePaths<'a> {
    terminal: bool,
    /// The rest of every path that continues below this node.
    paths: SmallVec<[&'a [String]; 4]>,
}

impl<'a> WirePaths<'a> {
    fn new(paths: impl Iterator<Item = &'a [String]>) -> Self {
        let mut terminal = false;
        let mut rest = SmallVec::new();
        for path in paths {
            if path.is_empty() {
                terminal = true;
            } else {
                rest.push(path);
            }
        }
        Self {
            terminal,
            paths: rest,
        }
    }

    /// The paths reaching an object's field. A path ending at the object does
    /// not reach into its fields.
    fn field(&self, key: &str) -> Self {
        Self::new(
            self.paths
                .iter()
                .filter(|path| path[0] == key)
                .map(|path| &path[1..]),
        )
    }

    fn is_noop(&self) -> bool {
        !self.terminal && self.paths.is_empty()
    }
}

struct WireValue<'a, 'w> {
    value: &'a serde_json::Value,
    wire: &'w WirePaths<'a>,
}

impl Serialize for WireValue<'_, '_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde_json::Value;
        if self.wire.is_noop() {
            return self.value.serialize(serializer);
        }
        match self.value {
            Value::Number(number) if self.wire.terminal => {
                if let Some(unsigned) = number.as_u64() {
                    serializer.serialize_str(&unsigned.to_string())
                } else if let Some(signed) = number.as_i64() {
                    serializer.serialize_str(&signed.to_string())
                } else {
                    number.serialize(serializer)
                }
            }
            // Paths and a path's end both reach every element of an array.
            Value::Array(values) => {
                let mut sequence = serializer.serialize_seq(Some(values.len()))?;
                for value in values {
                    sequence.serialize_element(&WireValue {
                        value,
                        wire: self.wire,
                    })?;
                }
                sequence.end()
            }
            Value::Object(fields) => {
                let mut map = serializer.serialize_map(Some(fields.len()))?;
                for (key, value) in fields {
                    let wire = self.wire.field(key);
                    if wire.is_noop() {
                        map.serialize_entry(key, value)?;
                    } else {
                        map.serialize_entry(key, &WireValue { value, wire: &wire })?;
                    }
                }
                map.end()
            }
            other => other.serialize(serializer),
        }
    }
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

    /// Serializing a shared entity with a wire format writes the bytes that
    /// applying the format to a copy and serializing the copy writes.
    #[test]
    fn a_wire_entity_serializes_as_the_formatted_copy() {
        let entity = json!({
            "_version": "e:7",
            "_seq": "10:000000000001",
            "amount": 42,
            "negative": -42,
            "max": u64::MAX,
            "ratio": 1.5,
            "label": "7",
            "small": 5,
            "nested": {"amount": 9, "deeper": {"amount": [1, -2, [3, 4.5]]}},
            "positions": [{"liquidity": 9}, {"liquidity": [11, 12]}, 13, {"other": 1}],
            "matrix": [[1, 2], [3, {"x": 4}]],
            "empty": {},
        });
        let paths = |paths: &[&[&str]]| WireFormat {
            wide_int_paths: paths
                .iter()
                .map(|path| path.iter().map(|segment| segment.to_string()).collect())
                .collect(),
        };
        let formats = [
            paths(&[]),
            paths(&[&["amount"]]),
            paths(&[&["negative"], &["max"], &["ratio"], &["label"]]),
            paths(&[&["positions", "liquidity"]]),
            paths(&[&["positions"]]),
            paths(&[
                &["nested"],
                &["nested", "amount"],
                &["nested", "deeper", "amount"],
            ]),
            paths(&[&["matrix"], &["matrix", "x"]]),
            paths(&[
                &["_version"],
                &["_seq"],
                &["missing", "path"],
                &["empty", "x"],
            ]),
            paths(&[&["amount"], &["amount"]]),
            paths(&[&[]]),
        ];
        for format in formats {
            for whole in [
                entity.clone(),
                json!([1, {"amount": 2}]),
                json!(3),
                json!(null),
            ] {
                let shared = SharedEntity::new(whole.clone());
                let mut copy = whole.clone();
                apply_wire_format(&mut copy, &format);
                assert_eq!(
                    serde_json::to_string(&WireEntity {
                        entity: &shared,
                        wire_format: &format,
                    })
                    .unwrap(),
                    serde_json::to_string(&copy).unwrap(),
                    "{whole} with {:?}",
                    format.wide_int_paths
                );
            }
        }
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
