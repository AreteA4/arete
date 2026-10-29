//! The dev environment switch (`docs/internal/dev-environment-switch.md`).
//!
//! One switch decides where an installed SDK reads and sends:
//!
//! - **Production** (the default): the generated descriptors as they are.
//!   With the switch unset nothing here runs and no dev input is read.
//! - **Dev through the edge**: every surface goes to the loopback edge
//!   `a4 dev up` serves, with no credential. Requests name their installed
//!   identity in [`DEV_IDENTITY_HEADER`]; the edge resolves it.
//! - **Dev direct**: servers, CI and agents with a secret key resolve their
//!   installed identities against the environment
//!   (`POST /api/dev/environments/{env}/resolve`) and mint environment tokens.
//!
//! A surface the environment can't serve fails with an
//! [`AreteError::DevEnvironment`] carrying the wire code, message and fix. No
//! dev request ever falls back to production.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::auth::{AuthConfig, StackRelease};
use crate::error::AreteError;

/// `production` or `dev`; unset or empty is production.
pub const ARETE_ENVIRONMENT_ENV: &str = "ARETE_ENVIRONMENT";
/// The edge origin `a4 dev up` serves.
pub const ARETE_DEV_URL_ENV: &str = "ARETE_DEV_URL";
/// An environment id (`denv_…`) or name, for direct mode.
pub const ARETE_DEV_ENV_ENV: &str = "ARETE_DEV_ENV";
/// The file `a4 dev up` writes, found at or above the working directory.
pub const DEV_SESSION_FILE: &str = ".arete/dev/session.json";
/// Every edge HTTP response carries this header with the value `1`.
pub const DEV_EDGE_HEADER: &str = "X-Arete-Dev-Edge";
/// The installed identity an edge request is for (a one-surface resolve
/// request, as compact JSON).
pub const DEV_IDENTITY_HEADER: &str = "X-Arete-Dev-Identity";
/// The check made before the first socket to an edge.
pub const DEV_EDGE_WELL_KNOWN_PATH: &str = "/.well-known/arete-dev-edge";

const DEFAULT_API_ORIGIN: &str = "https://api.arete.run";

/// Wire codes of the dev refusals.
pub mod codes {
    pub const NOT_CONFIGURED: &str = "devenv-not-configured";
    pub const EDGE_UNREACHABLE: &str = "devenv-edge-unreachable";
    pub const STACK_NOT_LOADED: &str = "devenv-stack-not-loaded";
    pub const PROGRAM_RELEASE_MISMATCH: &str = "devenv-program-release-mismatch";
    pub const PROTOCOL_NOT_DEPLOYED: &str = "devenv-protocol-not-deployed";
    pub const PROTOCOL_NOT_OWNED: &str = "devenv-protocol-not-owned";
    pub const SURFACE_UNAVAILABLE: &str = "devenv-surface-unavailable";
    pub const NOT_FOUND: &str = "devenv-not-found";
    pub const ENVIRONMENT_TOKEN_REQUIRED: &str = "environment-token-required";
    pub const ENVIRONMENT_TOKEN_REFUSED: &str = "environment-token-refused";
    /// A malformed switch, edge URL or environment reference
    /// ([`crate::AreteError::InvalidConfig`]).
    pub const INVALID_CONFIG: &str = "INVALID_CONFIG";
}

const NOT_CONFIGURED_FIX: &str = "Run the app with `a4 dev run -- <command>`, start `a4 dev up`, \
     or set `ARETE_DEV_ENV` with a secret key.";
const EDGE_UNREACHABLE_FIX: &str = "Start `a4 dev up` in the project (it serves the edge).";

/// Whether `code` is one of the dev refusal codes (and so becomes an
/// [`AreteError::DevEnvironment`] wherever a service answers with it).
pub fn is_dev_refusal_code(code: &str) -> bool {
    code.starts_with("devenv-")
        || code == codes::ENVIRONMENT_TOKEN_REQUIRED
        || code == codes::ENVIRONMENT_TOKEN_REFUSED
}

/// A dev refusal: the wire code, the message, the fix and every other
/// detail field the answer carried.
#[derive(Debug, Clone, PartialEq)]
pub struct DevEnvironmentError {
    pub code: String,
    pub message: String,
    pub fix: Option<String>,
    pub details: BTreeMap<String, Value>,
    /// HTTP status of the answer, when one was received.
    pub status: Option<u16>,
}

impl DevEnvironmentError {
    pub fn new(code: &str, message: impl Into<String>, fix: Option<&str>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
            fix: fix.map(str::to_string),
            details: BTreeMap::new(),
            status: None,
        }
    }

    pub(crate) fn into_error(self) -> AreteError {
        AreteError::DevEnvironment(Box::new(self))
    }
}

impl std::fmt::Display for DevEnvironmentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.message, self.code)?;
        if let Some(fix) = &self.fix {
            write!(f, ". {fix}")?;
        }
        Ok(())
    }
}

pub(crate) fn not_configured(message: &str) -> AreteError {
    DevEnvironmentError::new(codes::NOT_CONFIGURED, message, Some(NOT_CONFIGURED_FIX)).into_error()
}

pub(crate) fn edge_unreachable(edge_url: &str, detail: &str) -> AreteError {
    DevEnvironmentError::new(
        codes::EDGE_UNREACHABLE,
        format!("{edge_url} does not answer as the `a4 dev up` edge: {detail}"),
        Some(EDGE_UNREACHABLE_FIX),
    )
    .into_error()
}

pub(crate) fn surface_unavailable(message: impl Into<String>, fix: &str) -> AreteError {
    DevEnvironmentError::new(codes::SURFACE_UNAVAILABLE, message, Some(fix)).into_error()
}

pub(crate) fn stack_not_loaded(stack: &str) -> AreteError {
    DevEnvironmentError::new(
        codes::STACK_NOT_LOADED,
        format!(
            "Stack '{stack}' was not installed from the registry, so it has no release to \
             resolve in a dev environment"
        ),
        Some("Install the stack with `a4 install stack` and run `a4 dev up`."),
    )
    .into_error()
}

fn invalid(message: impl Into<String>) -> AreteError {
    AreteError::InvalidConfig(message.into())
}

// ---------------------------------------------------------------------------
// The switch
// ---------------------------------------------------------------------------

/// The `environment` option every client takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvironmentOption {
    /// Production, whatever the variables say.
    Production,
    /// Dev. With neither member set the target comes from `ARETE_DEV_URL`,
    /// `ARETE_DEV_ENV` or the session file.
    Dev {
        /// An edge URL: edge mode.
        url: Option<String>,
        /// An environment id or name: direct mode.
        env: Option<String>,
    },
}

impl EnvironmentOption {
    /// Dev, with the target from the variables or the session file.
    pub fn dev() -> Self {
        Self::Dev {
            url: None,
            env: None,
        }
    }

    /// Dev through the edge at `url`.
    pub fn dev_edge(url: impl Into<String>) -> Self {
        Self::Dev {
            url: Some(url.into()),
            env: None,
        }
    }

    /// Dev direct against the environment `env` (id or name).
    pub fn dev_direct(env: impl Into<String>) -> Self {
        Self::Dev {
            url: None,
            env: Some(env.into()),
        }
    }

    /// The option as an app passes it in JSON: `"production"`, or an object
    /// whose `kind` is `"dev"` and whose only other members are the strings
    /// `url` and `env`. Anything else is `INVALID_CONFIG`.
    pub fn from_json(value: &Value) -> Result<Self, AreteError> {
        match value {
            Value::String(text) if text == "production" => Ok(Self::Production),
            Value::Object(object) => {
                if object.get("kind").and_then(Value::as_str) != Some("dev") {
                    return Err(invalid(
                        "environment.kind must be 'dev' (or pass the string 'production')",
                    ));
                }
                let mut url = None;
                let mut env = None;
                for (name, member) in object {
                    let slot = match name.as_str() {
                        "kind" => continue,
                        "url" => &mut url,
                        "env" => &mut env,
                        other => {
                            return Err(invalid(format!(
                                "environment has an unknown member '{other}'; it takes kind, \
                                 url and env"
                            )))
                        }
                    };
                    let Some(text) = member.as_str() else {
                        return Err(invalid(format!("environment.{name} must be a string")));
                    };
                    *slot = Some(text.to_string());
                }
                Ok(Self::Dev { url, env })
            }
            _ => Err(invalid(
                "environment must be 'production' or { kind: 'dev', url?, env? }",
            )),
        }
    }
}

/// Which world a client talks to (`client.environment`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Environment {
    #[default]
    Production,
    Dev,
}

impl Environment {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Production => "production",
            Self::Dev => "dev",
        }
    }
}

impl std::fmt::Display for Environment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What the switch decided.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SwitchResult {
    #[default]
    Production,
    /// Dev through the edge; `edge_url` is normalized (no trailing slash).
    Edge { edge_url: String },
    /// Dev direct; `environment` is the id or name as given.
    Direct { environment: String },
}

impl SwitchResult {
    pub fn environment(&self) -> Environment {
        match self {
            Self::Production => Environment::Production,
            _ => Environment::Dev,
        }
    }
}

/// What the switch may read. Each lookup is only called when the answer
/// needs it, so production reads no dev input.
pub struct SwitchInputs<'a> {
    /// In a browser no variable and no file is read. Always `false` for
    /// native SDKs; kept for the shared vectors.
    pub browser: bool,
    pub read_var: &'a dyn Fn(&str) -> Option<String>,
    /// `edgeUrl` of the nearest `.arete/dev/session.json`, if any.
    pub session_edge_url: &'a dyn Fn() -> Option<String>,
    /// Whether the SDK's normal key resolution finds a secret key.
    pub has_secret_key: &'a dyn Fn() -> bool,
}

/// Decide the switch (contract §2.3).
pub fn resolve_switch(
    option: Option<&EnvironmentOption>,
    inputs: &SwitchInputs<'_>,
) -> Result<SwitchResult, AreteError> {
    let (url, env) = match option {
        Some(EnvironmentOption::Production) => return Ok(SwitchResult::Production),
        Some(EnvironmentOption::Dev { url, env }) => (url.clone(), env.clone()),
        None => {
            if inputs.browser {
                return Ok(SwitchResult::Production);
            }
            match (inputs.read_var)(ARETE_ENVIRONMENT_ENV).as_deref() {
                None | Some("") | Some("production") => return Ok(SwitchResult::Production),
                Some("dev") => (None, None),
                Some(other) => {
                    return Err(invalid(format!(
                        "{ARETE_ENVIRONMENT_ENV} is '{other}'; it must be 'production' or 'dev'"
                    )))
                }
            }
        }
    };

    if let Some(url) = url {
        return edge(&url, "environment.url");
    }
    if let Some(env) = env {
        return direct(&env, "environment.env", inputs);
    }
    if !inputs.browser {
        if let Some(url) = (inputs.read_var)(ARETE_DEV_URL_ENV) {
            return edge(&url, ARETE_DEV_URL_ENV);
        }
        if let Some(env) = (inputs.read_var)(ARETE_DEV_ENV_ENV) {
            return direct(&env, ARETE_DEV_ENV_ENV, inputs);
        }
        if let Some(url) = (inputs.session_edge_url)() {
            return edge(&url, DEV_SESSION_FILE);
        }
    }
    Err(not_configured(
        "Dev mode is on, but no edge URL or environment is set",
    ))
}

fn edge(url: &str, source: &str) -> Result<SwitchResult, AreteError> {
    Ok(SwitchResult::Edge {
        edge_url: normalize_edge_url(url).map_err(|reason| {
            invalid(format!(
                "{source} '{url}' is not an edge URL: {reason}. It must be http(s)://, a \
                 loopback host (127.0.0.1, [::1] or localhost) and an explicit port"
            ))
        })?,
    })
}

fn direct(env: &str, source: &str, inputs: &SwitchInputs<'_>) -> Result<SwitchResult, AreteError> {
    if !is_environment_reference(env) {
        return Err(invalid(format!(
            "{source} '{env}' is not an environment id (denv_…) or name (a-z, 0-9 and -)"
        )));
    }
    if inputs.browser {
        return Err(not_configured(
            "Direct mode needs a secret key and never runs in a browser",
        ));
    }
    if !(inputs.has_secret_key)() {
        return Err(not_configured(&format!(
            "{source} names an environment, but no secret key was found to resolve it with"
        )));
    }
    Ok(SwitchResult::Direct {
        environment: env.to_string(),
    })
}

/// Decide the switch from the process: the variables, the nearest session
/// file and whether `auth` holds a secret key.
pub fn resolve_switch_from_process(
    option: Option<&EnvironmentOption>,
    auth: Option<&AuthConfig>,
) -> Result<SwitchResult, AreteError> {
    resolve_switch(
        option,
        &SwitchInputs {
            browser: false,
            read_var: &|name| std::env::var(name).ok(),
            session_edge_url: &|| {
                std::env::current_dir()
                    .ok()
                    .and_then(|dir| session_edge_url_from(&dir))
            },
            has_secret_key: &|| auth.is_some_and(AuthConfig::has_secret_key),
        },
    )
}

/// `edgeUrl` of the nearest `.arete/dev/session.json` at or above `start`.
pub fn session_edge_url_from(start: &Path) -> Option<String> {
    let file = start
        .ancestors()
        .map(|dir| dir.join(DEV_SESSION_FILE))
        .find(|file| file.is_file())?;
    let text = std::fs::read_to_string(file).ok()?;
    let value: Value = serde_json::from_str(&text).ok()?;
    value
        .get("edgeUrl")
        .and_then(Value::as_str)
        .filter(|url| !url.is_empty())
        .map(str::to_string)
}

/// The normalized edge URL (`scheme://host:port`), or why it is not one.
pub fn normalize_edge_url(url: &str) -> Result<String, &'static str> {
    let (scheme, rest) = if let Some(rest) = url.strip_prefix("http://") {
        ("http", rest)
    } else if let Some(rest) = url.strip_prefix("https://") {
        ("https", rest)
    } else {
        return Err("the scheme is not http or https");
    };
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, remainder) = rest.split_at(end);
    if !remainder.is_empty() && remainder != "/" {
        return Err("it has a path, query or fragment");
    }
    if authority.contains('@') {
        return Err("it has user information");
    }
    let (host, port) = if authority.starts_with('[') {
        let Some(close) = authority.find(']') else {
            return Err("the host is malformed");
        };
        let (host, after) = authority.split_at(close + 1);
        (host, after.strip_prefix(':'))
    } else {
        match authority.rsplit_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (authority, None),
        }
    };
    let host = host.to_ascii_lowercase();
    if !matches!(host.as_str(), "127.0.0.1" | "[::1]" | "localhost") {
        return Err("the host is not loopback");
    }
    let Some(port) = port else {
        return Err("it has no explicit port");
    };
    if port.is_empty() || port.len() > 5 || !port.bytes().all(|b| b.is_ascii_digit()) {
        return Err("the port is malformed");
    }
    match port.parse::<u16>() {
        Ok(port) if port > 0 => Ok(format!("{scheme}://{host}:{port}")),
        _ => Err("the port is out of range"),
    }
}

/// `denv_` + 26 lowercase Crockford base32 characters (the first at most
/// `7`), or a name of 1–63 of `a-z`, `0-9` and `-`.
pub fn is_environment_reference(value: &str) -> bool {
    if let Some(id) = value.strip_prefix("denv_") {
        let crockford = |b: u8| matches!(b, b'0'..=b'9' | b'a'..=b'h' | b'j' | b'k' | b'm' | b'n' | b'p'..=b't' | b'v'..=b'z');
        if id.len() == 26 && id.bytes().all(crockford) && id.as_bytes()[0] <= b'7' {
            return true;
        }
    }
    (1..=63).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

// ---------------------------------------------------------------------------
// Identities and resolve
// ---------------------------------------------------------------------------

/// A Solana gateway binding identity (`chain` or `transactions`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayIdentity {
    pub solana_gateway_binding_id: String,
}

/// A Program Read binding identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgramReadIdentity {
    /// The program's address. Generated Rust program accessors don't pass
    /// it to the read transport, so it is sent only when known; the binding
    /// id names the program.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program_id: Option<String>,
    pub program_read_binding_id: String,
    pub program_release_hash: String,
}

/// A stack release identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StackIdentity {
    pub stack_manifest_hash: String,
    pub live_alias: String,
}

impl From<&StackRelease> for StackIdentity {
    fn from(release: &StackRelease) -> Self {
        Self {
            stack_manifest_hash: release.stack_manifest_hash.clone(),
            live_alias: release.live_alias.clone(),
        }
    }
}

/// A protocol identity. No `deployment_id` before the first publish.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolIdentity {
    pub package: String,
    pub protocol_release_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deployment_id: Option<String>,
}

/// The resolve request (contract §4.1); also the edge identity header for
/// one surface.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chain: Option<GatewayIdentity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transactions: Option<GatewayIdentity>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub program_reads: Vec<ProgramReadIdentity>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stacks: Vec<StackIdentity>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub protocols: Vec<ProtocolIdentity>,
}

impl ResolveRequest {
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }

    /// The compact JSON an edge request carries in [`DEV_IDENTITY_HEADER`].
    pub fn identity_header(&self) -> String {
        serde_json::to_string(self).expect("identities serialize")
    }
}

/// One surface of a resolve answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    Chain,
    Transactions,
    ProgramRead(usize),
    Stack(usize),
    Protocol(usize),
}

impl Surface {
    fn label(self) -> &'static str {
        match self {
            Self::Chain => "Chain reads",
            Self::Transactions => "Transactions",
            Self::ProgramRead(_) => "Program Read",
            Self::Stack(_) => "The stack",
            Self::Protocol(_) => "The protocol",
        }
    }
}

/// The answer for `surface`: its target, or the refusal it becomes.
pub fn surface_target(
    response: &Value,
    surface: Surface,
) -> Result<&Map<String, Value>, AreteError> {
    let answer = match surface {
        Surface::Chain => response.get("chain"),
        Surface::Transactions => response.get("transactions"),
        Surface::ProgramRead(index) => response.get("programReads").and_then(|a| a.get(index)),
        Surface::Stack(index) => response.get("stacks").and_then(|a| a.get(index)),
        Surface::Protocol(index) => response.get("protocols").and_then(|a| a.get(index)),
    };
    let Some(answer) = answer.and_then(Value::as_object) else {
        return Err(surface_unavailable(
            format!(
                "{} is not available in this environment: resolve gave no answer for it",
                surface.label()
            ),
            "Update a4 and the SDK, then run `a4 dev up`.",
        ));
    };
    match answer.get("status").and_then(Value::as_str) {
        Some("target") => Ok(answer),
        Some("refused") => Err(refusal_from_object(answer, None)
            .unwrap_or_else(|| {
                DevEnvironmentError::new(
                    codes::SURFACE_UNAVAILABLE,
                    format!("{} was refused by the environment", surface.label()),
                    None,
                )
            })
            .into_error()),
        _ => Err(surface_unavailable(
            format!("{} has an unreadable resolve answer", surface.label()),
            "Update a4 and the SDK.",
        )),
    }
}

pub(crate) fn target_str<'a>(
    target: &'a Map<String, Value>,
    name: &str,
) -> Result<&'a str, AreteError> {
    target
        .get(name)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            surface_unavailable(
                format!("The resolve answer names no {name}"),
                "Update a4 and the SDK.",
            )
        })
}

/// Where a stack target answers HTTP queries; a stack that serves only its
/// stream fails queries (not the stream) with `devenv-surface-unavailable`.
pub(crate) fn stack_query_base(
    target: &Map<String, Value>,
    stack: &str,
) -> Result<String, AreteError> {
    target
        .get("httpUrl")
        .and_then(Value::as_str)
        .filter(|url| !url.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            surface_unavailable(
                format!(
                    "The environment's '{stack}' stack serves its stream only, not HTTP queries"
                ),
                "Read the stack through its views (WebSocket) in dev mode.",
            )
        })
}

/// A resolve answer's refusal object (`{status, code, message, fix, …}`).
fn refusal_from_object(
    object: &Map<String, Value>,
    status: Option<u16>,
) -> Option<DevEnvironmentError> {
    let code = object.get("code").and_then(Value::as_str)?.to_string();
    let message = object
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or(&code)
        .to_string();
    let fix = object
        .get("fix")
        .and_then(Value::as_str)
        .map(str::to_string);
    let details = object
        .iter()
        .filter(|(name, _)| !matches!(name.as_str(), "status" | "code" | "message" | "fix"))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    Some(DevEnvironmentError {
        code,
        message,
        fix,
        details,
        status,
    })
}

/// Read a refusal from a service answer: the API problem envelope plus
/// `fix`, or Program Read's `{"error": {"code", "detail": {...}}}`, with the
/// code also in `X-Error-Code`. `None` unless the code is a dev refusal
/// code.
pub fn parse_refusal(
    status: u16,
    header_code: Option<&str>,
    body: &[u8],
) -> Option<DevEnvironmentError> {
    let parsed: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    let header_code = header_code.map(str::trim).filter(|code| !code.is_empty());
    let mut error = if let Some(inner) = parsed.get("error").and_then(Value::as_object) {
        let code = header_code.map(str::to_string).or_else(|| {
            inner
                .get("code")
                .and_then(Value::as_str)
                .map(str::to_string)
        })?;
        let detail = inner.get("detail").and_then(Value::as_object);
        let message = detail
            .and_then(|detail| detail.get("message"))
            .or_else(|| inner.get("message"))
            .and_then(Value::as_str)
            .unwrap_or(&code)
            .to_string();
        let fix = detail
            .and_then(|detail| detail.get("fix"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let details = detail
            .map(|detail| {
                detail
                    .iter()
                    .filter(|(name, _)| !matches!(name.as_str(), "message" | "fix"))
                    .map(|(name, value)| (name.clone(), value.clone()))
                    .collect()
            })
            .unwrap_or_default();
        DevEnvironmentError {
            code,
            message,
            fix,
            details,
            status: Some(status),
        }
    } else {
        let object = parsed.as_object();
        let code = header_code.map(str::to_string).or_else(|| {
            object
                .and_then(|object| object.get("code"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })?;
        let text = |name: &str| {
            object
                .and_then(|object| object.get(name))
                .and_then(Value::as_str)
                .map(str::to_string)
        };
        let message = text("error")
            .or_else(|| text("message"))
            .unwrap_or_else(|| code.clone());
        let details = object
            .map(|object| {
                object
                    .iter()
                    .filter(|(name, _)| {
                        !matches!(
                            name.as_str(),
                            "schemaVersion"
                                | "error"
                                | "message"
                                | "code"
                                | "retryable"
                                | "fix"
                                | "requestId"
                        )
                    })
                    .map(|(name, value)| (name.clone(), value.clone()))
                    .collect()
            })
            .unwrap_or_default();
        DevEnvironmentError {
            message,
            fix: text("fix"),
            details,
            status: Some(status),
            code,
        }
    };
    if !is_dev_refusal_code(&error.code) {
        return None;
    }
    error.status = Some(status);
    Some(error)
}

/// The error a non-success answer becomes when it carries a dev refusal.
pub(crate) fn refusal_error(
    status: u16,
    header_code: Option<&str>,
    body: &[u8],
) -> Option<AreteError> {
    if (200..300).contains(&status) {
        return None;
    }
    parse_refusal(status, header_code, body).map(DevEnvironmentError::into_error)
}

// ---------------------------------------------------------------------------
// Edge planning
// ---------------------------------------------------------------------------

/// `ws(s)://<edge host>/stacks/<liveAlias>?stackManifestHash=<hash>`.
pub fn edge_stack_websocket_url(edge_url: &str, release: &StackRelease) -> String {
    let (base, query) = edge_stack_query(edge_url, release);
    let base = if let Some(rest) = base.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = base.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        base
    };
    format!("{base}?{}={}", query[0].0, query[0].1)
}

/// Where stack HTTP queries go through the edge, and the query parameters
/// added to each.
pub fn edge_stack_query(edge_url: &str, release: &StackRelease) -> (String, Vec<(String, String)>) {
    (
        format!(
            "{}/stacks/{}",
            edge_url.trim_end_matches('/'),
            encode_segment(&release.live_alias)
        ),
        vec![(
            "stackManifestHash".to_string(),
            release.stack_manifest_hash.clone(),
        )],
    )
}

/// The protocol API through the edge.
pub fn edge_protocol_api_url(edge_url: &str) -> String {
    format!("{}/api", edge_url.trim_end_matches('/'))
}

/// The deployment path segment through the edge: the deployment id, or
/// `package:<package>` before the first publish.
pub fn edge_protocol_segment(deployment_id: Option<&str>, package: &str) -> String {
    match deployment_id.filter(|id| !id.is_empty()) {
        Some(id) => encode_segment(id),
        None => format!("package:{}", encode_segment(package)),
    }
}

/// `encodeURIComponent` with `:` kept, as the SDKs encode identities in
/// paths.
pub(crate) fn encode_segment(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.!~*'():".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// How every request of one surface goes through the edge: the identity
/// header it carries, the query parameters added to it, and the edge whose
/// marker the answer must carry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EdgeRequestPolicy {
    pub edge_url: String,
    /// [`DEV_IDENTITY_HEADER`], when the surface sends one.
    pub identity: Option<String>,
    pub query: Vec<(String, String)>,
}

impl EdgeRequestPolicy {
    pub fn new(edge_url: &str, identity: Option<&ResolveRequest>) -> Self {
        Self {
            edge_url: edge_url.to_string(),
            identity: identity.map(ResolveRequest::identity_header),
            query: Vec::new(),
        }
    }

    pub(crate) fn apply(&self, mut builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if let Some(identity) = &self.identity {
            builder = builder.header(DEV_IDENTITY_HEADER, identity);
        }
        if !self.query.is_empty() {
            builder = builder.query(&self.query);
        }
        builder
    }

    /// A send error: the edge did not answer.
    pub(crate) fn send_error(&self, error: &reqwest::Error) -> AreteError {
        edge_unreachable(&self.edge_url, &error.to_string())
    }

    /// Fails unless the answer came from the edge.
    pub(crate) fn check(&self, headers: &reqwest::header::HeaderMap) -> Result<(), AreteError> {
        if is_edge_answer(headers) {
            Ok(())
        } else {
            Err(edge_unreachable(
                &self.edge_url,
                "the answer has no X-Arete-Dev-Edge header",
            ))
        }
    }
}

pub(crate) fn is_edge_answer(headers: &reqwest::header::HeaderMap) -> bool {
    headers
        .get(DEV_EDGE_HEADER)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.trim() == "1")
}

/// `GET <edge>/.well-known/arete-dev-edge`: `200`, the marker header and
/// `{"edge": 1}`, or `devenv-edge-unreachable`.
pub async fn check_edge(http: &reqwest::Client, edge_url: &str) -> Result<(), AreteError> {
    let url = format!(
        "{}{DEV_EDGE_WELL_KNOWN_PATH}",
        edge_url.trim_end_matches('/')
    );
    let response = http
        .get(&url)
        .send()
        .await
        .map_err(|error| edge_unreachable(edge_url, &error.to_string()))?;
    let status = response.status().as_u16();
    let marked = is_edge_answer(response.headers());
    let body: Value = response.json().await.unwrap_or(Value::Null);
    if status == 200 && marked && body.get("edge").and_then(Value::as_i64) == Some(1) {
        Ok(())
    } else {
        Err(edge_unreachable(
            edge_url,
            &format!("{DEV_EDGE_WELL_KNOWN_PATH} answered {status} without the edge marker"),
        ))
    }
}

// ---------------------------------------------------------------------------
// Direct mode
// ---------------------------------------------------------------------------

/// Calls resolve for one client, with the client's secret key, and keeps
/// every answer for the life of the client.
pub struct DirectResolver {
    environment: String,
    url: String,
    auth: AuthConfig,
    http: reqwest::Client,
    /// Held across the request, so concurrent first uses resolve once.
    cache: tokio::sync::Mutex<HashMap<String, Arc<Value>>>,
}

impl std::fmt::Debug for DirectResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DirectResolver")
            .field("environment", &self.environment)
            .field("url", &self.url)
            .finish_non_exhaustive()
    }
}

/// The Arete API origin direct mode resolves and mints against.
pub(crate) fn api_origin() -> String {
    std::env::var(crate::protocol::ARETE_API_URL_ENV)
        .ok()
        .map(|url| url.trim().trim_end_matches('/').to_string())
        .filter(|url| !url.is_empty())
        .unwrap_or_else(|| DEFAULT_API_ORIGIN.to_string())
}

/// `<api>/api/dev/environments/<env>/resolve`.
pub fn resolve_url(api_origin: &str, environment: &str) -> String {
    format!(
        "{}/api/dev/environments/{}/resolve",
        api_origin.trim_end_matches('/'),
        encode_segment(environment)
    )
}

impl DirectResolver {
    pub(crate) fn new(environment: String, auth: AuthConfig, http: reqwest::Client) -> Self {
        let url = resolve_url(&api_origin(), &environment);
        Self {
            environment,
            url,
            auth,
            http,
            cache: tokio::sync::Mutex::new(HashMap::new()),
        }
    }

    /// This resolver calling resolve at `api_origin` (tests).
    #[cfg(test)]
    pub(crate) fn at(mut self, api_origin: &str) -> Self {
        self.url = resolve_url(api_origin, &self.environment);
        self
    }

    pub fn environment(&self) -> &str {
        &self.environment
    }

    /// The secret-key auth this client mints environment tokens with.
    pub(crate) fn auth(&self) -> &AuthConfig {
        &self.auth
    }

    pub(crate) fn http(&self) -> &reqwest::Client {
        &self.http
    }

    /// The answer for `request`, resolved once.
    pub async fn resolve(&self, request: &ResolveRequest) -> Result<Arc<Value>, AreteError> {
        let key = request.identity_header();
        let mut cache = self.cache.lock().await;
        if let Some(answer) = cache.get(&key) {
            return Ok(answer.clone());
        }
        let Some(key_value) = self.auth.api_key_for(&self.url)? else {
            return Err(not_configured(
                "Direct mode has no secret key to resolve with",
            ));
        };
        let response = self
            .http
            .post(&self.url)
            .bearer_auth(key_value)
            .json(request)
            .send()
            .await
            .map_err(|error| AreteError::ConnectionFailed(format!("Resolve failed: {error}")))?;
        let status = response.status().as_u16();
        let header_code = response
            .headers()
            .get(crate::http::HEADER_ERROR_CODE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let body = response
            .bytes()
            .await
            .map_err(|error| AreteError::ConnectionFailed(error.to_string()))?;
        if !(200..300).contains(&status) {
            if let Some(error) = refusal_error(status, header_code.as_deref(), &body) {
                return Err(error);
            }
            return Err(AreteError::from_auth_response(
                status,
                header_code.as_deref(),
                Some(&body),
                Some("Resolving the dev environment failed"),
                None,
            ));
        }
        let answer: Value = serde_json::from_slice(&body)
            .map_err(|error| AreteError::Serialization(error.to_string()))?;
        let answer = Arc::new(answer);
        cache.insert(key, answer.clone());
        Ok(answer)
    }

    /// A secret-key auth config minting environment tokens at
    /// `session_endpoint`.
    pub(crate) fn token_auth(&self, session_endpoint: &str) -> AuthConfig {
        let mut auth = self.auth.clone();
        auth.token = None;
        auth.get_token = None;
        auth.token_endpoint = Some(session_endpoint.to_string());
        auth.dev_environment = Some(self.environment.clone());
        auth
    }
}

/// The client's dev mode, carried to every transport it builds.
#[derive(Clone, Debug)]
pub enum DevMode {
    Edge { edge_url: String },
    Direct(Arc<DirectResolver>),
}

impl DevMode {
    /// The dev mode of `switch`, or `None` in production. Direct mode needs
    /// the secret key `auth` holds.
    pub(crate) fn of(
        switch: &SwitchResult,
        auth: Option<&AuthConfig>,
        http: &reqwest::Client,
    ) -> Result<Option<Self>, AreteError> {
        Ok(match switch {
            SwitchResult::Production => None,
            SwitchResult::Edge { edge_url } => Some(Self::Edge {
                edge_url: edge_url.clone(),
            }),
            SwitchResult::Direct { environment } => {
                let Some(auth) = auth.filter(|auth| auth.has_secret_key()) else {
                    return Err(not_configured("Direct mode needs a secret key"));
                };
                Some(Self::Direct(Arc::new(DirectResolver::new(
                    environment.clone(),
                    auth.clone(),
                    http.clone(),
                ))))
            }
        })
    }
}

/// A token source that fails every request with one error before anything
/// is sent: a surface the environment refused, or one dev mode can't serve.
pub(crate) struct RefusedTokens {
    pub(crate) error: AreteError,
}

#[async_trait::async_trait]
impl crate::http::TokenSource for RefusedTokens {
    async fn token(
        &self,
        _request: &crate::http::AuthTokenRequest,
        _force_refresh: bool,
    ) -> Result<Option<String>, AreteError> {
        Err(self.error.clone())
    }

    fn invalidate(&self, _request: &crate::http::AuthTokenRequest) {}
}

#[async_trait::async_trait]
impl crate::program_read_transport::BearerTokenSource for RefusedTokens {
    async fn bearer_token(
        &self,
        _scopes: &[&str],
        _target: Option<&crate::program_read_transport::ReadAuthTarget>,
        _force_refresh: bool,
    ) -> Result<Option<String>, AreteError> {
        Err(self.error.clone())
    }

    fn invalidate(
        &self,
        _scopes: &[&str],
        _target: Option<&crate::program_read_transport::ReadAuthTarget>,
    ) {
    }
}

/// Edge requests carry no credential: this source mints nothing and names
/// the edge policy the request follows.
pub(crate) struct EdgeTokens {
    pub(crate) policy: EdgeRequestPolicy,
}

#[async_trait::async_trait]
impl crate::http::TokenSource for EdgeTokens {
    async fn token(
        &self,
        _request: &crate::http::AuthTokenRequest,
        _force_refresh: bool,
    ) -> Result<Option<String>, AreteError> {
        Ok(None)
    }

    fn invalidate(&self, _request: &crate::http::AuthTokenRequest) {}

    fn edge_policy(&self) -> Option<&EdgeRequestPolicy> {
        Some(&self.policy)
    }
}

#[async_trait::async_trait]
impl crate::program_read_transport::BearerTokenSource for EdgeTokens {
    async fn bearer_token(
        &self,
        _scopes: &[&str],
        _target: Option<&crate::program_read_transport::ReadAuthTarget>,
        _force_refresh: bool,
    ) -> Result<Option<String>, AreteError> {
        Ok(None)
    }

    fn invalidate(
        &self,
        _scopes: &[&str],
        _target: Option<&crate::program_read_transport::ReadAuthTarget>,
    ) {
    }

    fn edge_policy(&self) -> Option<&EdgeRequestPolicy> {
        Some(&self.policy)
    }
}

/// The endpoint used for a surface that never sends: requests through it
/// fail at the token source before anything goes out.
pub(crate) const REFUSED_ENDPOINT: &str = "http://127.0.0.1:9";

/// The token request body of a targeted session in direct mode (contract
/// §5): the installed request plus `environment`.
pub fn environment_token_body(mut installed: Value, environment: &str) -> Value {
    if let Some(object) = installed.as_object_mut() {
        object.insert("environment".to_string(), json!(environment));
    }
    installed
}

#[cfg(test)]
mod vectors;
