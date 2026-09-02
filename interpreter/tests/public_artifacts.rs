use std::collections::BTreeMap;

use arete_artifacts::{
    compose_stack_manifest_v2, live_spec_v2, load_live_spec_v2, load_program_spec,
    load_stack_manifest_v2, LiveSpecArtifactV2, PortableEntity, ProgramAdapterV2,
    ProgramSpecArtifact, SelectedViewV2, StackManifestArtifactV2,
};
use arete_hash::{CanonicalIdlDocument, PdaDefinitionV1, PdaSeedV1, ProgramSpecV1};
use arete_interpreter::public_artifacts::stack_specs_from_artifacts_v2;
use arete_interpreter::rust::compile_composed_public_artifacts_v2 as compile_rust_composition;
use arete_interpreter::typescript::{
    compile_composed_public_artifacts_v2 as compile_typescript_composition,
    compile_public_artifacts_v2,
};

fn ore_artifacts() -> (
    Vec<ProgramSpecArtifact>,
    LiveSpecArtifactV2,
    StackManifestArtifactV2,
) {
    let manifest = load_stack_manifest_v2(include_bytes!(
        "../../stacks/ore/.arete/OreStream.stack-manifest.json"
    ))
    .unwrap()
    .artifact;
    let live = load_live_spec_v2(include_bytes!(
        "../../stacks/ore/.arete/OreStream.live-spec.json"
    ))
    .unwrap()
    .artifact;
    let candidates = [
        load_program_spec(include_bytes!(
            "../../stacks/ore/.arete/ore.program-spec.json"
        ))
        .unwrap()
        .artifact,
        load_program_spec(include_bytes!(
            "../../stacks/ore/.arete/entropy.program-spec.json"
        ))
        .unwrap()
        .artifact,
    ];
    let programs = manifest
        .payload
        .programs
        .iter()
        .map(|reference| {
            candidates
                .iter()
                .find(|program| program.artifact_hash == reference.artifact_hash)
                .unwrap()
                .clone()
        })
        .collect();
    (programs, live, manifest)
}

fn program() -> ProgramSpecArtifact {
    let document = CanonicalIdlDocument::parse(
        br#"{"address":"11111111111111111111111111111111","metadata":{"name":"system","version":"1.0.0","spec":"0.1.0"},"instructions":[],"accounts":[],"types":[],"events":[],"errors":[]}"#,
        None,
    )
    .unwrap();
    ProgramSpecArtifact::new(ProgramSpecV1::from_document(&document)).unwrap()
}

fn named_program(name: &str, address: &str) -> ProgramSpecV1 {
    let idl = format!(
        r#"{{"address":"{address}","metadata":{{"name":"{name}","version":"1.0.0","spec":"0.1.0"}},"instructions":[],"accounts":[],"types":[],"events":[],"errors":[]}}"#
    );
    let document = CanonicalIdlDocument::parse(idl.as_bytes(), None).unwrap();
    ProgramSpecV1::from_document(&document)
}

fn adapted_live(program: &ProgramSpecArtifact, entity: &str, pda_name: &str) -> LiveSpecArtifactV2 {
    live_spec_v2(
        std::slice::from_ref(program),
        vec![PortableEntity::new(entity, "id.address")],
        vec![ProgramAdapterV2 {
            program_spec_hash: program.artifact_hash,
            pdas: BTreeMap::from([(
                pda_name.to_string(),
                PdaDefinitionV1 {
                    name: pda_name.to_string(),
                    seeds: vec![PdaSeedV1::Literal {
                        value: pda_name.to_string(),
                    }],
                    program_id: None,
                    program: None,
                },
            )]),
            instruction_resolutions: Vec::new(),
        }],
    )
    .unwrap()
}

#[test]
fn ore_exact_artifact_closure_generates_typescript() {
    let (programs, live, manifest) = ore_artifacts();
    let output = compile_public_artifacts_v2(&programs, &live, &manifest, None)
        .expect("artifact TypeScript");
    assert!(output.full_file().contains("export const"));
}

#[test]
fn ore_artifacts_are_public_and_repeatable() {
    let (_, live, manifest) = ore_artifacts();
    let (_, live_again, manifest_again) = ore_artifacts();
    assert_eq!(live.artifact_hash, live_again.artifact_hash);
    assert_eq!(manifest.artifact_hash, manifest_again.artifact_hash);
    let public_bytes = [
        live.canonical_bytes().unwrap(),
        manifest.canonical_bytes().unwrap(),
    ]
    .concat();
    let public = String::from_utf8(public_bytes).unwrap();
    for forbidden in [
        "platformParser",
        "platform_parser",
        "decoderBindingId",
        "decoderContentHash",
        "artifactRef",
    ] {
        assert!(!public.contains(forbidden), "found private key {forbidden}");
    }
}

#[test]
fn ore_v2_artifacts_have_no_legacy_extensions() {
    let (programs, live, manifest) = ore_artifacts();
    compile_public_artifacts_v2(&programs, &live, &manifest, None).expect("V2 artifact TypeScript");
    let bytes = live.canonical_bytes().unwrap();
    let public = String::from_utf8(bytes).unwrap();
    assert!(!public.contains("legacyProgramExtensions"));
    assert!(public.contains("programAdapters"));
}

#[test]
fn ore_v2_single_live_rust_generation_uses_exact_artifacts() {
    let (programs, live, manifest) = ore_artifacts();
    let output =
        arete_interpreter::rust::compile_public_artifacts_v2(&programs, &live, &manifest, None)
            .unwrap();
    assert!(!output.lib_rs.is_empty());
    assert!(!output.types_rs.is_empty());
    assert!(!output.entity_rs.is_empty());
}

#[test]
fn multi_live_interpretation_projects_views_and_isolates_adapters() {
    let program = program();
    let alpha = adapted_live(&program, "AlphaState", "alpha_pda");
    let beta = adapted_live(&program, "BetaState", "beta_pda");
    let lives = vec![("alpha".to_string(), alpha), ("beta".to_string(), beta)];
    let manifest = compose_stack_manifest_v2(
        "Composed",
        std::slice::from_ref(&program),
        lives
            .iter()
            .map(|(alias, live)| (alias.clone(), live))
            .collect(),
        vec![
            SelectedViewV2 {
                live_alias: "alpha".to_string(),
                view_id: "AlphaState/state".to_string(),
            },
            SelectedViewV2 {
                live_alias: "beta".to_string(),
                view_id: "BetaState/list".to_string(),
            },
        ],
    )
    .unwrap();

    let composed =
        stack_specs_from_artifacts_v2(std::slice::from_ref(&program), &lives, &manifest).unwrap();
    assert_eq!(
        composed
            .live_specs
            .iter()
            .map(|live| live.alias.as_str())
            .collect::<Vec<_>>(),
        vec!["alpha", "beta"]
    );
    assert_eq!(
        composed.live_specs[0].stack_spec.entities[0]
            .views
            .iter()
            .map(|view| view.id.as_str())
            .collect::<Vec<_>>(),
        vec!["AlphaState/state"]
    );
    assert_eq!(
        composed.live_specs[1].stack_spec.entities[0]
            .views
            .iter()
            .map(|view| view.id.as_str())
            .collect::<Vec<_>>(),
        vec!["BetaState/list"]
    );
    let alpha_pdas = &composed.live_specs[0].stack_spec.pdas["system"];
    let beta_pdas = &composed.live_specs[1].stack_spec.pdas["system"];
    assert!(alpha_pdas.contains_key("alpha_pda"));
    assert!(!alpha_pdas.contains_key("beta_pda"));
    assert!(beta_pdas.contains_key("beta_pda"));
    assert!(!beta_pdas.contains_key("alpha_pda"));
}

#[test]
fn empty_selected_views_generate_no_rust_client_views() {
    let program = program();
    let live = adapted_live(&program, "EmptyState", "empty_pda");
    let manifest = compose_stack_manifest_v2(
        "EmptyViews",
        std::slice::from_ref(&program),
        vec![("empty".to_string(), &live)],
        Vec::new(),
    )
    .unwrap();
    let output = arete_interpreter::rust::compile_public_artifacts_v2(
        std::slice::from_ref(&program),
        &live,
        &manifest,
        None,
    )
    .unwrap();

    assert!(!output.entity_rs.contains("EmptyStateEntityViews"));
    assert!(!output.entity_rs.contains("pub fn state"));
    assert!(!output.entity_rs.contains("pub fn list"));
    assert!(output
        .entity_rs
        .contains("pub struct EmptyViewsStackViews {\n\n}"));
}

#[test]
fn typescript_and_rust_compositions_keep_two_and_three_lives_namespaced() {
    let program = program();
    let shared = adapted_live(&program, "SharedState", "shared_pda");
    let third = adapted_live(&program, "ThirdState", "third_pda");
    let lives = vec![
        ("first-live".to_string(), shared.clone()),
        ("second_live".to_string(), shared),
        ("third".to_string(), third),
    ];
    let manifest = compose_stack_manifest_v2(
        "Jurassic",
        std::slice::from_ref(&program),
        lives
            .iter()
            .map(|(alias, live)| (alias.clone(), live))
            .collect(),
        vec![
            SelectedViewV2 {
                live_alias: "first-live".to_string(),
                view_id: "SharedState/list".to_string(),
            },
            SelectedViewV2 {
                live_alias: "third".to_string(),
                view_id: "ThirdState/list".to_string(),
            },
        ],
    )
    .unwrap();

    let typescript =
        compile_typescript_composition(std::slice::from_ref(&program), &lives, &manifest, None)
            .unwrap();
    assert_eq!(
        typescript
            .live_stacks
            .iter()
            .map(|live| (live.alias.as_str(), live.module_name.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("first-live", "first-live-stack"),
            ("second_live", "second-live-stack"),
            ("third", "third-stack")
        ]
    );
    assert!(typescript
        .session_definition
        .contains("mode: 'composition'"));
    assert!(typescript
        .session_definition
        .contains("\"first-live\": FirstLiveStack"));
    assert!(typescript
        .session_definition
        .contains("second_live: SecondLiveStack"));
    assert!(typescript
        .session_definition
        .contains("createJurassicSession"));
    assert!(typescript.live_stacks[0]
        .output
        .stack_definition
        .contains("list: listView<SharedState>('SharedState/list')"));
    assert!(!typescript.live_stacks[0]
        .output
        .stack_definition
        .contains("state: stateView<SharedState"));
    assert!(!typescript.live_stacks[1]
        .output
        .stack_definition
        .contains("SharedState/list"));

    let rust =
        compile_rust_composition(std::slice::from_ref(&program), &lives, &manifest, None).unwrap();
    assert_eq!(
        rust.live_stacks
            .iter()
            .map(|live| live.module_name.as_str())
            .collect::<Vec<_>>(),
        vec!["first_live", "second_live", "third"]
    );
    assert_eq!(
        rust.lib_rs,
        "pub mod first_live;\npub mod second_live;\npub mod third;\n"
    );
    assert!(rust.live_stacks[0].output.entity_rs.contains("pub fn list"));
    assert!(!rust.live_stacks[0]
        .output
        .entity_rs
        .contains("pub fn state"));
    assert!(!rust.live_stacks[1].output.entity_rs.contains("pub fn list"));
}

#[test]
fn typescript_composition_keeps_independent_program_reads() {
    let live_program = program();
    let independent = ProgramSpecArtifact::new(named_program(
        "independent_program",
        "Independent11111111111111111111111111111111",
    ))
    .unwrap();
    let live = adapted_live(&live_program, "LiveState", "live_pda");
    let lives = vec![("live".to_string(), live.clone())];
    let programs = vec![live_program, independent];
    let manifest = compose_stack_manifest_v2(
        "Jurassic",
        &programs,
        vec![("live".to_string(), &live)],
        Vec::new(),
    )
    .unwrap();

    let output = compile_typescript_composition(&programs, &lives, &manifest, None).unwrap();
    let collection = output.program_collection.as_ref().unwrap();
    assert_eq!(collection.module_name, "jurassic-programs");
    assert!(collection
        .output
        .stack_definition
        .contains("independentProgram"));
    assert!(collection
        .output
        .stack_definition
        .contains("JurassicProgramsEntity = never"));
    assert!(output
        .session_definition
        .contains("independentProgram: JurassicPrograms.programs.independentProgram"));
    assert!(output
        .session_definition
        .contains("independentProgram: JurassicPrograms.programReads.independentProgram"));
    assert!(output
        .session_definition
        .contains("export const JURASSIC_SDK"));
}
