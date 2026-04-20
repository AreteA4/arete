use crate::claims::{AuthContext, SessionClaims};
use crate::error::VerifyError;
use crate::keys::{SigningKey, VerifyingKey};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json;

/// JWT Header for EdDSA (Ed25519) tokens
#[derive(Debug, Clone, Serialize, Deserialize)]
struct JwtHeader {
    alg: String,
    typ: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    kid: Option<String>,
}

impl Default for JwtHeader {
    fn default() -> Self {
        Self {
            alg: "EdDSA".to_string(),
            typ: "JWT".to_string(),
            kid: None,
        }
    }
}

/// Token signer for issuing session tokens using Ed25519 (EdDSA)
pub struct TokenSigner {
    signing_key: SigningKey,
    issuer: String,
}

impl TokenSigner {
    /// Create a new token signer with an Ed25519 signing key
    ///
    /// Uses EdDSA (Ed25519) for asymmetric signing. This is the recommended
    /// algorithm for production use as it provides better security than HMAC.
    pub fn new(signing_key: SigningKey, issuer: impl Into<String>) -> Self {
        Self {
            signing_key,
            issuer: issuer.into(),
        }
    }

    /// Sign a session token using Ed25519
    pub fn sign(&self, claims: SessionClaims) -> Result<String, TokenError> {
        // Create header with key ID
        let header = JwtHeader {
            kid: Some(self.signing_key.key_id()),
            ..Default::default()
        };

        // Encode header
        let header_json = serde_json::to_string(&header)?;
        let header_b64 =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(header_json.as_bytes());

        // Encode claims
        let claims_json = serde_json::to_string(&claims)?;
        let claims_b64 =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(claims_json.as_bytes());

        // Create message to sign
        let message = format!("{}.{}", header_b64, claims_b64);

        // Sign with Ed25519
        let signature = self.signing_key.sign(message.as_bytes());
        let signature_b64 =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(signature.to_bytes());

        // Combine into JWT
        Ok(format!("{}.{}.{}", header_b64, claims_b64, signature_b64))
    }

    /// Get the issuer
    pub fn issuer(&self) -> &str {
        &self.issuer
    }
}

/// Token error type
#[derive(Debug)]
pub enum TokenError {
    Serialization(serde_json::Error),
    Base64(base64::DecodeError),
    InvalidFormat(String),
}

impl std::fmt::Display for TokenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TokenError::Serialization(e) => write!(f, "Serialization error: {}", e),
            TokenError::Base64(e) => write!(f, "Base64 error: {}", e),
            TokenError::InvalidFormat(s) => write!(f, "Invalid format: {}", s),
        }
    }
}

impl std::error::Error for TokenError {}

impl From<serde_json::Error> for TokenError {
    fn from(e: serde_json::Error) -> Self {
        TokenError::Serialization(e)
    }
}

impl From<base64::DecodeError> for TokenError {
    fn from(e: base64::DecodeError) -> Self {
        TokenError::Base64(e)
    }
}

/// Token verifier for validating session tokens using Ed25519 (EdDSA)
pub struct TokenVerifier {
    verifying_key: VerifyingKey,
    issuer: String,
    audience: String,
    require_origin: bool,
    require_client_ip: bool,
}

impl TokenVerifier {
    /// Create a new token verifier with an Ed25519 verifying key
    ///
    /// Uses EdDSA (Ed25519) for asymmetric signature verification.
    /// This is the recommended algorithm for production use.
    pub fn new(
        verifying_key: VerifyingKey,
        issuer: impl Into<String>,
        audience: impl Into<String>,
    ) -> Self {
        Self {
            verifying_key,
            issuer: issuer.into(),
            audience: audience.into(),
            require_origin: false,
            require_client_ip: false,
        }
    }

    /// Require origin validation
    pub fn with_origin_validation(mut self) -> Self {
        self.require_origin = true;
        self
    }

    /// Require client IP validation
    pub fn with_client_ip_validation(mut self) -> Self {
        self.require_client_ip = true;
        self
    }

    /// Verify a token and return the auth context
    ///
    /// # Arguments
    /// * `token` - The JWT token to verify
    /// * `expected_origin` - Optional expected origin for origin validation
    /// * `expected_client_ip` - Optional expected client IP for IP binding validation
    pub fn verify(
        &self,
        token: &str,
        expected_origin: Option<&str>,
        expected_client_ip: Option<&str>,
    ) -> Result<AuthContext, VerifyError> {
        // Split token into parts
        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() != 3 {
            return Err(VerifyError::InvalidFormat("Invalid JWT format".to_string()));
        }

        let header_b64 = parts[0];
        let claims_b64 = parts[1];
        let signature_b64 = parts[2];

        // Decode and verify header
        let header_json = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(header_b64)
            .map_err(|e| VerifyError::InvalidFormat(format!("Invalid header base64: {}", e)))?;
        let header: JwtHeader = serde_json::from_slice(&header_json)
            .map_err(|e| VerifyError::InvalidFormat(format!("Invalid header JSON: {}", e)))?;

        if header.alg != "EdDSA" {
            return Err(VerifyError::InvalidFormat(format!(
                "Unsupported algorithm: {}",
                header.alg
            )));
        }

        // Decode claims
        let claims_json = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(claims_b64)
            .map_err(|e| VerifyError::InvalidFormat(format!("Invalid claims base64: {}", e)))?;
        let claims: SessionClaims = serde_json::from_slice(&claims_json)
            .map_err(|e| VerifyError::InvalidFormat(format!("Invalid claims JSON: {}", e)))?;

        // Decode signature
        let signature_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(signature_b64)
            .map_err(|e| VerifyError::InvalidFormat(format!("Invalid signature base64: {}", e)))?;
        if signature_bytes.len() != 64 {
            return Err(VerifyError::InvalidFormat(
                "Invalid signature length".to_string(),
            ));
        }
        let signature = ed25519_dalek::Signature::from_bytes(&signature_bytes.try_into().unwrap());

        // Verify signature
        let message = format!("{}.{}", header_b64, claims_b64);
        self.verifying_key
            .verify(message.as_bytes(), &signature)
            .map_err(|_| VerifyError::InvalidSignature)?;

        // Check issuer
        if claims.iss != self.issuer {
            return Err(VerifyError::InvalidIssuer);
        }

        // Check audience
        if claims.aud != self.audience {
            return Err(VerifyError::InvalidAudience);
        }

        // Check expiration
        use std::time::{SystemTime, UNIX_EPOCH};
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should not be before epoch")
            .as_secs();

        if claims.exp <= now {
            return Err(VerifyError::Expired);
        }

        if claims.nbf > now {
            return Err(VerifyError::NotYetValid);
        }

        // Validate origin if required or if token has origin binding
        let token_has_origin = claims.origin.is_some();
        let origin_provided = expected_origin.is_some();

        if token_has_origin && origin_provided {
            // Token is origin-bound and origin was provided - validate they match
            let expected = expected_origin.unwrap();
            let actual = claims.origin.as_ref().unwrap();

            if actual != expected {
                return Err(VerifyError::OriginMismatch {
                    expected: expected.to_string(),
                    actual: actual.clone(),
                });
            }
        } else if token_has_origin && self.require_origin {
            // Token has origin but none was provided, and origin is required
            return Err(VerifyError::OriginRequired {
                token_origin: claims.origin.as_ref().unwrap().clone(),
            });
        } else if !token_has_origin && self.require_origin {
            // Verifier requires origin but token doesn't have one bound
            return Err(VerifyError::MissingClaim("origin".to_string()));
        }
        // If token has origin but none provided, and origin is NOT required,
        // we allow the connection (defense-in-depth is optional)

        // Validate client IP if required
        if self.require_client_ip {
            if let Some(expected) = expected_client_ip {
                match &claims.client_ip {
                    Some(actual) if actual == expected => {}
                    Some(actual) => {
                        return Err(VerifyError::OriginMismatch {
                            expected: expected.to_string(),
                            actual: actual.clone(),
                        });
                    }
                    None => {
                        return Err(VerifyError::MissingClaim("client_ip".to_string()));
                    }
                }
            } else if claims.client_ip.is_none() {
                return Err(VerifyError::MissingClaim("client_ip".to_string()));
            }
        }

        Ok(AuthContext::from_claims(claims))
    }

    /// Get the expected issuer
    pub fn issuer(&self) -> &str {
        &self.issuer
    }

    /// Get the expected audience
    pub fn audience(&self) -> &str {
        &self.audience
    }
}

/// JWKS structure for key rotation
#[derive(Debug, Clone, Deserialize)]
pub struct Jwks {
    pub keys: Vec<Jwk>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Jwk {
    pub kty: String,
    #[serde(rename = "use")]
    pub use_: Option<String>,
    pub kid: String,
    pub x: String, // Base64-encoded public key
}

/// Token verifier with JWKS support for key rotation
#[derive(Clone)]
pub struct JwksVerifier {
    jwks: Jwks,
    issuer: String,
    audience: String,
    require_origin: bool,
}

impl JwksVerifier {
    /// Create a new JWKS verifier
    pub fn new(jwks: Jwks, issuer: impl Into<String>, audience: impl Into<String>) -> Self {
        Self {
            jwks,
            issuer: issuer.into(),
            audience: audience.into(),
            require_origin: false,
        }
    }

    /// Require origin validation
    pub fn with_origin_validation(mut self) -> Self {
        self.require_origin = true;
        self
    }

    /// Verify a token using the appropriate key from JWKS
    pub fn verify(
        &self,
        token: &str,
        expected_origin: Option<&str>,
        expected_client_ip: Option<&str>,
    ) -> Result<AuthContext, VerifyError> {
        // Decode header to get kid
        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() != 3 {
            return Err(VerifyError::InvalidFormat("Invalid JWT format".to_string()));
        }

        let header_json = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(parts[0])
            .map_err(|e| VerifyError::InvalidFormat(format!("Invalid header: {}", e)))?;
        let header: JwtHeader = serde_json::from_slice(&header_json)
            .map_err(|e| VerifyError::InvalidFormat(format!("Invalid header JSON: {}", e)))?;

        let kid = header
            .kid
            .ok_or_else(|| VerifyError::MissingClaim("kid".to_string()))?;

        // Find the key
        let jwk = self
            .jwks
            .keys
            .iter()
            .find(|k| k.kid == kid)
            .ok_or(VerifyError::KeyNotFound(kid))?;

        // Decode the public key from hex (first 16 chars of hex = 8 bytes of key id)
        // Actually, we need to decode the full public key from the JWKS
        // The JWKS should contain the full base64-encoded public key
        let public_key_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(&jwk.x)
            .map_err(|_| VerifyError::InvalidFormat("Invalid public key base64".to_string()))?;

        let public_key: [u8; 32] = public_key_bytes
            .try_into()
            .map_err(|_| VerifyError::InvalidFormat("Invalid key length".to_string()))?;

        // Create verifier for this key
        let verifying_key = VerifyingKey::from_bytes(&public_key)
            .map_err(|e| VerifyError::InvalidFormat(e.to_string()))?;

        let verifier = if self.require_origin {
            TokenVerifier::new(verifying_key, &self.issuer, &self.audience).with_origin_validation()
        } else {
            TokenVerifier::new(verifying_key, &self.issuer, &self.audience)
        };

        verifier.verify(token, expected_origin, expected_client_ip)
    }

    /// Fetch JWKS from a URL
    #[cfg(feature = "jwks")]
    pub async fn fetch_jwks(url: &str) -> Result<Jwks, reqwest::Error> {
        let response = reqwest::get(url).await?;
        let jwks: Jwks = response.json().await?;
        Ok(jwks)
    }
}

#[cfg(test)]
/// HMAC-based verifier for tests only
pub struct HmacVerifier {
    _secret: Vec<u8>,
    _issuer: String,
    _audience: String,
}

#[cfg(test)]
impl HmacVerifier {
    /// Create a new HMAC verifier (dev only)
    pub fn new(
        secret: impl Into<Vec<u8>>,
        issuer: impl Into<String>,
        audience: impl Into<String>,
    ) -> Self {
        Self {
            _secret: secret.into(),
            _issuer: issuer.into(),
            _audience: audience.into(),
        }
    }

    /// Verify a token using HMAC
    pub fn verify(
        &self,
        token: &str,
        _expected_origin: Option<&str>,
    ) -> Result<AuthContext, VerifyError> {
        // Split token
        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() != 3 {
            return Err(VerifyError::InvalidFormat("Invalid JWT format".to_string()));
        }

        // For HMAC, we'd need to verify the HMAC signature
        // This is a simplified implementation - in practice you'd use hmac-sha256
        // For now, just decode the claims without verification (dev only!)
        let claims_json = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(parts[1])
            .map_err(|e| VerifyError::InvalidFormat(format!("Invalid claims: {}", e)))?;
        let claims: SessionClaims = serde_json::from_slice(&claims_json)
            .map_err(|e| VerifyError::InvalidFormat(format!("Invalid claims JSON: {}", e)))?;

        Ok(AuthContext::from_claims(claims))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claims::{KeyClass, Limits};

    fn create_test_claims() -> SessionClaims {
        SessionClaims::builder("test-issuer", "test-subject", "test-audience")
            .with_ttl(300)
            .with_scope("read")
            .with_metering_key("meter-123")
            .with_key_class(KeyClass::Publishable)
            .with_limits(Limits {
                max_connections: Some(10),
                max_subscriptions: Some(100),
                max_snapshot_rows: Some(1000),
                max_messages_per_minute: Some(1000),
                max_bytes_per_minute: Some(10_000_000),
            })
            .build()
    }

    #[test]
    fn test_sign_and_verify() {
        // Generate keys
        let signing_key = crate::keys::SigningKey::generate();
        let verifying_key = signing_key.verifying_key();

        // Create signer and verifier
        let signer = TokenSigner::new(signing_key, "test-issuer");
        let verifier = TokenVerifier::new(verifying_key, "test-issuer", "test-audience");

        // Sign token
        let claims = create_test_claims();
        let token = signer.sign(claims.clone()).unwrap();

        // Verify token
        let context = verifier.verify(&token, None, None).unwrap();

        assert_eq!(context.subject, "test-subject");
        assert_eq!(context.issuer, "test-issuer");
        assert_eq!(context.metering_key, "meter-123");
    }

    #[test]
    fn test_expired_token() {
        let signing_key = crate::keys::SigningKey::generate();
        let verifying_key = signing_key.verifying_key();

        let signer = TokenSigner::new(signing_key, "test-issuer");
        let verifier = TokenVerifier::new(verifying_key, "test-issuer", "test-audience");

        // Create expired claims
        let claims = SessionClaims::builder("test-issuer", "test-subject", "test-audience")
            .with_ttl(0) // Already expired
            .with_scope("read")
            .with_metering_key("meter-123")
            .with_key_class(KeyClass::Publishable)
            .build();

        let token = signer.sign(claims).unwrap();

        // Should fail with expired error
        let result = verifier.verify(&token, None, None);
        assert!(matches!(result, Err(VerifyError::Expired)));
    }

    #[test]
    fn test_invalid_signature() {
        let signing_key = crate::keys::SigningKey::generate();
        let wrong_signing_key = crate::keys::SigningKey::generate();
        let wrong_verifying_key = wrong_signing_key.verifying_key();

        let signer = TokenSigner::new(signing_key, "test-issuer");
        let verifier = TokenVerifier::new(wrong_verifying_key, "test-issuer", "test-audience");

        let claims = create_test_claims();
        let token = signer.sign(claims).unwrap();

        // Should fail with invalid signature
        let result = verifier.verify(&token, None, None);
        assert!(matches!(result, Err(VerifyError::InvalidSignature)));
    }

    #[test]
    fn test_wrong_issuer() {
        let signing_key = crate::keys::SigningKey::generate();
        let verifying_key = signing_key.verifying_key();

        let signer = TokenSigner::new(signing_key, "wrong-issuer");
        let verifier = TokenVerifier::new(verifying_key, "test-issuer", "test-audience");

        // Create claims with the wrong issuer
        let claims = SessionClaims::builder("wrong-issuer", "test-subject", "test-audience")
            .with_ttl(300)
            .with_scope("read")
            .with_metering_key("meter-123")
            .with_key_class(KeyClass::Publishable)
            .build();
        let token = signer.sign(claims).unwrap();

        // Should fail with invalid issuer
        let result = verifier.verify(&token, None, None);
        assert!(matches!(result, Err(VerifyError::InvalidIssuer)));
    }

    #[test]
    fn test_wrong_audience() {
        let signing_key = crate::keys::SigningKey::generate();
        let verifying_key = signing_key.verifying_key();

        let signer = TokenSigner::new(signing_key, "test-issuer");
        let verifier = TokenVerifier::new(verifying_key, "test-issuer", "expected-audience");

        let claims = SessionClaims::builder("test-issuer", "test-subject", "wrong-audience")
            .with_ttl(300)
            .with_scope("read")
            .with_metering_key("meter-123")
            .with_key_class(KeyClass::Publishable)
            .build();
        let token = signer.sign(claims).unwrap();

        let result = verifier.verify(&token, None, None);
        assert!(matches!(result, Err(VerifyError::InvalidAudience)));
    }

    #[test]
    fn test_origin_mismatch() {
        let signing_key = crate::keys::SigningKey::generate();
        let verifying_key = signing_key.verifying_key();

        let signer = TokenSigner::new(signing_key, "test-issuer");
        let verifier = TokenVerifier::new(verifying_key, "test-issuer", "test-audience")
            .with_origin_validation();

        let claims = SessionClaims::builder("test-issuer", "test-subject", "test-audience")
            .with_ttl(300)
            .with_scope("read")
            .with_metering_key("meter-123")
            .with_origin("https://allowed.example")
            .with_key_class(KeyClass::Publishable)
            .build();
        let token = signer.sign(claims).unwrap();

        let result = verifier.verify(&token, Some("https://other.example"), None);
        assert!(matches!(result, Err(VerifyError::OriginMismatch { .. })));
    }

    #[test]
    fn test_origin_validation_success() {
        let signing_key = crate::keys::SigningKey::generate();
        let verifying_key = signing_key.verifying_key();

        let signer = TokenSigner::new(signing_key, "test-issuer");
        let verifier = TokenVerifier::new(verifying_key, "test-issuer", "test-audience")
            .with_origin_validation();

        let claims = SessionClaims::builder("test-issuer", "test-subject", "test-audience")
            .with_ttl(300)
            .with_scope("read")
            .with_metering_key("meter-123")
            .with_origin("https://allowed.example")
            .with_key_class(KeyClass::Publishable)
            .build();
        let token = signer.sign(claims).unwrap();

        let context = verifier
            .verify(&token, Some("https://allowed.example"), None)
            .unwrap();
        assert_eq!(context.origin.as_deref(), Some("https://allowed.example"));
    }

    #[test]
    fn test_origin_validation_requires_origin_claim() {
        let signing_key = crate::keys::SigningKey::generate();
        let verifying_key = signing_key.verifying_key();

        let signer = TokenSigner::new(signing_key, "test-issuer");
        let verifier = TokenVerifier::new(verifying_key, "test-issuer", "test-audience")
            .with_origin_validation();

        let claims = SessionClaims::builder("test-issuer", "test-subject", "test-audience")
            .with_ttl(300)
            .with_scope("read")
            .with_metering_key("meter-123")
            .with_key_class(KeyClass::Publishable)
            .build();
        let token = signer.sign(claims).unwrap();

        let result = verifier.verify(&token, None, None);
        assert!(matches!(
            result,
            Err(VerifyError::MissingClaim(ref claim)) if claim == "origin"
        ));
    }

    #[test]
    fn test_client_ip_validation_requires_client_ip_claim() {
        let signing_key = crate::keys::SigningKey::generate();
        let verifying_key = signing_key.verifying_key();

        let signer = TokenSigner::new(signing_key, "test-issuer");
        let verifier = TokenVerifier::new(verifying_key, "test-issuer", "test-audience")
            .with_client_ip_validation();

        let claims = SessionClaims::builder("test-issuer", "test-subject", "test-audience")
            .with_ttl(300)
            .with_scope("read")
            .with_metering_key("meter-123")
            .with_key_class(KeyClass::Publishable)
            .build();
        let token = signer.sign(claims).unwrap();

        let result = verifier.verify(&token, None, None);
        assert!(matches!(
            result,
            Err(VerifyError::MissingClaim(ref claim)) if claim == "client_ip"
        ));
    }

    #[test]
    fn test_origin_bound_token_allows_no_origin_when_not_required() {
        // This tests the non-browser client scenario (Rust, Python, etc.)
        // where the client doesn't send an Origin header, but the JWT has
        // an origin claim from when the token was minted via browser/API.
        // When require_origin is false, the connection should still be allowed
        // for defense-in-depth flexibility.
        let signing_key = crate::keys::SigningKey::generate();
        let verifying_key = signing_key.verifying_key();

        let signer = TokenSigner::new(signing_key, "test-issuer");
        // Verifier WITHOUT origin validation (the default for public stacks)
        let verifier = TokenVerifier::new(verifying_key, "test-issuer", "test-audience");

        let claims = SessionClaims::builder("test-issuer", "test-subject", "test-audience")
            .with_ttl(300)
            .with_scope("read")
            .with_metering_key("meter-123")
            .with_origin("https://example.com") // Token has origin claim
            .with_key_class(KeyClass::Publishable)
            .build();
        let token = signer.sign(claims).unwrap();

        // No origin provided, but require_origin is false - should succeed
        let context = verifier.verify(&token, None, None).unwrap();
        assert_eq!(context.origin.as_deref(), Some("https://example.com"));
    }

    #[test]
    fn test_origin_bound_token_validates_when_origin_provided_even_when_not_required() {
        // When origin IS provided, it should still be validated against the token
        // even when require_origin is false (defense-in-depth)
        let signing_key = crate::keys::SigningKey::generate();
        let verifying_key = signing_key.verifying_key();

        let signer = TokenSigner::new(signing_key, "test-issuer");
        // Verifier WITHOUT origin validation (the default)
        let verifier = TokenVerifier::new(verifying_key, "test-issuer", "test-audience");

        let claims = SessionClaims::builder("test-issuer", "test-subject", "test-audience")
            .with_ttl(300)
            .with_scope("read")
            .with_metering_key("meter-123")
            .with_origin("https://allowed.example")
            .with_key_class(KeyClass::Publishable)
            .build();
        let token = signer.sign(claims).unwrap();

        // Origin provided and matches - should succeed
        let context = verifier
            .verify(&token, Some("https://allowed.example"), None)
            .unwrap();
        assert_eq!(context.origin.as_deref(), Some("https://allowed.example"));

        // Origin provided but doesn't match - should fail
        let result = verifier.verify(&token, Some("https://evil.example"), None);
        assert!(matches!(result, Err(VerifyError::OriginMismatch { .. })));
    }
}
