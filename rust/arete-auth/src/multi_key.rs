use crate::claims::AuthContext;
use crate::error::VerifyError;
use crate::keys::VerifyingKey;
use crate::token::TokenVerifier;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// A key with its metadata for rotation
#[derive(Clone)]
pub struct RotationKey {
    /// The verifying key
    pub key: VerifyingKey,
    /// Key ID for JWKS compatibility
    pub key_id: String,
    /// When this key was added
    pub added_at: Instant,
    /// Optional: when this key should be removed (for grace period rotation)
    pub expires_at: Option<Instant>,
    /// Whether this is the primary (current) key
    pub is_primary: bool,
}

impl RotationKey {
    /// Create a new primary key
    pub fn primary(key: VerifyingKey, key_id: impl Into<String>) -> Self {
        Self {
            key,
            key_id: key_id.into(),
            added_at: Instant::now(),
            expires_at: None,
            is_primary: true,
        }
    }

    /// Create a secondary (rotating out) key with expiration
    pub fn secondary(key: VerifyingKey, key_id: impl Into<String>, grace_period: Duration) -> Self {
        Self {
            key,
            key_id: key_id.into(),
            added_at: Instant::now(),
            expires_at: Some(Instant::now() + grace_period),
            is_primary: false,
        }
    }

    /// Check if this key has expired
    pub fn is_expired(&self) -> bool {
        self.expires_at
            .map(|exp| Instant::now() > exp)
            .unwrap_or(false)
    }
}

/// Multi-key verifier supporting graceful key rotation
///
/// This verifier maintains multiple keys and attempts verification with each
/// until one succeeds. This allows zero-downtime key rotation:
///
/// 1. Generate new key pair
/// 2. Add new key as primary, mark old key as secondary with grace period
/// 3. Update JWKS to include both keys
/// 4. After grace period, remove old key
///
/// # Example
/// ```rust
/// use arete_auth::{MultiKeyVerifier, RotationKey, SigningKey};
/// use std::time::Duration;
///
/// // Generate key pairs
/// let old_signing_key = SigningKey::generate();
/// let old_verifying_key = old_signing_key.verifying_key();
/// let new_signing_key = SigningKey::generate();
/// let new_verifying_key = new_signing_key.verifying_key();
///
/// // Create rotation keys
/// let old_key = RotationKey::secondary(old_verifying_key, "key-1", Duration::from_secs(86400));
/// let new_key = RotationKey::primary(new_verifying_key, "key-2");
///
/// let verifier = MultiKeyVerifier::new(vec![old_key, new_key], "issuer", "audience")
///     .with_cleanup_interval(Duration::from_secs(3600));
/// ```
pub struct MultiKeyVerifier {
    keys: Arc<RwLock<HashMap<String, RotationKey>>>,
    issuer: String,
    audience: String,
    require_origin: bool,
    cleanup_interval: Duration,
    last_cleanup: Arc<RwLock<Instant>>,
}

impl MultiKeyVerifier {
    /// Create a new multi-key verifier
    pub fn new(
        keys: Vec<RotationKey>,
        issuer: impl Into<String>,
        audience: impl Into<String>,
    ) -> Self {
        let key_map: HashMap<String, RotationKey> =
            keys.into_iter().map(|k| (k.key_id.clone(), k)).collect();

        Self {
            keys: Arc::new(RwLock::new(key_map)),
            issuer: issuer.into(),
            audience: audience.into(),
            require_origin: false,
            cleanup_interval: Duration::from_secs(3600), // 1 hour default
            last_cleanup: Arc::new(RwLock::new(Instant::now())),
        }
    }

    /// Create from a single key (for backward compatibility)
    pub fn from_single_key(
        key: VerifyingKey,
        key_id: impl Into<String>,
        issuer: impl Into<String>,
        audience: impl Into<String>,
    ) -> Self {
        Self::new(vec![RotationKey::primary(key, key_id)], issuer, audience)
    }

    /// Require origin validation
    pub fn with_origin_validation(mut self) -> Self {
        self.require_origin = true;
        self
    }

    /// Set cleanup interval for expired keys
    pub fn with_cleanup_interval(mut self, interval: Duration) -> Self {
        self.cleanup_interval = interval;
        self
    }

    /// Add a new key to the verifier
    pub async fn add_key(&self, key: RotationKey) {
        let mut keys = self.keys.write().await;

        // If adding a primary key, demote existing primary to secondary
        if key.is_primary {
            for (_, existing) in keys.iter_mut() {
                if existing.is_primary {
                    existing.is_primary = false;
                    // Set grace period for old primary
                    existing.expires_at = Some(Instant::now() + Duration::from_secs(86400));
                    // 24 hours
                }
            }
        }

        keys.insert(key.key_id.clone(), key);
    }

    /// Remove a key by ID
    pub async fn remove_key(&self, key_id: &str) {
        let mut keys = self.keys.write().await;
        keys.remove(key_id);
    }

    /// Get all key IDs
    pub async fn key_ids(&self) -> Vec<String> {
        let keys = self.keys.read().await;
        keys.keys().cloned().collect()
    }

    /// Get primary key ID
    pub async fn primary_key_id(&self) -> Option<String> {
        let keys = self.keys.read().await;
        keys.values()
            .find(|k| k.is_primary)
            .map(|k| k.key_id.clone())
    }

    /// Clean up expired keys
    async fn cleanup_expired_keys(&self) {
        let should_cleanup = {
            let last = self.last_cleanup.read().await;
            last.elapsed() >= self.cleanup_interval
        };

        if !should_cleanup {
            return;
        }

        let mut keys = self.keys.write().await;
        let expired: Vec<String> = keys
            .iter()
            .filter(|(_, k)| k.is_expired())
            .map(|(id, _)| id.clone())
            .collect();

        for key_id in expired {
            keys.remove(&key_id);
        }

        // Update last cleanup time
        let mut last = self.last_cleanup.write().await;
        *last = Instant::now();
    }

    /// Verify a token against all keys
    pub async fn verify(
        &self,
        token: &str,
        expected_origin: Option<&str>,
        expected_client_ip: Option<&str>,
    ) -> Result<AuthContext, VerifyError> {
        // Clean up expired keys periodically
        self.cleanup_expired_keys().await;

        let keys = self.keys.read().await;

        if keys.is_empty() {
            return Err(VerifyError::KeyNotFound("no keys configured".to_string()));
        }

        let mut last_error = None;

        // Try primary key first, then secondary keys
        let mut key_order: Vec<&RotationKey> = keys.values().collect();
        key_order.sort_by_key(|k| !k.is_primary); // Primary first

        for key_entry in key_order {
            if key_entry.is_expired() {
                continue;
            }

            let verifier = if self.require_origin {
                TokenVerifier::new(
                    key_entry.key.clone(),
                    self.issuer.clone(),
                    self.audience.clone(),
                )
                .with_origin_validation()
            } else {
                TokenVerifier::new(
                    key_entry.key.clone(),
                    self.issuer.clone(),
                    self.audience.clone(),
                )
            };

            match verifier.verify(token, expected_origin, expected_client_ip) {
                Ok(ctx) => {
                    return Ok(ctx);
                }
                Err(VerifyError::InvalidSignature) => {
                    // Wrong key, try next
                    last_error = Some(VerifyError::InvalidSignature);
                    continue;
                }
                Err(e) => {
                    // Other errors (expired, invalid format, etc.) - don't try other keys
                    return Err(e);
                }
            }
        }

        // All keys failed
        Err(last_error.unwrap_or(VerifyError::InvalidSignature))
    }

    /// Verify without cleaning up (for high-throughput scenarios)
    pub async fn verify_fast(
        &self,
        token: &str,
        expected_origin: Option<&str>,
        expected_client_ip: Option<&str>,
    ) -> Result<AuthContext, VerifyError> {
        let keys = self.keys.read().await;

        if keys.is_empty() {
            return Err(VerifyError::KeyNotFound("no keys configured".to_string()));
        }

        let mut last_error = None;

        // Try primary key first, then secondary keys
        let mut key_order: Vec<&RotationKey> = keys.values().collect();
        key_order.sort_by_key(|k| !k.is_primary);

        for key_entry in key_order {
            if key_entry.is_expired() {
                continue;
            }

            let verifier = if self.require_origin {
                TokenVerifier::new(
                    key_entry.key.clone(),
                    self.issuer.clone(),
                    self.audience.clone(),
                )
                .with_origin_validation()
            } else {
                TokenVerifier::new(
                    key_entry.key.clone(),
                    self.issuer.clone(),
                    self.audience.clone(),
                )
            };

            match verifier.verify(token, expected_origin, expected_client_ip) {
                Ok(ctx) => return Ok(ctx),
                Err(VerifyError::InvalidSignature) => {
                    last_error = Some(VerifyError::InvalidSignature);
                    continue;
                }
                Err(e) => return Err(e),
            }
        }

        Err(last_error.unwrap_or(VerifyError::InvalidSignature))
    }
}

/// Builder for constructing a MultiKeyVerifier with rotation support
pub struct MultiKeyVerifierBuilder {
    keys: Vec<RotationKey>,
    issuer: String,
    audience: String,
    require_origin: bool,
    cleanup_interval: Duration,
}

impl MultiKeyVerifierBuilder {
    /// Create a new builder
    pub fn new(issuer: impl Into<String>, audience: impl Into<String>) -> Self {
        Self {
            keys: Vec::new(),
            issuer: issuer.into(),
            audience: audience.into(),
            require_origin: false,
            cleanup_interval: Duration::from_secs(3600),
        }
    }

    /// Add a primary key
    pub fn with_primary_key(mut self, key: VerifyingKey, key_id: impl Into<String>) -> Self {
        self.keys.push(RotationKey::primary(key, key_id));
        self
    }

    /// Add a secondary key with grace period
    pub fn with_secondary_key(
        mut self,
        key: VerifyingKey,
        key_id: impl Into<String>,
        grace_period: Duration,
    ) -> Self {
        self.keys
            .push(RotationKey::secondary(key, key_id, grace_period));
        self
    }

    /// Require origin validation
    pub fn with_origin_validation(mut self) -> Self {
        self.require_origin = true;
        self
    }

    /// Set cleanup interval
    pub fn with_cleanup_interval(mut self, interval: Duration) -> Self {
        self.cleanup_interval = interval;
        self
    }

    /// Build the verifier
    pub fn build(self) -> MultiKeyVerifier {
        let mut verifier = MultiKeyVerifier::new(self.keys, self.issuer, self.audience);
        if self.require_origin {
            verifier = verifier.with_origin_validation();
        }
        verifier.with_cleanup_interval(self.cleanup_interval)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claims::{KeyClass, SessionClaims};
    use crate::keys::SigningKey;
    use crate::token::TokenSigner;

    #[tokio::test]
    async fn test_multi_key_verifier_single_key() {
        let signing_key = SigningKey::generate();
        let verifying_key = signing_key.verifying_key();

        let signer = TokenSigner::new(signing_key, "test-issuer");
        let verifier = MultiKeyVerifier::from_single_key(
            verifying_key,
            "key-1",
            "test-issuer",
            "test-audience",
        );

        let claims = SessionClaims::builder("test-issuer", "test-subject", "test-audience")
            .with_scope("read")
            .with_metering_key("meter-123")
            .with_key_class(KeyClass::Publishable)
            .build();

        let token = signer.sign(claims).unwrap();
        let context = verifier.verify(&token, None, None).await.unwrap();

        assert_eq!(context.subject, "test-subject");
        assert_eq!(verifier.primary_key_id().await, Some("key-1".to_string()));
    }

    #[tokio::test]
    async fn test_key_rotation() {
        // Create old key pair
        let old_signing_key = SigningKey::generate();
        let old_verifying_key = old_signing_key.verifying_key();
        let old_signer = TokenSigner::new(old_signing_key, "test-issuer");

        // Create new key pair
        let new_signing_key = SigningKey::generate();
        let new_verifying_key = new_signing_key.verifying_key();
        let new_signer = TokenSigner::new(new_signing_key, "test-issuer");

        // Start with old key as primary
        let old_key = RotationKey::primary(old_verifying_key.clone(), "key-old");
        let verifier = MultiKeyVerifier::new(vec![old_key], "test-issuer", "test-audience");

        // Sign token with old key
        let old_claims = SessionClaims::builder("test-issuer", "subject-1", "test-audience")
            .with_scope("read")
            .with_metering_key("meter-1")
            .with_key_class(KeyClass::Publishable)
            .build();
        let old_token = old_signer.sign(old_claims).unwrap();

        // Verify old token works
        let ctx = verifier.verify(&old_token, None, None).await.unwrap();
        assert_eq!(ctx.subject, "subject-1");

        // Rotate: add new key as primary (old key becomes secondary)
        let new_key = RotationKey::primary(new_verifying_key, "key-new");
        verifier.add_key(new_key).await;

        // Verify old token still works (grace period)
        let ctx = verifier.verify(&old_token, None, None).await.unwrap();
        assert_eq!(ctx.subject, "subject-1");

        // Sign and verify new token
        let new_claims = SessionClaims::builder("test-issuer", "subject-2", "test-audience")
            .with_scope("read")
            .with_metering_key("meter-2")
            .with_key_class(KeyClass::Publishable)
            .build();
        let new_token = new_signer.sign(new_claims).unwrap();

        let ctx = verifier.verify(&new_token, None, None).await.unwrap();
        assert_eq!(ctx.subject, "subject-2");

        // Check that new key is now primary
        assert_eq!(verifier.primary_key_id().await, Some("key-new".to_string()));

        // Both keys should be present
        let key_ids = verifier.key_ids().await;
        assert!(key_ids.contains(&"key-old".to_string()));
        assert!(key_ids.contains(&"key-new".to_string()));
    }

    #[tokio::test]
    async fn test_verifier_builder() {
        let signing_key = SigningKey::generate();
        let verifying_key = signing_key.verifying_key();

        let verifier = MultiKeyVerifierBuilder::new("test-issuer", "test-audience")
            .with_primary_key(verifying_key, "key-1")
            .with_origin_validation()
            .build();

        let signer = TokenSigner::new(signing_key, "test-issuer");
        let claims = SessionClaims::builder("test-issuer", "test-subject", "test-audience")
            .with_scope("read")
            .with_origin("https://trusted.example.com")
            .with_key_class(KeyClass::Secret)
            .build();

        let token = signer.sign(claims).unwrap();
        let ctx = verifier
            .verify(&token, Some("https://trusted.example.com"), None)
            .await
            .unwrap();
        assert_eq!(ctx.subject, "test-subject");
    }

    #[tokio::test]
    async fn test_invalid_signature_with_multiple_keys() {
        // Create two different key pairs
        let key1_signing = SigningKey::generate();
        let key1_verifying = key1_signing.verifying_key();

        let key2_signing = SigningKey::generate();
        let _key2_verifying = key2_signing.verifying_key();

        let signer = TokenSigner::new(key1_signing, "test-issuer");

        // Create verifier with only key2
        let verifier = MultiKeyVerifier::from_single_key(
            key2_signing.verifying_key(),
            "key-2",
            "test-issuer",
            "test-audience",
        );

        let claims = SessionClaims::builder("test-issuer", "test-subject", "test-audience")
            .with_scope("read")
            .with_key_class(KeyClass::Publishable)
            .build();

        let token = signer.sign(claims).unwrap();

        // Should fail because token was signed with key1, verifier only has key2
        let result = verifier.verify(&token, None, None).await;
        assert!(matches!(result, Err(VerifyError::InvalidSignature)));
    }

    #[tokio::test]
    async fn test_jwks_key_rotation_grace_period() {
        use crate::token::{Jwk, Jwks};
        use base64::Engine;

        // Create old key pair with specific key ID
        let old_signing_key = SigningKey::generate();
        let old_verifying_key = old_signing_key.verifying_key();
        let old_kid = old_verifying_key.key_id();
        let old_signer = TokenSigner::new(old_signing_key, "test-issuer");

        // Create new key pair with specific key ID
        let new_signing_key = SigningKey::generate();
        let new_verifying_key = new_signing_key.verifying_key();
        let new_kid = new_verifying_key.key_id();
        let new_signer = TokenSigner::new(new_signing_key, "test-issuer");

        // Create JWKS with both keys using their actual key IDs
        let old_key_b64 =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(old_verifying_key.to_bytes());
        let new_key_b64 =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(new_verifying_key.to_bytes());

        let jwks = Jwks {
            keys: vec![
                Jwk {
                    kty: "OKP".to_string(),
                    use_: Some("sig".to_string()),
                    kid: old_kid,
                    x: old_key_b64,
                },
                Jwk {
                    kty: "OKP".to_string(),
                    use_: Some("sig".to_string()),
                    kid: new_kid,
                    x: new_key_b64,
                },
            ],
        };

        // Create verifier from JWKS
        let verifier =
            crate::verifier::AsyncVerifier::with_jwks(jwks, "test-issuer", "test-audience");

        // Sign and verify token with old key
        let old_claims = SessionClaims::builder("test-issuer", "subject-old", "test-audience")
            .with_scope("read")
            .with_key_class(KeyClass::Secret)
            .build();
        let old_token = old_signer.sign(old_claims).unwrap();

        // Old token should still verify during rotation
        let ctx = verifier.verify(&old_token, None, None).await.unwrap();
        assert_eq!(ctx.subject, "subject-old");

        // Sign and verify token with new key
        let new_claims = SessionClaims::builder("test-issuer", "subject-new", "test-audience")
            .with_scope("read")
            .with_key_class(KeyClass::Secret)
            .build();
        let new_token = new_signer.sign(new_claims).unwrap();

        // New token should also verify
        let ctx = verifier.verify(&new_token, None, None).await.unwrap();
        assert_eq!(ctx.subject, "subject-new");
    }

    #[tokio::test]
    async fn test_jwks_key_not_found() {
        use crate::token::{Jwk, Jwks};
        use base64::Engine;

        // Create a key pair
        let signing_key = SigningKey::generate();
        let _verifying_key = signing_key.verifying_key();
        let signer = TokenSigner::new(signing_key, "test-issuer");

        // Create JWKS with a different key (not the one used for signing)
        let different_key = SigningKey::generate();
        let different_verifying_key = different_key.verifying_key();
        let different_key_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(different_verifying_key.to_bytes());

        let jwks = Jwks {
            keys: vec![Jwk {
                kty: "OKP".to_string(),
                use_: Some("sig".to_string()),
                kid: "different-key".to_string(),
                x: different_key_b64,
            }],
        };

        let verifier =
            crate::verifier::AsyncVerifier::with_jwks(jwks, "test-issuer", "test-audience");

        let claims = SessionClaims::builder("test-issuer", "test-subject", "test-audience")
            .with_scope("read")
            .with_key_class(KeyClass::Secret)
            .build();
        let token = signer.sign(claims).unwrap();

        // Should fail with key not found
        let result = verifier.verify(&token, None, None).await;
        assert!(matches!(result, Err(VerifyError::KeyNotFound(_))));
    }

    #[tokio::test]
    async fn test_jwks_with_origin_validation() {
        use crate::token::{Jwk, Jwks};
        use base64::Engine;

        let signing_key = SigningKey::generate();
        let verifying_key = signing_key.verifying_key();
        let kid = verifying_key.key_id();
        let signer = TokenSigner::new(signing_key, "test-issuer");

        let key_b64 =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(verifying_key.to_bytes());

        let jwks = Jwks {
            keys: vec![Jwk {
                kty: "OKP".to_string(),
                use_: Some("sig".to_string()),
                kid,
                x: key_b64,
            }],
        };

        // Create verifier with origin validation
        let verifier =
            crate::verifier::AsyncVerifier::with_jwks(jwks, "test-issuer", "test-audience")
                .with_origin_validation();

        // Token with matching origin
        let claims = SessionClaims::builder("test-issuer", "test-subject", "test-audience")
            .with_scope("read")
            .with_key_class(KeyClass::Secret)
            .with_origin("https://trusted.example.com")
            .build();
        let token = signer.sign(claims).unwrap();

        // Should succeed with matching origin
        let ctx = verifier
            .verify(&token, Some("https://trusted.example.com"), None)
            .await
            .unwrap();
        assert_eq!(ctx.subject, "test-subject");

        // Should fail with wrong origin
        let result = verifier
            .verify(&token, Some("https://evil.example.com"), None)
            .await;
        assert!(matches!(result, Err(VerifyError::OriginMismatch { .. })));
    }
}
