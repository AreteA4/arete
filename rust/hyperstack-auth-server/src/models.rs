use serde::{Deserialize, Serialize};

/// Request to mint a new session token
#[derive(Debug, Deserialize)]
pub struct MintTokenRequest {
    /// Target deployment ID (optional, defaults to default_audience)
    pub deployment_id: Option<String>,
    /// Requested scope (optional, defaults to "read")
    pub scope: Option<String>,
    /// Requested TTL in seconds (optional, capped by server max)
    pub ttl_seconds: Option<u64>,
    /// Origin to bind the token to (optional)
    pub origin: Option<String>,
}

/// Response with minted session token
#[derive(Debug, Serialize)]
pub struct MintTokenResponse {
    /// The session token (JWT)
    pub token: String,
    /// Token expiration time (Unix timestamp)
    pub expires_at: u64,
    /// Token type
    pub token_type: String,
}

/// JWKS response
#[derive(Debug, Serialize)]
pub struct JwksResponse {
    pub keys: Vec<Jwk>,
}

/// JWK (JSON Web Key)
#[derive(Debug, Serialize)]
pub struct Jwk {
    /// Key type
    pub kty: String,
    /// Key ID
    pub kid: String,
    /// Key usage
    #[serde(rename = "use")]
    pub use_: String,
    /// Algorithm
    pub alg: String,
    /// Public key (base64url-encoded)
    pub x: String,
}

/// Health check response
#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
}

/// API key validation result
#[derive(Debug, Clone)]
pub struct ApiKeyInfo {
    /// Key identifier
    pub key_id: String,
    /// Key class (secret or publishable)
    pub key_class: hyperstack_auth::KeyClass,
    /// Associated subject
    pub subject: String,
    /// Associated metering key
    pub metering_key: String,
    /// Allowed deployments (None = all)
    pub allowed_deployments: Option<Vec<String>>,
    /// Rate limit tier
    pub rate_limit_tier: RateLimitTier,
}

#[derive(Debug, Clone)]
pub enum RateLimitTier {
    Low,
    Medium,
    High,
}

impl RateLimitTier {
    pub fn requests_per_minute(&self) -> u32 {
        match self {
            RateLimitTier::Low => 10,
            RateLimitTier::Medium => 60,
            RateLimitTier::High => 600,
        }
    }
}
