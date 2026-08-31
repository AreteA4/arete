//! Shared types for the instruction-building layer.

use std::collections::BTreeMap;

use serde_json::Value;
use solana_pubkey::Pubkey;
use thiserror::Error;

/// Account meta in a built instruction, ready for transaction assembly.
#[derive(Debug, Clone, PartialEq)]
pub struct BuiltAccountMeta {
    /// Account address.
    pub pubkey: Pubkey,
    /// Whether this account must sign the transaction.
    pub is_signer: bool,
    /// Whether this account is writable.
    pub is_writable: bool,
}

/// A fully built instruction: program, ordered accounts, and serialized data.
#[derive(Debug, Clone, PartialEq)]
pub struct BuiltInstruction {
    /// Program that processes this instruction.
    pub program_id: Pubkey,
    /// Ordered account metas (declared accounts, then any remaining accounts).
    pub accounts: Vec<BuiltAccountMeta>,
    /// Discriminator followed by Borsh-encoded arguments.
    pub data: Vec<u8>,
}

/// Supported argument types for Borsh serialization.
///
/// Struct and enum schemas are fully inlined (field names and types travel
/// with the schema), so the serializer needs no runtime type registry.
#[derive(Debug, Clone, PartialEq)]
pub enum ArgType {
    U8,
    U16,
    U32,
    U64,
    U128,
    I8,
    I16,
    I32,
    I64,
    I128,
    F32,
    F64,
    Bool,
    String,
    Pubkey,
    Bytes,
    /// Variable-length sequence: u32 LE count followed by the elements.
    Vec(Box<ArgType>),
    /// Variable-length sequence: u64 LE count followed by the elements (bincode-style).
    VecU64Len(Box<ArgType>),
    /// Optional value: 0x00 for None, 0x01 followed by the inner value for Some.
    Option(Box<ArgType>),
    /// Fixed-length array: exactly N elements, no length prefix.
    Array(Box<ArgType>, usize),
    /// String-keyed map: u32 LE count followed by key/value pairs sorted by the
    /// key's UTF-8 bytes.
    HashMap(Box<ArgType>, Box<ArgType>),
    /// Named fields serialized in declaration order.
    Struct(Vec<ArgField>),
    /// Borsh enum: one-byte variant index followed by the variant payload.
    Enum(Vec<EnumVariantDef>),
}

/// One field of a struct schema, in declaration (serialization) order.
#[derive(Debug, Clone, PartialEq)]
pub struct ArgField {
    /// Field name.
    pub name: String,
    /// Field type.
    pub ty: ArgType,
}

/// One enum variant definition.
///
/// Values: unit variants are passed as the variant name (or its index);
/// data-carrying variants as a single-key object, e.g. `{"transfer": {"amount": 1}}`
/// or `{"pair": [1, 2]}`.
#[derive(Debug, Clone, PartialEq)]
pub struct EnumVariantDef {
    /// Variant name.
    pub name: String,
    /// Variant shape.
    pub kind: EnumVariantKind,
}

/// The payload shape of an enum variant.
#[derive(Debug, Clone, PartialEq)]
pub enum EnumVariantKind {
    /// No payload.
    Unit,
    /// Named fields serialized like a struct.
    Struct(Vec<ArgField>),
    /// Positional elements serialized in order.
    Tuple(Vec<ArgType>),
}

/// Instruction argument schema entry: name plus type, in serialization order.
#[derive(Debug, Clone, PartialEq)]
pub struct ArgSchema {
    /// Argument name.
    pub name: String,
    /// Argument type.
    pub ty: ArgType,
}

/// A single seed in a PDA derivation.
#[derive(Debug, Clone, PartialEq)]
pub enum PdaSeed {
    /// Fixed string, encoded as UTF-8 bytes.
    Literal(String),
    /// Raw bytes.
    Bytes(Vec<u8>),
    /// Reference to an instruction argument (dot-path into args, falling back
    /// to the helper-only `resolve` map), serialized at `arg_type`'s width.
    ArgRef {
        /// Dot-path into the args / resolve maps (e.g. `"args.transactionIndex"`).
        arg: String,
        /// Declared seed type (e.g. `"u64"`, `"pubkey"`); heuristics apply if absent.
        arg_type: Option<String>,
    },
    /// The 32-byte address of a previously resolved account.
    AccountRef(String),
}

/// Configuration for PDA (Program-Derived Address) derivation.
#[derive(Debug, Clone, PartialEq)]
pub struct PdaConfig {
    /// Program that owns this PDA (defaults to the instruction's program ID).
    pub program_id: Option<String>,
    /// Seed definitions for derivation.
    pub seeds: Vec<PdaSeed>,
}

/// How an account's address is determined during resolution.
#[derive(Debug, Clone, PartialEq)]
pub enum AccountResolution {
    /// Must sign; resolved from an override or the fallback payer.
    Signer,
    /// Fixed, well-known address (e.g. the System Program).
    Known(String),
    /// Derived from seeds via [`PdaConfig`].
    Pda(PdaConfig),
    /// Must be supplied by the caller.
    UserProvided,
}

/// Metadata for a single account in an instruction.
#[derive(Debug, Clone, PartialEq)]
pub struct AccountMeta {
    /// Account name (e.g. `"user"`, `"mint"`).
    pub name: String,
    /// Whether this account must sign the transaction.
    pub is_signer: bool,
    /// Whether this account is writable.
    pub is_writable: bool,
    /// How the account's address is resolved.
    pub resolution: AccountResolution,
    /// Whether this account may be omitted.
    pub is_optional: bool,
}

/// Program error definition from the IDL.
#[derive(Debug, Clone, PartialEq)]
pub struct ErrorMetadata {
    /// Error code.
    pub code: u32,
    /// Error name (e.g. `"SlippageExceeded"`).
    pub name: String,
    /// Human-readable message.
    pub msg: String,
}

/// Options for building an instruction (no network access).
#[derive(Debug, Clone, Default)]
pub struct BuildOptions {
    /// Fallback signer address (mirror of the TS `wallet.publicKey` fallback).
    pub payer: Option<String>,
    /// Unvalidated account-address overrides (win over param-derived overrides).
    pub accounts: BTreeMap<String, String>,
    /// Extra account metas appended after declared accounts (Anchor `remainingAccounts`).
    pub remaining_accounts: Vec<BuiltAccountMeta>,
}

/// Errors produced while building an instruction.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum InstructionError {
    /// A non-optional argument was absent (or JSON null) in the params.
    #[error("Missing required argument \"{name}\"")]
    MissingArgument {
        /// Argument name.
        name: String,
    },
    /// A params key matched neither an argument nor an account name.
    #[error("Unknown parameter \"{name}\". Expected one of args [{}] or accounts [{}]", .args.join(", "), .accounts.join(", "))]
    UnknownParameter {
        /// The unrecognized key.
        name: String,
        /// Declared argument names.
        args: Vec<String>,
        /// Declared account names.
        accounts: Vec<String>,
    },
    /// Required accounts could not be resolved.
    #[error("Missing required accounts: {}", .0.join(", "))]
    MissingAccounts(Vec<String>),
    /// A value had the wrong shape or was out of range for its schema type.
    #[error("Invalid value for \"{context}\": {message}")]
    InvalidValue {
        /// Where in the params the value lives (dot-path).
        context: String,
        /// What went wrong.
        message: String,
    },
    /// PDA derivation or seed serialization failed.
    #[error("PDA error: {0}")]
    Pda(String),
    /// An address failed to parse as a base58 32-byte public key.
    #[error("Invalid pubkey: {0}")]
    InvalidPubkey(String),
    /// PDA `accountRef` seeds form a cycle.
    #[error("Circular dependency in PDA accounts: {0}")]
    CircularPdaDependency(String),
    /// A PDA account declared no program ID and no fallback was available.
    #[error("Cannot derive PDA for \"{0}\": no program ID specified")]
    MissingProgramId(String),
}

/// Human-readable JSON value kind, for error messages.
pub(crate) fn json_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}
