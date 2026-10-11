// Runs the generated slot-scheduler task against a real projector and
// snapshot service: a restart must resume where the parser stopped, not at
// the slot tip the scheduler stamped its batch with.
use arete::interpreter::{
    self as vm,
    ast::{ResolveStrategy, ResolverExtractSpec, ResolverType},
    compiler::MultiEntityBytecode,
    scheduler::SlotScheduler,
    Mutation, ScheduledCallback,
};
use arete::runtime::{
    arete_server::{
        journal::{EventJournal, JournalConfig},
        snapshot::{SnapshotConfig, SnapshotService, SnapshotTrigger},
        BusManager, Delivery, EntityCache, Filters, Mode, MutationBatch, Projection, Projector,
        SlotContext, SlotTracker, Spec, ViewIndex, ViewSpec,
    },
    serde_json::{self, json, Value},
    tokio,
};
use std::sync::{atomic::AtomicU64, Arc, Mutex};
use std::time::Duration;

__runtime_helpers!();

const PARSER_SLOT: u64 = 100;
/// Parsed, but its batch not yet applied by the projector: the parser marks a
/// slot processed when it handles it, ahead of the resume watermark.
const PROCESSED_SLOT: u64 = 120;
/// Where the parser has got to by the time the fetch returns.
const PROCESSED_AFTER_FETCH: u64 = 130;
const TIP_SLOT: u64 = 500;

/// Stands in for the token metadata backend: the scheduled callback resolves
/// to a new price for the entity the parser already wrote. Only the fetch is
/// stubbed, so the real apply path runs. The parser keeps going while the
/// fetch is in flight.
struct Resolver {
    processed: SlotTracker,
}
impl vm::RuntimeResolver for Resolver {
    fn resolve_batch<'a>(
        &'a self,
        requests: &'a [vm::RuntimeResolverRequest],
    ) -> vm::ResolverBatchFuture<'a> {
        Box::pin(async move {
            self.processed.record(PROCESSED_AFTER_FETCH);
            Ok(requests
                .iter()
                .map(|request| (request.key().to_string(), json!({"price": 99})))
                .collect())
        })
    }
}

/// A resolver that applies on its own and never asks for the update context,
/// which the trait allows.
struct IgnoresContext {
    processed: SlotTracker,
}
impl vm::RuntimeResolver for IgnoresContext {
    fn resolve_batch<'a>(
        &'a self,
        _: &'a [vm::RuntimeResolverRequest],
    ) -> vm::ResolverBatchFuture<'a> {
        Box::pin(async { Ok(Default::default()) })
    }
    fn resolve_and_apply<'a>(
        &'a self,
        _: &'a Mutex<vm::vm::VmContext>,
        _: &'a MultiEntityBytecode,
        _: Vec<vm::ResolverRequest>,
        _: vm::ApplyContextFn<'a>,
    ) -> vm::ResolverApplyFuture<'a> {
        Box::pin(async move {
            self.processed.record(PROCESSED_AFTER_FETCH);
            vec![token("mint1", 99)]
        })
    }
}

fn token(id: &str, price: u64) -> Mutation {
    serde_json::from_value(json!({
        "export": "Token",
        "key": id,
        "patch": {"id": id, "price": price},
    }))
    .unwrap()
}

fn spec() -> Spec {
    use arete::interpreter::ast::{IdentitySpec, TypedStreamSpec};
    let entity = TypedStreamSpec::<Value>::new(
        "Token".to_string(),
        IdentitySpec {
            primary_keys: vec!["id".to_string()],
            lookup_indexes: Vec::new(),
        },
        Vec::new(),
    );
    let serializable = entity.to_serializable();
    let bytecode = MultiEntityBytecode::new()
        .add_entity("Token".to_string(), entity, 1)
        .build();
    Spec::new(bytecode, "Program111").with_entity_specs(vec![serializable])
}

fn view_index() -> ViewIndex {
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
    index
}

async fn service(
    config: &SnapshotConfig,
    spec: &Spec,
    cache: &EntityCache,
    views: &ViewIndex,
    tx: tokio::sync::mpsc::Sender<MutationBatch>,
) -> Arc<SnapshotService> {
    SnapshotService::initialize(
        config.clone(),
        spec,
        cache.clone(),
        views,
        Arc::new(EventJournal::new(JournalConfig::default())),
        tx,
    )
    .await
    .unwrap()
}

async fn run(ignores_context: bool) {
    let dir = std::env::temp_dir().join(format!(
        "arete-scheduler-watermark-{}-{ignores_context}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let config = SnapshotConfig {
        enabled: true,
        url: Some(dir.display().to_string()),
        interval: Duration::from_secs(3_600),
        ..SnapshotConfig::default()
    };
    let spec = spec();
    let views = view_index();
    let cache = EntityCache::new();
    let (tx, rx) = tokio::sync::mpsc::channel::<MutationBatch>(16);
    let snapshots = service(&config, &spec, &cache, &views, tx.clone()).await;
    tokio::spawn(
        Projector::new(
            Arc::new(views.clone()),
            BusManager::new(),
            cache.clone(),
            rx,
        )
        .with_snapshot_runtime(snapshots.runtime())
        .run(),
    );

    // Inputs of the generated scheduler task.
    let vm = Arc::new(Mutex::new(vm::vm::VmContext::new()));
    vm.lock()
        .unwrap()
        .get_state_table_mut(0)
        .unwrap()
        .insert_with_eviction(json!("mint1"), json!({"id": "mint1", "price": 10}));
    // A computed field that records the slot it was evaluated at, so the test
    // can see which slot the resolver result was applied under.
    let mut bytecode = MultiEntityBytecode::new()
        .add_entity_with_evaluator(
            "Token".to_string(),
            vm::ast::TypedStreamSpec::<Value>::new(
                "Token".to_string(),
                vm::ast::IdentitySpec {
                    primary_keys: vec!["id".to_string()],
                    lookup_indexes: Vec::new(),
                },
                Vec::new(),
            ),
            0,
            Some(|state: &mut Value, slot: Option<u64>, _: i64| {
                state["computed_slot"] = json!(slot);
                Ok(())
            }),
        )
        .build();
    bytecode.entities.get_mut("Token").unwrap().computed_paths = vec!["computed_slot".to_string()];
    let bytecode_arc = Arc::new(bytecode);
    let slot_tracker = SlotTracker::new();
    let processed_slot_tracker = SlotTracker::new();
    processed_slot_tracker.record(PROCESSED_SLOT);
    let processed = processed_slot_tracker.clone();
    let runtime_resolver: vm::SharedRuntimeResolver = if ignores_context {
        Arc::new(IgnoresContext { processed })
    } else {
        Arc::new(Resolver { processed })
    };
    let async_resolver_order = Arc::new(AtomicU64::new(0));
    let snapshot_barrier: Option<arete::runtime::arete_server::snapshot::SnapshotBarrier> = None;
    let slot_scheduler = Arc::new(Mutex::new(SlotScheduler::new()));
    slot_scheduler.lock().unwrap().register(
        TIP_SLOT,
        ScheduledCallback {
            state_id: 0,
            entity_name: "Token".to_string(),
            primary_key: json!("mint1"),
            resolver: ResolverType::Token,
            url_template: None,
            input_value: Some(json!("mint1")),
            input_path: None,
            condition: None,
            strategy: ResolveStrategy::LastWrite,
            extracts: vec![ResolverExtractSpec {
                target_path: "price".to_string(),
                source_path: Some("price".to_string()),
                transform: None,
            }],
            retry_count: 0,
        },
    );
    snapshots
        .runtime()
        .register_runtime(vm.clone(), slot_tracker.clone());

    // The parser has applied slot 100.
    tx.send(MutationBatch::with_slot_context(
        vec![token("mint1", 10)].into_iter().collect(),
        SlotContext::new(PARSER_SLOT, 1),
    ))
    .await
    .unwrap();

    // Captured so the test knows when the scheduler has fired.
    let (scheduler_tx, mut scheduler_rx) = tokio::sync::mpsc::channel::<MutationBatch>(16);
    {
        let mutations_tx = scheduler_tx;
        __scheduler_task!();
    }
    // The slot subscription observes the tip, far ahead of the parser. The
    // task also polls every 5s, so a missed notification only slows this.
    tokio::time::sleep(Duration::from_millis(50)).await;
    slot_tracker.record(TIP_SLOT);

    let batch = tokio::time::timeout(Duration::from_secs(15), scheduler_rx.recv())
        .await
        .expect("scheduler fired")
        .unwrap();
    tx.send(batch).await.unwrap();

    let (ack_tx, ack_rx) = tokio::sync::oneshot::channel();
    tx.send(MutationBatch::flush_marker(ack_tx)).await.unwrap();
    ack_rx.await.unwrap();

    // Clients order and stale-check by `_seq`, and read the processed slot from
    // it: the scheduler's write must claim the parser's position, not the tip,
    // and the position when it is sent, not a stale one from before the fetch.
    // Parser batches up to that position are already ahead of it in the queue,
    // so an older stamp would make clients drop this write as stale.
    let live = cache.get_all("Token/list").await;
    let seq = live[0].1["_seq"].as_str().expect("_seq").to_string();
    assert_eq!(
        seq.split(':').next(),
        Some(PROCESSED_AFTER_FETCH.to_string().as_str()),
        "scheduler write stamped {seq}"
    );
    assert_eq!(live[0].1["price"], 99);
    // ...and the state it publishes was derived at that same slot. A resolver
    // that never asks for the context derives nothing from it.
    if !ignores_context {
        assert_eq!(
            live[0].1["computed_slot"],
            json!(PROCESSED_AFTER_FETCH),
            "resolver result applied under a different slot than its stamp {seq}"
        );
    }
    assert!(snapshots
        .snapshot_now(SnapshotTrigger::Shutdown)
        .await
        .unwrap());

    // --- Restart ---
    let restored_cache = EntityCache::new();
    let restored_views = view_index();
    let (restored_tx, _restored_rx) = tokio::sync::mpsc::channel::<MutationBatch>(16);
    let restored = service(
        &config,
        &spec,
        &restored_cache,
        &restored_views,
        restored_tx,
    )
    .await;

    // The scheduler's update was applied and survives...
    let entities = restored_cache.get_all("Token/list").await;
    assert_eq!(entities.len(), 1);
    assert_eq!(entities[0].1["price"], 99);
    // ...but the stream resumes where the parser stopped, not at the tip.
    let resume = restored.runtime().take_restored().unwrap().resume_watermark;
    assert_eq!(
        resume,
        Some(PARSER_SLOT),
        "resumed past the slot the projector applied"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

fn main() {
    for ignores_context in [false, true] {
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(run(ignores_context));
    }
}
