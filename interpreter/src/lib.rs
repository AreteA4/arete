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
pub mod event_type_helpers;
pub mod metrics_context;
pub mod proto_router;
pub mod resolvers;
pub mod runtime_resolvers;
pub mod runtime_resolvers_factory;
pub mod rust;
pub mod scheduler;
pub mod slot_hash_cache;
pub mod spec_trait;
pub mod typescript;
pub mod versioned;
pub mod vm;
pub mod vm_metrics;

// Re-export slot hash cache functions
pub use slot_hash_cache::{get_slot_hash, record_slot_hash};

pub use canonical_log::{CanonicalLog, LogLevel};
pub use metrics_context::{FieldAccessor, FieldRef, MetricsContext};
pub use resolvers::{
    InstructionContext, KeyResolution, ResolveContext, ReverseLookupUpdater, TokenMetadata,
};
pub use runtime_resolvers::{
    InProcessResolver, ResolverApplyFuture, ResolverBatchFuture, ResolverBatchResult,
    RuntimeResolver, RuntimeResolverBatchRequest, RuntimeResolverBatchResponse,
    RuntimeResolverRequest, RuntimeResolverResponse, SharedRuntimeResolver,
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
///   "signature": "4xNEYTVL8DB28W87..."
/// }
/// ```
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
