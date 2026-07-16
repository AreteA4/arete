use crate::http::transactions::{self, TransactionState};
use crate::{config::TransactionConfig, health::HealthMonitor, ProgramAccountReaderFn};
use anyhow::Result;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use dashmap::DashMap;
use http_body_util::BodyExt;
use http_body_util::Full;
use hyper::body::Bytes;
use hyper::header::{
    HeaderValue, ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_METHODS,
    ACCESS_CONTROL_ALLOW_ORIGIN, ACCESS_CONTROL_EXPOSE_HEADERS, ACCESS_CONTROL_MAX_AGE,
};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};
use std::convert::Infallible;
use std::env;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::net::TcpListener;
use tracing::{error, info};

use crate::websocket::auth::{AuthDecision, AuthDeny, ConnectionAuthRequest, WebSocketAuthPlugin};

/// Configuration for the HTTP health server
#[derive(Clone, Debug)]
pub struct HttpHealthConfig {
    pub bind_address: SocketAddr,
}

impl Default for HttpHealthConfig {
    fn default() -> Self {
        Self {
            bind_address: "[::]:8081".parse().expect("valid socket address"),
        }
    }
}

impl HttpHealthConfig {
    pub fn new(bind_address: impl Into<SocketAddr>) -> Self {
        Self {
            bind_address: bind_address.into(),
        }
    }
}

#[derive(Clone)]
struct HttpRequestState {
    health_monitor: Arc<Option<HealthMonitor>>,
    rpc_url: Arc<Option<String>>,
    rpc_client: Client,
    program_account_reader: Arc<Option<ProgramAccountReaderFn>>,
    auth_plugin: Arc<Option<Arc<dyn WebSocketAuthPlugin>>>,
    limit_state: Arc<HttpLimitState>,
    transaction_state: Arc<Option<TransactionState>>,
}

/// HTTP server that exposes health endpoints
pub struct HttpHealthServer {
    bind_addr: SocketAddr,
    health_monitor: Option<HealthMonitor>,
    program_account_reader: Option<ProgramAccountReaderFn>,
    auth_plugin: Option<Arc<dyn WebSocketAuthPlugin>>,
    transaction_config: Option<TransactionConfig>,
    #[cfg(feature = "otel")]
    metrics: Option<Arc<crate::metrics::Metrics>>,
}

impl HttpHealthServer {
    pub fn new(bind_addr: SocketAddr) -> Self {
        Self {
            bind_addr,
            health_monitor: None,
            program_account_reader: None,
            auth_plugin: None,
            transaction_config: None,
            #[cfg(feature = "otel")]
            metrics: None,
        }
    }

    pub fn with_health_monitor(mut self, monitor: HealthMonitor) -> Self {
        self.health_monitor = Some(monitor);
        self
    }

    pub fn with_program_account_reader(mut self, reader: ProgramAccountReaderFn) -> Self {
        self.program_account_reader = Some(reader);
        self
    }

    pub fn with_auth_plugin(mut self, plugin: Arc<dyn WebSocketAuthPlugin>) -> Self {
        self.auth_plugin = Some(plugin);
        self
    }

    pub fn with_transaction_config(mut self, config: TransactionConfig) -> Self {
        self.transaction_config = Some(config);
        self
    }

    #[cfg(feature = "otel")]
    pub fn with_metrics(mut self, metrics: Option<Arc<crate::metrics::Metrics>>) -> Self {
        self.metrics = metrics;
        self
    }

    pub async fn start(self) -> Result<()> {
        info!("Starting HTTP health server on {}", self.bind_addr);

        let listener = TcpListener::bind(&self.bind_addr).await?;
        info!("HTTP health server listening on {}", self.bind_addr);

        let transaction_state = self
            .transaction_config
            .filter(|config| config.enabled)
            .map(TransactionState::new)
            .transpose()?;
        #[cfg(feature = "otel")]
        let transaction_state = transaction_state.map(|state| state.with_metrics(self.metrics));
        let request_state = HttpRequestState {
            health_monitor: Arc::new(self.health_monitor),
            rpc_url: Arc::new(resolve_rpc_url()),
            rpc_client: Client::builder().build()?,
            program_account_reader: Arc::new(self.program_account_reader),
            auth_plugin: Arc::new(self.auth_plugin),
            limit_state: Arc::new(HttpLimitState::default()),
            transaction_state: Arc::new(transaction_state),
        };

        loop {
            match listener.accept().await {
                Ok((stream, remote_addr)) => {
                    let io = TokioIo::new(stream);
                    let request_state = request_state.clone();

                    tokio::spawn(async move {
                        let service = service_fn(move |req| {
                            let request_state = request_state.clone();
                            async move { handle_request(remote_addr, req, request_state).await }
                        });

                        if let Err(e) = http1::Builder::new().serve_connection(io, service).await {
                            error!("HTTP connection error: {}", e);
                        }
                    });
                }
                Err(e) => {
                    error!("Failed to accept HTTP connection: {}", e);
                }
            }
        }
    }
}

async fn handle_request(
    remote_addr: SocketAddr,
    req: Request<hyper::body::Incoming>,
    state: HttpRequestState,
) -> Result<Response<Full<Bytes>>, Infallible> {
    if req.method() == Method::OPTIONS {
        return Ok(with_cors(
            Response::builder()
                .status(StatusCode::NO_CONTENT)
                .body(Full::new(Bytes::new()))
                .unwrap(),
        ));
    }

    let response = handle_request_inner(remote_addr, req, state).await?;
    Ok(with_cors(response))
}

fn with_cors(mut response: Response<Full<Bytes>>) -> Response<Full<Bytes>> {
    let headers = response.headers_mut();
    headers.insert(ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
    headers.insert(
        ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, POST, OPTIONS"),
    );
    headers.insert(
        ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("Authorization, Content-Type"),
    );
    headers.insert(
        ACCESS_CONTROL_EXPOSE_HEADERS,
        HeaderValue::from_static(
            "Retry-After, X-Error-Code, X-Request-Id, X-Arete-Upstream-Attempted",
        ),
    );
    headers.insert(ACCESS_CONTROL_MAX_AGE, HeaderValue::from_static("86400"));
    response
}

async fn handle_request_inner(
    remote_addr: SocketAddr,
    req: Request<hyper::body::Incoming>,
    state: HttpRequestState,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let HttpRequestState {
        health_monitor,
        rpc_url,
        rpc_client,
        program_account_reader,
        auth_plugin,
        limit_state,
        transaction_state,
    } = state;
    let path = req.uri().path().to_string();

    match path.as_str() {
        "/health" | "/healthz" => {
            // Basic health check - server is running
            Ok(Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "text/plain")
                .body(Full::new(Bytes::from("OK")))
                .unwrap())
        }
        "/ready" | "/readiness" => {
            // Readiness check - check if stream is healthy
            if let Some(monitor) = health_monitor.as_ref() {
                if monitor.is_healthy().await {
                    Ok(Response::builder()
                        .status(StatusCode::OK)
                        .header("Content-Type", "text/plain")
                        .body(Full::new(Bytes::from("READY")))
                        .unwrap())
                } else {
                    Ok(Response::builder()
                        .status(StatusCode::SERVICE_UNAVAILABLE)
                        .header("Content-Type", "text/plain")
                        .body(Full::new(Bytes::from("NOT READY")))
                        .unwrap())
                }
            } else {
                // No health monitor configured, assume ready
                Ok(Response::builder()
                    .status(StatusCode::OK)
                    .header("Content-Type", "text/plain")
                    .body(Full::new(Bytes::from("READY")))
                    .unwrap())
            }
        }
        "/status" => {
            // Detailed status endpoint
            if let Some(monitor) = health_monitor.as_ref() {
                let status = monitor.status().await;
                let error_count = monitor.error_count().await;
                let is_healthy = monitor.is_healthy().await;

                let status_json = serde_json::json!({
                    "healthy": is_healthy,
                    "status": format!("{:?}", status),
                    "error_count": error_count
                });

                let status_code = if is_healthy {
                    StatusCode::OK
                } else {
                    StatusCode::SERVICE_UNAVAILABLE
                };

                Ok(Response::builder()
                    .status(status_code)
                    .header("Content-Type", "application/json")
                    .body(Full::new(Bytes::from(status_json.to_string())))
                    .unwrap())
            } else {
                let status_json = serde_json::json!({
                    "healthy": true,
                    "status": "no_monitor",
                    "error_count": 0
                });

                Ok(Response::builder()
                    .status(StatusCode::OK)
                    .header("Content-Type", "application/json")
                    .body(Full::new(Bytes::from(status_json.to_string())))
                    .unwrap())
            }
        }
        _ if path.starts_with("/transactions/") => {
            let Some(transaction_state) = transaction_state.as_ref() else {
                return Ok(error_response(StatusCode::NOT_FOUND, "Not Found"));
            };
            let client_addr = transaction_state.client_addr(remote_addr, req.headers());
            let auth_context = match authorize_http_request(
                client_addr,
                &req,
                auth_plugin.as_ref().as_ref(),
                &limit_state,
                None,
                false,
            )
            .await
            {
                Ok(context) => context,
                Err(response) => return Ok(transaction_auth_error(response, path.as_str())),
            };
            Ok(
                transactions::handle(client_addr, req, auth_context, transaction_state.clone())
                    .await,
            )
        }
        _ if path.starts_with("/chain/") => {
            let auth_context = match authorize_http_request(
                remote_addr,
                &req,
                auth_plugin.as_ref().as_ref(),
                &limit_state,
                Some("read"),
                true,
            )
            .await
            {
                Ok(context) => context,
                Err(response) => return Ok(response),
            };
            Ok(handle_chain_request(req, path.as_str(), rpc_url, rpc_client, auth_context).await)
        }
        _ if path.starts_with("/programs/") => {
            let auth_context = match authorize_http_request(
                remote_addr,
                &req,
                auth_plugin.as_ref().as_ref(),
                &limit_state,
                Some("read"),
                true,
            )
            .await
            {
                Ok(context) => context,
                Err(response) => return Ok(response),
            };
            Ok(handle_program_account_request(
                req,
                path.as_str(),
                rpc_url,
                rpc_client,
                program_account_reader,
                auth_context,
            )
            .await)
        }
        _ => Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header("Content-Type", "text/plain")
            .body(Full::new(Bytes::from("Not Found")))
            .unwrap()),
    }
}

fn transaction_auth_error(response: Response<Full<Bytes>>, path: &str) -> Response<Full<Bytes>> {
    let status = response.status();
    let code = response
        .headers()
        .get("X-Error-Code")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("authentication_failed")
        .to_string();
    let request_id = uuid::Uuid::new_v4().to_string();
    let mut value = json!({
        "code": code,
        "message": "Transaction request authentication failed",
        "retryable": status == StatusCode::TOO_MANY_REQUESTS || status == StatusCode::UNAUTHORIZED,
        "requestId": request_id,
    });
    if path == "/transactions/v1/send" {
        value["submissionState"] = json!("not_submitted");
    }
    Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .header("X-Error-Code", code)
        .header("X-Request-Id", request_id)
        .header("X-Arete-Upstream-Attempted", "false")
        .body(Full::new(Bytes::from(value.to_string())))
        .expect("valid transaction authentication response")
}

#[derive(Debug, Deserialize)]
struct AddressesBody {
    addresses: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct BalanceBody {
    owner: String,
    mint: String,
    #[serde(default, rename = "tokenProgram")]
    token_program: Option<String>,
    #[serde(default, rename = "minContextSlot")]
    min_context_slot: Option<String>,
}

#[derive(Debug, Deserialize)]
struct NativeBalanceBody {
    address: String,
    #[serde(default, rename = "minContextSlot")]
    min_context_slot: Option<String>,
}

#[derive(Default)]
struct HttpLimitState {
    per_subject_per_minute: DashMap<String, (u64, u32)>,
}

fn resolve_rpc_url() -> Option<String> {
    ["ARETE_READ_RPC_URL", "SOLANA_RPC_URL", "RPC_URL"]
        .iter()
        .find_map(|key| env::var(key).ok())
        .filter(|value| !value.is_empty())
}

fn json_response(status: StatusCode, value: Value) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .body(Full::new(Bytes::from(value.to_string())))
        .unwrap()
}

fn error_response(status: StatusCode, message: impl Into<String>) -> Response<Full<Bytes>> {
    json_response(status, json!({ "error": message.into() }))
}

fn parse_min_context_slot(
    value: Option<&str>,
) -> std::result::Result<Option<u64>, Response<Full<Bytes>>> {
    value
        .map(|slot| {
            slot.parse::<u64>().map_err(|_| {
                error_response(
                    StatusCode::BAD_REQUEST,
                    "minContextSlot must be a decimal u64 string",
                )
            })
        })
        .transpose()
}

fn auth_deny_response(deny: &AuthDeny) -> Response<Full<Bytes>> {
    let mut builder = Response::builder()
        .status(deny.http_status)
        .header("Content-Type", "application/json")
        .header("X-Error-Code", deny.code.as_str());

    if let Some(reset_at) = deny.reset_at {
        if let Ok(duration) = reset_at.duration_since(SystemTime::now()) {
            builder = builder.header("Retry-After", duration.as_secs().to_string());
        }
    }

    builder
        .body(Full::new(Bytes::from(
            json!({
                "error": deny.reason,
                "message": deny.reason,
                "code": deny.code.as_str(),
                "retryable": deny.code.should_retry(),
                "fatal": !deny.code.should_retry() && !deny.code.should_refresh_token(),
            })
            .to_string(),
        )))
        .unwrap()
}

async fn authorize_http_request(
    remote_addr: SocketAddr,
    req: &Request<hyper::body::Incoming>,
    auth_plugin: Option<&Arc<dyn WebSocketAuthPlugin>>,
    limit_state: &HttpLimitState,
    required_scope: Option<&str>,
    enforce_read_limits: bool,
) -> std::result::Result<Option<crate::websocket::auth::AuthContext>, Response<Full<Bytes>>> {
    let Some(plugin) = auth_plugin else {
        return Ok(None);
    };

    let mut auth_request = ConnectionAuthRequest::from_http_request(remote_addr, req);
    // HTTP reads are bearer-only. Do not allow query-param session tokens here.
    auth_request.query = None;
    let decision = plugin.authorize(&auth_request).await;
    let context = match decision {
        AuthDecision::Allow(context) => context,
        AuthDecision::Deny(deny) => return Err(auth_deny_response(&deny)),
    };

    if let Some(required_scope) = required_scope {
        if !context.has_scope(required_scope) {
            return Err(json_response(
                StatusCode::FORBIDDEN,
                json!({
                    "error": "insufficient_scope",
                    "message": format!("Required scope: {required_scope}"),
                    "code": "insufficient_scope",
                    "retryable": false,
                    "fatal": true
                }),
            ));
        }
    }

    if enforce_read_limits {
        enforce_http_limits(&context, limit_state).map_err(|deny| auth_deny_response(&deny))?;
    }
    Ok(Some(context))
}

fn enforce_http_limits(
    context: &crate::websocket::auth::AuthContext,
    limit_state: &HttpLimitState,
) -> std::result::Result<(), Box<AuthDeny>> {
    let Some(limit) = context.limits.max_http_requests_per_minute else {
        return Ok(());
    };

    let now_bucket = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::from_secs(0))
        .as_secs()
        / 60;
    let key = format!("{}:{}", context.subject, context.metering_key);
    let mut entry = limit_state
        .per_subject_per_minute
        .entry(key)
        .or_insert((now_bucket, 0));
    if entry.0 != now_bucket {
        *entry = (now_bucket, 0);
    }
    if entry.1 >= limit {
        return Err(Box::new(AuthDeny::rate_limited(
            Duration::from_secs(60),
            "http reads",
        )));
    }
    entry.1 += 1;
    Ok(())
}

async fn read_json_body<T: for<'de> Deserialize<'de>>(
    req: Request<hyper::body::Incoming>,
) -> std::result::Result<T, Response<Full<Bytes>>> {
    let collected = req
        .into_body()
        .collect()
        .await
        .map_err(|err| error_response(StatusCode::BAD_REQUEST, err.to_string()))?;
    serde_json::from_slice::<T>(&collected.to_bytes())
        .map_err(|err| error_response(StatusCode::BAD_REQUEST, err.to_string()))
}

async fn handle_chain_request(
    req: Request<hyper::body::Incoming>,
    path: &str,
    rpc_url: Arc<Option<String>>,
    rpc_client: Client,
    _auth_context: Option<crate::websocket::auth::AuthContext>,
) -> Response<Full<Bytes>> {
    let Some(rpc_url) = rpc_url.as_ref() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "No RPC URL configured for chain reads",
        );
    };

    match (req.method().as_str(), path) {
        ("GET", path) if path.starts_with("/chain/exists/") => {
            let address = path.trim_start_matches("/chain/exists/");
            match rpc_get_account_info(&rpc_client, rpc_url, address).await {
                Ok(value) => json_response(StatusCode::OK, json!({ "exists": !value.is_null() })),
                Err(err) => error_response(StatusCode::BAD_GATEWAY, err.to_string()),
            }
        }
        ("GET", path) if path.starts_with("/chain/lamports/") => {
            let address = path.trim_start_matches("/chain/lamports/");
            match rpc_call(
                &rpc_client,
                rpc_url,
                "getBalance",
                json!([address, { "commitment": "confirmed" }]),
            )
            .await
            {
                Ok(value) => json_response(
                    StatusCode::OK,
                    json!({ "lamports": value.pointer("/value").and_then(Value::as_u64).unwrap_or(0) }),
                ),
                Err(err) => error_response(StatusCode::BAD_GATEWAY, err.to_string()),
            }
        }
        ("POST", "/chain/native-balance") => match read_json_body::<NativeBalanceBody>(req).await {
            Ok(body) => {
                let min_context_slot =
                    match parse_min_context_slot(body.min_context_slot.as_deref()) {
                        Ok(slot) => slot,
                        Err(response) => return response,
                    };
                match rpc_get_native_balance(&rpc_client, rpc_url, &body.address, min_context_slot)
                    .await
                {
                    Ok(balance) => json_response(StatusCode::OK, balance),
                    Err(err) => error_response(StatusCode::BAD_GATEWAY, err.to_string()),
                }
            }
            Err(response) => response,
        },
        ("GET", path) if path.starts_with("/chain/rent-exemption/") => {
            let raw_space = path.trim_start_matches("/chain/rent-exemption/");
            let Ok(space) = raw_space.parse::<u64>() else {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    "rent-exemption space must be an integer",
                );
            };
            match rpc_call(
                &rpc_client,
                rpc_url,
                "getMinimumBalanceForRentExemption",
                json!([space, { "commitment": "confirmed" }]),
            )
            .await
            {
                Ok(value) => json_response(
                    StatusCode::OK,
                    json!({ "lamports": value.as_u64().unwrap_or(0) }),
                ),
                Err(err) => error_response(StatusCode::BAD_GATEWAY, err.to_string()),
            }
        }
        ("GET", "/chain/clock") => {
            let slot = rpc_call(
                &rpc_client,
                rpc_url,
                "getSlot",
                json!([{ "commitment": "confirmed" }]),
            )
            .await;
            let epoch_info = rpc_call(
                &rpc_client,
                rpc_url,
                "getEpochInfo",
                json!([{ "commitment": "confirmed" }]),
            )
            .await;
            match (slot, epoch_info) {
                (Ok(slot_value), Ok(epoch_value)) => {
                    let slot_num = slot_value.as_u64().unwrap_or(0);
                    let unix_timestamp =
                        rpc_call(&rpc_client, rpc_url, "getBlockTime", json!([slot_num]))
                            .await
                            .ok()
                            .and_then(|value| value.as_i64())
                            .unwrap_or_default();
                    json_response(
                        StatusCode::OK,
                        json!({
                            "slot": slot_num,
                            "epoch": epoch_value.get("epoch").and_then(Value::as_u64),
                            "leaderScheduleEpoch": epoch_value.get("leaderScheduleSlotOffset").and_then(Value::as_u64),
                            "unixTimestamp": unix_timestamp,
                        }),
                    )
                }
                (Err(err), _) | (_, Err(err)) => {
                    error_response(StatusCode::BAD_GATEWAY, err.to_string())
                }
            }
        }
        ("GET", path) if path.starts_with("/chain/accounts/") => {
            let address = path.trim_start_matches("/chain/accounts/");
            match rpc_get_account_info(&rpc_client, rpc_url, address).await {
                Ok(value) if value.is_null() => json_response(StatusCode::OK, Value::Null),
                Ok(value) => json_response(StatusCode::OK, raw_account_json(address, &value)),
                Err(err) => error_response(StatusCode::BAD_GATEWAY, err.to_string()),
            }
        }
        ("GET", path) if path.starts_with("/chain/mints/") => {
            let address = path.trim_start_matches("/chain/mints/");
            match rpc_get_parsed_account_info(&rpc_client, rpc_url, address).await {
                Ok(Some(value)) => json_response(StatusCode::OK, mint_info_json(address, &value)),
                Ok(None) => json_response(StatusCode::OK, Value::Null),
                Err(err) => error_response(StatusCode::BAD_GATEWAY, err.to_string()),
            }
        }
        ("GET", path) if path.starts_with("/chain/token-accounts/") => {
            let address = path.trim_start_matches("/chain/token-accounts/");
            match rpc_get_parsed_account_info(&rpc_client, rpc_url, address).await {
                Ok(Some(value)) => {
                    json_response(StatusCode::OK, token_account_json(address, &value))
                }
                Ok(None) => json_response(StatusCode::OK, Value::Null),
                Err(err) => error_response(StatusCode::BAD_GATEWAY, err.to_string()),
            }
        }
        ("POST", "/chain/balances") => match read_json_body::<BalanceBody>(req).await {
            Ok(body) => {
                let min_context_slot =
                    match parse_min_context_slot(body.min_context_slot.as_deref()) {
                        Ok(slot) => slot,
                        Err(response) => return response,
                    };
                match rpc_get_token_balance(
                    &rpc_client,
                    rpc_url,
                    &body.owner,
                    &body.mint,
                    body.token_program.as_deref(),
                    min_context_slot,
                )
                .await
                {
                    Ok(balance) => json_response(StatusCode::OK, balance),
                    Err(err) => error_response(StatusCode::BAD_GATEWAY, err.to_string()),
                }
            }
            Err(response) => response,
        },
        _ => error_response(StatusCode::NOT_FOUND, "Not Found"),
    }
}

async fn handle_program_account_request(
    req: Request<hyper::body::Incoming>,
    path: &str,
    rpc_url: Arc<Option<String>>,
    rpc_client: Client,
    program_account_reader: Arc<Option<ProgramAccountReaderFn>>,
    auth_context: Option<crate::websocket::auth::AuthContext>,
) -> Response<Full<Bytes>> {
    let Some(rpc_url) = rpc_url.as_ref() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "No RPC URL configured for program account reads",
        );
    };
    let Some(reader) = program_account_reader.as_ref() else {
        return error_response(
            StatusCode::NOT_IMPLEMENTED,
            "Program account reader is not configured",
        );
    };

    let segments: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    if segments.len() < 4
        || segments.first() != Some(&"programs")
        || segments.get(2) != Some(&"accounts")
    {
        return error_response(StatusCode::NOT_FOUND, "Not Found");
    }
    let program = segments[1];
    let account = segments[3];

    match (req.method().as_str(), segments.len()) {
        ("GET", 5) => {
            let address = segments[4];
            match rpc_get_account_info(&rpc_client, rpc_url, address).await {
                Ok(value) if value.is_null() => json_response(StatusCode::OK, Value::Null),
                Ok(value) => match decode_account_bytes(&value)
                    .and_then(|data| reader(program, account, &data).ok())
                {
                    Some(parsed) => json_response(StatusCode::OK, parsed),
                    None => {
                        error_response(StatusCode::BAD_REQUEST, "Unable to parse requested account")
                    }
                },
                Err(err) => error_response(StatusCode::BAD_GATEWAY, err.to_string()),
            }
        }
        ("GET", 6) if segments[5] == "exists" => {
            let address = segments[4];
            match rpc_get_account_info(&rpc_client, rpc_url, address).await {
                Ok(value) => json_response(StatusCode::OK, json!({ "exists": !value.is_null() })),
                Err(err) => error_response(StatusCode::BAD_GATEWAY, err.to_string()),
            }
        }
        ("POST", 4) => match read_json_body::<AddressesBody>(req).await {
            Ok(body) => {
                if let Some(limit) = auth_context
                    .as_ref()
                    .and_then(|ctx| ctx.limits.max_http_batch_addresses)
                {
                    if body.addresses.len() > limit as usize {
                        return auth_deny_response(&AuthDeny::rate_limited(
                            Duration::from_secs(60),
                            &format!(
                                "http batch reads ({} addresses > {})",
                                body.addresses.len(),
                                limit
                            ),
                        ));
                    }
                }
                match rpc_get_multiple_accounts(&rpc_client, rpc_url, &body.addresses).await {
                    Ok(values) => {
                        let parsed: Vec<Value> = values
                            .iter()
                            .map(|value| {
                                if value.is_null() {
                                    Value::Null
                                } else if let Some(data) = decode_account_bytes(value) {
                                    reader(program, account, &data).unwrap_or(Value::Null)
                                } else {
                                    Value::Null
                                }
                            })
                            .collect();
                        json_response(StatusCode::OK, Value::Array(parsed))
                    }
                    Err(err) => error_response(StatusCode::BAD_GATEWAY, err.to_string()),
                }
            }
            Err(response) => response,
        },
        _ => error_response(StatusCode::NOT_FOUND, "Not Found"),
    }
}

async fn rpc_call(
    client: &Client,
    rpc_url: &str,
    method: &str,
    params: Value,
) -> anyhow::Result<Value> {
    let response = client
        .post(rpc_url)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": "arete-read",
            "method": method,
            "params": params,
        }))
        .send()
        .await?
        .error_for_status()?;
    let value = response.json::<Value>().await?;
    if let Some(error) = value.get("error") {
        return Err(anyhow::anyhow!(error.to_string()));
    }
    Ok(value.get("result").cloned().unwrap_or(Value::Null))
}

fn rpc_read_config(encoding: Option<&str>, min_context_slot: Option<u64>) -> Value {
    let mut config = json!({ "commitment": "confirmed" });
    let object = config
        .as_object_mut()
        .expect("RPC read config is always an object");
    if let Some(encoding) = encoding {
        object.insert("encoding".to_string(), json!(encoding));
    }
    if let Some(min_context_slot) = min_context_slot {
        object.insert("minContextSlot".to_string(), json!(min_context_slot));
    }
    config
}

async fn rpc_get_native_balance(
    client: &Client,
    rpc_url: &str,
    address: &str,
    min_context_slot: Option<u64>,
) -> anyhow::Result<Value> {
    let result = rpc_call(
        client,
        rpc_url,
        "getBalance",
        json!([address, rpc_read_config(None, min_context_slot)]),
    )
    .await?;
    contextual_native_balance_json(&result)
}

fn contextual_native_balance_json(result: &Value) -> anyhow::Result<Value> {
    let lamports = result
        .get("value")
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow::anyhow!("getBalance response is missing a u64 value"))?;
    let context_slot = result
        .pointer("/context/slot")
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow::anyhow!("getBalance response is missing a u64 context slot"))?;
    Ok(json!({
        "lamports": lamports.to_string(),
        "contextSlot": context_slot.to_string(),
    }))
}

async fn rpc_get_account_info(
    client: &Client,
    rpc_url: &str,
    address: &str,
) -> anyhow::Result<Value> {
    let result = rpc_call(
        client,
        rpc_url,
        "getAccountInfo",
        json!([address, { "encoding": "base64", "commitment": "confirmed" }]),
    )
    .await?;
    Ok(result.get("value").cloned().unwrap_or(Value::Null))
}

async fn rpc_get_multiple_accounts(
    client: &Client,
    rpc_url: &str,
    addresses: &[String],
) -> anyhow::Result<Vec<Value>> {
    let result = rpc_call(
        client,
        rpc_url,
        "getMultipleAccounts",
        json!([addresses, { "encoding": "base64", "commitment": "confirmed" }]),
    )
    .await?;
    Ok(result
        .get("value")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default())
}

async fn rpc_get_parsed_account_info(
    client: &Client,
    rpc_url: &str,
    address: &str,
) -> anyhow::Result<Option<Value>> {
    let result = rpc_call(
        client,
        rpc_url,
        "getAccountInfo",
        json!([address, { "encoding": "jsonParsed", "commitment": "confirmed" }]),
    )
    .await?;
    Ok(result
        .get("value")
        .cloned()
        .filter(|value| !value.is_null()))
}

async fn rpc_get_token_balance(
    client: &Client,
    rpc_url: &str,
    owner: &str,
    mint: &str,
    token_program: Option<&str>,
    min_context_slot: Option<u64>,
) -> anyhow::Result<Value> {
    let filter = token_program
        .map(|program_id| json!({ "programId": program_id }))
        .unwrap_or_else(|| json!({ "mint": mint }));
    let result = rpc_call(
        client,
        rpc_url,
        "getTokenAccountsByOwner",
        json!([
            owner,
            filter,
            rpc_read_config(Some("jsonParsed"), min_context_slot)
        ]),
    )
    .await?;

    contextual_token_balance_json(&result, owner, mint, token_program)
}

fn contextual_token_balance_json(
    result: &Value,
    owner: &str,
    mint: &str,
    token_program: Option<&str>,
) -> anyhow::Result<Value> {
    let context_slot = result
        .pointer("/context/slot")
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            anyhow::anyhow!("getTokenAccountsByOwner response is missing a u64 context slot")
        })?;

    let account = result
        .get("value")
        .and_then(Value::as_array)
        .and_then(|items| {
            items.iter().find(|item| {
                item.pointer("/account/data/parsed/info/mint")
                    .and_then(Value::as_str)
                    == Some(mint)
            })
        })
        .cloned();

    if let Some(account) = account {
        let pubkey = account.get("pubkey").and_then(Value::as_str);
        let info = account.pointer("/account/data/parsed/info");
        return Ok(json!({
            "exists": true,
            "address": pubkey,
            "owner": owner,
            "mint": mint,
            "tokenProgram": token_program,
            "amount": info.and_then(|value| value.pointer("/tokenAmount/amount")).and_then(Value::as_str).unwrap_or("0"),
            "decimals": info.and_then(|value| value.pointer("/tokenAmount/decimals")).and_then(Value::as_u64),
            "uiAmountString": info.and_then(|value| value.pointer("/tokenAmount/uiAmountString")).and_then(Value::as_str),
            "contextSlot": context_slot.to_string(),
        }));
    }

    Ok(json!({
        "exists": false,
        "address": Value::Null,
        "owner": owner,
        "mint": mint,
        "tokenProgram": token_program,
        "amount": "0",
        "decimals": Value::Null,
        "uiAmountString": Value::Null,
        "contextSlot": context_slot.to_string(),
    }))
}

fn decode_account_bytes(value: &Value) -> Option<Vec<u8>> {
    let data = value.get("data")?.as_array()?;
    let encoded = data.first()?.as_str()?;
    BASE64_STANDARD.decode(encoded).ok()
}

fn raw_account_json(address: &str, value: &Value) -> Value {
    json!({
        "address": address,
        "ownerProgram": value.get("owner").and_then(Value::as_str).unwrap_or_default(),
        "lamports": value.get("lamports").and_then(Value::as_u64).unwrap_or(0),
        "executable": value.get("executable").and_then(Value::as_bool).unwrap_or(false),
        "data": value
            .pointer("/data/0")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    })
}

fn mint_info_json(address: &str, value: &Value) -> Value {
    let owner_program = value
        .get("owner")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let info = value.pointer("/data/parsed/info");
    json!({
        "address": address,
        "ownerProgram": owner_program,
        "decimals": info.and_then(|v| v.get("decimals")).and_then(Value::as_u64),
        "supply": info.and_then(|v| v.get("supply")).and_then(Value::as_str),
        "mintAuthority": info.and_then(|v| v.get("mintAuthority")).and_then(Value::as_str),
        "freezeAuthority": info.and_then(|v| v.get("freezeAuthority")).and_then(Value::as_str),
    })
}

fn token_account_json(address: &str, value: &Value) -> Value {
    let owner_program = value
        .get("owner")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let info = value.pointer("/data/parsed/info");
    json!({
        "address": address,
        "ownerProgram": owner_program,
        "mint": info.and_then(|v| v.get("mint")).and_then(Value::as_str),
        "owner": info.and_then(|v| v.get("owner")).and_then(Value::as_str),
        "amount": info.and_then(|v| v.pointer("/tokenAmount/amount")).and_then(Value::as_str),
        "uiAmountString": info.and_then(|v| v.pointer("/tokenAmount/uiAmountString")).and_then(Value::as_str),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cors_headers_allow_browser_sdk_reads() {
        let response = with_cors(
            Response::builder()
                .status(StatusCode::OK)
                .body(Full::new(Bytes::new()))
                .unwrap(),
        );

        assert_eq!(response.headers()[ACCESS_CONTROL_ALLOW_ORIGIN], "*");
        assert_eq!(
            response.headers()[ACCESS_CONTROL_ALLOW_METHODS],
            "GET, POST, OPTIONS"
        );
        assert_eq!(
            response.headers()[ACCESS_CONTROL_ALLOW_HEADERS],
            "Authorization, Content-Type"
        );
    }

    #[test]
    fn native_balance_serializes_u64_values_as_decimal_strings() {
        let value = contextual_native_balance_json(&json!({
            "context": { "slot": 9_007_199_254_740_995_u64 },
            "value": 9_007_199_254_740_993_u64,
        }))
        .unwrap();

        assert_eq!(value["lamports"], "9007199254740993");
        assert_eq!(value["contextSlot"], "9007199254740995");
    }

    #[test]
    fn rpc_read_config_propagates_min_context_slot() {
        let config = rpc_read_config(Some("jsonParsed"), Some(9_007_199_254_740_997));

        assert_eq!(config["commitment"], "confirmed");
        assert_eq!(config["encoding"], "jsonParsed");
        assert_eq!(config["minContextSlot"], 9_007_199_254_740_997_u64);
    }

    #[test]
    fn token_balance_preserves_raw_amount_and_stringifies_context_slot() {
        let value = contextual_token_balance_json(
            &json!({
                "context": { "slot": 9_007_199_254_740_995_u64 },
                "value": [{
                    "pubkey": "token-account",
                    "account": {
                        "data": {
                            "parsed": {
                                "info": {
                                    "mint": "mint",
                                    "tokenAmount": {
                                        "amount": "18446744073709551615",
                                        "decimals": 9,
                                        "uiAmountString": "18446744073.709551615"
                                    }
                                }
                            }
                        }
                    }
                }]
            }),
            "owner",
            "mint",
            None,
        )
        .unwrap();

        assert_eq!(value["amount"], "18446744073709551615");
        assert_eq!(value["contextSlot"], "9007199254740995");
    }

    #[test]
    fn balance_bodies_accept_decimal_string_min_context_slots() {
        let native: NativeBalanceBody = serde_json::from_value(json!({
            "address": "owner",
            "minContextSlot": "9007199254740997",
        }))
        .unwrap();
        let token: BalanceBody = serde_json::from_value(json!({
            "owner": "owner",
            "mint": "mint",
            "minContextSlot": "9007199254740999",
        }))
        .unwrap();

        assert_eq!(
            parse_min_context_slot(native.min_context_slot.as_deref()).unwrap(),
            Some(9_007_199_254_740_997)
        );
        assert_eq!(
            parse_min_context_slot(token.min_context_slot.as_deref()).unwrap(),
            Some(9_007_199_254_740_999)
        );
    }
}
