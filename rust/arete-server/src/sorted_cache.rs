//! Sorted view cache for maintaining ordered entity collections.
//!
//! This module provides incremental maintenance of sorted entity views,
//! enabling efficient windowed subscriptions (take/skip) with minimal
//! recomputation on updates.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};

/// A sortable key that combines the sort value with entity key for stable ordering.
/// Uses (sort_value, entity_key) tuple to ensure deterministic ordering even when
/// sort values are equal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SortKey {
    /// The extracted sort value (as comparable bytes)
    sort_value: SortValue,
    /// Entity key for tie-breaking
    entity_key: String,
    /// Direction to apply to the sort value comparison.
    order: SortOrder,
}

impl PartialOrd for SortKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SortKey {
    fn cmp(&self, other: &Self) -> Ordering {
        if self.order != other.order {
            return match (self.order, other.order) {
                (SortOrder::Asc, SortOrder::Desc) => Ordering::Less,
                (SortOrder::Desc, SortOrder::Asc) => Ordering::Greater,
                _ => Ordering::Equal,
            };
        }

        let sort_order = self.sort_value.cmp(&other.sort_value);
        let sort_order = match (&self.sort_value, &other.sort_value, self.order) {
            (SortValue::Null, _, _) | (_, SortValue::Null, _) | (_, _, SortOrder::Asc) => {
                sort_order
            }
            (_, _, SortOrder::Desc) => sort_order.reverse(),
        };

        match sort_order {
            Ordering::Equal => self.entity_key.cmp(&other.entity_key),
            other => other,
        }
    }
}

/// Comparable sort value extracted from JSON
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SortValue {
    Null,
    Bool(bool),
    Integer(i64),
    Float(OrderedFloat),
    String(String),
}

impl Ord for SortValue {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (SortValue::Null, SortValue::Null) => Ordering::Equal,
            (SortValue::Null, _) => Ordering::Less,
            (_, SortValue::Null) => Ordering::Greater,
            (SortValue::Bool(a), SortValue::Bool(b)) => a.cmp(b),
            (SortValue::Integer(a), SortValue::Integer(b)) => a.cmp(b),
            (SortValue::Float(a), SortValue::Float(b)) => a.cmp(b),
            (SortValue::String(a), SortValue::String(b)) => {
                compare_decimal_strings(a, b).unwrap_or_else(|| a.cmp(b))
            }
            // Cross-type comparisons: numbers < strings
            (SortValue::Integer(_), SortValue::String(_)) => Ordering::Less,
            (SortValue::String(_), SortValue::Integer(_)) => Ordering::Greater,
            (SortValue::Float(_), SortValue::String(_)) => Ordering::Less,
            (SortValue::String(_), SortValue::Float(_)) => Ordering::Greater,
            // Integer vs Float: convert to float
            (SortValue::Integer(a), SortValue::Float(b)) => OrderedFloat(*a as f64).cmp(b),
            (SortValue::Float(a), SortValue::Integer(b)) => a.cmp(&OrderedFloat(*b as f64)),
            // Bool vs others
            (SortValue::Bool(_), _) => Ordering::Less,
            (_, SortValue::Bool(_)) => Ordering::Greater,
        }
    }
}

impl PartialOrd for SortValue {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Wrapper for f64 that implements Ord (treats NaN as less than all values)
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OrderedFloat(pub f64);

impl Eq for OrderedFloat {}

impl Ord for OrderedFloat {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.partial_cmp(&other.0).unwrap_or_else(|| {
            if self.0.is_nan() && other.0.is_nan() {
                Ordering::Equal
            } else if self.0.is_nan() {
                Ordering::Less
            } else {
                Ordering::Greater
            }
        })
    }
}

impl PartialOrd for OrderedFloat {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Sort order for the cache
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SortOrder {
    Asc,
    Desc,
}

impl From<crate::materialized_view::SortOrder> for SortOrder {
    fn from(order: crate::materialized_view::SortOrder) -> Self {
        match order {
            crate::materialized_view::SortOrder::Asc => SortOrder::Asc,
            crate::materialized_view::SortOrder::Desc => SortOrder::Desc,
        }
    }
}

/// Delta representing a change to a client's windowed view
#[derive(Debug, Clone, PartialEq)]
pub enum ViewDelta {
    /// No change to the client's window
    None,
    /// Entity was added to the window
    Add { key: String, entity: Value },
    /// Entity was removed from the window
    Remove { key: String },
    /// Entity in the window was updated
    Update { key: String, entity: Value },
}

/// Sorted view cache maintaining entities in sort order.
///
/// # Bounding
///
/// The cache holds a full copy of every entity it has been given, so callers
/// that feed it from a bounded source (the projector and snapshot restore feed
/// it from the LRU-capped [`EntityCache`](crate::EntityCache)) should use
/// [`upsert_bounded`](Self::upsert_bounded) or
/// [`trim_to_max_entries`](Self::trim_to_max_entries) with that source's cap.
///
/// Entries are evicted from the *bottom* of the sort order, not by recency.
/// Evicting the least-recently-updated entries (mirroring the entity cache)
/// would drop a top-ranked entity that simply has not updated recently and
/// break leaderboard-style `sort` + `take` views. Evicting everything beyond
/// position `max_entries` keeps every window with `skip + take <= max_entries`
/// exact. An evicted entity re-enters on its next update, because the
/// projector re-reads the full entity from the entity cache before upserting.
///
/// Known edge: after entities are removed from (or move down out of) the top
/// of the order, a previously evicted entity that has not updated since is
/// missing from the cache until it next updates, so a window near the cap can
/// under-fill or show a lower-ranked entity in its place until then.
#[derive(Debug)]
pub struct SortedViewCache {
    /// View identifier
    view_id: String,
    /// Field path to sort by (e.g., ["id", "round_id"])
    sort_field: Vec<String>,
    /// Sort order
    order: SortOrder,
    /// Sorted entries: SortKey -> entity_key (for iteration in order)
    sorted: BTreeMap<SortKey, ()>,
    /// Entity data: entity_key -> (SortKey, Value)
    entities: HashMap<String, (SortKey, Value)>,
    /// Ordered keys cache (rebuilt on structural changes)
    keys_cache: Vec<String>,
    /// Whether keys_cache needs rebuild
    cache_dirty: bool,
}

impl SortedViewCache {
    pub fn new(view_id: String, sort_field: Vec<String>, order: SortOrder) -> Self {
        Self {
            view_id,
            sort_field,
            order,
            sorted: BTreeMap::new(),
            entities: HashMap::new(),
            keys_cache: Vec::new(),
            cache_dirty: true,
        }
    }

    pub fn view_id(&self) -> &str {
        &self.view_id
    }

    pub fn len(&self) -> usize {
        self.entities.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    /// Insert or update an entity, returns the position where it was inserted
    pub fn upsert(&mut self, entity_key: String, entity: Value) -> UpsertResult {
        let sort_value = self.extract_sort_value(&entity);

        // Check if entity already exists
        if let Some((old_sort_key, old_entity)) = self.entities.get(&entity_key).cloned() {
            let effective_sort_value = if matches!(sort_value, SortValue::Null)
                && !matches!(old_sort_key.sort_value, SortValue::Null)
            {
                old_sort_key.sort_value.clone()
            } else {
                sort_value
            };

            let new_sort_key = SortKey {
                sort_value: effective_sort_value,
                entity_key: entity_key.clone(),
                order: self.order,
            };

            // Merge incoming entity with existing to preserve fields not in the update
            let merged_entity = Self::deep_merge(old_entity, entity);

            if old_sort_key == new_sort_key {
                // Sort key unchanged - just update entity data
                self.entities
                    .insert(entity_key.clone(), (new_sort_key, merged_entity));
                // Position unchanged, no structural change
                let position = self.find_position(&entity_key);
                return UpsertResult::Updated { position };
            }

            // Sort key changed - need to reposition
            self.sorted.remove(&old_sort_key);
            self.sorted.insert(new_sort_key.clone(), ());
            self.entities
                .insert(entity_key.clone(), (new_sort_key, merged_entity));
            self.cache_dirty = true;

            let position = self.find_position(&entity_key);
            return UpsertResult::Inserted { position };
        }

        let new_sort_key = SortKey {
            sort_value,
            entity_key: entity_key.clone(),
            order: self.order,
        };

        self.sorted.insert(new_sort_key.clone(), ());
        self.entities
            .insert(entity_key.clone(), (new_sort_key, entity));
        self.cache_dirty = true;

        let position = self.find_position(&entity_key);

        UpsertResult::Inserted { position }
    }

    /// Upsert an entity, then evict from the bottom of the sort order so the
    /// cache holds at most `max_entries` entities.
    ///
    /// If the upserted entity itself sorts beyond `max_entries` it is evicted
    /// immediately and the returned position is `>= max_entries`.
    pub fn upsert_bounded(
        &mut self,
        entity_key: String,
        entity: Value,
        max_entries: usize,
    ) -> UpsertResult {
        let result = self.upsert(entity_key, entity);
        self.trim_to_max_entries(max_entries);
        result
    }

    /// Evict entities from the bottom of the sort order until at most
    /// `max_entries` remain. Returns the number of evicted entities.
    ///
    /// Each eviction is an `O(log n)` pop from the end of the ordered index,
    /// so a batch of evictions (e.g. after a bulk rebuild) costs no more than
    /// the inserts that caused it. The ordered-keys cache is truncated in place
    /// when it is current, so trimming does not force a full rebuild.
    pub fn trim_to_max_entries(&mut self, max_entries: usize) -> usize {
        let mut evicted = 0;
        while self.sorted.len() > max_entries {
            let Some((sort_key, ())) = self.sorted.pop_last() else {
                break;
            };
            self.entities.remove(&sort_key.entity_key);
            evicted += 1;
        }
        if evicted > 0 && !self.cache_dirty {
            // Only the tail of the order was removed, so the prefix is still
            // exact.
            self.keys_cache.truncate(self.sorted.len());
        }
        evicted
    }

    fn deep_merge(base: Value, patch: Value) -> Value {
        match (base, patch) {
            (Value::Object(mut base_map), Value::Object(patch_map)) => {
                for (key, patch_value) in patch_map {
                    if let Some(base_value) = base_map.remove(&key) {
                        base_map.insert(key, Self::deep_merge(base_value, patch_value));
                    } else {
                        base_map.insert(key, patch_value);
                    }
                }
                Value::Object(base_map)
            }
            (_, patch) => patch,
        }
    }

    /// Remove an entity, returns the position it was at
    pub fn remove(&mut self, entity_key: &str) -> Option<usize> {
        if let Some((sort_key, _)) = self.entities.remove(entity_key) {
            let position = self.find_position_by_sort_key(&sort_key);
            self.sorted.remove(&sort_key);
            self.cache_dirty = true;
            Some(position)
        } else {
            None
        }
    }

    /// Get entity by key
    pub fn get(&self, entity_key: &str) -> Option<&Value> {
        self.entities.get(entity_key).map(|(_, v)| v)
    }

    /// Get ordered keys (rebuilds cache if dirty)
    pub fn ordered_keys(&mut self) -> &[String] {
        if self.cache_dirty {
            self.rebuild_keys_cache();
        }
        &self.keys_cache
    }

    /// Get a window of entities
    pub fn get_window(&mut self, skip: usize, take: usize) -> Vec<(String, Value)> {
        if self.cache_dirty {
            self.rebuild_keys_cache();
        }

        self.keys_cache
            .iter()
            .skip(skip)
            .take(take)
            .filter_map(|key| {
                self.entities
                    .get(key)
                    .map(|(_, v)| (key.clone(), v.clone()))
            })
            .collect()
    }

    /// Get every entity in deterministic sort order for query-side filtering.
    pub fn get_all_ordered(&mut self) -> Vec<(String, Value)> {
        if self.cache_dirty {
            self.rebuild_keys_cache();
        }

        self.keys_cache
            .iter()
            .filter_map(|key| {
                self.entities
                    .get(key)
                    .map(|(_, value)| (key.clone(), value.clone()))
            })
            .collect()
    }

    /// Compute deltas for a client with a specific window
    pub fn compute_window_deltas(
        &mut self,
        old_window_keys: &[String],
        skip: usize,
        take: usize,
    ) -> Vec<ViewDelta> {
        if self.cache_dirty {
            self.rebuild_keys_cache();
        }

        let new_window_keys: Vec<&String> = self.keys_cache.iter().skip(skip).take(take).collect();

        let old_set: std::collections::HashSet<&String> = old_window_keys.iter().collect();
        let new_set: std::collections::HashSet<&String> = new_window_keys.iter().cloned().collect();

        let mut deltas = Vec::new();

        // Removed from window
        for key in old_set.difference(&new_set) {
            deltas.push(ViewDelta::Remove {
                key: (*key).clone(),
            });
        }

        // Added to window
        for key in new_set.difference(&old_set) {
            if let Some((_, entity)) = self.entities.get(*key) {
                deltas.push(ViewDelta::Add {
                    key: (*key).clone(),
                    entity: entity.clone(),
                });
            }
        }

        deltas
    }

    fn extract_sort_value(&self, entity: &Value) -> SortValue {
        let mut current = entity;
        for segment in &self.sort_field {
            match current.get(segment) {
                Some(v) => current = v,
                None => return SortValue::Null,
            }
        }

        value_to_sort_value(current)
    }

    fn find_position(&self, entity_key: &str) -> usize {
        if let Some((sort_key, _)) = self.entities.get(entity_key) {
            self.find_position_by_sort_key(sort_key)
        } else {
            0
        }
    }

    fn find_position_by_sort_key(&self, sort_key: &SortKey) -> usize {
        self.sorted.range(..sort_key).count()
    }

    fn rebuild_keys_cache(&mut self) {
        self.keys_cache = self.sorted.keys().map(|sk| sk.entity_key.clone()).collect();
        self.cache_dirty = false;
    }
}

/// Result of an upsert operation
#[derive(Debug, Clone, PartialEq)]
pub enum UpsertResult {
    /// Entity was inserted at a new position
    Inserted { position: usize },
    /// Entity was updated (may or may not have moved)
    Updated { position: usize },
}

fn value_to_sort_value(v: &Value) -> SortValue {
    match v {
        Value::Null => SortValue::Null,
        Value::Bool(b) => SortValue::Bool(*b),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                SortValue::Integer(i)
            } else if let Some(f) = n.as_f64() {
                SortValue::Float(OrderedFloat(f))
            } else {
                SortValue::Null
            }
        }
        Value::String(s) => SortValue::String(s.clone()),
        _ => SortValue::Null,
    }
}

fn compare_decimal_strings(left: &str, right: &str) -> Option<Ordering> {
    fn parts(value: &str) -> Option<(bool, &str)> {
        let (negative, digits) = match value.strip_prefix('-') {
            Some(digits) => (true, digits),
            None => (false, value),
        };
        if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }

        let digits = digits.trim_start_matches('0');
        let digits = if digits.is_empty() { "0" } else { digits };
        Some((negative && digits != "0", digits))
    }

    let (left_negative, left_digits) = parts(left)?;
    let (right_negative, right_digits) = parts(right)?;

    match (left_negative, right_negative) {
        (true, false) => Some(Ordering::Less),
        (false, true) => Some(Ordering::Greater),
        _ => {
            let magnitude = left_digits
                .len()
                .cmp(&right_digits.len())
                .then_with(|| left_digits.cmp(right_digits));
            Some(if left_negative {
                magnitude.reverse()
            } else {
                magnitude
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_sorted_cache_basic() {
        let mut cache = SortedViewCache::new(
            "test/latest".to_string(),
            vec!["id".to_string()],
            SortOrder::Desc,
        );

        cache.upsert("a".to_string(), json!({"id": 1, "name": "first"}));
        cache.upsert("b".to_string(), json!({"id": 3, "name": "third"}));
        cache.upsert("c".to_string(), json!({"id": 2, "name": "second"}));

        let keys = cache.ordered_keys();
        // Desc order: 3, 2, 1
        assert_eq!(keys, vec!["b", "c", "a"]);
    }

    #[test]
    fn test_sorted_cache_window() {
        let mut cache = SortedViewCache::new(
            "test/latest".to_string(),
            vec!["id".to_string()],
            SortOrder::Desc,
        );

        for i in 1..=10 {
            cache.upsert(format!("e{}", i), json!({"id": i}));
        }

        // Desc order: 10, 9, 8, 7, 6, 5, 4, 3, 2, 1
        let window = cache.get_window(0, 3);
        assert_eq!(window.len(), 3);
        assert_eq!(window[0].0, "e10");
        assert_eq!(window[1].0, "e9");
        assert_eq!(window[2].0, "e8");

        let window = cache.get_window(3, 3);
        assert_eq!(window[0].0, "e7");
    }

    #[test]
    fn all_ordered_preserves_stable_sort_and_tie_breaking() {
        let mut cache = SortedViewCache::new(
            "test/latest".to_string(),
            vec!["score".to_string()],
            SortOrder::Desc,
        );
        cache.upsert("b".to_string(), json!({"score": 10}));
        cache.upsert("a".to_string(), json!({"score": 10}));
        cache.upsert("c".to_string(), json!({"score": 9}));

        let keys: Vec<_> = cache
            .get_all_ordered()
            .into_iter()
            .map(|(key, _)| key)
            .collect();
        assert_eq!(keys, ["a", "b", "c"]);
    }

    #[test]
    fn test_sorted_cache_update_moves_position() {
        let mut cache = SortedViewCache::new(
            "test/latest".to_string(),
            vec!["score".to_string()],
            SortOrder::Desc,
        );

        cache.upsert("a".to_string(), json!({"score": 10}));
        cache.upsert("b".to_string(), json!({"score": 20}));
        cache.upsert("c".to_string(), json!({"score": 15}));

        // Order: b(20), c(15), a(10)
        assert_eq!(cache.ordered_keys(), vec!["b", "c", "a"]);

        // Update a to have highest score
        cache.upsert("a".to_string(), json!({"score": 25}));

        // New order: a(25), b(20), c(15)
        assert_eq!(cache.ordered_keys(), vec!["a", "b", "c"]);
    }

    #[test]
    fn test_sorted_cache_remove() {
        let mut cache = SortedViewCache::new(
            "test/latest".to_string(),
            vec!["id".to_string()],
            SortOrder::Asc,
        );

        cache.upsert("a".to_string(), json!({"id": 1}));
        cache.upsert("b".to_string(), json!({"id": 2}));
        cache.upsert("c".to_string(), json!({"id": 3}));

        assert_eq!(cache.len(), 3);

        let pos = cache.remove("b");
        assert_eq!(pos, Some(1));
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.ordered_keys(), vec!["a", "c"]);
    }

    #[test]
    fn test_compute_window_deltas() {
        let mut cache = SortedViewCache::new(
            "test/latest".to_string(),
            vec!["id".to_string()],
            SortOrder::Desc,
        );

        // Initial: 5, 4, 3, 2, 1
        for i in 1..=5 {
            cache.upsert(format!("e{}", i), json!({"id": i}));
        }

        let old_window: Vec<String> = vec!["e5".to_string(), "e4".to_string(), "e3".to_string()];

        // Add e6 (new top)
        cache.upsert("e6".to_string(), json!({"id": 6}));

        // New order: 6, 5, 4, 3, 2, 1
        // New top 3: e6, e5, e4
        let deltas = cache.compute_window_deltas(&old_window, 0, 3);

        assert_eq!(deltas.len(), 2);
        // e3 removed from window
        assert!(deltas
            .iter()
            .any(|d| matches!(d, ViewDelta::Remove { key } if key == "e3")));
        // e6 added to window
        assert!(deltas
            .iter()
            .any(|d| matches!(d, ViewDelta::Add { key, .. } if key == "e6")));
    }

    fn keys(cache: &mut SortedViewCache) -> Vec<String> {
        cache.ordered_keys().to_vec()
    }

    #[test]
    fn bounded_upsert_evicts_bottom_of_desc_order() {
        let mut cache = SortedViewCache::new(
            "test/top".to_string(),
            vec!["score".to_string()],
            SortOrder::Desc,
        );

        for i in 1..=10 {
            cache.upsert_bounded(format!("e{i}"), json!({"score": i}), 4);
            assert!(cache.len() <= 4);
        }

        assert_eq!(cache.len(), 4);
        assert_eq!(keys(&mut cache), ["e10", "e9", "e8", "e7"]);
        assert!(cache.get("e1").is_none());
        assert!(cache.get("e6").is_none());
    }

    #[test]
    fn bounded_upsert_evicts_bottom_of_asc_order() {
        let mut cache = SortedViewCache::new(
            "test/bottom".to_string(),
            vec!["score".to_string()],
            SortOrder::Asc,
        );

        for i in (1..=10).rev() {
            cache.upsert_bounded(format!("e{i}"), json!({"score": i}), 4);
            assert!(cache.len() <= 4);
        }

        assert_eq!(cache.len(), 4);
        assert_eq!(keys(&mut cache), ["e1", "e2", "e3", "e4"]);
        assert!(cache.get("e10").is_none());
    }

    #[test]
    fn bounded_upsert_does_not_evict_stale_top_entities() {
        let mut cache = SortedViewCache::new(
            "test/top".to_string(),
            vec!["score".to_string()],
            SortOrder::Desc,
        );

        // The leader is inserted first and never updated again; recency-based
        // eviction would drop it.
        cache.upsert_bounded("leader".to_string(), json!({"score": 1_000}), 3);
        for i in 1..=20 {
            cache.upsert_bounded(format!("e{i}"), json!({"score": i}), 3);
        }

        assert_eq!(keys(&mut cache), ["leader", "e20", "e19"]);
    }

    #[test]
    fn windows_within_cap_match_unbounded_cache() {
        let mut bounded = SortedViewCache::new(
            "test/top".to_string(),
            vec!["score".to_string()],
            SortOrder::Desc,
        );
        let mut unbounded = SortedViewCache::new(
            "test/top".to_string(),
            vec!["score".to_string()],
            SortOrder::Desc,
        );

        // Scores arrive out of order within each round and every entity moves
        // up on each later round. Entities never move down, so nothing
        // evicted can belong back inside the cap (see the edge-case test
        // below for what happens when they do).
        for i in 0..200u64 {
            let key = format!("e{}", i % 60);
            let score = (i / 60) * 1_000 + (i * 37) % 101;
            let entity = json!({"score": score, "n": i});
            bounded.upsert_bounded(key.clone(), entity.clone(), 25);
            unbounded.upsert(key, entity);
        }

        assert_eq!(bounded.len(), 25);
        for (skip, take) in [(0, 25), (0, 10), (5, 20), (24, 1)] {
            assert_eq!(
                bounded.get_window(skip, take),
                unbounded.get_window(skip, take),
                "window skip={skip} take={take}"
            );
        }
    }

    #[test]
    fn evicted_entity_is_missing_after_top_moves_down_until_it_updates() {
        let mut cache = SortedViewCache::new(
            "test/top".to_string(),
            vec!["score".to_string()],
            SortOrder::Desc,
        );

        for i in 1..=4 {
            cache.upsert_bounded(format!("e{i}"), json!({"score": i}), 3);
        }
        assert_eq!(keys(&mut cache), ["e4", "e3", "e2"]);

        // The leader drops to the bottom; e1 would now rank third but was
        // evicted and has not updated, so e4 holds third place instead.
        cache.upsert_bounded("e4".to_string(), json!({"score": 0}), 3);
        assert_eq!(keys(&mut cache), ["e3", "e2", "e4"]);

        // Once e1 updates it re-enters at its correct position.
        cache.upsert_bounded("e1".to_string(), json!({"score": 1}), 3);
        assert_eq!(keys(&mut cache), ["e3", "e2", "e1"]);
    }

    #[test]
    fn evicted_entity_reenters_when_upserted_again() {
        let mut cache = SortedViewCache::new(
            "test/top".to_string(),
            vec!["score".to_string()],
            SortOrder::Desc,
        );

        for i in 1..=5 {
            cache.upsert_bounded(format!("e{i}"), json!({"score": i}), 3);
        }
        assert!(cache.get("e1").is_none());

        let result = cache.upsert_bounded("e1".to_string(), json!({"score": 100}), 3);
        assert_eq!(result, UpsertResult::Inserted { position: 0 });
        assert_eq!(keys(&mut cache), ["e1", "e5", "e4"]);
        assert_eq!(cache.len(), 3);
    }

    #[test]
    fn upsert_below_full_cap_is_evicted_immediately() {
        let mut cache = SortedViewCache::new(
            "test/top".to_string(),
            vec!["score".to_string()],
            SortOrder::Desc,
        );

        for i in 10..=12 {
            cache.upsert_bounded(format!("e{i}"), json!({"score": i}), 3);
        }
        let result = cache.upsert_bounded("low".to_string(), json!({"score": 1}), 3);

        assert_eq!(result, UpsertResult::Inserted { position: 3 });
        assert!(cache.get("low").is_none());
        assert_eq!(keys(&mut cache), ["e12", "e11", "e10"]);
    }

    #[test]
    fn trim_keeps_keys_cache_and_entities_consistent() {
        let mut cache = SortedViewCache::new(
            "test/top".to_string(),
            vec!["score".to_string()],
            SortOrder::Desc,
        );

        for i in 1..=10 {
            cache.upsert(format!("e{i}"), json!({"score": i}));
        }
        // Build the keys cache so the trim takes the in-place truncate path.
        assert_eq!(cache.ordered_keys().len(), 10);

        assert_eq!(cache.trim_to_max_entries(4), 6);
        assert_eq!(cache.trim_to_max_entries(4), 0);
        assert_eq!(cache.len(), 4);
        assert_eq!(keys(&mut cache), ["e10", "e9", "e8", "e7"]);
        assert_eq!(cache.get_all_ordered().len(), 4);
        assert_eq!(cache.remove("e7"), Some(3));
        assert_eq!(keys(&mut cache), ["e10", "e9", "e8"]);
    }

    #[test]
    fn test_nested_sort_field() {
        let mut cache = SortedViewCache::new(
            "test/latest".to_string(),
            vec!["id".to_string(), "round_id".to_string()],
            SortOrder::Desc,
        );

        cache.upsert("a".to_string(), json!({"id": {"round_id": 1}}));
        cache.upsert("b".to_string(), json!({"id": {"round_id": 3}}));
        cache.upsert("c".to_string(), json!({"id": {"round_id": 2}}));

        let keys = cache.ordered_keys();
        assert_eq!(keys, vec!["b", "c", "a"]);
    }

    #[test]
    fn test_nested_decimal_string_sort_field() {
        let mut cache = SortedViewCache::new(
            "test/latest".to_string(),
            vec!["id".to_string(), "round_id".to_string()],
            SortOrder::Desc,
        );

        cache.upsert("9".to_string(), json!({"id": {"round_id": "9"}}));
        cache.upsert("100".to_string(), json!({"id": {"round_id": "100"}}));
        cache.upsert("10".to_string(), json!({"id": {"round_id": "10"}}));

        assert_eq!(cache.ordered_keys(), vec!["100", "10", "9"]);
    }

    #[test]
    fn test_descending_string_sort_field() {
        let mut cache = SortedViewCache::new(
            "test/latest".to_string(),
            vec!["name".to_string()],
            SortOrder::Desc,
        );

        cache.upsert("a".to_string(), json!({"name": "alpha"}));
        cache.upsert("c".to_string(), json!({"name": "charlie"}));
        cache.upsert("b".to_string(), json!({"name": "bravo"}));

        assert_eq!(cache.ordered_keys(), vec!["c", "b", "a"]);
    }

    #[test]
    fn test_update_with_missing_sort_field_preserves_position() {
        let mut cache = SortedViewCache::new(
            "test/latest".to_string(),
            vec!["id".to_string(), "round_id".to_string()],
            SortOrder::Desc,
        );

        cache.upsert(
            "100".to_string(),
            json!({"id": {"round_id": 100}, "data": "initial"}),
        );
        cache.upsert(
            "200".to_string(),
            json!({"id": {"round_id": 200}, "data": "initial"}),
        );
        cache.upsert(
            "300".to_string(),
            json!({"id": {"round_id": 300}, "data": "initial"}),
        );

        assert_eq!(cache.ordered_keys(), vec!["300", "200", "100"]);

        cache.upsert("200".to_string(), json!({"data": "updated_without_id"}));

        assert_eq!(
            cache.ordered_keys(),
            vec!["300", "200", "100"],
            "Entity 200 should retain its position even when updated without sort field"
        );

        let entity = cache.get("200").unwrap();
        assert_eq!(entity["data"], "updated_without_id");
    }

    #[test]
    fn test_new_entity_with_missing_sort_field_gets_null_position() {
        let mut cache = SortedViewCache::new(
            "test/latest".to_string(),
            vec!["id".to_string(), "round_id".to_string()],
            SortOrder::Desc,
        );

        cache.upsert("100".to_string(), json!({"id": {"round_id": 100}}));
        cache.upsert("200".to_string(), json!({"id": {"round_id": 200}}));

        cache.upsert("new".to_string(), json!({"data": "no_sort_field"}));

        let keys = cache.ordered_keys();
        assert_eq!(
            keys.first().unwrap(),
            "new",
            "New entity without sort field gets Null which sorts first (Null < any value)"
        );
    }
}
