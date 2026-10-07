//! What one entity row costs in a VM state table, and what handlers cost on
//! such rows.
//!
//! A counting global allocator tracks live heap bytes. Rows are shaped like
//! the ones that dominate a VM's memory:
//!
//! - `position`: a DEX liquidity position with three 70-element per-bin
//!   arrays (fee records of four u128-as-string fields, reward records, and
//!   liquidity shares), plus identity and state sections. About 13.7 KB of
//!   JSON.
//! - `pool`: a pool with two 100-element recent-activity arrays (swaps and
//!   liquidity changes, with signatures and addresses). About 50 KB of JSON.
//! - `token`: a small entity of a few dozen scalars in three sections.
//!
//! It uses only the public API, so the same file measures earlier versions
//! too:
//!
//! ```text
//! cargo test -p arete-interpreter --test state_table_memory -- --nocapture
//! cargo test --release -p arete-interpreter --test state_table_memory -- --ignored --nocapture
//! ```

use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use arete_interpreter::ast::FieldPath;
use arete_interpreter::compiler::{EntityBytecode, MultiEntityBytecode, OpCode};
use arete_interpreter::vm::VmContext;
use serde_json::{json, Value};

struct CountingAllocator;

static LIVE: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            LIVE.fetch_add(layout.size(), Ordering::Relaxed);
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) };
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let moved = unsafe { System.realloc(pointer, layout, new_size) };
        if !moved.is_null() {
            if new_size >= layout.size() {
                LIVE.fetch_add(new_size - layout.size(), Ordering::Relaxed);
            } else {
                LIVE.fetch_sub(layout.size() - new_size, Ordering::Relaxed);
            }
        }
        moved
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn live() -> usize {
    LIVE.load(Ordering::Relaxed)
}

/// Deterministic generator for addresses, signatures and amounts.
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

    fn base58(&mut self, length: usize) -> String {
        const ALPHABET: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
        (0..length)
            .map(|_| ALPHABET[self.below(58) as usize] as char)
            .collect()
    }

    fn address(&mut self) -> String {
        let length = if self.below(16) == 0 { 43 } else { 44 };
        self.base58(length)
    }

    fn digits(&mut self, count: usize) -> String {
        let mut digits = String::with_capacity(count);
        digits.push((b'1' + self.below(9) as u8) as char);
        for _ in 1..count {
            digits.push((b'0' + self.below(10) as u8) as char);
        }
        digits
    }

    /// A u128 carried as a string: zero two times in five, otherwise 6 to
    /// 30 digits.
    fn u128_string(&mut self) -> String {
        if self.below(5) < 2 {
            "0".to_string()
        } else {
            let count = 6 + self.below(25) as usize;
            self.digits(count)
        }
    }

    /// A u64 carried as a string: zero one time in four.
    fn u64_string(&mut self) -> String {
        if self.below(4) == 0 {
            "0".to_string()
        } else {
            let count = 4 + self.below(14) as usize;
            self.digits(count)
        }
    }
}

fn fee_records(r: &mut Rng) -> Vec<Value> {
    (0..70)
        .map(|_| {
            json!({
                "fee_x": r.u128_string(),
                "fee_y": r.u128_string(),
                "pending_x": r.u64_string(),
                "pending_y": r.u64_string(),
            })
        })
        .collect()
}

/// The fields of a position account, as an account update event carries
/// them.
fn position_account(seed: u64) -> Value {
    let mut r = Rng::new(seed);
    let fees = fee_records(&mut r);
    let rewards: Vec<Value> = (0..70)
        .map(|_| {
            json!({
                "per_token": [r.u128_string(), r.u128_string()],
                "pending": [r.u64_string(), r.u64_string()],
            })
        })
        .collect();
    let shares: Vec<Value> = (0..70).map(|_| json!(r.u128_string())).collect();
    json!({
        "position": r.address(),
        "pool": r.address(),
        "owner": r.address(),
        "liquidity": r.digits(30),
        "lower_bin_id": -(r.below(5000) as i64),
        "upper_bin_id": -(r.below(5000) as i64) + 69,
        "last_updated_at": 1_759_000_000 + r.below(1_000_000),
        "total_claimed_fee_x": r.u64_string(),
        "total_claimed_fee_y": r.u64_string(),
        "total_claimed_rewards": [r.u64_string(), r.u64_string()],
        "operator": r.address(),
        "fee_owner": r.address(),
        "lock_release_point": 0,
        "is_closed": false,
        "fees": fees,
        "rewards": rewards,
        "liquidity_shares": shares,
    })
}

/// Where a position handler puts each account field.
const POSITION_MAPPINGS: &[(&str, &str)] = &[
    ("position", "id.position"),
    ("pool", "id.pool"),
    ("owner", "id.owner"),
    ("liquidity", "state.liquidity"),
    ("lower_bin_id", "state.lower_bin_id"),
    ("upper_bin_id", "state.upper_bin_id"),
    ("last_updated_at", "state.last_updated_at"),
    ("total_claimed_fee_x", "state.total_claimed_fee_x"),
    ("total_claimed_fee_y", "state.total_claimed_fee_y"),
    ("total_claimed_rewards", "state.total_claimed_rewards"),
    ("operator", "state.operator"),
    ("fee_owner", "state.fee_owner"),
    ("lock_release_point", "state.lock_release_point"),
    ("is_closed", "state.is_closed"),
    ("fees", "fees"),
    ("rewards", "rewards"),
    ("liquidity_shares", "liquidity_shares"),
];

fn set_path(row: &mut Value, path: &str, value: Value) {
    let mut current = row;
    let mut segments = path.split('.').peekable();
    while let Some(segment) = segments.next() {
        if segments.peek().is_none() {
            current[segment] = value;
            return;
        }
        current = &mut current[segment];
    }
}

/// A position entity row, as a handler mapping [`position_account`] leaves
/// it.
fn position(seed: u64) -> Value {
    let account = position_account(seed);
    let mut row = json!({});
    for (field, path) in POSITION_MAPPINGS {
        set_path(&mut row, path, account[*field].clone());
    }
    let mut r = Rng::new(seed ^ 0xfeed);
    row["metrics"] = json!({
        "value_usd": r.below(10_000_000) as f64 / 100.0,
        "apr": r.below(10_000) as f64 / 10_000.0,
        "last_slot": 370_000_000 + r.below(1_000_000),
    });
    row
}

fn swap(r: &mut Rng, index: u64) -> Value {
    json!({
        "signature": r.base58(88),
        "slot": 370_000_000 + index * 3 + r.below(3),
        "timestamp": 1_759_000_000 + index,
        "trader": r.address(),
        "side": if r.below(2) == 0 { "buy" } else { "sell" },
        "amount_in": r.u64_string(),
        "amount_out": r.u64_string(),
        "price": r.below(100_000_000) as f64 / 1_000_000.0,
    })
}

fn pool(seed: u64) -> Value {
    let mut r = Rng::new(seed);
    let swaps: Vec<Value> = (0..100).map(|index| swap(&mut r, index)).collect();
    let liquidity: Vec<Value> = (0..100)
        .map(|index| {
            json!({
                "signature": r.base58(88),
                "slot": 370_000_000 + index * 7,
                "owner": r.address(),
                "kind": if r.below(3) == 0 { "remove" } else { "add" },
                "amount_y": r.u64_string(),
            })
        })
        .collect();
    json!({
        "id": {
            "address": r.address(), "mint_x": r.address(), "mint_y": r.address(),
            "vault_x": r.address(), "vault_y": r.address(),
        },
        "state": {
            "active_bin": -(r.below(5000) as i64),
            "bin_step": 25,
            "reserve_x": r.u64_string(),
            "reserve_y": r.u64_string(),
            "protocol_fee_x": r.u64_string(),
            "protocol_fee_y": r.u64_string(),
            "status": "enabled",
            "last_swap_slot": 370_000_000 + r.below(1_000_000),
            "creator": r.address(),
        },
        "config": {
            "base_fee_bps": 25,
            "max_fee_bps": 500,
            "volatility": {
                "accumulator": r.below(1_000_000),
                "reference": r.below(1_000_000),
                "decay": 5000,
            },
        },
        "metrics": {
            "volume_24h": r.below(1_000_000_000) as f64 / 100.0,
            "fees_24h": r.below(10_000_000) as f64 / 100.0,
            "tvl_usd": r.below(1_000_000_000) as f64 / 100.0,
            "price": r.below(100_000_000) as f64 / 1_000_000.0,
            "trades_24h": r.below(100_000),
        },
        "recent_swaps": swaps,
        "recent_liquidity": liquidity,
    })
}

fn token(seed: u64) -> Value {
    let mut r = Rng::new(seed);
    json!({
        "id": {"mint": r.address(), "symbol": "TOKEN", "name": "Example Token"},
        "info": {
            "decimals": 6,
            "supply": r.u64_string(),
            "creator": r.address(),
            "uri": format!("https://example.invalid/{}.json", r.base58(20)),
            "created_at": 1_759_000_000 + r.below(1_000_000),
            "is_mutable": false,
        },
        "trading": {
            "price_sol": r.below(1_000_000) as f64 / 1e9,
            "market_cap_sol": r.below(1_000_000_000) as f64 / 1e3,
            "volume_sol": r.u64_string(),
            "buys": r.below(10_000),
            "sells": r.below(10_000),
            "holders": r.below(100_000),
            "last_trade_slot": 370_000_000 + r.below(1_000_000),
            "last_trader": r.address(),
            "bonding_progress": r.below(10_000) as f64 / 100.0,
            "complete": false,
        },
    })
}

fn key(index: u64) -> Value {
    json!(Rng::new(index).address())
}

const ROWS: u64 = 200;

#[test]
fn state_table_rows_cost_about_their_json_size() {
    println!("| row | JSON | as `Value` | in a state table | table / JSON | table / `Value` |");
    println!("| --- | ---: | ---: | ---: | ---: | ---: |");
    let mut ratios = HashMap::new();
    for (name, build) in [
        ("position", position as fn(u64) -> Value),
        ("pool", pool),
        ("token", token),
    ] {
        let rows: Vec<Value> = (0..ROWS).map(build).collect();
        let json_bytes: usize = rows
            .iter()
            .map(|row| serde_json::to_vec(row).unwrap().len())
            .sum();

        // One standalone copy of every row, as the yardstick.
        let start = live();
        let copies = rows.clone();
        let value_bytes = live() - start;
        drop(copies);

        let mut vm = VmContext::new();
        let keys: Vec<Value> = (0..ROWS).map(key).collect();
        let start = live();
        {
            let table = vm.get_state_table_mut(0).unwrap();
            for (key, row) in keys.iter().zip(&rows) {
                table.insert_with_eviction(key.clone(), row.clone());
            }
        }
        let table_bytes = live() - start;
        for (key, row) in keys.iter().zip(&rows) {
            assert_eq!(vm.get_entity_state(0, key).as_ref(), Some(row));
        }

        let per_row = |bytes: usize| bytes as f64 / ROWS as f64;
        println!(
            "| {name} | {:.0} B | {:.0} B | {:.0} B | {:.2} | {:.3} |",
            per_row(json_bytes),
            per_row(value_bytes),
            per_row(table_bytes),
            table_bytes as f64 / json_bytes as f64,
            table_bytes as f64 / value_bytes as f64,
        );
        ratios.insert(name, table_bytes as f64 / value_bytes as f64);
    }
    println!("(table bytes include each row's key and the table's bookkeeping)");

    // Rows dominated by small objects and digit strings shrink the most.
    assert!(ratios["position"] < 0.1, "{ratios:?}");
    assert!(ratios["pool"] < 0.3, "{ratios:?}");
    assert!(ratios["token"] < 0.5, "{ratios:?}");
}

fn mapping_handler(entity: &str, mut body: Vec<OpCode>) -> Vec<OpCode> {
    let mut handler = vec![
        OpCode::LoadEventField {
            path: FieldPath::new(&["__account_address"]),
            dest: 0,
            default: None,
        },
        OpCode::ReadOrInitState {
            state_id: 0,
            key: 0,
            default: json!({}),
            dest: 2,
        },
    ];
    handler.append(&mut body);
    handler.extend([
        OpCode::UpdateState {
            state_id: 0,
            key: 0,
            value: 2,
        },
        OpCode::EmitMutation {
            entity_name: entity.to_string(),
            key: 0,
            state: 2,
        },
    ]);
    handler
}

fn bytecode(entity: &str, event: &str, handler: Vec<OpCode>) -> MultiEntityBytecode {
    let entity_bytecode = EntityBytecode {
        state_id: 0,
        handlers: HashMap::from([(event.to_string(), handler)]),
        entity_name: entity.to_string(),
        when_events: HashSet::new(),
        non_emitted_fields: HashSet::new(),
        computed_paths: vec![],
        computed_fields_evaluator: None,
    };
    MultiEntityBytecode {
        entities: HashMap::from([(entity.to_string(), entity_bytecode)]),
        event_routing: HashMap::from([(event.to_string(), vec![entity.to_string()])]),
        when_events: HashSet::new(),
        proto_router: Default::default(),
    }
}

/// A position account handler: every account field mapped onto the entity.
fn position_bytecode() -> MultiEntityBytecode {
    let mut body = Vec::new();
    for (field, path) in POSITION_MAPPINGS {
        body.push(OpCode::LoadEventField {
            path: FieldPath::new(&[*field]),
            dest: 3,
            default: None,
        });
        body.push(OpCode::SetField {
            object: 2,
            path: path.to_string(),
            value: 3,
        });
    }
    bytecode(
        "Position",
        "dex::PositionState",
        mapping_handler("Position", body),
    )
}

/// A swap instruction handler: appends the swap to the pool's recent swaps
/// (kept to the last 100) and updates its reserves.
fn swap_bytecode() -> MultiEntityBytecode {
    let mut body = vec![
        OpCode::LoadEventField {
            path: FieldPath::new(&["swap"]),
            dest: 3,
            default: None,
        },
        OpCode::AppendToArray {
            object: 2,
            path: "recent_swaps".to_string(),
            value: 3,
        },
    ];
    for (field, path) in [
        ("reserve_x", "state.reserve_x"),
        ("reserve_y", "state.reserve_y"),
        ("slot", "state.last_swap_slot"),
    ] {
        body.push(OpCode::LoadEventField {
            path: FieldPath::new(&[field]),
            dest: 4,
            default: None,
        });
        body.push(OpCode::SetField {
            object: 2,
            path: path.to_string(),
            value: 4,
        });
    }
    bytecode("Pool", "dex::SwapIxState", mapping_handler("Pool", body))
}

/// An update to position `index`: its liquidity, timestamp and a few bins'
/// fees change.
fn position_update(index: u64, round: u64) -> Value {
    let mut account = position_account(index);
    let mut r = Rng::new(index * 1_000 + round);
    account["__account_address"] = key(index);
    account["liquidity"] = json!(r.digits(30));
    account["last_updated_at"] = json!(1_760_000_000 + round);
    for _ in 0..3 {
        let bin = r.below(70) as usize;
        account["fees"][bin]["fee_x"] = json!(r.u128_string());
    }
    account
}

fn swap_event(pool_index: u64, round: u64) -> Value {
    let mut r = Rng::new(pool_index * 1_000_000 + round);
    json!({
        "__account_address": key(pool_index),
        "swap": swap(&mut r, 1_000 + round),
        "reserve_x": r.u64_string(),
        "reserve_y": r.u64_string(),
        "slot": 371_000_000 + round,
    })
}

/// Microseconds per event: the fastest of five rounds.
fn per_event(rounds: usize, mut run: impl FnMut(u64)) -> f64 {
    (0..5)
        .map(|round| {
            let started = Instant::now();
            for event in 0..rounds {
                run((round * rounds + event) as u64);
            }
            started.elapsed().as_secs_f64() * 1e6 / rounds as f64
        })
        .fold(f64::INFINITY, f64::min)
}

/// Run with `cargo test --release -p arete-interpreter --test
/// state_table_memory -- --ignored --nocapture`.
#[test]
#[ignore = "timing report, not an assertion"]
fn handler_cost_on_large_rows() {
    const POSITIONS: u64 = 2_500;
    const POOLS: u64 = 20;
    const EVENTS: usize = 2_000;

    // A table full of positions, then updates spread over all of them.
    let positions = position_bytecode();
    let mut vm = VmContext::new();
    let start = live();
    for index in 0..POSITIONS {
        let mut account = position_account(index);
        account["__account_address"] = key(index);
        vm.process_event(&positions, account, "dex::PositionState", None, None)
            .unwrap();
    }
    let table_bytes = live() - start;
    let events: Vec<Value> = (0..EVENTS as u64)
        .map(|event| position_update(event * 7_919 % POSITIONS, event))
        .collect();
    let spread = per_event(EVENTS, |event| {
        let update = events[event as usize % EVENTS].clone();
        vm.process_event(&positions, update, "dex::PositionState", None, None)
            .unwrap();
    });
    let hot = per_event(EVENTS, |event| {
        let mut update = events[0].clone();
        update["last_updated_at"] = json!(event);
        vm.process_event(&positions, update, "dex::PositionState", None, None)
            .unwrap();
    });

    // Pools with full activity arrays, then swaps that append to them.
    let swaps = swap_bytecode();
    let mut pool_vm = VmContext::new();
    {
        let table = pool_vm.get_state_table_mut(0).unwrap();
        for index in 0..POOLS {
            table.insert_with_eviction(key(index), pool(index));
        }
    }
    let swap_events: Vec<Value> = (0..EVENTS as u64)
        .map(|event| swap_event(event % POOLS, event))
        .collect();
    let swapped = per_event(EVENTS, |event| {
        let swap = swap_events[event as usize % EVENTS].clone();
        pool_vm
            .process_event(&swaps, swap, "dex::SwapIxState", None, None)
            .unwrap();
    });

    println!(
        "{POSITIONS} positions in the VM: {:.1} MiB of heap ({:.1} KiB each)",
        table_bytes as f64 / 1_048_576.0,
        table_bytes as f64 / POSITIONS as f64 / 1024.0,
    );
    println!("| handler | per event |");
    println!("| --- | ---: |");
    println!("| position update, spread over {POSITIONS} rows | {spread:.1} µs |");
    println!("| position update, one row | {hot:.1} µs |");
    println!("| swap appended to one of {POOLS} pools | {swapped:.1} µs |");
}
