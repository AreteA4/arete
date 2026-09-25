use std::collections::BTreeMap;

use arete_artifacts::{
    compose_stack_manifest_v2, live_spec_v2, load_live_spec_v2, load_program_spec,
    load_stack_manifest_v2, LiveSpecArtifactV2, PortableEntity, ProgramAdapterV2,
    ProgramSpecArtifact, SelectedViewV2, StackManifestArtifactV2,
};
use arete_hash::{CanonicalIdlDocument, PdaDefinitionV1, PdaSeedV1, ProgramSpecV1};
use arete_interpreter::public_artifacts::{
    stack_spec_from_artifacts_v2, stack_specs_from_artifacts_v2, StackRelease,
};
use arete_interpreter::python::{
    compile_composed_public_artifacts_v2 as compile_python_composition, PythonCompositionConfig,
    PythonStackConfig,
};
use arete_interpreter::rust::{
    compile_composed_public_artifacts_v2 as compile_rust_composition, RustCompositionConfig,
    RustStackConfig,
};
use arete_interpreter::typescript::{
    compile_composed_public_artifacts_v2 as compile_typescript_composition,
    compile_public_artifacts_v2, TypeScriptCompositionConfig, TypeScriptLiveEndpoints,
    TypeScriptStackConfig,
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

const ORE_WS: &str = "wss://ore.stack.arete.run";

fn ore_manifest_hash(manifest: &StackManifestArtifactV2) -> String {
    let hash = manifest.artifact_hash.to_string();
    assert!(hash.starts_with("arete:h1:stack-manifest:sha256:"));
    hash
}

/// Every string a generated SDK uses to name a served version.
fn names_no_release(generated: &str) -> bool {
    [
        "stackManifestHash",
        "stack_manifest_hash",
        "liveAlias",
        "live_alias",
        "StackRelease",
    ]
    .iter()
    .all(|marker| !generated.contains(marker))
}

#[test]
fn stack_release_names_the_manifest_hash_and_alias() {
    let (_, _, manifest) = ore_artifacts();
    assert_eq!(
        StackRelease::for_single_live(&manifest),
        Some(StackRelease {
            stack_manifest_hash: ore_manifest_hash(&manifest),
            live_alias: "live".to_string(),
        })
    );
}

#[test]
fn hosted_typescript_generation_only_adds_the_release_to_ast_generation() {
    let (programs, live, manifest) = ore_artifacts();
    let hash = ore_manifest_hash(&manifest);
    let config = TypeScriptStackConfig {
        websocket_url: Some(ORE_WS.to_string()),
        ..TypeScriptStackConfig::default()
    };

    let ast_only = arete_interpreter::typescript::compile_stack_spec_with_exact_views(
        stack_spec_from_artifacts_v2(&programs, &live, &manifest).unwrap(),
        Some(config.clone()),
    )
    .unwrap()
    .full_file();
    let hosted = compile_public_artifacts_v2(&programs, &live, &manifest, Some(config))
        .unwrap()
        .full_file();
    let local = compile_public_artifacts_v2(&programs, &live, &manifest, None)
        .unwrap()
        .full_file();

    assert!(
        names_no_release(&ast_only),
        "AST-only generation names no release"
    );
    assert!(names_no_release(&local), "no endpoint, no served version");
    let release_block =
        format!("  release: {{\n    stackManifestHash: '{hash}',\n    liveAlias: 'live',\n  }},\n");
    assert_eq!(
        hosted,
        ast_only.replacen("  views: {\n", &format!("{release_block}  views: {{\n"), 1)
    );
}

#[test]
fn hosted_rust_generation_only_adds_the_release_to_ast_generation() {
    let (programs, live, manifest) = ore_artifacts();
    let hash = ore_manifest_hash(&manifest);
    let config = RustStackConfig {
        url: Some(ORE_WS.to_string()),
        ..RustStackConfig::default()
    };

    let ast_only = arete_interpreter::rust::compile_stack_spec_with_exact_views(
        stack_spec_from_artifacts_v2(&programs, &live, &manifest).unwrap(),
        Some(config.clone()),
    )
    .unwrap();
    let hosted = arete_interpreter::rust::compile_public_artifacts_v2(
        &programs,
        &live,
        &manifest,
        Some(config),
    )
    .unwrap();

    assert!(names_no_release(&ast_only.entity_rs));
    let url_fn = format!("fn url() -> &'static str {{\n        \"{ORE_WS}\"\n    }}");
    let release_fns = format!(
        "{url_fn}\n\n    fn stack_manifest_hash() -> Option<&'static str> {{\n        Some(\"{hash}\")\n    }}\n\n    fn live_alias() -> Option<&'static str> {{\n        Some(\"live\")\n    }}"
    );
    assert_eq!(
        hosted.entity_rs,
        ast_only.entity_rs.replacen(&url_fn, &release_fns, 1)
    );
    assert_eq!(hosted.lib_rs, ast_only.lib_rs);
    assert_eq!(hosted.types_rs, ast_only.types_rs);
    assert_eq!(hosted.programs_rs, ast_only.programs_rs);
    assert_eq!(hosted.cargo_toml, ast_only.cargo_toml);
}

#[test]
fn hosted_python_generation_only_adds_the_release_to_ast_generation() {
    let (programs, live, manifest) = ore_artifacts();
    let hash = ore_manifest_hash(&manifest);
    let config = PythonStackConfig {
        url: Some(ORE_WS.to_string()),
        ..PythonStackConfig::default()
    };

    let ast_only = arete_interpreter::python::compile_stack_spec_with_exact_views(
        stack_spec_from_artifacts_v2(&programs, &live, &manifest).unwrap(),
        Some(config.clone()),
    )
    .unwrap();
    let hosted = arete_interpreter::python::compile_public_artifacts_v2(
        &programs,
        &live,
        &manifest,
        Some(config),
    )
    .unwrap();

    assert!(names_no_release(&ast_only.init_py));
    let expected = ast_only
        .init_py
        .replacen(
            "from arete.stack import StackDef, StackEndpoints\n",
            "from arete.stack import StackDef, StackEndpoints, StackRelease\n",
            1,
        )
        .replacen(
            "\n)\n\n__all__",
            &format!(
                "\n    release=StackRelease(\n        stack_manifest_hash=\"{hash}\",\n        live_alias=\"live\",\n    ),\n)\n\n__all__"
            ),
            1,
        );
    assert_eq!(hosted.init_py, expected);
    assert_eq!(hosted.models_py, ast_only.models_py);
    assert_eq!(hosted.views_py, ast_only.views_py);
    assert_eq!(hosted.programs_py, ast_only.programs_py);
    assert_eq!(hosted.pyproject_toml, ast_only.pyproject_toml);
}

#[test]
fn compositions_name_the_served_version_of_each_bound_alias_only() {
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
    let hash = manifest.artifact_hash.to_string();
    let bound = |alias: &str| format!("wss://{alias}.stack.arete.run");
    // `second_live` has no served endpoint, so it names no version.
    let live_urls = BTreeMap::from([
        ("first-live".to_string(), bound("first-live")),
        ("third".to_string(), bound("third")),
    ]);

    let typescript = compile_typescript_composition(
        std::slice::from_ref(&program),
        &lives,
        &manifest,
        Some(TypeScriptCompositionConfig {
            live_endpoints: live_urls
                .iter()
                .map(|(alias, url)| {
                    (
                        alias.clone(),
                        TypeScriptLiveEndpoints {
                            websocket_url: Some(url.clone()),
                            http_url: None,
                        },
                    )
                })
                .collect(),
            ..TypeScriptCompositionConfig::default()
        }),
    )
    .unwrap();
    for live in &typescript.live_stacks {
        let definition = &live.output.stack_definition;
        if live_urls.contains_key(&live.alias) {
            assert!(
                definition.contains(&format!(
                    "  release: {{\n    stackManifestHash: '{hash}',\n    liveAlias: '{}',\n  }},",
                    live.alias
                )),
                "{definition}"
            );
        } else {
            assert!(names_no_release(definition), "{definition}");
        }
    }

    let rust = compile_rust_composition(
        std::slice::from_ref(&program),
        &lives,
        &manifest,
        Some(RustCompositionConfig {
            live_urls: live_urls.clone(),
            ..RustCompositionConfig::default()
        }),
    )
    .unwrap();
    for live in &rust.live_stacks {
        let entity = &live.output.entity_rs;
        if live_urls.contains_key(&live.alias) {
            assert!(entity.contains(&format!(
                "fn stack_manifest_hash() -> Option<&'static str> {{\n        Some(\"{hash}\")\n    }}"
            )));
            assert!(entity.contains(&format!(
                "fn live_alias() -> Option<&'static str> {{\n        Some(\"{}\")\n    }}",
                live.alias
            )));
        } else {
            assert!(names_no_release(entity), "{entity}");
        }
    }

    let python = compile_python_composition(
        std::slice::from_ref(&program),
        &lives,
        &manifest,
        Some(PythonCompositionConfig {
            live_urls: live_urls.clone(),
            ..PythonCompositionConfig::default()
        }),
    )
    .unwrap();
    for live in &python.live_stacks {
        let init = &live.output.init_py;
        if live_urls.contains_key(&live.alias) {
            assert!(
                init.contains("from arete.stack import StackDef, StackEndpoints, StackRelease\n")
            );
            assert!(init.contains(&format!(
                "    release=StackRelease(\n        stack_manifest_hash=\"{hash}\",\n        live_alias=\"{}\",\n    ),\n)",
                live.alias
            )));
        } else {
            assert!(names_no_release(init), "{init}");
        }
    }

    // Without served endpoints (local generation) no alias names a version.
    let local =
        compile_typescript_composition(std::slice::from_ref(&program), &lives, &manifest, None)
            .unwrap();
    assert!(local
        .live_stacks
        .iter()
        .all(|live| names_no_release(&live.output.stack_definition)));
}
