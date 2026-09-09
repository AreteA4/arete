use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use arete_auth::{AuthContext, SCOPE_TRANSACTION_INSPECT, SCOPE_TRANSACTION_SEND};

use crate::account_policy::{redact_identity, AccountPolicyError, AccountPolicyRegistry};
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
    inflight: Arc<DashMap<String, u32>>,
    account_policies: Arc<AccountPolicyRegistry>,
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
        let state = Self {
            inspect_semaphore: Arc::new(Semaphore::new(config.inspect_concurrency)),
            send_semaphore: Arc::new(Semaphore::new(config.send_concurrency)),
            config: Arc::new(config),
            client,
            rate_buckets: Arc::new(DashMap::new()),
            inflight: Arc::new(DashMap::new()),
            account_policies: Arc::new(AccountPolicyRegistry::default()),
            usage_tx,
            #[cfg(feature = "otel")]
            metrics: None,
        };
        spawn_state_cleanup(
            state.rate_buckets.clone(),
            state.inflight.clone(),
            state.account_policies.clone(),
        );
        Ok(state)
    }

    /// Account policy state observed by the transaction relay.
    #[cfg(test)]
    pub(crate) fn account_policies(&self) -> &AccountPolicyRegistry {
        &self.account_policies
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
    SignatureStatuses,
    BlockHeight,
    Get,
    Signatures,
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
            "/transactions/v1/signature-statuses" => Some(Self::SignatureStatuses),
            "/transactions/v1/block-height" => Some(Self::BlockHeight),
            "/transactions/v1/get" => Some(Self::Get),
            "/transactions/v1/signatures" => Some(Self::Signatures),
            _ => None,
        }
    }

    fn scope(self) -> &'static str {
        if self == Self::Send {
            SCOPE_TRANSACTION_SEND
        } else {
            SCOPE_TRANSACTION_INSPECT
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::LatestBlockhash => "latest_blockhash",
            Self::Fee => "fee",
            Self::Simulate => "simulate",
            Self::Send => "send",
            Self::SignatureStatus => "signature_status",
            Self::SignatureStatuses => "signature_statuses",
            Self::BlockHeight => "block_height",
            Self::Get => "get",
            Self::Signatures => "signatures",
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

/// Solana's `getSignatureStatuses` resolves up to 256 signatures per call, so the batch is capped
/// there: one request in, one upstream call out.
///
/// A batch this size is roughly 23 KiB of JSON, which is why `TransactionConfig::max_body_bytes`
/// defaults above that. `a_full_batch_fits_the_default_body_limit` holds the two together.
const MAX_STATUS_BATCH: usize = 256;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SignatureStatusesRequest {
    signatures: Vec<String>,
    #[serde(default)]
    search_transaction_history: bool,
}

/// Highest transaction version `get` asks the cluster to encode. Balance deltas are
/// version-independent, so this only has to keep pace with the network — pinning it at 0 would
/// report every V1 transaction as unseen.
const MAX_SUPPORTED_TRANSACTION_VERSION: u8 = 1;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GetTransactionRequest {
    signature: String,
    #[serde(default)]
    commitment: Option<Commitment>,
    /// Numeric, not a decimal string: a version is not a u64 quantity.
    #[serde(default)]
    max_supported_transaction_version: Option<u8>,
}

/// Solana's `getSignaturesForAddress` ceiling, mirrored so an oversized page fails here rather
/// than as a remote 400.
const MAX_SIGNATURE_PAGE: u16 = 1_000;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SignaturesRequest {
    address: String,
    /// A count, not a lamport quantity, so it stays a JSON number.
    #[serde(default)]
    limit: Option<u16>,
    #[serde(default)]
    before: Option<String>,
    #[serde(default)]
    until: Option<String>,
    #[serde(default)]
    commitment: Option<Commitment>,
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
    let mut upstream_attempted = false;
    let result = dispatch(
        operation,
        &body,
        auth.as_ref(),
        &state,
        &mut upstream_attempted,
    )
    .await;
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
        Ok(value) => transaction_response(StatusCode::OK, &request_id, upstream_attempted, value),
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
    inflight_keys: Vec<String>,
    inflight: Arc<DashMap<String, u32>>,
}

impl Drop for Admission {
    fn drop(&mut self) {
        for key in &self.inflight_keys {
            if let Some(mut count) = self.inflight.get_mut(key) {
                *count = count.saturating_sub(1);
            }
        }
    }
}

fn acquire_inflight(state: &TransactionState, key: &str, max: u32) -> Result<(), TxError> {
    let mut count = state.inflight.entry(key.to_string()).or_insert(0);
    if *count >= max {
        #[cfg(feature = "otel")]
        if let Some(metrics) = &state.metrics {
            metrics.record_transaction_denial("concurrency");
        }
        return Err(limit_error("transaction concurrency limit exceeded"));
    }
    *count += 1;
    Ok(())
}

fn policy_admission_error(account: &str, error: AccountPolicyError) -> TxError {
    let (status, code, message) = match &error {
        AccountPolicyError::StaleVersion { presented, current } => {
            tracing::debug!(
                account = %redact_identity(account),
                presented,
                current,
                "stale policy version rejected"
            );
            (
                StatusCode::UNAUTHORIZED,
                "stale_policy_version",
                "Session policy version is stale; refresh the session token",
            )
        }
        AccountPolicyError::ConflictingLimits { version } => {
            tracing::warn!(
                account = %redact_identity(account),
                version,
                "signed account limits conflict for one policy version"
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "policy_conflict",
                "Signed account limits conflict with previously observed policy",
            )
        }
        AccountPolicyError::CapacityExhausted => (
            StatusCode::SERVICE_UNAVAILABLE,
            "server_busy",
            "Account policy state is at capacity",
        ),
    };
    TxError {
        status,
        code,
        message: message.into(),
        retryable: matches!(error, AccountPolicyError::CapacityExhausted),
        submission_state: Some("not_submitted"),
        signature: None,
        details: None,
        upstream_attempted: false,
    }
}

fn spawn_state_cleanup(
    rate_buckets: Arc<DashMap<String, (u64, u32)>>,
    inflight: Arc<DashMap<String, u32>>,
    account_policies: Arc<AccountPolicyRegistry>,
) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        loop {
            interval.tick().await;
            let minute = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
                / 60;
            rate_buckets.retain(|_, (bucket_minute, _)| minute.saturating_sub(*bucket_minute) <= 1);
            inflight.retain(|_, count| *count > 0);
            account_policies.evict_idle(|_| false);
        }
    });
}

async fn admit(
    operation: Operation,
    auth: Option<&AuthContext>,
    client_ip: IpAddr,
    state: &TransactionState,
) -> Result<Admission, TxError> {
    let server_limit = match operation {
        Operation::Send => state.config.send_requests_per_minute,
        Operation::SignatureStatus | Operation::SignatureStatuses => {
            state.config.status_requests_per_minute
        }
        _ => state.config.inspect_requests_per_minute,
    };
    let claim_limit = auth.and_then(|context| match operation {
        Operation::Send => context.limits.max_transaction_send_requests_per_minute,
        Operation::SignatureStatus | Operation::SignatureStatuses => {
            context.limits.max_transaction_status_requests_per_minute
        }
        _ => context.limits.max_transaction_inspect_requests_per_minute,
    });
    let limit = claim_limit.unwrap_or(server_limit).min(server_limit);
    let class = match operation {
        Operation::Send => "send",
        Operation::SignatureStatus | Operation::SignatureStatuses => "status",
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
    let mut admission = Admission {
        _permit: permit,
        inflight_keys: Vec::new(),
        inflight: state.inflight.clone(),
    };
    if let Some(context) = auth {
        if context.is_legacy_policy() {
            // Legacy tokens keep the old subject-keyed buckets and inflight
            // accounting exactly; count them so Plan 030 can end compatibility.
            let legacy_policy_token = state.account_policies.record_legacy_token();
            tracing::debug!(legacy_policy_token, "legacy policy token admitted");
            check_bucket(
                &state.rate_buckets,
                format!("subject:{}:{class}", context.subject),
                limit,
            )?;
            if let Some(max) = context.limits.max_transaction_concurrency {
                let key = format!("subject:{}", context.subject);
                acquire_inflight(state, &key, max)?;
                admission.inflight_keys.push(key);
            }
        } else {
            if let (Some(account), Some(policy_version)) =
                (context.account_key.as_deref(), context.policy_version)
            {
                state
                    .account_policies
                    .observe(account, policy_version, &context.account_limits)
                    .map_err(|error| policy_admission_error(account, error))?;
            }

            // Per-consumer operation rate from `limits`, hard-capped by the
            // runtime configuration.
            check_bucket(
                &state.rate_buckets,
                format!("consumer:{}:{class}", context.consumer_key()),
                limit,
            )?;

            // Aggregate account operation rate from the signed
            // `account_limits`, enforced only when present.
            if context.account_key.is_some() {
                let account_claim = match operation {
                    Operation::Send => {
                        context
                            .account_limits
                            .max_transaction_send_requests_per_minute
                    }
                    Operation::SignatureStatus | Operation::SignatureStatuses => {
                        context
                            .account_limits
                            .max_transaction_status_requests_per_minute
                    }
                    _ => {
                        context
                            .account_limits
                            .max_transaction_inspect_requests_per_minute
                    }
                };
                if let Some(account_limit) = account_claim {
                    check_bucket(
                        &state.rate_buckets,
                        format!("account:{}:{class}", context.account_key()),
                        account_limit.min(server_limit),
                    )?;
                }
            }

            if let Some(max) = context.limits.max_transaction_concurrency {
                let key = format!("consumer:{}", context.consumer_key());
                acquire_inflight(state, &key, max)?;
                admission.inflight_keys.push(key);
            }
            if context.account_key.is_some() {
                if let Some(max) = context.account_limits.max_transaction_concurrency {
                    let key = format!("account:{}", context.account_key());
                    acquire_inflight(state, &key, max)?;
                    admission.inflight_keys.push(key);
                }
            }
        }
    }
    Ok(admission)
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
    upstream_attempted: &mut bool,
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
                upstream_attempted,
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
                upstream_attempted,
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
                upstream_attempted,
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
                upstream_attempted,
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
                upstream_attempted,
            )
            .await?;
            let status = value.pointer("/value/0").cloned().unwrap_or(Value::Null);
            Ok(json!({ "status": status_json(&status)? }))
        }
        Operation::SignatureStatuses => {
            let request: SignatureStatusesRequest = parse_json(body)?;
            if request.signatures.is_empty() {
                return Ok(json!({ "statuses": [] }));
            }
            if request.signatures.len() > MAX_STATUS_BATCH {
                return Err(TxError::request(
                    "batch_limit_exceeded",
                    format!(
                        "signatures exceeds the {MAX_STATUS_BATCH}-signature limit for one batch"
                    ),
                ));
            }
            if let Some(invalid) = request
                .signatures
                .iter()
                .find(|signature| !valid_signature(signature))
            {
                return Err(TxError::request(
                    "invalid_signature",
                    format!("every signature must be a base58-encoded 64-byte value: {invalid}"),
                ));
            }

            let value = rpc_call(
                state,
                "getSignatureStatuses",
                json!([request.signatures, { "searchTransactionHistory": request.search_transaction_history }]),
                operation,
                None,
                upstream_attempted,
            )
            .await?;

            let statuses = value
                .pointer("/value")
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    upstream_malformed("Malformed signature status response", operation, None)
                })?;

            // Results are read positionally, so a short array would attribute one signature's
            // outcome to another. Reject rather than truncate.
            if statuses.len() != request.signatures.len() {
                return Err(upstream_malformed(
                    "Signature status response did not match the requested signatures",
                    operation,
                    None,
                ));
            }

            Ok(json!({
                "statuses": statuses
                    .iter()
                    .map(status_json)
                    .collect::<Result<Vec<_>, _>>()?
            }))
        }
        Operation::BlockHeight => {
            let request: CommonRequest = parse_json(body)?;
            let config = common_config(request.commitment, request.min_context_slot);
            let value = rpc_call(
                state,
                "getBlockHeight",
                json!([config]),
                operation,
                None,
                upstream_attempted,
            )
            .await?;
            let height = value.as_u64().ok_or_else(|| {
                upstream_malformed("Malformed block height response", operation, None)
            })?;
            Ok(json!({ "blockHeight": height.to_string() }))
        }
        Operation::Get => {
            let request: GetTransactionRequest = parse_json(body)?;
            if !valid_signature(&request.signature) {
                return Err(TxError::request(
                    "invalid_signature",
                    "signature must be a base58-encoded 64-byte value",
                ));
            }
            let commitment = request.commitment.unwrap_or(Commitment::Finalized);
            if matches!(commitment, Commitment::Processed) {
                return Err(TxError::request(
                    "invalid_commitment",
                    "getTransaction accepts confirmed or finalized, not processed",
                ));
            }
            let value = rpc_call(
                state,
                "getTransaction",
                json!([
                    request.signature,
                    {
                        // `jsonParsed` is what resolves lookup-table addresses into `accountKeys`.
                        // Under `json` the balance arrays outrun the key list on any v0
                        // transaction and the extra accounts vanish silently.
                        "encoding": "jsonParsed",
                        "commitment": commitment.as_str(),
                        "maxSupportedTransactionVersion": request
                            .max_supported_transaction_version
                            .unwrap_or(MAX_SUPPORTED_TRANSACTION_VERSION),
                    }
                ]),
                operation,
                None,
                upstream_attempted,
            )
            .await?;
            Ok(json!({
                "transaction": confirmed_transaction_json(&request.signature, &value)?
            }))
        }
        Operation::Signatures => {
            let request: SignaturesRequest = parse_json(body)?;
            if !valid_address(&request.address) {
                return Err(TxError::request(
                    "invalid_address",
                    "address must be a base58-encoded 32-byte value",
                ));
            }
            for (field, cursor) in [("before", &request.before), ("until", &request.until)] {
                if cursor
                    .as_deref()
                    .is_some_and(|value| !valid_signature(value))
                {
                    return Err(TxError::request(
                        "invalid_signature",
                        format!("{field} must be a base58-encoded 64-byte value"),
                    ));
                }
            }
            let limit = request.limit.unwrap_or(MAX_SIGNATURE_PAGE);
            if limit == 0 || limit > MAX_SIGNATURE_PAGE {
                return Err(TxError::request(
                    "invalid_limit",
                    format!("limit must be between 1 and {MAX_SIGNATURE_PAGE}"),
                ));
            }
            let commitment = request.commitment.unwrap_or(Commitment::Finalized);
            if matches!(commitment, Commitment::Processed) {
                return Err(TxError::request(
                    "invalid_commitment",
                    "getSignaturesForAddress accepts confirmed or finalized, not processed",
                ));
            }
            let mut config = json!({ "limit": limit, "commitment": commitment.as_str() });
            if let Some(before) = &request.before {
                config["before"] = json!(before);
            }
            if let Some(until) = &request.until {
                config["until"] = json!(until);
            }
            let value = rpc_call(
                state,
                "getSignaturesForAddress",
                json!([request.address, config]),
                operation,
                None,
                upstream_attempted,
            )
            .await?;
            let entries = value.as_array().ok_or_else(|| {
                upstream_malformed("Malformed signatures response", operation, None)
            })?;
            Ok(json!({
                "signatures": entries
                    .iter()
                    .map(signature_entry_json)
                    .collect::<Result<Vec<_>, _>>()?
            }))
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

/// v1 (SIMD-0385) leads with version byte `129` and moves the signature array to the tail, with
/// `num_required_signatures` entries and no length prefix. legacy/v0 lead with a shortvec count.
const V1_VERSION_BYTE: u8 = 129;
/// version, legacy header, config mask, lifetime specifier, instruction and address counts.
const V1_MIN_BODY_BYTES: usize = 1 + 3 + 4 + 32 + 1 + 1;
/// SIMD-0385 caps a v1 transaction at 12 signatures.
const V1_MAX_SIGNATURES: usize = 12;

fn transaction_signature(transaction: &[u8]) -> Result<String, TxError> {
    let (count, offset) = if transaction.first() == Some(&V1_VERSION_BYTE) {
        let count = usize::from(transaction.get(1).copied().unwrap_or(0));
        let body = transaction.len().saturating_sub(count * 64);
        if count == 0 || count > V1_MAX_SIGNATURES || body < V1_MIN_BODY_BYTES {
            return Err(invalid_signature_section());
        }
        (count, body)
    } else {
        let (count, prefix_len) = short_vec_len(transaction)?;
        if count == 0 || count > 64 || transaction.len() < prefix_len + count * 64 + 1 {
            return Err(invalid_signature_section());
        }
        (count, prefix_len)
    };
    let signatures = &transaction[offset..offset + count * 64];
    if signatures
        .as_chunks::<64>()
        .0
        .iter()
        .any(|signature| signature.iter().all(|byte| *byte == 0))
    {
        return Err(TxError::request(
            "unsigned_transaction",
            "send requires every transaction signature",
        ));
    }
    Ok(bs58::encode(&signatures[..64]).into_string())
}

fn invalid_signature_section() -> TxError {
    TxError::request(
        "invalid_transaction",
        "transaction has an invalid signature section",
    )
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

/// One `getSignatureStatuses` entry in the wire shape, or `null` when the cluster has not seen
/// the signature. Shared so the single and batch routes cannot describe a status differently.
fn status_json(status: &Value) -> Result<Value, TxError> {
    if status.is_null() {
        return Ok(Value::Null);
    }
    Ok(json!({
        "slot": required_u64(status, "/slot")?.to_string(),
        "confirmations": status
            .get("confirmations")
            .and_then(Value::as_u64)
            .map(|value| value.to_string()),
        "confirmationStatus": status.get("confirmationStatus"),
        "err": status.get("err")
    }))
}

/// Reshape one `getSignaturesForAddress` entry. `memo` and `confirmationStatus` are dropped: the
/// page is a cursor over history, and a caller that wants detail asks `get` for the signature.
fn signature_entry_json(entry: &Value) -> Result<Value, TxError> {
    Ok(json!({
        "signature": required_str(entry, "/signature")?,
        "slot": required_u64(entry, "/slot")?.to_string(),
        "blockTime": entry
            .pointer("/blockTime")
            .and_then(Value::as_i64)
            .map(|seconds| seconds.to_string()),
        "err": entry.pointer("/err").cloned().unwrap_or(Value::Null),
    }))
}

/// Reshape one `getTransaction` result down to what payout verification needs: who held what
/// before, who holds what after. `null` when the cluster has not seen the signature at the
/// requested commitment.
fn confirmed_transaction_json(signature: &str, value: &Value) -> Result<Value, TxError> {
    if value.is_null() {
        return Ok(Value::Null);
    }
    let keys = value
        .pointer("/transaction/message/accountKeys")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            upstream_malformed("Malformed transaction response", Operation::Get, None)
        })?;
    let pre = balances(value, "/meta/preBalances")?;
    let post = balances(value, "/meta/postBalances")?;
    // Balances are positional against the resolved key list; a short array would credit one
    // account's movement to another.
    if pre.len() != keys.len() || post.len() != keys.len() {
        return Err(upstream_malformed(
            "Transaction balances did not match its account keys",
            Operation::Get,
            None,
        ));
    }
    let accounts = keys
        .iter()
        .zip(pre)
        .zip(post)
        .map(|((key, pre), post)| {
            let pubkey = key.get("pubkey").and_then(Value::as_str).ok_or_else(|| {
                upstream_malformed("Malformed transaction account key", Operation::Get, None)
            })?;
            Ok(json!({
                "pubkey": pubkey,
                "preBalance": pre.to_string(),
                "postBalance": post.to_string(),
            }))
        })
        .collect::<Result<Vec<_>, TxError>>()?;
    Ok(json!({
        "signature": signature,
        "slot": required_u64(value, "/slot")?.to_string(),
        "blockTime": value
            .pointer("/blockTime")
            .and_then(Value::as_i64)
            .map(|seconds| seconds.to_string()),
        "err": value.pointer("/meta/err").cloned().unwrap_or(Value::Null),
        "accounts": accounts,
    }))
}

fn balances(value: &Value, pointer: &str) -> Result<Vec<u64>, TxError> {
    value
        .pointer(pointer)
        .and_then(Value::as_array)
        .and_then(|entries| {
            entries
                .iter()
                .map(Value::as_u64)
                .collect::<Option<Vec<_>>>()
        })
        .ok_or_else(|| upstream_malformed("Malformed transaction balances", Operation::Get, None))
}

async fn rpc_call(
    state: &TransactionState,
    method: &'static str,
    params: Value,
    operation: Operation,
    signature: Option<String>,
    upstream_attempted: &mut bool,
) -> Result<Value, TxError> {
    // Recorded here rather than assumed by the caller: `X-Arete-Upstream-Attempted` is a claim
    // about whether the relay reached the cluster, and only this function can know. A dispatch arm
    // that short-circuits — the empty status batch does — must not report an attempt it never made.
    *upstream_attempted = true;
    let timeout = match operation {
        Operation::Send => state.config.send_timeout,
        Operation::SignatureStatus | Operation::SignatureStatuses => state.config.status_timeout,
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
        // V1 budgets cannot be estimated without it: a caller has to know how much account data
        // the simulated transaction actually loaded before it can set a limit for the real one.
        // Absent stays absent — an older upstream that never reports it must not read as zero.
        "loadedAccountsDataSize": result
            .get("loadedAccountsDataSize")
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
        assert_eq!(
            Operation::from_path("/transactions/v1/signature-statuses"),
            Some(Operation::SignatureStatuses)
        );
        assert_eq!(
            Operation::from_path("/transactions/v1/get"),
            Some(Operation::Get)
        );
        assert_eq!(
            Operation::from_path("/transactions/v1/signatures"),
            Some(Operation::Signatures)
        );
        assert_eq!(Operation::Signatures.scope(), "transaction:inspect");
        // A history read, never the send scope.
        assert_eq!(Operation::Get.scope(), "transaction:inspect");
        // The batch reads chain state, so it must not require the send scope.
        assert_eq!(Operation::SignatureStatuses.scope(), "transaction:inspect");
        assert_eq!(Operation::Send.scope(), "transaction:send");
        assert_eq!(Operation::Simulate.scope(), "transaction:inspect");
    }

    #[tokio::test]
    async fn signatures_pages_history_in_the_cluster_order() {
        let address = bs58::encode([3u8; 32]).into_string();
        let body = json!({ "address": address, "limit": 2, "before": sig(4) }).to_string();
        let state = state_for(json!([
            { "signature": "newer", "slot": 12, "blockTime": 1_757_222_400i64, "err": null,
              "memo": null, "confirmationStatus": "finalized" },
            { "signature": "older", "slot": 11, "blockTime": null,
              "err": { "InstructionError": [0, "Custom"] } }
        ]))
        .await;

        let value = dispatch(
            Operation::Signatures,
            body.as_bytes(),
            None,
            &state,
            &mut false,
        )
        .await
        .unwrap();

        assert_eq!(
            value,
            json!({ "signatures": [
                { "signature": "newer", "slot": "12", "blockTime": "1757222400", "err": null },
                { "signature": "older", "slot": "11", "blockTime": null,
                  "err": { "InstructionError": [0, "Custom"] } }
            ]})
        );
    }

    /// Bad input must fail here, not as a remote 400 the caller cannot read.
    #[tokio::test]
    async fn signatures_rejects_a_bad_address_cursor_or_page_size() {
        let state = state_for(json!([])).await;
        let address = bs58::encode([3u8; 32]).into_string();
        let cases = [
            (json!({ "address": "not-base58!" }), "invalid_address"),
            (
                json!({ "address": address, "before": "nope" }),
                "invalid_signature",
            ),
            (json!({ "address": address, "limit": 0 }), "invalid_limit"),
            (
                json!({ "address": address, "limit": 1001 }),
                "invalid_limit",
            ),
            (
                json!({ "address": address, "commitment": "processed" }),
                "invalid_commitment",
            ),
        ];
        for (body, expected) in cases {
            let error = dispatch(
                Operation::Signatures,
                body.to_string().as_bytes(),
                None,
                &state,
                &mut false,
            )
            .await
            .expect_err("rejected before the cluster");
            assert_eq!(error.code, expected);
            assert!(!error.upstream_attempted);
        }
    }

    /// `jsonParsed` appends lookup-table accounts to `accountKeys` and the balance arrays cover
    /// them, so a winner paid through an ALT has to survive the reshape.
    #[tokio::test]
    async fn get_pairs_every_resolved_account_with_its_balances() {
        let signature = sig(7);
        let body = json!({ "signature": signature }).to_string();
        let state = state_for(json!({
            "slot": 319_482_771u64,
            "blockTime": 1_757_222_400i64,
            "meta": { "err": null, "preBalances": [5000, 10], "postBalances": [3995, 1010] },
            "transaction": { "message": { "accountKeys": [
                { "pubkey": "vault", "source": "transaction" },
                { "pubkey": "winner", "source": "lookupTable" }
            ] } }
        }))
        .await;

        let value = dispatch(Operation::Get, body.as_bytes(), None, &state, &mut false)
            .await
            .unwrap();

        assert_eq!(
            value,
            json!({ "transaction": {
                "signature": signature,
                "slot": "319482771",
                "blockTime": "1757222400",
                "err": null,
                "accounts": [
                    { "pubkey": "vault", "preBalance": "5000", "postBalance": "3995" },
                    { "pubkey": "winner", "preBalance": "10", "postBalance": "1010" }
                ]
            }})
        );
    }

    #[tokio::test]
    async fn get_answers_null_for_an_unseen_signature() {
        let body = json!({ "signature": sig(9) }).to_string();
        let state = state_for(Value::Null).await;
        let value = dispatch(Operation::Get, body.as_bytes(), None, &state, &mut false)
            .await
            .unwrap();
        assert_eq!(value, json!({ "transaction": null }));
    }

    /// Truncating instead of rejecting would credit one account's movement to another.
    #[tokio::test]
    async fn get_rejects_balances_that_do_not_cover_every_account() {
        let body = json!({ "signature": sig(11) }).to_string();
        let state = state_for(json!({
            "slot": 1u64,
            "meta": { "err": null, "preBalances": [5000], "postBalances": [3995] },
            "transaction": { "message": { "accountKeys": [
                { "pubkey": "vault" },
                { "pubkey": "winner" }
            ] } }
        }))
        .await;
        assert!(
            dispatch(Operation::Get, body.as_bytes(), None, &state, &mut false)
                .await
                .is_err()
        );
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
    fn send_derives_the_first_signature_from_a_v1_tail() {
        // version | header(3) | mask(4) | lifetime(32) | numIx | numAddresses | one address
        let mut transaction = vec![V1_VERSION_BYTE, 1, 0, 0];
        transaction.extend([0; 4]);
        transaction.extend([7; 32]);
        transaction.extend([0, 1]);
        transaction.extend([9; 32]);
        let body = transaction.len();
        transaction.extend(1u8..=64);
        assert_eq!(
            transaction_signature(&transaction).unwrap(),
            bs58::encode((1u8..=64).collect::<Vec<_>>()).into_string()
        );

        // A legacy parse of the same bytes reads a 129-signature shortvec and rejects it.
        assert!(short_vec_len(&transaction).unwrap().0 > 64);

        let mut unsigned = transaction[..body].to_vec();
        unsigned.extend([0; 64]);
        assert_eq!(
            transaction_signature(&unsigned).unwrap_err().code,
            "unsigned_transaction"
        );

        let truncated = transaction[..body].to_vec();
        assert_eq!(
            transaction_signature(&truncated).unwrap_err().code,
            "invalid_transaction"
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
            &mut false,
        )
        .await
        .unwrap();
        assert_eq!(latest["contextSlot"], "42");
        assert_eq!(latest["lastValidBlockHeight"], "99");

        let fee_state = state_for(json!({ "context": { "slot": 43 }, "value": 5000 })).await;
        let fee = dispatch(
            Operation::Fee,
            br#"{"message":"AQ=="}"#,
            None,
            &fee_state,
            &mut false,
        )
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
            &mut false,
        )
        .await
        .unwrap();
        assert_eq!(simulation["contextSlot"], "44");
        assert_eq!(simulation["unitsConsumed"], "12");
        assert_eq!(simulation["logs"], json!(["ok"]));
    }

    /// V1 budgets cannot be estimated without the loaded-account size, so the relay has to carry
    /// it. Absent must stay absent: an older upstream that never reports it is not reporting zero.
    #[tokio::test]
    async fn transaction_v1_simulation_carries_the_loaded_accounts_data_size() {
        let reported = state_for(json!({
            "context": { "slot": 44 },
            "value": { "err": null, "unitsConsumed": 12, "loadedAccountsDataSize": 65_536u64 }
        }))
        .await;
        let value = dispatch(
            Operation::Simulate,
            br#"{"transaction":"AQ=="}"#,
            None,
            &reported,
            &mut false,
        )
        .await
        .unwrap();
        assert_eq!(value["loadedAccountsDataSize"], "65536");

        let zero = state_for(json!({
            "context": { "slot": 44 },
            "value": { "err": null, "loadedAccountsDataSize": 0u64 }
        }))
        .await;
        let value = dispatch(
            Operation::Simulate,
            br#"{"transaction":"AQ=="}"#,
            None,
            &zero,
            &mut false,
        )
        .await
        .unwrap();
        assert_eq!(
            value["loadedAccountsDataSize"], "0",
            "zero is a measurement"
        );

        let silent = state_for(json!({
            "context": { "slot": 44 },
            "value": { "err": null }
        }))
        .await;
        let value = dispatch(
            Operation::Simulate,
            br#"{"transaction":"AQ=="}"#,
            None,
            &silent,
            &mut false,
        )
        .await
        .unwrap();
        assert_eq!(value["loadedAccountsDataSize"], Value::Null);
    }

    /// Real signed payloads from `tests/fixtures/transaction-v1`, produced by @solana/kit 8.2.0.
    /// Hand-assembled bytes would only prove the parser agrees with itself.
    fn fixture(name: &str) -> (String, Value) {
        let raw = include_str!("../../../../tests/fixtures/transaction-v1/transactions.json");
        let corpus: Value = serde_json::from_str(raw).expect("fixture corpus parses");
        let entry = corpus["fixtures"][name].clone();
        (entry["base64"].as_str().expect("base64").to_string(), entry)
    }

    /// The signature the relay derives for a submitted transaction is what a caller reconciles
    /// against after an ambiguous send, so it has to match the codec's own, in every version.
    #[test]
    fn transaction_v1_fixtures_yield_the_codec_signature_in_every_version() {
        use base64::Engine as _;
        for name in ["legacy", "v0", "v1", "v1_oversize", "v1_two_signatures"] {
            let (encoded, entry) = fixture(name);
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(&encoded)
                .expect("fixture decodes");
            assert_eq!(
                bytes.len(),
                entry["bytes"].as_u64().expect("bytes") as usize,
                "{name} length"
            );
            assert_eq!(
                transaction_signature(&bytes).expect("{name} signature"),
                entry["firstSignature"].as_str().expect("firstSignature"),
                "{name} first signature"
            );
        }
    }

    /// 1574 bytes: refused by anything still applying the legacy 1232-byte ceiling, accepted under
    /// V1's 4096. The minimal 177-byte V1 fixture passes either way and proves nothing here.
    #[test]
    fn transaction_v1_oversize_fixture_needs_the_v1_size_ceiling() {
        let (encoded, _) = fixture("v1_oversize");
        assert!(decode_bounded(&encoded, 1232, "transaction").is_err());
        let decoded = decode_bounded(&encoded, 4096, "transaction").expect("within the V1 ceiling");
        assert_eq!(decoded.first(), Some(&V1_VERSION_BYTE));
    }

    /// A valid base58 64-byte signature; the handler rejects anything else before dispatching.
    fn sig(seed: u8) -> String {
        bs58::encode(vec![seed; 64]).into_string()
    }

    /// `X-Arete-Upstream-Attempted` is a claim about whether the relay reached the cluster, so the
    /// empty batch — which answers without calling upstream — must not assert one. The flag used to
    /// be hardcoded `true` for every successful dispatch.
    #[tokio::test]
    async fn an_empty_batch_reports_no_upstream_attempt() {
        let state = state_for(json!({ "context": { "slot": 50 }, "value": [] })).await;
        let body = json!({ "signatures": [] }).to_string();
        let mut upstream_attempted = false;

        let value = dispatch(
            Operation::SignatureStatuses,
            body.as_bytes(),
            None,
            &state,
            &mut upstream_attempted,
        )
        .await
        .expect("an empty batch succeeds");

        assert_eq!(value["statuses"], json!([]));
        assert!(
            !upstream_attempted,
            "no upstream call was made, so none may be reported"
        );
    }

    /// The same flag must still be set on the path that does reach upstream, or the header becomes
    /// uniformly false and equally useless.
    #[tokio::test]
    async fn a_non_empty_batch_reports_an_upstream_attempt() {
        let state = state_for(json!({
            "context": { "slot": 50 },
            "value": [
                { "slot": 100, "confirmations": 3, "confirmationStatus": "confirmed", "err": null }
            ]
        }))
        .await;
        let body = json!({ "signatures": [sig(1)] }).to_string();
        let mut upstream_attempted = false;

        dispatch(
            Operation::SignatureStatuses,
            body.as_bytes(),
            None,
            &state,
            &mut upstream_attempted,
        )
        .await
        .expect("a one-signature batch succeeds");

        assert!(upstream_attempted, "the relay did call upstream");
    }

    #[tokio::test]
    async fn batch_signature_statuses_keep_absent_signatures_in_place() {
        let state = state_for(json!({
            "context": { "slot": 50 },
            "value": [
                { "slot": 100, "confirmations": 3, "confirmationStatus": "confirmed", "err": null },
                null,
                { "slot": 102, "confirmations": null, "confirmationStatus": "finalized", "err": null }
            ]
        }))
        .await;

        let body = json!({ "signatures": [sig(1), sig(2), sig(3)] }).to_string();
        let result = dispatch(
            Operation::SignatureStatuses,
            body.as_bytes(),
            None,
            &state,
            &mut false,
        )
        .await
        .unwrap();

        let statuses = result["statuses"].as_array().expect("statuses array");
        assert_eq!(statuses.len(), 3);
        assert_eq!(statuses[0]["slot"], "100");
        assert_eq!(statuses[0]["confirmations"], "3");
        // The middle signature is unseen; it must stay null rather than shifting the third up.
        assert!(statuses[1].is_null());
        assert_eq!(statuses[2]["confirmationStatus"], "finalized");
    }

    #[tokio::test]
    async fn a_short_status_array_is_rejected_rather_than_misattributed() {
        let state = state_for(json!({ "context": { "slot": 50 }, "value": [null] })).await;

        let body = json!({ "signatures": [sig(1), sig(2)] }).to_string();
        let error = dispatch(
            Operation::SignatureStatuses,
            body.as_bytes(),
            None,
            &state,
            &mut false,
        )
        .await
        .expect_err("a short array must not be accepted");
        assert_eq!(error.code, "upstream_unavailable");
        assert!(
            error.message.contains("did not match"),
            "unexpected message: {}",
            error.message
        );
    }

    /// The advertised batch has to survive the default configuration, or the capacity is a lie:
    /// `read_bounded_body` rejects the request as `request_too_large` before batch validation runs,
    /// so a caller sending the documented maximum gets a size error and never learns the batch was
    /// otherwise fine. 4 KiB admitted roughly 44 signatures.
    #[test]
    fn a_full_batch_fits_the_default_body_limit() {
        let signatures: Vec<String> = (0..MAX_STATUS_BATCH).map(|i| sig(i as u8)).collect();
        let body = json!({ "signatures": signatures }).to_string();
        let limit = crate::config::TransactionConfig::default().max_body_bytes;

        assert!(
            body.len() <= limit,
            "a {MAX_STATUS_BATCH}-signature batch is {} bytes, over the {limit}-byte default",
            body.len()
        );
    }

    #[tokio::test]
    async fn oversized_and_malformed_status_batches_are_refused_locally() {
        let state = state_for(json!({ "context": { "slot": 50 }, "value": [] })).await;

        let too_many: Vec<String> = (0..=MAX_STATUS_BATCH).map(|i| sig(i as u8)).collect();
        let body = json!({ "signatures": too_many }).to_string();
        assert_eq!(
            dispatch(
                Operation::SignatureStatuses,
                body.as_bytes(),
                None,
                &state,
                &mut false,
            )
            .await
            .expect_err("over the cap")
            .code,
            "batch_limit_exceeded"
        );

        let body = json!({ "signatures": ["not-a-signature"] }).to_string();
        assert_eq!(
            dispatch(
                Operation::SignatureStatuses,
                body.as_bytes(),
                None,
                &state,
                &mut false,
            )
            .await
            .expect_err("bad signature")
            .code,
            "invalid_signature"
        );

        // Empty short-circuits without an upstream call.
        let body = json!({ "signatures": [] }).to_string();
        let result = dispatch(
            Operation::SignatureStatuses,
            body.as_bytes(),
            None,
            &state,
            &mut false,
        )
        .await
        .unwrap();
        assert_eq!(result["statuses"], json!([]));
    }

    fn v2_context(
        consumer: &str,
        account: &str,
        limits: arete_auth::Limits,
        account_limits: arete_auth::Limits,
    ) -> AuthContext {
        AuthContext::from_claims(
            arete_auth::SessionClaims::builder("issuer", "user:1", "aud")
                .with_metering_key(account)
                .with_plan("pro")
                .with_actor_key("user:1")
                .with_account_key(account)
                .with_consumer_key(consumer)
                .with_policy_version(1)
                .with_limits(limits)
                .with_account_limits(account_limits)
                .build(),
        )
    }

    fn legacy_context(subject: &str, limits: arete_auth::Limits) -> AuthContext {
        AuthContext::from_claims(
            arete_auth::SessionClaims::builder("issuer", subject, "aud")
                .with_metering_key("api_key:1")
                .with_limits(limits)
                .build(),
        )
    }

    #[tokio::test]
    async fn account_transaction_concurrency_is_aggregate_across_consumers() {
        let state = state_for(Value::Null).await;
        let account_limits = arete_auth::Limits {
            max_transaction_concurrency: Some(1),
            ..arete_auth::Limits::default()
        };
        let ip: IpAddr = "1.1.1.1".parse().unwrap();

        let consumer_a = v2_context(
            "consumer:a",
            "account:42",
            arete_auth::Limits::default(),
            account_limits.clone(),
        );
        let consumer_b = v2_context(
            "consumer:b",
            "account:42",
            arete_auth::Limits::default(),
            account_limits.clone(),
        );
        let other_account = v2_context(
            "consumer:c",
            "account:43",
            arete_auth::Limits::default(),
            account_limits,
        );

        let held = admit(Operation::Simulate, Some(&consumer_a), ip, &state)
            .await
            .unwrap();

        // A sibling consumer under the same account is blocked by the
        // aggregate concurrency cap while another account is unaffected.
        assert!(admit(Operation::Simulate, Some(&consumer_b), ip, &state)
            .await
            .is_err());
        assert!(admit(
            Operation::Simulate,
            Some(&other_account),
            "2.2.2.2".parse().unwrap(),
            &state
        )
        .await
        .is_ok());

        // Releasing the first admission frees the account slot.
        drop(held);
        assert!(admit(Operation::Simulate, Some(&consumer_b), ip, &state)
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn account_rate_budget_is_shared_and_legacy_path_is_unchanged() {
        let state = state_for(Value::Null).await;
        let account_limits = arete_auth::Limits {
            max_transaction_inspect_requests_per_minute: Some(2),
            ..arete_auth::Limits::default()
        };
        let consumer_a = v2_context(
            "consumer:a",
            "account:42",
            arete_auth::Limits::default(),
            account_limits.clone(),
        );
        let consumer_b = v2_context(
            "consumer:b",
            "account:42",
            arete_auth::Limits::default(),
            account_limits,
        );

        // Two consumers consume the same account inspect budget.
        assert!(admit(
            Operation::Simulate,
            Some(&consumer_a),
            "1.1.1.1".parse().unwrap(),
            &state
        )
        .await
        .is_ok());
        assert!(admit(
            Operation::Simulate,
            Some(&consumer_b),
            "1.1.1.2".parse().unwrap(),
            &state
        )
        .await
        .is_ok());
        assert!(admit(
            Operation::Simulate,
            Some(&consumer_a),
            "1.1.1.3".parse().unwrap(),
            &state
        )
        .await
        .is_err());

        // Legacy tokens keep the subject-keyed path and are counted.
        let legacy = legacy_context(
            "legacy-user",
            arete_auth::Limits {
                max_transaction_inspect_requests_per_minute: Some(2),
                ..arete_auth::Limits::default()
            },
        );
        assert!(legacy.is_legacy_policy());
        for ip in ["3.3.3.1", "3.3.3.2"] {
            assert!(admit(
                Operation::Simulate,
                Some(&legacy),
                ip.parse().unwrap(),
                &state
            )
            .await
            .is_ok());
        }
        assert!(admit(
            Operation::Simulate,
            Some(&legacy),
            "3.3.3.3".parse().unwrap(),
            &state
        )
        .await
        .is_err());
        assert!(state
            .rate_buckets
            .contains_key("subject:legacy-user:inspect"));
        assert_eq!(state.account_policies().legacy_token_count(), 3);
    }

    #[tokio::test]
    async fn stale_policy_versions_are_rejected_for_transactions() {
        let state = state_for(Value::Null).await;
        let account_limits = arete_auth::Limits::default();

        let v2 = AuthContext::from_claims(
            arete_auth::SessionClaims::builder("issuer", "user:1", "aud")
                .with_metering_key("account:42")
                .with_plan("pro")
                .with_actor_key("user:1")
                .with_account_key("account:42")
                .with_consumer_key("consumer:a")
                .with_policy_version(5)
                .with_account_limits(account_limits.clone())
                .build(),
        );
        assert!(admit(
            Operation::Simulate,
            Some(&v2),
            "1.1.1.1".parse().unwrap(),
            &state
        )
        .await
        .is_ok());

        let stale = v2_context(
            "consumer:a",
            "account:42",
            arete_auth::Limits::default(),
            account_limits,
        );
        assert_eq!(stale.policy_version, Some(1));
        let error = match admit(
            Operation::Simulate,
            Some(&stale),
            "1.1.1.2".parse().unwrap(),
            &state,
        )
        .await
        {
            Ok(_) => panic!("stale policy version must be rejected"),
            Err(error) => error,
        };
        assert_eq!(error.code, "stale_policy_version");
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
