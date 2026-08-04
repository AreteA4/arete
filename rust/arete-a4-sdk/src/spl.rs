//! SPL token program helpers.
//!
//! Port of `typescript/core/src/spl.ts`: the well-known program addresses,
//! synchronous associated-token-account derivation, and token-program
//! resolution from a mint's owner.

use std::str::FromStr;

use solana_pubkey::Pubkey;
use thiserror::Error;

use crate::chain::{ChainClient, ChainError};
use crate::instruction::{derive_program_address, InstructionError};

/// SPL Token program.
pub const SPL_TOKEN_PROGRAM_ADDRESS: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
/// Token-2022 program.
pub const TOKEN_2022_PROGRAM_ADDRESS: &str = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb";
/// Associated Token Account program.
pub const ASSOCIATED_TOKEN_PROGRAM_ADDRESS: &str = "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL";
/// System program.
pub const SYSTEM_PROGRAM_ADDRESS: &str = "11111111111111111111111111111111";

/// Errors produced by SPL helpers.
#[derive(Debug, Error)]
pub enum SplError {
    /// An address is not a valid base58 32-byte public key.
    #[error("Invalid public key: {0}")]
    InvalidPubkey(String),

    /// PDA derivation failed.
    #[error(transparent)]
    Derivation(#[from] InstructionError),

    /// The mint account does not exist on the read endpoint.
    #[error("Mint account not found while resolving token program: {0}")]
    MintNotFound(String),

    /// The mint is owned by a program other than SPL Token / Token-2022.
    #[error("Mint {mint} is owned by unsupported token program {owner_program}")]
    UnsupportedTokenProgram { mint: String, owner_program: String },

    /// The chain read failed.
    #[error(transparent)]
    Chain(#[from] ChainError),
}

fn pubkey_seed(address: &str) -> Result<Vec<u8>, SplError> {
    Pubkey::from_str(address)
        .map(|key| key.to_bytes().to_vec())
        .map_err(|_| SplError::InvalidPubkey(address.to_string()))
}

/// Derives the associated token account for `owner` + `mint` (synchronously).
///
/// Seeds are `[owner, token_program, mint]` against the associated-token
/// program; `token_program` defaults to the SPL Token program.
pub fn derive_associated_token_account(
    owner: &str,
    mint: &str,
    token_program: Option<&str>,
) -> Result<String, SplError> {
    let seeds = vec![
        pubkey_seed(owner)?,
        pubkey_seed(token_program.unwrap_or(SPL_TOKEN_PROGRAM_ADDRESS))?,
        pubkey_seed(mint)?,
    ];
    let (address, _bump) = derive_program_address(&seeds, ASSOCIATED_TOKEN_PROGRAM_ADDRESS)?;
    Ok(address.to_string())
}

/// Resolves the token program owning `mint`.
///
/// An explicit `override_program` short-circuits without a chain read;
/// otherwise the mint's owner is read and must be SPL Token or Token-2022.
pub async fn resolve_token_program_address(
    chain: &dyn ChainClient,
    mint: &str,
    override_program: Option<&str>,
) -> Result<String, SplError> {
    if let Some(override_program) = override_program {
        return Ok(override_program.to_string());
    }
    let mint_account = chain
        .mint(mint)
        .await?
        .ok_or_else(|| SplError::MintNotFound(mint.to_string()))?;
    if mint_account.owner_program != SPL_TOKEN_PROGRAM_ADDRESS
        && mint_account.owner_program != TOKEN_2022_PROGRAM_ADDRESS
    {
        return Err(SplError::UnsupportedTokenProgram {
            mint: mint.to_string(),
            owner_program: mint_account.owner_program,
        });
    }
    Ok(mint_account.owner_program)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chain::{
        ChainClock, ContextSlotOptions, MintAccountInfo, NativeBalanceInfo, RawAccountInfo,
        TokenAccountInfo, TokenBalanceInfo, TokenBalanceInput,
    };
    use async_trait::async_trait;

    const WSOL_MINT: &str = "So11111111111111111111111111111111111111112";
    const USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";

    enum MintBehavior {
        Panic,
        Missing,
        Owned(&'static str),
    }

    struct FakeChain(MintBehavior);

    #[async_trait]
    impl ChainClient for FakeChain {
        async fn exists(&self, _address: &str) -> Result<bool, ChainError> {
            unimplemented!()
        }
        async fn lamports(&self, _address: &str) -> Result<u64, ChainError> {
            unimplemented!()
        }
        async fn native_balance(
            &self,
            _address: &str,
            _options: ContextSlotOptions,
        ) -> Result<NativeBalanceInfo, ChainError> {
            unimplemented!()
        }
        async fn minimum_balance_for_rent_exemption(&self, _space: u64) -> Result<u64, ChainError> {
            unimplemented!()
        }
        async fn clock(&self) -> Result<ChainClock, ChainError> {
            unimplemented!()
        }
        async fn account(&self, _address: &str) -> Result<Option<RawAccountInfo>, ChainError> {
            unimplemented!()
        }
        async fn mint(&self, address: &str) -> Result<Option<MintAccountInfo>, ChainError> {
            match &self.0 {
                MintBehavior::Panic => panic!("unexpected mint read"),
                MintBehavior::Missing => Ok(None),
                MintBehavior::Owned(owner_program) => Ok(Some(MintAccountInfo {
                    address: address.to_string(),
                    owner_program: owner_program.to_string(),
                    decimals: Some(6),
                    supply: None,
                    mint_authority: None,
                    freeze_authority: None,
                })),
            }
        }
        async fn token_account(
            &self,
            _address: &str,
        ) -> Result<Option<TokenAccountInfo>, ChainError> {
            unimplemented!()
        }
        async fn balance(
            &self,
            _input: &TokenBalanceInput,
            _options: ContextSlotOptions,
        ) -> Result<TokenBalanceInfo, ChainError> {
            unimplemented!()
        }
    }

    #[test]
    fn matches_known_mainnet_usdc_ata_derivation() {
        // Same vector as the TS spl.test.ts: the wSOL mint address used as an
        // owner + the mainnet USDC mint.
        let ata = derive_associated_token_account(WSOL_MINT, USDC_MINT, None).unwrap();
        assert_eq!(ata, "DHe62eeQVEnNK7vg5xUpDkJm7tuqHadjhvmPRFBG9UPo");
    }

    #[test]
    fn derivation_is_deterministic_and_uses_token_program_in_seeds() {
        let default_program =
            derive_associated_token_account(WSOL_MINT, USDC_MINT, None).unwrap();
        let explicit_spl = derive_associated_token_account(
            WSOL_MINT,
            USDC_MINT,
            Some(SPL_TOKEN_PROGRAM_ADDRESS),
        )
        .unwrap();
        let token_2022 = derive_associated_token_account(
            WSOL_MINT,
            USDC_MINT,
            Some(TOKEN_2022_PROGRAM_ADDRESS),
        )
        .unwrap();

        assert_eq!(default_program, explicit_spl);
        assert_ne!(explicit_spl, token_2022);
    }

    #[test]
    fn rejects_invalid_pubkeys() {
        assert!(matches!(
            derive_associated_token_account("not-base58!", USDC_MINT, None),
            Err(SplError::InvalidPubkey(_))
        ));
        assert!(matches!(
            derive_associated_token_account(WSOL_MINT, "short", None),
            Err(SplError::InvalidPubkey(_))
        ));
    }

    #[tokio::test]
    async fn override_short_circuits_without_reading_the_mint() {
        let chain = FakeChain(MintBehavior::Panic);
        let resolved = resolve_token_program_address(&chain, "mint", Some("custom-program"))
            .await
            .unwrap();
        assert_eq!(resolved, "custom-program");
    }

    #[tokio::test]
    async fn infers_supported_mint_owners() {
        for owner in [SPL_TOKEN_PROGRAM_ADDRESS, TOKEN_2022_PROGRAM_ADDRESS] {
            let chain = FakeChain(MintBehavior::Owned(owner));
            let resolved = resolve_token_program_address(&chain, "mint", None)
                .await
                .unwrap();
            assert_eq!(resolved, owner);
        }
    }

    #[tokio::test]
    async fn rejects_missing_mints_and_unsupported_owners() {
        let chain = FakeChain(MintBehavior::Missing);
        assert!(matches!(
            resolve_token_program_address(&chain, "missing", None).await,
            Err(SplError::MintNotFound(mint)) if mint == "missing"
        ));

        let chain = FakeChain(MintBehavior::Owned("UnsupportedProgram1111111111111111111111111"));
        assert!(matches!(
            resolve_token_program_address(&chain, "mint", None).await,
            Err(SplError::UnsupportedTokenProgram { .. })
        ));
    }
}
