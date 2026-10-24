//! Legacy and v0 transactions for the Squads helpers, and the JSON-RPC calls that
//! read accounts and send and confirm them.

use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use arete_mcp::dev::DevClient;
use base64::Engine as _;
use serde_json::{json, Value};

use super::{b58, parse_key, Key};

/// The largest wire transaction Solana accepts.
pub const MAX_TRANSACTION_BYTES: usize = 1232;
const CONFIRM_TIMEOUT: Duration = Duration::from_secs(60);

/// An instruction account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Meta {
    pub key: Key,
    pub signer: bool,
    pub writable: bool,
}

impl Meta {
    pub fn readonly(key: Key) -> Self {
        Self {
            key,
            signer: false,
            writable: false,
        }
    }

    pub fn writable(key: Key) -> Self {
        Self {
            key,
            signer: false,
            writable: true,
        }
    }

    pub fn readonly_signer(key: Key) -> Self {
        Self {
            key,
            signer: true,
            writable: false,
        }
    }

    pub fn writable_signer(key: Key) -> Self {
        Self {
            key,
            signer: true,
            writable: true,
        }
    }
}

/// One instruction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instruction {
    pub program: Key,
    pub accounts: Vec<Meta>,
    pub data: Vec<u8>,
}

fn compact_u16(out: &mut Vec<u8>, value: usize) {
    let mut value = value as u16;
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if value == 0 {
            break;
        }
    }
}

/// An address lookup table a v0 transaction may load accounts from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LookupTable {
    pub key: Key,
    pub addresses: Vec<Key>,
}

/// An unsigned legacy wire transaction (zeroed signature slots) paid by
/// `payer`. Accounts are ordered writable signers (payer first), read-only
/// signers, writable non-signers, read-only non-signers, each in first-use
/// order.
pub fn compile_legacy(
    payer: &Key,
    instructions: &[Instruction],
    blockhash: &Key,
) -> Result<Vec<u8>> {
    compile(payer, instructions, blockhash, None)
}

/// An unsigned v0 wire transaction paid by `payer` that loads every
/// non-signer account it can from `tables` (invoked programs stay static).
/// The static accounts are ordered as in [`compile_legacy`].
pub fn compile_v0(
    payer: &Key,
    instructions: &[Instruction],
    blockhash: &Key,
    tables: &[LookupTable],
) -> Result<Vec<u8>> {
    compile(payer, instructions, blockhash, Some(tables))
}

/// The (address index, key) pairs a transaction loads from one table.
type Loads = Vec<(u8, Key)>;

fn compile(
    payer: &Key,
    instructions: &[Instruction],
    blockhash: &Key,
    tables: Option<&[LookupTable]>,
) -> Result<Vec<u8>> {
    let mut keys: Vec<Meta> = vec![Meta::writable_signer(*payer)];
    let mut add = |meta: Meta| match keys.iter_mut().find(|known| known.key == meta.key) {
        Some(known) => {
            known.signer |= meta.signer;
            known.writable |= meta.writable;
        }
        None => keys.push(meta),
    };
    for instruction in instructions {
        add(Meta::readonly(instruction.program));
        for account in &instruction.accounts {
            add(*account);
        }
    }
    let rank = |meta: &Meta| match (meta.signer, meta.writable) {
        (true, true) => 0,
        (true, false) => 1,
        (false, true) => 2,
        (false, false) => 3,
    };
    // A stable sort keeps the payer first and first-use order within groups.
    keys.sort_by_key(rank);
    let versioned = tables.is_some();
    let tables = tables.unwrap_or_default();
    // Per table: the (address index, key) pairs loaded writable and read-only.
    let mut loaded: Vec<(Loads, Loads)> = vec![Default::default(); tables.len()];
    keys.retain(|meta| {
        if meta.signer
            || instructions
                .iter()
                .any(|instruction| instruction.program == meta.key)
        {
            return true;
        }
        for (table, found) in tables.iter().zip(loaded.iter_mut()) {
            if let Some(index) = table.addresses.iter().position(|key| *key == meta.key) {
                let Ok(index) = u8::try_from(index) else {
                    continue;
                };
                let side = if meta.writable {
                    &mut found.0
                } else {
                    &mut found.1
                };
                side.push((index, meta.key));
                return false;
            }
        }
        true
    });
    let signers = keys.iter().filter(|meta| meta.signer).count();
    let readonly_signed = keys
        .iter()
        .filter(|meta| meta.signer && !meta.writable)
        .count();
    let readonly_unsigned = keys
        .iter()
        .filter(|meta| !meta.signer && !meta.writable)
        .count();
    // Account indexes: static keys, then every table's writable loads, then
    // every table's read-only loads.
    let order: Vec<Key> = keys
        .iter()
        .map(|meta| meta.key)
        .chain(
            loaded
                .iter()
                .flat_map(|(writable, _)| writable.iter().map(|(_, key)| *key)),
        )
        .chain(
            loaded
                .iter()
                .flat_map(|(_, readonly)| readonly.iter().map(|(_, key)| *key)),
        )
        .collect();
    if order.len() > u8::MAX as usize {
        bail!("the transaction names more than 255 accounts");
    }
    let index_of = |key: &Key| -> u8 {
        order
            .iter()
            .position(|known| known == key)
            .expect("every account was added") as u8
    };
    let mut message = Vec::new();
    if versioned {
        message.push(0x80);
    }
    message.extend_from_slice(&[
        signers as u8,
        readonly_signed as u8,
        readonly_unsigned as u8,
    ]);
    compact_u16(&mut message, keys.len());
    for meta in &keys {
        message.extend_from_slice(&meta.key);
    }
    message.extend_from_slice(blockhash);
    compact_u16(&mut message, instructions.len());
    for instruction in instructions {
        message.push(index_of(&instruction.program));
        compact_u16(&mut message, instruction.accounts.len());
        for account in &instruction.accounts {
            message.push(index_of(&account.key));
        }
        compact_u16(&mut message, instruction.data.len());
        message.extend_from_slice(&instruction.data);
    }
    if versioned {
        let used: Vec<_> = tables
            .iter()
            .zip(&loaded)
            .filter(|(_, (writable, readonly))| !writable.is_empty() || !readonly.is_empty())
            .collect();
        compact_u16(&mut message, used.len());
        for (table, (writable, readonly)) in used {
            message.extend_from_slice(&table.key);
            compact_u16(&mut message, writable.len());
            message.extend(writable.iter().map(|(index, _)| *index));
            compact_u16(&mut message, readonly.len());
            message.extend(readonly.iter().map(|(index, _)| *index));
        }
    }
    let mut transaction = Vec::with_capacity(1 + 64 * signers + message.len());
    compact_u16(&mut transaction, signers);
    transaction.resize(transaction.len() + 64 * signers, 0);
    transaction.extend_from_slice(&message);
    if transaction.len() > MAX_TRANSACTION_BYTES {
        bail!(
            "the transaction is {} bytes, over Solana's {MAX_TRANSACTION_BYTES}-byte limit",
            transaction.len()
        );
    }
    Ok(transaction)
}

/// The first signature of a signed wire transaction, base58: its id.
pub fn transaction_signature(transaction: &[u8]) -> Result<String> {
    let decoded = arete_mcp::prepared_summary::decode_transaction(transaction)?;
    let first = decoded
        .signatures
        .first()
        .ok_or_else(|| anyhow!("the transaction has no signatures"))?;
    Ok(bs58::encode(first).into_string())
}

/// Where JSON-RPC calls go: the project's dev environment, or an RPC URL.
pub enum Chain<'a> {
    Dev(&'a DevClient),
    Url { http: reqwest::Client, url: String },
}

impl Chain<'_> {
    pub fn url(url: &str) -> Chain<'static> {
        Chain::Url {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
            url: url.to_string(),
        }
    }

    pub async fn call(&self, method: &str, params: Value) -> Result<Value> {
        match self {
            Chain::Dev(client) => client.rpc(method, params).await,
            Chain::Url { http, url } => {
                let response: Value = http
                    .post(url)
                    .json(&json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }))
                    .send()
                    .await
                    .with_context(|| format!("The RPC {url} is unreachable"))?
                    .json()
                    .await
                    .with_context(|| format!("{method} returned no JSON"))?;
                if let Some(error) = response.get("error").filter(|error| !error.is_null()) {
                    let logs = error
                        .pointer("/data/logs")
                        .and_then(Value::as_array)
                        .map(|logs| {
                            logs.iter()
                                .filter_map(Value::as_str)
                                .collect::<Vec<_>>()
                                .join("\n  ")
                        })
                        .filter(|logs| !logs.is_empty())
                        .map(|logs| format!("\n  {logs}"))
                        .unwrap_or_default();
                    bail!(
                        "{method} failed: {}{logs}",
                        error["message"].as_str().unwrap_or("unknown error")
                    );
                }
                Ok(response["result"].clone())
            }
        }
    }

    /// An account's data and owner, or `None` when it does not exist.
    pub async fn account(&self, key: &Key) -> Result<Option<(Vec<u8>, String)>> {
        let result = self
            .call(
                "getAccountInfo",
                json!([b58(key), { "encoding": "base64", "commitment": "confirmed" }]),
            )
            .await?;
        let value = &result["value"];
        if value.is_null() {
            return Ok(None);
        }
        let data = value["data"][0]
            .as_str()
            .ok_or_else(|| anyhow!("getAccountInfo returned no base64 data"))?;
        let data = base64::engine::general_purpose::STANDARD
            .decode(data)
            .context("getAccountInfo returned invalid base64")?;
        Ok(Some((
            data,
            value["owner"].as_str().unwrap_or_default().to_string(),
        )))
    }

    /// An account owned by the Squads program, or an error naming `what`.
    pub async fn squads_account(&self, key: &Key, what: &str) -> Result<Vec<u8>> {
        match self.account(key).await? {
            Some((data, owner)) if owner == super::PROGRAM_ID => Ok(data),
            Some((_, owner)) => bail!(
                "{} is not a Squads {what}: it is owned by {owner}",
                b58(key)
            ),
            None => bail!("No Squads {what} exists at {}", b58(key)),
        }
    }

    pub async fn latest_blockhash(&self) -> Result<Key> {
        let result = self
            .call("getLatestBlockhash", json!([{ "commitment": "confirmed" }]))
            .await?;
        result["value"]["blockhash"]
            .as_str()
            .ok_or_else(|| anyhow!("getLatestBlockhash returned no blockhash"))
            .and_then(parse_key)
    }

    /// Send a signed wire transaction and wait until it is confirmed; its
    /// signature. A transaction that fails on chain is an error.
    pub async fn send_and_confirm(&self, transaction: &[u8]) -> Result<String> {
        let signature = transaction_signature(transaction)?;
        let encoded = base64::engine::general_purpose::STANDARD.encode(transaction);
        self.call(
            "sendTransaction",
            json!([encoded, { "encoding": "base64", "preflightCommitment": "confirmed" }]),
        )
        .await?;
        let deadline = Instant::now() + CONFIRM_TIMEOUT;
        loop {
            let statuses = self
                .call(
                    "getSignatureStatuses",
                    json!([[signature], { "searchTransactionHistory": true }]),
                )
                .await?;
            let status = &statuses["value"][0];
            if !status.is_null() {
                if let Some(error) = status.get("err").filter(|error| !error.is_null()) {
                    bail!("Transaction {signature} failed: {error}");
                }
                if matches!(
                    status["confirmationStatus"].as_str(),
                    Some("confirmed" | "finalized")
                ) {
                    return Ok(signature);
                }
            }
            if Instant::now() >= deadline {
                bail!(
                    "Transaction {signature} was not confirmed within {}s; check it before sending again",
                    CONFIRM_TIMEOUT.as_secs()
                );
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
}
