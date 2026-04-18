use axum::{extract::State, http::HeaderMap, routing::post, Json, Router};
use base64::Engine as _;
use futures_util::{SinkExt, StreamExt};
use arete_sdk::{Arete, SocketIssue, Stack, TokenTransport, ViewBuilder, Views};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::{timeout, Duration};
use tokio_tungstenite::{
    accept_hdr_async,
    tungstenite::{
        handshake::server::{Request, Response},
        Message,
    },
};
use url::form_urlencoded;

#[derive(Clone)]
struct TestViews;

impl Views for TestViews {
    fn from_builder(_: ViewBuilder) -> Self {
        Self
    }
}

struct TestStack;

impl Stack for TestStack {
    type Views = TestViews;

    fn name() -> &'static str {
        "test-stack"
    }

    fn url() -> &'static str {
        "ws://127.0.0.1:1"
    }
}

#[derive(Debug, Clone)]
struct HandshakeCapture {
    query_token: Option<String>,
    authorization_header: Option<String>,
}

#[derive(Clone)]
struct TokenEndpointState {
    issued_tokens: Arc<tokio::sync::Mutex<Vec<String>>>,
    authorization_headers: Arc<tokio::sync::Mutex<Vec<Option<String>>>>,
    websocket_urls: Arc<tokio::sync::Mutex<Vec<String>>>,
    next_token_index: Arc<AtomicUsize>,
    expiries: Arc<Vec<u64>>,
}

#[derive(Debug, Deserialize)]
struct TokenEndpointRequestBody {
    websocket_url: String,
}

#[derive(Debug, Serialize)]
struct TokenEndpointResponseBody {
    token: String,
    expires_at: u64,
}

struct TokenEndpointHandle {
    state: TokenEndpointState,
    shutdown_tx: Option<oneshot::Sender<()>>,
    join_handle: JoinHandle<()>,
    url: String,
}

impl TokenEndpointHandle {
    async fn shutdown(mut self) {
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }
        let _ = self.join_handle.await;
    }
}

struct WebSocketServerHandle {
    join_handle: JoinHandle<()>,
    url: String,
}

impl WebSocketServerHandle {
    async fn shutdown(self) {
        let _ = self.join_handle.await;
    }
}

#[tokio::test]
async fn fetches_token_from_endpoint_and_refreshes_in_band() {
    let (handshake_tx, handshake_rx) = oneshot::channel();
    let (refresh_tx, mut refresh_rx) = mpsc::channel(4);

    let ws_server = spawn_websocket_server(handshake_tx, refresh_tx).await;
    let token_endpoint = spawn_token_endpoint(vec![61, 3600]).await;

    let client = Arete::<TestStack>::builder()
        .url(&ws_server.url)
        .publishable_key("hspk_test_123")
        .token_endpoint(token_endpoint.url.clone())
        .connect()
        .await
        .expect("client should connect through token endpoint");

    let handshake = timeout(Duration::from_secs(3), handshake_rx)
        .await
        .expect("websocket handshake should complete")
        .expect("handshake channel should resolve");

    let issued_tokens = token_endpoint.state.issued_tokens.lock().await.clone();
    assert_eq!(
        issued_tokens.len(),
        1,
        "first token should be minted on connect"
    );
    assert_eq!(handshake.authorization_header, None);
    assert_eq!(handshake.query_token, Some(issued_tokens[0].clone()));

    let endpoint_headers = token_endpoint
        .state
        .authorization_headers
        .lock()
        .await
        .clone();
    assert_eq!(
        endpoint_headers,
        vec![Some("Bearer hspk_test_123".to_string())],
        "publishable key should be forwarded to the token endpoint"
    );

    let requested_urls = token_endpoint.state.websocket_urls.lock().await.clone();
    assert_eq!(requested_urls, vec![ws_server.url.clone()]);

    let refreshed_token = timeout(Duration::from_secs(5), refresh_rx.recv())
        .await
        .expect("refresh_auth message should be sent before expiry")
        .expect("refresh channel should receive a token");

    let issued_tokens = token_endpoint.state.issued_tokens.lock().await.clone();
    assert_eq!(issued_tokens.len(), 2, "refresh should mint a second token");
    assert_eq!(refreshed_token, issued_tokens[1]);

    client.disconnect().await;
    ws_server.shutdown().await;
    token_endpoint.shutdown().await;
}

#[tokio::test]
async fn uses_bearer_transport_for_websocket_handshake() {
    let (handshake_tx, handshake_rx) = oneshot::channel();
    let (refresh_tx, _refresh_rx) = mpsc::channel(1);

    let ws_server = spawn_websocket_server(handshake_tx, refresh_tx).await;
    let token_endpoint = spawn_token_endpoint(vec![3600]).await;

    let client = Arete::<TestStack>::builder()
        .url(&ws_server.url)
        .publishable_key("hspk_test_123")
        .token_endpoint(token_endpoint.url.clone())
        .token_transport(TokenTransport::Bearer)
        .connect()
        .await
        .expect("client should connect with bearer transport");

    let handshake = timeout(Duration::from_secs(3), handshake_rx)
        .await
        .expect("websocket handshake should complete")
        .expect("handshake channel should resolve");

    let issued_tokens = token_endpoint.state.issued_tokens.lock().await.clone();
    assert_eq!(issued_tokens.len(), 1);
    assert_eq!(handshake.query_token, None);
    assert_eq!(
        handshake.authorization_header,
        Some(format!("Bearer {}", issued_tokens[0]))
    );

    client.disconnect().await;
    ws_server.shutdown().await;
    token_endpoint.shutdown().await;
}

#[tokio::test]
async fn exposes_socket_issues_via_public_api() {
    let ws_server = spawn_socket_issue_server(json!({
        "type": "error",
        "error": "subscription-limit-exceeded",
        "message": "Subscription limit exceeded",
        "code": "subscription-limit-exceeded",
        "retryable": false,
        "suggested_action": "unsubscribe first",
        "fatal": false
    }))
    .await;

    let client = Arete::<TestStack>::builder()
        .url(&ws_server.url)
        .connect()
        .await
        .expect("client should connect to socket issue server");

    let mut issues = client.subscribe_socket_issues();
    let issue = timeout(Duration::from_secs(3), issues.recv())
        .await
        .expect("socket issue should arrive")
        .expect("socket issue broadcast should succeed");

    assert_eq!(
        issue,
        SocketIssue {
            error: "subscription-limit-exceeded".to_string(),
            message: "Subscription limit exceeded".to_string(),
            code: Some(arete_sdk::AuthErrorCode::SubscriptionLimitExceeded),
            retryable: false,
            retry_after: None,
            suggested_action: Some("unsubscribe first".to_string()),
            docs_url: None,
            fatal: false,
        }
    );

    let last_issue = timeout(Duration::from_secs(3), async {
        loop {
            if let Some(issue) = client.last_socket_issue().await {
                break issue;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("last_socket_issue should be recorded");

    assert_eq!(last_issue.message, "Subscription limit exceeded");

    client.disconnect().await;
    ws_server.shutdown().await;
}

async fn spawn_token_endpoint(expiries_from_now: Vec<u64>) -> TokenEndpointHandle {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("token endpoint listener should bind");
    let addr = listener
        .local_addr()
        .expect("token endpoint listener should have an address");
    let (shutdown_tx, shutdown_rx) = oneshot::channel();

    let state = TokenEndpointState {
        issued_tokens: Arc::new(tokio::sync::Mutex::new(Vec::new())),
        authorization_headers: Arc::new(tokio::sync::Mutex::new(Vec::new())),
        websocket_urls: Arc::new(tokio::sync::Mutex::new(Vec::new())),
        next_token_index: Arc::new(AtomicUsize::new(0)),
        expiries: Arc::new(expiries_from_now),
    };

    let app = Router::new()
        .route("/ws/sessions", post(issue_ws_session))
        .with_state(state.clone());

    let join_handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await
            .expect("token endpoint server should run");
    });

    TokenEndpointHandle {
        state,
        shutdown_tx: Some(shutdown_tx),
        join_handle,
        url: format!("http://{addr}/ws/sessions"),
    }
}

async fn issue_ws_session(
    State(state): State<TokenEndpointState>,
    headers: HeaderMap,
    Json(body): Json<TokenEndpointRequestBody>,
) -> Json<TokenEndpointResponseBody> {
    let index = state.next_token_index.fetch_add(1, Ordering::SeqCst);
    let expires_at = current_unix_timestamp() + state.expiries[index];
    let token = make_test_jwt(expires_at, index);

    state.authorization_headers.lock().await.push(
        headers
            .get("Authorization")
            .and_then(|value| value.to_str().ok())
            .map(str::to_string),
    );
    state.websocket_urls.lock().await.push(body.websocket_url);
    state.issued_tokens.lock().await.push(token.clone());

    Json(TokenEndpointResponseBody { token, expires_at })
}

async fn spawn_websocket_server(
    handshake_tx: oneshot::Sender<HandshakeCapture>,
    refresh_tx: mpsc::Sender<String>,
) -> WebSocketServerHandle {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("websocket listener should bind");
    let addr = listener
        .local_addr()
        .expect("websocket listener should have an address");
    let handshake_tx = Arc::new(Mutex::new(Some(handshake_tx)));

    let join_handle = tokio::spawn(async move {
        let (stream, _) = listener
            .accept()
            .await
            .expect("websocket server should accept a connection");

        let ws_stream = accept_hdr_async(stream, {
            let handshake_tx = handshake_tx.clone();
            move |request: &Request, response: Response| {
                let query_token = extract_query_token(request.uri());
                let authorization_header = request
                    .headers()
                    .get("Authorization")
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_string);

                if let Some(tx) = handshake_tx
                    .lock()
                    .expect("handshake sender mutex should not be poisoned")
                    .take()
                {
                    let _ = tx.send(HandshakeCapture {
                        query_token,
                        authorization_header,
                    });
                }

                Ok(response)
            }
        })
        .await
        .expect("websocket handshake should succeed");

        let (_write, mut read) = ws_stream.split();
        while let Some(message) = read.next().await {
            match message.expect("websocket message should be readable") {
                Message::Text(text) => {
                    let payload: Value = serde_json::from_str(&text)
                        .expect("websocket text payload should be valid json");
                    if payload.get("type") == Some(&json!("refresh_auth")) {
                        if let Some(token) = payload.get("token").and_then(Value::as_str) {
                            let _ = refresh_tx.send(token.to_string()).await;
                        }
                    }
                }
                Message::Close(_) => break,
                _ => {}
            }
        }
    });

    WebSocketServerHandle {
        join_handle,
        url: format!("ws://{addr}"),
    }
}

async fn spawn_socket_issue_server(issue_payload: Value) -> WebSocketServerHandle {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("socket issue listener should bind");
    let addr = listener
        .local_addr()
        .expect("socket issue listener should have an address");

    let join_handle = tokio::spawn(async move {
        let (stream, _) = listener
            .accept()
            .await
            .expect("socket issue server should accept a connection");

        let mut ws_stream = accept_hdr_async(stream, |_request: &Request, response: Response| {
            Ok(response)
        })
        .await
        .expect("socket issue websocket handshake should succeed");

        tokio::time::sleep(Duration::from_millis(100)).await;
        ws_stream
            .send(Message::Text(issue_payload.to_string()))
            .await
            .expect("socket issue message should send");

        tokio::time::sleep(Duration::from_millis(100)).await;
    });

    WebSocketServerHandle {
        join_handle,
        url: format!("ws://{addr}"),
    }
}

fn extract_query_token(uri: &tokio_tungstenite::tungstenite::http::Uri) -> Option<String> {
    uri.query().and_then(|query| {
        form_urlencoded::parse(query.as_bytes())
            .find(|(key, _)| key == "hs_token")
            .map(|(_, value)| value.into_owned())
    })
}

fn make_test_jwt(exp: u64, sequence: usize) -> String {
    let header =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(br#"{"alg":"none","typ":"JWT"}"#);
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
        serde_json::to_string(&json!({
            "exp": exp,
            "seq": sequence,
        }))
        .expect("test jwt payload should serialize"),
    );

    format!("{header}.{payload}.signature")
}

fn current_unix_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock should be after unix epoch")
        .as_secs()
}
