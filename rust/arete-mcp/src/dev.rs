//! Agent tooling for `a4 dev` against an Arete cloud dev environment.
//!
//! A project develops against one cloud dev environment (a persistent
//! mainnet fork Arete hosts for the account), which `a4 dev up` creates or
//! wakes and remembers in `.arete/dev/environment.json`. While `a4 dev up`
//! runs it also writes `.arete/dev/session.json`: the local RPC URL (a
//! loopback proxy that adds the environment token, for wallets, apps and
//! tools that can't send one), the dev UI and its event feed, and the
//! protocol deployment it keeps in the environment.
//!
//! Everything here works from those two files and the account's API key, so
//! the `a4 dev` commands and the matching MCP tools share one
//! implementation:
//!
//! - [`DevClient::status`] reports the environment, the session and the
//!   chain clock;
//! - [`DevClient::step`] prepares a protocol step on the environment's
//!   deployment through the Arete API, signs it locally with a dev wallet
//!   (`.arete/dev/wallets/<name>.json`, created and funded on first use),
//!   submits it and waits until the execution intent settles. Each step is
//!   appended to `.arete/dev/history.jsonl`, which [`DevClient::replay`] can
//!   run again. A replay skips entries recorded as failed (unless asked to
//!   include them) and logs its own steps to `.arete/dev/replays.jsonl`, so
//!   the history only ever holds original steps;
//! - [`DevClient::time_travel`] warps the environment's clock forward and
//!   [`DevClient::rpc`] reads it, both through the environment's RPC
//!   endpoint ([`crate::dev_env::EnvRpc`]).
//!
//! Keys stay where they are: the API key in the a4 login (or
//! `ARETE_API_KEY`), wallet keys in `.arete/dev/wallets/`, and environment
//! tokens in memory.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use base64::Engine as _;
use ed25519_dalek::{Signer as _, SigningKey};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::dev_env::{EnvApi, EnvRpc, EnvSelection, WarpTarget};
use crate::protocol_api::{
    PrepareRequest, Prepared as ApiPrepared, ProtocolApi, TRANSACTION_SCOPES,
};

/// Project-relative directory `a4 dev` keeps its state in.
pub const DEV_DIR: &str = ".arete/dev";
/// Schema of `.arete/dev/session.json`.
pub const SESSION_SCHEMA: &str = "arete.dev-session/v2";
/// Schema of one `.arete/dev/history.jsonl` line.
pub const STEP_RECORD_SCHEMA: &str = "arete.dev-step/v1";
/// Schema of a replay file.
pub const REPLAY_SCHEMA: &str = "arete.dev-replay/v1";
/// Schema of one `.arete/dev/replays.jsonl` line.
pub const REPLAY_STEP_RECORD_SCHEMA: &str = "arete.dev-replay-step/v1";
/// What the dev tools say when a project has no environment yet.
pub const DEV_UP_HINT: &str = "Run `a4 dev up` in the project (in a terminal: it keeps running) to create or wake its Arete cloud dev environment.";

/// Lamports a new (or empty) dev wallet is funded with (100 SOL; the
/// environment's SOL is worthless).
pub const WALLET_FUND_LAMPORTS: u64 = 100_000_000_000;

const CLOCK_SYSVAR: &str = "SysvarC1ock11111111111111111111111111111111";
const INTENT_TIMEOUT: Duration = Duration::from_secs(90);

/// The environment a session runs against.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvRef {
    pub id: String,
    pub name: String,
}

/// `.arete/dev/session.json`: what an app, a test or an agent needs while
/// `a4 dev up` runs. It holds no key and no token: SDKs in dev mode reach
/// the environment through the edge (`edgeUrl`), which attaches tokens
/// itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DevSession {
    pub schema: String,
    pub environment: EnvRef,
    /// The Arete API origin; protocol routes are under `/api`.
    pub api_url: String,
    /// `POST` here (with the API key) to mint protocol and stack sessions.
    pub session_endpoint: String,
    /// The local RPC URL `a4 dev up` serves on loopback: JSON-RPC to the
    /// environment with its token added. Empty when `a4 dev up` is not
    /// running.
    #[serde(default)]
    pub rpc_url: String,
    /// The same proxy's WebSocket URL (subscriptions). Empty when `a4 dev
    /// up` is not running.
    #[serde(default)]
    pub rpc_ws_url: String,
    /// The edge: the environment's chain, transaction, Program Read, stack
    /// and protocol routes in their production shape, on loopback, for SDKs
    /// in dev mode (`ARETE_ENVIRONMENT=dev`). The same origin as `rpcUrl`.
    /// Empty when `a4 dev up` is not running.
    #[serde(default)]
    pub edge_url: String,
    /// The environment's own RPC URL. It needs an environment token as a
    /// bearer credential (`a4 dev token` mints one).
    #[serde(default)]
    pub environment_rpc_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol_slug: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_hash: Option<String>,
    /// The protocol deployment `a4 dev up` keeps in the environment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deployment_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub short_id: Option<String>,
    /// The protocol stack's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stack: Option<String>,
    /// The protocol stack's WebSocket URL. Connecting needs a stack session
    /// token minted with the account's key; `a4 stream --dev` mints its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stack_ws_url: Option<String>,
    /// The dev UI and its event feed (`GET <uiUrl>/dev/v1/events`), served
    /// by `a4 dev up` on loopback.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui_url: Option<String>,
    /// Unix seconds of the last write.
    #[serde(default)]
    pub updated_at: u64,
}

/// A stack session: the token the stack's WebSocket takes as `?hs_token=`
/// (or `Authorization: Bearer`), minted by `POST <apiUrl>/ws/sessions` with
/// `{"websocket_url": <stackWsUrl>, "protocolDeploymentId": <deploymentId>}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StackSession {
    pub token: String,
    /// Unix seconds.
    pub expires_at: u64,
    /// `stackWsUrl` with the token as `hs_token`: pass it to
    /// `a4 stream <View> --url`.
    pub stream_url: String,
}

/// `text` with every `hs_token=<token>` value replaced by `<redacted>`.
/// Idempotent: a value already redacted is left as it is.
pub fn redact_hs_tokens(text: &str) -> String {
    const MARKER: &str = "hs_token=";
    const REDACTED: &str = "<redacted>";
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(index) = rest.find(MARKER) {
        let (head, tail) = rest.split_at(index + MARKER.len());
        out.push_str(head);
        out.push_str(REDACTED);
        let tail = tail.strip_prefix(REDACTED).unwrap_or(tail);
        let end = tail
            .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '%')))
            .unwrap_or(tail.len());
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

/// Remove credentials from a report before it is printed: every
/// `hs_token=` value, every `Bearer <token>` and every string field named
/// `token`, at any depth. `a4 dev token` is the way to get a token.
pub fn redact_secrets(value: &mut Value) {
    match value {
        Value::String(text) => {
            let mut redacted = redact_hs_tokens(text);
            if let Some(index) = redacted.find("Bearer ") {
                redacted.truncate(index + "Bearer ".len());
                redacted.push_str("<redacted>");
            }
            *text = redacted;
        }
        Value::Array(items) => items.iter_mut().for_each(redact_secrets),
        Value::Object(fields) => {
            for (key, field) in fields.iter_mut() {
                if key == "token" && field.is_string() {
                    *field = json!("<redacted>");
                } else {
                    redact_secrets(field);
                }
            }
        }
        _ => {}
    }
}

/// `ws_url` with `token` as its `hs_token` query parameter.
pub fn stack_stream_url(ws_url: &str, token: &str) -> Result<String> {
    let mut url = reqwest::Url::parse(ws_url)
        .with_context(|| format!("'{ws_url}' is not a WebSocket URL"))?;
    url.query_pairs_mut().append_pair("hs_token", token);
    Ok(url.to_string())
}

impl DevSession {
    /// A session for `selection` with nothing running locally (no local RPC
    /// URL and no dev UI): what the dev tools use when `a4 dev up` is not
    /// running.
    pub fn offline(selection: &EnvSelection) -> Self {
        let api_url = crate::dev_env::api_origin(&selection.api_url);
        Self {
            schema: SESSION_SCHEMA.into(),
            environment: EnvRef {
                id: selection.id.clone(),
                name: selection.name.clone(),
            },
            session_endpoint: format!("{api_url}/ws/sessions"),
            api_url,
            rpc_url: String::new(),
            rpc_ws_url: String::new(),
            edge_url: String::new(),
            environment_rpc_url: String::new(),
            protocol_id: None,
            protocol_slug: None,
            release_hash: None,
            deployment_id: None,
            short_id: None,
            stack: None,
            stack_ws_url: None,
            ui_url: None,
            updated_at: 0,
        }
    }

    /// Refuse a session file whose local URLs are not on this machine, or
    /// whose API is not Arete's (or loopback): `a4 dev up` writes nothing
    /// else, and these URLs decide where a signer's simulation and the dev
    /// tools' calls go.
    pub fn check(&self) -> Result<()> {
        for (field, url) in [
            ("rpcUrl", &self.rpc_url),
            ("rpcWsUrl", &self.rpc_ws_url),
            ("edgeUrl", &self.edge_url),
        ] {
            if !url.is_empty() && !is_loopback_url(url) {
                bail!("{field} `{url}` is not a loopback URL");
            }
        }
        if let Some(url) = &self.ui_url {
            if !is_loopback_url(url) {
                bail!("uiUrl `{url}` is not a loopback URL");
            }
        }
        for (field, url) in [
            ("apiUrl", &self.api_url),
            ("sessionEndpoint", &self.session_endpoint),
        ] {
            if !crate::registry::is_arete_origin(&crate::dev_env::api_origin(url)) {
                bail!("{field} `{url}` is not an Arete API");
            }
        }
        // The account key goes to the session endpoint; it was checked for
        // apiUrl's origin only.
        if url_origin(&self.session_endpoint) != url_origin(&self.api_url) {
            bail!(
                "sessionEndpoint `{}` is not on apiUrl `{}`",
                self.session_endpoint,
                self.api_url
            );
        }
        if !crate::dev_env::is_env_id(&self.environment.id) {
            bail!("`{}` is not an environment id", self.environment.id);
        }
        Ok(())
    }

    /// The protocol API base (`<apiUrl>/api`), as generated clients take it.
    pub fn protocol_api(&self) -> String {
        format!("{}/api", self.api_url.trim_end_matches('/'))
    }

    /// The session as reports show it, with any token scrubbed (it holds
    /// none; a file an older `a4` wrote is parsed without one).
    pub fn redacted(&self) -> Value {
        let mut value = serde_json::to_value(self).unwrap_or(Value::Null);
        redact_secrets(&mut value);
        value
    }
}

/// A project with an `a4 dev` directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevProject {
    pub root: PathBuf,
}

impl DevProject {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The nearest directory at or above `start` (default: the current
    /// directory) with a dev environment (`.arete/dev/environment.json`).
    pub fn discover(start: Option<&Path>) -> Result<Self> {
        let start = match start {
            Some(path) => path.to_path_buf(),
            None => std::env::current_dir().context("Failed to read the current directory")?,
        };
        let start = fs::canonicalize(&start).unwrap_or(start);
        for dir in start.ancestors() {
            let dev = dir.join(DEV_DIR);
            if dev.join("environment.json").is_file() || dev.join("session.json").is_file() {
                return Ok(Self::new(dir));
            }
        }
        bail!(
            "No a4 dev project found at or above {}. {DEV_UP_HINT}",
            start.display()
        )
    }

    pub fn dev_dir(&self) -> PathBuf {
        self.root.join(DEV_DIR)
    }
    pub fn session_path(&self) -> PathBuf {
        self.dev_dir().join("session.json")
    }
    /// `.arete/dev/environment.json` ([`EnvSelection`]).
    pub fn environment_path(&self) -> PathBuf {
        self.dev_dir().join("environment.json")
    }
    pub fn wallets_dir(&self) -> PathBuf {
        self.dev_dir().join("wallets")
    }
    pub fn history_path(&self) -> PathBuf {
        self.dev_dir().join("history.jsonl")
    }
    /// The steps replays ran (never part of the history).
    pub fn replay_log_path(&self) -> PathBuf {
        self.dev_dir().join("replays.jsonl")
    }
    /// What `a4 dev up` deployed (`deploy.json`).
    pub fn deploy_state_path(&self) -> PathBuf {
        self.dev_dir().join("deploy.json")
    }

    /// The project's environment, if it has one.
    pub fn load_selection(&self) -> Result<Option<EnvSelection>> {
        EnvSelection::load(&self.environment_path())
    }

    /// The running `a4 dev up`'s session file.
    pub fn load_session(&self) -> Result<DevSession> {
        let path = self.session_path();
        let bytes = fs::read(&path).with_context(|| {
            format!("Failed to read {}; is `a4 dev up` running?", path.display())
        })?;
        let session: DevSession = serde_json::from_slice(&bytes)
            .with_context(|| format!("{} is not a dev session file", path.display()))?;
        if session.schema != SESSION_SCHEMA {
            bail!(
                "{} has schema '{}', expected '{SESSION_SCHEMA}' (written by an older a4; restart `a4 dev up`)",
                path.display(),
                session.schema
            );
        }
        session
            .check()
            .with_context(|| format!("{} is not a dev session file a4 wrote", path.display()))?;
        Ok(session)
    }

    /// The session file when `a4 dev up` runs for `selection`'s
    /// environment, else an offline session with the deployment `a4 dev up`
    /// last recorded.
    pub fn session_for(&self, selection: &EnvSelection) -> DevSession {
        // The session's API gets the key checked for the selection's API,
        // so it must be the same origin.
        if let Ok(session) = self.load_session() {
            if session.environment.id == selection.id
                && url_origin(&session.api_url) == url_origin(&selection.api_url)
            {
                return session;
            }
        }
        let mut session = DevSession::offline(selection);
        if let Ok(bytes) = fs::read(self.deploy_state_path()) {
            if let Ok(state) = serde_json::from_slice::<Value>(&bytes) {
                if state["environment"].as_str() == Some(selection.id.as_str()) {
                    let field = |name: &str| state[name].as_str().map(str::to_string);
                    session.protocol_id = field("protocolId");
                    session.protocol_slug = field("slug");
                    session.release_hash = field("releaseHash");
                    session.deployment_id = field("deploymentId");
                    session.short_id = field("shortId");
                }
            }
        }
        session
    }
}

/// `scheme://host[:port]` of `url` (default port omitted), `None` when it
/// does not parse.
fn url_origin(url: &str) -> Option<String> {
    reqwest::Url::parse(url.trim())
        .ok()
        .map(|url| url.origin().ascii_serialization())
        .filter(|origin| origin != "null")
}

/// Whether `url` is an HTTP or WebSocket URL on a loopback host
/// (`localhost`, `127.0.0.0/8` or `::1`).
pub fn is_loopback_url(url: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(url) else {
        return false;
    };
    if !matches!(url.scheme(), "http" | "https" | "ws" | "wss") {
        return false;
    }
    let Some(host) = url.host_str() else {
        return false;
    };
    let host = host.trim_start_matches('[').trim_end_matches(']');
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// Write `contents` to `path` with owner-only permissions, atomically.
pub fn write_private(path: &Path, contents: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }
    let tmp = path.with_extension("tmp");
    {
        let mut options = fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&tmp)
            .with_context(|| format!("Failed to write {}", tmp.display()))?;
        file.write_all(contents)?;
        file.sync_all().ok();
    }
    fs::rename(&tmp, path).with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(())
}

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Wallets
// ---------------------------------------------------------------------------

/// A dev wallet: a Solana CLI keypair file under `.arete/dev/wallets/`.
#[derive(Clone)]
pub struct DevWallet {
    pub name: String,
    pub path: PathBuf,
    key: SigningKey,
}

impl std::fmt::Debug for DevWallet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DevWallet")
            .field("name", &self.name)
            .field("pubkey", &self.pubkey())
            .finish_non_exhaustive()
    }
}

impl DevWallet {
    pub fn pubkey(&self) -> String {
        bs58::encode(self.key.verifying_key().as_bytes()).into_string()
    }

    pub fn pubkey_bytes(&self) -> [u8; 32] {
        *self.key.verifying_key().as_bytes()
    }

    /// Load `<dir>/<name>.json`, creating it when absent. Returns whether it
    /// was created.
    pub fn load_or_create(dir: &Path, name: &str) -> Result<(Self, bool)> {
        validate_wallet_name(name)?;
        let path = dir.join(format!("{name}.json"));
        if path.is_file() {
            return Self::load_existing(path, name).map(|wallet| (wallet, false));
        }
        let mut seed = [0u8; 32];
        getrandom::getrandom(&mut seed).map_err(|error| anyhow!("no OS randomness: {error}"))?;
        let key = SigningKey::from_bytes(&seed);
        let mut bytes = seed.to_vec();
        bytes.extend_from_slice(key.verifying_key().as_bytes());
        fs::create_dir_all(dir)?;
        // Publish a complete, owner-only file without replacing a concurrent
        // creator's key. Readers never observe a partially written keypair.
        let mut temporary = tempfile::NamedTempFile::new_in(dir)?;
        temporary.write_all(serde_json::to_string(&bytes)?.as_bytes())?;
        temporary.as_file().sync_all()?;
        match temporary.persist_noclobber(&path) {
            Ok(_) => {}
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                return Self::load_existing(path, name).map(|wallet| (wallet, false));
            }
            Err(error) => return Err(error.error).context("Failed to save the new dev wallet"),
        }
        Ok((
            Self {
                name: name.to_string(),
                path,
                key,
            },
            true,
        ))
    }

    /// Read a winner once. Invalid or disappeared destinations are errors,
    /// not a reason to generate another key or retry creation.
    fn load_existing(path: PathBuf, name: &str) -> Result<Self> {
        let metadata =
            fs::metadata(&path).with_context(|| format!("Failed to read {}", path.display()))?;
        if !metadata.is_file() {
            bail!("{} is not a regular keypair file", path.display());
        }
        let bytes: Vec<u8> = serde_json::from_slice(
            &fs::read(&path).with_context(|| format!("Failed to read {}", path.display()))?,
        )
        .with_context(|| format!("{} is not a Solana keypair file", path.display()))?;
        if bytes.len() != 64 {
            bail!("{} must hold 64 bytes", path.display());
        }
        let seed: [u8; 32] = bytes[..32].try_into().expect("checked length");
        let key = SigningKey::from_bytes(&seed);
        if key.verifying_key().as_bytes() != &bytes[32..] {
            bail!(
                "{}: the public key does not match the secret key",
                path.display()
            );
        }
        Ok(Self {
            name: name.to_string(),
            path,
            key,
        })
    }

    /// Add this wallet's signature to a serialized legacy or v0 transaction.
    pub fn sign_transaction(&self, transaction: &[u8]) -> Result<Vec<u8>> {
        sign_transaction(transaction, &self.key)
    }
}

/// The dev wallets under `dir`, by name, with their public keys. Files
/// that are not keypairs are skipped. Never reads more than the public half.
pub fn list_wallets(dir: &Path) -> Vec<(String, String)> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut wallets: Vec<(String, String)> = entries
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            let name = path
                .file_name()?
                .to_str()?
                .strip_suffix(".json")?
                .to_string();
            validate_wallet_name(&name).ok()?;
            let bytes: Vec<u8> = serde_json::from_slice(&fs::read(&path).ok()?).ok()?;
            (bytes.len() == 64).then(|| (name, bs58::encode(&bytes[32..]).into_string()))
        })
        .collect();
    wallets.sort();
    wallets
}

/// Whether `name` is a valid dev wallet name (`[A-Za-z0-9_-]{1,64}`).
pub fn validate_wallet_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > 64
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        bail!("Wallet name '{name}' must be 1-64 characters of [A-Za-z0-9_-]");
    }
    Ok(())
}

/// Sign a legacy, v0 or v1 wire transaction in the slot of the signer's key.
pub fn sign_transaction(transaction: &[u8], key: &SigningKey) -> Result<Vec<u8>> {
    let decoded = crate::prepared_summary::decode_transaction(transaction)?;
    let signer = key.verifying_key();
    let signer_key = bs58::encode(signer.as_bytes()).into_string();
    let index = decoded
        .static_keys
        .iter()
        .take(decoded.required_signatures.min(decoded.signatures.len()))
        .position(|candidate| *candidate == signer_key)
        .ok_or_else(|| {
            anyhow!("the wallet {signer_key} is not a required signer of this transaction")
        })?;
    let signature = key.sign(&transaction[decoded.message.clone()]);
    let mut signed = transaction.to_vec();
    let at = decoded.signature_offset + 64 * index;
    signed[at..at + 64].copy_from_slice(&signature.to_bytes());
    Ok(signed)
}

/// A compact-u16 at the start of `bytes`: its value and encoded length.
fn decode_compact_u16(bytes: &[u8]) -> Result<(usize, usize)> {
    let mut value = 0usize;
    for index in 0..3 {
        let byte = *bytes
            .get(index)
            .ok_or_else(|| anyhow!("truncated transaction"))?;
        value |= ((byte & 0x7f) as usize) << (7 * index);
        if byte & 0x80 == 0 {
            return Ok((value, index + 1));
        }
    }
    bail!("invalid compact-u16 length")
}

/// A readable view of a wire transaction (legacy or v0), for previews:
/// its signature slots (which are filled), the message's account keys with
/// their signer and writable flags, its instructions (program, accounts and
/// data) and address table lookups.
pub fn describe_transaction(transaction: &[u8]) -> Result<Value> {
    let (signature_count, prefix) = decode_compact_u16(transaction)?;
    let message_start = prefix + 64 * signature_count;
    let signatures: Vec<bool> = (0..signature_count)
        .map(|index| {
            let at = prefix + 64 * index;
            transaction
                .get(at..at + 64)
                .is_some_and(|slot| slot.iter().any(|byte| *byte != 0))
        })
        .collect();
    let message = transaction
        .get(message_start..)
        .filter(|message| !message.is_empty())
        .ok_or_else(|| anyhow!("truncated transaction"))?;
    let versioned = message[0] & 0x80 != 0;
    let mut offset = usize::from(versioned);
    let header = message
        .get(offset..offset + 3)
        .ok_or_else(|| anyhow!("truncated message header"))?;
    let (required, readonly_signed, readonly_unsigned) =
        (header[0] as usize, header[1] as usize, header[2] as usize);
    offset += 3;
    let read_len = |offset: &mut usize| -> Result<usize> {
        let (value, length) = decode_compact_u16(message.get(*offset..).unwrap_or_default())?;
        *offset += length;
        Ok(value)
    };
    let key_count = read_len(&mut offset)?;
    let keys_bytes = message
        .get(offset..offset + 32 * key_count)
        .ok_or_else(|| anyhow!("truncated account keys"))?;
    offset += 32 * key_count;
    let keys: Vec<String> = keys_bytes
        .as_chunks::<32>()
        .0
        .iter()
        .map(|key| bs58::encode(key).into_string())
        .collect();
    let accounts: Vec<Value> = keys
        .iter()
        .enumerate()
        .map(|(index, key)| {
            let signer = index < required;
            let writable = if signer {
                index < required.saturating_sub(readonly_signed)
            } else {
                index < key_count.saturating_sub(readonly_unsigned)
            };
            json!({ "index": index, "pubkey": key, "signer": signer, "writable": writable })
        })
        .collect();
    let blockhash = message
        .get(offset..offset + 32)
        .map(|hash| bs58::encode(hash).into_string())
        .ok_or_else(|| anyhow!("truncated blockhash"))?;
    offset += 32;
    let instruction_count = read_len(&mut offset)?;
    let mut instructions = Vec::with_capacity(instruction_count);
    for _ in 0..instruction_count {
        let program = *message
            .get(offset)
            .ok_or_else(|| anyhow!("truncated instruction"))? as usize;
        offset += 1;
        let account_count = read_len(&mut offset)?;
        let indexes = message
            .get(offset..offset + account_count)
            .ok_or_else(|| anyhow!("truncated instruction accounts"))?
            .to_vec();
        offset += account_count;
        let data_len = read_len(&mut offset)?;
        let data = message
            .get(offset..offset + data_len)
            .ok_or_else(|| anyhow!("truncated instruction data"))?;
        offset += data_len;
        let name = |index: usize| {
            keys.get(index)
                .cloned()
                .unwrap_or_else(|| format!("lookup#{index}"))
        };
        instructions.push(json!({
            "programId": name(program),
            "accounts": indexes.iter().map(|index| name(*index as usize)).collect::<Vec<_>>(),
            "dataBase64": base64::engine::general_purpose::STANDARD.encode(data),
            "dataLength": data.len(),
        }));
    }
    let mut lookups = Vec::new();
    if versioned {
        let table_count = read_len(&mut offset)?;
        for _ in 0..table_count {
            let table = message
                .get(offset..offset + 32)
                .map(|key| bs58::encode(key).into_string())
                .ok_or_else(|| anyhow!("truncated address table"))?;
            offset += 32;
            let writable_count = read_len(&mut offset)?;
            let writable = message
                .get(offset..offset + writable_count)
                .unwrap_or_default()
                .to_vec();
            offset += writable_count;
            let readonly_count = read_len(&mut offset)?;
            let readonly = message
                .get(offset..offset + readonly_count)
                .unwrap_or_default()
                .to_vec();
            offset += readonly_count;
            lookups.push(json!({ "table": table, "writable": writable, "readonly": readonly }));
        }
    }
    Ok(json!({
        "version": if versioned { "v0" } else { "legacy" },
        "signatures": signatures,
        "requiredSignatures": required,
        "accounts": accounts,
        "recentBlockhash": blockhash,
        "instructions": instructions,
        "addressTableLookups": lookups,
        "size": transaction.len(),
    }))
}

// ---------------------------------------------------------------------------
// Time travel
// ---------------------------------------------------------------------------

/// Where `time-travel` moves the fork's clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeTarget {
    /// An absolute Unix timestamp, in seconds.
    Absolute(i64),
    /// Seconds forward from the fork's current clock.
    Forward(i64),
}

impl TimeTarget {
    /// `1767225600` (Unix seconds), or `+90`, `+90s`, `+15m`, `+2h`, `+7d`.
    pub fn parse(value: &str) -> Result<Self> {
        let value = value.trim();
        if let Some(rest) = value.strip_prefix('+') {
            let (number, unit) = match rest.char_indices().find(|(_, c)| !c.is_ascii_digit()) {
                Some((index, _)) => rest.split_at(index),
                None => (rest, "s"),
            };
            let amount: i64 = number.parse().with_context(|| {
                format!("'{value}' is not a duration like +90s, +15m, +2h, +7d")
            })?;
            let factor = match unit {
                "s" => 1,
                "m" => 60,
                "h" => 3_600,
                "d" => 86_400,
                _ => bail!("'{value}': the unit must be s, m, h or d"),
            };
            if amount <= 0 {
                bail!("'{value}': the duration must be positive");
            }
            return Ok(Self::Forward(amount.saturating_mul(factor)));
        }
        let seconds: i64 = value
            .parse()
            .with_context(|| format!("'{value}' is neither a Unix timestamp nor +duration"))?;
        if seconds <= 0 {
            bail!("'{value}' must be a positive Unix timestamp in seconds");
        }
        Ok(Self::Absolute(seconds))
    }
}

// ---------------------------------------------------------------------------
// Steps and replays
// ---------------------------------------------------------------------------

/// One step to run: `a4 dev step` or one entry of a replay.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StepRequest {
    pub step: String,
    /// The instance for instance steps; absent for create steps. In a replay,
    /// `@N` names the instance the N-th entry (from 0) created or used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
    #[serde(default = "empty_args")]
    pub args: Value,
    /// Dev wallet name (default `default`).
    #[serde(default = "default_wallet")]
    pub wallet: String,
    /// What the recorded run produced; a replay maps its `instanceId` to the
    /// new run's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
}

fn empty_args() -> Value {
    json!({})
}

fn default_wallet() -> String {
    "default".to_string()
}

/// Parse a replay file: `{"schema":"arete.dev-replay/v1","steps":[…]}`, a bare
/// array of steps, or `history.jsonl` lines.
pub fn parse_replay(text: &str) -> Result<Vec<StepRequest>> {
    let trimmed = text.trim_start();
    if trimmed.starts_with('[') {
        return serde_json::from_str(trimmed).context("Replay array is not a list of steps");
    }
    if let Ok(document) = serde_json::from_str::<Value>(trimmed) {
        if let Some(steps) = document.get("steps") {
            if let Some(schema) = document.get("schema").and_then(Value::as_str) {
                if schema != REPLAY_SCHEMA {
                    bail!("Replay schema '{schema}' is not {REPLAY_SCHEMA}");
                }
            }
            return serde_json::from_value(steps.clone()).context("Replay steps are invalid");
        }
        if document.get("step").is_some() {
            return Ok(vec![serde_json::from_value(document)?]);
        }
    }
    text.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(index, line)| {
            serde_json::from_str(line)
                .with_context(|| format!("Replay line {} is invalid", index + 1))
        })
        .collect()
}

/// What [`DevClient::replay`] runs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReplayOptions {
    /// Also run entries recorded as rejected or failed. Their failing again
    /// does not stop the replay.
    pub include_failed: bool,
}

/// The recorded outcome of a history entry that did not complete
/// (`rejected`, `chain_failed`, `expired`, ...). Entries without a recorded
/// result (hand-written replay files) never count as failed.
pub fn recorded_failure(entry: &StepRequest) -> Option<&str> {
    let state = entry.result.as_ref()?.get("state")?.as_str()?;
    (state != "completed").then_some(state)
}

/// Resolves replay instance references: `@N`, and recorded instance ids
/// mapped to the ones the replay produced.
#[derive(Debug, Default)]
pub struct ReplayInstances {
    by_index: Vec<Option<String>>,
    by_recorded: BTreeMap<String, String>,
}

impl ReplayInstances {
    pub fn resolve(&self, reference: Option<&str>) -> Result<Option<String>> {
        let Some(reference) = reference else {
            return Ok(None);
        };
        if let Some(index) = reference.strip_prefix('@') {
            let index: usize = index
                .parse()
                .with_context(|| format!("'{reference}' is not @<entry index>"))?;
            return self
                .by_index
                .get(index)
                .cloned()
                .flatten()
                .map(Some)
                .ok_or_else(|| anyhow!("'{reference}' names no earlier entry with an instance"));
        }
        Ok(Some(
            self.by_recorded
                .get(reference)
                .cloned()
                .unwrap_or_else(|| reference.to_string()),
        ))
    }

    pub fn record(&mut self, entry: &StepRequest, produced: Option<String>) {
        let recorded = entry
            .result
            .as_ref()
            .and_then(|result| result.get("instanceId"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| entry.instance.clone().filter(|id| !id.starts_with('@')));
        if let (Some(recorded), Some(produced)) = (recorded, produced.clone()) {
            self.by_recorded.insert(recorded, produced);
        }
        self.by_index.push(produced);
    }
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

/// A step prepared for a signer: its intent, its exact transactions and the
/// session that may submit them.
#[derive(Debug, Clone)]
pub struct Prepared {
    pub execution_id: String,
    pub instance_id: Option<String>,
    /// The protocol session token prepare used; submit needs the same one.
    pub token: String,
    /// The exact bytes to sign, one per transaction.
    pub transactions: Vec<Vec<u8>>,
    /// The prepare response: `intent`, `anatomy`, `simulation`, ...
    pub response: Value,
}

/// The context of a [`DevClient::submit_prepared`] error raised before the
/// local runtime took the signed transactions (it refused them, or could not
/// be reached): the prepared step may be submitted again. Any other error
/// came after the runtime took them; do not resubmit.
#[derive(Debug, Clone)]
pub struct NotSubmitted {
    pub step: String,
}

impl std::fmt::Display for NotSubmitted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Submit of '{}' failed", self.step)
    }
}

/// Public phase of a dev protocol step. No signing material is exposed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepProgress<'a> {
    Preparing,
    Signing,
    Submitting { execution_id: &'a str },
    Confirming { execution_id: &'a str },
}

/// A prepare a guard (or access rule) refused.
#[derive(Debug, Clone)]
pub struct Rejection {
    pub code: String,
    pub detail: Value,
}

impl Rejection {
    /// The step result a rejection is (`state: rejected`).
    pub fn result(&self, step: &str, wallet: Value) -> Value {
        json!({
            "step": step,
            "wallet": wallet,
            "state": "rejected",
            "rejected": self.code,
            "detail": self.detail,
        })
    }
}

#[derive(Debug, Clone)]
pub enum PreparedStep {
    Ready(Box<Prepared>),
    Rejected(Rejection),
}

/// The human-readable reason in a rejection's detail: its `message` or
/// `error`, or the runtime's own `detail` (a string or one with a
/// `message`).
pub fn rejection_message(detail: &Value) -> Option<&str> {
    detail["message"]
        .as_str()
        .or(detail["error"].as_str())
        .or(detail["detail"].as_str())
        .or(detail["detail"]["message"].as_str())
}

/// One `history.jsonl` (or `replays.jsonl`) line: the request with the
/// result's identities and state.
fn step_record(schema: &str, request: &StepRequest, result: &Value) -> Value {
    let mut entry = serde_json::to_value(request).unwrap_or(Value::Null);
    entry["schema"] = json!(schema);
    entry["result"] = json!({
        "executionId": result["executionId"],
        "instanceId": result["instanceId"],
        "state": result["state"],
    });
    if let Some(code) = result["rejected"].as_str() {
        entry["result"]["rejected"] = json!(code);
        let detail = &result["detail"];
        if let Some(message) = rejection_message(detail) {
            entry["result"]["message"] = json!(message);
        }
        if let Some(hint) = detail["hint"].as_str() {
            entry["result"]["hint"] = json!(hint);
        }
    }
    entry["at"] = json!(unix_now());
    entry
}

/// Append one JSON line (best effort: a step's outcome never depends on it).
fn append_line(path: &Path, value: &Value) {
    if let Ok(line) = serde_json::to_string(value) {
        if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(file, "{line}");
        }
    }
}

/// Mint a read session for a stack's WebSocket with the account's API key.
/// With `deployment_id`, the session carries that protocol deployment's row
/// scopes.
pub async fn mint_stack_session(
    http: &reqwest::Client,
    session_endpoint: &str,
    secret_key: &str,
    ws_url: &str,
    deployment_id: Option<&str>,
) -> Result<StackSession> {
    let mut body = json!({ "websocket_url": ws_url });
    if let Some(deployment) = deployment_id {
        body["protocolDeploymentId"] = json!(deployment);
    }
    let response = http
        .post(session_endpoint)
        .bearer_auth(secret_key)
        .json(&body)
        .send()
        .await
        .context("The Arete session endpoint is unreachable")?;
    let status = response.status();
    let body: Value = response.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        bail!("Minting a stack session failed ({status}): {body}");
    }
    let token = body
        .get("token")
        .and_then(Value::as_str)
        .filter(|token| !token.is_empty())
        .ok_or_else(|| anyhow!("The session endpoint returned no token: {body}"))?
        .to_string();
    let expires_at = body
        .get("expires_at")
        .or_else(|| body.get("expiresAt"))
        .and_then(Value::as_u64)
        .unwrap_or_else(|| unix_now() + 3_600);
    Ok(StackSession {
        stream_url: stack_stream_url(ws_url, &token)?,
        token,
        expires_at,
    })
}

/// Talks to a project's cloud dev environment and the `a4 dev up` serving it.
#[derive(Debug, Clone)]
pub struct DevClient {
    pub project: DevProject,
    pub session: DevSession,
    /// The environment's RPC endpoint (and, through it, the API).
    pub env: EnvRpc,
    http: reqwest::Client,
}

impl DevClient {
    /// The project's environment, with the API key the a4 login (or
    /// `ARETE_API_KEY`) holds for its API.
    pub fn open(project: DevProject) -> Result<Self> {
        Self::open_with_key(project, None)
    }

    /// [`Self::open`] with the API key given, rather than resolved from the
    /// a4 login (`a4 dev up` and its dev UI already hold it).
    pub fn open_with_key(project: DevProject, key: Option<&str>) -> Result<Self> {
        let selection = project.load_selection()?.ok_or_else(|| {
            anyhow!(
                "{} has no dev environment yet. {DEV_UP_HINT}",
                project.root.display()
            )
        })?;
        let http = Self::http_client();
        let api = match key {
            Some(key) => EnvApi::new(http.clone(), &selection.api_url, key)?,
            None => EnvApi::for_origin(http.clone(), &selection.api_url)?,
        };
        let env = EnvRpc::new(api, &selection.id)?;
        let session = project.session_for(&selection);
        Ok(Self {
            project,
            session,
            env,
            http,
        })
    }

    pub fn new(project: DevProject, session: DevSession, env: EnvRpc) -> Self {
        Self {
            project,
            session,
            env,
            http: Self::http_client(),
        }
    }

    fn http_client() -> reqwest::Client {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .unwrap_or_default()
    }

    /// One JSON-RPC call to the environment; a JSON-RPC error is an error.
    pub async fn rpc(&self, method: &str, params: Value) -> Result<Value> {
        self.env.call(method, params).await
    }

    /// The environment clock's Unix timestamp (from the Clock sysvar).
    pub async fn clock(&self) -> Result<i64> {
        let result = self
            .rpc(
                "getAccountInfo",
                json!([CLOCK_SYSVAR, { "encoding": "jsonParsed", "commitment": "processed" }]),
            )
            .await?;
        result["value"]["data"]["parsed"]["info"]["unixTimestamp"]
            .as_i64()
            .ok_or_else(|| anyhow!("The environment returned no Clock sysvar"))
    }

    fn require_deployment(&self) -> Result<&str> {
        self.session.deployment_id.as_deref().ok_or_else(|| {
            anyhow!("No protocol deployment in this environment yet; `a4 dev up` deploys the project's protocol")
        })
    }

    /// The Arete protocol API, authenticated with the account's key.
    pub fn api(&self) -> ProtocolApi {
        ProtocolApi::new(
            self.http.clone(),
            &self.session.protocol_api(),
            &self.session.session_endpoint,
            self.env.api().key(),
        )
    }

    /// The deployment `a4 dev up` deployed.
    pub fn deployment(&self) -> Result<&str> {
        self.require_deployment()
    }

    /// A freshly minted stack session, minted with the account's key.
    /// `None` without a stack.
    pub async fn stack_session(&self) -> Result<Option<StackSession>> {
        let Some(ws_url) = self.session.stack_ws_url.as_deref() else {
            return Ok(None);
        };
        mint_stack_session(
            &self.http,
            &self.session.session_endpoint,
            self.env.api().key(),
            ws_url,
            self.session.deployment_id.as_deref(),
        )
        .await
        .map(Some)
    }

    /// The environment, the session and the chain clock.
    pub async fn status(&self) -> Value {
        let environment = match self.env.api().get_env(self.env.env()).await {
            Ok(view) => crate::dev_env::public_view(&view),
            Err(error) => json!({ "id": self.env.env(), "error": format!("{error:#}") }),
        };
        let running = environment["state"] == "running";
        let chain = if running {
            match (
                self.rpc("getSlot", json!([{ "commitment": "processed" }]))
                    .await,
                self.clock().await,
            ) {
                (Ok(slot), Ok(clock)) => {
                    json!({ "healthy": true, "slot": slot, "unixTimestamp": clock })
                }
                (Err(error), _) | (_, Err(error)) => {
                    json!({ "healthy": false, "error": format!("{error:#}") })
                }
            }
        } else {
            json!({ "healthy": false, "note": "The environment is not running; any RPC call (or `a4 dev up`) wakes it" })
        };
        let stack = match self.stack_session().await {
            Ok(Some(session)) => json!({
                "wsUrl": self.session.stack_ws_url,
                "streamUrl": redact_hs_tokens(&session.stream_url),
                "expiresAt": session.expires_at,
            }),
            Ok(None) => Value::Null,
            Err(error) => {
                json!({ "wsUrl": self.session.stack_ws_url, "error": format!("{error:#}") })
            }
        };
        let mut report = json!({
            "project": self.project.root.display().to_string(),
            "environment": environment,
            "upRunning": !self.session.rpc_url.is_empty(),
            "session": self.session.redacted(),
            "chain": chain,
            "stack": stack,
        });
        redact_secrets(&mut report);
        report
    }

    /// The dev wallet `name`, created on first use and funded whenever it
    /// is new or empty.
    pub async fn wallet(&self, name: &str) -> Result<DevWallet> {
        let (wallet, created) = DevWallet::load_or_create(&self.project.wallets_dir(), name)?;
        let empty = created
            || self
                .rpc(
                    "getBalance",
                    json!([wallet.pubkey(), { "commitment": "processed" }]),
                )
                .await?
                .get("value")
                .and_then(Value::as_u64)
                == Some(0);
        if empty {
            self.env
                .fund(&wallet.pubkey(), WALLET_FUND_LAMPORTS)
                .await
                .with_context(|| format!("Funding the new wallet '{name}' failed"))?;
        }
        Ok(wallet)
    }

    /// Read an instance; with `wallet` (a dev wallet name or a public key),
    /// its `allowedSteps` are evaluated for that wallet.
    pub async fn instance(&self, instance_id: &str, wallet: Option<&str>) -> Result<Value> {
        let wallet = wallet
            .map(|wallet| self.wallet_pubkey(wallet))
            .transpose()?;
        self.api()
            .instance(self.require_deployment()?, instance_id, wallet.as_deref())
            .await
    }

    /// What `wallet` (a dev wallet name or a public key) can do now: the
    /// deployment's create steps and the instances it relates to, each with
    /// its steps' availability.
    pub async fn actions(
        &self,
        wallet: &str,
        instance: Option<&str>,
        cursor: Option<&str>,
    ) -> Result<Value> {
        let wallet = self.wallet_pubkey(wallet)?;
        self.api()
            .wallet_actions(self.require_deployment()?, &wallet, instance, cursor, None)
            .await
    }

    /// The public key of the dev wallet `wallet`, or `wallet` itself when it
    /// is a public key. Never creates a wallet.
    pub fn wallet_pubkey(&self, wallet: &str) -> Result<String> {
        let path = self.project.wallets_dir().join(format!("{wallet}.json"));
        if validate_wallet_name(wallet).is_ok() && path.is_file() {
            return Ok(
                DevWallet::load_or_create(&self.project.wallets_dir(), wallet)?
                    .0
                    .pubkey(),
            );
        }
        if bs58::decode(wallet).into_vec().map(|bytes| bytes.len()) == Ok(32) {
            return Ok(wallet.to_string());
        }
        bail!("'{wallet}' is neither a dev wallet under {DEV_DIR}/wallets/ nor a base58 public key")
    }

    /// Every instance of `deployment` (default: the session's), newest first,
    /// up to `max` of them.
    pub async fn list_instances(&self, deployment: Option<&str>, max: usize) -> Result<Vec<Value>> {
        let deployment = match deployment {
            Some(deployment) => deployment.to_string(),
            None => self.require_deployment()?.to_string(),
        };
        let api = self.api();
        let mut instances = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let body = api
                .instances(&deployment, None, cursor.as_deref(), Some(100))
                .await?;
            instances.extend(body["instances"].as_array().cloned().unwrap_or_default());
            cursor = body["nextCursor"].as_str().map(str::to_string);
            if cursor.is_none() || instances.len() >= max {
                break;
            }
        }
        instances.truncate(max);
        Ok(instances)
    }

    /// The `a4 dev up` event feed since `after` (a cursor from an earlier
    /// call; everything still buffered when absent). With `wait_ms`, waits
    /// up to that long for the first new event.
    pub async fn events(
        &self,
        after: Option<&str>,
        limit: Option<usize>,
        wait_ms: Option<u64>,
    ) -> Result<Value> {
        let ui = self.session.ui_url.as_deref().ok_or_else(|| {
            anyhow!("This a4 dev session serves no event feed; restart it with a newer `a4 dev up`")
        })?;
        let mut url = reqwest::Url::parse(&format!("{}/dev/v1/events", ui.trim_end_matches('/')))
            .with_context(|| format!("'{ui}' is not a URL"))?;
        {
            let mut query = url.query_pairs_mut();
            if let Some(after) = after {
                query.append_pair("after", after);
            }
            if let Some(limit) = limit {
                query.append_pair("limit", &limit.to_string());
            }
            if let Some(wait) = wait_ms {
                query.append_pair("wait", &wait.min(30_000).to_string());
            }
        }
        let response = self
            .http
            .get(url)
            .header(reqwest::header::ACCEPT, "application/json")
            .send()
            .await
            .context("The a4 dev event feed is unreachable; is `a4 dev up` running?")?;
        let status = response.status();
        let body: Value = response.json().await.unwrap_or(Value::Null);
        if !status.is_success() {
            bail!("Reading the a4 dev event feed failed ({status}): {body}");
        }
        Ok(body)
    }

    /// Prepare, sign, submit and wait for one step, and append it to
    /// `history.jsonl`. A guard rejection is a result (`rejected`), not an
    /// error.
    pub async fn step(&self, request: &StepRequest) -> Result<Value> {
        self.step_with_progress(request, |_| {}).await
    }

    /// Like [`Self::step`], with phase notifications for interactive clients.
    /// Notifications contain only public execution ids, never tokens or bytes.
    pub async fn step_with_progress(
        &self,
        request: &StepRequest,
        mut progress: impl FnMut(StepProgress<'_>),
    ) -> Result<Value> {
        let result = self.run_step_with_progress(request, &mut progress).await?;
        append_line(
            &self.project.history_path(),
            &step_record(STEP_RECORD_SCHEMA, request, &result),
        );
        Ok(result)
    }

    /// [`Self::step`] without recording it.
    async fn run_step(&self, request: &StepRequest) -> Result<Value> {
        self.run_step_with_progress(request, &mut |_| {}).await
    }

    async fn run_step_with_progress(
        &self,
        request: &StepRequest,
        progress: &mut impl FnMut(StepProgress<'_>),
    ) -> Result<Value> {
        progress(StepProgress::Preparing);
        let wallet = self.wallet(&request.wallet).await?;
        let identity = json!({ "name": wallet.name, "pubkey": wallet.pubkey() });
        let prepared = match self.prepare_step(request, &wallet.pubkey()).await? {
            PreparedStep::Rejected(rejection) => {
                return Ok(rejection.result(&request.step, identity));
            }
            PreparedStep::Ready(prepared) => prepared,
        };
        progress(StepProgress::Signing);
        let mut signed = Vec::with_capacity(prepared.transactions.len());
        for bytes in &prepared.transactions {
            signed.push(
                base64::engine::general_purpose::STANDARD.encode(wallet.sign_transaction(bytes)?),
            );
        }
        self.submit_prepared_with_progress(&prepared, &request.step, signed, identity, progress)
            .await
    }

    /// Prepare one step for `signer` (a base58 public key) through the local
    /// API with the dev secret key, in the v0 format wallets take. A guard
    /// rejection is [`PreparedStep::Rejected`], not an error.
    pub async fn prepare_step(&self, request: &StepRequest, signer: &str) -> Result<PreparedStep> {
        let deployment = self.require_deployment()?.to_string();
        let api = self.api();
        let token = api.token(&deployment, &TRANSACTION_SCOPES).await?;
        let prepared = api
            .prepare_with(
                &token,
                &PrepareRequest {
                    deployment: &deployment,
                    instance: request.instance.as_deref(),
                    step: &request.step,
                    args: &request.args,
                    wallet: signer,
                    transaction_format: "v0",
                },
            )
            .await?;
        let prepared = match prepared {
            ApiPrepared::Intent(prepared) => prepared,
            ApiPrepared::Rejected(rejection) => {
                return Ok(PreparedStep::Rejected(Rejection {
                    code: rejection["rejected"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string(),
                    detail: rejection,
                }));
            }
        };
        let execution_id = prepared["intent"]["executionId"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let mut transactions = Vec::new();
        for transaction in prepared["transactions"].as_array().into_iter().flatten() {
            let encoded = transaction["transactionBase64"]
                .as_str()
                .ok_or_else(|| anyhow!("A prepared transaction has no transactionBase64"))?;
            transactions.push(
                base64::engine::general_purpose::STANDARD
                    .decode(encoded)
                    .context("A prepared transaction is not base64")?,
            );
        }
        let instance_id = prepared["intent"]["instanceId"]
            .as_str()
            .map(str::to_string)
            .or_else(|| request.instance.clone());
        Ok(PreparedStep::Ready(Box::new(Prepared {
            execution_id,
            instance_id,
            token,
            transactions,
            response: prepared,
        })))
    }

    /// Submit the signed transactions of a prepared step (base64, the exact
    /// bytes prepare issued with the signatures added) and wait until its
    /// execution intent settles. `wallet` is reported as the signer.
    pub async fn submit_prepared(
        &self,
        prepared: &Prepared,
        step: &str,
        signed: Vec<String>,
        wallet: Value,
    ) -> Result<Value> {
        self.submit_prepared_with_progress(prepared, step, signed, wallet, &mut |_| {})
            .await
    }

    async fn submit_prepared_with_progress(
        &self,
        prepared: &Prepared,
        step: &str,
        signed: Vec<String>,
        wallet: Value,
        progress: &mut impl FnMut(StepProgress<'_>),
    ) -> Result<Value> {
        let api = self.api();
        let execution_id = &prepared.execution_id;
        progress(StepProgress::Submitting { execution_id });
        let submitted = api
            .submit(&prepared.token, execution_id, &signed)
            .await
            .map_err(|error| {
                error.context(NotSubmitted {
                    step: step.to_string(),
                })
            })?;
        progress(StepProgress::Confirming { execution_id });
        let waited = api
            .wait_intent(&prepared.token, execution_id, submitted, INTENT_TIMEOUT)
            .await?;
        if !waited.settled {
            bail!(
                "Execution {execution_id} is still '{}' after {}s; read it again later and do not resubmit",
                waited.state["state"].as_str().unwrap_or("unknown"),
                INTENT_TIMEOUT.as_secs()
            );
        }
        let intent = waited.state;
        let instance_id = prepared
            .instance_id
            .as_deref()
            .or_else(|| intent["instanceId"].as_str())
            .map(str::to_string);
        Ok(json!({
            "step": step,
            "wallet": wallet,
            "executionId": execution_id,
            "instanceId": instance_id,
            "state": intent["state"],
            "transactions": intent["transactions"],
        }))
    }

    /// Record a step run outside [`Self::step`] (the dev UI's prepare and
    /// submit) in `history.jsonl`, as `a4 dev step` would have.
    pub fn record_step(&self, request: &StepRequest, result: &Value) {
        append_line(
            &self.project.history_path(),
            &step_record(STEP_RECORD_SCHEMA, request, result),
        );
    }

    /// Run `steps` in order, resolving instance references. Entries recorded
    /// as failed are skipped unless `options.include_failed`. Stops at the
    /// first step that errors, or that does not complete when it was not
    /// expected to fail. Each step run is logged to `replays.jsonl`, never to
    /// the history.
    pub async fn replay(&self, steps: &[StepRequest], options: ReplayOptions) -> Result<Value> {
        let replay_id = uuid::Uuid::new_v4().to_string();
        let mut instances = ReplayInstances::default();
        let mut results = Vec::new();
        let mut skipped = Vec::new();
        for (index, entry) in steps.iter().enumerate() {
            let failure = recorded_failure(entry);
            if let (Some(state), false) = (failure, options.include_failed) {
                let used = instances.resolve(entry.instance.as_deref()).ok().flatten();
                instances.record(entry, used);
                skipped.push(json!({ "entry": index, "step": entry.step, "recordedState": state }));
                continue;
            }
            let mut request = entry.clone();
            request.instance = instances.resolve(entry.instance.as_deref())?;
            request.result = None;
            let mut result = self
                .run_step(&request)
                .await
                .with_context(|| format!("Replay entry {index} ('{}') failed", entry.step))?;
            let mut record = step_record(REPLAY_STEP_RECORD_SCHEMA, &request, &result);
            record["replay"] = json!(replay_id);
            record["entry"] = json!(index);
            append_line(&self.project.replay_log_path(), &record);
            let produced = result["instanceId"].as_str().map(str::to_string);
            instances.record(entry, produced);
            let completed = result["state"] == "completed";
            result["entry"] = json!(index);
            results.push(result);
            if !completed && failure.is_none() {
                return Ok(json!({
                    "replay": replay_id,
                    "completed": false,
                    "stoppedAt": index,
                    "results": results,
                    "skipped": skipped,
                }));
            }
        }
        Ok(
            json!({ "replay": replay_id, "completed": true, "results": results, "skipped": skipped }),
        )
    }

    /// Move the environment's clock forward.
    pub async fn time_travel(&self, target: TimeTarget) -> Result<Value> {
        let before = self.clock().await?;
        let to = match target {
            TimeTarget::Absolute(seconds) => seconds,
            TimeTarget::Forward(seconds) => before.saturating_add(seconds),
        };
        if to <= before {
            bail!("The environment's clock is at {before}; it only moves forward");
        }
        let result = self
            .env
            .warp(WarpTarget::TimestampMs(to.saturating_mul(1000)))
            .await?;
        let after = self.clock().await?;
        Ok(json!({ "before": before, "requested": to, "unixTimestamp": after, "result": result }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_lose_every_credential() {
        let mut report = json!({
            "stack": { "streamUrl": "wss://s.example.test/?a=1&hs_token=abc.def-ghi&b=2" },
            "session": { "stackSession": { "token": "abc.def-ghi", "expiresAt": 5 } },
            "errors": ["Authorization: Bearer abc.def-ghi"],
        });
        redact_secrets(&mut report);
        let text = report.to_string();
        assert!(!text.contains("abc.def-ghi"), "{text}");
        assert_eq!(
            report["stack"]["streamUrl"],
            "wss://s.example.test/?a=1&hs_token=<redacted>&b=2"
        );
        assert_eq!(report["session"]["stackSession"]["expiresAt"], 5);

        // Scrubbing again (status scrubs the URL, then the report, then the
        // CLI scrubs it once more) leaves one marker.
        let once = report.clone();
        redact_secrets(&mut report);
        redact_secrets(&mut report);
        assert_eq!(report, once);
        assert_eq!(
            redact_hs_tokens(&redact_hs_tokens("x?hs_token=abc&hs_token=def")),
            "x?hs_token=<redacted>&hs_token=<redacted>"
        );
    }

    fn key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    /// A v0 transaction with two required signers (`first`, `second`), one
    /// readonly key and an empty instruction list.
    fn v0_transaction(first: &SigningKey, second: &SigningKey) -> Vec<u8> {
        let mut message = vec![0x80, 2, 0, 1, 3];
        message.extend_from_slice(first.verifying_key().as_bytes());
        message.extend_from_slice(second.verifying_key().as_bytes());
        message.extend_from_slice(&[9u8; 32]);
        message.extend_from_slice(&[7u8; 32]); // blockhash
        message.push(0); // instructions
        message.push(0); // address table lookups
        let mut tx = vec![2];
        tx.extend_from_slice(&[0u8; 128]);
        tx.extend_from_slice(&message);
        tx
    }

    #[test]
    fn signs_the_slot_of_its_own_key() {
        let (first, second) = (key(1), key(2));
        let tx = v0_transaction(&first, &second);
        let signed = sign_transaction(&tx, &second).unwrap();
        let message = &tx[1 + 128..];
        assert_eq!(&signed[1..65], &[0u8; 64]);
        let signature = ed25519_dalek::Signature::from_slice(&signed[65..129]).unwrap();
        second
            .verifying_key()
            .verify_strict(message, &signature)
            .unwrap();
        assert_eq!(&signed[129..], message);
    }

    #[test]
    fn describes_v0_transactions() {
        let (first, second) = (key(1), key(2));
        let mut tx = v0_transaction(&first, &second);
        let signed = sign_transaction(&tx, &first).unwrap();
        let described = describe_transaction(&signed).unwrap();
        assert_eq!(described["version"], "v0");
        assert_eq!(described["signatures"], json!([true, false]));
        assert_eq!(described["requiredSignatures"], 2);
        let accounts = described["accounts"].as_array().unwrap();
        assert_eq!(accounts.len(), 3);
        assert_eq!(
            accounts[0]["pubkey"],
            bs58::encode(first.verifying_key().as_bytes()).into_string()
        );
        // Header [2 required, 0 readonly signed, 1 readonly unsigned].
        assert_eq!(accounts[0]["writable"], true);
        assert_eq!(accounts[1]["signer"], true);
        assert_eq!(accounts[2]["signer"], false);
        assert_eq!(accounts[2]["writable"], false);
        assert_eq!(described["instructions"], json!([]));
        tx.truncate(10);
        assert!(describe_transaction(&tx).is_err());
    }

    #[test]
    fn refuses_a_wallet_that_is_not_a_signer() {
        let tx = v0_transaction(&key(1), &key(2));
        let error = sign_transaction(&tx, &key(3)).unwrap_err();
        assert!(
            error.to_string().contains("not a required signer"),
            "{error}"
        );
        assert!(sign_transaction(&[1, 0], &key(1)).is_err());
    }

    #[test]
    fn signs_legacy_messages() {
        let signer = key(4);
        let mut tx = vec![1];
        tx.extend_from_slice(&[0u8; 64]);
        tx.extend_from_slice(&[1, 0, 0, 1]);
        tx.extend_from_slice(signer.verifying_key().as_bytes());
        tx.extend_from_slice(&[7u8; 32]);
        tx.push(0);
        let signed = sign_transaction(&tx, &signer).unwrap();
        assert_ne!(&signed[1..65], &[0u8; 64]);
    }

    #[test]
    fn lists_wallets_by_name_with_public_keys_only() {
        let dir = tempfile::tempdir().unwrap();
        let (bob, _) = DevWallet::load_or_create(dir.path(), "bob").unwrap();
        let (alice, _) = DevWallet::load_or_create(dir.path(), "alice").unwrap();
        std::fs::write(dir.path().join("notes.json"), "{}").unwrap();
        std::fs::write(dir.path().join("bad name.json"), "[]").unwrap();
        assert_eq!(
            list_wallets(dir.path()),
            vec![
                ("alice".to_string(), alice.pubkey()),
                ("bob".to_string(), bob.pubkey())
            ]
        );
        assert!(list_wallets(&dir.path().join("missing")).is_empty());
    }

    #[test]
    fn history_records_keep_rejection_codes() {
        let request = StepRequest {
            step: "close".into(),
            instance: Some("i1".into()),
            args: json!({}),
            wallet: "bob".into(),
            result: None,
        };
        let record = step_record(
            STEP_RECORD_SCHEMA,
            &request,
            &json!({ "state": "rejected", "rejected": "not_recipient",
                     "detail": { "message": "only the recipient closes" } }),
        );
        assert_eq!(record["result"]["rejected"], "not_recipient");
        assert_eq!(record["result"]["message"], "only the recipient closes");
        assert!(recorded_failure(&serde_json::from_value(record).unwrap()).is_some());

        // A terminal-state rejection as the prepare route reports it: the
        // runtime's detail nested, and the hint the client adds.
        let record = step_record(
            STEP_RECORD_SCHEMA,
            &request,
            &json!({ "state": "rejected", "rejected": "instance_terminal",
                     "detail": { "rejected": "instance_terminal", "step": "close",
                                 "detail": { "message": "the jar is closed", "state": "closed" },
                                 "hint": "read protocol_actions" } }),
        );
        assert_eq!(record["result"]["message"], "the jar is closed");
        assert_eq!(record["result"]["hint"], "read protocol_actions");
    }

    #[test]
    fn concurrent_wallet_creators_keep_one_saved_key() {
        use std::sync::{Arc, Barrier};
        let dir = tempfile::tempdir().unwrap();
        let barrier = Arc::new(Barrier::new(16));
        let threads: Vec<_> = (0..16)
            .map(|_| {
                let path = dir.path().to_path_buf();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    let (wallet, created) = DevWallet::load_or_create(&path, "shared").unwrap();
                    (wallet.pubkey(), created)
                })
            })
            .collect();
        let results: Vec<_> = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect();
        let (saved, created) = DevWallet::load_or_create(dir.path(), "shared").unwrap();
        assert!(!created);
        assert_eq!(results.iter().filter(|(_, created)| *created).count(), 1);
        assert!(results.iter().all(|(key, _)| key == &saved.pubkey()));
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&saved.path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn wallet_creation_preserves_an_invalid_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("saved.json");
        fs::write(&path, b"invalid keypair").unwrap();
        assert!(DevWallet::load_or_create(dir.path(), "saved").is_err());
        assert_eq!(fs::read(path).unwrap(), b"invalid keypair");
    }

    #[test]
    fn wallet_creation_refuses_a_directory_without_retrying() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("alice.json");
        fs::create_dir(&path).unwrap();
        let error = DevWallet::load_or_create(dir.path(), "alice").unwrap_err();
        assert!(error.to_string().contains("regular keypair file"));
        assert!(path.is_dir());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn wallet_creation_refuses_a_dangling_symlink_without_retrying() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("alice.json");
        let missing = dir.path().join("missing.json");
        std::os::unix::fs::symlink(&missing, &path).unwrap();
        assert!(DevWallet::load_or_create(dir.path(), "alice").is_err());
        assert_eq!(fs::read_link(&path).unwrap(), missing);
        assert!(!missing.exists());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn wallets_round_trip_as_solana_keypair_files() {
        let dir = tempfile::tempdir().unwrap();
        let (created, fresh) = DevWallet::load_or_create(dir.path(), "alice").unwrap();
        assert!(fresh);
        let (loaded, fresh) = DevWallet::load_or_create(dir.path(), "alice").unwrap();
        assert!(!fresh);
        assert_eq!(created.pubkey(), loaded.pubkey());
        let bytes: Vec<u8> =
            serde_json::from_slice(&fs::read(dir.path().join("alice.json")).unwrap()).unwrap();
        assert_eq!(bytes.len(), 64);
        assert_eq!(&bytes[32..], &created.pubkey_bytes());
        assert!(DevWallet::load_or_create(dir.path(), "../evil").is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(dir.path().join("alice.json"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn parses_time_targets() {
        assert_eq!(
            TimeTarget::parse("1767225600").unwrap(),
            TimeTarget::Absolute(1_767_225_600)
        );
        assert_eq!(TimeTarget::parse("+90").unwrap(), TimeTarget::Forward(90));
        assert_eq!(TimeTarget::parse("+15m").unwrap(), TimeTarget::Forward(900));
        assert_eq!(
            TimeTarget::parse("+2h").unwrap(),
            TimeTarget::Forward(7_200)
        );
        assert_eq!(
            TimeTarget::parse("+7d").unwrap(),
            TimeTarget::Forward(604_800)
        );
        for bad in ["", "+", "+0", "+5w", "-5", "soon", "0"] {
            assert!(TimeTarget::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn parses_replay_formats() {
        let document = r#"{"schema":"arete.dev-replay/v1","steps":[{"step":"open","args":{"amount":1}},{"step":"close","instance":"@0","wallet":"bob"}]}"#;
        let steps = parse_replay(document).unwrap();
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0].wallet, "default");
        assert_eq!(steps[1].instance.as_deref(), Some("@0"));

        let history = "{\"schema\":\"arete.dev-step/v1\",\"step\":\"open\",\"args\":{},\"wallet\":\"default\",\"result\":{\"instanceId\":\"i-1\"}}\n\n{\"step\":\"close\",\"instance\":\"i-1\",\"args\":{},\"wallet\":\"default\"}\n";
        let steps = parse_replay(history).unwrap();
        assert_eq!(steps.len(), 2);
        assert!(parse_replay(r#"{"schema":"other","steps":[]}"#).is_err());
        assert_eq!(
            parse_replay(r#"[{"step":"a"}]"#).unwrap()[0].args,
            json!({})
        );
    }

    #[test]
    fn replay_maps_recorded_instances() {
        let steps = parse_replay(
            "{\"step\":\"open\",\"result\":{\"instanceId\":\"old-1\"}}\n{\"step\":\"close\",\"instance\":\"old-1\"}\n{\"step\":\"poke\",\"instance\":\"@0\"}\n{\"step\":\"other\",\"instance\":\"elsewhere\"}",
        )
        .unwrap();
        let mut instances = ReplayInstances::default();
        assert_eq!(instances.resolve(None).unwrap(), None);
        instances.record(&steps[0], Some("new-1".into()));
        assert_eq!(
            instances
                .resolve(steps[1].instance.as_deref())
                .unwrap()
                .as_deref(),
            Some("new-1")
        );
        instances.record(&steps[1], Some("new-1".into()));
        assert_eq!(
            instances.resolve(Some("@0")).unwrap().as_deref(),
            Some("new-1")
        );
        assert_eq!(
            instances.resolve(Some("elsewhere")).unwrap().as_deref(),
            Some("elsewhere")
        );
        assert!(instances.resolve(Some("@9")).is_err());
        assert!(instances.resolve(Some("@x")).is_err());
    }

    /// The Arete API and an environment's RPC endpoint in one: the token
    /// route names `<origin>/rpc`, `POST /rpc` answers every method with
    /// `{value: 5}`, `/ws/sessions` mints a token, `prepare` rejects `close`
    /// and otherwise returns a legacy transaction for the requesting wallet,
    /// `submit` completes. Returns its origin and the prepare paths it saw.
    fn mock_runtime() -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        use std::io::{BufRead, BufReader, Read};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = seen.clone();
        let origin_for_thread = origin.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut first = String::new();
                reader.read_line(&mut first).unwrap();
                let path = first.split_whitespace().nth(1).unwrap_or("/").to_string();
                let mut length = 0usize;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line.trim().is_empty() {
                        break;
                    }
                    if let Some((name, value)) = line.split_once(':') {
                        if name.eq_ignore_ascii_case("content-length") {
                            length = value.trim().parse().unwrap();
                        }
                    }
                }
                let mut body = vec![0u8; length];
                reader.read_exact(&mut body).unwrap();
                let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                let (status, response) = if path == "/rpc" {
                    (
                        200,
                        json!({ "jsonrpc": "2.0", "id": 1, "result": { "value": 5 } }),
                    )
                } else if path.starts_with("/api/dev/environments/") && path.ends_with("/token") {
                    (
                        200,
                        json!({ "token": "env-token", "expiresAt": unix_now() + 900, "rpcUrl": format!("{origin_for_thread}/rpc") }),
                    )
                } else if path.starts_with("/api/dev/environments/") {
                    (
                        200,
                        json!({ "id": TEST_ENV, "name": "app", "state": "sleeping", "tier": "free", "surfpoolVersion": "x" }),
                    )
                } else if path == "/ws/sessions" {
                    (200, json!({ "token": "tok" }))
                } else if path.ends_with("/prepare") {
                    log.lock().unwrap().push(path.clone());
                    if path.contains("/steps/close/") {
                        (422, json!({ "rejected": "not_open" }))
                    } else {
                        let signer = bs58::decode(body["wallet"].as_str().unwrap())
                            .into_vec()
                            .unwrap();
                        let mut message = vec![1, 0, 0, 1];
                        message.extend_from_slice(&signer);
                        message.extend_from_slice(&[7u8; 32]);
                        message.push(0);
                        let mut tx = vec![1];
                        tx.extend_from_slice(&[0u8; 64]);
                        tx.extend_from_slice(&message);
                        let instance = path
                            .strip_prefix("/api/protocol-instances/")
                            .and_then(|rest| rest.split('/').next())
                            .unwrap_or("new-1")
                            .to_string();
                        (
                            200,
                            json!({
                                "intent": { "executionId": "e-1", "instanceId": instance },
                                "transactions": [{ "transactionBase64": base64::engine::general_purpose::STANDARD.encode(tx) }],
                            }),
                        )
                    }
                } else if path.ends_with("/submit") {
                    (200, json!({ "state": "completed", "transactions": [] }))
                } else if path.contains("/actions") || path.starts_with("/api/protocol-instances/")
                {
                    log.lock().unwrap().push(path.clone());
                    (200, json!({ "path": path }))
                } else {
                    (404, json!({}))
                };
                let text = response.to_string();
                let _ = std::io::Write::write_all(
                    &mut stream,
                    format!(
                        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                        text.len()
                    )
                    .as_bytes(),
                );
            }
        });
        (origin, seen)
    }

    const TEST_ENV: &str = "denv_01hzx3k8m9q2r4s5t6v7w8x9yz";

    fn test_session(origin: &str) -> DevSession {
        let mut session = DevSession::offline(&EnvSelection::new(TEST_ENV, "app", origin));
        session.deployment_id = Some("d-1".into());
        session
    }

    fn mock_client(dir: &Path, origin: &str) -> DevClient {
        let project = DevProject::new(dir);
        fs::create_dir_all(project.dev_dir()).unwrap();
        let api = EnvApi::new(reqwest::Client::new(), origin, "a4_sk_dev").unwrap();
        DevClient::new(
            project,
            test_session(origin),
            EnvRpc::new(api, TEST_ENV).unwrap(),
        )
    }

    fn lines(path: &Path) -> Vec<Value> {
        fs::read_to_string(path)
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    #[tokio::test]
    async fn step_progress_reports_real_phases_and_rejections_never_reach_signing() {
        let (origin, _) = mock_runtime();
        let dir = tempfile::tempdir().unwrap();
        let client = mock_client(dir.path(), &origin);
        let mut phases = Vec::new();
        let result = client
            .step_with_progress(&parse_replay(r#"{"step":"open"}"#).unwrap()[0], |phase| {
                phases.push(format!("{phase:?}"))
            })
            .await
            .unwrap();
        assert_eq!(result["state"], "completed");
        assert_eq!(
            phases,
            [
                "Preparing",
                "Signing",
                "Submitting { execution_id: \"e-1\" }",
                "Confirming { execution_id: \"e-1\" }"
            ]
        );
        assert_eq!(lines(&client.project.history_path()).len(), 1);
        phases.clear();
        let result = client
            .step_with_progress(
                &parse_replay(r#"{"step":"close","instance":"new-1"}"#).unwrap()[0],
                |phase| phases.push(format!("{phase:?}")),
            )
            .await
            .unwrap();
        assert_eq!(result["state"], "rejected");
        assert_eq!(phases, ["Preparing"]);
        assert_eq!(lines(&client.project.history_path()).len(), 2);
    }

    #[tokio::test]
    async fn replays_skip_recorded_failures_and_stay_out_of_the_history() {
        let (origin, seen) = mock_runtime();
        let dir = tempfile::tempdir().unwrap();
        let client = mock_client(dir.path(), &origin);

        // The original run: open completes, close is rejected, poke completes.
        let open = client
            .step(&parse_replay(r#"{"step":"open"}"#).unwrap()[0])
            .await
            .unwrap();
        assert_eq!(open["state"], "completed");
        let close = client
            .step(&parse_replay(r#"{"step":"close","instance":"new-1"}"#).unwrap()[0])
            .await
            .unwrap();
        assert_eq!(close["state"], "rejected");
        client
            .step(&parse_replay(r#"{"step":"poke","instance":"new-1"}"#).unwrap()[0])
            .await
            .unwrap();
        let history_path = client.project.history_path();
        let history = fs::read_to_string(&history_path).unwrap();
        assert_eq!(history.lines().count(), 3);
        let steps = parse_replay(&history).unwrap();
        assert_eq!(recorded_failure(&steps[1]), Some("rejected"));
        assert_eq!(recorded_failure(&steps[0]), None);

        seen.lock().unwrap().clear();
        let result = client
            .replay(&steps, ReplayOptions::default())
            .await
            .unwrap();
        assert_eq!(result["completed"], true, "{result}");
        assert_eq!(result["results"].as_array().unwrap().len(), 2);
        assert_eq!(
            result["skipped"],
            json!([{ "entry": 1, "step": "close", "recordedState": "rejected" }])
        );
        assert!(seen
            .lock()
            .unwrap()
            .iter()
            .all(|path| !path.contains("/close/")));
        // The history only holds the original steps; the replay has its own log.
        assert_eq!(fs::read_to_string(&history_path).unwrap(), history);
        let replayed = lines(&client.project.replay_log_path());
        assert_eq!(replayed.len(), 2);
        assert_eq!(replayed[0]["schema"], REPLAY_STEP_RECORD_SCHEMA);
        assert_eq!(replayed[1]["entry"], 2);
        assert_eq!(replayed[0]["replay"], result["replay"]);

        // --include-failed runs the rejected entry; failing again as recorded
        // does not stop the replay.
        let result = client
            .replay(
                &steps,
                ReplayOptions {
                    include_failed: true,
                },
            )
            .await
            .unwrap();
        assert_eq!(result["completed"], true, "{result}");
        assert_eq!(result["results"][1]["state"], "rejected");
        assert_eq!(result["results"].as_array().unwrap().len(), 3);
        assert_eq!(lines(&client.project.replay_log_path()).len(), 5);

        // A step with no recorded failure that does not complete stops it.
        let steps = parse_replay(
            r#"[{"step":"open"},{"step":"close","instance":"@0"},{"step":"poke","instance":"@0"}]"#,
        )
        .unwrap();
        let result = client
            .replay(&steps, ReplayOptions::default())
            .await
            .unwrap();
        assert_eq!(result["completed"], false);
        assert_eq!(result["stoppedAt"], 1);
        assert_eq!(fs::read_to_string(&history_path).unwrap(), history);
    }

    #[tokio::test]
    async fn actions_and_instance_reads_take_dev_wallet_names_or_keys() {
        let (origin, seen) = mock_runtime();
        let dir = tempfile::tempdir().unwrap();
        let client = mock_client(dir.path(), &origin);
        let (alice, _) = DevWallet::load_or_create(&client.project.wallets_dir(), "alice").unwrap();
        let read = client.actions("alice", Some("i-1"), None).await.unwrap();
        assert_eq!(
            read["path"],
            format!(
                "/api/protocol-deployments/d-1/wallets/{}/actions?instance=i-1",
                alice.pubkey()
            )
        );
        let key = bs58::encode([8u8; 32]).into_string();
        let read = client.instance("i-1", Some(&key)).await.unwrap();
        assert_eq!(
            read["path"],
            format!("/api/protocol-instances/i-1?wallet={key}")
        );
        let read = client.instance("i-1", None).await.unwrap();
        assert_eq!(read["path"], "/api/protocol-instances/i-1");
        // An unknown name is neither a dev wallet nor a key, and no wallet
        // file is created for it.
        assert!(client.actions("bob", None, None).await.is_err());
        assert!(!client.project.wallets_dir().join("bob.json").exists());
        assert_eq!(seen.lock().unwrap().len(), 3);
    }

    #[test]
    fn discovers_projects_and_checks_session_files() {
        let dir = tempfile::tempdir().unwrap();
        let project = DevProject::new(dir.path());
        let missing = DevProject::discover(Some(dir.path()))
            .unwrap_err()
            .to_string();
        assert!(missing.contains("a4 dev up"), "{missing}");

        let selection = EnvSelection::new(TEST_ENV, "app", "http://127.0.0.1:1");
        selection.save(&project.environment_path()).unwrap();
        let nested = dir.path().join("handlers");
        fs::create_dir_all(&nested).unwrap();
        let found = DevProject::discover(Some(&nested)).unwrap();
        assert_eq!(found.load_selection().unwrap().unwrap(), selection);

        // No session file: an offline session with the recorded deployment.
        write_private(
            &project.deploy_state_path(),
            json!({ "environment": TEST_ENV, "deploymentId": "d-9", "slug": "tip-jar" })
                .to_string()
                .as_bytes(),
        )
        .unwrap();
        let offline = project.session_for(&selection);
        assert_eq!(offline.deployment_id.as_deref(), Some("d-9"));
        assert_eq!(offline.protocol_slug.as_deref(), Some("tip-jar"));
        assert!(offline.rpc_url.is_empty());
        assert_eq!(offline.protocol_api(), "http://127.0.0.1:1/api");

        // A running session wins.
        let mut session = test_session("http://127.0.0.1:1");
        session.rpc_url = "http://127.0.0.1:3".into();
        session.rpc_ws_url = "ws://127.0.0.1:3".into();
        session.ui_url = Some("http://127.0.0.1:4/".into());
        write_private(
            &project.session_path(),
            &serde_json::to_vec(&session).unwrap(),
        )
        .unwrap();
        assert_eq!(found.load_session().unwrap(), session);
        assert_eq!(project.session_for(&selection), session);

        // A session on another API than the selection's is not used: it
        // would get the key checked for the selection's API.
        let elsewhere = EnvSelection::new(TEST_ENV, "app", "http://127.0.0.1:2");
        let offline = project.session_for(&elsewhere);
        assert_eq!(offline.api_url, "http://127.0.0.1:2");
        assert_eq!(offline.session_endpoint, "http://127.0.0.1:2/ws/sessions");

        // A session file naming another host is refused.
        for (field, bad, expected) in [
            ("apiUrl", "https://evil.example", "not an Arete API"),
            (
                "sessionEndpoint",
                "http://10.0.0.1/ws/sessions",
                "not an Arete API",
            ),
            ("rpcUrl", "https://rpc.example", "not a loopback URL"),
            ("uiUrl", "http://ui.example/", "not a loopback URL"),
            // Both are Arete origins, but the key was checked for apiUrl's.
            (
                "sessionEndpoint",
                "https://api.arete.run/ws/sessions",
                "is not on apiUrl",
            ),
        ] {
            let mut forged = serde_json::to_value(&session).unwrap();
            forged[field] = json!(bad);
            write_private(&project.session_path(), forged.to_string().as_bytes()).unwrap();
            let error = format!("{:#}", project.load_session().unwrap_err());
            assert!(error.contains(expected), "{field}: {error}");
        }
        // An older session file is refused with a hint.
        write_private(
            &project.session_path(),
            json!({ "schema": "arete.dev-session/v1" })
                .to_string()
                .as_bytes(),
        )
        .unwrap();
        assert!(project.load_session().is_err());
    }

    #[test]
    fn recognises_loopback_urls() {
        for url in [
            "http://127.0.0.1:8899",
            "http://localhost:3000/api",
            "https://LOCALHOST",
            "http://[::1]:80",
            "ws://127.0.0.2:1",
        ] {
            assert!(is_loopback_url(url), "{url}");
        }
        for url in [
            "https://api.arete.run",
            "http://127.0.0.1.evil.example",
            "http://localhost.evil.example",
            "http://user@evil.example",
            "file:///tmp/x",
            "not a url",
        ] {
            assert!(!is_loopback_url(url), "{url}");
        }
    }

    #[test]
    fn stack_sessions_build_stream_urls_and_the_session_file_holds_no_token() {
        assert_eq!(
            stack_stream_url("ws://127.0.0.1:61321", "a.b-c_d").unwrap(),
            "ws://127.0.0.1:61321/?hs_token=a.b-c_d"
        );
        assert!(stack_stream_url("not a url", "t").is_err());
        let mut session = test_session("http://127.0.0.1:1");
        session.stack = Some("tip-jar".into());
        session.stack_ws_url = Some("ws://127.0.0.1:5".into());
        let written = serde_json::to_value(&session).unwrap();
        assert!(written.get("stackSession").is_none(), "{written}");
        let round: DevSession = serde_json::from_value(written).unwrap();
        assert_eq!(round, session);
        // A file an older a4 wrote still parses; its token is not kept.
        let mut old = serde_json::to_value(&session).unwrap();
        old["stackSession"] = json!({ "token": "tok", "expiresAt": 10, "streamUrl": "ws://127.0.0.1:5/?hs_token=tok" });
        let parsed: DevSession = serde_json::from_value(old).unwrap();
        assert!(!serde_json::to_string(&parsed).unwrap().contains("tok\""));
    }

    #[tokio::test]
    async fn status_reads_the_environment_and_funds_new_wallets() {
        let (origin, _) = mock_runtime();
        let dir = tempfile::tempdir().unwrap();
        let client = mock_client(dir.path(), &origin);
        let status = client.status().await;
        assert_eq!(status["environment"]["state"], "sleeping");
        assert!(status["environment"].get("surfpoolVersion").is_none());
        assert_eq!(status["chain"]["healthy"], false);
        assert_eq!(status["upRunning"], false);
        assert_eq!(status["stack"], Value::Null);
        let wallet = client.wallet("carol").await.unwrap();
        assert!(client.project.wallets_dir().join("carol.json").exists());
        assert_eq!(wallet.name, "carol");
    }
}
