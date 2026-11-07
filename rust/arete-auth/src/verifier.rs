use crate::claims::AuthContext;
use crate::error::VerifyError;
use crate::keys::VerifyingKey;
use crate::sealed::ClaimSealKeys;
use crate::token::{JwksVerifier, TokenVerifier};
use std::sync::{Arc, PoisonError, RwLock};
use std::time::{Duration, Instant};

/// Default minimum time between two JWKS fetches, whatever triggers them.
/// See [`AsyncVerifier::with_min_refresh_interval`].
pub const DEFAULT_MIN_JWKS_REFRESH_INTERVAL: Duration = Duration::from_secs(30);

/// Upper bound on a single JWKS fetch.
#[cfg(feature = "jwks")]
const JWKS_FETCH_TIMEOUT: Duration = Duration::from_secs(10);

// The cache is only filled with the `jwks` feature.
/// Keys from the last successful JWKS fetch.
#[cfg_attr(not(feature = "jwks"), allow(dead_code))]
struct CachedJwks {
    verifier: Arc<JwksVerifier>,
    fetched_at: Instant,
}

#[derive(Default)]
struct JwksCacheState {
    /// Last keys fetched successfully. A failed fetch leaves them in place.
    current: Option<Arc<CachedJwks>>,
    /// End of the last fetch attempt, successful or not.
    last_attempt: Option<Instant>,
}

/// JWKS cache shared by every request on one verifier.
///
/// Fetches are single-flight, and a new one starts no sooner than the minimum
/// refresh interval after the previous one finished, whether it is triggered
/// by expiry, an unknown `kid`, or a previous failure. Requests never queue
/// behind a fetch unless there are no keys at all yet.
#[derive(Default)]
#[cfg_attr(not(feature = "jwks"), allow(dead_code))]
struct JwksCache {
    state: RwLock<JwksCacheState>,
    /// Held while a fetch is in flight.
    fetch: tokio::sync::Mutex<()>,
}

/// Records a fetch attempt when dropped, however the fetch ended.
#[cfg(feature = "jwks")]
struct MarkAttemptOnDrop<'a>(&'a JwksCache);

#[cfg(feature = "jwks")]
impl Drop for MarkAttemptOnDrop<'_> {
    fn drop(&mut self) {
        self.0.mark_attempt();
    }
}

#[cfg_attr(not(feature = "jwks"), allow(dead_code))]
impl JwksCache {
    fn current(&self) -> Option<Arc<CachedJwks>> {
        self.state
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .current
            .clone()
    }

    fn refresh_due(&self, min_interval: Duration) -> bool {
        self.state
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .last_attempt
            .is_none_or(|at| at.elapsed() >= min_interval)
    }

    fn mark_attempt(&self) {
        self.state
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .last_attempt = Some(Instant::now());
    }

    fn store(&self, verifier: JwksVerifier) {
        self.state
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .current = Some(Arc::new(CachedJwks {
            verifier: Arc::new(verifier),
            fetched_at: Instant::now(),
        }));
    }
}

/// Async verifier with JWKS caching support
pub struct AsyncVerifier {
    inner: VerifierInner,
    jwks_url: Option<String>,
    cache_duration: Duration,
    min_refresh_interval: Duration,
    jwks_cache: Arc<JwksCache>,
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
            min_refresh_interval: DEFAULT_MIN_JWKS_REFRESH_INTERVAL,
            jwks_cache: Arc::default(),
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
            min_refresh_interval: DEFAULT_MIN_JWKS_REFRESH_INTERVAL,
            jwks_cache: Arc::default(),
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
            min_refresh_interval: DEFAULT_MIN_JWKS_REFRESH_INTERVAL,
            jwks_cache: Arc::default(),
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
        self.jwks_cache = Arc::default();
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
        self.jwks_cache = Arc::default();
        self
    }

    /// Set how long fetched JWKS keys count as fresh. Once they are stale,
    /// requests try to refetch them (subject to the minimum refresh interval)
    /// and keep verifying against them until a fetch succeeds.
    pub fn with_cache_duration(mut self, duration: Duration) -> Self {
        self.cache_duration = duration;
        self
    }

    /// Set the minimum time from the end of one JWKS fetch to the start of
    /// the next, whatever triggers it: expiry, an unknown `kid`, or a failed
    /// fetch. A key published at the JWKS URL is picked up within about this
    /// interval. Defaults to [`DEFAULT_MIN_JWKS_REFRESH_INTERVAL`].
    pub fn with_min_refresh_interval(mut self, interval: Duration) -> Self {
        self.min_refresh_interval = interval;
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

    /// Fetch the JWKS from the configured URL now, ignoring the minimum
    /// refresh interval. Waits for a fetch already in flight. On failure the
    /// previously fetched keys stay in use.
    #[cfg(feature = "jwks")]
    pub async fn refresh_cache(&self) -> Result<(), VerifyError> {
        let Some(jwks_url) = self.jwks_url.as_deref() else {
            return Ok(());
        };
        let _fetch = self.jwks_cache.fetch.lock().await;
        self.fetch_and_store(jwks_url).await
    }

    /// Fetch the JWKS unless one finished within the minimum refresh interval
    /// or one is in flight. Failures are not returned: callers fall
    /// back to whatever keys are cached.
    #[cfg(feature = "jwks")]
    async fn refresh_if_due(&self) {
        let Some(jwks_url) = self.jwks_url.as_deref() else {
            return;
        };
        let _fetch = if self.jwks_cache.current().is_some() {
            if !self.jwks_cache.refresh_due(self.min_refresh_interval) {
                return;
            }
            // Another request is fetching: use the cached keys rather than
            // holding this request.
            match self.jwks_cache.fetch.try_lock() {
                Ok(guard) => guard,
                Err(_) => return,
            }
        } else {
            // No keys yet: wait for a fetch in flight instead of failing.
            self.jwks_cache.fetch.lock().await
        };
        if !self.jwks_cache.refresh_due(self.min_refresh_interval) {
            return;
        }
        if let Err(error) = self.fetch_and_store(jwks_url).await {
            tracing::warn!(%error, "JWKS refresh failed; keeping the cached keys");
        }
    }

    /// Fetch and cache the JWKS. The caller holds the fetch lock.
    #[cfg(feature = "jwks")]
    async fn fetch_and_store(&self, jwks_url: &str) -> Result<(), VerifyError> {
        // The interval runs from the end of a fetch, failed or not. Marking on
        // drop also covers a fetch cancelled with its request (for example by
        // a handshake deadline), which would otherwise let the next request
        // fetch at once.
        let _mark = MarkAttemptOnDrop(&self.jwks_cache);
        let jwks = tokio::time::timeout(
            JWKS_FETCH_TIMEOUT,
            crate::token::JwksVerifier::fetch_jwks(jwks_url),
        )
        .await
        .map_err(|_| VerifyError::InvalidFormat("Failed to fetch JWKS: timed out".to_string()))?
        .map_err(|e| VerifyError::InvalidFormat(format!("Failed to fetch JWKS: {}", e)))?;

        let verifier = JwksVerifier::with_audience_set(jwks, &self.issuer, self.audiences.clone());
        let verifier = if self.require_origin {
            verifier.with_origin_validation()
        } else {
            verifier
        };
        let verifier = match &self.claim_seal_keys {
            Some(keys) => verifier.with_claim_seal_keys(keys.clone()),
            None => verifier,
        };
        self.jwks_cache.store(verifier);
        Ok(())
    }

    /// Verify a token against the JWKS fetched from the configured URL.
    ///
    /// Fresh keys are used directly. An unknown `kid`, stale keys, or no keys
    /// trigger a refetch, but at most one fetch runs at a time and the next
    /// waits out the minimum refresh interval, so tokens with made-up key IDs
    /// cannot drive outbound requests. If the fetch fails or is skipped, the
    /// last successfully fetched keys are used.
    #[cfg(feature = "jwks")]
    pub async fn verify_with_cache(
        &self,
        token: &str,
        expected_origin: Option<&str>,
        expected_client_ip: Option<&str>,
    ) -> Result<AuthContext, VerifyError> {
        if let Some(cached) = self.jwks_cache.current() {
            if cached.fetched_at.elapsed() < self.cache_duration {
                match cached
                    .verifier
                    .verify(token, expected_origin, expected_client_ip)
                {
                    Err(VerifyError::KeyNotFound(_)) => {}
                    result => return result,
                }
            }
        }

        self.refresh_if_due().await;

        if let Some(cached) = self.jwks_cache.current() {
            return cached
                .verifier
                .verify(token, expected_origin, expected_client_ip);
        }

        // No keys fetched yet: verifiers with a local key fall back to it; a
        // URL-backed verifier has none and must fail closed.
        match &self.inner {
            VerifierInner::Static(verifier) => {
                verifier.verify(token, expected_origin, expected_client_ip)
            }
            VerifierInner::Jwks(verifier) => {
                verifier.verify(token, expected_origin, expected_client_ip)
            }
            VerifierInner::Remote => {
                Err(VerifyError::InvalidFormat("JWKS unavailable".to_string()))
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

    /// A JWKS endpoint that counts requests and can be switched to new keys
    /// or to failing with a 500.
    #[cfg(feature = "jwks")]
    struct CountingJwks {
        addr: std::net::SocketAddr,
        hits: Arc<std::sync::atomic::AtomicUsize>,
        body: Arc<std::sync::Mutex<Option<String>>>,
    }

    #[cfg(feature = "jwks")]
    impl CountingJwks {
        /// Serve `body`, answering each request after `delay`.
        async fn start(body: String, delay: Duration) -> Self {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let body = Arc::new(std::sync::Mutex::new(Some(body)));
            let (served_hits, served_body) = (hits.clone(), body.clone());
            tokio::spawn(async move {
                loop {
                    let (mut socket, _) = listener.accept().await.unwrap();
                    let (hits, body) = (served_hits.clone(), served_body.clone());
                    tokio::spawn(async move {
                        let mut buffer = [0u8; 1024];
                        let _ = socket.read(&mut buffer).await;
                        hits.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        tokio::time::sleep(delay).await;
                        let (status, body) = match body.lock().unwrap().clone() {
                            Some(body) => ("200 OK", body),
                            None => ("500 Internal Server Error", "unavailable".to_string()),
                        };
                        let response = format!(
                            "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                            body.len(),
                            body
                        );
                        let _ = socket.write_all(response.as_bytes()).await;
                    });
                }
            });
            Self { addr, hits, body }
        }

        fn url(&self) -> String {
            format!("http://{}/jwks", self.addr)
        }

        fn hits(&self) -> usize {
            self.hits.load(std::sync::atomic::Ordering::SeqCst)
        }

        fn serve(&self, body: Option<String>) {
            *self.body.lock().unwrap() = body;
        }
    }

    #[cfg(feature = "jwks")]
    fn copy_key(key: &SigningKey) -> SigningKey {
        SigningKey::from_bytes(&key.to_bytes())
    }

    #[cfg(feature = "jwks")]
    async fn verify_concurrently(
        verifier: &Arc<AsyncVerifier>,
        tokens: Vec<String>,
    ) -> Vec<Result<AuthContext, VerifyError>> {
        let mut tasks = tokio::task::JoinSet::new();
        for token in tokens {
            let verifier = verifier.clone();
            tasks.spawn(async move { verifier.verify(&token, None, None).await });
        }
        tasks.join_all().await
    }

    #[cfg(feature = "jwks")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn unknown_kids_trigger_at_most_one_fetch_per_interval() {
        let trusted = SigningKey::generate();
        let server = CountingJwks::start(
            jwks_json(&[&trusted.verifying_key()]),
            Duration::from_millis(50),
        )
        .await;
        let verifier = Arc::new(AsyncVerifier::with_jwks_url(
            server.url(),
            "test-issuer",
            "test-audience",
        ));
        let random_kid_tokens = || -> Vec<String> {
            (0..64)
                .map(|_| signed_token(SigningKey::generate()))
                .collect()
        };

        // Cold cache: every request misses, one fetch serves them all.
        let results = verify_concurrently(&verifier, random_kid_tokens()).await;
        assert!(results
            .iter()
            .all(|result| matches!(result, Err(VerifyError::KeyNotFound(_)))));
        assert_eq!(server.hits(), 1);

        // Warm cache: more unknown kids within the interval fetch nothing.
        let results = verify_concurrently(&verifier, random_kid_tokens()).await;
        assert!(results
            .iter()
            .all(|result| matches!(result, Err(VerifyError::KeyNotFound(_)))));
        assert_eq!(server.hits(), 1);

        let context = verifier
            .verify(&signed_token(trusted), None, None)
            .await
            .unwrap();
        assert_eq!(context.subject, "test-subject");
        assert_eq!(server.hits(), 1);
    }

    #[cfg(feature = "jwks")]
    async fn wait_for_hits(server: &CountingJwks, hits: usize) {
        tokio::time::timeout(Duration::from_secs(10), async {
            while server.hits() < hits {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("JWKS request never arrived");
    }

    #[cfg(feature = "jwks")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn cached_tokens_do_not_wait_behind_a_due_refresh() {
        let trusted = SigningKey::generate();
        let fetch_time = Duration::from_secs(2);
        let server = CountingJwks::start(jwks_json(&[&trusted.verifying_key()]), fetch_time).await;
        let interval = Duration::from_millis(100);
        // Keys go stale at once, so every request after the interval is due
        // to refresh.
        let verifier = Arc::new(
            AsyncVerifier::with_jwks_url(server.url(), "test-issuer", "test-audience")
                .with_cache_duration(Duration::ZERO)
                .with_min_refresh_interval(interval),
        );
        verifier.refresh_cache().await.unwrap();
        tokio::time::sleep(interval + Duration::from_millis(50)).await;

        // One request starts the due refresh and holds it open.
        let refreshing = {
            let verifier = verifier.clone();
            let token = signed_token(copy_key(&trusted));
            tokio::spawn(async move { verifier.verify(&token, None, None).await })
        };
        wait_for_hits(&server, 2).await;

        // Requests arriving meanwhile use the cached keys at once and start
        // no fetch of their own.
        let tokens = (0..16).map(|_| signed_token(copy_key(&trusted))).collect();
        let results = tokio::time::timeout(fetch_time / 2, verify_concurrently(&verifier, tokens))
            .await
            .expect("requests waited behind the in-flight fetch");
        assert!(results.iter().all(Result::is_ok));
        assert!(!refreshing.is_finished());
        assert_eq!(server.hits(), 2);

        refreshing.await.unwrap().unwrap();
        assert_eq!(server.hits(), 2);
    }

    #[cfg(feature = "jwks")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn cancelled_fetch_still_counts_against_the_interval() {
        let server = CountingJwks::start(
            jwks_json(&[&SigningKey::generate().verifying_key()]),
            Duration::from_secs(2),
        )
        .await;
        let verifier = Arc::new(AsyncVerifier::with_jwks_url(
            server.url(),
            "test-issuer",
            "test-audience",
        ));

        // A request is dropped mid-fetch, as a handshake deadline would.
        let cancelled = {
            let verifier = verifier.clone();
            let token = signed_token(SigningKey::generate());
            tokio::spawn(async move { verifier.verify(&token, None, None).await })
        };
        wait_for_hits(&server, 1).await;
        cancelled.abort();
        assert!(cancelled.await.unwrap_err().is_cancelled());

        // The next unknown kid does not fetch again inside the interval.
        assert!(verifier
            .verify(&signed_token(SigningKey::generate()), None, None)
            .await
            .is_err());
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(server.hits(), 1);
    }

    #[cfg(feature = "jwks")]
    #[tokio::test]
    async fn rotated_key_is_picked_up_after_the_interval() {
        let old_key = SigningKey::generate();
        let new_key = SigningKey::generate();
        let new_token = signed_token(copy_key(&new_key));
        let server =
            CountingJwks::start(jwks_json(&[&old_key.verifying_key()]), Duration::ZERO).await;
        let interval = Duration::from_millis(300);
        let verifier = AsyncVerifier::with_jwks_url(server.url(), "test-issuer", "test-audience")
            .with_min_refresh_interval(interval);

        verifier
            .verify(&signed_token(copy_key(&old_key)), None, None)
            .await
            .unwrap();
        assert_eq!(server.hits(), 1);

        server.serve(Some(jwks_json(&[
            &old_key.verifying_key(),
            &new_key.verifying_key(),
        ])));
        assert!(matches!(
            verifier.verify(&new_token, None, None).await,
            Err(VerifyError::KeyNotFound(_))
        ));
        assert_eq!(server.hits(), 1, "no fetch inside the interval");

        tokio::time::sleep(interval + Duration::from_millis(50)).await;
        let context = verifier.verify(&new_token, None, None).await.unwrap();
        assert_eq!(context.subject, "test-subject");
        assert_eq!(server.hits(), 2);
        verifier
            .verify(&signed_token(old_key), None, None)
            .await
            .unwrap();
        assert_eq!(server.hits(), 2);
    }

    #[cfg(feature = "jwks")]
    #[tokio::test]
    async fn failed_refresh_keeps_serving_cached_keys() {
        let trusted = SigningKey::generate();
        let server =
            CountingJwks::start(jwks_json(&[&trusted.verifying_key()]), Duration::ZERO).await;
        let interval = Duration::from_millis(300);
        // Keys go stale at once, so every request after the interval refetches.
        let verifier = AsyncVerifier::with_jwks_url(server.url(), "test-issuer", "test-audience")
            .with_cache_duration(Duration::ZERO)
            .with_min_refresh_interval(interval);

        verifier
            .verify(&signed_token(copy_key(&trusted)), None, None)
            .await
            .unwrap();
        assert_eq!(server.hits(), 1);

        server.serve(None);
        tokio::time::sleep(interval + Duration::from_millis(50)).await;
        let context = verifier
            .verify(&signed_token(copy_key(&trusted)), None, None)
            .await
            .unwrap();
        assert_eq!(context.subject, "test-subject");
        assert_eq!(server.hits(), 2, "the failed fetch was attempted");

        // Failures are throttled like successes.
        assert!(matches!(
            verifier
                .verify(&signed_token(SigningKey::generate()), None, None)
                .await,
            Err(VerifyError::KeyNotFound(_))
        ));
        verifier
            .verify(&signed_token(copy_key(&trusted)), None, None)
            .await
            .unwrap();
        assert_eq!(server.hits(), 2);

        // An explicit refresh reports the failure and still keeps the keys.
        assert!(verifier.refresh_cache().await.is_err());
        assert_eq!(server.hits(), 3);
        verifier
            .verify(&signed_token(trusted), None, None)
            .await
            .unwrap();
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
