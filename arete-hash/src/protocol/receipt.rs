//! Execution receipts: a signed statement that one transaction of an
//! execution intent landed with the attested memo of its deployment.
//!
//! A receipt is `arete.protocol-execution-receipt/v1` JSON. Its **body** is
//! every field except `signatures`; each signature is an Ed25519 signature
//! by a receipt key over [`receipt_signing_message`]:
//!
//! ```text
//! arete.protocol-execution-receipt/v1
//! <keyId>
//! <lowercase hex SHA-256 of the body's Arete JCS bytes>
//! ```
//!
//! The receipt keys are published as an `arete.protocol-receipt-keys/v1`
//! key set. Anyone can check a receipt offline against that key set, and
//! check its claims against the transaction itself, fetched from any RPC:
//! the message hash, and the attested Memo v2
//! (`arete:p2:<shortId>:<scope|->:<instance>:<participant|->:<step>:<n>/<count>`)
//! signed by the attestation key. [`parse_memo_v2`] and [`memo_id`] give the
//! memo form of the receipt's fields.
//!
//! A receipt has no hash kind: it is identified by its transaction
//! signature.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::error::{ProtocolContractError, ProtocolResult};
use super::intent::{uuid, IntentFeeAssetV1};
use super::release::{decode_base64, key_id, TrustTier};
use super::shape::*;
use super::stack::MEMO_V2_PREFIX;
use crate::{
    canonicalize_jcs, ExecutionIntent, HashError, HashId, ProtocolRelease, ProtocolReleaseReview,
};

pub const PROTOCOL_EXECUTION_RECEIPT_SCHEMA_V1: &str = "arete.protocol-execution-receipt/v1";
pub const PROTOCOL_RECEIPT_KEYS_SCHEMA_V1: &str = "arete.protocol-receipt-keys/v1";

/// At most this many receipt-key signatures, sorted by `keyId`.
pub const MAX_RECEIPT_SIGNATURES: usize = 4;
/// At most this many keys in a published key set.
pub const MAX_RECEIPT_KEYS: usize = 32;
/// The most transactions one intent can have.
const MAX_TRANSACTIONS: i64 = 8;
const U64_MAX: u128 = u64::MAX as u128;
const MAX_SAFE: i64 = 9_007_199_254_740_991;

const RECEIPT_KEYS: &[&str] = &[
    "schema",
    "signature",
    "slot",
    "deployment",
    "trustTier",
    "reviewHash",
    "step",
    "instanceId",
    "participantId",
    "transaction",
    "messageSha256",
    "intentHash",
    "attestationPubkey",
    "fees",
    "status",
    "issuedAt",
    "signatures",
];

/// A signed execution receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProtocolExecutionReceiptV1 {
    pub schema: String,
    /// The transaction's first signature, base58.
    pub signature: String,
    /// The slot the transaction landed in, as a decimal string.
    pub slot: String,
    pub deployment: ReceiptDeploymentV1,
    pub trust_tier: TrustTier,
    /// The release review, present exactly when `trustTier` is `reviewed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_hash: Option<HashId<ProtocolReleaseReview>>,
    pub step: String,
    pub instance_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub participant_id: Option<String>,
    pub transaction: ReceiptTransactionV1,
    /// Lowercase hex SHA-256 of the exact message bytes.
    pub message_sha256: String,
    pub intent_hash: HashId<ExecutionIntent>,
    /// The revision's attestation key: the memo's only account and a
    /// signer of the message.
    pub attestation_pubkey: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fees: Option<ReceiptFeesV1>,
    pub status: ReceiptStatus,
    /// Unix seconds.
    pub issued_at: i64,
    pub signatures: Vec<ReceiptSignatureV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReceiptDeploymentV1 {
    pub deployment_id: String,
    /// The deployment short id the memo carries.
    pub short_id: String,
    pub revision: u32,
    pub release_hash: HashId<ProtocolRelease>,
}

/// Which transaction of the intent this is: `index` counts from 0, the
/// memo's `<n>/<count>` counts from 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReceiptTransactionV1 {
    pub index: u32,
    pub count: u32,
}

/// The fee legs charged in this intent, in base units of `asset`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReceiptFeesV1 {
    pub asset: IntentFeeAssetV1,
    pub platform: String,
    pub creator: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReceiptStatus {
    Confirmed,
    Finalized,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReceiptSignatureV1 {
    pub key_id: String,
    /// Base64 (standard, padded) Ed25519 signature over
    /// [`receipt_signing_message`].
    pub signature: String,
}

impl ProtocolExecutionReceiptV1 {
    /// The body's Arete JCS bytes: the receipt without `signatures`.
    pub fn canonical_body(&self) -> Result<Vec<u8>, HashError> {
        let mut body = serde_json::to_value(self)
            .map_err(|error| HashError::Serialization(error.to_string()))?;
        receipt_body_bytes(&mut body)
    }

    /// Lowercase hex SHA-256 of [`Self::canonical_body`].
    pub fn body_sha256(&self) -> Result<String, HashError> {
        Ok(hex::encode(Sha256::digest(self.canonical_body()?)))
    }

    /// The bytes receipt key `key_id` signs.
    pub fn signing_message(&self, key_id: &str) -> Result<Vec<u8>, HashError> {
        Ok(receipt_signing_message(key_id, &self.body_sha256()?))
    }
}

/// The body of a receipt JSON value (`signatures` removed) in Arete JCS
/// form. Works on any receipt-shaped value, so verifiers can hash the body
/// exactly as served.
pub fn receipt_body_bytes(receipt: &mut Value) -> Result<Vec<u8>, HashError> {
    if let Value::Object(map) = receipt {
        map.remove("signatures");
    }
    canonicalize_jcs(receipt)
}

/// The exact bytes a receipt key signs:
/// `arete.protocol-execution-receipt/v1\n<keyId>\n<bodySha256>`.
pub fn receipt_signing_message(key_id: &str, body_sha256: &str) -> Vec<u8> {
    format!("{PROTOCOL_EXECUTION_RECEIPT_SCHEMA_V1}\n{key_id}\n{body_sha256}").into_bytes()
}

/// A base58 Solana transaction signature (64 bytes).
fn transaction_signature<'a>(value: &'a Value, path: &str) -> ProtocolResult<&'a str> {
    let text = as_str(value, path)?;
    let valid = text.len() <= 88
        && bs58::decode(text)
            .into_vec()
            .is_ok_and(|bytes| bytes.len() == 64);
    if valid {
        Ok(text)
    } else {
        Err(err(
            "invalid-signature",
            path,
            "expected a base58 64-byte transaction signature",
        ))
    }
}

/// `^[A-Za-z0-9_-]{1,64}$`, the memo component alphabet.
fn short_id<'a>(value: &'a Value, path: &str) -> ProtocolResult<&'a str> {
    let text = as_str(value, path)?;
    let valid = (1..=64).contains(&text.len())
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    if valid {
        Ok(text)
    } else {
        Err(err(
            "invalid-identifier",
            path,
            "short ids match ^[A-Za-z0-9_-]{1,64}$",
        ))
    }
}

fn base64_signature(value: &Value, path: &str) -> ProtocolResult<()> {
    let text = as_str(value, path)?;
    match decode_base64(text) {
        Some(bytes) if bytes.len() == 64 => Ok(()),
        _ => Err(err(
            "invalid-signature",
            path,
            "expected a base64 64-byte Ed25519 signature",
        )),
    }
}

fn model<T: serde::de::DeserializeOwned>(value: &Value) -> ProtocolResult<T> {
    serde_json::from_value(value.clone()).map_err(|error| {
        ProtocolContractError::new("invalid-value", "", format!("model mismatch: {error}"))
    })
}

pub fn validate_protocol_execution_receipt_value(
    value: &Value,
) -> ProtocolResult<ProtocolExecutionReceiptV1> {
    reject_floats(value, "")?;
    let root = Obj::new(value, "", RECEIPT_KEYS)?;
    schema_tag(
        root.req("schema")?,
        "/schema",
        PROTOCOL_EXECUTION_RECEIPT_SCHEMA_V1,
    )?;
    transaction_signature(root.req("signature")?, "/signature")?;
    decimal(root.req("slot")?, "/slot", U64_MAX)?;
    let deployment = Obj::new(
        root.req("deployment")?,
        "/deployment",
        &["deploymentId", "shortId", "revision", "releaseHash"],
    )?;
    uuid(deployment.req("deploymentId")?, "/deployment/deploymentId")?;
    short_id(deployment.req("shortId")?, "/deployment/shortId")?;
    as_integer(
        deployment.req("revision")?,
        "/deployment/revision",
        1,
        u32::MAX as i64,
    )?;
    hash_id::<ProtocolRelease>(deployment.req("releaseHash")?, "/deployment/releaseHash")?;
    let tier = one_of(root.req("trustTier")?, "/trustTier", &TrustTier::ALL)?;
    match (tier, root.opt("reviewHash")?) {
        ("reviewed", None) => {
            return Err(err(
                "missing-field",
                "/reviewHash",
                "a reviewed receipt carries 'reviewHash'",
            ))
        }
        ("reviewed", Some(review)) => {
            hash_id::<ProtocolReleaseReview>(review, "/reviewHash")?;
        }
        (_, Some(_)) => {
            return Err(err(
                "invalid-value",
                "/reviewHash",
                "an unreviewed receipt carries no 'reviewHash'",
            ))
        }
        (_, None) => {}
    }
    identifier_value(root.req("step")?, "/step")?;
    uuid(root.req("instanceId")?, "/instanceId")?;
    if let Some(participant) = root.opt("participantId")? {
        uuid(participant, "/participantId")?;
    }
    let transaction = Obj::new(
        root.req("transaction")?,
        "/transaction",
        &["index", "count"],
    )?;
    let position = as_integer(
        transaction.req("index")?,
        "/transaction/index",
        0,
        MAX_TRANSACTIONS - 1,
    )?;
    let count = as_integer(
        transaction.req("count")?,
        "/transaction/count",
        1,
        MAX_TRANSACTIONS,
    )?;
    if position >= count {
        return Err(err(
            "invalid-value",
            "/transaction/index",
            "index must be below count",
        ));
    }
    sha256_hex(root.req("messageSha256")?, "/messageSha256")?;
    hash_id::<ExecutionIntent>(root.req("intentHash")?, "/intentHash")?;
    pubkey(root.req("attestationPubkey")?, "/attestationPubkey")?;
    if let Some(fees) = root.opt("fees")? {
        let fees = Obj::new(fees, "/fees", &["asset", "platform", "creator"])?;
        let (kind, asset) = super::spec::tagged(
            fees.req("asset")?,
            "/fees/asset",
            &[("native", &[]), ("spl", &["mint"])],
        )?;
        if kind == "spl" {
            pubkey(asset.req("mint")?, "/fees/asset/mint")?;
        }
        decimal(fees.req("platform")?, "/fees/platform", U64_MAX)?;
        decimal(fees.req("creator")?, "/fees/creator", U64_MAX)?;
    }
    one_of(root.req("status")?, "/status", &["confirmed", "finalized"])?;
    as_integer(root.req("issuedAt")?, "/issuedAt", 0, MAX_SAFE)?;
    let signatures = as_array(
        root.req("signatures")?,
        "/signatures",
        MAX_RECEIPT_SIGNATURES,
    )?;
    if signatures.is_empty() {
        return Err(err(
            "invalid-value",
            "/signatures",
            "a receipt carries at least one signature",
        ));
    }
    let mut previous: Option<&str> = None;
    for (i, signature) in signatures.iter().enumerate() {
        let path = index("/signatures", i);
        let signature = Obj::new(signature, &path, &["keyId", "signature"])?;
        let key = key_id(signature.req("keyId")?, &signature.path_of("keyId"))?;
        base64_signature(signature.req("signature")?, &signature.path_of("signature"))?;
        if let Some(previous) = previous {
            if key == previous {
                return Err(err(
                    "duplicate-entry",
                    &path,
                    format!("'{key}' is listed twice"),
                ));
            }
            if key < previous {
                return Err(err("unsorted-entries", &path, "entries must be sorted"));
            }
        }
        previous = Some(key);
    }
    model(value)
}

// ---------------------------------------------------------------------------
// Key set
// ---------------------------------------------------------------------------

/// The published receipt keys. A retired key still verifies receipts issued
/// before `retiredAt`, so old receipts stay checkable after a rotation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProtocolReceiptKeySetV1 {
    pub schema: String,
    pub keys: Vec<ProtocolReceiptKeyV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProtocolReceiptKeyV1 {
    pub key_id: String,
    /// Base58 Ed25519 public key.
    pub public_key: String,
    /// Unix seconds. Receipts issued at or after this time do not verify
    /// with this key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retired_at: Option<i64>,
}

impl ProtocolReceiptKeySetV1 {
    /// The key that may have signed a receipt issued at `issued_at`.
    pub fn usable_key(&self, key_id: &str, issued_at: i64) -> Option<&ProtocolReceiptKeyV1> {
        self.keys.iter().find(|key| {
            key.key_id == key_id && key.retired_at.is_none_or(|retired| issued_at < retired)
        })
    }
}

pub fn validate_protocol_receipt_key_set_value(
    value: &Value,
) -> ProtocolResult<ProtocolReceiptKeySetV1> {
    reject_floats(value, "")?;
    let root = Obj::new(value, "", &["schema", "keys"])?;
    schema_tag(
        root.req("schema")?,
        "/schema",
        PROTOCOL_RECEIPT_KEYS_SCHEMA_V1,
    )?;
    let keys = as_array(root.req("keys")?, "/keys", MAX_RECEIPT_KEYS)?;
    let mut previous: Option<&str> = None;
    for (i, key) in keys.iter().enumerate() {
        let path = index("/keys", i);
        let key = Obj::new(key, &path, &["keyId", "publicKey", "retiredAt"])?;
        let id = key_id(key.req("keyId")?, &key.path_of("keyId"))?;
        pubkey(key.req("publicKey")?, &key.path_of("publicKey"))?;
        if let Some(retired) = key.opt("retiredAt")? {
            as_integer(retired, &key.path_of("retiredAt"), 0, MAX_SAFE)?;
        }
        if let Some(previous) = previous {
            if id == previous {
                return Err(err(
                    "duplicate-entry",
                    &path,
                    format!("'{id}' is listed twice"),
                ));
            }
            if id < previous {
                return Err(err("unsorted-entries", &path, "entries must be sorted"));
            }
        }
        previous = Some(id);
    }
    model(value)
}

// ---------------------------------------------------------------------------
// Memo v2
// ---------------------------------------------------------------------------

/// The fields of an attested Memo v2:
/// `arete:p2:<shortId>:<scope|->:<instance>:<participant|->:<step>:<n>/<count>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoV2 {
    pub short_id: String,
    pub scope: Option<String>,
    /// [`memo_id`] of the instance id.
    pub instance: String,
    /// [`memo_id`] of the participant id.
    pub participant: Option<String>,
    pub step: String,
    /// 1-based.
    pub n: u32,
    pub count: u32,
}

/// Parse an attested Memo v2 text. `None` when it is not one.
pub fn parse_memo_v2(text: &str) -> Option<MemoV2> {
    let rest = text.strip_prefix(MEMO_V2_PREFIX)?;
    let parts: Vec<&str> = rest.split(':').collect();
    let [short_id, scope, instance, participant, step, position] = parts.as_slice() else {
        return None;
    };
    let component = |text: &str| {
        !text.is_empty()
            && text
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    };
    if ![*short_id, *scope, *instance, *participant, *step]
        .into_iter()
        .all(component)
    {
        return None;
    }
    let (n, count) = position.split_once('/')?;
    let number = |text: &str| -> Option<u32> {
        if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) || text.starts_with('0') {
            return None;
        }
        text.parse().ok()
    };
    let optional = |text: &str| (text != "-").then(|| text.to_string());
    Some(MemoV2 {
        short_id: short_id.to_string(),
        scope: optional(scope),
        instance: instance.to_string(),
        participant: optional(participant),
        step: step.to_string(),
        n: number(n)?,
        count: number(count)?,
    })
}

/// The memo form of an instance or participant UUID: base58 of its 16
/// bytes. `None` for text that is not a lowercase UUID.
pub fn memo_id(uuid_text: &str) -> Option<String> {
    let hex_text: String = uuid_text.chars().filter(|c| *c != '-').collect();
    if uuid(&Value::String(uuid_text.to_string()), "").is_err() {
        return None;
    }
    let bytes = hex::decode(hex_text).ok()?;
    Some(bs58::encode(bytes).into_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memo_v2_round_trips_the_fields() {
        let memo = parse_memo_v2("arete:p2:raise1:-:3mJr7AoUXx2Wqd:-:open:1/2").unwrap();
        assert_eq!(memo.short_id, "raise1");
        assert_eq!(memo.scope, None);
        assert_eq!(memo.participant, None);
        assert_eq!(memo.step, "open");
        assert_eq!((memo.n, memo.count), (1, 2));
        for bad in [
            "arete:p1:a:-:b:-:s:1/1",
            "arete:p2:a:-:b:-:s:1",
            "arete:p2:a:-:b:-:s:01/1",
            "arete:p2:a:-:b:-:s:1/1:extra",
            "arete:p2::-:b:-:s:1/1",
            "arete:p2:a:-:b c:-:s:1/1",
        ] {
            assert_eq!(parse_memo_v2(bad), None, "{bad}");
        }
    }

    #[test]
    fn memo_ids_are_base58_of_the_uuid_bytes() {
        assert_eq!(
            memo_id("00000000-0000-0000-0000-000000000000").as_deref(),
            Some("1111111111111111")
        );
        assert_eq!(memo_id("not-a-uuid"), None);
        assert_eq!(memo_id("6F1C2B9A-1D3E-4F5A-8B7C-9D0E1F2A3B4C"), None);
    }
}
