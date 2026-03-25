use hyperstack_sdk::deep_merge_with_append;
use serde_json::Value;
use std::collections::{HashMap, VecDeque};

const DEFAULT_MAX_HISTORY: usize = 1000;

pub struct EntityStore {
    entities: HashMap<String, EntityRecord>,
    max_history: usize,
}

pub struct EntityRecord {
    pub current: Value,
    pub history: VecDeque<HistoryEntry>,
}

#[derive(Clone)]
pub struct HistoryEntry {
    pub seq: Option<String>,
    pub op: String,
    pub state: Value,
    pub patch: Option<Value>,
}

#[allow(dead_code)]
impl EntityStore {
    pub fn new() -> Self {
        Self {
            entities: HashMap::new(),
            max_history: DEFAULT_MAX_HISTORY,
        }
    }

    pub fn entity_count(&self) -> usize {
        self.entities.len()
    }

    pub fn get(&self, key: &str) -> Option<&EntityRecord> {
        self.entities.get(key)
    }

    /// Apply an upsert/create operation. Returns the full entity state.
    pub fn upsert(&mut self, key: &str, data: Value, op: &str, seq: Option<String>) -> &Value {
        let record = self.entities.entry(key.to_string()).or_insert_with(|| {
            EntityRecord {
                current: Value::Null,
                history: VecDeque::new(),
            }
        });

        record.current = data.clone();
        record.history.push_back(HistoryEntry {
            seq,
            op: op.to_string(),
            state: data,
            patch: None,
        });

        if record.history.len() > self.max_history {
            record.history.pop_front();
        }

        &record.current
    }

    /// Apply a patch operation. Returns the merged entity state.
    pub fn patch(
        &mut self,
        key: &str,
        patch_data: &Value,
        append_paths: &[String],
        seq: Option<String>,
    ) -> &Value {
        let record = self.entities.entry(key.to_string()).or_insert_with(|| {
            EntityRecord {
                current: serde_json::json!({}),
                history: VecDeque::new(),
            }
        });

        let raw_patch = patch_data.clone();
        deep_merge_with_append(&mut record.current, patch_data, append_paths, "");

        record.history.push_back(HistoryEntry {
            seq,
            op: "patch".to_string(),
            state: record.current.clone(),
            patch: Some(raw_patch),
        });

        if record.history.len() > self.max_history {
            record.history.pop_front();
        }

        &record.current
    }

    /// Mark an entity as deleted, retaining its history for post-stream analysis.
    pub fn delete(&mut self, key: &str) {
        if let Some(record) = self.entities.get_mut(key) {
            let deleted_state = serde_json::json!({"_deleted": true});
            record.history.push_back(HistoryEntry {
                seq: None,
                op: "delete".to_string(),
                state: deleted_state.clone(),
                patch: None,
            });
            record.current = deleted_state;
            if record.history.len() > self.max_history {
                record.history.pop_front();
            }
        }
    }

    /// Get entity state at a specific history index (0 = latest).
    pub fn at(&self, key: &str, index: usize) -> Option<&HistoryEntry> {
        let record = self.entities.get(key)?;
        if index >= record.history.len() {
            return None;
        }
        let actual_idx = record.history.len().checked_sub(index.checked_add(1)?)?;
        record.history.get(actual_idx)
    }

    /// Get entity state at an absolute VecDeque index.
    pub fn at_absolute(&self, key: &str, abs_idx: usize) -> Option<&HistoryEntry> {
        let record = self.entities.get(key)?;
        record.history.get(abs_idx)
    }

    /// Get the history length for a key.
    pub fn history_len(&self, key: &str) -> usize {
        self.entities.get(key).map(|r| r.history.len()).unwrap_or(0)
    }

    /// Get the diff between two consecutive history entries.
    /// Returns (added/changed fields, removed fields).
    pub fn diff_at(&self, key: &str, index: usize) -> Option<Value> {
        let record = self.entities.get(key)?;
        if record.history.is_empty() {
            return None;
        }

        let actual_idx = record.history.len().checked_sub(index.checked_add(1)?)?;
        let entry = record.history.get(actual_idx)?;

        // If this entry has a raw patch, use it directly
        if let Some(patch) = &entry.patch {
            return Some(serde_json::json!({
                "op": entry.op,
                "index": index,
                "total": record.history.len(),
                "patch": patch,
                "state": entry.state,
            }));
        }

        // Otherwise diff against previous state
        let previous = if actual_idx > 0 {
            &record.history.get(actual_idx - 1)?.state
        } else {
            &Value::Null
        };

        let changes = compute_diff(previous, &entry.state);
        Some(serde_json::json!({
            "op": entry.op,
            "index": index,
            "total": record.history.len(),
            "changes": changes,
            "state": entry.state,
        }))
    }

    /// Get the full history for an entity as a JSON array.
    /// Entries are ordered newest-first. The `index` field matches `at(key, index)`.
    pub fn history(&self, key: &str) -> Option<Value> {
        let record = self.entities.get(key)?;
        let entries: Vec<Value> = record
            .history
            .iter()
            .enumerate()
            .rev()
            .map(|(i, entry)| {
                let rev_idx = record.history.len() - 1 - i;
                serde_json::json!({
                    "index": rev_idx,
                    "op": entry.op,
                    "seq": entry.seq,
                    "state": entry.state,
                })
            })
            .collect();
        Some(Value::Array(entries))
    }
}

/// Compute a shallow (top-level only) diff between two JSON values.
/// For nested objects, reports the entire sub-object as changed. Patch operations
/// use the raw patch instead of this diff, so this only affects upsert/snapshot history.
fn compute_diff(old: &Value, new: &Value) -> Value {
    match (old, new) {
        (Value::Object(old_map), Value::Object(new_map)) => {
            let mut diff = serde_json::Map::new();

            for (key, new_val) in new_map {
                match old_map.get(key) {
                    Some(old_val) if old_val != new_val => {
                        diff.insert(
                            key.clone(),
                            serde_json::json!({
                                "from": old_val,
                                "to": new_val,
                            }),
                        );
                    }
                    None => {
                        diff.insert(
                            key.clone(),
                            serde_json::json!({
                                "added": new_val,
                            }),
                        );
                    }
                    _ => {}
                }
            }

            for key in old_map.keys() {
                if !new_map.contains_key(key) {
                    diff.insert(
                        key.clone(),
                        serde_json::json!({
                            "removed": old_map.get(key),
                        }),
                    );
                }
            }

            Value::Object(diff)
        }
        _ if old != new => {
            serde_json::json!({
                "from": old,
                "to": new,
            })
        }
        _ => Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_upsert_and_history() {
        let mut store = EntityStore::new();
        store.upsert("k1", json!({"a": 1}), "upsert", None);
        store.upsert("k1", json!({"a": 2}), "upsert", None);

        assert_eq!(store.get("k1").unwrap().current, json!({"a": 2}));
        assert_eq!(store.get("k1").unwrap().history.len(), 2);

        let at0 = store.at("k1", 0).unwrap();
        assert_eq!(at0.state, json!({"a": 2}));

        let at1 = store.at("k1", 1).unwrap();
        assert_eq!(at1.state, json!({"a": 1}));
    }

    #[test]
    fn test_patch() {
        let mut store = EntityStore::new();
        store.upsert("k1", json!({"a": 1, "b": 2}), "upsert", None);
        store.patch("k1", &json!({"a": 10}), &[], None);

        assert_eq!(store.get("k1").unwrap().current, json!({"a": 10, "b": 2}));
        assert_eq!(store.get("k1").unwrap().history.len(), 2);
    }

    #[test]
    fn test_diff() {
        let mut store = EntityStore::new();
        store.upsert("k1", json!({"a": 1, "b": 2}), "upsert", None);
        store.patch("k1", &json!({"a": 10}), &[], None);

        let diff = store.diff_at("k1", 0).unwrap();
        // Latest entry is a patch, so it should include the raw patch
        assert_eq!(diff["patch"], json!({"a": 10}));
    }

    #[test]
    fn test_delete() {
        let mut store = EntityStore::new();
        store.upsert("k1", json!({"a": 1}), "upsert", None);
        store.delete("k1");
        // Entity is retained with tombstone for history access
        let record = store.get("k1").expect("deleted entity should be retained");
        assert_eq!(record.current, json!({"_deleted": true}));
        assert_eq!(record.history.len(), 2); // upsert + delete
        assert_eq!(record.history.back().unwrap().op, "delete");
    }

    #[test]
    fn test_compute_diff() {
        let old = json!({"a": 1, "b": 2, "c": 3});
        let new = json!({"a": 1, "b": 5, "d": 4});
        let diff = compute_diff(&old, &new);

        assert!(diff.get("a").is_none()); // unchanged
        assert_eq!(diff["b"]["from"], json!(2));
        assert_eq!(diff["b"]["to"], json!(5));
        assert_eq!(diff["c"]["removed"], json!(3));
        assert_eq!(diff["d"]["added"], json!(4));
    }
}
