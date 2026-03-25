mod support;

use support::{cargo_toml, escape_path, hyperstack_dir, macro_manifest_dir, TempCrate};

fn run_binary_success(name: &str, source: &str) {
    let hyperstack_dir = hyperstack_dir();
    let manifest_dir = macro_manifest_dir();
    let temp_crate = TempCrate::new(
        "phase1-runtime",
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
                "serde = { version = \"1\", features = [\"derive\"] }".to_string(),
            ],
        ),
        source,
        &[],
    );

    let output = temp_crate.cargo_run();

    assert!(
        output.status.success(),
        "expected cargo run to succeed for {name}, stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn generated_loaders_do_not_require_stack_artifact_file() {
    let source = r#"use hyperstack_macros::hyperstack;

#[hyperstack]
mod stream {
    #[entity(name = "Thing")]
    struct Thing {}
}

fn main() {
    let stack_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(".hyperstack/Stream.stack.json");

    if stack_path.exists() {
        std::fs::remove_file(&stack_path).unwrap();
    }

    let _spec = stream::create_thing_spec();
    let _views = stream::get_view_definitions();
    let _bytecode = stream::create_multi_entity_bytecode();
}
"#;

    run_binary_success(
        "generated_loaders_do_not_require_stack_artifact_file",
        source,
    );
}
