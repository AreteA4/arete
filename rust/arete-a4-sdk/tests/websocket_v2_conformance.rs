use arete_a4_sdk::{
    parse_server_message, ClientMessage, ServerMessage, SharedStore, SnapshotOptions, Subscription,
    SubscriptionQuery,
};
use serde_json::{json, Value};
use std::path::PathBuf;

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
