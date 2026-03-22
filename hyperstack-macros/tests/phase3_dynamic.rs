mod support;

use std::path::PathBuf;

use support::{cargo_toml, escape_path, macro_manifest_dir, TempCrate};

fn compile_failure_stderr(name: &str, source: &str) -> String {
    let manifest_dir = macro_manifest_dir();
    let temp_crate = TempCrate::new(
        "phase3-dynamic",
        name,
        cargo_toml(
            name,
            &[format!(
                "hyperstack-macros = {{ path = \"{}\" }}",
                escape_path(&manifest_dir)
            )],
        ),
        source,
        &[],
    );

    let output = temp_crate.cargo_check();

    assert!(
        !output.status.success(),
        "expected cargo check to fail for {name}"
    );

    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn pump_idl_path() -> String {
    escape_path(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("workspace root")
            .join("hyperstack-idl/tests/fixtures/pump.json"),
    )
}

#[test]
fn missing_instruction_error_points_to_from_path() {
    let source = format!(
        r#"use hyperstack_macros::hyperstack;

#[hyperstack(idl = "{}")]
mod broken {{
    #[entity(name = "Thing")]
    struct Thing {{
        #[event(
            from = pump_sdk::instructions::Initialise,
            fields = [user]
        )]
        trades: Vec<pump_sdk::instructions::Buy>,
    }}
}}

fn main() {{}}
"#,
        pump_idl_path()
    );

    let stderr = compile_failure_stderr("missing_instruction_error_points_to_from_path", &source);
    assert!(stderr.contains("Not found: 'Initialise' in instructions"));
    assert!(stderr.contains("src/main.rs:8:"), "stderr was:\n{stderr}");
}

#[test]
fn missing_instruction_field_error_points_to_field_token() {
    let source = format!(
        r#"use hyperstack_macros::hyperstack;

#[hyperstack(idl = "{}")]
mod broken {{
    #[entity(name = "Thing")]
    struct Thing {{
        #[event(
            from = pump_sdk::instructions::Buy,
            fields = [usr]
        )]
        trades: Vec<pump_sdk::instructions::Buy>,
    }}
}}

fn main() {{}}
"#,
        pump_idl_path()
    );

    let stderr = compile_failure_stderr(
        "missing_instruction_field_error_points_to_field_token",
        &source,
    );
    assert!(stderr.contains("Not found: 'usr' in instruction fields for 'buy'"));
    assert!(stderr.contains("src/main.rs:9:"), "stderr was:\n{stderr}");
}

#[test]
fn missing_account_error_points_to_map_path() {
    let source = format!(
        r#"use hyperstack_macros::hyperstack;

#[hyperstack(idl = "{}")]
mod broken {{
    #[entity(name = "Thing")]
    struct Thing {{
        #[map(
            pump_sdk::accounts::BondingCurv::complete,
            strategy = LastWrite
        )]
        complete: bool,
    }}
}}

fn main() {{}}
"#,
        pump_idl_path()
    );

    let stderr = compile_failure_stderr("missing_account_error_points_to_map_path", &source);
    assert!(stderr.contains("Not found: 'BondingCurv' in accounts"));
    assert!(stderr.contains("src/main.rs:8:"), "stderr was:\n{stderr}");
}
