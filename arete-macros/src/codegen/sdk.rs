//! SDK type generation from IDL specifications.
//!
//! Generates account types, instruction types, and custom types from an Anchor IDL.

use crate::parse::idl::IdlSpec;
use proc_macro2::TokenStream;

/// Generate SDK types (accounts, instructions, custom types) from an IDL spec.
///
/// This is the equivalent of `idl_codegen::generate_sdk_types`.
pub fn generate_sdk_from_idl(idl: &IdlSpec, module_name: &str) -> TokenStream {
    crate::idl_codegen::generate_sdk_types(idl, module_name)
}
