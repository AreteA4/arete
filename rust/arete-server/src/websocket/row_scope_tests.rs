//! Row scoping over a real socket: a session scoped to one scope never
//! receives another scope's row in a snapshot, patch, delta, recovery
//! snapshot, replay or sorted view page, and scoped views fill their pages.

use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use arete_auth::{KeyClass, SessionClaims, SigningKey, TokenSigner, TokenVerifier};
use arete_interpreter::Mutation;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{client_async, WebSocketStream};

use crate::bus::BusManager;
use crate::cache::{EntityCache, EntityCacheConfig};
use crate::journal::{EventJournal, JournalConfig};
use crate::materialized_view::{SortConfig, SortOrder, ViewPipeline};
use crate::projector::Projector;
use crate::row_scope::ScopeField;
use crate::view::{Delivery, Filters, Projection, ViewIndex, ViewSpec};
use crate::websocket::auth::SignedSessionAuthPlugin;
use crate::websocket::frame::Mode;
use crate::websocket::server::WebSocketServer;
use crate::{MutationBatch, SlotContext};

const PAGE: usize = 20;

fn spec(id: &str, export: &str, mode: Mode) -> ViewSpec {
    ViewSpec {
        id: id.to_string(),
        export: export.to_string(),
        mode,
        wire_format: Default::default(),
        projection: Projection::all(),
        filters: Filters::all(),
        delivery: Delivery::default(),
        pipeline: None,
        source_view: None,
    }
}

/// `Thing` and `Trade` are scoped by their `scope` field; `Other` is not.
fn scoped_index() -> ViewIndex {
    let mut index = ViewIndex::new();
    index.add_spec(spec("Thing/list", "Thing", Mode::List));
    index.add_spec(spec("Thing/state", "Thing", Mode::State));
    index.add_spec(ViewSpec {
        pipeline: Some(ViewPipeline {
            filter: None,
            sort: Some(SortConfig {
                field_path: vec!["amount".into()],
                order: SortOrder::Desc,
            }),
            limit: Some(PAGE),
        }),
        source_view: Some("Thing/list".into()),
        ..spec("Thing/top", "Thing", Mode::List)
    });
    index.add_spec(spec("Other/list", "Other", Mode::List));
    index.add_spec(spec("Trade/append", "Trade", Mode::Append));
    index.set_entity_scope("Thing", ScopeField::new("scope"));
    index.set_entity_scope("Trade", ScopeField::new("scope"));
    index
}

struct Harness {
    addr: SocketAddr,
    tx: mpsc::Sender<MutationBatch>,
    signer: TokenSigner,
    journal: Option<Arc<EventJournal>>,
    slot: std::sync::atomic::AtomicU64,
}

#[derive(Default)]
struct Options {
    max_rows: Option<usize>,
    bus_capacity: Option<usize>,
    journal: bool,
}

impl Harness {
    async fn start(options: Options) -> Self {
        Self::start_with_index(options, scoped_index()).await
    }

    async fn start_with_index(options: Options, index: ViewIndex) -> Self {
        let view_index = Arc::new(index);
        let entity_cache = match options.max_rows {
            Some(max_entities_per_view) => EntityCache::with_config(EntityCacheConfig {
                max_entities_per_view,
                ..EntityCacheConfig::default()
            }),
            None => EntityCache::new(),
        };
        let bus_manager = match options.bus_capacity {
            Some(capacity) => BusManager::with_capacity(capacity),
            None => BusManager::new(),
        };
        let journal = options.journal.then(|| {
            Arc::new(EventJournal::new(JournalConfig {
                enabled: true,
                max_bytes_per_view: u64::MAX,
                max_records_per_view: 10_000,
                max_age: Duration::from_secs(3_600),
            }))
        });
        let (tx, rx) = mpsc::channel::<MutationBatch>(256);
        let mut projector = Projector::new(
            view_index.clone(),
            bus_manager.clone(),
            entity_cache.clone(),
            rx,
            #[cfg(feature = "otel")]
            None,
        );
        if let Some(journal) = &journal {
            projector = projector.with_journal(journal.clone());
        }
        tokio::spawn(tokio::task::unconstrained(projector.run()));

        let signing_key = SigningKey::generate();
        let verifier =
            TokenVerifier::new(signing_key.verifying_key(), "test-issuer", "test-audience");
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut server = WebSocketServer::new(
            addr,
            bus_manager,
            entity_cache,
            view_index,
            #[cfg(feature = "otel")]
            None,
        )
        .with_auth_plugin(Arc::new(SignedSessionAuthPlugin::new(verifier)));
        if let Some(journal) = &journal {
            server = server.with_journal(journal.clone());
        }
        let (acceptor, _cleanup) = server.into_acceptor();
        tokio::spawn(async move { acceptor.serve_listener(listener).await });
        Self {
            addr,
            tx,
            signer: TokenSigner::new(signing_key, "test-issuer"),
            journal,
            slot: std::sync::atomic::AtomicU64::new(100),
        }
    }

    /// A session token; `None` carries no `row_scopes` claim at all.
    fn token(&self, row_scopes: Option<&[&str]>) -> String {
        let mut claims = SessionClaims::builder("test-issuer", "test-subject", "test-audience")
            .with_scope("read")
            .with_key_class(KeyClass::Secret)
            .with_ttl(3_600);
        if let Some(row_scopes) = row_scopes {
            claims = claims.with_row_scopes(row_scopes.iter().copied());
        }
        self.signer.sign(claims.build()).unwrap()
    }

    async fn connect(&self, row_scopes: Option<&[&str]>) -> WebSocketStream<TcpStream> {
        let stream = TcpStream::connect(self.addr).await.unwrap();
        let token = self.token(row_scopes);
        client_async(format!("ws://{}/?hs_token={token}", self.addr), stream)
            .await
            .unwrap()
            .0
    }

    /// Apply source patches as one batch and wait until they are published.
    async fn mutate(&self, mutations: Vec<(&str, &str, Value)>) {
        let slot = self.slot.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let batch = mutations
            .into_iter()
            .map(|(export, key, patch)| Mutation {
                export: export.to_string(),
                key: json!(key),
                patch,
                append: vec![],
            })
            .collect();
        self.tx
            .send(MutationBatch::with_slot_context(
                batch,
                SlotContext::new(slot, 0),
            ))
            .await
            .unwrap();
        let (ack, wait) = oneshot::channel();
        self.tx
            .send(MutationBatch::flush_marker(ack))
            .await
            .unwrap();
        wait.await.unwrap();
    }

    async fn patch(&self, key: &str, patch: Value) {
        self.mutate(vec![("Thing", key, patch)]).await;
    }
}

async fn send(socket: &mut WebSocketStream<TcpStream>, message: Value) {
    socket
        .send(Message::Text(message.to_string().into()))
        .await
        .unwrap();
}

async fn next_frame(socket: &mut WebSocketStream<TcpStream>) -> Value {
    loop {
        let message = tokio::time::timeout(Duration::from_secs(10), socket.next())
            .await
            .expect("the server answers within the timeout")
            .expect("the stream stays open")
            .expect("a readable frame");
        let bytes = match &message {
            Message::Text(text) => text.as_bytes().to_vec(),
            // Snapshots past the compression threshold arrive gzipped.
            Message::Binary(bytes) if bytes.starts_with(&[0x1f, 0x8b]) => {
                let mut json = Vec::new();
                std::io::Read::read_to_end(
                    &mut flate2::read::GzDecoder::new(bytes.as_ref()),
                    &mut json,
                )
                .expect("a gzip frame");
                json
            }
            Message::Binary(bytes) => bytes.to_vec(),
            _ => continue,
        };
        return serde_json::from_slice(&bytes).expect("frames are JSON");
    }
}

async fn subscribe(socket: &mut WebSocketStream<TcpStream>, id: &str, query: Value) {
    send(
        socket,
        json!({
            "type": "subscribe",
            "protocolVersion": 2,
            "subscriptionId": id,
            "query": query,
        }),
    )
    .await;
    let ack = next_frame(socket).await;
    assert_eq!(ack["op"], "subscribed", "unexpected ack: {ack}");
}

/// One whole snapshot: its row keys in order, and whether it is
/// authoritative.
async fn snapshot(socket: &mut WebSocketStream<TcpStream>) -> (Vec<String>, bool) {
    let mut keys = Vec::new();
    loop {
        let frame = next_frame(socket).await;
        assert_eq!(frame["op"], "snapshot", "expected a snapshot: {frame}");
        keys.extend(
            frame["data"]
                .as_array()
                .unwrap_or_else(|| panic!("snapshot rows: {frame}"))
                .iter()
                .map(|row| row["key"].as_str().unwrap().to_string()),
        );
        if frame["complete"] == json!(true) {
            return (keys, frame["authoritative"] == json!(true));
        }
    }
}

/// Frames up to and including the first one for `key`.
async fn frames_until_key(socket: &mut WebSocketStream<TcpStream>, key: &str) -> Vec<Value> {
    let mut frames = Vec::new();
    loop {
        let frame = next_frame(socket).await;
        let done = frame["key"] == json!(key);
        frames.push(frame);
        if done {
            return frames;
        }
    }
}

fn keys(frames: &[Value]) -> Vec<String> {
    frames
        .iter()
        .filter_map(|frame| frame["key"].as_str().map(str::to_string))
        .collect()
}

fn sorted(mut keys: Vec<String>) -> Vec<String> {
    keys.sort();
    keys
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

/// `a1..a3` in scope A, `b1..b3` in scope B, and public rows: `p1` without
/// the field, `p2` with `null`, `p3` with an empty scope.
async fn seed(harness: &Harness) {
    let mut rows = Vec::new();
    for (scope, prefix) in [("A", "a"), ("B", "b")] {
        for index in 1..=3 {
            rows.push((
                format!("{prefix}{index}"),
                json!({"scope": scope, "count": index}),
            ));
        }
    }
    rows.push(("p1".to_string(), json!({"count": 1})));
    rows.push(("p2".to_string(), json!({"scope": null, "count": 2})));
    rows.push(("p3".to_string(), json!({"scope": "", "count": 3})));
    harness
        .mutate(
            rows.iter()
                .map(|(key, patch)| ("Thing", key.as_str(), patch.clone()))
                .collect(),
        )
        .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_snapshot_carries_public_rows_and_only_the_sessions_own_scopes() {
    let harness = Harness::start(Options::default()).await;
    seed(&harness).await;

    for (row_scopes, expected) in [
        (
            Some(&["A"][..]),
            strings(&["a1", "a2", "a3", "p1", "p2", "p3"]),
        ),
        (
            Some(&["A", "B"][..]),
            strings(&["a1", "a2", "a3", "b1", "b2", "b3", "p1", "p2", "p3"]),
        ),
        (Some(&[][..]), strings(&["p1", "p2", "p3"])),
        (None, strings(&["p1", "p2", "p3"])),
    ] {
        let mut socket = harness.connect(row_scopes).await;
        subscribe(&mut socket, "things", json!({"view": "Thing/list"})).await;
        let (snapshot_keys, _) = snapshot(&mut socket).await;
        assert_eq!(sorted(snapshot_keys), expected, "row scopes {row_scopes:?}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn patches_and_deltas_never_reach_another_scope() {
    let harness = Harness::start(Options::default()).await;
    seed(&harness).await;
    let mut socket = harness.connect(Some(&["A"])).await;
    subscribe(&mut socket, "things", json!({"view": "Thing/list"})).await;
    snapshot(&mut socket).await;

    // Another scope's patches go nowhere; the session's own arrive.
    harness.patch("b1", json!({"count": 10})).await;
    harness.patch("b2", json!({"count": 11})).await;
    harness.patch("a1", json!({"count": 12})).await;
    let frames = frames_until_key(&mut socket, "a1").await;
    assert_eq!(keys(&frames), ["a1"], "{frames:?}");

    // A row that turns public enters the window whole.
    harness.patch("b3", json!({"scope": ""})).await;
    let frames = frames_until_key(&mut socket, "b3").await;
    assert_eq!(frames.len(), 1, "{frames:?}");
    assert_eq!(frames[0]["op"], "upsert");
    assert_eq!(frames[0]["data"]["count"], 3, "the whole row: {frames:?}");

    // A row that moves to another scope leaves it, and its later patches do
    // not follow it out.
    harness.patch("a2", json!({"scope": "B"})).await;
    let frames = frames_until_key(&mut socket, "a2").await;
    assert_eq!(frames.len(), 1, "{frames:?}");
    assert_eq!(frames[0]["op"], "remove");
    harness.patch("a2", json!({"count": 20})).await;
    harness.patch("a3", json!({"count": 21})).await;
    let frames = frames_until_key(&mut socket, "a3").await;
    assert_eq!(keys(&frames), ["a3"], "{frames:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_state_subscription_to_another_scopes_row_receives_nothing_of_it() {
    let harness = Harness::start(Options::default()).await;
    seed(&harness).await;
    let mut socket = harness.connect(Some(&["A"])).await;
    subscribe(
        &mut socket,
        "thing",
        json!({"view": "Thing/state", "key": "b1"}),
    )
    .await;
    let (snapshot_keys, _) = snapshot(&mut socket).await;
    assert!(snapshot_keys.is_empty(), "{snapshot_keys:?}");

    harness.patch("b1", json!({"count": 30})).await;
    harness.patch("b1", json!({"count": 31})).await;
    // Moving into the session's scope delivers the whole row, first: the
    // patches above never reached it.
    harness.patch("b1", json!({"scope": "A"})).await;
    let frame = next_frame(&mut socket).await;
    assert_eq!(frame["op"], "upsert", "{frame}");
    assert_eq!(frame["data"]["count"], 31);
    assert_eq!(frame["data"]["scope"], "A");

    // And out again.
    harness
        .patch("b1", json!({"scope": "B", "count": 32}))
        .await;
    let frame = next_frame(&mut socket).await;
    assert_eq!(frame["op"], "remove", "{frame}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scoped_sorted_views_return_full_pages_in_order() {
    // Each view keeps 20 rows. Unpartitioned, the top 20 of all 160 rows
    // would leave scope A a handful; per scope, A gets a full page.
    let harness = Harness::start(Options {
        max_rows: Some(PAGE),
        ..Options::default()
    })
    .await;
    let mut rows: Vec<(String, String, f64)> = Vec::new();
    for index in 0..50 {
        for (offset, scope) in [(0.0, "A"), (1.0, "B"), (2.0, "C")] {
            rows.push((
                format!("{}{index}", scope.to_lowercase()),
                scope.to_string(),
                f64::from(index) * 3.0 + offset,
            ));
        }
    }
    for index in 40..50 {
        rows.push((
            format!("p{index}"),
            String::new(),
            f64::from(index) * 3.0 + 1.5,
        ));
    }
    for chunk in rows.chunks(16) {
        harness
            .mutate(
                chunk
                    .iter()
                    .map(|(key, scope, amount)| {
                        (
                            "Thing",
                            key.as_str(),
                            json!({"scope": scope, "amount": amount}),
                        )
                    })
                    .collect(),
            )
            .await;
    }

    for scope in ["A", "B", "C"] {
        let mut expected: Vec<&(String, String, f64)> = rows
            .iter()
            .filter(|(_, row_scope, _)| row_scope == scope || row_scope.is_empty())
            .collect();
        expected.sort_by(|left, right| right.2.total_cmp(&left.2));
        let expected: Vec<String> = expected
            .iter()
            .take(PAGE)
            .map(|(key, _, _)| key.clone())
            .collect();

        let mut socket = harness.connect(Some(&[scope])).await;
        subscribe(&mut socket, "top", json!({"view": "Thing/top"})).await;
        let (page, _) = snapshot(&mut socket).await;
        assert_eq!(page.len(), PAGE, "a full page for {scope}: {page:?}");
        assert_eq!(page, expected, "scope {scope}");
        assert!(
            page.iter().any(|key| key.starts_with('p')),
            "public rows interleave in order: {page:?}"
        );
    }

    // A session without scopes pages through the public rows alone.
    let mut socket = harness.connect(None).await;
    subscribe(&mut socket, "top", json!({"view": "Thing/top"})).await;
    let (page, _) = snapshot(&mut socket).await;
    assert_eq!(page.len(), 10);
    assert!(page.iter().all(|key| key.starts_with('p')), "{page:?}");

    // Live: a new B row that outranks everything changes nothing for A; a
    // new A row enters A's page.
    let mut socket = harness.connect(Some(&["A"])).await;
    subscribe(&mut socket, "top", json!({"view": "Thing/top"})).await;
    snapshot(&mut socket).await;
    harness
        .patch("b-top", json!({"scope": "B", "amount": 10_000.0}))
        .await;
    harness
        .patch("a-top", json!({"scope": "A", "amount": 9_000.0}))
        .await;
    let frames = frames_until_key(&mut socket, "a-top").await;
    assert!(
        keys(&frames).iter().all(|key| !key.starts_with('b')),
        "{frames:?}"
    );
    assert_eq!(frames.last().unwrap()["op"], "upsert");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_client_supplied_partition_or_filter_cannot_widen_its_scopes() {
    let harness = Harness::start(Options::default()).await;
    seed(&harness).await;
    harness.patch("b1", json!({"_partition": "B"})).await;
    let mut socket = harness.connect(Some(&["A"])).await;
    for (id, query) in [
        (
            "by-filter",
            json!({"view": "Thing/list", "filters": {"scope": "B"}}),
        ),
        (
            "by-partition",
            json!({"view": "Thing/list", "partition": "B"}),
        ),
        ("by-key", json!({"view": "Thing/list", "key": "b1"})),
        ("by-state", json!({"view": "Thing/state", "key": "b1"})),
    ] {
        subscribe(&mut socket, id, query).await;
        let (snapshot_keys, _) = snapshot(&mut socket).await;
        assert!(snapshot_keys.is_empty(), "{id}: {snapshot_keys:?}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unscoped_entity_is_unaffected() {
    let harness = Harness::start(Options::default()).await;
    harness
        .mutate(vec![
            ("Other", "o1", json!({"scope": "B"})),
            ("Other", "o2", json!({"scope": "A"})),
            ("Other", "o3", json!({})),
        ])
        .await;
    let mut socket = harness.connect(Some(&["A"])).await;
    subscribe(&mut socket, "others", json!({"view": "Other/list"})).await;
    let (snapshot_keys, _) = snapshot(&mut socket).await;
    assert_eq!(sorted(snapshot_keys), strings(&["o1", "o2", "o3"]));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refresh_with_different_row_scopes_resnapshots() {
    let harness = Harness::start(Options::default()).await;
    seed(&harness).await;
    let mut socket = harness.connect(Some(&["A"])).await;
    subscribe(&mut socket, "things", json!({"view": "Thing/list"})).await;
    let (first, _) = snapshot(&mut socket).await;
    assert_eq!(
        sorted(first),
        strings(&["a1", "a2", "a3", "p1", "p2", "p3"])
    );
    subscribe(&mut socket, "others", json!({"view": "Other/list"})).await;
    let (others, _) = snapshot(&mut socket).await;
    assert!(others.is_empty());

    // The same scopes: nothing is resent.
    send(
        &mut socket,
        json!({"type": "refresh_auth", "token": harness.token(Some(&["A"]))}),
    )
    .await;
    let reply = next_frame(&mut socket).await;
    assert_eq!(reply["success"], true, "{reply}");
    harness.patch("a1", json!({"count": 40})).await;
    let frames = frames_until_key(&mut socket, "a1").await;
    assert_eq!(
        frames.len(),
        1,
        "no resnapshot for unchanged scopes: {frames:?}"
    );

    // New scopes: one authoritative snapshot of what they cover, for the
    // scoped subscription only.
    send(
        &mut socket,
        json!({"type": "refresh_auth", "token": harness.token(Some(&["B"]))}),
    )
    .await;
    let reply = next_frame(&mut socket).await;
    assert_eq!(reply["success"], true, "{reply}");
    let (rescoped, authoritative) = snapshot(&mut socket).await;
    assert!(
        authoritative,
        "the resnapshot replaces what the client holds"
    );
    assert_eq!(
        sorted(rescoped),
        strings(&["b1", "b2", "b3", "p1", "p2", "p3"])
    );

    // The old scope is gone from the live stream too.
    harness.patch("a1", json!({"count": 41})).await;
    harness.patch("b1", json!({"count": 42})).await;
    let frames = frames_until_key(&mut socket, "b1").await;
    assert_eq!(keys(&frames), ["b1"], "{frames:?}");
    assert_eq!(frames[0]["subscriptionId"], "things");

    // Dropping the claim narrows to public rows.
    send(
        &mut socket,
        json!({"type": "refresh_auth", "token": harness.token(None)}),
    )
    .await;
    let reply = next_frame(&mut socket).await;
    assert_eq!(reply["success"], true, "{reply}");
    let (public, authoritative) = snapshot(&mut socket).await;
    assert!(authoritative);
    assert_eq!(sorted(public), strings(&["p1", "p2", "p3"]));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn a_lagged_list_recovers_only_rows_in_scope() {
    let harness = Harness::start(Options {
        bus_capacity: Some(2),
        ..Options::default()
    })
    .await;
    seed(&harness).await;
    let mut socket = harness.connect(Some(&["A"])).await;
    subscribe(&mut socket, "things", json!({"view": "Thing/list"})).await;
    snapshot(&mut socket).await;

    // One batch far larger than the bus: the subscriber laps and recovers
    // from an authoritative snapshot.
    let mut burst = Vec::new();
    for index in 0..40 {
        let key = if index % 2 == 0 { "a1" } else { "b1" };
        burst.push(("Thing", key, json!({"count": 100 + index})));
    }
    harness.mutate(burst).await;
    let recovered = loop {
        let frame = next_frame(&mut socket).await;
        assert_ne!(frame["key"], "b1", "{frame}");
        if frame["op"] == "snapshot" {
            let mut keys: Vec<String> = frame["data"]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| row["key"].as_str().unwrap().to_string())
                .collect();
            assert_eq!(frame["authoritative"], true, "{frame}");
            if frame["complete"] != json!(true) {
                let (rest, _) = snapshot(&mut socket).await;
                keys.extend(rest);
            }
            break keys;
        }
    };
    assert_eq!(
        sorted(recovered),
        strings(&["a1", "a2", "a3", "p1", "p2", "p3"])
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_replay_and_its_live_tail_carry_only_rows_in_scope() {
    let harness = Harness::start(Options {
        journal: true,
        ..Options::default()
    })
    .await;
    // The first record of each pool sets its scope; the rest are patches
    // without it, whose scope is the cached row's.
    harness
        .mutate(vec![
            ("Trade", "poolA", json!({"scope": "A", "trade": 0})),
            ("Trade", "poolB", json!({"scope": "B", "trade": 1})),
            ("Trade", "poolP", json!({"trade": 2})),
        ])
        .await;
    for trade in 3..12 {
        let key = ["poolA", "poolB", "poolP"][trade % 3];
        harness
            .mutate(vec![("Trade", key, json!({"trade": trade}))])
            .await;
    }
    let journal = harness.journal.clone().unwrap();
    let window = journal.window("Trade/append").await;
    let cursor = format!("{}:{}", window.epoch, 0);

    let mut socket = harness.connect(Some(&["A"])).await;
    subscribe(
        &mut socket,
        "trades",
        json!({"view": "Trade/append", "after": cursor}),
    )
    .await;
    let mut seen = Vec::new();
    while seen.len() < 5 {
        let frame = next_frame(&mut socket).await;
        assert_ne!(frame["type"], "error", "{frame}");
        assert_ne!(frame["key"], "poolB", "a replayed B record: {frame}");
        if let Some(trade) = frame["data"]["trade"].as_u64() {
            seen.push(trade);
        }
    }
    // Past the cursor: A's and the public pool's records, never B's.
    assert!(seen.iter().all(|trade| trade % 3 != 1), "{seen:?}");

    // Live: B's records stay away, A's arrive.
    harness
        .mutate(vec![("Trade", "poolB", json!({"trade": 100}))])
        .await;
    harness
        .mutate(vec![("Trade", "poolA", json!({"trade": 101}))])
        .await;
    loop {
        let frame = next_frame(&mut socket).await;
        assert_ne!(frame["key"], "poolB", "a live B record: {frame}");
        if frame["data"]["trade"] == 101 {
            break;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refreshed_replay_stops_with_a_rescoped_issue() {
    let harness = Harness::start(Options {
        journal: true,
        ..Options::default()
    })
    .await;
    harness
        .mutate(vec![("Trade", "poolA", json!({"scope": "A", "trade": 0}))])
        .await;
    let mut socket = harness.connect(Some(&["A"])).await;
    subscribe(&mut socket, "trades", json!({"view": "Trade/append"})).await;
    send(
        &mut socket,
        json!({"type": "refresh_auth", "token": harness.token(Some(&["B"]))}),
    )
    .await;
    let mut codes = BTreeSet::new();
    while !codes.contains("subscription-rescoped") {
        let frame = next_frame(&mut socket).await;
        if let Some(code) = frame["code"].as_str() {
            codes.insert(code.to_string());
        }
    }
    harness
        .mutate(vec![("Trade", "poolA", json!({"trade": 1}))])
        .await;
    assert!(
        tokio::time::timeout(Duration::from_millis(300), socket.next())
            .await
            .is_err(),
        "the stopped tape sends nothing more"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_public_transition_does_not_expose_earlier_private_patches_in_replay() {
    let harness = Harness::start(Options {
        journal: true,
        ..Default::default()
    })
    .await;
    harness
        .mutate(vec![(
            "Trade",
            "pool",
            json!({"scope":"B","trade":"private-start"}),
        )])
        .await;
    harness
        .mutate(vec![("Trade", "pool", json!({"trade":"private-patch"}))])
        .await;
    harness
        .mutate(vec![(
            "Trade",
            "pool",
            json!({"scope":null,"trade":"public-transition"}),
        )])
        .await;
    let mut socket = harness.connect(Some(&["A"])).await;
    let window = harness
        .journal
        .as_ref()
        .unwrap()
        .window("Trade/append")
        .await;
    subscribe(
        &mut socket,
        "trades",
        json!({"view":"Trade/append","after":format!("{}:0",window.epoch)}),
    )
    .await;
    let frame = next_frame(&mut socket).await;
    assert_eq!(
        frame["data"]["trade"], "public-transition",
        "private history leaked: {frame}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_evicted_rows_explicit_scope_still_governs_live_append_delivery() {
    let harness = Harness::start(Options {
        journal: true,
        max_rows: Some(1),
        ..Default::default()
    })
    .await;
    harness
        .mutate(vec![("Trade", "evicted", json!({"scope":"B","trade":0}))])
        .await;
    harness
        .mutate(vec![("Trade", "other", json!({"scope":"B","trade":1}))])
        .await;
    let mut entitled = harness.connect(Some(&["B"])).await;
    let mut other = harness.connect(Some(&["A"])).await;
    let window = harness
        .journal
        .as_ref()
        .unwrap()
        .window("Trade/append")
        .await;
    let query =
        json!({"view":"Trade/append", "after": format!("{}:{}", window.epoch, window.next - 1)});
    subscribe(&mut entitled, "trades", query.clone()).await;
    subscribe(&mut other, "trades", query).await;
    harness
        .mutate(vec![("Trade", "evicted", json!({"scope":"B","trade":2}))])
        .await;
    assert_eq!(
        next_frame(&mut entitled).await["data"]["trade"],
        2,
        "an explicitly scoped event must survive cache eviction"
    );
    // With neither a scope in the patch nor a cached row, fail closed.
    harness
        .mutate(vec![("Trade", "evicted", json!({"trade":3}))])
        .await;
    harness
        .mutate(vec![("Trade", "evicted", json!({"scope":null,"trade":4}))])
        .await;
    assert_eq!(next_frame(&mut entitled).await["data"]["trade"], 4);
    assert_eq!(
        next_frame(&mut other).await["data"]["trade"],
        4,
        "private or unknown-scope frame leaked"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn replacing_a_scope_parent_publishes_the_public_transition_live_and_in_replay() {
    let mut index = scoped_index();
    index.set_entity_scope("Trade", ScopeField::new("meta.access.scope"));
    let harness = Harness::start_with_index(
        Options {
            journal: true,
            ..Default::default()
        },
        index,
    )
    .await;
    harness
        .mutate(vec![(
            "Trade",
            "row",
            json!({"meta":{"access":{"scope":"B"}},"trade":0}),
        )])
        .await;
    let window = harness
        .journal
        .as_ref()
        .unwrap()
        .window("Trade/append")
        .await;
    let query = json!({"view":"Trade/append","after":format!("{}:{}",window.epoch,window.next-1)});
    let mut live = harness.connect(None).await;
    subscribe(&mut live, "live", query.clone()).await;
    // A partial object preserves the private scope. Neither this patch nor
    // the private ancestor values it merges with may reach the public reader.
    harness
        .mutate(vec![(
            "Trade",
            "row",
            json!({"meta":{"other":true},"trade":1}),
        )])
        .await;
    harness
        .mutate(vec![(
            "Trade",
            "row",
            json!({"meta":{"access":null},"trade":2}),
        )])
        .await;
    assert_eq!(next_frame(&mut live).await["data"]["trade"], 2);
    let mut replay = harness.connect(None).await;
    subscribe(&mut replay, "replay", query).await;
    assert_eq!(next_frame(&mut replay).await["data"]["trade"], 2);
    // Clearing the outer parent has the same effect.
    harness
        .mutate(vec![(
            "Trade",
            "row",
            json!({"meta":{"access":{"scope":"B"}},"trade":3}),
        )])
        .await;
    harness
        .mutate(vec![("Trade", "row", json!({"meta":null,"trade":4}))])
        .await;
    assert_eq!(next_frame(&mut live).await["data"]["trade"], 4);
    assert_eq!(next_frame(&mut replay).await["data"]["trade"], 4);
}
