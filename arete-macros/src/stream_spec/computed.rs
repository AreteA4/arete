//! Computed expression parsing for stream specs.
//!
//! This module handles parsing of `#[computed(...)]` attribute expressions
//! into a ComputedExpr AST. It implements a simplified expression parser
//! that handles common patterns:
//! - Field references: `field_name`, `section.field_name`
//! - Method calls: `expr.method(args)`
//! - Binary operators: `+`, `-`, `*`, `/`, `%`, `^`, `&`, `|`, `<<`, `>>`
//! - Comparison operators: `>`, `<`, `>=`, `<=`, `==`, `!=`
//! - Logical operators: `&&`, `||`
//! - Type casts: `expr as f64`
//! - Parenthesized expressions: `(expr)`
//! - Literals: integers, floats, byte arrays
//! - Let bindings: `let x = expr; body`
//! - Conditionals: `if cond { then } else { else }`
//! - Option constructors: `Some(expr)`, `None`
//! - Slice syntax: `expr[start..end]`
//! - Byte conversion: `u64::from_le_bytes(expr)`
//! - Closures: `|x| body`

use std::collections::HashSet;

use crate::ast::{BinaryOp, ComputedExpr, UnaryOp};
use proc_macro2::TokenTree;

/// Parse a computed expression from a TokenStream into a ComputedExpr AST.
pub fn parse_computed_expression(tokens: &proc_macro2::TokenStream) -> ComputedExpr {
    let tokens_vec: Vec<proc_macro2::TokenTree> = tokens.clone().into_iter().collect();
    let (expr, _) = parse_expr(&tokens_vec, 0);
    // Post-process to resolve let bindings - convert FieldRefs to Vars for bound names
    resolve_bindings_in_expr(expr, &HashSet::new())
}

fn resolver_for_method(method: &str) -> Option<&'static str> {
    match method {
        "ui_amount" | "raw_amount" => Some("TokenMetadata"),
        "slot_hash" | "keccak_rng" => Some("SlotHash"),
        _ => None,
    }
}

/// Get the output type name for a resolver method.
/// This maps resolver method names to their TypeScript output type names.
pub fn resolver_output_type(method: &str) -> Option<&'static str> {
    match method {
        "slot_hash" => Some("SlotHashBytes"),
        "keccak_rng" => Some("KeccakRngValue"),
        "ui_amount" => Some("TokenUiAmount"),
        "raw_amount" => Some("TokenRawAmount"),
        _ => None,
    }
}

/// Returns true if the expression tree contains any U64FromLeBytes or U64FromBeBytes node.
/// Used to detect full-range u64 computations that must be serialized as strings in TypeScript.
pub fn expr_contains_u64_from_bytes(expr: &crate::ast::ComputedExpr) -> bool {
    use crate::ast::ComputedExpr;
    match expr {
        ComputedExpr::U64FromLeBytes { .. } | ComputedExpr::U64FromBeBytes { .. } => true,
        ComputedExpr::Some { value } => expr_contains_u64_from_bytes(value),
        ComputedExpr::Paren { expr } => expr_contains_u64_from_bytes(expr),
        ComputedExpr::Binary { left, right, .. } => {
            expr_contains_u64_from_bytes(left) || expr_contains_u64_from_bytes(right)
        }
        ComputedExpr::Unary { expr, .. } => expr_contains_u64_from_bytes(expr),
        ComputedExpr::MethodCall { expr, args, .. } => {
            expr_contains_u64_from_bytes(expr) || args.iter().any(expr_contains_u64_from_bytes)
        }
        ComputedExpr::Cast { expr, .. } => expr_contains_u64_from_bytes(expr),
        ComputedExpr::UnwrapOr { expr, .. } => expr_contains_u64_from_bytes(expr),
        ComputedExpr::Slice { expr, .. } => expr_contains_u64_from_bytes(expr),
        ComputedExpr::Index { expr, .. } => expr_contains_u64_from_bytes(expr),
        ComputedExpr::Let { value, body, .. } => {
            expr_contains_u64_from_bytes(value) || expr_contains_u64_from_bytes(body)
        }
        ComputedExpr::If {
            condition: _,
            then_branch,
            else_branch,
        } => expr_contains_u64_from_bytes(then_branch) || expr_contains_u64_from_bytes(else_branch),
        ComputedExpr::Keccak256 { expr } => expr_contains_u64_from_bytes(expr),
        ComputedExpr::JsonToBytes { expr } => expr_contains_u64_from_bytes(expr),
        ComputedExpr::Closure { body, .. } => expr_contains_u64_from_bytes(body),
        ComputedExpr::ResolverComputed { .. }
        | ComputedExpr::FieldRef { .. }
        | ComputedExpr::Var { .. }
        | ComputedExpr::Literal { .. }
        | ComputedExpr::ByteArray { .. }
        | ComputedExpr::None
        | ComputedExpr::ContextSlot
        | ComputedExpr::ContextTimestamp => false,
    }
}

/// Extract the resolver output type from a computed expression.
/// Returns the type name if the expression is a ResolverComputed call.
pub fn extract_resolver_type_from_computed_expr(
    expr: &crate::ast::ComputedExpr,
) -> Option<&'static str> {
    use crate::ast::ComputedExpr;
    match expr {
        ComputedExpr::ResolverComputed { method, .. } => resolver_output_type(method),
        ComputedExpr::Some { value } => extract_resolver_type_from_computed_expr(value),
        ComputedExpr::Paren { expr } => extract_resolver_type_from_computed_expr(expr),
        _ => None,
    }
}

/// Qualify unqualified field references in a computed expression with a section prefix.
///
/// This ensures that field references like `total_buy_volume` become `trading.total_buy_volume`
/// when the computed field is in the `trading` section.
pub fn qualify_field_refs(expr: ComputedExpr, section: &str) -> ComputedExpr {
    match expr {
        ComputedExpr::FieldRef { path } => {
            // If the path doesn't contain a dot, qualify it with the section prefix
            if !path.contains('.') {
                ComputedExpr::FieldRef {
                    path: format!("{}.{}", section, path),
                }
            } else {
                // Already qualified (e.g., reserves.virtual_sol_reserves)
                ComputedExpr::FieldRef { path }
            }
        }
        ComputedExpr::UnwrapOr { expr, default } => ComputedExpr::UnwrapOr {
            expr: Box::new(qualify_field_refs(*expr, section)),
            default,
        },
        ComputedExpr::Binary { op, left, right } => ComputedExpr::Binary {
            op,
            left: Box::new(qualify_field_refs(*left, section)),
            right: Box::new(qualify_field_refs(*right, section)),
        },
        ComputedExpr::Cast { expr, to_type } => ComputedExpr::Cast {
            expr: Box::new(qualify_field_refs(*expr, section)),
            to_type,
        },
        ComputedExpr::MethodCall { expr, method, args } => ComputedExpr::MethodCall {
            expr: Box::new(qualify_field_refs(*expr, section)),
            method,
            args: args
                .into_iter()
                .map(|a| qualify_field_refs(a, section))
                .collect(),
        },
        ComputedExpr::ResolverComputed {
            resolver,
            method,
            args,
        } => ComputedExpr::ResolverComputed {
            resolver,
            method,
            args: args
                .into_iter()
                .map(|a| qualify_field_refs(a, section))
                .collect(),
        },
        ComputedExpr::Paren { expr } => ComputedExpr::Paren {
            expr: Box::new(qualify_field_refs(*expr, section)),
        },
        ComputedExpr::Literal { value } => ComputedExpr::Literal { value },
        // New variants
        ComputedExpr::Var { name } => ComputedExpr::Var { name },
        ComputedExpr::Let { name, value, body } => ComputedExpr::Let {
            name,
            value: Box::new(qualify_field_refs(*value, section)),
            body: Box::new(qualify_field_refs(*body, section)),
        },
        ComputedExpr::If {
            condition,
            then_branch,
            else_branch,
        } => ComputedExpr::If {
            condition: Box::new(qualify_field_refs(*condition, section)),
            then_branch: Box::new(qualify_field_refs(*then_branch, section)),
            else_branch: Box::new(qualify_field_refs(*else_branch, section)),
        },
        ComputedExpr::None => ComputedExpr::None,
        ComputedExpr::Some { value } => ComputedExpr::Some {
            value: Box::new(qualify_field_refs(*value, section)),
        },
        ComputedExpr::Slice { expr, start, end } => ComputedExpr::Slice {
            expr: Box::new(qualify_field_refs(*expr, section)),
            start,
            end,
        },
        ComputedExpr::Index { expr, index } => ComputedExpr::Index {
            expr: Box::new(qualify_field_refs(*expr, section)),
            index,
        },
        ComputedExpr::U64FromLeBytes { bytes } => ComputedExpr::U64FromLeBytes {
            bytes: Box::new(qualify_field_refs(*bytes, section)),
        },
        ComputedExpr::U64FromBeBytes { bytes } => ComputedExpr::U64FromBeBytes {
            bytes: Box::new(qualify_field_refs(*bytes, section)),
        },
        ComputedExpr::ByteArray { bytes } => ComputedExpr::ByteArray { bytes },
        ComputedExpr::Closure { param, body } => ComputedExpr::Closure {
            param,
            body: Box::new(qualify_field_refs(*body, section)),
        },
        ComputedExpr::Unary { op, expr } => ComputedExpr::Unary {
            op,
            expr: Box::new(qualify_field_refs(*expr, section)),
        },
        ComputedExpr::JsonToBytes { expr } => ComputedExpr::JsonToBytes {
            expr: Box::new(qualify_field_refs(*expr, section)),
        },
        ComputedExpr::ContextSlot => ComputedExpr::ContextSlot,
        ComputedExpr::ContextTimestamp => ComputedExpr::ContextTimestamp,
        ComputedExpr::Keccak256 { expr } => ComputedExpr::Keccak256 {
            expr: Box::new(qualify_field_refs(*expr, section)),
        },
    }
}

/// Extract section references from a computed field expression.
///
/// Returns a set of section names that are referenced (e.g., "reserves" from "reserves.virtual_sol_reserves").
pub fn extract_section_references(expression: &proc_macro2::TokenStream) -> HashSet<String> {
    let mut sections = HashSet::new();
    extract_section_references_recursive(expression, &mut sections);
    sections
}

/// Recursive helper to extract section references, handling nested groups (parentheses, etc.)
fn extract_section_references_recursive(
    expression: &proc_macro2::TokenStream,
    sections: &mut HashSet<String>,
) {
    let tokens: Vec<TokenTree> = expression.clone().into_iter().collect();

    // Look for patterns like: ident "." ident where the first ident is likely a section name
    let mut i = 0;
    while i < tokens.len() {
        match &tokens[i] {
            TokenTree::Ident(ident) => {
                let ident_str = ident.to_string();

                // Check if next token is a dot
                if i + 1 < tokens.len() {
                    if let TokenTree::Punct(punct) = &tokens[i + 1] {
                        if punct.as_char() == '.' {
                            // Check if there's another identifier after the dot
                            if i + 2 < tokens.len() {
                                if let TokenTree::Ident(_field_ident) = &tokens[i + 2] {
                                    // This looks like section.field - add the section
                                    sections.insert(ident_str);
                                }
                            }
                        }
                    }
                }
            }
            TokenTree::Group(group) => {
                // Recursively search inside groups (parentheses, brackets, braces)
                extract_section_references_recursive(&group.stream(), sections);
            }
            _ => {}
        }
        i += 1;
    }
}

/// Extract all field references from a section in an expression.
///
/// Returns field names that are accessed from a specific section.
pub fn extract_field_references_from_section(
    expression: &proc_macro2::TokenStream,
    section_name: &str,
) -> HashSet<String> {
    let mut fields = HashSet::new();
    extract_field_references_recursive(expression, section_name, &mut fields);
    fields
}

/// Recursive helper to extract field references from a section, handling nested groups.
fn extract_field_references_recursive(
    expression: &proc_macro2::TokenStream,
    section_name: &str,
    fields: &mut HashSet<String>,
) {
    let tokens: Vec<TokenTree> = expression.clone().into_iter().collect();

    // Look for patterns like: section_name "." field_name
    let mut i = 0;
    while i < tokens.len() {
        match &tokens[i] {
            TokenTree::Ident(ident) if *ident == section_name => {
                // Match: section_name "." field_name
                if let (Some(TokenTree::Punct(punct)), Some(TokenTree::Ident(field_ident))) =
                    (tokens.get(i + 1), tokens.get(i + 2))
                {
                    if punct.as_char() == '.' {
                        fields.insert(field_ident.to_string());
                    }
                }
            }
            TokenTree::Group(group) => {
                // Recursively search inside groups (parentheses, brackets, braces)
                extract_field_references_recursive(&group.stream(), section_name, fields);
            }
            _ => {}
        }
        i += 1;
    }
}

// ============================================================================
// Recursive Descent Expression Parser
// ============================================================================

/// Recursive descent parser for expressions.
///
/// Grammar (simplified):
/// ```text
/// expr         = let_expr | if_expr | or_expr
/// let_expr     = "let" IDENT "=" expr ";" expr
/// if_expr      = "if" expr "{" expr "}" "else" "{" expr "}"
/// or_expr      = and_expr ("||" and_expr)*
/// and_expr     = bitor_expr ("&&" bitor_expr)*
/// bitor_expr   = xor_expr ("|" xor_expr)*
/// xor_expr     = bitand_expr ("^" bitand_expr)*
/// bitand_expr  = eq_expr ("&" eq_expr)*
/// eq_expr      = cmp_expr (("==" | "!=") cmp_expr)*
/// cmp_expr     = shift_expr (("<" | ">" | "<=" | ">=") shift_expr)*
/// shift_expr   = add_expr (("<<" | ">>") add_expr)*
/// add_expr     = mul_expr (("+" | "-") mul_expr)*
/// mul_expr     = unary_expr (("*" | "/" | "%") unary_expr)*
/// unary_expr   = "!" unary_expr | postfix_expr
/// postfix_expr = primary_expr (method_call | field_access | slice | "as" TYPE)*
/// primary_expr = "(" expr ")" | "[" byte_array "]" | LITERAL | "None" | "Some" "(" expr ")" | closure | type_fn | IDENT
/// closure      = "|" IDENT "|" expr
/// type_fn      = TYPE "::" IDENT "(" expr ")"
/// ```
fn parse_expr(tokens: &[proc_macro2::TokenTree], start: usize) -> (ComputedExpr, usize) {
    // Check for let binding: let name = value; body
    if start < tokens.len() {
        if let proc_macro2::TokenTree::Ident(ident) = &tokens[start] {
            if *ident == "let" {
                return parse_let_expr(tokens, start);
            }
            if *ident == "if" {
                return parse_if_expr(tokens, start);
            }
        }
    }
    parse_binary_expr(tokens, start, 0)
}

/// Convert FieldRefs that match bound variable names to Vars in a Let expression.
/// This is called as a post-processing step after parsing.
fn resolve_bindings_in_expr(expr: ComputedExpr, bindings: &HashSet<String>) -> ComputedExpr {
    match expr {
        ComputedExpr::FieldRef { ref path } => {
            // If the path is a simple identifier (no dots) and matches a binding, convert to Var
            if !path.contains('.') && bindings.contains(path) {
                ComputedExpr::Var { name: path.clone() }
            } else {
                expr
            }
        }
        ComputedExpr::Let { name, value, body } => {
            // Resolve bindings in the value expression
            let resolved_value = resolve_bindings_in_expr(*value, bindings);
            // Add the new binding and resolve in the body
            let mut new_bindings = bindings.clone();
            new_bindings.insert(name.clone());
            let resolved_body = resolve_bindings_in_expr(*body, &new_bindings);
            ComputedExpr::Let {
                name,
                value: Box::new(resolved_value),
                body: Box::new(resolved_body),
            }
        }
        ComputedExpr::If {
            condition,
            then_branch,
            else_branch,
        } => ComputedExpr::If {
            condition: Box::new(resolve_bindings_in_expr(*condition, bindings)),
            then_branch: Box::new(resolve_bindings_in_expr(*then_branch, bindings)),
            else_branch: Box::new(resolve_bindings_in_expr(*else_branch, bindings)),
        },
        ComputedExpr::Binary { op, left, right } => ComputedExpr::Binary {
            op,
            left: Box::new(resolve_bindings_in_expr(*left, bindings)),
            right: Box::new(resolve_bindings_in_expr(*right, bindings)),
        },
        ComputedExpr::Unary { op, expr } => ComputedExpr::Unary {
            op,
            expr: Box::new(resolve_bindings_in_expr(*expr, bindings)),
        },
        ComputedExpr::MethodCall { expr, method, args } => ComputedExpr::MethodCall {
            expr: Box::new(resolve_bindings_in_expr(*expr, bindings)),
            method,
            args: args
                .into_iter()
                .map(|a| resolve_bindings_in_expr(a, bindings))
                .collect(),
        },
        ComputedExpr::ResolverComputed {
            resolver,
            method,
            args,
        } => ComputedExpr::ResolverComputed {
            resolver,
            method,
            args: args
                .into_iter()
                .map(|a| resolve_bindings_in_expr(a, bindings))
                .collect(),
        },
        ComputedExpr::UnwrapOr { expr, default } => ComputedExpr::UnwrapOr {
            expr: Box::new(resolve_bindings_in_expr(*expr, bindings)),
            default,
        },
        ComputedExpr::Cast { expr, to_type } => ComputedExpr::Cast {
            expr: Box::new(resolve_bindings_in_expr(*expr, bindings)),
            to_type,
        },
        ComputedExpr::Paren { expr } => ComputedExpr::Paren {
            expr: Box::new(resolve_bindings_in_expr(*expr, bindings)),
        },
        ComputedExpr::Some { value } => ComputedExpr::Some {
            value: Box::new(resolve_bindings_in_expr(*value, bindings)),
        },
        ComputedExpr::Slice { expr, start, end } => ComputedExpr::Slice {
            expr: Box::new(resolve_bindings_in_expr(*expr, bindings)),
            start,
            end,
        },
        ComputedExpr::Index { expr, index } => ComputedExpr::Index {
            expr: Box::new(resolve_bindings_in_expr(*expr, bindings)),
            index,
        },
        ComputedExpr::U64FromLeBytes { bytes } => ComputedExpr::U64FromLeBytes {
            bytes: Box::new(resolve_bindings_in_expr(*bytes, bindings)),
        },
        ComputedExpr::U64FromBeBytes { bytes } => ComputedExpr::U64FromBeBytes {
            bytes: Box::new(resolve_bindings_in_expr(*bytes, bindings)),
        },
        ComputedExpr::JsonToBytes { expr } => ComputedExpr::JsonToBytes {
            expr: Box::new(resolve_bindings_in_expr(*expr, bindings)),
        },
        ComputedExpr::Keccak256 { expr } => ComputedExpr::Keccak256 {
            expr: Box::new(resolve_bindings_in_expr(*expr, bindings)),
        },
        ComputedExpr::Closure { param, body } => {
            // The closure param is also a binding
            let mut new_bindings = bindings.clone();
            new_bindings.insert(param.clone());
            ComputedExpr::Closure {
                param,
                body: Box::new(resolve_bindings_in_expr(*body, &new_bindings)),
            }
        }
        // These don't contain sub-expressions
        ComputedExpr::Var { .. }
        | ComputedExpr::None
        | ComputedExpr::Literal { .. }
        | ComputedExpr::ByteArray { .. }
        | ComputedExpr::ContextSlot
        | ComputedExpr::ContextTimestamp => expr,
    }
}

/// Parse a let expression: let name = value; body
fn parse_let_expr(tokens: &[proc_macro2::TokenTree], start: usize) -> (ComputedExpr, usize) {
    // Skip "let"
    let mut pos = start + 1;

    // Get variable name
    let name = if pos < tokens.len() {
        if let proc_macro2::TokenTree::Ident(ident) = &tokens[pos] {
            pos += 1;
            ident.to_string()
        } else {
            return (
                ComputedExpr::Literal {
                    value: serde_json::Value::Null,
                },
                pos,
            );
        }
    } else {
        return (
            ComputedExpr::Literal {
                value: serde_json::Value::Null,
            },
            pos,
        );
    };

    // Skip "="
    if pos < tokens.len() {
        if let proc_macro2::TokenTree::Punct(p) = &tokens[pos] {
            if p.as_char() == '=' {
                pos += 1;
            }
        }
    }

    // Parse value expression (until semicolon)
    let (value, new_pos) = parse_expr_until_semicolon(tokens, pos);
    pos = new_pos;

    // Skip semicolon
    if pos < tokens.len() {
        if let proc_macro2::TokenTree::Punct(p) = &tokens[pos] {
            if p.as_char() == ';' {
                pos += 1;
            }
        }
    }

    // Parse body expression
    let (body, final_pos) = parse_expr(tokens, pos);

    (
        ComputedExpr::Let {
            name,
            value: Box::new(value),
            body: Box::new(body),
        },
        final_pos,
    )
}

/// Parse expression until semicolon (for let bindings).
fn parse_expr_until_semicolon(
    tokens: &[proc_macro2::TokenTree],
    start: usize,
) -> (ComputedExpr, usize) {
    // Find the semicolon, accounting for nested braces
    let mut depth: i32 = 0;
    let mut end = start;

    while end < tokens.len() {
        match &tokens[end] {
            proc_macro2::TokenTree::Punct(p) if p.as_char() == ';' && depth == 0 => break,
            proc_macro2::TokenTree::Group(g) => {
                if g.delimiter() == proc_macro2::Delimiter::Brace {
                    depth += 1;
                }
                end += 1;
            }
            proc_macro2::TokenTree::Punct(p) if p.as_char() == '{' => {
                depth += 1;
                end += 1;
            }
            proc_macro2::TokenTree::Punct(p) if p.as_char() == '}' => {
                depth = depth.saturating_sub(1);
                end += 1;
            }
            _ => end += 1,
        }
    }

    let expr_tokens: Vec<_> = tokens[start..end].to_vec();
    let (expr, _) = parse_binary_expr(&expr_tokens, 0, 0);
    (expr, end)
}

/// Parse an if expression: if cond { then } else { else }
fn parse_if_expr(tokens: &[proc_macro2::TokenTree], start: usize) -> (ComputedExpr, usize) {
    // Skip "if"
    let mut pos = start + 1;

    // Parse condition (until opening brace)
    let mut cond_tokens = Vec::new();
    while pos < tokens.len() {
        if let proc_macro2::TokenTree::Group(g) = &tokens[pos] {
            if g.delimiter() == proc_macro2::Delimiter::Brace {
                break;
            }
        }
        cond_tokens.push(tokens[pos].clone());
        pos += 1;
    }
    let (condition, _) = parse_binary_expr(&cond_tokens, 0, 0);

    // Parse then branch (inside braces)
    let then_branch = if pos < tokens.len() {
        if let proc_macro2::TokenTree::Group(g) = &tokens[pos] {
            if g.delimiter() == proc_macro2::Delimiter::Brace {
                pos += 1;
                let inner_tokens: Vec<_> = g.stream().into_iter().collect();
                let (expr, _) = parse_expr(&inner_tokens, 0);
                expr
            } else {
                ComputedExpr::Literal {
                    value: serde_json::Value::Null,
                }
            }
        } else {
            ComputedExpr::Literal {
                value: serde_json::Value::Null,
            }
        }
    } else {
        ComputedExpr::Literal {
            value: serde_json::Value::Null,
        }
    };

    // Skip "else"
    if pos < tokens.len() {
        if let proc_macro2::TokenTree::Ident(ident) = &tokens[pos] {
            if *ident == "else" {
                pos += 1;
            }
        }
    }

    // Parse else branch (inside braces)
    let else_branch = if pos < tokens.len() {
        if let proc_macro2::TokenTree::Group(g) = &tokens[pos] {
            if g.delimiter() == proc_macro2::Delimiter::Brace {
                pos += 1;
                let inner_tokens: Vec<_> = g.stream().into_iter().collect();
                let (expr, _) = parse_expr(&inner_tokens, 0);
                expr
            } else {
                ComputedExpr::Literal {
                    value: serde_json::Value::Null,
                }
            }
        } else {
            ComputedExpr::Literal {
                value: serde_json::Value::Null,
            }
        }
    } else {
        ComputedExpr::Literal {
            value: serde_json::Value::Null,
        }
    };

    (
        ComputedExpr::If {
            condition: Box::new(condition),
            then_branch: Box::new(then_branch),
            else_branch: Box::new(else_branch),
        },
        pos,
    )
}

/// Binary expression parser with precedence climbing.
fn parse_binary_expr(
    tokens: &[proc_macro2::TokenTree],
    start: usize,
    min_prec: u8,
) -> (ComputedExpr, usize) {
    let (mut left, mut pos) = parse_unary_expr(tokens, start);

    loop {
        if pos >= tokens.len() {
            break;
        }

        // Check for binary operator
        let op_result = try_parse_binary_op(tokens, pos);
        if let Some((op, op_prec, new_pos)) = op_result {
            if op_prec < min_prec {
                break;
            }

            pos = new_pos;
            let (right, next_pos) = parse_binary_expr(tokens, pos, op_prec + 1);
            pos = next_pos;

            left = ComputedExpr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        } else {
            break;
        }
    }

    (left, pos)
}

/// Try to parse a binary operator at the given position.
///
/// Returns (operator, precedence, new position) if found.
///
/// Precedence levels (higher = tighter binding):
/// - 1: || (or)
/// - 2: && (and)
/// - 3: | (bitor)
/// - 4: ^ (xor)
/// - 5: & (bitand) - single &, not &&
/// - 6: == != (equality)
/// - 7: < > <= >= (comparison)
/// - 8: << >> (shift)
/// - 9: + - (additive)
/// - 10: * / % (multiplicative)
fn try_parse_binary_op(
    tokens: &[proc_macro2::TokenTree],
    pos: usize,
) -> Option<(BinaryOp, u8, usize)> {
    if pos >= tokens.len() {
        return None;
    }

    if let proc_macro2::TokenTree::Punct(p) = &tokens[pos] {
        let c = p.as_char();

        // Check for two-character operators
        if pos + 1 < tokens.len() {
            if let proc_macro2::TokenTree::Punct(p2) = &tokens[pos + 1] {
                let c2 = p2.as_char();
                match (c, c2) {
                    ('=', '=') => return Some((BinaryOp::Eq, 6, pos + 2)),
                    ('!', '=') => return Some((BinaryOp::Ne, 6, pos + 2)),
                    ('>', '=') => return Some((BinaryOp::Gte, 7, pos + 2)),
                    ('<', '=') => return Some((BinaryOp::Lte, 7, pos + 2)),
                    ('&', '&') => return Some((BinaryOp::And, 2, pos + 2)),
                    ('|', '|') => return Some((BinaryOp::Or, 1, pos + 2)),
                    ('<', '<') => return Some((BinaryOp::Shl, 8, pos + 2)),
                    ('>', '>') => return Some((BinaryOp::Shr, 8, pos + 2)),
                    _ => {}
                }
            }
        }

        // Single-character operators
        // Note: We need to be careful with & and | to not match && and ||
        // The check above already handles &&/||, so if we get here with & or |, it's a single one
        match c {
            '+' => return Some((BinaryOp::Add, 9, pos + 1)),
            '-' => return Some((BinaryOp::Sub, 9, pos + 1)),
            '*' => return Some((BinaryOp::Mul, 10, pos + 1)),
            '/' => return Some((BinaryOp::Div, 10, pos + 1)),
            '%' => return Some((BinaryOp::Mod, 10, pos + 1)),
            '>' => return Some((BinaryOp::Gt, 7, pos + 1)),
            '<' => return Some((BinaryOp::Lt, 7, pos + 1)),
            '^' => return Some((BinaryOp::Xor, 4, pos + 1)),
            '&' => {
                // Only match single & (bitand), not && (logical and)
                if pos + 1 < tokens.len() {
                    if let proc_macro2::TokenTree::Punct(p2) = &tokens[pos + 1] {
                        if p2.as_char() == '&' {
                            return None; // Let the && case above handle it
                        }
                    }
                }
                return Some((BinaryOp::BitAnd, 5, pos + 1));
            }
            '|' => {
                // Only match single | (bitor), not || (logical or)
                if pos + 1 < tokens.len() {
                    if let proc_macro2::TokenTree::Punct(p2) = &tokens[pos + 1] {
                        if p2.as_char() == '|' {
                            return None; // Let the || case above handle it
                        }
                    }
                }
                return Some((BinaryOp::BitOr, 3, pos + 1));
            }
            _ => {}
        }
    }

    None
}

/// Parse a unary expression.
fn parse_unary_expr(tokens: &[proc_macro2::TokenTree], start: usize) -> (ComputedExpr, usize) {
    if start >= tokens.len() {
        return (
            ComputedExpr::Literal {
                value: serde_json::Value::Null,
            },
            start,
        );
    }

    // Check for unary ! operator
    if let proc_macro2::TokenTree::Punct(p) = &tokens[start] {
        if p.as_char() == '!' {
            let (inner, pos) = parse_unary_expr(tokens, start + 1);
            return (
                ComputedExpr::Unary {
                    op: UnaryOp::Not,
                    expr: Box::new(inner),
                },
                pos,
            );
        }
    }

    parse_postfix_expr(tokens, start)
}

/// Parse a postfix expression (field access, method calls, type casts, slices).
fn parse_postfix_expr(tokens: &[proc_macro2::TokenTree], start: usize) -> (ComputedExpr, usize) {
    let (mut expr, mut pos) = parse_primary_expr(tokens, start);

    loop {
        if pos >= tokens.len() {
            break;
        }

        // Check for bracket (slice or index access): expr[start..end] or expr[index]
        if let proc_macro2::TokenTree::Group(group) = &tokens[pos] {
            if group.delimiter() == proc_macro2::Delimiter::Bracket {
                pos += 1;
                let inner_tokens: Vec<_> = group.stream().into_iter().collect();

                // Check if it's a range (contains ..)
                let mut is_range = false;
                let mut dot_dot_pos = None;
                for (i, token) in inner_tokens.iter().enumerate() {
                    if let proc_macro2::TokenTree::Punct(p) = token {
                        if p.as_char() == '.' && i + 1 < inner_tokens.len() {
                            if let proc_macro2::TokenTree::Punct(p2) = &inner_tokens[i + 1] {
                                if p2.as_char() == '.' {
                                    is_range = true;
                                    dot_dot_pos = Some(i);
                                    break;
                                }
                            }
                        }
                    }
                }

                if is_range {
                    // Parse start..end
                    let dot_pos = dot_dot_pos.unwrap();
                    let start_tokens: Vec<_> = inner_tokens[..dot_pos].to_vec();
                    let end_tokens: Vec<_> = inner_tokens[dot_pos + 2..].to_vec();

                    let start_val = if start_tokens.is_empty() {
                        0
                    } else {
                        parse_usize_literal(&start_tokens)
                    };

                    let end_val = if end_tokens.is_empty() {
                        usize::MAX
                    } else {
                        parse_usize_literal(&end_tokens)
                    };

                    expr = ComputedExpr::Slice {
                        expr: Box::new(expr),
                        start: start_val,
                        end: end_val,
                    };
                    continue;
                } else {
                    // Parse single index
                    let index = parse_usize_literal(&inner_tokens);
                    expr = ComputedExpr::Index {
                        expr: Box::new(expr),
                        index,
                    };
                    continue;
                }
            }
        }

        // Check for dot (method call or field access)
        if let proc_macro2::TokenTree::Punct(p) = &tokens[pos] {
            if p.as_char() == '.' {
                pos += 1;
                if pos >= tokens.len() {
                    break;
                }

                // Get method/field name
                if let proc_macro2::TokenTree::Ident(ident) = &tokens[pos] {
                    let name = ident.to_string();
                    pos += 1;

                    // Check for method call with parentheses
                    if pos < tokens.len() {
                        if let proc_macro2::TokenTree::Group(group) = &tokens[pos] {
                            if group.delimiter() == proc_macro2::Delimiter::Parenthesis {
                                // Parse arguments
                                let args = parse_method_args(&group.stream());
                                pos += 1;

                                // Check for unwrap_or specifically
                                if name == "unwrap_or" && args.len() == 1 {
                                    if let ComputedExpr::Literal { value } = &args[0] {
                                        expr = ComputedExpr::UnwrapOr {
                                            expr: Box::new(expr),
                                            default: value.clone(),
                                        };
                                        continue;
                                    }
                                }

                                // Handle reverse_bits() as a unary op
                                if name == "reverse_bits" && args.is_empty() {
                                    expr = ComputedExpr::Unary {
                                        op: UnaryOp::ReverseBits,
                                        expr: Box::new(expr),
                                    };
                                    continue;
                                }

                                // Handle to_bytes() for JSON array to Vec<u8> conversion
                                if name == "to_bytes" && args.is_empty() {
                                    expr = ComputedExpr::JsonToBytes {
                                        expr: Box::new(expr),
                                    };
                                    continue;
                                }

                                if let Some(resolver) = resolver_for_method(&name) {
                                    let base_expr = expr;
                                    let mut resolver_args = Vec::with_capacity(args.len() + 1);
                                    resolver_args.push(base_expr);
                                    resolver_args.extend(args);
                                    expr = ComputedExpr::ResolverComputed {
                                        resolver: resolver.to_string(),
                                        method: name,
                                        args: resolver_args,
                                    };
                                    continue;
                                }

                                expr = ComputedExpr::MethodCall {
                                    expr: Box::new(expr),
                                    method: name,
                                    args,
                                };
                                continue;
                            }
                        }
                    }

                    // It's a field access, merge into the path
                    if let ComputedExpr::FieldRef { path } = &expr {
                        expr = ComputedExpr::FieldRef {
                            path: format!("{}.{}", path, name),
                        };
                    } else {
                        // Convert existing expr to method call with no args?
                        // Actually this shouldn't happen for field refs
                        expr = ComputedExpr::MethodCall {
                            expr: Box::new(expr),
                            method: name,
                            args: vec![],
                        };
                    }
                    continue;
                }
            }
        }

        // Check for 'as' type cast
        if let proc_macro2::TokenTree::Ident(ident) = &tokens[pos] {
            if *ident == "as" {
                pos += 1;
                if pos < tokens.len() {
                    if let proc_macro2::TokenTree::Ident(type_ident) = &tokens[pos] {
                        let to_type = type_ident.to_string();
                        pos += 1;
                        expr = ComputedExpr::Cast {
                            expr: Box::new(expr),
                            to_type,
                        };
                        continue;
                    }
                }
            }
        }

        break;
    }

    (expr, pos)
}

/// Parse a usize literal from tokens.
fn parse_usize_literal(tokens: &[proc_macro2::TokenTree]) -> usize {
    if tokens.is_empty() {
        return 0;
    }

    if let proc_macro2::TokenTree::Literal(lit) = &tokens[0] {
        let lit_str = lit.to_string().replace('_', "");
        lit_str.parse::<usize>().unwrap_or(0)
    } else {
        0
    }
}

/// Parse a primary expression (parenthesized, identifier, literal, byte array, etc.).
fn parse_primary_expr(tokens: &[proc_macro2::TokenTree], start: usize) -> (ComputedExpr, usize) {
    if start >= tokens.len() {
        return (
            ComputedExpr::Literal {
                value: serde_json::Value::Null,
            },
            start,
        );
    }

    match &tokens[start] {
        // Parenthesized expression
        proc_macro2::TokenTree::Group(group)
            if group.delimiter() == proc_macro2::Delimiter::Parenthesis =>
        {
            let inner_tokens: Vec<_> = group.stream().into_iter().collect();
            let (inner_expr, _) = parse_expr(&inner_tokens, 0);
            (
                ComputedExpr::Paren {
                    expr: Box::new(inner_expr),
                },
                start + 1,
            )
        }

        // Bracket expression - byte array literal: [0u8; 32] or [1, 2, 3]
        proc_macro2::TokenTree::Group(group)
            if group.delimiter() == proc_macro2::Delimiter::Bracket =>
        {
            let inner_tokens: Vec<_> = group.stream().into_iter().collect();
            let bytes = parse_byte_array_literal(&inner_tokens);
            (ComputedExpr::ByteArray { bytes }, start + 1)
        }

        // Brace-delimited block expression: { let x = ...; expr }
        proc_macro2::TokenTree::Group(group)
            if group.delimiter() == proc_macro2::Delimiter::Brace =>
        {
            let inner_tokens: Vec<_> = group.stream().into_iter().collect();
            let (inner_expr, _) = parse_expr(&inner_tokens, 0);
            (inner_expr, start + 1)
        }

        // Closure: |param| body
        proc_macro2::TokenTree::Punct(p) if p.as_char() == '|' => parse_closure(tokens, start),

        // Identifier (field reference, None, Some, or type-qualified function)
        proc_macro2::TokenTree::Ident(ident) => {
            let name = ident.to_string();

            // Check for None
            if name == "None" {
                return (ComputedExpr::None, start + 1);
            }

            // Check for context access special identifiers
            if name == "__slot" {
                return (ComputedExpr::ContextSlot, start + 1);
            }
            if name == "__timestamp" {
                return (ComputedExpr::ContextTimestamp, start + 1);
            }

            // Check for Some(expr)
            if name == "Some" && start + 1 < tokens.len() {
                if let proc_macro2::TokenTree::Group(group) = &tokens[start + 1] {
                    if group.delimiter() == proc_macro2::Delimiter::Parenthesis {
                        let inner_tokens: Vec<_> = group.stream().into_iter().collect();
                        let (inner_expr, _) = parse_expr(&inner_tokens, 0);
                        return (
                            ComputedExpr::Some {
                                value: Box::new(inner_expr),
                            },
                            start + 2,
                        );
                    }
                }
            }

            // Check for type-qualified function call: Type::method(args)
            // e.g., u64::from_le_bytes(expr)
            if start + 1 < tokens.len() {
                if let proc_macro2::TokenTree::Punct(p1) = &tokens[start + 1] {
                    if p1.as_char() == ':' && start + 2 < tokens.len() {
                        if let proc_macro2::TokenTree::Punct(p2) = &tokens[start + 2] {
                            if p2.as_char() == ':' && start + 3 < tokens.len() {
                                if let proc_macro2::TokenTree::Ident(method_ident) =
                                    &tokens[start + 3]
                                {
                                    let method_name = method_ident.to_string();

                                    // Check for function call
                                    if start + 4 < tokens.len() {
                                        if let proc_macro2::TokenTree::Group(group) =
                                            &tokens[start + 4]
                                        {
                                            if group.delimiter()
                                                == proc_macro2::Delimiter::Parenthesis
                                            {
                                                let inner_tokens: Vec<_> =
                                                    group.stream().into_iter().collect();
                                                let (arg_expr, _) = parse_expr(&inner_tokens, 0);

                                                // Handle u64::from_le_bytes and u64::from_be_bytes
                                                if name == "u64" && method_name == "from_le_bytes" {
                                                    return (
                                                        ComputedExpr::U64FromLeBytes {
                                                            bytes: Box::new(arg_expr),
                                                        },
                                                        start + 5,
                                                    );
                                                }
                                                if name == "u64" && method_name == "from_be_bytes" {
                                                    return (
                                                        ComputedExpr::U64FromBeBytes {
                                                            bytes: Box::new(arg_expr),
                                                        },
                                                        start + 5,
                                                    );
                                                }

                                                // Generic type::method(arg) - treat as method call
                                                return (
                                                    ComputedExpr::MethodCall {
                                                        expr: Box::new(arg_expr),
                                                        method: format!(
                                                            "{}::{}",
                                                            name, method_name
                                                        ),
                                                        args: vec![],
                                                    },
                                                    start + 5,
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            (ComputedExpr::FieldRef { path: name }, start + 1)
        }

        // Literal (number)
        proc_macro2::TokenTree::Literal(lit) => {
            let lit_str = lit.to_string();
            // Parse as number
            let value = if lit_str.contains('.') {
                // Float
                lit_str
                    .parse::<f64>()
                    .map(serde_json::Value::from)
                    .unwrap_or(serde_json::Value::String(lit_str))
            } else {
                // Integer - handle underscore separators and type suffixes (0u8, etc.)
                let clean = lit_str
                    .replace('_', "")
                    .trim_end_matches(|c: char| c.is_alphabetic())
                    .to_string();
                clean
                    .parse::<i64>()
                    .map(serde_json::Value::from)
                    .unwrap_or(serde_json::Value::String(lit_str))
            };
            (ComputedExpr::Literal { value }, start + 1)
        }

        // Unknown token - skip
        _ => (
            ComputedExpr::Literal {
                value: serde_json::Value::Null,
            },
            start + 1,
        ),
    }
}

/// Parse a closure: |param| body
fn parse_closure(tokens: &[proc_macro2::TokenTree], start: usize) -> (ComputedExpr, usize) {
    // Skip first |
    let mut pos = start + 1;

    // Get parameter name
    let param = if pos < tokens.len() {
        if let proc_macro2::TokenTree::Ident(ident) = &tokens[pos] {
            pos += 1;
            ident.to_string()
        } else {
            "x".to_string()
        }
    } else {
        "x".to_string()
    };

    // Skip second |
    if pos < tokens.len() {
        if let proc_macro2::TokenTree::Punct(p) = &tokens[pos] {
            if p.as_char() == '|' {
                pos += 1;
            }
        }
    }

    // Parse body (rest of tokens)
    let remaining: Vec<_> = tokens[pos..].to_vec();
    let (body, consumed) = parse_expr(&remaining, 0);

    (
        ComputedExpr::Closure {
            param,
            body: Box::new(body),
        },
        pos + consumed,
    )
}

/// Parse a byte array literal: [0u8; 32] or [1, 2, 3]
fn parse_byte_array_literal(tokens: &[proc_macro2::TokenTree]) -> Vec<u8> {
    if tokens.is_empty() {
        return vec![];
    }

    // Check for repeat syntax: [value; count]
    // Look for semicolon
    let mut semicolon_pos = None;
    for (i, token) in tokens.iter().enumerate() {
        if let proc_macro2::TokenTree::Punct(p) = token {
            if p.as_char() == ';' {
                semicolon_pos = Some(i);
                break;
            }
        }
    }

    if let Some(semi_pos) = semicolon_pos {
        // Repeat syntax: [value; count]
        let value_tokens: Vec<_> = tokens[..semi_pos].to_vec();
        let count_tokens: Vec<_> = tokens[semi_pos + 1..].to_vec();

        let value = parse_byte_value(&value_tokens);
        let count = parse_usize_from_tokens(&count_tokens);

        return vec![value; count];
    }

    // List syntax: [1, 2, 3]
    let mut bytes = Vec::new();
    let mut current_tokens = Vec::new();

    for token in tokens {
        if let proc_macro2::TokenTree::Punct(p) = token {
            if p.as_char() == ',' {
                if !current_tokens.is_empty() {
                    bytes.push(parse_byte_value(&current_tokens));
                    current_tokens.clear();
                }
                continue;
            }
        }
        current_tokens.push(token.clone());
    }

    // Don't forget the last element
    if !current_tokens.is_empty() {
        bytes.push(parse_byte_value(&current_tokens));
    }

    bytes
}

/// Parse a single byte value from tokens (e.g., 0u8, 255, 0xFF, 0xFFu8)
fn parse_byte_value(tokens: &[proc_macro2::TokenTree]) -> u8 {
    if tokens.is_empty() {
        return 0;
    }

    if let proc_macro2::TokenTree::Literal(lit) = &tokens[0] {
        let lit_str = lit.to_string();
        // Handle hex literals (0xFF, 0xFFu8)
        // Note: The literal may be lowercase (0xff) or uppercase (0xFF)
        let lower = lit_str.to_lowercase();
        if lower.starts_with("0x") {
            // For hex literals, strip the 0x prefix and any type suffix (u8, u16, etc.)
            let hex_str = lower.trim_start_matches("0x");
            // Type suffix starts with non-hex letter (u, i)
            let hex_part = if let Some(pos) = hex_str.find(['u', 'i']) {
                &hex_str[..pos]
            } else {
                hex_str
            };
            return u8::from_str_radix(hex_part, 16).unwrap_or(0);
        }
        // Handle decimal literals (0u8, 255)
        let clean = lit_str.trim_end_matches(|c: char| c.is_alphabetic());
        return clean.parse::<u8>().unwrap_or(0);
    }

    0
}

/// Parse a usize from tokens
fn parse_usize_from_tokens(tokens: &[proc_macro2::TokenTree]) -> usize {
    if tokens.is_empty() {
        return 0;
    }

    if let proc_macro2::TokenTree::Literal(lit) = &tokens[0] {
        let lit_str = lit
            .to_string()
            .trim_end_matches(|c: char| c.is_alphabetic())
            .to_string();
        return lit_str.parse::<usize>().unwrap_or(0);
    }

    0
}

/// Parse method arguments from a TokenStream.
pub fn parse_method_args(tokens: &proc_macro2::TokenStream) -> Vec<ComputedExpr> {
    let tokens_vec: Vec<_> = tokens.clone().into_iter().collect();
    if tokens_vec.is_empty() {
        return vec![];
    }

    let mut args = vec![];
    let mut current_start = 0;
    let mut depth: usize = 0;

    for (i, token) in tokens_vec.iter().enumerate() {
        match token {
            proc_macro2::TokenTree::Punct(p) if p.as_char() == ',' && depth == 0 => {
                let arg_tokens: Vec<_> = tokens_vec[current_start..i].to_vec();
                if !arg_tokens.is_empty() {
                    let (expr, _) = parse_expr(&arg_tokens, 0);
                    args.push(expr);
                }
                current_start = i + 1;
            }
            proc_macro2::TokenTree::Group(_) => {
                depth += 1;
            }
            proc_macro2::TokenTree::Punct(p) if p.as_char() == ')' => {
                depth = depth.saturating_sub(1);
            }
            _ => {}
        }
    }

    // Parse remaining tokens as last argument
    let remaining: Vec<_> = tokens_vec[current_start..].to_vec();
    if !remaining.is_empty() {
        let (expr, _) = parse_expr(&remaining, 0);
        args.push(expr);
    }

    args
}
