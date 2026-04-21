//! # arete-macros
//!
//! Procedural macros for defining Arete streams.
//!
//! This crate provides the `#[arete]` attribute macro that transforms
//! annotated Rust structs into full streaming pipeline specifications, including:
//!
//! - State struct generation with field accessors
//! - Handler creation functions for event processing
//! - IDL/Proto parser integration for Solana programs
//! - Automatic AST serialization for deployment
//!
//! ## Module Usage (IDL-based)
//!
//! ```rust,ignore
//! use arete_macros::{arete, Stream};
//!
//! #[arete(idl = "idl.json")]
//! pub mod my_stream {
//!     #[entity(name = "MyEntity")]
//!     #[derive(Stream)]
//!     struct Entity {
//!         #[map(from = "MyAccount", field = "value")]
//!         pub value: u64,
//!     }
//! }
//! ```
//!
//! ## Supported Attributes
//!
//! - `#[map(...)]` - Map from account fields
//! - `#[from_instruction(...)]` - Map from instruction fields
//! - `#[event(...)]` - Capture instruction events
//! - `#[snapshot(...)]` - Capture entire source data
//! - `#[aggregate(...)]` - Aggregate field values
//! - `#[computed(...)]` - Computed fields from other fields
//! - `#[derive_from(...)]` - Derive values from instructions

// Public modules - AST types needed for SDK generation
pub(crate) mod ast;
mod diagnostic;
pub(crate) mod event_type_helpers;

// Internal modules - not exposed publicly
mod codegen;
mod idl_codegen;
mod idl_parser_gen;
mod idl_vixen_gen;
mod parse;
mod proto_codegen;
mod stream_spec;
mod utils;
mod validation;

use proc_macro::TokenStream;
use std::collections::HashMap;
use syn::{ItemMod, ItemStruct};

// Use the stream_spec module functions
use stream_spec::{process_module, process_struct_with_context};

/// Process a `#[arete(...)]` attribute.
///
/// This macro can be applied to:
/// - A module containing entity structs
/// - A single struct (legacy usage)
///
/// ## Module Usage (IDL-based)
///
/// ```rust,ignore
/// #[arete(idl = "idl.json")]
/// pub mod my_stream {
///     #[entity(name = "MyEntity")]
///     struct Entity {
///         // fields with mapping attributes
///     }
/// }
/// ```
///
/// ## Proto-based Usage
///
/// ```rust,ignore
/// #[arete(proto = ["events.proto"])]
/// pub mod my_stream {
///     // entity structs
/// }
/// ```
#[proc_macro_attribute]
pub fn arete(attr: TokenStream, item: TokenStream) -> TokenStream {
    expand_arete(attr, item)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

fn expand_arete(attr: TokenStream, item: TokenStream) -> syn::Result<proc_macro2::TokenStream> {
    let mod_err = match syn::parse::<ItemMod>(item.clone()) {
        Ok(module) => return process_module(module, attr),
        Err(e) => e,
    };

    let input = syn::parse::<ItemStruct>(item).map_err(|struct_err| {
        // If neither parse succeeds, prefer the module error since most usages
        // of #[arete] are on modules.
        let mut combined = mod_err;
        combined.combine(struct_err);
        combined
    })?;

    let config = parse::parse_stream_spec_attribute(attr)?;
    if !config.proto_files.is_empty() || !config.idl_files.is_empty() || config.skip_decoders {
        return Err(syn::Error::new(
            input.ident.span(),
            "#[arete(...)] arguments are only supported on modules",
        ));
    }

    process_struct_with_context(input, HashMap::new(), false)
}

/// Derive macro for `Stream`.
///
/// This is a marker derive that enables the following attributes on struct fields:
/// - `#[map(...)]` - Map from account fields
/// - `#[from_instruction(...)]` - Map from instruction fields
/// - `#[event(...)]` - Capture instruction events
/// - `#[snapshot(...)]` - Capture entire source
/// - `#[aggregate(...)]` - Aggregate field values
/// - `#[computed(...)]` - Computed fields from other fields
/// - `#[derive_from(...)]` - Derive values from instructions
/// - `#[resolve(...)]` - Resolve external data (token metadata via DAS API or data from URLs)
#[proc_macro_derive(
    Stream,
    attributes(
        map,
        from_instruction,
        event,
        snapshot,
        aggregate,
        computed,
        derive_from,
        resolve
    )
)]
pub fn stream_derive(_input: TokenStream) -> TokenStream {
    TokenStream::new()
}
