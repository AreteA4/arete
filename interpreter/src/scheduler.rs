use crate::ast::{ComparisonOp, ResolverCondition, UrlTemplatePart};
use crate::vm::ScheduledCallback;
use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};

pub const MAX_RETRIES: u32 = 100;

type DedupKey = (String, String, String);

pub struct SlotScheduler {
    callbacks: BTreeMap<u64, Vec<ScheduledCallback>>,
    registered: HashSet<DedupKey>,
    /// Reverse index: dedup_key → slot, for O(1) targeted removal in register().
    slot_index: HashMap<DedupKey, u64>,
}

impl Default for SlotScheduler {
    fn default() -> Self {
        Self::new()
    }
}

impl SlotScheduler {
    pub fn new() -> Self {
        Self {
            callbacks: BTreeMap::new(),
            registered: HashSet::new(),
            slot_index: HashMap::new(),
        }
    }

    pub fn register(&mut self, target_slot: u64, callback: ScheduledCallback) {
        let dedup_key = Self::dedup_key(&callback);
        if let Some(old_slot) = self.slot_index.remove(&dedup_key) {
            if let Some(cbs) = self.callbacks.get_mut(&old_slot) {
                cbs.retain(|cb| Self::dedup_key(cb) != dedup_key);
                if cbs.is_empty() {
                    self.callbacks.remove(&old_slot);
                }
            }
        }
        self.registered.insert(dedup_key.clone());
        self.slot_index.insert(dedup_key, target_slot);
        self.callbacks
            .entry(target_slot)
            .or_default()
            .push(callback);
    }

    pub fn take_due(&mut self, current_slot: u64) -> Vec<ScheduledCallback> {
        let future = self.callbacks.split_off(&current_slot.saturating_add(1));
        let due = std::mem::replace(&mut self.callbacks, future);

        let mut result = Vec::new();
        for (_slot, callbacks) in due {
            for cb in callbacks {
                let dedup_key = Self::dedup_key(&cb);
                self.registered.remove(&dedup_key);
                self.slot_index.remove(&dedup_key);
                result.push(cb);
            }
        }
        result
    }

    pub fn re_register(&mut self, callback: ScheduledCallback, next_slot: u64) {
        self.register(next_slot, callback);
    }

    pub fn pending_count(&self) -> usize {
        self.callbacks.values().map(|v| v.len()).sum()
    }

    fn dedup_key(cb: &ScheduledCallback) -> DedupKey {
        let resolver_key = serde_json::to_string(&cb.resolver).unwrap_or_default();
        let condition_key = cb
            .condition
            .as_ref()
            .map(|c| serde_json::to_string(c).unwrap_or_default())
            .unwrap_or_default();
        let pk_key = cb.primary_key.to_string();
        (
            cb.entity_name.clone(),
            pk_key,
            format!("{}:{}", resolver_key, condition_key),
        )
    }
}

pub fn evaluate_condition(condition: &ResolverCondition, state: &Value) -> bool {
    let field_val = get_value_at_path(state, &condition.field_path).unwrap_or(Value::Null);
    evaluate_comparison(&field_val, &condition.op, &condition.value)
}

/// NON_ALPHANUMERIC minus RFC 3986 unreserved chars (`-`, `.`, `_`, `~`) that are safe in URLs.
/// NOTE: This uses path-segment encoding for all field references, including those in query
/// parameter positions. Query strings permit additional chars (`!`, `$`, `'`, `(`, `)`, `+`, etc.)
/// that will be over-encoded. This is safe for the current numeric/base58 use-cases but may need
/// a path-vs-query split if general-purpose URL templates are needed.
const URL_SEGMENT_SET: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

pub fn build_url_from_template(template: &[UrlTemplatePart], state: &Value) -> Option<String> {
    let mut url = String::new();
    for part in template {
        match part {
            UrlTemplatePart::Literal(s) => url.push_str(s),
            UrlTemplatePart::FieldRef(path) => {
                let val = get_value_at_path(state, path)?;
                if val.is_null() {
                    return None;
                }
                let raw = match val.as_str() {
                    Some(s) => s.to_string(),
                    None => val.to_string().trim_matches('"').to_string(),
                };
                let encoded = utf8_percent_encode(&raw, URL_SEGMENT_SET).to_string();
                url.push_str(&encoded);
            }
        }
    }
    Some(url)
}

pub fn get_value_at_path(value: &Value, path: &str) -> Option<Value> {
    let mut current = value;
    for segment in path.split('.') {
        current = current.get(segment)?;
    }
    Some(current.clone())
}

/// Mirrors the VM's i64 → u64 → f64 comparison cascade to avoid divergent
/// condition evaluation between registration time and execution time.
fn evaluate_comparison(field_value: &Value, op: &ComparisonOp, condition_value: &Value) -> bool {
    match op {
        ComparisonOp::Equal => field_value == condition_value,
        ComparisonOp::NotEqual => field_value != condition_value,
        ComparisonOp::GreaterThan => compare_numeric(
            field_value,
            condition_value,
            |a, b| a > b,
            |a, b| a > b,
            |a, b| a > b,
        ),
        ComparisonOp::GreaterThanOrEqual => compare_numeric(
            field_value,
            condition_value,
            |a, b| a >= b,
            |a, b| a >= b,
            |a, b| a >= b,
        ),
        ComparisonOp::LessThan => compare_numeric(
            field_value,
            condition_value,
            |a, b| a < b,
            |a, b| a < b,
            |a, b| a < b,
        ),
        ComparisonOp::LessThanOrEqual => compare_numeric(
            field_value,
            condition_value,
            |a, b| a <= b,
            |a, b| a <= b,
            |a, b| a <= b,
        ),
    }
}

fn compare_numeric(
    a: &Value,
    b: &Value,
    cmp_i64: fn(i64, i64) -> bool,
    cmp_u64: fn(u64, u64) -> bool,
    cmp_f64: fn(f64, f64) -> bool,
) -> bool {
    match (a.as_i64(), b.as_i64()) {
        (Some(a), Some(b)) => cmp_i64(a, b),
        _ => match (a.as_u64(), b.as_u64()) {
            (Some(a), Some(b)) => cmp_u64(a, b),
            _ => match (a.as_f64(), b.as_f64()) {
                (Some(a), Some(b)) => cmp_f64(a, b),
                _ => false,
            },
        },
    }
}
