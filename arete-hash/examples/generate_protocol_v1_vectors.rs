//! Regenerate `test-vectors/protocol-v1.json`.
//!
//! ```bash
//! cargo run -p arete-hash --example generate_protocol_v1_vectors
//! ```
//!
//! Valid documents record their hash; invalid vectors apply a JSON patch to a
//! valid document and record the code and path Rust reports. Every expected
//! code below is asserted here, so a change in validation order fails loudly.

use std::fs;
use std::path::PathBuf;

use arete_hash::protocol::*;
use arete_hash::*;
use ed25519_dalek::{Signer, SigningKey};
use serde_json::{json, Value};

const OUTPUT: &str = "../test-vectors/protocol-v1.json";
const FIXTURES: &str = "../test-vectors/protocol";
/// Deterministic test review key. Never use it outside vectors and tests.
const TEST_REVIEW_SEED: [u8; 32] = [7u8; 32];
/// Deterministic test receipt key. Never use it outside vectors and tests.
const TEST_RECEIPT_SEED: [u8; 32] = [11u8; 32];

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn fixture(name: &str) -> Value {
    let path = root().join(FIXTURES).join(name);
    let bytes = fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    parse_contract_json(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn h(kind: &str, fill: char) -> String {
    format!("arete:h1:{kind}:sha256:{}", fill.to_string().repeat(64))
}

struct Generator {
    documents: serde_json::Map<String, Value>,
    valid: Vec<Value>,
    invalid: Vec<Value>,
}

impl Generator {
    fn document(&mut self, name: &str, kind: &str, document: Value) -> Option<String> {
        let contract = ProtocolContractKind::parse(kind).expect("known kind");
        let hash = contract
            .validate(&document)
            .unwrap_or_else(|e| panic!("valid document {name} failed: {e}"));
        self.documents.insert(
            name.to_string(),
            json!({ "kind": kind, "document": document }),
        );
        self.valid
            .push(json!({ "name": name, "kind": kind, "document": name, "hash": hash }));
        hash
    }

    fn invalid(&mut self, name: &str, base: &str, patch: Value, code: &str, path: &str) {
        let entry = &self.documents[base];
        let kind = entry["kind"].as_str().unwrap().to_string();
        let mut document = entry["document"].clone();
        apply_patch(&mut document, &patch);
        let error = ProtocolContractKind::parse(&kind)
            .unwrap()
            .validate(&document)
            .expect_err(&format!("{name} must fail"));
        assert_eq!(
            (error.code, error.path.as_str()),
            (code, path),
            "{name}: {}",
            error.message
        );
        self.invalid.push(json!({
            "name": name, "kind": kind, "document": base, "patch": patch,
            "code": code, "path": path,
        }));
    }
}

fn pointer_parts(path: &str) -> Vec<String> {
    path.split('/')
        .skip(1)
        .map(|p| p.replace("~1", "/").replace("~0", "~"))
        .collect()
}

fn apply_patch(document: &mut Value, patch: &Value) {
    for op in patch.as_array().unwrap() {
        let parts = pointer_parts(op["path"].as_str().unwrap());
        let (last, parents) = parts.split_last().unwrap();
        let mut target = &mut *document;
        for part in parents {
            target = match target {
                Value::Array(values) => &mut values[part.parse::<usize>().unwrap()],
                Value::Object(map) => map.get_mut(part).unwrap(),
                _ => panic!("bad patch path"),
            };
        }
        match (op["op"].as_str().unwrap(), target) {
            ("remove", Value::Object(map)) => {
                map.remove(last).expect("remove existing key");
            }
            ("remove", Value::Array(values)) => {
                values.remove(last.parse::<usize>().unwrap());
            }
            ("add" | "replace", Value::Object(map)) => {
                map.insert(last.clone(), op["value"].clone());
            }
            ("add", Value::Array(values)) if last == "-" => values.push(op["value"].clone()),
            ("add", Value::Array(values)) => {
                values.insert(last.parse().unwrap(), op["value"].clone())
            }
            ("replace", Value::Array(values)) => {
                values[last.parse::<usize>().unwrap()] = op["value"].clone()
            }
            _ => panic!("bad patch op"),
        }
    }
}

fn rep(path: &str, value: Value) -> Value {
    json!([{ "op": "replace", "path": path, "value": value }])
}

fn add(path: &str, value: Value) -> Value {
    json!([{ "op": "add", "path": path, "value": value }])
}

fn rem(path: &str) -> Value {
    json!([{ "op": "remove", "path": path }])
}

fn main() {
    let mut g = Generator {
        documents: serde_json::Map::new(),
        valid: Vec::new(),
        invalid: Vec::new(),
    };

    let raise = fixture("token-raise.protocol-spec.json");
    let raise_hash = g
        .document("tokenRaise", "protocol-spec", raise.clone())
        .unwrap();
    // A template instruction with trailing accounts no IDL names.
    let mut remaining = raise;
    remaining["steps"]["finalize"]["build"]["instructions"][0]["remainingAccounts"] = json!([
        { "pubkey": "instance.accounts.presale", "isWritable": true, "isSigner": false },
        { "pubkey": "params.quoteMint", "isWritable": false, "isSigner": false }
    ]);
    g.document("tokenRaiseRemainingAccounts", "protocol-spec", remaining);
    g.document(
        "escrowResolution",
        "protocol-spec",
        fixture("escrow-resolution.protocol-spec.json"),
    );

    let bundle = b"export async function createLaunch(input, ctx) { return { abi: \"arete.protocol-handler/v1\", transactions: [] }; }\n";
    let bundle_hash = hash_handler_bundle(bundle).unwrap().to_string();

    let stack_source = json!({
        "schema": "arete.protocol-stack-source/v1",
        "generator": { "name": "@usearete/protocol", "version": "0.1.0" },
        "profile": "interim",
        "stackSource": { "schema": "arete.stack-source/v1", "name": "TokenRaiseProtocol" }
    });
    let stack_source_hash = g
        .document(
            "tokenRaiseStackSource",
            "protocol-stack-source",
            stack_source,
        )
        .unwrap();

    let release = json!({
        "schema": "arete.protocol-release/v1",
        "protocol": "tokenRaise",
        "version": "1.0.0",
        "specHash": raise_hash,
        "handlerBundleHash": bundle_hash,
        "stackSourceHash": stack_source_hash,
        "compilerHash": h("compiler", 'e'),
        "dependencies": {
            "programs": [
                { "alias": "meteoraPresale", "programId": "presSVxnf9UU8jMxhgSMqaRwNiT36qeBdNeTRKjTdbj", "programSpecHash": h("program-spec", 'a') },
                { "alias": "squadsV4", "programId": "SQDS4ep65T869zMMBKyuUq6aD6EgTu8psMjkvj52pCf", "programSpecHash": h("program-spec", 'b') }
            ],
            "stacks": [ { "alias": "meteoraPresale", "stackManifestHash": h("stack-manifest", 'c') } ],
            "sdkExtensions": []
        }
    });
    let release_hash = g
        .document("tokenRaiseRelease", "protocol-release", release)
        .unwrap();

    let key = SigningKey::from_bytes(&TEST_REVIEW_SEED);
    let release_id: HashId<ProtocolRelease> = release_hash.parse().unwrap();
    let message = review_signing_message(&release_id, "test-review-1", TrustTier::Reviewed);
    let signature = encode_base64(&key.sign(&message).to_bytes());
    let review = json!({
        "schema": "arete.protocol-release-review/v1",
        "releaseHash": release_hash,
        "reviewKeyId": "test-review-1",
        "trustTier": "reviewed",
        "signature": signature,
    });
    let review_hash = g
        .document("tokenRaiseReview", "protocol-release-review", review)
        .unwrap();

    g.document(
        "tokenRaiseReceipt",
        "protocol-build-receipt",
        json!({
            "schema": "arete.protocol-build-receipt/v1",
            "sourceTreeSha256": "1".repeat(64),
            "lockfileSha256": "2".repeat(64),
            "dependencies": [
                { "name": "@usearete/protocol", "version": "0.1.0", "integrity": "sha512-test" },
                { "name": "esbuild", "version": "0.25.0", "integrity": "sha512-test" }
            ],
            "compiler": { "name": "@usearete/protocol", "version": "0.1.0", "esbuild": "0.25.0" },
            "outputs": {
                "specHash": raise_hash, "handlerBundleHash": bundle_hash,
                "stackSourceHash": stack_source_hash, "releaseHash": release_hash
            }
        }),
    );

    let resource_hash = g
        .document(
            "launchDefaults",
            "protocol-resource",
            json!({
                "schema": "arete.protocol-resource/v1",
                "name": "launchDefaults",
                "schemaSha256": "3".repeat(64),
                "value": { "tokenDecimals": 6 }
            }),
        )
        .unwrap();

    let snapshot_hash = g.document("contributeSnapshot", "provider-snapshot", json!({
        "schema": "arete.provider-snapshot/v1",
        "contextSlot": "312000000",
        "reads": [
            { "name": "escrow", "result": { "address": "SQDS4ep65T869zMMBKyuUq6aD6EgTu8psMjkvj52pCf", "exists": false, "slot": "312000000" } },
            { "name": "presale", "result": { "address": "presSVxnf9UU8jMxhgSMqaRwNiT36qeBdNeTRKjTdbj", "exists": true, "slot": "312000001", "lamports": "1461600", "owner": "presSVxnf9UU8jMxhgSMqaRwNiT36qeBdNeTRKjTdbj", "dataBase64": "AQID" } }
        ],
        "ctxCalls": [
            { "seq": 0, "method": "solana.account", "args": { "pubkey": "presSVxnf9UU8jMxhgSMqaRwNiT36qeBdNeTRKjTdbj" }, "resultSha256": "4".repeat(64) }
        ],
        "observedAt": 1760000000,
        "expiresAt": 1760000060
    })).unwrap();

    let contribute_intent = json!({
        "schema": "arete.execution-intent/v1",
        "executionId": "0b7d8f8e-5d0c-4a59-9a43-7a7f1d4e2c10",
        "deployment": { "deploymentId": "6f1c2b9a-1d3e-4f5a-8b7c-9d0e1f2a3b4c", "revision": 1, "releaseHash": release_hash },
        "step": "contribute",
        "instanceId": "2d4f6a8c-0e1f-4a2b-9c3d-5e6f7a8b9c0d",
        "participantId": null,
        "wallet": "SQDS4ep65T869zMMBKyuUq6aD6EgTu8psMjkvj52pCf",
        "role": null,
        "callerAccount": "acct_integrator_a",
        "sessionKeyClass": "publishable",
        "transactionFormat": "v1",
        "inputSha256": "5".repeat(64),
        "resourceHashes": [resource_hash],
        "providerSnapshotHash": snapshot_hash,
        "fees": {
            "asset": { "kind": "native" },
            "gross": "1000000000", "total": "10000000", "creator": "5000000", "platform": "5000000",
            "placement": "before",
            "platformDestination": "presSVxnf9UU8jMxhgSMqaRwNiT36qeBdNeTRKjTdbj",
            "creatorDestination": "SQDS4ep65T869zMMBKyuUq6aD6EgTu8psMjkvj52pCf"
        },
        "signers": [
            { "class": "wallet", "pubkey": "SQDS4ep65T869zMMBKyuUq6aD6EgTu8psMjkvj52pCf", "name": null },
            { "class": "attestation", "pubkey": "presSVxnf9UU8jMxhgSMqaRwNiT36qeBdNeTRKjTdbj", "name": null }
        ],
        "transactions": [
            { "index": 0, "messageSha256": "6".repeat(64), "signers": ["SQDS4ep65T869zMMBKyuUq6aD6EgTu8psMjkvj52pCf", "presSVxnf9UU8jMxhgSMqaRwNiT36qeBdNeTRKjTdbj"] }
        ],
        "blockhash": "presSVxnf9UU8jMxhgSMqaRwNiT36qeBdNeTRKjTdbj",
        "lastValidBlockHeight": "290000150",
        "expiresAt": 1760000060,
        "createdAt": 1760000000
    });
    let intent_hash = g
        .document(
            "contributeIntent",
            "execution-intent",
            contribute_intent.clone(),
        )
        .unwrap();
    // The fee adjustment is additive: the same intent with a cap applied
    // hashes differently, and without it hashes exactly as before.
    let mut capped = contribute_intent;
    capped["fees"]["feeAdjustment"] = json!({ "kind": "cap", "id": "cap_2026-10" });
    capped["fees"]["platform"] = json!("1000000");
    capped["fees"]["total"] = json!("6000000");
    g.document("contributeIntentCapped", "execution-intent", capped);

    let receipt_key = SigningKey::from_bytes(&TEST_RECEIPT_SEED);
    let sign_receipt = |mut receipt: Value| -> Value {
        let typed: ProtocolExecutionReceiptV1 = {
            let mut unsigned = receipt.clone();
            unsigned["signatures"] =
                json!([{ "keyId": "test-receipt-1", "signature": encode_base64(&[0u8; 64]) }]);
            validate_protocol_execution_receipt_value(&unsigned).expect("receipt shape")
        };
        let message = typed.signing_message("test-receipt-1").unwrap();
        receipt["signatures"] = json!([{
            "keyId": "test-receipt-1",
            "signature": encode_base64(&receipt_key.sign(&message).to_bytes()),
        }]);
        receipt
    };
    let reviewed_receipt = sign_receipt(json!({
        "schema": "arete.protocol-execution-receipt/v1",
        "signature": "5VERv8NMvzbJMEkV8xnrLkEaWRtSz9CosKDYjCJjBRnbJLgp8uirBgmQpjKhoR4tjF3ZpRzrFmBV6UjKdiSZkQUW",
        "slot": "312000123",
        "deployment": {
            "deploymentId": "6f1c2b9a-1d3e-4f5a-8b7c-9d0e1f2a3b4c",
            "shortId": "raise1",
            "revision": 1,
            "releaseHash": release_hash
        },
        "trustTier": "reviewed",
        "reviewHash": review_hash,
        "step": "contribute",
        "instanceId": "2d4f6a8c-0e1f-4a2b-9c3d-5e6f7a8b9c0d",
        "transaction": { "index": 0, "count": 1 },
        "messageSha256": "6".repeat(64),
        "intentHash": intent_hash,
        "attestationPubkey": "presSVxnf9UU8jMxhgSMqaRwNiT36qeBdNeTRKjTdbj",
        "fees": { "asset": { "kind": "native" }, "platform": "5000000", "creator": "5000000" },
        "status": "finalized",
        "issuedAt": 1760000100
    }));
    g.document(
        "contributeReceipt",
        "protocol-execution-receipt",
        reviewed_receipt,
    );
    g.document("unreviewedReceipt", "protocol-execution-receipt", sign_receipt(json!({
        "schema": "arete.protocol-execution-receipt/v1",
        "signature": "5VERv8NMvzbJMEkV8xnrLkEaWRtSz9CosKDYjCJjBRnbJLgp8uirBgmQpjKhoR4tjF3ZpRzrFmBV6UjKdiSZkQUW",
        "slot": "312000123",
        "deployment": {
            "deploymentId": "6f1c2b9a-1d3e-4f5a-8b7c-9d0e1f2a3b4c",
            "shortId": "raise1",
            "revision": 2,
            "releaseHash": release_hash
        },
        "trustTier": "unreviewed",
        "step": "contribute",
        "instanceId": "2d4f6a8c-0e1f-4a2b-9c3d-5e6f7a8b9c0d",
        "participantId": "0b7d8f8e-5d0c-4a59-9a43-7a7f1d4e2c10",
        "transaction": { "index": 1, "count": 2 },
        "messageSha256": "6".repeat(64),
        "intentHash": intent_hash,
        "attestationPubkey": "presSVxnf9UU8jMxhgSMqaRwNiT36qeBdNeTRKjTdbj",
        "status": "confirmed",
        "issuedAt": 1760000100
    })));
    g.document("receiptKeys", "protocol-receipt-keys", json!({
        "schema": "arete.protocol-receipt-keys/v1",
        "keys": [
            { "keyId": "test-receipt-0", "publicKey": "SQDS4ep65T869zMMBKyuUq6aD6EgTu8psMjkvj52pCf", "retiredAt": 1750000000 },
            { "keyId": "test-receipt-1", "publicKey": bs58_encode(receipt_key.verifying_key().as_bytes()) }
        ]
    }));

    g.document("depositOperation", "prepared-operation", json!({
        "abi": "arete.protocol-handler/v1",
        "transactions": [ { "name": "create", "instructions": [ {
            "programId": "presSVxnf9UU8jMxhgSMqaRwNiT36qeBdNeTRKjTdbj",
            "keys": [ { "pubkey": "SQDS4ep65T869zMMBKyuUq6aD6EgTu8psMjkvj52pCf", "isSigner": true, "isWritable": true } ],
            "dataBase64": "AQIDBA=="
        } ] } ]
    }));

    // --- Shape failures --------------------------------------------------
    g.invalid(
        "spec-unknown-field",
        "tokenRaise",
        add("/surprise", json!(1)),
        "unknown-field",
        "/surprise",
    );
    g.invalid(
        "spec-unknown-step-field",
        "tokenRaise",
        add("/steps/claim/repeatable", json!(true)),
        "unknown-field",
        "/steps/claim/repeatable",
    );
    g.invalid(
        "spec-missing-field",
        "tokenRaise",
        rem("/stack"),
        "missing-field",
        "/stack",
    );
    g.invalid(
        "spec-null-optional",
        "tokenRaise",
        rep("/description", Value::Null),
        "null-not-allowed",
        "/description",
    );
    g.invalid(
        "spec-wrong-schema",
        "tokenRaise",
        rep("/schema", json!("arete.protocol-spec/v2")),
        "invalid-schema-tag",
        "/schema",
    );
    g.invalid(
        "spec-float",
        "tokenRaise",
        rep("/metadata/instance/maxBytes", json!(1.5)),
        "float-not-allowed",
        "/metadata/instance/maxBytes",
    );
    g.invalid(
        "spec-bad-name",
        "tokenRaise",
        rep("/name", json!("Token-Raise")),
        "invalid-identifier",
        "/name",
    );
    g.invalid(
        "spec-bad-semver",
        "tokenRaise",
        rep("/version", json!("1.0")),
        "invalid-semver",
        "/version",
    );
    g.invalid(
        "spec-bad-pubkey",
        "tokenRaise",
        rep("/programs/squadsV4/programId", json!("not-a-key")),
        "invalid-pubkey",
        "/programs/squadsV4/programId",
    );
    g.invalid(
        "spec-bad-hash",
        "tokenRaise",
        rep(
            "/programs/squadsV4/programSpecHash",
            json!(h("live-spec", 'b')),
        ),
        "invalid-hash-id",
        "/programs/squadsV4/programSpecHash",
    );
    g.invalid(
        "spec-duplicate-program",
        "tokenRaise",
        add(
            "/programs/systemAgain",
            json!({ "kind": "builtin", "builtin": "system" }),
        ),
        "duplicate-entry",
        "/programs/systemAgain/builtin",
    );
    g.invalid(
        "spec-bad-visibility",
        "tokenRaise",
        rep("/instanceVisibility", json!("private")),
        "invalid-value",
        "/instanceVisibility",
    );
    g.invalid(
        "spec-bad-kind-tag",
        "tokenRaise",
        rep("/steps/claim/authority/kind", json!("anyone")),
        "invalid-value",
        "/steps/claim/authority/kind",
    );
    g.invalid(
        "spec-empty-expression",
        "tokenRaise",
        rep("/steps/claim/guards/0/expr", json!("  ")),
        "invalid-expression",
        "/steps/claim/guards/0/expr",
    );
    g.invalid(
        "spec-long-expression",
        "tokenRaise",
        rep("/steps/claim/guards/0/expr", json!("a".repeat(2049))),
        "string-too-long",
        "/steps/claim/guards/0/expr",
    );
    g.invalid(
        "spec-too-many-guards",
        "tokenRaise",
        rep(
            "/steps/claim/guards",
            json!((0..33)
                .map(|i| json!({"code": format!("g{i}"), "expr": "true"}))
                .collect::<Vec<_>>()),
        ),
        "array-too-large",
        "/steps/claim/guards",
    );
    g.invalid(
        "spec-schema-additional",
        "tokenRaise",
        rep("/params/additionalProperties", json!(true)),
        "invalid-schema",
        "/params/additionalProperties",
    );
    g.invalid(
        "spec-schema-format-on-int",
        "tokenRaise",
        add("/params/properties/deadline/minimum", json!(0)),
        "invalid-schema",
        "/params/properties/deadline/minimum",
    );
    g.invalid(
        "spec-schema-required-unknown",
        "tokenRaise",
        add("/params/required/-", json!("missing")),
        "invalid-schema",
        "/params/required/5",
    );
    g.invalid(
        "spec-duplicate-read-ref",
        "tokenRaise",
        add("/steps/contribute/reads/-", json!("presale")),
        "duplicate-entry",
        "/steps/contribute/reads/2",
    );
    g.invalid(
        "spec-unsafe-integer-bound",
        "tokenRaise",
        rep("/metadata/instance/maxBytes", json!(70000)),
        "integer-out-of-range",
        "/metadata/instance/maxBytes",
    );
    g.invalid(
        "spec-instance-metadata-participant-writer",
        "tokenRaise",
        rep(
            "/metadata/instance/fields/name/writer",
            json!("participant"),
        ),
        "invalid-value",
        "/metadata/instance/fields/name/writer",
    );

    // --- Semantic failures -----------------------------------------------
    g.invalid(
        "spec-handler-abi-unused",
        "escrowResolution",
        add("/handlerAbi", json!("arete.protocol-handler/v1")),
        "handler-abi-unused",
        "/handlerAbi",
    );
    g.invalid(
        "spec-handler-abi-required",
        "tokenRaise",
        rem("/handlerAbi"),
        "handler-abi-required",
        "/handlerAbi",
    );
    g.invalid(
        "spec-read-unknown-program",
        "tokenRaise",
        rep("/reads/presale/program", json!("jupiter")),
        "unknown-program",
        "/reads/presale/program",
    );
    g.invalid(
        "spec-read-builtin-program",
        "tokenRaise",
        rep("/reads/presale/program", json!("system")),
        "unknown-program",
        "/reads/presale/program",
    );
    g.invalid(
        "spec-unknown-initial",
        "tokenRaise",
        rep("/machines/instance/initial", json!("draft")),
        "unknown-state",
        "/machines/instance/initial",
    );
    g.invalid(
        "spec-derive-recorded",
        "tokenRaise",
        rep("/machines/instance/derive/0/to", json!("awaiting_approval")),
        "derive-targets-recorded-state",
        "/machines/instance/derive/0/to",
    );
    g.invalid(
        "spec-derived-without-rule",
        "tokenRaise",
        rem("/machines/instance/derive/3"),
        "derived-state-without-rule",
        "/machines/instance/states/finalized",
    );
    g.invalid(
        "spec-machine-unknown-read",
        "tokenRaise",
        add("/machines/instance/reads/-", json!("vault")),
        "unknown-read",
        "/machines/instance/reads/1",
    );
    g.invalid(
        "spec-missing-participant-machine",
        "tokenRaise",
        rem("/machines/participant"),
        "missing-participant-machine",
        "/steps/claim/scope",
    );
    g.invalid(
        "spec-two-create-steps",
        "escrowResolution",
        add(
            "/steps/reopen",
            g.documents["escrowResolution"]["document"]["steps"]["open"].clone(),
        ),
        "create-step-count",
        "/steps/reopen/scope",
    );
    g.invalid(
        "spec-creator-on-instance",
        "tokenRaise",
        rep("/steps/claim/authority", json!({ "kind": "creator" })),
        "invalid-authority",
        "/steps/claim/authority",
    );
    g.invalid(
        "spec-unknown-role",
        "escrowResolution",
        rep("/steps/release/authority/role", json!("judge")),
        "unknown-role",
        "/steps/release/authority/role",
    );
    g.invalid(
        "spec-missing-access",
        "tokenRaise",
        rem("/steps/claim/access"),
        "missing-access",
        "/steps/claim/access",
    );
    g.invalid(
        "spec-forbidden-access",
        "tokenRaise",
        add("/steps/finalize/access", json!("secret")),
        "forbidden-access",
        "/steps/finalize/access",
    );
    g.invalid(
        "spec-duplicate-guard-code",
        "tokenRaise",
        add(
            "/steps/contribute/guards/-",
            json!({ "code": "deadline_passed", "expr": "true" }),
        ),
        "duplicate-guard-code",
        "/steps/contribute/guards/4/code",
    );
    g.invalid(
        "spec-reserved-guard-code",
        "tokenRaise",
        rep(
            "/steps/contribute/guards/0/code",
            json!("instance_terminal"),
        ),
        "reserved-guard-code",
        "/steps/contribute/guards/0/code",
    );
    g.invalid(
        "spec-missing-role-guard",
        "escrowResolution",
        rem("/steps/release/guards/0"),
        "missing-role-guard",
        "/steps/release/guards",
    );
    g.invalid(
        "spec-step-unknown-read",
        "tokenRaise",
        rep("/steps/contribute/reads/0", json!("vault")),
        "unknown-read",
        "/steps/contribute/reads/0",
    );
    g.invalid(
        "spec-fee-unknown-arg",
        "tokenRaise",
        rep("/steps/contribute/fee/amountArg", json!("total")),
        "unknown-arg",
        "/steps/contribute/fee/amountArg",
    );
    g.invalid(
        "spec-fee-on-keeper",
        "tokenRaise",
        rep("/steps/finalize/fee", json!({ "kind": "fixed" })),
        "invalid-fee-basis",
        "/steps/finalize/fee",
    );
    g.invalid(
        "spec-create-transition",
        "tokenRaise",
        add(
            "/steps/create/transitions/-",
            json!({ "machine": "instance", "from": [], "to": "open" }),
        ),
        "create-step-transition",
        "/steps/create/transitions/0",
    );
    g.invalid(
        "spec-transition-derived",
        "tokenRaise",
        rep("/steps/finalize/transitions/0/to", json!("finalized")),
        "transition-targets-derived-state",
        "/steps/finalize/transitions/0/to",
    );
    g.invalid(
        "spec-transition-unknown-state",
        "tokenRaise",
        rep("/steps/claim/transitions/0/to", json!("withdrawn")),
        "unknown-state",
        "/steps/claim/transitions/0/to",
    );
    g.invalid(
        "spec-bind-outside-create",
        "tokenRaise",
        add("/steps/claim/bind", json!({ "accounts": {}, "roles": {} })),
        "bind-outside-create",
        "/steps/claim/bind",
    );
    g.invalid(
        "spec-role-not-bound",
        "escrowResolution",
        rem("/steps/open/bind/roles/resolver"),
        "role-not-bound",
        "/steps/open/bind/roles",
    );
    g.invalid(
        "spec-template-unknown-program",
        "tokenRaise",
        rep(
            "/steps/claim/build/instructions/0/program",
            json!("jupiter"),
        ),
        "unknown-program",
        "/steps/claim/build/instructions/0/program",
    );
    let remaining_path = "/steps/finalize/build/instructions/0/remainingAccounts";
    g.invalid(
        "spec-remaining-account-signer",
        "tokenRaiseRemainingAccounts",
        rep(&format!("{remaining_path}/1/isSigner"), json!(true)),
        "remaining-account-signer",
        &format!("{remaining_path}/1/isSigner"),
    );
    g.invalid(
        "spec-remaining-accounts-empty",
        "tokenRaiseRemainingAccounts",
        rep(remaining_path, json!([])),
        "invalid-value",
        remaining_path,
    );
    g.invalid(
        "spec-remaining-account-missing-signer",
        "tokenRaiseRemainingAccounts",
        rem(&format!("{remaining_path}/0/isSigner")),
        "missing-field",
        &format!("{remaining_path}/0/isSigner"),
    );
    g.invalid(
        "spec-remaining-account-unknown-field",
        "tokenRaiseRemainingAccounts",
        add(&format!("{remaining_path}/0/name"), json!("quoteVault")),
        "unknown-field",
        &format!("{remaining_path}/0/name"),
    );
    g.invalid(
        "spec-remaining-account-expression",
        "tokenRaiseRemainingAccounts",
        rep(&format!("{remaining_path}/0/pubkey"), json!("")),
        "invalid-expression",
        &format!("{remaining_path}/0/pubkey"),
    );
    g.invalid(
        "spec-stack-unbound-account",
        "tokenRaise",
        add(
            "/stack/instanceAccounts/escrow",
            json!({ "program": "meteoraPresale", "account": "Escrow" }),
        ),
        "unknown-bound-account",
        "/stack/instanceAccounts/escrow",
    );
    g.invalid(
        "spec-stack-unknown-step",
        "tokenRaise",
        rep("/stack/activity/0/step", json!("deposit")),
        "unknown-step",
        "/stack/activity/0/step",
    );
    g.invalid(
        "spec-unreachable-state",
        "escrowResolution",
        rem("/steps/refund/transitions/0"),
        "unreachable-state",
        "/machines/instance/states/refunded",
    );
    g.invalid(
        "spec-dead-end-state",
        "tokenRaise",
        rep("/machines/instance/states/finalized/terminal", json!(false)),
        "dead-end-state",
        "/machines/instance/states/finalized",
    );
    g.invalid(
        "spec-terminal-exit",
        "tokenRaise",
        rep("/machines/instance/states/closed/terminal", json!(true)),
        "terminal-state-exit",
        "/machines/instance/states/closed",
    );
    g.invalid(
        "spec-duplicate-reaction",
        "tokenRaise",
        rep("/reactions/1/id", json!("proposeFinalize")),
        "duplicate-entry",
        "/reactions/1/id",
    );
    g.invalid(
        "spec-reaction-unknown-step",
        "tokenRaise",
        rep("/reactions/3/trigger/step", json!("deposit")),
        "unknown-step",
        "/reactions/3/trigger/step",
    );
    g.invalid(
        "spec-reaction-mismatch",
        "tokenRaise",
        rep(
            "/reactions/0/action",
            json!({ "kind": "runKeeperStep", "step": "finalize" }),
        ),
        "reaction-action-mismatch",
        "/reactions/0/action/step",
    );
    g.invalid(
        "spec-unknown-event",
        "tokenRaise",
        rep("/reactions/1/action/event", json!("raise.done")),
        "unknown-event",
        "/reactions/1/action/event",
    );
    g.invalid(
        "spec-event-machine-mismatch",
        "tokenRaise",
        rep(
            "/reactions/3/trigger",
            json!({ "kind": "onTransition", "machine": "instance", "from": [], "to": ["failed"] }),
        ),
        "event-machine-mismatch",
        "/reactions/3/action/event",
    );
    g.invalid(
        "spec-event-not-emitted",
        "tokenRaise",
        add("/events/raise.closed", json!({ "machine": "instance" })),
        "event-not-emitted",
        "/events/raise.closed",
    );

    // --- Other contracts ---------------------------------------------------
    g.invalid(
        "release-unsorted",
        "tokenRaiseRelease",
        json!([{ "op": "replace", "path": "/dependencies/programs", "value": [
        g.documents["tokenRaiseRelease"]["document"]["dependencies"]["programs"][1].clone(),
        g.documents["tokenRaiseRelease"]["document"]["dependencies"]["programs"][0].clone()
    ] }]),
        "unsorted-entries",
        "/dependencies/programs/1",
    );
    g.invalid(
        "release-bundle-missing",
        "tokenRaiseRelease",
        rem("/handlerBundleHash"),
        "missing-field",
        "/handlerBundleHash",
    );
    g.invalid(
        "review-bad-signature",
        "tokenRaiseReview",
        rep("/signature", json!("AAAA")),
        "invalid-signature",
        "/signature",
    );
    g.invalid(
        "review-bad-tier",
        "tokenRaiseReview",
        rep("/trustTier", json!("audited")),
        "invalid-value",
        "/trustTier",
    );
    g.invalid(
        "review-bad-key-id",
        "tokenRaiseReview",
        rep("/reviewKeyId", json!("Test_Key")),
        "invalid-identifier",
        "/reviewKeyId",
    );
    g.invalid(
        "snapshot-missing-data",
        "contributeSnapshot",
        rem("/reads/1/result/owner"),
        "missing-field",
        "/reads/1/result/owner",
    );
    g.invalid(
        "snapshot-seq-gap",
        "contributeSnapshot",
        rep("/ctxCalls/0/seq", json!(1)),
        "invalid-value",
        "/ctxCalls/0/seq",
    );
    g.invalid(
        "snapshot-unknown-method",
        "contributeSnapshot",
        rep("/ctxCalls/0/method", json!("http.fetch")),
        "invalid-value",
        "/ctxCalls/0/method",
    );
    g.invalid(
        "intent-bad-decimal",
        "contributeIntent",
        rep("/fees/gross", json!("01000")),
        "invalid-decimal",
        "/fees/gross",
    );
    g.invalid(
        "intent-fee-sum",
        "contributeIntent",
        rep("/fees/platform", json!("4999999")),
        "invalid-value",
        "/fees/platform",
    );
    g.invalid(
        "intent-overflow-decimal",
        "contributeIntent",
        rep("/lastValidBlockHeight", json!("18446744073709551616")),
        "invalid-decimal",
        "/lastValidBlockHeight",
    );
    g.invalid(
        "intent-bad-format",
        "contributeIntent",
        rep("/transactionFormat", json!("legacy")),
        "invalid-value",
        "/transactionFormat",
    );
    g.invalid("intent-too-many-transactions", "contributeIntent", rep("/transactions", json!((0..9).map(|i| json!({ "index": i, "messageSha256": "6".repeat(64), "signers": ["SQDS4ep65T869zMMBKyuUq6aD6EgTu8psMjkvj52pCf"] })).collect::<Vec<_>>())), "array-too-large", "/transactions");
    g.invalid(
        "operation-signers-field",
        "depositOperation",
        add("/transactions/0/signers", json!([])),
        "unknown-field",
        "/transactions/0/signers",
    );
    g.invalid(
        "operation-empty",
        "depositOperation",
        rep("/transactions", json!([])),
        "invalid-value",
        "/transactions",
    );
    g.invalid(
        "operation-bad-data",
        "depositOperation",
        rep("/transactions/0/instructions/0/dataBase64", json!("AQID=")),
        "invalid-value",
        "/transactions/0/instructions/0/dataBase64",
    );

    // --- Fee adjustments ---------------------------------------------------
    g.invalid(
        "intent-fee-adjustment-kind",
        "contributeIntentCapped",
        rep("/fees/feeAdjustment/kind", json!("discount")),
        "invalid-value",
        "/fees/feeAdjustment/kind",
    );
    g.invalid(
        "intent-fee-adjustment-null",
        "contributeIntent",
        add("/fees/feeAdjustment", Value::Null),
        "null-not-allowed",
        "/fees/feeAdjustment",
    );
    g.invalid(
        "intent-fee-adjustment-id",
        "contributeIntentCapped",
        rep("/fees/feeAdjustment/id", json!("cap 1")),
        "invalid-identifier",
        "/fees/feeAdjustment/id",
    );
    g.invalid(
        "intent-fee-adjustment-unknown-field",
        "contributeIntentCapped",
        add("/fees/feeAdjustment/bps", json!(10)),
        "unknown-field",
        "/fees/feeAdjustment/bps",
    );

    // --- Receipts ----------------------------------------------------------
    g.invalid(
        "receipt-unknown-field",
        "contributeReceipt",
        add("/extra", json!(true)),
        "unknown-field",
        "/extra",
    );
    g.invalid(
        "receipt-bad-transaction-signature",
        "contributeReceipt",
        rep("/signature", json!("abc")),
        "invalid-signature",
        "/signature",
    );
    g.invalid(
        "receipt-reviewed-without-review",
        "contributeReceipt",
        rem("/reviewHash"),
        "missing-field",
        "/reviewHash",
    );
    g.invalid(
        "receipt-unreviewed-with-review",
        "unreviewedReceipt",
        add("/reviewHash", json!(h("protocol-release-review", 'd'))),
        "invalid-value",
        "/reviewHash",
    );
    g.invalid(
        "receipt-null-participant",
        "contributeReceipt",
        add("/participantId", Value::Null),
        "null-not-allowed",
        "/participantId",
    );
    g.invalid(
        "receipt-index-past-count",
        "contributeReceipt",
        rep("/transaction/index", json!(1)),
        "invalid-value",
        "/transaction/index",
    );
    g.invalid(
        "receipt-bad-short-id",
        "contributeReceipt",
        rep("/deployment/shortId", json!("raise:1")),
        "invalid-identifier",
        "/deployment/shortId",
    );
    g.invalid(
        "receipt-bad-status",
        "contributeReceipt",
        rep("/status", json!("processed")),
        "invalid-value",
        "/status",
    );
    g.invalid(
        "receipt-intent-hash-kind",
        "contributeReceipt",
        rep("/intentHash", json!(h("provider-snapshot", 'e'))),
        "invalid-hash-id",
        "/intentHash",
    );
    g.invalid(
        "receipt-no-signatures",
        "contributeReceipt",
        rep("/signatures", json!([])),
        "invalid-value",
        "/signatures",
    );
    g.invalid(
        "receipt-short-signature",
        "contributeReceipt",
        rep("/signatures/0/signature", json!("AQID")),
        "invalid-signature",
        "/signatures/0/signature",
    );
    g.invalid(
        "receipt-unsorted-signatures",
        "contributeReceipt",
        add(
            "/signatures/-",
            json!({ "keyId": "test-receipt-0", "signature": encode_base64(&[1u8; 64]) }),
        ),
        "unsorted-entries",
        "/signatures/1",
    );
    g.invalid(
        "receipt-keys-duplicate",
        "receiptKeys",
        add(
            "/keys/-",
            json!({ "keyId": "test-receipt-1", "publicKey": "SQDS4ep65T869zMMBKyuUq6aD6EgTu8psMjkvj52pCf" }),
        ),
        "duplicate-entry",
        "/keys/2",
    );
    g.invalid(
        "receipt-keys-bad-public-key",
        "receiptKeys",
        rep("/keys/0/publicKey", json!("not-a-key")),
        "invalid-pubkey",
        "/keys/0/publicKey",
    );

    let review_public_key = bs58_encode(key.verifying_key().as_bytes());
    let output = json!({
        "schema": "arete.protocol-test-vectors/v1",
        "errorCodes": PROTOCOL_ERROR_CODES,
        "documents": g.documents,
        "validVectors": g.valid,
        "invalidVectors": g.invalid,
        "bundleVectors": [ { "name": "createLaunchBundle", "bytesUtf8": String::from_utf8(bundle.to_vec()).unwrap(), "hash": bundle_hash } ],
        "reviewVectors": [ {
            "name": "tokenRaiseReview",
            "review": "tokenRaiseReview",
            "reviewKeyId": "test-review-1",
            "publicKeyBase58": review_public_key,
            "signingMessageUtf8": String::from_utf8(message).unwrap()
        } ],
        "rawJsonVectors": [
            { "name": "duplicate-key", "kind": "protocol-resource", "json": "{\"schema\":\"arete.protocol-resource/v1\",\"name\":\"a\",\"name\":\"b\",\"schemaSha256\":\"x\",\"value\":1}", "code": "duplicate-json-key", "path": "" },
            { "name": "unsafe-integer", "kind": "protocol-resource", "json": "{\"schema\":\"arete.protocol-resource/v1\",\"name\":\"a\",\"schemaSha256\":\"3333333333333333333333333333333333333333333333333333333333333333\",\"value\":9007199254740993}", "code": "unsafe-json-integer", "path": "" }
        ]
    });
    let path = root().join(OUTPUT);
    let mut text = serde_json::to_string_pretty(&output).unwrap();
    text.push('\n');
    fs::write(&path, text).unwrap();
    println!("wrote {}", path.display());
}

fn bs58_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    let mut digits: Vec<u8> = Vec::new();
    for byte in bytes {
        let mut carry = u32::from(*byte);
        for digit in digits.iter_mut() {
            carry += u32::from(*digit) << 8;
            *digit = (carry % 58) as u8;
            carry /= 58;
        }
        while carry > 0 {
            digits.push((carry % 58) as u8);
            carry /= 58;
        }
    }
    let zeros = bytes.iter().take_while(|b| **b == 0).count();
    std::iter::repeat_n('1', zeros)
        .chain(digits.iter().rev().map(|d| ALPHABET[*d as usize] as char))
        .collect()
}
