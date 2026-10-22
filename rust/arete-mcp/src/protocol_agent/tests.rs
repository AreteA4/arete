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
                "intent": { "executionId": "e-1", "instanceId": instance, "expiresAt": 1_900_000_000,
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
        return (
            200,
            json!({ "executionId": "e-1", "state": "submitting", "transactions": [] }),
        );
    }
    if path_only.starts_with("/api/execution-intents/") {
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
    let key = SigningKey::from_bytes(&[SEED; 32]);
    let mut bytes = vec![SEED; 32];
    bytes.extend_from_slice(key.verifying_key().as_bytes());
    let path = dir.join("agent.json");
    std::fs::write(&path, serde_json::to_string(&bytes).unwrap()).unwrap();
    let text = format!(
        "[signer]\nkeypair = {path:?}\nrpcUrl = \"{origin}/rpc\"\n[signer.policy]\n{policy}"
    );
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
    let agent = ProtocolAgent::new(Some(signer)).with_api(&origin, "a4_sk_test");
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
        "dev_actions",
    ] {
        assert!(listed.iter().any(|name| name == tool), "{tool} missing");
    }
    assert!(!listed.iter().any(|name| name == "protocol_execute"));

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

fn reset(state: &Shared) {
    let mut guard = state.lock().unwrap();
    let seen = std::mem::take(&mut guard.seen);
    *guard = MockState {
        seen,
        ..MockState::default()
    };
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
    let agent = ProtocolAgent::new(Some(signer)).with_api(&origin, "a4_sk_test");
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
