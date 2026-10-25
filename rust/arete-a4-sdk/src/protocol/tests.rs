//! End-to-end client tests against a mock hosted API.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::Response;
use axum::Router;
use futures_util::StreamExt;
use serde_json::{json, Map, Value};
use tokio::net::TcpListener;

use super::*;
use crate::auth::AuthConfig;

#[derive(Debug, Clone)]
struct Recorded {
    method: Method,
    path: String,
    query: Option<String>,
    headers: HeaderMap,
    body: Value,
}

impl Recorded {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|value| value.to_str().ok())
    }
}

type Responder =
    dyn Fn(&Recorded, usize) -> (u16, Vec<(&'static str, String)>, String) + Send + Sync;

#[derive(Clone)]
struct Mock {
    requests: Arc<Mutex<Vec<Recorded>>>,
    responder: Arc<Responder>,
}

impl Mock {
    fn requests(&self) -> Vec<Recorded> {
        self.requests.lock().unwrap().clone()
    }

    fn api(&self) -> Vec<Recorded> {
        self.requests()
            .into_iter()
            .filter(|request| request.path != "/ws/sessions")
            .collect()
    }

    fn sessions(&self) -> Vec<Recorded> {
        self.requests()
            .into_iter()
            .filter(|request| request.path == "/ws/sessions")
            .collect()
    }
}

async fn handle(
    State(mock): State<Mock>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let recorded = Recorded {
        method,
        path: uri.path().to_string(),
        query: uri.query().map(str::to_string),
        headers,
        body: serde_json::from_slice(&body).unwrap_or(Value::Null),
    };
    let index = {
        let mut requests = mock.requests.lock().unwrap();
        requests.push(recorded.clone());
        requests
            .iter()
            .filter(|request| request.path == recorded.path)
            .count()
            - 1
    };
    let (status, headers, body) = (mock.responder)(&recorded, index);
    let mut response = Response::builder().status(StatusCode::from_u16(status).unwrap());
    let mut has_type = false;
    for (name, value) in headers {
        has_type |= name.eq_ignore_ascii_case("content-type");
        response = response.header(name, value);
    }
    if !has_type {
        response = response.header("content-type", "application/json");
    }
    response.body(axum::body::Body::from(body)).unwrap()
}

/// Session tokens are `tok-<n>`, minted in order.
fn session(index: usize) -> (u16, Vec<(&'static str, String)>, String) {
    (
        200,
        vec![],
        json!({ "token": format!("tok-{index}"), "expires_at": 4_000_000_000u64 }).to_string(),
    )
}

async fn mock(
    responder: impl Fn(&Recorded, usize) -> (u16, Vec<(&'static str, String)>, String)
        + Send
        + Sync
        + 'static,
) -> (Mock, String) {
    let mock = Mock {
        requests: Arc::new(Mutex::new(Vec::new())),
        responder: Arc::new(responder),
    };
    let router = Router::new().fallback(handle).with_state(mock.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (mock, format!("http://{address}"))
}

const STEPS: &[ProtocolStepDefinition] = &[
    ProtocolStepDefinition {
        name: "contribute",
        scope: ProtocolStepScope::Participant,
        access: ProtocolStepAccess::Public,
        guards: &["raise_not_open", "deadline_passed"],
        args: &["amount"],
    },
    ProtocolStepDefinition {
        name: "create",
        scope: ProtocolStepScope::Create,
        access: ProtocolStepAccess::Secret,
        guards: &["soft_cap_above_hard_cap"],
        args: &["softCap"],
    },
];

fn definition() -> ProtocolDefinition {
    ProtocolDefinition {
        name: "tokenRaise",
        version: "1.0.0",
        package: "token-raise",
        package_release_hash: "release",
        protocol_spec_hash: "spec",
        protocol_release_hash: "protocol-release",
        deployment: ProtocolDeploymentDescriptor {
            deployment_id: "dep_1",
            access: ProtocolDeploymentAccess::Public,
            api_url: "https://unused.example.test/api",
            session_endpoint: "https://unused.example.test/ws/sessions",
        },
        instance_visibility: ProtocolInstanceVisibility::Account,
        steps: STEPS,
        events: &[ProtocolEventDefinition {
            name: "raise.succeeded",
            machine: ProtocolMachineKind::Instance,
        }],
        instance_metadata: &[
            ProtocolMetadataFieldDefinition {
                name: "name",
                writer: ProtocolMetadataWriter::Creator,
                filterable: true,
            },
            ProtocolMetadataFieldDefinition {
                name: "tagline",
                writer: ProtocolMetadataWriter::Creator,
                filterable: false,
            },
        ],
        participant_metadata: &[],
        live_stack: None,
    }
}

#[test]
fn endpoints_come_from_options_then_arete_api_url_then_the_descriptor() {
    let definition = definition();
    let none = |_: &str| None;
    let generated =
        resolve_protocol_endpoints(&definition, &ProtocolClientOptions::default(), none);
    assert_eq!(
        generated,
        ProtocolEndpoints {
            deployment_id: "dep_1".into(),
            api_url: "https://unused.example.test/api".into(),
            session_endpoint: "https://unused.example.test/ws/sessions".into(),
        }
    );

    let local = |name: &str| (name == ARETE_API_URL_ENV).then(|| "http://127.0.0.1:48705/".into());
    let switched =
        resolve_protocol_endpoints(&definition, &ProtocolClientOptions::default(), local);
    assert_eq!(switched.api_url, "http://127.0.0.1:48705/api");
    assert_eq!(
        switched.session_endpoint,
        "http://127.0.0.1:48705/ws/sessions"
    );
    assert_eq!(switched.deployment_id, "dep_1");

    let options = ProtocolClientOptions::default()
        .with_api_url("http://127.0.0.1:4100/api/")
        .with_deployment_id("dep_other");
    let overridden = resolve_protocol_endpoints(&definition, &options, local);
    assert_eq!(overridden.api_url, "http://127.0.0.1:4100/api");
    assert_eq!(
        overridden.session_endpoint,
        "http://127.0.0.1:4100/ws/sessions"
    );
    assert_eq!(overridden.deployment_id, "dep_other");

    let explicit = resolve_protocol_endpoints(
        &definition,
        &options.with_session_endpoint("http://sessions.test/ws/sessions"),
        none,
    );
    assert_eq!(
        explicit.session_endpoint,
        "http://sessions.test/ws/sessions"
    );
}

/// A secret key never follows a session endpoint the generated descriptor
/// (or an option on another origin than the configured API) names.
#[tokio::test]
async fn a_secret_key_is_not_sent_to_a_foreign_session_endpoint() {
    if std::env::var_os(ARETE_API_URL_ENV).is_some() {
        // ARETE_API_URL would replace the descriptor's endpoints.
        return;
    }
    let refused = |options: ProtocolClientOptions| async move {
        let client = ProtocolClient::new(&definition(), options).unwrap();
        let error = client
            .list_instances(ProtocolInstanceListFilter::default())
            .await
            .unwrap_err();
        assert_eq!(
            error.wire_code(),
            Some(crate::error::UNTRUSTED_CREDENTIAL_DESTINATION),
            "{error}"
        );
        assert!(!error.to_string().contains("a4_sk_test"), "{error}");
        error.to_string()
    };
    // The descriptor's own api_url is not an app configuration.
    let message = refused(ProtocolClientOptions::default().with_secret_key("a4_sk_test")).await;
    assert!(message.contains("https://unused.example.test"), "{message}");
    // An explicit api_url does not admit a session endpoint elsewhere.
    refused(
        ProtocolClientOptions::default()
            .with_secret_key("a4_sk_test")
            .with_api_url("https://api.example.test/api")
            .with_session_endpoint("https://sessions.example.test/ws/sessions"),
    )
    .await;
}

#[tokio::test]
async fn the_deployment_id_option_is_the_deployment_called() {
    let (mock, base) = mock(|request, index| {
        if request.path == "/ws/sessions" {
            session(index)
        } else {
            (
                200,
                vec![],
                json!({ "instances": [], "nextCursor": null }).to_string(),
            )
        }
    })
    .await;
    let client = ProtocolClient::new(
        &definition(),
        ProtocolClientOptions::default()
            .with_auth(AuthConfig::default().with_secret_key("a4_sk_test"))
            .with_api_url(format!("{base}/api"))
            .with_deployment_id("dep_other"),
    )
    .unwrap();
    assert_eq!(client.deployment_id(), "dep_other");
    client
        .list_instances(ProtocolInstanceListFilter::default())
        .await
        .unwrap();
    // The session endpoint follows the API URL's origin.
    assert_eq!(mock.sessions()[0].body["targetId"], "dep_other");
    assert_eq!(
        mock.api()[0].path,
        "/api/protocol-deployments/dep_other/instances"
    );
}

fn client(base: &str, auth: AuthConfig) -> ProtocolClient {
    ProtocolClient::new(
        &definition(),
        ProtocolClientOptions::default()
            .with_auth(auth)
            .with_api_url(format!("{base}/api/"))
            .with_session_endpoint(format!("{base}/ws/sessions")),
    )
    .unwrap()
}

fn intent_json() -> Value {
    json!({
        "schema": EXECUTION_INTENT_SCHEMA_V1,
        "executionId": "exec_1",
        "deployment": { "deploymentId": "dep_1", "revision": 3, "releaseHash": "r" },
        "step": "contribute",
        "instanceId": "inst_1",
        "participantId": null,
        "wallet": "wal",
        "role": null,
        "callerAccount": "acct",
        "sessionKeyClass": "publishable",
        "transactionFormat": "v1",
        "inputSha256": "aa",
        "resourceHashes": [],
        "providerSnapshotHash": "bb",
        "fees": { "asset": { "kind": "native" }, "gross": "10", "total": "11", "creator": "0", "platform": "1", "placement": "after", "platformDestination": "p", "creatorDestination": null },
        "signers": [{ "class": "wallet", "pubkey": "wal", "name": null }],
        "transactions": [{ "index": 0, "messageSha256": "cc", "signers": ["wal"] }],
        "blockhash": "bh",
        "lastValidBlockHeight": "100",
        "expiresAt": 1_790_000_100u64,
        "createdAt": 1_790_000_000u64
    })
}

#[derive(Debug, Clone, PartialEq)]
enum ContributeGuard {
    RaiseNotOpen,
    DeadlinePassed,
}

impl ProtocolGuardCode for ContributeGuard {
    fn from_wire(wire: &str) -> Option<Self> {
        match wire {
            "raise_not_open" => Some(Self::RaiseNotOpen),
            "deadline_passed" => Some(Self::DeadlinePassed),
            _ => None,
        }
    }

    fn as_wire(&self) -> &str {
        match self {
            Self::RaiseNotOpen => "raise_not_open",
            Self::DeadlinePassed => "deadline_passed",
        }
    }
}

#[tokio::test]
async fn prepare_mints_a_scoped_session_and_returns_the_intent() {
    let (mock, base) = mock(|request, index| match request.path.as_str() {
        "/ws/sessions" => session(index),
        "/api/protocol-instances/inst%201/steps/contribute/prepare" => (
            200,
            vec![],
            json!({ "intent": intent_json(), "transactions": [{ "index": 0, "transactionBase64": "AQID" }] })
                .to_string(),
        ),
        other => panic!("unexpected {other}"),
    })
    .await;
    let client = client(
        &base,
        AuthConfig::default().with_publishable_key("a4_pk_test"),
    );
    assert_eq!(client.key_class(), ProtocolKeyClass::Publishable);

    let result = client
        .prepare::<ContributeGuard, _>(
            "contribute",
            Some("inst 1"),
            "wal",
            &json!({ "amount": "1000" }),
            ProtocolPrepareOptions::default()
                .with_idempotency_key("idem-1")
                .with_request_id("req-1"),
        )
        .await
        .unwrap();
    let prepared = result.prepared().expect("prepared");
    assert_eq!(prepared.intent.execution_id, "exec_1");
    assert_eq!(prepared.transactions[0].bytes().unwrap(), vec![1, 2, 3]);

    let sessions = mock.sessions();
    assert_eq!(sessions.len(), 1);
    assert_eq!(
        sessions[0].header("authorization"),
        Some("Bearer a4_pk_test")
    );
    assert_eq!(
        sessions[0].body,
        json!({
            "targetKind": "protocol-deployment",
            "targetId": "dep_1",
            "scopes": ["protocol:prepare:contribute", "protocol:read", "protocol:submit"]
        })
    );
    let api = mock.api();
    assert_eq!(api[0].method, Method::POST);
    assert_eq!(api[0].header("authorization"), Some("Bearer tok-0"));
    assert_eq!(
        api[0].body,
        json!({
            "idempotencyKey": "idem-1",
            "wallet": "wal",
            "args": { "amount": "1000" },
            "transactionFormat": "v1",
            "clientContext": { "requestId": "req-1" }
        })
    );
}

#[tokio::test]
async fn create_steps_post_to_the_deployment_and_secret_keys_prepare_any_step() {
    let (mock, base) = mock(|request, index| match request.path.as_str() {
        "/ws/sessions" => session(index),
        "/api/protocol-deployments/dep_1/steps/create/prepare" => (
            200,
            vec![],
            json!({ "intent": intent_json(), "transactions": [] }).to_string(),
        ),
        other => panic!("unexpected {other}"),
    })
    .await;
    let client = client(&base, AuthConfig::default().with_secret_key("a4_sk_test"));
    assert_eq!(client.key_class(), ProtocolKeyClass::Secret);
    let result = client
        .prepare::<String, _>(
            "create",
            None,
            "wal",
            &json!({ "softCap": "1" }),
            ProtocolPrepareOptions::default()
                .with_transaction_format(ProtocolTransactionFormat::V0),
        )
        .await
        .unwrap();
    assert!(matches!(result, ProtocolPrepareResult::Prepared(_)));
    assert_eq!(
        mock.sessions()[0].body["scopes"],
        json!(["protocol:prepare:*", "protocol:read", "protocol:submit"])
    );
    let api = mock.api();
    assert_eq!(api[0].body["transactionFormat"], "v0");
    // A generated idempotency key is a UUID.
    assert_eq!(api[0].body["idempotencyKey"].as_str().unwrap().len(), 36);
}

#[test]
fn state_codes_classify_as_state() {
    for (wire, code) in [
        (
            "instance_terminal",
            ProtocolStateRejectionCode::InstanceTerminal,
        ),
        (
            "participant_terminal",
            ProtocolStateRejectionCode::ParticipantTerminal,
        ),
    ] {
        assert_eq!(ProtocolStateRejectionCode::from_wire(wire), Some(code));
        assert_eq!(code.as_wire(), wire);
        assert_eq!(code.to_string(), wire);
        assert_eq!(
            ProtocolRejectionCode::<ContributeGuard>::from_wire(wire),
            ProtocolRejectionCode::State(code)
        );
        assert_eq!(
            ProtocolRejectionCode::<String>::from_wire(wire),
            ProtocolRejectionCode::State(code)
        );
        assert_eq!(ProtocolAccessRejectionCode::from_wire(wire), None);
    }
    assert_eq!(
        ProtocolStateRejectionCode::from_wire(" INSTANCE_TERMINAL "),
        Some(ProtocolStateRejectionCode::InstanceTerminal)
    );
    assert_eq!(
        ProtocolStateRejectionCode::from_wire("step_requires_secret_key"),
        None
    );
    // `typed` keeps a state code.
    let rejection = ProtocolRejection::<String> {
        code: ProtocolRejectionCode::from_wire("instance_terminal"),
        step: "contribute".into(),
        guard: None,
        detail: None,
    };
    assert_eq!(
        rejection.typed::<ContributeGuard>().code,
        ProtocolRejectionCode::State(ProtocolStateRejectionCode::InstanceTerminal)
    );
}

#[tokio::test]
async fn rejections_are_typed() {
    let (_mock, base) = mock(|request, index| match request.path.as_str() {
        "/ws/sessions" => session(index),
        path if path.ends_with("/prepare") => match index {
            0 => (
                422,
                vec![],
                json!({ "rejected": "deadline_passed", "step": "contribute", "guard": "now < params.deadline", "detail": { "now": 5 } })
                    .to_string(),
            ),
            1 => (
                422,
                vec![],
                json!({ "rejected": "brand_new_guard", "step": "contribute" }).to_string(),
            ),
            2 => (
                403,
                vec![("x-error-code", "step_not_in_session_scope".to_string())],
                json!({ "error": "not in scope", "detail": { "scopes": [] } }).to_string(),
            ),
            3 => (
                422,
                vec![],
                json!({ "rejected": "participant_terminal", "step": "contribute", "detail": { "state": "left" } })
                    .to_string(),
            ),
            _ => (500, vec![], json!({ "error": "boom", "code": "internal" }).to_string()),
        },
        other => panic!("unexpected {other}"),
    })
    .await;
    let client = client(
        &base,
        AuthConfig::default().with_publishable_key("a4_pk_test"),
    );
    let args = json!({});
    let prepare = || {
        client.prepare::<ContributeGuard, _>(
            "contribute",
            Some("inst_1"),
            "wal",
            &args,
            ProtocolPrepareOptions::default(),
        )
    };

    let rejection = prepare().await.unwrap().rejection().cloned().unwrap();
    assert_eq!(
        rejection.code,
        ProtocolRejectionCode::Guard(ContributeGuard::DeadlinePassed)
    );
    assert_eq!(rejection.rejected(), "deadline_passed");
    assert_eq!(rejection.guard.as_deref(), Some("now < params.deadline"));
    assert_eq!(rejection.detail.unwrap()["now"], 5);

    let rejection = prepare().await.unwrap().rejection().cloned().unwrap();
    assert_eq!(
        rejection.code,
        ProtocolRejectionCode::Unrecognized("brand_new_guard".into())
    );

    let rejection = prepare().await.unwrap().rejection().cloned().unwrap();
    assert_eq!(
        rejection.code,
        ProtocolRejectionCode::Access(ProtocolAccessRejectionCode::StepNotInSessionScope)
    );
    assert!(rejection.detail.is_some());

    let rejection = prepare().await.unwrap().rejection().cloned().unwrap();
    assert_eq!(
        rejection.code,
        ProtocolRejectionCode::State(ProtocolStateRejectionCode::ParticipantTerminal)
    );
    assert_eq!(rejection.rejected(), "participant_terminal");
    assert_eq!(rejection.detail.unwrap()["state"], "left");

    let error = prepare().await.unwrap_err();
    assert_eq!(error.status(), Some(500));
    assert_eq!(error.wire_code(), Some("internal"));
    assert!(error.to_string().contains("boom"), "{error}");
}

#[tokio::test]
async fn publishable_clients_refuse_secret_steps_locally_and_mint_refusals_become_rejections() {
    let (mock, base) = mock(|request, _| match request.path.as_str() {
        "/ws/sessions" => (
            403,
            vec![],
            json!({ "error": "needs a secret key", "code": "step_requires_secret_key" })
                .to_string(),
        ),
        other => panic!("unexpected {other}"),
    })
    .await;
    let client = client(
        &base,
        AuthConfig::default().with_publishable_key("a4_pk_test"),
    );
    let local = client
        .prepare::<String, _>(
            "create",
            None,
            "wal",
            &json!({}),
            ProtocolPrepareOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        local.rejection().unwrap().code,
        ProtocolRejectionCode::Access(ProtocolAccessRejectionCode::StepRequiresSecretKey)
    );
    let unknown = client
        .prepare::<String, _>(
            "finalize",
            None,
            "wal",
            &json!({}),
            ProtocolPrepareOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        unknown.rejection().unwrap().code,
        ProtocolRejectionCode::Access(ProtocolAccessRejectionCode::StepNotClientCallable)
    );
    assert!(mock.requests().is_empty());

    let minted = client
        .prepare::<String, _>(
            "contribute",
            Some("inst_1"),
            "wal",
            &json!({}),
            ProtocolPrepareOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        minted.rejection().unwrap().code,
        ProtocolRejectionCode::Access(ProtocolAccessRejectionCode::StepRequiresSecretKey)
    );
    assert!(mock.api().is_empty());
}

#[tokio::test]
async fn a_secret_client_refuses_a_publishable_key() {
    let error = ProtocolClient::new(
        &definition(),
        ProtocolClientOptions::default()
            .with_publishable_key("a4_pk_test")
            .with_key_class(ProtocolKeyClass::Secret),
    )
    .unwrap_err();
    assert!(error.to_string().contains("publishable key"), "{error}");
}

#[tokio::test]
async fn expired_sessions_are_reminted_once() {
    let (mock, base) = mock(|request, index| match request.path.as_str() {
        "/ws/sessions" => session(index),
        "/api/execution-intents/exec_1/submit" => match index {
            0 => (
                401,
                vec![],
                json!({ "error": "expired", "code": "token-expired" }).to_string(),
            ),
            _ => (
                200,
                vec![],
                json!({ "executionId": "exec_1", "state": "submitted_unknown", "transactions": [{ "index": 0, "signature": "sig", "state": "sent" }] })
                    .to_string(),
            ),
        },
        "/api/execution-intents/exec_1" => (
            401,
            vec![("x-error-code", "TOKEN_INVALID_SIGNATURE".to_string())],
            String::new(),
        ),
        other => panic!("unexpected {other}"),
    })
    .await;
    let client = client(
        &base,
        AuthConfig::default().with_publishable_key("a4_pk_test"),
    );
    let state = client.submit("exec_1", [vec![1u8, 2, 3]]).await.unwrap();
    assert_eq!(state.state, ProtocolIntentStatus::SubmittedUnknown);
    assert!(!state.state.is_final());
    let api = mock.api();
    assert_eq!(api.len(), 2);
    assert_eq!(api[0].body, json!({ "signedTransactions": ["AQID"] }));
    assert_eq!(api[0].header("authorization"), Some("Bearer tok-0"));
    assert_eq!(api[1].header("authorization"), Some("Bearer tok-1"));
    assert_eq!(api[1].body, api[0].body);

    // A read retries once and then reports the failure.
    let error = client.intent("exec_1").await.unwrap_err();
    assert_eq!(error.status(), Some(401));
    assert_eq!(error.wire_code(), Some("TOKEN_INVALID_SIGNATURE"));
    let reads = mock
        .api()
        .into_iter()
        .filter(|request| request.path == "/api/execution-intents/exec_1")
        .count();
    assert_eq!(reads, 2);
    // Reads use a read-only session.
    assert_eq!(
        mock.sessions().last().unwrap().body["scopes"],
        json!(["protocol:read"])
    );
}

#[tokio::test]
async fn reads_and_metadata_updates() {
    let summary = json!({
        "id": "inst_1",
        "deploymentId": "dep_1",
        "revision": 3,
        "state": "open",
        "terminal": false,
        "visibility": "account",
        "accounts": { "presale": "P" },
        "roles": {},
        "createdByWallet": "wal",
        "createdAt": "2026-10-13T00:00:00Z",
        "updatedAt": "2026-10-13T00:00:00Z",
        "closedAt": null
    });
    let detail = {
        let mut detail = summary.clone();
        let extra = json!({
            "releaseHash": "r",
            "params": { "softCap": "1" },
            "lastErrorCode": null,
            "derived": {},
            "metadata": { "name": "Raise" },
            "metadataVersion": 4,
            "pendingTimers": [],
            "allowedSteps": [{ "step": "contribute", "allowed": false, "reason": "raise_not_open" }]
        });
        detail
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        detail
    };
    let page = json!({ "instances": [summary], "nextCursor": "c2" });
    let (mock, base) = mock(move |request, index| match request.path.as_str() {
        "/ws/sessions" => session(index),
        "/api/protocol-deployments/dep_1/instances" => (200, vec![], page.to_string()),
        "/api/protocol-instances/inst_1" => (200, vec![], detail.to_string()),
        "/api/protocol-instances/inst_1/participants/wal" => (
            200,
            vec![],
            json!({ "instanceId": "inst_1", "wallet": "wal", "participantId": null, "state": null, "metadata": {}, "history": [], "allowedSteps": [] }).to_string(),
        ),
        "/api/protocol-instances/inst_1/metadata" => (
            200,
            vec![],
            json!({ "instanceId": "inst_1", "participantId": "part_1", "version": 5, "metadata": { "bio": "hi" } }).to_string(),
        ),
        other => panic!("unexpected {other}"),
    })
    .await;
    let client = client(&base, AuthConfig::default().with_secret_key("a4_sk_test"));

    let listed = client
        .list_instances(
            ProtocolInstanceListFilter::default()
                .with_state("open")
                .with_metadata("name", "My Raise")
                .with_limit(10),
        )
        .await
        .unwrap();
    assert_eq!(listed.instances[0].id, "inst_1");
    assert_eq!(listed.next_cursor.as_deref(), Some("c2"));
    assert!(client
        .list_instances(ProtocolInstanceListFilter::default().with_metadata("tagline", "x"))
        .await
        .unwrap_err()
        .to_string()
        .contains("not filterable"));

    let instance = client.instance("inst_1", Some("wal")).await.unwrap();
    assert_eq!(instance.state, "open");
    assert_eq!(instance.metadata_version, Some(4));
    let allowed = client.allowed_steps("inst_1", "wal").await.unwrap();
    assert_eq!(allowed[0].reason.as_deref(), Some("raise_not_open"));
    let metadata = client.instance_metadata("inst_1").await.unwrap();
    assert_eq!(metadata.metadata["name"], "Raise");
    let participant = client.participant("inst_1", "wal").await.unwrap();
    assert_eq!(participant.participant_id, None);

    let patch = json!({ "bio": "hi" }).as_object().unwrap().clone();
    let proof = client
        .sign_metadata_proof("inst_1", "wal", &patch, None, |bytes| async move {
            let message = String::from_utf8(bytes).unwrap();
            assert!(message.starts_with("arete.protocol-metadata/v1\ndep_1\ninst_1\nwal\n"));
            Ok::<_, std::convert::Infallible>(vec![1u8; 64])
        })
        .await
        .unwrap();
    let updated = client
        .update_metadata(
            "inst_1",
            patch,
            ProtocolMetadataUpdateOptions {
                participant: Some("wal".into()),
                wallet_proof: Some(proof.clone()),
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.version, 5);

    let api = mock.api();
    assert_eq!(
        api[0].query.as_deref(),
        Some("state=open&limit=10&metadata.name=My+Raise")
    );
    let reads = api
        .iter()
        .filter(|request| request.path == "/api/protocol-instances/inst_1")
        .map(|request| request.query.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        reads,
        vec![Some("wallet=wal".into()), Some("wallet=wal".into()), None]
    );
    let update = api.last().unwrap();
    assert_eq!(update.method, Method::PATCH);
    assert_eq!(
        update.body,
        json!({
            "patch": { "bio": "hi" },
            "participant": "wal",
            "proof": { "wallet": "wal", "signature": proof.signature, "expiresAt": proof.expires_at }
        })
    );
    assert!(mock
        .sessions()
        .iter()
        .all(|session| session.body["scopes"] == json!(["protocol:read"])));
}

#[tokio::test]
async fn wallet_actions_read_the_wallet_route_with_a_read_session() {
    let full = json!({
        "deploymentId": "dep_1",
        "wallet": "wal",
        "createSteps": [{
            "step": "create",
            "allowed": true,
            "scope": "create",
            "args": [
                { "name": "softCap", "type": "string", "format": "u64", "optional": false, "description": "Minimum raise" },
                { "name": "memo", "type": "string", "optional": true },
                { "name": "members", "type": "array", "optional": false,
                  "items": { "type": "string", "format": "pubkey" }, "maxItems": 10 }
            ]
        }],
        "instances": [{
            "instanceId": "inst_1",
            "state": "open",
            "relation": ["participant", "role:admin", "creator"],
            "participantState": "contributed",
            "steps": [{
                "step": "contribute",
                "allowed": false,
                "reason": "deadline_passed",
                "argsDependent": ["raise_not_open"],
                "scope": "participant",
                "args": [{ "name": "amount", "type": "string", "format": "u64", "optional": false }]
            }]
        }],
        "nextCursor": "c2"
    });
    // An older or sparser response: no optional fields anywhere.
    let sparse = json!({
        "deploymentId": "dep_1",
        "wallet": "wal",
        "instances": [{
            "instanceId": "inst_2",
            "state": "closed",
            "steps": [{ "step": "contribute", "allowed": true }]
        }]
    });
    let (mock, base) = mock(move |request, index| match request.path.as_str() {
        "/ws/sessions" => session(index),
        "/api/protocol-deployments/dep_1/wallets/wal/actions" => {
            let body = if index == 0 { &full } else { &sparse };
            (200, vec![], body.to_string())
        }
        other => panic!("unexpected {other}"),
    })
    .await;
    let client = client(&base, AuthConfig::default().with_secret_key("a4_sk_test"));

    let actions = client
        .actions(
            ProtocolWalletActionsRequest::new("wal")
                .with_instance("inst_1")
                .with_cursor("c1")
                .with_limit(25),
        )
        .await
        .unwrap();
    assert_eq!(actions.deployment_id, "dep_1");
    assert_eq!(actions.next_cursor.as_deref(), Some("c2"));
    let create = &actions.create_steps[0];
    assert_eq!(create.scope, Some(ProtocolStepScope::Create));
    assert_eq!(
        create.args,
        vec![
            ProtocolStepArgDescriptor {
                name: "softCap".into(),
                arg_type: "string".into(),
                format: Some("u64".into()),
                optional: false,
                description: Some("Minimum raise".into()),
                schema: Map::new(),
            },
            ProtocolStepArgDescriptor {
                name: "memo".into(),
                arg_type: "string".into(),
                format: None,
                optional: true,
                description: None,
                schema: Map::new(),
            },
            ProtocolStepArgDescriptor {
                name: "members".into(),
                arg_type: "array".into(),
                format: None,
                optional: false,
                description: None,
                schema:
                    json!({ "items": { "type": "string", "format": "pubkey" }, "maxItems": 10 })
                        .as_object()
                        .cloned()
                        .unwrap(),
            },
        ]
    );
    assert_eq!(create.args[0].declared_type(), "u64");
    assert_eq!(create.args[1].declared_type(), "string");
    let instance = &actions.instances[0];
    assert_eq!(instance.instance_id, "inst_1");
    assert_eq!(instance.relation, ["participant", "role:admin", "creator"]);
    assert_eq!(instance.participant_state.as_deref(), Some("contributed"));
    assert_eq!(instance.steps[0].reason.as_deref(), Some("deadline_passed"));
    assert_eq!(
        instance.steps[0].scope,
        Some(ProtocolStepScope::Participant)
    );
    assert_eq!(instance.steps[0].args[0].name, "amount");

    let sparse = client
        .actions(ProtocolWalletActionsRequest::new("wal"))
        .await
        .unwrap();
    assert!(sparse.create_steps.is_empty());
    assert_eq!(sparse.next_cursor, None);
    let instance = &sparse.instances[0];
    assert!(instance.relation.is_empty());
    assert_eq!(instance.participant_state, None);
    assert_eq!(instance.steps[0].scope, None);
    assert!(instance.steps[0].args.is_empty());

    assert!(client
        .actions(ProtocolWalletActionsRequest::new(""))
        .await
        .is_err());

    let api = mock.api();
    assert_eq!(api.len(), 2);
    assert!(api.iter().all(|request| request.method == Method::GET));
    assert_eq!(
        api[0].query.as_deref(),
        Some("instance=inst_1&cursor=c1&limit=25")
    );
    assert_eq!(api[1].query, None);
    assert!(api
        .iter()
        .all(|request| request.header("authorization") == Some("Bearer tok-0")));
    assert!(mock
        .sessions()
        .iter()
        .all(|session| session.body["scopes"] == json!(["protocol:read"])));
}

#[tokio::test]
async fn event_streams_resume_after_the_last_event_id() {
    let (mock, base) = mock(|request, index| match request.path.as_str() {
        "/ws/sessions" => session(index),
        "/api/protocol-instances/inst_1/events" => {
            let sse = vec![("content-type", "text/event-stream".to_string())];
            match index {
                0 => (
                    200,
                    sse,
                    concat!(
                        ": keep-alive\n\n",
                        "id: 1\nevent: transition\ndata: {\"from\":\"open\",\"to\":\"succeeded\"}\n\n",
                        "id: 2\nevent: emit\ndata: {\"id\":7,\"event\":\"raise.succeeded\",\"reaction\":\"notify\",\"instanceId\":\"inst_1\",\"participantId\":null,\"transitionId\":3,\"at\":\"2026-10-13T00:00:00Z\"}\n\n",
                        "id: 3\nevent: transition\ndata: {\"partial\":"
                    )
                    .to_string(),
                ),
                1 => (
                    200,
                    sse,
                    concat!(
                        "event: error\ndata: {\"code\":\"session_expired\"}\n\n",
                        "id: 4\nevent: participant\ndata: {\"wallet\":\"wal\"}\n\n",
                        "event: unknown\ndata: {}\n\n",
                        "event: error\ndata: {\"code\":\"instance_paused\"}\n\n",
                        "id: 5\nevent: observation\ndata: {\"slot\":9}\n\n"
                    )
                    .to_string(),
                ),
                2 => (503, vec![], json!({ "error": "busy" }).to_string()),
                _ => (404, vec![], json!({ "error": "gone", "code": "instance_not_found" }).to_string()),
            }
        }
        other => panic!("unexpected {other}"),
    })
    .await;
    let client = client(
        &base,
        AuthConfig::default().with_publishable_key("a4_pk_test"),
    );
    let mut stream = client.events(
        "inst_1",
        ProtocolEventsOptions {
            initial_backoff: Duration::from_millis(10),
            ..ProtocolEventsOptions::default()
        },
    );
    let mut items = Vec::new();
    while let Some(item) = tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await
        .expect("stream item")
    {
        items.push(item);
    }

    let ids = items
        .iter()
        .filter_map(|item| {
            item.as_ref()
                .ok()
                .map(|event| (event.kind(), event.id().to_string()))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        ids,
        vec![
            ("transition", "1".to_string()),
            ("emit", "2".to_string()),
            ("participant", "4".to_string()),
            ("observation", "5".to_string()),
        ]
    );
    match items[1].as_ref().unwrap() {
        ProtocolStreamEvent::Emit { data, .. } => assert_eq!(data.event, "raise.succeeded"),
        other => panic!("expected emit, got {other:?}"),
    }
    let errors = items
        .iter()
        .filter_map(|item| item.as_ref().err())
        .collect::<Vec<_>>();
    assert_eq!(errors.len(), 3, "{errors:?}");
    assert_eq!(errors[0].wire_code(), Some("instance_paused"));
    assert_eq!(errors[1].status(), Some(503));
    assert_eq!(errors[2].status(), Some(404));
    assert_eq!(stream.last_event_id().as_deref(), Some("5"));

    let streams = mock
        .api()
        .into_iter()
        .filter(|request| request.path.ends_with("/events"))
        .collect::<Vec<_>>();
    assert_eq!(streams.len(), 4);
    assert_eq!(streams[0].header("accept"), Some("text/event-stream"));
    assert_eq!(streams[0].header("last-event-id"), None);
    // The partial event 3 is discarded, so the stream resumes after 2.
    assert_eq!(streams[1].header("last-event-id"), Some("2"));
    assert_eq!(streams[2].header("last-event-id"), Some("5"));
    // The `session_expired` error event forced a fresh session.
    assert_eq!(streams[0].header("authorization"), Some("Bearer tok-0"));
    assert_eq!(streams[1].header("authorization"), Some("Bearer tok-0"));
    assert_eq!(streams[2].header("authorization"), Some("Bearer tok-1"));
}

#[tokio::test]
async fn event_streams_start_after_a_given_id_and_stop_on_drop() {
    let (mock, base) = mock(|request, index| match request.path.as_str() {
        "/ws/sessions" => session(index),
        _ => (
            200,
            vec![("content-type", "text/event-stream".to_string())],
            "id: 8\nevent: transition\ndata: {}\n\n".to_string(),
        ),
    })
    .await;
    let client = client(&base, AuthConfig::default().with_token("static-token"));
    let mut stream = client.events("inst_1", ProtocolEventsOptions::default().resume_after("7"));
    let first = stream.next().await.unwrap().unwrap();
    assert_eq!(first.id(), "8");
    drop(stream);
    let first_request = mock.api()[0].clone();
    assert_eq!(first_request.header("last-event-id"), Some("7"));
    assert_eq!(
        first_request.header("authorization"),
        Some("Bearer static-token")
    );
    assert!(mock.sessions().is_empty());
}

#[tokio::test]
async fn squads_onboarding_reads_the_proposer_builds_the_proposal_and_checks_status() {
    let squads = "/api/protocol-deployments/dep_1/squads".to_string();
    let routes = squads.clone();
    let (mock, base) = mock(move |request, index| {
        let path = request.path.as_str();
        if path == "/ws/sessions" {
            return session(index);
        }
        let body = if path == format!("{routes}/proposer") {
            json!({ "pubkey": "Proposer1", "permissions": "initiate", "rootKeyId": "root-1" })
        } else if path == format!("{routes}/add-proposer") {
            if request.body["member"] == "Voter1" {
                return (
                    422,
                    vec![],
                    json!({ "error": "cannot initiate", "code": "member_cannot_initiate" })
                        .to_string(),
                );
            }
            json!({
                "transactionBase64": "AQID", "transactionFormat": "v0", "transactionIndex": 10,
                "proposal": "Proposal1", "configTransaction": "Config1", "multisig": "Multisig1",
                "member": "Member1", "proposer": "Proposer1", "lastValidBlockHeight": 900
            })
        } else if path == format!("{routes}/status") {
            json!({
                "ready": true, "member": true, "mask": 1, "threshold": 2,
                "multisig": "Multisig1", "proposer": "Proposer1"
            })
        } else {
            panic!("unexpected {path}");
        };
        (200, vec![], body.to_string())
    })
    .await;
    let client = client(&base, AuthConfig::default().with_secret_key("a4_sk_test"));

    let proposer = client.squads().proposer().await.unwrap();
    assert_eq!(proposer.pubkey, "Proposer1");
    assert_eq!(proposer.permissions, "initiate");
    assert_eq!(proposer.root_key_id.as_deref(), Some("root-1"));

    let built = client
        .squads()
        .add_proposer(
            &ProtocolSquadsAddProposerRequest::new("Multisig1", "Member1")
                .with_transaction_format(ProtocolTransactionFormat::V0),
        )
        .await
        .unwrap();
    assert_eq!(built.transaction_index, 10);
    assert_eq!(built.transaction_format, ProtocolTransactionFormat::V0);
    let refused = client
        .squads()
        .add_proposer(&ProtocolSquadsAddProposerRequest::new(
            "Multisig1",
            "Voter1",
        ))
        .await
        .unwrap_err();
    assert_eq!(refused.status(), Some(422));
    assert_eq!(refused.wire_code(), Some("member_cannot_initiate"));

    let status = client.squads().status("Multisig1").await.unwrap();
    assert!(status.ready);
    assert_eq!(status.mask, Some(1));
    assert_eq!(status.problem, None);

    let api = mock.api();
    assert_eq!(api[0].path, format!("{squads}/proposer"));
    assert_eq!(api[1].method, Method::POST);
    assert_eq!(
        api[1].body,
        json!({ "multisig": "Multisig1", "member": "Member1", "transactionFormat": "v0" })
    );
    assert_eq!(
        api[2].body,
        json!({ "multisig": "Multisig1", "member": "Voter1" })
    );
    assert_eq!(api[3].path, format!("{squads}/status"));
    assert_eq!(api[3].query.as_deref(), Some("multisig=Multisig1"));
}

#[tokio::test]
async fn proposer_missing_system_events_are_typed() {
    let (_mock, base) = mock(|request, index| match request.path.as_str() {
        "/ws/sessions" => session(index),
        _ => (
            200,
            vec![("content-type", "text/event-stream".to_string())],
            concat!(
                "id: 1-2-3-7\nevent: arete.squads.proposerMissing\n",
                "data: {\"id\":7,\"deploymentId\":\"d\",\"instanceId\":\"inst_1\",\"outboxId\":3,",
                "\"step\":\"finalize\",\"multisig\":\"Multisig1\",\"proposer\":\"Proposer1\",",
                "\"problem\":\"not_member\",\"at\":\"2026-10-14T00:00:00Z\"}\n\n"
            )
            .to_string(),
        ),
    })
    .await;
    let client = client(&base, AuthConfig::default().with_token("static-token"));
    let mut stream = client.events("inst_1", ProtocolEventsOptions::default());
    let event = stream.next().await.unwrap().unwrap();
    assert_eq!(event.kind(), SQUADS_PROPOSER_MISSING_EVENT);
    assert_eq!(event.id(), "1-2-3-7");
    match event {
        ProtocolStreamEvent::SquadsProposerMissing { data, .. } => {
            assert_eq!(data.proposer, "Proposer1");
            assert_eq!(data.problem, "not_member");
            assert_eq!(data.outbox_id, 3);
        }
        other => panic!("expected proposerMissing, got {other:?}"),
    }
}

fn instance_detail(revision: u64, presale: &str) -> String {
    json!({
        "id": "inst_1",
        "deploymentId": "dep_1",
        "revision": revision,
        "state": if revision > 3 { "succeeded" } else { "open" },
        "terminal": false,
        "visibility": "account",
        "accounts": { "presale": presale },
        "roles": {},
        "createdByWallet": "wal",
        "createdAt": "2026-10-13T00:00:00Z",
        "updatedAt": "2026-10-13T00:00:00Z",
        "releaseHash": "r",
        "allowedSteps": []
    })
    .to_string()
}

/// One instance read per request (revision 3 with presale `P1`, then
/// revision 4 with `P2`), and an event stream that sends one event per
/// connection and asks for a minute before reconnecting.
async fn handle_mock(events: &'static str) -> (Mock, String) {
    mock(move |request, index| match request.path.as_str() {
        "/ws/sessions" => session(index),
        "/api/protocol-instances/inst_1" if index == 0 => (200, vec![], instance_detail(3, "P1")),
        "/api/protocol-instances/inst_1" => (200, vec![], instance_detail(4, "P2")),
        "/api/protocol-instances/inst_1/events" => (
            200,
            vec![("content-type", "text/event-stream".to_string())],
            events.to_string(),
        ),
        other => panic!("unexpected {other}"),
    })
    .await
}

async fn wait_for(
    handle: &ProtocolInstanceHandle,
    what: &str,
    predicate: impl FnMut(&ProtocolInstanceState) -> bool,
) -> ProtocolInstanceState {
    let mut receiver = handle.subscribe();
    let state = tokio::time::timeout(Duration::from_secs(5), receiver.wait_for(predicate))
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for {what}"))
        .expect("handle open")
        .clone();
    state
}

#[tokio::test]
async fn instance_handles_load_and_refresh_on_events() {
    let (mock, base) = handle_mock(
        "retry: 60000\nid: 1\nevent: transition\ndata: {\"from\":\"open\",\"to\":\"succeeded\"}\n\nid: 2\nevent: participant\ndata: {}\n\n",
    )
    .await;
    let client = client(&base, AuthConfig::default().with_token("static-token"));
    let mut handle = client
        .instance_handle_with(
            "inst_1",
            ProtocolInstanceHandleOptions::default().with_wallet("wal"),
        )
        .await
        .unwrap();
    assert_eq!(handle.instance_id(), "inst_1");
    // The definition has no live stack, so there is never a live row.
    assert!(handle.current().live.is_none());

    let state = wait_for(&handle, "the refresh", |state| {
        state.snapshot.instance.revision == 4
    })
    .await;
    assert_eq!(state.snapshot.instance.state, "succeeded");
    assert!(state.error.is_none());

    let reads = mock
        .api()
        .into_iter()
        .filter(|request| request.path == "/api/protocol-instances/inst_1")
        .collect::<Vec<_>>();
    // Two events arrived together: one read for both.
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        mock.api()
            .iter()
            .filter(|request| request.path == "/api/protocol-instances/inst_1")
            .count(),
        2
    );
    assert!(reads
        .iter()
        .all(|request| request.query.as_deref() == Some("wallet=wal")));

    let refreshed = handle.refresh().await.unwrap();
    assert_eq!(refreshed.instance.revision, 4);

    handle.close();
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(handle.is_closed());
    let error = handle.refresh().await.unwrap_err();
    assert!(error.to_string().contains("closed"), "{error}");
}

#[tokio::test]
async fn instance_handles_fail_when_the_first_read_fails() {
    let (_mock, base) = mock(|request, index| match request.path.as_str() {
        "/ws/sessions" => session(index),
        _ => (
            404,
            vec![],
            json!({ "error": "gone", "code": "instance_not_found" }).to_string(),
        ),
    })
    .await;
    let client = client(&base, AuthConfig::default().with_token("static-token"));
    let error = client.instance_handle("inst_1").await.unwrap_err();
    assert_eq!(error.status(), Some(404));
}

/// A row asked for: stack, view, key, and the sender feeding it.
type AskedRow = (
    String,
    String,
    String,
    tokio::sync::mpsc::UnboundedSender<Value>,
);

/// Live views backed by channels, recording every row they are asked for.
#[derive(Default)]
struct FakeLiveViews {
    rows: Mutex<Vec<AskedRow>>,
}

impl FakeLiveViews {
    fn asked(&self) -> Vec<(String, String, String)> {
        self.rows
            .lock()
            .unwrap()
            .iter()
            .map(|(stack, view, key, _)| (stack.clone(), view.clone(), key.clone()))
            .collect()
    }

    fn send(&self, key: &str, row: Value) {
        let rows = self.rows.lock().unwrap();
        let (.., sender) = rows.iter().rev().find(|(.., k, _)| k == key).unwrap();
        sender.send(row).unwrap();
    }
}

impl ProtocolLiveViews for FakeLiveViews {
    fn listen(&self, stack: &str, view: &str, key: &str) -> Option<ProtocolLiveStream> {
        if stack != "raiseStack" {
            return None;
        }
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        self.rows
            .lock()
            .unwrap()
            .push((stack.into(), view.into(), key.into(), sender));
        Some(Box::pin(futures_util::stream::unfold(
            receiver,
            |mut receiver| async move { receiver.recv().await.map(|row| (row, receiver)) },
        )))
    }
}

#[tokio::test]
async fn instance_handles_follow_the_live_stack_row() {
    let (_mock, base) = handle_mock("retry: 60000\nid: 1\nevent: transition\ndata: {}\n\n").await;
    let views = Arc::new(FakeLiveViews::default());
    let definition = ProtocolDefinition {
        live_stack: Some(ProtocolLiveStackDescriptor {
            stack: "raiseStack",
            view: "TokenRaiseInstance/state",
            key_account: "presale",
        }),
        ..definition()
    };
    let client = ProtocolClient::new(
        &definition,
        ProtocolClientOptions::default()
            .with_auth(AuthConfig::default().with_token("static-token"))
            .with_api_url(format!("{base}/api"))
            .with_session_endpoint(format!("{base}/ws/sessions"))
            .with_live_views(views.clone()),
    )
    .unwrap();

    // The first read keys the row by P1; the event's re-read moves it to P2.
    let handle = client.instance_handle("inst_1").await.unwrap();
    let state = wait_for(&handle, "the re-keyed row", |state| {
        state.snapshot.instance.revision == 4
    })
    .await;
    assert_eq!(state.snapshot.instance.accounts["presale"], "P2");
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(
        views.asked(),
        vec![
            (
                "raiseStack".to_string(),
                "TokenRaiseInstance/state".to_string(),
                "P1".to_string()
            ),
            (
                "raiseStack".to_string(),
                "TokenRaiseInstance/state".to_string(),
                "P2".to_string()
            ),
        ]
    );

    views.send("P2", json!({ "totalDeposit": "42" }));
    let state = wait_for(&handle, "the live row", |state| {
        state.snapshot.live.is_some()
    })
    .await;
    assert_eq!(state.snapshot.live, Some(json!({ "totalDeposit": "42" })));
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Row {
        total_deposit: String,
    }
    let row: Row = state.snapshot.live_as().unwrap().unwrap();
    assert_eq!(row.total_deposit, "42");

    // A re-read with the same key account keeps the same row.
    handle.refresh().await.unwrap();
    assert_eq!(views.asked().len(), 2);
    assert_eq!(handle.current().live, Some(json!({ "totalDeposit": "42" })));
}

#[tokio::test]
async fn receipts_are_fetched_and_verify_offline_and_on_chain() {
    let vectors: Value = serde_json::from_str(include_str!(
        "../../../../test-vectors/protocol-client/execution-receipt-v1.json"
    ))
    .unwrap();
    let receipt = vectors["verifyCases"][0]["receipt"].clone();
    let transaction = vectors["onChainCases"][0]["transaction"].clone();
    let signature = receipt["signature"].as_str().unwrap().to_string();
    let receipt_path = format!("/api/protocol-receipts/{signature}");
    let keys = vectors["keySet"].clone();
    let served = receipt.clone();
    let (mock, base) = mock(move |request, index| match request.path.as_str() {
        "/ws/sessions" => session(index),
        "/api/protocol-receipts/keys" => (200, vec![], keys.to_string()),
        path if path == receipt_path => (200, vec![], served.to_string()),
        "/api/protocol-receipts/missing" => (
            404,
            vec![],
            json!({ "error": "no receipt", "code": "protocol-receipt-not-found" }).to_string(),
        ),
        other => panic!("unexpected {other}"),
    })
    .await;
    let client = client(
        &base,
        AuthConfig::default().with_publishable_key("a4_pk_test"),
    );
    let fetched = client.get_receipt(&signature).await.unwrap();
    let keys = client.receipt_keys().await.unwrap();
    let value = serde_json::to_value(&fetched).unwrap();
    assert_eq!(value, receipt);
    assert_eq!(
        verify_receipt(&value, &keys).unwrap().key_id,
        "test-receipt-1"
    );
    verify_receipt_on_chain(&value, &transaction).unwrap();
    let missing = client.get_receipt("missing").await.unwrap_err();
    assert_eq!(missing.status(), Some(404));
    assert_eq!(missing.wire_code(), Some("protocol-receipt-not-found"));
    // Receipts are reads.
    assert_eq!(
        mock.sessions().last().unwrap().body["scopes"],
        json!(["protocol:read"])
    );
}

#[tokio::test]
async fn live_rows_clear_atomically_when_accounts_change_or_disappear() {
    let (_mock, base) = mock(|request, index| match request.path.as_str() {
        "/ws/sessions" => session(index),
        "/api/protocol-instances/inst_1" => {
            let mut detail: Value = serde_json::from_str(&instance_detail(
                3 + index as u64,
                if index < 2 { "P1" } else { "P2" },
            ))
            .unwrap();
            if index >= 3 {
                detail["accounts"] = json!({});
            }
            (200, vec![], detail.to_string())
        }
        _ => (
            200,
            vec![("content-type", "text/event-stream".into())],
            "retry: 60000\n\n".into(),
        ),
    })
    .await;
    let views = Arc::new(FakeLiveViews::default());
    let definition = ProtocolDefinition {
        live_stack: Some(ProtocolLiveStackDescriptor {
            stack: "raiseStack",
            view: "TokenRaiseInstance/state",
            key_account: "presale",
        }),
        ..definition()
    };
    let client = ProtocolClient::new(
        &definition,
        ProtocolClientOptions::default()
            .with_auth(AuthConfig::default().with_token("static-token"))
            .with_api_url(format!("{base}/api"))
            .with_live_views(views.clone()),
    )
    .unwrap();
    let handle = client.instance_handle("inst_1").await.unwrap();
    for _ in 0..100 {
        if !views.asked().is_empty() {
            break;
        }
        tokio::task::yield_now().await;
    }
    views.send("P1", json!({"total": "11"}));
    wait_for(&handle, "first row", |s| s.snapshot.live.is_some()).await;
    assert_eq!(
        handle.refresh().await.unwrap().live,
        Some(json!({"total":"11"}))
    );
    let changed = handle.refresh().await.unwrap();
    assert_eq!(changed.instance.accounts["presale"], "P2");
    assert!(changed.live.is_none());
    views.send("P2", json!({"total": "22"}));
    wait_for(&handle, "second row", |s| s.snapshot.live.is_some()).await;
    assert!(handle.refresh().await.unwrap().live.is_none());
    assert!(views
        .rows
        .lock()
        .unwrap()
        .iter()
        .all(|(.., sender)| sender.is_closed()));
}
