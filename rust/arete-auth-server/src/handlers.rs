use arete_auth::{KeyClass, Limits, SessionClaims};
use axum::{extract::State, Json};
use chrono::Utc;
use std::sync::Arc;

use crate::error::AuthServerError;
use crate::models::{HealthResponse, Jwk, JwksResponse, MintTokenRequest, MintTokenResponse};
use crate::server::AppState;

/// Extract Bearer token from Authorization header
fn extract_bearer_token(auth_header: Option<&str>) -> Option<&str> {
    auth_header.and_then(|header| header.strip_prefix("Bearer "))
}

fn extract_client_ip(headers: &axum::http::HeaderMap) -> Option<String> {
    headers
        .get("x-forwarded-for")
        .and_then(|h| h.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            headers
                .get("x-real-ip")
                .and_then(|h| h.to_str().ok())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
}

/// Extract deployment ID from websocket URL
/// Supports formats like:
/// - wss://demo.stack.arete.run -> "demo"
/// - wss://example.com/my-deployment -> "my-deployment"
fn extract_deployment_from_url(url_str: &str) -> Option<String> {
    // Try to parse as URL
    if let Ok(parsed) = url::Url::parse(url_str) {
        // First, try to extract from hostname (subdomain)
        if let Some(host) = parsed.host_str() {
            let host_lower: String = host.to_lowercase();

            // Extract subdomain before known suffixes
            // e.g., "demo.stack.arete.run" -> "demo"
            if let Some(first_dot) = host_lower.find('.') {
                let subdomain: &str = &host_lower[..first_dot];
                // Filter out common non-deployment subdomains
                if !subdomain.is_empty()
                    && subdomain != "www"
                    && subdomain != "api"
                    && subdomain != "auth"
                {
                    return Some(subdomain.to_string());
                }
            }
        }

        // Fallback: extract from path
        let path: &str = parsed.path().trim_start_matches('/');
        if !path.is_empty() && path != "/" {
            // Take the first path segment
            return path.split('/').next().map(|s: &str| s.to_string());
        }
    }

    None
}

/// Health check endpoint
pub async fn health(State(_state): State<Arc<AppState>>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "healthy".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

/// JWKS endpoint for token verification
pub async fn jwks(
    State(state): State<Arc<AppState>>,
) -> Result<Json<JwksResponse>, AuthServerError> {
    let public_key_bytes = state.verifying_key.to_bytes();
    let public_key_b64 = base64::Engine::encode(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD,
        public_key_bytes,
    );

    let jwk = Jwk {
        kty: "OKP".to_string(),
        kid: state.verifying_key.key_id(),
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

    let api_key = extract_bearer_token(auth_header).ok_or(AuthServerError::MissingApiKey)?;

    // Validate API key
    let key_info = state.key_store.validate_key(api_key)?;

    // Extract origin from request headers
    let origin_header = headers
        .get("origin")
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string());

    // Check rate limits (if enabled)
    if let Some(rate_limiter) = state.rate_limiter.as_ref() {
        let client_ip = extract_client_ip(&headers).unwrap_or_else(|| "unknown".to_string());

        let rate_limit_key = format!(
            "{}:{}:{}",
            key_info.key_id,
            client_ip,
            origin_header.as_deref().unwrap_or("none")
        );

        // Simple in-memory rate limiting - check against configured limit
        // This is a placeholder for a proper distributed rate limiter
        // In production, use Redis or similar
        check_rate_limit(
            rate_limiter,
            &rate_limit_key,
            key_info
                .rate_limit_tier
                .requests_per_minute()
                .min(state.config.rate_limit_per_minute),
        )?;
    }

    // Validate origin for publishable keys
    state
        .key_store
        .authorize_origin(&key_info, origin_header.as_deref())?;

    // Parse websocket_url to extract deployment_id/audience
    // Format: wss://{deployment_id}.{domain} or wss://{domain}/{deployment_id}
    let deployment_id = if let Some(explicit_id) = request.deployment_id {
        explicit_id
    } else {
        // Parse URL to extract deployment identifier
        match extract_deployment_from_url(&request.websocket_url) {
            Some(id) => id,
            None => state.config.default_audience.clone(),
        }
    };

    state
        .key_store
        .authorize_deployment(&key_info, &deployment_id)?;

    // Determine TTL (capped by key class)
    let requested_ttl = request
        .ttl_seconds
        .unwrap_or(state.config.default_ttl_seconds);
    let max_ttl = match key_info.key_class {
        KeyClass::Secret => 3600,     // 1 hour for secret keys
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

    let mut claims = SessionClaims::builder(
        state.config.issuer.clone(),
        key_info.subject.clone(),
        deployment_id.clone(),
    )
    .with_ttl(ttl)
    .with_scope(request.scope.unwrap_or_else(|| "read".to_string()))
    .with_metering_key(key_info.metering_key.clone())
    .with_deployment_id(deployment_id)
    .with_limits(limits)
    .with_key_class(key_info.key_class);

    // Use origin header if not explicitly provided in request
    let token_origin = request.origin.or(origin_header);

    if let Some(origin) = token_origin {
        claims = claims.with_origin(origin);
    }

    let claims = claims.build();

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

/// Simple in-memory rate limiting check
///
/// Note: This is a placeholder implementation. In production, use a distributed
/// rate limiter like Redis or a proper rate limiting service.
fn check_rate_limit(
    rate_limiter: &crate::rate_limiter::MintRateLimiter,
    key: &str,
    limit: u32,
) -> Result<(), AuthServerError> {
    if rate_limiter.check(key, limit) {
        Ok(())
    } else {
        Err(AuthServerError::RateLimitExceeded)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, HeaderValue};

    #[test]
    fn extract_client_ip_uses_leftmost_forwarded_ip() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("198.51.100.7, 10.0.0.1, 10.0.0.2"),
        );

        assert_eq!(extract_client_ip(&headers).as_deref(), Some("198.51.100.7"));
    }

    #[test]
    fn extract_client_ip_falls_back_to_x_real_ip() {
        let mut headers = HeaderMap::new();
        headers.insert("x-real-ip", HeaderValue::from_static("203.0.113.9"));

        assert_eq!(extract_client_ip(&headers).as_deref(), Some("203.0.113.9"));
    }
}
