use std::fs;
use std::path::PathBuf;

use arete_hash::protocol::*;
use arete_hash::*;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde_json::Value;

fn corpus() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../test-vectors/protocol-v1.json");
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

fn document(corpus: &Value, name: &str) -> Value {
    corpus["documents"][name]["document"].clone()
}

fn apply_patch(document: &mut Value, patch: &Value) {
    for op in patch.as_array().unwrap() {
        let parts: Vec<String> = op["path"]
            .as_str()
            .unwrap()
            .split('/')
            .skip(1)
            .map(|p| p.replace("~1", "/").replace("~0", "~"))
            .collect();
        let (last, parents) = parts.split_last().unwrap();
        let mut target = &mut *document;
        for part in parents {
            target = match target {
                Value::Array(values) => &mut values[part.parse::<usize>().unwrap()],
                Value::Object(map) => map.get_mut(part).unwrap(),
                _ => panic!("bad path"),
            };
        }
        match (op["op"].as_str().unwrap(), target) {
            ("remove", Value::Object(map)) => {
                map.remove(last).unwrap();
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
            _ => panic!("bad op"),
        }
    }
}

#[test]
fn valid_documents_hash_identically() {
    let corpus = corpus();
    for vector in corpus["validVectors"].as_array().unwrap() {
        let name = vector["name"].as_str().unwrap();
        let kind = ProtocolContractKind::parse(vector["kind"].as_str().unwrap()).unwrap();
        let doc = document(&corpus, vector["document"].as_str().unwrap());
        let hash = kind
            .validate(&doc)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(hash.as_deref(), vector["hash"].as_str(), "{name}");
    }
}

#[test]
fn invalid_documents_fail_with_stable_codes() {
    let corpus = corpus();
    let codes: Vec<&str> = corpus["errorCodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_str().unwrap())
        .collect();
    assert_eq!(codes, PROTOCOL_ERROR_CODES);
    for vector in corpus["invalidVectors"].as_array().unwrap() {
        let name = vector["name"].as_str().unwrap();
        let kind = ProtocolContractKind::parse(vector["kind"].as_str().unwrap()).unwrap();
        let mut doc = document(&corpus, vector["document"].as_str().unwrap());
        apply_patch(&mut doc, &vector["patch"]);
        let error = kind.validate(&doc).expect_err(name);
        assert_eq!(
            error.code,
            vector["code"].as_str().unwrap(),
            "{name}: {}",
            error.message
        );
        assert_eq!(error.path, vector["path"].as_str().unwrap(), "{name}");
        assert!(PROTOCOL_ERROR_CODES.contains(&error.code), "{name}");
    }
    for vector in corpus["rawJsonVectors"].as_array().unwrap() {
        let error = parse_contract_json(vector["json"].as_str().unwrap().as_bytes()).unwrap_err();
        assert_eq!(error.code, vector["code"].as_str().unwrap());
    }
}

/// A validated document re-serializes to the same canonical bytes it was
/// hashed from, so typed consumers can never hash a different document.
#[test]
fn validated_specs_round_trip_canonically() {
    let corpus = corpus();
    for name in ["tokenRaise", "escrowResolution"] {
        let doc = document(&corpus, name);
        let spec = validate_protocol_spec_value(&doc).unwrap();
        assert_eq!(
            canonicalize_jcs(&spec).unwrap(),
            canonicalize_json_value(&doc).unwrap(),
            "{name}"
        );
    }
    let intent = validate_execution_intent_value(&document(&corpus, "contributeIntent")).unwrap();
    assert_eq!(
        canonicalize_jcs(&intent).unwrap(),
        canonicalize_json_value(&document(&corpus, "contributeIntent")).unwrap()
    );
}

#[test]
fn review_signature_verifies_over_the_signing_message() {
    let corpus = corpus();
    let vector = &corpus["reviewVectors"][0];
    let review =
        validate_protocol_release_review_value(&document(&corpus, "tokenRaiseReview")).unwrap();
    let message = review.signing_message();
    assert_eq!(
        message,
        vector["signingMessageUtf8"].as_str().unwrap().as_bytes()
    );
    let key_bytes: [u8; 32] = bs58::decode(vector["publicKeyBase58"].as_str().unwrap())
        .into_vec()
        .unwrap()
        .try_into()
        .unwrap();
    let key = VerifyingKey::from_bytes(&key_bytes).unwrap();
    let signature = Signature::from_bytes(&review.signature_bytes());
    key.verify(&message, &signature).unwrap();
    let mut tampered = message.clone();
    tampered.push(b'x');
    assert!(key.verify(&tampered, &signature).is_err());
}

#[test]
fn bundle_hash_is_raw_bytes() {
    let corpus = corpus();
    let vector = &corpus["bundleVectors"][0];
    let hash = hash_handler_bundle(vector["bytesUtf8"].as_str().unwrap().as_bytes()).unwrap();
    assert_eq!(hash.to_string(), vector["hash"].as_str().unwrap());
}

#[test]
fn base64_is_strict() {
    assert_eq!(decode_base64("AQID").unwrap(), vec![1, 2, 3]);
    assert_eq!(decode_base64("AQ==").unwrap(), vec![1]);
    assert!(decode_base64("AR==").is_none(), "non-zero padding bits");
    assert!(decode_base64("AQ=").is_none());
    assert!(decode_base64("A=QI").is_none());
    for len in 0..70u8 {
        let bytes: Vec<u8> = (0..len).collect();
        assert_eq!(decode_base64(&encode_base64(&bytes)).unwrap(), bytes);
    }
}

#[test]
fn guards_may_not_use_the_access_and_lifecycle_codes() {
    let corpus = corpus();
    for code in [
        "step_requires_secret_key",
        "step_not_in_session_scope",
        "step_not_client_callable",
        "instance_terminal",
        "participant_terminal",
    ] {
        let mut doc = document(&corpus, "tokenRaise");
        doc["steps"]["contribute"]["guards"][0]["code"] = Value::String(code.into());
        let error = ProtocolContractKind::parse("protocol-spec")
            .unwrap()
            .validate(&doc)
            .unwrap_err();
        assert_eq!(error.code, "reserved-guard-code", "{code}: {error}");
        assert_eq!(error.path, "/steps/contribute/guards/0/code", "{code}");
    }
}

#[test]
fn receipt_signatures_verify_over_the_canonical_body() {
    let corpus = corpus();
    let keys = validate_protocol_receipt_key_set_value(&document(&corpus, "receiptKeys")).unwrap();
    for name in ["contributeReceipt", "unreviewedReceipt"] {
        let value = document(&corpus, name);
        let receipt = validate_protocol_execution_receipt_value(&value).unwrap();
        // The typed body and the served JSON body canonicalize identically.
        let mut served = value.clone();
        assert_eq!(
            receipt.canonical_body().unwrap(),
            receipt_body_bytes(&mut served).unwrap()
        );
        let signature = &receipt.signatures[0];
        let key = keys
            .usable_key(&signature.key_id, receipt.issued_at)
            .expect("active key");
        let key_bytes: [u8; 32] = bs58::decode(&key.public_key)
            .into_vec()
            .unwrap()
            .try_into()
            .unwrap();
        let key = VerifyingKey::from_bytes(&key_bytes).unwrap();
        let bytes: [u8; 64] = decode_base64(&signature.signature)
            .unwrap()
            .try_into()
            .unwrap();
        let message = receipt.signing_message(&signature.key_id).unwrap();
        assert!(message.starts_with(b"arete.protocol-execution-receipt/v1\ntest-receipt-1\n"));
        key.verify(&message, &Signature::from_bytes(&bytes))
            .unwrap();
        let mut tampered = receipt.clone();
        tampered.slot = "1".into();
        let tampered_message = tampered.signing_message(&signature.key_id).unwrap();
        assert!(key
            .verify(&tampered_message, &Signature::from_bytes(&bytes))
            .is_err());
    }
    // A retired key verifies only receipts issued before it retired.
    assert!(keys.usable_key("test-receipt-0", 1_749_999_999).is_some());
    assert!(keys.usable_key("test-receipt-0", 1_750_000_000).is_none());
    assert!(keys.usable_key("unknown", 0).is_none());
}

#[test]
fn fee_adjustment_is_additive() {
    let corpus = corpus();
    let plain = validate_execution_intent_value(&document(&corpus, "contributeIntent")).unwrap();
    assert_eq!(
        plain.fees.as_ref().unwrap().fee_adjustment,
        None,
        "the plain intent has no adjustment"
    );
    let capped =
        validate_execution_intent_value(&document(&corpus, "contributeIntentCapped")).unwrap();
    let adjustment = capped
        .fees
        .as_ref()
        .unwrap()
        .fee_adjustment
        .clone()
        .unwrap();
    assert_eq!(adjustment.kind, FeeAdjustmentKind::Cap);
    // Removing the adjustment and restoring the legs gives the original hash.
    let mut restored = capped.clone();
    let fees = restored.fees.as_mut().unwrap();
    fees.fee_adjustment = None;
    fees.platform = "5000000".into();
    fees.total = "10000000".into();
    assert_eq!(restored.hash().unwrap(), plain.hash().unwrap());
    assert_ne!(capped.hash().unwrap(), plain.hash().unwrap());
}

#[test]
fn direct_resource_values_reject_unsafe_integers_before_hashing() {
    use serde_json::json;
    let resource = |value| {
        json!({
            "schema": "arete.protocol-resource/v1", "name": "example",
            "schemaSha256": "a".repeat(64), "value": value
        })
    };
    for number in [
        json!(9_007_199_254_740_992u64),
        json!(9_007_199_254_740_993u64),
        json!(u64::MAX),
        json!(-9_007_199_254_740_992i64),
        json!(i64::MIN),
    ] {
        let value = resource(json!({"nested": [number]}));
        let direct = validate_protocol_resource_value(&value).unwrap_err();
        assert_eq!(direct.code, "unsafe-json-integer");
        assert_eq!(direct.path, "/value/nested/0");
        let bytes = serde_json::to_vec(&value).unwrap();
        assert_eq!(parse_contract_json(&bytes).unwrap_err().code, direct.code);
    }
    for number in [
        json!(9_007_199_254_740_991u64),
        json!(-9_007_199_254_740_991i64),
    ] {
        validate_protocol_resource_value(&resource(number))
            .unwrap()
            .hash()
            .unwrap();
    }
}

#[test]
fn builtin_activity_is_valid_but_builtin_account_decoding_is_not() {
    let mut value = document(&corpus(), "tokenRaise");
    value["programs"]["system"] = serde_json::json!({"kind":"builtin", "builtin":"system"});
    value["stack"] = serde_json::json!({
        "instanceAccounts": {}, "instanceFields": {},
        "activity": [{"step":"contribute", "program":"system", "instruction":"transfer", "instanceAccount":"destination", "walletAccount":"source", "amountArg":"lamports"}]
    });
    let spec = validate_protocol_spec_value(&value).unwrap();
    let mut options = StackSourceOptions::default();
    options.programs.insert(
        "system".into(),
        StackProgramSource::Idl {
            idl: "./idl/system.idl.json".into(),
        },
    );
    let generated = generate_stack_source(&spec, &options).unwrap();
    assert_eq!(
        generated.stack_source["programs"]["system"]["program_id"],
        "11111111111111111111111111111111"
    );
    value["stack"]["activity"][0]["program"] = "missing".into();
    assert_eq!(
        validate_protocol_spec_value(&value).unwrap_err().code,
        "unknown-program"
    );
    value["stack"]["activity"][0]["program"] = "system".into();
    value["reads"]["systemAccount"] = serde_json::json!({"kind":"account", "program":"system", "account":"Account", "address":"wallet", "optional": false});
    assert_eq!(
        validate_protocol_spec_value(&value).unwrap_err().code,
        "unknown-program"
    );
}
