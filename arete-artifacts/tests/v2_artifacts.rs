use std::collections::BTreeMap;

use arete_artifacts::{
    atomic_write, author_stack_v2, compose_stack_manifest_v2, load_live_spec,
    normalize_live_spec_v1, resolve_stack_composition_v2, selected_views, stack_manifest_v2,
    write_authored_stack_v2, ArtifactError, LiveSpecArtifactV2, LiveSpecReferenceV2,
    PortableEntity, PortableView, PortableViewOutput, PortableViewSource, ProgramRequirementV2,
    ProgramSpecArtifact, ProgramSpecReferenceV2, SelectedViewV2, StackAuthoringV2,
    StackManifestArtifactV2, StackManifestV2, DEFAULT_LIVE_ALIAS, LIVE_SPEC_SCHEMA_V2,
    STACK_MANIFEST_SCHEMA_V2,
};
use arete_hash::{hash_raw_bytes, ArtifactFile, CanonicalIdlDocument, ProgramSpecV1};

fn program() -> ProgramSpecV1 {
    named_program("system", "11111111111111111111111111111111")
}

fn named_program(name: &str, address: &str) -> ProgramSpecV1 {
    let idl = format!(
        r#"{{"address":"{address}","metadata":{{"name":"{name}","version":"1.0.0","spec":"0.1.0"}},"instructions":[],"accounts":[],"types":[],"events":[],"errors":[]}}"#
    );
    let document = CanonicalIdlDocument::parse(idl.as_bytes(), None).unwrap();
    ProgramSpecV1::from_document(&document)
}

#[test]
fn multi_program_multi_entity_order_is_a_stable_golden_contract() {
    let artifacts = author_stack_v2(StackAuthoringV2::new(
        "ComposedStack",
        vec![
            program(),
            named_program("vote", "Vote111111111111111111111111111111111111111"),
        ],
        vec![
            PortableEntity::new("SystemState", "id.address"),
            PortableEntity::new("VoteState", "id.proposal"),
        ],
    ))
    .unwrap();
    let live = artifacts.live_spec.unwrap();
    assert_eq!(
        live.payload
            .programs
            .iter()
            .map(|program| program.program_id.as_str())
            .collect::<Vec<_>>(),
        vec![
            "11111111111111111111111111111111",
            "Vote111111111111111111111111111111111111111"
        ]
    );
    assert_eq!(
        live.payload
            .entities
            .iter()
            .map(|entity| entity.state_name.as_str())
            .collect::<Vec<_>>(),
        vec!["SystemState", "VoteState"]
    );
}

fn authored() -> arete_artifacts::AuthoredStackV2 {
    author_stack_v2(StackAuthoringV2::new(
        "SystemStack",
        vec![program()],
        vec![PortableEntity::new("SystemState", "id.address")],
    ))
    .unwrap()
}

fn artifact(name: &str, address: &str) -> ProgramSpecArtifact {
    ProgramSpecArtifact::new(named_program(name, address)).unwrap()
}

fn live(
    programs: &[ProgramSpecArtifact],
    entity_name: &str,
    derived_views: &[&str],
) -> LiveSpecArtifactV2 {
    let mut entity = PortableEntity::new(entity_name, "id.address");
    entity
        .views
        .extend(derived_views.iter().map(|name| PortableView {
            id: format!("{entity_name}/{name}"),
            source: PortableViewSource::Entity {
                name: entity_name.to_string(),
            },
            pipeline: Vec::new(),
            output: PortableViewOutput::Collection,
        }));
    arete_artifacts::live_spec_v2(programs, vec![entity], Vec::new()).unwrap()
}

#[test]
fn typed_v2_authoring_matches_the_golden_shape() {
    let artifacts = authored();
    let live = artifacts.live_spec.as_ref().unwrap();
    assert_eq!(live.payload.schema, LIVE_SPEC_SCHEMA_V2);
    assert!(live.payload.program_adapters.is_empty());
    assert_eq!(live.payload.entities[0].views.len(), 2);
    assert_eq!(live.payload.entities[0].views[0].id, "SystemState/state");
    assert_eq!(live.payload.entities[0].views[1].id, "SystemState/list");

    let manifest = &artifacts.stack_manifest.payload;
    assert_eq!(manifest.schema, STACK_MANIFEST_SCHEMA_V2);
    assert_eq!(manifest.live_specs[0].alias, DEFAULT_LIVE_ALIAS);
    assert_eq!(
        manifest
            .selected_views
            .iter()
            .map(|selected| selected.view_id.as_str())
            .collect::<Vec<_>>(),
        vec!["SystemState/state", "SystemState/list"]
    );
}

#[test]
fn v2_hashes_and_canonical_bytes_are_deterministic() {
    let first = authored();
    let second = authored();
    assert_eq!(
        first.live_spec.as_ref().unwrap().artifact_hash,
        second.live_spec.as_ref().unwrap().artifact_hash
    );
    assert_eq!(
        first.stack_manifest.artifact_hash,
        second.stack_manifest.artifact_hash
    );
    assert_eq!(
        first.live_spec.unwrap().canonical_bytes().unwrap(),
        second.live_spec.unwrap().canonical_bytes().unwrap()
    );
    assert_eq!(
        first.stack_manifest.canonical_bytes().unwrap(),
        second.stack_manifest.canonical_bytes().unwrap()
    );
}

#[test]
fn zero_live_manifest_is_the_program_only_convenience_artifact() {
    let artifacts = author_stack_v2(StackAuthoringV2::new(
        "SystemProgram",
        vec![program()],
        Vec::new(),
    ))
    .unwrap();
    assert!(artifacts.live_spec.is_none());
    assert!(artifacts.stack_manifest.payload.live_specs.is_empty());
    assert!(artifacts.stack_manifest.payload.selected_views.is_empty());
}

#[test]
fn aliases_and_selected_views_are_exactly_validated() {
    let artifacts = authored();
    let live = artifacts.live_spec.as_ref().unwrap();
    let mut selected = selected_views("other", &live.payload);
    selected.push(SelectedViewV2 {
        live_alias: "other".to_string(),
        view_id: "SystemState/missing".to_string(),
    });
    let error = stack_manifest_v2(
        "Invalid",
        &artifacts.program_specs,
        vec![("other".to_string(), live)],
        selected,
    )
    .unwrap_err();
    assert!(error.to_string().contains("does not exist"));

    let duplicate_alias = StackManifestV2 {
        schema: STACK_MANIFEST_SCHEMA_V2.to_string(),
        name: "Invalid".to_string(),
        programs: vec![ProgramSpecReferenceV2 {
            program_id: artifacts.program_specs[0].payload.program_id.clone(),
            artifact_hash: artifacts.program_specs[0].artifact_hash,
        }],
        live_specs: vec![
            LiveSpecReferenceV2 {
                alias: "same".to_string(),
                artifact_hash: live.artifact_hash,
            },
            LiveSpecReferenceV2 {
                alias: "same".to_string(),
                artifact_hash: live.artifact_hash,
            },
        ],
        selected_views: Vec::new(),
        queries: Vec::new(),
        extensions: BTreeMap::new(),
        metadata: BTreeMap::new(),
    };
    assert!(StackManifestArtifactV2::new(duplicate_alias).is_err());

    let manifest = &artifacts.stack_manifest.payload;
    assert!(manifest
        .validate_selected_views([(DEFAULT_LIVE_ALIAS, live), (DEFAULT_LIVE_ALIAS, live),])
        .unwrap_err()
        .to_string()
        .contains("supplied more than once"));

    let repeated_artifact = StackManifestArtifactV2::new(StackManifestV2::new(
        "RepeatedArtifact",
        Vec::new(),
        vec![
            LiveSpecReferenceV2 {
                alias: "primary".to_string(),
                artifact_hash: live.artifact_hash,
            },
            LiveSpecReferenceV2 {
                alias: "replica".to_string(),
                artifact_hash: live.artifact_hash,
            },
        ],
        Vec::new(),
    ))
    .unwrap();
    repeated_artifact
        .payload
        .validate_selected_views([("primary", live), ("replica", live)])
        .unwrap();
}

#[test]
fn two_and_three_live_compositions_resolve_hash_keyed_dependency_subsets() {
    let system = artifact("system", "11111111111111111111111111111111");
    let vote = artifact("vote", "Vote111111111111111111111111111111111111111");
    let memo = artifact("memo", "Memo111111111111111111111111111111111111111");
    let primary = live(&[system.clone(), vote.clone()], "Primary", &[]);
    let secondary = live(&[vote.clone()], "Secondary", &[]);
    let tertiary = live(&[memo.clone(), system.clone()], "Tertiary", &[]);
    let lives = vec![
        ("primary".to_string(), primary),
        ("secondary".to_string(), secondary),
        ("tertiary".to_string(), tertiary),
    ];
    let programs = vec![memo.clone(), system.clone(), vote.clone()];
    let selected = vec![
        SelectedViewV2 {
            live_alias: "primary".to_string(),
            view_id: "Primary/state".to_string(),
        },
        SelectedViewV2 {
            live_alias: "tertiary".to_string(),
            view_id: "Tertiary/list".to_string(),
        },
    ];
    let manifest = compose_stack_manifest_v2(
        "Three",
        &programs,
        lives
            .iter()
            .map(|(alias, live)| (alias.clone(), live))
            .collect(),
        selected,
    )
    .unwrap();
    let resolved = resolve_stack_composition_v2(&manifest, &lives, &programs).unwrap();

    assert_eq!(
        resolved
            .live_specs
            .iter()
            .map(|live| live.alias.as_str())
            .collect::<Vec<_>>(),
        vec!["primary", "secondary", "tertiary"]
    );
    assert_eq!(resolved.live_specs[0].program_specs.len(), 2);
    assert_eq!(resolved.live_specs[1].program_specs.len(), 1);
    assert_eq!(resolved.live_specs[2].program_specs.len(), 2);
    assert_eq!(resolved.live_specs[0].selected_views, vec!["Primary/state"]);
    assert!(resolved.live_specs[1].selected_views.is_empty());
    assert_eq!(resolved.live_specs[2].selected_views, vec!["Tertiary/list"]);
}

#[test]
fn composition_preserves_independent_program_read_dependencies() {
    let system = artifact("system", "11111111111111111111111111111111");
    let memo = artifact("memo", "Memo111111111111111111111111111111111111111");
    let live = live(std::slice::from_ref(&system), "SystemState", &[]);
    let lives = vec![("live".to_string(), live.clone())];
    let programs = vec![system.clone(), memo.clone()];
    let manifest = compose_stack_manifest_v2(
        "IndependentProgramRead",
        &programs,
        vec![("live".to_string(), &live)],
        Vec::new(),
    )
    .unwrap();

    let resolved = resolve_stack_composition_v2(&manifest, &lives, &programs).unwrap();
    assert_eq!(
        resolved
            .program_specs
            .iter()
            .map(|program| program.artifact_hash)
            .collect::<Vec<_>>(),
        vec![system.artifact_hash, memo.artifact_hash]
    );
    assert_eq!(resolved.live_specs[0].program_specs.len(), 1);
}

#[test]
fn the_same_live_hash_may_be_bound_to_multiple_ordered_aliases() {
    let program = artifact("system", "11111111111111111111111111111111");
    let shared = live(std::slice::from_ref(&program), "Shared", &[]);
    let lives = vec![
        ("first".to_string(), shared.clone()),
        ("second".to_string(), shared),
    ];
    let manifest = compose_stack_manifest_v2(
        "Repeated",
        std::slice::from_ref(&program),
        lives
            .iter()
            .map(|(alias, live)| (alias.clone(), live))
            .collect(),
        Vec::new(),
    )
    .unwrap();
    let resolved =
        resolve_stack_composition_v2(&manifest, &lives, std::slice::from_ref(&program)).unwrap();
    assert_eq!(resolved.live_specs.len(), 2);
    assert_eq!(
        resolved.live_specs[0].artifact.artifact_hash,
        resolved.live_specs[1].artifact.artifact_hash
    );

    let reversed = vec![lives[1].clone(), lives[0].clone()];
    assert!(
        resolve_stack_composition_v2(&manifest, &reversed, std::slice::from_ref(&program))
            .unwrap_err()
            .to_string()
            .contains("exact alias and hash")
    );
}

#[test]
fn composition_rejects_missing_unused_duplicate_and_mismatched_programs() {
    let system = artifact("system", "11111111111111111111111111111111");
    let unused = artifact("unused", "Unused1111111111111111111111111111111111111");
    let live = live(std::slice::from_ref(&system), "State", &[]);
    let lives = vec![("live".to_string(), live.clone())];
    let manifest = compose_stack_manifest_v2(
        "Exact",
        std::slice::from_ref(&system),
        vec![("live".to_string(), &live)],
        Vec::new(),
    )
    .unwrap();

    assert!(resolve_stack_composition_v2(&manifest, &lives, &[])
        .unwrap_err()
        .to_string()
        .contains("exactly match"));
    assert!(
        resolve_stack_composition_v2(&manifest, &lives, &[system.clone(), unused])
            .unwrap_err()
            .to_string()
            .contains("exactly match")
    );
    assert!(
        resolve_stack_composition_v2(&manifest, &lives, &[system.clone(), system.clone()])
            .unwrap_err()
            .to_string()
            .contains("supplied more than once")
    );

    let mismatched = LiveSpecArtifactV2::new(arete_artifacts::LiveSpecV2::new(
        vec![ProgramRequirementV2 {
            program_id: "different-program".to_string(),
            program_spec_hash: system.artifact_hash,
        }],
        Vec::new(),
        Vec::new(),
    ))
    .unwrap();
    let mismatch_lives = vec![("live".to_string(), mismatched.clone())];
    let mismatch_manifest = StackManifestArtifactV2::new(StackManifestV2::new(
        "Mismatch",
        vec![ProgramSpecReferenceV2 {
            program_id: system.payload.program_id.clone(),
            artifact_hash: system.artifact_hash,
        }],
        vec![LiveSpecReferenceV2 {
            alias: "live".to_string(),
            artifact_hash: mismatched.artifact_hash,
        }],
        Vec::new(),
    ))
    .unwrap();
    assert!(
        resolve_stack_composition_v2(&mismatch_manifest, &mismatch_lives, &[system])
            .unwrap_err()
            .to_string()
            .contains("program ID")
    );
}

#[test]
fn selected_views_are_an_exact_allowlist_and_collisions_are_deterministic() {
    let program = artifact("system", "11111111111111111111111111111111");
    let live = live(
        std::slice::from_ref(&program),
        "SystemState",
        &["latest-round", "latest_round"],
    );
    let empty = compose_stack_manifest_v2(
        "NoViews",
        std::slice::from_ref(&program),
        vec![("live".to_string(), &live)],
        Vec::new(),
    )
    .unwrap();
    let lives = vec![("live".to_string(), live.clone())];
    assert!(
        resolve_stack_composition_v2(&empty, &lives, std::slice::from_ref(&program))
            .unwrap()
            .live_specs[0]
            .selected_views
            .is_empty()
    );

    let selected = compose_stack_manifest_v2(
        "Selected",
        std::slice::from_ref(&program),
        vec![("live".to_string(), &live)],
        vec![SelectedViewV2 {
            live_alias: "live".to_string(),
            view_id: "SystemState/state".to_string(),
        }],
    )
    .unwrap();
    assert_eq!(
        resolve_stack_composition_v2(&selected, &lives, std::slice::from_ref(&program))
            .unwrap()
            .live_specs[0]
            .selected_views,
        vec!["SystemState/state"]
    );

    let collision = compose_stack_manifest_v2(
        "Collision",
        std::slice::from_ref(&program),
        vec![("live".to_string(), &live)],
        vec![
            SelectedViewV2 {
                live_alias: "live".to_string(),
                view_id: "SystemState/latest-round".to_string(),
            },
            SelectedViewV2 {
                live_alias: "live".to_string(),
                view_id: "SystemState/latest_round".to_string(),
            },
        ],
    )
    .unwrap_err();
    assert!(collision
        .to_string()
        .contains("language-safe normalization"));

    let alias_collision = StackManifestArtifactV2::new(StackManifestV2::new(
        "AliasCollision",
        vec![ProgramSpecReferenceV2 {
            program_id: program.payload.program_id.clone(),
            artifact_hash: program.artifact_hash,
        }],
        vec![
            LiveSpecReferenceV2 {
                alias: "foo-bar".to_string(),
                artifact_hash: live.artifact_hash,
            },
            LiveSpecReferenceV2 {
                alias: "foo_bar".to_string(),
                artifact_hash: live.artifact_hash,
            },
        ],
        Vec::new(),
    ))
    .unwrap();
    let alias_lives = vec![
        ("foo-bar".to_string(), live.clone()),
        ("foo_bar".to_string(), live),
    ];
    assert!(resolve_stack_composition_v2(
        &alias_collision,
        &alias_lives,
        std::slice::from_ref(&program)
    )
    .unwrap_err()
    .to_string()
    .contains("LiveSpec alias names"));

    let entity_collision = arete_artifacts::live_spec_v2(
        std::slice::from_ref(&program),
        vec![
            PortableEntity::new("FooBar", "id.address"),
            PortableEntity::new("foo_bar", "id.address"),
        ],
        Vec::new(),
    )
    .unwrap();
    assert!(compose_stack_manifest_v2(
        "EntityCollision",
        std::slice::from_ref(&program),
        vec![("live".to_string(), &entity_collision)],
        Vec::new(),
    )
    .unwrap_err()
    .to_string()
    .contains("entity in LiveSpec alias"));
}

#[test]
fn private_fields_are_rejected_from_typed_public_artifacts() {
    let artifacts = authored();
    let mut payload = artifacts.stack_manifest.payload;
    payload.metadata.insert(
        "decoderBindingId".to_string(),
        serde_json::Value::String("private".to_string()),
    );
    assert!(matches!(
        StackManifestArtifactV2::new(payload),
        Err(ArtifactError::PrivateField(_))
    ));

    let live_json =
        String::from_utf8(artifacts.live_spec.unwrap().canonical_bytes().unwrap()).unwrap();
    assert!(!live_json.contains("legacyProgramExtensions"));
    assert!(!live_json.contains("decoderBindingId"));
}

#[test]
fn canonical_writes_replace_files_without_leaving_partial_temps() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("artifact.json");
    std::fs::write(&path, b"old").unwrap();
    atomic_write(&path, b"new").unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"new");
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn failed_atomic_write_is_fatal_and_cleans_its_temporary_file() {
    let directory = tempfile::tempdir().unwrap();
    let occupied = directory.path().join("occupied.json");
    std::fs::create_dir(&occupied).unwrap();
    assert!(atomic_write(&occupied, b"new").is_err());
    assert!(occupied.is_dir());
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn colliding_program_filenames_fail_before_writing_any_artifact() {
    let artifacts = author_stack_v2(StackAuthoringV2::new(
        "Collision",
        vec![
            named_program("same", "11111111111111111111111111111111"),
            named_program("same", "Vote111111111111111111111111111111111111111"),
        ],
        Vec::new(),
    ))
    .unwrap();
    let root = tempfile::tempdir().unwrap();
    let output = root.path().join("artifacts");
    let error = write_authored_stack_v2(&output, "Collision", &artifacts).unwrap_err();
    assert!(error.to_string().contains("same.program-spec.json"));
    assert!(!output.exists());
}

#[test]
fn v1_loading_preserves_source_bytes_and_hash_while_normalizing_to_v2() {
    let program = arete_artifacts::ProgramSpecArtifact::new(program()).unwrap();
    let v1 = arete_artifacts::LiveSpecArtifact::new(arete_artifacts::LiveSpecV1 {
        schema: arete_artifacts::LIVE_SPEC_SCHEMA_V1.to_string(),
        compiler_contract_version: "compiler/v1".to_string(),
        wire_contract_version: "wire/v1".to_string(),
        programs: vec![arete_artifacts::ProgramRequirementV1 {
            program_id: program.payload.program_id.clone(),
            program_spec_hash: program.artifact_hash,
        }],
        entities: Vec::new(),
        legacy_program_extensions: None,
    })
    .unwrap();
    let original_artifact_hash = v1.artifact_hash;
    let original_bytes = serde_json::to_vec_pretty(&v1).unwrap();
    let loaded = load_live_spec(&original_bytes).unwrap();
    assert_eq!(loaded.original_bytes, original_bytes);
    assert_eq!(loaded.artifact.artifact_hash, original_artifact_hash);
    assert_eq!(
        loaded.source_hash,
        hash_raw_bytes::<ArtifactFile>(&loaded.original_bytes).unwrap()
    );
    let normalized = normalize_live_spec_v1(&loaded.artifact, &[program]).unwrap();
    assert_eq!(normalized.payload.schema, LIVE_SPEC_SCHEMA_V2);
}
