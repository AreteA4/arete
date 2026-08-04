//! ORE devex extension entry: re-exports the [`super::devex`] helpers and
//! attaches the [`OreDevex`] trait to the connected client.
//!
//! Staged verbatim from `extensions.json` by `a4 sdk create --rust
//! --extensions`; not generated. The generated `mod.rs` re-exports this
//! module at the stack root, so `use generated::ore::*;` (or a direct
//! `use generated::ore::OreDevex;`) brings the trait into scope.

pub use super::devex::*;

use arete_sdk::operations::{
    create_prepared_instruction, create_prepared_transaction, PreparedOperation,
    PreparedTransactionChildren,
};
use arete_sdk::{Arete, AreteError};

use super::programs::{entropy, ore};
use super::{OreRound, OreStreamStack};

fn instruction_error(
    context: &str,
    error: arete_sdk::instruction::InstructionError,
) -> AreteError {
    AreteError::InvalidConfig(format!("{context}: {error}"))
}

fn read_error(context: &str, error: arete_sdk::ReadError) -> AreteError {
    AreteError::ConnectionFailed(format!("{context}: {error}"))
}

/// Devex conveniences attached to `Arete<OreStreamStack>`.
#[async_trait::async_trait]
pub trait OreDevex {
    /// Streamed state of the board's current round: board state view →
    /// current round id → round state view. `None` while either view has no
    /// data (for example before the first snapshot arrives).
    async fn current_round(&self) -> Option<OreRound>;

    /// Prepare a deploy against the board's current round, prefixed by a
    /// checkpoint of the miner's previously recorded round when the Miner
    /// account exists (ORE treats current-round and already-checkpointed
    /// checkpoints as safe no-ops). Returns a single prepared instruction
    /// when no checkpoint is needed.
    async fn deploy_with_checkpoint(
        &self,
        input: DeployWithCheckpointInput,
    ) -> Result<PreparedOperation, AreteError>;
}

#[async_trait::async_trait]
impl OreDevex for Arete<OreStreamStack> {
    async fn current_round(&self) -> Option<OreRound> {
        let board_address = board_address().ok()?;
        let board = self.views.ore_board.state().get(&board_address).await?;
        let round_id = board.state.round_id?;
        self.views
            .ore_round
            .state()
            .get(&round_id.to_string())
            .await
    }

    async fn deploy_with_checkpoint(
        &self,
        input: DeployWithCheckpointInput,
    ) -> Result<PreparedOperation, AreteError> {
        let board_pda = board_address().map_err(|error| instruction_error("board PDA", error))?;
        let board = self
            .programs
            .ore
            .board_accounts()?
            .fetch(&board_pda)
            .await
            .map_err(|error| read_error("Board account read", error))?
            .ok_or_else(|| {
                AreteError::InvalidConfig(format!("ORE Board account not found: {board_pda}"))
            })?;
        let board_round_id = board.round_id.ok_or_else(|| {
            AreteError::InvalidConfig("ORE Board account omitted roundId".to_string())
        })?;
        if let Some(requested) = input.round_id {
            if requested != board_round_id {
                return Err(AreteError::InvalidConfig(format!(
                    "ORE round {requested} is stale; Board is currently on round {board_round_id}"
                )));
            }
        }

        let authority = input.authority.clone();
        let miner_pda =
            miner_address(&authority).map_err(|error| instruction_error("miner PDA", error))?;
        let miner = self
            .programs
            .ore
            .miner_accounts()?
            .fetch(&miner_pda)
            .await
            .map_err(|error| read_error("Miner account read", error))?;
        if miner.is_none() && input.checkpoint_round_id.is_some() {
            return Err(AreteError::InvalidConfig(format!(
                "Cannot checkpoint authority {authority} before its ORE Miner account exists"
            )));
        }

        let mut operations: Vec<PreparedOperation> = Vec::new();
        let mut checkpoint_round_id = None;
        if let Some(round_id) = miner
            .as_ref()
            .and_then(|miner| input.checkpoint_round_id.or(miner.round_id))
        {
            let round = round_address(round_id)
                .map_err(|error| instruction_error("checkpoint round PDA", error))?;
            let instruction = ore::checkpoint(ore::CheckpointParams {
                signer: input.signer.clone(),
                authority: authority.clone(),
                round: round.clone(),
            })
            .map_err(|error| instruction_error("checkpoint instruction", error))?;
            operations.push(
                create_prepared_instruction(
                    "checkpoint",
                    instruction,
                    serde_json::json!({
                        "authority": authority,
                        "roundId": round_id,
                        "round": round,
                    }),
                    None,
                    Some(ore::checkpoint_handler().errors),
                )
                .into(),
            );
            checkpoint_round_id = Some(round_id);
        }

        let round = round_address(board_round_id)
            .map_err(|error| instruction_error("deploy round PDA", error))?;
        let entropy_var =
            entropy_var_address().map_err(|error| instruction_error("entropy VAR PDA", error))?;
        let instruction = ore::deploy(ore::DeployParams {
            amount: input.amount,
            squares: input.squares,
            signer: input.signer.clone(),
            authority: authority.clone(),
            round: round.clone(),
            entropy_var,
            entropy_program: entropy::PROGRAM_ID.to_string(),
        })
        .map_err(|error| instruction_error("deploy instruction", error))?;
        let deploy = create_prepared_instruction(
            "deploy",
            instruction,
            serde_json::json!({
                "authority": authority,
                "roundId": board_round_id,
                "round": round,
                "amount": input.amount,
                "squares": input.squares,
            }),
            None,
            Some(ore::deploy_handler().errors),
        );

        if operations.is_empty() {
            return Ok(deploy.into());
        }
        operations.push(deploy.into());
        create_prepared_transaction(
            "deployWithCheckpoint",
            PreparedTransactionChildren::Operations(operations),
            serde_json::json!({
                "authority": authority,
                "roundId": board_round_id,
                "checkpointIncluded": checkpoint_round_id.is_some(),
                "checkpointRoundId": checkpoint_round_id,
            }),
            None,
            None,
        )
        .map(PreparedOperation::from)
        .map_err(|error| AreteError::InvalidConfig(format!("deployWithCheckpoint: {error}")))
    }
}
