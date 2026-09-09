use arete_hash::*;

#[test]
fn sdk_definition_v2_matches_shared_vector_and_tracks_content_contract() {
    let vector: serde_json::Value =
        serde_json::from_str(include_str!("../../test-vectors/sdk-definition-v2.json")).unwrap();
    let definition: SdkDefinitionV2 = serde_json::from_value(vector["projection"].clone()).unwrap();
    let expected = definition.hash().unwrap();
    assert_eq!(expected.to_string(), vector["expectedHash"]);
    for changed in [
        SdkDefinitionV2 {
            output_tree_hash: HashId::from_digest([3; 32]),
            ..definition.clone()
        },
        SdkDefinitionV2 {
            input_hash: HashId::from_digest([4; 32]),
            ..definition.clone()
        },
        SdkDefinitionV2 {
            runtime_contract: "@usearete/sdk/program-definition-v2".into(),
            ..definition.clone()
        },
        SdkDefinitionV2 {
            target: "rust".into(),
            ..definition.clone()
        },
    ] {
        assert_ne!(changed.hash().unwrap(), expected);
    }
    assert!(serde_json::from_value::<SdkDefinitionV1>(vector["projection"].clone()).is_err());
    let mut extra = vector["projection"].clone();
    extra["compilerHash"] = serde_json::json!("ignored?");
    assert!(serde_json::from_value::<SdkDefinitionV2>(extra).is_err());
    for value in ["", "has space", "nonascii-é"] {
        assert!(SdkDefinitionV2 {
            runtime_contract: value.into(),
            ..definition.clone()
        }
        .hash()
        .is_err());
    }
    assert!(SdkDefinitionV2 {
        target: "unknown".into(),
        ..definition.clone()
    }
    .hash()
    .is_err());
    assert!(SdkDefinitionV2 {
        schema: "arete.sdk-definition/v3".into(),
        ..definition
    }
    .hash()
    .is_err());
}
