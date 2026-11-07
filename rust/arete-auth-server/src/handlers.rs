use arete_auth::{KeyClass, Limits, SessionClaims};
use axum::{
    extract::{ConnectInfo, State},
    Json,
};
use chrono::Utc;
use std::net::SocketAddr;
use std::sync::Arc;

use crate::error::AuthServerError;
use crate::models::{
    ApiKeyInfo, HealthResponse, Jwk, JwksResponse, MintTokenRequest, MintTokenResponse,
};
use crate::server::AppState;

/// Extract Bearer token from Authorization header
fn extract_bearer_token(auth_header: Option<&str>) -> Option<&str> {
    auth_header.and_then(|header| header.strip_prefix("Bearer "))
}

fn forwarded_client_ip(headers: &axum::http::HeaderMap) -> Option<String> {
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

/// The client IP used for per-IP rate limiting: the forwarded IP when proxy
/// headers are trusted, otherwise the TCP peer.
fn client_ip(
    headers: &axum::http::HeaderMap,
    peer: SocketAddr,
    trust_proxy_headers: bool,
) -> String {
    trust_proxy_headers
        .then(|| forwarded_client_ip(headers))
        .flatten()
        .unwrap_or_else(|| peer.ip().to_string())
}

/// Charge one mint to the key's per-client-IP bucket and to its per-key
/// bucket. Neither bucket depends on caller-chosen values such as `Origin`,
/// so a caller cannot reset its limit by varying them.
fn enforce_mint_rate_limit(
    rate_limiter: &crate::rate_limiter::MintRateLimiter,
    config: &crate::config::Config,
    key_info: &ApiKeyInfo,
    client_ip: &str,
) -> Result<(), AuthServerError> {
    let per_ip_bucket = format!("ip:{}:{}", key_info.key_id, client_ip);
    let per_ip_limit = key_info
        .rate_limit_tier
        .requests_per_minute()
        .min(config.rate_limit_per_minute);
    let per_key_bucket = format!("key:{}", key_info.key_id);

    let mut buckets = vec![(per_ip_bucket.as_str(), per_ip_limit)];
    if config.rate_limit_per_key_per_minute > 0 {
        buckets.push((
            per_key_bucket.as_str(),
            config.rate_limit_per_key_per_minute,
        ));
    }

    if rate_limiter.check_all(&buckets) {
        Ok(())
    } else {
        Err(AuthServerError::RateLimitExceeded)
    }
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
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
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
        let client_ip = client_ip(&headers, peer, state.config.trust_proxy_headers);
        enforce_mint_rate_limit(rate_limiter, &state.config, &key_info, &client_ip)?;
    }

    // The token is bound to the requested origin, falling back to the
    // `Origin` header. With an allowlist, both must be allowed, so a caller
    // cannot pass the check with one origin and bind the token to another.
    let token_origin = request.origin.clone().or_else(|| origin_header.clone());
    state
        .key_store
        .authorize_origin(&key_info, token_origin.as_deref())?;
    if origin_header.is_some() {
        state
            .key_store
            .authorize_origin(&key_info, origin_header.as_deref())?;
    }

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
        max_http_requests_per_minute: Some(1200),
        max_http_batch_addresses: Some(100),
        ..Limits::default()
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, HeaderValue};

    fn peer() -> SocketAddr {
        "192.0.2.1:4000".parse().unwrap()
    }

    #[test]
    fn extract_client_ip_uses_leftmost_forwarded_ip() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("198.51.100.7, 10.0.0.1, 10.0.0.2"),
        );

        assert_eq!(client_ip(&headers, peer(), true), "198.51.100.7");
    }

    #[test]
    fn extract_client_ip_falls_back_to_x_real_ip() {
        let mut headers = HeaderMap::new();
        headers.insert("x-real-ip", HeaderValue::from_static("203.0.113.9"));

        assert_eq!(client_ip(&headers, peer(), true), "203.0.113.9");
    }

    #[test]
    fn client_ip_uses_the_peer_when_no_header_or_untrusted() {
        assert_eq!(client_ip(&HeaderMap::new(), peer(), true), "192.0.2.1");

        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", HeaderValue::from_static("198.51.100.7"));
        assert_eq!(client_ip(&headers, peer(), false), "192.0.2.1");
    }

    fn publishable_key() -> ApiKeyInfo {
        crate::keys::ApiKeyStore::new(vec![], vec!["pub123".to_string()])
            .validate_key("pub123")
            .unwrap()
    }

    #[test]
    fn per_ip_bucket_is_not_reset_by_other_request_values() {
        let limiter = crate::rate_limiter::MintRateLimiter::new(std::time::Duration::from_secs(60));
        let config = crate::config::Config {
            rate_limit_per_minute: 2,
            ..crate::config::Config::for_tests()
        };
        let key = publishable_key();

        assert!(enforce_mint_rate_limit(&limiter, &config, &key, "198.51.100.7").is_ok());
        assert!(enforce_mint_rate_limit(&limiter, &config, &key, "198.51.100.7").is_ok());
        assert!(matches!(
            enforce_mint_rate_limit(&limiter, &config, &key, "198.51.100.7"),
            Err(AuthServerError::RateLimitExceeded)
        ));
        // Another client IP has its own bucket.
        assert!(enforce_mint_rate_limit(&limiter, &config, &key, "203.0.113.9").is_ok());
    }

    #[test]
    fn per_key_bucket_caps_mints_across_client_ips() {
        let limiter = crate::rate_limiter::MintRateLimiter::new(std::time::Duration::from_secs(60));
        let config = crate::config::Config {
            rate_limit_per_minute: 60,
            rate_limit_per_key_per_minute: 3,
            ..crate::config::Config::for_tests()
        };
        let key = publishable_key();

        for ip in ["10.0.0.1", "10.0.0.2", "10.0.0.3"] {
            assert!(enforce_mint_rate_limit(&limiter, &config, &key, ip).is_ok());
        }
        assert!(matches!(
            enforce_mint_rate_limit(&limiter, &config, &key, "10.0.0.4"),
            Err(AuthServerError::RateLimitExceeded)
        ));
    }

    fn app_state(config: crate::config::Config) -> Arc<AppState> {
        let key_store = crate::keys::ApiKeyStore::new(
            config.secret_keys.clone(),
            config.publishable_keys.clone(),
        )
        .with_publishable_origin_allowlist(config.publishable_key_origins.clone());
        let signing_key = arete_auth::SigningKey::generate();
        let verifying_key = signing_key.verifying_key();
        Arc::new(AppState {
            rate_limiter: config.enable_rate_limit.then(|| {
                crate::rate_limiter::MintRateLimiter::new(std::time::Duration::from_secs(60))
            }),
            token_signer: arete_auth::TokenSigner::new(signing_key, config.issuer.clone()),
            verifying_key,
            key_store,
            config,
        })
    }

    async fn mint(
        state: &Arc<AppState>,
        header_origin: Option<&'static str>,
        body_origin: Option<&str>,
    ) -> Result<Json<MintTokenResponse>, AuthServerError> {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pub123"),
        );
        if let Some(origin) = header_origin {
            headers.insert("origin", HeaderValue::from_static(origin));
        }
        mint_token(
            State(state.clone()),
            ConnectInfo(peer()),
            headers,
            Json(MintTokenRequest {
                websocket_url: "wss://demo.stack.example.com".to_string(),
                deployment_id: None,
                scope: None,
                ttl_seconds: None,
                origin: body_origin.map(str::to_string),
            }),
        )
        .await
    }

    fn token_origin(response: &MintTokenResponse) -> Option<String> {
        use base64::Engine;
        let claims = response.token.split('.').nth(1).unwrap();
        let claims = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(claims)
            .unwrap();
        let claims: serde_json::Value = serde_json::from_slice(&claims).unwrap();
        claims["origin"].as_str().map(str::to_string)
    }

    #[tokio::test]
    async fn publishable_mint_is_restricted_to_allowed_origins() {
        let state = app_state(crate::config::Config {
            publishable_keys: vec!["pub123".to_string()],
            publishable_key_origins: vec!["https://app.example.com".to_string()],
            enable_rate_limit: false,
            ..crate::config::Config::for_tests()
        });

        let ok = mint(&state, Some("https://app.example.com"), None)
            .await
            .unwrap();
        assert_eq!(
            token_origin(&ok).as_deref(),
            Some("https://app.example.com")
        );

        for (header, body) in [
            (Some("https://evil.example"), None),
            (None, None),
            (None, Some("https://evil.example")),
            // The allowed header cannot vouch for a different bound origin.
            (
                Some("https://app.example.com"),
                Some("https://evil.example"),
            ),
        ] {
            assert!(matches!(
                mint(&state, header, body).await,
                Err(AuthServerError::OriginNotAllowed)
            ));
        }
    }

    #[tokio::test]
    async fn varying_the_origin_header_does_not_reset_the_mint_limit() {
        let state = app_state(crate::config::Config {
            publishable_keys: vec!["pub123".to_string()],
            rate_limit_per_minute: 2,
            ..crate::config::Config::for_tests()
        });

        assert!(mint(&state, Some("https://a.example"), None).await.is_ok());
        assert!(mint(&state, Some("https://b.example"), None).await.is_ok());
        assert!(matches!(
            mint(&state, Some("https://c.example"), None).await,
            Err(AuthServerError::RateLimitExceeded)
        ));
    }
}
