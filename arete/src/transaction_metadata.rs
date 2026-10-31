//! Preserve the transaction observations retained by Shipstern.

use std::sync::{Arc, Mutex, PoisonError, Weak};

use arete_interpreter::{
    SolanaTransactionConfig, SolanaTransactionMetadata, SolanaTransactionVersion, UpdateContext,
};
use serde_json::Value;
use shipstern_core::instruction::{InstructionShared, InstructionUpdate, Path};
use shipstern_core::Pubkey;

/// Config presence identifies V1, including an empty config. Shipstern discards
/// the source version flag, so absent config must remain unknown.
pub fn observed_transaction_metadata(
    shared: &InstructionShared,
) -> Option<SolanaTransactionMetadata> {
    shared
        .transaction_config
        .as_ref()
        .map(|config| SolanaTransactionMetadata {
            version: SolanaTransactionVersion::V1,
            config: Some(SolanaTransactionConfig {
                priority_fee_lamports: config.priority_fee,
                compute_unit_limit: config.compute_unit_limit,
                loaded_accounts_data_size_limit: config.loaded_accounts_data_size_limit,
                heap_size: config.heap_size,
            }),
        })
}

/// One originating context for the instruction, its hooks/events and continuations.
pub fn instruction_update_context(shared: &InstructionShared) -> UpdateContext {
    let context = UpdateContext::new_instruction(
        shared.slot,
        bs58::encode(&shared.signature).into_string(),
        shared.txn_index,
    );
    match observed_transaction_metadata(shared) {
        Some(metadata) => context.with_solana_transaction(metadata),
        None => context,
    }
}

/// Render an instruction's 0-based tree path as `"0"` / `"0.1"`.
///
/// `Path`'s own `Debug` renders 1-based, so this is spelled out to match the
/// 0-based `ix_path` the runtime publishes.
pub fn instruction_path(path: &Path) -> String {
    path.as_slice()
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(".")
}

/// One originating context located at this instruction's own occurrence.
///
/// The instruction's tree path identifies it. `log_range` is deliberately not
/// used here: a provider that omits or truncates logs leaves it empty for
/// every instruction in the transaction, which would collapse them all onto
/// one identity.
pub fn instruction_occurrence_context(update: &InstructionUpdate) -> UpdateContext {
    instruction_update_context(&update.shared).at_instruction(instruction_path(&update.path))
}

/// This instruction's own log lines, each with its absolute index in the
/// transaction's logs.
///
/// An instruction's log range also spans the logs of the instructions it
/// invokes. A `Program data:` line in there was emitted by the inner
/// instruction, so it is left for that instruction's handler, which decodes it
/// with that instruction's accounts.
pub fn direct_log_lines(update: &InstructionUpdate) -> impl Iterator<Item = (u64, &str)> {
    let mut direct = update.direct_log_messages().peekable();
    let offset = update.log_range.start;
    update
        .log_messages()
        .iter()
        .enumerate()
        .filter_map(move |(position, line)| {
            // `direct_log_messages` filters this same slice in order, so the
            // two walks meet on the identical `&str`.
            direct
                .next_if(|direct| std::ptr::eq(*direct, line.as_str()))
                .map(|line| ((offset + position) as u64, line))
        })
}

/// Accounts of the instruction that emitted each `emit_cpi!` event.
///
/// `emit_cpi!` delivers an event as a self-CPI whose only account is the
/// program's event authority; the accounts a stack wants (pool, mints, user)
/// belong to the instruction that made that CPI. Shipstern dispatches a
/// transaction's instructions in pre-order without linking a child to its
/// parent, so every handled instruction that invokes its own program records
/// its decoded accounts here, and its event children read them back by path.
///
/// A record is only visible to the transaction delivery it came from: it is
/// keyed by that delivery's shared transaction data and dropped with it.
#[derive(Debug, Default)]
pub struct CpiEventAccounts {
    emitters: Mutex<Vec<Emitter>>,
}

#[derive(Debug)]
struct Emitter {
    /// Holding the allocation keeps its address unique while this record lives.
    transaction: Weak<InstructionShared>,
    program: Pubkey,
    path: Path,
    accounts: Value,
}

impl CpiEventAccounts {
    /// Record `update`'s decoded `accounts` when it may emit CPI events, or,
    /// for a CPI event, set `value["accounts"]` to its emitting instruction's.
    ///
    /// An event whose emitting instruction was not handled (its discriminator
    /// is not in the IDL, or it failed to decode) gets no `accounts` and a
    /// warning; it never borrows another instruction's accounts.
    pub fn attach(&self, update: &InstructionUpdate, is_cpi_event: bool, value: &mut Value) {
        let mut emitters = self.emitters.lock().unwrap_or_else(PoisonError::into_inner);
        // ponytail: linear scan over the in-flight transactions' self-invoking
        // instructions; key by transaction if pipeline `jobs` grows large.
        emitters.retain(|emitter| emitter.transaction.strong_count() > 0);

        if !is_cpi_event {
            let invokes_itself = update
                .inner
                .iter()
                .any(|child| child.program == update.program);
            if let (true, Some(accounts)) = (invokes_itself, value.get("accounts")) {
                emitters.push(Emitter {
                    transaction: Arc::downgrade(&update.shared),
                    program: update.program,
                    path: update.path.clone(),
                    accounts: accounts.clone(),
                });
            }
            return;
        }

        let emitter = update.path.as_slice().split_last().and_then(|(_, parent)| {
            emitters.iter().find(|emitter| {
                std::ptr::eq(emitter.transaction.as_ptr(), Arc::as_ptr(&update.shared))
                    && emitter.program == update.program
                    && emitter.path.as_slice() == parent
            })
        });
        match (emitter, value.as_object_mut()) {
            (Some(emitter), Some(object)) => {
                object.insert("accounts".to_string(), emitter.accounts.clone());
            }
            _ => tracing::warn!(
                signature = %bs58::encode(&update.shared.signature).into_string(),
                ix_path = %instruction_path(&update.path),
                "CPI event has no handled emitting instruction; its accounts are unavailable"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn instruction(
        transaction: &Arc<InstructionShared>,
        program: u8,
        path: &[u32],
        inner: Vec<InstructionUpdate>,
    ) -> InstructionUpdate {
        InstructionUpdate {
            program: Pubkey::new([program; 32]),
            accounts: Vec::new(),
            data: Vec::new(),
            shared: Arc::clone(transaction),
            inner,
            path: Path::from(path.to_vec()),
            log_range: 0..0,
        }
    }

    fn event_accounts(accounts: &CpiEventAccounts, event: &InstructionUpdate) -> Option<Value> {
        let mut value = json!({ "data": {} });
        accounts.attach(event, true, &mut value);
        value.get("accounts").cloned()
    }

    /// Two live deliveries share every path, and each has two self-invoking
    /// parents under a foreign router. Each event must read its own parent.
    #[test]
    fn cpi_event_reads_only_its_own_emitting_instruction() {
        let accounts = CpiEventAccounts::default();
        let first = Arc::new(InstructionShared::default());
        let second = Arc::new(InstructionShared::default());
        let mut events = Vec::new();
        for (transaction, tag) in [(&first, "first"), (&second, "second")] {
            for parent in 0..2 {
                let event = instruction(transaction, 7, &[0, parent, 0], Vec::new());
                let emitter = instruction(transaction, 7, &[0, parent], vec![event.clone()]);
                let mut value = json!({ "accounts": { "pool": format!("{tag}-{parent}") } });
                accounts.attach(&emitter, false, &mut value);
                events.push((event, json!({ "pool": format!("{tag}-{parent}") })));
            }
        }
        // A foreign-program event at a recorded parent's child path, and an
        // event under an instruction that was never handled.
        let foreign = instruction(&first, 8, &[0, 0, 0], Vec::new());
        let orphan = instruction(&first, 7, &[1, 0], Vec::new());

        for (event, expected) in &events {
            assert_eq!(event_accounts(&accounts, event).as_ref(), Some(expected));
        }
        assert_eq!(event_accounts(&accounts, &foreign), None);
        assert_eq!(event_accounts(&accounts, &orphan), None);
    }
}
