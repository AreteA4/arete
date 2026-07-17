use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use arete_auth::AuthContext;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use dashmap::DashMap;
use futures_util::StreamExt;
use http_body_util::BodyExt;
use http_body_util::Full;
use hyper::body::{Bytes, Incoming};
use hyper::header::{CONTENT_LENGTH, CONTENT_TYPE};
use hyper::{Method, Request, Response, StatusCode};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use uuid::Uuid;

use crate::config::TransactionConfig;

const MAX_UPSTREAM_RESPONSE_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone)]
pub(crate) struct TransactionState {
    config: Arc<TransactionConfig>,
    client: Client,
    inspect_semaphore: Arc<Semaphore>,
    send_semaphore: Arc<Semaphore>,
    rate_buckets: Arc<DashMap<String, (u64, u32)>>,
    subject_inflight: Arc<DashMap<String, u32>>,
    usage_tx: Option<tokio::sync::mpsc::Sender<TransactionUsageEvent>>,
    #[cfg(feature = "otel")]
    metrics: Option<Arc<crate::metrics::Metrics>>,
}

impl TransactionState {
    pub(crate) fn new(config: TransactionConfig) -> anyhow::Result<Self> {
        config.validate()?;
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(3))
            .build()?;
        let usage_tx = if config.usage_enabled {
            let (tx, rx) = tokio::sync::mpsc::channel(config.usage_spool_capacity);
            spawn_usage_worker(
                rx,
                client.clone(),
                config
                    .usage_endpoint
                    .clone()
                    .expect("validated usage endpoint"),
                config.usage_token.clone().expect("validated usage token"),
            );
            Some(tx)
        } else {
            None
        };
        Ok(Self {
            inspect_semaphore: Arc::new(Semaphore::new(config.inspect_concurrency)),
            send_semaphore: Arc::new(Semaphore::new(config.send_concurrency)),
            config: Arc::new(config),
            client,
            rate_buckets: Arc::new(DashMap::new()),
            subject_inflight: Arc::new(DashMap::new()),
            usage_tx,
            #[cfg(feature = "otel")]
            metrics: None,
        })
    }

    pub(crate) fn client_addr(
        &self,
        remote_addr: SocketAddr,
        headers: &hyper::HeaderMap,
    ) -> SocketAddr {
        SocketAddr::new(
            trusted_client_ip(remote_addr, headers, &self.config),
            remote_addr.port(),
        )
    }

    #[cfg(feature = "otel")]
    pub(crate) fn with_metrics(mut self, metrics: Option<Arc<crate::metrics::Metrics>>) -> Self {
        self.metrics = metrics;
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Operation {
    LatestBlockhash,
    Fee,
    Simulate,
    Send,
    SignatureStatus,
    BlockHeight,
}

#[derive(Debug, Serialize)]
struct TransactionUsageEvent {
    event_id: String,
    occurred_at_ms: u64,
    deployment_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    subject: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    metering_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    key_class: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    plan: Option<String>,
    operation: &'static str,
    result: &'static str,
    request_bytes: u64,
    response_bytes: u64,
    latency_ms: u64,
}

impl Operation {
    fn from_path(path: &str) -> Option<Self> {
        match path {
            "/transactions/v1/latest-blockhash" => Some(Self::LatestBlockhash),
            "/transactions/v1/fee" => Some(Self::Fee),
            "/transactions/v1/simulate" => Some(Self::Simulate),
            "/transactions/v1/send" => Some(Self::Send),
            "/transactions/v1/signature-status" => Some(Self::SignatureStatus),
            "/transactions/v1/block-height" => Some(Self::BlockHeight),
            _ => None,
        }
    }

    fn scope(self) -> &'static str {
        if self == Self::Send {
            "transaction:send"
        } else {
            "transaction:inspect"
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::LatestBlockhash => "latest_blockhash",
            Self::Fee => "fee",
            Self::Simulate => "simulate",
            Self::Send => "send",
            Self::SignatureStatus => "signature_status",
            Self::BlockHeight => "block_height",
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ErrorEnvelope {
    code: &'static str,
    message: String,
    retryable: bool,
    request_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    submission_state: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    signature: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    details: Option<Box<RpcErrorDetails>>,
}

#[derive(Debug, Serialize)]
struct RpcErrorDetails {
    code: i64,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

#[derive(Debug)]
struct TxError {
    status: StatusCode,
    code: &'static str,
    message: String,
    retryable: bool,
    submission_state: Option<&'static str>,
    signature: Option<String>,
    details: Option<Box<RpcErrorDetails>>,
    upstream_attempted: bool,
}

impl TxError {
    fn request(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code,
            message: message.into(),
            retryable: false,
            submission_state: None,
            signature: None,
            details: None,
            upstream_attempted: false,
        }
    }

    fn response(self, request_id: &str) -> Response<Full<Bytes>> {
        transaction_response(
            self.status,
            request_id,
            self.upstream_attempted,
            serde_json::to_value(ErrorEnvelope {
                code: self.code,
                message: self.message,
                retryable: self.retryable,
                request_id: request_id.to_string(),
                submission_state: self.submission_state,
                signature: self.signature,
                details: self.details,
            })
            .expect("error envelope serializes"),
        )
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CommonRequest {
    #[serde(default)]
    commitment: Option<Commitment>,
    #[serde(default)]
    min_context_slot: Option<DecimalU64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FeeRequest {
    message: String,
    #[serde(default)]
    commitment: Option<Commitment>,
    #[serde(default)]
    min_context_slot: Option<DecimalU64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SimulateRequest {
    transaction: String,
    #[serde(default)]
    commitment: Option<Commitment>,
    #[serde(default)]
    min_context_slot: Option<DecimalU64>,
    #[serde(default)]
    sig_verify: bool,
    #[serde(default)]
    replace_recent_blockhash: bool,
    #[serde(default)]
    inner_instructions: bool,
    #[serde(default)]
    accounts: Option<SimulationAccounts>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SimulationAccounts {
    addresses: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SendRequest {
    transaction: String,
    #[serde(default)]
    skip_preflight: bool,
    #[serde(default)]
    preflight_commitment: Option<Commitment>,
    #[serde(default)]
    min_context_slot: Option<DecimalU64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SignatureStatusRequest {
    signature: String,
    #[serde(default)]
    search_transaction_history: bool,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
enum Commitment {
    Processed,
    Confirmed,
    Finalized,
}

impl Commitment {
    fn as_str(self) -> &'static str {
        match self {
            Self::Processed => "processed",
            Self::Confirmed => "confirmed",
            Self::Finalized => "finalized",
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct DecimalU64(u64);

impl<'de> Deserialize<'de> for DecimalU64 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value
            .parse()
            .map(Self)
            .map_err(|_| serde::de::Error::custom("expected a decimal u64 string"))
    }
}

pub(crate) async fn handle(
    remote_addr: SocketAddr,
    req: Request<Incoming>,
    auth: Option<AuthContext>,
    state: TransactionState,
) -> Response<Full<Bytes>> {
    let request_id = Uuid::new_v4().to_string();
    let start = Instant::now();
    let path = req.uri().path();
    let Some(operation) = Operation::from_path(path) else {
        return TxError {
            status: StatusCode::NOT_FOUND,
            code: "not_found",
            message: "Transaction route not found".into(),
            retryable: false,
            submission_state: None,
            signature: None,
            details: None,
            upstream_attempted: false,
        }
        .response(&request_id);
    };

    if req.method() != Method::POST {
        return TxError {
            status: StatusCode::METHOD_NOT_ALLOWED,
            code: "method_not_allowed",
            message: "Transaction routes accept POST only".into(),
            retryable: false,
            submission_state: None,
            signature: None,
            details: None,
            upstream_attempted: false,
        }
        .response(&request_id);
    }

    match &auth {
        Some(context) => {
            if !context.has_scope(operation.scope()) {
                #[cfg(feature = "otel")]
                if let Some(metrics) = &state.metrics {
                    metrics.record_transaction_denial("scope");
                }
                return TxError {
                    status: StatusCode::FORBIDDEN,
                    code: "insufficient_scope",
                    message: format!("Required scope: {}", operation.scope()),
                    retryable: false,
                    submission_state: (operation == Operation::Send).then_some("not_submitted"),
                    signature: None,
                    details: None,
                    upstream_attempted: false,
                }
                .response(&request_id);
            }
        }
        None => {
            if !state.config.allow_unauthenticated {
                #[cfg(feature = "otel")]
                if let Some(metrics) = &state.metrics {
                    metrics.record_transaction_denial("auth");
                }
                return TxError {
                    status: StatusCode::UNAUTHORIZED,
                    code: "authentication_required",
                    message: "Transaction relay requires an auth plugin; set ARETE_TRANSACTIONS_ALLOW_UNAUTHENTICATED=true for development only".into(),
                    retryable: false,
                    submission_state: (operation == Operation::Send).then_some("not_submitted"),
                    signature: None,
                    details: None,
                    upstream_attempted: false,
                }
                .response(&request_id);
            }
        }
    }

    let client_ip = trusted_client_ip(remote_addr, req.headers(), &state.config);
    let admission = match admit(operation, auth.as_ref(), client_ip, &state).await {
        Ok(admission) => admission,
        Err(error) => return error.response(&request_id),
    };
    let body_limit = auth
        .as_ref()
        .and_then(|context| context.limits.max_transaction_request_bytes)
        .map(|limit| limit as usize)
        .unwrap_or(state.config.max_body_bytes)
        .min(state.config.max_body_bytes);
    let body = match read_bounded_body(req, body_limit).await {
        Ok(body) => body,
        Err(error) => return error.response(&request_id),
    };

    #[cfg(feature = "otel")]
    if let Some(metrics) = &state.metrics {
        metrics.record_transaction_inflight(1, operation.name());
    }
    let result = dispatch(operation, &body, auth.as_ref(), &state).await;
    #[cfg(feature = "otel")]
    if let Some(metrics) = &state.metrics {
        metrics.record_transaction_inflight(-1, operation.name());
    }
    drop(admission);
    let result_name = if result.is_ok() { "ok" } else { "error" };
    emit_usage(
        &state,
        auth.as_ref(),
        operation,
        &result,
        body.len(),
        start.elapsed(),
    );
    tracing::info!(
        operation = operation.name(),
        result = result_name,
        latency_ms = start.elapsed().as_millis() as u64,
        request_bytes = body.len(),
        "transaction relay request"
    );
    #[cfg(feature = "otel")]
    if let Some(metrics) = &state.metrics {
        metrics.record_transaction_request(
            operation.name(),
            result_name,
            start.elapsed().as_secs_f64() * 1000.0,
            body.len() as u64,
        );
    }
    match result {
        Ok(value) => transaction_response(StatusCode::OK, &request_id, true, value),
        Err(error) => error.response(&request_id),
    }
}

fn emit_usage(
    state: &TransactionState,
    auth: Option<&AuthContext>,
    operation: Operation,
    result: &Result<Value, TxError>,
    request_bytes: usize,
    latency: Duration,
) {
    let Some(sender) = &state.usage_tx else {
        return;
    };
    let Some(deployment_id) = auth.and_then(|context| context.deployment_id.clone()) else {
        tracing::warn!(
            operation = operation.name(),
            "transaction usage event omitted because deployment ID is unavailable"
        );
        return;
    };
    let response_bytes = result
        .as_ref()
        .map(|value| value.to_string().len())
        .unwrap_or_default();
    let outcome = match (operation, result) {
        (Operation::Send, Ok(_)) => "accepted",
        (Operation::Send, Err(error)) if error.submission_state == Some("unknown") => "unknown",
        (Operation::Send, Err(_)) => "rejected",
        (_, Ok(_)) => "ok",
        (_, Err(_)) => "error",
    };
    let event = TransactionUsageEvent {
        event_id: Uuid::new_v4().to_string(),
        occurred_at_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX),
        deployment_id,
        subject: auth.map(|context| context.subject.clone()),
        metering_key: auth.map(|context| context.metering_key.clone()),
        key_class: auth.map(|context| match context.key_class {
            arete_auth::KeyClass::Secret => "secret",
            arete_auth::KeyClass::Publishable => "publishable",
        }),
        plan: auth.and_then(|context| context.plan.clone()),
        operation: operation.name(),
        result: outcome,
        request_bytes: request_bytes.try_into().unwrap_or(u64::MAX),
        response_bytes: response_bytes.try_into().unwrap_or(u64::MAX),
        latency_ms: latency.as_millis().try_into().unwrap_or(u64::MAX),
    };
    if sender.try_send(event).is_err() {
        tracing::warn!(
            operation = operation.name(),
            "transaction usage queue is full"
        );
    }
}

fn spawn_usage_worker(
    mut receiver: tokio::sync::mpsc::Receiver<TransactionUsageEvent>,
    client: Client,
    endpoint: String,
    token: String,
) {
    tokio::spawn(async move {
        while let Some(event) = receiver.recv().await {
            let payload = json!({ "events": [event] });
            let mut delivered = false;
            for attempt in 0..3 {
                let response = client
                    .post(&endpoint)
                    .bearer_auth(&token)
                    .timeout(Duration::from_secs(5))
                    .json(&payload)
                    .send()
                    .await;
                if response.is_ok_and(|response| response.status().is_success()) {
                    delivered = true;
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100 * (1 << attempt))).await;
            }
            if !delivered {
                tracing::warn!("transaction usage delivery failed after bounded retries");
            }
        }
    });
}

struct Admission {
    _permit: OwnedSemaphorePermit,
    subject: Option<String>,
    subject_inflight: Arc<DashMap<String, u32>>,
}

impl Drop for Admission {
    fn drop(&mut self) {
        if let Some(subject) = &self.subject {
            if let Some(mut count) = self.subject_inflight.get_mut(subject) {
                *count = count.saturating_sub(1);
            }
        }
    }
}

async fn admit(
    operation: Operation,
    auth: Option<&AuthContext>,
    client_ip: IpAddr,
    state: &TransactionState,
) -> Result<Admission, TxError> {
    let server_limit = match operation {
        Operation::Send => state.config.send_requests_per_minute,
        Operation::SignatureStatus => state.config.status_requests_per_minute,
        _ => state.config.inspect_requests_per_minute,
    };
    let claim_limit = auth.and_then(|context| match operation {
        Operation::Send => context.limits.max_transaction_send_requests_per_minute,
        Operation::SignatureStatus => context.limits.max_transaction_status_requests_per_minute,
        _ => context.limits.max_transaction_inspect_requests_per_minute,
    });
    let limit = claim_limit.unwrap_or(server_limit).min(server_limit);
    let class = match operation {
        Operation::Send => "send",
        Operation::SignatureStatus => "status",
        _ => "inspect",
    };
    check_bucket(
        &state.rate_buckets,
        format!("ip:{client_ip}:{class}"),
        limit,
    )?;
    let semaphore = if operation == Operation::Send {
        state.send_semaphore.clone()
    } else {
        state.inspect_semaphore.clone()
    };
    let permit = semaphore.try_acquire_owned().map_err(|_| {
        #[cfg(feature = "otel")]
        if let Some(metrics) = &state.metrics {
            metrics.record_transaction_denial("concurrency");
        }
        limit_error("transaction server is at capacity")
    })?;
    if let Some(context) = auth {
        check_bucket(
            &state.rate_buckets,
            format!("subject:{}:{class}", context.subject),
            limit,
        )?;
        if let Some(max) = context.limits.max_transaction_concurrency {
            let mut count = state
                .subject_inflight
                .entry(context.subject.clone())
                .or_insert(0);
            if *count >= max {
                #[cfg(feature = "otel")]
                if let Some(metrics) = &state.metrics {
                    metrics.record_transaction_denial("concurrency");
                }
                return Err(limit_error("transaction concurrency limit exceeded"));
            }
            *count += 1;
        }
    }
    Ok(Admission {
        _permit: permit,
        subject: auth
            .filter(|context| context.limits.max_transaction_concurrency.is_some())
            .map(|context| context.subject.clone()),
        subject_inflight: state.subject_inflight.clone(),
    })
}

fn check_bucket(
    buckets: &DashMap<String, (u64, u32)>,
    key: String,
    limit: u32,
) -> Result<(), TxError> {
    let minute = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        / 60;
    let mut entry = buckets.entry(key).or_insert((minute, 0));
    if entry.0 != minute {
        *entry = (minute, 0);
    }
    if entry.1 >= limit {
        return Err(limit_error("transaction rate limit exceeded"));
    }
    entry.1 += 1;
    Ok(())
}

fn limit_error(message: &'static str) -> TxError {
    TxError {
        status: StatusCode::TOO_MANY_REQUESTS,
        code: "rate_limit_exceeded",
        message: message.into(),
        retryable: true,
        submission_state: Some("not_submitted"),
        signature: None,
        details: None,
        upstream_attempted: false,
    }
}

async fn read_bounded_body(req: Request<Incoming>, max: usize) -> Result<Bytes, TxError> {
    if req
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok())
        .is_some_and(|length| length > max)
    {
        return Err(payload_too_large());
    }
    let mut body = req.into_body();
    let mut bytes = Vec::new();
    while let Some(frame) = body.frame().await {
        let frame =
            frame.map_err(|_| TxError::request("invalid_body", "Unable to read request body"))?;
        if let Ok(data) = frame.into_data() {
            if bytes.len().saturating_add(data.len()) > max {
                return Err(payload_too_large());
            }
            bytes.extend_from_slice(&data);
        }
    }
    Ok(Bytes::from(bytes))
}

fn payload_too_large() -> TxError {
    TxError {
        status: StatusCode::PAYLOAD_TOO_LARGE,
        code: "request_too_large",
        message: "Transaction request exceeds the configured limit".into(),
        retryable: false,
        submission_state: Some("not_submitted"),
        signature: None,
        details: None,
        upstream_attempted: false,
    }
}

async fn dispatch(
    operation: Operation,
    body: &[u8],
    auth: Option<&AuthContext>,
    state: &TransactionState,
) -> Result<Value, TxError> {
    let max_transaction_bytes = auth
        .and_then(|context| context.limits.max_transaction_bytes)
        .map(|limit| limit as usize)
        .unwrap_or(state.config.max_transaction_bytes)
        .min(state.config.max_transaction_bytes);
    match operation {
        Operation::LatestBlockhash => {
            let request: CommonRequest = parse_json(body)?;
            let config = common_config(request.commitment, request.min_context_slot);
            let value = rpc_call(
                state,
                "getLatestBlockhash",
                json!([config]),
                operation,
                None,
            )
            .await?;
            Ok(json!({
                "blockhash": required_str(&value, "/value/blockhash")?,
                "contextSlot": required_u64(&value, "/context/slot")?.to_string(),
                "lastValidBlockHeight": required_u64(&value, "/value/lastValidBlockHeight")?.to_string()
            }))
        }
        Operation::Fee => {
            let request: FeeRequest = parse_json(body)?;
            decode_bounded(&request.message, max_transaction_bytes, "message")?;
            let config = common_config(request.commitment, request.min_context_slot);
            let value = rpc_call(
                state,
                "getFeeForMessage",
                json!([request.message, config]),
                operation,
                None,
            )
            .await?;
            let fee = value
                .pointer("/value")
                .and_then(Value::as_u64)
                .map(|value| value.to_string());
            Ok(json!({
                "feeLamports": fee,
                "contextSlot": required_u64(&value, "/context/slot")?.to_string()
            }))
        }
        Operation::Simulate => {
            let request: SimulateRequest = parse_json(body)?;
            decode_bounded(&request.transaction, max_transaction_bytes, "transaction")?;
            if request.sig_verify && request.replace_recent_blockhash {
                return Err(TxError::request(
                    "invalid_options",
                    "sigVerify and replaceRecentBlockhash cannot both be true",
                ));
            }
            let mut config = common_config(request.commitment, request.min_context_slot);
            let object = config.as_object_mut().expect("config is object");
            object.insert("encoding".into(), json!("base64"));
            object.insert("sigVerify".into(), json!(request.sig_verify));
            object.insert(
                "replaceRecentBlockhash".into(),
                json!(request.replace_recent_blockhash),
            );
            object.insert(
                "innerInstructions".into(),
                json!(request.inner_instructions),
            );
            if let Some(accounts) = request.accounts {
                if accounts.addresses.len() > 16
                    || accounts
                        .addresses
                        .iter()
                        .any(|address| !valid_address(address))
                {
                    return Err(TxError::request(
                        "invalid_accounts",
                        "simulation accounts must contain at most 16 valid addresses",
                    ));
                }
                object.insert(
                    "accounts".into(),
                    json!({ "encoding": "base64", "addresses": accounts.addresses }),
                );
            }
            let value = rpc_call(
                state,
                "simulateTransaction",
                json!([request.transaction, config]),
                operation,
                None,
            )
            .await?;
            simulation_response(value)
        }
        Operation::Send => {
            let request: SendRequest = parse_json(body)?;
            let transaction =
                decode_bounded(&request.transaction, max_transaction_bytes, "transaction")?;
            let signature = transaction_signature(&transaction)?;
            let mut config = json!({
                "encoding": "base64",
                "skipPreflight": request.skip_preflight,
                "preflightCommitment": request.preflight_commitment.unwrap_or(Commitment::Confirmed).as_str(),
                "maxRetries": 0
            });
            if let Some(slot) = request.min_context_slot {
                config["minContextSlot"] = json!(slot.0);
            }
            let value = rpc_call(
                state,
                "sendTransaction",
                json!([request.transaction, config]),
                operation,
                Some(signature.clone()),
            )
            .await?;
            let upstream_signature = value.as_str().ok_or_else(|| {
                ambiguous_error("Malformed response from transaction RPC", signature.clone())
            })?;
            if upstream_signature != signature {
                return Err(ambiguous_error(
                    "Transaction RPC returned an unexpected signature",
                    signature,
                ));
            }
            Ok(json!({ "signature": upstream_signature }))
        }
        Operation::SignatureStatus => {
            let request: SignatureStatusRequest = parse_json(body)?;
            if !valid_signature(&request.signature) {
                return Err(TxError::request(
                    "invalid_signature",
                    "signature must be a base58-encoded 64-byte value",
                ));
            }
            let value = rpc_call(
                state,
                "getSignatureStatuses",
                json!([[request.signature], { "searchTransactionHistory": request.search_transaction_history }]),
                operation,
                None,
            )
            .await?;
            let status = value.pointer("/value/0").cloned().unwrap_or(Value::Null);
            if status.is_null() {
                Ok(json!({ "status": Value::Null }))
            } else {
                Ok(json!({
                    "status": {
                        "slot": required_u64(&status, "/slot")?.to_string(),
                        "confirmations": status.get("confirmations").and_then(Value::as_u64).map(|value| value.to_string()),
                        "confirmationStatus": status.get("confirmationStatus"),
                        "err": status.get("err")
                    }
                }))
            }
        }
        Operation::BlockHeight => {
            let request: CommonRequest = parse_json(body)?;
            let config = common_config(request.commitment, request.min_context_slot);
            let value = rpc_call(state, "getBlockHeight", json!([config]), operation, None).await?;
            let height = value.as_u64().ok_or_else(|| {
                upstream_malformed("Malformed block height response", operation, None)
            })?;
            Ok(json!({ "blockHeight": height.to_string() }))
        }
    }
}

fn parse_json<T: for<'de> Deserialize<'de>>(body: &[u8]) -> Result<T, TxError> {
    serde_json::from_slice(body).map_err(|_| {
        TxError::request(
            "invalid_request",
            "Request body does not match the route schema",
        )
    })
}

fn common_config(commitment: Option<Commitment>, min_context_slot: Option<DecimalU64>) -> Value {
    let mut value = json!({ "commitment": commitment.unwrap_or(Commitment::Confirmed).as_str() });
    if let Some(slot) = min_context_slot {
        value["minContextSlot"] = json!(slot.0);
    }
    value
}

fn decode_bounded(value: &str, max: usize, field: &'static str) -> Result<Vec<u8>, TxError> {
    if value.len() > max.saturating_mul(4).div_ceil(3).saturating_add(4) {
        return Err(TxError::request(
            "transaction_too_large",
            format!("{field} exceeds the configured size limit"),
        ));
    }
    let decoded = BASE64_STANDARD.decode(value).map_err(|_| {
        TxError::request(
            "invalid_base64",
            format!("{field} must be canonical base64"),
        )
    })?;
    if decoded.len() > max {
        return Err(TxError::request(
            "transaction_too_large",
            format!("{field} exceeds the configured size limit"),
        ));
    }
    Ok(decoded)
}

fn transaction_signature(transaction: &[u8]) -> Result<String, TxError> {
    let (count, prefix_len) = short_vec_len(transaction)?;
    if count == 0 || count > 64 || transaction.len() < prefix_len + count * 64 + 1 {
        return Err(TxError::request(
            "invalid_transaction",
            "transaction has an invalid signature section",
        ));
    }
    let signatures = &transaction[prefix_len..prefix_len + count * 64];
    if signatures
        .chunks_exact(64)
        .any(|signature| signature.iter().all(|byte| *byte == 0))
    {
        return Err(TxError::request(
            "unsigned_transaction",
            "send requires every transaction signature",
        ));
    }
    Ok(bs58::encode(&signatures[..64]).into_string())
}

fn short_vec_len(bytes: &[u8]) -> Result<(usize, usize), TxError> {
    let mut value = 0usize;
    for (index, byte) in bytes.iter().copied().take(3).enumerate() {
        value |= ((byte & 0x7f) as usize) << (index * 7);
        if byte & 0x80 == 0 {
            if index > 0 && byte == 0 {
                break;
            }
            return Ok((value, index + 1));
        }
    }
    Err(TxError::request(
        "invalid_transaction",
        "transaction signature count is invalid",
    ))
}

fn valid_address(value: &str) -> bool {
    bs58::decode(value)
        .into_vec()
        .is_ok_and(|bytes| bytes.len() == 32)
}

fn valid_signature(value: &str) -> bool {
    bs58::decode(value)
        .into_vec()
        .is_ok_and(|bytes| bytes.len() == 64)
}

async fn rpc_call(
    state: &TransactionState,
    method: &'static str,
    params: Value,
    operation: Operation,
    signature: Option<String>,
) -> Result<Value, TxError> {
    let timeout = match operation {
        Operation::Send => state.config.send_timeout,
        Operation::SignatureStatus => state.config.status_timeout,
        _ => state.config.inspect_timeout,
    };
    let url = state
        .config
        .rpc_url
        .as_deref()
        .expect("validated transaction RPC URL");
    let response = state
        .client
        .post(url)
        .timeout(timeout)
        .json(&json!({ "jsonrpc": "2.0", "id": "arete-transaction", "method": method, "params": params }))
        .send()
        .await
        .map_err(|_| {
            record_upstream(state, operation, "transport_error");
            upstream_transport(operation, signature.clone())
        })?;
    if !response.status().is_success() {
        record_upstream(state, operation, "http_error");
        return Err(upstream_transport(operation, signature));
    }
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| upstream_transport(operation, signature.clone()))?;
        if bytes.len().saturating_add(chunk.len()) > MAX_UPSTREAM_RESPONSE_BYTES {
            return Err(upstream_malformed(
                "Transaction RPC response exceeded the size limit",
                operation,
                signature,
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    let response: Value = serde_json::from_slice(&bytes).map_err(|_| {
        upstream_malformed(
            "Malformed response from transaction RPC",
            operation,
            signature.clone(),
        )
    })?;
    if let Some(error) = response.get("error") {
        record_upstream(state, operation, "rpc_error");
        let code = error.get("code").and_then(Value::as_i64).unwrap_or(-32000);
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("Transaction RPC rejected the request")
            .chars()
            .take(256)
            .collect();
        let data = error.get("data").cloned().map(bound_error_data);
        return Err(TxError {
            status: StatusCode::BAD_GATEWAY,
            code: "upstream_rpc_error",
            message: "Transaction RPC rejected the request".into(),
            retryable: operation != Operation::Send,
            submission_state: (operation == Operation::Send).then_some("not_submitted"),
            signature,
            details: Some(Box::new(RpcErrorDetails {
                code,
                message,
                data,
            })),
            upstream_attempted: true,
        });
    }
    record_upstream(state, operation, "ok");
    response.get("result").cloned().ok_or_else(|| {
        upstream_malformed(
            "Malformed response from transaction RPC",
            operation,
            signature,
        )
    })
}

fn record_upstream(state: &TransactionState, operation: Operation, outcome: &'static str) {
    #[cfg(feature = "otel")]
    if let Some(metrics) = &state.metrics {
        metrics.record_transaction_upstream(operation.name(), outcome);
    }
    #[cfg(not(feature = "otel"))]
    let _ = (state, operation, outcome);
}

fn bound_error_data(value: Value) -> Value {
    let serialized = value.to_string();
    if serialized.len() <= 2048 {
        value
    } else {
        json!({ "truncated": true })
    }
}

fn upstream_transport(operation: Operation, signature: Option<String>) -> TxError {
    TxError {
        status: StatusCode::BAD_GATEWAY,
        code: if operation == Operation::Send {
            "submission_unknown"
        } else {
            "upstream_unavailable"
        },
        message: "Transaction RPC request failed".into(),
        retryable: operation != Operation::Send,
        submission_state: (operation == Operation::Send).then_some("unknown"),
        signature,
        details: None,
        upstream_attempted: true,
    }
}

fn upstream_malformed(
    message: &'static str,
    operation: Operation,
    signature: Option<String>,
) -> TxError {
    let mut error = upstream_transport(operation, signature);
    error.message = message.into();
    error
}

fn ambiguous_error(message: &'static str, signature: String) -> TxError {
    upstream_malformed(message, Operation::Send, Some(signature))
}

fn required_u64(value: &Value, pointer: &str) -> Result<u64, TxError> {
    value
        .pointer(pointer)
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            upstream_malformed(
                "Malformed response from transaction RPC",
                Operation::LatestBlockhash,
                None,
            )
        })
}

fn required_str<'a>(value: &'a Value, pointer: &str) -> Result<&'a str, TxError> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .ok_or_else(|| {
            upstream_malformed(
                "Malformed response from transaction RPC",
                Operation::LatestBlockhash,
                None,
            )
        })
}

fn simulation_response(value: Value) -> Result<Value, TxError> {
    let result = value.get("value").ok_or_else(|| {
        upstream_malformed("Malformed simulation response", Operation::Simulate, None)
    })?;
    Ok(json!({
        "contextSlot": required_u64(&value, "/context/slot")?.to_string(),
        "err": result.get("err").cloned().unwrap_or(Value::Null),
        "logs": result.get("logs").cloned().unwrap_or(Value::Null),
        "unitsConsumed": result
            .get("unitsConsumed")
            .and_then(Value::as_u64)
            .map(|number| number.to_string()),
        "accounts": result.get("accounts").cloned().unwrap_or(Value::Null),
    }))
}

fn trusted_client_ip(
    remote_addr: SocketAddr,
    headers: &hyper::HeaderMap,
    config: &TransactionConfig,
) -> IpAddr {
    if !config
        .trusted_proxy_cidrs
        .iter()
        .any(|network| network.contains(&remote_addr.ip()))
    {
        return remote_addr.ip();
    }
    headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.rsplit(',').next())
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or_else(|| remote_addr.ip())
}

fn transaction_response(
    status: StatusCode,
    request_id: &str,
    upstream_attempted: bool,
    value: Value,
) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "application/json")
        .header("X-Request-Id", request_id)
        .header(
            "X-Arete-Upstream-Attempted",
            if upstream_attempted { "true" } else { "false" },
        )
        .body(Full::new(Bytes::from(value.to_string())))
        .expect("valid transaction response")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::convert::Infallible;

    async fn mock_rpc(result: Value) -> String {
        use hyper::server::conn::http1;
        use hyper::service::service_fn;
        use hyper_util::rt::TokioIo;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let response = result.clone();
            http1::Builder::new()
                .serve_connection(
                    TokioIo::new(stream),
                    service_fn(move |_request| {
                        let response = response.clone();
                        async move {
                            Ok::<_, Infallible>(Response::new(Full::new(Bytes::from(
                                json!({
                                    "jsonrpc": "2.0",
                                    "id": "arete-transaction",
                                    "result": response,
                                })
                                .to_string(),
                            ))))
                        }
                    }),
                )
                .await
                .unwrap();
        });
        format!("http://{address}")
    }

    async fn state_for(result: Value) -> TransactionState {
        let config = TransactionConfig {
            enabled: true,
            rpc_url: Some(mock_rpc(result).await),
            ..TransactionConfig::default()
        };
        TransactionState::new(config).unwrap()
    }

    #[test]
    fn trusted_client_ip_uses_proxy_appended_address() {
        let config = TransactionConfig {
            trusted_proxy_cidrs: vec!["10.0.0.0/8".parse().unwrap()],
            ..TransactionConfig::default()
        };
        let mut headers = hyper::HeaderMap::new();
        headers.insert("x-forwarded-for", "1.2.3.4, 5.6.7.8".parse().unwrap());

        let proxy: SocketAddr = "10.1.2.3:9000".parse().unwrap();
        assert_eq!(
            trusted_client_ip(proxy, &headers, &config),
            "5.6.7.8".parse::<IpAddr>().unwrap()
        );

        let direct: SocketAddr = "192.168.1.1:9000".parse().unwrap();
        assert_eq!(
            trusted_client_ip(direct, &headers, &config),
            "192.168.1.1".parse::<IpAddr>().unwrap()
        );
    }

    #[test]
    fn route_allowlist_is_fixed() {
        assert_eq!(
            Operation::from_path("/transactions/v1/send"),
            Some(Operation::Send)
        );
        assert_eq!(Operation::from_path("/transactions/v1/get-anything"), None);
    }

    #[test]
    fn send_requires_nonzero_signatures_and_derives_first_signature() {
        let mut transaction = vec![1];
        transaction.extend(1u8..=64);
        transaction.push(0x80);
        assert_eq!(
            transaction_signature(&transaction).unwrap(),
            bs58::encode((1u8..=64).collect::<Vec<_>>()).into_string()
        );

        let mut unsigned = vec![1];
        unsigned.extend([0; 64]);
        unsigned.push(0x80);
        assert_eq!(
            transaction_signature(&unsigned).unwrap_err().code,
            "unsigned_transaction"
        );
    }

    #[test]
    fn request_types_reject_unknown_fields_and_numeric_u64s() {
        assert!(
            serde_json::from_value::<CommonRequest>(json!({ "method": "getBalance" })).is_err()
        );
        assert!(serde_json::from_value::<CommonRequest>(json!({ "minContextSlot": 42 })).is_err());
        assert!(serde_json::from_value::<CommonRequest>(json!({ "minContextSlot": "42" })).is_ok());
    }

    #[tokio::test]
    async fn dispatch_normalizes_rpc_results_to_the_public_camel_case_contract() {
        let latest_state = state_for(json!({
            "context": { "slot": 42 },
            "value": {
                "blockhash": "11111111111111111111111111111111",
                "lastValidBlockHeight": 99
            }
        }))
        .await;
        let latest = dispatch(
            Operation::LatestBlockhash,
            br#"{"commitment":"confirmed","minContextSlot":"41"}"#,
            None,
            &latest_state,
        )
        .await
        .unwrap();
        assert_eq!(latest["contextSlot"], "42");
        assert_eq!(latest["lastValidBlockHeight"], "99");

        let fee_state = state_for(json!({ "context": { "slot": 43 }, "value": 5000 })).await;
        let fee = dispatch(Operation::Fee, br#"{"message":"AQ=="}"#, None, &fee_state)
            .await
            .unwrap();
        assert_eq!(fee["feeLamports"], "5000");
        assert_eq!(fee["contextSlot"], "43");

        let simulation_state = state_for(json!({
            "context": { "slot": 44 },
            "value": { "err": null, "logs": ["ok"], "unitsConsumed": 12 }
        }))
        .await;
        let simulation = dispatch(
            Operation::Simulate,
            br#"{"transaction":"AQ==","innerInstructions":true}"#,
            None,
            &simulation_state,
        )
        .await
        .unwrap();
        assert_eq!(simulation["contextSlot"], "44");
        assert_eq!(simulation["unitsConsumed"], "12");
        assert_eq!(simulation["logs"], json!(["ok"]));
    }

    #[test]
    fn usage_events_exclude_transaction_material() {
        let event = TransactionUsageEvent {
            event_id: "event".into(),
            occurred_at_ms: 1,
            deployment_id: "deployment".into(),
            subject: Some("subject".into()),
            metering_key: Some("meter".into()),
            key_class: Some("secret"),
            plan: Some("plan".into()),
            operation: "send",
            result: "accepted",
            request_bytes: 123,
            response_bytes: 0,
            latency_ms: 4,
        };
        let value = serde_json::to_value(event).unwrap();

        assert_eq!(value["occurred_at_ms"], 1);
        assert_eq!(value["deployment_id"], "deployment");
        assert_eq!(value["subject"], "subject");
        assert_eq!(value["request_bytes"], 123);
        assert_eq!(value["response_bytes"], 0);

        for sensitive in [
            "transaction",
            "signature",
            "logs",
            "accounts",
            "credentials",
        ] {
            assert!(value.get(sensitive).is_none());
        }
    }
}
