//! Token revocation support
//!
//! Provides functionality to revoke tokens before their natural expiry.
//! Revoked tokens are tracked by their JWT ID (jti) claim.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;

/// A revoked token entry with expiration tracking
#[derive(Debug, Clone)]
struct RevokedEntry {
    expires_at: u64,
}

/// Token revocation list with automatic cleanup
#[derive(Clone)]
pub struct TokenRevocationList {
    /// Revoked JWT IDs keyed by JTI with expiry tracking
    revoked: Arc<RwLock<HashMap<String, RevokedEntry>>>,
    /// Fallback retention window when the token expiry is unavailable
    max_age: Duration,
}

impl TokenRevocationList {
    /// Create a new empty revocation list
    pub fn new() -> Self {
        Self {
            revoked: Arc::new(RwLock::new(HashMap::new())),
            max_age: Duration::from_secs(86400), // 24 hours default
        }
    }

    /// Set the maximum age of revocation entries
    pub fn with_max_age(mut self, max_age: Duration) -> Self {
        self.max_age = max_age;
        self
    }

    /// Revoke a token by its JTI using `max_age` as a fallback expiry.
    pub async fn revoke(&self, jti: impl Into<String>) {
        let expires_at = current_unix_timestamp().saturating_add(self.max_age.as_secs());
        self.revoke_until(jti, expires_at).await;
    }

    /// Revoke a token by its JTI until the token naturally expires.
    pub async fn revoke_until(&self, jti: impl Into<String>, expires_at: u64) {
        let mut revoked = self.revoked.write().await;
        revoked.insert(jti.into(), RevokedEntry { expires_at });
    }

    /// Check if a token is revoked
    pub async fn is_revoked(&self, jti: &str) -> bool {
        let revoked = self.revoked.read().await;
        revoked.contains_key(jti)
    }

    /// Remove a token from the revocation list
    pub async fn unrevoke(&self, jti: &str) {
        let mut revoked = self.revoked.write().await;
        revoked.remove(jti);
    }

    /// Get the number of revoked tokens
    pub async fn len(&self) -> usize {
        let revoked = self.revoked.read().await;
        revoked.len()
    }

    /// Check if the revocation list is empty
    pub async fn is_empty(&self) -> bool {
        let revoked = self.revoked.read().await;
        revoked.is_empty()
    }

    /// Clear all revoked tokens
    pub async fn clear(&self) {
        let mut revoked = self.revoked.write().await;
        revoked.clear();
    }

    /// Clean up old revocation entries (should be called periodically)
    pub async fn cleanup_expired(&self, now: u64) -> usize {
        let mut revoked = self.revoked.write().await;
        let before = revoked.len();
        revoked.retain(|_, entry| entry.expires_at > now);
        before - revoked.len()
    }
}

fn current_unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time should not be before epoch")
        .as_secs()
}

impl Default for TokenRevocationList {
    fn default() -> Self {
        Self::new()
    }
}

/// Revocation checker trait for integration with verifiers
#[async_trait::async_trait]
pub trait RevocationChecker: Send + Sync {
    /// Check if a token is revoked
    async fn is_revoked(&self, jti: &str) -> bool;
}

#[async_trait::async_trait]
impl RevocationChecker for TokenRevocationList {
    async fn is_revoked(&self, jti: &str) -> bool {
        self.is_revoked(jti).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn test_revoke_and_check() {
        let list = TokenRevocationList::new();

        assert!(!list.is_revoked("token-1").await);

        list.revoke("token-1").await;
        assert!(list.is_revoked("token-1").await);

        list.unrevoke("token-1").await;
        assert!(!list.is_revoked("token-1").await);
    }

    #[tokio::test]
    async fn test_multiple_tokens() {
        let list = TokenRevocationList::new();

        list.revoke("token-1").await;
        list.revoke("token-2").await;

        assert!(list.is_revoked("token-1").await);
        assert!(list.is_revoked("token-2").await);
        assert!(!list.is_revoked("token-3").await);

        assert_eq!(list.len().await, 2);
    }

    #[tokio::test]
    async fn test_clear() {
        let list = TokenRevocationList::new();

        list.revoke("token-1").await;
        list.revoke("token-2").await;

        list.clear().await;

        assert!(list.is_empty().await);
        assert!(!list.is_revoked("token-1").await);
    }

    #[tokio::test]
    async fn test_cleanup_expired_removes_expired_entries() {
        let list = TokenRevocationList::new().with_max_age(Duration::from_secs(60));

        list.revoke_until("expired-token", 100).await;
        list.revoke_until("active-token", 200).await;

        let removed = list.cleanup_expired(150).await;

        assert_eq!(removed, 1);
        assert!(!list.is_revoked("expired-token").await);
        assert!(list.is_revoked("active-token").await);
    }
}
