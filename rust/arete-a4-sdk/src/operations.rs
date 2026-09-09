//! Prepared operations: composable instruction/transaction/flow plans,
//! receipts, and the execution driver.
//!
//! Rust port of `typescript/core/src/operations.ts`,
//! `typescript/core/src/signer-registry.ts`, and the transaction-outcome model
//! from `typescript/core/src/instructions/error-parser.ts`.
//!
//! Divergences from the TypeScript surface (by design):
//!
//! - **Artifacts** are [`serde_json::Value`] instead of a TS generic payload;
//!   typed artifacts ride in and out via serde
//!   (`serde_json::to_value` / `from_value`).
//! - **Failure causes** are structured. TS outcomes carry an opaque `cause`
//!   that the executor duck-types; Rust outcomes carry a `message` plus an
//!   optional parsed [`ProgramError`], and wallets classify their own failures
//!   via [`WalletError`](crate::wallet::WalletError) (see `src/wallet.rs`).
//! - **Callbacks are synchronous observers** (`Arc<dyn Fn(&…)>`); they observe
//!   and never alter outcomes. Unlike the TS executor (which collects callback
//!   errors into `callbackErrors` on receipts), callback panics are not caught
//!   — callbacks are expected not to panic.
//! - **Exactly-one-of `instructions`/`operations`** for
//!   [`create_prepared_transaction`] is enforced by the
//!   [`PreparedTransactionChildren`] enum instead of a runtime check; flows as
//!   children are rejected at runtime (TS rejects them at the type level).
//! - [`SignerRegistry`] stores `String -> Arc<dyn Signer>` where
//!   [`Signer::address`] is explicit, instead of the TS registry of fully
//!   opaque values whose addresses are duck-typed at validation time. The
//!   registry uses interior mutability (shared via `Arc`) and stores entries
//!   sorted by address rather than in insertion order.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, RwLock};

use serde_json::{json, Value};

use crate::instruction::{BuiltInstruction, ErrorMetadata};
use crate::wallet::{
    SendOptions, TransactionCapabilityError, TransactionInspectionOptions,
    TransactionInspectionResult, WalletAdapter, WalletError, WalletExecutionContext,
};

// ---------------------------------------------------------------------------
// Transaction outcome model (port of instructions/error-parser.ts)
// ---------------------------------------------------------------------------

/// Where in the pipeline a transaction failure occurred.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FailurePhase {
    /// Failed before dispatch (e.g. signer validation).
    Build,
    /// Rejected or failed inside the wallet (e.g. user rejection).
    Wallet,
    /// Failed while sending to the network.
    Send,
    /// Failed while waiting for confirmation.
    Confirmation,
    /// Failed on chain.
    Chain,
}

impl FailurePhase {
    /// Wire/display name (`"build" | "wallet" | "send" | "confirmation" | "chain"`).
    pub fn as_str(&self) -> &'static str {
        match self {
            FailurePhase::Build => "build",
            FailurePhase::Wallet => "wallet",
            FailurePhase::Send => "send",
            FailurePhase::Confirmation => "confirmation",
            FailurePhase::Chain => "chain",
        }
    }
}

impl fmt::Display for FailurePhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Custom error from a Solana program, resolved against IDL error metadata
/// when available.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgramError {
    /// Error code.
    pub code: u32,
    /// Error name.
    pub name: String,
    /// Human-readable message.
    pub message: String,
}

impl ProgramError {
    /// Fallback for a code with no matching IDL metadata (mirror of the TS
    /// `CustomError<code>` placeholder).
    pub fn unknown(code: u32) -> Self {
        ProgramError {
            code,
            name: format!("CustomError{code}"),
            message: format!("Unknown error with code {code}"),
        }
    }
}

impl fmt::Display for ProgramError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({}): {}", self.name, self.code, self.message)
    }
}

/// Resolve a raw program error code against IDL error metadata.
///
/// Returns `None` when no metadata entry matches; use
/// [`ProgramError::unknown`] for the TS-style placeholder fallback.
pub fn parse_program_error(code: u32, errors: &[ErrorMetadata]) -> Option<ProgramError> {
    errors
        .iter()
        .find(|entry| entry.code == code)
        .map(|entry| ProgramError {
            code: entry.code,
            name: entry.name.clone(),
            message: entry.msg.clone(),
        })
}

/// Format a program error as `"Name (code): message"` (mirror of the TS
/// `formatProgramError`).
pub fn format_program_error(error: &ProgramError) -> String {
    error.to_string()
}

/// Terminal outcome of a transaction attempt.
#[derive(Debug, Clone, PartialEq)]
pub enum TransactionOutcome {
    /// The transaction confirmed.
    Confirmed {
        /// Transaction signature (base58).
        signature: String,
        /// Slot in which the transaction landed, if known.
        slot: Option<u64>,
    },
    /// The transaction failed; see [`TransactionFailureOutcome`] for how far
    /// it got.
    Failed(TransactionFailureOutcome),
}

/// Structured transaction failure: how far the transaction got before it
/// failed (mirror of the TS `TransactionFailureOutcome` discriminated union).
#[derive(Debug, Clone, PartialEq)]
pub enum TransactionFailureOutcome {
    /// The transaction never reached the network.
    NotSubmitted {
        /// Pipeline phase where the failure occurred (build, wallet, or send).
        phase: FailurePhase,
        /// Human-readable failure message.
        message: String,
    },
    /// The transaction was submitted but its final status is unknown
    /// (e.g. confirmation timed out).
    SubmittedUnknown {
        /// Transaction signature (base58).
        signature: String,
        /// Slot in which the transaction landed, if known.
        slot: Option<u64>,
        /// Human-readable failure message.
        message: String,
    },
    /// The transaction failed on chain.
    ChainFailed {
        /// Transaction signature (base58), if known.
        signature: Option<String>,
        /// Slot in which the transaction failed, if known.
        slot: Option<u64>,
        /// Parsed program error, if the failure matched an error code.
        program_error: Option<ProgramError>,
        /// Human-readable failure message.
        message: String,
    },
}

impl TransactionFailureOutcome {
    /// Pipeline phase of the failure. `SubmittedUnknown` maps to
    /// [`FailurePhase::Confirmation`] and `ChainFailed` to
    /// [`FailurePhase::Chain`].
    pub fn phase(&self) -> FailurePhase {
        match self {
            TransactionFailureOutcome::NotSubmitted { phase, .. } => *phase,
            TransactionFailureOutcome::SubmittedUnknown { .. } => FailurePhase::Confirmation,
            TransactionFailureOutcome::ChainFailed { .. } => FailurePhase::Chain,
        }
    }

    /// Human-readable failure message.
    pub fn message(&self) -> &str {
        match self {
            TransactionFailureOutcome::NotSubmitted { message, .. }
            | TransactionFailureOutcome::SubmittedUnknown { message, .. }
            | TransactionFailureOutcome::ChainFailed { message, .. } => message,
        }
    }

    /// Transaction signature, when the transaction reached the network.
    pub fn signature(&self) -> Option<&str> {
        match self {
            TransactionFailureOutcome::NotSubmitted { .. } => None,
            TransactionFailureOutcome::SubmittedUnknown { signature, .. } => Some(signature),
            TransactionFailureOutcome::ChainFailed { signature, .. } => signature.as_deref(),
        }
    }

    /// Slot associated with the failure, if known.
    pub fn slot(&self) -> Option<u64> {
        match self {
            TransactionFailureOutcome::NotSubmitted { .. } => None,
            TransactionFailureOutcome::SubmittedUnknown { slot, .. }
            | TransactionFailureOutcome::ChainFailed { slot, .. } => *slot,
        }
    }

    /// Parsed program error for chain failures.
    pub fn program_error(&self) -> Option<&ProgramError> {
        match self {
            TransactionFailureOutcome::ChainFailed { program_error, .. } => program_error.as_ref(),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Prepared operation model (port of operations.ts)
// ---------------------------------------------------------------------------

/// Kind of a prepared operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OperationKind {
    /// Single instruction wrapped in a single transaction.
    Instruction,
    /// Single transaction composed of one or more instructions.
    Transaction,
    /// Multiple sequential transactions.
    Flow,
}

impl OperationKind {
    /// Wire/display name (`"instruction" | "transaction" | "flow"`).
    pub fn as_str(&self) -> &'static str {
        match self {
            OperationKind::Instruction => "instruction",
            OperationKind::Transaction => "transaction",
            OperationKind::Flow => "flow",
        }
    }
}

impl fmt::Display for OperationKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Errors from prepared-operation construction and composition.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OperationError {
    /// A transaction body was constructed with no instructions.
    #[error("Transaction '{0}' must contain at least one item")]
    EmptyTransaction(String),
    /// A flow was constructed with no transactions.
    #[error("Flow '{0}' must contain at least one item")]
    EmptyFlow(String),
    /// A flow was passed as a child operation of a transaction.
    #[error("Transaction '{transaction}' cannot include flow '{flow}' as a child operation")]
    FlowChild {
        /// Name of the transaction being composed.
        transaction: String,
        /// Name of the offending flow.
        flow: String,
    },
    /// A flow composition helper referenced a transaction index that does not
    /// exist.
    #[error("Flow '{flow}' has no transaction at index {index}")]
    MissingFlowTransaction {
        /// Flow name.
        flow: String,
        /// Out-of-range transaction index.
        index: usize,
    },
    /// A signer registry address was empty.
    #[error("Signer registry addresses must not be empty")]
    EmptySignerAddress,
    /// Inspection was requested for a flow.
    #[error("Cannot inspect flow '{0}': flow inspection is not supported")]
    FlowInspection(String),
}

/// One transaction inside a prepared operation: named, non-empty instruction
/// list, required signers, and IDL error metadata for failure parsing.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedTransactionBody {
    /// Transaction name (used in error messages and receipts).
    pub name: String,
    /// Instructions, in execution order. Non-empty (enforced by
    /// constructors).
    pub instructions: Vec<BuiltInstruction>,
    /// Addresses that must be able to sign before dispatch (deduplicated,
    /// order-preserving).
    pub required_signer_addresses: Vec<String>,
    /// IDL error metadata used to parse chain failures.
    pub errors: Vec<ErrorMetadata>,
}

fn dedupe(values: Vec<String>) -> Vec<String> {
    let mut result: Vec<String> = Vec::with_capacity(values.len());
    for value in values {
        if !result.contains(&value) {
            result.push(value);
        }
    }
    result
}

fn infer_signer_addresses(instructions: &[BuiltInstruction]) -> Vec<String> {
    dedupe(
        instructions
            .iter()
            .flat_map(|instruction| {
                instruction
                    .accounts
                    .iter()
                    .filter(|account| account.is_signer)
                    .map(|account| account.pubkey.to_string())
            })
            .collect(),
    )
}

/// Build a validated [`PreparedTransactionBody`].
///
/// When `required_signer_addresses` is `None`, required signers are inferred
/// from the instructions' `is_signer` account metas (deduplicated,
/// order-preserving). Explicit lists are deduplicated but otherwise taken
/// as-is.
pub fn create_prepared_transaction_body(
    name: impl Into<String>,
    instructions: Vec<BuiltInstruction>,
    required_signer_addresses: Option<Vec<String>>,
    errors: Option<Vec<ErrorMetadata>>,
) -> Result<PreparedTransactionBody, OperationError> {
    let name = name.into();
    if instructions.is_empty() {
        return Err(OperationError::EmptyTransaction(name));
    }
    let required_signer_addresses =
        dedupe(required_signer_addresses.unwrap_or_else(|| infer_signer_addresses(&instructions)));
    Ok(PreparedTransactionBody {
        name,
        instructions,
        required_signer_addresses,
        errors: errors.unwrap_or_default(),
    })
}

/// A prepared single-instruction operation.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedInstruction {
    /// Operation name.
    pub name: String,
    /// The built instruction.
    pub instruction: BuiltInstruction,
    /// The single-transaction plan wrapping the instruction.
    pub transaction: PreparedTransactionBody,
    /// Operation artifacts (typed payloads ride in via serde).
    pub artifacts: Value,
}

/// A prepared single-transaction operation.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedTransaction {
    /// Operation name.
    pub name: String,
    /// The transaction plan.
    pub transaction: PreparedTransactionBody,
    /// Operation artifacts (typed payloads ride in via serde).
    pub artifacts: Value,
}

/// A prepared multi-transaction operation.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedFlow {
    /// Operation name.
    pub name: String,
    /// Transaction plans, in execution order. Non-empty (enforced by
    /// constructors).
    pub transactions: Vec<PreparedTransactionBody>,
    /// Operation artifacts (typed payloads ride in via serde).
    pub artifacts: Value,
}

/// Any prepared operation: instruction, transaction, or flow.
#[derive(Debug, Clone, PartialEq)]
pub enum PreparedOperation {
    /// Single-instruction operation.
    Instruction(PreparedInstruction),
    /// Single-transaction operation.
    Transaction(PreparedTransaction),
    /// Multi-transaction operation.
    Flow(PreparedFlow),
}

impl PreparedOperation {
    /// Operation kind.
    pub fn kind(&self) -> OperationKind {
        match self {
            PreparedOperation::Instruction(_) => OperationKind::Instruction,
            PreparedOperation::Transaction(_) => OperationKind::Transaction,
            PreparedOperation::Flow(_) => OperationKind::Flow,
        }
    }

    /// Operation name.
    pub fn name(&self) -> &str {
        match self {
            PreparedOperation::Instruction(op) => &op.name,
            PreparedOperation::Transaction(op) => &op.name,
            PreparedOperation::Flow(op) => &op.name,
        }
    }

    /// The transaction plan, in execution order (mirror of the TS
    /// `operation.plan.transactions`). Single-transaction operations yield a
    /// one-element slice.
    pub fn plan(&self) -> &[PreparedTransactionBody] {
        match self {
            PreparedOperation::Instruction(op) => std::slice::from_ref(&op.transaction),
            PreparedOperation::Transaction(op) => std::slice::from_ref(&op.transaction),
            PreparedOperation::Flow(op) => &op.transactions,
        }
    }

    /// Operation artifacts.
    pub fn artifacts(&self) -> &Value {
        match self {
            PreparedOperation::Instruction(op) => &op.artifacts,
            PreparedOperation::Transaction(op) => &op.artifacts,
            PreparedOperation::Flow(op) => &op.artifacts,
        }
    }
}

impl From<PreparedInstruction> for PreparedOperation {
    fn from(value: PreparedInstruction) -> Self {
        PreparedOperation::Instruction(value)
    }
}

impl From<PreparedTransaction> for PreparedOperation {
    fn from(value: PreparedTransaction) -> Self {
        PreparedOperation::Transaction(value)
    }
}

impl From<PreparedFlow> for PreparedOperation {
    fn from(value: PreparedFlow) -> Self {
        PreparedOperation::Flow(value)
    }
}

/// Build a [`PreparedInstruction`] whose plan is the single wrapped
/// instruction. Signers are inferred from the instruction unless overridden.
pub fn create_prepared_instruction(
    name: impl Into<String>,
    instruction: BuiltInstruction,
    artifacts: Value,
    required_signer_addresses: Option<Vec<String>>,
    errors: Option<Vec<ErrorMetadata>>,
) -> PreparedInstruction {
    let name = name.into();
    let transaction = create_prepared_transaction_body(
        name.clone(),
        vec![instruction.clone()],
        required_signer_addresses,
        errors,
    )
    .expect("single-instruction transaction is never empty");
    PreparedInstruction {
        name,
        instruction,
        transaction,
        artifacts,
    }
}

/// One instruction input for [`create_prepared_transaction`]: either a raw
/// built instruction or an already-prepared instruction operation (whose
/// signer/error metadata is inherited).
#[derive(Debug, Clone, PartialEq)]
pub enum PreparedTransactionInstruction {
    /// A raw built instruction (signers inferred from its account metas).
    Built(BuiltInstruction),
    /// A prepared instruction (its transaction body is inherited).
    Prepared(PreparedInstruction),
}

/// Children of [`create_prepared_transaction`]: exactly one of raw/prepared
/// instructions or child operations (mirror of the TS exactly-one-of
/// `instructions`/`operations` input, enforced by this enum).
#[derive(Debug, Clone, PartialEq)]
pub enum PreparedTransactionChildren {
    /// Compose from instructions (raw or prepared).
    Instructions(Vec<PreparedTransactionInstruction>),
    /// Compose from child operations. Flows are rejected; instruction and
    /// transaction children contribute their instructions, signers, and error
    /// metadata.
    Operations(Vec<PreparedOperation>),
}

/// Build a [`PreparedTransaction`] from instructions or child operations.
///
/// Child signer and error metadata is inherited (concatenated in child order,
/// signers deduplicated) unless overridden via `required_signer_addresses` /
/// `errors`. Flows are rejected as children.
pub fn create_prepared_transaction(
    name: impl Into<String>,
    children: PreparedTransactionChildren,
    artifacts: Value,
    required_signer_addresses: Option<Vec<String>>,
    errors: Option<Vec<ErrorMetadata>>,
) -> Result<PreparedTransaction, OperationError> {
    let name = name.into();
    let parts: Vec<PreparedTransactionBody> = match children {
        PreparedTransactionChildren::Instructions(instructions) => instructions
            .into_iter()
            .map(|instruction| match instruction {
                PreparedTransactionInstruction::Prepared(prepared) => Ok(prepared.transaction),
                PreparedTransactionInstruction::Built(built) => {
                    create_prepared_transaction_body(name.clone(), vec![built], None, None)
                }
            })
            .collect::<Result<_, _>>()?,
        PreparedTransactionChildren::Operations(operations) => operations
            .into_iter()
            .map(|operation| match operation {
                PreparedOperation::Instruction(op) => Ok(op.transaction),
                PreparedOperation::Transaction(op) => Ok(op.transaction),
                PreparedOperation::Flow(flow) => Err(OperationError::FlowChild {
                    transaction: name.clone(),
                    flow: flow.name,
                }),
            })
            .collect::<Result<_, _>>()?,
    };
    let instructions: Vec<BuiltInstruction> = parts
        .iter()
        .flat_map(|part| part.instructions.clone())
        .collect();
    let inherited_signers = required_signer_addresses.unwrap_or_else(|| {
        parts
            .iter()
            .flat_map(|part| part.required_signer_addresses.clone())
            .collect()
    });
    let inherited_errors =
        errors.unwrap_or_else(|| parts.iter().flat_map(|part| part.errors.clone()).collect());
    let transaction = create_prepared_transaction_body(
        name.clone(),
        instructions,
        Some(inherited_signers),
        Some(inherited_errors),
    )?;
    Ok(PreparedTransaction {
        name,
        transaction,
        artifacts,
    })
}

/// Build a [`PreparedFlow`] from transaction bodies (each re-validated:
/// non-empty instructions, deduplicated signers).
pub fn create_prepared_flow(
    name: impl Into<String>,
    transactions: Vec<PreparedTransactionBody>,
    artifacts: Value,
) -> Result<PreparedFlow, OperationError> {
    let name = name.into();
    if transactions.is_empty() {
        return Err(OperationError::EmptyFlow(name));
    }
    let transactions = transactions
        .into_iter()
        .map(|body| {
            create_prepared_transaction_body(
                body.name,
                body.instructions,
                Some(body.required_signer_addresses),
                Some(body.errors),
            )
        })
        .collect::<Result<_, _>>()?;
    Ok(PreparedFlow {
        name,
        transactions,
        artifacts,
    })
}

/// Return a copy of `transaction` with `instructions` prepended. Signers
/// inferred from the new instructions are merged in front of the existing
/// required signers (deduplicated, order-preserving).
pub fn prepend_transaction_instructions(
    transaction: &PreparedTransactionBody,
    instructions: &[BuiltInstruction],
) -> Result<PreparedTransactionBody, OperationError> {
    let mut combined = instructions.to_vec();
    combined.extend(transaction.instructions.iter().cloned());
    let mut signers = infer_signer_addresses(instructions);
    signers.extend(transaction.required_signer_addresses.iter().cloned());
    create_prepared_transaction_body(
        transaction.name.clone(),
        combined,
        Some(signers),
        Some(transaction.errors.clone()),
    )
}

/// Return a copy of `transaction` with `instructions` appended. Signers
/// inferred from the new instructions are merged after the existing required
/// signers (deduplicated, order-preserving).
pub fn append_transaction_instructions(
    transaction: &PreparedTransactionBody,
    instructions: &[BuiltInstruction],
) -> Result<PreparedTransactionBody, OperationError> {
    let mut combined = transaction.instructions.clone();
    combined.extend(instructions.iter().cloned());
    let mut signers = transaction.required_signer_addresses.clone();
    signers.extend(infer_signer_addresses(instructions));
    create_prepared_transaction_body(
        transaction.name.clone(),
        combined,
        Some(signers),
        Some(transaction.errors.clone()),
    )
}

/// Return a copy of `flow` with `transactions` appended.
pub fn append_flow_transactions(
    flow: &PreparedFlow,
    transactions: Vec<PreparedTransactionBody>,
) -> Result<PreparedFlow, OperationError> {
    let mut combined = flow.transactions.clone();
    combined.extend(transactions);
    create_prepared_flow(flow.name.clone(), combined, flow.artifacts.clone())
}

/// Return a copy of `flow` with `instructions` prepended to the transaction
/// at `transaction_index`.
pub fn prepend_flow_transaction_instructions(
    flow: &PreparedFlow,
    transaction_index: usize,
    instructions: &[BuiltInstruction],
) -> Result<PreparedFlow, OperationError> {
    let Some(transaction) = flow.transactions.get(transaction_index) else {
        return Err(OperationError::MissingFlowTransaction {
            flow: flow.name.clone(),
            index: transaction_index,
        });
    };
    let mut transactions = flow.transactions.clone();
    transactions[transaction_index] = prepend_transaction_instructions(transaction, instructions)?;
    create_prepared_flow(flow.name.clone(), transactions, flow.artifacts.clone())
}

// ---------------------------------------------------------------------------
// Receipts
// ---------------------------------------------------------------------------

/// Receipt for one executed transaction within an operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationTransactionReceipt {
    /// Zero-based index of the transaction within the operation plan.
    pub transaction_index: usize,
    /// Name of the executed transaction.
    pub transaction_name: String,
    /// Transaction signature (base58).
    pub signature: String,
    /// Slot in which the transaction landed, if known.
    pub slot: Option<u64>,
}

/// Receipt for a fully executed prepared operation.
///
/// Single-transaction operations (instruction/transaction) have exactly one
/// entry in `transactions`; flows have one entry per transaction, in
/// execution order.
#[derive(Debug, Clone, PartialEq)]
pub struct OperationReceipt {
    /// Kind of the executed operation.
    pub kind: OperationKind,
    /// Name of the executed operation.
    pub operation_name: String,
    /// Operation artifacts (typed payloads ride out via serde).
    pub artifacts: Value,
    /// Transaction signatures, in execution order. Never empty.
    pub signatures: Vec<String>,
    /// Per-transaction receipts, in execution order. Never empty.
    pub transactions: Vec<OperationTransactionReceipt>,
}

impl OperationReceipt {
    /// The first (and, for instruction/transaction operations, only)
    /// transaction receipt.
    pub fn transaction(&self) -> &OperationTransactionReceipt {
        &self.transactions[0]
    }
}

// ---------------------------------------------------------------------------
// Signer registry (port of signer-registry.ts)
// ---------------------------------------------------------------------------

/// An opaque signer that knows its own address.
///
/// TS registers fully opaque values and duck-types their addresses at
/// validation time; the Rust idiom makes the address explicit. Concrete
/// signing material stays inside wallet adapters — the registry only
/// enumerates addresses for pre-dispatch validation and lets adapters fetch
/// the values they registered.
pub trait Signer: Send + Sync {
    /// Base58 address this signer can sign for.
    fn address(&self) -> String;
}

/// Address-keyed registry of opaque signers (port of the TS
/// `SignerRegistry`).
///
/// Uses interior mutability so it can be shared via `Arc` (mirroring the TS
/// closure-captured `Map`). Entries are stored sorted by address (divergence:
/// TS preserves insertion order; ordering is only observable through
/// [`addresses`](SignerRegistry::addresses)/[`values`](SignerRegistry::values)/
/// [`entries`](SignerRegistry::entries)).
#[derive(Default)]
pub struct SignerRegistry {
    signers: RwLock<BTreeMap<String, Arc<dyn Signer>>>,
}

impl SignerRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        SignerRegistry::default()
    }

    /// Register `signer` under `address` (replacing any existing entry).
    /// Empty addresses are rejected.
    pub fn register(
        &self,
        address: impl Into<String>,
        signer: Arc<dyn Signer>,
    ) -> Result<(), OperationError> {
        let address = address.into();
        if address.is_empty() {
            return Err(OperationError::EmptySignerAddress);
        }
        self.signers
            .write()
            .expect("signer registry poisoned")
            .insert(address, signer);
        Ok(())
    }

    /// Register `signer` under its own [`Signer::address`].
    pub fn register_signer(&self, signer: Arc<dyn Signer>) -> Result<(), OperationError> {
        let address = signer.address();
        self.register(address, signer)
    }

    /// Remove the entry for `address`, returning whether one existed.
    pub fn unregister(&self, address: &str) -> bool {
        self.signers
            .write()
            .expect("signer registry poisoned")
            .remove(address)
            .is_some()
    }

    /// The signer registered under `address`, if any.
    pub fn get(&self, address: &str) -> Option<Arc<dyn Signer>> {
        self.signers
            .read()
            .expect("signer registry poisoned")
            .get(address)
            .cloned()
    }

    /// Whether a signer is registered under `address`.
    pub fn has(&self, address: &str) -> bool {
        self.signers
            .read()
            .expect("signer registry poisoned")
            .contains_key(address)
    }

    /// All registered addresses (sorted).
    pub fn addresses(&self) -> Vec<String> {
        self.signers
            .read()
            .expect("signer registry poisoned")
            .keys()
            .cloned()
            .collect()
    }

    /// All registered signers (sorted by address).
    pub fn values(&self) -> Vec<Arc<dyn Signer>> {
        self.signers
            .read()
            .expect("signer registry poisoned")
            .values()
            .cloned()
            .collect()
    }

    /// All `(address, signer)` entries (sorted by address).
    pub fn entries(&self) -> Vec<(String, Arc<dyn Signer>)> {
        self.signers
            .read()
            .expect("signer registry poisoned")
            .iter()
            .map(|(address, signer)| (address.clone(), Arc::clone(signer)))
            .collect()
    }

    /// Remove all entries.
    pub fn clear(&self) {
        self.signers
            .write()
            .expect("signer registry poisoned")
            .clear();
    }

    /// Number of registered signers.
    pub fn len(&self) -> usize {
        self.signers.read().expect("signer registry poisoned").len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl fmt::Debug for SignerRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SignerRegistry")
            .field("addresses", &self.addresses())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Execution
// ---------------------------------------------------------------------------

/// Host context for operation execution: the wallet to dispatch through plus
/// any host-level signer addresses (mirror of the TS
/// `OperationExecutionHost`).
#[derive(Default, Clone)]
pub struct ExecutionHost<'a> {
    /// Wallet adapter used to sign and send each transaction.
    pub wallet: Option<&'a dyn WalletAdapter>,
    /// Additional signer addresses the host can satisfy.
    pub available_signer_addresses: Vec<String>,
    /// Transaction relay transport forwarded to the wallet adapter via
    /// [`WalletExecutionContext`] on every `sign_and_send` (mirror of the TS
    /// executor passing `transactionTransport` in the wallet context).
    pub transaction_transport: Option<Arc<dyn crate::transactions::TransactionTransport>>,
}

impl fmt::Debug for ExecutionHost<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExecutionHost")
            .field("wallet", &self.wallet.map(|wallet| wallet.public_key()))
            .field(
                "available_signer_addresses",
                &self.available_signer_addresses,
            )
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

/// Event passed to execution observers.
///
/// `receipt` is `None` for transaction-start events and `Some` for
/// transaction-success events.
#[derive(Debug, Clone)]
pub struct OperationExecutionEvent<'a> {
    /// The operation being executed.
    pub operation: &'a PreparedOperation,
    /// The transaction being executed.
    pub transaction: &'a PreparedTransactionBody,
    /// Zero-based index of the transaction within the operation plan.
    pub transaction_index: usize,
    /// The transaction receipt (success events only).
    pub receipt: Option<&'a OperationTransactionReceipt>,
}

/// Synchronous execution observer.
///
/// Divergence from TS: callbacks are plain synchronous `Fn`s that observe and
/// never alter outcomes. The TS executor awaits async callbacks and collects
/// their failures into `callbackErrors`; the Rust executor does neither —
/// callback panics are not caught, so callbacks must not panic.
pub type OperationCallback = Arc<dyn Fn(&OperationExecutionEvent<'_>) + Send + Sync>;

/// Options for [`execute_prepared_operation`].
#[derive(Clone, Default)]
pub struct ExecuteOptions {
    /// Send options forwarded to the wallet adapter.
    pub send: SendOptions,
    /// Registry whose addresses count toward signer validation.
    pub signer_registry: Option<Arc<SignerRegistry>>,
    /// Additional addresses that count toward signer validation.
    pub available_signer_addresses: Vec<String>,
    /// Observer invoked before each transaction is dispatched.
    pub on_transaction_start: Option<OperationCallback>,
    /// Observer invoked after each transaction confirms.
    pub on_transaction_success: Option<OperationCallback>,
}

impl fmt::Debug for ExecuteOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExecuteOptions")
            .field("send", &self.send)
            .field("signer_registry", &self.signer_registry)
            .field(
                "available_signer_addresses",
                &self.available_signer_addresses,
            )
            .field(
                "on_transaction_start",
                &self.on_transaction_start.as_ref().map(|_| "Fn"),
            )
            .field(
                "on_transaction_success",
                &self.on_transaction_success.as_ref().map(|_| "Fn"),
            )
            .finish()
    }
}

/// Failure of [`execute_prepared_operation`], carrying which transaction
/// failed, receipts for the transactions that had already completed, and the
/// structured failure outcome.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error(
    "Operation '{operation_name}' failed at transaction {n} ({failed_transaction_name}): {detail}",
    n = .failed_transaction_index + 1,
    detail = .outcome.message()
)]
pub struct OperationExecutionError {
    /// Name of the failed operation.
    pub operation_name: String,
    /// Zero-based index of the failed transaction within the plan.
    pub failed_transaction_index: usize,
    /// Name of the failed transaction.
    pub failed_transaction_name: String,
    /// Receipts for transactions that completed before the failure.
    pub completed_receipts: Vec<OperationTransactionReceipt>,
    /// Structured failure outcome.
    pub outcome: TransactionFailureOutcome,
}

impl OperationExecutionError {
    /// Signature of the failed transaction, when it reached the network.
    pub fn signature(&self) -> Option<&str> {
        self.outcome.signature()
    }

    /// Slot associated with the failure, if known.
    pub fn slot(&self) -> Option<u64> {
        self.outcome.slot()
    }
}

fn missing_signers(
    transaction: &PreparedTransactionBody,
    host: &ExecutionHost<'_>,
    options: &ExecuteOptions,
) -> Vec<String> {
    let mut available: Vec<&str> = Vec::new();
    let mut owned: Vec<String> = Vec::new();
    available.extend(
        options
            .available_signer_addresses
            .iter()
            .map(String::as_str),
    );
    available.extend(host.available_signer_addresses.iter().map(String::as_str));
    if let Some(registry) = &options.signer_registry {
        owned.extend(registry.addresses());
    }
    if let Some(wallet) = host.wallet {
        owned.extend(wallet.signer_addresses());
        owned.push(wallet.public_key());
    }
    transaction
        .required_signer_addresses
        .iter()
        .filter(|required| {
            !available.contains(&required.as_str()) && !owned.iter().any(|a| a == *required)
        })
        .cloned()
        .collect()
}

pub(crate) fn classify_wallet_error(
    error: WalletError,
    metadata: &[ErrorMetadata],
) -> TransactionFailureOutcome {
    let mut outcome = error.into_outcome(FailurePhase::Send);
    if let TransactionFailureOutcome::ChainFailed {
        program_error: Some(program_error),
        message,
        ..
    } = &mut outcome
    {
        if let Some(resolved) = parse_program_error(program_error.code, metadata) {
            *message = format_program_error(&resolved);
            *program_error = resolved;
        }
    }
    outcome
}

/// Execute a prepared operation transaction-by-transaction through the host's
/// wallet adapter.
///
/// Per transaction, mirroring the TS executor's order exactly:
///
/// 1. Validate required signers against the union of
///    `options.available_signer_addresses`,
///    `host.available_signer_addresses`, the signer registry's addresses, the
///    wallet's [`signer_addresses`](WalletAdapter::signer_addresses), and the
///    wallet's [`public_key`](WalletAdapter::public_key) — failing closed
///    (`"Missing signer(s) for <tx>: …"`, phase [`FailurePhase::Build`])
///    **before** dispatch.
/// 2. Invoke `on_transaction_start`.
/// 3. `wallet.sign_and_send(...)`.
/// 4. Record the transaction receipt.
/// 5. Invoke `on_transaction_success`.
///
/// On failure the returned [`OperationExecutionError`] carries the receipts
/// completed so far and a [`TransactionFailureOutcome`] classified from the
/// [`WalletError`] (chain-failure program errors are re-resolved against the
/// transaction body's `errors` metadata).
#[allow(clippy::result_large_err)]
pub async fn execute_prepared_operation(
    host: &ExecutionHost<'_>,
    operation: &PreparedOperation,
    options: &ExecuteOptions,
) -> Result<OperationReceipt, OperationExecutionError> {
    let mut receipts: Vec<OperationTransactionReceipt> = Vec::new();
    let fail = |receipts: &[OperationTransactionReceipt],
                transaction_index: usize,
                transaction: &PreparedTransactionBody,
                outcome: TransactionFailureOutcome| {
        OperationExecutionError {
            operation_name: operation.name().to_string(),
            failed_transaction_index: transaction_index,
            failed_transaction_name: transaction.name.clone(),
            completed_receipts: receipts.to_vec(),
            outcome,
        }
    };

    for (transaction_index, transaction) in operation.plan().iter().enumerate() {
        let missing = missing_signers(transaction, host, options);
        if !missing.is_empty() {
            return Err(fail(
                &receipts,
                transaction_index,
                transaction,
                TransactionFailureOutcome::NotSubmitted {
                    phase: FailurePhase::Build,
                    message: format!(
                        "Missing signer(s) for {}: {}",
                        transaction.name,
                        missing.join(", ")
                    ),
                },
            ));
        }

        if let Some(callback) = &options.on_transaction_start {
            callback(&OperationExecutionEvent {
                operation,
                transaction,
                transaction_index,
                receipt: None,
            });
        }

        let Some(wallet) = host.wallet else {
            return Err(fail(
                &receipts,
                transaction_index,
                transaction,
                TransactionFailureOutcome::NotSubmitted {
                    phase: FailurePhase::Wallet,
                    message: format!(
                        "No wallet adapter available to execute operation '{}'",
                        operation.name()
                    ),
                },
            ));
        };

        // Pre-dispatch (contract §2/§3): an explicit version the adapter does
        // not advertise, or a fee option bound to another version, fails here
        // rather than being downgraded or converted by the adapter.
        if let Err(error) = wallet
            .validate_transaction_options(options.send.transaction_version, &options.send.resources)
        {
            return Err(fail(
                &receipts,
                transaction_index,
                transaction,
                TransactionFailureOutcome::NotSubmitted {
                    phase: FailurePhase::Build,
                    message: error.to_string(),
                },
            ));
        }

        let context = WalletExecutionContext::new(host.transaction_transport.clone());
        let result = wallet
            .sign_and_send(&transaction.instructions, &options.send, &context)
            .await
            .map_err(|error| {
                fail(
                    &receipts,
                    transaction_index,
                    transaction,
                    classify_wallet_error(error, &transaction.errors),
                )
            })?;

        receipts.push(OperationTransactionReceipt {
            transaction_index,
            transaction_name: transaction.name.clone(),
            signature: result.signature,
            slot: result.slot,
        });

        if let Some(callback) = &options.on_transaction_success {
            let receipt = receipts.last().expect("receipt just pushed");
            callback(&OperationExecutionEvent {
                operation,
                transaction,
                transaction_index,
                receipt: Some(receipt),
            });
        }
    }

    Ok(OperationReceipt {
        kind: operation.kind(),
        operation_name: operation.name().to_string(),
        artifacts: operation.artifacts().clone(),
        signatures: receipts
            .iter()
            .map(|receipt| receipt.signature.clone())
            .collect(),
        transactions: receipts,
    })
}

// ---------------------------------------------------------------------------
// Description
// ---------------------------------------------------------------------------

/// JSON-safe description of a prepared operation: names, per-transaction
/// signer lists and error metadata, and per-instruction program ids, signer
/// lists, account counts, and data lengths.
///
/// Divergence from the TS `describePreparedOperation`: instruction bodies are
/// summarized (account counts and data lengths) instead of dumping full
/// account metas and data bytes; keys are `snake_case`.
pub fn describe_prepared_operation(operation: &PreparedOperation) -> Value {
    json!({
        "kind": operation.kind().as_str(),
        "name": operation.name(),
        "artifacts": operation.artifacts().clone(),
        "transactions": operation
            .plan()
            .iter()
            .map(|transaction| {
                json!({
                    "name": transaction.name,
                    "required_signer_addresses": transaction.required_signer_addresses,
                    "errors": transaction
                        .errors
                        .iter()
                        .map(|error| {
                            json!({
                                "code": error.code,
                                "name": error.name,
                                "msg": error.msg,
                            })
                        })
                        .collect::<Vec<_>>(),
                    "instruction_count": transaction.instructions.len(),
                    "instructions": transaction
                        .instructions
                        .iter()
                        .map(|instruction| {
                            json!({
                                "program_id": instruction.program_id.to_string(),
                                "signers": instruction
                                    .accounts
                                    .iter()
                                    .filter(|account| account.is_signer)
                                    .map(|account| account.pubkey.to_string())
                                    .collect::<Vec<_>>(),
                                "account_count": instruction.accounts.len(),
                                "data_len": instruction.data.len(),
                            })
                        })
                        .collect::<Vec<_>>(),
                })
            })
            .collect::<Vec<_>>(),
    })
}

// ---------------------------------------------------------------------------
// Unsigned inspection
// ---------------------------------------------------------------------------

/// Result of unsigned prepared-operation inspection (mirror of Python's
/// `OperationInspection`).
#[derive(Debug, Clone, PartialEq)]
pub struct OperationInspection {
    /// JSON description of the inspected operation
    /// ([`describe_prepared_operation`]).
    pub description: Value,
    /// What the adapter reported without signing anything.
    pub transaction: TransactionInspectionResult,
    /// Program error resolved from the inspection error against the
    /// transaction body's IDL metadata, when the error carries a custom code
    /// that matches an entry.
    pub program_error: Option<ProgramError>,
}

/// Failure of [`inspect_prepared_operation`].
///
/// Mirrors Python's two raise paths: the operation itself cannot be inspected
/// ([`OperationError`]), or the wallet could not inspect it ([`WalletError`],
/// carrying a
/// [`TransactionCapabilityError`](crate::wallet::TransactionCapabilityError)
/// source when the adapter lacks the capability or the options were refused).
#[derive(Debug, thiserror::Error)]
pub enum OperationInspectionError {
    /// The operation cannot be inspected at all.
    #[error(transparent)]
    Operation(#[from] OperationError),
    /// The wallet refused or failed the inspection.
    ///
    /// Boxed because `WalletError` carries a whole failure outcome: unboxed it
    /// makes every `Result` in this module 160 bytes wide, which is what
    /// `clippy::result_large_err` objects to. Same treatment the crate already
    /// gives `TransactionError::Transport`.
    #[error(transparent)]
    Wallet(Box<WalletError>),
}

impl From<WalletError> for OperationInspectionError {
    fn from(error: WalletError) -> Self {
        Self::Wallet(Box::new(error))
    }
}

impl From<TransactionCapabilityError> for OperationInspectionError {
    fn from(error: TransactionCapabilityError) -> Self {
        Self::from(WalletError::from(error))
    }
}

/// Best-effort custom program-error code from a raw inspection error value
/// (`{"InstructionError": [0, {"Custom": 6001}]}` and friends).
///
/// Divergence from Python's `extract_program_error_code`: only the error value
/// is walked, never the whole inspection result — the Rust result is typed, so
/// there is nothing else to duck-type.
fn program_error_code(value: &Value) -> Option<u32> {
    match value {
        Value::Object(fields) => {
            for key in ["Custom", "custom", "code"] {
                if let Some(code) = fields.get(key).and_then(Value::as_u64) {
                    return u32::try_from(code).ok();
                }
            }
            fields.values().find_map(program_error_code)
        }
        Value::Array(entries) => entries.iter().find_map(program_error_code),
        _ => None,
    }
}

/// Inspect one prepared instruction/transaction without signing or submitting
/// it (mirror of Python's `inspect_prepared_operation`).
///
/// Nothing here can reach the adapter's signing path: the only adapter call is
/// [`WalletAdapter::inspect_transaction`]. Flows are intentionally
/// unsupported, and an adapter without the inspection capability fails with
/// the structured default error.
pub async fn inspect_prepared_operation(
    wallet: Option<&dyn WalletAdapter>,
    operation: &PreparedOperation,
    options: &TransactionInspectionOptions,
    context: &WalletExecutionContext,
) -> Result<OperationInspection, OperationInspectionError> {
    // Divergence from Python, which also rejects a multi-transaction
    // non-flow operation: only a flow can hold more than one transaction
    // here, so the type system already covers that case.
    let transaction = match operation {
        PreparedOperation::Instruction(op) => &op.transaction,
        PreparedOperation::Transaction(op) => &op.transaction,
        PreparedOperation::Flow(op) => {
            return Err(OperationError::FlowInspection(op.name.clone()).into())
        }
    };
    let Some(wallet) = wallet else {
        return Err(TransactionCapabilityError::InspectionUnsupported.into());
    };
    wallet.validate_transaction_options(options.transaction_version, &options.resources)?;

    let description = describe_prepared_operation(operation);
    let inspection = wallet
        .inspect_transaction(&transaction.instructions, options, context)
        .await?;
    let program_error = inspection
        .error
        .as_ref()
        .and_then(program_error_code)
        .and_then(|code| parse_program_error(code, &transaction.errors));
    Ok(OperationInspection {
        description,
        transaction: inspection,
        program_error,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use super::*;
    use crate::instruction::{BuiltAccountMeta, Pubkey};
    use crate::wallet::{SendResult, TransactionResourceOptions, TransactionVersion};

    fn key(byte: u8) -> Pubkey {
        Pubkey::from([byte; 32])
    }

    fn addr(byte: u8) -> String {
        key(byte).to_string()
    }

    fn meta(byte: u8, is_signer: bool) -> BuiltAccountMeta {
        BuiltAccountMeta {
            pubkey: key(byte),
            is_signer,
            is_writable: false,
        }
    }

    fn instruction(program: u8, accounts: Vec<BuiltAccountMeta>) -> BuiltInstruction {
        BuiltInstruction {
            program_id: key(program),
            accounts,
            data: vec![program],
        }
    }

    fn error_metadata(code: u32, name: &str, msg: &str) -> ErrorMetadata {
        ErrorMetadata {
            code,
            name: name.to_string(),
            msg: msg.to_string(),
        }
    }

    struct MockWallet {
        public_key: String,
        extra_signer_addresses: Vec<String>,
        calls: Mutex<usize>,
        results: Mutex<VecDeque<Result<SendResult, WalletError>>>,
        events: Option<Arc<Mutex<Vec<String>>>>,
    }

    impl MockWallet {
        fn new(results: Vec<Result<SendResult, WalletError>>) -> Self {
            MockWallet {
                public_key: addr(0xAA),
                extra_signer_addresses: Vec::new(),
                calls: Mutex::new(0),
                results: Mutex::new(results.into()),
                events: None,
            }
        }

        fn calls(&self) -> usize {
            *self.calls.lock().unwrap()
        }
    }

    #[async_trait::async_trait]
    impl WalletAdapter for MockWallet {
        fn public_key(&self) -> String {
            self.public_key.clone()
        }

        fn signer_addresses(&self) -> Vec<String> {
            let mut addresses = vec![self.public_key.clone()];
            addresses.extend(self.extra_signer_addresses.iter().cloned());
            addresses
        }

        async fn sign_and_send(
            &self,
            _instructions: &[BuiltInstruction],
            _options: &SendOptions,
            _context: &WalletExecutionContext,
        ) -> Result<SendResult, WalletError> {
            let call = {
                let mut calls = self.calls.lock().unwrap();
                *calls += 1;
                *calls
            };
            if let Some(events) = &self.events {
                events.lock().unwrap().push(format!("send:{call}"));
            }
            self.results.lock().unwrap().pop_front().unwrap_or_else(|| {
                Ok(SendResult {
                    signature: format!("sig-{call}"),
                    slot: None,
                })
            })
        }
    }

    /// Inspection-capable adapter. `sign_and_send` panics: inspection
    /// reaching the signing path at all is a test failure, not a wrong value.
    struct InspectingWallet {
        result: TransactionInspectionResult,
        versions: Option<Vec<TransactionVersion>>,
        inspections: Mutex<Vec<usize>>,
    }

    impl InspectingWallet {
        fn new(result: TransactionInspectionResult) -> Self {
            InspectingWallet {
                result,
                versions: None,
                inspections: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait::async_trait]
    impl WalletAdapter for InspectingWallet {
        fn public_key(&self) -> String {
            addr(0xAA)
        }

        fn supported_transaction_versions(&self) -> Option<&[TransactionVersion]> {
            self.versions.as_deref()
        }

        async fn sign_and_send(
            &self,
            _instructions: &[BuiltInstruction],
            _options: &SendOptions,
            _context: &WalletExecutionContext,
        ) -> Result<SendResult, WalletError> {
            panic!("inspection must never reach the signing path");
        }

        async fn inspect_transaction(
            &self,
            instructions: &[BuiltInstruction],
            _options: &TransactionInspectionOptions,
            _context: &WalletExecutionContext,
        ) -> Result<TransactionInspectionResult, WalletError> {
            self.inspections.lock().unwrap().push(instructions.len());
            Ok(self.result.clone())
        }
    }

    struct AddressSigner(String);

    impl Signer for AddressSigner {
        fn address(&self) -> String {
            self.0.clone()
        }
    }

    // -- construction ------------------------------------------------------

    #[test]
    fn infers_and_dedupes_required_signers() {
        let body = create_prepared_transaction_body(
            "tx",
            vec![
                instruction(1, vec![meta(10, true), meta(11, false), meta(12, true)]),
                instruction(2, vec![meta(10, true), meta(13, true)]),
            ],
            None,
            None,
        )
        .unwrap();
        assert_eq!(
            body.required_signer_addresses,
            vec![addr(10), addr(12), addr(13)]
        );
    }

    #[test]
    fn explicit_required_signers_override_inference() {
        let body = create_prepared_transaction_body(
            "tx",
            vec![instruction(1, vec![meta(10, true)])],
            Some(vec!["alpha".into(), "beta".into(), "alpha".into()]),
            None,
        )
        .unwrap();
        assert_eq!(body.required_signer_addresses, vec!["alpha", "beta"]);
    }

    #[test]
    fn empty_instructions_are_rejected() {
        let error = create_prepared_transaction_body("empty-tx", vec![], None, None).unwrap_err();
        assert_eq!(error, OperationError::EmptyTransaction("empty-tx".into()));
        assert_eq!(
            error.to_string(),
            "Transaction 'empty-tx' must contain at least one item"
        );
    }

    #[test]
    fn empty_flow_is_rejected() {
        let error = create_prepared_flow("empty-flow", vec![], Value::Null).unwrap_err();
        assert_eq!(error, OperationError::EmptyFlow("empty-flow".into()));
        assert_eq!(
            error.to_string(),
            "Flow 'empty-flow' must contain at least one item"
        );
    }

    #[test]
    fn transaction_from_child_operations_inherits_signers_and_errors() {
        let child_instruction = create_prepared_instruction(
            "child-ix",
            instruction(1, vec![meta(10, true)]),
            Value::Null,
            None,
            Some(vec![error_metadata(6000, "First", "first failed")]),
        );
        let child_transaction = create_prepared_transaction(
            "child-tx",
            PreparedTransactionChildren::Instructions(vec![PreparedTransactionInstruction::Built(
                instruction(2, vec![meta(11, true), meta(10, true)]),
            )]),
            Value::Null,
            None,
            Some(vec![error_metadata(6001, "Second", "second failed")]),
        )
        .unwrap();

        let combined = create_prepared_transaction(
            "combined",
            PreparedTransactionChildren::Operations(vec![
                child_instruction.clone().into(),
                child_transaction.clone().into(),
            ]),
            json!({"combined": true}),
            None,
            None,
        )
        .unwrap();

        assert_eq!(
            combined.transaction.instructions,
            vec![
                child_instruction.instruction.clone(),
                child_transaction.transaction.instructions[0].clone(),
            ]
        );
        // Inherited, concatenated in child order, deduplicated.
        assert_eq!(
            combined.transaction.required_signer_addresses,
            vec![addr(10), addr(11)]
        );
        assert_eq!(
            combined.transaction.errors,
            vec![
                error_metadata(6000, "First", "first failed"),
                error_metadata(6001, "Second", "second failed"),
            ]
        );

        // Explicit overrides win.
        let overridden = create_prepared_transaction(
            "overridden",
            PreparedTransactionChildren::Operations(vec![child_instruction.into()]),
            Value::Null,
            Some(vec!["only".into()]),
            Some(vec![]),
        )
        .unwrap();
        assert_eq!(
            overridden.transaction.required_signer_addresses,
            vec!["only"]
        );
        assert!(overridden.transaction.errors.is_empty());
    }

    #[test]
    fn flow_as_child_is_rejected() {
        let flow = create_prepared_flow(
            "inner-flow",
            vec![create_prepared_transaction_body(
                "stage",
                vec![instruction(1, vec![])],
                None,
                None,
            )
            .unwrap()],
            Value::Null,
        )
        .unwrap();
        let error = create_prepared_transaction(
            "outer",
            PreparedTransactionChildren::Operations(vec![flow.into()]),
            Value::Null,
            None,
            None,
        )
        .unwrap_err();
        assert_eq!(
            error,
            OperationError::FlowChild {
                transaction: "outer".into(),
                flow: "inner-flow".into(),
            }
        );
    }

    #[test]
    fn prepend_and_append_transaction_instructions() {
        let base = create_prepared_transaction_body(
            "tx",
            vec![instruction(1, vec![meta(10, true)])],
            None,
            Some(vec![error_metadata(6000, "E", "m")]),
        )
        .unwrap();

        let prepended =
            prepend_transaction_instructions(&base, &[instruction(2, vec![meta(11, true)])])
                .unwrap();
        assert_eq!(
            prepended.instructions,
            vec![
                instruction(2, vec![meta(11, true)]),
                instruction(1, vec![meta(10, true)]),
            ]
        );
        // New signers first, then existing (mirrors TS).
        assert_eq!(
            prepended.required_signer_addresses,
            vec![addr(11), addr(10)]
        );
        assert_eq!(prepended.errors, base.errors);

        let appended = append_transaction_instructions(
            &base,
            &[instruction(3, vec![meta(12, true), meta(10, true)])],
        )
        .unwrap();
        assert_eq!(
            appended.instructions,
            vec![
                instruction(1, vec![meta(10, true)]),
                instruction(3, vec![meta(12, true), meta(10, true)]),
            ]
        );
        assert_eq!(appended.required_signer_addresses, vec![addr(10), addr(12)]);
    }

    #[test]
    fn flow_composition_helpers() {
        let flow = create_prepared_flow(
            "flow",
            vec![create_prepared_transaction_body(
                "first",
                vec![instruction(1, vec![meta(10, true)])],
                None,
                None,
            )
            .unwrap()],
            json!({"flow": true}),
        )
        .unwrap();

        let appended = append_flow_transactions(
            &flow,
            vec![create_prepared_transaction_body(
                "second",
                vec![instruction(2, vec![])],
                None,
                None,
            )
            .unwrap()],
        )
        .unwrap();
        assert_eq!(appended.transactions.len(), 2);
        assert_eq!(appended.transactions[1].name, "second");
        assert_eq!(appended.artifacts, json!({"flow": true}));

        let prepended = prepend_flow_transaction_instructions(
            &appended,
            1,
            &[instruction(3, vec![meta(12, true)])],
        )
        .unwrap();
        assert_eq!(
            prepended.transactions[1].instructions[0],
            instruction(3, vec![meta(12, true)])
        );
        assert_eq!(
            prepended.transactions[1].required_signer_addresses,
            vec![addr(12)]
        );

        let error = prepend_flow_transaction_instructions(&appended, 5, &[]).unwrap_err();
        assert_eq!(
            error,
            OperationError::MissingFlowTransaction {
                flow: "flow".into(),
                index: 5,
            }
        );
    }

    // -- signer registry ---------------------------------------------------

    #[test]
    fn signer_registry_round_trip() {
        let registry = SignerRegistry::new();
        registry
            .register("alpha", Arc::new(AddressSigner("alpha".into())))
            .unwrap();
        registry
            .register_signer(Arc::new(AddressSigner("beta".into())))
            .unwrap();

        assert!(registry.has("alpha"));
        assert_eq!(registry.len(), 2);
        assert_eq!(registry.addresses(), vec!["alpha", "beta"]);
        assert_eq!(registry.get("beta").unwrap().address(), "beta");
        assert_eq!(registry.values().len(), 2);
        assert_eq!(
            registry
                .entries()
                .iter()
                .map(|(a, _)| a.clone())
                .collect::<Vec<_>>(),
            vec!["alpha", "beta"]
        );

        assert!(registry.unregister("alpha"));
        assert!(!registry.unregister("alpha"));
        assert!(!registry.has("alpha"));

        registry.clear();
        assert!(registry.is_empty());

        assert_eq!(
            registry
                .register("", Arc::new(AddressSigner(String::new())))
                .unwrap_err(),
            OperationError::EmptySignerAddress
        );
    }

    // -- execution ---------------------------------------------------------

    #[tokio::test]
    async fn execute_happy_path_with_callback_ordering() {
        let events: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let mut wallet = MockWallet::new(vec![
            Ok(SendResult {
                signature: "first-signature".into(),
                slot: Some(10),
            }),
            Ok(SendResult {
                signature: "second-signature".into(),
                slot: Some(11),
            }),
        ]);
        wallet.events = Some(Arc::clone(&events));

        let operation: PreparedOperation = create_prepared_flow(
            "happy-flow",
            vec![
                create_prepared_transaction_body(
                    "first",
                    vec![instruction(
                        1,
                        vec![BuiltAccountMeta {
                            pubkey: key(0xAA),
                            is_signer: true,
                            is_writable: true,
                        }],
                    )],
                    None,
                    None,
                )
                .unwrap(),
                create_prepared_transaction_body(
                    "second",
                    vec![instruction(2, vec![])],
                    None,
                    None,
                )
                .unwrap(),
            ],
            json!({"flow": "artifacts"}),
        )
        .unwrap()
        .into();

        let start_events = Arc::clone(&events);
        let success_events = Arc::clone(&events);
        let options = ExecuteOptions {
            on_transaction_start: Some(Arc::new(move |event| {
                assert!(event.receipt.is_none());
                start_events.lock().unwrap().push(format!(
                    "start:{}:{}",
                    event.transaction_index, event.transaction.name
                ));
            })),
            on_transaction_success: Some(Arc::new(move |event| {
                let receipt = event.receipt.expect("success event carries receipt");
                success_events.lock().unwrap().push(format!(
                    "success:{}:{}",
                    event.transaction_index, receipt.signature
                ));
            })),
            ..ExecuteOptions::default()
        };

        let host = ExecutionHost {
            wallet: Some(&wallet),
            available_signer_addresses: Vec::new(),
            ..ExecutionHost::default()
        };
        let receipt = execute_prepared_operation(&host, &operation, &options)
            .await
            .unwrap();

        assert_eq!(receipt.kind, OperationKind::Flow);
        assert_eq!(receipt.operation_name, "happy-flow");
        assert_eq!(receipt.artifacts, json!({"flow": "artifacts"}));
        assert_eq!(
            receipt.signatures,
            vec!["first-signature", "second-signature"]
        );
        assert_eq!(
            receipt.transactions,
            vec![
                OperationTransactionReceipt {
                    transaction_index: 0,
                    transaction_name: "first".into(),
                    signature: "first-signature".into(),
                    slot: Some(10),
                },
                OperationTransactionReceipt {
                    transaction_index: 1,
                    transaction_name: "second".into(),
                    signature: "second-signature".into(),
                    slot: Some(11),
                },
            ]
        );
        assert_eq!(receipt.transaction(), &receipt.transactions[0]);
        assert_eq!(
            *events.lock().unwrap(),
            vec![
                "start:0:first",
                "send:1",
                "success:0:first-signature",
                "start:1:second",
                "send:2",
                "success:1:second-signature",
            ]
        );
    }

    #[tokio::test]
    async fn missing_signer_fails_closed_without_dispatch() {
        let wallet = MockWallet::new(vec![]);
        let started = Arc::new(Mutex::new(0usize));
        let started_probe = Arc::clone(&started);

        let operation: PreparedOperation = create_prepared_instruction(
            "needs-signer",
            instruction(1, vec![meta(10, true)]),
            Value::Null,
            None,
            None,
        )
        .into();

        let host = ExecutionHost {
            wallet: Some(&wallet),
            available_signer_addresses: Vec::new(),
            ..ExecutionHost::default()
        };
        let options = ExecuteOptions {
            on_transaction_start: Some(Arc::new(move |_| {
                *started_probe.lock().unwrap() += 1;
            })),
            ..ExecuteOptions::default()
        };

        let error = execute_prepared_operation(&host, &operation, &options)
            .await
            .unwrap_err();

        assert_eq!(wallet.calls(), 0, "wallet must not be called");
        assert_eq!(
            *started.lock().unwrap(),
            0,
            "start callback fires after validation"
        );
        assert_eq!(error.operation_name, "needs-signer");
        assert_eq!(error.failed_transaction_index, 0);
        assert!(error.completed_receipts.is_empty());
        assert_eq!(
            error.outcome,
            TransactionFailureOutcome::NotSubmitted {
                phase: FailurePhase::Build,
                message: format!("Missing signer(s) for needs-signer: {}", addr(10)),
            }
        );
        assert_eq!(
            error.to_string(),
            format!(
                "Operation 'needs-signer' failed at transaction 1 (needs-signer): Missing signer(s) for needs-signer: {}",
                addr(10)
            )
        );
    }

    #[tokio::test]
    async fn signer_availability_union_covers_registry_wallet_and_options() {
        let mut wallet = MockWallet::new(vec![]);
        wallet.extra_signer_addresses = vec![addr(11)];

        let registry = Arc::new(SignerRegistry::new());
        registry
            .register(addr(12), Arc::new(AddressSigner(addr(12))))
            .unwrap();

        let operation: PreparedOperation = create_prepared_transaction(
            "well-signed",
            PreparedTransactionChildren::Instructions(vec![PreparedTransactionInstruction::Built(
                instruction(
                    1,
                    vec![
                        meta(0xAA, true),
                        meta(11, true),
                        meta(12, true),
                        meta(13, true),
                        meta(14, true),
                    ],
                ),
            )]),
            Value::Null,
            None,
            None,
        )
        .unwrap()
        .into();

        let host = ExecutionHost {
            wallet: Some(&wallet),
            available_signer_addresses: vec![addr(13)],
            ..ExecutionHost::default()
        };
        let options = ExecuteOptions {
            signer_registry: Some(Arc::clone(&registry)),
            available_signer_addresses: vec![addr(14)],
            ..ExecuteOptions::default()
        };

        let receipt = execute_prepared_operation(&host, &operation, &options)
            .await
            .unwrap();
        assert_eq!(receipt.signatures, vec!["sig-1"]);
        assert_eq!(wallet.calls(), 1);
    }

    #[tokio::test]
    async fn mid_flow_failure_preserves_completed_receipts() {
        let wallet = MockWallet::new(vec![
            Ok(SendResult {
                signature: "first-signature".into(),
                slot: Some(10),
            }),
            Err(WalletError::from_outcome(
                TransactionFailureOutcome::ChainFailed {
                    signature: Some("second-signature".into()),
                    slot: Some(11),
                    program_error: None,
                    message: "second transaction failed".into(),
                },
            )),
        ]);

        let operation: PreparedOperation = create_prepared_flow(
            "partial-flow",
            vec![
                create_prepared_transaction_body("first", vec![instruction(1, vec![])], None, None)
                    .unwrap(),
                create_prepared_transaction_body(
                    "second",
                    vec![instruction(2, vec![])],
                    None,
                    None,
                )
                .unwrap(),
            ],
            Value::Null,
        )
        .unwrap()
        .into();

        let host = ExecutionHost {
            wallet: Some(&wallet),
            available_signer_addresses: Vec::new(),
            ..ExecutionHost::default()
        };
        let error = execute_prepared_operation(&host, &operation, &ExecuteOptions::default())
            .await
            .unwrap_err();

        assert_eq!(wallet.calls(), 2);
        assert_eq!(error.failed_transaction_index, 1);
        assert_eq!(error.failed_transaction_name, "second");
        assert_eq!(
            error.completed_receipts,
            vec![OperationTransactionReceipt {
                transaction_index: 0,
                transaction_name: "first".into(),
                signature: "first-signature".into(),
                slot: Some(10),
            }]
        );
        assert_eq!(error.signature(), Some("second-signature"));
        assert_eq!(error.slot(), Some(11));
    }

    #[tokio::test]
    async fn chain_failure_program_error_resolves_against_metadata() {
        let wallet = MockWallet::new(vec![Err(WalletError::from_outcome(
            TransactionFailureOutcome::ChainFailed {
                signature: Some("ore-signature".into()),
                slot: Some(123),
                program_error: Some(ProgramError::unknown(6000)),
                message: "transaction failed".into(),
            },
        ))]);

        let operation: PreparedOperation = create_prepared_instruction(
            "ore-deploy",
            instruction(1, vec![]),
            Value::Null,
            None,
            Some(vec![error_metadata(
                6000,
                "OreProgramError",
                "ORE deploy failed",
            )]),
        )
        .into();

        let host = ExecutionHost {
            wallet: Some(&wallet),
            available_signer_addresses: Vec::new(),
            ..ExecutionHost::default()
        };
        let error = execute_prepared_operation(&host, &operation, &ExecuteOptions::default())
            .await
            .unwrap_err();

        assert_eq!(
            error.outcome,
            TransactionFailureOutcome::ChainFailed {
                signature: Some("ore-signature".into()),
                slot: Some(123),
                program_error: Some(ProgramError {
                    code: 6000,
                    name: "OreProgramError".into(),
                    message: "ORE deploy failed".into(),
                }),
                message: "OreProgramError (6000): ORE deploy failed".into(),
            }
        );
        assert_eq!(
            error.outcome.program_error().unwrap().to_string(),
            "OreProgramError (6000): ORE deploy failed"
        );
    }

    #[tokio::test]
    async fn unclassified_wallet_error_is_not_submitted_in_send_phase() {
        let wallet = MockWallet::new(vec![Err(WalletError::new("connection reset"))]);
        let operation: PreparedOperation = create_prepared_instruction(
            "plain-failure",
            instruction(1, vec![]),
            Value::Null,
            None,
            None,
        )
        .into();

        let host = ExecutionHost {
            wallet: Some(&wallet),
            available_signer_addresses: Vec::new(),
            ..ExecutionHost::default()
        };
        let error = execute_prepared_operation(&host, &operation, &ExecuteOptions::default())
            .await
            .unwrap_err();

        assert_eq!(
            error.outcome,
            TransactionFailureOutcome::NotSubmitted {
                phase: FailurePhase::Send,
                message: "connection reset".into(),
            }
        );
    }

    #[tokio::test]
    async fn missing_wallet_is_classified_as_wallet_phase() {
        let operation: PreparedOperation = create_prepared_instruction(
            "no-wallet",
            instruction(1, vec![]),
            Value::Null,
            None,
            None,
        )
        .into();
        let host = ExecutionHost::default();
        let error = execute_prepared_operation(&host, &operation, &ExecuteOptions::default())
            .await
            .unwrap_err();
        assert_eq!(
            error.outcome,
            TransactionFailureOutcome::NotSubmitted {
                phase: FailurePhase::Wallet,
                message: "No wallet adapter available to execute operation 'no-wallet'".into(),
            }
        );
    }

    // -- outcome model & description --------------------------------------

    #[test]
    fn parse_and_format_program_errors() {
        let metadata = [error_metadata(
            6000,
            "SlippageExceeded",
            "Slippage tolerance exceeded",
        )];
        let parsed = parse_program_error(6000, &metadata).unwrap();
        assert_eq!(
            format_program_error(&parsed),
            "SlippageExceeded (6000): Slippage tolerance exceeded"
        );
        assert_eq!(parse_program_error(9999, &metadata), None);
        assert_eq!(
            ProgramError::unknown(9999).to_string(),
            "CustomError9999 (9999): Unknown error with code 9999"
        );
    }

    #[test]
    fn outcome_accessors() {
        let confirmed = TransactionOutcome::Confirmed {
            signature: "sig".into(),
            slot: Some(1),
        };
        assert!(matches!(confirmed, TransactionOutcome::Confirmed { .. }));

        let failure = TransactionFailureOutcome::SubmittedUnknown {
            signature: "sig".into(),
            slot: None,
            message: "unknown".into(),
        };
        assert_eq!(failure.phase(), FailurePhase::Confirmation);
        assert_eq!(failure.signature(), Some("sig"));
        assert_eq!(failure.slot(), None);
        assert_eq!(failure.message(), "unknown");
        assert!(failure.program_error().is_none());
    }

    #[test]
    fn describe_prepared_operation_is_json_safe() {
        let operation: PreparedOperation = create_prepared_instruction(
            "describe-me",
            instruction(7, vec![meta(10, true), meta(11, false)]),
            json!({"amount": "1"}),
            None,
            Some(vec![error_metadata(6000, "E", "m")]),
        )
        .into();

        let description = describe_prepared_operation(&operation);
        assert_eq!(description["kind"], "instruction");
        assert_eq!(description["name"], "describe-me");
        assert_eq!(description["artifacts"], json!({"amount": "1"}));
        let transaction = &description["transactions"][0];
        assert_eq!(transaction["name"], "describe-me");
        assert_eq!(transaction["required_signer_addresses"], json!([addr(10)]));
        assert_eq!(
            transaction["errors"],
            json!([{"code": 6000, "name": "E", "msg": "m"}])
        );
        assert_eq!(transaction["instruction_count"], 1);
        let ix = &transaction["instructions"][0];
        assert_eq!(ix["program_id"], addr(7));
        assert_eq!(ix["signers"], json!([addr(10)]));
        assert_eq!(ix["account_count"], 2);
        assert_eq!(ix["data_len"], 1);
    }

    #[tokio::test]
    async fn inspection_never_reaches_sign_and_send() {
        let wallet = InspectingWallet::new(TransactionInspectionResult {
            fee_lamports: Some(5_000),
            logs: Some(vec!["Program log: ok".to_string()]),
            compute_units_consumed: Some(1_200),
            loaded_accounts_data_size: Some(65_536),
            context_slot: Some(42),
            error: Some(json!({ "InstructionError": [0, { "Custom": 6000 }] })),
            extra: Default::default(),
        });
        let operation: PreparedOperation = create_prepared_instruction(
            "inspect-me",
            instruction(1, vec![meta(10, true)]),
            Value::Null,
            None,
            Some(vec![error_metadata(
                6000,
                "OreProgramError",
                "ORE deploy failed",
            )]),
        )
        .into();

        let inspection = inspect_prepared_operation(
            Some(&wallet),
            &operation,
            &TransactionInspectionOptions::default(),
            &WalletExecutionContext::default(),
        )
        .await
        .unwrap();

        assert_eq!(*wallet.inspections.lock().unwrap(), vec![1]);
        assert_eq!(inspection.description["name"], json!("inspect-me"));
        assert_eq!(
            inspection.transaction.loaded_accounts_data_size,
            Some(65_536)
        );
        assert_eq!(inspection.transaction.compute_units_consumed, Some(1_200));
        assert_eq!(inspection.transaction.fee_lamports, Some(5_000));
        assert_eq!(inspection.transaction.context_slot, Some(42));
        assert_eq!(
            inspection.program_error,
            Some(ProgramError {
                code: 6000,
                name: "OreProgramError".into(),
                message: "ORE deploy failed".into(),
            })
        );
    }

    #[tokio::test]
    async fn inspection_without_adapter_capability_is_a_typed_error() {
        let operation: PreparedOperation = create_prepared_instruction(
            "inspect-me",
            instruction(1, vec![]),
            Value::Null,
            None,
            None,
        )
        .into();
        let wallet = MockWallet::new(Vec::new());

        for adapter in [None, Some(&wallet as &dyn WalletAdapter)] {
            let error = inspect_prepared_operation(
                adapter,
                &operation,
                &TransactionInspectionOptions::default(),
                &WalletExecutionContext::default(),
            )
            .await
            .unwrap_err();
            let OperationInspectionError::Wallet(error) = error else {
                panic!("expected a wallet capability error");
            };
            assert_eq!(
                error.message(),
                "Wallet adapter does not support unsigned transaction inspection"
            );
        }
        assert_eq!(wallet.calls(), 0);
    }

    #[tokio::test]
    async fn inspection_rejects_flows() {
        let wallet = InspectingWallet::new(TransactionInspectionResult::default());
        let body = create_prepared_transaction_body("tx", vec![instruction(1, vec![])], None, None)
            .unwrap();
        let flow: PreparedOperation =
            create_prepared_flow("many", vec![body.clone(), body], Value::Null)
                .unwrap()
                .into();

        let error = inspect_prepared_operation(
            Some(&wallet),
            &flow,
            &TransactionInspectionOptions::default(),
            &WalletExecutionContext::default(),
        )
        .await
        .unwrap_err();
        assert!(matches!(
            error,
            OperationInspectionError::Operation(OperationError::FlowInspection(name)) if name == "many"
        ));
        assert!(wallet.inspections.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn transaction_v1_inspection_rejects_undeclared_version() {
        let mut wallet = InspectingWallet::new(TransactionInspectionResult::default());
        wallet.versions = Some(vec![TransactionVersion::V0]);
        let operation: PreparedOperation = create_prepared_instruction(
            "inspect-me",
            instruction(1, vec![]),
            Value::Null,
            None,
            None,
        )
        .into();

        let error = inspect_prepared_operation(
            Some(&wallet),
            &operation,
            &TransactionInspectionOptions {
                transaction_version: Some(TransactionVersion::V1),
                ..TransactionInspectionOptions::default()
            },
            &WalletExecutionContext::default(),
        )
        .await
        .unwrap_err();
        assert!(error
            .to_string()
            .contains("does not support transaction version 1"));
        assert!(wallet.inspections.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn transaction_v1_send_is_refused_before_dispatch() {
        let wallet = MockWallet::new(Vec::new());
        let operation: PreparedOperation =
            create_prepared_instruction("send-me", instruction(1, vec![]), Value::Null, None, None)
                .into();
        let host = ExecutionHost {
            wallet: Some(&wallet),
            available_signer_addresses: Vec::new(),
            ..ExecutionHost::default()
        };

        // Explicit V1 against an adapter that has not declared V1 support.
        let error = execute_prepared_operation(
            &host,
            &operation,
            &ExecuteOptions {
                send: SendOptions {
                    transaction_version: Some(TransactionVersion::V1),
                    ..SendOptions::default()
                },
                ..ExecuteOptions::default()
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(
            &error.outcome,
            TransactionFailureOutcome::NotSubmitted {
                phase: FailurePhase::Build,
                message,
            } if message.contains("does not support transaction version 1")
        ));

        // A V1-only fee option against the default (v0) version.
        let error = execute_prepared_operation(
            &host,
            &operation,
            &ExecuteOptions {
                send: SendOptions {
                    resources: TransactionResourceOptions {
                        priority_fee_lamports: Some(5_000),
                        ..TransactionResourceOptions::default()
                    },
                    ..SendOptions::default()
                },
                ..ExecuteOptions::default()
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(
            &error.outcome,
            TransactionFailureOutcome::NotSubmitted {
                phase: FailurePhase::Build,
                message,
            } if message.contains("'priorityFeeLamports' requires transaction version 1")
        ));

        assert_eq!(wallet.calls(), 0);
    }
}
