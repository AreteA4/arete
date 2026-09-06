//! Exact public-artifact authoring and AST projection helpers.

#![allow(dead_code)]

use std::collections::BTreeMap;

use super::types::*;
use crate::event_type_helpers::{
    scoped_instruction_event_type, scoped_instruction_or_cpi_event_type,
};
use crate::parse;
use crate::parse::idl as idl_parser;

/// Author and atomically write the authoritative explicit V2 artifacts from
/// semantic codegen data. The composite stack AST is deliberately not an input
/// to this path.
pub fn write_public_artifacts(
    stack_name: &str,
    program_specs: &[arete_hash::ProgramSpecV1],
    entity_specs: &[super::types::SerializableStreamSpec],
    pda_overrides: &BTreeMap<String, BTreeMap<String, PdaDefinition>>,
    instruction_overrides: &[InstructionDef],
) -> Result<(), arete_artifacts::ArtifactError> {
    let entities = entity_specs
        .iter()
        .map(transcode_artifact_projection)
        .collect::<Result<Vec<arete_artifacts::PortableEntity>, _>>()?;
    let mut input =
        arete_artifacts::StackAuthoringV2::new(stack_name, program_specs.to_vec(), entities);
    input.pda_overrides = transcode_artifact_projection(pda_overrides)?;
    input.instruction_overrides = transcode_artifact_projection(instruction_overrides)?;
    let artifacts = arete_artifacts::author_stack_v2(input)?;
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").map_err(|error| {
        arete_artifacts::ArtifactError::InvalidArtifact(format!(
            "CARGO_MANIFEST_DIR is unavailable: {error}"
        ))
    })?;
    let ast_dir = std::path::Path::new(&manifest_dir).join(".arete");
    arete_artifacts::write_authored_stack_v2(&ast_dir, stack_name, &artifacts)?;
    Ok(())
}

fn transcode_artifact_projection<T, U>(value: &T) -> Result<U, arete_artifacts::ArtifactError>
where
    T: serde::Serialize + ?Sized,
    U: serde::de::DeserializeOwned,
{
    let value = serde_json::to_value(value)
        .map_err(|error| arete_artifacts::ArtifactError::InvalidJson(error.to_string()))?;
    serde_json::from_value(value)
        .map_err(|error| arete_artifacts::ArtifactError::InvalidJson(error.to_string()))
}

/// Helper function to parse transformation string to enum
pub fn parse_transformation(transform_str: &str) -> Option<Transformation> {
    match transform_str {
        "HexEncode" => Some(Transformation::HexEncode),
        "HexDecode" => Some(Transformation::HexDecode),
        "Base58Encode" => Some(Transformation::Base58Encode),
        "Base58Decode" => Some(Transformation::Base58Decode),
        "ToString" => Some(Transformation::ToString),
        "ToNumber" => Some(Transformation::ToNumber),
        _ => None,
    }
}

/// Helper function to parse population strategy string to enum
pub fn parse_population_strategy(strategy_str: &str) -> PopulationStrategy {
    match strategy_str {
        "SetOnce" => PopulationStrategy::SetOnce,
        "LastWrite" => PopulationStrategy::LastWrite,
        "Append" => PopulationStrategy::Append,
        "Merge" => PopulationStrategy::Merge,
        "Max" => PopulationStrategy::Max,
        "Sum" => PopulationStrategy::Sum,
        "Count" => PopulationStrategy::Count,
        "Min" => PopulationStrategy::Min,
        "UniqueCount" => PopulationStrategy::UniqueCount,
        _ => PopulationStrategy::LastWrite, // Default fallback
    }
}

/// Resolve reserved update-context fields used by the Rust DSL.
///
/// These values live in the runtime's `__update_context` envelope rather than
/// in an account, instruction, or CPI-event payload.
pub fn context_field_name(source_field_name: &str) -> Option<&'static str> {
    match source_field_name {
        "__signature" => Some("signature"),
        "__slot" => Some("slot"),
        "__timestamp" => Some("timestamp"),
        _ => None,
    }
}

/// Convert idl_parser::IdlSpec to ast::IdlSnapshot for embedding in AST
pub fn convert_idl_to_snapshot(idl: &idl_parser::IdlSpec) -> IdlSnapshot {
    // Build types list: start with explicit types from idl.types
    let mut types: Vec<IdlTypeDefSnapshot> = idl
        .types
        .iter()
        .map(|typedef| IdlTypeDefSnapshot {
            name: typedef.name.clone(),
            docs: typedef.docs.clone(),
            serialization: typedef.serialization.as_ref().map(|s| match s {
                idl_parser::IdlSerialization::Borsh => IdlSerializationSnapshot::Borsh,
                idl_parser::IdlSerialization::Bytemuck => IdlSerializationSnapshot::Bytemuck,
                idl_parser::IdlSerialization::BytemuckUnsafe => {
                    IdlSerializationSnapshot::BytemuckUnsafe
                }
            }),
            type_def: match &typedef.type_def {
                idl_parser::IdlTypeDefKind::Struct { kind, fields } => {
                    IdlTypeDefKindSnapshot::Struct {
                        kind: kind.clone(),
                        fields: fields
                            .iter()
                            .map(|f| IdlFieldSnapshot {
                                name: f.name.clone(),
                                type_: convert_idl_type(&f.type_),
                                amount_hint: f.amount_hint.clone(),
                            })
                            .collect(),
                    }
                }
                idl_parser::IdlTypeDefKind::TupleStruct { kind, fields } => {
                    IdlTypeDefKindSnapshot::TupleStruct {
                        kind: kind.clone(),
                        fields: fields.iter().map(convert_idl_type).collect(),
                    }
                }
                idl_parser::IdlTypeDefKind::Enum { kind, variants } => {
                    IdlTypeDefKindSnapshot::Enum {
                        kind: kind.clone(),
                        variants: variants.iter().map(convert_enum_variant).collect(),
                    }
                }
            },
        })
        .collect();

    // Also include account type definitions (embedded in accounts with steel format)
    // These are needed for the ast-compiler-macros to generate proper account struct fields
    for account in &idl.accounts {
        // Skip if type already exists in types (avoid duplicates)
        if types.iter().any(|t| t.name == account.name) {
            continue;
        }

        // If account has an embedded type definition, add it to types
        if let Some(type_def) = &account.type_def {
            match type_def {
                idl_parser::IdlTypeDefKind::Struct { kind, fields } => {
                    types.push(IdlTypeDefSnapshot {
                        name: account.name.clone(),
                        docs: account.docs.clone(),
                        serialization: None,
                        type_def: IdlTypeDefKindSnapshot::Struct {
                            kind: kind.clone(),
                            fields: fields
                                .iter()
                                .map(|f| IdlFieldSnapshot {
                                    name: f.name.clone(),
                                    type_: convert_idl_type(&f.type_),
                                    amount_hint: f.amount_hint.clone(),
                                })
                                .collect(),
                        },
                    });
                }
                idl_parser::IdlTypeDefKind::TupleStruct { kind, fields } => {
                    types.push(IdlTypeDefSnapshot {
                        name: account.name.clone(),
                        docs: account.docs.clone(),
                        serialization: None,
                        type_def: IdlTypeDefKindSnapshot::TupleStruct {
                            kind: kind.clone(),
                            fields: fields.iter().map(convert_idl_type).collect(),
                        },
                    });
                }
                idl_parser::IdlTypeDefKind::Enum { kind, variants } => {
                    types.push(IdlTypeDefSnapshot {
                        name: account.name.clone(),
                        docs: account.docs.clone(),
                        serialization: None,
                        type_def: IdlTypeDefKindSnapshot::Enum {
                            kind: kind.clone(),
                            variants: variants.iter().map(convert_enum_variant).collect(),
                        },
                    });
                }
            }
        }
    }

    let discriminant_size = idl.instruction_discriminator_size();
    let program_id = idl
        .address
        .clone()
        .or_else(|| idl.metadata.as_ref().and_then(|m| m.address.clone()));

    IdlSnapshot {
        name: idl.get_name().to_string(),
        program_id,
        version: idl.get_version().to_string(),
        accounts: idl
            .accounts
            .iter()
            .map(|acc| {
                let serialization = idl
                    .types
                    .iter()
                    .find(|t| t.name == acc.name)
                    .and_then(|t| t.serialization.as_ref())
                    .map(|s| match s {
                        idl_parser::IdlSerialization::Borsh => IdlSerializationSnapshot::Borsh,
                        idl_parser::IdlSerialization::Bytemuck => {
                            IdlSerializationSnapshot::Bytemuck
                        }
                        idl_parser::IdlSerialization::BytemuckUnsafe => {
                            IdlSerializationSnapshot::BytemuckUnsafe
                        }
                    });
                IdlAccountSnapshot {
                    name: acc.name.clone(),
                    discriminator: acc.get_discriminator(),
                    docs: acc.docs.clone(),
                    serialization,
                    // Extract fields from inline type definition if present
                    fields: acc.type_def.as_ref().map_or_else(
                        Vec::new,
                        |type_def| match type_def {
                            idl_parser::IdlTypeDefKind::Struct { fields, .. } => fields
                                .iter()
                                .map(|f| IdlFieldSnapshot {
                                    name: f.name.clone(),
                                    type_: convert_idl_type(&f.type_),
                                    amount_hint: f.amount_hint.clone(),
                                })
                                .collect(),
                            _ => Vec::new(),
                        },
                    ),
                    type_def: None, // Fields are extracted above, no need to keep type_def
                }
            })
            .collect(),
        instructions: idl
            .instructions
            .iter()
            .map(|instr| IdlInstructionSnapshot {
                name: instr.name.clone(),
                discriminator: instr.get_discriminator(),
                discriminant: None,
                docs: instr.docs.clone(),
                accounts: instr
                    .flattened_accounts()
                    .iter()
                    .map(|acc| IdlInstructionAccountSnapshot {
                        name: acc.name.clone(),
                        writable: acc.is_mut,
                        signer: acc.is_signer,
                        optional: acc.optional,
                        address: acc.address.clone(),
                        docs: acc.docs.clone(),
                    })
                    .collect(),
                args: instr
                    .args
                    .iter()
                    .map(|arg| IdlFieldSnapshot {
                        name: arg.name.clone(),
                        type_: convert_idl_type(&arg.type_),
                        amount_hint: arg.amount_hint.clone(),
                    })
                    .collect(),
            })
            .collect(),
        types,
        events: idl
            .events
            .iter()
            .map(|event| IdlEventSnapshot {
                name: event.name.clone(),
                discriminator: event.get_discriminator(),
                docs: event.docs.clone(),
                fields: event
                    .fields
                    .iter()
                    .map(|field| IdlFieldSnapshot {
                        name: field.name.clone(),
                        type_: convert_idl_type(&field.type_),
                        amount_hint: field.amount_hint.clone(),
                    })
                    .collect(),
            })
            .collect(),
        errors: idl
            .errors
            .iter()
            .map(|err| IdlErrorSnapshot {
                code: err.code,
                name: err.name.clone(),
                msg: err.msg.clone(),
            })
            .collect(),
        discriminant_size,
    }
}

/// Convert idl_parser::IdlType to ast::IdlTypeSnapshot
pub fn convert_idl_type(idl_type: &idl_parser::IdlType) -> IdlTypeSnapshot {
    match idl_type {
        idl_parser::IdlType::Simple(s) => IdlTypeSnapshot::Simple(s.clone()),
        idl_parser::IdlType::Array(arr) => IdlTypeSnapshot::Array(IdlArrayTypeSnapshot {
            array: arr
                .array
                .iter()
                .map(|elem| match elem {
                    idl_parser::IdlTypeArrayElement::Nested(t) => {
                        IdlArrayElementSnapshot::Type(convert_idl_type(t))
                    }
                    idl_parser::IdlTypeArrayElement::Type(s) => {
                        IdlArrayElementSnapshot::TypeName(s.clone())
                    }
                    idl_parser::IdlTypeArrayElement::Size(n) => IdlArrayElementSnapshot::Size(*n),
                })
                .collect(),
        }),
        idl_parser::IdlType::Option(opt) => IdlTypeSnapshot::Option(IdlOptionTypeSnapshot {
            option: Box::new(convert_idl_type(&opt.option)),
        }),
        idl_parser::IdlType::Vec(vec) => IdlTypeSnapshot::Vec(IdlVecTypeSnapshot {
            vec: Box::new(convert_idl_type(&vec.vec)),
            length_prefix: vec.length_prefix,
        }),
        idl_parser::IdlType::Defined(def) => IdlTypeSnapshot::Defined(IdlDefinedTypeSnapshot {
            defined: match &def.defined {
                idl_parser::IdlTypeDefinedInner::Named { name } => {
                    IdlDefinedInnerSnapshot::Named { name: name.clone() }
                }
                idl_parser::IdlTypeDefinedInner::Simple(s) => {
                    IdlDefinedInnerSnapshot::Simple(s.clone())
                }
            },
        }),
        idl_parser::IdlType::HashMap(hm) => IdlTypeSnapshot::HashMap(IdlHashMapTypeSnapshot {
            hash_map: (
                Box::new(convert_idl_type(&hm.hash_map.0)),
                Box::new(convert_idl_type(&hm.hash_map.1)),
            ),
        }),
        idl_parser::IdlType::Tuple(tuple) => IdlTypeSnapshot::Tuple(IdlTupleTypeSnapshot {
            tuple: tuple.tuple.iter().map(convert_idl_type).collect(),
        }),
    }
}

fn convert_enum_variant(variant: &idl_parser::IdlEnumVariant) -> IdlEnumVariantSnapshot {
    IdlEnumVariantSnapshot {
        name: variant.name.clone(),
        fields: variant
            .fields
            .iter()
            .map(convert_enum_variant_field)
            .collect(),
    }
}

fn convert_enum_variant_field(
    field: &idl_parser::IdlEnumVariantField,
) -> IdlEnumVariantFieldSnapshot {
    match field {
        idl_parser::IdlEnumVariantField::Named(named) => {
            IdlEnumVariantFieldSnapshot::Named(IdlFieldSnapshot {
                name: named.name.clone(),
                type_: convert_idl_type(&named.type_),
                amount_hint: named.amount_hint.clone(),
            })
        }
        idl_parser::IdlEnumVariantField::Tuple(tuple) => {
            IdlEnumVariantFieldSnapshot::Tuple(convert_idl_type(tuple))
        }
    }
}

/// Build handlers from source mappings.
pub fn build_handlers_from_sources(
    sources_by_type: &BTreeMap<String, Vec<parse::MapAttribute>>,
    _events_by_instruction: &BTreeMap<String, Vec<(String, parse::EventAttribute, syn::Type)>>,
    aggregate_conditions: &BTreeMap<String, ConditionExpr>,
    idl: Option<&idl_parser::IdlSpec>,
) -> Vec<SerializableHandlerSpec> {
    let program_name = idl.map(|idl| idl.get_name());
    let mut handlers = Vec::new();

    // Group sources by type and join key (same logic as handler generation)
    // Use BTreeMap for deterministic ordering
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

    // Process account sources with full detail (now in sorted order)
    for ((source_type, join_key), mappings) in &sources_by_type_and_join {
        let account_type = source_type.split("::").last().unwrap_or(source_type);
        let is_instruction = mappings.iter().any(|m| m.is_instruction);
        // CPI events are sourced from `::events::` submodule paths.
        // All event fields live under "data.*" (no "accounts.*" section).
        let is_cpi_event = source_type.contains("::events::");

        // Skip if this is an event-derived mapping
        if is_instruction
            && mappings
                .iter()
                .any(|m| m.target_field_name.starts_with("events."))
        {
            continue;
        }

        let mut serializable_mappings = Vec::new();

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
                    // CPI events: all fields are under "data"
                    if mapping.source_field_name.is_empty() {
                        FieldPath::new(&["data"])
                    } else {
                        FieldPath::new(&["data", &mapping.source_field_name])
                    }
                } else if is_instruction {
                    if mapping.source_field_name.is_empty() {
                        FieldPath::new(&["data"])
                    } else {
                        let prefix = idl
                            .and_then(|idl| {
                                idl.get_instruction_field_prefix(
                                    account_type,
                                    &mapping.source_field_name,
                                )
                            })
                            .unwrap_or("data");
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
                scoped_instruction_event_type(program_name, instr_base)
            });

            let stop = mapping.stop.as_ref().map(|stop_path| {
                let instr_type = path_to_string(stop_path);
                let instr_base = instr_type.split("::").last().unwrap_or(&instr_type);
                scoped_instruction_event_type(program_name, instr_base)
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
                if let Some(field) = context_field {
                    primary_field = Some(format!("__update_context.{field}"));
                } else if is_cpi_event {
                    // CPI event fields are always in "data"
                    primary_field = Some(format!("data.{}", mapping.source_field_name));
                } else if is_instruction {
                    let prefix = idl
                        .and_then(|idl| {
                            idl.get_instruction_field_prefix(
                                account_type,
                                &mapping.source_field_name,
                            )
                        })
                        .unwrap_or("data");
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
                let segments: Vec<&str> = lookup_field.split('.').collect();
                KeyResolutionStrategy::Lookup {
                    primary_field: FieldPath::new(&segments),
                }
            } else if let Some(ref join_field) = join_key {
                KeyResolutionStrategy::Lookup {
                    primary_field: FieldPath::new(&[join_field]),
                }
            } else {
                // No lookup_by specified - use embedded with empty path
                KeyResolutionStrategy::Embedded {
                    primary_field: FieldPath::new(&[]),
                }
            }
        } else if let Some(ref join_field) = join_key {
            KeyResolutionStrategy::Lookup {
                primary_field: FieldPath::new(&[join_field]),
            }
        } else {
            KeyResolutionStrategy::Embedded {
                primary_field: FieldPath::new(&[]),
            }
        };

        // CPI events (from `::events::`) use "CpiEvent" suffix; instructions use "IxState"
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
                        idl_parser::IdlSerialization::Bytemuck => {
                            IdlSerializationSnapshot::Bytemuck
                        }
                        idl_parser::IdlSerialization::BytemuckUnsafe => {
                            IdlSerializationSnapshot::BytemuckUnsafe
                        }
                    })
            })
        };
        let type_name = if is_instruction || is_cpi_event {
            scoped_instruction_or_cpi_event_type(program_name, account_type, is_cpi_event)
        } else if let Some(program_name) = program_name {
            format!("{}::{}{}", program_name, account_type, type_suffix)
        } else {
            format!("{}{}", account_type, type_suffix)
        };
        handlers.push(SerializableHandlerSpec {
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
        });
    }

    handlers
}

/// Build resolver hooks from #[resolve_key] attributes
pub fn build_resolver_hooks(
    resolver_hooks: &[parse::ResolveKeyAttribute],
    idl: Option<&idl_parser::IdlSpec>,
) -> Vec<ResolverHook> {
    let program_name = idl.map(|idl| idl.get_name());
    resolver_hooks
        .iter()
        .map(|hook| {
            let account_type = path_to_string(&hook.account_path);
            let account_base = account_type.split("::").last().unwrap();
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
                                idl.instructions
                                    .iter()
                                    .find(|instr| instr.name.eq_ignore_ascii_case(&instr_name))
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

/// Build instruction hooks from PDA registrations and derive_from mappings
pub fn build_instruction_hooks(
    pda_registrations: &[parse::RegisterPdaAttribute],
    derive_from_mappings: &BTreeMap<String, Vec<parse::DeriveFromAttribute>>,
    aggregate_conditions: &BTreeMap<String, ConditionExpr>,
    sources_by_type: &BTreeMap<String, Vec<parse::MapAttribute>>,
    program_name: Option<&str>,
) -> Vec<InstructionHook> {
    // Use BTreeMap for deterministic ordering in the final output
    let mut instruction_hooks_map: BTreeMap<String, InstructionHook> = BTreeMap::new();

    // Process PDA registrations
    for registration in pda_registrations {
        let instr_type = path_to_string(&registration.instruction_path);
        let instr_base = instr_type.split("::").last().unwrap();
        let instr_type_state = scoped_instruction_event_type(program_name, instr_base);

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

    // Process derive_from mappings (sorted for deterministic output)
    let mut sorted_derive_from: Vec<_> = derive_from_mappings.iter().collect();
    sorted_derive_from.sort_by_key(|(k, _)| *k);
    for (instruction_type, derive_attrs) in sorted_derive_from {
        let instr_base = instruction_type.split("::").last().unwrap();
        let instr_type_state = scoped_instruction_event_type(program_name, instr_base);

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

            let lookup_by = derive_attr
                .lookup_by
                .as_ref()
                .map(|field_spec| FieldPath::new(&["accounts", &field_spec.ident.to_string()]));

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

    // Process aggregate conditions (sorted for deterministic output)
    let mut sorted_aggregate_conditions: Vec<_> = aggregate_conditions.iter().collect();
    sorted_aggregate_conditions.sort_by_key(|(k, _)| *k);
    let mut sorted_sources: Vec<_> = sources_by_type.iter().collect();
    sorted_sources.sort_by_key(|(k, _)| *k);
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
                    let instr_type_state = scoped_instruction_event_type(program_name, instr_base);

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

/// Helper to convert syn::Path to string
fn path_to_string(path: &syn::Path) -> String {
    path.segments
        .iter()
        .map(|seg| seg.ident.to_string())
        .collect::<Vec<_>>()
        .join("::")
}
