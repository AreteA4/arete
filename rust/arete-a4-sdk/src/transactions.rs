//! Transaction relay transport (`POST <base>/transactions/v1/*`).
//!
//! Port of `typescript/core/src/transactions.ts`: the [`TransactionTransport`]
//! trait with the relay routes and [`HttpTransactionTransport`], the HTTP
//! implementation authenticated through [`crate::http`].
//!
//! Scopes: every route uses `transaction:inspect` except `send`, which uses
//! `transaction:send` and only allows the auth refresh-replay when the failed
//! response carries `X-Arete-Upstream-Attempted: false` (the server proved the
//! transaction was never dispatched). Sends are never auto-retried otherwise.
//!
//! `u64` fields are decimal strings on the wire.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Map, Value};
use thiserror::Error;

use crate::error::AreteError;
use crate::http::{fetch_json, AuthedRequest, HttpMethod, TokenSource};

/// Commitment levels accepted by the relay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Commitment {
    Processed,
    Confirmed,
    Finalized,
}

impl Commitment {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Processed => "processed",
            Self::Confirmed => "confirmed",
            Self::Finalized => "finalized",
        }
    }

    fn from_wire(value: &str) -> Option<Self> {
        match value {
            "processed" => Some(Self::Processed),
            "confirmed" => Some(Self::Confirmed),
            "finalized" => Some(Self::Finalized),
            _ => None,
        }
    }
}

/// Shared request context (`commitment` + `minContextSlot`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TransactionRequestContext {
    pub commitment: Option<Commitment>,
    pub min_context_slot: Option<u64>,
}

/// Result of `latest-blockhash`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LatestBlockhashResult {
    pub blockhash: String,
    pub context_slot: u64,
    pub last_valid_block_height: u64,
}

/// Result of `fee`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransactionFeeResult {
    pub fee_lamports: Option<u64>,
    pub context_slot: u64,
}

/// Options for `simulate`.
#[derive(Debug, Clone, Default)]
pub struct TransactionSimulationOptions {
    pub commitment: Option<Commitment>,
    pub min_context_slot: Option<u64>,
    pub accounts: Option<Vec<String>>,
    pub inner_instructions: Option<bool>,
    pub replace_recent_blockhash: Option<bool>,
}

/// Result of `simulate`.
///
/// `loaded_accounts_data_size` is the V1 budget input (SIMD-0385): a V1
/// transaction's resource budget cannot be estimated without knowing how much
/// account data the transaction loads. `None` means the relay did not report
/// it (absent or explicit `null`); `Some(0)` means it reported zero — the
/// distinction is preserved.
#[derive(Debug, Clone, PartialEq)]
pub struct TransactionSimulationResult {
    pub context_slot: u64,
    pub err: Option<Value>,
    pub logs: Option<Vec<String>>,
    pub units_consumed: Option<u64>,
    pub loaded_accounts_data_size: Option<u64>,
    pub accounts: Option<Vec<Value>>,
}

/// Options for `send`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TransactionSendOptions {
    pub skip_preflight: Option<bool>,
    pub preflight_commitment: Option<Commitment>,
    pub min_context_slot: Option<u64>,
}

/// Result of `send`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransactionSendResult {
    pub signature: String,
}

/// Options for `signature-status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SignatureStatusOptions {
    pub search_transaction_history: Option<bool>,
}

/// Result of `signature-status` (when the signature is known).
#[derive(Debug, Clone, PartialEq)]
pub struct TransactionSignatureStatus {
    pub signature: String,
    pub slot: Option<u64>,
    pub confirmation_status: Option<Commitment>,
    pub err: Option<Value>,
}

/// Options for `get`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TransactionInspectOptions {
    pub commitment: Option<Commitment>,
    /// Highest transaction version the cluster may encode. `None` leaves the choice to the relay,
    /// which tracks the network; pin it only to reproduce an older client's view.
    pub max_supported_transaction_version: Option<u8>,
}

/// One account's lamport balance either side of a transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransactionAccountBalance {
    pub pubkey: String,
    pub pre_balance: u64,
    pub post_balance: u64,
}

/// Result of `get` — a confirmed transaction's effect, not its instructions.
///
/// `accounts` covers every account the transaction resolved, lookup-table entries included, in the
/// cluster's own order.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfirmedTransaction {
    pub signature: String,
    pub slot: u64,
    pub block_time: Option<i64>,
    pub err: Option<Value>,
    pub accounts: Vec<TransactionAccountBalance>,
}

/// Options for `signatures`. `before`/`until` are signatures, exclusive on both ends, and page
/// backwards through history the way `getSignaturesForAddress` does.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SignaturePageOptions {
    pub limit: Option<u16>,
    pub before: Option<String>,
    pub until: Option<String>,
    pub commitment: Option<Commitment>,
}

/// One entry of `signatures` — enough to decide what to fetch, not the transaction itself.
#[derive(Debug, Clone, PartialEq)]
pub struct SignaturePageEntry {
    pub signature: String,
    pub slot: u64,
    pub block_time: Option<i64>,
    pub err: Option<Value>,
}

/// Submission state reported by relay error bodies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmissionState {
    NotSubmitted,
    Unknown,
}

impl SubmissionState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotSubmitted => "not_submitted",
            Self::Unknown => "unknown",
        }
    }

    fn from_wire(value: &str) -> Option<Self> {
        match value {
            "not_submitted" => Some(Self::NotSubmitted),
            "unknown" => Some(Self::Unknown),
            _ => None,
        }
    }
}

/// Stable transport error metadata, porting the TS `TransactionTransportError`.
///
/// Bodies are parsed defensively (both `request_id`/`requestId` and
/// `submission_state`/`submissionState` spellings are accepted); non-JSON
/// bodies are never reflected — the error synthesizes the
/// `transaction_transport_error` code and a generic message instead.
#[derive(Debug, Clone, Error)]
#[error("{message}")]
pub struct TransactionTransportError {
    pub status: u16,
    pub code: String,
    pub message: String,
    pub retryable: bool,
    pub request_id: Option<String>,
    pub submission_state: Option<SubmissionState>,
    pub signature: Option<String>,
    pub details: Option<Value>,
}

impl TransactionTransportError {
    /// Parses a relay error body, mirroring the TS `parseError`.
    pub fn from_response(status: u16, body: &[u8]) -> Self {
        let parsed: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
        let get = |key: &str| parsed.get(key);
        let string = |key: &str| get(key).and_then(Value::as_str).map(str::to_string);

        Self {
            status,
            code: string("code").unwrap_or_else(|| "transaction_transport_error".to_string()),
            message: string("message")
                .unwrap_or_else(|| format!("Transaction request failed ({status})")),
            retryable: get("retryable").and_then(Value::as_bool) == Some(true),
            request_id: string("request_id").or_else(|| string("requestId")),
            submission_state: string("submission_state")
                .or_else(|| string("submissionState"))
                .as_deref()
                .and_then(SubmissionState::from_wire),
            signature: string("signature"),
            details: get("details").cloned(),
        }
    }
}

/// Errors produced by the transaction transport.
#[derive(Debug, Error)]
pub enum TransactionError {
    /// The relay answered with a non-2xx status.
    #[error(transparent)]
    Transport(Box<TransactionTransportError>),

    /// The response body did not match the wire contract.
    #[error("Invalid transaction response: {0}")]
    InvalidResponse(String),

    /// Transport or authentication failure from the SDK core.
    #[error(transparent)]
    Sdk(#[from] AreteError),
}

impl From<TransactionTransportError> for TransactionError {
    fn from(error: TransactionTransportError) -> Self {
        Self::Transport(Box::new(error))
    }
}

impl From<TransactionError> for AreteError {
    fn from(error: TransactionError) -> Self {
        match error {
            TransactionError::Transport(inner) => AreteError::ConnectionFailed(format!(
                "Transaction request failed ({}): {} [{}]",
                inner.status, inner.message, inner.code
            )),
            TransactionError::InvalidResponse(message) => AreteError::Serialization(message),
            TransactionError::Sdk(inner) => inner,
        }
    }
}

/// Solana's `getSignatureStatuses` ceiling, mirrored so an oversized batch fails here rather than
/// as a remote 400.
///
/// A batch this size is roughly 23 KiB of JSON, so the relay's transaction body limit
/// (`ARETE_TRANSACTION_MAX_BODY_BYTES`) has to admit it. Lowering that setting below ~24 KiB caps
/// the batch that can actually be sent, and the relay answers `request_too_large`.
pub const MAX_STATUS_SIGNATURES: usize = 256;

/// Access to the stack's transaction relay (`/transactions/v1/*`).
#[async_trait]
pub trait TransactionTransport: Send + Sync {
    /// `POST latest-blockhash`.
    async fn latest_blockhash(
        &self,
        options: TransactionRequestContext,
    ) -> Result<LatestBlockhashResult, TransactionError>;

    /// `POST fee` — fee for a base64-encoded message.
    async fn fee(
        &self,
        message: &str,
        options: TransactionRequestContext,
    ) -> Result<TransactionFeeResult, TransactionError>;

    /// `POST simulate` — simulate a base64-encoded transaction.
    async fn simulate(
        &self,
        transaction: &str,
        options: TransactionSimulationOptions,
    ) -> Result<TransactionSimulationResult, TransactionError>;

    /// `POST send` — submit a signed base64-encoded transaction. Never
    /// auto-retried; the auth refresh-replay requires the predispatch marker.
    async fn send(
        &self,
        transaction: &str,
        options: TransactionSendOptions,
    ) -> Result<TransactionSendResult, TransactionError>;

    /// `POST signature-status`.
    async fn signature_status(
        &self,
        signature: &str,
        options: SignatureStatusOptions,
    ) -> Result<Option<TransactionSignatureStatus>, TransactionError>;

    /// `POST signature-statuses` — up to [`MAX_STATUS_SIGNATURES`] signatures in one call.
    ///
    /// Results are positionally aligned with `signatures`; `None` means the cluster has not seen
    /// that signature.
    ///
    /// Defaults to one [`signature_status`](Self::signature_status) call per signature, which is
    /// what every caller did before the batch route existed. That keeps this addition
    /// source-compatible for anything implementing this trait outside the crate, and correct — the
    /// alignment is trivially preserved by construction. It is also the slow path this method
    /// exists to replace, so a transport that can reach `POST signature-statuses` should override
    /// it; the one in this crate does.
    async fn signature_statuses(
        &self,
        signatures: &[String],
        options: SignatureStatusOptions,
    ) -> Result<Vec<Option<TransactionSignatureStatus>>, TransactionError> {
        if signatures.len() > MAX_STATUS_SIGNATURES {
            return Err(TransactionError::Sdk(AreteError::InvalidConfig(format!(
                "Invalid transaction request: signatures exceeds the {MAX_STATUS_SIGNATURES}-signature limit for one batch"
            ))));
        }

        let mut statuses = Vec::with_capacity(signatures.len());
        for signature in signatures {
            statuses.push(self.signature_status(signature, options).await?);
        }
        Ok(statuses)
    }

    /// `POST block-height`.
    async fn block_height(
        &self,
        options: TransactionRequestContext,
    ) -> Result<u64, TransactionError>;

    /// `POST get` — a confirmed transaction's balance effect. `None` means the cluster has not
    /// seen the signature at the requested commitment.
    ///
    /// Defaults to an unsupported-capability error: unlike the status batch there is no slower
    /// path to fall back to, so a transport that cannot reach `POST /transactions/v1/get` must say
    /// so rather than invent an answer. Keeps this addition source-compatible for out-of-crate
    /// implementations; the one in this crate overrides it.
    async fn transaction(
        &self,
        _signature: &str,
        _options: TransactionInspectOptions,
    ) -> Result<Option<ConfirmedTransaction>, TransactionError> {
        Err(TransactionError::Sdk(AreteError::InvalidConfig(
            "This transport does not implement POST /transactions/v1/get".to_string(),
        )))
    }

    /// `POST signatures` — a page of an address's transaction history, newest first.
    ///
    /// Defaults to an unsupported-capability error for the same reason as
    /// [`transaction`](Self::transaction): no other route can enumerate history.
    async fn signatures(
        &self,
        _address: &str,
        _options: SignaturePageOptions,
    ) -> Result<Vec<SignaturePageEntry>, TransactionError> {
        Err(TransactionError::Sdk(AreteError::InvalidConfig(
            "This transport does not implement POST /transactions/v1/signatures".to_string(),
        )))
    }
}

const SCOPE_INSPECT: &str = "transaction:inspect";
const SCOPE_SEND: &str = "transaction:send";

/// Parse one wire status entry. `None` means the cluster has not seen the signature — shared so
/// the single and batch routes cannot interpret a status differently.
fn parse_signature_status(
    signature: &str,
    status: Option<&Value>,
) -> Result<Option<TransactionSignatureStatus>, TransactionError> {
    let status = match status {
        None | Some(Value::Null) => return Ok(None),
        Some(status) => status,
    };
    let slot = match status.get("slot") {
        None | Some(Value::Null) => None,
        other => Some(decimal_u64(other, "slot")?),
    };
    let confirmation_status = status
        .get("confirmationStatus")
        .and_then(Value::as_str)
        .and_then(Commitment::from_wire);
    let err = match status.get("err") {
        None | Some(Value::Null) => None,
        Some(other) => Some(other.clone()),
    };
    Ok(Some(TransactionSignatureStatus {
        signature: signature.to_string(),
        slot,
        confirmation_status,
        err,
    }))
}

/// Nullable decimal `i64` on the wire — `blockTime` is the only one, and it predates the epoch
/// for no cluster anyone runs, but the RPC type is signed so the parse is too.
fn optional_decimal_i64(
    value: Option<&Value>,
    field: &str,
) -> Result<Option<i64>, TransactionError> {
    let invalid = || {
        TransactionError::InvalidResponse(format!(
            "Invalid decimal i64 field '{field}' in transaction response"
        ))
    };
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => text.parse::<i64>().map(Some).map_err(|_| invalid()),
        Some(_) => Err(invalid()),
    }
}

/// Parse one wire entry of a `signatures` page.
fn parse_signature_entry(entry: &Value) -> Result<SignaturePageEntry, TransactionError> {
    Ok(SignaturePageEntry {
        signature: entry
            .get("signature")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                TransactionError::InvalidResponse(
                    "Missing 'signature' in transaction response".to_string(),
                )
            })?
            .to_string(),
        slot: decimal_u64(entry.get("slot"), "slot")?,
        block_time: optional_decimal_i64(entry.get("blockTime"), "blockTime")?,
        err: match entry.get("err") {
            None | Some(Value::Null) => None,
            Some(err) => Some(err.clone()),
        },
    })
}

/// Parse the `transaction` field of a `get` response. `None` means the cluster has not seen it —
/// only an explicit JSON `null` says that. An absent field is a malformed response, not an unseen
/// signature: a caller acting on "unseen" waits for a transaction that may already be final.
fn parse_confirmed_transaction(
    value: Option<&Value>,
) -> Result<Option<ConfirmedTransaction>, TransactionError> {
    let transaction = match value {
        None => {
            return Err(TransactionError::InvalidResponse(
                "Missing 'transaction' in transaction response".into(),
            ))
        }
        Some(Value::Null) => return Ok(None),
        Some(transaction) => transaction,
    };
    let signature = transaction
        .get("signature")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            TransactionError::InvalidResponse("Missing 'signature' in transaction response".into())
        })?
        .to_string();
    let block_time = optional_decimal_i64(transaction.get("blockTime"), "blockTime")?;
    let accounts = transaction
        .get("accounts")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            TransactionError::InvalidResponse(
                "'accounts' must be an array in transaction response".into(),
            )
        })?
        .iter()
        .map(|account| {
            Ok(TransactionAccountBalance {
                pubkey: account
                    .get("pubkey")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        TransactionError::InvalidResponse(
                            "Missing 'pubkey' in transaction response".to_string(),
                        )
                    })?
                    .to_string(),
                pre_balance: decimal_u64(account.get("preBalance"), "preBalance")?,
                post_balance: decimal_u64(account.get("postBalance"), "postBalance")?,
            })
        })
        .collect::<Result<Vec<_>, TransactionError>>()?;
    Ok(Some(ConfirmedTransaction {
        signature,
        slot: decimal_u64(transaction.get("slot"), "slot")?,
        block_time,
        err: match transaction.get("err") {
            None | Some(Value::Null) => None,
            Some(err) => Some(err.clone()),
        },
        accounts,
    }))
}

fn decimal(value: Option<u64>) -> Option<Value> {
    value.map(|v| Value::String(v.to_string()))
}

fn decimal_u64(value: Option<&Value>, field: &str) -> Result<u64, TransactionError> {
    let Some(Value::String(text)) = value else {
        return Err(TransactionError::InvalidResponse(format!(
            "Invalid decimal u64 field '{field}' in transaction response"
        )));
    };
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(TransactionError::InvalidResponse(format!(
            "Invalid decimal u64 field '{field}' in transaction response"
        )));
    }
    text.parse::<u64>().map_err(|_| {
        TransactionError::InvalidResponse(format!(
            "Invalid decimal u64 field '{field}' in transaction response"
        ))
    })
}

fn optional_decimal_u64(
    value: Option<&Value>,
    field: &str,
) -> Result<Option<u64>, TransactionError> {
    match value {
        None | Some(Value::Null) => Ok(None),
        some => decimal_u64(some, field).map(Some),
    }
}

struct BodyBuilder(Map<String, Value>);

impl BodyBuilder {
    fn new() -> Self {
        Self(Map::new())
    }

    fn set(mut self, key: &str, value: impl Into<Value>) -> Self {
        self.0.insert(key.to_string(), value.into());
        self
    }

    fn maybe(mut self, key: &str, value: Option<impl Into<Value>>) -> Self {
        if let Some(value) = value {
            self.0.insert(key.to_string(), value.into());
        }
        self
    }

    fn build(self) -> Value {
        Value::Object(self.0)
    }
}

fn commitment_value(commitment: Option<Commitment>) -> Option<Value> {
    commitment.map(|c| Value::String(c.as_str().to_string()))
}

/// HTTP [`TransactionTransport`] over `<base>/transactions/v1`.
pub struct HttpTransactionTransport {
    root: String,
    tokens: Arc<dyn TokenSource>,
    http: reqwest::Client,
}

impl HttpTransactionTransport {
    pub fn new(base_url: impl Into<String>, tokens: Arc<dyn TokenSource>) -> Self {
        Self::with_http_client(base_url, tokens, reqwest::Client::new())
    }

    pub fn with_http_client(
        base_url: impl Into<String>,
        tokens: Arc<dyn TokenSource>,
        http_client: reqwest::Client,
    ) -> Self {
        let base = base_url.into();
        Self {
            root: format!("{}/transactions/v1", base.trim_end_matches('/')),
            tokens,
            http: http_client,
        }
    }

    async fn post(
        &self,
        route: &str,
        body: Value,
        scope: &str,
        require_predispatch_marker: bool,
    ) -> Result<Value, TransactionError> {
        let request = AuthedRequest {
            method: HttpMethod::Post,
            url: format!("{}/{route}", self.root),
            body: Some(body),
            scopes: vec![scope.to_string()],
            target: None,
            require_predispatch_marker,
        };
        let response = fetch_json(&self.http, self.tokens.as_ref(), &request).await?;
        if !response.is_success() {
            return Err(TransactionError::from(
                TransactionTransportError::from_response(response.status, &response.body),
            ));
        }
        response
            .json()
            .map_err(|error| TransactionError::InvalidResponse(error.to_string()))
    }
}

#[async_trait]
impl TransactionTransport for HttpTransactionTransport {
    async fn latest_blockhash(
        &self,
        options: TransactionRequestContext,
    ) -> Result<LatestBlockhashResult, TransactionError> {
        let body = BodyBuilder::new()
            .maybe("commitment", commitment_value(options.commitment))
            .maybe("minContextSlot", decimal(options.min_context_slot))
            .build();
        let value = self
            .post("latest-blockhash", body, SCOPE_INSPECT, false)
            .await?;
        let blockhash = value
            .get("blockhash")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                TransactionError::InvalidResponse(
                    "Missing 'blockhash' in transaction response".to_string(),
                )
            })?
            .to_string();
        Ok(LatestBlockhashResult {
            blockhash,
            context_slot: decimal_u64(value.get("contextSlot"), "contextSlot")?,
            last_valid_block_height: decimal_u64(
                value.get("lastValidBlockHeight"),
                "lastValidBlockHeight",
            )?,
        })
    }

    async fn fee(
        &self,
        message: &str,
        options: TransactionRequestContext,
    ) -> Result<TransactionFeeResult, TransactionError> {
        let body = BodyBuilder::new()
            .set("message", message)
            .maybe("commitment", commitment_value(options.commitment))
            .maybe("minContextSlot", decimal(options.min_context_slot))
            .build();
        let value = self.post("fee", body, SCOPE_INSPECT, false).await?;
        let fee_lamports = match value.get("feeLamports") {
            Some(Value::Null) => None,
            other => Some(decimal_u64(other, "feeLamports")?),
        };
        Ok(TransactionFeeResult {
            fee_lamports,
            context_slot: decimal_u64(value.get("contextSlot"), "contextSlot")?,
        })
    }

    async fn simulate(
        &self,
        transaction: &str,
        options: TransactionSimulationOptions,
    ) -> Result<TransactionSimulationResult, TransactionError> {
        let body = BodyBuilder::new()
            .set("transaction", transaction)
            .maybe("commitment", commitment_value(options.commitment))
            .maybe("minContextSlot", decimal(options.min_context_slot))
            .maybe(
                "accounts",
                options
                    .accounts
                    .as_ref()
                    .map(|addresses| serde_json::json!({ "addresses": addresses })),
            )
            .maybe("innerInstructions", options.inner_instructions)
            .maybe("replaceRecentBlockhash", options.replace_recent_blockhash)
            .build();
        let value = self.post("simulate", body, SCOPE_INSPECT, false).await?;
        let err = match value.get("err") {
            None | Some(Value::Null) => None,
            Some(other) => Some(other.clone()),
        };
        let logs = match value.get("logs") {
            None | Some(Value::Null) => None,
            Some(Value::Array(entries)) => Some(
                entries
                    .iter()
                    .map(|entry| entry.as_str().map(str::to_string))
                    .collect::<Option<Vec<_>>>()
                    .ok_or_else(|| {
                        TransactionError::InvalidResponse(
                            "'logs' entries must be strings in transaction response".to_string(),
                        )
                    })?,
            ),
            Some(_) => {
                return Err(TransactionError::InvalidResponse(
                    "'logs' must be an array in transaction response".to_string(),
                ))
            }
        };
        let accounts = match value.get("accounts") {
            None | Some(Value::Null) => None,
            Some(Value::Array(entries)) => Some(entries.clone()),
            Some(_) => {
                return Err(TransactionError::InvalidResponse(
                    "'accounts' must be an array in transaction response".to_string(),
                ))
            }
        };
        Ok(TransactionSimulationResult {
            context_slot: decimal_u64(value.get("contextSlot"), "contextSlot")?,
            err,
            logs,
            units_consumed: optional_decimal_u64(value.get("unitsConsumed"), "unitsConsumed")?,
            loaded_accounts_data_size: optional_decimal_u64(
                value.get("loadedAccountsDataSize"),
                "loadedAccountsDataSize",
            )?,
            accounts,
        })
    }

    async fn send(
        &self,
        transaction: &str,
        options: TransactionSendOptions,
    ) -> Result<TransactionSendResult, TransactionError> {
        let body = BodyBuilder::new()
            .set("transaction", transaction)
            .maybe("skipPreflight", options.skip_preflight)
            .maybe(
                "preflightCommitment",
                commitment_value(options.preflight_commitment),
            )
            .maybe("minContextSlot", decimal(options.min_context_slot))
            .build();
        // `send` is the one route where the refresh-replay must be gated on
        // the predispatch marker; it is never otherwise retried.
        let value = self.post("send", body, SCOPE_SEND, true).await?;
        let signature = value
            .get("signature")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                TransactionError::InvalidResponse(
                    "Missing 'signature' in transaction response".to_string(),
                )
            })?
            .to_string();
        Ok(TransactionSendResult { signature })
    }

    async fn signature_status(
        &self,
        signature: &str,
        options: SignatureStatusOptions,
    ) -> Result<Option<TransactionSignatureStatus>, TransactionError> {
        let body = BodyBuilder::new()
            .set("signature", signature)
            .maybe(
                "searchTransactionHistory",
                options.search_transaction_history,
            )
            .build();
        let value = self
            .post("signature-status", body, SCOPE_INSPECT, false)
            .await?;
        parse_signature_status(signature, value.get("status"))
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

        let body = BodyBuilder::new()
            .set("signatures", signatures)
            .maybe(
                "searchTransactionHistory",
                options.search_transaction_history,
            )
            .build();
        let value = self
            .post("signature-statuses", body, SCOPE_INSPECT, false)
            .await?;

        let statuses = value
            .get("statuses")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                TransactionError::InvalidResponse(
                    "signature-statuses: statuses must be an array".to_string(),
                )
            })?;

        // Callers read these positionally against their own signature list, so a length
        // mismatch would attribute one transaction's outcome to another.
        if statuses.len() != signatures.len() {
            return Err(TransactionError::InvalidResponse(format!(
                "signature-statuses: expected {} statuses, got {}",
                signatures.len(),
                statuses.len()
            )));
        }

        signatures
            .iter()
            .zip(statuses)
            .map(|(signature, status)| parse_signature_status(signature, Some(status)))
            .collect()
    }

    async fn block_height(
        &self,
        options: TransactionRequestContext,
    ) -> Result<u64, TransactionError> {
        let body = BodyBuilder::new()
            .maybe("commitment", commitment_value(options.commitment))
            .maybe("minContextSlot", decimal(options.min_context_slot))
            .build();
        let value = self
            .post("block-height", body, SCOPE_INSPECT, false)
            .await?;
        decimal_u64(value.get("blockHeight"), "blockHeight")
    }

    async fn transaction(
        &self,
        signature: &str,
        options: TransactionInspectOptions,
    ) -> Result<Option<ConfirmedTransaction>, TransactionError> {
        let body = BodyBuilder::new()
            .set("signature", signature)
            .maybe("commitment", commitment_value(options.commitment))
            .maybe(
                "maxSupportedTransactionVersion",
                options.max_supported_transaction_version,
            )
            .build();
        let value = self.post("get", body, SCOPE_INSPECT, false).await?;
        parse_confirmed_transaction(value.get("transaction"))
    }

    async fn signatures(
        &self,
        address: &str,
        options: SignaturePageOptions,
    ) -> Result<Vec<SignaturePageEntry>, TransactionError> {
        let body = BodyBuilder::new()
            .set("address", address)
            .maybe("limit", options.limit)
            .maybe("before", options.before)
            .maybe("until", options.until)
            .maybe("commitment", commitment_value(options.commitment))
            .build();
        let value = self.post("signatures", body, SCOPE_INSPECT, false).await?;
        value
            .get("signatures")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                TransactionError::InvalidResponse(
                    "signatures: signatures must be an array".to_string(),
                )
            })?
            .iter()
            .map(parse_signature_entry)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::AuthTokenRequest;
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use axum::routing::post;
    use axum::{Json, Router};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use tokio::net::TcpListener;

    #[derive(Default)]
    struct MockTokens {
        issued: AtomicUsize,
        invalidations: AtomicUsize,
        scopes: Mutex<Vec<Vec<String>>>,
    }

    #[async_trait]
    impl TokenSource for MockTokens {
        async fn token(
            &self,
            request: &AuthTokenRequest,
            _force_refresh: bool,
        ) -> Result<Option<String>, AreteError> {
            let n = self.issued.fetch_add(1, Ordering::SeqCst);
            self.scopes.lock().unwrap().push(request.scopes.clone());
            Ok(Some(format!("token-{n}")))
        }

        fn invalidate(&self, _request: &AuthTokenRequest) {
            self.invalidations.fetch_add(1, Ordering::SeqCst);
        }
    }

    async fn spawn(router: Router) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        format!("http://{addr}")
    }

    fn transport(base: &str) -> (HttpTransactionTransport, Arc<MockTokens>) {
        let tokens = Arc::new(MockTokens::default());
        (
            HttpTransactionTransport::new(format!("{base}/"), tokens.clone()),
            tokens,
        )
    }

    #[tokio::test]
    async fn latest_blockhash_serializes_request_and_parses_decimal_u64s() {
        let bodies = Arc::new(Mutex::new(Vec::<Value>::new()));
        let bodies_handler = bodies.clone();
        let router = Router::new().route(
            "/transactions/v1/latest-blockhash",
            post(move |Json(body): Json<Value>| {
                let bodies = bodies_handler.clone();
                async move {
                    bodies.lock().unwrap().push(body);
                    Json(serde_json::json!({
                        "blockhash": "blockhash",
                        "contextSlot": "43",
                        "lastValidBlockHeight": "99",
                    }))
                }
            }),
        );
        let base = spawn(router).await;
        let (transport, tokens) = transport(&base);

        let result = transport
            .latest_blockhash(TransactionRequestContext {
                commitment: Some(Commitment::Confirmed),
                min_context_slot: Some(42),
            })
            .await
            .unwrap();
        assert_eq!(
            result,
            LatestBlockhashResult {
                blockhash: "blockhash".to_string(),
                context_slot: 43,
                last_valid_block_height: 99,
            }
        );
        assert_eq!(
            bodies.lock().unwrap()[0],
            serde_json::json!({ "commitment": "confirmed", "minContextSlot": "42" })
        );
        assert_eq!(
            tokens.scopes.lock().unwrap()[0],
            vec!["transaction:inspect".to_string()]
        );
    }

    #[tokio::test]
    async fn fee_and_block_height_parse_nullable_and_plain_u64s() {
        let router = Router::new()
            .route(
                "/transactions/v1/fee",
                post(|| async {
                    Json(serde_json::json!({ "feeLamports": null, "contextSlot": "10" }))
                }),
            )
            .route(
                "/transactions/v1/block-height",
                post(|| async { Json(serde_json::json!({ "blockHeight": "123456" })) }),
            );
        let base = spawn(router).await;
        let (transport, _) = transport(&base);

        let fee = transport
            .fee("base64-message", TransactionRequestContext::default())
            .await
            .unwrap();
        assert_eq!(
            fee,
            TransactionFeeResult {
                fee_lamports: None,
                context_slot: 10,
            }
        );
        assert_eq!(
            transport
                .block_height(TransactionRequestContext::default())
                .await
                .unwrap(),
            123456
        );
    }

    #[tokio::test]
    async fn invalid_decimal_u64_fields_are_rejected() {
        let router = Router::new().route(
            "/transactions/v1/block-height",
            post(|| async { Json(serde_json::json!({ "blockHeight": 123456 })) }),
        );
        let base = spawn(router).await;
        let (transport, _) = transport(&base);
        let error = transport
            .block_height(TransactionRequestContext::default())
            .await
            .unwrap_err();
        assert!(
            matches!(error, TransactionError::InvalidResponse(ref message)
            if message.contains("Invalid decimal u64 field 'blockHeight'"))
        );
    }

    #[tokio::test]
    async fn simulate_serializes_accounts_wrapper_and_parses_result() {
        let bodies = Arc::new(Mutex::new(Vec::<Value>::new()));
        let bodies_handler = bodies.clone();
        let router = Router::new().route(
            "/transactions/v1/simulate",
            post(move |Json(body): Json<Value>| {
                let bodies = bodies_handler.clone();
                async move {
                    bodies.lock().unwrap().push(body);
                    Json(serde_json::json!({
                        "contextSlot": "55",
                        "err": null,
                        "logs": ["log-1", "log-2"],
                        "unitsConsumed": "700",
                        "loadedAccountsDataSize": "65536",
                        "accounts": [null, { "lamports": "1" }],
                    }))
                }
            }),
        );
        let base = spawn(router).await;
        let (transport, _) = transport(&base);

        let result = transport
            .simulate(
                "signed-base64",
                TransactionSimulationOptions {
                    commitment: Some(Commitment::Processed),
                    accounts: Some(vec!["addr1".to_string()]),
                    inner_instructions: Some(true),
                    replace_recent_blockhash: Some(false),
                    ..TransactionSimulationOptions::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(result.context_slot, 55);
        assert_eq!(result.err, None);
        assert_eq!(
            result.logs,
            Some(vec!["log-1".to_string(), "log-2".to_string()])
        );
        assert_eq!(result.units_consumed, Some(700));
        assert_eq!(result.loaded_accounts_data_size, Some(65536));
        assert_eq!(result.accounts.as_ref().map(Vec::len), Some(2));
        assert_eq!(
            bodies.lock().unwrap()[0],
            serde_json::json!({
                "transaction": "signed-base64",
                "commitment": "processed",
                "accounts": { "addresses": ["addr1"] },
                "innerInstructions": true,
                "replaceRecentBlockhash": false,
            })
        );
    }

    /// Contract §1: `loadedAccountsDataSize` is optional and a decimal string;
    /// absent and `null` are the same, `"0"` is neither, and anything else is
    /// the same invalid-response error as a bad `unitsConsumed`.
    #[tokio::test]
    async fn transaction_v1_simulate_preserves_loaded_accounts_data_size() {
        async fn simulate_with(
            field: Option<Value>,
        ) -> Result<TransactionSimulationResult, TransactionError> {
            let mut response = serde_json::json!({ "contextSlot": "9", "err": null });
            if let Some(field) = field {
                response
                    .as_object_mut()
                    .unwrap()
                    .insert("loadedAccountsDataSize".to_string(), field);
            }
            let router = Router::new().route(
                "/transactions/v1/simulate",
                post(move || {
                    let response = response.clone();
                    async move { Json(response) }
                }),
            );
            let base = spawn(router).await;
            let (transport, _) = transport(&base);
            transport
                .simulate("tx", TransactionSimulationOptions::default())
                .await
        }

        let absent = simulate_with(None).await.unwrap();
        assert_eq!(absent.loaded_accounts_data_size, None);
        let null = simulate_with(Some(Value::Null)).await.unwrap();
        assert_eq!(null.loaded_accounts_data_size, None);
        let zero = simulate_with(Some(serde_json::json!("0"))).await.unwrap();
        assert_eq!(zero.loaded_accounts_data_size, Some(0));
        let positive = simulate_with(Some(serde_json::json!("65536")))
            .await
            .unwrap();
        assert_eq!(positive.loaded_accounts_data_size, Some(65536));

        for malformed in [
            serde_json::json!(65536),
            serde_json::json!("-1"),
            serde_json::json!("64k"),
            serde_json::json!(true),
        ] {
            let error = simulate_with(Some(malformed.clone())).await.unwrap_err();
            assert!(
                matches!(&error, TransactionError::InvalidResponse(message)
                    if message.contains("Invalid decimal u64 field 'loadedAccountsDataSize'")),
                "expected {malformed} to be rejected, got {error:?}"
            );
        }
    }

    #[tokio::test]
    async fn send_uses_send_scope_and_omitted_options_are_not_serialized() {
        let bodies = Arc::new(Mutex::new(Vec::<Value>::new()));
        let bodies_handler = bodies.clone();
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_handler = calls.clone();
        let router = Router::new().route(
            "/transactions/v1/send",
            post(move |Json(body): Json<Value>| {
                let bodies = bodies_handler.clone();
                let calls = calls_handler.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    bodies.lock().unwrap().push(body);
                    Json(serde_json::json!({ "signature": "sig" }))
                }
            }),
        );
        let base = spawn(router).await;
        let (transport, tokens) = transport(&base);

        let result = transport
            .send(
                "signed-base64",
                TransactionSendOptions {
                    skip_preflight: Some(true),
                    ..TransactionSendOptions::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(result.signature, "sig");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            tokens.scopes.lock().unwrap()[0],
            vec!["transaction:send".to_string()]
        );
        assert_eq!(
            bodies.lock().unwrap()[0],
            serde_json::json!({ "transaction": "signed-base64", "skipPreflight": true })
        );
    }

    #[tokio::test]
    async fn send_is_not_replayed_without_predispatch_marker() {
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_handler = calls.clone();
        let router = Router::new().route(
            "/transactions/v1/send",
            post(move || {
                let calls = calls_handler.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    (
                        StatusCode::UNAUTHORIZED,
                        [("X-Error-Code", "token-expired")],
                        Json(serde_json::json!({
                            "code": "token_expired",
                            "message": "token expired",
                            "retryable": false,
                        })),
                    )
                }
            }),
        );
        let base = spawn(router).await;
        let (transport, tokens) = transport(&base);

        let error = transport
            .send("signed", TransactionSendOptions::default())
            .await
            .unwrap_err();
        assert!(matches!(error, TransactionError::Transport(ref e) if e.status == 401));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(tokens.invalidations.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn send_replays_once_when_marker_proves_no_dispatch() {
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_handler = calls.clone();
        let router = Router::new().route(
            "/transactions/v1/send",
            post(move || {
                let calls = calls_handler.clone();
                async move {
                    let n = calls.fetch_add(1, Ordering::SeqCst);
                    if n == 0 {
                        (
                            StatusCode::UNAUTHORIZED,
                            [
                                ("X-Error-Code", "token-expired"),
                                ("X-Arete-Upstream-Attempted", "false"),
                            ],
                            Json(serde_json::json!({ "message": "token expired" })),
                        )
                            .into_response()
                    } else {
                        Json(serde_json::json!({ "signature": "sig-2" })).into_response()
                    }
                }
            }),
        );
        let base = spawn(router).await;
        let (transport, tokens) = transport(&base);

        let result = transport
            .send("signed", TransactionSendOptions::default())
            .await
            .unwrap();
        assert_eq!(result.signature, "sig-2");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(tokens.invalidations.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn inspect_routes_replay_once_on_refresh_worthy_errors() {
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_handler = calls.clone();
        let router = Router::new().route(
            "/transactions/v1/block-height",
            post(move || {
                let calls = calls_handler.clone();
                async move {
                    let n = calls.fetch_add(1, Ordering::SeqCst);
                    if n == 0 {
                        (
                            StatusCode::UNAUTHORIZED,
                            [("X-Error-Code", "token-expired")],
                            Json(serde_json::json!({ "message": "token expired" })),
                        )
                            .into_response()
                    } else {
                        Json(serde_json::json!({ "blockHeight": "5" })).into_response()
                    }
                }
            }),
        );
        let base = spawn(router).await;
        let (transport, tokens) = transport(&base);

        assert_eq!(
            transport
                .block_height(TransactionRequestContext::default())
                .await
                .unwrap(),
            5
        );
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(tokens.invalidations.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn error_bodies_expose_stable_metadata_in_both_spellings() {
        let router = Router::new().route(
            "/transactions/v1/send",
            post(|| async {
                (
                    StatusCode::GATEWAY_TIMEOUT,
                    Json(serde_json::json!({
                        "code": "upstream_timeout",
                        "message": "Submission outcome is unknown",
                        "retryable": false,
                        "requestId": "req-1",
                        "submissionState": "unknown",
                        "signature": "local-sig",
                    })),
                )
            }),
        );
        let base = spawn(router).await;
        let (transport, _) = transport(&base);

        let error = transport
            .send("signed", TransactionSendOptions::default())
            .await
            .unwrap_err();
        let TransactionError::Transport(error) = error else {
            panic!("expected transport error");
        };
        assert_eq!(error.status, 504);
        assert_eq!(error.code, "upstream_timeout");
        assert_eq!(error.message, "Submission outcome is unknown");
        assert!(!error.retryable);
        assert_eq!(error.request_id.as_deref(), Some("req-1"));
        assert_eq!(error.submission_state, Some(SubmissionState::Unknown));
        assert_eq!(error.signature.as_deref(), Some("local-sig"));

        // Snake-case spellings are accepted too.
        let snake = TransactionTransportError::from_response(
            502,
            br#"{"code":"x","message":"m","retryable":true,"request_id":"req-2","submission_state":"not_submitted"}"#,
        );
        assert!(snake.retryable);
        assert_eq!(snake.request_id.as_deref(), Some("req-2"));
        assert_eq!(snake.submission_state, Some(SubmissionState::NotSubmitted));
    }

    #[test]
    fn non_json_error_bodies_synthesize_stable_fallbacks() {
        let error = TransactionTransportError::from_response(503, b"<html>bad gateway</html>");
        assert_eq!(error.code, "transaction_transport_error");
        assert_eq!(error.message, "Transaction request failed (503)");
        assert!(!error.retryable);
        assert_eq!(error.request_id, None);
        assert_eq!(error.submission_state, None);
    }

    #[tokio::test]
    async fn signature_status_maps_null_and_populated_statuses() {
        let router = Router::new().route(
            "/transactions/v1/signature-status",
            post(|Json(body): Json<Value>| async move {
                if body.get("searchTransactionHistory") == Some(&Value::Bool(true)) {
                    Json(serde_json::json!({
                        "status": {
                            "slot": "100",
                            "confirmationStatus": "confirmed",
                            "err": null,
                        }
                    }))
                } else {
                    Json(serde_json::json!({ "status": null }))
                }
            }),
        );
        let base = spawn(router).await;
        let (transport, _) = transport(&base);

        assert_eq!(
            transport
                .signature_status("sig", SignatureStatusOptions::default())
                .await
                .unwrap(),
            None
        );
        let status = transport
            .signature_status(
                "sig",
                SignatureStatusOptions {
                    search_transaction_history: Some(true),
                },
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(status.signature, "sig");
        assert_eq!(status.slot, Some(100));
        assert_eq!(status.confirmation_status, Some(Commitment::Confirmed));
        assert_eq!(status.err, None);
    }

    #[tokio::test]
    async fn signature_statuses_posts_every_signature_and_keeps_absent_slots() {
        let bodies = Arc::new(Mutex::new(Vec::<Value>::new()));
        let bodies_handler = bodies.clone();
        let router = Router::new().route(
            "/transactions/v1/signature-statuses",
            post(move |Json(body): Json<Value>| {
                let bodies = bodies_handler.clone();
                async move {
                    bodies.lock().unwrap().push(body);
                    Json(serde_json::json!({
                        "statuses": [
                            { "slot": "100", "confirmationStatus": "confirmed", "err": null },
                            null,
                            { "slot": "102", "confirmationStatus": "finalized", "err": null }
                        ]
                    }))
                }
            }),
        );
        let base = spawn(router).await;
        let (transport, _tokens) = transport(&base);

        let signatures = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let statuses = transport
            .signature_statuses(&signatures, SignatureStatusOptions::default())
            .await
            .unwrap();

        assert_eq!(
            bodies.lock().unwrap()[0]["signatures"],
            serde_json::json!(["a", "b", "c"])
        );
        assert_eq!(statuses.len(), 3);
        assert_eq!(statuses[0].as_ref().unwrap().signature, "a");
        assert_eq!(statuses[0].as_ref().unwrap().slot, Some(100));
        // The absent middle signature must hold its slot, not shift `c` onto `b`.
        assert!(statuses[1].is_none());
        assert_eq!(statuses[2].as_ref().unwrap().signature, "c");
        assert_eq!(
            statuses[2].as_ref().unwrap().confirmation_status,
            Some(Commitment::Finalized)
        );
    }

    #[tokio::test]
    async fn signature_statuses_rejects_a_length_mismatch() {
        let router = Router::new().route(
            "/transactions/v1/signature-statuses",
            post(|| async { Json(serde_json::json!({ "statuses": [null] })) }),
        );
        let base = spawn(router).await;
        let (transport, _tokens) = transport(&base);

        let signatures = vec!["a".to_string(), "b".to_string()];
        let error = transport
            .signature_statuses(&signatures, SignatureStatusOptions::default())
            .await
            .expect_err("a short array must not be accepted");
        assert!(
            matches!(&error, TransactionError::InvalidResponse(m) if m.contains("expected 2")),
            "unexpected error: {error:?}"
        );
    }

    #[tokio::test]
    async fn no_signatures_never_reaches_the_server() {
        let hits = Arc::new(AtomicUsize::new(0));
        let hits_handler = hits.clone();
        let router = Router::new().route(
            "/transactions/v1/signature-statuses",
            post(move || {
                let hits = hits_handler.clone();
                async move {
                    hits.fetch_add(1, Ordering::SeqCst);
                    Json(serde_json::json!({ "statuses": [] }))
                }
            }),
        );
        let base = spawn(router).await;
        let (transport, _tokens) = transport(&base);

        assert!(transport
            .signature_statuses(&[], SignatureStatusOptions::default())
            .await
            .unwrap()
            .is_empty());
        assert_eq!(hits.load(Ordering::SeqCst), 0);
    }

    /// Over the cap must fail here, without consuming a server admission slot. The server refuses
    /// an oversized batch too, but reaching it costs an authenticated round trip for a request the
    /// SDK already knows is invalid.
    #[tokio::test]
    async fn an_oversized_batch_is_refused_without_requesting() {
        let hits = Arc::new(AtomicUsize::new(0));
        let hits_handler = hits.clone();
        let router = Router::new().route(
            "/transactions/v1/signature-statuses",
            post(move || {
                let hits = hits_handler.clone();
                async move {
                    hits.fetch_add(1, Ordering::SeqCst);
                    Json(serde_json::json!({ "statuses": [] }))
                }
            }),
        );
        let base = spawn(router).await;
        let (transport, _tokens) = transport(&base);

        let signatures: Vec<String> = (0..=MAX_STATUS_SIGNATURES).map(|i| i.to_string()).collect();
        let error = transport
            .signature_statuses(&signatures, SignatureStatusOptions::default())
            .await
            .expect_err("over the limit");

        assert!(
            matches!(
                &error,
                TransactionError::Sdk(AreteError::InvalidConfig(m)) if m.contains("256-signature")
            ),
            "unexpected error: {error:?}"
        );
        assert_eq!(hits.load(Ordering::SeqCst), 0, "nothing was sent");
    }

    /// A transport written before this method existed must still compile, and must still get
    /// correct answers. This double implements every method the trait requires and deliberately
    /// does NOT override `signature_statuses`, so it only builds while the default body stands.
    struct SingleOnlyTransport;

    #[async_trait]
    impl TransactionTransport for SingleOnlyTransport {
        async fn latest_blockhash(
            &self,
            _options: TransactionRequestContext,
        ) -> Result<LatestBlockhashResult, TransactionError> {
            unimplemented!("not exercised")
        }

        async fn fee(
            &self,
            _message: &str,
            _options: TransactionRequestContext,
        ) -> Result<TransactionFeeResult, TransactionError> {
            unimplemented!("not exercised")
        }

        async fn simulate(
            &self,
            _transaction: &str,
            _options: TransactionSimulationOptions,
        ) -> Result<TransactionSimulationResult, TransactionError> {
            unimplemented!("not exercised")
        }

        async fn send(
            &self,
            _transaction: &str,
            _options: TransactionSendOptions,
        ) -> Result<TransactionSendResult, TransactionError> {
            unimplemented!("not exercised")
        }

        /// Answers for "b" only, so an absent signature is distinguishable from a present one.
        async fn signature_status(
            &self,
            signature: &str,
            _options: SignatureStatusOptions,
        ) -> Result<Option<TransactionSignatureStatus>, TransactionError> {
            Ok((signature == "b").then(|| TransactionSignatureStatus {
                signature: signature.to_string(),
                slot: Some(7),
                confirmation_status: Some(Commitment::Finalized),
                err: None,
            }))
        }

        async fn block_height(
            &self,
            _options: TransactionRequestContext,
        ) -> Result<u64, TransactionError> {
            unimplemented!("not exercised")
        }
    }

    #[tokio::test]
    async fn the_default_batch_falls_back_to_single_calls_in_order() {
        let signatures = ["a", "b", "c"].map(str::to_string).to_vec();
        let statuses = SingleOnlyTransport
            .signature_statuses(&signatures, SignatureStatusOptions::default())
            .await
            .expect("the default implementation answers");

        assert_eq!(statuses.len(), 3);
        assert!(statuses[0].is_none(), "a is absent and holds its slot");
        assert_eq!(
            statuses[1].as_ref().and_then(|status| status.slot),
            Some(7),
            "b resolves in its own position"
        );
        assert!(statuses[2].is_none(), "c is absent and holds its slot");
    }

    /// The cap is the trait's contract, not one implementation's, so the fallback enforces it too.
    #[tokio::test]
    async fn the_default_batch_refuses_an_oversized_request() {
        let signatures: Vec<String> = (0..=MAX_STATUS_SIGNATURES).map(|i| i.to_string()).collect();
        let error = SingleOnlyTransport
            .signature_statuses(&signatures, SignatureStatusOptions::default())
            .await
            .expect_err("over the limit");

        assert!(
            matches!(
                &error,
                TransactionError::Sdk(AreteError::InvalidConfig(m)) if m.contains("256-signature")
            ),
            "unexpected error: {error:?}"
        );
    }

    #[tokio::test]
    async fn transaction_maps_balances_and_an_unseen_signature() {
        let router = Router::new().route(
            "/transactions/v1/get",
            post(|Json(body): Json<Value>| async move {
                if body["signature"] == Value::String("unseen".to_string()) {
                    return Json(serde_json::json!({ "transaction": null }));
                }
                // A version is a number on the wire, never a decimal string.
                assert_eq!(body["maxSupportedTransactionVersion"], serde_json::json!(1));
                Json(serde_json::json!({ "transaction": {
                    "signature": "sig",
                    "slot": "319482771",
                    "blockTime": "1757222400",
                    "err": null,
                    "accounts": [
                        { "pubkey": "vault", "preBalance": "5000", "postBalance": "3995" },
                        { "pubkey": "winner", "preBalance": "10", "postBalance": "1010" }
                    ]
                }}))
            }),
        );
        let base = spawn(router).await;
        let (transport, _) = transport(&base);

        let confirmed = transport
            .transaction(
                "sig",
                TransactionInspectOptions {
                    commitment: Some(Commitment::Finalized),
                    max_supported_transaction_version: Some(1),
                },
            )
            .await
            .expect("the relay answers")
            .expect("the cluster has seen it");
        assert_eq!(confirmed.slot, 319_482_771);
        assert_eq!(confirmed.block_time, Some(1_757_222_400));
        assert_eq!(confirmed.err, None);
        assert_eq!(
            confirmed.accounts[1],
            TransactionAccountBalance {
                pubkey: "winner".to_string(),
                pre_balance: 10,
                post_balance: 1010,
            }
        );

        assert!(transport
            .transaction("unseen", TransactionInspectOptions::default())
            .await
            .expect("the relay answers")
            .is_none());
    }

    /// Only an explicit `null` means unseen. A body without the field is malformed, and reading it
    /// as unseen would tell a caller to keep waiting for a transaction that may already be final.
    #[tokio::test]
    async fn a_transaction_body_without_the_field_is_not_an_unseen_signature() {
        let router = Router::new().route(
            "/transactions/v1/get",
            post(|| async { Json(serde_json::json!({})) }),
        );
        let base = spawn(router).await;
        let (transport, _) = transport(&base);

        let error = transport
            .transaction("sig", TransactionInspectOptions::default())
            .await
            .expect_err("a missing field is not an answer");
        assert!(
            matches!(&error, TransactionError::InvalidResponse(m) if m.contains("transaction")),
            "unexpected error: {error:?}"
        );
    }

    /// No slower path exists to fall back to, so a transport without the route must say so.
    #[tokio::test]
    async fn the_default_transaction_reports_an_unsupported_transport() {
        let error = SingleOnlyTransport
            .transaction("sig", TransactionInspectOptions::default())
            .await
            .expect_err("no default implementation");
        assert!(
            matches!(
                &error,
                TransactionError::Sdk(AreteError::InvalidConfig(m)) if m.contains("/transactions/v1/get")
            ),
            "unexpected error: {error:?}"
        );
    }

    #[tokio::test]
    async fn signatures_forwards_the_cursor_and_maps_every_entry() {
        let router = Router::new().route(
            "/transactions/v1/signatures",
            post(|Json(body): Json<Value>| async move {
                assert_eq!(body["address"], serde_json::json!("addr"));
                assert_eq!(body["limit"], serde_json::json!(2));
                assert_eq!(body["before"], serde_json::json!("cursor"));
                Json(serde_json::json!({ "signatures": [
                    { "signature": "newer", "slot": "12", "blockTime": "1757222400", "err": null },
                    { "signature": "older", "slot": "11", "blockTime": null, "err": { "X": 1 } }
                ]}))
            }),
        );
        let base = spawn(router).await;
        let (transport, _) = transport(&base);

        let page = transport
            .signatures(
                "addr",
                SignaturePageOptions {
                    limit: Some(2),
                    before: Some("cursor".to_string()),
                    ..SignaturePageOptions::default()
                },
            )
            .await
            .expect("the relay answers");

        assert_eq!(
            page,
            vec![
                SignaturePageEntry {
                    signature: "newer".to_string(),
                    slot: 12,
                    block_time: Some(1_757_222_400),
                    err: None,
                },
                SignaturePageEntry {
                    signature: "older".to_string(),
                    slot: 11,
                    block_time: None,
                    err: Some(serde_json::json!({ "X": 1 })),
                },
            ]
        );
    }

    #[tokio::test]
    async fn the_default_signatures_reports_an_unsupported_transport() {
        let error = SingleOnlyTransport
            .signatures("addr", SignaturePageOptions::default())
            .await
            .expect_err("no default implementation");
        assert!(
            matches!(
                &error,
                TransactionError::Sdk(AreteError::InvalidConfig(m)) if m.contains("/transactions/v1/signatures")
            ),
            "unexpected error: {error:?}"
        );
    }
}
