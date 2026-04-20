//! OpenTelemetry metrics for Arete Server
//!
//! This module provides centralized metrics collection for monitoring
//! server health, performance, and business metrics.
//!
//! # Metric Categories
//!
//! ## Engineering Metrics
//! - WebSocket connection metrics (active, total, duration)
//! - Projector throughput (mutations processed, frames published)
//! - Stream health (status, events received, errors)
//!
//! ## Business Metrics
//! - Active subscriptions by view
//! - Events processed by program/type
//!
//! # Usage
//!
//! Initialize metrics once at server startup:
//! ```ignore
//! let metrics = Metrics::new("my_service_name");
//! ```
//!
//! Then pass the metrics instance to components that need it.

use opentelemetry::{
    global,
    metrics::{Counter, Gauge, Histogram, Meter, UpDownCounter},
    KeyValue,
};

use std::time::Instant;

/// Central metrics container for Arete server components
#[derive(Clone)]
pub struct Metrics {
    #[allow(dead_code)]
    meter: Meter,

    // WebSocket metrics
    pub ws_connections_total: Counter<u64>,
    pub ws_connections_active: UpDownCounter<i64>,
    pub ws_messages_received: Counter<u64>,
    pub ws_messages_sent: Counter<u64>,
    pub ws_connection_duration: Histogram<f64>,
    pub ws_subscriptions_active: UpDownCounter<i64>,

    // Projector metrics
    pub projector_mutations_processed: Counter<u64>,
    pub projector_frames_published: Counter<u64>,
    pub projector_processing_latency: Histogram<f64>,

    // Stream/Parser metrics
    pub stream_events_received: Counter<u64>,
    pub stream_errors_total: Counter<u64>,

    // VM metrics (for interpreter)
    pub vm_instructions_executed: Counter<u64>,
    pub vm_events_processed: Counter<u64>,
    pub vm_event_processing_duration: Histogram<f64>,
    pub vm_mutations_emitted: Counter<u64>,
    pub vm_pda_cache_hits: Counter<u64>,
    pub vm_pda_cache_misses: Counter<u64>,
    pub vm_pending_queue_size: UpDownCounter<i64>,

    // Entity metrics (business)
    pub entities_active: UpDownCounter<i64>,

    // Interpreter cache gauges (updated periodically from VmMemoryStats)
    pub vm_state_table_entries: Gauge<i64>,
    pub vm_state_table_capacity: Gauge<i64>,
    pub vm_lookup_index_count: Gauge<i64>,
    pub vm_lookup_index_entries: Gauge<i64>,
    pub vm_temporal_index_count: Gauge<i64>,
    pub vm_temporal_index_entries: Gauge<i64>,
    pub vm_pda_reverse_lookup_count: Gauge<i64>,
    pub vm_pda_reverse_lookup_entries: Gauge<i64>,
    pub vm_version_tracker_entries: Gauge<i64>,
    pub vm_path_cache_size: Gauge<i64>,
    pub vm_pending_queue_updates: Gauge<i64>,
    pub vm_pending_queue_unique_pdas: Gauge<i64>,
    pub vm_pending_queue_memory_bytes: Gauge<i64>,
    pub vm_pending_queue_oldest_age: Histogram<f64>,

    // Interpreter event counters (recorded inline)
    pub vm_state_table_evictions: Counter<u64>,
    pub vm_state_table_at_capacity_events: Counter<u64>,
    pub vm_cleanup_pending_removed: Counter<u64>,
    pub vm_cleanup_temporal_removed: Counter<u64>,
    pub vm_path_cache_hits: Counter<u64>,
    pub vm_path_cache_misses: Counter<u64>,
    pub vm_lookup_index_hits: Counter<u64>,
    pub vm_lookup_index_misses: Counter<u64>,
    pub vm_pending_updates_queued: Counter<u64>,
    pub vm_pending_updates_flushed: Counter<u64>,
    pub vm_pending_updates_expired: Counter<u64>,
}

impl Metrics {
    /// Create a new Metrics instance with the given service name
    ///
    /// The service name is used as the meter name for all metrics.
    pub fn new(service_name: impl Into<std::borrow::Cow<'static, str>>) -> Self {
        let meter = global::meter(service_name);

        // WebSocket metrics
        let ws_connections_total = meter
            .u64_counter("arete.ws.connections.total")
            .with_description("Total number of WebSocket connections")
            .init();

        let ws_connections_active = meter
            .i64_up_down_counter("arete.ws.connections.active")
            .with_description("Number of active WebSocket connections")
            .init();

        let ws_messages_received = meter
            .u64_counter("arete.ws.messages.received")
            .with_description("Total WebSocket messages received from clients")
            .init();

        let ws_messages_sent = meter
            .u64_counter("arete.ws.messages.sent")
            .with_description("Total WebSocket messages sent to clients")
            .init();

        let ws_connection_duration = meter
            .f64_histogram("arete.ws.connection.duration")
            .with_description("Duration of WebSocket connections in seconds")
            .init();

        let ws_subscriptions_active = meter
            .i64_up_down_counter("arete.ws.subscriptions.active")
            .with_description("Number of active subscriptions by view")
            .init();

        // Projector metrics
        let projector_mutations_processed = meter
            .u64_counter("arete.projector.mutations.processed")
            .with_description("Total mutations processed by the projector")
            .init();

        let projector_frames_published = meter
            .u64_counter("arete.projector.frames.published")
            .with_description("Total frames published by mode")
            .init();

        let projector_processing_latency = meter
            .f64_histogram("arete.projector.latency")
            .with_description("Latency of mutation processing in milliseconds")
            .init();

        // Stream metrics
        let stream_events_received = meter
            .u64_counter("arete.stream.events.received")
            .with_description("Total events received from the stream")
            .init();

        let stream_errors_total = meter
            .u64_counter("arete.stream.errors.total")
            .with_description("Total stream errors")
            .init();

        // VM metrics
        let vm_instructions_executed = meter
            .u64_counter("arete.vm.instructions.executed")
            .with_description("Total VM instructions executed")
            .init();

        let vm_events_processed = meter
            .u64_counter("arete.vm.events.processed")
            .with_description("Total events processed by the VM")
            .init();

        let vm_event_processing_duration = meter
            .f64_histogram("arete.vm.event.duration")
            .with_description("Duration of event processing in the VM in milliseconds")
            .init();

        let vm_mutations_emitted = meter
            .u64_counter("arete.vm.mutations.emitted")
            .with_description("Total mutations emitted by the VM")
            .init();

        let vm_pda_cache_hits = meter
            .u64_counter("arete.vm.pda_cache.hits")
            .with_description("PDA reverse lookup cache hits")
            .init();

        let vm_pda_cache_misses = meter
            .u64_counter("arete.vm.pda_cache.misses")
            .with_description("PDA reverse lookup cache misses")
            .init();

        let vm_pending_queue_size = meter
            .i64_up_down_counter("arete.vm.pending_queue.size")
            .with_description("Size of the pending account updates queue")
            .init();

        // Business metrics
        let entities_active = meter
            .i64_up_down_counter("arete.entities.active")
            .with_description("Number of active entities being tracked")
            .init();

        // Interpreter cache gauges
        let vm_state_table_entries = meter
            .i64_gauge("arete.vm.state_table.entries")
            .with_description("Current entries in the state table")
            .init();

        let vm_state_table_capacity = meter
            .i64_gauge("arete.vm.state_table.capacity")
            .with_description("Maximum capacity of the state table")
            .init();

        let vm_lookup_index_count = meter
            .i64_gauge("arete.vm.lookup_index.count")
            .with_description("Number of lookup indexes")
            .init();

        let vm_lookup_index_entries = meter
            .i64_gauge("arete.vm.lookup_index.entries")
            .with_description("Total entries across lookup indexes")
            .init();

        let vm_temporal_index_count = meter
            .i64_gauge("arete.vm.temporal_index.count")
            .with_description("Number of temporal indexes")
            .init();

        let vm_temporal_index_entries = meter
            .i64_gauge("arete.vm.temporal_index.entries")
            .with_description("Total entries across temporal indexes")
            .init();

        let vm_pda_reverse_lookup_count = meter
            .i64_gauge("arete.vm.pda_reverse_lookup.count")
            .with_description("Number of PDA reverse lookup tables")
            .init();

        let vm_pda_reverse_lookup_entries = meter
            .i64_gauge("arete.vm.pda_reverse_lookup.entries")
            .with_description("Total entries across PDA reverse lookups")
            .init();

        let vm_version_tracker_entries = meter
            .i64_gauge("arete.vm.version_tracker.entries")
            .with_description("Entries in the version tracker")
            .init();

        let vm_path_cache_size = meter
            .i64_gauge("arete.vm.path_cache.size")
            .with_description("Size of the compiled path cache")
            .init();

        let vm_pending_queue_updates = meter
            .i64_gauge("arete.vm.pending_queue.updates")
            .with_description("Total pending updates in queue")
            .init();

        let vm_pending_queue_unique_pdas = meter
            .i64_gauge("arete.vm.pending_queue.unique_pdas")
            .with_description("Unique PDAs with pending updates")
            .init();

        let vm_pending_queue_memory_bytes = meter
            .i64_gauge("arete.vm.pending_queue.memory_bytes")
            .with_description("Estimated memory usage of pending queue")
            .init();

        let vm_pending_queue_oldest_age = meter
            .f64_histogram("arete.vm.pending_queue.oldest_age_seconds")
            .with_description("Age of oldest pending update in seconds")
            .init();

        // Interpreter event counters
        let vm_state_table_evictions = meter
            .u64_counter("arete.vm.state_table.evictions")
            .with_description("State table LRU evictions")
            .init();

        let vm_state_table_at_capacity_events = meter
            .u64_counter("arete.vm.state_table.at_capacity_events")
            .with_description("State table at capacity events")
            .init();

        let vm_cleanup_pending_removed = meter
            .u64_counter("arete.vm.cleanup.pending_removed")
            .with_description("Pending updates removed during cleanup")
            .init();

        let vm_cleanup_temporal_removed = meter
            .u64_counter("arete.vm.cleanup.temporal_removed")
            .with_description("Temporal entries removed during cleanup")
            .init();

        let vm_path_cache_hits = meter
            .u64_counter("arete.vm.path_cache.hits")
            .with_description("Path cache hits")
            .init();

        let vm_path_cache_misses = meter
            .u64_counter("arete.vm.path_cache.misses")
            .with_description("Path cache misses")
            .init();

        let vm_lookup_index_hits = meter
            .u64_counter("arete.vm.lookup_index.hits")
            .with_description("Lookup index hits")
            .init();

        let vm_lookup_index_misses = meter
            .u64_counter("arete.vm.lookup_index.misses")
            .with_description("Lookup index misses")
            .init();

        let vm_pending_updates_queued = meter
            .u64_counter("arete.vm.pending_updates.queued")
            .with_description("Updates queued for later processing")
            .init();

        let vm_pending_updates_flushed = meter
            .u64_counter("arete.vm.pending_updates.flushed")
            .with_description("Queued updates flushed after PDA resolution")
            .init();

        let vm_pending_updates_expired = meter
            .u64_counter("arete.vm.pending_updates.expired")
            .with_description("Queued updates that expired")
            .init();

        Self {
            meter,
            ws_connections_total,
            ws_connections_active,
            ws_messages_received,
            ws_messages_sent,
            ws_connection_duration,
            ws_subscriptions_active,
            projector_mutations_processed,
            projector_frames_published,
            projector_processing_latency,
            stream_events_received,
            stream_errors_total,
            vm_instructions_executed,
            vm_events_processed,
            vm_event_processing_duration,
            vm_mutations_emitted,
            vm_pda_cache_hits,
            vm_pda_cache_misses,
            vm_pending_queue_size,
            entities_active,
            vm_state_table_entries,
            vm_state_table_capacity,
            vm_lookup_index_count,
            vm_lookup_index_entries,
            vm_temporal_index_count,
            vm_temporal_index_entries,
            vm_pda_reverse_lookup_count,
            vm_pda_reverse_lookup_entries,
            vm_version_tracker_entries,
            vm_path_cache_size,
            vm_pending_queue_updates,
            vm_pending_queue_unique_pdas,
            vm_pending_queue_memory_bytes,
            vm_pending_queue_oldest_age,
            vm_state_table_evictions,
            vm_state_table_at_capacity_events,
            vm_cleanup_pending_removed,
            vm_cleanup_temporal_removed,
            vm_path_cache_hits,
            vm_path_cache_misses,
            vm_lookup_index_hits,
            vm_lookup_index_misses,
            vm_pending_updates_queued,
            vm_pending_updates_flushed,
            vm_pending_updates_expired,
        }
    }

    // ==================== WebSocket Helpers ====================

    /// Record a new WebSocket connection
    pub fn record_ws_connection(&self) {
        self.ws_connections_total.add(1, &[]);
        self.ws_connections_active.add(1, &[]);
    }

    /// Record a new WebSocket connection with metering key attribution
    pub fn record_ws_connection_with_metering(&self, metering_key: &str) {
        let attrs = &[KeyValue::new("metering_key", metering_key.to_string())];
        self.ws_connections_total.add(1, attrs);
        self.ws_connections_active.add(1, attrs);
    }

    /// Record a WebSocket disconnection with duration
    pub fn record_ws_disconnection(&self, duration_secs: f64) {
        self.ws_connections_active.add(-1, &[]);
        self.ws_connection_duration.record(duration_secs, &[]);
    }

    /// Record a WebSocket disconnection with metering key attribution
    pub fn record_ws_disconnection_with_metering(&self, duration_secs: f64, metering_key: &str) {
        let attrs = &[KeyValue::new("metering_key", metering_key.to_string())];
        self.ws_connections_active.add(-1, attrs);
        self.ws_connection_duration.record(duration_secs, attrs);
    }

    /// Record a WebSocket message received
    pub fn record_ws_message_received(&self) {
        self.ws_messages_received.add(1, &[]);
    }

    /// Record a WebSocket message received with metering key attribution
    pub fn record_ws_message_received_with_metering(&self, metering_key: &str) {
        self.ws_messages_received.add(
            1,
            &[KeyValue::new("metering_key", metering_key.to_string())],
        );
    }

    /// Record a WebSocket message sent
    pub fn record_ws_message_sent(&self) {
        self.ws_messages_sent.add(1, &[]);
    }

    /// Record a WebSocket message sent with metering key attribution
    pub fn record_ws_message_sent_with_metering(&self, metering_key: &str) {
        self.ws_messages_sent.add(
            1,
            &[KeyValue::new("metering_key", metering_key.to_string())],
        );
    }

    /// Record a subscription created for a view
    pub fn record_subscription_created(&self, view_id: &str) {
        self.ws_subscriptions_active
            .add(1, &[KeyValue::new("view_id", view_id.to_string())]);
    }

    /// Record a subscription created with metering key attribution
    pub fn record_subscription_created_with_metering(&self, view_id: &str, metering_key: &str) {
        self.ws_subscriptions_active.add(
            1,
            &[
                KeyValue::new("view_id", view_id.to_string()),
                KeyValue::new("metering_key", metering_key.to_string()),
            ],
        );
    }

    /// Record a subscription removed for a view
    pub fn record_subscription_removed(&self, view_id: &str) {
        self.ws_subscriptions_active
            .add(-1, &[KeyValue::new("view_id", view_id.to_string())]);
    }

    /// Record a subscription removed with metering key attribution
    pub fn record_subscription_removed_with_metering(&self, view_id: &str, metering_key: &str) {
        self.ws_subscriptions_active.add(
            -1,
            &[
                KeyValue::new("view_id", view_id.to_string()),
                KeyValue::new("metering_key", metering_key.to_string()),
            ],
        );
    }

    // ==================== Projector Helpers ====================

    /// Record a mutation processed
    pub fn record_mutation_processed(&self, entity: &str) {
        self.projector_mutations_processed
            .add(1, &[KeyValue::new("entity", entity.to_string())]);
    }

    /// Record a frame published
    pub fn record_frame_published(&self, mode: &str, entity: &str) {
        self.projector_frames_published.add(
            1,
            &[
                KeyValue::new("mode", mode.to_string()),
                KeyValue::new("entity", entity.to_string()),
            ],
        );
    }

    /// Record projector processing latency in milliseconds
    pub fn record_projector_latency(&self, latency_ms: f64) {
        self.projector_processing_latency.record(latency_ms, &[]);
    }

    // ==================== Stream Helpers ====================

    /// Record an event received from the stream
    pub fn record_stream_event(&self, event_type: &str) {
        self.stream_events_received
            .add(1, &[KeyValue::new("event_type", event_type.to_string())]);
    }

    /// Record a stream error
    pub fn record_stream_error(&self, error_type: &str) {
        self.stream_errors_total
            .add(1, &[KeyValue::new("error_type", error_type.to_string())]);
    }

    // ==================== VM Helpers ====================

    /// Record VM instructions executed
    pub fn record_vm_instructions(&self, count: u64) {
        self.vm_instructions_executed.add(count, &[]);
    }

    /// Record a VM event processed
    pub fn record_vm_event(&self, event_type: &str, program_id: &str) {
        self.vm_events_processed.add(
            1,
            &[
                KeyValue::new("event_type", event_type.to_string()),
                KeyValue::new("program_id", program_id.to_string()),
            ],
        );
    }

    /// Record VM event processing duration in milliseconds
    pub fn record_vm_event_duration(&self, duration_ms: f64, event_type: &str) {
        self.vm_event_processing_duration.record(
            duration_ms,
            &[KeyValue::new("event_type", event_type.to_string())],
        );
    }

    /// Record mutations emitted by the VM
    pub fn record_vm_mutations(&self, count: u64, entity: &str) {
        self.vm_mutations_emitted
            .add(count, &[KeyValue::new("entity", entity.to_string())]);
    }

    /// Record a PDA cache hit
    pub fn record_pda_cache_hit(&self) {
        self.vm_pda_cache_hits.add(1, &[]);
    }

    /// Record a PDA cache miss
    pub fn record_pda_cache_miss(&self) {
        self.vm_pda_cache_misses.add(1, &[]);
    }

    /// Update the pending queue size
    pub fn update_pending_queue_size(&self, delta: i64) {
        self.vm_pending_queue_size.add(delta, &[]);
    }

    // ==================== Interpreter Cache Helpers ====================

    /// Record all gauge metrics from VmMemoryStats
    pub fn record_vm_memory_stats(
        &self,
        stats: &arete_interpreter::VmMemoryStats,
        entity: &str,
    ) {
        let attrs = &[KeyValue::new("entity", entity.to_string())];

        self.vm_state_table_entries
            .record(stats.state_table_entity_count as i64, attrs);
        self.vm_state_table_capacity
            .record(stats.state_table_max_entries as i64, attrs);

        self.vm_lookup_index_count
            .record(stats.lookup_index_count as i64, attrs);
        self.vm_lookup_index_entries
            .record(stats.lookup_index_total_entries as i64, attrs);

        self.vm_temporal_index_count
            .record(stats.temporal_index_count as i64, attrs);
        self.vm_temporal_index_entries
            .record(stats.temporal_index_total_entries as i64, attrs);

        self.vm_pda_reverse_lookup_count
            .record(stats.pda_reverse_lookup_count as i64, attrs);
        self.vm_pda_reverse_lookup_entries
            .record(stats.pda_reverse_lookup_total_entries as i64, attrs);

        self.vm_version_tracker_entries
            .record(stats.version_tracker_entries as i64, attrs);

        self.vm_path_cache_size
            .record(stats.path_cache_size as i64, attrs);

        if let Some(ref pq) = stats.pending_queue_stats {
            self.vm_pending_queue_updates
                .record(pq.total_updates as i64, attrs);
            self.vm_pending_queue_unique_pdas
                .record(pq.unique_pdas as i64, attrs);
            self.vm_pending_queue_memory_bytes
                .record(pq.estimated_memory_bytes as i64, attrs);
            self.vm_pending_queue_oldest_age
                .record(pq.oldest_age_seconds as f64, attrs);
        }
    }

    /// Record state table evictions
    pub fn record_state_table_eviction(&self, count: u64, entity: &str) {
        self.vm_state_table_evictions
            .add(count, &[KeyValue::new("entity", entity.to_string())]);
    }

    /// Record state table at capacity event
    pub fn record_state_table_at_capacity(&self, entity: &str) {
        self.vm_state_table_at_capacity_events
            .add(1, &[KeyValue::new("entity", entity.to_string())]);
    }

    /// Record cleanup results
    pub fn record_vm_cleanup(&self, pending_removed: usize, temporal_removed: usize, entity: &str) {
        let attrs = &[KeyValue::new("entity", entity.to_string())];
        self.vm_cleanup_pending_removed
            .add(pending_removed as u64, attrs);
        self.vm_cleanup_temporal_removed
            .add(temporal_removed as u64, attrs);
    }

    /// Record a path cache hit
    pub fn record_path_cache_hit(&self) {
        self.vm_path_cache_hits.add(1, &[]);
    }

    /// Record a path cache miss
    pub fn record_path_cache_miss(&self) {
        self.vm_path_cache_misses.add(1, &[]);
    }

    /// Record a lookup index hit
    pub fn record_lookup_index_hit(&self, index_name: &str) {
        self.vm_lookup_index_hits
            .add(1, &[KeyValue::new("index", index_name.to_string())]);
    }

    /// Record a lookup index miss
    pub fn record_lookup_index_miss(&self, index_name: &str) {
        self.vm_lookup_index_misses
            .add(1, &[KeyValue::new("index", index_name.to_string())]);
    }

    /// Record an update queued for later processing
    pub fn record_pending_update_queued(&self, entity: &str) {
        self.vm_pending_updates_queued
            .add(1, &[KeyValue::new("entity", entity.to_string())]);
    }

    /// Record queued updates flushed
    pub fn record_pending_updates_flushed(&self, count: u64, entity: &str) {
        self.vm_pending_updates_flushed
            .add(count, &[KeyValue::new("entity", entity.to_string())]);
    }

    /// Record expired pending updates
    pub fn record_pending_updates_expired(&self, count: u64, entity: &str) {
        self.vm_pending_updates_expired
            .add(count, &[KeyValue::new("entity", entity.to_string())]);
    }

    // ==================== Business Helpers ====================

    /// Record an entity being tracked
    pub fn record_entity_active(&self, entity_name: &str) {
        self.entities_active
            .add(1, &[KeyValue::new("entity", entity_name.to_string())]);
    }

    /// Record an entity no longer being tracked
    pub fn record_entity_inactive(&self, entity_name: &str) {
        self.entities_active
            .add(-1, &[KeyValue::new("entity", entity_name.to_string())]);
    }
}

/// Timer guard for automatic duration recording
pub struct MetricsTimer {
    start: Instant,
    histogram: Histogram<f64>,
    attributes: Vec<KeyValue>,
}

impl MetricsTimer {
    /// Create a new timer for recording duration
    pub fn new(histogram: Histogram<f64>, attributes: Vec<KeyValue>) -> Self {
        Self {
            start: Instant::now(),
            histogram,
            attributes,
        }
    }

    /// Stop the timer and record the duration in milliseconds
    pub fn stop(self) -> f64 {
        let duration_ms = self.start.elapsed().as_secs_f64() * 1000.0;
        self.histogram.record(duration_ms, &self.attributes);
        duration_ms
    }
}

impl Drop for MetricsTimer {
    fn drop(&mut self) {
        // Timer is recorded on explicit stop() call, not on drop
        // This allows for optional recording
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_creation() {
        // Just verify that metrics can be created without panic
        let _metrics = Metrics::new("test_service");
    }
}
