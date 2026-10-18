//! Regenerate `test-vectors/protocol-client/execution-receipt-v1.json`, the
//! vectors every SDK's `verifyReceipt` and `verifyReceiptOnChain` run.
//!
//! ```bash
//! cargo run -p arete-hash --example generate_protocol_receipt_vectors
//! ```
//!
//! The keys are deterministic test keys. Never use them outside vectors.

use std::fs;
use std::path::PathBuf;

use arete_hash::protocol::*;
use ed25519_dalek::{Signer, SigningKey};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const OUTPUT: &str = "../test-vectors/protocol-client/execution-receipt-v1.json";
const RETIRED_KEY_SEED: [u8; 32] = [20u8; 32];
const ACTIVE_KEY_SEED: [u8; 32] = [21u8; 32];
const WALLET_SEED: [u8; 32] = [22u8; 32];
const ATTESTATION_SEED: [u8; 32] = [23u8; 32];
const RETIRED_AT: i64 = 1_760_000_000;
const MEMO_PROGRAM: &str = "MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr";
const SYSTEM_PROGRAM: [u8; 32] = [0u8; 32];
const SHORT_ID: &str = "raise1";
const INSTANCE_ID: &str = "2d4f6a8c-0e1f-4a2b-9c3d-5e6f7a8b9c0d";
const PARTICIPANT_ID: &str = "0b7d8f8e-5d0c-4a59-9a43-7a7f1d4e2c10";

fn b58(bytes: &[u8]) -> String {
    bs58::encode(bytes).into_string()
}

fn pubkey_bytes(text: &str) -> [u8; 32] {
    bs58::decode(text).into_vec().unwrap().try_into().unwrap()
}

fn compact(out: &mut Vec<u8>, mut value: usize) {
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

#[derive(Clone, Copy, PartialEq)]
enum Format {
    Legacy,
    V0,
    V1,
}

struct Ix {
    program: u8,
    accounts: Vec<u8>,
    data: Vec<u8>,
}

struct Message {
    /// (required signers, readonly signed, readonly unsigned)
    header: (u8, u8, u8),
    keys: Vec<[u8; 32]>,
    instructions: Vec<Ix>,
}

impl Message {
    fn bytes(&self, format: Format) -> Vec<u8> {
        let mut out = Vec::new();
        let blockhash = [9u8; 32];
        if format == Format::V1 {
            out.extend_from_slice(&[0x81, self.header.0, self.header.1, self.header.2]);
            out.extend_from_slice(&0u32.to_le_bytes());
            out.extend_from_slice(&blockhash);
            out.push(self.instructions.len() as u8);
            out.push(self.keys.len() as u8);
            for key in &self.keys {
                out.extend_from_slice(key);
            }
            for ix in &self.instructions {
                out.push(ix.program);
                out.push(ix.accounts.len() as u8);
                out.extend_from_slice(&(ix.data.len() as u16).to_le_bytes());
            }
            for ix in &self.instructions {
                out.extend_from_slice(&ix.accounts);
                out.extend_from_slice(&ix.data);
            }
            return out;
        }
        if format == Format::V0 {
            out.push(0x80);
        }
        out.extend_from_slice(&[self.header.0, self.header.1, self.header.2]);
        compact(&mut out, self.keys.len());
        for key in &self.keys {
            out.extend_from_slice(key);
        }
        out.extend_from_slice(&blockhash);
        compact(&mut out, self.instructions.len());
        for ix in &self.instructions {
            out.push(ix.program);
            compact(&mut out, ix.accounts.len());
            out.extend_from_slice(&ix.accounts);
            compact(&mut out, ix.data.len());
            out.extend_from_slice(&ix.data);
        }
        if format == Format::V0 {
            compact(&mut out, 0);
        }
        out
    }
}

struct Built {
    wire: Vec<u8>,
    message_sha256: String,
    signature: String,
}

/// Sign `message` with the first `required` of `signers`; a `None` slot
/// stays all-zero (not signed).
fn transaction(message: &Message, format: Format, signers: &[Option<&SigningKey>]) -> Built {
    let bytes = message.bytes(format);
    let signatures: Vec<[u8; 64]> = signers
        .iter()
        .map(|key| key.map_or([0u8; 64], |key| key.sign(&bytes).to_bytes()))
        .collect();
    let mut wire = Vec::new();
    if format == Format::V1 {
        wire.extend_from_slice(&bytes);
        for signature in &signatures {
            wire.extend_from_slice(signature);
        }
    } else {
        compact(&mut wire, signatures.len());
        for signature in &signatures {
            wire.extend_from_slice(signature);
        }
        wire.extend_from_slice(&bytes);
    }
    Built {
        wire,
        message_sha256: hex::encode(Sha256::digest(&bytes)),
        signature: b58(&signatures[0]),
    }
}

fn base64(bytes: &[u8]) -> String {
    encode_base64(bytes)
}

fn sign(receipt: &mut Value, signers: &[(&str, &SigningKey)]) {
    let mut entries = Vec::new();
    let body_sha256 = {
        let mut body = receipt.clone();
        hex::encode(Sha256::digest(receipt_body_bytes(&mut body).unwrap()))
    };
    for (key_id, key) in signers {
        let message = receipt_signing_message(key_id, &body_sha256);
        entries
            .push(json!({ "keyId": key_id, "signature": base64(&key.sign(&message).to_bytes()) }));
    }
    receipt["signatures"] = Value::Array(entries);
}

fn checked(receipt: Value) -> Value {
    validate_protocol_execution_receipt_value(&receipt)
        .unwrap_or_else(|e| panic!("receipt shape: {e}"));
    receipt
}

fn main() {
    let retired = SigningKey::from_bytes(&RETIRED_KEY_SEED);
    let active = SigningKey::from_bytes(&ACTIVE_KEY_SEED);
    let wallet = SigningKey::from_bytes(&WALLET_SEED);
    let attestation = SigningKey::from_bytes(&ATTESTATION_SEED);
    let wallet_key = wallet.verifying_key().to_bytes();
    let attestation_key = attestation.verifying_key().to_bytes();
    let attestation_b58 = b58(&attestation_key);
    let memo_program = pubkey_bytes(MEMO_PROGRAM);

    let key_set = json!({
        "schema": PROTOCOL_RECEIPT_KEYS_SCHEMA_V1,
        "keys": [
            { "keyId": "test-receipt-0", "publicKey": b58(retired.verifying_key().as_bytes()), "retiredAt": RETIRED_AT },
            { "keyId": "test-receipt-1", "publicKey": b58(active.verifying_key().as_bytes()) }
        ]
    });
    validate_protocol_receipt_key_set_value(&key_set).unwrap();

    let instance = memo_id(INSTANCE_ID).unwrap();
    let participant = memo_id(PARTICIPANT_ID).unwrap();
    let memo_text = |step: &str, participant: Option<&str>, n: u32, count: u32| {
        format!(
            "{MEMO_V2_PREFIX}{SHORT_ID}:-:{instance}:{}:{step}:{n}/{count}",
            participant.unwrap_or("-")
        )
    };
    // Wallet (fee payer), attestation (readonly signer), memo program.
    let attested = |text: String| Message {
        header: (2, 1, 1),
        keys: vec![wallet_key, attestation_key, memo_program],
        instructions: vec![Ix {
            program: 2,
            accounts: vec![1],
            data: text.into_bytes(),
        }],
    };
    let both = [Some(&wallet), Some(&attestation)];

    let base_receipt = |built: &Built, transaction: Value, participant_id: Option<&str>| {
        let mut receipt = json!({
            "schema": PROTOCOL_EXECUTION_RECEIPT_SCHEMA_V1,
            "signature": built.signature,
            "slot": "312000123",
            "deployment": {
                "deploymentId": "6f1c2b9a-1d3e-4f5a-8b7c-9d0e1f2a3b4c",
                "shortId": SHORT_ID,
                "revision": 3,
                "releaseHash": format!("arete:h1:protocol-release:sha256:{}", "e".repeat(64))
            },
            "trustTier": "reviewed",
            "reviewHash": format!("arete:h1:protocol-release-review:sha256:{}", "d".repeat(64)),
            "step": "contribute",
            "instanceId": INSTANCE_ID,
            "transaction": transaction,
            "messageSha256": built.message_sha256,
            "intentHash": format!("arete:h1:execution-intent:sha256:{}", "c".repeat(64)),
            "attestationPubkey": attestation_b58,
            "fees": { "asset": { "kind": "native" }, "platform": "5000000", "creator": "0" },
            "status": "finalized",
            "issuedAt": 1_760_000_100,
            "signatures": []
        });
        if let Some(participant_id) = participant_id {
            receipt["participantId"] = json!(participant_id);
        }
        receipt
    };

    // --- Offline signature cases ----------------------------------------
    let legacy = transaction(
        &attested(memo_text("contribute", None, 1, 1)),
        Format::Legacy,
        &both,
    );
    let mut signed = base_receipt(&legacy, json!({ "index": 0, "count": 1 }), None);
    sign(&mut signed, &[("test-receipt-1", &active)]);
    let signed = checked(signed);
    let body_sha256 = {
        let mut body = signed.clone();
        hex::encode(Sha256::digest(receipt_body_bytes(&mut body).unwrap()))
    };
    let canonical_body = {
        let mut body = signed.clone();
        String::from_utf8(receipt_body_bytes(&mut body).unwrap()).unwrap()
    };

    let mut verify_cases = vec![json!({
        "name": "active-key",
        "receipt": signed,
        "expected": {
            "valid": true,
            "keyId": "test-receipt-1",
            "canonicalBody": canonical_body,
            "bodySha256": body_sha256,
            "signingMessage": String::from_utf8(receipt_signing_message("test-receipt-1", &body_sha256)).unwrap()
        }
    })];

    let mut before = signed.clone();
    before["issuedAt"] = json!(RETIRED_AT - 1);
    sign(&mut before, &[("test-receipt-0", &retired)]);
    verify_cases.push(json!({ "name": "retired-key-before-retirement", "receipt": checked(before), "expected": { "valid": true, "keyId": "test-receipt-0" } }));

    let mut after = signed.clone();
    sign(&mut after, &[("test-receipt-0", &retired)]);
    verify_cases.push(json!({ "name": "retired-key-after-retirement", "receipt": checked(after), "expected": { "valid": false, "error": "key-retired" } }));

    let mut unknown = signed.clone();
    sign(&mut unknown, &[("test-receipt-9", &active)]);
    verify_cases.push(json!({ "name": "unknown-key", "receipt": checked(unknown), "expected": { "valid": false, "error": "unknown-key" } }));

    let mut tampered = signed.clone();
    tampered["slot"] = json!("312000124");
    verify_cases.push(json!({ "name": "tampered-body", "receipt": checked(tampered), "expected": { "valid": false, "error": "bad-signature" } }));

    let mut wrong_key = signed.clone();
    sign(&mut wrong_key, &[("test-receipt-1", &retired)]);
    verify_cases.push(json!({ "name": "signed-by-another-key", "receipt": checked(wrong_key), "expected": { "valid": false, "error": "bad-signature" } }));

    let mut rotated = signed.clone();
    sign(
        &mut rotated,
        &[("test-receipt-0", &retired), ("test-receipt-1", &active)],
    );
    verify_cases.push(json!({ "name": "any-usable-signature", "receipt": checked(rotated), "expected": { "valid": true, "keyId": "test-receipt-1" } }));

    let mut wrong_schema = signed.clone();
    wrong_schema["schema"] = json!("arete.protocol-execution-receipt/v2");
    verify_cases.push(json!({ "name": "unknown-schema", "receipt": wrong_schema, "expected": { "valid": false, "error": "invalid-receipt" } }));

    let mut unsigned = signed.clone();
    unsigned["signatures"] = json!([]);
    verify_cases.push(json!({ "name": "no-signatures", "receipt": unsigned, "expected": { "valid": false, "error": "invalid-receipt" } }));

    // --- On-chain cases ---------------------------------------------------
    let wire = |built: &Built| base64(&built.wire);
    let rpc = |built: &Built, slot: u64, err: Value| json!({ "slot": slot, "transaction": [wire(built), "base64"], "meta": { "err": err } });
    let mut on_chain = Vec::new();
    let mut case = |name: &str, receipt: Value, transaction: Value, error: Option<&str>| {
        let expected = match error {
            None => json!({ "valid": true }),
            Some(error) => json!({ "valid": false, "error": error }),
        };
        on_chain.push(json!({ "name": name, "receipt": receipt, "transaction": transaction, "expected": expected }));
    };

    case("legacy-wire", signed.clone(), json!(wire(&legacy)), None);
    let v0 = transaction(
        &attested(memo_text("contribute", None, 1, 1)),
        Format::V0,
        &both,
    );
    case(
        "v0-wire",
        base_receipt(&v0, json!({ "index": 0, "count": 1 }), None),
        json!(wire(&v0)),
        None,
    );
    let v1 = transaction(
        &attested(memo_text("contribute", Some(&participant), 2, 2)),
        Format::V1,
        &both,
    );
    let v1_receipt = base_receipt(&v1, json!({ "index": 1, "count": 2 }), Some(PARTICIPANT_ID));
    case(
        "v1-wire-participant",
        v1_receipt.clone(),
        json!(wire(&v1)),
        None,
    );
    case(
        "rpc-response",
        signed.clone(),
        rpc(&legacy, 312_000_123, Value::Null),
        None,
    );
    case(
        "rpc-transaction-failed",
        signed.clone(),
        rpc(
            &legacy,
            312_000_123,
            json!({ "InstructionError": [0, { "Custom": 1 }] }),
        ),
        Some("transaction-failed"),
    );
    case(
        "rpc-slot-mismatch",
        signed.clone(),
        rpc(&legacy, 312_000_124, Value::Null),
        Some("slot-mismatch"),
    );
    case(
        "rpc-json-encoding",
        signed.clone(),
        json!({ "slot": 312_000_123, "transaction": { "signatures": [legacy.signature], "message": {} }, "meta": { "err": null } }),
        Some("invalid-transaction"),
    );
    case(
        "not-a-transaction",
        signed.clone(),
        json!("AQID"),
        Some("invalid-transaction"),
    );

    let mut other = signed.clone();
    other["signature"] = json!(v0.signature);
    case(
        "signature-mismatch",
        other,
        json!(wire(&legacy)),
        Some("signature-mismatch"),
    );
    let mut other = signed.clone();
    other["messageSha256"] = json!("0".repeat(64));
    case(
        "message-hash-mismatch",
        other,
        json!(wire(&legacy)),
        Some("message-hash-mismatch"),
    );
    for (name, path, value) in [
        (
            "memo-other-deployment",
            "/deployment/shortId",
            json!("raise2"),
        ),
        ("memo-other-step", "/step", json!("refund")),
        (
            "memo-other-instance",
            "/instanceId",
            json!("2d4f6a8c-0e1f-4a2b-9c3d-5e6f7a8b9c0e"),
        ),
        (
            "memo-other-participant",
            "/participantId",
            json!(INSTANCE_ID),
        ),
        (
            "memo-other-position",
            "/transaction",
            json!({ "index": 0, "count": 2 }),
        ),
    ] {
        let mut other = v1_receipt.clone();
        *other.pointer_mut(path).unwrap() = value;
        case(name, other, json!(wire(&v1)), Some("memo-mismatch"));
    }
    let mut no_participant = v1_receipt.clone();
    no_participant
        .as_object_mut()
        .unwrap()
        .remove("participantId");
    case(
        "memo-participant-missing-from-receipt",
        no_participant,
        json!(wire(&v1)),
        Some("memo-mismatch"),
    );

    // The attestation key is listed by the memo but is not a signer.
    let unsigned_attestation = Message {
        header: (1, 0, 2),
        keys: vec![wallet_key, attestation_key, memo_program],
        instructions: vec![Ix {
            program: 2,
            accounts: vec![1],
            data: memo_text("contribute", None, 1, 1).into_bytes(),
        }],
    };
    let built = transaction(&unsigned_attestation, Format::Legacy, &[Some(&wallet)]);
    case(
        "attestation-not-signer",
        base_receipt(&built, json!({ "index": 0, "count": 1 }), None),
        json!(wire(&built)),
        Some("attestation-not-signer"),
    );
    // The attestation key signs, but no memo lists it.
    let no_memo = Message {
        header: (2, 1, 1),
        keys: vec![wallet_key, attestation_key, SYSTEM_PROGRAM],
        instructions: vec![Ix {
            program: 2,
            accounts: vec![0, 1],
            data: vec![],
        }],
    };
    let built = transaction(&no_memo, Format::Legacy, &both);
    case(
        "memo-missing",
        base_receipt(&built, json!({ "index": 0, "count": 1 }), None),
        json!(wire(&built)),
        Some("memo-missing"),
    );
    let mut twice = attested(memo_text("contribute", None, 1, 1));
    twice.instructions.push(Ix {
        program: 2,
        accounts: vec![1],
        data: memo_text("contribute", None, 1, 1).into_bytes(),
    });
    let built = transaction(&twice, Format::Legacy, &both);
    case(
        "memo-not-unique",
        base_receipt(&built, json!({ "index": 0, "count": 1 }), None),
        json!(wire(&built)),
        Some("memo-not-unique"),
    );
    let built = transaction(&attested("hello".into()), Format::Legacy, &both);
    case(
        "memo-not-attested",
        base_receipt(&built, json!({ "index": 0, "count": 1 }), None),
        json!(wire(&built)),
        Some("memo-invalid"),
    );
    // The memo also lists the wallet, so the attestation key is not its only account.
    let shared = Message {
        header: (2, 1, 1),
        keys: vec![wallet_key, attestation_key, memo_program],
        instructions: vec![Ix {
            program: 2,
            accounts: vec![1, 0],
            data: memo_text("contribute", None, 1, 1).into_bytes(),
        }],
    };
    let built = transaction(&shared, Format::Legacy, &both);
    case(
        "memo-extra-account",
        base_receipt(&built, json!({ "index": 0, "count": 1 }), None),
        json!(wire(&built)),
        Some("memo-invalid"),
    );
    let built = transaction(
        &attested(memo_text("contribute", None, 1, 1)),
        Format::Legacy,
        &[Some(&wallet), None],
    );
    case(
        "attestation-signature-missing",
        base_receipt(&built, json!({ "index": 0, "count": 1 }), None),
        json!(wire(&built)),
        Some("attestation-signature-invalid"),
    );

    let output = json!({
        "schema": "arete.protocol-client-vector/v1",
        "description": "Execution receipts. verifyReceipt(receipt, keySet): the body is the receipt without `signatures`, in RFC 8785 (JCS) form; each signature is base64 Ed25519 by `keyId` over the UTF-8 lines `arete.protocol-execution-receipt/v1`, keyId, lowercase hex SHA-256 of the body. A key with `retiredAt` verifies only receipts whose issuedAt is before it. A receipt is valid when any signature verifies; otherwise the error is the first of bad-signature (a usable key did not verify), key-retired, unknown-key; a malformed receipt is invalid-receipt. verifyReceiptOnChain(receipt, transaction): the transaction is base64 wire bytes or a getTransaction result with encoding base64. Checks in order: transaction-failed (meta.err), slot-mismatch, invalid-transaction, signature-mismatch (first signature), message-hash-mismatch (SHA-256 of the message bytes), attestation-not-signer, memo-missing / memo-not-unique / memo-invalid (exactly one memo instruction, through either SPL Memo program, lists the attestation key; it must be SPL Memo v2 (MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr), list only that key, and carry an `arete:p2:` memo with 8 colon-separated parts), memo-mismatch (short id, instance and participant as base58 of the UUID bytes, step, n/count = index+1/count), attestation-signature-invalid (Ed25519 over the message bytes).",
        "keySet": key_set,
        "verifyCases": verify_cases,
        "onChainCases": on_chain,
    });
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(OUTPUT);
    let mut text = serde_json::to_string_pretty(&output).unwrap();
    text.push('\n');
    fs::write(&path, text).unwrap();
    println!("wrote {}", path.display());
}
