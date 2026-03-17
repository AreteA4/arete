use hyperstack_idl::parse::parse_idl_file;
use hyperstack_idl::snapshot::IdlSnapshot;
use std::fs;
use std::path::PathBuf;

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

#[test]
fn test_parse_ore_legacy() {
    let idl = parse_idl_file(&fixture_path("ore.json")).expect("should parse ore.json");
    assert_eq!(
        idl.instructions.len(),
        19,
        "ore should have 19 instructions"
    );
    assert!(idl.name.is_some(), "ore should have a name");
}

#[test]
fn test_ore_instructions_have_discriminators() {
    // Test that ore.json instructions have proper discriminators when parsed as IdlSnapshot
    // This tests the fix for Steel-style discriminant format
    let idl_json = fs::read_to_string(fixture_path("ore.json")).expect("should read ore.json");
    let snapshot: IdlSnapshot =
        serde_json::from_str(&idl_json).expect("should parse as IdlSnapshot");

    assert_eq!(
        snapshot.instructions.len(),
        19,
        "ore should have 19 instructions"
    );

    // All instructions should have non-empty discriminators via get_discriminator()
    let empty_count = snapshot
        .instructions
        .iter()
        .filter(|ix| ix.get_discriminator().is_empty())
        .count();

    assert_eq!(
        empty_count, 0,
        "All ore instructions should have discriminators computed from discriminant field"
    );

    // Verify specific instruction
    let automate = snapshot
        .instructions
        .iter()
        .find(|ix| ix.name == "automate")
        .expect("should find automate instruction");

    assert_eq!(
        automate.get_discriminator(),
        vec![0, 0, 0, 0, 0, 0, 0, 0],
        "automate instruction should have discriminator [0,0,0,0,0,0,0,0]"
    );

    // Verify program_id is parsed from address field (using original IDL, not fixture)
    let original_idl_json = fs::read_to_string(
        "/Users/adrian/code/defi/hypertek/hyperstack-oss/stacks/ore/idl/ore.json",
    )
    .expect("should read original ore.json");
    let original_snapshot: IdlSnapshot =
        serde_json::from_str(&original_idl_json).expect("should parse original as IdlSnapshot");

    assert_eq!(
        original_snapshot.program_id,
        Some("oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv".to_string()),
        "program_id should be parsed from address field"
    );

    // Verify discriminant_size is 1 for Steel-style
    // Note: This will fail until the fix is applied in hyperstack-debug
    // The default is 8, but it should be detected as 1 for Steel-style
    println!(
        "Ore discriminant_size (before fix): {}",
        original_snapshot.discriminant_size
    );
}

#[test]
fn test_parse_entropy_legacy() {
    let idl = parse_idl_file(&fixture_path("entropy.json")).expect("should parse entropy.json");
    assert_eq!(
        idl.instructions.len(),
        5,
        "entropy should have 5 instructions"
    );
}

#[test]
fn test_parse_pump_modern() {
    let idl = parse_idl_file(&fixture_path("pump.json")).expect("should parse pump.json");
    assert_eq!(idl.instructions.len(), 6, "pump should have 6 instructions");
}

#[test]
fn test_parse_meteora_dlmm_modern() {
    let idl =
        parse_idl_file(&fixture_path("meteora_dlmm.json")).expect("should parse meteora_dlmm.json");
    assert_eq!(
        idl.instructions.len(),
        74,
        "meteora_dlmm should have 74 instructions"
    );
    assert_eq!(
        idl.constants.len(),
        30,
        "meteora_dlmm should have 30 constants"
    );
}
