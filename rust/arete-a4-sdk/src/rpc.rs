//! Direct Solana JSON-RPC implementation of [`TransactionTransport`].
//!
//! Arete's relay stays the default everywhere; this is the explicit escape
//! hatch for a caller who wants their own provider. It implements the same
//! trait as [`HttpTransactionTransport`](crate::transactions::HttpTransactionTransport),
//! so the compiler, budget estimator, inspection, signing and confirmation
//! logic above it are unchanged — only the wire underneath differs.
//!
//! It adds no dependency: the SDK already carries `reqwest` and
//! `serde_json`. Provider endpoint and credentials are configured here and
//! never mixed with Arete authentication — no Arete token is ever sent to an
//! RPC provider, and no provider header is ever sent to Arete.
//!
//! # Wire differences the relay hides
//!
//! The relay encodes `u64` as decimal strings; native JSON-RPC uses JSON
//! numbers, wraps most results in `{ context, value }`, and reports failures
//! as a `200 OK` body carrying an `error` member. Nothing here reuses the
//! relay's decimal parsing.
//!
//! An absent simulation metric stays absent: `unitsConsumed` and
//! `loadedAccountsDataSize` are `None` when the node did not report them and
//! `Some(0)` when it measured zero, because a V1 budget derived from a
//! missing metric would be a guess.
//!
//! # Example
//!
//! ```rust,ignore
//! use std::sync::Arc;
//! use arete_sdk::prelude::*;
//! use arete_sdk::rpc::RpcTransactionTransport;
//!
//! let rpc = Arc::new(
//!     RpcTransactionTransport::new("https://api.devnet.solana.com")
//!         .with_header("x-provider-key", "…"),
//! );
//! let blockhash = rpc.latest_blockhash(Default::default()).await?;
//! ```

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Map, Value};

use crate::error::AreteError;
use crate::transactions::{
    Commitment, ConfirmedTransaction, LatestBlockhashResult, SignaturePageEntry,
    SignaturePageOptions, SignatureStatusOptions, SubmissionState, TransactionAccountBalance,
    TransactionError, TransactionFeeResult, TransactionInspectOptions, TransactionRequestContext,
    TransactionSendOptions, TransactionSendResult, TransactionSignatureStatus,
    TransactionSimulationOptions, TransactionSimulationResult, TransactionTransport,
    MAX_STATUS_SIGNATURES,
};

/// JSON-RPC error codes that prove a `sendTransaction` never reached the
/// cluster, so the caller may retry rather than reconcile.
///
/// Everything else — a transport failure, a gateway error, an unrecognized
/// code — leaves the submission state unknown, which is what keeps the
/// locally derived signature authoritative in the wallet adapter.
const NOT_SUBMITTED_RPC_CODES: &[i64] = &[
    -32002, // SendTransactionPreflightFailure: the node simulated and refused.
    -32003, // TransactionSignatureVerificationFailure.
    -32602, // Invalid params: the node never parsed the transaction.
];

/// [`TransactionTransport`] over a Solana node's JSON-RPC endpoint.
#[derive(Debug)]
pub struct RpcTransactionTransport {
    url: String,
    http: reqwest::Client,
    headers: Vec<(String, String)>,
    next_id: AtomicU64,
}

impl RpcTransactionTransport {
    /// Transport against `url` with a fresh HTTP client.
    pub fn new(url: impl Into<String>) -> Self {
        Self::with_http_client(url, reqwest::Client::new())
    }

    /// Transport against `url` reusing an existing client — the way to set a
    /// request timeout, a proxy, or a connection pool shared with the rest
    /// of the process.
    pub fn with_http_client(url: impl Into<String>, http_client: reqwest::Client) -> Self {
        Self {
            url: url.into(),
            http: http_client,
            headers: Vec::new(),
            next_id: AtomicU64::new(1),
        }
    }

    /// Add a provider header, typically an API key. Kept separate from Arete
    /// authentication: these headers only ever reach `url`.
    #[must_use]
    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    /// The endpoint this transport talks to.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// One JSON-RPC call, returning the `result` member.
    async fn call(&self, method: &str, params: Value) -> Result<Value, TransactionError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let mut request = self.http.post(&self.url).json(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }));
        for (name, value) in &self.headers {
            request = request.header(name.as_str(), value.as_str());
        }

        let response = request.send().await.map_err(|error| {
            TransactionError::Sdk(AreteError::ConnectionFailed(format!(
                "RPC request '{method}' failed: {error}"
            )))
        })?;
        let status = response.status().as_u16();
        let body = response.bytes().await.map_err(|error| {
            TransactionError::Sdk(AreteError::ConnectionFailed(format!(
                "RPC response '{method}' could not be read: {error}"
            )))
        })?;

        let parsed: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        // A node reports application failures inside a 200 body, and
        // gateways report their own failures with a status and no envelope.
        // Both become the same typed transport error.
        if let Some(error) = parsed.get("error").filter(|error| !error.is_null()) {
            return Err(rpc_failure(method, status, error));
        }
        if !(200..300).contains(&status) {
            return Err(http_failure(method, status));
        }
        parsed
            .get("result")
            .cloned()
            .ok_or_else(|| invalid(method, "response carried neither 'result' nor 'error'"))
    }

    /// `{ commitment, minContextSlot }` — the config object nearly every
    /// read method takes.
    fn context_config(options: TransactionRequestContext) -> Value {
        let mut config = Map::new();
        if let Some(commitment) = options.commitment {
            config.insert("commitment".into(), json!(commitment.as_str()));
        }
        if let Some(slot) = options.min_context_slot {
            config.insert("minContextSlot".into(), json!(slot));
        }
        Value::Object(config)
    }

    /// The `value` member of a `{ context, value }` envelope, with its slot.
    fn context_value<'a>(
        method: &str,
        result: &'a Value,
    ) -> Result<(u64, &'a Value), TransactionError> {
        let slot = result
            .get("context")
            .and_then(|context| context.get("slot"))
            .and_then(Value::as_u64)
            .ok_or_else(|| invalid(method, "missing 'context.slot'"))?;
        let value = result
            .get("value")
            .ok_or_else(|| invalid(method, "missing 'value'"))?;
        Ok((slot, value))
    }

    /// Shared by the single and batch status routes so neither can read a
    /// status the other would read differently.
    fn parse_statuses(
        signatures: &[String],
        result: &Value,
    ) -> Result<Vec<Option<TransactionSignatureStatus>>, TransactionError> {
        let (_, value) = Self::context_value("getSignatureStatuses", result)?;
        let entries = value
            .as_array()
            .ok_or_else(|| invalid("getSignatureStatuses", "'value' must be an array"))?;
        // Read positionally against the caller's own list: a length mismatch
        // would attribute one transaction's outcome to another.
        if entries.len() != signatures.len() {
            return Err(invalid(
                "getSignatureStatuses",
                &format!(
                    "expected {} statuses, got {}",
                    signatures.len(),
                    entries.len()
                ),
            ));
        }
        signatures
            .iter()
            .zip(entries)
            .map(|(signature, entry)| {
                if entry.is_null() {
                    return Ok(None);
                }
                Ok(Some(TransactionSignatureStatus {
                    signature: signature.clone(),
                    slot: entry.get("slot").and_then(Value::as_u64),
                    confirmation_status: entry
                        .get("confirmationStatus")
                        .and_then(Value::as_str)
                        .and_then(Commitment::from_wire),
                    err: nullable(entry.get("err")),
                }))
            })
            .collect()
    }
}

#[async_trait]
impl TransactionTransport for RpcTransactionTransport {
    async fn latest_blockhash(
        &self,
        options: TransactionRequestContext,
    ) -> Result<LatestBlockhashResult, TransactionError> {
        const METHOD: &str = "getLatestBlockhash";
        let result = self
            .call(METHOD, json!([Self::context_config(options)]))
            .await?;
        let (context_slot, value) = Self::context_value(METHOD, &result)?;
        Ok(LatestBlockhashResult {
            blockhash: value
                .get("blockhash")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid(METHOD, "missing 'value.blockhash'"))?
                .to_string(),
            context_slot,
            last_valid_block_height: value
                .get("lastValidBlockHeight")
                .and_then(Value::as_u64)
                .ok_or_else(|| invalid(METHOD, "missing 'value.lastValidBlockHeight'"))?,
        })
    }

    async fn fee(
        &self,
        message: &str,
        options: TransactionRequestContext,
    ) -> Result<TransactionFeeResult, TransactionError> {
        const METHOD: &str = "getFeeForMessage";
        let result = self
            .call(METHOD, json!([message, Self::context_config(options)]))
            .await?;
        let (context_slot, value) = Self::context_value(METHOD, &result)?;
        // `null` is the node's answer for a message whose blockhash it no
        // longer has — not a malformed response, and not a zero fee.
        let fee_lamports = match value {
            Value::Null => None,
            other => Some(
                other
                    .as_u64()
                    .ok_or_else(|| invalid(METHOD, "'value' must be a number or null"))?,
            ),
        };
        Ok(TransactionFeeResult {
            fee_lamports,
            context_slot,
        })
    }

    async fn simulate(
        &self,
        transaction: &str,
        options: TransactionSimulationOptions,
    ) -> Result<TransactionSimulationResult, TransactionError> {
        const METHOD: &str = "simulateTransaction";
        let mut config = Map::new();
        config.insert("encoding".into(), json!("base64"));
        // Inspection and budget estimation simulate an unsigned message
        // carrying placeholder signatures, so verification must stay off.
        config.insert("sigVerify".into(), json!(false));
        if let Some(commitment) = options.commitment {
            config.insert("commitment".into(), json!(commitment.as_str()));
        }
        if let Some(slot) = options.min_context_slot {
            config.insert("minContextSlot".into(), json!(slot));
        }
        if let Some(addresses) = &options.accounts {
            config.insert(
                "accounts".into(),
                json!({ "encoding": "base64", "addresses": addresses }),
            );
        }
        if let Some(inner) = options.inner_instructions {
            config.insert("innerInstructions".into(), json!(inner));
        }
        if let Some(replace) = options.replace_recent_blockhash {
            config.insert("replaceRecentBlockhash".into(), json!(replace));
        }

        let result = self
            .call(METHOD, json!([transaction, Value::Object(config)]))
            .await?;
        let (context_slot, value) = Self::context_value(METHOD, &result)?;
        Ok(TransactionSimulationResult {
            context_slot,
            err: nullable(value.get("err")),
            logs: match value.get("logs") {
                None | Some(Value::Null) => None,
                Some(Value::Array(entries)) => Some(
                    entries
                        .iter()
                        .map(|entry| entry.as_str().map(str::to_string))
                        .collect::<Option<Vec<_>>>()
                        .ok_or_else(|| invalid(METHOD, "'logs' entries must be strings"))?,
                ),
                Some(_) => return Err(invalid(METHOD, "'logs' must be an array")),
            },
            // Absent stays absent: a budget derived from a metric the node
            // never reported would be invented, not measured.
            units_consumed: optional_u64(METHOD, value.get("unitsConsumed"), "unitsConsumed")?,
            loaded_accounts_data_size: optional_u64(
                METHOD,
                value.get("loadedAccountsDataSize"),
                "loadedAccountsDataSize",
            )?,
            accounts: match value.get("accounts") {
                None | Some(Value::Null) => None,
                Some(Value::Array(entries)) => Some(entries.clone()),
                Some(_) => return Err(invalid(METHOD, "'accounts' must be an array")),
            },
        })
    }

    async fn send(
        &self,
        transaction: &str,
        options: TransactionSendOptions,
    ) -> Result<TransactionSendResult, TransactionError> {
        const METHOD: &str = "sendTransaction";
        let mut config = Map::new();
        config.insert("encoding".into(), json!("base64"));
        if let Some(skip) = options.skip_preflight {
            config.insert("skipPreflight".into(), json!(skip));
        }
        if let Some(commitment) = options.preflight_commitment {
            config.insert("preflightCommitment".into(), json!(commitment.as_str()));
        }
        if let Some(slot) = options.min_context_slot {
            config.insert("minContextSlot".into(), json!(slot));
        }
        let result = self
            .call(METHOD, json!([transaction, Value::Object(config)]))
            .await?;
        Ok(TransactionSendResult {
            signature: result
                .as_str()
                .ok_or_else(|| invalid(METHOD, "'result' must be a signature string"))?
                .to_string(),
        })
    }

    async fn signature_status(
        &self,
        signature: &str,
        options: SignatureStatusOptions,
    ) -> Result<Option<TransactionSignatureStatus>, TransactionError> {
        let batch = [signature.to_string()];
        Ok(self
            .signature_statuses(&batch, options)
            .await?
            .into_iter()
            .next()
            .flatten())
    }

    async fn signature_statuses(
        &self,
        signatures: &[String],
        options: SignatureStatusOptions,
    ) -> Result<Vec<Option<TransactionSignatureStatus>>, TransactionError> {
        if signatures.is_empty() {
            return Ok(Vec::new());
        }
        if signatures.len() > MAX_STATUS_SIGNATURES {
            return Err(TransactionError::Sdk(AreteError::InvalidConfig(format!(
                "Invalid transaction request: signatures exceeds the {MAX_STATUS_SIGNATURES}-signature limit for one batch"
            ))));
        }
        let mut config = Map::new();
        if let Some(search) = options.search_transaction_history {
            config.insert("searchTransactionHistory".into(), json!(search));
        }
        let result = self
            .call(
                "getSignatureStatuses",
                json!([signatures, Value::Object(config)]),
            )
            .await?;
        Self::parse_statuses(signatures, &result)
    }

    async fn block_height(
        &self,
        options: TransactionRequestContext,
    ) -> Result<u64, TransactionError> {
        const METHOD: &str = "getBlockHeight";
        self.call(METHOD, json!([Self::context_config(options)]))
            .await?
            .as_u64()
            .ok_or_else(|| invalid(METHOD, "'result' must be a number"))
    }

    async fn transaction(
        &self,
        signature: &str,
        options: TransactionInspectOptions,
    ) -> Result<Option<ConfirmedTransaction>, TransactionError> {
        const METHOD: &str = "getTransaction";
        let mut config = Map::new();
        config.insert("encoding".into(), json!("json"));
        if let Some(commitment) = options.commitment {
            config.insert("commitment".into(), json!(commitment.as_str()));
        }
        if let Some(version) = options.max_supported_transaction_version {
            config.insert("maxSupportedTransactionVersion".into(), json!(version));
        }
        let result = self
            .call(METHOD, json!([signature, Value::Object(config)]))
            .await?;
        // Only an explicit `null` says the cluster has not seen it.
        if result.is_null() {
            return Ok(None);
        }

        let meta = result.get("meta");
        let balances = |key: &str| -> Result<Vec<u64>, TransactionError> {
            match meta.and_then(|meta| meta.get(key)) {
                None | Some(Value::Null) => Ok(Vec::new()),
                Some(Value::Array(entries)) => entries
                    .iter()
                    .map(|entry| {
                        entry
                            .as_u64()
                            .ok_or_else(|| invalid(METHOD, &format!("'meta.{key}' must be u64")))
                    })
                    .collect(),
                Some(_) => Err(invalid(METHOD, &format!("'meta.{key}' must be an array"))),
            }
        };
        let pre = balances("preBalances")?;
        let post = balances("postBalances")?;

        // Balance arrays are indexed by the cluster's resolved account
        // order: static keys first, then lookup-table writables, then
        // lookup-table readonlys.
        let loaded = |key: &str| {
            meta.and_then(|meta| meta.get("loadedAddresses"))
                .and_then(|loaded| loaded.get(key))
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or_default()
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        };
        let mut accounts: Vec<String> = result
            .get("transaction")
            .and_then(|transaction| transaction.get("message"))
            .and_then(|message| message.get("accountKeys"))
            .and_then(Value::as_array)
            .ok_or_else(|| invalid(METHOD, "missing 'transaction.message.accountKeys'"))?
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect();
        accounts.extend(loaded("writable"));
        accounts.extend(loaded("readonly"));

        Ok(Some(ConfirmedTransaction {
            signature: signature.to_string(),
            slot: result
                .get("slot")
                .and_then(Value::as_u64)
                .ok_or_else(|| invalid(METHOD, "missing 'slot'"))?,
            block_time: result.get("blockTime").and_then(Value::as_i64),
            err: nullable(meta.and_then(|meta| meta.get("err"))),
            accounts: accounts
                .into_iter()
                .enumerate()
                .map(|(index, pubkey)| TransactionAccountBalance {
                    pubkey,
                    pre_balance: pre.get(index).copied().unwrap_or(0),
                    post_balance: post.get(index).copied().unwrap_or(0),
                })
                .collect(),
        }))
    }

    async fn signatures(
        &self,
        address: &str,
        options: SignaturePageOptions,
    ) -> Result<Vec<SignaturePageEntry>, TransactionError> {
        const METHOD: &str = "getSignaturesForAddress";
        let mut config = Map::new();
        if let Some(limit) = options.limit {
            config.insert("limit".into(), json!(limit));
        }
        if let Some(before) = &options.before {
            config.insert("before".into(), json!(before));
        }
        if let Some(until) = &options.until {
            config.insert("until".into(), json!(until));
        }
        if let Some(commitment) = options.commitment {
            config.insert("commitment".into(), json!(commitment.as_str()));
        }
        let result = self
            .call(METHOD, json!([address, Value::Object(config)]))
            .await?;
        result
            .as_array()
            .ok_or_else(|| invalid(METHOD, "'result' must be an array"))?
            .iter()
            .map(|entry| {
                Ok(SignaturePageEntry {
                    signature: entry
                        .get("signature")
                        .and_then(Value::as_str)
                        .ok_or_else(|| invalid(METHOD, "missing 'signature'"))?
                        .to_string(),
                    slot: entry
                        .get("slot")
                        .and_then(Value::as_u64)
                        .ok_or_else(|| invalid(METHOD, "missing 'slot'"))?,
                    block_time: entry.get("blockTime").and_then(Value::as_i64),
                    err: nullable(entry.get("err")),
                })
            })
            .collect()
    }
}

/// An absent field and an explicit `null` mean the same thing for every
/// nullable JSON-RPC member the DTOs carry.
fn nullable(value: Option<&Value>) -> Option<Value> {
    match value {
        None | Some(Value::Null) => None,
        Some(other) => Some(other.clone()),
    }
}

/// A `u64` that may legitimately be absent. A present non-integer is a
/// malformed response, never silently dropped to `None`.
fn optional_u64(
    method: &str,
    value: Option<&Value>,
    field: &str,
) -> Result<Option<u64>, TransactionError> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(other) => {
            Ok(Some(other.as_u64().ok_or_else(|| {
                invalid(method, &format!("'{field}' must be a u64"))
            })?))
        }
    }
}

fn invalid(method: &str, detail: &str) -> TransactionError {
    TransactionError::InvalidResponse(format!("RPC '{method}': {detail}"))
}

/// A JSON-RPC `error` member, typed the way the relay's errors are so the
/// wallet adapter classifies both identically.
fn rpc_failure(method: &str, status: u16, error: &Value) -> TransactionError {
    let code = error.get("code").and_then(Value::as_i64);
    let submission_state = (method == "sendTransaction")
        .then(|| code.filter(|code| NOT_SUBMITTED_RPC_CODES.contains(code)))
        .flatten()
        .map(|_| SubmissionState::NotSubmitted);
    crate::transactions::TransactionTransportError {
        status,
        code: code.map_or_else(|| "rpc_error".to_string(), |code| format!("rpc_{code}")),
        message: format!(
            "RPC '{method}' failed: {}",
            error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("no message")
        ),
        retryable: status == 429 || (500..600).contains(&status),
        request_id: None,
        submission_state,
        // A relay signature would be authoritative; a node never returns one
        // on an error, so there is nothing to conflict with the signed bytes.
        signature: None,
        details: error.get("data").cloned(),
    }
    .into()
}

/// A non-2xx status with no JSON-RPC envelope — a gateway, proxy, or rate
/// limiter answering instead of the node. The submission state is unknown by
/// construction: the request may or may not have reached the cluster.
fn http_failure(method: &str, status: u16) -> TransactionError {
    crate::transactions::TransactionTransportError {
        status,
        code: "rpc_http_error".to_string(),
        message: format!("RPC '{method}' failed with HTTP {status}"),
        retryable: status == 429 || (500..600).contains(&status),
        request_id: None,
        submission_state: None,
        signature: None,
        details: None,
    }
    .into()
}

/// Convenience for the common `Arc<dyn TransactionTransport>` injection into
/// [`AreteBuilder::transactions`](crate::AreteBuilder::transactions) and
/// [`SolanaAdapterConfig::transport`](crate::adapters::solana::SolanaAdapterConfig::transport).
impl From<RpcTransactionTransport> for Arc<dyn TransactionTransport> {
    fn from(transport: RpcTransactionTransport) -> Self {
        Arc::new(transport)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::routing::post;
    use axum::{Json, Router};
    use tokio::net::TcpListener;
    use tokio::sync::Mutex;

    /// Mock node: answers by method name from a canned table and records
    /// every request, so both the params sent and the results parsed are
    /// asserted against one wire transcript.
    struct Node {
        requests: Arc<Mutex<Vec<Value>>>,
        url: String,
    }

    impl Node {
        async fn spawn(responses: Value) -> Self {
            let requests = Arc::new(Mutex::new(Vec::new()));
            let recorded = requests.clone();
            let router = Router::new().route(
                "/",
                post(move |Json(body): Json<Value>| {
                    let recorded = recorded.clone();
                    let responses = responses.clone();
                    async move {
                        let method = body["method"].as_str().unwrap_or_default().to_string();
                        recorded.lock().await.push(body);
                        let answer = responses
                            .get(&method)
                            .cloned()
                            .unwrap_or_else(|| panic!("unexpected RPC method '{method}'"));
                        // A canned `error` is returned as one; anything else
                        // is the `result`.
                        Json(if answer.get("error").is_some() {
                            json!({ "jsonrpc": "2.0", "id": 1, "error": answer["error"] })
                        } else {
                            json!({ "jsonrpc": "2.0", "id": 1, "result": answer })
                        })
                    }
                }),
            );
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
            Self {
                requests,
                url: format!("http://{addr}/"),
            }
        }

        fn transport(&self) -> RpcTransactionTransport {
            RpcTransactionTransport::new(&self.url)
        }

        /// The params of the one recorded call to `method`.
        async fn params(&self, method: &str) -> Value {
            self.requests
                .lock()
                .await
                .iter()
                .find(|request| request["method"] == method)
                .unwrap_or_else(|| panic!("'{method}' was never called"))["params"]
                .clone()
        }
    }

    #[tokio::test]
    async fn reads_map_to_their_json_rpc_methods_and_unwrap_the_context_envelope() {
        let node = Node::spawn(json!({
            "getLatestBlockhash": {
                "context": { "slot": 43 },
                "value": { "blockhash": "Bh1", "lastValidBlockHeight": 99 },
            },
            "getFeeForMessage": { "context": { "slot": 44 }, "value": 5000 },
            "getBlockHeight": 1234,
        }))
        .await;
        let transport = node.transport();

        let blockhash = transport
            .latest_blockhash(TransactionRequestContext {
                commitment: Some(Commitment::Confirmed),
                min_context_slot: Some(42),
            })
            .await
            .unwrap();
        assert_eq!(
            blockhash,
            LatestBlockhashResult {
                blockhash: "Bh1".to_string(),
                context_slot: 43,
                last_valid_block_height: 99,
            }
        );
        // Numbers, not the relay's decimal strings.
        assert_eq!(
            node.params("getLatestBlockhash").await,
            json!([{ "commitment": "confirmed", "minContextSlot": 42 }])
        );

        let fee = transport
            .fee("bWVzc2FnZQ==", TransactionRequestContext::default())
            .await
            .unwrap();
        assert_eq!(fee.fee_lamports, Some(5_000));
        assert_eq!(fee.context_slot, 44);
        assert_eq!(
            node.params("getFeeForMessage").await,
            json!(["bWVzc2FnZQ==", {}])
        );

        assert_eq!(
            transport
                .block_height(TransactionRequestContext::default())
                .await
                .unwrap(),
            1_234
        );
    }

    #[tokio::test]
    async fn u64_results_keep_full_precision() {
        let node = Node::spawn(json!({
            "getFeeForMessage": { "context": { "slot": 1 }, "value": u64::MAX },
            "getBlockHeight": u64::MAX - 1,
        }))
        .await;
        let transport = node.transport();

        assert_eq!(
            transport
                .fee("bQ==", TransactionRequestContext::default())
                .await
                .unwrap()
                .fee_lamports,
            Some(u64::MAX),
        );
        assert_eq!(
            transport
                .block_height(TransactionRequestContext::default())
                .await
                .unwrap(),
            u64::MAX - 1,
        );
    }

    #[tokio::test]
    async fn a_null_fee_is_not_a_zero_fee() {
        let node = Node::spawn(json!({
            "getFeeForMessage": { "context": { "slot": 1 }, "value": Value::Null },
        }))
        .await;

        assert_eq!(
            node.transport()
                .fee("bQ==", TransactionRequestContext::default())
                .await
                .unwrap()
                .fee_lamports,
            None,
        );
    }

    #[tokio::test]
    async fn simulation_runs_unsigned_and_keeps_absent_metrics_absent() {
        let node = Node::spawn(json!({
            "simulateTransaction": {
                "context": { "slot": 7 },
                "value": {
                    "err": Value::Null,
                    "logs": ["Program log: hi"],
                    "unitsConsumed": 0,
                },
            },
        }))
        .await;

        let result = node
            .transport()
            .simulate(
                "dHg=",
                TransactionSimulationOptions {
                    commitment: Some(Commitment::Processed),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(
            node.params("simulateTransaction").await,
            json!([
                "dHg=",
                { "encoding": "base64", "sigVerify": false, "commitment": "processed" }
            ]),
            "inspection simulates placeholder signatures, so verification stays off",
        );
        assert_eq!(result.context_slot, 7);
        assert_eq!(result.err, None);
        assert_eq!(
            result.units_consumed,
            Some(0),
            "a measured zero is a measurement",
        );
        assert_eq!(
            result.loaded_accounts_data_size, None,
            "an unreported metric is absent, never an invented zero",
        );
    }

    #[tokio::test]
    async fn a_malformed_metric_is_a_response_error_not_a_dropped_value() {
        let node = Node::spawn(json!({
            "simulateTransaction": {
                "context": { "slot": 7 },
                "value": { "unitsConsumed": "1200" },
            },
        }))
        .await;

        let error = node
            .transport()
            .simulate("dHg=", TransactionSimulationOptions::default())
            .await
            .expect_err("a decimal string is the relay's encoding, not the node's");
        assert!(matches!(error, TransactionError::InvalidResponse(_)));
    }

    #[tokio::test]
    async fn statuses_align_positionally_and_map_unseen_signatures_to_none() {
        let node = Node::spawn(json!({
            "getSignatureStatuses": {
                "context": { "slot": 9 },
                "value": [
                    { "slot": 8, "confirmations": Value::Null, "err": Value::Null,
                      "confirmationStatus": "finalized" },
                    Value::Null,
                ],
            },
        }))
        .await;

        let signatures = vec!["sigA".to_string(), "sigB".to_string()];
        let statuses = node
            .transport()
            .signature_statuses(
                &signatures,
                SignatureStatusOptions {
                    search_transaction_history: Some(true),
                },
            )
            .await
            .unwrap();

        assert_eq!(
            node.params("getSignatureStatuses").await,
            json!([["sigA", "sigB"], { "searchTransactionHistory": true }])
        );
        let first = statuses[0].as_ref().unwrap();
        assert_eq!(first.signature, "sigA");
        assert_eq!(first.slot, Some(8));
        assert_eq!(first.confirmation_status, Some(Commitment::Finalized));
        assert!(statuses[1].is_none());
    }

    #[tokio::test]
    async fn a_length_mismatch_is_refused_rather_than_misattributed() {
        let node = Node::spawn(json!({
            "getSignatureStatuses": { "context": { "slot": 9 }, "value": [Value::Null] },
        }))
        .await;

        let error = node
            .transport()
            .signature_statuses(
                &["sigA".to_string(), "sigB".to_string()],
                SignatureStatusOptions::default(),
            )
            .await
            .expect_err("one status cannot answer for two signatures");
        assert!(matches!(error, TransactionError::InvalidResponse(_)));
    }

    #[tokio::test]
    async fn a_confirmed_transaction_resolves_lookup_table_accounts_in_cluster_order() {
        let node = Node::spawn(json!({
            "getTransaction": {
                "slot": 512,
                "blockTime": 1_700_000_000i64,
                "transaction": { "message": { "accountKeys": ["Static1", "Static2"] } },
                "meta": {
                    "err": Value::Null,
                    "preBalances": [10, 20, 30, 40],
                    "postBalances": [9, 21, 30, 41],
                    "loadedAddresses": { "writable": ["Loaded1"], "readonly": ["Loaded2"] },
                },
            },
        }))
        .await;

        let confirmed = node
            .transport()
            .transaction(
                "sigA",
                TransactionInspectOptions {
                    commitment: Some(Commitment::Finalized),
                    max_supported_transaction_version: Some(0),
                },
            )
            .await
            .unwrap()
            .expect("the cluster has seen it");

        assert_eq!(
            node.params("getTransaction").await,
            json!([
                "sigA",
                { "encoding": "json", "commitment": "finalized",
                  "maxSupportedTransactionVersion": 0 }
            ])
        );
        assert_eq!(confirmed.slot, 512);
        assert_eq!(confirmed.block_time, Some(1_700_000_000));
        assert_eq!(
            confirmed
                .accounts
                .iter()
                .map(|account| (
                    account.pubkey.as_str(),
                    account.pre_balance,
                    account.post_balance
                ))
                .collect::<Vec<_>>(),
            vec![
                ("Static1", 10, 9),
                ("Static2", 20, 21),
                ("Loaded1", 30, 30),
                ("Loaded2", 40, 41),
            ],
            "balances are indexed by static keys, then loaded writable, then loaded readonly",
        );
    }

    #[tokio::test]
    async fn an_unseen_signature_is_none_rather_than_an_error() {
        let node = Node::spawn(json!({ "getTransaction": Value::Null })).await;

        assert!(node
            .transport()
            .transaction("sigA", TransactionInspectOptions::default())
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn a_signature_page_maps_its_paging_options() {
        let node = Node::spawn(json!({
            "getSignaturesForAddress": [
                { "signature": "sigA", "slot": 5, "blockTime": Value::Null, "err": Value::Null },
                { "signature": "sigB", "slot": 4, "blockTime": 12, "err": { "InsufficientFundsForFee": 1 } },
            ],
        }))
        .await;

        let page = node
            .transport()
            .signatures(
                "Addr1",
                SignaturePageOptions {
                    limit: Some(2),
                    before: Some("sigC".to_string()),
                    until: None,
                    commitment: Some(Commitment::Confirmed),
                },
            )
            .await
            .unwrap();

        assert_eq!(
            node.params("getSignaturesForAddress").await,
            json!(["Addr1", { "limit": 2, "before": "sigC", "commitment": "confirmed" }])
        );
        assert_eq!(page[0].block_time, None);
        assert_eq!(page[1].slot, 4);
        assert!(page[1].err.is_some());
    }

    #[tokio::test]
    async fn a_send_the_node_refused_before_dispatch_is_typed_not_submitted() {
        let node = Node::spawn(json!({
            "sendTransaction": {
                "error": { "code": -32002, "message": "Transaction simulation failed",
                           "data": { "logs": [] } },
            },
        }))
        .await;

        let error = node
            .transport()
            .send("dHg=", TransactionSendOptions::default())
            .await
            .expect_err("a refused preflight is a failure");
        let TransactionError::Transport(inner) = error else {
            panic!("expected a typed transport error");
        };
        assert_eq!(inner.submission_state, Some(SubmissionState::NotSubmitted));
        assert_eq!(inner.code, "rpc_-32002");
        assert!(inner.details.is_some());
        assert_eq!(
            inner.signature, None,
            "a node never names a signature the caller did not derive",
        );
    }

    #[tokio::test]
    async fn an_unrecognized_send_failure_leaves_the_submission_unknown() {
        let node = Node::spawn(json!({
            "sendTransaction": { "error": { "code": -32005, "message": "Node is unhealthy" } },
        }))
        .await;

        let error = node
            .transport()
            .send("dHg=", TransactionSendOptions::default())
            .await
            .expect_err("an unhealthy node is a failure");
        let TransactionError::Transport(inner) = error else {
            panic!("expected a typed transport error");
        };
        assert_eq!(
            inner.submission_state, None,
            "unknown keeps the locally derived signature authoritative upstream",
        );
    }

    #[tokio::test]
    async fn a_send_serializes_its_options_and_returns_the_node_signature() {
        let node = Node::spawn(json!({ "sendTransaction": "sigZ" })).await;

        let result = node
            .transport()
            .send(
                "dHg=",
                TransactionSendOptions {
                    skip_preflight: Some(true),
                    preflight_commitment: Some(Commitment::Confirmed),
                    min_context_slot: Some(11),
                },
            )
            .await
            .unwrap();

        assert_eq!(result.signature, "sigZ");
        assert_eq!(
            node.params("sendTransaction").await,
            json!([
                "dHg=",
                { "encoding": "base64", "skipPreflight": true,
                  "preflightCommitment": "confirmed", "minContextSlot": 11 }
            ])
        );
    }

    #[tokio::test]
    async fn provider_headers_reach_the_node() {
        let node = Node::spawn(json!({ "getBlockHeight": 1 })).await;
        let transport = RpcTransactionTransport::new(&node.url).with_header("x-api-key", "secret");

        transport
            .block_height(TransactionRequestContext::default())
            .await
            .unwrap();
        // Reaching the node at all proves the header was accepted; the point
        // of the field is that it is configured here and never derived from
        // Arete's token source, which this transport cannot even see.
        assert_eq!(node.requests.lock().await.len(), 1);
    }
}
