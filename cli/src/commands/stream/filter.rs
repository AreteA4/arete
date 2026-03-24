use anyhow::{bail, Result};
use regex::Regex;
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct Filter {
    pub predicates: Vec<Predicate>,
}

#[derive(Debug, Clone)]
pub struct Predicate {
    pub path: Vec<String>,
    pub op: FilterOp,
}

#[derive(Debug, Clone)]
pub enum FilterOp {
    Eq(String),
    NotEq(String),
    Gt(f64),
    Gte(f64),
    Lt(f64),
    Lte(f64),
    Regex(Regex),
    NotRegex(Regex),
    Exists,
    NotExists,
}

impl Filter {
    pub fn parse(exprs: &[String]) -> Result<Self> {
        let predicates = exprs
            .iter()
            .map(|expr| parse_predicate(expr))
            .collect::<Result<Vec<_>>>()?;
        Ok(Filter { predicates })
    }

    pub fn is_empty(&self) -> bool {
        self.predicates.is_empty()
    }

    pub fn matches(&self, value: &Value) -> bool {
        self.predicates.iter().all(|p| p.matches(value))
    }
}

impl Predicate {
    fn matches(&self, value: &Value) -> bool {
        let resolved = resolve_path(value, &self.path);

        match &self.op {
            FilterOp::Exists => resolved.is_some() && !resolved.unwrap().is_null(),
            FilterOp::NotExists => resolved.is_none() || resolved.unwrap().is_null(),
            FilterOp::Eq(expected) => match resolved {
                Some(v) => value_eq(v, expected),
                None => false,
            },
            FilterOp::NotEq(expected) => match resolved {
                Some(v) => !value_eq(v, expected),
                None => true,
            },
            FilterOp::Gt(n) => resolved.and_then(as_f64).is_some_and(|v| v > *n),
            FilterOp::Gte(n) => resolved.and_then(as_f64).is_some_and(|v| v >= *n),
            FilterOp::Lt(n) => resolved.and_then(as_f64).is_some_and(|v| v < *n),
            FilterOp::Lte(n) => resolved.and_then(as_f64).is_some_and(|v| v <= *n),
            FilterOp::Regex(re) => resolved
                .and_then(|v| v.as_str())
                .is_some_and(|s| re.is_match(s)),
            FilterOp::NotRegex(re) => match resolved.and_then(|v| v.as_str()) {
                Some(s) => !re.is_match(s),
                None => true, // absent/non-string: "does not match regex" — consistent with NotEq
            },
        }
    }
}

fn resolve_path<'a>(value: &'a Value, path: &[String]) -> Option<&'a Value> {
    let mut current = value;
    for segment in path {
        current = current.get(segment)?;
    }
    Some(current)
}

fn value_eq(value: &Value, expected: &str) -> bool {
    match value {
        Value::String(s) => s == expected,
        Value::Number(n) => {
            if let Ok(expected_n) = expected.parse::<f64>() {
                n.as_f64() == Some(expected_n)
            } else {
                n.to_string() == expected
            }
        }
        Value::Bool(b) => {
            (expected == "true" && *b) || (expected == "false" && !b)
        }
        Value::Null => expected == "null",
        _ => {
            let s = serde_json::to_string(value).unwrap_or_default();
            s == expected
        }
    }
}

fn as_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.parse::<f64>().ok(),
        _ => None,
    }
}

fn parse_predicate(expr: &str) -> Result<Predicate> {
    let expr = expr.trim();

    if expr.is_empty() {
        bail!("Empty filter expression; expected field=value, field>N, field~regex, etc.");
    }

    // Existence: field? or field!?
    if let Some(field) = expr.strip_suffix("!?") {
        return Ok(Predicate {
            path: parse_path(field),
            op: FilterOp::NotExists,
        });
    }
    if let Some(field) = expr.strip_suffix('?') {
        return Ok(Predicate {
            path: parse_path(field),
            op: FilterOp::Exists,
        });
    }

    // Two-char operators checked first so ">=" isn't misread as ">" with value "=...".
    // Operator is matched at its first occurrence, so the value portion (after the operator)
    // may contain operator characters (e.g. --where "name=a=b" → field="name", value="a=b").
    // This is intentional: the split is on the first operator found, rest is the value.
    for (op_str, make_op) in &[
        ("!=", make_not_eq as fn(&str) -> Result<FilterOp>),
        (">=", make_gte as fn(&str) -> Result<FilterOp>),
        ("<=", make_lte as fn(&str) -> Result<FilterOp>),
        ("!~", make_not_regex as fn(&str) -> Result<FilterOp>),
    ] {
        if let Some(idx) = expr.find(op_str) {
            let field = &expr[..idx];
            if field.is_empty() {
                bail!("Missing field name before '{}' in: '{}'", op_str, expr);
            }
            let value = &expr[idx + op_str.len()..];
            return Ok(Predicate {
                path: parse_path(field),
                op: make_op(value)?,
            });
        }
    }

    // Single-char operators: =, >, <, ~
    for (op_char, make_op) in &[
        ('>', make_gt as fn(&str) -> Result<FilterOp>),
        ('<', make_lt as fn(&str) -> Result<FilterOp>),
        ('~', make_regex as fn(&str) -> Result<FilterOp>),
        ('=', make_eq as fn(&str) -> Result<FilterOp>),
    ] {
        if let Some(idx) = expr.find(*op_char) {
            let field = &expr[..idx];
            if field.is_empty() {
                bail!("Missing field name before '{}' in: '{}'", op_char, expr);
            }
            let value = &expr[idx + 1..];
            return Ok(Predicate {
                path: parse_path(field),
                op: make_op(value)?,
            });
        }
    }

    bail!(
        "Invalid filter expression: '{}'\n\
         Expected: field=value, field>N, field~regex, field?, etc.",
        expr
    )
}

fn parse_path(field: &str) -> Vec<String> {
    field.split('.').map(|s| s.to_string()).collect()
}

fn make_eq(value: &str) -> Result<FilterOp> {
    Ok(FilterOp::Eq(value.to_string()))
}

fn make_not_eq(value: &str) -> Result<FilterOp> {
    Ok(FilterOp::NotEq(value.to_string()))
}

fn make_gt(value: &str) -> Result<FilterOp> {
    let n: f64 = value.parse().map_err(|_| anyhow::anyhow!("Expected number after '>', got '{}'", value))?;
    Ok(FilterOp::Gt(n))
}

fn make_gte(value: &str) -> Result<FilterOp> {
    let n: f64 = value.parse().map_err(|_| anyhow::anyhow!("Expected number after '>=', got '{}'", value))?;
    Ok(FilterOp::Gte(n))
}

fn make_lt(value: &str) -> Result<FilterOp> {
    let n: f64 = value.parse().map_err(|_| anyhow::anyhow!("Expected number after '<', got '{}'", value))?;
    Ok(FilterOp::Lt(n))
}

fn make_lte(value: &str) -> Result<FilterOp> {
    let n: f64 = value.parse().map_err(|_| anyhow::anyhow!("Expected number after '<=', got '{}'", value))?;
    Ok(FilterOp::Lte(n))
}

fn make_regex(value: &str) -> Result<FilterOp> {
    let re = Regex::new(value).map_err(|e| anyhow::anyhow!("Invalid regex '{}': {}", value, e))?;
    Ok(FilterOp::Regex(re))
}

fn make_not_regex(value: &str) -> Result<FilterOp> {
    let re = Regex::new(value).map_err(|e| anyhow::anyhow!("Invalid regex '{}': {}", value, e))?;
    Ok(FilterOp::NotRegex(re))
}

/// Project specific fields from a JSON value.
/// Returns a new object with only the selected dot-paths.
/// Uses full dot-path as key to avoid collisions (e.g. "a.id" and "b.id"
/// produce {"a.id": ..., "b.id": ...} instead of silently overwriting).
pub fn select_fields(value: &Value, fields: &[Vec<String>]) -> Value {
    let mut result = serde_json::Map::new();
    for path in fields {
        if let Some(v) = resolve_path(value, path) {
            let key = if path.len() == 1 {
                path[0].clone()
            } else {
                path.join(".")
            };
            result.insert(key, v.clone());
        }
    }
    Value::Object(result)
}

pub fn parse_select(select: &str) -> Vec<Vec<String>> {
    select
        .split(',')
        .map(|s| s.trim().split('.').map(|p| p.to_string()).collect())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_eq_string() {
        let f = Filter::parse(&["name=alice".to_string()]).unwrap();
        assert!(f.matches(&json!({"name": "alice"})));
        assert!(!f.matches(&json!({"name": "bob"})));
    }

    #[test]
    fn test_eq_number() {
        let f = Filter::parse(&["age=30".to_string()]).unwrap();
        assert!(f.matches(&json!({"age": 30})));
        assert!(!f.matches(&json!({"age": 31})));
    }

    #[test]
    fn test_gt() {
        let f = Filter::parse(&["score>100".to_string()]).unwrap();
        assert!(f.matches(&json!({"score": 150})));
        assert!(!f.matches(&json!({"score": 50})));
        assert!(!f.matches(&json!({"score": 100})));
    }

    #[test]
    fn test_nested_path() {
        let f = Filter::parse(&["user.name=alice".to_string()]).unwrap();
        assert!(f.matches(&json!({"user": {"name": "alice"}})));
        assert!(!f.matches(&json!({"user": {"name": "bob"}})));
    }

    #[test]
    fn test_exists() {
        let f = Filter::parse(&["email?".to_string()]).unwrap();
        assert!(f.matches(&json!({"email": "a@b.com"})));
        assert!(!f.matches(&json!({"name": "alice"})));
        assert!(!f.matches(&json!({"email": null})));
    }

    #[test]
    fn test_not_exists() {
        let f = Filter::parse(&["email!?".to_string()]).unwrap();
        assert!(!f.matches(&json!({"email": "a@b.com"})));
        assert!(f.matches(&json!({"name": "alice"})));
    }

    #[test]
    fn test_regex() {
        let f = Filter::parse(&["name~^ali".to_string()]).unwrap();
        assert!(f.matches(&json!({"name": "alice"})));
        assert!(!f.matches(&json!({"name": "bob"})));
    }

    #[test]
    fn test_multiple_filters_and() {
        let f = Filter::parse(&[
            "age>18".to_string(),
            "name=alice".to_string(),
        ]).unwrap();
        assert!(f.matches(&json!({"age": 25, "name": "alice"})));
        assert!(!f.matches(&json!({"age": 25, "name": "bob"})));
        assert!(!f.matches(&json!({"age": 15, "name": "alice"})));
    }

    #[test]
    fn test_select_fields() {
        let v = json!({"name": "alice", "age": 30, "nested": {"x": 1}});
        let fields = parse_select("name,nested.x");
        let result = select_fields(&v, &fields);
        assert_eq!(result, json!({"name": "alice", "nested.x": 1}));
    }

    #[test]
    fn test_select_fields_no_collision() {
        let v = json!({"a": {"id": 1}, "b": {"id": 2}});
        let fields = parse_select("a.id,b.id");
        let result = select_fields(&v, &fields);
        assert_eq!(result, json!({"a.id": 1, "b.id": 2}));
    }

    #[test]
    fn test_not_eq() {
        let f = Filter::parse(&["name!=alice".to_string()]).unwrap();
        assert!(!f.matches(&json!({"name": "alice"})));
        assert!(f.matches(&json!({"name": "bob"})));
        // Absent field: != should return true
        assert!(f.matches(&json!({"age": 30})));
    }

    #[test]
    fn test_gte() {
        let f = Filter::parse(&["score>=100".to_string()]).unwrap();
        assert!(f.matches(&json!({"score": 100})));
        assert!(f.matches(&json!({"score": 150})));
        assert!(!f.matches(&json!({"score": 99})));
    }

    #[test]
    fn test_lte() {
        let f = Filter::parse(&["score<=100".to_string()]).unwrap();
        assert!(f.matches(&json!({"score": 100})));
        assert!(f.matches(&json!({"score": 50})));
        assert!(!f.matches(&json!({"score": 101})));
    }

    #[test]
    fn test_not_regex() {
        let f = Filter::parse(&["name!~^ali".to_string()]).unwrap();
        assert!(!f.matches(&json!({"name": "alice"})));
        assert!(f.matches(&json!({"name": "bob"})));
        // Absent field: !~ should return true (consistent with !=)
        assert!(f.matches(&json!({"age": 30})));
    }

    #[test]
    fn test_two_char_operator_precedence() {
        // Ensure >= is not parsed as > with value "=100"
        let f = Filter::parse(&["score>=100".to_string()]).unwrap();
        assert!(f.matches(&json!({"score": 100})));

        // Ensure != is not parsed as ! with something else
        let f = Filter::parse(&["name!=x".to_string()]).unwrap();
        assert!(f.matches(&json!({"name": "y"})));
        assert!(!f.matches(&json!({"name": "x"})));
    }
}
