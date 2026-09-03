//! WP5 "Done when": with stdin closed (and `CI` unset, so only the TTY check
//! applies) every command either succeeds or exits 1 with a flag-listing
//! error; none blocks waiting on stdin.

use std::io::Read;
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(60);

struct Run {
    status: ExitStatus,
    stdout: String,
    stderr: String,
}

/// An `a4` invocation with stdin closed and all state confined to `home`.
fn a4(home: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_a4"));
    cmd.env_remove("CI")
        .env_remove("A4_NON_INTERACTIVE")
        .env_remove("A4_YES")
        .env_remove("ARETE_API_KEY")
        .env("HOME", home)
        .env("ARETE_HOME", home.join(".arete"))
        .env(
            "ARETE_CREDENTIALS_PATH",
            home.join(".arete").join("credentials.toml"),
        )
        .env("DO_NOT_TRACK", "1")
        .env("A4_NO_UPDATE_CHECK", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd
}

/// Run `cmd`, killing it (and failing) if it is still alive after the timeout.
fn run_with_timeout(mut cmd: Command, label: &str) -> Run {
    let mut child = cmd.spawn().unwrap_or_else(|e| panic!("spawn {label}: {e}"));
    let mut stdout = child.stdout.take().expect("stdout");
    let mut stderr = child.stderr.take().expect("stderr");
    let stdout_thread = thread::spawn(move || {
        let mut text = String::new();
        let _ = stdout.read_to_string(&mut text);
        text
    });
    let stderr_thread = thread::spawn(move || {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text);
        text
    });

    let deadline = Instant::now() + COMMAND_TIMEOUT;
    let status = loop {
        match child.try_wait().expect("try_wait") {
            Some(status) => break status,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("{label} did not exit within {COMMAND_TIMEOUT:?} with stdin closed (blocked on a prompt?)");
            }
            None => thread::sleep(Duration::from_millis(50)),
        }
    };
    Run {
        status,
        stdout: stdout_thread.join().unwrap_or_default(),
        stderr: stderr_thread.join().unwrap_or_default(),
    }
}

#[test]
fn create_without_args_fails_fast_and_names_the_template_flag() {
    let home = tempfile::tempdir().expect("tempdir");
    let project = tempfile::tempdir().expect("tempdir");
    let mut cmd = a4(home.path());
    cmd.arg("create").current_dir(project.path());
    let run = run_with_timeout(cmd, "a4 create");

    assert_eq!(
        run.status.code(),
        Some(1),
        "a4 create should exit 1; stdout:\n{}\nstderr:\n{}",
        run.stdout,
        run.stderr
    );
    assert!(
        run.stderr.contains("--template"),
        "stderr should list --template; stdout:\n{}\nstderr:\n{}",
        run.stdout,
        run.stderr
    );
}

#[test]
fn auth_login_without_key_fails_fast_and_names_the_key_flag() {
    let home = tempfile::tempdir().expect("tempdir");
    let mut cmd = a4(home.path());
    cmd.args(["auth", "login"]).current_dir(home.path());
    let run = run_with_timeout(cmd, "a4 auth login");

    assert_eq!(
        run.status.code(),
        Some(1),
        "a4 auth login should exit 1; stdout:\n{}\nstderr:\n{}",
        run.stdout,
        run.stderr
    );
    assert!(
        run.stderr.contains("--key"),
        "stderr should list --key; stdout:\n{}\nstderr:\n{}",
        run.stdout,
        run.stderr
    );
}

#[test]
fn init_yes_json_succeeds_in_an_empty_directory_without_a_tty() {
    let home = tempfile::tempdir().expect("tempdir");
    let project = tempfile::tempdir().expect("tempdir");
    let mut cmd = a4(home.path());
    cmd.args(["init", "-y", "--json", "--no-skills", "--no-mcp"])
        .current_dir(project.path());
    let run = run_with_timeout(cmd, "a4 init -y --json");

    assert_eq!(
        run.status.code(),
        Some(0),
        "a4 init -y --json should exit 0; stdout:\n{}\nstderr:\n{}",
        run.stdout,
        run.stderr
    );
    let report: serde_json::Value = serde_json::from_str(run.stdout.trim()).unwrap_or_else(|e| {
        panic!(
            "stdout is not JSON ({e}); stdout:\n{}\nstderr:\n{}",
            run.stdout, run.stderr
        )
    });
    assert_eq!(report["schemaVersion"], 1, "report: {report}");
    assert!(
        project.path().join("arete.toml").is_file(),
        "a4 init should create arete.toml; report: {report}"
    );
}
