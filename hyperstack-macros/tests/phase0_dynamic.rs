mod support;

use support::{cargo_toml, escape_path, macro_manifest_dir, TempCrate};

fn run_compile_failure(name: &str, source: &str, extra_files: &[(&str, &str)], expected: &[&str]) {
    let manifest_dir = macro_manifest_dir();
    let temp_crate = TempCrate::new(
        "phase0-dynamic",
        name,
        cargo_toml(
            name,
            &[format!(
                "hyperstack-macros = {{ path = \"{}\" }}",
                escape_path(&manifest_dir)
            )],
        ),
        source,
        extra_files,
    );

    let output = temp_crate.cargo_check();

    assert!(
        !output.status.success(),
        "expected cargo check to fail for {name}"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    for needle in expected {
        assert!(
            stderr.contains(needle),
            "expected stderr for {name} to contain {needle:?}, got:\n{stderr}"
        );
    }
}

fn run_idl_compile_failure(name: &str, module_body: &str, expected: &[&str]) {
    let source = r#"use hyperstack_macros::hyperstack;

#[hyperstack(idl = "fixture/minimal.json")]
mod broken {
__MODULE_BODY__
}

fn main() {}
"#
    .replace("__MODULE_BODY__", module_body);

    let minimal_idl = r#"{
  "name": "ore",
  "instructions": [],
  "accounts": [],
  "types": [],
  "events": [],
  "errors": [],
  "constants": []
}
"#;

    run_compile_failure(
        name,
        &source,
        &[("fixture/minimal.json", minimal_idl)],
        expected,
    );
}

#[test]
fn after_instruction_attribute_is_hard_error() {
    run_idl_compile_failure(
        "after_instruction_attribute_is_hard_error",
        "    #[after_instruction(ore_sdk::instructions::Initialize)]\n    fn bad() {}",
        &[
            "Direct use of #[after_instruction] is not allowed.",
            "Use declarative macros instead:",
        ],
    );
}

#[test]
fn resolve_key_attribute_is_hard_error() {
    run_idl_compile_failure(
        "resolve_key_attribute_is_hard_error",
        "    #[resolve_key(strategy = \"direct_field\")]\n    struct Marker;",
        &["#[resolve_key] requires 'account' parameter"],
    );
}

#[test]
fn register_pda_attribute_is_hard_error() {
    run_idl_compile_failure(
        "register_pda_attribute_is_hard_error",
        "    #[register_pda(instruction = ore_sdk::instructions::Initialize)]\n    struct Marker;",
        &["#[register_pda] requires 'pda_field' parameter"],
    );
}

#[test]
fn manual_pdas_block_is_hard_error() {
    run_idl_compile_failure(
        "manual_pdas_block_is_hard_error",
        "    #[entity(name = \"Thing\")]\n    struct Marker {}\n\n    pdas! {\n        missing_program {\n            broken = [literal(\"broken\")];\n        }\n    }",
        &["unknown program 'missing_program' in pdas! block"],
    );
}
