mod support;

use support::{arete_dir, cargo_toml, escape_path, macro_manifest_dir, TempCrate};

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
                "arete-macros = {{ path = \"{}\" }}",
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
    let arete_dir = arete_dir();
    let manifest_dir = macro_manifest_dir();
    let temp_crate = TempCrate::new(
        "key-resolution-dynamic",
        name,
        cargo_toml(
            name,
            &[
                format!("arete = {{ path = \"{}\" }}", escape_path(&arete_dir)),
                format!(
                    "arete-macros = {{ path = \"{}\" }}",
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

fn cargo_check_output_with_files(
    name: &str,
    source: &str,
    extra_files: &[(&str, &str)],
) -> std::process::Output {
    let arete_dir = arete_dir();
    let manifest_dir = macro_manifest_dir();
    let temp_crate = TempCrate::new(
        "key-resolution-dynamic",
        name,
        cargo_toml(
            name,
            &[
                format!("arete = {{ path = \"{}\" }}", escape_path(&arete_dir)),
                format!(
                    "arete-macros = {{ path = \"{}\" }}",
                    escape_path(&manifest_dir)
                ),
                "borsh = { version = \"1.5\", features = [\"derive\"] }".to_string(),
                "serde = { version = \"1\", features = [\"derive\"] }".to_string(),
            ],
        ),
        source,
        extra_files,
    );

    temp_crate.cargo_check()
}

fn minimal_idl() -> &'static str {
    r#"{
  "address": "Test111111111111111111111111111111111111111",
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

fn minimal_event_idl() -> &'static str {
    r#"{
  "address": "Test111111111111111111111111111111111111111",
  "name": "fake",
  "instructions": [],
  "accounts": [
    {
      "name": "Thing",
      "type": {
        "kind": "struct",
        "fields": [{ "name": "id", "type": "string" }]
      }
    }
  ],
  "types": [
    {
      "name": "TradeExecuted",
      "type": {
        "kind": "struct",
        "fields": [
          { "name": "id", "type": "string" },
          { "name": "user", "type": "string" },
          { "name": "amount", "type": "u64" }
        ]
      }
    }
  ],
  "events": [
    {
      "name": "TradeExecuted",
      "discriminator": [1]
    }
  ],
  "errors": [],
  "constants": []
}"#
}

#[test]
fn instruction_source_without_lookup_path_is_rejected() {
    let source = r#"use arete_macros::arete;

#[arete(idl = "fixture/minimal.json")]
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
    let source = r#"use arete_macros::arete;

#[arete(idl = "fixture/minimal.json")]
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
    let source = r#"use arete_macros::arete;

#[arete(idl = "fixture/minimal.json")]
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
    assert!(stderr.contains("Add `lookup_by = ...`, `join_on = ...`, or include the primary-key field in `fields = [...]`"));
}

#[test]
fn derive_from_primary_key_field_does_not_require_lookup_by() {
    let source = r#"use arete_macros::arete;

#[arete(idl = "fixture/minimal.json")]
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
fn derive_from_group_passes_when_any_field_resolves_key() {
    let source = r#"use arete_macros::arete;

#[arete(idl = "fixture/minimal.json")]
mod valid {
    #[entity(name = "Thing")]
    struct Thing {
        #[map(fake_sdk::accounts::Thing::id, primary_key, strategy = SetOnce)]
        id: String,

        #[derive_from(from = fake_sdk::instructions::Trade, field = id, strategy = LastWrite)]
        latest_id: String,

        #[derive_from(from = fake_sdk::instructions::Trade, field = user, strategy = LastWrite)]
        latest_user: String,
    }
}

fn main() {}
"#;

    compile_success_with_files(
        "derive_from_group_passes_when_any_field_resolves_key",
        source,
        &[("fixture/minimal.json", minimal_idl())],
    );
}

#[test]
fn derive_from_group_emits_single_error_on_failure() {
    let source = r#"use arete_macros::arete;

#[arete(idl = "fixture/minimal.json")]
mod broken {
    #[entity(name = "Thing")]
    struct Thing {
        #[map(fake_sdk::accounts::Thing::id, primary_key, strategy = SetOnce)]
        id: String,

        #[derive_from(from = fake_sdk::instructions::Trade, field = user, strategy = LastWrite)]
        latest_user: String,

        #[derive_from(from = fake_sdk::instructions::Trade, field = user, lookup_by = user, strategy = LastWrite)]
        latest_lookup: String,
    }
}

fn main() {}
"#;

    let stderr = compile_failure_stderr_with_files(
        "derive_from_group_emits_single_error_on_failure",
        source,
        &[("fixture/minimal.json", minimal_idl())],
    );

    // Group emits one error (the first bad lookup_by), not one per attribute
    assert!(stderr.contains("The `lookup_by` field 'user' is neither a primary-key field nor a lookup-index-backed field."));
    assert_eq!(
        stderr.matches("cannot resolve the primary key").count(),
        1,
        "expected single group error, stderr was:\n{stderr}"
    );
}

#[test]
fn event_group_passes_when_any_captured_field_resolves_key() {
    let source = r#"use arete_macros::arete;

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
struct TradeCapture {
    id: String,
}

#[arete(idl = "fixture/minimal.json")]
mod valid {
    use super::TradeCapture;

    #[entity(name = "Thing")]
    struct Thing {
        #[map(fake_sdk::accounts::Thing::id, primary_key, strategy = SetOnce)]
        id: String,

        #[event(from = fake_sdk::instructions::Trade, fields = [id])]
        trades: TradeCapture,
    }
}

fn main() {}
"#;

    compile_success_with_files(
        "event_group_passes_when_any_captured_field_resolves_key",
        source,
        &[("fixture/minimal.json", minimal_idl())],
    );
}

#[test]
fn event_group_accepts_any_valid_lookup_by_regardless_of_field_order() {
    let source = r#"use arete_macros::arete;

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
struct TradeCapture {
    user: String,
}

#[arete(idl = "fixture/minimal.json")]
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

#[test]
fn aggregate_from_cpi_event_validates_event_fields() {
    let source = r#"use arete_macros::arete;

#[arete(idl = "fixture/minimal_event.json")]
mod valid {
    #[entity(name = "Thing")]
    struct Thing {
        #[map(fake_sdk::accounts::Thing::id, primary_key, strategy = SetOnce)]
        id: String,

        #[aggregate(from = fake_sdk::events::TradeExecuted,
                    field = amount, strategy = Sum, lookup_by = id)]
        total_amount: u64,
    }
}

fn main() {}
"#;

    let output = cargo_check_output_with_files(
        "aggregate_from_cpi_event_validates_event_fields",
        source,
        &[("fixture/minimal_event.json", minimal_event_idl())],
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("Not found: 'TradeExecuted' in instructions"),
        "stderr was:\n{stderr}"
    );
    assert!(
        !stderr.contains("Not found: 'amount' in instruction fields"),
        "stderr was:\n{stderr}"
    );
}

#[test]
fn derive_from_cpi_event_validates_event_fields() {
    let source = r#"use arete_macros::arete;

#[arete(idl = "fixture/minimal_event.json")]
mod valid {
    #[entity(name = "Thing")]
    struct Thing {
        #[map(fake_sdk::accounts::Thing::id, primary_key, strategy = SetOnce)]
        id: String,

        #[derive_from(from = fake_sdk::events::TradeExecuted,
                      field = amount, strategy = LastWrite, lookup_by = id)]
        last_amount: u64,
    }
}

fn main() {}
"#;

    let output = cargo_check_output_with_files(
        "derive_from_cpi_event_validates_event_fields",
        source,
        &[("fixture/minimal_event.json", minimal_event_idl())],
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("Not found: 'TradeExecuted' in instructions"),
        "stderr was:\n{stderr}"
    );
    assert!(
        !stderr.contains("Not found: 'amount' in instruction fields"),
        "stderr was:\n{stderr}"
    );
}

#[test]
fn event_attribute_accepts_cpi_event_sources() {
    let source = r#"use arete_macros::arete;

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
struct TradeCapture {
    id: String,
    amount: u64,
}

#[arete(idl = "fixture/minimal_event.json")]
mod valid {
    use super::TradeCapture;

    #[entity(name = "Thing")]
    struct Thing {
        #[map(fake_sdk::accounts::Thing::id, primary_key, strategy = SetOnce)]
        id: String,

        #[event(from = fake_sdk::events::TradeExecuted, fields = [id, amount])]
        trades: TradeCapture,
    }
}

fn main() {}
"#;

    let output = cargo_check_output_with_files(
        "event_attribute_accepts_cpi_event_sources",
        source,
        &[("fixture/minimal_event.json", minimal_event_idl())],
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("Not found: 'TradeExecuted' in instructions"),
        "stderr was:\n{stderr}"
    );
    assert!(
        !stderr.contains("Not found: 'id' in instruction fields for 'TradeExecuted'"),
        "stderr was:\n{stderr}"
    );
    assert!(
        !stderr.contains("Not found: 'amount' in instruction fields for 'TradeExecuted'"),
        "stderr was:\n{stderr}"
    );
}

#[test]
fn aggregate_condition_from_cpi_event_accepts_event_fields() {
    let source = r#"use arete_macros::arete;

#[arete(idl = "fixture/minimal_event.json")]
mod valid {
    #[entity(name = "Thing")]
    struct Thing {
        #[map(fake_sdk::accounts::Thing::id, primary_key, strategy = SetOnce)]
        id: String,

        #[aggregate(from = fake_sdk::events::TradeExecuted,
                    field = amount, strategy = Sum, condition = "amount > 0", lookup_by = id)]
        total_amount: u64,
    }
}

fn main() {}
"#;

    let output = cargo_check_output_with_files(
        "aggregate_condition_from_cpi_event_accepts_event_fields",
        source,
        &[("fixture/minimal_event.json", minimal_event_idl())],
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "stderr was:\n{stderr}");
    assert!(
        !stderr.contains("Not found: 'amount' in event fields for 'TradeExecuted'"),
        "stderr was:\n{stderr}"
    );
}

#[test]
fn aggregate_condition_from_cpi_event_rejects_unknown_event_fields() {
    let source = r#"use arete_macros::arete;

#[arete(idl = "fixture/minimal_event.json")]
mod invalid {
    #[entity(name = "Thing")]
    struct Thing {
        #[map(fake_sdk::accounts::Thing::id, primary_key, strategy = SetOnce)]
        id: String,

        #[aggregate(from = fake_sdk::events::TradeExecuted,
                    field = amount, strategy = Sum, condition = "ammount > 0", lookup_by = id)]
        total_amount: u64,
    }
}

fn main() {}
"#;

    let output = cargo_check_output_with_files(
        "aggregate_condition_from_cpi_event_rejects_unknown_event_fields",
        source,
        &[("fixture/minimal_event.json", minimal_event_idl())],
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "cargo check unexpectedly succeeded"
    );
    assert!(
        stderr.contains("Not found: 'ammount' in event fields for 'TradeExecuted'"),
        "stderr was:\n{stderr}"
    );
    assert!(stderr.contains("amount"), "stderr was:\n{stderr}");
}
