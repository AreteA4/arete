use crate::error::AreteError;
use crate::frame::{Mode, Operation, ServerFrame, SnapshotEntity, SortConfig, SortOrder};
use crate::subscription::{canonical_subscription_identity, SnapshotOptions, SubscriptionQuery};
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use tokio::sync::{broadcast, watch, RwLock};

pub const DEFAULT_MAX_ENTRIES_PER_VIEW: usize = 10_000;

#[derive(Debug, Clone)]
pub struct StoreConfig {
    pub max_entries_per_view: Option<usize>,
}

impl Default for StoreConfig {
    fn default() -> Self {
        Self {
            max_entries_per_view: Some(DEFAULT_MAX_ENTRIES_PER_VIEW),
        }
    }
}

#[derive(Debug, Default)]
struct ViewData {
    entities: HashMap<String, Value>,
    access_order: VecDeque<String>,
}

impl ViewData {
    fn insert(&mut self, key: String, value: Value) {
        self.access_order.retain(|existing| existing != &key);
        self.access_order.push_back(key.clone());
        self.entities.insert(key, value);
    }

    fn remove(&mut self, key: &str) -> Option<Value> {
        self.access_order.retain(|existing| existing != key);
        self.entities.remove(key)
    }
}

#[derive(Debug, Clone)]
struct QueryData {
    requested_identity: String,
    effective_query: SubscriptionQuery,
    snapshot_enabled: bool,
    membership: Vec<String>,
    mode: Option<Mode>,
    sort: Option<SortConfig>,
}

#[derive(Debug)]
struct SnapshotStage {
    snapshot_id: String,
    authoritative: bool,
    mode: Mode,
    entity: String,
    key: Option<String>,
    rows: Vec<SnapshotEntity>,
}

impl SnapshotStage {
    fn append(&mut self, rows: Vec<SnapshotEntity>) {
        for row in rows {
            if let Some(existing) = self
                .rows
                .iter_mut()
                .find(|existing| existing.key == row.key)
            {
                existing.data = row.data;
            } else {
                self.rows.push(row);
            }
        }
    }
}

#[derive(Debug, Default)]
struct StoreState {
    views: HashMap<String, ViewData>,
    queries: HashMap<String, QueryData>,
    query_ids: HashMap<String, String>,
    snapshots: HashMap<String, SnapshotStage>,
    ready: HashSet<String>,
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
                    format!("{current_path}.{key}")
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
        (Value::Array(target_array), Value::Array(patch_array))
            if append_paths.iter().any(|path| path == current_path) =>
        {
            target_array.extend(patch_array.iter().cloned());
        }
        (target, patch) => *target = patch.clone(),
    }
}

#[derive(Debug, Clone)]
pub struct StoreUpdate {
    pub subscription_id: String,
    pub view: String,
    pub key: String,
    pub operation: Operation,
    pub data: Option<Value>,
    pub previous: Option<Value>,
    pub patch: Option<Value>,
}

pub struct SharedStore {
    state: Arc<RwLock<StoreState>>,
    updates_tx: broadcast::Sender<StoreUpdate>,
    ready_tx: watch::Sender<HashSet<String>>,
    ready_rx: watch::Receiver<HashSet<String>>,
    config: StoreConfig,
}

impl SharedStore {
    pub fn new() -> Self {
        Self::with_config(StoreConfig::default())
    }

    pub fn with_config(config: StoreConfig) -> Self {
        let (updates_tx, _) = broadcast::channel(1000);
        let (ready_tx, ready_rx) = watch::channel(HashSet::new());
        Self {
            state: Arc::new(RwLock::new(StoreState::default())),
            updates_tx,
            ready_tx,
            ready_rx,
            config,
        }
    }

    pub async fn register_subscription(
        &self,
        subscription_id: &str,
        query: SubscriptionQuery,
        snapshot_enabled: bool,
    ) -> Result<(), AreteError> {
        let identity = canonical_subscription_identity(
            &query,
            &SnapshotOptions {
                enabled: snapshot_enabled,
            },
        )?;
        let mut state = self.state.write().await;
        if let Some(existing) = state.queries.get_mut(subscription_id) {
            if existing.requested_identity != identity {
                return Err(protocol_error(
                    Some(subscription_id),
                    "subscriptionId was reused for a different query",
                ));
            }
            existing.snapshot_enabled = snapshot_enabled;
            return Ok(());
        }
        state
            .query_ids
            .insert(identity.clone(), subscription_id.to_string());
        state.queries.insert(
            subscription_id.to_string(),
            QueryData {
                requested_identity: identity,
                effective_query: query,
                snapshot_enabled,
                membership: Vec::new(),
                mode: None,
                sort: None,
            },
        );
        Ok(())
    }

    pub async fn begin_refresh(&self, subscription_id: &str) {
        self.state.write().await.snapshots.remove(subscription_id);
    }

    pub async fn unregister_subscription(&self, subscription_id: &str) {
        let mut state = self.state.write().await;
        if let Some(query) = state.queries.remove(subscription_id) {
            state.query_ids.remove(&query.requested_identity);
        }
        state.snapshots.remove(subscription_id);
        if state.ready.remove(subscription_id) {
            let _ = self.ready_tx.send(state.ready.clone());
        }
    }

    pub async fn apply_frame(&self, frame: ServerFrame) -> Result<(), AreteError> {
        match frame {
            ServerFrame::Subscribed {
                subscription_id,
                query,
                mode,
                sort,
                ..
            } => {
                self.apply_subscribed(subscription_id, query, mode, sort)
                    .await
            }
            ServerFrame::Unsubscribed {
                subscription_id, ..
            } => {
                self.unregister_subscription(&subscription_id).await;
                Ok(())
            }
            ServerFrame::Snapshot {
                subscription_id,
                snapshot_id,
                authoritative,
                mode,
                entity,
                key,
                data,
                complete,
                ..
            } => {
                self.apply_snapshot(
                    subscription_id,
                    snapshot_id,
                    authoritative,
                    mode,
                    entity,
                    key,
                    data,
                    complete,
                )
                .await
            }
            ServerFrame::Upsert {
                subscription_id,
                entity,
                key,
                data,
                ..
            } => {
                self.apply_live(
                    subscription_id,
                    entity,
                    Operation::Upsert,
                    key,
                    data,
                    vec![],
                )
                .await
            }
            ServerFrame::Patch {
                subscription_id,
                entity,
                key,
                data,
                append,
                ..
            } => {
                self.apply_live(subscription_id, entity, Operation::Patch, key, data, append)
                    .await
            }
            ServerFrame::Remove {
                subscription_id,
                entity,
                key,
                data,
                ..
            } => {
                self.apply_live(
                    subscription_id,
                    entity,
                    Operation::Remove,
                    key,
                    data,
                    vec![],
                )
                .await
            }
            ServerFrame::Delete {
                subscription_id,
                entity,
                key,
                data,
                ..
            } => {
                self.apply_live(
                    subscription_id,
                    entity,
                    Operation::Delete,
                    key,
                    data,
                    vec![],
                )
                .await
            }
        }
    }

    async fn apply_subscribed(
        &self,
        subscription_id: String,
        query: SubscriptionQuery,
        mode: Mode,
        sort: Option<SortConfig>,
    ) -> Result<(), AreteError> {
        let mark_ready = {
            let mut state = self.state.write().await;
            let Some(active) = state.queries.get_mut(&subscription_id) else {
                return Err(protocol_error(
                    Some(&subscription_id),
                    "received subscribed acknowledgement for an unknown subscriptionId",
                ));
            };
            if active.effective_query.view != query.view {
                return Err(protocol_error(
                    Some(&subscription_id),
                    "server acknowledgement changed query.view",
                ));
            }
            active.effective_query = query;
            active.mode = Some(mode);
            active.sort = sort;
            !active.snapshot_enabled
        };
        if mark_ready {
            self.mark_subscription_ready(&subscription_id).await;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn apply_snapshot(
        &self,
        subscription_id: String,
        snapshot_id: String,
        authoritative: bool,
        mode: Mode,
        entity: String,
        key: Option<String>,
        rows: Vec<SnapshotEntity>,
        complete: bool,
    ) -> Result<(), AreteError> {
        let completed = {
            let mut state = self.state.write().await;
            let Some(query) = state.queries.get(&subscription_id) else {
                return Err(protocol_error(
                    Some(&subscription_id),
                    "received snapshot for an unknown subscriptionId",
                ));
            };
            if query.effective_query.view != entity {
                return Err(protocol_error(
                    Some(&subscription_id),
                    "snapshot entity does not match the acknowledged query.view",
                ));
            }

            if let Some(stage) = state.snapshots.get(&subscription_id) {
                if stage.snapshot_id != snapshot_id
                    || stage.authoritative != authoritative
                    || stage.mode != mode
                    || stage.entity != entity
                    || stage.key != key
                {
                    return Err(protocol_error(
                        Some(&subscription_id),
                        "snapshot batches changed identity or metadata before completion",
                    ));
                }
            } else {
                state.snapshots.insert(
                    subscription_id.clone(),
                    SnapshotStage {
                        snapshot_id,
                        authoritative,
                        mode,
                        entity,
                        key,
                        rows: Vec::new(),
                    },
                );
            }
            state
                .snapshots
                .get_mut(&subscription_id)
                .expect("snapshot stage inserted")
                .append(rows);
            complete.then(|| {
                state
                    .snapshots
                    .remove(&subscription_id)
                    .expect("completed snapshot stage exists")
            })
        };

        if let Some(stage) = completed {
            self.commit_snapshot(&subscription_id, stage).await?;
        }
        Ok(())
    }

    async fn commit_snapshot(
        &self,
        subscription_id: &str,
        stage: SnapshotStage,
    ) -> Result<(), AreteError> {
        let mut updates = Vec::new();
        {
            let mut state = self.state.write().await;
            let old_membership = state
                .queries
                .get(subscription_id)
                .ok_or_else(|| {
                    protocol_error(
                        Some(subscription_id),
                        "subscription ended before its snapshot completed",
                    )
                })?
                .membership
                .clone();

            let mut snapshot_keys = Vec::with_capacity(stage.rows.len());
            for row in stage.rows {
                snapshot_keys.push(row.key.clone());
                let view = state.views.entry(stage.entity.clone()).or_default();
                let previous = view.entities.get(&row.key).cloned();
                view.insert(row.key.clone(), row.data.clone());
                updates.push(StoreUpdate {
                    subscription_id: subscription_id.to_string(),
                    view: stage.entity.clone(),
                    key: row.key,
                    operation: Operation::Upsert,
                    data: Some(row.data),
                    previous,
                    patch: None,
                });
            }

            let query = state
                .queries
                .get_mut(subscription_id)
                .expect("query checked above");
            if stage.authoritative {
                query.membership = snapshot_keys.clone();
            } else {
                for key in snapshot_keys {
                    if !query.membership.contains(&key) {
                        query.membership.push(key);
                    }
                }
            }

            if stage.authoritative {
                let retained: HashSet<&str> = query.membership.iter().map(String::as_str).collect();
                let removed: Vec<String> = old_membership
                    .into_iter()
                    .filter(|key| !retained.contains(key.as_str()))
                    .collect();
                for key in &removed {
                    updates.push(StoreUpdate {
                        subscription_id: subscription_id.to_string(),
                        view: stage.entity.clone(),
                        key: key.clone(),
                        operation: Operation::Remove,
                        data: None,
                        previous: state
                            .views
                            .get(&stage.entity)
                            .and_then(|view| view.entities.get(key).cloned()),
                        patch: None,
                    });
                }
                prune_unreferenced(&mut state, &stage.entity, &removed);
            }
            enforce_max_entries(&mut state, &stage.entity, self.config.max_entries_per_view);
        }

        for update in updates {
            let _ = self.updates_tx.send(update);
        }
        self.mark_subscription_ready(subscription_id).await;
        Ok(())
    }

    async fn apply_live(
        &self,
        subscription_id: String,
        entity: String,
        operation: Operation,
        key: String,
        data: Value,
        append: Vec<String>,
    ) -> Result<(), AreteError> {
        let mut updates = Vec::new();
        {
            let mut state = self.state.write().await;
            let Some(query) = state.queries.get(&subscription_id) else {
                return Err(protocol_error(
                    Some(&subscription_id),
                    "received live frame for an unknown subscriptionId",
                ));
            };
            if query.effective_query.view != entity {
                return Err(protocol_error(
                    Some(&subscription_id),
                    "live frame entity does not match the acknowledged query.view",
                ));
            }

            match operation {
                Operation::Upsert => {
                    let view = state.views.entry(entity.clone()).or_default();
                    let previous = view.entities.get(&key).cloned();
                    view.insert(key.clone(), data.clone());
                    let query = state
                        .queries
                        .get_mut(&subscription_id)
                        .expect("query checked above");
                    if !query.membership.contains(&key) {
                        query.membership.insert(0, key.clone());
                    }
                    updates.push(StoreUpdate {
                        subscription_id: subscription_id.clone(),
                        view: entity.clone(),
                        key,
                        operation,
                        data: Some(data),
                        previous,
                        patch: None,
                    });
                }
                Operation::Patch => {
                    let view = state.views.entry(entity.clone()).or_default();
                    let previous = view.entities.get(&key).cloned();
                    let entry = view
                        .entities
                        .entry(key.clone())
                        .or_insert_with(|| Value::Object(Default::default()));
                    deep_merge_with_append(entry, &data, &append, "");
                    let merged = entry.clone();
                    view.access_order.retain(|existing| existing != &key);
                    view.access_order.push_back(key.clone());
                    let query = state
                        .queries
                        .get_mut(&subscription_id)
                        .expect("query checked above");
                    if !query.membership.contains(&key) {
                        query.membership.insert(0, key.clone());
                    }
                    updates.push(StoreUpdate {
                        subscription_id: subscription_id.clone(),
                        view: entity.clone(),
                        key,
                        operation,
                        data: Some(merged),
                        previous,
                        patch: Some(data),
                    });
                }
                Operation::Remove => {
                    let query = state
                        .queries
                        .get_mut(&subscription_id)
                        .expect("query checked above");
                    query.membership.retain(|member| member != &key);
                    updates.push(StoreUpdate {
                        subscription_id: subscription_id.clone(),
                        view: entity.clone(),
                        key: key.clone(),
                        operation,
                        data: None,
                        previous: state
                            .views
                            .get(&entity)
                            .and_then(|view| view.entities.get(&key).cloned()),
                        patch: None,
                    });
                }
                Operation::Delete => {
                    let previous = state
                        .views
                        .get_mut(&entity)
                        .and_then(|view| view.remove(&key));
                    let mut affected = Vec::new();
                    for (id, query) in &mut state.queries {
                        if query.effective_query.view == entity && query.membership.contains(&key) {
                            query.membership.retain(|member| member != &key);
                            affected.push(id.clone());
                        }
                    }
                    if affected.is_empty() && previous.is_some() {
                        affected.push(subscription_id.clone());
                    }
                    for id in affected {
                        updates.push(StoreUpdate {
                            subscription_id: id,
                            view: entity.clone(),
                            key: key.clone(),
                            operation,
                            data: None,
                            previous: previous.clone(),
                            patch: None,
                        });
                    }
                }
            }
            state.ready.insert(subscription_id.clone());
            enforce_max_entries(&mut state, &entity, self.config.max_entries_per_view);
            let _ = self.ready_tx.send(state.ready.clone());
        }
        for update in updates {
            let _ = self.updates_tx.send(update);
        }
        Ok(())
    }

    async fn mark_subscription_ready(&self, subscription_id: &str) {
        let mut state = self.state.write().await;
        if state.ready.insert(subscription_id.to_string()) {
            let _ = self.ready_tx.send(state.ready.clone());
        }
    }

    pub async fn wait_for_subscription_ready(
        &self,
        subscription_id: &str,
        timeout: std::time::Duration,
    ) -> bool {
        if self.state.read().await.ready.contains(subscription_id) {
            return true;
        }
        let mut receiver = self.ready_rx.clone();
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return false;
            }
            tokio::select! {
                changed = receiver.changed() => {
                    if changed.is_err() {
                        return false;
                    }
                    if receiver.borrow().contains(subscription_id) {
                        return true;
                    }
                }
                _ = tokio::time::sleep(remaining) => return false,
            }
        }
    }

    pub async fn get_for_subscription<T: DeserializeOwned>(
        &self,
        subscription_id: &str,
        key: &str,
    ) -> Option<T> {
        let state = self.state.read().await;
        let query = state.queries.get(subscription_id)?;
        if !query.membership.iter().any(|member| member == key) {
            return None;
        }
        state
            .views
            .get(&query.effective_query.view)?
            .entities
            .get(key)
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok())
    }

    pub async fn list_for_subscription<T: DeserializeOwned>(
        &self,
        subscription_id: &str,
    ) -> Vec<T> {
        let state = self.state.read().await;
        list_query_raw(&state, subscription_id)
            .into_iter()
            .filter_map(|value| serde_json::from_value(value).ok())
            .collect()
    }

    pub async fn keys_for_subscription(&self, subscription_id: &str) -> Vec<String> {
        self.state
            .read()
            .await
            .queries
            .get(subscription_id)
            .map(|query| query.membership.clone())
            .unwrap_or_default()
    }

    pub fn get_for_query_sync<T: DeserializeOwned>(
        &self,
        query: &SubscriptionQuery,
        key: &str,
    ) -> Option<T> {
        let identity = canonical_subscription_identity(query, &SnapshotOptions::default()).ok()?;
        let state = self.state.try_read().ok()?;
        let subscription_id = state.query_ids.get(&identity)?;
        let query = state.queries.get(subscription_id)?;
        if !query.membership.iter().any(|member| member == key) {
            return None;
        }
        state
            .views
            .get(&query.effective_query.view)?
            .entities
            .get(key)
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok())
    }

    pub fn list_for_query_sync<T: DeserializeOwned>(&self, query: &SubscriptionQuery) -> Vec<T> {
        let Ok(identity) = canonical_subscription_identity(query, &SnapshotOptions::default())
        else {
            return Vec::new();
        };
        let Ok(state) = self.state.try_read() else {
            return Vec::new();
        };
        let Some(subscription_id) = state.query_ids.get(&identity) else {
            return Vec::new();
        };
        list_query_raw(&state, subscription_id)
            .into_iter()
            .filter_map(|value| serde_json::from_value(value).ok())
            .collect()
    }

    pub async fn get<T: DeserializeOwned>(&self, view: &str, key: &str) -> Option<T> {
        self.state
            .read()
            .await
            .views
            .get(view)?
            .entities
            .get(key)
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok())
    }

    pub async fn list<T: DeserializeOwned>(&self, view: &str) -> Vec<T> {
        self.state
            .read()
            .await
            .views
            .get(view)
            .map(|view| {
                view.entities
                    .values()
                    .cloned()
                    .filter_map(|value| serde_json::from_value(value).ok())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub async fn all_raw(&self, view: &str) -> HashMap<String, Value> {
        self.state
            .read()
            .await
            .views
            .get(view)
            .map(|view| view.entities.clone())
            .unwrap_or_default()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<StoreUpdate> {
        self.updates_tx.subscribe()
    }
}

fn list_query_raw(state: &StoreState, subscription_id: &str) -> Vec<Value> {
    let Some(query) = state.queries.get(subscription_id) else {
        return Vec::new();
    };
    let Some(view) = state.views.get(&query.effective_query.view) else {
        return Vec::new();
    };
    let mut rows: Vec<(String, Value)> = query
        .membership
        .iter()
        .filter_map(|key| {
            view.entities
                .get(key)
                .cloned()
                .map(|value| (key.clone(), value))
        })
        .collect();
    if let Some(sort) = &query.sort {
        rows.sort_by(|(left_key, left), (right_key, right)| {
            let order =
                compare_at_path(left, right, &sort.field).then_with(|| left_key.cmp(right_key));
            match sort.order {
                SortOrder::Asc => order,
                SortOrder::Desc => order.reverse(),
            }
        });
    }
    rows.into_iter().map(|(_, value)| value).collect()
}

fn compare_at_path(left: &Value, right: &Value, path: &[String]) -> Ordering {
    let left = value_at_path(left, path);
    let right = value_at_path(right, path);
    match (left, right) {
        (Some(Value::Null) | None, Some(Value::Null) | None) => Ordering::Equal,
        (Some(Value::Null) | None, _) => Ordering::Less,
        (_, Some(Value::Null) | None) => Ordering::Greater,
        (Some(Value::Bool(left)), Some(Value::Bool(right))) => left.cmp(right),
        (Some(Value::Number(left)), Some(Value::Number(right))) => left
            .as_f64()
            .partial_cmp(&right.as_f64())
            .unwrap_or(Ordering::Equal),
        (Some(Value::String(left)), Some(Value::String(right))) => left.cmp(right),
        (Some(left), Some(right)) => left.to_string().cmp(&right.to_string()),
    }
}

fn value_at_path<'a>(value: &'a Value, path: &[String]) -> Option<&'a Value> {
    path.iter()
        .try_fold(value, |current, segment| current.get(segment))
}

fn prune_unreferenced(state: &mut StoreState, view: &str, candidates: &[String]) {
    let referenced: HashSet<&str> = state
        .queries
        .values()
        .filter(|query| query.effective_query.view == view)
        .flat_map(|query| query.membership.iter().map(String::as_str))
        .collect();
    if let Some(view_data) = state.views.get_mut(view) {
        for key in candidates {
            if !referenced.contains(key.as_str()) {
                view_data.remove(key);
            }
        }
    }
}

fn enforce_max_entries(state: &mut StoreState, view: &str, max: Option<usize>) {
    let Some(max) = max else {
        return;
    };
    let referenced: HashSet<String> = state
        .queries
        .values()
        .filter(|query| query.effective_query.view == view)
        .flat_map(|query| query.membership.iter().cloned())
        .collect();
    let Some(view_data) = state.views.get_mut(view) else {
        return;
    };
    while view_data.entities.len() > max {
        let Some(index) = view_data
            .access_order
            .iter()
            .position(|key| !referenced.contains(key))
        else {
            break;
        };
        let key = view_data
            .access_order
            .remove(index)
            .expect("access order index exists");
        view_data.entities.remove(&key);
    }
}

fn protocol_error(subscription_id: Option<&str>, message: &str) -> AreteError {
    AreteError::Protocol {
        message: message.to_string(),
        subscription_id: subscription_id.map(str::to_string),
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
            state: self.state.clone(),
            updates_tx: self.updates_tx.clone(),
            ready_tx: self.ready_tx.clone(),
            ready_rx: self.ready_rx.clone(),
            config: self.config.clone(),
        }
    }
}
