use thiserror::Error;

/// Machine-readable error codes for authentication failures
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AuthErrorCode {
    /// Missing authentication token
    TokenMissing,
    /// Token has expired
    TokenExpired,
    /// Invalid token signature
    TokenInvalidSignature,
    /// Invalid token format
    TokenInvalidFormat,
    /// Token issuer mismatch
    TokenInvalidIssuer,
    /// Token audience mismatch
    TokenInvalidAudience,
    /// Required claim missing from token
    TokenMissingClaim,
    /// Token key ID not found
    TokenKeyNotFound,
    /// Origin mismatch for token
    OriginMismatch,
    /// Origin is required but not provided
    OriginRequired,
    /// Rate limit exceeded (token minting)
    RateLimitExceeded,
    /// Connection limit exceeded for subject
    ConnectionLimitExceeded,
    /// Subscription limit exceeded
    SubscriptionLimitExceeded,
    /// Snapshot limit exceeded
    SnapshotLimitExceeded,
    /// Egress limit exceeded
    EgressLimitExceeded,
    /// Invalid static token
    InvalidStaticToken,
    /// Internal server error during auth
    InternalError,
}

impl AuthErrorCode {
    /// Returns the error code as a kebab-case string identifier
    pub fn as_str(&self) -> &'static str {
        match self {
            AuthErrorCode::TokenMissing => "token-missing",
            AuthErrorCode::TokenExpired => "token-expired",
            AuthErrorCode::TokenInvalidSignature => "token-invalid-signature",
            AuthErrorCode::TokenInvalidFormat => "token-invalid-format",
            AuthErrorCode::TokenInvalidIssuer => "token-invalid-issuer",
            AuthErrorCode::TokenInvalidAudience => "token-invalid-audience",
            AuthErrorCode::TokenMissingClaim => "token-missing-claim",
            AuthErrorCode::TokenKeyNotFound => "token-key-not-found",
            AuthErrorCode::OriginMismatch => "origin-mismatch",
            AuthErrorCode::OriginRequired => "origin-required",
            AuthErrorCode::RateLimitExceeded => "rate-limit-exceeded",
            AuthErrorCode::ConnectionLimitExceeded => "connection-limit-exceeded",
            AuthErrorCode::SubscriptionLimitExceeded => "subscription-limit-exceeded",
            AuthErrorCode::SnapshotLimitExceeded => "snapshot-limit-exceeded",
            AuthErrorCode::EgressLimitExceeded => "egress-limit-exceeded",
            AuthErrorCode::InvalidStaticToken => "invalid-static-token",
            AuthErrorCode::InternalError => "internal-error",
        }
    }

    /// Returns whether the client should retry with the same token
    pub fn should_retry(&self) -> bool {
        matches!(
            self,
            AuthErrorCode::RateLimitExceeded | AuthErrorCode::InternalError
        )
    }

    /// Returns whether the client should fetch a new token
    pub fn should_refresh_token(&self) -> bool {
        matches!(
            self,
            AuthErrorCode::TokenExpired
                | AuthErrorCode::TokenInvalidSignature
                | AuthErrorCode::TokenInvalidFormat
                | AuthErrorCode::TokenInvalidIssuer
                | AuthErrorCode::TokenInvalidAudience
                | AuthErrorCode::TokenKeyNotFound
        )
    }

    /// Returns the HTTP status code equivalent for this error
    pub fn http_status(&self) -> u16 {
        match self {
            AuthErrorCode::TokenMissing => 401,
            AuthErrorCode::TokenExpired => 401,
            AuthErrorCode::TokenInvalidSignature => 401,
            AuthErrorCode::TokenInvalidFormat => 400,
            AuthErrorCode::TokenInvalidIssuer => 401,
            AuthErrorCode::TokenInvalidAudience => 401,
            AuthErrorCode::TokenMissingClaim => 400,
            AuthErrorCode::TokenKeyNotFound => 401,
            AuthErrorCode::OriginMismatch => 403,
            AuthErrorCode::OriginRequired => 403,
            AuthErrorCode::RateLimitExceeded => 429,
            AuthErrorCode::ConnectionLimitExceeded => 429,
            AuthErrorCode::SubscriptionLimitExceeded => 429,
            AuthErrorCode::SnapshotLimitExceeded => 429,
            AuthErrorCode::EgressLimitExceeded => 429,
            AuthErrorCode::InvalidStaticToken => 401,
            AuthErrorCode::InternalError => 500,
        }
    }

    /// Returns the default retry policy for this error
    pub fn default_retry_policy(&self) -> RetryPolicy {
        use std::time::Duration;

        match self {
            // Token errors - refresh token and retry
            AuthErrorCode::TokenExpired
            | AuthErrorCode::TokenInvalidSignature
            | AuthErrorCode::TokenInvalidFormat
            | AuthErrorCode::TokenInvalidIssuer
            | AuthErrorCode::TokenInvalidAudience
            | AuthErrorCode::TokenKeyNotFound => RetryPolicy::RetryWithFreshToken,

            // Rate limits - retry after delay
            AuthErrorCode::RateLimitExceeded
            | AuthErrorCode::ConnectionLimitExceeded
            | AuthErrorCode::SubscriptionLimitExceeded
            | AuthErrorCode::SnapshotLimitExceeded
            | AuthErrorCode::EgressLimitExceeded => RetryPolicy::RetryWithBackoff {
                initial: Duration::from_secs(1),
                max: Duration::from_secs(60),
            },

            // Internal errors - retry with backoff
            AuthErrorCode::InternalError => RetryPolicy::RetryWithBackoff {
                initial: Duration::from_secs(1),
                max: Duration::from_secs(30),
            },

            // Everything else - don't retry
            _ => RetryPolicy::NoRetry,
        }
    }
}

/// Retry policy for authentication errors
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryPolicy {
    /// Do not retry this request
    NoRetry,
    /// Retry immediately (for transient errors)
    RetryImmediately,
    /// Retry after a specific duration
    RetryAfter(std::time::Duration),
    /// Retry with exponential backoff
    RetryWithBackoff {
        /// Initial backoff duration
        initial: std::time::Duration,
        /// Maximum backoff duration
        max: std::time::Duration,
    },
    /// Refresh the token before retrying
    RetryWithFreshToken,
}

impl std::fmt::Display for AuthErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Convert VerifyError to AuthErrorCode
impl From<&VerifyError> for AuthErrorCode {
    fn from(err: &VerifyError) -> Self {
        match err {
            VerifyError::Expired => AuthErrorCode::TokenExpired,
            VerifyError::NotYetValid => AuthErrorCode::TokenInvalidFormat,
            VerifyError::InvalidSignature => AuthErrorCode::TokenInvalidSignature,
            VerifyError::InvalidIssuer => AuthErrorCode::TokenInvalidIssuer,
            VerifyError::InvalidAudience => AuthErrorCode::TokenInvalidAudience,
            VerifyError::MissingClaim(_) => AuthErrorCode::TokenMissingClaim,
            VerifyError::OriginMismatch { .. } => AuthErrorCode::OriginMismatch,
            VerifyError::OriginRequired { .. } => AuthErrorCode::OriginRequired,
            VerifyError::DecodeError(_) => AuthErrorCode::TokenInvalidFormat,
            VerifyError::KeyNotFound(_) => AuthErrorCode::TokenKeyNotFound,
            VerifyError::InvalidFormat(_) => AuthErrorCode::TokenInvalidFormat,
            VerifyError::Revoked => AuthErrorCode::TokenExpired,
        }
    }
}

/// Authentication errors
#[derive(Debug, Error)]
pub enum AuthError {
    #[error("invalid key format: {0}")]
    InvalidKeyFormat(String),

    #[error("key loading failed: {0}")]
    KeyLoadingFailed(String),

    #[error("signing failed: {0}")]
    SigningFailed(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Token verification errors
#[derive(Debug, Error, Clone, PartialEq)]
pub enum VerifyError {
    #[error("token has expired")]
    Expired,

    #[error("token is not yet valid")]
    NotYetValid,

    #[error("invalid signature")]
    InvalidSignature,

    #[error("invalid issuer")]
    InvalidIssuer,

    #[error("invalid audience")]
    InvalidAudience,

    #[error("missing required claim: {0}")]
    MissingClaim(String),

    #[error("origin mismatch: expected {expected}, got {actual}")]
    OriginMismatch { expected: String, actual: String },

    #[error("origin header required but not provided (token is origin-bound to '{token_origin}')")]
    OriginRequired { token_origin: String },

    #[error("decode error: {0}")]
    DecodeError(String),

    #[error("key not found: {0}")]
    KeyNotFound(String),

    #[error("invalid token format: {0}")]
    InvalidFormat(String),

    #[error("token has been revoked")]
    Revoked,
}
