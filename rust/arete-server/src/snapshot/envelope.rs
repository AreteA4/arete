//! On-disk snapshot format: a JSON header (readable with `head`/`jq` after
//! skipping 12 bytes) followed by a zstd-compressed JSON payload.
//!
//! Layout: `b"ARSNAP01" | u32-le header_len | header JSON | zstd(payload JSON)`.

use anyhow::{bail, Context, Result};
use arete_interpreter::snapshot::VmSnapshot;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

const MAGIC: &[u8; 8] = b"ARSNAP01";
const ZSTD_LEVEL: i32 = 3;
/// Guards against decompression bombs from a corrupted or hostile store.
const MAX_PAYLOAD_BYTES: usize = 1 << 30;

/// A versioned, normalized description of data that a snapshot persists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotContract {
    pub schema: String,
    pub hash: String,
}

/// Plain-JSON metadata used to validate a snapshot before decompressing it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotHeader {
    /// [`arete_interpreter::snapshot::SNAPSHOT_FORMAT_VERSION`] at write time.
    pub format_version: u32,
    /// Fingerprint of the compiled `MultiEntityBytecode`. A mismatch means the
    /// stack's logic changed; the snapshot is discarded (cold start).
    pub bytecode_hash: String,
    /// Durable entity structure, independent of handler opcode ordering.
    /// Absent on snapshots written before contract-aware restore.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_contract: Option<SnapshotContract>,
    /// Runtime view/cache structure. Kept separate so a future migration can
    /// make an explicit decision about rebuilding projections.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub projection_contract: Option<SnapshotContract>,
    pub program_ids: Vec<String>,
    /// Highest slot among mutation batches the projector had applied when the
    /// VM was dumped. Safe `from_slot` resume point.
    pub resume_watermark: u64,
    /// Slot-subscription tip at dump time (diagnostics / staleness clamping).
    pub observed_slot: u64,
    pub created_at_epoch_ms: u64,
    /// Entity-name -> row count, for logging and debugging.
    #[serde(default)]
    pub entry_counts: BTreeMap<String, u64>,
    /// What wrote this snapshot. A shutdown snapshot is taken at a consistency
    /// cut with nothing in flight, so its offsets are exactly what was
    /// published; a periodic one can be behind by up to the write interval.
    /// Absent in snapshots written before the distinction mattered, which are
    /// treated as the unclean case.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<crate::snapshot::SnapshotTrigger>,
}

/// The compressed body of a snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotPayload {
    pub vm: VmSnapshot,
    /// Per view id: `(entity_key, entity)` pairs, most-recently-used first.
    pub entity_cache: Vec<(String, Vec<(String, Value)>)>,
    /// Retained event tape per view. Absent in snapshots written before
    /// replayable subscriptions existed, which restore with an empty tape.
    #[serde(default)]
    pub journal: crate::journal::JournalSnapshot,
}

pub fn encode(header: &SnapshotHeader, payload: &SnapshotPayload) -> Result<Vec<u8>> {
    let header_json = serde_json::to_vec(header).context("serialize snapshot header")?;
    let payload_json = serde_json::to_vec(payload).context("serialize snapshot payload")?;
    let compressed =
        zstd::encode_all(payload_json.as_slice(), ZSTD_LEVEL).context("compress snapshot")?;

    let mut bytes = Vec::with_capacity(MAGIC.len() + 4 + header_json.len() + compressed.len());
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&(header_json.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&header_json);
    bytes.extend_from_slice(&compressed);
    Ok(bytes)
}

/// Parse only the header, without touching the compressed payload.
pub fn decode_header(bytes: &[u8]) -> Result<SnapshotHeader> {
    if bytes.len() < MAGIC.len() + 4 {
        bail!("snapshot blob truncated ({} bytes)", bytes.len());
    }
    if &bytes[..MAGIC.len()] != MAGIC {
        bail!("snapshot blob has wrong magic");
    }
    let header_len =
        u32::from_le_bytes(bytes[MAGIC.len()..MAGIC.len() + 4].try_into().unwrap()) as usize;
    let header_start = MAGIC.len() + 4;
    let header_end = header_start
        .checked_add(header_len)
        .filter(|end| *end <= bytes.len())
        .context("snapshot header length out of bounds")?;
    serde_json::from_slice(&bytes[header_start..header_end]).context("parse snapshot header")
}

pub fn decode_payload(bytes: &[u8]) -> Result<SnapshotPayload> {
    // Re-derive the payload offset the same way decode_header does.
    let header_len =
        u32::from_le_bytes(bytes[MAGIC.len()..MAGIC.len() + 4].try_into().unwrap()) as usize;
    let payload_start = MAGIC.len() + 4 + header_len;

    use std::io::Read;
    let decoder = zstd::Decoder::new(&bytes[payload_start..])?;
    let mut payload_json = Vec::new();
    decoder
        .take(MAX_PAYLOAD_BYTES as u64 + 1)
        .read_to_end(&mut payload_json)
        .context("decompress snapshot payload")?;
    if payload_json.len() > MAX_PAYLOAD_BYTES {
        bail!("snapshot payload exceeds {} bytes", MAX_PAYLOAD_BYTES);
    }
    serde_json::from_slice(&payload_json).context("parse snapshot payload")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> (SnapshotHeader, SnapshotPayload) {
        let header = SnapshotHeader {
            format_version: arete_interpreter::snapshot::SNAPSHOT_FORMAT_VERSION,
            bytecode_hash: "abc123".to_string(),
            state_contract: Some(SnapshotContract {
                schema: "arete.snapshot-state-contract/v1".to_string(),
                hash: "state123".to_string(),
            }),
            projection_contract: Some(SnapshotContract {
                schema: "arete.snapshot-projection-contract/v1".to_string(),
                hash: "projection123".to_string(),
            }),
            program_ids: vec!["Program111".to_string()],
            resume_watermark: 42,
            observed_slot: 50,
            created_at_epoch_ms: 1_000,
            trigger: Some(crate::snapshot::SnapshotTrigger::Shutdown),
            entry_counts: BTreeMap::new(),
        };
        let payload = SnapshotPayload {
            vm: VmSnapshot::default(),
            entity_cache: vec![(
                "tokens/list".to_string(),
                vec![("key1".to_string(), serde_json::json!({"id": 1}))],
            )],
            journal: crate::journal::JournalSnapshot::default(),
        };
        (header, payload)
    }

    #[test]
    fn round_trips_header_and_payload() {
        let (header, payload) = sample();
        let bytes = encode(&header, &payload).unwrap();

        let decoded_header = decode_header(&bytes).unwrap();
        assert_eq!(decoded_header.bytecode_hash, "abc123");
        assert_eq!(decoded_header.resume_watermark, 42);

        let decoded_payload = decode_payload(&bytes).unwrap();
        assert_eq!(decoded_payload.entity_cache.len(), 1);
        assert_eq!(decoded_payload.entity_cache[0].0, "tokens/list");
    }

    #[test]
    fn rejects_truncated_and_corrupt_blobs() {
        let (header, payload) = sample();
        let bytes = encode(&header, &payload).unwrap();

        assert!(decode_header(&bytes[..4]).is_err());
        assert!(decode_header(&[0u8; 32]).is_err());

        let mut truncated = bytes.clone();
        truncated.truncate(bytes.len() - 5);
        assert!(decode_payload(&truncated).is_err());

        let mut corrupted = bytes.clone();
        let last = corrupted.len() - 1;
        corrupted[last] ^= 0xFF;
        assert!(decode_payload(&corrupted).is_err());
    }
}
