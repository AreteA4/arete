//! Session-level integration tests: typed member accessors, wallet fan-out,
//! shared signer registry, execution-host delegation, and close().

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use arete_a4_sdk::{
    create_prepared_instruction, AreteError, BuiltAccountMeta, BuiltInstruction, ExecuteOptions,
    PreparedOperation, Pubkey, SendOptions, SendResult, Session, Signer, SignerRegistry, Stack,
    TransactionOptions, Transport, ViewBuilder, ViewHandle, Views, WalletAdapter, WalletError,
    WalletExecutionContext,
};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::net::TcpListener;
use tokio::time::timeout;
use tokio_tungstenite::accept_async;

struct TestViews {
    #[allow(dead_code)]
    things: ViewHandle<Value>,
}

impl Views for TestViews {
    fn from_builder(builder: ViewBuilder) -> Self {
        Self {
            things: builder.view("Thing/list"),
        }
    }
}

macro_rules! test_stack {
    ($name:ident, $stack_name:literal) => {
        struct $name;

        impl Stack for $name {
            type Views = TestViews;
            type Programs = ();

            fn name() -> &'static str {
                $stack_name
            }

            fn url() -> &'static str {
                ""
            }
        }
    };
}

test_stack!(StackA, "session-stack-a");
test_stack!(StackB, "session-stack-b");

#[derive(Default)]
struct MockWallet {
    key: String,
    calls: AtomicUsize,
    saw_transport: Mutex<Vec<bool>>,
}

impl MockWallet {
    fn new(key: impl Into<String>) -> Arc<Self> {
        Arc::new(Self {
            key: key.into(),
            ..Self::default()
        })
    }
}

#[async_trait::async_trait]
impl WalletAdapter for MockWallet {
    fn public_key(&self) -> String {
        self.key.clone()
    }

    async fn sign_and_send(
        &self,
        _instructions: &[BuiltInstruction],
        _options: &SendOptions,
        context: &WalletExecutionContext,
    ) -> Result<SendResult, WalletError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.saw_transport
            .lock()
            .unwrap()
            .push(context.transaction_transport.is_some());
        Ok(SendResult {
            signature: "session-signature".to_string(),
            slot: Some(1),
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
            is_writable: false,
        }],
        data: vec![9],
    }
}

async fn http_only_session() -> Session {
    Session::builder()
        .stack::<StackA>("a")
        .stack::<StackB>("b")
        .transport(Transport::Http)
        .endpoints_http("http://127.0.0.1:9")
        .connect()
        .await
        .expect("http-only session should connect without servers")
}

#[tokio::test]
async fn empty_sessions_and_duplicate_keys_are_rejected() {
    let error = Session::builder().connect().await.err().unwrap();
    assert!(matches!(error, AreteError::InvalidConfig(_)));

    let error = Session::builder()
        .stack::<StackA>("dup")
        .stack::<StackB>("dup")
        .transport(Transport::Http)
        .endpoints_http("http://127.0.0.1:9")
        .connect()
        .await
        .err()
        .unwrap();
    assert!(matches!(error, AreteError::InvalidConfig(ref message)
        if message.contains("unique")));
}

#[tokio::test]
async fn typed_accessor_downcasts_and_rejects_wrong_key_or_type() {
    let session = http_only_session().await;
    assert_eq!(session.keys(), vec!["a", "b"]);

    // Correct key + type.
    let a = session.stack::<StackA>("a").unwrap();
    assert_eq!(a.transport(), Transport::Http);
    session.stack::<StackB>("b").unwrap();

    // Unknown key.
    let error = session.stack::<StackA>("missing").err().unwrap();
    assert!(matches!(error, AreteError::InvalidConfig(ref message)
        if message.contains("no stack member 'missing'")));

    // Wrong stack type for the key.
    let error = session.stack::<StackB>("a").err().unwrap();
    assert!(matches!(error, AreteError::InvalidConfig(ref message)
        if message.contains("StackB") && message.contains("StackA")));
}

#[tokio::test]
async fn set_wallet_fans_out_to_every_member() {
    let session = http_only_session().await;
    assert!(session.wallet().is_none());
    assert!(session.stack::<StackA>("a").unwrap().wallet().is_none());

    let wallet = MockWallet::new(Pubkey::new_unique().to_string());
    session.set_wallet(Some(wallet.clone()));
    assert!(session.wallet().is_some());
    assert_eq!(
        session.stack::<StackA>("a").unwrap().public_key(),
        Some(wallet.public_key())
    );
    assert_eq!(
        session.stack::<StackB>("b").unwrap().public_key(),
        Some(wallet.public_key())
    );

    session.set_wallet(None);
    assert!(session.stack::<StackA>("a").unwrap().wallet().is_none());
    assert!(session.stack::<StackB>("b").unwrap().wallet().is_none());
}

#[tokio::test]
async fn execute_uses_the_first_member_with_shared_wallet_and_registry() {
    let signer = Pubkey::new_unique();
    let wallet = MockWallet::new(signer.to_string());
    let registry = Arc::new(SignerRegistry::new());
    registry
        .register_signer(Arc::new(NamedSigner("session-extra".to_string())))
        .unwrap();
    let session = Session::builder()
        .stack::<StackA>("a")
        .stack::<StackB>("b")
        .transport(Transport::Http)
        .endpoints_http("http://127.0.0.1:9")
        .wallet(wallet.clone())
        .signer_registry(registry)
        .connect()
        .await
        .unwrap();

    let operation: PreparedOperation = create_prepared_instruction(
        "session-op",
        instruction(&signer),
        Value::Null,
        Some(vec![signer.to_string(), "session-extra".to_string()]),
        None,
    )
    .into();
    let receipt = session
        .execute(&operation, ExecuteOptions::default())
        .await
        .unwrap();
    assert_eq!(receipt.signatures, vec!["session-signature"]);
    assert_eq!(wallet.calls.load(Ordering::SeqCst), 1);
    assert_eq!(*wallet.saw_transport.lock().unwrap(), vec![true]);

    let result = session
        .transaction(&[instruction(&signer)], TransactionOptions::default())
        .await
        .unwrap();
    assert_eq!(result.signature, "session-signature");
    assert_eq!(wallet.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn close_disconnects_websocket_members() {
    // One real WebSocket member proves close() tears the socket down.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let (closed_tx, closed_rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        while let Ok(Some(Ok(message))) = timeout(Duration::from_secs(5), socket.next()).await {
            if message.is_close() {
                break;
            }
            if message.is_ping() {
                let _ = socket
                    .send(tokio_tungstenite::tungstenite::Message::Pong(Vec::new()))
                    .await;
            }
        }
        let _ = closed_tx.send(());
    });

    let session = Session::builder()
        .stack_with::<StackA>("live", |member| {
            member.url(&url).transport(Transport::WebSocket)
        })
        .stack_with::<StackB>("point-reads", |member| member.transport(Transport::Http))
        .endpoints_http("http://127.0.0.1:9")
        .connect()
        .await
        .unwrap();
    assert_eq!(
        session.stack::<StackA>("live").unwrap().transport(),
        Transport::WebSocket
    );
    session.close().await;
    timeout(Duration::from_secs(3), closed_rx)
        .await
        .expect("server should observe the close")
        .unwrap();
}
