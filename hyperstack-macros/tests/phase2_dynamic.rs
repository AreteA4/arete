mod support;

use std::path::PathBuf;

use support::{cargo_toml, escape_path, macro_manifest_dir, TempCrate};

fn run_compile_failure(name: &str, source: &str, expected: &[&str]) {
    let manifest_dir = macro_manifest_dir();
    let temp_crate = TempCrate::new(
        "phase2-dynamic",
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

    let stderr = String::from_utf8_lossy(&output.stderr);
    for needle in expected {
        assert!(
            stderr.contains(needle),
            "expected stderr for {name} to contain {needle:?}, got:\n{stderr}"
        );
    }
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
fn invalid_map_strategy_is_rejected_early() {
    let source = r#"use hyperstack_macros::hyperstack;

#[hyperstack]
mod broken {
    #[entity(name = "Thing")]
    struct Thing {
        #[map(fake_sdk::accounts::Thing::value, strategy = Invalid)]
        value: u64,
    }
}

fn main() {}
"#;

    run_compile_failure(
        "invalid_map_strategy_is_rejected_early",
        source,
        &["invalid strategy 'Invalid' for #[map]"],
    );
}

#[test]
fn invalid_condition_is_rejected_early() {
    let source = r#"use hyperstack_macros::hyperstack;

#[hyperstack]
mod broken {
    #[entity(name = "Thing")]
    struct Thing {
        #[aggregate(from = fake_sdk::instructions::Trade, field = amount, condition = "amount >> 1")]
        total: u64,
    }
}

fn main() {}
"#;

    run_compile_failure(
        "invalid_condition_is_rejected_early",
        source,
        &["Invalid condition expression 'amount >> 1'"],
    );
}

#[test]
fn missing_instruction_gets_suggestion() {
    let source = format!(
        r#"use hyperstack_macros::hyperstack;

#[hyperstack(idl = "{}")]
mod broken {{
    #[entity(name = "Thing")]
    struct Thing {{
        #[event(from = pump_sdk::instructions::Initialise, fields = [user])]
        trades: Vec<pump_sdk::instructions::Buy>,
    }}
}}

fn main() {{}}
"#,
        pump_idl_path()
    );

    run_compile_failure(
        "missing_instruction_gets_suggestion",
        &source,
        &[
            "Not found: 'Initialise' in instructions",
            "Did you mean: initialize?",
        ],
    );
}

#[test]
fn missing_instruction_field_gets_suggestion() {
    let source = format!(
        r#"use hyperstack_macros::hyperstack;

#[hyperstack(idl = "{}")]
mod broken {{
    #[entity(name = "Thing")]
    struct Thing {{
        #[event(from = pump_sdk::instructions::Buy, fields = [usr])]
        trades: Vec<pump_sdk::instructions::Buy>,
    }}
}}

fn main() {{}}
"#,
        pump_idl_path()
    );

    run_compile_failure(
        "missing_instruction_field_gets_suggestion",
        &source,
        &[
            "Not found: 'usr' in instruction fields for 'buy'",
            "Did you mean: user?",
        ],
    );
}

#[test]
fn event_join_on_field_is_validated_even_when_instruction_lookup_fails() {
    let source = format!(
        r#"use hyperstack_macros::hyperstack;

#[hyperstack(idl = "{}")]
mod broken {{
    #[entity(name = "Thing")]
    struct Thing {{
        id: String,

        #[event(from = pump_sdk::instructions::Initialise, fields = [user], join_on = ghost)]
        trades: Vec<pump_sdk::instructions::Buy>,
    }}
}}

fn main() {{}}
"#,
        pump_idl_path()
    );

    run_compile_failure(
        "event_join_on_field_is_validated_even_when_instruction_lookup_fails",
        &source,
        &[
            "Not found: 'Initialise' in instructions",
            "unknown join_on field 'ghost' on entity 'Thing'",
        ],
    );
}

#[test]
fn unknown_legacy_event_program_reports_invalid_path() {
    let source = format!(
        r#"use hyperstack_macros::hyperstack;

#[hyperstack(idl = "{}")]
mod broken {{
    #[entity(name = "Thing")]
    struct Thing {{
        #[map(pump_sdk::accounts::BondingCurve::complete, primary_key, strategy = SetOnce)]
        id: String,

        #[event(instruction = "unknown_program::Transfer", capture = [user], lookup_by = id)]
        trades: String,
    }}
}}

fn main() {{}}
"#,
        pump_idl_path()
    );

    run_compile_failure(
        "unknown_legacy_event_program_reports_invalid_path",
        &source,
        &["Invalid path 'unknown_program::Transfer'"],
    );
}

#[test]
fn missing_account_type_gets_suggestion() {
    let source = format!(
        r#"use hyperstack_macros::hyperstack;

#[hyperstack(idl = "{}")]
mod broken {{
    #[entity(name = "Thing")]
    struct Thing {{
        #[map(pump_sdk::accounts::BondingCurv::complete, strategy = LastWrite)]
        complete: bool,
    }}
}}

fn main() {{}}
"#,
        pump_idl_path()
    );

    run_compile_failure(
        "missing_account_type_gets_suggestion",
        &source,
        &[
            "Not found: 'BondingCurv' in accounts",
            "Did you mean: BondingCurve?",
        ],
    );
}
