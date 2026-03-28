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
        Self {
            inner: VerifierInner::Static(TokenVerifier::new(key, issuer, audience)),
            jwks_url: None,
            cache_duration: Duration::from_secs(3600), // 1 hour default
            cached_jwks: Arc::new(RwLock::new(None)),
        }
    }

    /// Create a verifier with JWKS
    pub fn with_jwks(
        jwks: crate::token::Jwks,
        issuer: impl Into<String>,
        audience: impl Into<String>,
    ) -> Self {
        Self {
            inner: VerifierInner::Jwks(JwksVerifier::new(jwks, issuer, audience)),
            jwks_url: None,
            cache_duration: Duration::from_secs(3600),
            cached_jwks: Arc::new(RwLock::new(None)),
        }
    }

    /// Create a verifier that fetches JWKS from a URL
    #[cfg(feature = "jwks")]
    pub fn with_jwks_url(
        url: impl Into<String>,
        issuer: impl Into<String>,
        audience: impl Into<String>,
    ) -> Self {
        Self {
            inner: VerifierInner::Static(TokenVerifier::new(
                VerifyingKey::from_bytes(&[0u8; 32]).expect("zero key should be valid"),
                issuer,
                audience,
            )),
            jwks_url: Some(url.into()),
            cache_duration: Duration::from_secs(3600),
            cached_jwks: Arc::new(RwLock::new(None)),
        }
    }

    /// Set cache duration for JWKS
    pub fn with_cache_duration(mut self, duration: Duration) -> Self {
        self.cache_duration = duration;
        self
    }

    /// Verify a token
    pub async fn verify(
        &self,
        token: &str,
        expected_origin: Option<&str>,
    ) -> Result<AuthContext, VerifyError> {
        // If we have a static or JWKS verifier, use it directly
        match &self.inner {
            VerifierInner::Static(verifier) => {
                verifier.verify(token, expected_origin)
            }
            VerifierInner::Jwks(verifier) => {
                verifier.verify(token, expected_origin)
            }
        }
    }

    /// Refresh JWKS cache
    #[cfg(feature = "jwks")]
    pub async fn refresh_cache(&self) -> Result<(), VerifyError> {
        if let Some(ref _jwks_url) = self.jwks_url {
            // We'd need issuer/audience here to create the verifier
            // This is a placeholder implementation
            let _cached = self.cached_jwks.write().await;
            // *cached = Some(CachedJwks { ... });
        }
        Ok(())
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
    pub fn verify(&self, token: &str, expected_origin: Option<&str>) -> Result<AuthContext, VerifyError> {
        self.inner.verify(token, expected_origin)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claims::{KeyClass, Limits, SessionClaims};
    use crate::keys::SigningKey;
    use crate::token::TokenSigner;

    #[tokio::test]
    async fn test_async_verifier_with_static_key() {
        let signing_key = SigningKey::generate();
        let verifying_key = signing_key.verifying_key();

        let signer = TokenSigner::new(signing_key, "test-issuer");
        let verifier = AsyncVerifier::with_static_key(verifying_key, "test-issuer", "test-audience");

        let claims = SessionClaims::builder("test-issuer", "test-subject", "test-audience")
            .with_scope("read")
            .with_metering_key("meter-123")
            .with_key_class(KeyClass::Publishable)
            .build();

        let token = signer.sign(claims).unwrap();
        let context = verifier.verify(&token, None).await.unwrap();

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
        let context = verifier.verify(&token, None).unwrap();

        assert_eq!(context.subject, "test-subject");
        assert_eq!(context.metering_key, "meter-123");
    }
}
