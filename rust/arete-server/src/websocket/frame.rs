use serde::ser::{SerializeMap as _, SerializeSeq as _};
use serde::{Deserialize, Serialize, Serializer};
use serde_json::value::RawValue;
use smallvec::SmallVec;

use crate::shared_entity::{maps_sort_keys, SharedEntity};
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

/// A source frame's top-level fields, each kept as its JSON text, in the
/// order parsing the frame into a [`serde_json::Value`] would keep them.
/// Reading one field parses only that field: a frame's `data` (a whole
/// entity, for an `upsert`) is never built unless asked for.
pub(crate) struct SourceFields<'a>(Vec<(String, &'a RawValue)>);

impl<'a> SourceFields<'a> {
    /// The fields of `payload`, or `None` if it is not a JSON object.
    pub fn parse(payload: &'a [u8]) -> Option<Self> {
        serde_json::from_slice(payload).ok()
    }

    /// One field, parsed.
    pub fn value(&self, key: &str) -> Option<serde_json::Value> {
        let (_, raw) = self.0.iter().find(|(field, _)| field == key)?;
        serde_json::from_str(raw.get()).ok()
    }

    /// Set a field, as inserting into a parsed object does: a key already
    /// there keeps its place.
    pub fn insert(&mut self, key: &str, value: &'a RawValue) {
        match self.0.iter_mut().find(|(field, _)| field == key) {
            Some((_, held)) => *held = value,
            None => self.0.push((key.to_string(), value)),
        }
    }

    /// The frame scoped to one subscription: these fields plus
    /// `protocolVersion` and `subscriptionId`, byte for byte as serializing
    /// the parsed frame with them inserted would write it. Each field's text
    /// is copied as it is, which is what reserializing it would write: a
    /// source frame is itself `serde_json` output.
    pub fn scoped(payload: &[u8], subscription_id: &str) -> serde_json::Result<Vec<u8>> {
        let version = RawValue::from_string(PROTOCOL_VERSION.to_string())?;
        let subscription = serde_json::value::to_raw_value(subscription_id)?;
        let mut fields: SourceFields<'_> = serde_json::from_slice(payload)?;
        fields.insert("protocolVersion", &version);
        fields.insert("subscriptionId", &subscription);
        if maps_sort_keys() {
            fields.0.sort_by(|(left, _), (right, _)| left.cmp(right));
        }
        serde_json::to_vec(&fields)
    }
}

impl<'de> Deserialize<'de> for SourceFields<'de> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Fields;

        impl<'de> serde::de::Visitor<'de> for Fields {
            type Value = SourceFields<'de>;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a source frame object")
            }

            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                let mut fields = SourceFields(Vec::new());
                while let Some((key, value)) = map.next_entry::<String, &'de RawValue>()? {
                    fields.insert(&key, value);
                }
                Ok(fields)
            }
        }

        deserializer.deserialize_map(Fields)
    }
}

impl Serialize for SourceFields<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (key, value) in &self.0 {
            map.serialize_entry(key, value)?;
        }
        map.end()
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

    /// The old scoping: parse the frame, insert the two fields, serialize.
    fn scoped_by_parsing(payload: &[u8], subscription_id: &str) -> Option<Vec<u8>> {
        let mut value: serde_json::Value = serde_json::from_slice(payload).ok()?;
        let object = value.as_object_mut()?;
        object.insert("protocolVersion".to_string(), PROTOCOL_VERSION.into());
        object.insert(
            "subscriptionId".to_string(),
            serde_json::Value::String(subscription_id.to_string()),
        );
        serde_json::to_vec(&value).ok()
    }

    /// Scoping a source frame without parsing its data writes the bytes that
    /// parsing and reserializing it wrote, for any frame `serde_json` writes.
    #[test]
    fn scoped_source_frames_match_reserialized_ones() {
        let data = json!({
            "_seq": "10:000000000001",
            "_version": "e:1",
            "float": 1.5,
            "tiny": 1e-7,
            "whole_float": 3.0,
            "negative_zero": -0.0,
            "big": u64::MAX,
            "negative": i64::MIN,
            "text": "π \"quoted\" \\ \u{0001} \n\t / \u{2028} \u{1F600}",
            "nested": {"z": [1, {"b": null, "a": true}], "a": []},
        });
        let frames = [
            json!({"mode": "list", "entity": "Thing/list", "op": "patch", "key": "1", "data": data.clone(), "seq": "10:1"}),
            json!({"mode": "state", "entity": "Thing/state", "op": "upsert", "key": "k\"ey", "data": data, "append": ["trades"], "offset": 9}),
            json!({"op": "delete", "data": null, "protocolVersion": 1, "subscriptionId": "old"}),
            json!({}),
        ];
        for frame in frames {
            for payload in [
                serde_json::to_vec(&frame).unwrap(),
                frame.to_string().into_bytes(),
            ] {
                assert_eq!(
                    SourceFields::scoped(&payload, "sub \"1\"").ok(),
                    scoped_by_parsing(&payload, "sub \"1\""),
                    "{frame}"
                );
            }
        }
        // A frame with a key twice reads like the parsed one: the last value
        // wins.
        let twice = br#"{"op":"patch","data":{"a":1},"data":{"b":2},"key":"1"}"#;
        assert_eq!(
            SourceFields::scoped(twice, "s").ok(),
            scoped_by_parsing(twice, "s")
        );
        for invalid in [&b"[1,2]"[..], b"\"text\"", b"{\"op\":", b""] {
            assert!(SourceFields::scoped(invalid, "s").is_err());
            assert!(SourceFields::parse(invalid).is_none());
        }
    }

    #[test]
    fn source_fields_parse_one_field_at_a_time() {
        let payload = br#"{"op":"patch","seq":"10:1","offset":4,"data":{"a":[1,2]}}"#;
        let fields = SourceFields::parse(payload).unwrap();
        assert_eq!(fields.value("op"), Some(json!("patch")));
        assert_eq!(fields.value("offset"), Some(json!(4)));
        assert_eq!(fields.value("data"), Some(json!({"a": [1, 2]})));
        assert_eq!(fields.value("missing"), None);
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
