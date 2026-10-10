//! Token-amount scale of entity fields, read from a LiveSpec entity's
//! computed fields.
//!
//! A stack scales token amounts with `ui_amount(decimals)` (as a `#[map]`
//! `transform` or in a `#[computed]` expression) and back with
//! `raw_amount(decimals)`. Both compile to a `TokenMetadata` resolver
//! computation in the entity's `computed_field_specs`, so the scale of the
//! field it produces, and of the field it reads, is already in the AST:
//!
//! - the produced field holds `ui` amounts (whole tokens, `raw / 10^decimals`);
//! - the field it reads holds `raw` integer base units (for SOL, lamports).
//!
//! Nothing here guesses from field names or types: a field is annotated only
//! when a computation says how it is scaled, and left alone when two
//! computations disagree.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// How a numeric field is scaled.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AmountScale {
    /// Whole-token units: the raw amount divided by `10^decimals`, as a
    /// float (`1.5` SOL).
    Ui,
    /// Integer base units (`1500000000` lamports), often string-encoded
    /// because u64 values exceed JSON number precision.
    Raw,
}

impl AmountScale {
    fn other(self) -> Self {
        match self {
            AmountScale::Ui => AmountScale::Raw,
            AmountScale::Raw => AmountScale::Ui,
        }
    }
}

/// The token-amount scale of one field.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FieldAmount {
    pub scale: AmountScale,
    /// The token's decimals, when the stack fixes them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decimals: Option<u64>,
    /// The field the decimals are read from at runtime, when the stack does
    /// not fix them (e.g. a mint's metadata).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decimals_from: Option<String>,
    /// The emitted field holding the same amount at the other scale.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub counterpart: Option<String>,
}

impl FieldAmount {
    /// One line for text output, e.g. `ui amount (raw / 10^11)`.
    pub fn describe(&self) -> String {
        let divisor = match (self.decimals, &self.decimals_from) {
            (Some(decimals), _) => format!("10^{decimals}"),
            (None, Some(from)) => format!("10^{from}"),
            (None, None) => "10^decimals".to_string(),
        };
        let mut text = match self.scale {
            AmountScale::Ui => format!("token amount in whole units (raw base units / {divisor})"),
            AmountScale::Raw => {
                format!(
                    "token amount in raw integer base units (divide by {divisor} for whole units)"
                )
            }
        };
        if let Some(counterpart) = &self.counterpart {
            let scale = match self.scale.other() {
                AmountScale::Ui => "whole units",
                AmountScale::Raw => "raw units",
            };
            text.push_str(&format!("; {scale}: {counterpart}"));
        }
        text
    }

    fn same_scale(&self, other: &FieldAmount) -> bool {
        self.scale == other.scale
            && self.decimals == other.decimals
            && self.decimals_from == other.decimals_from
    }
}

/// The amount scale of every field of `entity` (a LiveSpec entity) that a
/// `ui_amount`/`raw_amount` computation produces or reads, by `section.field`
/// path. `visible` are the emitted field paths: counterparts outside it are
/// not named.
pub fn entity_field_amounts(
    entity: &Value,
    visible: &BTreeSet<String>,
) -> BTreeMap<String, FieldAmount> {
    let mut found: BTreeMap<String, FieldAmount> = BTreeMap::new();
    let mut conflicted: BTreeSet<String> = BTreeSet::new();
    let mut add = |path: String, amount: FieldAmount| {
        if conflicted.contains(&path) {
            return;
        }
        match found.get_mut(&path) {
            Some(existing) if existing.same_scale(&amount) => {
                if existing.counterpart.is_none() {
                    existing.counterpart = amount.counterpart;
                }
            }
            Some(_) => {
                found.remove(&path);
                conflicted.insert(path);
            }
            None => {
                found.insert(path, amount);
            }
        }
    };

    let specs = ["computed_field_specs", "computedFieldSpecs"]
        .iter()
        .find_map(|key| entity.get(*key).and_then(Value::as_array))
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    for spec in specs {
        let Some(target) = ["target_path", "targetPath"]
            .iter()
            .find_map(|key| spec.get(*key).and_then(Value::as_str))
        else {
            continue;
        };
        let Some(call) = spec.get("expression").and_then(amount_call) else {
            continue;
        };
        let (decimals, decimals_from) = match call.decimals {
            Value::Object(map) => match map.get("Literal") {
                Some(literal) => (literal.get("value").and_then(literal_u64), None),
                None => (None, field_ref(call.decimals).map(str::to_string)),
            },
            _ => (None, None),
        };
        if decimals.is_none() && decimals_from.is_none() {
            continue;
        }
        let source = field_root(call.value);
        // Only a field holding the same amount at the other scale is a
        // counterpart: not one that was summed or otherwise aggregated.
        let counterpart =
            |path: &str, exact: bool| (exact && visible.contains(path)).then(|| path.to_string());
        add(
            target.to_string(),
            FieldAmount {
                scale: call.produces,
                decimals,
                decimals_from: decimals_from.clone(),
                counterpart: source.and_then(|(path, exact)| counterpart(path, exact)),
            },
        );
        if let Some((source, exact)) = source {
            add(
                source.to_string(),
                FieldAmount {
                    scale: call.produces.other(),
                    decimals,
                    decimals_from,
                    counterpart: counterpart(target, exact),
                },
            );
        }
    }
    found
}

/// A `ui_amount`/`raw_amount` computation: the value it scales, its
/// decimals argument, and the scale it produces.
struct AmountCall<'a> {
    value: &'a Value,
    decimals: &'a Value,
    produces: AmountScale,
}

/// The amount computation an expression evaluates to, looking through
/// grouping, `Some(..)`, `.unwrap_or(..)` and an element-wise
/// `.map(|x| ui_amount(x, ..))`.
fn amount_call(expr: &Value) -> Option<AmountCall<'_>> {
    let (variant, body) = single_variant(expr)?;
    match variant {
        "ResolverComputed" => {
            let produces = match body.get("method")?.as_str()? {
                "ui_amount" => AmountScale::Ui,
                "raw_amount" => AmountScale::Raw,
                _ => return None,
            };
            let args = body.get("args")?.as_array()?;
            let [value, decimals] = args.as_slice() else {
                return None;
            };
            Some(AmountCall {
                value,
                decimals,
                produces,
            })
        }
        "Paren" | "UnwrapOr" => amount_call(body.get("expr")?),
        "Some" => amount_call(body.get("value")?),
        "MethodCall" if body.get("method")?.as_str()? == "map" => {
            let args = body.get("args")?.as_array()?;
            let [closure] = args.as_slice() else {
                return None;
            };
            let (kind, closure) = single_variant(closure)?;
            if kind != "Closure" {
                return None;
            }
            let param = closure.get("param")?.as_str()?;
            let inner = amount_call(closure.get("body")?)?;
            // Only `|x| ui_amount(x, ..)`: the closure scales each element.
            let (var, var_body) = single_variant(inner.value)?;
            if var != "Var" || var_body.get("name")?.as_str()? != param {
                return None;
            }
            Some(AmountCall {
                value: body.get("expr")?,
                decimals: inner.decimals,
                produces: inner.produces,
            })
        }
        _ => None,
    }
}

/// The field whose amounts `expr` reads: a field reference, possibly
/// grouped, cast or defaulted (`exact`: the same amount), or aggregated
/// (`.sum()`, `.max()`...), which keeps the scale of its input but not the
/// amount. `None` for anything that mixes values.
fn field_root(expr: &Value) -> Option<(&str, bool)> {
    let (variant, body) = single_variant(expr)?;
    match variant {
        "FieldRef" => Some((body.get("path")?.as_str()?, true)),
        "Paren" | "Cast" | "UnwrapOr" => field_root(body.get("expr")?),
        "Some" => field_root(body.get("value")?),
        "MethodCall" => match body.get("method")?.as_str()? {
            "unwrap_or" | "clone" => field_root(body.get("expr")?),
            "sum" | "max" | "min" | "first" | "last" => {
                field_root(body.get("expr")?).map(|(path, _)| (path, false))
            }
            _ => None,
        },
        _ => None,
    }
}

fn field_ref(expr: &Value) -> Option<&str> {
    let (variant, body) = single_variant(expr)?;
    (variant == "FieldRef").then(|| body.get("path")?.as_str())?
}

/// A serde externally tagged enum value: `{"Variant": {...}}`.
fn single_variant(expr: &Value) -> Option<(&str, &Value)> {
    let map = expr.as_object()?;
    if map.len() != 1 {
        return None;
    }
    map.iter().next().map(|(key, value)| (key.as_str(), value))
}

fn literal_u64(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ui_amount(value: Value, decimals: Value) -> Value {
        json!({"ResolverComputed": {
            "resolver": "TokenMetadata",
            "method": "ui_amount",
            "args": [value, decimals],
        }})
    }

    fn field(path: &str) -> Value {
        json!({"FieldRef": {"path": path}})
    }

    fn literal(value: u64) -> Value {
        json!({"Literal": {"value": value}})
    }

    fn visible(paths: &[&str]) -> BTreeSet<String> {
        paths.iter().map(|path| path.to_string()).collect()
    }

    /// The shapes the ORE stack compiles to.
    fn ore_round() -> Value {
        json!({
            "computed_field_specs": [
                {"target_path": "state.motherlode", "result_type": "Option < f64 >",
                 "expression": ui_amount(field("state.__motherlode_raw"), literal(11))},
                {"target_path": "state.total_deployed", "result_type": "Option < f64 >",
                 "expression": ui_amount(
                    json!({"MethodCall": {"expr": field("state.deployed_per_square"), "method": "sum", "args": []}}),
                    literal(9))},
                {"target_path": "state.deployed_per_square_ui", "result_type": "Option < Vec < f64 > >",
                 "expression": {"MethodCall": {
                    "expr": field("state.deployed_per_square"),
                    "method": "map",
                    "args": [{"Closure": {"param": "x", "body": ui_amount(json!({"Var": {"name": "x"}}), literal(9))}}],
                 }}},
                {"target_path": "results.winning_square", "result_type": "Option < u64 >",
                 "expression": {"MethodCall": {"expr": field("results.rng"), "method": "map", "args": []}}},
            ]
        })
    }

    #[test]
    fn reads_ui_and_raw_scales_from_amount_computations() {
        let amounts = entity_field_amounts(
            &ore_round(),
            &visible(&[
                "state.motherlode",
                "state.total_deployed",
                "state.deployed_per_square",
                "state.deployed_per_square_ui",
                "results.winning_square",
            ]),
        );

        assert_eq!(
            amounts["state.motherlode"],
            FieldAmount {
                scale: AmountScale::Ui,
                decimals: Some(11),
                decimals_from: None,
                // The raw input is hidden, so it is not named.
                counterpart: None,
            }
        );
        assert_eq!(amounts["state.total_deployed"].decimals, Some(9));
        // A sum is not the same amount as its input.
        assert_eq!(amounts["state.total_deployed"].counterpart, None);
        assert_eq!(
            amounts["state.deployed_per_square"],
            FieldAmount {
                scale: AmountScale::Raw,
                decimals: Some(9),
                decimals_from: None,
                counterpart: Some("state.deployed_per_square_ui".into()),
            }
        );
        assert_eq!(
            amounts["state.deployed_per_square_ui"]
                .counterpart
                .as_deref(),
            Some("state.deployed_per_square")
        );
        assert!(!amounts.contains_key("results.winning_square"));
    }

    #[test]
    fn decimals_read_from_a_field_are_named() {
        let entity = json!({"computed_field_specs": [{
            "target_path": "state.balance",
            "expression": ui_amount(field("state.raw_balance"), field("token.decimals")),
        }]});
        let amounts = entity_field_amounts(&entity, &visible(&["state.raw_balance"]));

        assert_eq!(
            amounts["state.balance"].decimals_from.as_deref(),
            Some("token.decimals")
        );
        assert_eq!(amounts["state.raw_balance"].scale, AmountScale::Raw);
        assert_eq!(
            amounts["state.balance"].describe(),
            "token amount in whole units (raw base units / 10^token.decimals); raw units: state.raw_balance"
        );
    }

    #[test]
    fn conflicting_scales_leave_the_field_unannotated() {
        let entity = json!({"computed_field_specs": [
            {"target_path": "a.ui9", "expression": ui_amount(field("a.raw"), literal(9))},
            {"target_path": "a.ui6", "expression": ui_amount(field("a.raw"), literal(6))},
        ]});
        let amounts = entity_field_amounts(&entity, &visible(&[]));

        assert!(!amounts.contains_key("a.raw"));
        assert_eq!(amounts["a.ui9"].decimals, Some(9));
        assert_eq!(amounts["a.ui6"].decimals, Some(6));
    }

    #[test]
    fn mixed_inputs_are_not_annotated_as_raw() {
        let entity = json!({"computed_field_specs": [{
            "target_path": "a.net",
            "expression": ui_amount(
                json!({"Binary": {"op": "Sub", "left": field("a.x"), "right": field("a.y")}}),
                literal(9)),
        }]});
        let amounts = entity_field_amounts(&entity, &visible(&["a.x", "a.y"]));

        assert_eq!(amounts.len(), 1);
        assert_eq!(amounts["a.net"].scale, AmountScale::Ui);
    }
}
