//! `a4 sandbox list` and `a4 sandbox use` against a stub Arete API: the
//! listing, a choice saved for the directory, and the reads that follow it.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::{Arc, Mutex};
use std::thread;

use serde_json::{json, Value};

const BOT: &str = "6f1c2c43-6d3c-4a51-9b8e-0e3c1e2a7b10";
const TREASURY: &str = "0b5a3e8e-2f7d-4c1a-8e2b-7f3c9d1e4a20";

/// An API granting two active sandboxes; it records each request line and
/// mint body.
fn stub_api() -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let mut request = Vec::new();
            let mut buffer = [0u8; 8192];
            loop {
                match stream.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(read) => request.extend_from_slice(&buffer[..read]),
                }
                let text = String::from_utf8_lossy(&request).to_string();
                if let Some(end) = text.find("\r\n\r\n") {
                    let length = text
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                        })
                        .unwrap_or(0);
                    if request.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            let text = String::from_utf8_lossy(&request).to_string();
            let line = text.lines().next().unwrap_or("").to_string();
            let body_text = text.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
            log.lock().unwrap().push(format!("{line} {body_text}"));
            let (status, body) = if line.starts_with("GET /api/agent/sandboxes ") {
                (
                    200,
                    json!({ "sandboxes": [
                        { "sandboxId": BOT, "name": "Bot", "walletAddress": "WalletBot",
                          "cluster": "devnet", "status": "active", "actions": ["invoke"],
                          "grantExpiresAt": "2026-12-01T00:00:00Z" },
                        { "sandboxId": TREASURY, "name": "Treasury", "walletAddress": "WalletTreasury",
                          "cluster": "devnet", "status": "active", "actions": ["withdraw"],
                          "grantExpiresAt": "2026-12-01T00:00:00Z" },
                    ], "requestId": "r-1" }),
                )
            } else if line.starts_with("GET /api/auth/me ")
                && text.contains("Bearer a4_ak_profile_agent")
            {
                (
                    200,
                    json!({ "accountKind": "agent", "commandGroups": ["account", "sandboxes", "registry"] }),
                )
            } else if line.starts_with("GET /api/auth/me ")
                && text.contains("Bearer a4_sk_profile_human")
            {
                (
                    200,
                    json!({ "accountKind": "human", "commandGroups": ["account", "sandboxes", "streams", "stacks", "registry"] }),
                )
            } else if line.starts_with("POST /ws/sessions ") {
                (
                    200,
                    json!({ "token": "tok-1", "expiresAt": 99999999999u64, "scopes": ["sandbox:invoke"] }),
                )
            } else if line.contains("/holdings ") {
                (
                    200,
                    json!({ "sol": "1000000000", "tokens": [], "positions": [], "asOfSlot": 7 }),
                )
            } else {
                (404, json!({ "code": "not-found" }))
            };
            let body = body.to_string();
            let _ = write!(
                stream,
                "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    (format!("http://{address}"), seen)
}

fn a4(home: &Path, project: &Path, api: &str, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_a4"))
        .args(args)
        .current_dir(project)
        .env("HOME", home)
        .env("ARETE_HOME", home.join(".arete"))
        .env("ARETE_API_URL", api)
        .env("ARETE_API_KEY", "a4_ak_test_agent")
        .env_remove("ARETE_SANDBOX_ID")
        .env_remove("ARETE_PROFILE")
        .env_remove("ARETE_CREDENTIALS_PATH")
        .env("CI", "1")
        .env("DO_NOT_TRACK", "1")
        .env("A4_NO_UPDATE_CHECK", "1")
        .output()
        .expect("run a4")
}

fn json_out(output: &Output) -> Value {
    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(stdout.trim()).unwrap_or_else(|error| {
        panic!(
            "not JSON ({error}): {stdout}\nstderr: {}",
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

#[test]
fn a_choice_made_with_use_is_remembered_for_the_directory() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let (api, seen) = stub_api();

    // Two grants and no choice: listed, nothing in use, and reads refuse.
    let listed = a4(
        home.path(),
        project.path(),
        &api,
        &["sandbox", "list", "--json"],
    );
    assert!(listed.status.success(), "{listed:?}");
    let listed = json_out(&listed);
    assert_eq!(listed["sandboxes"].as_array().unwrap().len(), 2);
    assert_eq!(listed["current"], Value::Null);
    assert!(listed["note"].as_str().unwrap().contains("a4 sandbox use"));
    assert!(listed.get("requestId").is_none());

    let refused = a4(
        home.path(),
        project.path(),
        &api,
        &["sandbox", "holdings", "--json"],
    );
    assert!(!refused.status.success());
    let refused = json_out(&refused);
    assert_eq!(refused["code"], "sandbox-selection-required");
    assert!(refused["hint"].as_str().unwrap().contains("a4 sandbox use"));

    // Choose by name; the holdings read then mints for that sandbox.
    let used = a4(
        home.path(),
        project.path(),
        &api,
        &["sandbox", "use", "treasury", "--json"],
    );
    assert!(used.status.success(), "{used:?}");
    let used = json_out(&used);
    assert_eq!(used["selected"]["sandboxId"], TREASURY);
    assert_eq!(used["saved"], true);

    let held = a4(home.path(), project.path(), &api, &["sandbox", "holdings"]);
    assert!(held.status.success(), "{held:?}");
    let shown = String::from_utf8_lossy(&held.stdout);
    assert!(shown.contains(TREASURY), "{shown}");
    let mints: Vec<String> = seen
        .lock()
        .unwrap()
        .iter()
        .filter(|request| request.starts_with("POST /ws/sessions"))
        .cloned()
        .collect();
    assert_eq!(mints.len(), 1, "{mints:?}");
    assert!(mints[0].contains(TREASURY), "{mints:?}");

    let listed = json_out(&a4(
        home.path(),
        project.path(),
        &api,
        &["sandbox", "list", "--json"],
    ));
    assert_eq!(
        listed["current"],
        json!({ "sandboxId": TREASURY, "source": "saved" })
    );

    // `--sandbox` overrides it for one command; another directory has no choice.
    let held = a4(
        home.path(),
        project.path(),
        &api,
        &["sandbox", "holdings", "--sandbox", BOT],
    );
    assert!(
        String::from_utf8_lossy(&held.stdout).contains(BOT),
        "{held:?}"
    );
    let other = json_out(&a4(
        home.path(),
        elsewhere.path(),
        &api,
        &["sandbox", "list", "--json"],
    ));
    assert_eq!(other["current"], Value::Null);

    // Forgetting it brings the refusal back.
    let cleared = a4(
        home.path(),
        project.path(),
        &api,
        &["sandbox", "use", "--clear", "--json"],
    );
    assert!(cleared.status.success(), "{cleared:?}");
    assert_eq!(json_out(&cleared)["cleared"], true);
    let listed = json_out(&a4(
        home.path(),
        project.path(),
        &api,
        &["sandbox", "list", "--json"],
    ));
    assert_eq!(listed["current"], Value::Null);
}

#[test]
fn use_needs_a_sandbox_or_clear() {
    let home = tempfile::tempdir().unwrap();
    let (api, _) = stub_api();
    let output = a4(home.path(), home.path(), &api, &["sandbox", "use"]);
    assert!(!output.status.success());
    let output = a4(
        home.path(),
        home.path(),
        &api,
        &["sandbox", "use", BOT, "--clear"],
    );
    assert!(!output.status.success());
}

/// `--profile human` in a project whose default profile is the sandbox-only
/// agent: the read is the human account's own, not the agent's sandbox path.
#[test]
fn an_explicit_profile_decides_whether_a_read_goes_through_a_sandbox() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let (api, seen) = stub_api();
    let credentials = home.path().join("credentials.toml");
    std::fs::write(
        &credentials,
        format!(
            "[profiles.agent.keys]\n\"{api}\" = \"a4_ak_profile_agent\"\n\n[profiles.human.keys]\n\"{api}\" = \"a4_sk_profile_human\"\n"
        ),
    )
    .unwrap();
    std::fs::create_dir_all(project.path().join(".arete")).unwrap();
    std::fs::write(
        project.path().join(".arete/auth.toml"),
        "default_profile = \"agent\"\n",
    )
    .unwrap();
    let run = |args: &[&str]| {
        seen.lock().unwrap().clear();
        Command::new(env!("CARGO_BIN_EXE_a4"))
            .args(args)
            .current_dir(project.path())
            .env("HOME", home.path())
            .env("ARETE_HOME", home.path().join(".arete"))
            .env("ARETE_API_URL", &api)
            .env("ARETE_CREDENTIALS_PATH", &credentials)
            .env_remove("ARETE_API_KEY")
            .env_remove("ARETE_SANDBOX_ID")
            .env_remove("ARETE_PROFILE")
            .env("CI", "1")
            .env("DO_NOT_TRACK", "1")
            .env("A4_NO_UPDATE_CHECK", "1")
            .output()
            .expect("run a4");
        seen.lock().unwrap().clone()
    };
    let through_sandbox = |requests: &[String]| {
        requests
            .iter()
            .any(|r| r.starts_with("GET /api/agent/sandboxes "))
    };

    // The project's agent profile reads only through a sandbox.
    let address = "11111111111111111111111111111111";
    let requests = run(&["program", "account", "decode", address, "--json"]);
    assert!(through_sandbox(&requests), "{requests:?}");

    // An explicit --profile human reads with the human key, as asked.
    let requests = run(&[
        "--profile",
        "human",
        "program",
        "account",
        "decode",
        address,
        "--json",
    ]);
    assert!(!through_sandbox(&requests), "{requests:?}");
}
