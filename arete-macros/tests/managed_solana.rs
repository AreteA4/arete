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

#[test]
fn complete_orca_and_spl_managed_decoders_compile() {
    let root = macro_manifest_dir()
        .parent()
        .unwrap()
        .join("tests/fixtures/managed-solana-v1/production-idls");
    let source = format!(
        r#"
use arete_macros::arete;
#[arete(idl = "{}")]
mod orca {{}}
#[arete(idl = "{}")]
mod token {{}}
#[arete(idl = "ambiguous.idl.json")]
mod ambiguous {{}}
fn main() {{
    assert_eq!(orca::spec().program_runtime_definitions.len(), 1);
    assert_eq!(token::spec().program_runtime_definitions.len(), 1);
    use orca::whirlpool_sdk::types::{{DynamicTick, DynamicTickData}};
    let array = orca::whirlpool_sdk::accounts::DynamicTickArray {{
        start_tick_index: -88, whirlpool: solana_pubkey::Pubkey::default(), tick_bitmap: 1,
        ticks: std::array::from_fn(|index| if index == 0 {{
            DynamicTick::Initialized(DynamicTickData {{ liquidity_net: -(1i128 << 100), liquidity_gross: 1u128 << 100,
                fee_growth_outside_a: 0, fee_growth_outside_b: 0, reward_growths_outside: [0;3] }})
        }} else {{ DynamicTick::Uninitialized }})
    }};
    let json = array.to_json_value();
    #[derive(serde::Deserialize)]
    struct Ticks {{
        #[serde(with = "arete::runtime::serde_helpers::big_array")]
        ticks: [DynamicTick; 88]
    }}
    let restored: Ticks = serde_json::from_value(serde_json::json!({{"ticks": vec!["Uninitialized"; 88]}})).unwrap();
    assert!(matches!(restored.ticks[0], DynamicTick::Uninitialized));
    assert_eq!(json["ticks"][0]["Initialized"]["liquidity_gross"], (1u128 << 100).to_string());
    let idl: serde_json::Value = serde_json::from_str(include_str!("{}")).unwrap();
    let discriminator = idl["accounts"].as_array().unwrap().iter().find(|account| account["name"] == "DynamicTickArray").unwrap()["discriminator"].as_array().unwrap();
    let mut bytes: Vec<u8> = discriminator.iter().map(|byte| byte.as_u64().unwrap() as u8).collect();
    bytes.extend(borsh::to_vec(&array).unwrap());
    let spec = orca::spec();
    let read = &spec.program_runtime_definitions[0].account_reader;
    assert_eq!(read("DynamicTickArray", &bytes).unwrap(), json);
    use base64::Engine;
    let fixture: serde_json::Value = serde_json::from_str(include_str!("{}")).unwrap();
    let bytes = base64::engine::general_purpose::STANDARD.decode(fixture["data"].as_str().unwrap()).unwrap();
    let spec = token::spec();
    assert_eq!((spec.program_runtime_definitions[0].account_reader)("Account", &bytes).unwrap(), fixture["expected"]);
    assert_eq!(token::spl_token_sdk::accounts::Account::try_from_bytes_exact(&bytes).unwrap().to_json_value(), fixture["expected"]);
    assert_eq!(token::parsers::SplTokenState::try_unpack(&bytes).unwrap().to_value(), fixture["expected"]);
    let bytes = 42u64.to_le_bytes();
    assert!(ambiguous::parsers::UntaggedState::try_unpack(&bytes).unwrap_err().to_string().contains("Ambiguous"));
    let spec = ambiguous::spec();
    for account in ["First", "Second"] {{
        assert_eq!((spec.program_runtime_definitions[0].account_reader)(account, &bytes).unwrap()["value"], "42");
    }}
    assert!(ambiguous::parsers::UntaggedState::try_unpack_as("Unknown", &bytes).is_err());
    assert!(ambiguous::parsers::UntaggedState::try_unpack_as("First", &[0; 9]).is_err());
}}
"#,
        escape_path(&root.join("whirlpool.idl.json")),
        escape_path(&root.join("spl-token.idl.json")),
        escape_path(&root.join("whirlpool.idl.json")),
        escape_path(&root.join("spl-token-layout-account.json"))
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
        "production-decode",
        cargo_toml("production-decode", &dependencies),
        &source,
        &[(
            "ambiguous.idl.json",
            r#"{
            "name":"untagged", "address":"11111111111111111111111111111111",
            "instructions":[{"name":"noop","discriminator":[1],"accounts":[],"args":[]}],
            "accounts":[
                {"name":"First","docs":["arete.account_untagged=true"]},
                {"name":"Second","docs":["arete.account_untagged=true"]}
            ],
            "types":[
                {"name":"First","type":{"kind":"struct","fields":[{"name":"value","type":"u64"}]}},
                {"name":"Second","type":{"kind":"struct","fields":[{"name":"value","type":"u64"}]}}
            ]
        }"#,
        )],
    );
    let output = temp.cargo_run();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
