mod support;

use std::path::PathBuf;

use support::{cargo_toml, escape_path, macro_manifest_dir, TempCrate};

fn compile_failure_stderr(name: &str, source: &str) -> String {
    let manifest_dir = macro_manifest_dir();
    compile_failure_stderr_with_cargo(
        name,
        cargo_toml(
            name,
            &[format!(
                "arete-macros = {{ path = \"{}\" }}",
                escape_path(&manifest_dir)
            )],
        ),
        source,
    )
}

fn compile_failure_stderr_with_cargo(name: &str, cargo_toml: String, source: &str) -> String {
    let temp_crate = TempCrate::new("phase4-dynamic", name, cargo_toml, source, &[]);

    let output = temp_crate.cargo_check();

    assert!(
        !output.status.success(),
        "expected cargo check to fail for {name}"
    );

    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn assert_compile_success(name: &str, cargo_toml: String, source: &str) {
    let temp_crate = TempCrate::new("phase4-dynamic", name, cargo_toml, source, &[]);
    let output = temp_crate.cargo_check();

    assert!(
        output.status.success(),
        "expected cargo check to succeed for {name}, stderr was:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn pump_idl_path() -> String {
    escape_path(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("workspace root")
            .join("arete-idl/tests/fixtures/pump.json"),
    )
}

fn meteora_presale_idl_path() -> String {
    escape_path(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("workspace root")
            .parent()
            .expect("hypertek root")
            .join("arete-examples/idls/meteora-presale.idl.json"),
    )
}

#[test]
fn unknown_account_field_is_rejected() {
    let source = format!(
        r#"use arete_macros::arete;

#[arete(idl = "{}")]
mod broken {{
    #[entity(name = "Thing")]
    struct Thing {{
        #[map(pump_sdk::accounts::BondingCurve::bogus, strategy = LastWrite)]
        value: u64,
    }}
}}

fn main() {{}}
"#,
        pump_idl_path()
    );

    let stderr = compile_failure_stderr("unknown_account_field_is_rejected", &source);
    assert!(stderr.contains("Not found: 'bogus' in account fields for 'BondingCurve'"));
}

#[test]
fn account_field_validation_is_case_sensitive() {
    let source = format!(
        r#"use arete_macros::arete;

#[arete(idl = "{}")]
mod broken {{
    #[entity(name = "Thing")]
    struct Thing {{
        #[map(pump_sdk::accounts::BondingCurve::Complete, strategy = LastWrite)]
        value: bool,
    }}
}}

fn main() {{}}
"#,
        pump_idl_path()
    );

    let stderr = compile_failure_stderr("account_field_validation_is_case_sensitive", &source);
    assert!(stderr.contains("Not found: 'Complete' in account fields for 'BondingCurve'"));
}

#[test]
fn missing_computed_section_reference_is_rejected() {
    let source = r#"use arete_macros::arete;

#[arete]
mod broken {
    #[entity(name = "Thing")]
    struct Thing {
        base: u64,
        #[computed(ghost.value + 1)]
        total: u64,
    }
}

fn main() {}
"#;

    let stderr = compile_failure_stderr("missing_computed_section_reference_is_rejected", source);
    assert!(stderr.contains("unknown computed field reference 'ghost.value' on entity 'Thing'"));
}

#[test]
fn invalid_resolver_input_field_is_rejected() {
    let source = r#"use arete_macros::arete;

#[arete]
mod broken {
    #[entity(name = "Thing")]
    struct Thing {
        existing: String,
        #[resolve(from = "ghost.value", resolver = Token)]
        metadata: String,
    }
}

fn main() {}
"#;

    let stderr = compile_failure_stderr("invalid_resolver_input_field_is_rejected", source);
    assert!(stderr.contains("unknown resolver input field 'ghost.value' on entity 'Thing'"));
}

#[test]
fn invalid_resolver_condition_field_is_rejected() {
    let source = r#"use arete_macros::arete;

#[arete]
mod broken {
    #[entity(name = "Thing")]
    struct Thing {
        existing: String,
        #[resolve(from = "existing", resolver = Token, condition = "ghost.value == pending")]
        metadata: String,
    }
}

fn main() {}
"#;

    let stderr = compile_failure_stderr("invalid_resolver_condition_field_is_rejected", source);
    assert!(stderr.contains("unknown resolver condition field 'ghost.value' on entity 'Thing'"));
}

#[test]
fn invalid_view_sort_by_is_rejected() {
    let source = r#"use arete_macros::arete;

#[arete]
mod broken {
    #[entity(name = "Thing")]
    #[view(name = "latest", sort_by = "ghost.value")]
    struct Thing {
        base: u64,
    }
}

fn main() {}
"#;

    let stderr = compile_failure_stderr("invalid_view_sort_by_is_rejected", source);
    assert!(stderr.contains("unknown view field 'ghost.value' on entity 'Thing'"));
}

#[test]
fn computed_cycle_is_rejected() {
    let source = r#"use arete_macros::arete;

#[arete]
mod broken {
    #[entity(name = "Thing")]
    struct Thing {
        #[computed(b)]
        a: u64,
        #[computed(a)]
        b: u64,
    }
}

fn main() {}
"#;

    let stderr = compile_failure_stderr("computed_cycle_is_rejected", source);
    assert!(stderr.contains("computed fields contain a dependency cycle"));
}

#[test]
fn nested_computed_reference_into_idl_struct_compiles() {
    let manifest_dir = macro_manifest_dir();
    let arete_dir = manifest_dir.parent().expect("workspace root").join("arete");
    let source = format!(
        r#"use arete::prelude::*;
use arete_macros::arete;

#[arete(idl = "{}")]
mod good {{
    #![allow(non_snake_case)]

    use arete::macros::Stream;
    use serde::{{Deserialize, Serialize}};

    #[entity(name = "Thing")]
    struct Thing {{
        id: Id,
        lifecycle: Lifecycle,
        sale: Sale,
    }}

    #[derive(Debug, Clone, Serialize, Deserialize, Stream)]
    struct Id {{
        #[map(presale_sdk::instructions::initialize_presale::presale, primary_key, strategy = SetOnce)]
        presale_address: String,
    }}

    #[derive(Debug, Clone, Serialize, Deserialize, Stream)]
    struct Lifecycle {{
        #[map(presale_sdk::instructions::initialize_presale::params, strategy = SetOnce, emit = false)]
        initialize_params: Option<presale_sdk::types::InitializePresaleArgs>,
    }}

    #[derive(Debug, Clone, Serialize, Deserialize, Stream)]
    struct Sale {{
        #[computed(lifecycle.initialize_params.presale_params.disable_earlier_presale_end_once_cap_reached)]
        disable_earlier_presale_end_once_cap_reached: Option<u8>,
    }}
}}

fn main() {{}}
"#,
        meteora_presale_idl_path()
    );

    let cargo = cargo_toml(
        "nested-computed-reference-into-idl-struct-compiles",
        &[
            format!(
                "arete = {{ path = \"{}\", features = [\"full\"] }}",
                escape_path(&arete_dir)
            ),
            format!(
                "arete-macros = {{ path = \"{}\" }}",
                escape_path(&manifest_dir)
            ),
            "serde = { version = \"1.0\", features = [\"derive\"] }".to_string(),
            "borsh = { version = \"1.5\", features = [\"derive\"] }".to_string(),
            "solana-pubkey = { version = \"2.2\", features = [\"serde\", \"borsh\"] }".to_string(),
        ],
    );

    assert_compile_success(
        "nested_computed_reference_into_idl_struct_compiles",
        cargo,
        &source,
    );
}

#[test]
fn invalid_nested_computed_reference_is_rejected() {
    let manifest_dir = macro_manifest_dir();
    let arete_dir = manifest_dir.parent().expect("workspace root").join("arete");
    let source = format!(
        r#"use arete::prelude::*;
use arete_macros::arete;

#[arete(idl = "{}")]
mod broken {{
    #![allow(non_snake_case)]

    use arete::macros::Stream;
    use serde::{{Deserialize, Serialize}};

    #[entity(name = "Thing")]
    struct Thing {{
        id: Id,
        lifecycle: Lifecycle,
        sale: Sale,
    }}

    #[derive(Debug, Clone, Serialize, Deserialize, Stream)]
    struct Id {{
        #[map(presale_sdk::instructions::initialize_presale::presale, primary_key, strategy = SetOnce)]
        presale_address: String,
    }}

    #[derive(Debug, Clone, Serialize, Deserialize, Stream)]
    struct Lifecycle {{
        #[map(presale_sdk::instructions::initialize_presale::params, strategy = SetOnce, emit = false)]
        initialize_params: Option<presale_sdk::types::InitializePresaleArgs>,
    }}

    #[derive(Debug, Clone, Serialize, Deserialize, Stream)]
    struct Sale {{
        #[computed(lifecycle.initialize_params.presale_params.disable_earlier_presale_end_once_cap_reachd)]
        disable_earlier_presale_end_once_cap_reached: Option<u8>,
    }}
}}

fn main() {{}}
"#,
        meteora_presale_idl_path()
    );

    let cargo = cargo_toml(
        "invalid-nested-computed-reference-is-rejected",
        &[
            format!(
                "arete = {{ path = \"{}\", features = [\"full\"] }}",
                escape_path(&arete_dir)
            ),
            format!(
                "arete-macros = {{ path = \"{}\" }}",
                escape_path(&manifest_dir)
            ),
            "serde = { version = \"1.0\", features = [\"derive\"] }".to_string(),
            "borsh = { version = \"1.5\", features = [\"derive\"] }".to_string(),
            "solana-pubkey = { version = \"2.2\", features = [\"serde\", \"borsh\"] }".to_string(),
        ],
    );

    let stderr = compile_failure_stderr_with_cargo(
        "invalid_nested_computed_reference_is_rejected",
        cargo,
        &source,
    );
    assert!(stderr.contains(
        "unknown computed field reference 'lifecycle.initialize_params.presale_params.disable_earlier_presale_end_once_cap_reachd'"
    ));
}

#[test]
fn validation_reports_multiple_errors() {
    let source = r#"use arete_macros::arete;

#[arete]
mod broken {
    #[entity(name = "Thing")]
    #[view(name = "latest", sort_by = "ghost.value")]
    struct Thing {
        existing: String,
        #[resolve(from = "missing.field", resolver = Token)]
        metadata: String,
    }
}

fn main() {}
"#;

    let stderr = compile_failure_stderr("validation_reports_multiple_errors", source);
    assert!(stderr.contains("unknown view field 'ghost.value' on entity 'Thing'"));
    assert!(stderr.contains("unknown resolver input field 'missing.field' on entity 'Thing'"));
}
