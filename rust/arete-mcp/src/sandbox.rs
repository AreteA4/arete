//! Sandbox wallet tools: an agent operates a hosted sandbox wallet through
//! the Arete API with a sandbox session key (`a4_sbx_…`).
//!
//! The sandbox's owner decides which programs and instructions the wallet
//! may use and how much may leave it; Arete builds, checks, signs and sends
//! every transaction. The agent sends named instructions and never sees a
//! private key or an RPC URL. Owner operations (rules, caps, pause, revoke)
//! are not reachable from here.
//!
//! The key comes from `ARETE_SANDBOX_KEY` and is sent only to the API origin
//! (`ARETE_API_URL`, else the default Arete API), and only when that origin is
//! an Arete host over HTTPS or loopback (as for every other key this server
//! holds). It is never returned in a tool result.

use std::time::Duration;

use rmcp::schemars;
use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::credentials::{normalize_api_url, DEFAULT_API_URL};
pub(crate) use crate::registry::MAX_RESPONSE_BYTES;

pub const ENV_VAR_SANDBOX_KEY: &str = "ARETE_SANDBOX_KEY";
const ENV_VAR_API_URL: &str = "ARETE_API_URL";
const KEY_PREFIX: &str = "a4_sbx_";
/// Invokes wait for the transaction to confirm.
const ACTION_TIMEOUT: Duration = Duration::from_secs(90);
const READ_TIMEOUT: Duration = Duration::from_secs(30);

/// The tools this module backs, hidden when no sandbox key is configured.
pub const TOOL_NAMES: [&str; 7] = [
    "sandbox_status",
    "sandbox_rules",
    "sandbox_derive",
    "sandbox_preview",
    "sandbox_invoke",
    "sandbox_withdraw",
    "sandbox_action",
];

/// An answer the API refused: its HTTP status and problem body.
#[derive(Debug, Clone)]
pub struct SandboxRefusal {
    pub status: u16,
    pub body: Value,
}

impl SandboxRefusal {
    /// What the agent sees: the API's code, message and the rule details,
    /// plus a hint for the codes an agent can act on.
    pub fn to_json(&self) -> Value {
        let mut out = Map::new();
        out.insert("status".into(), json!(self.status));
        if let Some(object) = self.body.as_object() {
            for (key, value) in object {
                if matches!(key.as_str(), "schemaVersion") {
                    continue;
                }
                out.insert(key.clone(), value.clone());
            }
        } else if !self.body.is_null() {
            out.insert("error".into(), self.body.clone());
        }
        if let Some(hint) = self
            .body
            .get("code")
            .and_then(Value::as_str)
            .and_then(hint_for)
        {
            out.insert("hint".into(), json!(hint));
        }
        Value::Object(out)
    }
}

/// What an agent can do about a refusal.
fn hint_for(code: &str) -> Option<&'static str> {
    Some(match code {
        "sandbox-program-not-allowlisted" | "sandbox-guard-program-not-allowlisted" => {
            "The program is not in the sandbox's rules (sandbox_rules lists them). Only the owner can add it, in the dashboard."
        }
        "sandbox-instruction-not-allowlisted" | "sandbox-guard-instruction-not-allowlisted" => {
            "That instruction is not allowed for this program. Use one sandbox_rules lists, or ask the owner to allow it."
        }
        "sandbox-guard-outflow-exceeded" | "sandbox-limit-per-action" | "sandbox-limit-window" => {
            "Over a cap: use a smaller amount (sandbox_status shows caps and what the window has spent)."
        }
        "sandbox-paused" => "The sandbox is paused. Only its owner can resume it.",
        "sandbox-rules-missing" | "sandbox-rules-unsigned" => {
            "The owner must set or re-sign the sandbox's rules in the dashboard."
        }
        "sandbox-intent-invalid" | "sandbox-intent-foreign-signer" => {
            "Fix the instruction: names, args and accounts must match sandbox_rules; the sandbox wallet is the only signer."
        }
        "sandbox-guard-trial-failed" => {
            "The transaction fails when run. Check accounts and amounts; the guard's logs are in the details when present."
        }
        "sandbox-duplicate-action" => {
            "This idempotency key already ran: look it up with sandbox_action instead of retrying."
        }
        "sandbox-rate-limited" => "Too many requests: wait for retryAfterSeconds.",
        "sandbox-api-unreachable" | "sandbox-response-unreadable" => {
            "If this was sandbox_invoke or sandbox_withdraw, the action may have run: look it up with sandbox_action and the idempotencyKey before retrying, and retry only with the same key."
        }
        "sandbox-response-too-large" => {
            "The answer is too large for one tool result. For an action, look it up with sandbox_action and the idempotencyKey."
        }
        "sandbox-session-invalid" | "sandbox-session-revoked" | "sandbox-session-expired" => {
            "The sandbox key no longer works. Ask the owner for a new one."
        }
        _ => return None,
    })
}

/// The Arete API's sandbox agent routes, under one session key.
#[derive(Clone)]
pub struct SandboxClient {
    http: reqwest::Client,
    api_base: String,
    key: Option<String>,
}

impl SandboxClient {
    /// From `ARETE_SANDBOX_KEY` and `ARETE_API_URL`. A value that is not a
    /// sandbox key is ignored (and logged), leaving the tools hidden.
    pub fn from_env() -> Self {
        Self::from_values(
            std::env::var(ENV_VAR_SANDBOX_KEY).ok(),
            std::env::var(ENV_VAR_API_URL).ok(),
        )
    }

    pub fn from_values(key: Option<String>, api_url: Option<String>) -> Self {
        let key = key.map(|key| key.trim().to_string()).filter(|key| {
            let ok = key.starts_with(KEY_PREFIX);
            if !ok && !key.is_empty() {
                tracing::warn!(
                    "{ENV_VAR_SANDBOX_KEY} is not a sandbox key; sandbox tools are disabled"
                );
            }
            ok
        });
        let api_base = api_url
            .map(|url| normalize_api_url(&url))
            .unwrap_or_else(|| DEFAULT_API_URL.to_string());
        Self {
            http: reqwest::Client::new(),
            api_base: api_base.trim_end_matches('/').to_string(),
            key,
        }
    }

    pub fn configured(&self) -> bool {
        self.key.is_some()
    }

    async fn call(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
        idempotency_key: Option<&str>,
    ) -> Result<Value, SandboxRefusal> {
        let Some(key) = &self.key else {
            return Err(SandboxRefusal {
                status: 0,
                body: json!({
                    "code": "sandbox-not-configured",
                    "error": format!("Set {ENV_VAR_SANDBOX_KEY} to a sandbox key to use the sandbox tools"),
                }),
            });
        };
        if !crate::registry::is_arete_origin(&self.api_base) {
            return Err(SandboxRefusal {
                status: 0,
                body: json!({
                    "code": "sandbox-untrusted-api",
                    "error": format!(
                        "Refusing to send the sandbox key to {}: only an https://*.arete.run origin or loopback. Nothing was sent.",
                        self.api_base
                    ),
                }),
            });
        }
        let url = format!("{}/api/sandbox/v1{path}", self.api_base);
        let timeout = if method == reqwest::Method::POST {
            ACTION_TIMEOUT
        } else {
            READ_TIMEOUT
        };
        let mut request = self
            .http
            .request(method, &url)
            .bearer_auth(key)
            .timeout(timeout);
        if let Some(idempotency_key) = idempotency_key {
            request = request.header("idempotency-key", idempotency_key);
        }
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await.map_err(|error| SandboxRefusal {
            status: 0,
            // Without the URL: it is the configured API origin, not a secret,
            // but keep tool output to what the agent needs.
            body: json!({
                "code": "sandbox-api-unreachable",
                "error": format!("The Arete API did not answer: {}", error.without_url()),
                "retryable": true,
            }),
        })?;
        let status = response.status().as_u16();
        let success = (200..300).contains(&status);
        let unreadable = |detail: String| SandboxRefusal {
            status,
            body: json!({
                "code": "sandbox-response-unreadable",
                "error": format!("The Arete API answered {status} but its body could not be read: {detail}"),
                "retryable": true,
            }),
        };
        let bytes = match read_bounded(response).await {
            Ok(bytes) => bytes,
            Err(BodyError::TooLarge) => {
                return Err(SandboxRefusal {
                    status,
                    body: json!({
                        "code": "sandbox-response-too-large",
                        "error": format!(
                            "The Arete API answer is over the {MAX_RESPONSE_BYTES} byte limit for a tool result"
                        ),
                    }),
                })
            }
            Err(BodyError::Read(detail)) => return Err(unreadable(detail)),
        };
        match serde_json::from_slice::<Value>(&bytes) {
            Ok(body) if success && body.is_object() => Ok(body),
            // A success the agent cannot read is never reported as one: an
            // action may have run, and the agent needs its key to look it up.
            Ok(_) if success => Err(unreadable("not a JSON object".into())),
            Err(error) if success => Err(unreadable(error.to_string())),
            Ok(body) => Err(SandboxRefusal { status, body }),
            Err(_) => Err(SandboxRefusal {
                status,
                body: Value::Null,
            }),
        }
    }

    pub async fn status(&self) -> Result<Value, SandboxRefusal> {
        self.call(reqwest::Method::GET, "/wallet", None, None).await
    }

    pub async fn rules(&self) -> Result<Value, SandboxRefusal> {
        self.call(reqwest::Method::GET, "/rules", None, None).await
    }

    pub async fn derive(&self, request: Value) -> Result<Value, SandboxRefusal> {
        self.call(reqwest::Method::POST, "/derive", Some(request), None)
            .await
    }

    pub async fn preview(&self, request: Value) -> Result<Value, SandboxRefusal> {
        self.call(
            reqwest::Method::POST,
            "/actions/preview",
            Some(request),
            None,
        )
        .await
    }

    pub async fn invoke(
        &self,
        request: Value,
        idempotency_key: &str,
    ) -> Result<Value, SandboxRefusal> {
        self.call(
            reqwest::Method::POST,
            "/actions/invoke",
            Some(request),
            Some(idempotency_key),
        )
        .await
    }

    pub async fn withdraw(
        &self,
        lamports: &str,
        transaction_format: Option<SandboxTransactionFormat>,
        idempotency_key: &str,
    ) -> Result<Value, SandboxRefusal> {
        let mut body = json!({ "lamports": lamports });
        if let Some(format) = transaction_format {
            body["transactionFormat"] = json!(format.as_str());
        }
        self.call(
            reqwest::Method::POST,
            "/actions/withdraw",
            Some(body),
            Some(idempotency_key),
        )
        .await
    }

    pub async fn action(&self, lookup: &ActionLookup) -> Result<Value, SandboxRefusal> {
        let path = match lookup {
            ActionLookup::Id(id) => format!("/actions/{}", path_segment(id)?),
            ActionLookup::IdempotencyKey(key) => {
                format!("/actions?idempotencyKey={}", url_encode(key))
            }
        };
        self.call(reqwest::Method::GET, &path, None, None).await
    }
}

enum BodyError {
    TooLarge,
    Read(String),
}

/// The response body, refusing to buffer more than a tool result may hold.
async fn read_bounded(mut response: reqwest::Response) -> Result<Vec<u8>, BodyError> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err(BodyError::TooLarge);
    }
    let mut bytes = Vec::new();
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                if bytes.len() + chunk.len() > MAX_RESPONSE_BYTES {
                    return Err(BodyError::TooLarge);
                }
                bytes.extend_from_slice(&chunk);
            }
            Ok(None) => return Ok(bytes),
            Err(error) => return Err(BodyError::Read(error.without_url().to_string())),
        }
    }
}

pub enum ActionLookup {
    Id(String),
    IdempotencyKey(String),
}

fn path_segment(value: &str) -> Result<String, SandboxRefusal> {
    let ok = !value.is_empty() && value.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    if ok {
        Ok(value.to_string())
    } else {
        Err(SandboxRefusal {
            status: 0,
            body: json!({ "code": "sandbox-invalid-argument", "error": "actionId must be a UUID" }),
        })
    }
}

fn url_encode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (byte as char).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}

/// A fresh idempotency key for an action the agent did not key itself.
pub fn new_idempotency_key() -> String {
    format!("mcp-{}", uuid::Uuid::new_v4())
}

/// The body of `preview` and `invoke`, from the tool arguments.
pub fn invoke_body(args: &SandboxInvokeArgs) -> Value {
    let mut body = Map::new();
    body.insert(
        "instructions".into(),
        Value::Array(
            args.instructions
                .iter()
                .map(|instruction| {
                    let mut item = Map::new();
                    item.insert("program".into(), json!(instruction.program));
                    item.insert("instruction".into(), json!(instruction.instruction));
                    if let Some(args) = &instruction.args {
                        item.insert("args".into(), args.clone());
                    }
                    if let Some(accounts) = &instruction.accounts {
                        item.insert("accounts".into(), accounts.clone());
                    }
                    if let Some(remaining) = &instruction.remaining_accounts {
                        item.insert(
                            "remainingAccounts".into(),
                            Value::Array(
                                remaining
                                    .iter()
                                    .map(|account| {
                                        json!({
                                            "address": account.address,
                                            "writable": account.writable.unwrap_or(false),
                                        })
                                    })
                                    .collect(),
                            ),
                        );
                    }
                    Value::Object(item)
                })
                .collect(),
        ),
    );
    if let Some(limit) = &args.compute_unit_limit {
        body.insert("computeUnitLimit".into(), limit.clone());
    }
    if let Some(price) = &args.compute_unit_price_micro_lamports {
        body.insert("computeUnitPriceMicroLamports".into(), price.clone());
    }
    if let Some(format) = args.transaction_format {
        body.insert("transactionFormat".into(), json!(format.as_str()));
    }
    Value::Object(body)
}

// ---- Tool arguments ----------------------------------------------------------

/// The Solana transaction format an action is built in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SandboxTransactionFormat {
    /// Transaction v1 (SIMD-0385), the default: up to 4096 bytes and 64
    /// accounts, the compute budget in the message itself.
    V1,
    /// The older v0 format: up to 1232 bytes, the compute budget as
    /// ComputeBudget instructions.
    V0,
}

impl SandboxTransactionFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::V1 => "v1",
            Self::V0 => "v0",
        }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SandboxRemainingAccountArg {
    /// Base58 address.
    pub address: String,
    /// Whether the instruction writes it (default false). Never a signer.
    #[serde(default)]
    pub writable: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SandboxInstructionArg {
    /// Program id (base58), as `sandbox_rules` lists it.
    pub program: String,
    /// Instruction name exactly as `sandbox_rules` lists it.
    pub instruction: String,
    /// Arguments by IDL name, e.g. `{"amount": "1000000", "max_sol_cost": "5000000"}`.
    /// Pass u64/u128 values as decimal strings.
    #[serde(default)]
    pub args: Option<Value>,
    /// Accounts by IDL name → base58 address. Accounts with a fixed address
    /// in the IDL may be omitted. Signer accounts must be the sandbox wallet.
    #[serde(default)]
    pub accounts: Option<Value>,
    /// Extra accounts appended after the named ones (e.g. bin arrays,
    /// reserves), in order.
    #[serde(default)]
    pub remaining_accounts: Option<Vec<SandboxRemainingAccountArg>>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SandboxInvokeArgs {
    /// 1 to 8 instructions, run in order in one transaction.
    pub instructions: Vec<SandboxInstructionArg>,
    /// Compute unit limit (default 400000, at most 1400000).
    #[serde(default)]
    pub compute_unit_limit: Option<Value>,
    /// Priority fee in micro-lamports per compute unit (default 0). A v1
    /// transaction pays the same total (price × limit, rounded up) as its
    /// lamport priority fee.
    #[serde(default)]
    pub compute_unit_price_micro_lamports: Option<Value>,
    /// `v1` (the default: up to 4096 bytes and 64 accounts) or `v0` (up to
    /// 1232 bytes). Use `v0` only when something downstream needs it.
    #[serde(default)]
    pub transaction_format: Option<SandboxTransactionFormat>,
    /// Your key for this action (1-128 chars). The same key never runs twice;
    /// after a timeout, look the action up with `sandbox_action` instead of
    /// retrying. Generated when omitted and returned in the result.
    #[serde(default)]
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SandboxPreviewArgs {
    /// The same instructions `sandbox_invoke` takes.
    pub instructions: Vec<SandboxInstructionArg>,
    #[serde(default)]
    pub compute_unit_limit: Option<Value>,
    #[serde(default)]
    pub compute_unit_price_micro_lamports: Option<Value>,
    /// As for `sandbox_invoke`: `v1` (the default) or `v0`.
    #[serde(default)]
    pub transaction_format: Option<SandboxTransactionFormat>,
}

impl SandboxPreviewArgs {
    pub fn into_invoke(self) -> SandboxInvokeArgs {
        SandboxInvokeArgs {
            instructions: self.instructions,
            compute_unit_limit: self.compute_unit_limit,
            compute_unit_price_micro_lamports: self.compute_unit_price_micro_lamports,
            transaction_format: self.transaction_format,
            idempotency_key: None,
        }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SandboxDeriveArgs {
    /// `ata` (associated token account) or `pda` (program-derived address).
    pub kind: String,
    /// `ata`: the mint.
    #[serde(default)]
    pub mint: Option<String>,
    /// `ata`: the owner (default: the sandbox wallet).
    #[serde(default)]
    pub owner: Option<String>,
    /// `ata`: SPL Token (default) or Token-2022 program id.
    #[serde(default)]
    pub token_program: Option<String>,
    /// `pda`: the program id.
    #[serde(default)]
    pub program_id: Option<String>,
    /// `pda`: seeds in order, each `{"kind": "string"|"pubkey"|"bytes"|"u8"|"u16"|"u32"|"u64", ...}`
    /// with `value` (or `hex` for bytes), e.g. `[{"kind":"string","value":"bonding-curve"},{"kind":"pubkey","value":"<mint>"}]`.
    #[serde(default)]
    pub seeds: Option<Vec<Value>>,
}

impl SandboxDeriveArgs {
    pub fn body(&self) -> Value {
        match self.kind.trim() {
            "pda" => json!({
                "kind": "pda",
                "programId": self.program_id,
                "seeds": self.seeds.clone().unwrap_or_default(),
            }),
            _ => {
                let mut body = json!({ "kind": self.kind.trim(), "mint": self.mint });
                if let Some(owner) = &self.owner {
                    body["owner"] = json!(owner);
                }
                if let Some(program) = &self.token_program {
                    body["tokenProgram"] = json!(program);
                }
                body
            }
        }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SandboxWithdrawArgs {
    /// Lamports to send to the owner's withdraw address (decimal string or number).
    pub lamports: Value,
    /// As for `sandbox_invoke`: `v1` (the default) or `v0`.
    #[serde(default)]
    pub transaction_format: Option<SandboxTransactionFormat>,
    /// As for `sandbox_invoke`.
    #[serde(default)]
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SandboxActionArgs {
    /// The `actionId` an invoke or withdraw returned.
    #[serde(default)]
    pub action_id: Option<String>,
    /// Or the idempotency key the action ran under.
    #[serde(default)]
    pub idempotency_key: Option<String>,
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::sync::{Arc, Mutex};

    use super::*;

    /// An API that records each request (head and body) and answers from
    /// `routes` by method and path; anything else is a 404.
    fn stub(
        routes: Vec<(&'static str, &'static str, u16, Value)>,
    ) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let mut request = Vec::new();
                let mut buffer = [0u8; 8192];
                loop {
                    match stream.read(&mut buffer) {
                        Ok(0) | Err(_) => break,
                        Ok(read) => request.extend_from_slice(&buffer[..read]),
                    }
                    let text = String::from_utf8_lossy(&request).to_string();
                    if let Some(end) = text.find("\r\n\r\n") {
                        let length = text
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                            })
                            .unwrap_or(0);
                        if request.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                let text = String::from_utf8_lossy(&request).to_string();
                log.lock().unwrap().push(text.clone());
                let mut words = text.split_whitespace();
                let method = words.next().unwrap_or("").to_string();
                let target = words.next().unwrap_or("").to_string();
                let (status, body) = routes
                    .iter()
                    .find(|(m, path, _, _)| *m == method && *path == target)
                    .map(|(_, _, status, body)| (*status, body.to_string()))
                    .unwrap_or((404, r#"{"code":"not-found"}"#.to_string()));
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        (format!("http://{address}"), seen)
    }

    fn client(base: &str) -> SandboxClient {
        SandboxClient::from_values(Some("a4_sbx_test".into()), Some(base.into()))
    }

    #[test]
    fn only_sandbox_keys_enable_the_tools() {
        assert!(!SandboxClient::from_values(None, None).configured());
        assert!(!SandboxClient::from_values(Some("a4_ak_agent".into()), None).configured());
        assert!(!SandboxClient::from_values(Some("  ".into()), None).configured());
        assert!(SandboxClient::from_values(Some(" a4_sbx_x ".into()), None).configured());
    }

    #[tokio::test]
    async fn status_sends_the_sandbox_key_as_a_bearer() {
        let (base, seen) = stub(vec![(
            "GET",
            "/api/sandbox/v1/wallet",
            200,
            json!({ "walletAddress": "W" }),
        )]);
        let body = client(&base).status().await.unwrap();
        assert_eq!(body["walletAddress"], "W");
        let request = seen.lock().unwrap()[0].to_ascii_lowercase();
        assert!(
            request.contains("authorization: bearer a4_sbx_test"),
            "{request}"
        );
    }

    #[tokio::test]
    async fn invoke_sends_the_idempotency_key_and_intents() {
        let (base, seen) = stub(vec![(
            "POST",
            "/api/sandbox/v1/actions/invoke",
            200,
            json!({ "status": "landed" }),
        )]);
        let args: SandboxInvokeArgs = serde_json::from_value(json!({
            "instructions": [{
                "program": "P", "instruction": "buy",
                "args": { "amount": "5" }, "accounts": { "user": "W" },
                "remainingAccounts": [{ "address": "X", "writable": true }]
            }],
            "computeUnitLimit": "300000"
        }))
        .unwrap();
        let body = client(&base)
            .invoke(invoke_body(&args), "key-1")
            .await
            .unwrap();
        assert_eq!(body["status"], "landed");
        let request = seen.lock().unwrap()[0].clone();
        assert!(
            request
                .to_ascii_lowercase()
                .contains("idempotency-key: key-1"),
            "{request}"
        );
        let sent: Value = serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(sent["instructions"][0]["instruction"], "buy");
        assert_eq!(
            sent["instructions"][0]["remainingAccounts"][0]["writable"],
            true
        );
        assert_eq!(sent["computeUnitLimit"], "300000");
        // No format given: the API builds v1, its default.
        assert!(sent.get("transactionFormat").is_none());
    }

    #[test]
    fn the_transaction_format_is_passed_through() {
        let args: SandboxPreviewArgs = serde_json::from_value(json!({
            "instructions": [{ "program": "P", "instruction": "buy" }],
            "transactionFormat": "v0"
        }))
        .unwrap();
        assert_eq!(invoke_body(&args.into_invoke())["transactionFormat"], "v0");
        let args: SandboxInvokeArgs = serde_json::from_value(json!({
            "instructions": [{ "program": "P", "instruction": "buy" }],
            "transactionFormat": "v1"
        }))
        .unwrap();
        assert_eq!(invoke_body(&args)["transactionFormat"], "v1");
        assert!(serde_json::from_value::<SandboxInvokeArgs>(json!({
            "instructions": [], "transactionFormat": "legacy"
        }))
        .is_err());
    }

    #[tokio::test]
    async fn withdraw_sends_the_format_only_when_given() {
        let (base, seen) = stub(vec![(
            "POST",
            "/api/sandbox/v1/actions/withdraw",
            200,
            json!({ "status": "landed", "transactionFormat": "v0" }),
        )]);
        let client = client(&base);
        client
            .withdraw("1000", Some(SandboxTransactionFormat::V0), "key-1")
            .await
            .unwrap();
        client.withdraw("1000", None, "key-2").await.unwrap();
        let bodies: Vec<Value> = seen
            .lock()
            .unwrap()
            .iter()
            .map(|request| serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap())
            .collect();
        assert_eq!(
            bodies[0],
            json!({ "lamports": "1000", "transactionFormat": "v0" })
        );
        assert_eq!(bodies[1], json!({ "lamports": "1000" }));
    }

    #[tokio::test]
    async fn denials_keep_the_code_rule_and_details() {
        let (base, _) = stub(vec![(
            "POST",
            "/api/sandbox/v1/actions/preview",
            403,
            json!({
                "schemaVersion": 1,
                "error": "over the cap",
                "code": "sandbox-guard-outflow-exceeded",
                "rule": "outflow-cap",
                "mint": "M",
                "attempted": "600",
                "cap": "500"
            }),
        )]);
        let refusal = client(&base).preview(json!({})).await.unwrap_err();
        let shown = refusal.to_json();
        assert_eq!(shown["status"], 403);
        assert_eq!(shown["code"], "sandbox-guard-outflow-exceeded");
        assert_eq!(shown["rule"], "outflow-cap");
        assert_eq!(shown["attempted"], "600");
        assert!(shown["hint"].as_str().unwrap().contains("cap"));
        assert!(shown.get("schemaVersion").is_none());
    }

    #[tokio::test]
    async fn actions_are_looked_up_by_id_or_key() {
        let (base, seen) = stub(vec![
            (
                "GET",
                "/api/sandbox/v1/actions/abc-1",
                200,
                json!({ "status": "sent" }),
            ),
            (
                "GET",
                "/api/sandbox/v1/actions?idempotencyKey=my%20key",
                200,
                json!({ "status": "landed" }),
            ),
        ]);
        let client = client(&base);
        assert_eq!(
            client
                .action(&ActionLookup::Id("abc-1".into()))
                .await
                .unwrap()["status"],
            "sent"
        );
        assert_eq!(
            client
                .action(&ActionLookup::IdempotencyKey("my key".into()))
                .await
                .unwrap()["status"],
            "landed"
        );
        assert!(client
            .action(&ActionLookup::Id("../wallet".into()))
            .await
            .is_err());
        assert_eq!(seen.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn the_key_goes_only_to_arete_or_loopback_origins() {
        for origin in [
            "http://api.arete.run",
            "https://arete.run.evil.test",
            "http://10.0.0.5:3000",
        ] {
            let refusal = client(origin).status().await.unwrap_err();
            assert_eq!(refusal.body["code"], "sandbox-untrusted-api", "{origin}");
        }
    }

    #[tokio::test]
    async fn an_unreadable_success_is_an_error_not_a_null_result() {
        // A 200 whose body is not JSON (as when it is cut off).
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let mut buffer = [0u8; 8192];
                let _ = stream.read(&mut buffer);
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 7\r\nconnection: close\r\n\r\n{{\"actio"
                );
            }
        });
        let refusal = client(&format!("http://{address}"))
            .invoke(json!({}), "key-9")
            .await
            .unwrap_err();
        assert_eq!(refusal.status, 200);
        assert_eq!(refusal.body["code"], "sandbox-response-unreadable");
        assert!(refusal.to_json()["hint"]
            .as_str()
            .unwrap()
            .contains("sandbox_action"));
    }

    #[tokio::test]
    async fn oversized_answers_are_refused_while_reading() {
        let big = json!({ "rules": "x".repeat(MAX_RESPONSE_BYTES + 10) });
        let (base, _) = stub(vec![("GET", "/api/sandbox/v1/rules", 200, big)]);
        let refusal = client(&base).rules().await.unwrap_err();
        assert_eq!(refusal.body["code"], "sandbox-response-too-large");
    }

    #[test]
    fn derive_bodies_follow_the_kind() {
        let ata: SandboxDeriveArgs =
            serde_json::from_value(json!({ "kind": "ata", "mint": "M", "tokenProgram": "T" }))
                .unwrap();
        assert_eq!(
            ata.body(),
            json!({ "kind": "ata", "mint": "M", "tokenProgram": "T" })
        );
        let pda: SandboxDeriveArgs = serde_json::from_value(json!({
            "kind": "pda", "programId": "P", "seeds": [{ "kind": "string", "value": "global" }]
        }))
        .unwrap();
        assert_eq!(
            pda.body(),
            json!({ "kind": "pda", "programId": "P", "seeds": [{ "kind": "string", "value": "global" }] })
        );
    }
}
