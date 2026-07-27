//! Hosted-stack WebSocket session tokens (`hs_token`), matching `arete-sdk` behavior.

use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::api_client::ApiClient;
use crate::config;

/// Host suffix for Arete Cloud WebSocket endpoints (see `arete_sdk::auth`).
const HOSTED_SUFFIX: &str = ".stack.arete.run";

/// Replace `hs_token` query values so session tokens are never logged, embedded in errors, or saved to snapshot headers.
pub fn redact_hs_token_for_display(url: &str) -> String {
    let Ok(mut u) = Url::parse(url) else {
        return url.to_string();
    };
    if u.query().is_none() {
        return url.to_string();
    }
    let pairs: Vec<(String, String)> = u
        .query_pairs()
        .map(|(k, v)| {
            if k == "hs_token" {
                (k.into_owned(), "<redacted>".to_string())
            } else {
                (k.into_owned(), v.into_owned())
            }
        })
        .collect();
    u.set_query(None);
    {
        let mut qp = u.query_pairs_mut();
        for (k, v) in &pairs {
            qp.append_pair(k, v);
        }
    }
    u.into()
}

#[derive(Serialize)]
struct MintBody<'a> {
    websocket_url: &'a str,
}

#[derive(Deserialize)]
struct MintResponse {
    token: String,
}

/// True if the URL targets Arete Cloud WebSockets (`*.stack.arete.run`), regardless of `hs_token`.
pub fn is_hosted_arete_cloud_url(url: &str) -> bool {
    let Ok(u) = Url::parse(url) else {
        return false;
    };
    let Some(host) = u.host_str() else {
        return false;
    };
    host.to_ascii_lowercase().ends_with(HOSTED_SUFFIX)
}

/// Returns true if this URL points at hosted Arete infrastructure and has no `hs_token` yet.
pub fn hosted_url_needs_token(url: &str) -> bool {
    let Ok(u) = Url::parse(url) else {
        return false;
    };
    let Some(host) = u.host_str() else {
        return false;
    };
    let host = host.to_ascii_lowercase();
    if !host.ends_with(HOSTED_SUFFIX) {
        return false;
    }
    !u.query_pairs().any(|(k, _)| k == "hs_token")
}

/// For `*.stack.arete.run` URLs without `hs_token`, mint a session token.
///
/// Uses `a4 auth login` credentials when available. Without stored credentials
/// the mint is attempted anonymously: the API allows keyless sessions for
/// public-tier stacks (read-only scope, IP/origin rate limits, short expiry),
/// matching `@usearete/sdk` behavior. Private/global stacks still require auth.
pub fn ensure_hosted_ws_token(url: String) -> Result<String> {
    if !hosted_url_needs_token(&url) {
        return Ok(url);
    }

    let api_key = ApiClient::load_optional_api_key().context(
        "Failed to load stored API credentials; fix ~/.arete/credentials.toml or run `a4 auth login`",
    )?;

    let base = config::get_api_url(None);
    let endpoint = format!("{}/ws/sessions", base.trim_end_matches('/'));

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .context("Failed to build HTTP client for token mint")?;
    let mut request = client.post(&endpoint).json(&MintBody {
        websocket_url: url.as_str(),
    });
    if let Some(key) = api_key.as_deref() {
        request = request.header("Authorization", format!("Bearer {}", key.trim()));
    }
    let response = request
        .send()
        .with_context(|| format!("Failed to reach token endpoint {}", endpoint))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().unwrap_or_default();
        if api_key.is_none()
            && matches!(
                status,
                reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN
            )
        {
            bail!(
                "This hosted stream requires authentication ({}): {}.\n\
                 • Run `a4 auth login`, then retry; the CLI will mint a token automatically.\n\
                 • Anonymous access is only available for public stacks.\n\
                 • Or pass `--url` with `?hs_token=...` from POST `{}`.",
                status,
                body.trim(),
                endpoint
            );
        }
        bail!(
            "Token mint failed ({}): {}.\n\
             Fix your API key (`a4 auth login`) or permissions for this stack.",
            status,
            body.trim()
        );
    }

    let mint: MintResponse = response
        .json()
        .context("Invalid JSON from /ws/sessions token endpoint")?;
    let token = mint.token.trim();
    if token.is_empty() {
        bail!("Token endpoint returned an empty token");
    }

    let mut u = Url::parse(&url).context("Invalid WebSocket URL")?;
    u.query_pairs_mut().append_pair("hs_token", token);
    Ok(u.to_string())
}
