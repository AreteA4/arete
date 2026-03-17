//! Shared slot hash cache accessible from both server and interpreter
//! 
//! This module provides a global cache for slot hashes that is populated
//! by the gRPC stream and accessed by computed field resolvers.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Global slot hash cache
static SLOT_HASH_CACHE: once_cell::sync::Lazy<Arc<RwLock<HashMap<u64, String>>>> =
    once_cell::sync::Lazy::new(|| Arc::new(RwLock::new(HashMap::new())));

/// Maximum number of slot hashes to keep in cache (prevent unbounded growth)
const MAX_CACHE_SIZE: usize = 50000;

/// Record a slot hash in the global cache
pub async fn record_slot_hash(slot: u64, slot_hash: String) {
    let mut cache: tokio::sync::RwLockWriteGuard<'_, HashMap<u64, String>> = SLOT_HASH_CACHE.write().await;
    cache.insert(slot, slot_hash);
    
    // Prune old entries if cache is too large
    if cache.len() > MAX_CACHE_SIZE {
        // Remove oldest 25% of entries
        let slots_to_remove: Vec<u64> = cache
            .keys()
            .take(cache.len() / 4)
            .copied()
            .collect();
        for slot in slots_to_remove {
            cache.remove(&slot);
        }
    }
}

/// Get a slot hash from the global cache
pub async fn get_slot_hash(slot: u64) -> Option<String> {
    let cache: tokio::sync::RwLockReadGuard<'_, HashMap<u64, String>> = SLOT_HASH_CACHE.read().await;
    cache.get(&slot).cloned()
}

/// Check if a slot hash is in the cache
pub async fn has_slot_hash(slot: u64) -> bool {
    let cache: tokio::sync::RwLockReadGuard<'_, HashMap<u64, String>> = SLOT_HASH_CACHE.read().await;
    cache.contains_key(&slot)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_slot_hash_cache() {
        record_slot_hash(100, "test_hash".to_string()).await;
        assert_eq!(get_slot_hash(100).await, Some("test_hash".to_string()));
        assert_eq!(get_slot_hash(101).await, None);
    }
}