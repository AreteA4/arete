use serde::Deserialize;
use thiserror::Error;
use tokio_tungstenite::tungstenite::{self, http::Response};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SocketIssue {
    pub protocol_version: u8,
    pub subscription_id: Option<String>,
    pub error: String,
    pub message: String,
    pub wire_code: String,
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

/// Why delivery on a subscription stopped.
///
/// The six wire variants are the server's replay refusals; each has its own
/// recovery, so they stay distinguishable instead of collapsing into one
/// "replay failed". [`GapCode::LocalLag`] is raised by this SDK when a
/// consumer's own bounded queue evicted cursor-bearing updates before the
/// consumer read them.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum GapCode {
    /// The cursor is older than the retained window. Resubscribe with no
    /// `after`: `after` is exclusive, so `after: earliest` would skip the
    /// oldest retained record.
    CursorExpired,
    /// The cursor belongs to a tape lifetime that no longer exists. Discard it.
    CursorEpochChanged,
    /// An offset this view never issued.
    CursorUnknown,
    /// Not an `{epoch}:{offset}` string at all.
    InvalidCursor,
    /// Replaying would cross a known discontinuity.
    ReplayGap,
    /// The server skipped records in flight; delivery on that subscription
    /// stopped.
    ReplayLagged,
    /// This consumer's queue overflowed and `skipped` updates were evicted
    /// before it read them. Never sent by the server.
    LocalLag { skipped: u64 },
    /// A record on a replayable view did not deserialize into the consumer's
    /// type. Skipping it would put a hole behind an advancing cursor, so
    /// delivery stops at it instead. `recover_from` is that record's own
    /// cursor: resuming from it accepts the loss, and a consumer that fixes
    /// its type replays from the cursor it stored earlier.
    Undecodable,
}

impl GapCode {
    pub fn from_wire(code: &str) -> Option<Self> {
        Some(match code {
            "cursor-expired" => Self::CursorExpired,
            "cursor-epoch-changed" => Self::CursorEpochChanged,
            "cursor-unknown" => Self::CursorUnknown,
            "invalid-cursor" => Self::InvalidCursor,
            "replay-gap" => Self::ReplayGap,
            "replay-lagged" => Self::ReplayLagged,
            _ => return None,
        })
    }

    pub fn as_wire(&self) -> &'static str {
        match self {
            Self::CursorExpired => "cursor-expired",
            Self::CursorEpochChanged => "cursor-epoch-changed",
            Self::CursorUnknown => "cursor-unknown",
            Self::InvalidCursor => "invalid-cursor",
            Self::ReplayGap => "replay-gap",
            Self::ReplayLagged => "replay-lagged",
            Self::LocalLag { .. } => "local-lag",
            Self::Undecodable => "undecodable",
        }
    }
}

/// Delivery on a subscription stopped. Always the final item of the stream
/// that reports it.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("stream gap ({code}){}", match .recover_from {
    Some(cursor) => format!("; recover from {cursor}"),
    None => "; resubscribe without a cursor".to_string(),
})]
pub struct StreamGap {
    pub code: GapCode,
    /// The cursor to resume from. `None` means no position is safe to resume
    /// from and the consumer must resubscribe with no `after`.
    pub recover_from: Option<String>,
    /// What the view can still serve, when the server said.
    pub replay_window: Option<crate::frame::ReplayWindow>,
}

impl std::fmt::Display for GapCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LocalLag { skipped } => write!(f, "local-lag: {skipped} updates evicted"),
            other => f.write_str(other.as_wire()),
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
    /// The session endpoint no longer serves the stack version this client
    /// was generated for. Terminal: see [`StackVersionRefusal`].
    StackVersionRetired,
    /// The session endpoint does not know the stack version this client was
    /// generated for. Terminal: see [`StackVersionRefusal`].
    StackVersionUnknown,
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
            "stack-version-retired" => Self::StackVersionRetired,
            "stack-version-unknown" => Self::StackVersionUnknown,
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
            Self::StackVersionRetired => "stack-version-retired",
            Self::StackVersionUnknown => "stack-version-unknown",
            Self::InternalError => "internal-error",
        }
    }

    pub fn should_retry(self) -> bool {
        matches!(self, Self::InternalError)
    }

    /// The session endpoint refused the client's stack version. No retry,
    /// token refresh or reconnect can change the answer.
    pub fn is_stack_version_refusal(self) -> bool {
        matches!(self, Self::StackVersionRetired | Self::StackVersionUnknown)
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

/// Structured fields of a `stack-version-retired` / `stack-version-unknown`
/// session refusal. Every field is optional on the wire.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StackVersionRefusal {
    /// Version that replaces the refused one, e.g. `1.3.0`.
    pub replacement_version: Option<String>,
    /// StackManifest hash of the replacement.
    pub replacement_stack_manifest_hash: Option<String>,
    /// Command that installs the replacement, e.g. `a4 install stack ore@1.3.0`.
    pub upgrade_command: Option<String>,
    /// RFC 3339 time the version was retired.
    pub retired_at: Option<String>,
}

impl StackVersionRefusal {
    /// `message` followed by the replacement and how to install it, when the
    /// server named them.
    fn describe(&self, message: String) -> String {
        let mut guidance = Vec::new();
        if let Some(replacement) = self
            .replacement_version
            .as_deref()
            .or(self.replacement_stack_manifest_hash.as_deref())
        {
            guidance.push(format!("Replacement: {replacement}."));
        }
        if let Some(command) = &self.upgrade_command {
            guidance.push(format!("Upgrade with: {command}"));
        }
        if guidance.is_empty() {
            return message;
        }
        let trimmed = message.trim_end();
        let separator = if trimmed.ends_with(['.', '!', '?']) {
            " "
        } else {
            ". "
        };
        format!("{trimmed}{separator}{}", guidance.join(" "))
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
        /// Present when `code` is a stack version refusal; `message` then
        /// already names the replacement and upgrade command.
        stack_version: Option<Box<StackVersionRefusal>>,
    },

    #[error("WebSocket closed by server: {message}")]
    ServerClosed {
        message: String,
        code: Option<AuthErrorCode>,
    },

    #[error("Socket issue: {0}")]
    SocketIssue(Box<SocketIssue>),

    #[error("JSON serialization error: {0}")]
    Serialization(String),

    #[error("Max reconnection attempts reached ({0})")]
    MaxReconnectAttempts(u32),

    #[error("Connection closed")]
    ConnectionClosed,

    #[error("Subscription failed: {0}")]
    SubscriptionFailed(String),

    #[error("WebSocket protocol error: {message}")]
    Protocol {
        message: String,
        subscription_id: Option<String>,
    },

    #[error("Channel error: {0}")]
    ChannelError(String),

    /// Invalid client/session/gateway configuration (mirror of the TS
    /// `INVALID_CONFIG` error code).
    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),

    /// View subscriptions require the streaming WebSocket, but this client
    /// was connected with `Transport::Http` (mirror of the TS
    /// `WEBSOCKET_DISABLED` error).
    #[error(
        "View subscriptions require the WebSocket transport; this client was connected with Transport::Http"
    )]
    WebSocketDisabled,

    /// A transaction dispatched through [`crate::Arete::transaction`] failed;
    /// carries the structured outcome from the operations failure model.
    #[error(
        "Transaction failed ({phase}): {message}",
        phase = .0.phase(),
        message = .0.message()
    )]
    TransactionFailed(Box<crate::operations::TransactionFailureOutcome>),
}

#[derive(Debug, Deserialize)]
struct ErrorPayload {
    error: Option<String>,
    code: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StackVersionRefusalPayload {
    #[serde(default)]
    replacement: Option<ReplacementPayload>,
    #[serde(default)]
    upgrade_command: Option<String>,
    #[serde(default)]
    retired_at: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReplacementPayload {
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    stack_manifest_hash: Option<String>,
}

fn parse_stack_version_refusal(body: Option<&[u8]>) -> StackVersionRefusal {
    let payload = body
        .and_then(|body| serde_json::from_slice::<StackVersionRefusalPayload>(body).ok())
        .unwrap_or_default();
    let non_empty = |value: Option<String>| value.filter(|value| !value.trim().is_empty());
    let replacement = payload.replacement.unwrap_or_default();
    StackVersionRefusal {
        replacement_version: non_empty(replacement.version),
        replacement_stack_manifest_hash: non_empty(replacement.stack_manifest_hash),
        upgrade_command: non_empty(payload.upgrade_command),
        retired_at: non_empty(payload.retired_at),
    }
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

    /// The replacement and upgrade guidance of a stack version refusal.
    pub fn stack_version_refusal(&self) -> Option<&StackVersionRefusal> {
        match self {
            Self::AuthRequestFailed {
                stack_version: Some(refusal),
                ..
            } => Some(refusal),
            _ => None,
        }
    }

    pub fn socket_issue(&self) -> Option<&SocketIssue> {
        match self {
            Self::SocketIssue(issue) => Some(issue),
            _ => None,
        }
    }

    /// Structured failure outcome for [`AreteError::TransactionFailed`].
    pub fn transaction_outcome(&self) -> Option<&crate::operations::TransactionFailureOutcome> {
        match self {
            Self::TransactionFailed(outcome) => Some(outcome),
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
            | Self::Protocol { .. }
            | Self::ChannelError(_)
            | Self::InvalidConfig(_)
            | Self::WebSocketDisabled
            | Self::TransactionFailed(_) => false,
        }
    }

    /// The session endpoint refused this client's stack version. Terminal:
    /// the SDK neither retries the request nor reconnects.
    pub fn is_stack_version_refusal(&self) -> bool {
        self.auth_code()
            .is_some_and(AuthErrorCode::is_stack_version_refusal)
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

        if code.is_some_and(AuthErrorCode::is_stack_version_refusal) {
            let refusal = parse_stack_version_refusal(body);
            return Self::AuthRequestFailed {
                status,
                message: refusal.describe(message),
                code,
                stack_version: Some(Box::new(refusal)),
            };
        }

        Self::AuthRequestFailed {
            status,
            message,
            code,
            stack_version: None,
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
        Self::SocketIssue(Box::new(issue))
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

/// A close reason is `code: message`, or a bare known code such as
/// `stack-version-retired`. Anything else carries no code.
fn parse_close_reason(reason: &str) -> (Option<AuthErrorCode>, String) {
    let reason = reason.trim();
    if let Some((wire_code, message)) = reason.split_once(':') {
        if let Some(code) = AuthErrorCode::from_wire(wire_code.trim()) {
            let message = message.trim();
            let message = if message.is_empty() {
                wire_code.trim()
            } else {
                message
            };
            return (Some(code), message.to_string());
        }
    }
    if let Some(code) = AuthErrorCode::from_wire(reason) {
        return (Some(code), reason.to_string());
    }

    (None, reason.to_string())
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
    fn a_retired_stack_version_names_its_replacement_and_is_terminal() {
        let error = AreteError::from_auth_response(
            409,
            Some("stack-version-retired"),
            Some(
                br#"{"error":"Stack ore 1.2.0 was retired.","code":"stack-version-retired","replacement":{"version":"1.3.0","stackManifestHash":"arete:h1:stack-manifest:sha256:bb"},"upgradeCommand":"a4 install stack ore@1.3.0","retiredAt":"2026-10-01T00:00:00Z"}"#,
            ),
            Some("Conflict"),
        );

        assert_eq!(
            error.to_string(),
            "Authentication request failed (409): Stack ore 1.2.0 was retired. Replacement: 1.3.0. Upgrade with: a4 install stack ore@1.3.0"
        );
        assert_eq!(error.auth_code(), Some(AuthErrorCode::StackVersionRetired));
        assert_eq!(
            error.stack_version_refusal(),
            Some(&StackVersionRefusal {
                replacement_version: Some("1.3.0".to_string()),
                replacement_stack_manifest_hash: Some(
                    "arete:h1:stack-manifest:sha256:bb".to_string()
                ),
                upgrade_command: Some("a4 install stack ore@1.3.0".to_string()),
                retired_at: Some("2026-10-01T00:00:00Z".to_string()),
            })
        );
        assert!(error.is_stack_version_refusal());
        assert!(!error.should_retry());
        assert!(!error.should_refresh_token());
    }

    #[test]
    fn an_unknown_stack_version_keeps_the_server_message_without_guidance() {
        let error = AreteError::from_auth_response(
            409,
            None,
            Some(br#"{"error":"Stack version is not served here","code":"stack-version-unknown"}"#),
            Some("Conflict"),
        );

        assert!(matches!(
            &error,
            AreteError::AuthRequestFailed {
                status: 409,
                code: Some(AuthErrorCode::StackVersionUnknown),
                message,
                stack_version: Some(_),
            } if message == "Stack version is not served here"
        ));
        assert!(error.is_stack_version_refusal());
        assert!(!error.should_retry());
    }

    #[test]
    fn other_auth_failures_carry_no_stack_version_refusal() {
        let error = AreteError::from_auth_response(
            403,
            Some("origin-required"),
            Some(br#"{"error":"Origin required","code":"origin-required"}"#),
            None,
        );

        assert_eq!(error.stack_version_refusal(), None);
        assert!(!error.is_stack_version_refusal());
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
    fn a_bare_refusal_close_reason_is_a_terminal_code() {
        for (reason, expected) in [
            ("stack-version-retired", AuthErrorCode::StackVersionRetired),
            (
                " stack-version-unknown ",
                AuthErrorCode::StackVersionUnknown,
            ),
            ("stack-version-retired:", AuthErrorCode::StackVersionRetired),
        ] {
            let error = AreteError::from_close_reason(reason).expect("close reason should parse");
            assert_eq!(error.auth_code(), Some(expected), "{reason:?}");
            assert!(error.is_stack_version_refusal(), "{reason:?}");
            assert!(!error.should_retry(), "{reason:?}");
        }
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
            protocol_version: 2,
            subscription_id: Some("rounds:latest".to_string()),
            error: "subscription-limit-exceeded".to_string(),
            message: "subscription limit exceeded".to_string(),
            wire_code: "subscription-limit-exceeded".to_string(),
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
