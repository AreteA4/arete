//! Real-socket delivery profiles for latest-state list views.
//!
//! Each scenario seeds a token-account-shaped list view, subscribes real
//! WebSocket clients, publishes a deterministic mutation burst through the
//! projector, and reads every socket until the client's merged state equals
//! the publisher's model. Waiting is driven by projector flush markers and
//! socket events under one deadline; nothing sleeps to decide an outcome.
//!
//! The summary keeps source updates, delivered entity operations, physical
//! WebSocket messages, bytes, recovery snapshots and disconnects apart, so a
//! later delivery change can be compared with this baseline one quantity at a
//! time. `smoke` is the small always-run correctness gate. `profiles` runs the
//! four capacity scenarios and is ignored by default: wall-clock numbers are
//! reported, never asserted. See
//! `docs/internal/websocket-delivery-load-baseline.md`.

use crate::bus::BusManager;
use crate::cache::{EntityCache, EntityCacheConfig};
use crate::compression::is_gzip;
use crate::projector::Projector;
use crate::view::{Delivery, Filters, Projection, ViewIndex, ViewSpec};
use crate::websocket::client_manager::RateLimitConfig;
use crate::websocket::frame::{apply_wire_format, Mode, WireFormat};
use crate::websocket::server::{ConnectionAcceptor, DeliveryProbe};
use crate::websocket::usage::{ChannelUsageEmitter, WebSocketUsageEvent};
use crate::websocket::WebSocketServer;
use crate::{MutationBatch, SlotContext, WebSocketDeliveryConfig};
use arete_interpreter::Mutation;
use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Read;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::Handle;
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tokio::time::{timeout, timeout_at, Instant};
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{client_async, WebSocketStream};

const VIEW: &str = "TokenAccount/list";
const EXPORT: &str = "TokenAccount";
const SUBSCRIPTION_ID: &str = "token-accounts";

/// The delivery settings every profile runs with: 100 ms collection
/// coalescing, with a list bus and per-client queue large enough that a
/// reader that keeps draining never lags.
const COALESCE_MS: u64 = 100;
const LIST_BUS_CAPACITY: usize = 16_384;
const MESSAGE_QUEUE_SIZE: usize = 2_048;
/// The runtime's projector channel (`runtime.rs`).
const PROJECTOR_CHANNEL: usize = 1_024;

/// Paced publishers release one chunk of updates per tick.
const PACING_TICK: Duration = Duration::from_millis(10);
const STOP_TIMEOUT: Duration = Duration::from_secs(30);
const QUIESCENCE_GRACE: Duration = Duration::from_secs(1);
const SUMMARY_VERSION: u32 = 1;

const SEED_SLOT: u64 = 300_000_000;
const BURST_SLOT: u64 = 300_100_000;
const UPDATES_PER_SLOT: usize = 400;
const MINT_DECIMALS: [u32; 8] = [6, 9, 6, 5, 8, 6, 9, 2];
const HOT_PATTERN_SEED: u64 = 0xA4_0120;

type Rows = HashMap<String, Value>;
type Socket = WebSocketStream<TcpStream>;

// ---------------------------------------------------------------------------
// Scenarios
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
enum KeyPattern {
    /// 80% of updates go to the first 10% of keys; the rest land anywhere.
    Hot,
    /// Every key is updated exactly once, in key order.
    Unique,
}

impl KeyPattern {
    fn key_index(self, ordinal: usize, keys: usize) -> usize {
        match self {
            Self::Hot => {
                let draw = mix(HOT_PATTERN_SEED ^ ordinal as u64);
                let hot = (keys / 10).max(1) as u64;
                let span = if draw % 10 < 8 { hot } else { keys as u64 };
                ((draw >> 8) % span) as usize
            }
            Self::Unique => ordinal % keys,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
enum OnDisconnect {
    /// Any disconnect fails the scenario.
    Fail,
    /// Reconnect and resubscribe like an SDK. The fresh authoritative
    /// snapshot must still reach the exact final state.
    Reconnect,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DeliveryProfile {
    collection_coalesce_ms: Option<u64>,
    list_bus_capacity: usize,
    message_queue_size: usize,
    /// Explicit so the list can hold every seeded row. A runtime built with
    /// `EntityCache::new()` retains 500 entities per view.
    cache_entities_per_view: usize,
    snapshot_initial_batch: usize,
    snapshot_subsequent_batch: usize,
}

impl DeliveryProfile {
    fn coalesced(rows: usize) -> Self {
        let cache = EntityCacheConfig::default();
        Self {
            collection_coalesce_ms: Some(COALESCE_MS),
            list_bus_capacity: LIST_BUS_CAPACITY,
            message_queue_size: MESSAGE_QUEUE_SIZE,
            cache_entities_per_view: rows.max(1),
            snapshot_initial_batch: cache.initial_snapshot_batch_size,
            snapshot_subsequent_batch: cache.subsequent_snapshot_batch_size,
        }
    }

    fn expected_snapshot_frames(&self, rows: usize) -> u64 {
        if rows <= self.snapshot_initial_batch {
            return 1;
        }
        let subsequent = self.snapshot_subsequent_batch.max(1);
        1 + (rows - self.snapshot_initial_batch).div_ceil(subsequent) as u64
    }
}

#[derive(Debug, Clone)]
struct Scenario {
    name: &'static str,
    /// Rows seeded before anyone subscribes; every key the burst touches.
    keys: usize,
    source_updates: usize,
    pattern: KeyPattern,
    subscribers: usize,
    /// Zero publishes as fast as the projector accepts.
    source_rate_per_sec: u64,
    /// How long each subscriber stops reading once the burst starts.
    read_pause_ms: u64,
    on_disconnect: OnDisconnect,
    max_reconnects: u64,
    timeout_secs: u64,
    /// Assert that server, usage and client accounting agree exactly. Only
    /// meaningful when no subscriber can lag or disconnect.
    exact_accounting: bool,
    delivery: DeliveryProfile,
}

fn smoke_scenario() -> Scenario {
    Scenario {
        name: "smoke",
        keys: 128,
        source_updates: 2_000,
        pattern: KeyPattern::Hot,
        subscribers: 2,
        // Paced like the profiles so the burst spans several flushes.
        source_rate_per_sec: 10_000,
        read_pause_ms: 0,
        on_disconnect: OnDisconnect::Fail,
        max_reconnects: 0,
        timeout_secs: 30,
        exact_accounting: true,
        delivery: DeliveryProfile::coalesced(128),
    }
}

/// Workload sizes for `profiles`. The defaults are the reproducible baseline;
/// every override is recorded in the summary it affects.
struct ProfileSizes {
    hot_keys: usize,
    hot_updates: usize,
    unique_updates: usize,
    fanout_subscribers: usize,
    slow_reader_pause_ms: u64,
    source_rate_per_sec: u64,
    timeout_secs: u64,
    server_threads: usize,
    client_threads: usize,
    /// `ARETE_WS_LOAD_ONLY`: comma-separated scenario names, for local
    /// profiling of one scenario. The baseline runs all four.
    only: Option<Vec<String>>,
    overrides: Vec<String>,
}

impl ProfileSizes {
    fn from_env() -> Self {
        let mut overrides = Vec::new();
        let mut read = |name: &str, default: u64, minimum: u64| -> u64 {
            let Ok(raw) = std::env::var(name) else {
                return default;
            };
            let value: u64 = raw
                .trim()
                .parse()
                .unwrap_or_else(|_| panic!("{name} must be an integer, got {raw:?}"));
            assert!(value >= minimum, "{name} must be at least {minimum}");
            overrides.push(format!("{name}={value}"));
            value
        };
        let hot_keys = read("ARETE_WS_LOAD_HOT_KEYS", 3_000, 1) as usize;
        let hot_updates = read("ARETE_WS_LOAD_HOT_UPDATES", 30_000, 1) as usize;
        let unique_updates = read("ARETE_WS_LOAD_UNIQUE_UPDATES", 10_000, 1) as usize;
        let fanout_subscribers = read("ARETE_WS_LOAD_FANOUT_SUBSCRIBERS", 16, 1) as usize;
        let slow_reader_pause_ms = read("ARETE_WS_LOAD_SLOW_READER_PAUSE_MS", 2_000, 0);
        let source_rate_per_sec = read("ARETE_WS_LOAD_SOURCE_RATE", 10_000, 0);
        let timeout_secs = read("ARETE_WS_LOAD_TIMEOUT_SECS", 180, 1);
        let server_threads = read("ARETE_WS_LOAD_SERVER_THREADS", 4, 2) as usize;
        let client_threads = read("ARETE_WS_LOAD_CLIENT_THREADS", 4, 1) as usize;
        let only = std::env::var("ARETE_WS_LOAD_ONLY").ok().map(|raw| {
            overrides.push(format!("ARETE_WS_LOAD_ONLY={raw}"));
            raw.split(',').map(|name| name.trim().to_string()).collect()
        });
        Self {
            hot_keys,
            hot_updates,
            unique_updates,
            fanout_subscribers,
            slow_reader_pause_ms,
            source_rate_per_sec,
            timeout_secs,
            server_threads,
            client_threads,
            only,
            overrides,
        }
    }
}

fn profile_scenarios(sizes: &ProfileSizes) -> Vec<Scenario> {
    let hot = |name, subscribers, read_pause_ms| Scenario {
        name,
        keys: sizes.hot_keys,
        source_updates: sizes.hot_updates,
        pattern: KeyPattern::Hot,
        subscribers,
        source_rate_per_sec: sizes.source_rate_per_sec,
        read_pause_ms,
        on_disconnect: OnDisconnect::Reconnect,
        max_reconnects: 8,
        timeout_secs: sizes.timeout_secs,
        exact_accounting: false,
        delivery: DeliveryProfile::coalesced(sizes.hot_keys),
    };
    vec![
        hot("hot-keys", 1, 0),
        Scenario {
            name: "unique-burst",
            keys: sizes.unique_updates,
            source_updates: sizes.unique_updates,
            pattern: KeyPattern::Unique,
            delivery: DeliveryProfile::coalesced(sizes.unique_updates),
            ..hot("unique-burst", 1, 0)
        },
        hot("slow-reader", 1, sizes.slow_reader_pause_ms),
        hot("fan-out", sizes.fanout_subscribers, 0),
    ]
    .into_iter()
    .filter(|scenario| {
        sizes
            .only
            .as_ref()
            .is_none_or(|only| only.iter().any(|name| name == scenario.name))
    })
    .collect()
}

// ---------------------------------------------------------------------------
// Deterministic token-account workload
// ---------------------------------------------------------------------------

struct SourceUpdate {
    key: String,
    patch: Value,
    slot: SlotContext,
}

impl SourceUpdate {
    fn batch(&self) -> MutationBatch {
        MutationBatch::with_slot_context(
            std::iter::once(Mutation {
                export: EXPORT.to_string(),
                key: Value::String(self.key.clone()),
                patch: self.patch.clone(),
                append: vec![],
            })
            .collect(),
            self.slot,
        )
    }

    /// Fold this update into the model the way the projector folds it into
    /// the entity cache: `_seq` injected, objects merged recursively.
    fn apply_to(&self, rows: &mut Rows) {
        let mut patch = self.patch.clone();
        if let Value::Object(fields) = &mut patch {
            fields.insert("_seq".to_string(), Value::String(self.slot.to_seq_string()));
        }
        match rows.get_mut(&self.key) {
            Some(row) => merge_patch(row, patch, &[], ""),
            None => {
                rows.insert(self.key.clone(), patch);
            }
        }
    }
}

struct Workload {
    seed: Vec<SourceUpdate>,
    burst: Vec<SourceUpdate>,
    unique_keys: usize,
    /// Client-visible rows after seeding: what the initial snapshot carries.
    seed_rows: Arc<Rows>,
    /// Client-visible rows after the burst: what every subscriber must reach.
    final_rows: Arc<Rows>,
}

impl Workload {
    fn generate(scenario: &Scenario) -> Self {
        let addresses: Vec<String> = (0..scenario.keys)
            .map(|index| fixture_address("token-account", index))
            .collect();
        let slot = |ordinal: usize, base: u64| {
            SlotContext::new(
                base + (ordinal / UPDATES_PER_SLOT) as u64,
                (ordinal % UPDATES_PER_SLOT) as u64,
            )
        };
        let seed: Vec<_> = addresses
            .iter()
            .enumerate()
            .map(|(index, address)| {
                let slot = slot(index, SEED_SLOT);
                SourceUpdate {
                    key: address.clone(),
                    patch: seed_account(index, address, slot.slot),
                    slot,
                }
            })
            .collect();
        let burst: Vec<_> = (0..scenario.source_updates)
            .map(|ordinal| {
                let index = scenario.pattern.key_index(ordinal, scenario.keys);
                let slot = slot(ordinal, BURST_SLOT);
                SourceUpdate {
                    key: addresses[index].clone(),
                    patch: account_update(index, ordinal, slot.slot),
                    slot,
                }
            })
            .collect();
        let unique_keys = burst
            .iter()
            .map(|update| update.key.as_str())
            .collect::<HashSet<_>>()
            .len();

        let mut model = Rows::new();
        seed.iter().for_each(|update| update.apply_to(&mut model));
        let seed_rows = Arc::new(wire_rows(&model));
        burst.iter().for_each(|update| update.apply_to(&mut model));
        let final_rows = Arc::new(wire_rows(&model));
        Self {
            seed,
            burst,
            unique_keys,
            seed_rows,
            final_rows,
        }
    }

    fn fixture_sizes(&self) -> FixtureSizes {
        let mean = |values: &mut dyn Iterator<Item = usize>| {
            let (sum, count) = values.fold((0, 0), |(sum, count), size| (sum + size, count + 1));
            sum.checked_div(count).unwrap_or(0)
        };
        let json_len = |value: &Value| serde_json::to_vec(value).map_or(0, |bytes| bytes.len());
        FixtureSizes {
            seed_row_json_bytes: mean(&mut self.seed_rows.values().map(json_len)),
            source_patch_json_bytes: mean(&mut self.burst.iter().map(|update| {
                let mut patch = update.patch.clone();
                apply_wire_format(&mut patch, &wire_format());
                json_len(&patch)
            })),
        }
    }
}

fn wire_format() -> WireFormat {
    let path = |segments: &[&str]| segments.iter().map(|s| s.to_string()).collect();
    WireFormat {
        wide_int_paths: vec![
            path(&["state", "amount"]),
            path(&["state", "delegatedAmount"]),
            path(&["lastUpdatedSlot"]),
        ],
    }
}

fn wire_rows(model: &Rows) -> Rows {
    let format = wire_format();
    model
        .iter()
        .map(|(key, row)| {
            let mut row = row.clone();
            apply_wire_format(&mut row, &format);
            (key.clone(), row)
        })
        .collect()
}

/// splitmix64: a fixed, dependency-free source of repeatable variety.
fn mix(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9E37_79B9_7F4A_7C15);
    value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    value ^ (value >> 31)
}

/// A base58 string the length of a Solana address, derived from a label.
/// Generated, so no fixture refers to a real account.
fn fixture_address(kind: &str, index: usize) -> String {
    bs58::encode(Sha256::digest(format!("arete-ws-load/{kind}/{index}"))).into_string()
}

fn token_amount(index: usize, version: usize) -> u64 {
    mix(((index as u64) << 32) ^ version as u64) % 1_000_000_000_000
}

fn ui_amount(amount: u64, decimals: u32) -> String {
    if decimals == 0 {
        return amount.to_string();
    }
    let scale = 10_u64.pow(decimals);
    format!(
        "{}.{:0width$}",
        amount / scale,
        amount % scale,
        width = decimals as usize
    )
}

fn seed_account(index: usize, address: &str, slot: u64) -> Value {
    let mint = index % MINT_DECIMALS.len();
    let decimals = MINT_DECIMALS[mint];
    let amount = token_amount(index, 0);
    json!({
        "id": {"address": address},
        "state": {
            "mint": fixture_address("mint", mint),
            "owner": fixture_address("owner", index / 4),
            "amount": amount,
            "delegate": null,
            "state": "initialized",
            "isNative": null,
            "delegatedAmount": 0,
            "closeAuthority": null,
        },
        "tokenAmount": {"decimals": decimals, "uiAmountString": ui_amount(amount, decimals)},
        "lastUpdatedSlot": slot,
    })
}

/// A sparse balance change, the common TokenAccount mutation.
fn account_update(index: usize, ordinal: usize, slot: u64) -> Value {
    let decimals = MINT_DECIMALS[index % MINT_DECIMALS.len()];
    let amount = token_amount(index, ordinal + 1);
    json!({
        "state": {"amount": amount},
        "tokenAmount": {"uiAmountString": ui_amount(amount, decimals)},
        "lastUpdatedSlot": slot,
    })
}

fn token_account_view() -> ViewSpec {
    ViewSpec {
        id: VIEW.to_string(),
        export: EXPORT.to_string(),
        mode: Mode::List,
        wire_format: wire_format(),
        projection: Projection::all(),
        filters: Filters::all(),
        delivery: Delivery::default(),
        pipeline: None,
        source_view: None,
    }
}

// ---------------------------------------------------------------------------
// Server side
// ---------------------------------------------------------------------------

struct Harness {
    addr: SocketAddr,
    tx: mpsc::Sender<MutationBatch>,
    projector: JoinHandle<()>,
    acceptor: ConnectionAcceptor,
    serving: JoinHandle<anyhow::Result<()>>,
    cleanup: JoinHandle<()>,
    probe: Arc<DeliveryProbe>,
    usage: mpsc::UnboundedReceiver<WebSocketUsageEvent>,
    /// Updates handed to the projector so far, seeding included.
    published: AtomicU64,
    /// A flush marker is in the projector's channel or being applied, and has
    /// not been acknowledged yet.
    marker_pending: AtomicBool,
}

impl Harness {
    async fn start(delivery: &DeliveryProfile, run_timeout: Duration) -> Self {
        let mut index = ViewIndex::new();
        index.add_spec(token_account_view());
        let view_index = Arc::new(index);
        let entity_cache = EntityCache::with_config(EntityCacheConfig {
            max_entities_per_view: delivery.cache_entities_per_view,
            initial_snapshot_batch_size: delivery.snapshot_initial_batch,
            subsequent_snapshot_batch_size: delivery.snapshot_subsequent_batch,
            ..EntityCacheConfig::default()
        });
        let bus_manager = BusManager::with_capacity(delivery.list_bus_capacity);

        let (tx, rx) = mpsc::channel::<MutationBatch>(PROJECTOR_CHANNEL);
        let projector = tokio::spawn(
            Projector::new(
                view_index.clone(),
                bus_manager.clone(),
                entity_cache.clone(),
                rx,
                #[cfg(feature = "otel")]
                None,
            )
            .run(),
        );

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        // A metered server emits usage for every message; keep that cost in the
        // measured path and use the events to cross-check client counts.
        let (usage_tx, usage) = mpsc::unbounded_channel();
        let server = WebSocketServer::new(
            addr,
            bus_manager,
            entity_cache,
            view_index,
            #[cfg(feature = "otel")]
            None,
        )
        .with_delivery_config(WebSocketDeliveryConfig {
            list_bus_capacity: delivery.list_bus_capacity,
            collection_coalesce_ms: delivery.collection_coalesce_ms,
        })
        .with_rate_limit_config({
            let defaults = RateLimitConfig::default();
            RateLimitConfig {
                message_queue_size: delivery.message_queue_size,
                // SDKs keep themselves out of the idle-client sweep with
                // periodic pings; these clients only read. Widen the window
                // only for local runs longer than it, so a default run keeps
                // the stock value.
                client_timeout: defaults
                    .client_timeout
                    .max(run_timeout + Duration::from_secs(60)),
                ..defaults
            }
        })
        .with_usage_emitter(Arc::new(ChannelUsageEmitter::new(usage_tx)));
        let probe = Arc::new(DeliveryProbe::default());
        let (acceptor, cleanup) = server.into_acceptor();
        let acceptor = acceptor.with_delivery_probe(probe.clone());
        let serving = tokio::spawn(acceptor.clone().serve_listener(listener));

        Self {
            addr,
            tx,
            projector,
            acceptor,
            serving,
            cleanup,
            probe,
            usage,
            published: AtomicU64::new(0),
            marker_pending: AtomicBool::new(false),
        }
    }

    /// Publish `updates` in order, paced in 10 ms ticks when a rate is set,
    /// then wait until the projector has applied and published all of them.
    async fn publish(&self, updates: &[SourceUpdate], rate_per_sec: u64) {
        let per_tick = if rate_per_sec == 0 {
            updates.len().max(1)
        } else {
            ((rate_per_sec / 100) as usize).max(1)
        };
        let started = Instant::now();
        for (tick, chunk) in updates.chunks(per_tick).enumerate() {
            if rate_per_sec > 0 {
                tokio::time::sleep_until(started + PACING_TICK * tick as u32).await;
            }
            for update in chunk {
                self.tx
                    .send(update.batch())
                    .await
                    .expect("the projector outlives the publisher");
                self.published.fetch_add(1, Ordering::Relaxed);
            }
        }
        let (ack, applied) = oneshot::channel();
        self.tx
            .send(MutationBatch::flush_marker(ack))
            .await
            .expect("the projector outlives the publisher");
        self.marker_pending.store(true, Ordering::Relaxed);
        applied.await.expect("the projector acknowledges flushes");
        self.marker_pending.store(false, Ordering::Relaxed);
    }

    /// Wait until timed delivery has flushed `expected` source updates, the
    /// server's own statement that nothing more is pending.
    ///
    /// While no subscription has lagged or stopped, the count is certain to
    /// arrive, so this waits for it up to `deadline` however slow the host is.
    /// Lag recovery and stopped deliveries discard pending updates; once the
    /// server reports either, the count is unreachable and a short grace
    /// period stands in for it.
    async fn quiesce(
        &self,
        expected: u64,
        delivery: &DeliveryProfile,
        deadline: Instant,
    ) -> Quiescence {
        if delivery.collection_coalesce_ms.is_none() {
            // Immediate delivery sends each source frame as it is received,
            // so convergence on the last update already ends the traffic.
            return Quiescence::Immediate;
        }
        let mut give_up = deadline.max(Instant::now() + QUIESCENCE_GRACE);
        loop {
            if self.probe.coalesced_updates.load(Ordering::Relaxed) >= expected {
                return Quiescence::Flushed;
            }
            let unreachable = self.probe.lag_events.load(Ordering::Relaxed) > 0
                || !self
                    .probe
                    .delivery_stopped
                    .lock()
                    .expect("delivery probe lock poisoned")
                    .is_empty();
            if unreachable {
                give_up = give_up.min(Instant::now() + QUIESCENCE_GRACE);
            }
            if Instant::now() >= give_up {
                return Quiescence::Grace;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    /// Stop serving, then wait for every session, delivery task and usage
    /// event to finish. A task still holding the server after that is a leak.
    async fn stop(self) -> StopReport {
        let Self {
            tx,
            projector,
            acceptor,
            serving,
            cleanup,
            probe,
            mut usage,
            ..
        } = self;
        let mut problems = Vec::new();
        acceptor.shutdown();
        if timeout(STOP_TIMEOUT, acceptor.wait_for_sessions())
            .await
            .is_err()
        {
            problems.push("sessions were still running 30s after shutdown".to_string());
        }
        if acceptor.client_count() != 0 {
            problems.push(format!(
                "{} clients were still registered after their sessions ended",
                acceptor.client_count()
            ));
        }
        let serving_abort = serving.abort_handle();
        match timeout(STOP_TIMEOUT, serving).await {
            Ok(Ok(Ok(()))) => {}
            other => {
                serving_abort.abort();
                problems.push(format!("the listener did not stop cleanly: {other:?}"));
            }
        }
        cleanup.abort();
        drop(acceptor);
        drop(tx);
        let projector_abort = projector.abort_handle();
        if timeout(STOP_TIMEOUT, projector).await.is_err() {
            projector_abort.abort();
            problems.push("the projector did not stop after its sender closed".to_string());
        }

        let mut totals = UsageTotals::default();
        let drained = timeout(STOP_TIMEOUT, async {
            while let Some(event) = usage.recv().await {
                totals.record(event);
            }
        })
        .await;
        if drained.is_err() {
            problems.push(
                "usage events were still referenced 30s after shutdown: a delivery task outlived its session"
                    .to_string(),
            );
        }

        let load = |counter: &AtomicU64| counter.load(Ordering::Relaxed);
        let server = ServerCounters {
            messages_sent: load(&probe.messages_sent),
            resnapshots: load(&probe.resnapshots),
            lag_events: load(&probe.lag_events),
            dropped_updates: load(&probe.dropped_updates),
            coalesced_flushes: load(&probe.coalesced_flushes),
            delivery_stopped: probe
                .delivery_stopped
                .lock()
                .expect("delivery probe lock poisoned")
                .clone(),
        };
        StopReport {
            coalesced_source_updates: load(&probe.coalesced_updates),
            server,
            usage: totals,
            problems,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
enum Quiescence {
    /// The server flushed one timed delivery per source update per subscriber.
    Flushed,
    /// Uncoalesced delivery: nothing is held back once clients converge.
    Immediate,
    /// A subscription lagged or stopped, so the count was unreachable; the
    /// run waited a bounded grace period instead.
    Grace,
}

struct PublishReport {
    elapsed: Duration,
    updates: u64,
}

struct StopReport {
    coalesced_source_updates: u64,
    server: ServerCounters,
    usage: UsageTotals,
    problems: Vec<String>,
}

// ---------------------------------------------------------------------------
// Client side
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Encoding {
    Text,
    BinaryJson,
    BinaryGzip,
}

struct Decoded<'a> {
    encoding: Encoding,
    wire_len: usize,
    json: Cow<'a, [u8]>,
}

/// Decode a data message the way the SDKs do: text is JSON, binary is JSON
/// unless it starts with the gzip magic bytes.
fn decode_message(message: &Message) -> Result<Option<Decoded<'_>>, String> {
    Ok(Some(match message {
        Message::Text(text) => Decoded {
            encoding: Encoding::Text,
            wire_len: text.len(),
            json: Cow::Borrowed(text.as_bytes()),
        },
        Message::Binary(bytes) if is_gzip(bytes) => {
            let mut json = Vec::new();
            flate2::read::GzDecoder::new(bytes.as_ref())
                .read_to_end(&mut json)
                .map_err(|error| format!("a gzip frame did not decode: {error}"))?;
            Decoded {
                encoding: Encoding::BinaryGzip,
                wire_len: bytes.len(),
                json: Cow::Owned(json),
            }
        }
        Message::Binary(bytes) => Decoded {
            encoding: Encoding::BinaryJson,
            wire_len: bytes.len(),
            json: Cow::Borrowed(bytes.as_ref()),
        },
        _ => return Ok(None),
    }))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SnapshotPurpose {
    Initial,
    Recovery,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FrameKind {
    Control,
    Snapshot {
        rows: usize,
        completed: Option<SnapshotPurpose>,
    },
    Live(&'static str),
}

struct PendingSnapshot {
    id: String,
    authoritative: bool,
    rows: Vec<(String, Value)>,
}

/// Per-connection protocol state. A reconnect starts a fresh one while the
/// committed rows stay visible until the new authoritative snapshot lands.
#[derive(Default)]
struct ConnectionState {
    subscribed: bool,
    snapshot: Option<PendingSnapshot>,
    completed_snapshots: HashSet<String>,
}

/// A list subscriber's local store, tracking how many keys still differ from
/// a target state. Frames apply with the SDKs' snapshot, upsert, patch and
/// remove semantics, but without their stale-sequence guard: an out-of-order
/// frame lands here and shows up as a wrong or regressed state instead of
/// being silently dropped.
struct ClientView {
    rows: Rows,
    target: Arc<Rows>,
    mismatched: usize,
    connection: ConnectionState,
}

impl ClientView {
    fn new(target: Arc<Rows>) -> Self {
        let mut view = Self {
            rows: Rows::new(),
            target,
            mismatched: 0,
            connection: ConnectionState::default(),
        };
        view.recount();
        view
    }

    fn retarget(&mut self, target: Arc<Rows>) {
        self.target = target;
        self.recount();
    }

    fn converged(&self) -> bool {
        self.mismatched == 0
    }

    fn new_connection(&mut self) {
        self.connection = ConnectionState::default();
    }

    fn initial_snapshot_complete(&self) -> bool {
        !self.connection.completed_snapshots.is_empty()
    }

    fn recount(&mut self) {
        let different = self
            .target
            .iter()
            .filter(|(key, row)| self.rows.get(*key) != Some(*row))
            .count();
        let extra = self
            .rows
            .keys()
            .filter(|key| !self.target.contains_key(*key))
            .count();
        self.mismatched = different + extra;
    }

    fn change_row(&mut self, key: &str, change: impl FnOnce(&mut Rows)) {
        let before = self.rows.get(key) == self.target.get(key);
        change(&mut self.rows);
        let after = self.rows.get(key) == self.target.get(key);
        match (before, after) {
            (true, false) => self.mismatched += 1,
            (false, true) => self.mismatched -= 1,
            _ => {}
        }
    }

    fn apply(&mut self, mut frame: Value) -> Result<FrameKind, String> {
        if frame.get("type").and_then(Value::as_str) == Some("error") {
            return Err(format!("server error frame: {frame}"));
        }
        if frame.get("protocolVersion") != Some(&json!(2)) {
            return Err(format!("frame is not protocol v2: {frame}"));
        }
        if frame.get("subscriptionId").and_then(Value::as_str) != Some(SUBSCRIPTION_ID) {
            return Err(format!("frame for another subscription: {frame}"));
        }
        let op = match frame.get("op").and_then(Value::as_str) {
            Some("subscribed") => {
                if self.connection.subscribed {
                    return Err("duplicate subscribed acknowledgement".to_string());
                }
                if frame.get("mode").and_then(Value::as_str) != Some("list") {
                    return Err(format!("acknowledged with the wrong mode: {frame}"));
                }
                self.connection.subscribed = true;
                return Ok(FrameKind::Control);
            }
            Some("snapshot") => return self.apply_snapshot(frame),
            Some("upsert") => "upsert",
            Some("patch") => "patch",
            Some("remove") => "remove",
            Some("delete") => "delete",
            _ => return Err(format!("unexpected frame: {frame}")),
        };

        if !self.connection.subscribed || !self.initial_snapshot_complete() {
            return Err(format!("live {op} before the initial snapshot completed"));
        }
        if self.connection.snapshot.is_some() {
            return Err(format!("live {op} interleaved with an incomplete snapshot"));
        }
        check_list_entity(&frame)?;
        let key = frame
            .get("key")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("live {op} without a key: {frame}"))?
            .to_string();
        let data = frame.get_mut("data").map(Value::take).unwrap_or_default();
        match op {
            "upsert" => self.change_row(&key, |rows| {
                rows.insert(key.clone(), data);
            }),
            "patch" => {
                let append: Vec<String> = frame
                    .get("append")
                    .and_then(Value::as_array)
                    .map(|paths| {
                        paths
                            .iter()
                            .filter_map(|path| path.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default();
                self.change_row(&key, |rows| match rows.get_mut(&key) {
                    Some(row) => merge_patch(row, data, &append, ""),
                    None => {
                        rows.insert(key.clone(), data);
                    }
                });
            }
            _ => self.change_row(&key, |rows| {
                rows.remove(&key);
            }),
        }
        Ok(FrameKind::Live(op))
    }

    fn apply_snapshot(&mut self, mut frame: Value) -> Result<FrameKind, String> {
        if !self.connection.subscribed {
            return Err("snapshot before the subscribed acknowledgement".to_string());
        }
        check_list_entity(&frame)?;
        let id = frame
            .get("snapshotId")
            .and_then(Value::as_str)
            .ok_or("snapshot without snapshotId")?
            .to_string();
        let authoritative = frame
            .get("authoritative")
            .and_then(Value::as_bool)
            .ok_or("snapshot without authoritative")?;
        let complete = frame
            .get("complete")
            .and_then(Value::as_bool)
            .ok_or("snapshot without complete")?;
        if self.connection.completed_snapshots.contains(&id) {
            return Err(format!(
                "snapshot {id} continued after completing (duplicate terminal state)"
            ));
        }
        let pending = self
            .connection
            .snapshot
            .get_or_insert_with(|| PendingSnapshot {
                id: id.clone(),
                authoritative,
                rows: Vec::new(),
            });
        if pending.id != id {
            return Err(format!(
                "snapshot {id} interleaved with incomplete snapshot {}",
                pending.id
            ));
        }
        if pending.authoritative != authoritative {
            return Err(format!("snapshot {id} changed authority between batches"));
        }
        let Some(Value::Array(entries)) = frame.get_mut("data").map(Value::take) else {
            return Err(format!("snapshot {id} without a data array"));
        };
        let rows = entries.len();
        for mut entry in entries {
            let key = entry
                .get("key")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("snapshot {id} row without a key"))?
                .to_string();
            let data = entry.get_mut("data").map(Value::take).unwrap_or_default();
            pending.rows.push((key, data));
        }
        if !complete {
            return Ok(FrameKind::Snapshot {
                rows,
                completed: None,
            });
        }

        let pending = self.connection.snapshot.take().expect("pending snapshot");
        let snapshot_rows = pending.rows.len();
        if pending.authoritative {
            self.rows = pending.rows.into_iter().collect();
            self.recount();
            if self.rows.len() != snapshot_rows {
                return Err(format!("authoritative snapshot {id} repeated a key"));
            }
        } else {
            for (key, data) in pending.rows {
                self.change_row(&key, |rows| {
                    rows.insert(key.clone(), data);
                });
            }
        }
        self.connection.completed_snapshots.insert(pending.id);
        let purpose = if self.connection.completed_snapshots.len() == 1 {
            SnapshotPurpose::Initial
        } else {
            SnapshotPurpose::Recovery
        };
        Ok(FrameKind::Snapshot {
            rows,
            completed: Some(purpose),
        })
    }

    fn mismatch_sample(&self, limit: usize) -> Vec<String> {
        let seq = |row: &Value| row.get("_seq").cloned().unwrap_or(Value::Null);
        let mut sample: Vec<String> = self
            .target
            .iter()
            .filter_map(|(key, expected)| match self.rows.get(key) {
                None => Some(format!("{key}: missing")),
                Some(actual) if actual != expected => Some(format!(
                    "{key}: expected _seq {} got {}",
                    seq(expected),
                    seq(actual)
                )),
                Some(_) => None,
            })
            .chain(
                self.rows
                    .keys()
                    .filter(|key| !self.target.contains_key(*key))
                    .map(|key| format!("{key}: unexpected")),
            )
            .take(limit)
            .collect();
        sample.sort();
        sample
    }
}

fn check_list_entity(frame: &Value) -> Result<(), String> {
    if frame.get("entity").and_then(Value::as_str) != Some(VIEW)
        || frame.get("mode").and_then(Value::as_str) != Some("list")
    {
        return Err(format!("frame for the wrong view or mode: {frame}"));
    }
    Ok(())
}

/// The SDK wire-patch merge: objects merge recursively, arrays at an append
/// path extend, anything else replaces.
fn merge_patch(base: &mut Value, patch: Value, append: &[String], path: &str) {
    match (base, patch) {
        (Value::Object(base), Value::Object(patch)) => {
            for (field, value) in patch {
                let child = if path.is_empty() {
                    field.clone()
                } else {
                    format!("{path}.{field}")
                };
                match base.get_mut(&field) {
                    Some(existing) => merge_patch(existing, value, append, &child),
                    None => {
                        base.insert(field, value);
                    }
                }
            }
        }
        (Value::Array(base), Value::Array(items)) if append.iter().any(|p| p == path) => {
            base.extend(items);
        }
        (base, patch) => *base = patch,
    }
}

fn state_digest(rows: &Rows) -> String {
    let mut keys: Vec<&String> = rows.keys().collect();
    keys.sort_unstable();
    let mut hasher = Sha256::new();
    let mut buffer = String::new();
    for key in keys {
        buffer.clear();
        write_canonical(&rows[key], &mut buffer);
        hasher.update(key.as_bytes());
        hasher.update(b"\0");
        hasher.update(buffer.as_bytes());
        hasher.update(b"\n");
    }
    format!("sha256:{}", hex::encode(hasher.finalize()))
}

/// JSON with object keys sorted, so the digest does not depend on field order.
fn write_canonical(value: &Value, out: &mut String) {
    match value {
        Value::Object(fields) => {
            let mut fields: Vec<_> = fields.iter().collect();
            fields.sort_unstable_by(|left, right| left.0.cmp(right.0));
            out.push('{');
            for (index, (field, value)) in fields.into_iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String(field.clone()).to_string());
                out.push(':');
                write_canonical(value, out);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        scalar => out.push_str(&scalar.to_string()),
    }
}

#[derive(Debug, Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct WireCounters {
    /// Every text or binary WebSocket message received.
    websocket_messages: u64,
    /// Message payload bytes as received (gzip frames at compressed size),
    /// excluding WebSocket framing.
    wire_bytes: u64,
    /// JSON bytes after decompression.
    decoded_bytes: u64,
    live_messages: u64,
    live_wire_bytes: u64,
    snapshot_messages: u64,
    snapshot_wire_bytes: u64,
    snapshot_rows: u64,
    control_messages: u64,
    /// Messages received after the state already equalled the final model:
    /// the redundant tail a real client still has to read.
    trailing_messages: u64,
    frame_encodings: FrameEncodings,
}

#[derive(Debug, Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct FrameEncodings {
    text: u64,
    binary_json: u64,
    binary_gzip: u64,
}

#[derive(Debug, Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct OperationCounts {
    upsert: u64,
    patch: u64,
    remove: u64,
    delete: u64,
}

impl OperationCounts {
    fn record(&mut self, op: &str) {
        match op {
            "upsert" => self.upsert += 1,
            "patch" => self.patch += 1,
            "remove" => self.remove += 1,
            _ => self.delete += 1,
        }
    }

    fn total(&self) -> u64 {
        self.upsert + self.patch + self.remove + self.delete
    }

    fn add(&mut self, other: &Self) {
        self.upsert += other.upsert;
        self.patch += other.patch;
        self.remove += other.remove;
        self.delete += other.delete;
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DisconnectReport {
    at_ms: f64,
    reason: String,
    messages_before: u64,
}

enum Observed {
    Frame,
    Ignored,
    Closed(String),
}

#[derive(Default)]
struct ClientRecorder {
    wire: WireCounters,
    operations: OperationCounts,
    initial_snapshots: u64,
    resnapshots: u64,
    disconnects: Vec<DisconnectReport>,
    reconnects: u64,
    protocol_errors: Vec<String>,
    failures: Vec<String>,
    first_live_update: Option<Duration>,
    drain: Option<Duration>,
}

impl ClientRecorder {
    fn observe(
        &mut self,
        view: &mut ClientView,
        message: &Message,
        burst_start: Option<Instant>,
    ) -> Result<Observed, String> {
        if let Message::Close(frame) = message {
            return Ok(Observed::Closed(close_reason(frame.as_ref())));
        }
        let Some(decoded) = decode_message(message)? else {
            return Ok(Observed::Ignored);
        };
        let wire_len = decoded.wire_len as u64;
        self.wire.websocket_messages += 1;
        self.wire.wire_bytes += wire_len;
        self.wire.decoded_bytes += decoded.json.len() as u64;
        match decoded.encoding {
            Encoding::Text => self.wire.frame_encodings.text += 1,
            Encoding::BinaryJson => self.wire.frame_encodings.binary_json += 1,
            Encoding::BinaryGzip => self.wire.frame_encodings.binary_gzip += 1,
        }
        let frame: Value = serde_json::from_slice(&decoded.json)
            .map_err(|error| format!("a frame is not JSON: {error}"))?;

        match view.apply(frame)? {
            FrameKind::Control => self.wire.control_messages += 1,
            FrameKind::Snapshot { rows, completed } => {
                self.wire.snapshot_messages += 1;
                self.wire.snapshot_wire_bytes += wire_len;
                self.wire.snapshot_rows += rows as u64;
                match completed {
                    Some(SnapshotPurpose::Initial) => self.initial_snapshots += 1,
                    Some(SnapshotPurpose::Recovery) => self.resnapshots += 1,
                    None => {}
                }
            }
            FrameKind::Live(op) => {
                self.wire.live_messages += 1;
                self.wire.live_wire_bytes += wire_len;
                self.operations.record(op);
                if let (None, Some(start)) = (self.first_live_update, burst_start) {
                    self.first_live_update = Some(start.elapsed());
                }
            }
        }
        Ok(Observed::Frame)
    }

    fn finish(self, client: usize, view: &ClientView) -> ClientReport {
        let converged = self.drain.is_some();
        let outcome = if !converged {
            "failed"
        } else if self.reconnects > 0 {
            "reconnected"
        } else if !self.disconnects.is_empty() {
            "closed-after-converging"
        } else if self.resnapshots > 0 {
            "recovered"
        } else {
            "drained"
        };
        ClientReport {
            client,
            outcome,
            converged,
            delivered_operations: self.operations.total(),
            operations: self.operations,
            wire: self.wire,
            initial_snapshots: self.initial_snapshots,
            resnapshots: self.resnapshots,
            disconnects: self.disconnects.len() as u64,
            disconnect_details: self.disconnects,
            reconnects: self.reconnects,
            protocol_errors: self.protocol_errors,
            failures: self.failures,
            first_live_update_ms: self.first_live_update.map(ms),
            drain_ms: self.drain.map(ms),
            final_state_digest: state_digest(&view.rows),
            final_rows: view.rows.len(),
            mismatched_rows: if converged { 0 } else { view.mismatched },
            mismatch_sample: if converged {
                vec![]
            } else {
                view.mismatch_sample(5)
            },
        }
    }
}

fn close_reason(frame: Option<&CloseFrame>) -> String {
    match frame {
        Some(frame) => format!("close {} {}", u16::from(frame.code), frame.reason),
        None => "close without a status".to_string(),
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ClientReport {
    client: usize,
    /// `drained`, `recovered` (lag resnapshot), `reconnected` (explicit close
    /// then a fresh subscription, before or after converging),
    /// `closed-after-converging` (the server dropped it after it converged,
    /// in a scenario that forbids reconnecting; also a failure), or `failed`.
    outcome: &'static str,
    converged: bool,
    delivered_operations: u64,
    operations: OperationCounts,
    #[serde(flatten)]
    wire: WireCounters,
    initial_snapshots: u64,
    resnapshots: u64,
    disconnects: u64,
    disconnect_details: Vec<DisconnectReport>,
    reconnects: u64,
    protocol_errors: Vec<String>,
    failures: Vec<String>,
    first_live_update_ms: Option<f64>,
    drain_ms: Option<f64>,
    final_state_digest: String,
    final_rows: usize,
    mismatched_rows: usize,
    mismatch_sample: Vec<String>,
}

struct ClientContext {
    index: usize,
    addr: SocketAddr,
    seed_rows: Arc<Rows>,
    final_rows: Arc<Rows>,
    read_pause: Duration,
    on_disconnect: OnDisconnect,
    max_reconnects: u64,
    deadline: Instant,
}

fn subscribe_request() -> String {
    json!({
        "type": "subscribe",
        "protocolVersion": 2,
        "subscriptionId": SUBSCRIPTION_ID,
        "query": {"view": VIEW},
        "snapshot": {"enabled": true},
    })
    .to_string()
}

async fn connect_and_subscribe(addr: SocketAddr, deadline: Instant) -> Result<Socket, String> {
    timeout_at(deadline, async {
        let stream = TcpStream::connect(addr)
            .await
            .map_err(|error| format!("connect failed: {error}"))?;
        let (mut socket, _) = client_async(format!("ws://{addr}/"), stream)
            .await
            .map_err(|error| format!("handshake failed: {error}"))?;
        socket
            .send(Message::Text(subscribe_request().into()))
            .await
            .map_err(|error| format!("subscribe was not sent: {error}"))?;
        Ok(socket)
    })
    .await
    .map_err(|_| "connecting did not finish before the deadline".to_string())?
}

/// Subscribe and read until the initial snapshot is complete and equals the
/// seeded rows.
async fn attach(
    ctx: &ClientContext,
    view: &mut ClientView,
    recorder: &mut ClientRecorder,
) -> Result<Socket, String> {
    let mut socket = connect_and_subscribe(ctx.addr, ctx.deadline).await?;
    while !view.initial_snapshot_complete() {
        let next = timeout_at(ctx.deadline, socket.next())
            .await
            .map_err(|_| "timed out waiting for the initial snapshot".to_string())?;
        match next {
            Some(Ok(message)) => match recorder.observe(view, &message, None) {
                Ok(Observed::Closed(reason)) => {
                    return Err(format!("closed before the initial snapshot: {reason}"))
                }
                Ok(_) => {}
                Err(error) => {
                    recorder.protocol_errors.push(error);
                    return Err("protocol error before the initial snapshot".to_string());
                }
            },
            Some(Err(error)) => return Err(format!("socket failed during attach: {error}")),
            None => return Err("stream ended before the initial snapshot".to_string()),
        }
    }
    if !view.converged() {
        return Err(format!(
            "the initial snapshot differs from the seeded rows in {} rows",
            view.mismatched
        ));
    }
    Ok(socket)
}

/// Coordination between one client task and the scenario runner.
struct ClientSignals {
    /// Attached with a complete initial snapshot equal to the seeded rows.
    ready: oneshot::Sender<Result<(), String>>,
    /// Set to the burst start once the publisher begins.
    start: watch::Receiver<Option<Instant>>,
    /// Whether the client holds the exact final state on its current
    /// connection. It goes back to false while a client dropped after
    /// converging reconnects.
    converged: watch::Sender<bool>,
    /// Set just before the runner shuts the server down.
    stopping: watch::Receiver<bool>,
}

async fn run_client(ctx: ClientContext, signals: ClientSignals) -> ClientReport {
    let ClientSignals {
        ready,
        mut start,
        converged,
        stopping,
    } = signals;
    let mut view = ClientView::new(ctx.seed_rows.clone());
    let mut recorder = ClientRecorder::default();
    let mut socket = match attach(&ctx, &mut view, &mut recorder).await {
        Ok(socket) => {
            let _ = ready.send(Ok(()));
            socket
        }
        Err(error) => {
            let _ = ready.send(Err(error.clone()));
            recorder.failures.push(error);
            return recorder.finish(ctx.index, &view);
        }
    };

    let burst_start = match timeout_at(ctx.deadline, start.wait_for(Option::is_some)).await {
        Ok(Ok(started)) => (*started).expect("wait_for returned a started burst"),
        _ => {
            recorder
                .failures
                .push("the burst never started".to_string());
            return recorder.finish(ctx.index, &view);
        }
    };
    view.retarget(ctx.final_rows.clone());
    if !ctx.read_pause.is_zero() {
        // Load shaping only: the publisher keeps going while this reader
        // leaves its socket unread.
        tokio::time::sleep(ctx.read_pause).await;
    }

    let mut open = true;
    loop {
        // After a reconnect the rows are still the old connection's; only a
        // completed snapshot on this connection makes them this connection's.
        if view.converged() && view.initial_snapshot_complete() {
            recorder.drain = Some(burst_start.elapsed());
            converged.send_replace(true);
            match read_until_server_closes(
                &ctx,
                &mut socket,
                &mut view,
                &mut recorder,
                &stopping,
                burst_start,
            )
            .await
            {
                AfterConverging::Open => {}
                AfterConverging::Closed => open = false,
                AfterConverging::Dropped(reason) => {
                    // The same policy as a drop before converging: reconnect
                    // and hold the final state again from a fresh snapshot.
                    converged.send_replace(false);
                    open = false;
                    if recorder.reconnects >= ctx.max_reconnects {
                        recorder.failures.push(format!(
                            "disconnected after converging without an allowed reconnect: {reason}"
                        ));
                        break;
                    }
                    match connect_and_subscribe(ctx.addr, ctx.deadline).await {
                        Ok(next_socket) => {
                            socket = next_socket;
                            open = true;
                            recorder.reconnects += 1;
                            view.new_connection();
                            continue;
                        }
                        Err(error) => {
                            recorder.failures.push(format!("reconnect failed: {error}"));
                            break;
                        }
                    }
                }
            }
            break;
        }
        // `timeout_at` polls the socket first, so a steady stream of ready
        // frames would otherwise carry this reader past its deadline.
        let next = if Instant::now() < ctx.deadline {
            timeout_at(ctx.deadline, socket.next()).await.ok()
        } else {
            None
        };
        let Some(next) = next else {
            recorder.failures.push(format!(
                "timed out with {} rows still different from the final state",
                view.mismatched
            ));
            break;
        };
        let reason = match next {
            Some(Ok(message)) => match recorder.observe(&mut view, &message, Some(burst_start)) {
                Ok(Observed::Frame | Observed::Ignored) => continue,
                Ok(Observed::Closed(reason)) => reason,
                Err(error) => {
                    recorder.protocol_errors.push(error);
                    break;
                }
            },
            Some(Err(error)) => {
                recorder
                    .protocol_errors
                    .push(format!("the socket failed without a close frame: {error}"));
                open = false;
                break;
            }
            None => "stream ended without a close frame".to_string(),
        };

        open = false;
        if *stopping.borrow() {
            // The runner has ended the run; this is its shutdown, not a drop.
            recorder.failures.push(format!(
                "the run ended with {} rows still different from the final state",
                view.mismatched
            ));
            break;
        }
        recorder.disconnects.push(DisconnectReport {
            at_ms: ms(burst_start.elapsed()),
            reason: reason.clone(),
            messages_before: recorder.wire.websocket_messages,
        });
        if ctx.on_disconnect == OnDisconnect::Fail || recorder.reconnects >= ctx.max_reconnects {
            recorder.failures.push(format!(
                "disconnected without an allowed reconnect: {reason}"
            ));
            break;
        }
        match connect_and_subscribe(ctx.addr, ctx.deadline).await {
            Ok(next_socket) => {
                socket = next_socket;
                open = true;
                recorder.reconnects += 1;
                view.new_connection();
            }
            Err(error) => {
                recorder.failures.push(format!("reconnect failed: {error}"));
                break;
            }
        }
    }
    if open {
        let _ = timeout(Duration::from_secs(5), socket.close(None)).await;
    }
    recorder.finish(ctx.index, &view)
}

/// How reading after convergence ended.
enum AfterConverging {
    /// Stopped reading with the socket still open (a failure was recorded).
    Open,
    /// The socket closed: the runner's shutdown, or a drop the scenario
    /// forbids (recorded as a failure).
    Closed,
    /// The server dropped this client before the runner signalled
    /// `stopping`, in a scenario that allows reconnecting.
    Dropped(String),
}

/// Keep reading after converging until the server closes the stream. The
/// runner stops the server only once it has flushed every source update, so
/// this counts the redundant traffic a real client would still receive and
/// proves none of it moves the state away from the final one. A close before
/// the runner signals `stopping` is the server dropping this client, and is
/// recorded as a disconnect.
async fn read_until_server_closes(
    ctx: &ClientContext,
    socket: &mut Socket,
    view: &mut ClientView,
    recorder: &mut ClientRecorder,
    stopping: &watch::Receiver<bool>,
    burst_start: Instant,
) -> AfterConverging {
    let mut regressed = false;
    loop {
        let reason = match timeout_at(ctx.deadline + STOP_TIMEOUT, socket.next()).await {
            Err(_) => {
                recorder
                    .failures
                    .push("the server never closed the stream after the run".to_string());
                return AfterConverging::Open;
            }
            Ok(Some(Ok(message))) => match recorder.observe(view, &message, None) {
                Ok(Observed::Frame) => {
                    recorder.wire.trailing_messages += 1;
                    if !view.converged() && !regressed {
                        regressed = true;
                        recorder.protocol_errors.push(format!(
                            "a frame after convergence moved {} rows away from the final state",
                            view.mismatched
                        ));
                    }
                    continue;
                }
                Ok(Observed::Ignored) => continue,
                Ok(Observed::Closed(reason)) => reason,
                Err(error) => {
                    recorder.protocol_errors.push(error);
                    return AfterConverging::Open;
                }
            },
            Ok(Some(Err(error))) => {
                recorder
                    .protocol_errors
                    .push(format!("the socket failed while draining: {error}"));
                return AfterConverging::Closed;
            }
            Ok(None) => "stream ended without a close frame".to_string(),
        };
        if !*stopping.borrow() {
            recorder.disconnects.push(DisconnectReport {
                at_ms: ms(burst_start.elapsed()),
                reason: format!("after converging: {reason}"),
                messages_before: recorder.wire.websocket_messages,
            });
            if ctx.on_disconnect == OnDisconnect::Fail {
                recorder
                    .failures
                    .push(format!("disconnected after converging: {reason}"));
            } else {
                return AfterConverging::Dropped(reason);
            }
        }
        return AfterConverging::Closed;
    }
}

// ---------------------------------------------------------------------------
// Scenario runner and summary
// ---------------------------------------------------------------------------

struct RunEnvironment {
    server_threads: usize,
    client_threads: Option<usize>,
    clients: Handle,
    overrides: Vec<String>,
}

#[derive(Debug, Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct UsageTotals {
    update_messages: u64,
    update_bytes: u64,
    snapshot_messages: u64,
    snapshot_rows: u64,
    snapshot_bytes: u64,
    connections: u64,
    connections_closed: u64,
}

impl UsageTotals {
    fn record(&mut self, event: WebSocketUsageEvent) {
        match event {
            WebSocketUsageEvent::UpdateSent {
                messages, bytes, ..
            } => {
                self.update_messages += u64::from(messages);
                self.update_bytes += bytes;
            }
            WebSocketUsageEvent::SnapshotSent {
                rows,
                messages,
                bytes,
                ..
            } => {
                self.snapshot_messages += u64::from(messages);
                self.snapshot_rows += u64::from(rows);
                self.snapshot_bytes += bytes;
            }
            WebSocketUsageEvent::ConnectionEstablished { .. } => self.connections += 1,
            WebSocketUsageEvent::ConnectionClosed { .. } => self.connections_closed += 1,
            WebSocketUsageEvent::SubscriptionCreated { .. }
            | WebSocketUsageEvent::SubscriptionRemoved { .. } => {}
        }
    }
}

#[derive(Debug, Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ServerCounters {
    messages_sent: u64,
    resnapshots: u64,
    lag_events: u64,
    dropped_updates: u64,
    coalesced_flushes: u64,
    delivery_stopped: BTreeMap<&'static str, u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SummaryConfig {
    #[serde(flatten)]
    delivery: DeliveryProfile,
    key_pattern: KeyPattern,
    target_source_rate_per_sec: u64,
    read_pause_ms: u64,
    on_disconnect: OnDisconnect,
    server_threads: usize,
    /// `null` when clients share the server runtime.
    client_threads: Option<usize>,
    build: &'static str,
    overrides: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct FixtureSizes {
    seed_row_json_bytes: usize,
    source_patch_json_bytes: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PerSubscriberSourceUpdate {
    delivered_operations: f64,
    live_messages: f64,
    live_wire_bytes: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PerDeliveredOperation {
    live_messages: f64,
    live_wire_bytes: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Summary {
    scenario: &'static str,
    summary_version: u32,
    config: SummaryConfig,
    fixture: FixtureSizes,
    seed_rows: usize,
    subscribers: usize,
    /// Burst mutations published through the projector (seeding excluded).
    source_updates: usize,
    /// Distinct keys the burst touched.
    unique_keys: usize,
    /// Live entity operations received, summed over subscribers. One entity
    /// frame each until list batching; one batch item each after it.
    delivered_operations: u64,
    operations: OperationCounts,
    /// `arete.ws.collection.coalesced_updates`: source updates consumed by
    /// timed flushes, summed over subscriptions.
    coalesced_source_updates: u64,
    #[serde(flatten)]
    wire: WireCounters,
    /// Lag recovery snapshots the subscribers completed.
    resnapshots: u64,
    protocol_errors: u64,
    disconnects: u64,
    reconnects: u64,
    outcomes: BTreeMap<&'static str, usize>,
    server: ServerCounters,
    usage: UsageTotals,
    /// Burst updates the projector applied. All of them when this equals
    /// `sourceUpdates`, since the burst ends with an acknowledged flush marker.
    /// When the deadline cuts the burst short, updates still queued for the
    /// projector are not counted; the one it may be applying at that moment
    /// is.
    published_updates: u64,
    publish_ms: f64,
    achieved_source_rate_per_sec: f64,
    /// How the run decided the server was done before closing sessions.
    quiescence: Quiescence,
    /// Slowest subscriber, from the first burst publish.
    first_live_update_ms: Option<f64>,
    /// Slowest subscriber to reach the exact final state, from the first
    /// burst publish.
    drain_ms: Option<f64>,
    /// Slowest subscriber to converge after the projector applied the last
    /// burst update.
    drain_after_publish_ms: Option<f64>,
    per_subscriber_source_update: PerSubscriberSourceUpdate,
    per_delivered_operation: PerDeliveredOperation,
    expected_state_digest: String,
    /// Shared by every subscriber when they all converged; otherwise null.
    final_state_digest: Option<String>,
    converged: bool,
    problems: Vec<String>,
    clients: Vec<ClientReport>,
}

/// Milliseconds to 0.1 ms.
fn ms(duration: Duration) -> f64 {
    tenth(duration.as_secs_f64() * 1_000.0)
}

fn tenth(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

fn ratio(numerator: u64, denominator: u64) -> f64 {
    if denominator == 0 {
        return 0.0;
    }
    (numerator as f64 / denominator as f64 * 10_000.0).round() / 10_000.0
}

async fn run_scenario(scenario: Scenario, environment: &RunEnvironment) -> (Summary, Vec<String>) {
    let workload = Workload::generate(&scenario);
    let deadline = Instant::now() + Duration::from_secs(scenario.timeout_secs);
    let mut problems = Vec::new();
    let harness = Harness::start(
        &scenario.delivery,
        Duration::from_secs(scenario.timeout_secs),
    )
    .await;

    // Seed before anyone subscribes: the initial snapshot is the
    // "thousands of rows" half of the reported shape.
    if timeout_at(deadline, harness.publish(&workload.seed, 0))
        .await
        .is_err()
    {
        problems.push("seeding did not finish before the deadline".to_string());
    }

    let (start_tx, start_rx) = watch::channel(None);
    let (stopping_tx, stopping_rx) = watch::channel(false);
    let mut clients = Vec::with_capacity(scenario.subscribers);
    let mut ready = Vec::with_capacity(scenario.subscribers);
    let mut converged = Vec::with_capacity(scenario.subscribers);
    for index in 0..scenario.subscribers {
        let (ready_tx, ready_rx) = oneshot::channel();
        let (converged_tx, converged_rx) = watch::channel(false);
        let ctx = ClientContext {
            index,
            addr: harness.addr,
            seed_rows: workload.seed_rows.clone(),
            final_rows: workload.final_rows.clone(),
            read_pause: Duration::from_millis(scenario.read_pause_ms),
            on_disconnect: scenario.on_disconnect,
            max_reconnects: scenario.max_reconnects,
            deadline,
        };
        let signals = ClientSignals {
            ready: ready_tx,
            start: start_rx.clone(),
            converged: converged_tx,
            stopping: stopping_rx.clone(),
        };
        clients.push(environment.clients.spawn(run_client(ctx, signals)));
        ready.push(ready_rx);
        converged.push(converged_rx);
    }
    for (index, ready) in ready.into_iter().enumerate() {
        match timeout_at(deadline, ready).await {
            Ok(Ok(Ok(()))) => {}
            Ok(Ok(Err(error))) => problems.push(format!("client {index} did not attach: {error}")),
            Ok(Err(_)) => problems.push(format!("client {index} stopped before attaching")),
            Err(_) => problems.push(format!("client {index} did not attach before the deadline")),
        }
    }

    let seeded = harness.published.load(Ordering::Relaxed);
    let burst_start = Instant::now();
    start_tx.send_replace(Some(burst_start));
    let published = timeout_at(
        deadline,
        harness.publish(&workload.burst, scenario.source_rate_per_sec),
    )
    .await;
    // Sent is not applied: a deadline can cut the burst short with batches
    // still waiting in the projector's channel. The flush marker is sent
    // last, so while it is outstanding and anything is queued, it is one of
    // them and not a source update.
    let in_channel = (harness.tx.max_capacity() - harness.tx.capacity()) as u64;
    let marker_queued = harness.marker_pending.load(Ordering::Relaxed) && in_channel > 0;
    let queued = in_channel - u64::from(marker_queued);
    let publish = PublishReport {
        elapsed: burst_start.elapsed(),
        updates: harness.published.load(Ordering::Relaxed) - seeded - queued,
    };
    if published.is_err() {
        problems.push(format!(
            "only {} of {} burst updates were applied before the deadline",
            publish.updates, scenario.source_updates
        ));
    }

    // Stop the server only once every subscriber holds the final state and the
    // server has flushed everything it received, so clients read the whole
    // tail and then an explicit close.
    for converged in &mut converged {
        let _ = timeout_at(deadline, converged.wait_for(|held| *held)).await;
    }
    let quiescence = harness
        .quiesce(
            (scenario.source_updates * scenario.subscribers) as u64,
            &scenario.delivery,
            deadline,
        )
        .await;
    // A client the server dropped after converging reconnects and converges
    // again, but its flag reads true until it has read the close. The server
    // removes a dropped client at once, though, so also wait until it counts
    // every subscriber as connected: then no drop is still in flight.
    let _ = timeout_at(deadline, async {
        while !(converged.iter().all(|held| *held.borrow())
            && harness.acceptor.client_count() >= scenario.subscribers)
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    // Any close a client sees before this is the server dropping it, not the
    // end of the run.
    stopping_tx.send_replace(true);
    harness.acceptor.shutdown();

    let mut reports = Vec::with_capacity(clients.len());
    for (index, client) in clients.into_iter().enumerate() {
        let abort = client.abort_handle();
        match timeout_at(deadline + STOP_TIMEOUT + Duration::from_secs(5), client).await {
            Ok(Ok(report)) => reports.push(report),
            Ok(Err(error)) => problems.push(format!("client {index} panicked: {error}")),
            Err(_) => {
                abort.abort();
                problems.push(format!("client {index} ignored its deadline"));
            }
        }
    }
    let stop = harness.stop().await;
    problems.extend(stop.problems.iter().cloned());

    let summary = summarize(
        &scenario,
        &workload,
        environment,
        stop,
        reports,
        publish,
        quiescence,
    );
    for client in &summary.clients {
        for error in &client.protocol_errors {
            problems.push(format!("client {} protocol error: {error}", client.client));
        }
        for failure in &client.failures {
            problems.push(format!("client {}: {failure}", client.client));
        }
    }
    problems.extend(check_invariants(&scenario, &summary));
    let summary = Summary {
        problems: problems.clone(),
        ..summary
    };
    (summary, problems)
}

fn summarize(
    scenario: &Scenario,
    workload: &Workload,
    environment: &RunEnvironment,
    stop: StopReport,
    clients: Vec<ClientReport>,
    publish: PublishReport,
    quiescence: Quiescence,
) -> Summary {
    let publish_elapsed = publish.elapsed;
    let mut wire = WireCounters::default();
    let mut operations = OperationCounts::default();
    let mut outcomes = BTreeMap::new();
    for client in &clients {
        let counters = &client.wire;
        wire.websocket_messages += counters.websocket_messages;
        wire.wire_bytes += counters.wire_bytes;
        wire.decoded_bytes += counters.decoded_bytes;
        wire.live_messages += counters.live_messages;
        wire.live_wire_bytes += counters.live_wire_bytes;
        wire.snapshot_messages += counters.snapshot_messages;
        wire.snapshot_wire_bytes += counters.snapshot_wire_bytes;
        wire.snapshot_rows += counters.snapshot_rows;
        wire.control_messages += counters.control_messages;
        wire.trailing_messages += counters.trailing_messages;
        wire.frame_encodings.text += counters.frame_encodings.text;
        wire.frame_encodings.binary_json += counters.frame_encodings.binary_json;
        wire.frame_encodings.binary_gzip += counters.frame_encodings.binary_gzip;
        operations.add(&client.operations);
        *outcomes.entry(client.outcome).or_default() += 1;
    }
    let delivered_operations = operations.total();
    let slowest =
        |pick: fn(&ClientReport) -> Option<f64>| clients.iter().filter_map(pick).reduce(f64::max);
    let all_converged =
        clients.len() == scenario.subscribers && clients.iter().all(|client| client.converged);
    let expected_state_digest = state_digest(&workload.final_rows);
    let final_state_digest = clients
        .first()
        .map(|client| client.final_state_digest.clone())
        .filter(|digest| {
            all_converged
                && clients
                    .iter()
                    .all(|client| &client.final_state_digest == digest)
        });
    let drain_ms = if all_converged {
        slowest(|client| client.drain_ms)
    } else {
        None
    };
    let subscriber_updates = (scenario.source_updates * scenario.subscribers) as u64;

    Summary {
        scenario: scenario.name,
        summary_version: SUMMARY_VERSION,
        config: SummaryConfig {
            delivery: scenario.delivery.clone(),
            key_pattern: scenario.pattern,
            target_source_rate_per_sec: scenario.source_rate_per_sec,
            read_pause_ms: scenario.read_pause_ms,
            on_disconnect: scenario.on_disconnect,
            server_threads: environment.server_threads,
            client_threads: environment.client_threads,
            build: if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            },
            overrides: environment.overrides.clone(),
        },
        fixture: workload.fixture_sizes(),
        seed_rows: workload.seed_rows.len(),
        subscribers: scenario.subscribers,
        source_updates: scenario.source_updates,
        unique_keys: workload.unique_keys,
        delivered_operations,
        operations,
        coalesced_source_updates: stop.coalesced_source_updates,
        resnapshots: clients.iter().map(|client| client.resnapshots).sum(),
        protocol_errors: clients
            .iter()
            .map(|client| client.protocol_errors.len() as u64)
            .sum(),
        disconnects: clients.iter().map(|client| client.disconnects).sum(),
        reconnects: clients.iter().map(|client| client.reconnects).sum(),
        outcomes,
        server: stop.server,
        usage: stop.usage,
        published_updates: publish.updates,
        publish_ms: ms(publish_elapsed),
        achieved_source_rate_per_sec: (publish.updates as f64
            / publish_elapsed.as_secs_f64().max(f64::EPSILON))
        .round(),
        quiescence,
        first_live_update_ms: slowest(|client| client.first_live_update_ms),
        drain_ms,
        drain_after_publish_ms: drain_ms.map(|drain| tenth((drain - ms(publish_elapsed)).max(0.0))),
        per_subscriber_source_update: PerSubscriberSourceUpdate {
            delivered_operations: ratio(delivered_operations, subscriber_updates),
            live_messages: ratio(wire.live_messages, subscriber_updates),
            live_wire_bytes: ratio(wire.live_wire_bytes, subscriber_updates),
        },
        per_delivered_operation: PerDeliveredOperation {
            live_messages: ratio(wire.live_messages, delivered_operations),
            live_wire_bytes: ratio(wire.live_wire_bytes, delivered_operations),
        },
        wire,
        expected_state_digest,
        final_state_digest,
        converged: all_converged,
        problems: Vec::new(),
        clients,
    }
}

/// Correctness and bounded-count invariants. Nothing here depends on how fast
/// the machine is.
fn check_invariants(scenario: &Scenario, summary: &Summary) -> Vec<String> {
    let mut problems = Vec::new();
    let mut expect = |ok: bool, problem: String| {
        if !ok {
            problems.push(problem);
        }
    };
    let source = summary.source_updates as u64;
    let subscribers = summary.subscribers as u64;

    for client in &summary.clients {
        let id = client.client;
        expect(
            !client.converged || client.final_state_digest == summary.expected_state_digest,
            format!("client {id} converged to the wrong digest"),
        );
        expect(
            !client.converged || client.initial_snapshots == client.reconnects + 1,
            format!(
                "client {id} completed {} initial snapshots over {} connections",
                client.initial_snapshots,
                client.reconnects + 1
            ),
        );
        if client.outcome == "drained" {
            // Every touched key changed, so each needs at least one live
            // operation. A flush sends a key for a pending update or for a
            // cache value that is ahead of the subscription's bus position,
            // and each source update can cause at most one of each.
            expect(
                client.delivered_operations >= summary.unique_keys as u64
                    && client.delivered_operations <= 2 * source,
                format!(
                    "client {id} received {} operations for {} keys and {source} source updates",
                    client.delivered_operations, summary.unique_keys
                ),
            );
            expect(
                client.operations.remove + client.operations.delete == 0,
                format!("client {id} saw membership exits the workload never causes"),
            );
        }
    }
    expect(
        summary.clients.len() == summary.subscribers && summary.converged,
        format!(
            "{} of {} subscribers reached the exact final state",
            summary
                .clients
                .iter()
                .filter(|client| client.converged)
                .count(),
            summary.subscribers
        ),
    );
    expect(
        summary.final_state_digest.as_deref() == Some(summary.expected_state_digest.as_str()),
        "subscribers do not share the expected final digest".to_string(),
    );
    expect(
        scenario.on_disconnect != OnDisconnect::Fail || summary.disconnects == 0,
        format!("{} unexpected disconnects", summary.disconnects),
    );
    expect(
        summary.coalesced_source_updates <= source * subscribers,
        format!(
            "{} source updates coalesced from {} published per subscriber",
            summary.coalesced_source_updates, source
        ),
    );

    if scenario.exact_accounting {
        let usage = &summary.usage;
        let wire = &summary.wire;
        expect(
            summary.outcomes.get("drained") == Some(&summary.subscribers),
            format!("expected every subscriber to drain: {:?}", summary.outcomes),
        );
        expect(
            summary.quiescence != Quiescence::Grace,
            "the server never reported flushing every source update".to_string(),
        );
        expect(
            summary.server.lag_events == 0
                && summary.server.resnapshots == 0
                && summary.server.delivery_stopped.is_empty(),
            format!("delivery degraded: {:?}", summary.server),
        );
        if scenario.delivery.collection_coalesce_ms.is_some() {
            expect(
                summary.coalesced_source_updates == source * subscribers,
                format!(
                    "every source update should pass one flush per subscriber: {} != {}",
                    summary.coalesced_source_updates,
                    source * subscribers
                ),
            );
        }
        expect(
            summary.server.messages_sent == wire.websocket_messages,
            format!(
                "server sent {} messages, clients received {}",
                summary.server.messages_sent, wire.websocket_messages
            ),
        );
        expect(
            usage.update_messages + usage.snapshot_messages == wire.websocket_messages
                && usage.update_bytes + usage.snapshot_bytes == wire.wire_bytes,
            format!(
                "usage reported {} messages / {} bytes, clients received {} / {}",
                usage.update_messages + usage.snapshot_messages,
                usage.update_bytes + usage.snapshot_bytes,
                wire.websocket_messages,
                wire.wire_bytes
            ),
        );
        expect(
            usage.snapshot_rows == wire.snapshot_rows
                && wire.snapshot_rows == (summary.seed_rows as u64) * subscribers,
            format!(
                "snapshot rows: usage {}, received {}, seeded {} per subscriber",
                usage.snapshot_rows, wire.snapshot_rows, summary.seed_rows
            ),
        );
        expect(
            wire.snapshot_messages
                == subscribers
                    * scenario
                        .delivery
                        .expected_snapshot_frames(summary.seed_rows),
            format!(
                "{} snapshot frames for {} rows per subscriber",
                wire.snapshot_messages, summary.seed_rows
            ),
        );
    }
    problems
}

fn print_summary(summary: &Summary) {
    println!(
        "{}",
        serde_json::to_string(summary).expect("the summary serializes")
    );
}

/// The always-run gate: a small coalesced-delivery workload over real
/// sockets, with exact state and exact server/usage/client accounting.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn smoke() {
    let environment = RunEnvironment {
        server_threads: 4,
        client_threads: None,
        clients: Handle::current(),
        overrides: Vec::new(),
    };
    let (summary, problems) = run_scenario(smoke_scenario(), &environment).await;
    print_summary(&summary);
    assert!(
        problems.is_empty(),
        "smoke delivery run failed:\n{}",
        problems.join("\n")
    );
}

/// The four capacity profiles, one JSON summary each. Clients run on their
/// own runtime so decoding does not occupy the server's worker threads.
#[test]
#[ignore = "capacity profile; run explicitly (docs/internal/websocket-delivery-load-baseline.md)"]
fn profiles() {
    let sizes = ProfileSizes::from_env();
    let mut failures = Vec::new();
    for scenario in profile_scenarios(&sizes) {
        let server = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(sizes.server_threads)
            .thread_name("load-server")
            .enable_all()
            .build()
            .expect("the server runtime starts");
        let clients = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(sizes.client_threads)
            .thread_name("load-client")
            .enable_all()
            .build()
            .expect("the client runtime starts");
        let environment = RunEnvironment {
            server_threads: sizes.server_threads,
            client_threads: Some(sizes.client_threads),
            clients: clients.handle().clone(),
            overrides: sizes.overrides.clone(),
        };
        let name = scenario.name;
        let (summary, problems) = server.block_on(run_scenario(scenario, &environment));
        print_summary(&summary);
        failures.extend(
            problems
                .into_iter()
                .map(|problem| format!("{name}: {problem}")),
        );
        clients.shutdown_timeout(Duration::from_secs(5));
        server.shutdown_timeout(Duration::from_secs(5));
    }
    assert!(
        failures.is_empty(),
        "delivery load profiles failed:\n{}",
        failures.join("\n")
    );
}

// ---------------------------------------------------------------------------
// Harness self-tests
// ---------------------------------------------------------------------------

fn subscribed_frame() -> Value {
    json!({
        "protocolVersion": 2,
        "subscriptionId": SUBSCRIPTION_ID,
        "op": "subscribed",
        "mode": "list",
        "query": {"view": VIEW},
    })
}

fn snapshot_frame(id: &str, rows: Value, complete: bool) -> Value {
    json!({
        "protocolVersion": 2,
        "subscriptionId": SUBSCRIPTION_ID,
        "snapshotId": id,
        "authoritative": true,
        "mode": "list",
        "entity": VIEW,
        "op": "snapshot",
        "data": rows,
        "complete": complete,
    })
}

fn live_frame(op: &str, key: &str, data: Value) -> Value {
    json!({
        "protocolVersion": 2,
        "subscriptionId": SUBSCRIPTION_ID,
        "mode": "list",
        "entity": VIEW,
        "op": op,
        "key": key,
        "data": data,
    })
}

#[test]
fn frames_decode_from_text_binary_and_gzip() {
    let frame = json!({"op": "upsert", "rows": vec![fixture_address("row", 0); 64]});
    let text = serde_json::to_string(&frame).unwrap();
    let compressed = crate::compression::maybe_compress(text.as_bytes());
    assert!(
        compressed.is_compressed(),
        "the fixture crosses the gzip threshold"
    );

    for (message, encoding) in [
        (Message::Text(text.clone().into()), Encoding::Text),
        (
            Message::Binary(text.clone().into_bytes().into()),
            Encoding::BinaryJson,
        ),
        (
            Message::Binary(compressed.into_bytes()),
            Encoding::BinaryGzip,
        ),
    ] {
        let decoded = decode_message(&message).unwrap().unwrap();
        assert_eq!(decoded.encoding, encoding);
        assert_eq!(
            serde_json::from_slice::<Value>(&decoded.json).unwrap(),
            frame
        );
        assert_eq!(decoded.wire_len, message.len());
    }
    assert!(decode_message(&Message::Ping(Vec::new().into()))
        .unwrap()
        .is_none());
}

#[test]
fn client_view_applies_snapshots_and_live_operations() {
    let target: Rows = [
        (
            "a".to_string(),
            json!({"state": {"amount": "5", "mint": "m"}, "log": [1, 2]}),
        ),
        ("c".to_string(), json!({"state": {"amount": "1"}})),
    ]
    .into_iter()
    .collect();
    let mut view = ClientView::new(Arc::new(target));
    assert_eq!(view.mismatched, 2);

    view.apply(subscribed_frame()).unwrap();
    let first = view
        .apply(snapshot_frame(
            "s1",
            json!([{"key": "a", "data": {"state": {"amount": "4", "mint": "m"}, "log": [1]}}]),
            false,
        ))
        .unwrap();
    assert_eq!(
        first,
        FrameKind::Snapshot {
            rows: 1,
            completed: None
        }
    );
    assert!(
        view.rows.is_empty(),
        "rows apply only when the snapshot completes"
    );
    view.apply(snapshot_frame(
        "s1",
        json!([{"key": "b", "data": {"state": {"amount": "9"}}}]),
        true,
    ))
    .unwrap();
    assert_eq!(view.rows.len(), 2);
    assert_eq!(view.mismatched, 3, "a differs, b is extra, c is missing");

    let mut patch = live_frame("patch", "a", json!({"state": {"amount": "5"}, "log": [2]}));
    patch["append"] = json!(["log"]);
    assert_eq!(view.apply(patch).unwrap(), FrameKind::Live("patch"));
    view.apply(live_frame("remove", "b", Value::Null)).unwrap();
    assert!(!view.converged());
    view.apply(live_frame("upsert", "c", json!({"state": {"amount": "1"}})))
        .unwrap();
    assert!(view.converged(), "nested merge, append, remove and upsert");
    assert_eq!(view.mismatched, 0);
    assert_eq!(state_digest(&view.rows), state_digest(&view.target));
}

#[test]
fn client_view_rejects_duplicate_terminal_state_and_interleaving() {
    let mut view = ClientView::new(Arc::new(Rows::new()));
    assert!(view.apply(live_frame("upsert", "a", json!({}))).is_err());
    view.apply(subscribed_frame()).unwrap();
    assert!(view.apply(subscribed_frame()).is_err());
    view.apply(snapshot_frame("s1", json!([]), true)).unwrap();
    assert!(
        view.apply(snapshot_frame("s1", json!([]), true)).is_err(),
        "a completed snapshot cannot complete again"
    );

    view.apply(snapshot_frame("s2", json!([]), false)).unwrap();
    assert!(view.apply(live_frame("upsert", "a", json!({}))).is_err());
    assert!(view.apply(snapshot_frame("s3", json!([]), true)).is_err());

    let mut other = live_frame("upsert", "a", json!({}));
    other["subscriptionId"] = json!("other");
    assert!(view.apply(other).is_err());
    assert!(view
        .apply(json!({"type": "error", "protocolVersion": 2, "subscriptionId": SUBSCRIPTION_ID, "code": "x"}))
        .is_err());
}

#[test]
fn workloads_are_deterministic_and_shaped() {
    let hot = smoke_scenario();
    let first = Workload::generate(&hot);
    let second = Workload::generate(&hot);
    assert_eq!(
        state_digest(&first.final_rows),
        state_digest(&second.final_rows)
    );
    assert_ne!(
        state_digest(&first.seed_rows),
        state_digest(&first.final_rows)
    );
    assert_eq!(first.seed_rows.len(), hot.keys);
    assert!(
        first.unique_keys < hot.source_updates / 4,
        "the hot pattern repeats keys"
    );
    let hot_set = first
        .burst
        .iter()
        .filter(|update| {
            (0..hot.keys / 10).any(|index| update.key == fixture_address("token-account", index))
        })
        .count();
    assert!(
        hot_set * 10 >= hot.source_updates * 7,
        "most updates are hot"
    );

    let unique = Scenario {
        keys: 300,
        source_updates: 300,
        pattern: KeyPattern::Unique,
        ..smoke_scenario()
    };
    assert_eq!(Workload::generate(&unique).unique_keys, 300);

    let row = &first.final_rows[&first.burst[0].key];
    assert!(row["state"]["amount"].is_string(), "wide ints are strings");
    assert!(row["_seq"].is_string());
}
