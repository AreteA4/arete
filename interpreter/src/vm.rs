use crate::ast::{
    self, BinaryOp, ComparisonOp, ComputedExpr, ComputedFieldSpec, FieldPath, ResolveStrategy,
    ResolverExtractSpec, ResolverType, Transformation,
};
use crate::compiler::{MultiEntityBytecode, OpCode};
use crate::Mutation;
use dashmap::DashMap;
use lru::LruCache;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet, VecDeque};
use std::num::NonZeroUsize;

#[cfg(feature = "otel")]
use tracing::instrument;
/// Context metadata for blockchain updates (accounts and instructions)
/// This structure is designed to be extended over time with additional metadata
#[derive(Debug, Clone, Default)]
pub struct UpdateContext {
    /// Blockchain slot number
    pub slot: Option<u64>,
    /// Transaction signature
    pub signature: Option<String>,
    /// Unix timestamp (seconds since epoch)
    /// If not provided, will default to current system time when accessed
    pub timestamp: Option<i64>,
    /// Write version for account updates (monotonically increasing per account within a slot)
    /// Used for staleness detection to reject out-of-order updates
    pub write_version: Option<u64>,
    /// Transaction index for instruction updates (orders transactions within a slot)
    /// Used for staleness detection to reject out-of-order updates
    pub txn_index: Option<u64>,
    /// When true, QueueResolver opcodes are skipped during handler execution.
    /// Set for reprocessed cached data from PDA mapping changes to prevent
    /// stale data from triggering resolvers or locking in wrong values via SetOnce.
    pub skip_resolvers: bool,
    /// Additional custom metadata that can be added without breaking changes
    pub metadata: HashMap<String, Value>,
}

impl UpdateContext {
    /// Create a new UpdateContext with slot and signature
    pub fn new(slot: u64, signature: String) -> Self {
        Self {
            slot: Some(slot),
            signature: Some(signature),
            timestamp: None,
            write_version: None,
            txn_index: None,
            skip_resolvers: false,
            metadata: HashMap::new(),
        }
    }

    /// Create a new UpdateContext with slot, signature, and timestamp
    pub fn with_timestamp(slot: u64, signature: String, timestamp: i64) -> Self {
        Self {
            slot: Some(slot),
            signature: Some(signature),
            timestamp: Some(timestamp),
            write_version: None,
            txn_index: None,
            skip_resolvers: false,
            metadata: HashMap::new(),
        }
    }

    /// Create context for account updates with write_version for staleness detection
    pub fn new_account(slot: u64, signature: String, write_version: u64) -> Self {
        Self {
            slot: Some(slot),
            signature: Some(signature),
            timestamp: None,
            write_version: Some(write_version),
            txn_index: None,
            skip_resolvers: false,
            metadata: HashMap::new(),
        }
    }

    /// Create context for instruction updates with txn_index for staleness detection
    pub fn new_instruction(slot: u64, signature: String, txn_index: u64) -> Self {
        Self {
            slot: Some(slot),
            signature: Some(signature),
            timestamp: None,
            write_version: None,
            txn_index: Some(txn_index),
            skip_resolvers: false,
            metadata: HashMap::new(),
        }
    }

    /// Create context for reprocessed cached account data from PDA mapping changes.
    /// Uses empty signature to prevent `when` guards from matching stale instructions,
    /// and sets skip_resolvers to prevent stale scheduling/SetOnce lock-in.
    pub fn new_reprocessed(slot: u64, write_version: u64) -> Self {
        Self {
            slot: Some(slot),
            signature: None,
            timestamp: None,
            write_version: Some(write_version),
            txn_index: None,
            skip_resolvers: true,
            metadata: HashMap::new(),
        }
    }

    /// Get the timestamp, falling back to current system time if not set
    pub fn timestamp(&self) -> i64 {
        self.timestamp.unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64
        })
    }

    /// Create an empty context (for testing or when context is not available)
    pub fn empty() -> Self {
        Self::default()
    }

    /// Add custom metadata
    /// Returns true if this is an account update context (has write_version, no txn_index)
    pub fn is_account_update(&self) -> bool {
        self.write_version.is_some() && self.txn_index.is_none()
    }

    /// Returns true if this is an instruction update context (has txn_index, no write_version)
    pub fn is_instruction_update(&self) -> bool {
        self.txn_index.is_some() && self.write_version.is_none()
    }

    pub fn with_metadata(mut self, key: String, value: Value) -> Self {
        self.metadata.insert(key, value);
        self
    }

    /// Get metadata value
    pub fn get_metadata(&self, key: &str) -> Option<&Value> {
        self.metadata.get(key)
    }

    /// Convert context to JSON value for injection into event data
    pub fn to_value(&self) -> Value {
        let mut obj = serde_json::Map::new();
        if let Some(slot) = self.slot {
            obj.insert("slot".to_string(), json!(slot));
        }
        if let Some(ref sig) = self.signature {
            obj.insert("signature".to_string(), json!(sig));
        }
        // Always include timestamp (use current time if not set)
        obj.insert("timestamp".to_string(), json!(self.timestamp()));
        for (key, value) in &self.metadata {
            obj.insert(key.clone(), value.clone());
        }
        Value::Object(obj)
    }
}

pub type Register = usize;
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

pub type RegisterValue = Value;

/// Trait for evaluating computed fields
/// Implement this in your generated spec to enable computed field evaluation
pub trait ComputedFieldsEvaluator {
    fn evaluate(&self, state: &mut Value) -> Result<()>;
}

// Pending queue configuration
const MAX_PENDING_UPDATES_TOTAL: usize = 2_500;
const MAX_PENDING_UPDATES_PER_PDA: usize = 50;
const PENDING_UPDATE_TTL_SECONDS: i64 = 300; // 5 minutes

// Temporal index configuration - prevents unbounded history growth
const TEMPORAL_HISTORY_TTL_SECONDS: i64 = 300; // 5 minutes, matches pending queue TTL
const MAX_TEMPORAL_ENTRIES_PER_KEY: usize = 250;

// State table configuration - aligned with downstream EntityCache (500 per view)
const DEFAULT_MAX_STATE_TABLE_ENTRIES: usize = 2_500;
const DEFAULT_MAX_ARRAY_LENGTH: usize = 100;

const DEFAULT_MAX_LOOKUP_INDEX_ENTRIES: usize = 2_500;

const DEFAULT_MAX_VERSION_TRACKER_ENTRIES: usize = 2_500;

// Smaller cache for instruction deduplication - provides shorter effective TTL
// since we don't expect instruction duplicates to arrive much later
const DEFAULT_MAX_INSTRUCTION_DEDUP_ENTRIES: usize = 500;

const DEFAULT_MAX_TEMPORAL_INDEX_KEYS: usize = 2_500;

const DEFAULT_MAX_PDA_REVERSE_LOOKUP_ENTRIES: usize = 2_500;

const DEFAULT_MAX_RESOLVER_CACHE_ENTRIES: usize = 5_000;

/// Estimate the size of a JSON value in bytes
fn estimate_json_size(value: &Value) -> usize {
    match value {
        Value::Null => 4,
        Value::Bool(_) => 5,
        Value::Number(_) => 8,
        Value::String(s) => s.len() + 2,
        Value::Array(arr) => 2 + arr.iter().map(|v| estimate_json_size(v) + 1).sum::<usize>(),
        Value::Object(obj) => {
            2 + obj
                .iter()
                .map(|(k, v)| k.len() + 3 + estimate_json_size(v) + 1)
                .sum::<usize>()
        }
    }
}

#[derive(Debug, Clone)]
pub struct CompiledPath {
    pub segments: std::sync::Arc<[String]>,
}

impl CompiledPath {
    pub fn new(path: &str) -> Self {
        let segments: Vec<String> = path.split('.').map(|s| s.to_string()).collect();
        CompiledPath {
            segments: segments.into(),
        }
    }

    fn segments(&self) -> &[String] {
        &self.segments
    }
}

/// Represents the type of change made to a field for granular dirty tracking.
/// This enables emitting only the actual changes rather than entire field values.
#[derive(Debug, Clone)]
pub enum FieldChange {
    /// Field was replaced with a new value (emit the full value from state)
    Replaced,
    /// Items were appended to an array field (emit only the new items)
    Appended(Vec<Value>),
}

/// Tracks field modifications during handler execution with granular change information.
/// This replaces the simple HashSet<String> approach to enable delta-only emissions.
#[derive(Debug, Clone, Default)]
pub struct DirtyTracker {
    changes: HashMap<String, FieldChange>,
}

impl DirtyTracker {
    /// Create a new empty DirtyTracker
    pub fn new() -> Self {
        Self {
            changes: HashMap::new(),
        }
    }

    /// Mark a field as replaced (full value will be emitted)
    pub fn mark_replaced(&mut self, path: &str) {
        // If there was an append, it's now superseded by a full replacement
        self.changes.insert(path.to_string(), FieldChange::Replaced);
    }

    /// Record an appended value for a field
    pub fn mark_appended(&mut self, path: &str, value: Value) {
        match self.changes.get_mut(path) {
            Some(FieldChange::Appended(values)) => {
                // Add to existing appended values
                values.push(value);
            }
            Some(FieldChange::Replaced) => {
                // Field was already replaced, keep it as replaced
                // (the full value including the append will be emitted)
            }
            None => {
                // First append to this field
                self.changes
                    .insert(path.to_string(), FieldChange::Appended(vec![value]));
            }
        }
    }

    /// Check if there are any changes tracked
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    /// Get the number of changed fields
    pub fn len(&self) -> usize {
        self.changes.len()
    }

    /// Iterate over all changes
    pub fn iter(&self) -> impl Iterator<Item = (&String, &FieldChange)> {
        self.changes.iter()
    }

    /// Get a set of all dirty field paths (for backward compatibility)
    pub fn dirty_paths(&self) -> HashSet<String> {
        self.changes.keys().cloned().collect()
    }

    /// Consume the tracker and return the changes map
    pub fn into_changes(self) -> HashMap<String, FieldChange> {
        self.changes
    }

    /// Get a reference to the changes map
    pub fn changes(&self) -> &HashMap<String, FieldChange> {
        &self.changes
    }

    /// Get paths that were appended (not replaced)
    pub fn appended_paths(&self) -> Vec<String> {
        self.changes
            .iter()
            .filter_map(|(path, change)| match change {
                FieldChange::Appended(_) => Some(path.clone()),
                FieldChange::Replaced => None,
            })
            .collect()
    }
}

pub struct VmContext {
    registers: Vec<RegisterValue>,
    states: HashMap<u32, StateTable>,
    pub instructions_executed: u64,
    pub cache_hits: u64,
    path_cache: HashMap<String, CompiledPath>,
    pub pda_cache_hits: u64,
    pub pda_cache_misses: u64,
    pub pending_queue_size: u64,
    resolver_requests: VecDeque<ResolverRequest>,
    resolver_pending: HashMap<String, PendingResolverEntry>,
    resolver_cache: LruCache<String, Value>,
    pub resolver_cache_hits: u64,
    pub resolver_cache_misses: u64,
    current_context: Option<UpdateContext>,
    warnings: Vec<String>,
    last_pda_lookup_miss: Option<String>,
    last_lookup_index_miss: Option<String>,
    last_pda_registered: Option<String>,
    last_lookup_index_keys: Vec<String>,
    scheduled_callbacks: Vec<(u64, ScheduledCallback)>,
}

#[derive(Debug)]
pub struct LookupIndex {
    index: std::sync::Mutex<LruCache<String, Value>>,
}

impl LookupIndex {
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_MAX_LOOKUP_INDEX_ENTRIES)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        LookupIndex {
            index: std::sync::Mutex::new(LruCache::new(
                NonZeroUsize::new(capacity).expect("capacity must be > 0"),
            )),
        }
    }

    pub fn lookup(&self, lookup_value: &Value) -> Option<Value> {
        let key = value_to_cache_key(lookup_value);
        self.index.lock().unwrap().get(&key).cloned()
    }

    pub fn insert(&self, lookup_value: Value, primary_key: Value) {
        let key = value_to_cache_key(&lookup_value);
        self.index.lock().unwrap().put(key, primary_key);
    }

    /// Remove an entry by lookup value.  Used when a PDA mapping changes to
    /// clear stale entries that would otherwise shadow the updated PDA mapping.
    pub fn remove(&self, lookup_value: &Value) {
        let key = value_to_cache_key(lookup_value);
        self.index.lock().unwrap().pop(&key);
    }

    pub fn len(&self) -> usize {
        self.index.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.index.lock().unwrap().is_empty()
    }
}

impl Default for LookupIndex {
    fn default() -> Self {
        Self::new()
    }
}

fn value_to_cache_key(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => "null".to_string(),
        _ => serde_json::to_string(value).unwrap_or_else(|_| "unknown".to_string()),
    }
}

fn resolver_type_key(resolver: &ResolverType) -> String {
    match resolver {
        ResolverType::Token => "token".to_string(),
        ResolverType::Url(config) => match &config.url_source {
            ast::UrlSource::FieldPath(path) => format!("url:{}", path),
            ast::UrlSource::Template(parts) => {
                let key: String = parts
                    .iter()
                    .map(|p| match p {
                        ast::UrlTemplatePart::Literal(s) => s.clone(),
                        ast::UrlTemplatePart::FieldRef(f) => format!("{{{}}}", f),
                    })
                    .collect();
                format!("url:{}", key)
            }
        },
    }
}

fn resolver_cache_key(resolver: &ResolverType, input: &Value) -> String {
    format!(
        "{}:{}",
        resolver_type_key(resolver),
        value_to_cache_key(input)
    )
}

#[derive(Debug)]
pub struct TemporalIndex {
    index: std::sync::Mutex<LruCache<String, Vec<(Value, i64)>>>,
}

impl Default for TemporalIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl TemporalIndex {
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_MAX_TEMPORAL_INDEX_KEYS)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        TemporalIndex {
            index: std::sync::Mutex::new(LruCache::new(
                NonZeroUsize::new(capacity).expect("capacity must be > 0"),
            )),
        }
    }

    pub fn lookup(&self, lookup_value: &Value, timestamp: i64) -> Option<Value> {
        let key = value_to_cache_key(lookup_value);
        let mut cache = self.index.lock().unwrap();
        if let Some(entries) = cache.get(&key) {
            for i in (0..entries.len()).rev() {
                if entries[i].1 <= timestamp {
                    return Some(entries[i].0.clone());
                }
            }
        }
        None
    }

    pub fn lookup_latest(&self, lookup_value: &Value) -> Option<Value> {
        let key = value_to_cache_key(lookup_value);
        let mut cache = self.index.lock().unwrap();
        if let Some(entries) = cache.get(&key) {
            if let Some(last) = entries.last() {
                return Some(last.0.clone());
            }
        }
        None
    }

    pub fn insert(&self, lookup_value: Value, primary_key: Value, timestamp: i64) {
        let key = value_to_cache_key(&lookup_value);
        let mut cache = self.index.lock().unwrap();

        let entries = cache.get_or_insert_mut(key, Vec::new);
        entries.push((primary_key, timestamp));
        entries.sort_by_key(|(_, ts)| *ts);

        let cutoff = timestamp - TEMPORAL_HISTORY_TTL_SECONDS;
        entries.retain(|(_, ts)| *ts >= cutoff);

        if entries.len() > MAX_TEMPORAL_ENTRIES_PER_KEY {
            let excess = entries.len() - MAX_TEMPORAL_ENTRIES_PER_KEY;
            entries.drain(0..excess);
        }
    }

    pub fn len(&self) -> usize {
        self.index.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.index.lock().unwrap().is_empty()
    }

    pub fn total_entries(&self) -> usize {
        self.index
            .lock()
            .unwrap()
            .iter()
            .map(|(_, entries)| entries.len())
            .sum()
    }

    pub fn cleanup_expired(&self, cutoff_timestamp: i64) -> usize {
        let mut cache = self.index.lock().unwrap();
        let mut total_removed = 0;

        for (_, entries) in cache.iter_mut() {
            let original_len = entries.len();
            entries.retain(|(_, ts)| *ts >= cutoff_timestamp);
            total_removed += original_len - entries.len();
        }

        total_removed
    }
}

#[derive(Debug)]
pub struct PdaReverseLookup {
    // Maps: PDA address -> seed value (e.g., bonding_curve_addr -> mint)
    index: LruCache<String, String>,
}

impl PdaReverseLookup {
    pub fn new(capacity: usize) -> Self {
        PdaReverseLookup {
            index: LruCache::new(NonZeroUsize::new(capacity).unwrap()),
        }
    }

    pub fn lookup(&mut self, pda_address: &str) -> Option<String> {
        self.index.get(pda_address).cloned()
    }

    pub fn insert(&mut self, pda_address: String, seed_value: String) -> Option<String> {
        let evicted = if self.index.len() >= self.index.cap().get() {
            self.index.peek_lru().map(|(k, _)| k.clone())
        } else {
            None
        };

        self.index.put(pda_address, seed_value);
        evicted
    }

    pub fn len(&self) -> usize {
        self.index.len()
    }

    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    pub fn contains(&self, pda_address: &str) -> bool {
        self.index.peek(pda_address).is_some()
    }
}

/// Input for queueing an account update.
#[derive(Debug, Clone)]
pub struct QueuedAccountUpdate {
    pub pda_address: String,
    pub account_type: String,
    pub account_data: Value,
    pub slot: u64,
    pub write_version: u64,
    pub signature: String,
}

/// Internal representation of a pending account update with queue metadata.
#[derive(Debug, Clone)]
pub struct PendingAccountUpdate {
    pub account_type: String,
    pub pda_address: String,
    pub account_data: Value,
    pub slot: u64,
    pub write_version: u64,
    pub signature: String,
    pub queued_at: i64,
    /// When true, this update was pulled from the `last_account_data` cache
    /// during a PDA mapping change. It carries stale data from a previous
    /// entity mapping and should use `UpdateContext::new_reprocessed` to
    /// prevent `when` guards from matching stale instruction signatures
    /// and to skip resolver scheduling.
    pub is_stale_reprocess: bool,
}

/// Input for queueing an instruction event when PDA lookup fails.
#[derive(Debug, Clone)]
pub struct QueuedInstructionEvent {
    pub pda_address: String,
    pub event_type: String,
    pub event_data: Value,
    pub slot: u64,
    pub signature: String,
}

/// Internal representation of a pending instruction event with queue metadata.
#[derive(Debug, Clone)]
pub struct PendingInstructionEvent {
    pub event_type: String,
    pub pda_address: String,
    pub event_data: Value,
    pub slot: u64,
    pub signature: String,
    pub queued_at: i64,
}

#[derive(Debug, Clone)]
pub struct DeferredWhenOperation {
    pub entity_name: String,
    pub primary_key: Value,
    pub field_path: String,
    pub field_value: Value,
    pub when_instruction: String,
    pub signature: String,
    pub slot: u64,
    pub deferred_at: i64,
    pub emit: bool,
}

#[derive(Debug, Clone)]
pub struct ResolverRequest {
    pub cache_key: String,
    pub resolver: ResolverType,
    pub input: Value,
}

#[derive(Debug, Clone)]
pub struct ResolverTarget {
    pub state_id: u32,
    pub entity_name: String,
    pub primary_key: Value,
    pub extracts: Vec<ResolverExtractSpec>,
}

#[derive(Debug, Clone)]
pub struct PendingResolverEntry {
    pub resolver: ResolverType,
    pub input: Value,
    pub targets: Vec<ResolverTarget>,
    pub queued_at: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScheduledCallback {
    pub state_id: u32,
    pub entity_name: String,
    pub primary_key: Value,
    pub resolver: ResolverType,
    pub url_template: Option<Vec<ast::UrlTemplatePart>>,
    pub input_value: Option<Value>,
    pub input_path: Option<String>,
    pub condition: Option<ast::ResolverCondition>,
    pub strategy: ResolveStrategy,
    pub extracts: Vec<ResolverExtractSpec>,
    pub retry_count: u32,
}

impl PendingResolverEntry {
    fn add_target(&mut self, target: ResolverTarget) {
        if let Some(existing) = self.targets.iter_mut().find(|t| {
            t.state_id == target.state_id
                && t.entity_name == target.entity_name
                && t.primary_key == target.primary_key
        }) {
            let mut seen = HashSet::new();
            for extract in &existing.extracts {
                seen.insert((extract.target_path.clone(), extract.source_path.clone()));
            }

            for extract in target.extracts {
                let key = (extract.target_path.clone(), extract.source_path.clone());
                if seen.insert(key) {
                    existing.extracts.push(extract);
                }
            }
        } else {
            self.targets.push(target);
        }
    }
}

#[derive(Debug, Clone)]
pub struct PendingQueueStats {
    pub total_updates: usize,
    pub unique_pdas: usize,
    pub oldest_age_seconds: i64,
    pub largest_pda_queue_size: usize,
    pub estimated_memory_bytes: usize,
}

#[derive(Debug, Clone, Default)]
pub struct VmMemoryStats {
    pub state_table_entity_count: usize,
    pub state_table_max_entries: usize,
    pub state_table_at_capacity: bool,
    pub lookup_index_count: usize,
    pub lookup_index_total_entries: usize,
    pub temporal_index_count: usize,
    pub temporal_index_total_entries: usize,
    pub pda_reverse_lookup_count: usize,
    pub pda_reverse_lookup_total_entries: usize,
    pub version_tracker_entries: usize,
    pub pending_queue_stats: Option<PendingQueueStats>,
    pub path_cache_size: usize,
}

#[derive(Debug, Clone, Default)]
pub struct CleanupResult {
    pub pending_updates_removed: usize,
    pub temporal_entries_removed: usize,
}

#[derive(Debug, Clone)]
pub struct CapacityWarning {
    pub current_entries: usize,
    pub max_entries: usize,
    pub entries_over_limit: usize,
}

#[derive(Debug, Clone)]
pub struct StateTableConfig {
    pub max_entries: usize,
    pub max_array_length: usize,
}

impl Default for StateTableConfig {
    fn default() -> Self {
        Self {
            max_entries: DEFAULT_MAX_STATE_TABLE_ENTRIES,
            max_array_length: DEFAULT_MAX_ARRAY_LENGTH,
        }
    }
}

#[derive(Debug)]
pub struct VersionTracker {
    cache: std::sync::Mutex<LruCache<String, (u64, u64)>>,
}

impl VersionTracker {
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_MAX_VERSION_TRACKER_ENTRIES)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        VersionTracker {
            cache: std::sync::Mutex::new(LruCache::new(
                NonZeroUsize::new(capacity).expect("capacity must be > 0"),
            )),
        }
    }

    fn make_key(primary_key: &Value, event_type: &str) -> String {
        format!("{}:{}", primary_key, event_type)
    }

    pub fn get(&self, primary_key: &Value, event_type: &str) -> Option<(u64, u64)> {
        let key = Self::make_key(primary_key, event_type);
        self.cache.lock().unwrap().get(&key).copied()
    }

    pub fn insert(&self, primary_key: &Value, event_type: &str, slot: u64, ordering_value: u64) {
        let key = Self::make_key(primary_key, event_type);
        self.cache.lock().unwrap().put(key, (slot, ordering_value));
    }

    pub fn len(&self) -> usize {
        self.cache.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.cache.lock().unwrap().is_empty()
    }
}

impl Default for VersionTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
pub struct StateTable {
    pub data: DashMap<Value, Value>,
    access_times: DashMap<Value, i64>,
    pub lookup_indexes: HashMap<String, LookupIndex>,
    pub temporal_indexes: HashMap<String, TemporalIndex>,
    pub pda_reverse_lookups: HashMap<String, PdaReverseLookup>,
    pub pending_updates: DashMap<String, Vec<PendingAccountUpdate>>,
    pub pending_instruction_events: DashMap<String, Vec<PendingInstructionEvent>>,
    /// Cache of the most recent account data per PDA address.  When a PDA
    /// mapping changes (same PDA, different seed) the cached data is returned
    /// for reprocessing so cross-account Lookup handlers resolve to the new key.
    pub last_account_data: DashMap<String, PendingAccountUpdate>,
    version_tracker: VersionTracker,
    instruction_dedup_cache: VersionTracker,
    config: StateTableConfig,
    #[cfg_attr(not(feature = "otel"), allow(dead_code))]
    entity_name: String,
    pub recent_tx_instructions:
        std::sync::Mutex<lru::LruCache<String, std::collections::HashSet<String>>>,
    pub deferred_when_ops: DashMap<(String, String), Vec<DeferredWhenOperation>>,
}

impl StateTable {
    pub fn is_at_capacity(&self) -> bool {
        self.data.len() >= self.config.max_entries
    }

    pub fn entries_over_limit(&self) -> usize {
        self.data.len().saturating_sub(self.config.max_entries)
    }

    pub fn max_array_length(&self) -> usize {
        self.config.max_array_length
    }

    fn touch(&self, key: &Value) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        self.access_times.insert(key.clone(), now);
    }

    fn evict_lru(&self, count: usize) -> usize {
        if count == 0 || self.data.is_empty() {
            return 0;
        }

        let mut entries: Vec<(Value, i64)> = self
            .access_times
            .iter()
            .map(|entry| (entry.key().clone(), *entry.value()))
            .collect();

        entries.sort_by_key(|(_, ts)| *ts);

        let to_evict: Vec<Value> = entries.iter().take(count).map(|(k, _)| k.clone()).collect();

        let mut evicted = 0;
        for key in to_evict {
            self.data.remove(&key);
            self.access_times.remove(&key);
            evicted += 1;
        }

        #[cfg(feature = "otel")]
        if evicted > 0 {
            crate::vm_metrics::record_state_table_eviction(evicted as u64, &self.entity_name);
        }

        evicted
    }

    pub fn insert_with_eviction(&self, key: Value, value: Value) {
        if self.data.len() >= self.config.max_entries && !self.data.contains_key(&key) {
            #[cfg(feature = "otel")]
            crate::vm_metrics::record_state_table_at_capacity(&self.entity_name);
            let to_evict = (self.data.len() + 1).saturating_sub(self.config.max_entries);
            self.evict_lru(to_evict.max(1));
        }
        self.data.insert(key.clone(), value);
        self.touch(&key);
    }

    pub fn get_and_touch(&self, key: &Value) -> Option<Value> {
        let result = self.data.get(key).map(|v| v.clone());
        if result.is_some() {
            self.touch(key);
        }
        result
    }

    /// Check if an update is fresh and update the version tracker.
    /// Returns true if the update should be processed (is fresh).
    /// Returns false if the update is stale and should be skipped.
    ///
    /// Comparison is lexicographic on (slot, ordering_value):
    /// (100, 5) > (100, 3) > (99, 999)
    pub fn is_fresh_update(
        &self,
        primary_key: &Value,
        event_type: &str,
        slot: u64,
        ordering_value: u64,
    ) -> bool {
        let dominated = self
            .version_tracker
            .get(primary_key, event_type)
            .map(|(last_slot, last_version)| {
                let is_stale = (slot, ordering_value) <= (last_slot, last_version);
                if is_stale && event_type == "ore::RoundState" {
                    tracing::debug!(
                        entity_key = %primary_key,
                        event_type = %event_type,
                        current_slot = slot,
                        current_write_version = ordering_value,
                        last_slot = last_slot,
                        last_write_version = last_version,
                        "RoundState recency check: stale update detected"
                    );
                }
                is_stale
            })
            .unwrap_or(false);

        if dominated {
            return false;
        }

        self.version_tracker
            .insert(primary_key, event_type, slot, ordering_value);
        true
    }

    /// Check if an instruction is a duplicate of one we've seen recently.
    /// Returns true if this exact instruction has been seen before (is a duplicate).
    /// Returns false if this is a new instruction that should be processed.
    ///
    /// Unlike account updates, instructions don't use recency checks - all
    /// unique instructions are processed. Only exact duplicates are skipped.
    /// Uses a smaller cache capacity for shorter effective TTL.
    pub fn is_duplicate_instruction(
        &self,
        primary_key: &Value,
        event_type: &str,
        slot: u64,
        txn_index: u64,
    ) -> bool {
        // Check if we've seen this exact instruction before
        let is_duplicate = self
            .instruction_dedup_cache
            .get(primary_key, event_type)
            .map(|(last_slot, last_txn_index)| slot == last_slot && txn_index == last_txn_index)
            .unwrap_or(false);

        if is_duplicate {
            return true;
        }

        // Record this instruction for deduplication
        self.instruction_dedup_cache
            .insert(primary_key, event_type, slot, txn_index);
        false
    }
}

impl VmContext {
    pub fn new() -> Self {
        let mut vm = VmContext {
            registers: vec![Value::Null; 256],
            states: HashMap::new(),
            instructions_executed: 0,
            cache_hits: 0,
            path_cache: HashMap::new(),
            pda_cache_hits: 0,
            pda_cache_misses: 0,
            pending_queue_size: 0,
            resolver_requests: VecDeque::new(),
            resolver_pending: HashMap::new(),
            resolver_cache: LruCache::new(
                NonZeroUsize::new(DEFAULT_MAX_RESOLVER_CACHE_ENTRIES)
                    .expect("capacity must be > 0"),
            ),
            resolver_cache_hits: 0,
            resolver_cache_misses: 0,
            current_context: None,
            warnings: Vec::new(),
            last_pda_lookup_miss: None,
            last_lookup_index_miss: None,
            last_pda_registered: None,
            last_lookup_index_keys: Vec::new(),
            scheduled_callbacks: Vec::new(),
        };
        vm.states.insert(
            0,
            StateTable {
                data: DashMap::new(),
                access_times: DashMap::new(),
                lookup_indexes: HashMap::new(),
                temporal_indexes: HashMap::new(),
                pda_reverse_lookups: HashMap::new(),
                pending_updates: DashMap::new(),
                pending_instruction_events: DashMap::new(),
                last_account_data: DashMap::new(),
                version_tracker: VersionTracker::new(),
                instruction_dedup_cache: VersionTracker::with_capacity(
                    DEFAULT_MAX_INSTRUCTION_DEDUP_ENTRIES,
                ),
                config: StateTableConfig::default(),
                entity_name: String::new(),
                recent_tx_instructions: std::sync::Mutex::new(LruCache::new(
                    NonZeroUsize::new(1000).unwrap(),
                )),
                deferred_when_ops: DashMap::new(),
            },
        );

        vm
    }

    /// Create a new VmContext specifically for multi-entity operation.
    pub fn new_multi_entity() -> Self {
        let vm = VmContext {
            registers: vec![Value::Null; 32],
            states: HashMap::new(),
            instructions_executed: 0,
            cache_hits: 0,
            path_cache: HashMap::new(),
            pda_cache_hits: 0,
            pda_cache_misses: 0,
            pending_queue_size: 0,
            resolver_requests: VecDeque::new(),
            resolver_pending: HashMap::new(),
            resolver_cache: LruCache::new(
                NonZeroUsize::new(DEFAULT_MAX_RESOLVER_CACHE_ENTRIES)
                    .expect("capacity must be > 0"),
            ),
            resolver_cache_hits: 0,
            resolver_cache_misses: 0,
            current_context: None,
            warnings: Vec::new(),
            last_pda_lookup_miss: None,
            last_lookup_index_miss: None,
            last_pda_registered: None,
            last_lookup_index_keys: Vec::new(),
            scheduled_callbacks: Vec::new(),
        };

        vm
    }

    pub fn new_with_config(state_config: StateTableConfig) -> Self {
        let mut vm = VmContext {
            registers: vec![Value::Null; 256],
            states: HashMap::new(),
            instructions_executed: 0,
            cache_hits: 0,
            path_cache: HashMap::new(),
            pda_cache_hits: 0,
            pda_cache_misses: 0,
            pending_queue_size: 0,
            resolver_requests: VecDeque::new(),
            resolver_pending: HashMap::new(),
            resolver_cache: LruCache::new(
                NonZeroUsize::new(DEFAULT_MAX_RESOLVER_CACHE_ENTRIES)
                    .expect("capacity must be > 0"),
            ),
            resolver_cache_hits: 0,
            resolver_cache_misses: 0,
            current_context: None,
            warnings: Vec::new(),
            last_pda_lookup_miss: None,
            last_lookup_index_miss: None,
            last_pda_registered: None,
            last_lookup_index_keys: Vec::new(),
            scheduled_callbacks: Vec::new(),
        };
        vm.states.insert(
            0,
            StateTable {
                data: DashMap::new(),
                access_times: DashMap::new(),
                lookup_indexes: HashMap::new(),
                temporal_indexes: HashMap::new(),
                pda_reverse_lookups: HashMap::new(),
                pending_updates: DashMap::new(),
                pending_instruction_events: DashMap::new(),
                last_account_data: DashMap::new(),
                version_tracker: VersionTracker::new(),
                instruction_dedup_cache: VersionTracker::with_capacity(
                    DEFAULT_MAX_INSTRUCTION_DEDUP_ENTRIES,
                ),
                config: state_config,
                entity_name: "default".to_string(),
                recent_tx_instructions: std::sync::Mutex::new(LruCache::new(
                    NonZeroUsize::new(1000).unwrap(),
                )),
                deferred_when_ops: DashMap::new(),
            },
        );
        vm
    }

    pub fn take_resolver_requests(&mut self) -> Vec<ResolverRequest> {
        self.resolver_requests.drain(..).collect()
    }

    pub fn take_scheduled_callbacks(&mut self) -> Vec<(u64, ScheduledCallback)> {
        std::mem::take(&mut self.scheduled_callbacks)
    }

    pub fn get_entity_state(&self, state_id: u32, key: &Value) -> Option<Value> {
        self.states.get(&state_id)?.get_and_touch(key)
    }

    pub fn restore_resolver_requests(&mut self, requests: Vec<ResolverRequest>) {
        if requests.is_empty() {
            return;
        }

        self.resolver_requests.extend(requests);
    }

    pub fn apply_resolver_result(
        &mut self,
        bytecode: &MultiEntityBytecode,
        cache_key: &str,
        resolved_value: Value,
    ) -> Result<Vec<Mutation>> {
        self.resolver_cache
            .put(cache_key.to_string(), resolved_value.clone());

        let entry = match self.resolver_pending.remove(cache_key) {
            Some(entry) => entry,
            None => return Ok(Vec::new()),
        };

        let mut mutations = Vec::new();

        for target in entry.targets {
            if target.primary_key.is_null() {
                continue;
            }

            let entity_bytecode = match bytecode.entities.get(&target.entity_name) {
                Some(bc) => bc,
                None => continue,
            };

            let state = match self.states.get(&target.state_id) {
                Some(s) => s,
                None => continue,
            };

            let mut entity_state = state
                .get_and_touch(&target.primary_key)
                .unwrap_or_else(|| json!({}));

            let mut dirty_tracker = DirtyTracker::new();
            let should_emit = |path: &str| !entity_bytecode.non_emitted_fields.contains(path);

            Self::apply_resolver_extractions_to_value(
                &mut entity_state,
                &resolved_value,
                &target.extracts,
                &mut dirty_tracker,
                &should_emit,
            )?;

            if let Some(evaluator) = entity_bytecode.computed_fields_evaluator.as_ref() {
                let old_values: Vec<_> = entity_bytecode
                    .computed_paths
                    .iter()
                    .map(|path| Self::get_value_at_path(&entity_state, path))
                    .collect();

                let context_slot = self.current_context.as_ref().and_then(|c| c.slot);
                let context_timestamp = self
                    .current_context
                    .as_ref()
                    .map(|c| c.timestamp())
                    .unwrap_or_else(|| {
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap()
                            .as_secs() as i64
                    });
                let eval_result = evaluator(&mut entity_state, context_slot, context_timestamp);

                if eval_result.is_ok() {
                    let mut changed_fields = Vec::new();
                    for (path, old_value) in
                        entity_bytecode.computed_paths.iter().zip(old_values.iter())
                    {
                        let new_value = Self::get_value_at_path(&entity_state, path);
                        let changed = new_value != *old_value;
                        let will_emit = should_emit(path);

                        if changed && will_emit {
                            dirty_tracker.mark_replaced(path);
                            changed_fields.push(path.clone());
                        }
                    }
                }
            }

            state.insert_with_eviction(target.primary_key.clone(), entity_state.clone());

            if dirty_tracker.is_empty() {
                continue;
            }

            let patch = Self::build_partial_state_from_value(&entity_state, &dirty_tracker)?;

            mutations.push(Mutation {
                export: target.entity_name.clone(),
                key: target.primary_key.clone(),
                patch,
                append: vec![],
            });
        }

        Ok(mutations)
    }

    pub fn enqueue_resolver_request(
        &mut self,
        cache_key: String,
        resolver: ResolverType,
        input: Value,
        target: ResolverTarget,
    ) {
        if let Some(entry) = self.resolver_pending.get_mut(&cache_key) {
            entry.add_target(target);
            return;
        }

        let queued_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        self.resolver_pending.insert(
            cache_key.clone(),
            PendingResolverEntry {
                resolver: resolver.clone(),
                input: input.clone(),
                targets: vec![target],
                queued_at,
            },
        );

        self.resolver_requests.push_back(ResolverRequest {
            cache_key,
            resolver,
            input,
        });
    }

    fn apply_resolver_extractions_to_value<F>(
        state: &mut Value,
        resolved_value: &Value,
        extracts: &[ResolverExtractSpec],
        dirty_tracker: &mut DirtyTracker,
        should_emit: &F,
    ) -> Result<()>
    where
        F: Fn(&str) -> bool,
    {
        for extract in extracts {
            let resolved = match extract.source_path.as_deref() {
                Some(path) => Self::get_value_at_path(resolved_value, path),
                None => Some(resolved_value.clone()),
            };

            let Some(value) = resolved else {
                continue;
            };

            let value = if let Some(transform) = &extract.transform {
                Self::apply_transformation(&value, transform)?
            } else {
                value
            };

            Self::set_nested_field_value(state, &extract.target_path, value)?;
            if should_emit(&extract.target_path) {
                dirty_tracker.mark_replaced(&extract.target_path);
            }
        }

        Ok(())
    }

    fn build_partial_state_from_value(state: &Value, tracker: &DirtyTracker) -> Result<Value> {
        if tracker.is_empty() {
            return Ok(json!({}));
        }

        let mut partial = serde_json::Map::new();

        for (path, change) in tracker.iter() {
            let segments: Vec<&str> = path.split('.').collect();

            let value_to_insert = match change {
                FieldChange::Replaced => {
                    let mut current = state;
                    let mut found = true;

                    for segment in &segments {
                        match current.get(*segment) {
                            Some(v) => current = v,
                            None => {
                                found = false;
                                break;
                            }
                        }
                    }

                    if !found {
                        continue;
                    }
                    current.clone()
                }
                FieldChange::Appended(values) => Value::Array(values.clone()),
            };

            let mut target = &mut partial;
            for (i, segment) in segments.iter().enumerate() {
                if i == segments.len() - 1 {
                    target.insert(segment.to_string(), value_to_insert.clone());
                } else {
                    target
                        .entry(segment.to_string())
                        .or_insert_with(|| json!({}));
                    target = target
                        .get_mut(*segment)
                        .and_then(|v| v.as_object_mut())
                        .ok_or("Failed to build nested structure")?;
                }
            }
        }

        Ok(Value::Object(partial))
    }

    /// Get a mutable reference to a state table by ID
    /// Returns None if the state ID doesn't exist
    pub fn get_state_table_mut(&mut self, state_id: u32) -> Option<&mut StateTable> {
        self.states.get_mut(&state_id)
    }

    /// Get public access to registers (for metrics context)
    pub fn registers_mut(&mut self) -> &mut Vec<RegisterValue> {
        &mut self.registers
    }

    /// Get public access to path cache (for metrics context)
    pub fn path_cache(&self) -> &HashMap<String, CompiledPath> {
        &self.path_cache
    }

    /// Get the current update context
    pub fn current_context(&self) -> Option<&UpdateContext> {
        self.current_context.as_ref()
    }

    /// Set the current update context for computed field evaluation
    pub fn set_current_context(&mut self, context: Option<UpdateContext>) {
        self.current_context = context;
    }

    fn add_warning(&mut self, msg: String) {
        self.warnings.push(msg);
    }

    pub fn take_warnings(&mut self) -> Vec<String> {
        std::mem::take(&mut self.warnings)
    }

    pub fn has_warnings(&self) -> bool {
        !self.warnings.is_empty()
    }

    pub fn update_state_from_register(
        &mut self,
        state_id: u32,
        key: Value,
        register: Register,
    ) -> Result<()> {
        let state = self.states.get(&state_id).ok_or("State table not found")?;
        let value = self.registers[register].clone();
        state.insert_with_eviction(key, value);
        Ok(())
    }

    fn reset_registers(&mut self) {
        for reg in &mut self.registers {
            *reg = Value::Null;
        }
    }

    /// Extract only the dirty fields from state (public for use by instruction hooks)
    pub fn extract_partial_state(
        &self,
        state_reg: Register,
        dirty_fields: &HashSet<String>,
    ) -> Result<Value> {
        let full_state = &self.registers[state_reg];

        if dirty_fields.is_empty() {
            return Ok(json!({}));
        }

        let mut partial = serde_json::Map::new();

        for path in dirty_fields {
            let segments: Vec<&str> = path.split('.').collect();

            let mut current = full_state;
            let mut found = true;

            for segment in &segments {
                match current.get(segment) {
                    Some(v) => current = v,
                    None => {
                        found = false;
                        break;
                    }
                }
            }

            if !found {
                continue;
            }

            let mut target = &mut partial;
            for (i, segment) in segments.iter().enumerate() {
                if i == segments.len() - 1 {
                    target.insert(segment.to_string(), current.clone());
                } else {
                    target
                        .entry(segment.to_string())
                        .or_insert_with(|| json!({}));
                    target = target
                        .get_mut(*segment)
                        .and_then(|v| v.as_object_mut())
                        .ok_or("Failed to build nested structure")?;
                }
            }
        }

        Ok(Value::Object(partial))
    }

    /// Extract a patch from state based on the DirtyTracker.
    /// For Replaced fields: extracts the full value from state.
    /// For Appended fields: emits only the appended values as an array.
    pub fn extract_partial_state_with_tracker(
        &self,
        state_reg: Register,
        tracker: &DirtyTracker,
    ) -> Result<Value> {
        let full_state = &self.registers[state_reg];

        if tracker.is_empty() {
            return Ok(json!({}));
        }

        let mut partial = serde_json::Map::new();

        for (path, change) in tracker.iter() {
            let segments: Vec<&str> = path.split('.').collect();

            let value_to_insert = match change {
                FieldChange::Replaced => {
                    let mut current = full_state;
                    let mut found = true;

                    for segment in &segments {
                        match current.get(*segment) {
                            Some(v) => current = v,
                            None => {
                                found = false;
                                break;
                            }
                        }
                    }

                    if !found {
                        continue;
                    }
                    current.clone()
                }
                FieldChange::Appended(values) => Value::Array(values.clone()),
            };

            let mut target = &mut partial;
            for (i, segment) in segments.iter().enumerate() {
                if i == segments.len() - 1 {
                    target.insert(segment.to_string(), value_to_insert.clone());
                } else {
                    target
                        .entry(segment.to_string())
                        .or_insert_with(|| json!({}));
                    target = target
                        .get_mut(*segment)
                        .and_then(|v| v.as_object_mut())
                        .ok_or("Failed to build nested structure")?;
                }
            }
        }

        Ok(Value::Object(partial))
    }

    fn get_compiled_path(&mut self, path: &str) -> CompiledPath {
        if let Some(compiled) = self.path_cache.get(path) {
            self.cache_hits += 1;
            #[cfg(feature = "otel")]
            crate::vm_metrics::record_path_cache_hit();
            return compiled.clone();
        }
        #[cfg(feature = "otel")]
        crate::vm_metrics::record_path_cache_miss();
        let compiled = CompiledPath::new(path);
        self.path_cache.insert(path.to_string(), compiled.clone());
        compiled
    }

    /// Process an event with optional context metadata
    #[cfg_attr(feature = "otel", instrument(
        name = "vm.process_event",
        skip(self, bytecode, event_value, log),
        level = "info",
        fields(
            event_type = %event_type,
            slot = context.as_ref().and_then(|c| c.slot),
        )
    ))]
    pub fn process_event(
        &mut self,
        bytecode: &MultiEntityBytecode,
        event_value: Value,
        event_type: &str,
        context: Option<&UpdateContext>,
        mut log: Option<&mut crate::canonical_log::CanonicalLog>,
    ) -> Result<Vec<Mutation>> {
        self.current_context = context.cloned();

        let mut event_value = event_value;
        if let Some(ctx) = context {
            if let Some(obj) = event_value.as_object_mut() {
                obj.insert("__update_context".to_string(), ctx.to_value());
            }
        }

        let mut all_mutations = Vec::new();

        if event_type.ends_with("IxState") && bytecode.when_events.contains(event_type) {
            if let Some(ctx) = context {
                if let Some(signature) = ctx.signature.clone() {
                    let state_ids: Vec<u32> = self.states.keys().cloned().collect();
                    for state_id in state_ids {
                        if let Some(state) = self.states.get(&state_id) {
                            {
                                let mut cache = state.recent_tx_instructions.lock().unwrap();
                                let entry =
                                    cache.get_or_insert_mut(signature.clone(), HashSet::new);
                                entry.insert(event_type.to_string());
                            }

                            let key = (signature.clone(), event_type.to_string());
                            if let Some((_, deferred_ops)) = state.deferred_when_ops.remove(&key) {
                                tracing::debug!(
                                    event_type = %event_type,
                                    signature = %signature,
                                    deferred_count = deferred_ops.len(),
                                    "flushing deferred when-ops"
                                );
                                for op in deferred_ops {
                                    // Look up the entity bytecode to get the computed fields evaluator
                                    let (evaluator, computed_paths) = bytecode
                                        .entities
                                        .get(&op.entity_name)
                                        .map(|eb| {
                                            (
                                                eb.computed_fields_evaluator.as_ref(),
                                                eb.computed_paths.as_slice(),
                                            )
                                        })
                                        .unwrap_or((None, &[]));

                                    match self.apply_deferred_when_op(
                                        state_id,
                                        &op,
                                        evaluator,
                                        Some(computed_paths),
                                    ) {
                                        Ok(mutations) => all_mutations.extend(mutations),
                                        Err(e) => tracing::warn!(
                                            "Failed to apply deferred when-op: {}",
                                            e
                                        ),
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        if let Some(entity_names) = bytecode.event_routing.get(event_type) {
            for entity_name in entity_names {
                if let Some(entity_bytecode) = bytecode.entities.get(entity_name) {
                    if let Some(handler) = entity_bytecode.handlers.get(event_type) {
                        if let Some(ref mut log) = log {
                            log.set("entity", entity_name.clone());
                            log.inc("handlers", 1);
                        }

                        let opcodes_before = self.instructions_executed;
                        let cache_before = self.cache_hits;
                        let pda_hits_before = self.pda_cache_hits;
                        let pda_misses_before = self.pda_cache_misses;

                        let mutations = self.execute_handler(
                            handler,
                            &event_value,
                            event_type,
                            entity_bytecode.state_id,
                            entity_name,
                            entity_bytecode.computed_fields_evaluator.as_ref(),
                            Some(&entity_bytecode.non_emitted_fields),
                        )?;

                        if let Some(ref mut log) = log {
                            log.inc(
                                "opcodes",
                                (self.instructions_executed - opcodes_before) as i64,
                            );
                            log.inc("cache_hits", (self.cache_hits - cache_before) as i64);
                            log.inc("pda_hits", (self.pda_cache_hits - pda_hits_before) as i64);
                            log.inc(
                                "pda_misses",
                                (self.pda_cache_misses - pda_misses_before) as i64,
                            );
                        }

                        if mutations.is_empty() {
                            // CPI events (suffix "CpiEvent") are transaction-scoped like instructions
                            // (suffix "IxState") and should be queued the same way when PDA lookup fails.
                            let is_tx_event =
                                event_type.ends_with("IxState") || event_type.ends_with("CpiEvent");
                            if let Some(missed_pda) = self.take_last_pda_lookup_miss() {
                                if is_tx_event {
                                    let slot = context.and_then(|c| c.slot).unwrap_or(0);
                                    let signature = context
                                        .and_then(|c| c.signature.clone())
                                        .unwrap_or_default();
                                    let _ = self.queue_instruction_event(
                                        entity_bytecode.state_id,
                                        QueuedInstructionEvent {
                                            pda_address: missed_pda,
                                            event_type: event_type.to_string(),
                                            event_data: event_value.clone(),
                                            slot,
                                            signature,
                                        },
                                    );
                                } else {
                                    // Queue account updates (e.g. BondingCurve) when PDA
                                    // reverse lookup fails. These will be flushed when an
                                    // instruction registers the PDA mapping via
                                    // UpdateLookupIndex.
                                    let slot = context.and_then(|c| c.slot).unwrap_or(0);
                                    let signature = context
                                        .and_then(|c| c.signature.clone())
                                        .unwrap_or_default();
                                    if let Some(write_version) =
                                        context.and_then(|c| c.write_version)
                                    {
                                        let _ = self.queue_account_update(
                                            entity_bytecode.state_id,
                                            QueuedAccountUpdate {
                                                pda_address: missed_pda,
                                                account_type: event_type.to_string(),
                                                account_data: event_value.clone(),
                                                slot,
                                                write_version,
                                                signature,
                                            },
                                        );
                                    } else {
                                        tracing::warn!(
                                            event_type = %event_type,
                                            "Dropping queued account update: write_version missing from context"
                                        );
                                    }
                                }
                            }
                            if let Some(missed_lookup) = self.take_last_lookup_index_miss() {
                                if !is_tx_event {
                                    let slot = context.and_then(|c| c.slot).unwrap_or(0);
                                    let signature = context
                                        .and_then(|c| c.signature.clone())
                                        .unwrap_or_default();
                                    if let Some(write_version) =
                                        context.and_then(|c| c.write_version)
                                    {
                                        let _ = self.queue_account_update(
                                            entity_bytecode.state_id,
                                            QueuedAccountUpdate {
                                                pda_address: missed_lookup,
                                                account_type: event_type.to_string(),
                                                account_data: event_value.clone(),
                                                slot,
                                                write_version,
                                                signature,
                                            },
                                        );
                                    } else {
                                        tracing::trace!(
                                            event_type = %event_type,
                                            "Discarding lookup_index_miss for tx-scoped event (IxState/CpiEvent do not use lookup-index queuing)"
                                        );
                                    }
                                }
                            }
                        }

                        all_mutations.extend(mutations);

                        if event_type.ends_with("IxState") || event_type.ends_with("CpiEvent") {
                            if let Some(ctx) = context {
                                if let Some(ref signature) = ctx.signature {
                                    if let Some(state) = self.states.get(&entity_bytecode.state_id)
                                    {
                                        {
                                            let mut cache =
                                                state.recent_tx_instructions.lock().unwrap();
                                            let entry = cache
                                                .get_or_insert_mut(signature.clone(), HashSet::new);
                                            entry.insert(event_type.to_string());
                                        }

                                        let key = (signature.clone(), event_type.to_string());
                                        if let Some((_, deferred_ops)) =
                                            state.deferred_when_ops.remove(&key)
                                        {
                                            tracing::debug!(
                                                event_type = %event_type,
                                                signature = %signature,
                                                deferred_count = deferred_ops.len(),
                                                "flushing deferred when-ops"
                                            );
                                            for op in deferred_ops {
                                                match self.apply_deferred_when_op(
                                                    entity_bytecode.state_id,
                                                    &op,
                                                    entity_bytecode
                                                        .computed_fields_evaluator
                                                        .as_ref(),
                                                    Some(&entity_bytecode.computed_paths),
                                                ) {
                                                    Ok(mutations) => {
                                                        all_mutations.extend(mutations)
                                                    }
                                                    Err(e) => {
                                                        tracing::warn!(
                                                            "Failed to apply deferred when-op: {}",
                                                            e
                                                        );
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        if let Some(registered_pda) = self.take_last_pda_registered() {
                            let pending_events = self.flush_pending_instruction_events(
                                entity_bytecode.state_id,
                                &registered_pda,
                            );
                            for pending in pending_events {
                                if let Some(pending_handler) =
                                    entity_bytecode.handlers.get(&pending.event_type)
                                {
                                    if let Ok(reprocessed_mutations) = self.execute_handler(
                                        pending_handler,
                                        &pending.event_data,
                                        &pending.event_type,
                                        entity_bytecode.state_id,
                                        entity_name,
                                        entity_bytecode.computed_fields_evaluator.as_ref(),
                                        Some(&entity_bytecode.non_emitted_fields),
                                    ) {
                                        all_mutations.extend(reprocessed_mutations);
                                    }
                                }
                            }
                        }

                        let lookup_keys = self.take_last_lookup_index_keys();
                        if !lookup_keys.is_empty() {
                            tracing::info!(
                                keys = ?lookup_keys,
                                entity = %entity_name,
                                "vm.process_event: flushing pending updates for lookup_keys"
                            );
                        }
                        for lookup_key in lookup_keys {
                            if let Ok(pending_updates) =
                                self.flush_pending_updates(entity_bytecode.state_id, &lookup_key)
                            {
                                for pending in pending_updates {
                                    if let Some(pending_handler) =
                                        entity_bytecode.handlers.get(&pending.account_type)
                                    {
                                        self.current_context = Some(UpdateContext::new_account(
                                            pending.slot,
                                            pending.signature.clone(),
                                            pending.write_version,
                                        ));
                                        match self.execute_handler(
                                            pending_handler,
                                            &pending.account_data,
                                            &pending.account_type,
                                            entity_bytecode.state_id,
                                            entity_name,
                                            entity_bytecode.computed_fields_evaluator.as_ref(),
                                            Some(&entity_bytecode.non_emitted_fields),
                                        ) {
                                            Ok(reprocessed) => {
                                                all_mutations.extend(reprocessed);
                                            }
                                            Err(e) => {
                                                tracing::warn!(
                                                    error = %e,
                                                    account_type = %pending.account_type,
                                                    "Flushed event reprocessing failed"
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    } else if let Some(ref mut log) = log {
                        log.set("skip_reason", "no_handler");
                    }
                } else if let Some(ref mut log) = log {
                    log.set("skip_reason", "entity_not_found");
                }
            }
        } else if let Some(ref mut log) = log {
            log.set("skip_reason", "no_event_routing");
        }

        if let Some(log) = log {
            log.set("mutations", all_mutations.len() as i64);
            if let Some(first) = all_mutations.first() {
                if let Some(key_str) = first.key.as_str() {
                    log.set("primary_key", key_str);
                } else if let Some(key_num) = first.key.as_u64() {
                    log.set("primary_key", key_num as i64);
                }
            }
            if let Some(state) = self.states.get(&0) {
                log.set("state_table_size", state.data.len() as i64);
            }

            let warnings = self.take_warnings();
            if !warnings.is_empty() {
                log.set("warnings", warnings.len() as i64);
                log.set(
                    "warning_messages",
                    Value::Array(warnings.into_iter().map(Value::String).collect()),
                );
                log.set_level(crate::canonical_log::LogLevel::Warn);
            }
        } else {
            self.warnings.clear();
        }

        if self.instructions_executed.is_multiple_of(1000) {
            let state_ids: Vec<u32> = self.states.keys().cloned().collect();
            for state_id in state_ids {
                let expired = self.cleanup_expired_when_ops(state_id, 60);
                if expired > 0 {
                    tracing::debug!(
                        "Cleaned up {} expired deferred when-ops for state {}",
                        expired,
                        state_id
                    );
                }
            }
        }

        Ok(all_mutations)
    }

    pub fn process_any(
        &mut self,
        bytecode: &MultiEntityBytecode,
        any: prost_types::Any,
    ) -> Result<Vec<Mutation>> {
        let (event_value, event_type) = bytecode.proto_router.decode(any)?;
        self.process_event(bytecode, event_value, &event_type, None, None)
    }

    #[cfg_attr(feature = "otel", instrument(
        name = "vm.execute_handler",
        skip(self, handler, event_value, entity_evaluator),
        level = "debug",
        fields(
            event_type = %event_type,
            handler_opcodes = handler.len(),
        )
    ))]
    #[allow(clippy::type_complexity, clippy::too_many_arguments)]
    fn execute_handler(
        &mut self,
        handler: &[OpCode],
        event_value: &Value,
        event_type: &str,
        override_state_id: u32,
        entity_name: &str,
        entity_evaluator: Option<
            &Box<dyn Fn(&mut Value, Option<u64>, i64) -> Result<()> + Send + Sync>,
        >,
        non_emitted_fields: Option<&HashSet<String>>,
    ) -> Result<Vec<Mutation>> {
        self.reset_registers();
        self.last_pda_lookup_miss = None;

        let mut pc: usize = 0;
        let mut output = Vec::new();
        let mut dirty_tracker = DirtyTracker::new();
        let should_emit = |path: &str| {
            non_emitted_fields
                .map(|fields| !fields.contains(path))
                .unwrap_or(true)
        };

        while pc < handler.len() {
            match &handler[pc] {
                OpCode::LoadEventField {
                    path,
                    dest,
                    default,
                } => {
                    let value = self.load_field(event_value, path, default.as_ref())?;
                    self.registers[*dest] = value;
                    pc += 1;
                }
                OpCode::LoadConstant { value, dest } => {
                    self.registers[*dest] = value.clone();
                    pc += 1;
                }
                OpCode::CopyRegister { source, dest } => {
                    self.registers[*dest] = self.registers[*source].clone();
                    pc += 1;
                }
                OpCode::CopyRegisterIfNull { source, dest } => {
                    if self.registers[*dest].is_null() {
                        self.registers[*dest] = self.registers[*source].clone();
                    }
                    pc += 1;
                }
                OpCode::GetEventType { dest } => {
                    self.registers[*dest] = json!(event_type);
                    pc += 1;
                }
                OpCode::CreateObject { dest } => {
                    self.registers[*dest] = json!({});
                    pc += 1;
                }
                OpCode::SetField {
                    object,
                    path,
                    value,
                } => {
                    self.set_field_auto_vivify(*object, path, *value)?;
                    if should_emit(path) {
                        dirty_tracker.mark_replaced(path);
                    }
                    pc += 1;
                }
                OpCode::SetFields { object, fields } => {
                    for (path, value_reg) in fields {
                        self.set_field_auto_vivify(*object, path, *value_reg)?;
                        if should_emit(path) {
                            dirty_tracker.mark_replaced(path);
                        }
                    }
                    pc += 1;
                }
                OpCode::GetField { object, path, dest } => {
                    let value = self.get_field(*object, path)?;
                    self.registers[*dest] = value;
                    pc += 1;
                }
                OpCode::AbortIfNullKey { key } => {
                    let key_value = &self.registers[*key];
                    if key_value.is_null()
                        && event_type.ends_with("State")
                        && !event_type.ends_with("IxState")
                        && !event_type.ends_with("CpiEvent")
                    {
                        tracing::debug!(
                            event_type = %event_type,
                            "AbortIfNullKey: key is null for account state event, \
                             returning empty mutations for queueing"
                        );
                        return Ok(Vec::new());
                    }
                    pc += 1;
                }
                OpCode::ReadOrInitState {
                    state_id: _,
                    key,
                    default,
                    dest,
                } => {
                    let actual_state_id = override_state_id;
                    let entity_name_owned = entity_name.to_string();
                    self.states
                        .entry(actual_state_id)
                        .or_insert_with(|| StateTable {
                            data: DashMap::new(),
                            access_times: DashMap::new(),
                            lookup_indexes: HashMap::new(),
                            temporal_indexes: HashMap::new(),
                            pda_reverse_lookups: HashMap::new(),
                            pending_updates: DashMap::new(),
                            pending_instruction_events: DashMap::new(),
                            last_account_data: DashMap::new(),
                            version_tracker: VersionTracker::new(),
                            instruction_dedup_cache: VersionTracker::with_capacity(
                                DEFAULT_MAX_INSTRUCTION_DEDUP_ENTRIES,
                            ),
                            config: StateTableConfig::default(),
                            entity_name: entity_name_owned,
                            recent_tx_instructions: std::sync::Mutex::new(LruCache::new(
                                NonZeroUsize::new(1000).unwrap(),
                            )),
                            deferred_when_ops: DashMap::new(),
                        });
                    let key_value = self.registers[*key].clone();
                    // Warn if key is null for account state events (not instruction events or CPI events)
                    let warn_null_key = key_value.is_null()
                        && event_type.ends_with("State")
                        && !event_type.ends_with("IxState")
                        && !event_type.ends_with("CpiEvent");

                    if warn_null_key {
                        self.add_warning(format!(
                            "ReadOrInitState: key register {} is NULL for account state, event_type={}",
                            key, event_type
                        ));
                    }

                    let state = self
                        .states
                        .get(&actual_state_id)
                        .ok_or("State table not found")?;

                    if !key_value.is_null() {
                        if let Some(ctx) = &self.current_context {
                            // Account updates: use recency check to discard stale updates
                            // IMPORTANT: Use account address (PDA) as the key, not entity key,
                            // because multiple accounts can update the same entity and each
                            // has its own independent write_version
                            if ctx.is_account_update() {
                                if let (Some(slot), Some(write_version)) =
                                    (ctx.slot, ctx.write_version)
                                {
                                    // Get account address from event_value for proper tracking
                                    let account_address = event_value
                                        .get("__account_address")
                                        .cloned()
                                        .unwrap_or_else(|| key_value.clone());

                                    if !state.is_fresh_update(
                                        &account_address,
                                        event_type,
                                        slot,
                                        write_version,
                                    ) {
                                        self.add_warning(format!(
                                            "Stale account update skipped: slot={}, write_version={}, account={}",
                                            slot, write_version, account_address
                                        ));
                                        return Ok(Vec::new());
                                    }
                                }
                            }
                            // Instruction updates: process all, but skip exact duplicates
                            else if ctx.is_instruction_update() {
                                if let (Some(slot), Some(txn_index)) = (ctx.slot, ctx.txn_index) {
                                    if state.is_duplicate_instruction(
                                        &key_value, event_type, slot, txn_index,
                                    ) {
                                        self.add_warning(format!(
                                            "Duplicate instruction skipped: slot={}, txn_index={}",
                                            slot, txn_index
                                        ));
                                        return Ok(Vec::new());
                                    }
                                }
                            }
                        }
                    }
                    let value = state
                        .get_and_touch(&key_value)
                        .unwrap_or_else(|| default.clone());

                    self.registers[*dest] = value;
                    pc += 1;
                }
                OpCode::UpdateState {
                    state_id: _,
                    key,
                    value,
                } => {
                    let actual_state_id = override_state_id;
                    let state = self
                        .states
                        .get(&actual_state_id)
                        .ok_or("State table not found")?;
                    let key_value = self.registers[*key].clone();
                    let value_data = self.registers[*value].clone();

                    state.insert_with_eviction(key_value, value_data);
                    pc += 1;
                }
                OpCode::AppendToArray {
                    object,
                    path,
                    value,
                } => {
                    let appended_value = self.registers[*value].clone();
                    let max_len = self
                        .states
                        .get(&override_state_id)
                        .map(|s| s.max_array_length())
                        .unwrap_or(DEFAULT_MAX_ARRAY_LENGTH);
                    self.append_to_array(*object, path, *value, max_len)?;
                    if should_emit(path) {
                        dirty_tracker.mark_appended(path, appended_value);
                    }
                    pc += 1;
                }
                OpCode::GetCurrentTimestamp { dest } => {
                    let timestamp = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs() as i64;
                    self.registers[*dest] = json!(timestamp);
                    pc += 1;
                }
                OpCode::CreateEvent { dest, event_value } => {
                    let timestamp = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs() as i64;

                    // Filter out __update_context from the event data
                    let mut event_data = self.registers[*event_value].clone();
                    if let Some(obj) = event_data.as_object_mut() {
                        obj.remove("__update_context");
                    }

                    // Create event with timestamp, data, and optional slot/signature from context
                    let mut event = serde_json::Map::new();
                    event.insert("timestamp".to_string(), json!(timestamp));
                    event.insert("data".to_string(), event_data);

                    // Add slot and signature if available from current context
                    if let Some(ref ctx) = self.current_context {
                        if let Some(slot) = ctx.slot {
                            event.insert("slot".to_string(), json!(slot));
                        }
                        if let Some(ref signature) = ctx.signature {
                            event.insert("signature".to_string(), json!(signature));
                        }
                    }

                    self.registers[*dest] = Value::Object(event);
                    pc += 1;
                }
                OpCode::CreateCapture {
                    dest,
                    capture_value,
                } => {
                    let timestamp = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs() as i64;

                    // Get the capture data (already filtered by load_field)
                    let capture_data = self.registers[*capture_value].clone();

                    // Extract account_address from the original event if available
                    let account_address = event_value
                        .get("__account_address")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();

                    // Create capture wrapper with timestamp, account_address, data, and optional slot/signature
                    let mut capture = serde_json::Map::new();
                    capture.insert("timestamp".to_string(), json!(timestamp));
                    capture.insert("account_address".to_string(), json!(account_address));
                    capture.insert("data".to_string(), capture_data);

                    // Add slot and signature if available from current context
                    if let Some(ref ctx) = self.current_context {
                        if let Some(slot) = ctx.slot {
                            capture.insert("slot".to_string(), json!(slot));
                        }
                        if let Some(ref signature) = ctx.signature {
                            capture.insert("signature".to_string(), json!(signature));
                        }
                    }

                    self.registers[*dest] = Value::Object(capture);
                    pc += 1;
                }
                OpCode::Transform {
                    source,
                    dest,
                    transformation,
                } => {
                    if source == dest {
                        self.transform_in_place(*source, transformation)?;
                    } else {
                        let source_value = &self.registers[*source];
                        let value = Self::apply_transformation(source_value, transformation)?;
                        self.registers[*dest] = value;
                    }
                    pc += 1;
                }
                OpCode::EmitMutation {
                    entity_name,
                    key,
                    state,
                } => {
                    let primary_key = self.registers[*key].clone();

                    if primary_key.is_null() || dirty_tracker.is_empty() {
                        let reason = if dirty_tracker.is_empty() {
                            "no_fields_modified"
                        } else {
                            "null_primary_key"
                        };
                        self.add_warning(format!(
                            "Skipping mutation for entity '{}': {} (dirty_fields={})",
                            entity_name,
                            reason,
                            dirty_tracker.len()
                        ));
                    } else {
                        let patch =
                            self.extract_partial_state_with_tracker(*state, &dirty_tracker)?;

                        let append = dirty_tracker.appended_paths();
                        let mutation = Mutation {
                            export: entity_name.clone(),
                            key: primary_key,
                            patch,
                            append,
                        };
                        output.push(mutation);
                    }
                    pc += 1;
                }
                OpCode::SetFieldIfNull {
                    object,
                    path,
                    value,
                } => {
                    let was_set = self.set_field_if_null(*object, path, *value)?;
                    if was_set && should_emit(path) {
                        dirty_tracker.mark_replaced(path);
                    }
                    pc += 1;
                }
                OpCode::SetFieldMax {
                    object,
                    path,
                    value,
                } => {
                    let was_updated = self.set_field_max(*object, path, *value)?;
                    if was_updated && should_emit(path) {
                        dirty_tracker.mark_replaced(path);
                    }
                    pc += 1;
                }
                OpCode::UpdateTemporalIndex {
                    state_id: _,
                    index_name,
                    lookup_value,
                    primary_key,
                    timestamp,
                } => {
                    let actual_state_id = override_state_id;
                    let state = self
                        .states
                        .get_mut(&actual_state_id)
                        .ok_or("State table not found")?;
                    let index = state
                        .temporal_indexes
                        .entry(index_name.clone())
                        .or_insert_with(TemporalIndex::new);

                    let lookup_val = self.registers[*lookup_value].clone();
                    let pk_val = self.registers[*primary_key].clone();
                    let ts_val = if let Some(val) = self.registers[*timestamp].as_i64() {
                        val
                    } else if let Some(val) = self.registers[*timestamp].as_u64() {
                        val as i64
                    } else {
                        return Err(format!(
                            "Timestamp must be a number (i64 or u64), got: {:?}",
                            self.registers[*timestamp]
                        )
                        .into());
                    };

                    index.insert(lookup_val, pk_val, ts_val);
                    pc += 1;
                }
                OpCode::LookupTemporalIndex {
                    state_id: _,
                    index_name,
                    lookup_value,
                    timestamp,
                    dest,
                } => {
                    let actual_state_id = override_state_id;
                    let state = self
                        .states
                        .get(&actual_state_id)
                        .ok_or("State table not found")?;
                    let lookup_val = &self.registers[*lookup_value];

                    let result = if self.registers[*timestamp].is_null() {
                        if let Some(index) = state.temporal_indexes.get(index_name) {
                            index.lookup_latest(lookup_val).unwrap_or(Value::Null)
                        } else {
                            Value::Null
                        }
                    } else {
                        let ts_val = if let Some(val) = self.registers[*timestamp].as_i64() {
                            val
                        } else if let Some(val) = self.registers[*timestamp].as_u64() {
                            val as i64
                        } else {
                            return Err(format!(
                                "Timestamp must be a number (i64 or u64), got: {:?}",
                                self.registers[*timestamp]
                            )
                            .into());
                        };

                        if let Some(index) = state.temporal_indexes.get(index_name) {
                            index.lookup(lookup_val, ts_val).unwrap_or(Value::Null)
                        } else {
                            Value::Null
                        }
                    };

                    self.registers[*dest] = result;
                    pc += 1;
                }
                OpCode::UpdateLookupIndex {
                    state_id: _,
                    index_name,
                    lookup_value,
                    primary_key,
                } => {
                    let actual_state_id = override_state_id;
                    let state = self
                        .states
                        .get_mut(&actual_state_id)
                        .ok_or("State table not found")?;
                    let index = state
                        .lookup_indexes
                        .entry(index_name.clone())
                        .or_insert_with(LookupIndex::new);

                    let lookup_val = self.registers[*lookup_value].clone();
                    let pk_val = self.registers[*primary_key].clone();

                    index.insert(lookup_val.clone(), pk_val);

                    // Track lookup keys so process_event can flush queued account updates
                    if let Some(key_str) = lookup_val.as_str() {
                        self.last_lookup_index_keys.push(key_str.to_string());
                    }

                    pc += 1;
                }
                OpCode::LookupIndex {
                    state_id: _,
                    index_name,
                    lookup_value,
                    dest,
                } => {
                    let actual_state_id = override_state_id;
                    let mut current_value = self.registers[*lookup_value].clone();

                    const MAX_CHAIN_DEPTH: usize = 5;
                    let mut iterations = 0;

                    let final_result = if self.states.contains_key(&actual_state_id) {
                        loop {
                            iterations += 1;
                            if iterations > MAX_CHAIN_DEPTH {
                                break current_value;
                            }

                            let resolved = self
                                .states
                                .get(&actual_state_id)
                                .and_then(|state| {
                                    if let Some(index) = state.lookup_indexes.get(index_name) {
                                        if let Some(found) = index.lookup(&current_value) {
                                            return Some(found);
                                        }
                                    }

                                    for (name, index) in state.lookup_indexes.iter() {
                                        if name == index_name {
                                            continue;
                                        }
                                        if let Some(found) = index.lookup(&current_value) {
                                            return Some(found);
                                        }
                                    }

                                    None
                                })
                                .unwrap_or(Value::Null);

                            let mut resolved_from_pda = false;
                            let resolved = if resolved.is_null() {
                                if let Some(pda_str) = current_value.as_str() {
                                    resolved_from_pda = true;
                                    self.states
                                        .get_mut(&actual_state_id)
                                        .and_then(|state_mut| {
                                            state_mut
                                                .pda_reverse_lookups
                                                .get_mut("default_pda_lookup")
                                        })
                                        .and_then(|pda_lookup| pda_lookup.lookup(pda_str))
                                        .map(Value::String)
                                        .unwrap_or(Value::Null)
                                } else {
                                    Value::Null
                                }
                            } else {
                                resolved
                            };

                            if resolved.is_null() {
                                if iterations == 1 {
                                    if let Some(pda_str) = current_value.as_str() {
                                        self.last_pda_lookup_miss = Some(pda_str.to_string());
                                    }
                                }
                                break Value::Null;
                            }

                            let can_chain =
                                self.can_resolve_further(&resolved, actual_state_id, index_name);

                            if !can_chain {
                                if resolved_from_pda {
                                    if let Some(resolved_str) = resolved.as_str() {
                                        self.last_lookup_index_miss =
                                            Some(resolved_str.to_string());
                                    }
                                    break Value::Null;
                                }
                                break resolved;
                            }

                            current_value = resolved;
                        }
                    } else {
                        Value::Null
                    };

                    self.registers[*dest] = final_result;
                    pc += 1;
                }
                OpCode::SetFieldSum {
                    object,
                    path,
                    value,
                } => {
                    let was_updated = self.set_field_sum(*object, path, *value)?;
                    if was_updated && should_emit(path) {
                        dirty_tracker.mark_replaced(path);
                    }
                    pc += 1;
                }
                OpCode::SetFieldIncrement { object, path } => {
                    let was_updated = self.set_field_increment(*object, path)?;
                    if was_updated && should_emit(path) {
                        dirty_tracker.mark_replaced(path);
                    }
                    pc += 1;
                }
                OpCode::SetFieldMin {
                    object,
                    path,
                    value,
                } => {
                    let was_updated = self.set_field_min(*object, path, *value)?;
                    if was_updated && should_emit(path) {
                        dirty_tracker.mark_replaced(path);
                    }
                    pc += 1;
                }
                OpCode::AddToUniqueSet {
                    state_id: _,
                    set_name,
                    value,
                    count_object,
                    count_path,
                } => {
                    let value_to_add = self.registers[*value].clone();

                    // Store the unique set within the entity object, not in the state table
                    // This ensures each entity instance has its own unique set
                    let set_field_path = format!("__unique_set:{}", set_name);

                    // Get or create the unique set from the entity object
                    let mut set: HashSet<Value> =
                        if let Ok(existing) = self.get_field(*count_object, &set_field_path) {
                            if !existing.is_null() {
                                serde_json::from_value(existing).unwrap_or_default()
                            } else {
                                HashSet::new()
                            }
                        } else {
                            HashSet::new()
                        };

                    // Add value to set
                    let was_new = set.insert(value_to_add);

                    // Store updated set back in the entity object
                    let set_as_vec: Vec<Value> = set.iter().cloned().collect();
                    self.registers[100] = serde_json::to_value(set_as_vec)?;
                    self.set_field_auto_vivify(*count_object, &set_field_path, 100)?;

                    // Update the count field in the object
                    if was_new {
                        self.registers[100] = Value::Number(serde_json::Number::from(set.len()));
                        self.set_field_auto_vivify(*count_object, count_path, 100)?;
                        if should_emit(count_path) {
                            dirty_tracker.mark_replaced(count_path);
                        }
                    }

                    pc += 1;
                }
                OpCode::ConditionalSetField {
                    object,
                    path,
                    value,
                    condition_field,
                    condition_op,
                    condition_value,
                } => {
                    let field_value = self.load_field(event_value, condition_field, None)?;
                    let condition_met =
                        self.evaluate_comparison(&field_value, condition_op, condition_value)?;

                    if condition_met {
                        self.set_field_auto_vivify(*object, path, *value)?;
                        if should_emit(path) {
                            dirty_tracker.mark_replaced(path);
                        }
                    }
                    pc += 1;
                }
                OpCode::SetFieldWhen {
                    object,
                    path,
                    value,
                    when_instruction,
                    entity_name,
                    key_reg,
                    condition_field,
                    condition_op,
                    condition_value,
                } => {
                    let actual_state_id = override_state_id;
                    let condition_met = if let (Some(field), Some(op), Some(cond_value)) = (
                        condition_field.as_ref(),
                        condition_op.as_ref(),
                        condition_value.as_ref(),
                    ) {
                        let field_value = self.load_field(event_value, field, None)?;
                        self.evaluate_comparison(&field_value, op, cond_value)?
                    } else {
                        true
                    };

                    if !condition_met {
                        pc += 1;
                        continue;
                    }

                    let signature = self
                        .current_context
                        .as_ref()
                        .and_then(|c| c.signature.clone())
                        .unwrap_or_default();

                    let emit = should_emit(path);

                    let instruction_seen = if !signature.is_empty() {
                        if let Some(state) = self.states.get(&actual_state_id) {
                            let mut cache = state.recent_tx_instructions.lock().unwrap();
                            cache
                                .get(&signature)
                                .map(|set| set.contains(when_instruction))
                                .unwrap_or(false)
                        } else {
                            false
                        }
                    } else {
                        false
                    };

                    if instruction_seen {
                        self.set_field_auto_vivify(*object, path, *value)?;
                        if emit {
                            dirty_tracker.mark_replaced(path);
                        }
                    } else if !signature.is_empty() {
                        let deferred = DeferredWhenOperation {
                            entity_name: entity_name.clone(),
                            primary_key: self.registers[*key_reg].clone(),
                            field_path: path.clone(),
                            field_value: self.registers[*value].clone(),
                            when_instruction: when_instruction.clone(),
                            signature: signature.clone(),
                            slot: self
                                .current_context
                                .as_ref()
                                .and_then(|c| c.slot)
                                .unwrap_or(0),
                            deferred_at: std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap()
                                .as_secs() as i64,
                            emit,
                        };

                        if let Some(state) = self.states.get(&actual_state_id) {
                            let key = (signature, when_instruction.clone());
                            state
                                .deferred_when_ops
                                .entry(key)
                                .or_insert_with(Vec::new)
                                .push(deferred);
                        }
                    }

                    pc += 1;
                }
                OpCode::SetFieldUnlessStopped {
                    object,
                    path,
                    value,
                    stop_field,
                    stop_instruction,
                    entity_name,
                    key_reg: _,
                } => {
                    let stop_value = self.get_field(*object, stop_field).unwrap_or(Value::Null);
                    let stopped = stop_value.as_bool().unwrap_or(false);

                    if stopped {
                        tracing::debug!(
                            entity = %entity_name,
                            field = %path,
                            stop_field = %stop_field,
                            stop_instruction = %stop_instruction,
                            "stop flag set; skipping field update"
                        );
                        pc += 1;
                        continue;
                    }

                    self.set_field_auto_vivify(*object, path, *value)?;
                    if should_emit(path) {
                        dirty_tracker.mark_replaced(path);
                    }
                    pc += 1;
                }
                OpCode::ConditionalIncrement {
                    object,
                    path,
                    condition_field,
                    condition_op,
                    condition_value,
                } => {
                    let field_value = self.load_field(event_value, condition_field, None)?;
                    let condition_met =
                        self.evaluate_comparison(&field_value, condition_op, condition_value)?;

                    if condition_met {
                        let was_updated = self.set_field_increment(*object, path)?;
                        if was_updated && should_emit(path) {
                            dirty_tracker.mark_replaced(path);
                        }
                    }
                    pc += 1;
                }
                OpCode::EvaluateComputedFields {
                    state,
                    computed_paths,
                } => {
                    if let Some(evaluator) = entity_evaluator {
                        let old_values: Vec<_> = computed_paths
                            .iter()
                            .map(|path| Self::get_value_at_path(&self.registers[*state], path))
                            .collect();

                        let state_value = &mut self.registers[*state];
                        let context_slot = self.current_context.as_ref().and_then(|c| c.slot);
                        let context_timestamp = self
                            .current_context
                            .as_ref()
                            .map(|c| c.timestamp())
                            .unwrap_or_else(|| {
                                std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap()
                                    .as_secs() as i64
                            });
                        let eval_result = evaluator(state_value, context_slot, context_timestamp);

                        if eval_result.is_ok() {
                            for (path, old_value) in computed_paths.iter().zip(old_values.iter()) {
                                let new_value =
                                    Self::get_value_at_path(&self.registers[*state], path);

                                if new_value != *old_value && should_emit(path) {
                                    dirty_tracker.mark_replaced(path);
                                }
                            }
                        }
                    }
                    pc += 1;
                }
                OpCode::QueueResolver {
                    state_id: _,
                    entity_name,
                    resolver,
                    input_path,
                    input_value,
                    url_template,
                    strategy,
                    extracts,
                    condition,
                    schedule_at,
                    state,
                    key,
                } => {
                    let actual_state_id = override_state_id;

                    // Log what triggered this resolver
                    let trigger_info = if let Some(ref ctx) = self.current_context {
                        if let Some(ref sig) = ctx.signature {
                            format!(
                                "instruction (slot={}, sig={})",
                                ctx.slot.unwrap_or(0),
                                &sig[..std::cmp::min(16, sig.len())]
                            )
                        } else {
                            format!("account update (slot={})", ctx.slot.unwrap_or(0))
                        }
                    } else {
                        "unknown".to_string()
                    };

                    // Skip resolvers for reprocessed cached data from PDA mapping changes.
                    // Stale data can carry wrong field values (e.g. old entropy_value) and
                    // wrong schedule_at slots that would lock in incorrect results via SetOnce.
                    if self
                        .current_context
                        .as_ref()
                        .map(|c| c.skip_resolvers)
                        .unwrap_or(false)
                    {
                        pc += 1;
                        continue;
                    }

                    // Evaluate condition if present
                    if let Some(cond) = condition {
                        let field_val =
                            Self::get_value_at_path(&self.registers[*state], &cond.field_path)
                                .unwrap_or(Value::Null);
                        let condition_met =
                            self.evaluate_comparison(&field_val, &cond.op, &cond.value)?;

                        if !condition_met {
                            pc += 1;
                            continue;
                        }
                    }

                    // Check schedule_at: defer if target slot is in the future
                    if let Some(schedule_path) = schedule_at {
                        let target_val =
                            Self::get_value_at_path(&self.registers[*state], schedule_path);

                        match target_val.and_then(|v| v.as_u64()) {
                            Some(target_slot) => {
                                let current_slot = self
                                    .current_context
                                    .as_ref()
                                    .and_then(|ctx| ctx.slot)
                                    .unwrap_or(0);
                                if current_slot < target_slot {
                                    let key_value = &self.registers[*key];
                                    if !key_value.is_null() {
                                        self.scheduled_callbacks.push((
                                            target_slot,
                                            ScheduledCallback {
                                                state_id: actual_state_id,
                                                entity_name: entity_name.clone(),
                                                primary_key: key_value.clone(),
                                                resolver: resolver.clone(),
                                                url_template: url_template.clone(),
                                                input_value: input_value.clone(),
                                                input_path: input_path.clone(),
                                                condition: condition.clone(),
                                                strategy: strategy.clone(),
                                                extracts: extracts.clone(),
                                                retry_count: 0,
                                            },
                                        ));
                                    }
                                    pc += 1;
                                    continue;
                                }
                                // current_slot >= target_slot: fall through to immediate resolution
                            }
                            None => {
                                // schedule_at path is missing or value is not a u64 —
                                // skip resolver entirely rather than executing immediately,
                                // since the state likely hasn't been fully populated yet.
                                pc += 1;
                                continue;
                            }
                        }
                    }

                    // Build resolver input from template, literal value, or field path
                    let resolved_input = if let Some(template) = url_template {
                        crate::scheduler::build_url_from_template(template, &self.registers[*state])
                            .map(Value::String)
                    } else if let Some(value) = input_value {
                        Some(value.clone())
                    } else if let Some(path) = input_path.as_ref() {
                        Self::get_value_at_path(&self.registers[*state], path)
                    } else {
                        None
                    };

                    if let Some(input) = resolved_input {
                        let key_value = &self.registers[*key];

                        if input.is_null() || key_value.is_null() {
                            pc += 1;
                            continue;
                        }

                        if matches!(strategy, ResolveStrategy::SetOnce)
                            // all(): fire resolver if any target is still null.
                            // Already-set fields are protected from overwrite by
                            // set_value_at_path's SetOnce null-source guard.
                            && extracts.iter().all(|extract| {
                                match Self::get_value_at_path(
                                    &self.registers[*state],
                                    &extract.target_path,
                                ) {
                                    Some(value) => !value.is_null(),
                                    None => false,
                                }
                            })
                        {
                            pc += 1;
                            continue;
                        }

                        let cache_key = resolver_cache_key(resolver, &input);

                        let cached_value = self.resolver_cache.get(&cache_key).cloned();
                        if let Some(cached) = cached_value {
                            self.resolver_cache_hits += 1;
                            Self::apply_resolver_extractions_to_value(
                                &mut self.registers[*state],
                                &cached,
                                extracts,
                                &mut dirty_tracker,
                                &should_emit,
                            )?;
                        } else {
                            self.resolver_cache_misses += 1;
                            let target = ResolverTarget {
                                state_id: actual_state_id,
                                entity_name: entity_name.clone(),
                                primary_key: self.registers[*key].clone(),
                                extracts: extracts.clone(),
                            };

                            self.enqueue_resolver_request(
                                cache_key,
                                resolver.clone(),
                                input,
                                target,
                            );
                        }
                    } else {
                    }

                    pc += 1;
                }
                OpCode::UpdatePdaReverseLookup {
                    state_id: _,
                    lookup_name,
                    pda_address,
                    primary_key,
                } => {
                    let actual_state_id = override_state_id;
                    let state = self
                        .states
                        .get_mut(&actual_state_id)
                        .ok_or("State table not found")?;

                    let pda_val = self.registers[*pda_address].clone();
                    let pk_val = self.registers[*primary_key].clone();

                    if let (Some(pda_str), Some(pk_str)) = (pda_val.as_str(), pk_val.as_str()) {
                        let pda_lookup = state
                            .pda_reverse_lookups
                            .entry(lookup_name.clone())
                            .or_insert_with(|| {
                                PdaReverseLookup::new(DEFAULT_MAX_PDA_REVERSE_LOOKUP_ENTRIES)
                            });

                        pda_lookup.insert(pda_str.to_string(), pk_str.to_string());
                        self.last_pda_registered = Some(pda_str.to_string());
                    } else if !pk_val.is_null() {
                        if let Some(pk_num) = pk_val.as_u64() {
                            if let Some(pda_str) = pda_val.as_str() {
                                let pda_lookup = state
                                    .pda_reverse_lookups
                                    .entry(lookup_name.clone())
                                    .or_insert_with(|| {
                                        PdaReverseLookup::new(
                                            DEFAULT_MAX_PDA_REVERSE_LOOKUP_ENTRIES,
                                        )
                                    });

                                pda_lookup.insert(pda_str.to_string(), pk_num.to_string());
                                self.last_pda_registered = Some(pda_str.to_string());
                            }
                        }
                    }

                    pc += 1;
                }
            }

            self.instructions_executed += 1;
        }

        Ok(output)
    }

    fn load_field(
        &self,
        event_value: &Value,
        path: &FieldPath,
        default: Option<&Value>,
    ) -> Result<Value> {
        if path.segments.is_empty() {
            if let Some(obj) = event_value.as_object() {
                let filtered: serde_json::Map<String, Value> = obj
                    .iter()
                    .filter(|(k, _)| !k.starts_with("__"))
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                return Ok(Value::Object(filtered));
            }
            return Ok(event_value.clone());
        }

        let mut current = event_value;
        for segment in path.segments.iter() {
            current = match current.get(segment) {
                Some(v) => v,
                None => {
                    tracing::trace!(
                        "load_field: segment={:?} not found in {:?}, returning default",
                        segment,
                        current
                    );
                    return Ok(default.cloned().unwrap_or(Value::Null));
                }
            };
        }

        tracing::trace!("load_field: path={:?}, result={:?}", path.segments, current);
        Ok(current.clone())
    }

    fn get_value_at_path(value: &Value, path: &str) -> Option<Value> {
        let mut current = value;
        for segment in path.split('.') {
            current = current.get(segment)?;
        }
        Some(current.clone())
    }

    fn set_field_auto_vivify(
        &mut self,
        object_reg: Register,
        path: &str,
        value_reg: Register,
    ) -> Result<()> {
        let compiled = self.get_compiled_path(path);
        let segments = compiled.segments();
        let value = self.registers[value_reg].clone();

        if !self.registers[object_reg].is_object() {
            self.registers[object_reg] = json!({});
        }

        let obj = self.registers[object_reg]
            .as_object_mut()
            .ok_or("Not an object")?;

        let mut current = obj;
        for (i, segment) in segments.iter().enumerate() {
            if i == segments.len() - 1 {
                current.insert(segment.to_string(), value);
                return Ok(());
            } else {
                current
                    .entry(segment.to_string())
                    .or_insert_with(|| json!({}));
                current = current
                    .get_mut(segment)
                    .and_then(|v| v.as_object_mut())
                    .ok_or("Path collision: expected object")?;
            }
        }

        Ok(())
    }

    fn set_field_if_null(
        &mut self,
        object_reg: Register,
        path: &str,
        value_reg: Register,
    ) -> Result<bool> {
        let compiled = self.get_compiled_path(path);
        let segments = compiled.segments();
        let value = self.registers[value_reg].clone();

        // SetOnce should only set meaningful values. A null source typically means
        // the field doesn't exist in this event type (e.g., instruction events don't
        // have account data). Skip to preserve any existing value.
        if value.is_null() {
            return Ok(false);
        }

        if !self.registers[object_reg].is_object() {
            self.registers[object_reg] = json!({});
        }

        let obj = self.registers[object_reg]
            .as_object_mut()
            .ok_or("Not an object")?;

        let mut current = obj;
        for (i, segment) in segments.iter().enumerate() {
            if i == segments.len() - 1 {
                if !current.contains_key(segment) || current.get(segment).unwrap().is_null() {
                    current.insert(segment.to_string(), value);
                    return Ok(true);
                }
                return Ok(false);
            } else {
                current
                    .entry(segment.to_string())
                    .or_insert_with(|| json!({}));
                current = current
                    .get_mut(segment)
                    .and_then(|v| v.as_object_mut())
                    .ok_or("Path collision: expected object")?;
            }
        }

        Ok(false)
    }

    fn set_field_max(
        &mut self,
        object_reg: Register,
        path: &str,
        value_reg: Register,
    ) -> Result<bool> {
        let compiled = self.get_compiled_path(path);
        let segments = compiled.segments();
        let new_value = self.registers[value_reg].clone();

        if !self.registers[object_reg].is_object() {
            self.registers[object_reg] = json!({});
        }

        let obj = self.registers[object_reg]
            .as_object_mut()
            .ok_or("Not an object")?;

        let mut current = obj;
        for (i, segment) in segments.iter().enumerate() {
            if i == segments.len() - 1 {
                let should_update = if let Some(current_value) = current.get(segment) {
                    if current_value.is_null() {
                        true
                    } else {
                        match (current_value.as_i64(), new_value.as_i64()) {
                            (Some(current_val), Some(new_val)) => new_val > current_val,
                            (Some(current_val), None) if new_value.as_u64().is_some() => {
                                new_value.as_u64().unwrap() as i64 > current_val
                            }
                            (None, Some(new_val)) if current_value.as_u64().is_some() => {
                                new_val > current_value.as_u64().unwrap() as i64
                            }
                            (None, None) => match (current_value.as_u64(), new_value.as_u64()) {
                                (Some(current_val), Some(new_val)) => new_val > current_val,
                                _ => match (current_value.as_f64(), new_value.as_f64()) {
                                    (Some(current_val), Some(new_val)) => new_val > current_val,
                                    _ => false,
                                },
                            },
                            _ => false,
                        }
                    }
                } else {
                    true
                };

                if should_update {
                    current.insert(segment.to_string(), new_value);
                    return Ok(true);
                }
                return Ok(false);
            } else {
                current
                    .entry(segment.to_string())
                    .or_insert_with(|| json!({}));
                current = current
                    .get_mut(segment)
                    .and_then(|v| v.as_object_mut())
                    .ok_or("Path collision: expected object")?;
            }
        }

        Ok(false)
    }

    fn set_field_sum(
        &mut self,
        object_reg: Register,
        path: &str,
        value_reg: Register,
    ) -> Result<bool> {
        let compiled = self.get_compiled_path(path);
        let segments = compiled.segments();
        let new_value = &self.registers[value_reg];

        // Extract numeric value before borrowing object_reg mutably
        tracing::trace!(
            "set_field_sum: path={:?}, value={:?}, value_type={}",
            path,
            new_value,
            match new_value {
                serde_json::Value::Null => "null",
                serde_json::Value::Bool(_) => "bool",
                serde_json::Value::Number(_) => "number",
                serde_json::Value::String(_) => "string",
                serde_json::Value::Array(_) => "array",
                serde_json::Value::Object(_) => "object",
            }
        );
        let new_val_num = new_value
            .as_i64()
            .or_else(|| new_value.as_u64().map(|n| n as i64))
            .ok_or("Sum requires numeric value")?;

        if !self.registers[object_reg].is_object() {
            self.registers[object_reg] = json!({});
        }

        let obj = self.registers[object_reg]
            .as_object_mut()
            .ok_or("Not an object")?;

        let mut current = obj;
        for (i, segment) in segments.iter().enumerate() {
            if i == segments.len() - 1 {
                let current_val = current
                    .get(segment)
                    .and_then(|v| {
                        if v.is_null() {
                            None
                        } else {
                            v.as_i64().or_else(|| v.as_u64().map(|n| n as i64))
                        }
                    })
                    .unwrap_or(0);

                let sum = current_val + new_val_num;
                current.insert(segment.to_string(), json!(sum));
                return Ok(true);
            } else {
                current
                    .entry(segment.to_string())
                    .or_insert_with(|| json!({}));
                current = current
                    .get_mut(segment)
                    .and_then(|v| v.as_object_mut())
                    .ok_or("Path collision: expected object")?;
            }
        }

        Ok(false)
    }

    fn set_field_increment(&mut self, object_reg: Register, path: &str) -> Result<bool> {
        let compiled = self.get_compiled_path(path);
        let segments = compiled.segments();

        if !self.registers[object_reg].is_object() {
            self.registers[object_reg] = json!({});
        }

        let obj = self.registers[object_reg]
            .as_object_mut()
            .ok_or("Not an object")?;

        let mut current = obj;
        for (i, segment) in segments.iter().enumerate() {
            if i == segments.len() - 1 {
                // Get current value (default to 0 if null/missing)
                let current_val = current
                    .get(segment)
                    .and_then(|v| {
                        if v.is_null() {
                            None
                        } else {
                            v.as_i64().or_else(|| v.as_u64().map(|n| n as i64))
                        }
                    })
                    .unwrap_or(0);

                let incremented = current_val + 1;
                current.insert(segment.to_string(), json!(incremented));
                return Ok(true);
            } else {
                current
                    .entry(segment.to_string())
                    .or_insert_with(|| json!({}));
                current = current
                    .get_mut(segment)
                    .and_then(|v| v.as_object_mut())
                    .ok_or("Path collision: expected object")?;
            }
        }

        Ok(false)
    }

    fn set_field_min(
        &mut self,
        object_reg: Register,
        path: &str,
        value_reg: Register,
    ) -> Result<bool> {
        let compiled = self.get_compiled_path(path);
        let segments = compiled.segments();
        let new_value = self.registers[value_reg].clone();

        if !self.registers[object_reg].is_object() {
            self.registers[object_reg] = json!({});
        }

        let obj = self.registers[object_reg]
            .as_object_mut()
            .ok_or("Not an object")?;

        let mut current = obj;
        for (i, segment) in segments.iter().enumerate() {
            if i == segments.len() - 1 {
                let should_update = if let Some(current_value) = current.get(segment) {
                    if current_value.is_null() {
                        true
                    } else {
                        match (current_value.as_i64(), new_value.as_i64()) {
                            (Some(current_val), Some(new_val)) => new_val < current_val,
                            (Some(current_val), None) if new_value.as_u64().is_some() => {
                                (new_value.as_u64().unwrap() as i64) < current_val
                            }
                            (None, Some(new_val)) if current_value.as_u64().is_some() => {
                                new_val < current_value.as_u64().unwrap() as i64
                            }
                            (None, None) => match (current_value.as_u64(), new_value.as_u64()) {
                                (Some(current_val), Some(new_val)) => new_val < current_val,
                                _ => match (current_value.as_f64(), new_value.as_f64()) {
                                    (Some(current_val), Some(new_val)) => new_val < current_val,
                                    _ => false,
                                },
                            },
                            _ => false,
                        }
                    }
                } else {
                    true
                };

                if should_update {
                    current.insert(segment.to_string(), new_value);
                    return Ok(true);
                }
                return Ok(false);
            } else {
                current
                    .entry(segment.to_string())
                    .or_insert_with(|| json!({}));
                current = current
                    .get_mut(segment)
                    .and_then(|v| v.as_object_mut())
                    .ok_or("Path collision: expected object")?;
            }
        }

        Ok(false)
    }

    fn get_field(&mut self, object_reg: Register, path: &str) -> Result<Value> {
        let compiled = self.get_compiled_path(path);
        let segments = compiled.segments();
        let mut current = &self.registers[object_reg];

        for segment in segments {
            current = current
                .get(segment)
                .ok_or_else(|| format!("Field not found: {}", segment))?;
        }

        Ok(current.clone())
    }

    fn append_to_array(
        &mut self,
        object_reg: Register,
        path: &str,
        value_reg: Register,
        max_length: usize,
    ) -> Result<()> {
        let compiled = self.get_compiled_path(path);
        let segments = compiled.segments();
        let value = self.registers[value_reg].clone();

        if !self.registers[object_reg].is_object() {
            self.registers[object_reg] = json!({});
        }

        let obj = self.registers[object_reg]
            .as_object_mut()
            .ok_or("Not an object")?;

        let mut current = obj;
        for (i, segment) in segments.iter().enumerate() {
            if i == segments.len() - 1 {
                current
                    .entry(segment.to_string())
                    .or_insert_with(|| json!([]));
                let arr = current
                    .get_mut(segment)
                    .and_then(|v| v.as_array_mut())
                    .ok_or("Path is not an array")?;
                arr.push(value.clone());

                if arr.len() > max_length {
                    let excess = arr.len() - max_length;
                    arr.drain(0..excess);
                }
            } else {
                current
                    .entry(segment.to_string())
                    .or_insert_with(|| json!({}));
                current = current
                    .get_mut(segment)
                    .and_then(|v| v.as_object_mut())
                    .ok_or("Path collision: expected object")?;
            }
        }

        Ok(())
    }

    fn transform_in_place(&mut self, reg: Register, transformation: &Transformation) -> Result<()> {
        let value = &self.registers[reg];
        let transformed = Self::apply_transformation(value, transformation)?;
        self.registers[reg] = transformed;
        Ok(())
    }

    fn apply_transformation(value: &Value, transformation: &Transformation) -> Result<Value> {
        match transformation {
            Transformation::HexEncode => {
                if let Some(arr) = value.as_array() {
                    let bytes: Vec<u8> = arr
                        .iter()
                        .filter_map(|v| v.as_u64().map(|n| n as u8))
                        .collect();
                    let hex = hex::encode(&bytes);
                    Ok(json!(hex))
                } else if value.is_string() {
                    Ok(value.clone())
                } else {
                    Err("HexEncode requires an array of numbers".into())
                }
            }
            Transformation::HexDecode => {
                if let Some(s) = value.as_str() {
                    let s = s.strip_prefix("0x").unwrap_or(s);
                    let bytes = hex::decode(s).map_err(|e| format!("Hex decode error: {}", e))?;
                    Ok(json!(bytes))
                } else {
                    Err("HexDecode requires a string".into())
                }
            }
            Transformation::Base58Encode => {
                if let Some(arr) = value.as_array() {
                    let bytes: Vec<u8> = arr
                        .iter()
                        .filter_map(|v| v.as_u64().map(|n| n as u8))
                        .collect();
                    let encoded = bs58::encode(&bytes).into_string();
                    Ok(json!(encoded))
                } else if value.is_string() {
                    Ok(value.clone())
                } else {
                    Err("Base58Encode requires an array of numbers".into())
                }
            }
            Transformation::Base58Decode => {
                if let Some(s) = value.as_str() {
                    let bytes = bs58::decode(s)
                        .into_vec()
                        .map_err(|e| format!("Base58 decode error: {}", e))?;
                    Ok(json!(bytes))
                } else {
                    Err("Base58Decode requires a string".into())
                }
            }
            Transformation::ToString => Ok(json!(value.to_string())),
            Transformation::ToNumber => {
                if let Some(s) = value.as_str() {
                    let n = s
                        .parse::<i64>()
                        .map_err(|e| format!("Parse error: {}", e))?;
                    Ok(json!(n))
                } else {
                    Ok(value.clone())
                }
            }
        }
    }

    fn evaluate_comparison(
        &self,
        field_value: &Value,
        op: &ComparisonOp,
        condition_value: &Value,
    ) -> Result<bool> {
        use ComparisonOp::*;

        match op {
            Equal => Ok(field_value == condition_value),
            NotEqual => Ok(field_value != condition_value),
            GreaterThan => {
                // Try to compare as numbers
                match (field_value.as_i64(), condition_value.as_i64()) {
                    (Some(a), Some(b)) => Ok(a > b),
                    _ => match (field_value.as_u64(), condition_value.as_u64()) {
                        (Some(a), Some(b)) => Ok(a > b),
                        _ => match (field_value.as_f64(), condition_value.as_f64()) {
                            (Some(a), Some(b)) => Ok(a > b),
                            _ => Err("Cannot compare non-numeric values with GreaterThan".into()),
                        },
                    },
                }
            }
            GreaterThanOrEqual => match (field_value.as_i64(), condition_value.as_i64()) {
                (Some(a), Some(b)) => Ok(a >= b),
                _ => match (field_value.as_u64(), condition_value.as_u64()) {
                    (Some(a), Some(b)) => Ok(a >= b),
                    _ => match (field_value.as_f64(), condition_value.as_f64()) {
                        (Some(a), Some(b)) => Ok(a >= b),
                        _ => {
                            Err("Cannot compare non-numeric values with GreaterThanOrEqual".into())
                        }
                    },
                },
            },
            LessThan => match (field_value.as_i64(), condition_value.as_i64()) {
                (Some(a), Some(b)) => Ok(a < b),
                _ => match (field_value.as_u64(), condition_value.as_u64()) {
                    (Some(a), Some(b)) => Ok(a < b),
                    _ => match (field_value.as_f64(), condition_value.as_f64()) {
                        (Some(a), Some(b)) => Ok(a < b),
                        _ => Err("Cannot compare non-numeric values with LessThan".into()),
                    },
                },
            },
            LessThanOrEqual => match (field_value.as_i64(), condition_value.as_i64()) {
                (Some(a), Some(b)) => Ok(a <= b),
                _ => match (field_value.as_u64(), condition_value.as_u64()) {
                    (Some(a), Some(b)) => Ok(a <= b),
                    _ => match (field_value.as_f64(), condition_value.as_f64()) {
                        (Some(a), Some(b)) => Ok(a <= b),
                        _ => Err("Cannot compare non-numeric values with LessThanOrEqual".into()),
                    },
                },
            },
        }
    }

    fn can_resolve_further(&self, value: &Value, state_id: u32, index_name: &str) -> bool {
        if let Some(state) = self.states.get(&state_id) {
            if let Some(index) = state.lookup_indexes.get(index_name) {
                if index.lookup(value).is_some() {
                    return true;
                }
            }

            for (name, index) in state.lookup_indexes.iter() {
                if name == index_name {
                    continue;
                }
                if index.lookup(value).is_some() {
                    return true;
                }
            }

            if let Some(pda_str) = value.as_str() {
                if let Some(pda_lookup) = state.pda_reverse_lookups.get("default_pda_lookup") {
                    if pda_lookup.contains(pda_str) {
                        return true;
                    }
                }
            }
        }

        false
    }

    fn apply_deferred_when_op(
        &mut self,
        state_id: u32,
        op: &DeferredWhenOperation,
        entity_evaluator: Option<
            &Box<dyn Fn(&mut Value, Option<u64>, i64) -> Result<()> + Send + Sync>,
        >,
        computed_paths: Option<&[String]>,
    ) -> Result<Vec<Mutation>> {
        let state = self.states.get(&state_id).ok_or("State not found")?;

        if op.primary_key.is_null() {
            return Ok(vec![]);
        }

        let mut entity_state = state
            .get_and_touch(&op.primary_key)
            .unwrap_or_else(|| json!({}));

        // Track old values of computed fields before setting the new value
        let old_computed_values: Vec<_> = computed_paths
            .map(|paths| {
                paths
                    .iter()
                    .map(|path| Self::get_value_at_path(&entity_state, path))
                    .collect()
            })
            .unwrap_or_default();

        Self::set_nested_field_value(&mut entity_state, &op.field_path, op.field_value.clone())?;

        // Re-evaluate computed fields if an evaluator is provided
        if let Some(evaluator) = entity_evaluator {
            let context_slot = self.current_context.as_ref().and_then(|c| c.slot);
            let context_timestamp = self
                .current_context
                .as_ref()
                .map(|c| c.timestamp())
                .unwrap_or_else(|| {
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs() as i64
                });

            tracing::debug!(
                entity_name = %op.entity_name,
                primary_key = %op.primary_key,
                field_path = %op.field_path,
                "Re-evaluating computed fields after deferred when-op"
            );

            if let Err(e) = evaluator(&mut entity_state, context_slot, context_timestamp) {
                tracing::warn!(
                    entity_name = %op.entity_name,
                    primary_key = %op.primary_key,
                    error = %e,
                    "Failed to evaluate computed fields after deferred when-op"
                );
            }
        }

        state.insert_with_eviction(op.primary_key.clone(), entity_state.clone());

        if !op.emit {
            return Ok(vec![]);
        }

        let mut patch = json!({});
        Self::set_nested_field_value(&mut patch, &op.field_path, op.field_value.clone())?;

        // Add computed field changes to the patch
        if let Some(paths) = computed_paths {
            tracing::debug!(
                entity_name = %op.entity_name,
                primary_key = %op.primary_key,
                computed_paths_count = paths.len(),
                "Checking computed fields for changes after deferred when-op"
            );
            for (path, old_value) in paths.iter().zip(old_computed_values.iter()) {
                let new_value = Self::get_value_at_path(&entity_state, path);
                tracing::debug!(
                    entity_name = %op.entity_name,
                    primary_key = %op.primary_key,
                    field_path = %path,
                    old_value = ?old_value,
                    new_value = ?new_value,
                    "Comparing computed field values"
                );
                if let Some(ref new_val) = new_value {
                    if Some(new_val) != old_value.as_ref() {
                        Self::set_nested_field_value(&mut patch, path, new_val.clone())?;
                        tracing::info!(
                            entity_name = %op.entity_name,
                            primary_key = %op.primary_key,
                            field_path = %path,
                            "Computed field changed after deferred when-op, including in mutation"
                        );
                    }
                }
            }
        }

        Ok(vec![Mutation {
            export: op.entity_name.clone(),
            key: op.primary_key.clone(),
            patch,
            append: vec![],
        }])
    }

    fn set_nested_field_value(obj: &mut Value, path: &str, value: Value) -> Result<()> {
        let parts: Vec<&str> = path.split('.').collect();
        let mut current = obj;

        for (i, part) in parts.iter().enumerate() {
            if i == parts.len() - 1 {
                if let Some(map) = current.as_object_mut() {
                    map.insert(part.to_string(), value);
                    return Ok(());
                }
                return Err("Cannot set field on non-object".into());
            }

            if current.get(*part).is_none() || !current.get(*part).unwrap().is_object() {
                if let Some(map) = current.as_object_mut() {
                    map.insert(part.to_string(), json!({}));
                }
            }

            current = current.get_mut(*part).ok_or("Path navigation failed")?;
        }

        Ok(())
    }

    pub fn cleanup_expired_when_ops(&mut self, state_id: u32, max_age_secs: i64) -> usize {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let state = match self.states.get(&state_id) {
            Some(s) => s,
            None => return 0,
        };

        let mut removed = 0;
        state.deferred_when_ops.retain(|_, ops| {
            let before = ops.len();
            ops.retain(|op| now - op.deferred_at < max_age_secs);
            removed += before - ops.len();
            !ops.is_empty()
        });

        removed
    }

    /// Update a PDA reverse lookup and return pending updates for reprocessing.
    /// Returns any pending account updates that were queued for this PDA.
    /// ```ignore
    /// let pending = vm.update_pda_reverse_lookup(state_id, lookup_name, pda_addr, seed)?;
    /// for update in pending {
    ///     vm.process_event(&bytecode, update.account_data, &update.account_type, None, None)?;
    /// }
    /// ```
    #[cfg_attr(feature = "otel", instrument(
        name = "vm.update_pda_lookup",
        skip(self),
        fields(
            pda = %pda_address,
            seed = %seed_value,
        )
    ))]
    pub fn update_pda_reverse_lookup(
        &mut self,
        state_id: u32,
        lookup_name: &str,
        pda_address: String,
        seed_value: String,
    ) -> Result<Vec<PendingAccountUpdate>> {
        let state = self
            .states
            .get_mut(&state_id)
            .ok_or("State table not found")?;

        let lookup = state
            .pda_reverse_lookups
            .entry(lookup_name.to_string())
            .or_insert_with(|| PdaReverseLookup::new(DEFAULT_MAX_PDA_REVERSE_LOOKUP_ENTRIES));

        // Detect if the PDA mapping is CHANGING (same PDA, different seed).
        // This happens at round boundaries when e.g. entropyVar is remapped
        // from old_round to new_round by a Reset instruction.
        let old_seed = lookup.index.peek(&pda_address).cloned();
        let mapping_changed = old_seed
            .as_ref()
            .map(|old| old != &seed_value)
            .unwrap_or(false);

        if !mapping_changed && old_seed.is_none() {
            tracing::info!(
                pda = %pda_address,
                seed = %seed_value,
                "[PDA] First-time PDA reverse lookup established"
            );
        } else if !mapping_changed {
            tracing::debug!(
                pda = %pda_address,
                seed = %seed_value,
                "[PDA] PDA reverse lookup re-registered (same mapping)"
            );
        }

        let evicted_pda = lookup.insert(pda_address.clone(), seed_value.clone());

        if let Some(ref evicted) = evicted_pda {
            if let Some((_, evicted_updates)) = state.pending_updates.remove(evicted) {
                let count = evicted_updates.len();
                self.pending_queue_size = self.pending_queue_size.saturating_sub(count as u64);
            }
        }

        // Flush pending updates from QueueUntil for this PDA
        let mut pending = self.flush_pending_updates(state_id, &pda_address)?;

        // When the mapping changed, the last account update for this PDA was
        // processed with the OLD seed (wrong key).  We need to:
        // 1. Remove stale lookup-index entries that map this PDA address to the
        //    old primary key — otherwise LookupIndex resolves the stale entry
        //    before PDA reverse lookup is even tried.
        // 2. Pull the cached account data and return it for reprocessing with
        //    the new mapping.
        if mapping_changed {
            if let Some(state) = self.states.get(&state_id) {
                // Clear stale lookup-index entries for this PDA address
                for index in state.lookup_indexes.values() {
                    index.remove(&Value::String(pda_address.clone()));
                }

                if let Some((_, mut cached)) = state.last_account_data.remove(&pda_address) {
                    tracing::info!(
                        pda = %pda_address,
                        old_seed = ?old_seed,
                        new_seed = %seed_value,
                        account_type = %cached.account_type,
                        "PDA mapping changed — clearing stale indexes and reprocessing cached data"
                    );
                    cached.is_stale_reprocess = true;
                    pending.push(cached);
                }
            }
        }

        Ok(pending)
    }

    /// Clean up expired pending updates that are older than the TTL
    ///
    /// Returns the number of updates that were removed.
    /// This should be called periodically to prevent memory leaks from orphaned updates.
    pub fn cleanup_expired_pending_updates(&mut self, state_id: u32) -> usize {
        let state = match self.states.get_mut(&state_id) {
            Some(s) => s,
            None => return 0,
        };

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let mut removed_count = 0;

        // Iterate through all pending updates and remove expired ones
        state.pending_updates.retain(|_pda_address, updates| {
            let original_len = updates.len();

            updates.retain(|update| {
                let age = now - update.queued_at;
                age <= PENDING_UPDATE_TTL_SECONDS
            });

            removed_count += original_len - updates.len();

            // Remove the entry entirely if no updates remain
            !updates.is_empty()
        });

        // Update the global counter
        self.pending_queue_size = self.pending_queue_size.saturating_sub(removed_count as u64);

        if removed_count > 0 {
            #[cfg(feature = "otel")]
            crate::vm_metrics::record_pending_updates_expired(
                removed_count as u64,
                &state.entity_name,
            );
        }

        removed_count
    }

    /// Queue an account update for later processing when PDA reverse lookup is not yet available
    ///
    /// # Workflow
    ///
    /// This implements a deferred processing pattern for account updates when the PDA reverse
    /// lookup needed to resolve the primary key is not yet available:
    ///
    /// 1. **Initial Account Update**: When an account update arrives but the PDA reverse lookup
    ///    is not available, call `queue_account_update()` to queue it for later.
    ///
    /// 2. **Register PDA Mapping**: When the instruction that establishes the PDA mapping is
    ///    processed, call `update_pda_reverse_lookup()` which returns pending updates.
    ///
    /// 3. **Reprocess Pending Updates**: Process the returned pending updates through the VM:
    ///    ```ignore
    ///    let pending = vm.update_pda_reverse_lookup(state_id, lookup_name, pda_addr, seed)?;
    ///    for update in pending {
    ///        let mutations = vm.process_event(
    ///            &bytecode, update.account_data, &update.account_type, None, None
    ///        )?;
    ///    }
    ///    ```
    ///
    /// # Arguments
    ///
    /// * `state_id` - The state table ID
    /// * `pda_address` - The PDA address that needs reverse lookup
    /// * `account_type` - The event type name for reprocessing
    /// * `account_data` - The account data (event value) for reprocessing
    /// * `slot` - The slot number when this update occurred
    /// * `signature` - The transaction signature
    #[cfg_attr(feature = "otel", instrument(
        name = "vm.queue_account_update",
        skip(self, update),
        fields(
            pda = %update.pda_address,
            account_type = %update.account_type,
            slot = update.slot,
        )
    ))]
    pub fn queue_account_update(
        &mut self,
        state_id: u32,
        update: QueuedAccountUpdate,
    ) -> Result<()> {
        if self.pending_queue_size >= MAX_PENDING_UPDATES_TOTAL as u64 {
            self.cleanup_expired_pending_updates(state_id);
            if self.pending_queue_size >= MAX_PENDING_UPDATES_TOTAL as u64 {
                self.drop_oldest_pending_update(state_id)?;
            }
        }

        let state = self
            .states
            .get_mut(&state_id)
            .ok_or("State table not found")?;

        let pending = PendingAccountUpdate {
            account_type: update.account_type,
            pda_address: update.pda_address.clone(),
            account_data: update.account_data,
            slot: update.slot,
            write_version: update.write_version,
            signature: update.signature,
            queued_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64,
            is_stale_reprocess: false,
        };

        let pda_address = pending.pda_address.clone();
        let slot = pending.slot;

        let mut updates = state
            .pending_updates
            .entry(pda_address.clone())
            .or_insert_with(Vec::new);

        let original_len = updates.len();
        updates.retain(|existing| existing.slot > slot);
        let removed_by_dedup = original_len - updates.len();

        if removed_by_dedup > 0 {
            self.pending_queue_size = self
                .pending_queue_size
                .saturating_sub(removed_by_dedup as u64);
        }

        if updates.len() >= MAX_PENDING_UPDATES_PER_PDA {
            updates.remove(0);
            self.pending_queue_size = self.pending_queue_size.saturating_sub(1);
        }

        updates.push(pending);
        #[cfg(feature = "otel")]
        crate::vm_metrics::record_pending_update_queued(&state.entity_name);

        Ok(())
    }

    pub fn queue_instruction_event(
        &mut self,
        state_id: u32,
        event: QueuedInstructionEvent,
    ) -> Result<()> {
        let state = self
            .states
            .get_mut(&state_id)
            .ok_or("State table not found")?;

        let pda_address = event.pda_address.clone();

        let pending = PendingInstructionEvent {
            event_type: event.event_type,
            pda_address: event.pda_address,
            event_data: event.event_data,
            slot: event.slot,
            signature: event.signature,
            queued_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64,
        };

        let mut events = state
            .pending_instruction_events
            .entry(pda_address)
            .or_insert_with(Vec::new);

        if events.len() >= MAX_PENDING_UPDATES_PER_PDA {
            events.remove(0);
        }

        events.push(pending);

        Ok(())
    }

    pub fn take_last_pda_lookup_miss(&mut self) -> Option<String> {
        self.last_pda_lookup_miss.take()
    }

    pub fn take_last_lookup_index_miss(&mut self) -> Option<String> {
        self.last_lookup_index_miss.take()
    }

    pub fn take_last_pda_registered(&mut self) -> Option<String> {
        self.last_pda_registered.take()
    }

    pub fn take_last_lookup_index_keys(&mut self) -> Vec<String> {
        std::mem::take(&mut self.last_lookup_index_keys)
    }

    pub fn flush_pending_instruction_events(
        &mut self,
        state_id: u32,
        pda_address: &str,
    ) -> Vec<PendingInstructionEvent> {
        let state = match self.states.get_mut(&state_id) {
            Some(s) => s,
            None => return Vec::new(),
        };

        if let Some((_, events)) = state.pending_instruction_events.remove(pda_address) {
            events
        } else {
            Vec::new()
        }
    }

    /// Get statistics about the pending queue for monitoring
    pub fn get_pending_queue_stats(&self, state_id: u32) -> Option<PendingQueueStats> {
        let state = self.states.get(&state_id)?;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let mut total_updates = 0;
        let mut oldest_timestamp = now;
        let mut largest_pda_queue = 0;
        let mut estimated_memory = 0;

        for entry in state.pending_updates.iter() {
            let (_, updates) = entry.pair();
            total_updates += updates.len();
            largest_pda_queue = largest_pda_queue.max(updates.len());

            for update in updates.iter() {
                oldest_timestamp = oldest_timestamp.min(update.queued_at);
                // Rough memory estimate
                estimated_memory += update.account_type.len() +
                                   update.pda_address.len() +
                                   update.signature.len() +
                                   16 + // slot + queued_at
                                   estimate_json_size(&update.account_data);
            }
        }

        Some(PendingQueueStats {
            total_updates,
            unique_pdas: state.pending_updates.len(),
            oldest_age_seconds: now - oldest_timestamp,
            largest_pda_queue_size: largest_pda_queue,
            estimated_memory_bytes: estimated_memory,
        })
    }

    pub fn get_memory_stats(&self, state_id: u32) -> VmMemoryStats {
        let mut stats = VmMemoryStats {
            path_cache_size: self.path_cache.len(),
            ..Default::default()
        };

        if let Some(state) = self.states.get(&state_id) {
            stats.state_table_entity_count = state.data.len();
            stats.state_table_max_entries = state.config.max_entries;
            stats.state_table_at_capacity = state.is_at_capacity();

            stats.lookup_index_count = state.lookup_indexes.len();
            stats.lookup_index_total_entries =
                state.lookup_indexes.values().map(|idx| idx.len()).sum();

            stats.temporal_index_count = state.temporal_indexes.len();
            stats.temporal_index_total_entries = state
                .temporal_indexes
                .values()
                .map(|idx| idx.total_entries())
                .sum();

            stats.pda_reverse_lookup_count = state.pda_reverse_lookups.len();
            stats.pda_reverse_lookup_total_entries = state
                .pda_reverse_lookups
                .values()
                .map(|lookup| lookup.len())
                .sum();

            stats.version_tracker_entries = state.version_tracker.len();

            stats.pending_queue_stats = self.get_pending_queue_stats(state_id);
        }

        stats
    }

    pub fn cleanup_all_expired(&mut self, state_id: u32) -> CleanupResult {
        let pending_removed = self.cleanup_expired_pending_updates(state_id);
        let temporal_removed = self.cleanup_temporal_indexes(state_id);

        #[cfg(feature = "otel")]
        if let Some(state) = self.states.get(&state_id) {
            crate::vm_metrics::record_cleanup(
                pending_removed,
                temporal_removed,
                &state.entity_name,
            );
        }

        CleanupResult {
            pending_updates_removed: pending_removed,
            temporal_entries_removed: temporal_removed,
        }
    }

    fn cleanup_temporal_indexes(&mut self, state_id: u32) -> usize {
        let state = match self.states.get_mut(&state_id) {
            Some(s) => s,
            None => return 0,
        };

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let cutoff = now - TEMPORAL_HISTORY_TTL_SECONDS;
        let mut total_removed = 0;

        for (_, index) in state.temporal_indexes.iter_mut() {
            total_removed += index.cleanup_expired(cutoff);
        }

        total_removed
    }

    pub fn check_state_table_capacity(&self, state_id: u32) -> Option<CapacityWarning> {
        let state = self.states.get(&state_id)?;

        if state.is_at_capacity() {
            Some(CapacityWarning {
                current_entries: state.data.len(),
                max_entries: state.config.max_entries,
                entries_over_limit: state.entries_over_limit(),
            })
        } else {
            None
        }
    }

    /// Drop the oldest pending update across all PDAs
    fn drop_oldest_pending_update(&mut self, state_id: u32) -> Result<()> {
        let state = self
            .states
            .get_mut(&state_id)
            .ok_or("State table not found")?;

        let mut oldest_pda: Option<String> = None;
        let mut oldest_timestamp = i64::MAX;

        // Find the PDA with the oldest update
        for entry in state.pending_updates.iter() {
            let (pda, updates) = entry.pair();
            if let Some(update) = updates.first() {
                if update.queued_at < oldest_timestamp {
                    oldest_timestamp = update.queued_at;
                    oldest_pda = Some(pda.clone());
                }
            }
        }

        // Remove the oldest update
        if let Some(pda) = oldest_pda {
            if let Some(mut updates) = state.pending_updates.get_mut(&pda) {
                if !updates.is_empty() {
                    updates.remove(0);
                    self.pending_queue_size = self.pending_queue_size.saturating_sub(1);

                    // Remove the entry if it's now empty
                    if updates.is_empty() {
                        drop(updates);
                        state.pending_updates.remove(&pda);
                    }
                }
            }
        }

        Ok(())
    }

    /// Flush and return pending updates for a PDA for external reprocessing
    ///
    /// Returns the pending updates that were queued for this PDA address.
    /// The caller should reprocess these through the VM using process_event().
    fn flush_pending_updates(
        &mut self,
        state_id: u32,
        pda_address: &str,
    ) -> Result<Vec<PendingAccountUpdate>> {
        let state = self
            .states
            .get_mut(&state_id)
            .ok_or("State table not found")?;

        if let Some((_, pending_updates)) = state.pending_updates.remove(pda_address) {
            let count = pending_updates.len();
            self.pending_queue_size = self.pending_queue_size.saturating_sub(count as u64);
            #[cfg(feature = "otel")]
            crate::vm_metrics::record_pending_updates_flushed(count as u64, &state.entity_name);
            Ok(pending_updates)
        } else {
            Ok(Vec::new())
        }
    }

    /// Cache the most recent account data for a PDA address.
    /// Called by the vixen runtime after a Lookup-handler account update is
    /// successfully processed.  When a PDA mapping later changes (e.g. at a
    /// round boundary), the cached data is returned for reprocessing with the
    /// new mapping.
    pub fn cache_last_account_data(
        &mut self,
        state_id: u32,
        pda_address: &str,
        update: PendingAccountUpdate,
    ) {
        if let Some(state) = self.states.get(&state_id) {
            state
                .last_account_data
                .insert(pda_address.to_string(), update);
        }
    }

    /// Try to resolve a primary key via PDA reverse lookup
    pub fn try_pda_reverse_lookup(
        &mut self,
        state_id: u32,
        lookup_name: &str,
        pda_address: &str,
    ) -> Option<String> {
        let state = self.states.get_mut(&state_id)?;

        if let Some(lookup) = state.pda_reverse_lookups.get_mut(lookup_name) {
            if let Some(value) = lookup.lookup(pda_address) {
                self.pda_cache_hits += 1;
                return Some(value);
            }
        }

        self.pda_cache_misses += 1;
        None
    }

    /// Try to resolve a value through lookup indexes.
    /// This attempts to resolve the value using any lookup index in the state.
    pub fn try_lookup_index_resolution(&self, state_id: u32, value: &Value) -> Option<Value> {
        let state = self.states.get(&state_id)?;

        for index in state.lookup_indexes.values() {
            if let Some(resolved) = index.lookup(value) {
                return Some(resolved);
            }
        }

        None
    }

    /// Try to resolve a primary key via chained PDA + lookup index resolution.
    /// First tries PDA reverse lookup, then tries to resolve the result through lookup indexes.
    /// This is useful when the PDA maps to an intermediate value (e.g., round address)
    /// that needs to be resolved to the actual primary key (e.g., round_id).
    pub fn try_chained_pda_lookup(
        &mut self,
        state_id: u32,
        lookup_name: &str,
        pda_address: &str,
    ) -> Option<String> {
        // First, try PDA reverse lookup
        let pda_result = self.try_pda_reverse_lookup(state_id, lookup_name, pda_address)?;

        // Try to resolve the PDA result through lookup indexes
        // (e.g., round address -> round_id)
        let pda_value = Value::String(pda_result.clone());
        if let Some(resolved) = self.try_lookup_index_resolution(state_id, &pda_value) {
            resolved.as_str().map(|s| s.to_string())
        } else {
            // Return the PDA result if we can't resolve further
            pda_value.as_str().map(|s| s.to_string())
        }
    }

    // ============================================================================
    // Computed Expression Evaluator (Task 5)
    // ============================================================================

    /// Evaluate a computed expression AST against the current state
    /// This is the core runtime evaluator for computed fields from the AST
    pub fn evaluate_computed_expr(&self, expr: &ComputedExpr, state: &Value) -> Result<Value> {
        self.evaluate_computed_expr_with_env(expr, state, &std::collections::HashMap::new())
    }

    /// Evaluate a computed expression with a variable environment (for let bindings)
    fn evaluate_computed_expr_with_env(
        &self,
        expr: &ComputedExpr,
        state: &Value,
        env: &std::collections::HashMap<String, Value>,
    ) -> Result<Value> {
        match expr {
            ComputedExpr::FieldRef { path } => self.get_field_from_state(state, path),

            ComputedExpr::Var { name } => env
                .get(name)
                .cloned()
                .ok_or_else(|| format!("Undefined variable: {}", name).into()),

            ComputedExpr::Let { name, value, body } => {
                let val = self.evaluate_computed_expr_with_env(value, state, env)?;
                let mut new_env = env.clone();
                new_env.insert(name.clone(), val);
                self.evaluate_computed_expr_with_env(body, state, &new_env)
            }

            ComputedExpr::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let cond_val = self.evaluate_computed_expr_with_env(condition, state, env)?;
                if self.value_to_bool(&cond_val) {
                    self.evaluate_computed_expr_with_env(then_branch, state, env)
                } else {
                    self.evaluate_computed_expr_with_env(else_branch, state, env)
                }
            }

            ComputedExpr::None => Ok(Value::Null),

            ComputedExpr::Some { value } => self.evaluate_computed_expr_with_env(value, state, env),

            ComputedExpr::Slice { expr, start, end } => {
                let val = self.evaluate_computed_expr_with_env(expr, state, env)?;
                match val {
                    Value::Array(arr) => {
                        let slice: Vec<Value> = arr.get(*start..*end).unwrap_or(&[]).to_vec();
                        Ok(Value::Array(slice))
                    }
                    _ => Err(format!("Cannot slice non-array value: {:?}", val).into()),
                }
            }

            ComputedExpr::Index { expr, index } => {
                let val = self.evaluate_computed_expr_with_env(expr, state, env)?;
                match val {
                    Value::Array(arr) => Ok(arr.get(*index).cloned().unwrap_or(Value::Null)),
                    _ => Err(format!("Cannot index non-array value: {:?}", val).into()),
                }
            }

            ComputedExpr::U64FromLeBytes { bytes } => {
                let val = self.evaluate_computed_expr_with_env(bytes, state, env)?;
                let byte_vec = self.value_to_bytes(&val)?;
                if byte_vec.len() < 8 {
                    return Err(format!(
                        "u64::from_le_bytes requires 8 bytes, got {}",
                        byte_vec.len()
                    )
                    .into());
                }
                let arr: [u8; 8] = byte_vec[..8]
                    .try_into()
                    .map_err(|_| "Failed to convert to [u8; 8]")?;
                Ok(json!(u64::from_le_bytes(arr)))
            }

            ComputedExpr::U64FromBeBytes { bytes } => {
                let val = self.evaluate_computed_expr_with_env(bytes, state, env)?;
                let byte_vec = self.value_to_bytes(&val)?;
                if byte_vec.len() < 8 {
                    return Err(format!(
                        "u64::from_be_bytes requires 8 bytes, got {}",
                        byte_vec.len()
                    )
                    .into());
                }
                let arr: [u8; 8] = byte_vec[..8]
                    .try_into()
                    .map_err(|_| "Failed to convert to [u8; 8]")?;
                Ok(json!(u64::from_be_bytes(arr)))
            }

            ComputedExpr::ByteArray { bytes } => {
                Ok(Value::Array(bytes.iter().map(|b| json!(*b)).collect()))
            }

            ComputedExpr::Closure { param, body } => {
                // Closures are stored as-is; they're evaluated when used in map()
                // Return a special representation
                Ok(json!({
                    "__closure": {
                        "param": param,
                        "body": serde_json::to_value(body).unwrap_or(Value::Null)
                    }
                }))
            }

            ComputedExpr::Unary { op, expr } => {
                let val = self.evaluate_computed_expr_with_env(expr, state, env)?;
                self.apply_unary_op(op, &val)
            }

            ComputedExpr::JsonToBytes { expr } => {
                let val = self.evaluate_computed_expr_with_env(expr, state, env)?;
                // Convert JSON array of numbers to byte array
                let bytes = self.value_to_bytes(&val)?;
                Ok(Value::Array(bytes.iter().map(|b| json!(*b)).collect()))
            }

            ComputedExpr::UnwrapOr { expr, default } => {
                let val = self.evaluate_computed_expr_with_env(expr, state, env)?;
                if val.is_null() {
                    Ok(default.clone())
                } else {
                    Ok(val)
                }
            }

            ComputedExpr::Binary { op, left, right } => {
                let l = self.evaluate_computed_expr_with_env(left, state, env)?;
                let r = self.evaluate_computed_expr_with_env(right, state, env)?;
                self.apply_binary_op(op, &l, &r)
            }

            ComputedExpr::Cast { expr, to_type } => {
                let val = self.evaluate_computed_expr_with_env(expr, state, env)?;
                self.apply_cast(&val, to_type)
            }

            ComputedExpr::ResolverComputed {
                resolver,
                method,
                args,
            } => {
                let evaluated_args: Vec<Value> = args
                    .iter()
                    .map(|arg| self.evaluate_computed_expr_with_env(arg, state, env))
                    .collect::<Result<Vec<_>>>()?;
                crate::resolvers::evaluate_resolver_computed(resolver, method, &evaluated_args)
            }

            ComputedExpr::MethodCall { expr, method, args } => {
                let val = self.evaluate_computed_expr_with_env(expr, state, env)?;
                // Special handling for map() with closures
                if method == "map" && args.len() == 1 {
                    if let ComputedExpr::Closure { param, body } = &args[0] {
                        // If the value is null, return null (Option::None.map returns None)
                        if val.is_null() {
                            return Ok(Value::Null);
                        }

                        if let Value::Array(arr) = &val {
                            let results: Result<Vec<Value>> = arr
                                .iter()
                                .map(|elem| {
                                    let mut closure_env = env.clone();
                                    closure_env.insert(param.clone(), elem.clone());
                                    self.evaluate_computed_expr_with_env(body, state, &closure_env)
                                })
                                .collect();
                            return Ok(Value::Array(results?));
                        }

                        let mut closure_env = env.clone();
                        closure_env.insert(param.clone(), val);
                        return self.evaluate_computed_expr_with_env(body, state, &closure_env);
                    }
                }
                let evaluated_args: Vec<Value> = args
                    .iter()
                    .map(|a| self.evaluate_computed_expr_with_env(a, state, env))
                    .collect::<Result<Vec<_>>>()?;
                self.apply_method_call(&val, method, &evaluated_args)
            }

            ComputedExpr::Literal { value } => Ok(value.clone()),

            ComputedExpr::Paren { expr } => self.evaluate_computed_expr_with_env(expr, state, env),

            ComputedExpr::ContextSlot => Ok(self
                .current_context
                .as_ref()
                .and_then(|ctx| ctx.slot)
                .map(|s| json!(s))
                .unwrap_or(Value::Null)),

            ComputedExpr::ContextTimestamp => Ok(self
                .current_context
                .as_ref()
                .map(|ctx| json!(ctx.timestamp()))
                .unwrap_or(Value::Null)),

            ComputedExpr::Keccak256 { expr } => {
                let val = self.evaluate_computed_expr_with_env(expr, state, env)?;
                let bytes = self.value_to_bytes(&val)?;
                use sha3::{Digest, Keccak256};
                let hash = Keccak256::digest(&bytes);
                Ok(Value::Array(
                    hash.to_vec().iter().map(|b| json!(*b)).collect(),
                ))
            }
        }
    }

    /// Convert a JSON value to a byte vector
    fn value_to_bytes(&self, val: &Value) -> Result<Vec<u8>> {
        match val {
            Value::Array(arr) => arr
                .iter()
                .map(|v| {
                    v.as_u64()
                        .map(|n| n as u8)
                        .ok_or_else(|| "Array element not a valid byte".into())
                })
                .collect(),
            Value::String(s) => {
                // Try to decode as hex
                if s.starts_with("0x") || s.starts_with("0X") {
                    hex::decode(&s[2..]).map_err(|e| format!("Invalid hex string: {}", e).into())
                } else {
                    hex::decode(s).map_err(|e| format!("Invalid hex string: {}", e).into())
                }
            }
            _ => Err(format!("Cannot convert {:?} to bytes", val).into()),
        }
    }

    /// Apply a unary operation
    fn apply_unary_op(&self, op: &crate::ast::UnaryOp, val: &Value) -> Result<Value> {
        use crate::ast::UnaryOp;
        match op {
            UnaryOp::Not => Ok(json!(!self.value_to_bool(val))),
            UnaryOp::ReverseBits => match val.as_u64() {
                Some(n) => Ok(json!(n.reverse_bits())),
                None => match val.as_i64() {
                    Some(n) => Ok(json!((n as u64).reverse_bits())),
                    None => Err("reverse_bits requires an integer".into()),
                },
            },
        }
    }

    /// Get a field value from state by path (e.g., "section.field" or just "field")
    fn get_field_from_state(&self, state: &Value, path: &str) -> Result<Value> {
        let segments: Vec<&str> = path.split('.').collect();
        let mut current = state;

        for segment in segments {
            match current.get(segment) {
                Some(v) => current = v,
                None => return Ok(Value::Null),
            }
        }

        Ok(current.clone())
    }

    /// Apply a binary operation to two values
    fn apply_binary_op(&self, op: &BinaryOp, left: &Value, right: &Value) -> Result<Value> {
        match op {
            // Arithmetic operations
            BinaryOp::Add => self.numeric_op(left, right, |a, b| a + b, |a, b| a + b),
            BinaryOp::Sub => self.numeric_op(left, right, |a, b| a - b, |a, b| a - b),
            BinaryOp::Mul => self.numeric_op(left, right, |a, b| a * b, |a, b| a * b),
            BinaryOp::Div => {
                // Check for division by zero
                if let Some(r) = right.as_i64() {
                    if r == 0 {
                        return Err("Division by zero".into());
                    }
                }
                if let Some(r) = right.as_f64() {
                    if r == 0.0 {
                        return Err("Division by zero".into());
                    }
                }
                self.numeric_op(left, right, |a, b| a / b, |a, b| a / b)
            }
            BinaryOp::Mod => {
                // Modulo - only for integers
                match (left.as_i64(), right.as_i64()) {
                    (Some(a), Some(b)) if b != 0 => Ok(json!(a % b)),
                    (None, _) | (_, None) => match (left.as_u64(), right.as_u64()) {
                        (Some(a), Some(b)) if b != 0 => Ok(json!(a % b)),
                        _ => Err("Modulo requires non-zero integer operands".into()),
                    },
                    _ => Err("Modulo by zero".into()),
                }
            }

            // Comparison operations
            BinaryOp::Gt => self.comparison_op(left, right, |a, b| a > b, |a, b| a > b),
            BinaryOp::Lt => self.comparison_op(left, right, |a, b| a < b, |a, b| a < b),
            BinaryOp::Gte => self.comparison_op(left, right, |a, b| a >= b, |a, b| a >= b),
            BinaryOp::Lte => self.comparison_op(left, right, |a, b| a <= b, |a, b| a <= b),
            BinaryOp::Eq => Ok(json!(left == right)),
            BinaryOp::Ne => Ok(json!(left != right)),

            // Logical operations
            BinaryOp::And => {
                let l_bool = self.value_to_bool(left);
                let r_bool = self.value_to_bool(right);
                Ok(json!(l_bool && r_bool))
            }
            BinaryOp::Or => {
                let l_bool = self.value_to_bool(left);
                let r_bool = self.value_to_bool(right);
                Ok(json!(l_bool || r_bool))
            }

            // Bitwise operations
            BinaryOp::Xor => match (left.as_u64(), right.as_u64()) {
                (Some(a), Some(b)) => Ok(json!(a ^ b)),
                _ => match (left.as_i64(), right.as_i64()) {
                    (Some(a), Some(b)) => Ok(json!(a ^ b)),
                    _ => Err("XOR requires integer operands".into()),
                },
            },
            BinaryOp::BitAnd => match (left.as_u64(), right.as_u64()) {
                (Some(a), Some(b)) => Ok(json!(a & b)),
                _ => match (left.as_i64(), right.as_i64()) {
                    (Some(a), Some(b)) => Ok(json!(a & b)),
                    _ => Err("BitAnd requires integer operands".into()),
                },
            },
            BinaryOp::BitOr => match (left.as_u64(), right.as_u64()) {
                (Some(a), Some(b)) => Ok(json!(a | b)),
                _ => match (left.as_i64(), right.as_i64()) {
                    (Some(a), Some(b)) => Ok(json!(a | b)),
                    _ => Err("BitOr requires integer operands".into()),
                },
            },
            BinaryOp::Shl => match (left.as_u64(), right.as_u64()) {
                (Some(a), Some(b)) => Ok(json!(a << b)),
                _ => match (left.as_i64(), right.as_i64()) {
                    (Some(a), Some(b)) => Ok(json!(a << b)),
                    _ => Err("Shl requires integer operands".into()),
                },
            },
            BinaryOp::Shr => match (left.as_u64(), right.as_u64()) {
                (Some(a), Some(b)) => Ok(json!(a >> b)),
                _ => match (left.as_i64(), right.as_i64()) {
                    (Some(a), Some(b)) => Ok(json!(a >> b)),
                    _ => Err("Shr requires integer operands".into()),
                },
            },
        }
    }

    /// Helper for numeric operations that can work on integers or floats
    fn numeric_op<F1, F2>(
        &self,
        left: &Value,
        right: &Value,
        int_op: F1,
        float_op: F2,
    ) -> Result<Value>
    where
        F1: Fn(i64, i64) -> i64,
        F2: Fn(f64, f64) -> f64,
    {
        // Try i64 first
        if let (Some(a), Some(b)) = (left.as_i64(), right.as_i64()) {
            return Ok(json!(int_op(a, b)));
        }

        // Try u64
        if let (Some(a), Some(b)) = (left.as_u64(), right.as_u64()) {
            // For u64, we need to be careful with underflow in subtraction
            return Ok(json!(int_op(a as i64, b as i64)));
        }

        // Try f64
        if let (Some(a), Some(b)) = (left.as_f64(), right.as_f64()) {
            return Ok(json!(float_op(a, b)));
        }

        // If either is null, return null
        if left.is_null() || right.is_null() {
            return Ok(Value::Null);
        }

        Err(format!(
            "Cannot perform numeric operation on {:?} and {:?}",
            left, right
        )
        .into())
    }

    /// Helper for comparison operations
    fn comparison_op<F1, F2>(
        &self,
        left: &Value,
        right: &Value,
        int_cmp: F1,
        float_cmp: F2,
    ) -> Result<Value>
    where
        F1: Fn(i64, i64) -> bool,
        F2: Fn(f64, f64) -> bool,
    {
        // Try i64 first
        if let (Some(a), Some(b)) = (left.as_i64(), right.as_i64()) {
            return Ok(json!(int_cmp(a, b)));
        }

        // Try u64
        if let (Some(a), Some(b)) = (left.as_u64(), right.as_u64()) {
            return Ok(json!(int_cmp(a as i64, b as i64)));
        }

        // Try f64
        if let (Some(a), Some(b)) = (left.as_f64(), right.as_f64()) {
            return Ok(json!(float_cmp(a, b)));
        }

        // If either is null, comparison returns false
        if left.is_null() || right.is_null() {
            return Ok(json!(false));
        }

        Err(format!("Cannot compare {:?} and {:?}", left, right).into())
    }

    /// Convert a value to boolean for logical operations
    fn value_to_bool(&self, value: &Value) -> bool {
        match value {
            Value::Null => false,
            Value::Bool(b) => *b,
            Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    i != 0
                } else if let Some(f) = n.as_f64() {
                    f != 0.0
                } else {
                    true
                }
            }
            Value::String(s) => !s.is_empty(),
            Value::Array(arr) => !arr.is_empty(),
            Value::Object(obj) => !obj.is_empty(),
        }
    }

    /// Apply a type cast to a value
    fn apply_cast(&self, value: &Value, to_type: &str) -> Result<Value> {
        match to_type {
            "i8" | "i16" | "i32" | "i64" | "isize" => {
                if let Some(n) = value.as_i64() {
                    Ok(json!(n))
                } else if let Some(n) = value.as_u64() {
                    Ok(json!(n as i64))
                } else if let Some(n) = value.as_f64() {
                    Ok(json!(n as i64))
                } else if let Some(s) = value.as_str() {
                    s.parse::<i64>()
                        .map(|n| json!(n))
                        .map_err(|e| format!("Cannot parse '{}' as integer: {}", s, e).into())
                } else {
                    Err(format!("Cannot cast {:?} to {}", value, to_type).into())
                }
            }
            "u8" | "u16" | "u32" | "u64" | "usize" => {
                if let Some(n) = value.as_u64() {
                    Ok(json!(n))
                } else if let Some(n) = value.as_i64() {
                    Ok(json!(n as u64))
                } else if let Some(n) = value.as_f64() {
                    Ok(json!(n as u64))
                } else if let Some(s) = value.as_str() {
                    s.parse::<u64>().map(|n| json!(n)).map_err(|e| {
                        format!("Cannot parse '{}' as unsigned integer: {}", s, e).into()
                    })
                } else {
                    Err(format!("Cannot cast {:?} to {}", value, to_type).into())
                }
            }
            "f32" | "f64" => {
                if let Some(n) = value.as_f64() {
                    Ok(json!(n))
                } else if let Some(n) = value.as_i64() {
                    Ok(json!(n as f64))
                } else if let Some(n) = value.as_u64() {
                    Ok(json!(n as f64))
                } else if let Some(s) = value.as_str() {
                    s.parse::<f64>()
                        .map(|n| json!(n))
                        .map_err(|e| format!("Cannot parse '{}' as float: {}", s, e).into())
                } else {
                    Err(format!("Cannot cast {:?} to {}", value, to_type).into())
                }
            }
            "String" | "string" => Ok(json!(value.to_string())),
            "bool" => Ok(json!(self.value_to_bool(value))),
            _ => {
                // Unknown type, return value as-is
                Ok(value.clone())
            }
        }
    }

    /// Apply a method call to a value
    fn apply_method_call(&self, value: &Value, method: &str, args: &[Value]) -> Result<Value> {
        match method {
            "unwrap_or" => {
                if value.is_null() && !args.is_empty() {
                    Ok(args[0].clone())
                } else {
                    Ok(value.clone())
                }
            }
            "unwrap_or_default" => {
                if value.is_null() {
                    // Return default for common types
                    Ok(json!(0))
                } else {
                    Ok(value.clone())
                }
            }
            "is_some" => Ok(json!(!value.is_null())),
            "is_none" => Ok(json!(value.is_null())),
            "abs" => {
                if let Some(n) = value.as_i64() {
                    Ok(json!(n.abs()))
                } else if let Some(n) = value.as_f64() {
                    Ok(json!(n.abs()))
                } else {
                    Err(format!("Cannot call abs() on {:?}", value).into())
                }
            }
            "len" => {
                if let Some(s) = value.as_str() {
                    Ok(json!(s.len()))
                } else if let Some(arr) = value.as_array() {
                    Ok(json!(arr.len()))
                } else if let Some(obj) = value.as_object() {
                    Ok(json!(obj.len()))
                } else {
                    Err(format!("Cannot call len() on {:?}", value).into())
                }
            }
            "to_string" => Ok(json!(value.to_string())),
            "min" => {
                if args.is_empty() {
                    return Err("min() requires an argument".into());
                }
                let other = &args[0];
                if let (Some(a), Some(b)) = (value.as_i64(), other.as_i64()) {
                    Ok(json!(a.min(b)))
                } else if let (Some(a), Some(b)) = (value.as_f64(), other.as_f64()) {
                    Ok(json!(a.min(b)))
                } else {
                    Err(format!("Cannot call min() on {:?} and {:?}", value, other).into())
                }
            }
            "max" => {
                if args.is_empty() {
                    return Err("max() requires an argument".into());
                }
                let other = &args[0];
                if let (Some(a), Some(b)) = (value.as_i64(), other.as_i64()) {
                    Ok(json!(a.max(b)))
                } else if let (Some(a), Some(b)) = (value.as_f64(), other.as_f64()) {
                    Ok(json!(a.max(b)))
                } else {
                    Err(format!("Cannot call max() on {:?} and {:?}", value, other).into())
                }
            }
            "saturating_add" => {
                if args.is_empty() {
                    return Err("saturating_add() requires an argument".into());
                }
                let other = &args[0];
                if let (Some(a), Some(b)) = (value.as_i64(), other.as_i64()) {
                    Ok(json!(a.saturating_add(b)))
                } else if let (Some(a), Some(b)) = (value.as_u64(), other.as_u64()) {
                    Ok(json!(a.saturating_add(b)))
                } else {
                    Err(format!(
                        "Cannot call saturating_add() on {:?} and {:?}",
                        value, other
                    )
                    .into())
                }
            }
            "saturating_sub" => {
                if args.is_empty() {
                    return Err("saturating_sub() requires an argument".into());
                }
                let other = &args[0];
                if let (Some(a), Some(b)) = (value.as_i64(), other.as_i64()) {
                    Ok(json!(a.saturating_sub(b)))
                } else if let (Some(a), Some(b)) = (value.as_u64(), other.as_u64()) {
                    Ok(json!(a.saturating_sub(b)))
                } else {
                    Err(format!(
                        "Cannot call saturating_sub() on {:?} and {:?}",
                        value, other
                    )
                    .into())
                }
            }
            _ => Err(format!("Unknown method call: {}()", method).into()),
        }
    }

    /// Evaluate all computed fields for an entity and update the state
    /// This takes a list of ComputedFieldSpec from the AST and applies them
    pub fn evaluate_computed_fields_from_ast(
        &self,
        state: &mut Value,
        computed_field_specs: &[ComputedFieldSpec],
    ) -> Result<Vec<String>> {
        let mut updated_paths = Vec::new();

        crate::resolvers::validate_resolver_computed_specs(computed_field_specs)?;

        for spec in computed_field_specs {
            match self.evaluate_computed_expr(&spec.expression, state) {
                Ok(result) => {
                    self.set_field_in_state(state, &spec.target_path, result)?;
                    updated_paths.push(spec.target_path.clone());
                }
                Err(e) => {
                    tracing::warn!(
                        target_path = %spec.target_path,
                        error = %e,
                        "Failed to evaluate computed field"
                    );
                }
            }
        }

        Ok(updated_paths)
    }

    /// Set a field value in state by path (e.g., "section.field")
    fn set_field_in_state(&self, state: &mut Value, path: &str, value: Value) -> Result<()> {
        let segments: Vec<&str> = path.split('.').collect();

        if segments.is_empty() {
            return Err("Empty path".into());
        }

        // Navigate to parent, creating intermediate objects as needed
        let mut current = state;
        for (i, segment) in segments.iter().enumerate() {
            if i == segments.len() - 1 {
                // Last segment - set the value
                if let Some(obj) = current.as_object_mut() {
                    obj.insert(segment.to_string(), value);
                    return Ok(());
                } else {
                    return Err(format!("Cannot set field '{}' on non-object", segment).into());
                }
            } else {
                // Intermediate segment - navigate or create
                if !current.is_object() {
                    *current = json!({});
                }
                let obj = current.as_object_mut().unwrap();
                current = obj.entry(segment.to_string()).or_insert_with(|| json!({}));
            }
        }

        Ok(())
    }

    /// Create a computed fields evaluator closure from AST specs
    /// This returns a function that can be passed to the bytecode builder
    pub fn create_evaluator_from_specs(
        specs: Vec<ComputedFieldSpec>,
    ) -> impl Fn(&mut Value, Option<u64>, i64) -> Result<()> + Send + Sync + 'static {
        move |state: &mut Value, context_slot: Option<u64>, context_timestamp: i64| {
            let mut vm = VmContext::new();
            vm.current_context = Some(UpdateContext {
                slot: context_slot,
                timestamp: Some(context_timestamp),
                ..Default::default()
            });
            vm.evaluate_computed_fields_from_ast(state, &specs)?;
            Ok(())
        }
    }
}

impl Default for VmContext {
    fn default() -> Self {
        Self::new()
    }
}

// Implement the ReverseLookupUpdater trait for VmContext
impl crate::resolvers::ReverseLookupUpdater for VmContext {
    fn update(&mut self, pda_address: String, seed_value: String) -> Vec<PendingAccountUpdate> {
        // Use default state_id=0 and default lookup name
        self.update_pda_reverse_lookup(0, "default_pda_lookup", pda_address, seed_value)
            .unwrap_or_else(|e| {
                tracing::error!("Failed to update PDA reverse lookup: {}", e);
                Vec::new()
            })
    }

    fn flush_pending(&mut self, pda_address: &str) -> Vec<PendingAccountUpdate> {
        // Flush is handled inside update_pda_reverse_lookup, but we can also call it directly
        self.flush_pending_updates(0, pda_address)
            .unwrap_or_else(|e| {
                tracing::error!("Failed to flush pending updates: {}", e);
                Vec::new()
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{BinaryOp, ComputedExpr, ComputedFieldSpec};

    #[test]
    fn test_computed_field_preserves_integer_type() {
        let vm = VmContext::new();

        let mut state = serde_json::json!({
            "trading": {
                "total_buy_volume": 20000000000_i64,
                "total_sell_volume": 17951316474_i64
            }
        });

        let spec = ComputedFieldSpec {
            target_path: "trading.total_volume".to_string(),
            result_type: "Option<u64>".to_string(),
            expression: ComputedExpr::Binary {
                op: BinaryOp::Add,
                left: Box::new(ComputedExpr::UnwrapOr {
                    expr: Box::new(ComputedExpr::FieldRef {
                        path: "trading.total_buy_volume".to_string(),
                    }),
                    default: serde_json::json!(0),
                }),
                right: Box::new(ComputedExpr::UnwrapOr {
                    expr: Box::new(ComputedExpr::FieldRef {
                        path: "trading.total_sell_volume".to_string(),
                    }),
                    default: serde_json::json!(0),
                }),
            },
        };

        vm.evaluate_computed_fields_from_ast(&mut state, &[spec])
            .unwrap();

        let total_volume = state
            .get("trading")
            .and_then(|t| t.get("total_volume"))
            .expect("total_volume should exist");

        let serialized = serde_json::to_string(total_volume).unwrap();
        assert!(
            !serialized.contains('.'),
            "Integer should not have decimal point: {}",
            serialized
        );
        assert_eq!(
            total_volume.as_i64(),
            Some(37951316474),
            "Value should be correct sum"
        );
    }

    #[test]
    fn test_set_field_sum_preserves_integer_type() {
        let mut vm = VmContext::new();
        vm.registers[0] = serde_json::json!({});
        vm.registers[1] = serde_json::json!(20000000000_i64);
        vm.registers[2] = serde_json::json!(17951316474_i64);

        vm.set_field_sum(0, "trading.total_buy_volume", 1).unwrap();
        vm.set_field_sum(0, "trading.total_sell_volume", 2).unwrap();

        let state = &vm.registers[0];
        let buy_vol = state
            .get("trading")
            .and_then(|t| t.get("total_buy_volume"))
            .unwrap();
        let sell_vol = state
            .get("trading")
            .and_then(|t| t.get("total_sell_volume"))
            .unwrap();

        let buy_serialized = serde_json::to_string(buy_vol).unwrap();
        let sell_serialized = serde_json::to_string(sell_vol).unwrap();

        assert!(
            !buy_serialized.contains('.'),
            "Buy volume should not have decimal: {}",
            buy_serialized
        );
        assert!(
            !sell_serialized.contains('.'),
            "Sell volume should not have decimal: {}",
            sell_serialized
        );
    }

    #[test]
    fn test_lookup_index_chaining() {
        let mut vm = VmContext::new();

        let state = vm.states.get_mut(&0).unwrap();

        state
            .pda_reverse_lookups
            .entry("default_pda_lookup".to_string())
            .or_insert_with(|| PdaReverseLookup::new(1000))
            .insert("pda_123".to_string(), "addr_456".to_string());

        state
            .lookup_indexes
            .entry("round_address_lookup_index".to_string())
            .or_insert_with(LookupIndex::new)
            .insert(json!("addr_456"), json!(789));

        let handler = vec![
            OpCode::LoadConstant {
                value: json!("pda_123"),
                dest: 0,
            },
            OpCode::LookupIndex {
                state_id: 0,
                index_name: "round_address_lookup_index".to_string(),
                lookup_value: 0,
                dest: 1,
            },
        ];

        vm.execute_handler(&handler, &json!({}), "test", 0, "TestEntity", None, None)
            .unwrap();

        assert_eq!(vm.registers[1], json!(789));
    }

    #[test]
    fn test_lookup_index_no_chain() {
        let mut vm = VmContext::new();

        let state = vm.states.get_mut(&0).unwrap();
        state
            .lookup_indexes
            .entry("test_index".to_string())
            .or_insert_with(LookupIndex::new)
            .insert(json!("key_abc"), json!(42));

        let handler = vec![
            OpCode::LoadConstant {
                value: json!("key_abc"),
                dest: 0,
            },
            OpCode::LookupIndex {
                state_id: 0,
                index_name: "test_index".to_string(),
                lookup_value: 0,
                dest: 1,
            },
        ];

        vm.execute_handler(&handler, &json!({}), "test", 0, "TestEntity", None, None)
            .unwrap();

        assert_eq!(vm.registers[1], json!(42));
    }

    #[test]
    fn test_conditional_set_field_with_zero_array() {
        let mut vm = VmContext::new();

        let event_zeros = json!({
            "value": [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                      0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
        });

        let event_nonzero = json!({
            "value": [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16,
                      17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32]
        });

        let zero_32: Value = json!([
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0
        ]);

        let handler = vec![
            OpCode::CreateObject { dest: 2 },
            OpCode::LoadEventField {
                path: FieldPath::new(&["value"]),
                dest: 10,
                default: None,
            },
            OpCode::ConditionalSetField {
                object: 2,
                path: "captured_value".to_string(),
                value: 10,
                condition_field: FieldPath::new(&["value"]),
                condition_op: ComparisonOp::NotEqual,
                condition_value: zero_32,
            },
        ];

        vm.execute_handler(&handler, &event_zeros, "test", 0, "Test", None, None)
            .unwrap();
        assert!(
            vm.registers[2].get("captured_value").is_none(),
            "Field should not be set when value is all zeros"
        );

        vm.reset_registers();
        vm.execute_handler(&handler, &event_nonzero, "test", 0, "Test", None, None)
            .unwrap();
        assert!(
            vm.registers[2].get("captured_value").is_some(),
            "Field should be set when value is non-zero"
        );
    }

    #[test]
    fn test_when_instruction_arrives_first() {
        let mut vm = VmContext::new();

        let signature = "test_sig_123".to_string();

        {
            let state = vm.states.get(&0).unwrap();
            let mut cache = state.recent_tx_instructions.lock().unwrap();
            let mut set = HashSet::new();
            set.insert("RevealIxState".to_string());
            cache.put(signature.clone(), set);
        }

        vm.current_context = Some(UpdateContext::new(100, signature.clone()));

        let handler = vec![
            OpCode::CreateObject { dest: 2 },
            OpCode::LoadConstant {
                value: json!("primary_key_value"),
                dest: 1,
            },
            OpCode::LoadConstant {
                value: json!("the_revealed_value"),
                dest: 10,
            },
            OpCode::SetFieldWhen {
                object: 2,
                path: "entropy_value".to_string(),
                value: 10,
                when_instruction: "RevealIxState".to_string(),
                entity_name: "TestEntity".to_string(),
                key_reg: 1,
                condition_field: None,
                condition_op: None,
                condition_value: None,
            },
        ];

        vm.execute_handler(
            &handler,
            &json!({}),
            "VarState",
            0,
            "TestEntity",
            None,
            None,
        )
        .unwrap();

        assert_eq!(
            vm.registers[2].get("entropy_value").unwrap(),
            "the_revealed_value",
            "Field should be set when instruction was already seen"
        );
    }

    #[test]
    fn test_when_account_arrives_first() {
        let mut vm = VmContext::new();

        let signature = "test_sig_456".to_string();

        vm.current_context = Some(UpdateContext::new(100, signature.clone()));

        let handler = vec![
            OpCode::CreateObject { dest: 2 },
            OpCode::LoadConstant {
                value: json!("pk_123"),
                dest: 1,
            },
            OpCode::LoadConstant {
                value: json!("deferred_value"),
                dest: 10,
            },
            OpCode::SetFieldWhen {
                object: 2,
                path: "entropy_value".to_string(),
                value: 10,
                when_instruction: "RevealIxState".to_string(),
                entity_name: "TestEntity".to_string(),
                key_reg: 1,
                condition_field: None,
                condition_op: None,
                condition_value: None,
            },
        ];

        vm.execute_handler(
            &handler,
            &json!({}),
            "VarState",
            0,
            "TestEntity",
            None,
            None,
        )
        .unwrap();

        assert!(
            vm.registers[2].get("entropy_value").is_none(),
            "Field should not be set when instruction hasn't been seen"
        );

        let state = vm.states.get(&0).unwrap();
        let key = (signature.clone(), "RevealIxState".to_string());
        assert!(
            state.deferred_when_ops.contains_key(&key),
            "Operation should be queued"
        );

        {
            let mut cache = state.recent_tx_instructions.lock().unwrap();
            let mut set = HashSet::new();
            set.insert("RevealIxState".to_string());
            cache.put(signature.clone(), set);
        }

        let deferred = state.deferred_when_ops.remove(&key).unwrap().1;
        for op in deferred {
            vm.apply_deferred_when_op(0, &op, None, None).unwrap();
        }

        let state = vm.states.get(&0).unwrap();
        let entity = state.data.get(&json!("pk_123")).unwrap();
        assert_eq!(
            entity.get("entropy_value").unwrap(),
            "deferred_value",
            "Field should be set after instruction arrives"
        );
    }

    #[test]
    fn test_when_cleanup_expired() {
        let mut vm = VmContext::new();

        let state = vm.states.get(&0).unwrap();
        let key = ("old_sig".to_string(), "SomeIxState".to_string());
        state.deferred_when_ops.insert(
            key,
            vec![DeferredWhenOperation {
                entity_name: "Test".to_string(),
                primary_key: json!("pk"),
                field_path: "field".to_string(),
                field_value: json!("value"),
                when_instruction: "SomeIxState".to_string(),
                signature: "old_sig".to_string(),
                slot: 0,
                deferred_at: 0,
                emit: true,
            }],
        );

        let removed = vm.cleanup_expired_when_ops(0, 60);

        assert_eq!(removed, 1, "Should have removed 1 expired op");
        assert!(
            vm.states.get(&0).unwrap().deferred_when_ops.is_empty(),
            "Deferred ops should be empty after cleanup"
        );
    }

    #[test]
    fn test_deferred_when_op_recomputes_dependent_fields() {
        use crate::ast::{BinaryOp, ComputedExpr, ComputedFieldSpec};

        let mut vm = VmContext::new();

        // Create computed field specs similar to the ore stack:
        // pre_reveal_rng depends on base_value
        // pre_reveal_winning_square depends on pre_reveal_rng
        let computed_specs = vec![
            ComputedFieldSpec {
                target_path: "results.pre_reveal_rng".to_string(),
                result_type: "Option<u64>".to_string(),
                expression: ComputedExpr::FieldRef {
                    path: "entropy.base_value".to_string(),
                },
            },
            ComputedFieldSpec {
                target_path: "results.pre_reveal_winning_square".to_string(),
                result_type: "Option<u64>".to_string(),
                expression: ComputedExpr::MethodCall {
                    expr: Box::new(ComputedExpr::FieldRef {
                        path: "results.pre_reveal_rng".to_string(),
                    }),
                    method: "map".to_string(),
                    args: vec![ComputedExpr::Closure {
                        param: "r".to_string(),
                        body: Box::new(ComputedExpr::Binary {
                            op: BinaryOp::Mod,
                            left: Box::new(ComputedExpr::Var {
                                name: "r".to_string(),
                            }),
                            right: Box::new(ComputedExpr::Literal {
                                value: serde_json::json!(25),
                            }),
                        }),
                    }],
                },
            },
        ];

        let evaluator: Box<dyn Fn(&mut Value, Option<u64>, i64) -> Result<()> + Send + Sync> =
            Box::new(VmContext::create_evaluator_from_specs(computed_specs));

        // Test that when we set entropy.base_value via deferred when-op,
        // both computed fields are updated
        let primary_key = json!("test_pk");
        let op = DeferredWhenOperation {
            entity_name: "TestEntity".to_string(),
            primary_key: primary_key.clone(),
            field_path: "entropy.base_value".to_string(),
            field_value: json!(100),
            when_instruction: "TestIxState".to_string(),
            signature: "test_sig".to_string(),
            slot: 100,
            deferred_at: 0,
            emit: true,
        };

        // Store the entity in state first
        let initial_state = json!({
            "results": {}
        });
        vm.states
            .get(&0)
            .unwrap()
            .insert_with_eviction(primary_key.clone(), initial_state);

        // Apply the deferred when-op with the evaluator
        let mutations = vm
            .apply_deferred_when_op(
                0,
                &op,
                Some(&evaluator),
                Some(&[
                    "results.pre_reveal_rng".to_string(),
                    "results.pre_reveal_winning_square".to_string(),
                ]),
            )
            .unwrap();

        // Check the entity state
        let state = vm.states.get(&0).unwrap();
        let entity = state.data.get(&primary_key).unwrap();

        println!(
            "Entity state: {}",
            serde_json::to_string_pretty(&*entity).unwrap()
        );

        // Verify base value was set
        assert_eq!(
            entity.get("entropy").and_then(|e| e.get("base_value")),
            Some(&json!(100)),
            "Base value should be set"
        );

        // Verify computed fields were calculated
        let pre_reveal_rng = entity
            .get("results")
            .and_then(|r| r.get("pre_reveal_rng"))
            .cloned();
        let pre_reveal_winning_square = entity
            .get("results")
            .and_then(|r| r.get("pre_reveal_winning_square"))
            .cloned();

        assert_eq!(
            pre_reveal_rng,
            Some(json!(100)),
            "pre_reveal_rng should be computed"
        );
        assert_eq!(
            pre_reveal_winning_square,
            Some(json!(0)),
            "pre_reveal_winning_square should be 100 % 25 = 0"
        );

        // Verify mutations include computed fields
        assert!(!mutations.is_empty(), "Should have mutations");
        let mutation = &mutations[0];
        let patch = &mutation.patch;

        assert!(
            patch
                .get("results")
                .and_then(|r| r.get("pre_reveal_rng"))
                .is_some(),
            "Mutation should include pre_reveal_rng"
        );
        assert!(
            patch
                .get("results")
                .and_then(|r| r.get("pre_reveal_winning_square"))
                .is_some(),
            "Mutation should include pre_reveal_winning_square"
        );
    }
}
