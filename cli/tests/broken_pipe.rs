//! Output made for pipelines ends quietly when the reader has gone away
//! (`a4 dev instance <id> --json | head -c 10`), instead of panicking with
//! "failed printing to stdout: Broken pipe".
#![cfg(unix)]

use std::os::fd::FromRawFd;
use std::os::unix::process::ExitStatusExt;
use std::process::{Command, Stdio};

/// A pipe whose read end is already closed, as stdout for a child.
fn closed_pipe() -> Stdio {
    let mut fds = [0; 2];
    // SAFETY: `pipe` fills both descriptors; the read end is closed at once
    // and the write end is handed to `Stdio`, which owns it from here.
    unsafe {
        assert_eq!(libc::pipe(fds.as_mut_ptr()), 0);
        libc::close(fds[0]);
        Stdio::from_raw_fd(fds[1])
    }
}

fn run_into_closed_pipe(args: &[&str]) -> (std::process::ExitStatus, String) {
    let home = tempfile::tempdir().expect("temp home");
    let output = Command::new(env!("CARGO_BIN_EXE_a4"))
        .args(args)
        .env("HOME", home.path())
        .env("ARETE_HOME", home.path().join(".arete"))
        .env("DO_NOT_TRACK", "1")
        .env("A4_NO_UPDATE_CHECK", "1")
        .env_remove("ARETE_API_KEY")
        .current_dir(home.path())
        .stdin(Stdio::null())
        .stdout(closed_pipe())
        .stderr(Stdio::piped())
        .output()
        .expect("a4 runs");
    (
        output.status,
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn dev_json_into_a_closed_pipe_ends_without_a_panic() {
    // No session here, so this prints `{"running":false}` and nothing else.
    let (status, stderr) = run_into_closed_pipe(&["dev", "status", "--json"]);

    assert!(!stderr.contains("panicked"), "{stderr}");
    assert_eq!(status.signal(), Some(libc::SIGPIPE), "{status:?}: {stderr}");
}
