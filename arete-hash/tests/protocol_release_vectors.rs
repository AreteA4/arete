//! The shared release-derivation vectors in `test-vectors/protocol-release/`.
//! The TypeScript `deriveServerRelease` and `buildRelease` wrote every
//! expected output; `derive_server_release` and `derive_release` must
//! reproduce each one exactly.

use std::fs;
use std::path::PathBuf;

use arete_hash::protocol::*;
use arete_hash::*;
use serde_json::Value;

fn vectors_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../test-vectors/protocol-release")
}

fn vectors() -> Value {
    serde_json::from_slice(&fs::read(vectors_dir().join("vectors.json")).unwrap()).unwrap()
}

fn spec_of(case: &Value) -> ProtocolSpecV1 {
    let path = vectors_dir().join(case["spec"].as_str().unwrap());
    let mut document = parse_contract_json(&fs::read(path).unwrap()).unwrap();
    if let Some(overrides) = case.get("specOverrides").and_then(Value::as_object) {
        let root = document.as_object_mut().unwrap();
        for (key, value) in overrides {
            root.insert(key.clone(), value.clone());
        }
    }
    validate_protocol_spec_value(&document).unwrap()
}

fn text<T: serde::Serialize>(value: &T) -> String {
    String::from_utf8(canonicalize_jcs(value).unwrap()).unwrap()
}

#[test]
fn the_server_compiler_identity_is_pinned() {
    let vectors = vectors();
    assert_eq!(vectors["schema"], "arete.protocol-release-vectors/v1");
    let pinned = &vectors["serverCompiler"];
    assert_eq!(pinned["name"], SERVER_COMPILER_NAME);
    assert_eq!(pinned["version"], SERVER_DERIVATION_VERSION);
    assert_eq!(
        server_compiler().compiler_hash().unwrap().to_string(),
        pinned["compilerHash"].as_str().unwrap()
    );
}

#[test]
fn rust_derivation_matches_the_shared_vectors() {
    let vectors = vectors();
    let cases = vectors["cases"].as_array().unwrap();
    assert!(cases.len() >= 7, "expected the full vector set");
    let (mut outputs, mut errors) = (0, 0);
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let spec = spec_of(case);
        let bundle = case
            .get("bundleBase64")
            .map(|encoded| decode_base64(encoded.as_str().unwrap()).unwrap());
        match case["mode"].as_str().unwrap() {
            "server" => match (
                derive_server_release(&spec, bundle.as_deref()),
                case.get("expected"),
            ) {
                (Ok(derived), Some(expected)) => {
                    assert_eq!(
                        text(&derived.release),
                        expected["release"],
                        "{name}: release"
                    );
                    assert_eq!(
                        text(&derived.receipt),
                        expected["receipt"],
                        "{name}: receipt"
                    );
                    assert_eq!(
                        derived.release_hash.to_string(),
                        expected["releaseHash"],
                        "{name}"
                    );
                    assert_eq!(
                        derived.stack_source_hash.to_string(),
                        expected["stackSourceHash"],
                        "{name}"
                    );
                    // Every derived document is a valid contract.
                    validate_protocol_release_value(
                        &serde_json::to_value(&derived.release).unwrap(),
                    )
                    .unwrap();
                    validate_protocol_build_receipt_value(
                        &serde_json::to_value(&derived.receipt).unwrap(),
                    )
                    .unwrap();
                    validate_protocol_stack_source_value(
                        &serde_json::to_value(&derived.stack_source).unwrap(),
                    )
                    .unwrap();
                    assert_eq!(derived.stack_source.generator, server_stack_generator());
                    // Deterministic.
                    assert_eq!(
                        derive_server_release(&spec, bundle.as_deref()).unwrap(),
                        derived
                    );
                    outputs += 1;
                }
                (Err(error), None) => {
                    let message = match &error {
                        ReleaseDerivationError::Stack(stack) => stack.message.clone(),
                        other => other.to_string(),
                    };
                    assert_eq!(message, case["error"].as_str().unwrap(), "{name}");
                    errors += 1;
                }
                (result, _) => panic!("{name}: unexpected result {result:?}"),
            },
            "release" => {
                let compiler: BuildCompilerV1 =
                    serde_json::from_value(case["compiler"].clone()).unwrap();
                let options: StackSourceOptions =
                    serde_json::from_value(case["stackOptions"].clone()).unwrap();
                let stack_source = generate_stack_source(&spec, &options).unwrap();
                let release =
                    derive_release(&spec, bundle.as_deref(), &stack_source, &compiler).unwrap();
                let expected = &case["expected"];
                assert_eq!(text(&release), expected["release"], "{name}: release");
                assert_eq!(release.hash().unwrap().to_string(), expected["releaseHash"]);
                assert_eq!(
                    stack_source.hash().unwrap().to_string(),
                    expected["stackSourceHash"]
                );
                outputs += 1;
            }
            other => panic!("{name}: unknown mode {other}"),
        }
    }
    assert!(
        outputs >= 4 && errors >= 3,
        "{outputs} outputs, {errors} errors"
    );
}

#[test]
fn derivation_rejects_bundle_mismatches_before_generating() {
    let vectors = vectors();
    let token_raise = vectors["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["name"] == "token-raise.server")
        .unwrap();
    let spec = spec_of(token_raise);
    assert!(matches!(
        derive_server_release(&spec, Some(&[])),
        Err(ReleaseDerivationError::EmptyBundle)
    ));
    assert_eq!(
        derive_server_release(&spec, None).unwrap_err().code(),
        "protocol-bundle-mismatch"
    );
}

#[test]
fn unreviewed_is_a_tier_but_never_a_review() {
    assert_eq!(TrustTier::parse("unreviewed"), Some(TrustTier::Unreviewed));
    assert_eq!(TrustTier::Unreviewed.as_str(), "unreviewed");
    assert_eq!(TrustTier::ALL, ["reviewed", "unreviewed"]);
    let review = serde_json::json!({
        "schema": PROTOCOL_RELEASE_REVIEW_SCHEMA_V1,
        "releaseHash": "arete:h1:protocol-release:sha256:1111111111111111111111111111111111111111111111111111111111111111",
        "reviewKeyId": "test-review-1",
        "trustTier": "unreviewed",
        "signature": encode_base64(&[0u8; 64]),
    });
    let error = validate_protocol_release_review_value(&review).unwrap_err();
    assert_eq!(error.path, "/trustTier");
}
