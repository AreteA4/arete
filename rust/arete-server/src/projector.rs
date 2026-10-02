use crate::bus::{BusManager, BusMessage};
use crate::cache::{CacheWrite, EntityCache, PatchOrigin};
use crate::mutation_batch::{MutationBatch, SlotContext};
use crate::view::{ViewIndex, ViewSpec};
use crate::websocket::frame::{apply_wire_format, Mode, SourceFrame};
use arete_interpreter::vm::{VmContext, WholeEntityRequests};
use arete_interpreter::{AccountPosition, CanonicalLog, WholeEntity};
use bytes::Bytes;
use lru::LruCache;
use serde_json::Value;
use smallvec::SmallVec;
use std::collections::HashSet;
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::mpsc;
use tracing::{debug, debug_span, error, instrument, warn};

#[cfg(feature = "otel")]
use crate::metrics::Metrics;

/// Whole-entity requests the projector keeps outstanding at once. One per
/// refused key; asking again after one is forgotten is always safe. The same
/// bound holds for the positions their resends keep.
const WHOLE_ENTITY_REQUEST_CAPACITY: usize = 4_096;

#[derive(Debug, Clone, PartialEq, Eq)]
struct RememberedChange {
    seq: Option<String>,
    account_position: Option<AccountPosition>,
}

/// `(export, key)` to the recency and account lifetime of its latest change.
type LastChanges = LruCache<(String, String), RememberedChange>;

/// The projector's way back to the VM its mutations come from.
///
/// The entity cache is bounded. A patch for a key it does not hold carries
/// only the fields that changed and cannot rebuild the entity, unless it is
/// the mutation that created the entity. A linked VM marks those
/// ([`WholeEntity::Created`]), so the cache stores a creation and refuses any
/// other patch for a key it lacks, whether it evicted the key or never held
/// it. The projector then asks the VM, which holds the whole entity, to send
/// all of it at the end of its next batch of mutations, whether or not the
/// key changes in it (see [`WholeEntityRequests`]).
///
/// A resend is the entity again, not a change, so it must not take the
/// position (`_seq`) of the batch it arrives in: in a view ordered by recency
/// it would rank ahead of entities that really changed there. The projector
/// remembers the position of each requested key's latest change — the refused
/// patch, or any change it sees for the key after it — and stamps the resend
/// with that, in the cache, in derived views and on the `upsert` it
/// publishes. It learns a key's changes in stream order, which the VM cannot:
/// positions are assigned per batch, after the VM has emitted it. A resend
/// with no remembered position (one past the bound, or from a source without
/// a VM, which sends whole entities as changes) takes its batch's.
/// For an account-owned entity it also remembers the explicit account position
/// at request time, so a whole entity produced for an old lifetime cannot land
/// after deletion and recreation.
///
/// The generated runtime hands its VM over through
/// [`crate::snapshot::register_runtime`], inside [`Self::scope`], before it
/// emits anything. A mutation source that registers no VM marks nothing and
/// cannot be asked: the cache takes a patch for a key it lacks for a new
/// entity unless it remembers evicting the key, and a remembered key stays
/// out until the source sends a mutation marked whole
/// ([`arete_interpreter::Mutation::mark_whole_entity`]) or a source delete.
#[derive(Clone)]
pub struct EntityResync {
    requests: WholeEntityRequests,
    /// For each key with a resend on its way: the `_seq` of its latest change
    /// and the account lifetime for which it was requested. Capped by hand at
    /// [`WHOLE_ENTITY_REQUEST_CAPACITY`], least recently changed out first.
    last_changes: Arc<StdMutex<LastChanges>>,
    linked: Arc<AtomicBool>,
    warned_unlinked: Arc<AtomicBool>,
}

tokio::task_local! {
    static ACTIVE_ENTITY_RESYNC: EntityResync;
}

impl Default for EntityResync {
    fn default() -> Self {
        Self::new()
    }
}

impl EntityResync {
    pub fn new() -> Self {
        Self {
            requests: WholeEntityRequests::new(WHOLE_ENTITY_REQUEST_CAPACITY),
            last_changes: Arc::new(StdMutex::new(LruCache::unbounded())),
            linked: Arc::new(AtomicBool::new(false)),
            warned_unlinked: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Run a mutation source (the generated parser) so that the VM it
    /// registers answers this projector's requests.
    pub async fn scope<F: Future>(&self, future: F) -> F::Output {
        ACTIVE_ENTITY_RESYNC.scope(self.clone(), future).await
    }

    /// Answer requests from `vm`.
    pub fn link(&self, vm: &Arc<StdMutex<VmContext>>) {
        vm.lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .set_whole_entity_requests(self.requests.clone());
        self.linked.store(true, Ordering::Release);
    }

    pub fn is_linked(&self) -> bool {
        self.linked.load(Ordering::Acquire)
    }

    /// What an unmarked patch from the source is: with a VM linked, which
    /// marks every creation, a change to an entity created earlier.
    fn unmarked_origin(&self) -> PatchOrigin {
        if self.is_linked() {
            PatchOrigin::Change
        } else {
            PatchOrigin::Unknown
        }
    }

    /// The request set, for inspection.
    pub fn requests(&self) -> &WholeEntityRequests {
        &self.requests
    }

    fn last_changes(&self) -> std::sync::MutexGuard<'_, LastChanges> {
        self.last_changes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Note a change to `export` `key` at `seq`. A refused change is about to
    /// be requested whole, so its position is remembered for the resend; any
    /// other change moves a remembered position forward. Only a linked VM
    /// resends.
    fn note_change(
        &self,
        export: &str,
        key: &str,
        seq: &Option<String>,
        account_position: Option<AccountPosition>,
        refused: bool,
    ) {
        if !self.is_linked() {
            return;
        }
        let mut last_changes = self.last_changes();
        if refused {
            last_changes.put(
                (export.to_string(), key.to_string()),
                RememberedChange {
                    seq: seq.clone(),
                    account_position,
                },
            );
            while last_changes.len() > WHOLE_ENTITY_REQUEST_CAPACITY {
                last_changes.pop_lru();
            }
        } else if !last_changes.is_empty() {
            if let Some(last) = last_changes.get_mut(&(export.to_string(), key.to_string())) {
                last.seq = seq.clone();
                if account_position.is_some() {
                    last.account_position = account_position;
                }
            }
        }
    }

    /// The recency and account-lifetime position a resend of `export` `key`
    /// keeps, if one is remembered. Forgets it.
    fn take_last_change(&self, export: &str, key: &str) -> Option<RememberedChange> {
        let mut last_changes = self.last_changes();
        if last_changes.is_empty() {
            return None;
        }
        last_changes.pop(&(export.to_string(), key.to_string()))
    }

    fn request(&self, export: &str, key: &Value) {
        if self.is_linked() {
            self.requests.request(export, key);
        } else if !self.warned_unlinked.swap(true, Ordering::Relaxed) {
            warn!(
                export,
                "Entity cache refused a patch for an evicted key and no VM is registered to \
                 send the whole entity; the key stays out of snapshots until its source sends \
                 a mutation marked whole or deletes it. Raise max_entities_per_view if the \
                 working set exceeds it."
            );
        }
    }

    fn cancel(&self, export: &str, key: &Value, cache_key: &str) {
        self.requests.cancel(export, key);
        self.take_last_change(export, cache_key);
    }
}

/// Link `vm` to the projector of the server whose parser is running, if any.
/// Called by [`crate::snapshot::register_runtime`].
pub(crate) fn link_active_resync(vm: &Arc<StdMutex<VmContext>>) {
    let _ = ACTIVE_ENTITY_RESYNC.try_with(|resync| resync.link(vm));
}

/// The `_version` stamped into every frame's data: `{epoch}:{counter}`.
///
/// Clients order one key's frames by it. `_seq` cannot do that: every update
/// decoded from one transaction shares it, and within a slot account updates
/// and instructions number themselves differently. The counter follows the
/// order this projector merges changes into the entity cache, which is the
/// order that defines the cached entity. The epoch names this projector, so a
/// client never compares counters from another one (a restart, or a stack
/// loaded again).
///
/// An entity restored from a snapshot keeps the version it was saved with,
/// from an earlier epoch. Clients take a version from another epoch as newer,
/// and every change the restored entity takes is stamped in this one, so an
/// old version never follows a new one for the same key.
struct FrameVersions {
    epoch: String,
    last: AtomicU64,
}

impl FrameVersions {
    fn new() -> Self {
        // Eight hex digits: enough to tell one projector's lifetime from
        // another, and every frame carries them.
        let epoch = hex::encode(&uuid::Uuid::new_v4().as_bytes()[..4]);
        Self {
            epoch,
            last: AtomicU64::new(0),
        }
    }

    /// Stamp the next version into `data`. Mutations are objects; anything
    /// else has nowhere to carry one and goes out unversioned.
    fn stamp(&self, data: &mut Value) {
        let Value::Object(map) = data else {
            debug!("mutation is not an object; publishing it without a version");
            return;
        };
        let counter = self.last.fetch_add(1, Ordering::Relaxed) + 1;
        map.insert(
            "_version".to_string(),
            Value::String(format!("{}:{counter}", self.epoch)),
        );
    }
}

pub struct Projector {
    view_index: Arc<ViewIndex>,
    bus_manager: BusManager,
    entity_cache: EntityCache,
    mutations_rx: mpsc::Receiver<MutationBatch>,
    snapshot_runtime: Option<crate::snapshot::SnapshotRuntime>,
    journal: Option<Arc<crate::journal::EventJournal>>,
    resync: EntityResync,
    versions: FrameVersions,
    #[cfg(feature = "otel")]
    metrics: Option<Arc<Metrics>>,
}

impl Projector {
    #[cfg(feature = "otel")]
    pub fn new(
        view_index: Arc<ViewIndex>,
        bus_manager: BusManager,
        entity_cache: EntityCache,
        mutations_rx: mpsc::Receiver<MutationBatch>,
        metrics: Option<Arc<Metrics>>,
    ) -> Self {
        Self {
            view_index,
            bus_manager,
            entity_cache,
            mutations_rx,
            snapshot_runtime: None,
            journal: None,
            resync: EntityResync::new(),
            versions: FrameVersions::new(),
            metrics,
        }
    }

    #[cfg(not(feature = "otel"))]
    pub fn new(
        view_index: Arc<ViewIndex>,
        bus_manager: BusManager,
        entity_cache: EntityCache,
        mutations_rx: mpsc::Receiver<MutationBatch>,
    ) -> Self {
        Self {
            view_index,
            bus_manager,
            entity_cache,
            mutations_rx,
            snapshot_runtime: None,
            journal: None,
            resync: EntityResync::new(),
            versions: FrameVersions::new(),
        }
    }

    /// Associate projection progress with one server's snapshot lifecycle.
    pub fn with_snapshot_runtime(
        mut self,
        snapshot_runtime: crate::snapshot::SnapshotRuntime,
    ) -> Self {
        self.snapshot_runtime = Some(snapshot_runtime);
        self
    }

    /// Retain published events for replayable append subscriptions.
    pub fn with_journal(mut self, journal: Arc<crate::journal::EventJournal>) -> Self {
        self.journal = Some(journal);
        self
    }

    /// Ask the VM linked to `resync` for whole entities the cache refused.
    pub fn with_entity_resync(mut self, resync: EntityResync) -> Self {
        self.resync = resync;
        self
    }

    pub async fn run(mut self) {
        debug!("Projector started");

        let mut json_buffer = Vec::with_capacity(4096);

        while let Some(mut batch) = self.mutations_rx.recv().await {
            let mut log = CanonicalLog::new();
            log.set("phase", "projector");

            let batch_size = batch.len();
            let slot_context = batch.slot_context;
            let batch_span = debug_span!(
                parent: &batch.span,
                "projector.batch",
                batch.mutations = batch_size,
                batch.position = tracing::field::Empty,
                frames_published = tracing::field::Empty,
            );
            if let (Some(context), false) = (slot_context, batch_span.is_disabled()) {
                batch_span.record("batch.position", context.to_seq_string());
            }
            let _span_guard = batch_span.enter();
            let mut frames_published = 0u32;
            let mut errors = 0u32;

            if let Some(ctx) = batch.event_context.as_ref() {
                log.set("program", &ctx.program)
                    .set("event_kind", &ctx.event_kind)
                    .set("event_type", &ctx.event_type)
                    .set("account", &ctx.account)
                    .set("accounts_count", ctx.accounts_count);
            }

            // Keys this batch carries whole, resent or created: a refused
            // patch ahead of one needs no request of its own.
            let arriving_whole: HashSet<(String, String)> = batch
                .mutations
                .iter()
                .filter(|mutation| mutation.is_whole_entity())
                .map(|mutation| (mutation.export.clone(), Self::extract_key(&mutation.key)))
                .collect();

            for mutation in std::mem::take(&mut batch.mutations).into_iter() {
                #[cfg(feature = "otel")]
                let export = mutation.export.clone();

                match self
                    .process_mutation(mutation, slot_context, &arriving_whole, &mut json_buffer)
                    .await
                {
                    Ok(count) => frames_published += count,
                    Err(e) => {
                        error!("Failed to process mutation: {}", e);
                        errors += 1;
                    }
                }

                #[cfg(feature = "otel")]
                if let Some(ref metrics) = self.metrics {
                    metrics.record_mutation_processed(&export);
                }
            }

            // The batch is now applied to the caches: advance the snapshot
            // resume watermark, then release the processing guard transferred
            // by the VM producer. An exclusive snapshot cut cannot begin until
            // every earlier guarded batch reaches this point.
            if batch_size > 0 {
                if let Some(snapshot_runtime) = &self.snapshot_runtime {
                    snapshot_runtime.record_applied_batch(slot_context.map(|ctx| ctx.slot));
                }
            }
            drop(batch.snapshot_guard.take());

            // Flush markers remain useful to non-snapshot callers that need
            // to observe a drained projector queue.
            if let Some(ack) = batch.flush_ack.take() {
                let _ = ack.send(());
            }

            log.set("batch_size", batch_size)
                .set("frames_published", frames_published)
                .set("errors", errors);
            batch_span.record("frames_published", frames_published);

            #[cfg(feature = "otel")]
            if let Some(ref metrics) = self.metrics {
                metrics.record_projector_latency(log.duration_ms());
            }

            log.emit();
        }

        debug!("Projector stopped");
    }

    #[instrument(
        name = "projector.mutation",
        level = "debug",
        skip(self, mutation, slot_context, arriving_whole, json_buffer),
        fields(export = %mutation.export)
    )]
    async fn process_mutation(
        &self,
        mut mutation: arete_interpreter::Mutation,
        slot_context: Option<SlotContext>,
        arriving_whole: &HashSet<(String, String)>,
        json_buffer: &mut Vec<u8>,
    ) -> anyhow::Result<u32> {
        let specs = self.view_index.by_export(&mutation.export);

        if specs.is_empty() {
            return Ok(0);
        }

        let account_position = mutation
            .take_account_position()
            .map_err(anyhow::Error::msg)?;

        if mutation.is_delete() {
            let key = Self::extract_key(&mutation.key);
            let seq = slot_context.map(|ctx| ctx.to_seq_string());
            self.resync.cancel(&mutation.export, &mutation.key, &key);
            let mut published = 0;
            for spec in specs.iter().filter(|spec| spec.filters.matches(&key)) {
                if !self
                    .entity_cache
                    .delete(&spec.id, &key, account_position)
                    .await
                {
                    continue;
                }
                let sorted = self.view_index.sorted_caches();
                let mut caches = sorted.write().await;
                for derived in self.view_index.get_derived_views_for_source(&spec.id) {
                    if let Some(cache) = caches.get_mut(&derived.id) {
                        cache.remove(&key);
                    }
                }
                drop(caches);
                // Append tapes are historical events; deletion only clears
                // their current entity cache, not their retained history.
                if spec.mode == Mode::Append {
                    continue;
                }
                let frame = SourceFrame {
                    mode: spec.mode,
                    export: spec.id.clone(),
                    op: "delete",
                    key: key.clone(),
                    data: Value::Null,
                    append: Vec::new(),
                    seq: seq.clone(),
                    offset: None,
                };
                json_buffer.clear();
                serde_json::to_writer(&mut *json_buffer, &frame)?;
                self.publish_frame(
                    spec,
                    Arc::new(BusMessage {
                        key: key.clone(),
                        entity: spec.id.clone(),
                        payload: Arc::new(Bytes::copy_from_slice(json_buffer)),
                    }),
                )
                .await;
                published += 1;
            }
            return Ok(published);
        }
        let mark = mutation.take_whole_entity_mark();
        let whole = mark == Some(WholeEntity::Resent);
        let origin = match mark {
            Some(WholeEntity::Created) => PatchOrigin::Creation,
            _ => self.resync.unmarked_origin(),
        };
        let key = Self::extract_key(&mutation.key);
        let arete_interpreter::Mutation {
            export,
            key: source_key,
            mut patch,
            append,
        } = mutation;

        // The position (`_seq`) recency order sorts by: the batch's for a
        // change, the latest change's for a resend (see `EntityResync`).
        let batch_seq = slot_context.map(|ctx| ctx.to_seq_string());
        let remembered = if whole {
            self.resync.take_last_change(&export, &key)
        } else {
            None
        };
        let seq = if whole {
            remembered
                .as_ref()
                .map(|change| change.seq.clone())
                .unwrap_or(batch_seq)
        } else {
            batch_seq
        };
        let resend_account_position = remembered.and_then(|change| change.account_position);
        if let (Some(seq), Value::Object(map)) = (&seq, &mut patch) {
            map.insert("_seq".to_string(), Value::String(seq.clone()));
        }

        let matching_specs: SmallVec<[&ViewSpec; 4]> = specs
            .iter()
            .filter(|spec| spec.filters.matches(&key))
            .collect();

        let match_count = matching_specs.len();
        if match_count == 0 {
            return Ok(0);
        }
        let current_account_position = self
            .entity_cache
            .account_position(&matching_specs[0].id, &key)
            .await;
        let change_account_position = account_position.or(current_account_position);

        let mut frames_published = 0u32;
        let mut refused = false;

        for (i, spec) in matching_specs.into_iter().enumerate() {
            if !whole
                && !self
                    .entity_cache
                    .accepts_lifetime_mutation(
                        &spec.id,
                        &key,
                        origin == PatchOrigin::Creation,
                        account_position,
                    )
                    .await
            {
                continue;
            }
            let is_last = i == match_count - 1;
            let patch_data = if is_last {
                std::mem::take(&mut patch)
            } else {
                patch.clone()
            };

            // Stamped after projection, which a field list could otherwise
            // strip it from, and before the cache merge and the frame, so
            // both carry it.
            let mut projected = spec.projection.apply(patch_data);
            self.versions.stamp(&mut projected);
            let mut wire_data = projected.clone();
            apply_wire_format(&mut wire_data, &spec.wire_format);

            let seq = seq.clone();

            if whole {
                frames_published += self
                    .apply_whole_entity(
                        spec,
                        &key,
                        projected,
                        wire_data,
                        RememberedChange {
                            seq,
                            account_position: resend_account_position,
                        },
                        json_buffer,
                    )
                    .await?;
                continue;
            }

            // Replayable append views carry the offset the record is about to
            // take, so a live subscriber can checkpoint the same cursor a
            // replay would hand it. The frame is built inside the journal's
            // lock so the published offset is always the one the record gets.
            let journal = self
                .journal
                .as_ref()
                .filter(|journal| journal.is_enabled() && spec.mode == Mode::Append);

            let mut frame = SourceFrame {
                mode: spec.mode,
                export: spec.id.clone(),
                op: if mark == Some(WholeEntity::Created) && spec.mode != Mode::Append {
                    "upsert"
                } else {
                    "patch"
                },
                key: key.clone(),
                data: wire_data,
                append: append.clone(),
                seq,
                offset: None,
            };

            let retained = match journal {
                Some(journal) => {
                    journal
                        .append_with(&spec.id, &key, |offset| {
                            frame.offset = Some(offset);
                            json_buffer.clear();
                            serde_json::to_writer(&mut *json_buffer, &frame)?;
                            Ok::<_, anyhow::Error>(Arc::new(Bytes::copy_from_slice(json_buffer)))
                        })
                        .await?
                }
                None => None,
            };
            let payload = match retained {
                Some((_offset, payload)) => payload,
                // No tape, or a sealed one: the event still publishes, it just
                // carries no position to resume from.
                None => {
                    frame.offset = None;
                    json_buffer.clear();
                    serde_json::to_writer(&mut *json_buffer, &frame)?;
                    Arc::new(Bytes::copy_from_slice(json_buffer))
                }
            };

            let write = self
                .entity_cache
                .upsert_with_lifetime(
                    &spec.id,
                    &key,
                    projected,
                    &frame.append,
                    origin,
                    account_position,
                )
                .await;

            match write {
                CacheWrite::Refused { patch } => {
                    refused = true;
                    if spec.mode == Mode::List {
                        self.merge_into_held_derived_entities(&spec.id, &key, patch, &frame.append)
                            .await;
                    }
                }
                CacheWrite::Merged | CacheWrite::Created => {
                    if spec.mode == Mode::List {
                        self.update_derived_view_caches(&spec.id, &key).await;
                    }
                }
            }

            let message = Arc::new(BusMessage {
                key: key.clone(),
                entity: spec.id.clone(),
                payload,
            });

            self.publish_frame(spec, message).await;
            frames_published += 1;

            #[cfg(feature = "otel")]
            if let Some(ref metrics) = self.metrics {
                let mode_str = match spec.mode {
                    Mode::List => "list",
                    Mode::State => "state",
                    Mode::Append => "append",
                };
                metrics.record_frame_published(mode_str, &spec.export);
            }
        }

        if !whole {
            self.resync
                .note_change(&export, &key, &seq, change_account_position, refused);
        }
        if refused && !arriving_whole.contains(&(export.clone(), key.clone())) {
            self.resync.request(&export, &source_key);
        }

        Ok(frames_published)
    }

    /// Store a whole entity the VM resent for a key the cache had to refuse,
    /// and tell list and state subscribers. `projected` and `seq` carry the
    /// position of the entity's latest change, not the resend's batch.
    ///
    /// It is state, not an event: append views get only their cache entry, and
    /// neither the journal nor their tape subscribers see it. List and state
    /// views publish it as an `upsert`, which the WebSocket layer hands to a
    /// client that holds the key and uses to (re)admit it to windows.
    async fn apply_whole_entity(
        &self,
        spec: &ViewSpec,
        key: &str,
        projected: Value,
        wire_data: Value,
        position: RememberedChange,
        json_buffer: &mut Vec<u8>,
    ) -> anyhow::Result<u32> {
        if !self
            .entity_cache
            .store_whole_for_lifetime(&spec.id, key, projected, position.account_position)
            .await
        {
            return Ok(0);
        }
        match spec.mode {
            Mode::Append => return Ok(0),
            Mode::List => self.update_derived_view_caches(&spec.id, key).await,
            Mode::State => {}
        }
        let frame = SourceFrame {
            mode: spec.mode,
            export: spec.id.clone(),
            op: "upsert",
            key: key.to_string(),
            data: wire_data,
            append: Vec::new(),
            seq: position.seq,
            offset: None,
        };
        json_buffer.clear();
        serde_json::to_writer(&mut *json_buffer, &frame)?;
        let message = Arc::new(BusMessage {
            key: key.to_string(),
            entity: spec.id.clone(),
            payload: Arc::new(Bytes::copy_from_slice(json_buffer)),
        });
        self.publish_frame(spec, message).await;
        Ok(1)
    }

    pub(crate) fn extract_key(key: &serde_json::Value) -> String {
        key.as_str()
            .map(|s| s.to_string())
            .or_else(|| key.as_u64().map(|n| n.to_string()))
            .or_else(|| key.as_i64().map(|n| n.to_string()))
            .or_else(|| {
                key.as_array().and_then(|arr| {
                    let bytes: Vec<u8> = arr
                        .iter()
                        .filter_map(|v| v.as_u64().map(|n| n as u8))
                        .collect();
                    if bytes.len() == arr.len() {
                        Some(hex::encode(&bytes))
                    } else {
                        None
                    }
                })
            })
            .unwrap_or_else(|| key.to_string())
    }

    async fn update_derived_view_caches(&self, source_view_id: &str, entity_key: &str) {
        let derived_views = self.view_index.get_derived_views_for_source(source_view_id);
        if derived_views.is_empty() {
            return;
        }

        let entity_data = match self.entity_cache.get(source_view_id, entity_key).await {
            Some(data) => data,
            None => return,
        };

        // Bound each derived sorted copy by the source view's cache size,
        // evicting from the bottom of the sort order (see `SortedViewCache`).
        let max_entries = self.entity_cache.max_entities_per_view();
        let sorted_caches = self.view_index.sorted_caches();
        let mut caches = sorted_caches.write().await;

        // A derived view holds only the entities its filter passes, so one
        // that stops passing leaves it. Of the rest, only a view that would
        // keep the entity gets a copy of it: once a view is full, most updates
        // sort below its last entry. The last one gets the entity itself.
        let mut keeping: SmallVec<[&str; 4]> = SmallVec::new();
        for spec in &derived_views {
            let Some(cache) = caches.get_mut(&spec.id) else {
                continue;
            };
            let passes = spec
                .pipeline
                .as_ref()
                .and_then(|pipeline| pipeline.filter.as_ref())
                .is_none_or(|filter| filter.matches(&entity_data));
            if !passes {
                cache.remove(entity_key);
                continue;
            }
            if cache.would_keep(entity_key, &entity_data, max_entries) {
                keeping.push(spec.id.as_str());
            }
        }
        let mut entity_data = Some(entity_data);
        for (index, view_id) in keeping.iter().enumerate() {
            let Some(cache) = caches.get_mut(*view_id) else {
                continue;
            };
            let entity = if index + 1 == keeping.len() {
                entity_data.take()
            } else {
                entity_data.clone()
            };
            let Some(entity) = entity else {
                continue;
            };
            cache.upsert_bounded(entity_key.to_string(), entity, max_entries);
            debug!(
                "Updated sorted cache for derived view {} with key {}",
                view_id, entity_key
            );
        }
    }

    /// Apply a patch the entity cache refused (its key was evicted there) to
    /// the derived views that still hold the whole entity.
    ///
    /// A derived view is bounded by sort position, not recency, so it can keep
    /// a top-ranked entity the entity cache has evicted; that copy is whole,
    /// so the patch merges into it exactly as it would have in the entity
    /// cache. A view that does not hold the key gets nothing: the patch alone
    /// is not an entity, and inserting it would rank a handful of changed
    /// fields as though they were the whole entity.
    async fn merge_into_held_derived_entities(
        &self,
        source_view_id: &str,
        entity_key: &str,
        patch: Value,
        append_paths: &[String],
    ) {
        let derived_views = self.view_index.get_derived_views_for_source(source_view_id);
        if derived_views.is_empty() {
            return;
        }
        let max_entries = self.entity_cache.max_entities_per_view();
        let sorted_caches = self.view_index.sorted_caches();
        let mut caches = sorted_caches.write().await;
        for spec in &derived_views {
            let Some(cache) = caches.get_mut(&spec.id) else {
                continue;
            };
            let Some(mut entity) = cache.get(entity_key).cloned() else {
                continue;
            };
            self.entity_cache
                .merge_patch(&mut entity, patch.clone(), append_paths);
            let passes = spec
                .pipeline
                .as_ref()
                .and_then(|pipeline| pipeline.filter.as_ref())
                .is_none_or(|filter| filter.matches(&entity));
            if passes {
                // Replace rather than merge: `entity` is already the whole
                // merged value, and the cache's own merge knows nothing of
                // append paths.
                cache.remove(entity_key);
                cache.upsert_bounded(entity_key.to_string(), entity, max_entries);
            } else {
                cache.remove(entity_key);
            }
        }
    }

    #[instrument(
        name = "projector.publish",
        level = "debug",
        skip(self, spec, message),
        fields(view_id = %spec.id, mode = ?spec.mode)
    )]
    async fn publish_frame(&self, spec: &ViewSpec, message: Arc<BusMessage>) {
        match spec.mode {
            Mode::State => {
                self.bus_manager
                    .publish_state(&spec.id, &message.key, message.payload.clone())
                    .await;
            }
            Mode::List | Mode::Append => {
                self.bus_manager.publish_list(&spec.id, message).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn linked() -> EntityResync {
        let resync = EntityResync::new();
        resync.link(&Arc::new(StdMutex::new(VmContext::new())));
        resync
    }

    fn seq(slot: u64) -> Option<String> {
        Some(SlotContext::new(slot, 0).to_seq_string())
    }

    /// Each refusal moves the position forward, and so does any change seen
    /// before the resend; the resend takes the position once.
    #[test]
    fn a_resend_keeps_the_latest_change_seen_before_it() {
        let resync = linked();
        let lifetime = Some(AccountPosition::new(4, 3));
        resync.note_change("Round", "1", &seq(5), lifetime, true);
        resync.note_change("Round", "1", &seq(6), lifetime, true);
        resync.note_change("Round", "2", &seq(7), None, false);
        assert_eq!(resync.take_last_change("Round", "2"), None, "never refused");
        resync.note_change("Round", "1", &seq(8), None, false);
        assert_eq!(
            resync.take_last_change("Round", "1"),
            Some(RememberedChange {
                seq: seq(8),
                account_position: lifetime,
            })
        );
        assert_eq!(resync.take_last_change("Round", "1"), None);
    }

    #[test]
    fn remembered_positions_are_bounded_like_requests() {
        let resync = linked();
        for key in 0..=WHOLE_ENTITY_REQUEST_CAPACITY {
            resync.note_change("Round", &key.to_string(), &seq(key as u64), None, true);
        }
        assert_eq!(
            resync.take_last_change("Round", "0"),
            None,
            "the oldest goes"
        );
        let newest = WHOLE_ENTITY_REQUEST_CAPACITY;
        assert_eq!(
            resync.take_last_change("Round", &newest.to_string()),
            Some(RememberedChange {
                seq: seq(newest as u64),
                account_position: None,
            })
        );
    }

    /// A source without a VM gets no resends, so nothing is remembered: a
    /// whole entity it sends is a change and takes its batch's position.
    #[test]
    fn nothing_is_remembered_without_a_vm() {
        let resync = EntityResync::new();
        resync.note_change("Round", "1", &seq(5), None, true);
        assert_eq!(resync.take_last_change("Round", "1"), None);
    }
}
