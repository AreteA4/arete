use std::collections::HashSet;

use crate::{
    framed_tuple_payload, hash_canonical_payload, hash_raw_bytes, require_profile, ArtifactFile,
    CanonicalizationProfile, HashError, HashId, Kind, TupleField,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactEntryKind {
    File,
    Symlink,
}

#[derive(Debug, Clone, Copy)]
pub struct ArtifactTreeEntry<'a> {
    pub path: &'a str,
    pub bytes: &'a [u8],
    pub kind: ArtifactEntryKind,
}

impl<'a> ArtifactTreeEntry<'a> {
    pub const fn file(path: &'a str, bytes: &'a [u8]) -> Self {
        Self {
            path,
            bytes,
            kind: ArtifactEntryKind::File,
        }
    }

    pub const fn symlink(path: &'a str) -> Self {
        Self {
            path,
            bytes: &[],
            kind: ArtifactEntryKind::Symlink,
        }
    }
}

pub fn validate_artifact_path(path: &str) -> Result<(), HashError> {
    let invalid = |reason| HashError::InvalidArtifactPath {
        path: path.to_string(),
        reason,
    };

    if path.is_empty() {
        return Err(invalid("path must contain at least one segment"));
    }
    if path.starts_with('/') || path.ends_with('/') {
        return Err(invalid("leading and trailing slashes are forbidden"));
    }
    if path.contains("//") {
        return Err(invalid("repeated slashes are forbidden"));
    }
    if path.contains('\\') {
        return Err(invalid("backslashes are forbidden"));
    }
    if path.contains('\0') {
        return Err(invalid("NUL bytes are forbidden"));
    }
    if path
        .split('/')
        .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return Err(invalid("empty, '.' and '..' segments are forbidden"));
    }
    Ok(())
}

pub fn artifact_tree_payload(entries: &[ArtifactTreeEntry<'_>]) -> Result<Vec<u8>, HashError> {
    let mut entries = entries.to_vec();
    for entry in &entries {
        validate_artifact_path(entry.path)?;
        if entry.kind == ArtifactEntryKind::Symlink {
            return Err(HashError::SymlinkArtifact(entry.path.to_string()));
        }
    }
    entries.sort_by(|left, right| left.path.as_bytes().cmp(right.path.as_bytes()));

    let mut seen = HashSet::with_capacity(entries.len());
    let mut payload = Vec::new();
    payload.extend_from_slice(&(entries.len() as u64).to_be_bytes());
    for entry in entries {
        if !seen.insert(entry.path) {
            return Err(HashError::DuplicateArtifactPath(entry.path.to_string()));
        }
        let file_hash = hash_raw_bytes::<ArtifactFile>(entry.bytes)?.to_string();
        let leaf = framed_tuple_payload(&[
            TupleField::new("path", entry.path.as_bytes()),
            TupleField::new("fileHash", file_hash.as_bytes()),
        ])?;
        payload.extend_from_slice(&leaf);
    }
    Ok(payload)
}

pub fn hash_artifact_tree<K: Kind>(
    entries: &[ArtifactTreeEntry<'_>],
) -> Result<HashId<K>, HashError> {
    require_profile::<K>(CanonicalizationProfile::ArtifactTreeV1)?;
    let payload = artifact_tree_payload(entries)?;
    Ok(hash_canonical_payload(&payload))
}
