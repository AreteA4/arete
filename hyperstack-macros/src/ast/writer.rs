//! AST JSON file serialization and writing.
//!
//! This module provides functions to serialize and write AST specifications
//! to JSON files during macro expansion.
//!
//! Note: Some functions in this module are currently unused but are kept for
//! future use when AST file generation is needed.

#![allow(dead_code)]

use std::collections::{BTreeMap, HashMap};

use super::types::*;
use crate::parse;
use crate::parse::conditions as condition_parser;
use crate::parse::idl as idl_parser;

/// Write a SerializableStreamSpec to a JSON file.
///
/// The file is written to `.hyperstack/{entity_name}.ast.json` relative to
/// `CARGO_MANIFEST_DIR`.
pub fn write_ast_to_file(spec: &SerializableStreamSpec, entity_name: &str) -> std::io::Result<()> {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::NotFound, e.to_string()))?;

    let ast_dir = std::path::Path::new(&manifest_dir).join(".hyperstack");
    std::fs::create_dir_all(&ast_dir)?;

    let ast_file = ast_dir.join(format!("{}.ast.json", entity_name));
    let json = serde_json::to_string_pretty(spec)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;

    std::fs::write(&ast_file, json)?;

    Ok(())
}

/// Write a SerializableStackSpec to a JSON file.
/// The file is written to `.hyperstack/{stack_name}.stack.json` relative to CARGO_MANIFEST_DIR.
pub fn write_stack_to_file(
    spec: &super::types::SerializableStackSpec,
    stack_name: &str,
) -> std::io::Result<()> {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::NotFound, e.to_string()))?;
    let ast_dir = std::path::Path::new(&manifest_dir).join(".hyperstack");
    std::fs::create_dir_all(&ast_dir)?;
    let stack_file = ast_dir.join(format!("{}.stack.json", stack_name));
    let json = serde_json::to_string_pretty(spec)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
    std::fs::write(&stack_file, json)?;
    Ok(())
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
                        variants: variants
                            .iter()
                            .map(|v| IdlEnumVariantSnapshot {
                                name: v.name.clone(),
                            })
                            .collect(),
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
                            variants: variants
                                .iter()
                                .map(|v| IdlEnumVariantSnapshot {
                                    name: v.name.clone(),
                                })
                                .collect(),
                        },
                    });
                }
            }
        }
    }

    // Determine if this IDL uses Steel-style discriminants (1 byte) or Anchor-style (8 bytes)
    // Steel IDLs have a `discriminant` field with {"type": "u8", "value": N}
    // Anchor IDLs have a `discriminator` array with 8 bytes
    let uses_steel_discriminant = idl
        .instructions
        .iter()
        .any(|ix| ix.discriminant.is_some() && ix.discriminator.is_empty());
    let discriminant_size: usize = if uses_steel_discriminant { 1 } else { 8 };
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
                    .accounts
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
    }
}

/// Build handlers from source mappings.
pub fn build_handlers_from_sources(
    sources_by_type: &HashMap<String, Vec<parse::MapAttribute>>,
    _events_by_instruction: &HashMap<String, Vec<(String, parse::EventAttribute, syn::Type)>>,
    aggregate_conditions: &HashMap<String, String>,
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
            let key = (source_type.clone(), mapping.join_on.clone());
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

            let condition = mapping.condition.as_ref().map(|cond| ConditionExpr {
                expression: cond.clone(),
                parsed: condition_parser::parse_condition_expression(cond),
            });

            let when = mapping.when.as_ref().map(|when_path| {
                let instr_type = path_to_string(when_path);
                let instr_base = instr_type.split("::").last().unwrap_or(&instr_type);
                if let Some(program_name) = program_name {
                    format!("{}::{}IxState", program_name, instr_base)
                } else {
                    format!("{}IxState", instr_base)
                }
            });

            let stop = mapping.stop.as_ref().map(|stop_path| {
                let instr_type = path_to_string(stop_path);
                let instr_base = instr_type.split("::").last().unwrap_or(&instr_type);
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
        let type_name = if let Some(program_name) = program_name {
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
    derive_from_mappings: &HashMap<String, Vec<parse::DeriveFromAttribute>>,
    aggregate_conditions: &HashMap<String, String>,
    sources_by_type: &HashMap<String, Vec<parse::MapAttribute>>,
    program_name: Option<&str>,
) -> Vec<InstructionHook> {
    // Use BTreeMap for deterministic ordering in the final output
    let mut instruction_hooks_map: BTreeMap<String, InstructionHook> = BTreeMap::new();

    // Process PDA registrations
    for registration in pda_registrations {
        let instr_type = path_to_string(&registration.instruction_path);
        let instr_base = instr_type.split("::").last().unwrap();
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

    // Process derive_from mappings (sorted for deterministic output)
    let mut sorted_derive_from: Vec<_> = derive_from_mappings.iter().collect();
    sorted_derive_from.sort_by_key(|(k, _)| *k);
    for (instruction_type, derive_attrs) in sorted_derive_from {
        let instr_base = instruction_type.split("::").last().unwrap();
        let instr_type_state = if let Some(program_name) = program_name {
            format!("{}::{}IxState", program_name, instr_base)
        } else {
            format!("{}IxState", instr_base)
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

            let condition = derive_attr.condition.as_ref().map(|cond| ConditionExpr {
                expression: cond.clone(),
                parsed: condition_parser::parse_condition_expression(cond),
            });

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
    for (field_path, condition_str) in sorted_aggregate_conditions {
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
                    let instr_type_state = if let Some(program_name) = program_name {
                        format!("{}::{}IxState", program_name, instr_base)
                    } else {
                        format!("{}IxState", instr_base)
                    };

                    let condition = ConditionExpr {
                        expression: condition_str.clone(),
                        parsed: condition_parser::parse_condition_expression(condition_str),
                    };

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

// ============================================================================
// PDA and Instruction Extraction from Anchor IDL
// ============================================================================

/// Extract PDA definitions from an Anchor IDL.
/// Returns a map of PDA name -> PdaDefinition.
pub fn extract_pdas_from_idl(idl: &idl_parser::IdlSpec) -> BTreeMap<String, PdaDefinition> {
    let mut pdas = BTreeMap::new();

    for instruction in &idl.instructions {
        for account in &instruction.accounts {
            if let Some(pda_info) = &account.pda {
                let pda_def = convert_anchor_pda_to_def(&account.name, pda_info);
                pdas.entry(account.name.clone()).or_insert(pda_def);
            }
        }
    }

    pdas
}

/// Convert Anchor IDL PDA info to our PdaDefinition type.
fn convert_anchor_pda_to_def(name: &str, pda: &idl_parser::IdlPda) -> PdaDefinition {
    let seeds = pda
        .seeds
        .iter()
        .map(|seed| match seed {
            idl_parser::IdlPdaSeed::Const { value } => {
                if let Ok(s) = String::from_utf8(value.clone()) {
                    PdaSeedDef::Literal { value: s }
                } else {
                    PdaSeedDef::Bytes {
                        value: value.clone(),
                    }
                }
            }
            idl_parser::IdlPdaSeed::Account { path, .. } => PdaSeedDef::AccountRef {
                account_name: path.clone(),
            },
            idl_parser::IdlPdaSeed::Arg { path, arg_type } => PdaSeedDef::ArgRef {
                arg_name: path.clone(),
                arg_type: arg_type.clone(),
            },
        })
        .collect();

    let program_id = pda.program.as_ref().and_then(|p| match p {
        idl_parser::IdlPdaProgram::Literal { value, .. } => Some(value.clone()),
        idl_parser::IdlPdaProgram::Const { value, .. } => Some(bs58::encode(value).into_string()),
        idl_parser::IdlPdaProgram::Account { .. } => None,
    });

    PdaDefinition {
        name: name.to_string(),
        seeds,
        program_id,
    }
}

/// Extract instruction definitions from an Anchor IDL.
pub fn extract_instructions_from_idl(
    idl: &idl_parser::IdlSpec,
    pdas: &BTreeMap<String, PdaDefinition>,
) -> Vec<InstructionDef> {
    let program_id = idl
        .address
        .clone()
        .or_else(|| idl.metadata.as_ref().and_then(|m| m.address.clone()));

    let uses_steel = idl
        .instructions
        .iter()
        .any(|ix| ix.discriminant.is_some() && ix.discriminator.is_empty());
    let discriminator_size = if uses_steel { 1 } else { 8 };

    idl.instructions
        .iter()
        .map(|ix| {
            let accounts = ix
                .accounts
                .iter()
                .map(|acc| convert_account_to_def(acc, pdas))
                .collect();

            let args = ix
                .args
                .iter()
                .map(|arg| InstructionArgDef {
                    name: arg.name.clone(),
                    arg_type: crate::parse::idl::to_rust_type_string(&arg.type_),
                    docs: vec![],
                })
                .collect();

            let errors: Vec<IdlErrorSnapshot> = idl
                .errors
                .iter()
                .map(|e| IdlErrorSnapshot {
                    code: e.code,
                    name: e.name.clone(),
                    msg: e.msg.clone(),
                })
                .collect();

            InstructionDef {
                name: ix.name.clone(),
                discriminator: ix.get_discriminator(),
                discriminator_size,
                accounts,
                args,
                errors,
                program_id: program_id.clone(),
                docs: ix.docs.clone(),
            }
        })
        .collect()
}

/// Convert an IDL account argument to our InstructionAccountDef type.
fn convert_account_to_def(
    acc: &idl_parser::IdlAccountArg,
    pdas: &BTreeMap<String, PdaDefinition>,
) -> InstructionAccountDef {
    let resolution = if acc.is_signer && acc.address.is_none() && acc.pda.is_none() {
        AccountResolution::Signer
    } else if let Some(address) = &acc.address {
        AccountResolution::Known {
            address: address.clone(),
        }
    } else if acc.pda.is_some() {
        if pdas.contains_key(&acc.name) {
            AccountResolution::PdaRef {
                pda_name: acc.name.clone(),
            }
        } else if let Some(pda_info) = &acc.pda {
            let pda_def = convert_anchor_pda_to_def(&acc.name, pda_info);
            AccountResolution::PdaInline {
                seeds: pda_def.seeds,
                program_id: pda_def.program_id,
            }
        } else {
            AccountResolution::UserProvided
        }
    } else {
        AccountResolution::UserProvided
    };

    InstructionAccountDef {
        name: acc.name.clone(),
        is_signer: acc.is_signer,
        is_writable: acc.is_mut,
        resolution,
        is_optional: acc.optional,
        docs: acc.docs.clone(),
    }
}
