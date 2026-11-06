use crate::claims::AuthContext;
use crate::error::VerifyError;
use crate::keys::VerifyingKey;
use crate::sealed::ClaimSealKeys;
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
    audiences: crate::AudienceSet,
    require_origin: bool,
    claim_seal_keys: Option<ClaimSealKeys>,
}

enum VerifierInner {
    Static(TokenVerifier),
    Jwks(JwksVerifier),
    /// Keys are fetched from `jwks_url`; there is no local key to fall back to.
    #[cfg(feature = "jwks")]
    Remote,
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
            audiences: crate::AudienceSet::single(audience_str),
            require_origin: false,
            claim_seal_keys: None,
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
            audiences: crate::AudienceSet::single(audience_str),
            require_origin: false,
            claim_seal_keys: None,
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
            inner: VerifierInner::Remote,
            jwks_url: Some(url.into()),
            issuer: issuer.into(),
            audiences: crate::AudienceSet::single(audience.into()),
            cache_duration: Duration::from_secs(3600),
            cached_jwks: Arc::new(RwLock::new(None)),
            require_origin: false,
            claim_seal_keys: None,
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
            #[cfg(feature = "jwks")]
            VerifierInner::Remote => VerifierInner::Remote,
        };
        self.cached_jwks = Arc::new(RwLock::new(None));
        self
    }

    /// Open sealed claims with these keys. Without them, a token that carries
    /// sealed claims is refused.
    pub fn with_claim_seal_keys(mut self, keys: ClaimSealKeys) -> Self {
        self.inner = match self.inner {
            VerifierInner::Static(verifier) => {
                VerifierInner::Static(verifier.with_claim_seal_keys(keys.clone()))
            }
            VerifierInner::Jwks(verifier) => {
                VerifierInner::Jwks(verifier.with_claim_seal_keys(keys.clone()))
            }
            #[cfg(feature = "jwks")]
            VerifierInner::Remote => VerifierInner::Remote,
        };
        self.claim_seal_keys = Some(keys);
        // A verifier cached before this call would still open with the old keys.
        self.cached_jwks = Arc::new(RwLock::new(None));
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
        match &self.inner {
            VerifierInner::Static(verifier) => {
                verifier.verify(token, expected_origin, expected_client_ip)
            }
            VerifierInner::Jwks(verifier) => {
                verifier.verify(token, expected_origin, expected_client_ip)
            }
            // A URL-backed verifier only trusts keys fetched from its JWKS URL.
            VerifierInner::Remote => {
                self.verify_with_cache(token, expected_origin, expected_client_ip)
                    .await
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
            let verifier =
                JwksVerifier::with_audience_set(jwks, &self.issuer, self.audiences.clone());
            let verifier = if self.require_origin {
                verifier.with_origin_validation()
            } else {
                verifier
            };
            let verifier = match &self.claim_seal_keys {
                Some(keys) => verifier.with_claim_seal_keys(keys.clone()),
                None => verifier,
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
            return verifier.verify(token, expected_origin, expected_client_ip);
        }

        // No cache: verifiers with a local key fall back to it; a URL-backed
        // verifier has none and must fail closed.
        match &self.inner {
            VerifierInner::Static(verifier) => {
                verifier.verify(token, expected_origin, expected_client_ip)
            }
            VerifierInner::Jwks(verifier) => {
                verifier.verify(token, expected_origin, expected_client_ip)
            }
            VerifierInner::Remote => Err(VerifyError::InvalidFormat(
                "JWKS cache unavailable after refresh".to_string(),
            )),
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

    fn b64(bytes: impl AsRef<[u8]>) -> String {
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
    }

    fn jwks_json(keys: &[&VerifyingKey]) -> String {
        let keys: Vec<_> = keys
            .iter()
            .map(|key| {
                serde_json::json!({
                    "kty": "OKP",
                    "use": "sig",
                    "kid": key.key_id(),
                    "x": b64(key.to_bytes()),
                })
            })
            .collect();
        serde_json::json!({ "keys": keys }).to_string()
    }

    /// Serve `body` as JSON to every connection until the test ends.
    #[cfg(feature = "jwks")]
    async fn serve_jwks(body: String) -> std::net::SocketAddr {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let body = body.clone();
                tokio::spawn(async move {
                    let mut buffer = [0u8; 1024];
                    let _ = socket.read(&mut buffer).await;
                    let response = format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                });
            }
        });
        addr
    }

    fn signed_token(signing_key: SigningKey) -> String {
        let claims = SessionClaims::builder("test-issuer", "test-subject", "test-audience")
            .with_scope("read")
            .with_metering_key("meter-123")
            .with_key_class(KeyClass::Publishable)
            .build();
        TokenSigner::new(signing_key, "test-issuer")
            .sign(claims)
            .unwrap()
    }

    /// A token "signed" for the all-zero public key, which is a small-order
    /// point: with R = identity and s = 0 the lax Ed25519 equation holds for
    /// about one message in four, so no private key is needed.
    fn forge_low_order_token(kid: Option<&str>) -> String {
        use ed25519_dalek::Verifier as _;

        let zero_key = ed25519_dalek::VerifyingKey::from_bytes(&[0u8; 32]).unwrap();
        let mut signature_bytes = [0u8; 64];
        signature_bytes[0] = 1; // compressed identity point; s stays 0
        let signature = ed25519_dalek::Signature::from_bytes(&signature_bytes);

        let mut header = serde_json::json!({ "alg": "EdDSA", "typ": "JWT" });
        if let Some(kid) = kid {
            header["kid"] = kid.into();
        }
        let header_b64 = b64(header.to_string());

        for attempt in 0..512 {
            let claims = SessionClaims::builder(
                "test-issuer",
                format!("forged-subject-{attempt}"),
                "test-audience",
            )
            .with_scope("admin")
            .with_metering_key("forged")
            .with_key_class(KeyClass::Secret)
            .build();
            let message = format!("{header_b64}.{}", b64(serde_json::to_vec(&claims).unwrap()));
            if zero_key.verify(message.as_bytes(), &signature).is_ok() {
                return format!("{message}.{}", b64(signature_bytes));
            }
        }
        panic!("no low-order forgery found");
    }

    fn zero_key() -> VerifyingKey {
        VerifyingKey::from_bytes(&[0u8; 32]).unwrap()
    }

    #[cfg(feature = "jwks")]
    #[tokio::test]
    async fn jwks_url_verify_uses_the_fetched_keys() {
        let signing_key = SigningKey::generate();
        let addr = serve_jwks(jwks_json(&[&signing_key.verifying_key()])).await;
        let verifier = AsyncVerifier::with_jwks_url(
            format!("http://{addr}/jwks"),
            "test-issuer",
            "test-audience",
        );

        let token = signed_token(signing_key);
        let context = verifier.verify(&token, None, None).await.unwrap();
        assert_eq!(context.subject, "test-subject");
        let context = verifier
            .verify_with_cache(&token, None, None)
            .await
            .unwrap();
        assert_eq!(context.subject, "test-subject");
    }

    #[cfg(feature = "jwks")]
    #[tokio::test]
    async fn jwks_url_verify_fails_closed_when_the_jwks_is_unreachable() {
        // Bind then drop, so nothing is listening on the port.
        let addr = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap()
            .local_addr()
            .unwrap();
        let verifier = AsyncVerifier::with_jwks_url(
            format!("http://{addr}/jwks"),
            "test-issuer",
            "test-audience",
        );

        for token in [
            signed_token(SigningKey::generate()),
            forge_low_order_token(None),
        ] {
            assert!(verifier.verify(&token, None, None).await.is_err());
            assert!(verifier
                .verify_with_cache(&token, None, None)
                .await
                .is_err());
        }
    }

    #[cfg(feature = "jwks")]
    #[tokio::test]
    async fn jwks_url_verify_rejects_unknown_keys_and_low_order_forgeries() {
        let trusted = SigningKey::generate();
        let trusted_key = trusted.verifying_key();
        let addr = serve_jwks(jwks_json(&[&trusted_key])).await;
        let verifier = AsyncVerifier::with_jwks_url(
            format!("http://{addr}/jwks"),
            "test-issuer",
            "test-audience",
        );

        let untrusted = signed_token(SigningKey::generate());
        let forged = forge_low_order_token(None);
        let forged_with_trusted_kid = forge_low_order_token(Some(&trusted_key.key_id()));

        for token in [&untrusted, &forged, &forged_with_trusted_kid] {
            assert!(verifier.verify(token, None, None).await.is_err());
            assert!(verifier.verify_with_cache(token, None, None).await.is_err());
        }
        assert!(matches!(
            verifier.verify(&forged_with_trusted_kid, None, None).await,
            Err(VerifyError::InvalidSignature)
        ));
    }

    #[tokio::test]
    async fn low_order_forgeries_are_rejected_by_every_verifier() {
        let zero = zero_key();
        let forged = forge_low_order_token(None);
        let forged_with_kid = forge_low_order_token(Some(&zero.key_id()));

        for token in [&forged, &forged_with_kid] {
            assert!(matches!(
                TokenVerifier::new(zero.clone(), "test-issuer", "test-audience")
                    .verify(token, None, None),
                Err(VerifyError::InvalidSignature)
            ));
            assert!(matches!(
                SimpleVerifier::new(zero.clone(), "test-issuer", "test-audience")
                    .verify(token, None, None),
                Err(VerifyError::InvalidSignature)
            ));

            let static_verifier =
                AsyncVerifier::with_static_key(zero.clone(), "test-issuer", "test-audience");
            assert!(static_verifier.verify(token, None, None).await.is_err());
        }

        let jwks: crate::token::Jwks = serde_json::from_str(&jwks_json(&[&zero])).unwrap();
        assert!(matches!(
            JwksVerifier::new(jwks.clone(), "test-issuer", "test-audience").verify(
                &forged_with_kid,
                None,
                None
            ),
            Err(VerifyError::InvalidSignature)
        ));
        let jwks_verifier = AsyncVerifier::with_jwks(jwks, "test-issuer", "test-audience");
        assert!(matches!(
            jwks_verifier.verify(&forged_with_kid, None, None).await,
            Err(VerifyError::InvalidSignature)
        ));
    }
}
