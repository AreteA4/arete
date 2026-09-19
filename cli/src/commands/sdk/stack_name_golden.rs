//! Golden SDK output keyed by stack name.
//!
//! Every SDK flavour `a4 sdk create --manifest` can emit (TypeScript stack and
//! program collection, Rust crate, Python package, TypeScript composition) is
//! generated from one small artifact closure under several stack names and
//! compared byte-for-byte with `cli/tests/golden/stack-names/<name>/`.
//! Regenerate with `A4_UPDATE_GOLDEN=1 cargo test -p a4-cli stack_name_golden`.
//!
//! The TypeScript goldens are type-checked in CI
//! (`cli/tests/golden/stack-names/tsconfig.json`).

use super::*;
use arete_artifacts::{
    compose_stack_manifest_v2, live_spec_v2, LiveSpecArtifactV2, PortableEntity,
    ProgramSpecArtifact, SelectedViewV2, StackManifestArtifactV2,
};

const VAULT_PROGRAM_ID: &str = "2c35Vf2AKSi7mTvaNdhSrgE3ppGAEyeSSLWNRkxbrQQM";

fn golden_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/stack-names")
}

fn program(name: &str, program_id: &str) -> ProgramSpecArtifact {
    let idl = format!(
        r#"{{
          "version": "0.1.0",
          "name": "{name}",
          "address": "{program_id}",
          "metadata": {{ "address": "{program_id}", "name": "{name}", "version": "0.1.0", "spec": "0.1.0" }},
          "instructions": [
            {{
              "name": "deposit",
              "discriminant": {{ "type": "u8", "value": 0 }},
              "accounts": [
                {{ "name": "authority", "isMut": true, "isSigner": true }},
                {{ "name": "vault", "isMut": true, "isSigner": false }}
              ],
              "args": [{{ "name": "amount", "type": "u64" }}]
            }}
          ],
          "accounts": [
            {{
              "name": "Vault",
              "discriminator": [1, 0, 0, 0, 0, 0, 0, 0],
              "type": {{
                "kind": "struct",
                "fields": [
                  {{ "name": "authority", "type": "publicKey" }},
                  {{ "name": "balance", "type": "u64" }}
                ]
              }}
            }}
          ],
          "types": [],
          "events": [],
          "errors": [{{ "code": 0, "name": "AmountTooSmall", "msg": "Amount too small" }}]
        }}"#
    );
    let spec = arete_hash::build_program_spec_v1_from_bytes(idl.as_bytes(), None)
        .expect("golden ProgramSpecV1");
    ProgramSpecArtifact::new(spec).expect("golden ProgramSpec artifact")
}

fn entity(name: &str) -> PortableEntity {
    let mut entity = PortableEntity::new(name, "id.address");
    entity.sections = serde_json::from_value(serde_json::json!([
        {
            "name": "id",
            "fields": [{
                "field_name": "address",
                "rust_type_name": "String",
                "base_type": "Pubkey",
                "is_optional": false,
                "is_array": false
            }]
        },
        {
            "name": "balance",
            "fields": [{
                "field_name": "amount",
                "rust_type_name": "u64",
                "base_type": "Integer",
                "is_optional": true,
                "is_array": false
            }]
        }
    ]))
    .expect("golden entity sections");
    entity
}

fn selected_views(alias: &str, live: &LiveSpecArtifactV2) -> Vec<SelectedViewV2> {
    live.payload
        .entities
        .iter()
        .flat_map(|entity| &entity.views)
        .map(|view| SelectedViewV2 {
            live_alias: alias.to_string(),
            view_id: view.id.clone(),
        })
        .collect()
}

fn local_stack(
    name: &str,
    programs: Vec<ProgramSpecArtifact>,
    lives: Vec<(String, LiveSpecArtifactV2)>,
) -> LocalArtifactStack {
    let selected = lives
        .iter()
        .flat_map(|(alias, live)| selected_views(alias, live))
        .collect();
    let stack_manifest: StackManifestArtifactV2 = compose_stack_manifest_v2(
        name,
        &programs,
        lives
            .iter()
            .map(|(alias, live)| (alias.clone(), live))
            .collect(),
        selected,
    )
    .expect("golden StackManifest");
    LocalArtifactStack {
        manifest_path: PathBuf::from(format!("{name}.stack-manifest.json")),
        manifest_hash: stack_manifest.artifact_hash.to_string(),
        program_specs: programs,
        live_specs: lives,
        stack_manifest,
    }
}

fn single_live_stack(name: &str, entity_name: &str) -> LocalArtifactStack {
    let program = program("vault", VAULT_PROGRAM_ID);
    let live = live_spec_v2(
        std::slice::from_ref(&program),
        vec![entity(entity_name)],
        Vec::new(),
    )
    .expect("golden LiveSpec");
    local_stack(name, vec![program], vec![("live".to_string(), live)])
}

fn composed_stack(name: &str) -> LocalArtifactStack {
    let program = program("vault", VAULT_PROGRAM_ID);
    let live = live_spec_v2(
        std::slice::from_ref(&program),
        vec![entity("Vault")],
        Vec::new(),
    )
    .expect("golden LiveSpec");
    local_stack(
        name,
        vec![program],
        vec![
            ("alpha".to_string(), live.clone()),
            ("beta".to_string(), live),
        ],
    )
}

/// Generate every SDK flavour for `name` into `output`.
fn generate_all(name: &str, output: &Path) -> Result<()> {
    let source = ResolvedStackSource::LocalArtifacts(Box::new(single_live_stack(name, "Vault")));
    let none = BTreeMap::new();
    generate_typescript_sdk_from_source(
        &source,
        &output.join("typescript"),
        "@usearete/sdk",
        None,
        None,
        None,
        &none,
        &none,
        false,
    )?;
    generate_typescript_sdk_from_source(
        &source,
        &output.join("typescript-programs"),
        "@usearete/sdk",
        None,
        None,
        None,
        &none,
        &none,
        true,
    )?;
    let crate_name = default_rust_crate_name(source.sdk_name());
    generate_rust_stack_sdk(
        &source,
        source.load_stack_spec(true)?,
        &output.join("rust"),
        &crate_name,
        false,
        None,
        None,
    )?;
    generate_python_stack_sdk(
        &source,
        source.load_stack_spec(true)?,
        &output.join("python"),
        &format!("{}-stack", source.sdk_name()),
        false,
        None,
        None,
    )?;

    let composed = composed_stack(name);
    let source = ResolvedStackSource::LocalArtifacts(Box::new(composed.clone()));
    generate_typescript_composition_sdk(
        &source,
        &composed.program_specs,
        &composed.live_specs,
        &composed.stack_manifest,
        &output.join("typescript-composition"),
        "@usearete/sdk",
        None,
        None,
        None,
        &none,
        &none,
    )
}

/// Relative path -> contents for every generated file. Provenance manifests
/// embed the compiler hash, which changes with every CLI build, so they are
/// not part of the golden output.
fn collect_files(root: &Path) -> BTreeMap<String, String> {
    fn walk(root: &Path, directory: &Path, files: &mut BTreeMap<String, String>) {
        let mut entries = fs::read_dir(directory)
            .unwrap_or_else(|error| panic!("read {}: {error}", directory.display()))
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                walk(root, &path, files);
            } else if path.file_name().and_then(|name| name.to_str()) != Some(SDK_PROVENANCE_FILE) {
                let relative = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                files.insert(relative, fs::read_to_string(&path).unwrap());
            }
        }
    }
    let mut files = BTreeMap::new();
    if root.is_dir() {
        walk(root, root, &mut files);
    }
    files
}

/// Every generated Rust file must parse and every Python file must compile.
/// (TypeScript is type-checked with `tsc` in CI.)
fn assert_syntax(name: &str, root: &Path, files: &BTreeMap<String, String>) {
    for (relative, contents) in files {
        if relative.ends_with(".rs") {
            if let Err(error) = syn::parse_file(contents) {
                panic!("{relative} for stack '{name}' is not valid Rust: {error}\n{contents}");
            }
        }
    }
    let python_files = files
        .keys()
        .filter(|relative| relative.ends_with(".py"))
        .map(|relative| root.join(relative))
        .collect::<Vec<_>>();
    if python_files.is_empty() {
        return;
    }
    let python = std::env::var_os("PYTHON").unwrap_or_else(|| "python3".into());
    let compiled = std::process::Command::new(python)
        .args([
            "-c",
            "import pathlib, sys\nfor path in sys.argv[1:]:\n    compile(pathlib.Path(path).read_text(), path, 'exec')",
        ])
        .args(&python_files)
        .output()
        .expect("Python must be available for generated syntax checks");
    assert!(
        compiled.status.success(),
        "generated Python for stack '{name}' failed to compile:\n{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
}

fn assert_golden(name: &str) {
    let temp = tempfile::tempdir().unwrap();
    generate_all(name, temp.path())
        .unwrap_or_else(|error| panic!("generate SDKs for stack '{name}': {error:#}"));
    let generated = collect_files(temp.path());
    assert_syntax(name, temp.path(), &generated);
    let golden_dir = golden_root().join(name);

    if std::env::var_os("A4_UPDATE_GOLDEN").is_some() {
        let _ = fs::remove_dir_all(&golden_dir);
        for (relative, contents) in &generated {
            let path = golden_dir.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        }
        return;
    }

    let expected = collect_files(&golden_dir);
    assert_eq!(
        generated.keys().collect::<Vec<_>>(),
        expected.keys().collect::<Vec<_>>(),
        "generated file set for stack '{name}' differs from {}",
        golden_dir.display()
    );
    for (relative, contents) in &generated {
        assert!(
            contents == &expected[relative],
            "{relative} for stack '{name}' differs from {}; rerun with A4_UPDATE_GOLDEN=1 and review the diff",
            golden_dir.join(relative).display()
        );
    }
}

// Names that already generated valid SDKs. Their output must never change.

#[test]
fn stack_name_golden_pascal_case() {
    assert_golden("VaultStream");
}

#[test]
fn stack_name_golden_snake_case() {
    assert_golden("vault_stream");
}

#[test]
fn stack_name_golden_lowercase() {
    assert_golden("vault");
}

// Names that used to produce invalid identifiers (`TOKEN-BALANCES_STACK_CORE`,
// `my.stackStack`, `9LIVES_STACK`). File names keep the stack name as written.

#[test]
fn stack_name_golden_hyphenated() {
    assert_golden("token-balances");
}

#[test]
fn stack_name_golden_dotted() {
    assert_golden("my.stack");
}

#[test]
fn stack_name_golden_leading_digit() {
    assert_golden("9lives");
}

fn expect_error(result: Result<()>) -> String {
    format!(
        "{:#}",
        result.expect_err("generation must report the collision")
    )
}

/// `my-vault` sanitizes to `MyVault`, so its `MyVaultStack` export collides
/// with an entity named `MyVaultStack`. Generation must fail instead of
/// letting one declaration shadow the other.
#[test]
fn stack_name_collision_after_sanitizing_is_reported() {
    let source = ResolvedStackSource::LocalArtifacts(Box::new(single_live_stack(
        "my-vault",
        "MyVaultStack",
    )));
    let temp = tempfile::tempdir().unwrap();
    let none = BTreeMap::new();

    let typescript = temp.path().join("typescript");
    let error = expect_error(generate_typescript_sdk_from_source(
        &source,
        &typescript,
        "@usearete/sdk",
        None,
        None,
        None,
        &none,
        &none,
        false,
    ));
    assert!(error.contains("`MyVaultStack`"), "{error}");
    assert!(
        collect_files(&typescript).is_empty(),
        "nothing may be written when a collision is reported"
    );

    let error = expect_error(generate_rust_stack_sdk(
        &source,
        source.load_stack_spec(true).unwrap(),
        &temp.path().join("rust"),
        "my-vault-stack",
        false,
        None,
        None,
    ));
    assert!(error.contains("`MyVaultStack`"), "{error}");
    assert!(error.contains("entity 'MyVaultStack'"), "{error}");
    assert!(error.contains("stack name 'my-vault'"), "{error}");
}

/// Two programs whose names differ only by separators generate the same
/// program key and constants.
#[test]
fn program_name_collision_after_sanitizing_is_reported() {
    let first = program("token-2022", "EsLvpLQPuFfHUoXZQuni57bGRqTXQshLGicqA4UdP8sQ");
    let second = program("token_2022", "Br9jAU97qteFboeqv34ph8XTsLnfCPTaZ8NepqqeLzDS");
    let live = live_spec_v2(
        &[first.clone(), second.clone()],
        vec![entity("Vault")],
        Vec::new(),
    )
    .expect("LiveSpec");
    let source = ResolvedStackSource::LocalArtifacts(Box::new(local_stack(
        "token-balances",
        vec![first, second],
        vec![("live".to_string(), live)],
    )));
    let temp = tempfile::tempdir().unwrap();
    let none = BTreeMap::new();
    for program_only in [false, true] {
        let error = expect_error(generate_typescript_sdk_from_source(
            &source,
            &temp.path().join(format!("typescript-{program_only}")),
            "@usearete/sdk",
            None,
            None,
            None,
            &none,
            &none,
            program_only,
        ));
        assert!(error.contains("'token2022'"), "{error}");
        assert!(error.contains("'token-2022'"), "{error}");
        assert!(error.contains("'token_2022'"), "{error}");
    }
}

/// Stack names are also file names; a name that is really a path must not
/// write outside the output directory.
#[test]
fn stack_names_that_are_paths_are_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let none = BTreeMap::new();
    for name in ["../escape", "/tmp/escape", "a\\b", "c:escape", ".."] {
        let source =
            ResolvedStackSource::LocalArtifacts(Box::new(single_live_stack(name, "Vault")));
        let output = temp.path().join("out").join("typescript");
        let error = expect_error(generate_typescript_sdk_from_source(
            &source,
            &output,
            "@usearete/sdk",
            None,
            None,
            None,
            &none,
            &none,
            false,
        ));
        assert!(
            error.contains("cannot be used as a generated file name"),
            "{error}"
        );
        let error = expect_error(generate_rust_stack_sdk(
            &source,
            source.load_stack_spec(true).unwrap(),
            &temp.path().join("out").join("rust"),
            "escape-stack",
            false,
            None,
            None,
        ));
        assert!(
            error.contains("cannot be used as a generated file name"),
            "{error}"
        );
    }
    assert!(
        collect_files(temp.path()).is_empty(),
        "nothing may be written for a path-like stack name"
    );
}

/// A program-only Rust crate re-exports its `<Stack>Programs` aggregate next
/// to every generated model (`pub use types::*`); a model with that name must
/// be reported instead of being silently shadowed.
#[test]
fn rust_program_aggregate_collision_is_reported() {
    let source =
        ResolvedStackSource::LocalArtifacts(Box::new(single_live_stack("vault", "VaultPrograms")));
    let spec = source.load_stack_spec(false).unwrap();
    let error = arete_interpreter::rust::compile_program_modules(spec, None).unwrap_err();
    assert!(error.contains("`VaultPrograms`"), "{error}");
    assert!(error.contains("entity 'VaultPrograms'"), "{error}");
    assert!(error.contains("stack name 'vault'"), "{error}");
}
