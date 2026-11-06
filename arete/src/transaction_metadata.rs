//! Preserve the transaction observations retained by Shipstern.

use arete_interpreter::{
    SolanaTransactionConfig, SolanaTransactionMetadata, SolanaTransactionVersion, UpdateContext,
};
use shipstern_core::instruction::{InstructionShared, InstructionUpdate, Path};

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

/// The log lines this instruction's own program emitted, each with its
/// absolute index into the transaction's logs.
///
/// An instruction's `log_range` spans its whole invocation, including every
/// line logged by the programs it calls. Those programs choose their own
/// `Program data:` / `Program log:` output, so reading them as this program's
/// events would let any CPI target forge them. Lines logged while a deeper
/// invocation is active are skipped here; a nested invocation of the same
/// program is visited as an instruction of its own and reports its lines then.
///
/// Equivalent to `InstructionUpdate::direct_log_messages`, which does not
/// expose the line positions that identify each event occurrence.
pub fn own_log_lines(update: &InstructionUpdate) -> impl Iterator<Item = (usize, &str)> {
    let start = update.log_range.start;
    let mut depth: u32 = 0;
    update
        .log_messages()
        .iter()
        .enumerate()
        .filter_map(move |(offset, line)| {
            let own = match log_line_kind(line) {
                LogLineKind::Invoke => {
                    depth += 1;
                    depth == 1
                }
                LogLineKind::Close => {
                    let own = depth == 1;
                    depth = depth.saturating_sub(1);
                    own
                }
                LogLineKind::Other => depth == 1,
            };
            own.then_some((start + offset, line.as_str()))
        })
}

enum LogLineKind {
    Invoke,
    Close,
    Other,
}

/// Structural classification of a runtime log line. Program output always
/// carries a `log:` / `data:` / `return:` tag, so it can never read as an
/// invoke or close line, whatever its payload.
fn log_line_kind(line: &str) -> LogLineKind {
    let Some((program, tail)) = line
        .strip_prefix("Program ")
        .and_then(|rest| rest.split_once(' '))
    else {
        return LogLineKind::Other;
    };
    if matches!(program, "log:" | "data:" | "return:") {
        return LogLineKind::Other;
    }
    if let Some(depth) = tail
        .strip_prefix("invoke [")
        .and_then(|rest| rest.split(']').next())
    {
        if !depth.is_empty() && depth.bytes().all(|b| b.is_ascii_digit()) {
            return LogLineKind::Invoke;
        }
    }
    if tail == "success" || tail.starts_with("failed:") {
        return LogLineKind::Close;
    }
    LogLineKind::Other
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use shipstern_core::{instruction::InstructionShared, KeyBytes};

    use super::*;

    fn update(
        shared: &Arc<InstructionShared>,
        path: Vec<u32>,
        log_range: std::ops::Range<usize>,
    ) -> InstructionUpdate {
        InstructionUpdate {
            program: KeyBytes::new([1; 32]),
            accounts: vec![],
            data: vec![],
            shared: Arc::clone(shared),
            inner: vec![],
            path: Path::from(path),
            log_range,
        }
    }

    fn shared(logs: &[&str]) -> Arc<InstructionShared> {
        Arc::new(InstructionShared {
            log_messages: logs.iter().map(|line| line.to_string()).collect(),
            ..InstructionShared::default()
        })
    }

    fn own(update: &InstructionUpdate) -> Vec<(usize, &str)> {
        own_log_lines(update).collect()
    }

    #[test]
    fn nested_program_data_is_not_attributed_to_the_caller() {
        let shared = shared(&[
            "Program Other111 invoke [1]",
            "Program Other111 success",
            "Program Indexed11 invoke [1]",
            "Program data: GENUINE",
            "Program Hook11111 invoke [2]",
            "Program data: SPOOFED",
            "Program log: ray_log: SPOOFED",
            "Program Hook11111 success",
            "Program data: GENUINE_AFTER",
            "Program Indexed11 success",
        ]);
        let outer = update(&shared, vec![1], 2..10);

        assert_eq!(
            own(&outer),
            vec![
                (2, "Program Indexed11 invoke [1]"),
                (3, "Program data: GENUINE"),
                (8, "Program data: GENUINE_AFTER"),
                (9, "Program Indexed11 success"),
            ]
        );
        assert!(own(&outer)
            .iter()
            .all(|(_, line)| !line.contains("SPOOFED")));
    }

    #[test]
    fn invoked_instruction_reports_its_own_lines_at_absolute_positions() {
        let shared = shared(&[
            "Program Router111 invoke [1]",
            "Program Indexed11 invoke [2]",
            "Program data: GENUINE",
            "Program Hook11111 invoke [3]",
            "Program data: SPOOFED",
            "Program Hook11111 failed: custom program error: 0x1",
            "Program Indexed11 success",
            "Program Router111 success",
        ]);
        let inner = update(&shared, vec![0, 0], 1..7);

        assert_eq!(
            own(&inner),
            vec![
                (1, "Program Indexed11 invoke [2]"),
                (2, "Program data: GENUINE"),
                (6, "Program Indexed11 success"),
            ]
        );
    }

    #[test]
    fn self_invocation_lines_belong_to_the_inner_instruction_only() {
        let shared = shared(&[
            "Program Indexed11 invoke [1]",
            "Program Indexed11 invoke [2]",
            "Program data: SELF_CPI",
            "Program Indexed11 success",
            "Program Indexed11 success",
        ]);
        let outer = update(&shared, vec![0], 0..5);
        let inner = update(&shared, vec![0, 0], 1..4);

        assert!(own(&outer)
            .iter()
            .all(|(_, line)| !line.contains("SELF_CPI")));
        assert!(own(&inner).contains(&(2, "Program data: SELF_CPI")));
    }

    #[test]
    fn program_output_cannot_fake_invocation_boundaries() {
        let shared = shared(&[
            "Program Indexed11 invoke [1]",
            "Program log: Program Fake111 invoke [2]",
            "Program data: GENUINE",
            "Program log: Program Fake111 success",
            "Program Hook11111 invoke [2]",
            "Program log: Program Hook11111 success",
            "Program data: SPOOFED",
            "Program Hook11111 success",
            "Program Indexed11 success",
        ]);
        let outer = update(&shared, vec![0], 0..9);
        let lines: Vec<&str> = own(&outer).into_iter().map(|(_, line)| line).collect();

        assert!(lines.contains(&"Program data: GENUINE"));
        assert!(!lines.contains(&"Program data: SPOOFED"));
    }

    #[test]
    fn matches_upstream_direct_log_messages() {
        let shared = shared(&[
            "Program Indexed11 invoke [1]",
            "Program log: before",
            "Program A11111111 invoke [2]",
            "Program B11111111 invoke [3]",
            "Program data: deep",
            "Program B11111111 success",
            "Program return: A11111111 AQ==",
            "Program A11111111 success",
            "Program Indexed11 consumed 100 of 200000 compute units",
            "Program return: Indexed11 Ag==",
            "Program Indexed11 success",
        ]);
        let outer = update(&shared, vec![0], 0..11);

        let ours: Vec<&str> = own(&outer).into_iter().map(|(_, line)| line).collect();
        let upstream: Vec<&str> = outer.direct_log_messages().collect();
        assert_eq!(ours, upstream);
    }
}
