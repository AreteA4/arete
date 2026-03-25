//! Entity cache for snapshot-on-subscribe functionality.
//!
//! This module provides an `EntityCache` that maintains full projected entities
//! in memory with LRU eviction. When a new client subscribes, they receive
//! cached snapshots immediately rather than waiting for the next live mutation.

use lru::LruCache;
use serde_json::Value;
use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::sync::Arc;
use tokio::sync::RwLock;

const DEFAULT_MAX_ENTITIES_PER_VIEW: usize = 500;
const DEFAULT_MAX_ARRAY_LENGTH: usize = 100;
const DEFAULT_INITIAL_SNAPSHOT_BATCH_SIZE: usize = 50;
const DEFAULT_SUBSEQUENT_SNAPSHOT_BATCH_SIZE: usize = 100;

/// Compare two `_seq` values numerically.
/// `_seq` format is "{slot}:{offset}" where slot is not zero-padded.
/// This handles digit-count boundaries correctly (e.g., 99999999 < 100000000).
pub fn cmp_seq(a: &str, b: &str) -> std::cmp::Ordering {
    fn parse(s: &str) -> (u64, u64) {
        let mut parts = s.splitn(2, ':');
        let slot = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
        let offset = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
        (slot, offset)
    }
    parse(a).cmp(&parse(b))
}

/// Configuration for the entity cache
#[derive(Debug, Clone)]
pub struct EntityCacheConfig {
    /// Maximum number of entities to cache per view
    pub max_entities_per_view: usize,
    /// Maximum array length before oldest elements are evicted
    pub max_array_length: usize,
    /// Number of entities to send in the first snapshot batch (for fast initial render)
    pub initial_snapshot_batch_size: usize,
    /// Number of entities to send in subsequent snapshot batches
    pub subsequent_snapshot_batch_size: usize,
}

impl Default for EntityCacheConfig {
    fn default() -> Self {
        Self {
            max_entities_per_view: DEFAULT_MAX_ENTITIES_PER_VIEW,
            max_array_length: DEFAULT_MAX_ARRAY_LENGTH,
            initial_snapshot_batch_size: DEFAULT_INITIAL_SNAPSHOT_BATCH_SIZE,
            subsequent_snapshot_batch_size: DEFAULT_SUBSEQUENT_SNAPSHOT_BATCH_SIZE,
        }
    }
}

/// Entity cache that maintains full projected entities with LRU eviction.
///
/// The cache is populated as mutations flow through the projector, regardless
/// of subscriber state. When a new subscriber connects, they receive snapshots
/// of all cached entities for their requested view.
#[derive(Clone)]
pub struct EntityCache {
    /// view_id -> LRU<entity_key, full_projected_entity>
    caches: Arc<RwLock<HashMap<String, LruCache<String, Value>>>>,
    config: EntityCacheConfig,
}

impl EntityCache {
    /// Create a new entity cache with default configuration
    pub fn new() -> Self {
        Self::with_config(EntityCacheConfig::default())
    }

    /// Create a new entity cache with custom configuration
    pub fn with_config(config: EntityCacheConfig) -> Self {
        Self {
            caches: Arc::new(RwLock::new(HashMap::new())),
            config,
        }
    }

    pub async fn upsert(&self, view_id: &str, key: &str, patch: Value) {
        self.upsert_with_append(view_id, key, patch, &[]).await;
    }

    pub async fn upsert_with_append(
        &self,
        view_id: &str,
        key: &str,
        patch: Value,
        append_paths: &[String],
    ) {
        let mut caches = self.caches.write().await;

        let cache = caches.entry(view_id.to_string()).or_insert_with(|| {
            LruCache::new(
                NonZeroUsize::new(self.config.max_entities_per_view)
                    .expect("max_entities_per_view must be > 0"),
            )
        });

        let max_array_length = self.config.max_array_length;

        if let Some(entity) = cache.get_mut(key) {
            deep_merge_with_append(entity, patch, append_paths, max_array_length);
        } else {
            let new_entity = truncate_arrays_if_needed(patch, max_array_length);
            cache.put(key.to_string(), new_entity);
        }
    }

    /// Get all cached entities for a view.
    ///
    /// Returns a vector of (key, entity) pairs for sending as snapshots
    /// to new subscribers.
    pub async fn get_all(&self, view_id: &str) -> Vec<(String, Value)> {
        let caches = self.caches.read().await;

        caches
            .get(view_id)
            .map(|cache| cache.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default()
    }

    /// Get entities with _seq greater than the provided cursor.
    ///
    /// Returns entities that have been updated after the given cursor,
    /// sorted by _seq in ascending order. Useful for resuming from
    /// a specific point in the stream.
    pub async fn get_after(
        &self,
        view_id: &str,
        cursor: &str,
        limit: Option<usize>,
    ) -> Vec<(String, Value)> {
        let caches = self.caches.read().await;

        if let Some(cache) = caches.get(view_id) {
            let mut results: Vec<(String, Value)> = cache
                .iter()
                .filter(|(_, entity)| {
                    entity
                        .get("_seq")
                        .and_then(|s| s.as_str())
                        .map(|seq| cmp_seq(seq, cursor) == std::cmp::Ordering::Greater)
                        .unwrap_or(false)
                })
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();

            // Sort by _seq (ascending)
            results.sort_by(|a, b| {
                let seq_a = a.1.get("_seq").and_then(|s| s.as_str()).unwrap_or("");
                let seq_b = b.1.get("_seq").and_then(|s| s.as_str()).unwrap_or("");
                cmp_seq(seq_a, seq_b)
            });

            // Apply limit if provided
            if let Some(limit) = limit {
                results.truncate(limit);
            }

            results
        } else {
            vec![]
        }
    }

    /// Get a specific entity from the cache
    pub async fn get(&self, view_id: &str, key: &str) -> Option<Value> {
        let caches = self.caches.read().await;
        caches
            .get(view_id)
            .and_then(|cache| cache.peek(key).cloned())
    }

    /// Get the number of cached entities for a view
    pub async fn len(&self, view_id: &str) -> usize {
        let caches = self.caches.read().await;
        caches.get(view_id).map(|c| c.len()).unwrap_or(0)
    }

    /// Check if the cache for a view is empty
    pub async fn is_empty(&self, view_id: &str) -> bool {
        self.len(view_id).await == 0
    }

    /// Get the snapshot batch configuration
    pub fn snapshot_config(&self) -> SnapshotBatchConfig {
        SnapshotBatchConfig {
            initial_batch_size: self.config.initial_snapshot_batch_size,
            subsequent_batch_size: self.config.subsequent_snapshot_batch_size,
        }
    }

    /// Clear all cached entities for a view
    pub async fn clear(&self, view_id: &str) {
        let mut caches = self.caches.write().await;
        if let Some(cache) = caches.get_mut(view_id) {
            cache.clear();
        }
    }

    pub async fn clear_all(&self) {
        let mut caches = self.caches.write().await;
        caches.clear();
    }

    pub async fn stats(&self) -> CacheStats {
        let caches = self.caches.read().await;
        let mut total_entities = 0;
        let mut views = Vec::new();

        for (view_id, cache) in caches.iter() {
            let count = cache.len();
            total_entities += count;
            views.push((view_id.clone(), count));
        }

        views.sort_by(|a, b| b.1.cmp(&a.1));

        CacheStats {
            view_count: caches.len(),
            total_entities,
            top_views: views.into_iter().take(5).collect(),
        }
    }
}

#[derive(Debug)]
pub struct CacheStats {
    pub view_count: usize,
    pub total_entities: usize,
    pub top_views: Vec<(String, usize)>,
}

#[derive(Debug, Clone, Copy)]
pub struct SnapshotBatchConfig {
    pub initial_batch_size: usize,
    pub subsequent_batch_size: usize,
}

impl Default for EntityCache {
    fn default() -> Self {
        Self::new()
    }
}

fn deep_merge_with_append(
    base: &mut Value,
    patch: Value,
    append_paths: &[String],
    max_array_length: usize,
) {
    deep_merge_with_append_inner(base, patch, append_paths, "", max_array_length);
}

fn deep_merge_with_append_inner(
    base: &mut Value,
    patch: Value,
    append_paths: &[String],
    current_path: &str,
    max_array_length: usize,
) {
    match (base, patch) {
        (Value::Object(base_map), Value::Object(patch_map)) => {
            for (key, patch_value) in patch_map {
                let child_path = if current_path.is_empty() {
                    key.clone()
                } else {
                    format!("{}.{}", current_path, key)
                };

                if let Some(base_value) = base_map.get_mut(&key) {
                    deep_merge_with_append_inner(
                        base_value,
                        patch_value,
                        append_paths,
                        &child_path,
                        max_array_length,
                    );
                } else {
                    base_map.insert(
                        key,
                        truncate_arrays_if_needed(patch_value, max_array_length),
                    );
                }
            }
        }

        (Value::Array(base_arr), Value::Array(patch_arr)) => {
            let should_append = append_paths.iter().any(|p| p == current_path);
            if should_append {
                base_arr.extend(patch_arr);
                if base_arr.len() > max_array_length {
                    let excess = base_arr.len() - max_array_length;
                    base_arr.drain(0..excess);
                }
            } else {
                *base_arr = patch_arr;
                if base_arr.len() > max_array_length {
                    let excess = base_arr.len() - max_array_length;
                    base_arr.drain(0..excess);
                }
            }
        }

        (base, patch_value) => {
            *base = truncate_arrays_if_needed(patch_value, max_array_length);
        }
    }
}

/// Recursively truncate any arrays in a value to the max length
fn truncate_arrays_if_needed(value: Value, max_array_length: usize) -> Value {
    match value {
        Value::Array(mut arr) => {
            // Truncate this array if needed
            if arr.len() > max_array_length {
                let excess = arr.len() - max_array_length;
                arr.drain(0..excess);
            }
            // Recursively process elements
            Value::Array(
                arr.into_iter()
                    .map(|v| truncate_arrays_if_needed(v, max_array_length))
                    .collect(),
            )
        }
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(k, v)| (k, truncate_arrays_if_needed(v, max_array_length)))
                .collect(),
        ),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_basic_upsert_and_get() {
        let cache = EntityCache::new();

        cache
            .upsert("tokens/list", "abc123", json!({"name": "Test Token"}))
            .await;

        let entity = cache.get("tokens/list", "abc123").await;
        assert!(entity.is_some());
        assert_eq!(entity.unwrap()["name"], "Test Token");
    }

    #[tokio::test]
    async fn test_deep_merge_objects() {
        let cache = EntityCache::new();

        cache
            .upsert(
                "tokens/list",
                "abc123",
                json!({
                    "id": "abc123",
                    "metrics": {"volume": 100}
                }),
            )
            .await;

        cache
            .upsert(
                "tokens/list",
                "abc123",
                json!({
                    "metrics": {"trades": 50}
                }),
            )
            .await;

        let entity = cache.get("tokens/list", "abc123").await.unwrap();
        assert_eq!(entity["id"], "abc123");
        assert_eq!(entity["metrics"]["volume"], 100);
        assert_eq!(entity["metrics"]["trades"], 50);
    }

    #[tokio::test]
    async fn test_array_append() {
        let cache = EntityCache::new();

        cache
            .upsert(
                "tokens/list",
                "abc123",
                json!({
                    "events": [{"type": "buy", "amount": 100}]
                }),
            )
            .await;

        cache
            .upsert_with_append(
                "tokens/list",
                "abc123",
                json!({
                    "events": [{"type": "sell", "amount": 50}]
                }),
                &["events".to_string()],
            )
            .await;

        let entity = cache.get("tokens/list", "abc123").await.unwrap();
        let events = entity["events"].as_array().unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0]["type"], "buy");
        assert_eq!(events[1]["type"], "sell");
    }

    #[tokio::test]
    async fn test_array_lru_eviction() {
        let config = EntityCacheConfig {
            max_entities_per_view: 1000,
            max_array_length: 3,
            ..Default::default()
        };
        let cache = EntityCache::with_config(config);

        cache
            .upsert(
                "tokens/list",
                "abc123",
                json!({
                    "events": [
                        {"id": 1}, {"id": 2}, {"id": 3}, {"id": 4}, {"id": 5}
                    ]
                }),
            )
            .await;

        let entity = cache.get("tokens/list", "abc123").await.unwrap();
        let events = entity["events"].as_array().unwrap();

        assert_eq!(events.len(), 3);
        assert_eq!(events[0]["id"], 3);
        assert_eq!(events[1]["id"], 4);
        assert_eq!(events[2]["id"], 5);
    }

    #[tokio::test]
    async fn test_array_append_with_lru() {
        let config = EntityCacheConfig {
            max_entities_per_view: 1000,
            max_array_length: 3,
            ..Default::default()
        };
        let cache = EntityCache::with_config(config);

        cache
            .upsert(
                "tokens/list",
                "abc123",
                json!({
                    "events": [{"id": 1}, {"id": 2}]
                }),
            )
            .await;

        cache
            .upsert_with_append(
                "tokens/list",
                "abc123",
                json!({
                    "events": [{"id": 3}, {"id": 4}]
                }),
                &["events".to_string()],
            )
            .await;

        let entity = cache.get("tokens/list", "abc123").await.unwrap();
        let events = entity["events"].as_array().unwrap();

        // [1,2] + [3,4] = [1,2,3,4] → LRU(3) = [2,3,4]
        assert_eq!(events.len(), 3);
        assert_eq!(events[0]["id"], 2);
        assert_eq!(events[1]["id"], 3);
        assert_eq!(events[2]["id"], 4);
    }

    #[tokio::test]
    async fn test_entity_lru_eviction() {
        let config = EntityCacheConfig {
            max_entities_per_view: 2,
            max_array_length: 100,
            ..Default::default()
        };
        let cache = EntityCache::with_config(config);

        cache.upsert("tokens/list", "key1", json!({"id": 1})).await;
        cache.upsert("tokens/list", "key2", json!({"id": 2})).await;
        cache.upsert("tokens/list", "key3", json!({"id": 3})).await;

        assert!(cache.get("tokens/list", "key1").await.is_none());
        assert!(cache.get("tokens/list", "key2").await.is_some());
        assert!(cache.get("tokens/list", "key3").await.is_some());
    }

    #[tokio::test]
    async fn test_get_all() {
        let cache = EntityCache::new();

        cache.upsert("tokens/list", "key1", json!({"id": 1})).await;
        cache.upsert("tokens/list", "key2", json!({"id": 2})).await;

        let all = cache.get_all("tokens/list").await;
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn test_separate_views() {
        let cache = EntityCache::new();

        cache
            .upsert("tokens/list", "key1", json!({"type": "token"}))
            .await;
        cache
            .upsert("games/list", "key1", json!({"type": "game"}))
            .await;

        let token = cache.get("tokens/list", "key1").await.unwrap();
        let game = cache.get("games/list", "key1").await.unwrap();

        assert_eq!(token["type"], "token");
        assert_eq!(game["type"], "game");
    }

    #[test]
    fn test_deep_merge_with_append() {
        let mut base = json!({
            "a": 1,
            "b": {"c": 2},
            "arr": [1, 2]
        });

        let patch = json!({
            "b": {"d": 3},
            "arr": [3],
            "e": 4
        });

        deep_merge_with_append(&mut base, patch, &["arr".to_string()], 100);

        assert_eq!(base["a"], 1);
        assert_eq!(base["b"]["c"], 2);
        assert_eq!(base["b"]["d"], 3);
        assert_eq!(base["arr"].as_array().unwrap().len(), 3);
        assert_eq!(base["e"], 4);
    }

    #[test]
    fn test_deep_merge_replace_array() {
        let mut base = json!({
            "arr": [1, 2, 3]
        });

        let patch = json!({
            "arr": [4, 5]
        });

        deep_merge_with_append(&mut base, patch, &[], 100);

        assert_eq!(base["arr"].as_array().unwrap().len(), 2);
        assert_eq!(base["arr"][0], 4);
        assert_eq!(base["arr"][1], 5);
    }

    #[test]
    fn test_deep_merge_nested_append() {
        let mut base = json!({
            "stats": {"events": [1, 2]}
        });

        let patch = json!({
            "stats": {"events": [3]}
        });

        deep_merge_with_append(&mut base, patch, &["stats.events".to_string()], 100);

        assert_eq!(base["stats"]["events"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn test_snapshot_config_defaults() {
        let cache = EntityCache::new();
        let config = cache.snapshot_config();

        assert_eq!(config.initial_batch_size, 50);
        assert_eq!(config.subsequent_batch_size, 100);
    }

    #[test]
    fn test_snapshot_config_custom() {
        let config = EntityCacheConfig {
            initial_snapshot_batch_size: 25,
            subsequent_snapshot_batch_size: 75,
            ..Default::default()
        };
        let cache = EntityCache::with_config(config);
        let snapshot_config = cache.snapshot_config();

        assert_eq!(snapshot_config.initial_batch_size, 25);
        assert_eq!(snapshot_config.subsequent_batch_size, 75);
    }

    #[tokio::test]
    async fn test_get_after() {
        let cache = EntityCache::new();

        // Insert entities with _seq values
        cache
            .upsert(
                "tokens/list",
                "key1",
                json!({"id": 1, "_seq": "100:000000000001"}),
            )
            .await;
        cache
            .upsert(
                "tokens/list",
                "key2",
                json!({"id": 2, "_seq": "100:000000000002"}),
            )
            .await;
        cache
            .upsert(
                "tokens/list",
                "key3",
                json!({"id": 3, "_seq": "100:000000000003"}),
            )
            .await;
        cache
            .upsert(
                "tokens/list",
                "key4",
                json!({"id": 4, "_seq": "101:000000000001"}),
            )
            .await;

        // Get all entities after "100:000000000002"
        let after = cache
            .get_after("tokens/list", "100:000000000002", None)
            .await;

        // Should return key3 and key4 (sorted by _seq)
        assert_eq!(after.len(), 2);
        assert_eq!(after[0].0, "key3");
        assert_eq!(after[1].0, "key4");
    }

    #[tokio::test]
    async fn test_get_after_with_limit() {
        let cache = EntityCache::new();

        // Insert entities with _seq values
        cache
            .upsert(
                "tokens/list",
                "key1",
                json!({"id": 1, "_seq": "100:000000000001"}),
            )
            .await;
        cache
            .upsert(
                "tokens/list",
                "key2",
                json!({"id": 2, "_seq": "100:000000000002"}),
            )
            .await;
        cache
            .upsert(
                "tokens/list",
                "key3",
                json!({"id": 3, "_seq": "100:000000000003"}),
            )
            .await;

        // Get entities after "100:000000000000" with limit 2
        let after = cache
            .get_after("tokens/list", "100:000000000000", Some(2))
            .await;

        // Should return only first 2 (key1 and key2)
        assert_eq!(after.len(), 2);
        assert_eq!(after[0].0, "key1");
        assert_eq!(after[1].0, "key2");
    }

    #[tokio::test]
    async fn test_get_after_empty_result() {
        let cache = EntityCache::new();

        cache
            .upsert(
                "tokens/list",
                "key1",
                json!({"id": 1, "_seq": "100:000000000001"}),
            )
            .await;

        // Get entities after a future cursor
        let after = cache
            .get_after("tokens/list", "999:000000000000", None)
            .await;

        assert!(after.is_empty());
    }

    #[tokio::test]
    async fn test_get_after_missing_seq() {
        let cache = EntityCache::new();

        // Insert entity without _seq
        cache.upsert("tokens/list", "key1", json!({"id": 1})).await;

        // Get entities after any cursor - entity without _seq should not be included
        let after = cache.get_after("tokens/list", "0:000000000000", None).await;

        assert!(after.is_empty());
    }
}
