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
use arete_sdk::ApiProblemV1;

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
    /// Local stacks only: the protocol deployment whose row scopes the
    /// session carries.
    #[serde(
        rename = "protocolDeploymentId",
        skip_serializing_if = "Option::is_none"
    )]
    protocol_deployment_id: Option<&'a str>,
}

#[derive(Deserialize)]
struct MintResponse {
    token: String,
    /// Seconds since the epoch.
    #[serde(default)]
    #[serde(alias = "expiresAt")]
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
    /// Sent with each mint for a local stack (`a4 stream --dev`).
    protocol_deployment_id: Option<String>,
    expires_at: Option<u64>,
    /// Mint through this sandbox (`POST /api/sandbox/v1/sessions`) instead
    /// of with the API key: only stacks whose programs are all in its rules
    /// open, and each refresh re-checks them.
    sandbox: Option<arete_mcp::sandbox::SandboxClient>,
}

/// What the refresher hands the socket loop.
pub enum RefreshEvent {
    /// A new token to send as `refresh_auth` on the open socket.
    Token(String),
    /// Minting failed; it is tried again shortly.
    Failed(anyhow::Error),
    /// The sandbox refused the next token for good (the stack's programs
    /// left its rules, or it was paused or revoked): the stream ends.
    Refused(arete_mcp::sandbox::SandboxRefusal),
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
        let mut retry_delay: Option<Duration> = None;
        loop {
            let delay = match (retry, refresh.expires_at) {
                (true, _) => retry_delay.take().unwrap_or(REFRESH_RETRY),
                (false, Some(expires_at)) => refresh_delay(expires_at, unix_now()),
                // The endpoint gave no expiry, so there is nothing to schedule.
                (false, None) => return,
            };
            tokio::select! {
                _ = tokio::time::sleep(delay) => {}
                _ = events.closed() => return,
            }
            let event = match refresh.mint_async().await {
                Ok(minted) => {
                    retry = false;
                    refresh.expires_at = minted.expires_at;
                    RefreshEvent::Token(minted.token)
                }
                Err(error) => match fatal_refusal(&error) {
                    Some(refusal) => {
                        let _ = events.send(RefreshEvent::Refused(refusal)).await;
                        return;
                    }
                    None => {
                        retry = true;
                        retry_delay = retry_after(&error);
                        RefreshEvent::Failed(error)
                    }
                },
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

    // Credentials-file errors already name the file and the fix.
    let api_key = ApiClient::load_optional_api_key().map_err(|error| {
        if error
            .downcast_ref::<arete_mcp::credentials::CredentialsFileError>()
            .is_some()
        {
            error
        } else {
            error.context("Failed to load stored API credentials; run `a4 auth login`")
        }
    })?;

    let base = config::get_api_url(None);
    let endpoint = format!("{}/ws/sessions", base.trim_end_matches('/'));
    let minted = mint_session_token(&endpoint, api_key.as_deref(), &url, None)?;

    let connect_url = url_with_token(&url, &minted.token)?;
    let refresh = SessionRefresh {
        endpoint,
        api_key,
        websocket_url: url,
        protocol_deployment_id: None,
        expires_at: minted.expires_at,
        sandbox: None,
    };
    Ok((connect_url, Some(refresh)))
}

/// Mint the stream's session through `sandbox`: the URL to connect with and
/// what the stream needs to re-mint through the sandbox before each token
/// expires (about a minute). The sandbox is resolved once, so every re-mint
/// names the same one. A refusal is final: the stream does not start.
pub async fn sandbox_ws_token(
    url: String,
    sandbox: arete_mcp::sandbox::SandboxClient,
) -> Result<(String, Option<SessionRefresh>)> {
    if Url::parse(&url).is_ok_and(|u| u.query_pairs().any(|(k, _)| k == "hs_token")) {
        bail!("--sandbox mints the stream's session; leave hs_token out of --url");
    }
    let (sandbox, resolved) = sandbox.pin().await.map_err(anyhow::Error::new)?;
    eprintln!(
        "Reading through sandbox {} ({})",
        resolved.sandbox_id,
        resolved.source.as_str()
    );
    let base = config::get_api_url(None);
    let mut refresh = SessionRefresh {
        endpoint: format!("{}/api/sandbox/v1/sessions", base.trim_end_matches('/')),
        api_key: None,
        websocket_url: url,
        protocol_deployment_id: None,
        expires_at: None,
        sandbox: Some(sandbox),
    };
    let connect_url = refresh.renew().await?;
    Ok((connect_url, Some(refresh)))
}

/// How long a rate-limited sandbox asked to wait (`retryAfterSeconds`). A
/// wait past the token's expiry lets the host close the stream, which then
/// reconnects with a fresh token.
fn retry_after(error: &anyhow::Error) -> Option<Duration> {
    let refusal = error.downcast_ref::<arete_mcp::sandbox::SandboxRefusal>()?;
    let seconds = refusal.to_json()["retryAfterSeconds"].as_u64()?;
    Some(Duration::from_secs(seconds.clamp(1, 60)))
}

/// The refusal to end a stream with for `error`: a sandbox refusal that
/// will not pass on a retry. `None` for anything worth retrying.
pub fn fatal_refusal(error: &anyhow::Error) -> Option<arete_mcp::sandbox::SandboxRefusal> {
    let refusal = error.downcast_ref::<arete_mcp::sandbox::SandboxRefusal>()?;
    (!refusal.retryable()).then(|| refusal.clone())
}

impl SessionRefresh {
    /// Sessions for an Arete cloud dev environment stack: minted at the API
    /// `endpoint` with the account key, carrying `protocol_deployment_id`'s
    /// row scopes when given. Nothing is minted until [`renew`](Self::renew).
    pub fn local(
        endpoint: String,
        secret_key: String,
        websocket_url: String,
        protocol_deployment_id: Option<String>,
    ) -> Self {
        Self {
            endpoint,
            api_key: Some(secret_key),
            websocket_url,
            protocol_deployment_id,
            expires_at: None,
            sandbox: None,
        }
    }

    /// Mint the next token: through the sandbox, or with the key.
    async fn mint_async(&self) -> Result<MintResponse> {
        if let Some(sandbox) = &self.sandbox {
            let target = arete_mcp::sandbox::ReadTarget::Stack {
                websocket_url: self.websocket_url.clone(),
            };
            let session = sandbox
                .read_session(&target)
                .await
                .map_err(anyhow::Error::new)?;
            return Ok(MintResponse {
                token: session.token,
                expires_at: session.expires_at,
            });
        }
        let this = self.clone();
        tokio::task::spawn_blocking(move || this.mint())
            .await
            .context("token mint task failed")?
    }

    fn mint(&self) -> Result<MintResponse> {
        mint_session_token(
            &self.endpoint,
            self.api_key.as_deref(),
            &self.websocket_url,
            self.protocol_deployment_id.as_deref(),
        )
    }

    /// Mint a token for a new connection, the way the first one was minted,
    /// and return the URL to connect with.
    pub async fn renew(&mut self) -> Result<String> {
        let minted = self.mint_async().await?;
        self.expires_at = minted.expires_at;
        url_with_token(&self.websocket_url, &minted.token)
    }
}

fn url_with_token(url: &str, token: &str) -> Result<String> {
    let mut u = Url::parse(url).context("Invalid WebSocket URL")?;
    u.query_pairs_mut().append_pair("hs_token", token);
    Ok(u.to_string())
}

fn mint_session_token(
    endpoint: &str,
    api_key: Option<&str>,
    websocket_url: &str,
    protocol_deployment_id: Option<&str>,
) -> Result<MintResponse> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .context("Failed to build HTTP client for token mint")?;
    let mut request = client.post(endpoint).json(&MintBody {
        websocket_url,
        protocol_deployment_id,
    });
    if let Some(key) = api_key {
        request = request.header("Authorization", format!("Bearer {}", key.trim()));
    }
    let response = request
        .send()
        .with_context(|| format!("Failed to reach token endpoint {}", endpoint))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().unwrap_or_default();
        bail!(mint_refusal(status, &body, api_key.is_some(), endpoint));
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

/// What `a4` prints when the API refuses a session token. The server's own
/// message leads, never its raw JSON body, followed by the step that lifts
/// the refusal: sign in when no key was sent, claim the agent when the body
/// carries the claim action, list the starter stacks for a service-class
/// refusal, or fix the key when it was rejected. Steps the server message
/// already names are not repeated.
fn mint_refusal(status: reqwest::StatusCode, body: &str, keyed: bool, endpoint: &str) -> String {
    let problem = serde_json::from_str::<ApiProblemV1>(body).ok();
    let message = problem
        .as_ref()
        .map(|problem| problem.error.trim().to_string())
        .unwrap_or_else(|| body.trim().to_string());
    let code = problem.as_ref().and_then(|problem| problem.code.as_deref());
    let claimable = problem
        .as_ref()
        .and_then(ApiProblemV1::recovery_action)
        .is_some_and(|action| action.is_claim_agent_materializer());
    let refused = match code {
        Some(code) => format!("({status}, {code})"),
        None => format!("({status})"),
    };
    let denied = matches!(
        status,
        reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN
    );

    if !keyed && denied {
        return format!(
            "This hosted stream requires an API key {refused}: {message}\n\
             • Keys are free: `a4 auth signup` creates an agent key, or create one at https://arete.run/keys and save it with `a4 auth login`. Then retry; the CLI mints a token automatically.\n\
             • Without a key, only public stacks can be read.\n\
             • Or pass `--url` with `?hs_token=...` from POST `{endpoint}`."
        );
    }

    let mut text = format!("This hosted stream refused the session {refused}: {message}");
    let mut step = |line: &str, named_by: &str| {
        if !message.contains(named_by) {
            text.push_str("\n• ");
            text.push_str(line);
        }
    };
    if claimable || code == Some("agent-claim-required") {
        step(
            "Claiming the agent is free: run `a4 auth claim-link` and have a human open the link, then retry.",
            "a4 auth claim-link",
        );
    }
    match code {
        Some("service-class-required") => step(
            "`a4 explore stacks --service-class starter` lists the stacks this account can read now.",
            "a4 explore stacks",
        ),
        _ if status == reqwest::StatusCode::UNAUTHORIZED => step(
            "Check the API key: `a4 auth login` stores a new one.",
            "a4 auth login",
        ),
        _ if status == reqwest::StatusCode::FORBIDDEN && !claimable => step(
            "This account cannot read this stack; `a4 explore stack <ref>` shows what it requires.",
            "a4 explore",
        ),
        _ => {}
    }
    text
}

#[cfg(test)]
impl SessionRefresh {
    /// The stack URL tokens are minted for, without a token.
    pub(crate) fn websocket_url(&self) -> &str {
        &self.websocket_url
    }

    pub(crate) fn for_test(endpoint: String, websocket_url: &str, expires_at: u64) -> Self {
        Self {
            endpoint,
            api_key: Some("a4_ak_test".to_string()),
            websocket_url: websocket_url.to_string(),
            protocol_deployment_id: None,
            expires_at: Some(expires_at),
            sandbox: None,
        }
    }

    /// Tokens minted through `sandbox` for `websocket_url`.
    pub(crate) fn sandbox_for_test(
        sandbox: arete_mcp::sandbox::SandboxClient,
        websocket_url: &str,
        expires_at: u64,
    ) -> Self {
        Self {
            endpoint: String::new(),
            api_key: None,
            websocket_url: websocket_url.to_string(),
            protocol_deployment_id: None,
            expires_at: Some(expires_at),
            sandbox: Some(sandbox),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api_client::test_support::MockServer;

    #[test]
    fn mint_responses_decode_either_spelling_of_the_expiry() {
        for body in [
            r#"{"token":"t","expires_at":1000}"#,
            r#"{"token":"t","expiresAt":1000}"#,
        ] {
            let mint: MintResponse = serde_json::from_str(body).unwrap();
            assert_eq!(mint.token, "t");
            assert_eq!(mint.expires_at, Some(1000));
        }
        let mint: MintResponse = serde_json::from_str(r#"{"token":"t"}"#).unwrap();
        assert_eq!(mint.expires_at, None);
    }

    #[test]
    fn a_service_class_refusal_names_the_claim_and_the_starter_list() {
        // An older server: bare message, no action.
        let body = r#"{"schemaVersion":1,"error":"Your plan does not include this service class","code":"service-class-required","retryable":false}"#;
        let text = mint_refusal(
            reqwest::StatusCode::FORBIDDEN,
            body,
            true,
            "https://api/ws/sessions",
        );
        assert!(text.starts_with("This hosted stream refused the session (403 Forbidden, service-class-required): Your plan does not include this service class"), "{text}");
        assert!(
            text.contains("`a4 explore stacks --service-class starter`"),
            "{text}"
        );
        assert!(!text.contains("a4 auth claim-link"), "{text}");
        assert!(!text.contains("\"schemaVersion\""), "{text}");
        assert!(!text.contains("Fix your API key"), "{text}");

        // A claimable refusal adds the claim step.
        let body = r#"{"schemaVersion":1,"error":"Your plan does not include this service class","code":"service-class-required","retryable":false,"action":{"type":"claim_agent","label":"Claim this agent","method":"POST","path":"/api/agents/me/claim-links"}}"#;
        let text = mint_refusal(
            reqwest::StatusCode::FORBIDDEN,
            body,
            true,
            "https://api/ws/sessions",
        );
        assert!(
            text.contains("• Claiming the agent is free: run `a4 auth claim-link`"),
            "{text}"
        );
        assert!(text.contains("--service-class starter"), "{text}");
    }

    #[test]
    fn steps_the_server_already_names_are_not_repeated() {
        let body = r#"{"error":"This stack is in the standard service class, and an unclaimed agent account reads only starter targets. Claiming the agent is free and lifts this: run `a4 auth claim-link` and have a human open the link. `a4 explore stacks --service-class starter` lists the stacks this account can read now.","code":"service-class-required","action":{"type":"claim_agent","path":"/api/agents/me/claim-links"}}"#;
        let text = mint_refusal(
            reqwest::StatusCode::FORBIDDEN,
            body,
            true,
            "https://api/ws/sessions",
        );
        assert_eq!(text.matches("a4 auth claim-link").count(), 1, "{text}");
        assert_eq!(text.matches("a4 explore stacks").count(), 1, "{text}");
        assert!(!text.contains('•'), "{text}");
    }

    #[test]
    fn an_anonymous_refusal_names_the_free_sign_in() {
        let body = r#"{"error":"Authentication required to mint websocket session tokens.","code":"auth-required"}"#;
        let text = mint_refusal(
            reqwest::StatusCode::UNAUTHORIZED,
            body,
            false,
            "https://api/ws/sessions",
        );
        assert!(
            text.contains("requires an API key (401 Unauthorized, auth-required)"),
            "{text}"
        );
        assert!(text.contains("Keys are free: `a4 auth signup`"), "{text}");
        assert!(text.contains("`a4 auth login`"), "{text}");
        assert!(text.contains("POST `https://api/ws/sessions`"), "{text}");
    }

    #[test]
    fn a_rejected_key_and_a_non_json_body_stay_readable() {
        let text = mint_refusal(
            reqwest::StatusCode::UNAUTHORIZED,
            r#"{"error":"Invalid API key","code":"invalid-api-key"}"#,
            true,
            "e",
        );
        assert!(text.contains("Invalid API key"), "{text}");
        assert!(text.contains("`a4 auth login` stores a new one"), "{text}");

        let text = mint_refusal(reqwest::StatusCode::BAD_GATEWAY, "upstream down", true, "e");
        assert_eq!(
            text,
            "This hosted stream refused the session (502 Bad Gateway): upstream down"
        );
    }

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

    const SANDBOX: &str = "6f1c2c43-6d3c-4a51-9b8e-0e3c1e2a7b10";

    /// A sandbox at `mint`, whose first answer is its sandbox token.
    fn sandbox_at(mint: &MockServer) -> arete_mcp::sandbox::SandboxClient {
        arete_mcp::sandbox::SandboxClient::from_values(
            Some(SANDBOX.into()),
            Some("a4_ak_test".into()),
            Some(mint.base_url().to_string()),
        )
    }

    fn sandbox_token() -> (u16, String) {
        let body = serde_json::json!({"token": "sbx", "expiresAt": unix_now() + 600, "scopes": []});
        (200, body.to_string())
    }

    #[tokio::test]
    async fn a_sandbox_stream_re_mints_through_the_sandbox() {
        let mint = MockServer::json_sequence(vec![
            sandbox_token(),
            (
                200,
                serde_json::json!({"token": "read-2", "expiresAt": unix_now() + 60}).to_string(),
            ),
        ]);
        let mut refresher = SessionRefresher::start(Some(SessionRefresh::sandbox_for_test(
            sandbox_at(&mint),
            "wss://ore.stack.arete.run",
            unix_now() + 2,
        )));
        let event = tokio::time::timeout(Duration::from_secs(10), refresher.next())
            .await
            .expect("a refresh within the timeout");
        let RefreshEvent::Token(token) = event else {
            panic!("the sandbox mint should succeed");
        };
        assert_eq!(token, "read-2");
        let sandbox_mint = mint.request();
        assert_eq!(sandbox_mint.request_line, "POST /ws/sessions HTTP/1.1");
        let read_mint = mint.request();
        assert_eq!(
            read_mint.request_line,
            "POST /api/sandbox/v1/sessions HTTP/1.1"
        );
        assert_eq!(read_mint.header("authorization"), Some("Bearer sbx"));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&read_mint.body).unwrap(),
            serde_json::json!({"websocketUrl": "wss://ore.stack.arete.run"})
        );
    }

    #[tokio::test]
    async fn a_sandbox_refusal_ends_the_stream_instead_of_retrying() {
        let refusal = serde_json::json!({
            "code": "sandbox-program-not-allowlisted",
            "error": "The stack decodes a program the sandbox's rules leave out",
            "retryable": false,
            "programs": ["KLend"],
        });
        let mint = MockServer::json_sequence(vec![sandbox_token(), (403, refusal.to_string())]);
        let mut refresher = SessionRefresher::start(Some(SessionRefresh::sandbox_for_test(
            sandbox_at(&mint),
            "wss://klend.stack.arete.run",
            unix_now() + 2,
        )));
        let event = tokio::time::timeout(Duration::from_secs(10), refresher.next())
            .await
            .expect("a refresh attempt within the timeout");
        let RefreshEvent::Refused(refusal) = event else {
            panic!("a rules refusal is final");
        };
        let reason = refusal.message();
        assert!(
            reason.contains("sandbox-program-not-allowlisted"),
            "{reason}"
        );
        assert!(reason.contains("KLend"), "{reason}");
        // Nothing more comes: the refresher stopped.
        assert!(
            tokio::time::timeout(Duration::from_millis(200), refresher.next())
                .await
                .is_err()
        );
    }

    #[test]
    fn a_rate_limited_sandbox_is_retried_when_it_asks() {
        let refusal = arete_mcp::sandbox::SandboxRefusal {
            status: 429,
            body: serde_json::json!({"code": "sandbox-rate-limited", "retryAfterSeconds": 7}),
            kind: arete_mcp::sandbox::CallKind::Read,
        };
        let error = anyhow::Error::new(refusal);
        assert_eq!(retry_after(&error), Some(Duration::from_secs(7)));
        assert!(fatal_refusal(&error).is_none(), "a rate limit is retried");
        assert_eq!(retry_after(&anyhow::anyhow!("down")), None);
    }

    #[tokio::test]
    async fn a_sandbox_outage_is_retried() {
        let mint = MockServer::json_sequence(vec![
            sandbox_token(),
            (
                503,
                r#"{"code":"sandbox-api-unavailable","error":"down"}"#.to_string(),
            ),
        ]);
        let mut refresher = SessionRefresher::start(Some(SessionRefresh::sandbox_for_test(
            sandbox_at(&mint),
            "wss://ore.stack.arete.run",
            unix_now() + 2,
        )));
        let event = tokio::time::timeout(Duration::from_secs(10), refresher.next())
            .await
            .expect("a refresh attempt within the timeout");
        assert!(matches!(event, RefreshEvent::Failed(_)));
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
    async fn renewing_mints_a_token_for_a_new_connection() {
        let expires_at = unix_now() + 3_600;
        let mint = MockServer::json(
            200,
            &serde_json::json!({"token": "fresh", "expires_at": expires_at}).to_string(),
        );
        let mut refresh = SessionRefresh::for_test(
            format!("{}/ws/sessions", mint.base_url()),
            "wss://ore.stack.arete.run/?region=eu",
            unix_now() + 10,
        );

        let url = refresh.renew().await.expect("the mint succeeds");

        assert_eq!(url, "wss://ore.stack.arete.run/?region=eu&hs_token=fresh");
        assert_eq!(refresh.expires_at, Some(expires_at));
        let request = mint.request();
        assert_eq!(request.header("authorization"), Some("Bearer a4_ak_test"));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&request.body).unwrap(),
            serde_json::json!({"websocket_url": "wss://ore.stack.arete.run/?region=eu"})
        );
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

    #[tokio::test]
    async fn local_sessions_are_minted_with_the_secret_key_and_deployment() {
        let mint = MockServer::json(
            200,
            &serde_json::json!({"token": "local", "expires_at": unix_now() + 3_600}).to_string(),
        );
        let mut refresh = SessionRefresh::local(
            format!("{}/ws/sessions", mint.base_url()),
            "a4_sk_dev".to_string(),
            "ws://127.0.0.1:49310".to_string(),
            Some("d-1".to_string()),
        );
        assert_eq!(refresh.expires_at, None);

        let url = refresh.renew().await.expect("the mint succeeds");

        assert_eq!(url, "ws://127.0.0.1:49310/?hs_token=local");
        assert!(refresh.expires_at.is_some());
        let request = mint.request();
        assert_eq!(request.header("authorization"), Some("Bearer a4_sk_dev"));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&request.body).unwrap(),
            serde_json::json!({
                "websocket_url": "ws://127.0.0.1:49310",
                "protocolDeploymentId": "d-1"
            })
        );
    }
}
