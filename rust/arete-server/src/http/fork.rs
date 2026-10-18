//! Read-only JSON-RPC endpoint for local mainnet forks (`POST /rpc/v1/fork`).
//!
//! A local fork (for example Surfpool) lazily reads accounts, signatures,
//! blocks and cluster metadata from a remote RPC. This endpoint lets it read
//! through the server's own read RPC instead of a user-supplied one, under a
//! dedicated `fork:read` scope.
//!
//! The contract is deliberately narrow:
//!
//! - only the allowlisted read methods in [`ForkMethod`] are forwarded; every
//!   other method, including every send, simulate and airdrop, is answered
//!   locally with `-32601` and never reaches the upstream;
//! - each method's parameters are checked against fixed limits before
//!   forwarding (batch size, keys per call, program-account filters, signature
//!   page size, block detail level, commitment and encoding);
//! - each upstream response is bounded in bytes and, for list results, in
//!   entries;
//! - upstream failures become generic JSON-RPC errors. Upstream URLs, error
//!   messages and error data never reach the caller.
//!
//! Each forwarded call is observed as one chain read with the `fork_read`
//! operation, so hosted embedders meter it like any other chain read.

use std::sync::Arc;
use std::time::Duration;

use futures_util::future::join_all;
use futures_util::StreamExt;
use http_body_util::{BodyExt, Full};
use hyper::body::{Bytes, Incoming};
use hyper::header::CONTENT_LENGTH;
use hyper::{Request, Response, StatusCode};
use reqwest::Client;
use serde_json::{json, Map, Value};
use tracing::warn;

use crate::websocket::auth::AuthContext;
use crate::{SolanaGatewayUsageObservation, SolanaGatewayUsageObserver, SolanaGatewayUsageSurface};

/// The single route served by this module.
pub(crate) const FORK_RPC_PATH: &str = "/rpc/v1/fork";
/// Usage operation label for one forwarded fork read.
pub(crate) const FORK_USAGE_OPERATION: &str = "fork_read";
/// Most calls one JSON-RPC batch may carry.
pub(crate) const MAX_FORK_BATCH: usize = 20;
/// Largest accepted request body.
pub(crate) const MAX_FORK_REQUEST_BYTES: usize = 256 * 1024;
/// Largest upstream response for one call, and largest response body overall.
pub(crate) const MAX_FORK_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
/// Most keys one `getMultipleAccounts` call may name.
pub(crate) const MAX_FORK_MULTIPLE_ACCOUNTS: usize = 100;
/// Largest `getSignaturesForAddress` page.
pub(crate) const MAX_FORK_SIGNATURES: u64 = 1_000;
/// Most entries a list result (`getProgramAccounts`, `getTokenAccountsByOwner`) may return.
pub(crate) const MAX_FORK_RESULT_ACCOUNTS: usize = 10_000;
/// Solana's own ceiling on `getProgramAccounts` filters.
const MAX_FORK_PROGRAM_FILTERS: usize = 4;
const FORK_UPSTREAM_TIMEOUT: Duration = Duration::from_secs(30);

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_ALLOWED: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
/// A result exceeded the endpoint's byte or entry limit.
pub(crate) const FORK_RESULT_TOO_LARGE: i64 = -32080;
/// The read RPC could not be reached, refused the request or answered malformed.
pub(crate) const FORK_UPSTREAM_UNAVAILABLE: i64 = -32081;

const METHOD_NOT_ALLOWED_MESSAGE: &str = "method not allowed on the fork endpoint";
const UPSTREAM_UNAVAILABLE_MESSAGE: &str = "Fork read upstream is unavailable";
const UPSTREAM_ERROR_MESSAGE: &str = "Fork read upstream returned an error";
const RESULT_TOO_LARGE_MESSAGE: &str = "Fork read result exceeds the endpoint limit";

const ALLOWED_COMMITMENTS: &[&str] = &["confirmed", "finalized"];
const ALLOWED_ENCODINGS: &[&str] = &["base64", "base64+zstd", "jsonParsed", "json"];

/// The only methods the fork endpoint forwards.
// Variants mirror the Solana RPC method names, which all start with `get`.
#[allow(clippy::enum_variant_names)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ForkMethod {
    GetAccountInfo,
    GetMultipleAccounts,
    GetTokenAccountsByOwner,
    GetProgramAccounts,
    GetSignaturesForAddress,
    GetTransaction,
    GetBlock,
    GetTokenLargestAccounts,
    GetEpochInfo,
    GetEpochSchedule,
    GetGenesisHash,
    GetSlot,
    GetBalance,
    GetVersion,
    GetHealth,
    GetMinimumBalanceForRentExemption,
    GetLatestBlockhash,
    GetFirstAvailableBlock,
    GetBlockHeight,
}

impl ForkMethod {
    pub(crate) const ALL: [ForkMethod; 19] = [
        Self::GetAccountInfo,
        Self::GetMultipleAccounts,
        Self::GetTokenAccountsByOwner,
        Self::GetProgramAccounts,
        Self::GetSignaturesForAddress,
        Self::GetTransaction,
        Self::GetBlock,
        Self::GetTokenLargestAccounts,
        Self::GetEpochInfo,
        Self::GetEpochSchedule,
        Self::GetGenesisHash,
        Self::GetSlot,
        Self::GetBalance,
        Self::GetVersion,
        Self::GetHealth,
        Self::GetMinimumBalanceForRentExemption,
        Self::GetLatestBlockhash,
        Self::GetFirstAvailableBlock,
        Self::GetBlockHeight,
    ];

    /// Exact, case-sensitive method lookup.
    pub(crate) fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|method| method.name() == name)
    }

    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::GetAccountInfo => "getAccountInfo",
            Self::GetMultipleAccounts => "getMultipleAccounts",
            Self::GetTokenAccountsByOwner => "getTokenAccountsByOwner",
            Self::GetProgramAccounts => "getProgramAccounts",
            Self::GetSignaturesForAddress => "getSignaturesForAddress",
            Self::GetTransaction => "getTransaction",
            Self::GetBlock => "getBlock",
            Self::GetTokenLargestAccounts => "getTokenLargestAccounts",
            Self::GetEpochInfo => "getEpochInfo",
            Self::GetEpochSchedule => "getEpochSchedule",
            Self::GetGenesisHash => "getGenesisHash",
            Self::GetSlot => "getSlot",
            Self::GetBalance => "getBalance",
            Self::GetVersion => "getVersion",
            Self::GetHealth => "getHealth",
            Self::GetMinimumBalanceForRentExemption => "getMinimumBalanceForRentExemption",
            Self::GetLatestBlockhash => "getLatestBlockhash",
            Self::GetFirstAvailableBlock => "getFirstAvailableBlock",
            Self::GetBlockHeight => "getBlockHeight",
        }
    }

    /// Required positional parameters, and whether a trailing config object is accepted.
    const fn shape(self) -> (&'static [Positional], bool) {
        use Positional::*;
        match self {
            Self::GetAccountInfo | Self::GetBalance | Self::GetTokenLargestAccounts => {
                (&[Address], true)
            }
            Self::GetMultipleAccounts => (&[AddressList], true),
            Self::GetTokenAccountsByOwner => (&[Address, TokenAccountsFilter], true),
            Self::GetProgramAccounts | Self::GetSignaturesForAddress => (&[Address], true),
            Self::GetTransaction => (&[Signature], true),
            Self::GetBlock => (&[U64], true),
            Self::GetMinimumBalanceForRentExemption => (&[U64], true),
            Self::GetEpochInfo
            | Self::GetSlot
            | Self::GetLatestBlockhash
            | Self::GetBlockHeight => (&[], true),
            Self::GetEpochSchedule
            | Self::GetGenesisHash
            | Self::GetVersion
            | Self::GetHealth
            | Self::GetFirstAvailableBlock => (&[], false),
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Positional {
    Address,
    AddressList,
    TokenAccountsFilter,
    Signature,
    U64,
}

/// A JSON-RPC error the endpoint answers with. Messages are always fixed text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ForkError {
    pub(crate) code: i64,
    pub(crate) message: &'static str,
}

impl ForkError {
    const fn new(code: i64, message: &'static str) -> Self {
        Self { code, message }
    }

    const fn params(message: &'static str) -> Self {
        Self::new(INVALID_PARAMS, message)
    }

    const fn upstream_unavailable() -> Self {
        Self::new(FORK_UPSTREAM_UNAVAILABLE, UPSTREAM_UNAVAILABLE_MESSAGE)
    }

    const fn too_large() -> Self {
        Self::new(FORK_RESULT_TOO_LARGE, RESULT_TOO_LARGE_MESSAGE)
    }
}

fn error_object(id: Value, error: ForkError) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": error.code, "message": error.message },
    })
}

/// A parsed request body: one call, a batch, or an error answered as a whole.
#[derive(Debug)]
pub(crate) enum ForkPayload {
    Rejected(Value),
    Single(Value),
    Batch(Vec<Value>),
}

impl ForkPayload {
    /// Calls this payload would forward at most: the rate-limit weight of the request.
    pub(crate) fn weight(&self) -> usize {
        match self {
            Self::Rejected(_) | Self::Single(_) => 1,
            Self::Batch(calls) => calls.len().max(1),
        }
    }
}

/// Read the request body, refusing anything over [`MAX_FORK_REQUEST_BYTES`].
#[allow(clippy::result_large_err)]
pub(crate) async fn read_body(req: Request<Incoming>) -> Result<Bytes, Response<Full<Bytes>>> {
    let too_large = || {
        json_rpc_response(
            StatusCode::PAYLOAD_TOO_LARGE,
            &error_object(
                Value::Null,
                ForkError::new(INVALID_REQUEST, "Request body exceeds the endpoint limit"),
            ),
        )
    };
    if req
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok())
        .is_some_and(|length| length > MAX_FORK_REQUEST_BYTES)
    {
        return Err(too_large());
    }
    let mut body = req.into_body();
    let mut bytes = Vec::new();
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|_| {
            json_rpc_response(
                StatusCode::BAD_REQUEST,
                &error_object(
                    Value::Null,
                    ForkError::new(INVALID_REQUEST, "Unable to read request body"),
                ),
            )
        })?;
        if let Ok(data) = frame.into_data() {
            if bytes.len().saturating_add(data.len()) > MAX_FORK_REQUEST_BYTES {
                return Err(too_large());
            }
            bytes.extend_from_slice(&data);
        }
    }
    Ok(Bytes::from(bytes))
}

/// Split a body into calls. Batch and parse errors are answered as a whole.
pub(crate) fn parse_payload(body: &[u8]) -> ForkPayload {
    let value = match serde_json::from_slice::<Value>(body) {
        Ok(value) => value,
        Err(_) => {
            return ForkPayload::Rejected(error_object(
                Value::Null,
                ForkError::new(PARSE_ERROR, "Parse error"),
            ))
        }
    };
    match value {
        Value::Array(calls) if calls.is_empty() => ForkPayload::Rejected(error_object(
            Value::Null,
            ForkError::new(INVALID_REQUEST, "Empty batch"),
        )),
        Value::Array(calls) if calls.len() > MAX_FORK_BATCH => ForkPayload::Rejected(error_object(
            Value::Null,
            ForkError::new(INVALID_REQUEST, "Batch exceeds 20 requests"),
        )),
        Value::Array(calls) => ForkPayload::Batch(calls),
        call => ForkPayload::Single(call),
    }
}

/// Upstream and metering services for one request.
pub(crate) struct ForkServices<'a> {
    pub(crate) rpc_url: Option<&'a str>,
    pub(crate) client: &'a Client,
    pub(crate) auth_context: Option<&'a AuthContext>,
    pub(crate) usage_observer: Option<&'a Arc<dyn SolanaGatewayUsageObserver>>,
    pub(crate) target_id: Option<&'a str>,
    pub(crate) max_response_bytes: usize,
}

/// Answer a parsed payload. JSON-RPC errors are carried in a `200` body.
pub(crate) async fn execute(
    payload: ForkPayload,
    services: &ForkServices<'_>,
) -> Response<Full<Bytes>> {
    match payload {
        ForkPayload::Rejected(error) => json_rpc_response(StatusCode::OK, &error),
        ForkPayload::Single(call) => {
            let id = call.get("id").cloned().unwrap_or(Value::Null);
            let answer = handle_call(call, services).await;
            let body = answer.to_string();
            if body.len() > services.max_response_bytes {
                return json_rpc_response(
                    StatusCode::OK,
                    &error_object(valid_id(&id), ForkError::too_large()),
                );
            }
            raw_response(StatusCode::OK, body)
        }
        ForkPayload::Batch(calls) => {
            let ids = calls
                .iter()
                .map(|call| valid_id(call.get("id").unwrap_or(&Value::Null)))
                .collect::<Vec<_>>();
            let answers = join_all(calls.into_iter().map(|call| handle_call(call, services))).await;
            // Reserve every error answer (including echoed ids) before
            // accepting results. A later replacement can never exceed the cap.
            let errors = ids
                .into_iter()
                .map(|id| error_object(id, ForkError::too_large()))
                .collect::<Vec<_>>();
            let error_lengths = errors
                .iter()
                .map(|e| e.to_string().len())
                .collect::<Vec<_>>();
            let reserved = 2 + errors.len().saturating_sub(1) + error_lengths.iter().sum::<usize>();
            if reserved > services.max_response_bytes {
                return json_rpc_response(
                    StatusCode::OK,
                    &error_object(Value::Null, ForkError::too_large()),
                );
            }
            let mut budget = services.max_response_bytes - reserved;
            let mut bounded = Vec::with_capacity(answers.len());
            for ((answer, error), error_length) in
                answers.into_iter().zip(errors).zip(error_lengths)
            {
                let length = answer.to_string().len();
                if length <= budget + error_length {
                    budget = budget + error_length - length;
                    bounded.push(answer);
                } else {
                    bounded.push(error);
                }
            }
            json_rpc_response(StatusCode::OK, &Value::Array(bounded))
        }
    }
}

fn valid_id(id: &Value) -> Value {
    match id {
        Value::String(_) | Value::Number(_) => id.clone(),
        _ => Value::Null,
    }
}

async fn handle_call(call: Value, services: &ForkServices<'_>) -> Value {
    let Value::Object(mut call) = call else {
        return error_object(
            Value::Null,
            ForkError::new(INVALID_REQUEST, "Invalid request"),
        );
    };
    let id = match call.remove("id") {
        None | Some(Value::Null) => Value::Null,
        Some(id @ (Value::String(_) | Value::Number(_))) => id,
        Some(_) => {
            return error_object(
                Value::Null,
                ForkError::new(INVALID_REQUEST, "Invalid request id"),
            )
        }
    };
    if call.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return error_object(
            id,
            ForkError::new(INVALID_REQUEST, "jsonrpc must be \"2.0\""),
        );
    }
    let Some(method_name) = call.get("method").and_then(Value::as_str) else {
        return error_object(
            id,
            ForkError::new(INVALID_REQUEST, "method must be a string"),
        );
    };
    let Some(method) = ForkMethod::parse(method_name) else {
        return error_object(
            id,
            ForkError::new(METHOD_NOT_ALLOWED, METHOD_NOT_ALLOWED_MESSAGE),
        );
    };
    let mut params = match call.remove("params") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(params)) => params,
        Some(_) => return error_object(id, ForkError::params("params must be an array")),
    };
    if let Err(error) = validate_params(method, &mut params) {
        return error_object(id, error);
    }
    let Some(rpc_url) = services.rpc_url else {
        return error_object(id, ForkError::upstream_unavailable());
    };

    let outcome = forward(services, rpc_url, method, params.clone()).await;
    let (result, response_bytes) = match outcome {
        Ok((result, bytes)) => match check_result(method, &result) {
            Ok(()) => (Ok(result), bytes),
            Err(error) => (Err(error), bytes),
        },
        Err(error) => (Err(error), 0),
    };
    observe(services, method, &params, &result, response_bytes).await;
    match result {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err(error) => error_object(id, error),
    }
}

/// Check positional parameters and the config object against the method's limits.
///
/// May normalize the config: `getBlock` without `rewards` is sent with `rewards: false`.
pub(crate) fn validate_params(
    method: ForkMethod,
    params: &mut Vec<Value>,
) -> Result<(), ForkError> {
    let (positional, accepts_config) = method.shape();
    let max = positional.len() + usize::from(accepts_config);
    if params.len() < positional.len() || params.len() > max {
        return Err(ForkError::params("Wrong number of params for this method"));
    }
    for (kind, value) in positional.iter().zip(params.iter()) {
        validate_positional(*kind, value)?;
    }
    if method == ForkMethod::GetMultipleAccounts {
        let count = params[0].as_array().map(Vec::len).unwrap_or_default();
        if count > MAX_FORK_MULTIPLE_ACCOUNTS {
            return Err(ForkError::params(
                "getMultipleAccounts accepts at most 100 keys on the fork endpoint",
            ));
        }
    }

    // `null` stands for an absent config, as Solana RPC accepts.
    if params.len() == max && accepts_config && params[max - 1].is_null() {
        params.pop();
    }
    let has_config = accepts_config && params.len() == max;
    if has_config && !params[max - 1].is_object() {
        return Err(ForkError::params("The config param must be an object"));
    }
    let mut empty = Map::new();
    let config = if has_config {
        params[max - 1].as_object_mut().expect("checked object")
    } else {
        &mut empty
    };
    validate_common_config(config)?;

    match method {
        ForkMethod::GetProgramAccounts => validate_program_accounts(config)?,
        ForkMethod::GetSignaturesForAddress => {
            if let Some(limit) = config.get("limit") {
                if !limit
                    .as_u64()
                    .is_some_and(|limit| (1..=MAX_FORK_SIGNATURES).contains(&limit))
                {
                    return Err(ForkError::params(
                        "getSignaturesForAddress limit must be between 1 and 1000",
                    ));
                }
            }
        }
        ForkMethod::GetBlock => {
            match config.get("transactionDetails").and_then(Value::as_str) {
                Some("none" | "signatures") => {}
                _ => {
                    return Err(ForkError::params(
                        "getBlock requires transactionDetails \"none\" or \"signatures\"",
                    ))
                }
            }
            match config.get("rewards") {
                None => {
                    config.insert("rewards".into(), Value::Bool(false));
                }
                Some(Value::Bool(false)) => {}
                Some(_) => return Err(ForkError::params("getBlock requires rewards false")),
            }
        }
        _ => {}
    }

    Ok(())
}

fn validate_positional(kind: Positional, value: &Value) -> Result<(), ForkError> {
    let valid_address = |value: &Value| {
        value
            .as_str()
            .is_some_and(|address| arete_solana_contracts::validate_address(address).is_ok())
    };
    let ok = match kind {
        Positional::Address => valid_address(value),
        Positional::AddressList => value
            .as_array()
            .is_some_and(|addresses| addresses.iter().all(valid_address)),
        Positional::TokenAccountsFilter => value.as_object().is_some_and(|filter| {
            filter.len() == 1
                && ["mint", "programId"]
                    .iter()
                    .any(|key| filter.get(*key).is_some_and(valid_address))
        }),
        Positional::Signature => value.as_str().is_some_and(|signature| {
            signature.len() <= 88
                && bs58::decode(signature)
                    .into_vec()
                    .is_ok_and(|bytes| bytes.len() == 64)
        }),
        Positional::U64 => value.as_u64().is_some(),
    };
    if ok {
        Ok(())
    } else {
        Err(ForkError::params(match kind {
            Positional::Address => "Expected a base58 address",
            Positional::AddressList => "Expected an array of base58 addresses",
            Positional::TokenAccountsFilter => {
                "Expected a filter with exactly one of mint or programId"
            }
            Positional::Signature => "Expected a base58 transaction signature",
            Positional::U64 => "Expected an unsigned integer",
        }))
    }
}

fn validate_common_config(config: &Map<String, Value>) -> Result<(), ForkError> {
    if let Some(commitment) = config.get("commitment") {
        if !commitment
            .as_str()
            .is_some_and(|commitment| ALLOWED_COMMITMENTS.contains(&commitment))
        {
            return Err(ForkError::params(
                "The fork endpoint accepts only confirmed or finalized commitment",
            ));
        }
    }
    if let Some(encoding) = config.get("encoding") {
        if !encoding
            .as_str()
            .is_some_and(|encoding| ALLOWED_ENCODINGS.contains(&encoding))
        {
            return Err(ForkError::params(
                "The fork endpoint accepts only base64, base64+zstd, jsonParsed or json encoding",
            ));
        }
    }
    Ok(())
}

fn validate_program_accounts(config: &Map<String, Value>) -> Result<(), ForkError> {
    let filters = config
        .get("filters")
        .and_then(Value::as_array)
        .filter(|filters| !filters.is_empty())
        .ok_or(ForkError::params(
            "getProgramAccounts requires at least one filter on the fork endpoint",
        ))?;
    if filters.len() > MAX_FORK_PROGRAM_FILTERS {
        return Err(ForkError::params(
            "getProgramAccounts accepts at most 4 filters",
        ));
    }
    let narrows = filters.iter().any(|filter| {
        filter
            .as_object()
            .is_some_and(|filter| filter.contains_key("dataSize") || filter.contains_key("memcmp"))
    });
    if !narrows {
        return Err(ForkError::params(
            "getProgramAccounts requires a dataSize or memcmp filter on the fork endpoint",
        ));
    }
    Ok(())
}

/// Reject list results over [`MAX_FORK_RESULT_ACCOUNTS`] entries.
fn check_result(method: ForkMethod, result: &Value) -> Result<(), ForkError> {
    if matches!(
        method,
        ForkMethod::GetProgramAccounts | ForkMethod::GetTokenAccountsByOwner
    ) && list_len(result).is_some_and(|count| count > MAX_FORK_RESULT_ACCOUNTS)
    {
        return Err(ForkError::too_large());
    }
    Ok(())
}

/// Entries in a list result, bare or wrapped in `{context, value}`.
fn list_len(result: &Value) -> Option<usize> {
    result
        .as_array()
        .or_else(|| result.get("value").and_then(Value::as_array))
        .map(Vec::len)
}

/// Forward one validated call. Returns the result and the upstream body size.
async fn forward(
    services: &ForkServices<'_>,
    rpc_url: &str,
    method: ForkMethod,
    params: Vec<Value>,
) -> Result<(Value, usize), ForkError> {
    let response = services
        .client
        .post(rpc_url)
        .timeout(FORK_UPSTREAM_TIMEOUT)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": "arete-fork",
            "method": method.name(),
            "params": params,
        }))
        .send()
        .await
        .map_err(|error| {
            // reqwest errors carry the URL; log only their kind.
            warn!(
                method = method.name(),
                timeout = error.is_timeout(),
                connect = error.is_connect(),
                "Fork read upstream request failed"
            );
            ForkError::upstream_unavailable()
        })?;
    if !response.status().is_success() {
        warn!(
            method = method.name(),
            status = response.status().as_u16(),
            "Fork read upstream returned an HTTP error"
        );
        return Err(ForkError::upstream_unavailable());
    }
    if response
        .content_length()
        .is_some_and(|length| length > services.max_response_bytes as u64)
    {
        return Err(ForkError::too_large());
    }
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| ForkError::upstream_unavailable())?;
        if bytes.len().saturating_add(chunk.len()) > services.max_response_bytes {
            return Err(ForkError::too_large());
        }
        bytes.extend_from_slice(&chunk);
    }
    let size = bytes.len();
    let mut envelope: Value =
        serde_json::from_slice(&bytes).map_err(|_| ForkError::upstream_unavailable())?;
    if let Some(error) = envelope.get("error") {
        return Err(sanitize_upstream_error(error));
    }
    match envelope.get_mut("result") {
        Some(result) => Ok((result.take(), size)),
        None => Err(ForkError::upstream_unavailable()),
    }
}

/// Keep only an upstream error's numeric class; its message and data never leave.
///
/// Solana's server-error codes (`-32000..=-32099`, e.g. a skipped slot or a block that is not
/// available) are kept so the fork can tell them apart. `-32602` stays an invalid-params error.
/// Anything else becomes a generic upstream error.
pub(crate) fn sanitize_upstream_error(error: &Value) -> ForkError {
    match error.get("code").and_then(Value::as_i64) {
        Some(INVALID_PARAMS) => ForkError::params("Invalid params"),
        Some(code) if (-32099..=-32000).contains(&code) => {
            ForkError::new(code, UPSTREAM_ERROR_MESSAGE)
        }
        _ => ForkError::new(-32000, UPSTREAM_ERROR_MESSAGE),
    }
}

async fn observe(
    services: &ForkServices<'_>,
    method: ForkMethod,
    params: &[Value],
    result: &Result<Value, ForkError>,
    response_bytes: usize,
) {
    let (Some(observer), Some(target_id)) = (services.usage_observer, services.target_id) else {
        return;
    };
    let address_count = match result {
        Err(_) => 0,
        Ok(result) => match method {
            ForkMethod::GetAccountInfo | ForkMethod::GetBalance => 1,
            ForkMethod::GetMultipleAccounts => params
                .first()
                .and_then(Value::as_array)
                .map(Vec::len)
                .unwrap_or_default(),
            ForkMethod::GetProgramAccounts
            | ForkMethod::GetTokenAccountsByOwner
            | ForkMethod::GetTokenLargestAccounts => list_len(result).unwrap_or_default(),
            _ => 0,
        },
    };
    observer
        .observe(SolanaGatewayUsageObservation {
            target_id: target_id.to_string(),
            auth_context: services.auth_context.cloned(),
            surface: SolanaGatewayUsageSurface::ChainRead,
            operation: FORK_USAGE_OPERATION,
            result: if result.is_ok() { "ok" } else { "error" },
            address_count: address_count as u64,
            response_bytes: response_bytes as u64,
        })
        .await;
}

fn json_rpc_response(status: StatusCode, value: &Value) -> Response<Full<Bytes>> {
    raw_response(status, value.to_string())
}

fn raw_response(status: StatusCode, body: String) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .body(Full::new(Bytes::from(body)))
        .expect("valid fork response")
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::convert::Infallible;
    use std::sync::Mutex;

    #[derive(Default)]
    struct RecordingUsageObserver {
        observations: Mutex<Vec<SolanaGatewayUsageObservation>>,
    }

    #[async_trait::async_trait]
    impl SolanaGatewayUsageObserver for RecordingUsageObserver {
        async fn observe(&self, observation: SolanaGatewayUsageObservation) {
            self.observations.lock().unwrap().push(observation);
        }
    }

    type Responder = Arc<dyn Fn(&Value) -> (u16, String) + Send + Sync>;

    /// A mock read RPC that answers every connection through `responder` and records requests.
    pub(crate) struct MockRpc {
        pub(crate) url: String,
        pub(crate) requests: Arc<Mutex<Vec<Value>>>,
    }

    pub(crate) async fn mock_rpc(responder: Responder) -> MockRpc {
        use hyper::server::conn::http1;
        use hyper::service::service_fn;
        use hyper_util::rt::TokioIo;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = requests.clone();
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                let responder = responder.clone();
                let recorded = recorded.clone();
                tokio::spawn(async move {
                    let _ = http1::Builder::new()
                        .serve_connection(
                            TokioIo::new(stream),
                            service_fn(move |request: Request<Incoming>| {
                                let responder = responder.clone();
                                let recorded = recorded.clone();
                                async move {
                                    let body =
                                        request.into_body().collect().await.unwrap().to_bytes();
                                    let body: Value = serde_json::from_slice(&body).unwrap();
                                    recorded.lock().unwrap().push(body.clone());
                                    let (status, text) = responder(&body);
                                    Ok::<_, Infallible>(
                                        Response::builder()
                                            .status(status)
                                            .body(Full::new(Bytes::from(text)))
                                            .unwrap(),
                                    )
                                }
                            }),
                        )
                        .await;
                });
            }
        });
        MockRpc { url, requests }
    }

    /// Answer every call with `result`.
    pub(crate) fn result_responder(result: Value) -> Responder {
        Arc::new(move |_| {
            (
                200,
                json!({ "jsonrpc": "2.0", "id": "arete-fork", "result": result }).to_string(),
            )
        })
    }

    fn address(byte: u8) -> String {
        bs58::encode([byte; 32]).into_string()
    }

    fn signature(byte: u8) -> String {
        bs58::encode([byte; 64]).into_string()
    }

    async fn run(
        body: Value,
        rpc: &MockRpc,
        observer: Option<&Arc<dyn SolanaGatewayUsageObserver>>,
        max_response_bytes: usize,
    ) -> Value {
        let client = Client::new();
        let services = ForkServices {
            rpc_url: Some(&rpc.url),
            client: &client,
            auth_context: None,
            usage_observer: observer,
            target_id: Some("gateway-test"),
            max_response_bytes,
        };
        let response = execute(parse_payload(body.to_string().as_bytes()), &services).await;
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap()
    }

    fn call(id: u64, method: &str, params: Value) -> Value {
        json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
    }

    #[test]
    fn the_allowlist_is_exact() {
        for method in ForkMethod::ALL {
            assert_eq!(ForkMethod::parse(method.name()), Some(method));
        }
        for denied in [
            "sendTransaction",
            "simulateTransaction",
            "requestAirdrop",
            "getaccountinfo",
            "getAccountInfo ",
            "getBlocks",
            "getLargestAccounts",
            "getSupply",
            "getConfirmedBlock",
            "accountSubscribe",
            "",
        ] {
            assert_eq!(ForkMethod::parse(denied), None, "{denied}");
        }
    }

    #[tokio::test]
    async fn writes_and_unknown_methods_never_reach_the_upstream() {
        let rpc = mock_rpc(result_responder(json!(1))).await;
        for method in [
            "sendTransaction",
            "simulateTransaction",
            "requestAirdrop",
            "getLargestAccounts",
            "surfnet_setAccount",
        ] {
            let answer = run(
                call(7, method, json!([])),
                &rpc,
                None,
                MAX_FORK_RESPONSE_BYTES,
            )
            .await;
            assert_eq!(answer["id"], 7);
            assert_eq!(answer["error"]["code"], -32601, "{method}");
            assert_eq!(
                answer["error"]["message"],
                "method not allowed on the fork endpoint"
            );
        }
        assert!(rpc.requests.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn allowed_reads_are_forwarded_verbatim() {
        let rpc = mock_rpc(result_responder(
            json!({ "context": { "slot": 9 }, "value": null }),
        ))
        .await;
        let params = json!([address(1), { "encoding": "base64+zstd", "commitment": "finalized" }]);
        let answer = run(
            call(1, "getAccountInfo", params.clone()),
            &rpc,
            None,
            MAX_FORK_RESPONSE_BYTES,
        )
        .await;

        assert_eq!(answer["result"]["context"]["slot"], 9);
        assert_eq!(answer["id"], 1);
        let requests = rpc.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0]["method"], "getAccountInfo");
        assert_eq!(requests[0]["params"], params);
    }

    #[test]
    fn limits_reject_oversized_or_unbounded_reads() {
        let keys = |count: u8| json!((0..count).map(address).collect::<Vec<_>>());
        let reject = |method: ForkMethod, params: Value| {
            let mut params = params.as_array().unwrap().clone();
            let error = validate_params(method, &mut params).unwrap_err();
            assert_eq!(error.code, -32602, "{} {params:?}", method.name());
        };
        let accept = |method: ForkMethod, params: Value| {
            let mut params = params.as_array().unwrap().clone();
            validate_params(method, &mut params).unwrap();
            params
        };

        accept(ForkMethod::GetMultipleAccounts, json!([keys(100)]));
        reject(ForkMethod::GetMultipleAccounts, json!([keys(101)]));
        reject(ForkMethod::GetMultipleAccounts, json!([["not-an-address"]]));

        let program = address(9);
        reject(ForkMethod::GetProgramAccounts, json!([program]));
        reject(
            ForkMethod::GetProgramAccounts,
            json!([program, { "filters": [] }]),
        );
        reject(
            ForkMethod::GetProgramAccounts,
            json!([program, { "filters": [{ "tokenAccountState": {} }] }]),
        );
        reject(
            ForkMethod::GetProgramAccounts,
            json!([program, { "filters": [{ "dataSize": 1 }, { "dataSize": 2 }, { "dataSize": 3 }, { "dataSize": 4 }, { "dataSize": 5 }] }]),
        );
        accept(
            ForkMethod::GetProgramAccounts,
            json!([program, { "filters": [{ "dataSize": 165 }], "encoding": "base64" }]),
        );
        accept(
            ForkMethod::GetProgramAccounts,
            json!([program, { "filters": [{ "memcmp": { "offset": 0, "bytes": "1" } }], "dataSlice": { "offset": 0, "length": 0 } }]),
        );

        accept(
            ForkMethod::GetSignaturesForAddress,
            json!([address(2), { "limit": 1000 }]),
        );
        reject(
            ForkMethod::GetSignaturesForAddress,
            json!([address(2), { "limit": 1001 }]),
        );
        reject(
            ForkMethod::GetSignaturesForAddress,
            json!([address(2), { "limit": 0 }]),
        );

        reject(ForkMethod::GetBlock, json!([5]));
        reject(
            ForkMethod::GetBlock,
            json!([5, { "transactionDetails": "full" }]),
        );
        reject(
            ForkMethod::GetBlock,
            json!([5, { "transactionDetails": "accounts" }]),
        );
        reject(
            ForkMethod::GetBlock,
            json!([5, { "transactionDetails": "none", "rewards": true }]),
        );
        let params = accept(
            ForkMethod::GetBlock,
            json!([5, { "transactionDetails": "signatures" }]),
        );
        assert_eq!(params[1]["rewards"], false, "rewards default to false");

        for commitment in ["processed", "recent", "single", "max", "bogus"] {
            reject(ForkMethod::GetSlot, json!([{ "commitment": commitment }]));
        }
        accept(ForkMethod::GetSlot, json!([{ "commitment": "confirmed" }]));
        accept(ForkMethod::GetSlot, json!([]));
        for encoding in ["base58", "binary"] {
            reject(
                ForkMethod::GetAccountInfo,
                json!([address(1), { "encoding": encoding }]),
            );
        }
        accept(
            ForkMethod::GetTransaction,
            json!([signature(3), { "encoding": "json", "maxSupportedTransactionVersion": 0 }]),
        );
        reject(ForkMethod::GetTransaction, json!(["short"]));

        reject(ForkMethod::GetTokenAccountsByOwner, json!([address(1), {}]));
        reject(
            ForkMethod::GetTokenAccountsByOwner,
            json!([address(1), { "mint": address(2), "programId": address(3) }]),
        );
        accept(
            ForkMethod::GetTokenAccountsByOwner,
            json!([address(1), { "programId": address(3) }, { "encoding": "jsonParsed" }]),
        );

        reject(ForkMethod::GetGenesisHash, json!([{}]));
        reject(ForkMethod::GetAccountInfo, json!([]));
        reject(ForkMethod::GetAccountInfo, json!([address(1), "base64"]));
        accept(ForkMethod::GetAccountInfo, json!([address(1), null]));
        accept(ForkMethod::GetMinimumBalanceForRentExemption, json!([165]));
    }

    #[tokio::test]
    async fn list_results_over_the_entry_cap_are_refused() {
        let accounts = vec![json!({ "pubkey": "x" }); MAX_FORK_RESULT_ACCOUNTS + 1];
        let rpc = mock_rpc(result_responder(json!(accounts))).await;
        let answer = run(
            call(
                1,
                "getProgramAccounts",
                json!([address(9), { "filters": [{ "dataSize": 8 }] }]),
            ),
            &rpc,
            None,
            MAX_FORK_RESPONSE_BYTES,
        )
        .await;
        assert_eq!(answer["error"]["code"], FORK_RESULT_TOO_LARGE);
        assert!(answer.get("result").is_none());
    }

    #[tokio::test]
    async fn oversized_upstream_bodies_are_refused() {
        let rpc = mock_rpc(result_responder(json!("x".repeat(4096)))).await;
        let answer = run(call(3, "getGenesisHash", json!([])), &rpc, None, 1024).await;
        assert_eq!(answer["id"], 3);
        assert_eq!(answer["error"]["code"], FORK_RESULT_TOO_LARGE);
    }

    #[tokio::test]
    async fn batches_answer_in_order_and_forward_only_allowed_calls() {
        let rpc = mock_rpc(result_responder(json!(42))).await;
        let answer = run(
            json!([
                call(1, "getSlot", json!([{ "commitment": "confirmed" }])),
                call(2, "sendTransaction", json!(["AAAA"])),
                call(3, "getBlockHeight", json!([])),
                json!({ "jsonrpc": "1.0", "id": 4, "method": "getSlot" }),
                json!("not an object"),
            ]),
            &rpc,
            None,
            MAX_FORK_RESPONSE_BYTES,
        )
        .await;
        let answers = answer.as_array().unwrap();
        assert_eq!(answers.len(), 5);
        assert_eq!(
            answers[0],
            json!({ "jsonrpc": "2.0", "id": 1, "result": 42 })
        );
        assert_eq!(answers[1]["error"]["code"], -32601);
        assert_eq!(answers[1]["id"], 2);
        assert_eq!(answers[2]["result"], 42);
        assert_eq!(answers[3]["error"]["code"], -32600);
        assert_eq!(answers[3]["id"], 4);
        assert_eq!(answers[4]["error"]["code"], -32600);
        let methods = rpc
            .requests
            .lock()
            .unwrap()
            .iter()
            .map(|request| request["method"].as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        assert_eq!(methods.len(), 2);
        assert!(methods.iter().all(|method| method != "sendTransaction"));
    }

    #[tokio::test]
    async fn batch_and_body_shape_errors_are_answered_as_a_whole() {
        let rpc = mock_rpc(result_responder(json!(1))).await;
        let too_many = Value::Array((0..21).map(|id| call(id, "getSlot", json!([]))).collect());
        let answer = run(too_many, &rpc, None, MAX_FORK_RESPONSE_BYTES).await;
        assert_eq!(answer["error"]["code"], -32600);
        assert_eq!(answer["id"], Value::Null);

        let twenty = Value::Array((0..20).map(|id| call(id, "getSlot", json!([]))).collect());
        let answer = run(twenty, &rpc, None, MAX_FORK_RESPONSE_BYTES).await;
        assert_eq!(answer.as_array().unwrap().len(), 20);

        let answer = run(json!([]), &rpc, None, MAX_FORK_RESPONSE_BYTES).await;
        assert_eq!(answer["error"]["code"], -32600);

        assert!(matches!(
            parse_payload(b"{not json"),
            ForkPayload::Rejected(value) if value["error"]["code"] == -32700
        ));
        assert_eq!(rpc.requests.lock().unwrap().len(), 20);
    }

    #[tokio::test]
    async fn the_batch_shares_one_response_byte_budget() {
        let rpc = mock_rpc(result_responder(json!("y".repeat(300)))).await;
        let batch = Value::Array(
            (0..4)
                .map(|id| call(id, "getGenesisHash", json!([])))
                .collect(),
        );
        let answer = run(batch, &rpc, None, 1024).await;
        assert!(answer.to_string().len() <= 1024);
        let answers = answer.as_array().unwrap();
        assert!(answers[0].get("result").is_some());
        assert!(answers[1].get("result").is_some());
        assert_eq!(answers[3]["error"]["code"], FORK_RESULT_TOO_LARGE);
        assert_eq!(answers[3]["id"], 3);
    }

    #[tokio::test]
    async fn batch_error_ids_cannot_exhaust_the_shared_budget() {
        let rpc = mock_rpc(result_responder(json!("ok"))).await;
        let batch = Value::Array((0..2).map(|_| json!({"jsonrpc":"2.0", "id":"x".repeat(800), "method":"getGenesisHash", "params":[]})).collect());
        let answer = run(batch, &rpc, None, 1024).await;
        assert!(answer.to_string().len() <= 1024);
        assert_eq!(answer["error"]["code"], FORK_RESULT_TOO_LARGE);
        assert_eq!(answer["id"], Value::Null);
    }

    #[tokio::test]
    async fn upstream_errors_never_leak_urls_messages_or_data() {
        let leaky: Responder = Arc::new(|request| {
            let code = match request["method"].as_str() {
                Some("getBlock") => -32009,
                Some("getTransaction") => -32602,
                _ => 403,
            };
            (
                200,
                json!({
                    "jsonrpc": "2.0",
                    "id": "arete-fork",
                    "error": {
                        "code": code,
                        "message": "denied by https://rpc.provider.example/?api-key=secret-key",
                        "data": { "endpoint": "https://rpc.provider.example/secret-key" }
                    }
                })
                .to_string(),
            )
        });
        let rpc = mock_rpc(leaky).await;
        let answer = run(
            json!([
                call(1, "getBlock", json!([5, { "transactionDetails": "none" }])),
                call(2, "getTransaction", json!([signature(1)])),
                call(3, "getSlot", json!([])),
            ]),
            &rpc,
            None,
            MAX_FORK_RESPONSE_BYTES,
        )
        .await;
        let text = answer.to_string();
        assert!(!text.contains("provider.example"), "{text}");
        assert!(!text.contains("secret-key"), "{text}");
        assert!(!text.contains("data"), "{text}");
        assert_eq!(answer[0]["error"]["code"], -32009);
        assert_eq!(answer[1]["error"]["code"], -32602);
        assert_eq!(answer[2]["error"]["code"], -32000);

        let http_error: Responder = Arc::new(|_| {
            (
                502,
                "bad gateway at https://rpc.provider.example/secret-key".into(),
            )
        });
        let rpc = mock_rpc(http_error).await;
        let answer = run(
            call(4, "getSlot", json!([])),
            &rpc,
            None,
            MAX_FORK_RESPONSE_BYTES,
        )
        .await;
        assert_eq!(answer["error"]["code"], FORK_UPSTREAM_UNAVAILABLE);
        assert!(!answer.to_string().contains("provider.example"));

        let malformed: Responder =
            Arc::new(|_| (200, "<html>https://rpc.provider.example</html>".into()));
        let rpc = mock_rpc(malformed).await;
        let answer = run(
            call(5, "getSlot", json!([])),
            &rpc,
            None,
            MAX_FORK_RESPONSE_BYTES,
        )
        .await;
        assert_eq!(answer["error"]["code"], FORK_UPSTREAM_UNAVAILABLE);
        assert!(!answer.to_string().contains("provider.example"));
    }

    #[tokio::test]
    async fn an_unreachable_upstream_does_not_reveal_its_address() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/secret-path", listener.local_addr().unwrap());
        drop(listener);
        let client = Client::new();
        let services = ForkServices {
            rpc_url: Some(&url),
            client: &client,
            auth_context: None,
            usage_observer: None,
            target_id: None,
            max_response_bytes: MAX_FORK_RESPONSE_BYTES,
        };
        let response = execute(
            parse_payload(call(1, "getSlot", json!([])).to_string().as_bytes()),
            &services,
        )
        .await;
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let text = std::str::from_utf8(&body).unwrap();
        assert!(text.contains(&FORK_UPSTREAM_UNAVAILABLE.to_string()));
        assert!(!text.contains("127.0.0.1"));
        assert!(!text.contains("secret-path"));
    }

    #[tokio::test]
    async fn each_forwarded_call_is_metered_as_one_fork_read() {
        let responder: Responder = Arc::new(|request| {
            let result = match request["method"].as_str() {
                Some("getMultipleAccounts") => {
                    json!({ "context": { "slot": 1 }, "value": [null, null, null] })
                }
                Some("getProgramAccounts") => json!([{ "pubkey": "a" }, { "pubkey": "b" }]),
                _ => json!(5),
            };
            (
                200,
                json!({ "jsonrpc": "2.0", "id": "arete-fork", "result": result }).to_string(),
            )
        });
        let rpc = mock_rpc(responder).await;
        let recorder = Arc::new(RecordingUsageObserver::default());
        let observer: Arc<dyn SolanaGatewayUsageObserver> = recorder.clone();
        run(
            json!([
                call(
                    1,
                    "getMultipleAccounts",
                    json!([[address(1), address(2), address(3)]])
                ),
                call(
                    2,
                    "getProgramAccounts",
                    json!([address(9), { "filters": [{ "dataSize": 8 }] }])
                ),
                call(3, "getSlot", json!([])),
                call(4, "requestAirdrop", json!([address(1), 1])),
                call(5, "getSlot", json!([{ "commitment": "processed" }])),
            ]),
            &rpc,
            Some(&observer),
            MAX_FORK_RESPONSE_BYTES,
        )
        .await;

        let observations = recorder.observations.lock().unwrap();
        assert_eq!(observations.len(), 3, "rejected calls are not metered");
        for observation in observations.iter() {
            assert_eq!(observation.surface, SolanaGatewayUsageSurface::ChainRead);
            assert_eq!(observation.operation, "fork_read");
            assert_eq!(observation.result, "ok");
            assert_eq!(observation.target_id, "gateway-test");
            assert!(observation.response_bytes > 0);
        }
        let mut counts = observations
            .iter()
            .map(|observation| observation.address_count)
            .collect::<Vec<_>>();
        counts.sort_unstable();
        assert_eq!(counts, vec![0, 2, 3]);
    }

    #[tokio::test]
    async fn a_missing_read_rpc_is_a_generic_unavailable_error() {
        let client = Client::new();
        let services = ForkServices {
            rpc_url: None,
            client: &client,
            auth_context: None,
            usage_observer: None,
            target_id: None,
            max_response_bytes: MAX_FORK_RESPONSE_BYTES,
        };
        let response = execute(
            parse_payload(call(1, "getSlot", json!([])).to_string().as_bytes()),
            &services,
        )
        .await;
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["error"]["code"], FORK_UPSTREAM_UNAVAILABLE);
    }
}
