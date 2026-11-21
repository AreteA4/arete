//! Which build of the Arete API a client is talking to.
//!
//! The API answers `GET /version` without a key: its service name, crate
//! version, and the commit and image digest it was deployed from (`null`
//! when the deploy did not say). The MCP `version` tool and `a4 version`
//! report it next to their own version, so a bug report names both builds.
//! An API without the endpoint (older deploys) is reported as such, not as
//! an error of the client.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::credentials::{normalize_api_url, DEFAULT_API_URL};

/// The unauthenticated path that reports the API's build.
pub const VERSION_PATH: &str = "/version";
/// How long a version read may take before it is reported as unreachable.
pub const TIMEOUT: Duration = Duration::from_secs(10);

/// What `GET /version` answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiBuild {
    #[serde(default)]
    pub service: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    /// The commit the running API was built from.
    #[serde(default)]
    pub commit: Option<String>,
    /// The digest of the running API image.
    #[serde(default)]
    pub image_digest: Option<String>,
}

/// Why the API's build could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiBuildError {
    /// The API has no version endpoint: a deploy from before it existed.
    NotReported,
    /// The API did not answer, or answered something else.
    Unavailable(String),
}

impl std::fmt::Display for ApiBuildError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotReported => formatter.write_str("this API does not report its build"),
            Self::Unavailable(reason) => {
                write!(formatter, "the API's build could not be read: {reason}")
            }
        }
    }
}

/// The API this process talks to: `ARETE_API_URL`, else the default.
pub fn configured_api_url() -> String {
    std::env::var("ARETE_API_URL")
        .ok()
        .map(|url| normalize_api_url(&url))
        .filter(|url| !url.is_empty())
        .unwrap_or_else(|| DEFAULT_API_URL.to_string())
}

/// The version URL of the API at `api_url`.
pub fn version_url(api_url: &str) -> String {
    format!("{}{VERSION_PATH}", normalize_api_url(api_url))
}

/// Read an answer to `GET /version`.
pub fn parse_response(status: u16, body: &str) -> Result<ApiBuild, ApiBuildError> {
    match status {
        200 => serde_json::from_str(body)
            .map_err(|error| ApiBuildError::Unavailable(format!("unexpected answer: {error}"))),
        404 | 405 => Err(ApiBuildError::NotReported),
        status => Err(ApiBuildError::Unavailable(format!("HTTP {status}"))),
    }
}

/// `GET /version` of the API at `api_url`. No key is sent.
pub async fn fetch(client: &reqwest::Client, api_url: &str) -> Result<ApiBuild, ApiBuildError> {
    let response = client
        .get(version_url(api_url))
        .timeout(TIMEOUT)
        .send()
        .await
        .map_err(|error| ApiBuildError::Unavailable(error.without_url().to_string()))?;
    let status = response.status().as_u16();
    let body = response
        .text()
        .await
        .map_err(|error| ApiBuildError::Unavailable(error.without_url().to_string()))?;
    parse_response(status, &body)
}

/// The `api` member of a version report: the API's URL and build, or why
/// the build is unknown.
pub fn api_report(api_url: &str, build: &Result<ApiBuild, ApiBuildError>) -> Value {
    let url = normalize_api_url(api_url);
    match build {
        Ok(build) => json!({
            "url": url,
            "service": build.service,
            "version": build.version,
            "commit": build.commit,
            "imageDigest": build.image_digest,
        }),
        Err(error) => json!({ "url": url, "error": error.to_string() }),
    }
}

/// One line about the API's build, for humans: `arete-api 0.1.0 at
/// 85c265de0f1a, image sha256:0123456789ab (https://api.arete.run)`.
/// The commit and digest are shortened by characters, never bytes, so an
/// unexpected answer cannot split a character.
pub fn api_line(api_url: &str, build: &Result<ApiBuild, ApiBuildError>) -> String {
    let url = normalize_api_url(api_url);
    let short = |text: &str, length: usize| text.chars().take(length).collect::<String>();
    match build {
        Ok(build) => {
            let name = build.service.as_deref().unwrap_or("API");
            let version = build
                .version
                .as_deref()
                .map(|version| format!(" {version}"))
                .unwrap_or_default();
            let commit = match &build.commit {
                Some(commit) => format!(" at {}", short(commit, 12)),
                None => ", commit not reported".to_string(),
            };
            let image = build
                .image_digest
                .as_deref()
                .map(|digest| format!(", image {}", short(digest, "sha256:".len() + 12)))
                .unwrap_or_default();
            format!("{name}{version}{commit}{image} ({url})")
        }
        Err(error) => format!("{error} ({url})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMMIT: &str = "85c265de0f1a2b3c4d5e6f708192a3b4c5d6e7f8";

    #[test]
    fn reads_the_build_an_api_reports() {
        let body = format!(
            r#"{{"service":"arete-api","version":"0.1.0","commit":"{COMMIT}","imageDigest":"sha256:ab"}}"#
        );
        let build = parse_response(200, &body).unwrap();
        assert_eq!(build.commit.as_deref(), Some(COMMIT));
        let build = Ok(build);
        assert_eq!(
            api_report("https://api.arete.run/", &build),
            json!({
                "url": "https://api.arete.run",
                "service": "arete-api",
                "version": "0.1.0",
                "commit": COMMIT,
                "imageDigest": "sha256:ab",
            })
        );
        assert_eq!(
            api_line("https://api.arete.run", &build),
            "arete-api 0.1.0 at 85c265de0f1a, image sha256:ab (https://api.arete.run)"
        );
    }

    #[test]
    fn the_digest_names_a_build_without_a_commit() {
        let digest = format!("sha256:{}", "0123456789abcdef".repeat(4));
        let build = Ok(ApiBuild {
            service: Some("arete-api".into()),
            version: None,
            commit: None,
            image_digest: Some(digest),
        });
        assert_eq!(
            api_line("https://api.arete.run", &build),
            "arete-api, commit not reported, image sha256:0123456789ab (https://api.arete.run)"
        );
    }

    #[test]
    fn unexpected_commit_text_is_shortened_by_characters() {
        let build = parse_response(
            200,
            r#"{"commit":"12345678901é234","imageDigest":"sha256:ééé"}"#,
        );
        assert_eq!(
            api_line("https://api.arete.run", &build),
            "API at 12345678901é, image sha256:ééé (https://api.arete.run)"
        );
    }

    #[test]
    fn a_build_without_a_commit_says_so() {
        let build = parse_response(
            200,
            r#"{"service":"arete-api","version":"0.1.0","commit":null,"imageDigest":null}"#,
        );
        assert_eq!(
            api_line("http://localhost:3000", &build),
            "arete-api 0.1.0, commit not reported (http://localhost:3000)"
        );
        assert!(api_report("http://localhost:3000", &build)["commit"].is_null());
    }

    #[test]
    fn an_api_without_the_endpoint_or_answering_badly_is_reported() {
        assert_eq!(
            parse_response(404, "Not Found"),
            Err(ApiBuildError::NotReported)
        );
        assert!(matches!(
            parse_response(503, ""),
            Err(ApiBuildError::Unavailable(reason)) if reason == "HTTP 503"
        ));
        assert!(matches!(
            parse_response(200, "OK"),
            Err(ApiBuildError::Unavailable(_))
        ));
        let report = api_report("https://api.arete.run", &Err(ApiBuildError::NotReported));
        assert_eq!(
            report,
            json!({ "url": "https://api.arete.run", "error": "this API does not report its build" })
        );
    }

    #[test]
    fn the_version_url_is_on_the_api_origin() {
        assert_eq!(
            version_url("https://api.arete.run/"),
            "https://api.arete.run/version"
        );
    }
}
