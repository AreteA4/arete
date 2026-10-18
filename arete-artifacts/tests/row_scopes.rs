//! LiveSpec `rowScopes`: opt-in, validated, and absent from every LiveSpec
//! that does not use it.

use std::collections::BTreeMap;

use arete_artifacts::{
    author_stack_v2, AuthoredStackV2, LiveSpecArtifactV2, PortableBaseType, PortableEntity,
    PortableFieldTypeInfo, StackAuthoringV2,
};
use arete_hash::{CanonicalIdlDocument, ProgramSpecV1};

fn program() -> ProgramSpecV1 {
    let idl = r#"{"address":"11111111111111111111111111111111","metadata":{"name":"system","version":"1.0.0","spec":"0.1.0"},"instructions":[],"accounts":[],"types":[],"events":[],"errors":[]}"#;
    let document = CanonicalIdlDocument::parse(idl.as_bytes(), None).unwrap();
    ProgramSpecV1::from_document(&document)
}

fn field(name: &str, base_type: PortableBaseType) -> PortableFieldTypeInfo {
    PortableFieldTypeInfo {
        field_name: name.to_string(),
        raw_name: None,
        canonical_name: None,
        rust_type_name: "String".to_string(),
        base_type,
        integer_kind: None,
        is_optional: true,
        is_array: false,
        inner_type: None,
        source_path: None,
        resolved_type: None,
        emit: true,
    }
}

fn entity() -> PortableEntity {
    let mut entity = PortableEntity::new("Round", "id.address");
    entity.field_mappings.insert(
        "meta.scope".to_string(),
        field("scope", PortableBaseType::String),
    );
    entity.field_mappings.insert(
        "meta.count".to_string(),
        field("count", PortableBaseType::Integer),
    );
    entity
}

fn author(row_scopes: &[(&str, &str)]) -> Result<AuthoredStackV2, arete_artifacts::ArtifactError> {
    let mut input = StackAuthoringV2::new("Scoped", vec![program()], vec![entity()]);
    input.row_scopes = row_scopes
        .iter()
        .map(|(entity, path)| (entity.to_string(), path.to_string()))
        .collect();
    author_stack_v2(input)
}

#[test]
fn a_stack_without_row_scopes_keeps_its_bytes() {
    let live = author(&[]).unwrap().live_spec.unwrap();
    assert!(live.payload.row_scopes.is_none());
    let bytes = String::from_utf8(live.canonical_bytes().unwrap()).unwrap();
    assert!(!bytes.contains("rowScopes"), "{bytes}");
}

#[test]
fn row_scopes_are_published_and_change_the_hash() {
    let plain = author(&[]).unwrap().live_spec.unwrap();
    let scoped = author(&[("Round", "meta.scope")])
        .unwrap()
        .live_spec
        .unwrap();
    assert_ne!(plain.artifact_hash, scoped.artifact_hash);
    assert_eq!(
        scoped.payload.row_scopes,
        Some(BTreeMap::from([(
            "Round".to_string(),
            "meta.scope".to_string()
        )]))
    );
    let bytes = scoped.canonical_bytes().unwrap();
    let text = String::from_utf8(bytes.clone()).unwrap();
    assert!(
        text.contains(r#""rowScopes":{"Round":"meta.scope"}"#),
        "{text}"
    );
    let loaded: LiveSpecArtifactV2 = serde_json::from_slice(&bytes).unwrap();
    loaded.validate().unwrap();
    assert_eq!(loaded, scoped);
}

#[test]
fn row_scopes_must_name_a_string_field_of_a_known_entity() {
    for (row_scopes, expected) in [
        (vec![("Board", "meta.scope")], "unknown entity"),
        (vec![("Round", "meta.missing")], "is not a field"),
        (vec![("Round", "meta.count")], "must be a string"),
        (vec![("Round", "meta..scope")], "dotted field path"),
    ] {
        let error = author(&row_scopes).unwrap_err().to_string();
        assert!(error.contains(expected), "{row_scopes:?}: {error}");
    }

    let mut payload = author(&[]).unwrap().live_spec.unwrap().payload;
    payload.row_scopes = Some(BTreeMap::new());
    let error = LiveSpecArtifactV2::new(payload).unwrap_err().to_string();
    assert!(error.contains("at least one entity"), "{error}");
}
