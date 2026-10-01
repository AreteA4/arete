mod support;
use support::{arete_dir, cargo_toml, escape_path, macro_manifest_dir, TempCrate};

#[test]
fn managed_solana_decodes_dynamic_tick_payload_and_rejects_extended_layout() {
    let root = macro_manifest_dir()
        .parent()
        .unwrap()
        .join("tests/fixtures/managed-solana-v1");
    let source = format!(
        r#"
use arete_macros::arete;
#[arete(idl = "{}")]
mod ticks {{}}
#[arete(idl = "{}")]
mod positions {{}}
fn main() {{
    use base64::Engine;
    let fixture: serde_json::Value = serde_json::from_str(include_str!("{}")).unwrap();
    let bytes = base64::engine::general_purpose::STANDARD.decode(fixture["data"].as_str().unwrap()).unwrap();
    let spec = ticks::spec();
    let read = &spec.program_runtime_definitions[0].account_reader;
    assert_eq!(read("TickFixture", &bytes).unwrap(), fixture["expected"]);
    let decoded = ticks::tick_fixture_sdk::accounts::TickFixture::try_from_bytes_exact(&bytes).unwrap();
    assert_eq!(decoded.to_json_value(), fixture["expected"]);
    let bad = base64::engine::general_purpose::STANDARD.decode(fixture["unknownVariantData"].as_str().unwrap()).unwrap();
    assert!(read("TickFixture", &bad).is_err());
    assert!(read("TickFixture", &bytes[..bytes.len()-1]).is_err());
    for case in fixture["enumPayloadCases"].as_array().unwrap() {{
        let bytes = base64::engine::general_purpose::STANDARD.decode(case["data"].as_str().unwrap()).unwrap();
        let value: ticks::tick_fixture_sdk::types::PayloadFixture = borsh::BorshDeserialize::try_from_slice(&bytes).unwrap();
        assert_eq!(value.to_json_value(), case["expected"]);
    }}
    let fixture: serde_json::Value = serde_json::from_str(include_str!("{}")).unwrap();
    let bytes = base64::engine::general_purpose::STANDARD.decode(fixture["data"].as_str().unwrap()).unwrap();
    let extended = base64::engine::general_purpose::STANDARD.decode(fixture["extendedData"].as_str().unwrap()).unwrap();
    let position = positions::position_fixture_sdk::accounts::PositionV2Fixture::try_from_bytes_exact(&bytes).unwrap();
    assert_eq!(position.liquidity_shares.len(), 70);
    let error = positions::position_fixture_sdk::accounts::PositionV2Fixture::try_from_bytes_exact(&extended).unwrap_err().to_string();
    assert!(error.contains("unsupported_layout"));
}}
"#,
        escape_path(&root.join("dynamic-tick.idl.json")),
        escape_path(&root.join("fixed-position.idl.json")),
        escape_path(&root.join("dynamic-tick-account.json")),
        escape_path(&root.join("fixed-position-account.json"))
    );
    let dependencies = vec![
        format!(
            "arete = {{ path = {:?}, features = [\"full\"] }}",
            arete_dir()
        ),
        format!("arete-macros = {{ path = {:?} }}", macro_manifest_dir()),
        "serde = { version = \"1\", features = [\"derive\"] }".into(),
        "serde_json = \"1\"".into(),
        "base64 = \"0.22\"".into(),
        "borsh = { version = \"1.5\", features = [\"derive\"] }".into(),
        "solana-pubkey = { version = \"2.3\", features = [\"serde\", \"borsh\"] }".into(),
    ];
    let temp = TempCrate::new(
        "managed-solana",
        "managed-decode",
        cargo_toml("managed-decode", &dependencies),
        &source,
        &[],
    );
    let output = temp.cargo_run();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
