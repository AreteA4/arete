//! Application-level compression for WebSocket payloads.
//!
//! Since tokio-tungstenite doesn't support permessage-deflate, we implement
//! application-level gzip compression for large payloads (like snapshots).
//!
//! Compressed payloads are sent as raw binary gzip data. Clients detect
//! compression by checking for the gzip magic bytes (0x1f, 0x8b) at the
//! start of binary WebSocket frames.
//!
//! This approach eliminates the ~33% overhead of base64 encoding that was
//! previously used with JSON-wrapped compressed data.

use bytes::Bytes;
use flate2::{write::GzEncoder, Compression};
use std::io::Write;

/// Minimum payload size (in bytes) before compression is applied.
/// Payloads smaller than this are sent uncompressed.
const COMPRESSION_THRESHOLD: usize = 1024; // 1KB

/// Result of attempting to compress a payload.
#[derive(Debug)]
pub enum CompressedPayload {
    /// Payload was compressed - contains raw gzip bytes.
    /// Should be sent as a binary WebSocket frame.
    Compressed(Bytes),
    /// Payload was not compressed - contains original JSON bytes.
    /// Should be sent as a text WebSocket frame (or binary, both work).
    Uncompressed(Bytes),
}

impl CompressedPayload {
    /// Returns true if the payload is compressed.
    pub fn is_compressed(&self) -> bool {
        matches!(self, CompressedPayload::Compressed(_))
    }

    /// Consumes self and returns the inner bytes.
    pub fn into_bytes(self) -> Bytes {
        match self {
            CompressedPayload::Compressed(b) => b,
            CompressedPayload::Uncompressed(b) => b,
        }
    }

    /// Returns a reference to the inner bytes.
    pub fn as_bytes(&self) -> &Bytes {
        match self {
            CompressedPayload::Compressed(b) => b,
            CompressedPayload::Uncompressed(b) => b,
        }
    }
}

/// Compress a payload if it exceeds the threshold.
///
/// Returns `CompressedPayload::Uncompressed` if:
/// - Payload is below threshold
/// - Compression fails
/// - Compressed size is larger than original (unlikely for JSON)
///
/// Returns `CompressedPayload::Compressed` with raw gzip bytes if compression
/// is beneficial. The gzip data starts with magic bytes 0x1f 0x8b which clients
/// use to detect compression.
pub fn maybe_compress(payload: &[u8]) -> CompressedPayload {
    if payload.len() < COMPRESSION_THRESHOLD {
        return CompressedPayload::Uncompressed(Bytes::copy_from_slice(payload));
    }

    match compress_gzip(payload) {
        Ok(compressed) => {
            // Only use compression if it actually reduces size
            if compressed.len() < payload.len() {
                CompressedPayload::Compressed(Bytes::from(compressed))
            } else {
                CompressedPayload::Uncompressed(Bytes::copy_from_slice(payload))
            }
        }
        Err(_) => CompressedPayload::Uncompressed(Bytes::copy_from_slice(payload)),
    }
}

fn compress_gzip(data: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(data)?;
    encoder.finish()
}

/// Gzip magic bytes - used by clients to detect compressed frames.
pub const GZIP_MAGIC: [u8; 2] = [0x1f, 0x8b];

/// Check if bytes start with gzip magic bytes.
pub fn is_gzip(data: &[u8]) -> bool {
    data.len() >= 2 && data[0] == GZIP_MAGIC[0] && data[1] == GZIP_MAGIC[1]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_small_payload_not_compressed() {
        let small = b"hello";
        let result = maybe_compress(small);
        assert!(!result.is_compressed());
        assert_eq!(result.as_bytes().as_ref(), small);
    }

    #[test]
    fn test_large_payload_compressed_as_raw_gzip() {
        let entities: Vec<_> = (0..100)
            .map(|i| {
                json!({
                    "key": format!("entity_{}", i),
                    "data": {
                        "id": i,
                        "name": format!("Entity number {}", i),
                        "description": "This is a test entity with some data that will be repeated",
                        "values": [1, 2, 3, 4, 5, 6, 7, 8, 9, 10],
                        "nested": {
                            "field1": "value1",
                            "field2": "value2",
                            "field3": "value3",
                        }
                    }
                })
            })
            .collect();

        let payload = serde_json::to_vec(&entities).unwrap();
        let original_size = payload.len();
        let result = maybe_compress(&payload);

        assert!(result.is_compressed());

        let bytes = result.as_bytes();
        assert!(
            is_gzip(bytes),
            "Compressed data should start with gzip magic bytes"
        );

        assert!(
            bytes.len() < original_size,
            "Compressed {} should be < original {}",
            bytes.len(),
            original_size
        );

        println!(
            "Original: {} bytes, Compressed: {} bytes, Ratio: {:.1}%",
            original_size,
            bytes.len(),
            (bytes.len() as f64 / original_size as f64) * 100.0
        );
    }

    #[test]
    fn test_gzip_magic_detection() {
        assert!(is_gzip(&[0x1f, 0x8b, 0x08]));
        assert!(!is_gzip(&[0x7b, 0x22]));
        assert!(!is_gzip(&[0x1f]));
        assert!(!is_gzip(&[]));
    }

    #[test]
    fn test_small_data_not_compressed() {
        let data = b"small";
        let result = maybe_compress(data);
        assert!(!result.is_compressed());
        assert_eq!(result.as_bytes().as_ref(), data);
    }
}
