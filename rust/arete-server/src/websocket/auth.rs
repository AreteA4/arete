use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio_tungstenite::tungstenite::http::Request;

// Re-export AuthContext from arete-auth for convenience
pub use arete_auth::AuthContext;
// Re-export AuthErrorCode for convenience
pub use arete_auth::AuthErrorCode;
// Re-export RetryPolicy for convenience
pub use arete_auth::RetryPolicy;
// Re-export audit types
pub use arete_auth::{
    auth_failure_event, auth_success_event, rate_limit_event, AuditEvent, AuditSeverity,
    ChannelAuditLogger, NoOpAuditLogger, SecurityAuditEvent, SecurityAuditLogger,
};
// Re-export metrics types
pub use arete_auth::{AuthMetrics, AuthMetricsCollector, AuthMetricsSnapshot};
// Re-export multi-key verifier types
pub use arete_auth::{MultiKeyVerifier, MultiKeyVerifierBuilder, RotationKey};

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

/// Structured error details for machine-readable error handling
#[derive(Debug, Clone, Default)]
pub struct AuthErrorDetails {
    /// The specific field or parameter that caused the error (if applicable)
    pub field: Option<String>,
    /// Additional context about the error
    pub context: Option<String>,
    /// Suggested action for the client to resolve the error
    pub suggested_action: Option<String>,
    /// Related documentation URL
    pub docs_url: Option<String>,
}

/// Enhanced authentication denial with structured error information
#[derive(Debug, Clone)]
pub struct AuthDeny {
    pub reason: String,
    pub code: AuthErrorCode,
    /// Structured error details for machine processing
    pub details: AuthErrorDetails,
    /// Retry policy hint
    pub retry_policy: RetryPolicy,
    /// HTTP status code equivalent for the error
    pub http_status: u16,
    /// When the error condition will reset (if applicable)
    pub reset_at: Option<std::time::SystemTime>,
}

impl AuthDeny {
    /// Create a new AuthDeny with the specified error code and reason
    pub fn new(code: AuthErrorCode, reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
            code,
            details: AuthErrorDetails::default(),
            retry_policy: code.default_retry_policy(),
            http_status: code.http_status(),
            reset_at: None,
        }
    }

    /// Create an AuthDeny for missing token
    pub fn token_missing() -> Self {
        Self::new(
            AuthErrorCode::TokenMissing,
            "Missing session token (expected Authorization: Bearer <token> or query token)",
        )
        .with_suggested_action(
            "Provide a valid session token in the Authorization header or as a query parameter",
        )
    }

    /// Create an AuthDeny from a VerifyError
    pub fn from_verify_error(err: arete_auth::VerifyError) -> Self {
        let code = AuthErrorCode::from(&err);
        Self::new(code, format!("Token verification failed: {}", err))
    }

    /// Add structured error details
    pub fn with_details(mut self, details: AuthErrorDetails) -> Self {
        self.details = details;
        self
    }

    /// Add a specific field that caused the error
    pub fn with_field(mut self, field: impl Into<String>) -> Self {
        self.details.field = Some(field.into());
        self
    }

    /// Add context to the error
    pub fn with_context(mut self, context: impl Into<String>) -> Self {
        self.details.context = Some(context.into());
        self
    }

    /// Add a suggested action for the client
    pub fn with_suggested_action(mut self, action: impl Into<String>) -> Self {
        self.details.suggested_action = Some(action.into());
        self
    }

    /// Add documentation URL
    pub fn with_docs_url(mut self, url: impl Into<String>) -> Self {
        self.details.docs_url = Some(url.into());
        self
    }

    /// Set a custom retry policy
    pub fn with_retry_policy(mut self, policy: RetryPolicy) -> Self {
        self.retry_policy = policy;
        self
    }

    /// Set when the error condition will reset
    pub fn with_reset_at(mut self, reset_at: std::time::SystemTime) -> Self {
        self.reset_at = Some(reset_at);
        self
    }

    /// Create an AuthDeny for rate limiting with retry information
    pub fn rate_limited(retry_after: Duration, limit_type: &str) -> Self {
        let reset_at = std::time::SystemTime::now() + retry_after;
        Self::new(
            AuthErrorCode::RateLimitExceeded,
            format!(
                "Rate limit exceeded for {}. Please retry after {:?}.",
                limit_type, retry_after
            ),
        )
        .with_retry_policy(RetryPolicy::RetryAfter(retry_after))
        .with_reset_at(reset_at)
        .with_suggested_action(format!(
            "Wait {:?} before retrying the request",
            retry_after
        ))
    }

    /// Create an AuthDeny for connection limits
    pub fn connection_limit_exceeded(limit_type: &str, current: usize, max: usize) -> Self {
        Self::new(
            AuthErrorCode::ConnectionLimitExceeded,
            format!(
                "Connection limit exceeded: {} has {} of {} allowed connections",
                limit_type, current, max
            ),
        )
        .with_suggested_action(
            "Disconnect existing connections or wait for other connections to close",
        )
    }

    /// Convert to a JSON-serializable error response
    pub fn to_error_response(&self) -> ErrorResponse {
        ErrorResponse {
            error: self.code.as_str().to_string(),
            message: self.reason.clone(),
            code: self.code.to_string(),
            retryable: matches!(
                self.retry_policy,
                RetryPolicy::RetryImmediately
                    | RetryPolicy::RetryAfter(_)
                    | RetryPolicy::RetryWithBackoff { .. }
                    | RetryPolicy::RetryWithFreshToken
            ),
            retry_after: match self.retry_policy {
                RetryPolicy::RetryAfter(d) => Some(d.as_secs()),
                _ => None,
            },
            suggested_action: self.details.suggested_action.clone(),
            docs_url: self.details.docs_url.clone(),
        }
    }
}

/// JSON-serializable error response for clients
#[derive(Debug, Clone, serde::Serialize)]
pub struct ErrorResponse {
    pub error: String,
    pub message: String,
    pub code: String,
    pub retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggested_action: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docs_url: Option<String>,
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
pub trait WebSocketAuthPlugin: Send + Sync + Any {
    async fn authorize(&self, request: &ConnectionAuthRequest) -> AuthDecision;

    fn as_any(&self) -> &dyn Any;

    /// Get the audit logger if configured
    fn audit_logger(&self) -> Option<&dyn SecurityAuditLogger> {
        None
    }

    /// Log a security audit event if audit logging is enabled
    async fn log_audit(&self, event: SecurityAuditEvent) {
        if let Some(logger) = self.audit_logger() {
            logger.log(event).await;
        }
    }

    /// Get auth metrics if configured
    fn auth_metrics(&self) -> Option<&AuthMetrics> {
        None
    }
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
            key_class: arete_auth::KeyClass::Secret,
            metering_key: "dev".to_string(),
            deployment_id: None,
            expires_at: u64::MAX, // Never expires
            scope: "read write".to_string(),
            limits: Default::default(),
            plan: None,
            origin: None,
            client_ip: None,
            jti: uuid::Uuid::new_v4().to_string(),
        };
        AuthDecision::Allow(context)
    }

    fn as_any(&self) -> &dyn Any {
        self
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
                return AuthDecision::Deny(AuthDeny::token_missing());
            }
        };

        if self.tokens.contains(token) {
            // Create auth context for static token
            let context = AuthContext {
                subject: format!("static:{}", &token[..token.len().min(8)]),
                issuer: "static-token".to_string(),
                key_class: arete_auth::KeyClass::Secret,
                metering_key: token.to_string(),
                deployment_id: None,
                expires_at: u64::MAX, // Static tokens don't expire
                scope: "read".to_string(),
                limits: Default::default(),
                plan: None,
                origin: request.origin.clone(),
                client_ip: None,
                jti: uuid::Uuid::new_v4().to_string(),
            };
            AuthDecision::Allow(context)
        } else {
            AuthDecision::Deny(AuthDeny::new(
                AuthErrorCode::InvalidStaticToken,
                "Invalid auth token",
            ))
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Signed session token authentication plugin
///
/// This plugin verifies JWT session tokens using Ed25519 signatures.
/// Tokens are expected to be passed either:
/// - In the Authorization header: `Authorization: Bearer <token>`
/// - As a query parameter: `?hs_token=<token>`
enum SignedSessionVerifier {
    Static(arete_auth::TokenVerifier),
    CachedJwks(arete_auth::AsyncVerifier),
    MultiKey(arete_auth::MultiKeyVerifier),
}

pub struct SignedSessionAuthPlugin {
    verifier: SignedSessionVerifier,
    query_param_name: String,
    require_origin: bool,
    audit_logger: Option<Arc<dyn SecurityAuditLogger>>,
    metrics: Option<Arc<AuthMetrics>>,
}

impl SignedSessionAuthPlugin {
    /// Create a new signed session auth plugin
    pub fn new(verifier: arete_auth::TokenVerifier) -> Self {
        Self {
            verifier: SignedSessionVerifier::Static(verifier),
            query_param_name: "hs_token".to_string(),
            require_origin: false,
            audit_logger: None,
            metrics: None,
        }
    }

    /// Create a signed session auth plugin backed by an async verifier, such as JWKS.
    pub fn new_with_async_verifier(verifier: arete_auth::AsyncVerifier) -> Self {
        Self {
            verifier: SignedSessionVerifier::CachedJwks(verifier),
            query_param_name: "hs_token".to_string(),
            require_origin: false,
            audit_logger: None,
            metrics: None,
        }
    }

    /// Create a signed session auth plugin backed by a multi-key verifier for key rotation.
    pub fn new_with_multi_key_verifier(verifier: arete_auth::MultiKeyVerifier) -> Self {
        Self {
            verifier: SignedSessionVerifier::MultiKey(verifier),
            query_param_name: "hs_token".to_string(),
            require_origin: false,
            audit_logger: None,
            metrics: None,
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

    /// Set an audit logger for security events
    pub fn with_audit_logger(mut self, logger: Arc<dyn SecurityAuditLogger>) -> Self {
        self.audit_logger = Some(logger);
        self
    }

    /// Set metrics collector for auth operations
    pub fn with_metrics(mut self, metrics: Arc<AuthMetrics>) -> Self {
        self.metrics = Some(metrics);
        self
    }

    /// Get metrics snapshot if metrics are enabled
    pub fn metrics_snapshot(&self) -> Option<AuthMetricsSnapshot> {
        self.metrics.as_ref().map(|m| m.snapshot())
    }

    fn extract_token<'a>(&self, request: &'a ConnectionAuthRequest) -> Option<&'a str> {
        request
            .bearer_token()
            .or_else(|| request.query_param(&self.query_param_name))
    }

    /// Verify a token for in-band refresh and return the auth context
    ///
    /// This is used when a client wants to refresh their auth without reconnecting.
    /// The origin is NOT validated here - we assume the client has already proven
    /// origin at connection time, and we're just refreshing the session token.
    pub async fn verify_refresh_token(&self, token: &str) -> Result<AuthContext, AuthDeny> {
        let result = match &self.verifier {
            SignedSessionVerifier::Static(verifier) => verifier.verify(token, None, None),
            SignedSessionVerifier::CachedJwks(verifier) => {
                verifier.verify_with_cache(token, None, None).await
            }
            SignedSessionVerifier::MultiKey(verifier) => verifier.verify(token, None, None).await,
        };

        match result {
            Ok(context) => Ok(context),
            Err(e) => Err(AuthDeny::from_verify_error(e)),
        }
    }
}

#[async_trait]
impl WebSocketAuthPlugin for SignedSessionAuthPlugin {
    async fn authorize(&self, request: &ConnectionAuthRequest) -> AuthDecision {
        let token = match self.extract_token(request) {
            Some(token) => token,
            None => {
                return AuthDecision::Deny(AuthDeny::token_missing());
            }
        };

        let expected_origin = request.origin.as_deref();

        let expected_client_ip = None; // IP validation can be added here if needed

        let result = match &self.verifier {
            SignedSessionVerifier::Static(verifier) => {
                verifier.verify(token, expected_origin, expected_client_ip)
            }
            SignedSessionVerifier::CachedJwks(verifier) => {
                verifier
                    .verify_with_cache(token, expected_origin, expected_client_ip)
                    .await
            }
            SignedSessionVerifier::MultiKey(verifier) => {
                verifier
                    .verify(token, expected_origin, expected_client_ip)
                    .await
            }
        };

        match result {
            Ok(context) => {
                // Log successful authentication
                let event = auth_success_event(&context.subject)
                    .with_client_ip(request.remote_addr)
                    .with_path(&request.path);
                if let Some(origin) = &request.origin {
                    let event = event.with_origin(origin.clone());
                    self.log_audit(event).await;
                } else {
                    self.log_audit(event).await;
                }
                AuthDecision::Allow(context)
            }
            Err(e) => {
                let deny = AuthDeny::from_verify_error(e);
                // Log failed authentication
                let event = auth_failure_event(&deny.code, &deny.reason)
                    .with_client_ip(request.remote_addr)
                    .with_path(&request.path);
                let event = if let Some(origin) = &request.origin {
                    event.with_origin(origin.clone())
                } else {
                    event
                };
                self.log_audit(event).await;
                AuthDecision::Deny(deny)
            }
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn audit_logger(&self) -> Option<&dyn SecurityAuditLogger> {
        self.audit_logger.as_ref().map(|l| l.as_ref())
    }

    fn auth_metrics(&self) -> Option<&AuthMetrics> {
        self.metrics.as_ref().map(|m| m.as_ref())
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

    // Integration tests for handshake auth failures

    #[tokio::test]
    async fn signed_session_plugin_denies_missing_token() {
        use arete_auth::TokenSigner;

        let signing_key = arete_auth::SigningKey::generate();
        let verifying_key = signing_key.verifying_key();
        let verifier =
            arete_auth::TokenVerifier::new(verifying_key, "test-issuer", "test-audience");
        let plugin = SignedSessionAuthPlugin::new(verifier);

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

        if let AuthDecision::Deny(deny) = decision {
            assert_eq!(deny.code, AuthErrorCode::TokenMissing);
        } else {
            panic!("Expected Deny decision");
        }
    }

    #[tokio::test]
    async fn signed_session_plugin_denies_expired_token() {
        use arete_auth::{KeyClass, SessionClaims, TokenSigner};
        use std::time::{SystemTime, UNIX_EPOCH};

        let signing_key = arete_auth::SigningKey::generate();
        let verifying_key = signing_key.verifying_key();
        let signer = TokenSigner::new(signing_key, "test-issuer");
        let verifier =
            arete_auth::TokenVerifier::new(verifying_key, "test-issuer", "test-audience");
        let plugin = SignedSessionAuthPlugin::new(verifier);

        // Create a token that expired 1 hour ago
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let claims = SessionClaims::builder("test-issuer", "test-subject", "test-audience")
            .with_scope("read")
            .with_key_class(KeyClass::Secret)
            .build();

        // Manually create expired claims
        let mut expired_claims = claims;
        expired_claims.exp = now - 3600; // Expired 1 hour ago
        expired_claims.iat = now - 7200; // Issued 2 hours ago
        expired_claims.nbf = now - 7200;

        let token = signer.sign(expired_claims).unwrap();

        let request = Request::builder()
            .uri(format!("/ws?hs_token={}", token))
            .body(())
            .expect("request should build");
        let auth_request = ConnectionAuthRequest::from_http_request(
            "127.0.0.1:8877".parse().expect("socket addr should parse"),
            &request,
        );

        let decision = plugin.authorize(&auth_request).await;
        assert!(!decision.is_allowed());

        if let AuthDecision::Deny(deny) = decision {
            assert_eq!(deny.code, AuthErrorCode::TokenExpired);
        } else {
            panic!("Expected Deny decision for expired token");
        }
    }

    #[tokio::test]
    async fn signed_session_plugin_denies_invalid_signature() {
        use arete_auth::{KeyClass, SessionClaims, TokenSigner};

        // Create two different key pairs
        let signing_key = arete_auth::SigningKey::generate();
        let wrong_key = arete_auth::SigningKey::generate();

        // Sign with one key, verify with another
        let signer = TokenSigner::new(signing_key, "test-issuer");
        let wrong_verifying_key = wrong_key.verifying_key();
        let verifier = arete_auth::TokenVerifier::new(
            wrong_verifying_key,
            "test-issuer",
            "test-audience",
        );
        let plugin = SignedSessionAuthPlugin::new(verifier);

        let claims = SessionClaims::builder("test-issuer", "test-subject", "test-audience")
            .with_scope("read")
            .with_key_class(KeyClass::Secret)
            .build();

        let token = signer.sign(claims).unwrap();

        let request = Request::builder()
            .uri(format!("/ws?hs_token={}", token))
            .body(())
            .expect("request should build");
        let auth_request = ConnectionAuthRequest::from_http_request(
            "127.0.0.1:8877".parse().expect("socket addr should parse"),
            &request,
        );

        let decision = plugin.authorize(&auth_request).await;
        assert!(!decision.is_allowed());

        if let AuthDecision::Deny(deny) = decision {
            assert_eq!(deny.code, AuthErrorCode::TokenInvalidSignature);
        } else {
            panic!("Expected Deny decision for invalid signature");
        }
    }

    #[tokio::test]
    async fn signed_session_plugin_denies_wrong_audience() {
        use arete_auth::{KeyClass, SessionClaims, TokenSigner};

        let signing_key = arete_auth::SigningKey::generate();
        let verifying_key = signing_key.verifying_key();
        let signer = TokenSigner::new(signing_key, "test-issuer");

        // Verifier expects "test-audience", token is for "wrong-audience"
        let verifier =
            arete_auth::TokenVerifier::new(verifying_key, "test-issuer", "test-audience");
        let plugin = SignedSessionAuthPlugin::new(verifier);

        let claims = SessionClaims::builder("test-issuer", "test-subject", "wrong-audience")
            .with_scope("read")
            .with_key_class(KeyClass::Secret)
            .build();

        let token = signer.sign(claims).unwrap();

        let request = Request::builder()
            .uri(format!("/ws?hs_token={}", token))
            .body(())
            .expect("request should build");
        let auth_request = ConnectionAuthRequest::from_http_request(
            "127.0.0.1:8877".parse().expect("socket addr should parse"),
            &request,
        );

        let decision = plugin.authorize(&auth_request).await;
        assert!(!decision.is_allowed());

        if let AuthDecision::Deny(deny) = decision {
            assert_eq!(deny.code, AuthErrorCode::TokenInvalidAudience);
        } else {
            panic!("Expected Deny decision for wrong audience");
        }
    }

    #[tokio::test]
    async fn signed_session_plugin_denies_origin_mismatch() {
        use arete_auth::{KeyClass, SessionClaims, TokenSigner};

        let signing_key = arete_auth::SigningKey::generate();
        let verifying_key = signing_key.verifying_key();
        let signer = TokenSigner::new(signing_key, "test-issuer");

        // Verifier requires origin validation
        let verifier =
            arete_auth::TokenVerifier::new(verifying_key, "test-issuer", "test-audience")
                .with_origin_validation();
        let plugin = SignedSessionAuthPlugin::new(verifier).with_origin_validation();

        // Token bound to specific origin
        let claims = SessionClaims::builder("test-issuer", "test-subject", "test-audience")
            .with_scope("read")
            .with_key_class(KeyClass::Secret)
            .with_origin("https://allowed.example.com")
            .build();

        let token = signer.sign(claims).unwrap();

        // Request from different origin
        let request = Request::builder()
            .uri(format!("/ws?hs_token={}", token))
            .header("Origin", "https://evil.example.com")
            .body(())
            .expect("request should build");
        let auth_request = ConnectionAuthRequest::from_http_request(
            "127.0.0.1:8877".parse().expect("socket addr should parse"),
            &request,
        );

        let decision = plugin.authorize(&auth_request).await;
        assert!(!decision.is_allowed());

        if let AuthDecision::Deny(deny) = decision {
            assert_eq!(deny.code, AuthErrorCode::OriginMismatch);
        } else {
            panic!("Expected Deny decision for origin mismatch");
        }
    }

    #[tokio::test]
    async fn signed_session_plugin_allows_valid_token() {
        use arete_auth::{KeyClass, SessionClaims, TokenSigner};

        let signing_key = arete_auth::SigningKey::generate();
        let verifying_key = signing_key.verifying_key();
        let signer = TokenSigner::new(signing_key, "test-issuer");
        let verifier =
            arete_auth::TokenVerifier::new(verifying_key, "test-issuer", "test-audience");
        let plugin = SignedSessionAuthPlugin::new(verifier);

        let claims = SessionClaims::builder("test-issuer", "test-subject", "test-audience")
            .with_scope("read")
            .with_key_class(KeyClass::Secret)
            .with_metering_key("meter-123")
            .build();

        let token = signer.sign(claims).unwrap();

        let request = Request::builder()
            .uri(format!("/ws?hs_token={}", token))
            .body(())
            .expect("request should build");
        let auth_request = ConnectionAuthRequest::from_http_request(
            "127.0.0.1:8877".parse().expect("socket addr should parse"),
            &request,
        );

        let decision = plugin.authorize(&auth_request).await;
        assert!(decision.is_allowed());

        if let AuthDecision::Allow(ctx) = decision {
            assert_eq!(ctx.subject, "test-subject");
            assert_eq!(ctx.metering_key, "meter-123");
            assert_eq!(ctx.key_class, KeyClass::Secret);
        } else {
            panic!("Expected Allow decision");
        }
    }

    #[tokio::test]
    async fn signed_session_plugin_allows_with_matching_origin() {
        use arete_auth::{KeyClass, SessionClaims, TokenSigner};

        let signing_key = arete_auth::SigningKey::generate();
        let verifying_key = signing_key.verifying_key();
        let signer = TokenSigner::new(signing_key, "test-issuer");

        let verifier =
            arete_auth::TokenVerifier::new(verifying_key, "test-issuer", "test-audience")
                .with_origin_validation();
        let plugin = SignedSessionAuthPlugin::new(verifier).with_origin_validation();

        let claims = SessionClaims::builder("test-issuer", "test-subject", "test-audience")
            .with_scope("read")
            .with_key_class(KeyClass::Secret)
            .with_origin("https://trusted.example.com")
            .build();

        let token = signer.sign(claims).unwrap();

        let request = Request::builder()
            .uri(format!("/ws?hs_token={}", token))
            .header("Origin", "https://trusted.example.com")
            .body(())
            .expect("request should build");
        let auth_request = ConnectionAuthRequest::from_http_request(
            "127.0.0.1:8877".parse().expect("socket addr should parse"),
            &request,
        );

        let decision = plugin.authorize(&auth_request).await;
        assert!(decision.is_allowed());

        if let AuthDecision::Allow(ctx) = decision {
            assert_eq!(ctx.origin, Some("https://trusted.example.com".to_string()));
        } else {
            panic!("Expected Allow decision");
        }
    }

    #[tokio::test]
    async fn signed_session_plugin_allows_token_with_origin_when_no_origin_provided_and_not_required(
    ) {
        // This tests the non-browser client scenario (Rust, Python, etc.)
        // where the client doesn't send an Origin header.
        // The token has an origin claim from when it was minted via browser/API,
        // but when the plugin doesn't require origin, the connection should still be allowed.
        use arete_auth::{KeyClass, SessionClaims, TokenSigner};

        let signing_key = arete_auth::SigningKey::generate();
        let verifying_key = signing_key.verifying_key();
        let signer = TokenSigner::new(signing_key, "test-issuer");

        // Plugin WITHOUT origin validation (default for public stacks)
        let verifier =
            arete_auth::TokenVerifier::new(verifying_key, "test-issuer", "test-audience");
        let plugin = SignedSessionAuthPlugin::new(verifier);

        let claims = SessionClaims::builder("test-issuer", "test-subject", "test-audience")
            .with_scope("read")
            .with_key_class(KeyClass::Publishable)
            .with_origin("https://example.com") // Token has origin claim
            .build();

        let token = signer.sign(claims).unwrap();

        // No Origin header provided (simulating non-browser client)
        let request = Request::builder()
            .uri(format!("/ws?hs_token={}", token))
            .body(())
            .expect("request should build");
        let auth_request = ConnectionAuthRequest::from_http_request(
            "127.0.0.1:8877".parse().expect("socket addr should parse"),
            &request,
        );

        // Should succeed even without Origin header
        let decision = plugin.authorize(&auth_request).await;
        assert!(decision.is_allowed(), "Expected Allow decision for non-browser client without Origin");

        if let AuthDecision::Allow(ctx) = decision {
            assert_eq!(ctx.origin, Some("https://example.com".to_string()));
        } else {
            panic!("Expected Allow decision");
        }
    }

    #[tokio::test]
    async fn signed_session_plugin_validates_origin_when_provided_even_when_not_required() {
        // When origin IS provided, it should still be validated against the token
        // even when require_origin is false (defense-in-depth)
        use arete_auth::{KeyClass, SessionClaims, TokenSigner};

        let signing_key = arete_auth::SigningKey::generate();
        let verifying_key = signing_key.verifying_key();
        let signer = TokenSigner::new(signing_key, "test-issuer");

        // Plugin WITHOUT origin validation (default)
        let verifier =
            arete_auth::TokenVerifier::new(verifying_key, "test-issuer", "test-audience");
        let plugin = SignedSessionAuthPlugin::new(verifier);

        let claims = SessionClaims::builder("test-issuer", "test-subject", "test-audience")
            .with_scope("read")
            .with_key_class(KeyClass::Publishable)
            .with_origin("https://allowed.example.com")
            .build();

        let token = signer.sign(claims).unwrap();

        // Origin provided and matches - should succeed
        let request = Request::builder()
            .uri(format!("/ws?hs_token={}", token))
            .header("Origin", "https://allowed.example.com")
            .body(())
            .expect("request should build");
        let auth_request = ConnectionAuthRequest::from_http_request(
            "127.0.0.1:8877".parse().expect("socket addr should parse"),
            &request,
        );

        let decision = plugin.authorize(&auth_request).await;
        assert!(decision.is_allowed());

        // Origin provided but doesn't match - should fail
        let request = Request::builder()
            .uri(format!("/ws?hs_token={}", token))
            .header("Origin", "https://evil.example.com")
            .body(())
            .expect("request should build");
        let auth_request = ConnectionAuthRequest::from_http_request(
            "127.0.0.1:8877".parse().expect("socket addr should parse"),
            &request,
        );

        let decision = plugin.authorize(&auth_request).await;
        assert!(!decision.is_allowed());

        if let AuthDecision::Deny(deny) = decision {
            assert_eq!(deny.code, AuthErrorCode::OriginMismatch);
        } else {
            panic!("Expected Deny decision for origin mismatch");
        }
    }

    // Tests for AuthErrorCode utility methods
    #[test]
    fn auth_error_code_should_retry_logic() {
        assert!(AuthErrorCode::RateLimitExceeded.should_retry());
        assert!(AuthErrorCode::InternalError.should_retry());
        assert!(!AuthErrorCode::TokenExpired.should_retry());
        assert!(!AuthErrorCode::TokenInvalidSignature.should_retry());
        assert!(!AuthErrorCode::TokenMissing.should_retry());
    }

    #[test]
    fn auth_error_code_should_refresh_token_logic() {
        assert!(AuthErrorCode::TokenExpired.should_refresh_token());
        assert!(AuthErrorCode::TokenInvalidSignature.should_refresh_token());
        assert!(AuthErrorCode::TokenInvalidFormat.should_refresh_token());
        assert!(AuthErrorCode::TokenInvalidIssuer.should_refresh_token());
        assert!(AuthErrorCode::TokenInvalidAudience.should_refresh_token());
        assert!(AuthErrorCode::TokenKeyNotFound.should_refresh_token());
        assert!(!AuthErrorCode::TokenMissing.should_refresh_token());
        assert!(!AuthErrorCode::RateLimitExceeded.should_refresh_token());
        assert!(!AuthErrorCode::ConnectionLimitExceeded.should_refresh_token());
    }

    #[test]
    fn auth_error_code_string_representation() {
        assert_eq!(AuthErrorCode::TokenMissing.as_str(), "token-missing");
        assert_eq!(AuthErrorCode::TokenExpired.as_str(), "token-expired");
        assert_eq!(
            AuthErrorCode::TokenInvalidSignature.as_str(),
            "token-invalid-signature"
        );
        assert_eq!(
            AuthErrorCode::RateLimitExceeded.as_str(),
            "rate-limit-exceeded"
        );
        assert_eq!(
            AuthErrorCode::ConnectionLimitExceeded.as_str(),
            "connection-limit-exceeded"
        );
    }

    // Tests for AuthDeny construction
    #[test]
    fn auth_deny_token_missing_factory() {
        let deny = AuthDeny::token_missing();
        assert_eq!(deny.code, AuthErrorCode::TokenMissing);
        assert!(deny.reason.contains("Missing session token"));
    }

    #[test]
    fn auth_deny_from_verify_error_mapping() {
        use arete_auth::VerifyError;

        let test_cases = vec![
            (VerifyError::Expired, AuthErrorCode::TokenExpired),
            (
                VerifyError::InvalidSignature,
                AuthErrorCode::TokenInvalidSignature,
            ),
            (
                VerifyError::InvalidIssuer,
                AuthErrorCode::TokenInvalidIssuer,
            ),
            (
                VerifyError::InvalidAudience,
                AuthErrorCode::TokenInvalidAudience,
            ),
            (
                VerifyError::KeyNotFound("kid123".to_string()),
                AuthErrorCode::TokenKeyNotFound,
            ),
            (
                VerifyError::OriginMismatch {
                    expected: "a".to_string(),
                    actual: "b".to_string(),
                },
                AuthErrorCode::OriginMismatch,
            ),
        ];

        for (err, expected_code) in test_cases {
            let deny = AuthDeny::from_verify_error(err);
            assert_eq!(deny.code, expected_code);
        }
    }

    // Tests for multiple auth failure scenarios in sequence
    #[tokio::test]
    async fn signed_session_plugin_handles_multiple_failure_reasons() {
        use arete_auth::{KeyClass, SessionClaims, TokenSigner};

        let signing_key = arete_auth::SigningKey::generate();
        let verifying_key = signing_key.verifying_key();
        let signer = TokenSigner::new(signing_key, "test-issuer");
        let verifier =
            arete_auth::TokenVerifier::new(verifying_key, "test-issuer", "test-audience")
                .with_origin_validation();
        let plugin = SignedSessionAuthPlugin::new(verifier).with_origin_validation();

        // Test 1: Missing token
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
        match decision {
            AuthDecision::Deny(deny) => assert_eq!(deny.code, AuthErrorCode::TokenMissing),
            _ => panic!("Expected Deny decision"),
        }

        // Test 2: Valid token with wrong origin
        let claims = SessionClaims::builder("test-issuer", "test-subject", "test-audience")
            .with_scope("read")
            .with_key_class(KeyClass::Secret)
            .with_origin("https://allowed.example.com")
            .build();
        let token = signer.sign(claims).unwrap();

        let request = Request::builder()
            .uri(format!("/ws?hs_token={}", token))
            .header("Origin", "https://evil.example.com")
            .body(())
            .expect("request should build");
        let auth_request = ConnectionAuthRequest::from_http_request(
            "127.0.0.1:8877".parse().expect("socket addr should parse"),
            &request,
        );
        let decision = plugin.authorize(&auth_request).await;
        assert!(!decision.is_allowed());
        match decision {
            AuthDecision::Deny(deny) => assert_eq!(deny.code, AuthErrorCode::OriginMismatch),
            _ => panic!("Expected Deny decision for origin mismatch"),
        }

        // Test 3: Valid token with correct origin
        let claims = SessionClaims::builder("test-issuer", "test-subject", "test-audience")
            .with_scope("read")
            .with_key_class(KeyClass::Secret)
            .with_origin("https://allowed.example.com")
            .build();
        let token = signer.sign(claims).unwrap();

        let request = Request::builder()
            .uri(format!("/ws?hs_token={}", token))
            .header("Origin", "https://allowed.example.com")
            .body(())
            .expect("request should build");
        let auth_request = ConnectionAuthRequest::from_http_request(
            "127.0.0.1:8877".parse().expect("socket addr should parse"),
            &request,
        );
        let decision = plugin.authorize(&auth_request).await;
        assert!(decision.is_allowed());
    }

    // Test for rate limit error code
    #[tokio::test]
    async fn auth_deney_with_rate_limit_code() {
        let deny = AuthDeny::new(
            AuthErrorCode::RateLimitExceeded,
            "Too many requests from this IP",
        );
        assert_eq!(deny.code, AuthErrorCode::RateLimitExceeded);
        assert!(deny.code.should_retry());
        assert!(!deny.code.should_refresh_token());
    }

    // Test for connection limit error code
    #[tokio::test]
    async fn auth_deny_with_connection_limit_code() {
        let deny = AuthDeny::new(
            AuthErrorCode::ConnectionLimitExceeded,
            "Maximum connections exceeded for subject user-123",
        );
        assert_eq!(deny.code, AuthErrorCode::ConnectionLimitExceeded);
        assert!(!deny.code.should_retry());
        assert!(!deny.code.should_refresh_token());
    }

    // Integration-style test: Token extraction from various sources
    #[test]
    fn token_extraction_priority() {
        // Header takes priority over query param
        let request = Request::builder()
            .uri("/ws?hs_token=query-value")
            .header("Authorization", "Bearer header-value")
            .body(())
            .expect("request should build");
        let auth_request = ConnectionAuthRequest::from_http_request(
            "127.0.0.1:8877".parse().expect("socket addr should parse"),
            &request,
        );

        // bearer_token should return header value
        assert_eq!(auth_request.bearer_token(), Some("header-value"));
        // query_param should return query value
        assert_eq!(auth_request.query_param("hs_token"), Some("query-value"));
    }

    // Test malformed authorization header handling
    #[test]
    fn malformed_authorization_header() {
        let test_cases = vec![
            ("Basic dXNlcjpwYXNz", None),                // Wrong scheme
            ("Bearer", None),                            // Missing token (no space after Bearer)
            ("", None),                                  // Empty
            ("Bearer token extra", Some("token extra")), // Extra parts (token includes everything after scheme)
        ];

        for (header_value, expected) in test_cases {
            let request = Request::builder()
                .uri("/ws")
                .header("Authorization", header_value)
                .body(())
                .expect("request should build");
            let auth_request = ConnectionAuthRequest::from_http_request(
                "127.0.0.1:8877".parse().expect("socket addr should parse"),
                &request,
            );
            assert_eq!(
                auth_request.bearer_token(),
                expected,
                "Failed for header: {}",
                header_value
            );
        }
    }

    // ============================================
    // WEBSOCKET HANDSHAKE AUTH FAILURE TESTS
    // ============================================
    // These tests simulate real-world handshake failure scenarios

    #[test]
    fn auth_deny_error_response_structure() {
        let deny = AuthDeny::new(AuthErrorCode::TokenExpired, "Token has expired")
            .with_field("exp")
            .with_context("Token expired 5 minutes ago")
            .with_suggested_action("Refresh your authentication token")
            .with_docs_url("https://docs.arete.run/auth/errors#token-expired");

        let response = deny.to_error_response();

        assert_eq!(response.code, "token-expired");
        assert_eq!(response.message, "Token has expired");
        assert_eq!(response.error, "token-expired");
        assert!(response.retryable);
        assert_eq!(
            response.suggested_action,
            Some("Refresh your authentication token".to_string())
        );
        assert_eq!(
            response.docs_url,
            Some("https://docs.arete.run/auth/errors#token-expired".to_string())
        );
    }

    #[test]
    fn auth_deny_rate_limited_response() {
        use std::time::Duration;

        let deny = AuthDeny::rate_limited(Duration::from_secs(30), "websocket connections");
        let response = deny.to_error_response();

        assert_eq!(response.code, "rate-limit-exceeded");
        assert!(response.message.contains("30s"));
        assert!(response.retryable);
        assert_eq!(response.retry_after, Some(30));
    }

    #[test]
    fn auth_deny_connection_limit_response() {
        let deny = AuthDeny::connection_limit_exceeded("user-123", 5, 5);
        let response = deny.to_error_response();

        assert_eq!(response.code, "connection-limit-exceeded");
        assert!(response.message.contains("user-123"));
        assert!(response.message.contains("5 of 5"));
        assert!(response.retryable); // Connection limits are retryable (may become available)
    }

    #[test]
    fn retry_policy_immediate() {
        let deny = AuthDeny::new(AuthErrorCode::InternalError, "Transient error")
            .with_retry_policy(RetryPolicy::RetryImmediately);

        assert_eq!(deny.retry_policy, RetryPolicy::RetryImmediately);
    }

    #[test]
    fn retry_policy_with_backoff() {
        use std::time::Duration;

        let deny = AuthDeny::new(AuthErrorCode::RateLimitExceeded, "Too many requests")
            .with_retry_policy(RetryPolicy::RetryWithBackoff {
                initial: Duration::from_secs(1),
                max: Duration::from_secs(60),
            });

        match deny.retry_policy {
            RetryPolicy::RetryWithBackoff { initial, max } => {
                assert_eq!(initial, Duration::from_secs(1));
                assert_eq!(max, Duration::from_secs(60));
            }
            _ => panic!("Expected RetryWithBackoff"),
        }
    }

    #[test]
    fn auth_error_code_http_status_mapping() {
        assert_eq!(AuthErrorCode::TokenMissing.http_status(), 401);
        assert_eq!(AuthErrorCode::TokenExpired.http_status(), 401);
        assert_eq!(AuthErrorCode::TokenInvalidSignature.http_status(), 401);
        assert_eq!(AuthErrorCode::OriginMismatch.http_status(), 403);
        assert_eq!(AuthErrorCode::RateLimitExceeded.http_status(), 429);
        assert_eq!(AuthErrorCode::ConnectionLimitExceeded.http_status(), 429);
        assert_eq!(AuthErrorCode::InternalError.http_status(), 500);
    }

    #[test]
    fn auth_error_code_default_retry_policies() {
        use std::time::Duration;

        // Should refresh token
        assert!(matches!(
            AuthErrorCode::TokenExpired.default_retry_policy(),
            RetryPolicy::RetryWithFreshToken
        ));
        assert!(matches!(
            AuthErrorCode::TokenInvalidSignature.default_retry_policy(),
            RetryPolicy::RetryWithFreshToken
        ));

        // Should retry with backoff
        assert!(matches!(
            AuthErrorCode::RateLimitExceeded.default_retry_policy(),
            RetryPolicy::RetryWithBackoff { .. }
        ));
        assert!(matches!(
            AuthErrorCode::InternalError.default_retry_policy(),
            RetryPolicy::RetryWithBackoff { .. }
        ));

        // Should not retry
        assert!(matches!(
            AuthErrorCode::TokenMissing.default_retry_policy(),
            RetryPolicy::NoRetry
        ));
        assert!(matches!(
            AuthErrorCode::OriginMismatch.default_retry_policy(),
            RetryPolicy::NoRetry
        ));
    }

    // Simulated handshake scenarios

    #[tokio::test]
    async fn handshake_rejects_missing_token_with_proper_error() {
        use tokio_tungstenite::tungstenite::http::StatusCode;

        let plugin = AllowAllAuthPlugin;

        // Create a request without a token
        let request = Request::builder()
            .uri("/ws")
            .body(())
            .expect("request should build");

        let auth_request = ConnectionAuthRequest::from_http_request(
            "127.0.0.1:8877".parse().expect("socket addr should parse"),
            &request,
        );

        // For this test, we'll use a plugin that requires tokens
        // Actually AllowAllAuthPlugin doesn't require tokens, so let's create a static token plugin
        let static_plugin = StaticTokenAuthPlugin::new(["valid-token".to_string()]);
        let decision = static_plugin.authorize(&auth_request).await;

        assert!(!decision.is_allowed());

        if let AuthDecision::Deny(deny) = decision {
            assert_eq!(deny.code, AuthErrorCode::TokenMissing);
            assert_eq!(deny.http_status, 401);
            assert!(deny.reason.contains("Missing"));
        } else {
            panic!("Expected Deny decision");
        }
    }

    #[tokio::test]
    async fn handshake_rejects_expired_token_with_retry_hint() {
        use arete_auth::{KeyClass, SessionClaims, TokenSigner};
        use std::time::{SystemTime, UNIX_EPOCH};

        let signing_key = arete_auth::SigningKey::generate();
        let verifying_key = signing_key.verifying_key();
        let signer = TokenSigner::new(signing_key, "test-issuer");

        // Create an expired token
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let claims = SessionClaims::builder("test-issuer", "test-subject", "test-audience")
            .with_scope("read")
            .with_key_class(KeyClass::Secret)
            .build();

        let mut expired_claims = claims;
        expired_claims.exp = now - 3600;
        expired_claims.iat = now - 7200;
        expired_claims.nbf = now - 7200;

        let token = signer.sign(expired_claims).unwrap();

        // Create verifier and plugin
        let verifier =
            arete_auth::TokenVerifier::new(verifying_key, "test-issuer", "test-audience");
        let plugin = SignedSessionAuthPlugin::new(verifier);

        let request = Request::builder()
            .uri(format!("/ws?hs_token={}", token))
            .body(())
            .expect("request should build");

        let auth_request = ConnectionAuthRequest::from_http_request(
            "127.0.0.1:8877".parse().expect("socket addr should parse"),
            &request,
        );

        let decision = plugin.authorize(&auth_request).await;

        assert!(!decision.is_allowed());

        if let AuthDecision::Deny(deny) = decision {
            assert_eq!(deny.code, AuthErrorCode::TokenExpired);
            assert_eq!(deny.http_status, 401);
            // Should suggest refreshing the token
            assert!(matches!(
                deny.retry_policy,
                RetryPolicy::RetryWithFreshToken
            ));
        } else {
            panic!("Expected Deny decision");
        }
    }

    #[tokio::test]
    async fn handshake_rejects_invalid_signature_with_retry_hint() {
        use arete_auth::{KeyClass, SessionClaims, TokenSigner};

        // Create two different key pairs
        let signing_key = arete_auth::SigningKey::generate();
        let wrong_key = arete_auth::SigningKey::generate();

        // Sign with one key, verify with another
        let signer = TokenSigner::new(signing_key, "test-issuer");
        let wrong_verifying_key = wrong_key.verifying_key();
        let verifier = arete_auth::TokenVerifier::new(
            wrong_verifying_key,
            "test-issuer",
            "test-audience",
        );
        let plugin = SignedSessionAuthPlugin::new(verifier);

        let claims = SessionClaims::builder("test-issuer", "test-subject", "test-audience")
            .with_scope("read")
            .with_key_class(KeyClass::Secret)
            .build();

        let token = signer.sign(claims).unwrap();

        let request = Request::builder()
            .uri(format!("/ws?hs_token={}", token))
            .body(())
            .expect("request should build");

        let auth_request = ConnectionAuthRequest::from_http_request(
            "127.0.0.1:8877".parse().expect("socket addr should parse"),
            &request,
        );

        let decision = plugin.authorize(&auth_request).await;

        assert!(!decision.is_allowed());

        if let AuthDecision::Deny(deny) = decision {
            assert_eq!(deny.code, AuthErrorCode::TokenInvalidSignature);
            assert_eq!(deny.http_status, 401);
            // Should suggest refreshing the token
            assert!(matches!(
                deny.retry_policy,
                RetryPolicy::RetryWithFreshToken
            ));
        } else {
            panic!("Expected Deny decision");
        }
    }

    #[tokio::test]
    async fn handshake_rejects_origin_mismatch_without_retry() {
        use arete_auth::{KeyClass, SessionClaims, TokenSigner};

        let signing_key = arete_auth::SigningKey::generate();
        let verifying_key = signing_key.verifying_key();
        let signer = TokenSigner::new(signing_key, "test-issuer");

        let verifier =
            arete_auth::TokenVerifier::new(verifying_key, "test-issuer", "test-audience")
                .with_origin_validation();
        let plugin = SignedSessionAuthPlugin::new(verifier).with_origin_validation();

        // Token bound to specific origin
        let claims = SessionClaims::builder("test-issuer", "test-subject", "test-audience")
            .with_scope("read")
            .with_key_class(KeyClass::Secret)
            .with_origin("https://allowed.example.com")
            .build();

        let token = signer.sign(claims).unwrap();

        // Request from different origin
        let request = Request::builder()
            .uri(format!("/ws?hs_token={}", token))
            .header("Origin", "https://evil.example.com")
            .body(())
            .expect("request should build");

        let auth_request = ConnectionAuthRequest::from_http_request(
            "127.0.0.1:8877".parse().expect("socket addr should parse"),
            &request,
        );

        let decision = plugin.authorize(&auth_request).await;

        assert!(!decision.is_allowed());

        if let AuthDecision::Deny(deny) = decision {
            assert_eq!(deny.code, AuthErrorCode::OriginMismatch);
            assert_eq!(deny.http_status, 403);
            // Should NOT suggest retrying - this is a security issue
            assert!(matches!(deny.retry_policy, RetryPolicy::NoRetry));
        } else {
            panic!("Expected Deny decision");
        }
    }

    // Test that AuthDeny can be converted to HTTP error response
    #[test]
    fn auth_deny_to_http_response() {
        let deny = AuthDeny::new(AuthErrorCode::RateLimitExceeded, "Too many requests")
            .with_suggested_action("Wait before retrying")
            .with_retry_policy(RetryPolicy::RetryAfter(Duration::from_secs(30)));

        let response = deny.to_error_response();

        // Verify the response is serializable
        let json = serde_json::to_string(&response).expect("Should serialize");
        assert!(json.contains("rate-limit-exceeded"));
        assert!(json.contains("Too many requests"));
        assert!(json.contains("Wait before retrying"));
        assert!(json.contains("\"retryable\":true"));
        assert!(json.contains("\"retry_after\":30"));
    }

    // Test comprehensive error scenarios
    #[tokio::test]
    async fn comprehensive_auth_error_scenarios() {
        use arete_auth::{KeyClass, SessionClaims, TokenSigner};

        let signing_key = arete_auth::SigningKey::generate();
        let verifying_key = signing_key.verifying_key();
        let signer = TokenSigner::new(signing_key, "test-issuer");
        let verifier =
            arete_auth::TokenVerifier::new(verifying_key, "test-issuer", "test-audience");
        let plugin = SignedSessionAuthPlugin::new(verifier);

        let test_cases = vec![
            ("missing_token", None, AuthErrorCode::TokenMissing),
            (
                "invalid_format",
                Some("not-a-valid-token"),
                AuthErrorCode::TokenInvalidFormat,
            ),
        ];

        for (name, token, expected_code) in test_cases {
            let uri = token.map_or_else(|| "/ws".to_string(), |t| format!("/ws?hs_token={}", t));

            let request = Request::builder()
                .uri(&uri)
                .body(())
                .expect("request should build");

            let auth_request = ConnectionAuthRequest::from_http_request(
                "127.0.0.1:8877".parse().expect("socket addr should parse"),
                &request,
            );

            let decision = plugin.authorize(&auth_request).await;

            assert!(!decision.is_allowed(), "{}: should deny", name);

            if let AuthDecision::Deny(deny) = decision {
                assert_eq!(deny.code, expected_code, "{}: wrong error code", name);
            } else {
                panic!("{}: Expected Deny decision", name);
            }
        }
    }
}
