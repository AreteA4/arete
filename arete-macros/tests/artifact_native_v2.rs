mod support;

use std::fs;

use arete_artifacts::{
    load_live_spec_v2, load_stack_manifest_v2, PortableKeyResolutionStrategy,
    PortableMappingSource, STACK_MANIFEST_SCHEMA_V2,
};
use support::{arete_dir, cargo_toml, escape_path, macro_manifest_dir, TempCrate};

fn dependencies() -> Vec<String> {
    vec![
        format!(
            "arete = {{ path = \"{}\", features = [\"full\"] }}",
            escape_path(&arete_dir())
        ),
        format!(
            "arete-macros = {{ path = \"{}\" }}",
            escape_path(&macro_manifest_dir())
        ),
        "serde = { version = \"1.0\", features = [\"derive\"] }".to_string(),
        "borsh = { version = \"1.5\", features = [\"derive\"] }".to_string(),
        "solana-pubkey = { version = \"2.2\", features = [\"serde\", \"borsh\"] }".to_string(),
    ]
}

#[test]
fn idl_only_macro_emits_program_and_zero_live_manifest_with_spec() {
    let idl_path = macro_manifest_dir()
        .parent()
        .unwrap()
        .join("arete-idl/tests/fixtures/pump.json");
    let source = format!(
        r#"use arete_macros::arete;

#[arete(idl = "{}")]
mod pump_program {{}}

fn main() {{
    let spec = pump_program::spec();
    assert_eq!(spec.program_runtime_definitions.len(), 1);
    assert!((spec.program_runtime_definitions[0].account_reader)("missing", &[]).is_err());
    let _server = arete::server::Server::builder().spec(spec).websocket();
}}
"#,
        escape_path(&idl_path)
    );
    let temp = TempCrate::new(
        "artifact-native-v2",
        "idl-only-artifacts",
        cargo_toml("idl-only-artifacts", &dependencies()),
        &source,
        &[],
    );
    let output = temp.cargo_check();
    assert!(
        output.status.success(),
        "IDL-only macro failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let artifact_dir = temp.path().join(".arete");
    let manifest_bytes = fs::read(artifact_dir.join("PumpProgram.stack-manifest.json")).unwrap();
    let manifest = load_stack_manifest_v2(&manifest_bytes).unwrap().artifact;
    assert_eq!(manifest.payload.schema, STACK_MANIFEST_SCHEMA_V2);
    assert_eq!(manifest.payload.programs.len(), 1);
    assert!(manifest.payload.live_specs.is_empty());
    assert!(manifest.payload.selected_views.is_empty());
    assert!(artifact_dir.join("pump.program-spec.json").exists());
    assert!(!artifact_dir.join("PumpProgram.live-spec.json").exists());
    assert!(!artifact_dir.join("PumpProgram.stack.json").exists());
}

#[test]
fn single_live_macro_emits_the_typed_default_view_golden() {
    let source = r#"use arete_macros::arete;

#[arete]
mod stream {
    #[entity(name = "Only")]
    struct Only {}
}

fn main() {
    let _ = stream::create_multi_entity_bytecode();
}
"#;
    let temp = TempCrate::new(
        "artifact-native-v2",
        "single-live-artifacts",
        cargo_toml("single-live-artifacts", &dependencies()),
        source,
        &[],
    );
    let output = temp.cargo_check();
    assert!(
        output.status.success(),
        "single-live macro failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let artifact_dir = temp.path().join(".arete");
    let live = load_live_spec_v2(&fs::read(artifact_dir.join("Stream.live-spec.json")).unwrap())
        .unwrap()
        .artifact;
    assert_eq!(live.payload.entities.len(), 1);
    assert_eq!(live.payload.entities[0].state_name, "Only");
    assert_eq!(live.payload.entities[0].views.len(), 1);
    assert_eq!(live.payload.entities[0].views[0].id, "Only/list");
    let manifest =
        load_stack_manifest_v2(&fs::read(artifact_dir.join("Stream.stack-manifest.json")).unwrap())
            .unwrap()
            .artifact;
    manifest
        .payload
        .validate_selected_views([("live", &live)])
        .unwrap();
}

#[test]
fn event_context_fields_can_populate_and_key_an_entity() {
    let idl_path = macro_manifest_dir()
        .parent()
        .unwrap()
        .join("arete-idl/tests/fixtures/pump.json");
    let source = format!(
        r#"use arete_macros::arete;

#[arete(idl = "{}")]
mod stream {{
    #[entity(name = "Trade")]
    struct Trade {{
        #[map(pump_sdk::events::TradeEvent::__signature, primary_key, strategy = SetOnce)]
        signature: String,

        #[map(pump_sdk::events::TradeEvent::__slot, strategy = SetOnce)]
        slot: u64,

        #[map(pump_sdk::events::TradeEvent::__timestamp, strategy = SetOnce)]
        timestamp: i64,

        #[map(pump_sdk::events::TradeEvent::mint, strategy = SetOnce)]
        mint: String,
    }}
}}

fn main() {{
    let _ = stream::create_multi_entity_bytecode();
}}
"#,
        escape_path(&idl_path)
    );
    let temp = TempCrate::new(
        "artifact-native-v2",
        "event-context-primary-key",
        cargo_toml("event-context-primary-key", &dependencies()),
        &source,
        &[],
    );
    let output = temp.cargo_check();
    assert!(
        output.status.success(),
        "event context macro failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let live =
        load_live_spec_v2(&fs::read(temp.path().join(".arete/Stream.live-spec.json")).unwrap())
            .unwrap()
            .artifact;
    let entity = live
        .payload
        .entities
        .iter()
        .find(|entity| entity.state_name == "Trade")
        .expect("Trade entity");
    assert_eq!(entity.identity.primary_keys, ["signature"]);
    assert_eq!(entity.handlers.len(), 1);

    let handler = &entity.handlers[0];
    match &handler.key_resolution {
        PortableKeyResolutionStrategy::Embedded { primary_field } => {
            assert_eq!(primary_field.segments, ["__update_context", "signature"]);
        }
        other => panic!("unexpected key resolution: {other:?}"),
    }

    for (target_path, context_field) in [
        ("signature", "signature"),
        ("slot", "slot"),
        ("timestamp", "timestamp"),
    ] {
        let mapping = handler
            .mappings
            .iter()
            .find(|mapping| mapping.target_path == target_path)
            .unwrap_or_else(|| panic!("missing mapping for {target_path}"));
        assert_eq!(
            mapping.source,
            PortableMappingSource::FromContext {
                field: context_field.to_string(),
            }
        );
    }
}

#[test]
fn multi_program_idl_macro_emits_ordered_program_specs_and_zero_live_manifest() {
    let fixture_dir = macro_manifest_dir()
        .parent()
        .unwrap()
        .join("arete-idl/tests/fixtures");
    let source = format!(
        r#"use arete_macros::arete;

#[arete(idl = ["{}", "{}"])]
mod programs {{}}

fn main() {{
    assert_eq!(programs::spec().program_runtime_definitions.len(), 2);
}}
"#,
        escape_path(&fixture_dir.join("pump.json")),
        escape_path(&fixture_dir.join("entropy.json")),
    );
    let temp = TempCrate::new(
        "artifact-native-v2",
        "multi-program-artifacts",
        cargo_toml("multi-program-artifacts", &dependencies()),
        &source,
        &[],
    );
    let output = temp.cargo_check();
    assert!(
        output.status.success(),
        "multi-program macro failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let artifact_dir = temp.path().join(".arete");
    let manifest = load_stack_manifest_v2(
        &fs::read(artifact_dir.join("Programs.stack-manifest.json")).unwrap(),
    )
    .unwrap()
    .artifact;
    assert_eq!(manifest.payload.programs.len(), 2);
    assert!(manifest.payload.live_specs.is_empty());
    assert!(artifact_dir.join("pump.program-spec.json").exists());
    assert!(artifact_dir.join("entropy.program-spec.json").exists());
}

#[test]
fn macro_live_output_is_typed_deterministic_and_private() {
    let source = r#"use arete_macros::arete;

#[arete]
mod stream {
    #[entity(name = "First")]
    struct First {}

    #[entity(name = "Second")]
    struct Second {}
}

fn main() {
    let _ = stream::create_multi_entity_bytecode();
}
"#;
    let temp = TempCrate::new(
        "artifact-native-v2",
        "multi-entity-artifacts",
        cargo_toml("multi-entity-artifacts", &dependencies()),
        source,
        &[],
    );
    let first = temp.cargo_check();
    assert!(
        first.status.success(),
        "multi-entity macro failed:\n{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let live_path = temp.path().join(".arete/Stream.live-spec.json");
    let first_bytes = fs::read(&live_path).unwrap();
    let live = load_live_spec_v2(&first_bytes).unwrap().artifact;
    assert_eq!(live.payload.entities.len(), 2);
    assert!(live.payload.program_adapters.is_empty());

    let second = temp.cargo_check();
    assert!(second.status.success());
    assert_eq!(first_bytes, fs::read(&live_path).unwrap());
    let public = String::from_utf8(first_bytes).unwrap();
    assert!(!public.contains("legacyProgramExtensions"));
    assert!(!public.contains("decoderBindingId"));
    assert!(!public.contains("platformParser"));
}

#[test]
fn authoritative_artifact_write_failure_is_a_compile_error() {
    let source = r#"use arete_macros::arete;

#[arete]
mod stream {
    #[entity(name = "Only")]
    struct Only {}
}

fn main() {}
"#;
    let temp = TempCrate::new(
        "artifact-native-v2",
        "fatal-artifact-write",
        cargo_toml("fatal-artifact-write", &dependencies()),
        source,
        &[(".arete/Stream.live-spec.json/blocker", "occupied")],
    );
    let output = temp.cargo_check();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Failed to write authoritative public stack artifacts"),
        "unexpected compiler error:\n{stderr}"
    );
}
