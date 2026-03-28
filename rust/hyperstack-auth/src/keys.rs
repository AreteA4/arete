use crate::error::AuthError;
use ed25519_dalek::{
    Signature, Signer, SigningKey as EdSigningKey, Verifier, VerifyingKey as EdVerifyingKey,
};
use std::fs;
use std::path::Path;

/// A signing key for token issuance
#[derive(Debug, Clone)]
pub struct SigningKey {
    inner: EdSigningKey,
}

impl SigningKey {
    /// Generate a new random signing key
    pub fn generate() -> Self {
        use rand::rngs::OsRng;
        use rand::RngCore;
        let mut bytes = [0u8; 32];
        OsRng.fill_bytes(&mut bytes);
        Self {
            inner: EdSigningKey::from_bytes(&bytes),
        }
    }

    /// Load from raw bytes (32-byte seed)
    pub fn from_bytes(bytes: &[u8; 32]) -> Self {
        Self {
            inner: EdSigningKey::from_bytes(bytes),
        }
    }

    /// Get the corresponding verifying key
    pub fn verifying_key(&self) -> VerifyingKey {
        VerifyingKey {
            inner: self.inner.verifying_key(),
        }
    }

    /// Sign a message
    pub fn sign(&self, message: &[u8]) -> Signature {
        self.inner.sign(message)
    }

    /// Export to bytes
    pub fn to_bytes(&self) -> [u8; 32] {
        self.inner.to_bytes()
    }

    /// Export to keypair bytes (64 bytes: 32 secret + 32 public)
    pub fn to_keypair_bytes(&self) -> [u8; 64] {
        self.inner.to_keypair_bytes()
    }

    /// Load from keypair bytes
    pub fn from_keypair_bytes(bytes: &[u8; 64]) -> Result<Self, AuthError> {
        let key = EdSigningKey::from_keypair_bytes(bytes)
            .map_err(|e| AuthError::InvalidKeyFormat(format!("Invalid keypair: {:?}", e)))?;
        Ok(Self { inner: key })
    }
}

/// A verifying key for token verification
#[derive(Debug, Clone)]
pub struct VerifyingKey {
    pub(crate) inner: EdVerifyingKey,
}

impl VerifyingKey {
    /// Load from raw bytes (32-byte public key)
    pub fn from_bytes(bytes: &[u8; 32]) -> Result<Self, AuthError> {
        let key = EdVerifyingKey::from_bytes(bytes)
            .map_err(|e| AuthError::InvalidKeyFormat(format!("Invalid public key: {:?}", e)))?;
        Ok(Self { inner: key })
    }

    /// Verify a signature
    pub fn verify(&self, message: &[u8], signature: &Signature) -> Result<(), AuthError> {
        self.inner
            .verify(message, signature)
            .map_err(|e| AuthError::InvalidKeyFormat(format!("Verification failed: {:?}", e)))
    }

    /// Get raw bytes
    pub fn to_bytes(&self) -> [u8; 32] {
        self.inner.to_bytes()
    }
}

/// Key loader for different sources
pub struct KeyLoader;

impl KeyLoader {
    /// Load signing key from environment variable (base64-encoded bytes)
    pub fn signing_key_from_env(var_name: &str) -> Result<SigningKey, AuthError> {
        let b64 = std::env::var(var_name).map_err(|_| {
            AuthError::KeyLoadingFailed(format!("Environment variable {} not set", var_name))
        })?;
        let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &b64)
            .map_err(|e| AuthError::InvalidKeyFormat(format!("Invalid base64: {}", e)))?;
        let key_bytes: [u8; 32] = bytes
            .try_into()
            .map_err(|_| AuthError::InvalidKeyFormat("Invalid key length".to_string()))?;
        Ok(SigningKey::from_bytes(&key_bytes))
    }

    /// Load verifying key from environment variable (base64-encoded bytes)
    pub fn verifying_key_from_env(var_name: &str) -> Result<VerifyingKey, AuthError> {
        let b64 = std::env::var(var_name).map_err(|_| {
            AuthError::KeyLoadingFailed(format!("Environment variable {} not set", var_name))
        })?;
        let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &b64)
            .map_err(|e| AuthError::InvalidKeyFormat(format!("Invalid base64: {}", e)))?;
        let key_bytes: [u8; 32] = bytes
            .try_into()
            .map_err(|_| AuthError::InvalidKeyFormat("Invalid key length".to_string()))?;
        VerifyingKey::from_bytes(&key_bytes)
    }

    /// Generate and save a new key pair to files (base64-encoded)
    pub fn generate_and_save_keys(
        signing_key_path: impl AsRef<Path>,
        verifying_key_path: impl AsRef<Path>,
    ) -> Result<(SigningKey, VerifyingKey), AuthError> {
        let signing_key = SigningKey::generate();
        let verifying_key = signing_key.verifying_key();

        // Save signing key (base64)
        let signing_b64 = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            signing_key.to_bytes(),
        );
        fs::write(signing_key_path, signing_b64)?;

        // Save verifying key (base64)
        let verifying_b64 = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            verifying_key.to_bytes(),
        );
        fs::write(verifying_key_path, verifying_b64)?;

        Ok((signing_key, verifying_key))
    }

    /// Load signing key from file (base64-encoded)
    pub fn signing_key_from_file(path: impl AsRef<Path>) -> Result<SigningKey, AuthError> {
        let b64 = fs::read_to_string(path)?;
        let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &b64)
            .map_err(|e| AuthError::InvalidKeyFormat(format!("Invalid base64: {}", e)))?;
        let key_bytes: [u8; 32] = bytes
            .try_into()
            .map_err(|_| AuthError::InvalidKeyFormat("Invalid key length".to_string()))?;
        Ok(SigningKey::from_bytes(&key_bytes))
    }

    /// Load verifying key from file (base64-encoded)
    pub fn verifying_key_from_file(path: impl AsRef<Path>) -> Result<VerifyingKey, AuthError> {
        let b64 = fs::read_to_string(path)?;
        let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &b64)
            .map_err(|e| AuthError::InvalidKeyFormat(format!("Invalid base64: {}", e)))?;
        let key_bytes: [u8; 32] = bytes
            .try_into()
            .map_err(|_| AuthError::InvalidKeyFormat("Invalid key length".to_string()))?;
        VerifyingKey::from_bytes(&key_bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_and_sign() {
        let signing_key = SigningKey::generate();
        let message = b"test message";
        let signature = signing_key.sign(message);

        let verifying_key = signing_key.verifying_key();
        assert!(verifying_key.verify(message, &signature).is_ok());
    }

    #[test]
    fn test_bytes_roundtrip() {
        let signing_key = SigningKey::generate();
        let bytes = signing_key.to_bytes();

        let loaded = SigningKey::from_bytes(&bytes);
        assert_eq!(
            signing_key.verifying_key().to_bytes(),
            loaded.verifying_key().to_bytes()
        );
    }

    #[test]
    fn test_keypair_bytes_roundtrip() {
        let signing_key = SigningKey::generate();
        let keypair_bytes = signing_key.to_keypair_bytes();

        let loaded = SigningKey::from_keypair_bytes(&keypair_bytes).unwrap();
        assert_eq!(
            signing_key.verifying_key().to_bytes(),
            loaded.verifying_key().to_bytes()
        );
    }
}
