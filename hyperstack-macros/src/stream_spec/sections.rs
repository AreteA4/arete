//! Section processing for hyperstack streams.
//!
//! This module handles processing of nested struct sections, including:
//! - Section extraction from struct definitions
//! - Field type analysis
//! - Nested struct attribute processing

use std::collections::{HashMap, HashSet};

use quote::quote;
use syn::{Fields, ItemStruct, Type};

use crate::ast::{BaseType, EntitySection, FieldTypeInfo, ResolvedField, ResolvedStructType};
use crate::event_type_helpers::{find_idl_for_type, IdlLookup};
use crate::parse;
use crate::parse::idl::{IdlSpec, IdlType, IdlTypeDefKind};
use crate::utils::path_to_string;

use super::handlers::{determine_event_instruction, extract_account_type_from_field};
use super::resolve_snapshot_source;

// ============================================================================
// Section Extraction
// ============================================================================

/// Extract section information from a struct definition.
#[allow(dead_code)]
pub fn extract_section_from_struct(
    section_name: &str,
    item_struct: &ItemStruct,
    parent_field: Option<String>,
) -> EntitySection {
    extract_section_from_struct_with_idl(section_name, item_struct, parent_field, &[])
}

pub fn extract_section_from_struct_with_idl(
    section_name: &str,
    item_struct: &ItemStruct,
    parent_field: Option<String>,
    idls: IdlLookup,
) -> EntitySection {
    let mut fields = Vec::new();

    if let Fields::Named(struct_fields) = &item_struct.fields {
        for field in &struct_fields.named {
            if let Some(field_ident) = &field.ident {
                let field_name = field_ident.to_string();
                let field_ty = &field.ty;
                let rust_type_name = quote::quote!(#field_ty).to_string();
                let mut field_type_info =
                    analyze_field_type_with_idl(&field_name, &rust_type_name, idls);
                field_type_info.emit = field_emit_from_attrs(field, &field_name);
                fields.push(field_type_info);
            }
        }
    }

    EntitySection {
        name: section_name.to_string(),
        fields,
        is_nested_struct: parent_field.is_some(),
        parent_field,
    }
}

fn field_emit_from_attrs(field: &syn::Field, field_name: &str) -> bool {
    let mut found_mapping = false;
    let mut any_emit = false;

    for attr in &field.attrs {
        if let Ok(Some(map_attrs)) = parse::parse_map_attribute(attr, field_name) {
            found_mapping = true;
            if map_attrs.iter().any(|m| m.emit) {
                any_emit = true;
            }
        }

        if let Ok(Some(map_attrs)) = parse::parse_from_instruction_attribute(attr, field_name) {
            found_mapping = true;
            if map_attrs.iter().any(|m| m.emit) {
                any_emit = true;
            }
        }
    }

    if found_mapping {
        any_emit
    } else {
        true
    }
}

// ============================================================================
// Field Type Analysis
// ============================================================================

/// Analyze a Rust type string and extract field type information.
#[allow(dead_code)]
pub fn analyze_field_type(field_name: &str, rust_type: &str) -> FieldTypeInfo {
    analyze_field_type_with_idl(field_name, rust_type, &[])
}

/// Analyze a Rust type string with IDL support for resolving complex types.
pub fn analyze_field_type_with_idl(
    field_name: &str,
    rust_type: &str,
    idls: IdlLookup,
) -> FieldTypeInfo {
    let type_str = rust_type.trim();

    if let Some(inner) = extract_generic_inner_type(type_str, "Option") {
        let inner_info = analyze_inner_type(&inner);
        let resolved_type = if inner_info.0 == BaseType::Object {
            resolve_complex_type(&inner, idls)
        } else {
            None
        };

        return FieldTypeInfo {
            field_name: field_name.to_string(),
            rust_type_name: rust_type.to_string(),
            base_type: infer_semantic_type(field_name, inner_info.0),
            is_optional: true,
            is_array: inner_info.1,
            inner_type: Some(inner.clone()),
            source_path: None,
            resolved_type,
            emit: true,
        };
    }

    if let Some(inner) = extract_generic_inner_type(type_str, "Vec") {
        let inner_base_type = analyze_simple_type(&inner);
        let resolved_type = if inner_base_type == BaseType::Object {
            resolve_complex_type(&inner, idls)
        } else {
            None
        };

        return FieldTypeInfo {
            field_name: field_name.to_string(),
            rust_type_name: rust_type.to_string(),
            base_type: BaseType::Array,
            is_optional: false,
            is_array: true,
            inner_type: Some(inner.clone()),
            source_path: None,
            resolved_type,
            emit: true,
        };
    }

    let base_type = analyze_simple_type(type_str);
    let resolved_type = if base_type == BaseType::Object {
        resolve_complex_type(type_str, idls)
    } else {
        None
    };

    FieldTypeInfo {
        field_name: field_name.to_string(),
        rust_type_name: rust_type.to_string(),
        base_type: infer_semantic_type(field_name, base_type),
        is_optional: false,
        is_array: false,
        inner_type: None,
        source_path: None,
        resolved_type,
        emit: true,
    }
}

/// Analyze inner type and return (BaseType, is_array).
fn analyze_inner_type(type_str: &str) -> (BaseType, bool) {
    if let Some(_vec_inner) = extract_generic_inner_type(type_str, "Vec") {
        (BaseType::Array, true)
    } else {
        (analyze_simple_type(type_str), false)
    }
}

/// Analyze a simple (non-generic) type string.
fn analyze_simple_type(type_str: &str) -> BaseType {
    match type_str {
        "i8" | "i16" | "i32" | "i64" | "isize" | "u8" | "u16" | "u32" | "u64" | "usize" => {
            BaseType::Integer
        }
        "f32" | "f64" => BaseType::Float,
        "bool" => BaseType::Boolean,
        "String" | "&str" | "str" => BaseType::String,
        "Value" | "serde_json::Value" | "serde_json :: Value" => BaseType::Any,
        "Pubkey" | "solana_pubkey::Pubkey" | ":: solana_pubkey :: Pubkey" => BaseType::Pubkey,
        _ => {
            if type_str.contains("Bytes") || type_str.contains("bytes") {
                BaseType::Binary
            } else if type_str.contains("Pubkey") {
                BaseType::Pubkey
            } else {
                BaseType::Object
            }
        }
    }
}

/// Extract inner type from generic like "Option<T>" -> "T".
fn extract_generic_inner_type(type_str: &str, generic_name: &str) -> Option<String> {
    let pattern = format!("{} <", generic_name);
    let pattern_no_space = format!("{}<", generic_name);

    // Try with space (how quote renders it: "Option < u64 >")
    if type_str.starts_with(&pattern) && type_str.ends_with('>') {
        let start = pattern.len();
        let end = type_str.len() - 1;
        if end > start {
            return Some(type_str[start..end].trim().to_string());
        }
    }

    // Try without space (standard Rust syntax: "Option<u64>")
    if type_str.starts_with(&pattern_no_space) && type_str.ends_with('>') {
        let start = pattern_no_space.len();
        let end = type_str.len() - 1;
        if end > start {
            return Some(type_str[start..end].trim().to_string());
        }
    }

    None
}

/// Infer semantic type based on field name patterns.
fn infer_semantic_type(field_name: &str, base_type: BaseType) -> BaseType {
    let lower_name = field_name.to_lowercase();

    // If already classified as integer, check if it should be timestamp
    if base_type == BaseType::Integer
        && (lower_name.ends_with("_at")
            || lower_name.ends_with("_time")
            || lower_name.contains("timestamp")
            || lower_name.contains("created")
            || lower_name.contains("settled")
            || lower_name.contains("activated"))
    {
        return BaseType::Timestamp;
    }

    base_type
}

// ============================================================================
// Primitive Type Checking
// ============================================================================

/// Check if a type is a primitive or common wrapper type.
///
/// Returns true for numeric types, bool, String, Option, and Vec.
/// These types don't represent nested structs that need special processing.
pub fn is_primitive_or_wrapper(ty: &Type) -> bool {
    match ty {
        Type::Path(type_path) => {
            if let Some(segment) = type_path.path.segments.last() {
                let type_name = segment.ident.to_string();
                matches!(
                    type_name.as_str(),
                    "u8" | "u16"
                        | "u32"
                        | "u64"
                        | "u128"
                        | "i8"
                        | "i16"
                        | "i32"
                        | "i64"
                        | "i128"
                        | "f32"
                        | "f64"
                        | "bool"
                        | "String"
                        | "Option"
                        | "Vec"
                )
            } else {
                false
            }
        }
        _ => true,
    }
}

// ============================================================================
// Nested Struct Processing
// ============================================================================

/// Process a nested struct (section) and extract all its field mappings.
#[allow(clippy::too_many_arguments)]
pub fn process_nested_struct(
    nested_struct: &ItemStruct,
    section_field_name: &syn::Ident,
    section_field_type: &Type,
    state_fields: &mut Vec<proc_macro2::TokenStream>,
    accessor_defs: &mut Vec<proc_macro2::TokenStream>,
    accessor_names: &mut HashSet<String>,
    primary_keys: &mut Vec<String>,
    lookup_indexes: &mut Vec<(String, Option<String>)>,
    sources_by_type: &mut HashMap<String, Vec<parse::MapAttribute>>,
    field_mappings: &mut Vec<parse::MapAttribute>,
    events_by_instruction: &mut HashMap<String, Vec<(String, parse::EventAttribute, Type)>>,
    has_events: &mut bool,
    computed_fields: &mut Vec<(String, proc_macro2::TokenStream, Type)>,
    resolve_specs: &mut Vec<parse::ResolveSpec>,
    derive_from_mappings: &mut HashMap<String, Vec<parse::DeriveFromAttribute>>,
    aggregate_conditions: &mut HashMap<String, String>,
    program_name: Option<&str>,
) {
    let section_name = section_field_name.to_string();

    let mut nested_fields = Vec::new();

    if let Fields::Named(fields) = &nested_struct.fields {
        for field in &fields.named {
            let field_name = field.ident.as_ref().unwrap();
            let field_type = &field.ty;

            nested_fields.push(quote! {
                pub #field_name: #field_type
            });

            for attr in &field.attrs {
                if let Ok(Some(map_attrs)) =
                    parse::parse_map_attribute(attr, &field_name.to_string())
                {
                    for mut map_attr in map_attrs {
                        if !map_attr.target_field_name.contains('.') {
                            map_attr.target_field_name =
                                format!("{}.{}", section_name, map_attr.target_field_name);
                        }

                        if let Some(rt) = map_attr.resolver_transform.take() {
                            let visible_target = map_attr.target_field_name.clone();
                            let raw_field_name = format!("__{}_raw", field_name);
                            let raw_target = format!("{}.{}", section_name, raw_field_name);

                            map_attr.target_field_name = raw_target.clone();
                            map_attr.emit = false;

                            super::entity::process_map_attribute(
                                &map_attr,
                                field_name,
                                field_type,
                                &mut Vec::new(),
                                accessor_defs,
                                accessor_names,
                                primary_keys,
                                lookup_indexes,
                                sources_by_type,
                                field_mappings,
                            );

                            let raw_ref = raw_target.clone();
                            let computed_expr: proc_macro2::TokenStream = {
                                let raw_ident: proc_macro2::TokenStream =
                                    raw_ref.replace('.', " . ").parse().unwrap_or_default();
                                let method_ident: proc_macro2::TokenStream =
                                    rt.method.parse().unwrap_or_default();
                                let args = &rt.args;
                                quote::quote! { #raw_ident . #method_ident ( #args ) }
                            };

                            computed_fields.push((
                                visible_target,
                                computed_expr,
                                field_type.clone(),
                            ));
                        } else {
                            super::entity::process_map_attribute(
                                &map_attr,
                                field_name,
                                field_type,
                                &mut Vec::new(),
                                accessor_defs,
                                accessor_names,
                                primary_keys,
                                lookup_indexes,
                                sources_by_type,
                                field_mappings,
                            );
                        }
                    }
                } else if let Ok(Some(map_attrs)) =
                    parse::parse_from_instruction_attribute(attr, &field_name.to_string())
                {
                    for mut map_attr in map_attrs {
                        if !map_attr.target_field_name.contains('.') {
                            map_attr.target_field_name =
                                format!("{}.{}", section_name, map_attr.target_field_name);
                        }

                        if let Some(rt) = map_attr.resolver_transform.take() {
                            let visible_target = map_attr.target_field_name.clone();
                            let raw_field_name = format!("__{}_raw", field_name);
                            let raw_target = format!("{}.{}", section_name, raw_field_name);

                            map_attr.target_field_name = raw_target.clone();
                            map_attr.emit = false;

                            super::entity::process_map_attribute(
                                &map_attr,
                                field_name,
                                field_type,
                                &mut Vec::new(),
                                accessor_defs,
                                accessor_names,
                                primary_keys,
                                lookup_indexes,
                                sources_by_type,
                                field_mappings,
                            );

                            let raw_ref = raw_target.clone();
                            let computed_expr: proc_macro2::TokenStream = {
                                let raw_ident: proc_macro2::TokenStream =
                                    raw_ref.replace('.', " . ").parse().unwrap_or_default();
                                let method_ident: proc_macro2::TokenStream =
                                    rt.method.parse().unwrap_or_default();
                                let args = &rt.args;
                                quote::quote! { #raw_ident . #method_ident ( #args ) }
                            };

                            computed_fields.push((
                                visible_target,
                                computed_expr,
                                field_type.clone(),
                            ));
                            continue;
                        }

                        super::entity::process_map_attribute(
                            &map_attr,
                            field_name,
                            field_type,
                            &mut Vec::new(),
                            accessor_defs,
                            accessor_names,
                            primary_keys,
                            lookup_indexes,
                            sources_by_type,
                            field_mappings,
                        );
                    }
                } else if let Ok(Some(mut event_attr)) =
                    parse::parse_event_attribute(attr, &field_name.to_string())
                {
                    *has_events = true;

                    if !event_attr.target_field_name.contains('.') {
                        event_attr.target_field_name =
                            format!("{}.{}", section_name, event_attr.target_field_name);
                    }

                    // Determine instruction path (type-safe or legacy)
                    if let Some((_instruction_path, instruction_str)) =
                        determine_event_instruction(&mut event_attr, field_type, program_name)
                    {
                        events_by_instruction
                            .entry(instruction_str)
                            .or_default()
                            .push((
                                event_attr.target_field_name.clone(),
                                event_attr,
                                field_type.clone(),
                            ));
                    } else {
                        // Fallback to legacy instruction string
                        events_by_instruction
                            .entry(event_attr.instruction.clone())
                            .or_default()
                            .push((
                                event_attr.target_field_name.clone(),
                                event_attr,
                                field_type.clone(),
                            ));
                    }
                } else if let Ok(Some(mut snapshot_attr)) =
                    parse::parse_snapshot_attribute(attr, &field_name.to_string())
                {
                    // Add section prefix if needed
                    if !snapshot_attr.target_field_name.contains('.') {
                        snapshot_attr.target_field_name =
                            format!("{}.{}", section_name, snapshot_attr.target_field_name);
                    }

                    // Infer account type from field type if not explicitly specified
                    let account_path = if let Some(ref path) = snapshot_attr.from_account {
                        Some(path.clone())
                    } else if let Some(inferred_path) = extract_account_type_from_field(field_type)
                    {
                        snapshot_attr.inferred_account = Some(inferred_path.clone());
                        Some(inferred_path)
                    } else {
                        None
                    };

                    if let Some(acct_path) = account_path {
                        let source_type_str = path_to_string(&acct_path);

                        // Determine source field name and whether this is a whole-source capture
                        let (source_field_name, is_whole_source) =
                            resolve_snapshot_source(&snapshot_attr);

                        let map_attr = parse::MapAttribute {
                            source_type_path: acct_path,
                            source_field_name,
                            target_field_name: snapshot_attr.target_field_name.clone(),
                            is_primary_key: false,
                            is_lookup_index: false,
                            register_from: Vec::new(),
                            temporal_field: None,
                            strategy: snapshot_attr.strategy.clone(),
                            join_on: snapshot_attr
                                .join_on
                                .as_ref()
                                .map(|fs| fs.ident.to_string()),
                            transform: None,
                            resolver_transform: None,
                            is_instruction: false,
                            is_whole_source,
                            lookup_by: snapshot_attr.lookup_by.clone(),
                            condition: None,
                            when: snapshot_attr.when.clone(),
                            stop: None,
                            stop_lookup_by: None,
                            emit: true,
                        };

                        sources_by_type
                            .entry(source_type_str)
                            .or_default()
                            .push(map_attr);
                    }
                } else if let Ok(Some(mut aggr_attr)) =
                    parse::parse_aggregate_attribute(attr, &field_name.to_string())
                {
                    // Add section prefix if needed
                    if !aggr_attr.target_field_name.contains('.') {
                        aggr_attr.target_field_name =
                            format!("{}.{}", section_name, aggr_attr.target_field_name);
                    }

                    // Store condition for later AST generation (with section prefix)
                    if let Some(condition) = &aggr_attr.condition {
                        aggregate_conditions
                            .insert(aggr_attr.target_field_name.clone(), condition.clone());
                    }

                    // Convert aggregate to map attributes for each instruction
                    for instr_path in &aggr_attr.from_instructions {
                        let source_field_name = aggr_attr
                            .field
                            .as_ref()
                            .map(|fs| fs.ident.to_string())
                            .unwrap_or_default();

                        let map_attr = parse::MapAttribute {
                            source_type_path: instr_path.clone(),
                            source_field_name,
                            target_field_name: aggr_attr.target_field_name.clone(),
                            is_primary_key: false,
                            is_lookup_index: false,
                            register_from: Vec::new(),
                            temporal_field: None,
                            strategy: aggr_attr.strategy.clone(),
                            join_on: aggr_attr.join_on.as_ref().map(|fs| fs.ident.to_string()),
                            transform: aggr_attr.transform.as_ref().map(|t| t.to_string()),
                            resolver_transform: None,
                            is_instruction: true,
                            is_whole_source: false,
                            lookup_by: aggr_attr.lookup_by.clone(),
                            condition: None,
                            when: None,
                            stop: None,
                            stop_lookup_by: None,
                            emit: true,
                        };

                        // Add to sources_by_type for handler generation
                        let source_type_str = path_to_string(instr_path);
                        sources_by_type
                            .entry(source_type_str)
                            .or_default()
                            .push(map_attr);
                    }
                } else if let Ok(Some(mut derive_attr)) =
                    parse::parse_derive_from_attribute(attr, &field_name.to_string())
                {
                    // Add section prefix if needed
                    if !derive_attr.target_field_name.contains('.') {
                        derive_attr.target_field_name =
                            format!("{}.{}", section_name, derive_attr.target_field_name);
                    }

                    // Group by instruction for handler merging
                    for instr_path in &derive_attr.from_instructions {
                        let source_type_str = path_to_string(instr_path);
                        derive_from_mappings
                            .entry(source_type_str)
                            .or_default()
                            .push(derive_attr.clone());
                    }
                } else if let Ok(Some(mut computed_attr)) =
                    parse::parse_computed_attribute(attr, &field_name.to_string())
                {
                    // Add section prefix if needed
                    if !computed_attr.target_field_name.contains('.') {
                        computed_attr.target_field_name =
                            format!("{}.{}", section_name, computed_attr.target_field_name);
                    }

                    // Store computed field for later processing (after aggregations)
                    computed_fields.push((
                        computed_attr.target_field_name.clone(),
                        computed_attr.expression.clone(),
                        field_type.clone(),
                    ));
                } else if let Ok(Some(resolve_attr)) =
                    parse::parse_resolve_attribute(attr, &field_name.to_string())
                {
                    // Determine resolver type: URL resolver if url is present, otherwise Token resolver
                    let qualified_url = resolve_attr.url.as_deref().map(|url_path_raw| {
                        if url_path_raw.contains('.') {
                            url_path_raw.to_string()
                        } else {
                            format!("{}.{}", section_name, url_path_raw)
                        }
                    });

                    let resolver = if let Some(ref _url_path) = qualified_url {
                        let method = resolve_attr
                            .method
                            .as_deref()
                            .map(|m| match m.to_lowercase().as_str() {
                                "post" => crate::ast::HttpMethod::Post,
                                _ => crate::ast::HttpMethod::Get,
                            })
                            .unwrap_or(crate::ast::HttpMethod::Get);

                        let url_source = if resolve_attr.url_is_template {
                            crate::ast::UrlSource::Template(super::entity::parse_url_template(
                                resolve_attr.url.as_deref().unwrap(),
                            ))
                        } else {
                            let url_path_raw = resolve_attr.url.as_deref().unwrap();
                            let qualified = if url_path_raw.contains('.') {
                                url_path_raw.to_string()
                            } else {
                                format!("{}.{}", section_name, url_path_raw)
                            };
                            crate::ast::UrlSource::FieldPath(qualified)
                        };

                        crate::ast::ResolverType::Url(crate::ast::UrlResolverConfig {
                            url_source,
                            method,
                            extract_path: resolve_attr.extract.clone(),
                        })
                    } else if let Some(name) = resolve_attr.resolver.as_deref() {
                        super::entity::parse_resolver_type_name(name, field_type)
                            .unwrap_or_else(|err| panic!("{}", err))
                    } else {
                        super::entity::infer_resolver_type(field_type)
                            .unwrap_or_else(|err| panic!("{}", err))
                    };

                    let mut target_field_name = resolve_attr.target_field_name;
                    if !target_field_name.contains('.') {
                        target_field_name = format!("{}.{}", section_name, target_field_name);
                    }

                    let from = if resolve_attr.url_is_template {
                        None
                    } else {
                        let qualified_url = resolve_attr.url.as_deref().map(|url_path_raw| {
                            if url_path_raw.contains('.') {
                                url_path_raw.to_string()
                            } else {
                                format!("{}.{}", section_name, url_path_raw)
                            }
                        });
                        qualified_url.or(resolve_attr.from)
                    };

                    resolve_specs.push(parse::ResolveSpec {
                        resolver,
                        from,
                        address: resolve_attr.address,
                        extract: resolve_attr.extract,
                        target_field_name,
                        strategy: resolve_attr.strategy,
                        condition: resolve_attr.condition,
                        schedule_at: resolve_attr.schedule_at,
                    });
                }
            }
        }
    }

    state_fields.push(quote! {
        pub #section_field_name: #section_field_type
    });
}

// ============================================================================
// IDL Type Resolution
// ============================================================================

fn resolve_complex_type(type_str: &str, idls: IdlLookup) -> Option<ResolvedStructType> {
    let idl_ref = find_idl_for_type(type_str, idls)?;

    // Extract the simple type name from patterns like "generated_sdk :: instructions :: Buy"
    let type_name = extract_type_name(type_str);
    let type_name_lower = type_name.to_lowercase();

    // Check if it's an instruction (case-insensitive match)
    let idl = Some(idl_ref);

    for instruction in &idl_ref.instructions {
        if instruction.name.to_lowercase() == type_name_lower {
            return Some(resolve_instruction_type(instruction, idl));
        }
    }

    for account in &idl_ref.accounts {
        if account.name.to_lowercase() == type_name_lower {
            let resolved = resolve_account_type(account, idl);
            if !resolved.fields.is_empty() || resolved.is_enum {
                return Some(resolved);
            }
            break;
        }
    }

    for type_def in &idl_ref.types {
        if type_def.name.to_lowercase() == type_name_lower {
            return Some(resolve_custom_type(type_def, idl));
        }
    }

    None
}

/// Extract simple type name from a qualified path like "generated_sdk :: instructions :: Buy" -> "Buy"
fn extract_type_name(type_str: &str) -> String {
    type_str
        .split("::")
        .last()
        .unwrap_or(type_str)
        .trim()
        .to_string()
}

/// Resolve an instruction type from IDL
fn resolve_instruction_type(
    instruction: &crate::parse::idl::IdlInstruction,
    idl: Option<&IdlSpec>,
) -> ResolvedStructType {
    let mut fields = Vec::new();

    // Add account fields
    for account in &instruction.accounts {
        fields.push(ResolvedField {
            field_name: account.name.clone(),
            field_type: "Pubkey".to_string(),
            base_type: BaseType::Pubkey,
            is_optional: account.optional,
            is_array: false,
        });
    }

    // Add data/arg fields
    for arg in &instruction.args {
        let (field_type, base_type, is_optional, is_array, _) =
            analyze_idl_type_with_resolution(&arg.type_, idl);
        fields.push(ResolvedField {
            field_name: arg.name.clone(),
            field_type,
            base_type,
            is_optional,
            is_array,
        });
    }

    ResolvedStructType {
        type_name: instruction.name.clone(),
        fields,
        is_instruction: true,
        is_account: false,
        is_event: false,
        is_enum: false,
        enum_variants: Vec::new(),
    }
}

/// Resolve an account type from IDL
fn resolve_account_type(
    account: &crate::parse::idl::IdlAccount,
    idl: Option<&IdlSpec>,
) -> ResolvedStructType {
    let mut fields = Vec::new();

    // Extract fields from embedded type definition (Steel format)
    if let Some(type_def) = &account.type_def {
        match type_def {
            IdlTypeDefKind::Struct {
                fields: struct_fields,
                ..
            } => {
                for field in struct_fields {
                    let (field_type, base_type, is_optional, is_array, _) =
                        analyze_idl_type_with_resolution(&field.type_, idl);
                    fields.push(ResolvedField {
                        field_name: field.name.clone(),
                        field_type,
                        base_type,
                        is_optional,
                        is_array,
                    });
                }
            }
            IdlTypeDefKind::TupleStruct {
                fields: tuple_fields,
                ..
            } => {
                for (i, field_type) in tuple_fields.iter().enumerate() {
                    let (field_type_str, base_type, is_optional, is_array, _) =
                        analyze_idl_type_with_resolution(field_type, idl);
                    fields.push(ResolvedField {
                        field_name: format!("_{}", i),
                        field_type: field_type_str,
                        base_type,
                        is_optional,
                        is_array,
                    });
                }
            }
            IdlTypeDefKind::Enum { variants, .. } => {
                // Enums: extract variant names
                let variant_names: Vec<String> = variants.iter().map(|v| v.name.clone()).collect();
                return ResolvedStructType {
                    type_name: account.name.clone(),
                    fields: Vec::new(),
                    is_instruction: false,
                    is_account: true,
                    is_event: false,
                    is_enum: true,
                    enum_variants: variant_names,
                };
            }
        }
    }

    ResolvedStructType {
        type_name: account.name.clone(),
        fields,
        is_instruction: false,
        is_account: true,
        is_event: false,
        is_enum: false,
        enum_variants: Vec::new(),
    }
}

/// Resolve a custom type definition from IDL
fn resolve_custom_type(
    type_def: &crate::parse::idl::IdlTypeDef,
    idl: Option<&IdlSpec>,
) -> ResolvedStructType {
    let mut fields = Vec::new();

    match &type_def.type_def {
        IdlTypeDefKind::Struct {
            fields: struct_fields,
            ..
        } => {
            for field in struct_fields {
                let (field_type, base_type, is_optional, is_array, _) =
                    analyze_idl_type_with_resolution(&field.type_, idl);
                fields.push(ResolvedField {
                    field_name: field.name.clone(),
                    field_type,
                    base_type,
                    is_optional,
                    is_array,
                });
            }

            ResolvedStructType {
                type_name: type_def.name.clone(),
                fields,
                is_instruction: false,
                is_account: false,
                is_event: false,
                is_enum: false,
                enum_variants: Vec::new(),
            }
        }
        IdlTypeDefKind::TupleStruct {
            fields: tuple_fields,
            ..
        } => {
            for (i, field_type) in tuple_fields.iter().enumerate() {
                let (field_type_str, base_type, is_optional, is_array, _) =
                    analyze_idl_type_with_resolution(field_type, idl);
                fields.push(ResolvedField {
                    field_name: format!("_{}", i),
                    field_type: field_type_str,
                    base_type,
                    is_optional,
                    is_array,
                });
            }

            ResolvedStructType {
                type_name: type_def.name.clone(),
                fields,
                is_instruction: false,
                is_account: false,
                is_event: false,
                is_enum: false,
                enum_variants: Vec::new(),
            }
        }
        IdlTypeDefKind::Enum { variants, .. } => {
            // Enums: extract variant names
            let variant_names: Vec<String> = variants.iter().map(|v| v.name.clone()).collect();

            ResolvedStructType {
                type_name: type_def.name.clone(),
                fields: Vec::new(),
                is_instruction: false,
                is_account: false,
                is_event: false,
                is_enum: true,
                enum_variants: variant_names,
            }
        }
    }
}

/// Analyze an IDL type and return (type_string, base_type, is_optional, is_array)
/// Analyze IDL type with optional resolution and return (type_name, base_type, is_optional, is_array, resolved_type)
fn analyze_idl_type_with_resolution(
    idl_type: &IdlType,
    idl: Option<&IdlSpec>,
) -> (String, BaseType, bool, bool, Option<ResolvedStructType>) {
    match idl_type {
        IdlType::Simple(s) => {
            let base_type = match s.as_str() {
                "u8" | "u16" | "u32" | "u64" | "u128" | "i8" | "i16" | "i32" | "i64" | "i128" => {
                    BaseType::Integer
                }
                "f32" | "f64" => BaseType::Float,
                "bool" => BaseType::Boolean,
                "string" => BaseType::String,
                "publicKey" | "pubkey" => BaseType::Pubkey,
                "bytes" => BaseType::Binary,
                _ => BaseType::Object,
            };
            (s.clone(), base_type, false, false, None)
        }
        IdlType::Option(opt) => {
            let (inner_type, base_type, _, is_array, resolved_type) =
                analyze_idl_type_with_resolution(&opt.option, idl);
            (
                format!("Option<{}>", inner_type),
                base_type,
                true,
                is_array,
                resolved_type,
            )
        }
        IdlType::Vec(vec) => {
            let (inner_type, base_type, is_optional, _, resolved_type) =
                analyze_idl_type_with_resolution(&vec.vec, idl);
            (
                format!("Vec<{}>", inner_type),
                base_type,
                is_optional,
                true,
                resolved_type,
            )
        }
        IdlType::Array(arr) => {
            // Fixed-size arrays like [u64; 25] or [u8; 32]
            if arr.array.len() >= 2 {
                // First element is the type, second is the size
                match &arr.array[0] {
                    crate::parse::idl::IdlTypeArrayElement::Type(ty) => {
                        // Map the element type to base type
                        let element_base_type = match ty.as_str() {
                            "u8" | "u16" | "u32" | "u64" | "u128" | "i8" | "i16" | "i32"
                            | "i64" | "i128" => BaseType::Integer,
                            "f32" | "f64" => BaseType::Float,
                            "bool" => BaseType::Boolean,
                            "string" => BaseType::String,
                            "publicKey" | "pubkey" => BaseType::Pubkey,
                            "bytes" => BaseType::Binary,
                            _ => BaseType::Object,
                        };
                        // Return as array with the element's base type
                        (format!("[{}]", ty), element_base_type, false, true, None)
                    }
                    crate::parse::idl::IdlTypeArrayElement::Nested(nested_type) => {
                        // Handle nested types in arrays
                        let (inner_type, base_type, is_optional, _, resolved_type) =
                            analyze_idl_type_with_resolution(nested_type, idl);
                        (
                            format!("[{}]", inner_type),
                            base_type,
                            is_optional,
                            true,
                            resolved_type,
                        )
                    }
                    _ => ("Array".to_string(), BaseType::Array, false, true, None),
                }
            } else {
                ("Array".to_string(), BaseType::Array, false, true, None)
            }
        }
        IdlType::Defined(def) => {
            let type_name = match &def.defined {
                crate::parse::idl::IdlTypeDefinedInner::Named { name } => name.clone(),
                crate::parse::idl::IdlTypeDefinedInner::Simple(s) => s.clone(),
            };

            let temp_idl_lookup: Vec<(String, &IdlSpec)> =
                idl.into_iter().map(|i| (String::new(), i)).collect();
            let resolved_type = resolve_complex_type(&type_name, &temp_idl_lookup);

            (type_name, BaseType::Object, false, false, resolved_type)
        }
        IdlType::HashMap(hm) => {
            let (val_type, base_type, _, _, resolved_type) =
                analyze_idl_type_with_resolution(&hm.hash_map.1, idl);
            (
                format!("HashMap<{}>", val_type),
                base_type,
                false,
                false,
                resolved_type,
            )
        }
    }
}

#[allow(dead_code)]
fn analyze_idl_type(idl_type: &IdlType) -> (String, BaseType, bool, bool) {
    let (type_name, base_type, is_optional, is_array, _) =
        analyze_idl_type_with_resolution(idl_type, None);
    (type_name, base_type, is_optional, is_array)
}
