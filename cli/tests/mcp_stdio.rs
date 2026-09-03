//! WP6 "Done when": `a4 mcp` answers an MCP `initialize` over stdio and
//! exits once the client closes stdin.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

const RESPONSE_TIMEOUT: Duration = Duration::from_secs(60);

#[test]
fn a4_mcp_answers_initialize_and_exits_when_stdin_closes() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_a4"))
        .arg("mcp")
        // Keep telemetry and the update nudge quiet.
        .env("CI", "1")
        .env("DO_NOT_TRACK", "1")
        .env("A4_NO_UPDATE_CHECK", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn a4 mcp");

    let mut stdin = child.stdin.take().expect("child stdin");
    let stdout = child.stdout.take().expect("child stdout");
    let stderr = child.stderr.take().expect("child stderr");

    // Drain stderr so the child never blocks on a full pipe; keep it for diagnostics.
    let stderr_thread = thread::spawn(move || {
        let mut text = String::new();
        let _ = BufReader::new(stderr).read_to_string(&mut text);
        text
    });

    // Forward every stdout line through a channel so reads can time out.
    let (line_tx, line_rx) = mpsc::channel::<String>();
    let stdout_thread = thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if line_tx.send(line).is_err() {
                        break;
                    }
                }
            }
        }
    });

    let initialize = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "a4-cli-test", "version": "0.0.0" }
        }
    });
    writeln!(stdin, "{initialize}").expect("write initialize");
    stdin.flush().expect("flush initialize");

    let first_line = match line_rx.recv_timeout(RESPONSE_TIMEOUT) {
        Ok(line) => line,
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            panic!(
                "a4 mcp produced no stdout line within {RESPONSE_TIMEOUT:?}; stderr:\n{}",
                stderr_thread.join().unwrap_or_default()
            );
        }
    };

    // The very first byte on stdout must be the initialize response: no banner,
    // no log line, nothing that would corrupt the MCP transport.
    let response: serde_json::Value = serde_json::from_str(first_line.trim()).unwrap_or_else(|e| {
        let _ = child.kill();
        let _ = child.wait();
        panic!("first stdout line is not the JSON-RPC response ({e}): {first_line:?}")
    });
    assert_eq!(response["jsonrpc"], "2.0", "response: {response}");
    assert_eq!(response["id"], 1, "response: {response}");
    assert!(
        response.get("error").is_none(),
        "initialize returned an error: {response}"
    );
    assert_eq!(
        response["result"]["serverInfo"]["name"], "arete-mcp",
        "response: {response}"
    );
    assert!(
        response["result"]["protocolVersion"].is_string(),
        "response: {response}"
    );

    let initialized = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    });
    writeln!(stdin, "{initialized}").expect("write initialized");
    stdin.flush().expect("flush initialized");
    drop(stdin);

    // The server must exit on EOF; kill it if it lingers.
    let deadline = Instant::now() + RESPONSE_TIMEOUT;
    let status = loop {
        match child.try_wait().expect("try_wait") {
            Some(status) => break status,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                panic!(
                    "a4 mcp did not exit within {RESPONSE_TIMEOUT:?} after stdin closed; stderr:\n{}",
                    stderr_thread.join().unwrap_or_default()
                );
            }
            None => thread::sleep(Duration::from_millis(50)),
        }
    };
    stdout_thread.join().expect("stdout reader");
    let stderr_text = stderr_thread.join().unwrap_or_default();

    assert!(
        status.success(),
        "a4 mcp exited with {status} after stdin closed; stderr:\n{stderr_text}"
    );

    // Anything else on stdout must still be a JSON-RPC frame.
    for line in line_rx.try_iter() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        assert!(
            serde_json::from_str::<serde_json::Value>(trimmed).is_ok(),
            "non-JSON output on stdout after initialize: {trimmed:?}"
        );
    }
}
