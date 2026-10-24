//! Squads v4 multisig instructions and accounts, encoded from the program's
//! Anchor IDL (`squads_multisig_program` 2.1.0, program
//! `SQDS4ep65T869zMMBKyuUq6aD6EgTu8psMjkvj52pCf`).
//!
//! Instruction data is the 8-byte Anchor discriminator
//! (`sha256("global:<snake_case name>")[..8]`) followed by Borsh args;
//! accounts start with `sha256("account:<Name>")[..8]`. The PDAs are the
//! program's seeds (`multisig` prefix). Only what `a4 dev squads` needs is
//! here: create a multisig, activate, approve and execute config and vault
//! transactions, and read the accounts those need.

pub mod tx;

use anyhow::{anyhow, bail, Result};
use sha2::{Digest, Sha256};

pub use tx::{Instruction, Meta};

/// The Squads v4 program.
pub const PROGRAM_ID: &str = "SQDS4ep65T869zMMBKyuUq6aD6EgTu8psMjkvj52pCf";
pub const COMPUTE_BUDGET_PROGRAM: &str = "ComputeBudget111111111111111111111111111111";

/// Member permission bits (`Permissions.mask`).
pub const PERMISSION_INITIATE: u8 = 1;
pub const PERMISSION_VOTE: u8 = 2;
pub const PERMISSION_EXECUTE: u8 = 4;
pub const PERMISSION_ALL: u8 = PERMISSION_INITIATE | PERMISSION_VOTE | PERMISSION_EXECUTE;

const SEED_PREFIX: &[u8] = b"multisig";
const SEED_MULTISIG: &[u8] = b"multisig";
const SEED_PROGRAM_CONFIG: &[u8] = b"program_config";
const SEED_VAULT: &[u8] = b"vault";
const SEED_TRANSACTION: &[u8] = b"transaction";
const SEED_PROPOSAL: &[u8] = b"proposal";
const SEED_EPHEMERAL_SIGNER: &[u8] = b"ephemeral_signer";

/// A public key.
pub type Key = [u8; 32];

/// Decode a base58 public key.
pub fn parse_key(value: &str) -> Result<Key> {
    bs58::decode(value.trim())
        .into_vec()
        .ok()
        .and_then(|bytes| <Key>::try_from(bytes).ok())
        .ok_or_else(|| anyhow!("'{value}' is not a base58 public key"))
}

pub fn b58(key: &Key) -> String {
    bs58::encode(key).into_string()
}

pub fn program_id() -> Key {
    parse_key(PROGRAM_ID).expect("valid program id")
}

pub fn system_program() -> Key {
    [0; 32]
}

fn discriminator(preimage: &str) -> [u8; 8] {
    let digest = Sha256::digest(preimage.as_bytes());
    digest[..8].try_into().expect("8 bytes")
}

/// An instruction's Anchor discriminator (`name` in snake_case).
pub fn instruction_discriminator(name: &str) -> [u8; 8] {
    discriminator(&format!("global:{name}"))
}

/// An account type's Anchor discriminator (`name` as in the IDL).
pub fn account_discriminator(name: &str) -> [u8; 8] {
    discriminator(&format!("account:{name}"))
}

fn pda(seeds: &[&[u8]]) -> Key {
    let seeds: Vec<Vec<u8>> = seeds.iter().map(|seed| seed.to_vec()).collect();
    arete_sdk::instruction::derive_program_address(&seeds, PROGRAM_ID)
        .expect("Squads seeds are within the runtime limits")
        .0
        .to_bytes()
}

/// The program config: `["multisig", "program_config"]`.
pub fn program_config_pda() -> Key {
    pda(&[SEED_PREFIX, SEED_PROGRAM_CONFIG])
}

/// A multisig: `["multisig", "multisig", create_key]`.
pub fn multisig_pda(create_key: &Key) -> Key {
    pda(&[SEED_PREFIX, SEED_MULTISIG, create_key])
}

/// A multisig's vault: `["multisig", multisig, "vault", [index]]`.
pub fn vault_pda(multisig: &Key, index: u8) -> Key {
    pda(&[SEED_PREFIX, multisig, SEED_VAULT, &[index]])
}

/// A config or vault transaction:
/// `["multisig", multisig, "transaction", index (u64 LE)]`.
pub fn transaction_pda(multisig: &Key, index: u64) -> Key {
    pda(&[
        SEED_PREFIX,
        multisig,
        SEED_TRANSACTION,
        &index.to_le_bytes(),
    ])
}

/// A transaction's proposal:
/// `["multisig", multisig, "transaction", index (u64 LE), "proposal"]`.
pub fn proposal_pda(multisig: &Key, index: u64) -> Key {
    pda(&[
        SEED_PREFIX,
        multisig,
        SEED_TRANSACTION,
        &index.to_le_bytes(),
        SEED_PROPOSAL,
    ])
}

/// A vault transaction's ephemeral signer:
/// `["multisig", transaction, "ephemeral_signer", [index]]`.
pub fn ephemeral_signer_pda(transaction: &Key, index: u8) -> Key {
    pda(&[SEED_PREFIX, transaction, SEED_EPHEMERAL_SIGNER, &[index]])
}

fn squads_instruction(accounts: Vec<Meta>, data: Vec<u8>) -> Instruction {
    Instruction {
        program: program_id(),
        accounts,
        data,
    }
}

/// `multisig_create_v2` for an autonomous multisig (no config authority,
/// no rent collector, no memo). `members` are `(key, permission mask)`.
pub struct CreateMultisig<'a> {
    pub treasury: Key,
    pub create_key: Key,
    pub creator: Key,
    pub threshold: u16,
    pub members: &'a [(Key, u8)],
    pub time_lock: u32,
}

pub fn multisig_create_v2(args: &CreateMultisig<'_>) -> Instruction {
    let mut data = instruction_discriminator("multisig_create_v2").to_vec();
    data.push(0); // config_authority: None
    data.extend_from_slice(&args.threshold.to_le_bytes());
    data.extend_from_slice(&(args.members.len() as u32).to_le_bytes());
    for (key, mask) in args.members {
        data.extend_from_slice(key);
        data.push(*mask);
    }
    data.extend_from_slice(&args.time_lock.to_le_bytes());
    data.push(0); // rent_collector: None
    data.push(0); // memo: None
    squads_instruction(
        vec![
            Meta::readonly(program_config_pda()),
            Meta::writable(args.treasury),
            Meta::writable(multisig_pda(&args.create_key)),
            Meta::readonly_signer(args.create_key),
            Meta::writable_signer(args.creator),
            Meta::readonly(system_program()),
        ],
        data,
    )
}

/// `proposal_activate`: a draft proposal becomes active (open to votes).
pub fn proposal_activate(multisig: &Key, member: &Key, index: u64) -> Instruction {
    squads_instruction(
        vec![
            Meta::readonly(*multisig),
            Meta::writable_signer(*member),
            Meta::writable(proposal_pda(multisig, index)),
        ],
        instruction_discriminator("proposal_activate").to_vec(),
    )
}

/// `proposal_approve` with no memo.
pub fn proposal_approve(multisig: &Key, member: &Key, index: u64) -> Instruction {
    let mut data = instruction_discriminator("proposal_approve").to_vec();
    data.push(0); // memo: None
    squads_instruction(
        vec![
            Meta::readonly(*multisig),
            Meta::writable_signer(*member),
            Meta::writable(proposal_pda(multisig, index)),
        ],
        data,
    )
}

/// `config_transaction_execute`. `member` (Execute permission) also pays
/// for any reallocation (adding a member grows the multisig account), so
/// the optional `rent_payer` and `system_program` are always passed.
pub fn config_transaction_execute(multisig: &Key, member: &Key, index: u64) -> Instruction {
    squads_instruction(
        vec![
            Meta::writable(*multisig),
            Meta::readonly_signer(*member),
            Meta::writable(proposal_pda(multisig, index)),
            Meta::readonly(transaction_pda(multisig, index)),
            Meta::writable_signer(*member),
            Meta::readonly(system_program()),
        ],
        instruction_discriminator("config_transaction_execute").to_vec(),
    )
}

/// `vault_transaction_execute` with the remaining accounts the stored
/// message needs ([`vault_execute_accounts`]).
pub fn vault_transaction_execute(
    multisig: &Key,
    member: &Key,
    index: u64,
    remaining: Vec<Meta>,
) -> Instruction {
    let mut accounts = vec![
        Meta::readonly(*multisig),
        Meta::writable(proposal_pda(multisig, index)),
        Meta::readonly(transaction_pda(multisig, index)),
        Meta::readonly_signer(*member),
    ];
    accounts.extend(remaining);
    squads_instruction(
        accounts,
        instruction_discriminator("vault_transaction_execute").to_vec(),
    )
}

/// `SetComputeUnitLimit`.
pub fn compute_unit_limit(units: u32) -> Instruction {
    let mut data = vec![2];
    data.extend_from_slice(&units.to_le_bytes());
    Instruction {
        program: parse_key(COMPUTE_BUDGET_PROGRAM).expect("valid program id"),
        accounts: Vec::new(),
        data,
    }
}

// ---------------------------------------------------------------------------
// Accounts
// ---------------------------------------------------------------------------

/// A Borsh reader over account data.
struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    /// After checking the account's discriminator.
    fn account(bytes: &'a [u8], name: &str) -> Result<Self> {
        if bytes.len() < 8 || bytes[..8] != account_discriminator(name) {
            bail!("the account is not a Squads {name}");
        }
        Ok(Self { bytes, at: 8 })
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8]> {
        let end = self
            .at
            .checked_add(len)
            .filter(|end| *end <= self.bytes.len())
            .ok_or_else(|| anyhow!("truncated Squads account"))?;
        let slice = &self.bytes[self.at..end];
        self.at = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into()?))
    }

    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into()?))
    }

    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into()?))
    }

    fn i64(&mut self) -> Result<i64> {
        Ok(i64::from_le_bytes(self.take(8)?.try_into()?))
    }

    fn key(&mut self) -> Result<Key> {
        Ok(self.take(32)?.try_into()?)
    }

    fn option_key(&mut self) -> Result<Option<Key>> {
        match self.u8()? {
            0 => Ok(None),
            1 => Ok(Some(self.key()?)),
            other => bail!("invalid option tag {other}"),
        }
    }

    fn len(&mut self) -> Result<usize> {
        let len = self.u32()? as usize;
        if len > self.bytes.len() {
            bail!("invalid vector length {len}");
        }
        Ok(len)
    }

    fn bytes(&mut self) -> Result<Vec<u8>> {
        let len = self.len()?;
        Ok(self.take(len)?.to_vec())
    }

    fn keys(&mut self) -> Result<Vec<Key>> {
        let len = self.len()?;
        (0..len).map(|_| self.key()).collect()
    }
}

/// `ProgramConfig`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgramConfig {
    pub authority: Key,
    pub multisig_creation_fee: u64,
    pub treasury: Key,
}

impl ProgramConfig {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let mut reader = Reader::account(bytes, "ProgramConfig")?;
        Ok(Self {
            authority: reader.key()?,
            multisig_creation_fee: reader.u64()?,
            treasury: reader.key()?,
        })
    }
}

/// `Multisig`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Multisig {
    pub create_key: Key,
    pub config_authority: Key,
    pub threshold: u16,
    pub time_lock: u32,
    pub transaction_index: u64,
    pub stale_transaction_index: u64,
    pub rent_collector: Option<Key>,
    pub bump: u8,
    /// `(key, permission mask)`.
    pub members: Vec<(Key, u8)>,
}

impl Multisig {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let mut reader = Reader::account(bytes, "Multisig")?;
        let create_key = reader.key()?;
        let config_authority = reader.key()?;
        let threshold = reader.u16()?;
        let time_lock = reader.u32()?;
        let transaction_index = reader.u64()?;
        let stale_transaction_index = reader.u64()?;
        let rent_collector = reader.option_key()?;
        let bump = reader.u8()?;
        let count = reader.len()?;
        let members = (0..count)
            .map(|_| Ok((reader.key()?, reader.u8()?)))
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            create_key,
            config_authority,
            threshold,
            time_lock,
            transaction_index,
            stale_transaction_index,
            rent_collector,
            bump,
            members,
        })
    }

    /// The permission mask of `key`, when it is a member.
    pub fn mask_of(&self, key: &Key) -> Option<u8> {
        self.members
            .iter()
            .find(|(member, _)| member == key)
            .map(|(_, mask)| *mask)
    }

    /// Whether config changes need the config authority's signature
    /// rather than the members' votes.
    pub fn is_controlled(&self) -> bool {
        self.config_authority != [0; 32]
    }
}

/// A proposal's status (`ProposalStatus`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProposalStatus {
    Draft,
    Active,
    Rejected,
    Approved,
    Executing,
    Executed,
    Cancelled,
}

impl ProposalStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Active => "active",
            Self::Rejected => "rejected",
            Self::Approved => "approved",
            Self::Executing => "executing",
            Self::Executed => "executed",
            Self::Cancelled => "cancelled",
        }
    }
}

/// `Proposal`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proposal {
    pub multisig: Key,
    pub transaction_index: u64,
    pub status: ProposalStatus,
    pub approved: Vec<Key>,
    pub rejected: Vec<Key>,
    pub cancelled: Vec<Key>,
}

impl Proposal {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let mut reader = Reader::account(bytes, "Proposal")?;
        let multisig = reader.key()?;
        let transaction_index = reader.u64()?;
        let status = match reader.u8()? {
            0 => ProposalStatus::Draft,
            1 => ProposalStatus::Active,
            2 => ProposalStatus::Rejected,
            3 => ProposalStatus::Approved,
            4 => ProposalStatus::Executing,
            5 => ProposalStatus::Executed,
            6 => ProposalStatus::Cancelled,
            other => bail!("unknown proposal status {other}"),
        };
        if status != ProposalStatus::Executing {
            reader.i64()?; // timestamp
        }
        reader.u8()?; // bump
        Ok(Self {
            multisig,
            transaction_index,
            status,
            approved: reader.keys()?,
            rejected: reader.keys()?,
            cancelled: reader.keys()?,
        })
    }
}

/// One action of a config transaction (`ConfigAction`), as far as the dev
/// helpers execute them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigAction {
    AddMember { key: Key, mask: u8 },
    RemoveMember { key: Key },
    ChangeThreshold { threshold: u16 },
    SetTimeLock { seconds: u32 },
    SetRentCollector { collector: Option<Key> },
}

impl ConfigAction {
    pub fn describe(&self) -> String {
        match self {
            Self::AddMember { key, mask } => format!("add member {} (mask {mask})", b58(key)),
            Self::RemoveMember { key } => format!("remove member {}", b58(key)),
            Self::ChangeThreshold { threshold } => format!("change threshold to {threshold}"),
            Self::SetTimeLock { seconds } => format!("set time lock to {seconds}s"),
            Self::SetRentCollector { collector } => format!(
                "set rent collector to {}",
                collector.as_ref().map(b58).unwrap_or_else(|| "none".into())
            ),
        }
    }
}

/// `ConfigTransaction`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigTransaction {
    pub multisig: Key,
    pub creator: Key,
    pub index: u64,
    pub actions: Vec<ConfigAction>,
}

impl ConfigTransaction {
    /// Spending-limit actions need extra accounts the dev helpers do not
    /// derive, so they are refused.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let mut reader = Reader::account(bytes, "ConfigTransaction")?;
        let multisig = reader.key()?;
        let creator = reader.key()?;
        let index = reader.u64()?;
        reader.u8()?; // bump
        let count = reader.len()?;
        let mut actions = Vec::with_capacity(count);
        for _ in 0..count {
            actions.push(match reader.u8()? {
                0 => ConfigAction::AddMember {
                    key: reader.key()?,
                    mask: reader.u8()?,
                },
                1 => ConfigAction::RemoveMember { key: reader.key()? },
                2 => ConfigAction::ChangeThreshold {
                    threshold: reader.u16()?,
                },
                3 => ConfigAction::SetTimeLock {
                    seconds: reader.u32()?,
                },
                4 | 5 => bail!(
                    "config transaction {index} changes a spending limit, which `a4 dev squads execute` does not support"
                ),
                6 => ConfigAction::SetRentCollector {
                    collector: reader.option_key()?,
                },
                other => bail!("unknown config action {other}"),
            });
        }
        Ok(Self {
            multisig,
            creator,
            index,
            actions,
        })
    }
}

/// A compiled instruction of a vault transaction's message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledInstruction {
    pub program_id_index: u8,
    pub account_indexes: Vec<u8>,
    pub data: Vec<u8>,
}

/// An address table lookup of a vault transaction's message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableLookup {
    pub account_key: Key,
    pub writable_indexes: Vec<u8>,
    pub readonly_indexes: Vec<u8>,
}

/// `VaultTransactionMessage`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultMessage {
    pub num_signers: u8,
    pub num_writable_signers: u8,
    pub num_writable_non_signers: u8,
    pub account_keys: Vec<Key>,
    pub instructions: Vec<CompiledInstruction>,
    pub address_table_lookups: Vec<TableLookup>,
}

impl VaultMessage {
    fn is_signer(&self, index: usize) -> bool {
        index < self.num_signers as usize
    }

    fn is_static_writable(&self, index: usize) -> bool {
        if index >= self.account_keys.len() {
            return false;
        }
        if self.is_signer(index) {
            return index < self.num_writable_signers as usize;
        }
        index - (self.num_signers as usize) < self.num_writable_non_signers as usize
    }
}

/// `VaultTransaction`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultTransaction {
    pub multisig: Key,
    pub creator: Key,
    pub index: u64,
    pub vault_index: u8,
    pub ephemeral_signer_bumps: Vec<u8>,
    pub message: VaultMessage,
}

impl VaultTransaction {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let mut reader = Reader::account(bytes, "VaultTransaction")?;
        let multisig = reader.key()?;
        let creator = reader.key()?;
        let index = reader.u64()?;
        reader.u8()?; // bump
        let vault_index = reader.u8()?;
        reader.u8()?; // vault_bump
        let ephemeral_signer_bumps = reader.bytes()?;
        let num_signers = reader.u8()?;
        let num_writable_signers = reader.u8()?;
        let num_writable_non_signers = reader.u8()?;
        let account_keys = reader.keys()?;
        let count = reader.len()?;
        let instructions = (0..count)
            .map(|_| {
                Ok(CompiledInstruction {
                    program_id_index: reader.u8()?,
                    account_indexes: reader.bytes()?,
                    data: reader.bytes()?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let count = reader.len()?;
        let address_table_lookups = (0..count)
            .map(|_| {
                Ok(TableLookup {
                    account_key: reader.key()?,
                    writable_indexes: reader.bytes()?,
                    readonly_indexes: reader.bytes()?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            multisig,
            creator,
            index,
            vault_index,
            ephemeral_signer_bumps,
            message: VaultMessage {
                num_signers,
                num_writable_signers,
                num_writable_non_signers,
                account_keys,
                instructions,
                address_table_lookups,
            },
        })
    }
}

/// The addresses an address lookup table account holds (after its 56-byte
/// header).
pub fn lookup_table_addresses(data: &[u8]) -> Result<Vec<Key>> {
    const HEADER: usize = 56;
    if data.len() < HEADER || !(data.len() - HEADER).is_multiple_of(32) {
        bail!("not an address lookup table account");
    }
    Ok(data[HEADER..].as_chunks::<32>().0.to_vec())
}

/// The remaining accounts of `vault_transaction_execute`, in the order the
/// program reads them: the lookup tables, the message's static keys (the
/// vault and ephemeral signers are PDAs and never signers here), then the
/// accounts loaded from each table, writable before read-only.
/// `tables` holds each lookup's table addresses, in lookup order.
pub fn vault_execute_accounts(
    transaction_key: &Key,
    transaction: &VaultTransaction,
    tables: &[Vec<Key>],
) -> Result<Vec<Meta>> {
    let message = &transaction.message;
    if tables.len() != message.address_table_lookups.len() {
        bail!("every address table lookup needs its table's addresses");
    }
    let vault = vault_pda(&transaction.multisig, transaction.vault_index);
    let ephemeral: Vec<Key> = (0..transaction.ephemeral_signer_bumps.len())
        .map(|index| ephemeral_signer_pda(transaction_key, index as u8))
        .collect();
    let mut accounts: Vec<Meta> = message
        .address_table_lookups
        .iter()
        .map(|lookup| Meta::readonly(lookup.account_key))
        .collect();
    for (index, key) in message.account_keys.iter().enumerate() {
        accounts.push(Meta {
            key: *key,
            signer: message.is_signer(index) && *key != vault && !ephemeral.contains(key),
            writable: message.is_static_writable(index),
        });
    }
    for (lookup, addresses) in message.address_table_lookups.iter().zip(tables) {
        let resolve = |index: &u8| {
            addresses.get(*index as usize).copied().ok_or_else(|| {
                anyhow!(
                    "lookup table {} has no address at index {index}",
                    b58(&lookup.account_key)
                )
            })
        };
        for index in &lookup.writable_indexes {
            accounts.push(Meta::writable(resolve(index)?));
        }
        for index in &lookup.readonly_indexes {
            accounts.push(Meta::readonly(resolve(index)?));
        }
    }
    Ok(accounts)
}

#[cfg(test)]
mod tests;
