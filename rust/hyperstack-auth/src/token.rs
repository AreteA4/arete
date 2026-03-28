use crate::claims::{AuthContext, SessionClaims};
use crate::error::VerifyError;
use crate::keys::{SigningKey, VerifyingKey};
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::Deserialize;

/// Token signer for issuing session tokens
pub struct TokenSigner {
    signing_key: SigningKey,
    encoding_key: EncodingKey,
    issuer: String,
}

impl TokenSigner {
    /// Create a new token signer with a signing key
    /// 
    /// Note: Currently uses HMAC-SHA256 for simplicity. Ed25519 support will be added in a future version.
    pub fn new(signing_key: SigningKey, issuer: impl Into<String>) -> Self {
        // For now, use HMAC-SHA256 which is simpler and well-supported
        // TODO: Add proper Ed25519 support with correct PKCS#8 encoding
        let key_bytes = signing_key.to_bytes();
        let encoding_key = EncodingKey::from_secret(&key_bytes);
        
        Self {
            signing_key,
            encoding_key,
            issuer: issuer.into(),
        }
    }

    /// Sign a session token
    pub fn sign(&self, claims: SessionClaims) -> Result<String, jsonwebtoken::errors::Error> {
        // Using HMAC-SHA256 for now
        let header = Header::new(Algorithm::HS256);
        encode(&header, &claims, &self.encoding_key)
    }

    /// Get the issuer
    pub fn issuer(&self) -> &str {
        &self.issuer
    }
}

/// Token verifier for validating session tokens
pub struct TokenVerifier {
    verifying_key: VerifyingKey,
    decoding_key: DecodingKey,
    issuer: String,
    audience: String,
    require_origin: bool,
}

impl TokenVerifier {
    /// Create a new token verifier with a verifying key
    /// 
    /// Note: Currently uses HMAC-SHA256 for simplicity. Ed25519 support will be added in a future version.
    pub fn new(verifying_key: VerifyingKey, issuer: impl Into<String>, audience: impl Into<String>) -> Self {
        // For now, use HMAC-SHA256 which is simpler and well-supported
        // TODO: Add proper Ed25519 support with correct key format
        let key_bytes = verifying_key.to_bytes();
        let decoding_key = DecodingKey::from_secret(&key_bytes);
        
        Self {
            verifying_key,
            decoding_key,
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

    /// Verify a token and return the auth context
    pub fn verify(&self,
        token: &str,
        expected_origin: Option<&str>,
    ) -> Result<AuthContext, VerifyError> {
        // Using HMAC-SHA256 for now
        let mut validation = Validation::new(Algorithm::HS256);
        validation.set_issuer(&[&self.issuer]);
        validation.set_audience(&[&self.audience]);
        
        let token_data = decode::<SessionClaims>(
            token,
            &self.decoding_key,
            &validation,
        ).map_err(|e| match e.kind() {
            jsonwebtoken::errors::ErrorKind::ExpiredSignature => VerifyError::Expired,
            jsonwebtoken::errors::ErrorKind::InvalidSignature => VerifyError::InvalidSignature,
            _ => VerifyError::DecodeError(e.to_string()),
        })?;

        let claims = token_data.claims;

        // Check not-before
        use std::time::{SystemTime, UNIX_EPOCH};
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should not be before epoch")
            .as_secs();

        if claims.nbf > now {
            return Err(VerifyError::NotYetValid);
        }

        // Validate origin if required
        if self.require_origin {
            if let Some(expected) = expected_origin {
                match &claims.origin {
                    Some(actual) if actual == expected => {}
                    Some(actual) => {
                        return Err(VerifyError::OriginMismatch {
                            expected: expected.to_string(),
                            actual: actual.clone(),
                        });
                    }
                    None => {
                        return Err(VerifyError::MissingClaim("origin".to_string()));
                    }
                }
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
    ) -> Result<AuthContext, VerifyError> {
        // Decode header to get kid
        let header = jsonwebtoken::decode_header(token)
            .map_err(|e| VerifyError::DecodeError(e.to_string()))?;
        
        let kid = header.kid
            .ok_or_else(|| VerifyError::MissingClaim("kid".to_string()))?;

        // Find the key
        let jwk = self.jwks.keys
            .iter()
            .find(|k| k.kid == kid)
            .ok_or_else(|| VerifyError::KeyNotFound(kid))?;

        // Decode the public key from base64
        let public_key_bytes = base64::Engine::decode(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
            &jwk.x,
        ).map_err(|e| VerifyError::InvalidFormat(format!("Invalid base64: {}", e)))?;

        let public_key: [u8; 32] = public_key_bytes
            .try_into()
            .map_err(|_| VerifyError::InvalidFormat("Invalid key length".to_string()))?;

        // Create verifier for this key
        let verifying_key = VerifyingKey::from_bytes(&public_key)
            .map_err(|e| VerifyError::InvalidFormat(e.to_string()))?;

        let verifier = if self.require_origin {
            TokenVerifier::new(verifying_key, &self.issuer, &self.audience)
                .with_origin_validation()
        } else {
            TokenVerifier::new(verifying_key, &self.issuer, &self.audience)
        };

        verifier.verify(token, expected_origin)
    }

    /// Fetch JWKS from a URL
    #[cfg(feature = "jwks")]
    pub async fn fetch_jwks(url: &str) -> Result<Jwks, reqwest::Error> {
        let response = reqwest::get(url).await?;
        let jwks: Jwks = response.json().await?;
        Ok(jwks)
    }
}

/// Convert signing key to PKCS#8 DER format for jsonwebtoken
fn _signing_key_to_pkcs8_der(_key: &SigningKey) -> Vec<u8> {
    // This is a simplified version - in production you'd use proper PKCS#8 encoding
    // For now, we use the raw key bytes with jsonwebtoken's EdDSA support
    vec![]
}

/// HMAC-based verifier for development (not recommended for production)
pub struct HmacVerifier {
    secret: Vec<u8>,
    issuer: String,
    audience: String,
}

impl HmacVerifier {
    /// Create a new HMAC verifier (dev only)
    pub fn new(secret: impl Into<Vec<u8>>, issuer: impl Into<String>, audience: impl Into<String>) -> Self {
        Self {
            secret: secret.into(),
            issuer: issuer.into(),
            audience: audience.into(),
        }
    }

    /// Verify a token using HMAC
    pub fn verify(&self,
        token: &str,
        _expected_origin: Option<&str>,
    ) -> Result<AuthContext, VerifyError> {
        let decoding_key = DecodingKey::from_secret(&self.secret);
        
        let mut validation = Validation::new(Algorithm::HS256);
        validation.set_issuer(&[&self.issuer]);
        validation.set_audience(&[&self.audience]);
        
        let token_data = decode::<SessionClaims>(
            token,
            &decoding_key,
            &validation,
        ).map_err(|e| match e.kind() {
            jsonwebtoken::errors::ErrorKind::ExpiredSignature => VerifyError::Expired,
            jsonwebtoken::errors::ErrorKind::InvalidSignature => VerifyError::InvalidSignature,
            _ => VerifyError::DecodeError(e.to_string()),
        })?;

        Ok(AuthContext::from_claims(token_data.claims))
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
        let context = verifier.verify(&token, None).unwrap();
        
        assert_eq!(context.subject, "test-subject");
        assert_eq!(context.issuer, "test-issuer");
        assert_eq!(context.metering_key, "meter-123");
    }

    #[test]
    fn test_hmac_verification() {
        let secret = b"dev-secret-key";
        let verifier = HmacVerifier::new(secret.to_vec(), "test-issuer", "test-audience");

        // Create a token with jsonwebtoken directly
        let claims = create_test_claims();
        let encoding_key = EncodingKey::from_secret(secret);
        let header = Header::new(Algorithm::HS256);
        let token = encode(&header, &claims, &encoding_key).unwrap();

        // Verify
        let context = verifier.verify(&token, None).unwrap();
        assert_eq!(context.subject, "test-subject");
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
        let result = verifier.verify(&token, None);
        assert!(matches!(result, Err(VerifyError::Expired)));
    }
}
