//! What one source update costs the projector, and how that cost grows with
//! the number of rows the views hold.
//!
//! The workload resembles a stack consuming a token program's account
//! firehose: a few thousand keys, rows with four nested sections, about 1.4
//! patches per update (a whole row now and then, mostly `balance`-only
//! patches), and a list, a state and three derived views over the list: an
//! unbounded collection sorted by address, a top 25 by balance and a
//! filtered top 25 by activity.
//!
//! It feeds the real projector through the public API and reports the wall
//! time and heap allocations per update, then times the read a subscriber's
//! window does after each change. No external harness: a counting global
//! allocator and `Instant` are enough, and the same file measures earlier
//! versions.
//!
//! ```text
//! cargo bench -p arete-server --bench view_maintenance
//! ```

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use arete_interpreter::Mutation;
use arete_server::materialized_view::{
    CompareOp, FilterConfig, SortConfig, SortOrder, ViewPipeline,
};
use arete_server::sorted_cache::{SortOrder as CacheOrder, SortedViewCache};
use arete_server::{
    BusManager, Delivery, EntityCache, EntityCacheConfig, Filters, Mode, MutationBatch, Projection,
    Projector, SlotContext, ViewIndex, ViewSpec,
};
use serde_json::{json, Value};
use tokio::sync::mpsc;

struct CountingAllocator;

static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);

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

/// Updates per timed round. Each measurement is the fastest of `ROUNDS`
/// rounds, which keeps a busy machine's noise out of the comparison.
const UPDATES: u64 = 4_000;
const ROUNDS: usize = 7;
const BATCH: usize = 64;
const CACHED_ENTITIES: usize = 20_000;
const EXPORT: &str = "TokenAccount";

/// A small deterministic generator, so every run replays the same updates.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 11
    }
}

fn address(seed: u64) -> String {
    const ALPHABET: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    let mut rng = Rng(seed ^ 0x9e37_79b9_7f4a_7c15);
    (0..44)
        .map(|_| ALPHABET[(rng.next() % ALPHABET.len() as u64) as usize] as char)
        .collect()
}

fn balance(rng: &mut Rng, slot: u64) -> Value {
    json!({
        "amount": (rng.next() % 1_000_000_000_000_000).to_string(),
        "delegated_amount": (rng.next() % 1_000).to_string(),
        "state": "initialized",
        "native_reserve": null,
        "updated_slot": slot,
    })
}

fn row(key: u64, rng: &mut Rng, slot: u64) -> Value {
    json!({
        "id": {
            "address": address(key),
            "owner": address(key + 1_000_000),
            "mint": address(key % 64 + 2_000_000),
            "token_program": "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
        },
        "authority": {
            "delegate": if key.is_multiple_of(7) { json!(address(key + 3_000_000)) } else { Value::Null },
            "close_authority": Value::Null,
        },
        "balance": balance(rng, slot),
        "activity": {
            "transfers_in": rng.next() % 100,
            "transfers_out": if key.is_multiple_of(3) { rng.next() % 100 } else { 0 },
            "last_signature": address(rng.next()),
            "last_slot": slot,
        },
    })
}

fn view(id: &str, mode: Mode, pipeline: Option<ViewPipeline>) -> ViewSpec {
    ViewSpec {
        id: id.to_string(),
        export: EXPORT.to_string(),
        mode,
        wire_format: Default::default(),
        projection: Projection::all(),
        filters: Filters::all(),
        delivery: Delivery::default(),
        source_view: pipeline.as_ref().map(|_| format!("{EXPORT}/list")),
        pipeline,
    }
}

fn sort(field: &[&str], order: SortOrder) -> Option<SortConfig> {
    Some(SortConfig {
        field_path: field.iter().map(|segment| segment.to_string()).collect(),
        order,
    })
}

fn index() -> ViewIndex {
    let mut index = ViewIndex::new();
    index.add_spec(view(&format!("{EXPORT}/list"), Mode::List, None));
    index.add_spec(view(&format!("{EXPORT}/state"), Mode::State, None));
    index.add_spec(view(
        &format!("{EXPORT}/byAddress"),
        Mode::List,
        Some(ViewPipeline {
            filter: None,
            sort: sort(&["id", "address"], SortOrder::Asc),
            limit: None,
        }),
    ));
    index.add_spec(view(
        &format!("{EXPORT}/richest"),
        Mode::List,
        Some(ViewPipeline {
            filter: None,
            sort: sort(&["balance", "amount"], SortOrder::Desc),
            limit: Some(25),
        }),
    ));
    index.add_spec(view(
        &format!("{EXPORT}/busiest"),
        Mode::List,
        Some(ViewPipeline {
            filter: Some(FilterConfig {
                field_path: vec!["activity".to_string(), "transfers_out".to_string()],
                op: CompareOp::Gt,
                value: json!(0),
            }),
            sort: sort(&["activity", "transfers_out"], SortOrder::Desc),
            limit: Some(25),
        }),
    ));
    index
}

fn mutation(key: u64, patch: Value) -> Mutation {
    Mutation {
        export: EXPORT.to_string(),
        key: json!(key.to_string()),
        patch,
        append: vec![],
    }
}

async fn flush(tx: &mpsc::Sender<MutationBatch>) {
    let (ack, wait) = tokio::sync::oneshot::channel();
    tx.send(MutationBatch::flush_marker(ack)).await.unwrap();
    tokio::time::timeout(Duration::from_secs(600), wait)
        .await
        .expect("the projector acks the flush")
        .unwrap();
}

async fn send(tx: &mpsc::Sender<MutationBatch>, slot: &mut u64, mutations: Vec<Mutation>) {
    let mut mutations = mutations.into_iter().peekable();
    while mutations.peek().is_some() {
        *slot += 1;
        tx.send(MutationBatch::with_slot_context(
            mutations.by_ref().take(BATCH).collect(),
            SlotContext::new(*slot, 0),
        ))
        .await
        .unwrap();
    }
    flush(tx).await;
}

/// The updates: each touches one key, with a `balance` patch and, two times
/// in five, the whole row as well (about 1.4 patches per update).
fn updates(keys: u64, rng: &mut Rng, slot: u64) -> (Vec<Mutation>, u64) {
    let mut mutations = Vec::new();
    for update in 0..UPDATES {
        let key = rng.next() % keys;
        let slot = slot + update / BATCH as u64;
        if rng.next() % 5 < 2 {
            mutations.push(mutation(key, row(key, rng, slot)));
        }
        mutations.push(mutation(key, json!({"balance": balance(rng, slot)})));
    }
    let patches = mutations.len() as u64;
    (mutations, patches)
}

struct Measured {
    per_update: Duration,
    allocations_per_update: f64,
    patches_per_update: f64,
}

async fn projector_run(keys: u64) -> Measured {
    let index = index();
    let (tx, rx) = mpsc::channel(1_024);
    let projector = tokio::spawn(
        Projector::new(
            Arc::new(index.clone()),
            BusManager::new(),
            // Room for every key, as a stack that holds all its rows has.
            EntityCache::with_config(EntityCacheConfig {
                max_entities_per_view: CACHED_ENTITIES,
                ..EntityCacheConfig::default()
            }),
            rx,
            #[cfg(feature = "otel")]
            None,
        )
        .run(),
    );
    let mut rng = Rng(keys);
    let mut slot = 300_000_000;
    let creations = (0..keys)
        .map(|key| {
            let mut mutation = mutation(key, row(key, &mut rng, slot));
            mutation.mark_created();
            mutation
        })
        .collect();
    send(&tx, &mut slot, creations).await;

    let mut fastest = Duration::MAX;
    let mut allocations = 0;
    let mut patches = 0;
    for _ in 0..ROUNDS {
        let (mutations, round_patches) = updates(keys, &mut rng, slot);
        let before = ALLOCATIONS.load(Ordering::Relaxed);
        let started = Instant::now();
        send(&tx, &mut slot, mutations).await;
        fastest = fastest.min(started.elapsed());
        allocations += ALLOCATIONS.load(Ordering::Relaxed) - before;
        patches += round_patches;
    }

    drop(tx);
    projector.await.unwrap();
    let updates = UPDATES as f64 * ROUNDS as f64;
    Measured {
        per_update: fastest / UPDATES as u32,
        allocations_per_update: allocations as f64 / updates,
        patches_per_update: patches as f64 / updates,
    }
}

/// A sorted cache read the way a subscriber's top-25 window reads it after
/// each change that moves a row.
fn window_reads(keys: u64) -> Duration {
    let mut rng = Rng(keys + 1);
    let mut cache = SortedViewCache::new(
        format!("{EXPORT}/richest"),
        vec!["balance".to_string(), "amount".to_string()],
        CacheOrder::Desc,
    );
    for key in 0..keys {
        cache.upsert(key.to_string(), row(key, &mut rng, 0));
    }
    let reads = 1_000;
    let mut fastest = Duration::MAX;
    for round in 0..ROUNDS as u64 {
        let rows: Vec<(u64, Value)> = (0..reads)
            .map(|_| {
                let key = rng.next() % keys;
                (key, row(key, &mut rng, round))
            })
            .collect();
        let started = Instant::now();
        for (key, row) in rows {
            cache.upsert(key.to_string(), row);
            std::hint::black_box(cache.get_window(0, 25));
        }
        fastest = fastest.min(started.elapsed());
    }
    fastest / reads as u32
}

fn main() {
    // `cargo test --benches` runs this with `--bench` absent; keep it quick.
    let quick = !std::env::args().any(|arg| arg == "--bench");
    // `VIEW_BENCH_KEYS=5000` measures one size, e.g. under a profiler.
    let sizes: Vec<u64> = match std::env::var("VIEW_BENCH_KEYS") {
        Ok(keys) => keys
            .split(',')
            .filter_map(|keys| keys.parse().ok())
            .collect(),
        Err(_) if quick => vec![200],
        Err(_) => vec![1_000, 2_500, 5_000],
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    println!("| keys | patches/update | projector time/update | allocations/update | upsert + top-25 window read |");
    println!("| ---: | ---: | ---: | ---: | ---: |");
    for keys in sizes {
        let measured = runtime.block_on(projector_run(keys));
        let window = window_reads(keys);
        println!(
            "| {keys} | {:.2} | {:.1} µs | {:.0} | {:.1} µs |",
            measured.patches_per_update,
            measured.per_update.as_secs_f64() * 1e6,
            measured.allocations_per_update,
            window.as_secs_f64() * 1e6,
        );
    }
}
