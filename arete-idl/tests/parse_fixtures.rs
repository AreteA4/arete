use arete_idl::parse::parse_idl_file;
use arete_idl::snapshot::IdlSnapshot;
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
        vec![0],
        "automate instruction should have discriminator [0]"
    );

    // Verify program_id is parsed from address field (using ore.json fixture)
    let original_idl_json =
        fs::read_to_string(fixture_path("ore.json")).expect("should read ore.json");
    let original_snapshot: IdlSnapshot =
        serde_json::from_str(&original_idl_json).expect("should parse ore.json as IdlSnapshot");

    assert_eq!(
        original_snapshot.program_id,
        Some("oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv".to_string()),
        "program_id should be parsed from address field"
    );

    assert_eq!(
        original_snapshot.discriminant_size, 1,
        "Steel-style IDL should use 1-byte discriminants"
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

#[test]
fn test_parse_mpl_core_tuple_types() {
    // Trimmed from mpl-core (Codama/Kinobi legacy-Anchor render), which uses
    // inline `{"tuple": [..]}` type nodes inside `vec` and `option<vec>`.
    // These previously failed deserialization with "data did not match any
    // variant of untagged enum IdlTypeDefKind".
    let idl = parse_idl_file(&fixture_path("mpl_core_tuple.json"))
        .expect("should parse mpl_core_tuple.json");

    assert_eq!(idl.instructions.len(), 1);
    assert_eq!(idl.types.len(), 4);

    let init_info = idl
        .types
        .iter()
        .find(|t| t.name == "AgentIdentityInitInfo")
        .expect("AgentIdentityInitInfo typedef");
    let arete_idl::IdlTypeDefKind::Struct { fields, .. } = &init_info.type_def else {
        panic!("AgentIdentityInitInfo should be a struct");
    };
    let arete_idl::IdlType::Vec(vec_type) = &fields[0].type_ else {
        panic!("lifecycleChecks should be a vec");
    };
    let arete_idl::IdlType::Tuple(tuple) = vec_type.vec.as_ref() else {
        panic!("vec element should be a tuple");
    };
    assert_eq!(tuple.tuple.len(), 2, "tuple should have two elements");
    for (element, expected) in tuple
        .tuple
        .iter()
        .zip(["HookableLifecycleEvent", "ExternalCheckResult"])
    {
        let arete_idl::IdlType::Defined(defined) = element else {
            panic!("tuple elements should be defined types");
        };
        let arete_idl::IdlTypeDefinedInner::Simple(name) = &defined.defined else {
            panic!("defined should use the legacy string form");
        };
        assert_eq!(name, expected);
    }

    // option<vec<tuple>> also parses.
    let update_info = idl
        .types
        .iter()
        .find(|t| t.name == "AgentIdentityUpdateInfo")
        .expect("AgentIdentityUpdateInfo typedef");
    let arete_idl::IdlTypeDefKind::Struct { fields, .. } = &update_info.type_def else {
        panic!("AgentIdentityUpdateInfo should be a struct");
    };
    let arete_idl::IdlType::Option(option_type) = &fields[1].type_ else {
        panic!("lifecycleChecks should be an option");
    };
    assert!(
        matches!(
            option_type.option.as_ref(),
            arete_idl::IdlType::Vec(inner) if matches!(inner.vec.as_ref(), arete_idl::IdlType::Tuple(_))
        ),
        "option should wrap vec<tuple>"
    );
}

#[test]
fn test_mpl_core_tuple_snapshot_roundtrip() {
    use arete_idl::snapshot::{normalize_idl_snapshot, IdlTypeSnapshot};

    let idl = parse_idl_file(&fixture_path("mpl_core_tuple.json"))
        .expect("should parse mpl_core_tuple.json");
    let snapshot = normalize_idl_snapshot(&idl);

    // Steel/shank-style 1-byte discriminant carries through.
    assert_eq!(snapshot.discriminant_size, 1);
    assert_eq!(snapshot.instructions[0].discriminator, vec![28]);

    let init_info = snapshot
        .types
        .iter()
        .find(|t| t.name == "AgentIdentityInitInfo")
        .expect("AgentIdentityInitInfo snapshot");
    let arete_idl::snapshot::IdlTypeDefKindSnapshot::Struct { fields, .. } = &init_info.type_def
    else {
        panic!("AgentIdentityInitInfo snapshot should be a struct");
    };
    let IdlTypeSnapshot::Vec(vec_type) = &fields[0].type_ else {
        panic!("lifecycleChecks snapshot should be a vec");
    };
    let IdlTypeSnapshot::Tuple(tuple) = vec_type.vec.as_ref() else {
        panic!("snapshot vec element should be a tuple");
    };
    assert_eq!(tuple.tuple.len(), 2);

    // The snapshot serializes tuples in the same `{"tuple": [..]}` wire shape
    // and deserializes back to the same variant.
    let json = serde_json::to_value(&fields[0].type_).expect("serialize");
    assert_eq!(
        json,
        serde_json::json!({
            "vec": {
                "tuple": [
                    {"defined": "HookableLifecycleEvent"},
                    {"defined": "ExternalCheckResult"}
                ]
            }
        })
    );
    let reparsed: IdlTypeSnapshot = serde_json::from_value(json).expect("deserialize");
    assert!(
        matches!(
            &reparsed,
            IdlTypeSnapshot::Vec(v) if matches!(v.vec.as_ref(), IdlTypeSnapshot::Tuple(t) if t.tuple.len() == 2)
        ),
        "tuple snapshot should round-trip through JSON"
    );
}
