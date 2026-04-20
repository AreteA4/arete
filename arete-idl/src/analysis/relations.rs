use crate::types::IdlSpec;
use crate::utils::to_pascal_case;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct InstructionRef {
    pub name: String,
    pub account_count: usize,
    pub arg_count: usize,
}

#[derive(Debug, Clone)]
pub struct AccountUsage {
    pub account_name: String,
    pub instructions: Vec<InstructionRef>,
    pub is_writable: bool,
    pub is_signer: bool,
    pub is_pda: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AccountCategory {
    Entity,
    Infrastructure,
    Role,
    Other,
}

#[derive(Debug, Clone)]
pub struct AccountRelation {
    pub account_name: String,
    pub matched_type: Option<String>,
    pub instruction_count: usize,
    pub category: AccountCategory,
}

#[derive(Debug, Clone)]
pub struct InstructionLink {
    pub instruction_name: String,
    pub account_a_writable: bool,
    pub account_b_writable: bool,
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

const ROLE_ACCOUNTS: &[&str] = &[
    "authority",
    "owner",
    "payer",
    "signer",
    "admin",
    "user",
    "sender",
    "receiver",
];

pub fn build_account_index(idl: &IdlSpec) -> HashMap<String, AccountUsage> {
    let mut index: HashMap<String, AccountUsage> = HashMap::new();

    for ix in &idl.instructions {
        let ix_ref = InstructionRef {
            name: ix.name.clone(),
            account_count: ix.accounts.len(),
            arg_count: ix.args.len(),
        };
        for acc in &ix.accounts {
            let entry = index
                .entry(acc.name.clone())
                .or_insert_with(|| AccountUsage {
                    account_name: acc.name.clone(),
                    instructions: Vec::new(),
                    is_writable: false,
                    is_signer: false,
                    is_pda: false,
                });
            entry.instructions.push(ix_ref.clone());
            if acc.is_mut {
                entry.is_writable = true;
            }
            if acc.is_signer {
                entry.is_signer = true;
            }
            if acc.pda.is_some() {
                entry.is_pda = true;
            }
        }
    }
    index
}

pub fn classify_accounts(idl: &IdlSpec) -> Vec<AccountRelation> {
    let index = build_account_index(idl);
    let type_names: Vec<String> = idl.accounts.iter().map(|a| a.name.clone()).collect();

    index
        .into_values()
        .map(|usage| {
            let pascal = to_pascal_case(&usage.account_name);
            let matched_type = type_names
                .iter()
                .find(|t| **t == pascal || **t == usage.account_name)
                .cloned();

            let category = if INFRASTRUCTURE_ACCOUNTS.contains(&usage.account_name.as_str()) {
                AccountCategory::Infrastructure
            } else if matched_type.is_some() {
                AccountCategory::Entity
            } else if ROLE_ACCOUNTS.iter().any(|r| usage.account_name.contains(r))
                || usage.is_signer
            {
                AccountCategory::Role
            } else {
                AccountCategory::Other
            };

            AccountRelation {
                account_name: usage.account_name.clone(),
                matched_type,
                instruction_count: usage.instructions.len(),
                category,
            }
        })
        .collect()
}

pub fn find_account_usage(idl: &IdlSpec, account_name: &str) -> Option<AccountUsage> {
    let index = build_account_index(idl);
    index.into_values().find(|u| u.account_name == account_name)
}

pub fn find_links(idl: &IdlSpec, account_a: &str, account_b: &str) -> Vec<InstructionLink> {
    idl.instructions
        .iter()
        .filter(|ix| {
            let names: Vec<&str> = ix.accounts.iter().map(|a| a.name.as_str()).collect();
            names.contains(&account_a) && names.contains(&account_b)
        })
        .map(|ix| {
            let a_writable = ix
                .accounts
                .iter()
                .find(|a| a.name == account_a)
                .map(|a| a.is_mut)
                .unwrap_or(false);
            let b_writable = ix
                .accounts
                .iter()
                .find(|a| a.name == account_b)
                .map(|a| a.is_mut)
                .unwrap_or(false);
            InstructionLink {
                instruction_name: ix.name.clone(),
                account_a_writable: a_writable,
                account_b_writable: b_writable,
            }
        })
        .collect()
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

    #[test]
    fn test_classify_accounts_dlmm() {
        let idl = meteora_fixture();
        let relations = classify_accounts(&idl);
        let lb_pair = relations.iter().find(|r| r.account_name == "lb_pair");
        assert!(lb_pair.is_some(), "lb_pair should be in relations");
        assert_eq!(
            lb_pair.expect("lb_pair relation should exist").category,
            AccountCategory::Entity,
            "lb_pair should be Entity"
        );

        let sys = relations
            .iter()
            .find(|r| r.account_name == "system_program");
        if let Some(sys) = sys {
            assert_eq!(sys.category, AccountCategory::Infrastructure);
        }
    }

    #[test]
    fn test_find_links() {
        let idl = meteora_fixture();
        let links = find_links(&idl, "lb_pair", "position");
        assert!(
            !links.is_empty(),
            "lb_pair and position should share instructions"
        );
    }

    #[test]
    fn test_build_account_index() {
        let idl = meteora_fixture();
        let index = build_account_index(&idl);
        let lb_pair = index.get("lb_pair");
        assert!(lb_pair.is_some(), "lb_pair should be in index");
        assert!(
            lb_pair
                .expect("lb_pair account usage should exist")
                .instructions
                .len()
                > 10,
            "lb_pair should appear in many instructions"
        );
    }
}
