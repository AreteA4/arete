//! The Rust protocol clients `a4 install protocol --rust` generates, compiled
//! against this SDK and driven against a mock hosted API.
//!
//! The modules are the CLI's goldens for the shared test-vector specs
//! (`cli/tests/golden/protocol-rust/`), rendered without hoisted stacks so
//! they build on their own. Regenerate them with
//! `A4_UPDATE_GOLDEN=1 cargo test -p a4-cli protocol_rust`. The local tip jar
//! client is `a4 dev sdk --rust`'s golden (`cli/tests/golden/dev-sdk/`).
#![deny(warnings)]

extern crate arete_a4_sdk as arete_sdk;

// Private here, so their public items read as unused.
#[allow(dead_code, unused_imports)]
#[rustfmt::skip] // Byte-for-byte generator output.
#[path = "../../../cli/tests/golden/protocol-rust/token-raise.rs"]
mod token_raise;

#[allow(dead_code, unused_imports)]
#[rustfmt::skip] // Byte-for-byte generator output.
#[path = "../../../cli/tests/golden/protocol-rust/escrow-resolution.rs"]
mod escrow_resolution;

// `a4 dev sdk --rust` for a local deployment (no hoisted stacks or programs).
#[allow(dead_code, unused_imports)]
#[rustfmt::skip] // Byte-for-byte generator output.
#[path = "../../../cli/tests/golden/dev-sdk/rust/lib.rs"]
mod local_tip_jar;

use std::sync::{Arc, Mutex};

use arete_sdk::protocol::{
    ProtocolClientOptions, ProtocolEventPayloadV1, ProtocolKeyClass, ProtocolPrepareOptions,
    ProtocolPrepareResult, ProtocolRejectionCode, ProtocolStepAccess, VerifyWebhookOptions,
};
use axum::body::Bytes;
use axum::extract::State;
use axum::http::Uri;
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{json, Value};
use tokio::net::TcpListener;

use token_raise::*;

type Seen = Arc<Mutex<Vec<(String, Value)>>>;

async fn record(
    State(seen): State<Seen>,
    uri: Uri,
    body: Bytes,
) -> (axum::http::StatusCode, Json<Value>) {
    let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    seen.lock()
        .unwrap()
        .push((uri.path().to_string(), body.clone()));
    if uri.path() == "/ws/sessions" {
        return (
            axum::http::StatusCode::OK,
            Json(json!({ "token": "tok", "expires_at": 4_000_000_000u64 })),
        );
    }
    (
        axum::http::StatusCode::UNPROCESSABLE_ENTITY,
        Json(
            json!({ "rejected": "hard_cap_exceeded", "step": "contribute", "guard": "total + amount <= params.hardCap" }),
        ),
    )
}

async fn server() -> (Seen, String) {
    let seen: Seen = Arc::default();
    let router = Router::new()
        .route("/ws/sessions", post(record))
        .fallback(record)
        .with_state(seen.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (seen, format!("http://{address}"))
}

#[test]
fn the_definition_lists_client_callable_steps_only() {
    let steps = TOKEN_RAISE_PROTOCOL
        .steps
        .iter()
        .map(|step| (step.name, step.access))
        .collect::<Vec<_>>();
    assert_eq!(
        steps,
        vec![
            ("claim", ProtocolStepAccess::Public),
            ("contribute", ProtocolStepAccess::Public),
            ("create", ProtocolStepAccess::Secret),
            ("refund", ProtocolStepAccess::Public),
        ]
    );
    assert!(TOKEN_RAISE_PROTOCOL.step("finalize").is_none());
    assert_eq!(
        escrow_resolution::ESCROW_RESOLUTION_PROTOCOL
            .steps
            .iter()
            .filter(|step| step.access == ProtocolStepAccess::Public)
            .count(),
        0
    );
}

#[test]
fn generated_names_round_trip() {
    for state in TokenRaiseInstanceState::ALL {
        assert_eq!(
            TokenRaiseInstanceState::from_wire(state.as_wire()),
            Some(state)
        );
        assert_eq!(
            serde_json::to_value(state).unwrap(),
            Value::String(state.as_wire().into())
        );
    }
    assert!(TokenRaiseInstanceState::Finalized.is_terminal());
    assert!(!TokenRaiseInstanceState::Open.is_terminal());
    assert_eq!(
        TokenRaiseEvent::from_wire("contribution.confirmed").map(TokenRaiseEvent::machine),
        Some(arete_sdk::protocol::ProtocolMachineKind::Participant)
    );
    let args = TokenRaiseCreateArgs {
        deadline: 1_790_000_000u64.into(),
        hard_cap: 1_000u64.into(),
        quote_mint: "q".into(),
        soft_cap: 10u64.into(),
        wallet_cap: u128::MAX.into(),
    };
    assert_eq!(
        serde_json::to_value(&args).unwrap(),
        json!({
            "deadline": "1790000000",
            "hardCap": "1000",
            "quoteMint": "q",
            "softCap": "10",
            "walletCap": u128::MAX.to_string(),
        })
    );
    let metadata = TokenRaiseInstanceMetadata {
        name: Some("Raise".into()),
        ..TokenRaiseInstanceMetadata::default()
    };
    let patch = metadata.to_patch();
    assert_eq!(Value::Object(patch.clone()), json!({ "name": "Raise" }));
    assert_eq!(
        TokenRaiseInstanceMetadata::from_map(&patch).unwrap(),
        metadata
    );
}

#[tokio::test]
async fn typed_clients_prepare_with_typed_rejections() {
    let (seen, base) = server().await;
    let options = || {
        ProtocolClientOptions::default()
            .with_api_url(format!("{base}/api"))
            .with_session_endpoint(format!("{base}/ws/sessions"))
    };
    let client =
        TokenRaisePublishableClient::with_options(options().with_publishable_key("a4_pk_x"))
            .unwrap();
    assert_eq!(client.key_class(), ProtocolKeyClass::Publishable);
    let result = client
        .prepare_contribute(
            "inst_1",
            "wal",
            &TokenRaiseContributeArgs {
                amount: 5u64.into(),
            },
            ProtocolPrepareOptions::default(),
        )
        .await
        .unwrap();
    let ProtocolPrepareResult::Rejected(rejection) = result else {
        panic!("expected a rejection");
    };
    assert_eq!(
        rejection.code,
        ProtocolRejectionCode::Guard(TokenRaiseContributeGuardCode::HardCapExceeded)
    );
    let seen = seen.lock().unwrap().clone();
    assert_eq!(
        seen[0].1["scopes"],
        json!([
            "protocol:prepare:claim",
            "protocol:prepare:contribute",
            "protocol:prepare:refund",
            "protocol:read",
            "protocol:submit"
        ])
    );
    assert_eq!(
        seen[1].0,
        "/api/protocol-instances/inst_1/steps/contribute/prepare"
    );
    assert_eq!(seen[1].1["args"], json!({ "amount": "5" }));

    // A secret client refuses a publishable key; a secret key gets every step.
    assert!(
        TokenRaiseSecretClient::with_options(options().with_publishable_key("a4_pk_x")).is_err()
    );
    let secret =
        TokenRaiseSecretClient::with_options(options().with_secret_key("a4_sk_x")).unwrap();
    assert_eq!(secret.key_class(), ProtocolKeyClass::Secret);
    assert!(secret
        .transaction_scopes()
        .contains(&"protocol:prepare:*".to_string()));
}

#[test]
fn generated_webhook_verifier_reads_the_shared_vector() {
    let vectors: Value = serde_json::from_str(include_str!(
        "../../../test-vectors/protocol-client/webhook-signature-v1.json"
    ))
    .unwrap();
    let case = &vectors["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["expected"]["id"].is_string())
        .unwrap()["input"];
    let headers = case["headers"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(key, value)| (key.clone(), value.as_str().unwrap().to_string()))
        .collect::<Vec<_>>();
    let verified = verify_token_raise_webhook(
        case["secret"].as_str().unwrap(),
        case["body"].as_str().unwrap(),
        &headers,
        VerifyWebhookOptions {
            now: case["now"].as_u64(),
            tolerance_seconds: None,
        },
    )
    .unwrap();
    let payload: &ProtocolEventPayloadV1 = &verified.payload;
    assert_eq!(
        TokenRaiseEvent::of(payload),
        Some(TokenRaiseEvent::RaiseSucceeded)
    );
}

/// The protocol's live stack, as a session member under the key the
/// definition names.
struct PresaleStack;

impl arete_sdk::Stack for PresaleStack {
    type Views = ();
    type Programs = ();

    fn name() -> &'static str {
        "meteora-presale"
    }

    fn url() -> &'static str {
        ""
    }
}

#[tokio::test]
async fn typed_clients_build_from_a_session() {
    let live = TOKEN_RAISE_PROTOCOL
        .live_stack
        .expect("token raise live stack");
    assert_eq!(live.view, "Presale/state");
    let session = arete_sdk::Session::builder()
        .stack_with::<PresaleStack>(live.stack, |member| {
            member.transport(arete_sdk::Transport::Http)
        })
        .auth(arete_sdk::AuthConfig::default().with_secret_key("a4_sk_x"))
        .connect()
        .await
        .unwrap();
    let secret =
        TokenRaiseSecretClient::from_session(&session, ProtocolClientOptions::default()).unwrap();
    assert_eq!(secret.key_class(), ProtocolKeyClass::Secret);
    let views = secret.live_views().expect("session members as live views");
    assert!(views.listen(live.stack, live.view, "Presale1").is_some());
    session.close().await;
}
