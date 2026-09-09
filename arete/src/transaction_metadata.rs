//! Preserve the transaction observations retained by Shipstern.

use arete_interpreter::{
    SolanaTransactionConfig, SolanaTransactionMetadata, SolanaTransactionVersion, UpdateContext,
};
use shipstern_core::instruction::InstructionShared;

/// Config presence identifies V1, including an empty config. Shipstern discards
/// the source version flag, so absent config must remain unknown.
pub fn observed_transaction_metadata(
    shared: &InstructionShared,
) -> Option<SolanaTransactionMetadata> {
    shared
        .transaction_config
        .as_ref()
        .map(|config| SolanaTransactionMetadata {
            version: SolanaTransactionVersion::V1,
            config: Some(SolanaTransactionConfig {
                priority_fee_lamports: config.priority_fee,
                compute_unit_limit: config.compute_unit_limit,
                loaded_accounts_data_size_limit: config.loaded_accounts_data_size_limit,
                heap_size: config.heap_size,
            }),
        })
}

/// One originating context for the instruction, its hooks/events and continuations.
pub fn instruction_update_context(shared: &InstructionShared) -> UpdateContext {
    let context = UpdateContext::new_instruction(
        shared.slot,
        bs58::encode(&shared.signature).into_string(),
        shared.txn_index,
    );
    match observed_transaction_metadata(shared) {
        Some(metadata) => context.with_solana_transaction(metadata),
        None => context,
    }
}
