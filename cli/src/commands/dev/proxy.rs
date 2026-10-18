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
use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio_tungstenite::tungstenite;

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
    }
    response
}

async fn handle(State(state): State<Arc<ProxyState>>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let origin = parts.headers.get(header::ORIGIN).cloned();
    if let Some(reason) = refusal(&parts.headers, state.http_port) {
        return rpc_error(StatusCode::FORBIDDEN, reason);
    }
    match parts.method {
        Method::OPTIONS => {
            let mut response = StatusCode::NO_CONTENT.into_response();
            let headers = response.headers_mut();
            headers.insert(
                header::ACCESS_CONTROL_ALLOW_METHODS,
                HeaderValue::from_static("POST, GET, OPTIONS"),
            );
            headers.insert(
                header::ACCESS_CONTROL_ALLOW_HEADERS,
                HeaderValue::from_static("content-type, solana-client"),
            );
            headers.insert(
                header::ACCESS_CONTROL_MAX_AGE,
                HeaderValue::from_static("600"),
            );
            return with_cors(response, origin.as_ref());
        }
        Method::GET => {
            // A health probe (`curl http://127.0.0.1:<port>/health`).
            let body = json!({ "ok": true, "environment": state.rpc.env() }).to_string();
            return with_cors(
                (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "application/json")],
                    body,
                )
                    .into_response(),
                origin.as_ref(),
            );
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
    let response = match state.rpc.post_raw(&bytes).await {
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
    };
    with_cors(response, origin.as_ref())
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
}
