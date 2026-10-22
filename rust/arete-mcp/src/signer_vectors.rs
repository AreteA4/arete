//! The shared signer-policy test vectors in `test-vectors/signer-policy/`.
//!
//! The cases are defined here and their expected results are what this crate
//! computes, so the Rust signer is the reference. The test fails when the
//! files on disk differ from what the code produces now; regenerate them
//! with
//!
//! ```text
//! ARETE_WRITE_SIGNER_POLICY_VECTORS=1 cargo test -p arete-mcp --lib signer_vectors
//! ```
//!
//! Every other implementation of the policy (the TypeScript automations
//! package) runs the same files.
//!
//! Chain state is a fixture rather than a node. `getMultipleAccounts`
//! answers each address from `chain.accounts` (`null` when absent), and
//! `simulateTransaction` answers from `chain.simulation`: `rpcError` makes
//! the call fail with that message; otherwise the result carries `err`,
//! `logs`, `innerInstructions` (only when the fixture has it) and, for each
//! address the request lists, the post-state from `simulation.accounts`.

use std::path::PathBuf;

use anyhow::anyhow;
use base64::Engine as _;
use serde_json::{json, Value};

use crate::authority::{
    check_inner, check_top_level, Checked, NATIVE_MINTS, STAKE_PROGRAM, SYSTEM_PROGRAM,
    UPGRADEABLE_LOADER,
};
use crate::prepared_summary::{decode_transaction, summarize, Rpc, TOKEN_PROGRAMS};
use crate::signer::SignerPolicy;

const WRITE_ENV: &str = "ARETE_WRITE_SIGNER_POLICY_VECTORS";
const LOOKUP_TABLE_PROGRAM: &str = "AddressLookupTab1e1111111111111111111111111";
const RENT: u64 = 2_039_280;

// ---------------------------------------------------------------------------
// Keys and wire transactions
// ---------------------------------------------------------------------------

fn seed(n: u8) -> [u8; 32] {
    [n; 32]
}

fn b58(bytes: &[u8]) -> String {
    bs58::encode(bytes).into_string()
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn addr(n: u8) -> String {
    b58(&seed(n))
}

fn raw(key: &str) -> [u8; 32] {
    bs58::decode(key).into_vec().unwrap().try_into().unwrap()
}

/// The wallet the policy signs for in every case.
fn signer() -> String {
    addr(1)
}

const OTHER_WALLET: u8 = 3;
const MINT: u8 = 20;
const OTHER_MINT: u8 = 21;
const TOKEN_ACCOUNT: u8 = 30;
const OTHER_TOKEN_ACCOUNT: u8 = 31;
const WRAPPED_ACCOUNT: u8 = 32;
const PROGRAM: u8 = 40;
const OTHER_PROGRAM: u8 = 41;
const LOOKUP_TABLE: u8 = 50;

fn compact_u16(mut value: usize, out: &mut Vec<u8>) {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

struct Ix {
    program: u8,
    accounts: Vec<u8>,
    data: Vec<u8>,
}

fn ix(program: u8, accounts: &[u8], data: &[u8]) -> Ix {
    Ix {
        program,
        accounts: accounts.to_vec(),
        data: data.to_vec(),
    }
}

/// A legacy or v0 message. `header` is (required signatures, read-only
/// signed, read-only unsigned).
struct Message {
    v0: bool,
    header: [u8; 3],
    keys: Vec<[u8; 32]>,
    instructions: Vec<Ix>,
    lookups: Vec<([u8; 32], Vec<u8>, Vec<u8>)>,
}

impl Message {
    /// `signers` sign and are writable, then `writable` unsigned keys, then
    /// `readonly` unsigned keys (programs).
    fn new(signers: &[[u8; 32]], writable: &[[u8; 32]], readonly: &[[u8; 32]]) -> Self {
        Self {
            v0: false,
            header: [signers.len() as u8, 0, readonly.len() as u8],
            keys: signers
                .iter()
                .chain(writable)
                .chain(readonly)
                .copied()
                .collect(),
            instructions: Vec::new(),
            lookups: Vec::new(),
        }
    }

    fn with(mut self, instruction: Ix) -> Self {
        self.instructions.push(instruction);
        self
    }

    fn v0(mut self, lookups: Vec<([u8; 32], Vec<u8>, Vec<u8>)>) -> Self {
        self.v0 = true;
        self.lookups = lookups;
        self
    }

    /// The wire transaction; signature slots are zero unless `signed`.
    fn transaction(&self, signed: &[bool]) -> Vec<u8> {
        let mut message = Vec::new();
        if self.v0 {
            message.push(0x80);
        }
        message.extend_from_slice(&self.header);
        compact_u16(self.keys.len(), &mut message);
        for key in &self.keys {
            message.extend_from_slice(key);
        }
        message.extend_from_slice(&seed(7)); // recent blockhash
        compact_u16(self.instructions.len(), &mut message);
        for instruction in &self.instructions {
            message.push(instruction.program);
            compact_u16(instruction.accounts.len(), &mut message);
            message.extend_from_slice(&instruction.accounts);
            compact_u16(instruction.data.len(), &mut message);
            message.extend_from_slice(&instruction.data);
        }
        if self.v0 {
            compact_u16(self.lookups.len(), &mut message);
            for (table, writable, readonly) in &self.lookups {
                message.extend_from_slice(table);
                compact_u16(writable.len(), &mut message);
                message.extend_from_slice(writable);
                compact_u16(readonly.len(), &mut message);
                message.extend_from_slice(readonly);
            }
        }
        let required = self.header[0] as usize;
        let mut transaction = Vec::new();
        compact_u16(required, &mut transaction);
        for index in 0..required {
            transaction.extend_from_slice(&[u8::from(signed.get(index) == Some(&true)); 64]);
        }
        transaction.extend(message);
        transaction
    }

    fn unsigned(&self) -> Vec<u8> {
        self.transaction(&[])
    }
}

/// A v1 transaction: `mask` config bits, each followed by a 4-byte value.
fn v1_transaction(
    header: [u8; 3],
    mask: u32,
    keys: &[[u8; 32]],
    instructions: &[Ix],
    signatures: usize,
) -> Vec<u8> {
    let mut tx = vec![0x81];
    tx.extend_from_slice(&header);
    tx.extend_from_slice(&mask.to_le_bytes());
    tx.extend_from_slice(&seed(7));
    tx.push(instructions.len() as u8);
    tx.push(keys.len() as u8);
    for key in keys {
        tx.extend_from_slice(key);
    }
    for bit in 0..32 {
        if mask & (1 << bit) != 0 {
            tx.extend_from_slice(&(200_000u32 + bit).to_le_bytes());
        }
    }
    for instruction in instructions {
        tx.push(instruction.program);
        tx.push(instruction.accounts.len() as u8);
        tx.extend_from_slice(&(instruction.data.len() as u16).to_le_bytes());
    }
    for instruction in instructions {
        tx.extend_from_slice(&instruction.accounts);
        tx.extend_from_slice(&instruction.data);
    }
    tx.extend(vec![0u8; 64 * signatures]);
    tx
}

fn u32_data(tag: u32) -> Vec<u8> {
    tag.to_le_bytes().to_vec()
}

// ---------------------------------------------------------------------------
// Chain fixtures
// ---------------------------------------------------------------------------

fn system_account(lamports: u64) -> Value {
    json!({ "lamports": lamports, "owner": SYSTEM_PROGRAM, "data": ["", "base64"], "executable": false })
}

fn token_account(program: &str, lamports: u64, mint: &str, owner: &str, amount: u64) -> Value {
    let mut data = vec![0u8; 165];
    data[0..32].copy_from_slice(&raw(mint));
    data[32..64].copy_from_slice(&raw(owner));
    data[64..72].copy_from_slice(&amount.to_le_bytes());
    json!({ "lamports": lamports, "owner": program, "data": [b64(&data), "base64"], "executable": false })
}

fn spl(owner: &str, amount: u64) -> Value {
    token_account(TOKEN_PROGRAMS[0], RENT, &addr(MINT), owner, amount)
}

fn wrapped(owner: &str, amount: u64) -> Value {
    token_account(
        TOKEN_PROGRAMS[0],
        RENT + amount,
        NATIVE_MINTS[0],
        owner,
        amount,
    )
}

fn lookup_table(addresses: &[[u8; 32]]) -> Value {
    let mut data = vec![0u8; 56];
    for address in addresses {
        data.extend_from_slice(address);
    }
    json!({ "lamports": 1, "owner": LOOKUP_TABLE_PROGRAM, "data": [b64(&data), "base64"], "executable": false })
}

fn object(entries: Vec<(String, Value)>) -> Value {
    Value::Object(entries.into_iter().collect())
}

/// A chain whose simulation succeeds with `post` as the post-state.
fn chain(pre: Vec<(String, Value)>, post: Vec<(String, Value)>) -> Value {
    json!({
        "slot": 100,
        "accounts": object(pre),
        "simulation": { "slot": 101, "err": null, "logs": ["Program log: ok"], "innerInstructions": [], "accounts": object(post) },
    })
}

fn fixture_rpc(chain: &Value) -> Rpc {
    let chain = chain.clone();
    Rpc::from_fixture(move |method, params| match method {
        "getMultipleAccounts" => {
            let values: Vec<Value> = params[0]
                .as_array()
                .ok_or_else(|| anyhow!("no addresses"))?
                .iter()
                .map(|address| {
                    chain["accounts"]
                        .get(address.as_str().unwrap_or_default())
                        .cloned()
                        .unwrap_or(Value::Null)
                })
                .collect();
            Ok(json!({ "context": { "slot": chain["slot"] }, "value": values }))
        }
        "simulateTransaction" => {
            let simulation = &chain["simulation"];
            if let Some(message) = simulation["rpcError"].as_str() {
                return Err(anyhow!("{message}"));
            }
            let post: Vec<Value> = params[1]["accounts"]["addresses"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .iter()
                .map(|address| {
                    simulation["accounts"]
                        .get(address.as_str().unwrap_or_default())
                        .cloned()
                        .unwrap_or(Value::Null)
                })
                .collect();
            let mut value = json!({
                "err": simulation["err"],
                "logs": simulation.get("logs").cloned().unwrap_or(json!([])),
                "accounts": post,
            });
            if let Some(inner) = simulation.get("innerInstructions") {
                value["innerInstructions"] = inner.clone();
            }
            Ok(json!({ "context": { "slot": simulation["slot"] }, "value": value }))
        }
        other => Err(anyhow!("unexpected method {other}")),
    })
}

fn prepared(deployment: &str, transactions: &[Vec<u8>]) -> Value {
    json!({
        "intent": { "deployment": { "deploymentId": deployment } },
        "transactions": transactions
            .iter()
            .enumerate()
            .map(|(index, bytes)| json!({ "index": index, "transactionBase64": b64(bytes) }))
            .collect::<Vec<_>>(),
    })
}

// ---------------------------------------------------------------------------
// The transactions the summary and check cases use
// ---------------------------------------------------------------------------

/// The signer sends lamports to another wallet.
fn sol_transfer() -> Vec<u8> {
    Message::new(&[seed(1)], &[seed(OTHER_WALLET)], &[raw(SYSTEM_PROGRAM)])
        .with(ix(
            2,
            &[0, 1],
            &[2, 0, 0, 0, 0x80, 0x96, 0x98, 0, 0, 0, 0, 0],
        ))
        .unsigned()
}

/// The protocol program moves tokens out of the signer's token account.
fn program_token_move() -> Vec<u8> {
    Message::new(
        &[seed(1)],
        &[seed(TOKEN_ACCOUNT), seed(OTHER_TOKEN_ACCOUNT)],
        &[seed(PROGRAM), raw(TOKEN_PROGRAMS[0])],
    )
    .with(ix(3, &[0, 1, 2, 4], &[1]))
    .unsigned()
}

/// The protocol program touches the signer's wrapped SOL account.
fn program_wrapped_move() -> Vec<u8> {
    Message::new(
        &[seed(1)],
        &[seed(WRAPPED_ACCOUNT), seed(OTHER_WALLET)],
        &[seed(PROGRAM), raw(TOKEN_PROGRAMS[0])],
    )
    .with(ix(3, &[0, 1, 2, 4], &[2]))
    .unsigned()
}

/// The signer closes its wrapped SOL account to another wallet.
fn close_to_other() -> Vec<u8> {
    Message::new(
        &[seed(1)],
        &[seed(WRAPPED_ACCOUNT), seed(OTHER_WALLET)],
        &[raw(TOKEN_PROGRAMS[0])],
    )
    .with(ix(3, &[1, 2, 0], &[9]))
    .unsigned()
}

/// A v0 transaction that closes an account loaded from an unresolved
/// lookup table to another wallet.
fn close_from_unresolved_lookup() -> Vec<u8> {
    Message::new(&[seed(1)], &[seed(OTHER_WALLET)], &[raw(TOKEN_PROGRAMS[0])])
        .with(ix(2, &[3, 1, 0], &[9]))
        .v0(vec![(seed(LOOKUP_TABLE), vec![0], vec![])])
        .unsigned()
}

/// A v0 transaction writing two lookup-table entries.
fn program_with_lookups() -> Vec<u8> {
    Message::new(&[seed(1)], &[], &[seed(PROGRAM)])
        .with(ix(1, &[0, 2, 3, 4], &[3]))
        .v0(vec![(seed(LOOKUP_TABLE), vec![0, 2], vec![1])])
        .unsigned()
}

/// The protocol program writes more accounts than a simulation reports on.
fn program_writing_many() -> Vec<u8> {
    let writable: Vec<[u8; 32]> = (100..170).map(seed).collect();
    Message::new(&[seed(1)], &writable, &[seed(PROGRAM)])
        .with(ix(71, &[0, 1], &[4]))
        .unsigned()
}

/// The signer approves a delegate on its token account.
fn top_level_approve() -> Vec<u8> {
    Message::new(
        &[seed(1)],
        &[seed(TOKEN_ACCOUNT), seed(OTHER_WALLET)],
        &[raw(TOKEN_PROGRAMS[0])],
    )
    .with(ix(3, &[1, 2, 0], &[4, 1, 0, 0, 0, 0, 0, 0, 0]))
    .unsigned()
}

fn base_policy() -> Value {
    json!({ "deployments": ["dep-1"], "maxSolPerTransaction": "0.05", "clusters": ["dev"] })
}

fn policy_with(fields: Value) -> Value {
    let mut policy = base_policy();
    for (key, value) in fields.as_object().unwrap() {
        policy[key] = value.clone();
    }
    policy
}

fn sol_transfer_chain() -> Value {
    chain(
        vec![(signer(), system_account(2_000_000_000))],
        vec![
            (signer(), system_account(2_000_000_000 - 10_005_000)),
            (addr(OTHER_WALLET), system_account(10_000_000)),
        ],
    )
}

fn token_move_chain(post_amount: u64) -> Value {
    chain(
        vec![
            (signer(), system_account(1_000_000_000)),
            (addr(TOKEN_ACCOUNT), spl(&signer(), 1000)),
            (addr(OTHER_TOKEN_ACCOUNT), spl(&addr(OTHER_WALLET), 0)),
        ],
        vec![
            (signer(), system_account(1_000_000_000 - 5000)),
            (addr(TOKEN_ACCOUNT), spl(&signer(), post_amount)),
            (
                addr(OTHER_TOKEN_ACCOUNT),
                spl(&addr(OTHER_WALLET), 1000u64.saturating_sub(post_amount)),
            ),
        ],
    )
}

fn handed_over_chain() -> Value {
    chain(
        vec![
            (signer(), system_account(1_000_000_000)),
            (addr(TOKEN_ACCOUNT), spl(&signer(), 500)),
        ],
        vec![
            (signer(), system_account(1_000_000_000 - 5000)),
            (addr(TOKEN_ACCOUNT), spl(&addr(OTHER_WALLET), 500)),
        ],
    )
}

fn wrapped_spent_chain() -> Value {
    chain(
        vec![
            (signer(), system_account(1_000_000_000)),
            (addr(WRAPPED_ACCOUNT), wrapped(&signer(), 1_000_000)),
        ],
        vec![
            (signer(), system_account(1_000_000_000 - 5000)),
            (addr(WRAPPED_ACCOUNT), wrapped(&signer(), 0)),
        ],
    )
}

fn wrapping_chain() -> Value {
    chain(
        vec![
            (signer(), system_account(2_000_000_000)),
            (addr(WRAPPED_ACCOUNT), wrapped(&signer(), 0)),
        ],
        vec![
            (signer(), system_account(2_000_000_000 - 1_005_000)),
            (addr(WRAPPED_ACCOUNT), wrapped(&signer(), 1_000_000)),
        ],
    )
}

fn close_to_other_chain() -> Value {
    chain(
        vec![
            (signer(), system_account(1_000_000_000)),
            (addr(WRAPPED_ACCOUNT), wrapped(&signer(), 500)),
        ],
        vec![
            (signer(), system_account(1_000_000_000 - 5000)),
            (addr(OTHER_WALLET), system_account(RENT + 500)),
        ],
    )
}

fn unresolved_lookup_chain() -> Value {
    chain(
        vec![(signer(), system_account(1_000_000_000))],
        vec![(signer(), system_account(1_000_000_000 - 5000))],
    )
}

fn lookups_chain() -> Value {
    chain(
        vec![
            (signer(), system_account(1_000_000_000)),
            (
                addr(LOOKUP_TABLE),
                lookup_table(&[seed(60), seed(61), seed(62)]),
            ),
            (addr(60), system_account(10)),
            (addr(62), system_account(20)),
        ],
        vec![
            (signer(), system_account(1_000_000_000 - 5000)),
            (addr(60), system_account(15)),
            (addr(62), system_account(20)),
        ],
    )
}

fn many_writes_chain() -> Value {
    chain(
        vec![(signer(), system_account(1_000_000_000))],
        vec![(signer(), system_account(1_000_000_000 - 5000))],
    )
}

fn failed_simulation_chain() -> Value {
    let logs: Vec<String> = (0..25)
        .map(|line| format!("Program log: line {line}"))
        .collect();
    json!({
        "slot": 100,
        "accounts": { signer(): system_account(1_000_000_000) },
        "simulation": { "slot": 101, "err": { "InstructionError": [0, { "Custom": 1 }] }, "logs": logs, "accounts": {} },
    })
}

fn rpc_error_chain() -> Value {
    json!({
        "slot": 100,
        "accounts": { signer(): system_account(1_000_000_000) },
        "simulation": { "rpcError": "node is behind" },
    })
}

fn inner_approve_chain() -> Value {
    let mut value = token_move_chain(1000);
    value["simulation"]["innerInstructions"] = json!([{ "index": 0, "instructions": [
        { "programId": TOKEN_PROGRAMS[0], "program": "spl-token", "parsed": {
            "type": "approve", "info": { "source": addr(TOKEN_ACCOUNT), "delegate": addr(OTHER_WALLET), "owner": signer(), "amount": "1" } } },
    ] }]);
    value
}

// ---------------------------------------------------------------------------
// Cases
// ---------------------------------------------------------------------------

fn policy_json(policy: &SignerPolicy) -> Value {
    json!({
        "deployments": policy.deployments,
        "maxLamportsPerTransaction": policy.max_lamports_per_transaction.to_string(),
        "programs": policy.programs,
        "maxTokenAmounts": policy.max_token_amounts.as_ref().map(|caps| {
            caps.iter()
                .map(|(mint, cap)| (mint.clone(), Value::String(cap.to_string())))
                .collect::<serde_json::Map<_, _>>()
        }),
        "clusters": policy.clusters,
        "clustersListed": policy.clusters_listed,
    })
}

fn config_cases() -> Vec<Value> {
    let mint = addr(5);
    let inputs: Vec<(&str, Value)> = vec![
        (
            "minimal policy uses the default clusters",
            json!({ "deployments": ["d"], "maxSolPerTransaction": "1" }),
        ),
        (
            "full policy",
            json!({ "deployments": ["d-1"], "maxSolPerTransaction": 0.05, "clusters": ["mainnet"], "programs": [addr(PROGRAM)], "maxTokenAmounts": { mint.clone(): "1000" } }),
        ),
        (
            "float SOL converts without rounding",
            json!({ "deployments": ["d"], "maxSolPerTransaction": 0.1 }),
        ),
        (
            "nine decimals are allowed",
            json!({ "deployments": ["d"], "maxSolPerTransaction": "1.000000001" }),
        ),
        (
            "integer SOL",
            json!({ "deployments": ["d"], "maxSolPerTransaction": 2 }),
        ),
        (
            "a float SOL in exponent form",
            json!({ "deployments": ["d"], "maxSolPerTransaction": 1e-7 }),
        ),
        (
            "zero SOL",
            json!({ "deployments": ["d"], "maxSolPerTransaction": "0" }),
        ),
        (
            "a leading decimal point",
            json!({ "deployments": ["d"], "maxSolPerTransaction": ".5" }),
        ),
        (
            "a trailing decimal point",
            json!({ "deployments": ["d"], "maxSolPerTransaction": "3." }),
        ),
        (
            "whitespace around a SOL string",
            json!({ "deployments": ["d"], "maxSolPerTransaction": " 0.25 " }),
        ),
        (
            "integer token caps",
            json!({ "deployments": ["d"], "maxSolPerTransaction": "1", "maxTokenAmounts": { mint.clone(): 42 } }),
        ),
        (
            "listing dev only",
            json!({ "deployments": ["d"], "maxSolPerTransaction": "1", "clusters": ["dev"] }),
        ),
        (
            "localnet, from earlier configs, is read as dev",
            json!({ "deployments": ["d"], "maxSolPerTransaction": "1", "clusters": ["localnet", "dev", "mainnet"] }),
        ),
        (
            "devnet and testnet, from earlier configs, are dropped",
            json!({ "deployments": ["d"], "maxSolPerTransaction": "1", "clusters": ["devnet", "testnet"] }),
        ),
        (
            "an empty programs list allows only runtime programs",
            json!({ "deployments": ["d"], "maxSolPerTransaction": "1", "programs": [] }),
        ),
        (
            "ten decimals are refused",
            json!({ "deployments": ["d"], "maxSolPerTransaction": "0.0000000001" }),
        ),
        (
            "non-numeric SOL is refused",
            json!({ "deployments": ["d"], "maxSolPerTransaction": "abc" }),
        ),
        (
            "negative SOL is refused",
            json!({ "deployments": ["d"], "maxSolPerTransaction": -1 }),
        ),
        (
            "a lone decimal point is refused",
            json!({ "deployments": ["d"], "maxSolPerTransaction": "." }),
        ),
        (
            "SOL that overflows lamports is refused",
            json!({ "deployments": ["d"], "maxSolPerTransaction": "18446744074" }),
        ),
        (
            "a boolean SOL is refused",
            json!({ "deployments": ["d"], "maxSolPerTransaction": true }),
        ),
        (
            "no deployments is refused",
            json!({ "deployments": [], "maxSolPerTransaction": 1 }),
        ),
        (
            "missing maxSolPerTransaction is refused",
            json!({ "deployments": ["d"] }),
        ),
        (
            "missing deployments is refused",
            json!({ "maxSolPerTransaction": 1 }),
        ),
        (
            "an unknown cluster is refused",
            json!({ "deployments": ["d"], "maxSolPerTransaction": 1, "clusters": ["moon"] }),
        ),
        (
            "a program that is not a pubkey is refused",
            json!({ "deployments": ["d"], "maxSolPerTransaction": 1, "programs": ["nope"] }),
        ),
        (
            "a mint that is not a pubkey is refused",
            json!({ "deployments": ["d"], "maxSolPerTransaction": 1, "maxTokenAmounts": { "nope": "1" } }),
        ),
        (
            "a negative token cap is refused",
            json!({ "deployments": ["d"], "maxSolPerTransaction": 1, "maxTokenAmounts": { mint.clone(): -1 } }),
        ),
        (
            "a non-digit token cap is refused",
            json!({ "deployments": ["d"], "maxSolPerTransaction": 1, "maxTokenAmounts": { mint.clone(): "1e3" } }),
        ),
        (
            "an empty token cap is refused",
            json!({ "deployments": ["d"], "maxSolPerTransaction": 1, "maxTokenAmounts": { mint.clone(): "" } }),
        ),
        (
            "a fractional token cap is refused",
            json!({ "deployments": ["d"], "maxSolPerTransaction": 1, "maxTokenAmounts": { mint.clone(): 1.5 } }),
        ),
        (
            "an unknown field is refused",
            json!({ "deployments": ["d"], "maxSolPerTransaction": 1, "surprise": true }),
        ),
    ];
    inputs
        .into_iter()
        .map(|(name, input)| {
            let expect = match SignerPolicy::from_json(input.clone()) {
                Ok(policy) => json!({ "policy": policy_json(&policy) }),
                Err(_) => json!({ "error": true }),
            };
            json!({ "name": name, "input": input, "expect": expect })
        })
        .collect()
}

fn refusal_json(result: Result<(), crate::signer::Refusal>) -> Value {
    match result {
        Ok(()) => Value::Null,
        Err(refusal) => refusal.to_json(),
    }
}

fn target_cases() -> Vec<Value> {
    let cases: Vec<(&str, Value, &str, &str)> = vec![
        (
            "an allowed deployment and cluster",
            base_policy(),
            "dep-1",
            "dev",
        ),
        (
            "a deployment outside the allow-list",
            base_policy(),
            "dep-2",
            "dev",
        ),
        (
            "a cluster outside the listed clusters",
            policy_with(json!({ "clusters": ["mainnet"] })),
            "dep-1",
            "dev",
        ),
        (
            "mainnet is excluded by default",
            json!({ "deployments": ["dep-1"], "maxSolPerTransaction": "1" }),
            "dep-1",
            "mainnet",
        ),
        (
            "dev is allowed by default",
            json!({ "deployments": ["dep-1"], "maxSolPerTransaction": "1" }),
            "dep-1",
            "dev",
        ),
        (
            "mainnet when listed",
            policy_with(json!({ "clusters": ["mainnet"] })),
            "dep-1",
            "mainnet",
        ),
        ("mainnet when not listed", base_policy(), "dep-1", "mainnet"),
    ];
    cases
        .into_iter()
        .map(|(name, policy, deployment, cluster)| {
            let expect = refusal_json(
                SignerPolicy::from_json(policy.clone())
                    .unwrap()
                    .check_target(deployment, cluster),
            );
            json!({ "name": name, "input": { "policy": policy, "deploymentId": deployment, "cluster": cluster }, "expect": expect })
        })
        .collect()
}

fn decoded_json(bytes: &[u8]) -> Value {
    match decode_transaction(bytes) {
        Err(_) => json!({ "error": true }),
        Ok(decoded) => json!({
            "version": decoded.version,
            "requiredSignatures": decoded.required_signatures,
            "staticKeys": decoded.static_keys,
            "writableStatic": decoded.writable_static,
            "writableStaticKeys": decoded.writable_static_keys(),
            "instructions": decoded.instructions.iter().map(|instruction| json!({
                "programIndex": instruction.program_index,
                "accounts": instruction.accounts,
                "data": b64(&instruction.data),
            })).collect::<Vec<_>>(),
            "lookups": decoded.lookups.iter().map(|lookup| json!({
                "table": lookup.table,
                "writable": lookup.writable,
                "readonly": lookup.readonly,
            })).collect::<Vec<_>>(),
            "message": [decoded.message.start, decoded.message.end],
            "signatureOffset": decoded.signature_offset,
            "signers": decoded.signers().iter().map(|(key, signed)| json!({ "key": key, "signed": signed })).collect::<Vec<_>>(),
            "programs": decoded.programs(),
        }),
    }
}

fn decode_cases() -> Vec<Value> {
    let legacy = Message::new(&[seed(1), seed(2)], &[seed(3)], &[seed(4), seed(5)])
        .with(ix(3, &[0], &[9]))
        .with(ix(4, &[0, 2], &[]))
        .transaction(&[false, true]);
    let mut readonly_signed = Message::new(&[seed(1), seed(2)], &[seed(3)], &[seed(4)]);
    readonly_signed.header = [2, 1, 1];
    let readonly_signed = readonly_signed.with(ix(3, &[0, 1, 2], &[1])).unsigned();
    let v0 = Message::new(&[seed(1)], &[], &[seed(4)])
        .with(ix(1, &[], &[]))
        .v0(vec![
            (seed(8), vec![0, 3], vec![5]),
            (seed(9), vec![], vec![1, 2]),
        ])
        .unsigned();
    let many_keys: Vec<[u8; 32]> = (10..140).map(seed).collect();
    let wide = Message::new(&[seed(1)], &many_keys, &[seed(4)])
        .with(ix(131, &(0..130).collect::<Vec<u8>>(), &[7; 200]))
        .unsigned();
    let v1 = v1_transaction(
        [1, 0, 1],
        0b100,
        &[seed(1), seed(4)],
        &[ix(1, &[0], &[9, 9])],
        1,
    );
    let v1_all_config = v1_transaction(
        [2, 0, 1],
        0b11111,
        &[seed(1), seed(2), seed(4)],
        &[ix(2, &[0, 1], &[1]), ix(2, &[1], &[])],
        2,
    );
    let mut v1_trailing = v1.clone();
    v1_trailing.push(0);
    let v1_unknown_config = v1_transaction([1, 0, 1], 0b100000, &[seed(1), seed(4)], &[], 1);
    let mut unsupported_version = Message::new(&[seed(1)], &[], &[seed(4)])
        .with(ix(1, &[], &[]))
        .v0(vec![])
        .unsigned();
    unsupported_version[65] = 0x82;
    let mut bad_header = Message::new(&[seed(1)], &[], &[seed(4)]);
    bad_header.header = [3, 0, 0];
    let bad_header = bad_header.with(ix(1, &[], &[])).unsigned();
    let mut readonly_overflow = Message::new(&[seed(1)], &[], &[seed(4)]);
    readonly_overflow.header = [1, 2, 0];
    let readonly_overflow = readonly_overflow.with(ix(1, &[], &[])).unsigned();
    let program_out_of_range = Message::new(&[seed(1)], &[], &[seed(4)])
        .with(ix(9, &[], &[]))
        .unsigned();
    let mut bad_compact = vec![1];
    bad_compact.extend_from_slice(&[0; 64]);
    bad_compact.extend_from_slice(&[1, 0, 0, 0xff, 0xff, 0xff, 0x01]);
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("legacy with two signers, one signed", legacy.clone()),
        ("legacy with a read-only signer", readonly_signed),
        ("v0 with two lookup tables", v0),
        ("legacy with 130 keys (two-byte compact lengths)", wide),
        ("v1 with one config value", v1),
        ("v1 with every config value", v1_all_config),
        ("truncated legacy", legacy[..legacy.len() - 3].to_vec()),
        ("empty bytes", Vec::new()),
        ("v1 with trailing bytes", v1_trailing),
        ("v1 with an unknown config bit", v1_unknown_config),
        ("an unsupported message version", unsupported_version),
        ("more required signatures than keys", bad_header),
        ("more read-only signers than signers", readonly_overflow),
        (
            "an instruction program outside the keys",
            program_out_of_range,
        ),
        ("an over-long compact length", bad_compact),
    ];
    cases
        .into_iter()
        .map(|(name, bytes)| {
            json!({ "name": name, "input": { "transactionBase64": b64(&bytes) }, "expect": decoded_json(&bytes) })
        })
        .collect()
}

fn checked_json(checked: Checked) -> Value {
    json!({
        "findings": checked.findings.iter().map(crate::authority::Finding::to_json).collect::<Vec<_>>(),
        "closedToOthers": checked.closed_to_others,
    })
}

fn authority_cases() -> Vec<Value> {
    let token = TOKEN_PROGRAMS[0];
    let token22 = TOKEN_PROGRAMS[1];
    // (name, program, accounts over [signer = 0, other = 1, program = 2], data)
    let mut top_level: Vec<(String, &str, Vec<u8>, Vec<u8>)> = Vec::new();
    let token_tags: [(u8, &str); 21] = [
        (3, "transfer"),
        (4, "approve"),
        (5, "revoke"),
        (6, "setAuthority"),
        (7, "mintTo"),
        (8, "burn"),
        (10, "freezeAccount"),
        (12, "transferChecked"),
        (13, "approveChecked"),
        (15, "burnChecked"),
        (17, "syncNative"),
        (18, "initializeAccount3"),
        (21, "getAccountDataSize"),
        (22, "initializeImmutableOwner"),
        (25, "initializeMintCloseAuthority"),
        (27, "tokenInstruction27"),
        (29, "reallocate"),
        (31, "createNativeMint"),
        (38, "withdrawExcessLamports"),
        (40, "tokenInstruction40"),
        (255, "tokenInstruction255"),
    ];
    for (program, label) in [(token, "SPL Token"), (token22, "Token-2022")] {
        for (tag, name) in token_tags {
            top_level.push((
                format!("{label} {name}"),
                program,
                vec![1, 1, 0],
                vec![tag, 1],
            ));
        }
        top_level.push((
            format!("{label} transferCheckedWithFee"),
            program,
            vec![1, 1, 0],
            vec![26, 1],
        ));
        top_level.push((
            format!("{label} other transfer-fee instruction"),
            program,
            vec![1, 1, 0],
            vec![26, 2],
        ));
        top_level.push((format!("{label} empty data"), program, vec![1, 0], vec![]));
    }
    let system_tags: [(u32, &str); 14] = [
        (0, "createAccount"),
        (1, "assign"),
        (2, "transfer"),
        (3, "createAccountWithSeed"),
        (4, "advanceNonce"),
        (5, "withdrawFromNonce"),
        (6, "initializeNonce"),
        (7, "authorizeNonce"),
        (8, "allocate"),
        (9, "allocateWithSeed"),
        (10, "assignWithSeed"),
        (11, "transferWithSeed"),
        (12, "upgradeNonce"),
        (13, "systemInstruction13"),
    ];
    for (tag, name) in system_tags {
        top_level.push((
            format!("System {name}"),
            SYSTEM_PROGRAM,
            vec![0, 1],
            u32_data(tag),
        ));
    }
    top_level.push((
        "System data too short for a tag".into(),
        SYSTEM_PROGRAM,
        vec![0, 1],
        vec![2, 0],
    ));
    for tag in 0..=16u32 {
        top_level.push((
            format!("Stake instruction {tag}"),
            STAKE_PROGRAM,
            vec![1, 0],
            u32_data(tag),
        ));
    }
    for tag in [0u32, 3, 4, 5] {
        top_level.push((
            format!("Upgradeable loader instruction {tag}"),
            UPGRADEABLE_LOADER,
            vec![1, 0],
            u32_data(tag),
        ));
    }
    top_level.push((
        "Upgradeable loader empty data".into(),
        UPGRADEABLE_LOADER,
        vec![1, 0],
        vec![],
    ));
    top_level.push((
        "approve without the signer's accounts".into(),
        token,
        vec![1, 1],
        vec![4],
    ));
    top_level.push((
        "an unchecked program with the signer".into(),
        "",
        vec![0, 1],
        vec![4],
    ));
    top_level.push((
        "closeAccount to another account".into(),
        token,
        vec![1, 1, 0],
        vec![9],
    ));
    top_level.push((
        "closeAccount to the signer".into(),
        token,
        vec![1, 0, 0],
        vec![9],
    ));
    top_level.push((
        "Token-2022 closeAccount to another account".into(),
        token22,
        vec![1, 1, 0],
        vec![9],
    ));

    let mut cases = Vec::new();
    for (name, program, accounts, data) in top_level {
        let program_key = if program.is_empty() {
            seed(PROGRAM)
        } else {
            raw(program)
        };
        let bytes = Message::new(&[seed(1)], &[seed(2)], &[program_key])
            .with(ix(2, &accounts, &data))
            .unsigned();
        let decoded = decode_transaction(&bytes).unwrap();
        let expect = checked_json(check_top_level(&decoded, &signer(), &[]));
        cases.push(json!({
            "name": format!("top level: {name}"),
            "input": { "kind": "topLevel", "transactionBase64": b64(&bytes), "signer": signer(), "lookupKeys": [] },
            "expect": expect,
        }));
    }

    // The signer is not a required signer: nothing it holds can be used.
    let not_signer = Message::new(&[seed(2)], &[seed(1)], &[raw(token)])
        .with(ix(2, &[1, 1, 0], &[4]))
        .unsigned();
    // closeAccount of lookup-table accounts, resolved and not.
    let lookup_close = Message::new(&[seed(1)], &[seed(2)], &[raw(token)])
        .with(ix(2, &[3, 1, 0], &[9]))
        .with(ix(2, &[4, 1, 0], &[9]))
        .v0(vec![(seed(LOOKUP_TABLE), vec![0, 1], vec![])])
        .unsigned();
    let extra: Vec<(&str, Vec<u8>, Value)> = vec![
        (
            "top level: approve by a wallet that does not sign",
            not_signer,
            json!([]),
        ),
        (
            "top level: closeAccount of lookup-table accounts",
            lookup_close,
            json!([addr(60), null]),
        ),
    ];
    for (name, bytes, lookup_keys) in extra {
        let decoded = decode_transaction(&bytes).unwrap();
        let keys: Vec<Option<String>> = serde_json::from_value(lookup_keys.clone()).unwrap();
        cases.push(json!({
            "name": name,
            "input": { "kind": "topLevel", "transactionBase64": b64(&bytes), "signer": signer(), "lookupKeys": lookup_keys },
            "expect": checked_json(check_top_level(&decoded, &signer(), &keys)),
        }));
    }

    let s = signer();
    let inner_cases: Vec<(&str, Value, Value)> = vec![
        (
            "parsed and partially decoded inner instructions",
            json!([{ "index": 0, "instructions": [
                { "programId": token, "program": "spl-token", "parsed": {
                    "type": "approve", "info": { "source": "S", "delegate": "D", "owner": s, "amount": "1" } } },
                { "programId": token, "program": "spl-token", "parsed": {
                    "type": "transfer", "info": { "source": "S", "destination": "T", "authority": s, "amount": "1" } } },
                { "programId": token, "program": "spl-token", "parsed": {
                    "type": "setAuthority", "info": { "account": "S", "authority": "someone-else" } } },
                { "programId": SYSTEM_PROGRAM, "accounts": [s], "data": b58(&u32_data(1)) },
                { "programId": token22, "program": "spl-token-2022", "parsed": {
                    "type": "closeAccount", "info": { "account": "A", "destination": "elsewhere", "owner": s } } },
            ] }]),
            json!([]),
        ),
        (
            "compiled inner instructions resolved through account keys",
            json!([{ "index": 0, "instructions": [
                { "programIdIndex": 2, "accounts": [0, 1], "data": b58(&u32_data(1)) },
                { "programIdIndex": 2, "accounts": [1, 0], "data": b58(&u32_data(2)) },
                { "programIdIndex": 3, "accounts": [4, 1, 0], "data": b58(&[9]) },
                { "programIdIndex": 3, "accounts": [4, 0, 0], "data": b58(&[9]) },
                { "programIdIndex": 3, "accounts": [4, 1, 0], "data": b58(&[6, 2]) },
            ] }, { "index": 1, "instructions": [
                { "programIdIndex": 9, "accounts": [0], "data": b58(&[4]) },
                { "programIdIndex": 3, "accounts": [9, 1], "data": b58(&[4]) },
            ] }]),
            json!([s, addr(2), SYSTEM_PROGRAM, token, null]),
        ),
        (
            "parsed instructions of other programs and without the signer",
            json!([{ "index": 0, "instructions": [
                { "programId": addr(PROGRAM), "parsed": { "type": "approve", "info": { "owner": s } } },
                { "programId": token, "parsed": { "type": "approve", "info": { "owner": "someone-else", "nested": { "list": ["x"] } } } },
                { "programId": token, "parsed": { "type": "approve", "info": { "owner": "x", "signers": ["y", s] } } },
                { "programId": token, "parsed": { "info": { "owner": s } } },
                { "programId": token, "parsed": "approve" },
                { "programId": SYSTEM_PROGRAM, "parsed": { "type": "assign", "info": { "account": s, "owner": "P" } } },
                { "programId": STAKE_PROGRAM, "parsed": { "type": "withdraw", "info": { "withdrawAuthority": s } } },
                { "programId": token, "parsed": { "type": "closeAccount", "info": { "account": "A", "destination": s, "owner": s } } },
            ] }]),
            json!([]),
        ),
        (
            "inner instructions with missing or malformed fields",
            json!([{ "index": 0, "instructions": [
                { "accounts": [s], "data": b58(&u32_data(1)) },
                { "programId": SYSTEM_PROGRAM, "data": b58(&u32_data(1)) },
                { "programId": SYSTEM_PROGRAM, "accounts": [s], "data": "0OIl" },
                { "programId": SYSTEM_PROGRAM, "accounts": [s, true] },
                { "programId": UPGRADEABLE_LOADER, "accounts": [{ "pubkey": s }, s], "data": b58(&u32_data(4)) },
            ] }, { "index": 1 }, "not an object"]),
            json!([]),
        ),
        ("no inner instructions", json!(null), json!([])),
    ];
    for (name, inner, account_keys) in inner_cases {
        let keys: Vec<Option<String>> = serde_json::from_value(account_keys.clone()).unwrap();
        cases.push(json!({
            "name": format!("inner: {name}"),
            "input": { "kind": "inner", "innerInstructions": inner, "signer": signer(), "accountKeys": account_keys },
            "expect": checked_json(check_inner(&inner, &signer(), &keys)),
        }));
    }
    cases
}

/// A summary with RPC error text replaced, since its wording is the
/// implementation's own.
fn normalize_summary(mut summary: Value) -> Value {
    for entry in summary["transactions"].as_array_mut().into_iter().flatten() {
        if entry["signerEffect"].get("error").is_some() {
            entry["signerEffect"]["error"] = json!("rpc_error");
        }
    }
    summary
}

async fn run_summary(prepared: &Value, chain: &Value) -> Value {
    let rpc = (!chain.is_null()).then(|| fixture_rpc(chain));
    summarize(prepared, &signer(), rpc.as_ref()).await.unwrap()
}

fn summary_inputs() -> Vec<(&'static str, Value, Value)> {
    let mut with_roles = prepared("dep-1", &[program_token_move()]);
    with_roles["anatomy"] = json!([{ "index": 0, "instructions": [
        { "role": "program", "kind": "deposit", "programId": addr(PROGRAM) },
        { "role": "program", "kind": "program", "programId": addr(PROGRAM) },
        { "role": "runtime", "programId": TOKEN_PROGRAMS[0] },
    ] }]);
    with_roles["simulation"] = json!([{ "index": 0, "unitsConsumed": "1200" }]);
    let mut dependent = prepared("dep-1", &[program_token_move()]);
    dependent["simulation"] = json!([{ "index": 0, "simulated": false }]);
    let mut two_chains = token_move_chain(400);
    for (address, account) in sol_transfer_chain()["simulation"]["accounts"]
        .as_object()
        .unwrap()
    {
        if address != &signer() {
            two_chains["simulation"]["accounts"][address] = account.clone();
        }
    }
    vec![
        (
            "without a simulator",
            prepared("dep-1", &[program_token_move()]),
            Value::Null,
        ),
        (
            "top-level approve without a simulator",
            prepared("dep-1", &[top_level_approve()]),
            Value::Null,
        ),
        (
            "a SOL transfer",
            prepared("dep-1", &[sol_transfer()]),
            sol_transfer_chain(),
        ),
        (
            "a token outflow",
            prepared("dep-1", &[program_token_move()]),
            token_move_chain(400),
        ),
        (
            "a token inflow",
            prepared("dep-1", &[program_token_move()]),
            token_move_chain(1500),
        ),
        (
            "a token account handed to another owner",
            prepared("dep-1", &[program_token_move()]),
            handed_over_chain(),
        ),
        (
            "wrapped SOL spent",
            prepared("dep-1", &[program_wrapped_move()]),
            wrapped_spent_chain(),
        ),
        (
            "SOL wrapped",
            prepared("dep-1", &[program_wrapped_move()]),
            wrapping_chain(),
        ),
        (
            "a wrapped SOL account closed to another wallet",
            prepared("dep-1", &[close_to_other()]),
            close_to_other_chain(),
        ),
        (
            "a close of an unresolved lookup-table account",
            prepared("dep-1", &[close_from_unresolved_lookup()]),
            unresolved_lookup_chain(),
        ),
        (
            "lookup-table writes resolved",
            prepared("dep-1", &[program_with_lookups()]),
            lookups_chain(),
        ),
        (
            "more written accounts than a simulation reports",
            prepared("dep-1", &[program_writing_many()]),
            many_writes_chain(),
        ),
        ("a failed simulation", dependent, failed_simulation_chain()),
        (
            "an RPC error",
            prepared("dep-1", &[program_token_move()]),
            rpc_error_chain(),
        ),
        (
            "an inner approve",
            prepared("dep-1", &[program_token_move()]),
            inner_approve_chain(),
        ),
        (
            "two transactions",
            prepared("dep-1", &[sol_transfer(), program_token_move()]),
            two_chains,
        ),
        (
            "anatomy roles and server simulation",
            with_roles,
            token_move_chain(400),
        ),
    ]
}

async fn summary_cases() -> Vec<Value> {
    let mut cases = Vec::new();
    for (name, prepared, chain) in summary_inputs() {
        let summary = normalize_summary(run_summary(&prepared, &chain).await);
        cases.push(json!({
            "name": name,
            "input": { "prepared": prepared, "signer": signer(), "chain": chain },
            "expect": summary,
        }));
    }
    cases
}

async fn check_cases() -> Vec<Value> {
    let mint = addr(MINT);
    let caps = |cap: &str| json!({ "maxTokenAmounts": { mint.clone(): cap } });
    let mut two_chains = sol_transfer_chain();
    for (address, account) in token_move_chain(400)["accounts"].as_object().unwrap() {
        if address != &signer() {
            two_chains["accounts"][address] = account.clone();
        }
    }
    for (address, account) in token_move_chain(400)["simulation"]["accounts"]
        .as_object()
        .unwrap()
    {
        if address != &signer() {
            two_chains["simulation"]["accounts"][address] = account.clone();
        }
    }
    let token_policy = policy_with(
        json!({ "programs": [addr(PROGRAM)], "maxTokenAmounts": { mint.clone(): "1000" } }),
    );
    let token_move = || prepared("dep-1", &[program_token_move()]);
    let cases: Vec<(&str, Value, &str, Value, Value)> = vec![
        (
            "a token move within every cap",
            token_policy.clone(),
            "dep-1",
            token_move(),
            token_move_chain(400),
        ),
        (
            "a deployment outside the allow-list",
            policy_with(json!({ "deployments": ["dep-1"] })),
            "dep-2",
            prepared("dep-2", &[sol_transfer()]),
            sol_transfer_chain(),
        ),
        (
            "an intent for another deployment",
            base_policy(),
            "dep-1",
            prepared("dep-9", &[sol_transfer()]),
            sol_transfer_chain(),
        ),
        (
            "a program outside the allow-list",
            policy_with(
                json!({ "programs": [addr(OTHER_PROGRAM)], "maxTokenAmounts": { mint.clone(): "1000" } }),
            ),
            "dep-1",
            token_move(),
            token_move_chain(400),
        ),
        (
            "runtime programs need no listing",
            policy_with(json!({ "programs": [] })),
            "dep-1",
            prepared("dep-1", &[sol_transfer()]),
            sol_transfer_chain(),
        ),
        (
            "no programs allow-list allows any program",
            policy_with(caps("1000")),
            "dep-1",
            token_move(),
            token_move_chain(400),
        ),
        (
            "a top-level approve",
            base_policy(),
            "dep-1",
            prepared("dep-1", &[top_level_approve()]),
            token_move_chain(1000),
        ),
        (
            "an inner approve",
            policy_with(caps("1000")),
            "dep-1",
            token_move(),
            inner_approve_chain(),
        ),
        (
            "without a simulator",
            base_policy(),
            "dep-1",
            prepared("dep-1", &[sol_transfer()]),
            Value::Null,
        ),
        (
            "more written accounts than a simulation reports",
            base_policy(),
            "dep-1",
            prepared("dep-1", &[program_writing_many()]),
            many_writes_chain(),
        ),
        (
            "a close the simulation could not measure",
            base_policy(),
            "dep-1",
            prepared("dep-1", &[close_from_unresolved_lookup()]),
            unresolved_lookup_chain(),
        ),
        (
            "a failed simulation",
            base_policy(),
            "dep-1",
            token_move(),
            failed_simulation_chain(),
        ),
        (
            "an RPC error",
            base_policy(),
            "dep-1",
            token_move(),
            rpc_error_chain(),
        ),
        (
            "SOL over the cap",
            policy_with(json!({ "maxSolPerTransaction": "0.005" })),
            "dep-1",
            prepared("dep-1", &[sol_transfer()]),
            sol_transfer_chain(),
        ),
        (
            "SOL exactly at the cap",
            policy_with(json!({ "maxSolPerTransaction": "0.010005" })),
            "dep-1",
            prepared("dep-1", &[sol_transfer()]),
            sol_transfer_chain(),
        ),
        (
            "wrapped SOL spent counts as SOL",
            policy_with(json!({ "maxSolPerTransaction": "0.0005" })),
            "dep-1",
            prepared("dep-1", &[program_wrapped_move()]),
            wrapped_spent_chain(),
        ),
        (
            "wrapping SOL is not an outflow",
            policy_with(json!({ "maxSolPerTransaction": "0.00001" })),
            "dep-1",
            prepared("dep-1", &[program_wrapped_move()]),
            wrapping_chain(),
        ),
        (
            "a close to another wallet counts as SOL",
            policy_with(json!({ "maxSolPerTransaction": "0.001" })),
            "dep-1",
            prepared("dep-1", &[close_to_other()]),
            close_to_other_chain(),
        ),
        (
            "a token outflow without maxTokenAmounts",
            base_policy(),
            "dep-1",
            token_move(),
            token_move_chain(400),
        ),
        (
            "a token outflow of a mint without a cap",
            policy_with(json!({ "maxTokenAmounts": { addr(OTHER_MINT): "1000" } })),
            "dep-1",
            token_move(),
            token_move_chain(400),
        ),
        (
            "a token outflow over the cap",
            policy_with(caps("599")),
            "dep-1",
            token_move(),
            token_move_chain(400),
        ),
        (
            "a token outflow exactly at the cap",
            policy_with(caps("600")),
            "dep-1",
            token_move(),
            token_move_chain(400),
        ),
        (
            "a token inflow without caps",
            base_policy(),
            "dep-1",
            token_move(),
            token_move_chain(1500),
        ),
        (
            "a handed-over token account counts in full",
            policy_with(caps("400")),
            "dep-1",
            token_move(),
            handed_over_chain(),
        ),
        (
            "the second of two transactions over a cap",
            base_policy(),
            "dep-1",
            prepared("dep-1", &[sol_transfer(), program_token_move()]),
            two_chains,
        ),
    ];
    let mut out = Vec::new();
    for (name, policy, deployment, prepared, chain) in cases {
        let summary = run_summary(&prepared, &chain).await;
        let simulates = !chain.is_null();
        let expect = refusal_json(
            SignerPolicy::from_json(policy.clone())
                .unwrap()
                .check_prepared(deployment, &prepared, &summary, simulates),
        );
        let mut case = json!({
            "name": name,
            "input": { "policy": policy, "deploymentId": deployment, "prepared": prepared, "signer": signer(), "chain": chain },
            "expect": expect,
        });
        // Without a simulator the reason names this signer's config key;
        // other implementations word that hint for their own setup.
        if !simulates {
            case["compareReason"] = json!(false);
        }
        out.push(case);
    }
    out
}

fn document(description: &str, cases: Vec<Value>) -> Value {
    json!({ "version": 1, "description": description, "cases": cases })
}

async fn documents() -> Vec<(&'static str, Value)> {
    vec![
        (
            "config.json",
            document(
                "Signer policy fields (as JSON) and the normalized policy, or an error.",
                config_cases(),
            ),
        ),
        (
            "target.json",
            document(
                "check_target: whether a deployment and cluster are allowed before preparing. `expect` is null or the refusal.",
                target_cases(),
            ),
        ),
        (
            "decode.json",
            document(
                "Legacy, v0 and v1 wire transactions and what decoding them yields, or an error.",
                decode_cases(),
            ),
        ),
        (
            "authority.json",
            document(
                "Instructions that hand control of the signer's funds to someone else: top-level (from wire bytes) and inner (from a simulation).",
                authority_cases(),
            ),
        ),
        (
            "summary.json",
            document(
                "The summary of a prepared step over a fixture chain (see README.md). `signerEffect.error` is normalized to \"rpc_error\".",
                summary_cases().await,
            ),
        ),
        (
            "check.json",
            document(
                "check_prepared over the summary of a prepared step. `expect` is null or the refusal; compare `reason` unless `compareReason` is false.",
                check_cases().await,
            ),
        ),
    ]
}

fn vectors_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../test-vectors/signer-policy")
}

#[tokio::test]
async fn signer_policy_vectors_are_current() {
    let dir = vectors_dir();
    let write = std::env::var(WRITE_ENV).is_ok_and(|value| value == "1");
    let mut stale = Vec::new();
    for (file, expected) in documents().await {
        let path = dir.join(file);
        if write {
            std::fs::create_dir_all(&dir).unwrap();
            let mut text = serde_json::to_string_pretty(&expected).unwrap();
            text.push('\n');
            std::fs::write(&path, text).unwrap();
            continue;
        }
        let on_disk: Value = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or(Value::Null);
        if on_disk != expected {
            stale.push(file);
        }
    }
    assert!(
        stale.is_empty(),
        "signer-policy vectors {stale:?} differ from what the signer computes; regenerate them with {WRITE_ENV}=1 cargo test -p arete-mcp --lib signer_vectors"
    );
}
