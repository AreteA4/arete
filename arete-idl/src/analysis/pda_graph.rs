//! PDA graph analysis — extracts PDA derivation info from IDL instructions.

use crate::types::{IdlPdaSeed, IdlSpec};

/// Classification of a PDA seed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SeedKind {
    /// Constant byte array seed (often a UTF-8 string like "pool", "lb_pair").
    Const,
    /// Reference to another account in the instruction.
    Account,
    /// Reference to an instruction argument.
    Arg,
}

/// A single seed in a PDA derivation.
#[derive(Debug, Clone)]
pub struct PdaSeedInfo {
    pub kind: SeedKind,
    /// For `Const`: UTF-8 decoded string or hex representation.
    /// For `Account`/`Arg`: the path field (e.g. "lb_pair", "base_mint").
    pub value: String,
}

/// A PDA node linking an account, its instruction context, and derivation seeds.
#[derive(Debug, Clone)]
pub struct PdaNode {
    pub account_name: String,
    pub instruction_name: String,
    pub seeds: Vec<PdaSeedInfo>,
}

/// Extract all PDA derivation nodes from an IDL spec.
///
/// Iterates every instruction's account list, collecting accounts that have
/// a `pda` field with seeds. Each seed is classified and its value extracted.
pub fn extract_pda_graph(idl: &IdlSpec) -> Vec<PdaNode> {
    let mut nodes = Vec::new();

    for ix in &idl.instructions {
        for acc in &ix.accounts {
            if let Some(pda) = &acc.pda {
                let seeds = pda.seeds.iter().map(extract_seed_info).collect();

                nodes.push(PdaNode {
                    account_name: acc.name.clone(),
                    instruction_name: ix.name.clone(),
                    seeds,
                });
            }
        }
    }
    nodes
}

/// Extract kind and human-readable value from an `IdlPdaSeed`.
fn extract_seed_info(seed: &IdlPdaSeed) -> PdaSeedInfo {
    match seed {
        IdlPdaSeed::Const { value } => {
            // Try to decode byte array as UTF-8; fall back to hex representation
            let decoded = String::from_utf8(value.clone()).unwrap_or_else(|_| hex_encode(value));
            PdaSeedInfo {
                kind: SeedKind::Const,
                value: decoded,
            }
        }
        IdlPdaSeed::Account { path, .. } => PdaSeedInfo {
            kind: SeedKind::Account,
            value: path.clone(),
        },
        IdlPdaSeed::Arg { path, .. } => PdaSeedInfo {
            kind: SeedKind::Arg,
            value: path.clone(),
        },
    }
}

/// Simple hex encoding for non-UTF-8 byte arrays.
fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse_idl_file;
    use std::path::PathBuf;

    #[test]
    fn test_pda_graph() {
        let path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/meteora_dlmm.json");
        let idl = parse_idl_file(&path).expect("should parse");
        let graph = extract_pda_graph(&idl);

        // meteora_dlmm has many PDA accounts
        assert!(
            !graph.is_empty(),
            "should extract PDA nodes from meteora_dlmm"
        );

        // Check that at least some nodes have seeds
        let with_seeds = graph.iter().filter(|n| !n.seeds.is_empty()).count();
        assert!(with_seeds > 0, "some PDA nodes should have seeds");

        // Verify seed kinds are present
        let has_account_seed = graph
            .iter()
            .flat_map(|n| &n.seeds)
            .any(|s| s.kind == SeedKind::Account);
        let has_arg_seed = graph
            .iter()
            .flat_map(|n| &n.seeds)
            .any(|s| s.kind == SeedKind::Arg);
        let has_const_seed = graph
            .iter()
            .flat_map(|n| &n.seeds)
            .any(|s| s.kind == SeedKind::Const);

        assert!(has_account_seed, "should have Account seeds");
        assert!(has_arg_seed, "should have Arg seeds");
        assert!(has_const_seed, "should have Const seeds");

        // Const seeds should decode to readable strings (e.g. "oracle", "preset_parameter")
        let const_seeds: Vec<&str> = graph
            .iter()
            .flat_map(|n| &n.seeds)
            .filter(|s| s.kind == SeedKind::Const)
            .map(|s| s.value.as_str())
            .collect();
        assert!(
            const_seeds
                .iter()
                .any(|s| s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')),
            "at least one const seed should be a readable ASCII string, got: {:?}",
            const_seeds
        );

        // Print summary for evidence
        println!("PDA graph nodes: {}", graph.len());
        println!("Nodes with seeds: {}", with_seeds);
        println!(
            "Account seeds: {}",
            graph
                .iter()
                .flat_map(|n| &n.seeds)
                .filter(|s| s.kind == SeedKind::Account)
                .count()
        );
        println!(
            "Arg seeds: {}",
            graph
                .iter()
                .flat_map(|n| &n.seeds)
                .filter(|s| s.kind == SeedKind::Arg)
                .count()
        );
        println!(
            "Const seeds: {}",
            graph
                .iter()
                .flat_map(|n| &n.seeds)
                .filter(|s| s.kind == SeedKind::Const)
                .count()
        );
        println!(
            "Sample const values: {:?}",
            &const_seeds[..const_seeds.len().min(10)]
        );

        // Print a few sample nodes
        for node in graph.iter().take(5) {
            println!(
                "  {} in {}: {:?}",
                node.account_name,
                node.instruction_name,
                node.seeds
                    .iter()
                    .map(|s| format!("{:?}={}", s.kind, s.value))
                    .collect::<Vec<_>>()
            );
        }
    }
}
