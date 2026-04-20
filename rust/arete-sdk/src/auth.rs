use crate::error::{AuthErrorCode, AreteError};
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use url::Url;

pub const TOKEN_REFRESH_BUFFER_SECONDS: u64 = 60;
pub const MIN_REFRESH_DELAY_SECONDS: u64 = 1;
pub const DEFAULT_QUERY_PARAMETER: &str = "hs_token";
pub const DEFAULT_HOSTED_TOKEN_ENDPOINT: &str = "https://api.arete.run/ws/sessions";
pub const HOSTED_WEBSOCKET_SUFFIX: &str = ".stack.arete.run";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthToken {
    pub token: String,
    pub expires_at: Option<u64>,
}

impl AuthToken {
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
            expires_at: None,
        }
    }

    pub fn with_expiry(mut self, expires_at: u64) -> Self {
        self.expires_at = Some(expires_at);
        self
    }
}

impl From<String> for AuthToken {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl From<&str> for AuthToken {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

pub type TokenProviderFuture =
    Pin<Box<dyn Future<Output = Result<AuthToken, AreteError>> + Send>>;
pub type TokenProvider = dyn Fn() -> TokenProviderFuture + Send + Sync;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TokenTransport {
    #[default]
    QueryParameter,
    Bearer,
}

#[derive(Clone, Default)]
pub struct AuthConfig {
    pub(crate) token: Option<String>,
    pub(crate) get_token: Option<Arc<TokenProvider>>,
    pub(crate) token_endpoint: Option<String>,
    pub(crate) publishable_key: Option<String>,
    pub(crate) token_endpoint_headers: HashMap<String, String>,
    pub(crate) token_transport: TokenTransport,
}

impl fmt::Debug for AuthConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuthConfig")
            .field("has_token", &self.token.is_some())
            .field("has_get_token", &self.get_token.is_some())
            .field("token_endpoint", &self.token_endpoint)
            .field(
                "publishable_key",
                &self.publishable_key.as_ref().map(|_| "***"),
            )
            .field(
                "token_endpoint_headers",
                &self.token_endpoint_headers.keys().collect::<Vec<_>>(),
            )
            .field("token_transport", &self.token_transport)
            .finish()
    }
}

impl AuthConfig {
    pub fn with_token(mut self, token: impl Into<String>) -> Self {
        self.token = Some(token.into());
        self
    }

    pub fn with_publishable_key(mut self, publishable_key: impl Into<String>) -> Self {
        self.publishable_key = Some(publishable_key.into());
        self
    }

    pub fn with_token_endpoint(mut self, token_endpoint: impl Into<String>) -> Self {
        self.token_endpoint = Some(token_endpoint.into());
        self
    }

    pub fn with_token_endpoint_header(
        mut self,
        key: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        self.token_endpoint_headers.insert(key.into(), value.into());
        self
    }

    pub fn with_token_transport(mut self, transport: TokenTransport) -> Self {
        self.token_transport = transport;
        self
    }

    pub fn with_token_provider<F, Fut>(mut self, provider: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<AuthToken, AreteError>> + Send + 'static,
    {
        self.get_token = Some(Arc::new(move || Box::pin(provider())));
        self
    }

    pub(crate) fn resolve_strategy(&self, websocket_url: &str) -> ResolvedAuthStrategy {
        if let Some(token) = self.token.clone() {
            return ResolvedAuthStrategy::StaticToken(token);
        }

        if let Some(get_token) = self.get_token.clone() {
            return ResolvedAuthStrategy::TokenProvider(get_token);
        }

        if let Some(token_endpoint) = self.token_endpoint.clone() {
            return ResolvedAuthStrategy::TokenEndpoint(token_endpoint);
        }

        if self.publishable_key.is_some() && is_hosted_arete_websocket_url(websocket_url) {
            return ResolvedAuthStrategy::TokenEndpoint(DEFAULT_HOSTED_TOKEN_ENDPOINT.to_string());
        }

        ResolvedAuthStrategy::None
    }

    pub(crate) fn has_refreshable_auth(&self, websocket_url: &str) -> bool {
        matches!(
            self.resolve_strategy(websocket_url),
            ResolvedAuthStrategy::TokenProvider(_) | ResolvedAuthStrategy::TokenEndpoint(_)
        )
    }
}

#[derive(Clone)]
pub(crate) enum ResolvedAuthStrategy {
    None,
    StaticToken(String),
    TokenProvider(Arc<TokenProvider>),
    TokenEndpoint(String),
}

#[derive(Debug, Deserialize)]
pub(crate) struct TokenEndpointResponse {
    pub token: String,
    #[serde(default)]
    pub expires_at: Option<u64>,
    #[serde(default, rename = "expiresAt")]
    pub expires_at_camel: Option<u64>,
}

impl TokenEndpointResponse {
    pub fn into_auth_token(self) -> AuthToken {
        AuthToken {
            token: self.token,
            expires_at: self.expires_at.or(self.expires_at_camel),
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct TokenEndpointRequest<'a> {
    pub websocket_url: &'a str,
}

pub(crate) fn parse_jwt_expiry(token: &str) -> Option<u64> {
    let mut parts = token.split('.');
    let _header = parts.next()?;
    let payload = parts.next()?;
    let _signature = parts.next()?;

    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.as_bytes())
        .ok()?;
    let payload: JwtPayload = serde_json::from_slice(&decoded).ok()?;
    payload.exp
}

pub(crate) fn token_is_expiring(expires_at: Option<u64>, now_epoch_seconds: u64) -> bool {
    match expires_at {
        Some(exp) => now_epoch_seconds >= exp.saturating_sub(TOKEN_REFRESH_BUFFER_SECONDS),
        None => false,
    }
}

pub(crate) fn token_refresh_delay(expires_at: Option<u64>, now_epoch_seconds: u64) -> Option<u64> {
    let expires_at = expires_at?;
    let refresh_at = expires_at.saturating_sub(TOKEN_REFRESH_BUFFER_SECONDS);
    Some(
        refresh_at
            .saturating_sub(now_epoch_seconds)
            .max(MIN_REFRESH_DELAY_SECONDS),
    )
}

pub(crate) fn is_hosted_arete_websocket_url(websocket_url: &str) -> bool {
    Url::parse(websocket_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
        .is_some_and(|host| host.ends_with(HOSTED_WEBSOCKET_SUFFIX))
}

pub(crate) fn build_websocket_url(
    websocket_url: &str,
    token: Option<&str>,
    transport: TokenTransport,
) -> Result<String, AreteError> {
    if transport == TokenTransport::Bearer || token.is_none() {
        return Ok(websocket_url.to_string());
    }

    let mut url = Url::parse(websocket_url)
        .map_err(|error| AreteError::ConnectionFailed(error.to_string()))?;
    url.query_pairs_mut()
        .append_pair(DEFAULT_QUERY_PARAMETER, token.expect("checked is_some"));
    Ok(url.to_string())
}

pub(crate) fn hosted_auth_required_error() -> AreteError {
    AreteError::WebSocket {
        message: "Hosted Arete websocket connections require auth.publishable_key, auth.get_token, auth.token_endpoint, or auth.token".to_string(),
        code: Some(AuthErrorCode::AuthRequired),
    }
}

#[derive(Debug, Deserialize)]
struct JwtPayload {
    exp: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode_base64url(input: &str) -> String {
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(input.as_bytes())
    }

    #[test]
    fn publishable_key_on_hosted_url_uses_default_token_endpoint() {
        let auth = AuthConfig::default().with_publishable_key("hspk_test");
        let strategy = auth.resolve_strategy("wss://demo.stack.arete.run");

        assert!(matches!(
            strategy,
            ResolvedAuthStrategy::TokenEndpoint(ref endpoint)
                if endpoint == DEFAULT_HOSTED_TOKEN_ENDPOINT
        ));
    }

    #[test]
    fn static_token_takes_precedence_over_endpoint_flow() {
        let auth = AuthConfig::default()
            .with_publishable_key("hspk_test")
            .with_token_endpoint("https://custom.example/ws/sessions")
            .with_token("static-token");

        assert!(matches!(
            auth.resolve_strategy("wss://demo.stack.arete.run"),
            ResolvedAuthStrategy::StaticToken(ref token) if token == "static-token"
        ));
    }

    #[test]
    fn build_websocket_url_adds_query_token_for_query_transport() {
        let url = build_websocket_url(
            "wss://demo.stack.arete.run/socket",
            Some("abc123"),
            TokenTransport::QueryParameter,
        )
        .expect("query auth url should build");

        assert!(url.contains("hs_token=abc123"));
    }

    #[test]
    fn parse_jwt_expiry_reads_exp_claim() {
        let header = encode_base64url(r#"{"alg":"none","typ":"JWT"}"#);
        let payload = encode_base64url(r#"{"exp":12345}"#);
        let token = format!("{}.{}.sig", header, payload);

        assert_eq!(parse_jwt_expiry(&token), Some(12345));
    }

    #[test]
    fn token_refresh_delay_respects_refresh_buffer() {
        let now = 1_000;
        let expires_at = Some(now + TOKEN_REFRESH_BUFFER_SECONDS + 15);

        assert_eq!(token_refresh_delay(expires_at, now), Some(15));
        assert_eq!(
            token_refresh_delay(Some(now + 10), now),
            Some(MIN_REFRESH_DELAY_SECONDS)
        );
    }
}
