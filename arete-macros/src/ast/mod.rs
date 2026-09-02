//! AST module for arete streams.
//!
//! This module contains the serializable compiler types used for compile-time
//! code generation and projection into exact public artifacts.
//!
//! ## Submodules
//!
//! - `types` - Serializable AST type definitions (~450 LOC)
//! - `writer` - exact public-artifact authoring and AST projection helpers
//!
//! The `#[arete]` path emits generated Rust plus ProgramSpec, LiveSpec, and
//! StackManifest artifacts. The serializable compiler model is embedded in
//! generated code where required; it is not emitted as a public file.
//!
//! ## Key Types
//!
//! - `SerializableStreamSpec` - Top-level spec containing all entity information
//! - `SerializableHandlerSpec` - Handler specification (source, key resolution, mappings)
//! - `SerializableFieldMapping` - Field mapping with source, target, and transformation
//! - `ResolverHook` - Key resolution hooks for PDA lookups
//! - `InstructionHook` - Post-instruction actions (PDA registration, field updates)
//!
//! ## Note on Duplication
//!
//! These types are intentionally duplicated from `arete_interpreter::ast` because proc-macro
//! crates cannot depend on their output crates (this would create a circular dependency).

mod types;
pub mod versioned;
pub(crate) mod writer;

// Re-export all types for easy access
pub use types::*;
