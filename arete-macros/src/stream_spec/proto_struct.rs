//! Proto-based struct processing for arete streams.
//!
//! This module handles processing of structs with proto-based mapping attributes,
//! generating handler code and AST files for proto-based pipelines.

use std::collections::{BTreeMap, HashMap, HashSet};

use quote::{format_ident, quote};
use syn::spanned::Spanned;
use syn::{Fields, ItemStruct, Type};

use crate::parse;
use crate::utils::{path_to_string, to_snake_case};
use crate::validation::{validate_key_resolution_paths, KeyResolutionValidationInput};

use super::entity::{infer_resolver_type, parse_resolver_type_name, process_map_attribute};
use super::handlers::{convert_event_to_map_attributes, determine_event_instruction};
use super::sections::{is_primitive_or_wrapper, process_nested_struct};

// ============================================================================
// Proto Struct Processing
// ============================================================================

/// Process a struct with proto-based mapping attributes.
///
/// This is used for processing the main struct in a proto-based `#[stream_spec]` module
/// when there are no explicit `#[entity]` attributes.
pub fn process_struct_with_context(
    input: ItemStruct,
    section_structs: HashMap<String, ItemStruct>,
    skip_game_event: bool,
) -> syn::Result<proc_macro2::TokenStream> {
    let name = &input.ident;
    let state_name = syn::Ident::new(&format!("{}State", name), name.span());

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
    let mut has_events = false;
    let mut computed_fields: Vec<(String, proc_macro2::TokenStream, Type)> = Vec::new();
    let mut computed_field_validations = Vec::new();
    let mut resolve_specs: Vec<parse::ResolveSpec> = Vec::new();
    let mut derive_from_mappings: BTreeMap<String, Vec<parse::DeriveFromAttribute>> =
        BTreeMap::new();
    let mut aggregate_conditions: BTreeMap<String, crate::ast::ConditionExpr> = BTreeMap::new();

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
                            determine_event_instruction(&mut event_attr, field_type, None)
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
                                    "post" => crate::ast::HttpMethod::Post,
                                    _ => crate::ast::HttpMethod::Get,
                                })
                                .unwrap_or(crate::ast::HttpMethod::Get);

                            let url_source = if resolve_attr.url_is_template {
                                crate::ast::UrlSource::Template(super::entity::parse_url_template(
                                    &url_path,
                                    attr.span(),
                                )?)
                            } else {
                                crate::ast::UrlSource::FieldPath(url_path)
                            };

                            crate::ast::ResolverType::Url(crate::ast::UrlResolverConfig {
                                url_source,
                                method,
                                extract_path: resolve_attr.extract.clone(),
                            })
                        } else if let Some(name) = resolve_attr.resolver.as_deref() {
                            parse_resolver_type_name(name, field_type)?
                        } else {
                            infer_resolver_type(field_type)?
                        };

                        resolve_specs.push(parse::ResolveSpec {
                            attr_span: resolve_attr.attr_span,
                            from_span: resolve_attr.from_span,
                            resolver,
                            from: if resolve_attr.url_is_template {
                                None
                            } else {
                                resolve_attr.url.clone().or(resolve_attr.from)
                            },
                            address: resolve_attr.address,
                            extract: resolve_attr.extract,
                            target_field_name: resolve_attr.target_field_name,
                            strategy: resolve_attr.strategy,
                            condition: resolve_attr.condition,
                            schedule_at: resolve_attr.schedule_at,
                        });
                    }
                    Some(_) | None => {}
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
                                None,
                            )?;
                        }
                    }
                }
            }
        }
    }

    let mut handler_fns = Vec::new();
    let mut handler_calls = Vec::new();

    // === HANDLER MERGING: Merge event mappings into sources_by_type ===
    // Convert events to map attributes and merge with sources_by_type (no IDL for proto-based specs)
    for event_mappings in events_by_instruction.values() {
        for (target_field, event_attr, _field_type) in event_mappings {
            // Get instruction path from event attribute
            let instruction_path = event_attr
                .from_instruction
                .as_ref()
                .or(event_attr.inferred_instruction.as_ref());

            if let Some(instr_path) = instruction_path {
                // Convert instruction path to string for sources_by_type key
                let source_type_str = path_to_string(instr_path);

                // Convert event to map attributes (without IDL)
                let map_attrs = convert_event_to_map_attributes(
                    target_field,
                    event_attr,
                    instr_path,
                    None, // No IDL for proto-based specs
                );

                // Merge into sources_by_type
                sources_by_type
                    .entry(source_type_str)
                    .or_default()
                    .extend(map_attrs);
            }
        }
    }

    let mut key_resolution_errors = crate::diagnostic::ErrorCollector::default();
    validate_key_resolution_paths(
        KeyResolutionValidationInput {
            entity_name: &name.to_string(),
            primary_keys: &primary_keys,
            lookup_indexes: &lookup_indexes,
            sources_by_type: &sources_by_type,
            events_by_instruction: &events_by_instruction,
            derive_from_mappings: &derive_from_mappings,
            resolver_hooks: &[],
        },
        &mut key_resolution_errors,
    );
    key_resolution_errors.finish()?;

    let mut sources_by_type_and_join: HashMap<(String, Option<String>), Vec<parse::MapAttribute>> =
        HashMap::new();
    for (source_type, mappings) in &sources_by_type {
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
        let handler_suffix = if let Some(ref join_field) = join_key {
            format!(
                "{}_{}",
                to_snake_case(source_type),
                to_snake_case(join_field)
            )
        } else {
            to_snake_case(source_type)
        };
        let handler_name = format_ident!("create_{}_handler", handler_suffix);
        let account_type = source_type.split("::").last().unwrap_or(source_type);

        // Check if any mapping is from an instruction
        let is_instruction = mappings.iter().any(|m| m.is_instruction);

        let mut field_mapping_code = Vec::new();
        let mut primary_field_path =
            quote! { arete::runtime::arete_interpreter::ast::FieldPath::new(&[]) };
        let mut has_primary_key = false;
        let mut lookup_primary_field = None;

        for mapping in mappings {
            let target_field = &mapping.target_field_name;
            let source_field = &mapping.source_field_name;
            let strategy_str = &mapping.strategy;
            let strategy_ident = format_ident!("{}", strategy_str);

            let mapping_expr = if mapping.is_whole_source && !is_instruction {
                // Whole account capture - use WholeSource for accounts (not instructions)
                quote! {
                    arete::runtime::arete_interpreter::ast::TypedFieldMapping::new(
                        #target_field.to_string(),
                        arete::runtime::arete_interpreter::ast::MappingSource::WholeSource,
                        arete::runtime::arete_interpreter::ast::PopulationStrategy::#strategy_ident,
                    )
                }
            } else if mapping.is_whole_source {
                // Whole instruction capture - use "data" path to capture entire instruction
                quote! {
                    arete::runtime::arete_interpreter::ast::TypedFieldMapping::new(
                        #target_field.to_string(),
                        arete::runtime::arete_interpreter::ast::MappingSource::FromSource {
                            path: arete::runtime::arete_interpreter::ast::FieldPath::new(&["data"]),
                            default: None,
                            transform: None,
                        },
                        arete::runtime::arete_interpreter::ast::PopulationStrategy::#strategy_ident,
                    )
                }
            } else {
                // Normal field mapping
                quote! {
                    arete::runtime::arete_interpreter::ast::TypedFieldMapping::new(
                        #target_field.to_string(),
                        arete::runtime::arete_interpreter::ast::MappingSource::FromSource {
                            path: arete::runtime::arete_interpreter::ast::FieldPath::new(&[#source_field]),
                            default: None,
                            transform: None,
                        },
                        arete::runtime::arete_interpreter::ast::PopulationStrategy::#strategy_ident,
                    )
                }
            };

            let mapping_expr = if let Some(ref transform_str) = mapping.transform {
                let transform_ident = format_ident!("{}", transform_str);
                quote! {
                    #mapping_expr.with_transform(arete::runtime::arete_interpreter::ast::Transformation::#transform_ident)
                }
            } else {
                mapping_expr
            };

            let mapping_expr = if !mapping.emit {
                quote! {
                    #mapping_expr.with_emit(false)
                }
            } else {
                mapping_expr
            };

            field_mapping_code.push(mapping_expr);

            if mapping.is_primary_key {
                has_primary_key = true;
                primary_field_path = quote! {
                    arete::runtime::arete_interpreter::ast::FieldPath::new(&[#source_field])
                };
            }

            if primary_keys.contains(&mapping.target_field_name) {
                lookup_primary_field = Some(quote! {
                    arete::runtime::arete_interpreter::ast::FieldPath::new(&[#source_field])
                });
            }
        }

        let key_resolution = if has_primary_key {
            quote! {
                arete::runtime::arete_interpreter::ast::KeyResolutionStrategy::Embedded {
                    primary_field: #primary_field_path,
                }
            }
        } else {
            let lookup_field = if let Some(ref join_field_name) = join_key {
                quote! {
                    arete::runtime::arete_interpreter::ast::FieldPath::new(&[#join_field_name])
                }
            } else {
                lookup_primary_field.unwrap_or_else(|| {
                    if let Some(pk) = primary_keys.first() {
                        for mapping in mappings {
                            if mapping.target_field_name == *pk {
                                let source_field = &mapping.source_field_name;
                                return quote! {
                                    arete::runtime::arete_interpreter::ast::FieldPath::new(&[#source_field])
                                };
                            }
                        }
                        let event_field = pk.split('.').next_back().unwrap_or(pk);
                        return quote! {
                            arete::runtime::arete_interpreter::ast::FieldPath::new(&[#event_field])
                        };
                    }
                    quote! { arete::runtime::arete_interpreter::ast::FieldPath::new(&[]) }
                })
            };

            quote! {
                arete::runtime::arete_interpreter::ast::KeyResolutionStrategy::Lookup {
                    primary_field: #lookup_field,
                }
            }
        };

        let type_suffix = if is_instruction { "IxState" } else { "State" };
        let is_account_source = !is_instruction;
        handler_fns.push(quote! {
            fn #handler_name() -> arete::runtime::arete_interpreter::ast::TypedHandlerSpec<#state_name> {
                arete::runtime::arete_interpreter::ast::TypedHandlerSpec::new(
                    arete::runtime::arete_interpreter::ast::SourceSpec::Source {
                        program_id: None,
                        discriminator: None,
                        type_name: format!("{}{}", #account_type, #type_suffix),
                        serialization: None,
                        is_account: #is_account_source,
                    },
                    #key_resolution,
                    vec![
                        #(#field_mapping_code),*
                    ],
                    true,
                )
            }
        });

        handler_calls.push(quote! {
            #handler_name()
        });
    }

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

    let lookup_index_creations: Vec<_> = lookup_indexes
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

    let mut resolver_specs_by_key: HashMap<
        (crate::ast::ResolverType, String, String),
        Vec<parse::ResolveSpec>,
    > = HashMap::new();
    for spec in &resolve_specs {
        let input_key = if let Some(from) = &spec.from {
            format!("path:{}", from)
        } else if let Some(address) = &spec.address {
            format!("value:{}", address)
        } else {
            "value:".to_string()
        };
        resolver_specs_by_key
            .entry((spec.resolver.clone(), input_key, spec.strategy.clone()))
            .or_default()
            .push(spec.clone());
    }

    let resolver_specs_code: Vec<_> = resolver_specs_by_key
        .into_iter()
        .map(|((resolver, _input_key, strategy), specs)| -> syn::Result<proc_macro2::TokenStream> {
            let resolver_code = match resolver {
                crate::ast::ResolverType::Token => quote! {
                    arete::runtime::arete_interpreter::ast::ResolverType::Token
                },
                crate::ast::ResolverType::Url(config) => {
                    let url_source_code = match &config.url_source {
                        crate::ast::UrlSource::FieldPath(path) => {
                            quote! {
                                arete::runtime::arete_interpreter::ast::UrlSource::FieldPath(#path.to_string())
                            }
                        }
                        crate::ast::UrlSource::Template(parts) => {
                            let parts_code: Vec<_> = parts.iter().map(|part| match part {
                                crate::ast::UrlTemplatePart::Literal(s) => quote! {
                                    arete::runtime::arete_interpreter::ast::UrlTemplatePart::Literal(#s.to_string())
                                },
                                crate::ast::UrlTemplatePart::FieldRef(f) => quote! {
                                    arete::runtime::arete_interpreter::ast::UrlTemplatePart::FieldRef(#f.to_string())
                                },
                            }).collect();
                            quote! {
                                arete::runtime::arete_interpreter::ast::UrlSource::Template(vec![#(#parts_code),*])
                            }
                        }
                    };
                    let method_code = match config.method {
                        crate::ast::HttpMethod::Get => quote! {
                            arete::runtime::arete_interpreter::ast::HttpMethod::Get
                        },
                        crate::ast::HttpMethod::Post => quote! {
                            arete::runtime::arete_interpreter::ast::HttpMethod::Post
                        },
                    };
                    let extract_path_code = match &config.extract_path {
                        Some(path) => quote! { Some(#path.to_string()) },
                        None => quote! { None },
                    };
                    quote! {
                        arete::runtime::arete_interpreter::ast::ResolverType::Url(
                            arete::runtime::arete_interpreter::ast::UrlResolverConfig {
                                url_source: #url_source_code,
                                method: #method_code,
                                extract_path: #extract_path_code,
                            }
                        )
                    }
                },
            };
            let strategy_code = match strategy.as_str() {
                "LastWrite" => quote! {
                    arete::runtime::arete_interpreter::ast::ResolveStrategy::LastWrite
                },
                _ => quote! {
                    arete::runtime::arete_interpreter::ast::ResolveStrategy::SetOnce
                },
            };
            let input_path_code = match specs.first().and_then(|spec| spec.from.as_ref()) {
                Some(value) => quote! { Some(#value.to_string()) },
                None => quote! { None },
            };
            let input_value_code = match specs.first().and_then(|spec| spec.address.as_ref()) {
                Some(value) => quote! {
                    Some(arete::runtime::serde_json::Value::String(#value.to_string()))
                },
                None => quote! { None },
            };

            let mut seen = HashSet::new();
            let extracts_code: Vec<_> = specs
                .iter()
                .filter_map(|spec| {
                    let key = format!("{}::{:?}", spec.target_field_name, spec.extract);
                    if !seen.insert(key) {
                        return None;
                    }
                    let target = &spec.target_field_name;
                    let source = spec.extract.as_ref();
                    let source_code = match source {
                        Some(value) => quote! { Some(#value.to_string()) },
                        None => quote! { None },
                    };
                    Some(quote! {
                        arete::runtime::arete_interpreter::ast::ResolverExtractSpec {
                            target_path: #target.to_string(),
                            source_path: #source_code,
                            transform: None,
                        }
                    })
                })
                .collect();

            let condition_code = match specs.first().and_then(|s| s.condition.as_ref()) {
                Some(cond_str) => {
                    let parsed = &cond_str.parsed;
                    let field_path = &parsed.field_path;
                    let op_code = match parsed.op {
                        crate::ast::ComparisonOp::Equal => quote! { arete::runtime::arete_interpreter::ast::ComparisonOp::Equal },
                        crate::ast::ComparisonOp::NotEqual => quote! { arete::runtime::arete_interpreter::ast::ComparisonOp::NotEqual },
                        crate::ast::ComparisonOp::GreaterThan => quote! { arete::runtime::arete_interpreter::ast::ComparisonOp::GreaterThan },
                        crate::ast::ComparisonOp::LessThan => quote! { arete::runtime::arete_interpreter::ast::ComparisonOp::LessThan },
                        crate::ast::ComparisonOp::GreaterThanOrEqual => quote! { arete::runtime::arete_interpreter::ast::ComparisonOp::GreaterThanOrEqual },
                        crate::ast::ComparisonOp::LessThanOrEqual => quote! { arete::runtime::arete_interpreter::ast::ComparisonOp::LessThanOrEqual },
                    };
                    let val_code = match &parsed.value {
                        serde_json::Value::Null => quote! { arete::runtime::serde_json::Value::Null },
                        serde_json::Value::Bool(b) => quote! { arete::runtime::serde_json::Value::Bool(#b) },
                        serde_json::Value::Number(n) => {
                            let n_str = n.to_string();
                            quote! { arete::runtime::serde_json::json!(#n_str.parse::<f64>().unwrap()) }
                        }
                        serde_json::Value::String(s) => quote! { arete::runtime::serde_json::Value::String(#s.to_string()) },
                        _ => quote! { arete::runtime::serde_json::Value::Null },
                    };
                    quote! {
                        Some(arete::runtime::arete_interpreter::ast::ResolverCondition {
                            field_path: #field_path.to_string(),
                            op: #op_code,
                            value: #val_code,
                        })
                    }
                }
                None => quote! { None },
            };

            let schedule_at_code = match specs.first().and_then(|s| s.schedule_at.as_ref()) {
                Some(path) => {
                    let raw = &path.raw;
                    quote! { Some(#raw.to_string()) }
                }
                None => quote! { None },
            };

            Ok(quote! {
                arete::runtime::arete_interpreter::ast::ResolverSpec {
                    resolver: #resolver_code,
                    input_path: #input_path_code,
                    input_value: #input_value_code,
                    strategy: #strategy_code,
                    extracts: vec![
                        #(#extracts_code),*
                    ],
                    condition: #condition_code,
                    schedule_at: #schedule_at_code,
                }
            })
        })
        .collect::<syn::Result<Vec<_>>>()?;

    let output = quote! {
        #[derive(Debug, Clone, arete::runtime::serde::Serialize, arete::runtime::serde::Deserialize)]
        pub struct #state_name {
            #(#state_fields),*
        }

        #game_event_struct

        pub mod fields {
            use super::*;

            #(#accessor_defs)*
        }

        pub fn create_spec() -> arete::runtime::arete_interpreter::ast::TypedStreamSpec<#state_name> {
            arete::runtime::arete_interpreter::ast::TypedStreamSpec::new(
                stringify!(#name).to_string(),
                arete::runtime::arete_interpreter::ast::IdentitySpec {
                    primary_keys: vec![#(#primary_keys.to_string()),*],
                    lookup_indexes: vec![
                        #(#lookup_index_creations),*
                    ],
                },
                vec![
                    #(#handler_calls),*
                ],
            )
            .with_resolver_specs(vec![
                #(#resolver_specs_code),*
            ])
        }

        #(#handler_fns)*
    };

    Ok(output)
}
