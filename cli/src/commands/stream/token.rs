//! Hosted-stack WebSocket session tokens (`hs_token`), matching `arete-sdk` behavior.

use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use url::Url;

use crate::api_client::ApiClient;
use crate::config;

/// Which host suffixes count as Arete Cloud comes from the SDK, so the CLI
/// and the SDKs agree on what needs a session. A deployment served outside
/// the default suffix - the live service's test domain, say - is otherwise
/// not recognised here, no token is minted, and the connection is refused
/// with a 401 that looks like bad credentials rather than an unknown host.
use arete_sdk::is_hosted_websocket_host;

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
    /// Seconds since the epoch.
    #[serde(default)]
    expires_at: Option<u64>,
}

/// How long before expiry a minted token is replaced.
const REFRESH_LEAD: Duration = Duration::from_secs(60);
/// How long to wait before minting again after a failed attempt.
const REFRESH_RETRY: Duration = Duration::from_secs(15);

/// What it takes to mint the next token for a hosted stream.
///
/// Only a token the CLI minted itself can be refreshed; one passed in with
/// `--url ...?hs_token=` lasts as long as it lasts.
#[derive(Debug, Clone)]
pub struct SessionRefresh {
    endpoint: String,
    api_key: Option<String>,
    /// The stack's URL without `hs_token`; every token is minted for it.
    websocket_url: String,
    expires_at: Option<u64>,
}

/// What the refresher hands the socket loop.
pub enum RefreshEvent {
    /// A new token to send as `refresh_auth` on the open socket.
    Token(String),
    /// Minting failed; it is tried again shortly.
    Failed(anyhow::Error),
}

/// Keeps a hosted stream's session token fresh while the socket is open.
///
/// A background task mints a replacement shortly before the current token
/// expires; the socket loop sends it as `refresh_auth`, and the server swaps
/// it in without reconnecting. Without a [`SessionRefresh`] (or once the
/// task stops), [`next`](Self::next) never resolves.
pub struct SessionRefresher {
    events: Option<mpsc::Receiver<RefreshEvent>>,
}

impl SessionRefresher {
    pub fn start(refresh: Option<SessionRefresh>) -> Self {
        Self {
            events: refresh.map(spawn_refresh),
        }
    }

    pub async fn next(&mut self) -> RefreshEvent {
        if let Some(events) = &mut self.events {
            if let Some(event) = events.recv().await {
                return event;
            }
            self.events = None;
        }
        std::future::pending().await
    }
}

fn spawn_refresh(mut refresh: SessionRefresh) -> mpsc::Receiver<RefreshEvent> {
    let (events, receiver) = mpsc::channel(1);
    tokio::spawn(async move {
        let mut retry = false;
        loop {
            let delay = match (retry, refresh.expires_at) {
                (true, _) => REFRESH_RETRY,
                (false, Some(expires_at)) => refresh_delay(expires_at, unix_now()),
                // The endpoint gave no expiry, so there is nothing to schedule.
                (false, None) => return,
            };
            tokio::select! {
                _ = tokio::time::sleep(delay) => {}
                _ = events.closed() => return,
            }
            let next = refresh.clone();
            let minted = tokio::task::spawn_blocking(move || {
                mint_session_token(&next.endpoint, next.api_key.as_deref(), &next.websocket_url)
            })
            .await
            .map_err(anyhow::Error::from)
            .and_then(|minted| minted);
            let event = match minted {
                Ok(minted) => {
                    retry = false;
                    refresh.expires_at = minted.expires_at;
                    RefreshEvent::Token(minted.token)
                }
                Err(error) => {
                    retry = true;
                    RefreshEvent::Failed(error)
                }
            };
            if events.send(event).await.is_err() {
                return;
            }
        }
    });
    receiver
}

/// Time until a token expiring at `expires_at` should be replaced: a minute
/// ahead, or halfway through what is left when that is shorter, so a
/// short-lived token is not re-minted in a tight loop.
fn refresh_delay(expires_at: u64, now: u64) -> Duration {
    let remaining = Duration::from_secs(expires_at.saturating_sub(now));
    let lead = REFRESH_LEAD.min(remaining / 2);
    remaining.saturating_sub(lead).max(Duration::from_secs(1))
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// The server's answer to `refresh_auth`. It carries no `type`, so it is
/// told apart from frames by having exactly these fields.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RefreshAuthResponse {
    pub success: bool,
    #[serde(default)]
    pub error: Option<String>,
    /// Declared so the match stays exact; the refresher already knows when
    /// the token it minted expires.
    #[serde(default, rename = "expiresAt")]
    _expires_at: Option<serde::de::IgnoredAny>,
}

pub fn parse_refresh_response(text: &str) -> Option<RefreshAuthResponse> {
    serde_json::from_str(text).ok()
}

/// True if the URL targets Arete Cloud WebSockets (`*.stack.arete.run`), regardless of `hs_token`.
pub fn is_hosted_arete_cloud_url(url: &str) -> bool {
    let Ok(u) = Url::parse(url) else {
        return false;
    };
    let Some(host) = u.host_str() else {
        return false;
    };
    is_hosted_websocket_host(&host.to_ascii_lowercase())
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
    if !is_hosted_websocket_host(&host) {
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
///
/// Returns the URL to connect with and, when a token was minted, what the
/// stream needs to replace it before it expires.
pub fn ensure_hosted_ws_token(url: String) -> Result<(String, Option<SessionRefresh>)> {
    if !hosted_url_needs_token(&url) {
        return Ok((url, None));
    }

    let api_key = ApiClient::load_optional_api_key().context(
        "Failed to load stored API credentials; fix ~/.arete/credentials.toml or run `a4 auth login`",
    )?;

    let base = config::get_api_url(None);
    let endpoint = format!("{}/ws/sessions", base.trim_end_matches('/'));
    let minted = mint_session_token(&endpoint, api_key.as_deref(), &url)?;

    let mut u = Url::parse(&url).context("Invalid WebSocket URL")?;
    u.query_pairs_mut().append_pair("hs_token", &minted.token);
    let refresh = SessionRefresh {
        endpoint,
        api_key,
        websocket_url: url,
        expires_at: minted.expires_at,
    };
    Ok((u.to_string(), Some(refresh)))
}

fn mint_session_token(
    endpoint: &str,
    api_key: Option<&str>,
    websocket_url: &str,
) -> Result<MintResponse> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .context("Failed to build HTTP client for token mint")?;
    let mut request = client.post(endpoint).json(&MintBody { websocket_url });
    if let Some(key) = api_key {
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

    let mut mint: MintResponse = response
        .json()
        .context("Invalid JSON from /ws/sessions token endpoint")?;
    mint.token = mint.token.trim().to_string();
    if mint.token.is_empty() {
        bail!("Token endpoint returned an empty token");
    }
    Ok(mint)
}

#[cfg(test)]
impl SessionRefresh {
    pub(crate) fn for_test(endpoint: String, websocket_url: &str, expires_at: u64) -> Self {
        Self {
            endpoint,
            api_key: Some("a4_ak_test".to_string()),
            websocket_url: websocket_url.to_string(),
            expires_at: Some(expires_at),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api_client::test_support::MockServer;

    #[test]
    fn a_token_is_replaced_a_minute_before_it_expires() {
        assert_eq!(refresh_delay(1_300, 1_000), Duration::from_secs(240));
    }

    #[test]
    fn a_short_lived_token_is_replaced_halfway() {
        assert_eq!(refresh_delay(1_030, 1_000), Duration::from_secs(15));
    }

    #[test]
    fn an_expired_token_is_replaced_at_once() {
        assert_eq!(refresh_delay(900, 1_000), Duration::from_secs(1));
    }

    #[test]
    fn refresh_replies_are_told_apart_from_frames() {
        let accepted = parse_refresh_response(r#"{"success":true,"expiresAt":1790000000}"#)
            .expect("an accepted refresh");
        assert!(accepted.success);

        let refused = parse_refresh_response(r#"{"success":false,"error":"token-expired"}"#)
            .expect("a refused refresh");
        assert!(!refused.success);
        assert_eq!(refused.error.as_deref(), Some("token-expired"));

        assert!(parse_refresh_response(
            r#"{"type":"error","code":"token-expired","message":"expired","fatal":false}"#
        )
        .is_none());
        assert!(parse_refresh_response(r#"{"mode":"list","op":"upsert","key":"a"}"#).is_none());
    }

    #[tokio::test]
    async fn the_refresher_mints_a_replacement_before_expiry() {
        let expires_at = unix_now() + 3_600;
        let mint = MockServer::json(
            200,
            &serde_json::json!({"token": " second ", "expires_at": expires_at}).to_string(),
        );
        let endpoint = format!("{}/ws/sessions", mint.base_url());
        let mut refresher = SessionRefresher::start(Some(SessionRefresh::for_test(
            endpoint,
            "wss://ore.stack.arete.run",
            unix_now() + 2,
        )));

        let event = tokio::time::timeout(Duration::from_secs(10), refresher.next())
            .await
            .expect("a refresh within the timeout");
        let RefreshEvent::Token(token) = event else {
            panic!("the mint should succeed");
        };
        assert_eq!(token, "second");

        let request = mint.request();
        assert_eq!(request.request_line, "POST /ws/sessions HTTP/1.1");
        assert_eq!(request.header("authorization"), Some("Bearer a4_ak_test"));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&request.body).unwrap(),
            serde_json::json!({"websocket_url": "wss://ore.stack.arete.run"})
        );
    }

    #[tokio::test]
    async fn a_failed_mint_is_reported_and_retried() {
        let mint = MockServer::json(500, r#"{"error":"unavailable"}"#);
        let endpoint = format!("{}/ws/sessions", mint.base_url());
        let mut refresher = SessionRefresher::start(Some(SessionRefresh::for_test(
            endpoint,
            "wss://ore.stack.arete.run",
            unix_now() + 2,
        )));

        let event = tokio::time::timeout(Duration::from_secs(10), refresher.next())
            .await
            .expect("a refresh attempt within the timeout");
        let RefreshEvent::Failed(error) = event else {
            panic!("the mint should fail");
        };
        assert!(format!("{error:#}").contains("500"), "{error:#}");
    }

    #[tokio::test]
    async fn without_a_minted_token_nothing_is_refreshed() {
        let mut refresher = SessionRefresher::start(None);
        assert!(
            tokio::time::timeout(Duration::from_millis(100), refresher.next())
                .await
                .is_err()
        );
    }
}
