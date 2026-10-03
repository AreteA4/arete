//! The machine-wide cache of immutable registry artifacts. `a4 install` stores
//! every StackManifest, LiveSpec, ProgramSpec and SDK extension it resolves as
//! `<kind>/<content hash>.json`; `a4 up` deploys an installed stack from it.

use std::path::{Path, PathBuf};

use anyhow::{bail, Result};

/// `~/.arete/cache/registry/v1`.
pub fn root() -> Result<PathBuf> {
    Ok(dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("Could not determine Arete cache directory"))?
        .join(".arete")
        .join("cache")
        .join("registry")
        .join("v1"))
}

/// The cache file of one artifact. The hash names the file, so it must be a
/// plain content identity: no separators, no parent references.
pub fn file(root: &Path, kind: &str, hash: &str) -> Result<PathBuf> {
    if hash.is_empty()
        || !hash
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b':' | b'-' | b'_'))
    {
        bail!("Cannot cache invalid {kind} identity '{hash}'");
    }
    Ok(root.join(kind).join(format!("{hash}.json")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_files_are_named_by_their_content_identity() {
        let root = Path::new("/cache");
        assert_eq!(
            file(root, "live-spec", "arete:h1:live-spec:sha256:abc").unwrap(),
            root.join("live-spec/arete:h1:live-spec:sha256:abc.json")
        );
        for hash in ["", "../escape", "a/b", "a b", "a.json"] {
            assert!(file(root, "live-spec", hash).is_err(), "{hash:?}");
        }
    }
}
