//! `object_store`-backed snapshot storage (S3/GCS/Azure), behind the
//! `snapshot-object-store` cargo feature.
//!
//! Cloud credentials come from the standard environment for each provider
//! (`AWS_*`, `GOOGLE_*`, `AZURE_*` variables, instance/workload identity):
//! each builder is initialized with `from_env()`. Object PUTs are atomic on
//! all supported backends, which gives the same crash-safety as the
//! filesystem store's temp-file + rename.

use super::store::{SnapshotStore, SNAPSHOT_FILE_PREFIX, SNAPSHOT_FILE_SUFFIX};
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use futures_util::TryStreamExt;
use object_store::path::Path as ObjectPath;
use object_store::{ObjectStore, PutPayload};
use std::sync::Arc;

/// Snapshot store over any [`object_store::ObjectStore`], writing blobs under
/// a fixed key prefix (one writer per prefix, matching `replicas: 1` atoms).
pub struct ObjectSnapshotStore {
    store: Arc<dyn ObjectStore>,
    prefix: ObjectPath,
    /// Human-readable location for logs (the configured URL).
    location: String,
}

impl ObjectSnapshotStore {
    pub fn new(
        store: Arc<dyn ObjectStore>,
        prefix: ObjectPath,
        location: impl Into<String>,
    ) -> Self {
        Self {
            store,
            prefix,
            location: location.into(),
        }
    }

    fn path_for(&self, name: &str) -> ObjectPath {
        self.prefix.child(name)
    }

    /// Snapshot blob names under the prefix, newest first.
    async fn snapshot_names(&self) -> Result<Vec<String>> {
        let mut names: Vec<String> = self
            .store
            .list(Some(&self.prefix))
            .try_filter_map(|meta| async move {
                let name = meta.location.filename().map(str::to_string);
                Ok(name.filter(|name| {
                    name.starts_with(SNAPSHOT_FILE_PREFIX) && name.ends_with(SNAPSHOT_FILE_SUFFIX)
                }))
            })
            .try_collect()
            .await
            .context("list snapshot objects")?;
        // Newest first (names embed a zero-padded timestamp).
        names.sort_by(|a, b| b.cmp(a));
        Ok(names)
    }
}

#[async_trait]
impl SnapshotStore for ObjectSnapshotStore {
    async fn write(&self, name: &str, bytes: &[u8]) -> Result<()> {
        let path = self.path_for(name);
        self.store
            .put(&path, PutPayload::from(bytes.to_vec()))
            .await
            .with_context(|| format!("put {path}"))?;
        Ok(())
    }

    async fn load_latest(&self) -> Result<Option<(String, Vec<u8>)>> {
        let names = self.snapshot_names().await?;
        let Some(name) = names.into_iter().next() else {
            return Ok(None);
        };
        let path = self.path_for(&name);
        let bytes = self
            .store
            .get(&path)
            .await
            .with_context(|| format!("get {path}"))?
            .bytes()
            .await
            .with_context(|| format!("read {path}"))?;
        Ok(Some((name, bytes.to_vec())))
    }

    async fn prune(&self, keep: usize) -> Result<usize> {
        let names = self.snapshot_names().await?;
        let mut removed = 0;
        for name in names.into_iter().skip(keep.max(1)) {
            let path = self.path_for(&name);
            if let Err(err) = self.store.delete(&path).await {
                tracing::warn!(%path, error = %err, "Failed to prune snapshot object");
            } else {
                removed += 1;
            }
        }
        Ok(removed)
    }

    fn describe(&self) -> String {
        self.location.clone()
    }
}

/// Build an object store from a cloud URL like `s3://bucket/prefix`,
/// `gs://bucket/prefix`, or `az://container/prefix`. The URL's host selects
/// the bucket/container; its path becomes the key prefix.
pub(crate) fn from_url(url_str: &str) -> Result<Arc<dyn SnapshotStore>> {
    let url =
        url::Url::parse(url_str).with_context(|| format!("invalid snapshot URL {url_str:?}"))?;

    let store: Arc<dyn ObjectStore> = match url.scheme() {
        "s3" | "s3a" => Arc::new(
            object_store::aws::AmazonS3Builder::from_env()
                .with_url(url_str)
                .build()
                .context("configure S3 snapshot store")?,
        ),
        "gs" => Arc::new(
            object_store::gcp::GoogleCloudStorageBuilder::from_env()
                .with_url(url_str)
                .build()
                .context("configure GCS snapshot store")?,
        ),
        "az" | "azure" | "abfs" | "abfss" => Arc::new(
            object_store::azure::MicrosoftAzureBuilder::from_env()
                .with_url(url_str)
                .build()
                .context("configure Azure snapshot store")?,
        ),
        other => bail!(
            "unsupported snapshot URL scheme {other:?}; expected s3://, gs://, az://, or file://"
        ),
    };

    let prefix = ObjectPath::parse(url.path())
        .with_context(|| format!("invalid snapshot key prefix in {url_str:?}"))?;
    Ok(Arc::new(ObjectSnapshotStore::new(store, prefix, url_str)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::store::snapshot_name;
    use object_store::memory::InMemory;

    fn memory_store() -> ObjectSnapshotStore {
        ObjectSnapshotStore::new(
            Arc::new(InMemory::new()),
            ObjectPath::from("snapshots/stack1"),
            "mem://snapshots/stack1",
        )
    }

    #[tokio::test]
    async fn write_load_latest_and_prune() {
        let store = memory_store();

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

        let names = store.snapshot_names().await.unwrap();
        assert_eq!(names, vec![snapshot_name(3, 30), snapshot_name(2, 20)]);
    }

    #[tokio::test]
    async fn ignores_foreign_objects_under_the_prefix() {
        let inner: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
        inner
            .put(
                &ObjectPath::from("snapshots/stack1/notes.txt"),
                PutPayload::from_static(b"hi"),
            )
            .await
            .unwrap();
        // Same bucket, different stack prefix: must be invisible.
        inner
            .put(
                &ObjectPath::from(format!("snapshots/stack2/{}", snapshot_name(9, 90))),
                PutPayload::from_static(b"other"),
            )
            .await
            .unwrap();

        let store = ObjectSnapshotStore::new(
            inner,
            ObjectPath::from("snapshots/stack1"),
            "mem://snapshots/stack1",
        );
        assert!(store.load_latest().await.unwrap().is_none());

        store.write(&snapshot_name(1, 10), b"mine").await.unwrap();
        let (name, bytes) = store.load_latest().await.unwrap().unwrap();
        assert_eq!(name, snapshot_name(1, 10));
        assert_eq!(bytes, b"mine");
    }

    #[test]
    fn from_url_rejects_unknown_schemes_and_accepts_s3() {
        assert!(from_url("ftp://bucket/prefix").is_err());
        // Credentials are only used at first request, but the builder needs a
        // region to construct; no other test reads these variables.
        std::env::set_var("AWS_REGION", "us-east-1");
        assert!(from_url("s3://bucket/some/prefix").is_ok());
        std::env::remove_var("AWS_REGION");
    }
}
