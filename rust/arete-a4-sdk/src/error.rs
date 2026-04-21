use serde::Deserialize;
use thiserror::Error;
use tokio_tungstenite::tungstenite::{self, http::Response};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SocketIssue {
    pub error: String,
    pub message: String,
    pub code: Option<AuthErrorCode>,
    pub retryable: bool,
    pub retry_after: Option<u64>,
    pub suggested_action: Option<String>,
    pub docs_url: Option<String>,
    pub fatal: bool,
}

impl std::fmt::Display for SocketIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct SocketIssuePayload {
    #[serde(rename = "type")]
    pub kind: String,
    pub error: String,
    pub message: String,
    pub code: String,
    pub retryable: bool,
    #[serde(default)]
    pub retry_after: Option<u64>,
    #[serde(default)]
    pub suggested_action: Option<String>,
    #[serde(default)]
    pub docs_url: Option<String>,
    pub fatal: bool,
}

impl SocketIssuePayload {
    pub fn is_socket_issue(&self) -> bool {
        self.kind == "error"
    }

    pub fn into_socket_issue(self) -> SocketIssue {
        SocketIssue {
            error: self.error,
            message: self.message,
            code: AuthErrorCode::from_wire(&self.code),
            retryable: self.retryable,
            retry_after: self.retry_after,
            suggested_action: self.suggested_action,
            docs_url: self.docs_url,
            fatal: self.fatal,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthErrorCode {
    TokenMissing,
    TokenExpired,
    TokenInvalidSignature,
    TokenInvalidFormat,
    TokenInvalidIssuer,
    TokenInvalidAudience,
    TokenMissingClaim,
    TokenKeyNotFound,
    OriginMismatch,
    OriginRequired,
    OriginNotAllowed,
    AuthRequired,
    MissingAuthorizationHeader,
    InvalidAuthorizationFormat,
    InvalidApiKey,
    ExpiredApiKey,
    UserNotFound,
    SecretKeyRequired,
    DeploymentAccessDenied,
    RateLimitExceeded,
    WebSocketSessionRateLimitExceeded,
    ConnectionLimitExceeded,
    SubscriptionLimitExceeded,
    SnapshotLimitExceeded,
    EgressLimitExceeded,
    QuotaExceeded,
    InvalidStaticToken,
    InternalError,
}

impl AuthErrorCode {
    pub fn from_wire(code: &str) -> Option<Self> {
        Some(match code.trim().to_ascii_lowercase().as_str() {
            "token-missing" => Self::TokenMissing,
            "token-expired" => Self::TokenExpired,
            "token-invalid-signature" => Self::TokenInvalidSignature,
            "token-invalid-format" => Self::TokenInvalidFormat,
            "token-invalid-issuer" => Self::TokenInvalidIssuer,
            "token-invalid-audience" => Self::TokenInvalidAudience,
            "token-missing-claim" => Self::TokenMissingClaim,
            "token-key-not-found" => Self::TokenKeyNotFound,
            "origin-mismatch" => Self::OriginMismatch,
            "origin-required" => Self::OriginRequired,
            "origin-not-allowed" => Self::OriginNotAllowed,
            "auth-required" => Self::AuthRequired,
            "missing-authorization-header" => Self::MissingAuthorizationHeader,
            "invalid-authorization-format" => Self::InvalidAuthorizationFormat,
            "invalid-api-key" => Self::InvalidApiKey,
            "expired-api-key" => Self::ExpiredApiKey,
            "user-not-found" => Self::UserNotFound,
            "secret-key-required" => Self::SecretKeyRequired,
            "deployment-access-denied" => Self::DeploymentAccessDenied,
            "rate-limit-exceeded" => Self::RateLimitExceeded,
            "websocket-session-rate-limit-exceeded" => Self::WebSocketSessionRateLimitExceeded,
            "connection-limit-exceeded" => Self::ConnectionLimitExceeded,
            "subscription-limit-exceeded" => Self::SubscriptionLimitExceeded,
            "snapshot-limit-exceeded" => Self::SnapshotLimitExceeded,
            "egress-limit-exceeded" => Self::EgressLimitExceeded,
            "quota-exceeded" => Self::QuotaExceeded,
            "invalid-static-token" => Self::InvalidStaticToken,
            "internal-error" => Self::InternalError,
            _ => return None,
        })
    }

    pub fn as_wire(self) -> &'static str {
        match self {
            Self::TokenMissing => "token-missing",
            Self::TokenExpired => "token-expired",
            Self::TokenInvalidSignature => "token-invalid-signature",
            Self::TokenInvalidFormat => "token-invalid-format",
            Self::TokenInvalidIssuer => "token-invalid-issuer",
            Self::TokenInvalidAudience => "token-invalid-audience",
            Self::TokenMissingClaim => "token-missing-claim",
            Self::TokenKeyNotFound => "token-key-not-found",
            Self::OriginMismatch => "origin-mismatch",
            Self::OriginRequired => "origin-required",
            Self::OriginNotAllowed => "origin-not-allowed",
            Self::AuthRequired => "auth-required",
            Self::MissingAuthorizationHeader => "missing-authorization-header",
            Self::InvalidAuthorizationFormat => "invalid-authorization-format",
            Self::InvalidApiKey => "invalid-api-key",
            Self::ExpiredApiKey => "expired-api-key",
            Self::UserNotFound => "user-not-found",
            Self::SecretKeyRequired => "secret-key-required",
            Self::DeploymentAccessDenied => "deployment-access-denied",
            Self::RateLimitExceeded => "rate-limit-exceeded",
            Self::WebSocketSessionRateLimitExceeded => "websocket-session-rate-limit-exceeded",
            Self::ConnectionLimitExceeded => "connection-limit-exceeded",
            Self::SubscriptionLimitExceeded => "subscription-limit-exceeded",
            Self::SnapshotLimitExceeded => "snapshot-limit-exceeded",
            Self::EgressLimitExceeded => "egress-limit-exceeded",
            Self::QuotaExceeded => "quota-exceeded",
            Self::InvalidStaticToken => "invalid-static-token",
            Self::InternalError => "internal-error",
        }
    }

    pub fn should_retry(self) -> bool {
        matches!(self, Self::InternalError)
    }

    pub fn should_refresh_token(self) -> bool {
        matches!(
            self,
            Self::TokenExpired
                | Self::TokenInvalidSignature
                | Self::TokenInvalidFormat
                | Self::TokenInvalidIssuer
                | Self::TokenInvalidAudience
                | Self::TokenKeyNotFound
        )
    }
}

impl std::fmt::Display for AuthErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_wire())
    }
}

#[derive(Error, Debug, Clone)]
pub enum AreteError {
    #[error("Missing WebSocket URL")]
    MissingUrl,

    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    #[error("WebSocket error: {message}")]
    WebSocket {
        message: String,
        code: Option<AuthErrorCode>,
    },

    #[error("WebSocket handshake rejected ({status}): {message}")]
    HandshakeRejected {
        status: u16,
        message: String,
        code: Option<AuthErrorCode>,
    },

    #[error("Authentication request failed ({status}): {message}")]
    AuthRequestFailed {
        status: u16,
        message: String,
        code: Option<AuthErrorCode>,
    },

    #[error("WebSocket closed by server: {message}")]
    ServerClosed {
        message: String,
        code: Option<AuthErrorCode>,
    },

    #[error("Socket issue: {0}")]
    SocketIssue(SocketIssue),

    #[error("JSON serialization error: {0}")]
    Serialization(String),

    #[error("Max reconnection attempts reached ({0})")]
    MaxReconnectAttempts(u32),

    #[error("Connection closed")]
    ConnectionClosed,

    #[error("Subscription failed: {0}")]
    SubscriptionFailed(String),

    #[error("Channel error: {0}")]
    ChannelError(String),
}

#[derive(Debug, Deserialize)]
struct ErrorPayload {
    error: Option<String>,
    code: Option<String>,
}

impl AreteError {
    pub fn auth_code(&self) -> Option<AuthErrorCode> {
        match self {
            Self::WebSocket { code, .. }
            | Self::HandshakeRejected { code, .. }
            | Self::AuthRequestFailed { code, .. }
            | Self::ServerClosed { code, .. } => *code,
            Self::SocketIssue(issue) => issue.code,
            _ => None,
        }
    }

    pub fn socket_issue(&self) -> Option<&SocketIssue> {
        match self {
            Self::SocketIssue(issue) => Some(issue),
            _ => None,
        }
    }

    pub fn should_retry(&self) -> bool {
        match self {
            Self::HandshakeRejected { status, code, .. }
            | Self::AuthRequestFailed { status, code, .. } => code
                .map(AuthErrorCode::should_retry)
                .unwrap_or(*status >= 500),
            Self::ServerClosed { code, .. } | Self::WebSocket { code, .. } => {
                code.map(AuthErrorCode::should_retry).unwrap_or(true)
            }
            Self::SocketIssue(issue) => issue.retryable,
            Self::ConnectionFailed(_) | Self::ConnectionClosed => true,
            Self::MissingUrl
            | Self::Serialization(_)
            | Self::MaxReconnectAttempts(_)
            | Self::SubscriptionFailed(_)
            | Self::ChannelError(_) => false,
        }
    }

    pub fn should_refresh_token(&self) -> bool {
        self.auth_code()
            .map(AuthErrorCode::should_refresh_token)
            .unwrap_or(false)
    }

    pub(crate) fn from_tungstenite(error: tungstenite::Error) -> Self {
        match error {
            tungstenite::Error::Http(response) => Self::from_http_response(response),
            other => Self::WebSocket {
                message: other.to_string(),
                code: None,
            },
        }
    }

    pub(crate) fn from_http_response(response: Response<Option<Vec<u8>>>) -> Self {
        let status = response.status().as_u16();
        let header_code = response
            .headers()
            .get("X-Error-Code")
            .and_then(|value| value.to_str().ok())
            .and_then(AuthErrorCode::from_wire);
        let (body_message, body_code) = parse_error_payload(response.body().as_deref());
        let code = header_code.or(body_code);

        let message = body_message.unwrap_or_else(|| {
            response
                .status()
                .canonical_reason()
                .unwrap_or("WebSocket handshake rejected")
                .to_string()
        });

        Self::HandshakeRejected {
            status,
            message,
            code,
        }
    }

    pub(crate) fn from_auth_response(
        status: u16,
        header_code: Option<&str>,
        body: Option<&[u8]>,
        fallback_message: Option<&str>,
    ) -> Self {
        let header_code = header_code.and_then(AuthErrorCode::from_wire);
        let (body_message, body_code) = parse_error_payload(body);
        let code = header_code.or(body_code);
        let message = body_message.unwrap_or_else(|| {
            fallback_message
                .unwrap_or("Authentication request failed")
                .to_string()
        });

        Self::AuthRequestFailed {
            status,
            message,
            code,
        }
    }

    pub(crate) fn from_close_reason(reason: &str) -> Option<Self> {
        let trimmed = reason.trim();
        if trimmed.is_empty() {
            return None;
        }

        let (code, message) = parse_close_reason(trimmed);
        Some(Self::ServerClosed { code, message })
    }

    pub(crate) fn from_socket_issue(issue: SocketIssue) -> Self {
        Self::SocketIssue(issue)
    }
}

impl From<serde_json::Error> for AreteError {
    fn from(value: serde_json::Error) -> Self {
        Self::Serialization(value.to_string())
    }
}

impl From<tungstenite::Error> for AreteError {
    fn from(value: tungstenite::Error) -> Self {
        Self::from_tungstenite(value)
    }
}

fn parse_error_payload(body: Option<&[u8]>) -> (Option<String>, Option<AuthErrorCode>) {
    let Some(body) = body.filter(|value| !value.is_empty()) else {
        return (None, None);
    };

    if let Ok(payload) = serde_json::from_slice::<ErrorPayload>(body) {
        let code = payload.code.as_deref().and_then(AuthErrorCode::from_wire);
        let message = payload.error.map(|value| value.trim().to_string());
        return (message.filter(|value| !value.is_empty()), code);
    }

    let message = String::from_utf8_lossy(body).trim().to_string();
    if message.is_empty() {
        (None, None)
    } else {
        (Some(message), None)
    }
}

fn parse_close_reason(reason: &str) -> (Option<AuthErrorCode>, String) {
    if let Some((wire_code, message)) = reason.split_once(':') {
        let code = AuthErrorCode::from_wire(wire_code);
        let message = message.trim();

        if code.is_some() && !message.is_empty() {
            return (code, message.to_string());
        }
    }

    (None, reason.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_platform_handshake_rejection() {
        let response = Response::builder()
            .status(403)
            .header("X-Error-Code", "origin-required")
            .body(Some(
                br#"{"error":"Publishable key requires Origin header","code":"origin-required"}"#
                    .to_vec(),
            ))
            .expect("response should build");

        let error = AreteError::from_http_response(response);
        assert!(matches!(
            error,
            AreteError::HandshakeRejected {
                status: 403,
                code: Some(AuthErrorCode::OriginRequired),
                ..
            }
        ));
        assert!(!error.should_retry());
    }

    #[test]
    fn parses_token_endpoint_error_response() {
        let error = AreteError::from_auth_response(
            429,
            Some("websocket-session-rate-limit-exceeded"),
            Some(
                br#"{"error":"WebSocket session mint rate limit exceeded","code":"websocket-session-rate-limit-exceeded"}"#,
            ),
            Some("Too Many Requests"),
        );

        assert!(matches!(
            error,
            AreteError::AuthRequestFailed {
                status: 429,
                code: Some(AuthErrorCode::WebSocketSessionRateLimitExceeded),
                ..
            }
        ));
        assert!(!error.should_retry());
    }

    #[test]
    fn parses_rate_limit_close_reason() {
        let error = AreteError::from_close_reason(
            "websocket-session-rate-limit-exceeded: WebSocket session mint rate limit exceeded",
        )
        .expect("close reason should parse");

        assert!(matches!(
            error,
            AreteError::ServerClosed {
                code: Some(AuthErrorCode::WebSocketSessionRateLimitExceeded),
                ..
            }
        ));
        assert!(!error.should_retry());
    }

    #[test]
    fn parses_unknown_close_reason_without_code() {
        let error = AreteError::from_close_reason("server maintenance")
            .expect("non-empty reason should be preserved");

        assert!(matches!(
            error,
            AreteError::ServerClosed {
                code: None,
                ref message,
            } if message == "server maintenance"
        ));
    }

    #[test]
    fn socket_issue_error_uses_issue_retryability() {
        let error = AreteError::from_socket_issue(SocketIssue {
            error: "subscription-limit-exceeded".to_string(),
            message: "subscription limit exceeded".to_string(),
            code: Some(AuthErrorCode::SubscriptionLimitExceeded),
            retryable: false,
            retry_after: None,
            suggested_action: Some("unsubscribe first".to_string()),
            docs_url: None,
            fatal: false,
        });

        assert!(!error.should_retry());
        assert!(
            matches!(error.socket_issue(), Some(issue) if issue.message == "subscription limit exceeded")
        );
    }
}
