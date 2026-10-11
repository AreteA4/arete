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

/// `describe_sdk` reads the reference `a4 install` wrote into the SDK folder
/// of the project `a4 mcp` runs in, found from a subdirectory too.
#[test]
fn a4_mcp_describes_an_installed_sdk_of_the_project_it_runs_in() {
    let project = tempfile::tempdir().expect("project directory");
    std::fs::write(
        project.path().join("arete.toml"),
        "manifest_version = 1\n\n[project]\nname = \"app\"\n\n[dependencies.stacks.ore]\nsource = { registry = \"ore\" }\nversion = \"^1.1.9\"\ntargets = [\"typescript\"]\n",
    )
    .expect("write arete.toml");
    let sdk = project.path().join("generated/typescript/stacks/ore");
    std::fs::create_dir_all(&sdk).expect("SDK folder");
    std::fs::create_dir_all(project.path().join("src")).expect("src");
    let reference = serde_json::json!({
        "schemaVersion": 1,
        "kind": "stack",
        "alias": "ore",
        "language": "typescript",
        "import": { "module": "ore.ts", "export": "ORE_STREAM_STACK" },
        "entities": [{
            "name": "OreRound",
            "typeName": "OreRound",
            "views": [{ "id": "OreRound/latest", "kind": "list", "access": "views.OreRound.latest" }],
            "fields": [{ "path": "id.roundId", "wire": "id.round_id", "type": "bigint", "nullable": true }]
        }]
    });
    std::fs::write(sdk.join("sdk-reference.json"), reference.to_string()).expect("reference");

    let mut child = Command::new(env!("CARGO_BIN_EXE_a4"))
        .arg("mcp")
        .current_dir(project.path().join("src"))
        .env("CI", "1")
        .env("DO_NOT_TRACK", "1")
        .env("A4_NO_UPDATE_CHECK", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn a4 mcp");
    let mut stdin = child.stdin.take().expect("child stdin");
    let stdout = child.stdout.take().expect("child stdout");
    let (line_tx, line_rx) = mpsc::channel::<String>();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if line_tx.send(line).is_err() {
                break;
            }
        }
    });
    let mut send = |message: serde_json::Value| {
        writeln!(stdin, "{message}").expect("write message");
        stdin.flush().expect("flush message");
    };
    let response = |id: u64| loop {
        let line = line_rx
            .recv_timeout(RESPONSE_TIMEOUT)
            .expect("a response from a4 mcp");
        let message: serde_json::Value = serde_json::from_str(&line).expect("JSON-RPC frame");
        if message["id"] == id {
            break message;
        }
    };

    send(serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "a4-cli-test", "version": "0.0.0" }
        }
    }));
    response(1);
    send(serde_json::json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));
    send(serde_json::json!({
        "jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": { "name": "describe_sdk", "arguments": { "alias": "ore", "view": "OreRound/latest" } }
    }));
    let described = response(2);
    send(serde_json::json!({
        "jsonrpc": "2.0", "id": 3, "method": "tools/call",
        "params": { "name": "describe_sdk", "arguments": { "alias": "raydium" } }
    }));
    let unknown = response(3);
    drop(stdin);
    let _ = child.wait();

    let text = described["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("describe_sdk result: {described}"));
    assert!(
        text.contains("Views: `views.OreRound.latest` (list)."),
        "{text}"
    );
    assert!(
        text.contains("id.roundId  bigint | null  ← round_id"),
        "{text}"
    );
    assert!(
        text.contains("Everything else: generated/typescript/stacks/ore/README.md"),
        "{text}"
    );
    assert_eq!(unknown["error"]["code"], -32602, "{unknown}");
    assert!(
        unknown["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("installed: stack ore")),
        "{unknown}"
    );
}
