//! Latest-version pointer: `https://docs.arete.run/a4/latest.json`.
//!
//! Version discovery never uses the GitHub REST API (60 req/h unauthenticated)
//! and never `releases/latest` (this repo cuts many non-CLI releases). The
//! pointer is bumped by release-please (`release-please-config.json`,
//! `extra-files`) and served by the docs site.

use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;

use super::platform::latest_url;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LatestPointer {
    #[serde(default)]
    pub schema_version: u32,
    pub version: String,
}

/// Fetch the latest pointer with a hard timeout. Errors are returned, never
/// printed; callers decide whether silence is appropriate.
pub fn fetch_latest(timeout: Duration) -> Result<LatestPointer> {
    let url = latest_url();
    let client = reqwest::blocking::Client::builder()
        .timeout(timeout)
        .user_agent(format!("a4/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .context("Failed to build HTTP client")?;
    let response = client
        .get(&url)
        .send()
        .with_context(|| format!("Failed to fetch {url}"))?;
    if !response.status().is_success() {
        return Err(anyhow!("{url} returned HTTP {}", response.status()));
    }
    let pointer: LatestPointer = response
        .json()
        .with_context(|| format!("{url} is not a valid latest.json"))?;
    semver::Version::parse(&pointer.version)
        .with_context(|| format!("{url} carries a non-semver version {:?}", pointer.version))?;
    Ok(pointer)
}

/// Compare two semver strings: `Some(true)` when `latest` is newer than
/// `current`, `None` when either fails to parse.
pub fn is_newer(latest: &str, current: &str) -> Option<bool> {
    let latest = semver::Version::parse(latest).ok()?;
    let current = semver::Version::parse(current).ok()?;
    Some(latest > current)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_detection_handles_prerelease_and_garbage() {
        assert_eq!(is_newer("0.14.0", "0.13.0"), Some(true));
        assert_eq!(is_newer("0.13.0", "0.13.0"), Some(false));
        assert_eq!(is_newer("0.13.0", "0.14.0"), Some(false));
        assert_eq!(is_newer("0.13.0", "0.13.0-rc.1"), Some(true));
        assert_eq!(is_newer("latest", "0.13.0"), None);
    }
}
