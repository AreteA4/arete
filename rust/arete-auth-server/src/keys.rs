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
                    key_class: arete_auth::KeyClass::Secret,
                    subject: format!("secret:{}", key_id),
                    metering_key: format!("meter:secret:{}", key_id),
                    allowed_deployments: None, // Secret keys can access all deployments
                    origin_allowlist: None,    // Secret keys don't need origin validation
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
                    key_class: arete_auth::KeyClass::Publishable,
                    subject: format!("publishable:{}", key_id),
                    metering_key: format!("meter:publishable:{}", key_id),
                    allowed_deployments: None, // Can be restricted per key
                    origin_allowlist: None,    // Can be restricted per key
                    rate_limit_tier: RateLimitTier::Medium,
                },
            );
        }

        Self { keys }
    }

    /// Restrict every publishable key to `origins`. An empty list leaves
    /// publishable keys unrestricted.
    pub fn with_publishable_origin_allowlist(mut self, origins: Vec<String>) -> Self {
        if origins.is_empty() {
            return self;
        }
        for info in self.keys.values_mut() {
            if matches!(info.key_class, arete_auth::KeyClass::Publishable) {
                info.origin_allowlist = Some(origins.clone());
            }
        }
        self
    }

    /// Create a new key store with publishable keys that have origin allowlists
    ///
    /// # Arguments
    /// * `secret_keys` - List of secret API keys
    /// * `publishable_keys` - List of (key, origin_allowlist) tuples
    #[allow(dead_code)]
    pub fn with_origin_allowlists(
        secret_keys: Vec<String>,
        publishable_keys: Vec<(String, Vec<String>)>,
    ) -> Self {
        let mut keys = HashMap::new();

        // Add secret keys
        for (idx, key) in secret_keys.iter().enumerate() {
            let key_id = format!("sk_{}", idx);
            keys.insert(
                key.clone(),
                ApiKeyInfo {
                    key_id: key_id.clone(),
                    key_class: arete_auth::KeyClass::Secret,
                    subject: format!("secret:{}", key_id),
                    metering_key: format!("meter:secret:{}", key_id),
                    allowed_deployments: None,
                    origin_allowlist: None,
                    rate_limit_tier: RateLimitTier::High,
                },
            );
        }

        // Add publishable keys with origin allowlists
        for (idx, (key, allowlist)) in publishable_keys.iter().enumerate() {
            let key_id = format!("pk_{}", idx);
            keys.insert(
                key.clone(),
                ApiKeyInfo {
                    key_id: key_id.clone(),
                    key_class: arete_auth::KeyClass::Publishable,
                    subject: format!("publishable:{}", key_id),
                    metering_key: format!("meter:publishable:{}", key_id),
                    allowed_deployments: None,
                    origin_allowlist: Some(allowlist.clone()),
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
        if matches!(key_info.key_class, arete_auth::KeyClass::Secret) {
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

    /// Check if the origin is allowed for the given key
    pub fn authorize_origin(
        &self,
        key_info: &ApiKeyInfo,
        origin: Option<&str>,
    ) -> Result<(), AuthServerError> {
        // Secret keys don't need origin validation
        if matches!(key_info.key_class, arete_auth::KeyClass::Secret) {
            return Ok(());
        }

        // Check if origin allowlist is configured
        if let Some(ref allowed_origins) = key_info.origin_allowlist {
            let origin_str = origin.ok_or(AuthServerError::OriginNotAllowed)?;

            // Normalize origin for comparison
            let origin_normalized = origin_str.trim_end_matches('/').to_lowercase();

            // Check if origin is in allowlist
            let allowed = allowed_origins.iter().any(|allowed| {
                let allowed_normalized = allowed.trim_end_matches('/').to_lowercase();
                // Exact match or subdomain match
                origin_normalized == allowed_normalized
                    || origin_normalized.ends_with(&format!(".{}", allowed_normalized))
            });

            if !allowed {
                return Err(AuthServerError::OriginNotAllowed);
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
        assert!(matches!(info.key_class, arete_auth::KeyClass::Secret));
    }

    #[test]
    fn test_validate_publishable_key() {
        let store = ApiKeyStore::new(vec![], vec!["pub123".to_string()]);
        let info = store.validate_key("pub123").unwrap();
        assert!(matches!(info.key_class, arete_auth::KeyClass::Publishable));
    }

    #[test]
    fn publishable_origin_allowlist_rejects_other_origins() {
        let store = ApiKeyStore::new(vec!["secret123".to_string()], vec!["pub123".to_string()])
            .with_publishable_origin_allowlist(vec!["https://app.example.com".to_string()]);
        let publishable = store.validate_key("pub123").unwrap();

        assert!(store
            .authorize_origin(&publishable, Some("https://app.example.com"))
            .is_ok());
        assert!(matches!(
            store.authorize_origin(&publishable, Some("https://evil.example")),
            Err(AuthServerError::OriginNotAllowed)
        ));
        assert!(matches!(
            store.authorize_origin(&publishable, None),
            Err(AuthServerError::OriginNotAllowed)
        ));

        // Secret keys are server-side and never origin-restricted.
        let secret = store.validate_key("secret123").unwrap();
        assert!(store.authorize_origin(&secret, None).is_ok());
    }

    #[test]
    fn empty_origin_allowlist_leaves_publishable_keys_unrestricted() {
        let store = ApiKeyStore::new(vec![], vec!["pub123".to_string()])
            .with_publishable_origin_allowlist(vec![]);
        let publishable = store.validate_key("pub123").unwrap();
        assert!(store
            .authorize_origin(&publishable, Some("https://anything.example"))
            .is_ok());
        assert!(store.authorize_origin(&publishable, None).is_ok());
    }

    #[test]
    fn test_invalid_key() {
        let store = ApiKeyStore::new(vec![], vec![]);
        assert!(store.validate_key("invalid").is_err());
    }
}
