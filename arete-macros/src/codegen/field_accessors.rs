//! Field accessor code generation for type-safe view definitions.
//!
//! This module generates `fields::` modules containing `FieldPath` constants
//! that can be used in view definitions for type-safe field references.
//!
//! # Example Output
//!
//! For an entity with sections `id` and `state`:
//!
//! ```rust,ignore
//! pub mod fields {
//!     use arete::runtime::arete_interpreter::ast::FieldPath;
//!     
//!     pub mod id {
//!         use super::*;
//!         
//!         #[allow(non_upper_case_globals)]
//!         pub const round_id: FieldPath = /* ... */;
//!         #[allow(non_upper_case_globals)]
//!         pub const round_address: FieldPath = /* ... */;
//!     }
//!     
//!     pub mod state {
//!         use super::*;
//!         
//!         #[allow(non_upper_case_globals)]
//!         pub const expires_at: FieldPath = /* ... */;
//!         #[allow(non_upper_case_globals)]
//!         pub const motherlode: FieldPath = /* ... */;
//!     }
//! }
//! ```

use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use crate::ast::{EntitySection, FieldTypeInfo, ResolvedField};

/// Generate field accessor module for an entity's sections.
///
/// This creates a `fields` module containing sub-modules for each section,
/// with `FieldPath` constants for each field that can be used in view definitions.
pub fn generate_field_accessors(sections: &[EntitySection]) -> TokenStream {
    let section_modules: Vec<TokenStream> = sections
        .iter()
        .filter(|s| s.name != "root") // Skip the root pseudo-section
        .map(|section| generate_section_module(&section.name, &section.fields))
        .collect();

    if section_modules.is_empty() {
        return quote! {};
    }

    quote! {
        /// Field path accessors for type-safe view definitions.
        ///
        /// Use these constants to reference entity fields in views:
        /// ```rust,ignore
        /// use crate::fields;
        ///
        /// ViewDef::new("MyEntity/latest")
        ///     .sort(fields::state::expires_at, SortOrder::Desc)
        ///     .first()
        /// ```
        pub mod fields {
            use arete::runtime::arete_interpreter::ast::FieldPath;

            #(#section_modules)*
        }
    }
}

/// Generate a module for a single section containing its field accessors.
fn generate_section_module(section_name: &str, fields: &[FieldTypeInfo]) -> TokenStream {
    let section_ident = format_ident!("{}", section_name);

    // Generate constants for each field in the section
    let field_constants: Vec<TokenStream> = fields
        .iter()
        .filter(|field| field.emit)
        .map(|field| generate_field_constant(section_name, &field.field_name))
        .collect();

    // Generate nested modules for complex types with resolved fields
    let nested_modules: Vec<TokenStream> = fields
        .iter()
        .filter(|field| field.emit)
        .filter_map(|field| {
            field.resolved_type.as_ref().and_then(|rt| {
                // Only generate nested accessors for struct types (not enums)
                // and only if they have fields
                if !rt.is_enum && !rt.fields.is_empty() {
                    Some(generate_nested_accessors(
                        section_name,
                        &field.field_name,
                        &rt.fields,
                    ))
                } else {
                    None
                }
            })
        })
        .collect();

    quote! {
        pub mod #section_ident {
            use super::*;

            #(#field_constants)*

            #(#nested_modules)*
        }
    }
}

/// Generate a single field constant.
fn generate_field_constant(section_name: &str, field_name: &str) -> TokenStream {
    let field_ident = format_ident!("{}", field_name);

    quote! {
        /// Field path for `#section_name.#field_name`
        #[allow(non_upper_case_globals)]
        pub fn #field_ident() -> FieldPath {
            FieldPath::new(&[#section_name, #field_name])
        }
    }
}

/// Generate nested accessors for a field that has a resolved struct type.
///
/// This handles cases like:
/// ```rust,ignore
/// pub struct OreRoundId {
///     pub round_id: u64,
///     pub round_address: Pubkey,
/// }
/// ```
///
/// Generates:
/// ```rust,ignore
/// pub mod round_id {
///     pub fn round_id() -> FieldPath { ... }
///     pub fn round_address() -> FieldPath { ... }
/// }
/// ```
fn generate_nested_accessors(
    parent_section: &str,
    field_name: &str,
    nested_fields: &[ResolvedField],
) -> TokenStream {
    let module_ident = format_ident!("{}", field_name);

    let field_constants: Vec<TokenStream> = nested_fields
        .iter()
        .map(|field| {
            let nested_field_ident = format_ident!("{}", field.field_name);
            let nested_field_name = &field.field_name;

            quote! {
                /// Field path for `#parent_section.#field_name.#nested_field_name`
                #[allow(non_upper_case_globals)]
                pub fn #nested_field_ident() -> FieldPath {
                    FieldPath::new(&[#parent_section, #field_name, #nested_field_name])
                }
            }
        })
        .collect();

    quote! {
        pub mod #module_ident {
            use super::*;

            #(#field_constants)*
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{BaseType, ResolvedStructType};

    #[test]
    fn test_generate_empty_sections() {
        let sections: Vec<EntitySection> = vec![];
        let output = generate_field_accessors(&sections);
        assert!(output.is_empty());
    }

    #[test]
    fn test_generate_simple_section() {
        let sections = vec![EntitySection {
            name: "id".to_string(),
            fields: vec![
                FieldTypeInfo {
                    field_name: "round_id".to_string(),
                    rust_type_name: "u64".to_string(),
                    base_type: BaseType::Integer,
                    is_optional: false,
                    is_array: false,
                    inner_type: None,
                    source_path: None,
                    resolved_type: None,
                    emit: true,
                },
                FieldTypeInfo {
                    field_name: "round_address".to_string(),
                    rust_type_name: "Pubkey".to_string(),
                    base_type: BaseType::Pubkey,
                    is_optional: false,
                    is_array: false,
                    inner_type: None,
                    source_path: None,
                    resolved_type: None,
                    emit: true,
                },
            ],
            is_nested_struct: false,
            parent_field: None,
        }];

        let output = generate_field_accessors(&sections);
        let output_str = output.to_string();

        assert!(output_str.contains("pub mod fields"));
        assert!(output_str.contains("pub mod id"));
        assert!(output_str.contains("round_id"));
        assert!(output_str.contains("round_address"));
    }

    #[test]
    fn test_generate_nested_accessors() {
        let sections = vec![EntitySection {
            name: "state".to_string(),
            fields: vec![FieldTypeInfo {
                field_name: "identity".to_string(),
                rust_type_name: "MyIdentity".to_string(),
                base_type: BaseType::Object,
                is_optional: false,
                is_array: false,
                inner_type: None,
                source_path: None,
                resolved_type: Some(ResolvedStructType {
                    type_name: "MyIdentity".to_string(),
                    fields: vec![
                        ResolvedField {
                            field_name: "owner".to_string(),
                            field_type: "Pubkey".to_string(),
                            base_type: BaseType::Pubkey,
                            is_optional: false,
                            is_array: false,
                        },
                        ResolvedField {
                            field_name: "created_at".to_string(),
                            field_type: "i64".to_string(),
                            base_type: BaseType::Timestamp,
                            is_optional: false,
                            is_array: false,
                        },
                    ],
                    is_instruction: false,
                    is_account: false,
                    is_event: false,
                    is_enum: false,
                    enum_variants: vec![],
                }),
                emit: true,
            }],
            is_nested_struct: false,
            parent_field: None,
        }];

        let output = generate_field_accessors(&sections);
        let output_str = output.to_string();

        assert!(output_str.contains("pub mod state"));
        assert!(output_str.contains("pub mod identity"));
        assert!(output_str.contains("owner"));
        assert!(output_str.contains("created_at"));
    }

    #[test]
    fn test_skip_root_section() {
        let sections = vec![
            EntitySection {
                name: "root".to_string(),
                fields: vec![FieldTypeInfo {
                    field_name: "some_root_field".to_string(),
                    rust_type_name: "u64".to_string(),
                    base_type: BaseType::Integer,
                    is_optional: false,
                    is_array: false,
                    inner_type: None,
                    source_path: None,
                    resolved_type: None,
                    emit: true,
                }],
                is_nested_struct: false,
                parent_field: None,
            },
            EntitySection {
                name: "id".to_string(),
                fields: vec![FieldTypeInfo {
                    field_name: "key".to_string(),
                    rust_type_name: "u64".to_string(),
                    base_type: BaseType::Integer,
                    is_optional: false,
                    is_array: false,
                    inner_type: None,
                    source_path: None,
                    resolved_type: None,
                    emit: true,
                }],
                is_nested_struct: false,
                parent_field: None,
            },
        ];

        let output = generate_field_accessors(&sections);
        let output_str = output.to_string();

        // Should NOT contain root section
        assert!(!output_str.contains("pub mod root"));
        assert!(!output_str.contains("some_root_field"));

        // Should contain id section
        assert!(output_str.contains("pub mod id"));
        assert!(output_str.contains("key"));
    }

    #[test]
    fn test_skip_enum_types() {
        let sections = vec![EntitySection {
            name: "state".to_string(),
            fields: vec![FieldTypeInfo {
                field_name: "status".to_string(),
                rust_type_name: "MyStatus".to_string(),
                base_type: BaseType::Object,
                is_optional: false,
                is_array: false,
                inner_type: None,
                source_path: None,
                resolved_type: Some(ResolvedStructType {
                    type_name: "MyStatus".to_string(),
                    fields: vec![],
                    is_instruction: false,
                    is_account: false,
                    is_event: false,
                    is_enum: true,
                    enum_variants: vec!["Active".to_string(), "Inactive".to_string()],
                }),
                emit: true,
            }],
            is_nested_struct: false,
            parent_field: None,
        }];

        let output = generate_field_accessors(&sections);
        let output_str = output.to_string();

        // Should have the status field accessor
        assert!(output_str.contains("status"));

        // Should NOT have a nested module for the enum
        // (enums don't have field accessors, just variants)
        assert!(!output_str.contains("Active"));
        assert!(!output_str.contains("Inactive"));
    }
}
