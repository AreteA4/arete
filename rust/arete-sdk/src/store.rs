use crate::frame::{
    parse_snapshot_entities, Frame, Operation, SortConfig, SortOrder, SubscribedFrame,
};
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::Arc;
use tokio::sync::{broadcast, watch, RwLock};

/// Default maximum number of entries per view before LRU eviction kicks in.
/// Set to 10,000 to provide a reasonable balance between memory usage and data retention.
pub const DEFAULT_MAX_ENTRIES_PER_VIEW: usize = 10_000;

/// Configuration for the SharedStore.
#[derive(Debug, Clone)]
pub struct StoreConfig {
    /// Maximum number of entries to keep per view. When exceeded, oldest entries
    /// are evicted using LRU (Least Recently Used) strategy.
    /// Set to `None` to disable size limiting (not recommended for long-running clients).
    pub max_entries_per_view: Option<usize>,
}

impl Default for StoreConfig {
    fn default() -> Self {
        Self {
            max_entries_per_view: Some(DEFAULT_MAX_ENTRIES_PER_VIEW),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SortKey {
    sort_value: SortValue,
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

#[derive(Debug, Clone, PartialEq, Eq)]
enum SortValue {
    Null,
    Bool(bool),
    Integer(i64),
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
            (SortValue::String(a), SortValue::String(b)) => a.cmp(b),
            (SortValue::Integer(_), SortValue::String(_)) => Ordering::Less,
            (SortValue::String(_), SortValue::Integer(_)) => Ordering::Greater,
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

fn extract_sort_value(entity: &Value, field_path: &[String]) -> SortValue {
    let mut current = entity;
    for segment in field_path {
        match current.get(segment) {
            Some(v) => current = v,
            None => return SortValue::Null,
        }
    }

    match current {
        Value::Null => SortValue::Null,
        Value::Bool(b) => SortValue::Bool(*b),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                SortValue::Integer(i)
            } else if let Some(f) = n.as_f64() {
                SortValue::Integer(f as i64)
            } else {
                SortValue::Null
            }
        }
        Value::String(s) => SortValue::String(s.clone()),
        _ => SortValue::Null,
    }
}

struct ViewData {
    entities: HashMap<String, serde_json::Value>,
    access_order: VecDeque<String>,
    sort_config: Option<SortConfig>,
    sorted_keys: BTreeMap<SortKey, ()>,
}

pub fn deep_merge_with_append(
    target: &mut Value,
    patch: &Value,
    append_paths: &[String],
    current_path: &str,
) {
    match (target, patch) {
        (Value::Object(target_map), Value::Object(patch_map)) => {
            for (key, patch_value) in patch_map {
                let field_path = if current_path.is_empty() {
                    key.clone()
                } else {
                    format!("{}.{}", current_path, key)
                };
                match target_map.get_mut(key) {
                    Some(target_value) => {
                        deep_merge_with_append(target_value, patch_value, append_paths, &field_path)
                    }
                    None => {
                        target_map.insert(key.clone(), patch_value.clone());
                    }
                }
            }
        }
        (Value::Array(target_arr), Value::Array(patch_arr))
            if append_paths.contains(&current_path.to_string()) =>
        {
            target_arr.extend(patch_arr.iter().cloned());
        }
        (target, patch) => {
            *target = patch.clone();
        }
    }
}

#[derive(Debug, Clone)]
pub struct StoreUpdate {
    pub view: String,
    pub key: String,
    pub operation: Operation,
    pub data: Option<serde_json::Value>,
    pub previous: Option<serde_json::Value>,
    /// The raw patch data for Patch operations (before merging into full state).
    /// This allows consumers to see exactly what fields changed without diffing.
    pub patch: Option<serde_json::Value>,
}

pub struct SharedStore {
    views: Arc<RwLock<HashMap<String, ViewData>>>,
    view_configs: Arc<RwLock<HashMap<String, SortConfig>>>,
    updates_tx: broadcast::Sender<StoreUpdate>,
    ready_views: Arc<RwLock<HashSet<String>>>,
    ready_tx: watch::Sender<HashSet<String>>,
    ready_rx: watch::Receiver<HashSet<String>>,
    config: StoreConfig,
}

impl ViewData {
    fn new() -> Self {
        Self {
            entities: HashMap::new(),
            access_order: VecDeque::new(),
            sort_config: None,
            sorted_keys: BTreeMap::new(),
        }
    }

    fn with_sort_config(sort_config: SortConfig) -> Self {
        Self {
            entities: HashMap::new(),
            access_order: VecDeque::new(),
            sort_config: Some(sort_config),
            sorted_keys: BTreeMap::new(),
        }
    }

    fn set_sort_config(&mut self, config: SortConfig) {
        if self.sort_config.is_some() {
            return;
        }
        self.sort_config = Some(config);
        self.rebuild_sorted_keys();
    }

    fn rebuild_sorted_keys(&mut self) {
        self.sorted_keys.clear();
        if let Some(ref config) = self.sort_config {
            for (key, value) in &self.entities {
                let sort_value = extract_sort_value(value, &config.field);
                let sort_key = SortKey {
                    sort_value,
                    entity_key: key.clone(),
                };
                self.sorted_keys.insert(sort_key, ());
            }
        }
        self.access_order.clear();
    }

    fn touch(&mut self, key: &str) {
        if self.sort_config.is_some() {
            return;
        }
        self.access_order.retain(|k| k != key);
        self.access_order.push_back(key.to_string());
    }

    fn insert(&mut self, key: String, value: serde_json::Value) {
        if let Some(ref config) = self.sort_config {
            if let Some(old_value) = self.entities.get(&key) {
                let old_sort_value = extract_sort_value(old_value, &config.field);
                let old_sort_key = SortKey {
                    sort_value: old_sort_value,
                    entity_key: key.clone(),
                };
                self.sorted_keys.remove(&old_sort_key);
            }

            let sort_value = extract_sort_value(&value, &config.field);
            let sort_key = SortKey {
                sort_value,
                entity_key: key.clone(),
            };
            self.sorted_keys.insert(sort_key, ());
        } else if !self.entities.contains_key(&key) {
            self.access_order.push_back(key.clone());
        } else {
            self.touch(&key);
        }
        self.entities.insert(key, value);
    }

    fn remove(&mut self, key: &str) -> Option<serde_json::Value> {
        if let Some(ref config) = self.sort_config {
            if let Some(value) = self.entities.get(key) {
                let sort_value = extract_sort_value(value, &config.field);
                let sort_key = SortKey {
                    sort_value,
                    entity_key: key.to_string(),
                };
                self.sorted_keys.remove(&sort_key);
            }
        } else {
            self.access_order.retain(|k| k != key);
        }
        self.entities.remove(key)
    }

    fn evict_oldest(&mut self) -> Option<String> {
        if self.sort_config.is_some() {
            if let Some((sort_key, _)) = self
                .sorted_keys
                .iter()
                .next_back()
                .map(|(k, v)| (k.clone(), *v))
            {
                self.sorted_keys.remove(&sort_key);
                self.entities.remove(&sort_key.entity_key);
                return Some(sort_key.entity_key);
            }
            return None;
        }

        if let Some(oldest_key) = self.access_order.pop_front() {
            self.entities.remove(&oldest_key);
            Some(oldest_key)
        } else {
            None
        }
    }

    fn len(&self) -> usize {
        self.entities.len()
    }

    #[allow(dead_code)]
    fn ordered_keys(&self) -> Vec<String> {
        if let Some(ref config) = self.sort_config {
            let keys: Vec<String> = self
                .sorted_keys
                .keys()
                .map(|sk| sk.entity_key.clone())
                .collect();
            match config.order {
                SortOrder::Asc => keys,
                SortOrder::Desc => keys.into_iter().rev().collect(),
            }
        } else {
            self.entities.keys().cloned().collect()
        }
    }

    fn ordered_values(&self) -> Vec<serde_json::Value> {
        if let Some(ref config) = self.sort_config {
            let values: Vec<serde_json::Value> = self
                .sorted_keys
                .keys()
                .filter_map(|sk| self.entities.get(&sk.entity_key).cloned())
                .collect();
            match config.order {
                SortOrder::Asc => values,
                SortOrder::Desc => values.into_iter().rev().collect(),
            }
        } else {
            self.entities.values().cloned().collect()
        }
    }
}

impl SharedStore {
    pub fn new() -> Self {
        Self::with_config(StoreConfig::default())
    }

    pub fn with_config(config: StoreConfig) -> Self {
        let (updates_tx, _) = broadcast::channel(1000);
        let (ready_tx, ready_rx) = watch::channel(HashSet::new());
        Self {
            views: Arc::new(RwLock::new(HashMap::new())),
            view_configs: Arc::new(RwLock::new(HashMap::new())),
            updates_tx,
            ready_views: Arc::new(RwLock::new(HashSet::new())),
            ready_tx,
            ready_rx,
            config,
        }
    }

    fn enforce_max_entries(&self, view_data: &mut ViewData) {
        if let Some(max) = self.config.max_entries_per_view {
            while view_data.len() > max {
                if let Some(evicted_key) = view_data.evict_oldest() {
                    tracing::debug!("evicted oldest entry: {}", evicted_key);
                }
            }
        }
    }

    pub async fn apply_frame(&self, frame: Frame) {
        let view_path = &frame.entity;
        tracing::debug!(
            "apply_frame: view={}, key={}, op={}",
            view_path,
            frame.key,
            frame.op,
        );

        let operation = frame.operation();

        if operation == Operation::Snapshot {
            self.apply_snapshot(&frame).await;
            return;
        }

        let sort_config = self.view_configs.read().await.get(view_path).cloned();

        let mut views = self.views.write().await;
        let view_data = views.entry(view_path.to_string()).or_insert_with(|| {
            if let Some(config) = sort_config {
                ViewData::with_sort_config(config)
            } else {
                ViewData::new()
            }
        });

        let previous = view_data.entities.get(&frame.key).cloned();

        let (current, patch) = match operation {
            Operation::Upsert | Operation::Create => {
                view_data.insert(frame.key.clone(), frame.data.clone());
                self.enforce_max_entries(view_data);
                (Some(frame.data), None)
            }
            Operation::Patch => {
                let raw_patch = frame.data.clone();
                let entry = view_data
                    .entities
                    .entry(frame.key.clone())
                    .or_insert_with(|| serde_json::json!({}));
                deep_merge_with_append(entry, &frame.data, &frame.append, "");
                let merged = entry.clone();
                view_data.touch(&frame.key);
                self.enforce_max_entries(view_data);
                (Some(merged), Some(raw_patch))
            }
            Operation::Delete => {
                view_data.remove(&frame.key);
                (None, None)
            }
            Operation::Snapshot | Operation::Subscribed => unreachable!(),
        };

        let _ = self.updates_tx.send(StoreUpdate {
            view: view_path.to_string(),
            key: frame.key,
            operation,
            data: current,
            previous,
            patch,
        });

        self.mark_view_ready(view_path).await;
    }

    async fn apply_snapshot(&self, frame: &Frame) {
        let view_path = &frame.entity;
        let snapshot_entities = parse_snapshot_entities(&frame.data);

        tracing::debug!(
            "apply_snapshot: view={}, count={}",
            view_path,
            snapshot_entities.len()
        );

        let sort_config = self.view_configs.read().await.get(view_path).cloned();

        let mut views = self.views.write().await;
        let view_data = views.entry(view_path.to_string()).or_insert_with(|| {
            if let Some(config) = sort_config {
                ViewData::with_sort_config(config)
            } else {
                ViewData::new()
            }
        });

        for entity in snapshot_entities {
            let previous = view_data.entities.get(&entity.key).cloned();
            view_data.insert(entity.key.clone(), entity.data.clone());

            let _ = self.updates_tx.send(StoreUpdate {
                view: view_path.to_string(),
                key: entity.key,
                operation: Operation::Upsert,
                data: Some(entity.data),
                previous,
                patch: None,
            });
        }

        self.enforce_max_entries(view_data);
        drop(views);
        self.mark_view_ready(view_path).await;
    }

    pub async fn mark_view_ready(&self, view: &str) {
        let mut ready = self.ready_views.write().await;
        if ready.insert(view.to_string()) {
            let _ = self.ready_tx.send(ready.clone());
        }
    }

    pub async fn wait_for_view_ready(&self, view: &str, timeout: std::time::Duration) -> bool {
        if self.ready_views.read().await.contains(view) {
            return true;
        }

        let mut rx = self.ready_rx.clone();
        let deadline = tokio::time::Instant::now() + timeout;

        loop {
            let timeout_remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if timeout_remaining.is_zero() {
                return false;
            }

            tokio::select! {
                result = rx.changed() => {
                    if result.is_err() {
                        return false;
                    }
                    if rx.borrow().contains(view) {
                        return true;
                    }
                }
                _ = tokio::time::sleep(timeout_remaining) => {
                    return false;
                }
            }
        }
    }

    pub async fn get<T: DeserializeOwned>(&self, view: &str, key: &str) -> Option<T> {
        let views = self.views.read().await;
        views
            .get(view)?
            .entities
            .get(key)
            .and_then(|v| serde_json::from_value(v.clone()).ok())
    }

    pub async fn list<T: DeserializeOwned>(&self, view: &str) -> Vec<T> {
        let views = self.views.read().await;
        views
            .get(view)
            .map(|view_data| {
                view_data
                    .ordered_values()
                    .into_iter()
                    .filter_map(|v| serde_json::from_value(v).ok())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub async fn all_raw(&self, view: &str) -> HashMap<String, serde_json::Value> {
        let views = self.views.read().await;
        views
            .get(view)
            .map(|view_data| view_data.entities.clone())
            .unwrap_or_default()
    }

    /// Synchronously get a single entity by key.
    ///
    /// Returns `None` if the view doesn't exist or the key is not found.
    /// This is a non-blocking operation using `try_read()`.
    ///
    /// Use this in synchronous contexts where you can't await.
    /// If the lock is held by another task, returns `None`.
    pub fn get_sync<T: DeserializeOwned>(&self, view: &str, key: &str) -> Option<T> {
        let views = self.views.try_read().ok()?;
        views
            .get(view)?
            .entities
            .get(key)
            .and_then(|v| serde_json::from_value(v.clone()).ok())
    }

    /// Synchronously get all entities from a view.
    ///
    /// Returns an empty vector if the view doesn't exist.
    /// This is a non-blocking operation using `try_read()`.
    ///
    /// Use this in synchronous contexts where you can't await.
    /// If the lock is held by another task, returns an empty vector.
    pub fn list_sync<T: DeserializeOwned>(&self, view: &str) -> Vec<T> {
        let Some(views) = self.views.try_read().ok() else {
            return Vec::new();
        };
        views
            .get(view)
            .map(|view_data| {
                view_data
                    .ordered_values()
                    .into_iter()
                    .filter_map(|v| serde_json::from_value(v).ok())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<StoreUpdate> {
        self.updates_tx.subscribe()
    }

    pub async fn apply_subscribed_frame(&self, frame: SubscribedFrame) {
        let view_path = &frame.view;
        tracing::debug!(
            "apply_subscribed_frame: view={}, mode={:?}, sort={:?}",
            view_path,
            frame.mode,
            frame.sort,
        );

        if let Some(sort_config) = frame.sort {
            self.view_configs
                .write()
                .await
                .insert(view_path.to_string(), sort_config.clone());

            let mut views = self.views.write().await;
            if let Some(view_data) = views.get_mut(view_path) {
                view_data.set_sort_config(sort_config);
            }
        }
    }

    pub async fn get_view_sort_config(&self, view: &str) -> Option<SortConfig> {
        self.view_configs.read().await.get(view).cloned()
    }

    pub async fn set_view_sort_config(&self, view: &str, config: SortConfig) {
        self.view_configs
            .write()
            .await
            .insert(view.to_string(), config.clone());

        let mut views = self.views.write().await;
        if let Some(view_data) = views.get_mut(view) {
            view_data.set_sort_config(config);
        }
    }
}

impl Default for SharedStore {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for SharedStore {
    fn clone(&self) -> Self {
        Self {
            views: self.views.clone(),
            view_configs: self.view_configs.clone(),
            updates_tx: self.updates_tx.clone(),
            ready_views: self.ready_views.clone(),
            ready_tx: self.ready_tx.clone(),
            ready_rx: self.ready_rx.clone(),
            config: self.config.clone(),
        }
    }
}
