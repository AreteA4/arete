//! Execution-side contracts: provider snapshots and execution intents.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::error::{ProtocolContractError, ProtocolResult};
use super::model::SignerClass;
use super::release::sorted_entries;
use super::shape::*;
use crate::{
    hash_jcs, ExecutionIntent, HashError, HashId, ProtocolRelease, ProtocolResource,
    ProviderSnapshot,
};

pub const PROVIDER_SNAPSHOT_SCHEMA_V1: &str = "arete.provider-snapshot/v1";
pub const EXECUTION_INTENT_SCHEMA_V1: &str = "arete.execution-intent/v1";

pub const MAX_CTX_CALLS: usize = 64;
pub const MAX_INTENT_TRANSACTIONS: usize = 8;
const MAX_SNAPSHOT_READS: usize = 64;
const MAX_INTENT_SIGNERS: usize = 32;
const U64_MAX: u128 = u64::MAX as u128;
const MAX_SAFE: i64 = 9_007_199_254_740_991;

fn model<T: serde::de::DeserializeOwned>(value: &Value) -> ProtocolResult<T> {
    serde_json::from_value(value.clone()).map_err(|error| {
        ProtocolContractError::new("invalid-value", "", format!("model mismatch: {error}"))
    })
}

// ---------------------------------------------------------------------------
// Provider snapshot
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderSnapshotV1 {
    pub schema: String,
    /// Every read and `ctx` call used this slot or a later one.
    pub context_slot: String,
    /// Declared-read results, sorted by name.
    pub reads: Vec<SnapshotReadV1>,
    /// The ordered `ctx` call log; `seq` counts from 0.
    pub ctx_calls: Vec<CtxCallRecordV1>,
    /// Unix seconds.
    pub observed_at: i64,
    pub expires_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SnapshotReadV1 {
    pub name: String,
    pub result: AccountReadResultV1,
}

/// One account as read from chain. A missing account has `exists: false` and
/// no other data fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AccountReadResultV1 {
    pub address: String,
    pub exists: bool,
    pub slot: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lamports: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_base64: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CtxCallRecordV1 {
    pub seq: u32,
    pub method: CtxMethod,
    /// Canonical call arguments.
    pub args: Value,
    /// Lowercase hex SHA-256 of the canonical JSON result.
    pub result_sha256: String,
}

/// The closed `ctx` method set of `arete.protocol-ctx/v1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CtxMethod {
    #[serde(rename = "solana.account")]
    SolanaAccount,
    #[serde(rename = "solana.accounts")]
    SolanaAccounts,
    #[serde(rename = "solana.tokenAccountsByOwner")]
    SolanaTokenAccountsByOwner,
    #[serde(rename = "stacks.view")]
    StacksView,
}

impl CtxMethod {
    pub const ALL: [&'static str; 4] = [
        "solana.account",
        "solana.accounts",
        "solana.tokenAccountsByOwner",
        "stacks.view",
    ];
}

impl ProviderSnapshotV1 {
    pub fn hash(&self) -> Result<HashId<ProviderSnapshot>, HashError> {
        hash_jcs::<ProviderSnapshot, _>(self)
    }
}

pub(crate) fn shape_account_read(value: &Value, path: &str) -> ProtocolResult<()> {
    let read = Obj::new(
        value,
        path,
        &[
            "address",
            "exists",
            "slot",
            "lamports",
            "owner",
            "dataBase64",
        ],
    )?;
    pubkey(read.req("address")?, &read.path_of("address"))?;
    let exists = as_bool(read.req("exists")?, &read.path_of("exists"))?;
    decimal(read.req("slot")?, &read.path_of("slot"), U64_MAX)?;
    for key in ["lamports", "owner", "dataBase64"] {
        let present = read.opt(key)?.is_some();
        if present != exists {
            let message = if exists {
                format!("existing accounts carry '{key}'")
            } else {
                format!("missing accounts carry no '{key}'")
            };
            let code = if exists {
                "missing-field"
            } else {
                "invalid-value"
            };
            return Err(err(code, &read.path_of(key), message));
        }
    }
    if exists {
        decimal(read.req("lamports")?, &read.path_of("lamports"), U64_MAX)?;
        pubkey(read.req("owner")?, &read.path_of("owner"))?;
        let data = as_str(read.req("dataBase64")?, &read.path_of("dataBase64"))?;
        if super::release::decode_base64(data).is_none() {
            return Err(err(
                "invalid-value",
                &read.path_of("dataBase64"),
                "expected canonical base64",
            ));
        }
    }
    Ok(())
}

pub fn validate_provider_snapshot_value(value: &Value) -> ProtocolResult<ProviderSnapshotV1> {
    reject_floats(value, "")?;
    let root = Obj::new(
        value,
        "",
        &[
            "schema",
            "contextSlot",
            "reads",
            "ctxCalls",
            "observedAt",
            "expiresAt",
        ],
    )?;
    schema_tag(root.req("schema")?, "/schema", PROVIDER_SNAPSHOT_SCHEMA_V1)?;
    decimal(root.req("contextSlot")?, "/contextSlot", U64_MAX)?;
    let reads = root.req("reads")?;
    as_array(reads, "/reads", MAX_SNAPSHOT_READS)?;
    sorted_entries(reads, "/reads", |entry, path| {
        let entry = Obj::new(entry, path, &["name", "result"])?;
        let name = identifier_value(entry.req("name")?, &entry.path_of("name"))?;
        shape_account_read(entry.req("result")?, &entry.path_of("result"))?;
        Ok(name.to_string())
    })?;
    let calls = as_array(root.req("ctxCalls")?, "/ctxCalls", MAX_CTX_CALLS)?;
    for (i, call) in calls.iter().enumerate() {
        let path = index("/ctxCalls", i);
        let call = Obj::new(call, &path, &["seq", "method", "args", "resultSha256"])?;
        let seq = as_integer(
            call.req("seq")?,
            &call.path_of("seq"),
            0,
            MAX_CTX_CALLS as i64,
        )?;
        if seq != i as i64 {
            return Err(err(
                "invalid-value",
                &call.path_of("seq"),
                format!("expected seq {i}"),
            ));
        }
        one_of(
            call.req("method")?,
            &call.path_of("method"),
            &CtxMethod::ALL,
        )?;
        call.req("args")?;
        sha256_hex(call.req("resultSha256")?, &call.path_of("resultSha256"))?;
    }
    let observed = as_integer(root.req("observedAt")?, "/observedAt", 0, MAX_SAFE)?;
    let expires = as_integer(root.req("expiresAt")?, "/expiresAt", 0, MAX_SAFE)?;
    if expires < observed {
        return Err(err(
            "invalid-value",
            "/expiresAt",
            "expiresAt is before observedAt",
        ));
    }
    model(value)
}

// ---------------------------------------------------------------------------
// Execution intent
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecutionIntentV1 {
    pub schema: String,
    pub execution_id: String,
    pub deployment: IntentDeploymentV1,
    pub step: String,
    pub instance_id: Option<String>,
    pub participant_id: Option<String>,
    pub wallet: String,
    /// The role the wallet signs as, for role steps.
    pub role: Option<String>,
    /// Opaque id of the integrator account the session belongs to.
    pub caller_account: String,
    pub session_key_class: SessionKeyClass,
    pub transaction_format: TransactionFormat,
    /// Lowercase hex SHA-256 of the canonical args JSON.
    pub input_sha256: String,
    pub resource_hashes: Vec<HashId<ProtocolResource>>,
    pub provider_snapshot_hash: HashId<ProviderSnapshot>,
    pub fees: Option<IntentFeesV1>,
    pub signers: Vec<IntentSignerV1>,
    pub transactions: Vec<IntentTransactionV1>,
    pub blockhash: String,
    pub last_valid_block_height: String,
    pub expires_at: i64,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IntentDeploymentV1 {
    pub deployment_id: String,
    pub revision: u32,
    pub release_hash: HashId<ProtocolRelease>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SessionKeyClass {
    Publishable,
    Secret,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TransactionFormat {
    /// One Transaction V1 message per transaction. The default.
    #[default]
    V1,
    /// Legacy v0 messages, split to fit 1,232 bytes, for wallets without V1.
    V0,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IntentFeesV1 {
    pub asset: IntentFeeAssetV1,
    pub gross: String,
    pub total: String,
    pub creator: String,
    pub platform: String,
    pub placement: FeePlacement,
    pub platform_destination: String,
    pub creator_destination: Option<String>,
    /// Set when a cap or an override changed the platform leg. Additive: an
    /// intent without it hashes exactly as before the field existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fee_adjustment: Option<FeeAdjustmentV1>,
}

/// Why the platform leg differs from the fee policy's rate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FeeAdjustmentV1 {
    pub kind: FeeAdjustmentKind,
    /// The cap or override applied: `^[A-Za-z0-9_-]{1,64}$`.
    pub id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FeeAdjustmentKind {
    /// A periodic cap shrank the platform leg to the remaining headroom.
    Cap,
    /// A negotiated rate replaced the policy's platform rate.
    Override,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum IntentFeeAssetV1 {
    Native,
    Spl { mint: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FeePlacement {
    Before,
    After,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IntentSignerV1 {
    pub class: SignerClass,
    pub pubkey: String,
    /// Ephemeral key name or role name.
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IntentTransactionV1 {
    pub index: u32,
    /// Lowercase hex SHA-256 of the exact message bytes.
    pub message_sha256: String,
    pub signers: Vec<String>,
}

impl ExecutionIntentV1 {
    pub fn hash(&self) -> Result<HashId<ExecutionIntent>, HashError> {
        hash_jcs::<ExecutionIntent, _>(self)
    }
}

/// Lowercase canonical UUID text.
pub(crate) fn uuid<'a>(value: &'a Value, path: &str) -> ProtocolResult<&'a str> {
    let text = as_str(value, path)?;
    let bytes = text.as_bytes();
    let valid = bytes.len() == 36
        && bytes.iter().enumerate().all(|(i, b)| match i {
            8 | 13 | 18 | 23 => *b == b'-',
            _ => b.is_ascii_digit() || (b'a'..=b'f').contains(b),
        });
    if valid {
        Ok(text)
    } else {
        Err(err("invalid-value", path, "expected a lowercase UUID"))
    }
}

const INTENT_KEYS: &[&str] = &[
    "schema",
    "executionId",
    "deployment",
    "step",
    "instanceId",
    "participantId",
    "wallet",
    "role",
    "callerAccount",
    "sessionKeyClass",
    "transactionFormat",
    "inputSha256",
    "resourceHashes",
    "providerSnapshotHash",
    "fees",
    "signers",
    "transactions",
    "blockhash",
    "lastValidBlockHeight",
    "expiresAt",
    "createdAt",
];

pub fn validate_execution_intent_value(value: &Value) -> ProtocolResult<ExecutionIntentV1> {
    reject_floats(value, "")?;
    let root = Obj::new(value, "", INTENT_KEYS)?;
    schema_tag(root.req("schema")?, "/schema", EXECUTION_INTENT_SCHEMA_V1)?;
    uuid(root.req("executionId")?, "/executionId")?;
    let deployment = Obj::new(
        root.req("deployment")?,
        "/deployment",
        &["deploymentId", "revision", "releaseHash"],
    )?;
    uuid(deployment.req("deploymentId")?, "/deployment/deploymentId")?;
    as_integer(
        deployment.req("revision")?,
        "/deployment/revision",
        1,
        u32::MAX as i64,
    )?;
    hash_id::<ProtocolRelease>(deployment.req("releaseHash")?, "/deployment/releaseHash")?;
    identifier_value(root.req("step")?, "/step")?;
    if let Some(value) = root.nullable("instanceId")? {
        uuid(value, "/instanceId")?;
    }
    if let Some(value) = root.nullable("participantId")? {
        uuid(value, "/participantId")?;
    }
    pubkey(root.req("wallet")?, "/wallet")?;
    if let Some(value) = root.nullable("role")? {
        identifier_value(value, "/role")?;
    }
    let caller = bounded_str(root.req("callerAccount")?, "/callerAccount", 64)?;
    if caller.is_empty() {
        return Err(err(
            "invalid-value",
            "/callerAccount",
            "callerAccount must not be empty",
        ));
    }
    one_of(
        root.req("sessionKeyClass")?,
        "/sessionKeyClass",
        &["publishable", "secret"],
    )?;
    one_of(
        root.req("transactionFormat")?,
        "/transactionFormat",
        &["v1", "v0"],
    )?;
    sha256_hex(root.req("inputSha256")?, "/inputSha256")?;
    sorted_entries(
        root.req("resourceHashes")?,
        "/resourceHashes",
        |entry, path| Ok(hash_id::<ProtocolResource>(entry, path)?.to_string()),
    )?;
    hash_id::<ProviderSnapshot>(root.req("providerSnapshotHash")?, "/providerSnapshotHash")?;
    if let Some(value) = root.nullable("fees")? {
        shape_fees(value, "/fees")?;
    }
    let signers = as_array(root.req("signers")?, "/signers", MAX_INTENT_SIGNERS)?;
    for (i, signer) in signers.iter().enumerate() {
        let path = index("/signers", i);
        let signer = Obj::new(signer, &path, &["class", "pubkey", "name"])?;
        one_of(
            signer.req("class")?,
            &signer.path_of("class"),
            &SignerClass::ALL,
        )?;
        pubkey(signer.req("pubkey")?, &signer.path_of("pubkey"))?;
        if let Some(value) = signer.nullable("name")? {
            identifier_value(value, &signer.path_of("name"))?;
        }
    }
    let transactions = as_array(
        root.req("transactions")?,
        "/transactions",
        MAX_INTENT_TRANSACTIONS,
    )?;
    if transactions.is_empty() {
        return Err(err(
            "invalid-value",
            "/transactions",
            "an intent has at least one transaction",
        ));
    }
    for (i, transaction) in transactions.iter().enumerate() {
        let path = index("/transactions", i);
        let transaction = Obj::new(transaction, &path, &["index", "messageSha256", "signers"])?;
        let position = as_integer(
            transaction.req("index")?,
            &transaction.path_of("index"),
            0,
            7,
        )?;
        if position != i as i64 {
            return Err(err(
                "invalid-value",
                &transaction.path_of("index"),
                format!("expected index {i}"),
            ));
        }
        sha256_hex(
            transaction.req("messageSha256")?,
            &transaction.path_of("messageSha256"),
        )?;
        let signer_path = transaction.path_of("signers");
        let signer_list = unique_strings(
            transaction.req("signers")?,
            &signer_path,
            MAX_INTENT_SIGNERS,
            pubkey,
        )?;
        if signer_list.is_empty() {
            return Err(err(
                "invalid-value",
                &signer_path,
                "a transaction has at least one signer",
            ));
        }
    }
    pubkey(root.req("blockhash")?, "/blockhash")?;
    decimal(
        root.req("lastValidBlockHeight")?,
        "/lastValidBlockHeight",
        U64_MAX,
    )?;
    let expires = as_integer(root.req("expiresAt")?, "/expiresAt", 0, MAX_SAFE)?;
    let created = as_integer(root.req("createdAt")?, "/createdAt", 0, MAX_SAFE)?;
    if expires < created {
        return Err(err(
            "invalid-value",
            "/expiresAt",
            "expiresAt is before createdAt",
        ));
    }
    model(value)
}

fn shape_fees(value: &Value, path: &str) -> ProtocolResult<()> {
    let fees = Obj::new(
        value,
        path,
        &[
            "asset",
            "gross",
            "total",
            "creator",
            "platform",
            "placement",
            "platformDestination",
            "creatorDestination",
            "feeAdjustment",
        ],
    )?;
    let (kind, asset) = super::spec::tagged(
        fees.req("asset")?,
        &fees.path_of("asset"),
        &[("native", &[]), ("spl", &["mint"])],
    )?;
    if kind == "spl" {
        pubkey(asset.req("mint")?, &asset.path_of("mint"))?;
    }
    let gross = decimal(fees.req("gross")?, &fees.path_of("gross"), U64_MAX)?;
    let total = decimal(fees.req("total")?, &fees.path_of("total"), U64_MAX)?;
    let creator = decimal(fees.req("creator")?, &fees.path_of("creator"), U64_MAX)?;
    let platform = decimal(fees.req("platform")?, &fees.path_of("platform"), U64_MAX)?;
    if total > gross {
        return Err(err(
            "invalid-value",
            &fees.path_of("total"),
            "the fee exceeds the gross amount",
        ));
    }
    if creator + platform != total {
        return Err(err(
            "invalid-value",
            &fees.path_of("platform"),
            "creator + platform must equal total",
        ));
    }
    one_of(
        fees.req("placement")?,
        &fees.path_of("placement"),
        &["before", "after"],
    )?;
    pubkey(
        fees.req("platformDestination")?,
        &fees.path_of("platformDestination"),
    )?;
    match fees.nullable("creatorDestination")? {
        Some(value) => {
            pubkey(value, &fees.path_of("creatorDestination"))?;
        }
        None if creator != 0 => {
            return Err(err(
                "invalid-value",
                &fees.path_of("creatorDestination"),
                "a non-zero creator leg needs a destination",
            ))
        }
        None => {}
    }
    if let Some(value) = fees.opt("feeAdjustment")? {
        let path = fees.path_of("feeAdjustment");
        let adjustment = Obj::new(value, &path, &["kind", "id"])?;
        one_of(
            adjustment.req("kind")?,
            &adjustment.path_of("kind"),
            &["cap", "override"],
        )?;
        let id_path = adjustment.path_of("id");
        let id = as_str(adjustment.req("id")?, &id_path)?;
        let valid = (1..=64).contains(&id.len())
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
        if !valid {
            return Err(err(
                "invalid-identifier",
                &id_path,
                "adjustment ids match ^[A-Za-z0-9_-]{1,64}$",
            ));
        }
    }
    Ok(())
}
