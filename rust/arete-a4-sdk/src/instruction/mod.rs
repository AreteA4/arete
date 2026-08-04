//! Instruction building: Borsh argument serialization, PDA derivation, and
//! account resolution.
//!
//! Rust port of the TypeScript instruction layer
//! (`typescript/core/src/instructions/`), with byte-for-byte identical wire
//! output: little-endian integers, u32 length prefixes, single-byte option and
//! enum prefixes, UTF-8-sorted map keys, and Anchor's placeholder convention
//! for omitted optional accounts.
//!
//! Generated stack code produces [`InstructionHandler`] values; callers invoke
//! [`InstructionHandler::build`] with a merged params object (args plus
//! account-address overrides) and receive a [`BuiltInstruction`] ready for
//! transaction assembly. Building is pure — no network access.

mod handler;
mod resolver;
mod seed;
mod serializer;
mod types;

pub use handler::InstructionHandler;
pub use resolver::{resolve_accounts, AccountResolutionResult, ResolvedAccount};
pub use seed::{derive_program_address, normalize_seed_type, serialize_seed_value, CanonicalSeedType};
pub use serializer::serialize_instruction_data;
pub use solana_pubkey::Pubkey;
pub use types::{
    AccountMeta, AccountResolution, ArgField, ArgSchema, ArgType, BuildOptions, BuiltAccountMeta,
    BuiltInstruction, EnumVariantDef, EnumVariantKind, ErrorMetadata, InstructionError, PdaConfig,
    PdaSeed,
};
