//! Materialized view evaluation for view pipelines.
//!
//! This module handles the runtime evaluation of ViewDef pipelines,
//! maintaining materialized results that update as source data changes.

use crate::cache::EntityCache;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Result of evaluating whether an update affects a materialized view
#[derive(Debug, Clone, PartialEq)]
pub enum ViewEffect {
    /// Update does not affect the view result
    NoEffect,
    /// Entity should be added to the view result
    Add { key: String },
    /// Entity should be removed from the view result
    Remove { key: String },
    /// Entity in view was updated
    Update { key: String },
    /// Entity replaces another in the view (for single-result views)
    Replace { old_key: String, new_key: String },
}

/// Sort order for view evaluation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortOrder {
    Asc,
    Desc,
}

/// Comparison operators
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompareOp {
    Eq,
    Ne,
    Gt,
    Gte,
    Lt,
    Lte,
}

/// A materialized view that tracks a subset of entities based on a pipeline
#[derive(Debug)]
pub struct MaterializedView {
    /// View identifier
    pub id: String,
    /// Source view/entity this derives from
    pub source_id: String,
    /// Current set of entity keys in this view's result
    current_keys: Arc<RwLock<HashSet<String>>>,
    /// Pipeline configuration (simplified for now)
    pipeline: ViewPipeline,
}

#[derive(Debug, Clone, Default)]
pub struct ViewPipeline {
    /// Filter predicate (field path, op, value)
    pub filter: Option<FilterConfig>,
    /// Sort configuration
    pub sort: Option<SortConfig>,
    /// Limit (take N) - if Some(1), treated as single-result view for Replace effects
    pub limit: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct FilterConfig {
    pub field_path: Vec<String>,
    pub op: CompareOp,
    pub value: Value,
}

#[derive(Debug, Clone)]
pub struct SortConfig {
    pub field_path: Vec<String>,
    pub order: SortOrder,
}

impl MaterializedView {
    /// Create a new materialized view
    pub fn new(id: String, source_id: String, pipeline: ViewPipeline) -> Self {
        Self {
            id,
            source_id,
            current_keys: Arc::new(RwLock::new(HashSet::new())),
            pipeline,
        }
    }

    /// Get current keys in the view
    pub async fn get_keys(&self) -> HashSet<String> {
        self.current_keys.read().await.clone()
    }

    /// Evaluate initial state from cache
    pub async fn evaluate_initial(&self, cache: &EntityCache) -> Vec<(String, Value)> {
        let entities = cache.get_all(&self.source_id).await;
        self.evaluate_pipeline(entities).await
    }

    /// Evaluate pipeline on a set of entities
    async fn evaluate_pipeline(&self, mut entities: Vec<(String, Value)>) -> Vec<(String, Value)> {
        // Apply filter
        if let Some(ref filter) = self.pipeline.filter {
            entities.retain(|(_, v)| self.matches_filter(v, filter));
        }

        // Apply sort
        if let Some(ref sort) = self.pipeline.sort {
            entities.sort_by(|(_, a), (_, b)| {
                let a_val = extract_field(a, &sort.field_path);
                let b_val = extract_field(b, &sort.field_path);
                let cmp = compare_values(&a_val, &b_val);
                match sort.order {
                    SortOrder::Asc => cmp,
                    SortOrder::Desc => cmp.reverse(),
                }
            });
        }

        // Apply limit
        if let Some(limit) = self.pipeline.limit {
            entities.truncate(limit);
        }

        // Update current keys
        let keys: HashSet<String> = entities.iter().map(|(k, _)| k.clone()).collect();
        *self.current_keys.write().await = keys;

        entities
    }

    /// Check if an entity matches the filter
    fn matches_filter(&self, entity: &Value, filter: &FilterConfig) -> bool {
        let field_val = extract_field(entity, &filter.field_path);
        match filter.op {
            CompareOp::Eq => field_val == filter.value,
            CompareOp::Ne => field_val != filter.value,
            CompareOp::Gt => {
                compare_values(&field_val, &filter.value) == std::cmp::Ordering::Greater
            }
            CompareOp::Gte => compare_values(&field_val, &filter.value) != std::cmp::Ordering::Less,
            CompareOp::Lt => compare_values(&field_val, &filter.value) == std::cmp::Ordering::Less,
            CompareOp::Lte => {
                compare_values(&field_val, &filter.value) != std::cmp::Ordering::Greater
            }
        }
    }

    /// Determine the effect of an entity update on this view
    pub async fn compute_effect(
        &self,
        key: &str,
        new_value: Option<&Value>,
        _cache: &EntityCache,
    ) -> ViewEffect {
        let current_keys = self.current_keys.read().await;
        let was_in_view = current_keys.contains(key);
        drop(current_keys);

        // Check if entity now matches filter
        let matches_now = match new_value {
            Some(v) => {
                if let Some(ref filter) = self.pipeline.filter {
                    self.matches_filter(v, filter)
                } else {
                    true
                }
            }
            None => false, // Deleted
        };

        match (was_in_view, matches_now) {
            (false, true) => {
                if self.pipeline.limit == Some(1) {
                    let current_keys = self.current_keys.read().await;
                    if let Some(current_key) = current_keys.iter().next() {
                        if current_key != key {
                            return ViewEffect::Replace {
                                old_key: current_key.clone(),
                                new_key: key.to_string(),
                            };
                        }
                    }
                }
                ViewEffect::Add {
                    key: key.to_string(),
                }
            }
            (true, false) => ViewEffect::Remove {
                key: key.to_string(),
            },
            (true, true) => ViewEffect::Update {
                key: key.to_string(),
            },
            (false, false) => ViewEffect::NoEffect,
        }
    }

    /// Apply an effect to update the current keys
    pub async fn apply_effect(&self, effect: &ViewEffect) {
        let mut keys = self.current_keys.write().await;
        match effect {
            ViewEffect::Add { key } => {
                keys.insert(key.clone());
            }
            ViewEffect::Remove { key } => {
                keys.remove(key);
            }
            ViewEffect::Replace { old_key, new_key } => {
                keys.remove(old_key);
                keys.insert(new_key.clone());
            }
            ViewEffect::Update { .. } | ViewEffect::NoEffect => {}
        }
    }
}

/// Extract a field value from a JSON object using a path
fn extract_field(value: &Value, path: &[String]) -> Value {
    let mut current = value;
    for segment in path {
        match current.get(segment) {
            Some(v) => current = v,
            None => return Value::Null,
        }
    }
    current.clone()
}

/// Compare two JSON values
fn compare_values(a: &Value, b: &Value) -> std::cmp::Ordering {
    match (a, b) {
        (Value::Number(a), Value::Number(b)) => {
            let a_f = a.as_f64().unwrap_or(0.0);
            let b_f = b.as_f64().unwrap_or(0.0);
            a_f.partial_cmp(&b_f).unwrap_or(std::cmp::Ordering::Equal)
        }
        (Value::String(a), Value::String(b)) => a.cmp(b),
        (Value::Bool(a), Value::Bool(b)) => a.cmp(b),
        _ => std::cmp::Ordering::Equal,
    }
}

/// Registry of materialized views
#[derive(Default)]
pub struct MaterializedViewRegistry {
    views: HashMap<String, Arc<MaterializedView>>,
    /// Map from source view ID to dependent materialized views
    dependencies: HashMap<String, Vec<String>>,
}

impl MaterializedViewRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a materialized view
    pub fn register(&mut self, view: MaterializedView) {
        let view_id = view.id.clone();
        let source_id = view.source_id.clone();

        self.dependencies
            .entry(source_id)
            .or_default()
            .push(view_id.clone());

        self.views.insert(view_id, Arc::new(view));
    }

    /// Get a materialized view by ID
    pub fn get(&self, id: &str) -> Option<Arc<MaterializedView>> {
        self.views.get(id).cloned()
    }

    /// Get all views that depend on a source
    pub fn get_dependents(&self, source_id: &str) -> Vec<Arc<MaterializedView>> {
        self.dependencies
            .get(source_id)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| self.views.get(id).cloned())
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_filter_evaluation() {
        let pipeline = ViewPipeline {
            filter: Some(FilterConfig {
                field_path: vec!["status".to_string()],
                op: CompareOp::Eq,
                value: json!("active"),
            }),
            sort: None,
            limit: None,
        };

        let view =
            MaterializedView::new("test/active".to_string(), "test/list".to_string(), pipeline);

        let entities = vec![
            ("1".to_string(), json!({"status": "active", "value": 10})),
            ("2".to_string(), json!({"status": "inactive", "value": 20})),
            ("3".to_string(), json!({"status": "active", "value": 30})),
        ];

        let result = view.evaluate_pipeline(entities).await;
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].0, "1");
        assert_eq!(result[1].0, "3");
    }

    #[tokio::test]
    async fn test_sort_and_limit() {
        let pipeline = ViewPipeline {
            filter: None,
            sort: Some(SortConfig {
                field_path: vec!["value".to_string()],
                order: SortOrder::Desc,
            }),
            limit: Some(2),
        };

        let view =
            MaterializedView::new("test/top2".to_string(), "test/list".to_string(), pipeline);

        let entities = vec![
            ("1".to_string(), json!({"value": 10})),
            ("2".to_string(), json!({"value": 30})),
            ("3".to_string(), json!({"value": 20})),
        ];

        let result = view.evaluate_pipeline(entities).await;
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].0, "2"); // value: 30
        assert_eq!(result[1].0, "3"); // value: 20
    }
}
