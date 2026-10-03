//! Every frame the projector publishes carries `_version`: `{epoch}:{counter}`,
//! its place in the order the projector merged changes into the entity cache.
//!
//! Clients order one key's frames by it. `_seq` cannot do that: every update
//! decoded from one transaction shares it, and within a slot account updates
//! and instructions number themselves differently. The version rides in the
//! entity data, so whatever the server builds from the cache (snapshot rows,
//! derived views, a state subscriber's catch-up) carries the latest one.

use arete_interpreter::Mutation;
use arete_server::materialized_view::{SortConfig, SortOrder, ViewPipeline};
use arete_server::{
    BusManager, Delivery, EntityCache, Filters, Mode, MutationBatch, Projection, Projector,
    SlotContext, ViewIndex, ViewSpec,
};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

fn spec(id: &str, mode: Mode, projection: Projection) -> ViewSpec {
    ViewSpec {
        id: id.to_string(),
        export: id.split('/').next().unwrap().to_string(),
        mode,
        wire_format: Default::default(),
        projection,
        filters: Filters::all(),
        delivery: Delivery::default(),
        pipeline: None,
        source_view: None,
    }
}

struct Harness {
    index: ViewIndex,
    cache: EntityCache,
    bus: BusManager,
    tx: mpsc::Sender<MutationBatch>,
}

impl Harness {
    fn start(index: ViewIndex) -> Self {
        let cache = EntityCache::new();
        let bus = BusManager::new();
        let (tx, rx) = mpsc::channel(64);
        tokio::spawn(
            Projector::new(
                Arc::new(index.clone()),
                bus.clone(),
                cache.clone(),
                rx,
                #[cfg(feature = "otel")]
                None,
            )
            .run(),
        );
        Self {
            index,
            cache,
            bus,
            tx,
        }
    }

    async fn send(&self, slot: u64, index: u64, patches: Vec<(&str, Value)>) {
        let mutations = patches
            .into_iter()
            .map(|(key, patch)| Mutation {
                export: "Round".to_string(),
                key: json!(key),
                patch,
                append: vec![],
            })
            .collect();
        self.tx
            .send(MutationBatch::with_slot_context(
                mutations,
                SlotContext::new(slot, index),
            ))
            .await
            .unwrap();
        self.flush().await;
    }

    async fn flush(&self) {
        let (ack, wait) = tokio::sync::oneshot::channel();
        self.tx
            .send(MutationBatch::flush_marker(ack))
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), wait)
            .await
            .expect("the projector acks the flush")
            .unwrap();
    }
}

/// `(epoch, counter)` from a `_version`.
fn version(data: &Value) -> (String, u64) {
    let version = data["_version"]
        .as_str()
        .unwrap_or_else(|| panic!("no _version in {data}"));
    let (epoch, counter) = version.split_once(':').expect("epoch:counter");
    assert_eq!(epoch.len(), 8, "eight hex digits: {version}");
    assert!(
        epoch.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "{version}"
    );
    (epoch.to_string(), counter.parse().expect("decimal counter"))
}

fn frame(payload: &[u8]) -> Value {
    serde_json::from_slice(payload).expect("frames are JSON")
}

/// Two patches from one transaction share a `seq`, which is why a client
/// cannot tell them apart by it. Their versions differ, in the order they
/// were merged.
#[tokio::test]
async fn one_transactions_patches_share_a_seq_but_not_a_version() {
    let mut index = ViewIndex::new();
    index.add_spec(spec("Round/list", Mode::List, Projection::all()));
    let harness = Harness::start(index);
    let mut frames = harness.bus.get_or_create_list_bus("Round/list").await;

    harness
        .send(
            140,
            1200,
            vec![
                ("7", json!({"entropy": {"seed": "abc"}})),
                ("7", json!({"results": {"winning_square": 3}})),
            ],
        )
        .await;

    let first = frame(&frames.recv().await.unwrap().payload);
    let second = frame(&frames.recv().await.unwrap().payload);
    assert_eq!(first["seq"], second["seq"], "one transaction, one seq");
    let (first_epoch, first_counter) = version(&first["data"]);
    let (second_epoch, second_counter) = version(&second["data"]);
    assert_eq!(first_epoch, second_epoch);
    assert!(second_counter > first_counter, "{first} then {second}");

    let cached = harness.cache.get("Round/list", "7").await.unwrap();
    assert_eq!(cached["_version"], second["data"]["_version"]);
    assert_eq!(cached["entropy"]["seed"], "abc");
    assert_eq!(cached["results"]["winning_square"], 3);
}

/// A projection keeps only its listed fields. The version is stamped after
/// it, so the frame and the cache still carry one.
#[tokio::test]
async fn a_projection_with_a_field_list_keeps_the_version() {
    let mut index = ViewIndex::new();
    index.add_spec(spec(
        "Round/list",
        Mode::List,
        Projection {
            fields: Some(vec!["total".to_string()]),
        },
    ));
    let harness = Harness::start(index);
    let mut frames = harness.bus.get_or_create_list_bus("Round/list").await;

    harness
        .send(141, 2, vec![("7", json!({"total": 1, "hidden": true}))])
        .await;

    let published = frame(&frames.recv().await.unwrap().payload);
    assert_eq!(published["data"]["hidden"], Value::Null, "projected away");
    version(&published["data"]);
    let cached = harness.cache.get("Round/list", "7").await.unwrap();
    assert_eq!(cached["_version"], published["data"]["_version"]);
}

/// A keyed state subscriber's frames, and anything the server rebuilds from
/// the cache (its catch-up, a snapshot row, a derived view's copy), carry the
/// version of the latest change merged.
#[tokio::test]
async fn frames_and_everything_built_from_the_cache_carry_the_latest_version() {
    let mut index = ViewIndex::new();
    index.add_spec(spec("Round/state", Mode::State, Projection::all()));
    index.add_spec(spec("Round/list", Mode::List, Projection::all()));
    index.add_spec(ViewSpec {
        pipeline: Some(ViewPipeline {
            filter: None,
            sort: Some(SortConfig {
                field_path: vec!["id".to_string()],
                order: SortOrder::Desc,
            }),
            limit: None,
        }),
        source_view: Some("Round/list".to_string()),
        ..spec("Round/latest", Mode::List, Projection::all())
    });
    let harness = Harness::start(index);
    let mut state = harness
        .bus
        .get_or_create_state_bus("Round/state", "7")
        .await;

    harness.send(150, 3, vec![("7", json!({"id": 7}))]).await;
    harness.send(150, 9, vec![("7", json!({"total": 2}))]).await;

    let latest_state_frame = frame(&state.borrow_and_update().payload);
    let cached_state = harness.cache.get("Round/state", "7").await.unwrap();
    assert_eq!(
        cached_state["_version"], latest_state_frame["data"]["_version"],
        "a catch-up sends this cached entity, so it carries the latest version"
    );

    let cached_list = harness.cache.get("Round/list", "7").await.unwrap();
    let caches = harness.index.sorted_caches();
    let mut caches = caches.write().await;
    let window = caches
        .get_mut("Round/latest")
        .expect("derived view cache")
        .get_window(0, usize::MAX);
    let (_, derived) = window.first().expect("round 7 ranks");
    assert_eq!(derived["_version"], cached_list["_version"]);
    assert_eq!(derived["total"], 2);

    // Each view's frame gets its own place in the order.
    assert_ne!(cached_state["_version"], cached_list["_version"]);
    assert_eq!(version(&cached_state).0, version(&cached_list).0);
}

/// A version is only comparable within its epoch, and each projector (a
/// restart, a stack loaded again) starts a new one.
#[tokio::test]
async fn each_projector_has_its_own_epoch() {
    let mut index = ViewIndex::new();
    index.add_spec(spec("Round/list", Mode::List, Projection::all()));
    let first = Harness::start(index.clone());
    let second = Harness::start(index);

    first.send(160, 1, vec![("7", json!({"total": 1}))]).await;
    second.send(160, 1, vec![("7", json!({"total": 1}))]).await;

    let first = version(&first.cache.get("Round/list", "7").await.unwrap());
    let second = version(&second.cache.get("Round/list", "7").await.unwrap());
    assert_ne!(first.0, second.0);
    assert_eq!((first.1, second.1), (1, 1), "counters start over");
}
