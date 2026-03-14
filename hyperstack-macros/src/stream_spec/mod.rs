//! Hyperstack stream processing module.
//!
//! This module handles the processing of `#[hyperstack]` macro attributes,
//! converting entity struct definitions into stream specifications.
//!
//! ## Module Structure
//!
//! - `module` - Entry point for module-level macro processing
//! - `entity` - Core entity struct processing logic (2,072 LOC)
//! - `handlers` - Handler generation for events and resolvers (559 LOC)
//! - `sections` - Nested struct and section processing (464 LOC)
//! - `computed` - Computed field expression parsing (461 LOC)
//! - `ast_writer` - AST JSON file generation at compile time (620 LOC)
//! - `idl_spec` - IDL-based stream processing (~300 LOC)
//! - `proto_struct` - Proto-based struct processing (~380 LOC)
//!
//! ## Processing Flow
//!
//! 1. `process_module` receives a module with `#[hyperstack]` attribute
//! 2. Parses IDL or proto files based on attribute arguments
//! 3. Processes each entity struct to extract field mappings
//! 4. Generates handler functions for each source type
//! 5. Writes AST JSON file for runtime consumption
//! 6. Returns generated code including state struct and spec function
//!
//! ## Example
//!
//! ```rust,ignore
//! #[hyperstack(idl = "idl.json")]
//! pub mod my_stream {
//!     #[entity(name = "MyEntity")]
//!     struct MyEntity {
//!         #[map(from = MyAccount::mint, primary_key)]
//!         pub mint: String,
//!     }
//! }
//! ```

mod ast_writer;
mod computed;
mod entity;
mod handlers;
mod idl_spec;
mod module;
mod proto_struct;
mod sections;

// Re-export module processing functions (used by lib.rs)
pub use module::process_module;

// Re-export proto struct processing (used by lib.rs)
pub use proto_struct::process_struct_with_context;

/// Resolves the source field name and whether this is a whole-source capture
/// based on the `field` and `field_transforms` parameters.
pub fn resolve_snapshot_source(
    snapshot_attr: &crate::parse::attributes::CaptureAttribute,
) -> (String, bool) {
    if let Some(ref field_ident) = snapshot_attr.field {
        // Single field extraction: field = token_mint_0
        (field_ident.to_string(), false)
    } else if !snapshot_attr.field_transforms.is_empty() {
        // Whole source with transforms
        (
            format!(
                "__snapshot_with_transforms:{}",
                snapshot_attr
                    .field_transforms
                    .iter()
                    .map(|(k, v)| format!("{}={}", k, v))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            true,
        )
    } else {
        // Whole source capture (no field, no transforms)
        (String::new(), true)
    }
}
