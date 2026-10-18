//! Expressions evaluated against the incoming event (`MappingSource::FromExpr`)
//! and max, min and sum over the decimal strings decoders emit for wide
//! integers, exercised by one-minute OHLC candles for a CLMM pool: each swap
//! event routes to a row keyed by its pool and minute, which it folds its
//! post-trade square-root price into.

use std::collections::BTreeMap;

use arete_interpreter::ast::{
    BinaryOp, ComputedExpr, FieldPath, IdentitySpec, KeyResolutionStrategy, MappingSource,
    PopulationStrategy, SerializableFieldMapping, SerializableHandlerSpec, SerializableStreamSpec,
    SourceSpec, Transformation, TypedStreamSpec, CURRENT_AST_VERSION,
};
use arete_interpreter::compiler::MultiEntityBytecode;
use arete_interpreter::vm::{UpdateContext, VmContext};
use serde_json::{json, Value};

const ENTITY: &str = "Candle1m";
const TRADED: &str = "whirlpool::TradedCpiEvent";

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

fn from(field: &str) -> MappingSource {
    MappingSource::FromSource {
        path: FieldPath::new(&[field]),
        default: None,
        transform: None,
    }
}

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

/// `__timestamp / 60 * 60`: the start of the event's minute.
fn minute() -> ComputedExpr {
    binary(
        BinaryOp::Mul,
        binary(
            BinaryOp::Div,
            ComputedExpr::ContextTimestamp,
            literal(json!(60)),
        ),
        literal(json!(60)),
    )
}

/// `concat(whirlpool, ":", minute)`.
fn candle_key() -> ComputedExpr {
    ComputedExpr::MethodCall {
        expr: Box::new(field("whirlpool")),
        method: "concat".to_string(),
        args: vec![literal(json!(":")), minute()],
    }
}

/// `f64(input_amount) / f64(output_amount)`: a per-trade price.
fn trade_price() -> ComputedExpr {
    let as_f64 = |path| ComputedExpr::Cast {
        expr: Box::new(field(path)),
        to_type: "f64".to_string(),
    };
    binary(
        BinaryOp::Div,
        as_f64("input_amount"),
        as_f64("output_amount"),
    )
}

fn candle_spec() -> TypedStreamSpec<Value> {
    candle_spec_keyed(mapping(
        "id.key",
        MappingSource::FromExpr { expr: candle_key() },
        PopulationStrategy::SetOnce,
    ))
}

/// The candle spec with `key` as its primary key mapping.
fn candle_spec_keyed(key: SerializableFieldMapping) -> TypedStreamSpec<Value> {
    use PopulationStrategy::{Count, LastWrite, Max, Min, SetOnce, Sum};
    let expr = |expr| MappingSource::FromExpr { expr };
    let traded = SerializableHandlerSpec {
        source: SourceSpec::Source {
            program_id: None,
            discriminator: None,
            type_name: TRADED.to_string(),
            serialization: None,
            is_account: false,
        },
        // No key field: the primary key mapping's expression is the key.
        key_resolution: KeyResolutionStrategy::Embedded {
            primary_field: FieldPath::new(&[]),
        },
        mappings: vec![
            key,
            mapping("id.pool", from("whirlpool"), SetOnce),
            mapping("id.bucket", expr(minute()), SetOnce),
            mapping("ohlc.open", from("post_sqrt_price"), SetOnce),
            mapping("ohlc.high", from("post_sqrt_price"), Max),
            mapping("ohlc.low", from("post_sqrt_price"), Min),
            mapping("ohlc.close", from("post_sqrt_price"), LastWrite),
            mapping("trade_price.high", expr(trade_price()), Max),
            mapping("trade_price.low", expr(trade_price()), Min),
            mapping("volume.input", from("input_amount"), Sum),
            mapping("volume.trades", from("whirlpool"), Count),
        ],
        conditions: vec![],
        emit: true,
    };
    TypedStreamSpec::from_serializable(SerializableStreamSpec {
        ast_version: CURRENT_AST_VERSION.to_string(),
        state_name: ENTITY.to_string(),
        program_id: None,
        idl: None,
        identity: IdentitySpec {
            primary_keys: vec!["id.key".to_string()],
            lookup_indexes: vec![],
        },
        handlers: vec![traded],
        sections: vec![],
        field_mappings: BTreeMap::new(),
        resolver_hooks: vec![],
        instruction_hooks: vec![],
        resolver_specs: vec![],
        computed_fields: vec![],
        computed_field_specs: vec![],
        content_hash: None,
        views: vec![],
        scope_field: None,
    })
}

fn bytecode() -> MultiEntityBytecode {
    bytecode_for(candle_spec())
}

fn bytecode_for(spec: TypedStreamSpec<Value>) -> MultiEntityBytecode {
    MultiEntityBytecode::new()
        .add_entity(ENTITY.to_string(), spec, 0)
        .build()
}

/// A swap on `pool` at `timestamp`. The decoder emits the u128 square-root
/// price as a decimal string, and u64 amounts above 2^53 as strings too.
fn swap(
    vm: &mut VmContext,
    bytecode: &MultiEntityBytecode,
    pool: &str,
    timestamp: i64,
    sqrt_price: &str,
    input_amount: Value,
    output_amount: Value,
) {
    let context =
        UpdateContext::with_timestamp(400_000_000, format!("sig-{pool}-{timestamp}"), timestamp);
    vm.process_event(
        bytecode,
        json!({
            "whirlpool": pool,
            "post_sqrt_price": sqrt_price,
            "input_amount": input_amount,
            "output_amount": output_amount,
        }),
        TRADED,
        Some(&context),
        None,
    )
    .unwrap();
}

fn candle(vm: &VmContext, key: &str) -> Value {
    vm.get_entity_state(0, &json!(key))
        .unwrap_or_else(|| panic!("no candle {key}"))
}

#[test]
fn swaps_fold_into_the_candle_of_their_pool_and_minute() {
    let bytecode = bytecode();
    let mut vm = VmContext::new();

    // Minute 120 of pool A: prices of different digit counts, so a string
    // comparison would order them wrongly.
    swap(
        &mut vm,
        &bytecode,
        "PoolA",
        125,
        "95000000000000000000",
        json!(100),
        json!(50),
    );
    swap(
        &mut vm,
        &bytecode,
        "PoolA",
        130,
        "100000000000000000000",
        json!(300),
        json!(100),
    );
    swap(
        &mut vm,
        &bytecode,
        "PoolA",
        179,
        "9000000000000000000",
        json!(10),
        json!(10),
    );
    // The next minute of pool A, and the same minute of pool B.
    swap(&mut vm, &bytecode, "PoolA", 180, "200", json!(1), json!(1));
    swap(&mut vm, &bytecode, "PoolB", 150, "7", json!(1), json!(1));

    let a = candle(&vm, "PoolA:120");
    assert_eq!(
        a["id"],
        json!({ "key": "PoolA:120", "pool": "PoolA", "bucket": 120 })
    );
    assert_eq!(
        a["ohlc"],
        json!({
            "open": "95000000000000000000",
            "high": "100000000000000000000",
            "low": "9000000000000000000",
            "close": "9000000000000000000",
        })
    );
    assert_eq!(a["trade_price"], json!({ "high": 3.0, "low": 1.0 }));
    assert_eq!(a["volume"], json!({ "input": 410, "trades": 3 }));

    let next = candle(&vm, "PoolA:180");
    assert_eq!(next["ohlc"]["open"], json!("200"));
    assert_eq!(next["volume"]["trades"], json!(1));

    let b = candle(&vm, "PoolB:120");
    assert_eq!(b["id"]["pool"], json!("PoolB"));
    assert_eq!(b["ohlc"]["high"], json!("7"));
}

#[test]
fn sums_of_wide_integers_stay_exact() {
    let bytecode = bytecode();
    let mut vm = VmContext::new();
    let large = u64::MAX - 1;

    swap(
        &mut vm,
        &bytecode,
        "PoolA",
        60,
        "1",
        json!(large.to_string()),
        json!(1),
    );
    swap(&mut vm, &bytecode, "PoolA", 61, "1", json!(large), json!(1));

    // 2 * (2^64 - 2) is past u64, so it is kept as a decimal string.
    let expected = (u128::from(large) * 2).to_string();
    assert_eq!(candle(&vm, "PoolA:60")["volume"]["input"], json!(expected));
}

#[test]
fn an_event_whose_key_expression_is_null_creates_no_row() {
    let bytecode = bytecode();
    let mut vm = VmContext::new();
    let context = UpdateContext::with_timestamp(400_000_000, "sig".to_string(), 60);

    // No `whirlpool`, so `concat` yields null and no key.
    let mutations = vm
        .process_event(
            &bytecode,
            json!({ "post_sqrt_price": "1", "input_amount": 1, "output_amount": 1 }),
            TRADED,
            Some(&context),
            None,
        )
        .unwrap();

    assert!(mutations.is_empty());
    assert!(vm.get_entity_state(0, &json!(":60")).is_none());
}

#[test]
fn integers_above_i128_stay_exact() {
    let bytecode = bytecode();
    let mut vm = VmContext::new();
    // 2^127 and 2^127 + 1: equal once rounded to f64.
    let low = "170141183460469231731687303715884105728";
    let high = "170141183460469231731687303715884105729";

    swap(&mut vm, &bytecode, "PoolA", 60, high, json!(low), json!(1));
    swap(&mut vm, &bytecode, "PoolA", 61, low, json!(1), json!(1));
    swap(&mut vm, &bytecode, "PoolA", 62, high, json!(-2), json!(1));

    let candle = candle(&vm, "PoolA:60");
    assert_eq!(candle["ohlc"]["high"], json!(high));
    assert_eq!(candle["ohlc"]["low"], json!(low));
    // 2^127 + 1 - 2, past i128 and back below it.
    assert_eq!(
        candle["volume"]["input"],
        json!("170141183460469231731687303715884105727")
    );
}

#[test]
fn an_expression_key_takes_the_key_mapping_transform() {
    let mut key = mapping(
        "id.key",
        MappingSource::FromExpr { expr: minute() },
        PopulationStrategy::SetOnce,
    );
    key.transform = Some(Transformation::ToString);
    let bytecode = bytecode_for(candle_spec_keyed(key));
    let mut vm = VmContext::new();

    swap(&mut vm, &bytecode, "PoolA", 125, "1", json!(1), json!(1));

    // The row is keyed by the value written to its key field.
    let candle = candle(&vm, "120");
    assert_eq!(candle["id"]["key"], json!("120"));
    assert!(vm.get_entity_state(0, &json!(120)).is_none());
}

#[test]
fn a_null_expression_key_stays_null_through_the_key_transform() {
    let mut key = mapping(
        "id.key",
        MappingSource::FromExpr {
            expr: field("whirlpool"),
        },
        PopulationStrategy::SetOnce,
    );
    key.transform = Some(Transformation::ToString);
    let bytecode = bytecode_for(candle_spec_keyed(key));
    let mut vm = VmContext::new();
    let context = UpdateContext::with_timestamp(400_000_000, "sig".to_string(), 60);

    // No `whirlpool`: the key is null, not the string "null".
    let mutations = vm
        .process_event(
            &bytecode,
            json!({ "post_sqrt_price": "1", "input_amount": 1, "output_amount": 1 }),
            TRADED,
            Some(&context),
            None,
        )
        .unwrap();

    assert!(mutations.is_empty());
    assert!(vm.get_entity_state(0, &json!("null")).is_none());
}
