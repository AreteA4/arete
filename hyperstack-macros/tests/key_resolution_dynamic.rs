mod support;

use support::{cargo_toml, escape_path, hyperstack_dir, macro_manifest_dir, TempCrate};

fn compile_failure_stderr_with_files(
    name: &str,
    source: &str,
    extra_files: &[(&str, &str)],
) -> String {
    let manifest_dir = macro_manifest_dir();
    let temp_crate = TempCrate::new(
        "key-resolution-dynamic",
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

    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn compile_success_with_files(name: &str, source: &str, extra_files: &[(&str, &str)]) {
    let hyperstack_dir = hyperstack_dir();
    let manifest_dir = macro_manifest_dir();
    let temp_crate = TempCrate::new(
        "key-resolution-dynamic",
        name,
        cargo_toml(
            name,
            &[
                format!(
                    "hyperstack = {{ path = \"{}\" }}",
                    escape_path(&hyperstack_dir)
                ),
                format!(
                    "hyperstack-macros = {{ path = \"{}\" }}",
                    escape_path(&manifest_dir)
                ),
                "borsh = { version = \"1.5\", features = [\"derive\"] }".to_string(),
                "serde = { version = \"1\", features = [\"derive\"] }".to_string(),
            ],
        ),
        source,
        extra_files,
    );

    let output = temp_crate.cargo_check();

    assert!(
        output.status.success(),
        "expected cargo check to succeed for {name}, stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn minimal_idl() -> &'static str {
    r#"{
  "name": "fake",
  "instructions": [
    {
      "name": "Trade",
      "accounts": [{ "name": "thing" }],
      "args": [
        { "name": "user", "type": "string" },
        { "name": "id", "type": "string" }
      ]
    }
  ],
  "accounts": [
    {
      "name": "Thing",
      "type": {
        "kind": "struct",
        "fields": [{ "name": "id", "type": "string" }]
      }
    },
    {
      "name": "Position",
      "type": {
        "kind": "struct",
        "fields": [{ "name": "amount", "type": "u64" }]
      }
    }
  ],
  "types": [],
  "events": [],
  "errors": [],
  "constants": []
}"#
}

#[test]
fn instruction_source_without_lookup_path_is_rejected() {
    let source = r#"use hyperstack_macros::hyperstack;

#[hyperstack(idl = "fixture/minimal.json")]
mod broken {
    #[entity(name = "Thing")]
    struct Thing {
        #[map(fake_sdk::accounts::Thing::id, primary_key, strategy = SetOnce)]
        id: String,

        #[aggregate(from = fake_sdk::instructions::Trade, strategy = Count)]
        trades: u64,
    }
}

fn main() {}
"#;

    let stderr = compile_failure_stderr_with_files(
        "instruction_source_without_lookup_path_is_rejected",
        source,
        &[("fixture/minimal.json", minimal_idl())],
    );
    assert!(stderr.contains(
        "instruction source 'fake_sdk::instructions::Trade' cannot resolve the primary key"
    ));
    assert!(stderr.contains("Add a `primary_key` mapping or `lookup_by = ...`"));
}

#[test]
fn account_source_without_pk_lookup_or_resolver_is_rejected() {
    let source = r#"use hyperstack_macros::hyperstack;

#[hyperstack(idl = "fixture/minimal.json")]
mod broken {
    #[entity(name = "Thing")]
    struct Thing {
        #[map(fake_sdk::accounts::Thing::id, primary_key, strategy = SetOnce)]
        id: String,

        #[map(fake_sdk::accounts::Position::amount, strategy = LastWrite)]
        amount: u64,
    }
}

fn main() {}
"#;

    let stderr = compile_failure_stderr_with_files(
        "account_source_without_pk_lookup_or_resolver_is_rejected",
        source,
        &[("fixture/minimal.json", minimal_idl())],
    );
    assert!(stderr
        .contains("account source 'fake_sdk::accounts::Position' cannot resolve the primary key"));
    assert!(stderr.contains("lookup-index-backed field"));
}

#[test]
fn event_source_without_lookup_or_join_is_rejected() {
    let source = r#"use hyperstack_macros::hyperstack;

#[hyperstack(idl = "fixture/minimal.json")]
mod broken {
    #[entity(name = "Thing")]
    struct Thing {
        #[map(fake_sdk::accounts::Thing::id, primary_key, strategy = SetOnce)]
        id: String,

        #[event(from = fake_sdk::instructions::Trade, fields = [user])]
        trades: Vec<fake_sdk::instructions::Trade>,
    }
}

fn main() {}
"#;

    let stderr = compile_failure_stderr_with_files(
        "event_source_without_lookup_or_join_is_rejected",
        source,
        &[("fixture/minimal.json", minimal_idl())],
    );
    assert!(stderr.contains("event source '"));
    assert!(stderr.contains("cannot resolve the primary key"));
    assert!(stderr.contains("Add `lookup_by = ...` or `join_on = ...`"));
}

#[test]
fn derive_from_primary_key_field_does_not_require_lookup_by() {
    let source = r#"use hyperstack_macros::hyperstack;

#[hyperstack(idl = "fixture/minimal.json")]
mod valid {
    #[entity(name = "Thing")]
    struct Thing {
        #[map(fake_sdk::accounts::Thing::id, primary_key, strategy = SetOnce)]
        id: String,

        #[derive_from(from = fake_sdk::instructions::Trade, field = id, strategy = LastWrite)]
        latest_id: String,
    }
}

fn main() {}
"#;

    compile_success_with_files(
        "derive_from_primary_key_field_does_not_require_lookup_by",
        source,
        &[("fixture/minimal.json", minimal_idl())],
    );
}

#[test]
fn event_group_accepts_any_valid_lookup_by_regardless_of_field_order() {
    let source = r#"use hyperstack_macros::hyperstack;

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
struct TradeCapture {
    user: String,
}

#[hyperstack(idl = "fixture/minimal.json")]
mod valid {
    use super::TradeCapture;

    #[entity(name = "Thing")]
    struct Thing {
        #[map(fake_sdk::accounts::Thing::id, primary_key, strategy = SetOnce)]
        id: String,

        #[event(from = fake_sdk::instructions::Trade, fields = [user])]
        raw_trades: TradeCapture,

        #[event(from = fake_sdk::instructions::Trade, fields = [user], lookup_by = id)]
        keyed_trades: TradeCapture,
    }
}

fn main() {}
"#;

    compile_success_with_files(
        "event_group_accepts_any_valid_lookup_by_regardless_of_field_order",
        source,
        &[("fixture/minimal.json", minimal_idl())],
    );
}
