use crate::analysis::relations::build_account_index;
use crate::search::suggest_similar;
use crate::types::IdlSpec;
use std::collections::HashSet;

#[derive(Debug, Clone)]
pub struct AccountRole {
    pub writable: bool,
    pub signer: bool,
    pub pda: bool,
}

#[derive(Debug, Clone)]
pub struct InstructionContext {
    pub instruction_name: String,
    pub from_role: AccountRole,
    pub to_role: AccountRole,
    pub all_accounts: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct DirectConnection {
    pub from: String,
    pub to: String,
    pub instructions: Vec<InstructionContext>,
}

#[derive(Debug, Clone)]
pub struct TransitiveConnection {
    pub from: String,
    pub intermediary: String,
    pub to: String,
    pub hop1_instruction: String,
    pub hop2_instruction: String,
}

#[derive(Debug, Clone)]
pub struct ConnectionReport {
    pub new_account: String,
    pub direct: Vec<DirectConnection>,
    pub transitive: Vec<TransitiveConnection>,
    pub invalid_existing: Vec<(String, Vec<String>)>,
}

const INFRASTRUCTURE_ACCOUNTS: &[&str] = &[
    "system_program",
    "token_program",
    "rent",
    "event_authority",
    "program",
    "associated_token_program",
    "memo_program",
    "token_2022_program",
    "clock",
    "instructions",
    "sysvar_instructions",
];

pub fn find_connections(idl: &IdlSpec, new_account: &str, existing: &[&str]) -> ConnectionReport {
    let all_account_names: Vec<&str> = idl
        .instructions
        .iter()
        .flat_map(|ix| ix.accounts.iter().map(|a| a.name.as_str()))
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();

    let new_account_exists = all_account_names.contains(&new_account);

    let mut invalid_existing = Vec::new();
    let mut valid_existing = Vec::new();

    for &account in existing {
        if all_account_names.contains(&account) {
            valid_existing.push(account);
        } else {
            let suggestions = suggest_similar(account, &all_account_names, 3);
            let suggestion_names: Vec<String> =
                suggestions.iter().map(|s| s.candidate.clone()).collect();
            invalid_existing.push((account.to_string(), suggestion_names));
        }
    }

    if !new_account_exists {
        return ConnectionReport {
            new_account: new_account.to_string(),
            direct: Vec::new(),
            transitive: Vec::new(),
            invalid_existing,
        };
    }

    let mut direct = Vec::new();
    for &existing_account in &valid_existing {
        let mut instructions = Vec::new();

        for instruction in &idl.instructions {
            let account_names: Vec<&str> = instruction
                .accounts
                .iter()
                .map(|account| account.name.as_str())
                .collect();

            if account_names.contains(&new_account) && account_names.contains(&existing_account) {
                let from_account = instruction
                    .accounts
                    .iter()
                    .find(|account| account.name == new_account);
                let to_account = instruction
                    .accounts
                    .iter()
                    .find(|account| account.name == existing_account);

                if let (Some(from_account), Some(to_account)) = (from_account, to_account) {
                    instructions.push(InstructionContext {
                        instruction_name: instruction.name.clone(),
                        from_role: AccountRole {
                            writable: from_account.is_mut,
                            signer: from_account.is_signer,
                            pda: from_account.pda.is_some(),
                        },
                        to_role: AccountRole {
                            writable: to_account.is_mut,
                            signer: to_account.is_signer,
                            pda: to_account.pda.is_some(),
                        },
                        all_accounts: account_names.iter().map(|name| name.to_string()).collect(),
                    });
                }
            }
        }

        if !instructions.is_empty() {
            direct.push(DirectConnection {
                from: new_account.to_string(),
                to: existing_account.to_string(),
                instructions,
            });
        }
    }

    let mut transitive = Vec::new();
    let directly_connected: HashSet<&str> = direct
        .iter()
        .map(|connection| connection.to.as_str())
        .collect();
    let unconnected: Vec<&str> = valid_existing
        .iter()
        .filter(|&&account| !directly_connected.contains(account))
        .copied()
        .collect();

    if !unconnected.is_empty() {
        let index = build_account_index(idl);
        let new_account_instructions: HashSet<&str> = index
            .get(new_account)
            .map(|usage| {
                usage
                    .instructions
                    .iter()
                    .map(|instruction| instruction.name.as_str())
                    .collect()
            })
            .unwrap_or_default();

        for &target in &unconnected {
            let target_instructions: HashSet<&str> = index
                .get(target)
                .map(|usage| {
                    usage
                        .instructions
                        .iter()
                        .map(|instruction| instruction.name.as_str())
                        .collect()
                })
                .unwrap_or_default();

            for (intermediary, usage) in &index {
                if intermediary == new_account || intermediary == target {
                    continue;
                }
                if INFRASTRUCTURE_ACCOUNTS.contains(&intermediary.as_str()) {
                    continue;
                }

                let intermediary_instructions: HashSet<&str> = usage
                    .instructions
                    .iter()
                    .map(|instruction| instruction.name.as_str())
                    .collect();

                let hop1 = new_account_instructions
                    .iter()
                    .find(|instruction| intermediary_instructions.contains(**instruction));
                let hop2 = target_instructions
                    .iter()
                    .find(|instruction| intermediary_instructions.contains(**instruction));

                if let (Some(hop1), Some(hop2)) = (hop1, hop2) {
                    transitive.push(TransitiveConnection {
                        from: new_account.to_string(),
                        intermediary: intermediary.clone(),
                        to: target.to_string(),
                        hop1_instruction: (*hop1).to_string(),
                        hop2_instruction: (*hop2).to_string(),
                    });
                    break;
                }
            }
        }
    }

    ConnectionReport {
        new_account: new_account.to_string(),
        direct,
        transitive,
        invalid_existing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse_idl_file;
    use std::path::PathBuf;

    fn meteora_fixture() -> IdlSpec {
        let path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/meteora_dlmm.json");
        parse_idl_file(&path).expect("should parse meteora_dlmm.json")
    }

    fn ore_fixture() -> IdlSpec {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ore.json");
        parse_idl_file(&path).expect("should parse ore.json")
    }

    #[test]
    fn test_connect_reward_vault() {
        let idl = meteora_fixture();
        let report = find_connections(&idl, "reward_vault", &["lb_pair", "position"]);
        assert!(
            !report.direct.is_empty(),
            "reward_vault should have direct connections"
        );

        let lb_pair_connection = report
            .direct
            .iter()
            .find(|connection| connection.to == "lb_pair");
        assert!(
            lb_pair_connection.is_some(),
            "reward_vault should connect to lb_pair"
        );
        assert!(!lb_pair_connection
            .expect("connection should exist")
            .instructions
            .is_empty());
    }

    #[test]
    fn test_connect_invalid_name() {
        let idl = meteora_fixture();
        let report = find_connections(&idl, "lb_pair", &["bogus_account_xyz"]);
        assert!(
            !report.invalid_existing.is_empty(),
            "bogus_account_xyz should be invalid"
        );

        let (name, suggestions) = &report.invalid_existing[0];
        assert_eq!(name, "bogus_account_xyz");
        let _ = suggestions;
    }

    #[test]
    fn test_connect_ore_entropyvar() {
        let idl = ore_fixture();
        let report = find_connections(&idl, "entropyVar", &["round"]);
        assert!(
            !report.direct.is_empty() || !report.transitive.is_empty(),
            "entropyVar should connect to round somehow"
        );
    }
}
