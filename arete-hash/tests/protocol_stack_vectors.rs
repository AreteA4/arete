//! The shared protocol stack vectors in `test-vectors/protocol-stack/`. The
//! TypeScript generator wrote every expected output; the Rust generator must
//! reproduce each one byte for byte.

use std::fs;
use std::path::PathBuf;

use arete_hash::protocol::*;
use arete_hash::*;
use serde_json::Value;

fn vectors_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../test-vectors/protocol-stack")
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
    validate_protocol_spec_value(&document)
        .unwrap_or_else(|error| panic!("{}: spec is invalid: {error}", case["name"]))
}

fn options_of(case: &Value, generator: &GeneratorV1) -> StackSourceOptions {
    let mut options: StackSourceOptions = serde_json::from_value(case["options"].clone())
        .unwrap_or_else(|error| panic!("{}: options: {error}", case["name"]));
    options.generator = Some(generator.clone());
    options
}

#[test]
fn rust_generator_matches_the_shared_vectors() {
    let vectors = vectors();
    assert_eq!(vectors["schema"], "arete.protocol-stack-vectors/v1");
    let generator: GeneratorV1 = serde_json::from_value(vectors["generator"].clone()).unwrap();
    let cases = vectors["cases"].as_array().unwrap();
    assert!(cases.len() >= 20, "expected the full vector set");
    let (mut outputs, mut errors) = (0, 0);
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let spec = spec_of(case);
        let result = generate_stack_source(&spec, &options_of(case, &generator));
        match (case.get("output"), case.get("error")) {
            (Some(output), None) => {
                let generated = result.unwrap_or_else(|error| panic!("{name}: {error}"));
                let expected = fs::read(vectors_dir().join(output.as_str().unwrap())).unwrap();
                let actual = canonicalize_jcs(&generated).unwrap();
                assert_eq!(
                    String::from_utf8(actual).unwrap(),
                    String::from_utf8(expected).unwrap(),
                    "{name}: canonical bytes differ"
                );
                assert_eq!(
                    generated.hash().unwrap().to_string(),
                    case["hash"].as_str().unwrap(),
                    "{name}: hash differs"
                );
                // The output is a valid contract that round-trips unchanged.
                let value = serde_json::to_value(&generated).unwrap();
                assert_eq!(
                    validate_protocol_stack_source_value(&value).unwrap(),
                    generated
                );
                outputs += 1;
            }
            (None, Some(error)) => {
                let actual = result.expect_err(name);
                assert_eq!(actual.message, error.as_str().unwrap(), "{name}");
                errors += 1;
            }
            _ => panic!("{name}: a case has exactly one of output and error"),
        }
    }
    assert!(outputs > 0 && errors > 0);
}

#[test]
fn generator_identity_defaults_to_this_crate() {
    let vectors = vectors();
    let case = &vectors["cases"][0];
    let spec = spec_of(case);
    let options: StackSourceOptions = serde_json::from_value(case["options"].clone()).unwrap();
    let generated = generate_stack_source(&spec, &options).unwrap();
    assert_eq!(generated.generator, default_stack_generator());
    assert_eq!(generated.generator.name, "arete-hash");
    assert_eq!(generated.generator.version, env!("CARGO_PKG_VERSION"));
    let value = serde_json::to_value(&generated).unwrap();
    validate_protocol_stack_source_value(&value).unwrap();
}
