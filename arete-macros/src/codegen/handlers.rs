//! Shared handler code generation for arete-macros.
//!
//! This module provides unified handler generation that can be used by both:
//! - `#[arete]` - generates handlers during macro expansion
//! - `#[ast_spec]` - generates handlers from serialized AST
//!
//! The key abstraction is `build_handler_code` which takes a `SerializableHandlerSpec`
//! and generates the corresponding Rust code for creating a `TypedHandlerSpec`.

use proc_macro2::TokenStream;
use quote::{format_ident, quote, quote_spanned};

use crate::ast::{
    ComparisonOp, ConditionExpr, FieldPath, IdlSerializationSnapshot, KeyResolutionStrategy,
    LogicalOp, MappingSource, ParsedCondition, PopulationStrategy, SerializableFieldMapping,
    SerializableHandlerSpec, SourceSpec, Transformation,
};

/// Build handler code from a serializable handler spec.
///
/// This is the main entry point for generating handler code. It takes a
/// `SerializableHandlerSpec` (which can come from either macro expansion or
/// deserialized AST) and generates the corresponding `TypedHandlerSpec` construction code.
///
/// # Arguments
///
/// * `handler` - The handler specification to generate code for
/// * `state_name` - The name of the state struct (e.g., "BondingCurveState")
///
/// # Returns
///
/// A `TokenStream` containing the code to construct a `TypedHandlerSpec`.
pub fn build_handler_code(
    handler: &SerializableHandlerSpec,
    state_name: &syn::Ident,
) -> TokenStream {
    // Generate source spec code
    let source_code = build_source_spec_code(&handler.source);

    // Generate key resolution code
    let key_resolution_code = build_key_resolution_code(&handler.key_resolution);

    // Generate field mapping code
    let mappings_code: Vec<TokenStream> = handler
        .mappings
        .iter()
        .map(build_field_mapping_code)
        .collect();

    let emit = handler.emit;

    quote_spanned! { state_name.span()=>
        arete::runtime::arete_interpreter::ast::TypedHandlerSpec::<#state_name>::new(
            #source_code,
            #key_resolution_code,
            vec![
                #(#mappings_code),*
            ],
            #emit,
        )
    }
}

/// Build a handler function definition.
///
/// This generates a complete function that returns a `TypedHandlerSpec`.
///
/// # Arguments
///
/// * `handler` - The handler specification
/// * `handler_name` - The name for the generated function
/// * `state_name` - The name of the state struct
pub fn build_handler_fn(
    handler: &SerializableHandlerSpec,
    handler_name: &syn::Ident,
    state_name: &syn::Ident,
) -> TokenStream {
    let handler_code = build_handler_code(handler, state_name);

    quote_spanned! { handler_name.span()=>
        fn #handler_name() -> arete::runtime::arete_interpreter::ast::TypedHandlerSpec<#state_name> {
            #handler_code
        }
    }
}

/// Generate code for SourceSpec.
fn build_source_spec_code(source: &SourceSpec) -> TokenStream {
    match source {
        SourceSpec::Source {
            program_id,
            discriminator,
            type_name,
            serialization,
            is_account,
        } => {
            let program_id_code = match program_id {
                Some(id) => quote! { Some(#id.to_string()) },
                None => quote! { None },
            };

            let discriminator_code = match discriminator {
                Some(disc) => {
                    let bytes = disc.iter();
                    quote! { Some(vec![#(#bytes),*]) }
                }
                None => quote! { None },
            };

            let serialization_code = match serialization {
                Some(IdlSerializationSnapshot::Borsh) => quote! {
                    Some(arete::runtime::arete_interpreter::ast::IdlSerializationSnapshot::Borsh)
                },
                Some(IdlSerializationSnapshot::Bytemuck) => quote! {
                    Some(arete::runtime::arete_interpreter::ast::IdlSerializationSnapshot::Bytemuck)
                },
                Some(IdlSerializationSnapshot::BytemuckUnsafe) => quote! {
                    Some(arete::runtime::arete_interpreter::ast::IdlSerializationSnapshot::BytemuckUnsafe)
                },
                None => quote! { None },
            };

            quote! {
                arete::runtime::arete_interpreter::ast::SourceSpec::Source {
                    program_id: #program_id_code,
                    discriminator: #discriminator_code,
                    type_name: #type_name.to_string(),
                    serialization: #serialization_code,
                    is_account: #is_account,
                }
            }
        }
    }
}

/// Generate code for KeyResolutionStrategy.
fn build_key_resolution_code(strategy: &KeyResolutionStrategy) -> TokenStream {
    match strategy {
        KeyResolutionStrategy::Embedded { primary_field } => {
            let field_path_code = build_field_path_code(primary_field);
            quote! {
                arete::runtime::arete_interpreter::ast::KeyResolutionStrategy::Embedded {
                    primary_field: #field_path_code,
                }
            }
        }
        KeyResolutionStrategy::Lookup { primary_field } => {
            let field_path_code = build_field_path_code(primary_field);
            quote! {
                arete::runtime::arete_interpreter::ast::KeyResolutionStrategy::Lookup {
                    primary_field: #field_path_code,
                }
            }
        }
        KeyResolutionStrategy::Computed {
            primary_field,
            compute_partition,
        } => {
            let field_path_code = build_field_path_code(primary_field);
            let compute_code = build_compute_function_code(compute_partition);
            quote! {
                arete::runtime::arete_interpreter::ast::KeyResolutionStrategy::Computed {
                    primary_field: #field_path_code,
                    compute_partition: #compute_code,
                }
            }
        }
        KeyResolutionStrategy::TemporalLookup {
            lookup_field,
            timestamp_field,
            index_name,
        } => {
            let lookup_code = build_field_path_code(lookup_field);
            let timestamp_code = build_field_path_code(timestamp_field);
            quote! {
                arete::runtime::arete_interpreter::ast::KeyResolutionStrategy::TemporalLookup {
                    lookup_field: #lookup_code,
                    timestamp_field: #timestamp_code,
                    index_name: #index_name.to_string(),
                }
            }
        }
    }
}

/// Generate code for FieldPath.
fn build_field_path_code(path: &FieldPath) -> TokenStream {
    let segments: Vec<&str> = path.segments.iter().map(|s| s.as_str()).collect();
    quote! {
        arete::runtime::arete_interpreter::ast::FieldPath::new(&[#(#segments),*])
    }
}

/// Generate code for ComputeFunction.
fn build_compute_function_code(func: &crate::ast::ComputeFunction) -> TokenStream {
    match func {
        crate::ast::ComputeFunction::Sum => {
            quote! { arete::runtime::arete_interpreter::ast::ComputeFunction::Sum }
        }
        crate::ast::ComputeFunction::Concat => {
            quote! { arete::runtime::arete_interpreter::ast::ComputeFunction::Concat }
        }
        crate::ast::ComputeFunction::Format(fmt) => {
            quote! { arete::runtime::arete_interpreter::ast::ComputeFunction::Format(#fmt.to_string()) }
        }
        crate::ast::ComputeFunction::Custom(name) => {
            quote! { arete::runtime::arete_interpreter::ast::ComputeFunction::Custom(#name.to_string()) }
        }
    }
}

/// Generate code for a single field mapping.
fn build_field_mapping_code(mapping: &SerializableFieldMapping) -> TokenStream {
    let target_path = &mapping.target_path;
    let source_code = build_mapping_source_code(&mapping.source);
    let population_code = build_population_strategy_code(&mapping.population);

    let mut mapping_code = quote! {
        arete::runtime::arete_interpreter::ast::TypedFieldMapping::new(
            #target_path.to_string(),
            #source_code,
            #population_code,
        )
    };

    if let Some(transform) = &mapping.transform {
        let transform_code = build_transformation_code(transform);
        mapping_code = quote! {
            #mapping_code.with_transform(#transform_code)
        };
    }

    if let Some(condition) = &mapping.condition {
        let condition_code = build_condition_expr_code(condition);
        mapping_code = quote! {
            #mapping_code.with_condition(#condition_code)
        };
    }

    if let Some(when) = &mapping.when {
        mapping_code = quote! {
            #mapping_code.with_when(#when.to_string())
        };
    }

    if let Some(stop) = &mapping.stop {
        mapping_code = quote! {
            #mapping_code.with_stop(#stop.to_string())
        };
    }

    if !mapping.emit {
        mapping_code = quote! {
            #mapping_code.with_emit(false)
        };
    }

    mapping_code
}

fn build_condition_expr_code(condition: &ConditionExpr) -> TokenStream {
    let expression = &condition.expression;
    let parsed_code = match &condition.parsed {
        Some(parsed) => {
            let parsed_code = build_parsed_condition_code(parsed);
            quote! { Some(#parsed_code) }
        }
        None => quote! { None },
    };

    quote! {
        arete::runtime::arete_interpreter::ast::ConditionExpr {
            expression: #expression.to_string(),
            parsed: #parsed_code,
        }
    }
}

fn build_parsed_condition_code(condition: &ParsedCondition) -> TokenStream {
    match condition {
        ParsedCondition::Comparison { field, op, value } => {
            let field_code = build_field_path_code(field);
            let op_code = build_comparison_op_code(op);
            let value_str = serde_json::to_string(value).unwrap_or_else(|_| "null".to_string());
            quote! {
                arete::runtime::arete_interpreter::ast::ParsedCondition::Comparison {
                    field: #field_code,
                    op: #op_code,
                    value: arete::runtime::serde_json::from_str(#value_str)
                        .unwrap_or(arete::runtime::serde_json::Value::Null),
                }
            }
        }
        ParsedCondition::Logical { op, conditions } => {
            let op_code = build_logical_op_code(op);
            let nested: Vec<TokenStream> =
                conditions.iter().map(build_parsed_condition_code).collect();
            quote! {
                arete::runtime::arete_interpreter::ast::ParsedCondition::Logical {
                    op: #op_code,
                    conditions: vec![#(#nested),*],
                }
            }
        }
    }
}

fn build_comparison_op_code(op: &ComparisonOp) -> TokenStream {
    match op {
        ComparisonOp::Equal => {
            quote! { arete::runtime::arete_interpreter::ast::ComparisonOp::Equal }
        }
        ComparisonOp::NotEqual => {
            quote! { arete::runtime::arete_interpreter::ast::ComparisonOp::NotEqual }
        }
        ComparisonOp::GreaterThan => {
            quote! { arete::runtime::arete_interpreter::ast::ComparisonOp::GreaterThan }
        }
        ComparisonOp::GreaterThanOrEqual => {
            quote! { arete::runtime::arete_interpreter::ast::ComparisonOp::GreaterThanOrEqual }
        }
        ComparisonOp::LessThan => {
            quote! { arete::runtime::arete_interpreter::ast::ComparisonOp::LessThan }
        }
        ComparisonOp::LessThanOrEqual => {
            quote! { arete::runtime::arete_interpreter::ast::ComparisonOp::LessThanOrEqual }
        }
    }
}

fn build_logical_op_code(op: &LogicalOp) -> TokenStream {
    match op {
        LogicalOp::And => {
            quote! { arete::runtime::arete_interpreter::ast::LogicalOp::And }
        }
        LogicalOp::Or => quote! { arete::runtime::arete_interpreter::ast::LogicalOp::Or },
    }
}

/// Generate code for MappingSource.
fn build_mapping_source_code(source: &MappingSource) -> TokenStream {
    match source {
        MappingSource::FromSource {
            path,
            default,
            transform,
        } => {
            let path_code = build_field_path_code(path);
            let default_code = match default {
                Some(val) => {
                    let val_str = serde_json::to_string(val).unwrap_or_else(|_| "null".to_string());
                    quote! { Some(arete::runtime::serde_json::from_str(#val_str).unwrap_or(arete::runtime::serde_json::Value::Null)) }
                }
                None => quote! { None },
            };
            let transform_code = match transform {
                Some(t) => {
                    let t_code = build_transformation_code(t);
                    quote! { Some(#t_code) }
                }
                None => quote! { None },
            };
            quote! {
                arete::runtime::arete_interpreter::ast::MappingSource::FromSource {
                    path: #path_code,
                    default: #default_code,
                    transform: #transform_code,
                }
            }
        }
        MappingSource::Constant(val) => {
            let val_str = serde_json::to_string(val).unwrap_or_else(|_| "null".to_string());
            quote! {
                arete::runtime::arete_interpreter::ast::MappingSource::Constant(
                    arete::runtime::serde_json::from_str(#val_str).unwrap_or(arete::runtime::serde_json::Value::Null)
                )
            }
        }
        MappingSource::Computed { inputs, function } => {
            let inputs_code: Vec<TokenStream> = inputs.iter().map(build_field_path_code).collect();
            let func_code = build_compute_function_code(function);
            quote! {
                arete::runtime::arete_interpreter::ast::MappingSource::Computed {
                    inputs: vec![#(#inputs_code),*],
                    function: #func_code,
                }
            }
        }
        MappingSource::FromState { path } => {
            quote! {
                arete::runtime::arete_interpreter::ast::MappingSource::FromState {
                    path: #path.to_string(),
                }
            }
        }
        MappingSource::AsEvent { fields } => {
            let fields_code: Vec<TokenStream> = fields
                .iter()
                .map(|f| {
                    let source_code = build_mapping_source_code(f);
                    quote! { Box::new(#source_code) }
                })
                .collect();
            quote! {
                arete::runtime::arete_interpreter::ast::MappingSource::AsEvent {
                    fields: vec![#(#fields_code),*],
                }
            }
        }
        MappingSource::WholeSource => {
            quote! { arete::runtime::arete_interpreter::ast::MappingSource::WholeSource }
        }
        MappingSource::AsCapture { field_transforms } => {
            let transform_insertions: Vec<TokenStream> = field_transforms
                .iter()
                .map(|(field, transform)| {
                    let transform_code = build_transformation_code(transform);
                    quote! {
                        field_transforms.insert(#field.to_string(), #transform_code);
                    }
                })
                .collect();

            if transform_insertions.is_empty() {
                quote! {
                    arete::runtime::arete_interpreter::ast::MappingSource::AsCapture {
                        field_transforms: std::collections::BTreeMap::new(),
                    }
                }
            } else {
                quote! {
                    {
                        let mut field_transforms = std::collections::BTreeMap::new();
                        #(#transform_insertions)*
                        arete::runtime::arete_interpreter::ast::MappingSource::AsCapture {
                            field_transforms,
                        }
                    }
                }
            }
        }
        MappingSource::FromContext { field } => {
            quote! {
                arete::runtime::arete_interpreter::ast::MappingSource::FromContext {
                    field: #field.to_string(),
                }
            }
        }
    }
}

/// Generate code for PopulationStrategy.
fn build_population_strategy_code(strategy: &PopulationStrategy) -> TokenStream {
    match strategy {
        PopulationStrategy::SetOnce => {
            quote! { arete::runtime::arete_interpreter::ast::PopulationStrategy::SetOnce }
        }
        PopulationStrategy::LastWrite => {
            quote! { arete::runtime::arete_interpreter::ast::PopulationStrategy::LastWrite }
        }
        PopulationStrategy::Append => {
            quote! { arete::runtime::arete_interpreter::ast::PopulationStrategy::Append }
        }
        PopulationStrategy::Merge => {
            quote! { arete::runtime::arete_interpreter::ast::PopulationStrategy::Merge }
        }
        PopulationStrategy::Max => {
            quote! { arete::runtime::arete_interpreter::ast::PopulationStrategy::Max }
        }
        PopulationStrategy::Sum => {
            quote! { arete::runtime::arete_interpreter::ast::PopulationStrategy::Sum }
        }
        PopulationStrategy::Count => {
            quote! { arete::runtime::arete_interpreter::ast::PopulationStrategy::Count }
        }
        PopulationStrategy::Min => {
            quote! { arete::runtime::arete_interpreter::ast::PopulationStrategy::Min }
        }
        PopulationStrategy::UniqueCount => {
            quote! { arete::runtime::arete_interpreter::ast::PopulationStrategy::UniqueCount }
        }
    }
}

/// Generate code for Transformation.
fn build_transformation_code(transform: &Transformation) -> TokenStream {
    match transform {
        Transformation::HexEncode => {
            quote! { arete::runtime::arete_interpreter::ast::Transformation::HexEncode }
        }
        Transformation::HexDecode => {
            quote! { arete::runtime::arete_interpreter::ast::Transformation::HexDecode }
        }
        Transformation::Base58Encode => {
            quote! { arete::runtime::arete_interpreter::ast::Transformation::Base58Encode }
        }
        Transformation::Base58Decode => {
            quote! { arete::runtime::arete_interpreter::ast::Transformation::Base58Decode }
        }
        Transformation::ToString => {
            quote! { arete::runtime::arete_interpreter::ast::Transformation::ToString }
        }
        Transformation::ToNumber => {
            quote! { arete::runtime::arete_interpreter::ast::Transformation::ToNumber }
        }
    }
}

/// Generate handlers from a list of handler specs.
///
/// This is a higher-level helper that generates all handler functions and their calls.
///
/// # Arguments
///
/// * `handlers` - List of handler specifications
/// * `entity_name` - The entity name (used for function naming)
/// * `state_name` - The state struct name
///
/// # Returns
///
/// A tuple of (handler_functions, handler_calls) where:
/// - handler_functions are the fn definitions
/// - handler_calls are the invocation expressions
pub fn generate_handlers_from_specs(
    handlers: &[SerializableHandlerSpec],
    entity_name: &str,
    state_name: &syn::Ident,
) -> (Vec<TokenStream>, Vec<TokenStream>) {
    let mut handler_fns = Vec::new();
    let mut handler_calls = Vec::new();

    for (i, handler) in handlers.iter().enumerate() {
        // Extract type name for handler naming
        let type_name = match &handler.source {
            SourceSpec::Source { type_name, .. } => type_name.clone(),
        };

        // Generate handler name from type
        let handler_suffix = crate::utils::to_snake_case(&type_name);
        let handler_name = format_ident!(
            "create_{}_{}_handler_{}",
            crate::utils::to_snake_case(entity_name),
            handler_suffix,
            i
        );

        let handler_fn = build_handler_fn(handler, &handler_name, state_name);
        handler_fns.push(handler_fn);
        handler_calls.push(quote! { #handler_name() });
    }

    (handler_fns, handler_calls)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_field_path_code() {
        let path = FieldPath::new(&["accounts", "mint"]);
        let code = build_field_path_code(&path);
        let code_str = code.to_string();
        assert!(code_str.contains("FieldPath"));
        assert!(code_str.contains("accounts"));
        assert!(code_str.contains("mint"));
    }

    #[test]
    fn test_build_population_strategy_code() {
        let strategy = PopulationStrategy::Sum;
        let code = build_population_strategy_code(&strategy);
        let code_str = code.to_string();
        assert!(code_str.contains("Sum"));
    }
}
