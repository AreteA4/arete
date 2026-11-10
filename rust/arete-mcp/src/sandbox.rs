//! Sandbox wallet tools: an agent operates a hosted sandbox wallet through
//! the Arete API's Action API, with a sandbox-scoped session token.
//!
//! The sandbox's owner decides which programs and instructions the wallet
//! may use, which actions an agent may run and how much may leave it; Arete
//! builds, checks, signs and sends every transaction. The agent names an
//! action and its arguments and never sees a private key or an RPC URL.
//! Owner operations (rules, caps, pause, revoke) are not reachable from here.
//!
//! Auth: the agent's own API key (`ARETE_API_KEY` or its a4 login) mints a
//! short-lived token for the sandbox in `ARETE_SANDBOX_ID` at
//! `POST /ws/sessions` (`targetKind: "sandbox"`). The mint succeeds only
//! while the sandbox's owner grants this agent access, and the token carries
//! one `sandbox:<action>` scope per granted action. The token, not the API
//! key, is what the sandbox routes see; it is re-minted when it expires.
//! Neither is sent anywhere but the API origin (`ARETE_API_URL`, else the
//! default Arete API), and only when that origin is an Arete host over HTTPS
//! or loopback. Neither is ever returned in a tool result.

use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub mod ledger;
pub mod rules;

use rmcp::schemars;
use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::credentials::{normalize_api_url, DEFAULT_API_URL};
pub(crate) use crate::registry::MAX_RESPONSE_BYTES;

pub const ENV_VAR_SANDBOX_ID: &str = "ARETE_SANDBOX_ID";
/// The retired admin-issued session key. Read only to say it is retired.
const ENV_VAR_RETIRED_SANDBOX_KEY: &str = "ARETE_SANDBOX_KEY";
const ENV_VAR_API_URL: &str = "ARETE_API_URL";
/// Actions wait for the transaction to confirm.
const ACTION_TIMEOUT: Duration = Duration::from_secs(90);
const READ_TIMEOUT: Duration = Duration::from_secs(30);
/// A token this close to expiry is re-minted before use.
const TOKEN_REFRESH_MARGIN_SECS: u64 = 30;

/// The tools this module backs, hidden when no sandbox is configured.
pub const TOOL_NAMES: [&str; 8] = [
    "sandbox_status",
    "sandbox_rules",
    "sandbox_derive",
    "sandbox_preview",
    "sandbox_invoke",
    "sandbox_action",
    "sandbox_history",
    "sandbox_holdings",
];

/// What a sandbox call does, which decides what retrying it means.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CallKind {
    /// Changes nothing: the wallet, rules, an address, an action lookup,
    /// history, holdings, and minting the sandbox token.
    #[default]
    Read,
    /// A dry run: nothing is signed, sent or recorded.
    Preview,
    /// May sign and send a transaction, under an idempotency key.
    Action,
}

impl CallKind {
    /// How long this server waits for the API's answer.
    fn timeout(self) -> Duration {
        match self {
            Self::Read => READ_TIMEOUT,
            Self::Preview | Self::Action => ACTION_TIMEOUT,
        }
    }

    /// `path` is the sandbox route, e.g. `/actions/invoke`.
    pub fn of(method: &reqwest::Method, path: &str) -> Self {
        let path = path.split('?').next().unwrap_or(path).trim_end_matches('/');
        if method == reqwest::Method::GET || path.ends_with("/derive") {
            Self::Read
        } else if path.ends_with("/preview") {
            Self::Preview
        } else {
            Self::Action
        }
    }
}

/// An answer the API refused: its HTTP status and problem body, and what
/// kind of call it answered.
#[derive(Debug, Clone)]
pub struct SandboxRefusal {
    pub status: u16,
    pub body: Value,
    pub kind: CallKind,
}

impl SandboxRefusal {
    /// Refused here. A request refused before anything was sent is refused
    /// again, whatever status 0 would otherwise suggest; an API answer this
    /// server could not use (a mint without a token, a history page without
    /// items) may be fine next time, so that stays retryable.
    fn local(code: &str, error: impl Into<String>) -> Self {
        Self {
            status: 0,
            body: json!({
                "code": code,
                "error": error.into(),
                "retryable": transport_failure(code),
            }),
            kind: CallKind::Read,
        }
    }

    /// What the agent sees: the API's code, message and the rule details,
    /// and always `code`, `retryable` and a `hint`, whatever answered: the
    /// API, a proxy in front of it with no readable body, or nothing.
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
        let code = match out.get("code").and_then(Value::as_str) {
            Some(code) if !code.is_empty() => code.to_string(),
            _ => {
                let code = fallback_code(self.status);
                out.insert("code".into(), json!(code));
                code.to_string()
            }
        };
        if !out.contains_key("error") {
            out.insert(
                "error".into(),
                json!(format!(
                    "The Arete API answered HTTP {} with no readable error body",
                    self.status
                )),
            );
        }
        let retryable = match out.get("retryable").and_then(Value::as_bool) {
            Some(retryable) => retryable,
            None => {
                let retryable = retryable_status(self.status);
                out.insert("retryable".into(), json!(retryable));
                retryable
            }
        };
        // This server's hint names the MCP tools, so it wins; then the
        // API's own; then the rule for the kind of call.
        // The funds code covers SOL and tokens; the API marks a token
        // shortfall `shortfall: "token"`. A missing balance alone proves
        // nothing, so without the marker the SOL hint stands.
        let token_shortfall = code == "sandbox-guard-insufficient-funds"
            && out.get("shortfall").and_then(Value::as_str) == Some("token");
        let hint = token_shortfall
            .then(|| TOKEN_FUNDS_HINT.to_string())
            .or_else(|| hint_for(&code, retryable, self.kind))
            .or_else(|| out.get("hint").and_then(Value::as_str).map(str::to_string))
            .unwrap_or_else(|| generic_hint(retryable, self.kind).to_string());
        out.insert("hint".into(), json!(hint));
        Value::Object(out)
    }
}

/// The code for an answer that carries none, from its status alone.
fn fallback_code(status: u16) -> &'static str {
    match status {
        0 => "sandbox-api-unreachable",
        400 | 422 => "sandbox-request-invalid",
        401 => "sandbox-unauthorized",
        403 => "sandbox-forbidden",
        404 => "sandbox-not-found",
        408 | 504 => "sandbox-api-timeout",
        409 => "sandbox-conflict",
        429 => "sandbox-rate-limited",
        502 | 503 => "sandbox-api-unavailable",
        500..=599 => "sandbox-internal-error",
        _ => "sandbox-request-refused",
    }
}

/// Whether a status alone says the same call may succeed later.
fn retryable_status(status: u16) -> bool {
    matches!(status, 0 | 408 | 429 | 500..=599)
}

/// The API, or the way to it, failed: nothing about the request itself.
fn transport_failure(code: &str) -> bool {
    matches!(
        code,
        "sandbox-api-unreachable"
            | "sandbox-api-timeout"
            | "sandbox-api-unavailable"
            | "sandbox-internal-error"
            | "sandbox-response-unreadable"
    )
}

/// The guard, not the transaction, failed.
fn guard_infrastructure(code: &str) -> bool {
    matches!(
        code,
        "sandbox-guard-unavailable"
            | "sandbox-guard-fork-unavailable"
            | "sandbox-guard-lease-timeout"
            | "sandbox-guard-lease-unavailable"
            | "sandbox-guard-timeout"
            | "sandbox-guard-incomplete"
            | "sandbox-guard-clock-skew"
    )
}

const TOKEN_FUNDS_HINT: &str = "A token account this transaction spends from holds less than it moves; adding SOL does not help. Check the amounts against sandbox_holdings (the logs show which transfer failed), then hold more of that token or move less.";

const LOOK_UP_THE_ACTION: &str = "The action may have run. Look it up with sandbox_action and the idempotencyKey before retrying: if it ran, do not repeat it; if it was refused, retry with a new key; if no action has the key, retry with the same key.";

/// The rule for a call with no more specific hint.
fn generic_hint(retryable: bool, kind: CallKind) -> &'static str {
    match (retryable, kind) {
        (true, CallKind::Read) => {
            "Nothing was changed: retry the same call in a few seconds (retryAfterSeconds when present)."
        }
        (true, CallKind::Preview) => {
            "A dry run signs and records nothing: retry the same call in a few seconds (retryAfterSeconds when present)."
        }
        (true, CallKind::Action) => LOOK_UP_THE_ACTION,
        (false, CallKind::Action) => {
            "Refused before anything was signed: the same call is refused again. Fix it and run it with a new idempotencyKey."
        }
        (false, _) => "The same call is refused again: fix it before retrying.",
    }
}

/// What an agent can do about a refusal.
fn hint_for(code: &str, retryable: bool, kind: CallKind) -> Option<String> {
    if transport_failure(code) {
        return Some(generic_hint(true, kind).to_string());
    }
    if guard_infrastructure(code) {
        return Some(
            match (retryable, kind) {
                (true, CallKind::Action) => {
                    "The guard could not check this transaction right now, so nothing was signed or sent. Retry after retryAfterSeconds with a new idempotencyKey: this one is recorded as refused."
                }
                (true, _) => {
                    "The guard could not check this transaction right now; nothing was signed. Retry the same call after retryAfterSeconds."
                }
                (false, _) => {
                    "The guard could not check this transaction and the same call will not pass on a retry; nothing was signed. Change the transaction, or report the requestId to Arete."
                }
            }
            .to_string(),
        );
    }
    Some(match code {
        "sandbox-guard-insufficient-funds" => {
            "The sandbox wallet cannot pay for this: balanceLamports is what it holds and shortfallLamports, when present, how much more it needs. Ask the owner to fund the wallet, or spend less; the same call is refused until then. When `remaining` is set and your amount is over it, funding is not enough: use a smaller amount."
        }
        "sandbox-program-not-allowlisted" | "sandbox-guard-program-not-allowlisted" => {
            "The program is not in the sandbox's rules (sandbox_rules lists them). Only the owner can add it, in the dashboard."
        }
        "sandbox-instruction-not-allowlisted" | "sandbox-guard-instruction-not-allowlisted" => {
            "That instruction is not allowed for this program. Use one sandbox_rules lists, or ask the owner to allow it."
        }
        "sandbox-guard-outflow-exceeded" | "sandbox-limit-per-action" | "sandbox-limit-window" => {
            "Over a cap: use a smaller amount (sandbox_status shows caps and what the window has spent)."
        }
        "sandbox-action-not-granted" => {
            "The owner has not granted this agent that action (sandbox_status lists the granted ones). Only the owner can grant it, in the dashboard."
        }
        "sandbox-preview-unavailable" => {
            "This action has no dry run. sandbox_status shows the caps it is checked against; sandbox_invoke checks it again before signing."
        }
        "sandbox-paused" => "The sandbox is paused. Only its owner can resume it.",
        "sandbox-rules-missing" | "sandbox-rules-unsigned" => {
            "The owner must set or re-sign the sandbox's rules in the dashboard."
        }
        "sandbox-derive-invalid" => {
            "Fix programId or the seeds: each seed is {kind, value} (or {kind: \"bytes\", hex}), at most 15 seeds of at most 32 bytes each."
        }
        "sandbox-intent-invalid" | "sandbox-intent-foreign-signer" => {
            "Fix the instruction: names, args and accounts must match sandbox_rules; the sandbox wallet is the only signer."
        }
        "sandbox-guard-trial-failed" => {
            "The transaction fails when run. Check accounts and amounts; the guard's logs are in the details when present."
        }
        "sandbox-swap-venue-not-allowed" | "sandbox-swap-no-venues" => {
            "No allowed venue can fill this swap. sandbox_rules lists the venues the owner allows."
        }
        "sandbox-swap-slippage-too-high" => {
            "Lower slippageBps to at most what sandbox_rules allows."
        }
        "sandbox-duplicate-action" => {
            "This idempotency key already ran: look it up with sandbox_action instead of retrying."
        }
        "sandbox-rate-limited" => "Too many requests: wait for retryAfterSeconds.",
        "sandbox-response-too-large" => match kind {
            CallKind::Action => {
                "The answer is too large for one tool result. Look the action up with sandbox_action and the idempotencyKey."
            }
            _ => {
                "The answer is too large for one tool result. Ask for less: a smaller limit, or without includeRaw."
            }
        },
        "sandbox-grant-missing" | "sandbox-consent-unsigned" => {
            "The sandbox's owner has not granted this agent access (or must grant it again). Ask the owner to grant access in the dashboard."
        }
        "sandbox-agent-key-required" => {
            "A sandbox token is minted with the agent account's secret API key: set ARETE_API_KEY to it (or log in as the agent)."
        }
        "sandbox-session-invalid" | "sandbox-session-revoked" | "sandbox-session-expired" => {
            "The sandbox token no longer works and could not be renewed. Ask the owner whether this agent's access was revoked."
        }
        _ => return None,
    }
    .to_string())
}

/// A minted sandbox token and what it may do.
#[derive(Clone)]
struct SessionToken {
    token: String,
    /// Unix seconds; 0 when the API did not say.
    expires_at: u64,
    /// The granted actions (`sandbox:<action>` scopes without the prefix).
    actions: Vec<String>,
}

impl SessionToken {
    fn fresh(&self, now: u64) -> bool {
        self.expires_at > now + TOKEN_REFRESH_MARGIN_SECS
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

/// The Arete API's sandbox agent routes, under one sandbox-scoped token.
#[derive(Clone)]
pub struct SandboxClient {
    http: reqwest::Client,
    api_base: String,
    sandbox_id: Option<String>,
    /// The agent's account API key, used only to mint sandbox tokens.
    api_key: Option<String>,
    token: Arc<Mutex<Option<SessionToken>>>,
}

impl SandboxClient {
    /// From `ARETE_SANDBOX_ID`, `ARETE_API_URL` and the account key the
    /// rest of this server uses (`ARETE_API_KEY`, else the a4 login for that
    /// API). Without a sandbox id the tools stay hidden.
    pub fn from_env() -> Self {
        Self::from_env_for(std::env::var(ENV_VAR_SANDBOX_ID).ok())
    }

    /// As [`Self::from_env`], for `sandbox_id` (else `ARETE_SANDBOX_ID`).
    pub fn from_env_for(sandbox_id: Option<String>) -> Self {
        if std::env::var(ENV_VAR_RETIRED_SANDBOX_KEY).is_ok_and(|key| !key.trim().is_empty()) {
            tracing::warn!(
                "{ENV_VAR_RETIRED_SANDBOX_KEY} is no longer used: set {ENV_VAR_SANDBOX_ID} to the sandbox id and use the agent's own API key"
            );
        }
        let api_url = std::env::var(ENV_VAR_API_URL).ok();
        let base = api_url
            .as_deref()
            .map(normalize_api_url)
            .unwrap_or_else(|| DEFAULT_API_URL.to_string());
        // The key is resolved for exactly the origin it will be sent to.
        let api_key = crate::credentials::resolve_key_for_api(&base)
            .ok()
            .flatten()
            .map(|key| key.key);
        let sandbox_id = sandbox_id.or_else(|| std::env::var(ENV_VAR_SANDBOX_ID).ok());
        Self::from_values(sandbox_id, api_key, api_url)
    }

    pub fn from_values(
        sandbox_id: Option<String>,
        api_key: Option<String>,
        api_url: Option<String>,
    ) -> Self {
        let sandbox_id = sandbox_id
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
            .filter(|id| {
                let ok = uuid::Uuid::parse_str(id).is_ok();
                if !ok {
                    tracing::warn!(
                        "{ENV_VAR_SANDBOX_ID} is not a sandbox id; sandbox tools are disabled"
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
            sandbox_id,
            api_key: api_key
                .map(|key| key.trim().to_string())
                .filter(|key| !key.is_empty()),
            token: Arc::new(Mutex::new(None)),
        }
    }

    pub fn configured(&self) -> bool {
        self.sandbox_id.is_some()
    }

    /// The sandbox id, once it parsed as one.
    pub fn sandbox_id(&self) -> Option<&str> {
        self.sandbox_id.as_deref()
    }

    fn trusted_origin(&self) -> Result<(), SandboxRefusal> {
        if crate::registry::is_arete_origin(&self.api_base) {
            Ok(())
        } else {
            Err(SandboxRefusal::local(
                "sandbox-untrusted-api",
                format!(
                    "Refusing to send credentials to {}: only an https://*.arete.run origin or loopback. Nothing was sent.",
                    self.api_base
                ),
            ))
        }
    }

    fn cached_token(&self) -> Option<SessionToken> {
        self.token.lock().ok().and_then(|cached| cached.clone())
    }

    fn forget_token(&self) {
        if let Ok(mut cached) = self.token.lock() {
            *cached = None;
        }
    }

    /// A live sandbox token: the cached one, or a fresh mint.
    async fn session(&self) -> Result<SessionToken, SandboxRefusal> {
        if let Some(token) = self.cached_token().filter(|t| t.fresh(unix_now())) {
            return Ok(token);
        }
        let Some(sandbox_id) = &self.sandbox_id else {
            return Err(SandboxRefusal::local(
                "sandbox-not-configured",
                format!("Set {ENV_VAR_SANDBOX_ID} to the sandbox id to use the sandbox tools"),
            ));
        };
        let Some(api_key) = &self.api_key else {
            return Err(SandboxRefusal::local(
                "sandbox-agent-key-required",
                "No API key: set ARETE_API_KEY to the agent account's secret key (or log in as the agent) to mint a sandbox token",
            ));
        };
        self.trusted_origin()?;
        let body = self
            .send(
                reqwest::Method::POST,
                &format!("{}/ws/sessions", self.api_base),
                api_key,
                Some(json!({ "targetKind": "sandbox", "targetId": sandbox_id })),
                None,
                CallKind::Read,
            )
            .await?;
        let token = body
            .get("token")
            .and_then(Value::as_str)
            .filter(|token| !token.is_empty())
            .ok_or_else(|| {
                SandboxRefusal::local(
                    "sandbox-response-unreadable",
                    "The Arete API minted no sandbox token",
                )
            })?;
        // Fail closed: no scopes means no actions.
        let actions = body
            .get("scopes")
            .and_then(Value::as_array)
            .map(|scopes| {
                scopes
                    .iter()
                    .filter_map(Value::as_str)
                    .filter_map(|scope| scope.strip_prefix("sandbox:"))
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        let minted = SessionToken {
            token: token.to_string(),
            expires_at: body.get("expiresAt").and_then(Value::as_u64).unwrap_or(0),
            actions,
        };
        if let Ok(mut cached) = self.token.lock() {
            *cached = Some(minted.clone());
        }
        Ok(minted)
    }

    async fn call(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
        idempotency_key: Option<&str>,
    ) -> Result<Value, SandboxRefusal> {
        self.call_with(None, method, path, body, idempotency_key)
            .await
    }

    /// `path` under the sandbox routes with `session`'s token. A token the
    /// API calls expired or invalid is re-minted once and the call repeated:
    /// the API refuses those before running anything, so the repeat (with
    /// the same idempotency key) cannot run an action twice. With `action`,
    /// every token used (the first and a renewed one) must grant it.
    async fn call_with(
        &self,
        action: Option<SandboxActionName>,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
        idempotency_key: Option<&str>,
    ) -> Result<Value, SandboxRefusal> {
        let session = self.authorized(action).await?;
        self.trusted_origin()?;
        let url = format!("{}/api/sandbox/v1{path}", self.api_base);
        let kind = CallKind::of(&method, path);
        let first = self
            .send(
                method.clone(),
                &url,
                &session.token,
                body.clone(),
                idempotency_key,
                kind,
            )
            .await;
        let stale = matches!(&first, Err(refusal) if refusal.status == 401
        && matches!(
            refusal.body.get("code").and_then(Value::as_str),
            Some("sandbox-session-expired" | "sandbox-session-invalid")
        ));
        if !stale {
            return first;
        }
        self.forget_token();
        let renewed = self.authorized(action).await?;
        self.send(method, &url, &renewed.token, body, idempotency_key, kind)
            .await
    }

    async fn send(
        &self,
        method: reqwest::Method,
        url: &str,
        bearer: &str,
        body: Option<Value>,
        idempotency_key: Option<&str>,
        kind: CallKind,
    ) -> Result<Value, SandboxRefusal> {
        let timeout = kind.timeout();
        let mut request = self
            .http
            .request(method, url)
            .bearer_auth(bearer)
            .timeout(timeout);
        if let Some(idempotency_key) = idempotency_key {
            request = request.header("idempotency-key", idempotency_key);
        }
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await.map_err(|error| {
            // Without the URL: it is the configured API origin, not a
            // secret, but keep tool output to what the agent needs.
            let (code, message) = if error.is_timeout() {
                (
                    "sandbox-api-timeout",
                    format!(
                        "The Arete API did not answer within {} s",
                        timeout.as_secs()
                    ),
                )
            } else {
                (
                    "sandbox-api-unreachable",
                    format!(
                        "The Arete API could not be reached: {}",
                        error.without_url()
                    ),
                )
            };
            SandboxRefusal {
                status: 0,
                body: json!({ "code": code, "error": message, "retryable": true }),
                kind,
            }
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
            kind,
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
                    kind,
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
            Ok(body) => Err(SandboxRefusal { status, body, kind }),
            // Not JSON (a proxy's HTML 504, an empty body): the status is
            // all there is, and `to_json` fills in the rest.
            Err(_) => Err(SandboxRefusal {
                status,
                body: Value::Null,
                kind,
            }),
        }
    }

    pub async fn status(&self) -> Result<Value, SandboxRefusal> {
        self.call(reqwest::Method::GET, "/wallet", None, None).await
    }

    pub async fn rules(&self) -> Result<Value, SandboxRefusal> {
        self.call(reqwest::Method::GET, "/rules", None, None).await
    }

    /// An address for `args`. A PDA is derived here, without the API or
    /// the chain ([`derive_pda`]); an associated token account asks the
    /// API, which reads the mint's owner (SPL Token or Token-2022) from the
    /// sandbox's chain.
    pub async fn derive(&self, args: &SandboxDeriveArgs) -> Result<Value, SandboxRefusal> {
        if args.kind.trim() == "pda" {
            return derive_pda(
                args.program_id.as_deref(),
                args.seeds.as_deref().unwrap_or_default(),
            );
        }
        self.call(reqwest::Method::POST, "/derive", Some(args.body()), None)
            .await
    }

    /// A live session, granting `action` when one is given: an action the
    /// owner's grant does not include is refused before anything is sent to
    /// the sandbox routes.
    async fn authorized(
        &self,
        action: Option<SandboxActionName>,
    ) -> Result<SessionToken, SandboxRefusal> {
        let session = self.session().await?;
        let Some(action) = action else {
            return Ok(session);
        };
        if session.actions.iter().any(|a| a == action.as_str()) {
            return Ok(session);
        }
        Err(SandboxRefusal {
            status: 0,
            body: json!({
                "code": "sandbox-action-not-granted", "retryable": false,
                "rule": "session-grant",
                "error": format!("This agent is not granted `{}` on this sandbox", action.as_str()),
                "action": action.as_str(),
                "granted": session.actions,
            }),
            kind: CallKind::Read,
        })
    }

    /// Dry-run `action` with `args`: plan and guard result, nothing signed
    /// or recorded.
    pub async fn preview(
        &self,
        action: SandboxActionName,
        args: Value,
    ) -> Result<Value, SandboxRefusal> {
        let Some(path) = action.preview_path() else {
            return Err(SandboxRefusal {
                status: 0,
                body: json!({
                    "code": "sandbox-preview-unavailable", "retryable": false,
                    "error": format!("`{}` has no dry run", action.as_str()),
                    "action": action.as_str(),
                }),
                kind: CallKind::Preview,
            });
        };
        self.call_with(Some(action), reqwest::Method::POST, path, Some(args), None)
            .await
    }

    /// Run `action` with `args` under `idempotency_key`.
    pub async fn invoke(
        &self,
        action: SandboxActionName,
        args: Value,
        idempotency_key: &str,
    ) -> Result<Value, SandboxRefusal> {
        self.call_with(
            Some(action),
            reqwest::Method::POST,
            action.invoke_path(),
            Some(args),
            Some(idempotency_key),
        )
        .await
    }

    /// `tail` under this sandbox's ledger routes. The id parsed as a UUID,
    /// so it is safe in a path.
    fn ledger_path(&self, tail: &str) -> Result<String, SandboxRefusal> {
        let Some(sandbox_id) = &self.sandbox_id else {
            return Err(SandboxRefusal::local(
                "sandbox-not-configured",
                format!("Set {ENV_VAR_SANDBOX_ID} to the sandbox id to use the sandbox tools"),
            ));
        };
        Ok(format!("/sandboxes/{sandbox_id}{tail}"))
    }

    /// The sandbox's actions, newest first, one page at `query.cursor`.
    /// With filters, further pages are read (up to
    /// [`ledger::MAX_SCAN_PAGES`]), each asking for the items still wanted,
    /// until `limit` items match; `nextCursor` carries on after the last page
    /// read.
    pub async fn history(&self, query: &ledger::HistoryQuery) -> Result<Value, SandboxRefusal> {
        let mut cursor = query.cursor.clone();
        let mut items = Vec::new();
        let mut pages_read = 0;
        loop {
            // Ask only for what is still wanted, so the answer never holds
            // more than `limit` items and none are skipped behind the cursor.
            let wanted = query.limit().saturating_sub(items.len() as u32).max(1);
            let path = self.ledger_path(&format!(
                "/actions{}",
                query.page_query(cursor.as_deref(), wanted)
            ))?;
            let page = self.call(reqwest::Method::GET, &path, None, None).await?;
            pages_read += 1;
            let Some(page_items) = page.get("items").and_then(Value::as_array) else {
                return Err(SandboxRefusal::local(
                    "sandbox-response-unreadable",
                    "The Arete API answered the history read without an `items` list",
                ));
            };
            if !query.filtered() {
                return Ok(page);
            }
            items.extend(
                page_items
                    .iter()
                    .filter(|item| query.matches(item))
                    .cloned(),
            );
            cursor = page
                .get("nextCursor")
                .and_then(Value::as_str)
                .filter(|next| !next.is_empty())
                .map(str::to_string);
            let enough = items.len() >= query.limit() as usize;
            if enough || cursor.is_none() || pages_read >= ledger::MAX_SCAN_PAGES {
                return Ok(ledger::Scan {
                    items,
                    next_cursor: cursor,
                    pages_read,
                }
                .into_json(query));
            }
        }
    }

    /// What the sandbox holds now: SOL, tokens and program positions.
    pub async fn holdings(&self) -> Result<Value, SandboxRefusal> {
        let path = self.ledger_path("/holdings")?;
        self.call(reqwest::Method::GET, &path, None, None).await
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
        Err(SandboxRefusal::local(
            "sandbox-invalid-argument",
            "actionId must be a UUID",
        ))
    }
}

pub(crate) fn url_encode(value: &str) -> String {
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

// ---- Tool arguments ----------------------------------------------------------

/// An Action API action an agent may run, when the owner grants it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SandboxActionName {
    /// Named program instructions, encoded from the program's IDL. `args`:
    /// `{ instructions: [{ program, instruction, args?, accounts?,
    /// remainingAccounts?: [{ address, writable? }] }], computeUnitLimit?,
    /// computeUnitPriceMicroLamports?, transactionFormat?: "v1" | "v0" }`,
    /// 1 to 8 instructions named exactly as `sandbox_rules` lists them.
    Invoke,
    /// SOL to the owner's withdraw address, the only destination a withdraw
    /// can have. `args`: `{ lamports, transactionFormat?: "v1" | "v0" }`.
    Withdraw,
    /// An exact-input token swap through the venues the owner allows.
    /// `args`: `{ inputMint, outputMint, amount, slippageBps?,
    /// computeUnitLimit?, computeUnitPriceMicroLamports? }`, `amount` in the
    /// input mint's smallest unit.
    Swap,
}

impl SandboxActionName {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Invoke => "invoke",
            Self::Withdraw => "withdraw",
            Self::Swap => "swap",
        }
    }

    fn invoke_path(self) -> &'static str {
        match self {
            Self::Invoke => "/actions/invoke",
            Self::Withdraw => "/actions/withdraw",
            Self::Swap => "/actions/swap",
        }
    }

    fn preview_path(self) -> Option<&'static str> {
        match self {
            Self::Invoke => Some("/actions/preview"),
            Self::Withdraw => None,
            Self::Swap => Some("/actions/swap/preview"),
        }
    }
}

/// `args` as the API takes them: a JSON object (absent is `{}`).
pub fn action_args(args: Option<Value>) -> Result<Value, String> {
    match args {
        None | Some(Value::Null) => Ok(json!({})),
        Some(Value::Object(object)) => Ok(Value::Object(object)),
        Some(other) => Err(format!("args must be a JSON object, not {other}")),
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SandboxInvokeArgs {
    /// The action: `invoke` (named program instructions), `withdraw` (SOL
    /// to the owner) or `swap`. `sandbox_status` lists the ones granted.
    pub action: SandboxActionName,
    /// The action's arguments, as the action's description gives them.
    /// Pass u64/u128 amounts as decimal strings.
    #[serde(default)]
    pub args: Option<Value>,
    /// Your key for this action (1-128 chars). The same key never runs twice;
    /// after a timeout, look the action up with `sandbox_action` instead of
    /// retrying. Generated when omitted and returned in the result.
    #[serde(default)]
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SandboxPreviewArgs {
    /// The action, as for `sandbox_invoke`.
    pub action: SandboxActionName,
    /// The same arguments `sandbox_invoke` takes for it.
    #[serde(default)]
    pub args: Option<Value>,
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

/// The most seeds a PDA takes besides its bump, and the longest one.
const MAX_PDA_SEEDS: usize = 15;
const MAX_PDA_SEED_BYTES: usize = 32;

/// One `sandbox_derive` PDA seed: `{"kind": "string"|"pubkey"|"bytes"|"u8"|
/// "u16"|"u32"|"u64", ...}`, encoded as the sandbox API encodes it.
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
enum PdaSeed {
    String { value: String },
    Pubkey { value: String },
    Bytes { hex: String },
    U8 { value: FlexibleU64 },
    U16 { value: FlexibleU64 },
    U32 { value: FlexibleU64 },
    U64 { value: FlexibleU64 },
}

/// A u64 as a JSON number or a decimal string.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum FlexibleU64 {
    Number(u64),
    Text(String),
}

impl FlexibleU64 {
    fn get(&self, position: usize) -> Result<u64, String> {
        match self {
            Self::Number(value) => Ok(*value),
            Self::Text(text) => text
                .trim()
                .parse()
                .map_err(|_| format!("seeds[{position}]: {text} is not a u64")),
        }
    }
}

fn hex_bytes(text: &str) -> Option<Vec<u8>> {
    let text = text.trim();
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(text.get(at..at + 2)?, 16).ok())
        .collect()
}

fn pda_seed_bytes(seed: &Value, position: usize) -> Result<Vec<u8>, String> {
    let seed: PdaSeed = serde_json::from_value(seed.clone())
        .map_err(|error| format!("seeds[{position}]: {error}"))?;
    let narrow = |value: &FlexibleU64, max: u64, kind: &str| {
        let value = value.get(position)?;
        if value > max {
            Err(format!("seeds[{position}] is too large for {kind}"))
        } else {
            Ok(value)
        }
    };
    Ok(match seed {
        PdaSeed::String { value } => value.into_bytes(),
        PdaSeed::Pubkey { value } => arete_sdk::instruction::Pubkey::from_str(value.trim())
            .map_err(|_| format!("seeds[{position}] is not a base58 address: {value}"))?
            .to_bytes()
            .to_vec(),
        PdaSeed::Bytes { hex } => {
            hex_bytes(&hex).ok_or_else(|| format!("seeds[{position}] is not hex"))?
        }
        PdaSeed::U8 { value } => vec![narrow(&value, u64::from(u8::MAX), "u8")? as u8],
        PdaSeed::U16 { value } => (narrow(&value, u64::from(u16::MAX), "u16")? as u16)
            .to_le_bytes()
            .to_vec(),
        PdaSeed::U32 { value } => (narrow(&value, u64::from(u32::MAX), "u32")? as u32)
            .to_le_bytes()
            .to_vec(),
        PdaSeed::U64 { value } => value.get(position)?.to_le_bytes().to_vec(),
    })
}

/// A program-derived address, computed here: no network round trip, so it
/// answers even when the API is slow or unreachable. The same seeds, limits
/// and answer (`address`, `bump`) as the API's `/derive`.
pub fn derive_pda(program_id: Option<&str>, seeds: &[Value]) -> Result<Value, SandboxRefusal> {
    let invalid = |message: String| SandboxRefusal::local("sandbox-derive-invalid", message);
    let program_id = program_id
        .map(str::trim)
        .filter(|program| !program.is_empty())
        .ok_or_else(|| invalid("a PDA needs programId".into()))?;
    let program = arete_sdk::instruction::Pubkey::from_str(program_id)
        .map_err(|_| invalid(format!("programId is not a base58 address: {program_id}")))?;
    if seeds.len() > MAX_PDA_SEEDS {
        return Err(invalid(format!(
            "a PDA takes at most {MAX_PDA_SEEDS} seeds besides the bump"
        )));
    }
    let bytes = seeds
        .iter()
        .enumerate()
        .map(|(position, seed)| pda_seed_bytes(seed, position))
        .collect::<Result<Vec<_>, _>>()
        .map_err(invalid)?;
    if let Some(position) = bytes
        .iter()
        .position(|seed| seed.len() > MAX_PDA_SEED_BYTES)
    {
        return Err(invalid(format!(
            "seeds[{position}] is longer than {MAX_PDA_SEED_BYTES} bytes"
        )));
    }
    let refs: Vec<&[u8]> = bytes.iter().map(Vec::as_slice).collect();
    let (address, bump) = arete_sdk::instruction::Pubkey::try_find_program_address(&refs, &program)
        .ok_or_else(|| invalid("no bump gives an address off the curve".into()))?;
    Ok(json!({ "address": address.to_string(), "bump": bump }))
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SandboxActionArgs {
    /// The `actionId` a `sandbox_invoke` returned.
    #[serde(default)]
    pub action_id: Option<String>,
    /// Or the idempotency key the action ran under.
    #[serde(default)]
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SandboxHoldingsArgs {
    /// Keep each position's `raw` account data (default false).
    #[serde(default)]
    pub include_raw: bool,
}

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SandboxHistoryArgs {
    /// The `nextCursor` of the previous page; omit for the newest actions.
    #[serde(default)]
    pub cursor: Option<String>,
    /// Actions per page, 1 to 100 (default 20).
    #[serde(default)]
    pub limit: Option<u32>,
    /// Only actions with this status: `landed`, `failed`, `rejected`,
    /// `sent`, ...
    #[serde(default)]
    pub status: Option<String>,
    /// Only actions that called this program, by id or name.
    #[serde(default)]
    pub program: Option<String>,
    /// Only actions that moved this token mint.
    #[serde(default)]
    pub mint: Option<String>,
    /// Only actions at or after this RFC 3339 time, e.g.
    /// `2026-11-15T00:00:00Z`.
    #[serde(default)]
    pub since: Option<String>,
    /// Keep each action's `raw` parsed transaction (large; default false).
    #[serde(default)]
    pub include_raw: bool,
}

impl SandboxHistoryArgs {
    pub fn query(&self) -> Result<ledger::HistoryQuery, String> {
        let since = self
            .since
            .as_deref()
            .map(|text| {
                chrono::DateTime::parse_from_rfc3339(text.trim())
                    .map(|time| time.with_timezone(&chrono::Utc))
                    .map_err(|_| format!("since must be an RFC 3339 time, not {text:?}"))
            })
            .transpose()?;
        let text = |value: &Option<String>| {
            value
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        };
        Ok(ledger::HistoryQuery {
            cursor: text(&self.cursor),
            limit: self.limit,
            status: text(&self.status),
            program: text(&self.program),
            mint: text(&self.mint),
            since,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::sync::{Arc, Mutex};

    use super::*;

    const SANDBOX: &str = "6f1c2c43-6d3c-4a51-9b8e-0e3c1e2a7b10";

    /// An API that records each request (head and body) and answers from
    /// `routes` by method and path; anything else is a 404. `POST
    /// /ws/sessions` mints `tok-<n>` (granting `scopes`) unless `routes`
    /// answers it.
    fn stub_with(
        routes: Vec<(&'static str, &'static str, u16, Value)>,
        scopes: Value,
    ) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        std::thread::spawn(move || {
            let mut mints = 0;
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
                let routed = routes
                    .iter()
                    .find(|(m, path, _, _)| *m == method && *path == target)
                    .map(|(_, _, status, body)| (*status, body.to_string()));
                let (status, body) = match routed {
                    Some(answer) => answer,
                    None if method == "POST" && target == "/ws/sessions" => {
                        mints += 1;
                        let body = json!({
                            "token": format!("tok-{mints}"),
                            "expiresAt": unix_now() + 600,
                            "tokenType": "Bearer",
                            "scopes": scopes,
                        });
                        (200, body.to_string())
                    }
                    None => (404, r#"{"code":"not-found"}"#.to_string()),
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        (format!("http://{address}"), seen)
    }

    fn stub(
        routes: Vec<(&'static str, &'static str, u16, Value)>,
    ) -> (String, Arc<Mutex<Vec<String>>>) {
        stub_with(
            routes,
            json!(["sandbox:invoke", "sandbox:withdraw", "sandbox:swap"]),
        )
    }

    fn client(base: &str) -> SandboxClient {
        SandboxClient::from_values(
            Some(SANDBOX.into()),
            Some("a4_sk_agent".into()),
            Some(base.into()),
        )
    }

    fn requests(seen: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
        seen.lock().unwrap().clone()
    }

    fn body_of(request: &str) -> Value {
        serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap()
    }

    #[test]
    fn only_a_sandbox_id_enables_the_tools() {
        assert!(!SandboxClient::from_values(None, Some("k".into()), None).configured());
        assert!(!SandboxClient::from_values(Some("  ".into()), None, None).configured());
        assert!(!SandboxClient::from_values(Some("a4_sbx_x".into()), None, None).configured());
        assert!(SandboxClient::from_values(Some(format!(" {SANDBOX} ")), None, None).configured());
    }

    #[tokio::test]
    async fn the_api_key_mints_a_scoped_token_and_only_the_token_reaches_the_sandbox_routes() {
        let (base, seen) = stub(vec![
            (
                "GET",
                "/api/sandbox/v1/wallet",
                200,
                json!({ "walletAddress": "W" }),
            ),
            ("GET", "/api/sandbox/v1/rules", 200, json!({ "rules": [] })),
        ]);
        let client = client(&base);
        assert_eq!(client.status().await.unwrap()["walletAddress"], "W");
        client.rules().await.unwrap();
        let seen = requests(&seen);
        assert_eq!(seen.len(), 3, "one mint, reused: {seen:?}");
        let mint = seen[0].to_ascii_lowercase();
        assert!(mint.starts_with("post /ws/sessions"), "{mint}");
        assert!(mint.contains("authorization: bearer a4_sk_agent"), "{mint}");
        assert_eq!(
            body_of(&seen[0]),
            json!({ "targetKind": "sandbox", "targetId": SANDBOX })
        );
        for request in &seen[1..] {
            let request = request.to_ascii_lowercase();
            assert!(request.contains("authorization: bearer tok-1"), "{request}");
            assert!(!request.contains("a4_sk_agent"), "{request}");
        }
    }

    #[tokio::test]
    async fn an_expired_token_is_reminted_once_with_the_same_idempotency_key() {
        let (base, seen) = stub(vec![(
            "POST",
            "/api/sandbox/v1/actions/withdraw",
            401,
            json!({ "code": "sandbox-session-expired", "error": "expired" }),
        )]);
        let refusal = client(&base)
            .invoke(
                SandboxActionName::Withdraw,
                json!({ "lamports": "5" }),
                "key-7",
            )
            .await
            .unwrap_err();
        assert_eq!(refusal.body["code"], "sandbox-session-expired");
        let seen = requests(&seen);
        // mint, withdraw (401), mint, withdraw (401): no third attempt.
        assert_eq!(seen.len(), 4, "{seen:?}");
        assert!(seen[2].starts_with("POST /ws/sessions"));
        assert!(seen[3].to_ascii_lowercase().contains("bearer tok-2"));
        assert!(seen[3]
            .to_ascii_lowercase()
            .contains("idempotency-key: key-7"));
    }

    #[tokio::test]
    async fn a_renewed_token_must_still_grant_the_action() {
        // The first mint grants withdraw; after expiry the owner narrowed the
        // grant, and the renewed token grants nothing.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(Vec::<String>::new()));
        let log = seen.clone();
        std::thread::spawn(move || {
            let mut mints = 0;
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let mut buffer = [0u8; 8192];
                let read = stream.read(&mut buffer).unwrap_or(0);
                let head = String::from_utf8_lossy(&buffer[..read]).to_string();
                log.lock().unwrap().push(head.clone());
                let (status, body) = if head.starts_with("POST /ws/sessions") {
                    mints += 1;
                    let scopes = if mints == 1 {
                        json!(["sandbox:withdraw"])
                    } else {
                        json!([])
                    };
                    (
                        200,
                        json!({ "token": format!("tok-{mints}"), "expiresAt": unix_now() + 600, "scopes": scopes }),
                    )
                } else {
                    (
                        401,
                        json!({ "code": "sandbox-session-expired", "error": "expired" }),
                    )
                };
                let body = body.to_string();
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        let refusal = client(&format!("http://{address}"))
            .invoke(
                SandboxActionName::Withdraw,
                json!({ "lamports": "5" }),
                "k-1",
            )
            .await
            .unwrap_err();
        assert_eq!(refusal.body["code"], "sandbox-action-not-granted");
        let seen = requests(&seen);
        // mint, withdraw (401), mint: the retry is never sent.
        assert_eq!(seen.len(), 3, "{seen:?}");
        assert!(seen[2].starts_with("POST /ws/sessions"));
    }

    #[tokio::test]
    async fn a_revoked_session_is_not_reminted() {
        let (base, seen) = stub(vec![(
            "GET",
            "/api/sandbox/v1/wallet",
            401,
            json!({ "code": "sandbox-session-revoked", "error": "revoked" }),
        )]);
        let refusal = client(&base).status().await.unwrap_err();
        assert_eq!(refusal.body["code"], "sandbox-session-revoked");
        assert_eq!(requests(&seen).len(), 2);
    }

    #[tokio::test]
    async fn a_refused_mint_names_why_and_sends_nothing_else() {
        let (base, seen) = stub(vec![(
            "POST",
            "/ws/sessions",
            403,
            json!({ "code": "sandbox-grant-missing", "error": "not granted" }),
        )]);
        let refusal = client(&base).status().await.unwrap_err();
        let shown = refusal.to_json();
        assert_eq!(shown["code"], "sandbox-grant-missing");
        assert!(shown["hint"].as_str().unwrap().contains("owner"));
        assert_eq!(requests(&seen).len(), 1);
    }

    #[tokio::test]
    async fn without_an_api_key_nothing_is_sent() {
        let (base, seen) = stub(vec![]);
        let client = SandboxClient::from_values(Some(SANDBOX.into()), None, Some(base));
        let refusal = client.status().await.unwrap_err();
        assert_eq!(refusal.body["code"], "sandbox-agent-key-required");
        assert!(requests(&seen).is_empty());
    }

    #[tokio::test]
    async fn actions_go_to_their_api_routes_with_args_verbatim() {
        let (base, seen) = stub(vec![
            (
                "POST",
                "/api/sandbox/v1/actions/invoke",
                200,
                json!({ "status": "landed" }),
            ),
            (
                "POST",
                "/api/sandbox/v1/actions/withdraw",
                200,
                json!({ "status": "landed" }),
            ),
            (
                "POST",
                "/api/sandbox/v1/actions/swap",
                200,
                json!({ "status": "landed" }),
            ),
        ]);
        let client = client(&base);
        let invoke = json!({
            "instructions": [{
                "program": "P", "instruction": "buy",
                "args": { "amount": "5" }, "accounts": { "user": "W" },
                "remainingAccounts": [{ "address": "X", "writable": true }]
            }],
            "computeUnitLimit": 300000,
            "transactionFormat": "v0"
        });
        let withdraw = json!({ "lamports": "1000" });
        let swap =
            json!({ "inputMint": "So1", "outputMint": "M", "amount": "10", "slippageBps": 50 });
        for (action, args, key) in [
            (SandboxActionName::Invoke, &invoke, "k-1"),
            (SandboxActionName::Withdraw, &withdraw, "k-2"),
            (SandboxActionName::Swap, &swap, "k-3"),
        ] {
            let body = client.invoke(action, args.clone(), key).await.unwrap();
            assert_eq!(body["status"], "landed");
        }
        let seen = requests(&seen);
        assert_eq!(seen.len(), 4);
        for (request, (path, args, key)) in seen[1..].iter().zip([
            ("/api/sandbox/v1/actions/invoke", &invoke, "k-1"),
            ("/api/sandbox/v1/actions/withdraw", &withdraw, "k-2"),
            ("/api/sandbox/v1/actions/swap", &swap, "k-3"),
        ]) {
            assert!(request.starts_with(&format!("POST {path} ")), "{request}");
            assert!(request
                .to_ascii_lowercase()
                .contains(&format!("idempotency-key: {key}")));
            assert_eq!(&body_of(request), args);
        }
    }

    #[tokio::test]
    async fn previews_use_the_dry_run_routes_and_never_carry_a_key() {
        let (base, seen) = stub(vec![
            (
                "POST",
                "/api/sandbox/v1/actions/preview",
                200,
                json!({ "pass": true }),
            ),
            (
                "POST",
                "/api/sandbox/v1/actions/swap/preview",
                200,
                json!({ "pass": true }),
            ),
        ]);
        let client = client(&base);
        client
            .preview(SandboxActionName::Invoke, json!({ "instructions": [] }))
            .await
            .unwrap();
        client
            .preview(SandboxActionName::Swap, json!({ "amount": "1" }))
            .await
            .unwrap();
        let refusal = client
            .preview(SandboxActionName::Withdraw, json!({ "lamports": "1" }))
            .await
            .unwrap_err();
        assert_eq!(refusal.body["code"], "sandbox-preview-unavailable");
        let seen = requests(&seen);
        assert_eq!(
            seen.len(),
            3,
            "the withdraw preview sends nothing: {seen:?}"
        );
        for request in &seen[1..] {
            assert!(!request.to_ascii_lowercase().contains("idempotency-key"));
        }
    }

    #[tokio::test]
    async fn an_action_outside_the_grant_fails_closed_before_it_is_sent() {
        let (base, seen) = stub_with(vec![], json!(["sandbox:withdraw"]));
        let client = client(&base);
        let refusal = client
            .invoke(SandboxActionName::Swap, json!({}), "k-1")
            .await
            .unwrap_err();
        let shown = refusal.to_json();
        assert_eq!(shown["code"], "sandbox-action-not-granted");
        assert_eq!(shown["rule"], "session-grant");
        assert_eq!(shown["granted"], json!(["withdraw"]));
        let refusal = client
            .preview(SandboxActionName::Invoke, json!({}))
            .await
            .unwrap_err();
        assert_eq!(refusal.body["code"], "sandbox-action-not-granted");
        // Only the mint went out.
        assert_eq!(requests(&seen).len(), 1);
    }

    #[tokio::test]
    async fn a_mint_without_scopes_grants_nothing() {
        let (base, _) = stub_with(vec![], Value::Null);
        let refusal = client(&base)
            .invoke(SandboxActionName::Withdraw, json!({}), "k-1")
            .await
            .unwrap_err();
        assert_eq!(refusal.body["code"], "sandbox-action-not-granted");
    }

    #[test]
    fn action_names_and_args_are_checked() {
        let args: SandboxInvokeArgs = serde_json::from_value(json!({
            "action": "withdraw", "args": { "lamports": "1" }
        }))
        .unwrap();
        assert_eq!(args.action, SandboxActionName::Withdraw);
        assert!(
            serde_json::from_value::<SandboxInvokeArgs>(json!({ "action": "transfer" })).is_err()
        );
        assert_eq!(action_args(None).unwrap(), json!({}));
        assert!(action_args(Some(json!([1]))).is_err());
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
        let refusal = client(&base)
            .preview(SandboxActionName::Invoke, json!({}))
            .await
            .unwrap_err();
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
        // One mint and two lookups.
        assert_eq!(requests(&seen).len(), 3);
    }

    #[tokio::test]
    async fn credentials_go_only_to_arete_or_loopback_origins() {
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
        // A 200 whose body is not JSON (as when it is cut off), after a mint.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let mut buffer = [0u8; 8192];
                let read = stream.read(&mut buffer).unwrap_or(0);
                let head = String::from_utf8_lossy(&buffer[..read]).to_string();
                if head.starts_with("POST /ws/sessions") {
                    let body = json!({
                        "token": "tok", "expiresAt": unix_now() + 600, "scopes": ["sandbox:invoke"]
                    })
                    .to_string();
                    let _ = write!(
                        stream,
                        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    );
                } else {
                    let _ = write!(
                        stream,
                        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 7\r\nconnection: close\r\n\r\n{{\"actio"
                    );
                }
            }
        });
        let refusal = client(&format!("http://{address}"))
            .invoke(SandboxActionName::Invoke, json!({}), "key-9")
            .await
            .unwrap_err();
        assert_eq!(refusal.status, 200);
        assert_eq!(refusal.body["code"], "sandbox-response-unreadable");
        assert!(refusal.to_json()["hint"]
            .as_str()
            .unwrap()
            .contains("sandbox_action"));
    }

    /// A read that fails in transport says to retry it; it never talks
    /// about idempotency keys, which only an action has. An action that
    /// fails the same way says to look it up first.
    #[test]
    fn transport_failures_are_hinted_for_the_kind_of_call() {
        let unreachable = |kind| SandboxRefusal {
            status: 0,
            body: json!({
                "code": "sandbox-api-unreachable",
                "error": "The Arete API could not be reached: error sending request",
                "retryable": true,
            }),
            kind,
        };
        for kind in [CallKind::Read, CallKind::Preview] {
            let shown = unreachable(kind).to_json();
            let hint = shown["hint"].as_str().unwrap();
            assert!(!hint.contains("idempotency"), "{kind:?}: {hint}");
            assert!(!hint.contains("sandbox_action"), "{kind:?}: {hint}");
            assert!(hint.contains("retry the same call"), "{kind:?}: {hint}");
            assert_eq!(shown["retryable"], true);
        }
        let hint = unreachable(CallKind::Action).to_json()["hint"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(hint.contains("sandbox_action"), "{hint}");
        assert!(hint.contains("idempotencyKey"), "{hint}");
    }

    /// A refusal made here is final when the request was at fault, and
    /// retryable when the API's answer was: retryable and hint agree.
    #[test]
    fn local_refusals_are_retryable_only_for_an_unusable_answer() {
        let request = SandboxRefusal::local("sandbox-not-configured", "no id").to_json();
        assert_eq!(request["retryable"], false);
        let answer =
            SandboxRefusal::local("sandbox-response-unreadable", "no token minted").to_json();
        assert_eq!(answer["retryable"], true);
        assert!(answer["hint"]
            .as_str()
            .unwrap()
            .contains("retry the same call"));
    }

    #[tokio::test]
    async fn a_mint_without_a_token_is_retryable() {
        let (base, _) = stub(vec![("POST", "/ws/sessions", 200, json!({ "scopes": [] }))]);
        let shown = client(&base).status().await.unwrap_err().to_json();
        assert_eq!(shown["code"], "sandbox-response-unreadable");
        assert_eq!(shown["retryable"], true);
    }

    #[test]
    fn calls_are_read_preview_or_action() {
        use reqwest::Method;
        for (method, path, kind) in [
            (Method::GET, "/wallet", CallKind::Read),
            (Method::GET, "/rules", CallKind::Read),
            (Method::GET, "/actions?idempotencyKey=k", CallKind::Read),
            (Method::POST, "/derive", CallKind::Read),
            (Method::POST, "/actions/preview", CallKind::Preview),
            (Method::POST, "/actions/swap/preview", CallKind::Preview),
            (Method::POST, "/actions/call/preview", CallKind::Preview),
            (Method::POST, "/actions/invoke", CallKind::Action),
            (Method::POST, "/actions/withdraw", CallKind::Action),
            (Method::POST, "/actions/swap", CallKind::Action),
        ] {
            assert_eq!(CallKind::of(&method, path), kind, "{method} {path}");
        }
    }

    /// The agent sees `code`, `retryable` and `hint` whatever answered: a
    /// bare 504 from a proxy with no JSON body, or a 500 with `code: null`.
    #[tokio::test]
    async fn bare_5xx_answers_get_a_code_retryable_and_hint() {
        let (base, _) = stub(vec![
            ("POST", "/api/sandbox/v1/actions/preview", 504, Value::Null),
            (
                "GET",
                "/api/sandbox/v1/rules",
                500,
                json!({ "code": null, "error": "Internal server error", "retryable": true }),
            ),
        ]);
        let api = client(&base);
        let preview = api
            .preview(SandboxActionName::Invoke, json!({}))
            .await
            .unwrap_err()
            .to_json();
        assert_eq!(preview["status"], 504);
        assert_eq!(preview["code"], "sandbox-api-timeout");
        assert_eq!(preview["retryable"], true);
        assert!(preview["error"].as_str().unwrap().contains("504"));
        assert!(preview["hint"].as_str().unwrap().contains("dry run"));

        let rules = api.rules().await.unwrap_err().to_json();
        assert_eq!(rules["code"], "sandbox-internal-error");
        assert_eq!(rules["retryable"], true);
        assert_eq!(rules["error"], "Internal server error");
        let hint = rules["hint"].as_str().unwrap();
        assert!(!hint.contains("idempotency"), "{hint}");

        // With no answer at all, the bare status still gets the envelope.
        let bare = SandboxRefusal {
            status: 502,
            body: Value::Null,
            kind: CallKind::Action,
        }
        .to_json();
        assert_eq!(bare["code"], "sandbox-api-unavailable");
        assert_eq!(bare["retryable"], true);
        assert!(bare["hint"].as_str().unwrap().contains("sandbox_action"));
    }

    /// A hint the API sent is kept when this server has none for the code;
    /// this server's own (naming the MCP tools) wins when it has one.
    #[test]
    fn the_apis_hint_is_kept_unless_this_server_has_one() {
        let api_hint = SandboxRefusal {
            status: 403,
            body: json!({ "code": "sandbox-derive-mint-not-found", "retryable": false, "hint": "from the API" }),
            kind: CallKind::Read,
        }
        .to_json();
        assert_eq!(api_hint["hint"], "from the API");
        let ours = SandboxRefusal {
            status: 403,
            body: json!({
                "code": "sandbox-guard-insufficient-funds",
                "retryable": false,
                "balanceLamports": "0",
                "hint": "from the API",
            }),
            kind: CallKind::Preview,
        }
        .to_json();
        assert!(ours["hint"].as_str().unwrap().contains("fund the wallet"));
        // The same code marked as a token shortfall gets the token hint.
        let tokens = SandboxRefusal {
            status: 403,
            body: json!({
                "code": "sandbox-guard-insufficient-funds",
                "retryable": false,
                "shortfall": "token",
            }),
            kind: CallKind::Preview,
        }
        .to_json();
        let hint = tokens["hint"].as_str().unwrap();
        assert!(hint.contains("token"), "{hint}");
        assert!(!hint.contains("balanceLamports"), "{hint}");
        // Unmarked, a missing balance alone keeps the SOL hint.
        let unmarked = SandboxRefusal {
            status: 403,
            body: json!({ "code": "sandbox-guard-insufficient-funds", "retryable": false }),
            kind: CallKind::Preview,
        }
        .to_json();
        assert!(unmarked["hint"]
            .as_str()
            .unwrap()
            .contains("fund the wallet"));
    }

    /// A guard that could not decide: retry the same dry run, or an action
    /// with a new key (the old one is recorded as refused).
    #[test]
    fn guard_infrastructure_hints_follow_retryable_and_kind() {
        let refusal = |retryable, kind| {
            SandboxRefusal {
                status: if retryable { 503 } else { 403 },
                body: json!({ "code": "sandbox-guard-unavailable", "retryable": retryable }),
                kind,
            }
            .to_json()["hint"]
                .as_str()
                .unwrap()
                .to_string()
        };
        assert!(refusal(true, CallKind::Preview).contains("Retry the same call"));
        assert!(refusal(true, CallKind::Action).contains("new idempotencyKey"));
        assert!(refusal(false, CallKind::Preview).contains("will not pass"));
    }

    /// PDAs are derived here: the same answer as `solana
    /// find-program-derived-address`, and no request reaches the API (here,
    /// an origin with nothing listening).
    #[tokio::test]
    async fn pdas_are_derived_locally_without_the_api() {
        let api = client("http://127.0.0.1:9");
        let derive = |args: Value| {
            let args: SandboxDeriveArgs = serde_json::from_value(args).unwrap();
            let api = api.clone();
            async move { api.derive(&args).await }
        };
        const PUMP: &str = "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P";
        const USDC: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
        let global = derive(json!({
            "kind": "pda", "programId": PUMP,
            "seeds": [{ "kind": "string", "value": "global" }],
        }))
        .await
        .unwrap();
        assert_eq!(
            global,
            json!({ "address": "4wTV1YmiEkRvAtNtsSGPtUrqRYQMe5SKy2uB4Jjaxnjf", "bump": 255 })
        );
        let curve = derive(json!({
            "kind": "pda", "programId": PUMP,
            "seeds": [
                { "kind": "string", "value": "bonding-curve" },
                { "kind": "pubkey", "value": USDC },
            ],
        }))
        .await
        .unwrap();
        assert_eq!(
            curve,
            json!({ "address": "8Rr2Qo9ch94zRZxojHfg7Xeq8DfAp9mfNaLZnQScnpbA", "bump": 253 })
        );
        // Every seed kind, integers as numbers or decimal strings.
        let mixed = derive(json!({
            "kind": "pda", "programId": PUMP,
            "seeds": [
                { "kind": "string", "value": "x" },
                { "kind": "u8", "value": 7 },
                { "kind": "u16", "value": "513" },
                { "kind": "u32", "value": 70000 },
                { "kind": "u64", "value": "1234567890123" },
                { "kind": "bytes", "hex": "deadbeef" },
            ],
        }))
        .await
        .unwrap();
        assert_eq!(
            mixed,
            json!({ "address": "3QhyWoQ3n7GZ1xYunPYTi1orfEPqwndDuJVdgBw1creX", "bump": 255 })
        );
        // An ATA still needs the API, which is not there.
        let ata = derive(json!({ "kind": "ata", "mint": USDC }))
            .await
            .unwrap_err();
        assert_eq!(ata.body["code"], "sandbox-api-unreachable");
    }

    #[test]
    fn bad_pda_seeds_are_refused_locally() {
        const PUMP: &str = "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P";
        let seeds = |list: Value| list.as_array().unwrap().clone();
        for (program, list, needle) in [
            (None, json!([]), "programId"),
            (Some("not-an-address"), json!([]), "programId"),
            (
                Some(PUMP),
                json!([{ "kind": "u8", "value": 256 }]),
                "too large for u8",
            ),
            (
                Some(PUMP),
                json!([{ "kind": "u16", "value": "x" }]),
                "not a u64",
            ),
            (
                Some(PUMP),
                json!([{ "kind": "bytes", "hex": "abc" }]),
                "not hex",
            ),
            (
                Some(PUMP),
                json!([{ "kind": "pubkey", "value": "nope" }]),
                "base58",
            ),
            (
                Some(PUMP),
                json!([{ "kind": "float", "value": 1 }]),
                "seeds[0]",
            ),
            (
                Some(PUMP),
                json!([{ "kind": "string", "value": "y".repeat(33) }]),
                "longer than 32",
            ),
            (
                Some(PUMP),
                Value::Array(vec![json!({ "kind": "u8", "value": 1 }); 16]),
                "at most 15",
            ),
        ] {
            let refusal = derive_pda(program, &seeds(list.clone())).unwrap_err();
            let shown = refusal.to_json();
            assert_eq!(shown["code"], "sandbox-derive-invalid", "{list}");
            assert_eq!(shown["retryable"], false, "{list}");
            let error = shown["error"].as_str().unwrap();
            assert!(error.contains(needle), "{list}: {error}");
            assert!(shown["hint"].as_str().unwrap().contains("seeds"));
        }
    }

    #[tokio::test]
    async fn oversized_answers_are_refused_while_reading() {
        let big = json!({ "rules": "x".repeat(MAX_RESPONSE_BYTES + 10) });
        let (base, _) = stub(vec![("GET", "/api/sandbox/v1/rules", 200, big)]);
        let refusal = client(&base).rules().await.unwrap_err();
        assert_eq!(refusal.body["code"], "sandbox-response-too-large");
    }

    const ACTIONS: &str = "/api/sandbox/v1/sandboxes/6f1c2c43-6d3c-4a51-9b8e-0e3c1e2a7b10/actions";

    fn action_item(id: &str, status: &str, program: &str) -> Value {
        json!({
            "id": id, "createdAt": "2026-11-12T10:00:00Z", "status": status,
            "signature": format!("sig-{id}"), "slot": 1,
            "programs": [program], "instructions": [], "solDelta": "-5000",
            "tokenDeltas": [], "accounts": [], "raw": { "meta": {} }
        })
    }

    #[tokio::test]
    async fn an_unfiltered_history_read_is_one_page_verbatim() {
        let page = json!({ "items": [action_item("a1", "landed", "P")], "nextCursor": "c2" });
        let (base, seen) = stub(vec![(
            "GET",
            "/api/sandbox/v1/sandboxes/6f1c2c43-6d3c-4a51-9b8e-0e3c1e2a7b10/actions?limit=20",
            200,
            page.clone(),
        )]);
        let body = client(&base)
            .history(&ledger::HistoryQuery::default())
            .await
            .unwrap();
        assert_eq!(body, page);
        let seen = requests(&seen);
        assert_eq!(seen.len(), 2, "{seen:?}");
        let read = seen[1].to_ascii_lowercase();
        assert!(
            read.starts_with(&format!("get {ACTIONS}?limit=20 ")),
            "{read}"
        );
        assert!(read.contains("authorization: bearer tok-1"), "{read}");
        assert!(!read.contains("a4_sk_agent"), "{read}");
    }

    #[tokio::test]
    async fn a_filtered_history_read_scans_pages_until_enough_match() {
        let (base, seen) = stub(vec![
            (
                "GET",
                "/api/sandbox/v1/sandboxes/6f1c2c43-6d3c-4a51-9b8e-0e3c1e2a7b10/actions?limit=2&cursor=c1",
                200,
                json!({
                    "items": [action_item("a1", "failed", "Pump"), action_item("a2", "landed", "Pump")],
                    "nextCursor": "c2"
                }),
            ),
            (
                "GET",
                "/api/sandbox/v1/sandboxes/6f1c2c43-6d3c-4a51-9b8e-0e3c1e2a7b10/actions?limit=1&cursor=c2",
                200,
                json!({ "items": [action_item("a3", "landed", "Other")], "nextCursor": "c3" }),
            ),
            (
                "GET",
                "/api/sandbox/v1/sandboxes/6f1c2c43-6d3c-4a51-9b8e-0e3c1e2a7b10/actions?limit=1&cursor=c3",
                200,
                json!({ "items": [action_item("a4", "landed", "Pump")], "nextCursor": "c4" }),
            ),
        ]);
        let query = ledger::HistoryQuery {
            cursor: Some("c1".into()),
            limit: Some(2),
            status: Some("landed".into()),
            program: Some("pump".into()),
            ..Default::default()
        };
        let body = client(&base).history(&query).await.unwrap();
        let ids: Vec<&str> = body["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, vec!["a2", "a4"]);
        assert_eq!(body["nextCursor"], "c4");
        assert_eq!(body["pagesRead"], 3);
        // A mint, then pages of 2, 1 and 1: never more than still wanted.
        assert_eq!(requests(&seen).len(), 4);
    }

    #[tokio::test]
    async fn a_filtered_history_read_ends_on_the_last_page() {
        let (base, seen) = stub(vec![(
            "GET",
            "/api/sandbox/v1/sandboxes/6f1c2c43-6d3c-4a51-9b8e-0e3c1e2a7b10/actions?limit=20",
            200,
            json!({ "items": [action_item("a1", "failed", "P")], "nextCursor": null }),
        )]);
        let query = ledger::HistoryQuery {
            status: Some("landed".into()),
            ..Default::default()
        };
        let body = client(&base).history(&query).await.unwrap();
        assert_eq!(body["items"], json!([]));
        assert!(body["nextCursor"].is_null());
        assert_eq!(requests(&seen).len(), 2);
    }

    #[tokio::test]
    async fn a_history_page_without_items_is_an_error() {
        let (base, _) = stub(vec![(
            "GET",
            "/api/sandbox/v1/sandboxes/6f1c2c43-6d3c-4a51-9b8e-0e3c1e2a7b10/actions?limit=20",
            200,
            json!({ "actions": [] }),
        )]);
        let refusal = client(&base)
            .history(&ledger::HistoryQuery::default())
            .await
            .unwrap_err();
        assert_eq!(refusal.body["code"], "sandbox-response-unreadable");
    }

    #[tokio::test]
    async fn holdings_are_read_with_the_sandbox_token() {
        let holdings = json!({
            "sol": "1500000000",
            "tokens": [{ "mint": "M", "amount": "10", "decimals": 6, "acquiredBySandbox": true }],
            "positions": [], "asOfSlot": 42
        });
        let (base, seen) = stub(vec![(
            "GET",
            "/api/sandbox/v1/sandboxes/6f1c2c43-6d3c-4a51-9b8e-0e3c1e2a7b10/holdings",
            200,
            holdings.clone(),
        )]);
        assert_eq!(client(&base).holdings().await.unwrap(), holdings);
        let seen = requests(&seen);
        assert!(seen[1].to_ascii_lowercase().contains("bearer tok-1"));
    }

    #[test]
    fn history_args_parse_into_a_query() {
        let args: SandboxHistoryArgs = serde_json::from_value(json!({
            "cursor": " c ", "limit": 5, "status": "", "since": "2026-11-15T00:00:00+01:00"
        }))
        .unwrap();
        let query = args.query().unwrap();
        assert_eq!(query.cursor.as_deref(), Some("c"));
        assert_eq!(query.limit(), 5);
        assert!(query.status.is_none());
        assert_eq!(
            query.since.unwrap().to_rfc3339(),
            "2026-11-14T23:00:00+00:00"
        );
        let bad: SandboxHistoryArgs =
            serde_json::from_value(json!({ "since": "yesterday" })).unwrap();
        assert!(bad.query().unwrap_err().contains("RFC 3339"));
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
