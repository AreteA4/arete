//! Hand-authored ORE address and workflow helpers layered over the generated
//! `ore` stack module.
//!
//! Staged verbatim from `extensions.json` by `a4 sdk create --rust
//! --extensions`; not generated. Helpers stay reachable as
//! `generated::ore::devex::…` and are re-exported at the stack module root by
//! the `extensions.rs` entry.

use arete_sdk::instruction::{derive_program_address, serialize_seed_value, InstructionError};

use super::programs::{entropy, ore};

/// Base-58 address of the ORE board PDA (seeds `["board"]`).
pub fn board_address() -> Result<String, InstructionError> {
    ore::pdas::board().map(|(address, _)| address.to_string())
}

/// Base-58 address of the ORE treasury PDA (seeds `["treasury"]`).
pub fn treasury_address() -> Result<String, InstructionError> {
    ore::pdas::treasury().map(|(address, _)| address.to_string())
}

/// Base-58 address of a miner PDA (seeds `["miner", authority]`).
pub fn miner_address(authority: &str) -> Result<String, InstructionError> {
    ore::pdas::miner(authority).map(|(address, _)| address.to_string())
}

/// Base-58 address of a round PDA (seeds `["round", round_id as u64 LE]`).
///
/// The generated `pdas` module omits `round` because its seed comes from an
/// instruction argument instead of an account; derive it here with the same
/// seed serializer the generated helpers use.
pub fn round_address(round_id: u64) -> Result<String, InstructionError> {
    let seeds: Vec<Vec<u8>> = vec![
        "round".as_bytes().to_vec(),
        serialize_seed_value(&serde_json::json!(round_id), Some("u64"))?,
    ];
    derive_program_address(&seeds, ore::PROGRAM_ID).map(|(address, _)| address.to_string())
}

/// Base-58 address of the entropy VAR PDA feeding the ORE board
/// (seeds `["var", board, 0u64]` on the entropy program).
pub fn entropy_var_address() -> Result<String, InstructionError> {
    let board = board_address()?;
    let seeds: Vec<Vec<u8>> = vec![
        "var".as_bytes().to_vec(),
        serialize_seed_value(&serde_json::json!(board), Some("pubkey"))?,
        serialize_seed_value(&serde_json::json!(0u64), Some("u64"))?,
    ];
    derive_program_address(&seeds, entropy::PROGRAM_ID).map(|(address, _)| address.to_string())
}

/// Input for [`super::extensions::OreDevex::deploy_with_checkpoint`]: a
/// deploy against the board's current round, prefixed by a checkpoint of the
/// miner's previously recorded round when the Miner account exists.
#[derive(Debug, Clone, Default)]
pub struct DeployWithCheckpointInput {
    /// Miner authority (also the default transaction signer).
    pub authority: String,
    /// Lamports to deploy.
    pub amount: u64,
    /// Encoded square selection (same encoding as
    /// [`super::programs::ore::DeployParams::squares`]).
    pub squares: u32,
    /// Optional signer override applied to both instructions.
    pub signer: Option<String>,
    /// Deploy round override; defaults to the board's current round and is
    /// rejected when stale.
    pub round_id: Option<u64>,
    /// Checkpoint round override; defaults to the miner's recorded round.
    pub checkpoint_round_id: Option<u64>,
}
