//! Generated instruction -> public adapter -> recording relay -> upstream codec.
extern crate arete_a4_sdk as arete_sdk;

#[allow(dead_code)]
#[path = "../../../examples/ore-rust/src/generated/ore/programs.rs"]
mod programs;
#[allow(dead_code)]
#[path = "../../../examples/ore-rust/src/generated/ore/types.rs"]
mod types;

use std::sync::{Arc, Mutex};

use arete_sdk::adapters::solana::{SolanaAdapterConfig, SolanaWalletAdapter};
use arete_sdk::transactions::*;
use arete_sdk::wallet::{
    SendOptions, TransactionResourceOptions, TransactionVersion, WalletAdapter,
    WalletExecutionContext,
};
use base64::Engine as _;
use solana_keypair::Keypair;
use solana_message::VersionedMessage;
use solana_signer::Signer;
use solana_transaction::versioned::VersionedTransaction;

#[derive(Default)]
struct RecordingRelay(Mutex<Vec<String>>);

fn decode(wire: &str) -> VersionedTransaction {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(wire)
        .unwrap();
    wincode::deserialize_exact(&bytes).unwrap()
}

#[async_trait::async_trait]
impl TransactionTransport for RecordingRelay {
    async fn latest_blockhash(
        &self,
        _: TransactionRequestContext,
    ) -> Result<LatestBlockhashResult, TransactionError> {
        Ok(LatestBlockhashResult {
            blockhash: "11111111111111111111111111111111".into(),
            context_slot: 1,
            last_valid_block_height: 100,
        })
    }

    async fn fee(
        &self,
        _: &str,
        _: TransactionRequestContext,
    ) -> Result<TransactionFeeResult, TransactionError> {
        panic!("Unexpected fee request")
    }

    async fn simulate(
        &self,
        _: &str,
        _: TransactionSimulationOptions,
    ) -> Result<TransactionSimulationResult, TransactionError> {
        Ok(TransactionSimulationResult {
            context_slot: 1,
            err: None,
            logs: Some(vec![]),
            units_consumed: Some(1_000),
            loaded_accounts_data_size: Some(1_024),
            accounts: None,
        })
    }

    async fn send(
        &self,
        transaction: &str,
        _: TransactionSendOptions,
    ) -> Result<TransactionSendResult, TransactionError> {
        self.0.lock().unwrap().push(transaction.into());
        Ok(TransactionSendResult {
            signature: decode(transaction).signatures[0].to_string(),
        })
    }

    async fn signature_status(
        &self,
        signature: &str,
        _: SignatureStatusOptions,
    ) -> Result<Option<TransactionSignatureStatus>, TransactionError> {
        Ok(Some(TransactionSignatureStatus {
            signature: signature.into(),
            slot: Some(2),
            confirmation_status: Some(Commitment::Confirmed),
            err: None,
        }))
    }

    async fn block_height(&self, _: TransactionRequestContext) -> Result<u64, TransactionError> {
        Ok(1)
    }
}

#[tokio::test]
async fn generated_ore_instruction_through_v1_adapter() {
    let payer = Arc::new(Keypair::new_from_array([1; 32]));
    let relay = Arc::new(RecordingRelay::default());
    let wallet = SolanaWalletAdapter::with_config(
        payer.clone(),
        SolanaAdapterConfig {
            transport: Some(relay.clone()),
            ..SolanaAdapterConfig::default()
        },
    );
    let instruction = programs::ore::log(programs::ore::LogParams {
        signer: Some(payer.pubkey().to_string()),
    })
    .unwrap();
    let result = wallet
        .sign_and_send(
            &[instruction],
            &SendOptions {
                transaction_version: Some(TransactionVersion::V1),
                resources: TransactionResourceOptions {
                    compute_unit_limit: Some(200_000),
                    loaded_accounts_data_size_limit: Some(1_048_576),
                    heap_size: Some(32_768),
                    priority_fee_lamports: Some(7),
                    ..Default::default()
                },
                ..Default::default()
            },
            &WalletExecutionContext::default(),
        )
        .await
        .unwrap();
    let sent = relay.0.lock().unwrap();
    assert_eq!(sent.len(), 1);
    let transaction = decode(&sent[0]);
    let VersionedMessage::V1(message) = transaction.message else {
        panic!("expected V1")
    };
    assert_eq!(message.config.compute_unit_limit, Some(200_000));
    assert_eq!(
        message.config.loaded_accounts_data_size_limit,
        Some(1_048_576)
    );
    assert_eq!(message.config.heap_size, Some(32_768));
    assert_eq!(message.config.priority_fee, Some(7));
    assert_eq!(message.instructions.len(), 1);
    let ix = &message.instructions[0];
    assert_eq!(
        message.account_keys[ix.program_id_index as usize].to_string(),
        programs::ore::PROGRAM_ID
    );
    assert_eq!(ix.data, [8]);
    assert_eq!(
        ix.accounts
            .iter()
            .map(|i| message.account_keys[*i as usize])
            .collect::<Vec<_>>(),
        [payer.pubkey()]
    );
    assert_eq!(result.signature, transaction.signatures[0].to_string());
}
