use arete_interpreter::ast::*;
use arete_interpreter::{rust, typescript};
use std::collections::BTreeMap;
use std::{fs, path::PathBuf, process::Command};

#[test]
fn managed_liquidity_generated_program_models_compile_and_preserve_payloads() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf();
    let fixture_dir = root.join("tests/fixtures/managed-solana-v1");
    let mut idl: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(fixture_dir.join("dynamic-tick.idl.json")).unwrap(),
    )
    .unwrap();
    idl["instructions"] = serde_json::json!([{
        "name": "noop", "discriminator": [9,9,9,9,9,9,9,9], "accounts": [],
        "args": [{ "name": "amount", "type": "u64" }]
    }]);
    let parsed = arete_idl::parse::parse_idl_content(&idl.to_string()).unwrap();
    let spec = arete_interpreter::program_sdk::build_program_only_stack_spec_from_idl(
        &parsed,
        "ManagedLiquidity",
    );
    let config = rust::RustStackConfig::default();
    let output = rust::compile_program_modules(spec.clone(), Some(config.clone())).unwrap();
    assert!(output.types_rs.contains("pub enum DynamicTick"));
    assert!(output.types_rs.contains("Initialized(DynamicTickData)"));
    assert!(output
        .programs_rs
        .contains("AccountReader<crate::types::TickFixture>"));
    assert!(output.programs_rs.contains("pub fn noop"));
    let mut stack_spec = spec.clone();
    stack_spec.entities.push(SerializableStreamSpec {
        ast_version: arete_interpreter::ast::CURRENT_AST_VERSION.to_string(),
        state_name: "ManagedPosition".into(),
        program_id: None,
        idl: None,
        identity: IdentitySpec {
            primary_keys: vec!["id.address".into()],
            lookup_indexes: vec![],
        },
        handlers: vec![],
        sections: vec![EntitySection {
            name: "id".into(),
            fields: vec![FieldTypeInfo::new("address".into(), "String".into())],
            is_nested_struct: false,
            parent_field: None,
        }],
        field_mappings: BTreeMap::new(),
        resolver_hooks: vec![],
        instruction_hooks: vec![],
        resolver_specs: vec![],
        computed_fields: vec![],
        computed_field_specs: vec![],
        content_hash: None,
        views: vec![],
    });
    let mut legacy_entity = stack_spec.entities[0].clone();
    legacy_entity.idl = Some(spec.idls[0].clone());
    let legacy_ts = typescript::compile_serializable_spec(
        legacy_entity.clone(),
        "ManagedPosition".into(),
        None,
    )
    .unwrap();
    let mut legacy_stack_spec = stack_spec.clone();
    let mut other_entity = legacy_entity.clone();
    other_entity.state_name = "OtherPosition".into();
    legacy_stack_spec.entities = vec![legacy_entity, other_entity];
    let legacy_stack_ts = typescript::compile_stack_spec(legacy_stack_spec, None).unwrap();
    let collision = rust::compile_stack_spec(
        collision_spec(stack_spec.entities[0].clone()),
        Some(rust::RustStackConfig {
            module_mode: true,
            ..Default::default()
        }),
    )
    .unwrap();
    assert!(collision
        .programs_rs
        .as_ref()
        .unwrap()
        .contains("AccountReader<super::super::types::SecondPosition>"));
    let stack = rust::compile_stack_spec(stack_spec, Some(config)).unwrap();
    assert!(stack.types_rs.contains("pub struct TickFixture"));
    assert!(stack.entity_rs.contains("ManagedPositionEntityViews"));
    let ts = typescript::compile_stack_spec(spec, None).unwrap();
    let full_ts = ts.full_file();
    assert!(full_ts.contains("Initialized"));
    assert!(full_ts.contains("liquidityGross"));
    assert!(
        full_ts.contains("z.lazy"),
        "enum payload dependencies must not initialize before their schemas"
    );

    let dir = root.join("target/managed-liquidity-generated");
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(dir.join("generated.ts"), full_ts).unwrap();
    fs::write(dir.join("legacy-stack.ts"), legacy_stack_ts.full_file()).unwrap();
    fs::write(
        dir.join("legacy.ts"),
        format!("import {{ z }} from 'zod';\n{}", legacy_ts.interfaces),
    )
    .unwrap();
    fs::create_dir_all(dir.join("src/collision")).unwrap();
    fs::write(dir.join("src/collision/types.rs"), collision.types_rs).unwrap();
    fs::write(
        dir.join("src/collision/programs.rs"),
        collision.programs_rs.unwrap(),
    )
    .unwrap();
    fs::write(
        dir.join("Cargo.toml"),
        format!(
            r#"[package]
name = "managed-liquidity-generated"
version = "0.0.0"
edition = "2021"
[workspace]
[dependencies]
arete-sdk = {{ package = "arete-a4-sdk", path = {:?} }}
serde = {{ version = "1", features = ["derive"] }}
serde_json = "1"
tokio = {{ version = "1", features = ["macros", "rt-multi-thread"] }}
"#,
            root.join("rust/arete-a4-sdk")
        ),
    )
    .unwrap();
    fs::write(dir.join("src/types.rs"), stack.types_rs).unwrap();
    fs::write(dir.join("src/programs.rs"), stack.programs_rs.unwrap()).unwrap();
    fs::write(dir.join("src/standalone.rs"), output.programs_rs).unwrap();
    fs::write(dir.join("src/entity.rs"), stack.entity_rs).unwrap();
    fs::write(dir.join("src/main.rs"), format!(r#"
mod types;
pub mod programs;
pub mod standalone;
pub mod entity;
mod collision {{ pub mod types; pub mod programs; }}
#[tokio::main]
async fn main() {{
    let first: collision::types::Position = serde_json::from_value(serde_json::json!({{"amount": "9"}})).unwrap();
    let second: collision::types::SecondPosition = serde_json::from_value(serde_json::json!({{"mint": "second-mint"}})).unwrap();
    assert_eq!(first.amount, Some(9));
    assert_eq!(second.mint, "second-mint");
    fn distinct_readers(first: collision::programs::first::FirstProgram, second: collision::programs::second::SecondProgram) {{
        let _: arete_sdk::AccountReader<collision::types::Position> = first.position_accounts().unwrap();
        let _: arete_sdk::AccountReader<collision::types::SecondPosition> = second.position_accounts().unwrap();
    }}
    let _ = distinct_readers;
    let fixture: serde_json::Value = serde_json::from_str(include_str!({:?})).unwrap();
    let value: types::TickFixture = serde_json::from_value(fixture["expected"].clone()).unwrap();
    match value.tick {{
        types::DynamicTick::Initialized(data) => {{
            assert_eq!(data.liquidity_gross, 1u128 << 100);
            assert_eq!(data.liquidity_net, -(1i128 << 80));
            assert_eq!(data.reward_growths_outside.len(), 3);
        }}
        _ => panic!("payload lost"),
    }}
    for case in fixture["enumPayloadCases"].as_array().unwrap() {{
        let value: types::PayloadFixture = serde_json::from_value(case["expected"].clone()).unwrap();
        match value {{
            types::PayloadFixture::Empty => {{}},
            types::PayloadFixture::Named {{ exact_amount }} => assert_eq!(exact_amount, (1u64 << 53) + 1),
            types::PayloadFixture::Tuple(amount, signed) => {{
                assert_eq!(amount, (1u64 << 53) + 1);
                assert_eq!(signed, -((1i64 << 53) + 1));
            }},
            types::PayloadFixture::Large(amounts) => assert_eq!(amounts, vec![(1u64 << 53) + 1; 40]),
        }}
    }}
    fn managed_subscription_surface(client: &arete_sdk::Arete<entity::ManagedLiquidityStack>) {{
        let _updates = client.views.managed_position.list().watch().filter("owner", "fixture");
        let _state_updates = client.views.managed_position.state().watch("fixture");
    }}
    fn native_query_surface(reader: arete_sdk::AccountReader<types::TickFixture>) {{
        let query = arete_sdk::managed_solana::NativePositionQuery::default();
        let _query = reader.query_positions(&query);
    }}
    let instruction = standalone::tick_fixture::noop(standalone::tick_fixture::NoopParams {{ amount: 42 }}).unwrap();
    assert_eq!(&instruction.data[8..], &42u64.to_le_bytes());
    let _ = native_query_surface;

    // A generated reader must work through its managed HTTP contract with no RPC helper.
    use std::io::{{Read, Write}};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{{}}", listener.local_addr().unwrap());
    let response = serde_json::json!({{ "context": {{ "slot": "42" }}, "value": fixture["expected"] }}).to_string();
    let mock = std::thread::spawn(move || {{
        let (mut socket, _) = listener.accept().unwrap();
        socket.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
        let mut bytes = Vec::new();
        let mut buffer = [0u8; 4096];
        loop {{
            let count = socket.read(&mut buffer).unwrap();
            assert!(count > 0, "truncated request");
            bytes.extend_from_slice(&buffer[..count]);
            let request = String::from_utf8_lossy(&bytes);
            if let Some(end) = request.find("\r\n\r\n") {{
                let length: usize = request[..end].lines().find_map(|line| {{
                    let (key, value) = line.split_once(':')?;
                    key.eq_ignore_ascii_case("content-length").then(|| value.trim().parse().unwrap())
                }}).unwrap_or(0);
                if bytes.len() >= end + 4 + length {{ break; }}
            }}
        }}
        let request = String::from_utf8(bytes).unwrap();
        assert!(request.starts_with("POST /v1/releases/"));
        assert!(request.contains("/TickFixture/11111111111111111111111111111111/context "));
        assert!(request.contains("\"commitment\":\"finalized\""));
        assert!(request.contains("\"minContextSlot\":\"40\""));
        write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {{}}\r\nConnection: close\r\n\r\n{{}}", response.len(), response).unwrap();
    }});
    let client = arete_sdk::Arete::<entity::ManagedLiquidityStack>::builder()
        .transport(arete_sdk::Transport::Http).http_url(endpoint).connect().await.unwrap();
    managed_subscription_surface(&client);
    let reader = client.programs.tick_fixture.tick_fixture_accounts().unwrap();
    let read = reader.fetch_with_context("11111111111111111111111111111111", arete_sdk::managed_solana::ReadOptions {{
        commitment: Some(arete_sdk::managed_solana::Commitment::Finalized), min_context_slot: Some(40),
    }}).await.unwrap();
    assert_eq!(read.context.unwrap().slot, 42);
    assert!(matches!(read.value.unwrap().tick, types::DynamicTick::Initialized(_)));
    mock.join().unwrap();
    let bindings: arete_sdk::HostedSolanaGatewayBindings = serde_json::from_str(include_str!({:?})).unwrap();
    arete_sdk::create_hosted_solana_gateway_transports(&bindings, None, None).unwrap();
}}
"#, fixture_dir.join("dynamic-tick-account.json"), fixture_dir.join("binding-descriptors.json"))).unwrap();
    let result = Command::new("cargo")
        .args(["run", "--quiet"])
        .current_dir(&dir)
        .env("CARGO_TARGET_DIR", root.join("target"))
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "generated Rust program failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
}

// Only the first program has an entity mapping Position. The second program's
// account reader must still get its own incompatible IDL-only account model.
fn collision_spec(mut entity: SerializableStreamSpec) -> SerializableStackSpec {
    let make_program = |name: &str, address: &str, field: &str, ty: &str| {
        let idl = serde_json::json!({
            "name": name, "version": "0.1.0", "address": address,
            "instructions": [{"name": "noop", "discriminator": [0,0,0,0,0,0,0,0], "accounts": [], "args": [{"name":"amount", "type":"u64"}]}],
            "accounts": [{"name":"Position", "discriminator":[1,0,0,0,0,0,0,0]}],
            "types": [{"name":"Position", "type":{"kind":"struct", "fields":[{"name":field, "type":ty}]}}]
        });
        arete_interpreter::program_sdk::build_program_only_stack_spec_from_idl_bytes(
            idl.to_string().as_bytes(),
            Some(address),
            "Collisions",
        )
        .unwrap()
    };
    let mut first = make_program("first", "11111111111111111111111111111111", "amount", "u64");
    let second = make_program(
        "second",
        "So11111111111111111111111111111111111111112",
        "mint",
        "pubkey",
    );
    entity.program_id = Some(first.program_ids[0].clone());
    entity.idl = Some(first.idls[0].clone());
    let mut field = FieldTypeInfo::new("position".into(), "Position".into());
    field.base_type = BaseType::Object;
    field.resolved_type = Some(serde_json::from_value(serde_json::json!({
        "type_name":"Position", "is_account":true, "is_instruction":false, "is_event":false,
        "fields":[{"field_name":"amount", "field_type":"u64", "base_type":"Integer", "is_optional":false, "is_array":false}]
    })).unwrap());
    entity.sections.push(EntitySection {
        name: "position".into(),
        fields: vec![field],
        is_nested_struct: false,
        parent_field: None,
    });
    first.entities.push(entity);
    first.idls.extend(second.idls);
    first.program_ids.extend(second.program_ids);
    first.program_specs.extend(second.program_specs);
    first.instructions.extend(second.instructions);
    first.with_content_hash()
}
