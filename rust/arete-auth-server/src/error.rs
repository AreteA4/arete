use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum AuthServerError {
    #[error("Invalid API key")]
    InvalidApiKey,

    #[error("Missing API key")]
    MissingApiKey,

    #[error("Key not authorized for this deployment")]
    UnauthorizedDeployment,

    #[error("Origin not allowed for this key")]
    OriginNotAllowed,

    #[error("Rate limit exceeded")]
    RateLimitExceeded,

    #[error("Invalid request: {0}")]
    #[allow(dead_code)]
    InvalidRequest(String),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Key generation failed: {0}")]
    #[allow(dead_code)]
    KeyGenerationFailed(String),
}

impl AuthServerError {
    /// Returns the error code as a kebab-case string for machine-readable responses
    pub fn error_code(&self) -> &'static str {
        match self {
            AuthServerError::InvalidApiKey => "invalid-api-key",
            AuthServerError::MissingApiKey => "missing-authorization-header",
            AuthServerError::UnauthorizedDeployment => "deployment-access-denied",
            AuthServerError::OriginNotAllowed => "origin-not-allowed",
            AuthServerError::RateLimitExceeded => "rate-limit-exceeded",
            AuthServerError::InvalidRequest(_) => "invalid-request",
            AuthServerError::Internal(_) => "internal-error",
            AuthServerError::KeyGenerationFailed(_) => "internal-error",
        }
    }
}

impl IntoResponse for AuthServerError {
    fn into_response(self) -> Response {
        let (status, error_message) = match &self {
            AuthServerError::InvalidApiKey => (StatusCode::UNAUTHORIZED, self.to_string()),
            AuthServerError::MissingApiKey => (StatusCode::UNAUTHORIZED, self.to_string()),
            AuthServerError::UnauthorizedDeployment => (StatusCode::FORBIDDEN, self.to_string()),
            AuthServerError::OriginNotAllowed => (StatusCode::FORBIDDEN, self.to_string()),
            AuthServerError::RateLimitExceeded => (StatusCode::TOO_MANY_REQUESTS, self.to_string()),
            AuthServerError::InvalidRequest(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            AuthServerError::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
            AuthServerError::KeyGenerationFailed(_) => {
                (StatusCode::INTERNAL_SERVER_ERROR, self.to_string())
            }
        };

        let body = Json(json!({
            "error": error_message,
            "code": self.error_code(),
        }));

        (status, body).into_response()
    }
}

impl From<anyhow::Error> for AuthServerError {
    fn from(err: anyhow::Error) -> Self {
        AuthServerError::Internal(err.to_string())
    }
}

impl From<std::io::Error> for AuthServerError {
    fn from(err: std::io::Error) -> Self {
        AuthServerError::Internal(err.to_string())
    }
}

impl From<arete_auth::AuthError> for AuthServerError {
    fn from(err: arete_auth::AuthError) -> Self {
        AuthServerError::Internal(err.to_string())
    }
}
