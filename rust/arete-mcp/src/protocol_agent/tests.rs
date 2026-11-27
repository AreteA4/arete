use std::io::{BufRead, BufReader, Read};
use std::sync::{Arc, Mutex};

use ed25519_dalek::SigningKey;

use super::*;
use crate::prepared_summary::tests::{legacy_transaction, legacy_transaction_with};
use crate::prepared_summary::TOKEN_PROGRAMS;

const PROGRAM: [u8; 32] = [4; 32];
const TOKEN_ACCOUNT: [u8; 32] = [3; 32];
const MINT: [u8; 32] = [5; 32];

fn b58(bytes: &[u8]) -> String {
    bs58::encode(bytes).into_string()
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// One request the mock saw.
#[derive(Debug, Clone)]
struct Seen {
    method: String,
    path: String,
    body: Value,
}

#[derive(Default)]
struct MockState {
    seen: Vec<Seen>,
    /// Lamports the simulated transaction takes from the signer.
    sol_out: u64,
    /// Base units of MINT the simulated transaction takes from the signer.
    tokens_out: u64,
    /// The program prepared transactions invoke.
    program: Option<[u8; 32]>,
    /// The instruction's accounts and data (default: the signer, `[9]`).
    instruction: Option<(Vec<u8>, Vec<u8>)>,
    /// Extra writable accounts the prepared transaction names.
    extra_writable: u8,
    /// Inner instructions the simulation returns.
    inner: Option<Value>,
    /// The token account's mint (default: MINT).
    mint: Option<[u8; 32]>,
    /// Whether the simulated transaction hands the token account to
    /// another owner.
    handover: bool,
    /// A request (method, path) the mock reads and never answers.
    stall: Option<(&'static str, &'static str)>,
    /// What a submit answers instead of taking the transactions.
    submit_answer: Option<(u16, Value)>,
    /// What reading an execution intent answers instead of `completed`.
    intent_answer: Option<(u16, Value)>,
    /// The `expiresAt` a prepare answers (default: 1_900_000_000).
    expires_at: Option<Value>,
}

type Shared = Arc<Mutex<MockState>>;

fn account(lamports: u64, owner: &str, data: &[u8]) -> Value {
    json!({ "lamports": lamports, "owner": owner, "data": [b64(data), "base64"], "executable": false, "rentEpoch": 0 })
}

fn token_data(mint: &[u8; 32], owner: &str, amount: u64) -> Vec<u8> {
    let mut data = Vec::with_capacity(165);
    data.extend_from_slice(mint);
    data.extend_from_slice(&bs58::decode(owner).into_vec().unwrap());
    data.extend_from_slice(&amount.to_le_bytes());
    data.resize(165, 0);
    data
}

fn rpc_accounts(addresses: &Value, signer: &str, state: &MockState, post: bool) -> Value {
    let token_account = b58(&TOKEN_ACCOUNT);
    Value::Array(
        addresses
            .as_array()
            .unwrap()
            .iter()
            .map(|address| {
                let address = address.as_str().unwrap();
                if address == signer {
                    let lamports = if post {
                        10_000_000_000 - state.sol_out
                    } else {
                        10_000_000_000
                    };
                    account(lamports, "11111111111111111111111111111111", &[])
                } else if address == token_account {
                    let amount = if post {
                        1_000_000 - state.tokens_out
                    } else {
                        1_000_000
                    };
                    let owner = if post && state.handover {
                        b58(&[77; 32])
                    } else {
                        signer.to_string()
                    };
                    let mint = state.mint.unwrap_or(MINT);
                    account(
                        2_039_280,
                        TOKEN_PROGRAMS[0],
                        &token_data(&mint, &owner, amount),
                    )
                } else {
                    Value::Null
                }
            })
            .collect(),
    )
}

fn respond(state: &Shared, method: &str, path: &str, body: &Value) -> (u16, Value) {
    let mut guard = state.lock().unwrap();
    guard.seen.push(Seen {
        method: method.into(),
        path: path.into(),
        body: body.clone(),
    });
    let (path_only, _) = path.split_once('?').unwrap_or((path, ""));
    if path_only == "/ws/sessions" {
        return (200, json!({ "token": "session-token" }));
    }
    if path_only == "/api/protocols/check" {
        if body["spec"]["name"] == "proxied" {
            // A 200 that is not a check result (a proxy page, a wrong route).
            return (200, json!({ "ok": true }));
        }
        if body["spec"]["name"] == "broken" {
            return (
                400,
                json!({
                    "schemaVersion": 1,
                    "error": "The spec's templates do not resolve against their pinned ProgramSpecs",
                    "code": "protocol-template-check-failed",
                    "retryable": false,
                    "templateCheck": { "errors": [{
                        "code": "template_unknown_instruction",
                        "path": "/steps/open/build/instructions/0/instruction",
                        "message": "'vault' has no instruction 'withdraw'"
                    }] }
                }),
            );
        }
        return (
            200,
            json!({
                "schema": "arete.protocol-release-check/v1",
                "releaseHash": "arete:h1:protocol-release:sha256:00",
                "hasHandlers": body.get("bundleBase64").is_some(),
                "templateCheck": "passed"
            }),
        );
    }
    if path_only == "/rpc" {
        let params = &body["params"];
        let result = match body["method"].as_str().unwrap_or_default() {
            "getBalance" => json!({ "value": 5 }),
            "getMultipleAccounts" => {
                let signer = guard_signer(&guard);
                json!({ "value": rpc_accounts(&params[0], &signer, &guard, false) })
            }
            "simulateTransaction" => {
                let signer = guard_signer(&guard);
                json!({ "value": {
                    "err": null,
                    "logs": ["Program log: ok"],
                    "accounts": rpc_accounts(&params[1]["accounts"]["addresses"], &signer, &guard, true),
                    "innerInstructions": guard.inner.clone().unwrap_or(json!([])),
                } })
            }
            "getTransaction" => receipt_case("legacy-wire")["transaction"]
                .as_str()
                .map(|wire| json!({ "slot": 312_000_123, "transaction": [wire, "base64"], "meta": { "err": null } }))
                .unwrap_or(Value::Null),
            _ => Value::Null,
        };
        return (200, json!({ "jsonrpc": "2.0", "id": 1, "result": result }));
    }
    if path_only == "/api/protocol-receipts/keys" {
        return (200, receipt_vectors()["keySet"].clone());
    }
    if let Some(signature) = path_only.strip_prefix("/api/protocol-receipts/") {
        let receipt = receipt_case("legacy-wire")["receipt"].clone();
        if receipt["signature"].as_str() == Some(signature) {
            return (200, receipt);
        }
        return (
            404,
            json!({ "error": "no receipt", "code": "protocol-receipt-not-found" }),
        );
    }
    if let Some(route) = path_only.strip_prefix("/api/account/squads") {
        let proposer = b58(&[61; 32]);
        let multisig = b58(&[62; 32]);
        let attachment = |state: &str| {
            json!({
                "address": multisig, "label": "Treasury", "state": state,
                "problem": if state == "active" { Value::Null } else { json!("not_member") },
                "createdVia": "page", "stateChangedAt": "2026-10-24T00:00:00Z",
                "createdAt": "2026-10-24T00:00:00Z", "threshold": 2, "members": null,
                "proposerMask": null, "vault": b58(&[64; 32]),
                "squadsUrl": "https://app.squads.so/x",
                "dashboardUrl": format!("https://app.arete.run/dashboard/squads/{multisig}"),
                "deploymentConfigSnippet": "multisig = \"...\"", "deployments": []
            })
        };
        return match (method, route) {
            ("GET", "") => (
                200,
                json!({
                    "setup": "needsAttention",
                    "proposer": { "pubkey": proposer, "permissions": "initiate", "lamports": 5 },
                    "dashboardUrl": "https://app.arete.run/dashboard/squads",
                    "multisigs": [attachment("pending")]
                }),
            ),
            ("GET", "/notifications") => (200, json!([])),
            ("POST", "/multisigs") => (201, attachment("pending")),
            ("GET", r) if r == format!("/multisigs/{multisig}") => {
                let mut detail = attachment("pending");
                detail["proposer"] = json!(proposer);
                detail["proposals"] = json!([]);
                (200, detail)
            }
            _ => (
                404,
                json!({ "error": "not attached", "code": "protocol-squads-multisig-not-found" }),
            ),
        };
    }
    if let Some(route) = path_only.strip_prefix("/api/protocol-deployments/d-1/squads/") {
        let proposer = b58(&[61; 32]);
        return match route {
            "proposer" => (
                200,
                json!({ "pubkey": proposer, "permissions": "initiate", "rootKeyId": "rk-1" }),
            ),
            "status" => {
                let multisig = path.split("multisig=").nth(1).unwrap_or_default();
                if multisig == b58(&[62; 32]) {
                    (
                        200,
                        json!({ "ready": true, "member": true, "mask": 1, "threshold": 2,
                        "multisig": multisig, "proposer": proposer }),
                    )
                } else {
                    (
                        200,
                        json!({ "ready": false, "member": false, "mask": null, "threshold": 2,
                        "multisig": multisig, "proposer": proposer, "problem": "not_member" }),
                    )
                }
            }
            "add-proposer" => panic!("Arete builds no add-proposer transaction"),
            _ => (404, json!({ "error": "not found" })),
        };
    }
    if path_only == "/api/registry/protocols/tip-jar/install" {
        return (
            200,
            json!({ "protocol": { "deployment": { "deploymentId": "d-1", "access": "public" } } }),
        );
    }
    if path_only.ends_with("/actions") {
        return (
            200,
            json!({
                "deploymentId": "d-1",
                "wallet": path_only.split('/').nth(5),
                "createSteps": [{ "step": "open", "allowed": true, "scope": "create",
                    "args": [{ "name": "amount", "type": "string", "format": "u64", "optional": false }] }],
                "instances": [{ "instanceId": "i-1", "state": "open", "relation": ["creator"],
                    "participantState": null,
                    "steps": [
                        { "step": "tip", "allowed": true, "scope": "instance", "args": [] },
                        { "step": "close", "allowed": true, "scope": "instance", "args": [] }
                    ] }],
                "nextCursor": null,
            }),
        );
    }
    if path_only == "/api/protocol-deployments/d-1/instances" {
        return (
            200,
            json!({ "instances": [{ "id": "i-2", "state": "open" }], "nextCursor": null }),
        );
    }
    if let Some(id) = path_only.strip_prefix("/api/protocol-instances/") {
        if !id.contains('/') {
            return (
                200,
                json!({ "id": id, "state": "open", "allowedSteps": [] }),
            );
        }
    }
    if path_only.ends_with("/prepare") {
        if path_only.contains("/steps/blocked/") {
            return (
                422,
                json!({ "rejected": "raise_closed", "step": "blocked", "guard": "instance.state == 'open'", "detail": { "logs": [] } }),
            );
        }
        let wallet: [u8; 32] = bs58::decode(body["wallet"].as_str().unwrap())
            .into_vec()
            .unwrap()
            .try_into()
            .unwrap();
        let program = guard.program.unwrap_or(PROGRAM);
        let mut writable = vec![TOKEN_ACCOUNT];
        writable.extend((0..guard.extra_writable).map(|n| [n.wrapping_add(100); 32]));
        let tx = match &guard.instruction {
            Some((accounts, data)) => {
                legacy_transaction_with(&[wallet], &[false], &writable, &[program], accounts, data)
            }
            None => legacy_transaction(&[wallet], &[false], &writable, &[program]),
        };
        let instance = path_only
            .strip_prefix("/api/protocol-instances/")
            .and_then(|rest| rest.split('/').next())
            .unwrap_or("i-new");
        return (
            200,
            json!({
                "ok": true,
                "status": "prepared",
                "intent": { "executionId": "e-1", "instanceId": instance,
                    "expiresAt": guard.expires_at.clone().unwrap_or(json!(1_900_000_000)),
                    "deployment": { "deploymentId": "d-1" } },
                "intentHash": "h",
                "transactions": [{ "index": 0, "transactionBase64": b64(&tx), "messageSha256": "m",
                    "requiredWalletSigners": [b58(&wallet)] }],
                "anatomy": [{ "index": 0, "memo": "", "instructions": [
                    { "role": "program", "kind": "program", "programId": b58(&program) }
                ] }],
                "simulation": [{ "index": 0, "unitsConsumed": "1200" }],
            }),
        );
    }
    if path_only.ends_with("/submit") {
        if let Some(answer) = guard.submit_answer.clone() {
            return answer;
        }
        // Taken: the intent reads as its default from here on.
        guard.intent_answer = None;
        return (
            200,
            json!({ "executionId": "e-1", "state": "submitting", "transactions": [] }),
        );
    }
    if path_only.starts_with("/api/execution-intents/") {
        if let Some(answer) = guard.intent_answer.clone() {
            return answer;
        }
        return (
            200,
            json!({ "executionId": "e-1", "state": "completed", "instanceId": "i-new",
                "transactions": [{ "index": 0, "signature": "sig", "state": "confirmed" }] }),
        );
    }
    (404, json!({ "error": "not found" }))
}

fn receipt_vectors() -> Value {
    serde_json::from_str(include_str!(
        "../../../../test-vectors/protocol-client/execution-receipt-v1.json"
    ))
    .unwrap()
}

fn receipt_case(name: &str) -> Value {
    receipt_vectors()["onChainCases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["name"] == name)
        .cloned()
        .unwrap()
}

/// The wallet of the latest prepare (the signer the RPC answers for).
fn guard_signer(state: &MockState) -> String {
    state
        .seen
        .iter()
        .rev()
        .find(|seen| seen.path.ends_with("/prepare"))
        .and_then(|seen| seen.body["wallet"].as_str())
        .unwrap_or_default()
        .to_string()
}

fn mock_api() -> (String, Shared) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let state: Shared = Arc::default();
    let shared = state.clone();
    std::thread::spawn(move || {
        // Stalled requests' connections, open until the mock ends.
        let mut held = Vec::new();
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut first = String::new();
            if reader.read_line(&mut first).is_err() {
                continue;
            }
            let mut parts = first.split_whitespace();
            let method = parts.next().unwrap_or("GET").to_string();
            let path = parts.next().unwrap_or("/").to_string();
            let mut length = 0usize;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line.trim().is_empty() {
                    break;
                }
                if let Some((name, value)) = line.split_once(':') {
                    if name.eq_ignore_ascii_case("content-length") {
                        length = value.trim().parse().unwrap();
                    }
                }
            }
            let mut body = vec![0u8; length];
            reader.read_exact(&mut body).unwrap();
            let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
            {
                let mut guard = shared.lock().unwrap();
                if guard.stall == Some((method.as_str(), path.as_str())) {
                    guard.seen.push(Seen { method, path, body });
                    held.push(stream);
                    continue;
                }
            }
            let (status, response) = respond(&shared, &method, &path, &body);
            let text = response.to_string();
            let _ = std::io::Write::write_all(
                &mut stream,
                format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                    text.len()
                )
                .as_bytes(),
            );
        }
    });
    (origin, state)
}

fn hosted(deployment: Option<&str>) -> TargetArgs {
    TargetArgs {
        deployment: deployment.map(str::to_string),
        ..TargetArgs::default()
    }
}

fn seen(state: &Shared) -> Vec<Seen> {
    state.lock().unwrap().seen.clone()
}

const SEED: u8 = 42;

/// A signer whose keypair file is in `dir`, with `policy` (TOML lines).
fn signer(dir: &Path, origin: &str, policy: &str) -> (LocalSigner, SigningKey) {
    signer_with_rpc(dir, &format!("{origin}/rpc"), policy)
}

/// [`signer`] with its `rpcUrl` at `rpc`.
fn signer_with_rpc(dir: &Path, rpc: &str, policy: &str) -> (LocalSigner, SigningKey) {
    let key = SigningKey::from_bytes(&[SEED; 32]);
    let mut bytes = vec![SEED; 32];
    bytes.extend_from_slice(key.verifying_key().as_bytes());
    let path = dir.join("agent.json");
    std::fs::write(&path, serde_json::to_string(&bytes).unwrap()).unwrap();
    let text =
        format!("[signer]\nkeypair = {path:?}\nrpcUrl = \"{rpc}\"\n[signer.policy]\n{policy}");
    (LocalSigner::from_toml(&text).unwrap().unwrap(), key)
}

/// Every textual form of the signer's secret this server could leak.
fn secret_forms(key: &SigningKey) -> Vec<String> {
    let keypair = key.to_keypair_bytes();
    let seed = key.to_bytes();
    vec![
        serde_json::to_string(&keypair.to_vec()).unwrap(),
        serde_json::to_string(&seed.to_vec()).unwrap(),
        format!("{:?}", keypair.to_vec()),
        format!("{:?}", seed.to_vec()),
        b58(&keypair),
        b58(&seed),
        b64(&keypair),
        b64(&seed),
        seed.iter().map(|byte| format!("{byte:02x}")).collect(),
    ]
}

fn assert_no_secret(text: &str, key: &SigningKey) {
    for form in secret_forms(key) {
        assert!(!text.contains(&form), "the signer secret leaked: {text}");
    }
}

#[tokio::test]
async fn actions_resolve_protocol_refs_and_remember_instances() {
    let (origin, state) = mock_api();
    let agent = ProtocolAgent::new(None).with_api(&origin, "a4_sk_test");
    let wallet = b58(&[9; 32]);
    let actions = agent
        .actions(
            &TargetArgs {
                protocol: Some("tip-jar".into()),
                ..TargetArgs::default()
            },
            &wallet,
            None,
            Some("c 1"),
        )
        .await
        .unwrap();
    assert_eq!(actions["createSteps"][0]["scope"], "create");
    assert_eq!(actions["createSteps"][0]["args"][0]["name"], "amount");
    assert_eq!(
        actions["instances"][0]["steps"].as_array().unwrap().len(),
        2
    );
    assert_eq!(actions["target"], "arete");
    let requests = seen(&state);
    let read = requests
        .iter()
        .find(|seen| seen.path.contains("/actions"))
        .unwrap();
    assert_eq!(
        read.path,
        format!("/api/protocol-deployments/d-1/wallets/{wallet}/actions?cursor=c%201")
    );
    let mint = requests
        .iter()
        .find(|seen| seen.path == "/ws/sessions")
        .unwrap();
    assert_eq!(mint.body["scopes"], json!(["protocol:read"]));
    assert_eq!(mint.body["targetId"], "d-1");

    // The instance the actions listed needs no deployment afterwards.
    let instance = agent
        .instance(&TargetArgs::default(), "i-1", Some(&wallet))
        .await
        .unwrap();
    assert_eq!(instance["id"], "i-1");
    assert!(seen(&state)
        .iter()
        .any(|seen| seen.path == format!("/api/protocol-instances/i-1?wallet={wallet}")));

    // An unknown instance without a deployment is the caller's to fix.
    let error = agent
        .instance(&TargetArgs::default(), "i-unknown", None)
        .await
        .unwrap_err();
    assert!(error.downcast_ref::<InvalidArgs>().is_some(), "{error}");
    let error = agent
        .actions(&hosted(Some("d-1")), "not-a-key", None, None)
        .await
        .unwrap_err();
    assert!(error.downcast_ref::<InvalidArgs>().is_some(), "{error}");

    let page = agent
        .instances(&hosted(Some("d-1")), Some("open"), None)
        .await
        .unwrap();
    assert_eq!(page["instances"][0]["id"], "i-2");
    assert!(seen(&state)
        .iter()
        .any(|seen| seen.path == "/api/protocol-deployments/d-1/instances?state=open"));
}

#[tokio::test]
async fn prepare_returns_unsigned_transactions_and_submit_takes_them_signed() {
    let (origin, state) = mock_api();
    let agent = ProtocolAgent::new(None).with_api(&origin, "a4_sk_test");
    let key = SigningKey::from_bytes(&[7; 32]);
    let wallet = b58(key.verifying_key().as_bytes());

    let rejected = agent
        .prepare(
            &hosted(Some("d-1")),
            &StepCall {
                step: "blocked",
                instance: Some("i-1"),
                args: &json!({}),
                transaction_format: None,
            },
            &wallet,
        )
        .await
        .unwrap();
    assert_eq!(rejected["state"], "rejected");
    assert_eq!(rejected["rejected"], "raise_closed");
    assert_eq!(rejected["guard"], "instance.state == 'open'");

    let prepared = agent
        .prepare(
            &hosted(Some("d-1")),
            &StepCall {
                step: "tip",
                instance: Some("i-1"),
                args: &json!({ "amount": "5" }),
                transaction_format: None,
            },
            &wallet,
        )
        .await
        .unwrap();
    assert_eq!(prepared["state"], "prepared");
    assert_eq!(prepared["executionId"], "e-1");
    let summary = &prepared["summary"];
    assert_eq!(summary["programsInvoked"], json!([b58(&PROGRAM)]));
    assert_eq!(summary["signaturesStillRequired"], json!([wallet]));
    assert_eq!(
        summary["transactions"][0]["accountsWritten"],
        json!([wallet, b58(&TOKEN_ACCOUNT)])
    );
    assert_eq!(summary["signerBalanceChanges"]["available"], false);
    let prepare = seen(&state)
        .into_iter()
        .find(|seen| seen.path.ends_with("/steps/tip/prepare"))
        .unwrap();
    assert_eq!(
        prepare.path,
        "/api/protocol-instances/i-1/steps/tip/prepare"
    );
    assert_eq!(prepare.body["transactionFormat"], "v0");
    assert_eq!(prepare.body["args"], json!({ "amount": "5" }));

    // Unsigned transactions are refused before they are sent.
    let unsigned = prepared["transactions"][0]["transactionBase64"]
        .as_str()
        .unwrap()
        .to_string();
    let error = agent
        .submit(
            &TargetArgs::default(),
            "e-1",
            std::slice::from_ref(&unsigned),
            None,
            None,
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("lacks the signature"), "{error}");
    assert!(error.downcast_ref::<InvalidArgs>().is_some());

    // Signed outside the server: the execution is remembered, so no
    // deployment is needed.
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(unsigned)
        .unwrap();
    let signed = b64(&crate::dev::sign_transaction(&bytes, &key).unwrap());
    let result = agent
        .submit(
            &TargetArgs::default(),
            "e-1",
            std::slice::from_ref(&signed),
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(result["state"], "completed");
    assert_eq!(result["settled"], true);
    assert_eq!(result["step"], "tip");
    assert_eq!(result["instanceId"], "i-1");
    let submit = seen(&state)
        .into_iter()
        .find(|seen| seen.path.ends_with("/submit"))
        .unwrap();
    assert_eq!(submit.method, "POST");
    assert_eq!(submit.body["signedTransactions"], json!([signed]));

    let intent = agent.intent(&TargetArgs::default(), "e-1").await.unwrap();
    assert_eq!(intent["state"], "completed");
}

#[tokio::test]
async fn execute_signs_within_policy_and_refuses_outside_it() {
    let (origin, state) = mock_api();
    let dir = tempfile::tempdir().unwrap();
    let mint = b58(&MINT);
    let (signer, key) = signer(
        dir.path(),
        &origin,
        &format!(
            "deployments = [\"d-1\"]\nmaxSolPerTransaction = 0.01\nprograms = [\"{}\"]\n[signer.policy.maxTokenAmounts]\n\"{mint}\" = \"500\"\n",
            b58(&PROGRAM)
        ),
    );
    let pubkey = signer.pubkey();
    let mut agent = ProtocolAgent::new(Some(signer)).with_api(&origin, "a4_sk_test");
    // No hold for completed runs, so each run of the step is a new
    // execution.
    agent.completed_hold = (0, 0);
    let call = StepCall {
        step: "tip",
        instance: Some("i-1"),
        args: &json!({}),
        transaction_format: None,
    };
    let mut outputs = Vec::new();

    // Within policy: 0.005 SOL and 100 tokens out.
    {
        let mut guard = state.lock().unwrap();
        guard.sol_out = 5_000_000;
        guard.tokens_out = 100;
    }
    let result = agent
        .execute(&hosted(Some("d-1")), &call, None, None)
        .await
        .unwrap();
    outputs.push(result.to_string());
    assert_eq!(result["state"], "completed", "{result}");
    assert_eq!(
        result["summary"]["signerBalanceChanges"]["solLamports"],
        "-5000000"
    );
    assert_eq!(
        result["summary"]["signerBalanceChanges"]["tokens"],
        json!([{ "mint": mint, "amount": "-100" }])
    );
    let submit = seen(&state)
        .into_iter()
        .rev()
        .find(|seen| seen.path.ends_with("/submit"))
        .unwrap();
    let signed = base64::engine::general_purpose::STANDARD
        .decode(submit.body["signedTransactions"][0].as_str().unwrap())
        .unwrap();
    let decoded = decode_transaction(&signed).unwrap();
    assert_eq!(decoded.signers(), vec![(pubkey.clone(), true)]);
    let signature = ed25519_dalek::Signature::from_slice(&decoded.signatures[0]).unwrap();
    key.verifying_key()
        .verify_strict(&signed[decoded.message.clone()], &signature)
        .unwrap();

    let submits = |state: &Shared| {
        seen(state)
            .iter()
            .filter(|seen| seen.path.ends_with("/submit"))
            .count()
    };
    let before = submits(&state);

    // Over the SOL cap.
    state.lock().unwrap().sol_out = 20_000_000;
    let refused = agent
        .execute(&hosted(Some("d-1")), &call, None, None)
        .await
        .unwrap();
    outputs.push(refused.to_string());
    assert_eq!(refused["state"], "refused");
    assert_eq!(refused["rule"], "maxSolPerTransaction");
    assert!(refused["reason"].as_str().unwrap().contains("0.02 SOL"));

    // Over the token cap.
    {
        let mut guard = state.lock().unwrap();
        guard.sol_out = 0;
        guard.tokens_out = 501;
    }
    let refused = agent
        .execute(&hosted(Some("d-1")), &call, None, None)
        .await
        .unwrap();
    outputs.push(refused.to_string());
    assert_eq!(refused["rule"], "maxTokenAmounts");

    // A program outside the allow-list.
    {
        let mut guard = state.lock().unwrap();
        guard.tokens_out = 0;
        guard.program = Some([6; 32]);
    }
    let refused = agent
        .execute(&hosted(Some("d-1")), &call, None, None)
        .await
        .unwrap();
    outputs.push(refused.to_string());
    assert_eq!(refused["rule"], "programs");
    state.lock().unwrap().program = None;

    // A deployment outside the allow-list is refused before preparing.
    let prepares = seen(&state)
        .iter()
        .filter(|s| s.path.ends_with("/prepare"))
        .count();
    let refused = agent
        .execute(&hosted(Some("d-2")), &call, None, None)
        .await
        .unwrap();
    outputs.push(refused.to_string());
    assert_eq!(refused["rule"], "deployments");
    assert_eq!(
        seen(&state)
            .iter()
            .filter(|s| s.path.ends_with("/prepare"))
            .count(),
        prepares
    );
    assert_eq!(submits(&state), before, "a refused step was submitted");

    // Guard rejections stay results.
    let rejected = agent
        .execute(
            &hosted(Some("d-1")),
            &StepCall {
                step: "blocked",
                ..call.clone()
            },
            None,
            None,
        )
        .await
        .unwrap();
    outputs.push(rejected.to_string());
    assert_eq!(rejected["state"], "rejected");

    // Errors and every request the server made carry no secret either.
    let error = agent
        .execute(&hosted(None), &call, None, None)
        .await
        .unwrap_err();
    outputs.push(format!("{error:#} {error:?}"));
    outputs.push(format!("{:?}", agent.signer().unwrap()));
    outputs.push(agent.signer().unwrap().describe().to_string());
    for request in seen(&state) {
        outputs.push(request.body.to_string());
    }
    for output in &outputs {
        assert_no_secret(output, &key);
    }
}

#[tokio::test]
async fn mainnet_needs_an_explicit_cluster() {
    let (origin, _state) = mock_api();
    let dir = tempfile::tempdir().unwrap();
    let (signer, _) = signer(
        dir.path(),
        &origin,
        "deployments = [\"d-1\"]\nmaxSolPerTransaction = 1\nclusters = [\"mainnet\"]\n",
    );
    // The mock is on loopback, so the target is a local runtime.
    let agent = ProtocolAgent::new(Some(signer)).with_api(&origin, "a4_sk_test");
    let refused = agent
        .execute(
            &hosted(Some("d-1")),
            &StepCall {
                step: "tip",
                instance: Some("i-1"),
                args: &json!({}),
                transaction_format: None,
            },
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(refused["rule"], "clusters");
    assert_eq!(refused["cluster"], "dev");
    assert!(is_loopback("http://127.0.0.1:4000"));
    assert!(is_loopback("http://localhost"));
    assert!(is_loopback("http://[::1]:80/api"));
    assert!(!is_loopback("https://api.arete.run"));
}

#[test]
fn the_execute_tool_is_listed_only_with_a_signer() {
    let names = |server: &crate::AreteMcp| -> Vec<String> {
        server
            .tool_names()
            .into_iter()
            .map(|name| name.to_string())
            .collect()
    };
    let without = crate::AreteMcp::with_protocol_agent(ProtocolAgent::new(None));
    let listed = names(&without);
    for tool in [
        "protocol_actions",
        "protocol_instances",
        "protocol_instance",
        "protocol_prepare",
        "protocol_submit",
        "protocol_intent",
        "protocol_squads_proposer",
        "protocol_squads_status",
        "squads_status",
        "squads_attach",
        "protocol_check",
        "dev_actions",
    ] {
        assert!(listed.iter().any(|name| name == tool), "{tool} missing");
    }
    assert!(!listed.iter().any(|name| name == "protocol_execute"));
    // Arete builds no account Squads transactions.
    for gone in [
        "squads_add_proposer",
        "squads_submit",
        "squads_create",
        "protocol_squads_add_proposer",
    ] {
        assert!(!listed.iter().any(|name| name == gone), "{gone} listed");
    }

    let dir = tempfile::tempdir().unwrap();
    let (signer, _) = signer(
        dir.path(),
        "http://127.0.0.1:1",
        "deployments = [\"d-1\"]\nmaxSolPerTransaction = 1\n",
    );
    let with = crate::AreteMcp::with_protocol_agent(ProtocolAgent::new(Some(signer)));
    assert!(names(&with).iter().any(|name| name == "protocol_execute"));
}

#[tokio::test]
async fn protocol_check_sends_the_built_release_and_reports_the_servers_errors() {
    let (origin, state) = mock_api();
    let agent = ProtocolAgent::new(None).with_api(&origin, "a4_sk_test");
    let project = tempfile::tempdir().unwrap();
    let build = project.path().join(".arete/protocol");
    std::fs::create_dir_all(&build).unwrap();
    std::fs::write(build.join("protocol-spec.json"), r#"{"name":"tips"}"#).unwrap();
    std::fs::write(build.join("handlers.mjs"), b"export {};\n").unwrap();

    let passed = agent
        .check(Some(project.path().to_str().unwrap()), None)
        .await
        .unwrap();
    assert_eq!(passed["passed"], true);
    assert_eq!(passed["hasHandlers"], true);
    let sent = seen(&state).pop().unwrap();
    assert_eq!(sent.method, "POST");
    assert_eq!(sent.path, "/api/protocols/check");
    assert_eq!(sent.body["spec"]["name"], "tips");
    assert_eq!(sent.body["bundleBase64"], b64(b"export {};\n"));

    let rejected = agent
        .check(None, Some(json!({ "name": "broken" })))
        .await
        .unwrap();
    assert_eq!(rejected["passed"], false);
    assert_eq!(rejected["code"], "protocol-template-check-failed");
    assert_eq!(
        rejected["errors"][0]["path"],
        "/steps/open/build/instructions/0/instruction"
    );
    assert!(seen(&state)
        .pop()
        .unwrap()
        .body
        .get("bundleBase64")
        .is_none());

    let proxied = agent
        .check(None, Some(json!({ "name": "proxied" })))
        .await
        .unwrap_err();
    assert!(
        proxied
            .to_string()
            .contains("without a release hash and template check status"),
        "{proxied}"
    );

    let missing = agent.check(Some("/nonexistent"), None).await.unwrap_err();
    assert!(
        missing.to_string().contains("Build the protocol first"),
        "{missing}"
    );
}

#[tokio::test]
async fn keys_are_only_sent_to_arete_origins() {
    for origin in ["https://evil.example", "http://api.arete.run"] {
        let agent = ProtocolAgent::new(None).with_api(origin, "a4_sk_test");
        let error = agent
            .instances(&hosted(Some("d-1")), None, None)
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("not a recognised Arete origin"),
            "{error}"
        );
    }
}

fn tip_call(args: &Value) -> StepCall<'_> {
    StepCall {
        step: "tip",
        instance: Some("i-1"),
        args,
        transaction_format: None,
    }
}

/// The mock's answers as new, keeping what it saw and its `expiresAt`.
fn reset(state: &Shared) {
    let mut guard = state.lock().unwrap();
    let seen = std::mem::take(&mut guard.seen);
    let expires_at = guard.expires_at.take();
    *guard = MockState {
        seen,
        expires_at,
        ..MockState::default()
    };
}

/// Once the signed transactions are sent, a stalled API never costs the
/// agent the execution id: a stalled status poll returns the execution,
/// unsettled, with `pollError` and the do-not-resubmit hint, and a submit
/// that stalls says to read the execution before running the step again.
#[tokio::test]
async fn execute_keeps_the_execution_id_when_the_api_stalls_after_sending() {
    let (origin, state) = mock_api();
    let dir = tempfile::tempdir().unwrap();
    let (signer, _) = signer(
        dir.path(),
        &origin,
        &format!(
            "deployments = [\"d-1\"]\nmaxSolPerTransaction = 0.01\nprograms = [\"{}\"]\n",
            b58(&PROGRAM)
        ),
    );
    let mut agent = ProtocolAgent::new(Some(signer)).with_api(&origin, "a4_sk_test");
    agent.http = reqwest::Client::builder()
        .timeout(Duration::from_millis(300))
        .build()
        .unwrap();
    let call = StepCall {
        step: "tip",
        instance: Some("i-1"),
        args: &json!({}),
        transaction_format: None,
    };
    let target = hosted(Some("d-1"));
    let execute = || {
        crate::deadline::watchdog(
            Duration::from_secs(10),
            agent.execute(&target, &call, None, None),
        )
    };

    state.lock().unwrap().stall = Some(("GET", "/api/execution-intents/e-1"));
    let result = execute().await.unwrap();
    assert!(
        seen(&state)
            .iter()
            .any(|seen| seen.method == "POST" && seen.path.ends_with("/e-1/submit")),
        "{result}"
    );
    assert_eq!(result["executionId"], "e-1", "{result}");
    assert_eq!(result["settled"], false, "{result}");
    assert_eq!(result["state"], "submitting", "{result}");
    assert_eq!(
        result["next"],
        "Not settled yet: read it with protocol_intent later. Do not resubmit."
    );
    let poll_error = result["pollError"].as_str().unwrap();
    assert!(
        poll_error.contains(&format!("{origin}/api/execution-intents/e-1")),
        "{poll_error}"
    );
    assert!(!result.to_string().contains("a4_sk_test"), "{result}");
    assert!(!result.to_string().contains("session-token"), "{result}");

    // The submit stalls, and one read finds the execution still prepared:
    // it may yet be taken.
    {
        let mut guard = state.lock().unwrap();
        guard.stall = Some(("POST", "/api/execution-intents/e-1/submit"));
        guard.intent_answer = Some((200, json!({ "executionId": "e-1", "state": "prepared" })));
    }
    let error = execute().await.unwrap_err();
    let unknown = error.downcast_ref::<MaybeSubmitted>().expect("unknown");
    assert_eq!(unknown.execution_id, "e-1");
    let error = error.to_string();
    assert!(
        error.contains(
            "; a read then showed it still `prepared`. Execution e-1 may have been submitted"
        ),
        "{error}"
    );
    assert!(!error.contains("a4_sk_test"), "{error}");
    assert!(!error.contains("session-token"), "{error}");
}

/// The mock as new: no overrides, nothing seen.
fn restart(state: &Shared) {
    *state.lock().unwrap() = MockState::default();
}

/// A signer agent on the mock, its policy allowing the `tip` step.
fn signing_agent(dir: &Path, origin: &str) -> ProtocolAgent {
    let (signer, _) = signer(
        dir,
        origin,
        &format!(
            "deployments = [\"d-1\"]\nmaxSolPerTransaction = 0.01\nprograms = [\"{}\"]\n",
            b58(&PROGRAM)
        ),
    );
    ProtocolAgent::new(Some(signer)).with_api(origin, "a4_sk_test")
}

/// The requests the mock saw with `method` whose path ends with `suffix`.
fn sent(state: &Shared, method: &str, suffix: &str) -> Vec<Seen> {
    seen(state)
        .into_iter()
        .filter(|seen| seen.method == method && seen.path.ends_with(suffix))
        .collect()
}

/// The `e-1` intent after the API took its transactions.
fn taken(state: &str) -> (u16, Value) {
    (
        200,
        json!({ "executionId": "e-1", "state": state, "instanceId": "i-1",
            "transactions": [{ "index": 0, "signature": "sig", "state": "confirmed" }] }),
    )
}

/// The guidance an unknown submit ends with, from `protocol_submit` (the
/// agent holds the signed transactions) or `protocol_execute` (they are in
/// the error's data).
fn unknown_guidance(execution: &str, expires_at: &str, execute: bool) -> String {
    let mut text = format!(
        "Execution {execution} may have been submitted, even if protocol_intent shows it \
         `prepared`. Do not prepare and sign the step again: resend the same signed \
         transactions with protocol_submit{}, which cannot land twice (the chain takes a \
         signed transaction once), or wait until protocol_intent shows the execution settled \
         or `expired` (expiresAt {expires_at}).",
        if execute {
            " (`signedTransactions` in this error's data)"
        } else {
            ""
        }
    );
    if execute {
        text.push_str(
            " Running protocol_execute again for this step with the same args resends them \
             itself, but restarting a4 mcp clears that guard: after a restart, a run would \
             prepare and sign a new execution.",
        );
    }
    text
}

/// Any 5xx, 408 or 429 answer to a submit may hide a submit the API took,
/// whether a gateway answered it or the API did, with a `code` or without.
/// One read settles it: taken, the execution is reported submitted;
/// otherwise the error keeps the execution id, says how the read failed to
/// settle it, and gives the safe ways on, never a plain refusal. Only a
/// body that says `submissionState: "not_submitted"` is a refusal.
#[tokio::test]
async fn a_gateway_answer_to_a_submit_is_an_unknown_outcome() {
    let (origin, state) = mock_api();
    let key = SigningKey::from_bytes(&[7; 32]);
    let wallet = b58(key.verifying_key().as_bytes());
    let (key, wallet) = (&key, wallet.as_str());
    let prepare_signed = |agent: ProtocolAgent| async move {
        let prepared = agent
            .prepare(&hosted(Some("d-1")), &tip_call(&json!({})), wallet)
            .await
            .unwrap();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(
                prepared["transactions"][0]["transactionBase64"]
                    .as_str()
                    .unwrap(),
            )
            .unwrap();
        let signed = b64(&crate::dev::sign_transaction(&bytes, key).unwrap());
        (agent, signed)
    };
    let answers = [
        (502, json!("<html>upstream timed out</html>")),
        (504, json!("<html>upstream timed out</html>")),
        (408, json!("<html>upstream timed out</html>")),
        (429, json!("<html>upstream timed out</html>")),
        // The transport error the transactions SDKs model for a submit
        // whose outcome the API itself does not know.
        (
            504,
            json!({ "code": "upstream_timeout", "message": "Submission outcome is unknown",
                "retryable": false, "submissionState": "unknown" }),
        ),
        (
            500,
            json!({ "error": "internal", "code": "internal-error" }),
        ),
        (
            503,
            json!({ "error": "submits are paused", "code": "protocol-submit-paused" }),
        ),
    ];
    for (status, body) in answers.clone() {
        restart(&state);
        {
            let mut guard = state.lock().unwrap();
            guard.submit_answer = Some((status, body.clone()));
            guard.intent_answer = Some((200, json!({ "executionId": "e-1", "state": "prepared" })));
        }
        let (agent, signed) =
            prepare_signed(ProtocolAgent::new(None).with_api(&origin, "a4_sk_test")).await;
        let error = agent
            .submit(&TargetArgs::default(), "e-1", &[signed], None, None)
            .await
            .unwrap_err();
        let unknown = error.downcast_ref::<MaybeSubmitted>().expect("unknown");
        assert_eq!(unknown.execution_id, "e-1");
        assert_eq!(unknown.expires_at, Some(json!(1_900_000_000)));
        assert_eq!(unknown.signed, None);
        let message = error.to_string();
        assert!(
            message.starts_with(&format!("The submit was answered {status} ")),
            "{message}"
        );
        assert!(
            message.ends_with(&format!(
                "; a read then showed it still `prepared`. {}",
                unknown_guidance("e-1", "1900000000", false)
            )),
            "{message}"
        );
        assert!(!message.contains("a4_sk_test"), "{message}");
        assert!(!message.contains("session-token"), "{message}");
        assert_eq!(sent(&state, "GET", "/execution-intents/e-1").len(), 1);
        assert_eq!(sent(&state, "POST", "/submit").len(), 1);
    }

    // The API took them behind the error, whatever the body: reported
    // submitted.
    for (status, body) in answers {
        restart(&state);
        {
            let mut guard = state.lock().unwrap();
            guard.submit_answer = Some((status, body));
            guard.intent_answer = Some(taken("submitting"));
        }
        let (agent, signed) =
            prepare_signed(ProtocolAgent::new(None).with_api(&origin, "a4_sk_test")).await;
        let result = agent
            .submit(&TargetArgs::default(), "e-1", &[signed], Some(false), None)
            .await
            .unwrap();
        assert_eq!(result["executionId"], "e-1", "{result}");
        assert_eq!(result["state"], "submitting", "{result}");
        assert_eq!(sent(&state, "POST", "/submit").len(), 1);
    }

    // A read that names no state (a proxy's page) shows nothing either way.
    restart(&state);
    {
        let mut guard = state.lock().unwrap();
        guard.submit_answer = Some((504, Value::Null));
        guard.intent_answer = Some((200, json!("<html>ok</html>")));
    }
    let (agent, signed) =
        prepare_signed(ProtocolAgent::new(None).with_api(&origin, "a4_sk_test")).await;
    let error = agent
        .submit(&TargetArgs::default(), "e-1", &[signed], Some(false), None)
        .await
        .unwrap_err();
    assert!(error.downcast_ref::<MaybeSubmitted>().is_some(), "{error}");
    assert!(error.to_string().contains("returned no state"), "{error}");

    // The API says it did not submit them: a refusal, after the read.
    for spelling in ["submissionState", "submission_state"] {
        restart(&state);
        {
            let mut guard = state.lock().unwrap();
            guard.submit_answer = Some((
                503,
                json!({ "error": "submits are paused", "code": "protocol-submit-paused",
                    spelling: "not_submitted" }),
            ));
            guard.intent_answer = Some((200, json!({ "executionId": "e-1", "state": "prepared" })));
        }
        let (agent, signed) =
            prepare_signed(ProtocolAgent::new(None).with_api(&origin, "a4_sk_test")).await;
        let error = agent
            .submit(&TargetArgs::default(), "e-1", &[signed], None, None)
            .await
            .unwrap_err();
        assert!(error.downcast_ref::<MaybeSubmitted>().is_none(), "{error}");
        assert!(
            error
                .to_string()
                .starts_with("The submit was refused with 503 Service Unavailable, not submitted"),
            "{error}"
        );
        assert_eq!(sent(&state, "GET", "/execution-intents/e-1").len(), 1);
    }

    // protocol_execute: a coded 500 after the API took the transactions
    // completes the step; a coded unknown outcome is remembered, and
    // running the step again prepares nothing new.
    let dir = tempfile::tempdir().unwrap();
    let call_args = json!({});
    let call = tip_call(&call_args);
    let target = hosted(Some("d-1"));
    restart(&state);
    {
        let mut guard = state.lock().unwrap();
        guard.submit_answer = Some((
            500,
            json!({ "error": "internal", "code": "internal-error" }),
        ));
        guard.intent_answer = Some(taken("completed"));
    }
    let done = signing_agent(dir.path(), &origin)
        .execute(&target, &call, None, None)
        .await
        .unwrap();
    let agent = signing_agent(dir.path(), &origin);
    assert_eq!(done["state"], "completed", "{done}");
    assert_eq!(sent(&state, "GET", "/execution-intents/e-1").len(), 1);
    restart(&state);
    {
        let mut guard = state.lock().unwrap();
        guard.submit_answer = Some((
            504,
            json!({ "code": "upstream_timeout", "submissionState": "unknown" }),
        ));
        guard.intent_answer = Some((200, json!({ "executionId": "e-1", "state": "prepared" })));
    }
    let error = agent.execute(&target, &call, None, None).await.unwrap_err();
    let unknown = error.downcast_ref::<MaybeSubmitted>().expect("unknown");
    let submitted: Vec<String> = sent(&state, "POST", "/submit")[0].body["signedTransactions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|signed| signed.as_str().unwrap().to_string())
        .collect();
    assert_eq!(unknown.signed.as_ref(), Some(&submitted));
    assert!(
        error
            .to_string()
            .ends_with(&unknown_guidance("e-1", "1900000000", true)),
        "{error}"
    );
    // The API had taken them: running the step again reports that.
    state.lock().unwrap().intent_answer = Some(taken("completed"));
    let again = agent.execute(&target, &call, None, None).await.unwrap();
    assert_eq!(again["state"], "completed", "{again}");
    assert_eq!(sent(&state, "POST", "/prepare").len(), 1);
    assert_eq!(sent(&state, "POST", "/submit").len(), 1);
}

/// `protocol_execute` on the mock, failing the test rather than hanging.
async fn exec(
    agent: &ProtocolAgent,
    call: &StepCall<'_>,
    timeout: Option<Duration>,
) -> Result<Value> {
    crate::deadline::watchdog(
        Duration::from_secs(10),
        agent.execute(&hosted(Some("d-1")), call, None, timeout),
    )
    .await
}

/// The time (Unix seconds) a held run's `note` names: "Until <time> (this
/// server's clock)".
fn held_until(note: &Value) -> i64 {
    let note = note.as_str().expect("a note");
    let at = note
        .split_once("Until ")
        .and_then(|(_, rest)| rest.split_once(" (this server's clock)"))
        .unwrap_or_else(|| panic!("no hold in {note}"))
        .0;
    chrono::DateTime::parse_from_rfc3339(at)
        .unwrap()
        .timestamp()
}

/// `at` is `from_now` seconds from now, give or take 5 s.
fn assert_about(at: i64, from_now: i64, expires_at: &Value) {
    let expected = crate::dev::unix_now() as i64 + from_now;
    assert!(
        (at - expected).abs() <= 5,
        "held until {at}, not about {expected}, for expiresAt {expires_at}"
    );
}

/// However missing, unreadable, past or absurd its `expiresAt`, a completed
/// run is held for at least the minimum and at most the maximum after it
/// completed; within that, until its `expiresAt` (seconds, milliseconds or
/// RFC 3339).
#[test]
fn a_completed_run_is_held_between_the_minimum_and_the_maximum() {
    let done = 1_900_000_000_i64;
    let (floor, ceiling) = (done + 600, done + 3600);
    let in_between = done + 1800;
    let cases = [
        (None, floor),
        (Some(Value::Null), floor),
        (Some(json!({})), floor),
        (Some(json!([in_between])), floor),
        (Some(json!(true)), floor),
        (Some(json!("9999-12-31T23:59:59Z")), ceiling),
        (Some(json!(i64::MAX)), ceiling),
        (Some(json!(i64::MIN)), floor),
        (Some(json!(u64::MAX)), floor),
        (Some(json!(in_between as f64)), floor),
        (Some(json!(f64::NAN)), floor),
        (Some(json!("NaN")), floor),
        (Some(json!("9".repeat(1 << 20))), floor),
        (Some(json!(-5)), floor),
        (Some(json!(done - 30)), floor),
        // Ahead, but sooner than the minimum, or later than the maximum.
        (Some(json!(done + 1)), floor),
        (Some(json!(done + 3601)), ceiling),
        (Some(json!(in_between)), in_between),
        (Some(json!(in_between * 1000)), in_between),
        (Some(json!(shown_time(in_between))), in_between),
        (Some(json!("2030-03-17T19:16:40+01:00")), in_between),
        // The largest number read as seconds, and the smallest read as
        // milliseconds.
        (Some(json!(100_000_000_000_i64)), ceiling),
        (Some(json!(100_000_000_001_i64)), floor),
    ];
    for (expires_at, held) in cases {
        let shown: String = format!("{expires_at:?}").chars().take(60).collect();
        assert_eq!(
            hold_until(expires_at.as_ref(), done, (600, 3600)),
            held,
            "{shown}"
        );
    }
    assert_eq!(hold_until(None, i64::MAX - 10, (600, 3600)), i64::MAX);
}

/// A run with other args that fails before signing (here the policy
/// refuses it) leaves the step's completed run held; one that signs and
/// completes takes its place.
#[tokio::test]
async fn a_refused_run_with_other_args_keeps_the_completed_one() {
    let (origin, state) = mock_api();
    let dir = tempfile::tempdir().unwrap();
    let agent = signing_agent(dir.path(), &origin);
    let args = json!({});
    let call = tip_call(&args);
    let other_args = json!({ "amount": "2" });
    let other = tip_call(&other_args);
    let held = |answer: &Value| {
        answer["note"].as_str().is_some_and(|note| {
            note.starts_with("This answers the last protocol_execute of step 'tip' with these args")
        })
    };

    exec(&agent, &call, None).await.unwrap();
    state.lock().unwrap().sol_out = 20_000_000;
    let refused = exec(&agent, &other, None).await.unwrap();
    assert_eq!(refused["state"], "refused", "{refused}");
    state.lock().unwrap().sol_out = 0;
    let again = exec(&agent, &call, None).await.unwrap();
    assert!(held(&again), "{again}");
    assert_eq!(sent(&state, "POST", "/prepare").len(), 2);
    assert_eq!(sent(&state, "POST", "/submit").len(), 1);

    let fresh = exec(&agent, &other, None).await.unwrap();
    assert_eq!(fresh["state"], "completed", "{fresh}");
    assert!(!held(&fresh), "{fresh}");
    let anew = exec(&agent, &call, None).await.unwrap();
    assert!(!held(&anew), "{anew}");
    assert_eq!(sent(&state, "POST", "/prepare").len(), 4);
    assert_eq!(sent(&state, "POST", "/submit").len(), 3);
}

/// A run with other args that signs ends the step's held answer, even
/// when the API then refuses its submit (a 400): the same args run inside
/// the hold prepare and send again. The held note says so.
#[tokio::test]
async fn a_run_with_other_args_that_signs_ends_the_held_answer() {
    let (origin, state) = mock_api();
    let dir = tempfile::tempdir().unwrap();
    let agent = signing_agent(dir.path(), &origin);
    let args = json!({});
    let call = tip_call(&args);
    let other_args = json!({ "amount": "2" });
    let other = tip_call(&other_args);

    let done = exec(&agent, &call, None).await.unwrap();
    assert_eq!(done["state"], "completed", "{done}");
    let held = exec(&agent, &call, None).await.unwrap();
    let note = held["note"].as_str().expect("a held note");
    assert!(
        note.contains(
            "unless a run with other args signs first (even one the API then refuses), which \
             ends this answer."
        ),
        "{note}"
    );
    assert_eq!(sent(&state, "POST", "/prepare").len(), 1);
    assert_eq!(sent(&state, "POST", "/submit").len(), 1);

    state.lock().unwrap().submit_answer = Some((
        400,
        json!({ "error": "bad signature", "code": "intent-invalid" }),
    ));
    let error = exec(&agent, &other, None).await.unwrap_err();
    assert!(error.downcast_ref::<MaybeSubmitted>().is_none(), "{error}");
    assert!(error.downcast_ref::<StepBusy>().is_none(), "{error}");
    assert_eq!(sent(&state, "POST", "/prepare").len(), 2);
    assert_eq!(sent(&state, "POST", "/submit").len(), 2);

    // Inside the hold, but the signed run with other args ended it.
    state.lock().unwrap().submit_answer = None;
    let again = exec(&agent, &call, None).await.unwrap();
    assert_eq!(again["state"], "completed", "{again}");
    assert!(again.get("note").is_none(), "{again}");
    assert_eq!(sent(&state, "POST", "/prepare").len(), 3);
    assert_eq!(sent(&state, "POST", "/submit").len(), 3);
}

/// A run answering for an unsettled execution names that execution to a
/// call refused while it waits.
#[tokio::test]
async fn a_resumed_run_names_its_execution_while_it_waits() {
    let (origin, state) = mock_api();
    let dir = tempfile::tempdir().unwrap();
    let mut agent = signing_agent(dir.path(), &origin);
    // The stalled submit is cut off at 500 ms: an unknown outcome.
    agent.http = reqwest::Client::builder()
        .timeout(Duration::from_millis(500))
        .build()
        .unwrap();
    {
        let mut guard = state.lock().unwrap();
        guard.stall = Some(("POST", "/api/execution-intents/e-1/submit"));
        guard.intent_answer = Some((200, json!({ "executionId": "e-1", "state": "prepared" })));
    }
    let args = json!({});
    let call = tip_call(&args);
    let error = exec(&agent, &call, None).await.unwrap_err();
    assert!(error.downcast_ref::<MaybeSubmitted>().is_some(), "{error}");

    // Taken, not settled yet: the next run waits on it.
    {
        let mut guard = state.lock().unwrap();
        guard.stall = None;
        guard.intent_answer = Some(taken("submitting"));
    }
    let reads = sent(&state, "GET", "/execution-intents/e-1").len();
    let (resumed, refused) =
        tokio::join!(exec(&agent, &call, Some(Duration::from_secs(2))), async {
            eventually("the resumed run never read its execution", || {
                sent(&state, "GET", "/execution-intents/e-1").len() > reads
            })
            .await;
            exec(&agent, &call, None).await
        });
    let error = refused.unwrap_err();
    let busy = error.downcast_ref::<StepBusy>().expect("busy");
    assert_eq!(busy.code, "step-in-flight");
    assert_eq!(busy.execution_id.as_deref(), Some("e-1"));
    assert!(error.to_string().contains("as execution e-1;"), "{error}");
    let resumed = resumed.unwrap();
    assert_eq!(resumed["executionId"], "e-1", "{resumed}");
    assert_eq!(resumed["settled"], false, "{resumed}");
    assert_eq!(sent(&state, "POST", "/prepare").len(), 1);
    assert_eq!(sent(&state, "POST", "/submit").len(), 1);
}

/// A running call whose execution id is missing or empty is refused
/// without naming one.
#[tokio::test]
async fn a_running_call_without_an_execution_id_names_none() {
    let (origin, _state) = mock_api();
    let dir = tempfile::tempdir().unwrap();
    let agent = signing_agent(dir.path(), &origin);
    let args = json!({});
    let call = tip_call(&args);
    exec(&agent, &call, None).await.unwrap();
    let key = agent.steps.iter().next().unwrap().key().clone();
    for running in [None, Some(String::new())] {
        agent
            .steps
            .insert(key.clone(), StepRun::InFlight(running.clone()));
        let error = exec(&agent, &call, None).await.unwrap_err();
        let busy = error.downcast_ref::<StepBusy>().expect("busy");
        assert_eq!(busy.code, "step-in-flight");
        assert_eq!(busy.execution_id, None, "{running:?}");
        let message = error.to_string();
        assert!(message.contains("has not finished preparing;"), "{message}");
        assert!(!message.contains("as execution"), "{message}");
    }
}

/// Running `protocol_execute` again before its last execution settled (an
/// unknown submit, or a wait that ended first) answers for that execution
/// instead of preparing and signing a new one: the transactions are sent
/// once, or, while the API has not taken them, the same signed bytes are
/// sent again. Once it completed, the same args get its answer again until
/// its `expiresAt`, but for at least 10 minutes and at most an hour after
/// it completed on this server's clock, whatever the `expiresAt` says,
/// unless a run with other args signs first; other args, or the same args
/// after then, execute again.
#[tokio::test]
async fn running_protocol_execute_again_never_submits_a_step_twice() {
    let (origin, state) = mock_api();
    let dir = tempfile::tempdir().unwrap();
    let agent = signing_agent(dir.path(), &origin);
    let args = json!({});
    let call = tip_call(&args);
    let other_args = json!({ "amount": "2" });
    let other = tip_call(&other_args);

    // A gateway answers the submit, and the read that would settle it
    // fails too.
    {
        let mut guard = state.lock().unwrap();
        guard.submit_answer = Some((504, Value::Null));
        guard.intent_answer = Some((502, Value::Null));
    }
    let error = exec(&agent, &call, None).await.unwrap_err();
    assert!(error.downcast_ref::<MaybeSubmitted>().is_some(), "{error}");
    // Run again while the API is still unreadable: nothing is sent.
    let error = exec(&agent, &call, None).await.unwrap_err();
    assert!(error.downcast_ref::<MaybeSubmitted>().is_some(), "{error}");
    assert_eq!(sent(&state, "POST", "/submit").len(), 1);
    assert_eq!(sent(&state, "POST", "/prepare").len(), 1);

    // The API had taken them: running again reports that execution.
    state.lock().unwrap().intent_answer = Some(taken("completed"));
    let result = exec(&agent, &call, None).await.unwrap();
    assert_eq!(result["executionId"], "e-1", "{result}");
    assert_eq!(result["state"], "completed", "{result}");
    let until = held_until(&result["note"]);
    assert_eq!(
        result["note"],
        format!(
            "This answers the previous protocol_execute of this step, which had not settled: \
             nothing new was prepared or signed. Until {} (this server's clock), running the \
             step again with the same args answers for this execution, unless a run with other \
             args signs first (even one the API then refuses), which ends this answer. Other \
             args, or the same args after then, execute again.",
            shown_time(until)
        ),
        "{result}"
    );
    // An `expiresAt` years ahead holds for the hour at most.
    assert_about(until, 3600, &json!(1_900_000_000));
    assert_eq!(sent(&state, "POST", "/submit").len(), 1);
    assert_eq!(sent(&state, "POST", "/prepare").len(), 1);

    // Completed: the same args get that answer again, and nothing is
    // read, prepared or sent.
    restart(&state);
    let again = exec(&agent, &call, None).await.unwrap();
    assert_eq!(again["executionId"], "e-1", "{again}");
    assert_eq!(again["state"], "completed", "{again}");
    assert_eq!(
        again["note"],
        format!(
            "This answers the last protocol_execute of step 'tip' with these args, which \
             completed: nothing new was prepared or signed. Until {} (this server's clock), \
             running the step with the same args answers this way, unless a run with other \
             args signs first (even one the API then refuses), which ends this answer. Other \
             args, or the same args after then, execute again.",
            shown_time(until)
        ),
        "{again}"
    );
    assert!(seen(&state).is_empty(), "{:?}", seen(&state));
    // Other args are a new execution.
    let fresh = exec(&agent, &other, None).await.unwrap();
    assert!(fresh.get("note").is_none(), "{fresh}");
    assert_eq!(sent(&state, "POST", "/prepare").len(), 1);
    assert_eq!(sent(&state, "POST", "/submit").len(), 1);

    // The wait ends before the execution settles: running again answers
    // for it until it settles, and never says to run the step again.
    restart(&state);
    let agent = signing_agent(dir.path(), &origin);
    {
        let mut guard = state.lock().unwrap();
        guard.submit_answer = Some(taken("submitting"));
        guard.intent_answer = Some(taken("submitting"));
    }
    let short = Some(Duration::from_millis(600));
    let pending = exec(&agent, &call, short).await.unwrap();
    assert_eq!(pending["settled"], false, "{pending}");
    let again = exec(&agent, &call, short).await.unwrap();
    assert_eq!(again["settled"], false, "{again}");
    assert_eq!(again["executionId"], "e-1", "{again}");
    let note = again["note"].as_str().unwrap();
    assert!(note.contains("until it settles"), "{note}");
    assert!(!note.contains("Run the step again"), "{note}");
    state.lock().unwrap().intent_answer = Some(taken("completed"));
    let done = exec(&agent, &call, None).await.unwrap();
    assert_eq!(done["state"], "completed", "{done}");
    assert_eq!(sent(&state, "POST", "/prepare").len(), 1);
    assert_eq!(sent(&state, "POST", "/submit").len(), 1);
    restart(&state);
    exec(&agent, &call, None).await.unwrap();
    assert_eq!(sent(&state, "POST", "/prepare").len(), 0);

    // Still `prepared` after the unknown submit: running again sends the
    // same signed bytes, not a new prepare.
    restart(&state);
    let agent = signing_agent(dir.path(), &origin);
    {
        let mut guard = state.lock().unwrap();
        guard.submit_answer = Some((502, Value::Null));
        guard.intent_answer = Some((200, json!({ "executionId": "e-1", "state": "prepared" })));
    }
    exec(&agent, &call, None).await.unwrap_err();
    // The API answers again; the intent stays `prepared` until it takes
    // the transactions.
    state.lock().unwrap().submit_answer = None;
    let result = exec(&agent, &call, None).await.unwrap();
    assert_eq!(result["state"], "completed", "{result}");
    let submits = sent(&state, "POST", "/submit");
    assert_eq!(submits.len(), 2);
    assert_eq!(submits[0].body, submits[1].body);
    assert_eq!(sent(&state, "POST", "/prepare").len(), 1);

    // Held until `expiresAt`, but for 10 minutes at least and an hour at
    // most: one already past (a clock ahead of the API's), a missing one or
    // one years ahead holds the run all the same.
    let in_half_an_hour = crate::dev::unix_now() as i64 + 1800;
    for (expires_at, held_for) in [
        (json!(1), 600),
        (Value::Null, 600),
        (json!("9999-12-31T23:59:59Z"), 3600),
        (json!(shown_time(in_half_an_hour)), 1800),
    ] {
        restart(&state);
        state.lock().unwrap().expires_at = Some(expires_at.clone());
        let agent = signing_agent(dir.path(), &origin);
        exec(&agent, &call, None).await.unwrap();
        let again = exec(&agent, &call, None).await.unwrap();
        assert_eq!(again["state"], "completed", "{again}");
        assert_eq!(sent(&state, "POST", "/prepare").len(), 1, "{expires_at}");
        assert_eq!(sent(&state, "POST", "/submit").len(), 1, "{expires_at}");
        assert_about(held_until(&again["note"]), held_for, &expires_at);
    }

    // Completed just before its expiresAt: still held once it passed.
    restart(&state);
    state.lock().unwrap().expires_at = Some(json!(crate::dev::unix_now() + 1));
    let agent = signing_agent(dir.path(), &origin);
    exec(&agent, &call, None).await.unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;
    let again = exec(&agent, &call, None).await.unwrap();
    assert!(again["note"]
        .as_str()
        .unwrap()
        .starts_with("This answers the last protocol_execute of step 'tip' with these args"));
    assert_eq!(sent(&state, "POST", "/prepare").len(), 1);
    assert_eq!(sent(&state, "POST", "/submit").len(), 1);

    // First read as completed after its expiresAt passed (the wait ended
    // first, and the run after it found the execution completed): held.
    restart(&state);
    {
        let mut guard = state.lock().unwrap();
        guard.expires_at = Some(json!(crate::dev::unix_now() + 1));
        guard.submit_answer = Some(taken("submitting"));
        guard.intent_answer = Some(taken("submitting"));
    }
    let agent = signing_agent(dir.path(), &origin);
    let pending = exec(&agent, &call, Some(Duration::from_millis(600)))
        .await
        .unwrap();
    assert_eq!(pending["settled"], false, "{pending}");
    tokio::time::sleep(Duration::from_secs(2)).await;
    state.lock().unwrap().intent_answer = Some(taken("completed"));
    let done = exec(&agent, &call, None).await.unwrap();
    assert_eq!(done["state"], "completed", "{done}");
    assert_about(held_until(&done["note"]), 600, &json!("past"));
    let again = exec(&agent, &call, None).await.unwrap();
    assert!(again["note"]
        .as_str()
        .unwrap()
        .starts_with("This answers the last protocol_execute of step 'tip' with these args"));
    assert_eq!(sent(&state, "POST", "/prepare").len(), 1);
    assert_eq!(sent(&state, "POST", "/submit").len(), 1);

    // After its hold (1 s here) the same args execute again, and the run
    // is dropped once another step call runs.
    restart(&state);
    let mut agent = signing_agent(dir.path(), &origin);
    agent.completed_hold = (1, 1);
    exec(&agent, &call, None).await.unwrap();
    assert_eq!(agent.steps.len(), 1);
    tokio::time::sleep(Duration::from_millis(2100)).await;
    let elsewhere = StepCall {
        instance: Some("i-2"),
        ..tip_call(&args)
    };
    exec(&agent, &elsewhere, None).await.unwrap();
    // Only the run just made is held.
    assert_eq!(agent.steps.len(), 1);
    let anew = exec(&agent, &call, None).await.unwrap();
    assert!(anew.get("note").is_none(), "{anew}");
    assert_eq!(sent(&state, "POST", "/prepare").len(), 3);

    // Settled without completing (the chain failed it): nothing was done,
    // so the same args run anew.
    restart(&state);
    {
        let mut guard = state.lock().unwrap();
        guard.submit_answer = Some(taken("submitting"));
        guard.intent_answer = Some(taken("chain_failed"));
    }
    let agent = signing_agent(dir.path(), &origin);
    exec(&agent, &call, None).await.unwrap();
    let again = exec(&agent, &call, None).await.unwrap();
    assert_eq!(again["state"], "chain_failed", "{again}");
    assert!(again.get("note").is_none(), "{again}");
    assert_eq!(sent(&state, "POST", "/prepare").len(), 2);
}

/// Running `protocol_submit` again for an execution whose submit had an
/// unknown outcome reads it first and sends nothing when the API took it.
#[tokio::test]
async fn running_protocol_submit_again_sends_an_execution_once() {
    let (origin, state) = mock_api();
    let agent = ProtocolAgent::new(None).with_api(&origin, "a4_sk_test");
    let key = SigningKey::from_bytes(&[7; 32]);
    let wallet = b58(key.verifying_key().as_bytes());
    let prepared = agent
        .prepare(&hosted(Some("d-1")), &tip_call(&json!({})), &wallet)
        .await
        .unwrap();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(
            prepared["transactions"][0]["transactionBase64"]
                .as_str()
                .unwrap(),
        )
        .unwrap();
    let signed = vec![b64(&crate::dev::sign_transaction(&bytes, &key).unwrap())];
    {
        let mut guard = state.lock().unwrap();
        guard.stall = Some(("POST", "/api/execution-intents/e-1/submit"));
        guard.intent_answer = Some((500, Value::Null));
    }
    let mut short = agent.clone();
    short.http = reqwest::Client::builder()
        .timeout(Duration::from_millis(300))
        .build()
        .unwrap();
    let submit = |agent: &ProtocolAgent| {
        let agent = agent.clone();
        let signed = signed.clone();
        async move {
            crate::deadline::watchdog(
                Duration::from_secs(10),
                agent.submit(&TargetArgs::default(), "e-1", &signed, None, None),
            )
            .await
        }
    };
    let error = submit(&short).await.unwrap_err();
    assert!(error.downcast_ref::<MaybeSubmitted>().is_some(), "{error}");

    state.lock().unwrap().intent_answer = Some(taken("completed"));
    let result = submit(&agent).await.unwrap();
    assert_eq!(result["state"], "completed", "{result}");
    assert_eq!(result["settled"], true, "{result}");
    assert_eq!(sent(&state, "POST", "/submit").len(), 1);
}

/// The safe ways on after an unknown submit hold across a restart of the
/// server (a fresh agent remembers nothing): resending the same signed
/// transactions with protocol_submit, from `signedTransactions` in
/// protocol_execute's error or from the external signer's own, never
/// prepares a second execution, and every submit carries the same bytes.
/// So do duplicate protocol_submit calls in parallel. Waiting until the
/// execution is `expired` is the other way: only then does a fresh run
/// prepare anew.
#[tokio::test]
async fn after_a_restart_the_safe_way_on_sends_no_second_transaction() {
    let (origin, state) = mock_api();
    let dir = tempfile::tempdir().unwrap();
    let call_args = json!({});
    let call = tip_call(&call_args);
    let target = hosted(Some("d-1"));
    let unknown_submit = |state: &Shared| {
        let mut guard = state.lock().unwrap();
        guard.submit_answer = Some((504, Value::Null));
        guard.intent_answer = Some((200, json!({ "executionId": "e-1", "state": "prepared" })));
    };
    let same_bytes = |state: &Shared| {
        let submits = sent(state, "POST", "/submit");
        assert!(submits.iter().all(|seen| seen.body == submits[0].body));
        submits.len()
    };

    // protocol_execute, then a restarted server.
    unknown_submit(&state);
    let error = signing_agent(dir.path(), &origin)
        .execute(&target, &call, None, None)
        .await
        .unwrap_err();
    let unknown = error.downcast_ref::<MaybeSubmitted>().unwrap();
    let signed = unknown.signed.clone().unwrap();
    // The error names the deployment protocol_submit needs after a restart.
    let resend = hosted(unknown.deployment_id.as_deref());
    assert_eq!(resend.deployment.as_deref(), Some("d-1"));
    state.lock().unwrap().submit_answer = None;
    let fresh = signing_agent(dir.path(), &origin);
    let result = fresh
        .submit(&resend, "e-1", &signed, None, None)
        .await
        .unwrap();
    assert_eq!(result["state"], "completed", "{result}");
    assert_eq!(same_bytes(&state), 2);
    assert_eq!(sent(&state, "POST", "/prepare").len(), 1);

    // The external signer: protocol_prepare, sign, protocol_submit.
    restart(&state);
    unknown_submit(&state);
    let key = SigningKey::from_bytes(&[7; 32]);
    let wallet = b58(key.verifying_key().as_bytes());
    let external = ProtocolAgent::new(None).with_api(&origin, "a4_sk_test");
    let prepared = external.prepare(&target, &call, &wallet).await.unwrap();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(
            prepared["transactions"][0]["transactionBase64"]
                .as_str()
                .unwrap(),
        )
        .unwrap();
    let signed = vec![b64(&crate::dev::sign_transaction(&bytes, &key).unwrap())];
    let error = external
        .submit(&target, "e-1", &signed, None, None)
        .await
        .unwrap_err();
    assert!(error.downcast_ref::<MaybeSubmitted>().is_some(), "{error}");
    // Two duplicate calls at once on a restarted server, the API still
    // answering 504: only the same bytes are sent.
    let fresh = ProtocolAgent::new(None).with_api(&origin, "a4_sk_test");
    let (first, second) = tokio::join!(
        fresh.submit(&target, "e-1", &signed, None, None),
        fresh.submit(&target, "e-1", &signed, None, None)
    );
    assert!(first.is_err() && second.is_err());
    assert_eq!(same_bytes(&state), 3);
    state.lock().unwrap().submit_answer = None;
    let result = fresh
        .submit(&target, "e-1", &signed, None, None)
        .await
        .unwrap();
    assert_eq!(result["state"], "completed", "{result}");
    assert_eq!(same_bytes(&state), 4);
    assert_eq!(sent(&state, "POST", "/prepare").len(), 1);

    // Expired: it can no longer land, so a fresh run is a new execution.
    restart(&state);
    state.lock().unwrap().intent_answer = Some(taken("expired"));
    let fresh = signing_agent(dir.path(), &origin);
    assert_eq!(
        fresh.intent(&target, "e-1").await.unwrap()["state"],
        "expired"
    );
    state.lock().unwrap().intent_answer = None;
    fresh.execute(&target, &call, None, None).await.unwrap();
    assert_eq!(sent(&state, "POST", "/prepare").len(), 1);
}

/// protocol_execute reserves its step call before preparing: a second
/// call while the first runs (two at once, or a host that timed out and
/// called again) is refused `step-in-flight` and prepares and signs
/// nothing. While the first one's execution is unsettled, a call with
/// other args is refused `step-unsettled`; the same args answer for it,
/// and once it settles other args run anew.
#[tokio::test]
async fn a_step_in_flight_is_prepared_and_sent_once() {
    let (origin, state) = mock_api();
    let dir = tempfile::tempdir().unwrap();
    let mut agent = signing_agent(dir.path(), &origin);
    // The stalled submit is cut off at 500 ms: an unknown outcome.
    agent.http = reqwest::Client::builder()
        .timeout(Duration::from_millis(500))
        .build()
        .unwrap();
    {
        let mut guard = state.lock().unwrap();
        guard.stall = Some(("POST", "/api/execution-intents/e-1/submit"));
        guard.intent_answer = Some((200, json!({ "executionId": "e-1", "state": "prepared" })));
    }
    let call_args = json!({});
    let call = tip_call(&call_args);
    let target = hosted(Some("d-1"));
    let other_args = json!({ "amount": "2" });
    let other = tip_call(&other_args);
    let run = || {
        crate::deadline::watchdog(
            Duration::from_secs(10),
            agent.execute(&target, &call, None, None),
        )
    };
    let run_other = || {
        crate::deadline::watchdog(
            Duration::from_secs(10),
            agent.execute(&target, &other, None, None),
        )
    };
    let (first, second) = tokio::join!(run(), run());
    let errors = [first.unwrap_err(), second.unwrap_err()];
    let busy: Vec<&StepBusy> = errors
        .iter()
        .filter_map(|error| error.downcast_ref::<StepBusy>())
        .collect();
    assert_eq!(busy.len(), 1, "{errors:?}");
    assert_eq!(busy[0].code, "step-in-flight");
    // Refused before the first call prepared: no execution to name yet.
    assert_eq!(busy[0].execution_id, None);
    assert_eq!(
        busy[0].to_string(),
        "protocol_execute of step 'tip' is already running for this signer in this a4 mcp \
         session and has not finished preparing; nothing was prepared or signed for this \
         call. Its answer goes to the call that started it. If your host gave up on that call, \
         its answer is lost: call again shortly for its `executionId`, and read it with \
         protocol_intent before re-running the step."
    );
    assert!(errors
        .iter()
        .any(|error| error.downcast_ref::<MaybeSubmitted>().is_some()));
    assert_eq!(sent(&state, "POST", "/prepare").len(), 1);
    assert_eq!(sent(&state, "POST", "/submit").len(), 1);

    let error = run_other().await.unwrap_err();
    let busy = error.downcast_ref::<StepBusy>().expect("busy");
    assert_eq!(busy.code, "step-unsettled");
    assert_eq!(busy.execution_id.as_deref(), Some("e-1"));
    assert_eq!(
        error.to_string(),
        "Step 'tip' was last run with other args, and its execution e-1 has not settled; \
         nothing was prepared or signed for this call. Run the step again with its previous \
         args ({}) to answer for that execution; once that answer, or protocol_submit with its \
         signed transactions, shows it settled, the step runs with new args. Restarting a4 mcp \
         clears this guard, so after a restart do not run the step until protocol_intent shows \
         execution e-1 settled or `expired`."
    );
    assert_eq!(sent(&state, "POST", "/prepare").len(), 1);

    {
        let mut guard = state.lock().unwrap();
        guard.stall = None;
        guard.intent_answer = Some(taken("completed"));
    }
    let done = run().await.unwrap();
    assert_eq!(done["executionId"], "e-1", "{done}");
    assert_eq!(sent(&state, "POST", "/prepare").len(), 1);
    assert_eq!(sent(&state, "POST", "/submit").len(), 1);
    restart(&state);
    run_other().await.unwrap();
    assert_eq!(sent(&state, "POST", "/prepare").len(), 1);

    // Unsettled again: resending its signed transactions with
    // protocol_submit, as the error says, completes it, so the same args
    // get that answer and the step runs with other args.
    restart(&state);
    {
        let mut guard = state.lock().unwrap();
        guard.stall = Some(("POST", "/api/execution-intents/e-1/submit"));
        guard.intent_answer = Some((200, json!({ "executionId": "e-1", "state": "prepared" })));
    }
    let error = run().await.unwrap_err();
    let signed = error
        .downcast_ref::<MaybeSubmitted>()
        .and_then(|unknown| unknown.signed.clone())
        .expect("signed");
    state.lock().unwrap().stall = None;
    let resent = agent
        .submit(&target, "e-1", &signed, None, None)
        .await
        .unwrap();
    assert_eq!(resent["settled"], true, "{resent}");
    let again = run().await.unwrap();
    assert_eq!(again["executionId"], "e-1", "{again}");
    assert!(again["note"]
        .as_str()
        .unwrap()
        .starts_with("This answers the last protocol_execute of step 'tip' with these args"));
    assert_eq!(sent(&state, "POST", "/prepare").len(), 1);
    assert_eq!(sent(&state, "POST", "/submit").len(), 2);
    let fresh = run_other().await.unwrap();
    assert!(fresh.get("note").is_none(), "{fresh}");
    assert_eq!(sent(&state, "POST", "/prepare").len(), 2);

    // A submit the API refused took nothing: the step is free at once.
    restart(&state);
    state.lock().unwrap().submit_answer = Some((
        400,
        json!({ "error": "bad signature", "code": "intent-invalid" }),
    ));
    let error = run().await.unwrap_err();
    assert!(error.downcast_ref::<MaybeSubmitted>().is_none(), "{error}");
    state.lock().unwrap().submit_answer = None;
    let fresh = run().await.unwrap();
    assert!(fresh.get("note").is_none(), "{fresh}");
    assert_eq!(sent(&state, "POST", "/prepare").len(), 2);
}

/// A protocol_execute dropped mid-call (a host cancelled it) frees its
/// step while nothing is signed, and leaves it unsettled once signed, so
/// the next run answers for that execution with the same bytes.
#[tokio::test]
async fn a_cancelled_execute_frees_its_step_until_it_signs() {
    let (origin, state) = mock_api();
    let dir = tempfile::tempdir().unwrap();
    let mut agent = signing_agent(dir.path(), &origin);
    // No hold for completed runs, so the next run is a new execution.
    agent.completed_hold = (0, 0);
    let call_args = json!({});
    let call = tip_call(&call_args);
    let target = hosted(Some("d-1"));
    let cancelled = |after| tokio::time::timeout(after, agent.execute(&target, &call, None, None));

    // Dropped while preparing.
    state.lock().unwrap().stall = Some(("POST", "/api/protocol-instances/i-1/steps/tip/prepare"));
    cancelled(Duration::from_millis(300))
        .await
        .expect_err("still running");
    restart(&state);
    let done = agent.execute(&target, &call, None, None).await.unwrap();
    assert!(done.get("note").is_none(), "{done}");
    assert_eq!(sent(&state, "POST", "/prepare").len(), 1);

    // Dropped while sending.
    restart(&state);
    state.lock().unwrap().stall = Some(("POST", "/api/execution-intents/e-1/submit"));
    cancelled(Duration::from_secs(2))
        .await
        .expect_err("still running");
    state.lock().unwrap().stall = None;
    let again = agent.execute(&target, &call, None, None).await.unwrap();
    assert_eq!(again["state"], "completed", "{again}");
    assert!(again["note"].is_string(), "{again}");
    assert_eq!(sent(&state, "POST", "/prepare").len(), 1);
    assert_eq!(sent(&state, "POST", "/submit").len(), 1);
}

/// Wait until `done`, failing the test after 10 s.
async fn eventually(what: &str, done: impl Fn() -> bool) {
    let started = std::time::Instant::now();
    while !done() {
        assert!(started.elapsed() < Duration::from_secs(10), "{what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// A host that cancels protocol_execute mid-wait (`notifications/cancelled`)
/// does not stop it: the call runs on and its answer is dropped. A retry
/// while it runs is refused `step-in-flight` with the execution to read,
/// and a retry after it completed gets that execution's answer: one
/// prepare and one submit in all.
#[tokio::test]
async fn a_host_cancel_mid_wait_sends_the_step_once() {
    let (origin, state) = mock_api();
    let dir = tempfile::tempdir().unwrap();
    let agent = signing_agent(dir.path(), &origin);
    let mut session =
        crate::server::deadline_tests::Session::open(crate::server::AreteMcp::with_agents(
            agent.clone(),
            crate::sandbox::SandboxClient::from_values(None, None, None),
        ))
        .await;
    let execute = |id: u64| {
        json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {
            "name": "protocol_execute",
            "arguments": {"step": "tip", "instance": "i-1", "deployment": "d-1", "args": {}},
        }})
        .to_string()
    };
    {
        let mut guard = state.lock().unwrap();
        guard.submit_answer = Some(taken("submitting"));
        guard.intent_answer = Some(taken("submitting"));
    }
    session.send(&execute(21)).await;
    eventually("the first call never waited on its execution", || {
        !sent(&state, "GET", "/execution-intents/e-1").is_empty()
    })
    .await;
    session
        .send(r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":21,"reason":"host timeout"}}"#)
        .await;

    // Retried while the first call runs on.
    session.send(&execute(22)).await;
    let busy = session.reply(22).await;
    assert_eq!(
        busy["error"]["data"],
        json!({"code": "step-in-flight", "retryable": false, "executionId": "e-1"}),
        "{busy}"
    );
    assert_eq!(
        busy["error"]["message"],
        "protocol_execute of step 'tip' is already running for this signer in this a4 mcp \
         session, as execution e-1; nothing was prepared or signed for this call. Its answer \
         goes to the call that started it. If your host gave up on that call, its answer is \
         lost: read execution e-1 with protocol_intent before re-running the step.",
        "{busy}"
    );

    // The first call completes; its answer goes nowhere.
    state.lock().unwrap().intent_answer = Some(taken("completed"));
    eventually("the first call never completed", || {
        agent
            .steps
            .iter()
            .any(|run| matches!(run.value(), StepRun::Completed { .. }))
    })
    .await;

    // Retried after that: its answer, nothing prepared or sent again.
    session.send(&execute(23)).await;
    let answer = session.reply(23).await;
    assert!(answer.get("error").is_none(), "{answer}");
    let body: Value =
        serde_json::from_str(answer["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(body["executionId"], "e-1", "{body}");
    assert_eq!(body["state"], "completed", "{body}");
    assert!(
        body["note"]
            .as_str()
            .unwrap()
            .starts_with("This answers the last protocol_execute of step 'tip' with these args"),
        "{body}"
    );
    // A reply to 21 written after the one to 23 counts too.
    session.drain(Duration::from_millis(500)).await;
    assert!(
        session
            .skipped
            .iter()
            .all(|message| message["id"] != json!(21)),
        "{:?}",
        session.skipped
    );
    assert_eq!(sent(&state, "POST", "/prepare").len(), 1);
    assert_eq!(sent(&state, "POST", "/submit").len(), 1);
}

/// `http://<address>` of an RPC that answers every request with a 200
/// whose body is not JSON.
fn not_json_rpc() -> String {
    use std::io::Write;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut buffer = [0u8; 8192];
            let _ = stream.read(&mut buffer);
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\ncontent-type: text/html\r\ncontent-length: 11\r\nconnection: close\r\n\r\n<html></ht>",
            );
        }
    });
    origin
}

/// An RPC whose URL carries its provider's key never shows that key, and
/// never holds a call past its bound, whether it refuses the connection,
/// answers something that is not JSON, or stalls: the receipt's
/// `getTransaction`, the proposer's balance, and the simulation of a
/// prepared step, which ends within the agent's simulation budget even
/// when each RPC request has no bound of its own.
#[tokio::test]
async fn rpc_failures_never_show_the_rpc_key_or_hang() {
    let (origin, _state) = mock_api();
    let closed = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        format!("http://{}", listener.local_addr().unwrap())
    };
    let signature = receipt_case("legacy-wire")["receipt"]["signature"]
        .as_str()
        .unwrap()
        .to_string();
    let target = hosted(Some("d-1"));
    let call_args = json!({});
    let call = tip_call(&call_args);
    let short = || {
        reqwest::Client::builder()
            .timeout(Duration::from_millis(300))
            .build()
            .unwrap()
    };
    for base in [closed, not_json_rpc(), crate::deadline::blackhole()] {
        let rpc = format!("{base}/?api-key=rpc-secret");
        let dir = tempfile::tempdir().unwrap();
        let (signer, _) = signer_with_rpc(
            dir.path(),
            &rpc,
            "deployments = [\"d-1\"]\nmaxSolPerTransaction = 1\n",
        );
        let mut agent = ProtocolAgent::new(Some(signer)).with_api(&origin, "a4_sk_test");
        agent.http = short();

        let receipt = crate::deadline::watchdog(Duration::from_secs(10), agent.receipt(&signature))
            .await
            .unwrap();
        assert_eq!(receipt["onChain"]["checked"], false, "{receipt}");
        assert!(receipt["onChain"]["error"].is_string(), "{receipt}");
        assert!(!receipt.to_string().contains("rpc-secret"), "{receipt}");

        let proposer = crate::deadline::watchdog(
            Duration::from_secs(10),
            agent.squads_proposer(&target, Some(&rpc)),
        )
        .await
        .unwrap();
        assert_eq!(proposer["balance"], "unavailable", "{proposer}");
        assert!(!proposer.to_string().contains("rpc-secret"), "{proposer}");

        // No bound on each RPC request: only the budget ends the stall.
        agent.http = reqwest::Client::new();
        agent.simulation_budget = Duration::from_millis(300);
        let started = std::time::Instant::now();
        let refused = crate::deadline::watchdog(
            Duration::from_secs(10),
            agent.execute(&target, &call, None, None),
        )
        .await
        .unwrap();
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "{:?}",
            started.elapsed()
        );
        assert_eq!(refused["state"], "refused", "{refused}");
        assert!(!refused.to_string().contains("rpc-secret"), "{refused}");
    }
}

#[tokio::test]
async fn execute_refuses_authority_changes_and_uncapped_outflows() {
    let (origin, state) = mock_api();
    let dir = tempfile::tempdir().unwrap();
    // No maxTokenAmounts and no programs allow-list.
    let (signer, _) = signer(
        dir.path(),
        &origin,
        "deployments = [\"d-1\"]\nmaxSolPerTransaction = 0.0001\n",
    );
    let pubkey = signer.pubkey();
    let mut agent = ProtocolAgent::new(Some(signer)).with_api(&origin, "a4_sk_test");
    // No hold for completed runs, so each run of the step is a new
    // execution.
    agent.completed_hold = (0, 0);
    let args = json!({});
    let call = tip_call(&args);
    let target = hosted(Some("d-1"));
    let run = || agent.execute(&target, &call, None, None);
    let submits = |state: &Shared| {
        seen(state)
            .iter()
            .filter(|seen| seen.path.ends_with("/submit"))
            .count()
    };

    // Without maxTokenAmounts no token may leave.
    state.lock().unwrap().tokens_out = 100;
    let refused = run().await.unwrap();
    assert_eq!(refused["rule"], "maxTokenAmounts", "{refused}");
    assert!(refused["reason"]
        .as_str()
        .unwrap()
        .contains("sets no maxTokenAmounts"));

    // A token account handed to another owner is a whole-balance outflow,
    // attributed to its owner before the transaction.
    reset(&state);
    state.lock().unwrap().handover = true;
    let refused = run().await.unwrap();
    assert_eq!(refused["rule"], "maxTokenAmounts", "{refused}");
    assert!(refused["reason"]
        .as_str()
        .unwrap()
        .contains("1000000 base units"));

    // A top-level token approval by the signer.
    reset(&state);
    {
        let mut guard = state.lock().unwrap();
        guard.program = Some(
            bs58::decode(TOKEN_PROGRAMS[0])
                .into_vec()
                .unwrap()
                .try_into()
                .unwrap(),
        );
        guard.instruction = Some((vec![1, 1, 0], vec![4, 1, 0, 0, 0, 0, 0, 0, 0]));
    }
    let refused = run().await.unwrap();
    assert_eq!(refused["rule"], "authority", "{refused}");
    assert!(refused["reason"].as_str().unwrap().contains("`approve`"));
    assert_eq!(
        refused["summary"]["transactions"][0]["unsafeInstructions"][0]["instruction"],
        "approve"
    );

    // An inner System assign of the signer, from the simulation.
    reset(&state);
    state.lock().unwrap().inner = Some(json!([{ "index": 0, "instructions": [
        { "programId": "11111111111111111111111111111111", "program": "system",
          "parsed": { "type": "assign", "info": { "account": pubkey, "owner": b58(&[66; 32]) } } }
    ] }]));
    let refused = run().await.unwrap();
    assert_eq!(refused["rule"], "authority", "{refused}");
    assert!(refused["reason"]
        .as_str()
        .unwrap()
        .contains("an inner `assign`"));

    // Closing a token account to someone else counts its lamports as SOL out.
    reset(&state);
    {
        let mut guard = state.lock().unwrap();
        guard.program = Some(
            bs58::decode(TOKEN_PROGRAMS[0])
                .into_vec()
                .unwrap()
                .try_into()
                .unwrap(),
        );
        guard.instruction = Some((vec![1, 1, 0], vec![9]));
    }
    let refused = run().await.unwrap();
    assert_eq!(refused["rule"], "maxSolPerTransaction", "{refused}");
    assert!(refused["reason"]
        .as_str()
        .unwrap()
        .contains("closed token accounts"));

    // Wrapped SOL counts against the SOL cap, not as an uncapped token.
    reset(&state);
    {
        let mut guard = state.lock().unwrap();
        guard.mint = Some(
            bs58::decode(crate::authority::NATIVE_MINTS[0])
                .into_vec()
                .unwrap()
                .try_into()
                .unwrap(),
        );
        guard.tokens_out = 200_000;
    }
    let refused = run().await.unwrap();
    assert_eq!(refused["rule"], "maxSolPerTransaction", "{refused}");
    state.lock().unwrap().tokens_out = 50_000;
    let before = submits(&state);
    let done = run().await.unwrap();
    assert_eq!(done["state"], "completed", "{done}");
    assert_eq!(submits(&state), before + 1);

    // More written accounts than the simulation reports on: unknown.
    reset(&state);
    state.lock().unwrap().extra_writable = 70;
    let refused = run().await.unwrap();
    assert_eq!(refused["rule"], "maxSolPerTransaction", "{refused}");
    assert!(refused["reason"]
        .as_str()
        .unwrap()
        .contains("more accounts"));
    assert_eq!(
        refused["summary"]["transactions"][0]["signerEffect"]["omittedAccounts"],
        8
    );
}

#[tokio::test]
async fn execute_only_uses_dev_projects_under_the_working_directory() {
    let (origin, _state) = mock_api();
    let dir = tempfile::tempdir().unwrap();
    let (signer, _) = signer(
        dir.path(),
        &origin,
        "deployments = [\"d-1\"]\nmaxSolPerTransaction = 1\n",
    );
    let agent = ProtocolAgent::new(Some(signer));
    let args = json!({});
    let error = agent
        .execute(
            &TargetArgs {
                project: Some(dir.path().display().to_string()),
                ..TargetArgs::default()
            },
            &tip_call(&args),
            None,
            None,
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("working directory"), "{error:#}");
}

#[test]
fn execute_simulates_with_the_signers_rpc() {
    let dir = tempfile::tempdir().unwrap();
    let env = "denv_01hzx3k8m9q2r4s5t6v7w8x9yz";
    let mut session = crate::dev::DevSession::offline(&crate::dev_env::EnvSelection::new(
        env,
        "app",
        "http://127.0.0.1:1",
    ));
    session.rpc_url = "http://127.0.0.1:3".into();
    session.rpc_ws_url = "ws://127.0.0.1:3".into();
    let env_rpc = crate::dev_env::EnvRpc::new(
        crate::dev_env::EnvApi::new(reqwest::Client::new(), "http://127.0.0.1:1", "a4_sk_local")
            .unwrap(),
        env,
    )
    .unwrap();
    let dev_target = || Target {
        api: ProtocolApi::new(reqwest::Client::new(), "http://127.0.0.1:1/api", "", ""),
        cluster: "dev",
        rpc_url: Some("http://127.0.0.1:3".into()),
        dev: Some(DevClient::new(
            DevProject::new(dir.path()),
            session.clone(),
            env_rpc.clone(),
        )),
    };
    let hosted_target = Target {
        api: ProtocolApi::new(reqwest::Client::new(), "https://api.arete.run/api", "", ""),
        cluster: "mainnet",
        rpc_url: Some("https://env-rpc.example".into()),
        dev: None,
    };
    // Writes the keypair file the configs below name.
    signer(
        dir.path(),
        "http://127.0.0.1:9",
        "deployments = [\"d\"]\nmaxSolPerTransaction = 1\n",
    );
    let keypair = dir.path().join("agent.json");
    let config = |rpc: &str, policy: &str| {
        LocalSigner::from_toml(&format!(
            "[signer]\nkeypair = {keypair:?}\n{rpc}[signer.policy]\ndeployments = [\"d\"]\nmaxSolPerTransaction = 1\n{policy}"
        ))
        .unwrap()
        .unwrap()
    };
    let with_rpc = |policy: &str| config("rpcUrl = \"https://own-rpc.example\"\n", policy);
    // The signer's own RPC wins, on Arete and in a dev session...
    let own = with_rpc("");
    assert_eq!(
        simulation_rpc(&own, &hosted_target).as_deref(),
        Some("https://own-rpc.example")
    );
    assert_eq!(
        simulation_rpc(&own, &dev_target()).as_deref(),
        Some("https://own-rpc.example")
    );
    // ...unless the policy lists dev explicitly (or localnet, its earlier name).
    for listed in ["dev", "localnet"] {
        let local = with_rpc(&format!("clusters = [\"{listed}\"]\n"));
        assert_eq!(
            simulation_rpc(&local, &dev_target()).as_deref(),
            Some("http://127.0.0.1:3")
        );
    }
    // Without an RPC of its own, the dev session's loopback RPC.
    let bare = config("", "");
    assert_eq!(
        simulation_rpc(&bare, &dev_target()).as_deref(),
        Some("http://127.0.0.1:3")
    );
    assert_eq!(
        simulation_rpc(&bare, &hosted_target).as_deref(),
        Some("https://env-rpc.example")
    );
}

#[test]
fn a_dev_target_signs_only_for_its_own_deployment() {
    let dir = tempfile::tempdir().unwrap();
    let env = "denv_01hzx3k8m9q2r4s5t6v7w8x9yz";
    let mut session = crate::dev::DevSession::offline(&crate::dev_env::EnvSelection::new(
        env,
        "app",
        "http://127.0.0.1:1",
    ));
    let env_rpc = crate::dev_env::EnvRpc::new(
        crate::dev_env::EnvApi::new(reqwest::Client::new(), "http://127.0.0.1:1", "a4_sk_local")
            .unwrap(),
        env,
    )
    .unwrap();
    let dev_target = |session: &crate::dev::DevSession| Target {
        api: ProtocolApi::new(reqwest::Client::new(), "http://127.0.0.1:1/api", "", ""),
        cluster: "dev",
        rpc_url: None,
        dev: Some(DevClient::new(
            DevProject::new(dir.path()),
            session.clone(),
            env_rpc.clone(),
        )),
    };
    // Without a deployment in the environment nothing binds to dev.
    session.deployment_id = None;
    let refusal = dev_deployment_binding(&dev_target(&session), "d-mainnet").unwrap_err();
    assert_eq!(refusal.rule, "clusters");
    session.deployment_id = Some("d-dev".into());
    let target = dev_target(&session);
    assert!(dev_deployment_binding(&target, "d-dev").is_ok());
    // An explicit deployment id other than the environment's, a mainnet one
    // say, is not labelled dev, even when the policy allows both.
    let refusal = dev_deployment_binding(&target, "d-mainnet").unwrap_err();
    assert_eq!(refusal.rule, "clusters");
    assert!(refusal.reason.contains("d-mainnet"), "{}", refusal.reason);
    // A hosted target is labelled mainnet already.
    let hosted_target = Target {
        api: ProtocolApi::new(reqwest::Client::new(), "https://api.arete.run/api", "", ""),
        cluster: "mainnet",
        rpc_url: None,
        dev: None,
    };
    assert!(dev_deployment_binding(&hosted_target, "d-mainnet").is_ok());
}

#[tokio::test]
async fn execute_checks_programs_invoked_by_cpi() {
    let (origin, state) = mock_api();
    let dir = tempfile::tempdir().unwrap();
    let (signer, _) = signer(
        dir.path(),
        &origin,
        &format!(
            "deployments = [\"d-1\"]\nmaxSolPerTransaction = 1\nprograms = [\"{}\"]\n",
            b58(&PROGRAM)
        ),
    );
    let agent = ProtocolAgent::new(Some(signer)).with_api(&origin, "a4_sk_test");
    let args = json!({});
    let call = tip_call(&args);
    let target = hosted(Some("d-1"));
    let submits = |state: &Shared| {
        seen(state)
            .iter()
            .filter(|seen| seen.path.ends_with("/submit"))
            .count()
    };
    let before = submits(&state);

    // The listed program calls an unlisted one.
    state.lock().unwrap().inner = Some(json!([{ "index": 0, "instructions": [
        { "programId": b58(&[66; 32]), "accounts": [], "data": "" }
    ] }]));
    let refused = agent.execute(&target, &call, None, None).await.unwrap();
    assert_eq!(refused["rule"], "programs", "{refused}");
    assert!(refused["reason"].as_str().unwrap().contains("by CPI"));
    assert_eq!(
        refused["summary"]["transactions"][0]["innerPrograms"],
        json!([b58(&[66; 32])])
    );

    // A CPI whose program index does not resolve fails closed.
    reset(&state);
    state.lock().unwrap().inner = Some(json!([{ "index": 0, "instructions": [
        { "programIdIndex": 99, "accounts": [], "data": "" }
    ] }]));
    let refused = agent.execute(&target, &call, None, None).await.unwrap();
    assert_eq!(refused["rule"], "programs", "{refused}");
    assert!(refused["reason"].as_str().unwrap().contains("resolved"));
    assert_eq!(submits(&state), before, "a refused step was submitted");

    // Runtime programs by CPI need no listing.
    reset(&state);
    state.lock().unwrap().inner = Some(json!([{ "index": 0, "instructions": [
        { "programId": TOKEN_PROGRAMS[0], "accounts": [], "data": "" }
    ] }]));
    let done = agent.execute(&target, &call, None, None).await.unwrap();
    assert_eq!(done["state"], "completed", "{done}");
}

#[tokio::test]
async fn receipts_are_fetched_and_verified_offline_and_on_chain() {
    let (origin, state) = mock_api();
    let signature = receipt_case("legacy-wire")["receipt"]["signature"]
        .as_str()
        .unwrap()
        .to_string();

    // Without an RPC the receipt's own signature is still checked.
    let agent = ProtocolAgent::new(None).with_api(&origin, "a4_sk_test");
    let result = agent.receipt(&signature).await.unwrap();
    assert_eq!(result["found"], true);
    assert_eq!(
        result["signatureCheck"],
        json!({ "valid": true, "keyId": "test-receipt-1" })
    );
    assert_eq!(result["onChain"]["checked"], false);

    // With the signer's RPC the transaction is fetched and checked too.
    let dir = tempfile::tempdir().unwrap();
    let (signer, _) = signer(
        dir.path(),
        &origin,
        "deployments = [\"d-1\"]\nmaxSolPerTransaction = 1\n",
    );
    let agent = ProtocolAgent::new(Some(signer)).with_api(&origin, "a4_sk_test");
    let result = agent.receipt(&signature).await.unwrap();
    assert_eq!(result["onChain"], json!({ "checked": true, "valid": true }));
    let fetched = seen(&state)
        .into_iter()
        .find(|seen| seen.body["method"] == "getTransaction")
        .unwrap();
    assert_eq!(fetched.body["params"][0], json!(signature));
    assert_eq!(fetched.body["params"][1]["encoding"], "base64");

    let missing = agent.receipt("missing").await.unwrap();
    assert_eq!(missing["found"], false);
    assert_eq!(missing["code"], "protocol-receipt-not-found");
    assert!(agent.receipt(" ").await.is_err());
}

#[tokio::test]
async fn squads_onboarding_reads_the_proposer_and_status_and_builds_nothing() {
    let (origin, state) = mock_api();
    let agent = ProtocolAgent::new(None).with_api(&origin, "a4_sk_test");
    let target = hosted(Some("d-1"));
    let rpc = format!("{origin}/rpc");

    let proposer = agent.squads_proposer(&target, Some(&rpc)).await.unwrap();
    assert_eq!(proposer["pubkey"], b58(&[61; 32]));
    assert_eq!(proposer["permissions"], "initiate");
    assert_eq!(proposer["deploymentId"], "d-1");
    // The mock RPC answers 5 lamports.
    assert_eq!(proposer["proposerLamports"], 5);
    assert_eq!(proposer["balance"], "low");
    assert!(proposer["next"].as_str().unwrap().contains("mask 1"));
    // Without an RPC the balance is unavailable, never zero.
    let without = agent.squads_proposer(&target, None).await.unwrap();
    assert_eq!(without["proposerLamports"], Value::Null);
    assert_eq!(without["balance"], "unavailable");
    // An RPC that cannot be reached: the note never names its URL's key.
    let closed = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        format!(
            "http://{}/?api-key=rpc-secret",
            listener.local_addr().unwrap()
        )
    };
    let unreachable = agent.squads_proposer(&target, Some(&closed)).await.unwrap();
    assert_eq!(unreachable["balance"], "unavailable");
    let note = unreachable["balanceNote"].as_str().unwrap();
    assert!(note.contains("The RPC is unreachable"), "{note}");
    assert!(
        !unreachable.to_string().contains("rpc-secret"),
        "{unreachable}"
    );

    let ready = agent
        .squads_status(&target, &b58(&[62; 32]), Some(&rpc))
        .await
        .unwrap();
    assert_eq!(ready["ready"], true);
    assert!(ready["next"]
        .as_str()
        .unwrap()
        .starts_with("The proposer can propose"));
    let missing = agent
        .squads_status(&target, &b58(&[66; 32]), None)
        .await
        .unwrap();
    assert_eq!(missing["problem"], "not_member");
    let next = missing["next"].as_str().unwrap();
    assert!(
        next.contains(&format!(
            "https://app.arete.run/dashboard/squads/{}",
            b58(&[66; 32])
        )),
        "{next}"
    );
    assert!(next.contains("squads-v4 program SDK"), "{next}");
    assert!(!next.contains("protocol_squads_add_proposer"), "{next}");
    let error = agent
        .squads_status(&target, "not-a-key", None)
        .await
        .unwrap_err();
    assert!(error.downcast_ref::<InvalidArgs>().is_some(), "{error}");

    // The routes are read with a read-only protocol session.
    let mint = seen(&state)
        .into_iter()
        .find(|seen| seen.path == "/ws/sessions")
        .unwrap();
    assert_eq!(mint.body["scopes"], json!(["protocol:read"]));
    assert_eq!(mint.body["targetId"], "d-1");

    assert!(seen(&state)
        .iter()
        .all(|seen| !seen.path.ends_with("/add-proposer")));
}

#[test]
fn squads_refusals_carry_what_to_do() {
    for code in ["squads_multisig_missing", "protocol_squads_not_used"] {
        assert!(
            crate::protocol_api::squads_refusal_hint(code).is_some(),
            "{code}"
        );
    }
    let refusal = crate::protocol_api::SquadsRefused {
        what: "Reading the Squads proposer status".into(),
        status: 409,
        code: "protocol-squads-not-used".into(),
        body: json!({ "error": "no squads step" }),
    };
    assert_eq!(refusal.normalized_code(), "protocol_squads_not_used");
    assert!(
        refusal.to_string().contains("no step whose authority"),
        "{refusal}"
    );
    let other = crate::protocol_api::SquadsRefused {
        what: "Reading the Squads proposer".into(),
        status: 500,
        code: String::new(),
        body: json!({ "error": "boom" }),
    };
    assert_eq!(
        other.to_string(),
        "Reading the Squads proposer failed (500): boom"
    );
    assert!(squads_status_next(
        &json!({ "ready": false, "problem": "permissions_invalid", "mask": 7 }),
        false
    )
    .contains("mask 7"));
    let dev = squads_status_next(
        &json!({ "ready": false, "problem": "not_member", "multisig": "Ms1", "proposer": "Prop1" }),
        true,
    );
    assert!(
        dev.contains("`a4 dev squads add-proposer --multisig Ms1 --proposer Prop1 --wallet <a dev wallet that can initiate>`"),
        "{dev}"
    );
}

#[test]
fn the_squads_tools_take_a_target_and_a_multisig() {
    let tools = crate::AreteMcp::with_protocol_agent(ProtocolAgent::new(None)).tools();
    let schema = |name: &str| -> Value {
        let tool = tools
            .iter()
            .find(|tool| tool.name == name)
            .unwrap_or_else(|| panic!("{name} is not listed"));
        Value::Object((*tool.input_schema).clone())
    };
    let required = |name: &str| -> Vec<String> {
        let mut required: Vec<String> = schema(name)["required"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect();
        required.sort();
        required
    };
    assert!(required("protocol_squads_proposer").is_empty());
    assert_eq!(required("protocol_squads_status"), ["multisig"]);
    for name in ["protocol_squads_proposer", "protocol_squads_status"] {
        let properties = &schema(name)["properties"];
        for target in ["deployment", "protocol", "dev", "project"] {
            assert!(properties.get(target).is_some(), "{name} has no {target}");
        }
    }
    // Arete builds no Squads transactions.
    assert!(tools
        .iter()
        .all(|tool| tool.name != "protocol_squads_add_proposer"));
}

#[tokio::test]
async fn account_squads_tools_read_and_attach_and_build_no_transactions() {
    let (origin, state) = mock_api();
    let agent = ProtocolAgent::new(None).with_api(&origin, "a4_sk_test");
    let multisig = b58(&[62; 32]);

    let status = agent.account_squads_status(None).await.unwrap();
    assert_eq!(status["setup"], "needsAttention");
    assert!(status.get("notifications").is_none());
    let next = status["next"].as_str().unwrap();
    assert!(next.contains(&format!(
        "https://app.arete.run/dashboard/squads/{multisig}"
    )));
    assert!(next.contains("Members → Add member"), "{next}");
    assert!(next.contains("squads-v4 program SDK"), "{next}");

    let detail = agent.account_squads_status(Some(&multisig)).await.unwrap();
    assert_eq!(detail["proposer"], b58(&[61; 32]));
    assert!(detail["squadsAppSteps"].as_array().unwrap().len() >= 4);

    let attached = agent
        .account_squads_attach(&multisig, Some(" Treasury "))
        .await
        .unwrap();
    assert_eq!(attached["state"], "pending");
    assert_eq!(attached["proposer"], b58(&[61; 32]));
    assert!(attached["squadsAppSteps"].as_array().unwrap().len() >= 4);

    let error = agent
        .account_squads_attach("not-a-key", None)
        .await
        .unwrap_err();
    assert!(error.downcast_ref::<InvalidArgs>().is_some(), "{error}");

    let missing = agent
        .account_squads_status(Some(&b58(&[70; 32])))
        .await
        .unwrap_err();
    let refused = missing
        .downcast_ref::<crate::protocol_api::SquadsRefused>()
        .unwrap();
    assert_eq!(
        refused.normalized_code(),
        "protocol_squads_multisig_not_found"
    );

    // The key itself is the bearer: no protocol session is minted, and only
    // the read and attach routes are called.
    let seen = seen(&state);
    assert!(seen.iter().all(|seen| seen.path != "/ws/sessions"));
    let attach = seen
        .iter()
        .find(|seen| seen.method == "POST" && seen.path == "/api/account/squads/multisigs")
        .unwrap();
    assert_eq!(
        attach.body,
        json!({ "address": multisig, "label": "Treasury" })
    );
    assert!(seen
        .iter()
        .filter(|seen| seen.path.starts_with("/api/account/squads"))
        .all(|seen| seen.method == "GET" || seen.path == "/api/account/squads/multisigs"));
}

#[test]
fn setup_required_refusals_print_the_dashboard_link() {
    let refused = crate::protocol_api::SquadsRefused {
        what: "Creating the deployment".into(),
        status: 409,
        code: "protocol-squads-setup-required".into(),
        body: json!({
            "code": "protocol-squads-setup-required",
            "message": "Step 'finalize' proposes to Squads multisig Ms1, which is not set up for your account (not_member).",
            "details": {
                "step": "finalize", "multisig": "Ms1", "state": "pending", "problem": "not_member",
                "proposer": "Prop1", "setupUrl": "https://app.arete.run/dashboard/squads/Ms1",
                "commands": { "status": "a4 squads status Ms1" }
            }
        }),
    };
    let text = refused.to_string();
    assert!(text.starts_with("Step 'finalize'"), "{text}");
    assert!(text.contains("Set it up: https://app.arete.run/dashboard/squads/Ms1"));
    assert!(text.contains("Initiate only"));
    assert!(text.contains("a4 squads status Ms1"));
}
