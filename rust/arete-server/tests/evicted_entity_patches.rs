//! A source patch for a key the entity cache has evicted is not an entity.
//!
//! The VM emits only the fields a handler changed. While the cache holds the
//! key that patch merges into the full entity; once the bounded cache has
//! evicted it, the patch alone would be stored — and served, and fed to
//! derived views — as though it were the whole entity. The cache refuses it,
//! and the projector asks the VM for the whole entity, which comes back with
//! the key's next mutation.
//!
//! A linked VM marks each entity's creation, so the cache needs no memory of
//! what it evicted to tell the two apart: a key it lacks is stored only from
//! its creation, however many entities the VM keeps.

use arete_interpreter::ast::{
    FieldPath, IdentitySpec, KeyResolutionStrategy, MappingSource, PopulationStrategy,
    SerializableFieldMapping, SerializableHandlerSpec, SerializableStreamSpec, SourceSpec,
    TypedStreamSpec,
};
use arete_interpreter::compiler::MultiEntityBytecode;
use arete_interpreter::vm::VmContext;
use arete_interpreter::Mutation;
use arete_server::materialized_view::{SortConfig, SortOrder, ViewPipeline};
use arete_server::{
    BusManager, Delivery, EntityCache, EntityCacheConfig, EntityResync, Filters, Mode,
    MutationBatch, Projection, Projector, SlotContext, ViewIndex, ViewSpec,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;
use tokio::sync::mpsc;

const CAP: usize = 3;

fn view(id: &str, pipeline: Option<ViewPipeline>, source: Option<&str>) -> ViewSpec {
    ViewSpec {
        id: id.to_string(),
        export: id.split('/').next().unwrap().to_string(),
        mode: Mode::List,
        wire_format: Default::default(),
        projection: Projection::all(),
        filters: Filters::all(),
        delivery: Delivery::default(),
        pipeline,
        source_view: source.map(str::to_string),
    }
}

fn sorted_by(field: &[&str], order: SortOrder) -> Option<ViewPipeline> {
    Some(ViewPipeline {
        filter: None,
        sort: Some(SortConfig {
            field_path: field.iter().map(|segment| segment.to_string()).collect(),
            order,
        }),
        limit: None,
    })
}

/// `Round/list`, plus `Round/latest` (newest round first) and `Round/busiest`
/// (most checkpoints first) derived from it.
fn view_index() -> ViewIndex {
    let mut index = ViewIndex::new();
    index.add_spec(view("Round/list", None, None));
    index.add_spec(view(
        "Round/latest",
        sorted_by(&["id", "round_id"], SortOrder::Desc),
        Some("Round/list"),
    ));
    index.add_spec(view(
        "Round/busiest",
        sorted_by(&["metrics", "checkpoint_count"], SortOrder::Desc),
        Some("Round/list"),
    ));
    index
}

fn round_mutation(key: u64, patch: Value) -> Mutation {
    Mutation {
        export: "Round".to_string(),
        key: json!(key),
        patch,
        append: vec![],
    }
}

fn batch(mutations: Vec<Mutation>, slot: u64) -> MutationBatch {
    MutationBatch::with_slot_context(mutations.into_iter().collect(), SlotContext::new(slot, 0))
}

fn full_round(round_id: u64, checkpoints: u64) -> Value {
    json!({
        "id": {"round_id": round_id},
        "state": {"expires_at": round_id * 10},
        "metrics": {"checkpoint_count": checkpoints},
    })
}

struct Harness {
    index: ViewIndex,
    cache: EntityCache,
    tx: mpsc::Sender<MutationBatch>,
    slot: u64,
}

impl Harness {
    fn start() -> Self {
        Self::start_with(view_index(), EntityResync::new())
    }

    fn start_with(index: ViewIndex, resync: EntityResync) -> Self {
        let cache = EntityCache::with_config(EntityCacheConfig {
            max_entities_per_view: CAP,
            ..EntityCacheConfig::default()
        });
        let (tx, rx) = mpsc::channel::<MutationBatch>(64);
        tokio::spawn(
            Projector::new(
                Arc::new(index.clone()),
                BusManager::new(),
                cache.clone(),
                rx,
                #[cfg(feature = "otel")]
                None,
            )
            .with_entity_resync(resync)
            .run(),
        );
        Self {
            index,
            cache,
            tx,
            slot: 1,
        }
    }

    async fn send(&mut self, key: u64, patch: Value) {
        self.send_batch(vec![round_mutation(key, patch)]).await;
    }

    async fn send_batch(&mut self, mutations: Vec<Mutation>) {
        self.slot += 1;
        self.tx.send(batch(mutations, self.slot)).await.unwrap();
    }

    async fn flush(&self) {
        let (ack_tx, ack_rx) = tokio::sync::oneshot::channel();
        self.tx
            .send(MutationBatch::flush_marker(ack_tx))
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), ack_rx)
            .await
            .expect("projector should ack the flush marker")
            .unwrap();
    }

    /// The cached entity, without the `_seq` the projector stamps on it.
    async fn cached(&self, key: &str) -> Option<Value> {
        self.cached_in("Round/list", key).await
    }

    async fn cached_in(&self, view: &str, key: &str) -> Option<Value> {
        let mut entity = self.cache.get(view, key).await?;
        entity.as_object_mut()?.remove("_seq");
        Some(entity)
    }

    async fn window(&self, view: &str) -> Vec<(String, Value)> {
        let caches = self.index.sorted_caches();
        let mut caches = caches.write().await;
        caches
            .get_mut(view)
            .expect("sorted cache exists")
            .get_window(0, usize::MAX)
    }
}

/// The live `OreRound/latest` failure: checkpoint-only updates to old rounds
/// the cache had evicted became entities with no `id`, and a missing sort key
/// ranked above every real round.
#[tokio::test]
async fn a_patch_for_an_evicted_key_neither_stores_nor_ranks_a_partial_entity() {
    let mut harness = Harness::start();
    for round in 1..=5 {
        harness.send(round, full_round(round, 0)).await;
    }
    // Round 1 was evicted long ago; this carries only what changed.
    harness
        .send(1, json!({"metrics": {"checkpoint_count": 7}}))
        .await;
    harness.flush().await;

    assert_eq!(
        harness.cached("1").await,
        None,
        "the cache must not hold a checkpoint count as round 1"
    );
    let latest = harness.window("Round/latest").await;
    let keys: Vec<&str> = latest.iter().map(|(key, _)| key.as_str()).collect();
    assert_eq!(keys, ["5", "4", "3"]);
    assert!(latest
        .iter()
        .all(|(_, entity)| entity["id"]["round_id"].is_u64() && entity["state"].is_object()));
}

/// A key the derived view still holds (it ranks, though the recency-bounded
/// entity cache evicted it) keeps receiving its changes, merged into the full
/// copy the derived view already has.
#[tokio::test]
async fn a_derived_view_still_merges_patches_for_a_key_it_holds() {
    let mut harness = Harness::start();
    harness.send(1, full_round(1, 50)).await;
    for round in 2..=6 {
        harness.send(round, full_round(round, 0)).await;
    }
    harness
        .send(1, json!({"metrics": {"checkpoint_count": 51}}))
        .await;
    harness.flush().await;

    let busiest = harness.window("Round/busiest").await;
    let (key, entity) = &busiest[0];
    assert_eq!(key, "1");
    assert_eq!(entity["metrics"]["checkpoint_count"], 51);
    assert_eq!(entity["id"]["round_id"], 1, "still the whole entity");
    assert_eq!(entity["state"]["expires_at"], 10);
}

/// A key the cache never held is a new entity: its first patch is all of it.
#[tokio::test]
async fn a_new_key_is_still_created_from_its_first_patch() {
    let mut harness = Harness::start();
    harness.send(9, full_round(9, 0)).await;
    harness.flush().await;
    assert_eq!(harness.cached("9").await, Some(full_round(9, 0)));
}

/// A source delete ends the entity; a later patch recreates it from scratch.
#[tokio::test]
async fn a_deleted_key_may_be_created_again() {
    let mut harness = Harness::start();
    for round in 1..=5 {
        harness.send(round, full_round(round, 0)).await;
    }
    harness.flush().await;
    harness.cache.remove("Round/list", "1").await;
    harness.send(1, full_round(1, 3)).await;
    harness.flush().await;
    assert_eq!(harness.cached("1").await, Some(full_round(1, 3)));
}

/// A VM-less resync: linked to a VM that never processes anything, so the
/// test can see what the projector asks for and answer it by hand.
fn linked_resync() -> EntityResync {
    let resync = EntityResync::new();
    resync.link(&Arc::new(StdMutex::new(VmContext::new())));
    resync
}

fn whole_round(key: u64, round: Value) -> Mutation {
    let mut mutation = round_mutation(key, round);
    mutation.mark_whole_entity();
    mutation
}

/// A round's first mutation, marked as its creation the way a linked VM
/// marks it.
fn created_round(key: u64, round: Value) -> Mutation {
    let mut mutation = round_mutation(key, round);
    mutation.mark_created();
    mutation
}

/// A refused patch asks the VM for the whole entity, and the whole entity it
/// sends back makes the key a cached, ranked entity again.
#[tokio::test]
async fn an_evicted_key_is_requested_whole_and_comes_back() {
    let resync = linked_resync();
    let mut harness = Harness::start_with(view_index(), resync.clone());
    for round in 1..=5 {
        harness
            .send_batch(vec![created_round(round, full_round(round, 0))])
            .await;
    }
    harness
        .send(1, json!({"metrics": {"checkpoint_count": 7}}))
        .await;
    harness.flush().await;
    assert!(resync.requests().is_requested("Round", &json!(1)));
    assert_eq!(harness.cached("1").await, None);

    // The VM's answer: the change, then the whole entity after it.
    harness
        .send_batch(vec![
            round_mutation(1, json!({"metrics": {"checkpoint_count": 8}})),
            whole_round(1, full_round(1, 8)),
        ])
        .await;
    harness.flush().await;
    assert_eq!(harness.cached("1").await, Some(full_round(1, 8)));
    let busiest = harness.window("Round/busiest").await;
    assert_eq!(busiest[0].0, "1", "ranked again, as the whole entity");
    assert_eq!(busiest[0].1["id"]["round_id"], 1);
    // The marker is the VM's to the projector, never part of the entity.
    let raw = harness.cache.get("Round/list", "1").await.unwrap();
    assert!(raw.get(arete_interpreter::WHOLE_ENTITY_MARKER).is_none());

    // It is held again: its patches merge.
    harness
        .send(1, json!({"metrics": {"checkpoint_count": 9}}))
        .await;
    harness.flush().await;
    assert_eq!(harness.cached("1").await, Some(full_round(1, 9)));
}

/// Without a VM there is no one to ask: the key stays out rather than coming
/// back as a fragment. A source that sends whole entities can say so.
#[tokio::test]
async fn without_a_vm_a_mutation_marked_whole_restores_the_key() {
    let resync = EntityResync::new();
    let mut harness = Harness::start_with(view_index(), resync.clone());
    for round in 1..=5 {
        harness.send(round, full_round(round, 0)).await;
    }
    harness
        .send(1, json!({"metrics": {"checkpoint_count": 7}}))
        .await;
    harness.flush().await;
    assert!(resync.requests().is_empty(), "nothing is linked to ask");
    assert_eq!(harness.cached("1").await, None);

    harness
        .send_batch(vec![whole_round(1, full_round(1, 7))])
        .await;
    harness.flush().await;
    assert_eq!(harness.cached("1").await, Some(full_round(1, 7)));
}

fn pool_mapping(
    target: &str,
    source: &str,
    population: PopulationStrategy,
) -> SerializableFieldMapping {
    SerializableFieldMapping {
        target_path: target.to_string(),
        source: MappingSource::FromSource {
            path: FieldPath::new(&[source]),
            default: None,
            transform: None,
        },
        transform: None,
        population,
        condition: None,
        when: None,
        stop: None,
        emit: true,
    }
}

/// `Pool`, keyed by its account address: a name set once, a price, and a
/// fill history appended to on every update.
fn pool_bytecode() -> MultiEntityBytecode {
    let spec = TypedStreamSpec::<Value>::from_serializable(SerializableStreamSpec {
        ast_version: arete_interpreter::ast::CURRENT_AST_VERSION.to_string(),
        state_name: "Pool".to_string(),
        program_id: None,
        idl: None,
        identity: IdentitySpec {
            primary_keys: vec!["id.address".to_string()],
            lookup_indexes: vec![],
        },
        handlers: vec![SerializableHandlerSpec {
            source: SourceSpec::Source {
                program_id: None,
                discriminator: None,
                type_name: "amm::PoolState".to_string(),
                serialization: None,
                is_account: true,
            },
            key_resolution: KeyResolutionStrategy::Embedded {
                primary_field: FieldPath::new(&["__account_address"]),
            },
            mappings: vec![
                pool_mapping(
                    "id.address",
                    "__account_address",
                    PopulationStrategy::SetOnce,
                ),
                pool_mapping("id.name", "name", PopulationStrategy::SetOnce),
                pool_mapping("state.price", "price", PopulationStrategy::LastWrite),
                pool_mapping("state.fills", "fill", PopulationStrategy::Append),
            ],
            conditions: vec![],
            emit: true,
        }],
        sections: vec![],
        field_mappings: BTreeMap::new(),
        resolver_hooks: vec![],
        instruction_hooks: vec![],
        resolver_specs: vec![],
        computed_fields: vec![],
        computed_field_specs: vec![],
        content_hash: None,
        views: vec![],
    });
    MultiEntityBytecode::from_single("Pool".to_string(), spec, 0)
}

/// A VM running `pool_bytecode`, linked to a projector's resync.
struct PoolVm {
    vm: Arc<StdMutex<VmContext>>,
    bytecode: MultiEntityBytecode,
}

impl PoolVm {
    fn linked_to(resync: &EntityResync) -> Self {
        Self::link(VmContext::new(), resync)
    }

    /// A VM restored from `other`'s state, as the generated runtime restores
    /// one from a snapshot.
    fn restored_from(other: &PoolVm, resync: &EntityResync) -> Self {
        let mut vm = VmContext::new();
        vm.hydrate(other.vm.lock().unwrap().dump());
        Self::link(vm, resync)
    }

    fn link(vm: VmContext, resync: &EntityResync) -> Self {
        let vm = Arc::new(StdMutex::new(vm));
        resync.link(&vm);
        Self {
            vm,
            bytecode: pool_bytecode(),
        }
    }

    fn update(&self, address: &str, price: u64) -> Vec<Mutation> {
        let event = json!({
            "__account_address": address,
            "name": format!("pool {address}"),
            "price": price,
            "fill": price,
        });
        self.vm
            .lock()
            .unwrap()
            .process_event(&self.bytecode, event, "amm::PoolState", None, None)
            .unwrap()
    }

    fn holds(&self, address: &str) -> bool {
        self.vm
            .lock()
            .unwrap()
            .get_entity_state(0, &json!(address))
            .is_some()
    }
}

/// The whole loop with a real VM: the cache evicts a pool, refuses its next
/// change, and the VM's next mutations for it bring the entity back whole —
/// including the name set once at creation, which no later patch carries.
#[tokio::test]
async fn the_vm_sends_back_an_entity_the_cache_evicted() {
    let mut index = ViewIndex::new();
    index.add_spec(view("Pool/list", None, None));
    let resync = EntityResync::new();
    let pools = PoolVm::linked_to(&resync);
    let mut harness = Harness::start_with(index, resync.clone());

    for pool in ["a", "b", "c", "d", "e"] {
        harness.send_batch(pools.update(pool, 1)).await;
    }
    harness.flush().await;
    assert_eq!(harness.cached_in("Pool/list", "a").await, None, "evicted");

    // Its next change is only the change: refused, and requested whole.
    let mutations = pools.update("a", 2);
    assert_eq!(mutations.len(), 1);
    harness.send_batch(mutations).await;
    harness.flush().await;
    assert_eq!(harness.cached_in("Pool/list", "a").await, None);
    assert!(resync.requests().is_requested("Pool", &json!("a")));

    // The one after carries the whole entity as well. Its own change, ahead
    // of the whole entity in the same batch, is refused without asking again.
    let mutations = pools.update("a", 3);
    assert_eq!(mutations.len(), 2);
    harness.send_batch(mutations).await;
    harness.flush().await;
    assert_eq!(
        harness.cached_in("Pool/list", "a").await,
        Some(json!({
            "id": {"address": "a", "name": "pool a"},
            "state": {"price": 3, "fills": [1, 2, 3]},
        }))
    );
    assert!(resync.requests().is_empty());
}

/// The cache's memory of evicted keys is bounded — eight times its own bound —
/// and the VM's table is not tied to it: here the VM keeps every pool while
/// the cache holds three. A pool evicted so long ago that such a memory would
/// have forgotten it still never becomes the fragment its next change
/// carries, in the cache or in a derived view: the VM marks creations, so any
/// other patch for a key the cache lacks is refused, and the whole entity
/// follows with the next change.
#[tokio::test]
async fn a_key_evicted_past_any_eviction_memory_is_never_stored_partial() {
    let mut index = ViewIndex::new();
    index.add_spec(view("Pool/list", None, None));
    index.add_spec(view(
        "Pool/priciest",
        sorted_by(&["state", "price"], SortOrder::Desc),
        Some("Pool/list"),
    ));
    let resync = EntityResync::new();
    let pools = PoolVm::linked_to(&resync);
    let mut harness = Harness::start_with(index, resync.clone());

    let count = CAP * 8 * 2;
    for n in 0..count {
        let address = format!("p{n}");
        harness.send_batch(pools.update(&address, n as u64)).await;
        harness.flush().await;
        assert!(
            harness.cached_in("Pool/list", &address).await.is_some(),
            "a pool is cached the moment it is created"
        );
    }
    assert!(pools.holds("p0"), "the VM still holds the first pool");
    assert_eq!(harness.cached_in("Pool/list", "p0").await, None);

    // Its next change carries only what changed; it is not the entity.
    let change = pools.update("p0", 1_000);
    assert_eq!(change.len(), 1);
    assert!(!change[0].is_whole_entity());
    harness.send_batch(change).await;
    harness.flush().await;
    assert_eq!(harness.cached_in("Pool/list", "p0").await, None);
    let priciest = harness.window("Pool/priciest").await;
    assert!(
        priciest.iter().all(|(key, _)| key != "p0"),
        "the fragment ranks nowhere"
    );
    assert!(resync.requests().is_requested("Pool", &json!("p0")));

    // The next brings the whole entity, with the name only its creation set.
    harness.send_batch(pools.update("p0", 1_001)).await;
    harness.flush().await;
    assert_eq!(
        harness.cached_in("Pool/list", "p0").await,
        Some(json!({
            "id": {"address": "p0", "name": "pool p0"},
            "state": {"price": 1_001, "fills": [0, 1_000, 1_001]},
        }))
    );
    let priciest = harness.window("Pool/priciest").await;
    assert_eq!(priciest[0].0, "p0");
    assert_eq!(priciest[0].1["id"]["name"], "pool p0");
}

/// After a restore the VM holds entities the cache lacks — after a legacy
/// migration, every one of them. Their next mutations are changes, not
/// creations, so they are refused and the entities follow whole, however
/// many there are; an entity created after the restore is stored at once.
#[tokio::test]
async fn an_entity_restored_into_the_vm_alone_comes_back_whole() {
    let mut index = ViewIndex::new();
    index.add_spec(view("Pool/list", None, None));
    let before = PoolVm::linked_to(&EntityResync::new());
    for pool in ["a", "b"] {
        before.update(pool, 1);
    }
    let resync = EntityResync::new();
    let pools = PoolVm::restored_from(&before, &resync);
    let mut harness = Harness::start_with(index, resync.clone());

    harness.send_batch(pools.update("a", 2)).await;
    harness.flush().await;
    assert_eq!(harness.cached_in("Pool/list", "a").await, None);
    assert!(resync.requests().is_requested("Pool", &json!("a")));

    harness.send_batch(pools.update("a", 3)).await;
    harness.send_batch(pools.update("c", 1)).await;
    harness.flush().await;
    assert_eq!(
        harness.cached_in("Pool/list", "a").await,
        Some(json!({
            "id": {"address": "a", "name": "pool a"},
            "state": {"price": 3, "fills": [1, 2, 3]},
        }))
    );
    assert_eq!(
        harness.cached_in("Pool/list", "c").await,
        Some(json!({
            "id": {"address": "c", "name": "pool c"},
            "state": {"price": 1, "fills": [1]},
        }))
    );
}
