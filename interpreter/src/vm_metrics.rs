//! VM metrics for OpenTelemetry integration.
//!
//! This module provides metrics recording functions that are always available.
//! When the `otel` feature is disabled, all functions are no-ops.
//! When enabled, metrics are recorded via OpenTelemetry.

#[cfg(feature = "otel")]
use opentelemetry::{
    global,
    metrics::{Counter, Gauge, Histogram},
    KeyValue,
};
#[cfg(feature = "otel")]
use std::sync::OnceLock;

#[cfg(feature = "otel")]
static VM_METRICS: OnceLock<VmMetrics> = OnceLock::new();

#[cfg(feature = "otel")]
pub struct VmMetrics {
    pub state_table_entries: Gauge<i64>,
    pub state_table_capacity: Gauge<i64>,
    pub lookup_index_count: Gauge<i64>,
    pub lookup_index_entries: Gauge<i64>,
    pub temporal_index_count: Gauge<i64>,
    pub temporal_index_entries: Gauge<i64>,
    pub pda_reverse_lookup_count: Gauge<i64>,
    pub pda_reverse_lookup_entries: Gauge<i64>,
    pub version_tracker_entries: Gauge<i64>,
    pub path_cache_size: Gauge<i64>,
    pub pending_queue_updates: Gauge<i64>,
    pub pending_queue_unique_pdas: Gauge<i64>,
    pub pending_queue_memory_bytes: Gauge<i64>,
    pub pending_queue_oldest_age: Histogram<f64>,
    pub state_table_evictions: Counter<u64>,
    pub state_table_at_capacity_events: Counter<u64>,
    pub cleanup_pending_removed: Counter<u64>,
    pub cleanup_temporal_removed: Counter<u64>,
    pub path_cache_hits: Counter<u64>,
    pub path_cache_misses: Counter<u64>,
    pub lookup_index_hits: Counter<u64>,
    pub lookup_index_misses: Counter<u64>,
    pub pending_updates_queued: Counter<u64>,
    pub pending_updates_flushed: Counter<u64>,
    pub pending_updates_expired: Counter<u64>,
}

#[cfg(feature = "otel")]
impl VmMetrics {
    fn new() -> Self {
        let meter = global::meter("arete-interpreter");

        Self {
            state_table_entries: meter
                .i64_gauge("arete.vm.state_table.entries")
                .with_description("Current entries in the state table")
                .init(),
            state_table_capacity: meter
                .i64_gauge("arete.vm.state_table.capacity")
                .with_description("Maximum capacity of the state table")
                .init(),
            lookup_index_count: meter
                .i64_gauge("arete.vm.lookup_index.count")
                .with_description("Number of lookup indexes")
                .init(),
            lookup_index_entries: meter
                .i64_gauge("arete.vm.lookup_index.entries")
                .with_description("Total entries across lookup indexes")
                .init(),
            temporal_index_count: meter
                .i64_gauge("arete.vm.temporal_index.count")
                .with_description("Number of temporal indexes")
                .init(),
            temporal_index_entries: meter
                .i64_gauge("arete.vm.temporal_index.entries")
                .with_description("Total entries across temporal indexes")
                .init(),
            pda_reverse_lookup_count: meter
                .i64_gauge("arete.vm.pda_reverse_lookup.count")
                .with_description("Number of PDA reverse lookup tables")
                .init(),
            pda_reverse_lookup_entries: meter
                .i64_gauge("arete.vm.pda_reverse_lookup.entries")
                .with_description("Total entries across PDA reverse lookups")
                .init(),
            version_tracker_entries: meter
                .i64_gauge("arete.vm.version_tracker.entries")
                .with_description("Entries in the version tracker")
                .init(),
            path_cache_size: meter
                .i64_gauge("arete.vm.path_cache.size")
                .with_description("Size of the compiled path cache")
                .init(),
            pending_queue_updates: meter
                .i64_gauge("arete.vm.pending_queue.updates")
                .with_description("Total pending updates in queue")
                .init(),
            pending_queue_unique_pdas: meter
                .i64_gauge("arete.vm.pending_queue.unique_pdas")
                .with_description("Unique PDAs with pending updates")
                .init(),
            pending_queue_memory_bytes: meter
                .i64_gauge("arete.vm.pending_queue.memory_bytes")
                .with_description("Estimated memory usage of pending queue")
                .init(),
            pending_queue_oldest_age: meter
                .f64_histogram("arete.vm.pending_queue.oldest_age_seconds")
                .with_description("Age of oldest pending update in seconds")
                .init(),
            state_table_evictions: meter
                .u64_counter("arete.vm.state_table.evictions")
                .with_description("State table LRU evictions")
                .init(),
            state_table_at_capacity_events: meter
                .u64_counter("arete.vm.state_table.at_capacity_events")
                .with_description("State table at capacity events")
                .init(),
            cleanup_pending_removed: meter
                .u64_counter("arete.vm.cleanup.pending_removed")
                .with_description("Pending updates removed during cleanup")
                .init(),
            cleanup_temporal_removed: meter
                .u64_counter("arete.vm.cleanup.temporal_removed")
                .with_description("Temporal entries removed during cleanup")
                .init(),
            path_cache_hits: meter
                .u64_counter("arete.vm.path_cache.hits")
                .with_description("Path cache hits")
                .init(),
            path_cache_misses: meter
                .u64_counter("arete.vm.path_cache.misses")
                .with_description("Path cache misses")
                .init(),
            lookup_index_hits: meter
                .u64_counter("arete.vm.lookup_index.hits")
                .with_description("Lookup index hits")
                .init(),
            lookup_index_misses: meter
                .u64_counter("arete.vm.lookup_index.misses")
                .with_description("Lookup index misses")
                .init(),
            pending_updates_queued: meter
                .u64_counter("arete.vm.pending_updates.queued")
                .with_description("Updates queued for later processing")
                .init(),
            pending_updates_flushed: meter
                .u64_counter("arete.vm.pending_updates.flushed")
                .with_description("Queued updates flushed after PDA resolution")
                .init(),
            pending_updates_expired: meter
                .u64_counter("arete.vm.pending_updates.expired")
                .with_description("Queued updates that expired")
                .init(),
        }
    }
}

#[cfg(feature = "otel")]
pub fn get_vm_metrics() -> &'static VmMetrics {
    VM_METRICS.get_or_init(VmMetrics::new)
}

#[cfg(feature = "otel")]
pub fn record_state_table_eviction(count: u64, entity: &str) {
    get_vm_metrics()
        .state_table_evictions
        .add(count, &[KeyValue::new("entity", entity.to_string())]);
}

#[cfg(not(feature = "otel"))]
#[inline]
pub fn record_state_table_eviction(_count: u64, _entity: &str) {}

#[cfg(feature = "otel")]
pub fn record_state_table_at_capacity(entity: &str) {
    get_vm_metrics()
        .state_table_at_capacity_events
        .add(1, &[KeyValue::new("entity", entity.to_string())]);
}

#[cfg(not(feature = "otel"))]
#[inline]
pub fn record_state_table_at_capacity(_entity: &str) {}

#[cfg(feature = "otel")]
pub fn record_cleanup(pending_removed: usize, temporal_removed: usize, entity: &str) {
    let m = get_vm_metrics();
    let attrs = &[KeyValue::new("entity", entity.to_string())];
    m.cleanup_pending_removed.add(pending_removed as u64, attrs);
    m.cleanup_temporal_removed
        .add(temporal_removed as u64, attrs);
}

#[cfg(not(feature = "otel"))]
#[inline]
pub fn record_cleanup(_pending_removed: usize, _temporal_removed: usize, _entity: &str) {}

#[cfg(feature = "otel")]
pub fn record_path_cache_hit() {
    get_vm_metrics().path_cache_hits.add(1, &[]);
}

#[cfg(not(feature = "otel"))]
#[inline]
pub fn record_path_cache_hit() {}

#[cfg(feature = "otel")]
pub fn record_path_cache_miss() {
    get_vm_metrics().path_cache_misses.add(1, &[]);
}

#[cfg(not(feature = "otel"))]
#[inline]
pub fn record_path_cache_miss() {}

#[cfg(feature = "otel")]
pub fn record_lookup_index_hit(index_name: &str) {
    get_vm_metrics()
        .lookup_index_hits
        .add(1, &[KeyValue::new("index", index_name.to_string())]);
}

#[cfg(not(feature = "otel"))]
#[inline]
pub fn record_lookup_index_hit(_index_name: &str) {}

#[cfg(feature = "otel")]
pub fn record_lookup_index_miss(index_name: &str) {
    get_vm_metrics()
        .lookup_index_misses
        .add(1, &[KeyValue::new("index", index_name.to_string())]);
}

#[cfg(not(feature = "otel"))]
#[inline]
pub fn record_lookup_index_miss(_index_name: &str) {}

#[cfg(feature = "otel")]
pub fn record_pending_update_queued(entity: &str) {
    get_vm_metrics()
        .pending_updates_queued
        .add(1, &[KeyValue::new("entity", entity.to_string())]);
}

#[cfg(not(feature = "otel"))]
#[inline]
pub fn record_pending_update_queued(_entity: &str) {}

#[cfg(feature = "otel")]
pub fn record_pending_updates_flushed(count: u64, entity: &str) {
    get_vm_metrics()
        .pending_updates_flushed
        .add(count, &[KeyValue::new("entity", entity.to_string())]);
}

#[cfg(not(feature = "otel"))]
#[inline]
pub fn record_pending_updates_flushed(_count: u64, _entity: &str) {}

#[cfg(feature = "otel")]
pub fn record_pending_updates_expired(count: u64, entity: &str) {
    get_vm_metrics()
        .pending_updates_expired
        .add(count, &[KeyValue::new("entity", entity.to_string())]);
}

#[cfg(not(feature = "otel"))]
#[inline]
pub fn record_pending_updates_expired(_count: u64, _entity: &str) {}

#[cfg(feature = "otel")]
pub fn record_memory_stats(stats: &crate::vm::VmMemoryStats, entity: &str) {
    let m = get_vm_metrics();
    let attrs = &[KeyValue::new("entity", entity.to_string())];

    m.state_table_entries
        .record(stats.state_table_entity_count as i64, attrs);
    m.state_table_capacity
        .record(stats.state_table_max_entries as i64, attrs);
    m.lookup_index_count
        .record(stats.lookup_index_count as i64, attrs);
    m.lookup_index_entries
        .record(stats.lookup_index_total_entries as i64, attrs);
    m.temporal_index_count
        .record(stats.temporal_index_count as i64, attrs);
    m.temporal_index_entries
        .record(stats.temporal_index_total_entries as i64, attrs);
    m.pda_reverse_lookup_count
        .record(stats.pda_reverse_lookup_count as i64, attrs);
    m.pda_reverse_lookup_entries
        .record(stats.pda_reverse_lookup_total_entries as i64, attrs);
    m.version_tracker_entries
        .record(stats.version_tracker_entries as i64, attrs);
    m.path_cache_size
        .record(stats.path_cache_size as i64, attrs);

    if let Some(ref pq) = stats.pending_queue_stats {
        m.pending_queue_updates
            .record(pq.total_updates as i64, attrs);
        m.pending_queue_unique_pdas
            .record(pq.unique_pdas as i64, attrs);
        m.pending_queue_memory_bytes
            .record(pq.estimated_memory_bytes as i64, attrs);
        m.pending_queue_oldest_age
            .record(pq.oldest_age_seconds as f64, attrs);
    }
}

#[cfg(not(feature = "otel"))]
#[inline]
pub fn record_memory_stats(_stats: &crate::vm::VmMemoryStats, _entity: &str) {}
