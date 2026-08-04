use arete_a4_sdk::{Arete, Stack, ViewBuilder, ViewHandle, Views};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::time::timeout;
use tokio_tungstenite::{accept_async, tungstenite::Message};

struct TestViews {
    things: ViewHandle<Value>,
}

impl Views for TestViews {
    fn from_builder(builder: ViewBuilder) -> Self {
        Self {
            things: builder.view("Thing/list"),
        }
    }
}

struct TestStack;

impl Stack for TestStack {
    type Views = TestViews;
    type Programs = ();

    fn name() -> &'static str {
        "protocol-v2-test"
    }

    fn url() -> &'static str {
        "ws://127.0.0.1:1"
    }
}

async fn next_json(
    socket: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
) -> Value {
    loop {
        let message = timeout(Duration::from_secs(3), socket.next())
            .await
            .expect("client message should arrive")
            .expect("socket should remain open")
            .expect("client message should be valid");
        let value: Value = serde_json::from_str(message.to_text().expect("message should be text"))
            .expect("message should be JSON");
        if value["type"] != "ping" {
            return value;
        }
    }
}

async fn send_snapshot(
    socket: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    subscribe: &Value,
    snapshot_id: &str,
    id: i64,
) {
    let subscription_id = subscribe["subscriptionId"].as_str().unwrap();
    socket
        .send(Message::Text(
            json!({
                "protocolVersion": 2,
                "subscriptionId": subscription_id,
                "op": "subscribed",
                "query": subscribe["query"],
                "mode": "list"
            })
            .to_string(),
        ))
        .await
        .unwrap();
    socket
        .send(Message::Text(
            json!({
                "protocolVersion": 2,
                "subscriptionId": subscription_id,
                "snapshotId": snapshot_id,
                "authoritative": true,
                "mode": "list",
                "entity": "Thing/list",
                "op": "snapshot",
                "data": [{"key": id.to_string(), "data": {"id": id}}],
                "complete": true
            })
            .to_string(),
        ))
        .await
        .unwrap();
}

#[tokio::test]
async fn equivalent_streams_refcount_wire_subscription_and_final_drop_unsubscribes() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let (message_tx, mut message_rx) = mpsc::channel(4);
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        let subscribe = next_json(&mut socket).await;
        message_tx.send(subscribe.clone()).await.unwrap();
        send_snapshot(&mut socket, &subscribe, "shared", 7).await;
        while let Ok(Some(message)) = timeout(Duration::from_secs(3), socket.next()).await {
            let message = message.unwrap();
            if message.is_text() {
                let value: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
                if value["type"] != "ping" {
                    message_tx.send(value).await.unwrap();
                }
            }
        }
    });

    let client = Arete::<TestStack>::builder()
        .url(&url)
        .connect()
        .await
        .unwrap();
    let mut first = Box::pin(
        client
            .views
            .things
            .watch()
            .filter("state.status", "open")
            .filter("market.symbol", "SOL")
            .filter("state.active", true)
            .take(10)
            .skip(2),
    );
    let mut second = Box::pin(
        client
            .views
            .things
            .watch()
            .filter("state.active", true)
            .filter("market.symbol", "SOL")
            .filter("state.status", "open")
            .take(10)
            .skip(2),
    );
    let (first_update, second_update) = tokio::join!(
        timeout(Duration::from_secs(3), first.next()),
        timeout(Duration::from_secs(3), second.next())
    );
    assert!(first_update.unwrap().is_some());
    assert!(second_update.unwrap().is_some());

    let subscribe = message_rx.recv().await.unwrap();
    assert_eq!(subscribe["type"], "subscribe");
    assert_eq!(subscribe["protocolVersion"], 2);
    assert_eq!(subscribe["query"]["take"], 10);
    assert_eq!(subscribe["query"]["skip"], 2);
    assert_eq!(
        subscribe["query"]["filters"],
        json!({"market.symbol": "SOL", "state.active": true, "state.status": "open"})
    );
    assert!(timeout(Duration::from_millis(100), message_rx.recv())
        .await
        .is_err());

    drop(first);
    assert!(timeout(Duration::from_millis(100), message_rx.recv())
        .await
        .is_err());
    drop(second);
    let unsubscribe = timeout(Duration::from_secs(3), message_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(unsubscribe["type"], "unsubscribe");
    assert_eq!(unsubscribe["protocolVersion"], 2);
    assert_eq!(unsubscribe["subscriptionId"], subscribe["subscriptionId"]);

    client.disconnect().await;
    server.await.unwrap();
}

#[tokio::test]
async fn reconnect_resubscribes_with_the_same_opaque_id() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let (message_tx, mut message_rx) = mpsc::channel(4);
    let server = tokio::spawn(async move {
        for (connection, id) in [10_i64, 11].into_iter().enumerate() {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            let subscribe = next_json(&mut socket).await;
            message_tx.send(subscribe.clone()).await.unwrap();
            send_snapshot(
                &mut socket,
                &subscribe,
                &format!("connection-{connection}"),
                id,
            )
            .await;
            if connection == 0 {
                socket.close(None).await.unwrap();
            } else {
                let unsubscribe = next_json(&mut socket).await;
                message_tx.send(unsubscribe).await.unwrap();
            }
        }
    });

    let client = Arete::<TestStack>::builder()
        .url(&url)
        .reconnect_intervals(vec![Duration::from_millis(10)])
        .max_reconnect_attempts(3)
        .connect()
        .await
        .unwrap();
    let mut stream = Box::pin(client.views.things.watch());
    assert!(timeout(Duration::from_secs(3), stream.next())
        .await
        .unwrap()
        .is_some());
    let first = timeout(Duration::from_secs(3), message_rx.recv())
        .await
        .unwrap()
        .unwrap();
    let second = timeout(Duration::from_secs(3), message_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first["type"], "subscribe");
    assert_eq!(second["type"], "subscribe");
    assert_eq!(first["subscriptionId"], second["subscriptionId"]);

    drop(stream);
    let unsubscribe = timeout(Duration::from_secs(3), message_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(unsubscribe["type"], "unsubscribe");
    assert_eq!(unsubscribe["subscriptionId"], first["subscriptionId"]);

    client.disconnect().await;
    server.await.unwrap();
}
