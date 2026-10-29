//! The edge: the production-shaped routes of the environment, served by
//! `a4 dev up` on its loopback RPC port.
//!
//! An app built with installed SDKs in dev mode sends every request here,
//! with no credential. The edge resolves the installed identity the request
//! names to the environment's target (`POST /api/dev/environments/:env/resolve`),
//! mints the environment token that target takes, and forwards:
//!
//! - `/chain/*`, `/transactions/v1/*`: the environment's gateway routes, with
//!   the environment token.
//! - `/v1/releases/<release>/*`: the environment's Program Read, at the
//!   release the environment serves, with a Program Read session minted for
//!   the environment.
//! - `/stacks/<liveAlias>?stackManifestHash=<hash>`: the environment's stack
//!   (WebSocket, and HTTP queries under the same path), with a stack session
//!   minted for the environment.
//! - `/api/protocol-deployments/<id | package:<package>>/*`: the Arete API,
//!   at the environment's deployment of the protocol, with the account's key.
//! - `POST /ws/sessions`: a token only the edge accepts, for code that always
//!   mints. The edge ignores every `Authorization` it receives.
//! - `GET /.well-known/arete-dev-edge`: `{"edge": 1}`, so an SDK can check
//!   the edge before it opens a socket.
//!
//! Requests other than stack sockets name their installed identity in the
//! `X-Arete-Dev-Identity` header: the resolve request for that one surface,
//! as JSON (`{"programReads":[{"programId", "programReadBindingId",
//! "programReleaseHash"}]}`, `{"chain": {...}}`, `{"protocols": [...]}`).
//! Identities are not credentials. The key and every token stay in this
//! process; the edge never returns one.
//!
//! Refusals use the API problem envelope plus `fix`, with the dev refusal
//! codes. Nothing falls back to production.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use arete_mcp::dev_env::{ApiFailure, EnvRpc};
use axum::body::Bytes;
use axum::extract::ws::{self, WebSocket};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite;

/// Every edge response carries this header with the value `1`.
pub const EDGE_HEADER: &str = "x-arete-dev-edge";
/// The installed identity a request is for (a one-surface resolve request).
pub const IDENTITY_HEADER: &str = "x-arete-dev-identity";
/// The check an SDK makes before it opens a socket.
pub const WELL_KNOWN_PATH: &str = "/.well-known/arete-dev-edge";

/// How long a resolve answer is reused. Short, so a deploy `a4 dev up` makes
/// is picked up by the next request.
const RESOLVE_TTL: Duration = Duration::from_secs(10);
/// A minted session is renewed this long before it expires.
const RENEW_BEFORE: i64 = 60;
/// The largest identity header accepted.
const MAX_IDENTITY: usize = 8 * 1024;
/// Response headers forwarded from a target; every other one is dropped.
const FORWARDED_RESPONSE_HEADERS: [&str; 5] = [
    "content-type",
    "retry-after",
    "x-error-code",
    "cache-control",
    "etag",
];

/// Protocol routes named by an id of their own, not a deployment: the
/// protocol client reads instances, settles intents and fetches receipts
/// here. The API checks the account may use each.
const PROTOCOL_BY_ID_PREFIXES: [&str; 3] = [
    "/api/protocol-instances/",
    "/api/execution-intents/",
    "/api/protocol-receipts/",
];

/// The dev refusal codes (the contract's table).
pub mod codes {
    pub const STACK_NOT_LOADED: &str = "devenv-stack-not-loaded";
    pub const PROGRAM_RELEASE_MISMATCH: &str = "devenv-program-release-mismatch";
    pub const PROTOCOL_NOT_DEPLOYED: &str = "devenv-protocol-not-deployed";
    pub const PROTOCOL_NOT_OWNED: &str = "devenv-protocol-not-owned";
    pub const SURFACE_UNAVAILABLE: &str = "devenv-surface-unavailable";
}

/// The HTTP status of a refusal code.
fn refusal_status(code: &str) -> StatusCode {
    match code {
        codes::STACK_NOT_LOADED
        | codes::PROGRAM_RELEASE_MISMATCH
        | codes::PROTOCOL_NOT_DEPLOYED => StatusCode::CONFLICT,
        codes::PROTOCOL_NOT_OWNED => StatusCode::FORBIDDEN,
        _ => StatusCode::SERVICE_UNAVAILABLE,
    }
}

/// A refusal in the API problem envelope, with `fix`.
pub fn refusal(status: StatusCode, code: &str, message: &str, fix: &str) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, "application/json")],
        json!({
            "schemaVersion": 1,
            "error": message,
            "code": code,
            "retryable": false,
            "fix": fix,
        })
        .to_string(),
    )
        .into_response()
}

fn unavailable(message: &str, fix: &str) -> Response {
    refusal(
        StatusCode::SERVICE_UNAVAILABLE,
        codes::SURFACE_UNAVAILABLE,
        message,
        fix,
    )
}

fn bad_request(message: &str) -> Response {
    refusal(
        StatusCode::BAD_REQUEST,
        "invalid-request",
        message,
        "Send the request through an installed SDK in dev mode.",
    )
}

/// An API failure (resolve, a session mint) as the edge's answer.
fn api_refusal(error: &anyhow::Error) -> Response {
    if let Some(failure) = error.downcast_ref::<ApiFailure>() {
        let status = StatusCode::from_u16(failure.status).unwrap_or(StatusCode::BAD_GATEWAY);
        let code = failure.code.as_deref().unwrap_or("api-error");
        let fix = failure
            .hint()
            .unwrap_or("Check `a4 dev status`; restart `a4 dev up` if the environment is gone.");
        return refusal(status, code, &failure.message, fix);
    }
    unavailable(
        &format!("{error:#}"),
        "Check your network and `a4 dev status`.",
    )
}

/// The surface a request path names.
#[derive(Debug, PartialEq, Eq)]
pub enum Route<'a> {
    WellKnown,
    Sessions,
    /// `/chain/*` or `/transactions/v1/*`: the resolve key and the path.
    Gateway(&'static str),
    /// `/v1/releases/<release>/<rest>`.
    ProgramRead {
        release: &'a str,
        rest: &'a str,
    },
    /// `/stacks/<alias>` and `/stacks/<alias>/<rest>`.
    Stack {
        alias: &'a str,
        rest: &'a str,
    },
    /// `/api/protocol-deployments/<segment>/<rest>`.
    Protocol {
        segment: &'a str,
        rest: &'a str,
    },
    /// A protocol route named by its own id (an instance, an execution
    /// intent, a receipt): forwarded as it is, with the account key.
    ProtocolById,
    /// Any other `/api/*` path.
    OtherApi,
    /// JSON-RPC and the health probe.
    Rpc,
}

pub fn route(path: &str) -> Route<'_> {
    if path == WELL_KNOWN_PATH {
        return Route::WellKnown;
    }
    if path == "/ws/sessions" {
        return Route::Sessions;
    }
    if path.starts_with("/chain/") {
        return Route::Gateway("chain");
    }
    if path.starts_with("/transactions/v1/") {
        return Route::Gateway("transactions");
    }
    if let Some(after) = path.strip_prefix("/v1/releases/") {
        let (release, rest) = after.split_once('/').unwrap_or((after, ""));
        return Route::ProgramRead { release, rest };
    }
    if let Some(after) = path.strip_prefix("/stacks/") {
        let (alias, rest) = after.split_once('/').unwrap_or((after, ""));
        return Route::Stack { alias, rest };
    }
    if let Some(after) = path.strip_prefix("/api/protocol-deployments/") {
        let (segment, rest) = after.split_once('/').unwrap_or((after, ""));
        return Route::Protocol { segment, rest };
    }
    if PROTOCOL_BY_ID_PREFIXES
        .iter()
        .any(|prefix| path.starts_with(prefix))
    {
        return Route::ProtocolById;
    }
    if path == "/api" || path.starts_with("/api/") {
        return Route::OtherApi;
    }
    Route::Rpc
}

/// One forwarded request's parts.
struct Outgoing<'a> {
    method: Method,
    url: String,
    bearer: &'a str,
    content_type: Option<&'a HeaderValue>,
    body: Bytes,
}

pub struct Edge {
    rpc: Arc<EnvRpc>,
    http: reqwest::Client,
    resolved: tokio::sync::Mutex<HashMap<String, (Instant, Value)>>,
    sessions: tokio::sync::Mutex<HashMap<String, (String, i64)>>,
}

impl Edge {
    pub fn new(rpc: Arc<EnvRpc>) -> Self {
        Self {
            http: rpc.api().http().clone(),
            rpc,
            resolved: tokio::sync::Mutex::new(HashMap::new()),
            sessions: tokio::sync::Mutex::new(HashMap::new()),
        }
    }

    fn env(&self) -> &str {
        self.rpc.env()
    }

    /// The resolve answer for `identity`, reused for [`RESOLVE_TTL`].
    async fn resolve(&self, identity: &Value) -> Result<Value> {
        let key = identity.to_string();
        if let Some((at, answer)) = self.resolved.lock().await.get(&key) {
            if at.elapsed() < RESOLVE_TTL {
                return Ok(answer.clone());
            }
        }
        let answer = self.rpc.api().resolve(self.env(), identity.clone()).await?;
        let mut cache = self.resolved.lock().await;
        if cache.len() > 512 {
            cache.clear();
        }
        cache.insert(key, (Instant::now(), answer.clone()));
        Ok(answer)
    }

    /// A session token for `request` (minted with `environment`), reused
    /// until a minute before it expires.
    async fn session(&self, mut request: Value) -> Result<String> {
        request["environment"] = json!(self.env());
        let key = request.to_string();
        let now = arete_mcp::dev::unix_now() as i64;
        if let Some((token, expires_at)) = self.sessions.lock().await.get(&key) {
            if *expires_at > now + RENEW_BEFORE {
                return Ok(token.clone());
            }
        }
        let answer = self.rpc.api().session(request).await?;
        let token = answer["token"]
            .as_str()
            .filter(|token| !token.is_empty())
            .ok_or_else(|| anyhow!("The Arete API minted no session token"))?
            .to_string();
        let expires_at = answer["expires_at"]
            .as_i64()
            .or_else(|| answer["expiresAt"].as_i64())
            .unwrap_or(now + 300);
        let mut cache = self.sessions.lock().await;
        if cache.len() > 512 {
            cache.clear();
        }
        cache.insert(key, (token.clone(), expires_at));
        Ok(token)
    }

    /// Answer an HTTP request on an edge route, or `None` for JSON-RPC.
    pub async fn handle(
        &self,
        method: &Method,
        uri: &Uri,
        headers: &HeaderMap,
        body: Bytes,
    ) -> Option<Response> {
        let path = uri.path();
        if route(path) == Route::Rpc {
            return None;
        }
        // A forwarded path never leaves its route: a target URL would
        // normalize `..` away, after the route was chosen and its
        // credential attached.
        if !safe_path(path) {
            return Some(bad_request(
                "The path has a dot segment, an encoded separator or a backslash",
            ));
        }
        Some(match route(path) {
            Route::Rpc => return None,
            Route::WellKnown => (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/json")],
                json!({ "edge": 1 }).to_string(),
            )
                .into_response(),
            Route::Sessions if method == Method::POST => {
                let expires_at = arete_mcp::dev::unix_now() + 3600;
                (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "application/json")],
                    json!({
                        "token": format!("edge_{}", uuid::Uuid::new_v4().simple()),
                        "expires_at": expires_at,
                        "expiresAt": expires_at,
                    })
                    .to_string(),
                )
                    .into_response()
            }
            Route::Sessions => bad_request("POST here"),
            Route::Gateway(surface) => self.gateway(surface, method, uri, headers, body).await,
            Route::ProgramRead { release, rest } => {
                self.program_read(release, rest, method, uri, headers, body)
                    .await
            }
            Route::Stack { alias, rest } => {
                self.stack_query(alias, rest, method, uri, headers, body)
                    .await
            }
            Route::Protocol { segment, rest } => {
                self.protocol(segment, rest, method, uri, headers, body)
                    .await
            }
            Route::ProtocolById => {
                let api = self.rpc.api();
                forward(
                    &self.http,
                    Outgoing {
                        method: method.clone(),
                        url: join(api.origin(), path, uri.query()),
                        bearer: api.key(),
                        content_type: headers.get(header::CONTENT_TYPE),
                        body,
                    },
                )
                .await
            }
            Route::OtherApi => unavailable(
                "The edge serves the protocol routes of the Arete API only",
                "Call other Arete API routes with your API key, not through the edge.",
            ),
        })
    }

    async fn gateway(
        &self,
        surface: &'static str,
        method: &Method,
        uri: &Uri,
        headers: &HeaderMap,
        body: Bytes,
    ) -> Response {
        let identity = match identity(headers, surface) {
            Ok(identity) => identity,
            Err(response) => return response,
        };
        let target = match self.target(&identity, surface).await {
            Ok(target) => target,
            Err(response) => return response,
        };
        let Some(endpoint) = target["endpoint"].as_str() else {
            return unavailable("resolve named no gateway endpoint", "Update a4.");
        };
        let mut renewed = false;
        loop {
            let token = match self.rpc.token(renewed).await {
                Ok(token) => token,
                Err(error) => return api_refusal(&error),
            };
            let response = forward(
                &self.http,
                Outgoing {
                    method: method.clone(),
                    url: join(endpoint, uri.path(), uri.query()),
                    bearer: &token.token,
                    content_type: headers.get(header::CONTENT_TYPE),
                    body: body.clone(),
                },
            )
            .await;
            if response.status() == StatusCode::UNAUTHORIZED && !renewed {
                renewed = true;
                continue;
            }
            return response;
        }
    }

    async fn program_read(
        &self,
        release: &str,
        rest: &str,
        method: &Method,
        uri: &Uri,
        headers: &HeaderMap,
        body: Bytes,
    ) -> Response {
        let identity = match identity(headers, "programReads") {
            Ok(identity) => identity,
            Err(response) => return response,
        };
        let installed = &identity["programReads"][0];
        if installed["programReleaseHash"].as_str() != Some(release) {
            return bad_request(
                "X-Arete-Dev-Identity names another Program Read release than the request path",
            );
        }
        let Some(binding) = installed["programReadBindingId"].as_str() else {
            return bad_request("X-Arete-Dev-Identity names no programReadBindingId");
        };
        let target = match self.target(&identity, "programReads").await {
            Ok(target) => target,
            Err(response) => return response,
        };
        let (Some(endpoint), Some(served)) = (
            target["endpoint"].as_str(),
            target["programReleaseHash"].as_str(),
        ) else {
            return unavailable("resolve named no Program Read target", "Update a4.");
        };
        // The session authenticates the installed release; the path reads
        // the release the environment serves.
        let token = match self
            .session(json!({
                "targetKind": "program-read-binding",
                "targetId": binding,
                "programReleaseHash": release,
                "scopes": ["read"],
            }))
            .await
        {
            Ok(token) => token,
            Err(error) => return api_refusal(&error),
        };
        let path = if rest.is_empty() {
            format!("/v1/releases/{served}")
        } else {
            format!("/v1/releases/{served}/{rest}")
        };
        forward(
            &self.http,
            Outgoing {
                method: method.clone(),
                url: join(endpoint, &path, uri.query()),
                bearer: &token,
                content_type: headers.get(header::CONTENT_TYPE),
                body,
            },
        )
        .await
    }

    /// The environment's stack for `alias` and the request's
    /// `stackManifestHash`, and a session for it. `Err` is the refusal.
    #[allow(clippy::result_large_err)] // the refusal is the response to send
    async fn stack_target(
        &self,
        alias: &str,
        query: &[(String, String)],
    ) -> Result<(Value, String), Response> {
        let Some(manifest) = query
            .iter()
            .find(|(name, _)| name == "stackManifestHash")
            .map(|(_, value)| value.as_str())
        else {
            return Err(refusal(
                StatusCode::CONFLICT,
                codes::STACK_NOT_LOADED,
                "The request names no stackManifestHash, so the edge cannot tell which stack it is",
                "Use a stack SDK installed from the registry (`a4 install stack`).",
            ));
        };
        if alias.is_empty() {
            return Err(bad_request("The stack path names no live alias"));
        }
        let identity = json!({
            "stacks": [{ "stackManifestHash": manifest, "liveAlias": alias }]
        });
        let target = self.target(&identity, "stacks").await?;
        let (Some(websocket_url), Some(served)) = (
            target["websocketUrl"].as_str(),
            target["stackManifestHash"].as_str(),
        ) else {
            return Err(unavailable("resolve named no stack target", "Update a4."));
        };
        let token = self
            .session(json!({
                "websocket_url": websocket_url,
                "scopes": ["read"],
                "stackManifestHash": served,
                "liveAlias": alias,
            }))
            .await
            .map_err(|error| api_refusal(&error))?;
        Ok((target, token))
    }

    async fn stack_query(
        &self,
        alias: &str,
        rest: &str,
        method: &Method,
        uri: &Uri,
        headers: &HeaderMap,
        body: Bytes,
    ) -> Response {
        let query = query_pairs(uri.query());
        let (target, token) = match self.stack_target(alias, &query).await {
            Ok(found) => found,
            Err(response) => return response,
        };
        let Some(http_url) = target["httpUrl"].as_str().filter(|url| !url.is_empty()) else {
            return unavailable(
                "The environment's stack serves its stream only, not HTTP queries",
                "Read the stack through its WebSocket in dev mode.",
            );
        };
        let forwarded = without(&query, "stackManifestHash");
        forward(
            &self.http,
            Outgoing {
                method: method.clone(),
                url: join(http_url, &format!("/{rest}"), forwarded.as_deref()),
                bearer: &token,
                content_type: headers.get(header::CONTENT_TYPE),
                body,
            },
        )
        .await
    }

    async fn protocol(
        &self,
        segment: &str,
        rest: &str,
        method: &Method,
        uri: &Uri,
        headers: &HeaderMap,
        body: Bytes,
    ) -> Response {
        let segment = percent_encoding::percent_decode_str(segment)
            .decode_utf8_lossy()
            .to_string();
        let identity = if headers.contains_key(IDENTITY_HEADER) {
            match identity(headers, "protocols") {
                Ok(identity) => identity,
                Err(response) => return response,
            }
        } else if let Some(package) = segment.strip_prefix("package:") {
            json!({ "protocols": [{ "package": package }] })
        } else {
            json!({ "protocols": [{ "package": segment, "deploymentId": segment }] })
        };
        let target = match self.target(&identity, "protocols").await {
            Ok(target) => target,
            Err(response) => return response,
        };
        let Some(deployment) = target["deploymentId"].as_str() else {
            return unavailable("resolve named no protocol deployment", "Update a4.");
        };
        let path = if rest.is_empty() {
            format!("/api/protocol-deployments/{deployment}")
        } else {
            format!("/api/protocol-deployments/{deployment}/{rest}")
        };
        let api = self.rpc.api();
        forward(
            &self.http,
            Outgoing {
                method: method.clone(),
                url: join(api.origin(), &path, uri.query()),
                bearer: api.key(),
                content_type: headers.get(header::CONTENT_TYPE),
                body,
            },
        )
        .await
    }

    /// The one answer of `surface` in the resolve answer for `identity`, if
    /// it is a target. `Err` is the refusal to send.
    #[allow(clippy::result_large_err)] // the refusal is the response to send
    async fn target(&self, identity: &Value, surface: &str) -> Result<Value, Response> {
        let answer = self
            .resolve(identity)
            .await
            .map_err(|error| api_refusal(&error))?;
        let one = match &answer[surface] {
            Value::Array(answers) => answers.first().cloned().unwrap_or(Value::Null),
            other => other.clone(),
        };
        match one["status"].as_str() {
            Some("target") => Ok(one),
            Some("refused") => {
                let code = one["code"].as_str().unwrap_or(codes::SURFACE_UNAVAILABLE);
                Err(refusal(
                    refusal_status(code),
                    code,
                    one["message"]
                        .as_str()
                        .unwrap_or("The environment refused this request"),
                    one["fix"]
                        .as_str()
                        .unwrap_or("Run `a4 dev up` in the project."),
                ))
            }
            _ => Err(unavailable(
                &format!("The environment did not answer for {surface}"),
                "Update a4 and the Arete API client to versions that agree.",
            )),
        }
    }

    /// Open the environment's stack socket for `alias` and relay `socket`
    /// to it until either side closes.
    pub async fn relay_stack(
        &self,
        alias: &str,
        query: Option<&str>,
        socket: WebSocket,
    ) -> Result<()> {
        let pairs = query_pairs(query);
        let (target, token) = match self.stack_target(alias, &pairs).await {
            Ok(found) => found,
            Err(response) => {
                let reason = refusal_text(response).await;
                let mut socket = socket;
                let _ = socket
                    .send(ws::Message::Close(Some(ws::CloseFrame {
                        code: 1008,
                        reason: reason.into(),
                    })))
                    .await;
                return Ok(());
            }
        };
        let mut url = url::Url::parse(target["websocketUrl"].as_str().unwrap_or_default())
            .context("resolve named an invalid stack WebSocket URL")?;
        url.set_query(without(&pairs, "stackManifestHash").as_deref());
        relay(socket, url.as_str(), &token).await
    }

    /// Relay a JSON-RPC subscription socket to the environment's.
    pub async fn relay_rpc(&self, socket: WebSocket) -> Result<()> {
        let token = self.rpc.token(false).await?;
        relay(
            socket,
            &arete_mcp::dev_env::ws_url_of(&token.rpc_url),
            &token.token,
        )
        .await
    }
}

/// A refusal response's body, shortened to fit a close frame's reason.
async fn refusal_text(response: Response) -> String {
    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap_or_default();
    let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    let text = format!(
        "{}: {}",
        body["code"].as_str().unwrap_or("refused"),
        body["error"].as_str().unwrap_or_default()
    );
    // A close reason is at most 123 bytes.
    let mut end = text.len().min(120);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_string()
}

/// Whether every segment of `path`, percent-decoded, is a plain name: no
/// `.` or `..`, no `/` or `\\` inside it.
pub fn safe_path(path: &str) -> bool {
    path.split('/').all(|segment| {
        let decoded = percent_encoding::percent_decode_str(segment).decode_utf8_lossy();
        decoded != "."
            && decoded != ".."
            && !decoded.contains('/')
            && !decoded.contains('\\')
            && !segment.contains('\\')
    })
}

/// The identity the request names for `surface`, as a resolve request.
/// `Err` is the refusal to send.
#[allow(clippy::result_large_err)] // the refusal is the response to send
fn identity(headers: &HeaderMap, surface: &str) -> Result<Value, Response> {
    let Some(value) = headers.get(IDENTITY_HEADER) else {
        return Err(unavailable(
            &format!("The request names no installed {surface} identity (X-Arete-Dev-Identity)"),
            "Use an SDK installed with `a4 install` in dev mode; it names the identity itself.",
        ));
    };
    if value.len() > MAX_IDENTITY {
        return Err(bad_request("X-Arete-Dev-Identity is too large"));
    }
    let parsed: Value = value
        .to_str()
        .ok()
        .and_then(|text| serde_json::from_str(text).ok())
        .ok_or_else(|| bad_request("X-Arete-Dev-Identity is not JSON"))?;
    let Some(object) = parsed.as_object() else {
        return Err(bad_request("X-Arete-Dev-Identity is not a JSON object"));
    };
    let one = match object.get(surface) {
        Some(Value::Array(items)) if items.len() == 1 && items[0].is_object() => true,
        Some(Value::Object(_)) => matches!(surface, "chain" | "transactions"),
        _ => false,
    };
    if !one || object.len() != 1 {
        return Err(bad_request(&format!(
            "X-Arete-Dev-Identity must name exactly one {surface} identity and nothing else"
        )));
    }
    Ok(parsed)
}

/// `base` with `path` appended and `query`, if any.
fn join(base: &str, path: &str, query: Option<&str>) -> String {
    let mut url = format!("{}{path}", base.trim_end_matches('/'));
    if let Some(query) = query.filter(|query| !query.is_empty()) {
        url.push('?');
        url.push_str(query);
    }
    url
}

fn query_pairs(query: Option<&str>) -> Vec<(String, String)> {
    query
        .map(|query| {
            url::form_urlencoded::parse(query.as_bytes())
                .map(|(name, value)| (name.into_owned(), value.into_owned()))
                .collect()
        })
        .unwrap_or_default()
}

/// The query string of `pairs` without `name`, if anything is left.
fn without(pairs: &[(String, String)], name: &str) -> Option<String> {
    let kept: Vec<_> = pairs.iter().filter(|(key, _)| key != name).collect();
    (!kept.is_empty()).then(|| {
        url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(kept)
            .finish()
    })
}

/// Send one request to a target and answer with its status, the forwarded
/// headers and the body.
async fn forward(http: &reqwest::Client, outgoing: Outgoing<'_>) -> Response {
    let method = reqwest::Method::from_bytes(outgoing.method.as_str().as_bytes())
        .unwrap_or(reqwest::Method::GET);
    let mut request = http
        .request(method, &outgoing.url)
        .bearer_auth(outgoing.bearer)
        .timeout(Duration::from_secs(120));
    if let Some(content_type) = outgoing.content_type.and_then(|value| value.to_str().ok()) {
        request = request.header(reqwest::header::CONTENT_TYPE, content_type);
    }
    if !outgoing.body.is_empty() {
        request = request.body(outgoing.body.to_vec());
    }
    let response = match request.send().await {
        Ok(response) => response,
        Err(error) => {
            return unavailable(
                &format!("The environment did not answer: {error}"),
                "Check `a4 dev status`; a sleeping environment wakes on the next request.",
            )
        }
    };
    let status =
        StatusCode::from_u16(response.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    // An event stream (an instance's events) is passed through as it
    // arrives, not buffered until the target closes it.
    let streaming = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("text/event-stream"));
    let mut headers = HeaderMap::new();
    for name in FORWARDED_RESPONSE_HEADERS {
        if let Some(value) = response
            .headers()
            .get(name)
            .and_then(|value| HeaderValue::from_bytes(value.as_bytes()).ok())
        {
            headers.insert(header::HeaderName::from_static(name), value);
        }
    }
    if streaming {
        let mut answer = (
            status,
            axum::body::Body::from_stream(response.bytes_stream()),
        )
            .into_response();
        answer.headers_mut().extend(headers);
        return answer;
    }
    let body = match response.bytes().await {
        Ok(body) => body,
        Err(error) => {
            return refusal(
                StatusCode::BAD_GATEWAY,
                codes::SURFACE_UNAVAILABLE,
                &format!("The environment's answer could not be read: {error}"),
                "Retry the request; check `a4 dev status` if it keeps failing.",
            )
        }
    };
    let mut answer = (status, body).into_response();
    answer.headers_mut().extend(headers);
    answer
}

/// Open `url` with `token` and relay `socket` to it both ways.
async fn relay(socket: WebSocket, url: &str, token: &str) -> Result<()> {
    let mut request = tungstenite::client::IntoClientRequest::into_client_request(url)
        .context("The environment's WebSocket URL is invalid")?;
    request.headers_mut().insert(
        header::AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {token}"))
            .context("The session token is not a header value")?,
    );
    let (upstream, _) = tokio_tungstenite::connect_async(request)
        .await
        .map_err(|error| anyhow!("The environment's WebSocket refused: {error}"))?;
    let (mut down_tx, mut down_rx) = socket.split();
    let (mut up_tx, mut up_rx) = upstream.split();
    let to_env = async {
        while let Some(Ok(message)) = down_rx.next().await {
            let Some(message) = to_upstream(message) else {
                continue;
            };
            let closing = message.is_close();
            if up_tx.send(message).await.is_err() || closing {
                break;
            }
        }
        let _ = up_tx.close().await;
    };
    let from_env = async {
        while let Some(Ok(message)) = up_rx.next().await {
            let Some(message) = to_downstream(message) else {
                continue;
            };
            let closing = matches!(message, ws::Message::Close(_));
            if down_tx.send(message).await.is_err() || closing {
                break;
            }
        }
        let _ = down_tx.close().await;
    };
    tokio::select! {
        _ = to_env => {}
        _ = from_env => {}
    }
    Ok(())
}

fn to_upstream(message: ws::Message) -> Option<tungstenite::Message> {
    Some(match message {
        ws::Message::Text(text) => tungstenite::Message::Text(text),
        ws::Message::Binary(bytes) => tungstenite::Message::Binary(bytes),
        ws::Message::Ping(bytes) => tungstenite::Message::Ping(bytes),
        ws::Message::Pong(bytes) => tungstenite::Message::Pong(bytes),
        ws::Message::Close(frame) => {
            tungstenite::Message::Close(frame.map(|frame| tungstenite::protocol::CloseFrame {
                code: frame.code.into(),
                reason: frame.reason,
            }))
        }
    })
}

fn to_downstream(message: tungstenite::Message) -> Option<ws::Message> {
    Some(match message {
        tungstenite::Message::Text(text) => ws::Message::Text(text),
        tungstenite::Message::Binary(bytes) => ws::Message::Binary(bytes),
        tungstenite::Message::Ping(bytes) => ws::Message::Ping(bytes),
        tungstenite::Message::Pong(bytes) => ws::Message::Pong(bytes),
        tungstenite::Message::Close(frame) => {
            ws::Message::Close(frame.map(|frame| ws::CloseFrame {
                code: frame.code.into(),
                reason: frame.reason,
            }))
        }
        tungstenite::Message::Frame(_) => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_name_their_surface() {
        assert_eq!(route(WELL_KNOWN_PATH), Route::WellKnown);
        assert_eq!(route("/ws/sessions"), Route::Sessions);
        assert_eq!(route("/chain/v1/account"), Route::Gateway("chain"));
        assert_eq!(
            route("/transactions/v1/send"),
            Route::Gateway("transactions")
        );
        assert_eq!(
            route("/v1/releases/arete:h1:program-release:sha256:aa/accounts/Reserve/x"),
            Route::ProgramRead {
                release: "arete:h1:program-release:sha256:aa",
                rest: "accounts/Reserve/x"
            }
        );
        assert_eq!(
            route("/stacks/default"),
            Route::Stack {
                alias: "default",
                rest: ""
            }
        );
        assert_eq!(
            route("/stacks/default/views/Reserve/list"),
            Route::Stack {
                alias: "default",
                rest: "views/Reserve/list"
            }
        );
        assert_eq!(
            route("/api/protocol-deployments/package:token-raise/instances"),
            Route::Protocol {
                segment: "package:token-raise",
                rest: "instances"
            }
        );
        assert_eq!(
            route("/api/protocol-instances/i-1/events"),
            Route::ProtocolById
        );
        assert_eq!(
            route("/api/execution-intents/e-1/submit"),
            Route::ProtocolById
        );
        assert_eq!(route("/api/protocol-receipts/keys"), Route::ProtocolById);
        assert_eq!(route("/api/protocols"), Route::OtherApi);
        assert_eq!(route("/api/keys"), Route::OtherApi);
        assert_eq!(route("/"), Route::Rpc);
        assert_eq!(route("/health"), Route::Rpc);
    }

    #[test]
    fn identity_names_exactly_one_surface() {
        let with = |value: &str| {
            let mut headers = HeaderMap::new();
            headers.insert(IDENTITY_HEADER, HeaderValue::from_str(value).unwrap());
            headers
        };
        assert!(identity(
            &with(r#"{"chain":{"solanaGatewayBindingId":"sgb_1"}}"#),
            "chain"
        )
        .is_ok());
        assert!(identity(
            &with(r#"{"programReads":[{"programReleaseHash":"r"}]}"#),
            "programReads"
        )
        .is_ok());
        for bad in [
            "not json",
            "[]",
            r#"{"programReads":[]}"#,
            r#"{"programReads":[{},{}]}"#,
            r#"{"programReads":[{}],"chain":{}}"#,
            r#"{"programReads":{}}"#,
        ] {
            assert!(identity(&with(bad), "programReads").is_err(), "{bad}");
        }
        assert!(identity(&HeaderMap::new(), "chain").is_err());
    }

    #[test]
    fn queries_keep_everything_but_the_manifest() {
        let pairs = query_pairs(Some("stackManifestHash=h&view=Reserve%2Flist&limit=5"));
        assert_eq!(
            without(&pairs, "stackManifestHash").as_deref(),
            Some("view=Reserve%2Flist&limit=5")
        );
        assert_eq!(
            without(
                &query_pairs(Some("stackManifestHash=h")),
                "stackManifestHash"
            ),
            None
        );
        assert_eq!(
            join("https://x.test/", "/a/b", Some("q=1")),
            "https://x.test/a/b?q=1"
        );
        assert_eq!(join("wss://s.test", "", None), "wss://s.test");
    }

    #[test]
    fn forwarded_paths_never_leave_their_route() {
        assert!(safe_path(
            "/api/protocol-deployments/package:token-raise/instances"
        ));
        assert!(safe_path(
            "/v1/releases/arete:h1:program-release:sha256:aa/accounts/X/y"
        ));
        for path in [
            "/api/protocol-deployments/package:x/../../protocols",
            "/api/protocol-deployments/package:x/%2e%2e/%2E%2E/protocols",
            "/api/protocol-deployments/package:x/.%2e/protocols",
            "/api/protocol-deployments/x/./instances",
            "/api/protocol-deployments/x%2f..%2fkeys/instances",
            "/chain/v1/..\\..\\api",
            "/stacks/default/%5c..",
        ] {
            assert!(!safe_path(path), "{path}");
        }
    }

    #[test]
    fn refusals_carry_their_status() {
        assert_eq!(
            refusal_status(codes::STACK_NOT_LOADED),
            StatusCode::CONFLICT
        );
        assert_eq!(
            refusal_status(codes::PROGRAM_RELEASE_MISMATCH),
            StatusCode::CONFLICT
        );
        assert_eq!(
            refusal_status(codes::PROTOCOL_NOT_OWNED),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            refusal_status(codes::SURFACE_UNAVAILABLE),
            StatusCode::SERVICE_UNAVAILABLE
        );
    }
}
