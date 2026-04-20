mod support;

use std::path::PathBuf;

use support::{cargo_toml, escape_path, macro_manifest_dir, TempCrate};

fn compile_failure_stderr(name: &str, source: &str) -> String {
    let manifest_dir = macro_manifest_dir();
    let temp_crate = TempCrate::new(
        "phase5-dynamic",
        name,
        cargo_toml(
            name,
            &[format!(
                "arete-macros = {{ path = \"{}\" }}",
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
            .join("arete-idl/tests/fixtures/pump.json"),
    )
}

#[test]
fn invalid_strategy_suggests_valid_value() {
    let source = r#"use arete_macros::arete;

#[arete]
mod broken {
    #[entity(name = "Thing")]
    struct Thing {
        #[map(fake_sdk::accounts::Thing::value, strategy = LastWrit)]
        value: u64,
    }
}

fn main() {}
"#;

    let stderr = compile_failure_stderr("invalid_strategy_suggests_valid_value", source);
    assert!(stderr.contains("invalid strategy 'LastWrit' for #[map]"));
    assert!(stderr.contains("Expected one of: SetOnce, LastWrite"));
    assert!(stderr.contains("Did you mean: LastWrite?"));
}

#[test]
fn unknown_resolver_suggests_valid_name() {
    let source = r#"use arete_macros::arete;

#[arete]
mod broken {
    #[entity(name = "Thing")]
    struct Thing {
        existing: String,
        #[resolve(from = "existing", resolver = Toke)]
        metadata: String,
    }
}

fn main() {}
"#;

    let stderr = compile_failure_stderr("unknown_resolver_suggests_valid_name", source);
    assert!(stderr.contains("unknown resolver 'Toke'"));
    assert!(stderr.contains("Did you mean: Token?"));
}

#[test]
fn unknown_pda_program_suggests_available_name() {
    let source = format!(
        r#"use arete_macros::arete;

#[arete(idl = "{}")]
mod broken {{
    #[entity(name = "Thing")]
    struct Thing {{}}

    pdas! {{
        pum {{
            broken = [literal("broken")];
        }}
    }}
}}

fn main() {{}}
"#,
        pump_idl_path()
    );

    let stderr = compile_failure_stderr("unknown_pda_program_suggests_available_name", &source);
    assert!(stderr.contains("unknown program 'pum' in pdas! block"));
    assert!(stderr.contains("Did you mean: pump?"));
}

#[test]
fn empty_url_template_field_is_rejected() {
    let source = r#"use arete_macros::arete;

#[arete]
mod broken {
    #[entity(name = "Thing")]
    struct Thing {
        #[resolve(url = "https://example.com/{   }/metadata", extract = "name")]
        metadata: String,
    }
}

fn main() {}
"#;

    let stderr = compile_failure_stderr("empty_url_template_field_is_rejected", source);
    assert!(stderr.contains("Empty field reference '{}' in URL template"));
}
