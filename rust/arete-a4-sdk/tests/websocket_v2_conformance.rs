use arete_a4_sdk::{
    parse_server_message, ClientMessage, EntityStream, GapCode, ServerMessage, SharedStore,
    SnapshotOptions, Subscription, SubscriptionQuery, Update,
};
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;
use tokio::time::timeout;

fn fixture(name: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/websocket-v2")
        .join(name);
    serde_json::from_str(&std::fs::read_to_string(path).expect("fixture should be readable"))
        .expect("fixture should be valid JSON")
}

fn subscription(value: &Value) -> Subscription {
    match serde_json::from_value::<ClientMessage>(value.clone()).expect("valid client fixture") {
        ClientMessage::Subscribe(subscription) => subscription,
        other => panic!("expected subscribe fixture, got {other:?}"),
    }
}

async fn register(store: &SharedStore, subscription: &Subscription) {
    store
        .register_subscription(
            &subscription.subscription_id,
            subscription.query.clone(),
            subscription.snapshot.enabled,
        )
        .await
        .expect("subscription should register");
}

async fn apply(store: &SharedStore, frame: &Value) {
    match parse_server_message(&serde_json::to_vec(frame).unwrap()).expect("valid server fixture") {
        ServerMessage::Frame(frame) => store.apply_frame(frame).await.expect("frame should apply"),
        ServerMessage::Error(_) => panic!("expected data frame"),
    }
}

fn ids(values: &[Value]) -> Vec<i64> {
    values
        .iter()
        .map(|value| value["id"].as_i64().expect("fixture id should be numeric"))
        .collect()
}

#[test]
fn manifest_and_client_fixtures_are_protocol_v2() {
    let manifest = fixture("manifest.json");
    assert_eq!(manifest["protocolVersion"], 2);
    for name in manifest["fixtures"].as_array().unwrap() {
        let fixture = fixture(name.as_str().unwrap());
        for client in fixture["client"].as_array().into_iter().flatten() {
            let message: ClientMessage =
                serde_json::from_value(client.clone()).expect("client fixture should deserialize");
            assert_eq!(
                serde_json::to_value(message).unwrap(),
                *client,
                "{name} must round-trip through typed client messages"
            );
        }
    }
}

#[test]
fn list_windows_and_filters_have_distinct_canonical_identities() {
    let windows = fixture("list-windows.json");
    let first = subscription(&windows["client"][0]);
    let second = subscription(&windows["client"][1]);
    assert_ne!(
        first.query.canonical_identity().unwrap(),
        second.query.canonical_identity().unwrap()
    );
    assert_eq!(first.query.take, Some(2));
    assert_eq!(second.query.skip, Some(2));

    let filters = fixture("filters.json");
    let filtered = subscription(&filters["client"][0]);
    assert_eq!(filtered.query.filters["state.status"], "open");
    assert_eq!(filtered.query.filters["market.symbol"], "SOL");
    assert_eq!(filtered.query.take, Some(10));
    assert_eq!(filtered.query.skip, Some(0));
}

#[tokio::test]
async fn keyed_snapshot_and_patch_use_exact_membership() {
    let fixture = fixture("keyed-state.json");
    let subscription = subscription(&fixture["client"][0]);
    let store = SharedStore::new();
    register(&store, &subscription).await;
    for frame in fixture["server"].as_array().unwrap() {
        apply(&store, frame).await;
    }

    let value: Value = store
        .get_for_subscription(&subscription.subscription_id, "wallet-a")
        .await
        .expect("key should remain in query membership");
    assert_eq!(value["score"], 2);
    assert!(store
        .get_for_subscription::<Value>(&subscription.subscription_id, "wallet-b")
        .await
        .is_none());
}

#[tokio::test]
async fn authoritative_batches_stage_then_replace_and_prune() {
    let fixture = fixture("multi-batch-authoritative.json");
    let store = SharedStore::new();
    let subscription = Subscription::new("things:all", SubscriptionQuery::new("Thing/list"));
    register(&store, &subscription).await;

    apply(
        &store,
        &json!({
            "protocolVersion": 2,
            "subscriptionId": "things:all",
            "op": "subscribed",
            "query": {"view": "Thing/list"},
            "mode": "list"
        }),
    )
    .await;
    apply(
        &store,
        &json!({
            "protocolVersion": 2,
            "subscriptionId": "things:all",
            "snapshotId": "old",
            "authoritative": true,
            "mode": "list",
            "entity": "Thing/list",
            "op": "snapshot",
            "data": [{"key": "old", "data": {"id": 0}}],
            "complete": true
        }),
    )
    .await;
    assert_eq!(
        ids(&store.list_for_subscription("things:all").await),
        vec![0]
    );

    store.begin_refresh("things:all").await;
    apply(&store, &fixture["server"][0]).await;
    assert_eq!(
        ids(&store.list_for_subscription("things:all").await),
        vec![0],
        "incomplete replacement must keep serving old data"
    );
    apply(&store, &fixture["server"][1]).await;
    assert_eq!(
        ids(&store.list_for_subscription("things:all").await),
        vec![3, 2, 1]
    );
    assert!(!store.all_raw("Thing/list").await.contains_key("old"));
}

#[tokio::test]
async fn empty_authoritative_snapshot_clears_only_its_query() {
    let fixture = fixture("empty-snapshot.json");
    let store = SharedStore::new();
    let subscription = Subscription::new(
        "state:missing",
        SubscriptionQuery::new("Thing/state").with_key("missing"),
    );
    register(&store, &subscription).await;
    apply(&store, &fixture["server"][0]).await;
    assert!(store
        .list_for_subscription::<Value>("state:missing")
        .await
        .is_empty());
    assert!(
        store
            .wait_for_subscription_ready("state:missing", std::time::Duration::from_millis(10))
            .await
    );
}

#[tokio::test]
async fn incremental_snapshot_merges_without_pruning() {
    let fixture = fixture("incremental-snapshot.json");
    let subscription = subscription(&fixture["client"][0]);
    let store = SharedStore::new();
    register(&store, &subscription).await;
    apply(
        &store,
        &json!({
            "protocolVersion": 2,
            "subscriptionId": "orders:resume",
            "snapshotId": "existing",
            "authoritative": true,
            "mode": "list",
            "entity": "Order/list",
            "op": "snapshot",
            "data": [{"key": "order-10", "data": {"_seq": "40:000000000010"}}],
            "complete": true
        }),
    )
    .await;
    apply(&store, &fixture["server"][0]).await;
    let rows: Vec<Value> = store.list_for_subscription("orders:resume").await;
    assert_eq!(rows.len(), 3);
    assert!(rows.iter().any(|row| row["_seq"] == "40:000000000010"));
}

#[tokio::test]
async fn remove_is_query_local_and_delete_is_source_global() {
    let remove = fixture("remove.json");
    let delete = fixture("delete.json");
    let store = SharedStore::new();
    let open = Subscription::new(
        "orders:open",
        SubscriptionQuery::new("Order/list").with_filter("state.status", "open"),
    );
    let all = Subscription::new("orders:all", SubscriptionQuery::new("Order/list"));
    register(&store, &open).await;
    register(&store, &all).await;
    for id in ["orders:open", "orders:all"] {
        apply(
            &store,
            &json!({
                "protocolVersion": 2,
                "subscriptionId": id,
                "snapshotId": format!("snapshot-{id}"),
                "authoritative": true,
                "mode": "list",
                "entity": "Order/list",
                "op": "snapshot",
                "data": [{"key": "order-7", "data": {"id": 7}}],
                "complete": true
            }),
        )
        .await;
    }
    assert_eq!(
        store.keys_for_subscription("orders:open").await,
        vec!["order-7"]
    );

    apply(&store, &remove["server"][0]).await;
    assert!(store
        .list_for_subscription::<Value>("orders:open")
        .await
        .is_empty());
    assert_eq!(
        store
            .list_for_subscription::<Value>("orders:all")
            .await
            .len(),
        1
    );
    assert_eq!(
        store.keys_for_subscription("orders:all").await,
        vec!["order-7"]
    );
    assert!(store.all_raw("Order/list").await.contains_key("order-7"));

    apply(&store, &delete["server"][0]).await;
    assert!(store
        .list_for_subscription::<Value>("orders:all")
        .await
        .is_empty());
    assert!(!store.all_raw("Order/list").await.contains_key("order-7"));
}

#[test]
fn reconnect_fixture_reuses_the_same_opaque_id_and_errors_stay_scoped() {
    let reconnect = fixture("reconnect-replacement.json");
    assert_eq!(
        reconnect["sessions"][0]["subscriptionId"],
        reconnect["sessions"][1]["subscriptionId"]
    );

    let errors = fixture("errors.json");
    for case in errors["cases"].as_array().unwrap() {
        let message = parse_server_message(&serde_json::to_vec(&case["response"]).unwrap())
            .expect("error fixture should parse");
        let ServerMessage::Error(error) = message else {
            panic!("expected protocol error")
        };
        assert_eq!(
            error.subscription_id.as_deref(),
            case["response"]["subscriptionId"].as_str()
        );
        assert_eq!(error.code, case["response"]["code"].as_str().unwrap());
        assert!(!error.fatal);
    }
}

#[test]
fn snapshot_options_are_not_part_of_query_identity() {
    let query = SubscriptionQuery::new("Thing/list").with_take(1);
    let first = Subscription::new("first", query.clone());
    let second = Subscription {
        snapshot: SnapshotOptions { enabled: false },
        ..Subscription::new("second", query)
    };
    assert_eq!(
        first.query.canonical_identity().unwrap(),
        second.query.canonical_identity().unwrap()
    );
}

const APPEND_VIEW: &str = "Trade/append";

/// A consumer of the append view, plus the store the fixture frames feed.
async fn append_consumer() -> (SharedStore, impl futures_util::Stream<Item = Update<Value>>) {
    let store = SharedStore::new();
    store
        .register_subscription("trades", SubscriptionQuery::new(APPEND_VIEW), true)
        .await
        .expect("subscription should register");
    let stream = Box::pin(EntityStream::<Value>::new(&store, APPEND_VIEW.to_string()));
    (store, stream)
}

async fn next_update(
    stream: &mut (impl futures_util::Stream<Item = Update<Value>> + Unpin),
) -> Option<Update<Value>> {
    timeout(Duration::from_secs(3), stream.next())
        .await
        .expect("the stream should not hang")
}

#[tokio::test]
async fn append_frames_deliver_one_cursor_each_even_when_they_share_a_seq() {
    let fixture = fixture("replay-cursors.json");
    let (store, mut stream) = append_consumer().await;
    for frame in fixture["server"].as_array().unwrap() {
        apply(&store, frame).await;
    }

    let expected: Vec<&str> = fixture["expected"]["cursors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|cursor| cursor.as_str().unwrap())
        .collect();
    let mut delivered = Vec::new();
    let mut payloads = Vec::new();
    for _ in 0..expected.len() {
        let update = next_update(&mut stream)
            .await
            .expect("every append frame should reach the consumer");
        delivered.push(
            update
                .cursor()
                .expect("an append update carries a cursor")
                .to_string(),
        );
        payloads.push(
            update
                .data()
                .cloned()
                .expect("an append update carries data"),
        );
    }
    assert_eq!(delivered, expected);

    // Each cursor must carry the event the server sent at that offset. The
    // stale-seq guard would re-emit the previous value here, checkpointing a
    // payload that was never published under a position that was.
    let server = fixture["server"].as_array().unwrap();
    let sent: Vec<&Value> = server[1..].iter().map(|frame| &frame["data"]).collect();
    assert_eq!(payloads.iter().collect::<Vec<_>>(), sent);

    // The first two frames share one `seq` — its second component is the
    // transaction index — so only the offset tells them apart.
    let server = fixture["server"].as_array().unwrap();
    assert_eq!(server[1]["seq"], server[2]["seq"]);
    assert_ne!(delivered[0], delivered[1]);

    assert_eq!(
        store.resume_cursor("trades").await.as_deref(),
        fixture["expected"]["resumeCursor"].as_str()
    );
}

#[tokio::test]
async fn every_replay_refusal_reaches_the_consumer_with_its_own_code() {
    let fixture = fixture("replay-gaps.json");
    let delivered = self::fixture("replay-cursors.json");
    let mut codes = HashSet::new();
    for case in fixture["cases"].as_array().unwrap() {
        let response = &case["response"];
        let (store, mut stream) = append_consumer().await;
        // Put a real cursor in flight first: a refusal has to displace the
        // position the subscription was holding, not just arrive.
        for frame in &delivered["server"].as_array().unwrap()[..2] {
            apply(&store, frame).await;
        }
        next_update(&mut stream).await.expect("first record");

        let message = parse_server_message(&serde_json::to_vec(response).unwrap())
            .expect("a replay refusal should parse");
        let ServerMessage::Error(error) = message else {
            panic!("expected an error envelope")
        };
        store.apply_error_frame(&error).await;

        let update = next_update(&mut stream)
            .await
            .expect("a refusal must reach the consumer, not leave the stream empty");
        let gap = update.gap().expect("a refusal arrives as a gap");
        assert_eq!(gap.code.as_wire(), response["code"].as_str().unwrap());
        assert_eq!(
            gap.recover_from.as_deref(),
            response["recoverFrom"].as_str(),
            "{} must carry the server's recovery cursor verbatim",
            response["code"]
        );
        assert_eq!(
            gap.replay_window
                .as_ref()
                .and_then(|window| window.gap_after),
            response["replayWindow"]["gapAfter"].as_u64()
        );
        // A reconnect must never re-send a cursor the server just rejected;
        // `replay-lagged` is the one refusal that names where to pick up.
        assert_eq!(
            store.resume_cursor("trades").await.as_deref(),
            response["recoverFrom"].as_str()
        );
        assert!(
            next_update(&mut stream).await.is_none(),
            "delivery stops at the refusal"
        );
        codes.insert(gap.code.clone());
    }
    assert_eq!(codes.len(), fixture["cases"].as_array().unwrap().len());
}

#[tokio::test]
async fn a_local_overflow_ends_a_cursor_bearing_stream_at_its_resume_point() {
    let fixture = fixture("replay-cursors.json");
    let server = fixture["server"].as_array().unwrap();
    let (store, mut stream) = append_consumer().await;
    apply(&store, &server[0]).await;
    apply(&store, &server[1]).await;

    let first = next_update(&mut stream).await.expect("first record");
    let resume = first.cursor().expect("append cursor").to_string();

    // Overflow this consumer's queue while it is not polling. The broadcast
    // channel holds 1000 updates and evicts the oldest beyond that.
    let mut frame = server[1].clone();
    for offset in 4210..5400 {
        frame["offset"] = json!(offset);
        frame["seq"] = json!(format!("381471241:{offset:012}"));
        apply(&store, &frame).await;
    }

    let update = next_update(&mut stream)
        .await
        .expect("an overflow must be reported, not swallowed");
    let gap = update.gap().expect("an overflow arrives as a gap");
    assert!(
        matches!(gap.code, GapCode::LocalLag { skipped } if skipped > 0),
        "expected a local lag, got {:?}",
        gap.code
    );
    assert_eq!(gap.recover_from.as_deref(), Some(resume.as_str()));
    assert!(
        next_update(&mut stream).await.is_none(),
        "delivery stops at the gap rather than continuing past lost records"
    );
}

/// The overflow can arrive before the consumer has been handed anything, and
/// then there is no cursor to infer the view's nature from. Reading that
/// absence as "a projection, carry on" drops the head of the tape in silence,
/// which is the exact failure this whole path exists to prevent.
#[tokio::test]
async fn an_overflow_before_the_first_record_is_still_a_gap() {
    let fixture = fixture("replay-cursors.json");
    let server = fixture["server"].as_array().unwrap();
    let (store, mut stream) = append_consumer().await;
    // Only the acknowledgement: the consumer has received no record yet.
    apply(&store, &server[0]).await;

    let mut frame = server[1].clone();
    for offset in 4209..5400 {
        frame["offset"] = json!(offset);
        frame["seq"] = json!(format!("381471241:{offset:012}"));
        apply(&store, &frame).await;
    }

    let update = next_update(&mut stream)
        .await
        .expect("losing the head of a tape must be reported");
    let gap = update.gap().expect("an overflow arrives as a gap");
    assert!(
        matches!(gap.code, GapCode::LocalLag { skipped } if skipped > 0),
        "expected a local lag, got {:?}",
        gap.code
    );
    assert_eq!(
        gap.recover_from, None,
        "nothing was delivered, so there is no position to resume from — the \
         recovery is to resubscribe with no `after` and take the whole window"
    );
    assert!(
        next_update(&mut stream).await.is_none(),
        "delivery stops rather than continuing from the middle of the tape"
    );
}

/// A projection has no per-event identity, so a dropped row is superseded by
/// the next write rather than lost. Those streams must keep running.
#[tokio::test]
async fn an_overflow_on_a_projection_keeps_delivering() {
    let store = SharedStore::new();
    store
        .register_subscription("things", SubscriptionQuery::new("Thing/list"), true)
        .await
        .expect("subscription should register");
    let mut stream = Box::pin(EntityStream::<Value>::new(&store, "Thing/list".to_string()));

    apply(
        &store,
        &json!({
            "protocolVersion": 2,
            "subscriptionId": "things",
            "op": "subscribed",
            "mode": "list",
            "query": {"view": "Thing/list"},
        }),
    )
    .await;

    for index in 0..1_200 {
        apply(
            &store,
            &json!({
                "protocolVersion": 2,
                "subscriptionId": "things",
                "mode": "list",
                "entity": "Thing/list",
                "op": "upsert",
                "key": format!("thing{index}"),
                "data": {"n": index},
            }),
        )
        .await;
    }

    let update = next_update(&mut stream)
        .await
        .expect("a projection stream survives its own backlog");
    assert!(
        update.gap().is_none(),
        "a dropped projection row is not a gap: {:?}",
        update.gap()
    );
}

#[derive(Debug, Clone, serde::Deserialize)]
struct Trade {
    #[allow(dead_code)]
    amount: u64,
}

async fn typed_consumer() -> (SharedStore, impl futures_util::Stream<Item = Update<Trade>>) {
    let store = SharedStore::new();
    store
        .register_subscription("trades", SubscriptionQuery::new(APPEND_VIEW), true)
        .await
        .expect("subscription should register");
    let stream = Box::pin(EntityStream::<Trade>::new(&store, APPEND_VIEW.to_string()));
    (store, stream)
}

async fn next_trade(
    stream: &mut (impl futures_util::Stream<Item = Update<Trade>> + Unpin),
) -> Option<Update<Trade>> {
    timeout(Duration::from_secs(3), stream.next())
        .await
        .expect("the stream should not hang")
}

/// A record the consumer's type cannot read is still retained on the server,
/// so the recovery has to replay it. `after` is exclusive, so naming the
/// failed record would skip the very thing a consumer came back for after
/// fixing its type.
#[tokio::test]
async fn an_undecodable_record_recovers_from_the_position_in_front_of_it() {
    let fixture = fixture("replay-cursors.json");
    let server = fixture["server"].as_array().unwrap();
    let (store, mut stream) = typed_consumer().await;
    apply(&store, &server[0]).await;
    apply(&store, &server[1]).await;

    let first = next_trade(&mut stream).await.expect("first record");
    let delivered = first.cursor().expect("append cursor").to_string();

    let mut broken = server[2].clone();
    broken["data"] = json!({"amount": "not a number"});
    apply(&store, &broken).await;

    let update = next_trade(&mut stream)
        .await
        .expect("an unreadable record must not be skipped in silence");
    let gap = update
        .gap()
        .expect("an undecodable record arrives as a gap");
    assert!(matches!(gap.code, GapCode::Undecodable));
    assert_eq!(
        gap.recover_from.as_deref(),
        Some(delivered.as_str()),
        "recovery must replay the record that failed, not start after it"
    );
    assert!(
        next_trade(&mut stream).await.is_none(),
        "delivery stops at the gap"
    );
}

/// Nothing was delivered before it, so there is no position in front of it.
#[tokio::test]
async fn an_undecodable_first_record_offers_no_resume_position() {
    let fixture = fixture("replay-cursors.json");
    let server = fixture["server"].as_array().unwrap();
    let (store, mut stream) = typed_consumer().await;
    apply(&store, &server[0]).await;

    let mut broken = server[1].clone();
    broken["data"] = json!({"amount": "not a number"});
    apply(&store, &broken).await;

    let update = next_trade(&mut stream).await.expect("a gap, not silence");
    let gap = update
        .gap()
        .expect("an undecodable record arrives as a gap");
    assert_eq!(
        gap.recover_from, None,
        "resubscribing with no `after` is the only recovery that replays it"
    );
}

/// The failure that opened this PR was a class, not three fields: the server
/// deploys on its own schedule while a pinned SDK upgrades on the consumer's,
/// so rejecting an added field breaks every deployed client at once. A4-278's
/// occurrence provenance is the next such field.
#[tokio::test]
async fn a_field_this_sdk_has_never_heard_of_is_not_a_protocol_error() {
    let fixture = fixture("replay-cursors.json");
    let server = fixture["server"].as_array().unwrap();

    let mut ack = server[0].clone();
    ack["replayWindow"]["retentionSeconds"] = json!(3_600);
    ack["somethingLater"] = json!("ignored");
    let mut record = server[1].clone();
    record["occurrence"] = json!("ix:3:1");
    let error = json!({
        "protocolVersion": 2,
        "type": "error",
        "subscriptionId": "trades",
        "code": "replay-lagged",
        "fatal": false,
        "recoverFrom": "0f8c2b31-6a4e-4f0b-9a77-1d2c3e4f5a6b:4180",
        "diagnosticsUrl": "https://example.invalid/why",
    });

    for frame in [&ack, &record, &error] {
        parse_server_message(frame.to_string().as_bytes())
            .unwrap_or_else(|e| panic!("an added field must not break the client: {e}"));
    }

    // Still delivered, not merely parsed.
    let (store, mut stream) = append_consumer().await;
    apply(&store, &ack).await;
    apply(&store, &record).await;
    let update = next_update(&mut stream).await.expect("the record arrives");
    assert_eq!(
        update.cursor(),
        Some("0f8c2b31-6a4e-4f0b-9a77-1d2c3e4f5a6b:4209")
    );
}
