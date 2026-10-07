//! How much heap one cached entity costs across the views that hold it.
//!
//! A counting global allocator tracks live heap bytes. The test feeds entities
//! shaped like a busy on-chain account (scalar fields, a few nested objects,
//! an array of 80 small trade objects that every update appends to) through
//! the real projector into the views a server registers for an entity (list,
//! state, append) and three derived sorted views over the list, then applies
//! rounds of patches. It reports the bytes the caches retain per entity, in
//! units of one standalone copy of the entity, and the peak while updates
//! were applied.
//!
//! It uses only the public API, so the same file measures earlier versions
//! too:
//!
//! ```text
//! cargo test -p arete-server --test cache_memory -- --nocapture
//! ```

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use arete_interpreter::Mutation;
use arete_server::materialized_view::{
    CompareOp, FilterConfig, SortConfig, SortOrder, ViewPipeline,
};
use arete_server::{
    BusManager, Delivery, EntityCache, Filters, Mode, MutationBatch, Projection, Projector,
    SlotContext, ViewIndex, ViewSpec,
};
use serde_json::{json, Value};
use tokio::sync::mpsc;

struct CountingAllocator;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            let live = LIVE.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
            PEAK.fetch_max(live, Ordering::Relaxed);
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
                let live = LIVE.fetch_add(new_size - layout.size(), Ordering::Relaxed) + new_size
                    - layout.size();
                PEAK.fetch_max(live, Ordering::Relaxed);
            } else {
                LIVE.fetch_sub(layout.size() - new_size, Ordering::Relaxed);
            }
        }
        moved
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

const ENTITIES: u64 = 200;
const TRADES_PER_ENTITY: u64 = 80;
const UPDATE_ROUNDS: u64 = 3;
const BATCH: usize = 50;

fn live() -> usize {
    LIVE.load(Ordering::Relaxed)
}

fn address(seed: u64, length: usize) -> String {
    const ALPHABET: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    let mut state = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    (0..length)
        .map(|_| {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ALPHABET[(state >> 33) as usize % ALPHABET.len()] as char
        })
        .collect()
}

fn trade(entity: u64, index: u64) -> Value {
    json!({
        "signature": address(entity * 1_000 + index, 88),
        "side": if index.is_multiple_of(3) { "sell" } else { "buy" },
        "price": 1_000_000 + entity * 37 + index,
        "amount": 5_000 + index * 11,
        "slot": 300_000_000 + index,
    })
}

fn entity(id: u64) -> Value {
    json!({
        "id": {"address": address(id, 44), "index": id},
        "active": id.is_multiple_of(2),
        "createdAt": 1_700_000_000 + id,
        "lastSlot": 300_000_000,
        "authority": address(id + 7, 44),
        "metrics": {
            "volume": id * 1_000,
            "fees": id * 3,
            "tradeCount": TRADES_PER_ENTITY,
            "lastPrice": format!("{}.{:06}", id, id * 13 % 1_000_000),
        },
        "config": {"feeBps": 25, "paused": false, "curve": {"a": 100, "b": 200}},
        "trades": (0..TRADES_PER_ENTITY).map(|index| trade(id, index)).collect::<Vec<_>>(),
    })
}

fn view(id: &str, pipeline: Option<ViewPipeline>) -> ViewSpec {
    let mode = match id.rsplit('/').next() {
        Some("state") => Mode::State,
        Some("append") => Mode::Append,
        _ => Mode::List,
    };
    ViewSpec {
        id: id.to_string(),
        export: "Pool".to_string(),
        mode,
        wire_format: Default::default(),
        projection: Projection::all(),
        filters: Filters::all(),
        delivery: Delivery::default(),
        source_view: pipeline.as_ref().map(|_| "Pool/list".to_string()),
        pipeline,
    }
}

fn sorted(field: &[&str], order: SortOrder, filter: Option<FilterConfig>) -> Option<ViewPipeline> {
    Some(ViewPipeline {
        filter,
        sort: Some(SortConfig {
            field_path: field.iter().map(|segment| segment.to_string()).collect(),
            order,
        }),
        limit: None,
    })
}

fn index() -> ViewIndex {
    let mut index = ViewIndex::new();
    for id in ["Pool/list", "Pool/state", "Pool/append"] {
        index.add_spec(view(id, None));
    }
    index.add_spec(view(
        "Pool/byVolume",
        sorted(&["metrics", "volume"], SortOrder::Desc, None),
    ));
    index.add_spec(view(
        "Pool/newest",
        sorted(&["_seq"], SortOrder::Desc, None),
    ));
    index.add_spec(view(
        "Pool/active",
        sorted(
            &["createdAt"],
            SortOrder::Asc,
            Some(FilterConfig {
                field_path: vec!["active".to_string()],
                op: CompareOp::Eq,
                value: json!(true),
            }),
        ),
    ));
    index
}

async fn flush(tx: &mpsc::Sender<MutationBatch>) {
    let (ack, wait) = tokio::sync::oneshot::channel();
    tx.send(MutationBatch::flush_marker(ack)).await.unwrap();
    tokio::time::timeout(Duration::from_secs(60), wait)
        .await
        .expect("the projector acks the flush")
        .unwrap();
}

async fn send(tx: &mpsc::Sender<MutationBatch>, slot: u64, mutations: Vec<Mutation>) {
    let mut mutations = mutations.into_iter().peekable();
    let mut offset = 0;
    while mutations.peek().is_some() {
        tx.send(MutationBatch::with_slot_context(
            mutations.by_ref().take(BATCH).collect(),
            SlotContext::new(slot, offset),
        ))
        .await
        .unwrap();
        offset += 1;
    }
    flush(tx).await;
}

#[tokio::test(flavor = "current_thread")]
async fn cached_entities_cost_about_one_copy_across_views() {
    // One standalone copy of every entity, as the yardstick.
    let start = live();
    let standalone: Vec<Value> = (0..ENTITIES).map(entity).collect();
    let copy_bytes = live() - start;
    let json_bytes: usize = standalone
        .iter()
        .map(|entity| serde_json::to_vec(entity).unwrap().len())
        .sum();
    drop(standalone);

    let index = index();
    let cache = EntityCache::new();
    let (tx, rx) = mpsc::channel(1_024);
    let projector = tokio::spawn(
        Projector::new(
            Arc::new(index.clone()),
            BusManager::new(),
            cache.clone(),
            rx,
            #[cfg(feature = "otel")]
            None,
        )
        .run(),
    );
    flush(&tx).await;
    let before = live();

    let creations = (0..ENTITIES)
        .map(|id| {
            let mut mutation = Mutation {
                export: "Pool".to_string(),
                key: json!(id.to_string()),
                patch: entity(id),
                append: vec![],
            };
            mutation.mark_created();
            mutation
        })
        .collect();
    send(&tx, 300_000_000, creations).await;
    let created = live() - before;

    PEAK.store(live(), Ordering::Relaxed);
    for round in 1..=UPDATE_ROUNDS {
        let updates = (0..ENTITIES)
            .map(|id| Mutation {
                export: "Pool".to_string(),
                key: json!(id.to_string()),
                patch: json!({
                    "lastSlot": 300_000_000 + round,
                    "metrics": {"volume": id * 1_000 + round * 7},
                    "trades": [trade(id, TRADES_PER_ENTITY + round)],
                }),
                append: vec!["trades".to_string()],
            })
            .collect();
        send(&tx, 300_000_000 + round, updates).await;
    }
    let updated = live() - before;
    let peak = PEAK.load(Ordering::Relaxed) - before;

    // The updates grew every entity: measure one standalone copy again.
    let start = live();
    let mut copies_now = Vec::new();
    for id in 0..ENTITIES {
        copies_now.push(cache.get("Pool/list", &id.to_string()).await.unwrap());
    }
    let updated_copy_bytes = live() - start;
    drop(copies_now);

    let sorted_caches = index.sorted_caches();
    let mut derived_rows = 0;
    for cache in sorted_caches.read().await.values() {
        derived_rows += cache.len();
    }
    let views = cache.stats().await;

    let per_entity = |bytes: usize| bytes as f64 / ENTITIES as f64 / 1024.0;
    println!(
        "{} entities, {:.1} KiB of JSON and {:.1} KiB of heap each ({:.1} KiB after the \
         updates); {} rows in {} views, {} in derived sorted views",
        ENTITIES,
        json_bytes as f64 / ENTITIES as f64 / 1024.0,
        per_entity(copy_bytes),
        per_entity(updated_copy_bytes),
        views.total_entities,
        views.view_count,
        derived_rows,
    );
    println!("| after | heap per entity | copies of the entity |");
    println!("| --- | ---: | ---: |");
    for (label, bytes, copy) in [
        ("creation", created, copy_bytes),
        ("update rounds", updated, updated_copy_bytes),
        ("peak during updates", peak, updated_copy_bytes),
    ] {
        println!(
            "| {label} | {:.1} KiB | {:.2} |",
            per_entity(bytes),
            bytes as f64 / copy as f64
        );
    }
    let copies = updated as f64 / updated_copy_bytes as f64;

    drop(tx);
    projector.await.unwrap();

    assert_eq!(views.total_entities as u64, ENTITIES * 3);
    assert_eq!(derived_rows as u64, ENTITIES * 2 + ENTITIES / 2);
    assert!(
        copies < 1.5,
        "six views should hold about one copy of each entity, not {copies:.2}"
    );
}
