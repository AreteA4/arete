use crate::bus::{BusManager, BusMessage};
use crate::cache::EntityCache;
use crate::mutation_batch::{MutationBatch, SlotContext};
use crate::view::{ViewIndex, ViewSpec};
use crate::websocket::frame::{transform_large_u64_to_strings, Frame, Mode};
use bytes::Bytes;
use arete_interpreter::CanonicalLog;
use serde_json::Value;
use smallvec::SmallVec;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error, instrument};

#[cfg(feature = "otel")]
use crate::metrics::Metrics;

pub struct Projector {
    view_index: Arc<ViewIndex>,
    bus_manager: BusManager,
    entity_cache: EntityCache,
    mutations_rx: mpsc::Receiver<MutationBatch>,
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
        }
    }

    pub async fn run(mut self) {
        debug!("Projector started");

        let mut json_buffer = Vec::with_capacity(4096);

        while let Some(batch) = self.mutations_rx.recv().await {
            let _span_guard = batch.span.enter();

            let mut log = CanonicalLog::new();
            log.set("phase", "projector");

            let batch_size = batch.len();
            let slot_context = batch.slot_context;
            let mut frames_published = 0u32;
            let mut errors = 0u32;

            if let Some(ctx) = batch.event_context.as_ref() {
                log.set("program", &ctx.program)
                    .set("event_kind", &ctx.event_kind)
                    .set("event_type", &ctx.event_type)
                    .set("account", &ctx.account)
                    .set("accounts_count", ctx.accounts_count);
            }

            for mutation in batch.mutations.into_iter() {
                #[cfg(feature = "otel")]
                let export = mutation.export.clone();

                match self
                    .process_mutation(mutation, slot_context, &mut json_buffer)
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

            log.set("batch_size", batch_size)
                .set("frames_published", frames_published)
                .set("errors", errors);

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
        skip(self, mutation, slot_context, json_buffer),
        fields(export = %mutation.export)
    )]
    async fn process_mutation(
        &self,
        mutation: arete_interpreter::Mutation,
        slot_context: Option<SlotContext>,
        json_buffer: &mut Vec<u8>,
    ) -> anyhow::Result<u32> {
        let specs = self.view_index.by_export(&mutation.export);

        if specs.is_empty() {
            return Ok(0);
        }

        let key = Self::extract_key(&mutation.key);
        let arete_interpreter::Mutation {
            mut patch, append, ..
        } = mutation;

        // Inject _seq for recency sorting if slot context is available
        if let Some(ctx) = slot_context {
            if let Value::Object(ref mut map) = patch {
                map.insert("_seq".to_string(), Value::String(ctx.to_seq_string()));
            }
        }

        let matching_specs: SmallVec<[&ViewSpec; 4]> = specs
            .iter()
            .filter(|spec| spec.filters.matches(&key))
            .collect();

        let match_count = matching_specs.len();
        if match_count == 0 {
            return Ok(0);
        }

        let mut frames_published = 0u32;

        for (i, spec) in matching_specs.into_iter().enumerate() {
            let is_last = i == match_count - 1;
            let patch_data = if is_last {
                std::mem::take(&mut patch)
            } else {
                patch.clone()
            };

            let mut projected = spec.projection.apply(patch_data);
            transform_large_u64_to_strings(&mut projected);

            // Extract _seq from the patch data to include in the frame
            let seq = slot_context.map(|ctx| ctx.to_seq_string());

            let frame = Frame {
                mode: spec.mode,
                export: spec.id.clone(),
                op: "patch",
                key: key.clone(),
                data: projected,
                append: append.clone(),
                seq,
            };

            json_buffer.clear();
            serde_json::to_writer(&mut *json_buffer, &frame)?;
            let payload = Arc::new(Bytes::copy_from_slice(json_buffer));

            self.entity_cache
                .upsert_with_append(&spec.id, &key, frame.data.clone(), &frame.append)
                .await;

            if spec.mode == Mode::List {
                self.update_derived_view_caches(&spec.id, &key).await;
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

        Ok(frames_published)
    }

    fn extract_key(key: &serde_json::Value) -> String {
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

        let sorted_caches = self.view_index.sorted_caches();
        let mut caches = sorted_caches.write().await;

        for derived_spec in derived_views {
            if let Some(cache) = caches.get_mut(&derived_spec.id) {
                cache.upsert(entity_key.to_string(), entity_data.clone());
                debug!(
                    "Updated sorted cache for derived view {} with key {}",
                    derived_spec.id, entity_key
                );
            }
        }
    }

    #[instrument(
        name = "projector.publish",
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
