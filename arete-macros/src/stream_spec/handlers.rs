//! Handler generation for stream specs.
//!
//! This module handles the generation of event and resolver handlers, including:
//! - Converting EventAttribute to MapAttribute for handler merging
//! - Generating resolver functions from declarative attributes
//! - Generating PDA registration functions
//! - Processing event fields for mapping

use quote::{format_ident, quote};
use syn::spanned::Spanned;
use syn::{Path, Type};

use crate::ast::{ResolverHook, ResolverStrategy};
use crate::diagnostic::idl_error_to_syn;
use crate::parse;
use crate::parse::idl as idl_parser;
use crate::utils::{path_to_string, to_snake_case};
use arete_idl::error::IdlSearchError;
use arete_idl::search::{lookup_instruction_field, InstructionFieldKind};

// ============================================================================
// Type Extraction Helpers
// ============================================================================

/// Extract account type from field type (e.g., Option<generated_sdk::accounts::BondingCurve> -> BondingCurve path).
pub fn extract_account_type_from_field(field_type: &Type) -> Option<Path> {
    match field_type {
        Type::Path(type_path) => {
            if let Some(segment) = type_path.path.segments.last() {
                let type_name = segment.ident.to_string();

                // Handle Option<AccountType>
                if type_name == "Option" {
                    if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
                        if let Some(syn::GenericArgument::Type(Type::Path(inner_type))) =
                            args.args.first()
                        {
                            let extracted_path = &inner_type.path;
                            // Only return if it looks like an account type (has multiple segments)
                            if is_likely_account_path(extracted_path) {
                                return Some(extracted_path.clone());
                            }
                        }
                    }
                }

                // Direct AccountType (no Option wrapper)
                if is_likely_account_path(&type_path.path) {
                    return Some(type_path.path.clone());
                }
            }
            None
        }
        _ => None,
    }
}

/// Check if a path looks like an account type path.
fn is_likely_account_path(path: &Path) -> bool {
    // Check that it has multiple segments (e.g., generated_sdk::accounts::BondingCurve)
    if path.segments.len() < 2 {
        return false;
    }

    // Check if it contains "accounts" in the path
    let path_str = path_to_string(path);
    if path_str.contains("::accounts::") {
        return true;
    }

    // Check that it's not a common non-account type
    let last_segment = path.segments.last().unwrap().ident.to_string();
    let excluded_types = [
        "Value", "String", "u64", "u32", "i64", "i32", "bool", "Vec", "Option", "HashMap",
        "BTreeMap",
    ];

    !excluded_types.contains(&last_segment.as_str())
}

/// Extract instruction type from field type (e.g., Vec<generated_sdk::instructions::Buy> -> Buy path).
pub fn extract_instruction_type_from_field(field_type: &Type) -> Option<Path> {
    match field_type {
        Type::Path(type_path) => {
            if let Some(segment) = type_path.path.segments.last() {
                let type_name = segment.ident.to_string();

                // Handle Vec<InstructionType>
                if type_name == "Vec" {
                    if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
                        if let Some(syn::GenericArgument::Type(Type::Path(inner_type))) =
                            args.args.first()
                        {
                            let extracted_path = &inner_type.path;
                            // Only return if it looks like an instruction type (has multiple segments and doesn't look like a primitive)
                            if is_likely_instruction_path(extracted_path) {
                                return Some(extracted_path.clone());
                            }
                        }
                    }
                }

                // Handle Option<InstructionType>
                if type_name == "Option" {
                    if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
                        if let Some(syn::GenericArgument::Type(Type::Path(inner_type))) =
                            args.args.first()
                        {
                            let extracted_path = &inner_type.path;
                            // Only return if it looks like an instruction type (has multiple segments and doesn't look like a primitive)
                            if is_likely_instruction_path(extracted_path) {
                                return Some(extracted_path.clone());
                            }
                        }
                    }
                }
            }
            None
        }
        _ => None,
    }
}

/// Check if a path looks like an instruction type path (not a primitive type).
fn is_likely_instruction_path(path: &Path) -> bool {
    // Check that it has multiple segments (e.g., generated_sdk::instructions::Buy)
    if path.segments.len() < 2 {
        return false;
    }

    // Check that it's not a common non-instruction type
    let last_segment = path.segments.last().unwrap().ident.to_string();
    let excluded_types = [
        "Value", "String", "u64", "u32", "i64", "i32", "bool", "Vec", "Option", "HashMap",
        "BTreeMap",
    ];

    !excluded_types.contains(&last_segment.as_str())
}

// ============================================================================
// IDL Field Location Helpers
// ============================================================================

/// Find field in IDL instruction and determine if it's an arg or account.
pub fn find_field_in_instruction(
    instruction_path: &Path,
    field_name: &str,
    idl: Option<&idl_parser::IdlSpec>,
) -> Result<parse::FieldLocation, IdlSearchError> {
    let idl = match idl {
        Some(idl) => idl,
        None => return Ok(parse::FieldLocation::InstructionArg), // Default to arg if no IDL
    };

    // Extract instruction name from path (e.g., "Buy" from "generated_sdk::instructions::Buy")
    let instruction_name = instruction_path
        .segments
        .last()
        .map(|s| s.ident.to_string())
        .ok_or_else(|| IdlSearchError::InvalidPath {
            path: path_to_string(instruction_path),
        })?;
    match lookup_instruction_field(idl, &instruction_name, field_name)?.kind {
        InstructionFieldKind::Account => Ok(parse::FieldLocation::Account),
        InstructionFieldKind::Arg => Ok(parse::FieldLocation::InstructionArg),
    }
}

// ============================================================================
// Event Instruction Determination
// ============================================================================

/// Determine the instruction path for an event attribute.
///
/// Returns (instruction_path_for_codegen, instruction_string_for_backward_compat).
pub fn determine_event_instruction(
    event_attr: &mut parse::EventAttribute,
    field_type: &Type,
    program_name: Option<&str>,
) -> Option<(Path, String)> {
    // Priority 1: Explicit `from = ...`
    if let Some(ref path) = event_attr.from_instruction {
        let path_str = path_to_string(path);
        // Convert to program::Instruction format
        let parts: Vec<&str> = path_str.split("::").collect();
        if parts.len() >= 2 {
            let program = parts[parts.len() - 2];
            let instruction = parts[parts.len() - 1];
            return Some((path.clone(), format!("{}::{}", program, instruction)));
        }
        return Some((path.clone(), path_str));
    }

    // Priority 2: Inferred from field type
    if let Some(inferred_path) = extract_instruction_type_from_field(field_type) {
        event_attr.inferred_instruction = Some(inferred_path.clone());
        let path_str = path_to_string(&inferred_path);
        let parts: Vec<&str> = path_str.split("::").collect();
        if parts.len() >= 2 {
            let program = parts[parts.len() - 2];
            let instruction = parts[parts.len() - 1];
            return Some((inferred_path, format!("{}::{}", program, instruction)));
        }
        return Some((inferred_path, path_str));
    }

    // Priority 3: Legacy string-based instruction
    if !event_attr.instruction.is_empty() {
        // Try to construct a path from the string
        // e.g., "pump::Buy" -> generated_sdk::instructions::Buy
        let parts: Vec<&str> = event_attr.instruction.split("::").collect();
        if parts.len() == 2 {
            let instruction_name = parts[1];
            // Construct a simple path - this is a best-effort approach
            let path_str = if let Some(program_name) = program_name {
                format!("{}_sdk::instructions::{}", program_name, instruction_name)
            } else {
                format!("generated_sdk::instructions::{}", instruction_name)
            };
            if let Ok(path) = syn::parse_str::<Path>(&path_str) {
                return Some((path, event_attr.instruction.clone()));
            }
        }
    }

    None
}

// ============================================================================
// Event Field Helpers
// ============================================================================

/// Get the lookup_by field as a string, handling both FieldSpec and legacy string.
#[allow(dead_code)]
pub fn get_lookup_by_field(lookup_by: &Option<parse::FieldSpec>) -> Option<String> {
    lookup_by
        .as_ref()
        .map(|field_spec| field_spec.ident.to_string())
}

/// Get the join_on field as a string, handling both FieldSpec and legacy string.
pub fn get_join_on_field(join_on: &Option<parse::FieldSpec>) -> Option<String> {
    join_on
        .as_ref()
        .map(|field_spec| field_spec.ident.to_string())
}

// ============================================================================
// Event to Map Attribute Conversion
// ============================================================================

/// Convert an EventAttribute to MapAttributes for handler merging.
pub fn convert_event_to_map_attributes(
    target_field: &str,
    event_attr: &parse::EventAttribute,
    instruction_path: &syn::Path,
    _idl: Option<&idl_parser::IdlSpec>,
) -> Vec<parse::MapAttribute> {
    let mut map_attrs = Vec::new();

    // Check if this is a whole source capture (no specific fields)
    let has_fields =
        !event_attr.capture_fields.is_empty() || !event_attr.capture_fields_legacy.is_empty();

    if !has_fields {
        // Whole instruction capture - create a single mapping for the whole source
        map_attrs.push(parse::MapAttribute {
            attr_span: event_attr.attr_span,
            source_type_span: instruction_path.span(),
            source_field_span: event_attr.attr_span,
            is_event_source: true,
            is_account_source: false,
            source_type_path: instruction_path.clone(),
            source_field_name: String::new(),
            target_field_name: target_field.to_string(),
            is_primary_key: false,
            is_lookup_index: false,
            register_from: Vec::new(),
            temporal_field: None,
            strategy: event_attr.strategy.clone(),
            join_on: event_attr.join_on.clone(),
            transform: None,
            resolver_transform: None,
            is_instruction: true,
            is_whole_source: true,
            lookup_by: event_attr.lookup_by.clone(),
            condition: None,
            when: None,
            stop: None,
            stop_lookup_by: None,
            emit: true,
        });
        return map_attrs;
    }

    // If event has specific fields (type-safe), create one MapAttribute per field
    for field_spec in &event_attr.capture_fields {
        let field_name = field_spec.ident.to_string();
        let transform = event_attr
            .field_transforms
            .get(&field_name)
            .map(|t| t.to_string());

        map_attrs.push(parse::MapAttribute {
            attr_span: event_attr.attr_span,
            source_type_span: instruction_path.span(),
            source_field_span: field_spec.ident.span(),
            is_event_source: true,
            is_account_source: false,
            source_type_path: instruction_path.clone(),
            source_field_name: field_name.clone(),
            target_field_name: format!("{}.{}", target_field, field_name),
            is_primary_key: false,
            is_lookup_index: false,
            register_from: Vec::new(),
            temporal_field: None,
            strategy: event_attr.strategy.clone(),
            join_on: event_attr.join_on.clone(),
            transform,
            resolver_transform: None,
            is_instruction: true,
            is_whole_source: false,
            lookup_by: event_attr.lookup_by.clone(),
            condition: None,
            when: None,
            stop: None,
            stop_lookup_by: None,
            emit: true,
        });
    }

    // Process legacy string-based fields
    for field_name in &event_attr.capture_fields_legacy {
        let transform = event_attr
            .field_transforms_legacy
            .get(field_name)
            .map(|t| t.to_string());

        map_attrs.push(parse::MapAttribute {
            attr_span: event_attr.attr_span,
            source_type_span: instruction_path.span(),
            source_field_span: event_attr.attr_span,
            is_event_source: true,
            is_account_source: false,
            source_type_path: instruction_path.clone(),
            source_field_name: field_name.clone(),
            target_field_name: format!("{}.{}", target_field, field_name),
            is_primary_key: false,
            is_lookup_index: false,
            register_from: Vec::new(),
            temporal_field: None,
            strategy: event_attr.strategy.clone(),
            join_on: event_attr.join_on.clone(),
            transform,
            resolver_transform: None,
            is_instruction: true,
            is_whole_source: false,
            lookup_by: event_attr.lookup_by.clone(),
            condition: None,
            when: None,
            stop: None,
            stop_lookup_by: None,
            emit: true,
        });
    }

    map_attrs
}

/// Process event fields for mapping - handles both new type-safe and legacy string syntax.
#[allow(dead_code)]
pub fn process_event_fields_for_mapping(
    event_attr: &parse::EventAttribute,
    instruction_path: Option<&Path>,
    idl: Option<&idl_parser::IdlSpec>,
) -> Vec<proc_macro2::TokenStream> {
    let mut captured_fields = Vec::new();

    // Check if we're using new type-safe fields or legacy string fields
    if !event_attr.capture_fields.is_empty() {
        // New type-safe syntax
        for field_spec in &event_attr.capture_fields {
            let field_name = field_spec.ident.to_string();

            // Determine the field location (accounts vs data)
            let field_location = if let Some(explicit_loc) = &field_spec.explicit_location {
                explicit_loc.clone()
            } else if let Some(instr_path) = instruction_path {
                // Try to find in IDL
                find_field_in_instruction(instr_path, &field_name, idl)
                    .unwrap_or(parse::FieldLocation::InstructionArg)
            } else {
                parse::FieldLocation::InstructionArg
            };

            // Generate field path based on location
            let field_path = match field_location {
                parse::FieldLocation::Account => vec!["accounts", &field_name],
                parse::FieldLocation::InstructionArg => vec!["data", &field_name],
            };

            // Check for transforms
            if let Some(transform_ident) = event_attr.field_transforms.get(&field_name) {
                captured_fields.push(quote! {
                    Box::new(arete::runtime::arete_interpreter::ast::MappingSource::FromSource {
                        path: arete::runtime::arete_interpreter::ast::FieldPath::new(&[#(#field_path),*]),
                        default: None,
                        transform: Some(arete::runtime::arete_interpreter::ast::Transformation::#transform_ident),
                    })
                });
            } else {
                captured_fields.push(quote! {
                    Box::new(arete::runtime::arete_interpreter::ast::MappingSource::FromSource {
                        path: arete::runtime::arete_interpreter::ast::FieldPath::new(&[#(#field_path),*]),
                        default: None,
                        transform: None,
                    })
                });
            }
        }
    } else if !event_attr.capture_fields_legacy.is_empty() {
        // Legacy string-based syntax - all fields are assumed to be in "data"
        for field_name in &event_attr.capture_fields_legacy {
            if let Some(transform_str) = event_attr.field_transforms_legacy.get(field_name) {
                let transform_ident = format_ident!("{}", transform_str);
                captured_fields.push(quote! {
                    Box::new(arete::runtime::arete_interpreter::ast::MappingSource::FromSource {
                        path: arete::runtime::arete_interpreter::ast::FieldPath::new(&["data", #field_name]),
                        default: None,
                        transform: Some(arete::runtime::arete_interpreter::ast::Transformation::#transform_ident),
                    })
                });
            } else {
                captured_fields.push(quote! {
                    Box::new(arete::runtime::arete_interpreter::ast::MappingSource::FromSource {
                        path: arete::runtime::arete_interpreter::ast::FieldPath::new(&["data", #field_name]),
                        default: None,
                        transform: None,
                    })
                });
            }
        }
    }

    captured_fields
}

// ============================================================================
// Resolver and PDA Function Generation
// ============================================================================

/// Generate #[resolve_key_for] functions from declarative #[resolve_key] attributes.
pub fn generate_resolver_functions(
    resolver_hooks: &[parse::ResolveKeyAttribute],
    idl: Option<&idl_parser::IdlSpec>,
) -> proc_macro2::TokenStream {
    let mut functions = Vec::new();

    for hook in resolver_hooks {
        let _account_type = &hook.account_path;
        let account_name = hook
            .account_path
            .segments
            .last()
            .map(|seg| seg.ident.to_string())
            .unwrap_or_else(|| "unknown".to_string());
        let fn_name = format_ident!("resolve_{}_key", to_snake_case(&account_name));

        match hook.strategy.as_str() {
            "pda_reverse_lookup" => {
                // Extract discriminators from queue_until instruction paths
                let mut disc_bytes: Vec<u8> = Vec::new();

                if let Some(idl) = idl {
                    for instr_path in &hook.queue_until {
                        if let Some(instr_name) = instr_path.segments.last() {
                            let instr_name_str = instr_name.ident.to_string();
                            if let Some(discriminator) =
                                idl.get_instruction_discriminator(&instr_name_str)
                            {
                                disc_bytes.extend_from_slice(&discriminator);
                            }
                        }
                    }
                }

                // Note: We do NOT emit #[resolve_key_for] attribute here because:
                // 1. These functions are generated during macro expansion
                // 2. Attributes need to be registered in the AST/bytecode system instead
                // 3. The resolver registry is built separately from the AST
                functions.push(quote! {
                    pub fn #fn_name(
                        account_address: &str,
                        _account_data: &arete::runtime::serde_json::Value,
                        ctx: &mut arete::runtime::arete_interpreter::resolvers::ResolveContext,
                    ) -> arete::runtime::arete_interpreter::resolvers::KeyResolution {
                        if let Some(key) = ctx.pda_reverse_lookup(account_address) {
                            return arete::runtime::arete_interpreter::resolvers::KeyResolution::Found(key);
                        }
                        arete::runtime::arete_interpreter::resolvers::KeyResolution::QueueUntil(&[#(#disc_bytes),*])
                    }
                });
            }
            _ => {
                // Future: other strategies like "direct_field"
                // For now, skip unknown strategies rather than panic
            }
        }
    }

    quote! { #(#functions)* }
}

/// Generate #[after_instruction] hooks for PDA registration from declarative #[register_pda] attributes.
pub fn generate_pda_registration_functions(
    pda_registrations: &[parse::RegisterPdaAttribute],
) -> proc_macro2::TokenStream {
    let mut functions = Vec::new();

    for (i, registration) in pda_registrations.iter().enumerate() {
        let _instruction_type = &registration.instruction_path;
        let fn_name = format_ident!("register_pda_{}", i);
        let pda_raw = registration.pda_field.ident.to_string();
        let pk_raw = registration.primary_key_field.ident.to_string();
        let pda_camel = crate::event_type_helpers::snake_to_lower_camel(&pda_raw);
        let pk_camel = crate::event_type_helpers::snake_to_lower_camel(&pk_raw);

        // IDL account names can be camelCase (e.g. Pumpfun: "bondingCurve") or
        // snake_case (e.g. Raydium: "pool_state"). The register_from attribute
        // uses Rust field names (always snake_case), so try both the camelCase
        // conversion and the raw snake_case name to match the IDL.
        functions.push(quote! {
                    pub fn #fn_name(ctx: &mut arete::runtime::arete_interpreter::resolvers::InstructionContext) {
                        let pk_val = ctx.account(#pk_camel).or_else(|| ctx.account(#pk_raw));
                        let pda_val = ctx.account(#pda_camel).or_else(|| ctx.account(#pda_raw));
                        if let (Some(primary_key), Some(pda)) = (pk_val, pda_val) {
                            ctx.register_pda_reverse_lookup(&pda, &primary_key);
                        }
                    }
                });
    }

    quote! { #(#functions)* }
}

pub fn generate_auto_resolver_functions(hooks: &[ResolverHook]) -> proc_macro2::TokenStream {
    let mut functions = Vec::new();

    for hook in hooks {
        let account_name = crate::event_type_helpers::strip_event_type_suffix(&hook.account_type);
        let fn_name = format_ident!("resolve_{}_key", to_snake_case(account_name));

        match &hook.strategy {
            ResolverStrategy::PdaReverseLookup {
                queue_discriminators,
                ..
            } => {
                let disc_bytes: Vec<u8> = queue_discriminators.iter().flatten().copied().collect();
                functions.push(quote! {
                    pub fn #fn_name(
                        account_address: &str,
                        _account_data: &arete::runtime::serde_json::Value,
                        ctx: &mut arete::runtime::arete_interpreter::resolvers::ResolveContext,
                    ) -> arete::runtime::arete_interpreter::resolvers::KeyResolution {
                        if let Some(key) = ctx.pda_reverse_lookup(account_address) {
                            return arete::runtime::arete_interpreter::resolvers::KeyResolution::Found(key);
                        }
                        arete::runtime::arete_interpreter::resolvers::KeyResolution::QueueUntil(&[#(#disc_bytes),*])
                    }
                });
            }
            ResolverStrategy::DirectField { .. } => {}
        }
    }

    quote! { #(#functions)* }
}

// ============================================================================
// Event Field Validation (unused but kept for reference)
// ============================================================================

/// Validate all fields exist in instruction and return their locations.
#[allow(dead_code)]
pub fn validate_event_fields(
    instruction_path: &Path,
    field_specs: &[parse::FieldSpec],
    idl: Option<&idl_parser::IdlSpec>,
) -> syn::Result<Vec<(String, parse::FieldLocation)>> {
    let mut result = Vec::new();

    for field_spec in field_specs {
        let field_name = field_spec.ident.to_string();

        // If explicit location is specified, use it
        let location = if let Some(explicit_loc) = &field_spec.explicit_location {
            explicit_loc.clone()
        } else {
            // Otherwise, find it in the IDL
            match find_field_in_instruction(instruction_path, &field_name, idl) {
                Ok(loc) => loc,
                Err(err_msg) => {
                    return Err(idl_error_to_syn(field_spec.ident.span(), err_msg));
                }
            }
        };

        result.push((field_name, location));
    }

    Ok(result)
}
