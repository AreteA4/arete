//! Release manifest (`manifest.json` on every `a4-cli-v<version>` release)
//! and the HTTP download helpers shared by `a4 self update`.
//!
//! The manifest is written by `scripts/write-release-manifest.sh` from the
//! same `checksums.txt` that installers verify; it exists so readers can
//! learn the asset names, the checksum/signature file names and a future
//! `minimumVersion` without scraping the GitHub API. Readers must tolerate a
//! `null`/absent `minimumVersion`.
//!
//! A 404 while a release is being cut is expected (the docs site can bump
//! `latest.json` minutes before the binaries finish uploading), so it maps to
//! [`DownloadError::StillPublishing`] instead of a generic download error.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::Path;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

use super::platform::release_base_url;

pub const MANIFEST_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestAsset {
    pub name: String,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    #[serde(default)]
    pub schema_version: u32,
    pub name: String,
    pub version: String,
    pub tag: String,
    #[serde(default)]
    pub released_at: Option<String>,
    #[serde(default)]
    pub assets: BTreeMap<String, ManifestAsset>,
    #[serde(default = "default_checksums_name")]
    pub checksums: String,
    #[serde(default = "default_signature_name")]
    pub signature: String,
    #[serde(default)]
    pub minimum_version: Option<String>,
}

fn default_checksums_name() -> String {
    "checksums.txt".to_string()
}

fn default_signature_name() -> String {
    "checksums.txt.minisig".to_string()
}

impl Manifest {
    pub fn parse(text: &str) -> Result<Manifest> {
        serde_json::from_str(text).context("manifest.json is not a valid release manifest")
    }
}

/// Why a release file could not be fetched.
#[derive(Debug)]
pub enum DownloadError {
    /// HTTP 404: the release exists in `latest.json` but its assets are not
    /// uploaded yet (or the version does not exist at all).
    StillPublishing { version: String, url: String },
    /// Any other HTTP status.
    Status { url: String, status: u16 },
}

impl fmt::Display for DownloadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DownloadError::StillPublishing { version, url } => write!(
                f,
                "Release {version} is still publishing; retry in a few minutes (HTTP 404 for {url})"
            ),
            DownloadError::Status { url, status } => write!(f, "{url} returned HTTP {status}"),
        }
    }
}

impl std::error::Error for DownloadError {}

/// URL of a file on the `a4-cli-v<version>` release (honours `A4_MANIFEST_BASE_URL`).
pub fn release_file_url(version: &str, file_name: &str) -> String {
    format!("{}/{file_name}", release_base_url(version))
}

/// Blocking HTTP client for release downloads: follows redirects (GitHub
/// release assets redirect to object storage) and has a generous timeout for
/// 20–30 MB binaries.
pub fn download_client(timeout: Duration) -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::limited(10))
        .user_agent(format!("a4/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .context("Failed to build HTTP client")
}

/// GET a release file into memory. 404 becomes [`DownloadError::StillPublishing`].
pub fn fetch_release_bytes(
    client: &reqwest::blocking::Client,
    version: &str,
    file_name: &str,
) -> Result<Vec<u8>> {
    let url = release_file_url(version, file_name);
    let response = client
        .get(&url)
        .send()
        .with_context(|| format!("Failed to download {url}"))?;
    let status = response.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        return Err(DownloadError::StillPublishing {
            version: version.to_string(),
            url,
        }
        .into());
    }
    if !status.is_success() {
        return Err(DownloadError::Status {
            url,
            status: status.as_u16(),
        }
        .into());
    }
    let bytes = response
        .bytes()
        .with_context(|| format!("Failed to read {url}"))?;
    Ok(bytes.to_vec())
}

/// GET a release file to `dest` (parent directory must exist).
pub fn download_release_file(
    client: &reqwest::blocking::Client,
    version: &str,
    file_name: &str,
    dest: &Path,
) -> Result<()> {
    let bytes = fetch_release_bytes(client, version, file_name)?;
    fs::write(dest, bytes).with_context(|| format!("Failed to write {}", dest.display()))?;
    Ok(())
}

/// Fetch and parse `manifest.json` for a version.
pub fn fetch_manifest(client: &reqwest::blocking::Client, version: &str) -> Result<Manifest> {
    let bytes = fetch_release_bytes(client, version, "manifest.json")?;
    let text = String::from_utf8(bytes).map_err(|_| anyhow!("manifest.json is not UTF-8"))?;
    Manifest::parse(&text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_tolerates_null_minimum_version_and_defaults() {
        let text = r#"{
          "schemaVersion": 1, "name": "a4", "version": "0.13.0", "tag": "a4-cli-v0.13.0",
          "releasedAt": "2026-09-10T12:00:00Z",
          "assets": { "linux-x64": { "name": "a4-linux-x64", "sha256": "ab" } },
          "checksums": "checksums.txt", "signature": "checksums.txt.minisig",
          "minimumVersion": null
        }"#;
        let manifest = Manifest::parse(text).unwrap();
        assert_eq!(manifest.minimum_version, None);
        assert_eq!(manifest.assets["linux-x64"].name, "a4-linux-x64");

        let minimal = r#"{ "name": "a4", "version": "0.13.0", "tag": "a4-cli-v0.13.0" }"#;
        let manifest = Manifest::parse(minimal).unwrap();
        assert_eq!(manifest.checksums, "checksums.txt");
        assert_eq!(manifest.signature, "checksums.txt.minisig");
        assert!(manifest.assets.is_empty());
    }

    #[test]
    fn still_publishing_message_names_the_version() {
        let error = DownloadError::StillPublishing {
            version: "0.13.0".into(),
            url: "https://example.invalid/manifest.json".into(),
        };
        assert!(error
            .to_string()
            .starts_with("Release 0.13.0 is still publishing; retry in a few minutes"));
    }
}
