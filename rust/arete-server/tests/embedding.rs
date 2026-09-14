//! The embedding surface: `Runtime::spawn` and `RuntimeHandle` instead of
//! `start()`. The caller owns the listener and hands accepted connections to
//! the runtime, and a handle stops exactly the runtime it belongs to.

use std::net::SocketAddr;
use std::time::Duration;

use arete_server::config::RuntimePlan;
use arete_server::{RuntimeHandle, Server};
use futures_util::{SinkExt, StreamExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::Message;

fn embedded_plan() -> RuntimePlan {
    RuntimePlan {
        health: false,
        chain_reads: false,
        program_reads: false,
        stack_queries: false,
        transactions: false,
        websocket: false,
        live_runtime: true,
    }
}

async fn spawn_embedded() -> RuntimeHandle {
    Server::builder()
        .runtime_plan(embedded_plan())
        .build()
        .expect("build runtime")
        .spawn()
        .await
        .expect("spawn runtime")
}

/// Open a WebSocket over `stream` and confirm the server answers a frame.
async fn websocket_round_trip(stream: TcpStream, addr: SocketAddr) {
    let (mut socket, _) = tokio_tungstenite::client_async(format!("ws://{addr}/"), stream)
        .await
        .expect("websocket handshake");
    socket
        .send(Message::Text(
            r#"{"type":"subscribe","view":"Missing/state"}"#.into(),
        ))
        .await
        .expect("send");
    let reply = tokio::time::timeout(Duration::from_secs(5), socket.next())
        .await
        .expect("server replies within the timeout")
        .expect("stream open")
        .expect("frame");
    assert!(reply.is_text(), "expected a text frame, got {reply:?}");
    socket.close(None).await.ok();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_handle_serves_caller_accepted_connections_and_stops_only_its_own_runtime() {
    let first = spawn_embedded().await;
    let second = spawn_embedded().await;
    assert!(first.is_ready().await);
    assert!(second.is_ready().await);
    assert_eq!(first.client_count(), 0);
    assert_eq!(second.client_count(), 0);

    // The test owns the listener; nothing in either runtime bound a port.
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");

    for handle in [&first, &second] {
        let client = TcpStream::connect(addr).await.expect("connect");
        let (accepted, peer) = listener.accept().await.expect("accept");
        let handle_clone = handle_view(handle);
        let served =
            tokio::spawn(async move { handle_clone.serve_connection(accepted, peer).await });
        websocket_round_trip(client, addr).await;
        served
            .await
            .expect("serve task")
            .expect("connection served cleanly");
    }

    // A session still open when its runtime shuts down is ended by the
    // server, and shutting one handle down does not affect another.
    let server = first.connection_server().expect("live runtime");
    let client = TcpStream::connect(addr).await.expect("connect");
    let (accepted, peer) = listener.accept().await.expect("accept");
    let served = tokio::spawn(async move { server.serve(accepted, peer).await });
    let (mut open_socket, _) = tokio_tungstenite::client_async(format!("ws://{addr}/"), client)
        .await
        .expect("websocket handshake");
    // The server registers the client on the serving task after the
    // handshake completes on its side, so the count can trail the client's
    // view of the handshake by a scheduling tick.
    tokio::time::timeout(Duration::from_secs(5), async {
        while first.client_count() != 1 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the open session is counted");
    first.shutdown().await.expect("first shutdown");
    served
        .await
        .expect("serve task")
        .expect("session ended cleanly on shutdown");
    let ended = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(frame) = open_socket.next().await {
            if frame.is_err() || frame.as_ref().is_ok_and(Message::is_close) {
                break;
            }
        }
    })
    .await;
    assert!(
        ended.is_ok(),
        "the open session should end when its runtime stops"
    );
    let client = TcpStream::connect(addr).await.expect("connect");
    let (accepted, peer) = listener.accept().await.expect("accept");
    let second_view = handle_view(&second);
    let served = tokio::spawn(async move { second_view.serve_connection(accepted, peer).await });
    websocket_round_trip(client, addr).await;
    served
        .await
        .expect("serve task")
        .expect("served after the first handle shut down");

    second.shutdown().await.expect("second shutdown");
}

#[tokio::test]
async fn a_runtime_without_a_live_runtime_refuses_connections() {
    let plan = RuntimePlan {
        live_runtime: false,
        ..embedded_plan()
    };
    let handle = Server::builder()
        .runtime_plan(plan)
        .build()
        .expect("build")
        .spawn()
        .await
        .expect("spawn");
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let _client = TcpStream::connect(addr).await.expect("connect");
    let (accepted, peer) = listener.accept().await.expect("accept");
    let error = handle
        .serve_connection(accepted, peer)
        .await
        .expect_err("no live runtime to serve from");
    assert!(error.to_string().contains("no live runtime"));
    handle.shutdown().await.expect("shutdown");
}

/// `serve_connection` borrows the handle, and the handle is not `Clone` -
/// callers keep it in an `Arc`. Model that here.
fn handle_view(handle: &RuntimeHandle) -> std::sync::Arc<RuntimeHandleView> {
    std::sync::Arc::new(RuntimeHandleView(handle as *const RuntimeHandle))
}

struct RuntimeHandleView(*const RuntimeHandle);
unsafe impl Send for RuntimeHandleView {}
unsafe impl Sync for RuntimeHandleView {}
impl RuntimeHandleView {
    async fn serve_connection(&self, stream: TcpStream, addr: SocketAddr) -> anyhow::Result<()> {
        // SAFETY: the test awaits the spawned task before the handle it points
        // at is moved or dropped.
        unsafe { &*self.0 }.serve_connection(stream, addr).await
    }
}
