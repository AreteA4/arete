//! Declared (strict) lookups: a `Lookup` key resolution that names its index
//! resolves through that index only (directly or after PDA reverse lookups),
//! its index holds only the values its guarded field accepted, and its route
//! is deduplicated on its own. Lookups that declare no index keep the
//! inferred index, the fallback to the entity's other indexes and the
//! row-and-event deduplication.

use std::collections::BTreeMap;

use arete_interpreter::ast::{
    BinaryOp, ComputedExpr, ConditionExpr, FieldPath, HookAction, IdentitySpec, InstructionHook,
    KeyResolutionStrategy, LookupIndexSpec, MappingSource, ParsedCondition, PopulationStrategy,
    SerializableFieldMapping, SerializableHandlerSpec, SerializableStreamSpec, SourceSpec,
    TypedStreamSpec, CURRENT_AST_VERSION,
};
use arete_interpreter::compiler::MultiEntityBytecode;
use arete_interpreter::vm::{UpdateContext, VmContext};
use serde_json::{json, Value};

const MEMO: &str = "memo::MemoIxState";
const DEPOSIT: &str = "raise::DepositIxState";

fn field(path: &str) -> ComputedExpr {
    ComputedExpr::FieldRef {
        path: path.to_string(),
    }
}

fn literal(value: Value) -> ComputedExpr {
    ComputedExpr::Literal { value }
}

/// `split_part(data.memo, ":", part)`.
fn memo_part(part: u64) -> ComputedExpr {
    ComputedExpr::MethodCall {
        expr: Box::new(field("data.memo")),
        method: "split_part".to_string(),
        args: vec![literal(json!(":")), literal(json!(part))],
    }
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

/// Write `mapping` only when the memo names `step`.
fn for_step(mut mapping: SerializableFieldMapping, step: &str) -> SerializableFieldMapping {
    mapping.condition = Some(ConditionExpr {
        expression: format!("step == {step}"),
        parsed: Some(ParsedCondition::Guard {
            expr: ComputedExpr::Binary {
                op: BinaryOp::Eq,
                left: Box::new(memo_part(2)),
                right: Box::new(literal(json!(step))),
            },
        }),
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

fn source(type_name: &str, is_account: bool) -> SourceSpec {
    SourceSpec::Source {
        program_id: None,
        discriminator: None,
        type_name: type_name.to_string(),
        serialization: None,
        is_account,
    }
}

fn index(field_name: &str) -> LookupIndexSpec {
    LookupIndexSpec {
        field_name: field_name.to_string(),
        temporal_field: None,
    }
}

fn lookup(path: &[&str], index_name: Option<&str>) -> KeyResolutionStrategy {
    KeyResolutionStrategy::Lookup {
        primary_field: FieldPath::new(path),
        index_name: index_name.map(str::to_string),
    }
}

fn handler(
    source: SourceSpec,
    key_resolution: KeyResolutionStrategy,
    mappings: Vec<SerializableFieldMapping>,
) -> SerializableHandlerSpec {
    SerializableHandlerSpec {
        source,
        key_resolution,
        mappings,
        conditions: vec![],
        emit: true,
    }
}

fn stream_spec(
    entity: &str,
    identity: IdentitySpec,
    handlers: Vec<SerializableHandlerSpec>,
    instruction_hooks: Vec<InstructionHook>,
) -> TypedStreamSpec<Value> {
    TypedStreamSpec::from_serializable(SerializableStreamSpec {
        ast_version: CURRENT_AST_VERSION.to_string(),
        state_name: entity.to_string(),
        program_id: None,
        idl: None,
        identity,
        handlers,
        sections: vec![],
        field_mappings: BTreeMap::new(),
        resolver_hooks: vec![],
        instruction_hooks,
        resolver_specs: vec![],
        computed_fields: vec![],
        computed_field_specs: vec![],
        content_hash: None,
        views: vec![],
        scope_field: None,
    })
}

/// A participant fed by an attested memo (`<participant>:<step>`) and the
/// deposit sharing its transaction. The deposit reaches the row three ways:
/// directly by the participant it names (`deposits`), and by its signature
/// through the signature of the last memo naming step `a` (`a_amount`) or
/// `b` (`b_amount`). `declared` makes the two signature joins declare their
/// index.
fn participant(declared: bool) -> MultiEntityBytecode {
    use PopulationStrategy::{Count, LastWrite, SetOnce, Sum};
    let memo = handler(
        source(MEMO, false),
        KeyResolutionStrategy::Embedded {
            primary_field: FieldPath::new(&[]),
        },
        vec![
            mapping(
                "id.participant",
                MappingSource::FromExpr { expr: memo_part(1) },
                SetOnce,
            ),
            for_step(
                mapping("last_a_signature", from(&["data", "sig"]), LastWrite),
                "a",
            ),
            for_step(
                mapping("last_b_signature", from(&["data", "sig"]), LastWrite),
                "b",
            ),
        ],
    );
    let direct = handler(
        source(DEPOSIT, false),
        KeyResolutionStrategy::Embedded {
            primary_field: FieldPath::new(&["data", "participant"]),
        },
        vec![
            mapping("id.participant", from(&["data", "participant"]), SetOnce),
            mapping("deposits", from(&["data", "amount"]), Count),
        ],
    );
    let joined = |step: &str| {
        let index_name = format!("last_{step}_signature_lookup_index");
        handler(
            source(DEPOSIT, false),
            lookup(&["data", "sig"], declared.then_some(index_name.as_str())),
            vec![mapping(
                &format!("{step}_amount"),
                from(&["data", "amount"]),
                Sum,
            )],
        )
    };
    let spec = stream_spec(
        "Participant",
        IdentitySpec {
            primary_keys: vec!["id.participant".to_string()],
            lookup_indexes: vec![index("last_a_signature"), index("last_b_signature")],
        },
        vec![memo, direct, joined("a"), joined("b")],
        vec![],
    );
    MultiEntityBytecode::new()
        .add_entity("Participant".to_string(), spec, 0)
        .build()
}

/// An instruction at `ix_path` of transaction `signature`.
fn instruction(slot: u64, signature: &str, ix_path: &str) -> UpdateContext {
    let txn_index = signature.bytes().map(u64::from).sum();
    UpdateContext::new_instruction(slot, signature.to_string(), txn_index)
        .at_instruction(ix_path.to_string())
}

/// One transaction: the memo naming `step`, then the deposit.
fn transaction(
    vm: &mut VmContext,
    bytecode: &MultiEntityBytecode,
    slot: u64,
    signature: &str,
    step: &str,
    amount: u64,
) {
    vm.process_event(
        bytecode,
        json!({"data": {"memo": format!("alice:{step}"), "sig": signature}}),
        MEMO,
        Some(&instruction(slot, signature, "0")),
        None,
    )
    .unwrap();
    vm.process_event(
        bytecode,
        json!({"data": {"participant": "alice", "sig": signature, "amount": amount}}),
        DEPOSIT,
        Some(&instruction(slot, signature, "1")),
        None,
    )
    .unwrap();
}

fn replay_steps(declared: bool) -> Value {
    let bytecode = participant(declared);
    let mut vm = VmContext::new();
    transaction(&mut vm, &bytecode, 500, "sig-1", "a", 200);
    transaction(&mut vm, &bytecode, 501, "sig-2", "b", 50);
    transaction(&mut vm, &bytecode, 502, "sig-3", "a", 100);
    vm.get_entity_state(0, &json!("alice"))
        .expect("the memos created the participant")
}

#[test]
fn a_declared_join_counts_each_deposit_under_the_step_its_memo_names() {
    let row = replay_steps(true);
    assert_eq!(row["a_amount"], json!(300));
    assert_eq!(row["b_amount"], json!(50));
    // The direct route and both joins reach the same row from one deposit;
    // each applies once.
    assert_eq!(row["deposits"], json!(3));
}

#[test]
fn a_declared_join_is_deduplicated_on_its_own_route() {
    let bytecode = participant(true);
    let mut vm = VmContext::new();
    transaction(&mut vm, &bytecode, 500, "sig-1", "a", 200);
    // The same deposit delivered again (as by an overlapping replay).
    vm.process_event(
        &bytecode,
        json!({"data": {"participant": "alice", "sig": "sig-1", "amount": 200}}),
        DEPOSIT,
        Some(&instruction(500, "sig-1", "1")),
        None,
    )
    .unwrap();
    let row = vm.get_entity_state(0, &json!("alice")).unwrap();
    assert_eq!(row["a_amount"], json!(200));
    assert_eq!(row["deposits"], json!(1));
}

/// Without declared indexes the two signature joins load their key the same
/// way, so they share one handler; that handler finds the row through
/// whichever index holds the signature, and, reaching the row the direct
/// route already wrote for this instruction, is skipped as a duplicate.
/// This is the historical behaviour, kept for every stack that declares no
/// index.
#[test]
fn undeclared_joins_keep_the_inferred_fallback_and_shared_deduplication() {
    let row = replay_steps(false);
    assert_eq!(row["deposits"], json!(3));
    assert!(row.get("a_amount").is_none(), "{row}");
    assert!(row.get("b_amount").is_none(), "{row}");
}

/// A round account indexed by its address, a reset instruction registering
/// the treasury PDA to the current round's address, and a treasury account
/// reaching its round through that registration: PDA reverse lookup, then the
/// `round_address` index.
fn rounds(index_name: Option<&str>) -> MultiEntityBytecode {
    use PopulationStrategy::{LastWrite, SetOnce};
    let round = handler(
        source("ore::RoundState", true),
        KeyResolutionStrategy::Embedded {
            primary_field: FieldPath::new(&["data", "id"]),
        },
        vec![
            mapping("id.round_id", from(&["data", "id"]), SetOnce),
            mapping("id.round_address", from(&["__account_address"]), SetOnce),
        ],
    );
    let treasury = handler(
        source("ore::TreasuryState", true),
        lookup(&["__account_address"], index_name),
        vec![mapping(
            "treasury.balance",
            from(&["data", "balance"]),
            LastWrite,
        )],
    );
    let reset = InstructionHook {
        instruction_type: "ore::ResetIxState".to_string(),
        actions: vec![HookAction::RegisterPdaMapping {
            pda_field: FieldPath::new(&["accounts", "treasury"]),
            seed_field: FieldPath::new(&["accounts", "round"]),
            lookup_name: "default_pda_lookup".to_string(),
            sibling: None,
            seed_expr: None,
            guard: None,
        }],
        lookup_by: None,
    };
    let spec = stream_spec(
        "Round",
        IdentitySpec {
            primary_keys: vec!["id.round_id".to_string()],
            lookup_indexes: vec![index("id.round_address")],
        },
        vec![round, treasury],
        vec![reset],
    );
    MultiEntityBytecode::new()
        .add_entity("Round".to_string(), spec, 0)
        .build()
}

fn replay_rounds(index_name: Option<&str>) -> (Value, Value) {
    let bytecode = rounds(index_name);
    let mut vm = VmContext::new();
    let mut write_version = 0;
    let mut account = |vm: &mut VmContext, slot: u64, event_type: &str, value: Value| {
        write_version += 1;
        vm.process_event(
            &bytecode,
            value,
            event_type,
            Some(&UpdateContext::new_account(
                slot,
                format!("acct-{write_version}"),
                write_version,
            )),
            None,
        )
        .unwrap();
    };
    let reset = |vm: &mut VmContext, slot: u64, round: &str| {
        vm.process_event(
            &bytecode,
            json!({"accounts": {"treasury": "Treasury1", "round": round}}),
            "ore::ResetIxState",
            Some(&instruction(slot, &format!("reset-{round}"), "0")),
            None,
        )
        .unwrap();
    };
    for (slot, id, address, balance) in [(100, 42, "Round42", 7), (200, 43, "Round43", 9)] {
        account(
            &mut vm,
            slot,
            "ore::RoundState",
            json!({"__account_address": address, "data": {"id": id}}),
        );
        reset(&mut vm, slot, address);
        account(
            &mut vm,
            slot,
            "ore::TreasuryState",
            json!({"__account_address": "Treasury1", "data": {"balance": balance}}),
        );
    }
    (
        vm.get_entity_state(0, &json!(42)).expect("round 42"),
        vm.get_entity_state(0, &json!(43)).expect("round 43"),
    )
}

#[test]
fn a_treasury_reaches_its_round_through_the_reset_registration() {
    // Undeclared: the compiler infers no index for `__account_address`, so the
    // lookup reaches the round only through the fallback.
    let (round_42, round_43) = replay_rounds(None);
    assert_eq!(round_42["treasury"]["balance"], json!(7));
    assert_eq!(round_43["treasury"]["balance"], json!(9));

    // Declared: PDA reverse lookup, then the declared index. Same rows.
    let (declared_42, declared_43) = replay_rounds(Some("round_address_lookup_index"));
    assert_eq!(declared_42, round_42);
    assert_eq!(declared_43, round_43);
}

#[test]
fn a_declared_lookup_does_not_reach_a_row_through_another_index() {
    // Declaring an index the treasury's chain does not end in finds nothing,
    // where the undeclared lookup would fall back to `round_address`.
    let bytecode = rounds(Some("missing_lookup_index"));
    let mut vm = VmContext::new();
    vm.process_event(
        &bytecode,
        json!({"__account_address": "Round42", "data": {"id": 42}}),
        "ore::RoundState",
        Some(&UpdateContext::new_account(100, "a".to_string(), 1)),
        None,
    )
    .unwrap();
    vm.process_event(
        &bytecode,
        json!({"accounts": {"treasury": "Treasury1", "round": "Round42"}}),
        "ore::ResetIxState",
        Some(&instruction(100, "reset", "0")),
        None,
    )
    .unwrap();
    vm.process_event(
        &bytecode,
        json!({"__account_address": "Treasury1", "data": {"balance": 7}}),
        "ore::TreasuryState",
        Some(&UpdateContext::new_account(100, "b".to_string(), 2)),
        None,
    )
    .unwrap();
    let round = vm.get_entity_state(0, &json!(42)).expect("round 42");
    assert!(round.get("treasury").is_none(), "{round}");
}

#[test]
fn an_undeclared_lookup_serializes_without_an_index_name() {
    let undeclared = lookup(&["accounts", "position"], None);
    assert_eq!(
        serde_json::to_value(&undeclared).unwrap(),
        json!({"Lookup": {"primary_field": {"segments": ["accounts", "position"], "offsets": null}}})
    );
    let declared = lookup(&["accounts", "position"], Some("position_lookup_index"));
    let value = serde_json::to_value(&declared).unwrap();
    assert_eq!(
        value["Lookup"]["index_name"],
        json!("position_lookup_index")
    );
    // Older ASTs (no index_name) still read.
    let read: KeyResolutionStrategy = serde_json::from_value(
        json!({"Lookup": {"primary_field": {"segments": ["x"], "offsets": null}}}),
    )
    .unwrap();
    assert!(matches!(
        read,
        KeyResolutionStrategy::Lookup {
            index_name: None,
            ..
        }
    ));
}
