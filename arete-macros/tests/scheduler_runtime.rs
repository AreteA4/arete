mod support;

// Compile and execute the emitted slot-scheduler task.
#[allow(dead_code)]
#[path = "../src/codegen/vixen_runtime.rs"]
mod runtime_codegen;

use support::{arete_dir, cargo_toml, escape_path, TempCrate};

#[test]
fn scheduler_batches_do_not_move_the_resume_watermark() {
    let source = include_str!("fixtures/scheduler-resume-watermark.rs")
        .replace(
            "__runtime_helpers!();",
            &runtime_codegen::generate_runtime_helpers().to_string(),
        )
        .replace(
            "__scheduler_task!();",
            &runtime_codegen::generate_slot_scheduler_task().to_string(),
        );
    let temp = TempCrate::new(
        "scheduler-runtime",
        "scheduler-resume-watermark",
        cargo_toml(
            "scheduler-resume-watermark",
            &[format!(
                "arete = {{ path = \"{}\" }}",
                escape_path(&arete_dir())
            )],
        ),
        &source,
        &[],
    );
    let output = temp.cargo_run();
    assert!(
        output.status.success(),
        "generated scheduler failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
