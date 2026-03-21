use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use std::io::Read;

const GZIP_MAGIC: [u8; 2] = [0x1f, 0x8b];

fn is_gzip(data: &[u8]) -> bool {
    data.len() >= 2 && data[0] == GZIP_MAGIC[0] && data[1] == GZIP_MAGIC[1]
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
pub struct SortConfig {
    pub field: Vec<String>,
    pub order: SortOrder,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscribedFrame {
    pub op: String,
    pub view: String,
    pub mode: Mode,
    #[serde(default)]
    pub sort: Option<SortConfig>,
}

impl SubscribedFrame {
    pub fn is_subscribed_frame(op: &str) -> bool {
        op == "subscribed"
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Upsert,
    Patch,
    Delete,
    Create,
    Snapshot,
    Subscribed,
}

impl std::str::FromStr for Operation {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "upsert" => Operation::Upsert,
            "patch" => Operation::Patch,
            "delete" => Operation::Delete,
            "create" => Operation::Create,
            "snapshot" => Operation::Snapshot,
            "subscribed" => Operation::Subscribed,
            _ => Operation::Upsert,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Frame {
    pub mode: Mode,
    #[serde(rename = "entity")]
    pub entity: String,
    pub op: String,
    #[serde(default)]
    pub key: String,
    pub data: serde_json::Value,
    #[serde(default)]
    pub append: Vec<String>,
    /// Sequence cursor for ordering and resume capability
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seq: Option<String>,
}

impl Frame {
    pub fn entity_name(&self) -> &str {
        &self.entity
    }

    pub fn operation(&self) -> Operation {
        self.op.parse().unwrap()
    }

    pub fn is_snapshot(&self) -> bool {
        self.op == "snapshot"
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotEntity {
    pub key: String,
    pub data: serde_json::Value,
}

fn decompress_gzip(data: &[u8]) -> Result<String, Box<dyn std::error::Error>> {
    let mut decoder = GzDecoder::new(data);
    let mut decompressed = String::new();
    decoder.read_to_string(&mut decompressed)?;
    Ok(decompressed)
}

pub fn parse_frame(bytes: &[u8]) -> Result<Frame, serde_json::Error> {
    if is_gzip(bytes) {
        if let Ok(decompressed) = decompress_gzip(bytes) {
            return serde_json::from_str(&decompressed);
        }
    }

    let text = String::from_utf8_lossy(bytes);
    serde_json::from_str(&text)
}

pub fn parse_snapshot_entities(data: &serde_json::Value) -> Vec<SnapshotEntity> {
    match data {
        serde_json::Value::Array(arr) => arr
            .iter()
            .filter_map(|v| serde_json::from_value(v.clone()).ok())
            .collect(),
        _ => Vec::new(),
    }
}

#[allow(dead_code)]
pub fn parse_subscribed_frame(bytes: &[u8]) -> Result<SubscribedFrame, serde_json::Error> {
    if is_gzip(bytes) {
        if let Ok(decompressed) = decompress_gzip(bytes) {
            return serde_json::from_str(&decompressed);
        }
    }

    let text = String::from_utf8_lossy(bytes);
    serde_json::from_str(&text)
}

#[allow(dead_code)]
pub fn try_parse_subscribed_frame(bytes: &[u8]) -> Option<SubscribedFrame> {
    let frame: serde_json::Value = if is_gzip(bytes) {
        decompress_gzip(bytes)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())?
    } else {
        serde_json::from_slice(bytes).ok()?
    };

    if frame.get("op")?.as_str()? == "subscribed" {
        serde_json::from_value(frame).ok()
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{write::GzEncoder, Compression};
    use std::io::Write;

    #[test]
    fn test_parse_uncompressed_frame() {
        let frame_json = r#"{"mode":"list","entity":"test/list","op":"snapshot","key":"","data":[{"key":"1","data":{"id":1}}]}"#;
        let frame = parse_frame(frame_json.as_bytes()).unwrap();
        assert_eq!(frame.op, "snapshot");
        assert_eq!(frame.entity, "test/list");
    }

    #[test]
    fn test_parse_raw_gzip_frame() {
        let original = r#"{"mode":"list","entity":"test/list","op":"snapshot","key":"","data":[{"key":"1","data":{"id":1}}]}"#;

        let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
        encoder.write_all(original.as_bytes()).unwrap();
        let compressed = encoder.finish().unwrap();

        assert!(is_gzip(&compressed));

        let frame = parse_frame(&compressed).unwrap();
        assert_eq!(frame.op, "snapshot");
        assert_eq!(frame.entity, "test/list");
    }

    #[test]
    fn test_gzip_magic_detection() {
        assert!(is_gzip(&[0x1f, 0x8b, 0x08]));
        assert!(!is_gzip(&[0x7b, 0x22]));
        assert!(!is_gzip(&[0x1f]));
        assert!(!is_gzip(&[]));
    }
}
