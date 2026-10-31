//! Arete cloud dev environments: the client `a4 dev`, the dev UI server and
//! the MCP `dev_*` tools share.
//!
//! A cloud dev environment is a persistent mainnet fork that Arete hosts for
//! one account. Two endpoints reach it:
//!
//! - **The Arete API** (`<origin>/api/dev/...`, the account's secret key)
//!   manages environments: create, list, read, wake, sleep, delete,
//!   checkpoints, branch, restore, rebase, reset, the lifecycle event feed,
//!   quotas, and short-lived environment tokens ([`EnvApi`]).
//! - **The environment's RPC endpoint** (the URL a token names, with the
//!   token as a bearer credential) answers Solana JSON-RPC reads, simulations
//!   and sends, account subscriptions over WebSocket, and the typed
//!   `devenv_*` methods: fund, token balances, clock warps, pins, follows,
//!   scenarios, previews and repro exports ([`EnvRpc`]).
//!
//! Tokens last about 15 minutes; [`EnvRpc`] mints them from the API and
//! renews them before they expire. Keys and tokens are only ever sent as
//! bearer credentials; nothing here returns or logs them.
//!
//! A project remembers its environment in `.arete/dev/environment.json`
//! ([`EnvSelection`]), which `a4 dev up` writes.

use std::path::Path;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::credentials;

/// Schema of `.arete/dev/environment.json`.
pub const SELECTION_SCHEMA: &str = "arete.dev-environment/v1";
/// The environment to use when a command names none (a name or an id).
pub const ENV_VAR_ENVIRONMENT: &str = "ARETE_DEV_ENV";
/// Overrides the RPC origin of environments (`<origin>/rpc/v1/devenv/<id>`),
/// for an API that names none.
pub const ENV_VAR_GATEWAY_URL: &str = "ARETE_DEV_GATEWAY_URL";
/// Prefix of environment ids.
pub const ENV_ID_PREFIX: &str = "denv_";
/// Prefix of checkpoint ids.
pub const CHECKPOINT_ID_PREFIX: &str = "ckpt_";
/// The most lamports one `fund` call may set (1,000,000 SOL).
pub const MAX_FUND_LAMPORTS: u64 = 1_000_000 * LAMPORTS_PER_SOL;
pub const LAMPORTS_PER_SOL: u64 = 1_000_000_000;
/// Renew a token when it has less than this left (seconds).
const TOKEN_RENEW_BEFORE: i64 = 60;
/// How often a call is retried while a sleeping environment wakes.
const WAKE_RETRIES: usize = 4;
/// The longest `Retry-After` a call waits out.
const MAX_RETRY_AFTER: Duration = Duration::from_secs(10);
const DEFAULT_API_ORIGIN: &str = "https://api.arete.run";

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// An error answer from the Arete API (`{error, code}`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiFailure {
    pub status: u16,
    pub code: Option<String>,
    pub message: String,
}

impl std::fmt::Display for ApiFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.code {
            Some(code) => write!(f, "{} ({code}, HTTP {})", self.message, self.status),
            None => write!(f, "{} (HTTP {})", self.message, self.status),
        }
    }
}

impl std::error::Error for ApiFailure {}

impl ApiFailure {
    /// The API answered with a code, or a status, that says what to do next.
    pub fn hint(&self) -> Option<&'static str> {
        match self.code.as_deref() {
            Some("devenv-environment-quota") => Some(
                "Your plan's environment limit is reached: reuse one with `a4 dev up --env <name>` (`a4 dev list` shows them) or delete one with `a4 dev delete --env <name>`.",
            ),
            Some("devenv-checkpoint-quota") => Some(
                "Your plan's checkpoint limit is reached: delete one with `a4 dev checkpoint --delete <id or name>`.",
            ),
            Some("devenv-active-hours-exhausted") => Some(
                "This month's environment hours on your plan are used up.",
            ),
            Some("devenv-busy") => Some("Another operation is running on this environment; try again in a moment."),
            Some("devenv-not-found") => Some(
                "The environment does not exist (or was deleted); `a4 dev list` shows yours, `a4 dev up` creates one.",
            ),
            _ if self.status == 503 => Some(
                "Arete could not start or reach the environment right now; try again shortly.",
            ),
            _ => None,
        }
    }
}

/// An error answer from the environment's RPC endpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct RpcFailure {
    pub method: String,
    pub status: u16,
    pub code: Option<i64>,
    pub message: String,
    pub data: Option<Value>,
}

impl std::fmt::Display for RpcFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} failed: {}", self.method, self.message)?;
        if let Some(code) = self.code {
            write!(f, " (code {code})")?;
        }
        if let Some(text) = self.data.as_ref().and_then(Value::as_str) {
            write!(f, ": {text}")?;
        }
        Ok(())
    }
}

impl std::error::Error for RpcFailure {}

// ---------------------------------------------------------------------------
// Names and ids
// ---------------------------------------------------------------------------

/// Whether `value` looks like an environment id (`denv_` and 26 lowercase
/// base32 characters).
pub fn is_env_id(value: &str) -> bool {
    value.strip_prefix(ENV_ID_PREFIX).is_some_and(|body| {
        body.len() == 26
            && body
                .bytes()
                .all(|byte| byte.is_ascii_digit() || byte.is_ascii_lowercase())
    })
}

/// Whether `name` is a valid environment name: 1–63 lowercase letters,
/// digits or `-`.
pub fn valid_env_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 63
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

/// A valid environment name derived from `text` (a project or directory
/// name): lowercased, other characters as `-`, trimmed to 63.
pub fn env_name_from(text: &str) -> String {
    let mut name = String::new();
    for character in text.chars() {
        let mapped = if character.is_ascii_alphanumeric() {
            character.to_ascii_lowercase()
        } else {
            '-'
        };
        if mapped == '-' && (name.is_empty() || name.ends_with('-')) {
            continue;
        }
        name.push(mapped);
    }
    let mut name: String = name.trim_end_matches('-').chars().take(63).collect();
    while name.ends_with('-') {
        name.pop();
    }
    if name.is_empty() {
        "dev".to_string()
    } else {
        name
    }
}

/// Whether `value` is a canonical base58 public key (32 bytes).
pub fn is_pubkey(value: &str) -> bool {
    bs58::decode(value)
        .into_vec()
        .is_ok_and(|bytes| bytes.len() == 32)
}

/// Lamports from a SOL amount (`1.5`) with at most 9 decimals.
pub fn sol_to_lamports(sol: &str) -> Result<u64> {
    let sol = sol.trim();
    let (whole, fraction) = sol.split_once('.').unwrap_or((sol, ""));
    if whole.is_empty() && fraction.is_empty()
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
        || fraction.len() > 9
    {
        bail!("'{sol}' is not a SOL amount (e.g. 10 or 0.5)");
    }
    let whole: u64 = if whole.is_empty() {
        0
    } else {
        whole.parse().map_err(|_| anyhow!("'{sol}' is too large"))?
    };
    let fraction: u64 = format!("{fraction:0<9}").parse().unwrap_or(0);
    let lamports = whole
        .checked_mul(LAMPORTS_PER_SOL)
        .and_then(|lamports| lamports.checked_add(fraction))
        .ok_or_else(|| anyhow!("'{sol}' is too large"))?;
    if lamports > MAX_FUND_LAMPORTS {
        bail!("At most 1,000,000 SOL per fund");
    }
    Ok(lamports)
}

// ---------------------------------------------------------------------------
// The project's selection
// ---------------------------------------------------------------------------

/// `.arete/dev/environment.json`: the environment a project develops
/// against. No key or token is stored here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvSelection {
    pub schema: String,
    pub id: String,
    pub name: String,
    /// The Arete API origin the environment belongs to.
    pub api_url: String,
}

impl EnvSelection {
    pub fn new(id: &str, name: &str, api_url: &str) -> Self {
        Self {
            schema: SELECTION_SCHEMA.into(),
            id: id.into(),
            name: name.into(),
            api_url: api_origin(api_url),
        }
    }

    pub fn load(path: &Path) -> Result<Option<Self>> {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(error).with_context(|| format!("Failed to read {}", path.display()))
            }
        };
        let selection: Self = serde_json::from_slice(&bytes)
            .with_context(|| format!("{} is not an environment selection", path.display()))?;
        if selection.schema != SELECTION_SCHEMA {
            bail!(
                "{} has schema '{}', expected '{SELECTION_SCHEMA}'",
                path.display(),
                selection.schema
            );
        }
        if !is_env_id(&selection.id) {
            bail!(
                "{} names '{}', which is not an environment id",
                path.display(),
                selection.id
            );
        }
        Ok(Some(selection))
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        crate::dev::write_private(path, &serde_json::to_vec_pretty(self)?)
    }
}

/// `url` as an API origin: no trailing slash and no `/api` suffix.
pub fn api_origin(url: &str) -> String {
    url.trim()
        .trim_end_matches('/')
        .trim_end_matches("/api")
        .trim_end_matches('/')
        .to_string()
}

/// The API origin this process talks to: `ARETE_API_URL`, else Arete's.
pub fn configured_api_origin() -> String {
    std::env::var("ARETE_API_URL")
        .ok()
        .map(|url| api_origin(&credentials::normalize_api_url(&url)))
        .filter(|url| !url.is_empty())
        .unwrap_or_else(|| DEFAULT_API_ORIGIN.to_string())
}

/// Whether credentials may be sent to `url`: HTTPS, or HTTP on loopback.
pub fn is_secure_or_loopback(url: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return false;
    };
    match parsed.scheme() {
        "https" | "wss" => parsed.host_str().is_some(),
        "http" | "ws" => crate::dev::is_loopback_url(url),
        _ => false,
    }
}

/// The WebSocket URL of an environment's RPC URL (same path).
pub fn ws_url_of(rpc_url: &str) -> String {
    if let Some(rest) = rpc_url.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = rpc_url.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        rpc_url.to_string()
    }
}

// ---------------------------------------------------------------------------
// The API
// ---------------------------------------------------------------------------

/// The environment routes of one Arete API, with the account's secret key.
#[derive(Clone)]
pub struct EnvApi {
    http: reqwest::Client,
    origin: String,
    key: String,
}

impl std::fmt::Debug for EnvApi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnvApi")
            .field("origin", &self.origin)
            .finish_non_exhaustive()
    }
}

impl EnvApi {
    /// `origin` must be an Arete origin (or loopback) so the key never goes
    /// anywhere else. The caller vouches that `key` may go to `origin`; an
    /// ambient key goes through [`Self::for_origin`].
    pub fn new(http: reqwest::Client, origin: &str, key: &str) -> Result<Self> {
        let origin = api_origin(origin);
        if !crate::registry::is_arete_origin(&origin) {
            bail!(
                "`{origin}` is not a recognised Arete origin (or is a non-loopback host over plain HTTP), so no credential will be sent to it. Point ARETE_API_URL at https://api.arete.run or a loopback API."
            );
        }
        if key.trim().is_empty() {
            bail!("No Arete API key");
        }
        Ok(Self {
            http,
            origin,
            key: key.trim().to_string(),
        })
    }

    /// The API this process is configured for (`ARETE_API_URL`), with the
    /// key from `ARETE_API_KEY` or the a4 login.
    pub fn from_environment(http: reqwest::Client) -> Result<Self> {
        Self::for_origin(http, &configured_api_origin())
    }

    /// `origin` with the key the a4 login (or `ARETE_API_KEY`) has for it.
    /// `origin` may come from a project file, so the key must be one that
    /// may go there ([`credentials::check_key_destination`]).
    pub fn for_origin(http: reqwest::Client, origin: &str) -> Result<Self> {
        let origin = api_origin(origin);
        let key = credentials::resolve_key_for_api(&origin)?.ok_or_else(|| {
            anyhow!(
                "Cloud dev environments need an Arete secret key for {origin}: run `a4 auth login` (or `a4 auth signup`), or set ARETE_API_KEY."
            )
        })?;
        credentials::check_key_destination(&key, &origin)?;
        Self::new(http, &origin, &key.key)
    }

    pub fn origin(&self) -> &str {
        &self.origin
    }

    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }

    /// The account's key, for the protocol API calls that go with it.
    pub fn key(&self) -> &str {
        &self.key
    }

    async fn call(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value> {
        self.call_leased(method, path, body, None).await
    }

    /// [`Self::call`] with the caller's environment lease generation in
    /// [`LEASE_HEADER`].
    async fn call_leased(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
        lease: Option<i64>,
    ) -> Result<Value> {
        self.call_at_leased(method, &format!("/api{path}"), body, lease)
            .await
    }

    /// `call` for a path under the origin itself (`/ws/sessions`).
    async fn call_at(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value> {
        self.call_at_leased(method, path, body, None).await
    }

    async fn call_at_leased(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
        lease: Option<i64>,
    ) -> Result<Value> {
        let url = format!("{}{path}", self.origin);
        let mut request = self
            .http
            .request(method.clone(), &url)
            .bearer_auth(&self.key)
            .timeout(Duration::from_secs(150));
        if let Some(generation) = lease {
            request = request.header(LEASE_HEADER, generation.to_string());
        }
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request
            .send()
            .await
            .with_context(|| format!("The Arete API is unreachable at {}", self.origin))?;
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        let parsed: Value = if text.trim().is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&text).unwrap_or_else(|_| json!({ "error": text }))
        };
        if status.is_success() {
            return Ok(parsed);
        }
        let code = parsed["code"].as_str().map(str::to_string);
        let mut message = parsed["error"]
            .as_str()
            .or(parsed["message"].as_str())
            .unwrap_or("request failed")
            .to_string();
        if status == reqwest::StatusCode::NOT_FOUND && code.is_none() {
            message = format!(
                "{method} {path} is not served: cloud dev environments are not enabled on {}",
                self.origin
            );
        }
        Err(ApiFailure {
            status: status.as_u16(),
            code,
            message,
        }
        .into())
    }

    async fn get(&self, path: &str) -> Result<Value> {
        self.call(reqwest::Method::GET, path, None).await
    }

    async fn post(&self, path: &str, body: Value) -> Result<Value> {
        self.call(reqwest::Method::POST, path, Some(body)).await
    }

    fn env_path(env: &str, rest: &str) -> Result<String> {
        if !is_env_id(env) {
            bail!("'{env}' is not an environment id");
        }
        Ok(format!("/dev/environments/{env}{rest}"))
    }

    /// The account's environments (not deleted).
    pub async fn list(&self) -> Result<Vec<Value>> {
        let body = self.get("/dev/environments").await?;
        Ok(body["environments"].as_array().cloned().unwrap_or_default())
    }

    /// Create (and start) an environment.
    pub async fn create(&self, name: &str, template: Option<&str>) -> Result<Value> {
        if !valid_env_name(name) {
            bail!("Environment names are 1-63 lowercase letters, digits or '-' (got '{name}')");
        }
        let mut body = json!({ "name": name });
        if let Some(template) = template {
            body["template"] = json!(template);
        }
        self.post("/dev/environments", body).await
    }

    pub async fn get_env(&self, env: &str) -> Result<Value> {
        self.get(&Self::env_path(env, "")?).await
    }

    /// The environment `reference` names: an id, or a name among the
    /// account's environments.
    pub async fn find(&self, reference: &str) -> Result<Option<Value>> {
        if is_env_id(reference) {
            return match self.get_env(reference).await {
                Ok(env) => Ok(Some(env)),
                Err(error) if is_not_found(&error) => Ok(None),
                Err(error) => Err(error),
            };
        }
        Ok(self
            .list()
            .await?
            .into_iter()
            .find(|env| env["name"].as_str() == Some(reference)))
    }

    pub async fn delete(&self, env: &str) -> Result<()> {
        self.call(reqwest::Method::DELETE, &Self::env_path(env, "")?, None)
            .await
            .map(|_| ())
    }

    pub async fn wake(&self, env: &str) -> Result<Value> {
        self.post(&Self::env_path(env, "/wake")?, json!({})).await
    }

    pub async fn sleep(&self, env: &str) -> Result<Value> {
        self.post(&Self::env_path(env, "/sleep")?, json!({})).await
    }

    pub async fn checkpoint(&self, env: &str, name: Option<&str>) -> Result<Value> {
        let body = match name {
            Some(name) => json!({ "name": name }),
            None => json!({}),
        };
        self.post(&Self::env_path(env, "/checkpoints")?, body).await
    }

    pub async fn checkpoints(&self, env: &str) -> Result<Vec<Value>> {
        let body = self.get(&Self::env_path(env, "/checkpoints")?).await?;
        Ok(body["checkpoints"].as_array().cloned().unwrap_or_default())
    }

    /// Delete a checkpoint by id or name.
    pub async fn delete_checkpoint(&self, env: &str, checkpoint: &str) -> Result<()> {
        let id = if checkpoint.starts_with(CHECKPOINT_ID_PREFIX) {
            checkpoint.to_string()
        } else {
            self.checkpoints(env)
                .await?
                .into_iter()
                .find(|item| item["name"].as_str() == Some(checkpoint))
                .and_then(|item| item["id"].as_str().map(str::to_string))
                .ok_or_else(|| anyhow!("No checkpoint named '{checkpoint}'"))?
        };
        self.call(
            reqwest::Method::DELETE,
            &Self::env_path(env, &format!("/checkpoints/{id}"))?,
            None,
        )
        .await
        .map(|_| ())
    }

    /// A new environment `name` from `checkpoint` (an id or a name).
    pub async fn branch(&self, env: &str, checkpoint: &str, name: &str) -> Result<Value> {
        if !valid_env_name(name) {
            bail!("Environment names are 1-63 lowercase letters, digits or '-' (got '{name}')");
        }
        self.post(
            &Self::env_path(env, "/branch")?,
            json!({ "checkpoint": checkpoint, "name": name }),
        )
        .await
    }

    pub async fn restore(&self, env: &str, checkpoint: &str) -> Result<Value> {
        self.post(
            &Self::env_path(env, "/restore")?,
            json!({ "checkpoint": checkpoint }),
        )
        .await
    }

    /// Refresh every account the environment never wrote (and did not pin)
    /// from mainnet; `keep` adds accounts to leave alone this time.
    pub async fn rebase(&self, env: &str, keep: &[String]) -> Result<Value> {
        self.post(&Self::env_path(env, "/rebase")?, json!({ "pinned": keep }))
            .await
    }

    pub async fn reset(&self, env: &str) -> Result<Value> {
        self.post(&Self::env_path(env, "/reset")?, json!({})).await
    }

    pub async fn token(&self, env: &str) -> Result<Value> {
        self.post(&Self::env_path(env, "/token")?, json!({})).await
    }

    /// Lifecycle events after `after` (an event id).
    pub async fn events(&self, env: &str, after: Option<i64>, limit: Option<u32>) -> Result<Value> {
        let mut path = Self::env_path(env, "/events")?;
        let mut query = Vec::new();
        if let Some(after) = after {
            query.push(format!("after={after}"));
        }
        if let Some(limit) = limit {
            query.push(format!("limit={limit}"));
        }
        if !query.is_empty() {
            path = format!("{path}?{}", query.join("&"));
        }
        self.get(&path).await
    }

    pub async fn quotas(&self) -> Result<Value> {
        self.get("/dev/quotas").await
    }

    /// The stack deployments loaded into the environment.
    pub async fn stacks(&self, env: &str) -> Result<Value> {
        self.get(&Self::env_path(env, "/stacks")?).await
    }

    /// Load a published registry stack (its StackManifest, at `live_alias`
    /// when it has several) into the environment. `status` in the answer is
    /// `loaded` or `already-loaded`.
    pub async fn load_registry_stack(
        &self,
        env: &str,
        stack_manifest_hash: &str,
        live_alias: Option<&str>,
    ) -> Result<Value> {
        let mut body = json!({ "stackManifestHash": stack_manifest_hash });
        if let Some(alias) = live_alias {
            body["liveAlias"] = json!(alias);
        }
        self.post(&Self::env_path(env, "/stacks")?, body).await
    }

    /// The environment's targets for installed identities: `body` is a
    /// resolve request, the answer has one target or refusal per identity.
    pub async fn resolve(&self, env: &str, body: Value) -> Result<Value> {
        self.post(&Self::env_path(env, "/resolve")?, body).await
    }

    /// Mint a session token at `/ws/sessions` (`body` names the target,
    /// its scopes and, for an environment, `environment`).
    pub async fn session(&self, body: Value) -> Result<Value> {
        self.call_at(reqwest::Method::POST, "/ws/sessions", Some(body))
            .await
    }

    /// Take the environment's lease for `project_id` (`force` takes it from
    /// another project). `None` when this API has no environment leases.
    pub async fn take_lease(
        &self,
        env: &str,
        project_id: &str,
        holder: &str,
        force: bool,
    ) -> Result<Option<Lease>> {
        let answer = self
            .post(
                &Self::env_path(env, "/lease")?,
                json!({ "projectId": project_id, "holder": holder, "force": force }),
            )
            .await;
        match answer {
            Ok(answer) => Lease::from_answer(&answer).map(Some),
            Err(error) if is_unserved(&error) => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Renew lease `generation`; `devenv-lease-lost` once another take
    /// happened.
    pub async fn renew_lease(
        &self,
        env: &str,
        project_id: &str,
        holder: &str,
        generation: i64,
    ) -> Result<Lease> {
        let answer = self
            .post(
                &Self::env_path(env, "/lease")?,
                json!({ "projectId": project_id, "holder": holder, "leaseGeneration": generation }),
            )
            .await?;
        Lease::from_answer(&answer)
    }

    /// Release lease `generation` (a stale one releases nothing).
    pub async fn release_lease(&self, env: &str, generation: i64) -> Result<()> {
        self.call_leased(
            reqwest::Method::DELETE,
            &Self::env_path(env, "/lease")?,
            None,
            Some(generation),
        )
        .await
        .map(|_| ())
    }

    /// The environment's lease (`lease: null` when never leased). `None`
    /// when this API has no environment leases.
    pub async fn lease(&self, env: &str) -> Result<Option<Value>> {
        match self.get(&Self::env_path(env, "/lease")?).await {
            Ok(answer) => Ok(Some(answer["lease"].clone())),
            Err(error) if is_unserved(&error) => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// The environment's overrides (`{kind, key, target, ...}`): program
    /// releases and stack aliases the environment serves in place of the
    /// installed ones.
    pub async fn overrides(&self, env: &str) -> Result<Vec<Value>> {
        overrides_of(self.get(&Self::env_path(env, "/overrides")?).await?)
    }
}


/// The `overrides` array of an overrides answer. Any other shape is an
/// error, never an empty list: the deploy check would read it as clean.
fn overrides_of(answer: Value) -> Result<Vec<Value>> {
    match answer {
        Value::Object(mut answer) => match answer.remove("overrides") {
            Some(Value::Array(overrides)) => Ok(overrides),
            _ => bail!("The environment's overrides answer has no `overrides` list"),
        },
        _ => bail!("The environment's overrides answer is not a JSON object"),
    }
}


/// The request header that carries an environment lease generation on the
/// writes it fences.
pub const LEASE_HEADER: &str = "Arete-Dev-Lease-Generation";

/// `409`: another project holds the environment.
pub const LEASED: &str = "devenv-environment-leased";
/// `409`: the environment was taken over; the write was refused.
pub const LEASE_LOST: &str = "devenv-lease-lost";

/// An environment lease this process holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lease {
    pub generation: i64,
    pub holder: String,
    pub ttl_seconds: u64,
    /// The holder a take replaced, when it was another project.
    pub replaced: Option<String>,
}

impl Lease {
    fn from_answer(answer: &Value) -> Result<Self> {
        let lease = &answer["lease"];
        Ok(Self {
            generation: lease["leaseGeneration"]
                .as_i64()
                .ok_or_else(|| anyhow!("The lease answer has no leaseGeneration: {answer}"))?,
            holder: lease["holder"].as_str().unwrap_or_default().to_string(),
            ttl_seconds: lease["ttlSeconds"].as_u64().unwrap_or(60),
            replaced: answer["replaced"]["holder"].as_str().map(str::to_string),
        })
    }
}

/// The route is not served by this API (an older one): a 404 without a code.
fn is_unserved(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<ApiFailure>()
        .is_some_and(|failure| failure.status == 404 && failure.code.is_none())
}

/// Whether `error` is the API's "not found".
pub fn is_not_found(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<ApiFailure>()
        .is_some_and(|failure| failure.status == 404)
}

/// The API's error code, if `error` is an API answer.
pub fn api_code(error: &anyhow::Error) -> Option<&str> {
    error
        .downcast_ref::<ApiFailure>()
        .and_then(|failure| failure.code.as_deref())
}

// ---------------------------------------------------------------------------
// The environment's RPC endpoint
// ---------------------------------------------------------------------------

/// A minted environment token.
#[derive(Clone, PartialEq, Eq)]
pub struct EnvToken {
    pub token: String,
    /// Unix seconds.
    pub expires_at: i64,
    /// The environment's JSON-RPC URL (`POST`; the same path upgrades to a
    /// WebSocket).
    pub rpc_url: String,
}

impl std::fmt::Debug for EnvToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnvToken")
            .field("expires_at", &self.expires_at)
            .field("rpc_url", &self.rpc_url)
            .finish_non_exhaustive()
    }
}

/// The RPC URL of `env` under a gateway origin.
pub fn gateway_rpc_url(origin: &str, env: &str) -> String {
    format!("{}/rpc/v1/devenv/{env}", origin.trim_end_matches('/'))
}

/// One environment's RPC endpoint, with tokens minted (and renewed) from
/// the API.
pub struct EnvRpc {
    api: EnvApi,
    env: String,
    gateway_override: Option<String>,
    token: tokio::sync::Mutex<Option<EnvToken>>,
}

impl std::fmt::Debug for EnvRpc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnvRpc")
            .field("env", &self.env)
            .finish_non_exhaustive()
    }
}

impl Clone for EnvRpc {
    fn clone(&self) -> Self {
        let token = self.token.try_lock().ok().and_then(|token| token.clone());
        Self {
            api: self.api.clone(),
            env: self.env.clone(),
            gateway_override: self.gateway_override.clone(),
            token: tokio::sync::Mutex::new(token),
        }
    }
}

impl EnvRpc {
    pub fn new(api: EnvApi, env: &str) -> Result<Self> {
        if !is_env_id(env) {
            bail!("'{env}' is not an environment id");
        }
        let gateway_override = std::env::var(ENV_VAR_GATEWAY_URL)
            .ok()
            .map(|url| url.trim().trim_end_matches('/').to_string())
            .filter(|url| !url.is_empty());
        if let Some(url) = &gateway_override {
            if !is_secure_or_loopback(url) {
                bail!("{ENV_VAR_GATEWAY_URL} must be an https URL (or loopback http)");
            }
        }
        Ok(Self {
            api,
            env: env.to_string(),
            gateway_override,
            token: tokio::sync::Mutex::new(None),
        })
    }

    pub fn env(&self) -> &str {
        &self.env
    }

    pub fn api(&self) -> &EnvApi {
        &self.api
    }

    /// A token with at least a minute left: the cached one, or a new one.
    pub async fn token(&self, force_refresh: bool) -> Result<EnvToken> {
        let mut cached = self.token.lock().await;
        if let Some(token) = cached.as_ref() {
            if !force_refresh
                && token.expires_at > crate::dev::unix_now() as i64 + TOKEN_RENEW_BEFORE
            {
                return Ok(token.clone());
            }
        }
        let body = self.api.token(&self.env).await?;
        let token = body["token"]
            .as_str()
            .filter(|token| !token.is_empty())
            .ok_or_else(|| anyhow!("The Arete API minted no environment token"))?
            .to_string();
        let expires_at = body["expiresAt"]
            .as_i64()
            .unwrap_or(crate::dev::unix_now() as i64 + 600);
        let rpc_url = match (&self.gateway_override, body["rpcUrl"].as_str()) {
            (Some(origin), _) => gateway_rpc_url(origin, &self.env),
            (None, Some(url)) if !url.is_empty() => url.trim_end_matches('/').to_string(),
            (None, _) => bail!(
                "The Arete API did not name an RPC endpoint for this environment; set {ENV_VAR_GATEWAY_URL} to the dev gateway's origin"
            ),
        };
        if !is_secure_or_loopback(&rpc_url) {
            bail!("The environment's RPC URL `{rpc_url}` is neither https nor loopback; refusing to send a token to it");
        }
        let token = EnvToken {
            token,
            expires_at,
            rpc_url,
        };
        *cached = Some(token.clone());
        Ok(token)
    }

    /// The environment's JSON-RPC URL.
    pub async fn rpc_url(&self) -> Result<String> {
        Ok(self.token(false).await?.rpc_url)
    }

    /// POST one JSON-RPC body (a request or a batch) with a fresh token,
    /// renewing it on a 401 and waiting out a wake (503 with
    /// `Retry-After`). Returns the HTTP status and the body.
    pub async fn post_raw(&self, body: &[u8]) -> Result<(u16, Vec<u8>, Option<String>)> {
        let mut token = self.token(false).await?;
        let mut renewed = false;
        let mut attempts = 0;
        loop {
            attempts += 1;
            let response = self
                .api
                .http
                .post(&token.rpc_url)
                .bearer_auth(&token.token)
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(body.to_vec())
                .timeout(Duration::from_secs(120))
                .send()
                .await
                .map_err(|error| {
                    anyhow!(
                        "The environment's RPC endpoint is unreachable: {}",
                        error.without_url()
                    )
                })?;
            let status = response.status();
            if status == reqwest::StatusCode::UNAUTHORIZED && !renewed {
                renewed = true;
                token = self.token(true).await?;
                continue;
            }
            let retry_after = response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.trim().parse::<u64>().ok())
                .map(Duration::from_secs);
            if status == reqwest::StatusCode::SERVICE_UNAVAILABLE && attempts <= WAKE_RETRIES {
                if let Some(wait) = retry_after.filter(|wait| *wait <= MAX_RETRY_AFTER) {
                    tokio::time::sleep(wait).await;
                    continue;
                }
            }
            let content_type = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .map(str::to_string);
            let bytes = response.bytes().await.unwrap_or_default().to_vec();
            return Ok((status.as_u16(), bytes, content_type));
        }
    }

    /// One JSON-RPC call; a JSON-RPC error is an [`RpcFailure`].
    pub async fn call(&self, method: &str, params: Value) -> Result<Value> {
        let body = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
        let (status, bytes, _) = self.post_raw(body.to_string().as_bytes()).await?;
        let parsed: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        if let Some(error) = parsed.get("error").filter(|error| !error.is_null()) {
            return Err(RpcFailure {
                method: method.to_string(),
                status,
                code: error["code"].as_i64(),
                message: error["message"]
                    .as_str()
                    .unwrap_or("unknown error")
                    .to_string(),
                data: error.get("data").cloned().filter(|data| !data.is_null()),
            }
            .into());
        }
        if !(200..300).contains(&status) || parsed.get("result").is_none() {
            let text = String::from_utf8_lossy(&bytes);
            return Err(RpcFailure {
                method: method.to_string(),
                status,
                code: None,
                message: format!(
                    "HTTP {status}: {}",
                    text.chars().take(300).collect::<String>()
                ),
                data: None,
            }
            .into());
        }
        Ok(parsed["result"].clone())
    }

    /// A typed environment operation (`devenv_<op>`).
    /// The operation's own result: the endpoint answers `{result: ...}`
    /// inside the JSON-RPC result, which is unwrapped here.
    pub async fn op(&self, op: &str, params: Value) -> Result<Value> {
        let answer = self.call(&format!("devenv_{op}"), params).await?;
        Ok(unwrap_op_result(answer))
    }

    /// Set an account's SOL balance (lamports only; owner and data stay).
    pub async fn fund(&self, pubkey: &str, lamports: u64) -> Result<Value> {
        check_pubkey(pubkey, "pubkey")?;
        if lamports > MAX_FUND_LAMPORTS {
            bail!("At most 1,000,000 SOL per fund");
        }
        self.op("fund", json!({ "pubkey": pubkey, "lamports": lamports }))
            .await
    }

    /// Set `owner`'s token balance of `mint` (base units, as a decimal
    /// string).
    pub async fn set_token_balance(
        &self,
        owner: &str,
        mint: &str,
        amount: &str,
        token_program: Option<&str>,
    ) -> Result<Value> {
        check_pubkey(owner, "owner")?;
        check_pubkey(mint, "mint")?;
        if amount.is_empty() || !amount.bytes().all(|byte| byte.is_ascii_digit()) {
            bail!("The amount is a whole number of base units (got '{amount}')");
        }
        let mut params = json!({ "owner": owner, "mint": mint, "amount": amount });
        if let Some(program) = token_program {
            check_pubkey(program, "tokenProgram")?;
            params["tokenProgram"] = json!(program);
        }
        self.op("setTokenBalance", params).await
    }

    pub async fn warp(&self, target: WarpTarget) -> Result<Value> {
        self.op("warp", target.params()).await
    }

    pub async fn pause(&self) -> Result<Value> {
        self.op("pause", json!({})).await
    }

    pub async fn resume(&self) -> Result<Value> {
        self.op("resume", json!({})).await
    }

    /// Drop one account so the next read refetches it from mainnet.
    pub async fn reset_account(&self, pubkey: &str, include_owned: bool) -> Result<Value> {
        check_pubkey(pubkey, "pubkey")?;
        let mut params = json!({ "pubkey": pubkey });
        if include_owned {
            params["includeOwned"] = json!(true);
        }
        self.op("resetAccount", params).await
    }

    /// Pin an account (never refreshed, skipped by rebase), or unpin it.
    pub async fn pin(&self, pubkey: &str, include_owned: bool, unpin: bool) -> Result<Value> {
        check_pubkey(pubkey, "pubkey")?;
        let mut params = json!({ "pubkey": pubkey });
        if unpin {
            params["unpin"] = json!(true);
        } else if include_owned {
            params["includeOwned"] = json!(true);
        }
        self.op("pin", params).await
    }

    /// Follow an account (refetched from mainnet every block), or stop.
    pub async fn follow(&self, pubkey: &str, stop: bool) -> Result<Value> {
        check_pubkey(pubkey, "pubkey")?;
        let mut params = json!({ "pubkey": pubkey });
        if stop {
            params["stop"] = json!(true);
        }
        self.op("follow", params).await
    }

    /// Run an `arete.scenario/v1` document (or only encode it, `dry_run`).
    pub async fn scenario(&self, scenario: Value, dry_run: bool) -> Result<Value> {
        if !scenario.is_object() {
            bail!("A scenario is a JSON object (arete.scenario/v1)");
        }
        let mut params = json!({ "scenario": scenario });
        if dry_run {
            params["dryRun"] = json!(true);
        }
        self.op("scenario", params).await
    }

    /// Simulate one transaction (base64) without committing it: compute
    /// units per instruction and every account before and after.
    pub async fn preview(&self, transaction_base64: &str) -> Result<Value> {
        self.op("preview", json!({ "transaction": transaction_base64 }))
            .await
    }

    /// The minimal state to replay one sent transaction (by signature).
    pub async fn repro(&self, signature: &str) -> Result<Value> {
        if bs58::decode(signature)
            .into_vec()
            .map(|bytes| bytes.len() != 64)
            .unwrap_or(true)
        {
            bail!("'{signature}' is not a transaction signature");
        }
        self.op("reproExport", json!({ "signature": signature }))
            .await
    }

    /// Send 1-5 transactions (base64) as one bundle, or only simulate them.
    pub async fn bundle(&self, transactions: &[String], simulate: bool) -> Result<Value> {
        if transactions.is_empty() || transactions.len() > 5 {
            bail!("A bundle has 1 to 5 transactions");
        }
        let op = if simulate {
            "simulateBundle"
        } else {
            "sendBundle"
        };
        self.op(op, json!({ "transactions": transactions })).await
    }
}

/// `{result: x}` as `x`; anything else as it is.
pub fn unwrap_op_result(answer: Value) -> Value {
    match answer {
        Value::Object(mut object) if object.len() == 1 && object.contains_key("result") => {
            object.remove("result").unwrap_or(Value::Null)
        }
        other => other,
    }
}

fn check_pubkey(value: &str, field: &str) -> Result<()> {
    if is_pubkey(value) {
        Ok(())
    } else {
        bail!("{field} '{value}' is not a base58 public key")
    }
}

/// Where `warp` moves the environment's clock (forward only).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarpTarget {
    Slot(u64),
    Epoch(u64),
    /// Unix milliseconds.
    TimestampMs(i64),
}

impl WarpTarget {
    pub fn params(self) -> Value {
        match self {
            Self::Slot(slot) => json!({ "slot": slot }),
            Self::Epoch(epoch) => json!({ "epoch": epoch }),
            Self::TimestampMs(ms) => json!({ "timestampMs": ms }),
        }
    }
}

// ---------------------------------------------------------------------------
// Which environment
// ---------------------------------------------------------------------------

/// The environment a command acts on: `reference` (a name or an id), else
/// `ARETE_DEV_ENV`, else the project's (`.arete/dev/environment.json` at or
/// above `project`, default the working directory). A named environment is
/// looked up on the configured API (`ARETE_API_URL`); a project's on the API
/// it was created on.
pub async fn resolve_env(project: Option<&Path>, reference: Option<&str>) -> Result<EnvRpc> {
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(150))
        .build()
        .unwrap_or_default();
    let reference = reference
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            std::env::var(ENV_VAR_ENVIRONMENT)
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        });
    if let Some(reference) = reference {
        let api = EnvApi::from_environment(http)?;
        let id = if is_env_id(&reference) {
            reference
        } else {
            api.find(&reference)
                .await?
                .and_then(|env| env["id"].as_str().map(str::to_string))
                .ok_or_else(|| {
                    anyhow!("You have no dev environment named '{reference}' (`a4 dev list` shows them)")
                })?
        };
        return EnvRpc::new(api, &id);
    }
    let project = crate::dev::DevProject::discover(project)?;
    let selection = project.load_selection()?.ok_or_else(|| {
        anyhow!(
            "{} has no dev environment yet. {}",
            project.root.display(),
            crate::dev::DEV_UP_HINT
        )
    })?;
    let api = EnvApi::for_origin(http, &selection.api_url)?;
    EnvRpc::new(api, &selection.id)
}

/// The environment's slot and clock (Unix seconds), at `processed`.
pub async fn read_clock(rpc: &EnvRpc) -> Result<(u64, i64)> {
    let slot = rpc
        .call("getSlot", json!([{ "commitment": "processed" }]))
        .await?
        .as_u64()
        .ok_or_else(|| anyhow!("getSlot returned no slot"))?;
    let clock = rpc
        .call(
            "getAccountInfo",
            json!(["SysvarC1ock11111111111111111111111111111111", { "encoding": "jsonParsed", "commitment": "processed" }]),
        )
        .await?;
    let unix = clock["value"]["data"]["parsed"]["info"]["unixTimestamp"]
        .as_i64()
        .ok_or_else(|| anyhow!("The environment returned no Clock sysvar"))?;
    Ok((slot, unix))
}

/// Warp the clock to `to`: `slot:<n>`, `epoch:<n>`, a Unix timestamp in
/// seconds, or `+<n>[s|m|h|d]` from the environment's clock. Forward only.
pub async fn warp_to(rpc: &EnvRpc, to: &str) -> Result<Value> {
    let to = to.trim();
    let (slot_before, unix_before) = read_clock(rpc).await?;
    let target = if let Some(slot) = to.strip_prefix("slot:") {
        let slot: u64 = slot
            .trim()
            .parse()
            .with_context(|| format!("'{to}': the slot is a number"))?;
        if slot <= slot_before {
            bail!("The environment is at slot {slot_before}; warp only moves forward");
        }
        WarpTarget::Slot(slot)
    } else if let Some(epoch) = to.strip_prefix("epoch:") {
        WarpTarget::Epoch(
            epoch
                .trim()
                .parse()
                .with_context(|| format!("'{to}': the epoch is a number"))?,
        )
    } else {
        let seconds = match crate::dev::TimeTarget::parse(to)? {
            crate::dev::TimeTarget::Absolute(seconds) => seconds,
            crate::dev::TimeTarget::Forward(seconds) => unix_before.saturating_add(seconds),
        };
        if seconds <= unix_before {
            bail!("The environment's clock is at {unix_before}; warp only moves forward");
        }
        WarpTarget::TimestampMs(seconds.saturating_mul(1000))
    };
    let result = rpc.warp(target).await?;
    let after = read_clock(rpc).await.ok();
    Ok(json!({
        "before": { "slot": slot_before, "unixTimestamp": unix_before },
        "target": target.params(),
        "after": after.map(|(slot, unix)| json!({ "slot": slot, "unixTimestamp": unix })),
        "result": result,
    }))
}

// ---------------------------------------------------------------------------
// Rendering helpers
// ---------------------------------------------------------------------------

/// A one-line summary of an environment view.
pub fn describe_env(env: &Value) -> String {
    let mut line = format!(
        "{} ({}) {}",
        env["name"].as_str().unwrap_or("?"),
        env["id"].as_str().unwrap_or("?"),
        env["state"].as_str().unwrap_or("unknown")
    );
    if let Some(busy) = env["busy"].as_str() {
        line.push_str(&format!(" [{busy}]"));
    }
    if let Some(tier) = env["tier"].as_str() {
        line.push_str(&format!(", {tier} tier"));
    }
    line
}

/// The environment's view without engine details.
pub fn public_view(env: &Value) -> Value {
    let mut env = env.clone();
    if let Some(object) = env.as_object_mut() {
        object.remove("surfpoolVersion");
    }
    env
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_overrides_answer_without_the_list_is_an_error() {
        let overrides = json!([{ "kind": "program_release", "key": "P", "target": "r" }]);
        assert_eq!(
            overrides_of(json!({ "overrides": overrides.clone() })).unwrap(),
            overrides.as_array().unwrap().clone()
        );
        assert!(overrides_of(json!({ "overrides": [] })).unwrap().is_empty());
        assert!(overrides_of(json!({})).is_err());
        assert!(overrides_of(json!({ "overrides": null })).is_err());
        assert!(overrides_of(json!({ "items": [] })).is_err());
        assert!(overrides_of(json!("<html>")).is_err());
        assert!(overrides_of(Value::Null).is_err());
    }

    #[test]
    fn env_ids_and_names() {
        assert!(is_env_id("denv_01hzx3k8m9q2r4s5t6v7w8x9yz"));
        assert!(!is_env_id("denv_01HZX3K8M9Q2R4S5T6V7W8X9YZ"));
        assert!(!is_env_id("denv_short"));
        assert!(!is_env_id("my-env"));
        assert!(valid_env_name("my-env-2"));
        assert!(!valid_env_name("My Env"));
        assert!(!valid_env_name(""));
        assert!(!valid_env_name(&"a".repeat(64)));
        assert_eq!(env_name_from("Token Raise_App"), "token-raise-app");
        assert_eq!(env_name_from("--__--"), "dev");
        assert_eq!(env_name_from("ünïcode"), "n-code");
        assert_eq!(env_name_from(&"x".repeat(80)).len(), 63);
    }

    #[test]
    fn sol_amounts() {
        assert_eq!(sol_to_lamports("1").unwrap(), LAMPORTS_PER_SOL);
        assert_eq!(sol_to_lamports("0.5").unwrap(), LAMPORTS_PER_SOL / 2);
        assert_eq!(sol_to_lamports(".000000001").unwrap(), 1);
        assert!(sol_to_lamports("1.0000000001").is_err());
        assert!(sol_to_lamports("-1").is_err());
        assert!(sol_to_lamports("abc").is_err());
        assert!(sol_to_lamports("").is_err());
        assert!(sol_to_lamports("1000001").is_err());
        assert_eq!(sol_to_lamports("1000000").unwrap(), MAX_FUND_LAMPORTS);
    }

    #[test]
    fn origins_and_urls() {
        assert_eq!(
            api_origin("https://api.arete.run/api/"),
            "https://api.arete.run"
        );
        assert_eq!(
            api_origin("http://127.0.0.1:3000/"),
            "http://127.0.0.1:3000"
        );
        assert!(is_secure_or_loopback("https://gw.example/rpc"));
        assert!(is_secure_or_loopback("http://127.0.0.1:9/rpc"));
        assert!(!is_secure_or_loopback("http://gw.example/rpc"));
        assert_eq!(
            ws_url_of("https://gw.example/rpc/v1/devenv/denv_x"),
            "wss://gw.example/rpc/v1/devenv/denv_x"
        );
        assert_eq!(ws_url_of("http://127.0.0.1:9/r"), "ws://127.0.0.1:9/r");
        assert_eq!(
            gateway_rpc_url("https://gw.example/", "denv_01hzx3k8m9q2r4s5t6v7w8x9yz"),
            "https://gw.example/rpc/v1/devenv/denv_01hzx3k8m9q2r4s5t6v7w8x9yz"
        );
        assert!(EnvApi::new(reqwest::Client::new(), "https://evil.example", "k").is_err());
        assert!(EnvApi::new(reqwest::Client::new(), "https://api.arete.run/api", "k").is_ok());
    }

    #[test]
    fn selection_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("environment.json");
        assert!(EnvSelection::load(&path).unwrap().is_none());
        let selection = EnvSelection::new(
            "denv_01hzx3k8m9q2r4s5t6v7w8x9yz",
            "app",
            "http://127.0.0.1:3000/api",
        );
        selection.save(&path).unwrap();
        let loaded = EnvSelection::load(&path).unwrap().unwrap();
        assert_eq!(loaded, selection);
        assert_eq!(loaded.api_url, "http://127.0.0.1:3000");
        std::fs::write(
            &path,
            r#"{"schema":"arete.dev-environment/v1","id":"x","name":"a","apiUrl":"u"}"#,
        )
        .unwrap();
        assert!(EnvSelection::load(&path).is_err());
    }

    #[test]
    fn op_results_are_unwrapped() {
        assert_eq!(
            unwrap_op_result(json!({ "result": { "pinned": true } })),
            json!({ "pinned": true })
        );
        assert_eq!(
            unwrap_op_result(json!({ "result": 1, "x": 2 })),
            json!({ "result": 1, "x": 2 })
        );
        assert_eq!(unwrap_op_result(json!(5)), json!(5));
    }

    #[test]
    fn warp_params_and_views() {
        assert_eq!(WarpTarget::Slot(5).params(), json!({ "slot": 5 }));
        assert_eq!(
            WarpTarget::TimestampMs(1_000).params(),
            json!({ "timestampMs": 1_000 })
        );
        let env = json!({ "id": "denv_a", "name": "app", "state": "running", "tier": "free", "surfpoolVersion": "1" });
        assert_eq!(describe_env(&env), "app (denv_a) running, free tier");
        assert!(public_view(&env).get("surfpoolVersion").is_none());
        let failure = ApiFailure {
            status: 403,
            code: Some("devenv-environment-quota".into()),
            message: "limit".into(),
        };
        assert!(failure.to_string().contains("devenv-environment-quota"));
        assert!(failure.hint().is_some());
    }
}
