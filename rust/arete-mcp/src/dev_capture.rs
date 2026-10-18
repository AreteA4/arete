//! Build an `arete.raw-capture/v1` file from a Solana JSON-RPC node, with no
//! Arete infrastructure involved.
//!
//! The node lists the programs' signatures newest first
//! (`getSignaturesForAddress`), the window's transactions are fetched one by
//! one (`getTransaction`) and converted to exactly the updates a geyser
//! subscription for the programs would have delivered, block order comes
//! from `getBlock`, and the starting state is read with
//! `getMultipleAccounts`. JSON-RPC serves no historical account data, so the
//! starting state is as of a slot after the window, and account writes
//! inside the window are not captured. `getProgramAccounts` is never called.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::future::Future;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering as AtomicOrdering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context};
use arete_solana_contracts::raw_capture::{
    CaptureAccount, CaptureAccountFlags, CaptureError, CaptureHeader, CaptureInstruction,
    CaptureOrdering, CaptureProducer, CaptureTopLevelInstruction, CaptureTransaction,
    CaptureUpdate, CaptureUpdateKind, CaptureWindow, CaptureWriter, StartingState, StateFidelity,
};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Approximate slots per second, for converting a `+duration` after a slot.
pub const SLOTS_PER_SECOND: f64 = 2.5;
/// `getSignaturesForAddress`'s largest page.
const SIGNATURE_PAGE: usize = 1000;
/// `getMultipleAccounts`'s largest batch.
const ACCOUNTS_BATCH: usize = 100;
const INITIAL_BACKOFF: Duration = Duration::from_millis(500);
const MAX_TRIES: u32 = 6;
const MAX_RETRY_WAIT: Duration = Duration::from_secs(30);

const MAINNET_GENESIS: &str = "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d";
const DEVNET_GENESIS: &str = "EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG";
const TESTNET_GENESIS: &str = "4uhcVJyU9pJkvQyS88uRDiswHXSCkY3zQawwpjk2NsNY";

const NO_ACCOUNT_WRITES: &str =
    "account writes inside the window are not captured: JSON-RPC history has transactions, not account states";

/// One end of a capture window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowBound {
    Slot(u64),
    /// Unix seconds.
    Time(i64),
}

/// Parse a capture window.
///
/// `from` is `<slot>`, an RFC 3339 time, or a duration before now
/// (`-10m`, `-2h`, `-90s`, `-1d`). `to` takes the same forms, or
/// `+<duration>` after `from`, or `None` for now. After a slot `from`, a
/// `+duration` converts at [`SLOTS_PER_SECOND`], which is approximate.
pub fn parse_window(
    from: &str,
    to: Option<&str>,
    now_unix: i64,
) -> anyhow::Result<(WindowBound, WindowBound)> {
    let from_text = from.trim();
    if from_text.starts_with('+') {
        bail!("--from cannot be relative to itself: use a slot, an RFC 3339 time or -<duration>");
    }
    let from = parse_bound(from_text, now_unix).context("--from")?;
    let to = match to.map(str::trim) {
        None => WindowBound::Time(now_unix),
        Some(text) => match text.strip_prefix('+') {
            Some(duration) => {
                let seconds = parse_duration(duration).context("--to")?;
                match from {
                    WindowBound::Slot(slot) => WindowBound::Slot(
                        slot.saturating_add((seconds as f64 * SLOTS_PER_SECOND).ceil() as u64),
                    ),
                    WindowBound::Time(time) => WindowBound::Time(time.saturating_add(seconds)),
                }
            }
            None => parse_bound(text, now_unix).context("--to")?,
        },
    };
    match (from, to) {
        (WindowBound::Slot(a), WindowBound::Slot(b)) if a > b => {
            bail!("the window starts at slot {a}, after its end at slot {b}")
        }
        (WindowBound::Time(a), WindowBound::Time(b)) if a > b => {
            bail!("the window starts after it ends ({a} > {b}, Unix seconds)")
        }
        _ => {}
    }
    Ok((from, to))
}

fn parse_bound(text: &str, now_unix: i64) -> anyhow::Result<WindowBound> {
    if let Some(duration) = text.strip_prefix('-') {
        return Ok(WindowBound::Time(now_unix - parse_duration(duration)?));
    }
    if !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()) {
        return Ok(WindowBound::Slot(
            text.parse().with_context(|| format!("slot {text:?}"))?,
        ));
    }
    let time = chrono::DateTime::parse_from_rfc3339(text).map_err(|_| {
        anyhow!(
            "{text:?} is not a slot, an RFC 3339 time (2026-01-02T15:04:05Z) or a duration (-10m)"
        )
    })?;
    Ok(WindowBound::Time(time.timestamp()))
}

/// `90s`, `10m`, `2h`, `1d` to seconds.
fn parse_duration(text: &str) -> anyhow::Result<i64> {
    let text = text.trim();
    let split = text
        .find(|c: char| !c.is_ascii_digit())
        .ok_or_else(|| anyhow!("duration {text:?} needs a unit: s, m, h or d"))?;
    let (number, unit) = text.split_at(split);
    let number: i64 = number
        .parse()
        .map_err(|_| anyhow!("duration {text:?} must start with a whole number"))?;
    let scale = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 3600,
        "d" => 86_400,
        _ => bail!("duration {text:?}: unit must be s, m, h or d"),
    };
    Ok(number.saturating_mul(scale))
}

/// What to capture and how.
#[derive(Debug, Clone)]
pub struct RpcCaptureOptions {
    pub rpc_url: String,
    /// Program ids (base58).
    pub programs: Vec<String>,
    pub from: WindowBound,
    pub to: WindowBound,
    /// Accounts always included in the starting state, whoever owns them.
    pub accounts: Vec<String>,
    /// Add the accounts the window's transactions reference that the
    /// programs own to the starting state.
    pub touched_state: bool,
    pub max_transactions: usize,
    pub max_signature_pages: usize,
    pub max_state_accounts: usize,
    /// Order each slot's transactions by their block index (`getBlock`).
    pub block_order: bool,
    pub requests_per_second: f64,
    pub output: PathBuf,
    /// The producer version written to the header.
    pub tool_version: String,
}

impl RpcCaptureOptions {
    pub fn new(
        rpc_url: impl Into<String>,
        programs: Vec<String>,
        from: WindowBound,
        to: WindowBound,
        output: impl Into<PathBuf>,
    ) -> Self {
        Self {
            rpc_url: rpc_url.into(),
            programs,
            from,
            to,
            accounts: Vec::new(),
            touched_state: true,
            max_transactions: 500,
            max_signature_pages: 20,
            max_state_accounts: 500,
            block_order: true,
            requests_per_second: 8.0,
            output: output.into(),
            tool_version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }
}

/// A Solana JSON-RPC client: `call` returns the response's `result`.
pub trait JsonRpc {
    fn call(
        &self,
        method: &str,
        params: Value,
    ) -> impl Future<Output = anyhow::Result<Value>> + Send;
}

/// [`JsonRpc`] over HTTP, paced to a request rate, retrying rate limits and
/// server errors with exponential backoff.
pub struct HttpRpc {
    client: reqwest::Client,
    url: String,
    min_interval: Duration,
    next_at: tokio::sync::Mutex<Instant>,
    next_id: AtomicU64,
}

impl HttpRpc {
    pub fn new(url: impl Into<String>, requests_per_second: f64) -> anyhow::Result<Self> {
        let rate = if requests_per_second.is_finite() && requests_per_second > 0.0 {
            requests_per_second
        } else {
            bail!("requests per second must be positive");
        };
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .context("building the HTTP client")?;
        Ok(Self {
            client,
            url: url.into(),
            min_interval: Duration::from_secs_f64(1.0 / rate),
            next_at: tokio::sync::Mutex::new(Instant::now()),
            next_id: AtomicU64::new(1),
        })
    }

    async fn pace(&self) {
        let mut next = self.next_at.lock().await;
        let now = Instant::now();
        if *next > now {
            tokio::time::sleep(*next - now).await;
        }
        *next = Instant::now() + self.min_interval;
    }

    /// One attempt: `Ok(Ok(result))`, `Ok(Err((error, retry_after)))` when
    /// retryable, `Err` when not.
    async fn attempt(
        &self,
        method: &str,
        params: &Value,
    ) -> anyhow::Result<Result<Value, (anyhow::Error, Option<Duration>)>> {
        let id = self.next_id.fetch_add(1, AtomicOrdering::Relaxed);
        let body = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        // reqwest errors carry the URL, which can hold an API key.
        let response = match self.client.post(&self.url).json(&body).send().await {
            Ok(response) => response,
            Err(error) => return Ok(Err((anyhow!(error.without_url()), None))),
        };
        let status = response.status();
        let retry_after = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse::<u64>().ok())
            .map(Duration::from_secs);
        if status.as_u16() == 429 || status.is_server_error() {
            return Ok(Err((anyhow!("HTTP {status}"), retry_after)));
        }
        if !status.is_success() {
            bail!("HTTP {status}");
        }
        let value: Value = match response.json().await {
            Ok(value) => value,
            Err(error) => return Ok(Err((anyhow!(error.without_url()), None))),
        };
        if let Some(error) = value.get("error").filter(|error| !error.is_null()) {
            let code = error.get("code").and_then(Value::as_i64).unwrap_or(0);
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let lower = message.to_ascii_lowercase();
            let rate_limited = code == -32005
                || code == -32429
                || lower.contains("rate limit")
                || lower.contains("too many requests");
            let error = anyhow!("RPC error {code}: {message}");
            return if rate_limited {
                Ok(Err((error, retry_after)))
            } else {
                Err(error)
            };
        }
        Ok(Ok(value.get("result").cloned().unwrap_or(Value::Null)))
    }
}

impl JsonRpc for HttpRpc {
    async fn call(&self, method: &str, params: Value) -> anyhow::Result<Value> {
        let mut delay = INITIAL_BACKOFF;
        for attempt in 1..=MAX_TRIES {
            self.pace().await;
            match self
                .attempt(method, &params)
                .await
                .with_context(|| method.to_string())?
            {
                Ok(result) => return Ok(result),
                Err((error, retry_after)) => {
                    if attempt == MAX_TRIES {
                        return Err(
                            error.context(format!("{method}: gave up after {MAX_TRIES} tries"))
                        );
                    }
                    tokio::time::sleep(retry_after.unwrap_or(delay).min(MAX_RETRY_WAIT)).await;
                    delay *= 2;
                }
            }
        }
        unreachable!("the retry loop returns on its last try")
    }
}

/// What [`capture_from_rpc`] wrote.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureSummary {
    pub path: String,
    pub header: CaptureHeader,
    pub transactions: usize,
    pub skipped_failed: usize,
    pub state_accounts: usize,
    /// Explicit `accounts` the node does not have.
    pub missing_accounts: Vec<String>,
    /// JSON-RPC requests made.
    pub requests: usize,
}

/// A file path, absolute or relative to `base`, with an existing parent
/// directory and no `..` components. Shared by JSON and raw-capture tools.
pub fn resolve_file_path(output: &str, base: &std::path::Path) -> anyhow::Result<PathBuf> {
    let path = std::path::Path::new(output.trim());
    if path.as_os_str().is_empty() {
        bail!("`output` is empty");
    }
    if path
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        bail!("`output` may not contain `..`: {output}");
    }
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    };
    if path.is_dir() {
        bail!("`output` is a directory: {}", path.display());
    }
    match path.parent() {
        Some(parent) if parent.is_dir() => Ok(path),
        _ => bail!(
            "the directory of `output` does not exist: {}",
            path.display()
        ),
    }
}

/// A raw-capture output path: the shared file checks plus the `.arcap` suffix.
pub fn resolve_capture_output(output: &str, base: &std::path::Path) -> anyhow::Result<PathBuf> {
    let path = resolve_file_path(output, base)?;
    if path.extension().and_then(|extension| extension.to_str())
        != Some(arete_solana_contracts::raw_capture::RAW_CAPTURE_EXTENSION)
    {
        bail!(
            "`output` must name a .{} file: {output}",
            arete_solana_contracts::raw_capture::RAW_CAPTURE_EXTENSION
        );
    }
    Ok(path)
}

/// An RPC URL without credentials: scheme and host (and port) only. Many
/// providers put the API key in the path or the query.
pub fn rpc_source_label(url: &str) -> String {
    match reqwest::Url::parse(url) {
        Ok(parsed) => match parsed.host_str() {
            Some(host) => match parsed.port() {
                Some(port) => format!("{}://{host}:{port}", parsed.scheme()),
                None => format!("{}://{host}", parsed.scheme()),
            },
            None => "unknown".into(),
        },
        Err(_) => "unknown".into(),
    }
}

fn cluster_name(genesis_hash: &str) -> &'static str {
    match genesis_hash {
        MAINNET_GENESIS => "mainnet-beta",
        DEVNET_GENESIS => "devnet",
        TESTNET_GENESIS => "testnet",
        _ => "unknown",
    }
}

/// Counts the requests made through it.
struct Counted<'a, R> {
    rpc: &'a R,
    requests: AtomicUsize,
}

impl<R: JsonRpc> Counted<'_, R> {
    async fn call(&self, method: &str, params: Value) -> anyhow::Result<Value> {
        self.requests.fetch_add(1, AtomicOrdering::Relaxed);
        self.rpc.call(method, params).await
    }
}

/// A signature the node listed inside the window.
#[derive(Debug, Clone)]
struct ListedSignature {
    signature: String,
    slot: u64,
    block_time: Option<i64>,
    failed: bool,
}

enum Position {
    Older,
    Inside,
    Newer,
}

fn position(slot: u64, block_time: Option<i64>, from: WindowBound, to: WindowBound) -> Position {
    // An entry without a block time cannot be placed by time: keep it.
    let older = match from {
        WindowBound::Slot(first) => slot < first,
        WindowBound::Time(first) => block_time.is_some_and(|time| time < first),
    };
    if older {
        return Position::Older;
    }
    let newer = match to {
        WindowBound::Slot(last) => slot > last,
        WindowBound::Time(last) => block_time.is_some_and(|time| time > last),
    };
    if newer {
        Position::Newer
    } else {
        Position::Inside
    }
}

/// The program's signatures inside the window, newest first, and the pages
/// read.
async fn list_signatures<R: JsonRpc>(
    rpc: &Counted<'_, R>,
    program: &str,
    options: &RpcCaptureOptions,
) -> anyhow::Result<(Vec<ListedSignature>, usize)> {
    let mut before: Option<String> = None;
    let mut listed = Vec::new();
    let mut seen = 0usize;
    for page in 1..=options.max_signature_pages {
        let mut config = json!({"limit": SIGNATURE_PAGE, "commitment": "confirmed"});
        if let Some(before) = &before {
            config["before"] = json!(before);
        }
        let result = rpc
            .call("getSignaturesForAddress", json!([program, config]))
            .await
            .with_context(|| format!("listing signatures for {program}"))?;
        let entries = result
            .as_array()
            .ok_or_else(|| anyhow!("getSignaturesForAddress returned no list for {program}"))?;
        seen += entries.len();
        for entry in entries {
            let signature = entry
                .get("signature")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("a signature entry without a signature"))?;
            let slot = entry
                .get("slot")
                .and_then(Value::as_u64)
                .ok_or_else(|| anyhow!("signature {signature} has no slot"))?;
            let block_time = entry.get("blockTime").and_then(Value::as_i64);
            match position(slot, block_time, options.from, options.to) {
                Position::Older => return Ok((listed, page)),
                Position::Newer => {}
                Position::Inside => listed.push(ListedSignature {
                    signature: signature.to_string(),
                    slot,
                    block_time,
                    failed: entry.get("err").is_some_and(|err| !err.is_null()),
                }),
            }
        }
        if entries.len() < SIGNATURE_PAGE {
            // The program's history ends here.
            return Ok((listed, page));
        }
        before = entries
            .last()
            .and_then(|entry| entry.get("signature"))
            .and_then(Value::as_str)
            .map(str::to_string);
    }
    bail!(
        "{program}: read {} pages ({seen} signatures) without reaching the window's start. \
         The node lists signatures newest first, so an old window of a busy program is expensive: \
         pick a more recent window or raise the signature page limit",
        options.max_signature_pages
    )
}

// --- getTransaction ("json" encoding) ---

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RpcTransaction {
    slot: u64,
    #[serde(default)]
    block_time: Option<i64>,
    #[serde(default)]
    version: Option<Value>,
    /// The transaction's position in its block, which nodes return with
    /// `getTransaction`. Older nodes leave it out.
    #[serde(default)]
    transaction_index: Option<u64>,
    #[serde(default)]
    meta: Option<RpcMeta>,
    transaction: RpcTransactionBody,
}

#[derive(Deserialize)]
struct RpcTransactionBody {
    signatures: Vec<String>,
    message: RpcMessage,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RpcMessage {
    account_keys: Vec<String>,
    header: RpcMessageHeader,
    instructions: Vec<RpcInstruction>,
    /// A v1 message's inline configuration.
    #[serde(default)]
    transaction_config: Option<RpcTransactionConfig>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RpcTransactionConfig {
    #[serde(default)]
    priority_fee: Option<u64>,
    #[serde(default)]
    compute_unit_limit: Option<u32>,
    #[serde(default)]
    loaded_accounts_data_size_limit: Option<u32>,
    #[serde(default)]
    heap_size: Option<u32>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RpcMessageHeader {
    num_required_signatures: usize,
    num_readonly_signed_accounts: usize,
    num_readonly_unsigned_accounts: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RpcMeta {
    #[serde(default)]
    err: Option<Value>,
    #[serde(default)]
    log_messages: Option<Vec<String>>,
    #[serde(default)]
    inner_instructions: Option<Vec<RpcInnerInstructions>>,
    #[serde(default)]
    loaded_addresses: Option<RpcLoadedAddresses>,
}

#[derive(Deserialize)]
struct RpcInnerInstructions {
    index: usize,
    instructions: Vec<RpcInstruction>,
}

#[derive(Default, Deserialize)]
struct RpcLoadedAddresses {
    #[serde(default)]
    writable: Vec<String>,
    #[serde(default)]
    readonly: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RpcInstruction {
    program_id_index: usize,
    #[serde(default)]
    accounts: Vec<usize>,
    data: String,
    #[serde(default)]
    stack_height: Option<u32>,
}

/// A converted transaction.
struct Converted {
    transaction: CaptureTransaction,
    slot: u64,
    /// The position in the block the node reported, if it did.
    block_index: Option<u64>,
    block_time: Option<i64>,
    failed: bool,
    /// Every key the message references, static then loaded.
    account_keys: Vec<String>,
    /// Every program the transaction invokes, top-level or inner.
    invoked: Vec<String>,
}

/// Convert a `getTransaction` result (`json` encoding) to the update a
/// geyser subscription for `programs` delivers.
fn convert_transaction(value: Value, programs: &HashSet<&str>) -> anyhow::Result<Converted> {
    let rpc: RpcTransaction =
        serde_json::from_value(value).context("reading a getTransaction result")?;
    let signature = rpc
        .transaction
        .signatures
        .first()
        .cloned()
        .ok_or_else(|| anyhow!("a transaction without signatures"))?;
    let meta = rpc.meta;
    let failed = meta
        .as_ref()
        .is_some_and(|meta| meta.err.as_ref().is_some_and(|err| !err.is_null()));
    let (log_messages, inner_groups, loaded) = match meta {
        Some(meta) => (
            meta.log_messages.unwrap_or_default(),
            meta.inner_instructions.unwrap_or_default(),
            meta.loaded_addresses.unwrap_or_default(),
        ),
        None => (Vec::new(), Vec::new(), RpcLoadedAddresses::default()),
    };
    let message = rpc.transaction.message;
    let static_keys = message.account_keys;
    let account_keys: Vec<String> = static_keys
        .iter()
        .chain(&loaded.writable)
        .chain(&loaded.readonly)
        .cloned()
        .collect();
    let key = |index: usize| -> anyhow::Result<String> {
        account_keys.get(index).cloned().ok_or_else(|| {
            anyhow!(
                "transaction {signature}: account index {index} is out of range ({} keys)",
                account_keys.len()
            )
        })
    };
    let keys = |indexes: &[usize]| -> anyhow::Result<Vec<String>> {
        indexes.iter().map(|index| key(*index)).collect()
    };
    let data = |text: &str| -> anyhow::Result<Vec<u8>> {
        bs58::decode(text)
            .into_vec()
            .with_context(|| format!("transaction {signature}: instruction data is not base58"))
    };

    let mut top_level_instructions = Vec::with_capacity(message.instructions.len());
    let mut instructions = Vec::new();
    for (index, instruction) in message.instructions.iter().enumerate() {
        let program_id = key(instruction.program_id_index)?;
        let bytes = data(&instruction.data)?;
        let accounts = keys(&instruction.accounts)?;
        if programs.contains(program_id.as_str()) {
            instructions.push(CaptureInstruction {
                program_id: program_id.clone(),
                data: bytes.clone(),
                accounts: accounts.clone(),
                is_inner: false,
                top_level_index: Some(index),
                parent_index: None,
                ix_path: Some(index.to_string()),
            });
        }
        top_level_instructions.push(CaptureTopLevelInstruction {
            program_id,
            data: bytes,
            accounts,
        });
    }

    let mut logged = logged_inner_paths(&log_messages);
    for group in &inner_groups {
        let paths = instruction_paths(
            group.index,
            group
                .instructions
                .iter()
                .map(|instruction| instruction.stack_height),
        );
        for (instruction, (path, used_fallback)) in group.instructions.iter().zip(paths) {
            let program_id = key(instruction.program_id_index)?;
            // Pop for every instruction so the queue stays in step.
            let logged_path = logged
                .get_mut(&(group.index, program_id.clone()))
                .and_then(VecDeque::pop_front);
            if !programs.contains(program_id.as_str()) {
                continue;
            }
            let path = match (used_fallback, logged_path) {
                (true, Some(logged_path)) => logged_path,
                _ => path,
            };
            instructions.push(CaptureInstruction {
                program_id,
                data: data(&instruction.data)?,
                accounts: keys(&instruction.accounts)?,
                is_inner: true,
                top_level_index: Some(group.index),
                parent_index: Some(group.index),
                ix_path: Some(path),
            });
        }
    }

    let mut invoked: Vec<String> = Vec::new();
    for index in message
        .instructions
        .iter()
        .chain(inner_groups.iter().flat_map(|group| &group.instructions))
        .map(|instruction| instruction.program_id_index)
    {
        let program_id = key(index)?;
        if !invoked.contains(&program_id) {
            invoked.push(program_id);
        }
    }

    let account_flags = CaptureAccountFlags::from_message(
        message.header.num_required_signatures,
        message.header.num_readonly_signed_accounts,
        message.header.num_readonly_unsigned_accounts,
        &static_keys,
        &loaded.writable,
    );
    // As a geyser stream reports it: the version, and for a v1 message its
    // inline configuration (the priority fee as a decimal string).
    let transaction_metadata = rpc
        .version
        .filter(|version| !version.is_null())
        .map(|version| {
            let mut metadata = json!({ "version": version });
            if version == json!(1) {
                let mut config = serde_json::Map::new();
                if let Some(c) = &message.transaction_config {
                    if let Some(fee) = c.priority_fee {
                        config.insert("priority_fee_lamports".into(), json!(fee.to_string()));
                    }
                    if let Some(limit) = c.compute_unit_limit {
                        config.insert("compute_unit_limit".into(), json!(limit));
                    }
                    if let Some(limit) = c.loaded_accounts_data_size_limit {
                        config.insert("loaded_accounts_data_size_limit".into(), json!(limit));
                    }
                    if let Some(size) = c.heap_size {
                        config.insert("heap_size".into(), json!(size));
                    }
                }
                metadata["config"] = Value::Object(config);
            }
            metadata
        });
    Ok(Converted {
        transaction: CaptureTransaction {
            account_flags,
            transaction_metadata,
            signature,
            error: None,
            txn_index: 0,
            top_level_instructions,
            log_messages,
            instructions,
        },
        slot: rpc.slot,
        block_index: rpc.transaction_index,
        block_time: rpc.block_time,
        failed,
        account_keys,
        invoked,
    })
}

/// Instruction-tree paths for one top-level instruction's inner
/// instructions, from their stack heights. The bool is true when a height
/// was missing or inconsistent and the instruction was placed directly
/// under the top-level instruction.
fn instruction_paths(
    top_level_index: usize,
    stack_heights: impl IntoIterator<Item = Option<u32>>,
) -> Vec<(String, bool)> {
    let root = vec![top_level_index];
    let mut active: Vec<Vec<usize>> = vec![root.clone()];
    let mut next_child: BTreeMap<Vec<usize>, usize> = BTreeMap::new();
    let mut out = Vec::new();
    for height in stack_heights {
        let parent_depth = height
            .filter(|height| *height >= 2)
            .map(|height| height as usize - 1);
        let resolved = parent_depth
            .filter(|depth| *depth <= active.len())
            .and_then(|depth| active.get(depth - 1).cloned().map(|parent| (parent, depth)));
        let used_fallback = resolved.is_none();
        let (parent, depth) = resolved.unwrap_or((root.clone(), 1));
        let ordinal = next_child.entry(parent.clone()).or_insert(0);
        let mut path = parent;
        path.push(*ordinal);
        *ordinal += 1;
        active.truncate(depth);
        active.push(path.clone());
        out.push((join_path(&path), used_fallback));
    }
    out
}

fn join_path(path: &[usize]) -> String {
    path.iter()
        .map(usize::to_string)
        .collect::<Vec<_>>()
        .join(".")
}

/// Inner-instruction paths read from the call tree in the logs, keyed by
/// (top-level index, program id), in invocation order.
fn logged_inner_paths(logs: &[String]) -> HashMap<(usize, String), VecDeque<String>> {
    let mut out: HashMap<(usize, String), VecDeque<String>> = HashMap::new();
    let mut stack: Vec<(String, Vec<usize>)> = Vec::new();
    let mut next_top = 0usize;
    let mut top: Option<usize> = None;
    let mut next_child: HashMap<Vec<usize>, usize> = HashMap::new();
    for line in logs {
        let Some(rest) = line.strip_prefix("Program ") else {
            continue;
        };
        let Some((program, event)) = rest.split_once(' ') else {
            continue;
        };
        if let Some(depth) = event
            .strip_prefix("invoke [")
            .and_then(|depth| depth.strip_suffix(']'))
            .and_then(|depth| depth.parse::<usize>().ok())
        {
            if depth == 1 {
                let index = next_top;
                next_top += 1;
                top = Some(index);
                stack.clear();
                stack.push((program.to_string(), vec![index]));
                continue;
            }
            let Some(top) = top else { continue };
            if depth < 2 || stack.len() < depth - 1 {
                continue;
            }
            stack.truncate(depth - 1);
            let mut path = stack[depth - 2].1.clone();
            let ordinal = next_child.entry(path.clone()).or_insert(0);
            path.push(*ordinal);
            *ordinal += 1;
            out.entry((top, program.to_string()))
                .or_default()
                .push_back(join_path(&path));
            stack.push((program.to_string(), path));
        } else if (event == "success" || event.starts_with("failed"))
            && stack.last().is_some_and(|(top, _)| top == program)
        {
            stack.pop();
        }
    }
    out
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

fn rfc3339_millis(millis: i64) -> String {
    chrono::DateTime::from_timestamp_millis(millis)
        .map(|time| time.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .unwrap_or_default()
}

fn check_pubkey(kind: &str, key: &str) -> anyhow::Result<()> {
    match bs58::decode(key).into_vec() {
        Ok(bytes) if bytes.len() == 32 => Ok(()),
        _ => bail!("{kind} {key:?} is not a base58 public key"),
    }
}

fn dedupe(keys: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    keys.iter()
        .filter(|key| seen.insert(key.as_str()))
        .cloned()
        .collect()
}

/// Capture `options.programs`' transactions in the window from a JSON-RPC
/// node and write them, with a starting state, to `options.output`.
pub async fn capture_from_rpc<R: JsonRpc>(
    rpc: &R,
    options: &RpcCaptureOptions,
    progress: &mut (dyn FnMut(&str) + Send),
) -> anyhow::Result<CaptureSummary> {
    if options.programs.is_empty() {
        bail!("name at least one program to capture");
    }
    let programs = dedupe(&options.programs);
    for program in &programs {
        check_pubkey("program", program)?;
    }
    let explicit = dedupe(&options.accounts);
    for account in &explicit {
        check_pubkey("account", account)?;
    }
    let program_set: HashSet<&str> = programs.iter().map(String::as_str).collect();
    let rpc = Counted {
        rpc,
        requests: AtomicUsize::new(0),
    };
    let started_ms = now_millis();
    let now = started_ms / 1000;

    // 1. Cluster, and the window must have ended.
    let genesis_hash = rpc
        .call("getGenesisHash", json!([]))
        .await?
        .as_str()
        .ok_or_else(|| anyhow!("getGenesisHash returned no hash"))?
        .to_string();
    let cluster = cluster_name(&genesis_hash);
    match options.to {
        WindowBound::Time(to) if to > now => bail!(
            "the window must have ended: it ends {}s from now. A JSON-RPC capture reads history; it cannot record live",
            to - now
        ),
        WindowBound::Slot(to) => {
            let tip = rpc
                .call("getSlot", json!([{"commitment": "confirmed"}]))
                .await?
                .as_u64()
                .ok_or_else(|| anyhow!("getSlot returned no slot"))?;
            if to > tip {
                bail!(
                    "the window must have ended: it ends at slot {to}, after the node's confirmed slot {tip}. \
                     A JSON-RPC capture reads history; it cannot record live"
                );
            }
        }
        WindowBound::Time(_) => {}
    }
    progress(&format!("cluster: {cluster}"));
    let mut limits = vec![NO_ACCOUNT_WRITES.to_string()];

    // 2. Signatures.
    let mut failed: HashSet<String> = HashSet::new();
    let mut selected: HashMap<String, ListedSignature> = HashMap::new();
    // Each slot's signatures oldest first, as the node listed them: one
    // program's after another's, since each program is listed on its own.
    let mut slot_listing: BTreeMap<u64, Vec<String>> = BTreeMap::new();
    // The program whose listing placed each signature.
    let mut listed_by: HashMap<String, usize> = HashMap::new();
    let mut without_time = 0usize;
    for (program_index, program) in programs.iter().enumerate() {
        let (listed, pages) = list_signatures(&rpc, program, options).await?;
        progress(&format!(
            "signatures: {} in window for {program} ({pages} page{})",
            listed.len(),
            if pages == 1 { "" } else { "s" }
        ));
        for entry in listed.into_iter().rev() {
            if entry.failed {
                failed.insert(entry.signature);
                continue;
            }
            if selected.contains_key(&entry.signature) {
                continue;
            }
            if entry.block_time.is_none() {
                without_time += 1;
            }
            slot_listing
                .entry(entry.slot)
                .or_default()
                .push(entry.signature.clone());
            listed_by.insert(entry.signature.clone(), program_index);
            selected.insert(entry.signature.clone(), entry);
        }
    }
    let mut skipped_failed = failed.len();
    let time_bounded =
        matches!(options.from, WindowBound::Time(_)) || matches!(options.to, WindowBound::Time(_));
    if without_time > 0 && time_bounded {
        limits.push(format!(
            "{without_time} signatures had no block time and were kept without checking the window's time bounds"
        ));
    }
    if selected.len() > options.max_transactions {
        bail!(
            "the window holds {} transactions, over the limit of {}: narrow the window or raise --max-transactions",
            selected.len(),
            options.max_transactions
        );
    }
    progress(&format!(
        "signatures: {} in window, {} failed skipped",
        selected.len(),
        skipped_failed
    ));

    // 3. Transactions.
    let total = selected.len();
    let mut converted: Vec<Converted> = Vec::with_capacity(total);
    let mut no_block_time = 0usize;
    let mut fetched = 0usize;
    for signatures in slot_listing.values() {
        for signature in signatures {
            let result = rpc
                .call(
                    "getTransaction",
                    json!([signature, {
                        "encoding": "json",
                        "maxSupportedTransactionVersion": 1,
                        "commitment": "confirmed",
                    }]),
                )
                .await
                .with_context(|| format!("fetching transaction {signature}"))?;
            fetched += 1;
            if fetched.is_multiple_of(25) || fetched == total {
                progress(&format!("transactions {fetched}/{total}"));
            }
            if result.is_null() {
                bail!("the node has no transaction {signature}: it may have pruned it; use an archive node or a more recent window");
            }
            let mut transaction = convert_transaction(result, &program_set)
                .with_context(|| format!("converting transaction {signature}"))?;
            if transaction.failed {
                skipped_failed += 1;
                continue;
            }
            if transaction.block_time.is_none() {
                transaction.block_time = selected.get(signature).and_then(|entry| entry.block_time);
                if transaction.block_time.is_none() {
                    no_block_time += 1;
                }
            }
            converted.push(transaction);
        }
    }
    if no_block_time > 0 {
        limits.push(format!(
            "{no_block_time} transactions have no block time: their received_at is 0"
        ));
    }

    // 4. Ordering.
    let mut ordering = if options.block_order {
        CaptureOrdering::Block
    } else {
        CaptureOrdering::Approximate
    };
    let slots: Vec<u64> = {
        let mut slots: Vec<u64> = converted.iter().map(|t| t.slot).collect();
        slots.sort_unstable();
        slots.dedup();
        slots
    };
    let mut block_index: HashMap<String, u64> = HashMap::new();
    let mut fallback_slots: HashSet<u64> = HashSet::new();
    // Nodes report each transaction's block position with it; read a block
    // only for a slot where one is missing.
    let reported: HashSet<u64> = slots
        .iter()
        .copied()
        .filter(|slot| {
            converted
                .iter()
                .filter(|t| t.slot == *slot)
                .all(|t| t.block_index.is_some())
        })
        .collect();
    for transaction in &converted {
        if let Some(index) = transaction.block_index {
            if reported.contains(&transaction.slot) {
                block_index.insert(transaction.transaction.signature.clone(), index);
            }
        }
    }
    if options.block_order {
        for slot in slots.iter().filter(|slot| !reported.contains(slot)) {
            let block = rpc
                .call(
                    "getBlock",
                    json!([slot, {
                        "encoding": "json",
                        "transactionDetails": "signatures",
                        "rewards": false,
                        "maxSupportedTransactionVersion": 1,
                        "commitment": "confirmed",
                    }]),
                )
                .await;
            let signatures = block
                .ok()
                .and_then(|block| block.get("signatures").and_then(Value::as_array).cloned());
            match signatures {
                Some(signatures) => {
                    for (index, signature) in signatures.iter().enumerate() {
                        if let Some(signature) = signature.as_str() {
                            block_index.insert(signature.to_string(), index as u64);
                        }
                    }
                }
                None => {
                    fallback_slots.insert(*slot);
                }
            }
        }
        let read = slots.len() - reported.len();
        if read > 0 {
            progress(&format!("blocks: {} read", read - fallback_slots.len()));
        }
    }
    for transaction in &converted {
        if options.block_order && !block_index.contains_key(&transaction.transaction.signature) {
            fallback_slots.insert(transaction.slot);
        }
    }
    if !fallback_slots.is_empty() {
        ordering = CaptureOrdering::Approximate;
        limits.push(format!(
            "{} slots could not be read with getBlock: their transactions are in the order the node listed them, which is usually but not provably block order",
            fallback_slots.len()
        ));
    }
    for transaction in &mut converted {
        let signature = &transaction.transaction.signature;
        let block = if options.block_order && !fallback_slots.contains(&transaction.slot) {
            block_index.get(signature).copied()
        } else {
            None
        };
        // Without the block, the position among the slot's signatures as
        // the node listed them, oldest first.
        transaction.transaction.txn_index = block.unwrap_or_else(|| {
            slot_listing
                .get(&transaction.slot)
                .and_then(|listing| listing.iter().position(|s| s == signature))
                .unwrap_or(0) as u64
        });
    }
    converted.sort_by_key(|t| (t.slot, t.transaction.txn_index));
    // Without a block, a slot's order is each program's listing in turn:
    // the node lists one address at a time, so nothing says how two
    // programs' transactions interleave in the block.
    let grouped = slot_listing
        .iter()
        .filter(|(slot, _)| !options.block_order || fallback_slots.contains(slot))
        .filter(|(_, listing)| {
            listing
                .iter()
                .filter_map(|signature| listed_by.get(signature))
                .collect::<HashSet<_>>()
                .len()
                > 1
        })
        .count();
    if grouped > 0 {
        limits.push(format!(
            "{grouped} slots without block order hold transactions of several programs: each program's transactions are together, in program order, not interleaved as in the block"
        ));
    }

    // 5. Starting state.
    let explicit_set: HashSet<&str> = explicit.iter().map(String::as_str).collect();
    let touched: Vec<String> = if options.touched_state {
        let invoked: HashSet<&str> = converted
            .iter()
            .flat_map(|t| t.invoked.iter().map(String::as_str))
            .collect();
        let mut seen = HashSet::new();
        converted
            .iter()
            .flat_map(|t| t.account_keys.iter())
            .filter(|key| {
                !explicit_set.contains(key.as_str())
                    && !invoked.contains(key.as_str())
                    && seen.insert(key.as_str())
            })
            .cloned()
            .collect()
    } else {
        Vec::new()
    };
    let min_context_slot = converted.iter().map(|t| t.slot).max().or(match options.to {
        WindowBound::Slot(slot) => Some(slot),
        WindowBound::Time(_) => None,
    });
    let cap = options.max_state_accounts;
    let mut state: Vec<(CaptureAccount, u64)> = Vec::new();
    let mut missing_accounts = Vec::new();
    let mut truncated = false;
    let mut context_slots: Vec<u64> = Vec::new();
    for (chunk, is_explicit) in explicit
        .chunks(ACCOUNTS_BATCH)
        .map(|chunk| (chunk, true))
        .chain(touched.chunks(ACCOUNTS_BATCH).map(|chunk| (chunk, false)))
    {
        if state.len() >= cap {
            truncated = true;
            break;
        }
        let mut config = json!({"encoding": "base64", "commitment": "confirmed"});
        if let Some(slot) = min_context_slot {
            config["minContextSlot"] = json!(slot);
        }
        let result = rpc
            .call("getMultipleAccounts", json!([chunk, config]))
            .await
            .context("reading the starting state")?;
        let context_slot = result
            .pointer("/context/slot")
            .and_then(Value::as_u64)
            .ok_or_else(|| anyhow!("getMultipleAccounts returned no context slot"))?;
        context_slots.push(context_slot);
        let values = result
            .get("value")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow!("getMultipleAccounts returned no accounts"))?;
        for (pubkey, value) in chunk.iter().zip(values) {
            if value.is_null() {
                if is_explicit {
                    missing_accounts.push(pubkey.clone());
                }
                continue;
            }
            let owner = value
                .get("owner")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("account {pubkey} has no owner"))?;
            if !is_explicit && !program_set.contains(owner) {
                continue;
            }
            if state.len() >= cap {
                truncated = true;
                continue;
            }
            let data = value
                .pointer("/data/0")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("account {pubkey} has no base64 data"))?;
            let data = base64::engine::general_purpose::STANDARD
                .decode(data)
                .with_context(|| format!("account {pubkey}: data is not base64"))?;
            state.push((
                CaptureAccount {
                    pubkey: pubkey.clone(),
                    owner: owner.to_string(),
                    data,
                    lamports: value.get("lamports").and_then(Value::as_u64).unwrap_or(0),
                    write_version: 0,
                    txn_signature: None,
                },
                context_slot,
            ));
        }
    }
    if truncated {
        limits.push(format!(
            "the starting state is capped at {cap} accounts (explicit accounts first); more may have qualified"
        ));
    }
    if !missing_accounts.is_empty() {
        limits.push(format!(
            "{} requested accounts do not exist on the node",
            missing_accounts.len()
        ));
    }
    progress(&format!("starting state: {} accounts", state.len()));
    let selection = match (options.touched_state, !explicit.is_empty()) {
        (true, true) => "touched+explicit",
        (true, false) => "touched",
        (false, true) => "explicit",
        (false, false) => "none",
    };
    let state_slot = context_slots.iter().copied().max();
    // Starting state is delivered before the window, in slot order: at the
    // slot before the window's first transaction (or the window's first
    // slot), stamped with that transaction's block time. A capture is
    // slot-ordered, and a stack (or a client's sequence guard) would read
    // the window's updates as stale after rows stamped with the later slot
    // the accounts were read at. The read slot stays in the header.
    let delivery_slot = converted
        .first()
        .map(|t| t.slot.saturating_sub(1))
        .or(match options.from {
            WindowBound::Slot(slot) => Some(slot.saturating_sub(1)),
            WindowBound::Time(_) => None,
        })
        .or(state_slot)
        .unwrap_or(0);
    let state_received_at = converted
        .first()
        .and_then(|t| t.block_time)
        .map(|time| time.saturating_mul(1000))
        .unwrap_or_else(now_millis);
    let starting_state = StartingState {
        fidelity: if state.is_empty() {
            StateFidelity::None
        } else {
            StateFidelity::Current
        },
        slot: state_slot,
        selection: selection.to_string(),
        note: state_slot.filter(|_| !state.is_empty()).map(|slot| {
            format!(
                "JSON-RPC serves no historical account data: these accounts are as of slot {slot}, after the window; they are delivered at slot {delivery_slot}, before the window's first update"
            )
        }),
    };

    // 6. Header.
    let mut window = CaptureWindow::default();
    match options.from {
        WindowBound::Slot(slot) => window.from_slot = Some(slot),
        WindowBound::Time(time) => window.from_time = Some(time),
    }
    match options.to {
        WindowBound::Slot(slot) => window.to_slot = Some(slot),
        WindowBound::Time(time) => window.to_time = Some(time),
    }
    let mut header = CaptureHeader::new(
        programs.clone(),
        cluster,
        CaptureProducer {
            tool: "a4".into(),
            version: options.tool_version.clone(),
            method: "rpc".into(),
            source: Some(rpc_source_label(&options.rpc_url)),
            created_at: rfc3339_millis(started_ms),
        },
    );
    header.genesis_hash = Some(genesis_hash);
    header.window = Some(window);
    header.starting_state = starting_state;
    header.ordering = ordering;
    header.limits = limits;

    // 7. Write.
    if let Some(parent) = options
        .output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let mut writer = CaptureWriter::create(&options.output, header)
        .with_context(|| format!("creating {}", options.output.display()))?;
    let state: Vec<(CaptureAccount, u64)> = state
        .into_iter()
        .map(|(account, _read_slot)| (account, delivery_slot))
        .collect();
    let transactions = converted.len();
    let state_accounts = state.len();
    if let Err(error) = write_records(&mut writer, state, converted, state_received_at) {
        writer.abandon();
        return Err(anyhow!(error).context(format!("writing {}", options.output.display())));
    }
    let header = writer
        .finish()
        .with_context(|| format!("writing {}", options.output.display()))?;
    progress(&format!(
        "wrote {} ({transactions} transactions, {state_accounts} state accounts)",
        options.output.display()
    ));
    Ok(CaptureSummary {
        path: options.output.display().to_string(),
        header,
        transactions,
        skipped_failed,
        state_accounts,
        missing_accounts,
        requests: rpc.requests.load(AtomicOrdering::Relaxed),
    })
}

fn write_records(
    writer: &mut CaptureWriter,
    state: Vec<(CaptureAccount, u64)>,
    transactions: Vec<Converted>,
    state_received_at: i64,
) -> Result<(), CaptureError> {
    let mut seq = 0u64;
    for (account, slot) in state {
        writer.state(CaptureUpdate {
            seq,
            slot,
            received_at: state_received_at,
            kind: CaptureUpdateKind::Account(account),
        })?;
        seq += 1;
    }
    for transaction in transactions {
        writer.update(CaptureUpdate {
            seq,
            slot: transaction.slot,
            received_at: transaction.block_time.unwrap_or(0).saturating_mul(1000),
            kind: CaptureUpdateKind::Transaction(transaction.transaction),
        })?;
        seq += 1;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use arete_solana_contracts::raw_capture::read_capture;

    use super::*;

    #[test]
    fn capture_outputs_are_arcap_files_without_parent_steps() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        assert_eq!(
            resolve_capture_output("window.arcap", base).unwrap(),
            base.join("window.arcap")
        );
        let absolute = base.join("abs.arcap");
        assert_eq!(
            resolve_capture_output(absolute.to_str().unwrap(), std::path::Path::new("/nowhere"))
                .unwrap(),
            absolute
        );
        for bad in [
            "",
            "window.json",
            "window",
            "../window.arcap",
            "sub/../../window.arcap",
            "missing/window.arcap",
        ] {
            assert!(resolve_capture_output(bad, base).is_err(), "{bad}");
        }
        std::fs::create_dir(base.join("dir.arcap")).unwrap();
        assert!(resolve_capture_output("dir.arcap", base).is_err());
    }

    #[test]
    fn json_paths_share_file_checks_without_capture_suffixes() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        for name in ["scenario.json", "repro.json"] {
            assert_eq!(resolve_file_path(name, base).unwrap(), base.join(name));
            let absolute = base.join(name);
            assert_eq!(
                resolve_file_path(absolute.to_str().unwrap(), base).unwrap(),
                absolute
            );
            assert!(resolve_capture_output(name, base).is_err());
        }
        std::fs::create_dir(base.join("directory.json")).unwrap();
        for bad in [
            "",
            "../scenario.json",
            "sub/../../repro.json",
            "missing/repro.json",
            "directory.json",
        ] {
            assert!(resolve_file_path(bad, base).is_err(), "{bad}");
        }
    }

    const PROGRAM: &str = "whirLbMiicVdio4qvUfM5KS9gd5StM6dENbZyRy7Jpc";
    const TOKEN: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
    const COMPUTE_BUDGET: &str = "ComputeBudget111111111111111111111111111111";
    const SYSTEM: &str = "11111111111111111111111111111111";
    const PAYER: &str = "9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM";
    const POOL: &str = "HJPjoWUrhoZzkNfRpHuieeFk9WcZWjwy6PBjZ81ngndJ";
    const VAULT: &str = "3YQm7ujtXWJU2e9jhp2QGHpnn1ShXn12QjvzMvDgabpX";
    const MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
    const MISSING: &str = "7xKXtg2CW87d97TXJSDpbD5jBkheTqA83TZRuJosgAsU";

    fn signature(n: u8) -> String {
        bs58::encode([n; 64]).into_string()
    }

    type Handler = Box<dyn Fn(&str, &Value) -> anyhow::Result<Value> + Send + Sync>;

    struct MockRpc {
        handler: Handler,
        calls: Mutex<Vec<(String, Value)>>,
    }

    impl MockRpc {
        fn new(
            handler: impl Fn(&str, &Value) -> anyhow::Result<Value> + Send + Sync + 'static,
        ) -> Self {
            Self {
                handler: Box::new(handler),
                calls: Mutex::new(Vec::new()),
            }
        }

        fn methods(&self) -> Vec<String> {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .map(|(method, _)| method.clone())
                .collect()
        }
    }

    impl JsonRpc for MockRpc {
        async fn call(&self, method: &str, params: Value) -> anyhow::Result<Value> {
            self.calls
                .lock()
                .unwrap()
                .push((method.to_string(), params.clone()));
            (self.handler)(method, &params)
        }
    }

    /// A v0 swap: compute budget, then the program, which calls the token
    /// program, then itself twice (a nested self-CPI).
    fn swap(sig: &str, slot: u64, heights: [Option<u32>; 3], failed: bool) -> Value {
        // Static: payer, pool, compute budget, program, token. Loaded:
        // vault (writable), mint (read-only).
        json!({
            "slot": slot,
            "blockTime": 1_760_000_000 + slot as i64,
            "version": 0,
            "meta": {
                "err": if failed { json!({"InstructionError": [1, {"Custom": 6001}]}) } else { Value::Null },
                "logMessages": [
                    format!("Program {COMPUTE_BUDGET} invoke [1]"),
                    format!("Program {COMPUTE_BUDGET} success"),
                    format!("Program {PROGRAM} invoke [1]"),
                    "Program log: Instruction: Swap",
                    format!("Program {TOKEN} invoke [2]"),
                    format!("Program {TOKEN} success"),
                    format!("Program {PROGRAM} invoke [2]"),
                    format!("Program {PROGRAM} invoke [3]"),
                    format!("Program {PROGRAM} success"),
                    format!("Program {PROGRAM} success"),
                    format!("Program {PROGRAM} consumed 41000 of 200000 compute units"),
                    format!("Program {PROGRAM} success"),
                ],
                "innerInstructions": [{
                    "index": 1,
                    "instructions": [
                        {"programIdIndex": 4, "accounts": [5, 0], "data": bs58::encode([3u8, 1, 2]).into_string(), "stackHeight": heights[0]},
                        {"programIdIndex": 3, "accounts": [1], "data": bs58::encode([0xe4u8, 0x45]).into_string(), "stackHeight": heights[1]},
                        {"programIdIndex": 3, "accounts": [1, 6], "data": bs58::encode([0x1du8]).into_string(), "stackHeight": heights[2]},
                    ],
                }],
                "loadedAddresses": {"writable": [VAULT], "readonly": [MINT]},
            },
            "transaction": {
                "signatures": [sig],
                "message": {
                    "accountKeys": [PAYER, POOL, COMPUTE_BUDGET, PROGRAM, TOKEN],
                    "header": {"numRequiredSignatures": 1, "numReadonlySignedAccounts": 0, "numReadonlyUnsignedAccounts": 3},
                    "instructions": [
                        {"programIdIndex": 2, "accounts": [], "data": bs58::encode([2u8, 0x40, 0x0d, 0x03, 0]).into_string(), "stackHeight": null},
                        {"programIdIndex": 3, "accounts": [0, 1, 5, 6, 4], "data": bs58::encode([0xf8u8, 0xc6, 9]).into_string(), "stackHeight": null},
                    ],
                    "addressTableLookups": [{"accountKey": SYSTEM, "writableIndexes": [0], "readonlyIndexes": [1]}],
                },
            },
        })
    }

    fn programs() -> HashSet<&'static str> {
        [PROGRAM].into_iter().collect()
    }

    #[test]
    fn parse_window_forms() {
        let now = 1_760_000_000;
        assert_eq!(
            parse_window("300000000", Some("300000100"), now).unwrap(),
            (
                WindowBound::Slot(300_000_000),
                WindowBound::Slot(300_000_100)
            )
        );
        assert_eq!(
            parse_window("-10m", None, now).unwrap(),
            (WindowBound::Time(now - 600), WindowBound::Time(now))
        );
        assert_eq!(
            parse_window("-2h", Some("-90s"), now).unwrap(),
            (WindowBound::Time(now - 7200), WindowBound::Time(now - 90))
        );
        assert_eq!(
            parse_window("2025-10-09T08:00:00Z", Some("+5m"), now).unwrap(),
            (
                WindowBound::Time(1_759_996_800),
                WindowBound::Time(1_759_997_100)
            )
        );
        assert_eq!(
            parse_window(
                "2025-10-09T10:00:00+02:00",
                Some("2025-10-09T08:01:00Z"),
                now
            )
            .unwrap(),
            (
                WindowBound::Time(1_759_996_800),
                WindowBound::Time(1_759_996_860)
            )
        );
        // +duration after a slot converts at 2.5 slots per second.
        assert_eq!(
            parse_window("300000000", Some("+1m"), now).unwrap(),
            (
                WindowBound::Slot(300_000_000),
                WindowBound::Slot(300_000_150)
            )
        );
        assert!(parse_window("300000100", Some("300000000"), now).is_err());
        assert!(parse_window("-5m", Some("-10m"), now).is_err());
        assert!(parse_window("yesterday", None, now).is_err());
        assert!(parse_window("-10", None, now).is_err());
        assert!(parse_window("+10m", None, now).is_err());
    }

    #[test]
    fn rpc_source_label_drops_credentials() {
        assert_eq!(
            rpc_source_label("https://mainnet.helius-rpc.com/?api-key=abc"),
            "https://mainnet.helius-rpc.com"
        );
        assert_eq!(
            rpc_source_label("https://x.quiknode.pro/SECRET/"),
            "https://x.quiknode.pro"
        );
        assert_eq!(
            rpc_source_label("https://user:pass@rpc.example.com:8899/v1/key"),
            "https://rpc.example.com:8899"
        );
        assert_eq!(rpc_source_label("not a url"), "unknown");
    }

    #[test]
    fn converts_a_v0_transaction_like_geyser() {
        let sig = signature(1);
        let converted = convert_transaction(
            swap(&sig, 1003, [Some(2), Some(2), Some(3)], false),
            &programs(),
        )
        .unwrap();
        let tx = converted.transaction;
        assert_eq!(tx.signature, sig);
        assert_eq!(tx.transaction_metadata, Some(json!({"version": 0})));
        assert_eq!(
            converted.account_keys,
            vec![PAYER, POOL, COMPUTE_BUDGET, PROGRAM, TOKEN, VAULT, MINT]
        );
        assert_eq!(tx.account_flags.signers, vec![PAYER]);
        assert_eq!(tx.account_flags.writable, vec![PAYER, POOL, VAULT]);
        // Every top-level instruction.
        let top: Vec<&str> = tx
            .top_level_instructions
            .iter()
            .map(|i| i.program_id.as_str())
            .collect();
        assert_eq!(top, vec![COMPUTE_BUDGET, PROGRAM]);
        // Only the program's instructions, with stable paths: the token
        // transfer took ordinal 0.
        let paths: Vec<(&str, bool, Option<&str>)> = tx
            .instructions
            .iter()
            .map(|i| (i.program_id.as_str(), i.is_inner, i.ix_path.as_deref()))
            .collect();
        assert_eq!(
            paths,
            vec![
                (PROGRAM, false, Some("1")),
                (PROGRAM, true, Some("1.1")),
                (PROGRAM, true, Some("1.1.0")),
            ]
        );
        let first = &tx.instructions[0];
        assert_eq!(first.data, vec![0xf8, 0xc6, 9]);
        assert_eq!(first.accounts, vec![PAYER, POOL, VAULT, MINT, TOKEN]);
        assert_eq!(first.top_level_index, Some(1));
        assert_eq!(first.parent_index, None);
        let inner = &tx.instructions[2];
        assert_eq!(inner.accounts, vec![POOL, MINT]);
        assert_eq!(inner.top_level_index, Some(1));
        assert_eq!(inner.parent_index, Some(1));
        assert_eq!(tx.log_messages.len(), 12);
    }

    #[test]
    fn legacy_version_is_kept_as_a_string() {
        let mut value = swap(&signature(1), 5, [Some(2), Some(2), Some(3)], false);
        value["version"] = json!("legacy");
        let converted = convert_transaction(value, &programs()).unwrap();
        assert_eq!(
            converted.transaction.transaction_metadata,
            Some(json!({"version": "legacy"}))
        );
    }

    #[test]
    fn a_v1_transaction_carries_its_inline_config_and_block_index() {
        let mut value = swap(
            &signature(9),
            300_000_010,
            [Some(2), Some(3), Some(2)],
            false,
        );
        value["version"] = json!(1);
        value["transactionIndex"] = json!(417);
        value["transaction"]["message"]["transactionConfig"] = json!({
            "computeUnitLimit": 275000, "heapSize": null,
            "loadedAccountsDataSizeLimit": 10485760, "priorityFee": 1512
        });
        let converted = convert_transaction(value, &programs()).unwrap();
        assert_eq!(converted.block_index, Some(417));
        assert_eq!(
            converted.transaction.transaction_metadata,
            Some(json!({ "version": 1, "config": {
                "priority_fee_lamports": "1512",
                "compute_unit_limit": 275000,
                "loaded_accounts_data_size_limit": 10485760
            }}))
        );
    }

    #[test]
    fn missing_stack_heights_fall_back_to_the_logs() {
        let converted = convert_transaction(
            swap(&signature(1), 5, [None, None, None], false),
            &programs(),
        )
        .unwrap();
        let paths: Vec<Option<&str>> = converted
            .transaction
            .instructions
            .iter()
            .map(|i| i.ix_path.as_deref())
            .collect();
        assert_eq!(paths, vec![Some("1"), Some("1.1"), Some("1.1.0")]);
    }

    #[test]
    fn stack_height_paths() {
        let paths = instruction_paths(2, [Some(2), Some(3), Some(3), Some(4), Some(2), None]);
        let expected = [
            ("2.0", false),
            ("2.0.0", false),
            ("2.0.1", false),
            ("2.0.1.0", false),
            ("2.1", false),
            ("2.2", true),
        ];
        assert_eq!(
            paths,
            expected
                .iter()
                .map(|(p, f)| (p.to_string(), *f))
                .collect::<Vec<_>>()
        );
    }

    #[derive(Clone, Copy)]
    struct Scenario {
        block_1005_fails: bool,
        pages_of_newer: usize,
    }

    /// Program history, newest first: sig 6 (slot 1010, after the window),
    /// sig 5 (1005, failed), sig 4 (1005), sig 3 and 2 (1003), sig 1 (999,
    /// before the window).
    fn mock(scenario: Scenario) -> MockRpc {
        MockRpc::new(move |method, params| {
            let entry = |n: u8, slot: u64, failed: bool| {
                json!({
                    "signature": signature(n),
                    "slot": slot,
                    "blockTime": 1_760_000_000 + slot as i64,
                    "err": if failed { json!({"InstructionError": [0, "Custom"]}) } else { Value::Null },
                    "memo": null,
                    "confirmationStatus": "finalized",
                })
            };
            match method {
                "getGenesisHash" => Ok(json!(MAINNET_GENESIS)),
                "getSlot" => Ok(json!(5000)),
                "getSignaturesForAddress" => {
                    if scenario.pages_of_newer > 0 {
                        return Ok(Value::Array(
                            (0..SIGNATURE_PAGE).map(|_| entry(9, 2000, false)).collect(),
                        ));
                    }
                    assert_eq!(params[0], PROGRAM);
                    Ok(json!([
                        entry(6, 1010, false),
                        entry(5, 1005, true),
                        entry(4, 1005, false),
                        entry(3, 1003, false),
                        entry(2, 1003, false),
                        entry(1, 999, false),
                    ]))
                }
                "getTransaction" => {
                    let sig = params[0].as_str().unwrap();
                    let slot = if sig == signature(4) { 1005 } else { 1003 };
                    Ok(swap(sig, slot, [Some(2), Some(2), Some(3)], false))
                }
                "getBlock" => match params[0].as_u64().unwrap() {
                    1003 => Ok(
                        json!({"signatures": [signature(7), signature(3), signature(8), signature(2)]}),
                    ),
                    1005 if scenario.block_1005_fails => Err(anyhow!(
                        "RPC error -32007: Slot 1005 was skipped, or missing due to ledger jump"
                    )),
                    1005 => Ok(json!({"signatures": [signature(4)]})),
                    other => panic!("unexpected block {other}"),
                },
                "getMultipleAccounts" => {
                    assert_eq!(params[1]["minContextSlot"], 1005);
                    let account = |owner: &str, data: &[u8]| {
                        json!({
                            "data": [base64::engine::general_purpose::STANDARD.encode(data), "base64"],
                            "owner": owner,
                            "lamports": 2_039_280,
                            "executable": false,
                            "rentEpoch": 18_446_744_073_709_551_615u64,
                            "space": data.len(),
                        })
                    };
                    let values: Vec<Value> = params[0]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|key| match key.as_str().unwrap() {
                            POOL => account(PROGRAM, &[1, 2, 3]),
                            VAULT => account(TOKEN, &[4; 165]),
                            MINT => account(TOKEN, &[5; 82]),
                            PAYER => account(SYSTEM, &[]),
                            MISSING => Value::Null,
                            other => panic!("unexpected account {other}"),
                        })
                        .collect();
                    Ok(json!({"context": {"slot": 1020, "apiVersion": "2.3.0"}, "value": values}))
                }
                other => panic!("unexpected method {other}"),
            }
        })
    }

    fn options(dir: &std::path::Path) -> RpcCaptureOptions {
        let mut options = RpcCaptureOptions::new(
            "https://mainnet.helius-rpc.com/?api-key=abc",
            vec![PROGRAM.into()],
            WindowBound::Slot(1000),
            WindowBound::Slot(1006),
            dir.join("captures").join("window.arcap"),
        );
        options.tool_version = "0.0.0-test".into();
        options
    }

    async fn run(rpc: &MockRpc, options: &RpcCaptureOptions) -> anyhow::Result<CaptureSummary> {
        let mut lines = Vec::new();
        let result = capture_from_rpc(rpc, options, &mut |line| lines.push(line.to_string())).await;
        assert!(result.is_err() || !lines.is_empty());
        result
    }

    fn signatures_in_order(capture: &arete_solana_contracts::raw_capture::Capture) -> Vec<String> {
        capture
            .records
            .iter()
            .filter(|r| !r.is_state())
            .map(|r| r.update().signature().unwrap().to_string())
            .collect()
    }

    #[tokio::test]
    async fn captures_a_window_in_block_order() {
        let dir = tempfile::tempdir().unwrap();
        let rpc = mock(Scenario {
            block_1005_fails: false,
            pages_of_newer: 0,
        });
        let mut options = options(dir.path());
        options.accounts = vec![MINT.into(), MISSING.into()];
        let summary = run(&rpc, &options).await.unwrap();

        assert_eq!(summary.transactions, 3);
        assert_eq!(summary.skipped_failed, 1);
        assert_eq!(summary.missing_accounts, vec![MISSING]);
        // Explicit mint (token-owned, kept) and the program-owned pool; not
        // the token vault or the payer.
        assert_eq!(summary.state_accounts, 2);
        assert_eq!(summary.requests, rpc.calls.lock().unwrap().len());
        assert!(!rpc.methods().iter().any(|m| m == "getProgramAccounts"));

        let header = &summary.header;
        assert_eq!(header.cluster, "mainnet-beta");
        assert_eq!(header.genesis_hash.as_deref(), Some(MAINNET_GENESIS));
        assert_eq!(header.ordering, CaptureOrdering::Block);
        assert_eq!(header.producer.method, "rpc");
        assert_eq!(header.producer.version, "0.0.0-test");
        assert_eq!(
            header.producer.source.as_deref(),
            Some("https://mainnet.helius-rpc.com")
        );
        assert_eq!(header.window.as_ref().unwrap().from_slot, Some(1000));
        assert_eq!(header.window.as_ref().unwrap().to_slot, Some(1006));
        assert_eq!(header.starting_state.fidelity, StateFidelity::Current);
        assert_eq!(header.starting_state.slot, Some(1020));
        assert_eq!(header.starting_state.selection, "touched+explicit");
        assert!(header
            .starting_state
            .note
            .as_deref()
            .unwrap()
            .contains("slot 1020"));
        assert_eq!(header.limits[0], NO_ACCOUNT_WRITES);

        let capture = read_capture(&options.output).unwrap();
        assert_eq!(&capture.header, header);
        assert_eq!(capture.header.counts.state, 2);
        assert_eq!(capture.header.counts.transactions, 3);
        // Block order: in slot 1003 sig 3 (index 1) precedes sig 2 (index 3).
        assert_eq!(
            signatures_in_order(&capture),
            vec![signature(3), signature(2), signature(4)]
        );
        let seqs: Vec<u64> = capture.updates().map(|u| u.seq).collect();
        assert_eq!(seqs, (0..5).collect::<Vec<_>>());
        let state: Vec<&str> = capture
            .records
            .iter()
            .filter(|r| r.is_state())
            .map(|r| match &r.update().kind {
                CaptureUpdateKind::Account(account) => account.pubkey.as_str(),
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(state, vec![MINT, POOL]);
        let first_tx = capture
            .records
            .iter()
            .find(|r| !r.is_state())
            .unwrap()
            .update();
        assert_eq!(first_tx.received_at, (1_760_000_000 + 1003) * 1000);
        match &first_tx.kind {
            CaptureUpdateKind::Transaction(tx) => assert_eq!(tx.txn_index, 1),
            _ => unreachable!(),
        }
    }

    #[tokio::test]
    async fn a_missing_block_falls_back_to_approximate_order() {
        let dir = tempfile::tempdir().unwrap();
        let rpc = mock(Scenario {
            block_1005_fails: true,
            pages_of_newer: 0,
        });
        let summary = run(&rpc, &options(dir.path())).await.unwrap();
        assert_eq!(summary.header.ordering, CaptureOrdering::Approximate);
        assert!(summary.header.limits.iter().any(|l| l.contains("getBlock")));
        let capture = read_capture(&options(dir.path()).output).unwrap();
        assert_eq!(
            signatures_in_order(&capture),
            vec![signature(3), signature(2), signature(4)]
        );
    }

    #[tokio::test]
    async fn without_block_order_the_listing_order_is_used() {
        let dir = tempfile::tempdir().unwrap();
        let rpc = mock(Scenario {
            block_1005_fails: false,
            pages_of_newer: 0,
        });
        let mut options = options(dir.path());
        options.block_order = false;
        options.touched_state = false;
        let summary = run(&rpc, &options).await.unwrap();
        assert_eq!(summary.header.ordering, CaptureOrdering::Approximate);
        assert!(!rpc
            .methods()
            .iter()
            .any(|m| m == "getBlock" || m == "getMultipleAccounts"));
        assert_eq!(summary.header.starting_state.fidelity, StateFidelity::None);
        assert_eq!(summary.header.starting_state.selection, "none");
        // Oldest listed first: sig 2 before sig 3.
        let capture = read_capture(&options.output).unwrap();
        assert_eq!(
            signatures_in_order(&capture),
            vec![signature(2), signature(3), signature(4)]
        );
    }

    #[tokio::test]
    async fn several_programs_without_block_order_are_flagged_as_grouped() {
        let dir = tempfile::tempdir().unwrap();
        let rpc = MockRpc::new(|method, params| {
            let entry = |n: u8| json!({ "signature": signature(n), "slot": 1003, "blockTime": 1_760_001_003i64, "err": null });
            match method {
                "getGenesisHash" => Ok(json!(MAINNET_GENESIS)),
                "getSlot" => Ok(json!(5000)),
                "getSignaturesForAddress" if params[0] == PROGRAM => {
                    Ok(json!([entry(3), entry(2)]))
                }
                "getSignaturesForAddress" => Ok(json!([entry(7)])),
                "getTransaction" => Ok(swap(
                    params[0].as_str().unwrap(),
                    1003,
                    [Some(2), Some(2), Some(3)],
                    false,
                )),
                other => panic!("unexpected method {other}"),
            }
        });
        let mut options = options(dir.path());
        options.programs = vec![PROGRAM.into(), TOKEN.into()];
        options.block_order = false;
        options.touched_state = false;
        let summary = run(&rpc, &options).await.unwrap();
        assert_eq!(summary.header.ordering, CaptureOrdering::Approximate);
        assert!(
            summary.header.limits.iter().any(|limit| limit
                .starts_with("1 slots without block order hold transactions of several programs")),
            "{:?}",
            summary.header.limits
        );
    }

    #[tokio::test]
    async fn touched_state_keeps_program_owned_accounts_only() {
        let dir = tempfile::tempdir().unwrap();
        let rpc = mock(Scenario {
            block_1005_fails: false,
            pages_of_newer: 0,
        });
        let summary = run(&rpc, &options(dir.path())).await.unwrap();
        assert_eq!(summary.state_accounts, 1);
        assert_eq!(summary.header.starting_state.selection, "touched");
        assert!(summary.missing_accounts.is_empty());
        let fetched: Vec<Value> = rpc
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(m, _)| m == "getMultipleAccounts")
            .map(|(_, p)| p[0].clone())
            .collect();
        // Invoked programs are not fetched.
        assert_eq!(fetched, vec![json!([PAYER, POOL, VAULT, MINT])]);
        assert!(!rpc.methods().iter().any(|m| m == "getProgramAccounts"));
        // The state is delivered before the window, in slot order; the slot
        // it was read at is in the header.
        let capture = read_capture(std::path::Path::new(&summary.path)).unwrap();
        let first_window_slot = capture
            .records
            .iter()
            .find(|record| !record.is_state())
            .map(|record| record.update().slot)
            .unwrap();
        let state = capture
            .records
            .iter()
            .find(|record| record.is_state())
            .unwrap();
        assert_eq!(state.update().slot, first_window_slot - 1);
        let slots: Vec<u64> = capture.records.iter().map(|r| r.update().slot).collect();
        assert!(slots.windows(2).all(|pair| pair[0] <= pair[1]), "{slots:?}");
        assert!(summary.header.starting_state.slot.unwrap() > first_window_slot);
    }

    #[tokio::test]
    async fn the_state_cap_keeps_explicit_accounts_first() {
        let dir = tempfile::tempdir().unwrap();
        let rpc = mock(Scenario {
            block_1005_fails: false,
            pages_of_newer: 0,
        });
        let mut options = options(dir.path());
        options.accounts = vec![MINT.into()];
        options.max_state_accounts = 1;
        let summary = run(&rpc, &options).await.unwrap();
        assert_eq!(summary.state_accounts, 1);
        assert!(summary
            .header
            .limits
            .iter()
            .any(|l| l.contains("capped at 1")));
    }

    #[tokio::test]
    async fn exhausting_the_signature_pages_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let rpc = mock(Scenario {
            block_1005_fails: false,
            pages_of_newer: 1,
        });
        let mut options = options(dir.path());
        options.max_signature_pages = 2;
        let error = run(&rpc, &options).await.unwrap_err().to_string();
        assert!(error.contains("2 pages"), "{error}");
        assert!(error.contains("more recent window"), "{error}");
        let pages = rpc
            .methods()
            .iter()
            .filter(|m| *m == "getSignaturesForAddress")
            .count();
        assert_eq!(pages, 2);
        assert!(!options.output.exists());
    }

    #[tokio::test]
    async fn too_many_transactions_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let rpc = mock(Scenario {
            block_1005_fails: false,
            pages_of_newer: 0,
        });
        let mut options = options(dir.path());
        options.max_transactions = 2;
        let error = run(&rpc, &options).await.unwrap_err().to_string();
        assert!(error.contains("3 transactions"), "{error}");
        assert!(error.contains("--max-transactions"), "{error}");
    }

    #[tokio::test]
    async fn a_window_that_has_not_ended_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let rpc = mock(Scenario {
            block_1005_fails: false,
            pages_of_newer: 0,
        });
        let mut options = options(dir.path());
        options.to = WindowBound::Slot(6000);
        let error = run(&rpc, &options).await.unwrap_err().to_string();
        assert!(error.contains("must have ended"), "{error}");
        options.to = WindowBound::Time(now_millis() / 1000 + 600);
        let error = run(&rpc, &options).await.unwrap_err().to_string();
        assert!(error.contains("must have ended"), "{error}");
    }

    #[test]
    fn failed_transactions_are_flagged() {
        let converted = convert_transaction(
            swap(&signature(1), 5, [Some(2), Some(2), Some(3)], true),
            &programs(),
        )
        .unwrap();
        assert!(converted.failed);
    }

    #[test]
    fn genesis_hashes_name_clusters() {
        assert_eq!(cluster_name(MAINNET_GENESIS), "mainnet-beta");
        assert_eq!(cluster_name(DEVNET_GENESIS), "devnet");
        assert_eq!(cluster_name(TESTNET_GENESIS), "testnet");
        assert_eq!(cluster_name(SYSTEM), "unknown");
    }
}
