//! Entity struct processing for stream specs.
//!
//! This module contains the main entity processing logic, including:
//! - `process_entity_struct` - Process an entity with default options
//! - `process_entity_struct_with_idl` - Process an entity with IDL context
//! - Handler generation using shared `codegen::generate_handlers_from_specs`
//!
//! ## Architecture
//!
//! Both `#[stream_spec]` and `#[ast_spec]` now use the same code generation path:
//! 1. Parse macro attributes into data structures
//! 2. Build `SerializableStreamSpec` via `ast_writer::build_and_write_ast`
//! 3. Generate handler code via `codegen::generate_handlers_from_specs`
//! 4. Return AST for unified stack file writing

use std::collections::{BTreeMap, HashMap, HashSet};

use quote::{format_ident, quote};
use syn::spanned::Spanned;
use syn::{Fields, GenericArgument, ItemStruct, PathArguments, Type};

use super::resolve_snapshot_source;

use crate::ast::{
    EntitySection, FieldTypeInfo, HttpMethod, ResolverHook, ResolverType, UrlResolverConfig,
    UrlSource, UrlTemplatePart,
};
use crate::codegen;
use crate::diagnostic::{internal_codegen_error, unknown_value_message};
use crate::event_type_helpers::IdlLookup;
use crate::parse;
use crate::utils::{path_to_string, to_pascal_case, to_snake_case};
use crate::validation::{validate_semantics, ComputedFieldValidation, ValidationInput};

// ============================================================================
// Process Entity Result
// ============================================================================

/// Result of processing an entity struct, containing both the generated code
/// and any auto-generated resolver hooks that need to be threaded into the
/// resolver registry.
pub struct ProcessEntityResult {
    pub token_stream: proc_macro2::TokenStream,
    /// Auto-generated resolver hooks (e.g., for lookup_index account types).
    /// These come from the AST's `auto_generate_lookup_resolvers()` and need
    /// to be converted to `ResolverHookSpec` entries in the IDL path.
    pub auto_resolver_hooks: Vec<ResolverHook>,
    /// The built AST spec for this entity (used to build the unified stack file).
    pub ast_spec: Option<crate::ast::SerializableStreamSpec>,
}

use super::ast_writer::build_and_write_ast;
use super::computed::{
    extract_field_references_from_section, extract_section_references, parse_computed_expression,
    qualify_field_refs,
};
use super::handlers::{
    convert_event_to_map_attributes, determine_event_instruction, extract_account_type_from_field,
};
use super::sections::{
    self as sections, extract_section_from_struct_with_idl, is_primitive_or_wrapper,
    process_nested_struct,
};

pub fn parse_url_template(s: &str, span: proc_macro2::Span) -> syn::Result<Vec<UrlTemplatePart>> {
    let mut parts = Vec::new();
    let mut rest = s;

    while let Some(open) = rest.find('{') {
        if open > 0 {
            parts.push(UrlTemplatePart::Literal(rest[..open].to_string()));
        }
        let close = rest[open..]
            .find('}')
            .ok_or_else(|| syn::Error::new(span, format!("Unclosed '{{' in URL template: {s}")))?
            + open;
        let field_ref = rest[open + 1..close].trim().to_string();
        if field_ref.is_empty() {
            return Err(syn::Error::new(
                span,
                format!("Empty field reference '{{}}' in URL template: {s}"),
            ));
        }
        parts.push(UrlTemplatePart::FieldRef(field_ref));
        rest = &rest[close + 1..];
    }

    if !rest.is_empty() {
        parts.push(UrlTemplatePart::Literal(rest.to_string()));
    }

    Ok(parts)
}

// ============================================================================
// Entity Processing
// ============================================================================

pub fn process_entity_struct(
    input: ItemStruct,
    entity_name: String,
    section_structs: HashMap<String, ItemStruct>,
    skip_game_event: bool,
    stack_name: &str,
) -> syn::Result<ProcessEntityResult> {
    process_entity_struct_with_idl(
        input,
        entity_name,
        section_structs,
        skip_game_event,
        stack_name,
        &[],
        Vec::new(),
        Vec::new(),
    )
}

/// Process an entity struct with optional IDL context.
///
/// This is the main entry point for processing entity definitions. It:
/// 1. Extracts all field mappings from attributes
/// 2. Generates handlers for each source type
/// 3. Writes the AST file at compile time
/// 4. Generates the spec function and state struct
#[allow(clippy::too_many_arguments)]
pub fn process_entity_struct_with_idl(
    input: ItemStruct,
    entity_name: String,
    section_structs: HashMap<String, ItemStruct>,
    skip_game_event: bool,
    _stack_name: &str,
    idls: IdlLookup,
    resolver_hooks: Vec<parse::ResolveKeyAttribute>,
    pda_registrations: Vec<parse::RegisterPdaAttribute>,
) -> syn::Result<ProcessEntityResult> {
    let _name = syn::Ident::new(&entity_name, input.ident.span());
    let state_name = syn::Ident::new(&format!("{}State", entity_name), input.ident.span());
    let spec_fn_name = format_ident!("create_{}_spec", to_snake_case(&entity_name));

    let mut field_mappings = Vec::new();
    let mut primary_keys = Vec::new();
    let mut lookup_indexes: Vec<(String, Option<String>)> = Vec::new();
    let mut accessor_defs = Vec::new();
    let mut accessor_names = HashSet::new();
    let mut state_fields = Vec::new();
    let mut sources_by_type: BTreeMap<String, Vec<parse::MapAttribute>> = BTreeMap::new();
    let mut events_by_instruction: BTreeMap<
        String,
        Vec<(String, parse::EventAttribute, syn::Type)>,
    > = BTreeMap::new();
    // Derive primary IDL and program_name from the first entry (backward compat)
    let idl = idls.first().map(|(_, idl)| *idl);
    let program_name = idl.map(|idl| idl.get_name());
    let mut has_events = false;
    let mut computed_fields: Vec<(String, proc_macro2::TokenStream, syn::Type)> = Vec::new();
    let mut computed_field_validations: Vec<ComputedFieldValidation> = Vec::new();
    let mut resolve_specs: Vec<parse::ResolveSpec> = Vec::new();

    // Level 1: Declarative hook macros passed from caller
    // resolver_hooks and pda_registrations are now passed as parameters
    let mut derive_from_mappings: BTreeMap<String, Vec<parse::DeriveFromAttribute>> =
        BTreeMap::new();
    let mut aggregate_conditions: BTreeMap<String, crate::ast::ConditionExpr> = BTreeMap::new();

    // Collect ALL section names from the entity struct FIRST
    // This is needed to properly detect cross-section references in #[computed] expressions
    let mut all_section_names: HashSet<String> = HashSet::new();
    if let Fields::Named(entity_fields) = &input.fields {
        for field in &entity_fields.named {
            let field_name = field.ident.as_ref().unwrap().to_string();
            let field_type = &field.ty;

            // Check if this field is a section type (non-primitive, non-wrapper)
            if !is_primitive_or_wrapper(field_type) {
                // This field represents a section - add its name
                all_section_names.insert(field_name);
            }
        }
    }

    // Task 3: Collect section definitions for AST
    let mut section_specs: Vec<EntitySection> = Vec::new();
    let mut root_fields: Vec<FieldTypeInfo> = Vec::new();

    if let Fields::Named(entity_fields) = &input.fields {
        for field in &entity_fields.named {
            let field_name = field.ident.as_ref().unwrap().to_string();
            let field_type = &field.ty;
            let rust_type_name = quote::quote!(#field_type).to_string();

            // Check if this field references a section struct
            if !is_primitive_or_wrapper(field_type) {
                if let Type::Path(type_path) = field_type {
                    if let Some(type_ident) = type_path.path.segments.last() {
                        let type_name = type_ident.ident.to_string();
                        // Look up the section struct definition
                        if let Some(section_struct) = section_structs.get(&type_name) {
                            let section = extract_section_from_struct_with_idl(
                                &field_name,
                                section_struct,
                                None,
                                idls,
                            )?;
                            section_specs.push(section);
                        } else {
                            let field_type_info = sections::analyze_field_type_with_idl(
                                &field_name,
                                &rust_type_name,
                                idls,
                            );
                            root_fields.push(field_emit_override(
                                field,
                                field_name,
                                field_type_info,
                            )?);
                        }
                    }
                }
            } else {
                // Even if it's a "wrapper", we might want its type info if it's not truly primitive
                // For example, Option<ComplexType> should be included
                let field_type_info =
                    sections::analyze_field_type_with_idl(&field_name, &rust_type_name, idls);
                // Only add if it has a resolved_type (meaning it's a complex type from IDL)
                if field_type_info.resolved_type.is_some()
                    || field_type_info.base_type == crate::ast::BaseType::Object
                {
                    root_fields.push(field_emit_override(field, field_name, field_type_info)?);
                }
            }
        }
    }

    // Add root fields as a pseudo-section if there are any
    if !root_fields.is_empty() {
        section_specs.push(EntitySection {
            name: "root".to_string(),
            fields: root_fields,
            is_nested_struct: false,
            parent_field: None,
        });
    }

    if let Fields::Named(fields) = &input.fields {
        for field in &fields.named {
            let field_name = field.ident.as_ref().unwrap();
            let field_type = &field.ty;
            let field_name_str = field_name.to_string();

            let mut has_attrs = false;
            for attr in &field.attrs {
                match parse::parse_recognized_field_attribute(attr, &field_name_str)? {
                    Some(parse::RecognizedFieldAttribute::Map(map_attrs))
                    | Some(parse::RecognizedFieldAttribute::FromInstruction(map_attrs)) => {
                        has_attrs = true;
                        for map_attr in map_attrs {
                            process_map_attribute(
                                &map_attr,
                                field_name,
                                field_type,
                                &mut state_fields,
                                &mut accessor_defs,
                                &mut accessor_names,
                                &mut primary_keys,
                                &mut lookup_indexes,
                                &mut sources_by_type,
                                &mut field_mappings,
                            );
                        }
                    }
                    Some(parse::RecognizedFieldAttribute::Event(mut event_attr)) => {
                        has_attrs = true;
                        has_events = true;

                        state_fields.push(quote! {
                            pub #field_name: #field_type
                        });

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
                            events_by_instruction
                                .entry(event_attr.instruction.clone())
                                .or_default()
                                .push((
                                    event_attr.target_field_name.clone(),
                                    event_attr,
                                    field_type.clone(),
                                ));
                        }
                    }
                    Some(parse::RecognizedFieldAttribute::Snapshot(mut snapshot_attr)) => {
                        has_attrs = true;

                        state_fields.push(quote! {
                            pub #field_name: #field_type
                        });

                        let account_path = if let Some(ref path) = snapshot_attr.from_account {
                            Some(path.clone())
                        } else if let Some(inferred_path) =
                            extract_account_type_from_field(field_type)
                        {
                            snapshot_attr.inferred_account = Some(inferred_path.clone());
                            Some(inferred_path)
                        } else {
                            None
                        };

                        if let Some(acct_path) = account_path {
                            let source_type_str = path_to_string(&acct_path);
                            let (source_field_name, is_whole_source) =
                                resolve_snapshot_source(&snapshot_attr);
                            let source_field_span = snapshot_attr
                                .field
                                .as_ref()
                                .map(|field| field.span())
                                .unwrap_or(snapshot_attr.attr_span);

                            let map_attr = parse::MapAttribute {
                                attr_span: snapshot_attr.attr_span,
                                source_type_span: acct_path.span(),
                                source_field_span,
                                // NOTE: is_event_source=false is correct here.
                                // Event-derived attributes are created in handlers.rs with
                                // is_event_source=true and validated separately via
                                // validate_event_handler_keys before being merged.
                                is_event_source: false,
                                is_account_source: true,
                                source_type_path: acct_path,
                                source_field_name,
                                target_field_name: snapshot_attr.target_field_name.clone(),
                                is_primary_key: false,
                                is_lookup_index: false,
                                register_from: Vec::new(),
                                temporal_field: None,
                                strategy: snapshot_attr.strategy.clone(),
                                join_on: snapshot_attr.join_on.clone(),
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
                    }
                    Some(parse::RecognizedFieldAttribute::Aggregate(aggr_attr)) => {
                        has_attrs = true;

                        state_fields.push(quote! {
                            pub #field_name: #field_type
                        });

                        if let Some(condition) = &aggr_attr.condition {
                            let field_path = format!("{}.{}", entity_name, field_name);
                            aggregate_conditions.insert(field_path, condition.clone());
                        }

                        for instr_path in &aggr_attr.from_instructions {
                            let source_field_name = aggr_attr
                                .field
                                .as_ref()
                                .map(|fs| fs.ident.to_string())
                                .unwrap_or_default();
                            let source_field_span = aggr_attr
                                .field
                                .as_ref()
                                .map(|field| field.ident.span())
                                .unwrap_or(aggr_attr.attr_span);

                            let map_attr = parse::MapAttribute {
                                attr_span: aggr_attr.attr_span,
                                source_type_span: instr_path.span(),
                                source_field_span,
                                // NOTE: is_event_source=false is correct here.
                                // Event-derived attributes are created in handlers.rs with
                                // is_event_source=true and validated separately via
                                // validate_event_handler_keys before being merged.
                                is_event_source: false,
                                is_account_source: false,
                                source_type_path: instr_path.clone(),
                                source_field_name,
                                target_field_name: aggr_attr.target_field_name.clone(),
                                is_primary_key: false,
                                is_lookup_index: false,
                                register_from: Vec::new(),
                                temporal_field: None,
                                strategy: aggr_attr.strategy.clone(),
                                join_on: aggr_attr.join_on.clone(),
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

                            let source_type_str = path_to_string(instr_path);
                            sources_by_type
                                .entry(source_type_str)
                                .or_default()
                                .push(map_attr);
                        }
                    }
                    Some(parse::RecognizedFieldAttribute::DeriveFrom(derive_attr)) => {
                        has_attrs = true;
                        state_fields.push(quote! { pub #field_name: #field_type });

                        for instr_path in &derive_attr.from_instructions {
                            let source_type_str = path_to_string(instr_path);
                            derive_from_mappings
                                .entry(source_type_str)
                                .or_default()
                                .push(derive_attr.clone());
                        }
                    }
                    Some(parse::RecognizedFieldAttribute::Resolve(resolve_attr)) => {
                        has_attrs = true;

                        state_fields.push(quote! {
                            pub #field_name: #field_type
                        });

                        let resolver = if let Some(url_path) = resolve_attr.url.clone() {
                            let method = resolve_attr
                                .method
                                .as_deref()
                                .map(|m| match m.to_lowercase().as_str() {
                                    "post" => HttpMethod::Post,
                                    _ => HttpMethod::Get,
                                })
                                .unwrap_or(HttpMethod::Get);

                            let url_source = if resolve_attr.url_is_template {
                                UrlSource::Template(parse_url_template(&url_path, attr.span())?)
                            } else {
                                UrlSource::FieldPath(url_path)
                            };

                            ResolverType::Url(UrlResolverConfig {
                                url_source,
                                method,
                                extract_path: resolve_attr.extract.clone(),
                            })
                        } else if let Some(name) = resolve_attr.resolver.as_deref() {
                            parse_resolver_type_name(name, field_type)?
                        } else {
                            infer_resolver_type(field_type)?
                        };

                        let from = if resolve_attr.url_is_template {
                            None
                        } else {
                            resolve_attr.url.clone().or(resolve_attr.from)
                        };

                        resolve_specs.push(parse::ResolveSpec {
                            attr_span: resolve_attr.attr_span,
                            from_span: resolve_attr.from_span,
                            resolver,
                            from,
                            address: resolve_attr.address,
                            extract: resolve_attr.extract,
                            target_field_name: resolve_attr.target_field_name,
                            strategy: resolve_attr.strategy,
                            condition: resolve_attr.condition,
                            schedule_at: resolve_attr.schedule_at,
                        });
                    }
                    Some(parse::RecognizedFieldAttribute::Computed(computed_attr)) => {
                        has_attrs = true;

                        state_fields.push(quote! {
                            pub #field_name: #field_type
                        });

                        computed_fields.push((
                            computed_attr.target_field_name.clone(),
                            computed_attr.expression.clone(),
                            field_type.clone(),
                        ));
                        computed_field_validations.push(ComputedFieldValidation {
                            target_path: computed_attr.target_field_name.clone(),
                            expression: computed_attr.expression.clone(),
                            span: computed_attr.attr_span,
                        });
                    }
                    None => {}
                }
            }

            if !has_attrs && !is_primitive_or_wrapper(field_type) {
                if let Type::Path(type_path) = field_type {
                    if let Some(type_ident) = type_path.path.segments.last() {
                        let type_name = type_ident.ident.to_string();
                        if let Some(nested_struct) = section_structs.get(&type_name) {
                            process_nested_struct(
                                nested_struct,
                                field_name,
                                field_type,
                                &mut state_fields,
                                &mut accessor_defs,
                                &mut accessor_names,
                                &mut primary_keys,
                                &mut lookup_indexes,
                                &mut sources_by_type,
                                &mut field_mappings,
                                &mut events_by_instruction,
                                &mut has_events,
                                &mut computed_fields,
                                &mut computed_field_validations,
                                &mut resolve_specs,
                                &mut derive_from_mappings,
                                &mut aggregate_conditions,
                                program_name,
                            )?;
                        }
                    }
                }
            }
        }
    }

    // === EVENT MERGING: Merge event mappings into sources_by_type ===
    // Convert events to map attributes and merge with sources_by_type
    // Events with lookup_by are kept separate for special handling in AST building

    for event_mappings in events_by_instruction.values() {
        for (target_field, event_attr, _field_type) in event_mappings {
            // Skip events with lookup_by - they need separate handler generation
            // (They stay in events_by_instruction for AST building)
            if event_attr.lookup_by.is_some() {
                continue;
            }

            // Get instruction path from event attribute
            let instruction_path = event_attr
                .from_instruction
                .as_ref()
                .or(event_attr.inferred_instruction.as_ref());

            if let Some(instr_path) = instruction_path {
                // Convert instruction path to string for sources_by_type key
                let source_type_str = path_to_string(instr_path);

                // Convert event to map attributes
                let map_attrs =
                    convert_event_to_map_attributes(target_field, event_attr, instr_path, idl);

                // Merge into sources_by_type
                sources_by_type
                    .entry(source_type_str)
                    .or_default()
                    .extend(map_attrs);
            }
        }
    }

    let mut view_specs = parse::parse_view_attribute_specs(&input.attrs)?;
    for view_spec in &mut view_specs {
        let view = &mut view_spec.view;
        if let crate::ast::ViewSource::Entity { name } = &mut view.source {
            *name = entity_name.clone();
        }
        if !view.id.contains('/') {
            view.id = format!("{}/{}", entity_name, view.id);
        }
    }
    validate_semantics(ValidationInput {
        entity_name: &entity_name,
        primary_keys: &primary_keys,
        lookup_indexes: &lookup_indexes,
        sources_by_type: &sources_by_type,
        events_by_instruction: &events_by_instruction,
        derive_from_mappings: &derive_from_mappings,
        aggregate_conditions: &aggregate_conditions,
        resolver_hooks: &resolver_hooks,
        computed_fields: &computed_field_validations,
        resolve_specs: &resolve_specs,
        section_specs: &section_specs,
        view_specs: &view_specs,
        idls,
    })?;

    let views = view_specs.into_iter().map(|spec| spec.view).collect();

    let ast = build_and_write_ast(
        &entity_name,
        &primary_keys,
        &lookup_indexes,
        &sources_by_type,
        &events_by_instruction,
        &resolver_hooks,
        &pda_registrations,
        &derive_from_mappings,
        &aggregate_conditions,
        &computed_fields,
        &resolve_specs,
        &section_specs,
        idls,
        views,
    )?;

    let spec_json = serde_json::to_string(&ast).map_err(|error| {
        internal_codegen_error(
            input.ident.span(),
            format!("failed to serialize embedded stream spec: {error}"),
        )
    })?;

    // Round-trip check: ensure the JSON we embed can be deserialized at runtime
    let _: crate::ast::SerializableStreamSpec =
        serde_json::from_str(&spec_json).map_err(|error| {
            internal_codegen_error(
                input.ident.span(),
                format!("embedded stream spec round-trip check failed: {error}"),
            )
        })?;

    let explicit_account_types: HashSet<String> = resolver_hooks
        .iter()
        .map(|h| {
            let segments: Vec<String> = h
                .account_path
                .segments
                .iter()
                .map(|seg| seg.ident.to_string())
                .collect();
            let account_name = segments.last().cloned().unwrap_or_default();
            let resolved_program = segments
                .first()
                .filter(|s| s.ends_with("_sdk"))
                .map(|s| crate::event_type_helpers::program_name_from_sdk_prefix(s).to_string());
            let prog = resolved_program.as_deref().or(program_name);
            if let Some(p) = prog {
                format!("{}::{}State", p, account_name)
            } else {
                format!("{}State", account_name)
            }
        })
        .collect();
    let auto_resolver_hooks: Vec<ResolverHook> = ast
        .resolver_hooks
        .iter()
        .filter(|h| !explicit_account_types.contains(&h.account_type))
        .cloned()
        .collect();

    // Generate handler functions using the shared codegen
    let (handler_fns, _handler_calls) =
        codegen::generate_handlers_from_specs(&ast.handlers, &entity_name, &state_name);

    let game_event_struct = if has_events && !skip_game_event {
        quote! {
            #[derive(Debug, Clone, arete::runtime::serde::Serialize, arete::runtime::serde::Deserialize)]
            pub struct GameEvent {
                pub timestamp: i64,
                #[serde(flatten)]
                pub data: arete::runtime::serde_json::Value,
            }
        }
    } else {
        quote! {}
    };

    let _lookup_index_creations: Vec<_> = lookup_indexes
        .iter()
        .map(|(field_name, temporal_field)| {
            if let Some(tf) = temporal_field {
                quote! {
                    arete::runtime::arete_interpreter::ast::LookupIndexSpec {
                        field_name: #field_name.to_string(),
                        temporal_field: Some(#tf.to_string()),
                    }
                }
            } else {
                quote! {
                    arete::runtime::arete_interpreter::ast::LookupIndexSpec {
                        field_name: #field_name.to_string(),
                        temporal_field: None,
                    }
                }
            }
        })
        .collect();

    // Generate computed fields evaluation function if there are any computed fields
    // This function will be called after aggregations complete to evaluate derived fields
    let computed_fields_hook = if !computed_fields.is_empty() {
        generate_computed_fields_hook(&computed_fields, &all_section_names)
    } else {
        // Generate a no-op function even when there are no computed fields
        // so the evaluator callback can still reference it
        quote! {
            /// No-op evaluate_computed_fields (no computed fields defined)
            pub fn evaluate_computed_fields(
                _state: &mut arete::runtime::serde_json::Value,
                _context_slot: Option<u64>,
                _context_timestamp: i64,
            ) -> Result<(), Box<dyn std::error::Error>> {
                Ok(())
            }

            /// Returns the list of computed field paths (section.field format)
            pub fn computed_field_paths() -> &'static [&'static str] {
                &[]
            }
        }
    };

    // Generate field accessors for type-safe view definitions
    let field_accessors = codegen::generate_field_accessors(&section_specs);

    let module_name = format_ident!("{}", to_snake_case(&entity_name));

    let output = quote! {
        #[derive(Debug, Clone, arete::runtime::serde::Serialize, arete::runtime::serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        pub struct #state_name {
            #(#state_fields),*
        }

        #game_event_struct

        pub mod #module_name {
            use super::*;

            #(#accessor_defs)*

            #field_accessors

            #computed_fields_hook
        }

        pub fn #spec_fn_name() -> arete::runtime::arete_interpreter::ast::TypedStreamSpec<#state_name> {
            let spec_json = #spec_json;
            let spec: arete::runtime::arete_interpreter::ast::SerializableStreamSpec =
                arete::runtime::serde_json::from_str(spec_json)
                    .unwrap_or_else(|error| panic!("embedded stream spec is invalid: {}", error));

            arete::runtime::arete_interpreter::ast::TypedStreamSpec::from_serializable(spec)
        }

        #(#handler_fns)*
    };

    Ok(ProcessEntityResult {
        token_stream: output,
        auto_resolver_hooks,
        ast_spec: Some(ast),
    })
}

fn field_emit_override(
    field: &syn::Field,
    field_name: String,
    mut field_type_info: FieldTypeInfo,
) -> syn::Result<FieldTypeInfo> {
    let mut found_mapping = false;
    let mut any_emit = false;

    for attr in &field.attrs {
        match parse::parse_recognized_field_attribute(attr, &field_name)? {
            Some(parse::RecognizedFieldAttribute::Map(map_attrs))
            | Some(parse::RecognizedFieldAttribute::FromInstruction(map_attrs)) => {
                found_mapping = true;
                if map_attrs.iter().any(|m| m.emit) {
                    any_emit = true;
                }
            }
            _ => {}
        }
    }

    if found_mapping {
        field_type_info.emit = any_emit;
    }

    Ok(field_type_info)
}

pub(super) fn parse_resolver_type_name(name: &str, field_type: &Type) -> syn::Result<ResolverType> {
    match name.to_lowercase().as_str() {
        "token" => Ok(ResolverType::Token),
        _ => Err(syn::Error::new_spanned(
            field_type,
            unknown_value_message(
                "resolver",
                name,
                "Available resolvers",
                &["Token".to_string()],
            ),
        )),
    }
}

pub(super) fn infer_resolver_type(field_type: &Type) -> syn::Result<ResolverType> {
    let type_ident = extract_resolver_type_ident(field_type).ok_or_else(|| {
        syn::Error::new_spanned(field_type, "Unable to infer resolver type from field")
    })?;

    match type_ident.as_str() {
        "TokenMetadata" => Ok(ResolverType::Token),
        _ => Err(syn::Error::new_spanned(
            field_type,
            unknown_value_message(
                "resolver-backed type",
                &type_ident,
                "Available types",
                &["TokenMetadata".to_string()],
            ),
        )),
    }
}

fn extract_resolver_type_ident(field_type: &Type) -> Option<String> {
    match field_type {
        Type::Path(type_path) => {
            let segment = type_path.path.segments.last()?;
            if segment.ident == "Option" || segment.ident == "Vec" {
                if let PathArguments::AngleBracketed(args) = &segment.arguments {
                    for arg in &args.args {
                        if let GenericArgument::Type(inner_ty) = arg {
                            return extract_resolver_type_ident(inner_ty);
                        }
                    }
                }
                return None;
            }
            Some(segment.ident.to_string())
        }
        _ => None,
    }
}

// ============================================================================
// Map Attribute Processing
// ============================================================================

/// Process a map attribute and update all the relevant collections.
#[allow(clippy::too_many_arguments)]
pub fn process_map_attribute(
    map_attr: &parse::MapAttribute,
    field_name: &syn::Ident,
    field_type: &Type,
    state_fields: &mut Vec<proc_macro2::TokenStream>,
    accessor_defs: &mut Vec<proc_macro2::TokenStream>,
    accessor_names: &mut HashSet<String>,
    primary_keys: &mut Vec<String>,
    lookup_indexes: &mut Vec<(String, Option<String>)>,
    sources_by_type: &mut BTreeMap<String, Vec<parse::MapAttribute>>,
    field_mappings: &mut Vec<parse::MapAttribute>,
) {
    let target_field = &map_attr.target_field_name;

    state_fields.push(quote! {
        pub #field_name: #field_type
    });

    let accessor_name = to_pascal_case(&field_name.to_string());
    let accessor_ident = format_ident!("{}", accessor_name);

    if accessor_names.insert(accessor_name.clone()) {
        accessor_defs.push(quote! {
            pub struct #accessor_ident;

            impl #accessor_ident {
                pub fn path(&self) -> String {
                    #target_field.to_string()
                }
            }
        });
    }

    if map_attr.is_primary_key {
        primary_keys.push(target_field.clone());
    }

    if map_attr.is_lookup_index {
        lookup_indexes.push((target_field.clone(), map_attr.temporal_field.clone()));
    }

    let source_type = path_to_string(&map_attr.source_type_path);
    sources_by_type
        .entry(source_type.clone())
        .or_default()
        .push(map_attr.clone());

    field_mappings.push(map_attr.clone());
}

// ============================================================================
// Computed Fields Hook Generation
// ============================================================================

/// Generate the computed fields evaluation hook.
fn generate_computed_fields_hook(
    computed_fields: &[(String, proc_macro2::TokenStream, Type)],
    all_section_names: &HashSet<String>,
) -> proc_macro2::TokenStream {
    // Group computed fields by section
    let mut fields_by_section: HashMap<String, Vec<(String, proc_macro2::TokenStream, Type)>> =
        HashMap::new();

    for (target_field, expression, field_type) in computed_fields {
        // Extract section name from target_field (format: "section.field")
        let parts: Vec<&str> = target_field.split('.').collect();
        if parts.len() == 2 {
            let section = parts[0].to_string();
            let field = parts[1].to_string();
            fields_by_section.entry(section.clone()).or_default().push((
                field.clone(),
                expression.clone(),
                field_type.clone(),
            ));
        }
    }

    // Analyze expressions to determine which sections they reference
    // Use the all_section_names collected earlier to properly identify cross-section references
    let mut section_dependencies: HashMap<String, HashSet<String>> = HashMap::new();
    for (section, fields) in &fields_by_section {
        let mut deps = HashSet::new();
        for (_field_name, expression, _field_type) in fields {
            let referenced_sections = extract_section_references(expression);

            // Filter to only include references that are:
            // 1. NOT the current section (same-section references don't need special handling)
            // 2. ACTUALLY known section names (not method names like unwrap_or, max, etc.)
            let valid_refs: HashSet<String> = referenced_sections
                .into_iter()
                .filter(|ref_name| {
                    let is_known = all_section_names.contains(ref_name);
                    let is_not_current = ref_name != section;
                    is_known && is_not_current
                })
                .collect();

            deps.extend(valid_refs);
        }
        section_dependencies.insert(section.clone(), deps);
    }

    // First, collect ALL struct definitions that we need (from all sections' dependencies)
    let mut all_struct_defs: Vec<proc_macro2::TokenStream> = Vec::new();
    let mut generated_structs: HashSet<String> = HashSet::new();

    for (section, fields) in &fields_by_section {
        let deps = section_dependencies
            .get(section)
            .cloned()
            .unwrap_or_default();

        for dep_section in &deps {
            // Only generate struct once per dependency section
            if generated_structs.contains(dep_section) {
                continue;
            }
            generated_structs.insert(dep_section.clone());

            let section_struct_ident = format_ident!("{}Section", dep_section);

            // Collect all fields accessed from this section across ALL expressions
            let mut section_fields = HashSet::new();
            for (_field_name, expression, _field_type) in fields {
                let field_refs = extract_field_references_from_section(expression, dep_section);
                section_fields.extend(field_refs);
            }

            // Generate struct definition for this section
            let field_defs: Vec<_> = section_fields
                .iter()
                .map(|field| {
                    let field_ident = format_ident!("{}", field);
                    quote! {
                        pub #field_ident: Option<u64>
                    }
                })
                .collect();

            // Generate field extraction in constructor
            let field_extractors: Vec<_> = section_fields
                .iter()
                .map(|field| {
                    let field_ident = format_ident!("{}", field);
                    let field_str = field.as_str();
                    quote! {
                        #field_ident: obj.get(#field_str)
                            .and_then(|v| arete::runtime::serde_json::from_value(v.clone()).ok())
                    }
                })
                .collect();

            all_struct_defs.push(quote! {
                // Helper struct for cross-section field access
                #[allow(dead_code)]
                struct #section_struct_ident {
                    #(#field_defs),*
                }

                impl #section_struct_ident {
                    fn from_object(obj: &arete::runtime::serde_json::Map<String, arete::runtime::serde_json::Value>) -> Self {
                        Self {
                            #(#field_extractors),*
                        }
                    }
                }
            });
        }
    }

    // Now generate the evaluation functions
    let eval_functions: Vec<_> = fields_by_section.iter().map(|(section, fields)| {
        let _section_ident = format_ident!("{}", section);
        let eval_fn_name = format_ident!("evaluate_computed_fields_{}", section);

        // Get dependencies for this section
        let deps = section_dependencies.get(section).cloned().unwrap_or_default();

        let section_str = section.as_str();

        // Collect all computed field names in this section for cache tracking
        let computed_field_names: Vec<String> = fields.iter().map(|(field_name, _expression, _field_type)| {
            field_name.clone()
        }).collect();

        let field_evaluations: Vec<_> = fields.iter().map(|(field_name, expression, field_type)| {
            let field_str = field_name.as_str();
            let field_ident = format_ident!("{}", field_name);

            // Parse the expression into an AST and generate evaluation code
            // This transforms expressions like `round_snapshot.slot_hash.to_bytes()` into
            // proper JSON state access code
            let parsed_expr = parse_computed_expression(expression);
            // Qualify the expression with the section prefix for unqualified field refs
            let qualified_expr = qualify_field_refs(parsed_expr, section);
            // Use cache-aware code generation for intra-section dependencies
            let expr_code = crate::codegen::generate_computed_expr_code_with_cache(&qualified_expr, section_str, &computed_field_names);

            quote! {
                // Evaluate: #field_name
                let computed_value = {
                    // state is the full entity JSON state (for cross-section references)
                    let state = &section_parent_state;
                    #expr_code
                };
                let serialized_value = arete::runtime::serde_json::to_value(&computed_value)?;
                // Update cache so dependent fields can read this value
                computed_cache.insert(#field_str.to_string(), serialized_value.clone());
                section_obj.insert(#field_str.to_string(), serialized_value);

                let #field_ident: #field_type = section_obj
                    .get(#field_str)
                    .and_then(|v| arete::runtime::serde_json::from_value(v.clone()).ok());
            }
        }).collect();

        // Generate function parameters for cross-section dependencies
        let cross_section_params: Vec<_> = deps.iter().map(|dep_section| {
            let dep_section_ident = format_ident!("{}", dep_section);
            let section_struct_ident = format_ident!("{}Section", dep_section);
            quote! {
                #dep_section_ident: &#section_struct_ident
            }
        }).collect();

        quote! {
            #[allow(clippy::too_many_arguments)]
            fn #eval_fn_name(
                section_obj: &mut arete::runtime::serde_json::Map<String, arete::runtime::serde_json::Value>,
                section_parent_state: &arete::runtime::serde_json::Value,
                __context_slot: Option<u64>,
                __context_timestamp: i64,
                #(#cross_section_params),*
            ) -> Result<(), Box<dyn std::error::Error>> {
                // Initialize cache with current section values for intra-section computed field dependencies
                let mut computed_cache: std::collections::HashMap<String, arete::runtime::serde_json::Value> = std::collections::HashMap::new();
                for (key, value) in section_obj.iter() {
                    computed_cache.insert(key.clone(), value.clone());
                }

                // Evaluate computed fields (they read from cache for intra-section dependencies)
                #(#field_evaluations)*

                Ok(())
            }
        }
    }).collect();

    // Generate the main hook function that applies to state after handlers execute
    // We need to extract cross-section data BEFORE getting mutable borrow of target section
    // to avoid borrow checker issues
    let eval_calls: Vec<_> = fields_by_section.keys().map(|section| {
        let section_str = section.as_str();
        let eval_fn_name = format_ident!("evaluate_computed_fields_{}", section);
        let deps = section_dependencies.get(section).cloned().unwrap_or_default();

        // Generate pre-extraction of cross-section data
        // These return Option so we can gracefully skip evaluation if a dependency doesn't exist yet
        let dep_extractions: Vec<_> = deps.iter().map(|dep_section| {
            // Use a unique variable name to avoid shadowing issues
            let dep_section_ident = format_ident!("{}_section", dep_section);
            let dep_section_str = dep_section.as_str();
            let section_struct_ident = format_ident!("{}Section", dep_section);
            quote! {
                let #dep_section_ident = state
                    .get(#dep_section_str)
                    .and_then(|v| v.as_object())
                    .map(|obj| #section_struct_ident::from_object(obj));
            }
        }).collect();

        if dep_extractions.is_empty() {
            quote! {
                let state_snapshot = state.clone();

                if let Some(section_value) = state.get_mut(#section_str) {
                    if let Some(section_obj) = section_value.as_object_mut() {
                        #eval_fn_name(section_obj, &state_snapshot, __context_slot, __context_timestamp)?;
                    }
                }
            }
        } else {
            // Has cross-section dependencies - extract first, then compute
            // If ANY dependency is missing, skip evaluation (the computed fields will remain None)
            let dep_param_names: Vec<_> = deps.iter().map(|dep| format_ident!("{}_section", dep)).collect();
            let dep_checks: Vec<_> = deps.iter().map(|dep| {
                let dep_ident = format_ident!("{}_section", dep);
                quote! { #dep_ident.is_some() }
            }).collect();
            let dep_unwraps: Vec<_> = deps.iter().map(|dep| {
                // Use consistent variable naming
                let dep_ident = format_ident!("{}_section", dep);
                quote! { let #dep_ident = #dep_ident.unwrap(); }
            }).collect();
            quote! {
                // Clone the state for immutable reference during computation
                let state_snapshot = state.clone();

                // Extract cross-section data first (immutable borrow)
                #(#dep_extractions)*

                // Only evaluate if ALL cross-section dependencies exist
                // If any dependency is missing, skip evaluation (computed fields stay None)
                if #(#dep_checks)&&* {
                    #(#dep_unwraps)*

                    // Now get mutable borrow of target section and compute
                    if let Some(section_value) = state.get_mut(#section_str) {
                        if let Some(section_obj) = section_value.as_object_mut() {
                            #eval_fn_name(section_obj, &state_snapshot, __context_slot, __context_timestamp, #(&#dep_param_names),*)?;
                        }
                    }
                } else {
                    arete::runtime::tracing::trace!("Skipping computed fields for section '{}' (dependencies not available)", #section_str);
                }
            }
        }
    }).collect();

    // Generate the list of computed field paths as string literals
    let computed_field_path_strs: Vec<&str> = computed_fields
        .iter()
        .map(|(path, _, _)| path.as_str())
        .collect();

    quote! {
        // Helper structs for cross-section field access (generated by macro)
        #(#all_struct_defs)*

        // Computed field evaluation functions (generated by macro)
        #(#eval_functions)*

        /// Evaluate all computed fields for the entity state
        /// This should be called after aggregations complete but before hooks run
        pub fn evaluate_computed_fields(
            state: &mut arete::runtime::serde_json::Value,
            __context_slot: Option<u64>,
            __context_timestamp: i64,
        ) -> Result<(), Box<dyn std::error::Error>> {
            #(#eval_calls)*
            Ok(())
        }

        /// Returns the list of computed field paths (section.field format)
        pub fn computed_field_paths() -> &'static [&'static str] {
            &[#(#computed_field_path_strs),*]
        }
    }
}
