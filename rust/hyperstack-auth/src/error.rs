use thiserror::Error;

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

    #[error("invalid issuer: expected {expected}, got {actual}")]
    InvalidIssuer { expected: String, actual: String },

    #[error("invalid audience: expected {expected}, got {actual}")]
    InvalidAudience { expected: String, actual: String },

    #[error("missing required claim: {0}")]
    MissingClaim(String),

    #[error("origin mismatch: expected {expected}, got {actual}")]
    OriginMismatch { expected: String, actual: String },

    #[error("decode error: {0}")]
    DecodeError(String),

    #[error("key not found: {0}")]
    KeyNotFound(String),

    #[error("invalid token format: {0}")]
    InvalidFormat(String),
}
