//! End-to-end snapshot/restore cycle: drive mutations through the real
//! projector, snapshot, simulate a restart, and assert entity parity in the
//! VM, the entity cache, and rebuilt sorted views.
//!
//! The snapshot registry is process-global, so tests that touch it serialize
//! on `GLOBAL_LOCK`; holding that guard across awaits is the point.
#![allow(clippy::await_holding_lock)]

use arete_server::materialized_view::{SortConfig, SortOrder, ViewPipeline};
use arete_server::snapshot::{self, SnapshotConfig, SnapshotService, SnapshotTrigger};
use arete_server::{
    BusManager, Delivery, EntityCache, Filters, Mode, MutationBatch, Projection, Projector,
    SlotContext, SlotTracker, Spec, ViewIndex, ViewSpec,
};
use arete_interpreter::vm::VmContext;
use arete_interpreter::Mutation;
use serde_json::json;
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;
use tokio::sync::mpsc;

static GLOBAL_LOCK: StdMutex<()> = StdMutex::new(());

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "arete-snapshot-restore-{tag}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn make_spec(entity_name: &str) -> Spec {
    use arete_interpreter::ast::{IdentitySpec, TypedStreamSpec};

    let entity_spec = TypedStreamSpec::<serde_json::Value>::new(
        entity_name.to_string(),
        IdentitySpec {
            primary_keys: vec!["id".to_string()],
            lookup_indexes: Vec::new(),
        },
        Vec::new(),
    );
    let bytecode = arete_interpreter::compiler::MultiEntityBytecode::new()
        .add_entity(entity_name.to_string(), entity_spec, 1)
        .build();
    Spec::new(bytecode, "Program111")
}

/// A canonical `Token/list` view plus a derived, price-sorted `Token/top`.
fn make_view_index() -> ViewIndex {
    let mut index = ViewIndex::new();
    index.add_spec(ViewSpec {
        id: "Token/list".to_string(),
        export: "Token".to_string(),
        mode: Mode::List,
        wire_format: Default::default(),
        projection: Projection::all(),
        filters: Filters::all(),
        delivery: Delivery::default(),
        pipeline: None,
        source_view: None,
    });
    index.add_spec(ViewSpec {
        id: "Token/top".to_string(),
        export: "Token".to_string(),
        mode: Mode::List,
        wire_format: Default::default(),
        projection: Projection::all(),
        filters: Filters::all(),
        delivery: Delivery::default(),
        pipeline: Some(ViewPipeline {
            filter: None,
            sort: Some(SortConfig {
                field_path: vec!["price".to_string()],
                order: SortOrder::Desc,
            }),
            limit: None,
        }),
        source_view: Some("Token/list".to_string()),
    });
    index
}

fn config_for(dir: &std::path::Path) -> SnapshotConfig {
    SnapshotConfig {
        enabled: true,
        url: Some(dir.display().to_string()),
        // The periodic task is never relied on in tests; snapshot_now is
        // called directly.
        interval: Duration::from_secs(3_600),
        ready_max_lag_slots: 100,
        ..SnapshotConfig::default()
    }
}

fn spawn_projector(
    view_index: &ViewIndex,
    entity_cache: &EntityCache,
) -> mpsc::Sender<MutationBatch> {
    let (tx, rx) = mpsc::channel::<MutationBatch>(64);
    let projector = Projector::new(
        Arc::new(view_index.clone()),
        BusManager::new(),
        entity_cache.clone(),
        rx,
    );
    tokio::spawn(projector.run());
    tx
}

fn token_batch(id: &str, price: u64, slot: u64) -> MutationBatch {
    let mutation = Mutation {
        export: "Token".to_string(),
        key: json!(id),
        patch: json!({"id": id, "price": price}),
        append: vec![],
    };
    MutationBatch::with_slot_context(
        vec![mutation].into_iter().collect(),
        SlotContext::new(slot, 1),
    )
}

async fn flush_projector(tx: &mpsc::Sender<MutationBatch>) {
    let (ack_tx, ack_rx) = tokio::sync::oneshot::channel();
    tx.send(MutationBatch::flush_marker(ack_tx)).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), ack_rx)
        .await
        .expect("projector should ack the flush marker")
        .unwrap();
}

#[tokio::test]
async fn snapshot_then_restore_recovers_vm_and_caches() {
    let _guard = GLOBAL_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    snapshot::reset_for_tests();

    let dir = temp_dir("cycle");
    let config = config_for(&dir);
    let spec = make_spec("Token");

    // --- First server lifetime ---
    let view_index = make_view_index();
    let entity_cache = EntityCache::new();
    let tx = spawn_projector(&view_index, &entity_cache);

    let service = SnapshotService::initialize(
        config.clone(),
        &spec,
        entity_cache.clone(),
        &view_index,
        tx.clone(),
    )
    .await
    .unwrap();
    assert!(
        snapshot::take_restored().is_none(),
        "empty store must cold start"
    );

    // Register a populated VM, as the generated runtime would.
    let vm = Arc::new(StdMutex::new(VmContext::new()));
    {
        let mut guard = vm.lock().unwrap();
        let table = guard.get_state_table_mut(0).unwrap();
        table.insert_with_eviction(json!("mint1"), json!({"id": "mint1", "price": 10}));
        table.insert_with_eviction(json!("mint2"), json!({"id": "mint2", "price": 20}));
        assert!(table.is_fresh_update(&json!("mint1"), "TokenState", 100, 1));
        assert!(table.is_fresh_update(&json!("mint2"), "TokenState", 110, 1));
    }
    let slot_tracker = SlotTracker::new();
    slot_tracker.record(120);
    snapshot::register_runtime(vm.clone(), slot_tracker);

    // Drive the projection path and wait until both batches are applied so
    // the recorded watermark is deterministic (110).
    tx.send(token_batch("mint1", 10, 100)).await.unwrap();
    tx.send(token_batch("mint2", 20, 110)).await.unwrap();
    flush_projector(&tx).await;

    assert!(service
        .snapshot_now(SnapshotTrigger::Periodic)
        .await
        .unwrap());

    // A second cycle with no new mutations is skipped (adaptive cadence).
    assert!(!service
        .snapshot_now(SnapshotTrigger::Periodic)
        .await
        .unwrap());
    // ...but a shutdown snapshot is always taken.
    assert!(service
        .snapshot_now(SnapshotTrigger::Shutdown)
        .await
        .unwrap());

    // --- Simulated restart ---
    snapshot::reset_for_tests();
    let view_index2 = make_view_index();
    let entity_cache2 = EntityCache::new();
    let tx2 = spawn_projector(&view_index2, &entity_cache2);

    let _service2 = SnapshotService::initialize(
        config.clone(),
        &spec,
        entity_cache2.clone(),
        &view_index2,
        tx2,
    )
    .await
    .unwrap();

    // Entity cache parity, including a WS snapshot-on-subscribe read.
    let mut entities = entity_cache2.get_all("Token/list").await;
    entities.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(entities.len(), 2);
    assert_eq!(entities[0].1["price"], 10);
    assert_eq!(entities[1].1["price"], 20);

    // Sorted view rebuilt from the entity cache, in sort order.
    {
        let sorted_caches = view_index2.sorted_caches();
        let mut caches = sorted_caches.write().await;
        let cache = caches.get_mut("Token/top").expect("sorted cache exists");
        let window = cache.get_window(0, 10);
        assert_eq!(
            window
                .iter()
                .map(|(key, _)| key.as_str())
                .collect::<Vec<_>>(),
            vec!["mint2", "mint1"],
            "price-desc order after rebuild"
        );
    }

    // The VM portion is stashed exactly once, with the projector watermark.
    let restored = snapshot::take_restored().expect("VM snapshot stashed");
    assert_eq!(restored.resume_watermark, Some(110));
    assert!(snapshot::take_restored().is_none(), "consumed exactly once");

    let mut vm2 = VmContext::new();
    vm2.hydrate(restored.vm);
    assert_eq!(
        vm2.get_entity_state(0, &json!("mint1")),
        Some(json!({"id": "mint1", "price": 10}))
    );
    let table = vm2.get_state_table_mut(0).unwrap();
    // Overlap replay from slot 110 must be dropped as stale...
    assert!(!table.is_fresh_update(&json!("mint2"), "TokenState", 110, 1));
    // ...while genuinely new updates are applied.
    assert!(table.is_fresh_update(&json!("mint2"), "TokenState", 111, 1));

    // Readiness gate: active after a watermark resume. With no runtime
    // registered there is no tip yet, so the pod is not ready...
    assert!(!snapshot::resume_gate_ready());
    // ...and becomes ready once the parser side registers and the applied
    // watermark (110) is within `ready_max_lag_slots` of the tip.
    let tracker = SlotTracker::new();
    tracker.record(150);
    snapshot::register_runtime(Arc::new(StdMutex::new(VmContext::new())), tracker);
    assert!(snapshot::resume_gate_ready());
    // The gate is sticky-off once released.
    assert!(snapshot::resume_gate_ready());

    snapshot::reset_for_tests();
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn mismatched_bytecode_and_corrupt_blobs_cold_start() {
    let _guard = GLOBAL_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    snapshot::reset_for_tests();

    let dir = temp_dir("mismatch");
    let config = config_for(&dir);

    // Write a snapshot for entity "Token".
    let view_index = make_view_index();
    let entity_cache = EntityCache::new();
    let tx = spawn_projector(&view_index, &entity_cache);
    let spec = make_spec("Token");
    let service = SnapshotService::initialize(
        config.clone(),
        &spec,
        entity_cache.clone(),
        &view_index,
        tx.clone(),
    )
    .await
    .unwrap();

    let vm = Arc::new(StdMutex::new(VmContext::new()));
    vm.lock()
        .unwrap()
        .get_state_table_mut(0)
        .unwrap()
        .insert_with_eviction(json!("mint1"), json!({"id": "mint1"}));
    snapshot::register_runtime(vm, SlotTracker::new());
    tx.send(token_batch("mint1", 10, 100)).await.unwrap();
    flush_projector(&tx).await;
    assert!(service
        .snapshot_now(SnapshotTrigger::Shutdown)
        .await
        .unwrap());

    // Restart with a different stack build (different bytecode fingerprint):
    // the snapshot must be discarded.
    snapshot::reset_for_tests();
    let entity_cache2 = EntityCache::new();
    let view_index2 = make_view_index();
    let tx2 = spawn_projector(&view_index2, &entity_cache2);
    let _service2 = SnapshotService::initialize(
        config.clone(),
        &make_spec("Renamed"),
        entity_cache2.clone(),
        &view_index2,
        tx2,
    )
    .await
    .unwrap();
    assert!(snapshot::take_restored().is_none());
    assert!(entity_cache2.get_all("Token/list").await.is_empty());
    assert!(
        snapshot::resume_gate_ready(),
        "no gate on cold start"
    );

    // A corrupt newest blob must also cold start, not crash.
    std::fs::write(
        dir.join("snapshot-999999999999999-000000000000000.arsnap"),
        b"not a snapshot",
    )
    .unwrap();
    snapshot::reset_for_tests();
    let entity_cache3 = EntityCache::new();
    let view_index3 = make_view_index();
    let tx3 = spawn_projector(&view_index3, &entity_cache3);
    let _service3 = SnapshotService::initialize(
        config,
        &spec,
        entity_cache3.clone(),
        &view_index3,
        tx3,
    )
    .await
    .unwrap();
    assert!(snapshot::take_restored().is_none());

    snapshot::reset_for_tests();
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn stale_snapshot_hydrates_but_starts_live() {
    let _guard = GLOBAL_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    snapshot::reset_for_tests();

    let dir = temp_dir("stale");
    let mut config = config_for(&dir);
    // Anything older than "0 slots" is stale: every restore clamps to live.
    config.max_resume_age_slots = 0;

    let view_index = make_view_index();
    let entity_cache = EntityCache::new();
    let tx = spawn_projector(&view_index, &entity_cache);
    let spec = make_spec("Token");
    let service = SnapshotService::initialize(
        config.clone(),
        &spec,
        entity_cache.clone(),
        &view_index,
        tx.clone(),
    )
    .await
    .unwrap();

    let vm = Arc::new(StdMutex::new(VmContext::new()));
    vm.lock()
        .unwrap()
        .get_state_table_mut(0)
        .unwrap()
        .insert_with_eviction(json!("mint1"), json!({"id": "mint1"}));
    snapshot::register_runtime(vm, SlotTracker::new());
    tx.send(token_batch("mint1", 10, 100)).await.unwrap();
    flush_projector(&tx).await;
    // Ensure measurable snapshot age before restoring.
    assert!(service
        .snapshot_now(SnapshotTrigger::Shutdown)
        .await
        .unwrap());
    tokio::time::sleep(Duration::from_millis(500)).await;

    snapshot::reset_for_tests();
    let entity_cache2 = EntityCache::new();
    let view_index2 = make_view_index();
    let tx2 = spawn_projector(&view_index2, &entity_cache2);
    let _service2 = SnapshotService::initialize(
        config,
        &spec,
        entity_cache2.clone(),
        &view_index2,
        tx2,
    )
    .await
    .unwrap();

    let restored = snapshot::take_restored().expect("state still hydrates");
    assert_eq!(
        restored.resume_watermark, None,
        "stale snapshot must start live"
    );
    assert_eq!(entity_cache2.get_all("Token/list").await.len(), 1);
    assert!(
        snapshot::resume_gate_ready(),
        "no readiness gate without a watermark resume"
    );

    snapshot::reset_for_tests();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn snapshot_config_from_env_round_trip() {
    // No other test reads these variables, so this is parallel-safe.
    std::env::set_var("ARETE_SNAPSHOT_ENABLED", "true");
    std::env::set_var("ARETE_SNAPSHOT_URL", "file:///tmp/arete-snaps");
    std::env::set_var("ARETE_SNAPSHOT_INTERVAL_SECS", "120");
    std::env::set_var("ARETE_SNAPSHOT_KEEP", "7");
    std::env::set_var("ARETE_SNAPSHOT_ON_SHUTDOWN", "false");
    std::env::set_var("ARETE_SNAPSHOT_MIN_MUTATIONS", "5");
    std::env::set_var("ARETE_SNAPSHOT_MAX_RESUME_AGE_SLOTS", "9000");

    let config = SnapshotConfig::from_env().unwrap();
    assert!(config.enabled);
    assert_eq!(config.url.as_deref(), Some("file:///tmp/arete-snaps"));
    assert_eq!(config.interval, Duration::from_secs(120));
    assert_eq!(config.keep, 7);
    assert!(!config.snapshot_on_shutdown);
    assert_eq!(config.min_mutations, 5);
    assert_eq!(config.max_resume_age_slots, 9_000);

    // Enabled without a URL is a configuration error.
    std::env::remove_var("ARETE_SNAPSHOT_URL");
    assert!(SnapshotConfig::from_env().is_err());

    for key in [
        "ARETE_SNAPSHOT_ENABLED",
        "ARETE_SNAPSHOT_INTERVAL_SECS",
        "ARETE_SNAPSHOT_KEEP",
        "ARETE_SNAPSHOT_ON_SHUTDOWN",
        "ARETE_SNAPSHOT_MIN_MUTATIONS",
        "ARETE_SNAPSHOT_MAX_RESUME_AGE_SLOTS",
    ] {
        std::env::remove_var(key);
    }
    assert!(!SnapshotConfig::from_env().unwrap().enabled);
}
