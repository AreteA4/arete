//! Preserve the transaction observations retained by Shipstern.

use arete_interpreter::{
    SolanaTransactionConfig, SolanaTransactionMetadata, SolanaTransactionVersion, UpdateContext,
};
use shipstern_core::instruction::{InstructionShared, InstructionUpdate, Path};

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

/// Render an instruction's 0-based tree path as `"0"` / `"0.1"`.
///
/// `Path`'s own `Debug` renders 1-based, so this is spelled out to match the
/// 0-based `ix_path` the runtime publishes.
pub fn instruction_path(path: &Path) -> String {
    path.as_slice()
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(".")
}

/// One originating context located at this instruction's own occurrence.
///
/// The instruction's tree path identifies it. `log_range` is deliberately not
/// used here: a provider that omits or truncates logs leaves it empty for
/// every instruction in the transaction, which would collapse them all onto
/// one identity.
pub fn instruction_occurrence_context(update: &InstructionUpdate) -> UpdateContext {
    instruction_update_context(&update.shared).at_instruction(instruction_path(&update.path))
}
