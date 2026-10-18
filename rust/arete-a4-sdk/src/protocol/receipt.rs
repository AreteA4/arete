//! Execution receipts: check a receipt offline, and check its claims against
//! the transaction itself.
//!
//! Mirror of `typescript/core/src/protocol/receipt.ts`; every SDK runs the
//! shared vectors in `test-vectors/protocol-client/execution-receipt-v1.json`.
//!
//! - [`verify_receipt`] checks the receipt-key signatures. A receipt's body
//!   is every field except `signatures`; each signature is Ed25519 over
//!   `arete.protocol-execution-receipt/v1\n<keyId>\n<bodySha256>`, where
//!   `bodySha256` is the lowercase hex SHA-256 of the body in RFC 8785 (JCS)
//!   form. A retired key verifies only receipts issued before `retiredAt`.
//! - [`verify_receipt_on_chain`] checks a transaction fetched from any RPC:
//!   its first signature and message hash, and the attested Memo v2
//!   (`arete:p2:<shortId>:<scope|->:<instance>:<participant|->:<step>:<n>/<count>`)
//!   that the attestation key signs.

use base64::Engine;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::metadata_proof::canonicalize_json;
use crate::encoding::{decode_base58, encode_base58, sha256};

/// The receipt schema tag.
pub const PROTOCOL_EXECUTION_RECEIPT_SCHEMA_V1: &str = "arete.protocol-execution-receipt/v1";
/// The receipt key set schema tag.
pub const PROTOCOL_RECEIPT_KEYS_SCHEMA_V1: &str = "arete.protocol-receipt-keys/v1";
/// The attested memo prefix.
pub const MEMO_V2_PREFIX: &str = "arete:p2:";
/// The SPL Memo program the attested memo goes through.
pub const MEMO_PROGRAM_ID: &str = "MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr";
/// The legacy SPL Memo program. A memo through it never attests.
const LEGACY_MEMO_PROGRAM_ID: &str = "Memo1UhkJRfHyvLMcVucJwxXeuD728EqVDDwQDxFMNo";

/// A signed execution receipt as `GET /api/protocol-receipts/:signature`
/// returns it. Unknown fields are kept in `extra` so a receipt round-trips
/// byte-for-byte into [`verify_receipt`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolExecutionReceipt {
    pub schema: String,
    /// The transaction's first signature, base58.
    pub signature: String,
    /// Decimal string.
    pub slot: String,
    pub deployment: ProtocolReceiptDeployment,
    /// `reviewed` or `unreviewed`.
    pub trust_tier: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_hash: Option<String>,
    pub step: String,
    pub instance_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub participant_id: Option<String>,
    pub transaction: ProtocolReceiptTransaction,
    pub message_sha256: String,
    pub intent_hash: String,
    pub attestation_pubkey: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fees: Option<ProtocolReceiptFees>,
    /// `confirmed` or `finalized`.
    pub status: String,
    /// Unix seconds.
    pub issued_at: i64,
    pub signatures: Vec<ProtocolReceiptSignature>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolReceiptDeployment {
    pub deployment_id: String,
    pub short_id: String,
    pub revision: u32,
    pub release_hash: String,
}

/// `index` counts from 0; the memo's `<n>/<count>` from 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolReceiptTransaction {
    pub index: u32,
    pub count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolReceiptFees {
    /// `{"kind":"native"}` or `{"kind":"spl","mint":…}`.
    pub asset: Value,
    pub platform: String,
    pub creator: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolReceiptSignature {
    pub key_id: String,
    /// Base64 Ed25519 signature.
    pub signature: String,
}

/// The published receipt keys (`GET /api/protocol-receipts/keys`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolReceiptKeySet {
    pub schema: String,
    pub keys: Vec<ProtocolReceiptKey>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolReceiptKey {
    pub key_id: String,
    /// Base58 Ed25519 public key.
    pub public_key: String,
    /// Unix seconds. Receipts issued at or after this time do not verify
    /// with this key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retired_at: Option<i64>,
}

/// Why a receipt did not verify. `code` is stable across the SDKs.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{code}: {message}")]
pub struct ReceiptVerificationError {
    /// `invalid-receipt`, `unknown-key`, `key-retired`, `bad-signature`,
    /// `invalid-transaction`, `transaction-failed`, `slot-mismatch`,
    /// `signature-mismatch`, `message-hash-mismatch`,
    /// `attestation-not-signer`, `memo-missing`, `memo-not-unique`,
    /// `memo-invalid`, `memo-mismatch` or `attestation-signature-invalid`.
    pub code: &'static str,
    pub message: String,
}

fn fail<T>(code: &'static str, message: impl Into<String>) -> Result<T, ReceiptVerificationError> {
    Err(ReceiptVerificationError {
        code,
        message: message.into(),
    })
}

/// A receipt whose signature verified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedReceipt {
    /// The key whose signature verified.
    pub key_id: String,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The receipt body (`signatures` removed) in RFC 8785 (JCS) form.
pub fn receipt_canonical_body(receipt: &Value) -> Result<String, ReceiptVerificationError> {
    let Some(object) = receipt.as_object() else {
        return fail("invalid-receipt", "a receipt is a JSON object");
    };
    let mut body = object.clone();
    body.remove("signatures");
    canonicalize_json(&Value::Object(body)).map_err(|error| ReceiptVerificationError {
        code: "invalid-receipt",
        message: error.to_string(),
    })
}

/// The exact bytes a receipt key signs.
pub fn receipt_signing_message(key_id: &str, body_sha256: &str) -> Vec<u8> {
    format!("{PROTOCOL_EXECUTION_RECEIPT_SCHEMA_V1}\n{key_id}\n{body_sha256}").into_bytes()
}

fn verifying_key(base58: &str) -> Option<VerifyingKey> {
    let bytes: [u8; 32] = decode_base58(base58).ok()?.try_into().ok()?;
    VerifyingKey::from_bytes(&bytes).ok()
}

/// Verify a receipt's signatures against the published key set, offline.
///
/// The receipt is valid when any of its signatures verifies with a key
/// that was not retired when the receipt was issued. Pass the receipt JSON
/// exactly as served (or [`ProtocolExecutionReceipt`] through
/// `serde_json::to_value`).
pub fn verify_receipt(
    receipt: &Value,
    keys: &ProtocolReceiptKeySet,
) -> Result<VerifiedReceipt, ReceiptVerificationError> {
    if receipt.get("schema").and_then(Value::as_str) != Some(PROTOCOL_EXECUTION_RECEIPT_SCHEMA_V1) {
        return fail(
            "invalid-receipt",
            "not an arete.protocol-execution-receipt/v1",
        );
    }
    let Some(issued_at) = receipt.get("issuedAt").and_then(Value::as_i64) else {
        return fail("invalid-receipt", "issuedAt must be unix seconds");
    };
    let signatures: Vec<ProtocolReceiptSignature> = receipt
        .get("signatures")
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default();
    if signatures.is_empty() {
        return fail("invalid-receipt", "the receipt carries no signatures");
    }
    let body = receipt_canonical_body(receipt)?;
    let body_sha256 = hex(&sha256(&[body.as_bytes()]));
    let (mut bad, mut retired) = (false, false);
    for entry in &signatures {
        let Some(key) = keys.keys.iter().find(|key| key.key_id == entry.key_id) else {
            continue;
        };
        if key.retired_at.is_some_and(|at| issued_at >= at) {
            retired = true;
            continue;
        }
        let signature = base64::engine::general_purpose::STANDARD
            .decode(&entry.signature)
            .ok()
            .and_then(|bytes| <[u8; 64]>::try_from(bytes).ok());
        let message = receipt_signing_message(&entry.key_id, &body_sha256);
        let verified = match (verifying_key(&key.public_key), signature) {
            (Some(public_key), Some(signature)) => public_key
                .verify(&message, &Signature::from_bytes(&signature))
                .is_ok(),
            _ => false,
        };
        if verified {
            return Ok(VerifiedReceipt {
                key_id: entry.key_id.clone(),
            });
        }
        bad = true;
    }
    if bad {
        fail("bad-signature", "no receipt signature verifies")
    } else if retired {
        fail(
            "key-retired",
            "the receipt was signed by a key retired before it was issued",
        )
    } else {
        fail("unknown-key", "no receipt signature names a published key")
    }
}

// ---------------------------------------------------------------------------
// On-chain check
// ---------------------------------------------------------------------------

struct Instruction {
    program: usize,
    accounts: Vec<usize>,
    data: Vec<u8>,
}

struct WireTransaction {
    signatures: Vec<[u8; 64]>,
    message: Vec<u8>,
    required_signers: usize,
    static_keys: Vec<[u8; 32]>,
    instructions: Vec<Instruction>,
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, len: usize) -> Option<&'a [u8]> {
        let slice = self.bytes.get(self.at..self.at.checked_add(len)?)?;
        self.at += len;
        Some(slice)
    }

    fn byte(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }

    fn compact(&mut self) -> Option<usize> {
        let mut value = 0usize;
        for index in 0..3 {
            let byte = self.byte()?;
            value |= ((byte & 0x7f) as usize) << (7 * index);
            if byte & 0x80 == 0 {
                return Some(value);
            }
        }
        None
    }

    fn key(&mut self) -> Option<[u8; 32]> {
        self.take(32)?.try_into().ok()
    }

    fn signature(&mut self) -> Option<[u8; 64]> {
        self.take(64)?.try_into().ok()
    }
}

/// Configuration bits a V1 message may set; each adds one 4-byte value.
const V1_KNOWN_CONFIG_BITS: u32 = 0b11111;

/// Decode a legacy, v0 or V1 wire transaction.
fn decode_wire(bytes: &[u8]) -> Option<WireTransaction> {
    let mut reader = Reader { bytes, at: 0 };
    if bytes.first() == Some(&0x81) {
        reader.at = 1;
        let header = reader.take(3)?;
        let required = header[0] as usize;
        let mask = u32::from_le_bytes(reader.take(4)?.try_into().ok()?);
        if mask & !V1_KNOWN_CONFIG_BITS != 0 {
            return None;
        }
        reader.take(32)?;
        let instruction_count = reader.byte()? as usize;
        let key_count = reader.byte()? as usize;
        let static_keys = (0..key_count)
            .map(|_| reader.key())
            .collect::<Option<Vec<_>>>()?;
        reader.take(mask.count_ones() as usize * 4)?;
        let headers = reader.take(instruction_count * 4)?;
        let mut instructions = Vec::with_capacity(instruction_count);
        for header in headers.chunks(4) {
            let accounts = reader.take(header[1] as usize)?;
            let data = reader.take(u16::from_le_bytes([header[2], header[3]]) as usize)?;
            instructions.push(Instruction {
                program: header[0] as usize,
                accounts: accounts.iter().map(|&a| a as usize).collect(),
                data: data.to_vec(),
            });
        }
        let message = bytes[..reader.at].to_vec();
        let signatures = (0..required)
            .map(|_| reader.signature())
            .collect::<Option<Vec<_>>>()?;
        if reader.at != bytes.len() || required > key_count {
            return None;
        }
        return Some(WireTransaction {
            signatures,
            message,
            required_signers: required,
            static_keys,
            instructions,
        });
    }
    let signature_count = reader.compact()?;
    let signatures = (0..signature_count)
        .map(|_| reader.signature())
        .collect::<Option<Vec<_>>>()?;
    let message_start = reader.at;
    let versioned = *bytes.get(reader.at)? & 0x80 != 0;
    if versioned {
        if bytes[reader.at] != 0x80 {
            return None;
        }
        reader.at += 1;
    }
    let header = reader.take(3)?;
    let required = header[0] as usize;
    let key_count = reader.compact()?;
    let static_keys = (0..key_count)
        .map(|_| reader.key())
        .collect::<Option<Vec<_>>>()?;
    reader.take(32)?;
    let instruction_count = reader.compact()?;
    let mut instructions = Vec::with_capacity(instruction_count);
    for _ in 0..instruction_count {
        let program = reader.byte()? as usize;
        let account_count = reader.compact()?;
        let accounts = reader.take(account_count)?;
        let data_len = reader.compact()?;
        let data = reader.take(data_len)?;
        instructions.push(Instruction {
            program,
            accounts: accounts.iter().map(|&a| a as usize).collect(),
            data: data.to_vec(),
        });
    }
    if versioned {
        let lookups = reader.compact()?;
        for _ in 0..lookups {
            reader.take(32)?;
            let writable = reader.compact()?;
            reader.take(writable)?;
            let readonly = reader.compact()?;
            reader.take(readonly)?;
        }
    }
    if reader.at != bytes.len() || signature_count != required || required > key_count {
        return None;
    }
    Some(WireTransaction {
        signatures,
        message: bytes[message_start..].to_vec(),
        required_signers: required,
        static_keys,
        instructions,
    })
}

/// The memo form of a lowercase UUID: base58 of its 16 bytes.
pub fn receipt_memo_id(uuid: &str) -> Option<String> {
    let valid = uuid.len() == 36
        && uuid.bytes().enumerate().all(|(i, b)| match i {
            8 | 13 | 18 | 23 => b == b'-',
            _ => b.is_ascii_digit() || (b'a'..=b'f').contains(&b),
        });
    if !valid {
        return None;
    }
    let hex_text: String = uuid.chars().filter(|c| *c != '-').collect();
    let bytes: Vec<u8> = (0..16)
        .map(|i| u8::from_str_radix(&hex_text[i * 2..i * 2 + 2], 16).ok())
        .collect::<Option<_>>()?;
    Some(encode_base58(&bytes))
}

fn invalid_transaction<T>(message: &str) -> Result<T, ReceiptVerificationError> {
    fail("invalid-transaction", message)
}

/// The wire bytes and, for an RPC response, its slot.
fn transaction_bytes(
    transaction: &Value,
) -> Result<(Vec<u8>, Option<u64>), ReceiptVerificationError> {
    let base64_text = |text: &str| {
        base64::engine::general_purpose::STANDARD
            .decode(text)
            .or_else(|_| invalid_transaction("the transaction is not base64"))
    };
    match transaction {
        Value::String(text) => Ok((base64_text(text)?, None)),
        Value::Object(response) => {
            let meta_error = response
                .get("meta")
                .and_then(|meta| meta.get("err"))
                .filter(|error| !error.is_null());
            if let Some(error) = meta_error {
                return fail(
                    "transaction-failed",
                    format!("the transaction failed: {error}"),
                );
            }
            let slot = match response.get("slot") {
                None | Some(Value::Null) => None,
                Some(slot) => Some(
                    slot.as_u64()
                        .map_or_else(|| invalid_transaction("slot must be an integer"), Ok)?,
                ),
            };
            let encoded = response
                .get("transaction")
                .and_then(Value::as_array)
                .filter(|pair| pair.get(1).and_then(Value::as_str) == Some("base64"))
                .and_then(|pair| pair.first())
                .and_then(Value::as_str);
            match encoded {
                Some(text) => Ok((base64_text(text)?, slot)),
                None => invalid_transaction(
                    "fetch the transaction with encoding \"base64\" so its message bytes are exact",
                ),
            }
        }
        _ => invalid_transaction(
            "expected base64 wire bytes or a getTransaction result with encoding base64",
        ),
    }
}

/// Check a receipt's claims against the transaction itself.
///
/// `transaction` is the base64 wire transaction, or the result of
/// `getTransaction` with `encoding: "base64"` from any RPC (its `slot` and
/// `meta.err` are checked too). This does not check the receipt's own
/// signature: use [`verify_receipt`] for that.
pub fn verify_receipt_on_chain(
    receipt: &Value,
    transaction: &Value,
) -> Result<(), ReceiptVerificationError> {
    let receipt: ProtocolExecutionReceipt = serde_json::from_value(receipt.clone())
        .or_else(|error| fail("invalid-receipt", error.to_string()))?;
    let (bytes, slot) = transaction_bytes(transaction)?;
    if let Some(slot) = slot {
        if slot.to_string() != receipt.slot {
            return fail(
                "slot-mismatch",
                format!(
                    "the transaction landed in slot {slot}, not {}",
                    receipt.slot
                ),
            );
        }
    }
    verify_wire(&receipt, &bytes)
}

/// [`verify_receipt_on_chain`] over raw wire bytes.
pub fn verify_receipt_on_chain_bytes(
    receipt: &ProtocolExecutionReceipt,
    wire: &[u8],
) -> Result<(), ReceiptVerificationError> {
    verify_wire(receipt, wire)
}

fn verify_wire(
    receipt: &ProtocolExecutionReceipt,
    wire: &[u8],
) -> Result<(), ReceiptVerificationError> {
    let Some(tx) = decode_wire(wire) else {
        return invalid_transaction("not a legacy, v0 or V1 wire transaction");
    };
    let first = tx
        .signatures
        .first()
        .map(|signature| encode_base58(signature));
    if first.as_deref() != Some(receipt.signature.as_str()) {
        return fail(
            "signature-mismatch",
            "the transaction's signature is not the receipt's",
        );
    }
    if hex(&sha256(&[&tx.message])) != receipt.message_sha256 {
        return fail(
            "message-hash-mismatch",
            "the message hash differs from the receipt's",
        );
    }
    let attestation = decode_base58(&receipt.attestation_pubkey)
        .ok()
        .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok());
    let Some(attestation) = attestation else {
        return fail("invalid-receipt", "attestationPubkey is not a public key");
    };
    let Some(signer_index) = tx.static_keys[..tx.required_signers]
        .iter()
        .position(|key| *key == attestation)
    else {
        return fail(
            "attestation-not-signer",
            "the attestation key does not sign the transaction",
        );
    };
    let memo_program = decode_base58(MEMO_PROGRAM_ID).unwrap_or_default();
    let legacy_memo_program = decode_base58(LEGACY_MEMO_PROGRAM_ID).unwrap_or_default();
    let program_of = |ix: &Instruction| tx.static_keys.get(ix.program).map(|key| key.as_slice());
    // Memo instructions (either SPL Memo program) that list the key.
    let listing: Vec<&Instruction> = tx
        .instructions
        .iter()
        .filter(|ix| {
            program_of(ix).is_some_and(|program| {
                program == memo_program.as_slice() || program == legacy_memo_program.as_slice()
            }) && ix
                .accounts
                .iter()
                .any(|&account| tx.static_keys.get(account) == Some(&attestation))
        })
        .collect();
    let memo = match listing.as_slice() {
        [] => {
            return fail(
                "memo-missing",
                "no memo instruction lists the attestation key",
            )
        }
        [memo] => *memo,
        _ => {
            return fail(
                "memo-not-unique",
                "more than one memo instruction lists the attestation key",
            )
        }
    };
    let is_memo = program_of(memo) == Some(memo_program.as_slice());
    let only_account = memo.accounts.len() == 1;
    let text = std::str::from_utf8(&memo.data).ok();
    let fields = match (is_memo && only_account, text.and_then(parse_memo)) {
        (true, Some(fields)) => fields,
        _ => {
            return fail(
                "memo-invalid",
                "the attestation key's instruction is not an attested Memo v2",
            )
        }
    };
    let expected_participant = match &receipt.participant_id {
        Some(id) => receipt_memo_id(id),
        None => Some("-".to_string()),
    };
    let matches = fields.short_id == receipt.deployment.short_id
        && Some(fields.instance.as_str()) == receipt_memo_id(&receipt.instance_id).as_deref()
        && Some(fields.participant.as_str()) == expected_participant.as_deref()
        && fields.step == receipt.step
        && fields.position
            == format!(
                "{}/{}",
                receipt.transaction.index + 1,
                receipt.transaction.count
            );
    if !matches {
        return fail(
            "memo-mismatch",
            "the attested memo names a different deployment, instance, participant, step or position",
        );
    }
    let verified = VerifyingKey::from_bytes(&attestation)
        .ok()
        .zip(tx.signatures.get(signer_index))
        .is_some_and(|(key, signature)| {
            key.verify(&tx.message, &Signature::from_bytes(signature))
                .is_ok()
        });
    if !verified {
        return fail(
            "attestation-signature-invalid",
            "the attestation key's signature does not verify over the message",
        );
    }
    Ok(())
}

struct MemoFields {
    short_id: String,
    instance: String,
    participant: String,
    step: String,
    position: String,
}

fn parse_memo(text: &str) -> Option<MemoFields> {
    let rest = text.strip_prefix(MEMO_V2_PREFIX)?;
    let parts: Vec<&str> = rest.split(':').collect();
    let [short_id, _scope, instance, participant, step, position] = parts.as_slice() else {
        return None;
    };
    Some(MemoFields {
        short_id: short_id.to_string(),
        instance: instance.to_string(),
        participant: participant.to_string(),
        step: step.to_string(),
        position: position.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vectors() -> Value {
        serde_json::from_str(include_str!(
            "../../../../test-vectors/protocol-client/execution-receipt-v1.json"
        ))
        .unwrap()
    }

    #[test]
    fn offline_vectors() {
        let vectors = vectors();
        let keys: ProtocolReceiptKeySet =
            serde_json::from_value(vectors["keySet"].clone()).unwrap();
        for case in vectors["verifyCases"].as_array().unwrap() {
            let name = case["name"].as_str().unwrap();
            let expected = &case["expected"];
            let result = verify_receipt(&case["receipt"], &keys);
            if expected["valid"].as_bool().unwrap() {
                let verified = result.unwrap_or_else(|e| panic!("{name}: {e}"));
                assert_eq!(
                    verified.key_id,
                    expected["keyId"].as_str().unwrap(),
                    "{name}"
                );
            } else {
                let error = result.expect_err(name);
                assert_eq!(error.code, expected["error"].as_str().unwrap(), "{name}");
            }
            if let Some(body) = expected["canonicalBody"].as_str() {
                assert_eq!(receipt_canonical_body(&case["receipt"]).unwrap(), body);
                let sha = expected["bodySha256"].as_str().unwrap();
                assert_eq!(hex(&sha256(&[body.as_bytes()])), sha);
                assert_eq!(
                    receipt_signing_message("test-receipt-1", sha),
                    expected["signingMessage"].as_str().unwrap().as_bytes()
                );
            }
            // A typed round trip keeps the exact body.
            if let Ok(typed) =
                serde_json::from_value::<ProtocolExecutionReceipt>(case["receipt"].clone())
            {
                assert_eq!(
                    receipt_canonical_body(&serde_json::to_value(&typed).unwrap()).unwrap(),
                    receipt_canonical_body(&case["receipt"]).unwrap(),
                    "{name}"
                );
            }
        }
    }

    #[test]
    fn on_chain_vectors() {
        let vectors = vectors();
        for case in vectors["onChainCases"].as_array().unwrap() {
            let name = case["name"].as_str().unwrap();
            let result = verify_receipt_on_chain(&case["receipt"], &case["transaction"]);
            match case["expected"]["error"].as_str() {
                None => result.unwrap_or_else(|e| panic!("{name}: {e}")),
                Some(code) => assert_eq!(result.expect_err(name).code, code, "{name}"),
            }
        }
    }

    #[test]
    fn memo_ids_are_base58_of_the_uuid_bytes() {
        assert_eq!(
            receipt_memo_id("00000000-0000-0000-0000-000000000000").as_deref(),
            Some("1111111111111111")
        );
        assert_eq!(receipt_memo_id("not-a-uuid"), None);
    }
}
