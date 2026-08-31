use crate::http::transactions::{self, TransactionState};
use crate::{
    config::{RuntimePlan, TransactionConfig},
    health::HealthMonitor,
    ProgramRuntimeCatalog, ProgramRuntimeDefinition,
};
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
use arete_auth::SCOPE_READ;

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
    snapshot_runtime: Option<crate::snapshot::SnapshotRuntime>,
    runtime_plan: RuntimePlan,
    rpc_url: Arc<Option<String>>,
    rpc_client: Client,
    program_runtime_catalog: Arc<ProgramRuntimeCatalog>,
    auth_plugin: Arc<Option<Arc<dyn WebSocketAuthPlugin>>>,
    limit_state: Arc<HttpLimitState>,
    transaction_state: Arc<Option<TransactionState>>,
    solana_gateway_target_id: Arc<Option<String>>,
    program_read_binding_target_id: Arc<Option<String>>,
}

/// HTTP server that exposes health endpoints
pub struct HttpHealthServer {
    bind_addr: SocketAddr,
    health_monitor: Option<HealthMonitor>,
    snapshot_runtime: Option<crate::snapshot::SnapshotRuntime>,
    runtime_plan: RuntimePlan,
    program_runtime_catalog: ProgramRuntimeCatalog,
    auth_plugin: Option<Arc<dyn WebSocketAuthPlugin>>,
    transaction_config: Option<TransactionConfig>,
    solana_gateway_target_id: Option<String>,
    program_read_binding_target_id: Option<String>,
    #[cfg(feature = "otel")]
    metrics: Option<Arc<crate::metrics::Metrics>>,
}

impl HttpHealthServer {
    pub fn new(bind_addr: SocketAddr) -> Self {
        Self {
            bind_addr,
            health_monitor: None,
            snapshot_runtime: None,
            runtime_plan: RuntimePlan::http(),
            program_runtime_catalog: ProgramRuntimeCatalog::default(),
            auth_plugin: None,
            transaction_config: None,
            solana_gateway_target_id: None,
            program_read_binding_target_id: None,
            #[cfg(feature = "otel")]
            metrics: None,
        }
    }

    pub fn with_health_monitor(mut self, monitor: HealthMonitor) -> Self {
        self.health_monitor = Some(monitor);
        self
    }

    pub fn with_snapshot_runtime(
        mut self,
        snapshot_runtime: crate::snapshot::SnapshotRuntime,
    ) -> Self {
        self.snapshot_runtime = Some(snapshot_runtime);
        self
    }

    pub fn with_runtime_plan(mut self, runtime_plan: RuntimePlan) -> Self {
        self.runtime_plan = runtime_plan;
        self
    }

    pub fn with_program_runtime_catalog(mut self, catalog: ProgramRuntimeCatalog) -> Self {
        self.program_runtime_catalog = catalog;
        self
    }

    pub fn with_auth_plugin(mut self, plugin: Arc<dyn WebSocketAuthPlugin>) -> Self {
        self.auth_plugin = Some(plugin);
        self
    }

    pub fn with_transaction_config(mut self, config: TransactionConfig) -> Self {
        self.runtime_plan.transactions = config.enabled;
        self.transaction_config = Some(config);
        self
    }

    pub fn with_solana_gateway_target(mut self, target_id: impl Into<String>) -> Self {
        self.solana_gateway_target_id = Some(target_id.into());
        self
    }

    pub fn with_program_read_binding_target(mut self, target_id: impl Into<String>) -> Self {
        self.program_read_binding_target_id = Some(target_id.into());
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
            snapshot_runtime: self.snapshot_runtime,
            runtime_plan: self.runtime_plan,
            rpc_url: Arc::new(resolve_rpc_url()),
            rpc_client: Client::builder().build()?,
            program_runtime_catalog: Arc::new(self.program_runtime_catalog),
            auth_plugin: Arc::new(self.auth_plugin),
            limit_state: Arc::new(HttpLimitState::default()),
            transaction_state: Arc::new(transaction_state),
            solana_gateway_target_id: Arc::new(self.solana_gateway_target_id),
            program_read_binding_target_id: Arc::new(self.program_read_binding_target_id),
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
            "Retry-After, X-Error-Code, X-Request-Id, X-Arete-Upstream-Attempted, X-Arete-Program-Release-Hash, X-Arete-Idl-Content-Hash, X-Arete-Account-Address, X-Arete-Account-Exists",
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
        snapshot_runtime,
        runtime_plan,
        rpc_url,
        rpc_client,
        program_runtime_catalog,
        auth_plugin,
        limit_state,
        transaction_state,
        solana_gateway_target_id,
        program_read_binding_target_id,
    } = state;
    let path = req.uri().path().to_string();

    match path.as_str() {
        "/health" | "/healthz" if runtime_plan.health => {
            // Basic health check - server is running
            Ok(Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "text/plain")
                .body(Full::new(Bytes::from("OK")))
                .unwrap())
        }
        "/ready" | "/readiness" if runtime_plan.health => {
            // Readiness check - stream must be healthy, and after a snapshot
            // resume the parser must have caught back up to the slot tip
            // before this pod should take traffic.
            let stream_ready = match health_monitor.as_ref() {
                Some(monitor) => monitor.is_healthy().await,
                // No health monitor configured, assume ready
                None => true,
            };
            let snapshot_ready = snapshot_runtime
                .as_ref()
                .is_none_or(crate::snapshot::SnapshotRuntime::resume_gate_ready);
            if stream_ready && snapshot_ready {
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
        }
        "/status" if runtime_plan.health => {
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
        _ if runtime_plan.transactions && path.starts_with("/transactions/") => {
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
                solana_gateway_target_id.as_deref(),
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
        _ if runtime_plan.chain_reads && path.starts_with("/chain/") => {
            let auth_context = match authorize_http_request(
                remote_addr,
                &req,
                auth_plugin.as_ref().as_ref(),
                &limit_state,
                Some(SCOPE_READ),
                true,
                solana_gateway_target_id.as_deref(),
            )
            .await
            {
                Ok(context) => context,
                Err(response) => return Ok(response),
            };
            Ok(handle_chain_request(req, path.as_str(), rpc_url, rpc_client, auth_context).await)
        }
        _ if path.starts_with("/v1/releases/") => {
            if !runtime_plan.program_reads {
                return Ok(program_read_error_response(
                    ProgramReadError::ProgramReadsDisabled,
                ));
            }
            let auth_context = match authorize_http_request(
                remote_addr,
                &req,
                auth_plugin.as_ref().as_ref(),
                &limit_state,
                Some(SCOPE_READ),
                true,
                None,
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
                program_runtime_catalog,
                auth_context,
                program_read_binding_target_id,
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

#[derive(Debug, Deserialize)]
struct AccountsBody {
    addresses: Vec<String>,
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

fn parse_min_context_slot(value: Option<&str>) -> std::result::Result<Option<u64>, &'static str> {
    value
        .map(|slot| {
            slot.parse::<u64>()
                .map_err(|_| "minContextSlot must be a decimal u64 string")
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

#[allow(clippy::result_large_err)]
async fn authorize_http_request(
    remote_addr: SocketAddr,
    req: &Request<hyper::body::Incoming>,
    auth_plugin: Option<&Arc<dyn WebSocketAuthPlugin>>,
    limit_state: &HttpLimitState,
    required_scope: Option<&str>,
    enforce_read_limits: bool,
    solana_gateway_target_id: Option<&str>,
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

    if let Some(target_id) = solana_gateway_target_id {
        if let Err(error) =
            arete_auth::SolanaGatewayAuthorization::validate_target(&context, target_id)
        {
            return Err(Response::builder()
                .status(StatusCode::FORBIDDEN)
                .header("Content-Type", "application/json")
                .header("X-Error-Code", "invalid_gateway_target")
                .body(Full::new(Bytes::from(
                    json!({
                        "error": "invalid_gateway_target",
                        "message": error.to_string(),
                        "code": "invalid_gateway_target",
                        "retryable": false,
                        "fatal": true
                    })
                    .to_string(),
                )))
                .expect("valid gateway target error response"));
        }
    }

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

#[allow(clippy::result_large_err)]
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
    auth_context: Option<crate::websocket::auth::AuthContext>,
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
                        Err(message) => return error_response(StatusCode::BAD_REQUEST, message),
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
        ("POST", "/chain/accounts") => match read_json_body::<AccountsBody>(req).await {
            Ok(body) => {
                let configured_limit =
                    batch_address_limit(auth_context.as_ref(), MAX_CHAIN_BATCH_ADDRESSES);
                if body.addresses.len() > configured_limit {
                    return error_response(
                        StatusCode::BAD_REQUEST,
                        format!(
                            "addresses exceeds the {configured_limit}-address limit for one batch"
                        ),
                    );
                }
                if body.addresses.is_empty() {
                    return json_response(StatusCode::OK, json!({ "items": [] }));
                }
                match rpc_get_multiple_accounts(&rpc_client, rpc_url, &body.addresses).await {
                    // A short array would silently misalign items with addresses, so treat a
                    // length mismatch as an upstream fault rather than padding it.
                    Ok(values) if values.len() == body.addresses.len() => json_response(
                        StatusCode::OK,
                        json!({ "items": batch_accounts_json(&body.addresses, &values) }),
                    ),
                    Ok(values) => error_response(
                        StatusCode::BAD_GATEWAY,
                        format!(
                            "getMultipleAccounts returned {} entries for {} addresses",
                            values.len(),
                            body.addresses.len()
                        ),
                    ),
                    Err(err) => error_response(StatusCode::BAD_GATEWAY, err.to_string()),
                }
            }
            Err(response) => response,
        },
        ("POST", "/chain/balances") => match read_json_body::<BalanceBody>(req).await {
            Ok(body) => {
                let min_context_slot =
                    match parse_min_context_slot(body.min_context_slot.as_deref()) {
                        Ok(slot) => slot,
                        Err(message) => return error_response(StatusCode::BAD_REQUEST, message),
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
    program_runtime_catalog: Arc<ProgramRuntimeCatalog>,
    auth_context: Option<crate::websocket::auth::AuthContext>,
    program_read_binding_target_id: Arc<Option<String>>,
) -> Response<Full<Bytes>> {
    let route = match ProgramReadRoute::parse(req.method(), path) {
        Ok(route) => route,
        Err(error) => return program_read_error_response(error),
    };
    let release_hash = match route.release_hash.parse() {
        Ok(release_hash) => release_hash,
        Err(_) => return program_read_error_response(ProgramReadError::InvalidReleaseHash),
    };
    let Some(definition) = program_runtime_catalog.get(&release_hash).cloned() else {
        return program_read_error_response(ProgramReadError::ReleaseNotFound);
    };
    if let Err(error) = authorize_program_read(
        auth_context.as_ref(),
        program_read_binding_target_id.as_deref(),
        &definition,
    ) {
        return with_release_metadata(program_read_error_response(error), &definition, None, None);
    }
    let Some(rpc_url) = rpc_url.as_ref() else {
        return with_release_metadata(
            program_read_error_response(ProgramReadError::RpcNotConfigured),
            &definition,
            None,
            None,
        );
    };

    match route.operation {
        ProgramReadOperation::Fetch { address } => {
            let account_value = match rpc_get_account_info(&rpc_client, rpc_url, &address).await {
                Ok(value) => value,
                Err(error) => {
                    error!("Program account RPC read failed: {}", error);
                    return with_release_metadata(
                        program_read_error_response(ProgramReadError::RpcFailed),
                        &definition,
                        Some(&address),
                        None,
                    );
                }
            };
            match decode_release_account(&definition, &route.account, &account_value).await {
                AccountReadOutcome::Missing => with_release_metadata(
                    json_response(StatusCode::OK, Value::Null),
                    &definition,
                    Some(&address),
                    Some(false),
                ),
                AccountReadOutcome::Value(value) => with_release_metadata(
                    json_response(StatusCode::OK, value),
                    &definition,
                    Some(&address),
                    Some(true),
                ),
                AccountReadOutcome::Error(error) => with_release_metadata(
                    program_read_error_response(error),
                    &definition,
                    Some(&address),
                    Some(true),
                ),
            }
        }
        ProgramReadOperation::Exists { address } => {
            let account_value = match rpc_get_account_info(&rpc_client, rpc_url, &address).await {
                Ok(value) => value,
                Err(error) => {
                    error!("Program account existence RPC read failed: {}", error);
                    return with_release_metadata(
                        program_read_error_response(ProgramReadError::RpcFailed),
                        &definition,
                        Some(&address),
                        None,
                    );
                }
            };
            let exists = !account_value.is_null();
            if exists && !account_owner_matches(&definition, &account_value) {
                return with_release_metadata(
                    program_read_error_response(ProgramReadError::AccountOwnerMismatch),
                    &definition,
                    Some(&address),
                    Some(true),
                );
            }
            with_release_metadata(
                json_response(StatusCode::OK, json!({ "exists": exists })),
                &definition,
                Some(&address),
                Some(exists),
            )
        }
        ProgramReadOperation::Batch => {
            let body = match read_json_body::<AddressesBody>(req).await {
                Ok(body) => body,
                Err(_) => return program_read_error_response(ProgramReadError::InvalidRequest),
            };
            let configured_limit =
                batch_address_limit(auth_context.as_ref(), MAX_PROGRAM_BATCH_ADDRESSES);
            if body.addresses.len() > configured_limit {
                return with_release_metadata(
                    program_read_error_response(ProgramReadError::BatchLimitExceeded),
                    &definition,
                    None,
                    None,
                );
            }
            if body.addresses.is_empty() {
                return with_release_metadata(
                    json_response(StatusCode::OK, json!({ "items": [] })),
                    &definition,
                    None,
                    None,
                );
            }

            let values =
                match rpc_get_multiple_accounts(&rpc_client, rpc_url, &body.addresses).await {
                    Ok(values) if values.len() == body.addresses.len() => values,
                    Ok(_) => {
                        return with_release_metadata(
                            program_read_error_response(ProgramReadError::RpcResponseInvalid),
                            &definition,
                            None,
                            None,
                        )
                    }
                    Err(error) => {
                        error!("Program account batch RPC read failed: {}", error);
                        return with_release_metadata(
                            program_read_error_response(ProgramReadError::RpcFailed),
                            &definition,
                            None,
                            None,
                        );
                    }
                };

            let outcomes = futures_util::future::join_all(
                values
                    .iter()
                    .map(|value| decode_release_account(&definition, &route.account, value)),
            )
            .await;
            let mut items = Vec::with_capacity(outcomes.len());
            for (address, outcome) in body.addresses.iter().zip(outcomes) {
                let item = match outcome {
                    AccountReadOutcome::Missing => {
                        json!({ "address": address, "status": "missing" })
                    }
                    AccountReadOutcome::Value(value) => {
                        json!({ "address": address, "status": "ok", "value": value })
                    }
                    AccountReadOutcome::Error(error) => json!({
                        "address": address,
                        "status": "error",
                        "error": { "code": error.code() }
                    }),
                };
                items.push(item);
            }
            with_release_metadata(
                json_response(StatusCode::OK, json!({ "items": items })),
                &definition,
                None,
                None,
            )
        }
    }
}

const MAX_PROGRAM_BATCH_ADDRESSES: usize = 100;
const MAX_PROGRAM_ACCOUNT_BYTES: usize = 10 * 1024 * 1024;
const PROGRAM_DECODE_TIMEOUT: Duration = Duration::from_secs(2);

fn authorize_program_read(
    auth_context: Option<&crate::websocket::auth::AuthContext>,
    expected_target_id: Option<&str>,
    definition: &ProgramRuntimeDefinition,
) -> std::result::Result<(), ProgramReadError> {
    let Some(context) = auth_context else {
        return Ok(());
    };
    let Some(expected_target_id) = expected_target_id.filter(|target_id| !target_id.is_empty())
    else {
        return Err(ProgramReadError::AuthorizationNotConfigured);
    };

    arete_auth::ProgramReadAuthorization::try_from_context(
        context,
        expected_target_id,
        &definition.program_id,
        &definition.program_release_hash.to_string(),
    )
    .map(|_| ())
    .map_err(|_| ProgramReadError::Unauthorized)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProgramReadError {
    NotFound,
    ProgramReadsDisabled,
    InvalidReleaseHash,
    ReleaseNotFound,
    InvalidRequest,
    Unauthorized,
    AuthorizationNotConfigured,
    BatchLimitExceeded,
    RpcNotConfigured,
    RpcFailed,
    RpcResponseInvalid,
    AccountOwnerMismatch,
    AccountDataInvalid,
    AccountDataTooLarge,
    AccountDecodeFailed,
    AccountDecodeTimeout,
}

impl ProgramReadError {
    fn code(self) -> &'static str {
        match self {
            Self::NotFound => "NOT_FOUND",
            Self::ProgramReadsDisabled => "PROGRAM_READS_DISABLED",
            Self::InvalidReleaseHash => "INVALID_PROGRAM_RELEASE_HASH",
            Self::ReleaseNotFound => "PROGRAM_RELEASE_NOT_FOUND",
            Self::InvalidRequest => "INVALID_REQUEST",
            Self::Unauthorized => "PROGRAM_READ_UNAUTHORIZED",
            Self::AuthorizationNotConfigured => "PROGRAM_READ_AUTH_NOT_CONFIGURED",
            Self::BatchLimitExceeded => "BATCH_LIMIT_EXCEEDED",
            Self::RpcNotConfigured => "READ_RPC_NOT_CONFIGURED",
            Self::RpcFailed => "RPC_REQUEST_FAILED",
            Self::RpcResponseInvalid => "RPC_RESPONSE_INVALID",
            Self::AccountOwnerMismatch => "ACCOUNT_OWNER_MISMATCH",
            Self::AccountDataInvalid => "ACCOUNT_DATA_INVALID",
            Self::AccountDataTooLarge => "ACCOUNT_DATA_TOO_LARGE",
            Self::AccountDecodeFailed => "ACCOUNT_DECODE_FAILED",
            Self::AccountDecodeTimeout => "ACCOUNT_DECODE_TIMEOUT",
        }
    }

    fn status(self) -> StatusCode {
        match self {
            Self::NotFound | Self::ReleaseNotFound => StatusCode::NOT_FOUND,
            Self::InvalidReleaseHash | Self::InvalidRequest => StatusCode::BAD_REQUEST,
            Self::Unauthorized => StatusCode::FORBIDDEN,
            Self::AuthorizationNotConfigured => StatusCode::SERVICE_UNAVAILABLE,
            Self::BatchLimitExceeded | Self::AccountDataTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            Self::ProgramReadsDisabled | Self::RpcNotConfigured => StatusCode::SERVICE_UNAVAILABLE,
            Self::RpcFailed | Self::RpcResponseInvalid | Self::AccountDataInvalid => {
                StatusCode::BAD_GATEWAY
            }
            Self::AccountOwnerMismatch | Self::AccountDecodeFailed => {
                StatusCode::UNPROCESSABLE_ENTITY
            }
            Self::AccountDecodeTimeout => StatusCode::GATEWAY_TIMEOUT,
        }
    }
}

fn program_read_error_response(error: ProgramReadError) -> Response<Full<Bytes>> {
    Response::builder()
        .status(error.status())
        .header("Content-Type", "application/json")
        .header("X-Error-Code", error.code())
        .body(Full::new(Bytes::from(
            json!({ "error": { "code": error.code() } }).to_string(),
        )))
        .expect("valid program read error response")
}

#[derive(Debug)]
struct ProgramReadRoute {
    release_hash: String,
    account: String,
    operation: ProgramReadOperation,
}

#[derive(Debug)]
enum ProgramReadOperation {
    Fetch { address: String },
    Batch,
    Exists { address: String },
}

impl ProgramReadRoute {
    fn parse(method: &Method, path: &str) -> std::result::Result<Self, ProgramReadError> {
        let segments: Vec<&str> = path.trim_start_matches('/').split('/').collect();
        if segments.len() < 5
            || segments[0] != "v1"
            || segments[1] != "releases"
            || segments[2].is_empty()
            || segments[3] != "accounts"
            || segments[4].is_empty()
        {
            return Err(ProgramReadError::NotFound);
        }
        let operation = match (method, segments.as_slice()) {
            (&Method::POST, [_, _, _, _, _]) => ProgramReadOperation::Batch,
            (&Method::GET, [_, _, _, _, _, address]) if !address.is_empty() => {
                ProgramReadOperation::Fetch {
                    address: (*address).to_string(),
                }
            }
            (&Method::GET, [_, _, _, _, _, address, "exists"]) if !address.is_empty() => {
                ProgramReadOperation::Exists {
                    address: (*address).to_string(),
                }
            }
            _ => return Err(ProgramReadError::NotFound),
        };
        Ok(Self {
            release_hash: segments[2].to_string(),
            account: segments[4].to_string(),
            operation,
        })
    }
}

enum AccountReadOutcome {
    Missing,
    Value(Value),
    Error(ProgramReadError),
}

fn account_owner_matches(definition: &ProgramRuntimeDefinition, value: &Value) -> bool {
    value.get("owner").and_then(Value::as_str) == Some(definition.program_id.as_str())
}

async fn decode_release_account(
    definition: &ProgramRuntimeDefinition,
    account: &str,
    value: &Value,
) -> AccountReadOutcome {
    if value.is_null() {
        return AccountReadOutcome::Missing;
    }
    if !account_owner_matches(definition, value) {
        return AccountReadOutcome::Error(ProgramReadError::AccountOwnerMismatch);
    }
    let data = match decode_account_bytes(value) {
        Some(data) => data,
        None => return AccountReadOutcome::Error(ProgramReadError::AccountDataInvalid),
    };
    if data.len() > MAX_PROGRAM_ACCOUNT_BYTES {
        return AccountReadOutcome::Error(ProgramReadError::AccountDataTooLarge);
    }

    let reader = definition.account_reader.clone();
    let account = account.to_string();
    let decode = tokio::task::spawn_blocking(move || reader(&account, &data));
    match tokio::time::timeout(PROGRAM_DECODE_TIMEOUT, decode).await {
        Ok(Ok(Ok(value))) => AccountReadOutcome::Value(value),
        Ok(Ok(Err(_))) | Ok(Err(_)) => {
            AccountReadOutcome::Error(ProgramReadError::AccountDecodeFailed)
        }
        Err(_) => AccountReadOutcome::Error(ProgramReadError::AccountDecodeTimeout),
    }
}

fn with_release_metadata(
    mut response: Response<Full<Bytes>>,
    definition: &ProgramRuntimeDefinition,
    address: Option<&str>,
    exists: Option<bool>,
) -> Response<Full<Bytes>> {
    let headers = response.headers_mut();
    if let Ok(value) = HeaderValue::from_str(&definition.program_release_hash.to_string()) {
        headers.insert("X-Arete-Program-Release-Hash", value);
    }
    if let Ok(value) = HeaderValue::from_str(&definition.idl_content_hash.to_string()) {
        headers.insert("X-Arete-Idl-Content-Hash", value);
    }
    if let Some(address) = address {
        if let Ok(value) = HeaderValue::from_str(address) {
            headers.insert("X-Arete-Account-Address", value);
        }
    }
    if let Some(exists) = exists {
        headers.insert(
            "X-Arete-Account-Exists",
            HeaderValue::from_static(if exists { "true" } else { "false" }),
        );
    }
    response
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

/// `lamports` is a decimal string, not a JSON number.
///
/// A `u64` above 2^53 does not survive a JSON number in every client: JavaScript rounds
/// `9007199254740993` to `...92` during `JSON.parse`, before any SDK can intervene. The
/// native-balance route already answers with a string for this reason, and the batch route makes it
/// matter — a custody sweep reading many accounts at once is exactly where a silently rounded
/// balance would be acted on.
fn raw_account_json(address: &str, value: &Value) -> Value {
    json!({
        "address": address,
        "ownerProgram": value.get("owner").and_then(Value::as_str).unwrap_or_default(),
        "lamports": value
            .get("lamports")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            .to_string(),
        "executable": value.get("executable").and_then(Value::as_bool).unwrap_or(false),
        "data": value
            .pointer("/data/0")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    })
}

/// How many addresses one batch may carry: the session's own ceiling, bounded by the protocol's.
///
/// Shared by every batch route rather than repeated at each one. Rate limiting charges a batch as a
/// single HTTP request, so a route that consulted only the protocol limit would let a session
/// capped at ten addresses read a hundred for the same cost. That is exactly what
/// `POST /chain/accounts` did while the program-account batch computed it correctly, so the two
/// call sites now derive it from one place and cannot drift apart again.
fn batch_address_limit(
    auth_context: Option<&crate::websocket::auth::AuthContext>,
    protocol_max: usize,
) -> usize {
    auth_context
        .and_then(|ctx| ctx.limits.max_http_batch_addresses)
        .map(|limit| limit as usize)
        .unwrap_or(protocol_max)
        .min(protocol_max)
}

/// Solana's own `getMultipleAccounts` ceiling, so one batch is one upstream call.
const MAX_CHAIN_BATCH_ADDRESSES: usize = 100;

/// Positionally aligned with `addresses`; `null` where the account is absent, matching
/// both `getMultipleAccounts` and the single-address route.
fn batch_accounts_json(addresses: &[String], values: &[Value]) -> Vec<Value> {
    addresses
        .iter()
        .zip(values)
        .map(|(address, value)| {
            if value.is_null() {
                Value::Null
            } else {
                raw_account_json(address, value)
            }
        })
        .collect()
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

    /// A missing account must hold its slot, or every later address is attributed to the
    /// wrong account — silently, and with real money downstream.
    #[test]
    fn batch_accounts_keep_position_when_an_account_is_absent() {
        let addresses = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let values = vec![
            json!({ "owner": "prog", "lamports": 7u64, "executable": false, "data": ["AQI=", "base64"] }),
            Value::Null,
            json!({ "owner": "prog", "lamports": 9u64, "executable": false, "data": ["AwQ=", "base64"] }),
        ];

        let items = batch_accounts_json(&addresses, &values);

        assert_eq!(items.len(), 3);
        assert_eq!(items[0]["address"], "a");
        assert_eq!(items[0]["lamports"], "7");
        assert_eq!(items[0]["data"], "AQI=");
        assert_eq!(items[1], Value::Null, "absent account keeps its slot");
        assert_eq!(items[2]["address"], "c");
        assert_eq!(items[2]["lamports"], "9");
    }

    /// The batch is where a rounded balance would be acted on, so the wire must carry a `u64`
    /// no JavaScript client can round. 9007199254740993 is the first integer a JSON number
    /// cannot represent: `JSON.parse` returns ...92.
    #[test]
    fn batch_accounts_serialize_lamports_as_decimal_strings() {
        let addresses = vec!["big".to_string()];
        let values = vec![json!({
            "owner": "prog",
            "lamports": 9_007_199_254_740_993_u64,
            "executable": false,
            "data": ["AQI=", "base64"],
        })];

        let items = batch_accounts_json(&addresses, &values);

        assert_eq!(items[0]["lamports"], "9007199254740993");
    }

    fn auth_context_allowing(
        max_batch_addresses: Option<u32>,
    ) -> crate::websocket::auth::AuthContext {
        crate::websocket::auth::AuthContext {
            subject: "test".to_string(),
            issuer: "test-issuer".to_string(),
            audience: "test-audience".to_string(),
            key_class: arete_auth::KeyClass::Publishable,
            metering_key: "meter-test".to_string(),
            deployment_id: None,
            target_kind: None,
            target_id: None,
            program_id: None,
            program_release_hash: None,
            expires_at: u64::MAX,
            scope: "read".to_string(),
            limits: arete_auth::Limits {
                max_http_batch_addresses: max_batch_addresses,
                ..Default::default()
            },
            plan: None,
            origin: None,
            client_ip: None,
            jti: "test-jti".to_string(),
            actor_key: None,
            account_key: None,
            consumer_key: None,
            policy_version: None,
            account_limits: arete_auth::Limits::default(),
        }
    }

    /// A batch is charged as one HTTP request, so a session's address ceiling is the only thing
    /// standing between a ten-address plan and a hundred-address read. `POST /chain/accounts`
    /// ignored it while the program-account batch honoured it; both now share this computation.
    #[test]
    fn a_session_ceiling_bounds_the_batch_below_the_protocol_limit() {
        let context = auth_context_allowing(Some(10));

        assert_eq!(
            batch_address_limit(Some(&context), MAX_CHAIN_BATCH_ADDRESSES),
            10
        );
    }

    /// A generous claim must not raise the protocol ceiling: one batch is still one upstream call.
    #[test]
    fn a_session_ceiling_cannot_exceed_the_protocol_limit() {
        let context = auth_context_allowing(Some(1_000));

        assert_eq!(
            batch_address_limit(Some(&context), MAX_CHAIN_BATCH_ADDRESSES),
            MAX_CHAIN_BATCH_ADDRESSES
        );
    }

    /// No claim, and no auth at all, both fall back to the protocol ceiling.
    #[test]
    fn an_absent_ceiling_falls_back_to_the_protocol_limit() {
        let context = auth_context_allowing(None);

        assert_eq!(
            batch_address_limit(Some(&context), MAX_CHAIN_BATCH_ADDRESSES),
            MAX_CHAIN_BATCH_ADDRESSES
        );
        assert_eq!(
            batch_address_limit(None, MAX_PROGRAM_BATCH_ADDRESSES),
            MAX_PROGRAM_BATCH_ADDRESSES
        );
    }

    #[test]
    fn batch_accounts_json_is_empty_for_no_addresses() {
        assert!(batch_accounts_json(&[], &[]).is_empty());
    }

    /// The batch item shape must stay identical to the single-address route's, or a client
    /// needs two parsers for one concept.
    #[test]
    fn batch_item_matches_the_single_address_shape() {
        let value = json!({ "owner": "prog", "lamports": 5u64, "executable": true, "data": ["BQY=", "base64"] });
        let addresses = vec!["solo".to_string()];

        assert_eq!(
            batch_accounts_json(&addresses, std::slice::from_ref(&value))[0],
            raw_account_json("solo", &value)
        );
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

    fn runtime_definition(reader: crate::ProgramAccountReaderFn) -> ProgramRuntimeDefinition {
        let program_spec_hash = crate::ProgramSpecHash::from_digest([1; 32]);
        let idl_content_hash = crate::IdlContentHash::from_digest([2; 32]);
        let normalized_idl_hash = crate::NormalizedIdlHash::from_digest([3; 32]);
        let program_release_hash = arete_hash::OssGeneratedProgramReleaseV1::new(
            "Program111",
            program_spec_hash,
            idl_content_hash,
            normalized_idl_hash,
        )
        .hash()
        .unwrap();
        ProgramRuntimeDefinition {
            program_id: "Program111".to_string(),
            program_spec_hash,
            idl_content_hash,
            normalized_idl_hash,
            program_release_hash,
            account_reader: reader,
        }
    }

    fn program_read_context(
        target_id: &str,
        program_id: &str,
        release_hash: &str,
    ) -> crate::websocket::auth::AuthContext {
        crate::websocket::auth::AuthContext::from_claims(
            arete_auth::SessionClaims::program_read_builder(
                "issuer",
                "user:1",
                target_id,
                program_id,
                release_hash,
            )
            .build(),
        )
    }

    #[test]
    fn program_read_auth_is_bound_to_target_program_and_release() {
        let definition = runtime_definition(Arc::new(|_, _| Ok(Value::Null)));
        let release_hash = definition.program_release_hash.to_string();
        let valid = program_read_context("binding-1", &definition.program_id, &release_hash);

        assert_eq!(
            authorize_program_read(Some(&valid), Some("binding-1"), &definition),
            Ok(())
        );

        let wrong_target = program_read_context("binding-2", &definition.program_id, &release_hash);
        let wrong_program = program_read_context("binding-1", "Program222", &release_hash);
        let wrong_release = program_read_context(
            "binding-1",
            &definition.program_id,
            "arete:h1:program-release:sha256:different",
        );

        for context in [&wrong_target, &wrong_program, &wrong_release] {
            assert_eq!(
                authorize_program_read(Some(context), Some("binding-1"), &definition),
                Err(ProgramReadError::Unauthorized)
            );
        }
        assert_eq!(
            authorize_program_read(Some(&valid), None, &definition),
            Err(ProgramReadError::AuthorizationNotConfigured)
        );
        assert_eq!(authorize_program_read(None, None, &definition), Ok(()));
    }

    #[test]
    fn release_routes_are_exact_and_legacy_program_routes_are_not_accepted() {
        let hash = crate::ProgramReleaseHash::from_digest([4; 32]);
        let fetch_path = format!("/v1/releases/{hash}/accounts/Vault/address");
        let exists_path = format!("{fetch_path}/exists");
        let batch_path = format!("/v1/releases/{hash}/accounts/Vault");

        assert!(matches!(
            ProgramReadRoute::parse(&Method::GET, &fetch_path)
                .unwrap()
                .operation,
            ProgramReadOperation::Fetch { .. }
        ));
        assert!(matches!(
            ProgramReadRoute::parse(&Method::GET, &exists_path)
                .unwrap()
                .operation,
            ProgramReadOperation::Exists { .. }
        ));
        assert!(matches!(
            ProgramReadRoute::parse(&Method::POST, &batch_path)
                .unwrap()
                .operation,
            ProgramReadOperation::Batch
        ));
        assert_eq!(
            ProgramReadRoute::parse(&Method::GET, "/programs/demo/accounts/Vault/address")
                .unwrap_err(),
            ProgramReadError::NotFound
        );
    }

    #[tokio::test]
    async fn owner_mismatch_is_rejected_before_decoder_execution() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let called = Arc::new(AtomicBool::new(false));
        let called_by_reader = called.clone();
        let definition = runtime_definition(Arc::new(move |_, _| {
            called_by_reader.store(true, Ordering::SeqCst);
            Ok(json!({ "decoded": true }))
        }));
        let value = json!({
            "owner": "DifferentProgram",
            "data": [BASE64_STANDARD.encode([1, 2, 3]), "base64"]
        });

        assert!(matches!(
            decode_release_account(&definition, "Vault", &value).await,
            AccountReadOutcome::Error(ProgramReadError::AccountOwnerMismatch)
        ));
        assert!(!called.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn decode_failures_remain_typed_errors_and_metadata_is_public_only() {
        let definition = runtime_definition(Arc::new(|_, _| anyhow::bail!("private diagnostic")));
        let value = json!({
            "owner": "Program111",
            "data": [BASE64_STANDARD.encode([1, 2, 3]), "base64"]
        });
        assert!(matches!(
            decode_release_account(&definition, "Vault", &value).await,
            AccountReadOutcome::Error(ProgramReadError::AccountDecodeFailed)
        ));

        let response = with_release_metadata(
            program_read_error_response(ProgramReadError::AccountDecodeFailed),
            &definition,
            Some("address"),
            Some(true),
        );
        assert_eq!(response.headers()["X-Error-Code"], "ACCOUNT_DECODE_FAILED");
        assert_eq!(
            response.headers()["X-Arete-Program-Release-Hash"],
            definition.program_release_hash.to_string()
        );
        assert_eq!(
            response.headers()["X-Arete-Idl-Content-Hash"],
            definition.idl_content_hash.to_string()
        );
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let body = std::str::from_utf8(&body).unwrap();
        assert_eq!(body, r#"{"error":{"code":"ACCOUNT_DECODE_FAILED"}}"#);
        assert!(!body.contains("private diagnostic"));
        assert!(!body.contains("decoder"));
    }
}
