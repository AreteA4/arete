//! Transaction V1 end-to-end through the optional Solana adapter.
//!
//! ```text
//! cargo run -p arete-a4-sdk --features solana-adapter --example solana_v1
//! ```
//!
//! Defaults are local and disposable: a freshly generated keypair and a relay
//! on `http://127.0.0.1:8080` (`arete-server` in front of a local validator).
//! The run **inspects only**. Broadcasting is opt-in:
//!
//! ```text
//! ARETE_TRANSACTION_RELAY=http://127.0.0.1:8080 \
//! ARETE_API_KEY=hspk_… \
//! ARETE_EXAMPLE_EXECUTE=1 \
//!   cargo run -p arete-a4-sdk --features solana-adapter --example solana_v1
//! ```
//!
//! Arete is the default backend. The escape hatch is one variable — the
//! adapter then talks to that node directly, with the same compiler, budget
//! estimator, inspection, signing and confirmation above it:
//!
//! ```text
//! ARETE_EXAMPLE_RPC_URL=http://127.0.0.1:8899 \
//!   cargo run -p arete-a4-sdk --features solana-adapter --example solana_v1
//! ```

use std::env;
use std::sync::Arc;

use arete_a4_sdk::http::{AuthTokenRequest, TokenSource};
use arete_a4_sdk::instruction::BuiltInstruction;
use arete_a4_sdk::rpc::RpcTransactionTransport;
use arete_a4_sdk::transactions::{HttpTransactionTransport, TransactionTransport};
use arete_a4_sdk::wallet::{
    SendOptions, TransactionInspectionOptions, TransactionResourceOptions, TransactionVersion,
    WalletAdapter, WalletExecutionContext,
};
use arete_a4_sdk::{
    AdapterTransportSelection, AreteError, SolanaAdapterConfig, SolanaWalletAdapter,
};
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;

/// Local validator relays run unauthenticated; a hosted stack takes an API
/// key from the environment.
struct EnvApiKey(Option<String>);

#[async_trait::async_trait]
impl TokenSource for EnvApiKey {
    async fn token(
        &self,
        _request: &AuthTokenRequest,
        _force_refresh: bool,
    ) -> Result<Option<String>, AreteError> {
        Ok(self.0.clone())
    }

    fn invalidate(&self, _request: &AuthTokenRequest) {}
}

/// SPL Memo v3 — a neutral instruction that touches no accounts and moves no
/// funds, so the example demonstrates the transaction shape and nothing else.
const MEMO_PROGRAM: Pubkey = Pubkey::from_str_const("MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr");

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let relay =
        env::var("ARETE_TRANSACTION_RELAY").unwrap_or_else(|_| "http://127.0.0.1:8080".to_string());
    let execute = env::var("ARETE_EXAMPLE_EXECUTE").is_ok_and(|value| value == "1");

    // A disposable signer: this example never reads a key file and never
    // touches mainnet. Fund it on a local validator to execute for real.
    let payer = Arc::new(Keypair::new());

    // Arete by default; an explicit RPC URL selects the node instead and
    // wins over any transport a connected client would pass in.
    let (backend, transport, transport_selection) = match env::var("ARETE_EXAMPLE_RPC_URL") {
        Ok(url) => (
            url.clone(),
            Arc::new(RpcTransactionTransport::new(url)) as Arc<dyn TransactionTransport>,
            AdapterTransportSelection::Direct,
        ),
        Err(_) => (
            relay.clone(),
            Arc::new(HttpTransactionTransport::new(
                &relay,
                Arc::new(EnvApiKey(env::var("ARETE_API_KEY").ok())),
            )) as Arc<dyn TransactionTransport>,
            AdapterTransportSelection::Auto,
        ),
    };

    let adapter = SolanaWalletAdapter::with_config(
        payer.clone(),
        SolanaAdapterConfig {
            transport: Some(transport),
            transport_selection,
            ..SolanaAdapterConfig::default()
        },
    );
    println!("payer   {}", adapter.public_key());
    println!("backend {backend} ({transport_selection:?})");
    println!("versions {:?}", adapter.supported_transaction_versions());

    let instructions = vec![BuiltInstruction {
        program_id: MEMO_PROGRAM,
        accounts: Vec::new(),
        data: b"arete transaction v1 example".to_vec(),
    }];

    // Unsigned inspection: no signature, no broadcast, no prompt. V1 is
    // requested explicitly — an omitted version would keep the v0 default.
    let context = WalletExecutionContext::default();
    let inspection = adapter
        .inspect_transaction(
            &instructions,
            &TransactionInspectionOptions {
                transaction_version: Some(TransactionVersion::V1),
                resources: TransactionResourceOptions {
                    priority_fee_lamports: Some(5_000),
                    ..TransactionResourceOptions::default()
                },
                ..TransactionInspectionOptions::default()
            },
            &context,
        )
        .await?;
    println!("fee     {:?} lamports", inspection.fee_lamports);
    println!("units   {:?}", inspection.compute_units_consumed);
    println!("loaded  {:?} bytes", inspection.loaded_accounts_data_size);
    if let Some(error) = &inspection.error {
        println!("simulation error {error}");
    }

    if !execute {
        println!("\ninspection only; set ARETE_EXAMPLE_EXECUTE=1 to submit");
        return Ok(());
    }

    // The two V1 budgets are left out on purpose: the adapter measures them
    // for itself (provisional message at the protocol maxima, simulated with
    // signature verification off, then derived with headroom). Pass explicit
    // values instead and they are used verbatim, never raised.
    let result = adapter
        .sign_and_send(
            &instructions,
            &SendOptions {
                transaction_version: Some(TransactionVersion::V1),
                resources: TransactionResourceOptions {
                    heap_size: Some(64 * 1024),
                    priority_fee_lamports: Some(5_000),
                    ..TransactionResourceOptions::default()
                },
                ..SendOptions::default()
            },
            &context,
        )
        .await?;
    println!("signature {} (slot {:?})", result.signature, result.slot);
    Ok(())
}
