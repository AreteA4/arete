use crate::ast::*;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use tracing;

pub type Register = usize;

fn stop_field_path(target_path: &str) -> String {
    format!("__stop:{}", target_path)
}

#[derive(Debug, Clone)]
pub enum OpCode {
    /// Abort the handler with empty mutations when the key register is null
    /// and the event is an account-state update (not IxState / CpiEvent).
    /// Placed immediately after key resolution so that downstream opcodes
    /// (index updates, field mappings, resolvers, emit) never execute with
    /// a garbage key. The null-key event is then eligible for queueing by
    /// process_event's miss-handling logic.
    AbortIfNullKey {
        key: Register,
        is_account_event: bool,
    },
    LoadEventField {
        path: FieldPath,
        dest: Register,
        default: Option<Value>,
    },
    LoadConstant {
        value: Value,
        dest: Register,
    },
    CopyRegister {
        source: Register,
        dest: Register,
    },
    /// Copy from source to dest only if dest is currently null
    CopyRegisterIfNull {
        source: Register,
        dest: Register,
    },
    GetEventType {
        dest: Register,
    },
    CreateObject {
        dest: Register,
    },
    SetField {
        object: Register,
        path: String,
        value: Register,
    },
    SetFields {
        object: Register,
        fields: Vec<(String, Register)>,
    },
    GetField {
        object: Register,
        path: String,
        dest: Register,
    },
    ReadOrInitState {
        state_id: u32,
        key: Register,
        default: Value,
        dest: Register,
    },
    UpdateState {
        state_id: u32,
        key: Register,
        value: Register,
    },
    AppendToArray {
        object: Register,
        path: String,
        value: Register,
    },
    GetCurrentTimestamp {
        dest: Register,
    },
    CreateEvent {
        dest: Register,
        event_value: Register,
    },
    CreateCapture {
        dest: Register,
        capture_value: Register,
    },
    Transform {
        source: Register,
        dest: Register,
        transformation: Transformation,
    },
    EmitMutation {
        entity_name: String,
        key: Register,
        state: Register,
    },
    SetFieldIfNull {
        object: Register,
        path: String,
        value: Register,
    },
    SetFieldMax {
        object: Register,
        path: String,
        value: Register,
    },
    UpdateTemporalIndex {
        state_id: u32,
        index_name: String,
        lookup_value: Register,
        primary_key: Register,
        timestamp: Register,
    },
    LookupTemporalIndex {
        state_id: u32,
        index_name: String,
        lookup_value: Register,
        timestamp: Register,
        dest: Register,
    },
    UpdateLookupIndex {
        state_id: u32,
        index_name: String,
        lookup_value: Register,
        primary_key: Register,
    },
    LookupIndex {
        state_id: u32,
        index_name: String,
        lookup_value: Register,
        dest: Register,
    },
    /// Sum a numeric value to a field (accumulator)
    SetFieldSum {
        object: Register,
        path: String,
        value: Register,
    },
    /// Increment a counter field by 1
    SetFieldIncrement {
        object: Register,
        path: String,
    },
    /// Set field to minimum value
    SetFieldMin {
        object: Register,
        path: String,
        value: Register,
    },
    /// Set field only if a specific instruction type was seen in the same transaction.
    /// If not seen yet, defers the operation for later completion.
    SetFieldWhen {
        object: Register,
        path: String,
        value: Register,
        when_instruction: String,
        entity_name: String,
        key_reg: Register,
        condition_field: Option<FieldPath>,
        condition_op: Option<ComparisonOp>,
        condition_value: Option<Value>,
    },
    /// Set field unless stopped by a specific instruction.
    /// Stop is tracked by a per-entity stop flag.
    SetFieldUnlessStopped {
        object: Register,
        path: String,
        value: Register,
        stop_field: String,
        stop_instruction: String,
        entity_name: String,
        key_reg: Register,
    },
    /// Add value to unique set and update count
    /// Maintains internal Set, field stores count
    AddToUniqueSet {
        state_id: u32,
        set_name: String,
        value: Register,
        count_object: Register,
        count_path: String,
    },
    /// Conditionally set a field based on a comparison
    ConditionalSetField {
        object: Register,
        path: String,
        value: Register,
        condition_field: FieldPath,
        condition_op: ComparisonOp,
        condition_value: Value,
    },
    /// Conditionally increment a field based on a comparison
    ConditionalIncrement {
        object: Register,
        path: String,
        condition_field: FieldPath,
        condition_op: ComparisonOp,
        condition_value: Value,
    },
    /// Evaluate computed fields (calls external hook if provided)
    /// computed_paths: List of paths that will be computed (for dirty tracking)
    EvaluateComputedFields {
        state: Register,
        computed_paths: Vec<String>,
    },
    /// Queue a resolver for asynchronous enrichment
    QueueResolver {
        state_id: u32,
        entity_name: String,
        resolver: ResolverType,
        input_path: Option<String>,
        input_value: Option<Value>,
        url_template: Option<Vec<UrlTemplatePart>>,
        strategy: ResolveStrategy,
        extracts: Vec<ResolverExtractSpec>,
        condition: Option<ResolverCondition>,
        schedule_at: Option<String>,
        state: Register,
        key: Register,
    },
    /// Update PDA reverse lookup table
    /// Maps a PDA address to its primary key for reverse lookups
    UpdatePdaReverseLookup {
        state_id: u32,
        lookup_name: String,
        pda_address: Register,
        primary_key: Register,
    },
}

pub struct EntityBytecode {
    pub state_id: u32,
    pub handlers: HashMap<String, Vec<OpCode>>,
    pub entity_name: String,
    pub when_events: HashSet<String>,
    pub non_emitted_fields: HashSet<String>,
    pub computed_paths: Vec<String>,
    /// Optional callback for evaluating computed fields
    /// Parameters: state, context_slot (Option<u64>), context_timestamp (i64)
    #[allow(clippy::type_complexity)]
    pub computed_fields_evaluator: Option<
        Box<
            dyn Fn(
                    &mut Value,
                    Option<u64>,
                    i64,
                ) -> std::result::Result<(), Box<dyn std::error::Error>>
                + Send
                + Sync,
        >,
    >,
}

impl std::fmt::Debug for EntityBytecode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EntityBytecode")
            .field("state_id", &self.state_id)
            .field("handlers", &self.handlers)
            .field("entity_name", &self.entity_name)
            .field("when_events", &self.when_events)
            .field("non_emitted_fields", &self.non_emitted_fields)
            .field("computed_paths", &self.computed_paths)
            .field(
                "computed_fields_evaluator",
                &self.computed_fields_evaluator.is_some(),
            )
            .finish()
    }
}

#[derive(Debug)]
pub struct MultiEntityBytecode {
    pub entities: HashMap<String, EntityBytecode>,
    pub event_routing: HashMap<String, Vec<String>>,
    pub when_events: HashSet<String>,
    pub proto_router: crate::proto_router::ProtoRouter,
}

impl MultiEntityBytecode {
    pub fn from_single<S>(entity_name: String, spec: TypedStreamSpec<S>, state_id: u32) -> Self {
        let compiler = TypedCompiler::new(spec, entity_name.clone()).with_state_id(state_id);
        let entity_bytecode = compiler.compile_entity();

        let mut entities = HashMap::new();
        let mut event_routing = HashMap::new();
        let mut when_events = HashSet::new();

        for event_type in entity_bytecode.handlers.keys() {
            event_routing
                .entry(event_type.clone())
                .or_insert_with(Vec::new)
                .push(entity_name.clone());
        }

        when_events.extend(entity_bytecode.when_events.iter().cloned());

        entities.insert(entity_name, entity_bytecode);

        MultiEntityBytecode {
            entities,
            event_routing,
            when_events,
            proto_router: crate::proto_router::ProtoRouter::new(),
        }
    }

    pub fn from_entities(entities_vec: Vec<(String, Box<dyn std::any::Any>, u32)>) -> Self {
        let entities = HashMap::new();
        let event_routing = HashMap::new();
        let when_events = HashSet::new();

        if let Some((_entity_name, _spec_any, _state_id)) = entities_vec.into_iter().next() {
            panic!("from_entities requires type information - use builder pattern instead");
        }

        MultiEntityBytecode {
            entities,
            event_routing,
            when_events,
            proto_router: crate::proto_router::ProtoRouter::new(),
        }
    }

    #[allow(clippy::new_ret_no_self)]
    pub fn new() -> MultiEntityBytecodeBuilder {
        MultiEntityBytecodeBuilder {
            entities: HashMap::new(),
            event_routing: HashMap::new(),
            when_events: HashSet::new(),
            proto_router: crate::proto_router::ProtoRouter::new(),
        }
    }
}

pub struct MultiEntityBytecodeBuilder {
    entities: HashMap<String, EntityBytecode>,
    event_routing: HashMap<String, Vec<String>>,
    when_events: HashSet<String>,
    proto_router: crate::proto_router::ProtoRouter,
}

impl MultiEntityBytecodeBuilder {
    pub fn add_entity<S>(
        self,
        entity_name: String,
        spec: TypedStreamSpec<S>,
        state_id: u32,
    ) -> Self {
        self.add_entity_with_evaluator(
            entity_name,
            spec,
            state_id,
            None::<
                fn(
                    &mut Value,
                    Option<u64>,
                    i64,
                ) -> std::result::Result<(), Box<dyn std::error::Error>>,
            >,
        )
    }

    pub fn add_entity_with_evaluator<S, F>(
        mut self,
        entity_name: String,
        spec: TypedStreamSpec<S>,
        state_id: u32,
        evaluator: Option<F>,
    ) -> Self
    where
        F: Fn(&mut Value, Option<u64>, i64) -> std::result::Result<(), Box<dyn std::error::Error>>
            + Send
            + Sync
            + 'static,
    {
        let compiler = TypedCompiler::new(spec, entity_name.clone()).with_state_id(state_id);
        let mut entity_bytecode = compiler.compile_entity();

        // Store the evaluator callback if provided
        if let Some(eval) = evaluator {
            entity_bytecode.computed_fields_evaluator = Some(Box::new(eval));
        }

        for event_type in entity_bytecode.handlers.keys() {
            self.event_routing
                .entry(event_type.clone())
                .or_default()
                .push(entity_name.clone());
        }

        self.when_events
            .extend(entity_bytecode.when_events.iter().cloned());

        self.entities.insert(entity_name, entity_bytecode);
        self
    }

    pub fn build(self) -> MultiEntityBytecode {
        MultiEntityBytecode {
            entities: self.entities,
            event_routing: self.event_routing,
            when_events: self.when_events,
            proto_router: self.proto_router,
        }
    }
}

pub struct TypedCompiler<S> {
    pub spec: TypedStreamSpec<S>,
    entity_name: String,
    state_id: u32,
}

impl<S> TypedCompiler<S> {
    pub fn new(spec: TypedStreamSpec<S>, entity_name: String) -> Self {
        TypedCompiler {
            spec,
            entity_name,
            state_id: 0,
        }
    }

    pub fn with_state_id(mut self, state_id: u32) -> Self {
        self.state_id = state_id;
        self
    }

    pub fn compile(&self) -> MultiEntityBytecode {
        let entity_bytecode = self.compile_entity();

        let mut entities = HashMap::new();
        let mut event_routing = HashMap::new();
        let mut when_events = HashSet::new();

        for event_type in entity_bytecode.handlers.keys() {
            event_routing
                .entry(event_type.clone())
                .or_insert_with(Vec::new)
                .push(self.entity_name.clone());
        }

        when_events.extend(entity_bytecode.when_events.iter().cloned());

        entities.insert(self.entity_name.clone(), entity_bytecode);

        MultiEntityBytecode {
            entities,
            event_routing,
            when_events,
            proto_router: crate::proto_router::ProtoRouter::new(),
        }
    }

    fn compile_entity(&self) -> EntityBytecode {
        let mut handlers: HashMap<String, Vec<OpCode>> = HashMap::new();
        let mut when_events: HashSet<String> = HashSet::new();
        let mut emit_by_path: HashMap<String, bool> = HashMap::new();

        // DEBUG: Collect all handler info before processing
        let mut debug_info = Vec::new();
        for (index, handler_spec) in self.spec.handlers.iter().enumerate() {
            let event_type = self.get_event_type(&handler_spec.source);
            let program_id = match &handler_spec.source {
                crate::ast::SourceSpec::Source { program_id, .. } => {
                    program_id.as_ref().map(|s| s.as_str()).unwrap_or("null")
                }
            };
            debug_info.push(format!(
                "  [{}] EventType={}, Mappings={}, ProgramId={}",
                index,
                event_type,
                handler_spec.mappings.len(),
                program_id
            ));
        }

        // DEBUG: Log handler information (optional - can be removed later)
        // Uncomment to debug handler processing:
        // if self.entity_name == "PumpfunToken" {
        //     eprintln!("🔍 Compiling {} handlers for {}", self.spec.handlers.len(), self.entity_name);
        //     for info in &debug_info {
        //         eprintln!("{}", info);
        //     }
        // }

        for handler_spec in &self.spec.handlers {
            for mapping in &handler_spec.mappings {
                if let Some(when) = &mapping.when {
                    when_events.insert(when.clone());
                }
                let entry = emit_by_path
                    .entry(mapping.target_path.clone())
                    .or_insert(false);
                *entry |= mapping.emit;
                if mapping.stop.is_some() {
                    emit_by_path
                        .entry(stop_field_path(&mapping.target_path))
                        .or_insert(false);
                }
            }
            let opcodes = self.compile_handler(handler_spec);
            let event_type = self.get_event_type(&handler_spec.source);

            if let Some(existing_opcodes) = handlers.get_mut(&event_type) {
                // Merge strategy: Take ALL operations from BOTH handlers
                // Keep setup from first, combine all mappings, keep one teardown

                // Split existing handler into: setup, mappings, teardown
                let mut existing_setup = Vec::new();
                let mut existing_mappings = Vec::new();
                let mut existing_teardown = Vec::new();
                let mut section = 0; // 0=setup, 1=mappings, 2=teardown

                for opcode in existing_opcodes.iter() {
                    match opcode {
                        OpCode::ReadOrInitState { .. } => {
                            existing_setup.push(opcode.clone());
                            section = 1; // Next opcodes are mappings
                        }
                        OpCode::UpdateState { .. } => {
                            existing_teardown.push(opcode.clone());
                            section = 2; // Next opcodes are teardown
                        }
                        OpCode::EmitMutation { .. } => {
                            existing_teardown.push(opcode.clone());
                        }
                        _ if section == 0 => existing_setup.push(opcode.clone()),
                        _ if section == 1 => existing_mappings.push(opcode.clone()),
                        _ => existing_teardown.push(opcode.clone()),
                    }
                }

                // Extract mappings from new handler (skip setup and teardown)
                let mut new_mappings = Vec::new();
                section = 0;

                for opcode in opcodes.iter() {
                    match opcode {
                        OpCode::ReadOrInitState { .. } => {
                            section = 1; // Start capturing mappings
                        }
                        OpCode::UpdateState { .. } | OpCode::EmitMutation { .. } => {
                            section = 2; // Stop capturing
                        }
                        _ if section == 1 => {
                            new_mappings.push(opcode.clone());
                        }
                        _ => {} // Skip setup and teardown from new handler
                    }
                }

                // Rebuild: setup + existing_mappings + new_mappings + teardown
                let mut merged = Vec::new();
                merged.extend(existing_setup);
                merged.extend(existing_mappings);
                merged.extend(new_mappings.clone());
                merged.extend(existing_teardown);

                *existing_opcodes = merged;
            } else {
                handlers.insert(event_type, opcodes);
            }
        }

        // Process instruction_hooks to add SetField/IncrementField operations
        for hook in &self.spec.instruction_hooks {
            let event_type = hook.instruction_type.clone();

            let handler_opcodes = handlers.entry(event_type.clone()).or_insert_with(|| {
                let key_reg = 20;
                let state_reg = 2;
                let resolved_key_reg = 19;
                let temp_reg = 18;

                let mut ops = Vec::new();

                // First, try to load __resolved_primary_key from resolver
                ops.push(OpCode::LoadEventField {
                    path: FieldPath::new(&["__resolved_primary_key"]),
                    dest: resolved_key_reg,
                    default: Some(serde_json::json!(null)),
                });

                // Copy to key_reg (unconditionally, may be null)
                ops.push(OpCode::CopyRegister {
                    source: resolved_key_reg,
                    dest: key_reg,
                });

                // If hook has lookup_by, use it to load primary key from instruction accounts
                if let Some(lookup_path) = &hook.lookup_by {
                    // Load the primary key from the instruction's lookup_by field (e.g., accounts.signer)
                    ops.push(OpCode::LoadEventField {
                        path: lookup_path.clone(),
                        dest: temp_reg,
                        default: None,
                    });

                    // Apply HexEncode transformation (accounts are byte arrays)
                    ops.push(OpCode::Transform {
                        source: temp_reg,
                        dest: temp_reg,
                        transformation: Transformation::HexEncode,
                    });

                    // Use this as fallback if __resolved_primary_key was null
                    ops.push(OpCode::CopyRegisterIfNull {
                        source: temp_reg,
                        dest: key_reg,
                    });
                }

                ops.push(OpCode::ReadOrInitState {
                    state_id: self.state_id,
                    key: key_reg,
                    default: serde_json::json!({}),
                    dest: state_reg,
                });

                ops.push(OpCode::UpdateState {
                    state_id: self.state_id,
                    key: key_reg,
                    value: state_reg,
                });

                ops
            });

            // Generate opcodes for each action in the hook
            let hook_opcodes = self.compile_instruction_hook_actions(&hook.actions);

            // Insert hook opcodes before EvaluateComputedFields (if present) or UpdateState
            // Hook actions (like whale_trade_count increment) must run before computed fields
            // are evaluated, since computed fields may depend on the modified state
            let insert_pos = handler_opcodes
                .iter()
                .position(|op| matches!(op, OpCode::EvaluateComputedFields { .. }))
                .or_else(|| {
                    handler_opcodes
                        .iter()
                        .position(|op| matches!(op, OpCode::UpdateState { .. }))
                });

            if let Some(pos) = insert_pos {
                // Insert hook opcodes before EvaluateComputedFields or UpdateState
                for (i, opcode) in hook_opcodes.into_iter().enumerate() {
                    handler_opcodes.insert(pos + i, opcode);
                }
            }
        }

        let non_emitted_fields: HashSet<String> = emit_by_path
            .into_iter()
            .filter_map(|(path, emit)| if emit { None } else { Some(path) })
            .collect();

        EntityBytecode {
            state_id: self.state_id,
            handlers,
            entity_name: self.entity_name.clone(),
            when_events,
            non_emitted_fields,
            computed_paths: self.spec.computed_fields.clone(),
            computed_fields_evaluator: None,
        }
    }

    fn compile_handler(&self, spec: &TypedHandlerSpec<S>) -> Vec<OpCode> {
        let mut ops = Vec::new();
        let state_reg = 2;
        let key_reg = 20;

        ops.extend(self.compile_key_loading(&spec.key_resolution, key_reg, &spec.mappings));

        // Guard: if key resolved to null on an account-state event, abort
        // early with empty mutations so process_event can queue the update
        // for later reprocessing.  Without this, downstream opcodes would
        // create a phantom entity keyed by null and produce non-empty
        // mutations that prevent queueing.
        let is_account_event = matches!(
            spec.source,
            SourceSpec::Source {
                is_account: true,
                ..
            }
        );
        ops.push(OpCode::AbortIfNullKey {
            key: key_reg,
            is_account_event,
        });

        ops.push(OpCode::ReadOrInitState {
            state_id: self.state_id,
            key: key_reg,
            default: serde_json::json!({}),
            dest: state_reg,
        });

        // Index updates must come AFTER ReadOrInitState so the state table exists.
        // ReadOrInitState lazily creates the state table via entry().or_insert_with(),
        // but index opcodes (UpdateLookupIndex, UpdateTemporalIndex, UpdatePdaReverseLookup)
        // use get_mut() which fails if the table doesn't exist yet.
        // This ordering also means stale/duplicate updates (caught by ReadOrInitState's
        // recency check) correctly skip index updates too.
        ops.extend(self.compile_temporal_index_update(
            &spec.key_resolution,
            key_reg,
            &spec.mappings,
        ));

        for mapping in &spec.mappings {
            ops.extend(self.compile_mapping(mapping, state_reg, key_reg));
        }

        ops.extend(self.compile_resolvers(state_reg, key_reg));

        // Evaluate computed fields after all mappings but before updating state
        ops.push(OpCode::EvaluateComputedFields {
            state: state_reg,
            computed_paths: self.spec.computed_fields.clone(),
        });

        ops.push(OpCode::UpdateState {
            state_id: self.state_id,
            key: key_reg,
            value: state_reg,
        });

        if spec.emit {
            ops.push(OpCode::EmitMutation {
                entity_name: self.entity_name.clone(),
                key: key_reg,
                state: state_reg,
            });
        }

        ops
    }

    fn compile_resolvers(&self, state_reg: Register, key_reg: Register) -> Vec<OpCode> {
        let mut ops = Vec::new();

        for resolver_spec in &self.spec.resolver_specs {
            let url_template = match &resolver_spec.resolver {
                ResolverType::Url(config) => match &config.url_source {
                    UrlSource::Template(parts) => Some(parts.clone()),
                    _ => None,
                },
                _ => None,
            };

            ops.push(OpCode::QueueResolver {
                state_id: self.state_id,
                entity_name: self.entity_name.clone(),
                resolver: resolver_spec.resolver.clone(),
                input_path: resolver_spec.input_path.clone(),
                input_value: resolver_spec.input_value.clone(),
                url_template,
                strategy: resolver_spec.strategy.clone(),
                extracts: resolver_spec.extracts.clone(),
                condition: resolver_spec.condition.clone(),
                schedule_at: resolver_spec.schedule_at.clone(),
                state: state_reg,
                key: key_reg,
            });
        }

        ops
    }

    fn compile_mapping(
        &self,
        mapping: &TypedFieldMapping<S>,
        state_reg: Register,
        key_reg: Register,
    ) -> Vec<OpCode> {
        let mut ops = Vec::new();
        let temp_reg = 10;

        ops.extend(self.compile_mapping_source(&mapping.source, temp_reg));

        if let Some(transform) = &mapping.transform {
            ops.push(OpCode::Transform {
                source: temp_reg,
                dest: temp_reg,
                transformation: transform.clone(),
            });
        }

        if let Some(stop_instruction) = &mapping.stop {
            if mapping.when.is_some() {
                tracing::warn!(
                    "#[map] stop and when both set for {}. Ignoring when.",
                    mapping.target_path
                );
            }
            if !matches!(mapping.population, PopulationStrategy::LastWrite)
                && !matches!(mapping.population, PopulationStrategy::Merge)
            {
                tracing::warn!(
                    "#[map] stop ignores population strategy {:?}",
                    mapping.population
                );
            }

            ops.push(OpCode::SetFieldUnlessStopped {
                object: state_reg,
                path: mapping.target_path.clone(),
                value: temp_reg,
                stop_field: stop_field_path(&mapping.target_path),
                stop_instruction: stop_instruction.clone(),
                entity_name: self.entity_name.clone(),
                key_reg,
            });
            return ops;
        }

        if let Some(when_instruction) = &mapping.when {
            if !matches!(mapping.population, PopulationStrategy::LastWrite)
                && !matches!(mapping.population, PopulationStrategy::Merge)
            {
                tracing::warn!(
                    "#[map] when ignores population strategy {:?}",
                    mapping.population
                );
            }
            let (condition_field, condition_op, condition_value) = mapping
                .condition
                .as_ref()
                .and_then(|cond| cond.parsed.as_ref())
                .and_then(|parsed| match parsed {
                    ParsedCondition::Comparison { field, op, value } => {
                        Some((Some(field.clone()), Some(op.clone()), Some(value.clone())))
                    }
                    ParsedCondition::Logical { .. } => {
                        tracing::warn!("Logical conditions not yet supported for #[map] when");
                        None
                    }
                })
                .unwrap_or((None, None, None));

            ops.push(OpCode::SetFieldWhen {
                object: state_reg,
                path: mapping.target_path.clone(),
                value: temp_reg,
                when_instruction: when_instruction.clone(),
                entity_name: self.entity_name.clone(),
                key_reg,
                condition_field,
                condition_op,
                condition_value,
            });
            return ops;
        }

        if let Some(condition) = &mapping.condition {
            if let Some(parsed) = &condition.parsed {
                match parsed {
                    ParsedCondition::Comparison {
                        field,
                        op,
                        value: cond_value,
                    } => {
                        if matches!(mapping.population, PopulationStrategy::LastWrite)
                            || matches!(mapping.population, PopulationStrategy::Merge)
                        {
                            ops.push(OpCode::ConditionalSetField {
                                object: state_reg,
                                path: mapping.target_path.clone(),
                                value: temp_reg,
                                condition_field: field.clone(),
                                condition_op: op.clone(),
                                condition_value: cond_value.clone(),
                            });
                            return ops;
                        }

                        if matches!(mapping.population, PopulationStrategy::Count) {
                            ops.push(OpCode::ConditionalIncrement {
                                object: state_reg,
                                path: mapping.target_path.clone(),
                                condition_field: field.clone(),
                                condition_op: op.clone(),
                                condition_value: cond_value.clone(),
                            });
                            return ops;
                        }

                        tracing::warn!(
                            "Conditional #[map] not supported for population strategy {:?}",
                            mapping.population
                        );
                    }
                    ParsedCondition::Logical { .. } => {
                        tracing::warn!("Logical conditions not yet supported for #[map]");
                    }
                }
            }
        }

        match &mapping.population {
            PopulationStrategy::Append => {
                ops.push(OpCode::AppendToArray {
                    object: state_reg,
                    path: mapping.target_path.clone(),
                    value: temp_reg,
                });
            }
            PopulationStrategy::LastWrite => {
                ops.push(OpCode::SetField {
                    object: state_reg,
                    path: mapping.target_path.clone(),
                    value: temp_reg,
                });
            }
            PopulationStrategy::SetOnce => {
                ops.push(OpCode::SetFieldIfNull {
                    object: state_reg,
                    path: mapping.target_path.clone(),
                    value: temp_reg,
                });
            }
            PopulationStrategy::Merge => {
                ops.push(OpCode::SetField {
                    object: state_reg,
                    path: mapping.target_path.clone(),
                    value: temp_reg,
                });
            }
            PopulationStrategy::Max => {
                ops.push(OpCode::SetFieldMax {
                    object: state_reg,
                    path: mapping.target_path.clone(),
                    value: temp_reg,
                });
            }
            PopulationStrategy::Sum => {
                ops.push(OpCode::SetFieldSum {
                    object: state_reg,
                    path: mapping.target_path.clone(),
                    value: temp_reg,
                });
            }
            PopulationStrategy::Count => {
                // Count doesn't need the value, just increment
                ops.push(OpCode::SetFieldIncrement {
                    object: state_reg,
                    path: mapping.target_path.clone(),
                });
            }
            PopulationStrategy::Min => {
                ops.push(OpCode::SetFieldMin {
                    object: state_reg,
                    path: mapping.target_path.clone(),
                    value: temp_reg,
                });
            }
            PopulationStrategy::UniqueCount => {
                // UniqueCount requires maintaining an internal set
                // The field stores the count, but we track unique values in a hidden set
                let set_name = format!("{}_unique_set", mapping.target_path);
                ops.push(OpCode::AddToUniqueSet {
                    state_id: self.state_id,
                    set_name,
                    value: temp_reg,
                    count_object: state_reg,
                    count_path: mapping.target_path.clone(),
                });
            }
        }

        ops
    }

    fn compile_mapping_source(&self, source: &MappingSource, dest: Register) -> Vec<OpCode> {
        match source {
            MappingSource::FromSource {
                path,
                default,
                transform,
            } => {
                let mut ops = vec![OpCode::LoadEventField {
                    path: path.clone(),
                    dest,
                    default: default.clone(),
                }];

                // Apply transform if specified in the source
                if let Some(transform_type) = transform {
                    ops.push(OpCode::Transform {
                        source: dest,
                        dest,
                        transformation: transform_type.clone(),
                    });
                }

                ops
            }
            MappingSource::Constant(val) => {
                vec![OpCode::LoadConstant {
                    value: val.clone(),
                    dest,
                }]
            }
            MappingSource::AsEvent { fields } => {
                let mut ops = Vec::new();

                if fields.is_empty() {
                    let event_data_reg = dest + 1;
                    ops.push(OpCode::LoadEventField {
                        path: FieldPath::new(&[]),
                        dest: event_data_reg,
                        default: Some(serde_json::json!({})),
                    });
                    ops.push(OpCode::CreateEvent {
                        dest,
                        event_value: event_data_reg,
                    });
                } else {
                    let data_obj_reg = dest + 1;
                    ops.push(OpCode::CreateObject { dest: data_obj_reg });

                    let mut field_registers = Vec::new();
                    let mut current_reg = dest + 2;

                    for field_source in fields.iter() {
                        if let MappingSource::FromSource {
                            path,
                            default,
                            transform,
                        } = &**field_source
                        {
                            ops.push(OpCode::LoadEventField {
                                path: path.clone(),
                                dest: current_reg,
                                default: default.clone(),
                            });

                            if let Some(transform_type) = transform {
                                ops.push(OpCode::Transform {
                                    source: current_reg,
                                    dest: current_reg,
                                    transformation: transform_type.clone(),
                                });
                            }

                            if let Some(field_name) = path.segments.last() {
                                field_registers.push((field_name.clone(), current_reg));
                            }
                            current_reg += 1;
                        }
                    }

                    if !field_registers.is_empty() {
                        ops.push(OpCode::SetFields {
                            object: data_obj_reg,
                            fields: field_registers,
                        });
                    }

                    ops.push(OpCode::CreateEvent {
                        dest,
                        event_value: data_obj_reg,
                    });
                }

                ops
            }
            MappingSource::WholeSource => {
                vec![OpCode::LoadEventField {
                    path: FieldPath::new(&[]),
                    dest,
                    default: Some(serde_json::json!({})),
                }]
            }
            MappingSource::AsCapture { field_transforms } => {
                // AsCapture loads the whole source, applies field-level transforms, and wraps in CaptureWrapper
                let capture_data_reg = 22; // Temp register for capture data before wrapping
                let mut ops = vec![OpCode::LoadEventField {
                    path: FieldPath::new(&[]),
                    dest: capture_data_reg,
                    default: Some(serde_json::json!({})),
                }];

                // Apply transforms to specific fields in the loaded object
                // IMPORTANT: Use registers that don't conflict with key_reg (20)
                // Using 24 and 25 to avoid conflicts with key loading (uses 18, 19, 20, 23)
                let field_reg = 24;
                let transformed_reg = 25;

                for (field_name, transform) in field_transforms {
                    // Load the field from the capture_data_reg (not from event!)
                    // Use GetField opcode to read from a register instead of LoadEventField
                    ops.push(OpCode::GetField {
                        object: capture_data_reg,
                        path: field_name.clone(),
                        dest: field_reg,
                    });

                    // Transform it
                    ops.push(OpCode::Transform {
                        source: field_reg,
                        dest: transformed_reg,
                        transformation: transform.clone(),
                    });

                    // Set it back into the capture data object
                    ops.push(OpCode::SetField {
                        object: capture_data_reg,
                        path: field_name.clone(),
                        value: transformed_reg,
                    });
                }

                // Wrap the capture data in CaptureWrapper with metadata
                ops.push(OpCode::CreateCapture {
                    dest,
                    capture_value: capture_data_reg,
                });

                ops
            }
            MappingSource::FromContext { field } => {
                // Load from instruction context (timestamp, slot, signature)
                vec![OpCode::LoadEventField {
                    path: FieldPath::new(&["__update_context", field.as_str()]),
                    dest,
                    default: Some(serde_json::json!(null)),
                }]
            }
            MappingSource::Computed { .. } => {
                vec![]
            }
            MappingSource::FromState { .. } => {
                vec![]
            }
        }
    }

    pub fn compile_key_loading(
        &self,
        resolution: &KeyResolutionStrategy,
        key_reg: Register,
        mappings: &[TypedFieldMapping<S>],
    ) -> Vec<OpCode> {
        let mut ops = Vec::new();

        // First, try to load __resolved_primary_key from resolver
        // This allows resolvers to override the key resolution
        let resolved_key_reg = 19; // Use a temp register
        ops.push(OpCode::LoadEventField {
            path: FieldPath::new(&["__resolved_primary_key"]),
            dest: resolved_key_reg,
            default: Some(serde_json::json!(null)),
        });

        // Now do the normal key resolution
        match resolution {
            KeyResolutionStrategy::Embedded { primary_field } => {
                // Copy resolver result to key_reg (may be null)
                ops.push(OpCode::CopyRegister {
                    source: resolved_key_reg,
                    dest: key_reg,
                });

                // Enhanced key resolution: check for auto-inheritance when primary_field is empty
                let effective_primary_field = if primary_field.segments.is_empty() {
                    // Try to auto-detect primary field from account schema
                    if let Some(auto_field) = self.auto_detect_primary_field(mappings) {
                        auto_field
                    } else {
                        primary_field.clone()
                    }
                } else {
                    primary_field.clone()
                };

                // Skip fallback key loading if effective primary_field is still empty
                // This happens for account types that rely solely on __resolved_primary_key
                // (e.g., accounts with #[resolve_key_for] resolvers)
                if !effective_primary_field.segments.is_empty() {
                    let temp_reg = 18;
                    let transform_reg = 23; // Register for transformed key

                    ops.push(OpCode::LoadEventField {
                        path: effective_primary_field.clone(),
                        dest: temp_reg,
                        default: None,
                    });

                    // Check if there's a transformation for the primary key field
                    // First try the current mappings, then inherited transformations
                    let primary_key_transform = self
                        .find_primary_key_transformation(mappings)
                        .or_else(|| self.find_inherited_primary_key_transformation());

                    if let Some(transform) = primary_key_transform {
                        // Apply transformation to the loaded key
                        ops.push(OpCode::Transform {
                            source: temp_reg,
                            dest: transform_reg,
                            transformation: transform,
                        });
                        // Use transformed value as key
                        ops.push(OpCode::CopyRegisterIfNull {
                            source: transform_reg,
                            dest: key_reg,
                        });
                    } else {
                        // No transformation, use raw value
                        ops.push(OpCode::CopyRegisterIfNull {
                            source: temp_reg,
                            dest: key_reg,
                        });
                    }
                }
                // If effective_primary_field is empty, key_reg will only contain __resolved_primary_key
                // (loaded earlier at line 513-522), or remain null if resolver didn't set it
            }
            KeyResolutionStrategy::Lookup { primary_field } => {
                let lookup_reg = 15;
                let result_reg = 17;

                // Prefer resolver-provided key as lookup input.
                // When __resolved_primary_key is set (e.g. round_address from
                // PDA reverse lookup), use it directly — this gives a one-hop
                // lookup (round_address → round_id) instead of a two-hop chain
                // (Var address → PDA → round_address → round_id).
                ops.push(OpCode::CopyRegister {
                    source: resolved_key_reg,
                    dest: lookup_reg,
                });

                let temp_reg = 18;
                ops.push(OpCode::LoadEventField {
                    path: primary_field.clone(),
                    dest: temp_reg,
                    default: None,
                });
                ops.push(OpCode::CopyRegisterIfNull {
                    source: temp_reg,
                    dest: lookup_reg,
                });

                let index_name = self.find_lookup_index_for_lookup_field(primary_field, mappings);
                let effective_index_name =
                    index_name.unwrap_or_else(|| "default_pda_lookup".to_string());

                ops.push(OpCode::LookupIndex {
                    state_id: self.state_id,
                    index_name: effective_index_name,
                    lookup_value: lookup_reg,
                    dest: result_reg,
                });
                // CRITICAL: For Lookup resolution, we ONLY use the lookup result.
                // If the lookup fails (result_reg is null), the mutation will be skipped.
                // We do NOT fall back to __resolved_primary_key or the raw lookup value,
                // because that would create a separate entity with the wrong key.
                // The resolver-provided key (e.g., PDA address) is only used as the lookup input,
                // NOT as the entity key. The entity key must come from the lookup index.
                ops.push(OpCode::CopyRegister {
                    source: result_reg,
                    dest: key_reg,
                });
                // NOTE: We intentionally do NOT fall back to lookup_reg when LookupIndex returns null.
                // If the lookup fails (because the RoundState account hasn't been processed yet),
                // the result_reg will remain null, and the mutation will be skipped.
                // Previously we had: CopyRegisterIfNull { source: lookup_reg, dest: result_reg }
                // which caused the PDA address to be used as the key instead of the round_id.
                // This resulted in mutations with key = PDA address instead of key = primary_key.

                // Use LookupIndex result as the primary key. Do NOT fall back to
                // resolved_key_reg (__resolved_primary_key) because for Lookup handlers
                // it contains the PDA reverse-lookup result (e.g. round_address), which
                // is an intermediate value, not the actual primary key (e.g. round_id).
                // If the LookupIndex chain returns null (round not yet indexed), key
                // stays null so the update is queued and reprocessed once the lookup
                // index is populated—matching the pre-23503ac behaviour.
                ops.push(OpCode::CopyRegister {
                    source: result_reg,
                    dest: key_reg,
                });
            }
            KeyResolutionStrategy::Computed {
                primary_field,
                compute_partition: _,
            } => {
                // Copy resolver result to key_reg (may be null)
                ops.push(OpCode::CopyRegister {
                    source: resolved_key_reg,
                    dest: key_reg,
                });
                let temp_reg = 18;
                ops.push(OpCode::LoadEventField {
                    path: primary_field.clone(),
                    dest: temp_reg,
                    default: None,
                });
                ops.push(OpCode::CopyRegisterIfNull {
                    source: temp_reg,
                    dest: key_reg,
                });
            }
            KeyResolutionStrategy::TemporalLookup {
                lookup_field,
                timestamp_field,
                index_name,
            } => {
                // Copy resolver result to key_reg (may be null)
                ops.push(OpCode::CopyRegister {
                    source: resolved_key_reg,
                    dest: key_reg,
                });
                let lookup_reg = 15;
                let timestamp_reg = 16;
                let result_reg = 17;

                ops.push(OpCode::LoadEventField {
                    path: lookup_field.clone(),
                    dest: lookup_reg,
                    default: None,
                });

                ops.push(OpCode::LoadEventField {
                    path: timestamp_field.clone(),
                    dest: timestamp_reg,
                    default: None,
                });

                ops.push(OpCode::LookupTemporalIndex {
                    state_id: self.state_id,
                    index_name: index_name.clone(),
                    lookup_value: lookup_reg,
                    timestamp: timestamp_reg,
                    dest: result_reg,
                });

                ops.push(OpCode::CopyRegisterIfNull {
                    source: result_reg,
                    dest: key_reg,
                });
            }
        }

        ops
    }

    fn find_primary_key_transformation(
        &self,
        mappings: &[TypedFieldMapping<S>],
    ) -> Option<Transformation> {
        // Find the first primary key in the identity spec
        let primary_key = self.spec.identity.primary_keys.first()?;
        let primary_field_name = self.extract_primary_field_name(primary_key)?;

        // Look for a mapping that targets this primary key
        for mapping in mappings {
            // Check if this mapping targets the primary key field
            if mapping.target_path == *primary_key
                || mapping.target_path.ends_with(&format!(".{}", primary_key))
            {
                // Check mapping-level transform first
                if let Some(transform) = &mapping.transform {
                    return Some(transform.clone());
                }

                // Then check source-level transform
                if let MappingSource::FromSource {
                    transform: Some(transform),
                    ..
                } = &mapping.source
                {
                    return Some(transform.clone());
                }
            }
        }

        // If no explicit primary key mapping found, check AsCapture field transforms
        for mapping in mappings {
            if let MappingSource::AsCapture { field_transforms } = &mapping.source {
                if let Some(transform) = field_transforms.get(&primary_field_name) {
                    return Some(transform.clone());
                }
            }
        }

        None
    }

    /// Look for primary key mappings in other handlers of the same entity
    /// This enables cross-handler inheritance of key transformations
    pub fn find_inherited_primary_key_transformation(&self) -> Option<Transformation> {
        let primary_key = self.spec.identity.primary_keys.first()?;

        // Extract the field name from the primary key path (e.g., "id.authority" -> "authority")
        let primary_field_name = self.extract_primary_field_name(primary_key)?;

        // Search through all handlers in the spec for primary key mappings
        for handler in &self.spec.handlers {
            for mapping in &handler.mappings {
                // Look for mappings targeting the primary key
                if mapping.target_path == *primary_key
                    || mapping.target_path.ends_with(&format!(".{}", primary_key))
                {
                    // Check if this mapping comes from a field matching the primary key name
                    if let MappingSource::FromSource {
                        path, transform, ..
                    } = &mapping.source
                    {
                        if path.segments.last() == Some(&primary_field_name) {
                            // Return mapping-level transform first, then source-level transform
                            return mapping.transform.clone().or_else(|| transform.clone());
                        }
                    }
                }

                // Also check AsCapture field transforms for the primary field
                if let MappingSource::AsCapture { field_transforms } = &mapping.source {
                    if let Some(transform) = field_transforms.get(&primary_field_name) {
                        return Some(transform.clone());
                    }
                }
            }
        }

        None
    }

    /// Extract the field name from a primary key path (e.g., "id.authority" -> "authority")
    fn extract_primary_field_name(&self, primary_key: &str) -> Option<String> {
        // Split by '.' and take the last segment
        primary_key.split('.').next_back().map(|s| s.to_string())
    }

    /// Auto-detect primary field from account schema when no explicit mapping exists
    /// This looks for account types that have an 'authority' field and tries to use it
    pub fn auto_detect_primary_field(
        &self,
        current_mappings: &[TypedFieldMapping<S>],
    ) -> Option<FieldPath> {
        let primary_key = self.spec.identity.primary_keys.first()?;

        // Extract the field name from the primary key (e.g., "id.authority" -> "authority")
        let primary_field_name = self.extract_primary_field_name(primary_key)?;

        // Check if current handler can access the primary field
        if self.current_account_has_primary_field(&primary_field_name, current_mappings) {
            return Some(FieldPath::new(&[&primary_field_name]));
        }

        None
    }

    /// Check if the current account type has the primary field
    /// This is determined by looking at the mappings to see what fields are available
    fn current_account_has_primary_field(
        &self,
        field_name: &str,
        mappings: &[TypedFieldMapping<S>],
    ) -> bool {
        // Look through the mappings to see if any reference the primary field
        for mapping in mappings {
            if let MappingSource::FromSource { path, .. } = &mapping.source {
                // Check if this mapping sources from the primary field
                if path.segments.last() == Some(&field_name.to_string()) {
                    return true;
                }
            }
        }

        false
    }

    /// Check if handler has access to a specific field in its source account
    #[allow(dead_code)]
    fn handler_has_field(&self, field_name: &str, mappings: &[TypedFieldMapping<S>]) -> bool {
        for mapping in mappings {
            if let MappingSource::FromSource { path, .. } = &mapping.source {
                if path.segments.last() == Some(&field_name.to_string()) {
                    return true;
                }
            }
        }
        false
    }

    /// Check if field exists by looking at mappings (IDL-agnostic approach)
    /// This avoids hardcoding account schemas and uses actual mapping evidence
    #[allow(dead_code)]
    fn field_exists_in_mappings(
        &self,
        field_name: &str,
        mappings: &[TypedFieldMapping<S>],
    ) -> bool {
        // Look through current mappings to see if the field is referenced
        for mapping in mappings {
            if let MappingSource::FromSource { path, .. } = &mapping.source {
                if path.segments.last() == Some(&field_name.to_string()) {
                    return true;
                }
            }
            // Also check AsCapture field transforms
            if let MappingSource::AsCapture { field_transforms } = &mapping.source {
                if field_transforms.contains_key(field_name) {
                    return true;
                }
            }
        }
        false
    }

    fn find_lookup_index_for_field(&self, field_path: &FieldPath) -> Option<String> {
        if field_path.segments.is_empty() {
            return None;
        }

        let lookup_field_name = field_path.segments.last().unwrap();

        for lookup_index in &self.spec.identity.lookup_indexes {
            let index_field_name = lookup_index
                .field_name
                .split('.')
                .next_back()
                .unwrap_or(&lookup_index.field_name);
            let matches_directly = index_field_name == lookup_field_name;
            // An index field named `foo_address` is treated as an alias for the
            // bare field `foo` (for example, `mint_address` resolves handlers
            // keyed on `mint`). This is intentionally one-way: bare `foo` does
            // not imply a `foo_address` lookup index. The macro crate mirrors
            // this convention via `lookup_index_leafs` in
            // `hyperstack-macros/src/validation/mod.rs`.
            let matches_address_alias = index_field_name
                .strip_suffix("_address")
                .map(|base| base == lookup_field_name)
                .unwrap_or(false);

            if matches_directly || matches_address_alias {
                return Some(format!("{}_lookup_index", index_field_name));
            }
        }

        None
    }

    /// Find lookup index for a Lookup key resolution by checking if there's a mapping
    /// from the primary_field to a lookup index field.
    fn find_lookup_index_for_lookup_field(
        &self,
        primary_field: &FieldPath,
        mappings: &[TypedFieldMapping<S>],
    ) -> Option<String> {
        // Build the primary field path string
        let primary_path = primary_field.segments.join(".");

        // Check if there's a mapping from this primary field to a lookup index field
        for mapping in mappings {
            // Check if the mapping source path matches the primary field
            if let MappingSource::FromSource { path, .. } = &mapping.source {
                let source_path = path.segments.join(".");
                if source_path == primary_path {
                    // Check if the target is a lookup index field
                    for lookup_index in &self.spec.identity.lookup_indexes {
                        if mapping.target_path == lookup_index.field_name {
                            let index_field_name = lookup_index
                                .field_name
                                .split('.')
                                .next_back()
                                .unwrap_or(&lookup_index.field_name);
                            return Some(format!("{}_lookup_index", index_field_name));
                        }
                    }
                }
            }
        }

        // Fall back to direct field name matching
        self.find_lookup_index_for_field(primary_field)
    }

    /// Find the source path for a lookup index field by looking at mappings.
    /// For example, if target_path is "id.round_address" and the mapping is
    /// `id.round_address <- __account_address`, this returns ["__account_address"].
    fn find_source_path_for_lookup_index(
        &self,
        mappings: &[TypedFieldMapping<S>],
        lookup_field_name: &str,
    ) -> Option<Vec<String>> {
        for mapping in mappings {
            if mapping.target_path == lookup_field_name {
                if let MappingSource::FromSource { path, .. } = &mapping.source {
                    return Some(path.segments.clone());
                }
            }
        }
        None
    }

    fn compile_temporal_index_update(
        &self,
        resolution: &KeyResolutionStrategy,
        key_reg: Register,
        mappings: &[TypedFieldMapping<S>],
    ) -> Vec<OpCode> {
        let mut ops = Vec::new();

        for lookup_index in &self.spec.identity.lookup_indexes {
            let lookup_reg = 17;
            let source_field = lookup_index
                .field_name
                .split('.')
                .next_back()
                .unwrap_or(&lookup_index.field_name);

            match resolution {
                KeyResolutionStrategy::Embedded { primary_field: _ } => {
                    // For Embedded handlers, find the mapping that targets this lookup index field
                    // and use its source path to load the lookup value
                    let source_path_opt =
                        self.find_source_path_for_lookup_index(mappings, &lookup_index.field_name);

                    let load_path = if let Some(ref path) = source_path_opt {
                        FieldPath::new(&path.iter().map(|s| s.as_str()).collect::<Vec<_>>())
                    } else {
                        // Fallback to source_field if no mapping found
                        FieldPath::new(&[source_field])
                    };

                    ops.push(OpCode::LoadEventField {
                        path: load_path,
                        dest: lookup_reg,
                        default: None,
                    });

                    if let Some(temporal_field_name) = &lookup_index.temporal_field {
                        let timestamp_reg = 18;

                        ops.push(OpCode::LoadEventField {
                            path: FieldPath::new(&[temporal_field_name]),
                            dest: timestamp_reg,
                            default: None,
                        });

                        let index_name = format!("{}_temporal_index", source_field);
                        ops.push(OpCode::UpdateTemporalIndex {
                            state_id: self.state_id,
                            index_name,
                            lookup_value: lookup_reg,
                            primary_key: key_reg,
                            timestamp: timestamp_reg,
                        });

                        let simple_index_name = format!("{}_lookup_index", source_field);
                        ops.push(OpCode::UpdateLookupIndex {
                            state_id: self.state_id,
                            index_name: simple_index_name,
                            lookup_value: lookup_reg,
                            primary_key: key_reg,
                        });
                    } else {
                        let index_name = format!("{}_lookup_index", source_field);
                        ops.push(OpCode::UpdateLookupIndex {
                            state_id: self.state_id,
                            index_name,
                            lookup_value: lookup_reg,
                            primary_key: key_reg,
                        });
                    }

                    // Also update PDA reverse lookup table if there's a resolver configured for this entity
                    // This allows instruction handlers to look up the primary key from PDA addresses
                    // Only do this when the source path is different (e.g., __account_address -> id.round_address)
                    if source_path_opt.is_some() {
                        ops.push(OpCode::UpdatePdaReverseLookup {
                            state_id: self.state_id,
                            lookup_name: "default_pda_lookup".to_string(),
                            pda_address: lookup_reg,
                            primary_key: key_reg,
                        });
                    }
                }
                KeyResolutionStrategy::Lookup { primary_field } => {
                    // For Lookup handlers, check if there's a mapping that targets this lookup index field
                    // If so, the lookup value is the same as the primary_field used for key resolution
                    let has_mapping_to_lookup_field = mappings
                        .iter()
                        .any(|m| m.target_path == lookup_index.field_name);

                    if has_mapping_to_lookup_field {
                        // Load the lookup value from the event using the primary_field path
                        // (this is the same value used for key resolution)
                        let path_segments: Vec<&str> =
                            primary_field.segments.iter().map(|s| s.as_str()).collect();
                        ops.push(OpCode::LoadEventField {
                            path: FieldPath::new(&path_segments),
                            dest: lookup_reg,
                            default: None,
                        });

                        let index_name = format!("{}_lookup_index", source_field);
                        ops.push(OpCode::UpdateLookupIndex {
                            state_id: self.state_id,
                            index_name,
                            lookup_value: lookup_reg,
                            primary_key: key_reg,
                        });
                    }
                }
                KeyResolutionStrategy::Computed { .. }
                | KeyResolutionStrategy::TemporalLookup { .. } => {
                    // Computed and TemporalLookup handlers don't populate lookup indexes
                }
            }
        }

        ops
    }

    fn get_event_type(&self, source: &SourceSpec) -> String {
        match source {
            SourceSpec::Source { type_name, .. } => type_name.clone(),
        }
    }

    fn compile_instruction_hook_actions(&self, actions: &[HookAction]) -> Vec<OpCode> {
        let mut ops = Vec::new();
        let state_reg = 2;

        for action in actions {
            match action {
                HookAction::SetField {
                    target_field,
                    source,
                    condition,
                } => {
                    // Check if there's a condition - evaluation handled in VM
                    let _ = condition;

                    let temp_reg = 11; // Use register 11 for hook values

                    // Load the source value
                    let load_ops = self.compile_mapping_source(source, temp_reg);
                    ops.extend(load_ops);

                    // Apply transformation if specified in source
                    if let MappingSource::FromSource {
                        transform: Some(transform_type),
                        ..
                    } = source
                    {
                        ops.push(OpCode::Transform {
                            source: temp_reg,
                            dest: temp_reg,
                            transformation: transform_type.clone(),
                        });
                    }

                    // Conditionally set the field based on parsed condition
                    if let Some(cond_expr) = condition {
                        if let Some(parsed) = &cond_expr.parsed {
                            // Generate condition check opcodes
                            let cond_check_ops = self.compile_condition_check(
                                parsed,
                                temp_reg,
                                state_reg,
                                target_field,
                            );
                            ops.extend(cond_check_ops);
                        } else {
                            // No parsed condition, set unconditionally
                            ops.push(OpCode::SetField {
                                object: state_reg,
                                path: target_field.clone(),
                                value: temp_reg,
                            });
                        }
                    } else {
                        // No condition, set unconditionally
                        ops.push(OpCode::SetField {
                            object: state_reg,
                            path: target_field.clone(),
                            value: temp_reg,
                        });
                    }
                }
                HookAction::IncrementField {
                    target_field,
                    increment_by,
                    condition,
                } => {
                    if let Some(cond_expr) = condition {
                        if let Some(parsed) = &cond_expr.parsed {
                            // For increment with condition, we need to:
                            // 1. Load the condition field
                            // 2. Check the condition
                            // 3. Conditionally increment
                            let cond_check_ops = self.compile_conditional_increment(
                                parsed,
                                state_reg,
                                target_field,
                                *increment_by,
                            );
                            ops.extend(cond_check_ops);
                        } else {
                            // No parsed condition, increment unconditionally
                            ops.push(OpCode::SetFieldIncrement {
                                object: state_reg,
                                path: target_field.clone(),
                            });
                        }
                    } else {
                        // No condition, increment unconditionally
                        ops.push(OpCode::SetFieldIncrement {
                            object: state_reg,
                            path: target_field.clone(),
                        });
                    }
                }
                HookAction::RegisterPdaMapping { .. } => {
                    // PDA registration is handled elsewhere (in resolvers)
                    // Skip for now
                }
            }
        }

        ops
    }

    fn compile_condition_check(
        &self,
        condition: &ParsedCondition,
        value_reg: Register,
        state_reg: Register,
        target_field: &str,
    ) -> Vec<OpCode> {
        match condition {
            ParsedCondition::Comparison {
                field,
                op,
                value: cond_value,
            } => {
                // Generate ConditionalSetField opcode
                vec![OpCode::ConditionalSetField {
                    object: state_reg,
                    path: target_field.to_string(),
                    value: value_reg,
                    condition_field: field.clone(),
                    condition_op: op.clone(),
                    condition_value: cond_value.clone(),
                }]
            }
            ParsedCondition::Logical { .. } => {
                // Logical conditions not yet supported, fall back to unconditional
                tracing::warn!("Logical conditions not yet supported in instruction hooks");
                vec![OpCode::SetField {
                    object: state_reg,
                    path: target_field.to_string(),
                    value: value_reg,
                }]
            }
        }
    }

    fn compile_conditional_increment(
        &self,
        condition: &ParsedCondition,
        state_reg: Register,
        target_field: &str,
        _increment_by: i64,
    ) -> Vec<OpCode> {
        match condition {
            ParsedCondition::Comparison {
                field,
                op,
                value: cond_value,
            } => {
                vec![OpCode::ConditionalIncrement {
                    object: state_reg,
                    path: target_field.to_string(),
                    condition_field: field.clone(),
                    condition_op: op.clone(),
                    condition_value: cond_value.clone(),
                }]
            }
            ParsedCondition::Logical { .. } => {
                tracing::warn!("Logical conditions not yet supported in instruction hooks");
                vec![OpCode::SetFieldIncrement {
                    object: state_reg,
                    path: target_field.to_string(),
                }]
            }
        }
    }
}
