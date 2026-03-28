use axum::{
    Json,
    extract::State,
};
use chrono::Utc;
use hyperstack_auth::{KeyClass, Limits, SessionClaims};
use std::sync::Arc;

use crate::error::AuthServerError;
use crate::models::{
    HealthResponse, JwksResponse, Jwk, MintTokenRequest, MintTokenResponse,
};
use crate::server::AppState;

/// Extract Bearer token from Authorization header
fn extract_bearer_token(auth_header: Option<&str>) -> Option<&str> {
    auth_header
        .and_then(|header| header.strip_prefix("Bearer "))
}

/// Health check endpoint
pub async fn health(State(_state): State<Arc<AppState>>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "healthy".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

/// JWKS endpoint for token verification
pub async fn jwks(State(state): State<Arc<AppState>>) -> Result<Json<JwksResponse>, AuthServerError> {
    let public_key_bytes = state.verifying_key.to_bytes();
    let public_key_b64 = base64::Engine::encode(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD,
        public_key_bytes,
    );

    let jwk = Jwk {
        kty: "OKP".to_string(),
        kid: "key-1".to_string(),
        use_: "sig".to_string(),
        alg: "EdDSA".to_string(),
        x: public_key_b64,
    };

    Ok(Json(JwksResponse { keys: vec![jwk] }))
}

/// Mint a new session token
pub async fn mint_token(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Json(request): Json<MintTokenRequest>,
) -> Result<Json<MintTokenResponse>, AuthServerError> {
    // Extract API key from Authorization header
    let auth_header = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok());

    let api_key = extract_bearer_token(auth_header)
        .ok_or(AuthServerError::MissingApiKey)?;

    // Validate API key
    let key_info = state.key_store.validate_key(api_key)?;

    // Check deployment authorization for publishable keys
    let deployment_id = request
        .deployment_id
        .clone()
        .unwrap_or_else(|| state.config.default_audience.clone());

    state
        .key_store
        .authorize_deployment(&key_info, &deployment_id)?;

    // Determine TTL (capped by key class)
    let requested_ttl = request.ttl_seconds.unwrap_or(state.config.default_ttl_seconds);
    let max_ttl = match key_info.key_class {
        KeyClass::Secret => 3600,      // 1 hour for secret keys
        KeyClass::Publishable => 300, // 5 minutes for publishable keys
    };
    let ttl = requested_ttl.min(max_ttl);

    // Build claims
    let now = Utc::now().timestamp() as u64;
    let expires_at = now + ttl;

    let limits = Limits {
        max_connections: Some(state.config.max_connections_per_subject),
        max_subscriptions: Some(state.config.max_subscriptions_per_connection),
        max_snapshot_rows: Some(1000),
        max_messages_per_minute: Some(10000),
        max_bytes_per_minute: Some(100 * 1024 * 1024), // 100 MB
    };

    let claims = SessionClaims::builder(
        state.config.issuer.clone(),
        key_info.subject.clone(),
        deployment_id.clone(),
    )
    .with_ttl(ttl)
    .with_scope(request.scope.unwrap_or_else(|| "read".to_string()))
    .with_metering_key(key_info.metering_key.clone())
    .with_deployment_id(deployment_id)
    .with_limits(limits)
    .with_key_class(key_info.key_class)
    .with_jti(format!("{}-{}", key_info.key_id, now))
    .build();

    // Sign token
    let token = state
        .token_signer
        .sign(claims)
        .map_err(|e| AuthServerError::Internal(format!("Failed to sign token: {}", e)))?;

    Ok(Json(MintTokenResponse {
        token,
        expires_at,
        token_type: "Bearer".to_string(),
    }))
}
