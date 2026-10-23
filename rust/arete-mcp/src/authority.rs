//! Instructions that hand control of the signer's funds to someone else
//! without moving a balance.
//!
//! The signer's caps measure balance changes, and some instructions change
//! no balance yet give the funds away: a token delegation (`Approve`), a new
//! owner or authority (`SetAuthority`, System `Assign`), or lamports leaving
//! an account the signer controls that is not the signer's own (a nonce or
//! stake withdrawal, a token account closed to someone else). This module
//! names those instructions in the System, SPL Token, Token-2022, upgradeable
//! loader and stake programs when the signer takes part in them (signs, or
//! is one of their accounts).
//!
//! Top-level instructions are read from the wire bytes. Inner (CPI)
//! instructions are read from a simulation when the RPC returns them.
//!
//! A closed token account whose lamports go to an account other than the
//! signer is not refused; its lamports count as SOL leaving the signer.

use serde_json::Value;

use crate::prepared_summary::{DecodedTransaction, TOKEN_PROGRAMS};

pub const SYSTEM_PROGRAM: &str = "11111111111111111111111111111111";
pub const UPGRADEABLE_LOADER: &str = "BPFLoaderUpgradeab1e11111111111111111111111";
pub const STAKE_PROGRAM: &str = "Stake11111111111111111111111111111111111111";
/// Wrapped SOL mints (SPL Token and Token-2022).
pub const NATIVE_MINTS: [&str; 2] = [
    "So11111111111111111111111111111111111111112",
    "9pan9bMn5HatX4EJdBwg9VgCa7Uz5HL8N1m5D3NdXejP",
];

/// Token instructions the signer may take part in. Everything else that
/// involves the signer is refused, including Token-2022 extension
/// instructions this list does not name.
const TOKEN_ALLOWED: [&str; 24] = [
    "initializeMint",
    "initializeMint2",
    "initializeAccount",
    "initializeAccount2",
    "initializeAccount3",
    "initializeMultisig",
    "initializeMultisig2",
    "initializeImmutableOwner",
    "transfer",
    "transferChecked",
    "transferCheckedWithFee",
    "mintTo",
    "mintToChecked",
    "burn",
    "burnChecked",
    "revoke",
    "freezeAccount",
    "thawAccount",
    "syncNative",
    "getAccountDataSize",
    "amountToUiAmount",
    "uiAmountToAmount",
    "reallocate",
    "createNativeMint",
];

/// Why a transaction with a durable nonce is refused.
pub const DURABLE_NONCE: &str =
    "makes it a durable-nonce transaction, valid until the nonce advances instead of expiring";

/// System instructions the signer may take part in.
const SYSTEM_ALLOWED: [&str; 7] = [
    "createAccount",
    "createAccountWithSeed",
    "transfer",
    "advanceNonce",
    "initializeNonce",
    "allocateWithSeed",
    "upgradeNonce",
];

/// Stake instructions the signer may take part in.
const STAKE_ALLOWED: [&str; 8] = [
    "initialize",
    "initializeChecked",
    "delegate",
    "split",
    "deactivate",
    "deactivateDelinquent",
    "redelegate",
    "getMinimumDelegation",
];

/// An instruction the signer must not sign.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub program: String,
    pub instruction: String,
    /// Whether it is an inner (CPI) instruction from the simulation.
    pub inner: bool,
    pub reason: String,
}

impl Finding {
    pub fn to_json(&self) -> Value {
        serde_json::json!({
            "programId": self.program,
            "instruction": self.instruction,
            "inner": self.inner,
            "reason": self.reason,
        })
    }
}

/// What the checks found in one transaction.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Checked {
    pub findings: Vec<Finding>,
    /// Token accounts the signer closes to another account: their lamports
    /// leave the signer. `None` when the account's address is not known
    /// (a lookup-table entry that was not resolved).
    pub closed_to_others: Vec<Option<String>>,
}

impl Checked {
    pub fn merge(&mut self, other: Checked) {
        self.findings.extend(other.findings);
        self.closed_to_others.extend(other.closed_to_others);
    }
}

fn token_name(data: &[u8]) -> String {
    let Some(&tag) = data.first() else {
        return "emptyInstruction".into();
    };
    match tag {
        0 => "initializeMint",
        1 => "initializeAccount",
        2 => "initializeMultisig",
        3 => "transfer",
        4 => "approve",
        5 => "revoke",
        6 => "setAuthority",
        7 => "mintTo",
        8 => "burn",
        9 => "closeAccount",
        10 => "freezeAccount",
        11 => "thawAccount",
        12 => "transferChecked",
        13 => "approveChecked",
        14 => "mintToChecked",
        15 => "burnChecked",
        16 => "initializeAccount2",
        17 => "syncNative",
        18 => "initializeAccount3",
        19 => "initializeMultisig2",
        20 => "initializeMint2",
        21 => "getAccountDataSize",
        22 => "initializeImmutableOwner",
        23 => "amountToUiAmount",
        24 => "uiAmountToAmount",
        25 => "initializeMintCloseAuthority",
        26 if data.get(1) == Some(&1) => "transferCheckedWithFee",
        29 => "reallocate",
        31 => "createNativeMint",
        38 => "withdrawExcessLamports",
        other => return format!("tokenInstruction{other}"),
    }
    .to_string()
}

fn u32_tag(data: &[u8]) -> Option<u32> {
    data.get(..4)
        .map(|bytes| u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn system_name(data: &[u8]) -> String {
    match u32_tag(data) {
        Some(0) => "createAccount",
        Some(1) => "assign",
        Some(2) => "transfer",
        Some(3) => "createAccountWithSeed",
        Some(4) => "advanceNonce",
        Some(5) => "withdrawFromNonce",
        Some(6) => "initializeNonce",
        Some(7) => "authorizeNonce",
        Some(8) => "allocate",
        Some(9) => "allocateWithSeed",
        Some(10) => "assignWithSeed",
        Some(11) => "transferWithSeed",
        Some(12) => "upgradeNonce",
        Some(other) => return format!("systemInstruction{other}"),
        None => "emptyInstruction",
    }
    .to_string()
}

fn stake_name(data: &[u8]) -> String {
    match u32_tag(data) {
        Some(0) => "initialize",
        Some(1) => "authorize",
        Some(2) => "delegate",
        Some(3) => "split",
        Some(4) => "withdraw",
        Some(5) => "deactivate",
        Some(6) => "setLockup",
        Some(7) => "merge",
        Some(8) => "authorizeWithSeed",
        Some(9) => "initializeChecked",
        Some(10) => "authorizeChecked",
        Some(11) => "authorizeCheckedWithSeed",
        Some(12) => "setLockupChecked",
        Some(13) => "getMinimumDelegation",
        Some(14) => "deactivateDelinquent",
        Some(15) => "redelegate",
        Some(other) => return format!("stakeInstruction{other}"),
        None => "emptyInstruction",
    }
    .to_string()
}

/// The instruction's name, for the programs this module checks.
fn instruction_name(program: &str, data: &[u8]) -> Option<String> {
    if TOKEN_PROGRAMS.contains(&program) {
        Some(token_name(data))
    } else if program == SYSTEM_PROGRAM {
        Some(system_name(data))
    } else if program == STAKE_PROGRAM {
        Some(stake_name(data))
    } else if program == UPGRADEABLE_LOADER {
        Some(match u32_tag(data) {
            Some(tag) => format!("upgradeableLoaderInstruction{tag}"),
            None => "emptyInstruction".into(),
        })
    } else {
        None
    }
}

/// Why the signer may not take part in `name` of `program`, or `None`.
fn refusal_reason(program: &str, name: &str) -> Option<String> {
    let reason = if TOKEN_PROGRAMS.contains(&program) {
        match name {
            "approve" | "approveChecked" => "delegates tokens the signer controls".to_string(),
            "setAuthority" => "changes an authority the signer holds".to_string(),
            "closeAccount" => return None,
            "withdrawExcessLamports" => {
                "moves lamports out of an account the signer controls".to_string()
            }
            name if TOKEN_ALLOWED.contains(&name) => return None,
            _ => "is a token instruction the signer's policy cannot check".to_string(),
        }
    } else if program == SYSTEM_PROGRAM {
        match name {
            "assign" | "assignWithSeed" => {
                "changes the owner of an account the signer controls".to_string()
            }
            "allocate" => "allocates data on the signer's account".to_string(),
            "authorizeNonce" => "changes a nonce authority the signer holds".to_string(),
            "withdrawFromNonce" | "transferWithSeed" => {
                "moves lamports out of an account the signer controls".to_string()
            }
            name if SYSTEM_ALLOWED.contains(&name) => return None,
            _ => "is a System instruction the signer's policy cannot check".to_string(),
        }
    } else if program == STAKE_PROGRAM {
        match name {
            name if STAKE_ALLOWED.contains(&name) => return None,
            _ => "changes the authority or withdraws from a stake account the signer controls"
                .to_string(),
        }
    } else if program == UPGRADEABLE_LOADER {
        "changes a program or buffer the signer controls".to_string()
    } else {
        return None;
    };
    Some(reason)
}

/// Check one instruction in which the signer takes part. `destination` is
/// the account a `closeAccount` sends its lamports to, `closed` the account
/// it closes.
fn check_one(
    program: &str,
    name: String,
    inner: bool,
    closed: Option<String>,
    destination_is_signer: bool,
    out: &mut Checked,
) {
    if TOKEN_PROGRAMS.contains(&program) && name == "closeAccount" {
        if !destination_is_signer {
            out.closed_to_others.push(closed);
        }
        return;
    }
    if let Some(reason) = refusal_reason(program, &name) {
        out.findings.push(Finding {
            program: program.to_string(),
            instruction: name,
            inner,
            reason,
        });
    }
}

/// Check the top-level instructions of `decoded`. `lookup_keys` are the
/// addresses its lookup tables load (writable entries, then read-only),
/// where they were resolved.
pub fn check_top_level(
    decoded: &DecodedTransaction,
    signer: &str,
    lookup_keys: &[Option<String>],
) -> Checked {
    let mut out = Checked::default();
    let Some(signer_index) = decoded
        .static_keys
        .iter()
        .take(decoded.required_signatures)
        .position(|key| key == signer)
    else {
        // Not a signer of this transaction: no authority of its can be used.
        return out;
    };
    let key = |index: usize| -> Option<String> {
        decoded.static_keys.get(index).cloned().or_else(|| {
            lookup_keys
                .get(index - decoded.static_keys.len())
                .cloned()
                .flatten()
        })
    };
    // A transaction that starts by advancing a nonce uses a durable nonce
    // instead of a recent blockhash: whoever the nonce authority is, the
    // signed bytes stay valid until the nonce advances, so they could land
    // long after the step was checked and its consent revoked.
    if let Some(first) = decoded.instructions.first() {
        if decoded
            .static_keys
            .get(first.program_index)
            .map(String::as_str)
            == Some(SYSTEM_PROGRAM)
            && u32_tag(&first.data) == Some(4)
        {
            out.findings.push(Finding {
                program: SYSTEM_PROGRAM.to_string(),
                instruction: "advanceNonce".to_string(),
                inner: false,
                reason: DURABLE_NONCE.to_string(),
            });
        }
    }
    for instruction in &decoded.instructions {
        let Some(program) = decoded.static_keys.get(instruction.program_index) else {
            continue;
        };
        let Some(name) = instruction_name(program, &instruction.data) else {
            continue;
        };
        if !instruction.accounts.contains(&signer_index) {
            continue;
        }
        if name == "advanceNonce" && program == SYSTEM_PROGRAM {
            continue;
        }
        let closed = instruction.accounts.first().and_then(|&index| key(index));
        let destination_is_signer = instruction.accounts.get(1) == Some(&signer_index);
        check_one(
            program,
            name,
            false,
            closed,
            destination_is_signer,
            &mut out,
        );
    }
    out
}

fn mentions(value: &Value, signer: &str) -> bool {
    match value {
        Value::String(text) => text == signer,
        Value::Array(items) => items.iter().any(|item| mentions(item, signer)),
        Value::Object(map) => map.values().any(|item| mentions(item, signer)),
        _ => false,
    }
}

/// Check the inner instructions a simulation returned (`innerInstructions`
/// of `simulateTransaction`, in the RPC's parsed or partially decoded
/// form). `account_keys` resolves compiled instructions' indexes.
pub fn check_inner(inner: &Value, signer: &str, account_keys: &[Option<String>]) -> Checked {
    let mut out = Checked::default();
    for group in inner.as_array().into_iter().flatten() {
        for instruction in group["instructions"].as_array().into_iter().flatten() {
            out.merge(check_inner_one(instruction, signer, account_keys));
        }
    }
    out
}

fn check_inner_one(instruction: &Value, signer: &str, account_keys: &[Option<String>]) -> Checked {
    let mut out = Checked::default();
    let program = match instruction["programId"].as_str() {
        Some(program) => program.to_string(),
        None => match instruction["programIdIndex"]
            .as_u64()
            .and_then(|index| account_keys.get(index as usize).cloned().flatten())
        {
            Some(program) => program,
            None => return out,
        },
    };
    if let Some(parsed) = instruction.get("parsed").filter(|p| p.is_object()) {
        let Some(name) = parsed["type"].as_str() else {
            return out;
        };
        let info = &parsed["info"];
        if instruction_name(&program, &[]).is_none() || !mentions(info, signer) {
            return out;
        }
        let closed = info["account"].as_str().map(str::to_string);
        let destination_is_signer = info["destination"].as_str() == Some(signer);
        check_one(
            &program,
            name.to_string(),
            true,
            closed,
            destination_is_signer,
            &mut out,
        );
        return out;
    }
    let accounts: Vec<Option<String>> = match instruction["accounts"].as_array() {
        Some(accounts) => accounts
            .iter()
            .map(|account| match account {
                Value::String(key) => Some(key.clone()),
                Value::Number(index) => index
                    .as_u64()
                    .and_then(|index| account_keys.get(index as usize).cloned().flatten()),
                _ => None,
            })
            .collect(),
        None => return out,
    };
    let data = instruction["data"]
        .as_str()
        .and_then(|data| bs58::decode(data).into_vec().ok())
        .unwrap_or_default();
    let Some(name) = instruction_name(&program, &data) else {
        return out;
    };
    if !accounts.iter().any(|key| key.as_deref() == Some(signer)) {
        return out;
    }
    let closed = accounts.first().cloned().flatten();
    let destination_is_signer = accounts.get(1).and_then(Option::as_deref) == Some(signer);
    check_one(
        &program,
        name,
        true,
        closed,
        destination_is_signer,
        &mut out,
    );
    out
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::prepared_summary::CompiledInstruction;

    fn b58(bytes: &[u8]) -> String {
        bs58::encode(bytes).into_string()
    }

    /// A transaction signed by `signer` (key 0) with one instruction of
    /// `program` (key 2) over `accounts`, and one other writable key (1).
    fn decoded(program: &str, accounts: Vec<usize>, data: Vec<u8>) -> DecodedTransaction {
        DecodedTransaction {
            version: None,
            signatures: vec![[0; 64]],
            signature_offset: 1,
            message: 0..0,
            required_signatures: 1,
            static_keys: vec![b58(&[1; 32]), b58(&[2; 32]), program.to_string()],
            writable_static: vec![0, 1],
            instruction_programs: vec![program.to_string()],
            instructions: vec![CompiledInstruction {
                program_index: 2,
                accounts,
                data,
            }],
            lookups: Vec::new(),
        }
    }

    fn findings(program: &str, accounts: Vec<usize>, data: Vec<u8>) -> Vec<String> {
        check_top_level(&decoded(program, accounts, data), &b58(&[1; 32]), &[])
            .findings
            .into_iter()
            .map(|finding| finding.instruction)
            .collect()
    }

    #[test]
    fn refuses_token_delegations_and_authority_changes_by_the_signer() {
        let token = TOKEN_PROGRAMS[0];
        assert_eq!(findings(token, vec![1, 1, 0], vec![4, 1]), vec!["approve"]);
        assert_eq!(
            findings(TOKEN_PROGRAMS[1], vec![1, 1, 1, 0], vec![13]),
            vec!["approveChecked"]
        );
        assert_eq!(
            findings(token, vec![1, 0], vec![6, 2]),
            vec!["setAuthority"]
        );
        // Transfers are bounded by the caps, not refused here.
        assert!(findings(token, vec![1, 1, 0], vec![3]).is_empty());
        // Unknown Token-2022 extension instructions with the signer are refused.
        assert_eq!(
            findings(TOKEN_PROGRAMS[1], vec![1, 0], vec![40, 1]),
            vec!["tokenInstruction40"]
        );
        // Without the signer nothing is refused.
        assert!(findings(token, vec![1, 1], vec![4]).is_empty());
    }

    #[test]
    fn refuses_system_assignments_of_the_signer() {
        assert_eq!(
            findings(SYSTEM_PROGRAM, vec![0], vec![1, 0, 0, 0]),
            vec!["assign"]
        );
        assert_eq!(
            findings(SYSTEM_PROGRAM, vec![1, 0], vec![10, 0, 0, 0]),
            vec!["assignWithSeed"]
        );
        assert_eq!(
            findings(SYSTEM_PROGRAM, vec![1, 1, 0], vec![7, 0, 0, 0]),
            vec!["authorizeNonce"]
        );
        assert!(findings(SYSTEM_PROGRAM, vec![0, 1], vec![2, 0, 0, 0]).is_empty());
        assert_eq!(
            findings(UPGRADEABLE_LOADER, vec![1, 0], vec![4, 0, 0, 0]),
            vec!["upgradeableLoaderInstruction4"]
        );
        assert_eq!(
            findings(STAKE_PROGRAM, vec![1, 0], vec![1, 0, 0, 0]),
            vec!["authorize"]
        );
    }

    #[test]
    fn closing_to_another_account_counts_as_an_outflow() {
        let signer = b58(&[1; 32]);
        let token = TOKEN_PROGRAMS[0];
        // closeAccount(account = key 1, destination = key 1, owner = signer)
        let checked = check_top_level(&decoded(token, vec![1, 1, 0], vec![9]), &signer, &[]);
        assert!(checked.findings.is_empty());
        assert_eq!(checked.closed_to_others, vec![Some(b58(&[2; 32]))]);
        // Closing to the signer is not an outflow.
        let checked = check_top_level(&decoded(token, vec![1, 0, 0], vec![9]), &signer, &[]);
        assert!(checked.closed_to_others.is_empty());
    }

    #[test]
    fn reads_parsed_and_partially_decoded_inner_instructions() {
        let signer = b58(&[1; 32]);
        let inner = json!([{ "index": 0, "instructions": [
            { "programId": TOKEN_PROGRAMS[0], "program": "spl-token", "parsed": {
                "type": "approve", "info": { "source": "S", "delegate": "D", "owner": signer, "amount": "1" } } },
            { "programId": TOKEN_PROGRAMS[0], "program": "spl-token", "parsed": {
                "type": "transfer", "info": { "source": "S", "destination": "T", "authority": signer, "amount": "1" } } },
            { "programId": TOKEN_PROGRAMS[0], "program": "spl-token", "parsed": {
                "type": "setAuthority", "info": { "account": "S", "authority": "someone-else" } } },
            { "programId": SYSTEM_PROGRAM, "accounts": [signer], "data": bs58::encode([1u8, 0, 0, 0]).into_string() },
            { "programId": TOKEN_PROGRAMS[1], "program": "spl-token-2022", "parsed": {
                "type": "closeAccount", "info": { "account": "A", "destination": "elsewhere", "owner": signer } } },
        ] }]);
        let checked = check_inner(&inner, &signer, &[]);
        let names: Vec<&str> = checked
            .findings
            .iter()
            .map(|finding| finding.instruction.as_str())
            .collect();
        assert_eq!(names, vec!["approve", "assign"]);
        assert!(checked.findings.iter().all(|finding| finding.inner));
        assert_eq!(checked.closed_to_others, vec![Some("A".to_string())]);
    }
}
