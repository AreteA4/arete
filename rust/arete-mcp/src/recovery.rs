//! Safe account/recovery calls used by MCP tools.
//!
//! This module is deliberately narrow: v1 recognizes only the fixed agent
//! claim materializer and never returns or logs the API key it resolves.

use anyhow::{anyhow, Context, Result};
use arete_sdk::{ApiProblemV1, ReadyRecoveryActionV1};
use reqwest::{Method, StatusCode, Url};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::credentials;

const DEFAULT_API_URL: &str = "https://api.arete.run";
const DEFAULT_APP_ORIGIN: &str = "https://arete.run";
const MAX_RECOVERY_BODY_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AccountStatus {
    pub schema_version: u8,
    pub slug: String,
    pub display_name: String,
    pub status: String,
    pub created_at: String,
    pub last_seen_at: Option<String>,
    pub claim_state: String,
    pub plan: Option<String>,
    pub entitlement_expires_at: Option<String>,
    pub trial_access_enabled: bool,
    pub starter_guidance: Option<String>,
}

#[derive(Deserialize)]
struct AccountStatusWire {
    slug: String,
    #[serde(alias = "displayName")]
    display_name: String,
    status: String,
    #[serde(alias = "createdAt")]
    created_at: String,
    #[serde(default, alias = "lastSeenAt")]
    last_seen_at: Option<String>,
    #[serde(rename = "claimState", alias = "claim_state")]
    claim_state: String,
    #[serde(default)]
    plan: Option<String>,
    #[serde(rename = "entitlementExpiresAt", default)]
    entitlement_expires_at: Option<String>,
    #[serde(rename = "trialAccessEnabled", default)]
    trial_access_enabled: bool,
    #[serde(rename = "starterGuidance", default)]
    starter_guidance: Option<String>,
}

impl From<AccountStatusWire> for AccountStatus {
    fn from(value: AccountStatusWire) -> Self {
        Self {
            schema_version: 1,
            slug: value.slug,
            display_name: value.display_name,
            status: value.status,
            created_at: value.created_at,
            last_seen_at: value.last_seen_at,
            claim_state: value.claim_state,
            plan: value.plan,
            entitlement_expires_at: value.entitlement_expires_at,
            trial_access_enabled: value.trial_access_enabled,
            starter_guidance: value.starter_guidance,
        }
    }
}

#[derive(Debug)]
pub struct RecoveryApiError {
    pub status: StatusCode,
    pub problem: ApiProblemV1,
}

impl std::fmt::Display for RecoveryApiError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "account request failed ({}): {}",
            self.status, self.problem.error
        )
    }
}

impl std::error::Error for RecoveryApiError {}

#[derive(Clone)]
pub struct RecoveryClient {
    base_url: String,
    app_origin: Url,
    http: reqwest::Client,
}

impl RecoveryClient {
    pub fn new() -> Self {
        let base_url = std::env::var("ARETE_API_URL")
            .ok()
            .map(|value| value.trim().trim_end_matches('/').to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_API_URL.to_string());
        let app_origin = std::env::var("ARETE_APP_ORIGIN")
            .ok()
            .and_then(|value| parse_app_origin(&value).ok())
            .unwrap_or_else(|| default_app_origin(&base_url));
        Self {
            base_url,
            app_origin,
            http: reqwest::Client::new(),
        }
    }

    pub async fn account_status(&self) -> Result<AccountStatus> {
        let status: AccountStatusWire = self.request_json(Method::GET, "/api/agents/me").await?;
        Ok(status.into())
    }

    pub async fn create_claim_link(&self) -> Result<ReadyRecoveryActionV1> {
        let ready: ReadyRecoveryActionV1 = self
            .request_json(Method::POST, "/api/agents/me/claim-links")
            .await?;
        if !ready.action.is_safe_claim_url_for_origin(&self.app_origin) {
            return Err(anyhow!("server returned an unsafe agent claim URL"));
        }
        Ok(ready)
    }

    async fn request_json<T: DeserializeOwned>(&self, method: Method, path: &str) -> Result<T> {
        validate_api_origin(&self.base_url)?;
        let key = credentials::resolve(None, "")?
            .key
            .ok_or_else(|| anyhow!("no Arete API key is configured"))?;
        let response = self
            .http
            .request(method, format!("{}{path}", self.base_url))
            .bearer_auth(key)
            .send()
            .await
            .context("account request failed")?;
        let status = response.status();
        if response.content_length().is_some_and(|length| {
            length > u64::try_from(MAX_RECOVERY_BODY_BYTES).unwrap_or(u64::MAX)
        }) {
            return Err(anyhow!("account response exceeded safe size limit"));
        }
        let body = response
            .bytes()
            .await
            .context("could not read account response")?;
        if body.len() > MAX_RECOVERY_BODY_BYTES {
            return Err(anyhow!("account response exceeded safe size limit"));
        }
        if !status.is_success() {
            let problem =
                serde_json::from_slice::<ApiProblemV1>(&body).unwrap_or_else(|_| ApiProblemV1 {
                    schema_version: None,
                    error: status
                        .canonical_reason()
                        .unwrap_or("Account request failed")
                        .to_string(),
                    code: None,
                    retryable: status.is_server_error().then_some(true),
                    request_id: None,
                    retry_after_seconds: None,
                    usage: None,
                    action: None,
                    extra: Default::default(),
                });
            return Err(RecoveryApiError { status, problem }.into());
        }
        serde_json::from_slice(&body).context("account endpoint returned invalid JSON")
    }
}

fn default_app_origin(base_url: &str) -> Url {
    let uses_loopback_api = Url::parse(base_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
        .is_some_and(|host| {
            host.eq_ignore_ascii_case("localhost")
                || host
                    .trim_matches(['[', ']'])
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|address| address.is_loopback())
        });
    Url::parse(if uses_loopback_api {
        "http://localhost:3000"
    } else {
        DEFAULT_APP_ORIGIN
    })
    .expect("static app origin")
}

fn parse_app_origin(value: &str) -> Result<Url> {
    let mut url = Url::parse(value.trim()).context("invalid ARETE_APP_ORIGIN")?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(anyhow!(
            "ARETE_APP_ORIGIN must not contain credentials, query, or fragment"
        ));
    }
    url.set_path("");
    Ok(url)
}

fn validate_api_origin(value: &str) -> Result<()> {
    let url = Url::parse(value).context("invalid ARETE_API_URL")?;
    let host = url
        .host_str()
        .ok_or_else(|| anyhow!("ARETE_API_URL has no host"))?
        .trim_end_matches('.')
        .to_ascii_lowercase();
    let loopback = host == "localhost"
        || host
            .trim_matches(['[', ']'])
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback());
    let safe = match url.scheme() {
        "https" => loopback || host == "arete.run" || host.ends_with(".arete.run"),
        "http" => loopback,
        _ => false,
    };
    if !safe
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(anyhow!(
            "refusing to send an Arete credential to an untrusted API origin"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_origin_allowlist_accepts_arete_and_loopback_only() {
        assert!(validate_api_origin("https://api.arete.run").is_ok());
        assert!(validate_api_origin("http://127.0.0.1:8080").is_ok());
        assert!(validate_api_origin("https://evil.example").is_err());
        assert!(validate_api_origin("http://api.arete.run").is_err());
        assert!(validate_api_origin("https://user:pass@api.arete.run").is_err());
    }

    #[test]
    fn loopback_api_defaults_to_the_local_claim_app() {
        assert_eq!(
            default_app_origin("http://127.0.0.1:8080").as_str(),
            "http://localhost:3000/"
        );
        assert_eq!(
            default_app_origin("https://api.arete.run").as_str(),
            "https://arete.run/"
        );
    }

    #[test]
    fn account_status_reports_trial_fields_and_defaults_old_responses_safely() {
        let current: AccountStatus = serde_json::from_str::<AccountStatusWire>(
            r#"{"slug":"agent-1","display_name":"Agent One","status":"active","created_at":"2026-09-29T00:00:00Z","last_seen_at":null,"claimState":"unclaimed","plan":"agent_trial","entitlementExpiresAt":"2026-10-06T00:00:00Z","trialAccessEnabled":true,"starterGuidance":"Use starter stacks; public stacks remain available."}"#,
        )
        .unwrap()
        .into();
        let value = serde_json::to_value(&current).unwrap();
        assert_eq!(value["schemaVersion"], 1);
        assert_eq!(value["plan"], "agent_trial");
        assert_eq!(value["trialAccessEnabled"], true);
        assert_eq!(value["claimState"], "unclaimed");
        assert!(value["starterGuidance"]
            .as_str()
            .unwrap()
            .contains("public stacks"));

        let old: AccountStatus = serde_json::from_str::<AccountStatusWire>(
            r#"{"slug":"agent-1","display_name":"Agent One","status":"active","created_at":"2026-09-29T00:00:00Z","claimState":"unclaimed"}"#,
        )
        .unwrap()
        .into();
        assert_eq!(old.plan, None);
        assert_eq!(old.entitlement_expires_at, None);
        assert!(!old.trial_access_enabled);
        assert_eq!(old.starter_guidance, None);
    }
}
