//! Opt-in periodic snapshots of in-memory server state (VM entity tables +
//! projection caches) and a restore path that rehydrates on startup, so a
//! restarted server comes back with its history instead of starting empty.
//!
//! arete-server owns all snapshot logic. The generated runtime's only
//! responsibilities are (1) registering the `VmContext`/`SlotTracker` it
//! creates via [`register_runtime`] and (2) hydrating from a restored blob via
//! [`take_restored`] before connecting to Yellowstone. Those hooks resolve
//! through a task-local [`SnapshotRuntime`], so multiple servers embedded in
//! one process cannot consume or replace each other's snapshot state. A stack
//! built with an older arete-macros simply never registers a VM; snapshots
//! stay disabled with a warning.
//!
//! Consistency cut: every generated VM update holds a shared snapshot barrier
//! guard until its mutation batch has been applied by the projector. Snapshot
//! capture takes the exclusive guard before dumping either side, so the VM,
//! projection caches, and resume watermark all describe the same processing
//! cut. On restore the stream replays from that watermark; the snapshotted
//! version trackers drop the overlap.

pub mod envelope;
#[cfg(feature = "snapshot-object-store")]
pub mod object;
pub mod store;

pub use envelope::{SnapshotContract, SnapshotHeader, SnapshotPayload};
#[cfg(feature = "snapshot-object-store")]
pub use object::ObjectSnapshotStore;
pub use store::{FsStore, SnapshotStore};

use crate::cache::EntityCache;
use crate::health::SlotTracker;
use crate::mutation_batch::MutationBatch;
use crate::view::ViewIndex;
use anyhow::{Context, Result};
use arete_interpreter::snapshot::{VmSnapshot, SNAPSHOT_FORMAT_VERSION};
use arete_interpreter::vm::VmContext;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock};
use tracing::{debug, info, info_span, warn, Instrument};

/// Rough Solana slot duration, used only to convert snapshot age into an
/// estimated slot distance for the staleness clamp.
const ESTIMATED_SLOT_MILLIS: u64 = 400;
/// How long a snapshot cycle waits for in-flight VM updates and their queued
/// projection batches to finish.
const CONSISTENCY_CUT_TIMEOUT: Duration = Duration::from_secs(10);
const STATE_CONTRACT_SCHEMA_V1: &str = "arete.snapshot-state-contract/v1";
const PROJECTION_CONTRACT_SCHEMA_V1: &str = "arete.snapshot-projection-contract/v1";

/// Configuration for state snapshots. Disabled by default; enable via
/// `ServerBuilder::snapshots(...)` or `ARETE_SNAPSHOT_*` env vars.
#[derive(Clone, Debug)]
pub struct SnapshotConfig {
    /// Master opt-in.
    pub enabled: bool,
    /// Where blobs live: `file:///var/lib/arete/snapshots`, a plain path, or
    /// (with the `snapshot-object-store` feature) `s3://`/`gs://`/`az://`.
    pub url: Option<String>,
    /// Periodic snapshot cadence.
    pub interval: Duration,
    /// Retained snapshots; older ones are pruned after each write.
    pub keep: usize,
    /// Take a final snapshot on SIGTERM/SIGINT before exit.
    pub snapshot_on_shutdown: bool,
    /// Skip a periodic cycle when fewer batches were applied since the last
    /// snapshot (quiet stacks snapshot rarely).
    pub min_mutations: u64,
    /// If the snapshot is older than this many (estimated) slots, hydrate
    /// state but start the stream live instead of resuming from the watermark.
    pub max_resume_age_slots: u64,
    /// `/ready` stays 503 after a watermark resume until the projector is
    /// within this many slots of the observed tip...
    pub ready_max_lag_slots: u64,
    /// ...or until this much time has passed (guards quiet stacks, where the
    /// watermark never advances because nothing happens on-chain).
    pub ready_max_hold: Duration,
    /// Bytecode hashes from pre-contract snapshots that an operator has
    /// explicitly approved for one state-only migration. These snapshots are
    /// hydrated after structural state-id remapping and always start live.
    pub legacy_bytecode_hashes: BTreeSet<String>,
}

impl Default for SnapshotConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            url: None,
            interval: Duration::from_secs(60),
            keep: 4,
            snapshot_on_shutdown: true,
            min_mutations: 1,
            // ~10 minutes of slots: conservative vs. typical provider
            // `from_slot` replay windows (in-cluster richat rings are far
            // more generous; raw Triton is minutes).
            max_resume_age_slots: 1_500,
            ready_max_lag_slots: 50,
            ready_max_hold: Duration::from_secs(60),
            legacy_bytecode_hashes: BTreeSet::new(),
        }
    }
}

impl SnapshotConfig {
    /// Load snapshot settings from `ARETE_SNAPSHOT_*` env vars. Snapshots stay
    /// disabled unless `ARETE_SNAPSHOT_ENABLED=true`.
    pub fn from_env() -> Result<Self> {
        let mut config = Self::default();
        config.enabled = crate::config::env_bool("ARETE_SNAPSHOT_ENABLED")?.unwrap_or(false);
        config.url = std::env::var("ARETE_SNAPSHOT_URL")
            .ok()
            .filter(|value| !value.trim().is_empty());
        config.interval = Duration::from_secs(
            crate::config::env_parse("ARETE_SNAPSHOT_INTERVAL_SECS")?
                .unwrap_or(config.interval.as_secs()),
        );
        config.keep = crate::config::env_parse("ARETE_SNAPSHOT_KEEP")?.unwrap_or(config.keep);
        config.snapshot_on_shutdown = crate::config::env_bool("ARETE_SNAPSHOT_ON_SHUTDOWN")?
            .unwrap_or(config.snapshot_on_shutdown);
        config.min_mutations = crate::config::env_parse("ARETE_SNAPSHOT_MIN_MUTATIONS")?
            .unwrap_or(config.min_mutations);
        config.max_resume_age_slots =
            crate::config::env_parse("ARETE_SNAPSHOT_MAX_RESUME_AGE_SLOTS")?
                .unwrap_or(config.max_resume_age_slots);
        config.ready_max_lag_slots =
            crate::config::env_parse("ARETE_SNAPSHOT_READY_MAX_LAG_SLOTS")?
                .unwrap_or(config.ready_max_lag_slots);
        config.ready_max_hold = Duration::from_secs(
            crate::config::env_parse("ARETE_SNAPSHOT_READY_MAX_HOLD_SECS")?
                .unwrap_or(config.ready_max_hold.as_secs()),
        );
        config.legacy_bytecode_hashes = std::env::var("ARETE_SNAPSHOT_LEGACY_BYTECODE_HASHES")
            .ok()
            .into_iter()
            .flat_map(|value| {
                value
                    .split(',')
                    .map(str::trim)
                    .filter(|hash| !hash.is_empty())
                    .map(str::to_ascii_lowercase)
                    .collect::<Vec<_>>()
            })
            .collect();
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        if self.enabled && self.url.as_deref().is_none_or(|url| url.trim().is_empty()) {
            anyhow::bail!("snapshots are enabled but ARETE_SNAPSHOT_URL is not set");
        }
        if self.enabled && (self.interval.is_zero() || self.keep == 0) {
            anyhow::bail!("snapshot interval and keep count must be greater than zero");
        }
        for hash in &self.legacy_bytecode_hashes {
            if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                anyhow::bail!(
                    "ARETE_SNAPSHOT_LEGACY_BYTECODE_HASHES contains invalid SHA-256 hash '{hash}'"
                );
            }
        }
        Ok(())
    }
}

fn contract_hash<T: Serialize>(schema: &str, value: &T) -> SnapshotContract {
    let canonical = arete_hash::canonicalize_jcs(value)
        .expect("snapshot contracts contain only canonical JSON values");
    let hash = hex::encode(Sha256::digest(canonical));
    SnapshotContract {
        schema: schema.to_string(),
        hash,
    }
}

fn state_contract(spec: &crate::Spec) -> Option<SnapshotContract> {
    if spec.entity_specs.is_empty() {
        return None;
    }
    let mut entities = Vec::with_capacity(spec.entity_specs.len());
    for entity in &spec.entity_specs {
        let mut indexes = entity
            .identity
            .lookup_indexes
            .iter()
            .map(|index| (&index.field_name, &index.temporal_field))
            .collect::<Vec<_>>();
        indexes.sort();
        indexes.dedup();
        let fields = entity
            .field_mappings
            .iter()
            .map(|(path, field)| {
                (
                    path,
                    serde_json::json!({
                        "baseType": field.base_type,
                        "integerKind": field.integer_kind,
                        "isOptional": field.is_optional,
                        "isArray": field.is_array,
                        "innerType": field.inner_type,
                        "resolvedType": field.resolved_type,
                        "emit": field.emit,
                    }),
                )
            })
            .collect::<BTreeMap<_, _>>();
        entities.push(serde_json::json!({
            "name": entity.state_name,
            "primaryKeys": entity.identity.primary_keys,
            "lookupIndexes": indexes,
            "fields": fields,
        }));
    }
    entities.sort_by_key(|entity| entity["name"].as_str().unwrap_or_default().to_string());
    Some(contract_hash(
        STATE_CONTRACT_SCHEMA_V1,
        &serde_json::json!({"entities": entities}),
    ))
}

fn projection_contract(view_index: &ViewIndex) -> SnapshotContract {
    contract_hash(PROJECTION_CONTRACT_SCHEMA_V1, &view_index.snapshot_specs())
}

fn state_ids_by_entity(spec: &crate::Spec) -> HashMap<String, u32> {
    spec.bytecode
        .entities
        .iter()
        .map(|(name, entity)| (name.clone(), entity.state_id))
        .collect()
}

fn remap_snapshot_states(vm: &mut VmSnapshot, state_ids: &HashMap<String, u32>) -> Result<()> {
    let states = std::mem::take(&mut vm.states);
    for (_, table) in states {
        let state_id = state_ids
            .get(&table.entity_name)
            .with_context(|| format!("snapshot contains unknown entity '{}'", table.entity_name))?;
        if vm.states.insert(*state_id, table).is_some() {
            anyhow::bail!("snapshot contains duplicate entity state for id {state_id}");
        }
    }
    Ok(())
}

/// VM state handed from the restore path to the generated runtime, consumed
/// exactly once via [`take_restored`].
pub struct RestoredState {
    pub vm: VmSnapshot,
    /// `Some(slot)` to resume the Yellowstone stream from that slot; `None`
    /// when the snapshot was too stale (state still hydrates, stream starts
    /// live and account-derived state self-heals).
    pub resume_watermark: Option<u64>,
}

#[derive(Clone)]
struct RuntimeRegistration {
    vm: Arc<StdMutex<VmContext>>,
    slot_tracker: SlotTracker,
}

struct ResumeGate {
    started: Instant,
    max_lag_slots: u64,
    max_hold: Duration,
}

/// Per-runtime barrier that keeps snapshot capture from splitting a VM update
/// from the projection batch it produced.
///
/// Generated mutation producers enter the barrier in shared mode before
/// touching the VM and transfer the guard to their [`MutationBatch`]. The
/// projector releases it only after applying that batch. Snapshot capture
/// enters in exclusive mode, which therefore waits for both in-flight parser
/// work and queued projection work to finish.
#[derive(Clone, Default)]
pub struct SnapshotBarrier {
    inner: Arc<RwLock<()>>,
}

impl SnapshotBarrier {
    pub async fn enter_processing(&self) -> SnapshotProcessingGuard {
        SnapshotProcessingGuard(self.inner.clone().read_owned().await)
    }

    async fn enter_snapshot(&self) -> OwnedRwLockWriteGuard<()> {
        self.inner.clone().write_owned().await
    }
}

/// Shared processing guard carried by a mutation batch until projection is
/// complete. The inner guard is intentionally opaque outside arete-server.
pub struct SnapshotProcessingGuard(#[allow(dead_code)] OwnedRwLockReadGuard<()>);

impl std::fmt::Debug for SnapshotProcessingGuard {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SnapshotProcessingGuard")
    }
}

#[derive(Default)]
struct SnapshotRuntimeState {
    registered: StdMutex<Option<RuntimeRegistration>>,
    restored: StdMutex<Option<RestoredState>>,
    resume_gate: StdMutex<Option<ResumeGate>>,
    processing_barrier: SnapshotBarrier,
    /// Highest slot among mutation batches this runtime's projector has
    /// applied. This is the safe `from_slot` resume point (`SlotTracker` is
    /// not: it follows the raw slot subscription, not parser progress).
    resume_watermark: AtomicU64,
    applied_batches: AtomicU64,
}

/// Per-server snapshot coordination shared by its parser, projector, snapshot
/// manager, and readiness endpoint.
///
/// The generated parser hooks use [`scope`](Self::scope) so their existing
/// argument-free calls cannot accidentally bind to another server running in
/// the same process.
#[derive(Clone, Default)]
pub struct SnapshotRuntime {
    state: Arc<SnapshotRuntimeState>,
}

tokio::task_local! {
    static ACTIVE_SNAPSHOT_RUNTIME: SnapshotRuntime;
}

impl SnapshotRuntime {
    /// Run a generated parser future with this server's snapshot state.
    pub async fn scope<F>(&self, future: F) -> F::Output
    where
        F: Future,
    {
        ACTIVE_SNAPSHOT_RUNTIME.scope(self.clone(), future).await
    }

    /// Associate the parser's VM and slot tracker with this server only.
    pub fn register_runtime(
        &self,
        vm: Arc<StdMutex<VmContext>>,
        slot_tracker: SlotTracker,
    ) -> SnapshotBarrier {
        let mut registered = self.state.registered.lock().unwrap();
        if registered.is_some() {
            debug!("Snapshot runtime registration replaced");
        }
        *registered = Some(RuntimeRegistration { vm, slot_tracker });
        self.state.processing_barrier.clone()
    }

    /// Consume this server's restored VM state exactly once.
    pub fn take_restored(&self) -> Option<RestoredState> {
        self.state.restored.lock().unwrap().take()
    }

    /// Record a batch applied by this server's projector.
    pub(crate) fn record_applied_batch(&self, slot: Option<u64>) {
        self.state.applied_batches.fetch_add(1, Ordering::Relaxed);
        if let Some(slot) = slot {
            self.state
                .resume_watermark
                .fetch_max(slot, Ordering::Relaxed);
        }
    }

    /// Returns `true` unless this server's watermark resume is still catching
    /// up to its observed slot tip.
    pub fn resume_gate_ready(&self) -> bool {
        let mut gate_slot = self.state.resume_gate.lock().unwrap();
        let Some(gate) = gate_slot.as_ref() else {
            return true;
        };
        if gate.started.elapsed() >= gate.max_hold {
            info!("Snapshot resume readiness gate released (max hold reached)");
            *gate_slot = None;
            return true;
        }
        let tip = self
            .state
            .registered
            .lock()
            .unwrap()
            .as_ref()
            .map(|registration| registration.slot_tracker.get())
            .unwrap_or(0);
        let applied = self.state.resume_watermark.load(Ordering::Relaxed);
        if tip > 0 && tip.saturating_sub(applied) <= gate.max_lag_slots {
            info!(tip, applied, "Snapshot resume caught up; marking ready");
            *gate_slot = None;
            return true;
        }
        false
    }
}

/// Called by the generated runtime after it creates its `VmContext` and
/// `SlotTracker`, so the snapshot manager can dump them later.
pub fn register_runtime(
    vm: Arc<StdMutex<VmContext>>,
    slot_tracker: SlotTracker,
) -> Option<SnapshotBarrier> {
    match ACTIVE_SNAPSHOT_RUNTIME.try_with(|runtime| runtime.register_runtime(vm, slot_tracker)) {
        Ok(barrier) => Some(barrier),
        Err(_) => {
            debug!("Snapshot runtime registration ignored (snapshots disabled)");
            None
        }
    }
}

/// Called by the generated runtime before connecting: returns the restored VM
/// state (if any) exactly once.
pub fn take_restored() -> Option<RestoredState> {
    ACTIVE_SNAPSHOT_RUNTIME
        .try_with(SnapshotRuntime::take_restored)
        .ok()
        .flatten()
}

/// Select a reconnect checkpoint for the generated Yellowstone runtime.
///
/// A restored replay never falls back to live: retries advance only to slots
/// the main parser stream has finished processing. Without a restored replay,
/// the existing live fallback remains available after repeated short-lived
/// connections.
#[doc(hidden)]
pub fn select_reconnect_from_slot(
    restored_watermark: Option<u64>,
    processed_watermark: u64,
    attempt: u32,
    live_fallback_attempts: u32,
) -> Option<u64> {
    if let Some(restored_watermark) = restored_watermark {
        return Some(restored_watermark.max(processed_watermark));
    }
    if attempt >= live_fallback_attempts {
        return None;
    }
    (processed_watermark > 0).then_some(processed_watermark)
}

fn now_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

/// What kicked off a snapshot cycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotTrigger {
    Periodic,
    Shutdown,
}

/// Owns the store plus everything needed to dump and restore state. Created by
/// `Runtime::run` when snapshots are enabled.
pub struct SnapshotService {
    config: SnapshotConfig,
    store: Arc<dyn SnapshotStore>,
    runtime: SnapshotRuntime,
    bytecode_hash: String,
    state_contract: Option<SnapshotContract>,
    projection_contract: SnapshotContract,
    state_ids: HashMap<String, u32>,
    program_ids: Vec<String>,
    entity_cache: EntityCache,
    batches_at_last_snapshot: AtomicU64,
    warned_missing_vm: AtomicBool,
}

impl SnapshotService {
    /// Build the store, then attempt a restore (any failure logs a warning
    /// and cold-starts — restore problems must never block startup).
    pub async fn initialize(
        config: SnapshotConfig,
        spec: &crate::Spec,
        entity_cache: EntityCache,
        view_index: &ViewIndex,
        _mutations_tx: mpsc::Sender<MutationBatch>,
    ) -> Result<Arc<Self>> {
        let url = config
            .url
            .clone()
            .context("snapshots are enabled but no snapshot URL is configured")?;
        let store = store::store_from_url(&url)?;

        let mut program_ids = spec.program_ids.clone();
        program_ids.sort();

        let service = Arc::new(Self {
            config,
            store,
            runtime: SnapshotRuntime::default(),
            bytecode_hash: spec.bytecode.fingerprint(),
            state_contract: state_contract(spec),
            projection_contract: projection_contract(view_index),
            state_ids: state_ids_by_entity(spec),
            program_ids,
            entity_cache,
            batches_at_last_snapshot: AtomicU64::new(0),
            warned_missing_vm: AtomicBool::new(false),
        });
        info!(
            store = %service.store.describe(),
            interval_secs = service.config.interval.as_secs(),
            keep = service.config.keep,
            "State snapshots enabled"
        );

        match service.restore(view_index).await {
            Ok(true) => {}
            Ok(false) => info!("No usable snapshot found; starting cold"),
            Err(err) => warn!(
                error = format!("{err:#}"),
                "Failed to restore snapshot; starting cold"
            ),
        }
        Ok(service)
    }

    pub fn config(&self) -> &SnapshotConfig {
        &self.config
    }

    /// Return the per-server coordination handle that must be shared with the
    /// matching parser, projector, and readiness endpoint.
    pub fn runtime(&self) -> SnapshotRuntime {
        self.runtime.clone()
    }

    /// Load and validate the latest snapshot, hydrate the projection caches,
    /// and stash the VM portion for the generated runtime. Returns whether a
    /// snapshot was applied.
    async fn restore(&self, view_index: &ViewIndex) -> Result<bool> {
        let Some((name, bytes)) = self.store.load_latest().await? else {
            return Ok(false);
        };

        let header = envelope::decode_header(&bytes)
            .with_context(|| format!("snapshot {name} has an unreadable header"))?;

        if header.format_version != SNAPSHOT_FORMAT_VERSION {
            warn!(
                snapshot = %name,
                found = header.format_version,
                expected = SNAPSHOT_FORMAT_VERSION,
                "Snapshot format version mismatch; discarding (cold start)"
            );
            return Ok(false);
        }
        let exact_bytecode = header.bytecode_hash == self.bytecode_hash;
        let matching_contracts = self.state_contract.is_some()
            && header.state_contract == self.state_contract
            && header.projection_contract.as_ref() == Some(&self.projection_contract);
        let approved_legacy = self
            .config
            .legacy_bytecode_hashes
            .contains(&header.bytecode_hash);
        let legacy_migration = approved_legacy && !exact_bytecode && !matching_contracts;
        if !exact_bytecode && !matching_contracts && !approved_legacy {
            warn!(
                snapshot = %name,
                "Snapshot was taken by an incompatible stack build; discarding (cold start)"
            );
            return Ok(false);
        }
        let mut snapshot_program_ids = header.program_ids.clone();
        snapshot_program_ids.sort();
        if snapshot_program_ids != self.program_ids {
            warn!(
                snapshot = %name,
                "Snapshot program ids do not match this server; discarding (cold start)"
            );
            return Ok(false);
        }

        let mut payload = tokio::task::spawn_blocking(move || envelope::decode_payload(&bytes))
            .await
            .context("snapshot decode task panicked")?
            .with_context(|| format!("snapshot {name} has an unreadable payload"))?;
        if !exact_bytecode {
            remap_snapshot_states(&mut payload.vm, &self.state_ids)
                .with_context(|| format!("snapshot {name} state contract is incompatible"))?;
        }
        if legacy_migration {
            // A pre-contract snapshot cannot prove that its materialized views
            // still match the current projections. Preserve only durable VM
            // state and let live input rebuild every projection cache.
            payload.entity_cache.clear();
        }

        let cached_views = payload.entity_cache.len();
        let cached_entities: usize = payload
            .entity_cache
            .iter()
            .map(|(_, entries)| entries.len())
            .sum();
        self.entity_cache.hydrate(payload.entity_cache).await;
        rebuild_sorted_caches(view_index, &self.entity_cache).await;

        // Even when the stream starts live, the watermark seeds the applied
        // position: the hydrated state already contains everything up to it.
        self.runtime
            .state
            .resume_watermark
            .fetch_max(header.resume_watermark, Ordering::Relaxed);

        let age_ms = now_epoch_ms().saturating_sub(header.created_at_epoch_ms);
        let estimated_age_slots = age_ms / ESTIMATED_SLOT_MILLIS;
        let resume_watermark = if exact_bytecode
            && header.resume_watermark > 0
            && estimated_age_slots <= self.config.max_resume_age_slots
        {
            Some(header.resume_watermark)
        } else {
            if header.resume_watermark > 0 && exact_bytecode {
                warn!(
                    resume_watermark = header.resume_watermark,
                    estimated_age_slots,
                    max_resume_age_slots = self.config.max_resume_age_slots,
                    "Snapshot is older than the resume window; hydrating state but \
                     starting the stream live. Account-derived state self-heals from \
                     full account writes; only instruction events in the gap are missed."
                );
            }
            if !exact_bytecode {
                warn!(
                    snapshot = %name,
                    matching_contracts,
                    approved_legacy,
                    legacy_state_only = legacy_migration,
                    "Hydrated snapshot state from different bytecode; starting live"
                );
            }
            None
        };

        if resume_watermark.is_some() {
            *self.runtime.state.resume_gate.lock().unwrap() = Some(ResumeGate {
                started: Instant::now(),
                max_lag_slots: self.config.ready_max_lag_slots,
                max_hold: self.config.ready_max_hold,
            });
        }

        info!(
            snapshot = %name,
            vm_entities = payload.vm.total_entries(),
            cached_views,
            cached_entities,
            resume_watermark = header.resume_watermark,
            resuming = resume_watermark.is_some(),
            age_secs = age_ms / 1_000,
            "Restored state from snapshot"
        );

        *self.runtime.state.restored.lock().unwrap() = Some(RestoredState {
            vm: payload.vm,
            resume_watermark,
        });
        Ok(true)
    }

    /// Spawn the periodic snapshot task.
    pub fn spawn(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let service = Arc::clone(self);
        tokio::spawn(
            async move {
                let mut interval = tokio::time::interval(service.config.interval);
                interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                // The first tick fires immediately; skip it so the first
                // snapshot lands one full interval after startup.
                interval.tick().await;
                loop {
                    interval.tick().await;
                    if let Err(err) = service.snapshot_now(SnapshotTrigger::Periodic).await {
                        // Snapshotting must never take down a healthy server.
                        warn!(
                            error = format!("{err:#}"),
                            "Snapshot cycle failed; will retry next interval"
                        );
                    }
                }
            }
            .instrument(info_span!("snapshot.manager")),
        )
    }

    /// Run one snapshot cycle. Returns `Ok(false)` when skipped (no VM
    /// registered yet, or too few mutations since the last snapshot).
    pub async fn snapshot_now(&self, trigger: SnapshotTrigger) -> Result<bool> {
        let Some(registration) = self.runtime.state.registered.lock().unwrap().clone() else {
            if !self.warned_missing_vm.swap(true, Ordering::Relaxed) {
                warn!(
                    "Snapshots are enabled but no VM has been registered; the stack \
                     may have been built with an older arete-macros version"
                );
            }
            return Ok(false);
        };

        let applied_batches = self.runtime.state.applied_batches.load(Ordering::Relaxed);
        if trigger == SnapshotTrigger::Periodic {
            let since_last = applied_batches
                .saturating_sub(self.batches_at_last_snapshot.load(Ordering::Relaxed));
            if since_last < self.config.min_mutations {
                debug!(since_last, "Skipping snapshot cycle (too few mutations)");
                return Ok(false);
            }
        }

        // Wait for every in-flight VM update and its queued projection batch
        // to finish, then block new updates until both sides have been dumped.
        // Processing guards move with their mutation batches and are released
        // by the projector only after cache application, so this exclusive
        // guard establishes one exact cut without an enqueue race.
        let consistency_guard = tokio::time::timeout(
            CONSISTENCY_CUT_TIMEOUT,
            self.runtime.state.processing_barrier.enter_snapshot(),
        )
        .await
        .context("timed out waiting for a consistent VM/projection snapshot cut")?;

        let dump_started = Instant::now();
        let (vm_snapshot, resume_watermark) = {
            let vm = registration
                .vm
                .lock()
                .map_err(|_| anyhow::anyhow!("VM mutex poisoned"))?;
            let resume_watermark = self.runtime.state.resume_watermark.load(Ordering::Relaxed);
            let vm_snapshot = vm.dump();
            (vm_snapshot, resume_watermark)
        };
        let vm_lock_held = dump_started.elapsed();
        let observed_slot = registration.slot_tracker.get();
        let entity_cache_dump = self.entity_cache.dump().await;
        let applied_batches = self.runtime.state.applied_batches.load(Ordering::Relaxed);
        drop(consistency_guard);

        let created_at_epoch_ms = now_epoch_ms();
        let header = SnapshotHeader {
            format_version: SNAPSHOT_FORMAT_VERSION,
            bytecode_hash: self.bytecode_hash.clone(),
            state_contract: self.state_contract.clone(),
            projection_contract: Some(self.projection_contract.clone()),
            program_ids: self.program_ids.clone(),
            resume_watermark,
            observed_slot,
            created_at_epoch_ms,
            entry_counts: vm_snapshot.entry_counts().into_iter().collect(),
        };
        let payload = SnapshotPayload {
            vm: vm_snapshot,
            entity_cache: entity_cache_dump,
        };
        let bytes = tokio::task::spawn_blocking(move || envelope::encode(&header, &payload))
            .await
            .context("snapshot encode task panicked")??;

        let name = store::snapshot_name(created_at_epoch_ms, resume_watermark);
        self.store.write(&name, &bytes).await?;
        if let Err(err) = self.store.prune(self.config.keep).await {
            warn!(error = format!("{err:#}"), "Failed to prune old snapshots");
        }
        self.batches_at_last_snapshot
            .store(applied_batches, Ordering::Relaxed);

        info!(
            snapshot = %name,
            bytes = bytes.len(),
            resume_watermark,
            observed_slot,
            vm_lock_ms = vm_lock_held.as_millis() as u64,
            trigger = ?trigger,
            "Snapshot written"
        );
        Ok(true)
    }
}

/// Rebuild each derived `SortedViewCache` from the hydrated `EntityCache`,
/// mirroring the projector's upsert path. Sorted caches are not persisted:
/// they are derived state and MB-scale rebuilds are sub-millisecond.
async fn rebuild_sorted_caches(view_index: &ViewIndex, entity_cache: &EntityCache) {
    let sorted_caches = view_index.sorted_caches();
    let max_entries = entity_cache.max_entities_per_view();
    for spec in view_index.get_derived_views() {
        let Some(source_view) = spec.source_view.as_ref() else {
            continue;
        };
        let entities = entity_cache.get_all(source_view).await;
        if entities.is_empty() {
            continue;
        }
        let mut caches = sorted_caches.write().await;
        if let Some(cache) = caches.get_mut(&spec.id) {
            let count = entities.len();
            for (key, entity) in entities {
                cache.upsert(key, entity);
            }
            // Trim once after the batch; the bound matches the projector's.
            cache.trim_to_max_entries(max_entries);
            debug!(view_id = %spec.id, count, "Rebuilt sorted cache from snapshot");
        }
    }
}
