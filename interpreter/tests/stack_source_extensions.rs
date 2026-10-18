//! Write guards, string and byte functions, sibling registrations and the
//! PDA reverse lookup capacity, exercised by an instance entity fed by an
//! attested memo and the program instruction that shares its transaction:
//! the memo names the instance, and its `contribute` sibling's position
//! account is registered to that instance so the contribution finds the row.

use std::collections::BTreeMap;

use arete_interpreter::ast::{
    BinaryOp, ComputedExpr, ConditionExpr, FieldPath, HookAction, IdentitySpec, InstructionHook,
    KeyResolutionStrategy, LookupIndexSpec, MappingSource, ParsedCondition, PopulationStrategy,
    SerializableFieldMapping, SerializableHandlerSpec, SerializableStreamSpec, SiblingSelector,
    SourceSpec, TypedStreamSpec, CURRENT_AST_VERSION,
};
use arete_interpreter::compiler::MultiEntityBytecode;
use arete_interpreter::vm::{StateTableConfig, UpdateContext, VmContext};
use serde_json::{json, Value};

const ENTITY: &str = "Instance";
const MEMO: &str = "spl_memo::MemoIxState";
const CONTRIBUTE: &str = "raise::ContributeIxState";
const ATTESTATION: &str = "AttestationKey111111111111111111111111111111";
const LOOKUP: &str = "instance_accounts";

fn field(path: &str) -> ComputedExpr {
    ComputedExpr::FieldRef {
        path: path.to_string(),
    }
}

fn literal(value: Value) -> ComputedExpr {
    ComputedExpr::Literal { value }
}

fn binary(op: BinaryOp, left: ComputedExpr, right: ComputedExpr) -> ComputedExpr {
    ComputedExpr::Binary {
        op,
        left: Box::new(left),
        right: Box::new(right),
    }
}

fn method(receiver: ComputedExpr, name: &str, args: Vec<ComputedExpr>) -> ComputedExpr {
    ComputedExpr::MethodCall {
        expr: Box::new(receiver),
        method: name.to_string(),
        args,
    }
}

/// `split_part(data.memo, ":", 3)`: the instance id of `arete:p2:<id>:...`.
fn instance_id() -> ComputedExpr {
    method(
        field("data.memo"),
        "split_part",
        vec![literal(json!(":")), literal(json!(3))],
    )
}

/// `starts_with(data.memo, "arete:p2:") && (accounts.signer == ATTESTATION
/// || accounts.signer == "BackupKey")`.
fn attested() -> ComputedExpr {
    binary(
        BinaryOp::And,
        method(
            field("data.memo"),
            "starts_with",
            vec![literal(json!("arete:p2:"))],
        ),
        ComputedExpr::Paren {
            expr: Box::new(binary(
                BinaryOp::Or,
                binary(
                    BinaryOp::Eq,
                    field("accounts.signer"),
                    literal(json!(ATTESTATION)),
                ),
                binary(
                    BinaryOp::Eq,
                    field("accounts.signer"),
                    literal(json!("BackupKey")),
                ),
            )),
        },
    )
}

fn mapping(
    target: &str,
    source: MappingSource,
    population: PopulationStrategy,
) -> SerializableFieldMapping {
    SerializableFieldMapping {
        target_path: target.to_string(),
        source,
        transform: None,
        population,
        condition: None,
        when: None,
        stop: None,
        emit: true,
    }
}

fn guarded(mut mapping: SerializableFieldMapping) -> SerializableFieldMapping {
    mapping.condition = Some(ConditionExpr {
        expression: "attested".to_string(),
        parsed: Some(ParsedCondition::Guard { expr: attested() }),
    });
    mapping
}

fn from(path: &[&str]) -> MappingSource {
    MappingSource::FromSource {
        path: FieldPath::new(path),
        default: None,
        transform: None,
    }
}

fn source(type_name: &str) -> SourceSpec {
    SourceSpec::Source {
        program_id: None,
        discriminator: None,
        type_name: type_name.to_string(),
        serialization: None,
        is_account: false,
    }
}

fn instance_spec(registration: HookAction) -> TypedStreamSpec<Value> {
    use PopulationStrategy::{Append, Count, LastWrite, SetOnce, Sum};
    let memo = SerializableHandlerSpec {
        source: source(MEMO),
        // The key mapping's expression is the key.
        key_resolution: KeyResolutionStrategy::Embedded {
            primary_field: FieldPath::new(&[]),
        },
        mappings: vec![
            mapping(
                "id.instance",
                MappingSource::FromExpr {
                    expr: instance_id(),
                },
                SetOnce,
            ),
            mapping("memos.seen", from(&["data", "memo"]), Count),
            guarded(mapping("memos.attested", from(&["data", "memo"]), Count)),
            guarded(mapping("memos.last", from(&["data", "memo"]), LastWrite)),
            guarded(mapping(
                "memos.log",
                MappingSource::AsEvent {
                    fields: vec![Box::new(from(&["data", "memo"]))],
                },
                Append,
            )),
        ],
        conditions: vec![],
        emit: true,
    };
    let contribute = SerializableHandlerSpec {
        source: source(CONTRIBUTE),
        key_resolution: KeyResolutionStrategy::Lookup {
            primary_field: FieldPath::new(&["accounts", "position"]),
            index_name: None,
        },
        mappings: vec![mapping("raised", from(&["data", "amount"]), Sum)],
        conditions: vec![],
        emit: true,
    };
    TypedStreamSpec::from_serializable(SerializableStreamSpec {
        ast_version: CURRENT_AST_VERSION.to_string(),
        state_name: ENTITY.to_string(),
        program_id: None,
        idl: None,
        // A PDA reverse lookup resolves to an index value, so the
        // registered seed (the instance id) is indexed.
        identity: IdentitySpec {
            primary_keys: vec!["id.instance".to_string()],
            lookup_indexes: vec![LookupIndexSpec {
                field_name: "id.instance".to_string(),
                temporal_field: None,
            }],
        },
        handlers: vec![memo, contribute],
        sections: vec![],
        field_mappings: BTreeMap::new(),
        resolver_hooks: vec![],
        instruction_hooks: vec![InstructionHook {
            instruction_type: MEMO.to_string(),
            actions: vec![registration],
            lookup_by: None,
        }],
        resolver_specs: vec![],
        computed_fields: vec![],
        computed_field_specs: vec![],
        content_hash: None,
        views: vec![],
        scope_field: None,
    })
}

/// Register the `contribute` sibling's position account to the memo's
/// instance, only for an attested memo.
fn sibling_registration() -> HookAction {
    HookAction::RegisterPdaMapping {
        pda_field: FieldPath::new(&["accounts", "position"]),
        seed_field: FieldPath::new(&["data", "memo"]),
        lookup_name: LOOKUP.to_string(),
        sibling: Some(SiblingSelector {
            instruction_type: CONTRIBUTE.to_string(),
            pda_from_sibling: true,
            seed_from_sibling: false,
        }),
        seed_expr: Some(instance_id()),
        guard: Some(binary(
            BinaryOp::Eq,
            field("accounts.signer"),
            literal(json!(ATTESTATION)),
        )),
    }
}

fn bytecode() -> MultiEntityBytecode {
    MultiEntityBytecode::new()
        .add_entity(ENTITY.to_string(), instance_spec(sibling_registration()), 0)
        .build()
}

/// An instruction at `ix_path` of transaction `signature`; each transaction
/// gets its own index in the slot.
fn context(signature: &str, ix_path: &str) -> UpdateContext {
    let txn_index = signature.bytes().map(u64::from).sum();
    UpdateContext::new_instruction(300_000_000, signature.to_string(), txn_index)
        .at_instruction(ix_path.to_string())
}

fn memo(
    vm: &mut VmContext,
    bytecode: &MultiEntityBytecode,
    signature: &str,
    text: &str,
    signer: &str,
) {
    vm.process_event(
        bytecode,
        json!({"data": {"memo": text}, "accounts": {"signer": signer}}),
        MEMO,
        Some(&context(signature, "1")),
        None,
    )
    .unwrap();
}

fn contribute(
    vm: &mut VmContext,
    bytecode: &MultiEntityBytecode,
    signature: &str,
    position: &str,
    amount: u64,
) {
    vm.process_event(
        bytecode,
        json!({"data": {"amount": amount}, "accounts": {"position": position}}),
        CONTRIBUTE,
        Some(&context(signature, "0")),
        None,
    )
    .unwrap();
}

fn instance(vm: &VmContext, id: &str) -> Value {
    vm.get_entity_state(0, &json!(id))
        .unwrap_or_else(|| panic!("no instance {id}"))
}

#[test]
fn guards_gate_every_strategy_and_honour_and_or() {
    let bytecode = bytecode();
    let mut vm = VmContext::new();
    memo(
        &mut vm,
        &bytecode,
        "sig-1",
        "arete:p2:inst-1:-:a",
        ATTESTATION,
    );
    memo(
        &mut vm,
        &bytecode,
        "sig-2",
        "arete:p2:inst-1:-:b",
        "Impostor",
    );
    memo(
        &mut vm,
        &bytecode,
        "sig-3",
        "arete:p2:inst-1:-:c",
        "BackupKey",
    );
    memo(
        &mut vm,
        &bytecode,
        "sig-4",
        "hello:p2:inst-1:-:d",
        ATTESTATION,
    );

    let row = instance(&vm, "inst-1");
    assert_eq!(row["memos"]["seen"], json!(4), "unguarded count");
    assert_eq!(row["memos"]["attested"], json!(2), "guarded count");
    assert_eq!(row["memos"]["last"], json!("arete:p2:inst-1:-:c"));
    let log = row["memos"]["log"].as_array().unwrap();
    assert_eq!(
        log.iter()
            .map(|entry| entry["data"]["memo"].clone())
            .collect::<Vec<_>>(),
        vec![json!("arete:p2:inst-1:-:a"), json!("arete:p2:inst-1:-:c")]
    );
    // `hello:p2:inst-1` names instance `inst-1` too but fails the guard;
    // only the unguarded count saw it.
}

#[test]
fn a_sibling_that_arrives_after_the_memo_is_registered_to_its_instance() {
    let bytecode = bytecode();
    let mut vm = VmContext::new();
    memo(
        &mut vm,
        &bytecode,
        "sig-1",
        "arete:p2:inst-1:-:a",
        ATTESTATION,
    );
    contribute(&mut vm, &bytecode, "sig-1", "Position1", 40);
    assert_eq!(instance(&vm, "inst-1")["raised"], json!(40));

    // Registered for good: a later direct contribution routes too.
    contribute(&mut vm, &bytecode, "sig-9", "Position1", 2);
    assert_eq!(instance(&vm, "inst-1")["raised"], json!(42));
}

#[test]
fn a_sibling_that_arrives_before_the_memo_is_replayed_once_registered() {
    let bytecode = bytecode();
    let mut vm = VmContext::new();
    contribute(&mut vm, &bytecode, "sig-1", "Position1", 40);
    assert!(vm.get_entity_state(0, &json!("inst-1")).is_none());
    memo(
        &mut vm,
        &bytecode,
        "sig-1",
        "arete:p2:inst-1:-:a",
        ATTESTATION,
    );
    assert_eq!(instance(&vm, "inst-1")["raised"], json!(40));
}

#[test]
fn an_unattested_memo_registers_nothing() {
    let bytecode = bytecode();
    let mut vm = VmContext::new();
    memo(
        &mut vm,
        &bytecode,
        "sig-1",
        "arete:p2:inst-1:-:a",
        "Impostor",
    );
    contribute(&mut vm, &bytecode, "sig-1", "Position1", 40);
    assert!(instance(&vm, "inst-1").get("raised").is_none());
}

#[test]
fn siblings_of_other_transactions_are_not_paired() {
    let bytecode = bytecode();
    let mut vm = VmContext::new();
    memo(
        &mut vm,
        &bytecode,
        "sig-1",
        "arete:p2:inst-1:-:a",
        ATTESTATION,
    );
    contribute(&mut vm, &bytecode, "sig-2", "Position2", 40);
    assert!(instance(&vm, "inst-1").get("raised").is_none());
}

#[test]
fn waiting_registrations_and_captures_survive_a_snapshot() {
    let bytecode = bytecode();

    // The memo waits for its sibling across a restart.
    let mut vm = VmContext::new();
    memo(
        &mut vm,
        &bytecode,
        "sig-1",
        "arete:p2:inst-1:-:a",
        ATTESTATION,
    );
    let snapshot = serde_json::to_value(vm.dump()).unwrap();
    let mut restored = VmContext::new();
    restored.hydrate(serde_json::from_value(snapshot).unwrap());
    contribute(&mut restored, &bytecode, "sig-1", "Position1", 40);
    assert_eq!(instance(&restored, "inst-1")["raised"], json!(40));

    // A captured sibling is found by a memo after a restart.
    let mut vm = VmContext::new();
    vm.process_event(
        &bytecode,
        json!({"data": {"amount": 5}, "accounts": {"position": "Position3"}}),
        CONTRIBUTE,
        Some(&context("sig-3", "0")),
        None,
    )
    .unwrap();
    let snapshot = vm.dump();
    assert!(!snapshot.states[&0].sibling_instructions.is_empty());
    let mut restored = VmContext::new();
    restored.hydrate(snapshot);
    memo(
        &mut restored,
        &bytecode,
        "sig-3",
        "arete:p2:inst-3:-:a",
        ATTESTATION,
    );
    // The contribution itself was queued before the restart and queues are
    // not durable, but the registration landed: the next one routes.
    contribute(&mut restored, &bytecode, "sig-4", "Position3", 1);
    assert_eq!(instance(&restored, "inst-3")["raised"], json!(1));
}

#[test]
fn snapshots_without_siblings_keep_their_shape() {
    let mut vm = VmContext::new();
    let bytecode = bytecode();
    vm.process_event(
        &bytecode,
        json!({"data": {"memo": "arete:p2:inst-1"}, "accounts": {}}),
        MEMO,
        None,
        None,
    )
    .unwrap();
    let dumped = serde_json::to_value(vm.dump()).unwrap();
    let table = &dumped["states"]["0"];
    assert!(table.get("sibling_instructions").is_none());
    assert!(table.get("pending_sibling_registrations").is_none());
}

#[test]
fn the_pda_reverse_lookup_capacity_is_configurable_and_evictions_are_counted() {
    let plain = HookAction::RegisterPdaMapping {
        pda_field: FieldPath::new(&["accounts", "position"]),
        seed_field: FieldPath::new(&["accounts", "signer"]),
        lookup_name: LOOKUP.to_string(),
        sibling: None,
        seed_expr: None,
        guard: None,
    };
    let bytecode = MultiEntityBytecode::new()
        .add_entity(ENTITY.to_string(), instance_spec(plain), 0)
        .build();
    let mut vm = VmContext::new_with_config(StateTableConfig {
        max_pda_reverse_lookup_entries: 2,
        ..StateTableConfig::default()
    });
    for index in 0..3 {
        vm.process_event(
            &bytecode,
            json!({
                "data": {"memo": format!("arete:p2:inst-{index}")},
                "accounts": {"signer": format!("seed-{index}"), "position": format!("pda-{index}")},
            }),
            MEMO,
            Some(&context(&format!("sig-{index}"), "0")),
            None,
        )
        .unwrap();
    }
    let stats = vm.get_cache_stats(0);
    assert_eq!(stats.pda_reverse_lookup_capacity, 2);
    assert_eq!(stats.pda_reverse_lookup_evictions, 1);
    assert_eq!(vm.get_memory_stats(0).pda_reverse_lookup_total_entries, 2);

    // Re-registering a held address replaces it in place.
    vm.process_event(
        &bytecode,
        json!({
            "data": {"memo": "arete:p2:inst-2"},
            "accounts": {"signer": "seed-2b", "position": "pda-2"},
        }),
        MEMO,
        Some(&context("sig-x", "0")),
        None,
    )
    .unwrap();
    assert_eq!(vm.get_cache_stats(0).pda_reverse_lookup_evictions, 1);

    // The capacity survives a restore into a VM configured the same way.
    let mut restored = VmContext::new_with_config(StateTableConfig {
        max_pda_reverse_lookup_entries: 2,
        ..StateTableConfig::default()
    });
    restored.hydrate(vm.dump());
    assert_eq!(restored.get_cache_stats(0).pda_reverse_lookup_capacity, 2);

    let default = VmContext::new();
    assert_eq!(
        default.get_cache_stats(0).pda_reverse_lookup_capacity,
        StateTableConfig::DEFAULT_MAX_PDA_REVERSE_LOOKUP_ENTRIES
    );
}

#[test]
fn string_and_byte_functions() {
    let vm = VmContext::new();
    let eval = |expr: ComputedExpr, state: Value| vm.evaluate_computed_expr(&expr, &state).unwrap();
    let memo = json!({"memo": "arete:p2:inst-1::step"});

    let starts = |prefix: Value| method(field("memo"), "starts_with", vec![literal(prefix)]);
    assert_eq!(eval(starts(json!("arete:")), memo.clone()), json!(true));
    assert_eq!(eval(starts(json!("p2")), memo.clone()), json!(false));
    assert_eq!(eval(starts(json!("x")), json!({})), Value::Null);
    assert_eq!(eval(starts(Value::Null), memo.clone()), Value::Null);

    let part = |n: u64| {
        method(
            field("memo"),
            "split_part",
            vec![literal(json!(":")), literal(json!(n))],
        )
    };
    assert_eq!(eval(part(1), memo.clone()), json!("arete"));
    assert_eq!(eval(part(3), memo.clone()), json!("inst-1"));
    assert_eq!(eval(part(4), memo.clone()), json!(""), "an empty field");
    assert_eq!(eval(part(5), memo.clone()), json!("step"));
    assert_eq!(eval(part(6), memo.clone()), Value::Null, "past the end");
    assert_eq!(eval(part(0), memo.clone()), Value::Null, "1-based");
    assert_eq!(eval(part(1), json!({})), Value::Null);

    let utf8 = || method(field("bytes"), "utf8", vec![]);
    assert_eq!(
        eval(utf8(), json!({"bytes": [104, 105]})),
        json!("hi"),
        "inline byte array"
    );
    assert_eq!(eval(utf8(), json!({"bytes": [255, 0]})), Value::Null);
    assert_eq!(eval(utf8(), json!({})), Value::Null);
    let compact = |hex: &str, len: u64, truncated: bool| {
        json!({"bytes": {
            "__bytes_len": len,
            "__bytes_preview_hex": format!("0x{hex}"),
            "__bytes_truncated": truncated,
        }})
    };
    assert_eq!(eval(utf8(), compact("6869", 2, false)), json!("hi"));
    assert_eq!(
        eval(utf8(), compact("6869", 80, true)),
        Value::Null,
        "a truncated preview has lost bytes"
    );
    assert_eq!(eval(utf8(), json!({"bytes": "text"})), json!("text"));
}
