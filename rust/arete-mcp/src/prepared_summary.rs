//! What a prepared step's transactions do, for an agent to check before it
//! signs.
//!
//! A prepare answers with the wire transactions, the role-labelled
//! instructions (`anatomy`) and the server's simulation metrics. The summary
//! reads the transactions themselves rather than trusting the labels:
//!
//! - the programs each transaction invokes (labelled with the anatomy's
//!   `role`/`kind` where it names the same program);
//! - the accounts it writes (static keys, plus address lookup table entries,
//!   resolved when an RPC is available);
//! - the signatures it carries and the ones still required;
//! - with an RPC URL, the signer's SOL and token balance changes from a
//!   simulation of the exact bytes (`simulateTransaction` with the written
//!   accounts' post-state), the simulation error and the tail of its logs;
//! - the instructions that would hand control of the signer's funds to
//!   someone else (`unsafeInstructions`, see [`crate::authority`]): read
//!   from the top-level instructions always, and from the inner instructions
//!   when the simulation returns them.
//!
//! Without an RPC URL the balance changes are reported as unavailable, never
//! as zero.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{anyhow, bail, Context, Result};
use base64::Engine as _;
use serde_json::{json, Value};

/// Programs that own SPL token accounts.
pub const TOKEN_PROGRAMS: [&str; 2] = [
    "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
    "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb",
];
const LOOKUP_TABLE_HEADER: usize = 56;
const LOG_TAIL: usize = 20;
/// The most accounts one transaction's simulation reports on.
const MAX_SIMULATED_ACCOUNTS: usize = 64;

/// A decoded wire transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedTransaction {
    /// `None` for legacy messages, else the message version (0 or 1).
    pub version: Option<u8>,
    /// Signature slots, in key order; all zero when not signed yet.
    pub signatures: Vec<[u8; 64]>,
    /// Where the first signature slot starts in the wire bytes.
    pub signature_offset: usize,
    /// The bytes every signer signs.
    pub message: std::ops::Range<usize>,
    pub required_signatures: usize,
    pub static_keys: Vec<String>,
    /// Indexes of the static keys the message marks writable.
    pub writable_static: Vec<usize>,
    /// The program of each top-level instruction, in order.
    pub instruction_programs: Vec<String>,
    /// The top-level instructions, in order.
    pub instructions: Vec<CompiledInstruction>,
    pub lookups: Vec<Lookup>,
}

/// One top-level instruction: indexes into the message's account keys
/// (static keys, then lookup-table writable entries, then read-only ones).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledInstruction {
    pub program_index: usize,
    pub accounts: Vec<usize>,
    pub data: Vec<u8>,
}

/// One address lookup table a v0 message loads from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lookup {
    pub table: String,
    pub writable: Vec<u8>,
    pub readonly: Vec<u8>,
}

fn compact_u16(bytes: &[u8], at: &mut usize) -> Result<usize> {
    let mut value = 0usize;
    for index in 0..3 {
        let byte = *bytes
            .get(*at)
            .ok_or_else(|| anyhow!("truncated transaction"))?;
        *at += 1;
        value |= ((byte & 0x7f) as usize) << (7 * index);
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    bail!("invalid compact-u16 length")
}

fn take<'a>(bytes: &'a [u8], at: &mut usize, len: usize) -> Result<&'a [u8]> {
    let slice = bytes
        .get(*at..*at + len)
        .ok_or_else(|| anyhow!("truncated transaction"))?;
    *at += len;
    Ok(slice)
}

fn base58(bytes: &[u8]) -> String {
    bs58::encode(bytes).into_string()
}

/// The version byte of a v1 transaction, which leads its wire bytes.
const V1_PREFIX: u8 = 0x81;
/// Configuration mask bits a v1 message may set (priority fee, compute unit
/// limit, loaded accounts data size, heap size); each bit adds one 4-byte
/// value.
const V1_KNOWN_CONFIG_BITS: u32 = 0b11111;

fn writable_indexes(
    key_count: usize,
    required: usize,
    readonly_signed: usize,
    readonly_unsigned: usize,
) -> Result<Vec<usize>> {
    if required > key_count
        || readonly_signed > required
        || readonly_unsigned > key_count - required
    {
        bail!("invalid message header");
    }
    Ok((0..key_count)
        .filter(|&index| {
            if index < required {
                index < required - readonly_signed
            } else {
                index < key_count - readonly_unsigned
            }
        })
        .collect())
}

/// Decode a legacy, v0 or v1 wire transaction.
pub fn decode_transaction(bytes: &[u8]) -> Result<DecodedTransaction> {
    if bytes.first() == Some(&V1_PREFIX) {
        return decode_v1(bytes);
    }
    let mut at = 0;
    let signature_count = compact_u16(bytes, &mut at)?;
    let signature_offset = at;
    let mut signatures = Vec::with_capacity(signature_count);
    for _ in 0..signature_count {
        let mut signature = [0u8; 64];
        signature.copy_from_slice(take(bytes, &mut at, 64)?);
        signatures.push(signature);
    }
    let message_start = at;
    let first = *bytes.get(at).ok_or_else(|| anyhow!("truncated message"))?;
    let version = if first & 0x80 != 0 {
        at += 1;
        Some(first & 0x7f)
    } else {
        None
    };
    if version.is_some_and(|version| version != 0) {
        bail!("unsupported transaction version");
    }
    let header = take(bytes, &mut at, 3)?;
    let (required, readonly_signed, readonly_unsigned) =
        (header[0] as usize, header[1] as usize, header[2] as usize);
    let key_count = compact_u16(bytes, &mut at)?;
    let mut static_keys = Vec::with_capacity(key_count);
    for _ in 0..key_count {
        static_keys.push(base58(take(bytes, &mut at, 32)?));
    }
    let writable_static =
        writable_indexes(key_count, required, readonly_signed, readonly_unsigned)?;
    take(bytes, &mut at, 32)?; // recent blockhash
    let instruction_count = compact_u16(bytes, &mut at)?;
    let mut instruction_programs = Vec::with_capacity(instruction_count);
    let mut instructions = Vec::with_capacity(instruction_count);
    for _ in 0..instruction_count {
        let program = take(bytes, &mut at, 1)?[0] as usize;
        let accounts = compact_u16(bytes, &mut at)?;
        let accounts = take(bytes, &mut at, accounts)?;
        let data = compact_u16(bytes, &mut at)?;
        let data = take(bytes, &mut at, data)?;
        instructions.push(CompiledInstruction {
            program_index: program,
            accounts: accounts.iter().map(|&index| index as usize).collect(),
            data: data.to_vec(),
        });
        instruction_programs.push(
            static_keys
                .get(program)
                .cloned()
                .ok_or_else(|| anyhow!("an instruction's program is not a static key"))?,
        );
    }
    let mut lookups = Vec::new();
    if version.is_some() {
        let count = compact_u16(bytes, &mut at)?;
        for _ in 0..count {
            let table = base58(take(bytes, &mut at, 32)?);
            let writable_count = compact_u16(bytes, &mut at)?;
            let writable = take(bytes, &mut at, writable_count)?.to_vec();
            let readonly_count = compact_u16(bytes, &mut at)?;
            let readonly = take(bytes, &mut at, readonly_count)?.to_vec();
            lookups.push(Lookup {
                table,
                writable,
                readonly,
            });
        }
    }
    Ok(DecodedTransaction {
        version,
        signatures,
        signature_offset,
        message: message_start..bytes.len(),
        required_signatures: required,
        static_keys,
        writable_static,
        instruction_programs,
        instructions,
        lookups,
    })
}

/// A v1 transaction: the version byte and message, then one signature per
/// required signer.
fn decode_v1(bytes: &[u8]) -> Result<DecodedTransaction> {
    let mut at = 1;
    let header = take(bytes, &mut at, 3)?;
    let (required, readonly_signed, readonly_unsigned) =
        (header[0] as usize, header[1] as usize, header[2] as usize);
    let mut mask = [0u8; 4];
    mask.copy_from_slice(take(bytes, &mut at, 4)?);
    let mask = u32::from_le_bytes(mask);
    if mask & !V1_KNOWN_CONFIG_BITS != 0 {
        bail!("unsupported v1 transaction configuration");
    }
    take(bytes, &mut at, 32)?; // lifetime specifier
    let instruction_count = take(bytes, &mut at, 1)?[0] as usize;
    let key_count = take(bytes, &mut at, 1)?[0] as usize;
    let mut static_keys = Vec::with_capacity(key_count);
    for _ in 0..key_count {
        static_keys.push(base58(take(bytes, &mut at, 32)?));
    }
    take(bytes, &mut at, mask.count_ones() as usize * 4)?;
    let headers = take(bytes, &mut at, instruction_count * 4)?.to_vec();
    let mut instruction_programs = Vec::with_capacity(instruction_count);
    let mut instructions = Vec::with_capacity(instruction_count);
    for header in headers.chunks(4) {
        let program = header[0] as usize;
        let accounts = header[1] as usize;
        let data = u16::from_le_bytes([header[2], header[3]]) as usize;
        let accounts = take(bytes, &mut at, accounts)?;
        let data = take(bytes, &mut at, data)?;
        instructions.push(CompiledInstruction {
            program_index: program,
            accounts: accounts.iter().map(|&index| index as usize).collect(),
            data: data.to_vec(),
        });
        instruction_programs.push(
            static_keys
                .get(program)
                .cloned()
                .ok_or_else(|| anyhow!("an instruction's program is not an address"))?,
        );
    }
    let message_end = at;
    let signature_offset = at;
    let mut signatures = Vec::with_capacity(required);
    for _ in 0..required {
        let mut signature = [0u8; 64];
        signature.copy_from_slice(take(bytes, &mut at, 64)?);
        signatures.push(signature);
    }
    if at != bytes.len() {
        bail!("trailing bytes after the transaction");
    }
    let writable_static =
        writable_indexes(key_count, required, readonly_signed, readonly_unsigned)?;
    Ok(DecodedTransaction {
        version: Some(1),
        signatures,
        signature_offset,
        message: 0..message_end,
        required_signatures: required,
        static_keys,
        writable_static,
        instruction_programs,
        instructions,
        lookups: Vec::new(),
    })
}

impl DecodedTransaction {
    /// The required signers and whether each has signed.
    pub fn signers(&self) -> Vec<(String, bool)> {
        self.static_keys
            .iter()
            .take(self.required_signatures)
            .enumerate()
            .map(|(index, key)| {
                let signed = self
                    .signatures
                    .get(index)
                    .is_some_and(|signature| signature.iter().any(|byte| *byte != 0));
                (key.clone(), signed)
            })
            .collect()
    }

    /// The distinct programs invoked, in first-use order.
    pub fn programs(&self) -> Vec<String> {
        let mut seen = BTreeSet::new();
        self.instruction_programs
            .iter()
            .filter(|program| seen.insert(program.as_str()))
            .cloned()
            .collect()
    }

    pub fn writable_static_keys(&self) -> Vec<String> {
        self.writable_static
            .iter()
            .map(|&index| self.static_keys[index].clone())
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Simulation over RPC
// ---------------------------------------------------------------------------

/// A Solana JSON-RPC endpoint used to simulate prepared transactions.
/// Answers an RPC call in place of the endpoint (tests only).
#[cfg(test)]
type RpcFixture = std::sync::Arc<dyn Fn(&str, &Value) -> Result<Value> + Send + Sync>;

#[derive(Clone)]
pub struct Rpc {
    http: reqwest::Client,
    url: String,
    /// Answers calls in place of the endpoint, for tests that describe the
    /// chain as a fixture.
    #[cfg(test)]
    fixture: Option<RpcFixture>,
}

impl std::fmt::Debug for Rpc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Rpc").finish_non_exhaustive()
    }
}

impl Rpc {
    pub fn new(http: reqwest::Client, url: &str) -> Self {
        Self {
            http,
            url: url.to_string(),
            #[cfg(test)]
            fixture: None,
        }
    }

    /// An RPC whose calls `answer` handles: `Ok(result)` or `Err` with the
    /// JSON-RPC error message.
    #[cfg(test)]
    pub(crate) fn from_fixture(
        answer: impl Fn(&str, &Value) -> Result<Value> + Send + Sync + 'static,
    ) -> Self {
        Self {
            http: reqwest::Client::new(),
            url: String::new(),
            fixture: Some(std::sync::Arc::new(answer)),
        }
    }

    async fn call(&self, method: &str, params: Value) -> Result<Value> {
        #[cfg(test)]
        if let Some(answer) = &self.fixture {
            return answer(method, &params).map_err(|error| anyhow!("{method} failed: {error}"));
        }
        let response: Value = self
            .http
            .post(&self.url)
            .json(&json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }))
            .send()
            .await
            .with_context(|| format!("The RPC is unreachable for {method}"))?
            .json()
            .await
            .with_context(|| format!("The RPC answered {method} with invalid JSON"))?;
        if let Some(error) = response.get("error") {
            bail!(
                "{method} failed: {}",
                error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown error")
            );
        }
        Ok(response.get("result").cloned().unwrap_or(Value::Null))
    }

    /// The accounts at `addresses` and the highest slot the RPC read them
    /// at.
    async fn accounts_at(&self, addresses: &[String]) -> Result<(Vec<Value>, Option<u64>)> {
        let mut out = Vec::with_capacity(addresses.len());
        let mut slot: Option<u64> = None;
        for chunk in addresses.chunks(100) {
            let result = self
                .call(
                    "getMultipleAccounts",
                    json!([chunk, { "encoding": "base64", "commitment": "confirmed" }]),
                )
                .await?;
            let values = result["value"]
                .as_array()
                .ok_or_else(|| anyhow!("getMultipleAccounts returned no accounts"))?;
            if let Some(read) = result["context"]["slot"].as_u64() {
                slot = Some(slot.map_or(read, |slot| slot.max(read)));
            }
            out.extend(values.iter().cloned());
        }
        Ok((out, slot))
    }

    async fn accounts(&self, addresses: &[String]) -> Result<Vec<Value>> {
        Ok(self.accounts_at(addresses).await?.0)
    }
}

/// An SPL token account's mint, owner and amount.
type TokenFacts = (String, String, u128);

/// One account's lamports, and its SPL token balance when it is a token
/// account.
type AccountFacts = (u64, Option<TokenFacts>);

fn account_facts(account: &Value) -> Option<AccountFacts> {
    if account.is_null() {
        return None;
    }
    let lamports = account["lamports"].as_u64().unwrap_or(0);
    let owner = account["owner"].as_str().unwrap_or_default();
    let token = if TOKEN_PROGRAMS.contains(&owner) {
        account["data"]
            .get(0)
            .and_then(Value::as_str)
            .and_then(|data| base64::engine::general_purpose::STANDARD.decode(data).ok())
            .filter(|data| data.len() >= 72)
            .map(|data| {
                let mut amount = [0u8; 8];
                amount.copy_from_slice(&data[64..72]);
                (
                    base58(&data[0..32]),
                    base58(&data[32..64]),
                    u64::from_le_bytes(amount) as u128,
                )
            })
    } else {
        None
    };
    Some((lamports, token))
}

/// What simulating one transaction showed about the signer.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SignerEffect {
    /// Lamports: post minus pre. `None` when unknown.
    pub sol_lamports: Option<i128>,
    /// Per mint: the signer's balance after minus before, over the token
    /// accounts the transaction writes. An account counts as the signer's
    /// on each side when the signer owns it on that side, so an account
    /// handed to someone else shows its whole balance as an outflow.
    pub tokens: BTreeMap<String, i128>,
    /// Lamports of token accounts the signer closes to another account
    /// (their rent, and for wrapped SOL anything above the token amount).
    /// `None` when one of them could not be measured.
    pub closed_lamports_out: Option<u128>,
    pub err: Value,
    pub logs_tail: Vec<String>,
    /// Lookup-table accounts that could not be resolved.
    pub unresolved: Vec<String>,
    /// Written accounts left out of the simulation because the transaction
    /// writes more than it reports on.
    pub omitted_accounts: usize,
    /// Instructions the signer must not sign, top-level and inner.
    pub findings: Vec<crate::authority::Finding>,
    /// Whether the simulation returned inner instructions to check.
    pub inner_checked: bool,
    /// The slot the pre-state was read at and the slot simulated at.
    pub slots: (Option<u64>, Option<u64>),
}

impl SignerEffect {
    /// Whether every balance change of the signer is known.
    pub fn known(&self) -> bool {
        self.sol_lamports.is_some()
            && self.unresolved.is_empty()
            && self.omitted_accounts == 0
            && self.closed_lamports_out.is_some()
    }
}

/// The lookup-table addresses of a v0 message.
#[derive(Debug, Default)]
struct ResolvedLookups {
    /// Resolved writable entries.
    writable: Vec<String>,
    /// Every entry in message order (writable entries of each table, then
    /// read-only entries of each table); `None` where unresolved.
    keys: Vec<Option<String>>,
    /// Tables whose writable entries could not be resolved.
    unresolved: Vec<String>,
}

/// Resolve the lookup-table accounts of `decoded`.
async fn resolve_lookups(rpc: &Rpc, decoded: &DecodedTransaction) -> ResolvedLookups {
    let mut out = ResolvedLookups::default();
    if decoded.lookups.is_empty() {
        return out;
    }
    let tables: Vec<String> = decoded.lookups.iter().map(|l| l.table.clone()).collect();
    let accounts = rpc.accounts(&tables).await.unwrap_or_default();
    let table_addresses: Vec<Option<Vec<u8>>> = decoded
        .lookups
        .iter()
        .enumerate()
        .map(|(index, _)| {
            accounts
                .get(index)
                .and_then(|account| account["data"].get(0))
                .and_then(Value::as_str)
                .and_then(|data| base64::engine::general_purpose::STANDARD.decode(data).ok())
                .filter(|data| data.len() >= LOOKUP_TABLE_HEADER)
                .map(|data| data[LOOKUP_TABLE_HEADER..].to_vec())
        })
        .collect();
    let entry = |table: usize, entry: u8| -> Option<String> {
        let addresses = table_addresses[table].as_ref()?;
        let start = entry as usize * 32;
        addresses.get(start..start + 32).map(base58)
    };
    for (index, lookup) in decoded.lookups.iter().enumerate() {
        for &position in &lookup.writable {
            let key = entry(index, position);
            match &key {
                Some(key) => out.writable.push(key.clone()),
                None => {
                    if !out.unresolved.contains(&lookup.table) {
                        out.unresolved.push(lookup.table.clone());
                    }
                }
            }
            out.keys.push(key);
        }
    }
    for (index, lookup) in decoded.lookups.iter().enumerate() {
        for &position in &lookup.readonly {
            out.keys.push(entry(index, position));
        }
    }
    out
}

/// Simulate `transaction` (wire bytes) and measure its effect on `signer`.
///
/// The pre-state is read with `getMultipleAccounts` and the simulation is
/// asked not to run on an older slot (`minContextSlot`), but the two are
/// separate reads: an account that changes in between skews the deltas.
pub async fn simulate_signer_effect(
    rpc: &Rpc,
    transaction: &[u8],
    decoded: &DecodedTransaction,
    signer: &str,
) -> Result<SignerEffect> {
    let lookups = resolve_lookups(rpc, decoded).await;
    simulate_signer_effect_with_lookups(rpc, transaction, decoded, signer, &lookups).await
}

async fn simulate_signer_effect_with_lookups(
    rpc: &Rpc,
    transaction: &[u8],
    decoded: &DecodedTransaction,
    signer: &str,
    lookups: &ResolvedLookups,
) -> Result<SignerEffect> {
    let mut addresses: Vec<String> = Vec::new();
    for key in std::iter::once(signer.to_string())
        .chain(decoded.writable_static_keys())
        .chain(lookups.writable.iter().cloned())
    {
        if !addresses.contains(&key) {
            addresses.push(key);
        }
    }
    let omitted_accounts = addresses.len().saturating_sub(MAX_SIMULATED_ACCOUNTS);
    addresses.truncate(MAX_SIMULATED_ACCOUNTS);
    let account_keys: Vec<Option<String>> = decoded
        .static_keys
        .iter()
        .cloned()
        .map(Some)
        .chain(lookups.keys.iter().cloned())
        .collect();
    let mut checked = crate::authority::check_top_level(decoded, signer, &lookups.keys);
    let (before, pre_slot) = rpc.accounts_at(&addresses).await?;
    let mut config = json!({
        "encoding": "base64",
        "sigVerify": false,
        "replaceRecentBlockhash": true,
        "commitment": "confirmed",
        "innerInstructions": true,
        "accounts": { "encoding": "base64", "addresses": addresses },
    });
    if let Some(slot) = pre_slot {
        config["minContextSlot"] = json!(slot);
    }
    let result = rpc
        .call(
            "simulateTransaction",
            json!([
                base64::engine::general_purpose::STANDARD.encode(transaction),
                config
            ]),
        )
        .await?;
    let value = &result["value"];
    let logs: Vec<String> = value["logs"]
        .as_array()
        .map(|logs| {
            logs.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let logs_tail = logs[logs.len().saturating_sub(LOG_TAIL)..].to_vec();
    let err = value.get("err").cloned().unwrap_or(Value::Null);
    let inner_checked = value["innerInstructions"].is_array();
    checked.merge(crate::authority::check_inner(
        &value["innerInstructions"],
        signer,
        &account_keys,
    ));
    let slots = (pre_slot, result["context"]["slot"].as_u64());
    let after = value["accounts"].as_array().cloned().unwrap_or_default();
    if !err.is_null() || after.len() != addresses.len() {
        return Ok(SignerEffect {
            err,
            logs_tail,
            unresolved: lookups.unresolved.clone(),
            omitted_accounts,
            findings: checked.findings,
            inner_checked,
            slots,
            ..SignerEffect::default()
        });
    }
    let mut sol = None;
    let mut tokens: BTreeMap<String, i128> = BTreeMap::new();
    for (index, address) in addresses.iter().enumerate() {
        let pre = before.get(index).and_then(account_facts);
        let post = after.get(index).and_then(account_facts);
        if address == signer {
            let pre_lamports = pre.as_ref().map_or(0, |facts| facts.0) as i128;
            let post_lamports = post.as_ref().map_or(0, |facts| facts.0) as i128;
            sol = Some(post_lamports - pre_lamports);
        }
        let signers_side = |facts: Option<&AccountFacts>| -> Option<(String, i128)> {
            let (mint, owner, amount) = facts?.1.as_ref()?;
            (owner == signer).then(|| (mint.clone(), *amount as i128))
        };
        if let Some((mint, amount)) = signers_side(pre.as_ref()) {
            *tokens.entry(mint).or_default() -= amount;
        }
        if let Some((mint, amount)) = signers_side(post.as_ref()) {
            *tokens.entry(mint).or_default() += amount;
        }
    }
    tokens.retain(|_, delta| *delta != 0);
    let mut closed_lamports_out = Some(0u128);
    for closed in &checked.closed_to_others {
        let lamports = closed
            .as_ref()
            .and_then(|closed| addresses.iter().position(|address| address == closed))
            .and_then(|index| before.get(index).and_then(account_facts))
            .map(|(lamports, token)| {
                let wrapped = token
                    .filter(|(mint, _, _)| crate::authority::NATIVE_MINTS.contains(&mint.as_str()))
                    .map_or(0, |(_, _, amount)| amount);
                (lamports as u128).saturating_sub(wrapped)
            });
        closed_lamports_out = match (closed_lamports_out, lamports) {
            (Some(total), Some(lamports)) => Some(total + lamports),
            _ => None,
        };
    }
    Ok(SignerEffect {
        sol_lamports: sol,
        tokens,
        closed_lamports_out,
        err,
        logs_tail,
        unresolved: lookups.unresolved.clone(),
        omitted_accounts,
        findings: checked.findings,
        inner_checked,
        slots,
    })
}

// ---------------------------------------------------------------------------
// Summary
// ---------------------------------------------------------------------------

/// The base64 wire transactions of a prepare body.
pub fn prepared_transactions(prepared: &Value) -> Result<Vec<(u64, Vec<u8>)>> {
    prepared["transactions"]
        .as_array()
        .ok_or_else(|| anyhow!("the prepare returned no transactions"))?
        .iter()
        .enumerate()
        .map(|(position, transaction)| {
            let encoded = transaction["transactionBase64"]
                .as_str()
                .ok_or_else(|| anyhow!("a prepared transaction has no transactionBase64"))?;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .context("a prepared transaction is not base64")?;
            Ok((
                transaction["index"].as_u64().unwrap_or(position as u64),
                bytes,
            ))
        })
        .collect()
}

fn entry_for(list: &Value, index: u64, position: usize) -> Value {
    list.as_array()
        .and_then(|entries| {
            entries
                .iter()
                .find(|entry| entry["index"].as_u64() == Some(index))
                .or_else(|| entries.get(position))
        })
        .cloned()
        .unwrap_or(Value::Null)
}

/// The summary of a prepared step for `signer` (the wallet that signs).
/// `rpc` adds the simulated balance changes.
pub async fn summarize(prepared: &Value, signer: &str, rpc: Option<&Rpc>) -> Result<Value> {
    let transactions = prepared_transactions(prepared)?;
    let mut entries = Vec::with_capacity(transactions.len());
    let mut programs_all: Vec<String> = Vec::new();
    let mut sol_total: Option<i128> = Some(0);
    let mut token_totals: BTreeMap<String, i128> = BTreeMap::new();
    let mut simulation_errors = Vec::new();
    let mut still_required: Vec<String> = Vec::new();
    for (position, (index, bytes)) in transactions.iter().enumerate() {
        let decoded = decode_transaction(bytes)
            .with_context(|| format!("prepared transaction {index} does not decode"))?;
        let anatomy = entry_for(&prepared["anatomy"], *index, position);
        let server_simulation = entry_for(&prepared["simulation"], *index, position);
        let labels: Vec<Value> = anatomy["instructions"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let programs: Vec<Value> = decoded
            .programs()
            .into_iter()
            .map(|program| {
                if !programs_all.contains(&program) {
                    programs_all.push(program.clone());
                }
                let roles: BTreeSet<String> = labels
                    .iter()
                    .filter(|label| label["programId"].as_str() == Some(program.as_str()))
                    .filter_map(|label| {
                        let role = label["role"].as_str()?;
                        Some(match label["kind"].as_str() {
                            Some(kind) if kind != role => format!("{role}:{kind}"),
                            _ => role.to_string(),
                        })
                    })
                    .collect();
                json!({ "programId": program, "roles": roles })
            })
            .collect();
        let signers = decoded.signers();
        let missing: Vec<String> = signers
            .iter()
            .filter(|(_, signed)| !signed)
            .map(|(key, _)| key.clone())
            .collect();
        for key in &missing {
            if !still_required.contains(key) {
                still_required.push(key.clone());
            }
        }
        // Resolve once for both the displayed write set and simulation. Even
        // if simulation fails, the write set must not silently omit ALT keys.
        let lookups = match rpc {
            Some(rpc) => resolve_lookups(rpc, &decoded).await,
            None => ResolvedLookups::default(),
        };
        let mut accounts_written = decoded.writable_static_keys();
        let mut unresolved_writes = Vec::new();
        let mut offset = 0;
        for lookup in &decoded.lookups {
            for &address_index in &lookup.writable {
                match lookups.keys.get(offset).and_then(Option::as_ref) {
                    Some(key) => accounts_written.push(key.clone()),
                    None => unresolved_writes.push(json!({
                        "table": lookup.table,
                        "addressIndex": address_index,
                    })),
                }
                offset += 1;
            }
        }
        let mut entry = json!({
            "index": index,
            "version": match decoded.version { Some(v) => json!(format!("v{v}")), None => json!("legacy") },
            "feePayer": decoded.static_keys.first(),
            "programs": programs,
            "accountsWritten": accounts_written,
            "accountsWrittenComplete": unresolved_writes.is_empty(),
            "unresolvedWritableAccounts": unresolved_writes,
            "signatures": {
                "required": signers.iter().map(|(key, _)| key).collect::<Vec<_>>(),
                "present": signers.iter().filter(|(_, signed)| *signed).map(|(key, _)| key).collect::<Vec<_>>(),
                "missing": missing,
            },
            "serverSimulation": server_simulation,
        });
        let lookup_writable: usize = decoded.lookups.iter().map(|l| l.writable.len()).sum();
        if lookup_writable > 0 {
            entry["lookupTables"] = json!(decoded
                .lookups
                .iter()
                .map(|lookup| json!({
                    "table": lookup.table,
                    "writableEntries": lookup.writable.len(),
                    "readonlyEntries": lookup.readonly.len(),
                }))
                .collect::<Vec<_>>());
        }
        let dependent = server_simulation["simulated"] == Value::Bool(false);
        let mut unsafe_instructions: Vec<crate::authority::Finding> =
            crate::authority::check_top_level(&decoded, signer, &[]).findings;
        match rpc {
            Some(rpc) => {
                match simulate_signer_effect_with_lookups(rpc, bytes, &decoded, signer, &lookups)
                    .await
                {
                    Ok(effect) => {
                        unsafe_instructions = effect.findings.clone();
                        if !effect.err.is_null() {
                            simulation_errors.push(json!({
                                "transaction": index,
                                "err": effect.err,
                                "dependsOnEarlierTransactions": dependent,
                            }));
                        }
                        match (effect.sol_lamports, sol_total) {
                            (Some(delta), Some(total)) => sol_total = Some(total + delta),
                            _ => sol_total = None,
                        }
                        for (mint, delta) in &effect.tokens {
                            *token_totals.entry(mint.clone()).or_default() += delta;
                        }
                        entry["signerEffect"] = json!({
                            "solLamports": effect.sol_lamports.map(|v| v.to_string()),
                            "sol": effect.sol_lamports.map(lamports_to_sol),
                            "tokens": effect.tokens.iter().map(|(mint, delta)| json!({ "mint": mint, "amount": delta.to_string() })).collect::<Vec<_>>(),
                            "closedAccountLamportsOut": effect.closed_lamports_out.map(|v| v.to_string()),
                            "known": effect.known(),
                            "unresolvedLookupTables": effect.unresolved,
                            "omittedAccounts": effect.omitted_accounts,
                            "innerInstructionsChecked": effect.inner_checked,
                            "preStateSlot": effect.slots.0,
                            "simulatedSlot": effect.slots.1,
                        });
                        if effect.omitted_accounts > 0 {
                            sol_total = None;
                        }
                        entry["simulation"] =
                            json!({ "err": effect.err, "logsTail": effect.logs_tail });
                    }
                    Err(error) => {
                        sol_total = None;
                        entry["signerEffect"] =
                            json!({ "known": false, "error": format!("{error:#}") });
                    }
                }
            }
            None => {
                sol_total = None;
                entry["signerEffect"] = json!({ "known": false });
            }
        }
        entry["unsafeInstructions"] = json!(unsafe_instructions
            .iter()
            .map(crate::authority::Finding::to_json)
            .collect::<Vec<_>>());
        entries.push(entry);
    }
    let balance = match rpc {
        None => json!({
            "available": false,
            "reason": "no RPC URL is configured to simulate the transactions, so the signer's balance changes are unknown",
        }),
        Some(_) => json!({
            "available": true,
            "complete": sol_total.is_some() && entries.iter().all(|e| e["signerEffect"]["known"] == true),
            "solLamports": sol_total.map(|v| v.to_string()),
            "sol": sol_total.map(lamports_to_sol),
            "tokens": token_totals.iter().filter(|(_, d)| **d != 0).map(|(mint, delta)| json!({ "mint": mint, "amount": delta.to_string() })).collect::<Vec<_>>(),
            "note": "Simulated one transaction at a time against current chain state; a transaction that depends on an earlier one may not simulate on its own.",
        }),
    };
    Ok(json!({
        "signer": signer,
        "transactions": entries,
        "programsInvoked": programs_all,
        "signerBalanceChanges": balance,
        "simulationErrors": simulation_errors,
        "signaturesStillRequired": still_required,
    }))
}

/// Lamports as a decimal SOL string.
pub fn lamports_to_sol(lamports: i128) -> String {
    let sign = if lamports < 0 { "-" } else { "" };
    let abs = lamports.unsigned_abs();
    let whole = abs / 1_000_000_000;
    let fraction = abs % 1_000_000_000;
    if fraction == 0 {
        return format!("{sign}{whole}");
    }
    let fraction = format!("{fraction:09}");
    format!("{sign}{whole}.{}", fraction.trim_end_matches('0'))
}

/// The detail of a `simulation_failed` rejection, bounded: the error and the
/// last log lines, as the server reports them.
pub fn rejection_summary(rejection: &Value) -> Value {
    let mut out = rejection.clone();
    if let Some(logs) = out
        .pointer_mut("/detail/logs")
        .and_then(Value::as_array_mut)
    {
        let start = logs.len().saturating_sub(LOG_TAIL);
        logs.drain(..start);
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A legacy transaction: `signers` sign (slots zeroed unless in
    /// `signed`), then `writable_unsigned` writable keys, then `programs`
    /// as readonly keys each invoked once.
    pub(crate) fn legacy_transaction(
        signers: &[[u8; 32]],
        signed: &[bool],
        writable_unsigned: &[[u8; 32]],
        programs: &[[u8; 32]],
    ) -> Vec<u8> {
        legacy_transaction_with(signers, signed, writable_unsigned, programs, &[0], &[9])
    }

    /// [`legacy_transaction`] whose instructions take `accounts` (key
    /// indexes) and `data`.
    pub(crate) fn legacy_transaction_with(
        signers: &[[u8; 32]],
        signed: &[bool],
        writable_unsigned: &[[u8; 32]],
        programs: &[[u8; 32]],
        accounts: &[u8],
        data: &[u8],
    ) -> Vec<u8> {
        let mut message = vec![signers.len() as u8, 0, programs.len() as u8];
        let keys = signers.len() + writable_unsigned.len() + programs.len();
        message.push(keys as u8);
        for key in signers.iter().chain(writable_unsigned).chain(programs) {
            message.extend_from_slice(key);
        }
        message.extend_from_slice(&[7u8; 32]);
        message.push(programs.len() as u8);
        for index in 0..programs.len() {
            message.push((signers.len() + writable_unsigned.len() + index) as u8);
            message.push(accounts.len() as u8);
            message.extend_from_slice(accounts);
            message.push(data.len() as u8);
            message.extend_from_slice(data);
        }
        let mut tx = vec![signers.len() as u8];
        for index in 0..signers.len() {
            tx.extend_from_slice(&[u8::from(signed.get(index) == Some(&true)); 64]);
        }
        tx.extend_from_slice(&message);
        tx
    }

    #[test]
    fn decodes_programs_writes_and_signatures() {
        let tx = legacy_transaction(
            &[[1; 32], [2; 32]],
            &[false, true],
            &[[3; 32]],
            &[[4; 32], [5; 32]],
        );
        let decoded = decode_transaction(&tx).unwrap();
        assert_eq!(decoded.version, None);
        assert_eq!(decoded.programs(), vec![base58(&[4; 32]), base58(&[5; 32])]);
        assert_eq!(
            decoded.writable_static_keys(),
            vec![base58(&[1; 32]), base58(&[2; 32]), base58(&[3; 32])]
        );
        assert_eq!(
            decoded.signers(),
            vec![(base58(&[1; 32]), false), (base58(&[2; 32]), true)]
        );
        assert!(decode_transaction(&tx[..tx.len() - 3]).is_err());
    }

    #[test]
    fn decodes_v0_lookups() {
        let mut message = vec![0x80, 1, 0, 1, 2];
        message.extend_from_slice(&[1; 32]);
        message.extend_from_slice(&[4; 32]);
        message.extend_from_slice(&[7; 32]);
        message.extend_from_slice(&[1, 1, 0, 0]); // one ix: program 1, no accounts, no data
        message.extend_from_slice(&[1]);
        message.extend_from_slice(&[8; 32]);
        message.extend_from_slice(&[2, 0, 3, 1, 5]);
        let mut tx = vec![1];
        tx.extend_from_slice(&[0; 64]);
        tx.extend_from_slice(&message);
        let decoded = decode_transaction(&tx).unwrap();
        assert_eq!(decoded.version, Some(0));
        assert_eq!(decoded.lookups.len(), 1);
        assert_eq!(decoded.lookups[0].writable, vec![0, 3]);
        assert_eq!(decoded.lookups[0].readonly, vec![5]);
        assert_eq!(decoded.writable_static_keys(), vec![base58(&[1; 32])]);
    }

    fn lookup_transaction() -> Vec<u8> {
        let mut message = vec![0x80, 1, 0, 1, 2];
        message.extend_from_slice(&[1; 32]);
        message.extend_from_slice(&[4; 32]);
        message.extend_from_slice(&[7; 32]);
        message.extend_from_slice(&[1, 1, 0, 0, 1]);
        message.extend_from_slice(&[8; 32]);
        message.extend_from_slice(&[2, 0, 3, 1, 5]);
        let mut transaction = vec![1];
        transaction.extend_from_slice(&[0; 64]);
        transaction.extend_from_slice(&message);
        transaction
    }

    #[tokio::test]
    async fn summaries_mark_writable_lookups_unresolved_without_rpc() {
        let prepared = json!({ "transactions": [{ "transactionBase64": base64::engine::general_purpose::STANDARD.encode(lookup_transaction()) }] });
        let result = summarize(&prepared, &base58(&[1; 32]), None).await.unwrap();
        let entry = &result["transactions"][0];
        assert_eq!(entry["accountsWritten"], json!([base58(&[1; 32])]));
        assert_eq!(entry["accountsWrittenComplete"], false);
        assert_eq!(
            entry["unresolvedWritableAccounts"],
            json!([
                { "table": base58(&[8; 32]), "addressIndex": 0 },
                { "table": base58(&[8; 32]), "addressIndex": 3 },
            ])
        );
    }

    #[tokio::test]
    async fn summaries_include_lookup_writes_even_when_simulation_fails() {
        use std::io::{BufRead, BufReader, Read, Write};
        for count in [Some(6), Some(1), None] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let server = std::thread::spawn(move || {
                // Lookup resolution, pre-state, simulation.
                for _ in 0..3 {
                    let (mut stream, _) = listener.accept().unwrap();
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut length = 0;
                    loop {
                        let mut line = String::new();
                        reader.read_line(&mut line).unwrap();
                        if line == "\r\n" {
                            break;
                        }
                        if let Some(value) =
                            line.to_ascii_lowercase().strip_prefix("content-length:")
                        {
                            length = value.trim().parse().unwrap();
                        }
                    }
                    let mut bytes = vec![0; length];
                    reader.read_exact(&mut bytes).unwrap();
                    let request: Value = serde_json::from_slice(&bytes).unwrap();
                    let response = match request["method"].as_str().unwrap() {
                        "getMultipleAccounts" => {
                            let values: Vec<Value> = request["params"][0].as_array().unwrap().iter().map(|key| {
                                if key == &json!(base58(&[8; 32])) {
                                    count.map(|count| {
                                        let mut bytes = vec![0; LOOKUP_TABLE_HEADER];
                                        for index in 0..count { bytes.extend_from_slice(&[10 + index as u8; 32]); }
                                        json!({ "data": [base64::engine::general_purpose::STANDARD.encode(bytes), "base64"] })
                                    }).unwrap_or(Value::Null)
                                } else { Value::Null }
                            }).collect();
                            json!({ "result": { "context": { "slot": 1 }, "value": values } })
                        }
                        "simulateTransaction" => json!({ "error": { "message": "simulation unavailable" } }),
                        other => panic!("unexpected request {other}"),
                    }.to_string();
                    write!(stream, "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{response}", response.len()).unwrap();
                }
            });
            let http = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(5))
                .build()
                .unwrap();
            let rpc = Rpc::new(http, &format!("http://{address}"));
            let prepared = json!({ "transactions": [{ "transactionBase64": base64::engine::general_purpose::STANDARD.encode(lookup_transaction()) }] });
            let result = summarize(&prepared, &base58(&[1; 32]), Some(&rpc))
                .await
                .unwrap();
            let entry = &result["transactions"][0];
            let mut expected = vec![base58(&[1; 32])];
            if count.is_some() {
                expected.push(base58(&[10; 32]));
            }
            if count == Some(6) {
                expected.push(base58(&[13; 32]));
            }
            assert_eq!(entry["accountsWritten"], json!(expected));
            assert_eq!(entry["accountsWrittenComplete"], count == Some(6));
            assert_eq!(
                entry["unresolvedWritableAccounts"]
                    .as_array()
                    .unwrap()
                    .len(),
                match count {
                    Some(6) => 0,
                    Some(_) => 1,
                    None => 2,
                }
            );
            assert!(entry["signerEffect"]["error"]
                .as_str()
                .unwrap()
                .contains("simulation unavailable"));
            server.join().unwrap();
        }
    }

    #[test]
    fn decodes_v1_transactions() {
        let mut tx = vec![0x81, 1, 0, 1];
        tx.extend_from_slice(&0b100u32.to_le_bytes()); // compute unit limit
        tx.extend_from_slice(&[7; 32]);
        tx.extend_from_slice(&[1, 2]); // one instruction, two addresses
        tx.extend_from_slice(&[1; 32]);
        tx.extend_from_slice(&[4; 32]);
        tx.extend_from_slice(&200_000u32.to_le_bytes());
        tx.extend_from_slice(&[1, 1, 2, 0]); // program 1, one account, two data bytes
        tx.extend_from_slice(&[0, 9, 9]);
        let message_end = tx.len();
        tx.extend_from_slice(&[0; 64]);
        let decoded = decode_transaction(&tx).unwrap();
        assert_eq!(decoded.version, Some(1));
        assert_eq!(decoded.message, 0..message_end);
        assert_eq!(decoded.signature_offset, message_end);
        assert_eq!(decoded.programs(), vec![base58(&[4; 32])]);
        assert_eq!(decoded.writable_static_keys(), vec![base58(&[1; 32])]);
        assert_eq!(decoded.signers(), vec![(base58(&[1; 32]), false)]);
        let mut trailing = tx.clone();
        trailing.push(0);
        assert!(decode_transaction(&trailing).is_err());
    }

    #[test]
    fn formats_lamports() {
        assert_eq!(lamports_to_sol(1_500_000_000), "1.5");
        assert_eq!(lamports_to_sol(-5000), "-0.000005");
        assert_eq!(lamports_to_sol(2_000_000_000), "2");
    }

    #[tokio::test]
    async fn summaries_without_an_rpc_say_balances_are_unknown() {
        let signer = [1u8; 32];
        let tx = legacy_transaction(&[signer], &[false], &[[3; 32]], &[[4; 32]]);
        let prepared = json!({
            "transactions": [{ "index": 0, "transactionBase64": base64::engine::general_purpose::STANDARD.encode(&tx) }],
            "anatomy": [{ "index": 0, "instructions": [{ "role": "program", "kind": "program", "programId": base58(&[4; 32]) }] }],
            "simulation": [{ "index": 0, "unitsConsumed": "1200" }],
        });
        let summary = summarize(&prepared, &base58(&signer), None).await.unwrap();
        assert_eq!(summary["signerBalanceChanges"]["available"], false);
        assert_eq!(summary["programsInvoked"], json!([base58(&[4; 32])]));
        assert_eq!(
            summary["transactions"][0]["programs"][0]["roles"],
            json!(["program"])
        );
        assert_eq!(summary["signaturesStillRequired"], json!([base58(&signer)]));
        assert_eq!(
            summary["transactions"][0]["serverSimulation"]["unitsConsumed"],
            "1200"
        );
    }
}
