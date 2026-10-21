//! Computed field evaluator generation.
//!
//! Generates the `evaluate_computed_fields` function that evaluates
//! computed expressions at runtime.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use crate::ast::{BinaryOp, ComputedExpr, ComputedFieldSpec, UnaryOp};

fn generate_json_array_sum(inner: TokenStream) -> TokenStream {
    quote! {
        {
            let inner_result = #inner;
            let sum_result: Option<arete::runtime::serde_json::Value> = inner_result.and_then(|value| {
                let values = value.as_array()?;

                let unsigned_values = values
                    .iter()
                    .map(|item| {
                        item.as_u64()
                            .or_else(|| item.as_str().and_then(|text| text.parse::<u64>().ok()))
                    })
                    .collect::<Option<Vec<_>>>();
                if let Some(unsigned_values) = unsigned_values {
                    return unsigned_values
                        .iter()
                        .try_fold(0_u64, |total, item| total.checked_add(*item))
                        .map(arete::runtime::serde_json::Value::from);
                }

                let signed_values = values
                    .iter()
                    .map(|item| {
                        item.as_i64()
                            .or_else(|| item.as_str().and_then(|text| text.parse::<i64>().ok()))
                    })
                    .collect::<Option<Vec<_>>>();
                if let Some(signed_values) = signed_values {
                    return signed_values
                        .iter()
                        .try_fold(0_i64, |total, item| total.checked_add(*item))
                        .map(arete::runtime::serde_json::Value::from);
                }

                let total = values
                    .iter()
                    .try_fold(0.0_f64, |total, item| {
                        item.as_f64()
                            .or_else(|| item.as_str().and_then(|text| text.parse::<f64>().ok()))
                            .map(|value| total + value)
                    })?;
                if !total.is_finite() {
                    return None;
                }
                arete::runtime::serde_json::Number::from_f64(total)
                    .map(arete::runtime::serde_json::Value::Number)
            });
            sum_result
        }
    }
}

/// Every field path a computed expression reads. Literal values are data,
/// not code, so nothing inside them is a read.
fn collect_field_refs(expr: &ComputedExpr, paths: &mut Vec<String>) {
    match expr {
        ComputedExpr::FieldRef { path } => paths.push(path.clone()),
        ComputedExpr::UnwrapOr { expr, .. }
        | ComputedExpr::Cast { expr, .. }
        | ComputedExpr::Paren { expr }
        | ComputedExpr::Slice { expr, .. }
        | ComputedExpr::Index { expr, .. }
        | ComputedExpr::Unary { expr, .. }
        | ComputedExpr::JsonToBytes { expr }
        | ComputedExpr::Keccak256 { expr }
        | ComputedExpr::Some { value: expr }
        | ComputedExpr::U64FromLeBytes { bytes: expr }
        | ComputedExpr::U64FromBeBytes { bytes: expr }
        | ComputedExpr::Closure { body: expr, .. } => collect_field_refs(expr, paths),
        ComputedExpr::Binary { left, right, .. } => {
            collect_field_refs(left, paths);
            collect_field_refs(right, paths);
        }
        ComputedExpr::Let { value, body, .. } => {
            collect_field_refs(value, paths);
            collect_field_refs(body, paths);
        }
        ComputedExpr::If {
            condition,
            then_branch,
            else_branch,
        } => {
            collect_field_refs(condition, paths);
            collect_field_refs(then_branch, paths);
            collect_field_refs(else_branch, paths);
        }
        ComputedExpr::MethodCall { expr, args, .. } => {
            collect_field_refs(expr, paths);
            args.iter().for_each(|arg| collect_field_refs(arg, paths));
        }
        ComputedExpr::ResolverComputed { args, .. } => {
            args.iter().for_each(|arg| collect_field_refs(arg, paths));
        }
        ComputedExpr::Literal { .. }
        | ComputedExpr::Var { .. }
        | ComputedExpr::None
        | ComputedExpr::ByteArray { .. }
        | ComputedExpr::ContextSlot
        | ComputedExpr::ContextTimestamp => {}
    }
}

/// Order computed fields so each runs after every computed field it reads,
/// across sections: the runtime evaluates them in this order, once per
/// update. Fields with no dependency between them keep declaration order. A
/// field that reads itself, directly or through others, is an error: it
/// would read the previous update's value. Matches the Stack Source
/// compiler.
pub fn order_computed_specs(
    specs: &[ComputedFieldSpec],
) -> Result<Vec<&ComputedFieldSpec>, String> {
    let reads = |path: &str, target: &str| {
        path == target
            || path
                .strip_prefix(target)
                .is_some_and(|rest| rest.starts_with('.'))
            || target
                .strip_prefix(path)
                .is_some_and(|rest| rest.starts_with('.'))
    };
    let dependencies = specs
        .iter()
        .map(|spec| {
            let mut paths = Vec::new();
            collect_field_refs(&spec.expression, &mut paths);
            specs
                .iter()
                .enumerate()
                .filter(|(_, other)| paths.iter().any(|path| reads(path, &other.target_path)))
                .map(|(index, _)| index)
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let mut placed = vec![false; specs.len()];
    let mut order = Vec::with_capacity(specs.len());
    while order.len() < specs.len() {
        let next = (0..specs.len()).find(|&index| {
            !placed[index]
                && dependencies[index]
                    .iter()
                    .all(|&dependency| placed[dependency])
        });
        let Some(next) = next else {
            let cycle = (0..specs.len())
                .filter(|&index| !placed[index])
                .map(|index| format!("'{}'", specs[index].target_path))
                .collect::<Vec<_>>()
                .join(", ");
            return Err(format!(
                "computed fields {cycle} depend on themselves; a computed field cannot read its own value"
            ));
        };
        placed[next] = true;
        order.push(&specs[next]);
    }
    Ok(order)
}

fn contains_resolver_computed(expr: &ComputedExpr) -> bool {
    match expr {
        ComputedExpr::ResolverComputed { .. } => true,
        ComputedExpr::FieldRef { .. }
        | ComputedExpr::Literal { .. }
        | ComputedExpr::None
        | ComputedExpr::Var { .. }
        | ComputedExpr::ByteArray { .. }
        | ComputedExpr::ContextSlot
        | ComputedExpr::ContextTimestamp => false,
        ComputedExpr::UnwrapOr { expr, .. }
        | ComputedExpr::Cast { expr, .. }
        | ComputedExpr::Paren { expr }
        | ComputedExpr::Some { value: expr }
        | ComputedExpr::Slice { expr, .. }
        | ComputedExpr::Index { expr, .. }
        | ComputedExpr::U64FromLeBytes { bytes: expr }
        | ComputedExpr::U64FromBeBytes { bytes: expr }
        | ComputedExpr::JsonToBytes { expr }
        | ComputedExpr::Keccak256 { expr }
        | ComputedExpr::Unary { expr, .. } => contains_resolver_computed(expr),
        ComputedExpr::Binary { left, right, .. } => {
            contains_resolver_computed(left) || contains_resolver_computed(right)
        }
        ComputedExpr::MethodCall { expr, args, .. } => {
            contains_resolver_computed(expr) || args.iter().any(contains_resolver_computed)
        }
        ComputedExpr::Let { value, body, .. } => {
            contains_resolver_computed(value) || contains_resolver_computed(body)
        }
        ComputedExpr::If {
            condition,
            then_branch,
            else_branch,
        } => {
            contains_resolver_computed(condition)
                || contains_resolver_computed(then_branch)
                || contains_resolver_computed(else_branch)
        }
        ComputedExpr::Closure { body, .. } => contains_resolver_computed(body),
    }
}

/// Generate computed field evaluator from computed field specs.
pub fn generate_computed_evaluator(computed_field_specs: &[ComputedFieldSpec]) -> TokenStream {
    if computed_field_specs.is_empty() {
        return quote! {
            /// No-op evaluate_computed_fields (no computed fields defined)
            pub fn evaluate_computed_fields(
                _state: &mut arete::runtime::serde_json::Value,
                _context_slot: Option<u64>,
                _context_timestamp: i64,
            ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
                Ok(())
            }

            /// Returns the list of computed field paths (section.field format)
            pub fn computed_field_paths() -> &'static [&'static str] {
                &[]
            }
        };
    }

    // One statement per field, in dependency order: each is computed and
    // written before any field that reads it.
    let ordered = match order_computed_specs(computed_field_specs) {
        Ok(ordered) => ordered,
        Err(message) => return quote! { compile_error!(#message); },
    };
    let field_evals: Vec<TokenStream> = ordered
        .iter()
        .filter_map(|spec| {
            let (section, field_name) = spec.target_path.split_once('.')?;
            let field_name = field_name.rsplit('.').next().unwrap_or(field_name);
            let expr_code = generate_computed_expr_code(&spec.expression);
            Some(quote! {
                {
                    let computed_value = { #expr_code };
                    if let Some(section_value) = state.get_mut(#section) {
                        if let Some(section_obj) = section_value.as_object_mut() {
                            section_obj.insert(#field_name.to_string(), arete::runtime::serde_json::to_value(computed_value)?);
                        }
                    }
                }
            })
        })
        .collect();

    // Generate the list of computed field paths for the helper function
    let computed_paths: Vec<String> = computed_field_specs
        .iter()
        .map(|spec| spec.target_path.clone())
        .collect();
    let computed_paths_strs: Vec<&str> = computed_paths.iter().map(|s| s.as_str()).collect();

    quote! {
        /// Evaluate all computed fields for the entity state
        pub fn evaluate_computed_fields(
            state: &mut arete::runtime::serde_json::Value,
            __context_slot: Option<u64>,
            __context_timestamp: i64,
        ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            #(#field_evals)*
            Ok(())
        }

        /// Returns the list of computed field paths (section.field format)
        pub fn computed_field_paths() -> &'static [&'static str] {
            &[#(#computed_paths_strs),*]
        }
    }
}

/// Generate code for a computed expression.
fn generate_option_safe_expr_code(expr: &ComputedExpr) -> TokenStream {
    match expr {
        ComputedExpr::ResolverComputed {
            resolver,
            method,
            args,
        } => {
            let resolver_name = resolver.as_str();
            let method_name = method.as_str();
            let arg_codes: Vec<TokenStream> =
                args.iter().map(generate_option_safe_expr_code).collect();
            quote! {
                {
                    let resolver_args = vec![#(arete::runtime::serde_json::to_value(#arg_codes).ok()?),*];
                    let resolver_value = arete::runtime::arete_interpreter::resolvers::evaluate_resolver_computed(
                        #resolver_name,
                        #method_name,
                        &resolver_args,
                    ).ok()?;
                    if resolver_value.is_null() {
                        None
                    } else {
                        Some(resolver_value)
                    }
                }
            }
        }
        ComputedExpr::Var { name } => {
            let name_ident = format_ident!("{}", name);
            quote! { #name_ident }
        }
        ComputedExpr::Literal { value } => match value {
            serde_json::Value::Number(n) => {
                if let Some(u) = n.as_u64() {
                    quote! { #u }
                } else if let Some(i) = n.as_i64() {
                    quote! { #i }
                } else {
                    let num = n.as_f64().unwrap_or(0.0);
                    quote! { #num }
                }
            }
            serde_json::Value::Bool(b) => quote! { #b },
            serde_json::Value::Null => quote! { () },
            _ => quote! { 0.0_f64 },
        },
        _ => generate_computed_expr_code(expr),
    }
}

///
/// This generates code that extracts values from JSON and performs calculations.
/// The code generation handles multiple types including f64, u64, Option<T>, byte arrays, etc.
///
/// The generated code expects a `state` variable in scope that is `&serde_json::Value`.
pub fn generate_computed_expr_code(expr: &ComputedExpr) -> TokenStream {
    match expr {
        ComputedExpr::FieldRef { path } => {
            let parts: Vec<&str> = path.split('.').collect();
            if parts.len() >= 2 {
                let section = parts[0];
                let mut chain = quote! { state.get(#section) };

                for field in &parts[1..] {
                    chain = quote! { #chain.and_then(|s| s.get(#field)) };
                }

                quote! { #chain.cloned() }
            } else if parts.len() == 1 {
                let ident = format_ident!("{}", parts[0]);
                quote! { #ident }
            } else {
                quote! { None::<serde_json::Value> }
            }
        }
        ComputedExpr::UnwrapOr { expr, default } => {
            let inner = generate_computed_expr_code(expr);
            let default_num = match default {
                serde_json::Value::Number(n) => {
                    if let Some(i) = n.as_i64() {
                        quote! { #i }
                    } else if let Some(u) = n.as_u64() {
                        quote! { #u }
                    } else {
                        let f = n.as_f64().unwrap_or(0.0);
                        quote! { #f }
                    }
                }
                serde_json::Value::Bool(b) => {
                    if *b {
                        quote! { 1i64 }
                    } else {
                        quote! { 0i64 }
                    }
                }
                _ => quote! { 0i64 },
            };
            quote! {
                #inner.and_then(|v| {
                    v.as_i64()
                        .or_else(|| v.as_u64().map(|u| u as i64))
                        .or_else(|| v.as_str().and_then(|text| text.parse::<i64>().ok()))
                }).unwrap_or(#default_num)
            }
        }
        ComputedExpr::Binary { op, left, right } => {
            let left_code = generate_computed_expr_code(left);
            let right_code = generate_computed_expr_code(right);
            let op_code = match op {
                BinaryOp::Add => quote! { + },
                BinaryOp::Sub => quote! { - },
                BinaryOp::Mul => quote! { * },
                BinaryOp::Div => quote! { / },
                BinaryOp::Mod => quote! { % },
                BinaryOp::Gt => quote! { > },
                BinaryOp::Lt => quote! { < },
                BinaryOp::Gte => quote! { >= },
                BinaryOp::Lte => quote! { <= },
                BinaryOp::Eq => quote! { == },
                BinaryOp::Ne => quote! { != },
                BinaryOp::And => quote! { && },
                BinaryOp::Or => quote! { || },
                BinaryOp::Xor => quote! { ^ },
                BinaryOp::BitAnd => quote! { & },
                BinaryOp::BitOr => quote! { | },
                BinaryOp::Shl => quote! { << },
                BinaryOp::Shr => quote! { >> },
            };
            quote! { (#left_code #op_code #right_code) }
        }
        ComputedExpr::Cast { expr, to_type } => {
            let type_ident = format_ident!("{}", to_type);

            // Special handling for .max() calls being cast to f64
            // This avoids type ambiguity: (expr.max(1) as f64) should generate (expr.max(1) as f64)
            // but we need to ensure the max argument is also properly typed for the target type
            if to_type == "f64" {
                if let ComputedExpr::MethodCall {
                    expr: inner_expr,
                    method,
                    args,
                } = expr.as_ref()
                {
                    if method == "max" && args.len() == 1 {
                        // Generate the inner expression and max call with explicit f64 cast on argument
                        let inner_code = generate_computed_expr_code(inner_expr);
                        let arg_code = generate_computed_expr_code(&args[0]);
                        return quote! { ((#inner_code as f64).max(#arg_code as f64)) };
                    }
                }
            }

            let inner = generate_computed_expr_code(expr);
            quote! { (#inner as #type_ident) }
        }
        ComputedExpr::MethodCall { expr, method, args } => {
            let inner = generate_computed_expr_code(expr);
            let method_ident = format_ident!("{}", method);
            let arg_codes: Vec<TokenStream> =
                args.iter().map(generate_computed_expr_code).collect();

            if method == "sum" && args.is_empty() {
                return generate_json_array_sum(inner);
            }

            if method == "map" && args.len() == 1 {
                if let ComputedExpr::Closure { param, body } = &args[0] {
                    let param_ident = format_ident!("{}", param);

                    if contains_resolver_computed(body) {
                        // For closures with resolver calls, pass the element as JSON value
                        // and let the resolver extract what it needs
                        let body_code = generate_option_safe_expr_code(body);
                        return quote! {
                            #inner.as_ref().and_then(|v| {
                                v.as_array().map(|arr| {
                                    arr.iter()
                                        .filter_map(|elem| {
                                            // Pass element as serde_json::Value for resolvers
                                            let #param_ident = elem;
                                            #body_code
                                        })
                                        .collect::<Vec<_>>()
                                })
                            })
                        };
                    }

                    let body_code = generate_computed_expr_code(body);
                    return quote! {
                        #inner.and_then(|v| v.as_u64()).map(|#param_ident| #body_code)
                    };
                }
            }

            // Special handling for .max() to avoid type ambiguity when expr is a cast
            // If the expr is a Cast to f64, we need to ensure max arguments are also f64
            if method == "max" && args.len() == 1 {
                if let ComputedExpr::Cast { to_type, .. } = expr.as_ref() {
                    if to_type == "f64" {
                        // Cast the argument to f64 as well
                        let arg = &arg_codes[0];
                        return quote! { #inner.max(#arg as f64) };
                    }
                }
            }

            quote! { #inner.#method_ident(#(#arg_codes),*) }
        }
        ComputedExpr::ResolverComputed {
            resolver,
            method,
            args,
        } => {
            let resolver_name = resolver.as_str();
            let method_name = method.as_str();
            let arg_codes: Vec<TokenStream> =
                args.iter().map(generate_computed_expr_code).collect();
            // Wrap in an immediately-invoked closure returning Option so that
            // .ok()? works regardless of whether the surrounding context returns
            // Result or Option.
            quote! {
                (|| -> Option<arete::runtime::serde_json::Value> {
                    let resolver_args = vec![#(arete::runtime::serde_json::to_value(#arg_codes).ok()?),*];
                    let resolver_value = arete::runtime::arete_interpreter::resolvers::evaluate_resolver_computed(
                        #resolver_name,
                        #method_name,
                        &resolver_args,
                    ).ok()?;
                    if resolver_value.is_null() {
                        None
                    } else {
                        Some(resolver_value)
                    }
                })()
            }
        }
        ComputedExpr::Literal { value } => {
            // Convert the JSON value to a literal
            // Use u64 for non-negative integers to be compatible with the computed field types
            match value {
                serde_json::Value::Number(n) => {
                    if let Some(u) = n.as_u64() {
                        quote! { #u }
                    } else if let Some(i) = n.as_i64() {
                        quote! { #i }
                    } else {
                        let num = n.as_f64().unwrap_or(0.0);
                        quote! { #num }
                    }
                }
                serde_json::Value::Bool(b) => {
                    quote! { #b }
                }
                serde_json::Value::Null => {
                    quote! { () }
                }
                _ => quote! { 0.0_f64 },
            }
        }
        ComputedExpr::Paren { expr } => {
            let inner = generate_computed_expr_code(expr);
            quote! { (#inner) }
        }
        // New expression types
        ComputedExpr::Var { name } => {
            let name_ident = format_ident!("{}", name);
            quote! { #name_ident }
        }
        ComputedExpr::Let { name, value, body } => {
            let name_ident = format_ident!("{}", name);
            let value_code = generate_computed_expr_code(value);
            let body_code = generate_computed_expr_code(body);
            quote! {
                {
                    let #name_ident = #value_code;
                    #body_code
                }
            }
        }
        ComputedExpr::If {
            condition,
            then_branch,
            else_branch,
        } => {
            let cond_code = generate_computed_expr_code(condition);
            let then_code = generate_computed_expr_code(then_branch);
            let else_code = generate_computed_expr_code(else_branch);
            quote! {
                if #cond_code {
                    #then_code
                } else {
                    #else_code
                }
            }
        }
        ComputedExpr::None => {
            quote! { None }
        }
        ComputedExpr::Some { value } => {
            let inner = generate_computed_expr_code(value);
            quote! { Some(#inner) }
        }
        ComputedExpr::Slice { expr, start, end } => {
            let inner = generate_computed_expr_code(expr);
            quote! { &#inner[#start..#end] }
        }
        ComputedExpr::Index { expr, index } => {
            let inner = generate_computed_expr_code(expr);
            quote! { #inner[#index] }
        }
        ComputedExpr::U64FromLeBytes { bytes } => {
            let bytes_code = generate_computed_expr_code(bytes);
            quote! {
                {
                    let slice = #bytes_code;
                    let arr: [u8; 8] = slice.try_into().unwrap_or([0u8; 8]);
                    u64::from_le_bytes(arr)
                }
            }
        }
        ComputedExpr::U64FromBeBytes { bytes } => {
            let bytes_code = generate_computed_expr_code(bytes);
            quote! {
                {
                    let slice = #bytes_code;
                    let arr: [u8; 8] = slice.try_into().unwrap_or([0u8; 8]);
                    u64::from_be_bytes(arr)
                }
            }
        }
        ComputedExpr::ByteArray { bytes } => {
            let byte_literals: Vec<proc_macro2::TokenStream> = bytes
                .iter()
                .map(|b| {
                    quote! { #b }
                })
                .collect();
            quote! { [#(#byte_literals),*] }
        }
        ComputedExpr::Closure { param, body } => {
            let param_ident = format_ident!("{}", param);
            let body_code = generate_computed_expr_code(body);
            quote! { |#param_ident| #body_code }
        }
        ComputedExpr::Unary { op, expr } => {
            let inner = generate_computed_expr_code(expr);
            match op {
                UnaryOp::Not => quote! { !#inner },
                UnaryOp::ReverseBits => quote! { #inner.reverse_bits() },
            }
        }
        ComputedExpr::JsonToBytes { expr } => {
            let inner = generate_computed_expr_code(expr);
            // Convert Option<serde_json::Value> containing a JSON array to Vec<u8>
            quote! {
                {
                    #inner.and_then(|v| {
                        v.as_array().map(|arr| {
                            arr.iter()
                                .filter_map(|x| x.as_u64().map(|n| n as u8))
                                .collect::<Vec<u8>>()
                        })
                    }).unwrap_or_default()
                }
            }
        }
        ComputedExpr::ContextSlot => {
            // __context_slot is Option<u64>, unwrap to u64 with 0 as default
            quote! { __context_slot.unwrap_or(0) }
        }
        ComputedExpr::ContextTimestamp => {
            // __context_timestamp is i64, use directly
            quote! { __context_timestamp }
        }
        ComputedExpr::Keccak256 { expr } => {
            let inner = generate_computed_expr_code(expr);
            quote! {
                {
                    use arete::runtime::sha3::{Digest, Keccak256};
                    let bytes = #inner;
                    let hash = Keccak256::digest(&bytes);
                    hash.to_vec()
                }
            }
        }
    }
}

/// Generate code for a computed expression that uses a local cache for intra-section field references.
/// This ensures dependent computed fields see the newly computed values within the same evaluation cycle.
pub fn generate_computed_expr_code_with_cache(
    expr: &ComputedExpr,
    section: &str,
    computed_field_names: &[String],
) -> TokenStream {
    match expr {
        ComputedExpr::FieldRef { path } => {
            let parts: Vec<&str> = path.split('.').collect();
            if parts.len() >= 2 && parts[0] == section {
                // This is a field reference within the same section
                let field_name = parts[1];
                if computed_field_names.contains(&field_name.to_string()) {
                    // It's a computed field - read from cache to get the latest value
                    let remaining_path: Vec<&str> = parts[2..].to_vec();
                    if remaining_path.is_empty() {
                        return quote! {
                            computed_cache.get(#field_name).cloned()
                        };
                    } else {
                        let path_tokens: Vec<_> = remaining_path
                            .iter()
                            .map(|p| quote! { .and_then(|v| v.get(#p)) })
                            .collect();
                        return quote! {
                            computed_cache.get(#field_name).cloned()#(#path_tokens)*
                        };
                    }
                }
            }
            // Not a computed field in this section - delegate to original implementation
            generate_computed_expr_code(expr)
        }
        ComputedExpr::Binary { op, left, right } => {
            let left_code =
                generate_computed_expr_code_with_cache(left, section, computed_field_names);
            let right_code =
                generate_computed_expr_code_with_cache(right, section, computed_field_names);
            let op_code = match op {
                BinaryOp::Add => quote! { + },
                BinaryOp::Sub => quote! { - },
                BinaryOp::Mul => quote! { * },
                BinaryOp::Div => quote! { / },
                BinaryOp::Mod => quote! { % },
                BinaryOp::Gt => quote! { > },
                BinaryOp::Lt => quote! { < },
                BinaryOp::Gte => quote! { >= },
                BinaryOp::Lte => quote! { <= },
                BinaryOp::Eq => quote! { == },
                BinaryOp::Ne => quote! { != },
                BinaryOp::And => quote! { && },
                BinaryOp::Or => quote! { || },
                BinaryOp::Xor => quote! { ^ },
                BinaryOp::BitAnd => quote! { & },
                BinaryOp::BitOr => quote! { | },
                BinaryOp::Shl => quote! { << },
                BinaryOp::Shr => quote! { >> },
            };
            quote! { (#left_code #op_code #right_code) }
        }
        ComputedExpr::MethodCall {
            expr: inner,
            method,
            args,
        } => {
            let inner_code =
                generate_computed_expr_code_with_cache(inner, section, computed_field_names);
            let method_ident = format_ident!("{}", method);

            if method == "sum" && args.is_empty() {
                return generate_json_array_sum(inner_code);
            }

            // Special handling for .map() with closures - check if closure body references computed fields
            if method == "map" && args.len() == 1 {
                if let ComputedExpr::Closure { param, body } = &args[0] {
                    let param_ident = format_ident!("{}", param);
                    // Check if the closure body references any computed fields in this section
                    let mut body_paths = Vec::new();
                    collect_field_refs(body, &mut body_paths);
                    let has_computed_deps: Vec<&str> = body_paths
                        .iter()
                        .filter_map(|path| path.strip_prefix(section)?.strip_prefix('.'))
                        .filter_map(|rest| rest.split('.').next())
                        .filter(|name| computed_field_names.iter().any(|field| field == name))
                        .collect();

                    // Closures in .map() return Option, so use option-safe code generation
                    let body_code = if has_computed_deps.is_empty() {
                        generate_option_safe_expr_code(body)
                    } else {
                        generate_computed_expr_code_with_cache_option_safe(
                            body,
                            section,
                            computed_field_names,
                        )
                    };

                    return quote! {
                        {
                            let inner_result = #inner_code;
                            // Handle both arrays (Vec<u64>) and single values (u64)
                            let map_result: Option<arete::runtime::serde_json::Value> = inner_result.and_then(|v| {
                                if let Some(arr) = v.as_array() {
                                    // Handle array: map over each element
                                    let mapped: Vec<_> = arr
                                        .iter()
                                        .filter_map(|elem| {
                                            elem.as_u64()
                                                .or_else(|| elem.as_str().and_then(|text| text.parse::<u64>().ok()))
                                                .and_then(|#param_ident| {
                                                    let result = #body_code;
                                                    arete::runtime::serde_json::to_value(result).ok()
                                                })
                                        })
                                        .collect();
                                    arete::runtime::serde_json::to_value(mapped).ok()
                                } else {
                                    // Handle single value
                                    v.as_u64()
                                        .or_else(|| v.as_str().and_then(|text| text.parse::<u64>().ok()))
                                        .and_then(|#param_ident| {
                                            let result = #body_code;
                                            arete::runtime::serde_json::to_value(result).ok()
                                        })
                                }
                            });
                            map_result
                        }
                    };
                }
            }

            let arg_codes: Vec<TokenStream> = args
                .iter()
                .map(|arg| {
                    generate_computed_expr_code_with_cache(arg, section, computed_field_names)
                })
                .collect();

            quote! { #inner_code.#method_ident(#(#arg_codes),*) }
        }
        ComputedExpr::Cast {
            expr: inner,
            to_type,
        } => {
            let inner_code =
                generate_computed_expr_code_with_cache(inner, section, computed_field_names);
            let type_ident = format_ident!("{}", to_type);
            quote! { (#inner_code as #type_ident) }
        }
        ComputedExpr::Paren { expr: inner } => {
            let inner_code =
                generate_computed_expr_code_with_cache(inner, section, computed_field_names);
            quote! { (#inner_code) }
        }
        ComputedExpr::UnwrapOr {
            expr: inner,
            default,
        } => {
            let inner_code =
                generate_computed_expr_code_with_cache(inner, section, computed_field_names);
            let default_num = match default {
                serde_json::Value::Number(n) => {
                    if let Some(i) = n.as_i64() {
                        quote! { #i }
                    } else if let Some(u) = n.as_u64() {
                        quote! { #u }
                    } else {
                        let f = n.as_f64().unwrap_or(0.0);
                        quote! { #f }
                    }
                }
                serde_json::Value::Bool(b) => {
                    if *b {
                        quote! { 1i64 }
                    } else {
                        quote! { 0i64 }
                    }
                }
                _ => quote! { 0i64 },
            };
            quote! {
                #inner_code.and_then(|v| {
                    v.as_i64()
                        .or_else(|| v.as_u64().map(|u| u as i64))
                        .or_else(|| v.as_str().and_then(|text| text.parse::<i64>().ok()))
                }).unwrap_or(#default_num)
            }
        }
        ComputedExpr::Some { value } => {
            let inner =
                generate_computed_expr_code_with_cache(value, section, computed_field_names);
            quote! { Some(#inner) }
        }
        ComputedExpr::None => {
            quote! { None }
        }
        ComputedExpr::Unary { op, expr: inner } => {
            let inner_code =
                generate_computed_expr_code_with_cache(inner, section, computed_field_names);
            match op {
                UnaryOp::Not => quote! { !#inner_code },
                UnaryOp::ReverseBits => quote! { #inner_code.reverse_bits() },
            }
        }
        ComputedExpr::Keccak256 { expr: inner } => {
            let inner_code =
                generate_computed_expr_code_with_cache(inner, section, computed_field_names);
            quote! {
                {
                    use arete::runtime::sha3::{Digest, Keccak256};
                    let bytes = #inner_code;
                    let hash = Keccak256::digest(&bytes);
                    hash.to_vec()
                }
            }
        }
        ComputedExpr::U64FromLeBytes { bytes } => {
            let bytes_code =
                generate_computed_expr_code_with_cache(bytes, section, computed_field_names);
            quote! {
                {
                    let slice = #bytes_code;
                    let arr: [u8; 8] = slice.try_into().unwrap_or([0u8; 8]);
                    u64::from_le_bytes(arr)
                }
            }
        }
        ComputedExpr::ByteArray { bytes } => {
            let byte_literals: Vec<proc_macro2::TokenStream> = bytes
                .iter()
                .map(|b| {
                    let byte_val = *b;
                    quote! { #byte_val }
                })
                .collect();
            quote! { vec![#(#byte_literals),*] }
        }
        ComputedExpr::Var { name } => {
            let name_ident = format_ident!("{}", name);
            quote! { #name_ident }
        }
        ComputedExpr::Literal { value } => match value {
            serde_json::Value::Number(n) => {
                if let Some(u) = n.as_u64() {
                    quote! { #u }
                } else if let Some(i) = n.as_i64() {
                    quote! { #i }
                } else {
                    let num = n.as_f64().unwrap_or(0.0);
                    quote! { #num }
                }
            }
            serde_json::Value::Bool(b) => {
                quote! { #b }
            }
            serde_json::Value::Null => {
                quote! { () }
            }
            _ => quote! { 0.0_f64 },
        },
        ComputedExpr::ResolverComputed {
            resolver,
            method,
            args,
        } => {
            let resolver_name = resolver.as_str();
            let method_name = method.as_str();
            // Use cache-aware codegen for args so that intra-section computed
            // field references (e.g. results.expires_at_slot_hash) read from
            // computed_cache instead of the stale section_parent_state snapshot.
            let arg_codes: Vec<TokenStream> = args
                .iter()
                .map(|arg| {
                    generate_computed_expr_code_with_cache(arg, section, computed_field_names)
                })
                .collect();
            quote! {
                (|| -> Option<arete::runtime::serde_json::Value> {
                    let resolver_args = vec![#(arete::runtime::serde_json::to_value(#arg_codes).ok()?),*];
                    let resolver_value = arete::runtime::arete_interpreter::resolvers::evaluate_resolver_computed(
                        #resolver_name,
                        #method_name,
                        &resolver_args,
                    ).ok()?;
                    if resolver_value.is_null() {
                        None
                    } else {
                        Some(resolver_value)
                    }
                })()
            }
        }
        ComputedExpr::Closure { param, body } => {
            let param_ident = format_ident!("{}", param);
            let body_code =
                generate_computed_expr_code_with_cache(body, section, computed_field_names);
            quote! { |#param_ident| #body_code }
        }
        // Fallback for any unhandled expressions
        _ => generate_computed_expr_code(expr),
    }
}

/// Option-safe version of generate_computed_expr_code_with_cache.
/// Use this when the generated code will be used inside a closure that returns Option.
fn generate_computed_expr_code_with_cache_option_safe(
    expr: &ComputedExpr,
    section: &str,
    computed_field_names: &[String],
) -> TokenStream {
    match expr {
        ComputedExpr::FieldRef { path } => {
            let parts: Vec<&str> = path.split('.').collect();
            if parts.len() >= 2 && parts[0] == section {
                let field_name = parts[1];
                if computed_field_names.contains(&field_name.to_string()) {
                    let remaining_path: Vec<&str> = parts[2..].to_vec();
                    if remaining_path.is_empty() {
                        return quote! {
                            computed_cache.get(#field_name).cloned()
                        };
                    } else {
                        let path_tokens: Vec<_> = remaining_path
                            .iter()
                            .map(|p| quote! { .and_then(|v| v.get(#p)) })
                            .collect();
                        return quote! {
                            computed_cache.get(#field_name).cloned()#(#path_tokens)*
                        };
                    }
                }
            }
            generate_option_safe_expr_code(expr)
        }
        ComputedExpr::Binary { op, left, right } => {
            let left_code = generate_computed_expr_code_with_cache_option_safe(
                left,
                section,
                computed_field_names,
            );
            let right_code = generate_computed_expr_code_with_cache_option_safe(
                right,
                section,
                computed_field_names,
            );
            let op_code = match op {
                BinaryOp::Add => quote! { + },
                BinaryOp::Sub => quote! { - },
                BinaryOp::Mul => quote! { * },
                BinaryOp::Div => quote! { / },
                BinaryOp::Mod => quote! { % },
                BinaryOp::Gt => quote! { > },
                BinaryOp::Lt => quote! { < },
                BinaryOp::Gte => quote! { >= },
                BinaryOp::Lte => quote! { <= },
                BinaryOp::Eq => quote! { == },
                BinaryOp::Ne => quote! { != },
                BinaryOp::And => quote! { && },
                BinaryOp::Or => quote! { || },
                BinaryOp::Xor => quote! { ^ },
                BinaryOp::BitAnd => quote! { & },
                BinaryOp::BitOr => quote! { | },
                BinaryOp::Shl => quote! { << },
                BinaryOp::Shr => quote! { >> },
            };
            quote! { (#left_code #op_code #right_code) }
        }
        ComputedExpr::ResolverComputed {
            resolver,
            method,
            args,
        } => {
            let resolver_name = resolver.as_str();
            let method_name = method.as_str();
            let arg_codes: Vec<TokenStream> =
                args.iter().map(generate_option_safe_expr_code).collect();
            quote! {
                {
                    let resolver_args = vec![#(arete::runtime::serde_json::to_value(#arg_codes).ok()?),*];
                    let resolver_value = arete::runtime::arete_interpreter::resolvers::evaluate_resolver_computed(
                        #resolver_name,
                        #method_name,
                        &resolver_args,
                    ).ok()?;
                    if resolver_value.is_null() {
                        None
                    } else {
                        Some(resolver_value)
                    }
                }
            }
        }
        ComputedExpr::Var { name } => {
            let name_ident = format_ident!("{}", name);
            quote! { #name_ident }
        }
        ComputedExpr::Literal { value } => match value {
            serde_json::Value::Number(n) => {
                if let Some(u) = n.as_u64() {
                    quote! { #u }
                } else if let Some(i) = n.as_i64() {
                    quote! { #i }
                } else {
                    let num = n.as_f64().unwrap_or(0.0);
                    quote! { #num }
                }
            }
            serde_json::Value::Bool(b) => quote! { #b },
            serde_json::Value::Null => quote! { () },
            _ => quote! { 0.0_f64 },
        },
        _ => generate_option_safe_expr_code(expr),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(target_path: &str, expression: serde_json::Value) -> ComputedFieldSpec {
        ComputedFieldSpec {
            target_path: target_path.to_string(),
            expression: serde_json::from_value(expression).unwrap(),
            result_type: "f64".to_string(),
        }
    }

    fn field(path: &str) -> serde_json::Value {
        serde_json::json!({"FieldRef": {"path": path}})
    }

    fn sum(left: serde_json::Value, right: serde_json::Value) -> serde_json::Value {
        serde_json::json!({"Binary": {"op": "Add", "left": left, "right": right}})
    }

    #[test]
    fn computed_specs_follow_their_dependencies_across_sections() {
        let specs = [
            spec("rates.apr", field("market.utilization")),
            spec(
                "summary.total",
                sum(field("rates.apr"), field("market.utilization")),
            ),
            spec("market.utilization", field("state.borrowed")),
            spec("state.other", field("state.borrowed")),
        ];
        let order = order_computed_specs(&specs)
            .unwrap()
            .into_iter()
            .map(|spec| spec.target_path.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            order,
            [
                "market.utilization",
                "rates.apr",
                "summary.total",
                "state.other"
            ]
        );
    }

    #[test]
    fn computed_specs_in_dependency_order_keep_declaration_order() {
        let specs = [
            spec("b.second", field("state.x")),
            spec("a.first", field("b.second")),
        ];
        let order = order_computed_specs(&specs).unwrap();
        assert_eq!(order[0].target_path, "b.second");
        assert_eq!(order[1].target_path, "a.first");
    }

    #[test]
    fn computed_specs_cannot_read_themselves() {
        let looped = [spec("a.x", field("a.y")), spec("a.y", field("a.x"))];
        let error = order_computed_specs(&looped).unwrap_err();
        assert!(
            error.contains("'a.x'") && error.contains("'a.y'"),
            "{error}"
        );
        let own = [spec(
            "a.count",
            sum(
                field("a.count"),
                serde_json::json!({"Literal": {"value": 1}}),
            ),
        )];
        assert!(order_computed_specs(&own).is_err());
        // A literal shaped like a reference reads nothing.
        let literal = [spec(
            "a.shape",
            serde_json::json!({"Literal": {"value": {"FieldRef": {"path": "a.shape"}}}}),
        )];
        assert!(order_computed_specs(&literal).is_ok());
    }
}
