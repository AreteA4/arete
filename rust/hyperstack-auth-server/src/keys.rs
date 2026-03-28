use std::collections::HashMap;

use crate::error::AuthServerError;
use crate::models::{ApiKeyInfo, RateLimitTier};

/// Simple in-memory API key store
///
/// In production, this would be backed by a database
pub struct ApiKeyStore {
    keys: HashMap<String, ApiKeyInfo>,
}

impl ApiKeyStore {
    /// Create a new key store with the given secret and publishable keys
    pub fn new(secret_keys: Vec<String>, publishable_keys: Vec<String>) -> Self {
        let mut keys = HashMap::new();

        // Add secret keys
        for (idx, key) in secret_keys.iter().enumerate() {
            let key_id = format!("sk_{}", idx);
            keys.insert(
                key.clone(),
                ApiKeyInfo {
                    key_id: key_id.clone(),
                    key_class: hyperstack_auth::KeyClass::Secret,
                    subject: format!("secret:{}", key_id),
                    metering_key: format!("meter:secret:{}", key_id),
                    allowed_deployments: None, // Secret keys can access all deployments
                    rate_limit_tier: RateLimitTier::High,
                },
            );
        }

        // Add publishable keys
        for (idx, key) in publishable_keys.iter().enumerate() {
            let key_id = format!("pk_{}", idx);
            keys.insert(
                key.clone(),
                ApiKeyInfo {
                    key_id: key_id.clone(),
                    key_class: hyperstack_auth::KeyClass::Publishable,
                    subject: format!("publishable:{}", key_id),
                    metering_key: format!("meter:publishable:{}", key_id),
                    allowed_deployments: None, // Can be restricted per key
                    rate_limit_tier: RateLimitTier::Medium,
                },
            );
        }

        Self { keys }
    }

    /// Validate an API key and return its info
    pub fn validate_key(&self, key: &str) -> Result<ApiKeyInfo, AuthServerError> {
        self.keys
            .get(key)
            .cloned()
            .ok_or(AuthServerError::InvalidApiKey)
    }

    /// Check if a key is authorized for a deployment
    pub fn authorize_deployment(
        &self,
        key_info: &ApiKeyInfo,
        deployment_id: &str,
    ) -> Result<(), AuthServerError> {
        // Secret keys can access all deployments
        if matches!(key_info.key_class, hyperstack_auth::KeyClass::Secret) {
            return Ok(());
        }

        // Check if deployment is in allowed list
        if let Some(ref allowed) = key_info.allowed_deployments {
            if !allowed.contains(&deployment_id.to_string()) {
                return Err(AuthServerError::UnauthorizedDeployment);
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_secret_key() {
        let store = ApiKeyStore::new(vec!["secret123".to_string()], vec![]);
        let info = store.validate_key("secret123").unwrap();
        assert!(matches!(info.key_class, hyperstack_auth::KeyClass::Secret));
    }

    #[test]
    fn test_validate_publishable_key() {
        let store = ApiKeyStore::new(vec![], vec!["pub123".to_string()]);
        let info = store.validate_key("pub123").unwrap();
        assert!(matches!(
            info.key_class,
            hyperstack_auth::KeyClass::Publishable
        ));
    }

    #[test]
    fn test_invalid_key() {
        let store = ApiKeyStore::new(vec![], vec![]);
        assert!(store.validate_key("invalid").is_err());
    }
}
