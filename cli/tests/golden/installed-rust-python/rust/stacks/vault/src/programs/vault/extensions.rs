//! Vault program package extension (Rust).

pub use super::vault_math as math;

pub mod constants {
    pub const TREASURY: &str = "Treasury11111111111111111111111111111111111";
    pub const VAULT_DECIMALS: u8 = super::super::vault_math::VAULT_DECIMALS;
}

pub mod addresses {
    use arete_sdk::instruction::InstructionError;

    pub fn treasury() -> Result<String, InstructionError> {
        Ok(super::constants::TREASURY.to_string())
    }

    pub fn program() -> &'static str {
        super::super::generated::PROGRAM_ID
    }
}

pub mod instructions {
    pub mod treasury {
        use arete_sdk::operations::{create_prepared_instruction, PreparedInstruction};
        use arete_sdk::{AreteError, ProgramContext};
        use serde::Deserialize;

        use super::super::super::generated::{DepositParams, VaultProgram};

        #[derive(Debug, Clone, Deserialize)]
        #[serde(rename_all = "camelCase")]
        pub struct DepositInput {
            #[serde(default)]
            pub authority: Option<String>,
            pub mint: String,
            pub amount: u64,
        }

        /// Deposit into the treasury vault, signed by the context's wallet
        /// unless `authority` names another signer.
        pub async fn deposit(
            ctx: &ProgramContext<'_, VaultProgram>,
            input: DepositInput,
        ) -> Result<PreparedInstruction, AreteError> {
            let authority = input.authority.or_else(|| ctx.public_key()).ok_or_else(|| {
                AreteError::invalid_input("deposit needs an authority or a wallet")
            })?;
            let instruction = ctx
                .program()
                .deposit(DepositParams {
                    authority: Some(authority.clone()),
                    vault: super::super::constants::TREASURY.to_string(),
                    mint: input.mint,
                    amount: input.amount,
                })?;
            Ok(create_prepared_instruction(
                "treasury.deposit",
                instruction,
                serde_json::json!({ "authority": authority }),
                None,
                None,
            ))
        }
    }
}

pub mod read {
    use arete_sdk::{AreteError, ProgramContext};

    use super::super::generated::{Vault, VaultProgram};

    /// The cluster slot, through the client's chain reader.
    pub async fn slot(ctx: &ProgramContext<'_, VaultProgram>) -> Result<u64, AreteError> {
        ctx.chain()
            .clock()
            .await
            .map(|clock| clock.slot)
            .map_err(|error| AreteError::ConnectionFailed(error.to_string()))
    }

    /// The `Vault` account at `address`, decoded into the generated model
    /// (`None` when it does not exist).
    pub async fn vault(
        ctx: &ProgramContext<'_, VaultProgram>,
        address: &str,
    ) -> Result<Option<Vault>, AreteError> {
        ctx.program()
            .vault_accounts()?
            .fetch(address)
            .await
            .map_err(|error| AreteError::ConnectionFailed(error.to_string()))
    }

    /// The raw balance of the `Vault` account at `address`.
    pub async fn balance(
        ctx: &ProgramContext<'_, VaultProgram>,
        address: &str,
    ) -> Result<Option<u64>, AreteError> {
        Ok(vault(ctx, address).await?.and_then(|vault| vault.balance))
    }
}
