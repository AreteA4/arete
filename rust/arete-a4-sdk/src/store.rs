use crate::collation::{collation_key, locale_compare, CollationKey};
use crate::error::AreteError;
use crate::frame::{
    compare_seq, Mode, Operation, ServerFrame, SnapshotEntity, SortConfig, SortOrder,
};
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
    /// Last sequence written for each key, used by the duplicate/stale guard in
    /// [`SharedStore::apply_live`].
    ///
    /// TypeScript hangs a non-enumerable `__seq` off the stored entity
    /// (`frame-processor.ts:323`), which is not reproducible here: a
    /// `serde_json::Value` has no hidden fields, so an injected `__seq` key
    /// would surface in `all_raw`/`Value` reads and break typed
    /// `serde_json::from_value` deserialization of user structs. Python solved
    /// it the same way, with a sibling `_seqs[view][key]` map (`store.py:170`).
    /// Keeping the map inside `ViewData` ties its lifecycle to the entity's:
    /// every write goes through [`ViewData::insert`]/[`ViewData::set_seq`] and
    /// every eviction through [`ViewData::remove`].
    seqs: HashMap<String, String>,
    access_order: VecDeque<String>,
}

impl ViewData {
    fn insert(&mut self, key: String, value: Value, seq: Option<String>) {
        self.access_order.retain(|existing| existing != &key);
        self.access_order.push_back(key.clone());
        self.set_seq(key.clone(), seq);
        self.entities.insert(key, value);
    }

    /// Replace the tracked sequence for `key`, clearing it when `seq` is `None`.
    ///
    /// Clearing matches TypeScript: `attachInternalSeq` (`frame-processor.ts:323`)
    /// is a no-op for an undefined seq and the value it decorates is a freshly
    /// normalized object, so a write without a sequence leaves the stored entity
    /// with no `__seq` at all.
    /// Record a sequence for `key`. An unsequenced write leaves the tracked
    /// sequence in place rather than clearing it: dropping it would disarm the
    /// staleness guard for this key until the next sequenced frame, letting a
    /// later older frame overwrite newer data. Matches Python
    /// (`store.py::_set_entity`); TypeScript clears here as a side effect of
    /// carrying `__seq` on the entity object it replaces — see the divergence
    /// note in `docs/internal/sdk-api-surface.md`.
    fn set_seq(&mut self, key: String, seq: Option<String>) {
        if let Some(seq) = seq {
            self.seqs.insert(key, seq);
        }
    }

    fn remove(&mut self, key: &str) -> Option<Value> {
        self.access_order.retain(|existing| existing != key);
        self.seqs.remove(key);
        self.entities.remove(key)
    }
}

/// Read the `_seq` field off a frame payload.
///
/// Mirrors `extractSeq` (`frame-processor.ts:286`) and `_extract_seq`
/// (`store.py:154`): strings pass through, finite numbers are stringified.
fn extract_seq(data: &Value) -> Option<String> {
    match data.get("_seq") {
        Some(Value::String(seq)) => Some(seq.clone()),
        Some(Value::Number(seq)) => Some(seq.to_string()),
        _ => None,
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
                seq,
                ..
            } => {
                self.apply_live(
                    subscription_id,
                    entity,
                    Operation::Upsert,
                    key,
                    data,
                    vec![],
                    seq,
                )
                .await
            }
            ServerFrame::Patch {
                subscription_id,
                entity,
                key,
                data,
                append,
                seq,
                ..
            } => {
                self.apply_live(
                    subscription_id,
                    entity,
                    Operation::Patch,
                    key,
                    data,
                    append,
                    seq,
                )
                .await
            }
            // `remove`/`delete` ignore `seq` in every SDK: they carry no payload
            // to keep, so there is nothing for a stale sequence to protect
            // (`frame-processor.ts:747-766`, `store.py:393-402`).
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
                    None,
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
                    None,
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
                // Snapshot rows deliberately bypass the stale-sequence guard and
                // replace whatever is cached — `handleSnapshotFrameWithoutEnforce`
                // (`frame-processor.ts:588`) and `_handle_snapshot`
                // (`store.py:282`) both write unconditionally — but they still
                // publish their `_seq` so later live frames can be ordered.
                let seq = extract_seq(&row.data);
                view.insert(row.key.clone(), row.data.clone(), seq);
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

    #[allow(clippy::too_many_arguments)]
    async fn apply_live(
        &self,
        subscription_id: String,
        entity: String,
        operation: Operation,
        key: String,
        data: Value,
        append: Vec<String>,
        seq: Option<String>,
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

            // `handleEntityFrameWithoutEnforce` (`frame-processor.ts:627`):
            //   frame.seq !== undefined && previousSequence !== undefined
            //     && compareSeq(frame.seq, previousSequence) <= 0
            // A frame at or behind the sequence already stored must not overwrite
            // the newer cached entity. `<= 0` makes an exact replay a duplicate,
            // and a frame with no `seq` is never stale.
            let previous_seq = state
                .views
                .get(&entity)
                .and_then(|view| view.seqs.get(&key).cloned());
            let duplicate_or_stale_sequence = match (seq.as_deref(), previous_seq.as_deref()) {
                (Some(incoming), Some(previous)) => {
                    compare_seq(incoming, previous) != Ordering::Greater
                }
                _ => false,
            };

            match operation {
                Operation::Upsert => {
                    let view = state.views.entry(entity.clone()).or_default();
                    let previous = view.entities.get(&key).cloned();
                    let stale_cached = if duplicate_or_stale_sequence {
                        previous.clone()
                    } else {
                        None
                    };
                    let update = if let Some(cached) = stale_cached {
                        // Storage is left untouched; the update still fires so a
                        // subscription that lacks the key gains membership and
                        // converges on the newer value we already hold. TS emits
                        // `{type:'upsert', data: previousValue}` with
                        // `createRichUpdate(key, null, previousValue)` — a
                        // `created` rich update, hence `previous: None`.
                        StoreUpdate {
                            subscription_id: subscription_id.clone(),
                            view: entity.clone(),
                            key: key.clone(),
                            operation,
                            data: Some(cached),
                            previous: None,
                            patch: None,
                        }
                    } else {
                        // `frame.seq ?? extractSeq(frame.data)` (`frame-processor.ts:662`).
                        let next_seq = seq.clone().or_else(|| extract_seq(&data));
                        view.insert(key.clone(), data.clone(), next_seq);
                        StoreUpdate {
                            subscription_id: subscription_id.clone(),
                            view: entity.clone(),
                            key: key.clone(),
                            operation,
                            data: Some(data),
                            previous,
                            patch: None,
                        }
                    };
                    let query = state
                        .queries
                        .get_mut(&subscription_id)
                        .expect("query checked above");
                    if !query.membership.contains(&key) {
                        query.membership.insert(0, key.clone());
                    }
                    updates.push(update);
                }
                Operation::Patch => {
                    let view = state.views.entry(entity.clone()).or_default();
                    let previous = view.entities.get(&key).cloned();
                    let stale_existing = if duplicate_or_stale_sequence {
                        previous.clone()
                    } else {
                        None
                    };
                    let update = if let Some(existing) = stale_existing {
                        // Nothing is merged — re-merging a replayed patch would
                        // double-apply every `append` path. TS emits
                        // `{type:'patch', data: normalizedPatch}` with
                        // `createRichUpdate(key, existing, existing, patch)`:
                        // before and after are both the cached value.
                        StoreUpdate {
                            subscription_id: subscription_id.clone(),
                            view: entity.clone(),
                            key: key.clone(),
                            operation,
                            data: Some(existing.clone()),
                            previous: Some(existing),
                            patch: Some(data),
                        }
                    } else {
                        let entry = view
                            .entities
                            .entry(key.clone())
                            .or_insert_with(|| Value::Object(Default::default()));
                        deep_merge_with_append(entry, &data, &append, "");
                        let merged = entry.clone();
                        view.access_order.retain(|existing| existing != &key);
                        view.access_order.push_back(key.clone());
                        // `frame.seq ?? extractSeq(frame.data) ?? getInternalSeq(existing)`
                        // (`frame-processor.ts:721`): a patch that carries no
                        // sequence keeps the one the entity already had.
                        let next_seq = seq
                            .clone()
                            .or_else(|| extract_seq(&data))
                            .or_else(|| previous_seq.clone());
                        view.set_seq(key.clone(), next_seq);
                        StoreUpdate {
                            subscription_id: subscription_id.clone(),
                            view: entity.clone(),
                            key: key.clone(),
                            operation,
                            data: Some(merged),
                            previous,
                            patch: Some(data),
                        }
                    };
                    let query = state
                        .queries
                        .get_mut(&subscription_id)
                        .expect("query checked above");
                    if !query.membership.contains(&key) {
                        query.membership.insert(0, key.clone());
                    }
                    updates.push(update);
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
    // Entity keys are decorated with their collation key: it is recomputed on
    // every comparison otherwise, and the tie-break consults it constantly.
    let mut rows: Vec<(CollationKey, Value)> = query
        .membership
        .iter()
        .filter_map(|key| {
            view.entities
                .get(key)
                .cloned()
                .map(|value| (collation_key(key), value))
        })
        .collect();
    if let Some(sort) = &query.sort {
        rows.sort_by(|(left_key, left), (right_key, right)| {
            let order = compare_at_path(left, right, &sort.field);
            let order = match sort.order {
                SortOrder::Asc => order,
                SortOrder::Desc => order.reverse(),
            };
            // query-store.ts:387 breaks ties on the entity key with
            // `localeCompare`, ascending, *after* the desc negation.
            order.then_with(|| left_key.cmp(right_key))
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
        // query-store.ts:64 falls through to `String(left).localeCompare(...)`,
        // so both string branches collate rather than compare bytes.
        (Some(Value::String(left)), Some(Value::String(right))) => locale_compare(left, right),
        (Some(left), Some(right)) => locale_compare(&left.to_string(), &right.to_string()),
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
        view_data.seqs.remove(&key);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subscription::PROTOCOL_VERSION;
    use serde_json::json;

    async fn store_with_sorted_list(keys: &[&str], sort: SortConfig) -> (SharedStore, String) {
        let store = SharedStore::new();
        let subscription_id = "sub-1";
        store
            .register_subscription(
                subscription_id,
                SubscriptionQuery::new("Account/list"),
                true,
            )
            .await
            .unwrap();
        store
            .apply_frame(ServerFrame::Subscribed {
                protocol_version: PROTOCOL_VERSION,
                subscription_id: subscription_id.to_string(),
                query: SubscriptionQuery::new("Account/list"),
                mode: Mode::List,
                sort: Some(sort),
            })
            .await
            .unwrap();
        store
            .apply_frame(ServerFrame::Snapshot {
                protocol_version: PROTOCOL_VERSION,
                subscription_id: subscription_id.to_string(),
                snapshot_id: "snap-1".to_string(),
                authoritative: true,
                mode: Mode::List,
                entity: "Account/list".to_string(),
                key: None,
                // Every row ties on `rank`, so ordering is decided purely by the
                // entity-key tie-break — the common no-`_seq` case.
                data: keys
                    .iter()
                    .map(|key| SnapshotEntity {
                        key: (*key).to_string(),
                        data: json!({"owner": *key, "rank": 1}),
                    })
                    .collect(),
                complete: true,
            })
            .await
            .unwrap();
        (store, subscription_id.to_string())
    }

    async fn owners(store: &SharedStore, subscription_id: &str) -> Vec<String> {
        store
            .list_for_subscription::<Value>(subscription_id)
            .await
            .into_iter()
            .map(|row| row["owner"].as_str().unwrap().to_string())
            .collect()
    }

    /// Regression: the entity-key tie-break must use `localeCompare`, not bytes.
    ///
    /// Byte order yields `["Bqq", "Zap1", "aBc1", "apple"]` (all uppercase
    /// first); TypeScript's `leftKey.localeCompare(rightKey)` yields
    /// `["aBc1", "apple", "Bqq", "Zap1"]`. Mixed-case base58 addresses hit this
    /// constantly.
    #[tokio::test]
    async fn list_key_tie_break_uses_collation_not_byte_order() {
        let (store, subscription_id) = store_with_sorted_list(
            &["Zap1", "aBc1", "Bqq", "apple"],
            SortConfig {
                field: vec!["rank".to_string()],
                order: SortOrder::Asc,
            },
        )
        .await;

        assert_eq!(
            owners(&store, &subscription_id).await,
            ["aBc1", "apple", "Bqq", "Zap1"]
        );
    }

    /// The tie-break stays ascending under `order: desc`, matching
    /// `query-store.ts:387` where it is applied after the desc negation.
    #[tokio::test]
    async fn list_key_tie_break_is_ascending_even_when_sort_is_desc() {
        let (store, subscription_id) = store_with_sorted_list(
            &["Zap1", "aBc1", "Bqq", "apple"],
            SortConfig {
                field: vec!["rank".to_string()],
                order: SortOrder::Desc,
            },
        )
        .await;

        assert_eq!(
            owners(&store, &subscription_id).await,
            ["aBc1", "apple", "Bqq", "Zap1"]
        );
    }

    /// A registered (not yet acknowledged) subscription, which is all
    /// `apply_live` requires — the Python `Harness` registers the same way.
    async fn register(store: &SharedStore, subscription_id: &str, query: SubscriptionQuery) {
        store
            .register_subscription(subscription_id, query, true)
            .await
            .unwrap();
    }

    fn live_frame(
        subscription_id: &str,
        operation: Operation,
        key: &str,
        data: Value,
        append: Vec<String>,
        seq: Option<&str>,
    ) -> ServerFrame {
        let subscription_id = subscription_id.to_string();
        let entity = "Thing/state".to_string();
        let key = key.to_string();
        let seq = seq.map(str::to_string);
        match operation {
            Operation::Upsert => ServerFrame::Upsert {
                protocol_version: PROTOCOL_VERSION,
                subscription_id,
                mode: Mode::State,
                entity,
                key,
                data,
                append,
                seq,
            },
            Operation::Patch => ServerFrame::Patch {
                protocol_version: PROTOCOL_VERSION,
                subscription_id,
                mode: Mode::State,
                entity,
                key,
                data,
                append,
                seq,
            },
            other => panic!("unsupported live frame operation {other:?}"),
        }
    }

    fn upsert(subscription_id: &str, key: &str, data: Value, seq: Option<&str>) -> ServerFrame {
        live_frame(subscription_id, Operation::Upsert, key, data, vec![], seq)
    }

    fn patch(
        subscription_id: &str,
        key: &str,
        data: Value,
        append: Vec<String>,
        seq: Option<&str>,
    ) -> ServerFrame {
        live_frame(subscription_id, Operation::Patch, key, data, append, seq)
    }

    async fn entity(store: &SharedStore, key: &str) -> Option<Value> {
        store.get::<Value>("Thing/state", key).await
    }

    /// Mirrors `test_stale_sequence_upsert_keeps_newer_entity`
    /// (`python/arete-sdk/tests/test_store.py:420`).
    #[tokio::test]
    async fn stale_sequence_upsert_keeps_newer_entity() {
        let store = SharedStore::new();
        register(&store, "s", SubscriptionQuery::new("Thing/state")).await;

        store
            .apply_frame(upsert(
                "s",
                "k",
                json!({"v": "new"}),
                Some("50:000000000002"),
            ))
            .await
            .unwrap();
        store
            .apply_frame(upsert(
                "s",
                "k",
                json!({"v": "old"}),
                Some("50:000000000001"),
            ))
            .await
            .unwrap();

        assert_eq!(entity(&store, "k").await, Some(json!({"v": "new"})));
    }

    /// Mirrors `test_stale_sequence_patch_does_not_remerge`
    /// (`python/arete-sdk/tests/test_store.py:397`). The append path is the
    /// reason a re-merge is not merely redundant: it would duplicate elements.
    #[tokio::test]
    async fn stale_sequence_patch_does_not_remerge() {
        let store = SharedStore::new();
        register(&store, "s", SubscriptionQuery::new("Thing/state")).await;

        store
            .apply_frame(upsert(
                "s",
                "k",
                json!({"values": ["a"]}),
                Some("50:000000000001"),
            ))
            .await
            .unwrap();
        store
            .apply_frame(patch(
                "s",
                "k",
                json!({"values": ["b"]}),
                vec!["values".to_string()],
                Some("49:000000000001"),
            ))
            .await
            .unwrap();

        assert_eq!(entity(&store, "k").await, Some(json!({"values": ["a"]})));
    }

    /// A stale upsert still grants membership to a subscription that did not
    /// have the key, and the update it emits carries the newer cached value —
    /// late subscribers converge on the authoritative entity instead of the
    /// replayed one (`frame-processor.ts:635-649`).
    #[tokio::test]
    async fn stale_sequence_upsert_adds_membership_with_cached_value() {
        let store = SharedStore::new();
        register(&store, "first", SubscriptionQuery::new("Thing/state")).await;
        register(
            &store,
            "second",
            SubscriptionQuery::new("Thing/state").with_filter("status", "open"),
        )
        .await;

        store
            .apply_frame(upsert(
                "first",
                "k",
                json!({"v": "new"}),
                Some("50:000000000002"),
            ))
            .await
            .unwrap();
        assert!(store.keys_for_subscription("second").await.is_empty());

        let mut updates = store.subscribe();
        store
            .apply_frame(upsert(
                "second",
                "k",
                json!({"v": "old"}),
                Some("50:000000000001"),
            ))
            .await
            .unwrap();

        assert_eq!(store.keys_for_subscription("second").await, ["k"]);
        assert_eq!(entity(&store, "k").await, Some(json!({"v": "new"})));
        let update = updates.try_recv().unwrap();
        assert_eq!(update.subscription_id, "second");
        assert_eq!(update.data, Some(json!({"v": "new"})));
        // `createRichUpdate(key, null, previousValue)` — a `created` update.
        assert_eq!(update.previous, None);
    }

    /// The comparison is `<= 0`, so an exact replay of the stored sequence is a
    /// duplicate and is dropped just like an older one.
    #[tokio::test]
    async fn equal_sequence_is_treated_as_duplicate() {
        let store = SharedStore::new();
        register(&store, "s", SubscriptionQuery::new("Thing/state")).await;

        store
            .apply_frame(upsert("s", "k", json!({"v": "first"}), Some("50:0001")))
            .await
            .unwrap();
        store
            .apply_frame(upsert("s", "k", json!({"v": "replay"}), Some("50:0001")))
            .await
            .unwrap();

        assert_eq!(entity(&store, "k").await, Some(json!({"v": "first"})));
    }

    /// A frame without `seq` is never stale: with no sequence to compare, the
    /// newest frame always wins.
    #[tokio::test]
    async fn frame_without_seq_is_never_stale() {
        let store = SharedStore::new();
        register(&store, "s", SubscriptionQuery::new("Thing/state")).await;

        store
            .apply_frame(upsert("s", "k", json!({"v": "first"}), Some("50:0009")))
            .await
            .unwrap();
        store
            .apply_frame(upsert("s", "k", json!({"v": "second"}), None))
            .await
            .unwrap();
        assert_eq!(entity(&store, "k").await, Some(json!({"v": "second"})));

        // The unsequenced write leaves the tracked sequence intact, so the
        // guard stays armed and a later older frame is still rejected.
        store
            .apply_frame(upsert("s", "k", json!({"v": "third"}), Some("50:0001")))
            .await
            .unwrap();
        assert_eq!(entity(&store, "k").await, Some(json!({"v": "second"})));
    }

    /// `_seq` inside the payload is the fallback sequence source
    /// (`extractSeq`), and a patch with no sequence of its own inherits the
    /// entity's (`frame-processor.ts:721`) — so a later stale frame is still
    /// caught.
    #[tokio::test]
    async fn payload_seq_is_tracked_and_patch_inherits_it() {
        let store = SharedStore::new();
        register(&store, "s", SubscriptionQuery::new("Thing/state")).await;

        store
            .apply_frame(upsert(
                "s",
                "k",
                json!({"v": "new", "_seq": "50:0005"}),
                None,
            ))
            .await
            .unwrap();
        store
            .apply_frame(patch("s", "k", json!({"extra": true}), vec![], None))
            .await
            .unwrap();
        store
            .apply_frame(upsert("s", "k", json!({"v": "old"}), Some("50:0004")))
            .await
            .unwrap();

        assert_eq!(
            entity(&store, "k").await,
            Some(json!({"v": "new", "_seq": "50:0005", "extra": true})),
        );
    }

    /// Snapshot rows keep bypassing the guard: they are authoritative
    /// replacements (`handleSnapshotFrameWithoutEnforce`, `frame-processor.ts:588`).
    #[tokio::test]
    async fn snapshot_rows_replace_regardless_of_sequence() {
        let store = SharedStore::new();
        register(&store, "s", SubscriptionQuery::new("Thing/state")).await;
        store
            .apply_frame(upsert("s", "k", json!({"v": "live"}), Some("50:0009")))
            .await
            .unwrap();

        store
            .apply_frame(ServerFrame::Snapshot {
                protocol_version: PROTOCOL_VERSION,
                subscription_id: "s".to_string(),
                snapshot_id: "snap-1".to_string(),
                authoritative: true,
                mode: Mode::State,
                entity: "Thing/state".to_string(),
                key: None,
                data: vec![SnapshotEntity {
                    key: "k".to_string(),
                    data: json!({"v": "snapshot", "_seq": "50:0001"}),
                }],
                complete: true,
            })
            .await
            .unwrap();

        assert_eq!(
            entity(&store, "k").await,
            Some(json!({"v": "snapshot", "_seq": "50:0001"})),
        );
    }

    /// Regression: string sort-field values collate too (`query-store.ts:64`).
    #[tokio::test]
    async fn string_sort_field_uses_collation_not_byte_order() {
        assert_eq!(
            compare_at_path(
                &json!({"label": "état"}),
                &json!({"label": "zone"}),
                &["label".to_string()],
            ),
            Ordering::Less,
        );
        // The `to_string()` fallback (non-scalar values) collates as well.
        assert_eq!(
            compare_at_path(
                &json!({"label": ["état"]}),
                &json!({"label": ["zone"]}),
                &["label".to_string()],
            ),
            Ordering::Less,
        );
    }
}
