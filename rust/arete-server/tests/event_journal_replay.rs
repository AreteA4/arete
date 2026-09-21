//! An append view driven through the real projector retains every event and
//! replays it from a cursor, past the point the entity cache stops being able
//! to answer.

use arete_interpreter::Mutation;
use arete_server::journal::{EventJournal, JournalConfig, ReplayError, ReplayWindow};
use arete_server::{
    BusManager, Delivery, EntityCache, EntityCacheConfig, Filters, Mode, MutationBatch, Projection,
    Projector, SlotContext, ViewIndex, ViewSpec,
};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

/// Well below the event count, so the cache cannot be what serves the replay.
const CACHE_CAP: usize = 50;
const EVENTS: u64 = 1_200;

fn view_index() -> ViewIndex {
    let mut index = ViewIndex::new();
    index.add_spec(ViewSpec {
        id: "Trade/append".to_string(),
        export: "Trade".to_string(),
        mode: Mode::Append,
        wire_format: Default::default(),
        projection: Projection::all(),
        filters: Filters::all(),
        delivery: Delivery::default(),
        pipeline: None,
        source_view: None,
    });
    index
}

/// Several trades share one transaction, which is the case a `_seq`-based
/// cursor cannot express: `slot_index` is the transaction index, so every
/// event here carries an identical `_seq`.
fn trade_batch(index: u64) -> MutationBatch {
    let slot = 100 + index / 3;
    let txn_index = index % 3;
    let mutation = Mutation {
        export: "Trade".to_string(),
        key: json!(format!("pool{}", index % 7)),
        patch: json!({"trade": index, "amount": index * 10}),
        append: vec![],
    };
    MutationBatch::with_slot_context(
        vec![mutation].into_iter().collect(),
        SlotContext::new(slot, txn_index),
    )
}

async fn drain(tx: &mpsc::Sender<MutationBatch>) {
    let (ack_tx, ack_rx) = tokio::sync::oneshot::channel();
    tx.send(MutationBatch::flush_marker(ack_tx)).await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), ack_rx)
        .await
        .expect("projector should ack the flush marker")
        .unwrap();
}

fn journal_for(max_records: usize) -> Arc<EventJournal> {
    Arc::new(EventJournal::new(JournalConfig {
        enabled: true,
        max_records_per_view: max_records,
        max_age: Duration::from_secs(3_600),
    }))
}

async fn run_projector(
    journal: Arc<EventJournal>,
    events: u64,
) -> (EntityCache, mpsc::Sender<MutationBatch>, tokio::task::JoinHandle<()>) {
    let entity_cache = EntityCache::with_config(EntityCacheConfig {
        max_entities_per_view: CACHE_CAP,
        ..Default::default()
    });
    let (tx, rx) = mpsc::channel::<MutationBatch>(256);
    let projector = Projector::new(
        Arc::new(view_index()),
        BusManager::new(),
        entity_cache.clone(),
        rx,
        #[cfg(feature = "otel")]
        None,
    )
    .with_journal(journal);
    let handle = tokio::spawn(projector.run());

    for index in 0..events {
        tx.send(trade_batch(index)).await.unwrap();
    }
    drain(&tx).await;
    (entity_cache, tx, handle)
}

#[tokio::test]
async fn every_event_after_a_cursor_replays_in_order_exactly_once() {
    let journal = journal_for(10_000);
    let (entity_cache, tx, handle) = run_projector(journal.clone(), EVENTS).await;

    // The cache cannot answer this: it holds at most one row per key, folded.
    let cached = entity_cache.get_all("Trade/append").await;
    assert!(
        cached.len() <= 7,
        "the cache keeps only the latest state per key, got {} rows",
        cached.len()
    );

    let window = journal.window("Trade/append").await;
    assert_eq!(
        window,
        ReplayWindow {
            earliest: 0,
            next: EVENTS
        }
    );

    // Resume from well past the cache's capacity.
    let replayed = journal
        .replay_after("Trade/append", Some(499))
        .await
        .expect("cursor is inside the retained window");
    assert_eq!(replayed.len() as u64, EVENTS - 500);

    let offsets: Vec<u64> = replayed.iter().map(|record| record.offset).collect();
    let expected: Vec<u64> = (500..EVENTS).collect();
    assert_eq!(offsets, expected, "in order, no gaps, no repeats");

    // Each retained payload is the frame that was published, carrying the same
    // offset a live subscriber would have checkpointed.
    let first: Value = serde_json::from_slice(&replayed[0].payload).unwrap();
    assert_eq!(first["offset"], json!(500));
    assert_eq!(first["entity"], json!("Trade/append"));
    assert_eq!(first["data"]["trade"], json!(500));

    drop(tx);
    handle.await.unwrap();
}

#[tokio::test]
async fn events_sharing_one_transaction_get_distinct_cursors() {
    let journal = journal_for(10_000);
    let (_cache, tx, handle) = run_projector(journal.clone(), 3).await;

    let records = journal
        .replay_after("Trade/append", None)
        .await
        .expect("no cursor replays the window");
    assert_eq!(records.len(), 3);

    // All three were produced under slot 100 and would collapse onto the same
    // `_seq`-derived position; their journal offsets still separate them.
    let seqs: Vec<String> = records
        .iter()
        .map(|record| {
            let frame: Value = serde_json::from_slice(&record.payload).unwrap();
            frame["seq"].as_str().unwrap().to_string()
        })
        .collect();
    assert_eq!(seqs[0], "100:000000000000");
    assert_eq!(
        records.iter().map(|r| r.offset).collect::<Vec<_>>(),
        [0, 1, 2],
        "offsets distinguish events a shared seq cannot"
    );

    drop(tx);
    handle.await.unwrap();
}

#[tokio::test]
async fn a_cursor_evicted_by_retention_is_reported_with_the_window() {
    // Retain far fewer records than are published.
    let journal = journal_for(100);
    let (_cache, tx, handle) = run_projector(journal.clone(), EVENTS).await;

    let window = journal.window("Trade/append").await;
    assert_eq!(
        window,
        ReplayWindow {
            earliest: EVENTS - 100,
            next: EVENTS
        },
        "retention trims the oldest records without rewinding offsets"
    );

    let error = journal
        .replay_after("Trade/append", Some(10))
        .await
        .expect_err("a cursor below the window cannot be served");
    assert_eq!(error, ReplayError::CursorExpired(window));

    // A consumer that restarts at the advertised earliest cursor succeeds.
    let recovered = journal
        .replay_after("Trade/append", Some(window.earliest))
        .await
        .expect("the advertised cursor is serviceable");
    assert_eq!(recovered.len(), 99);

    drop(tx);
    handle.await.unwrap();
}

#[tokio::test]
async fn a_disabled_journal_retains_nothing() {
    let journal = Arc::new(EventJournal::new(JournalConfig::default()));
    assert!(!journal.is_enabled());
    let (_cache, tx, handle) = run_projector(journal.clone(), 10).await;

    assert!(journal.window("Trade/append").await.is_empty());
    assert!(journal
        .replay_after("Trade/append", None)
        .await
        .unwrap()
        .is_empty());

    drop(tx);
    handle.await.unwrap();
}
