//! What the VM spends per event on a stack whose state table is full and
//! evicting.
//!
//! The workload is a stack consuming the full SPL Token program firehose:
//! account updates for many more token accounts than the table holds, mixed
//! with `TransferChecked` instructions against them, so most events read a
//! row, change a few fields and write it back, and new rows keep evicting old
//! ones. The entity:
//!
//! - `id`: `address`, `owner`, `mint` and a constant `token_program`.
//! - `authority`: `delegate`, `close_authority`.
//! - `balance`: `amount`, `delegated_amount`, `state`, `native_reserve`,
//!   `updated_slot`.
//! - `activity`: `transfers_out` (a count), `recent_transfers` (an append of
//!   destination and amount) and `has_observed_transfers`, computed from the
//!   count.
//!
//! A counting global allocator reports allocations per event. Run with:
//!
//! ```text
//! cargo test --release -p arete-interpreter --test vm_hot_path -- --ignored --nocapture
//! ```
//!
//! The non-ignored test checks that the workload does what it says, so the
//! timing report measures the path it claims to.

use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use arete_interpreter::ast::{
    BinaryOp, ComputedExpr, ComputedFieldSpec, FieldPath, IdentitySpec, KeyResolutionStrategy,
    MappingSource, PopulationStrategy, SerializableFieldMapping, SerializableHandlerSpec,
    SerializableStreamSpec, SourceSpec, TypedStreamSpec, CURRENT_AST_VERSION,
};
use arete_interpreter::compiler::MultiEntityBytecode;
use arete_interpreter::vm::{StateTableConfig, UpdateContext, VmContext};
use serde_json::{json, Value};

struct CountingAllocator;

static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) };
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.realloc(pointer, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn allocations() -> usize {
    ALLOCATIONS.load(Ordering::Relaxed)
}

const ENTITY: &str = "TokenAccount";
const ACCOUNT_EVENT: &str = "spl_token::TokenAccountState";
const TRANSFER_EVENT: &str = "spl_token::TransferCheckedIxState";
const TOKEN_PROGRAM: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";

/// Rows the table holds, and how many distinct accounts the events touch.
const CAPACITY: usize = 2_500;
const ACCOUNTS: u64 = 5_000;
/// Accounts that see a large share of the traffic, as pool vaults and
/// exchange wallets do. Their rows stay resident and fill their recent
/// transfers.
const HOT_ACCOUNTS: u64 = 50;
/// Recent transfers kept per account: the VM's default array cap.
const DEFAULT_MAX_ARRAY_LENGTH: usize = 100;

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

fn from(path: &[&str]) -> MappingSource {
    MappingSource::FromSource {
        path: FieldPath::new(path),
        default: None,
        transform: None,
    }
}

fn handler(
    type_name: &str,
    is_account: bool,
    key: &[&str],
    mappings: Vec<SerializableFieldMapping>,
) -> SerializableHandlerSpec {
    SerializableHandlerSpec {
        source: SourceSpec::Source {
            program_id: None,
            discriminator: None,
            type_name: type_name.to_string(),
            serialization: None,
            is_account,
        },
        key_resolution: KeyResolutionStrategy::Embedded {
            primary_field: FieldPath::new(key),
        },
        mappings,
        conditions: vec![],
        emit: true,
    }
}

fn token_account_spec() -> TypedStreamSpec<Value> {
    use PopulationStrategy::{Append, Count, LastWrite, SetOnce};
    let account = handler(
        ACCOUNT_EVENT,
        true,
        &["__account_address"],
        vec![
            mapping("id.address", from(&["__account_address"]), SetOnce),
            mapping("id.owner", from(&["owner"]), LastWrite),
            mapping("id.mint", from(&["mint"]), SetOnce),
            mapping(
                "id.token_program",
                MappingSource::Constant(json!(TOKEN_PROGRAM)),
                SetOnce,
            ),
            mapping("authority.delegate", from(&["delegate"]), LastWrite),
            mapping(
                "authority.close_authority",
                from(&["close_authority"]),
                LastWrite,
            ),
            mapping("balance.amount", from(&["amount"]), LastWrite),
            mapping(
                "balance.delegated_amount",
                from(&["delegated_amount"]),
                LastWrite,
            ),
            mapping("balance.state", from(&["state"]), LastWrite),
            mapping("balance.native_reserve", from(&["is_native"]), LastWrite),
            mapping(
                "balance.updated_slot",
                MappingSource::FromContext {
                    field: "slot".to_string(),
                },
                LastWrite,
            ),
        ],
    );
    let transfer = handler(
        TRANSFER_EVENT,
        false,
        &["accounts", "source"],
        vec![
            mapping("id.address", from(&["accounts", "source"]), SetOnce),
            mapping("activity.transfers_out", from(&["data"]), Count),
            mapping(
                "activity.recent_transfers",
                MappingSource::AsEvent {
                    fields: vec![
                        Box::new(from(&["accounts", "destination"])),
                        Box::new(from(&["data", "amount"])),
                    ],
                },
                Append,
            ),
        ],
    );
    TypedStreamSpec::from_serializable(SerializableStreamSpec {
        ast_version: CURRENT_AST_VERSION.to_string(),
        state_name: ENTITY.to_string(),
        program_id: None,
        idl: None,
        identity: IdentitySpec {
            primary_keys: vec!["id.address".to_string()],
            lookup_indexes: vec![],
        },
        handlers: vec![account, transfer],
        sections: vec![],
        field_mappings: BTreeMap::new(),
        resolver_hooks: vec![],
        instruction_hooks: vec![],
        resolver_specs: vec![],
        computed_fields: vec!["activity.has_observed_transfers".to_string()],
        computed_field_specs: vec![ComputedFieldSpec {
            target_path: "activity.has_observed_transfers".to_string(),
            expression: ComputedExpr::Binary {
                op: BinaryOp::Gt,
                left: Box::new(ComputedExpr::UnwrapOr {
                    expr: Box::new(ComputedExpr::FieldRef {
                        path: "activity.transfers_out".to_string(),
                    }),
                    default: json!(0),
                }),
                right: Box::new(ComputedExpr::Literal { value: json!(0) }),
            },
            result_type: "bool".to_string(),
        }],
        content_hash: None,
        views: vec![],
    })
}

type EvaluatorResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

/// What generated code evaluates for `activity.has_observed_transfers`.
fn evaluate_computed(state: &mut Value, _slot: Option<u64>, _timestamp: i64) -> EvaluatorResult {
    let observed = state
        .pointer("/activity/transfers_out")
        .and_then(Value::as_u64)
        .is_some_and(|count| count > 0);
    if let Some(row) = state.as_object_mut() {
        if row.contains_key("activity") || observed {
            row.entry("activity")
                .or_insert_with(|| json!({}))
                .as_object_mut()
                .ok_or("activity is not an object")?
                .insert("has_observed_transfers".to_string(), json!(observed));
        }
    }
    Ok(())
}

fn bytecode() -> MultiEntityBytecode {
    MultiEntityBytecode::new()
        .add_entity_with_evaluator(
            ENTITY.to_string(),
            token_account_spec(),
            0,
            Some(evaluate_computed),
        )
        .build()
}

fn vm() -> VmContext {
    VmContext::new_with_config(StateTableConfig {
        max_entries: CAPACITY,
        ..StateTableConfig::default()
    })
}

/// Deterministic generator for addresses and amounts.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let x = self.0;
        (x ^ (x >> 29)).wrapping_mul(0xBF58_476D_1CE4_E5B9) ^ (x >> 32)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    /// An account index: a hot account `hot_percent` of the time, otherwise
    /// any of them.
    fn account(&mut self, hot_percent: u64) -> u64 {
        if self.below(100) < hot_percent {
            self.below(HOT_ACCOUNTS)
        } else {
            self.below(ACCOUNTS)
        }
    }

    fn address(&mut self) -> String {
        const ALPHABET: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
        let length = if self.below(16) == 0 { 43 } else { 44 };
        (0..length)
            .map(|_| ALPHABET[self.below(58) as usize] as char)
            .collect()
    }
}

fn account_address(index: u64) -> String {
    Rng::new(index).address()
}

/// One event of the workload: what it is, its payload and its context.
struct Event {
    event_type: &'static str,
    value: Value,
    context: UpdateContext,
}

/// `count` events over [`ACCOUNTS`] accounts: account updates, and after
/// about one in three a transfer out of an account. A fifth of the updates
/// and half the transfers go to the [`HOT_ACCOUNTS`]. Run workloads in
/// the order of their seeds.
fn workload(count: usize, seed: u64) -> Vec<Event> {
    let mut r = Rng::new(seed);
    let mints: Vec<String> = (0..50).map(|_| r.address()).collect();
    let mut events = Vec::with_capacity(count);
    // Later seeds come later in the chain, so no account update is stale
    // against one an earlier workload delivered.
    let mut slot = 380_000_000 + seed * 1_000_000;
    let mut write_version = seed * 100_000_000;
    while events.len() < count {
        slot += r.below(2);
        write_version += 1;
        let index = r.account(20);
        let mut account_rng = Rng::new(index ^ 0x5eed);
        let mint = &mints[account_rng.below(mints.len() as u64) as usize];
        let owner = account_rng.address();
        let delegated = account_rng.below(10) == 0;
        events.push(Event {
            event_type: ACCOUNT_EVENT,
            value: json!({
                "__account_address": account_address(index),
                "mint": mint,
                "owner": owner,
                "amount": r.below(10_000_000_000_000),
                "delegate": if delegated { json!(account_rng.address()) } else { Value::Null },
                "state": "initialized",
                "is_native": Value::Null,
                "delegated_amount": if delegated { r.below(1_000_000) } else { 0 },
                "close_authority": Value::Null,
            }),
            context: UpdateContext::new_account(slot, r.address(), write_version),
        });
        if r.below(3) == 0 {
            let source = r.account(50);
            let txn_index = r.below(2_000);
            events.push(transfer(&mut r, source, slot, txn_index));
        }
    }
    events.truncate(count);
    events
}

fn transfer(r: &mut Rng, source: u64, slot: u64, txn_index: u64) -> Event {
    Event {
        event_type: TRANSFER_EVENT,
        value: json!({
            "accounts": {
                "source": account_address(source),
                "mint": r.address(),
                "destination": account_address(r.below(ACCOUNTS)),
                "authority": r.address(),
            },
            "data": { "amount": r.below(1_000_000_000), "decimals": 6 },
        }),
        context: UpdateContext::new_instruction(slot, r.address(), txn_index),
    }
}

fn run(vm: &mut VmContext, bytecode: &MultiEntityBytecode, event: &Event) -> usize {
    vm.process_event(
        bytecode,
        event.value.clone(),
        event.event_type,
        Some(&event.context),
        None,
    )
    .unwrap()
    .len()
}

/// A VM in steady state: the hot accounts' recent transfers full, and the
/// table full.
fn warm(bytecode: &MultiEntityBytecode) -> VmContext {
    let mut vm = vm();
    let mut r = Rng::new(0);
    for round in 0..DEFAULT_MAX_ARRAY_LENGTH as u64 {
        for source in 0..HOT_ACCOUNTS {
            run(
                &mut vm,
                bytecode,
                &transfer(&mut r, source, 370_000_000 + round, source),
            );
        }
    }
    for event in workload(2 * CAPACITY, 1) {
        run(&mut vm, bytecode, &event);
    }
    vm
}

fn key(event: &Event) -> &Value {
    match event.event_type {
        ACCOUNT_EVENT => &event.value["__account_address"],
        _ => &event.value["accounts"]["source"],
    }
}

#[test]
fn workload_keeps_the_table_full_and_evicting() {
    let bytecode = bytecode();
    let mut vm = warm(&bytecode);
    assert_eq!(vm.get_state_table_mut(0).unwrap().len(), CAPACITY);

    let events = workload(2_000, 2);
    let mut mutations = 0;
    let mut new_rows = 0;
    for event in &events {
        let table = vm.get_state_table_mut(0).unwrap();
        if !table.contains_key(key(event)) {
            new_rows += 1;
        }
        mutations += run(&mut vm, &bytecode, event);
        assert_eq!(vm.get_state_table_mut(0).unwrap().len(), CAPACITY);
    }
    // Every event changes its row: none is stale or a duplicate.
    assert_eq!(mutations, events.len());
    // Most events change a resident row, and a good share bring in a new
    // one, evicting another.
    assert!(
        (300..900).contains(&new_rows),
        "{new_rows} of 2000 rows new"
    );

    // A hot account's row carries every section and a full history.
    let row = vm.get_entity_state(0, &json!(account_address(0))).unwrap();
    assert_eq!(row["id"]["address"], json!(account_address(0)));
    assert_eq!(row["id"]["token_program"], json!(TOKEN_PROGRAM));
    assert!(row["balance"]["amount"].is_u64());
    assert!(row["activity"]["transfers_out"].as_u64().unwrap() > 100);
    assert_eq!(row["activity"]["has_observed_transfers"], json!(true));
    assert_eq!(
        row["activity"]["recent_transfers"]
            .as_array()
            .unwrap()
            .len(),
        DEFAULT_MAX_ARRAY_LENGTH
    );
}

/// Run with `cargo test --release -p arete-interpreter --test vm_hot_path --
/// --ignored --nocapture`.
#[test]
#[ignore = "timing report, not an assertion"]
fn per_event_cost_with_a_full_table() {
    // Many short rounds, so the fastest is likely to have run undisturbed
    // even on a busy machine.
    const EVENTS: usize = 2_000;
    const ROUNDS: usize = 50;

    let bytecode = bytecode();
    let mut vm = warm(&bytecode);
    let rounds: Vec<Vec<Event>> = (0..ROUNDS)
        .map(|round| workload(EVENTS, 100 + round as u64))
        .collect();

    let mut best_ns = f64::INFINITY;
    let mut best_allocations = usize::MAX;
    for events in &rounds {
        // Cloning each payload, as the caller hands the VM an owned event,
        // is counted apart from the VM's own work.
        let payloads: Vec<Value> = events.iter().map(|event| event.value.clone()).collect();
        let before = allocations();
        let started = Instant::now();
        for (event, payload) in events.iter().zip(payloads) {
            vm.process_event(
                &bytecode,
                payload,
                event.event_type,
                Some(&event.context),
                None,
            )
            .unwrap();
        }
        let elapsed = started.elapsed();
        let allocated = allocations() - before;
        best_ns = best_ns.min(elapsed.as_nanos() as f64 / EVENTS as f64);
        best_allocations = best_allocations.min(allocated);
    }

    println!(
        "{EVENTS} events per round over {ACCOUNTS} accounts, table capacity {CAPACITY}, best of {ROUNDS}"
    );
    println!("| per event | allocations per event |");
    println!("| ---: | ---: |");
    println!(
        "| {best_ns:.0} ns | {:.1} |",
        best_allocations as f64 / EVENTS as f64
    );
}
