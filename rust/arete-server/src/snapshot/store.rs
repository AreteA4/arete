//! Pluggable snapshot blob storage.
//!
//! The filesystem store (self-hosters, k8s PVCs) is always available. An
//! `object_store`-backed implementation (S3/GCS/Azure) lives in
//! [`super::object`] behind the `snapshot-object-store` cargo feature.

use anyhow::{Context, Result};
use async_trait::async_trait;
use std::path::{Path, PathBuf};

pub const SNAPSHOT_FILE_PREFIX: &str = "snapshot-";
pub const SNAPSHOT_FILE_SUFFIX: &str = ".arsnap";

/// Build the blob name for a snapshot. Zero-padded so lexicographic order
/// equals chronological order, which `load_latest`/`prune` rely on.
pub fn snapshot_name(created_at_epoch_ms: u64, resume_watermark: u64) -> String {
    format!("{SNAPSHOT_FILE_PREFIX}{created_at_epoch_ms:015}-{resume_watermark:015}{SNAPSHOT_FILE_SUFFIX}")
}

#[async_trait]
pub trait SnapshotStore: Send + Sync {
    /// Atomically persist one snapshot blob under `name`.
    async fn write(&self, name: &str, bytes: &[u8]) -> Result<()>;

    /// Return the newest snapshot blob, or `None` if the store is empty.
    async fn load_latest(&self) -> Result<Option<(String, Vec<u8>)>>;

    /// Delete all but the newest `keep` snapshots. Returns how many were removed.
    async fn prune(&self, keep: usize) -> Result<usize>;

    /// Human-readable location for logs.
    fn describe(&self) -> String;
}

/// Local-filesystem store. Writes go to a temp file in the same directory
/// followed by a rename, so a crash mid-write never corrupts the latest
/// snapshot.
pub struct FsStore {
    dir: PathBuf,
}

impl FsStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    async fn snapshot_names(&self) -> Result<Vec<String>> {
        let mut names = Vec::new();
        let mut entries = match tokio::fs::read_dir(&self.dir).await {
            Ok(entries) => entries,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(names),
            Err(err) => return Err(err).context("read snapshot directory"),
        };
        while let Some(entry) = entries.next_entry().await? {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with(SNAPSHOT_FILE_PREFIX) && name.ends_with(SNAPSHOT_FILE_SUFFIX) {
                names.push(name);
            }
        }
        // Newest first (names embed a zero-padded timestamp).
        names.sort_by(|a, b| b.cmp(a));
        Ok(names)
    }
}

#[async_trait]
impl SnapshotStore for FsStore {
    async fn write(&self, name: &str, bytes: &[u8]) -> Result<()> {
        tokio::fs::create_dir_all(&self.dir)
            .await
            .with_context(|| format!("create snapshot directory {}", self.dir.display()))?;
        let tmp_path = self.dir.join(format!(".tmp-{name}"));
        let final_path = self.dir.join(name);
        tokio::fs::write(&tmp_path, bytes)
            .await
            .with_context(|| format!("write {}", tmp_path.display()))?;
        tokio::fs::rename(&tmp_path, &final_path)
            .await
            .with_context(|| format!("rename into {}", final_path.display()))?;
        Ok(())
    }

    async fn load_latest(&self) -> Result<Option<(String, Vec<u8>)>> {
        let names = self.snapshot_names().await?;
        let Some(name) = names.into_iter().next() else {
            return Ok(None);
        };
        let path = self.dir.join(&name);
        let bytes = tokio::fs::read(&path)
            .await
            .with_context(|| format!("read {}", path.display()))?;
        Ok(Some((name, bytes)))
    }

    async fn prune(&self, keep: usize) -> Result<usize> {
        let names = self.snapshot_names().await?;
        let mut removed = 0;
        for name in names.into_iter().skip(keep.max(1)) {
            let path = self.dir.join(&name);
            if let Err(err) = tokio::fs::remove_file(&path).await {
                tracing::warn!(path = %path.display(), error = %err, "Failed to prune snapshot");
            } else {
                removed += 1;
            }
        }
        Ok(removed)
    }

    fn describe(&self) -> String {
        format!("file://{}", self.dir.display())
    }
}

/// Build a store from a URL-ish string:
/// - `file:///var/lib/arete/snapshots` or a plain path -> [`FsStore`]
/// - `s3://` / `gs://` / `az://` -> [`super::object::ObjectSnapshotStore`],
///   available behind the `snapshot-object-store` feature; rejected with a
///   clear error when the feature is not compiled in.
pub fn store_from_url(url: &str) -> Result<std::sync::Arc<dyn SnapshotStore>> {
    let trimmed = url.trim();
    if let Some(path) = trimmed.strip_prefix("file://") {
        if path.is_empty() {
            anyhow::bail!("snapshot URL {trimmed:?} has an empty path");
        }
        return Ok(std::sync::Arc::new(FsStore::new(Path::new(path))));
    }
    if trimmed.contains("://") {
        #[cfg(feature = "snapshot-object-store")]
        return super::object::from_url(trimmed);
        #[cfg(not(feature = "snapshot-object-store"))]
        {
            let scheme = trimmed.split_once("://").map(|(s, _)| s).unwrap_or_default();
            anyhow::bail!(
                "snapshot URL scheme {scheme:?} requires the `snapshot-object-store` feature, \
                 which is not enabled in this build; use a file:// URL or a local path"
            );
        }
    }
    if trimmed.is_empty() {
        anyhow::bail!("snapshot URL is empty");
    }
    Ok(std::sync::Arc::new(FsStore::new(Path::new(trimmed))))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "arete-snapshot-store-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[tokio::test]
    async fn write_load_latest_and_prune() {
        let dir = temp_dir("basic");
        let store = FsStore::new(&dir);

        assert!(store.load_latest().await.unwrap().is_none());

        for (ts, payload) in [(1u64, b"one".as_slice()), (2, b"two"), (3, b"three")] {
            store
                .write(&snapshot_name(ts, ts * 10), payload)
                .await
                .unwrap();
        }

        let (name, bytes) = store.load_latest().await.unwrap().unwrap();
        assert_eq!(name, snapshot_name(3, 30));
        assert_eq!(bytes, b"three");

        let removed = store.prune(2).await.unwrap();
        assert_eq!(removed, 1);
        assert!(!dir.join(snapshot_name(1, 10)).exists());
        assert!(dir.join(snapshot_name(2, 20)).exists());
        assert!(dir.join(snapshot_name(3, 30)).exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn ignores_foreign_files() {
        let dir = temp_dir("foreign");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("notes.txt"), b"hi").unwrap();
        std::fs::write(dir.join(".tmp-snapshot-x.arsnap.partial"), b"junk").unwrap();

        let store = FsStore::new(&dir);
        assert!(store.load_latest().await.unwrap().is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn store_from_url_variants() {
        assert!(store_from_url("file:///tmp/snaps").is_ok());
        assert!(store_from_url("/tmp/snaps").is_ok());
        assert!(store_from_url("relative/snaps").is_ok());
        // Cloud schemes only work when the object-store backend is compiled in.
        #[cfg(not(feature = "snapshot-object-store"))]
        assert!(store_from_url("s3://bucket/prefix").is_err());
        assert!(store_from_url("ftp://bucket/prefix").is_err());
        assert!(store_from_url("").is_err());
    }
}
