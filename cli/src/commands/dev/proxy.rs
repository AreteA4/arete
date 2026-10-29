//! The local RPC endpoint `a4 dev up` serves for the cloud dev environment.
//!
//! The environment's own RPC endpoint needs an environment token as a
//! bearer credential, which wallets, the Solana CLI, web3 clients and the
//! local signer cannot add. So `a4 dev up` serves the environment on
//! loopback, the way a local validator would be:
//!
//! - `http://127.0.0.1:<port>`: JSON-RPC (`POST`, a request or a batch),
//!   forwarded to the environment with a fresh token. A sleeping environment
//!   wakes on the first call.
//! - `ws://127.0.0.1:<port + 1>`: subscriptions, relayed to the
//!   environment's WebSocket (port + 1 is the Solana convention web3
//!   clients assume). A relayed socket closes when its token expires; clients
//!   reconnect.
//!
//! The HTTP port is also the edge ([`super::edge`]): the environment's
//! chain, transaction, Program Read, stack and protocol routes in the shape
//! the SDKs call in production, so an app in dev mode needs no credential.
//! It answers JSON-RPC subscriptions on `/` as well.
//!
//! Tokens are minted from the account's key by [`EnvRpc`] and never leave
//! this process. Only this machine can connect, and browser pages only from
//! a loopback origin (a local app): every request must name the endpoint by
//! a loopback host, and a browser's `Origin` must be loopback too, so a page
//! on another site (or a rebound DNS name) cannot use the environment.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use arete_mcp::dev_env::{ws_url_of, EnvRpc};
use axum::body::Bytes;
use axum::extract::ws::WebSocketUpgrade;
use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio_tungstenite::tungstenite;

use super::edge::{self, Edge};
use super::runtime::LogSink;

/// The largest JSON-RPC body forwarded (the environment takes less).
const MAX_BODY: usize = 1024 * 1024;

/// The running endpoint.
pub struct RpcProxy {
    pub http_addr: SocketAddr,
    pub ws_addr: SocketAddr,
    shutdown: Option<tokio::sync::watch::Sender<bool>>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}

impl RpcProxy {
    pub fn rpc_url(&self) -> String {
        format!("http://{}", self.http_addr)
    }

    pub fn ws_url(&self) -> String {
        format!("ws://{}", self.ws_addr)
    }

    pub async fn stop(mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(true);
        }
        for task in self.tasks.drain(..) {
            task.abort();
            let _ = tokio::time::timeout(Duration::from_secs(2), task).await;
        }
    }
}

struct ProxyState {
    rpc: Arc<EnvRpc>,
    edge: Edge,
    http_port: u16,
    log: Option<LogSink>,
}

/// Whether `host` (a `Host` header value) is a loopback name with `port`.
pub fn loopback_host(host: &str, port: u16) -> bool {
    let host = host.trim().to_ascii_lowercase();
    ["127.0.0.1", "localhost", "[::1]"]
        .iter()
        .any(|name| host == format!("{name}:{port}"))
}

/// Whether `origin` is a page served from this machine (any port).
pub fn loopback_origin(origin: &str) -> bool {
    let origin = origin.trim().to_ascii_lowercase();
    let Some(rest) = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
    else {
        return false;
    };
    if rest.contains('/') || rest.contains('@') {
        return false;
    }
    let host = if let Some(bracketed) = rest.strip_prefix('[') {
        match bracketed.split_once(']') {
            Some((host, tail)) if tail.is_empty() || tail.starts_with(':') => host,
            _ => return false,
        }
    } else {
        rest.split(':').next().unwrap_or_default()
    };
    host == "localhost"
        || host == "::1"
        || host
            .parse::<std::net::Ipv4Addr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// Why a request may not use the endpoint, if it may not.
pub fn refusal(headers: &HeaderMap, port: u16) -> Option<&'static str> {
    let host = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if !loopback_host(host, port) {
        return Some("this endpoint only answers as 127.0.0.1 or localhost");
    }
    if let Some(origin) = headers.get(header::ORIGIN) {
        if !origin.to_str().is_ok_and(loopback_origin) {
            return Some("only pages served from this machine may use this endpoint");
        }
    }
    None
}

fn rpc_error(status: StatusCode, message: &str) -> Response {
    (
        status,
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        )],
        json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32081, "message": message } })
            .to_string(),
    )
        .into_response()
}

fn with_cors(mut response: Response, origin: Option<&HeaderValue>) -> Response {
    if let Some(origin) = origin {
        let headers = response.headers_mut();
        headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin.clone());
        headers.insert(header::VARY, HeaderValue::from_static("origin"));
        headers.insert(
            header::ACCESS_CONTROL_EXPOSE_HEADERS,
            HeaderValue::from_static("x-arete-dev-edge, x-error-code, retry-after"),
        );
    }
    response
}

/// Every answer says it comes from the edge.
fn from_edge(mut response: Response) -> Response {
    response.headers_mut().insert(
        header::HeaderName::from_static(edge::EDGE_HEADER),
        HeaderValue::from_static("1"),
    );
    response
}

async fn handle(
    State(state): State<Arc<ProxyState>>,
    upgrade: Option<WebSocketUpgrade>,
    request: Request,
) -> Response {
    let (parts, body) = request.into_parts();
    let origin = parts.headers.get(header::ORIGIN).cloned();
    if let Some(reason) = refusal(&parts.headers, state.http_port) {
        return from_edge(rpc_error(StatusCode::FORBIDDEN, reason));
    }
    if let Some(upgrade) = upgrade {
        return from_edge(socket(state, upgrade, parts.uri));
    }
    from_edge(with_cors(
        answer(&state, parts, body).await,
        origin.as_ref(),
    ))
}

/// A WebSocket on the HTTP port: a stack (`/stacks/<alias>`) or JSON-RPC
/// subscriptions (`/`).
fn socket(state: Arc<ProxyState>, upgrade: WebSocketUpgrade, uri: axum::http::Uri) -> Response {
    let path = uri.path().to_string();
    if path != "/" && !matches!(edge::route(&path), edge::Route::Stack { rest: "", .. }) {
        return edge::refusal(
            StatusCode::NOT_FOUND,
            "not-found",
            "No WebSocket is served on this path",
            "Open stack streams on /stacks/<liveAlias> and JSON-RPC subscriptions on /.",
        );
    }
    upgrade.on_upgrade(move |socket| async move {
        let relayed = match edge::route(&path) {
            edge::Route::Stack { alias, .. } => {
                state.edge.relay_stack(alias, uri.query(), socket).await
            }
            _ => state.edge.relay_rpc(socket).await,
        };
        if let (Err(error), Some(log)) = (relayed, &state.log) {
            log.line(&format!("edge socket {path}: {error:#}"));
        }
    })
}

async fn answer(
    state: &ProxyState,
    parts: axum::http::request::Parts,
    body: axum::body::Body,
) -> Response {
    match parts.method {
        Method::OPTIONS => {
            let mut response = StatusCode::NO_CONTENT.into_response();
            let headers = response.headers_mut();
            headers.insert(
                header::ACCESS_CONTROL_ALLOW_METHODS,
                HeaderValue::from_static("GET, POST, PUT, PATCH, DELETE, OPTIONS"),
            );
            headers.insert(
                header::ACCESS_CONTROL_ALLOW_HEADERS,
                HeaderValue::from_static(
                    "content-type, solana-client, authorization, x-arete-dev-identity",
                ),
            );
            headers.insert(
                header::ACCESS_CONTROL_MAX_AGE,
                HeaderValue::from_static("600"),
            );
            return response;
        }
        _ if edge::route(parts.uri.path()) != edge::Route::Rpc => {
            let bytes: Bytes = match axum::body::to_bytes(body, MAX_BODY).await {
                Ok(bytes) => bytes,
                Err(_) => return rpc_error(StatusCode::PAYLOAD_TOO_LARGE, "request too large"),
            };
            if let Some(response) = state
                .edge
                .handle(&parts.method, &parts.uri, &parts.headers, bytes)
                .await
            {
                return response;
            }
            return rpc_error(StatusCode::NOT_FOUND, "not found");
        }
        Method::GET => {
            // A health probe (`curl http://127.0.0.1:<port>/health`).
            let body = json!({ "ok": true, "environment": state.rpc.env() }).to_string();
            return (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/json")],
                body,
            )
                .into_response();
        }
        Method::POST => {}
        _ => return rpc_error(StatusCode::METHOD_NOT_ALLOWED, "POST JSON-RPC here"),
    }
    let json_body = parts
        .headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.to_ascii_lowercase().starts_with("application/json"));
    if !json_body {
        return rpc_error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "send JSON-RPC with Content-Type: application/json",
        );
    }
    let bytes: Bytes = match axum::body::to_bytes(body, MAX_BODY).await {
        Ok(bytes) => bytes,
        Err(_) => return rpc_error(StatusCode::PAYLOAD_TOO_LARGE, "request too large"),
    };
    match state.rpc.post_raw(&bytes).await {
        Ok((status, body, content_type)) => {
            let status = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY);
            let content_type = content_type.unwrap_or_else(|| "application/json".into());
            (status, [(header::CONTENT_TYPE, content_type)], body).into_response()
        }
        Err(error) => {
            if let Some(log) = &state.log {
                log.line(&format!("rpc proxy: {error:#}"));
            }
            rpc_error(StatusCode::BAD_GATEWAY, &format!("{error:#}"))
        }
    }
}

/// Serve the environment on `http` (JSON-RPC) and `ws` (subscriptions),
/// both bound to loopback.
pub async fn start(
    http: std::net::TcpListener,
    ws: std::net::TcpListener,
    rpc: Arc<EnvRpc>,
    log: Option<LogSink>,
) -> Result<RpcProxy> {
    let http_addr = http.local_addr()?;
    let ws_addr = ws.local_addr()?;
    if !http_addr.ip().is_loopback() || !ws_addr.ip().is_loopback() {
        bail!("the local RPC endpoint must listen on loopback");
    }
    http.set_nonblocking(true)?;
    ws.set_nonblocking(true)?;
    let http = tokio::net::TcpListener::from_std(http)?;
    let ws = tokio::net::TcpListener::from_std(ws)?;
    let state = Arc::new(ProxyState {
        edge: Edge::new(rpc.clone()),
        rpc,
        http_port: http_addr.port(),
        log,
    });
    let app = Router::new()
        .fallback(handle)
        .layer(DefaultBodyLimit::max(MAX_BODY))
        .with_state(state.clone());
    let (shutdown, mut stopped) = tokio::sync::watch::channel(false);
    let mut http_stopped = stopped.clone();
    let http_task = tokio::spawn(async move {
        let _ = axum::serve(http, app)
            .with_graceful_shutdown(async move {
                let _ = http_stopped.changed().await;
            })
            .await;
    });
    let ws_task = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = stopped.changed() => break,
                accepted = ws.accept() => {
                    let Ok((stream, _)) = accepted else { continue };
                    let state = state.clone();
                    tokio::spawn(async move {
                        if let Err(error) = relay(stream, &state, ws_addr.port()).await {
                            if let Some(log) = &state.log {
                                log.line(&format!("ws proxy: {error:#}"));
                            }
                        }
                    });
                }
            }
        }
    });
    Ok(RpcProxy {
        http_addr,
        ws_addr,
        shutdown: Some(shutdown),
        tasks: vec![http_task, ws_task],
    })
}

/// Accept one WebSocket (after the same checks as HTTP), open the
/// environment's with a token, and relay both ways until either closes.
async fn relay(stream: tokio::net::TcpStream, state: &ProxyState, port: u16) -> Result<()> {
    #[allow(clippy::result_large_err)] // tungstenite's callback signature
    let check = |request: &tungstenite::handshake::server::Request,
                 response: tungstenite::handshake::server::Response| {
        match refusal(request.headers(), port) {
            None => Ok(response),
            Some(reason) => {
                let mut refused =
                    tungstenite::handshake::server::ErrorResponse::new(Some(reason.to_string()));
                *refused.status_mut() = StatusCode::FORBIDDEN;
                Err(refused)
            }
        }
    };
    let downstream = tokio_tungstenite::accept_hdr_async(stream, check)
        .await
        .context("WebSocket handshake failed")?;
    let token = state.rpc.token(false).await?;
    let mut request =
        tungstenite::client::IntoClientRequest::into_client_request(ws_url_of(&token.rpc_url))
            .context("The environment's WebSocket URL is invalid")?;
    request.headers_mut().insert(
        header::AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {}", token.token))
            .context("The environment token is not a header value")?,
    );
    let (upstream, _) = tokio_tungstenite::connect_async(request)
        .await
        .map_err(|error| anyhow::anyhow!("The environment's WebSocket refused: {error}"))?;
    let (mut down_tx, mut down_rx) = downstream.split();
    let (mut up_tx, mut up_rx) = upstream.split();
    let to_env = async {
        while let Some(Ok(message)) = down_rx.next().await {
            let closing = message.is_close();
            if up_tx.send(message).await.is_err() || closing {
                break;
            }
        }
        let _ = up_tx.close().await;
    };
    let from_env = async {
        while let Some(Ok(message)) = up_rx.next().await {
            let closing = message.is_close();
            if down_tx.send(message).await.is_err() || closing {
                break;
            }
        }
        let _ = down_tx.close().await;
    };
    tokio::select! {
        _ = to_env => {}
        _ = from_env => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn only_loopback_hosts_and_origins_pass() {
        assert!(loopback_host("127.0.0.1:8899", 8899));
        assert!(loopback_host("LOCALHOST:8899", 8899));
        assert!(loopback_host("[::1]:8899", 8899));
        assert!(!loopback_host("127.0.0.1:8900", 8899));
        assert!(!loopback_host("evil.example:8899", 8899));
        assert!(!loopback_host("127.0.0.1", 8899));
        for origin in [
            "http://localhost:3000",
            "http://127.0.0.1:5173",
            "https://localhost",
            "http://[::1]:3000",
            "http://127.0.0.2",
        ] {
            assert!(loopback_origin(origin), "{origin}");
        }
        for origin in [
            "https://evil.example",
            "http://localhost.evil.example",
            "http://127.0.0.1.evil.example:80",
            "null",
            "file://",
            "http://user@localhost:3000",
        ] {
            assert!(!loopback_origin(origin), "{origin}");
        }
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_static("127.0.0.1:9"));
        assert_eq!(refusal(&headers, 9), None);
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("https://evil.example"),
        );
        assert!(refusal(&headers, 9).is_some());
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("http://localhost:3000"),
        );
        assert_eq!(refusal(&headers, 9), None);
        headers.insert(header::HOST, HeaderValue::from_static("rebound.example:9"));
        assert!(refusal(&headers, 9).is_some());
    }

    /// An environment WebSocket that echoes text and remembers the
    /// `Authorization` it was opened with, and an API whose token route
    /// names it.
    async fn fake_environment() -> (String, Arc<std::sync::Mutex<Vec<String>>>) {
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let ws = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let ws_port = ws.local_addr().unwrap().port();
        let auth = seen.clone();
        tokio::spawn(async move {
            while let Ok((stream, _)) = ws.accept().await {
                let auth = auth.clone();
                tokio::spawn(async move {
                    #[allow(clippy::result_large_err)] // tungstenite's callback signature
                    let record = |request: &tungstenite::handshake::server::Request,
                                  response: tungstenite::handshake::server::Response| {
                        auth.lock().unwrap().push(
                            request
                                .headers()
                                .get("authorization")
                                .and_then(|value| value.to_str().ok())
                                .unwrap_or_default()
                                .to_string(),
                        );
                        Ok(response)
                    };
                    let Ok(mut socket) = tokio_tungstenite::accept_hdr_async(stream, record).await
                    else {
                        return;
                    };
                    while let Some(Ok(message)) = socket.next().await {
                        if message.is_text() {
                            let _ = socket.send(message).await;
                        }
                    }
                });
            }
        });
        let app = axum::Router::new().route(
            "/api/dev/environments/:env/token",
            axum::routing::post(move || async move {
                axum::Json(json!({
                    "token": "env-token",
                    "expiresAt": 4_000_000_000i64,
                    "rpcUrl": format!("http://127.0.0.1:{ws_port}/rpc/v1/devenv/x"),
                }))
            }),
        );
        let api = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", api.local_addr().unwrap());
        tokio::spawn(async move {
            let _ = axum::serve(api, app).await;
        });
        (origin, seen)
    }

    #[tokio::test]
    async fn relays_websockets_with_the_token_and_refuses_foreign_pages() {
        let (origin, seen) = fake_environment().await;
        let api =
            arete_mcp::dev_env::EnvApi::new(reqwest::Client::new(), &origin, "a4_sk_x").unwrap();
        let rpc = Arc::new(EnvRpc::new(api, "denv_01hzx3k8m9q2r4s5t6v7w8x9yz").unwrap());
        let proxy = start(
            std::net::TcpListener::bind("127.0.0.1:0").unwrap(),
            std::net::TcpListener::bind("127.0.0.1:0").unwrap(),
            rpc,
            None,
        )
        .await
        .unwrap();

        let (mut socket, _) = tokio_tungstenite::connect_async(proxy.ws_url())
            .await
            .unwrap();
        socket
            .send(tungstenite::Message::text("{\"method\":\"slotSubscribe\"}"))
            .await
            .unwrap();
        let echoed = tokio::time::timeout(Duration::from_secs(10), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(
            echoed.into_text().unwrap(),
            "{\"method\":\"slotSubscribe\"}"
        );
        assert_eq!(seen.lock().unwrap().as_slice(), ["Bearer env-token"]);

        // A page from another site is refused at the handshake.
        let mut request =
            tungstenite::client::IntoClientRequest::into_client_request(proxy.ws_url()).unwrap();
        request.headers_mut().insert(
            header::ORIGIN,
            HeaderValue::from_static("https://evil.example"),
        );
        assert!(tokio_tungstenite::connect_async(request).await.is_err());
        assert_eq!(seen.lock().unwrap().len(), 1);
        proxy.stop().await;
    }

    /// An Arete API and its dev targets on one loopback origin: resolve
    /// answers a target for every surface but the stack, sessions mint
    /// `session-token`, and every other request is echoed with the path,
    /// query and `Authorization` it arrived with.
    async fn fake_api(stack_ws: Option<String>) -> (String, Arc<std::sync::Mutex<Vec<Value>>>) {
        use axum::routing::post;
        let minted = Arc::new(std::sync::Mutex::new(Vec::new()));
        let api = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", api.local_addr().unwrap());
        let base = origin.clone();
        let sessions = minted.clone();
        let app = axum::Router::new()
            .route(
                "/api/dev/environments/:env/token",
                post({
                    let base = base.clone();
                    move || async move {
                        axum::Json(json!({
                            "token": "env-token",
                            "expiresAt": 4_000_000_000i64,
                            "rpcUrl": format!("{base}/rpc/v1/devenv/x"),
                        }))
                    }
                }),
            )
            .route(
                "/api/dev/environments/:env/resolve",
                post({
                    let base = base.clone();
                    let stack_ws = stack_ws.clone();
                    move |axum::extract::Path(env): axum::extract::Path<String>,
                          axum::Json(request): axum::Json<Value>| async move {
                        let mut answer = json!({ "environment": { "id": env, "name": "app" } });
                        if request.get("chain").is_some() {
                            answer["chain"] = json!({ "status": "target", "endpoint": format!("{base}/devenv/v1/{env}") });
                        }
                        if request.get("programReads").is_some() {
                            answer["programReads"] = json!([{
                                "status": "target", "endpoint": base,
                                "programReleaseHash": "served", "releaseDiffers": true,
                            }]);
                        }
                        if request.get("stacks").is_some() {
                            answer["stacks"] = match &stack_ws {
                                Some(url) => json!([{
                                    "status": "target", "websocketUrl": url, "httpUrl": "",
                                    "stackManifestHash": "served", "releaseDiffers": true,
                                    "source": "override", "loadedAtSlot": 1,
                                }]),
                                None => json!([{
                                    "status": "refused", "code": "devenv-stack-not-loaded",
                                    "message": "not loaded", "fix": "Run `a4 dev up`",
                                }]),
                            };
                        }
                        if request.get("protocols").is_some() {
                            answer["protocols"] = json!([{
                                "status": "target", "deploymentId": "env-dep",
                                "apiUrl": format!("{base}/api"), "sessionEndpoint": format!("{base}/ws/sessions"),
                                "protocolReleaseHash": "r", "releaseDiffers": false,
                            }]);
                        }
                        axum::Json(answer)
                    }
                }),
            )
            .route(
                "/api/protocol-instances/:id",
                axum::routing::get(
                    |axum::extract::Path(id): axum::extract::Path<String>| async move {
                        let deployment = if id == "i-env" { "env-dep" } else { "prod-dep" };
                        axum::Json(json!({ "id": id, "deploymentId": deployment }))
                    },
                ),
            )
            .route(
                "/ws/sessions",
                post(move |axum::Json(request): axum::Json<Value>| async move {
                    sessions.lock().unwrap().push(request);
                    axum::Json(json!({ "token": "session-token", "expires_at": 4_000_000_000i64 }))
                }),
            )
            .fallback(|request: Request| async move {
                let auth = request
                    .headers()
                    .get(header::AUTHORIZATION)
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or_default()
                    .to_string();
                axum::Json(json!({
                    "path": request.uri().path(),
                    "query": request.uri().query(),
                    "auth": auth,
                }))
            });
        tokio::spawn(async move {
            let _ = axum::serve(api, app).await;
        });
        (origin, minted)
    }

    #[tokio::test]
    async fn the_edge_forwards_each_surface_with_its_own_credential() {
        let (origin, minted) = fake_api(None).await;
        let env = "denv_01hzx3k8m9q2r4s5t6v7w8x9yz";
        let api =
            arete_mcp::dev_env::EnvApi::new(reqwest::Client::new(), &origin, "a4_sk_x").unwrap();
        let rpc = Arc::new(EnvRpc::new(api, env).unwrap());
        let proxy = start(
            std::net::TcpListener::bind("127.0.0.1:0").unwrap(),
            std::net::TcpListener::bind("127.0.0.1:0").unwrap(),
            rpc,
            None,
        )
        .await
        .unwrap();
        let edge = proxy.rpc_url();
        let client = reqwest::Client::new();
        let get = |path: &str, identity: Option<Value>| {
            let mut request = client
                .get(format!("{edge}{path}"))
                .header("authorization", "Bearer from-the-page")
                .header("origin", "http://localhost:5173");
            if let Some(identity) = identity {
                request = request.header(edge::IDENTITY_HEADER, identity.to_string());
            }
            request.send()
        };
        let read = |response: reqwest::Response| async move {
            assert_eq!(
                response.headers().get(edge::EDGE_HEADER).unwrap(),
                "1",
                "every answer says it is the edge"
            );
            let status = response.status().as_u16();
            (status, response.json::<Value>().await.unwrap())
        };

        let (status, body) = read(get(edge::WELL_KNOWN_PATH, None).await.unwrap()).await;
        assert_eq!((status, body), (200, json!({ "edge": 1 })));

        // Chain reads: the environment's gateway prefix, the environment
        // token, never the page's credential.
        let (status, body) = read(
            get(
                "/chain/v1/account?address=abc",
                Some(json!({ "chain": { "solanaGatewayBindingId": "sgb_1" } })),
            )
            .await
            .unwrap(),
        )
        .await;
        assert_eq!(status, 200);
        assert_eq!(body["path"], format!("/devenv/v1/{env}/chain/v1/account"));
        assert_eq!(body["query"], "address=abc");
        assert_eq!(body["auth"], "Bearer env-token");

        // Program Read: the served release in the path, a session minted
        // for the installed release and the environment.
        let (status, body) = read(
            get(
                "/v1/releases/installed/accounts/Reserve/x",
                Some(json!({ "programReads": [{
                    "programId": "P", "programReadBindingId": "prb_1", "programReleaseHash": "installed",
                }] })),
            )
            .await
            .unwrap(),
        )
        .await;
        assert_eq!(status, 200);
        assert_eq!(body["path"], "/v1/releases/served/accounts/Reserve/x");
        assert_eq!(body["auth"], "Bearer session-token");
        assert_eq!(
            minted.lock().unwrap().as_slice(),
            [json!({
                "targetKind": "program-read-binding", "targetId": "prb_1",
                "programReleaseHash": "installed", "scopes": ["read"], "environment": env,
            })]
        );

        // A protocol named by package before its first publish: the
        // environment's deployment, with the account's key.
        let (status, body) = read(
            get(
                "/api/protocol-deployments/package:token-raise/instances",
                None,
            )
            .await
            .unwrap(),
        )
        .await;
        assert_eq!(status, 200);
        assert_eq!(body["path"], "/api/protocol-deployments/env-dep/instances");
        assert_eq!(body["auth"], "Bearer a4_sk_x");

        // Instances, intents and receipts are named by their own id:
        // forwarded unchanged, with the account key, when they belong to
        // the environment's deployment...
        let (status, body) = read(
            get("/api/protocol-instances/i-env/metadata", None)
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, 200);
        assert_eq!(body["path"], "/api/protocol-instances/i-env/metadata");
        assert_eq!(body["auth"], "Bearer a4_sk_x");
        // ...and refused when they belong to production's.
        let (status, body) = read(
            get("/api/protocol-instances/i-prod/metadata", None)
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(
            (status, body["code"].clone()),
            (403, json!("environment-token-refused"))
        );
        let (status, _) = read(get("/api/protocol-receipts/keys", None).await.unwrap()).await;
        assert_eq!(status, 200);

        // Refusals carry the code and the fix, and go nowhere else.
        let (status, body) = read(
            get("/stacks/default/views?stackManifestHash=h", None)
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, 409);
        assert_eq!(body["code"], "devenv-stack-not-loaded");
        assert_eq!(body["fix"], "Run `a4 dev up`");
        let (status, body) = read(get("/chain/v1/account", None).await.unwrap()).await;
        assert_eq!(
            (status, body["code"].clone()),
            (503, json!("devenv-surface-unavailable"))
        );
        let (status, body) = read(get("/api/keys", None).await.unwrap()).await;
        assert_eq!(
            (status, body["code"].clone()),
            (503, json!("devenv-surface-unavailable"))
        );

        // Another site's page is refused on every route.
        let refused = client
            .get(format!("{edge}{}", edge::WELL_KNOWN_PATH))
            .header("origin", "https://evil.example")
            .send()
            .await
            .unwrap();
        assert_eq!(refused.status().as_u16(), 403);
        proxy.stop().await;
    }

    #[tokio::test]
    async fn the_edge_relays_a_stack_with_a_session_for_the_environment() {
        let log_dir = tempfile::tempdir().unwrap();
        let (ws_url, seen) = echo_socket().await;
        let (origin, minted) = fake_api(Some(ws_url)).await;
        let env = "denv_01hzx3k8m9q2r4s5t6v7w8x9yz";
        let api =
            arete_mcp::dev_env::EnvApi::new(reqwest::Client::new(), &origin, "a4_sk_x").unwrap();
        let rpc = Arc::new(EnvRpc::new(api, env).unwrap());
        let proxy = start(
            std::net::TcpListener::bind("127.0.0.1:0").unwrap(),
            std::net::TcpListener::bind("127.0.0.1:0").unwrap(),
            rpc,
            Some(LogSink::open(&log_dir.path().join("a4.log"), "a4", false).unwrap()),
        )
        .await
        .unwrap();
        let url = format!(
            "ws://{}/stacks/default?stackManifestHash=installed&view=Reserve",
            proxy.http_addr
        );
        let mut request = tungstenite::client::IntoClientRequest::into_client_request(url).unwrap();
        request.headers_mut().insert(
            header::ORIGIN,
            HeaderValue::from_static("http://localhost:5173"),
        );
        let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
        socket
            .send(tungstenite::Message::text("{\"subscribe\":\"Reserve\"}"))
            .await
            .unwrap();
        let echoed = tokio::time::timeout(Duration::from_secs(10), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap_or_else(|error| {
                panic!(
                    "{error}: {}",
                    std::fs::read_to_string(log_dir.path().join("a4.log")).unwrap_or_default()
                )
            });
        assert_eq!(echoed.into_text().unwrap(), "{\"subscribe\":\"Reserve\"}");
        // The stack session names the served manifest and the environment,
        // and the socket opened with it, keeping the other query parameters.
        assert_eq!(
            minted.lock().unwrap().as_slice(),
            [json!({
                "websocket_url": seen.url(), "scopes": ["read"],
                "stackManifestHash": "served", "liveAlias": "default", "environment": env,
            })]
        );
        assert_eq!(
            seen.opened(),
            [(
                "Bearer session-token".to_string(),
                Some("view=Reserve".to_string())
            )]
        );
        proxy.stop().await;
    }

    /// A WebSocket that echoes text and remembers each opening's
    /// `Authorization` and query.
    struct Seen {
        url: String,
        opened: Openings,
    }

    /// Each opening's `Authorization` and query.
    type Openings = Arc<std::sync::Mutex<Vec<(String, Option<String>)>>>;

    impl Seen {
        fn url(&self) -> &str {
            &self.url
        }
        fn opened(&self) -> Vec<(String, Option<String>)> {
            self.opened.lock().unwrap().clone()
        }
    }

    async fn echo_socket() -> (String, Seen) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let opened = Arc::new(std::sync::Mutex::new(Vec::new()));
        let record = opened.clone();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let record = record.clone();
                tokio::spawn(async move {
                    #[allow(clippy::result_large_err)] // tungstenite's callback signature
                    let check = |request: &tungstenite::handshake::server::Request,
                                 response: tungstenite::handshake::server::Response| {
                        record.lock().unwrap().push((
                            request
                                .headers()
                                .get("authorization")
                                .and_then(|value| value.to_str().ok())
                                .unwrap_or_default()
                                .to_string(),
                            request.uri().query().map(str::to_string),
                        ));
                        Ok(response)
                    };
                    let Ok(mut socket) = tokio_tungstenite::accept_hdr_async(stream, check).await
                    else {
                        return;
                    };
                    while let Some(Ok(message)) = socket.next().await {
                        if message.is_text() {
                            let _ = socket.send(message).await;
                        }
                    }
                });
            }
        });
        (url.clone(), Seen { url, opened })
    }
}
