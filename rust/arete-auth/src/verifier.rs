use crate::claims::AuthContext;
use crate::error::VerifyError;
use crate::keys::VerifyingKey;
use crate::token::{JwksVerifier, TokenVerifier};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// Cached JWKS with expiration
#[derive(Clone)]
struct CachedJwks {
    verifier: JwksVerifier,
    fetched_at: Instant,
}

/// Async verifier with JWKS caching support
pub struct AsyncVerifier {
    inner: VerifierInner,
    jwks_url: Option<String>,
    cache_duration: Duration,
    cached_jwks: Arc<RwLock<Option<CachedJwks>>>,
    /// Issuer for JWKS-based verification
    issuer: String,
    /// Audience for JWKS-based verification
    audience: String,
    require_origin: bool,
}

enum VerifierInner {
    Static(TokenVerifier),
    Jwks(JwksVerifier),
}

impl AsyncVerifier {
    /// Create a verifier with a static key
    pub fn with_static_key(
        key: VerifyingKey,
        issuer: impl Into<String>,
        audience: impl Into<String>,
    ) -> Self {
        let issuer_str = issuer.into();
        let audience_str = audience.into();
        Self {
            inner: VerifierInner::Static(TokenVerifier::new(
                key,
                issuer_str.clone(),
                audience_str.clone(),
            )),
            jwks_url: None,
            cache_duration: Duration::from_secs(3600), // 1 hour default
            cached_jwks: Arc::new(RwLock::new(None)),
            issuer: issuer_str,
            audience: audience_str,
            require_origin: false,
        }
    }

    /// Create a verifier with JWKS
    pub fn with_jwks(
        jwks: crate::token::Jwks,
        issuer: impl Into<String>,
        audience: impl Into<String>,
    ) -> Self {
        let issuer_str = issuer.into();
        let audience_str = audience.into();
        Self {
            inner: VerifierInner::Jwks(JwksVerifier::new(
                jwks,
                issuer_str.clone(),
                audience_str.clone(),
            )),
            jwks_url: None,
            cache_duration: Duration::from_secs(3600),
            cached_jwks: Arc::new(RwLock::new(None)),
            issuer: issuer_str,
            audience: audience_str,
            require_origin: false,
        }
    }

    /// Create a verifier that fetches JWKS from a URL
    #[cfg(feature = "jwks")]
    pub fn with_jwks_url(
        url: impl Into<String>,
        issuer: impl Into<String>,
        audience: impl Into<String>,
    ) -> Self {
        let issuer_str = issuer.into();
        let audience_str = audience.into();
        Self {
            inner: VerifierInner::Static(TokenVerifier::new(
                VerifyingKey::from_bytes(&[0u8; 32]).expect("zero key should be valid"),
                issuer_str.clone(),
                audience_str.clone(),
            )),
            jwks_url: Some(url.into()),
            issuer: issuer_str,
            audience: audience_str,
            cache_duration: Duration::from_secs(3600),
            cached_jwks: Arc::new(RwLock::new(None)),
            require_origin: false,
        }
    }

    /// Require origin validation on verified tokens.
    pub fn with_origin_validation(mut self) -> Self {
        self.require_origin = true;
        self.inner = match self.inner {
            VerifierInner::Static(verifier) => {
                VerifierInner::Static(verifier.with_origin_validation())
            }
            VerifierInner::Jwks(verifier) => VerifierInner::Jwks(verifier.with_origin_validation()),
        };
        self
    }

    /// Set cache duration for JWKS
    pub fn with_cache_duration(mut self, duration: Duration) -> Self {
        self.cache_duration = duration;
        self
    }

    /// Verify a token with automatic JWKS fetching and caching
    #[cfg(feature = "jwks")]
    pub async fn verify(
        &self,
        token: &str,
        expected_origin: Option<&str>,
        expected_client_ip: Option<&str>,
    ) -> Result<AuthContext, VerifyError> {
        // If using static JWKS or static key, use directly
        match &self.inner {
            VerifierInner::Static(verifier) => {
                verifier.verify(token, expected_origin, expected_client_ip)
            }
            VerifierInner::Jwks(verifier) => {
                verifier.verify(token, expected_origin, expected_client_ip)
            }
        }
    }

    /// Verify a token (non-JWKS version)
    #[cfg(not(feature = "jwks"))]
    pub fn verify(
        &self,
        token: &str,
        expected_origin: Option<&str>,
        expected_client_ip: Option<&str>,
    ) -> Result<AuthContext, VerifyError> {
        match &self.inner {
            VerifierInner::Static(verifier) => {
                verifier.verify(token, expected_origin, expected_client_ip)
            }
            VerifierInner::Jwks(verifier) => {
                verifier.verify(token, expected_origin, expected_client_ip)
            }
        }
    }

    /// Refresh JWKS cache from the configured URL
    #[cfg(feature = "jwks")]
    pub async fn refresh_cache(&self) -> Result<(), VerifyError> {
        if let Some(ref jwks_url) = self.jwks_url {
            // Fetch JWKS from URL
            let jwks = crate::token::JwksVerifier::fetch_jwks(jwks_url)
                .await
                .map_err(|e| VerifyError::InvalidFormat(format!("Failed to fetch JWKS: {}", e)))?;

            // Create new verifier with fetched JWKS
            let verifier = if self.require_origin {
                JwksVerifier::new(jwks, &self.issuer, &self.audience).with_origin_validation()
            } else {
                JwksVerifier::new(jwks, &self.issuer, &self.audience)
            };

            // Update cache
            let mut cached = self.cached_jwks.write().await;
            *cached = Some(CachedJwks {
                verifier,
                fetched_at: Instant::now(),
            });
        }
        Ok(())
    }

    /// Get cached verifier if available and not expired
    async fn get_cached_verifier(&self) -> Option<JwksVerifier> {
        let cached = self.cached_jwks.read().await;
        if let Some(ref cached_jwks) = *cached {
            if cached_jwks.fetched_at.elapsed() < self.cache_duration {
                return Some(cached_jwks.verifier.clone());
            }
        }
        None
    }

    /// Verify a token with automatic JWKS caching
    #[cfg(feature = "jwks")]
    pub async fn verify_with_cache(
        &self,
        token: &str,
        expected_origin: Option<&str>,
        expected_client_ip: Option<&str>,
    ) -> Result<AuthContext, VerifyError> {
        // Try cached verifier first
        if let Some(verifier) = self.get_cached_verifier().await {
            match verifier.verify(token, expected_origin, expected_client_ip) {
                Ok(ctx) => return Ok(ctx),
                Err(VerifyError::KeyNotFound(_)) => {
                    // Key not found in cache, refresh and retry
                }
                Err(e) => return Err(e),
            }
        }

        // Refresh cache and try again
        self.refresh_cache().await?;

        if let Some(verifier) = self.get_cached_verifier().await {
            verifier.verify(token, expected_origin, expected_client_ip)
        } else if self.jwks_url.is_some() {
            Err(VerifyError::InvalidFormat(
                "JWKS cache unavailable after refresh".to_string(),
            ))
        } else {
            // Fallback to inner verifier if no cache available
            match &self.inner {
                VerifierInner::Static(verifier) => {
                    verifier.verify(token, expected_origin, expected_client_ip)
                }
                VerifierInner::Jwks(verifier) => {
                    verifier.verify(token, expected_origin, expected_client_ip)
                }
            }
        }
    }
}

/// Simple synchronous verifier for use in non-async contexts
pub struct SimpleVerifier {
    inner: TokenVerifier,
}

impl SimpleVerifier {
    /// Create a new simple verifier
    pub fn new(key: VerifyingKey, issuer: impl Into<String>, audience: impl Into<String>) -> Self {
        Self {
            inner: TokenVerifier::new(key, issuer, audience),
        }
    }

    /// Verify a token synchronously
    pub fn verify(
        &self,
        token: &str,
        expected_origin: Option<&str>,
        expected_client_ip: Option<&str>,
    ) -> Result<AuthContext, VerifyError> {
        self.inner
            .verify(token, expected_origin, expected_client_ip)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claims::{KeyClass, SessionClaims};
    use crate::keys::SigningKey;
    use crate::token::TokenSigner;
    use base64::Engine;

    #[cfg(feature = "jwks")]
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn test_async_verifier_with_static_key() {
        let signing_key = SigningKey::generate();
        let verifying_key = signing_key.verifying_key();

        let signer = TokenSigner::new(signing_key, "test-issuer");
        let verifier =
            AsyncVerifier::with_static_key(verifying_key, "test-issuer", "test-audience");

        let claims = SessionClaims::builder("test-issuer", "test-subject", "test-audience")
            .with_scope("read")
            .with_metering_key("meter-123")
            .with_key_class(KeyClass::Publishable)
            .build();

        let token = signer.sign(claims).unwrap();
        let context = verifier.verify(&token, None, None).await.unwrap();

        assert_eq!(context.subject, "test-subject");
    }

    #[test]
    fn test_simple_verifier() {
        let signing_key = SigningKey::generate();
        let verifying_key = signing_key.verifying_key();

        let signer = TokenSigner::new(signing_key, "test-issuer");
        let verifier = SimpleVerifier::new(verifying_key, "test-issuer", "test-audience");

        let claims = SessionClaims::builder("test-issuer", "test-subject", "test-audience")
            .with_scope("read")
            .with_metering_key("meter-123")
            .with_key_class(KeyClass::Publishable)
            .build();

        let token = signer.sign(claims).unwrap();
        let context = verifier.verify(&token, None, None).unwrap();

        assert_eq!(context.subject, "test-subject");
        assert_eq!(context.metering_key, "meter-123");
    }

    #[cfg(feature = "jwks")]
    #[test]
    fn test_verify_with_cache_returns_explicit_error_when_cache_stays_empty() {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let signing_key = SigningKey::generate();
            let verifying_key = signing_key.verifying_key();
            let signer = TokenSigner::new(signing_key, "test-issuer");

            let jwks = serde_json::json!({
                "keys": [{
                    "kty": "OKP",
                    "use": "sig",
                    "kid": verifying_key.key_id(),
                    "x": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(verifying_key.to_bytes()),
                }]
            })
            .to_string();

            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let response_body = jwks.clone();
            tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buffer = [0u8; 1024];
                let _ = socket.read(&mut buffer).await;

                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    response_body.len(),
                    response_body
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            });

            let verifier = AsyncVerifier::with_jwks_url(
                format!("http://{addr}/jwks"),
                "test-issuer",
                "test-audience",
            )
            .with_cache_duration(Duration::ZERO);

            let claims = SessionClaims::builder("test-issuer", "test-subject", "test-audience")
                .with_scope("read")
                .with_metering_key("meter-123")
                .with_key_class(KeyClass::Publishable)
                .build();
            let token = signer.sign(claims).unwrap();

            let result = verifier.verify_with_cache(&token, None, None).await;
            assert!(matches!(
                result,
                Err(VerifyError::InvalidFormat(ref msg)) if msg == "JWKS cache unavailable after refresh"
            ));
        });
    }
}
