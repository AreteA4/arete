//! Client-level integration tests for the phase-2 runtime wiring:
//! `transaction()` / `execute()`, HTTP-only transport mode, and the
//! HTTP-base-URL resolution (explicit > generated > derived).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use arete_a4_sdk::{
    create_prepared_instruction, Arete, AreteError, BuiltAccountMeta, BuiltInstruction,
    ErrorMetadata, ExecuteOptions, FailurePhase, PreparedOperation, Pubkey, SendOptions,
    SendResult, Signer, SignerRegistry, Stack, TransactionFailureOutcome, TransactionOptions,
    Transport, ViewBuilder, ViewHandle, Views, WalletAdapter, WalletError, WalletExecutionContext,
};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::StreamExt;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::time::timeout;

// ---------------------------------------------------------------------------
// Test stack + wallet fixtures
// ---------------------------------------------------------------------------

struct TestViews {
    things: ViewHandle<Value>,
}

impl Views for TestViews {
    fn from_builder(builder: ViewBuilder) -> Self {
        Self {
            things: builder.view("Thing/list"),
        }
    }
}

struct TestStack;

impl Stack for TestStack {
    type Views = TestViews;
    type Programs = ();

    fn name() -> &'static str {
        "client-runtime-test"
    }

    fn url() -> &'static str {
        ""
    }
}

struct GeneratedHttpStack;

impl Stack for GeneratedHttpStack {
    type Views = TestViews;
    type Programs = ();

    fn name() -> &'static str {
        "generated-http-test"
    }

    fn url() -> &'static str {
        "wss://demo.example/socket"
    }

    fn http_url() -> &'static str {
        "http://generated.example"
    }
}

#[derive(Default)]
struct MockWallet {
    key: String,
    fail_with: Mutex<Option<TransactionFailureOutcome>>,
    calls: AtomicUsize,
    saw_transport: Mutex<Vec<bool>>,
    instruction_counts: Mutex<Vec<usize>>,
}

impl MockWallet {
    fn new(key: impl Into<String>) -> Arc<Self> {
        Arc::new(Self {
            key: key.into(),
            ..Self::default()
        })
    }

    fn fail_next(&self, outcome: TransactionFailureOutcome) {
        *self.fail_with.lock().unwrap() = Some(outcome);
    }
}

#[async_trait::async_trait]
impl WalletAdapter for MockWallet {
    fn public_key(&self) -> String {
        self.key.clone()
    }

    async fn sign_and_send(
        &self,
        instructions: &[BuiltInstruction],
        _options: &SendOptions,
        context: &WalletExecutionContext,
    ) -> Result<SendResult, WalletError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.saw_transport
            .lock()
            .unwrap()
            .push(context.transaction_transport.is_some());
        self.instruction_counts
            .lock()
            .unwrap()
            .push(instructions.len());
        if let Some(outcome) = self.fail_with.lock().unwrap().take() {
            return Err(WalletError::from_outcome(outcome));
        }
        Ok(SendResult {
            signature: "mock-signature".to_string(),
            slot: Some(42),
        })
    }
}

struct NamedSigner(String);

impl Signer for NamedSigner {
    fn address(&self) -> String {
        self.0.clone()
    }
}

fn instruction(signer: &Pubkey) -> BuiltInstruction {
    BuiltInstruction {
        program_id: Pubkey::new_unique(),
        accounts: vec![BuiltAccountMeta {
            pubkey: *signer,
            is_signer: true,
            is_writable: true,
        }],
        data: vec![1, 2, 3],
    }
}

async fn http_only_client() -> Arete<TestStack> {
    Arete::<TestStack>::builder()
        .transport(Transport::Http)
        .http_url("http://127.0.0.1:9")
        .connect()
        .await
        .expect("http-only connect should not open a socket")
}

// ---------------------------------------------------------------------------
// transaction()
// ---------------------------------------------------------------------------

#[tokio::test]
async fn transaction_happy_path_passes_transport_in_wallet_context() {
    let signer = Pubkey::new_unique();
    let wallet = MockWallet::new(signer.to_string());
    let client = Arete::<TestStack>::builder()
        .transport(Transport::Http)
        .http_url("http://127.0.0.1:9")
        .wallet(wallet.clone())
        .connect()
        .await
        .unwrap();

    let result = client
        .transaction(&[instruction(&signer)], TransactionOptions::default())
        .await
        .unwrap();
    assert_eq!(result.signature, "mock-signature");
    assert_eq!(result.slot, Some(42));
    assert_eq!(wallet.calls.load(Ordering::SeqCst), 1);
    assert_eq!(*wallet.saw_transport.lock().unwrap(), vec![true]);
    assert_eq!(*wallet.instruction_counts.lock().unwrap(), vec![1]);
}

#[tokio::test]
async fn transaction_without_wallet_is_not_submitted_in_wallet_phase() {
    let client = http_only_client().await;
    let signer = Pubkey::new_unique();
    let error = client
        .transaction(&[instruction(&signer)], TransactionOptions::default())
        .await
        .unwrap_err();
    let outcome = error.transaction_outcome().expect("structured outcome");
    assert_eq!(outcome.phase(), FailurePhase::Wallet);
    assert!(matches!(
        outcome,
        TransactionFailureOutcome::NotSubmitted { .. }
    ));
}

#[tokio::test]
async fn transaction_failure_resolves_program_errors_against_metadata() {
    let signer = Pubkey::new_unique();
    let wallet = MockWallet::new(signer.to_string());
    let client = http_only_client().await;
    wallet.fail_next(TransactionFailureOutcome::ChainFailed {
        signature: Some("failed-signature".to_string()),
        slot: Some(7),
        program_error: Some(arete_a4_sdk::ProgramError::unknown(6001)),
        message: "Unknown error with code 6001".to_string(),
    });

    let error = client
        .transaction(
            &[instruction(&signer)],
            TransactionOptions {
                wallet: Some(wallet.clone()),
                errors: vec![ErrorMetadata {
                    code: 6001,
                    name: "SlippageExceeded".to_string(),
                    msg: "Slippage tolerance exceeded".to_string(),
                }],
                ..TransactionOptions::default()
            },
        )
        .await
        .unwrap_err();

    let outcome = error.transaction_outcome().expect("structured outcome");
    assert_eq!(outcome.signature(), Some("failed-signature"));
    assert_eq!(outcome.slot(), Some(7));
    let program_error = outcome.program_error().expect("resolved program error");
    assert_eq!(program_error.name, "SlippageExceeded");
    assert_eq!(
        outcome.message(),
        "SlippageExceeded (6001): Slippage tolerance exceeded"
    );
    // The call-level wallet override was used (client has no default wallet).
    assert_eq!(wallet.calls.load(Ordering::SeqCst), 1);
}

// ---------------------------------------------------------------------------
// execute()
// ---------------------------------------------------------------------------

fn prepared_requiring(signers: Vec<String>, signer: &Pubkey) -> PreparedOperation {
    create_prepared_instruction(
        "test-op",
        instruction(signer),
        json!({"kind": "test"}),
        Some(signers),
        None,
    )
    .into()
}

#[tokio::test]
async fn execute_merges_client_wallet_and_signer_registry() {
    let signer = Pubkey::new_unique();
    let wallet = MockWallet::new(signer.to_string());
    let registry = Arc::new(SignerRegistry::new());
    registry
        .register_signer(Arc::new(NamedSigner("extra-signer".to_string())))
        .unwrap();
    let client = Arete::<TestStack>::builder()
        .transport(Transport::Http)
        .http_url("http://127.0.0.1:9")
        .wallet(wallet.clone())
        .signer_registry(registry)
        .connect()
        .await
        .unwrap();

    let operation = prepared_requiring(
        vec![signer.to_string(), "extra-signer".to_string()],
        &signer,
    );
    let receipt = client
        .execute(&operation, ExecuteOptions::default())
        .await
        .unwrap();
    assert_eq!(receipt.signatures, vec!["mock-signature"]);
    assert_eq!(receipt.operation_name, "test-op");
    assert_eq!(wallet.calls.load(Ordering::SeqCst), 1);
    // The client's transaction transport rode along in the wallet context.
    assert_eq!(*wallet.saw_transport.lock().unwrap(), vec![true]);
}

#[tokio::test]
async fn execute_call_options_replace_the_client_registry() {
    let signer = Pubkey::new_unique();
    let wallet = MockWallet::new(signer.to_string());
    let registry = Arc::new(SignerRegistry::new());
    registry
        .register_signer(Arc::new(NamedSigner("extra-signer".to_string())))
        .unwrap();
    let client = Arete::<TestStack>::builder()
        .transport(Transport::Http)
        .http_url("http://127.0.0.1:9")
        .wallet(wallet.clone())
        .signer_registry(registry)
        .connect()
        .await
        .unwrap();

    let operation = prepared_requiring(
        vec![signer.to_string(), "extra-signer".to_string()],
        &signer,
    );
    // Call options win: an explicit empty registry replaces the client's,
    // so the required extra signer is now missing and execution fails
    // closed before dispatch.
    let error = client
        .execute(
            &operation,
            ExecuteOptions {
                signer_registry: Some(Arc::new(SignerRegistry::new())),
                ..ExecuteOptions::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(error.outcome.phase(), FailurePhase::Build);
    assert!(error.outcome.message().contains("extra-signer"));
    assert_eq!(wallet.calls.load(Ordering::SeqCst), 0);
}

// ---------------------------------------------------------------------------
// HTTP-only transport mode
// ---------------------------------------------------------------------------

async fn spawn_http_stack() -> String {
    let router = Router::new()
        .route(
            "/chain/clock",
            get(|| async { Json(json!({ "slot": 555u64, "unixTimestamp": 1_700_000_000i64 })) }),
        )
        .route(
            "/transactions/v1/latest-blockhash",
            post(|| async {
                Json(json!({
                    "blockhash": "hash123",
                    "contextSlot": "9",
                    "lastValidBlockHeight": "100",
                }))
            }),
        );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    format!("{addr}")
}

#[tokio::test]
async fn http_only_mode_serves_chain_and_transactions_but_fails_views_fast() {
    let addr = spawn_http_stack().await;
    // Only a WebSocket-style URL is configured: the HTTP base is derived
    // from it (ws:// -> http://) and no socket is ever opened.
    let client = Arete::<TestStack>::builder()
        .url(&format!("ws://{addr}"))
        .transport(Transport::Http)
        .connect()
        .await
        .unwrap();
    assert_eq!(client.transport(), Transport::Http);
    assert_eq!(
        client.http_base_url(),
        Some(format!("http://{addr}")).as_deref()
    );

    // Chain reads and the transaction relay work over the derived base.
    let clock = client.chain().clock().await.unwrap();
    assert_eq!(clock.slot, 555);
    let blockhash = client
        .transactions()
        .latest_blockhash(Default::default())
        .await
        .unwrap();
    assert_eq!(blockhash.blockhash, "hash123");

    // View subscriptions fail fast with a clear error instead of hanging:
    // streams terminate immediately and the error is recorded.
    let mut stream = Box::pin(client.views.things.watch());
    let next = timeout(Duration::from_secs(1), stream.next())
        .await
        .expect("stream must not hang in http-only mode");
    assert!(next.is_none());
    let error = client.last_error().await.expect("recorded error");
    assert!(matches!(*error, AreteError::WebSocketDisabled));

    // get() reports no data (subscription acquisition failed).
    assert!(client.views.things.get().await.is_empty());

    client.disconnect().await;
}

#[tokio::test]
async fn websocket_transport_requires_a_url() {
    let error = Arete::<TestStack>::builder().connect().await.err().unwrap();
    assert!(matches!(error, AreteError::MissingUrl));
}

// ---------------------------------------------------------------------------
// HTTP base resolution: explicit > generated Stack::http_url > derived
// ---------------------------------------------------------------------------

#[tokio::test]
async fn http_base_prefers_explicit_then_generated_then_derived() {
    // Explicit builder URL wins over the generated Stack::http_url.
    let client = Arete::<GeneratedHttpStack>::builder()
        .transport(Transport::Http)
        .http_url("http://explicit.example")
        .connect()
        .await
        .unwrap();
    assert_eq!(client.http_base_url(), Some("http://explicit.example"));

    // Generated Stack::http_url wins over derivation.
    let client = Arete::<GeneratedHttpStack>::builder()
        .transport(Transport::Http)
        .connect()
        .await
        .unwrap();
    assert_eq!(client.http_base_url(), Some("http://generated.example"));

    // Default Stack::http_url is empty: derive from the WebSocket URL.
    let client = Arete::<TestStack>::builder()
        .url("wss://derived.example/socket")
        .transport(Transport::Http)
        .connect()
        .await
        .unwrap();
    assert_eq!(
        client.http_base_url(),
        Some("https://derived.example/socket")
    );

    // No URL at all: no base; chain calls fail with a clear config error.
    let client = Arete::<TestStack>::builder()
        .transport(Transport::Http)
        .connect()
        .await
        .unwrap();
    assert_eq!(client.http_base_url(), None);
    let error = client.chain().clock().await.err().unwrap();
    let error: AreteError = error.into();
    assert!(matches!(error, AreteError::InvalidConfig(_)));
}
