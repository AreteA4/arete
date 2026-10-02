//! ORE devex extension entry: namespace modules of free functions
//! (`docs/internal/sdk-core-api.md` §9, Rust contract).
//!
//! Staged verbatim from `extensions.json` by `a4 sdk create --rust
//! --extensions`; not generated. The generated `mod.rs` re-exports this
//! module at the stack module root, so its namespaces resolve as
//! `generated::ore::<namespace>::…`:
//!
//! - `addresses` — pure PDA addresses;
//! - `read` — stack reads, taking the connected `Arete<OreStreamStack>`;
//! - `transactions` — ORE program operations, taking the `ore` program's
//!   `ProgramContext` (`a4.programs.ore.context()`). In a published package
//!   these belong to the `ore` program package's own bundle, which a stack
//!   SDK embeds in its `programs::ore` module.

pub use super::devex::DeployWithCheckpointInput;

/// Pure PDA addresses.
pub mod addresses {
    pub use super::super::devex::{
        board_address as board, entropy_var_address as entropy_var, miner_address as miner,
        round_address as round, treasury_address as treasury,
    };
}

/// Stack reads.
pub mod read {
    use arete_sdk::Arete;

    use super::super::generated::{OreRound, OreStreamStack};

    /// Streamed state of the board's current round: board state view →
    /// current round id → round state view. `None` while either view has no
    /// data (for example before the first snapshot arrives).
    pub async fn current_round(a4: &Arete<OreStreamStack>) -> Option<OreRound> {
        let board_address = super::addresses::board().ok()?;
        let board = a4.views.ore_board.state().get(&board_address).await?;
        let round_id = board.state.round_id?;
        a4.views.ore_round.state().get(&round_id.to_string()).await
    }
}

/// ORE program operations.
pub mod transactions {
    pub mod mining {
        use arete_sdk::operations::{
            create_prepared_instruction, create_prepared_transaction, PreparedOperation,
            PreparedTransactionChildren,
        };
        use arete_sdk::{AreteError, ProgramContext};

        use super::super::super::devex::{
            board_address, entropy_var_address, miner_address, round_address,
            DeployWithCheckpointInput,
        };
        use super::super::super::generated::programs::{entropy, ore};

        fn instruction_error(
            context: &str,
            error: arete_sdk::instruction::InstructionError,
        ) -> AreteError {
            AreteError::InvalidConfig(format!("{context}: {error}"))
        }

        fn read_error(context: &str, error: arete_sdk::ReadError) -> AreteError {
            AreteError::ConnectionFailed(format!("{context}: {error}"))
        }

        /// Prepare a deploy against the board's current round, prefixed by a
        /// checkpoint of the miner's previously recorded round when the Miner
        /// account exists (ORE treats current-round and already-checkpointed
        /// checkpoints as safe no-ops). Returns a single prepared instruction
        /// when no checkpoint is needed.
        pub async fn deploy_with_checkpoint(
            ctx: &ProgramContext<'_, ore::OreProgram>,
            input: DeployWithCheckpointInput,
        ) -> Result<PreparedOperation, AreteError> {
            let program = ctx.program();
            let board_pda =
                board_address().map_err(|error| instruction_error("board PDA", error))?;
            let board = program
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
            let miner_pda = miner_address(&authority)
                .map_err(|error| instruction_error("miner PDA", error))?;
            let miner = program
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
            let entropy_var = entropy_var_address()
                .map_err(|error| instruction_error("entropy VAR PDA", error))?;
            let instruction = ore::deploy(ore::DeployParams {
                amount: input.amount,
                squares: input.squares,
                signer: input.signer.clone(),
                authority: authority.clone(),
                round: round.clone(),
                entropy_var: Some(entropy_var),
                entropy_program: Some(entropy::PROGRAM_ID.to_string()),
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
}
