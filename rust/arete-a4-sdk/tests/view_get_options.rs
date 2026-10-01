//! One-shot reads with query options (`ViewHandle::get_with`,
//! `StateView::get_with`): the subscription they send is the one TypeScript's
//! `list.get(options)` / `state.get(key, options)` sends, and they release it
//! after the snapshot.

use arete_a4_sdk::{Arete, GetOptions, Stack, StateView, ViewBuilder, ViewHandle, Views};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::time::timeout;
use tokio_tungstenite::{accept_async, tungstenite::Message, WebSocketStream};

struct TestViews {
    things: ViewHandle<Value>,
    thing_state: StateView<Value>,
}

impl Views for TestViews {
    fn from_builder(builder: ViewBuilder) -> Self {
        Self {
            things: builder.view("Thing/list"),
            thing_state: StateView::new(
                builder.connection().clone(),
                builder.store().clone(),
                "Thing/state".to_string(),
                builder.initial_data_timeout(),
            ),
        }
    }
}

struct TestStack;

impl Stack for TestStack {
    type Views = TestViews;
    type Programs = ();

    fn name() -> &'static str {
        "view-get-options-test"
    }

    fn url() -> &'static str {
        "ws://127.0.0.1:1"
    }
}

async fn send_json(socket: &mut WebSocketStream<tokio::net::TcpStream>, value: Value) {
    socket.send(Message::Text(value.to_string())).await.unwrap();
}

/// A protocol v2 server that forwards every client message (pings aside) and
/// answers each `subscribe` with an acknowledgement and, when the
/// subscription asks for one and `snapshots` is set, an authoritative
/// snapshot: rows `1` and `2` for a list query, the key's entity for a keyed
/// one.
async fn serve(snapshots: bool) -> (String, mpsc::UnboundedReceiver<Value>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let (message_tx, message_rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        while let Some(Ok(message)) = socket.next().await {
            let Ok(text) = message.to_text() else {
                continue;
            };
            let Ok(request) = serde_json::from_str::<Value>(text) else {
                continue;
            };
            if request["type"] == "ping" {
                continue;
            }
            let _ = message_tx.send(request.clone());
            if request["type"] != "subscribe" {
                continue;
            }
            let subscription_id = request["subscriptionId"].clone();
            let query = request["query"].clone();
            let key = query.get("key").cloned();
            let mode = if key.is_some() { "state" } else { "list" };
            send_json(
                &mut socket,
                json!({
                    "protocolVersion": 2,
                    "subscriptionId": subscription_id,
                    "op": "subscribed",
                    "query": query,
                    "mode": mode,
                }),
            )
            .await;
            if !snapshots || request["snapshot"]["enabled"] == false {
                continue;
            }
            let rows = match &key {
                Some(key) => vec![json!({"key": key, "data": {"id": key}})],
                None => vec![
                    json!({"key": "1", "data": {"id": 1}}),
                    json!({"key": "2", "data": {"id": 2}}),
                ],
            };
            let mut snapshot = json!({
                "protocolVersion": 2,
                "subscriptionId": subscription_id,
                "snapshotId": "get-options",
                "authoritative": true,
                "mode": mode,
                "entity": query["view"],
                "op": "snapshot",
                "data": rows,
                "complete": true,
            });
            if let Some(key) = key {
                snapshot["key"] = key;
            }
            send_json(&mut socket, snapshot).await;
        }
    });
    (url, message_rx)
}

async fn next_message(messages: &mut mpsc::UnboundedReceiver<Value>) -> Value {
    timeout(Duration::from_secs(3), messages.recv())
        .await
        .expect("client message should arrive")
        .expect("server should still be forwarding")
}

/// The subscribe frame without its opaque id, and that id.
fn subscription_of(mut subscribe: Value) -> (Value, Value) {
    assert_eq!(subscribe["type"], "subscribe");
    let id = subscribe
        .as_object_mut()
        .unwrap()
        .remove("subscriptionId")
        .expect("subscribe carries a subscriptionId");
    (subscribe, id)
}

async fn expect_unsubscribe(messages: &mut mpsc::UnboundedReceiver<Value>, id: &Value) {
    let unsubscribe = next_message(messages).await;
    assert_eq!(unsubscribe["type"], "unsubscribe");
    assert_eq!(&unsubscribe["subscriptionId"], id);
}

async fn connect(url: &str) -> Arete<TestStack> {
    Arete::<TestStack>::builder()
        .url(url)
        .initial_data_timeout(Duration::from_secs(3))
        .connect()
        .await
        .unwrap()
}

#[tokio::test]
async fn list_get_with_sends_every_query_option_and_releases_the_read() {
    let (url, mut messages) = serve(true).await;
    let client = connect(&url).await;

    let rows = client
        .views
        .things
        .get_with(
            GetOptions::new()
                .filter("state.authority", "Auth1111")
                .filter("market.symbol", "SOL")
                .take(5)
                .skip(1)
                .partition("solana-mainnet")
                .after("3:4")
                .with_snapshot_limit(2),
        )
        .await;
    assert_eq!(rows, vec![json!({"id": 1}), json!({"id": 2})]);

    let (subscribe, id) = subscription_of(next_message(&mut messages).await);
    assert_eq!(
        subscribe,
        json!({
            "type": "subscribe",
            "protocolVersion": 2,
            "query": {
                "view": "Thing/list",
                "partition": "solana-mainnet",
                "filters": {"market.symbol": "SOL", "state.authority": "Auth1111"},
                "take": 5,
                "skip": 1,
                "after": "3:4",
                "snapshotLimit": 2
            },
            "snapshot": {"enabled": true}
        })
    );
    expect_unsubscribe(&mut messages, &id).await;
    client.disconnect().await;
}

/// TypeScript `list.get({ filters: { 'tokens.base_mint': a, 'tokens.quote_mint': b } })`
/// sends exactly the filters, nothing else.
#[tokio::test]
async fn list_get_with_filters_only_sends_only_the_filters() {
    let (url, mut messages) = serve(true).await;
    let client = connect(&url).await;

    let rows = client
        .views
        .things
        .get_with(
            GetOptions::new()
                .filter(
                    "tokens.quote_mint",
                    "So11111111111111111111111111111111111111112",
                )
                .filter(
                    "tokens.base_mint",
                    "Base111111111111111111111111111111111111111",
                ),
        )
        .await;
    assert_eq!(rows.len(), 2);

    let (subscribe, id) = subscription_of(next_message(&mut messages).await);
    assert_eq!(
        subscribe["query"],
        json!({
            "view": "Thing/list",
            "filters": {
                "tokens.base_mint": "Base111111111111111111111111111111111111111",
                "tokens.quote_mint": "So11111111111111111111111111111111111111112"
            }
        })
    );
    assert_eq!(subscribe["snapshot"], json!({"enabled": true}));
    expect_unsubscribe(&mut messages, &id).await;
    client.disconnect().await;
}

#[tokio::test]
async fn get_and_default_get_with_send_the_bare_query() {
    let (url, mut messages) = serve(true).await;
    let client = connect(&url).await;

    assert_eq!(client.views.things.get().await.len(), 2);
    let (plain, plain_id) = subscription_of(next_message(&mut messages).await);
    expect_unsubscribe(&mut messages, &plain_id).await;

    assert_eq!(
        client
            .views
            .things
            .get_with(GetOptions::default())
            .await
            .len(),
        2
    );
    let (defaulted, defaulted_id) = subscription_of(next_message(&mut messages).await);
    expect_unsubscribe(&mut messages, &defaulted_id).await;

    let bare = json!({
        "type": "subscribe",
        "protocolVersion": 2,
        "query": {"view": "Thing/list"},
        "snapshot": {"enabled": true}
    });
    assert_eq!(plain, bare);
    assert_eq!(defaulted, bare);
    client.disconnect().await;
}

#[tokio::test]
async fn state_get_with_sends_the_key_and_the_options() {
    let (url, mut messages) = serve(true).await;
    let client = connect(&url).await;

    let entity = client
        .views
        .thing_state
        .get_with(
            "7",
            GetOptions::new()
                .partition("solana-mainnet")
                .filter("state.open", true),
        )
        .await;
    assert_eq!(entity, Some(json!({"id": "7"})));

    let (subscribe, id) = subscription_of(next_message(&mut messages).await);
    assert_eq!(
        subscribe["query"],
        json!({
            "view": "Thing/state",
            "key": "7",
            "partition": "solana-mainnet",
            "filters": {"state.open": true}
        })
    );
    expect_unsubscribe(&mut messages, &id).await;

    assert_eq!(
        client.views.thing_state.get("7").await,
        Some(json!({"id": "7"}))
    );
    let (plain, plain_id) = subscription_of(next_message(&mut messages).await);
    assert_eq!(plain["query"], json!({"view": "Thing/state", "key": "7"}));
    expect_unsubscribe(&mut messages, &plain_id).await;
    client.disconnect().await;
}

#[tokio::test]
async fn get_with_snapshot_disabled_resolves_on_the_acknowledgement() {
    let (url, mut messages) = serve(true).await;
    let client = connect(&url).await;

    let rows = client
        .views
        .things
        .get_with(GetOptions::new().with_snapshot(false))
        .await;
    assert!(rows.is_empty());

    let (subscribe, id) = subscription_of(next_message(&mut messages).await);
    assert_eq!(subscribe["query"], json!({"view": "Thing/list"}));
    assert_eq!(subscribe["snapshot"], json!({"enabled": false}));
    expect_unsubscribe(&mut messages, &id).await;
    client.disconnect().await;
}

#[tokio::test]
async fn get_with_timeout_bounds_the_snapshot_wait() {
    let (url, mut messages) = serve(false).await;
    let client = Arete::<TestStack>::builder()
        .url(&url)
        .initial_data_timeout(Duration::from_secs(30))
        .connect()
        .await
        .unwrap();

    let started = Instant::now();
    let rows = timeout(
        Duration::from_secs(5),
        client
            .views
            .things
            .get_with(GetOptions::new().timeout(Duration::from_millis(50))),
    )
    .await
    .expect("the per-read timeout replaces initial_data_timeout");
    assert!(rows.is_empty());
    assert!(started.elapsed() < Duration::from_secs(5));

    let (subscribe, id) = subscription_of(next_message(&mut messages).await);
    assert_eq!(subscribe["query"], json!({"view": "Thing/list"}));
    expect_unsubscribe(&mut messages, &id).await;
    client.disconnect().await;
}
