use crate::{health::HealthMonitor, ProgramAccountReaderFn};
use anyhow::Result;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use dashmap::DashMap;
use http_body_util::BodyExt;
use http_body_util::Full;
use hyper::body::Bytes;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
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

/// HTTP server that exposes health endpoints
pub struct HttpHealthServer {
    bind_addr: SocketAddr,
    health_monitor: Option<HealthMonitor>,
    program_account_reader: Option<ProgramAccountReaderFn>,
    auth_plugin: Option<Arc<dyn WebSocketAuthPlugin>>,
}

impl HttpHealthServer {
    pub fn new(bind_addr: SocketAddr) -> Self {
        Self {
            bind_addr,
            health_monitor: None,
            program_account_reader: None,
            auth_plugin: None,
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

    pub async fn start(self) -> Result<()> {
        info!("Starting HTTP health server on {}", self.bind_addr);

        let listener = TcpListener::bind(&self.bind_addr).await?;
        info!("HTTP health server listening on {}", self.bind_addr);

        let health_monitor = Arc::new(self.health_monitor);
        let program_account_reader = Arc::new(self.program_account_reader);
        let auth_plugin = Arc::new(self.auth_plugin);
        let rpc_url = Arc::new(resolve_rpc_url());
        let rpc_client = Client::builder().build()?;
        let http_limit_state = Arc::new(HttpLimitState::default());

        loop {
            match listener.accept().await {
                Ok((stream, remote_addr)) => {
                    let io = TokioIo::new(stream);
                    let monitor = health_monitor.clone();
                    let reader = program_account_reader.clone();
                    let auth_plugin = auth_plugin.clone();
                    let rpc_url = rpc_url.clone();
                    let rpc_client = rpc_client.clone();
                    let http_limit_state = http_limit_state.clone();

                    tokio::spawn(async move {
                        let service = service_fn(move |req| {
                            let monitor = monitor.clone();
                            let reader = reader.clone();
                            let auth_plugin = auth_plugin.clone();
                            let rpc_url = rpc_url.clone();
                            let rpc_client = rpc_client.clone();
                            let http_limit_state = http_limit_state.clone();
                            async move {
                                handle_request(
                                    remote_addr,
                                    req,
                                    monitor,
                                    rpc_url,
                                    rpc_client,
                                    reader,
                                    auth_plugin,
                                    http_limit_state,
                                )
                                .await
                            }
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
    health_monitor: Arc<Option<HealthMonitor>>,
    rpc_url: Arc<Option<String>>,
    rpc_client: Client,
    program_account_reader: Arc<Option<ProgramAccountReaderFn>>,
    auth_plugin: Arc<Option<Arc<dyn WebSocketAuthPlugin>>>,
    http_limit_state: Arc<HttpLimitState>,
) -> Result<Response<Full<Bytes>>, Infallible> {
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
        _ if path.starts_with("/chain/") => {
            let auth_context = match authorize_http_request(
                remote_addr,
                &req,
                auth_plugin.as_ref().as_ref(),
                &http_limit_state,
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
                &http_limit_state,
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

    enforce_http_limits(&context, limit_state).map_err(|deny| auth_deny_response(&deny))?;
    Ok(Some(context))
}

fn enforce_http_limits(
    context: &crate::websocket::auth::AuthContext,
    limit_state: &HttpLimitState,
) -> std::result::Result<(), AuthDeny> {
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
        return Err(AuthDeny::rate_limited(
            Duration::from_secs(60),
            "http reads",
        ));
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
            Ok(body) => match rpc_get_token_balance(
                &rpc_client,
                rpc_url,
                &body.owner,
                &body.mint,
                body.token_program.as_deref(),
            )
            .await
            {
                Ok(balance) => json_response(StatusCode::OK, balance),
                Err(err) => error_response(StatusCode::BAD_GATEWAY, err.to_string()),
            },
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
            { "encoding": "jsonParsed", "commitment": "confirmed" }
        ]),
    )
    .await?;

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
