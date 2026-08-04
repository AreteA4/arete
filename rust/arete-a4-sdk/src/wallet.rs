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
//! - The optional TS `inspectTransaction` capability is not part of the trait
//!   yet; it arrives with the transaction-transport integration pass.

use std::error::Error as StdError;
use std::fmt;
use std::sync::Arc;

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
                &self.transaction_transport.as_ref().map(|_| "TransactionTransport"),
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operations::FailurePhase;

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

    #[test]
    fn signer_addresses_default_to_public_key() {
        let wallet = FixedKeyWallet;
        assert_eq!(wallet.signer_addresses(), vec!["wallet-address".to_string()]);
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
}
