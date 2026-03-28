use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;

use async_trait::async_trait;
use tokio_tungstenite::tungstenite::http::Request;

// Re-export AuthContext from hyperstack-auth for convenience
pub use hyperstack_auth::AuthContext;

#[derive(Debug, Clone)]
pub struct ConnectionAuthRequest {
    pub remote_addr: SocketAddr,
    pub path: String,
    pub query: Option<String>,
    pub headers: HashMap<String, String>,
    /// Origin header from the request (for browser origin validation)
    pub origin: Option<String>,
}

impl ConnectionAuthRequest {
    pub fn from_http_request<B>(remote_addr: SocketAddr, request: &Request<B>) -> Self {
        let mut headers = HashMap::new();
        for (name, value) in request.headers() {
            if let Ok(value_str) = value.to_str() {
                headers.insert(name.as_str().to_ascii_lowercase(), value_str.to_string());
            }
        }

        let origin = headers.get("origin").cloned();

        Self {
            remote_addr,
            path: request.uri().path().to_string(),
            query: request.uri().query().map(|q| q.to_string()),
            headers,
            origin,
        }
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }

    pub fn bearer_token(&self) -> Option<&str> {
        let value = self.header("authorization")?;
        let (scheme, token) = value.split_once(' ')?;
        if scheme.eq_ignore_ascii_case("bearer") {
            Some(token)
        } else {
            None
        }
    }

    pub fn query_param(&self, key: &str) -> Option<&str> {
        let query = self.query.as_deref()?;
        query
            .split('&')
            .filter_map(|pair| pair.split_once('='))
            .find_map(|(k, v)| if k == key { Some(v) } else { None })
    }
}

#[derive(Debug, Clone)]
pub struct AuthDeny {
    pub reason: String,
}

impl AuthDeny {
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

/// Authentication decision with optional auth context
#[derive(Debug, Clone)]
pub enum AuthDecision {
    /// Connection is authorized with the given context
    Allow(AuthContext),
    /// Connection is denied
    Deny(AuthDeny),
}

impl AuthDecision {
    /// Check if the decision is Allow
    pub fn is_allowed(&self) -> bool {
        matches!(self, AuthDecision::Allow(_))
    }

    /// Get the auth context if allowed
    pub fn auth_context(&self) -> Option<&AuthContext> {
        match self {
            AuthDecision::Allow(ctx) => Some(ctx),
            AuthDecision::Deny(_) => None,
        }
    }
}

#[async_trait]
pub trait WebSocketAuthPlugin: Send + Sync {
    async fn authorize(&self, request: &ConnectionAuthRequest) -> AuthDecision;
}

/// Development-only plugin that allows all connections
/// 
/// # Warning
/// This should only be used for local development. Never use in production.
pub struct AllowAllAuthPlugin;

#[async_trait]
impl WebSocketAuthPlugin for AllowAllAuthPlugin {
    async fn authorize(&self, _request: &ConnectionAuthRequest) -> AuthDecision {
        // Create a default auth context for development
        let context = AuthContext {
            subject: "anonymous".to_string(),
            issuer: "allow-all".to_string(),
            key_class: hyperstack_auth::KeyClass::Secret,
            metering_key: "dev".to_string(),
            deployment_id: None,
            expires_at: u64::MAX, // Never expires
            scope: "read write".to_string(),
            limits: Default::default(),
            origin: None,
            jti: uuid::Uuid::new_v4().to_string(),
        };
        AuthDecision::Allow(context)
    }
}

#[derive(Debug, Clone)]
pub struct StaticTokenAuthPlugin {
    tokens: HashSet<String>,
    query_param_name: String,
}

impl StaticTokenAuthPlugin {
    pub fn new(tokens: impl IntoIterator<Item = String>) -> Self {
        Self {
            tokens: tokens.into_iter().collect(),
            query_param_name: "token".to_string(),
        }
    }

    pub fn with_query_param_name(mut self, query_param_name: impl Into<String>) -> Self {
        self.query_param_name = query_param_name.into();
        self
    }

    fn extract_token<'a>(&self, request: &'a ConnectionAuthRequest) -> Option<&'a str> {
        request
            .bearer_token()
            .or_else(|| request.query_param(&self.query_param_name))
    }
}

#[async_trait]
impl WebSocketAuthPlugin for StaticTokenAuthPlugin {
    async fn authorize(&self, request: &ConnectionAuthRequest) -> AuthDecision {
        let token = match self.extract_token(request) {
            Some(token) => token,
            None => {
                return AuthDecision::Deny(AuthDeny::new(
                    "Missing auth token (expected Authorization: Bearer <token> or query token)",
                ));
            }
        };

        if self.tokens.contains(token) {
            // Create auth context for static token
            let context = AuthContext {
                subject: format!("static:{}", &token[..token.len().min(8)]),
                issuer: "static-token".to_string(),
                key_class: hyperstack_auth::KeyClass::Secret,
                metering_key: token.to_string(),
                deployment_id: None,
                expires_at: u64::MAX, // Static tokens don't expire
                scope: "read".to_string(),
                limits: Default::default(),
                origin: request.origin.clone(),
                jti: uuid::Uuid::new_v4().to_string(),
            };
            AuthDecision::Allow(context)
        } else {
            AuthDecision::Deny(AuthDeny::new("Invalid auth token"))
        }
    }
}

/// Signed session token authentication plugin
/// 
/// This plugin verifies JWT session tokens using Ed25519 signatures.
/// Tokens are expected to be passed either:
/// - In the Authorization header: `Authorization: Bearer <token>`
/// - As a query parameter: `?hs_token=<token>`
pub struct SignedSessionAuthPlugin {
    verifier: hyperstack_auth::TokenVerifier,
    query_param_name: String,
    require_origin: bool,
}

impl SignedSessionAuthPlugin {
    /// Create a new signed session auth plugin
    pub fn new(verifier: hyperstack_auth::TokenVerifier) -> Self {
        Self {
            verifier,
            query_param_name: "hs_token".to_string(),
            require_origin: false,
        }
    }

    /// Set a custom query parameter name for the token
    pub fn with_query_param_name(mut self, name: impl Into<String>) -> Self {
        self.query_param_name = name.into();
        self
    }

    /// Require origin validation (defense-in-depth for browser clients)
    pub fn with_origin_validation(mut self) -> Self {
        self.require_origin = true;
        self
    }

    fn extract_token<'a>(&self, request: &'a ConnectionAuthRequest) -> Option<&'a str> {
        request
            .bearer_token()
            .or_else(|| request.query_param(&self.query_param_name))
    }
}

#[async_trait]
impl WebSocketAuthPlugin for SignedSessionAuthPlugin {
    async fn authorize(&self, request: &ConnectionAuthRequest) -> AuthDecision {
        let token = match self.extract_token(request) {
            Some(token) => token,
            None => {
                return AuthDecision::Deny(AuthDeny::new(
                    "Missing session token (expected Authorization: Bearer <token> or ?hs_token=<token>)",
                ));
            }
        };

        let expected_origin = if self.require_origin {
            request.origin.as_deref()
        } else {
            None
        };

        match self.verifier.verify(token, expected_origin) {
            Ok(context) => AuthDecision::Allow(context),
            Err(e) => AuthDecision::Deny(AuthDeny::new(format!("Token verification failed: {}", e))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_bearer_and_query_tokens() {
        let request = Request::builder()
            .uri("/ws?token=query-token")
            .header("Authorization", "Bearer header-token")
            .body(())
            .expect("request should build");

        let auth_request = ConnectionAuthRequest::from_http_request(
            "127.0.0.1:8877".parse().expect("socket addr should parse"),
            &request,
        );

        assert_eq!(auth_request.bearer_token(), Some("header-token"));
        assert_eq!(auth_request.query_param("token"), Some("query-token"));
    }

    #[tokio::test]
    async fn static_token_plugin_allows_matching_token() {
        let plugin = StaticTokenAuthPlugin::new(["secret".to_string()]);
        let request = Request::builder()
            .uri("/ws?token=secret")
            .body(())
            .expect("request should build");
        let auth_request = ConnectionAuthRequest::from_http_request(
            "127.0.0.1:8877".parse().expect("socket addr should parse"),
            &request,
        );

        let decision = plugin.authorize(&auth_request).await;
        assert!(decision.is_allowed());
        assert!(decision.auth_context().is_some());
    }

    #[tokio::test]
    async fn static_token_plugin_denies_missing_token() {
        let plugin = StaticTokenAuthPlugin::new(["secret".to_string()]);
        let request = Request::builder()
            .uri("/ws")
            .body(())
            .expect("request should build");
        let auth_request = ConnectionAuthRequest::from_http_request(
            "127.0.0.1:8877".parse().expect("socket addr should parse"),
            &request,
        );

        let decision = plugin.authorize(&auth_request).await;
        assert!(!decision.is_allowed());
    }

    #[tokio::test]
    async fn allow_all_plugin_allows_with_context() {
        let plugin = AllowAllAuthPlugin;
        let request = Request::builder()
            .uri("/ws")
            .body(())
            .expect("request should build");
        let auth_request = ConnectionAuthRequest::from_http_request(
            "127.0.0.1:8877".parse().expect("socket addr should parse"),
            &request,
        );

        let decision = plugin.authorize(&auth_request).await;
        assert!(decision.is_allowed());
        let ctx = decision.auth_context().unwrap();
        assert_eq!(ctx.subject, "anonymous");
    }
}
