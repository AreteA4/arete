//! AST building and file writing for hyperstack streams.
//!
//! This module handles:
//! 1. Building `SerializableStreamSpec` from parsed macro attributes
//! 2. Writing AST JSON files during macro expansion for cloud compilation
//!
//! The same AST is used for both inline code generation (via `codegen::generate_handlers_from_specs`)
//! and for the `#[ast_spec]` macro, ensuring identical output.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::ast::writer::{
    convert_idl_to_snapshot, parse_population_strategy, parse_transformation,
};
use crate::ast::{
    ComputedFieldSpec, ConditionExpr, EntitySection, FieldPath, FieldTypeInfo, HookAction,
    IdentitySpec, IdlSerializationSnapshot, InstructionHook, KeyResolutionStrategy,
    LookupIndexSpec, MappingSource, ResolveStrategy, ResolverCondition, ResolverExtractSpec,
    ResolverHook, ResolverSpec, ResolverStrategy, ResolverType, SerializableFieldMapping,
    SerializableHandlerSpec, SerializableStreamSpec, SourceSpec,
};
use crate::diagnostic::{idl_error_to_syn, internal_codegen_error};
use crate::event_type_helpers::{find_idl_for_type, program_name_for_type, IdlLookup};
use crate::parse;
use crate::parse::conditions as condition_parser;
use crate::parse::idl as idl_parser;
use crate::utils::path_to_string;
use hyperstack_idl::error::IdlSearchError;
use hyperstack_idl::search::{lookup_account, lookup_instruction_field, InstructionFieldKind};

use super::computed::{
    expr_contains_u64_from_bytes, extract_resolver_type_from_computed_expr,
    parse_computed_expression, qualify_field_refs,
};
use super::handlers::{find_field_in_instruction, get_join_on_field};

// ============================================================================
// AST Building
// ============================================================================

/// Build the complete AST from parsed macro attributes.
///
/// This is the single source of truth for building `SerializableStreamSpec`.
/// The returned AST is used for:
/// 1. Code generation via `codegen::generate_handlers_from_specs()`
/// 2. Writing to disk for `#[ast_spec]` cloud compilation
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
    sources_by_type: &HashMap<String, Vec<parse::MapAttribute>>,
    events_by_instruction: &HashMap<String, Vec<(String, parse::EventAttribute, syn::Type)>>,
    resolver_hooks: &[parse::ResolveKeyAttribute],
    pda_registrations: &[parse::RegisterPdaAttribute],
    derive_from_mappings: &HashMap<String, Vec<parse::DeriveFromAttribute>>,
    aggregate_conditions: &HashMap<String, ConditionExpr>,
    computed_fields: &[(String, proc_macro2::TokenStream, syn::Type)],
    resolve_specs: &[parse::ResolveSpec],
    section_specs: &[EntitySection],
    idls: IdlLookup,
    views: Vec<crate::ast::ViewDef>,
) -> syn::Result<SerializableStreamSpec> {
    let idl = idls.first().map(|(_, idl)| *idl);
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

            let field_info = FieldTypeInfo {
                field_name: computed_spec.target_path.clone(),
                rust_type_name: computed_spec.result_type.clone(),
                base_type: if is_array {
                    crate::ast::BaseType::Array
                } else {
                    crate::ast::BaseType::Any
                },
                is_optional,
                is_array,
                inner_type: Some(resolver_type.to_string()),
                source_path: None,
                resolved_type: None,
                emit: true,
            };
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
    sources_by_type: &HashMap<String, Vec<parse::MapAttribute>>,
    events_by_instruction: &HashMap<String, Vec<(String, parse::EventAttribute, syn::Type)>>,
    resolver_hooks: &[parse::ResolveKeyAttribute],
    pda_registrations: &[parse::RegisterPdaAttribute],
    derive_from_mappings: &HashMap<String, Vec<parse::DeriveFromAttribute>>,
    aggregate_conditions: &HashMap<String, ConditionExpr>,
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
    sources_by_type: &HashMap<String, Vec<parse::MapAttribute>>,
    events_by_instruction: &HashMap<String, Vec<(String, parse::EventAttribute, syn::Type)>>,
    primary_keys: &[String],
    lookup_indexes: &[(String, Option<String>)],
    aggregate_conditions: &HashMap<String, ConditionExpr>,
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
        if let Some(handler) = build_source_handler(
            source_type,
            join_key,
            mappings,
            aggregate_conditions,
            primary_keys,
            lookup_indexes,
            idls,
        )? {
            handlers.push(handler);
        }
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
        if let Some(handler) = build_event_handler(
            instruction,
            join_key,
            event_mappings,
            primary_keys,
            lookup_indexes,
            idls,
        )? {
            handlers.push(handler);
        }
    }

    Ok(handlers)
}

fn build_source_handler(
    source_type: &str,
    join_key: &Option<String>,
    mappings: &[parse::MapAttribute],
    aggregate_conditions: &HashMap<String, ConditionExpr>,
    primary_keys: &[String],
    lookup_indexes: &[(String, Option<String>)],
    idls: IdlLookup,
) -> syn::Result<Option<SerializableHandlerSpec>> {
    let account_type = source_type.split("::").last().unwrap_or(source_type);
    let idl = find_idl_for_type(source_type, idls);
    let program_name = program_name_for_type(source_type, idls);
    let is_instruction = mappings.iter().any(|m| m.is_instruction);
    // CPI events are sourced from `::events::` submodule paths (e.g. generated_sdk::events::Swap).
    // All event fields are stored under "data.*" (no "accounts.*" section).
    let is_cpi_event = source_type.contains("::events::");

    // Skip event-derived mappings
    if is_instruction
        && mappings
            .iter()
            .any(|m| m.target_field_name.starts_with("events."))
    {
        return Ok(None);
    }

    if !is_instruction && !is_cpi_event {
        if let Some(idl) = idl {
            lookup_account(idl, account_type)
                .map_err(|error| idl_error_to_syn(mappings[0].source_type_span, error))?;
        }
    }

    let mut serializable_mappings = Vec::new();
    let mut has_primary_key = false;
    let mut primary_field = None;

    for mapping in mappings {
        // Skip conditional aggregates
        if aggregate_conditions.contains_key(&mapping.target_field_name) {
            continue;
        }

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
        } else {
            let field_path = if is_cpi_event {
                // CPI events: all fields (including identifiers like lb_pair, from) are under "data"
                if mapping.source_field_name.is_empty() {
                    FieldPath::new(&["data"])
                } else {
                    FieldPath::new(&["data", &mapping.source_field_name])
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
            if let Some(program_name) = program_name {
                format!("{}::{}IxState", program_name, instr_base)
            } else {
                format!("{}IxState", instr_base)
            }
        });

        let stop = mapping.stop.as_ref().map(|stop_path| {
            let instr_type = path_to_string(stop_path);
            let instr_base = instr_type.split("::").last().unwrap_or(&instr_type);
            let program_name = program_name_for_type(&instr_type, idls);
            if let Some(program_name) = program_name {
                format!("{}::{}IxState", program_name, instr_base)
            } else {
                format!("{}IxState", instr_base)
            }
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

        if mapping.is_primary_key {
            has_primary_key = true;
            if is_cpi_event {
                // CPI event fields are always in "data"
                primary_field = Some(format!("data.{}", mapping.source_field_name));
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
        }
    }

    let is_aggregation = mappings.iter().any(|m| {
        matches!(
            m.strategy.as_str(),
            "Sum" | "Count" | "Min" | "Max" | "UniqueCount"
        )
    });

    // Try to find lookup_by from the first mapping that has it
    let lookup_by_field = mappings
        .iter()
        .find_map(|m| m.lookup_by.as_ref())
        .map(|fs| {
            // FieldSpec has explicit_location which tells us if it's accounts:: or data::
            // For CPI events, all fields (including identifiers) are under "data".
            let prefix = match &fs.explicit_location {
                Some(parse::FieldLocation::Account) => "accounts",
                Some(parse::FieldLocation::InstructionArg) => "data",
                None => {
                    if is_cpi_event {
                        "data" // CPI event fields are always in "data"
                    } else {
                        "accounts" // Default to accounts for instruction compatibility
                    }
                }
            };
            format!("{}.{}", prefix, fs.ident)
        });

    let key_resolution = if has_primary_key {
        let primary_field_str = primary_field.as_deref().unwrap_or("");
        let segments: Vec<&str> = primary_field_str.split('.').collect();
        KeyResolutionStrategy::Embedded {
            primary_field: FieldPath::new(&segments),
        }
    } else if is_aggregation && is_instruction {
        // Use lookup_by if available, otherwise fall back to join_key or a sensible default
        if let Some(ref lookup_field) = lookup_by_field {
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
        } else if let Some(ref join_field) = join_key {
            KeyResolutionStrategy::Lookup {
                primary_field: FieldPath::new(&[join_field]),
            }
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
    let serialization = if is_instruction {
        None
    } else {
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
    };
    let type_name = if let Some(program_name) = program_name {
        format!("{}::{}{}", program_name, account_type, type_suffix)
    } else {
        format!("{}{}", account_type, type_suffix)
    };

    Ok(Some(SerializableHandlerSpec {
        source: SourceSpec::Source {
            program_id: None,
            discriminator: None,
            type_name,
            serialization,
            is_account: !is_instruction && !is_cpi_event,
        },
        key_resolution,
        mappings: serializable_mappings,
        conditions: Vec::new(),
        emit: true,
    }))
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
        None => (
            idls.first().map(|(_, idl)| *idl),
            idls.first().map(|(_, idl)| idl.get_name()),
        ),
    };
    let parts: Vec<&str> = instruction.split("::").collect();
    if parts.len() != 2 {
        return Ok(None);
    }

    let program_id = parts[0];
    let instruction_type = parts[1];
    let instruction_type_pascal = idl_parser::to_pascal_case(instruction_type);

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

    let type_name = if let Some(program_name) = program_name {
        format!("{}::{}IxState", program_name, instruction_type_pascal)
    } else {
        format!("{}IxState", instruction_type_pascal)
    };

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
    sources_by_type: &HashMap<String, Vec<parse::MapAttribute>>,
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
    derive_from_mappings: &HashMap<String, Vec<parse::DeriveFromAttribute>>,
    aggregate_conditions: &HashMap<String, ConditionExpr>,
    sources_by_type: &HashMap<String, Vec<parse::MapAttribute>>,
    idls: IdlLookup,
) -> Vec<InstructionHook> {
    let mut instruction_hooks_map: BTreeMap<String, InstructionHook> = BTreeMap::new();

    for registration in pda_registrations {
        let instr_type = path_to_string(&registration.instruction_path);
        let instr_base = instr_type.split("::").last().unwrap();
        let program_name = program_name_for_type(&instr_type, idls);
        let instr_type_state = if let Some(program_name) = program_name {
            format!("{}::{}IxState", program_name, instr_base)
        } else {
            format!("{}IxState", instr_base)
        };

        let action = HookAction::RegisterPdaMapping {
            pda_field: FieldPath::new(&["accounts", &registration.pda_field.ident.to_string()]),
            seed_field: FieldPath::new(&[
                "accounts",
                &registration.primary_key_field.ident.to_string(),
            ]),
            lookup_name: registration.lookup_name.clone(),
        };

        instruction_hooks_map
            .entry(instr_type_state.clone())
            .or_insert_with(|| InstructionHook {
                instruction_type: instr_type_state,
                actions: Vec::new(),
                lookup_by: None,
            })
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
        let type_suffix = if is_cpi_event { "CpiEvent" } else { "IxState" };
        let instr_type_state = if let Some(program_name) = program_name {
            format!("{}::{}{}", program_name, instr_base, type_suffix)
        } else {
            format!("{}{}", instr_base, type_suffix)
        };

        for derive_attr in derive_attrs {
            let source = if derive_attr.field.ident.to_string().starts_with("__") {
                match derive_attr.field.ident.to_string().as_str() {
                    "__timestamp" => MappingSource::FromContext {
                        field: "timestamp".to_string(),
                    },
                    "__slot" => MappingSource::FromContext {
                        field: "slot".to_string(),
                    },
                    "__signature" => MappingSource::FromContext {
                        field: "signature".to_string(),
                    },
                    _ => continue,
                }
            } else {
                let path_prefix = if is_cpi_event {
                    // CPI event fields are always under "data" (no "accounts" section)
                    "data"
                } else {
                    match &derive_attr.field.explicit_location {
                        Some(parse::FieldLocation::Account) => "accounts",
                        Some(parse::FieldLocation::InstructionArg) | None => "data",
                    }
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

            // CPI events: lookup_by fields are under "data"; instructions: under "accounts"
            let lookup_by_prefix = if is_cpi_event { "data" } else { "accounts" };
            let lookup_by = derive_attr.lookup_by.as_ref().map(|field_spec| {
                FieldPath::new(&[lookup_by_prefix, &field_spec.ident.to_string()])
            });

            let hook = instruction_hooks_map
                .entry(instr_type_state.clone())
                .or_insert_with(|| InstructionHook {
                    instruction_type: instr_type_state.clone(),
                    actions: Vec::new(),
                    lookup_by: lookup_by.clone(),
                });

            hook.actions.push(action);

            if hook.lookup_by.is_none() {
                hook.lookup_by = lookup_by;
            }
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
            let stop_type_state = if let Some(program_name) = stop_program {
                format!("{}::{}IxState", program_name, stop_base)
            } else {
                format!("{}IxState", stop_base)
            };

            let stop_field = format!("__stop:{}", mapping.target_field_name);

            let lookup_by = mapping
                .stop_lookup_by
                .as_ref()
                .map(|field_spec| {
                    let prefix = match &field_spec.explicit_location {
                        Some(parse::FieldLocation::InstructionArg) => "data",
                        _ => "accounts",
                    };
                    FieldPath::new(&[prefix, &field_spec.ident.to_string()])
                })
                .or_else(|| {
                    mapping.lookup_by.as_ref().map(|field_spec| {
                        FieldPath::new(&["accounts", &field_spec.ident.to_string()])
                    })
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

            let hook = instruction_hooks_map
                .entry(stop_type_state.clone())
                .or_insert_with(|| InstructionHook {
                    instruction_type: stop_type_state.clone(),
                    actions: Vec::new(),
                    lookup_by: lookup_by.clone(),
                });

            hook.actions.push(action);

            if hook.lookup_by.is_none() {
                hook.lookup_by = lookup_by;
            }
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
                    let instr_type_state = if let Some(program_name) = program_name {
                        format!("{}::{}IxState", program_name, instr_base)
                    } else {
                        format!("{}IxState", instr_base)
                    };

                    let condition = condition_expr.clone();

                    if mapping.strategy == "Count" {
                        let action = HookAction::IncrementField {
                            target_field: field_path.clone(),
                            increment_by: 1,
                            condition: Some(condition),
                        };

                        instruction_hooks_map
                            .entry(instr_type_state.clone())
                            .or_insert_with(|| InstructionHook {
                                instruction_type: instr_type_state,
                                actions: Vec::new(),
                                lookup_by: None,
                            })
                            .actions
                            .push(action);
                    }
                }
            }
        }
    }

    instruction_hooks_map.into_values().collect()
}
