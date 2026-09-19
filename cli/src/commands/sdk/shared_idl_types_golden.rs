//! Golden SDK output for stacks whose entities map same-named IDL types.
//!
//! A stack SDK declares every entity in one module, so each IDL type its
//! entities map must be declared there once:
//!
//! - `identical-type`: programs `alpha` and `beta` both define the same
//!   `Header`, each mapped by one entity. Every SDK declares `Header` once.
//! - `conflicting-type`: `beta` defines a different `Header`. It is declared
//!   as `BetaHeader`, the `beta` entity references it, and `beta`'s
//!   instructions encode their own `Header` layout.
//! - `subscriptions`: the arete-artifacts `subscriptions` stack
//!   (`cli/tests/fixtures/subscriptions-stack/`), whose three entities map the
//!   same `header` type. a4 0.19.0 declared `HeaderSchema` three times and
//!   0.20.2 refused to generate its TypeScript SDK.
//!
//! Regenerate with `A4_UPDATE_GOLDEN=1 cargo test -p a4-cli shared_idl_types_golden`.
//! The TypeScript goldens are type-checked in CI
//! (`cli/tests/golden/shared-idl-types/tsconfig.json`).

use super::stack_name_golden::{
    assert_syntax, collect_files, compare_with_golden, local_stack, without_release_version,
};
use super::*;
use arete_artifacts::{live_spec_v2, PortableEntity, ProgramSpecArtifact};

const ALPHA_PROGRAM_ID: &str = "2c35Vf2AKSi7mTvaNdhSrgE3ppGAEyeSSLWNRkxbrQQM";
const BETA_PROGRAM_ID: &str = "Br9jAU97qteFboeqv34ph8XTsLnfCPTaZ8NepqqeLzDS";

fn golden_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/shared-idl-types")
}

/// `(name, IDL type, resolved base type, integer kind)` of a `Header` field.
type HeaderField = (
    &'static str,
    &'static str,
    &'static str,
    Option<&'static str>,
);

const HEADER: &[HeaderField] = &[
    ("version", "u8", "Integer", Some("U8")),
    ("owner", "publicKey", "Pubkey", None),
];

const WIDER_HEADER: &[HeaderField] = &[
    ("version", "u8", "Integer", Some("U8")),
    ("owner", "publicKey", "Pubkey", None),
    ("flags", "u16", "Integer", Some("U16")),
];

/// A program whose `Vault` account and `configure` instruction both use its
/// `Header` type.
fn program(name: &str, program_id: &str, header: &[HeaderField]) -> ProgramSpecArtifact {
    let header_fields = header
        .iter()
        .map(|(field, idl_type, _, _)| serde_json::json!({ "name": field, "type": idl_type }))
        .collect::<Vec<_>>();
    let idl = serde_json::json!({
        "version": "0.1.0",
        "name": name,
        "address": program_id,
        "metadata": { "address": program_id, "name": name, "version": "0.1.0", "spec": "0.1.0" },
        "instructions": [{
            "name": "configure",
            "discriminant": { "type": "u8", "value": 0 },
            "accounts": [
                { "name": "authority", "isMut": true, "isSigner": true },
                { "name": "vault", "isMut": true, "isSigner": false }
            ],
            "args": [{ "name": "header", "type": { "defined": "Header" } }]
        }],
        "accounts": [{
            "name": "Vault",
            "discriminator": [1, 0, 0, 0, 0, 0, 0, 0],
            "type": {
                "kind": "struct",
                "fields": [
                    { "name": "authority", "type": "publicKey" },
                    { "name": "header", "type": { "defined": "Header" } }
                ]
            }
        }],
        "types": [{ "name": "Header", "type": { "kind": "struct", "fields": header_fields } }],
        "events": [],
        "errors": []
    });
    let spec = arete_hash::build_program_spec_v1_from_bytes(idl.to_string().as_bytes(), None)
        .expect("golden ProgramSpecV1");
    ProgramSpecArtifact::new(spec).expect("golden ProgramSpec artifact")
}

/// An entity of `program_id` whose `state.header` field maps its `Header`.
fn entity(name: &str, program_id: &str, header: &[HeaderField]) -> PortableEntity {
    let resolved_fields = header
        .iter()
        .map(|(field, idl_type, base_type, integer_kind)| {
            let mut resolved = serde_json::json!({
                "field_name": field,
                "field_type": idl_type,
                "base_type": base_type,
                "is_optional": false,
                "is_array": false
            });
            if let Some(integer_kind) = integer_kind {
                resolved["integer_kind"] = serde_json::json!(integer_kind);
            }
            resolved
        })
        .collect::<Vec<_>>();
    let mut entity = PortableEntity::new(name, "id.address");
    entity.program_id = Some(program_id.to_string());
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
            "name": "state",
            "fields": [{
                "field_name": "header",
                "rust_type_name": "Header",
                "base_type": "Object",
                "is_optional": true,
                "is_array": false,
                "resolved_type": {
                    "type_name": "Header",
                    "fields": resolved_fields,
                    "is_instruction": false,
                    "is_account": false,
                    "is_event": false,
                    "is_enum": false,
                    "enum_variants": []
                }
            }]
        }
    ]))
    .expect("golden entity sections");
    entity
}

/// Programs `alpha` and `beta`, each mapped by one entity (`AlphaVault`,
/// `BetaVault`); `beta`'s `Header` is `beta_header`.
fn two_program_stack(beta_header: &[HeaderField]) -> ResolvedStackSource {
    let alpha = program("alpha", ALPHA_PROGRAM_ID, HEADER);
    let beta = program("beta", BETA_PROGRAM_ID, beta_header);
    let live = live_spec_v2(
        &[alpha.clone(), beta.clone()],
        vec![
            entity("AlphaVault", ALPHA_PROGRAM_ID, HEADER),
            entity("BetaVault", BETA_PROGRAM_ID, beta_header),
        ],
        Vec::new(),
    )
    .expect("golden LiveSpec");
    ResolvedStackSource::LocalArtifacts(Box::new(local_stack(
        "HeaderStream",
        vec![alpha, beta],
        vec![("live".to_string(), live)],
    )))
}

fn subscriptions_stack() -> ResolvedStackSource {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/subscriptions-stack/SubscriptionsStream.stack-manifest.json");
    ResolvedStackSource::LocalArtifacts(Box::new(
        load_local_artifact_stack(&manifest).expect("subscriptions fixture"),
    ))
}

fn generate_typescript(source: &ResolvedStackSource, output: &Path) -> Result<()> {
    let none = BTreeMap::new();
    generate_typescript_sdk_from_source(
        source,
        &output.join("typescript"),
        "@usearete/sdk",
        None,
        None,
        None,
        &none,
        &none,
        false,
    )
}

fn generate_all(source: &ResolvedStackSource, output: &Path) -> Result<()> {
    generate_typescript(source, output)?;
    generate_rust_stack_sdk(
        source,
        source.load_stack_spec(true)?,
        &output.join("rust"),
        &default_rust_crate_name(source.sdk_name()),
        false,
        None,
        None,
    )?;
    generate_python_stack_sdk(
        source,
        source.load_stack_spec(true)?,
        &output.join("python"),
        &format!("{}-stack", source.sdk_name()),
        false,
        None,
        None,
    )
}

/// Generate with `generate` and compare with the `name` golden; returns the
/// generated files.
fn assert_golden(
    name: &str,
    source: &ResolvedStackSource,
    generate: fn(&ResolvedStackSource, &Path) -> Result<()>,
) -> BTreeMap<String, String> {
    let temp = tempfile::tempdir().unwrap();
    generate(source, temp.path())
        .unwrap_or_else(|error| panic!("generate SDKs for '{name}': {error:#}"));
    let generated = collect_files(temp.path());
    assert_syntax(name, temp.path(), &generated);
    let generated = without_release_version(generated);
    compare_with_golden(name, &golden_root().join(name), generated.clone());
    generated
}

/// Occurrences of `needle` in the generated file `path`.
fn count(files: &BTreeMap<String, String>, path: &str, needle: &str) -> usize {
    files
        .get(path)
        .unwrap_or_else(|| panic!("{path} was not generated"))
        .matches(needle)
        .count()
}

#[test]
fn shared_idl_types_golden_identical_type() {
    let files = assert_golden("identical-type", &two_program_stack(HEADER), generate_all);

    let core = "typescript/HeaderStream-core.ts";
    assert_eq!(count(&files, core, "export interface Header {"), 1);
    assert_eq!(count(&files, core, "export const HeaderSchema ="), 1);
    assert_eq!(count(&files, core, "export const HeaderPatchSchema ="), 1);
    // Both entities reference the one declaration.
    assert_eq!(count(&files, core, "header: Header | null;"), 2);
    assert_eq!(count(&files, "rust/src/types.rs", "pub struct Header {"), 1);
    assert_eq!(
        count(
            &files,
            "python/headerstream_stack/models.py",
            "class Header:"
        ),
        1
    );
}

#[test]
fn shared_idl_types_golden_conflicting_type() {
    let files = assert_golden(
        "conflicting-type",
        &two_program_stack(WIDER_HEADER),
        generate_all,
    );

    // `alpha` keeps `Header`; `beta`'s different `Header` is `BetaHeader`, and
    // the `BetaVault` entity is typed against it.
    let core = "typescript/HeaderStream-core.ts";
    assert_eq!(count(&files, core, "export interface Header {"), 1);
    assert_eq!(count(&files, core, "export const HeaderSchema ="), 1);
    assert_eq!(count(&files, core, "export interface BetaHeader {"), 1);
    assert_eq!(count(&files, core, "export const BetaHeaderSchema ="), 1);
    assert_eq!(count(&files, core, "header: BetaHeader | null;"), 1);
    assert_eq!(count(&files, core, "header: Header | null;"), 1);
    assert_eq!(
        count(&files, "rust/src/types.rs", "pub struct BetaHeader {"),
        1
    );
    assert_eq!(
        count(
            &files,
            "rust/src/types.rs",
            "pub header: Option<Option<BetaHeader>>,"
        ),
        1
    );
    let models = "python/headerstream_stack/models.py";
    assert_eq!(count(&files, models, "class Header:"), 1);
    assert_eq!(count(&files, models, "class BetaHeader:"), 1);

    // Each program's `configure` encodes its own `Header` layout.
    for path in [
        core,
        "rust/src/programs.rs",
        "python/headerstream_stack/programs.py",
    ] {
        assert!(
            files[path].contains("flags"),
            "{path} must encode beta's `Header.flags`"
        );
    }
    assert_eq!(
        count(
            &files,
            core,
            "{ struct: [{ name: 'version', type: 'u8' }, { name: 'owner', type: 'pubkey' }] }"
        ),
        1,
        "alpha's configure keeps alpha's layout"
    );
    assert_eq!(
        count(
            &files,
            core,
            "{ struct: [{ name: 'version', type: 'u8' }, { name: 'owner', type: 'pubkey' }, { name: 'flags', type: 'u16' }] }"
        ),
        1,
        "beta's configure encodes beta's layout"
    );
}

#[test]
fn shared_idl_types_golden_subscriptions() {
    let files = assert_golden("subscriptions", &subscriptions_stack(), generate_typescript);

    let core = "typescript/SubscriptionsStream-core.ts";
    assert_eq!(count(&files, core, "export interface Header {"), 1);
    assert_eq!(count(&files, core, "export const HeaderSchema ="), 1);
    assert_eq!(count(&files, core, "export const HeaderPatchSchema ="), 1);
    // All three entities that map `header` reference the one declaration.
    assert_eq!(
        count(&files, core, "header: HeaderSchema.nullable().optional(),"),
        3
    );
}
