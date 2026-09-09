//! Conformance and behaviour tests for the optional Solana adapter.
//!
//! The codec half is cross-implementation: the payloads in
//! `tests/fixtures/transaction-v1/transactions.json` were produced by
//! `@solana/kit` 8.2.0, and the decode expectations in `rust-codec.json` were
//! written from the generator's inputs. A Rust codec that disagrees with kit
//! therefore fails here instead of agreeing with itself.
#![cfg(feature = "solana-adapter")]

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use arete_a4_sdk::adapters::solana::{
    SolanaAdapterConfig, SolanaWalletAdapter, MAX_LEGACY_TRANSACTION_BYTES,
    MAX_V1_TRANSACTION_BYTES, PROVISIONAL_COMPUTE_UNIT_LIMIT,
    PROVISIONAL_LOADED_ACCOUNTS_DATA_SIZE,
};
use arete_a4_sdk::instruction::{BuiltAccountMeta, BuiltInstruction};
use arete_a4_sdk::operations::{FailurePhase, TransactionFailureOutcome};
use arete_a4_sdk::transactions::{
    Commitment, ConfirmedTransaction, LatestBlockhashResult, SignaturePageEntry,
    SignaturePageOptions, SignatureStatusOptions, SubmissionState, TransactionError,
    TransactionFeeResult, TransactionInspectOptions, TransactionRequestContext,
    TransactionSendOptions, TransactionSendResult, TransactionSignatureStatus,
    TransactionSimulationOptions, TransactionSimulationResult, TransactionTransport,
    TransactionTransportError,
};
use arete_a4_sdk::wallet::{
    ConfirmationLevel, SendOptions, SendResult, TransactionInspectionOptions,
    TransactionResourceOptions, TransactionVersion, WalletAdapter, WalletError,
    WalletExecutionContext,
};

use base64::Engine as _;
use serde_json::{json, Value};
use solana_address::Address;
use solana_hash::Hash;
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::Keypair;
use solana_message::{v0, v1, Message as LegacyMessage, VersionedMessage};
use solana_pubkey::Pubkey;
use solana_signature::Signature;
use solana_signer::{Signer, SignerError};
use solana_transaction::versioned::VersionedTransaction;

const MEMO_PROGRAM: &str = "MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr";
/// The `@solana/kit` fixtures use the all-zero (system program) blockhash.
const FIXTURE_BLOCKHASH: &str = "11111111111111111111111111111111";
const FIXTURE_NAMES: [&str; 5] = ["legacy", "v0", "v1", "v1_oversize", "v1_two_signatures"];

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/transaction-v1")
}

fn load(name: &str) -> Value {
    let path = fixture_dir().join(name);
    serde_json::from_str(&std::fs::read_to_string(&path).expect("fixture is readable"))
        .expect("fixture is JSON")
}

fn base64_bytes(value: &Value) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD
        .decode(value.as_str().expect("base64 string"))
        .expect("valid base64")
}

fn decode(wire: &[u8]) -> VersionedTransaction {
    wincode::deserialize_exact(wire).expect("upstream decoder accepts the payload")
}

fn encode(transaction: &VersionedTransaction) -> Vec<u8> {
    wincode::serialize(transaction).expect("upstream encoder")
}

/// The fixture keys: 32 repeated bytes as an Ed25519 seed.
fn fixture_keypair(seed_byte: u8) -> Keypair {
    Keypair::new_from_array([seed_byte; 32])
}

fn memo_instruction(data: Vec<u8>, signers: &[Address]) -> Instruction {
    Instruction {
        program_id: MEMO_PROGRAM.parse().expect("memo address"),
        accounts: signers
            .iter()
            .map(|address| AccountMeta::new(*address, true))
            .collect(),
        data,
    }
}

fn built_memo(data: Vec<u8>, signers: &[Pubkey]) -> BuiltInstruction {
    BuiltInstruction {
        program_id: MEMO_PROGRAM.parse().expect("memo pubkey"),
        accounts: signers
            .iter()
            .map(|pubkey| BuiltAccountMeta {
                pubkey: *pubkey,
                is_signer: true,
                is_writable: true,
            })
            .collect(),
        data,
    }
}

// ---------------------------------------------------------------------------
// Test doubles
// ---------------------------------------------------------------------------

#[derive(Default, Clone)]
struct Log {
    blockhash_calls: usize,
    fee_messages: Vec<String>,
    simulated: Vec<String>,
    sent: Vec<String>,
    status_calls: usize,
}

enum SendBehaviour {
    Accept,
    /// Relay acknowledges the submission under a signature of its own.
    AcceptAs(String),
    /// Relay error carrying a submission state and an optional signature.
    Reject {
        submission_state: Option<SubmissionState>,
        signature: Option<String>,
    },
}

struct FakeTransport {
    blockhash: String,
    last_valid_block_height: u64,
    block_height: u64,
    fee_lamports: Option<u64>,
    simulation_error: Option<Value>,
    simulate_fails: bool,
    units_consumed: Option<u64>,
    loaded_accounts_data_size: Option<u64>,
    status: Option<TransactionSignatureStatus>,
    /// How long `signature_status` stalls before answering.
    status_delay: Duration,
    send: SendBehaviour,
    log: Mutex<Log>,
}

impl Default for FakeTransport {
    fn default() -> Self {
        Self {
            blockhash: "9zjKZ3cCSHu9tqjX2gDaBUBpDzMSxaGyKmVMhbSNn5vP".to_string(),
            last_valid_block_height: 200,
            block_height: 100,
            fee_lamports: Some(5_000),
            simulation_error: None,
            simulate_fails: false,
            units_consumed: Some(1_234),
            loaded_accounts_data_size: Some(4_096),
            status: None,
            status_delay: Duration::ZERO,
            send: SendBehaviour::Accept,
            log: Mutex::new(Log::default()),
        }
    }
}

impl FakeTransport {
    fn confirmed_at(slot: u64) -> Self {
        Self {
            status: Some(TransactionSignatureStatus {
                signature: String::new(),
                slot: Some(slot),
                confirmation_status: Some(Commitment::Confirmed),
                err: None,
            }),
            ..Self::default()
        }
    }

    fn log(&self) -> Log {
        self.log.lock().expect("log").clone()
    }

    /// The single transaction handed to `send`, decoded.
    fn sent_transaction(&self) -> VersionedTransaction {
        let log = self.log();
        assert_eq!(log.sent.len(), 1, "expected exactly one submission");
        decode(&base64_bytes(&Value::String(log.sent[0].clone())))
    }

    fn simulated_transaction(&self) -> VersionedTransaction {
        self.simulated_transaction_at(self.log().simulated.len() - 1)
    }

    fn simulated_transaction_at(&self, index: usize) -> VersionedTransaction {
        let log = self.log();
        decode(&base64_bytes(&Value::String(log.simulated[index].clone())))
    }
}

#[async_trait::async_trait]
impl TransactionTransport for FakeTransport {
    async fn latest_blockhash(
        &self,
        _options: TransactionRequestContext,
    ) -> Result<LatestBlockhashResult, TransactionError> {
        self.log.lock().expect("log").blockhash_calls += 1;
        Ok(LatestBlockhashResult {
            blockhash: self.blockhash.clone(),
            context_slot: 42,
            last_valid_block_height: self.last_valid_block_height,
        })
    }

    async fn fee(
        &self,
        message: &str,
        _options: TransactionRequestContext,
    ) -> Result<TransactionFeeResult, TransactionError> {
        self.log
            .lock()
            .expect("log")
            .fee_messages
            .push(message.to_string());
        Ok(TransactionFeeResult {
            fee_lamports: self.fee_lamports,
            context_slot: 42,
        })
    }

    async fn simulate(
        &self,
        transaction: &str,
        _options: TransactionSimulationOptions,
    ) -> Result<TransactionSimulationResult, TransactionError> {
        self.log
            .lock()
            .expect("log")
            .simulated
            .push(transaction.to_string());
        if self.simulate_fails {
            return Err(TransactionError::InvalidResponse(
                "relay unavailable".into(),
            ));
        }
        Ok(TransactionSimulationResult {
            context_slot: 42,
            err: self.simulation_error.clone(),
            logs: Some(vec!["Program log: hello".to_string()]),
            units_consumed: self.units_consumed,
            loaded_accounts_data_size: self.loaded_accounts_data_size,
            accounts: None,
        })
    }

    async fn send(
        &self,
        transaction: &str,
        _options: TransactionSendOptions,
    ) -> Result<TransactionSendResult, TransactionError> {
        self.log
            .lock()
            .expect("log")
            .sent
            .push(transaction.to_string());
        match &self.send {
            SendBehaviour::Accept => {
                let decoded = decode(&base64_bytes(&Value::String(transaction.to_string())));
                Ok(TransactionSendResult {
                    signature: decoded.signatures[0].to_string(),
                })
            }
            SendBehaviour::AcceptAs(signature) => Ok(TransactionSendResult {
                signature: signature.clone(),
            }),
            SendBehaviour::Reject {
                submission_state,
                signature,
            } => Err(TransactionError::from(TransactionTransportError {
                status: 504,
                code: "upstream_timeout".to_string(),
                message: "Submission outcome is unknown".to_string(),
                retryable: false,
                request_id: Some("req-1".to_string()),
                submission_state: *submission_state,
                signature: signature.clone(),
                details: None,
            })),
        }
    }

    async fn signature_status(
        &self,
        signature: &str,
        _options: SignatureStatusOptions,
    ) -> Result<Option<TransactionSignatureStatus>, TransactionError> {
        self.log.lock().expect("log").status_calls += 1;
        tokio::time::sleep(self.status_delay).await;
        Ok(self
            .status
            .clone()
            .map(|status| TransactionSignatureStatus {
                signature: signature.to_string(),
                ..status
            }))
    }

    async fn block_height(
        &self,
        _options: TransactionRequestContext,
    ) -> Result<u64, TransactionError> {
        Ok(self.block_height)
    }

    async fn transaction(
        &self,
        _signature: &str,
        _options: TransactionInspectOptions,
    ) -> Result<Option<ConfirmedTransaction>, TransactionError> {
        Ok(None)
    }

    async fn signatures(
        &self,
        _address: &str,
        _options: SignaturePageOptions,
    ) -> Result<Vec<SignaturePageEntry>, TransactionError> {
        Ok(Vec::new())
    }
}

/// Signer that fails the test if anything asks it for a signature.
struct SignerSpy(Address);

impl Signer for SignerSpy {
    fn try_pubkey(&self) -> Result<Address, SignerError> {
        Ok(self.0)
    }

    fn try_sign_message(&self, _message: &[u8]) -> Result<Signature, SignerError> {
        panic!("a signer was reached: this code path must never sign");
    }

    fn is_interactive(&self) -> bool {
        false
    }
}

fn fast_config(transport: Arc<FakeTransport>) -> SolanaAdapterConfig {
    SolanaAdapterConfig {
        transport: Some(transport),
        confirmation_timeout: Duration::from_millis(30),
        poll_interval: Duration::from_millis(1),
        ..SolanaAdapterConfig::default()
    }
}

fn v1_send_options(resources: TransactionResourceOptions) -> SendOptions {
    SendOptions {
        transaction_version: Some(TransactionVersion::V1),
        resources,
        ..SendOptions::default()
    }
}

/// The four V1 budget fields a final message must carry.
fn full_v1_resources() -> TransactionResourceOptions {
    TransactionResourceOptions {
        compute_unit_limit: Some(20_000),
        loaded_accounts_data_size_limit: Some(64 * 1024),
        heap_size: Some(64 * 1024),
        priority_fee_lamports: Some(5_000),
        compute_unit_price_micro_lamports: None,
    }
}

fn outcome(error: WalletError) -> TransactionFailureOutcome {
    error
        .outcome()
        .cloned()
        .unwrap_or_else(|| panic!("adapter failures carry a structured outcome"))
}

fn v1_config(transaction: &VersionedTransaction) -> v1::TransactionConfig {
    match &transaction.message {
        VersionedMessage::V1(message) => message.config,
        other => panic!("expected a V1 message, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Cross-codec conformance against the @solana/kit corpus
// ---------------------------------------------------------------------------

#[test]
fn fixture_payloads_round_trip_through_the_upstream_codec() {
    let corpus = load("transactions.json");
    let expectations = load("rust-codec.json");

    for name in FIXTURE_NAMES {
        let fixture = &corpus["fixtures"][name];
        let expected = &expectations["fixtures"][name];
        let wire = base64_bytes(&fixture["base64"]);
        assert_eq!(
            wire.len(),
            expected["wireBytes"].as_u64().unwrap() as usize,
            "{name}: wire length"
        );

        let transaction = decode(&wire);
        let message = &transaction.message;

        assert_eq!(
            transaction.signatures.len(),
            expected["signatureCount"].as_u64().unwrap() as usize,
            "{name}: signature count"
        );
        assert_eq!(
            transaction.signatures[0].to_string(),
            fixture["firstSignature"].as_str().unwrap(),
            "{name}: first signature"
        );

        let header = &expected["header"];
        assert_eq!(
            u64::from(message.header().num_required_signatures),
            header["numRequiredSignatures"].as_u64().unwrap(),
            "{name}: required signatures"
        );
        assert_eq!(
            u64::from(message.header().num_readonly_signed_accounts),
            header["numReadonlySignedAccounts"].as_u64().unwrap(),
            "{name}: readonly signed"
        );
        assert_eq!(
            u64::from(message.header().num_readonly_unsigned_accounts),
            header["numReadonlyUnsignedAccounts"].as_u64().unwrap(),
            "{name}: readonly unsigned"
        );

        let keys: Vec<String> = message
            .static_account_keys()
            .iter()
            .map(ToString::to_string)
            .collect();
        let expected_keys: Vec<String> = expected["accountKeys"]
            .as_array()
            .unwrap()
            .iter()
            .map(|key| key.as_str().unwrap().to_string())
            .collect();
        assert_eq!(keys, expected_keys, "{name}: canonical account ordering");
        assert_eq!(
            message.recent_blockhash().to_string(),
            expectations["lifetimeSpecifier"].as_str().unwrap(),
            "{name}: lifetime specifier"
        );

        let instructions = message.instructions();
        let expected_instructions = expected["instructions"].as_array().unwrap();
        assert_eq!(
            instructions.len(),
            expected_instructions.len(),
            "{name}: instruction count"
        );
        for (instruction, expected) in instructions.iter().zip(expected_instructions) {
            assert_eq!(
                u64::from(instruction.program_id_index),
                expected["programIdIndex"].as_u64().unwrap(),
                "{name}: program id index"
            );
            let accounts: Vec<u64> = instruction.accounts.iter().map(|i| u64::from(*i)).collect();
            let expected_accounts: Vec<u64> = expected["accounts"]
                .as_array()
                .unwrap()
                .iter()
                .map(|index| index.as_u64().unwrap())
                .collect();
            assert_eq!(accounts, expected_accounts, "{name}: instruction accounts");
            assert_eq!(
                instruction.data,
                base64_bytes(&expected["dataBase64"]),
                "{name}: instruction data"
            );
        }

        if let Some(expected_config) = expected.get("config") {
            let config = v1_config(&transaction);
            assert_eq!(
                config.priority_fee,
                expected_config["priorityFee"].as_u64(),
                "{name}: priority fee"
            );
            assert_eq!(
                config.compute_unit_limit.map(u64::from),
                expected_config["computeUnitLimit"].as_u64(),
                "{name}: compute unit limit"
            );
            assert_eq!(
                config.loaded_accounts_data_size_limit.map(u64::from),
                expected_config["loadedAccountsDataSizeLimit"].as_u64(),
                "{name}: loaded accounts data size"
            );
            assert_eq!(
                config.heap_size.map(u64::from),
                expected_config["heapSize"].as_u64(),
                "{name}: heap size"
            );
        }

        assert_eq!(
            encode(&transaction),
            wire,
            "{name}: re-encoded bytes differ"
        );
    }
}

#[test]
fn recompiling_the_fixtures_reproduces_the_kit_bytes() {
    let corpus = load("transactions.json");
    let payer = fixture_keypair(1);
    let cosigner = fixture_keypair(2);
    let blockhash: Hash = FIXTURE_BLOCKHASH.parse().expect("fixture blockhash");

    let cases: Vec<(&str, VersionedMessage, Vec<&Keypair>)> = vec![
        (
            "legacy",
            VersionedMessage::Legacy(LegacyMessage::new_with_blockhash(
                &[memo_instruction(vec![1, 2, 3], &[])],
                Some(&payer.pubkey()),
                &blockhash,
            )),
            vec![&payer],
        ),
        (
            "v0",
            VersionedMessage::V0(
                v0::Message::try_compile(
                    &payer.pubkey(),
                    &[memo_instruction(vec![1, 2, 3], &[])],
                    &[],
                    blockhash,
                )
                .expect("v0 compiles"),
            ),
            vec![&payer],
        ),
        (
            "v1",
            VersionedMessage::V1(
                v1::Message::try_compile_with_config(
                    &payer.pubkey(),
                    &[memo_instruction(vec![1, 2, 3], &[])],
                    blockhash,
                    v1::TransactionConfig::empty(),
                )
                .expect("v1 compiles"),
            ),
            vec![&payer],
        ),
        (
            "v1_oversize",
            VersionedMessage::V1(
                v1::Message::try_compile_with_config(
                    &payer.pubkey(),
                    &[memo_instruction(vec![7; 1400], &[])],
                    blockhash,
                    v1::TransactionConfig::empty(),
                )
                .expect("v1 compiles"),
            ),
            vec![&payer],
        ),
        (
            "v1_two_signatures",
            VersionedMessage::V1(
                v1::Message::try_compile_with_config(
                    &payer.pubkey(),
                    &[memo_instruction(
                        vec![9],
                        &[payer.pubkey(), cosigner.pubkey()],
                    )],
                    blockhash,
                    v1::TransactionConfig::empty(),
                )
                .expect("v1 compiles"),
            ),
            vec![&payer, &cosigner],
        ),
    ];

    for (name, message, signers) in cases {
        let transaction =
            VersionedTransaction::try_new(message, &signers).expect("signing succeeds");
        assert_eq!(
            encode(&transaction),
            base64_bytes(&corpus["fixtures"][name]["base64"]),
            "{name}: recompiled bytes differ from @solana/kit's"
        );
    }
}

// ---------------------------------------------------------------------------
// Adapter behaviour
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_send_returns_the_signature_and_slot_and_submits_once() {
    let transport = Arc::new(FakeTransport::confirmed_at(777));
    let payer = Arc::new(fixture_keypair(1));
    let adapter = SolanaWalletAdapter::with_config(payer.clone(), fast_config(transport.clone()));

    let result = adapter
        .sign_and_send(
            &[built_memo(b"hello".to_vec(), &[])],
            &v1_send_options(full_v1_resources()),
            &WalletExecutionContext::default(),
        )
        .await
        .expect("send succeeds");

    let sent = transport.sent_transaction();
    assert_eq!(result.signature, sent.signatures[0].to_string());
    assert_eq!(result.slot, Some(777));
    assert_eq!(transport.log().sent.len(), 1);
    assert_eq!(
        adapter.public_key(),
        payer.pubkey().to_string(),
        "the fee payer is the adapter's public key"
    );
    assert_eq!(
        adapter.supported_transaction_versions(),
        Some(
            [
                TransactionVersion::Legacy,
                TransactionVersion::V0,
                TransactionVersion::V1
            ]
            .as_slice()
        )
    );
}

#[tokio::test]
async fn canonical_account_ordering_and_instruction_bytes_survive_compilation() {
    let transport = Arc::new(FakeTransport::confirmed_at(1));
    let payer = Arc::new(fixture_keypair(1));
    let cosigner = Arc::new(fixture_keypair(2));
    let writable = Pubkey::new_from_array([9; 32]);
    let readonly = Pubkey::new_from_array([8; 32]);
    let adapter = SolanaWalletAdapter::with_config(payer.clone(), fast_config(transport.clone()))
        .with_signer(cosigner.clone());

    let instruction = BuiltInstruction {
        program_id: MEMO_PROGRAM.parse().unwrap(),
        accounts: vec![
            BuiltAccountMeta {
                pubkey: readonly,
                is_signer: false,
                is_writable: false,
            },
            BuiltAccountMeta {
                pubkey: writable,
                is_signer: false,
                is_writable: true,
            },
            BuiltAccountMeta {
                pubkey: Pubkey::new_from_array(cosigner.pubkey().to_bytes()),
                is_signer: true,
                is_writable: true,
            },
        ],
        data: vec![0xde, 0xad, 0xbe, 0xef],
    };

    adapter
        .sign_and_send(
            &[instruction],
            &v1_send_options(full_v1_resources()),
            &WalletExecutionContext::default(),
        )
        .await
        .expect("send succeeds");

    let sent = transport.sent_transaction();
    let keys: Vec<String> = sent
        .message
        .static_account_keys()
        .iter()
        .map(ToString::to_string)
        .collect();
    // Four classes in order — writable signers (fee payer leading), readonly
    // signers, writable non-signers, readonly non-signers — and inside a
    // class, address order. The memo program id is a readonly non-signer, so
    // it sorts against the readonly account rather than trailing the list.
    assert_eq!(
        keys,
        vec![
            payer.pubkey().to_string(),
            cosigner.pubkey().to_string(),
            writable.to_string(),
            MEMO_PROGRAM.to_string(),
            readonly.to_string(),
        ]
    );
    assert_eq!(sent.message.header().num_required_signatures, 2);
    assert_eq!(sent.message.header().num_readonly_signed_accounts, 0);
    assert_eq!(sent.message.header().num_readonly_unsigned_accounts, 2);
    assert_eq!(sent.signatures.len(), 2);

    let compiled = &sent.message.instructions()[0];
    assert_eq!(usize::from(compiled.program_id_index), 3);
    // The instruction keeps the caller's account order, by index.
    assert_eq!(compiled.accounts, vec![4, 2, 1]);
    assert_eq!(compiled.data, vec![0xde, 0xad, 0xbe, 0xef]);
    assert_eq!(
        adapter.signer_addresses(),
        vec![payer.pubkey().to_string(), cosigner.pubkey().to_string()]
    );
}

#[tokio::test]
async fn multiple_signers_sign_in_message_order() {
    let transport = Arc::new(FakeTransport::confirmed_at(1));
    let payer = Arc::new(fixture_keypair(1));
    let cosigner = Arc::new(fixture_keypair(2));
    let adapter = SolanaWalletAdapter::with_config(payer.clone(), fast_config(transport.clone()))
        .with_signer(cosigner.clone());

    adapter
        .sign_and_send(
            &[built_memo(
                vec![9],
                &[
                    Pubkey::new_from_array(payer.pubkey().to_bytes()),
                    Pubkey::new_from_array(cosigner.pubkey().to_bytes()),
                ],
            )],
            &v1_send_options(full_v1_resources()),
            &WalletExecutionContext::default(),
        )
        .await
        .expect("send succeeds");

    let sent = transport.sent_transaction();
    let signed_bytes = sent.message.serialize();
    assert_eq!(sent.signatures.len(), 2);
    // Each signature sits at its signer's index and verifies against the
    // signed message bytes.
    assert_eq!(sent.signatures[0], payer.sign_message(&signed_bytes));
    assert_eq!(sent.signatures[1], cosigner.sign_message(&signed_bytes));
    assert_ne!(sent.signatures[0], sent.signatures[1]);
}

#[tokio::test]
async fn a_missing_signer_is_refused_before_signing() {
    let transport = Arc::new(FakeTransport::confirmed_at(1));
    let stranger = Pubkey::new_from_array([5; 32]);
    let adapter = SolanaWalletAdapter::with_config(
        Arc::new(fixture_keypair(1)),
        fast_config(transport.clone()),
    );

    let error = adapter
        .sign_and_send(
            &[built_memo(vec![1], &[stranger])],
            &v1_send_options(full_v1_resources()),
            &WalletExecutionContext::default(),
        )
        .await
        .expect_err("a transaction we cannot sign is refused");

    assert!(matches!(
        outcome(error),
        TransactionFailureOutcome::NotSubmitted {
            phase: FailurePhase::Build,
            ref message,
        } if message.contains(&stranger.to_string())
    ));
    assert!(transport.log().sent.is_empty());
}

#[tokio::test]
async fn v1_priority_fee_keeps_full_u64_precision() {
    let transport = Arc::new(FakeTransport::confirmed_at(1));
    let adapter = SolanaWalletAdapter::with_config(
        Arc::new(fixture_keypair(1)),
        fast_config(transport.clone()),
    );

    adapter
        .sign_and_send(
            &[built_memo(vec![1], &[])],
            &v1_send_options(TransactionResourceOptions {
                priority_fee_lamports: Some(u64::MAX),
                ..full_v1_resources()
            }),
            &WalletExecutionContext::default(),
        )
        .await
        .expect("send succeeds");

    assert_eq!(
        v1_config(&transport.sent_transaction()).priority_fee,
        Some(u64::MAX),
        "a u64 fee must reach the wire unrounded"
    );
}

#[tokio::test]
async fn legacy_resources_become_canonical_compute_budget_instructions() {
    let transport = Arc::new(FakeTransport::confirmed_at(1));
    let adapter = SolanaWalletAdapter::with_config(
        Arc::new(fixture_keypair(1)),
        fast_config(transport.clone()),
    );

    adapter
        .sign_and_send(
            &[built_memo(vec![1], &[])],
            &SendOptions {
                transaction_version: Some(TransactionVersion::V0),
                resources: TransactionResourceOptions {
                    compute_unit_limit: Some(20_000),
                    loaded_accounts_data_size_limit: Some(64 * 1024),
                    heap_size: Some(64 * 1024),
                    priority_fee_lamports: None,
                    compute_unit_price_micro_lamports: Some(u64::MAX),
                },
                ..SendOptions::default()
            },
            &WalletExecutionContext::default(),
        )
        .await
        .expect("send succeeds");

    let sent = transport.sent_transaction();
    assert!(matches!(sent.message, VersionedMessage::V0(_)));
    let instructions = sent.message.instructions();
    assert_eq!(
        instructions.len(),
        4 + 1,
        "four budget instructions, then ours"
    );
    // ComputeBudget discriminants: 1 heap frame, 2 unit limit, 3 unit price,
    // 4 loaded accounts data size; values little-endian.
    let mut expected_limit = vec![2u8];
    expected_limit.extend_from_slice(&20_000u32.to_le_bytes());
    let mut expected_price = vec![3u8];
    expected_price.extend_from_slice(&u64::MAX.to_le_bytes());
    let mut expected_heap = vec![1u8];
    expected_heap.extend_from_slice(&(64u32 * 1024).to_le_bytes());
    let mut expected_loaded = vec![4u8];
    expected_loaded.extend_from_slice(&(64u32 * 1024).to_le_bytes());
    assert_eq!(instructions[0].data, expected_limit);
    assert_eq!(instructions[1].data, expected_price);
    assert_eq!(instructions[2].data, expected_heap);
    assert_eq!(instructions[3].data, expected_loaded);
    assert_eq!(instructions[4].data, vec![1]);
}

#[tokio::test]
async fn a_v1_message_carries_no_compute_budget_instructions() {
    let transport = Arc::new(FakeTransport::confirmed_at(1));
    let adapter = SolanaWalletAdapter::with_config(
        Arc::new(fixture_keypair(1)),
        fast_config(transport.clone()),
    );

    adapter
        .sign_and_send(
            &[built_memo(vec![1], &[])],
            &v1_send_options(full_v1_resources()),
            &WalletExecutionContext::default(),
        )
        .await
        .expect("send succeeds");

    let sent = transport.sent_transaction();
    assert_eq!(sent.message.instructions().len(), 1);
    assert_eq!(
        v1_config(&sent),
        v1::TransactionConfig::empty()
            .with_compute_unit_limit(20_000)
            .with_loaded_accounts_data_size_limit(64 * 1024)
            .with_heap_size(64 * 1024)
            .with_priority_fee(5_000),
        "the V1 budget travels in the message config"
    );
}

#[tokio::test]
async fn inspection_uses_a_provisional_config_and_never_reaches_a_signer() {
    let transport = Arc::new(FakeTransport::default());
    let spy = Arc::new(SignerSpy(fixture_keypair(1).pubkey()));
    let adapter = SolanaWalletAdapter::with_config(spy, fast_config(transport.clone()));

    let inspection = adapter
        .inspect_transaction(
            &[built_memo(b"inspect".to_vec(), &[])],
            &TransactionInspectionOptions {
                transaction_version: Some(TransactionVersion::V1),
                resources: TransactionResourceOptions {
                    priority_fee_lamports: Some(5_000),
                    ..TransactionResourceOptions::default()
                },
                ..TransactionInspectionOptions::default()
            },
            &WalletExecutionContext::default(),
        )
        .await
        .expect("inspection succeeds without a signature");

    assert_eq!(inspection.fee_lamports, Some(5_000));
    assert_eq!(inspection.compute_units_consumed, Some(1_234));
    assert_eq!(inspection.loaded_accounts_data_size, Some(4_096));
    assert_eq!(inspection.context_slot, Some(42));
    assert_eq!(inspection.error, None);

    let log = transport.log();
    assert!(log.sent.is_empty(), "inspection must never broadcast");
    assert_eq!(log.fee_messages.len(), 1);

    let inspected = transport.simulated_transaction();
    assert_eq!(
        inspected.signatures,
        vec![Signature::default()],
        "inspection carries placeholder signatures only"
    );
    // The provisional budget is what makes the measurement possible: under
    // SIMD-0385 an omitted limit requests zero.
    let config = v1_config(&inspected);
    assert_eq!(
        config.compute_unit_limit,
        Some(PROVISIONAL_COMPUTE_UNIT_LIMIT)
    );
    assert_eq!(
        config.loaded_accounts_data_size_limit,
        Some(PROVISIONAL_LOADED_ACCOUNTS_DATA_SIZE)
    );
    assert_eq!(config.priority_fee, Some(5_000));
    assert_eq!(
        log.fee_messages[0],
        base64::engine::general_purpose::STANDARD.encode(inspected.message.serialize()),
        "the fee is quoted for exactly the message that was simulated"
    );
}

#[tokio::test]
async fn the_v1_structural_caps_are_enforced_before_signing() {
    let transport = Arc::new(FakeTransport::confirmed_at(1));
    // 13 owned signers: enough to overshoot V1's 12-signature cap without
    // the send failing for a missing key instead.
    let signers: Vec<Arc<Keypair>> = (1..=13)
        .map(|seed| Arc::new(fixture_keypair(seed)))
        .collect();
    let adapter = signers[1..].iter().fold(
        SolanaWalletAdapter::with_config(signers[0].clone(), fast_config(transport.clone())),
        |adapter, signer| adapter.with_signer(signer.clone()),
    );

    let too_many_signatures = built_memo(
        vec![1],
        &signers
            .iter()
            .map(|signer| Pubkey::new_from_array(signer.pubkey().to_bytes()))
            .collect::<Vec<_>>(),
    );
    let too_many_accounts = BuiltInstruction {
        program_id: MEMO_PROGRAM.parse().unwrap(),
        accounts: (0..64)
            .map(|index| BuiltAccountMeta {
                pubkey: Pubkey::new_from_array([index as u8; 32]),
                is_signer: false,
                is_writable: false,
            })
            .collect(),
        data: vec![1],
    };
    let too_many_instructions: Vec<BuiltInstruction> =
        (0..65).map(|index| built_memo(vec![index], &[])).collect();

    for (label, instructions) in [
        ("13 signatures", vec![too_many_signatures]),
        ("66 accounts", vec![too_many_accounts]),
        ("65 instructions", too_many_instructions),
    ] {
        let error = adapter
            .sign_and_send(
                &instructions,
                &v1_send_options(full_v1_resources()),
                &WalletExecutionContext::default(),
            )
            .await
            .unwrap_err();
        let outcome = outcome(error);
        assert_eq!(outcome.phase(), FailurePhase::Build, "{label}");
        assert!(
            outcome.message().contains("Invalid V1 message"),
            "{label}: {}",
            outcome.message()
        );
        assert!(transport.log().sent.is_empty(), "{label}: submitted anyway");
    }
}

#[tokio::test]
async fn explicit_v1_budgets_are_forwarded_verbatim() {
    // Metrics that would derive very different numbers if they were consulted.
    let transport = Arc::new(FakeTransport {
        units_consumed: Some(999_999),
        loaded_accounts_data_size: Some(8 * 1024 * 1024),
        ..FakeTransport::confirmed_at(1)
    });
    let adapter = SolanaWalletAdapter::with_config(
        Arc::new(fixture_keypair(1)),
        fast_config(transport.clone()),
    );

    adapter
        .sign_and_send(
            &[built_memo(vec![1], &[])],
            &v1_send_options(full_v1_resources()),
            &WalletExecutionContext::default(),
        )
        .await
        .expect("send succeeds");

    let sent = transport.sent_transaction();
    let config = v1_config(&sent);
    assert_eq!(config.compute_unit_limit, Some(20_000));
    assert_eq!(config.loaded_accounts_data_size_limit, Some(64 * 1024));
    let log = transport.log();
    assert_eq!(
        log.simulated.len(),
        1,
        "nothing to estimate, so no provisional probe: only the preflight"
    );
    assert_eq!(
        log.simulated[0], log.sent[0],
        "the preflighted bytes are the submitted bytes: one config, compiled once"
    );
}

#[tokio::test]
async fn absent_v1_budgets_are_estimated_from_the_simulation_metrics() {
    let transport = Arc::new(FakeTransport {
        units_consumed: Some(1_234),
        loaded_accounts_data_size: Some(4_096),
        ..FakeTransport::confirmed_at(1)
    });
    let adapter = SolanaWalletAdapter::with_config(
        Arc::new(fixture_keypair(1)),
        fast_config(transport.clone()),
    );

    adapter
        .sign_and_send(
            &[built_memo(vec![1], &[])],
            &v1_send_options(TransactionResourceOptions {
                compute_unit_limit: None,
                loaded_accounts_data_size_limit: None,
                heap_size: Some(64 * 1024),
                priority_fee_lamports: Some(5_000),
                compute_unit_price_micro_lamports: None,
            }),
            &WalletExecutionContext::default(),
        )
        .await
        .expect("an unbudgeted V1 send estimates its budget and lands");

    let log = transport.log();
    assert_eq!(log.simulated.len(), 2, "provisional probe, then preflight");

    // The probe declares the protocol maxima, so the metrics it returns are
    // real consumption rather than a budget-exceeded failure.
    let probe = v1_config(&transport.simulated_transaction_at(0));
    assert_eq!(
        probe.compute_unit_limit,
        Some(PROVISIONAL_COMPUTE_UNIT_LIMIT)
    );
    assert_eq!(
        probe.loaded_accounts_data_size_limit,
        Some(PROVISIONAL_LOADED_ACCOUNTS_DATA_SIZE)
    );
    assert_eq!(
        transport.log().sent.len(),
        1,
        "the probe is a simulation, never a second submission"
    );

    // 1234 units + 20% headroom, rounded up; 4096 bytes rounded up to one
    // 32 KiB page plus one page of headroom.
    let signed = v1_config(&transport.sent_transaction());
    assert_eq!(signed.compute_unit_limit, Some(1_481));
    assert_eq!(signed.loaded_accounts_data_size_limit, Some(65_536));
    assert_eq!(signed.heap_size, Some(64 * 1024));
    assert_eq!(signed.priority_fee, Some(5_000));
    assert_eq!(
        log.simulated[1], log.sent[0],
        "the derived config is what got preflighted and signed"
    );
}

#[tokio::test]
async fn an_explicit_v1_budget_survives_alongside_an_estimated_one() {
    let transport = Arc::new(FakeTransport::confirmed_at(1));
    let adapter = SolanaWalletAdapter::with_config(
        Arc::new(fixture_keypair(1)),
        fast_config(transport.clone()),
    );

    adapter
        .sign_and_send(
            &[built_memo(vec![1], &[])],
            &v1_send_options(TransactionResourceOptions {
                compute_unit_limit: Some(7_777),
                loaded_accounts_data_size_limit: None,
                ..full_v1_resources()
            }),
            &WalletExecutionContext::default(),
        )
        .await
        .expect("send succeeds");

    // The caller's compute limit travels into the probe untouched, and only
    // the omitted data budget is measured.
    let probe = v1_config(&transport.simulated_transaction_at(0));
    assert_eq!(probe.compute_unit_limit, Some(7_777));
    assert_eq!(
        probe.loaded_accounts_data_size_limit,
        Some(PROVISIONAL_LOADED_ACCOUNTS_DATA_SIZE)
    );
    let signed = v1_config(&transport.sent_transaction());
    assert_eq!(
        signed.compute_unit_limit,
        Some(7_777),
        "an explicit budget is never silently raised"
    );
    assert_eq!(signed.loaded_accounts_data_size_limit, Some(65_536));
}

#[tokio::test]
async fn a_metric_the_simulation_omits_requires_an_explicit_v1_budget() {
    for (units, loaded, option, metric) in [
        (None, Some(4_096), "computeUnitLimit", "unitsConsumed"),
        (
            Some(1_234),
            None,
            "loadedAccountsDataSizeLimit",
            "loadedAccountsDataSize",
        ),
    ] {
        let transport = Arc::new(FakeTransport {
            units_consumed: units,
            loaded_accounts_data_size: loaded,
            ..FakeTransport::confirmed_at(1)
        });
        let spy = Arc::new(SignerSpy(fixture_keypair(1).pubkey()));
        let adapter = SolanaWalletAdapter::with_config(spy, fast_config(transport.clone()));

        let error = adapter
            .sign_and_send(
                &[built_memo(vec![1], &[])],
                &v1_send_options(TransactionResourceOptions {
                    compute_unit_limit: None,
                    loaded_accounts_data_size_limit: None,
                    ..full_v1_resources()
                }),
                &WalletExecutionContext::default(),
            )
            .await
            .expect_err("an unmeasurable budget cannot be invented");

        let outcome = outcome(error);
        assert_eq!(outcome.phase(), FailurePhase::Build, "{option}");
        assert!(
            outcome.message().contains(option) && outcome.message().contains(metric),
            "the error names the option and the missing metric: {}",
            outcome.message()
        );
        assert!(
            transport.log().sent.is_empty(),
            "{option}: submitted anyway"
        );
    }
}

#[tokio::test]
async fn a_failed_estimation_simulation_never_submits() {
    let transport = Arc::new(FakeTransport {
        simulation_error: Some(json!({"InstructionError": [0, {"Custom": 6_002}]})),
        ..FakeTransport::confirmed_at(1)
    });
    let spy = Arc::new(SignerSpy(fixture_keypair(1).pubkey()));
    let adapter = SolanaWalletAdapter::with_config(spy, fast_config(transport.clone()));

    let error = adapter
        .sign_and_send(
            &[built_memo(vec![1], &[])],
            &v1_send_options(TransactionResourceOptions {
                compute_unit_limit: None,
                loaded_accounts_data_size_limit: None,
                ..full_v1_resources()
            }),
            &WalletExecutionContext::default(),
        )
        .await
        .expect_err("a transaction that fails at the maxima is not worth signing");

    let outcome = outcome(error);
    assert_eq!(outcome.phase(), FailurePhase::Send);
    assert!(transport.log().sent.is_empty());
}

#[tokio::test]
async fn the_v1_wire_limit_is_exact_at_4096_bytes() {
    let payer = Arc::new(fixture_keypair(1));
    let resources = full_v1_resources();

    // Measure the fixed overhead with a known payload, then aim exactly at the
    // ceiling: the encoding is linear in the instruction data length here.
    let probe = Arc::new(FakeTransport::confirmed_at(1));
    let adapter = SolanaWalletAdapter::with_config(payer.clone(), fast_config(probe.clone()));
    adapter
        .sign_and_send(
            &[built_memo(vec![0; 1_000], &[])],
            &v1_send_options(resources),
            &WalletExecutionContext::default(),
        )
        .await
        .expect("the probe send succeeds");
    let probe_len = base64_bytes(&Value::String(probe.log().sent[0].clone())).len();
    let exact = 1_000 + MAX_V1_TRANSACTION_BYTES - probe_len;

    let at_limit = Arc::new(FakeTransport::confirmed_at(1));
    let adapter = SolanaWalletAdapter::with_config(payer.clone(), fast_config(at_limit.clone()));
    adapter
        .sign_and_send(
            &[built_memo(vec![0; exact], &[])],
            &v1_send_options(resources),
            &WalletExecutionContext::default(),
        )
        .await
        .expect("exactly 4096 bytes is inside the V1 limit");
    assert_eq!(
        base64_bytes(&Value::String(at_limit.log().sent[0].clone())).len(),
        MAX_V1_TRANSACTION_BYTES
    );

    let over = Arc::new(FakeTransport::confirmed_at(1));
    let adapter = SolanaWalletAdapter::with_config(payer.clone(), fast_config(over.clone()));
    let error = adapter
        .sign_and_send(
            &[built_memo(vec![0; exact + 1], &[])],
            &v1_send_options(resources),
            &WalletExecutionContext::default(),
        )
        .await
        .expect_err("4097 bytes is over the V1 limit");
    assert!(outcome(error).message().contains("4097 bytes"));
    assert!(over.log().sent.is_empty());
}

#[tokio::test]
async fn the_legacy_wire_limit_still_applies_to_v0() {
    let transport = Arc::new(FakeTransport::confirmed_at(1));
    let adapter = SolanaWalletAdapter::with_config(
        Arc::new(fixture_keypair(1)),
        fast_config(transport.clone()),
    );

    // The payload the V1 corpus carries as a 1574-byte V1 transaction: it
    // clears V1's ceiling and busts the legacy one.
    let corpus = load("transactions.json");
    let oversize = base64_bytes(&corpus["fixtures"]["v1_oversize"]["base64"]);
    assert!(oversize.len() > MAX_LEGACY_TRANSACTION_BYTES);
    assert!(oversize.len() < MAX_V1_TRANSACTION_BYTES);
    let error = adapter
        .sign_and_send(
            &[built_memo(vec![7; 1_400], &[])],
            &SendOptions {
                transaction_version: Some(TransactionVersion::V0),
                ..SendOptions::default()
            },
            &WalletExecutionContext::default(),
        )
        .await
        .expect_err("a 1574-byte v0 transaction is over the limit");
    let outcome = outcome(error);
    assert!(
        outcome
            .message()
            .contains(&format!("{MAX_LEGACY_TRANSACTION_BYTES}-byte limit")),
        "{}",
        outcome.message()
    );
    assert!(transport.log().sent.is_empty());
}

#[tokio::test]
async fn a_failed_simulation_never_submits() {
    let transport = Arc::new(FakeTransport {
        simulation_error: Some(json!({"InstructionError": [0, {"Custom": 6_001}]})),
        ..FakeTransport::confirmed_at(1)
    });
    let adapter = SolanaWalletAdapter::with_config(
        Arc::new(fixture_keypair(1)),
        fast_config(transport.clone()),
    );

    let error = adapter
        .sign_and_send(
            &[built_memo(vec![1], &[])],
            &v1_send_options(full_v1_resources()),
            &WalletExecutionContext::default(),
        )
        .await
        .expect_err("preflight failure stops the send");

    let outcome = outcome(error);
    assert_eq!(outcome.phase(), FailurePhase::Send);
    assert_eq!(outcome.signature(), None);
    let log = transport.log();
    assert_eq!(log.simulated.len(), 1);
    assert!(log.sent.is_empty(), "a refused transaction is never sent");
}

#[tokio::test]
async fn a_confirmation_timeout_submits_once_and_keeps_the_signature() {
    let transport = Arc::new(FakeTransport {
        status: None,
        ..FakeTransport::default()
    });
    let adapter = SolanaWalletAdapter::with_config(
        Arc::new(fixture_keypair(1)),
        fast_config(transport.clone()),
    );

    let error = adapter
        .sign_and_send(
            &[built_memo(vec![1], &[])],
            &v1_send_options(full_v1_resources()),
            &WalletExecutionContext::default(),
        )
        .await
        .expect_err("an unconfirmed transaction is not a success");

    let expected = transport.sent_transaction().signatures[0].to_string();
    let outcome = outcome(error);
    assert!(matches!(
        outcome,
        TransactionFailureOutcome::SubmittedUnknown { .. }
    ));
    assert_eq!(outcome.signature(), Some(expected.as_str()));
    let log = transport.log();
    assert_eq!(log.sent.len(), 1, "exactly one submission, never a retry");
    assert!(log.status_calls >= 1);
}

#[tokio::test]
async fn an_expired_lifetime_reports_an_unknown_submission() {
    let transport = Arc::new(FakeTransport {
        status: None,
        last_valid_block_height: 100,
        block_height: 101,
        ..FakeTransport::default()
    });
    let adapter = SolanaWalletAdapter::with_config(
        Arc::new(fixture_keypair(1)),
        SolanaAdapterConfig {
            // Long timeout: the expiry, not the clock, must end the wait.
            confirmation_timeout: Duration::from_secs(30),
            ..fast_config(transport.clone())
        },
    );

    let error = adapter
        .sign_and_send(
            &[built_memo(vec![1], &[])],
            &v1_send_options(full_v1_resources()),
            &WalletExecutionContext::default(),
        )
        .await
        .expect_err("an expired lifetime is not a success");

    let outcome = outcome(error);
    assert!(matches!(
        outcome,
        TransactionFailureOutcome::SubmittedUnknown { .. }
    ));
    assert!(
        outcome.message().contains("expired"),
        "{}",
        outcome.message()
    );
    assert_eq!(transport.log().sent.len(), 1);
}

#[tokio::test]
async fn an_on_chain_failure_maps_to_chain_failed_with_the_program_code() {
    let transport = Arc::new(FakeTransport {
        status: Some(TransactionSignatureStatus {
            signature: String::new(),
            slot: Some(9_001),
            confirmation_status: Some(Commitment::Confirmed),
            err: Some(json!({"InstructionError": [0, {"Custom": 6_000}]})),
        }),
        ..FakeTransport::default()
    });
    let adapter = SolanaWalletAdapter::with_config(
        Arc::new(fixture_keypair(1)),
        fast_config(transport.clone()),
    );

    let error = adapter
        .sign_and_send(
            &[built_memo(vec![1], &[])],
            &v1_send_options(full_v1_resources()),
            &WalletExecutionContext::default(),
        )
        .await
        .expect_err("an on-chain failure is a failure");

    let outcome = outcome(error);
    assert_eq!(outcome.phase(), FailurePhase::Chain);
    assert_eq!(outcome.slot(), Some(9_001));
    assert_eq!(outcome.program_error().map(|error| error.code), Some(6_000));
    assert!(outcome.signature().is_some());
}

#[tokio::test]
async fn an_ambiguous_submission_keeps_the_derived_signature() {
    let transport = Arc::new(FakeTransport {
        send: SendBehaviour::Reject {
            submission_state: Some(SubmissionState::Unknown),
            signature: None,
        },
        ..FakeTransport::default()
    });
    let adapter = SolanaWalletAdapter::with_config(
        Arc::new(fixture_keypair(1)),
        fast_config(transport.clone()),
    );

    let error = adapter
        .sign_and_send(
            &[built_memo(vec![1], &[])],
            &v1_send_options(full_v1_resources()),
            &WalletExecutionContext::default(),
        )
        .await
        .expect_err("an unknown submission is not a success");

    let expected = transport.sent_transaction().signatures[0].to_string();
    let outcome = outcome(error);
    assert!(matches!(
        outcome,
        TransactionFailureOutcome::SubmittedUnknown { .. }
    ));
    assert_eq!(
        outcome.signature(),
        Some(expected.as_str()),
        "the locally derived signature survives a relay error with no signature"
    );
}

#[tokio::test]
async fn a_proven_non_dispatch_is_not_submitted() {
    let transport = Arc::new(FakeTransport {
        send: SendBehaviour::Reject {
            submission_state: Some(SubmissionState::NotSubmitted),
            signature: None,
        },
        ..FakeTransport::default()
    });
    let adapter = SolanaWalletAdapter::with_config(
        Arc::new(fixture_keypair(1)),
        fast_config(transport.clone()),
    );

    let error = adapter
        .sign_and_send(
            &[built_memo(vec![1], &[])],
            &v1_send_options(full_v1_resources()),
            &WalletExecutionContext::default(),
        )
        .await
        .expect_err("a rejected submission is a failure");

    let outcome = outcome(error);
    assert_eq!(outcome.phase(), FailurePhase::Send);
    assert_eq!(outcome.signature(), None);
}

#[tokio::test]
async fn the_context_transport_wins_over_the_configured_one() {
    let configured = Arc::new(FakeTransport::confirmed_at(1));
    let context_transport = Arc::new(FakeTransport::confirmed_at(2));
    let adapter = SolanaWalletAdapter::with_config(
        Arc::new(fixture_keypair(1)),
        fast_config(configured.clone()),
    );

    let result = adapter
        .sign_and_send(
            &[built_memo(vec![1], &[])],
            &v1_send_options(full_v1_resources()),
            &WalletExecutionContext::new(Some(context_transport.clone())),
        )
        .await
        .expect("send succeeds");

    assert_eq!(result.slot, Some(2), "the context transport confirmed it");
    assert_eq!(context_transport.log().sent.len(), 1);
    assert_eq!(
        configured.log().blockhash_calls,
        0,
        "the configured transport is the fallback, not a second destination"
    );
}

#[tokio::test]
async fn the_configured_transport_is_the_fallback() {
    let configured = Arc::new(FakeTransport::confirmed_at(3));
    let adapter = SolanaWalletAdapter::with_config(
        Arc::new(fixture_keypair(1)),
        fast_config(configured.clone()),
    );

    let result = adapter
        .sign_and_send(
            &[built_memo(vec![1], &[])],
            &v1_send_options(full_v1_resources()),
            &WalletExecutionContext::default(),
        )
        .await
        .expect("send succeeds through the configured relay");
    assert_eq!(result.slot, Some(3));
    assert_eq!(configured.log().sent.len(), 1);
}

#[tokio::test]
async fn no_transport_at_all_is_a_build_failure() {
    let adapter = SolanaWalletAdapter::new(Arc::new(fixture_keypair(1)));

    let error = adapter
        .sign_and_send(
            &[built_memo(vec![1], &[])],
            &v1_send_options(full_v1_resources()),
            &WalletExecutionContext::default(),
        )
        .await
        .expect_err("there is nowhere to send");
    assert_eq!(outcome(error).phase(), FailurePhase::Build);
}

#[tokio::test]
async fn caller_supplied_compute_budget_and_lookup_tables_are_refused() {
    let transport = Arc::new(FakeTransport::confirmed_at(1));
    let adapter = SolanaWalletAdapter::with_config(
        Arc::new(fixture_keypair(1)),
        fast_config(transport.clone()),
    );

    let compute_budget = BuiltInstruction {
        program_id: "ComputeBudget111111111111111111111111111111"
            .parse()
            .unwrap(),
        accounts: Vec::new(),
        data: vec![2, 0x20, 0x4e, 0, 0],
    };
    let error = adapter
        .sign_and_send(
            &[compute_budget],
            &v1_send_options(full_v1_resources()),
            &WalletExecutionContext::default(),
        )
        .await
        .expect_err("ComputeBudget instructions belong to the typed options");
    assert!(outcome(error).message().contains("ComputeBudget"));

    let mut extra = serde_json::Map::new();
    extra.insert(
        "addressLookupTables".to_string(),
        json!(["7Zb1bGi3pbZUeorUQrCLuJvQvJoWYFsGRe4uSNsWjqCG"]),
    );
    let error = adapter
        .sign_and_send(
            &[built_memo(vec![1], &[])],
            &SendOptions {
                transaction_version: Some(TransactionVersion::V1),
                resources: full_v1_resources(),
                extra,
                ..SendOptions::default()
            },
            &WalletExecutionContext::default(),
        )
        .await
        .expect_err("V1 has no lookup tables");
    assert!(outcome(error).message().contains("lookup table"));
    assert!(transport.log().sent.is_empty());
}

#[tokio::test]
async fn an_empty_instruction_list_is_refused() {
    let transport = Arc::new(FakeTransport::confirmed_at(1));
    let adapter = SolanaWalletAdapter::with_config(
        Arc::new(fixture_keypair(1)),
        fast_config(transport.clone()),
    );

    let error = adapter
        .sign_and_send(
            &[],
            &v1_send_options(full_v1_resources()),
            &WalletExecutionContext::default(),
        )
        .await
        .expect_err("an empty transaction is refused");
    assert_eq!(outcome(error).phase(), FailurePhase::Build);
}

#[tokio::test]
async fn a_version_bound_fee_option_is_refused_rather_than_converted() {
    let transport = Arc::new(FakeTransport::confirmed_at(1));
    let adapter = SolanaWalletAdapter::with_config(
        Arc::new(fixture_keypair(1)),
        fast_config(transport.clone()),
    );

    let error = adapter
        .sign_and_send(
            &[built_memo(vec![1], &[])],
            &v1_send_options(TransactionResourceOptions {
                priority_fee_lamports: None,
                compute_unit_price_micro_lamports: Some(1_000),
                ..full_v1_resources()
            }),
            &WalletExecutionContext::default(),
        )
        .await
        .expect_err("the legacy fee model is not converted into V1's");
    assert_eq!(outcome(error).phase(), FailurePhase::Build);
    assert!(transport.log().sent.is_empty());
}

// ---------------------------------------------------------------------------
// The effective version, not the contract default, binds the resources
// ---------------------------------------------------------------------------

/// Adapter whose configured default is V1, so an omitted `transaction_version`
/// still compiles a V1 message.
fn v1_default_adapter(transport: Arc<FakeTransport>) -> SolanaWalletAdapter {
    SolanaWalletAdapter::with_config(
        Arc::new(fixture_keypair(1)),
        SolanaAdapterConfig {
            default_version: TransactionVersion::V1,
            ..fast_config(transport)
        },
    )
}

#[tokio::test]
async fn a_v1_default_adapter_accepts_the_v1_fee_when_the_caller_omits_the_version() {
    let transport = Arc::new(FakeTransport::confirmed_at(5));
    let adapter = v1_default_adapter(transport.clone());

    adapter
        .sign_and_send(
            &[built_memo(vec![1], &[])],
            &SendOptions {
                transaction_version: None,
                resources: full_v1_resources(),
                ..SendOptions::default()
            },
            &WalletExecutionContext::default(),
        )
        .await
        .expect("the fee is legal for the version this adapter actually builds");

    let sent = transport.sent_transaction();
    assert_eq!(
        v1_config(&sent).priority_fee,
        Some(5_000),
        "the priority fee reaches the compiled V1 message"
    );
}

#[tokio::test]
async fn a_v1_default_adapter_refuses_the_v0_only_fee_when_the_caller_omits_the_version() {
    let transport = Arc::new(FakeTransport::confirmed_at(5));
    let adapter = v1_default_adapter(transport.clone());

    let error = adapter
        .sign_and_send(
            &[built_memo(vec![1], &[])],
            &SendOptions {
                transaction_version: None,
                resources: TransactionResourceOptions {
                    priority_fee_lamports: None,
                    compute_unit_price_micro_lamports: Some(1_000),
                    ..full_v1_resources()
                },
                ..SendOptions::default()
            },
            &WalletExecutionContext::default(),
        )
        .await
        .expect_err("a v0-only fee is refused, never dropped from the V1 message");

    assert_eq!(outcome(error).phase(), FailurePhase::Build);
    assert!(
        transport.log().sent.is_empty(),
        "a refused fee option never reaches a submission"
    );
}

#[tokio::test]
async fn inspection_binds_the_resources_to_the_effective_version_too() {
    let transport = Arc::new(FakeTransport::default());
    let adapter = SolanaWalletAdapter::with_config(
        Arc::new(SignerSpy(fixture_keypair(1).pubkey())),
        SolanaAdapterConfig {
            default_version: TransactionVersion::V1,
            ..fast_config(transport.clone())
        },
    );

    let error = adapter
        .inspect_transaction(
            &[built_memo(vec![1], &[])],
            &TransactionInspectionOptions {
                transaction_version: None,
                resources: TransactionResourceOptions {
                    compute_unit_price_micro_lamports: Some(1_000),
                    ..TransactionResourceOptions::default()
                },
                ..TransactionInspectionOptions::default()
            },
            &WalletExecutionContext::default(),
        )
        .await
        .expect_err("a v0-only fee is refused, never dropped from the inspected V1 message");

    assert_eq!(outcome(error).phase(), FailurePhase::Build);
    assert!(transport.log().simulated.is_empty());
}

// ---------------------------------------------------------------------------
// The confirmation deadline and the authoritative signature
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_stalled_status_request_still_ends_at_the_confirmation_deadline() {
    let transport = Arc::new(FakeTransport {
        status: None,
        // A relay that accepts the poll and never answers it: the adapter's
        // own deadline, not the request, has to end the wait.
        status_delay: Duration::from_secs(30),
        ..FakeTransport::default()
    });
    let adapter = SolanaWalletAdapter::with_config(
        Arc::new(fixture_keypair(1)),
        fast_config(transport.clone()),
    );

    let error = tokio::time::timeout(
        Duration::from_secs(2),
        adapter.sign_and_send(
            &[built_memo(vec![1], &[])],
            &v1_send_options(full_v1_resources()),
            &WalletExecutionContext::default(),
        ),
    )
    .await
    .expect("the 30ms confirmation timeout bounds the in-flight status request")
    .expect_err("an unconfirmed transaction is not a success");

    let expected = transport.sent_transaction().signatures[0].to_string();
    let outcome = outcome(error);
    assert!(matches!(
        outcome,
        TransactionFailureOutcome::SubmittedUnknown { .. }
    ));
    assert_eq!(outcome.signature(), Some(expected.as_str()));
    assert_eq!(
        transport.log().sent.len(),
        1,
        "exactly one submission, never a retry"
    );
}

#[tokio::test]
async fn a_relay_signature_that_differs_from_the_signed_one_is_never_polled_for() {
    let echoed = Signature::from([7u8; 64]).to_string();
    let transport = Arc::new(FakeTransport {
        send: SendBehaviour::AcceptAs(echoed.clone()),
        // Any signature would confirm here: adopting the relay's would
        // report another transaction's status as this one's.
        ..FakeTransport::confirmed_at(31)
    });
    let adapter = SolanaWalletAdapter::with_config(
        Arc::new(fixture_keypair(1)),
        fast_config(transport.clone()),
    );

    let error = adapter
        .sign_and_send(
            &[built_memo(vec![1], &[])],
            &v1_send_options(full_v1_resources()),
            &WalletExecutionContext::default(),
        )
        .await
        .expect_err("a signature the adapter did not sign is not a confirmation");

    let derived = transport.sent_transaction().signatures[0].to_string();
    assert_ne!(derived, echoed);
    let outcome = outcome(error);
    assert!(matches!(
        outcome,
        TransactionFailureOutcome::SubmittedUnknown { .. }
    ));
    assert_eq!(
        outcome.signature(),
        Some(derived.as_str()),
        "reconciliation keeps the locally derived signature"
    );
    assert!(outcome.message().contains(&echoed), "{}", outcome.message());
    assert_eq!(
        transport.log().status_calls,
        0,
        "the adapter never polls a signature it did not sign"
    );
    assert_eq!(transport.log().sent.len(), 1);
}

// ---------------------------------------------------------------------------
// Source compatibility for adapters written before the capability existed
// ---------------------------------------------------------------------------

/// Implements only the two methods the trait has always required. This
/// failing to compile is the regression.
struct PreCapabilityAdapter;

#[async_trait::async_trait]
impl WalletAdapter for PreCapabilityAdapter {
    fn public_key(&self) -> String {
        "AKnL4NNf3DGWZJS6cPknBuEGnVsV4A4m5tgebLHaRSZ9".to_string()
    }

    async fn sign_and_send(
        &self,
        _instructions: &[BuiltInstruction],
        _options: &SendOptions,
        _context: &WalletExecutionContext,
    ) -> Result<SendResult, WalletError> {
        Ok(SendResult {
            signature: "sig".to_string(),
            slot: None,
        })
    }
}

#[tokio::test]
async fn a_pre_capability_adapter_still_compiles_and_declares_nothing() {
    let adapter = PreCapabilityAdapter;
    assert_eq!(adapter.supported_transaction_versions(), None);
    // Undeclared means unknown: ordinary sends still work, V1 is refused.
    assert!(adapter
        .validate_transaction_options(None, &TransactionResourceOptions::default())
        .is_ok());
    assert!(adapter
        .validate_transaction_options(
            Some(TransactionVersion::V1),
            &TransactionResourceOptions::default()
        )
        .is_err());
    assert!(adapter
        .inspect_transaction(
            &[],
            &TransactionInspectionOptions::default(),
            &WalletExecutionContext::default(),
        )
        .await
        .is_err());
    let _ = ConfirmationLevel::Confirmed;
}
