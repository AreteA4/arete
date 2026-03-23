use crate::ast::{ComparisonOp, FieldPath, LogicalOp, ParsedCondition, ResolverCondition};

/// Parse a condition expression string into a ParsedCondition AST
///
/// Supported syntax:
/// - Comparisons: "field > 100", "amount >= 1000000"
/// - Logical ops: "amount > 100 && user != \"excluded\"", "a < 10 || a > 1000"
/// - Field refs: "amount", "data.field", "accounts.user"
///
#[allow(dead_code)]
pub fn parse_condition_expression(expr: &str) -> Option<ParsedCondition> {
    parse_condition_expression_strict(expr).ok()
}

pub fn parse_condition_expression_strict(expr: &str) -> Result<ParsedCondition, String> {
    let expr = expr.trim();

    if expr.is_empty() {
        return Err("Condition expression cannot be empty".to_string());
    }

    // Try to parse as logical expression first (contains && or ||)
    if let Some(parsed) = try_parse_logical(expr)? {
        return Ok(parsed);
    }

    // Parse as comparison
    try_parse_comparison(expr)
}

fn try_parse_logical(expr: &str) -> Result<Option<ParsedCondition>, String> {
    // Split on && or || (respecting precedence: && before ||)
    // For simplicity, we can use a basic tokenizer

    // Find top-level || first (lowest precedence)
    if let Some(pos) = find_top_level_operator(expr, "||") {
        let left = expr[..pos].trim();
        let right = expr[pos + 2..].trim();

        return Ok(Some(ParsedCondition::Logical {
            op: LogicalOp::Or,
            conditions: vec![
                parse_condition_expression_strict(left)?,
                parse_condition_expression_strict(right)?,
            ],
        }));
    }

    // Find top-level && (higher precedence)
    if let Some(pos) = find_top_level_operator(expr, "&&") {
        let left = expr[..pos].trim();
        let right = expr[pos + 2..].trim();

        return Ok(Some(ParsedCondition::Logical {
            op: LogicalOp::And,
            conditions: vec![
                parse_condition_expression_strict(left)?,
                parse_condition_expression_strict(right)?,
            ],
        }));
    }

    Ok(None)
}

fn try_parse_comparison(expr: &str) -> Result<ParsedCondition, String> {
    // Try each comparison operator in order (longer ones first to avoid conflicts)
    let operators = [
        (">=", ComparisonOp::GreaterThanOrEqual),
        ("<=", ComparisonOp::LessThanOrEqual),
        ("==", ComparisonOp::Equal),
        ("!=", ComparisonOp::NotEqual),
        (">", ComparisonOp::GreaterThan),
        ("<", ComparisonOp::LessThan),
    ];

    for (op_str, op) in &operators {
        if let Some(pos) = find_top_level_operator(expr, op_str) {
            let field = expr[..pos].trim();
            let value = expr[pos + op_str.len()..].trim();

            if field.is_empty() {
                return Err(format!(
                    "Invalid condition expression '{}': missing field before operator",
                    expr
                ));
            }

            if value.is_empty() {
                return Err(format!(
                    "Invalid condition expression '{}': missing value after operator",
                    expr
                ));
            }

            if operators
                .iter()
                .any(|(other_op, _)| value.starts_with(other_op))
            {
                return Err(format!(
                    "Invalid condition expression '{}': unexpected operator sequence near '{}'",
                    expr, value
                ));
            }

            // Parse field path
            let field_segments: Vec<&str> = field.split('.').collect();
            let field_path = FieldPath::new(&field_segments);

            // Parse value (number, string, or bool)
            let value_json = parse_value(value)?;

            return Ok(ParsedCondition::Comparison {
                field: field_path,
                op: op.clone(),
                value: value_json,
            });
        }
    }

    Err(format!(
        "Invalid condition expression '{}'. Expected a comparison like 'field > 100' or a logical expression using && / ||",
        expr
    ))
}

fn find_top_level_operator(expr: &str, op: &str) -> Option<usize> {
    // Find operator position, ignoring those inside quotes or parentheses
    let mut depth = 0;
    let mut in_quotes = false;
    let mut quote_char = '\0';
    let mut previous_char = None;

    for (byte_idx, c) in expr.char_indices() {
        if !in_quotes {
            if c == '"' || c == '\'' {
                in_quotes = true;
                quote_char = c;
            } else if c == '(' {
                depth += 1;
            } else if c == ')' {
                depth -= 1;
            } else if depth == 0 && expr[byte_idx..].starts_with(op) {
                return Some(byte_idx);
            }
        } else if c == quote_char && previous_char != Some('\\') {
            in_quotes = false;
        }

        previous_char = Some(c);
    }

    None
}

fn parse_value(value: &str) -> Result<serde_json::Value, String> {
    use serde_json::Value;

    let value_trimmed = value.trim();
    if value_trimmed == "null" {
        return Ok(Value::Null);
    }
    if value_trimmed == "ZERO_32" {
        return Ok(Value::Array(vec![Value::Number(0.into()); 32]));
    }
    if value_trimmed == "ZERO_64" {
        return Ok(Value::Array(vec![Value::Number(0.into()); 64]));
    }

    // Remove underscores from numeric literals (Rust style: 1_000_000)
    let value_clean = value.replace('_', "");

    // Try parsing as different types
    if value_clean == "true" {
        Ok(Value::Bool(true))
    } else if value_clean == "false" {
        Ok(Value::Bool(false))
    } else if let Ok(num) = value_clean.parse::<i64>() {
        Ok(Value::Number(num.into()))
    } else if let Ok(num) = value_clean.parse::<f64>() {
        serde_json::Number::from_f64(num)
            .map(Value::Number)
            .ok_or_else(|| format!("Invalid numeric value '{}'.", value))
    } else if (value.starts_with('"') && value.ends_with('"'))
        || (value.starts_with('\'') && value.ends_with('\''))
    {
        Ok(Value::String(value[1..value.len() - 1].to_string()))
    } else {
        // Could be a field reference - for now, treat as string
        Ok(Value::String(value.to_string()))
    }
}

pub fn parse_resolver_condition_expression(expr: &str) -> Result<ResolverCondition, String> {
    let operators = [">=", "<=", "==", "!=", ">", "<"];
    for op_str in &operators {
        if let Some(pos) = find_top_level_operator(expr, op_str) {
            let field_path = expr[..pos].trim().to_string();
            let raw_value = expr[pos + op_str.len()..].trim();

            if field_path.is_empty() {
                return Err(format!(
                    "Invalid condition expression: '{}'. Missing field before operator.",
                    expr
                ));
            }

            if raw_value.is_empty() {
                return Err(format!(
                    "Invalid condition expression: '{}'. Missing value after operator.",
                    expr
                ));
            }

            if operators
                .iter()
                .any(|other_op| raw_value.starts_with(other_op))
            {
                return Err(format!(
                    "Invalid condition expression: '{}'. Unexpected operator sequence near '{}'.",
                    expr, raw_value
                ));
            }

            let op = match *op_str {
                "==" => ComparisonOp::Equal,
                "!=" => ComparisonOp::NotEqual,
                ">=" => ComparisonOp::GreaterThanOrEqual,
                "<=" => ComparisonOp::LessThanOrEqual,
                ">" => ComparisonOp::GreaterThan,
                "<" => ComparisonOp::LessThan,
                _ => unreachable!(),
            };

            let value = match raw_value {
                "null" => serde_json::Value::Null,
                "true" => serde_json::Value::Bool(true),
                "false" => serde_json::Value::Bool(false),
                s => {
                    if let Ok(number) = s.parse::<i64>() {
                        serde_json::Value::Number(number.into())
                    } else if let Ok(number) = s.parse::<f64>() {
                        match serde_json::Number::from_f64(number) {
                            Some(number) => serde_json::Value::Number(number),
                            None => {
                                return Err(format!(
                                    "Invalid numeric value '{}' in condition expression '{}': non-finite floats are not supported.",
                                    s, expr
                                ))
                            }
                        }
                    } else {
                        serde_json::Value::String(
                            if (s.starts_with('"') && s.ends_with('"'))
                                || (s.starts_with('\'') && s.ends_with('\''))
                            {
                                s[1..s.len() - 1].to_string()
                            } else {
                                s.to_string()
                            },
                        )
                    }
                }
            };

            return Ok(ResolverCondition {
                field_path,
                op,
                value,
            });
        }
    }

    Err(format!(
        "Invalid condition expression: '{}'. Expected format: 'field.path <op> value' (supported operators: ==, !=, >, >=, <, <=)",
        expr
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_comparison() {
        let parsed = parse_condition_expression("amount > 1000").unwrap();
        match parsed {
            ParsedCondition::Comparison { field, op, value } => {
                assert_eq!(field.segments, vec!["amount"]);
                assert!(matches!(op, ComparisonOp::GreaterThan));
                assert_eq!(value, serde_json::json!(1000));
            }
            _ => panic!("Expected comparison"),
        }
    }

    #[test]
    fn test_numeric_with_underscores() {
        let parsed = parse_condition_expression("amount > 1_000_000_000_000").unwrap();
        match parsed {
            ParsedCondition::Comparison { field, op, value } => {
                assert_eq!(field.segments, vec!["amount"]);
                assert!(matches!(op, ComparisonOp::GreaterThan));
                assert_eq!(value, serde_json::json!(1000000000000i64));
            }
            _ => panic!("Expected comparison"),
        }
    }

    #[test]
    fn test_logical_and() {
        let parsed = parse_condition_expression("amount > 100 && user != \"excluded\"").unwrap();
        match parsed {
            ParsedCondition::Logical { op, conditions } => {
                assert!(matches!(op, LogicalOp::And));
                assert_eq!(conditions.len(), 2);
            }
            _ => panic!("Expected logical"),
        }
    }

    #[test]
    fn test_nested_field_path() {
        let parsed = parse_condition_expression("data.amount >= 500").unwrap();
        match parsed {
            ParsedCondition::Comparison { field, op, value } => {
                assert_eq!(field.segments, vec!["data", "amount"]);
                assert!(matches!(op, ComparisonOp::GreaterThanOrEqual));
                assert_eq!(value, serde_json::json!(500));
            }
            _ => panic!("Expected comparison"),
        }
    }

    #[test]
    fn test_zero_32_constant() {
        let parsed = parse_condition_expression("value != ZERO_32").unwrap();
        match parsed {
            ParsedCondition::Comparison { field, op, value } => {
                assert_eq!(field.segments, vec!["value"]);
                assert!(matches!(op, ComparisonOp::NotEqual));
                let arr = value.as_array().unwrap();
                assert_eq!(arr.len(), 32);
                assert!(arr.iter().all(|v| v == &serde_json::json!(0)));
            }
            _ => panic!("Expected comparison"),
        }
    }

    #[test]
    fn test_zero_64_constant() {
        let parsed = parse_condition_expression("data != ZERO_64").unwrap();
        match parsed {
            ParsedCondition::Comparison { value, .. } => {
                let arr = value.as_array().unwrap();
                assert_eq!(arr.len(), 64);
            }
            _ => panic!("Expected comparison"),
        }
    }

    #[test]
    fn test_resolver_condition_ignores_operators_inside_quotes() {
        let parsed =
            parse_resolver_condition_expression("status == \"pending >= review\"").unwrap();

        assert_eq!(parsed.field_path, "status");
        assert!(matches!(parsed.op, ComparisonOp::Equal));
        assert_eq!(parsed.value, serde_json::json!("pending >= review"));
    }

    #[test]
    fn test_resolver_condition_rejects_non_finite_float_values() {
        let error = parse_resolver_condition_expression("score == NaN").unwrap_err();

        assert!(error.contains("non-finite floats are not supported"));
    }

    #[test]
    fn test_resolver_condition_preserves_large_integer_values() {
        let parsed =
            parse_resolver_condition_expression("lamport_balance == 9999999999999999").unwrap();

        assert_eq!(parsed.field_path, "lamport_balance");
        assert_eq!(parsed.value, serde_json::json!(9999999999999999i64));
    }

    #[test]
    fn test_resolver_condition_strips_single_quoted_strings() {
        let parsed = parse_resolver_condition_expression("status == 'pending'").unwrap();

        assert_eq!(parsed.field_path, "status");
        assert_eq!(parsed.value, serde_json::json!("pending"));
    }

    #[test]
    fn test_map_condition_null_literal_parses_as_json_null() {
        let parsed = parse_condition_expression("status == null").unwrap();

        match parsed {
            ParsedCondition::Comparison { value, .. } => {
                assert_eq!(value, serde_json::Value::Null);
            }
            _ => panic!("Expected comparison"),
        }
    }
}
