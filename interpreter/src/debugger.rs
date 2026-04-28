use serde::{Deserialize, Serialize};
use serde_json::Value;

pub trait VmDebugger: Send + Sync {
    fn record(&self, event: VmDebugEvent);
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VmDebugEvent {
    ProcessEventStart {
        event_type: String,
        context: Option<Value>,
    },
    ProcessEventEnd {
        event_type: String,
        mutations: usize,
        warnings: Vec<String>,
    },
    HandlerStart {
        entity_name: String,
        event_type: String,
        state_id: u32,
    },
    HandlerEnd {
        entity_name: String,
        event_type: String,
        mutations: usize,
    },
    LoadEventField {
        entity_name: String,
        event_type: String,
        path: Vec<String>,
        value: Value,
    },
    ReadOrInitState {
        entity_name: String,
        event_type: String,
        key: Value,
        existing_state: Option<Value>,
        loaded_state: Value,
        skipped_reason: Option<String>,
    },
    LookupIndex {
        entity_name: String,
        event_type: String,
        index_name: String,
        lookup_value: Value,
        hops: Vec<VmLookupHop>,
        final_result: Value,
        miss_kind: Option<String>,
    },
    FieldWrite {
        entity_name: String,
        event_type: String,
        op: String,
        path: String,
        old_value: Option<Value>,
        new_value: Option<Value>,
        applied: bool,
        reason: Option<String>,
    },
    EmitMutation {
        entity_name: String,
        event_type: String,
        key: Value,
        emitted: bool,
        reason: Option<String>,
        patch: Option<Value>,
        dirty_fields: Vec<String>,
    },
    QueueAction {
        entity_name: String,
        event_type: String,
        queue_kind: String,
        lookup_value: String,
    },
    FlushAction {
        entity_name: String,
        event_type: String,
        flush_kind: String,
        trigger: String,
        count: usize,
    },
    PdaReverseLookupUpdate {
        entity_name: String,
        event_type: String,
        lookup_name: String,
        pda_address: String,
        primary_key: Value,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VmLookupHop {
    pub source: String,
    pub input: Value,
    pub result: Value,
    pub chained: bool,
}
