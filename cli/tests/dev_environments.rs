//! `a4 dev` and the MCP `dev_*` tools against a mock Arete API and dev
//! environment RPC endpoint (one axum server on loopback), through the real
//! `a4` binary.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use axum::extract::{Path as AxumPath, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};

const KEY: &str = "a4_sk_devtest";
const TOKEN: &str = "env-token-1";
const ENV_A: &str = "denv_01hzx3k8m9q2r4s5t6v7w8x9ya";
const ENV_B: &str = "denv_01hzx3k8m9q2r4s5t6v7w8x9yb";
const WALLET: &str = "Gh9ZwEmdLJ8DscKNTkTqPbNwLNNBjuSzaG9Vp2KGtKJr";
const TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Default)]
struct Mock {
    origin: Mutex<String>,
    environments: Mutex<BTreeMap<String, Value>>,
    checkpoints: Mutex<Vec<Value>>,
    events: Mutex<Vec<Value>>,
    /// `METHOD path body`, in order.
    calls: Mutex<Vec<String>>,
    /// JSON-RPC calls the environment endpoint answered: (method, params).
    rpc: Mutex<Vec<(String, Value)>>,
    /// Unix seconds of the environment clock.
    clock: Mutex<i64>,
    next_id: Mutex<u8>,
    stack_artifacts: Mutex<Value>,
    stack_listing: Mutex<Vec<Value>>,
    stack_plan: Mutex<Value>,
    stack_build: Mutex<Value>,
    compile_failed: Mutex<bool>,
    /// The environment lease: `{leaseGeneration, projectId, holder, live}`.
    lease: Mutex<Option<Value>>,
    /// The lease generation header each environment stack build carried.
    stack_build_leases: Mutex<Vec<Option<String>>>,
    /// Registry stack loads answer that the environment was taken over.
    stack_load_lost: Mutex<bool>,
    /// Environment stack builds never finish.
    stack_build_stuck: Mutex<bool>,
}

type Shared = State<Arc<Mock>>;

fn authorized(headers: &HeaderMap, expected: &str) -> bool {
    headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        == Some(&format!("Bearer {expected}"))
}

fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({ "error": "bad key", "code": "unauthorized" })),
    )
        .into_response()
}

fn not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(json!({ "error": "Dev environment not found", "code": "devenv-not-found" })),
    )
        .into_response()
}

fn view(id: &str, name: &str, state: &str) -> Value {
    json!({
        "id": id, "name": name, "tier": "free", "state": state,
        "surfpoolVersion": "x", "storageBytes": 0, "journalOffset": null,
        "lastActiveAt": "2026-10-17T00:00:00Z", "sleptAt": null, "preSleepSlot": null,
        "restoredFrom": null, "retentionWarnedAt": null, "createdAt": "2026-10-17T00:00:00Z",
        "deletedAt": null,
    })
}

impl Mock {
    fn note(&self, line: String) {
        self.calls.lock().unwrap().push(line);
    }

    fn event(&self, env: &str, kind: &str, detail: Value) {
        let mut events = self.events.lock().unwrap();
        let id = events.len() as i64 + 1;
        events.push(json!({ "id": id, "env": env, "kind": kind, "detail": detail, "createdAt": "2026-10-17T00:00:00Z" }));
    }

    fn set_state(&self, env: &str, state: &str) -> Option<Value> {
        let mut environments = self.environments.lock().unwrap();
        let entry = environments.get_mut(env)?;
        entry["state"] = json!(state);
        Some(entry.clone())
    }
}

async fn list(State(mock): Shared, headers: HeaderMap) -> Response {
    if !authorized(&headers, KEY) {
        return unauthorized();
    }
    mock.note("GET /environments".into());
    let environments: Vec<Value> = mock
        .environments
        .lock()
        .unwrap()
        .values()
        .cloned()
        .collect();
    Json(json!({ "environments": environments })).into_response()
}

async fn create(State(mock): Shared, headers: HeaderMap, Json(body): Json<Value>) -> Response {
    if !authorized(&headers, KEY) {
        return unauthorized();
    }
    mock.note(format!("POST /environments {body}"));
    let mut next = mock.next_id.lock().unwrap();
    let id = if *next == 0 { ENV_A } else { ENV_B };
    *next += 1;
    let created = view(id, body["name"].as_str().unwrap_or("x"), "running");
    mock.environments
        .lock()
        .unwrap()
        .insert(id.to_string(), created.clone());
    mock.event(id, "created", json!({}));
    (StatusCode::CREATED, Json(created)).into_response()
}

async fn get_one(
    State(mock): Shared,
    headers: HeaderMap,
    AxumPath(env): AxumPath<String>,
) -> Response {
    if !authorized(&headers, KEY) {
        return unauthorized();
    }
    match mock.environments.lock().unwrap().get(&env) {
        Some(found) => Json(found.clone()).into_response(),
        None => not_found(),
    }
}

async fn delete_one(
    State(mock): Shared,
    headers: HeaderMap,
    AxumPath(env): AxumPath<String>,
) -> Response {
    if !authorized(&headers, KEY) {
        return unauthorized();
    }
    mock.note(format!("DELETE {env}"));
    match mock.environments.lock().unwrap().remove(&env) {
        Some(_) => StatusCode::NO_CONTENT.into_response(),
        None => not_found(),
    }
}

async fn action(
    State(mock): Shared,
    headers: HeaderMap,
    AxumPath((env, action)): AxumPath<(String, String)>,
    body: Option<Json<Value>>,
) -> Response {
    if !authorized(&headers, KEY) {
        return unauthorized();
    }
    let body = body.map(|Json(body)| body).unwrap_or(Value::Null);
    mock.note(format!("POST {env}/{action} {body}"));
    if !mock.environments.lock().unwrap().contains_key(&env) {
        return not_found();
    }
    match action.as_str() {
        "wake" => {
            mock.event(&env, "woken", json!({}));
            Json(mock.set_state(&env, "running").unwrap()).into_response()
        }
        "sleep" => {
            mock.event(&env, "slept", json!({ "reason": "requested" }));
            Json(mock.set_state(&env, "sleeping").unwrap()).into_response()
        }
        "token" => {
            let origin = mock.origin.lock().unwrap().clone();
            Json(json!({
                "token": TOKEN, "expiresAt": 4_000_000_000i64, "envId": env,
                "scope": format!("devenv:{env}"),
                "rpcUrl": format!("{origin}/rpc/v1/devenv/{env}"),
            }))
            .into_response()
        }
        "checkpoints" => {
            let checkpoint = json!({
                "id": format!("ckpt_{:064}", mock.checkpoints.lock().unwrap().len()),
                "environmentId": env, "name": body["name"], "journalOffset": 3, "bytes": 1234,
                "createdAt": "2026-10-17T00:00:00Z",
            });
            mock.checkpoints.lock().unwrap().push(checkpoint.clone());
            (StatusCode::CREATED, Json(checkpoint)).into_response()
        }
        "branch" => {
            let created = view(ENV_B, body["name"].as_str().unwrap_or("b"), "running");
            mock.environments
                .lock()
                .unwrap()
                .insert(ENV_B.to_string(), created.clone());
            (StatusCode::CREATED, Json(created)).into_response()
        }
        "restore" => Json(mock.set_state(&env, "running").unwrap()).into_response(),
        "rebase" => Json(json!({ "reset": 7, "kept": 2, "slot": 500 })).into_response(),
        "reset" => Json(json!({ "slot": 501 })).into_response(),
        _ => not_found(),
    }
}

async fn checkpoints(
    State(mock): Shared,
    headers: HeaderMap,
    AxumPath(_env): AxumPath<String>,
) -> Response {
    if !authorized(&headers, KEY) {
        return unauthorized();
    }
    Json(json!({ "checkpoints": mock.checkpoints.lock().unwrap().clone() })).into_response()
}

async fn events(
    State(mock): Shared,
    headers: HeaderMap,
    AxumPath(env): AxumPath<String>,
    Query(query): Query<BTreeMap<String, String>>,
) -> Response {
    if !authorized(&headers, KEY) {
        return unauthorized();
    }
    let after: i64 = query
        .get("after")
        .and_then(|after| after.parse().ok())
        .unwrap_or(0);
    let events: Vec<Value> = mock
        .events
        .lock()
        .unwrap()
        .iter()
        .filter(|event| event["env"] == env.as_str() && event["id"].as_i64().unwrap() > after)
        .cloned()
        .collect();
    let next = events
        .last()
        .map(|event| event["id"].clone())
        .unwrap_or(Value::Null);
    Json(json!({ "events": events, "next": next })).into_response()
}

async fn quotas(headers: HeaderMap) -> Response {
    if !authorized(&headers, KEY) {
        return unauthorized();
    }
    Json(json!({
        "tier": "free", "disabled": false,
        "limits": { "maxEnvironments": 1, "idleRetentionDays": 7, "maxCheckpointsPerEnvironment": 3,
                    "maxStorageBytes": 1000000, "maxActiveSecondsPerMonth": 100, "rpcRequestsPerMinute": 1200,
                    "sendsPerMinute": 60, "idleSleepSeconds": 900 },
        "usage": { "environments": 1, "storageBytes": 0, "activeSecondsThisMonth": 0 },
    }))
    .into_response()
}

/// The environment's JSON-RPC endpoint: reads, the clock and typed ops.
async fn rpc(
    State(mock): Shared,
    headers: HeaderMap,
    AxumPath(env): AxumPath<String>,
    Json(body): Json<Value>,
) -> Response {
    if !authorized(&headers, TOKEN) {
        return unauthorized();
    }
    let method = body["method"].as_str().unwrap_or_default().to_string();
    let params = body["params"].clone();
    mock.rpc
        .lock()
        .unwrap()
        .push((method.clone(), params.clone()));
    if !mock.environments.lock().unwrap().contains_key(&env) {
        return (StatusCode::NOT_FOUND, Json(json!({}))).into_response();
    }
    let result = match method.as_str() {
        "getSlot" => json!(1000),
        "getBalance" => json!({ "context": { "slot": 1000 }, "value": 5_000_000_000u64 }),
        "getAccountInfo" => {
            let clock = *mock.clock.lock().unwrap();
            json!({ "context": { "slot": 1000 }, "value": { "data": { "parsed": { "info": { "unixTimestamp": clock } } } } })
        }
        "devenv_warp" => {
            if let Some(ms) = params["timestampMs"].as_i64() {
                *mock.clock.lock().unwrap() = ms / 1000;
            }
            json!({ "warped": params })
        }
        method if method.starts_with("devenv_") => json!({ "ok": true, "params": params }),
        _ => {
            return Json(json!({ "jsonrpc": "2.0", "id": body["id"], "error": { "code": -32601, "message": "Method not allowed" } }))
                .into_response()
        }
    };
    Json(json!({ "jsonrpc": "2.0", "id": body["id"], "result": result })).into_response()
}

/// Serve the mock on a loopback port from a background runtime.
const STACK_PLAN: &str = "8d50e26b-e8b1-4d8f-90bf-b1cb0d025d1a";
const STACK_URL: &str = "wss://s81-01hzx3k8m9q2r4s5t6v7w8x9ya.devstack.arete.run";

fn stack_spec() -> Value {
    json!({ "id": 81, "user_id": 1, "name": "transfers", "entity_name": "Thing", "crate_name": "", "module_path": "", "url_slug": "transfers", "created_at": "now", "updated_at": "now" })
}

async fn stack_compile(
    State(mock): Shared,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if !authorized(&headers, KEY) {
        return unauthorized();
    }
    mock.note(format!("POST /stack-compilations/compile {body}"));
    if *mock.compile_failed.lock().unwrap() {
        return (StatusCode::UNPROCESSABLE_ENTITY, Json(json!({ "valid": false, "diagnostics": [{ "file": "transfers.stack.yaml", "range": { "start": { "line": 3, "column": 2 } }, "message": "unknown field" }] }))).into_response();
    }
    Json(json!({ "schema": "arete.stack-compilation-response/v1", "valid": true, "artifacts": mock.stack_artifacts.lock().unwrap().clone() })).into_response()
}

async fn stack_plan(State(mock): Shared, headers: HeaderMap, Json(body): Json<Value>) -> Response {
    if !authorized(&headers, KEY) {
        return unauthorized();
    }
    mock.note(format!("POST /deployments/plans {body}"));
    *mock.stack_plan.lock().unwrap() = body.clone();
    let targets: Vec<Value> = body["liveSpecs"].as_array().unwrap().iter().map(|live| json!({ "alias": live["alias"], "liveSpecHash": live["artifact"]["artifactHash"] })).collect();
    Json(json!({ "schema": "arete.stack-deployment-plan/v2", "persisted": true, "deploymentPlanId": STACK_PLAN,
        "stackManifestHash": body["stackManifest"]["artifactHash"], "branch": body["branch"], "targets": targets,
        "selectionDigest": format!("sha256:{}", "a".repeat(64)), "releases": [], "createdAt": "2026-10-17T00:00:00Z", "expiresAt": "2026-10-17T01:00:00Z", "idempotent": false, "programSdks": [] })).into_response()
}

async fn environment_stacks(State(mock): Shared, headers: HeaderMap) -> Response {
    if !authorized(&headers, KEY) {
        return unauthorized();
    }
    Json(json!({ "envId": ENV_A, "stacks": mock.stack_listing.lock().unwrap().clone() }))
        .into_response()
}

async fn deploy_environment_stack(
    State(mock): Shared,
    headers: HeaderMap,
    AxumPath(env): AxumPath<String>,
    Json(body): Json<Value>,
) -> Response {
    if !authorized(&headers, KEY) {
        return unauthorized();
    }
    assert_eq!(env, ENV_A);
    mock.note(format!("POST {env}/stacks {body}"));
    mock.stack_build_leases.lock().unwrap().push(
        headers
            .get("arete-dev-lease-generation")
            .and_then(|value| value.to_str().ok())
            .map(str::to_string),
    );
    if body.get("stackManifestHash").is_some() {
        if *mock.stack_load_lost.lock().unwrap() {
            return (
                StatusCode::CONFLICT,
                Json(json!({ "error": format!("Dev environment {env} was taken over by savings-app (a4 dev up); nothing was written"), "code": "devenv-lease-lost" })),
            )
                .into_response();
        }
        return Json(json!({ "status": "loaded", "deploymentId": 181 })).into_response();
    }
    *mock.stack_build.lock().unwrap() = body.clone();
    *mock.stack_listing.lock().unwrap() = vec![
        json!({ "deploymentId": 181, "specId": 81, "specName": "transfers", "assigned": true, "websocketUrl": STACK_URL, "status": "active", "protocolDeploymentId": null }),
    ];
    Json(json!({ "build_id": 281, "message": "queued", "deploymentPlanId": body["deploymentPlanId"], "selectionDigest": body["selectionDigest"], "envId": env, "deploymentId": 181, "websocketUrl": STACK_URL })).into_response()
}

/// The environment lease routes, as the API answers them.
async fn lease_take(
    State(mock): Shared,
    headers: HeaderMap,
    AxumPath(env): AxumPath<String>,
    Json(body): Json<Value>,
) -> Response {
    if !authorized(&headers, KEY) {
        return unauthorized();
    }
    mock.note(format!("POST {env}/lease {body}"));
    let mut lease = mock.lease.lock().unwrap();
    let current = lease.clone().unwrap_or(Value::Null);
    if let Some(generation) = body["leaseGeneration"].as_i64() {
        if current["leaseGeneration"] != generation {
            return (
                StatusCode::CONFLICT,
                Json(json!({ "error": format!("Dev environment {env} was taken over by {}; nothing was written", current["holder"].as_str().unwrap_or("?")), "code": "devenv-lease-lost", "holder": current["holder"] })),
            )
                .into_response();
        }
        return Json(json!({ "lease": current, "renewed": true })).into_response();
    }
    if current["live"] == true && current["projectId"] != body["projectId"] && body["force"] != true
    {
        return (
            StatusCode::CONFLICT,
            Json(json!({ "error": format!("Dev environment {env} is in use by {} (lease until 2026-10-29T10:00:00Z). Run `a4 dev up --take-over` to move it to this project, or `a4 dev branch` to give this project its own copy", current["holder"].as_str().unwrap_or("?")), "code": "devenv-environment-leased", "holder": current["holder"] })),
        )
            .into_response();
    }
    let generation = current["leaseGeneration"].as_i64().unwrap_or(0) + 1;
    let replaced = (current["projectId"] != body["projectId"] && !current.is_null())
        .then(|| json!({ "holder": current["holder"] }));
    let taken = json!({ "leaseGeneration": generation, "projectId": body["projectId"], "holder": body["holder"], "live": true, "expiresAt": "2026-10-29T10:00:00Z", "ttlSeconds": 60 });
    *lease = Some(taken.clone());
    Json(json!({ "lease": taken, "renewed": false, "replaced": replaced })).into_response()
}

async fn lease_get(State(mock): Shared, headers: HeaderMap) -> Response {
    if !authorized(&headers, KEY) {
        return unauthorized();
    }
    Json(json!({ "lease": mock.lease.lock().unwrap().clone() })).into_response()
}

async fn lease_release(
    State(mock): Shared,
    headers: HeaderMap,
    AxumPath(env): AxumPath<String>,
) -> Response {
    if !authorized(&headers, KEY) {
        return unauthorized();
    }
    let generation = headers
        .get("arete-dev-lease-generation")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<i64>().ok());
    mock.note(format!("DELETE {env}/lease {generation:?}"));
    let mut lease = mock.lease.lock().unwrap();
    let released = match lease.as_mut() {
        Some(current) if current["leaseGeneration"].as_i64() == generation => {
            current["live"] = json!(false);
            true
        }
        _ => false,
    };
    Json(json!({ "released": released })).into_response()
}

async fn stack_build(State(mock): Shared, headers: HeaderMap) -> Response {
    if !authorized(&headers, KEY) {
        return unauthorized();
    }
    mock.note("GET /builds/281".into());
    if *mock.stack_build_stuck.lock().unwrap() {
        return Json(json!({ "build": { "id": 281, "spec_id": 81, "status": "building", "created_at": "now", "phase": "building", "progress": 50 }, "events": [], "related_deployment_id": 181 })).into_response();
    }
    Json(json!({ "build": { "id": 281, "spec_id": 81, "status": "completed", "created_at": "now", "phase": "completed", "progress": 100 }, "events": [], "related_deployment_id": 181 })).into_response()
}

async fn stack_deployment(headers: HeaderMap) -> Response {
    if !authorized(&headers, KEY) {
        return unauthorized();
    }
    Json(json!({ "id": 181, "spec_id": 81, "spec_name": "transfers", "atom_name": "s81", "branch": ENV_A,
        "current_build_id": 281, "websocket_url": STACK_URL, "http_url": "https://example.test", "status": "active",
        "live_status": { "phase": "running", "source": "environment" } })).into_response()
}

async fn stack_composition(
    State(mock): Shared,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if !authorized(&headers, KEY) {
        return unauthorized();
    }
    mock.note(format!("POST /stack-compositions {body}"));
    let plan = mock.stack_plan.lock().unwrap();
    let alias = plan["liveSpecs"][0]["alias"].clone();
    Json(json!({ "compositionId": 1, "stackManifestHash": body["stackManifestHash"], "deploymentPlanId": body["deploymentPlanId"], "selectionDigest": body["selectionDigest"], "branch": body["branch"],
        "liveSpecs": [{ "alias": alias, "liveSpecHash": plan["liveSpecs"][0]["artifact"]["artifactHash"], "deploymentId": 181, "websocketEndpoint": STACK_URL, "queryEndpoint": "https://example.test", "websocketAuthPolicy": "signed_session", "queryAuthPolicy": "signed_session", "observedGeneration": 1 }] })).into_response()
}

async fn stack_session(
    State(mock): Shared,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if !authorized(&headers, KEY) {
        return unauthorized();
    }
    mock.note(format!("POST /ws/sessions {body}"));
    Json(json!({ "token": "stack-token", "expiresAt": 4_000_000_000i64 })).into_response()
}

fn serve() -> (Arc<Mock>, String) {
    let mock = Arc::new(Mock::default());
    *mock.clock.lock().unwrap() = 1_800_000_000;
    let app = Router::new()
        .route("/api/dev/environments", get(list).post(create))
        .route(
            "/api/dev/environments/:env",
            get(get_one).delete(delete_one),
        )
        .route(
            "/api/dev/environments/:env/checkpoints",
            get(checkpoints).post(
                |state: Shared,
                 headers: HeaderMap,
                 AxumPath(env): AxumPath<String>,
                 body: Option<Json<Value>>| async move {
                    action(
                        state,
                        headers,
                        AxumPath((env, "checkpoints".to_string())),
                        body,
                    )
                    .await
                },
            ),
        )
        .route("/api/stack-compilations/compile", post(stack_compile))
        .route("/api/deployments/plans", post(stack_plan))
        .route(
            "/api/dev/environments/:env/stacks",
            get(environment_stacks).post(deploy_environment_stack),
        )
        .route(
            "/api/specs",
            get(|headers: HeaderMap| async move {
                if !authorized(&headers, KEY) {
                    return unauthorized();
                }
                Json(json!([stack_spec()])).into_response()
            }),
        )
        .route("/api/builds/281", get(stack_build))
        .route("/api/deployments/181", get(stack_deployment))
        .route("/api/deployments/compositions", post(stack_composition))
        .route("/ws/sessions", post(stack_session))
        .route("/api/dev/environments/:env/events", get(events))
        .route(
            "/api/dev/environments/:env/lease",
            post(lease_take).get(lease_get).delete(lease_release),
        )
        .route("/api/dev/environments/:env/:action", post(action))
        .route("/api/dev/quotas", get(quotas))
        .route("/rpc/v1/devenv/:env", post(rpc))
        .with_state(mock.clone());
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async move {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            tx.send(listener.local_addr().unwrap()).unwrap();
            axum::serve(listener, app).await.unwrap();
        });
    });
    let addr = rx.recv().unwrap();
    let origin = format!("http://{addr}");
    *mock.origin.lock().unwrap() = origin.clone();
    (mock, origin)
}

fn a4(origin: &str, dir: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_a4"));
    command
        .current_dir(dir)
        .env("ARETE_API_URL", origin)
        .env("ARETE_API_KEY", KEY)
        .env("ARETE_CREDENTIALS_PATH", dir.join("no-credentials.toml"))
        .env_remove("ARETE_PROFILE")
        .env_remove("ARETE_DEV_ENV")
        .env_remove("ARETE_DEV_GATEWAY_URL")
        .env("CI", "1")
        .env("DO_NOT_TRACK", "1")
        .env("A4_NO_UPDATE_CHECK", "1");
    command
}

fn run_json(origin: &str, dir: &Path, args: &[&str]) -> Value {
    let output = a4(origin, dir)
        .arg("--json")
        .args(args)
        .output()
        .expect("run a4");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "a4 {args:?} failed: {}\n{stdout}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_str(stdout.lines().last().unwrap_or_default())
        .unwrap_or_else(|error| panic!("a4 {args:?} printed no JSON ({error}): {stdout}"))
}

fn run_fails(origin: &str, dir: &Path, args: &[&str]) -> String {
    let output = a4(origin, dir).args(args).output().expect("run a4");
    assert!(!output.status.success(), "a4 {args:?} succeeded");
    String::from_utf8_lossy(&output.stderr).to_string()
}

fn seed(mock: &Mock, state: &str) {
    mock.environments
        .lock()
        .unwrap()
        .insert(ENV_A.to_string(), view(ENV_A, "app", state));
}

#[test]
fn environment_commands_drive_the_api_and_the_environment() {
    let (mock, origin) = serve();
    seed(&mock, "sleeping");
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();

    let listed = run_json(&origin, dir, &["dev", "list"]);
    assert_eq!(listed["environments"][0]["id"], ENV_A);
    assert!(listed["environments"][0].get("surfpoolVersion").is_none());
    assert_eq!(listed["quotas"]["tier"], "free");

    // Point a project at it by name.
    let used = run_json(&origin, dir, &["dev", "use", "app"]);
    assert_eq!(used["id"], ENV_A);
    assert!(dir.join(".arete/dev/environment.json").is_file());

    let woken = run_json(&origin, dir, &["dev", "wake"]);
    assert_eq!(woken["state"], "running");

    let status = run_json(&origin, dir, &["dev", "status"]);
    assert_eq!(status["environment"]["state"], "running");
    assert_eq!(status["chain"]["slot"], 1000);
    assert_eq!(status["upRunning"], false);

    // Funding a dev wallet name creates the wallet and sets its balance.
    let funded = run_json(&origin, dir, &["dev", "fund", "alice", "2.5"]);
    let alice = funded["account"].as_str().unwrap().to_string();
    assert_eq!(funded["lamports"], 2_500_000_000u64);
    assert!(dir.join(".arete/dev/wallets/alice.json").is_file());
    let token_funded = run_json(
        &origin,
        dir,
        &["dev", "fund", WALLET, "--mint", WALLET, "--amount", "1000"],
    );
    assert_eq!(token_funded["amount"], "1000");

    let warped = run_json(&origin, dir, &["dev", "warp", "+2h"]);
    assert_eq!(warped["before"]["unixTimestamp"], 1_800_000_000i64);
    assert_eq!(warped["after"]["unixTimestamp"], 1_800_007_200i64);
    assert!(run_fails(&origin, dir, &["dev", "warp", "1"]).contains("only moves forward"));

    run_json(&origin, dir, &["dev", "pin", WALLET, "--include-owned"]);
    run_json(&origin, dir, &["dev", "pin", WALLET, "--unpin"]);
    run_json(&origin, dir, &["dev", "follow", WALLET]);
    run_json(&origin, dir, &["dev", "pause"]);
    run_json(&origin, dir, &["dev", "resume"]);
    run_json(&origin, dir, &["dev", "reset", "--account", WALLET]);
    let rebased = run_json(&origin, dir, &["dev", "rebase", "--keep", WALLET]);
    assert_eq!(rebased["reset"], 7);

    let ops: Vec<(String, Value)> = mock.rpc.lock().unwrap().clone();
    let op = |name: &str| -> Vec<Value> {
        ops.iter()
            .filter(|(method, _)| method == name)
            .map(|(_, params)| params.clone())
            .collect()
    };
    assert_eq!(
        op("devenv_fund"),
        vec![json!({ "pubkey": alice, "lamports": 2_500_000_000u64 })]
    );
    assert_eq!(
        op("devenv_setTokenBalance"),
        vec![json!({ "owner": WALLET, "mint": WALLET, "amount": "1000" })]
    );
    assert_eq!(
        op("devenv_warp"),
        vec![json!({ "timestampMs": 1_800_007_200_000i64 })]
    );
    assert_eq!(
        op("devenv_pin"),
        vec![
            json!({ "pubkey": WALLET, "includeOwned": true }),
            json!({ "pubkey": WALLET, "unpin": true })
        ]
    );
    assert_eq!(op("devenv_follow"), vec![json!({ "pubkey": WALLET })]);
    assert_eq!(op("devenv_pause"), vec![json!({})]);
    assert_eq!(op("devenv_resetAccount"), vec![json!({ "pubkey": WALLET })]);
    let calls = mock.calls.lock().unwrap().clone();
    assert!(
        calls
            .iter()
            .any(|call| call == &format!("POST {ENV_A}/rebase {{\"pinned\":[\"{WALLET}\"]}}")),
        "{calls:?}"
    );

    // Checkpoints, branch and restore.
    let checkpoint = run_json(&origin, dir, &["dev", "checkpoint", "funded"]);
    assert_eq!(checkpoint["name"], "funded");
    let listed = run_json(&origin, dir, &["dev", "checkpoint", "--list"]);
    assert_eq!(listed["checkpoints"].as_array().unwrap().len(), 1);
    let branched = run_json(&origin, dir, &["dev", "branch", "funded", "app-b"]);
    assert_eq!(branched["id"], ENV_B);
    let restored = run_json(&origin, dir, &["dev", "restore", "funded"]);
    assert_eq!(restored["state"], "running");

    // The other environment by name, without the project.
    let other = run_json(&origin, dir, &["dev", "status", "--env", "app-b"]);
    assert_eq!(other["environment"]["id"], ENV_B);

    let token = run_json(&origin, dir, &["dev", "token"]);
    assert_eq!(token["token"], TOKEN);
    assert_eq!(
        token["wsUrl"],
        format!(
            "{}/rpc/v1/devenv/{ENV_A}",
            origin.replace("http://", "ws://")
        )
    );

    let events = run_json(&origin, dir, &["dev", "events"]);
    let kinds: Vec<&str> = events["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|event| event["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["woken"]);

    // Destructive commands need --yes without a terminal.
    assert!(run_fails(&origin, dir, &["dev", "reset"]).contains("--yes"));
    assert!(run_fails(&origin, dir, &["dev", "delete", "--env", "app-b"]).contains("--yes"));
    let deleted = run_json(&origin, dir, &["dev", "delete", "--env", "app-b", "--yes"]);
    assert_eq!(deleted["deleted"], ENV_B);
    let reset = run_json(&origin, dir, &["--yes", "dev", "reset"]);
    assert_eq!(reset["slot"], 501);

    let slept = run_json(&origin, dir, &["dev", "sleep"]);
    assert_eq!(slept["state"], "sleeping");
    let deleted = run_json(&origin, dir, &["--yes", "dev", "delete"]);
    assert_eq!(deleted["deleted"], ENV_A);
    assert!(!dir.join(".arete/dev/environment.json").exists());
    assert!(run_fails(&origin, dir, &["dev", "status"]).contains("a4 dev up"));
}

#[test]
fn api_errors_name_the_code_and_what_to_do() {
    let (_mock, origin) = serve();
    let dir = tempfile::tempdir().unwrap();
    let error = run_fails(&origin, dir.path(), &["dev", "status", "--env", "nope"]);
    assert!(error.contains("no dev environment named 'nope'"), "{error}");
    let output = a4(&origin, dir.path())
        .env("ARETE_API_KEY", "a4_sk_wrong")
        .args(["dev", "list"])
        .output()
        .unwrap();
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(error.contains("HTTP 401"), "{error}");
}

/// Lines from a child's stdout, readable with a timeout.
fn lines_of(child: &mut Child) -> mpsc::Receiver<String> {
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    rx
}

fn wait_for(lines: &mpsc::Receiver<String>, event: &str) -> Value {
    let deadline = Instant::now() + TIMEOUT;
    let mut seen = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let line = lines
            .recv_timeout(left)
            .unwrap_or_else(|_| panic!("no {event} event; received: {seen:?}"));
        if let Ok(value) = serde_json::from_str::<Value>(&line) {
            if value["event"] == event {
                return value;
            }
        }
        seen.push(line);
    }
}

#[test]
fn up_creates_the_environment_and_serves_it_on_loopback() {
    let (mock, origin) = serve();
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("Tip Jar");
    std::fs::create_dir_all(&project).unwrap();
    let mut child = a4(&origin, &project)
        .args(["--json", "dev", "up", "--port-base", "0"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let lines = lines_of(&mut child);
    let environment = wait_for(&lines, "environment");
    assert_eq!(environment["outcome"], "created");
    assert_eq!(environment["environment"]["name"], "tip-jar");
    let ready = wait_for(&lines, "ready");
    let session = &ready["session"];
    assert_eq!(session["environment"]["id"], ENV_A);
    assert_eq!(
        ready["environmentRpcUrl"],
        format!("{origin}/rpc/v1/devenv/{ENV_A}")
    );
    let rpc_url = session["rpcUrl"].as_str().unwrap().to_string();
    let ui_url = session["uiUrl"].as_str().unwrap().to_string();
    assert!(rpc_url.starts_with("http://127.0.0.1:"), "{rpc_url}");

    // The local endpoint adds the token; the caller sends none.
    let http = reqwest::blocking::Client::new();
    let answer: Value = http
        .post(&rpc_url)
        .json(&json!({ "jsonrpc": "2.0", "id": 9, "method": "getBalance", "params": [WALLET] }))
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(answer["id"], 9);
    assert_eq!(answer["result"]["value"], 5_000_000_000u64);
    // A page from another site may not use it; a local app may.
    let refused = http
        .post(&rpc_url)
        .header("origin", "https://evil.example")
        .json(&json!({ "jsonrpc": "2.0", "id": 1, "method": "getSlot" }))
        .send()
        .unwrap();
    assert_eq!(refused.status().as_u16(), 403);
    let local = http
        .post(&rpc_url)
        .header("origin", "http://localhost:3000")
        .json(&json!({ "jsonrpc": "2.0", "id": 1, "method": "getSlot" }))
        .send()
        .unwrap();
    assert_eq!(local.status().as_u16(), 200);
    assert_eq!(
        local.headers()["access-control-allow-origin"],
        "http://localhost:3000"
    );

    // The session files and the dev UI.
    let saved: Value =
        serde_json::from_slice(&std::fs::read(project.join(".arete/dev/session.json")).unwrap())
            .unwrap();
    assert_eq!(saved["rpcUrl"], rpc_url.as_str());
    assert!(!saved.to_string().contains(KEY) && !saved.to_string().contains(TOKEN));
    let protocol: Value = http
        .get(format!("{}dev/v1/protocol", ui_url))
        .header("accept", "application/json")
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(protocol["environment"]["id"], ENV_A);
    assert_eq!(protocol["urls"]["rpc"], rpc_url.as_str());
    assert!(!protocol.to_string().contains(KEY));

    // Lifecycle events reach the dev UI's feed.
    mock.event(ENV_A, "checkpointed", json!({ "name": "x" }));
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let page: Value = http
            .get(format!("{}dev/v1/events?wait=2000", ui_url))
            .header("accept", "application/json")
            .send()
            .unwrap()
            .json()
            .unwrap();
        let found =
            page["events"].as_array().unwrap().iter().any(|event| {
                event["type"] == "env.changed" && event["data"]["kind"] == "checkpointed"
            });
        if found {
            break;
        }
        assert!(Instant::now() < deadline, "no env.changed event: {page}");
    }

    // `a4 dev status` sees the running session; `a4 dev down` stops it.
    let status = run_json(&origin, &project, &["dev", "status"]);
    assert_eq!(status["upRunning"], true);
    assert!(status["up"]["pid"].is_number());
    let down = run_json(&origin, &project, &["dev", "down"]);
    assert_eq!(down["stopped"][0]["name"], "a4 dev up");
    let stopped = wait_for(&lines, "stopped");
    assert!(stopped["event"].is_string());
    let deadline = Instant::now() + TIMEOUT;
    while child.try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline, "a4 dev up did not exit");
        thread::sleep(Duration::from_millis(100));
    }
    assert!(!project.join(".arete/dev/session.json").exists());
    assert!(project.join(".arete/dev/environment.json").exists());

    // A second `a4 dev up` reuses (and wakes) the same environment.
    mock.set_state(ENV_A, "sleeping");
    let mut again = a4(&origin, &project)
        .args(["--json", "dev", "up", "--port-base", "0", "--no-ui"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let lines = lines_of(&mut again);
    let environment = wait_for(&lines, "environment");
    assert_eq!(environment["outcome"], "woken");
    assert_eq!(environment["environment"]["id"], ENV_A);
    wait_for(&lines, "ready");
    let _ = again.kill();
    let _ = again.wait();
    let creates = mock
        .calls
        .lock()
        .unwrap()
        .iter()
        .filter(|call| call.starts_with("POST /environments"))
        .count();
    assert_eq!(creates, 1);
}

/// One MCP session with `a4 mcp` in `dir`.
struct Mcp {
    child: Child,
    stdin: std::process::ChildStdin,
    lines: mpsc::Receiver<String>,
    next: u64,
}

impl Mcp {
    fn start(origin: &str, dir: &Path) -> Self {
        let mut child = a4(origin, dir)
            .arg("mcp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let lines = lines_of(&mut child);
        let mut mcp = Self {
            child,
            stdin,
            lines,
            next: 1,
        };
        mcp.request(
            "initialize",
            json!({ "protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": { "name": "t", "version": "0" } }),
        );
        writeln!(
            mcp.stdin,
            "{}",
            json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })
        )
        .unwrap();
        mcp
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next;
        self.next += 1;
        writeln!(
            self.stdin,
            "{}",
            json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
        )
        .unwrap();
        self.stdin.flush().unwrap();
        loop {
            let line = self.lines.recv_timeout(TIMEOUT).expect("an MCP answer");
            let value: Value = serde_json::from_str(&line).unwrap();
            if value["id"] == id {
                return value;
            }
        }
    }

    /// A tool's structured result, or its error message.
    fn call(&mut self, tool: &str, arguments: Value) -> Result<Value, String> {
        let answer = self.request(
            "tools/call",
            json!({ "name": tool, "arguments": arguments }),
        );
        if let Some(error) = answer.get("error") {
            return Err(error["message"].as_str().unwrap_or_default().to_string());
        }
        let result = &answer["result"];
        if result["isError"] == true {
            return Err(result["content"][0]["text"]
                .as_str()
                .unwrap_or_default()
                .to_string());
        }
        Ok(result.get("structuredContent").cloned().unwrap_or_else(|| {
            serde_json::from_str(result["content"][0]["text"].as_str().unwrap_or("null"))
                .unwrap_or(Value::Null)
        }))
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn mcp_dev_tools_act_on_the_environment() {
    let (mock, origin) = serve();
    seed(&mock, "running");
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path();
    // The project's environment, as `a4 dev up` records it.
    run_json(&origin, project, &["dev", "use", "app"]);
    let mut mcp = Mcp::start(&origin, project);

    let tools = mcp.request("tools/list", json!({}));
    let names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    for expected in [
        "dev_status",
        "dev_environments",
        "dev_wake",
        "dev_sleep",
        "dev_delete",
        "dev_checkpoint",
        "dev_branch",
        "dev_restore",
        "dev_rebase",
        "dev_reset",
        "dev_reset_account",
        "dev_pin",
        "dev_follow",
        "dev_fund",
        "dev_warp",
        "dev_scenario",
        "dev_preview",
        "dev_repro",
        "dev_environment_events",
        "dev_step",
        "dev_events",
    ] {
        assert!(
            names.contains(&expected),
            "{expected} missing from {names:?}"
        );
    }
    for gone in [
        "dev_logs",
        "dev_time_travel",
        "dev_replay_load",
        "dev_capture_fork",
    ] {
        assert!(!names.contains(&gone), "{gone}");
    }

    let environments = mcp.call("dev_environments", json!({})).unwrap();
    assert_eq!(environments["environments"][0]["id"], ENV_A);
    let status = mcp.call("dev_status", json!({})).unwrap();
    assert_eq!(status["environment"]["id"], ENV_A);
    assert_eq!(status["chain"]["slot"], 1000);

    let funded = mcp
        .call("dev_fund", json!({ "account": WALLET, "sol": "3" }))
        .unwrap();
    assert_eq!(funded["account"], WALLET);
    let warped = mcp.call("dev_warp", json!({ "to": "+1d" })).unwrap();
    assert_eq!(warped["target"]["timestampMs"], 1_800_086_400_000i64);
    mcp.call("dev_pin", json!({ "account": WALLET })).unwrap();
    mcp.call("dev_follow", json!({ "account": WALLET, "undo": true }))
        .unwrap();
    let scenario = mcp
        .call(
            "dev_scenario",
            json!({ "scenario": { "schema": "arete.scenario/v1", "name": "crash", "steps": [] }, "dry_run": true }),
        )
        .unwrap();
    assert_eq!(scenario["ok"], true);
    let checkpoint = mcp
        .call("dev_checkpoint", json!({ "checkpoint": "before" }))
        .unwrap();
    assert_eq!(checkpoint["name"], "before");
    let rebase = mcp.call("dev_rebase", json!({})).unwrap();
    assert_eq!(rebase["kept"], 2);

    // Destructive tools need the environment's name.
    let refused = mcp
        .call("dev_reset", json!({ "confirm": "wrong" }))
        .unwrap_err();
    assert!(refused.contains("`app`"), "{refused}");
    let reset = mcp.call("dev_reset", json!({ "confirm": "app" })).unwrap();
    assert_eq!(reset["slot"], 501);
    let deleted = mcp
        .call(
            "dev_delete",
            json!({ "confirm": "app", "environment": "app" }),
        )
        .unwrap();
    assert_eq!(deleted["deleted"], ENV_A);

    let ops: Vec<String> = mock
        .rpc
        .lock()
        .unwrap()
        .iter()
        .map(|(method, _)| method.clone())
        .collect();
    for op in [
        "devenv_fund",
        "devenv_warp",
        "devenv_pin",
        "devenv_follow",
        "devenv_scenario",
    ] {
        assert!(ops.iter().any(|seen| seen == op), "{op} not in {ops:?}");
    }
    let scenario_params = mock
        .rpc
        .lock()
        .unwrap()
        .iter()
        .find(|(method, _)| method == "devenv_scenario")
        .map(|(_, params)| params.clone())
        .unwrap();
    assert_eq!(scenario_params["dryRun"], true);
}

fn project_stack_artifacts() -> Value {
    let live = arete_artifacts::LiveSpecArtifactV2::new(arete_artifacts::LiveSpecV2::new(
        Vec::new(),
        Vec::new(),
        Vec::new(),
    ))
    .unwrap();
    let manifest = arete_artifacts::compose_stack_manifest_v2(
        "transfers",
        &[],
        vec![("live".into(), &live)],
        Vec::new(),
    )
    .unwrap();
    json!({ "programSpecs": [], "liveSpec": live, "stackManifest": manifest, "sourceIdls": {} })
}

#[test]
fn up_compiles_and_deploys_project_stacks_to_the_environment() {
    let (mock, origin) = serve();
    seed(&mock, "running");
    *mock.stack_artifacts.lock().unwrap() = project_stack_artifacts();
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("stacks")).unwrap();
    std::fs::write(
        dir.path().join("stacks/transfers.stack.yaml"),
        "schema: arete.stack-source/v1\nname: transfers\nprograms: {}\nentities: {}\n",
    )
    .unwrap();
    let mut child = a4(&origin, dir.path())
        .args([
            "--json",
            "dev",
            "up",
            "--env",
            "app",
            "--port-base",
            "0",
            "--no-ui",
            "--no-watch",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let lines = lines_of(&mut child);
    let ready = wait_for(&lines, "ready");
    assert_eq!(ready["session"]["stack"], "transfers");
    assert_eq!(ready["session"]["stackWsUrl"], STACK_URL);
    let plan = mock.stack_plan.lock().unwrap().clone();
    let build = mock.stack_build.lock().unwrap().clone();
    assert_eq!(plan["branch"], ENV_A);
    assert_eq!(build["branch"], ENV_A);
    assert_eq!(build["deploymentPlanId"], STACK_PLAN);
    assert!(build["sourceIdls"].is_object());
    let calls = mock.calls.lock().unwrap().clone();
    assert!(calls
        .iter()
        .any(|call| call.starts_with("POST /stack-compilations/compile")));
    assert!(calls
        .iter()
        .any(|call| call.starts_with(&format!("POST {ENV_A}/stacks"))));
    assert!(!calls.iter().any(|call| call.contains("/builds/artifacts")));
    // The session file holds no stack token, so `a4 dev up` mints none.
    assert!(!calls.iter().any(|call| call.contains("POST /ws/sessions")));
    // The lease came first, and the stack build carried its generation.
    let lease_at = calls
        .iter()
        .position(|call| call.starts_with(&format!("POST {ENV_A}/lease")));
    let build_at = calls
        .iter()
        .position(|call| call.starts_with(&format!("POST {ENV_A}/stacks")));
    assert!(lease_at.unwrap() < build_at.unwrap(), "{calls:?}");
    assert_eq!(
        *mock.stack_build_leases.lock().unwrap(),
        vec![Some("1".to_string())]
    );
    run_json(&origin, dir.path(), &["dev", "down"]);
    assert!(child.wait().unwrap().success());
    // Released on exit.
    assert_eq!(mock.lease.lock().unwrap().as_ref().unwrap()["live"], false);
}

/// A project with one registry stack in its arete.lock.
fn project_with_locked_stack() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("arete.lock"),
        r#"lock_version = 1
manifest_hash = "arete-manifest-v1:test"
resolver_contract = "registry-resolver-v1"

[[dependency]]
kind = "stack"
alias = "ledger"
source = "registry:ledger"
requirement = "*"
version = "1.0.0"
package_release_hash = "release"
stack_manifest_hash = "hash-ledger"
generator_contract = "sdk-generator-v1"
"#,
    )
    .unwrap();
    dir
}

const UP_ONCE: [&str; 8] = [
    "dev",
    "up",
    "--env",
    "app",
    "--no-ui",
    "--once",
    "--port-base",
    "0",
];

/// Registry stacks load under the lease `a4 dev up` holds.
#[test]
fn locked_stacks_load_with_the_lease_generation() {
    let (mock, origin) = serve();
    seed(&mock, "running");
    let dir = project_with_locked_stack();
    run_json(&origin, dir.path(), &UP_ONCE);
    assert_eq!(
        *mock.stack_build_leases.lock().unwrap(),
        vec![Some("1".to_string())]
    );
}

/// A takeover during a registry stack load ends the run: it isn't one more
/// stack that failed to load.
#[test]
fn a_takeover_during_a_locked_stack_load_stops_the_run() {
    let (mock, origin) = serve();
    seed(&mock, "running");
    *mock.stack_load_lost.lock().unwrap() = true;
    let dir = project_with_locked_stack();
    let error = run_fails(&origin, dir.path(), &UP_ONCE);
    assert!(error.contains("taken over by savings-app"), "{error}");
}

/// A takeover during a stack build ends the process, though the blocking
/// work that polls the build could go on for up to 30 minutes.
#[test]
fn a_takeover_during_a_stack_build_ends_the_process() {
    let (mock, origin) = serve();
    seed(&mock, "running");
    *mock.stack_artifacts.lock().unwrap() = project_stack_artifacts();
    *mock.stack_build_stuck.lock().unwrap() = true;
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("stacks")).unwrap();
    std::fs::write(
        dir.path().join("stacks/transfers.stack.yaml"),
        "schema: arete.stack-source/v1\nname: transfers\nprograms: {}\nentities: {}\n",
    )
    .unwrap();
    let mut child = a4(&origin, dir.path())
        .args(UP_ONCE)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let polling = Instant::now();
    while !mock
        .calls
        .lock()
        .unwrap()
        .iter()
        .any(|call| call == "GET /builds/281")
    {
        assert!(polling.elapsed() < TIMEOUT, "the build was never polled");
        thread::sleep(Duration::from_millis(50));
    }
    {
        let mut lease = mock.lease.lock().unwrap();
        let lease = lease.as_mut().unwrap();
        lease["leaseGeneration"] = json!(2);
        lease["holder"] = json!("savings-app");
    }
    let taken_over = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if taken_over.elapsed() > TIMEOUT {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("a4 dev up was still running after the takeover");
        }
        thread::sleep(Duration::from_millis(100));
    };
    assert!(!status.success());
    let mut error = String::new();
    std::io::Read::read_to_string(child.stderr.as_mut().unwrap(), &mut error).unwrap();
    assert!(error.contains("taken over by savings-app"), "{error}");
}

/// One project per environment: a second project is refused with the
/// holder and the ways out and deploys nothing; `--take-over` moves the
/// environment to it.
#[test]
fn a_leased_environment_refuses_another_project_until_it_takes_over() {
    let (mock, origin) = serve();
    seed(&mock, "running");
    *mock.lease.lock().unwrap() = Some(
        json!({ "leaseGeneration": 4, "projectId": "proj_somebodyelse00000000", "holder": "savings-app (a4 dev up)", "live": true, "expiresAt": "2026-10-29T10:00:00Z", "ttlSeconds": 60 }),
    );
    *mock.stack_artifacts.lock().unwrap() = project_stack_artifacts();
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("stacks")).unwrap();
    std::fs::write(
        dir.path().join("stacks/transfers.stack.yaml"),
        "schema: arete.stack-source/v1\nname: transfers\nprograms: {}\nentities: {}\n",
    )
    .unwrap();
    let up = [
        "dev",
        "up",
        "--env",
        "app",
        "--no-ui",
        "--once",
        "--port-base",
        "0",
    ];
    let error = run_fails(&origin, dir.path(), &up);
    assert!(error.contains("savings-app (a4 dev up)"), "{error}");
    assert!(error.contains("a4 dev up --take-over"), "{error}");
    assert!(error.contains("a4 dev branch"), "{error}");
    assert!(error.contains("devenv-environment-leased"), "{error}");
    let calls = mock.calls.lock().unwrap().clone();
    assert!(
        !calls
            .iter()
            .any(|call| call.contains("/stacks") || call.contains("compile")),
        "nothing deploys: {calls:?}"
    );
    assert!(mock.stack_build_leases.lock().unwrap().is_empty());

    let mut take_over = up.to_vec();
    take_over.push("--take-over");
    let output = a4(&origin, dir.path())
        .arg("--json")
        .args(&take_over)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{}\n{stdout}",
        String::from_utf8_lossy(&output.stderr)
    );
    let lease = stdout
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|event| event["event"] == "lease")
        .unwrap_or_else(|| panic!("no lease event: {stdout}"));
    assert_eq!(lease["leaseGeneration"], 5);
    assert_eq!(lease["replaced"], "savings-app (a4 dev up)");
    assert_eq!(
        *mock.stack_build_leases.lock().unwrap(),
        vec![Some("5".to_string())]
    );

    // Status names the holder (released after the --once run).
    let status = run_json(&origin, dir.path(), &["dev", "status"]);
    assert_eq!(status["lease"]["held"], false, "{status}");
    assert!(status["lease"]["holder"]
        .as_str()
        .unwrap()
        .ends_with("(a4 dev up)"));
}

#[test]
fn existing_protocol_stack_is_selected_by_deployment_id_not_listing_order() {
    let (mock, origin) = serve();
    seed(&mock, "running");
    let protocol = "00000000-0000-0000-0000-000000000081";
    *mock.stack_listing.lock().unwrap() = vec![
        json!({ "deploymentId": 181, "specName": "right", "assigned": true, "websocketUrl": STACK_URL, "protocolDeploymentId": protocol, "protocolSlug": "mine" }),
        json!({ "deploymentId": 999, "specName": "unrelated", "assigned": true, "websocketUrl": "wss://unrelated.devstack.arete.run", "protocolDeploymentId": "another" }),
    ];
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".arete/dev")).unwrap();
    std::fs::write(
        dir.path().join(".arete/dev/deploy.json"),
        json!({ "environment": ENV_A, "deploymentId": protocol, "slug": "mine" }).to_string(),
    )
    .unwrap();
    let mut child = a4(&origin, dir.path())
        .args([
            "--json",
            "dev",
            "up",
            "--env",
            "app",
            "--port-base",
            "0",
            "--no-ui",
            "--no-deploy",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let lines = lines_of(&mut child);
    let ready = wait_for(&lines, "ready");
    assert_eq!(ready["session"]["stack"], "right");
    assert_eq!(ready["session"]["stackWsUrl"], STACK_URL);
    let listing = run_json(&origin, dir.path(), &["stream", "--dev"]);
    assert_eq!(listing["stacks"][0]["deploymentId"], protocol);
    assert_eq!(listing["stacks"][0]["wsUrl"], STACK_URL);
    assert_eq!(listing["stacks"][1]["deploymentId"], "another");
    assert!(ready["session"].get("stackSession").is_none());
    assert!(!mock
        .calls
        .lock()
        .unwrap()
        .iter()
        .any(|call| call.contains("POST /ws/sessions")));
    run_json(&origin, dir.path(), &["dev", "down"]);
    assert!(child.wait().unwrap().success());
}

#[test]
fn project_stack_compile_errors_keep_source_locations() {
    let (mock, origin) = serve();
    seed(&mock, "running");
    *mock.compile_failed.lock().unwrap() = true;
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("stacks")).unwrap();
    std::fs::write(dir.path().join("stacks/transfers.stack.yaml"), "broken").unwrap();
    let error = run_fails(
        &origin,
        dir.path(),
        &[
            "dev",
            "up",
            "--env",
            "app",
            "--no-ui",
            "--no-watch",
            "--port-base",
            "0",
        ],
    );
    assert!(
        error.contains("stacks/transfers.stack.yaml:3:2: unknown field"),
        "{error}"
    );
    assert!(mock.stack_plan.lock().unwrap().is_null());
}

#[cfg(unix)]
#[test]
fn up_builds_rust_stack_artifacts_and_deploys_them_into_the_environment() {
    use std::os::unix::fs::PermissionsExt;
    let (mock, origin) = serve();
    seed(&mock, "running");
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let output = root.join("crate/.arete");
    std::fs::create_dir_all(&output).unwrap();
    let artifacts = project_stack_artifacts();
    for (field, file) in [
        ("stackManifest", "transfers.stack-manifest.json"),
        ("liveSpec", "transfers.live-spec.json"),
    ] {
        std::fs::write(output.join(file), artifacts[field].to_string()).unwrap();
    }
    std::fs::write(
        root.join("crate/Cargo.toml"),
        "[package]\nname = \"transfers\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::write(
        root.join("arete.toml"),
        "[authoring.stacks.transfers]\nmanifest = \"crate/.arete/transfers.stack-manifest.json\"\n",
    )
    .unwrap();
    let tools = root.join("tools");
    std::fs::create_dir_all(&tools).unwrap();
    let cargo = tools.join("cargo");
    std::fs::write(&cargo, "#!/bin/sh\nprintf '%s\\n' \"$@\" > cargo-args\n").unwrap();
    std::fs::set_permissions(&cargo, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut child = a4(&origin, root)
        .env(
            "PATH",
            format!(
                "{}:{}",
                tools.display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .args([
            "--json",
            "dev",
            "up",
            "--env",
            "app",
            "--no-ui",
            "--no-watch",
            "--port-base",
            "0",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let lines = lines_of(&mut child);
    let ready = wait_for(&lines, "ready");
    assert_eq!(ready["session"]["stackWsUrl"], STACK_URL);
    let args = std::fs::read_to_string(root.join("crate/cargo-args")).unwrap();
    assert!(args.contains("check\n--locked\n"), "{args}");
    assert_eq!(mock.stack_build.lock().unwrap()["branch"], ENV_A);
    assert!(!mock
        .calls
        .lock()
        .unwrap()
        .iter()
        .any(|call| call.contains("/stack-compilations/compile")));
    run_json(&origin, root, &["dev", "down"]);
    assert!(child.wait().unwrap().success());
}
