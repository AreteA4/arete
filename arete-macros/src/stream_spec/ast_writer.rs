//! AST building and file writing for arete streams.
//!
//! This module handles:
//! 1. Building `SerializableStreamSpec` from parsed macro attributes
//! 2. Projecting that model into exact public artifacts during macro expansion
//!
//! The same AST is used for both inline code generation (via `codegen::generate_handlers_from_specs`)
//! and internal compiler consumers, ensuring identical output.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use crate::ast::writer::{
    context_field_name, convert_idl_to_snapshot, parse_population_strategy, parse_transformation,
};
use crate::ast::{
    ComputedFieldSpec, ConditionExpr, EntitySection, FieldPath, HookAction, IdentitySpec,
    IdlSerializationSnapshot, InstructionHook, KeyResolutionStrategy, LookupIndexSpec,
    MappingSource, ResolveStrategy, ResolverCondition, ResolverExtractSpec, ResolverHook,
    ResolverSpec, ResolverStrategy, ResolverType, SerializableFieldMapping,
    SerializableHandlerSpec, SerializableStreamSpec, SourceSpec,
};
use crate::diagnostic::{idl_error_to_syn, internal_codegen_error};
use crate::event_type_helpers::{
    find_idl_by_prefix, find_idl_for_type, program_name_for_type, scoped_instruction_event_type,
    scoped_instruction_or_cpi_event_type, IdlLookup,
};
use crate::parse;
use crate::parse::conditions as condition_parser;
use crate::parse::idl as idl_parser;
use crate::utils::path_to_string;
use arete_idl::error::IdlSearchError;
use arete_idl::search::{lookup_account, lookup_instruction_field, InstructionFieldKind};

use super::computed::{
    expr_contains_u64_from_bytes, extract_resolver_type_from_computed_expr,
    parse_computed_expression, qualify_field_refs,
};
use super::handlers::{find_field_in_instruction, get_join_on_field};

// ============================================================================
// AST Building
// ============================================================================

/// The program owning a declared source, accepting either spelling the macro
/// produces: an sdk-prefixed type (`entropy_sdk::accounts::Var`) or a
/// program-qualified event key (`entropy::Reveal`).
fn source_program_name<'a>(source: &str, idls: IdlLookup<'a>) -> Option<&'a str> {
    if let Some(idl) = find_idl_by_prefix(source, idls) {
        return Some(idl.get_name());
    }
    let program = source.split("::").next()?.trim();
    idls.iter()
        .map(|(_, idl)| *idl)
        .find(|idl| idl.get_name() == program)
        .map(|idl| idl.get_name())
}

/// One entry per distinct declared source, so a source cannot be counted
/// twice for being captured by several fields.
///
/// The tail keeps the kind segment, which separates an account from an
/// instruction of the same name.
fn source_identity<'a>(source: &str, idls: IdlLookup<'a>) -> Option<(&'a str, String)> {
    let program = source_program_name(source, idls)?;
    let tail = source.split("::").skip(1).collect::<Vec<_>>().join("::");
    Some((program, tail))
}

/// Resolve the IDL that owns this entity, from the sources it declares.
///
/// A single-program entity resolves to its own program whatever order the
/// stack declares its IDLs in, which picking `idls.first()` could not do: an
/// entity sourced entirely from the second IDL used to advertise the first
/// IDL's program id and be typed against the first IDL's schema.
fn resolve_entity_idl<'a>(
    sources_by_type: &BTreeMap<String, Vec<parse::MapAttribute>>,
    events_by_instruction: &BTreeMap<String, Vec<(String, parse::EventAttribute, syn::Type)>>,
    idls: IdlLookup<'a>,
) -> Option<&'a idl_parser::IdlSpec> {
    if idls.len() <= 1 {
        return idls.first().map(|(_, idl)| *idl);
    }

    // ponytail: an entity spanning programs (ore's `OreRound` reads Entropy
    // accounts) is attributed to whichever program owns most of the distinct
    // sources it declares, tie-broken by program name so the result never
    // depends on IDL declaration order. Per-section program attribution is the
    // upgrade path if one entity ever needs two programs to own different
    // sections.
    let mut declared: BTreeSet<(&str, String)> = sources_by_type
        .keys()
        .filter_map(|source_type| source_identity(source_type, idls))
        .collect();

    // Every event names a source, including the legacy
    // `event(instruction = "entropy::Reveal")` form, which keeps no path and so
    // is never merged into `sources_by_type`. An event that *was* merged
    // resolves to the identity already in the set, and several fields may
    // capture one instruction, so the set collapses both to one vote.
    for (instruction_key, event_mappings) in events_by_instruction {
        for (_, event_attr, _) in event_mappings {
            let instruction_path = event_attr
                .from_instruction
                .as_ref()
                .or(event_attr.inferred_instruction.as_ref())
                .map(path_to_string);
            let identity = instruction_path
                .as_deref()
                .and_then(|path| source_identity(path, idls))
                .or_else(|| source_identity(instruction_key, idls));
            if let Some(identity) = identity {
                declared.insert(identity);
            }
        }
    }

    let mut sources_per_program: BTreeMap<&str, usize> = BTreeMap::new();
    for (program, _) in &declared {
        *sources_per_program.entry(program).or_default() += 1;
    }

    let owner = sources_per_program
        .into_iter()
        .max_by(|left, right| left.1.cmp(&right.1).then_with(|| right.0.cmp(left.0)))
        .map(|(program_name, _)| program_name);

    owner
        .and_then(|program_name| {
            idls.iter()
                .find(|(_, idl)| idl.get_name() == program_name)
                .map(|(_, idl)| *idl)
        })
        .or_else(|| idls.first().map(|(_, idl)| *idl))
}

/// Build the complete AST from parsed macro attributes.
///
/// This is the single source of truth for building `SerializableStreamSpec`.
/// The returned AST is used for:
/// 1. Code generation via `codegen::generate_handlers_from_specs()`
/// 2. Projection into ProgramSpec, LiveSpec, and StackManifest artifacts
///
/// # Arguments
///
/// * `entity_name` - The name of the entity
/// * `primary_keys` - List of primary key field names
/// * `lookup_indexes` - List of lookup index definitions
/// * `sources_by_type` - Map of source type to field mappings
/// * `events_by_instruction` - Map of instruction to event mappings
/// * `resolver_hooks` - Resolver hook definitions
/// * `pda_registrations` - PDA registration definitions
/// * `derive_from_mappings` - Derive-from field mappings
/// * `aggregate_conditions` - Conditional aggregate definitions
/// * `computed_fields` - Computed field definitions
/// * `section_specs` - Entity section specifications
/// * `idl` - Optional IDL specification for field resolution
/// * `views` - View definitions for derived views
#[allow(clippy::too_many_arguments)]
pub fn build_ast(
    entity_name: &str,
    primary_keys: &[String],
    lookup_indexes: &[(String, Option<String>)],
    sources_by_type: &BTreeMap<String, Vec<parse::MapAttribute>>,
    events_by_instruction: &BTreeMap<String, Vec<(String, parse::EventAttribute, syn::Type)>>,
    resolver_hooks: &[parse::ResolveKeyAttribute],
    pda_registrations: &[parse::RegisterPdaAttribute],
    derive_from_mappings: &BTreeMap<String, Vec<parse::DeriveFromAttribute>>,
    aggregate_conditions: &BTreeMap<String, ConditionExpr>,
    computed_fields: &[(String, proc_macro2::TokenStream, syn::Type)],
    resolve_specs: &[parse::ResolveSpec],
    section_specs: &[EntitySection],
    idls: IdlLookup,
    views: Vec<crate::ast::ViewDef>,
) -> syn::Result<SerializableStreamSpec> {
    let idl = resolve_entity_idl(sources_by_type, events_by_instruction, idls);
    let handlers = build_handlers(
        sources_by_type,
        events_by_instruction,
        primary_keys,
        lookup_indexes,
        aggregate_conditions,
        idls,
    )?;

    let mut resolver_hooks_ast = build_resolver_hooks_ast(resolver_hooks, idls);
    resolver_hooks_ast.extend(auto_generate_lookup_resolvers(
        &handlers,
        &resolver_hooks_ast,
        sources_by_type,
        idls,
    ));
    resolver_hooks_ast.sort_by(|left, right| left.account_type.cmp(&right.account_type));
    let instruction_hooks_ast = build_instruction_hooks_ast(
        pda_registrations,
        derive_from_mappings,
        aggregate_conditions,
        sources_by_type,
        idls,
    );

    let computed_field_paths: Vec<String> = computed_fields
        .iter()
        .map(|(path, _, _)| path.clone())
        .collect();

    let program_id = idl.and_then(|i| {
        i.address.clone().or_else(|| {
            i.metadata
                .as_ref()
                .and_then(|m| m.address.as_ref().cloned())
        })
    });
    let idl_snapshot = idl.map(convert_idl_to_snapshot);

    // Parse computed field expressions into ComputedFieldSpec
    let computed_field_specs: Vec<ComputedFieldSpec> = computed_fields
        .iter()
        .map(|(target_path, expr_tokens, field_type)| {
            let result_type = quote::quote!(#field_type).to_string();
            let expression = parse_computed_expression(expr_tokens);

            // Extract section name from target_path and qualify field references
            let section = target_path.split('.').next().unwrap_or("");
            let qualified_expression = if !section.is_empty() {
                qualify_field_refs(expression, section)
            } else {
                expression
            };

            ComputedFieldSpec {
                target_path: target_path.clone(),
                expression: qualified_expression,
                result_type,
            }
        })
        .collect();

    let resolver_specs = build_resolver_specs(resolve_specs)?;

    // Build field_mappings from sections - this provides type information for ALL fields
    let mut field_mappings = BTreeMap::new();
    for section in section_specs {
        for field_info in &section.fields {
            // Handle root-level fields (no section prefix)
            let field_path = if section.name == "root" {
                field_info.field_name.clone()
            } else {
                format!("{}.{}", section.name, field_info.field_name)
            };
            field_mappings.insert(field_path, field_info.clone());
        }
    }

    // Add computed fields to field_mappings with resolver type information
    // This ensures computed fields that use resolvers get proper TypeScript schema generation
    for computed_spec in &computed_field_specs {
        // Determine the TypeScript type override for this computed field.
        // Priority 1: explicit resolver method (e.g. .keccak_rng(...)) → its declared output type.
        // Priority 2: expression assembles a u64 from raw bytes → "KeccakRngValue" (string),
        //             because values span [0, 2^64-1] and exceed Number.MAX_SAFE_INTEGER.
        let resolver_type: Option<&'static str> =
            extract_resolver_type_from_computed_expr(&computed_spec.expression).or_else(|| {
                let result_type = &computed_spec.result_type;
                let is_u64 = result_type == "u64"
                    || result_type == "Option < u64 >"
                    || result_type == "Option<u64>";
                if is_u64 && expr_contains_u64_from_bytes(&computed_spec.expression) {
                    Some("KeccakRngValue")
                } else {
                    None
                }
            });

        if let Some(resolver_type) = resolver_type {
            // Parse the result type to determine if it's optional and if it's an array
            let result_type = &computed_spec.result_type;
            let is_optional =
                result_type.starts_with("Option <") || result_type.starts_with("Option<");
            let is_array = result_type.contains("Vec <")
                || result_type.contains("Vec<")
                || result_type.contains("[");

            let mut field_info = crate::stream_spec::sections::analyze_field_type(
                &computed_spec.target_path,
                result_type,
            );
            field_info.base_type = if is_array {
                crate::ast::BaseType::Array
            } else {
                crate::ast::BaseType::Any
            };
            field_info.is_optional = is_optional;
            field_info.is_array = is_array;
            field_info.inner_type = Some(resolver_type.to_string());
            field_mappings.insert(computed_spec.target_path.clone(), field_info);
        }
    }

    let mut spec = SerializableStreamSpec {
        ast_version: crate::ast::CURRENT_AST_VERSION.to_string(),
        state_name: entity_name.to_string(),
        program_id,
        idl: idl_snapshot,
        identity: IdentitySpec {
            primary_keys: primary_keys.to_vec(),
            lookup_indexes: lookup_indexes
                .iter()
                .map(|(field_name, temporal_field)| LookupIndexSpec {
                    field_name: field_name.clone(),
                    temporal_field: temporal_field.clone(),
                })
                .collect(),
        },
        handlers,
        sections: section_specs.to_vec(),
        field_mappings,
        resolver_hooks: resolver_hooks_ast,
        instruction_hooks: instruction_hooks_ast,
        resolver_specs,
        computed_fields: computed_field_paths,
        computed_field_specs,
        content_hash: None,
        views,
    };
    // Compute and set the content hash
    spec.content_hash = Some(spec.try_compute_content_hash().map_err(|error| {
        internal_codegen_error(
            proc_macro2::Span::call_site(),
            format!("failed to serialize stream spec for hashing: {error}"),
        )
    })?);
    Ok(spec)
}

fn build_resolver_specs(resolve_specs: &[parse::ResolveSpec]) -> syn::Result<Vec<ResolverSpec>> {
    let mut grouped: BTreeMap<String, ResolverSpec> = BTreeMap::new();

    for spec in resolve_specs {
        let input_key = if let Some(from) = &spec.from {
            format!("path:{}", from)
        } else if let Some(address) = &spec.address {
            format!("value:{}", address)
        } else {
            "value:".to_string()
        };
        let condition_key = spec
            .condition
            .as_ref()
            .map(|condition| condition.expression.as_str())
            .unwrap_or("");
        let schedule_key = spec
            .schedule_at
            .as_ref()
            .map(|path| path.raw.as_str())
            .unwrap_or("");
        let key = format!(
            "{}::{}::{}::{}::{}",
            resolver_type_key(&spec.resolver),
            input_key,
            spec.strategy,
            condition_key,
            schedule_key,
        );

        let condition = spec
            .condition
            .as_ref()
            .map(|condition| condition.parsed.clone());

        let entry = grouped.entry(key).or_insert_with(|| ResolverSpec {
            resolver: spec.resolver.clone(),
            input_path: spec.from.clone(),
            input_value: spec
                .address
                .as_ref()
                .map(|value| serde_json::Value::String(value.clone())),
            strategy: parse_resolve_strategy(&spec.strategy),
            extracts: Vec::new(),
            condition,
            schedule_at: spec.schedule_at.as_ref().map(|path| path.raw.clone()),
        });

        let source_path = spec.extract.clone();

        let extract = ResolverExtractSpec {
            target_path: spec.target_field_name.clone(),
            source_path,
            transform: None,
        };

        if !entry.extracts.iter().any(|existing| {
            existing.target_path == extract.target_path
                && existing.source_path == extract.source_path
        }) {
            entry.extracts.push(extract);
        }
    }

    Ok(grouped.into_values().collect())
}

fn parse_resolve_strategy(strategy: &str) -> ResolveStrategy {
    match strategy {
        "LastWrite" => ResolveStrategy::LastWrite,
        _ => ResolveStrategy::SetOnce,
    }
}

#[allow(dead_code)]
pub fn parse_resolver_condition_from_str(s: &str) -> syn::Result<ResolverCondition> {
    condition_parser::parse_resolver_condition_expression(s)
        .map_err(|error| syn::Error::new(proc_macro2::Span::call_site(), error))
}

fn resolver_type_key(resolver: &ResolverType) -> String {
    match resolver {
        ResolverType::Token => "token".to_string(),
        ResolverType::Url(config) => match &config.url_source {
            crate::ast::UrlSource::FieldPath(path) => format!("url:{}", path),
            crate::ast::UrlSource::Template(parts) => {
                let key: String = parts
                    .iter()
                    .map(|p| match p {
                        crate::ast::UrlTemplatePart::Literal(s) => s.clone(),
                        crate::ast::UrlTemplatePart::FieldRef(f) => format!("{{{}}}", f),
                    })
                    .collect();
                format!("url:{}", key)
            }
        },
    }
}

// ============================================================================
// AST Building (no file writing — unified stack file is written at module level)
// ============================================================================

/// Build AST, returning the AST for code generation.
#[allow(clippy::too_many_arguments)]
pub fn build_and_write_ast(
    entity_name: &str,
    primary_keys: &[String],
    lookup_indexes: &[(String, Option<String>)],
    sources_by_type: &BTreeMap<String, Vec<parse::MapAttribute>>,
    events_by_instruction: &BTreeMap<String, Vec<(String, parse::EventAttribute, syn::Type)>>,
    resolver_hooks: &[parse::ResolveKeyAttribute],
    pda_registrations: &[parse::RegisterPdaAttribute],
    derive_from_mappings: &BTreeMap<String, Vec<parse::DeriveFromAttribute>>,
    aggregate_conditions: &BTreeMap<String, ConditionExpr>,
    computed_fields: &[(String, proc_macro2::TokenStream, syn::Type)],
    resolve_specs: &[parse::ResolveSpec],
    section_specs: &[EntitySection],
    idls: IdlLookup,
    views: Vec<crate::ast::ViewDef>,
) -> syn::Result<SerializableStreamSpec> {
    build_ast(
        entity_name,
        primary_keys,
        lookup_indexes,
        sources_by_type,
        events_by_instruction,
        resolver_hooks,
        pda_registrations,
        derive_from_mappings,
        aggregate_conditions,
        computed_fields,
        resolve_specs,
        section_specs,
        idls,
        views,
    )
}

// ============================================================================
// Handler Building
// ============================================================================

fn build_handlers(
    sources_by_type: &BTreeMap<String, Vec<parse::MapAttribute>>,
    events_by_instruction: &BTreeMap<String, Vec<(String, parse::EventAttribute, syn::Type)>>,
    primary_keys: &[String],
    lookup_indexes: &[(String, Option<String>)],
    aggregate_conditions: &BTreeMap<String, ConditionExpr>,
    idls: IdlLookup,
) -> syn::Result<Vec<SerializableHandlerSpec>> {
    let mut handlers = Vec::new();

    // Group sources by type and join key
    let mut sources_by_type_and_join: BTreeMap<(String, Option<String>), Vec<parse::MapAttribute>> =
        BTreeMap::new();
    for (source_type, mappings) in sources_by_type {
        for mapping in mappings {
            let key = (
                source_type.clone(),
                mapping
                    .join_on
                    .as_ref()
                    .map(|field_spec| field_spec.ident.to_string()),
            );
            sources_by_type_and_join
                .entry(key)
                .or_default()
                .push(mapping.clone());
        }
    }

    for ((source_type, join_key), mappings) in &sources_by_type_and_join {
        handlers.extend(build_source_handler(
            source_type,
            join_key,
            mappings,
            aggregate_conditions,
            primary_keys,
            lookup_indexes,
            idls,
        )?);
    }

    // Group events by instruction and join key
    #[allow(clippy::type_complexity)]
    let mut events_by_instruction_and_join: BTreeMap<
        (String, Option<String>),
        Vec<(String, parse::EventAttribute, syn::Type)>,
    > = BTreeMap::new();
    for (instruction, event_mappings) in events_by_instruction {
        for event_mapping in event_mappings {
            let join_on_str = get_join_on_field(&event_mapping.1.join_on);
            let key = (instruction.clone(), join_on_str);
            events_by_instruction_and_join
                .entry(key)
                .or_default()
                .push(event_mapping.clone());
        }
    }

    for ((instruction, join_key), event_mappings) in &events_by_instruction_and_join {
        for event_mappings in split_event_mappings_by_lookup(join_key, event_mappings)? {
            if let Some(handler) = build_event_handler(
                instruction,
                join_key,
                &event_mappings,
                primary_keys,
                lookup_indexes,
                idls,
            )? {
                handlers.push(handler);
            }
        }
    }

    Ok(handlers)
}

fn build_source_handler(
    source_type: &str,
    join_key: &Option<String>,
    mappings: &[parse::MapAttribute],
    aggregate_conditions: &BTreeMap<String, ConditionExpr>,
    primary_keys: &[String],
    lookup_indexes: &[(String, Option<String>)],
    idls: IdlLookup,
) -> syn::Result<Vec<SerializableHandlerSpec>> {
    let account_type = source_type.split("::").last().unwrap_or(source_type);
    let idl = find_idl_for_type(source_type, idls);
    let is_instruction = mappings.iter().any(|m| m.is_instruction);
    // CPI events are sourced from `::events::` submodule paths (e.g. generated_sdk::events::Swap).
    // A bare field is the event payload under "data.*"; `accounts::name` is the emitting
    // instruction's account under "accounts.*".
    let is_cpi_event = source_type.contains("::events::");

    // Skip event-derived mappings
    if is_instruction
        && mappings
            .iter()
            .any(|m| m.target_field_name.starts_with("events."))
    {
        return Ok(Vec::new());
    }

    if !is_instruction && !is_cpi_event {
        if let Some(idl) = idl {
            lookup_account(idl, account_type)
                .map_err(|error| idl_error_to_syn(mappings[0].source_type_span, error))?;
        }
    }

    let mut serializable_mappings = Vec::new();
    // The parsed attribute and primary-key field behind each serializable mapping.
    let mut mapping_origins: Vec<(&parse::MapAttribute, Option<String>)> = Vec::new();
    let mut has_primary_key = false;
    let mut primary_field = None;

    for mapping in mappings {
        // Skip conditional aggregates
        if aggregate_conditions.contains_key(&mapping.target_field_name) {
            continue;
        }

        let context_field = context_field_name(&mapping.source_field_name);
        let source = if mapping.is_whole_source {
            let field_transforms = if mapping
                .source_field_name
                .starts_with("__snapshot_with_transforms:")
            {
                let transforms_str = mapping
                    .source_field_name
                    .strip_prefix("__snapshot_with_transforms:")
                    .unwrap_or("");
                transforms_str
                    .split(',')
                    .filter_map(|pair| {
                        let parts: Vec<&str> = pair.split('=').collect();
                        if parts.len() == 2 {
                            parse_transformation(parts[1]).map(|t| (parts[0].to_string(), t))
                        } else {
                            None
                        }
                    })
                    .collect()
            } else {
                BTreeMap::new()
            };

            MappingSource::AsCapture { field_transforms }
        } else if let Some(field) = context_field {
            MappingSource::FromContext {
                field: field.to_string(),
            }
        } else {
            let field_path = if is_cpi_event {
                if mapping.source_field_name.is_empty() {
                    FieldPath::new(&["data"])
                } else {
                    FieldPath::new(&[
                        event_field_prefix(mapping.source_field_location.as_ref()),
                        &mapping.source_field_name,
                    ])
                }
            } else if is_instruction {
                if mapping.source_field_name.is_empty() {
                    FieldPath::new(&["data"])
                } else {
                    let prefix = if let Some(idl) = idl {
                        match lookup_instruction_field(
                            idl,
                            account_type,
                            &mapping.source_field_name,
                        )
                        .map_err(|error| {
                            idl_error_to_syn(span_for_map_lookup_error(mapping, &error), error)
                        })?
                        .kind
                        {
                            InstructionFieldKind::Account => "accounts",
                            InstructionFieldKind::Arg => "data",
                        }
                    } else {
                        "data"
                    };
                    FieldPath::new(&[prefix, &mapping.source_field_name])
                }
            } else if mapping.source_field_name.is_empty() {
                FieldPath::new(&[])
            } else {
                FieldPath::new(&[&mapping.source_field_name])
            };

            MappingSource::FromSource {
                path: field_path,
                default: None,
                transform: mapping
                    .transform
                    .as_ref()
                    .and_then(|t| parse_transformation(t)),
            }
        };

        let population = parse_population_strategy(&mapping.strategy);

        let condition = mapping.condition.clone();

        let when = mapping.when.as_ref().map(|when_path| {
            let instr_type = path_to_string(when_path);
            let instr_base = instr_type.split("::").last().unwrap_or(&instr_type);
            let program_name = program_name_for_type(&instr_type, idls);
            let is_cpi_event = instr_type.contains("::events::");
            scoped_instruction_or_cpi_event_type(program_name, instr_base, is_cpi_event)
        });

        let stop = mapping.stop.as_ref().map(|stop_path| {
            let instr_type = path_to_string(stop_path);
            let instr_base = instr_type.split("::").last().unwrap_or(&instr_type);
            let program_name = program_name_for_type(&instr_type, idls);
            let is_cpi_event = instr_type.contains("::events::");
            scoped_instruction_or_cpi_event_type(program_name, instr_base, is_cpi_event)
        });

        serializable_mappings.push(SerializableFieldMapping {
            target_path: mapping.target_field_name.clone(),
            source,
            transform: None,
            population,
            condition,
            when,
            stop,
            emit: mapping.emit,
        });

        let mut mapping_primary_field = None;
        if mapping.is_primary_key {
            has_primary_key = true;
            if let Some(field) = context_field {
                primary_field = Some(format!("__update_context.{field}"));
            } else if is_cpi_event {
                primary_field = Some(format!(
                    "{}.{}",
                    event_field_prefix(mapping.source_field_location.as_ref()),
                    mapping.source_field_name
                ));
            } else if is_instruction {
                let prefix = if let Some(idl) = idl {
                    match lookup_instruction_field(idl, account_type, &mapping.source_field_name)
                        .map_err(|error| {
                            idl_error_to_syn(span_for_map_lookup_error(mapping, &error), error)
                        })?
                        .kind
                    {
                        InstructionFieldKind::Account => "accounts",
                        InstructionFieldKind::Arg => "data",
                    }
                } else {
                    "data"
                };
                primary_field = Some(format!("{}.{}", prefix, mapping.source_field_name));
            } else {
                primary_field = Some(mapping.source_field_name.clone());
            }
            mapping_primary_field = primary_field.clone();
        }
        mapping_origins.push((mapping, mapping_primary_field));
    }

    let is_aggregation = mappings.iter().any(|m| {
        matches!(
            m.strategy.as_str(),
            "Sum" | "Count" | "Min" | "Max" | "UniqueCount"
        )
    });

    let lookup_by_path = |fs: &parse::FieldSpec| {
        format!(
            "{}.{}",
            lookup_by_prefix(fs.explicit_location.as_ref(), is_cpi_event),
            fs.ident
        )
    };

    // Try to find lookup_by from the first mapping that has it
    let lookup_by_field = mappings
        .iter()
        .find_map(|m| m.lookup_by.as_ref())
        .map(lookup_by_path);

    // Each primary-key mapping and each aggregate `lookup_by` declares the key
    // its update is routed by. When one source declares several distinct keys
    // (e.g. `split_position` counting `split_source_count` via `first_position`
    // and `split_child_count` via `second_position`), every key gets its own
    // handler instead of all updates riding whichever key was seen last.
    let honors_lookup_by = is_aggregation && (is_instruction || is_cpi_event);
    let primary_key_paths: Vec<&str> = mapping_origins
        .iter()
        .filter_map(|(_, field)| field.as_deref())
        .collect();
    let lookup_route = |path: String| {
        let leaf = path.split('.').next_back().unwrap_or(&path);
        let is_primary_key_field = primary_key_paths.contains(&path.as_str())
            || primary_keys
                .iter()
                .any(|pk| pk.split('.').next_back().unwrap_or(pk) == leaf);
        if is_primary_key_field {
            RouteKey::Embedded(path)
        } else {
            RouteKey::Lookup(path)
        }
    };
    let mut declared_routes: Vec<RouteKey> = Vec::new();
    for mapping in mappings {
        let route = if mapping.is_primary_key {
            mapping_origins
                .iter()
                .find(|(origin, _)| std::ptr::eq(*origin, mapping))
                .and_then(|(_, field)| field.clone())
                .map(RouteKey::Embedded)
        } else if honors_lookup_by {
            mapping
                .lookup_by
                .as_ref()
                .map(|fs| lookup_route(lookup_by_path(fs)))
        } else {
            None
        };
        if let Some(route) = route {
            if !declared_routes.contains(&route) {
                declared_routes.push(route);
            }
        }
    }

    if declared_routes.len() > 1 {
        // Conditional aggregates compile to instruction hooks without a key of
        // their own, so they cannot follow one of several keys.
        if let Some(conditional) = mappings
            .iter()
            .find(|m| aggregate_conditions.contains_key(&m.target_field_name))
        {
            return Err(syn::Error::new(
                conditional.attr_span,
                format!(
                    "conditional aggregate `{}` on '{}' is not supported: this source updates \
                     the entity through several keys ({}), and a conditional aggregate cannot \
                     choose one of them.",
                    conditional.target_field_name,
                    source_type,
                    declared_routes
                        .iter()
                        .map(|route| format!("`{}`", route.path()))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            ));
        }
        return split_source_handler_by_route(
            source_type,
            &declared_routes,
            &mapping_origins,
            serializable_mappings,
            honors_lookup_by,
            &lookup_route,
            &lookup_by_path,
            SourceSpec::Source {
                program_id: None,
                discriminator: None,
                type_name: source_type_name(source_type, is_instruction, is_cpi_event, idls),
                serialization: source_serialization(idl, account_type, is_instruction),
                is_account: !is_instruction && !is_cpi_event,
            },
        );
    }

    // A keyless instruction or CPI-event source routes by the source field that
    // key validation accepted for it (a primary-key or lookup-index field).
    // These events carry no `__account_address`, so the account-handler
    // fallback below would leave the key null and drop the update.
    let infer_route = || {
        if is_instruction || is_cpi_event {
            infer_source_route(
                source_type,
                &mapping_origins,
                &serializable_mappings,
                primary_keys,
                lookup_indexes,
            )
        } else {
            Ok(None)
        }
    };

    let key_resolution = if has_primary_key {
        let primary_field_str = primary_field.as_deref().unwrap_or("");
        let segments: Vec<&str> = primary_field_str.split('.').collect();
        KeyResolutionStrategy::Embedded {
            primary_field: FieldPath::new(&segments),
        }
    } else if let (true, Some(lookup_field)) = (honors_lookup_by, &lookup_by_field) {
        // Check if lookup_by points directly to a field that matches the primary key.
        // If the lookup_by field name matches the primary key field name, it means we're
        // pointing directly to the primary key field itself (e.g., accounts.mint when
        // id.mint is the primary key), so we should use Embedded resolution.
        //
        // If it doesn't match the primary key, we need a Lookup resolution to do a
        // reverse lookup (e.g., accounts.bonding_curve -> mint via PDA lookup).
        let lookup_field_name = lookup_field.split('.').next_back().unwrap_or(lookup_field);

        // Check if any primary key field name matches the lookup_by field name
        // Primary keys are like "id.mint", so we compare the last segment
        let is_primary_key_field = primary_keys
            .iter()
            .any(|pk| pk.split('.').next_back().unwrap_or(pk) == lookup_field_name);

        if is_primary_key_field {
            // The lookup_by field IS the primary key itself - use Embedded
            let segments: Vec<&str> = lookup_field.split('.').collect();
            KeyResolutionStrategy::Embedded {
                primary_field: FieldPath::new(&segments),
            }
        } else {
            // The lookup_by field is a PDA that needs reverse lookup
            let segments: Vec<&str> = lookup_field.split('.').collect();
            KeyResolutionStrategy::Lookup {
                primary_field: FieldPath::new(&segments),
            }
        }
    } else if is_aggregation && is_instruction {
        // No lookup_by: fall back to join_key, the inferred key field, or a
        // resolver-provided key.
        if let Some(ref join_field) = join_key {
            KeyResolutionStrategy::Lookup {
                primary_field: FieldPath::new(&[join_field]),
            }
        } else if let Some(route) = infer_route()? {
            route.to_strategy()
        } else {
            // No lookup_by specified - use embedded with empty path
            // The instruction handler will need the primary key from elsewhere
            KeyResolutionStrategy::Embedded {
                primary_field: FieldPath::new(&[]),
            }
        }
    } else if let Some(ref join_field) = join_key {
        KeyResolutionStrategy::Lookup {
            primary_field: FieldPath::new(&[join_field]),
        }
    } else if let Some(route) = infer_route()? {
        route.to_strategy()
    } else if !lookup_indexes.is_empty() && !is_instruction {
        // Entity has lookup indexes and this is an account handler without an embedded
        // primary key. Use Lookup strategy with __account_address so the VM can resolve
        // the entity via the lookup index populated by instruction handlers.
        // __resolved_primary_key from explicit resolvers takes precedence if set.
        KeyResolutionStrategy::Lookup {
            primary_field: FieldPath::new(&["__account_address"]),
        }
    } else {
        KeyResolutionStrategy::Embedded {
            primary_field: FieldPath::new(&[]),
        }
    };

    Ok(vec![SerializableHandlerSpec {
        source: SourceSpec::Source {
            program_id: None,
            discriminator: None,
            type_name: source_type_name(source_type, is_instruction, is_cpi_event, idls),
            serialization: source_serialization(idl, account_type, is_instruction),
            is_account: !is_instruction && !is_cpi_event,
        },
        key_resolution,
        mappings: serializable_mappings,
        conditions: Vec::new(),
        emit: true,
    }])
}

/// Where an event field lives in the decoded event value: the payload under `data`, unless
/// `accounts::` selects the emitting instruction's account.
fn event_field_prefix(location: Option<&parse::FieldLocation>) -> &'static str {
    match location {
        Some(parse::FieldLocation::Account) => "accounts",
        Some(parse::FieldLocation::InstructionArg) | None => "data",
    }
}

/// Where a `lookup_by`-style key lives. An explicit `accounts::` / `data::` wins; a bare name is
/// the payload for a CPI event and an account for an instruction.
fn lookup_by_prefix(location: Option<&parse::FieldLocation>, is_cpi_event: bool) -> &'static str {
    match location {
        Some(parse::FieldLocation::Account) => "accounts",
        Some(parse::FieldLocation::InstructionArg) => "data",
        None if is_cpi_event => "data",
        None => "accounts",
    }
}

/// Scoped event type name of a `#[map]` source, e.g. `cp_amm::SplitPositionIxState`.
fn source_type_name(
    source_type: &str,
    is_instruction: bool,
    is_cpi_event: bool,
    idls: IdlLookup,
) -> String {
    let account_type = source_type.split("::").last().unwrap_or(source_type);
    // Determine type suffix:
    // - CPI events (from `::events::` submodule) use "CpiEvent"
    // - Instructions use "IxState"
    // - Account state uses "State"
    let type_suffix = if is_cpi_event {
        "CpiEvent"
    } else if is_instruction {
        "IxState"
    } else {
        "State"
    };
    if let Some(program_name) = program_name_for_type(source_type, idls) {
        format!("{}::{}{}", program_name, account_type, type_suffix)
    } else {
        format!("{}{}", account_type, type_suffix)
    }
}

fn source_serialization(
    idl: Option<&idl_parser::IdlSpec>,
    account_type: &str,
    is_instruction: bool,
) -> Option<IdlSerializationSnapshot> {
    if is_instruction {
        return None;
    }
    idl.and_then(|idl| {
        idl.types
            .iter()
            .find(|t| t.name == account_type)
            .and_then(|t| t.serialization.as_ref())
            .map(|s| match s {
                idl_parser::IdlSerialization::Borsh => IdlSerializationSnapshot::Borsh,
                idl_parser::IdlSerialization::Bytemuck => IdlSerializationSnapshot::Bytemuck,
                idl_parser::IdlSerialization::BytemuckUnsafe => {
                    IdlSerializationSnapshot::BytemuckUnsafe
                }
            })
    })
}

/// The key of a keyless instruction or CPI-event source: the event field of a
/// mapping whose leaf names a primary-key field (used directly) or a
/// lookup-index field (resolved through that index). This is the same field
/// key validation accepts for such a source. Several distinct candidate fields
/// are ambiguous and rejected.
fn infer_source_route(
    source_type: &str,
    mapping_origins: &[(&parse::MapAttribute, Option<String>)],
    serializable_mappings: &[SerializableFieldMapping],
    primary_keys: &[String],
    lookup_indexes: &[(String, Option<String>)],
) -> syn::Result<Option<RouteKey>> {
    let leaf = |path: &str| path.split('.').next_back().unwrap_or(path).to_string();
    let primary_key_leafs: HashSet<String> = primary_keys.iter().map(|pk| leaf(pk)).collect();
    let mut lookup_index_leafs: HashSet<String> = HashSet::new();
    for (field, _) in lookup_indexes {
        let field_leaf = leaf(field);
        if let Some(stripped) = field_leaf.strip_suffix("_address") {
            lookup_index_leafs.insert(stripped.to_string());
        }
        lookup_index_leafs.insert(field_leaf);
    }

    let mut candidates: Vec<(RouteKey, proc_macro2::Span)> = Vec::new();
    for ((origin, _), mapping) in mapping_origins.iter().zip(serializable_mappings) {
        let Some(path) = mapping_source_path(mapping) else {
            continue;
        };
        let path_leaf = leaf(&path);
        let route = if primary_key_leafs.contains(&path_leaf) {
            RouteKey::Embedded(path)
        } else if lookup_index_leafs.contains(&path_leaf) {
            RouteKey::Lookup(path)
        } else {
            continue;
        };
        if !candidates.iter().any(|(existing, _)| *existing == route) {
            candidates.push((route, origin.attr_span));
        }
    }

    match candidates.len() {
        0 => Ok(None),
        1 => Ok(candidates.pop().map(|(route, _)| route)),
        _ => Err(syn::Error::new(
            candidates[1].1,
            format!(
                "'{}' has no `primary_key` mapping or `lookup_by`, and its mappings read \
                 several key fields ({}); it cannot tell which entity instance to update. \
                 Mark the key mapping `primary_key` or route the update with `lookup_by`.",
                source_type,
                candidates
                    .iter()
                    .map(|(route, _)| format!("`{}`", route.path()))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        )),
    }
}

/// How one update of a source reaches its entity instance.
#[derive(Debug, Clone, PartialEq)]
enum RouteKey {
    /// The dotted event path holds the primary key itself.
    Embedded(String),
    /// The dotted event path is resolved to the primary key via a lookup index.
    Lookup(String),
}

impl RouteKey {
    fn path(&self) -> &str {
        match self {
            RouteKey::Embedded(path) | RouteKey::Lookup(path) => path,
        }
    }

    fn to_strategy(&self) -> KeyResolutionStrategy {
        let segments: Vec<&str> = self.path().split('.').collect();
        match self {
            RouteKey::Embedded(_) => KeyResolutionStrategy::Embedded {
                primary_field: FieldPath::new(&segments),
            },
            RouteKey::Lookup(_) => KeyResolutionStrategy::Lookup {
                primary_field: FieldPath::new(&segments),
            },
        }
    }
}

fn mapping_source_path(mapping: &SerializableFieldMapping) -> Option<String> {
    match &mapping.source {
        MappingSource::FromSource { path, .. } if !path.segments.is_empty() => {
            Some(path.segments.join("."))
        }
        _ => None,
    }
}

/// Split one source's mappings into one handler per declared routing key.
///
/// Mappings that declare no key of their own (for example the lookup-index
/// mapping `id.first_position <- accounts.first_position`) follow the key that
/// reads the same event field. A mapping that matches no key cannot be routed
/// unambiguously and is rejected at compile time.
#[allow(clippy::too_many_arguments)]
fn split_source_handler_by_route(
    source_type: &str,
    routes: &[RouteKey],
    mapping_origins: &[(&parse::MapAttribute, Option<String>)],
    serializable_mappings: Vec<SerializableFieldMapping>,
    honors_lookup_by: bool,
    lookup_route: &dyn Fn(String) -> RouteKey,
    lookup_by_path: &dyn Fn(&parse::FieldSpec) -> String,
    source: SourceSpec,
) -> syn::Result<Vec<SerializableHandlerSpec>> {
    let key_list = routes
        .iter()
        .map(|route| format!("`{}`", route.path()))
        .collect::<Vec<_>>()
        .join(", ");

    let mut errors = crate::diagnostic::ErrorCollector::default();
    let mut routed: Vec<Vec<SerializableFieldMapping>> = vec![Vec::new(); routes.len()];

    for ((origin, primary_field), mapping) in mapping_origins.iter().zip(serializable_mappings) {
        let declared = if let Some(field) = primary_field {
            Some(RouteKey::Embedded(field.clone()))
        } else if honors_lookup_by {
            origin
                .lookup_by
                .as_ref()
                .map(|fs| lookup_route(lookup_by_path(fs)))
        } else {
            None
        };
        let index = match declared {
            Some(route) => routes.iter().position(|candidate| *candidate == route),
            None => mapping_source_path(&mapping)
                .and_then(|path| routes.iter().position(|route| route.path() == path)),
        };
        match index {
            Some(index) => routed[index].push(mapping),
            None => errors.push(syn::Error::new(
                origin.attr_span,
                format!(
                    "`{}` from '{}' cannot be routed to one entity instance: this source \
                     updates the entity through several keys ({}), and this mapping reads \
                     none of them. Map it from one of those fields, or give it its own \
                     `lookup_by`.",
                    mapping.target_path, source_type, key_list
                ),
            )),
        }
    }

    errors.finish()?;

    Ok(routes
        .iter()
        .zip(routed)
        .map(|(route, mappings)| SerializableHandlerSpec {
            source: source.clone(),
            key_resolution: route.to_strategy(),
            mappings,
            conditions: Vec::new(),
            emit: true,
        })
        .collect())
}

fn span_for_map_lookup_error(
    mapping: &parse::MapAttribute,
    error: &IdlSearchError,
) -> proc_macro2::Span {
    match error {
        IdlSearchError::NotFound { section, .. }
            if section == "instructions" || section == "accounts" || section == "types" =>
        {
            mapping.source_type_span
        }
        IdlSearchError::NotFound { section, .. } if section.starts_with("instruction fields") => {
            mapping.source_field_span
        }
        IdlSearchError::InvalidPath { .. } => mapping.attr_span,
        _ => mapping.attr_span,
    }
}

fn span_for_event_lookup_error(
    event_attr: &parse::EventAttribute,
    field_spec: &parse::FieldSpec,
    error: &IdlSearchError,
) -> proc_macro2::Span {
    match error {
        IdlSearchError::NotFound { section, .. } if section == "instructions" => {
            event_attr.instruction_span.unwrap_or(event_attr.attr_span)
        }
        IdlSearchError::NotFound { section, .. } if section.starts_with("instruction fields") => {
            field_spec.ident.span()
        }
        IdlSearchError::InvalidPath { .. } => {
            event_attr.instruction_span.unwrap_or(event_attr.attr_span)
        }
        _ => field_spec.ident.span(),
    }
}

type EventMapping = (String, parse::EventAttribute, syn::Type);

/// Split an `#[event]` group whose mappings name different `lookup_by`
/// fields into one group per field, so each event capture is routed by its own
/// key. Groups with at most one `lookup_by` field are returned unchanged. A
/// mapping without `lookup_by` in a group that has several is ambiguous and
/// rejected.
fn split_event_mappings_by_lookup(
    join_key: &Option<String>,
    event_mappings: &[EventMapping],
) -> syn::Result<Vec<Vec<EventMapping>>> {
    let lookup_key = |attr: &parse::EventAttribute| {
        attr.lookup_by.as_ref().map(|field_spec| {
            let location = match field_spec.explicit_location {
                Some(parse::FieldLocation::Account) => "accounts::",
                Some(parse::FieldLocation::InstructionArg) => "data::",
                None => "",
            };
            format!("{}{}", location, field_spec.ident)
        })
    };

    let mut keys: Vec<String> = Vec::new();
    for (_, attr, _) in event_mappings {
        if let Some(key) = lookup_key(attr) {
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
    }
    // `join_on` takes precedence over `lookup_by` when routing events.
    if keys.len() <= 1 || join_key.is_some() {
        return Ok(vec![event_mappings.to_vec()]);
    }

    let mut groups: Vec<Vec<EventMapping>> = vec![Vec::new(); keys.len()];
    let mut errors = crate::diagnostic::ErrorCollector::default();
    for event_mapping in event_mappings {
        match lookup_key(&event_mapping.1)
            .and_then(|key| keys.iter().position(|candidate| *candidate == key))
        {
            Some(index) => groups[index].push(event_mapping.clone()),
            None => errors.push(syn::Error::new(
                event_mapping.1.attr_span,
                format!(
                    "`{}` cannot be routed to one entity instance: other events from the same \
                     instruction use different `lookup_by` fields ({}). Add `lookup_by` to \
                     choose one.",
                    event_mapping.0,
                    keys.join(", ")
                ),
            )),
        }
    }
    errors.finish()?;
    Ok(groups)
}

fn build_event_handler(
    instruction: &str,
    join_key: &Option<String>,
    event_mappings: &[(String, parse::EventAttribute, syn::Type)],
    primary_keys: &[String],
    lookup_indexes: &[(String, Option<String>)],
    idls: IdlLookup,
) -> syn::Result<Option<SerializableHandlerSpec>> {
    let instruction_path_str = event_mappings
        .first()
        .and_then(|(_, attr, _)| {
            attr.from_instruction
                .as_ref()
                .or(attr.inferred_instruction.as_ref())
        })
        .map(path_to_string);
    let (idl, program_name) = match &instruction_path_str {
        Some(path_str) => (
            find_idl_for_type(path_str, idls),
            program_name_for_type(path_str, idls),
        ),
        None => {
            // The instruction is program-qualified ("pump::Trade"), so resolve
            // from its own program instead of whichever IDL comes first.
            let program = instruction.split("::").next().unwrap_or_default();
            let owning = idls
                .iter()
                .find(|(_, idl)| idl.get_name() == program)
                .map(|(_, idl)| *idl)
                .or_else(|| idls.first().map(|(_, idl)| *idl));
            (owning, owning.map(|idl| idl.get_name()))
        }
    };
    let parts: Vec<&str> = instruction.split("::").collect();
    if parts.len() != 2 {
        return Ok(None);
    }

    let program_id = parts[0];
    let instruction_type = parts[1];
    let field_resolves_key = |field_name: &str| {
        primary_keys
            .iter()
            .any(|pk| pk.split('.').next_back().unwrap_or(pk) == field_name)
            || lookup_indexes.iter().any(|(field, _)| {
                let leaf = field.split('.').next_back().unwrap_or(field);
                leaf == field_name || leaf.strip_suffix("_address") == Some(field_name)
            })
    };

    let mut serializable_mappings = Vec::new();

    for (target_field, event_attr, _field_type) in event_mappings {
        let has_fields =
            !event_attr.capture_fields.is_empty() || !event_attr.capture_fields_legacy.is_empty();

        let source = if !has_fields {
            MappingSource::AsEvent { fields: vec![] }
        } else if !event_attr.capture_fields.is_empty() {
            let captured_fields: Vec<MappingSource> = event_attr
                .capture_fields
                .iter()
                .map(|field_spec| -> syn::Result<MappingSource> {
                    let field_name = field_spec.ident.to_string();
                    let transform = event_attr
                        .field_transforms
                        .get(&field_name)
                        .and_then(|t| parse_transformation(&t.to_string()));

                    let field_location = if let Some(explicit_loc) = &field_spec.explicit_location {
                        explicit_loc.clone()
                    } else {
                        let instruction_path = event_attr
                            .from_instruction
                            .as_ref()
                            .or(event_attr.inferred_instruction.as_ref());

                        if let Some(instr_path) = instruction_path {
                            find_field_in_instruction(instr_path, &field_name, idl).map_err(
                                |error| {
                                    idl_error_to_syn(
                                        span_for_event_lookup_error(event_attr, field_spec, &error),
                                        error,
                                    )
                                },
                            )?
                        } else {
                            parse::FieldLocation::InstructionArg
                        }
                    };

                    let field_path = match field_location {
                        parse::FieldLocation::Account => FieldPath::new(&["accounts", &field_name]),
                        parse::FieldLocation::InstructionArg => {
                            FieldPath::new(&["data", &field_name])
                        }
                    };

                    Ok(MappingSource::FromSource {
                        path: field_path,
                        default: None,
                        transform,
                    })
                })
                .collect::<syn::Result<Vec<_>>>()?;

            MappingSource::AsEvent {
                fields: captured_fields,
            }
        } else {
            let captured_fields: Vec<MappingSource> = event_attr
                .capture_fields_legacy
                .iter()
                .map(|field| {
                    let transform = event_attr
                        .field_transforms_legacy
                        .get(field)
                        .and_then(|t| parse_transformation(t));
                    MappingSource::FromSource {
                        path: FieldPath::new(&["data", field]),
                        default: None,
                        transform,
                    }
                })
                .collect();

            MappingSource::AsEvent {
                fields: captured_fields,
            }
        };

        let population = parse_population_strategy(&event_attr.strategy);

        serializable_mappings.push(SerializableFieldMapping {
            target_path: target_field.clone(),
            source,
            transform: None,
            population,
            condition: None,
            when: None,
            stop: None,
            emit: true,
        });
    }

    // Determine key resolution for events
    let (lookup_field_name, lookup_field_location) = if let Some(ref join_field_name) = join_key {
        (
            join_field_name.clone(),
            parse::FieldLocation::InstructionArg,
        )
    } else if let Some((_, first_event_attr, _)) =
        event_mappings.iter().find(|(_, event_attr, _)| {
            event_attr
                .lookup_by
                .as_ref()
                .is_some_and(|field_spec| field_resolves_key(&field_spec.ident.to_string()))
        })
    {
        if let Some(ref lookup_by_field_spec) = first_event_attr.lookup_by {
            let field_name = lookup_by_field_spec.ident.to_string();

            let field_location = if let Some(explicit_loc) = &lookup_by_field_spec.explicit_location
            {
                explicit_loc.clone()
            } else {
                let instruction_path = first_event_attr
                    .from_instruction
                    .as_ref()
                    .or(first_event_attr.inferred_instruction.as_ref());

                if let Some(instr_path) = instruction_path {
                    find_field_in_instruction(instr_path, &field_name, idl).map_err(|error| {
                        idl_error_to_syn(
                            span_for_event_lookup_error(
                                first_event_attr,
                                lookup_by_field_spec,
                                &error,
                            ),
                            error,
                        )
                    })?
                } else {
                    parse::FieldLocation::InstructionArg
                }
            };

            (field_name, field_location)
        } else {
            (String::new(), parse::FieldLocation::InstructionArg)
        }
    } else if let Some((_, first_event_attr, _)) = event_mappings
        .iter()
        .find(|(_, event_attr, _)| event_attr.lookup_by.is_some())
    {
        if let Some(ref lookup_by_field_spec) = first_event_attr.lookup_by {
            let field_name = lookup_by_field_spec.ident.to_string();

            let field_location = if let Some(explicit_loc) = &lookup_by_field_spec.explicit_location
            {
                explicit_loc.clone()
            } else {
                let instruction_path = first_event_attr
                    .from_instruction
                    .as_ref()
                    .or(first_event_attr.inferred_instruction.as_ref());

                if let Some(instr_path) = instruction_path {
                    find_field_in_instruction(instr_path, &field_name, idl).map_err(|error| {
                        idl_error_to_syn(
                            span_for_event_lookup_error(
                                first_event_attr,
                                lookup_by_field_spec,
                                &error,
                            ),
                            error,
                        )
                    })?
                } else {
                    parse::FieldLocation::InstructionArg
                }
            };

            (field_name, field_location)
        } else {
            (String::new(), parse::FieldLocation::InstructionArg)
        }
    } else {
        (String::new(), parse::FieldLocation::InstructionArg)
    };

    let is_temporal_lookup = lookup_indexes.iter().any(|(field, temporal_field)| {
        field.ends_with(&lookup_field_name) && temporal_field.is_some()
    });

    let lookup_field_prefix = match lookup_field_location {
        parse::FieldLocation::Account => "accounts",
        parse::FieldLocation::InstructionArg => "data",
    };

    let key_resolution = if is_temporal_lookup {
        let index_name = format!("{}_temporal_index", lookup_field_name);
        KeyResolutionStrategy::TemporalLookup {
            lookup_field: FieldPath::new(&[lookup_field_prefix, &lookup_field_name]),
            timestamp_field: FieldPath::new(&["timestamp"]),
            index_name,
        }
    } else if !lookup_field_name.is_empty() {
        // Check if lookup_by points directly to a field that matches a lookup_index
        // If the lookup_by field is NOT in the lookup_indexes, it means we're pointing
        // directly to the primary key field itself (e.g., accounts.mint when id.mint is the pk),
        // so we should use Embedded resolution instead of Lookup.
        //
        // Check if any primary key field name matches the lookup_by field name
        // Primary keys are like "id.mint", so we compare the last segment
        let is_primary_key_field = primary_keys
            .iter()
            .any(|pk| pk.split('.').next_back().unwrap_or(pk) == lookup_field_name);

        if is_primary_key_field {
            // The lookup_by field IS the primary key itself - use Embedded
            KeyResolutionStrategy::Embedded {
                primary_field: FieldPath::new(&[lookup_field_prefix, &lookup_field_name]),
            }
        } else {
            // The lookup_by field is NOT the primary key - needs reverse lookup
            KeyResolutionStrategy::Lookup {
                primary_field: FieldPath::new(&[lookup_field_prefix, &lookup_field_name]),
            }
        }
    } else {
        KeyResolutionStrategy::Lookup {
            primary_field: FieldPath::new(&[]),
        }
    };

    let is_cpi_event = instruction_path_str
        .as_deref()
        .is_some_and(|path| path.contains("::events::"));
    let type_name =
        scoped_instruction_or_cpi_event_type(program_name, instruction_type, is_cpi_event);

    Ok(Some(SerializableHandlerSpec {
        source: SourceSpec::Source {
            program_id: Some(program_id.to_string()),
            discriminator: None,
            type_name,
            serialization: None,
            is_account: false,
        },
        key_resolution,
        mappings: serializable_mappings,
        conditions: Vec::new(),
        emit: true,
    }))
}

// ============================================================================
// Hook Building
// ============================================================================

fn build_resolver_hooks_ast(
    resolver_hooks: &[parse::ResolveKeyAttribute],
    idls: IdlLookup,
) -> Vec<ResolverHook> {
    resolver_hooks
        .iter()
        .map(|hook| {
            let account_type = path_to_string(&hook.account_path);
            let account_base = account_type.split("::").last().unwrap();
            let program_name = program_name_for_type(&account_type, idls);
            let idl = find_idl_for_type(&account_type, idls);
            let account_type_state = if let Some(program_name) = program_name {
                format!("{}::{}State", program_name, account_base)
            } else {
                format!("{}State", account_base)
            };

            let strategy = match hook.strategy.as_str() {
                "pda_reverse_lookup" => {
                    let discriminators = hook
                        .queue_until
                        .iter()
                        .filter_map(|instr_path| {
                            idl.and_then(|idl| {
                                let instr_name = instr_path.segments.last()?.ident.to_string();
                                let instr_snake = crate::utils::to_snake_case(&instr_name);
                                idl.instructions
                                    .iter()
                                    .find(|instr| instr.name == instr_snake)
                                    .map(|instr| instr.get_discriminator())
                            })
                        })
                        .collect();

                    ResolverStrategy::PdaReverseLookup {
                        lookup_name: hook
                            .lookup_name
                            .clone()
                            .unwrap_or_else(|| "default_pda_lookup".to_string()),
                        queue_discriminators: discriminators,
                    }
                }
                _ => ResolverStrategy::PdaReverseLookup {
                    lookup_name: "default_pda_lookup".to_string(),
                    queue_discriminators: Vec::new(),
                },
            };

            ResolverHook {
                account_type: account_type_state,
                strategy,
            }
        })
        .collect()
}

fn auto_generate_lookup_resolvers(
    handlers: &[SerializableHandlerSpec],
    existing_resolvers: &[ResolverHook],
    sources_by_type: &BTreeMap<String, Vec<parse::MapAttribute>>,
    idls: IdlLookup,
) -> Vec<ResolverHook> {
    let mut auto_hooks = Vec::new();

    let account_types_needing_resolver: Vec<String> = handlers
        .iter()
        .filter_map(|handler| {
            if let KeyResolutionStrategy::Lookup { primary_field } = &handler.key_resolution {
                if primary_field.segments.as_slice() == ["__account_address"] {
                    let SourceSpec::Source { ref type_name, .. } = handler.source;
                    if type_name.ends_with("State") && !type_name.ends_with("IxState") {
                        return Some(type_name.to_string());
                    }
                }
            }
            None
        })
        .collect();

    if account_types_needing_resolver.is_empty() {
        return auto_hooks;
    }

    let mut queue_discriminators: Vec<Vec<u8>> = Vec::new();
    for mappings in sources_by_type.values() {
        for mapping in mappings {
            if mapping.is_instruction && mapping.is_lookup_index {
                let source_path_str = path_to_string(&mapping.source_type_path);
                let idl = find_idl_for_type(&source_path_str, idls);
                if let Some(idl) = idl {
                    let instr_name = mapping
                        .source_type_path
                        .segments
                        .last()
                        .map(|s| s.ident.to_string())
                        .unwrap_or_default();
                    let instr_snake = crate::utils::to_snake_case(&instr_name);

                    if let Some(idl_instr) = idl.instructions.iter().find(|i| i.name == instr_snake)
                    {
                        let disc = idl_instr.get_discriminator();
                        if !disc.is_empty() && !queue_discriminators.contains(&disc) {
                            queue_discriminators.push(disc);
                        }
                    }
                }
            }
        }
    }

    let mut seen_account_types = HashSet::new();
    for account_type in account_types_needing_resolver {
        if !seen_account_types.insert(account_type.clone()) {
            continue;
        }
        if existing_resolvers
            .iter()
            .any(|r| r.account_type == account_type)
        {
            continue;
        }
        auto_hooks.push(ResolverHook {
            account_type,
            strategy: ResolverStrategy::PdaReverseLookup {
                lookup_name: "default_pda_lookup".to_string(),
                queue_discriminators: queue_discriminators.clone(),
            },
        });
    }

    auto_hooks
}

fn build_instruction_hooks_ast(
    pda_registrations: &[parse::RegisterPdaAttribute],
    derive_from_mappings: &BTreeMap<String, Vec<parse::DeriveFromAttribute>>,
    aggregate_conditions: &BTreeMap<String, ConditionExpr>,
    sources_by_type: &BTreeMap<String, Vec<parse::MapAttribute>>,
    idls: IdlLookup,
) -> Vec<InstructionHook> {
    // Hooks per instruction type. Actions that route by different `lookup_by`
    // fields get separate hooks so each lands on its own entity instance.
    let mut instruction_hooks_map: BTreeMap<String, Vec<InstructionHook>> = BTreeMap::new();

    for registration in pda_registrations {
        let instr_type = path_to_string(&registration.instruction_path);
        let instr_base = instr_type.split("::").last().unwrap();
        let program_name = program_name_for_type(&instr_type, idls);
        let instr_type_state = scoped_instruction_event_type(program_name, instr_base);

        let action = HookAction::RegisterPdaMapping {
            pda_field: FieldPath::new(&["accounts", &registration.pda_field.ident.to_string()]),
            seed_field: FieldPath::new(&[
                "accounts",
                &registration.primary_key_field.ident.to_string(),
            ]),
            lookup_name: registration.lookup_name.clone(),
        };

        hook_for_lookup(&mut instruction_hooks_map, &instr_type_state, None)
            .actions
            .push(action);
    }

    let mut sorted_derive_from: Vec<_> = derive_from_mappings.iter().collect();
    sorted_derive_from.sort_by_key(|(k, _)| *k);
    for (instruction_type, derive_attrs) in sorted_derive_from {
        let instr_base = instruction_type.split("::").last().unwrap();
        let program_name = program_name_for_type(instruction_type, idls);
        // CPI events (from `::events::` submodule) use "CpiEvent" suffix; instructions use "IxState"
        let is_cpi_event = instruction_type.contains("::events::");
        let instr_type_state =
            scoped_instruction_or_cpi_event_type(program_name, instr_base, is_cpi_event);

        for derive_attr in derive_attrs {
            let source = if derive_attr.field.ident.to_string().starts_with("__") {
                match crate::ast::writer::context_field_name(&derive_attr.field.ident.to_string()) {
                    Some(field) => MappingSource::FromContext {
                        field: field.to_string(),
                    },
                    None => continue,
                }
            } else {
                let path_prefix = match &derive_attr.field.explicit_location {
                    Some(parse::FieldLocation::Account) => "accounts",
                    Some(parse::FieldLocation::InstructionArg) | None => "data",
                };

                MappingSource::FromSource {
                    path: FieldPath::new(&[path_prefix, &derive_attr.field.ident.to_string()]),
                    default: None,
                    transform: derive_attr
                        .transform
                        .as_ref()
                        .and_then(|t| parse_transformation(&t.to_string())),
                }
            };

            let condition = derive_attr.condition.clone();

            let action = HookAction::SetField {
                target_field: derive_attr.target_field_name.clone(),
                source,
                condition,
            };

            let lookup_by = derive_attr.lookup_by.as_ref().map(|field_spec| {
                FieldPath::new(&[
                    lookup_by_prefix(field_spec.explicit_location.as_ref(), is_cpi_event),
                    &field_spec.ident.to_string(),
                ])
            });

            hook_for_lookup(&mut instruction_hooks_map, &instr_type_state, lookup_by)
                .actions
                .push(action);
        }
    }

    let mut sorted_sources: Vec<_> = sources_by_type.iter().collect();
    sorted_sources.sort_by_key(|(k, _)| *k);
    for (_source_type, mappings) in &sorted_sources {
        for mapping in *mappings {
            let Some(stop_path) = &mapping.stop else {
                continue;
            };

            let stop_type = path_to_string(stop_path);
            let stop_base = stop_type.split("::").last().unwrap_or(&stop_type);
            let stop_program = program_name_for_type(&stop_type, idls);
            let stop_is_cpi_event = stop_type.contains("::events::");
            let stop_type_state =
                scoped_instruction_or_cpi_event_type(stop_program, stop_base, stop_is_cpi_event);

            let stop_field = format!("__stop:{}", mapping.target_field_name);

            let lookup_by = mapping
                .stop_lookup_by
                .as_ref()
                .or(mapping.lookup_by.as_ref())
                .map(|field_spec| {
                    FieldPath::new(&[
                        lookup_by_prefix(field_spec.explicit_location.as_ref(), stop_is_cpi_event),
                        &field_spec.ident.to_string(),
                    ])
                })
                .or_else(|| {
                    mapping
                        .register_from
                        .iter()
                        .find(|reg| path_to_string(&reg.instruction_path) == stop_type)
                        .map(|reg| {
                            let prefix = match &reg.primary_key_field.explicit_location {
                                Some(parse::FieldLocation::InstructionArg) => "data",
                                _ => "accounts",
                            };
                            FieldPath::new(&[prefix, &reg.primary_key_field.ident.to_string()])
                        })
                });

            let action = HookAction::SetField {
                target_field: stop_field,
                source: MappingSource::Constant(serde_json::Value::Bool(true)),
                condition: None,
            };

            hook_for_lookup(&mut instruction_hooks_map, &stop_type_state, lookup_by)
                .actions
                .push(action);
        }
    }

    let mut sorted_aggregate_conditions: Vec<_> = aggregate_conditions.iter().collect();
    sorted_aggregate_conditions.sort_by_key(|(k, _)| *k);
    for (field_path, condition_expr) in sorted_aggregate_conditions {
        for (source_type, mappings) in &sorted_sources {
            for mapping in *mappings {
                if &mapping.target_field_name == field_path
                    && mapping.is_instruction
                    && matches!(
                        mapping.strategy.as_str(),
                        "Sum" | "Count" | "Min" | "Max" | "UniqueCount"
                    )
                {
                    let instr_base = source_type.split("::").last().unwrap();
                    let program_name = program_name_for_type(source_type, idls);
                    let is_cpi_event = source_type.contains("::events::");
                    let instr_type_state = scoped_instruction_or_cpi_event_type(
                        program_name,
                        instr_base,
                        is_cpi_event,
                    );

                    let condition = condition_expr.clone();

                    if mapping.strategy == "Count" {
                        let action = HookAction::IncrementField {
                            target_field: field_path.clone(),
                            increment_by: 1,
                            condition: Some(condition),
                        };

                        hook_for_lookup(&mut instruction_hooks_map, &instr_type_state, None)
                            .actions
                            .push(action);
                    }
                }
            }
        }
    }

    instruction_hooks_map.into_values().flatten().collect()
}

/// The hook that collects actions for `instruction_type` routed by
/// `lookup_by`. Actions without `lookup_by` join the first hook; a hook
/// without `lookup_by` adopts the first one offered; a different `lookup_by`
/// starts a separate hook instead of being routed by another action's key.
fn hook_for_lookup<'a>(
    hooks_by_instruction: &'a mut BTreeMap<String, Vec<InstructionHook>>,
    instruction_type: &str,
    lookup_by: Option<FieldPath>,
) -> &'a mut InstructionHook {
    let hooks = hooks_by_instruction
        .entry(instruction_type.to_string())
        .or_default();
    let index = match &lookup_by {
        None => (!hooks.is_empty()).then_some(0),
        Some(field) => hooks
            .iter()
            .position(|hook| hook.lookup_by.as_ref() == Some(field))
            .or_else(|| hooks.iter().position(|hook| hook.lookup_by.is_none())),
    };
    let index = match index {
        Some(index) => {
            if hooks[index].lookup_by.is_none() {
                hooks[index].lookup_by = lookup_by;
            }
            index
        }
        None => {
            hooks.push(InstructionHook {
                instruction_type: instruction_type.to_string(),
                actions: Vec::new(),
                lookup_by,
            });
            hooks.len() - 1
        }
    };
    &mut hooks[index]
}

#[cfg(test)]
mod entity_ownership_tests {
    use super::*;

    fn idl(name: &str, address: &str) -> idl_parser::IdlSpec {
        serde_json::from_value(serde_json::json!({
            "name": name,
            "address": address,
            "instructions": [],
        }))
        .expect("minimal idl deserializes")
    }

    /// `path` is the resolved `from_instruction`. The legacy
    /// `event(instruction = "...")` form has none, which is why it never
    /// reaches `sources_by_type`.
    fn event_attribute(
        instruction: &str,
        path: Option<&str>,
        lookup_by: bool,
    ) -> parse::EventAttribute {
        parse::EventAttribute {
            attr_span: proc_macro2::Span::call_site(),
            instruction_span: None,
            from_instruction: path.map(|path| syn::parse_str::<syn::Path>(path).unwrap()),
            inferred_instruction: None,
            capture_fields: Vec::new(),
            field_transforms: std::collections::HashMap::new(),
            instruction: instruction.to_string(),
            capture_fields_legacy: Vec::new(),
            field_transforms_legacy: std::collections::HashMap::new(),
            strategy: "LastWrite".to_string(),
            target_field_name: "field".to_string(),
            join_on: None,
            lookup_by: lookup_by.then(|| parse::FieldSpec {
                ident: syn::parse_str::<syn::Ident>("anchor").unwrap(),
                explicit_location: None,
            }),
        }
    }

    fn event_mapping(
        instruction: &str,
        path: Option<&str>,
        lookup_by: bool,
    ) -> (String, parse::EventAttribute, syn::Type) {
        (
            "field".to_string(),
            event_attribute(instruction, path, lookup_by),
            syn::parse_str::<syn::Type>("u64").unwrap(),
        )
    }

    #[test]
    fn a_program_qualified_event_key_resolves_to_its_program() {
        let pump = idl("pump", "PumpAddr");
        let entropy = idl("entropy", "EntropyAddr");
        let idls = [("pump_sdk".to_string(), &pump), ("entropy_sdk".to_string(), &entropy)];

        // The sdk-prefixed spelling the macro uses for map sources.
        assert_eq!(
            source_program_name("entropy_sdk::accounts::Var", &idls),
            Some("entropy")
        );
        // The program-qualified spelling the macro uses for event keys, which
        // an sdk-prefix match alone can never resolve.
        assert_eq!(source_program_name("entropy::Reveal", &idls), Some("entropy"));
        assert_eq!(source_program_name("token::Transfer", &idls), None);
    }

    #[test]
    fn lookup_based_events_decide_ownership_when_they_outnumber_map_sources() {
        let pump = idl("pump", "PumpAddr");
        let entropy = idl("entropy", "EntropyAddr");
        let idls = [("pump_sdk".to_string(), &pump), ("entropy_sdk".to_string(), &entropy)];

        let mut sources = BTreeMap::new();
        sources.insert("pump_sdk::accounts::BondingCurve".to_string(), Vec::new());

        // `lookup_by` events never reach `sources_by_type`, so ownership has to
        // count them here or the entity is attributed to the wrong program.
        let mut events = BTreeMap::new();
        events.insert(
            "entropy::Reveal".to_string(),
            vec![event_mapping(
                "entropy::Reveal",
                Some("entropy_sdk::instructions::Reveal"),
                true,
            )],
        );
        events.insert(
            "entropy::Sample".to_string(),
            vec![event_mapping(
                "entropy::Sample",
                Some("entropy_sdk::instructions::Sample"),
                true,
            )],
        );

        let resolved = resolve_entity_idl(&sources, &events, &idls);
        assert_eq!(
            resolved.and_then(|idl| idl.address.as_deref()),
            Some("EntropyAddr")
        );
    }

    /// The legacy `event(instruction = "entropy::Reveal")` form keeps no
    /// instruction path, so `entity.rs` has nothing to merge into
    /// `sources_by_type`. Skipping it here left the source with no vote at all.
    #[test]
    fn legacy_string_events_still_vote_for_their_program() {
        let pump = idl("pump", "PumpAddr");
        let entropy = idl("entropy", "EntropyAddr");
        let idls = [("pump_sdk".to_string(), &pump), ("entropy_sdk".to_string(), &entropy)];

        let mut sources = BTreeMap::new();
        sources.insert("pump_sdk::accounts::BondingCurve".to_string(), Vec::new());

        let mut events = BTreeMap::new();
        events.insert(
            "entropy::Reveal".to_string(),
            vec![event_mapping("entropy::Reveal", None, false)],
        );
        events.insert(
            "entropy::Sample".to_string(),
            vec![event_mapping("entropy::Sample", None, false)],
        );

        let resolved = resolve_entity_idl(&sources, &events, &idls);
        assert_eq!(
            resolved.and_then(|idl| idl.address.as_deref()),
            Some("EntropyAddr")
        );
    }

    #[test]
    fn an_event_already_merged_into_its_source_type_votes_once() {
        let pump = idl("pump", "PumpAddr");
        let entropy = idl("entropy", "EntropyAddr");
        let idls = [("pump_sdk".to_string(), &pump), ("entropy_sdk".to_string(), &entropy)];

        let mut sources = BTreeMap::new();
        sources.insert("pump_sdk::accounts::BondingCurve".to_string(), Vec::new());
        sources.insert("pump_sdk::accounts::Global".to_string(), Vec::new());
        // A non-`lookup_by` event is merged under its instruction path.
        sources.insert(
            "entropy_sdk::instructions::Reveal".to_string(),
            Vec::new(),
        );

        // Counting the same source again, once per capturing field, would let
        // entropy outweigh pump's two distinct sources.
        let mut events = BTreeMap::new();
        events.insert(
            "entropy::Reveal".to_string(),
            vec![
                event_mapping(
                    "entropy::Reveal",
                    Some("entropy_sdk::instructions::Reveal"),
                    false,
                ),
                event_mapping(
                    "entropy::Reveal",
                    Some("entropy_sdk::instructions::Reveal"),
                    false,
                ),
            ],
        );

        let resolved = resolve_entity_idl(&sources, &events, &idls);
        assert_eq!(
            resolved.and_then(|idl| idl.address.as_deref()),
            Some("PumpAddr")
        );
    }

    #[test]
    fn several_fields_capturing_one_instruction_are_one_source() {
        let pump = idl("pump", "PumpAddr");
        let entropy = idl("entropy", "EntropyAddr");
        let idls = [("pump_sdk".to_string(), &pump), ("entropy_sdk".to_string(), &entropy)];

        let mut sources = BTreeMap::new();
        sources.insert("pump_sdk::accounts::BondingCurve".to_string(), Vec::new());
        sources.insert("pump_sdk::accounts::Global".to_string(), Vec::new());

        // Three fields, one entropy instruction. Counting per field would let it
        // outvote pump's two distinct sources, so adding a captured field would
        // silently change the entity's program.
        let mut events = BTreeMap::new();
        events.insert(
            "entropy::Reveal".to_string(),
            vec![
                event_mapping("entropy::Reveal", Some("entropy_sdk::instructions::Reveal"), true),
                event_mapping("entropy::Reveal", Some("entropy_sdk::instructions::Reveal"), true),
                event_mapping("entropy::Reveal", Some("entropy_sdk::instructions::Reveal"), true),
            ],
        );

        let resolved = resolve_entity_idl(&sources, &events, &idls);
        assert_eq!(
            resolved.and_then(|idl| idl.address.as_deref()),
            Some("PumpAddr")
        );
    }
}
