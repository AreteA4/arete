//! MutationBatch - Envelope type for propagating trace context across async boundaries.

use arete_interpreter::Mutation;
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use tracing::Span;

/// The producer-local meaning of [`SlotContext::slot_index`].
///
/// Values from different domains are deliberately incomparable. In
/// particular, an async resolver counter must never make a later instruction
/// transaction index look stale within the same slot.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SlotIndexDomain {
    /// Backward-compatible domain for callers of [`SlotContext::new`].
    #[default]
    Legacy,
    Account,
    Instruction,
    Resolver,
}

/// Slot context for ordering mutations by blockchain position.
/// Used to derive `_seq` field for default recency sorting.
#[derive(Debug, Clone, Copy, Default)]
pub struct SlotContext {
    /// Solana slot number
    pub slot: u64,
    /// Index within the slot (write_version for accounts, txn_index for instructions)
    pub slot_index: u64,
    /// The producer-local domain in which `slot_index` is ordered.
    pub slot_index_domain: SlotIndexDomain,
}

impl SlotContext {
    pub fn with_domain(slot: u64, slot_index: u64, slot_index_domain: SlotIndexDomain) -> Self {
        Self {
            slot,
            slot_index,
            slot_index_domain,
        }
    }

    /// Construct a context in the legacy generic domain.
    ///
    /// Generated ingestion uses the explicit constructors below so account,
    /// instruction and resolver offsets are never compared accidentally.
    pub fn new(slot: u64, slot_index: u64) -> Self {
        Self::with_domain(slot, slot_index, SlotIndexDomain::Legacy)
    }

    pub fn account(slot: u64, write_version: u64) -> Self {
        Self::with_domain(slot, write_version, SlotIndexDomain::Account)
    }

    pub fn instruction(slot: u64, txn_index: u64) -> Self {
        Self::with_domain(slot, txn_index, SlotIndexDomain::Instruction)
    }

    pub fn resolver(slot: u64, resolver_index: u64) -> Self {
        Self::with_domain(slot, resolver_index, SlotIndexDomain::Resolver)
    }

    /// Compute a monotonic sequence number for sorting.
    /// Encodes as string to preserve precision in JSON: "{slot}:{slot_index:012}"
    /// This gives lexicographic ordering that matches (slot, slot_index) tuple ordering.
    pub fn to_seq_string(&self) -> String {
        format!("{}:{:012}", self.slot, self.slot_index)
    }
}

/// Envelope type that carries mutations along with their originating span context.
///
/// This enables trace context propagation across the mpsc channel boundary
/// from the Vixen parser to the Projector.
#[derive(Debug)]
pub struct MutationBatch {
    /// The span from which these mutations originated
    pub span: Span,
    /// The mutations to process
    pub mutations: SmallVec<[Mutation; 6]>,
    /// Slot context for ordering (optional for backward compatibility)
    pub slot_context: Option<SlotContext>,
    /// Event metadata for logging and diagnostics
    pub event_context: Option<EventContext>,
    /// When set, this batch is a flush marker: the projector acknowledges it
    /// after every batch queued before it has been applied to the caches.
    /// Used by the snapshot manager to establish a consistency cut.
    pub flush_ack: Option<tokio::sync::oneshot::Sender<()>>,
    /// Keeps snapshot capture blocked from the VM update that produced this
    /// batch until the projector has applied it.
    pub(crate) snapshot_guard: Option<crate::snapshot::SnapshotProcessingGuard>,
}

#[derive(Debug, Clone)]
pub struct EventContext {
    pub program: String,
    pub event_kind: String,
    pub event_type: String,
    pub account: Option<String>,
    pub accounts_count: Option<usize>,
}

impl MutationBatch {
    pub fn new(mutations: SmallVec<[Mutation; 6]>) -> Self {
        Self {
            span: Span::current(),
            mutations,
            slot_context: None,
            event_context: None,
            flush_ack: None,
            snapshot_guard: None,
        }
    }

    pub fn with_span(span: Span, mutations: SmallVec<[Mutation; 6]>) -> Self {
        Self {
            span,
            mutations,
            slot_context: None,
            event_context: None,
            flush_ack: None,
            snapshot_guard: None,
        }
    }

    pub fn with_slot_context(
        mutations: SmallVec<[Mutation; 6]>,
        slot_context: SlotContext,
    ) -> Self {
        Self {
            span: Span::current(),
            mutations,
            slot_context: Some(slot_context),
            event_context: None,
            flush_ack: None,
            snapshot_guard: None,
        }
    }

    /// An empty batch whose only purpose is to be acknowledged once the
    /// projector has drained everything queued before it.
    pub fn flush_marker(ack: tokio::sync::oneshot::Sender<()>) -> Self {
        Self {
            span: Span::current(),
            mutations: SmallVec::new(),
            slot_context: None,
            event_context: None,
            flush_ack: Some(ack),
            snapshot_guard: None,
        }
    }

    /// Transfer a VM processing guard to this batch. The projector retains it
    /// until cache application and watermark advancement are complete.
    pub fn with_snapshot_guard(mut self, guard: crate::snapshot::SnapshotProcessingGuard) -> Self {
        self.snapshot_guard = Some(guard);
        self
    }

    pub fn with_event_context(mut self, event_context: EventContext) -> Self {
        self.event_context = Some(event_context);
        self
    }

    pub fn len(&self) -> usize {
        self.mutations.len()
    }

    pub fn is_empty(&self) -> bool {
        self.mutations.is_empty()
    }
}
