//! The dev Squads commands against a fake environment: an API that mints
//! environment tokens and a JSON-RPC endpoint that holds accounts, checks
//! every signature of a sent transaction and applies the votes it carries.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::sync::{Arc, Mutex};

use arete_mcp::dev::{DevClient, DevProject, DevSession, DevWallet};
use arete_mcp::dev_env::{EnvApi, EnvRpc, EnvSelection};
use base64::Engine as _;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};

use super::*;

const ENV_ID: &str = "denv_01hzx3k8m9q2r4s5t6v7w8x9yz";
const MULTISIG: Key = [77; 32];
const THRESHOLD: usize = 2;

#[derive(Default)]
pub(crate) struct FakeChain {
    /// base58 key -> (owner, data).
    accounts: HashMap<String, (String, Vec<u8>)>,
    pub(crate) sent: Vec<Vec<u8>>,
}

pub(crate) type Shared = Arc<Mutex<FakeChain>>;

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn multisig_data(members: &[(Key, u8)], transaction_index: u64) -> Vec<u8> {
    let mut data = v4::account_discriminator("Multisig").to_vec();
    data.extend_from_slice(&[1; 32]); // create key
    data.extend_from_slice(&[0; 32]); // no config authority
    data.extend_from_slice(&(THRESHOLD as u16).to_le_bytes());
    data.extend_from_slice(&0u32.to_le_bytes());
    data.extend_from_slice(&transaction_index.to_le_bytes());
    data.extend_from_slice(&0u64.to_le_bytes());
    data.push(0); // no rent collector
    data.push(255);
    data.extend_from_slice(&(members.len() as u32).to_le_bytes());
    for (key, mask) in members {
        data.extend_from_slice(key);
        data.push(*mask);
    }
    data
}

fn proposal_data(index: u64, status: u8, approved: &[Key]) -> Vec<u8> {
    let mut data = v4::account_discriminator("Proposal").to_vec();
    data.extend_from_slice(&MULTISIG);
    data.extend_from_slice(&index.to_le_bytes());
    data.push(status);
    if status != 4 {
        data.extend_from_slice(&1_700_000_000i64.to_le_bytes());
    }
    data.push(254);
    data.extend_from_slice(&(approved.len() as u32).to_le_bytes());
    for key in approved {
        data.extend_from_slice(key);
    }
    data.extend_from_slice(&0u32.to_le_bytes());
    data.extend_from_slice(&0u32.to_le_bytes());
    data
}

fn add_member_data(index: u64, member: &Key) -> Vec<u8> {
    let mut data = v4::account_discriminator("ConfigTransaction").to_vec();
    data.extend_from_slice(&MULTISIG);
    data.extend_from_slice(&[2; 32]);
    data.extend_from_slice(&index.to_le_bytes());
    data.push(255);
    data.extend_from_slice(&1u32.to_le_bytes());
    data.push(0);
    data.extend_from_slice(member);
    data.push(v4::PERMISSION_INITIATE);
    data
}

/// Check every signature and apply proposal votes and executions.
fn apply(state: &mut FakeChain, wire: &[u8]) -> Result<(), String> {
    let decoded =
        arete_mcp::prepared_summary::decode_transaction(wire).map_err(|e| e.to_string())?;
    let message = &wire[decoded.message.clone()];
    for (index, signature) in decoded
        .signatures
        .iter()
        .enumerate()
        .take(decoded.required_signatures)
    {
        let key: [u8; 32] = parse_key(&decoded.static_keys[index]).unwrap();
        VerifyingKey::from_bytes(&key)
            .unwrap()
            .verify(message, &Signature::from_bytes(signature))
            .map_err(|_| format!("signature {index} is missing or invalid"))?;
    }
    for instruction in &decoded.instructions {
        if decoded.static_keys[instruction.program_index] != v4::PROGRAM_ID {
            continue;
        }
        let account = |n: usize| decoded.static_keys[instruction.accounts[n]].clone();
        let data = &instruction.data;
        let proposal_key = |n: usize| account(n);
        if data[..8] == v4::instruction_discriminator("proposal_activate") {
            let key = proposal_key(2);
            let (_, bytes) = state.accounts.get(&key).cloned().unwrap();
            let proposal = v4::Proposal::decode(&bytes).unwrap();
            state.accounts.insert(
                key,
                (
                    v4::PROGRAM_ID.into(),
                    proposal_data(proposal.transaction_index, 1, &[]),
                ),
            );
        } else if data[..8] == v4::instruction_discriminator("proposal_approve") {
            let key = proposal_key(2);
            let (_, bytes) = state.accounts.get(&key).cloned().unwrap();
            let proposal = v4::Proposal::decode(&bytes).unwrap();
            if proposal.status != ProposalStatus::Active {
                return Err("the proposal is not active".into());
            }
            let mut approved = proposal.approved.clone();
            approved.push(parse_key(&account(1)).unwrap());
            let status = if approved.len() >= THRESHOLD { 3 } else { 1 };
            state.accounts.insert(
                key,
                (
                    v4::PROGRAM_ID.into(),
                    proposal_data(proposal.transaction_index, status, &approved),
                ),
            );
        } else if data[..8] == v4::instruction_discriminator("config_transaction_execute") {
            let key = proposal_key(2);
            let (_, bytes) = state.accounts.get(&key).cloned().unwrap();
            let proposal = v4::Proposal::decode(&bytes).unwrap();
            state.accounts.insert(
                key,
                (
                    v4::PROGRAM_ID.into(),
                    proposal_data(proposal.transaction_index, 5, &proposal.approved),
                ),
            );
        }
    }
    state.sent.push(wire.to_vec());
    Ok(())
}

fn rpc(state: &Shared, origin: &str, path: &str, body: &Value) -> Value {
    if path.ends_with("/token") {
        return json!({ "token": "env-token", "expiresAt": 4_102_444_800i64, "rpcUrl": format!("{origin}/rpc") });
    }
    let mut state = state.lock().unwrap();
    let params = &body["params"];
    let result = match body["method"].as_str().unwrap_or_default() {
        "getBalance" => json!({ "context": { "slot": 1 }, "value": 10_000_000_000u64 }),
        "getAccountInfo" => {
            let value = state
                .accounts
                .get(params[0].as_str().unwrap())
                .map(|(owner, data)| {
                    json!({ "owner": owner, "data": [b64(data), "base64"], "lamports": 1, "executable": false })
                })
                .unwrap_or(Value::Null);
            json!({ "context": { "slot": 1 }, "value": value })
        }
        "getLatestBlockhash" => {
            json!({ "context": { "slot": 1 }, "value": { "blockhash": b58(&[5; 32]), "lastValidBlockHeight": 9 } })
        }
        "sendTransaction" => {
            let wire = base64::engine::general_purpose::STANDARD
                .decode(params[0].as_str().unwrap())
                .unwrap();
            if let Err(error) = apply(&mut state, &wire) {
                return json!({ "jsonrpc": "2.0", "id": 1, "error": { "code": -32002, "message": error } });
            }
            json!(crate::squads::tx::transaction_signature(&wire).unwrap())
        }
        "getSignatureStatuses" => {
            json!({ "context": { "slot": 1 }, "value": [{ "confirmationStatus": "confirmed", "err": null }] })
        }
        other => panic!("unexpected RPC method {other}"),
    };
    json!({ "jsonrpc": "2.0", "id": 1, "result": result })
}

pub(crate) fn fake_chain() -> (String, Shared) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let state: Shared = Arc::default();
    let shared = state.clone();
    let served = origin.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut first = String::new();
            if reader.read_line(&mut first).is_err() {
                continue;
            }
            let path = first.split_whitespace().nth(1).unwrap_or("/").to_string();
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
            let text = rpc(&shared, &served, &path, &body).to_string();
            let _ = std::io::Write::write_all(
                &mut stream,
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                    text.len()
                )
                .as_bytes(),
            );
        }
    });
    (origin, state)
}

fn dev_client(dir: &Path, origin: &str) -> DevClient {
    let project = DevProject::new(dir);
    std::fs::create_dir_all(project.dev_dir()).unwrap();
    let session = DevSession::offline(&EnvSelection::new(ENV_ID, "app", origin));
    let api = EnvApi::new(reqwest::Client::new(), origin, "a4_sk_dev").unwrap();
    DevClient::new(project, session, EnvRpc::new(api, ENV_ID).unwrap())
}

fn wallet(client: &DevClient, name: &str) -> DevWallet {
    DevWallet::load_or_create(&client.project.wallets_dir(), name)
        .unwrap()
        .0
}

fn proposal_args(index: u64, wallet: &str) -> ProposalArgs {
    ProposalArgs {
        project: ProjectArgs::default(),
        multisig: b58(&MULTISIG),
        index,
        wallet: wallet.into(),
    }
}

fn last_instructions(state: &Shared) -> Vec<(String, Vec<String>, Vec<u8>)> {
    let state = state.lock().unwrap();
    let wire = state.sent.last().unwrap();
    let decoded = arete_mcp::prepared_summary::decode_transaction(wire).unwrap();
    decoded
        .instructions
        .iter()
        .map(|instruction| {
            (
                decoded.static_keys[instruction.program_index].clone(),
                instruction
                    .accounts
                    .iter()
                    .map(|index| decoded.static_keys[*index].clone())
                    .collect(),
                instruction.data.clone(),
            )
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn create_signs_with_the_creator_and_the_create_key() {
    let (origin, state) = fake_chain();
    let dir = tempfile::tempdir().unwrap();
    let client = dev_client(dir.path(), &origin);
    let alice = wallet(&client, "alice");
    let bob = wallet(&client, "bob");
    let vectors: Value =
        serde_json::from_str(include_str!("../../../squads/mainnet-vectors.json")).unwrap();
    let config = base64::engine::general_purpose::STANDARD
        .decode(vectors["programConfig"]["data"].as_str().unwrap())
        .unwrap();
    let treasury = v4::ProgramConfig::decode(&config).unwrap().treasury;
    state.lock().unwrap().accounts.insert(
        b58(&v4::program_config_pda()),
        (v4::PROGRAM_ID.into(), config),
    );
    let chain = Chain::Dev(&client);
    let outsider = b58(&[42; 32]);
    let result = create(
        &client,
        &chain,
        &CreateArgs {
            project: ProjectArgs::default(),
            members: vec!["alice".into(), "bob".into(), outsider.clone()],
            threshold: 2,
            wallet: None,
            proposer: None,
        },
    )
    .await
    .unwrap();
    let create_key = parse_key(result["createKey"].as_str().unwrap()).unwrap();
    let multisig = v4::multisig_pda(&create_key);
    assert_eq!(result["multisig"], b58(&multisig));
    assert_eq!(result["vault"], b58(&v4::vault_pda(&multisig, 0)));
    assert_eq!(result["creator"], alice.pubkey());
    assert_eq!(result["members"][0]["wallet"], "alice");
    assert_eq!(result["members"][2]["wallet"], Value::Null);
    let next = result["next"].as_str().unwrap();
    assert!(
        next.contains("a4 dev squads add-proposer --multisig"),
        "{next}"
    );
    assert!(!next.contains("a4 protocol squads add-proposer"), "{next}");
    assert_eq!(result["proposer"], Value::Null);

    let instructions = last_instructions(&state);
    assert_eq!(instructions.len(), 1);
    let (program, accounts, data) = &instructions[0];
    assert_eq!(program, v4::PROGRAM_ID);
    let mut members = vec![
        (alice.pubkey_bytes(), v4::PERMISSION_ALL),
        (bob.pubkey_bytes(), v4::PERMISSION_ALL),
        ([42; 32], v4::PERMISSION_ALL),
    ];
    members.sort();
    let expected = v4::multisig_create_v2(&v4::CreateMultisig {
        treasury,
        create_key,
        creator: alice.pubkey_bytes(),
        threshold: 2,
        members: &members,
        time_lock: 0,
    });
    assert_eq!(*data, expected.data);
    assert_eq!(
        *accounts,
        expected
            .accounts
            .iter()
            .map(|meta| b58(&meta.key))
            .collect::<Vec<_>>()
    );

    // Bad input is refused before anything is sent.
    for (members, threshold, wallet) in [
        (vec!["alice", "alice"], 1, None),
        (vec!["alice"], 2, None),
        (vec![outsider.as_str()], 1, None),
        (vec!["alice"], 1, Some("nobody")),
    ] {
        let error = create(
            &client,
            &chain,
            &CreateArgs {
                project: ProjectArgs::default(),
                members: members.iter().map(|m| m.to_string()).collect(),
                threshold,
                wallet: wallet.map(str::to_string),
                proposer: None,
            },
        )
        .await
        .unwrap_err();
        assert!(!error.to_string().is_empty());
    }
    assert_eq!(state.lock().unwrap().sent.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn approve_activates_drafts_and_execute_runs_the_config_transaction() {
    let (origin, state) = fake_chain();
    let dir = tempfile::tempdir().unwrap();
    let client = dev_client(dir.path(), &origin);
    let alice = wallet(&client, "alice");
    let bob = wallet(&client, "bob");
    wallet(&client, "carol");
    let proposer = [61u8; 32];
    {
        let mut state = state.lock().unwrap();
        let members = [
            (alice.pubkey_bytes(), v4::PERMISSION_ALL),
            (bob.pubkey_bytes(), v4::PERMISSION_ALL),
        ];
        state.accounts.insert(
            b58(&MULTISIG),
            (v4::PROGRAM_ID.into(), multisig_data(&members, 1)),
        );
        state.accounts.insert(
            b58(&v4::proposal_pda(&MULTISIG, 1)),
            (v4::PROGRAM_ID.into(), proposal_data(1, 0, &[])),
        );
        state.accounts.insert(
            b58(&v4::transaction_pda(&MULTISIG, 1)),
            (v4::PROGRAM_ID.into(), add_member_data(1, &proposer)),
        );
    }
    let chain = Chain::Dev(&client);

    // Executing before the votes is refused.
    let error = execute(&client, &chain, &proposal_args(1, "alice"), false)
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("must be approved first"),
        "{error}"
    );

    // The draft is activated and approved in one transaction.
    let first = approve(&client, &chain, &proposal_args(1, "alice"))
        .await
        .unwrap();
    assert_eq!(first["status"], "active");
    assert_eq!(first["approvals"], 1);
    let instructions = last_instructions(&state);
    assert_eq!(
        instructions
            .iter()
            .map(|(_, _, data)| data[..8].to_vec())
            .collect::<Vec<_>>(),
        vec![
            v4::instruction_discriminator("proposal_activate").to_vec(),
            v4::instruction_discriminator("proposal_approve").to_vec()
        ]
    );
    assert_eq!(
        instructions[1].1,
        vec![
            b58(&MULTISIG),
            alice.pubkey(),
            b58(&v4::proposal_pda(&MULTISIG, 1))
        ]
    );
    let again = approve(&client, &chain, &proposal_args(1, "alice"))
        .await
        .unwrap_err();
    assert!(again.to_string().contains("already approved"), "{again}");
    let outsider = approve(&client, &chain, &proposal_args(1, "carol"))
        .await
        .unwrap_err();
    assert!(outsider.to_string().contains("not a member"), "{outsider}");
    let unknown = approve(&client, &chain, &proposal_args(1, "dave"))
        .await
        .unwrap_err();
    assert!(
        unknown.to_string().contains("not a dev wallet"),
        "{unknown}"
    );

    let second = approve(&client, &chain, &proposal_args(1, "bob"))
        .await
        .unwrap();
    assert_eq!(second["status"], "approved");
    assert!(second["next"]
        .as_str()
        .unwrap()
        .contains("a4 dev squads execute"));
    assert_eq!(last_instructions(&state).len(), 1);

    // A config transaction is not a vault transaction.
    let error = execute(&client, &chain, &proposal_args(1, "bob"), true)
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("run `a4 dev squads execute`"),
        "{error}"
    );

    let executed = execute(&client, &chain, &proposal_args(1, "bob"), false)
        .await
        .unwrap();
    assert_eq!(executed["kind"], "config");
    assert_eq!(
        executed["actions"],
        json!([format!("add member {} (mask 1)", b58(&proposer))])
    );
    let instructions = last_instructions(&state);
    assert_eq!(
        instructions[0].2,
        v4::instruction_discriminator("config_transaction_execute")
    );
    assert_eq!(
        instructions[0].1,
        vec![
            b58(&MULTISIG),
            bob.pubkey(),
            b58(&v4::proposal_pda(&MULTISIG, 1)),
            b58(&v4::transaction_pda(&MULTISIG, 1)),
            bob.pubkey(),
            b58(&v4::system_program()),
        ]
    );
    let shown = show(&chain, &MULTISIG).await.unwrap();
    assert_eq!(shown["transactionIndex"], 1);
    assert_eq!(
        shown["members"][0]["permissions"],
        "initiate, vote, execute"
    );
    assert_eq!(shown["configAuthority"], Value::Null);
}

#[tokio::test(flavor = "multi_thread")]
async fn create_with_a_proposer_adds_it_with_initiate_only() {
    let (origin, state) = fake_chain();
    let dir = tempfile::tempdir().unwrap();
    let client = dev_client(dir.path(), &origin);
    let alice = wallet(&client, "alice");
    let vectors: Value =
        serde_json::from_str(include_str!("../../../squads/mainnet-vectors.json")).unwrap();
    let config = base64::engine::general_purpose::STANDARD
        .decode(vectors["programConfig"]["data"].as_str().unwrap())
        .unwrap();
    let treasury = v4::ProgramConfig::decode(&config).unwrap().treasury;
    state.lock().unwrap().accounts.insert(
        b58(&v4::program_config_pda()),
        (v4::PROGRAM_ID.into(), config),
    );
    let chain = Chain::Dev(&client);
    let proposer = [61u8; 32];
    let args = |members: Vec<String>| CreateArgs {
        project: ProjectArgs::default(),
        members,
        threshold: 1,
        wallet: None,
        proposer: Some(b58(&proposer)),
    };
    let result = create(&client, &chain, &args(vec!["alice".into()]))
        .await
        .unwrap();
    assert_eq!(result["proposer"], b58(&proposer));
    assert_eq!(result["members"][1]["mask"], v4::PERMISSION_INITIATE);
    assert!(result["next"]
        .as_str()
        .unwrap()
        .contains("a4 protocol squads status --dev"));
    let create_key = parse_key(result["createKey"].as_str().unwrap()).unwrap();
    let mut members = vec![
        (alice.pubkey_bytes(), v4::PERMISSION_ALL),
        (proposer, v4::PERMISSION_INITIATE),
    ];
    members.sort();
    let expected = v4::multisig_create_v2(&v4::CreateMultisig {
        treasury,
        create_key,
        creator: alice.pubkey_bytes(),
        threshold: 1,
        members: &members,
        time_lock: 0,
    });
    assert_eq!(last_instructions(&state)[0].2, expected.data);

    // The proposer is never also a full member.
    let error = create(&client, &chain, &args(vec!["alice".into(), b58(&proposer)]))
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("leave it out of --members"),
        "{error}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn add_proposer_opens_an_initiate_only_config_proposal() {
    let (origin, state) = fake_chain();
    let dir = tempfile::tempdir().unwrap();
    let client = dev_client(dir.path(), &origin);
    let alice = wallet(&client, "alice");
    let voter = wallet(&client, "voter");
    wallet(&client, "carol");
    let proposer = [61u8; 32];
    let set_members = |members: &[(Key, u8)]| {
        state.lock().unwrap().accounts.insert(
            b58(&MULTISIG),
            (v4::PROGRAM_ID.into(), multisig_data(members, 3)),
        );
    };
    set_members(&[
        (alice.pubkey_bytes(), v4::PERMISSION_ALL),
        (voter.pubkey_bytes(), v4::PERMISSION_VOTE),
    ]);
    let chain = Chain::Dev(&client);
    let args = |wallet: &str| AddProposerArgs {
        project: ProjectArgs::default(),
        multisig: b58(&MULTISIG),
        proposer: b58(&proposer),
        wallet: wallet.into(),
    };

    // Only a member that can initiate opens it.
    for (wallet, expected) in [
        ("voter", "cannot initiate"),
        ("carol", "not a member"),
        ("dave", "not a dev wallet"),
    ] {
        let error = add_proposer(&client, &chain, &args(wallet))
            .await
            .unwrap_err();
        assert!(error.to_string().contains(expected), "{wallet}: {error}");
    }
    assert!(state.lock().unwrap().sent.is_empty());

    let result = add_proposer(&client, &chain, &args("alice")).await.unwrap();
    assert_eq!(result["transactionIndex"], 4);
    assert_eq!(result["mask"], 1);
    let next = result["next"].as_str().unwrap();
    assert!(next.contains("a4 dev squads approve --multisig"), "{next}");
    assert!(next.contains("--index 4 --wallet alice"), "{next}");
    assert!(next.contains("a4 dev squads execute"), "{next}");

    let instructions = last_instructions(&state);
    assert_eq!(instructions.len(), 2);
    let expected = [
        v4::config_transaction_create(
            &MULTISIG,
            &alice.pubkey_bytes(),
            4,
            &[v4::ConfigAction::AddMember {
                key: proposer,
                mask: v4::PERMISSION_INITIATE,
            }],
        ),
        v4::proposal_create(&MULTISIG, &alice.pubkey_bytes(), 4, false),
    ];
    for ((program, accounts, data), expected) in instructions.iter().zip(&expected) {
        assert_eq!(program, v4::PROGRAM_ID);
        assert_eq!(*data, expected.data);
        assert_eq!(
            *accounts,
            expected
                .accounts
                .iter()
                .map(|meta| b58(&meta.key))
                .collect::<Vec<_>>()
        );
    }

    // Already an Initiate-only member: nothing to add. Another mask: remove
    // it first.
    set_members(&[
        (alice.pubkey_bytes(), v4::PERMISSION_ALL),
        (proposer, v4::PERMISSION_INITIATE),
    ]);
    let error = add_proposer(&client, &chain, &args("alice"))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("nothing to add"), "{error}");
    set_members(&[
        (alice.pubkey_bytes(), v4::PERMISSION_ALL),
        (proposer, v4::PERMISSION_ALL),
    ]);
    let error = add_proposer(&client, &chain, &args("alice"))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("remove it first"), "{error}");
    assert_eq!(state.lock().unwrap().sent.len(), 1);
}
