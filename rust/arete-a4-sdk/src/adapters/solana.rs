//! Optional first-party Solana transaction adapter: legacy, v0, and V1.
//!
//! Enabled by the `solana-adapter` cargo feature. Nothing in this module is
//! reachable from the base install, and the core
//! [`Pubkey`](solana_pubkey::Pubkey) type is untouched: upstream types are
//! built at the boundary from canonical 32-byte arrays.
//!
//! What the adapter owns (the RPC-free core owns none of it): blockhash
//! fetching, message compilation, signing, one submission, and confirmation —
//! all through the stack's [`TransactionTransport`] relay, never a direct RPC
//! connection.
//!
//! # Version semantics (A4-250 contract §3 + SIMD-0385)
//!
//! - An omitted `transaction_version` keeps the existing v0 default.
//! - legacy/v0 carry the resource budget as `ComputeBudget` program
//!   instructions, prepended in canonical order.
//! - V1 carries it in the message's typed
//!   [`TransactionConfig`](solana_message::v1::TransactionConfig) instead, so
//!   caller-supplied `ComputeBudget` instructions are refused for every
//!   version rather than silently ignored by the runtime.
//! - V1 has no address lookup tables; ALT inputs are refused.
//! - SIMD-0385 makes an **absent** V1 compute-unit limit mean *zero* compute
//!   units (and an absent loaded-accounts limit mean *zero* bytes), not a
//!   generous default like legacy/v0. A final, signable V1 message therefore
//!   has to carry both, so the adapter resolves them in three rungs
//!   ([`SolanaWalletAdapter::resolve_v1_budgets`]): an explicit caller budget
//!   is used verbatim and never raised; a missing one is measured by
//!   simulating the *provisional* message — the same message unsigned
//!   inspection builds, declaring the protocol maxima for what the caller
//!   omitted — and derived from `unitsConsumed` /
//!   `loadedAccountsDataSize` plus headroom; and only a metric the
//!   simulation never reported is refused, naming the budget the caller then
//!   has to supply.
//!
//! # Wire limits
//!
//! 1232 bytes for legacy/v0 ([`MAX_LEGACY_TRANSACTION_BYTES`]), 4096 for V1
//! ([`MAX_V1_TRANSACTION_BYTES`]), checked on the final serialized bytes.
//! V1's 12-signature / 64-account / 64-instruction caps come from upstream
//! [`v1::Message::validate`].
//!
//! # Example
//!
//! See `examples/solana_v1.rs` for a compiling end-to-end V1 send against a
//! local validator relay.

// `WalletError` carries a whole failure outcome, so every `Result` here is
// 160 bytes wide. The trait this module implements fixes the type, so boxing
// internally would only add an unwrap at the boundary.
#![allow(clippy::result_large_err)]

use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use serde_json::Value;
use solana_address::Address;
use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_hash::Hash;
use solana_instruction::{AccountMeta, Instruction};
use solana_message::{v0, v1, Message as LegacyMessage, VersionedMessage};
use solana_signature::Signature;
use solana_signer::Signer;
use solana_transaction::versioned::VersionedTransaction;

use crate::instruction::BuiltInstruction;
use crate::operations::{FailurePhase, ProgramError, TransactionFailureOutcome};
use crate::transactions::{
    Commitment, SignatureStatusOptions, SubmissionState, TransactionError,
    TransactionRequestContext, TransactionSendOptions, TransactionSimulationOptions,
    TransactionTransport,
};
use crate::wallet::{
    ConfirmationLevel, SendOptions, SendResult, TransactionCapabilityError,
    TransactionInspectionOptions, TransactionInspectionResult, TransactionResourceOptions,
    TransactionVersion, WalletAdapter, WalletError, WalletExecutionContext,
};

/// Wire ceiling for legacy and v0 transactions, in bytes (one UDP packet).
pub const MAX_LEGACY_TRANSACTION_BYTES: usize = 1232;

/// Wire ceiling for V1 transactions, in bytes (SIMD-0385).
pub const MAX_V1_TRANSACTION_BYTES: usize = v1::MAX_TRANSACTION_SIZE;

/// Compute-unit limit a *provisional* V1 message declares when the caller
/// supplied none: the runtime per-transaction maximum, so the simulation
/// reports real consumption instead of failing against a zero budget.
pub const PROVISIONAL_COMPUTE_UNIT_LIMIT: u32 = 1_400_000;

/// Loaded-accounts-data ceiling a *provisional* V1 message declares when the
/// caller supplied none: the runtime maximum (64 MiB).
pub const PROVISIONAL_LOADED_ACCOUNTS_DATA_SIZE: u32 = 64 * 1024 * 1024;

/// Headroom added to a *measured* compute-unit consumption, in percent: a
/// simulation is one slot's view of the chain, and the transaction that lands
/// may take a slightly different branch.
pub const ESTIMATED_COMPUTE_UNIT_HEADROOM_PERCENT: u64 = 20;

/// Loaded-account-data page, in bytes. Estimated data budgets are rounded up
/// to a whole page plus one page of headroom, because the runtime accounts
/// for loaded data in pages of this size (SIMD-0385 cost model).
pub const LOADED_ACCOUNTS_DATA_PAGE_BYTES: u32 = 32 * 1024;

/// Versions this adapter builds and therefore advertises (contract §3).
const SUPPORTED_VERSIONS: &[TransactionVersion] = &[
    TransactionVersion::Legacy,
    TransactionVersion::V0,
    TransactionVersion::V1,
];

/// Passthrough keys in `extra` that would carry address lookup tables.
const LOOKUP_TABLE_KEYS: &[&str] = &[
    "addressLookupTables",
    "addressLookupTableAccounts",
    "lookupTables",
];

/// A signer the adapter owns: a local keypair, a remote signer, anything
/// implementing upstream [`Signer`].
pub type SharedSigner = Arc<dyn Signer + Send + Sync>;

/// Configuration for [`SolanaWalletAdapter`].
#[derive(Clone)]
pub struct SolanaAdapterConfig {
    /// Relay used when the executing client passes no transport in the
    /// [`WalletExecutionContext`]. The context transport always wins: it is
    /// the connection that actually owns the session.
    pub transport: Option<Arc<dyn TransactionTransport>>,
    /// Version used when the caller requests none. Defaults to
    /// [`TransactionVersion::DEFAULT`] (v0) — the version first-party
    /// builders already used before V1 existed.
    pub default_version: TransactionVersion,
    /// Confirmation level used when the caller requests none.
    pub default_confirmation_level: ConfirmationLevel,
    /// How long to wait for confirmation before reporting the submission as
    /// unknown. The transaction is never rebuilt, re-signed, or resent.
    pub confirmation_timeout: Duration,
    /// Delay between signature-status polls.
    pub poll_interval: Duration,
}

impl Default for SolanaAdapterConfig {
    fn default() -> Self {
        Self {
            transport: None,
            default_version: TransactionVersion::DEFAULT,
            default_confirmation_level: ConfirmationLevel::Confirmed,
            confirmation_timeout: Duration::from_secs(60),
            poll_interval: Duration::from_millis(500),
        }
    }
}

impl fmt::Debug for SolanaAdapterConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SolanaAdapterConfig")
            .field(
                "transport",
                &self.transport.as_ref().map(|_| "TransactionTransport"),
            )
            .field("default_version", &self.default_version)
            .field(
                "default_confirmation_level",
                &self.default_confirmation_level,
            )
            .field("confirmation_timeout", &self.confirmation_timeout)
            .field("poll_interval", &self.poll_interval)
            .finish()
    }
}

/// [`WalletAdapter`] backed by the upstream Solana message/transaction crates.
///
/// Owns a fee-payer signer plus any number of additional signers, so
/// multi-signature transactions do not need a per-send signer list.
pub struct SolanaWalletAdapter {
    payer: SharedSigner,
    additional_signers: Vec<SharedSigner>,
    config: SolanaAdapterConfig,
}

impl fmt::Debug for SolanaWalletAdapter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SolanaWalletAdapter")
            .field("payer", &self.public_key())
            .field("additional_signers", &self.additional_signers.len())
            .field("config", &self.config)
            .finish()
    }
}

impl SolanaWalletAdapter {
    /// Adapter with the default configuration (v0 default, confirmed, relay
    /// taken from the execution context).
    pub fn new(payer: SharedSigner) -> Self {
        Self::with_config(payer, SolanaAdapterConfig::default())
    }

    /// Adapter with an explicit configuration.
    pub fn with_config(payer: SharedSigner, config: SolanaAdapterConfig) -> Self {
        Self {
            payer,
            additional_signers: Vec::new(),
            config,
        }
    }

    /// Add an owned signer that can satisfy an additional required signature.
    #[must_use]
    pub fn with_signer(mut self, signer: SharedSigner) -> Self {
        self.additional_signers.push(signer);
        self
    }

    /// The active configuration.
    pub fn config(&self) -> &SolanaAdapterConfig {
        &self.config
    }

    /// Fee payer address.
    fn payer_address(&self) -> Address {
        self.payer.pubkey()
    }

    /// Every signer this adapter owns, fee payer first.
    fn owned_signers(&self) -> impl Iterator<Item = &SharedSigner> {
        std::iter::once(&self.payer).chain(self.additional_signers.iter())
    }

    /// Context transport first, configured transport second (contract §5).
    fn transport(
        &self,
        context: &WalletExecutionContext,
    ) -> Result<Arc<dyn TransactionTransport>, WalletError> {
        context
            .transaction_transport
            .clone()
            .or_else(|| self.config.transport.clone())
            .ok_or_else(|| {
                build_failure(
                    "No transaction transport available: pass one in the execution context or \
                     set SolanaAdapterConfig::transport",
                )
            })
    }

    /// Convert built instructions into upstream ones, refusing inputs the
    /// typed configuration owns.
    fn upstream_instructions(
        &self,
        instructions: &[BuiltInstruction],
        version: TransactionVersion,
        resources: &TransactionResourceOptions,
        extra: &serde_json::Map<String, Value>,
    ) -> Result<Vec<Instruction>, WalletError> {
        if instructions.is_empty() {
            return Err(build_failure(
                "A transaction requires at least one instruction",
            ));
        }
        if let Some(key) = LOOKUP_TABLE_KEYS
            .iter()
            .find(|key| extra.contains_key(**key))
        {
            return Err(build_failure(format!(
                "Address lookup tables are not supported by this adapter (option '{key}'); \
                 transaction version 1 has no lookup tables at all (SIMD-0385)"
            )));
        }

        let compute_budget = solana_compute_budget_interface::ID;
        let mut upstream = Vec::with_capacity(instructions.len() + 4);
        upstream.extend(budget_instructions(version, resources));
        for instruction in instructions {
            let program_id = address(&instruction.program_id);
            if program_id == compute_budget {
                return Err(build_failure(
                    "Caller-supplied ComputeBudget instructions are refused: request compute \
                     units, heap size, loaded-accounts size and fees through the typed \
                     `resources` options instead",
                ));
            }
            upstream.push(Instruction {
                program_id,
                accounts: instruction
                    .accounts
                    .iter()
                    .map(|account| AccountMeta {
                        pubkey: address(&account.pubkey),
                        is_signer: account.is_signer,
                        is_writable: account.is_writable,
                    })
                    .collect(),
                data: instruction.data.clone(),
            });
        }
        Ok(upstream)
    }

    /// Compile a message for `version`, at `stage`, against `blockhash`.
    fn compile(
        &self,
        version: TransactionVersion,
        stage: Stage,
        instructions: &[Instruction],
        resources: &TransactionResourceOptions,
        blockhash: Hash,
    ) -> Result<VersionedMessage, WalletError> {
        let payer = self.payer_address();
        let message = match version {
            TransactionVersion::Legacy => VersionedMessage::Legacy(
                LegacyMessage::new_with_blockhash(instructions, Some(&payer), &blockhash),
            ),
            TransactionVersion::V0 => VersionedMessage::V0(
                v0::Message::try_compile(&payer, instructions, &[], blockhash).map_err(
                    |error| build_failure(format!("Failed to compile message: {error}")),
                )?,
            ),
            TransactionVersion::V1 => VersionedMessage::V1(
                v1::Message::try_compile_with_config(
                    &payer,
                    instructions,
                    blockhash,
                    v1_config(resources, stage)?,
                )
                .map_err(|error| build_failure(format!("Failed to compile message: {error}")))?,
            ),
        };
        // V1's caps (12 signatures, 64 accounts, 64 instructions, heap-size
        // bounds) are enforced upstream, before a signer is ever reached.
        if let VersionedMessage::V1(v1_message) = &message {
            v1_message
                .validate()
                .map_err(|error| build_failure(format!("Invalid V1 message: {error}")))?;
        }
        Ok(message)
    }

    /// Resolve the two version-bound V1 budgets a final message must declare.
    ///
    /// Three rungs, in order:
    ///
    /// 1. An explicit caller budget wins verbatim and is never raised — if it
    ///    turns out to be too low, the failure is reported as it happened.
    /// 2. A missing budget is **measured**: the provisional message (maxima
    ///    for what the caller omitted, the caller's own values for the rest)
    ///    is simulated through the same transport with signature
    ///    verification off — the placeholder signatures are zeroed and the
    ///    relay's `simulate` route never asks for `sigVerify` — and the
    ///    budget is derived from the reported metric plus headroom.
    /// 3. A metric the simulation did not report is the only remaining
    ///    refusal: an explicit budget is then required.
    async fn resolve_v1_budgets(
        &self,
        transport: &Arc<dyn TransactionTransport>,
        instructions: &[Instruction],
        resources: &TransactionResourceOptions,
        blockhash: Hash,
        level: ConfirmationLevel,
    ) -> Result<TransactionResourceOptions, WalletError> {
        let mut resolved = *resources;
        if resolved.compute_unit_limit.is_some()
            && resolved.loaded_accounts_data_size_limit.is_some()
        {
            return Ok(resolved);
        }

        let probe = self.unsigned(self.compile(
            TransactionVersion::V1,
            Stage::Provisional,
            instructions,
            resources,
            blockhash,
        )?)?;
        let simulation = transport
            .simulate(
                &probe,
                TransactionSimulationOptions {
                    commitment: Some(commitment_of(level)),
                    ..Default::default()
                },
            )
            .await
            .map_err(|error| {
                send_refused(format!("Budget estimation simulation failed: {error}"))
            })?;
        if let Some(error) = simulation.err {
            return Err(send_refused(format!(
                "Budget estimation simulation reported {error}; nothing was submitted"
            )));
        }

        if resolved.compute_unit_limit.is_none() {
            let units = simulation
                .units_consumed
                .ok_or_else(|| unestimable_v1_option(COMPUTE_UNIT_LIMIT, "unitsConsumed"))?;
            resolved.compute_unit_limit = Some(estimated_compute_unit_limit(units));
        }
        if resolved.loaded_accounts_data_size_limit.is_none() {
            let bytes = simulation.loaded_accounts_data_size.ok_or_else(|| {
                unestimable_v1_option(LOADED_ACCOUNTS_DATA_SIZE_LIMIT, "loadedAccountsDataSize")
            })?;
            resolved.loaded_accounts_data_size_limit =
                Some(estimated_loaded_accounts_data_size(bytes));
        }
        Ok(resolved)
    }

    /// Base64 wire form of an unsigned message with placeholder signatures,
    /// for simulation only: nothing here touches a signer.
    fn unsigned(&self, message: VersionedMessage) -> Result<String, WalletError> {
        let transaction = VersionedTransaction {
            signatures: vec![
                Signature::default();
                usize::from(message.header().num_required_signatures)
            ],
            message,
        };
        let wire = serialize(&transaction)?;
        check_size(wire.len(), version_of(&transaction.message))?;
        Ok(base64::engine::general_purpose::STANDARD.encode(&wire))
    }

    /// The owned signers matching the message's required-signature prefix, in
    /// message order — exactly what [`VersionedTransaction::try_new`] expects.
    fn ordered_signers(
        &self,
        message: &VersionedMessage,
    ) -> Result<Vec<SharedSigner>, WalletError> {
        let available: HashMap<Address, SharedSigner> = self
            .owned_signers()
            .map(|signer| (signer.pubkey(), signer.clone()))
            .collect();
        let required = usize::from(message.header().num_required_signatures);
        let keys = message.static_account_keys();
        let mut ordered = Vec::with_capacity(required);
        let mut missing = Vec::new();
        for key in keys.iter().take(required) {
            match available.get(key) {
                Some(signer) => ordered.push(signer.clone()),
                None => missing.push(key.to_string()),
            }
        }
        if !missing.is_empty() {
            return Err(build_failure(format!(
                "Missing signer(s) for transaction: {}",
                missing.join(", ")
            )));
        }
        Ok(ordered)
    }

    /// Poll the relay until the signature reaches `level`, the lifetime
    /// expires, or the timeout elapses. Never resubmits.
    ///
    /// The whole wait — including whatever request is in flight — is bounded
    /// by [`SolanaAdapterConfig::confirmation_timeout`], so a relay that
    /// stops answering still yields the uncertain-submission outcome instead
    /// of hanging the caller after the one submission.
    async fn confirm(
        &self,
        transport: &Arc<dyn TransactionTransport>,
        signature: &str,
        level: ConfirmationLevel,
        last_valid_block_height: u64,
    ) -> Result<SendResult, WalletError> {
        let polling = self.poll_confirmation(transport, signature, level, last_valid_block_height);
        tokio::time::timeout(self.config.confirmation_timeout, polling)
            .await
            .unwrap_or_else(|_| {
                Err(submitted_unknown(
                    signature,
                    None,
                    format!(
                        "Timed out after {:?} waiting for {level} confirmation; the transaction \
                         was submitted once and may still land",
                        self.config.confirmation_timeout
                    ),
                ))
            })
    }

    /// The unbounded half of [`SolanaWalletAdapter::confirm`]: poll until the
    /// signature reaches `level` or its lifetime expires. Cancelled by the
    /// confirmation deadline, so it needs no clock of its own.
    async fn poll_confirmation(
        &self,
        transport: &Arc<dyn TransactionTransport>,
        signature: &str,
        level: ConfirmationLevel,
        last_valid_block_height: u64,
    ) -> Result<SendResult, WalletError> {
        let options = SignatureStatusOptions {
            search_transaction_history: Some(true),
        };
        loop {
            // A status lookup that fails, or that reports nothing, says
            // nothing about a transaction that is already submitted: keep
            // polling until the deadline, then report it as unknown.
            if let Ok(Some(status)) = transport.signature_status(signature, options).await {
                if let Some(error) = &status.err {
                    return Err(chain_failure(signature, status.slot, error));
                }
                if reached(status.confirmation_status, level) {
                    return Ok(SendResult {
                        signature: signature.to_string(),
                        slot: status.slot,
                    });
                }
            }

            let expired = matches!(
                transport
                    .block_height(TransactionRequestContext {
                        commitment: Some(commitment_of(level)),
                        min_context_slot: None,
                    })
                    .await,
                Ok(height) if height > last_valid_block_height
            );
            if expired {
                return Err(submitted_unknown(
                    signature,
                    None,
                    format!(
                        "Transaction lifetime expired past block height {last_valid_block_height} \
                         without a confirmed status; reconcile by signature rather than resending"
                    ),
                ));
            }
            tokio::time::sleep(self.config.poll_interval).await;
        }
    }
}

#[async_trait::async_trait]
impl WalletAdapter for SolanaWalletAdapter {
    fn public_key(&self) -> String {
        self.payer_address().to_string()
    }

    fn signer_addresses(&self) -> Vec<String> {
        let mut addresses: Vec<String> = Vec::with_capacity(self.additional_signers.len() + 1);
        for signer in self.owned_signers() {
            let address = signer.pubkey().to_string();
            if !addresses.contains(&address) {
                addresses.push(address);
            }
        }
        addresses
    }

    fn supported_transaction_versions(&self) -> Option<&[TransactionVersion]> {
        Some(SUPPORTED_VERSIONS)
    }

    async fn sign_and_send(
        &self,
        instructions: &[BuiltInstruction],
        options: &SendOptions,
        context: &WalletExecutionContext,
    ) -> Result<SendResult, WalletError> {
        // The effective version, not the caller's `Option`, is what the
        // resource options are bound to: validating before it is resolved
        // checks them against the contract default while the message is
        // compiled for this adapter's configured one.
        let version = options
            .transaction_version
            .unwrap_or(self.config.default_version);
        self.validate_transaction_options(Some(version), &options.resources)
            .map_err(capability_failure)?;
        let level = options
            .confirmation_level
            .unwrap_or(self.config.default_confirmation_level);
        let transport = self.transport(context)?;
        let upstream =
            self.upstream_instructions(instructions, version, &options.resources, &options.extra)?;

        let lifetime = transport
            .latest_blockhash(TransactionRequestContext {
                commitment: Some(commitment_of(level)),
                min_context_slot: None,
            })
            .await
            .map_err(|error| build_failure(format!("Failed to fetch a blockhash: {error}")))?;
        let blockhash = Hash::from_str(&lifetime.blockhash).map_err(|error| {
            build_failure(format!("Relay returned an invalid blockhash: {error}"))
        })?;

        // V1 budgets are resolved before the final message exists, so the
        // config that is compiled, sized, simulated and signed is one config.
        let resources = if version == TransactionVersion::V1 {
            self.resolve_v1_budgets(&transport, &upstream, &options.resources, blockhash, level)
                .await?
        } else {
            options.resources
        };
        let message = self.compile(version, Stage::Final, &upstream, &resources, blockhash)?;
        let signers = self.ordered_signers(&message)?;
        let transaction = VersionedTransaction::try_new(message, &signers).map_err(|error| {
            WalletError::from_outcome(TransactionFailureOutcome::NotSubmitted {
                phase: FailurePhase::Wallet,
                message: format!("Failed to sign transaction: {error}"),
            })
        })?;

        let wire = serialize(&transaction)?;
        check_size(wire.len(), version)?;
        let encoded = base64::engine::general_purpose::STANDARD.encode(&wire);
        // Derived locally: an ambiguous submission must still be reconcilable
        // even when the relay's error body carries no signature.
        let derived = transaction
            .signatures
            .first()
            .ok_or_else(|| build_failure("Signed transaction carries no signature"))?
            .to_string();

        if options.skip_preflight != Some(true) {
            let simulation = transport
                .simulate(
                    &encoded,
                    TransactionSimulationOptions {
                        commitment: Some(commitment_of(level)),
                        ..Default::default()
                    },
                )
                .await
                .map_err(|error| send_refused(format!("Preflight simulation failed: {error}")))?;
            if let Some(error) = simulation.err {
                return Err(send_refused(format!(
                    "Preflight simulation reported {error}; nothing was submitted"
                )));
            }
        }

        // One submission. No rebuild, no re-sign, no resend. The locally
        // derived signature is authoritative: it is the one these bytes
        // carry, so a relay echoing a different one is a reportable
        // condition, never a correction to poll for.
        match transport
            .send(
                &encoded,
                TransactionSendOptions {
                    skip_preflight: options.skip_preflight,
                    preflight_commitment: Some(commitment_of(level)),
                    min_context_slot: None,
                },
            )
            .await
        {
            Ok(result) if result.signature == derived => {}
            Ok(result) => {
                tracing::warn!(
                    relay_signature = %result.signature,
                    signed_signature = %derived,
                    "relay acknowledged a submission under a signature the adapter did not sign"
                );
                return Err(submitted_unknown(
                    &derived,
                    None,
                    format!(
                        "Relay acknowledged the submission as signature {} but these bytes were \
                         signed as {derived}; reconcile by the signed signature rather than \
                         resending",
                        result.signature
                    ),
                ));
            }
            Err(error) => return Err(classify_send_error(error, &derived)),
        }

        self.confirm(
            &transport,
            &derived,
            level,
            lifetime.last_valid_block_height,
        )
        .await
    }

    async fn inspect_transaction(
        &self,
        instructions: &[BuiltInstruction],
        options: &TransactionInspectionOptions,
        context: &WalletExecutionContext,
    ) -> Result<TransactionInspectionResult, WalletError> {
        let version = options
            .transaction_version
            .unwrap_or(self.config.default_version);
        self.validate_transaction_options(Some(version), &options.resources)
            .map_err(capability_failure)?;
        let transport = self.transport(context)?;
        let upstream =
            self.upstream_instructions(instructions, version, &options.resources, &options.extra)?;

        let commitment = commitment_of(self.config.default_confirmation_level);
        let lifetime = transport
            .latest_blockhash(TransactionRequestContext {
                commitment: Some(commitment),
                min_context_slot: None,
            })
            .await
            .map_err(|error| build_failure(format!("Failed to fetch a blockhash: {error}")))?;
        let blockhash = Hash::from_str(&lifetime.blockhash).map_err(|error| {
            build_failure(format!("Relay returned an invalid blockhash: {error}"))
        })?;

        let message = self.compile(
            version,
            Stage::Provisional,
            &upstream,
            &options.resources,
            blockhash,
        )?;
        let encoded_message = base64::engine::general_purpose::STANDARD.encode(message.serialize());
        // Placeholder signatures: inspection never touches a signer. V1
        // serializes exactly `num_required_signatures` of them, so the
        // inspected payload has the size and shape of the real one, and the
        // fee is quoted for the same message that is simulated.
        let encoded_transaction = self.unsigned(message)?;

        let fee = transport
            .fee(
                &encoded_message,
                TransactionRequestContext {
                    commitment: Some(commitment),
                    min_context_slot: None,
                },
            )
            .await
            .map_err(|error| build_failure(format!("Failed to estimate the fee: {error}")))?;
        let simulation = transport
            .simulate(
                &encoded_transaction,
                TransactionSimulationOptions {
                    commitment: Some(commitment),
                    ..Default::default()
                },
            )
            .await
            .map_err(|error| build_failure(format!("Failed to simulate: {error}")))?;

        Ok(TransactionInspectionResult {
            fee_lamports: fee.fee_lamports,
            logs: simulation.logs,
            compute_units_consumed: simulation.units_consumed,
            loaded_accounts_data_size: simulation.loaded_accounts_data_size,
            context_slot: Some(simulation.context_slot),
            error: simulation.err,
            extra: serde_json::Map::new(),
        })
    }
}

/// Which build stage a message is compiled for: a provisional message is
/// measured, a final one is signed and submitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Provisional,
    Final,
}

/// Core [`Pubkey`](solana_pubkey::Pubkey) to upstream [`Address`], through
/// canonical bytes: the core type stays on its own version line.
fn address(pubkey: &solana_pubkey::Pubkey) -> Address {
    Address::new_from_array(pubkey.to_bytes())
}

/// The transaction version a compiled message belongs to.
fn version_of(message: &VersionedMessage) -> TransactionVersion {
    match message {
        VersionedMessage::Legacy(_) => TransactionVersion::Legacy,
        VersionedMessage::V0(_) => TransactionVersion::V0,
        VersionedMessage::V1(_) => TransactionVersion::V1,
    }
}

/// `ComputeBudget` instructions for the legacy/v0 resource budget, in
/// canonical order. V1 carries the same budget in its message config, so it
/// gets none.
fn budget_instructions(
    version: TransactionVersion,
    resources: &TransactionResourceOptions,
) -> Vec<Instruction> {
    if version == TransactionVersion::V1 {
        return Vec::new();
    }
    let mut instructions = Vec::new();
    if let Some(limit) = resources.compute_unit_limit {
        instructions.push(ComputeBudgetInstruction::set_compute_unit_limit(limit));
    }
    if let Some(price) = resources.compute_unit_price_micro_lamports {
        instructions.push(ComputeBudgetInstruction::set_compute_unit_price(price));
    }
    if let Some(heap) = resources.heap_size {
        instructions.push(ComputeBudgetInstruction::request_heap_frame(heap));
    }
    if let Some(bytes) = resources.loaded_accounts_data_size_limit {
        instructions.push(ComputeBudgetInstruction::set_loaded_accounts_data_size_limit(bytes));
    }
    instructions
}

/// Canonical wire keys of the two version-bound V1 budgets.
const COMPUTE_UNIT_LIMIT: &str = "computeUnitLimit";
const LOADED_ACCOUNTS_DATA_SIZE_LIMIT: &str = "loadedAccountsDataSizeLimit";

/// The V1 message config for `stage`.
///
/// Under SIMD-0385 an unset bit means the *minimum* value, not a default: a
/// message without a compute-unit limit requests zero compute units and one
/// without a loaded-accounts limit requests zero bytes of account data, so
/// either could only fail on chain. A provisional message therefore declares
/// the protocol maxima for what the caller omitted — that is the message
/// whose simulation measures the real budget — and a final one is compiled
/// from budgets already resolved by
/// [`SolanaWalletAdapter::resolve_v1_budgets`].
fn v1_config(
    resources: &TransactionResourceOptions,
    stage: Stage,
) -> Result<v1::TransactionConfig, WalletError> {
    let mut config = v1::TransactionConfig::empty();
    config.priority_fee = resources.priority_fee_lamports;
    config.heap_size = resources.heap_size;
    config.compute_unit_limit = match (resources.compute_unit_limit, stage) {
        (Some(limit), _) => Some(limit),
        (None, Stage::Provisional) => Some(PROVISIONAL_COMPUTE_UNIT_LIMIT),
        (None, Stage::Final) => return Err(unresolved_v1_option(COMPUTE_UNIT_LIMIT)),
    };
    config.loaded_accounts_data_size_limit =
        match (resources.loaded_accounts_data_size_limit, stage) {
            (Some(limit), _) => Some(limit),
            (None, Stage::Provisional) => Some(PROVISIONAL_LOADED_ACCOUNTS_DATA_SIZE),
            (None, Stage::Final) => {
                return Err(unresolved_v1_option(LOADED_ACCOUNTS_DATA_SIZE_LIMIT))
            }
        };
    Ok(config)
}

/// Measured consumption plus [`ESTIMATED_COMPUTE_UNIT_HEADROOM_PERCENT`],
/// rounded up, positive, and capped at the protocol maximum.
fn estimated_compute_unit_limit(units_consumed: u64) -> u32 {
    let padded = units_consumed
        .saturating_mul(100 + ESTIMATED_COMPUTE_UNIT_HEADROOM_PERCENT)
        .div_ceil(100);
    padded.clamp(1, u64::from(PROVISIONAL_COMPUTE_UNIT_LIMIT)) as u32
}

/// Measured loaded-account data rounded up to a whole page, plus one page of
/// headroom, positive and capped at the protocol maximum.
fn estimated_loaded_accounts_data_size(loaded_bytes: u64) -> u32 {
    let page = u64::from(LOADED_ACCOUNTS_DATA_PAGE_BYTES);
    let bytes = loaded_bytes
        .div_ceil(page)
        .saturating_add(1)
        .saturating_mul(page);
    bytes.clamp(1, u64::from(PROVISIONAL_LOADED_ACCOUNTS_DATA_SIZE)) as u32
}

/// The one budget refusal left: the simulation reported no metric to derive
/// the option from, so only the caller can supply it.
fn unestimable_v1_option(option: &'static str, metric: &'static str) -> WalletError {
    unresolvable(
        option,
        format!(
            "could not be estimated because the simulation reported no `{metric}`. Pass an \
             explicit budget, or use a relay whose simulation reports {metric}"
        ),
    )
}

/// A final message reached compilation with an unresolved budget. Reachable
/// only by compiling [`Stage::Final`] without resolving budgets first.
fn unresolved_v1_option(option: &'static str) -> WalletError {
    unresolvable(
        option,
        "was neither supplied by the caller nor estimated from a simulation".to_string(),
    )
}

fn unresolvable(option: &'static str, reason: String) -> WalletError {
    let error = TransactionCapabilityError::InvalidResourceOption {
        option,
        reason: format!(
            "required for transaction version 1: an omitted value requests the minimum \
             (SIMD-0385), so the transaction could only fail on chain, and this one {reason}"
        ),
    };
    build_failure(error.to_string()).with_source(error)
}

/// Serialize a transaction with the upstream codec (V1-aware: V1 writes its
/// signatures as a fixed-length array after the message, legacy/v0 write a
/// short-vec prefix before it).
fn serialize(transaction: &VersionedTransaction) -> Result<Vec<u8>, WalletError> {
    wincode::serialize(transaction)
        .map_err(|error| build_failure(format!("Failed to serialize transaction: {error}")))
}

/// Enforce the version's wire ceiling on the final bytes.
fn check_size(len: usize, version: TransactionVersion) -> Result<(), WalletError> {
    let limit = match version {
        TransactionVersion::V1 => MAX_V1_TRANSACTION_BYTES,
        TransactionVersion::Legacy | TransactionVersion::V0 => MAX_LEGACY_TRANSACTION_BYTES,
    };
    if len > limit {
        return Err(build_failure(format!(
            "Transaction is {len} bytes, over the {limit}-byte limit for version {version}"
        )));
    }
    Ok(())
}

fn commitment_of(level: ConfirmationLevel) -> Commitment {
    match level {
        ConfirmationLevel::Processed => Commitment::Processed,
        ConfirmationLevel::Confirmed => Commitment::Confirmed,
        ConfirmationLevel::Finalized => Commitment::Finalized,
    }
}

/// Whether a reported commitment satisfies the requested confirmation level.
fn reached(reported: Option<Commitment>, wanted: ConfirmationLevel) -> bool {
    let rank = |commitment| match commitment {
        Commitment::Processed => 1u8,
        Commitment::Confirmed => 2,
        Commitment::Finalized => 3,
    };
    reported.is_some_and(|reported| rank(reported) >= rank(commitment_of(wanted)))
}

fn build_failure(message: impl Into<String>) -> WalletError {
    WalletError::from_outcome(TransactionFailureOutcome::NotSubmitted {
        phase: FailurePhase::Build,
        message: message.into(),
    })
}

fn capability_failure(error: TransactionCapabilityError) -> WalletError {
    build_failure(error.to_string()).with_source(error)
}

fn send_refused(message: impl Into<String>) -> WalletError {
    WalletError::from_outcome(TransactionFailureOutcome::NotSubmitted {
        phase: FailurePhase::Send,
        message: message.into(),
    })
}

fn submitted_unknown(
    signature: &str,
    slot: Option<u64>,
    message: impl Into<String>,
) -> WalletError {
    WalletError::from_outcome(TransactionFailureOutcome::SubmittedUnknown {
        signature: signature.to_string(),
        slot,
        message: message.into(),
    })
}

fn chain_failure(signature: &str, slot: Option<u64>, error: &Value) -> WalletError {
    WalletError::from_outcome(TransactionFailureOutcome::ChainFailed {
        signature: Some(signature.to_string()),
        slot,
        // Left unresolved: the operation executor re-resolves the code
        // against the transaction body's IDL error metadata.
        program_error: custom_program_error(error),
        message: format!("Transaction {signature} failed on chain: {error}"),
    })
}

/// The `Custom(code)` of an `InstructionError`, if the failure carries one.
fn custom_program_error(error: &Value) -> Option<ProgramError> {
    error
        .get("InstructionError")?
        .as_array()?
        .get(1)?
        .get("Custom")?
        .as_u64()
        .and_then(|code| u32::try_from(code).ok())
        .map(ProgramError::unknown)
}

/// Classify a failed submission. Only a relay that proved the transaction was
/// never dispatched yields a not-submitted outcome; anything else keeps the
/// signature so the caller can reconcile instead of resending.
fn classify_send_error(error: TransactionError, derived: &str) -> WalletError {
    let transport = match &error {
        TransactionError::Transport(inner) => Some(inner.as_ref()),
        _ => None,
    };
    let message = format!("Failed to submit transaction: {error}");
    match transport.and_then(|inner| inner.submission_state) {
        Some(SubmissionState::NotSubmitted) => send_refused(message),
        _ => {
            let signature = transport
                .and_then(|inner| inner.signature.as_deref())
                .unwrap_or(derived);
            submitted_unknown(signature, None, message)
        }
    }
}
