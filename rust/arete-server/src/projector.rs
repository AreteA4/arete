use crate::bus::{BusManager, BusMessage};
use crate::cache::EntityCache;
use crate::mutation_batch::{MutationBatch, SlotContext};
use crate::view::{ViewIndex, ViewSpec};
use crate::websocket::frame::{apply_wire_format, Mode, SourceFrame};
use arete_interpreter::CanonicalLog;
use bytes::Bytes;
use serde_json::Value;
use smallvec::SmallVec;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, debug_span, error, instrument};

#[cfg(feature = "otel")]
use crate::metrics::Metrics;

pub struct Projector {
    view_index: Arc<ViewIndex>,
    bus_manager: BusManager,
    entity_cache: EntityCache,
    mutations_rx: mpsc::Receiver<MutationBatch>,
    snapshot_runtime: Option<crate::snapshot::SnapshotRuntime>,
    journal: Option<Arc<crate::journal::EventJournal>>,
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

            for mutation in std::mem::take(&mut batch.mutations).into_iter() {
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
            mut patch,
            append,
            occurrence,
            ..
        } = mutation;

        // What lets a resume recognise an event it already retained. Needs the
        // slot as well as the decode site: an occurrence is only unique within
        // the transaction it came from.
        let origin = slot_context.map(|ctx| crate::journal::EventOrigin {
            slot: ctx.slot,
            index: ctx.slot_index,
            occurrence,
        });

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

            let projected = spec.projection.apply(patch_data);
            let mut wire_data = projected.clone();
            apply_wire_format(&mut wire_data, &spec.wire_format);

            // Extract _seq from the patch data to include in the frame
            let seq = slot_context.map(|ctx| ctx.to_seq_string());

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
                op: "patch",
                key: key.clone(),
                data: wire_data,
                append: append.clone(),
                seq,
                offset: None,
            };

            let retained = match journal {
                Some(journal) => {
                    journal
                        .append_with(&spec.id, &key, origin.clone(), |offset| {
                            frame.offset = Some(offset);
                            json_buffer.clear();
                            serde_json::to_writer(&mut *json_buffer, &frame)?;
                            Ok::<_, anyhow::Error>(Arc::new(Bytes::copy_from_slice(json_buffer)))
                        })
                        .await?
                }
                None => crate::journal::Append::Untracked,
            };
            let payload = match retained {
                crate::journal::Append::Retained { payload, .. } => payload,
                // A resume re-delivered an event the tape already holds.
                // Publishing it would hand live subscribers a duplicate too.
                crate::journal::Append::Duplicate => continue,
                // No tape, or a sealed one: the event still publishes, it just
                // carries no position to resume from.
                crate::journal::Append::Untracked => {
                    frame.offset = None;
                    json_buffer.clear();
                    serde_json::to_writer(&mut *json_buffer, &frame)?;
                    Arc::new(Bytes::copy_from_slice(json_buffer))
                }
            };

            self.entity_cache
                .upsert_with_append(&spec.id, &key, projected, &frame.append)
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
