use crate::ast::{ComparisonOp, ResolverCondition, UrlTemplatePart};
use crate::vm::ScheduledCallback;
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

pub const MAX_RETRIES: u32 = 100;

pub struct SlotScheduler {
    callbacks: BTreeMap<u64, Vec<ScheduledCallback>>,
    registered: HashSet<(String, String, u64)>,
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
        }
    }

    pub fn register(&mut self, target_slot: u64, callback: ScheduledCallback) {
        let dedup_key = Self::dedup_key(&callback);
        if self.registered.contains(&dedup_key) {
            for cbs in self.callbacks.values_mut() {
                cbs.retain(|cb| Self::dedup_key(cb) != dedup_key);
            }
            self.callbacks.retain(|_, cbs| !cbs.is_empty());
        }
        self.registered.insert(dedup_key);
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
                result.push(cb);
            }
        }
        result
    }

    pub fn re_register(&mut self, callback: ScheduledCallback, next_slot: u64) {
        let dedup_key = Self::dedup_key(&callback);
        self.registered.insert(dedup_key);
        self.callbacks
            .entry(next_slot)
            .or_default()
            .push(callback);
    }

    pub fn pending_count(&self) -> usize {
        self.callbacks.values().map(|v| v.len()).sum()
    }

    fn dedup_key(cb: &ScheduledCallback) -> (String, String, u64) {
        let mut hasher = DefaultHasher::new();
        cb.resolver.hash(&mut hasher);
        let resolver_hash = hasher.finish();
        let pk_key = cb.primary_key.to_string();
        (cb.entity_name.clone(), pk_key, resolver_hash)
    }
}

pub fn evaluate_condition(condition: &ResolverCondition, state: &Value) -> bool {
    let field_val = get_value_at_path(state, &condition.field_path).unwrap_or(Value::Null);
    evaluate_comparison(&field_val, &condition.op, &condition.value)
}

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
                let encoded = utf8_percent_encode(&raw, NON_ALPHANUMERIC).to_string();
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

fn evaluate_comparison(field_value: &Value, op: &ComparisonOp, condition_value: &Value) -> bool {
    match op {
        ComparisonOp::Equal => field_value == condition_value,
        ComparisonOp::NotEqual => field_value != condition_value,
        ComparisonOp::GreaterThan => compare_numeric(field_value, condition_value, |a, b| a > b),
        ComparisonOp::GreaterThanOrEqual => {
            compare_numeric(field_value, condition_value, |a, b| a >= b)
        }
        ComparisonOp::LessThan => compare_numeric(field_value, condition_value, |a, b| a < b),
        ComparisonOp::LessThanOrEqual => {
            compare_numeric(field_value, condition_value, |a, b| a <= b)
        }
    }
}

fn compare_numeric(a: &Value, b: &Value, cmp: fn(f64, f64) -> bool) -> bool {
    match (a.as_f64(), b.as_f64()) {
        (Some(a), Some(b)) => cmp(a, b),
        _ => false,
    }
}
