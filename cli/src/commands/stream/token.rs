//! Hosted-stack WebSocket session tokens (`hs_token`), matching `hyperstack-sdk` behavior.

use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::api_client::ApiClient;
use crate::config;

/// Host suffix for Hyperstack Cloud WebSocket endpoints (see `hyperstack_sdk::auth`).
const HOSTED_SUFFIX: &str = ".stack.usehyperstack.com";

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

/// True if the URL targets Hyperstack Cloud WebSockets (`*.stack.usehyperstack.com`), regardless of `hs_token`.
pub fn is_hosted_hyperstack_cloud_url(url: &str) -> bool {
    let Ok(u) = Url::parse(url) else {
        return false;
    };
    let Some(host) = u.host_str() else {
        return false;
    };
    host.to_ascii_lowercase().ends_with(HOSTED_SUFFIX)
}

/// Returns true if this URL points at hosted Hyperstack infrastructure and has no `hs_token` yet.
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

/// For `*.stack.usehyperstack.com` URLs without `hs_token`, mint a session using `hs auth login` credentials.
pub fn ensure_hosted_ws_token(url: String) -> Result<String> {
    if !hosted_url_needs_token(&url) {
        return Ok(url);
    }

    let api_key = ApiClient::load_api_key().context(
        "Hosted Hyperstack streams require a WebSocket session token.\n\
         • Run `hs auth login`, then retry; the CLI will mint a token automatically.\n\
         • Or pass `--url` with `?hs_token=...` from POST `https://api.usehyperstack.com/ws/sessions` (or your `HYPERSTACK_API_URL`).",
    )?;

    let base = config::get_api_url(None);
    let endpoint = format!("{}/ws/sessions", base.trim_end_matches('/'));

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .context("Failed to build HTTP client for token mint")?;
    let response = client
        .post(&endpoint)
        .header("Authorization", format!("Bearer {}", api_key.trim()))
        .json(&MintBody {
            websocket_url: url.as_str(),
        })
        .send()
        .with_context(|| format!("Failed to reach token endpoint {}", endpoint))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().unwrap_or_default();
        bail!(
            "Token mint failed ({}): {}.\n\
             Fix your API key (`hs auth login`) or permissions for this stack.",
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
