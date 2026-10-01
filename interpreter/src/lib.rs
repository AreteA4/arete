//! # arete-interpreter
//!
//! AST transformation runtime and VM for Arete streaming pipelines.
//!
//! This crate provides the core components for processing Solana blockchain
//! events into typed state projections:
//!
//! - **AST Definition** - Type-safe schemas for state and event handlers
//! - **Bytecode Compiler** - Compiles specs into optimized bytecode  
//! - **Virtual Machine** - Executes bytecode to process events
//! - **TypeScript Generation** - Generate client SDKs automatically
//!
//! ## Example
//!
//! ```rust,ignore
//! use arete_interpreter::{TypeScriptCompiler, TypeScriptConfig};
//!
//! let config = TypeScriptConfig::default();
//! let compiler = TypeScriptCompiler::new(config);
//! let typescript = compiler.compile(&spec)?;
//! ```
//!
//! ## Feature Flags
//!
//! - `otel` - OpenTelemetry integration for distributed tracing and metrics

pub mod ast;
pub mod canonical_log;
pub mod compiler;
pub mod debugger;
pub mod event_type_helpers;
pub mod identifiers;
mod managed_account_models;
pub mod metrics_context;
pub mod program_sdk;
pub mod proto_router;
pub mod public_artifacts;
pub mod python;
pub mod resolvers;
pub mod runtime_resolvers;
pub mod runtime_resolvers_factory;
pub mod rust;
pub mod scheduler;
pub mod slot_hash_cache;
pub mod snapshot;
pub mod spec_trait;
mod stack_types;
pub mod transaction_metadata;
pub mod typescript;
pub mod typescript_instructions;
pub mod versioned;
pub mod vm;
pub mod vm_metrics;

// Re-export slot hash cache functions
pub use slot_hash_cache::{get_slot_hash, record_slot_hash};

pub use canonical_log::{CanonicalLog, LogLevel};
pub use debugger::{VmDebugEvent, VmDebugger, VmLookupHop};
pub use metrics_context::{FieldAccessor, FieldRef, MetricsContext};
pub use resolvers::{
    InstructionContext, KeyResolution, ResolveContext, ReverseLookupUpdater, TokenMetadata,
};
pub use runtime_resolvers::{
    InProcessResolver, ResolverApplyFuture, ResolverBatchFuture, ResolverBatchResult,
    RuntimeResolver, RuntimeResolverBatchRequest, RuntimeResolverBatchResponse,
    RuntimeResolverRequest, RuntimeResolverResponse, SharedRuntimeResolver,
};
pub use transaction_metadata::{
    SolanaTransactionConfig, SolanaTransactionMetadata, SolanaTransactionVersion,
    SOLANA_TRANSACTION_METADATA_KEY,
};
pub use typescript::{write_typescript_to_file, TypeScriptCompiler, TypeScriptConfig};
pub use vm::{
    CapacityWarning, CleanupResult, DirtyTracker, FieldChange, PendingAccountUpdate,
    PendingQueueStats, QueuedAccountUpdate, ResolverRequest, ResolverTarget, ScheduledCallback,
    StateTableConfig, UpdateContext, VmMemoryStats,
};

// Re-export macros for convenient use
// The field! macro is the new recommended way to create field references
// The field_accessor! macro is kept for backward compatibility

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mutation {
    pub export: String,
    pub key: Value,
    pub patch: Value,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub append: Vec<String>,
}

/// Field that marks a mutation's `patch` as the whole entity rather than only
/// the fields that changed; its value says which kind ([`WholeEntity`]). Set
/// with [`Mutation::mark_whole_entity`] or [`Mutation::mark_created`] and
/// removed with [`Mutation::take_whole_entity_mark`] before the patch goes
/// anywhere else; it is never part of an entity.
pub const WHOLE_ENTITY_MARKER: &str = "__arete_whole_entity";

/// Reserved mutation metadata for an explicit source deletion. Never an entity field.
pub const ENTITY_DELETE_MARKER: &str = "__arete_entity_delete";

/// [`WHOLE_ENTITY_MARKER`]'s value for [`WholeEntity::Created`]. The value for
/// [`WholeEntity::Resent`] is `true`.
const CREATED_MARK: &str = "created";

/// Why a marked mutation's patch is the whole entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WholeEntity {
    /// The entity's first mutation since its source created it: everything it
    /// has is new, so the patch is a change like any other and all of the
    /// entity at once.
    Created,
    /// The entity again, right after a mutation that carried the change: state
    /// to restore, not a change of its own.
    Resent,
}

impl Mutation {
    /// End an entity's lifetime. Submit in the same ordered stream as its updates,
    /// with the deletion's slot/write version. This is distinct from view eviction.
    /// VM-backed sources must use [`vm::VmContext::delete_entity`] to remove state too.
    pub fn delete(export: impl Into<String>, key: Value) -> Self {
        Self {
            export: export.into(),
            key,
            patch: serde_json::json!({ ENTITY_DELETE_MARKER: true }),
            append: Vec::new(),
        }
    }

    pub fn is_delete(&self) -> bool {
        self.patch.get(ENTITY_DELETE_MARKER) == Some(&Value::Bool(true))
    }

    /// Declare that `patch` holds the whole entity again, after a mutation
    /// that carried the change ([`WholeEntity::Resent`]).
    ///
    /// A consumer that bounds how many entities it keeps (arete-server's
    /// entity cache) cannot rebuild an entity it dropped from later patches,
    /// which carry only changed fields; a mutation marked whole restores it.
    /// The VM marks the mutations it emits for keys requested through
    /// [`vm::WholeEntityRequests`]. A custom mutation source that emits whole
    /// entities can mark them itself. Only an object patch can carry the mark;
    /// on anything else this does nothing.
    pub fn mark_whole_entity(&mut self) {
        self.set_whole_entity_mark(Value::Bool(true));
    }

    /// Declare that this mutation creates its entity, so `patch` is all of it
    /// ([`WholeEntity::Created`]).
    ///
    /// A VM given a [`vm::WholeEntityRequests`] marks every entity's first
    /// mutation this way, which lets a consumer tell a new entity from a later
    /// patch for one it does not hold. Only an object patch can carry the mark;
    /// on anything else this does nothing.
    pub fn mark_created(&mut self) {
        self.set_whole_entity_mark(Value::String(CREATED_MARK.to_string()));
    }

    fn set_whole_entity_mark(&mut self, mark: Value) {
        if let Value::Object(fields) = &mut self.patch {
            fields.insert(WHOLE_ENTITY_MARKER.to_string(), mark);
        }
    }

    /// Which whole-entity mark `patch` carries, if any.
    pub fn whole_entity_mark(&self) -> Option<WholeEntity> {
        Self::read_whole_entity_mark(self.patch.get(WHOLE_ENTITY_MARKER)?)
    }

    /// Whether `patch` is marked as the whole entity, of either kind.
    pub fn is_whole_entity(&self) -> bool {
        self.whole_entity_mark().is_some()
    }

    /// Remove the whole-entity mark, returning which one was set.
    pub fn take_whole_entity_mark(&mut self) -> Option<WholeEntity> {
        match &mut self.patch {
            Value::Object(fields) => {
                Self::read_whole_entity_mark(&fields.remove(WHOLE_ENTITY_MARKER)?)
            }
            _ => None,
        }
    }

    fn read_whole_entity_mark(mark: &Value) -> Option<WholeEntity> {
        match mark {
            Value::Bool(true) => Some(WholeEntity::Resent),
            Value::String(kind) if kind == CREATED_MARK => Some(WholeEntity::Created),
            _ => None,
        }
    }
}

/// Generic wrapper for event data that includes context metadata
/// This ensures type safety for events captured in entity specs
///
/// # Runtime Structure
/// Events captured with `#[event]` are automatically wrapped in this structure:
/// ```json
/// {
///   "timestamp": 1234567890,
///   "data": { /* event-specific data */ },
///   "slot": 381471241,
///   "signature": "4xNEYTVL8DB28W87...",
///   "event_index": 3,
///   "ix_path": "0.1"
/// }
/// ```
///
/// `ix_path` is always present for a transaction-sourced event and identifies
/// the instruction it came from, so two same-type events under one signature
/// stay distinct. `event_index` is the absolute log-line index and appears
/// only for events decoded from a log: it is what makes one log line decoded
/// by both an outer instruction and its inner CPI resolve to a single
/// occurrence. An `emit_cpi!` event arrives as its own instruction and is
/// therefore identified by `ix_path` alone.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventWrapper<T = Value> {
    /// Unix timestamp when the event was processed
    pub timestamp: i64,
    /// The event-specific data
    pub data: T,
    /// Optional slot number from UpdateContext
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slot: Option<u64>,
    /// Optional transaction signature from UpdateContext
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    /// Absolute log-line index, for events decoded from a transaction log
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_index: Option<u64>,
    /// 0-based instruction path within the transaction (e.g. `"0.1"`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ix_path: Option<String>,
}

/// Generic wrapper for account capture data that includes context metadata
/// This ensures type safety for accounts captured with `#[capture]` in entity specs
///
/// # Runtime Structure
/// Accounts captured with `#[capture]` are automatically wrapped in this structure:
/// ```json
/// {
///   "timestamp": 1234567890,
///   "account_address": "C6P5CpJnYHgpGvCGuXYAWL6guKH5LApn3QwTAZmNUPCj",
///   "data": { /* account-specific data (filtered, no __ fields) */ },
///   "slot": 381471241,
///   "signature": "4xNEYTVL8DB28W87..."
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureWrapper<T = Value> {
    /// Unix timestamp when the account was captured
    pub timestamp: i64,
    /// The account address (base58 encoded public key)
    pub account_address: String,
    /// The account data (already filtered to remove internal __ fields)
    pub data: T,
    /// Optional slot number from UpdateContext
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slot: Option<u64>,
    /// Optional transaction signature from UpdateContext
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}
