//! Core utilities shared across code generation modules.

use proc_macro2::TokenStream;
use quote::quote;

use crate::ast::*;

/// Generate hook action implementations.
pub fn generate_hook_actions(
    actions: &[HookAction],
    _lookup_by: &Option<FieldPath>,
) -> TokenStream {
    let action_code: Vec<TokenStream> = actions
        .iter()
        .map(|action| {
            match action {
                HookAction::RegisterPdaMapping {
                    pda_field,
                    seed_field,
                    lookup_name: _,
                } => {
                    let pda_raw = pda_field.segments.last().cloned().unwrap_or_default();
                    let seed_raw = seed_field.segments.last().cloned().unwrap_or_default();
                    let pda_camel = crate::event_type_helpers::snake_to_lower_camel(&pda_raw);
                    let seed_camel = crate::event_type_helpers::snake_to_lower_camel(&seed_raw);

                    // IDL account names can be camelCase (e.g. Pumpfun: "bondingCurve") or
                    // snake_case (e.g. Raydium: "pool_state"). The register_from attribute
                    // uses Rust field names (always snake_case), so try both the camelCase
                    // conversion and the raw snake_case name to match the IDL.
                    quote! {
                        let pda_val = ctx.account(#pda_camel).or_else(|| ctx.account(#pda_raw));
                        let seed_val = ctx.account(#seed_camel).or_else(|| ctx.account(#seed_raw));
                        if let (Some(pda), Some(seed)) = (pda_val, seed_val) {
                            ctx.register_pda_reverse_lookup(&pda, &seed);
                        }
                    }
                }
                HookAction::SetField {
                    target_field,
                    source,
                    condition,
                } => {
                    let set_code = generate_set_field_code(target_field, source);
                    if let Some(cond) = condition {
                        let cond_code = generate_condition_code(cond);
                        quote! {
                            if #cond_code {
                                #set_code
                            }
                        }
                    } else {
                        set_code
                    }
                }
                HookAction::IncrementField {
                    target_field,
                    increment_by,
                    condition,
                } => {
                    let increment_code = quote! {
                        ctx.increment(#target_field, #increment_by);
                    };
                    if let Some(cond) = condition {
                        let cond_code = generate_condition_code(cond);
                        quote! {
                            if #cond_code {
                                #increment_code
                            }
                        }
                    } else {
                        increment_code
                    }
                }
            }
        })
        .collect();

    quote! {
        #(#action_code)*
    }
}

/// Generate code for a SetField action.
pub fn generate_set_field_code(target_field: &str, source: &MappingSource) -> TokenStream {
    match source {
        MappingSource::FromSource {
            path,
            default: _,
            transform: _,
        } => {
            let field_str = path.segments.last().cloned().unwrap_or_default();
            quote! {
                if let Some(value) = ctx.data::<hyperstack::runtime::serde_json::Value>(#field_str) {
                    ctx.set(#target_field, value);
                }
            }
        }
        MappingSource::Constant(value) => {
            // Convert serde_json::Value to a string representation for embedding
            let value_str = serde_json::to_string(value).unwrap_or_else(|_| "null".to_string());
            quote! {
                ctx.set(#target_field, hyperstack::runtime::serde_json::from_str::<hyperstack::runtime::serde_json::Value>(#value_str).unwrap_or(hyperstack::runtime::serde_json::Value::Null));
            }
        }
        MappingSource::FromContext { field } => match field.as_str() {
            "slot" => {
                quote! { ctx.set(#target_field, hyperstack::runtime::serde_json::json!(ctx.slot().unwrap_or(0))); }
            }
            "signature" => {
                quote! { ctx.set(#target_field, hyperstack::runtime::serde_json::json!(ctx.signature().unwrap_or_default())); }
            }
            "timestamp" => {
                quote! { ctx.set(#target_field, hyperstack::runtime::serde_json::json!(ctx.timestamp())); }
            }
            _ => quote! {},
        },
        _ => quote! {},
    }
}

/// Generate code for evaluating a condition expression.
pub fn generate_condition_code(condition: &ConditionExpr) -> TokenStream {
    if let Some(ref parsed) = condition.parsed {
        generate_parsed_condition_code(parsed)
    } else {
        // Fall back to string-based expression parsing
        let _expr = &condition.expression;
        // This is a simplified version - in practice you'd want to properly parse the expression
        quote! { true /* condition: #_expr */ }
    }
}

/// Generate code for a parsed condition.
pub fn generate_parsed_condition_code(condition: &ParsedCondition) -> TokenStream {
    match condition {
        ParsedCondition::Comparison { field, op, value } => {
            let field_str = field.segments.last().cloned().unwrap_or_default();
            let value_str = serde_json::to_string(value).unwrap_or_else(|_| "null".to_string());

            // For numeric comparisons, extract the value as appropriate type
            match op {
                ComparisonOp::Equal | ComparisonOp::NotEqual => {
                    let op_code = match op {
                        ComparisonOp::Equal => quote! { == },
                        ComparisonOp::NotEqual => quote! { != },
                        _ => unreachable!(),
                    };
                    quote! {
                        ctx.data::<hyperstack::runtime::serde_json::Value>(#field_str)
                            .map(|v| v #op_code hyperstack::runtime::serde_json::from_str::<hyperstack::runtime::serde_json::Value>(#value_str).unwrap_or(hyperstack::runtime::serde_json::Value::Null))
                            .unwrap_or(false)
                    }
                }
                ComparisonOp::GreaterThan
                | ComparisonOp::GreaterThanOrEqual
                | ComparisonOp::LessThan
                | ComparisonOp::LessThanOrEqual => {
                    // For ordering comparisons, try to compare as numbers
                    let op_code = match op {
                        ComparisonOp::GreaterThan => quote! { > },
                        ComparisonOp::GreaterThanOrEqual => quote! { >= },
                        ComparisonOp::LessThan => quote! { < },
                        ComparisonOp::LessThanOrEqual => quote! { <= },
                        _ => unreachable!(),
                    };
                    quote! {
                        {
                            let field_val: Option<i64> = ctx.data(#field_str);
                            let compare_val: Option<i64> = hyperstack::runtime::serde_json::from_str(#value_str).ok();
                            match (field_val, compare_val) {
                                (Some(f), Some(c)) => f #op_code c,
                                _ => false
                            }
                        }
                    }
                }
            }
        }
        ParsedCondition::Logical { op, conditions } => {
            let condition_codes: Vec<TokenStream> = conditions
                .iter()
                .map(generate_parsed_condition_code)
                .collect();
            match op {
                LogicalOp::And => quote! { #(#condition_codes)&&* },
                LogicalOp::Or => quote! { #(#condition_codes)||* },
            }
        }
    }
}

/// Convert a string to snake_case.
pub fn to_snake_case(s: &str) -> String {
    s.split("::")
        .last()
        .unwrap_or(s)
        .chars()
        .enumerate()
        .flat_map(|(i, c)| {
            if c.is_uppercase() && i > 0 {
                vec!['_', c.to_lowercase().next().unwrap()]
            } else {
                vec![c.to_lowercase().next().unwrap()]
            }
        })
        .collect()
}
