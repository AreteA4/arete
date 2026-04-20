//! Vixen parser generation from IDL specifications.
//!
//! Generates account parsers and instruction parsers for yellowstone-vixen runtime.

use crate::parse::idl::IdlSpec;
use proc_macro2::TokenStream;

/// Generate Vixen parsers for accounts and instructions from an IDL spec.
///
/// This is the equivalent of `idl_parser_gen::generate_parsers`.
pub fn generate_parsers_from_idl(
    idl: &IdlSpec,
    program_id: &str,
    sdk_module_name: &str,
) -> TokenStream {
    crate::idl_parser_gen::generate_parsers(idl, program_id, sdk_module_name)
}
