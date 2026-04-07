//! Filter and projection logic for `query_entities`.
//!
//! The core `Filter` / `Predicate` types and the string-DSL parser are ported
//! verbatim from `cli/src/commands/stream/filter.rs` so the MCP and CLI use
//! identical query semantics. Keeping a duplicate copy (rather than depending
//! on the `cli` crate) avoids an SDK→CLI inversion in the workspace.
//!
//! On top of that we expose [`Filter::from_structured`], which compiles a
//! sequence of [`StructuredPredicate`] objects (LLM-friendly JSON) into the
//! same `Filter`. Both inputs may be combined and are ANDed together by
//! `Filter::matches`.

use anyhow::{bail, Result};
use regex::Regex;
use rmcp::schemars::{self, JsonSchema};
use serde::Deserialize;
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

    /// Compile structured predicates (the LLM-friendly JSON form) into a
    /// `Filter`. The result can be combined with a string-DSL `Filter` by
    /// concatenating their `predicates` vectors.
    pub fn from_structured(preds: &[StructuredPredicate]) -> Result<Self> {
        let predicates = preds
            .iter()
            .map(structured_to_predicate)
            .collect::<Result<Vec<_>>>()?;
        Ok(Filter { predicates })
    }

    pub fn is_empty(&self) -> bool {
        self.predicates.is_empty()
    }

    pub fn matches(&self, value: &Value) -> bool {
        self.predicates.iter().all(|p| p.matches(value))
    }

    /// Append predicates from another filter into this one. Used to merge a
    /// string-DSL filter and a structured filter into a single AND'd query.
    pub fn extend(&mut self, other: Filter) {
        self.predicates.extend(other.predicates);
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
                None => true,
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
            if n.to_string() == expected {
                return true;
            }
            if let (Some(lhs), Ok(rhs)) = (n.as_f64(), expected.parse::<f64>()) {
                lhs == rhs
            } else {
                false
            }
        }
        Value::Bool(b) => (expected == "true" && *b) || (expected == "false" && !b),
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
    let n: f64 = value
        .parse()
        .map_err(|_| anyhow::anyhow!("Expected number after '>', got '{}'", value))?;
    Ok(FilterOp::Gt(n))
}
fn make_gte(value: &str) -> Result<FilterOp> {
    let n: f64 = value
        .parse()
        .map_err(|_| anyhow::anyhow!("Expected number after '>=', got '{}'", value))?;
    Ok(FilterOp::Gte(n))
}
fn make_lt(value: &str) -> Result<FilterOp> {
    let n: f64 = value
        .parse()
        .map_err(|_| anyhow::anyhow!("Expected number after '<', got '{}'", value))?;
    Ok(FilterOp::Lt(n))
}
fn make_lte(value: &str) -> Result<FilterOp> {
    let n: f64 = value
        .parse()
        .map_err(|_| anyhow::anyhow!("Expected number after '<=', got '{}'", value))?;
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

// ── Structured (JSON) filter form ────────────────────────────────────────────

/// LLM-friendly structured predicate. One of these maps 1:1 to a [`Predicate`].
///
/// Example JSON:
/// ```json
/// {"path": "user.age", "op": "gt", "value": 18}
/// {"path": "email", "op": "exists"}
/// ```
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct StructuredPredicate {
    /// Dot-path to the field, e.g. `"user.name"`.
    pub path: String,
    /// Operator: `eq`, `not_eq`, `gt`, `gte`, `lt`, `lte`, `regex`, `not_regex`,
    /// `exists`, `not_exists`.
    pub op: StructuredOp,
    /// Right-hand value. Required for all ops except `exists` / `not_exists`.
    /// Strings, numbers, and booleans are all accepted; numeric comparisons
    /// require a number.
    #[serde(default)]
    pub value: Option<Value>,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StructuredOp {
    Eq,
    NotEq,
    Gt,
    Gte,
    Lt,
    Lte,
    Regex,
    NotRegex,
    Exists,
    NotExists,
}

fn structured_to_predicate(p: &StructuredPredicate) -> Result<Predicate> {
    let path = parse_path(&p.path);

    let need_value = || -> Result<&Value> {
        p.value
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("op {:?} requires a `value`", p.op))
    };
    let need_number = || -> Result<f64> {
        let v = need_value()?;
        as_f64(v).ok_or_else(|| anyhow::anyhow!("op {:?} requires a numeric `value`", p.op))
    };
    let need_string = || -> Result<String> {
        let v = need_value()?;
        match v {
            Value::String(s) => Ok(s.clone()),
            other => Ok(other.to_string()),
        }
    };

    let op = match p.op {
        StructuredOp::Eq => FilterOp::Eq(value_to_string(need_value()?)),
        StructuredOp::NotEq => FilterOp::NotEq(value_to_string(need_value()?)),
        StructuredOp::Gt => FilterOp::Gt(need_number()?),
        StructuredOp::Gte => FilterOp::Gte(need_number()?),
        StructuredOp::Lt => FilterOp::Lt(need_number()?),
        StructuredOp::Lte => FilterOp::Lte(need_number()?),
        StructuredOp::Regex => {
            let s = need_string()?;
            FilterOp::Regex(
                Regex::new(&s).map_err(|e| anyhow::anyhow!("invalid regex '{}': {}", s, e))?,
            )
        }
        StructuredOp::NotRegex => {
            let s = need_string()?;
            FilterOp::NotRegex(
                Regex::new(&s).map_err(|e| anyhow::anyhow!("invalid regex '{}': {}", s, e))?,
            )
        }
        StructuredOp::Exists => FilterOp::Exists,
        StructuredOp::NotExists => FilterOp::NotExists,
    };
    Ok(Predicate { path, op })
}

/// Convert a JSON value into the string form expected by `FilterOp::Eq`.
/// Strings come through unquoted; everything else stringifies via serde so
/// numbers and bools round-trip cleanly.
fn value_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── DSL parser tests (mirrors cli/src/commands/stream/filter.rs) ─────────

    #[test]
    fn dsl_eq_string() {
        let f = Filter::parse(&["name=alice".to_string()]).unwrap();
        assert!(f.matches(&json!({"name": "alice"})));
        assert!(!f.matches(&json!({"name": "bob"})));
    }

    #[test]
    fn dsl_gt_and_gte() {
        let f = Filter::parse(&["score>=100".to_string()]).unwrap();
        assert!(f.matches(&json!({"score": 100})));
        assert!(!f.matches(&json!({"score": 99})));
    }

    #[test]
    fn dsl_nested_path() {
        let f = Filter::parse(&["user.name=alice".to_string()]).unwrap();
        assert!(f.matches(&json!({"user": {"name": "alice"}})));
    }

    #[test]
    fn dsl_exists_and_not_exists() {
        let f = Filter::parse(&["email?".to_string()]).unwrap();
        assert!(f.matches(&json!({"email": "x"})));
        assert!(!f.matches(&json!({"email": null})));

        let g = Filter::parse(&["email!?".to_string()]).unwrap();
        assert!(g.matches(&json!({"name": "alice"})));
        assert!(!g.matches(&json!({"email": "x"})));
    }

    #[test]
    fn dsl_regex() {
        let f = Filter::parse(&["name~^ali".to_string()]).unwrap();
        assert!(f.matches(&json!({"name": "alice"})));
        assert!(!f.matches(&json!({"name": "bob"})));
    }

    #[test]
    fn dsl_two_char_op_precedence() {
        // ">=" must not parse as ">" with value "=100"
        let f = Filter::parse(&["score>=100".to_string()]).unwrap();
        assert!(f.matches(&json!({"score": 100})));
    }

    // ── Structured form tests ────────────────────────────────────────────────

    fn structured(json: serde_json::Value) -> Filter {
        let preds: Vec<StructuredPredicate> = serde_json::from_value(json).unwrap();
        Filter::from_structured(&preds).unwrap()
    }

    #[test]
    fn structured_eq_string_value() {
        let f = structured(json!([{"path": "name", "op": "eq", "value": "alice"}]));
        assert!(f.matches(&json!({"name": "alice"})));
        assert!(!f.matches(&json!({"name": "bob"})));
    }

    #[test]
    fn structured_eq_number_value() {
        let f = structured(json!([{"path": "age", "op": "eq", "value": 30}]));
        assert!(f.matches(&json!({"age": 30})));
        assert!(!f.matches(&json!({"age": 31})));
    }

    #[test]
    fn structured_gt_requires_number() {
        let res = Filter::from_structured(&[StructuredPredicate {
            path: "x".to_string(),
            op: StructuredOp::Gt,
            value: Some(json!("not a number")),
        }]);
        assert!(res.is_err());
    }

    #[test]
    fn structured_exists_no_value() {
        let f = structured(json!([{"path": "email", "op": "exists"}]));
        assert!(f.matches(&json!({"email": "x"})));
        assert!(!f.matches(&json!({"name": "y"})));
    }

    #[test]
    fn structured_and_dsl_compose() {
        let mut f = Filter::parse(&["age>=18".to_string()]).unwrap();
        let g = structured(json!([{"path": "name", "op": "eq", "value": "alice"}]));
        f.extend(g);
        assert!(f.matches(&json!({"age": 25, "name": "alice"})));
        assert!(!f.matches(&json!({"age": 25, "name": "bob"})));
        assert!(!f.matches(&json!({"age": 16, "name": "alice"})));
    }

    // ── Projection ───────────────────────────────────────────────────────────

    #[test]
    fn select_projects_dot_paths_without_collision() {
        let v = json!({"a": {"id": 1}, "b": {"id": 2}});
        let fields = parse_select("a.id,b.id");
        assert_eq!(select_fields(&v, &fields), json!({"a.id": 1, "b.id": 2}));
    }
}
