//! The dev UI server `a4 dev up` runs on `127.0.0.1:<port base + 2>`.
//!
//! - `GET /dev/v1/events`: the event feed ([`events`]), as Server-Sent
//!   Events (`Last-Event-ID` or `?after=` resumes) or, with `Accept:
//!   application/json`, as one page (`?after=&limit=&wait=`), which the
//!   `dev_events` MCP tool reads.
//! - `GET /dev/v1/protocol`: the last good build (spec, machine map,
//!   anatomy), the last build's outcome, the active deployment, the dev
//!   environment, the dev wallets' public keys, the chain clock and the
//!   session's URLs. Never a key.
//! - `GET /dev/v1/instances` and `GET /dev/v1/instances/:id`: the active
//!   deployment's instances, through the Arete API.
//! - Running steps with dev wallets, airdrops, time travel and new dev
//!   wallets ([`run`]).
//! - `/`: the UI.
//!
//! Every request passes [`guard::check`].

/// The session's [`DevClient`], or return the 503 [`client`] answers.
macro_rules! session {
    ($state:expr) => {
        match super::client(&$state) {
            Ok(client) => client,
            Err(response) => return *response,
        }
    };
}

pub mod assets;
pub mod diagnostics;
pub mod diff;
pub mod events;
pub mod feeds;
pub mod guard;
pub mod run;
pub mod wallet;

use std::convert::Infallible;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use anyhow::Result;
use arete_mcp::dev::{list_wallets, DevClient, DevProject};
use axum::extract::{Path, Query, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use futures_util::Stream;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::broadcast::error::RecvError;
use tokio::task::JoinHandle;

use events::{Cursor, DevEvent, EventHub};

/// Shared by the routes and the feeds.
pub struct UiState {
    pub hub: Arc<EventHub>,
    pub project: DevProject,
    /// The account's API key (never served to the browser).
    api_key: Option<String>,
    pub policy: guard::Policy,
    /// `--ui-dev-url`: the Vite dev server the UI's files come from.
    pub ui_dev_url: Option<String>,
    snapshot: RwLock<Snapshot>,
    pub http: reqwest::Client,
    /// Steps prepared for a later submit.
    pub pending: run::PendingSteps,
}

/// What `GET /dev/v1/protocol` serves.
#[derive(Debug, Clone, Default)]
struct Snapshot {
    spec: Option<Value>,
    machine_map: Option<Value>,
    anatomy: Option<Value>,
    build: Value,
    deployment: Value,
    environment: Value,
    urls: Value,
    chain: Value,
    stack_error: Option<String>,
    activity_source: &'static str,
}

impl UiState {
    fn read(&self) -> std::sync::RwLockReadGuard<'_, Snapshot> {
        self.snapshot
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, Snapshot> {
        self.snapshot
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The project's client, with the key `a4 dev up` holds.
    pub fn dev_client(&self) -> anyhow::Result<DevClient> {
        DevClient::open_with_key(self.project.clone(), self.api_key.as_deref())
    }

    pub fn set_chain(&self, chain: Value) {
        self.write().chain = chain;
    }

    pub fn set_stack_error(&self, error: Option<String>) {
        self.write().stack_error = error;
    }

    pub fn set_activity_source(&self, source: &'static str) {
        self.write().activity_source = source;
    }

    /// The spec and machine map of the last good build, if any.
    pub fn last_good(&self) -> Option<(Value, Option<Value>)> {
        let snapshot = self.read();
        Some((snapshot.spec.clone()?, snapshot.machine_map.clone()))
    }
}

/// The running server and its feeds.
pub struct DevUi {
    pub state: Arc<UiState>,
    /// The UI's URL; `None` with `--no-ui` ([`disabled`]).
    pub url: Option<String>,
    tasks: Mutex<Vec<JoinHandle<()>>>,
}

pub struct UiOptions {
    pub project: DevProject,
    pub ui_dev_url: Option<String>,
    /// The account's API key, which the UI's calls use (never served).
    pub api_key: Option<String>,
}

/// Serve the UI on `listener` (already bound to 127.0.0.1).
pub fn start(listener: std::net::TcpListener, options: UiOptions) -> Result<DevUi> {
    let port = listener.local_addr()?.port();
    let state = ui_state(port, options);
    let url = format!("{}/", state.policy.origin());
    listener.set_nonblocking(true)?;
    let listener = tokio::net::TcpListener::from_std(listener)?;
    let app = router(state.clone());
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Ok(DevUi {
        state,
        url: Some(url),
        tasks: Mutex::new(vec![server]),
    })
}

/// `--no-ui`: no server and no feeds. The session still reports builds and
/// deploys to it, which costs nothing.
pub fn disabled(options: UiOptions) -> DevUi {
    DevUi {
        state: ui_state(0, options),
        url: None,
        tasks: Mutex::new(Vec::new()),
    }
}

fn ui_state(port: u16, options: UiOptions) -> Arc<UiState> {
    let extra: Vec<String> = options
        .ui_dev_url
        .iter()
        .filter_map(|url| origin_of(url))
        .collect();
    let policy = guard::Policy::new(port, &extra);
    Arc::new(UiState {
        hub: Arc::new(EventHub::new()),
        project: options.project,
        api_key: options.api_key,
        policy,
        ui_dev_url: options
            .ui_dev_url
            .map(|url| url.trim_end_matches('/').to_string()),
        snapshot: RwLock::new(Snapshot {
            build: json!({ "state": "pending" }),
            activity_source: "instances",
            ..Snapshot::default()
        }),
        http: reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_default(),
        pending: run::PendingSteps::default(),
    })
}

fn origin_of(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    Some(parsed.origin().ascii_serialization())
}

impl DevUi {
    fn spawn(&self, task: impl std::future::Future<Output = ()> + Send + 'static) {
        let handle = tokio::spawn(task);
        self.tasks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(handle);
    }

    /// Start the feeds that watch the deployment, the environment and its
    /// chain.
    pub fn start_feeds(&self, rpc: std::sync::Arc<arete_mcp::dev_env::EnvRpc>) {
        if self.url.is_none() {
            return;
        }
        self.state.set_activity_source("instances");
        self.spawn(feeds::instances(self.state.clone()));
        self.spawn(feeds::history(self.state.clone()));
        self.spawn(feeds::clock(self.state.clone(), rpc.clone()));
        self.spawn(feeds::environment(self.state.clone(), rpc));
    }

    /// Start streaming the protocol stack's rows (after the first deploy).
    pub fn start_stack_rows(&self) {
        if self.url.is_none() {
            return;
        }
        self.spawn(feeds::stack_rows(self.state.clone()));
    }

    pub fn set_urls(&self, urls: Value) {
        self.state.write().urls = urls;
    }

    /// The dev environment the session runs against (`{id, name}`).
    pub fn set_environment(&self, environment: Value) {
        self.state.write().environment = environment;
    }

    pub fn build_started(&self, changed: &[String]) {
        self.state.write().build =
            json!({ "state": "building", "at": events::now(), "changed": changed });
        self.state
            .hub
            .publish("build.started", json!({ "changed": changed }));
    }

    pub fn build_failed(&self, error: &anyhow::Error) {
        let message = format!("{error:#}");
        let diagnostics = diagnostics::parse(&message);
        let build = json!({
            "state": "failed",
            "at": events::now(),
            "error": message,
            "diagnostics": diagnostics,
        });
        self.state.write().build = build.clone();
        self.state.hub.publish("build.failed", build);
    }

    /// A good build: read its files, diff it against the last good one. A
    /// build without a readable `protocol-spec.json` is reported as failed,
    /// so the UI does not wait on it forever.
    pub fn build_succeeded(&self, build_dir: &std::path::Path) {
        let read = |name: &str| -> Option<Value> {
            serde_json::from_slice(&std::fs::read(build_dir.join(name)).ok()?).ok()
        };
        let spec_path = build_dir.join("protocol-spec.json");
        let spec = match std::fs::read(&spec_path) {
            Ok(bytes) => serde_json::from_slice::<Value>(&bytes)
                .map_err(|error| format!("{} is not valid JSON: {error}", spec_path.display())),
            Err(error) => Err(format!("could not read {}: {error}", spec_path.display())),
        };
        let spec = match spec {
            Ok(spec) => spec,
            Err(problem) => {
                self.build_failed(&anyhow::anyhow!(
                    "`arete-protocol build` finished but its output is unusable: {problem}"
                ));
                return;
            }
        };
        let machine_map = read("machine-map.json");
        let anatomy = read("anatomy.json");
        let release = read("protocol-release.json");
        let before = self.state.last_good();
        let diff = diff::spec_diff(
            before.as_ref().map(|(spec, map)| (spec, map.as_ref())),
            (&spec, machine_map.as_ref()),
        );
        let build = json!({
            "state": "ok",
            "at": events::now(),
            "protocol": spec["name"],
            "version": spec["version"],
            "specHash": release.as_ref().map(|release| release["specHash"].clone()),
            "diff": diff,
        });
        {
            let mut snapshot = self.state.write();
            snapshot.spec = Some(spec);
            snapshot.machine_map = machine_map;
            snapshot.anatomy = anatomy;
            snapshot.build = build.clone();
        }
        self.state.hub.publish("build.succeeded", build);
    }

    pub fn deployed(&self, deployment: Value, changed: bool) {
        self.state.write().deployment = deployment.clone();
        if changed {
            self.state.hub.publish("deploy.activated", deployment);
        }
    }

    pub fn deploy_failed(&self, error: &anyhow::Error) {
        self.state
            .hub
            .publish("deploy.failed", json!({ "error": format!("{error:#}") }));
    }

    pub fn shutdown(&self) {
        for task in self
            .tasks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .drain(..)
        {
            task.abort();
        }
    }
}

impl Drop for DevUi {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Every route, behind the guard.
pub fn router(state: Arc<UiState>) -> Router {
    Router::new()
        .route("/dev/v1/events", get(events_route))
        .route("/dev/v1/protocol", get(protocol_route))
        .route("/dev/v1/instances", get(instances_route))
        .route("/dev/v1/instances/:id", get(instance_route))
        .merge(run::routes())
        .fallback(assets::serve)
        .layer(middleware::from_fn_with_state(state.clone(), guard_layer))
        .with_state(state)
}

async fn guard_layer(State(state): State<Arc<UiState>>, request: Request, next: Next) -> Response {
    if let Err(refusal) = guard::check(
        &state.policy,
        request.method(),
        request.uri().path(),
        request.headers(),
    ) {
        let status = StatusCode::from_u16(refusal.status).unwrap_or(StatusCode::FORBIDDEN);
        return with_security_headers(
            (
                status,
                Json(json!({ "error": refusal.message, "code": refusal.code })),
            )
                .into_response(),
        );
    }
    with_security_headers(next.run(request).await)
}

fn with_security_headers(mut response: Response) -> Response {
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        "cross-origin-resource-policy",
        HeaderValue::from_static("same-origin"),
    );
    response
}

/// An error body.
pub fn error_response(status: StatusCode, code: &str, error: impl std::fmt::Display) -> Response {
    (
        status,
        Json(json!({ "error": format!("{error:#}"), "code": code })),
    )
        .into_response()
}

#[derive(Debug, Deserialize)]
struct EventsQuery {
    after: Option<String>,
    limit: Option<usize>,
    /// Milliseconds to wait for a first event (JSON reads only).
    wait: Option<u64>,
}

fn wants_json(headers: &HeaderMap) -> bool {
    let accept = headers
        .get(header::ACCEPT)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    accept.contains("application/json") && !accept.contains("text/event-stream")
}

async fn events_route(
    State(state): State<Arc<UiState>>,
    headers: HeaderMap,
    Query(query): Query<EventsQuery>,
) -> Response {
    let raw_cursor = headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
        .or(query.after.clone())
        .filter(|raw| !raw.is_empty());
    let cursor = match raw_cursor.as_deref().map(Cursor::parse) {
        Some(None) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "dev-ui-cursor-invalid",
                "The cursor must be <boot>:<seq> or <seq>",
            )
        }
        Some(Some(cursor)) => Some(cursor),
        None => None,
    };
    if wants_json(&headers) {
        return json_page(&state, cursor, &query).await;
    }
    Sse::new(sse_stream(state.hub.clone(), cursor))
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
        .into_response()
}

fn page_body(hub: &EventHub, cursor: Option<&Cursor>, limit: usize) -> Value {
    page_from(hub, hub.since(cursor, limit), cursor)
}

/// A page of `replay`. With no events, the next cursor is the head taken
/// with the replay, under the hub's lock: reading the head afterwards could
/// pass an event published in between, which the reader would never see.
fn page_from(hub: &EventHub, replay: events::Replay, cursor: Option<&Cursor>) -> Value {
    let next = match replay.events.last() {
        Some(last) => hub.event_id(last),
        None if replay.reset || cursor.is_none() => hub.cursor_at(replay.head),
        None => hub.cursor_at(cursor.map(|cursor| cursor.seq).unwrap_or_default()),
    };
    json!({
        "boot": hub.boot(),
        "cursor": next,
        "truncated": replay.reset,
        "more": replay.more,
        "events": replay.events,
    })
}

async fn json_page(state: &UiState, cursor: Option<Cursor>, query: &EventsQuery) -> Response {
    let limit = query
        .limit
        .unwrap_or(events::DEFAULT_PAGE)
        .clamp(1, events::MAX_PAGE);
    let hub = &state.hub;
    let mut receiver = hub.listen();
    let mut body = page_body(hub, cursor.as_ref(), limit);
    let empty = body["events"].as_array().is_none_or(Vec::is_empty);
    if let (true, Some(wait)) = (empty, query.wait.filter(|wait| *wait > 0)) {
        let deadline = tokio::time::Instant::now() + Duration::from_millis(wait.min(30_000));
        loop {
            match tokio::time::timeout_at(deadline, receiver.recv()).await {
                Ok(Ok(event)) if event.ephemeral() => continue,
                Ok(Ok(_)) | Ok(Err(RecvError::Lagged(_))) => {
                    body = page_body(hub, cursor.as_ref(), limit);
                    break;
                }
                Ok(Err(RecvError::Closed)) | Err(_) => break,
            }
        }
    }
    Json(body).into_response()
}

fn sse_event(hub: &EventHub, event: &DevEvent) -> Event {
    let data = serde_json::to_string(event).unwrap_or_else(|_| "{}".into());
    let sse = Event::default().event(&event.kind).data(data);
    if event.ephemeral() {
        sse
    } else {
        sse.id(hub.event_id(event))
    }
}

struct SseState {
    hub: Arc<EventHub>,
    pending: std::collections::VecDeque<Event>,
    receiver: tokio::sync::broadcast::Receiver<DevEvent>,
    last: u64,
}

/// The replay after `cursor`, a `stream.live` marker, then live events.
fn sse_stream(
    hub: Arc<EventHub>,
    cursor: Option<Cursor>,
) -> impl Stream<Item = Result<Event, Infallible>> {
    let (replay, receiver) = hub.subscribe(cursor.as_ref());
    let mut pending = std::collections::VecDeque::new();
    let mut last = cursor
        .as_ref()
        .filter(|_| !replay.reset)
        .map(|cursor| cursor.seq)
        .unwrap_or(0);
    if replay.reset {
        pending.push_back(sse_event(&hub, &events::reset_event(&hub)));
    }
    for event in &replay.events {
        last = event.seq;
        pending.push_back(sse_event(&hub, event));
    }
    pending.push_back(
        Event::default()
            .event("stream.live")
            .data(json!({ "boot": hub.boot(), "head": hub.cursor_at(replay.head) }).to_string()),
    );
    let state = SseState {
        hub,
        pending,
        receiver,
        last,
    };
    futures_util::stream::unfold(state, |mut state| async move {
        loop {
            if let Some(event) = state.pending.pop_front() {
                return Some((Ok(event), state));
            }
            match state.receiver.recv().await {
                Ok(event) if event.ephemeral() => {
                    let sse = sse_event(&state.hub, &event);
                    return Some((Ok(sse), state));
                }
                Ok(event) if event.seq > state.last => {
                    state.last = event.seq;
                    let sse = sse_event(&state.hub, &event);
                    return Some((Ok(sse), state));
                }
                Ok(_) => continue,
                Err(RecvError::Lagged(_)) => {
                    let cursor = Cursor {
                        boot: Some(state.hub.boot().to_string()),
                        seq: state.last,
                    };
                    let replay = state.hub.since(Some(&cursor), events::BUFFER_CAPACITY);
                    if replay.reset {
                        state
                            .pending
                            .push_back(sse_event(&state.hub, &events::reset_event(&state.hub)));
                    }
                    for event in replay.events {
                        state.last = event.seq;
                        state.pending.push_back(sse_event(&state.hub, &event));
                    }
                }
                Err(RecvError::Closed) => return None,
            }
        }
    })
}

async fn protocol_route(State(state): State<Arc<UiState>>) -> Response {
    let snapshot = state.read().clone();
    let wallets: Vec<Value> = list_wallets(&state.project.wallets_dir())
        .into_iter()
        .map(|(name, pubkey)| json!({ "name": name, "pubkey": pubkey }))
        .collect();
    let mut urls = snapshot.urls.clone();
    if let Some(object) = urls.as_object_mut() {
        object.insert("ui".into(), json!(format!("{}/", state.policy.origin())));
        object.insert(
            "events".into(),
            json!(format!("{}/dev/v1/events", state.policy.origin())),
        );
    }
    Json(json!({
        "schema": "arete.dev-ui-protocol/v1",
        "boot": state.hub.boot(),
        "cursor": state.hub.head(),
        "project": state.project.root.file_name().and_then(|name| name.to_str()),
        "spec": snapshot.spec,
        "machineMap": snapshot.machine_map,
        "anatomy": snapshot.anatomy,
        "build": snapshot.build,
        "deployment": snapshot.deployment,
        "environment": snapshot.environment,
        "wallets": wallets,
        "urls": urls,
        "chain": snapshot.chain,
        "stackError": snapshot.stack_error,
        "activitySource": snapshot.activity_source,
    }))
    .into_response()
}

/// The project's client, or a 503 while `a4 dev up` is still starting.
pub fn client(state: &UiState) -> Result<DevClient, Box<Response>> {
    state.dev_client().map_err(|error| {
        Box::new(error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "dev-ui-session-unavailable",
            error,
        ))
    })
}

async fn instances_route(State(state): State<Arc<UiState>>) -> Response {
    let client = match client(&state) {
        Ok(client) => client,
        Err(response) => return *response,
    };
    let deployment = client.session.deployment_id.clone();
    match client.list_instances(None, 1000).await {
        Ok(instances) => {
            Json(json!({ "deploymentId": deployment, "instances": instances })).into_response()
        }
        Err(error) => error_response(StatusCode::BAD_GATEWAY, "dev-ui-runtime-error", error),
    }
}

async fn instance_route(State(state): State<Arc<UiState>>, Path(id): Path<String>) -> Response {
    let client = match client(&state) {
        Ok(client) => client,
        Err(response) => return *response,
    };
    match client.instance(&id, None).await {
        Ok(instance) => Json(instance).into_response(),
        Err(error) => error_response(StatusCode::BAD_GATEWAY, "dev-ui-runtime-error", error),
    }
}

/// Open `url` in the default browser.
pub fn open_browser(url: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = std::process::Command::new("cmd");
        command.args(["/C", "start", ""]);
        command
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = std::process::Command::new("xdg-open");
    command
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    Ok(())
}

#[cfg(test)]
mod tests;
