//! Entity cache for snapshot-on-subscribe functionality.
//!
//! This module provides an `EntityCache` that maintains full projected entities
//! in memory with LRU eviction. When a new client subscribes, they receive
//! cached snapshots immediately rather than waiting for the next live mutation.

use arete_interpreter::AccountPosition;
use lru::LruCache;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

const DEFAULT_MAX_ENTITIES_PER_VIEW: usize = 500;
const DEFAULT_MAX_ARRAY_LENGTH: usize = 100;
const DEFAULT_INITIAL_SNAPSHOT_BATCH_SIZE: usize = 50;
const DEFAULT_SUBSEQUENT_SNAPSHOT_BATCH_SIZE: usize = 100;
/// Evicted keys remembered per cached entity, for a source that does not mark
/// creations (see [`PatchOrigin::Unknown`]). Such a source can keep far more
/// entities than a view caches (a VM keeps 2,500 per state table by default
/// against 500 here), so a key the cache evicted can keep receiving patches
/// long afterwards; eight times the cache bound covers the VM's default table
/// with room to spare. A key evicted longer ago than that is taken for new.
///
/// A source that marks creations needs no memory: an unmarked patch for a key
/// a view does not hold is refused however long ago the key left, or whether
/// it was ever there.
const EVICTED_KEYS_PER_CACHED_ENTITY: usize = 8;

/// Whether a patch creates its entity, as far as its source says.
///
/// Every patch carries only the fields that changed. For a key a view holds
/// that is all it needs; for one it does not hold, the patch is the entity
/// only if it created it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchOrigin {
    /// The source marked the patch as its entity's creation
    /// ([`arete_interpreter::WholeEntity::Created`]), so it is all of the
    /// entity.
    Creation,
    /// The source marks every creation and did not mark this patch, so it
    /// changes an entity created earlier and is only part of it. A VM linked
    /// through [`crate::EntityResync`] is such a source.
    Change,
    /// The source does not mark creations. A patch for a key the view does not
    /// hold is taken for a new entity unless the view remembers evicting it.
    Unknown,
}

/// What [`EntityCache::upsert_with_append`] did with a patch.
#[derive(Debug, Clone, PartialEq)]
pub enum CacheWrite {
    /// Merged into the entity the cache already held.
    Merged,
    /// Stored as a new entity: the patch created it, or its source does not
    /// say and the cache has no record of the key.
    Created,
    /// Not stored. The cache does not hold the key and the patch carries only
    /// the fields that changed — part of an entity, not one: its source marks
    /// creations and did not mark this one, or the cache evicted the key. The
    /// patch is handed back for callers that hold a full copy elsewhere.
    Refused { patch: Value },
}

/// One view's cached entities and the keys it evicted to stay bounded.
///
/// Both maps are unbounded `LruCache`s capped by hand: a bounded one
/// allocates its full capacity up front, which for the evicted-key memory is
/// several times the entity bound, per view, whether or not anything is ever
/// evicted from it.
struct ViewEntries {
    max_entities: usize,
    entities: LruCache<String, Value>,
    /// Keys evicted for space, most recently evicted or refused first.
    /// Allocated on the first eviction, and never for a view fed by a source
    /// that marks creations.
    evicted: Option<LruCache<String, ()>>,
    /// Whether a source that marks creations writes to this view. Such a
    /// source never consults `evicted`.
    creations_marked: bool,
    // Recent authoritative account positions and deletion barriers survive
    // snapshots. They are intentionally separate from entity `_seq`, whose
    // offset may instead be an instruction txn_index.
    lifetimes: LruCache<String, EntityLifetimeCheckpoint>,
}

impl ViewEntries {
    fn new(max_entities: usize) -> Self {
        assert!(max_entities > 0, "max_entities_per_view must be > 0");
        Self {
            max_entities,
            entities: LruCache::unbounded(),
            evicted: None,
            creations_marked: false,
            lifetimes: LruCache::unbounded(),
        }
    }

    /// Note a patch from a source that marks creations: this view no longer
    /// needs to remember its evictions.
    fn mark_creations(&mut self) {
        self.creations_marked = true;
        self.evicted = None;
    }

    /// Store `entity` under `key`, remembering whatever it pushes out.
    fn store(&mut self, key: String, entity: Value) {
        self.forget_evicted(&key);
        self.entities.put(key, entity);
        while self.entities.len() > self.max_entities {
            match self.entities.pop_lru() {
                Some((evicted, _)) => self.remember_evicted(evicted),
                None => break,
            }
        }
    }

    fn remember_evicted(&mut self, key: String) {
        if self.creations_marked {
            return;
        }
        let capacity = self
            .max_entities
            .saturating_mul(EVICTED_KEYS_PER_CACHED_ENTITY);
        let evicted = self.evicted.get_or_insert_with(LruCache::unbounded);
        evicted.put(key, ());
        while evicted.len() > capacity {
            evicted.pop_lru();
        }
    }

    fn forget_evicted(&mut self, key: &str) {
        if let Some(evicted) = self.evicted.as_mut() {
            evicted.pop(key);
        }
    }

    /// Whether `key` was evicted and is still remembered. Asking refreshes
    /// it, so an evicted key that keeps changing is not forgotten.
    fn was_evicted(&mut self, key: &str) -> bool {
        self.evicted
            .as_mut()
            .is_some_and(|evicted| evicted.get(key).is_some())
    }

    fn accepts(
        &self,
        key: &str,
        creation: bool,
        account_position: Option<AccountPosition>,
        source_seq: Option<&str>,
        source_seq_is_ordered: bool,
    ) -> bool {
        if let Some(checkpoint) = self.lifetimes.peek(key) {
            match (checkpoint.account_position, account_position) {
                (Some(previous), Some(incoming)) if incoming <= previous => return false,
                // Once a producer opts into authoritative account ordering it
                // must carry it on every later account-owned lifetime change.
                (Some(_), None) if creation || checkpoint.deleted => return false,
                _ => {}
            }
            if checkpoint.deleted && !creation {
                return false;
            }
        }

        if account_position.is_none() && source_seq_is_ordered && source_seq.is_some() {
            return self.accepts_source_ordering(key, source_seq, !creation);
        }
        true
    }

    fn latest_source_seq<'a>(&'a self, key: &str) -> Option<&'a str> {
        let checkpoint = self.lifetimes.peek(key);
        // An account cursor and an instruction/source cursor are independent.
        // In particular, do not recover the account update's `_seq` from the
        // entity and compare its write version with an instruction index.
        if checkpoint.is_some_and(|checkpoint| checkpoint.account_position.is_some()) {
            return checkpoint.and_then(|checkpoint| checkpoint.source_seq.as_deref());
        }
        let checkpoint = checkpoint.and_then(|checkpoint| checkpoint.source_seq.as_deref());
        let entity = self
            .entities
            .peek(key)
            .and_then(|entity| entity.get("_seq"))
            .and_then(Value::as_str);
        match (checkpoint, entity) {
            (Some(left), Some(right)) if cmp_seq(left, right).is_lt() => Some(right),
            (Some(left), _) => Some(left),
            (None, entity) => entity,
        }
    }

    fn accepts_source_ordering(
        &self,
        key: &str,
        incoming: Option<&str>,
        allow_equal: bool,
    ) -> bool {
        match (self.latest_source_seq(key), incoming) {
            (Some(previous), Some(incoming)) => {
                let ordering = cmp_seq(incoming, previous);
                ordering.is_gt() || (allow_equal && ordering.is_eq())
            }
            // Once a markerless source supplies ordering, an unsequenced
            // lifetime change cannot prove that it is newer.
            (Some(_), None) => false,
            (None, _) => true,
        }
    }

    fn trim_lifetimes(&mut self) {
        while self.lifetimes.len()
            > self
                .max_entities
                .saturating_mul(EVICTED_KEYS_PER_CACHED_ENTITY)
        {
            self.lifetimes.pop_lru();
        }
    }
}

/// The authoritative account lifetime known for one projected entity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntityLifetimeCheckpoint {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_position: Option<AccountPosition>,
    /// Ordering for producers that do not carry an authoritative account
    /// position. This is never compared with `account_position`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_seq: Option<String>,
    pub deleted: bool,
}

/// `(view, key, checkpoint)` in most-recently-changed order.
pub type EntityLifetimes = Vec<(String, String, EntityLifetimeCheckpoint)>;

/// Independent ordering metadata attached to one projected cache write.
#[derive(Debug, Clone, Copy, Default)]
pub struct LifetimeOrdering<'a> {
    pub account_position: Option<AccountPosition>,
    pub source_seq: Option<&'a str>,
    /// Whether `source_seq` belongs to one comparable recency domain. Legacy
    /// callers that can mix account and instruction offsets leave this false.
    pub source_seq_is_ordered: bool,
}

/// Legacy `(view, key, _seq)` barriers, retained only to restore snapshots
/// written before account positions had their own ordering domain.
pub type EntityTombstones = Vec<(String, String, Option<String>)>;

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

fn account_position_from_seq(seq: &str) -> Option<AccountPosition> {
    let mut parts = seq.splitn(2, ':');
    Some(AccountPosition::new(
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    ))
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
///
/// # Only whole entities
///
/// Source mutations carry only the fields that changed. A patch for a key a
/// view holds merges into the whole entity. A patch for a key it does not
/// hold is stored only if it is the whole entity: storing a few changed
/// fields would serve them as though they were the entity, in snapshots and
/// in every derived view.
///
/// A VM linked through [`crate::EntityResync`] marks the mutation that creates
/// each entity ([`PatchOrigin::Creation`]), which is stored as-is. Any other
/// patch from it for a key a view does not hold ([`PatchOrigin::Change`]) is
/// refused with [`CacheWrite::Refused`] — whether the view evicted the key or
/// never held it, as after a restore. The projector then asks the VM for the
/// whole entity, which follows in the VM's next batch and is stored with
/// [`EntityCache::store_whole`]. That holds however many entities the VM
/// keeps and however long ago the view evicted the key.
///
/// A source that marks nothing ([`PatchOrigin::Unknown`]) leaves a new key and
/// an evicted one looking alike. Each view therefore remembers the keys it
/// evicted (bounded, most recent first) and refuses patches for them; a key
/// evicted so long ago that it has been forgotten is taken for new. A source
/// delete also ends an eviction.
#[derive(Clone)]
pub struct EntityCache {
    /// view_id -> LRU<entity_key, full_projected_entity>, plus evicted keys
    caches: Arc<RwLock<HashMap<String, ViewEntries>>>,
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

    /// Maximum number of entities cached per view.
    ///
    /// Derived sorted view caches are bounded by the same value so they never
    /// hold more entities than their source view caches.
    pub fn max_entities_per_view(&self) -> usize {
        self.config.max_entities_per_view
    }

    /// Write a patch from a source that does not mark creations
    /// ([`PatchOrigin::Unknown`]).
    pub async fn upsert(&self, view_id: &str, key: &str, patch: Value) -> CacheWrite {
        self.upsert_with_append(view_id, key, patch, &[], PatchOrigin::Unknown)
            .await
    }

    /// Merge `patch` into the cached entity, store it as a new entity, or
    /// refuse it as part of an entity the cache does not hold (see the type
    /// docs).
    pub async fn upsert_with_append(
        &self,
        view_id: &str,
        key: &str,
        patch: Value,
        append_paths: &[String],
        origin: PatchOrigin,
    ) -> CacheWrite {
        self.upsert_with_lifetime(view_id, key, patch, append_paths, origin, None)
            .await
    }

    /// Apply a projected patch with optional authoritative account ordering.
    /// `_seq` is consulted for lifetime changes only when no account position
    /// exists; the two ordering domains are never compared.
    pub async fn upsert_with_lifetime(
        &self,
        view_id: &str,
        key: &str,
        patch: Value,
        append_paths: &[String],
        origin: PatchOrigin,
        account_position: Option<AccountPosition>,
    ) -> CacheWrite {
        let source_seq = patch.get("_seq").and_then(Value::as_str).map(str::to_owned);
        self.upsert_with_ordering(
            view_id,
            key,
            patch,
            append_paths,
            origin,
            LifetimeOrdering {
                account_position,
                source_seq: source_seq.as_deref(),
                // The convenience path supports markerless lifetime ordering,
                // but sparse legacy patches may mix account write versions
                // and instruction indices within one slot.
                source_seq_is_ordered: origin == PatchOrigin::Creation,
            },
        )
        .await
    }

    /// Apply a projected patch with both ordering domains supplied
    /// explicitly. Projections may omit `_seq`, so the projector passes its
    /// source cursor separately here.
    pub async fn upsert_with_ordering(
        &self,
        view_id: &str,
        key: &str,
        patch: Value,
        append_paths: &[String],
        origin: PatchOrigin,
        ordering: LifetimeOrdering<'_>,
    ) -> CacheWrite {
        let LifetimeOrdering {
            account_position,
            source_seq,
            source_seq_is_ordered,
        } = ordering;
        let mut caches = self.caches.write().await;

        let view = caches
            .entry(view_id.to_string())
            .or_insert_with(|| ViewEntries::new(self.config.max_entities_per_view));
        if origin != PatchOrigin::Unknown && !view.creations_marked {
            view.mark_creations();
        }

        let max_array_length = self.config.max_array_length;

        let creation = origin == PatchOrigin::Creation;
        if !view.accepts(
            key,
            creation,
            account_position,
            source_seq,
            source_seq_is_ordered,
        ) {
            return CacheWrite::Refused { patch };
        }

        if let Some(position) = account_position {
            let source_seq = (!creation)
                .then(|| {
                    view.lifetimes
                        .peek(key)
                        .and_then(|checkpoint| checkpoint.source_seq.clone())
                })
                .flatten();
            view.lifetimes.put(
                key.to_string(),
                EntityLifetimeCheckpoint {
                    account_position: Some(position),
                    source_seq,
                    deleted: false,
                },
            );
            view.trim_lifetimes();
        } else {
            if let Some(source_seq) = source_seq.filter(|_| source_seq_is_ordered) {
                let account_position = view
                    .lifetimes
                    .peek(key)
                    .and_then(|checkpoint| checkpoint.account_position);
                view.lifetimes.put(
                    key.to_string(),
                    EntityLifetimeCheckpoint {
                        account_position,
                        source_seq: Some(source_seq.to_string()),
                        deleted: false,
                    },
                );
                view.trim_lifetimes();
            } else if creation
                && view
                    .lifetimes
                    .peek(key)
                    .is_some_and(|checkpoint| checkpoint.source_seq.is_none())
            {
                // An entirely unsequenced source preserves its historical
                // recreate behavior; there is no ordering evidence to retain.
                view.lifetimes.pop(key);
            }
        }

        let whole = match origin {
            PatchOrigin::Creation => true,
            PatchOrigin::Change => view.entities.contains(key),
            PatchOrigin::Unknown => view.entities.contains(key) || !view.was_evicted(key),
        };
        if !whole {
            return CacheWrite::Refused { patch };
        }

        // A creation is a complete replacement. Deep-merging it would retain
        // fields from the previous account lifetime.
        if creation {
            let new_entity = truncate_arrays_if_needed(patch, max_array_length);
            view.store(key.to_string(), new_entity);
            return CacheWrite::Created;
        }

        if let Some(entity) = view.entities.get_mut(key) {
            deep_merge_with_append(entity, patch, append_paths, max_array_length);
            return CacheWrite::Merged;
        }
        let new_entity = truncate_arrays_if_needed(patch, max_array_length);
        view.store(key.to_string(), new_entity);
        CacheWrite::Created
    }

    /// Store `entity` as the whole entity for `key`, replacing anything held
    /// and clearing an eviction: the source vouched for it being complete
    /// (see [`arete_interpreter::Mutation::mark_whole_entity`]).
    pub async fn store_whole(&self, view_id: &str, key: &str, entity: Value) -> bool {
        self.store_whole_for_lifetime(view_id, key, entity, None)
            .await
    }

    /// Store a requested whole entity only if the request was made for the
    /// account lifetime that is still current.
    pub async fn store_whole_for_lifetime(
        &self,
        view_id: &str,
        key: &str,
        entity: Value,
        requested_account_position: Option<AccountPosition>,
    ) -> bool {
        self.store_whole_checked(view_id, key, entity, requested_account_position, None)
            .await
    }

    pub async fn store_whole_for_ordering(
        &self,
        view_id: &str,
        key: &str,
        entity: Value,
        requested_account_position: Option<AccountPosition>,
        requested_source_seq: Option<&str>,
    ) -> bool {
        self.store_whole_checked(
            view_id,
            key,
            entity,
            requested_account_position,
            Some(requested_source_seq),
        )
        .await
    }

    async fn store_whole_checked(
        &self,
        view_id: &str,
        key: &str,
        entity: Value,
        requested_account_position: Option<AccountPosition>,
        requested_source_seq: Option<Option<&str>>,
    ) -> bool {
        let mut caches = self.caches.write().await;
        let view = caches
            .entry(view_id.to_string())
            .or_insert_with(|| ViewEntries::new(self.config.max_entities_per_view));
        let entity = truncate_arrays_if_needed(entity, self.config.max_array_length);
        if let Some(checkpoint) = view.lifetimes.peek(key) {
            if checkpoint.deleted || checkpoint.account_position != requested_account_position {
                return false;
            }
            if let Some(requested_source_seq) = requested_source_seq {
                if (checkpoint.account_position.is_none()
                    && checkpoint.source_seq.as_deref() != requested_source_seq)
                    || view.entities.contains(key)
                {
                    return false;
                }
            } else if checkpoint.account_position.is_some() && view.entities.contains(key) {
                return false;
            }
        }
        view.store(key.to_string(), entity);
        true
    }

    pub async fn account_position(&self, view_id: &str, key: &str) -> Option<AccountPosition> {
        self.caches
            .read()
            .await
            .get(view_id)
            .and_then(|view| view.lifetimes.peek(key))
            .and_then(|checkpoint| checkpoint.account_position)
    }

    pub async fn accepts_mutation(
        &self,
        view_id: &str,
        key: &str,
        _entity: &Value,
        creation: bool,
    ) -> bool {
        self.accepts_lifetime_mutation(view_id, key, creation, None)
            .await
    }

    pub async fn accepts_lifetime_mutation(
        &self,
        view_id: &str,
        key: &str,
        creation: bool,
        account_position: Option<AccountPosition>,
    ) -> bool {
        self.accepts_ordered_lifetime_mutation(view_id, key, creation, account_position, None)
            .await
    }

    pub async fn accepts_ordered_lifetime_mutation(
        &self,
        view_id: &str,
        key: &str,
        creation: bool,
        account_position: Option<AccountPosition>,
        source_seq: Option<&str>,
    ) -> bool {
        self.caches
            .read()
            .await
            .get(view_id)
            .is_none_or(|view| view.accepts(key, creation, account_position, source_seq, true))
    }

    /// Apply an ordered source deletion. A delayed deletion never removes a
    /// newer recreation. Only a newer, explicitly marked creation can restart
    /// a deleted row; sparse patches and resends cannot resurrect it.
    pub async fn delete(
        &self,
        view_id: &str,
        key: &str,
        account_position: Option<AccountPosition>,
    ) -> bool {
        match account_position {
            Some(position) => {
                self.delete_ordered(view_id, key, Some(position), None)
                    .await
            }
            None => self.delete_current(view_id, key).await,
        }
    }

    /// Delete the lifetime currently held by the cache when the producer has
    /// no cursor to attach. This is an explicit, trusted operation: callers
    /// with ordering evidence must use [`Self::delete_ordered`] instead.
    pub async fn delete_current(&self, view_id: &str, key: &str) -> bool {
        let mut caches = self.caches.write().await;
        let view = caches
            .entry(view_id.to_string())
            .or_insert_with(|| ViewEntries::new(self.config.max_entities_per_view));
        let checkpoint = view
            .lifetimes
            .peek(key)
            .cloned()
            .unwrap_or(EntityLifetimeCheckpoint {
                account_position: None,
                source_seq: None,
                deleted: false,
            });
        if checkpoint.deleted {
            return true;
        }
        view.entities.pop(key);
        view.forget_evicted(key);
        view.lifetimes.put(
            key.to_string(),
            EntityLifetimeCheckpoint {
                deleted: true,
                ..checkpoint
            },
        );
        view.trim_lifetimes();
        true
    }

    /// Delete using the producer's own `_seq` ordering only when no
    /// authoritative account position is present.
    pub async fn delete_ordered(
        &self,
        view_id: &str,
        key: &str,
        account_position: Option<AccountPosition>,
        source_seq: Option<&str>,
    ) -> bool {
        let mut caches = self.caches.write().await;
        let view = caches
            .entry(view_id.to_string())
            .or_insert_with(|| ViewEntries::new(self.config.max_entities_per_view));
        if let Some(checkpoint) = view.lifetimes.peek(key) {
            match (checkpoint.account_position, account_position) {
                (Some(previous), Some(incoming)) if incoming < previous => return false,
                (Some(previous), Some(incoming)) if incoming == previous => {
                    return checkpoint.deleted;
                }
                _ => {}
            }
        }
        if account_position.is_none()
            && source_seq.is_some()
            && !view.accepts_source_ordering(key, source_seq, true)
        {
            return false;
        }
        let previous = view.lifetimes.peek(key).cloned();
        view.entities.pop(key);
        view.forget_evicted(key);
        view.lifetimes.put(
            key.to_string(),
            EntityLifetimeCheckpoint {
                account_position: account_position.or_else(|| {
                    previous
                        .as_ref()
                        .and_then(|checkpoint| checkpoint.account_position)
                }),
                source_seq: if account_position.is_some() {
                    None
                } else {
                    source_seq
                        .map(str::to_owned)
                        .or_else(|| previous.and_then(|checkpoint| checkpoint.source_seq))
                },
                deleted: true,
            },
        );
        view.trim_lifetimes();
        true
    }

    pub async fn dump_lifetimes(&self) -> EntityLifetimes {
        self.caches
            .read()
            .await
            .iter()
            .flat_map(|(id, view)| {
                view.lifetimes
                    .iter()
                    .map(|(key, checkpoint)| (id.clone(), key.clone(), checkpoint.clone()))
            })
            .collect()
    }

    pub async fn hydrate_lifetimes(&self, lifetimes: EntityLifetimes) {
        let mut caches = self.caches.write().await;
        for (id, key, checkpoint) in lifetimes.into_iter().rev() {
            let view = caches
                .entry(id)
                .or_insert_with(|| ViewEntries::new(self.config.max_entities_per_view));
            view.lifetimes.put(key, checkpoint);
            view.trim_lifetimes();
        }
    }

    /// Restore the old `_seq` barriers as account positions. Those barriers
    /// were written only by authoritative account deletion/recreation paths;
    /// whether the row exists distinguishes a live recreation from deletion.
    pub async fn hydrate_legacy_tombstones(&self, tombstones: EntityTombstones) {
        let mut caches = self.caches.write().await;
        for (id, key, seq) in tombstones.into_iter().rev() {
            let view = caches
                .entry(id)
                .or_insert_with(|| ViewEntries::new(self.config.max_entities_per_view));
            let deleted = !view.entities.contains(&key);
            let account_position = seq.as_deref().and_then(account_position_from_seq);
            view.lifetimes.put(
                key,
                EntityLifetimeCheckpoint {
                    account_position,
                    source_seq: None,
                    deleted,
                },
            );
            view.trim_lifetimes();
        }
    }

    /// Whether the shared cache says a published delete still represents the
    /// latest lifetime. Subscription tasks use this without mutating the cache
    /// a second time after the projector has already applied the delete.
    pub async fn deletion_is_current(&self, view_id: &str, key: &str) -> bool {
        let caches = self.caches.read().await;
        caches.get(view_id).is_none_or(|view| {
            view.lifetimes
                .peek(key)
                .map_or(!view.entities.contains(key), |checkpoint| {
                    checkpoint.deleted
                })
        })
    }

    /// Merge a source patch into `base` with this cache's append and array
    /// rules, for callers that keep their own full copy of an entity.
    pub fn merge_patch(&self, base: &mut Value, patch: Value, append_paths: &[String]) {
        deep_merge_with_append(base, patch, append_paths, self.config.max_array_length);
    }

    /// Treat `keys` as evicted from `view_id` unless the view holds them, so
    /// patches for them from a source that does not mark creations are
    /// refused. Used after a restore, when the VM holds entities the restored
    /// cache does not; a VM that marks creations makes this moot, and the
    /// view drops the memory with its first patch.
    pub async fn remember_evicted(&self, view_id: &str, keys: impl IntoIterator<Item = String>) {
        let mut caches = self.caches.write().await;
        let view = caches
            .entry(view_id.to_string())
            .or_insert_with(|| ViewEntries::new(self.config.max_entities_per_view));
        for key in keys {
            if !view.entities.contains(&key) {
                view.remember_evicted(key);
            }
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
            .map(|view| {
                view.entities
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect()
            })
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

        if let Some(view) = caches.get(view_id) {
            let mut results: Vec<(String, Value)> = view
                .entities
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
            .and_then(|view| view.entities.peek(key).cloned())
    }

    /// Remove one entity after a source-wide delete.
    ///
    /// The delete ends the entity, so the key is also forgotten as evicted: a
    /// later patch for it creates a new entity.
    pub async fn remove(&self, view_id: &str, key: &str) -> Option<Value> {
        let mut caches = self.caches.write().await;
        caches.get_mut(view_id).and_then(|view| {
            view.forget_evicted(key);
            view.entities.pop(key)
        })
    }

    /// Get the number of cached entities for a view
    pub async fn len(&self, view_id: &str) -> usize {
        let caches = self.caches.read().await;
        caches
            .get(view_id)
            .map(|view| view.entities.len())
            .unwrap_or(0)
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
        if let Some(view) = caches.get_mut(view_id) {
            view.entities.clear();
            view.evicted = None;
        }
    }

    pub async fn clear_all(&self) {
        let mut caches = self.caches.write().await;
        caches.clear();
    }

    /// Dump every view's entities for a state snapshot.
    ///
    /// Entries are ordered most-recently-used first; [`Self::hydrate`] relies
    /// on that to reconstruct LRU eviction order.
    pub async fn dump(&self) -> Vec<(String, Vec<(String, Value)>)> {
        let caches = self.caches.read().await;
        caches
            .iter()
            .map(|(view_id, view)| {
                (
                    view_id.clone(),
                    view.entities
                        .iter()
                        .map(|(key, entity)| (key.clone(), entity.clone()))
                        .collect(),
                )
            })
            .collect()
    }

    /// Restore entities dumped by [`Self::dump`], preserving LRU order.
    ///
    /// Entities are inserted as-is (no merge): a snapshot holds fully
    /// projected entities, not patches.
    pub async fn hydrate(&self, views: Vec<(String, Vec<(String, Value)>)>) {
        let mut caches = self.caches.write().await;
        caches.clear();
        for (view_id, entries) in views {
            let view = caches
                .entry(view_id)
                .or_insert_with(|| ViewEntries::new(self.config.max_entities_per_view));
            for (key, entity) in entries.into_iter().rev() {
                view.store(key, entity);
            }
        }
    }

    pub async fn stats(&self) -> CacheStats {
        let caches = self.caches.read().await;
        let mut total_entities = 0;
        let mut views = Vec::new();

        for (view_id, view) in caches.iter() {
            let count = view.entities.len();
            total_entities += count;
            views.push((view_id.clone(), count));
        }

        views.sort_by_key(|b| std::cmp::Reverse(b.1));

        CacheStats {
            view_count: caches.len(),
            total_entities,
            top_views: views.into_iter().take(5).collect(),
        }
    }
}

/// A point-in-time count of an [`EntityCache`]'s contents.
#[derive(Debug, Clone, PartialEq, Eq)]
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
    async fn same_slot_instruction_activity_cannot_block_authoritative_deletion() {
        let cache = EntityCache::new();
        assert_eq!(
            cache
                .upsert_with_lifetime(
                    "v",
                    "a",
                    json!({"account":true,"_seq":"100:000000000009"}),
                    &[],
                    PatchOrigin::Creation,
                    Some(AccountPosition::new(100, 9)),
                )
                .await,
            CacheWrite::Created
        );
        assert_eq!(
            cache
                .upsert_with_append(
                    "v",
                    "a",
                    json!({"instruction":true,"_seq":"100:000000000900"}),
                    &[],
                    PatchOrigin::Change
                )
                .await,
            CacheWrite::Merged
        );
        assert!(
            cache
                .delete("v", "a", Some(AccountPosition::new(100, 10)))
                .await
        );
        assert!(cache.get("v", "a").await.is_none());
        assert!(cache.deletion_is_current("v", "a").await);
    }

    #[tokio::test]
    async fn recreation_has_its_own_ordering_and_replaces_old_fields() {
        let cache = EntityCache::with_config(EntityCacheConfig {
            max_entities_per_view: 1,
            ..Default::default()
        });
        assert!(
            cache
                .delete("v", "a", Some(AccountPosition::new(100, 1001)))
                .await
        );
        assert_eq!(
            cache
                .upsert_with_lifetime(
                    "v",
                    "a",
                    json!({"fresh":true, "_seq":"100:000000001002"}),
                    &[],
                    PatchOrigin::Creation,
                    Some(AccountPosition::new(100, 1002)),
                )
                .await,
            CacheWrite::Created
        );
        assert_eq!(
            cache
                .upsert_with_append(
                    "v",
                    "a",
                    json!({"instruction":true, "_seq":"100:000000000900"}),
                    &[],
                    PatchOrigin::Change
                )
                .await,
            CacheWrite::Merged
        );

        // Neither a stale authoritative write/tombstone nor a delayed resend
        // from the deleted lifetime can contaminate the replacement.
        assert!(matches!(
            cache
                .upsert_with_lifetime(
                    "v",
                    "a",
                    json!({"old":true, "_seq":"100:000000001001"}),
                    &[],
                    PatchOrigin::Change,
                    Some(AccountPosition::new(100, 1001)),
                )
                .await,
            CacheWrite::Refused { .. }
        ));
        assert!(
            !cache
                .delete("v", "a", Some(AccountPosition::new(100, 1001)))
                .await
        );
        assert!(
            !cache
                .store_whole("v", "a", json!({"old":true, "_seq":"100:000000000900"}))
                .await
        );
        let row = cache.get("v", "a").await.unwrap();
        assert_eq!(row["fresh"], true);
        assert_eq!(row["instruction"], true);
        assert!(row.get("old").is_none());

        cache
            .upsert_with_append(
                "v",
                "other",
                json!({"other":true}),
                &[],
                PatchOrigin::Creation,
            )
            .await;
        assert!(cache.get("v", "a").await.is_none());
        assert!(matches!(
            cache
                .upsert_with_lifetime(
                    "v",
                    "a",
                    json!({"accountUpdate":true, "_seq":"100:000000001003"}),
                    &[],
                    PatchOrigin::Change,
                    Some(AccountPosition::new(100, 1003)),
                )
                .await,
            CacheWrite::Refused { .. }
        ));
        assert!(
            !cache
                .store_whole_for_lifetime(
                    "v",
                    "a",
                    row.clone(),
                    Some(AccountPosition::new(100, 1002)),
                )
                .await
        );
        let mut current = row;
        current["accountUpdate"] = Value::Bool(true);
        current["_seq"] = Value::String("100:000000001003".into());
        assert!(
            cache
                .store_whole_for_lifetime("v", "a", current, Some(AccountPosition::new(100, 1003)),)
                .await
        );

        // Current lifetime metadata survives a snapshot and continues to
        // admit instruction-domain offsets while rejecting stale account data.
        let restored = EntityCache::new();
        restored.hydrate(cache.dump().await).await;
        restored
            .hydrate_lifetimes(cache.dump_lifetimes().await)
            .await;
        assert_eq!(
            restored
                .upsert_with_append(
                    "v",
                    "a",
                    json!({"afterRestore":true, "_seq":"100:000000000901"}),
                    &[],
                    PatchOrigin::Change,
                )
                .await,
            CacheWrite::Merged
        );
        assert!(matches!(
            restored
                .upsert_with_lifetime(
                    "v",
                    "a",
                    json!({"old":true, "_seq":"100:000000001001"}),
                    &[],
                    PatchOrigin::Change,
                    Some(AccountPosition::new(100, 1001)),
                )
                .await,
            CacheWrite::Refused { .. }
        ));
    }

    #[tokio::test]
    async fn markerless_sources_keep_seq_ordering_across_recreation_and_restore() {
        let cache = EntityCache::new();
        assert_eq!(
            cache
                .upsert_with_append(
                    "v",
                    "a",
                    json!({"old":true,"_seq":"100:000000000010"}),
                    &[],
                    PatchOrigin::Creation,
                )
                .await,
            CacheWrite::Created
        );
        assert!(
            !cache
                .delete_ordered("v", "a", None, Some("100:000000000009"))
                .await
        );
        assert!(cache.get("v", "a").await.is_some());
        assert!(
            cache
                .delete_ordered("v", "a", None, Some("100:000000000011"))
                .await
        );
        assert_eq!(
            cache
                .upsert_with_append(
                    "v",
                    "a",
                    json!({"fresh":true,"_seq":"100:000000000012"}),
                    &[],
                    PatchOrigin::Creation,
                )
                .await,
            CacheWrite::Created
        );
        assert!(matches!(
            cache
                .upsert_with_append(
                    "v",
                    "a",
                    json!({"stale":true,"_seq":"100:000000000010"}),
                    &[],
                    PatchOrigin::Creation,
                )
                .await,
            CacheWrite::Refused { .. }
        ));
        assert!(
            !cache
                .delete_ordered("v", "a", None, Some("100:000000000011"))
                .await
        );

        let restored = EntityCache::new();
        restored.hydrate(cache.dump().await).await;
        restored
            .hydrate_lifetimes(cache.dump_lifetimes().await)
            .await;
        assert!(
            !restored
                .delete_ordered("v", "a", None, Some("100:000000000011"))
                .await
        );
        let row = restored.get("v", "a").await.unwrap();
        assert_eq!(row["fresh"], true);
        assert!(row.get("old").is_none());
        assert!(row.get("stale").is_none());

        // An evicted row may be filled by a requested whole-entity resend,
        // but a response requested for the previous lifetime must not do it.
        let resync_cache = EntityCache::new();
        resync_cache
            .hydrate_lifetimes(cache.dump_lifetimes().await)
            .await;
        assert!(
            !resync_cache
                .store_whole_for_ordering(
                    "v",
                    "a",
                    json!({"stale_resend":true}),
                    None,
                    Some("100:000000000010"),
                )
                .await
        );
        assert!(
            resync_cache
                .store_whole_for_ordering(
                    "v",
                    "a",
                    json!({"fresh_resend":true}),
                    None,
                    Some("100:000000000012"),
                )
                .await
        );
    }

    #[tokio::test]
    async fn older_source_patch_cannot_replace_newer_row_or_move_its_cursor_back() {
        let cache = EntityCache::new();
        assert_eq!(
            cache
                .upsert_with_append(
                    "v",
                    "a",
                    json!({"value":"created","_seq":"100:000000000010"}),
                    &[],
                    PatchOrigin::Creation,
                )
                .await,
            CacheWrite::Created
        );
        assert_eq!(
            cache
                .upsert_with_ordering(
                    "v",
                    "a",
                    json!({"value":"newer","_seq":"100:000000000012"}),
                    &[],
                    PatchOrigin::Change,
                    LifetimeOrdering {
                        account_position: None,
                        source_seq: Some("100:000000000012"),
                        source_seq_is_ordered: true,
                    },
                )
                .await,
            CacheWrite::Merged
        );
        assert!(matches!(
            cache
                .upsert_with_ordering(
                    "v",
                    "a",
                    json!({"value":"older","stale":true,"_seq":"100:000000000011"}),
                    &[],
                    PatchOrigin::Change,
                    LifetimeOrdering {
                        account_position: None,
                        source_seq: Some("100:000000000011"),
                        source_seq_is_ordered: true,
                    },
                )
                .await,
            CacheWrite::Refused { .. }
        ));
        assert!(
            !cache
                .delete_ordered("v", "a", None, Some("100:000000000011"))
                .await
        );
        let row = cache.get("v", "a").await.unwrap();
        assert_eq!(row["value"], "newer");
        assert_eq!(row["_seq"], "100:000000000012");
        assert!(row.get("stale").is_none());
    }

    #[tokio::test]
    async fn explicit_current_delete_accepts_a_source_without_position() {
        let cache = EntityCache::new();
        cache
            .upsert_with_append(
                "v",
                "a",
                json!({"value":true,"_seq":"100:000000000010"}),
                &[],
                PatchOrigin::Creation,
            )
            .await;

        assert!(cache.delete("v", "a", None).await);
        assert!(cache.get("v", "a").await.is_none());
        assert!(cache.deletion_is_current("v", "a").await);
    }

    #[tokio::test]
    async fn evicted_live_row_does_not_make_an_earlier_delete_current() {
        let cache = EntityCache::with_config(EntityCacheConfig {
            max_entities_per_view: 1,
            ..Default::default()
        });
        cache
            .upsert_with_lifetime(
                "v",
                "a",
                json!({"value":true,"_seq":"100:000000000010"}),
                &[],
                PatchOrigin::Creation,
                Some(AccountPosition::new(100, 10)),
            )
            .await;
        cache
            .upsert_with_append("v", "b", json!({"value":true}), &[], PatchOrigin::Creation)
            .await;

        assert!(cache.get("v", "a").await.is_none());
        assert!(!cache.deletion_is_current("v", "a").await);
        assert!(cache.delete_current("v", "a").await);
        assert!(cache.deletion_is_current("v", "a").await);
    }

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
                PatchOrigin::Unknown,
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
                PatchOrigin::Unknown,
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
    async fn a_patch_for_an_evicted_key_is_refused_not_stored() {
        let cache = EntityCache::with_config(EntityCacheConfig {
            max_entities_per_view: 2,
            ..Default::default()
        });
        for id in 1..=3 {
            assert_eq!(
                cache
                    .upsert(
                        "tokens/list",
                        &format!("key{id}"),
                        json!({"id": id, "n": 0})
                    )
                    .await,
                CacheWrite::Created
            );
        }
        // key1 was evicted; this carries only what changed.
        assert_eq!(
            cache.upsert("tokens/list", "key1", json!({"n": 5})).await,
            CacheWrite::Refused {
                patch: json!({"n": 5})
            }
        );
        assert!(cache.get("tokens/list", "key1").await.is_none());
        assert_eq!(
            cache.upsert("tokens/list", "key3", json!({"n": 1})).await,
            CacheWrite::Merged
        );

        // A source delete ends the entity: it may then be created again.
        cache.remove("tokens/list", "key1").await;
        assert_eq!(
            cache
                .upsert("tokens/list", "key1", json!({"id": 1, "n": 6}))
                .await,
            CacheWrite::Created
        );
    }

    #[tokio::test]
    async fn a_whole_entity_ends_an_eviction() {
        let cache = EntityCache::with_config(EntityCacheConfig {
            max_entities_per_view: 1,
            ..Default::default()
        });
        cache.upsert("v", "a", json!({"id": "a", "n": 0})).await;
        cache.upsert("v", "b", json!({"id": "b", "n": 0})).await;
        assert!(matches!(
            cache.upsert("v", "a", json!({"n": 1})).await,
            CacheWrite::Refused { .. }
        ));
        cache
            .store_whole("v", "a", json!({"id": "a", "n": 1}))
            .await;
        assert_eq!(cache.get("v", "a").await, Some(json!({"id": "a", "n": 1})));
        assert_eq!(cache.len("v").await, 1, "still bounded: b made room");
        assert_eq!(
            cache.upsert("v", "a", json!({"n": 2})).await,
            CacheWrite::Merged
        );
    }

    #[tokio::test]
    async fn the_evicted_key_memory_is_bounded() {
        let cache = EntityCache::with_config(EntityCacheConfig {
            max_entities_per_view: 1,
            ..Default::default()
        });
        let remembered = EVICTED_KEYS_PER_CACHED_ENTITY;
        for key in 0..=remembered + 1 {
            cache
                .upsert("v", &key.to_string(), json!({"id": key}))
                .await;
        }
        // Key 0 was pushed out of the memory by the evictions after it; the
        // most recent evictions are still remembered.
        assert_eq!(
            cache.upsert("v", "0", json!({"id": 0})).await,
            CacheWrite::Created
        );
        assert!(matches!(
            cache
                .upsert("v", &remembered.to_string(), json!({"n": 1}))
                .await,
            CacheWrite::Refused { .. }
        ));
    }

    async fn write(
        cache: &EntityCache,
        key: &str,
        patch: Value,
        origin: PatchOrigin,
    ) -> CacheWrite {
        cache.upsert_with_append("v", key, patch, &[], origin).await
    }

    /// A source that marks creations says outright which patches are whole:
    /// no memory of evictions is needed, and none can run out.
    #[tokio::test]
    async fn an_unmarked_patch_for_a_key_not_held_is_refused_however_old() {
        let cache = EntityCache::with_config(EntityCacheConfig {
            max_entities_per_view: 1,
            ..Default::default()
        });
        let evictions = EVICTED_KEYS_PER_CACHED_ENTITY * 4;
        for key in 0..=evictions {
            assert_eq!(
                write(
                    &cache,
                    &key.to_string(),
                    json!({"id": key}),
                    PatchOrigin::Creation
                )
                .await,
                CacheWrite::Created,
                "a creation is stored as soon as it arrives"
            );
        }
        // Key 0 was evicted far longer ago than any memory would reach.
        assert_eq!(
            write(&cache, "0", json!({"n": 1}), PatchOrigin::Change).await,
            CacheWrite::Refused {
                patch: json!({"n": 1})
            }
        );
        assert!(
            matches!(
                write(&cache, "never", json!({"n": 1}), PatchOrigin::Change).await,
                CacheWrite::Refused { .. }
            ),
            "nor does it matter whether the view ever held the key"
        );
        assert_eq!(cache.get("v", "0").await, None);
        assert_eq!(
            write(
                &cache,
                &evictions.to_string(),
                json!({"n": 1}),
                PatchOrigin::Change
            )
            .await,
            CacheWrite::Merged
        );
        // The source started key 0 again.
        assert_eq!(
            write(&cache, "0", json!({"id": 0, "n": 2}), PatchOrigin::Creation).await,
            CacheWrite::Created
        );
        assert_eq!(cache.get("v", "0").await, Some(json!({"id": 0, "n": 2})));
    }

    /// A creation for a key the view still holds starts a new lifetime and
    /// replaces fields that no longer exist.
    #[tokio::test]
    async fn a_creation_for_a_key_held_replaces() {
        let cache = EntityCache::new();
        write(
            &cache,
            "a",
            json!({"id": "a", "old": 1}),
            PatchOrigin::Creation,
        )
        .await;
        assert_eq!(
            write(
                &cache,
                "a",
                json!({"id": "a", "n": 1}),
                PatchOrigin::Creation
            )
            .await,
            CacheWrite::Created
        );
        assert_eq!(cache.get("v", "a").await, Some(json!({"id": "a", "n": 1})));
    }

    async fn remembers_evictions(cache: &EntityCache, view_id: &str) -> bool {
        cache
            .caches
            .read()
            .await
            .get(view_id)
            .is_some_and(|view| view.evicted.is_some())
    }

    /// A view fed by a source that marks creations never consults the
    /// evicted-key memory, so it drops what a restore put there and keeps no
    /// more.
    #[tokio::test]
    async fn a_view_fed_marked_creations_keeps_no_eviction_memory() {
        let cache = EntityCache::with_config(EntityCacheConfig {
            max_entities_per_view: 1,
            ..Default::default()
        });
        cache.remember_evicted("v", ["restored".to_string()]).await;
        assert!(remembers_evictions(&cache, "v").await);

        write(&cache, "a", json!({"id": "a"}), PatchOrigin::Creation).await;
        assert!(!remembers_evictions(&cache, "v").await);
        write(&cache, "b", json!({"id": "b"}), PatchOrigin::Creation).await;
        assert!(!remembers_evictions(&cache, "v").await, "a was evicted");
        assert!(matches!(
            write(&cache, "restored", json!({"n": 1}), PatchOrigin::Change).await,
            CacheWrite::Refused { .. }
        ));
        assert!(matches!(
            write(&cache, "a", json!({"n": 1}), PatchOrigin::Change).await,
            CacheWrite::Refused { .. }
        ));
    }

    #[tokio::test]
    async fn remembered_keys_are_refused_unless_held() {
        let cache = EntityCache::new();
        cache.upsert("tokens/list", "held", json!({"id": 1})).await;
        cache
            .remember_evicted("tokens/list", ["held".to_string(), "gone".to_string()])
            .await;
        assert_eq!(
            cache.upsert("tokens/list", "held", json!({"n": 1})).await,
            CacheWrite::Merged
        );
        assert!(matches!(
            cache.upsert("tokens/list", "gone", json!({"n": 1})).await,
            CacheWrite::Refused { .. }
        ));
        // Other views are unaffected.
        assert_eq!(
            cache.upsert("tokens/state", "gone", json!({"id": 2})).await,
            CacheWrite::Created
        );
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
    async fn remove_is_scoped_to_one_entity() {
        let cache = EntityCache::new();
        cache.upsert("tokens/list", "one", json!({"id": 1})).await;
        cache.upsert("tokens/list", "two", json!({"id": 2})).await;

        assert_eq!(cache.remove("tokens/list", "one").await.unwrap()["id"], 1);
        assert!(cache.get("tokens/list", "one").await.is_none());
        assert!(cache.get("tokens/list", "two").await.is_some());
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
