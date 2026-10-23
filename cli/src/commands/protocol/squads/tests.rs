use clap::Parser;
use serde_json::{json, Value};

use super::*;
use crate::api_client::test_support::MockServer;

const SECRET_KEY: &str = "a4_sk_0123456789012345678901234567890123456789";
const DEPLOYMENT_ID: &str = "0b5cb1c2-8c5f-4d0e-8f4d-2c6a7c1f9e10";
const MULTISIG: &str = "9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM";
const PROPOSER: &str = "4Nd1mBQtrMJVYVfKf2PJy9NZUZdTAsp7D4xWLs4gDB4T";

#[derive(Parser, Debug)]
struct Cli {
    #[command(subcommand)]
    command: SquadsCommands,
}

fn parse(args: &[&str]) -> Result<SquadsCommands, clap::Error> {
    let mut argv = vec!["squads"];
    argv.extend_from_slice(args);
    Cli::try_parse_from(argv).map(|cli| cli.command)
}

fn agent(server: &MockServer) -> ProtocolAgent {
    ProtocolAgent::new(None).with_api(server.base_url(), SECRET_KEY)
}

fn hosted() -> SquadsTarget {
    SquadsTarget {
        target: Some(DEPLOYMENT_ID.into()),
        ..SquadsTarget::default()
    }
}

#[test]
fn targets_resolve_like_protocol_actions() {
    let SquadsCommands::Status(args) =
        parse(&["status", "token-raise", "--multisig", MULTISIG]).unwrap()
    else {
        panic!("not status");
    };
    let target = args.target.tool_target().unwrap();
    assert_eq!(target.protocol.as_deref(), Some("token-raise"));
    assert!(!target.dev);
    assert_eq!(args.multisig, MULTISIG);

    let SquadsCommands::Proposer(args) = parse(&["proposer", DEPLOYMENT_ID]).unwrap() else {
        panic!("not proposer");
    };
    assert_eq!(
        args.target.tool_target().unwrap().deployment.as_deref(),
        Some(DEPLOYMENT_ID)
    );
    let SquadsCommands::Proposer(args) = parse(&["proposer", "--dev"]).unwrap() else {
        panic!("not proposer");
    };
    assert!(args.target.tool_target().unwrap().dev);
    assert_eq!(args.target.words(), "--dev");
    let SquadsCommands::Proposer(args) = parse(&["proposer", "--project", "app"]).unwrap() else {
        panic!("not proposer");
    };
    let target = args.target.tool_target().unwrap();
    assert!(target.dev);
    assert_eq!(target.project.as_deref(), Some("app"));
    assert_eq!(args.target.words(), "--project app");
    // Without a target or --dev there is nothing to ask about.
    let SquadsCommands::Proposer(args) = parse(&["proposer"]).unwrap() else {
        panic!("not proposer");
    };
    assert!(args.target.tool_target().is_err());

    // Status and add-proposer need the multisig; add-proposer the member.
    assert!(parse(&["status", "token-raise"]).is_err());
    assert!(parse(&["add-proposer", "token-raise", "--multisig", MULTISIG]).is_err());
    assert!(parse(&[
        "add-proposer",
        "--dev",
        "--multisig",
        MULTISIG,
        "--member",
        "alice",
        "--send",
        "--keypair",
        "k.json"
    ])
    .is_err());
    assert!(parse(&[
        "add-proposer",
        "x",
        "--multisig",
        MULTISIG,
        "--member",
        "m",
        "--transaction-format",
        "v2"
    ])
    .is_err());
}

fn add_args(extra: &[&str]) -> AddProposerArgs {
    let mut argv = vec![
        "add-proposer",
        DEPLOYMENT_ID,
        "--multisig",
        MULTISIG,
        "--member",
        PROPOSER,
    ];
    argv.extend_from_slice(extra);
    let SquadsCommands::AddProposer(args) = parse(&argv).unwrap() else {
        panic!("not add-proposer");
    };
    args
}

#[test]
fn signing_options_are_checked_before_anything_is_fetched() {
    assert!(matches!(signing(&add_args(&[])).unwrap(), Signing::None));
    let error = signing(&add_args(&["--send"])).err().unwrap();
    assert!(error.to_string().contains("needs --dev"), "{error}");
    let error = signing(&add_args(&["--keypair", "k.json"])).err().unwrap();
    assert!(error.to_string().contains("needs --rpc-url"), "{error}");
    let error = signing(&add_args(&["--rpc-url", "http://127.0.0.1:1"]))
        .err()
        .unwrap();
    assert!(error.to_string().contains("pass --keypair"), "{error}");
    let error = signing(&add_args(&["--dev", "--rpc-url", "http://127.0.0.1:1"]))
        .err()
        .unwrap();
    assert!(error.to_string().contains("dev environment"), "{error}");
    assert!(matches!(
        signing(&add_args(&["--dev", "--send"])).unwrap(),
        Signing::DevWallet
    ));

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("member.json");
    let key = SigningKey::from_bytes(&[7; 32]);
    let mut bytes = vec![7u8; 32];
    bytes.extend_from_slice(key.verifying_key().as_bytes());
    std::fs::write(&path, serde_json::to_string(&bytes).unwrap()).unwrap();
    let path = path.to_str().unwrap();
    assert!(matches!(
        signing(&add_args(&[
            "--keypair",
            path,
            "--rpc-url",
            "http://127.0.0.1:1"
        ]))
        .unwrap(),
        Signing::Keypair { rpc: Some(_), .. }
    ));
    bytes[40] ^= 1;
    std::fs::write(
        dir.path().join("bad.json"),
        serde_json::to_string(&bytes).unwrap(),
    )
    .unwrap();
    assert!(load_keypair(&dir.path().join("bad.json")).is_err());
}

#[test]
fn proposer_and_status_are_read_and_rendered() {
    colored::control::set_override(false);
    let server = MockServer::json_sequence(vec![
        (200, json!({ "token": "session" }).to_string()),
        (
            200,
            json!({ "pubkey": PROPOSER, "permissions": "initiate", "rootKeyId": "rk-1" })
                .to_string(),
        ),
        (
            200,
            json!({ "jsonrpc": "2.0", "id": 1, "result": { "context": { "slot": 1 }, "value": 120_000_000u64 } })
                .to_string(),
        ),
    ]);
    let rpc = format!("{}/rpc", server.base_url());
    let result = block_on(fetch_proposer(
        &agent(&server),
        &ProposerArgs {
            target: hosted(),
            rpc_url: Some(rpc.clone()),
        },
    ))
    .unwrap();
    let mint = server.request();
    assert_eq!(mint.request_line, "POST /ws/sessions HTTP/1.1");
    let mint_body: Value = serde_json::from_str(&mint.body).unwrap();
    assert_eq!(mint_body["scopes"], json!(["protocol:read"]));
    let read = server.request();
    assert_eq!(
        read.request_line,
        format!("GET /api/protocol-deployments/{DEPLOYMENT_ID}/squads/proposer HTTP/1.1")
    );
    assert_eq!(read.header("authorization"), Some("Bearer session"));
    let balance = server.request();
    assert_eq!(balance.request_line, "POST /rpc HTTP/1.1");
    assert!(balance.body.contains(PROPOSER), "{}", balance.body);

    // The JSON shape: the server's fields plus the target, balance and next step.
    assert_eq!(result["pubkey"], PROPOSER);
    assert_eq!(result["permissions"], "initiate");
    assert_eq!(result["rootKeyId"], "rk-1");
    assert_eq!(result["deploymentId"], DEPLOYMENT_ID);
    assert_eq!(result["target"], "arete");
    assert_eq!(result["proposerLamports"], 120_000_000u64);
    assert_eq!(result["balance"], "ok");
    let text = render_proposer(&result);
    assert!(text.contains(&format!("pubkey       {PROPOSER}")), "{text}");
    assert!(text.contains("Initiate only (mask 1)"), "{text}");
    assert!(text.contains("0.12 SOL"), "{text}");
    assert!(
        text.contains(&format!(
            "a4 protocol squads add-proposer {DEPLOYMENT_ID} --multisig <multisig>"
        )),
        "{text}"
    );

    // Not a member, and low on SOL.
    let server = MockServer::json_sequence(vec![
        (200, json!({ "token": "session" }).to_string()),
        (
            200,
            json!({ "ready": false, "member": false, "mask": null, "threshold": 2,
                "multisig": MULTISIG, "proposer": PROPOSER, "problem": "not_member" })
            .to_string(),
        ),
        (
            200,
            json!({ "jsonrpc": "2.0", "id": 1, "result": { "value": 1_000_000u64 } }).to_string(),
        ),
    ]);
    let rpc = format!("{}/rpc", server.base_url());
    let result = block_on(fetch_status(
        &agent(&server),
        &StatusArgs {
            target: hosted(),
            multisig: MULTISIG.into(),
            rpc_url: Some(rpc),
        },
    ))
    .unwrap();
    server.request();
    assert_eq!(
        server.request().request_line,
        format!(
            "GET /api/protocol-deployments/{DEPLOYMENT_ID}/squads/status?multisig={MULTISIG} HTTP/1.1"
        )
    );
    assert_eq!(result["ready"], false);
    assert_eq!(result["problem"], "not_member");
    assert_eq!(result["balance"], "low");
    let text = render_status(&result);
    assert!(text.contains("ready      no"), "{text}");
    assert!(text.contains("problem    not_member"), "{text}");
    assert!(text.contains("0.001 SOL low: fund it"), "{text}");
    assert!(
        text.contains(&format!(
            "Add it with `a4 protocol squads add-proposer {DEPLOYMENT_ID} --multisig {MULTISIG} --member <member with Initiate>`"
        )),
        "{text}"
    );

    let ready = json!({ "ready": true, "member": true, "mask": 1, "threshold": 2,
        "multisig": MULTISIG, "proposer": PROPOSER, "proposerLamports": null,
        "balance": "unavailable", "balanceNote": "The balance was not read: no RPC URL is configured" });
    assert!(status_next(&hosted(), &ready).starts_with("Ready"));
    let text = render_status(&ready);
    assert!(text.contains("member     yes (mask 1)"), "{text}");
    assert!(
        text.contains("unavailable (no RPC URL is configured)"),
        "{text}"
    );
    let wrong = json!({ "ready": false, "member": true, "mask": 7, "problem": "permissions_invalid", "multisig": MULTISIG });
    assert!(status_next(&hosted(), &wrong).contains("mask 7"));
}

#[test]
fn add_proposer_prints_the_unsigned_transaction() {
    colored::control::set_override(false);
    let answer = json!({
        "transactionBase64": "AQAB", "transactionFormat": "v0", "transactionIndex": 4,
        "proposal": "Prop1111111111111111111111111111111111111111", "configTransaction": "Conf111111111111111111111111111111111111111",
        "multisig": MULTISIG, "member": PROPOSER, "proposer": "Pro2222222222222222222222222222222222222222",
        "lastValidBlockHeight": 99
    });
    let server = MockServer::json_sequence(vec![
        (200, json!({ "token": "session" }).to_string()),
        (200, answer.to_string()),
    ]);
    let result = block_on(add_proposer(
        &agent(&server),
        &add_args(&["--transaction-format", "v0"]),
    ))
    .unwrap();
    server.request();
    let post = server.request();
    assert_eq!(
        post.request_line,
        format!("POST /api/protocol-deployments/{DEPLOYMENT_ID}/squads/add-proposer HTTP/1.1")
    );
    let body: Value = serde_json::from_str(&post.body).unwrap();
    assert_eq!(
        body,
        json!({ "multisig": MULTISIG, "member": PROPOSER, "transactionFormat": "v0" })
    );
    assert_eq!(result["sent"], false);
    assert_eq!(result["transactionIndex"], 4);
    assert_eq!(result["deploymentId"], DEPLOYMENT_ID);
    let text = render_add_proposer(&result);
    assert!(
        text.contains("Unsigned transaction (v0, base64):\nAQAB\n"),
        "{text}"
    );
    assert!(text.contains("(transaction 4)"), "{text}");
    assert!(
        text.contains(&format!("{PROPOSER} signs and sends this transaction")),
        "{text}"
    );
    assert!(
        text.contains("approve and execute config proposal 4 in Squads"),
        "{text}"
    );

    // A refusal says what to do.
    let server = MockServer::json_sequence(vec![
        (200, json!({ "token": "session" }).to_string()),
        (
            409,
            json!({ "error": "not used", "code": "protocol-squads-not-used" }).to_string(),
        ),
    ]);
    let error = block_on(add_proposer(&agent(&server), &add_args(&[]))).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("no step whose authority is a Squads proposer"),
        "{error}"
    );
}

#[test]
fn a_keypair_must_be_the_member() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("other.json");
    let key = SigningKey::from_bytes(&[9; 32]);
    let mut bytes = vec![9u8; 32];
    bytes.extend_from_slice(key.verifying_key().as_bytes());
    std::fs::write(&path, serde_json::to_string(&bytes).unwrap()).unwrap();
    let server = MockServer::json_sequence(vec![
        (200, json!({ "token": "session" }).to_string()),
        (
            200,
            json!({ "transactionBase64": "AQAB", "transactionIndex": 1, "multisig": MULTISIG, "member": PROPOSER })
                .to_string(),
        ),
    ]);
    let error = block_on(add_proposer(
        &agent(&server),
        &add_args(&[
            "--keypair",
            path.to_str().unwrap(),
            "--rpc-url",
            "http://127.0.0.1:1",
        ]),
    ))
    .unwrap_err();
    assert!(error.to_string().contains("the member signs"), "{error}");
}

#[test]
fn a_keypair_signs_and_sends_through_the_rpc() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("member.json");
    let key = SigningKey::from_bytes(&[11; 32]);
    let member = bs58::encode(key.verifying_key().as_bytes()).into_string();
    let mut bytes = vec![11u8; 32];
    bytes.extend_from_slice(key.verifying_key().as_bytes());
    std::fs::write(&path, serde_json::to_string(&bytes).unwrap()).unwrap();
    // An unsigned transaction the member pays for, as add-proposer returns.
    let unsigned = crate::squads::tx::compile_legacy(
        key.verifying_key().as_bytes(),
        &[crate::squads::Instruction {
            program: crate::squads::program_id(),
            accounts: vec![crate::squads::Meta::writable([3; 32])],
            data: vec![9; 8],
        }],
        &[5; 32],
    )
    .unwrap();
    let server = MockServer::json_sequence(vec![
        (200, json!({ "token": "session" }).to_string()),
        (
            200,
            json!({ "transactionBase64": base64::engine::general_purpose::STANDARD.encode(&unsigned),
                "transactionFormat": "legacy", "transactionIndex": 2, "proposal": "P", "configTransaction": "C",
                "multisig": MULTISIG, "member": member, "proposer": PROPOSER })
            .to_string(),
        ),
    ]);
    let (chain, state) = crate::commands::dev::squads::chain_tests::fake_chain();
    let mut args = add_args(&[
        "--keypair",
        path.to_str().unwrap(),
        "--rpc-url",
        &format!("{chain}/rpc"),
    ]);
    args.member = member.clone();
    let result = block_on(add_proposer(&agent(&server), &args)).unwrap();
    assert_eq!(result["sent"], true);
    // The fake chain checked the member's signature before accepting it.
    let sent = state.lock().unwrap().sent.clone();
    assert_eq!(sent.len(), 1);
    assert_eq!(
        result["signature"],
        crate::squads::tx::transaction_signature(&sent[0]).unwrap()
    );
    assert!(result["next"]
        .as_str()
        .unwrap()
        .contains("approve and execute it in Squads"));
}

#[test]
fn balances_print_in_sol() {
    assert_eq!(sol(120_000_000), "0.12 SOL");
    assert_eq!(sol(1_000_000_000), "1 SOL");
    assert_eq!(sol(5), "0.000000005 SOL");
}

#[test]
fn an_unfunded_proposer_is_reported_with_the_servers_balance_and_minimum() {
    colored::control::set_override(false);
    // A newer server reports the balance and the minimum itself, so no RPC
    // is asked (the sequence has no RPC answer).
    let server = MockServer::json_sequence(vec![
        (200, json!({ "token": "session" }).to_string()),
        (
            200,
            json!({ "ready": false, "member": true, "mask": 1, "threshold": 2,
                "multisig": MULTISIG, "proposer": PROPOSER, "problem": "proposer_unfunded",
                "proposerLamports": 1_000_000u64, "proposerMinLamports": 4_000_000u64 })
            .to_string(),
        ),
    ]);
    let result = block_on(fetch_status(
        &agent(&server),
        &StatusArgs {
            target: hosted(),
            multisig: MULTISIG.into(),
            rpc_url: None,
        },
    ))
    .unwrap();
    assert_eq!(result["proposerLamports"], 1_000_000u64);
    assert_eq!(result["proposerMinLamports"], 4_000_000u64);
    assert_eq!(result["balance"], "low");
    let text = render_status(&result);
    assert!(text.contains("problem    proposer_unfunded"), "{text}");
    assert!(
        text.contains("0.001 SOL (one proposal needs 0.004 SOL) low: fund it"),
        "{text}"
    );
    assert!(text.contains(&format!("Send SOL to {PROPOSER}")), "{text}");

    let dev = SquadsTarget {
        dev: true,
        ..SquadsTarget::default()
    };
    assert!(status_next(&dev, &result).contains(&format!("`a4 dev fund {PROPOSER}`")));
    // Enough SOL by the server's minimum is fine even below the default.
    let mut funded = result.clone();
    funded["proposerLamports"] = json!(5_000_000u64);
    assert!(render_balance(&funded).starts_with("0.005 SOL (one proposal needs 0.004 SOL)\n"));
}
