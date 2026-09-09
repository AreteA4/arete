mod support;

// Compile and execute the actual emitters for both runtime variants. The
// standalone ingestion gate separately exercises the public #[arete] surface.
#[allow(dead_code)]
#[path = "../src/codegen/vixen_runtime.rs"]
mod runtime_codegen;

use support::{arete_dir, cargo_toml, escape_path, TempCrate};

#[test]
fn transaction_v1_proto_through_generated_handlers() {
    let source = include_str!("fixtures/transaction-v1-runtime.rs")
        .replace(
            "__single_handler!();",
            &runtime_codegen::generate_vm_handler("AccountValue", "InstructionValue", "test")
                .to_string(),
        )
        .replace(
            "__multi_handler!();",
            &format!(
                "{} {} {}",
                runtime_codegen::generate_vm_handler_struct(),
                runtime_codegen::generate_instruction_handler_impl(
                    "parsers",
                    "InstructionValue",
                    "test"
                ),
                runtime_codegen::generate_account_handler_impl("parsers", "AccountValue"),
            ),
        );
    let temp = TempCrate::new(
        "transaction-v1-runtime",
        "transaction-v1-runtime",
        cargo_toml(
            "transaction-v1-runtime",
            &[
                format!("arete = {{ path = \"{}\" }}", escape_path(&arete_dir())),
                "serde = { version = \"1\", features = [\"derive\"] }".into(),
            ],
        ),
        &source,
        &[],
    );
    let output = temp.cargo_run();
    assert!(
        output.status.success(),
        "generated runtime failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn managed_transport_reconnect_replay_pressure_and_slots() {
    let source = include_str!("fixtures/transaction-v1-transport.rs")
        .replace(
            "__managed_helpers!();",
            &runtime_codegen::generate_managed_grpc_helpers().to_string(),
        )
        .replace(
            "__slot_task!();",
            &runtime_codegen::generate_slot_subscription_task().to_string(),
        );
    let temp = TempCrate::new(
        "transaction-v1-runtime",
        "transaction-v1-transport",
        cargo_toml(
            "transaction-v1-transport",
            &[
                format!("arete = {{ path = \"{}\" }}", escape_path(&arete_dir())),
                "tokio-stream = { version = \"0.1\", features = [\"net\"] }".into(),
            ],
        ),
        &source,
        &[],
    );
    let output = temp.cargo_run();
    assert!(
        output.status.success(),
        "transport runtime failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
