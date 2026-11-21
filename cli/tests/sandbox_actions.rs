//! `a4 sandbox preview`, `a4 sandbox invoke` and `a4 sandbox action`
//! against a stub Arete API: each action goes to the route the MCP tools
//! use, with the same body and idempotency key, and its result or refusal
//! comes back as the tools return it.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;

use serde_json::{json, Value};

const BOT: &str = "6f1c2c43-6d3c-4a51-9b8e-0e3c1e2a7b10";

/// One request the stub saw: its request line, headers and JSON body.
#[derive(Debug, Clone)]
struct Seen {
    line: String,
    head: String,
    body: Value,
}

/// An API granting one active sandbox that answers each sandbox route in
/// `routes` (`"POST /api/sandbox/v1/actions/call/preview"`) with its status
/// and body, and records every request.
fn stub_api(routes: Vec<(&'static str, u16, Value)>) -> (String, Arc<Mutex<Vec<Seen>>>) {
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
            let (head, body_text) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
            let line = head.lines().next().unwrap_or("").to_string();
            log.lock().unwrap().push(Seen {
                line: line.clone(),
                head: head.to_ascii_lowercase(),
                body: serde_json::from_str(body_text).unwrap_or(Value::Null),
            });
            // `POST /path HTTP/1.1` → `POST /path`
            let route = line.rsplit_once(' ').map_or(line.as_str(), |(r, _)| r);
            let (status, body) = if route == "GET /api/agent/sandboxes" {
                (
                    200,
                    json!({ "sandboxes": [
                        { "sandboxId": BOT, "name": "Bot", "walletAddress": "WalletBot",
                          "cluster": "devnet", "status": "active", "actions": ["invoke", "withdraw"],
                          "grantExpiresAt": "2026-12-01T00:00:00Z" },
                    ] }),
                )
            } else if route == "POST /ws/sessions" {
                (
                    200,
                    json!({ "token": "tok-1", "expiresAt": 99999999999u64,
                            "scopes": ["sandbox:invoke", "sandbox:withdraw"] }),
                )
            } else if let Some((_, status, body)) =
                routes.iter().find(|(wanted, _, _)| *wanted == route)
            {
                (*status, body.clone())
            } else {
                (404, json!({ "code": "not-found", "error": route }))
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

fn a4_with_stdin(home: &Path, api: &str, args: &[&str], stdin: Option<&str>) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_a4"))
        .args(args)
        .current_dir(home)
        .env("HOME", home)
        .env("ARETE_HOME", home.join(".arete"))
        .env("ARETE_API_URL", api)
        .env("ARETE_API_KEY", "a4_ak_test_agent")
        .env_remove("ARETE_SANDBOX_ID")
        .env_remove("ARETE_PROFILE")
        .env_remove("ARETE_CREDENTIALS_PATH")
        .env("CI", "1")
        .env("NO_COLOR", "1")
        .env("DO_NOT_TRACK", "1")
        .env("A4_NO_UPDATE_CHECK", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run a4");
    let mut input = child.stdin.take().unwrap();
    if let Some(stdin) = stdin {
        input.write_all(stdin.as_bytes()).unwrap();
    }
    drop(input);
    child.wait_with_output().expect("a4 output")
}

fn a4(home: &Path, api: &str, args: &[&str]) -> Output {
    a4_with_stdin(home, api, args, None)
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

/// The requests to the sandbox routes, after the listing and the mint.
fn action_requests(seen: &Arc<Mutex<Vec<Seen>>>) -> Vec<Seen> {
    seen.lock()
        .unwrap()
        .iter()
        .filter(|seen| seen.line.contains("/api/sandbox/v1/"))
        .cloned()
        .collect()
}

fn idempotency_header(seen: &Seen) -> Option<String> {
    seen.head
        .lines()
        .find_map(|line| line.strip_prefix("idempotency-key:"))
        .map(|value| value.trim().to_string())
}

/// The API's refusal of an action the token does not grant.
fn not_granted() -> Value {
    json!({
        "code": "sandbox-action-not-granted",
        "error": "This agent is not granted `withdraw` on this sandbox",
        "rule": "session-grant",
        "retryable": false,
        "requestId": "req-42",
    })
}

/// A call the API refused at its `build` stage.
fn refused_call() -> Value {
    json!({
        "code": "sandbox-function-input-invalid",
        "error": "input.amount: a bare decimal string is not an amount",
        "rule": "function-input",
        "retryable": false,
        "requestId": "req-7",
        "stages": [
            { "name": "request" },
            { "name": "plan", "code": "sandbox-function-input-invalid",
              "reason": "input.amount: a bare decimal string is not an amount" },
        ],
        "failedStage": "plan",
        "plan": { "function": "transactions.obligation.deposit", "program": "KLend" },
        "guardPipeline": "execute-v1",
    })
}

#[test]
fn a_call_preview_sends_the_function_and_its_input_from_a_file() {
    let home = tempfile::tempdir().unwrap();
    let passed = json!({
        "pass": true, "walletLamportsChange": "-100005000", "outflowLamports": "100005000",
        "networkFeeLamports": "5000", "guardPipeline": "execute-v1",
        "plan": { "function": "transactions.obligation.deposit",
                  "instructions": [{ "program": "KLend", "instruction": "refreshReserve" },
                                   { "program": "KLend", "instruction": "depositReserveLiquidityAndObligationCollateral" }] },
    });
    let (api, seen) = stub_api(vec![(
        "POST /api/sandbox/v1/actions/call/preview",
        200,
        passed.clone(),
    )]);
    let input = home.path().join("deposit.json");
    std::fs::write(&input, r#"{"reserve":"R1","amount":{"ui":"0.1"}}"#).unwrap();
    let at_input = format!("@{}", input.display());

    let output = a4(
        home.path(),
        &api,
        &[
            "--json",
            "sandbox",
            "preview",
            "--call",
            "KLend",
            "transactions.obligation.deposit",
            "--input",
            &at_input,
        ],
    );
    assert!(output.status.success(), "{output:?}");
    // The result exactly as the API (and the MCP tool) gives it.
    assert_eq!(json_out(&output), passed);
    let requests = action_requests(&seen);
    assert_eq!(requests.len(), 1, "{requests:?}");
    assert!(requests[0]
        .line
        .starts_with("POST /api/sandbox/v1/actions/call/preview "));
    assert_eq!(
        requests[0].body,
        json!({ "program": "KLend", "function": "transactions.obligation.deposit",
                "input": { "reserve": "R1", "amount": { "ui": "0.1" } } })
    );
    // A dry run carries no idempotency key, and uses the sandbox token.
    assert_eq!(idempotency_header(&requests[0]), None);
    assert!(requests[0].head.contains("bearer tok-1"));

    // Without --json: the verdict, the wallet's change and the plan.
    let output = a4(
        home.path(),
        &api,
        &[
            "sandbox",
            "preview",
            "--call",
            "KLend",
            "transactions.obligation.deposit",
            "--input",
            &at_input,
        ],
    );
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("call transactions.obligation.deposit: passes"),
        "{stdout}"
    );
    assert!(stdout.contains("-0.100005 SOL"), "{stdout}");
    assert!(stdout.contains("execute-v1"), "{stdout}");
    assert!(stdout.contains("refreshReserve"), "{stdout}");
    assert!(stdout.contains("a4 sandbox invoke"), "{stdout}");
}

#[test]
fn a_withdraw_has_a_dry_run_and_runs_under_the_given_key() {
    let home = tempfile::tempdir().unwrap();
    let landed = json!({
        "actionId": "a-1", "status": "landed", "signature": "Sig1",
        "walletLamportsChange": "-50005000", "outflowLamports": "50005000",
        "guardPipeline": "execute-v1",
    });
    let (api, seen) = stub_api(vec![
        (
            "POST /api/sandbox/v1/actions/withdraw/preview",
            200,
            json!({ "pass": true, "outflowLamports": "50005000" }),
        ),
        ("POST /api/sandbox/v1/actions/withdraw", 200, landed.clone()),
    ]);

    let output = a4(
        home.path(),
        &api,
        &[
            "--json",
            "sandbox",
            "preview",
            "--withdraw",
            "50000000",
            "--transaction-format",
            "v0",
        ],
    );
    assert!(output.status.success(), "{output:?}");
    assert_eq!(json_out(&output)["pass"], true);

    let output = a4(
        home.path(),
        &api,
        &[
            "--json",
            "sandbox",
            "invoke",
            "--withdraw",
            "50000000",
            "--idempotency-key",
            "withdraw-1",
        ],
    );
    assert!(output.status.success(), "{output:?}");
    let mut expected = landed;
    expected["idempotencyKey"] = json!("withdraw-1");
    assert_eq!(json_out(&output), expected);

    let requests = action_requests(&seen);
    assert_eq!(requests.len(), 2, "{requests:?}");
    assert!(requests[0]
        .line
        .starts_with("POST /api/sandbox/v1/actions/withdraw/preview "));
    // u64 amounts go as decimal strings.
    assert_eq!(
        requests[0].body,
        json!({ "lamports": "50000000", "transactionFormat": "v0" })
    );
    assert_eq!(idempotency_header(&requests[0]), None);
    assert!(requests[1]
        .line
        .starts_with("POST /api/sandbox/v1/actions/withdraw "));
    assert_eq!(requests[1].body, json!({ "lamports": "50000000" }));
    assert_eq!(
        idempotency_header(&requests[1]).as_deref(),
        Some("withdraw-1")
    );
}

#[test]
fn an_invoke_takes_its_instructions_from_stdin_with_a_generated_key() {
    let home = tempfile::tempdir().unwrap();
    let (api, seen) = stub_api(vec![(
        "POST /api/sandbox/v1/actions/invoke",
        200,
        json!({ "actionId": "a-2", "status": "landed" }),
    )]);
    let instructions =
        r#"[{"program":"pump_amm","instruction":"buy","args":{"base_amount_out":"1000"}}]"#;
    let output = a4_with_stdin(
        home.path(),
        &api,
        &[
            "--json",
            "sandbox",
            "invoke",
            "--invoke",
            "-",
            "--compute-unit-limit",
            "200000",
            "--compute-unit-price",
            "1000",
        ],
        Some(instructions),
    );
    assert!(output.status.success(), "{output:?}");
    let body = json_out(&output);
    let key = body["idempotencyKey"].as_str().unwrap().to_string();
    assert!(key.starts_with("a4-"), "{key}");

    let requests = action_requests(&seen);
    assert_eq!(requests.len(), 1, "{requests:?}");
    assert!(requests[0]
        .line
        .starts_with("POST /api/sandbox/v1/actions/invoke "));
    // The instructions array alone becomes `{ instructions }`.
    assert_eq!(
        requests[0].body,
        json!({
            "instructions": serde_json::from_str::<Value>(instructions).unwrap(),
            "computeUnitLimit": 200000,
            "computeUnitPriceMicroLamports": "1000",
        })
    );
    // The key printed is the key sent.
    assert_eq!(
        idempotency_header(&requests[0]).as_deref(),
        Some(key.as_str())
    );
}

#[test]
fn an_action_outside_the_grant_is_sent_and_refused_by_the_api() {
    let home = tempfile::tempdir().unwrap();
    let (api, seen) = stub_api(vec![(
        "POST /api/sandbox/v1/actions/withdraw",
        403,
        not_granted(),
    )]);
    let output = a4(
        home.path(),
        &api,
        &["--json", "sandbox", "invoke", "--withdraw", "5"],
    );
    // An auth refusal exits 77, as the other sandbox commands do.
    assert_eq!(output.status.code(), Some(77), "{output:?}");
    let shown = json_out(&output);
    assert_eq!(shown["status"], 403);
    assert_eq!(shown["code"], "sandbox-action-not-granted");
    assert_eq!(shown["rule"], "session-grant");
    assert_eq!(shown["requestId"], "req-42");
    assert_eq!(shown["retryable"], false);
    assert!(shown["idempotencyKey"].as_str().unwrap().starts_with("a4-"));
    // The hint names the CLI command, not the MCP tool.
    let hint = shown["hint"].as_str().unwrap();
    assert!(hint.contains("`a4 sandbox status`"), "{hint}");
    assert!(!hint.contains("sandbox_status"), "{hint}");
    // The request went out: the API decided.
    assert_eq!(action_requests(&seen).len(), 1);

    // Without --json, the same details on stderr.
    let output = a4(home.path(), &api, &["sandbox", "invoke", "--withdraw", "5"]);
    assert_eq!(output.status.code(), Some(77), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("(sandbox-action-not-granted)"), "{stderr}");
    assert!(stderr.contains("session-grant"), "{stderr}");
    assert!(stderr.contains("req-42"), "{stderr}");
    assert!(stderr.contains("Idempotency key"), "{stderr}");
}

#[test]
fn a_refused_call_shows_the_stages_plan_and_guard_pipeline() {
    let home = tempfile::tempdir().unwrap();
    let (api, _seen) = stub_api(vec![(
        "POST /api/sandbox/v1/actions/call/preview",
        422,
        refused_call(),
    )]);
    let args = [
        "sandbox",
        "preview",
        "--call",
        "KLend",
        "transactions.obligation.deposit",
        "--input",
        r#"{"reserve":"R1","amount":"0.1"}"#,
    ];
    let mut with_json = vec!["--json"];
    with_json.extend(args);
    let output = a4(home.path(), &api, &with_json);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let shown = json_out(&output);
    // Every field of the refusal, as the API sent it.
    for (key, value) in refused_call().as_object().unwrap() {
        if key != "hint" {
            assert_eq!(&shown[key], value, "{key}");
        }
    }
    assert_eq!(shown["status"], 422);
    assert!(shown["hint"]
        .as_str()
        .unwrap()
        .contains("`a4 sandbox rules show`"));
    // A dry run has no idempotency key.
    assert!(shown.get("idempotencyKey").is_none());

    let output = a4(home.path(), &api, &args);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    for wanted in [
        "(sandbox-function-input-invalid)",
        "function-input",
        "request → plan (sandbox-function-input-invalid)",
        "plan: input.amount",
        "execute-v1",
        "req-7",
    ] {
        assert!(stderr.contains(wanted), "{wanted}: {stderr}");
    }
}

#[test]
fn a_preview_that_does_not_pass_exits_1() {
    let home = tempfile::tempdir().unwrap();
    let (api, _seen) = stub_api(vec![(
        "POST /api/sandbox/v1/actions/preview",
        200,
        json!({ "pass": false, "reason": "the guard's trial failed" }),
    )]);
    let output = a4(
        home.path(),
        &api,
        &[
            "--json",
            "sandbox",
            "preview",
            "--invoke",
            r#"{"instructions":[]}"#,
        ],
    );
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert_eq!(json_out(&output)["pass"], false);
}

#[test]
fn an_action_is_looked_up_by_its_idempotency_key() {
    let home = tempfile::tempdir().unwrap();
    let record = json!({ "id": "a-1", "status": "landed", "outflowLamports": "50005000" });
    let (api, seen) = stub_api(vec![(
        "GET /api/sandbox/v1/actions?idempotencyKey=withdraw-1",
        200,
        record.clone(),
    )]);
    let output = a4(
        home.path(),
        &api,
        &[
            "--json",
            "sandbox",
            "action",
            "--idempotency-key",
            "withdraw-1",
        ],
    );
    assert!(output.status.success(), "{output:?}");
    assert_eq!(json_out(&output), record);
    assert_eq!(action_requests(&seen).len(), 1);
}

#[test]
fn an_action_needs_exactly_one_of_call_invoke_and_withdraw() {
    let home = tempfile::tempdir().unwrap();
    let (api, seen) = stub_api(vec![]);
    for args in [
        vec!["sandbox", "preview"],
        vec!["sandbox", "invoke", "--withdraw", "5", "--invoke", "{}"],
        vec!["sandbox", "preview", "--input", "{}", "--withdraw", "5"],
        vec![
            "sandbox",
            "invoke",
            "--withdraw",
            "5",
            "--compute-unit-limit",
            "1",
        ],
        vec!["sandbox", "preview", "--call", "KLend"],
    ] {
        let output = a4(home.path(), &api, &args);
        assert_eq!(output.status.code(), Some(2), "{args:?}: {output:?}");
    }
    // Bad JSON is refused before anything is sent.
    let output = a4(
        home.path(),
        &api,
        &["sandbox", "preview", "--invoke", "{not json"],
    );
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("--invoke"),
        "{output:?}"
    );
    assert!(action_requests(&seen).is_empty());
}
