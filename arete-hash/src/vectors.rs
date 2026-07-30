//! Language-neutral conformance vector support.
//!
//! The shared corpus at `test-vectors/hash-v1.json` describes inputs,
//! canonical payloads, framed preimages, digests, textual identifiers, and
//! expected failures. Rust and TypeScript tests execute the same vectors
//! through this single dispatch so both languages prove byte-level parity
//! against one implementation of the profile dispatch rules.

use sha2::{Digest, Sha256};

use crate::{
    artifact_tree_payload, canonicalize_json_bytes, framed_preimage, framed_tuple_payload,
    identity_metadata, AnyHashId, ArtifactEntryKind, ArtifactTreeEntry, CanonicalizationProfile,
    HashError, HashKindName, TupleField,
};

/// Owned form of the language-neutral vector input encodings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VectorInput {
    /// `{"encoding": "utf8", "data": "..."}` and `{"encoding": "hex", ...}`.
    RawBytes(Vec<u8>),
    /// `{"encoding": "tuple", "fields": [{"label", "valueUtf8"|"valueHex"}]}`.
    TupleFields(Vec<(String, Vec<u8>)>),
    /// `{"encoding": "tree", "entries": [{"path", "bytesHex", "type"}]}`.
    TreeEntries(Vec<VectorTreeEntry>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VectorTreeEntry {
    pub path: String,
    pub bytes: Vec<u8>,
    pub symlink: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VectorOutcome {
    pub canonical_payload: Vec<u8>,
    pub preimage: Vec<u8>,
    pub digest: [u8; 32],
    pub hash_id: AnyHashId,
}

/// Execute one hash vector: build the canonical payload for the profile,
/// frame it, and digest it. The declared profile must match the registry
/// profile for the kind; unknown kinds fail closed during vector parsing.
pub fn execute_vector(
    kind: HashKindName,
    profile: CanonicalizationProfile,
    input: &VectorInput,
) -> Result<VectorOutcome, HashError> {
    let metadata = identity_metadata(kind);
    if metadata.profile != profile {
        return Err(HashError::ProfileMismatch {
            kind: kind.to_string(),
            expected: metadata.profile.to_string(),
            actual: profile.to_string(),
        });
    }

    let canonical_payload = match (profile, input) {
        (CanonicalizationProfile::RawBytesV1, VectorInput::RawBytes(bytes)) => bytes.clone(),
        (CanonicalizationProfile::AreteJcsV1, VectorInput::RawBytes(bytes)) => {
            canonicalize_json_bytes(bytes)?
        }
        (CanonicalizationProfile::FramedTupleV1, VectorInput::TupleFields(fields)) => {
            let fields: Vec<TupleField<'_>> = fields
                .iter()
                .map(|(label, value)| TupleField::new(label, value))
                .collect();
            framed_tuple_payload(&fields)?
        }
        (CanonicalizationProfile::ArtifactTreeV1, VectorInput::TreeEntries(entries)) => {
            let entries: Vec<ArtifactTreeEntry<'_>> = entries
                .iter()
                .map(|entry| ArtifactTreeEntry {
                    path: &entry.path,
                    bytes: &entry.bytes,
                    kind: if entry.symlink {
                        ArtifactEntryKind::Symlink
                    } else {
                        ArtifactEntryKind::File
                    },
                })
                .collect();
            artifact_tree_payload(&entries)?
        }
        _ => {
            return Err(HashError::InvalidHashId(
                "vector input encoding does not match the canonicalization profile",
            ))
        }
    };

    let preimage = framed_preimage(kind, profile, &canonical_payload);
    let digest: [u8; 32] = Sha256::digest(&preimage).into();
    Ok(VectorOutcome {
        canonical_payload,
        preimage,
        digest,
        hash_id: AnyHashId::from_parts(kind, digest),
    })
}
