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
}

impl PartialOrd for SortKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SortKey {
    fn cmp(&self, other: &Self) -> Ordering {
        match self.sort_value.cmp(&other.sort_value) {
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
            (SortValue::String(a), SortValue::String(b)) => a.cmp(b),
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

/// Sorted view cache maintaining entities in sort order
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
        };

        self.sorted.insert(new_sort_key.clone(), ());
        self.entities
            .insert(entity_key.clone(), (new_sort_key, entity));
        self.cache_dirty = true;

        let position = self.find_position(&entity_key);

        UpsertResult::Inserted { position }
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

        match self.order {
            SortOrder::Asc => value_to_sort_value(current),
            SortOrder::Desc => value_to_sort_value_desc(current),
        }
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

fn value_to_sort_value_desc(v: &Value) -> SortValue {
    match v {
        Value::Null => SortValue::Null,
        Value::Bool(b) => SortValue::Bool(!*b),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                SortValue::Integer(-i)
            } else if let Some(f) = n.as_f64() {
                SortValue::Float(OrderedFloat(-f))
            } else {
                SortValue::Null
            }
        }
        Value::String(s) => {
            // For desc strings, we'd need a more complex approach
            // For now, just negate won't work for strings
            // We'll handle this at the comparison level instead
            SortValue::String(s.clone())
        }
        _ => SortValue::Null,
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
