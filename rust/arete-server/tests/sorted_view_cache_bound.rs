//! Derived sorted view caches stay bounded by the entity cache's per-view cap
//! when driven through the real projector.

use arete_interpreter::Mutation;
use arete_server::materialized_view::{
    CompareOp, FilterConfig, SortConfig, SortOrder, ViewPipeline,
};
use arete_server::{
    BusManager, Delivery, EntityCache, EntityCacheConfig, Filters, Mode, MutationBatch, Projection,
    Projector, SlotContext, ViewIndex, ViewSpec,
};
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

const CAP: usize = 5;

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

#[tokio::test]
async fn projector_bounds_derived_sorted_cache_by_entity_cache_cap() {
    let view_index = make_view_index();
    let entity_cache = EntityCache::with_config(EntityCacheConfig {
        max_entities_per_view: CAP,
        ..EntityCacheConfig::default()
    });
    let (tx, rx) = mpsc::channel::<MutationBatch>(64);
    let projector = Projector::new(
        Arc::new(view_index.clone()),
        BusManager::new(),
        entity_cache.clone(),
        rx,
        #[cfg(feature = "otel")]
        None,
    );
    let handle = tokio::spawn(projector.run());

    // A stale leader followed by many more distinct keys than the cap.
    tx.send(token_batch("leader", 1_000_000, 1)).await.unwrap();
    for i in 0..(CAP as u64 * 20) {
        let price = (i * 7919) % 1_000;
        tx.send(token_batch(&format!("mint{i}"), price, i + 2))
            .await
            .unwrap();
    }

    let (ack_tx, ack_rx) = tokio::sync::oneshot::channel();
    tx.send(MutationBatch::flush_marker(ack_tx)).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), ack_rx)
        .await
        .expect("projector should ack the flush marker")
        .unwrap();

    let sorted_caches = view_index.sorted_caches();
    let mut caches = sorted_caches.write().await;
    let cache = caches.get_mut("Token/top").expect("sorted cache exists");
    assert_eq!(cache.len(), CAP);

    let window = cache.get_window(0, CAP);
    assert_eq!(window[0].0, "leader", "stale top entity is not evicted");
    let prices: Vec<u64> = window
        .iter()
        .map(|(_, entity)| entity["price"].as_u64().unwrap())
        .collect();
    assert!(prices.windows(2).all(|pair| pair[0] >= pair[1]));
    // The window beneath the leader is the true top of everything upserted.
    let mut expected: Vec<u64> = (0..(CAP as u64 * 20)).map(|i| (i * 7919) % 1_000).collect();
    expected.sort_unstable_by(|a, b| b.cmp(a));
    assert_eq!(&prices[1..], &expected[..CAP - 1]);

    drop(caches);
    drop(tx);
    handle.await.unwrap();
}

#[tokio::test]
async fn a_filtered_derived_view_holds_only_entities_that_pass_it() {
    let mut view_index = make_view_index();
    view_index.add_spec(ViewSpec {
        id: "Token/priced".to_string(),
        export: "Token".to_string(),
        mode: Mode::List,
        wire_format: Default::default(),
        projection: Projection::all(),
        filters: Filters::all(),
        delivery: Delivery::default(),
        pipeline: Some(ViewPipeline {
            filter: Some(FilterConfig {
                field_path: vec!["price".to_string()],
                op: CompareOp::Gt,
                value: json!(0),
            }),
            sort: Some(SortConfig {
                field_path: vec!["price".to_string()],
                order: SortOrder::Desc,
            }),
            limit: None,
        }),
        source_view: Some("Token/list".to_string()),
    });
    let entity_cache = EntityCache::with_config(EntityCacheConfig {
        max_entities_per_view: CAP,
        ..EntityCacheConfig::default()
    });
    let (tx, rx) = mpsc::channel::<MutationBatch>(64);
    let projector = Projector::new(
        Arc::new(view_index.clone()),
        BusManager::new(),
        entity_cache.clone(),
        rx,
        #[cfg(feature = "otel")]
        None,
    );
    let handle = tokio::spawn(projector.run());

    tx.send(token_batch("free", 0, 1)).await.unwrap();
    tx.send(token_batch("cheap", 5, 2)).await.unwrap();
    tx.send(token_batch("dear", 9, 3)).await.unwrap();
    // Stops passing: leaves the filtered view, stays in the unfiltered one.
    tx.send(token_batch("cheap", 0, 4)).await.unwrap();

    let (ack_tx, ack_rx) = tokio::sync::oneshot::channel();
    tx.send(MutationBatch::flush_marker(ack_tx)).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), ack_rx)
        .await
        .expect("projector should ack the flush marker")
        .unwrap();

    let sorted_caches = view_index.sorted_caches();
    let mut caches = sorted_caches.write().await;
    let keys =
        |cache: &mut arete_server::sorted_cache::SortedViewCache| cache.ordered_keys().to_vec();
    assert_eq!(keys(caches.get_mut("Token/priced").unwrap()), ["dear"]);
    assert_eq!(
        keys(caches.get_mut("Token/top").unwrap()),
        ["dear", "cheap", "free"]
    );

    drop(caches);
    drop(tx);
    handle.await.unwrap();
}
