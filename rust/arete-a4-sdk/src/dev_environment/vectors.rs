//! The shared vectors in `test-vectors/dev-environment-switch/v1/`.

use std::cell::RefCell;
use std::path::PathBuf;

use serde_json::{json, Value};

use super::*;
use crate::auth::DEFAULT_HOSTED_TOKEN_ENDPOINT;
use crate::gateway::HostedSolanaGatewayBindings;
use crate::http::{AuthTokenTarget, HttpAuthClient};
use crate::read::ProgramReadDescriptor;

fn load(file: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../test-vectors/dev-environment-switch/v1")
        .join(file);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    let value: Value = serde_json::from_str(&text).expect("vector JSON");
    assert_eq!(value["version"], 1, "{file} version");
    value
}

fn cases(file: &Value) -> &Vec<Value> {
    file["cases"].as_array().expect("cases")
}

const DEV_INPUTS: [&str; 2] = [ARETE_DEV_URL_ENV, ARETE_DEV_ENV_ENV];

/// Run the switch on a vector input, recording which dev inputs it read.
fn run_switch(input: &Value) -> (Result<SwitchResult, AreteError>, bool) {
    let read = RefCell::new(false);
    let variables = input["variables"].as_object().cloned().unwrap_or_default();
    let read_var = |name: &str| {
        if DEV_INPUTS.contains(&name) {
            *read.borrow_mut() = true;
        }
        if input["browser"].as_bool() == Some(true) {
            panic!("a browser read {name}");
        }
        variables
            .get(name)
            .and_then(Value::as_str)
            .map(str::to_string)
    };
    let session_edge_url = || {
        *read.borrow_mut() = true;
        input["sessionFile"]["edgeUrl"].as_str().map(str::to_string)
    };
    let has_secret_key = || input["secretKey"].as_bool() == Some(true);
    let option = match &input["option"] {
        Value::Null => None,
        value => match EnvironmentOption::from_json(value) {
            Ok(option) => Some(option),
            Err(error) => return (Err(error), false),
        },
    };
    let result = resolve_switch(
        option.as_ref(),
        &SwitchInputs {
            browser: input["browser"].as_bool() == Some(true),
            read_var: &read_var,
            session_edge_url: &session_edge_url,
            has_secret_key: &has_secret_key,
        },
    );
    let dev_inputs_read = *read.borrow();
    (result, dev_inputs_read)
}

fn mode_json(result: &SwitchResult) -> Value {
    match result {
        SwitchResult::Production => json!({ "mode": "production", "devInputsRead": false }),
        SwitchResult::Edge { edge_url } => {
            json!({ "mode": "dev", "via": "edge", "edgeUrl": edge_url })
        }
        SwitchResult::Direct { environment } => {
            json!({ "mode": "dev", "via": "direct", "environment": environment })
        }
    }
}

fn mode_of(value: &Value) -> SwitchResult {
    match (value["mode"].as_str(), value["via"].as_str()) {
        (Some("production"), _) => SwitchResult::Production,
        (Some("dev"), Some("edge")) => SwitchResult::Edge {
            edge_url: value["edgeUrl"].as_str().unwrap().to_string(),
        },
        (Some("dev"), Some("direct")) => SwitchResult::Direct {
            environment: value["environment"].as_str().unwrap().to_string(),
        },
        other => panic!("unknown mode {other:?}"),
    }
}

#[test]
fn switch_vectors() {
    let file = load("switch.json");
    for case in cases(&file) {
        let name = case["name"].as_str().unwrap();
        let (result, dev_inputs_read) = run_switch(&case["input"]);
        let expect = &case["expect"];
        if let Some(code) = expect["error"]["code"].as_str() {
            let error = result.expect_err(name);
            assert_eq!(error.dev_code(), Some(code), "{name}: {error}");
            continue;
        }
        let result = result.unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(&mode_json(&result), expect, "{name}");
        if expect["devInputsRead"] == json!(false) {
            assert!(!dev_inputs_read, "{name}: read a dev input");
        }
    }
}

// ---------------------------------------------------------------------------
// targets.json
// ---------------------------------------------------------------------------

struct Installed {
    gateway: HostedSolanaGatewayBindings,
    program_id: String,
    read: ProgramReadDescriptor,
    stack_ws: String,
    stack_http: String,
    release: StackRelease,
    protocol: Value,
}

fn installed(file: &Value, name: &str) -> Installed {
    let descriptor = &file["installed"][name];
    Installed {
        gateway: serde_json::from_value(descriptor["gateway"].clone()).expect("gateway"),
        program_id: descriptor["programs"]["klend"]["programId"]
            .as_str()
            .unwrap()
            .to_string(),
        read: serde_json::from_value(descriptor["programReads"]["klend"].clone())
            .expect("program read"),
        stack_ws: descriptor["stack"]["endpoints"]["ws"]
            .as_str()
            .unwrap()
            .to_string(),
        stack_http: descriptor["stack"]["endpoints"]["http"]
            .as_str()
            .unwrap()
            .to_string(),
        release: StackRelease {
            stack_manifest_hash: descriptor["stack"]["release"]["stackManifestHash"]
                .as_str()
                .unwrap()
                .to_string(),
            live_alias: descriptor["stack"]["release"]["liveAlias"]
                .as_str()
                .unwrap()
                .to_string(),
        },
        protocol: descriptor["protocol"].clone(),
    }
}

fn header(policy: Option<EdgeRequestPolicy>) -> Value {
    policy
        .and_then(|policy| policy.identity)
        .map(|identity| serde_json::from_str(&identity).expect("identity JSON"))
        .unwrap_or(Value::Null)
}

fn secret_auth() -> AuthConfig {
    AuthConfig::default().with_secret_key("a4_sk_vector")
}

impl Installed {
    fn protocol_identity(&self) -> ProtocolIdentity {
        ProtocolIdentity {
            package: self.protocol["package"].as_str().unwrap().to_string(),
            protocol_release_hash: self.protocol["protocolReleaseHash"]
                .as_str()
                .unwrap()
                .to_string(),
            deployment_id: self.protocol["deployment"]["deploymentId"]
                .as_str()
                .map(str::to_string),
        }
    }

    fn program_read_identity(&self) -> ProgramReadIdentity {
        let ProgramReadDescriptor::HostedBinding { release, binding } = &self.read else {
            panic!("hosted binding");
        };
        ProgramReadIdentity {
            program_id: Some(self.program_id.clone()),
            program_read_binding_id: binding.program_read_binding_id.clone(),
            program_release_hash: release.program_release_hash.clone(),
        }
    }

    fn gateway_identity(&self, chain: bool) -> GatewayIdentity {
        let binding = if chain {
            &self.gateway.chain
        } else {
            &self.gateway.transactions
        };
        GatewayIdentity {
            solana_gateway_binding_id: binding.solana_gateway_binding_id.clone(),
        }
    }

    /// Everything direct mode resolves.
    fn resolve_request(&self) -> ResolveRequest {
        ResolveRequest {
            chain: Some(self.gateway_identity(true)),
            transactions: Some(self.gateway_identity(false)),
            program_reads: vec![self.program_read_identity()],
            stacks: vec![StackIdentity::from(&self.release)],
            protocols: vec![self.protocol_identity()],
        }
    }
}

/// What each surface does in `mode`, from the same pieces the transports
/// are built from.
fn observe(installed: &Installed, mode: &SwitchResult, answer: Option<&Value>) -> Value {
    let http = reqwest::Client::new();
    let dev = DevMode::of(mode, Some(&secret_auth()), &http).expect("dev mode");
    let resolver = match &dev {
        Some(DevMode::Direct(resolver)) => Some(resolver.clone()),
        _ => None,
    };
    let edge_url = match mode {
        SwitchResult::Edge { edge_url } => Some(edge_url.as_str()),
        _ => None,
    };

    let gateway = |chain: bool| {
        let binding = if chain {
            &installed.gateway.chain
        } else {
            &installed.gateway.transactions
        };
        let target = AuthTokenTarget::solana_gateway_binding(&binding.solana_gateway_binding_id);
        let scopes = crate::http::normalize_scopes(&binding.auth.scopes);
        if let Some(edge_url) = edge_url {
            let identity = installed.gateway_identity(chain);
            let request = if chain {
                ResolveRequest {
                    chain: Some(identity),
                    ..ResolveRequest::default()
                }
            } else {
                ResolveRequest {
                    transactions: Some(identity),
                    ..ResolveRequest::default()
                }
            };
            return json!({
                "endpoint": edge_url,
                "tokenRequest": null,
                "sessionEndpoint": null,
                "identityHeader": header(Some(EdgeRequestPolicy::new(edge_url, Some(&request)))),
            });
        }
        let (endpoint, auth) = match &resolver {
            Some(resolver) => {
                let surface = if chain {
                    Surface::Chain
                } else {
                    Surface::Transactions
                };
                let target = surface_target(answer.unwrap(), surface).expect("target");
                (
                    target_str(target, "endpoint").unwrap().to_string(),
                    resolver.token_auth(&binding.auth.session_endpoint),
                )
            }
            None => (
                binding.endpoint.clone(),
                AuthConfig::default()
                    .with_publishable_key("a4_pk_vector")
                    .with_token_endpoint(binding.auth.session_endpoint.clone()),
            ),
        };
        let client = HttpAuthClient::new(Some(auth), None, http.clone());
        json!({
            "endpoint": endpoint,
            "tokenRequest": client.token_endpoint_body(Some(&target), &scopes),
            "sessionEndpoint": binding.auth.session_endpoint,
            "identityHeader": null,
        })
    };

    let ProgramReadDescriptor::HostedBinding { release, binding } = &installed.read else {
        panic!("hosted binding");
    };
    let program_read = if let Some(edge_url) = edge_url {
        let request = ResolveRequest {
            program_reads: vec![installed.program_read_identity()],
            ..ResolveRequest::default()
        };
        json!({
            "endpoint": edge_url,
            "pathReleaseHash": release.program_release_hash,
            "tokenRequest": null,
            "sessionEndpoint": null,
            "identityHeader": header(Some(EdgeRequestPolicy::new(edge_url, Some(&request)))),
        })
    } else {
        let target = AuthTokenTarget::program_read_binding(
            &binding.program_read_binding_id,
            &release.program_release_hash,
        );
        let (endpoint, path_release, auth) = match &resolver {
            Some(resolver) => {
                let target = surface_target(answer.unwrap(), Surface::ProgramRead(0)).unwrap();
                (
                    target_str(target, "endpoint").unwrap().to_string(),
                    target_str(target, "programReleaseHash")
                        .unwrap()
                        .to_string(),
                    resolver.token_auth(&binding.auth.session_endpoint),
                )
            }
            None => (
                binding.endpoint.clone(),
                release.program_release_hash.clone(),
                AuthConfig::default()
                    .with_publishable_key("a4_pk_vector")
                    .with_token_endpoint(binding.auth.session_endpoint.clone()),
            ),
        };
        let client = HttpAuthClient::new(Some(auth), None, http.clone());
        json!({
            "endpoint": endpoint,
            "pathReleaseHash": path_release,
            "tokenRequest": client.token_endpoint_body(Some(&target), &["read".to_string()]),
            "sessionEndpoint": binding.auth.session_endpoint,
            "identityHeader": null,
        })
    };

    let stack = if let Some(edge_url) = edge_url {
        let (base, query) = edge_stack_query(edge_url, &installed.release);
        json!({
            "websocketUrl": edge_stack_websocket_url(edge_url, &installed.release),
            "queryBaseUrl": base,
            "queryParams": query
                .into_iter()
                .map(|(name, value)| (name, json!(value)))
                .collect::<serde_json::Map<String, Value>>(),
            "sessionRequired": false,
            "sessionRequest": null,
            "sessionEndpoint": null,
            "identityHeader": null,
        })
    } else {
        let (websocket_url, query_base, auth) = match &resolver {
            Some(resolver) => {
                let target = surface_target(answer.unwrap(), Surface::Stack(0)).unwrap();
                let mut auth = resolver.token_auth(DEFAULT_HOSTED_TOKEN_ENDPOINT);
                auth.stack_release = Some(StackRelease {
                    stack_manifest_hash: target_str(target, "stackManifestHash")
                        .unwrap()
                        .to_string(),
                    live_alias: installed.release.live_alias.clone(),
                });
                (
                    target_str(target, "websocketUrl").unwrap().to_string(),
                    stack_query_base(target, "klend").unwrap(),
                    auth,
                )
            }
            None => (
                installed.stack_ws.clone(),
                installed.stack_http.clone(),
                AuthConfig::default()
                    .with_publishable_key("a4_pk_vector")
                    .with_stack_release(Some(installed.release.clone())),
            ),
        };
        let client = HttpAuthClient::new(Some(auth), Some(websocket_url.clone()), http.clone());
        json!({
            "websocketUrl": websocket_url,
            "queryBaseUrl": query_base,
            "queryParams": {},
            "sessionRequired": true,
            "sessionRequest": client.token_endpoint_body(None, &["read".to_string()]),
            "sessionEndpoint": DEFAULT_HOSTED_TOKEN_ENDPOINT,
            "identityHeader": null,
        })
    };

    let identity = installed.protocol_identity();
    let protocol = if let Some(edge_url) = edge_url {
        let api_url = edge_protocol_api_url(edge_url);
        let segment = edge_protocol_segment(identity.deployment_id.as_deref(), &identity.package);
        let request = ResolveRequest {
            protocols: vec![identity.clone()],
            ..ResolveRequest::default()
        };
        json!({
            "apiUrl": api_url,
            "sessionEndpoint": null,
            "deploymentId": identity.deployment_id,
            "deploymentPathSegment": segment,
            "instancesUrl": format!("{api_url}/protocol-deployments/{segment}/instances"),
            "identityHeader": header(Some(EdgeRequestPolicy::new(edge_url, Some(&request)))),
        })
    } else {
        let (api_url, session_endpoint, deployment_id) = match &resolver {
            Some(_) => {
                let target = surface_target(answer.unwrap(), Surface::Protocol(0)).unwrap();
                (
                    target_str(target, "apiUrl").unwrap().to_string(),
                    target_str(target, "sessionEndpoint").unwrap().to_string(),
                    target_str(target, "deploymentId").unwrap().to_string(),
                )
            }
            None => (
                installed.protocol["deployment"]["apiUrl"]
                    .as_str()
                    .unwrap()
                    .to_string(),
                installed.protocol["deployment"]["sessionEndpoint"]
                    .as_str()
                    .unwrap()
                    .to_string(),
                identity.deployment_id.clone().unwrap(),
            ),
        };
        let segment = encode_segment(&deployment_id);
        json!({
            "apiUrl": api_url,
            "sessionEndpoint": session_endpoint,
            "deploymentId": deployment_id,
            "deploymentPathSegment": segment,
            "instancesUrl": format!("{api_url}/protocol-deployments/{segment}/instances"),
            "identityHeader": null,
        })
    };

    let resolve_request = match &resolver {
        Some(resolver) => json!({
            "url": resolve_url(DEFAULT_API_ORIGIN, resolver.environment()),
            "authorization": "secret-key",
            "body": serde_json::to_value(installed.resolve_request()).unwrap(),
        }),
        None => Value::Null,
    };

    json!({
        "resolveRequest": resolve_request,
        "chain": gateway(true),
        "transactions": gateway(false),
        "programReads": { "klend": program_read },
        "stack": stack,
        "protocol": protocol,
    })
}

#[test]
fn target_vectors() {
    let file = load("targets.json");
    for case in cases(&file) {
        let name = case["name"].as_str().unwrap();
        let input = &case["input"];
        let installed = installed(&file, input["installed"].as_str().unwrap());
        let mode = mode_of(&input["mode"]);
        let answer = input["resolveResponse"]
            .as_object()
            .map(|_| &input["resolveResponse"]);
        let observed = observe(&installed, &mode, answer);
        for (surface, expected) in case["expect"].as_object().unwrap() {
            assert_eq!(&observed[surface], expected, "{name}: {surface}");
        }
    }
}

// ---------------------------------------------------------------------------
// refusals.json
// ---------------------------------------------------------------------------

fn assert_refusal(name: &str, error: &AreteError, expect: &Value) {
    assert_eq!(error.dev_code(), expect["code"].as_str(), "{name}: {error}");
    let Some(refusal) = error.dev_environment_error() else {
        return;
    };
    if let Some(message) = expect["message"].as_str() {
        assert_eq!(refusal.message, message, "{name}: message");
    }
    if let Some(fix) = expect["fix"].as_str() {
        assert_eq!(refusal.fix.as_deref(), Some(fix), "{name}: fix");
    }
    if let Some(details) = expect["details"].as_object() {
        let observed: serde_json::Map<String, Value> = refusal
            .details
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect();
        assert_eq!(&observed, details, "{name}: details");
    }
}

fn surface_of(name: &str) -> Surface {
    match name {
        "chain" => Surface::Chain,
        "transactions" => Surface::Transactions,
        "stack" | "stackQuery" => Surface::Stack(0),
        "protocol" => Surface::Protocol(0),
        other if other.starts_with("programReads.") => Surface::ProgramRead(0),
        other => panic!("unknown surface {other}"),
    }
}

/// One answer from a local server, for every request.
async fn serve(status: u16, headers: Value, body: Value) -> String {
    use axum::response::IntoResponse;
    let app = axum::Router::new().fallback(move || {
        let headers = headers.clone();
        let body = body.clone();
        async move {
            let mut response = match &body {
                Value::String(text) => text.clone().into_response(),
                other => axum::Json(other.clone()).into_response(),
            };
            *response.status_mut() = axum::http::StatusCode::from_u16(status).unwrap();
            for (name, value) in headers.as_object().unwrap() {
                response.headers_mut().insert(
                    axum::http::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                    axum::http::HeaderValue::from_str(value.as_str().unwrap()).unwrap(),
                );
            }
            response
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{address}")
}

/// A loopback port nothing listens on.
async fn closed_port() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    format!("http://{address}")
}

/// The first chain read of a client in edge mode, against `edge_url`.
async fn first_edge_request(edge_url: &str) -> AreteError {
    let file = load("targets.json");
    let installed = installed(&file, "default");
    let dev = DevMode::Edge {
        edge_url: edge_url.to_string(),
    };
    let (chain, _) = crate::gateway::dev_gateway_transports(
        Some(&installed.gateway),
        &dev,
        None,
        &reqwest::Client::new(),
    );
    match chain.clock().await.expect_err("the edge refuses") {
        crate::chain::ChainError::Sdk(error) => error,
        other => panic!("not an SDK error: {other:?}"),
    }
}

#[tokio::test]
async fn refusal_vectors() {
    let file = load("refusals.json");
    for case in cases(&file) {
        let name = case["name"].as_str().unwrap();
        let input = &case["input"];
        let expect = &case["expect"];
        match input["source"].as_str().unwrap() {
            "switch" => {
                let (result, _) = run_switch(&input["switch"]);
                assert_refusal(name, &result.expect_err(name), expect);
            }
            "edge" => {
                let answer = &input["edgeAnswer"];
                let edge_url = match answer["kind"].as_str().unwrap() {
                    "connection-refused" => closed_port().await,
                    _ => {
                        serve(
                            answer["status"].as_u64().unwrap() as u16,
                            answer["headers"].clone(),
                            answer["body"].clone(),
                        )
                        .await
                    }
                };
                let error = if answer["request"].as_str() == Some("GET /.well-known/arete-dev-edge")
                {
                    // The check before a socket: it fails, so none opens.
                    check_edge(&reqwest::Client::new(), &edge_url)
                        .await
                        .expect_err(name)
                } else {
                    first_edge_request(&edge_url).await
                };
                assert_refusal(name, &error, expect);
            }
            "resolve" => {
                let answer = &input["resolveResponse"];
                let failing = expect["failingSurface"].as_str().unwrap();
                let error = if failing == "stackQuery" {
                    let target = surface_target(answer, Surface::Stack(0)).expect(name);
                    stack_query_base(target, "klend").expect_err(name)
                } else {
                    surface_target(answer, surface_of(failing)).expect_err(name)
                };
                assert_refusal(name, &error, expect);
                if expect["otherSurfacesUsable"] == json!(true) {
                    for surface in [
                        "chain",
                        "transactions",
                        "stack",
                        "protocol",
                        "programReads.klend",
                    ] {
                        if surface == failing {
                            continue;
                        }
                        assert!(
                            surface_target(answer, surface_of(surface)).is_ok(),
                            "{name}: {surface} still works"
                        );
                    }
                }
            }
            "service" => {
                let response = &input["response"];
                let headers = response["headers"].as_object().unwrap();
                let header_code = headers
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("X-Error-Code"))
                    .and_then(|(_, value)| value.as_str());
                let body = serde_json::to_vec(&response["body"]).unwrap();
                let error = refusal_error(
                    response["status"].as_u64().unwrap() as u16,
                    header_code,
                    &body,
                )
                .expect(name);
                assert_refusal(name, &error, expect);
                // Through the edge, the same answer is relayed as the
                // error of the first request.
                if input["mode"]["via"] == "edge" {
                    let edge_url = serve(
                        response["status"].as_u64().unwrap() as u16,
                        response["headers"].clone(),
                        response["body"].clone(),
                    )
                    .await;
                    assert_refusal(name, &first_edge_request(&edge_url).await, expect);
                }
            }
            other => panic!("{name}: unknown source {other}"),
        }
    }
}

/// An edge request carries the identity header and no credential, and a
/// production client's requests are unchanged by the switch code.
#[tokio::test]
async fn edge_requests_name_the_identity_and_carry_no_credential() {
    use std::sync::{Arc, Mutex};
    let seen: Arc<Mutex<Vec<axum::http::HeaderMap>>> = Arc::default();
    let recorded = seen.clone();
    let app = axum::Router::new().fallback(move |headers: axum::http::HeaderMap| {
        let recorded = recorded.clone();
        async move {
            recorded.lock().unwrap().push(headers);
            (
                [(DEV_EDGE_HEADER, "1")],
                axum::Json(json!({ "slot": 1, "unixTimestamp": 2, "epoch": 3 })),
            )
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let edge_url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let file = load("targets.json");
    let installed = installed(&file, "default");
    let (chain, _) = crate::gateway::dev_gateway_transports(
        Some(&installed.gateway),
        &DevMode::Edge {
            edge_url: edge_url.clone(),
        },
        None,
        &reqwest::Client::new(),
    );
    let _ = chain.clock().await;
    let seen = seen.lock().unwrap();
    let headers = seen.first().expect("a request reached the edge");
    assert!(headers.get("authorization").is_none());
    let identity: Value = serde_json::from_str(
        headers
            .get(DEV_IDENTITY_HEADER)
            .expect("identity header")
            .to_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        identity,
        json!({ "chain": { "solanaGatewayBindingId": installed.gateway.chain.solana_gateway_binding_id } })
    );
}

/// A Program Read built the way generated code builds it
/// (`ProgramBuilder::account_transport_of` with the program's address)
/// names `programId` in the edge identity header and in direct mode's
/// resolve request.
#[tokio::test]
async fn program_reads_name_the_program_id_in_edge_and_direct_mode() {
    use std::sync::{Arc, Mutex};

    use axum::extract::Request;
    use axum::response::IntoResponse;

    #[derive(Debug, Clone)]
    struct Seen {
        path: String,
        identity: Option<String>,
        body: Value,
    }
    let seen: Arc<Mutex<Vec<Seen>>> = Arc::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let recorded = seen.clone();
    let answer_origin = origin.clone();
    let app = axum::Router::new().fallback(move |request: Request| {
        let recorded = recorded.clone();
        let origin = answer_origin.clone();
        async move {
            let path = request.uri().path().to_string();
            let identity = request
                .headers()
                .get(DEV_IDENTITY_HEADER)
                .and_then(|value| value.to_str().ok())
                .map(str::to_string);
            let bytes = axum::body::to_bytes(request.into_body(), usize::MAX)
                .await
                .unwrap();
            let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
            recorded.lock().unwrap().push(Seen {
                path: path.clone(),
                identity,
                body,
            });
            let json = if path.ends_with("/resolve") {
                json!({
                    "environment": { "id": "denv_01jabcdefghjkmnpqrstvwxyz0", "name": "my-env" },
                    "programReads": [{
                        "status": "target",
                        "endpoint": origin,
                        "programReleaseHash": "arete:h1:program-release:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                        "releaseDiffers": false
                    }]
                })
            } else if path.ends_with("/ws/sessions") {
                json!({ "token": "t", "expires_at": 4_102_444_800u64 })
            } else {
                Value::Null
            };
            ([(DEV_EDGE_HEADER, "1")], axum::Json(json)).into_response()
        }
    });
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let file = load("targets.json");
    let mut descriptor_json = file["installed"]["default"]["programReads"]["klend"].clone();
    descriptor_json["transport"]["binding"]["endpoint"] = json!(origin);
    descriptor_json["transport"]["binding"]["auth"]["sessionEndpoint"] =
        json!(format!("{origin}/ws/sessions"));
    let descriptor: ProgramReadDescriptor = serde_json::from_value(descriptor_json).unwrap();
    let program_id = file["installed"]["default"]["programs"]["klend"]["programId"]
        .as_str()
        .unwrap();
    let request = crate::program_read_transport::ProgramReadRequest::Fetch {
        account: "Reserve",
        address: "x",
    };
    let http = reqwest::Client::new();

    // Edge.
    let edge = crate::program::ProgramBuilder::for_client(http.clone(), None, None, None).with_dev(
        Some(DevMode::Edge {
            edge_url: origin.clone(),
        }),
        None,
    );
    edge.account_transport_of("klend", Some(program_id), &descriptor)
        .unwrap()
        .read(&request)
        .await
        .expect("edge read");
    let identity: Value = serde_json::from_str(
        seen.lock().unwrap()[0]
            .identity
            .as_deref()
            .expect("identity header"),
    )
    .unwrap();
    assert_eq!(identity["programReads"][0]["programId"], json!(program_id));
    seen.lock().unwrap().clear();

    // Direct.
    let auth = AuthConfig::default()
        .with_secret_key("a4_sk_vector")
        .with_api_url(origin.clone());
    let resolver = DirectResolver::new(
        "denv_01jabcdefghjkmnpqrstvwxyz0".to_string(),
        auth,
        http.clone(),
    )
    .at(&origin);
    let direct = crate::program::ProgramBuilder::for_client(http, None, None, None)
        .with_dev(Some(DevMode::Direct(Arc::new(resolver))), None);
    direct
        .account_transport_of("klend", Some(program_id), &descriptor)
        .unwrap()
        .read(&request)
        .await
        .expect("direct read");
    let seen = seen.lock().unwrap();
    let resolve = seen
        .iter()
        .find(|request| request.path.ends_with("/resolve"))
        .expect("a resolve request");
    assert_eq!(
        resolve.body["programReads"][0]["programId"],
        json!(program_id)
    );
}
