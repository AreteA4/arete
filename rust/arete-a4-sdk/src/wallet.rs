//! Wallet adapter boundary for the Arete SDK.
//!
//! Rust port of `typescript/core/src/wallet/types.ts`. The core SDK is
//! intentionally RPC-free: it only constructs
//! [`BuiltInstruction`](crate::instruction::BuiltInstruction) values.
//! Everything network-related (recent blockhash, message compilation, signing,
//! sending, and confirmation) lives behind the [`WalletAdapter`] boundary,
//! implemented by adapters that wrap the Solana library of your choice
//! (`solana-sdk` keypairs for scripts, remote signers, etc.).
//!
//! Divergences from the TypeScript surface (by design):
//!
//! - TS wallet failures are arbitrary thrown values that the executor
//!   duck-types (`normalizeTransactionError` /
//!   `getTransactionFailureOutcome` walk `cause` chains looking for outcome
//!   shapes, 4001 rejection codes, and program error codes). Rust adapters
//!   instead classify their own failures: [`WalletError`] carries an optional
//!   structured
//!   [`TransactionFailureOutcome`](crate::operations::TransactionFailureOutcome)
//!   which the executor consumes directly. A `WalletError` without an outcome
//!   is classified as not-submitted in the send phase.
//! - Unsigned inspection ([`WalletAdapter::inspect_transaction`]) is a
//!   defaulted trait method: adapters that cannot inspect inherit a
//!   structured [`TransactionCapabilityError::InspectionUnsupported`] failure
//!   instead of the TS optional-method shape.

use std::error::Error as StdError;
use std::fmt;
use std::sync::Arc;

use serde_json::{Map, Value};

use crate::instruction::BuiltInstruction;
use crate::operations::TransactionFailureOutcome;
use crate::transactions::TransactionTransport;

/// Confirmation level for transaction processing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConfirmationLevel {
    /// Transaction processed but not confirmed.
    Processed,
    /// Transaction confirmed by the cluster.
    Confirmed,
    /// Transaction finalized (recommended for production).
    Finalized,
}

impl ConfirmationLevel {
    /// Wire/display name (`"processed" | "confirmed" | "finalized"`).
    pub fn as_str(&self) -> &'static str {
        match self {
            ConfirmationLevel::Processed => "processed",
            ConfirmationLevel::Confirmed => "confirmed",
            ConfirmationLevel::Finalized => "finalized",
        }
    }
}

impl fmt::Display for ConfirmationLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Transaction version requested of a wallet adapter (A4-250 contract §3).
///
/// JSON encoding is `"legacy"` or a **bare JSON number** (`0`/`1`) — never a
/// stringified number. This is the one quantity-shaped value in the build
/// options that is not a decimal string, because a version is not a quantity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransactionVersion {
    /// Legacy (unversioned) transaction.
    Legacy,
    /// Versioned transaction, version 0 (address lookup tables).
    V0,
    /// Versioned transaction, version 1 (SIMD-0385): 4096-byte transactions,
    /// no lookup tables, total-lamport priority fee.
    V1,
}

impl TransactionVersion {
    /// Version first-party builders use when the caller requests none.
    pub const DEFAULT: Self = Self::V0;

    /// The JSON encoding: `"legacy"`, `0`, or `1`.
    pub fn to_json(self) -> Value {
        match self {
            Self::Legacy => Value::String("legacy".to_string()),
            Self::V0 => Value::from(0u8),
            Self::V1 => Value::from(1u8),
        }
    }

    /// Parse the JSON encoding. Stringified numbers (`"0"`) are rejected: the
    /// decimal-string rule does not apply to versions.
    pub fn from_json(value: &Value) -> Result<Self, TransactionCapabilityError> {
        match value {
            Value::String(text) if text == "legacy" => Ok(Self::Legacy),
            Value::Number(number) => match number.as_u64() {
                Some(0) => Ok(Self::V0),
                Some(1) => Ok(Self::V1),
                _ => Err(TransactionCapabilityError::InvalidVersion(
                    number.to_string(),
                )),
            },
            other => Err(TransactionCapabilityError::InvalidVersion(
                other.to_string(),
            )),
        }
    }
}

impl fmt::Display for TransactionVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Legacy => f.write_str("legacy"),
            Self::V0 => f.write_str("0"),
            Self::V1 => f.write_str("1"),
        }
    }
}

/// Canonical wire keys of [`TransactionResourceOptions`] (contract §2).
const COMPUTE_UNIT_LIMIT: &str = "computeUnitLimit";
const LOADED_ACCOUNTS_DATA_SIZE_LIMIT: &str = "loadedAccountsDataSizeLimit";
const HEAP_SIZE: &str = "heapSize";
const PRIORITY_FEE_LAMPORTS: &str = "priorityFeeLamports";
const COMPUTE_UNIT_PRICE_MICRO_LAMPORTS: &str = "computeUnitPriceMicroLamports";

/// Resource budgets requested for one transaction (A4-250 contract §2),
/// shared by [`SendOptions`] and [`TransactionInspectionOptions`].
///
/// Serialization is uniform: [`to_json`](Self::to_json) writes every present
/// field as a decimal string, the `u32` ones included. Parsing is
/// deliberately asymmetric: the three `u32` fields accept a decimal string or
/// a JSON number (no precision risk below 2^53), while the two `u64` fee
/// fields accept a decimal string only — a `u64` that arrived as a double may
/// already have lost precision, and coercing it would launder that into a fee
/// amount.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TransactionResourceOptions {
    /// Compute unit ceiling. All versions.
    pub compute_unit_limit: Option<u32>,
    /// Loaded account data ceiling, in bytes. All versions.
    pub loaded_accounts_data_size_limit: Option<u32>,
    /// Heap frame size, in bytes. All versions.
    pub heap_size: Option<u32>,
    /// Total priority fee in lamports. **V1 only.**
    pub priority_fee_lamports: Option<u64>,
    /// Per-compute-unit price in micro-lamports. **legacy/v0 only.**
    pub compute_unit_price_micro_lamports: Option<u64>,
}

impl TransactionResourceOptions {
    /// Reject fee options bound to a different version, and the two fee
    /// options together. Never converts one fee model into the other.
    pub fn validate_for(
        &self,
        version: TransactionVersion,
    ) -> Result<(), TransactionCapabilityError> {
        if self.priority_fee_lamports.is_some() && self.compute_unit_price_micro_lamports.is_some()
        {
            return Err(TransactionCapabilityError::ConflictingFeeOptions);
        }
        if self.priority_fee_lamports.is_some() && version != TransactionVersion::V1 {
            return Err(TransactionCapabilityError::VersionBoundOption {
                option: PRIORITY_FEE_LAMPORTS,
                required: "1",
                requested: version,
            });
        }
        if self.compute_unit_price_micro_lamports.is_some() && version == TransactionVersion::V1 {
            return Err(TransactionCapabilityError::VersionBoundOption {
                option: COMPUTE_UNIT_PRICE_MICRO_LAMPORTS,
                required: "legacy/0",
                requested: version,
            });
        }
        Ok(())
    }

    /// The canonical camelCase JSON object; every present value a decimal
    /// string. Absent options are omitted.
    pub fn to_json(&self) -> Map<String, Value> {
        let mut object = Map::new();
        let mut set = |key: &str, value: Option<u64>| {
            if let Some(value) = value {
                object.insert(key.to_string(), Value::String(value.to_string()));
            }
        };
        set(COMPUTE_UNIT_LIMIT, self.compute_unit_limit.map(u64::from));
        set(
            LOADED_ACCOUNTS_DATA_SIZE_LIMIT,
            self.loaded_accounts_data_size_limit.map(u64::from),
        );
        set(HEAP_SIZE, self.heap_size.map(u64::from));
        set(PRIORITY_FEE_LAMPORTS, self.priority_fee_lamports);
        set(
            COMPUTE_UNIT_PRICE_MICRO_LAMPORTS,
            self.compute_unit_price_micro_lamports,
        );
        object
    }

    /// Parse the canonical camelCase JSON object. Unrecognized keys are
    /// rejected rather than ignored.
    pub fn from_json(object: &Map<String, Value>) -> Result<Self, TransactionCapabilityError> {
        let mut options = Self::default();
        for (key, value) in object {
            match key.as_str() {
                COMPUTE_UNIT_LIMIT => {
                    options.compute_unit_limit = Some(parse_u32(COMPUTE_UNIT_LIMIT, value)?)
                }
                LOADED_ACCOUNTS_DATA_SIZE_LIMIT => {
                    options.loaded_accounts_data_size_limit =
                        Some(parse_u32(LOADED_ACCOUNTS_DATA_SIZE_LIMIT, value)?)
                }
                HEAP_SIZE => options.heap_size = Some(parse_u32(HEAP_SIZE, value)?),
                PRIORITY_FEE_LAMPORTS => {
                    options.priority_fee_lamports = Some(parse_fee(PRIORITY_FEE_LAMPORTS, value)?)
                }
                COMPUTE_UNIT_PRICE_MICRO_LAMPORTS => {
                    options.compute_unit_price_micro_lamports =
                        Some(parse_fee(COMPUTE_UNIT_PRICE_MICRO_LAMPORTS, value)?)
                }
                unknown => {
                    return Err(TransactionCapabilityError::UnknownResourceOption(
                        unknown.to_string(),
                    ))
                }
            }
        }
        Ok(options)
    }
}

/// Decimal `u64` text (digits only, no sign, no exponent).
fn parse_decimal(text: &str) -> Option<u64> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

fn parse_u32(option: &'static str, value: &Value) -> Result<u32, TransactionCapabilityError> {
    let parsed = match value {
        Value::String(text) => parse_decimal(text),
        Value::Number(number) => number.as_u64(),
        _ => None,
    };
    parsed
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| TransactionCapabilityError::InvalidResourceOption {
            option,
            reason: "expected a decimal string or a JSON number within u32 range".to_string(),
        })
}

fn parse_fee(option: &'static str, value: &Value) -> Result<u64, TransactionCapabilityError> {
    match value {
        Value::String(text) => {
            parse_decimal(text).ok_or_else(|| TransactionCapabilityError::InvalidResourceOption {
                option,
                reason: "expected a decimal string".to_string(),
            })
        }
        Value::Number(_) => Err(TransactionCapabilityError::InvalidResourceOption {
            option,
            reason: "expected a decimal string: a u64 that arrives as a JSON number may already \
                    have lost precision, so it is rejected rather than coerced into a fee amount"
                .to_string(),
        }),
        _ => Err(TransactionCapabilityError::InvalidResourceOption {
            option,
            reason: "expected a decimal string".to_string(),
        }),
    }
}

fn describe_versions(declared: &Option<Vec<TransactionVersion>>) -> String {
    match declared {
        None => "no version support (capability undeclared)".to_string(),
        Some(versions) if versions.is_empty() => "no versions".to_string(),
        Some(versions) => versions
            .iter()
            .map(TransactionVersion::to_string)
            .collect::<Vec<_>>()
            .join(", "),
    }
}

/// Rejection of a version or resource-option request (contract §2/§3/§4).
///
/// Every variant is a refusal, never a downgrade or a conversion: an
/// unsupported version fails instead of falling back, and a fee field bound
/// to the other version fails instead of being translated.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TransactionCapabilityError {
    /// An explicit version the adapter does not advertise.
    #[error("Wallet adapter does not support transaction version {requested} (advertises: {})", describe_versions(.declared))]
    UnsupportedVersion {
        /// The version the caller asked for.
        requested: TransactionVersion,
        /// The adapter's declared versions; `None` means undeclared/unknown.
        declared: Option<Vec<TransactionVersion>>,
    },
    /// A resource option that only exists for another transaction version.
    #[error("Resource option '{option}' requires transaction version {required}, but version {requested} was requested")]
    VersionBoundOption {
        /// Canonical option key.
        option: &'static str,
        /// Version(s) the option belongs to.
        required: &'static str,
        /// Version actually requested.
        requested: TransactionVersion,
    },
    /// Both fee models supplied at once.
    #[error("Resource options '{PRIORITY_FEE_LAMPORTS}' and '{COMPUTE_UNIT_PRICE_MICRO_LAMPORTS}' are mutually exclusive")]
    ConflictingFeeOptions,
    /// A `transactionVersion` value outside `"legacy" | 0 | 1`.
    #[error("Invalid transaction version: {0} (expected \"legacy\", 0 or 1)")]
    InvalidVersion(String),
    /// A resource option whose value did not match the wire contract.
    #[error("Invalid resource option '{option}': {reason}")]
    InvalidResourceOption {
        /// Canonical option key.
        option: &'static str,
        /// Why the value was refused.
        reason: String,
    },
    /// A resource option key the SDK does not know.
    #[error("Unrecognized resource option '{0}'")]
    UnknownResourceOption(String),
    /// The adapter does not implement unsigned inspection.
    #[error("Wallet adapter does not support unsigned transaction inspection")]
    InspectionUnsupported,
}

impl From<TransactionCapabilityError> for WalletError {
    fn from(error: TransactionCapabilityError) -> Self {
        WalletError::new(error.to_string()).with_source(error)
    }
}

/// Options forwarded to the wallet adapter when sending a transaction.
///
/// The core SDK does not interpret these; it passes them straight through to
/// the adapter, which owns all RPC semantics. `extra` is the Rust rendering of
/// the TS index signature: adapter-specific passthrough options (priority
/// fees, lookup tables, etc.).
#[derive(Debug, Clone, Default)]
pub struct SendOptions {
    /// Confirmation level the adapter should wait for.
    pub confirmation_level: Option<ConfirmationLevel>,
    /// Skip the RPC preflight simulation.
    pub skip_preflight: Option<bool>,
    /// Explicit transaction version request (contract §3). `None` leaves the
    /// adapter's existing default in place.
    pub transaction_version: Option<TransactionVersion>,
    /// Resource budgets for this transaction (contract §2).
    pub resources: TransactionResourceOptions,
    /// Adapter-specific passthrough options.
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// Result returned by a wallet adapter after broadcasting a transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendResult {
    /// Transaction signature (base58).
    pub signature: String,
    /// Slot in which the transaction landed, if the adapter reports it.
    pub slot: Option<u64>,
}

/// Options for unsigned transaction inspection (contract §4).
///
/// The Rust counterpart of Python's `inspect` mapping: the same version and
/// resource contract as [`SendOptions`], plus `extra` for adapter-specific
/// passthrough.
#[derive(Debug, Clone, Default)]
pub struct TransactionInspectionOptions {
    /// Explicit transaction version request (contract §3).
    pub transaction_version: Option<TransactionVersion>,
    /// Resource budgets to estimate against (contract §2).
    pub resources: TransactionResourceOptions,
    /// Adapter-specific passthrough options.
    pub extra: Map<String, Value>,
}

/// Result of unsigned transaction inspection (contract §4), mirroring
/// Python's `TransactionInspectionResult`.
///
/// Producing one must never sign, broadcast, or prompt a wallet.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TransactionInspectionResult {
    /// Estimated fee in lamports.
    pub fee_lamports: Option<u64>,
    /// Simulation logs.
    pub logs: Option<Vec<String>>,
    /// Compute units the simulation consumed.
    pub compute_units_consumed: Option<u64>,
    /// Loaded account data size the simulation reported, in bytes — the V1
    /// budget input from contract §1. `None` means the relay did not report
    /// it; `Some(0)` means it reported zero.
    pub loaded_accounts_data_size: Option<u64>,
    /// Slot the simulation ran against.
    pub context_slot: Option<u64>,
    /// Raw simulation error, if the transaction would fail.
    pub error: Option<Value>,
    /// Adapter-specific extra metadata.
    pub extra: Map<String, Value>,
}

/// Execution context passed through to wallet adapters.
///
/// Mirror of the TS `WalletExecutionContext`: the client passes its
/// [`TransactionTransport`] on every `sign_and_send` so adapters can fetch
/// blockhashes, simulate, send, and poll signature status through the stack
/// relay instead of a direct RPC connection. `#[non_exhaustive]` keeps future
/// fields non-breaking; construct via [`WalletExecutionContext::new`] or
/// `Default`.
#[non_exhaustive]
#[derive(Clone, Default)]
pub struct WalletExecutionContext {
    /// Transaction relay transport supplied by the executing client, if any.
    pub transaction_transport: Option<Arc<dyn TransactionTransport>>,
}

impl WalletExecutionContext {
    /// Context carrying the given transaction transport.
    pub fn new(transaction_transport: Option<Arc<dyn TransactionTransport>>) -> Self {
        Self {
            transaction_transport,
        }
    }
}

impl fmt::Debug for WalletExecutionContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WalletExecutionContext")
            .field(
                "transaction_transport",
                &self
                    .transaction_transport
                    .as_ref()
                    .map(|_| "TransactionTransport"),
            )
            .finish()
    }
}

/// Failure reported by a wallet adapter.
///
/// Adapters classify their own failures: when the adapter knows how far the
/// transaction got (wallet rejection, submitted-but-unconfirmed, failed on
/// chain with a program error code), it attaches a structured
/// [`TransactionFailureOutcome`] via [`WalletError::with_outcome`] and the
/// operation executor consumes it directly (no TS-style duck-typing of thrown
/// values). Without an outcome the executor classifies the failure as
/// not-submitted in the send phase.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct WalletError {
    message: String,
    outcome: Option<TransactionFailureOutcome>,
    #[source]
    source: Option<Box<dyn StdError + Send + Sync + 'static>>,
}

impl WalletError {
    /// Create a wallet error from a plain message.
    pub fn new(message: impl Into<String>) -> Self {
        WalletError {
            message: message.into(),
            outcome: None,
            source: None,
        }
    }

    /// Create a wallet error whose message is derived from a structured
    /// failure outcome.
    pub fn from_outcome(outcome: TransactionFailureOutcome) -> Self {
        WalletError {
            message: outcome.message().to_string(),
            outcome: Some(outcome),
            source: None,
        }
    }

    /// Attach a structured failure outcome (how far the transaction got).
    pub fn with_outcome(mut self, outcome: TransactionFailureOutcome) -> Self {
        self.outcome = Some(outcome);
        self
    }

    /// Attach the underlying error source (RPC error, IO error, etc.).
    pub fn with_source(
        mut self,
        source: impl Into<Box<dyn StdError + Send + Sync + 'static>>,
    ) -> Self {
        self.source = Some(source.into());
        self
    }

    /// Human-readable failure message.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// The structured failure outcome, if the adapter classified one.
    pub fn outcome(&self) -> Option<&TransactionFailureOutcome> {
        self.outcome.as_ref()
    }

    /// Consume the error, yielding its structured outcome or a
    /// not-submitted fallback at `fallback_phase` carrying the message.
    pub fn into_outcome(
        self,
        fallback_phase: crate::operations::FailurePhase,
    ) -> TransactionFailureOutcome {
        match self.outcome {
            Some(outcome) => outcome,
            None => TransactionFailureOutcome::NotSubmitted {
                phase: fallback_phase,
                message: self.message,
            },
        }
    }
}

impl From<String> for WalletError {
    fn from(message: String) -> Self {
        WalletError::new(message)
    }
}

impl From<&str> for WalletError {
    fn from(message: &str) -> Self {
        WalletError::new(message)
    }
}

/// Wallet adapter interface for signing and sending transactions.
///
/// Implementations own blockhash fetching, message compilation (legacy or
/// v0), signing, sending, and confirmation. The core SDK only needs
/// [`public_key`](WalletAdapter::public_key) for signer-account resolution and
/// [`sign_and_send`](WalletAdapter::sign_and_send) to broadcast built
/// instructions.
#[async_trait::async_trait]
pub trait WalletAdapter: Send + Sync {
    /// The wallet's public key as a base58-encoded string.
    fn public_key(&self) -> String;

    /// Signer addresses the adapter can satisfy without per-send signers.
    ///
    /// Defaults to `[self.public_key()]`.
    fn signer_addresses(&self) -> Vec<String> {
        vec![self.public_key()]
    }

    /// Compile, sign, and broadcast one or more built instructions as a
    /// single transaction.
    ///
    /// Accepting a slice (rather than a single instruction) makes batching
    /// and composition fall out for free.
    async fn sign_and_send(
        &self,
        instructions: &[BuiltInstruction],
        options: &SendOptions,
        context: &WalletExecutionContext,
    ) -> Result<SendResult, WalletError>;

    /// Transaction versions this adapter declares it can build (contract §3).
    ///
    /// `None` means **unknown**, not "none": adapters written before this
    /// method existed keep working for ordinary sends, while an explicit V1
    /// request against an undeclared adapter is refused rather than silently
    /// downgraded. Adapters declare what they build, e.g.
    /// `Some(&[TransactionVersion::V0])`.
    fn supported_transaction_versions(&self) -> Option<&[TransactionVersion]> {
        None
    }

    /// Refuse an explicit version this adapter does not advertise, and any
    /// resource option bound to a different version. Never downgrades and
    /// never converts between fee models.
    ///
    /// A `None` version validates the resource options against
    /// [`TransactionVersion::DEFAULT`], the version first-party builders
    /// already use when the caller passes nothing.
    fn validate_transaction_options(
        &self,
        version: Option<TransactionVersion>,
        resources: &TransactionResourceOptions,
    ) -> Result<(), TransactionCapabilityError> {
        if let Some(requested) = version {
            let declared = self.supported_transaction_versions();
            let supported = match declared {
                Some(versions) => versions.contains(&requested),
                // Undeclared capability is unknown, not unsupported: only V1
                // (which no pre-capability adapter can build) is refused.
                None => requested != TransactionVersion::V1,
            };
            if !supported {
                return Err(TransactionCapabilityError::UnsupportedVersion {
                    requested,
                    declared: declared.map(<[TransactionVersion]>::to_vec),
                });
            }
        }
        resources.validate_for(version.unwrap_or(TransactionVersion::DEFAULT))
    }

    /// Inspect instructions without signing, broadcasting, or prompting
    /// (contract §4).
    ///
    /// Defaults to [`TransactionCapabilityError::InspectionUnsupported`] for
    /// the same reason as
    /// [`TransactionTransport::transaction`](crate::transactions::TransactionTransport::transaction):
    /// there is no slower path to fall back to, so an adapter that cannot
    /// inspect must say so rather than invent an answer. Keeps this addition
    /// source-compatible for out-of-crate adapters.
    async fn inspect_transaction(
        &self,
        _instructions: &[BuiltInstruction],
        _options: &TransactionInspectionOptions,
        _context: &WalletExecutionContext,
    ) -> Result<TransactionInspectionResult, WalletError> {
        Err(TransactionCapabilityError::InspectionUnsupported.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operations::FailurePhase;

    /// Adapter written before the version/inspection capability existed: it
    /// implements none of the new trait methods, so this fixture failing to
    /// compile is the source-compatibility regression test.
    struct FixedKeyWallet;

    #[async_trait::async_trait]
    impl WalletAdapter for FixedKeyWallet {
        fn public_key(&self) -> String {
            "wallet-address".to_string()
        }

        async fn sign_and_send(
            &self,
            _instructions: &[BuiltInstruction],
            _options: &SendOptions,
            _context: &WalletExecutionContext,
        ) -> Result<SendResult, WalletError> {
            Err(WalletError::new("unused"))
        }
    }

    /// The capability every first-party adapter declares in this ticket.
    struct V0OnlyWallet;

    #[async_trait::async_trait]
    impl WalletAdapter for V0OnlyWallet {
        fn public_key(&self) -> String {
            "v0-wallet".to_string()
        }

        fn supported_transaction_versions(&self) -> Option<&[TransactionVersion]> {
            Some(&[TransactionVersion::V0])
        }

        async fn sign_and_send(
            &self,
            _instructions: &[BuiltInstruction],
            _options: &SendOptions,
            _context: &WalletExecutionContext,
        ) -> Result<SendResult, WalletError> {
            Err(WalletError::new("unused"))
        }
    }

    #[test]
    fn signer_addresses_default_to_public_key() {
        let wallet = FixedKeyWallet;
        assert_eq!(
            wallet.signer_addresses(),
            vec!["wallet-address".to_string()]
        );
    }

    #[test]
    fn wallet_error_without_outcome_falls_back_to_phase() {
        let error = WalletError::new("connection reset");
        assert_eq!(error.message(), "connection reset");
        assert!(error.outcome().is_none());
        assert_eq!(
            error.into_outcome(FailurePhase::Send),
            TransactionFailureOutcome::NotSubmitted {
                phase: FailurePhase::Send,
                message: "connection reset".to_string(),
            }
        );
    }

    #[test]
    fn wallet_error_prefers_attached_outcome() {
        let outcome = TransactionFailureOutcome::SubmittedUnknown {
            signature: "sig".to_string(),
            slot: Some(42),
            message: "confirmation timed out".to_string(),
        };
        let error = WalletError::from_outcome(outcome.clone());
        assert_eq!(error.message(), "confirmation timed out");
        assert_eq!(error.into_outcome(FailurePhase::Wallet), outcome);
    }

    #[test]
    fn wallet_error_carries_source() {
        let io = std::io::Error::other("socket closed");
        let error = WalletError::new("send failed").with_source(io);
        let source = std::error::Error::source(&error).expect("source");
        assert_eq!(source.to_string(), "socket closed");
    }

    #[test]
    fn v1_contract_undeclared_capability_means_unknown_not_unsupported() {
        let wallet = FixedKeyWallet;
        let resources = TransactionResourceOptions::default();
        assert_eq!(wallet.supported_transaction_versions(), None);

        // Ordinary operations keep working against an undeclared adapter.
        wallet
            .validate_transaction_options(None, &resources)
            .unwrap();
        wallet
            .validate_transaction_options(Some(TransactionVersion::V0), &resources)
            .unwrap();
        wallet
            .validate_transaction_options(Some(TransactionVersion::Legacy), &resources)
            .unwrap();

        // An explicit V1 request does not silently downgrade.
        assert_eq!(
            wallet
                .validate_transaction_options(Some(TransactionVersion::V1), &resources)
                .unwrap_err(),
            TransactionCapabilityError::UnsupportedVersion {
                requested: TransactionVersion::V1,
                declared: None,
            }
        );
    }

    #[test]
    fn v1_contract_declared_capability_rejects_every_undeclared_version() {
        let wallet = V0OnlyWallet;
        let resources = TransactionResourceOptions::default();
        wallet
            .validate_transaction_options(Some(TransactionVersion::V0), &resources)
            .unwrap();

        let error = wallet
            .validate_transaction_options(Some(TransactionVersion::V1), &resources)
            .unwrap_err();
        assert_eq!(
            error,
            TransactionCapabilityError::UnsupportedVersion {
                requested: TransactionVersion::V1,
                declared: Some(vec![TransactionVersion::V0]),
            }
        );
        assert_eq!(
            error.to_string(),
            "Wallet adapter does not support transaction version 1 (advertises: 0)"
        );
        assert!(wallet
            .validate_transaction_options(Some(TransactionVersion::Legacy), &resources)
            .is_err());
    }

    #[tokio::test]
    async fn v1_contract_inspection_defaults_to_unsupported_capability() {
        let error = FixedKeyWallet
            .inspect_transaction(
                &[],
                &TransactionInspectionOptions::default(),
                &WalletExecutionContext::default(),
            )
            .await
            .unwrap_err();
        assert_eq!(
            error.message(),
            "Wallet adapter does not support unsigned transaction inspection"
        );
    }

    #[test]
    fn v1_contract_fee_options_are_version_bound_and_mutually_exclusive() {
        let priority = TransactionResourceOptions {
            priority_fee_lamports: Some(5_000),
            ..TransactionResourceOptions::default()
        };
        priority.validate_for(TransactionVersion::V1).unwrap();
        for version in [TransactionVersion::Legacy, TransactionVersion::V0] {
            assert_eq!(
                priority.validate_for(version).unwrap_err(),
                TransactionCapabilityError::VersionBoundOption {
                    option: "priorityFeeLamports",
                    required: "1",
                    requested: version,
                }
            );
        }

        let price = TransactionResourceOptions {
            compute_unit_price_micro_lamports: Some(7),
            ..TransactionResourceOptions::default()
        };
        price.validate_for(TransactionVersion::Legacy).unwrap();
        price.validate_for(TransactionVersion::V0).unwrap();
        assert_eq!(
            price.validate_for(TransactionVersion::V1).unwrap_err(),
            TransactionCapabilityError::VersionBoundOption {
                option: "computeUnitPriceMicroLamports",
                required: "legacy/0",
                requested: TransactionVersion::V1,
            }
        );

        let both = TransactionResourceOptions {
            priority_fee_lamports: Some(1),
            compute_unit_price_micro_lamports: Some(1),
            ..TransactionResourceOptions::default()
        };
        assert_eq!(
            both.validate_for(TransactionVersion::V1).unwrap_err(),
            TransactionCapabilityError::ConflictingFeeOptions
        );
    }

    #[test]
    fn v1_contract_resource_options_serialize_as_decimal_strings() {
        let options = TransactionResourceOptions {
            compute_unit_limit: Some(200_000),
            loaded_accounts_data_size_limit: Some(65_536),
            heap_size: Some(0),
            priority_fee_lamports: Some(u64::MAX),
            compute_unit_price_micro_lamports: None,
        };
        assert_eq!(
            Value::Object(options.to_json()),
            serde_json::json!({
                "computeUnitLimit": "200000",
                "loadedAccountsDataSizeLimit": "65536",
                "heapSize": "0",
                "priorityFeeLamports": "18446744073709551615",
            })
        );
        assert_eq!(
            TransactionResourceOptions::from_json(&options.to_json()).unwrap(),
            options
        );
        assert!(TransactionResourceOptions::default().to_json().is_empty());
    }

    #[test]
    fn v1_contract_u32_options_accept_numbers_but_fees_require_decimal_strings() {
        let object = serde_json::json!({ "computeUnitLimit": 200000, "heapSize": "32768" });
        let parsed = TransactionResourceOptions::from_json(object.as_object().unwrap()).unwrap();
        assert_eq!(parsed.compute_unit_limit, Some(200_000));
        assert_eq!(parsed.heap_size, Some(32_768));

        let numeric_fee = serde_json::json!({ "priorityFeeLamports": 9007199254740993u64 });
        let error =
            TransactionResourceOptions::from_json(numeric_fee.as_object().unwrap()).unwrap_err();
        assert!(
            error.to_string().contains("lost precision"),
            "rejection must name the precision hazard, got: {error}"
        );

        let unknown = serde_json::json!({ "tip": "1" });
        assert_eq!(
            TransactionResourceOptions::from_json(unknown.as_object().unwrap()).unwrap_err(),
            TransactionCapabilityError::UnknownResourceOption("tip".to_string())
        );

        for invalid in [
            serde_json::json!({ "computeUnitLimit": "-1" }),
            serde_json::json!({ "computeUnitLimit": 4294967296u64 }),
            serde_json::json!({ "heapSize": 1.5 }),
            serde_json::json!({ "priorityFeeLamports": "1e9" }),
        ] {
            assert!(
                TransactionResourceOptions::from_json(invalid.as_object().unwrap()).is_err(),
                "expected {invalid} to be rejected"
            );
        }
    }

    #[test]
    fn v1_contract_version_json_is_never_a_stringified_number() {
        assert_eq!(
            TransactionVersion::Legacy.to_json(),
            serde_json::json!("legacy")
        );
        assert_eq!(TransactionVersion::V0.to_json(), serde_json::json!(0));
        assert_eq!(TransactionVersion::V1.to_json(), serde_json::json!(1));
        for value in [
            serde_json::json!("legacy"),
            serde_json::json!(0),
            serde_json::json!(1),
        ] {
            assert_eq!(
                TransactionVersion::from_json(&value).unwrap().to_json(),
                value
            );
        }
        for invalid in [
            serde_json::json!("0"),
            serde_json::json!("1"),
            serde_json::json!(2),
            serde_json::json!("v0"),
            serde_json::json!(null),
        ] {
            assert!(
                TransactionVersion::from_json(&invalid).is_err(),
                "expected {invalid} to be rejected"
            );
        }
    }
}
