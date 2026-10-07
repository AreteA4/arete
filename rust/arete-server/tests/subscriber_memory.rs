//! How much heap a server spends on cached entities per subscriber, per
//! snapshot and per frame, beyond the caches themselves.
//!
//! A counting global allocator tracks live, peak and total allocated heap
//! bytes. The test feeds entities shaped like a busy on-chain account (80
//! nested trade objects each) through the real projector, then measures:
//!
//! - writing a state snapshot (`SnapshotService::snapshot_now`);
//! - state subscribers to one key, against subscribers to a key that does not
//!   exist (the connection and subscription overhead alone);
//! - list subscribers taking their snapshot at once;
//! - updates delivered to state, list and derived-view subscribers.
//!
//! Every number includes the in-process clients, which do the same work
//! whatever the server does. It uses only the public API, so the same file
//! measures earlier versions too:
//!
//! ```text
//! cargo test -p arete-server --test subscriber_memory -- --nocapture
//! ```

use std::alloc::{GlobalAlloc, Layout, System};
use std::io::Read;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use arete_interpreter::vm::VmContext;
use arete_interpreter::Mutation;
use arete_server::journal::{EventJournal, JournalConfig};
use arete_server::materialized_view::{SortConfig, SortOrder, ViewPipeline};
use arete_server::snapshot::{SnapshotConfig, SnapshotService, SnapshotTrigger};
use arete_server::{
    BusManager, Delivery, EntityCache, Filters, Mode, MutationBatch, Projection, Projector,
    SlotContext, SlotTracker, Spec, ViewIndex, ViewSpec, WebSocketServer,
};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

struct CountingAllocator;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static TOTAL: AtomicUsize = AtomicUsize::new(0);

fn grew(by: usize) {
    TOTAL.fetch_add(by, Ordering::Relaxed);
    let live = LIVE.fetch_add(by, Ordering::Relaxed) + by;
    PEAK.fetch_max(live, Ordering::Relaxed);
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            grew(layout.size());
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
                grew(new_size - layout.size());
            } else {
                LIVE.fetch_sub(layout.size() - new_size, Ordering::Relaxed);
            }
        }
        moved
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

const ENTITIES: u64 = 100;
const TRADES_PER_ENTITY: u64 = 80;
const STATE_SUBSCRIBERS: usize = 40;
const LIST_SUBSCRIBERS: usize = 8;
const DERIVED_SUBSCRIBERS: usize = 8;
const DERIVED_WINDOW: usize = 20;
const UPDATE_ROUNDS: u64 = 3;
const HOT_KEY_UPDATES: u64 = 30;
const QUIET: Duration = Duration::from_millis(400);

fn live() -> usize {
    LIVE.load(Ordering::Relaxed)
}

fn total() -> usize {
    TOTAL.load(Ordering::Relaxed)
}

fn reset_peak() -> usize {
    let now = live();
    PEAK.store(now, Ordering::Relaxed);
    now
}

fn kib(bytes: usize) -> f64 {
    bytes as f64 / 1024.0
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
        "id": id.to_string(),
        "authority": address(id + 7, 44),
        "lastSlot": 300_000_000,
        "metrics": {"volume": id * 1_000, "fees": id * 3, "tradeCount": TRADES_PER_ENTITY},
        "config": {"feeBps": 25, "paused": false},
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

fn index() -> ViewIndex {
    let mut index = ViewIndex::new();
    for id in ["Pool/list", "Pool/state", "Pool/append"] {
        index.add_spec(view(id, None));
    }
    index.add_spec(view(
        "Pool/byVolume",
        Some(ViewPipeline {
            filter: None,
            sort: Some(SortConfig {
                field_path: vec!["metrics".to_string(), "volume".to_string()],
                order: SortOrder::Desc,
            }),
            limit: None,
        }),
    ));
    index
}

fn spec() -> Spec {
    use arete_interpreter::ast::{IdentitySpec, TypedStreamSpec};

    let entity_spec = TypedStreamSpec::<Value>::new(
        "Pool".to_string(),
        IdentitySpec {
            primary_keys: vec!["id".to_string()],
            lookup_indexes: Vec::new(),
        },
        Vec::new(),
    );
    let serializable = entity_spec.to_serializable();
    let bytecode = arete_interpreter::compiler::MultiEntityBytecode::new()
        .add_entity("Pool".to_string(), entity_spec, 1)
        .build();
    Spec::new(bytecode, "Program111").with_entity_specs(vec![serializable])
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
            mutations.by_ref().take(25).collect(),
            SlotContext::new(slot, offset),
        ))
        .await
        .unwrap();
        offset += 1;
    }
    flush(tx).await;
}

fn update(id: u64, round: u64) -> Mutation {
    Mutation {
        export: "Pool".to_string(),
        key: json!(id.to_string()),
        patch: json!({
            "lastSlot": 300_000_000 + round,
            "metrics": {"volume": id * 1_000 + round * 7},
            "trades": [trade(id, TRADES_PER_ENTITY + round)],
        }),
        append: vec!["trades".to_string()],
    }
}

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// A frame as JSON, inflating a compressed one.
fn decode(message: &Message) -> Option<Value> {
    let bytes: Vec<u8> = match message {
        Message::Text(text) => text.as_bytes().to_vec(),
        Message::Binary(bytes) if bytes.starts_with(&[0x1f, 0x8b]) => {
            let mut inflated = Vec::new();
            flate2::read::GzDecoder::new(bytes.as_ref())
                .read_to_end(&mut inflated)
                .ok()?;
            inflated
        }
        Message::Binary(bytes) => bytes.to_vec(),
        _ => return None,
    };
    serde_json::from_slice(&bytes).ok()
}

/// Connect, subscribe, and read up to the end of the snapshot.
async fn subscribe(addr: SocketAddr, id: usize, query: Value) -> Socket {
    let (mut socket, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/"))
        .await
        .expect("the server accepts");
    socket
        .send(Message::Text(
            json!({
                "type": "subscribe",
                "protocolVersion": 2,
                "subscriptionId": format!("s{id}"),
                "query": query,
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();
    loop {
        let message = tokio::time::timeout(Duration::from_secs(30), socket.next())
            .await
            .expect("the server answers")
            .expect("the socket stays open")
            .expect("a readable frame");
        let Some(frame) = decode(&message) else {
            continue;
        };
        if frame["op"] == "snapshot" && frame["complete"] == true {
            return socket;
        }
    }
}

/// Read frames until `done` is set and the socket has been quiet for
/// [`QUIET`], counting them without decoding.
async fn drain(mut socket: Socket, done: Arc<AtomicBool>) -> (Socket, usize) {
    let mut frames = 0;
    loop {
        match tokio::time::timeout(QUIET, socket.next()).await {
            Ok(Some(Ok(_))) => frames += 1,
            Ok(_) => break,
            Err(_) if done.load(Ordering::Relaxed) => break,
            Err(_) => {}
        }
    }
    (socket, frames)
}

fn free_addr() -> SocketAddr {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn subscribers_and_snapshots_hold_no_entity_copies_of_their_own() {
    // One standalone copy of every entity, as the yardstick.
    let start = live();
    let standalone: Vec<Value> = (0..ENTITIES).map(entity).collect();
    let copy_bytes = live() - start;
    let json_bytes: usize = standalone
        .iter()
        .map(|entity| serde_json::to_vec(entity).unwrap().len())
        .sum();
    drop(standalone);
    let entity_bytes = copy_bytes / ENTITIES as usize;

    let snapshot_dir =
        std::env::temp_dir().join(format!("arete-subscriber-memory-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&snapshot_dir);

    let index = index();
    let cache = EntityCache::new();
    let bus = BusManager::new();
    let (tx, rx) = mpsc::channel(1_024);
    let snapshots = SnapshotService::initialize(
        SnapshotConfig {
            enabled: true,
            url: Some(snapshot_dir.display().to_string()),
            interval: Duration::from_secs(3_600),
            ..SnapshotConfig::default()
        },
        &spec(),
        cache.clone(),
        &index,
        Arc::new(EventJournal::new(JournalConfig::default())),
        tx.clone(),
    )
    .await
    .unwrap();
    tokio::spawn(
        Projector::new(
            Arc::new(index.clone()),
            bus.clone(),
            cache.clone(),
            rx,
            #[cfg(feature = "otel")]
            None,
        )
        .with_snapshot_runtime(snapshots.runtime())
        .run(),
    );
    snapshots.runtime().register_runtime(
        Arc::new(StdMutex::new(VmContext::new())),
        SlotTracker::new(),
    );

    let addr = free_addr();
    tokio::spawn(
        WebSocketServer::new(
            addr,
            bus.clone(),
            cache.clone(),
            Arc::new(index.clone()),
            #[cfg(feature = "otel")]
            None,
        )
        .start(),
    );

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
    // The server must be listening before the first client connects.
    for _ in 0..100 {
        if TcpStream::connect(addr).await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Writing a state snapshot of the three views.
    let before = reset_peak();
    assert!(snapshots
        .snapshot_now(SnapshotTrigger::Periodic)
        .await
        .unwrap());
    let snapshot_write_peak = PEAK.load(Ordering::Relaxed) - before;
    let _ = std::fs::remove_dir_all(&snapshot_dir);

    // State subscribers to a key that does not exist: the overhead of a
    // connection and a subscription, with no entity to hold.
    let before = live();
    let mut idle = Vec::new();
    for id in 0..STATE_SUBSCRIBERS {
        idle.push(subscribe(addr, id, json!({"view": "Pool/state", "key": "missing"})).await);
    }
    let idle_bytes = live() - before;

    // State subscribers to one key.
    let before = live();
    let mut state = Vec::new();
    for id in 0..STATE_SUBSCRIBERS {
        state.push(subscribe(addr, id, json!({"view": "Pool/state", "key": "0"})).await);
    }
    let state_bytes = live() - before;
    let per_state_subscriber = state_bytes.saturating_sub(idle_bytes) / STATE_SUBSCRIBERS;

    // List subscribers taking the whole list at once.
    let before = reset_peak();
    let lists = futures_util::future::join_all(
        (0..LIST_SUBSCRIBERS).map(|id| subscribe(addr, 100 + id, json!({"view": "Pool/list"}))),
    )
    .await;
    let list_snapshot_peak = PEAK.load(Ordering::Relaxed) - before;

    let mut derived = Vec::new();
    for id in 0..DERIVED_SUBSCRIBERS {
        derived.push(
            subscribe(
                addr,
                200 + id,
                json!({"view": "Pool/byVolume", "take": DERIVED_WINDOW}),
            )
            .await,
        );
    }

    // Updates, delivered to every subscriber as they arrive.
    let done = Arc::new(AtomicBool::new(false));
    let readers: Vec<_> = state
        .into_iter()
        .chain(lists)
        .chain(derived)
        .map(|socket| tokio::spawn(drain(socket, done.clone())))
        .collect();
    let allocated_before = total();
    let before = reset_peak();
    for round in 1..=UPDATE_ROUNDS {
        let updates = (0..ENTITIES).map(|id| update(id, round)).collect();
        send(&tx, 300_000_000 + round, updates).await;
    }
    for round in 1..=HOT_KEY_UPDATES {
        send(&tx, 300_001_000 + round, vec![update(0, 100 + round)]).await;
    }
    done.store(true, Ordering::Relaxed);
    let mut frames = 0;
    for reader in readers {
        frames += reader.await.unwrap().1;
    }
    let update_allocated = total() - allocated_before;
    let update_peak = PEAK.load(Ordering::Relaxed) - before;

    println!(
        "{ENTITIES} entities, {:.1} KiB of JSON and {:.1} KiB of heap each",
        kib(json_bytes) / ENTITIES as f64,
        kib(entity_bytes),
    );
    println!("| measure | heap | in entity copies |");
    println!("| --- | ---: | ---: |");
    let row = |label: &str, bytes: usize, unit: usize| {
        println!(
            "| {label} | {:.1} KiB | {:.2} |",
            kib(bytes),
            bytes as f64 / unit as f64
        );
    };
    row(
        "snapshot write, peak (3 views)",
        snapshot_write_peak,
        copy_bytes,
    );
    row(
        "per state subscriber, retained beyond an idle one",
        per_state_subscriber,
        entity_bytes,
    );
    row(
        &format!("{LIST_SUBSCRIBERS} list snapshots at once, peak"),
        list_snapshot_peak,
        copy_bytes,
    );
    row(
        &format!("updates, allocated per frame delivered ({frames} frames)"),
        update_allocated / frames.max(1),
        entity_bytes,
    );
    row("updates, peak", update_peak, copy_bytes);

    drop(idle);
    assert!(frames > 0, "subscribers received the updates");
    assert!(
        snapshot_write_peak < copy_bytes,
        "writing a snapshot of three views should not copy their entities, peak {:.2} copies",
        snapshot_write_peak as f64 / copy_bytes as f64
    );
    assert!(
        per_state_subscriber < entity_bytes / 2,
        "a state subscriber should hold no copy of its entity, held {:.2} copies",
        per_state_subscriber as f64 / entity_bytes as f64
    );
}
