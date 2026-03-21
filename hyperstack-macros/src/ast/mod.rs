//! AST module for hyperstack streams.
//!
//! This module contains the serializable AST types used for:
//! - Compile-time AST serialization (from `#[hyperstack]`)
//! - AST-based compilation (via `#[ast_spec]`)
//! - Cross-crate communication (hyperstack-macros -> hyperstack runtime)
//!
//! ## Submodules
//!
//! - `types` - Serializable AST type definitions (~450 LOC)
//! - `writer` - AST JSON file serialization (~620 LOC)
//! - `reader` - AST JSON file deserialization (~150 LOC)
//!
//! ## Compilation Paths
//!
//! The AST module enables two compilation paths:
//!
//! ### 1. Traditional Path (`#[hyperstack]`)
//!
//! ```text
//! Rust Source -> #[hyperstack] macro -> Generated Code
//!                      |
//!                      +-> AST JSON file (side effect)
//! ```
//!
//! ### 2. AST Path (`#[ast_spec]`)
//!
//! ```text
//! AST JSON file -> #[ast_spec] macro -> Generated Code
//! ```
//!
//! This enables:
//! - Decoupled compilation (generate AST once, compile many times)
//! - Cloud deployment (upload AST JSON, compile remotely)
//! - Cross-language support (any language can generate AST JSON)
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
//! These types are intentionally duplicated from `hyperstack_interpreter::ast` because proc-macro
//! crates cannot depend on their output crates (this would create a circular dependency).

mod reader;
mod types;
pub mod versioned;
pub(crate) mod writer;

// Re-export all types for easy access
pub use types::*;
