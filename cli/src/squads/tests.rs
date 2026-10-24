use base64::Engine as _;
use serde_json::Value;

use super::tx::{compile_legacy, compile_v0, LookupTable};
use super::*;

fn vectors() -> Value {
    serde_json::from_str(include_str!("mainnet-vectors.json")).unwrap()
}

fn key(value: &Value) -> Key {
    parse_key(value.as_str().unwrap()).unwrap()
}

fn keys(value: &Value) -> Vec<String> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|key| key.as_str().unwrap().to_string())
        .collect()
}

fn account_keys(instruction: &Instruction) -> Vec<String> {
    instruction
        .accounts
        .iter()
        .map(|meta| b58(&meta.key))
        .collect()
}

fn base64_data(value: &Value) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD
        .decode(value["data"].as_str().unwrap())
        .unwrap()
}

fn hex_data(value: &Value) -> Vec<u8> {
    let text = value["data"].as_str().unwrap();
    (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&text[at..at + 2], 16).unwrap())
        .collect()
}

#[test]
fn discriminators_are_sha256_of_the_anchor_names() {
    // Known Anchor discriminators of the deployed program.
    assert_eq!(
        instruction_discriminator("multisig_create_v2"),
        [0x32, 0xdd, 0xc7, 0x5d, 0x28, 0xf5, 0x8b, 0xe9]
    );
    assert_eq!(
        instruction_discriminator("proposal_approve"),
        [0x90, 0x25, 0xa4, 0x88, 0xbc, 0xd8, 0x2a, 0xf8]
    );
    assert_eq!(
        instruction_discriminator("proposal_activate"),
        [0x0b, 0x22, 0x5c, 0xf8, 0x9a, 0x1b, 0x33, 0x6a]
    );
    assert_eq!(
        instruction_discriminator("config_transaction_execute"),
        [0x72, 0x92, 0xf4, 0xbd, 0xfc, 0x8c, 0x24, 0x28]
    );
    assert_eq!(
        account_discriminator("VaultTransaction"),
        [0xa8, 0xfa, 0xa2, 0x64, 0x51, 0x0e, 0xa2, 0xcf]
    );
}

#[test]
fn pdas_match_the_deployed_program() {
    let vectors = vectors();
    assert_eq!(
        b58(&program_config_pda()),
        "BSTq9w3kZwNwpBXJEvTZz2G9ZTNyKBvoSeXMvwb4cNZr"
    );
    assert_eq!(b58(&program_config_pda()), vectors["programConfig"]["key"]);

    // multisig_create_v2's multisig is the PDA of its create key.
    let create = &vectors["multisigCreateV2"]["accounts"];
    assert_eq!(b58(&multisig_pda(&key(&create[3]))), create[2]);

    // proposal_create and vault_transaction_create of one transaction name
    // the proposal and transaction PDAs of the same index.
    let proposal = &vectors["proposalCreate"];
    let data = hex_data(proposal);
    assert_eq!(data[..8], instruction_discriminator("proposal_create"));
    let index = u64::from_le_bytes(data[8..16].try_into().unwrap());
    let multisig = key(&proposal["accounts"][0]);
    assert_eq!(
        b58(&proposal_pda(&multisig, index)),
        proposal["accounts"][1]
    );
    let transaction = &vectors["vaultTransactionCreate"]["accounts"];
    assert_eq!(transaction[0], proposal["accounts"][0]);
    assert_eq!(b58(&transaction_pda(&multisig, index)), transaction[1]);
}

#[test]
fn accounts_decode() {
    let vectors = vectors();
    let config = ProgramConfig::decode(&base64_data(&vectors["programConfig"])).unwrap();
    // The creation fee goes to the treasury multisig_create_v2 names.
    assert_eq!(
        b58(&config.treasury),
        vectors["multisigCreateV2"]["accounts"][1]
    );

    let multisig = Multisig::decode(&base64_data(&vectors["multisig"])).unwrap();
    assert_eq!(
        b58(&multisig_pda(&multisig.create_key)),
        vectors["multisig"]["key"]
    );
    assert!(!multisig.is_controlled());
    assert!(multisig.threshold >= 1);
    assert!(!multisig.members.is_empty());
    assert!(ProgramConfig::decode(&base64_data(&vectors["multisig"])).is_err());

    // Accounts of another type, or truncated, are refused.
    let mut truncated = base64_data(&vectors["multisig"]);
    truncated.truncate(60);
    assert!(Multisig::decode(&truncated).is_err());
}

#[test]
fn multisig_create_v2_encodes_like_the_deployed_program_expects() {
    let vectors = vectors();
    let observed = &vectors["multisigCreateV2"];
    let data = hex_data(observed);
    let accounts = &observed["accounts"];
    // Decode the observed args (config authority, threshold, members) and
    // re-encode them; this vector has no config authority and a memo, so
    // compare up to the memo.
    assert_eq!(data[8], 0, "no config authority");
    let threshold = u16::from_le_bytes(data[9..11].try_into().unwrap());
    let count = u32::from_le_bytes(data[11..15].try_into().unwrap()) as usize;
    let members: Vec<(Key, u8)> = (0..count)
        .map(|n| {
            let at = 15 + 33 * n;
            (data[at..at + 32].try_into().unwrap(), data[at + 32])
        })
        .collect();
    let after_members = 15 + 33 * count;
    let time_lock = u32::from_le_bytes(data[after_members..after_members + 4].try_into().unwrap());
    let built = multisig_create_v2(&CreateMultisig {
        treasury: key(&accounts[1]),
        create_key: key(&accounts[3]),
        creator: key(&accounts[4]),
        threshold,
        members: &members,
        time_lock,
    });
    assert_eq!(account_keys(&built), keys(accounts));
    assert_eq!(built.data[..after_members + 4], data[..after_members + 4]);
    // No rent collector and no memo.
    assert_eq!(built.data[after_members + 4..], [0, 0]);
    let flags: Vec<(bool, bool)> = built
        .accounts
        .iter()
        .map(|meta| (meta.signer, meta.writable))
        .collect();
    assert_eq!(
        flags,
        [
            (false, false),
            (false, true),
            (false, true),
            (true, false),
            (true, true),
            (false, false)
        ]
    );
}

#[test]
fn votes_and_config_execution_match_observed_instructions() {
    let vectors = vectors();
    let approve = &vectors["proposalApprove"];
    let multisig = key(&approve["accounts"][0]);
    let member = key(&approve["accounts"][1]);
    // The approve vector belongs to the proposal_create vector's proposal.
    assert_eq!(
        approve["accounts"][2],
        vectors["proposalCreate"]["accounts"][1]
    );
    let index = u64::from_le_bytes(
        hex_data(&vectors["proposalCreate"])[8..16]
            .try_into()
            .unwrap(),
    );
    let built = proposal_approve(&multisig, &member, index);
    assert_eq!(account_keys(&built), keys(&approve["accounts"]));
    assert_eq!(built.data, hex_data(approve));

    let activate = &vectors["proposalActivate"];
    let built = proposal_activate(
        &key(&activate["accounts"][0]),
        &key(&activate["accounts"][1]),
        0,
    );
    assert_eq!(built.data, hex_data(activate));
    assert_eq!(account_keys(&built)[..2], keys(&activate["accounts"])[..2]);

    let execute = &vectors["configTransactionExecute"];
    assert_eq!(
        hex_data(execute),
        config_transaction_execute(&[1; 32], &[2; 32], 1).data
    );
    let built = config_transaction_execute(&[1; 32], &[2; 32], 3);
    assert_eq!(built.accounts.len(), keys(&execute["accounts"]).len());
    assert_eq!(built.accounts[2].key, proposal_pda(&[1; 32], 3));
    assert_eq!(built.accounts[3].key, transaction_pda(&[1; 32], 3));
    assert_eq!(built.accounts[5].key, system_program());
    assert_eq!(b58(&built.accounts[5].key), execute["accounts"][5]);
}

#[test]
fn vault_execution_passes_the_accounts_the_program_reads() {
    for observed in vectors()["vaultTransactionExecute"].as_array().unwrap() {
        let accounts = keys(&observed["accounts"]);
        let transaction_key = parse_key(&accounts[2]).unwrap();
        let transaction =
            VaultTransaction::decode(&base64_data(&observed["transactionAccount"])).unwrap();
        assert_eq!(b58(&transaction.multisig), accounts[0]);
        assert_eq!(
            b58(&transaction_pda(&transaction.multisig, transaction.index)),
            accounts[2]
        );
        assert_eq!(
            b58(&proposal_pda(&transaction.multisig, transaction.index)),
            accounts[1]
        );
        // Lookup tables resolve to what the observed instruction passed.
        let lookups = &transaction.message.address_table_lookups;
        let statics = lookups.len() + transaction.message.account_keys.len();
        let mut loaded = accounts[4 + statics..].iter();
        let tables: Vec<Vec<Key>> = lookups
            .iter()
            .map(|lookup| {
                let mut table = vec![[0u8; 32]; 256];
                for index in lookup
                    .writable_indexes
                    .iter()
                    .chain(&lookup.readonly_indexes)
                {
                    table[*index as usize] = parse_key(loaded.next().unwrap()).unwrap();
                }
                table
            })
            .collect();
        let member = parse_key(&accounts[3]).unwrap();
        let built = vault_transaction_execute(
            &transaction.multisig,
            &member,
            transaction.index,
            vault_execute_accounts(&transaction_key, &transaction, &tables).unwrap(),
        );
        assert_eq!(account_keys(&built), accounts, "{}", observed["signature"]);
        assert_eq!(built.data, hex_data(observed));
        // The vault is never a signer of the outer transaction.
        let vault = vault_pda(&transaction.multisig, transaction.vault_index);
        assert!(built
            .accounts
            .iter()
            .filter(|meta| meta.key == vault)
            .all(|meta| !meta.signer));
    }
}

#[test]
fn proposal_and_config_transaction_layouts_decode() {
    let multisig = [9u8; 32];
    let mut proposal = account_discriminator("Proposal").to_vec();
    proposal.extend_from_slice(&multisig);
    proposal.extend_from_slice(&5u64.to_le_bytes());
    proposal.push(3); // Approved
    proposal.extend_from_slice(&1_700_000_000i64.to_le_bytes());
    proposal.push(254); // bump
    proposal.extend_from_slice(&1u32.to_le_bytes());
    proposal.extend_from_slice(&[7; 32]);
    proposal.extend_from_slice(&0u32.to_le_bytes());
    proposal.extend_from_slice(&0u32.to_le_bytes());
    let decoded = Proposal::decode(&proposal).unwrap();
    assert_eq!(decoded.status, ProposalStatus::Approved);
    assert_eq!(decoded.transaction_index, 5);
    assert_eq!(decoded.approved, vec![[7; 32]]);

    // Executing carries no timestamp.
    let mut executing = account_discriminator("Proposal").to_vec();
    executing.extend_from_slice(&multisig);
    executing.extend_from_slice(&5u64.to_le_bytes());
    executing.push(4);
    executing.push(254);
    executing.extend_from_slice(&[0; 12]);
    assert_eq!(
        Proposal::decode(&executing).unwrap().status,
        ProposalStatus::Executing
    );

    let mut config = account_discriminator("ConfigTransaction").to_vec();
    config.extend_from_slice(&multisig);
    config.extend_from_slice(&[8; 32]);
    config.extend_from_slice(&5u64.to_le_bytes());
    config.push(255);
    config.extend_from_slice(&2u32.to_le_bytes());
    config.push(0); // AddMember
    config.extend_from_slice(&[6; 32]);
    config.push(PERMISSION_INITIATE);
    config.push(2); // ChangeThreshold
    config.extend_from_slice(&2u16.to_le_bytes());
    let decoded = ConfigTransaction::decode(&config).unwrap();
    assert_eq!(
        decoded.actions,
        vec![
            ConfigAction::AddMember {
                key: [6; 32],
                mask: 1
            },
            ConfigAction::ChangeThreshold { threshold: 2 }
        ]
    );
    assert!(decoded.actions[0].describe().contains("(mask 1)"));

    let mut spending = account_discriminator("ConfigTransaction").to_vec();
    spending.extend_from_slice(&[0; 73]); // multisig, creator, index, bump
    spending.extend_from_slice(&1u32.to_le_bytes());
    spending.push(4);
    assert!(ConfigTransaction::decode(&spending)
        .unwrap_err()
        .to_string()
        .contains("spending limit"));
}

#[test]
fn legacy_transactions_order_accounts_and_signers() {
    let payer = [1u8; 32];
    let other_signer = [2u8; 32];
    let program = [3u8; 32];
    let writable = [4u8; 32];
    let readonly = [5u8; 32];
    let instruction = Instruction {
        program,
        accounts: vec![
            Meta::readonly(readonly),
            Meta::writable(writable),
            Meta::readonly_signer(other_signer),
            Meta::writable_signer(payer),
        ],
        data: vec![9, 9],
    };
    let blockhash = [7u8; 32];
    let wire = compile_legacy(&payer, &[instruction], &blockhash).unwrap();
    let decoded = arete_mcp::prepared_summary::decode_transaction(&wire).unwrap();
    assert_eq!(decoded.required_signatures, 2);
    assert_eq!(
        decoded.static_keys,
        vec![
            b58(&payer),
            b58(&other_signer),
            b58(&writable),
            b58(&program),
            b58(&readonly)
        ]
    );
    assert_eq!(decoded.writable_static, vec![0, 2]);
    assert_eq!(decoded.instructions.len(), 1);
    // header, keys, blockhash, then the instruction: program 3, accounts
    // [4, 2, 1, 0], data [9, 9].
    let tail = &wire[wire.len() - 10..];
    assert_eq!(tail, [1, 3, 4, 4, 2, 1, 0, 2, 9, 9]);

    // Too big to send is refused before signing.
    let big = Instruction {
        program,
        accounts: Vec::new(),
        data: vec![0; 1300],
    };
    assert!(compile_legacy(&payer, &[big], &blockhash).is_err());
}

#[test]
fn compute_budget_and_lookup_tables() {
    assert_eq!(
        compute_unit_limit(400_000).data,
        [2, 0x80, 0x1a, 0x06, 0x00]
    );
    let mut table = vec![0u8; 56];
    table.extend_from_slice(&[3; 32]);
    table.extend_from_slice(&[4; 32]);
    assert_eq!(
        lookup_table_addresses(&table).unwrap(),
        vec![[3; 32], [4; 32]]
    );
    assert!(lookup_table_addresses(&[0; 57]).is_err());
}

#[test]
fn large_vault_execute_fits_through_lookup_tables() {
    let multisig = [1u8; 32];
    let member = [2u8; 32];
    let table_key = [3u8; 32];
    let program = [4u8; 32];
    // 40 accounts loaded from one table: 1,280 bytes of addresses alone.
    let addresses: Vec<Key> = (0..40u8).map(|index| [100 + index; 32]).collect();
    let transaction = VaultTransaction {
        multisig,
        creator: member,
        index: 7,
        vault_index: 0,
        ephemeral_signer_bumps: Vec::new(),
        message: VaultMessage {
            num_signers: 1,
            num_writable_signers: 1,
            num_writable_non_signers: 0,
            account_keys: vec![vault_pda(&multisig, 0), program],
            instructions: vec![CompiledInstruction {
                program_id_index: 1,
                account_indexes: (0..42).collect(),
                data: vec![1],
            }],
            address_table_lookups: vec![TableLookup {
                account_key: table_key,
                writable_indexes: (0..20).collect(),
                readonly_indexes: (20..40).collect(),
            }],
        },
    };
    let transaction_key = transaction_pda(&multisig, 7);
    let remaining = vault_execute_accounts(
        &transaction_key,
        &transaction,
        std::slice::from_ref(&addresses),
    )
    .unwrap();
    let instructions = [
        compute_unit_limit(400_000),
        vault_transaction_execute(&multisig, &member, 7, remaining),
    ];
    let blockhash = [9u8; 32];
    assert!(compile_legacy(&member, &instructions, &blockhash).is_err());

    let tables = [LookupTable {
        key: table_key,
        addresses: addresses.clone(),
    }];
    let wire = compile_v0(&member, &instructions, &blockhash, &tables).unwrap();
    assert!(wire.len() <= super::tx::MAX_TRANSACTION_BYTES);
    let decoded = arete_mcp::prepared_summary::decode_transaction(&wire).unwrap();
    assert_eq!(decoded.version, Some(0));
    assert_eq!(decoded.required_signatures, 1);
    assert_eq!(decoded.lookups.len(), 1);
    assert_eq!(decoded.lookups[0].table, b58(&table_key));
    assert_eq!(decoded.lookups[0].writable, (0..20).collect::<Vec<u8>>());
    assert_eq!(decoded.lookups[0].readonly, (20..40).collect::<Vec<u8>>());
    // Loaded accounts are never static; the table and programs stay static.
    for address in &addresses {
        assert!(!decoded.static_keys.contains(&b58(address)));
    }
    assert!(decoded.static_keys.contains(&b58(&table_key)));
    // The execute instruction's accounts resolve to the same keys in order:
    // static keys, then writable loads, then read-only loads.
    let order: Vec<String> = decoded
        .static_keys
        .iter()
        .cloned()
        .chain(addresses.iter().map(b58))
        .collect();
    let execute = &decoded.instructions[1];
    let expected = &instructions[1].accounts;
    assert_eq!(execute.accounts.len(), expected.len());
    for (index, meta) in execute.accounts.iter().zip(expected) {
        assert_eq!(order[*index], b58(&meta.key));
    }
}

// ---------------------------------------------------------------------------
// Builders checked against the vendored IDL subset
// ---------------------------------------------------------------------------

fn idl() -> Value {
    serde_json::from_str(include_str!("idl-subset.json")).unwrap()
}

fn idl_instruction<'a>(idl: &'a Value, name: &str) -> &'a Value {
    idl["instructions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|instruction| instruction["name"] == name)
        .unwrap()
}

fn idl_type<'a>(idl: &'a Value, name: &str) -> &'a Value {
    &idl["types"]
        .as_array()
        .unwrap()
        .iter()
        .find(|ty| ty["name"] == name)
        .unwrap_or_else(|| panic!("type {name} is not in the IDL subset"))["type"]
}

/// Borsh-encode `value` as the IDL type `ty` says (the subset's types only).
fn idl_encode(idl: &Value, ty: &Value, value: &Value, out: &mut Vec<u8>) {
    match ty {
        Value::String(name) => match name.as_str() {
            "u8" => out.push(value.as_u64().unwrap() as u8),
            "bool" => out.push(u8::from(value.as_bool().unwrap())),
            "u16" => out.extend_from_slice(&(value.as_u64().unwrap() as u16).to_le_bytes()),
            "u32" => out.extend_from_slice(&(value.as_u64().unwrap() as u32).to_le_bytes()),
            "u64" => out.extend_from_slice(&value.as_u64().unwrap().to_le_bytes()),
            "publicKey" => out.extend_from_slice(&key(value)),
            "string" => {
                let text = value.as_str().unwrap();
                out.extend_from_slice(&(text.len() as u32).to_le_bytes());
                out.extend_from_slice(text.as_bytes());
            }
            other => panic!("unsupported IDL type {other}"),
        },
        Value::Object(map) if map.contains_key("option") => {
            if value.is_null() {
                out.push(0);
            } else {
                out.push(1);
                idl_encode(idl, &map["option"], value, out);
            }
        }
        Value::Object(map) if map.contains_key("vec") => {
            let items = value.as_array().unwrap();
            out.extend_from_slice(&(items.len() as u32).to_le_bytes());
            for item in items {
                idl_encode(idl, &map["vec"], item, out);
            }
        }
        Value::Object(map) if map.contains_key("defined") => {
            let defined = idl_type(idl, map["defined"].as_str().unwrap());
            match defined["kind"].as_str().unwrap() {
                "struct" => {
                    for field in defined["fields"].as_array().unwrap() {
                        let name = field["name"].as_str().unwrap();
                        idl_encode(idl, &field["type"], &value[name], out);
                    }
                }
                "enum" => {
                    // `{ "<Variant>": { fields } }`
                    let (variant, fields) = value.as_object().unwrap().iter().next().unwrap();
                    let variants = defined["variants"].as_array().unwrap();
                    let at = variants
                        .iter()
                        .position(|candidate| candidate["name"] == variant.as_str())
                        .unwrap();
                    out.push(at as u8);
                    for field in variants[at]["fields"].as_array().into_iter().flatten() {
                        let name = field["name"].as_str().unwrap();
                        idl_encode(idl, &field["type"], &fields[name], out);
                    }
                }
                other => panic!("unsupported IDL kind {other}"),
            }
        }
        other => panic!("unsupported IDL type {other}"),
    }
}

/// The instruction data the IDL gives for `name` with `args`.
fn idl_data(idl: &Value, name: &str, args: &Value) -> Vec<u8> {
    let instruction = idl_instruction(idl, name);
    let snake: String = name
        .chars()
        .flat_map(|c| {
            if c.is_ascii_uppercase() {
                vec!['_', c.to_ascii_lowercase()]
            } else {
                vec![c]
            }
        })
        .collect();
    let mut data = instruction_discriminator(&snake).to_vec();
    for arg in instruction["args"].as_array().unwrap() {
        idl_encode(
            idl,
            &arg["type"],
            &args[arg["name"].as_str().unwrap()],
            &mut data,
        );
    }
    data
}

/// The builder's accounts carry the IDL's mutability and signer flags.
fn assert_idl_accounts(idl: &Value, name: &str, built: &Instruction) {
    let accounts = idl_instruction(idl, name)["accounts"].as_array().unwrap();
    assert_eq!(accounts.len(), built.accounts.len(), "{name}");
    for (account, meta) in accounts.iter().zip(&built.accounts) {
        assert_eq!(
            account["isMut"].as_bool().unwrap(),
            meta.writable,
            "{name}.{}",
            account["name"]
        );
        assert_eq!(
            account["isSigner"].as_bool().unwrap(),
            meta.signer,
            "{name}.{}",
            account["name"]
        );
    }
}

#[test]
fn config_transaction_create_encodes_like_the_idl() {
    let idl = idl();
    assert_eq!(idl["address"], PROGRAM_ID);
    let multisig = [7u8; 32];
    let creator = [8u8; 32];
    let proposer = [9u8; 32];
    let built = config_transaction_create(
        &multisig,
        &creator,
        4,
        &[ConfigAction::AddMember {
            key: proposer,
            mask: PERMISSION_INITIATE,
        }],
    );
    assert_eq!(
        built.data,
        idl_data(
            &idl,
            "configTransactionCreate",
            &serde_json::json!({ "args": {
                "actions": [{ "AddMember": { "newMember": {
                    "key": b58(&proposer), "permissions": { "mask": 1 }
                } } }],
                "memo": null
            } })
        )
    );
    assert_idl_accounts(&idl, "configTransactionCreate", &built);
    assert_eq!(
        account_keys(&built),
        vec![
            b58(&multisig),
            b58(&transaction_pda(&multisig, 4)),
            b58(&creator),
            b58(&creator),
            b58(&system_program()),
        ]
    );

    // Every modelled action encodes as its IDL variant, and decodes back.
    let actions = [
        ConfigAction::RemoveMember { key: proposer },
        ConfigAction::ChangeThreshold { threshold: 2 },
        ConfigAction::SetTimeLock { seconds: 60 },
        ConfigAction::SetRentCollector {
            collector: Some(creator),
        },
    ];
    let built = config_transaction_create(&multisig, &creator, 5, &actions);
    assert_eq!(
        built.data,
        idl_data(
            &idl,
            "configTransactionCreate",
            &serde_json::json!({ "args": {
                "actions": [
                    { "RemoveMember": { "oldMember": b58(&proposer) } },
                    { "ChangeThreshold": { "newThreshold": 2 } },
                    { "SetTimeLock": { "newTimeLock": 60 } },
                    { "SetRentCollector": { "newRentCollector": b58(&creator) } }
                ],
                "memo": null
            } })
        )
    );
    let mut account = account_discriminator("ConfigTransaction").to_vec();
    account.extend_from_slice(&multisig);
    account.extend_from_slice(&creator);
    account.extend_from_slice(&5u64.to_le_bytes());
    account.push(255);
    account.extend_from_slice(&built.data[8..built.data.len() - 1]);
    assert_eq!(
        ConfigTransaction::decode(&account).unwrap().actions,
        actions.to_vec()
    );
}

#[test]
fn proposal_create_encodes_like_the_idl_and_the_deployed_program() {
    let idl = idl();
    let multisig = [7u8; 32];
    let creator = [8u8; 32];
    let built = proposal_create(&multisig, &creator, 4, false);
    assert_eq!(
        built.data,
        idl_data(
            &idl,
            "proposalCreate",
            &serde_json::json!({ "args": { "transactionIndex": 4, "draft": false } })
        )
    );
    assert_idl_accounts(&idl, "proposalCreate", &built);

    // An observed mainnet proposal_create (creator and rent payer the same
    // member) re-encodes byte for byte.
    let vectors = vectors();
    let observed = &vectors["proposalCreate"];
    let data = hex_data(observed);
    let index = u64::from_le_bytes(data[8..16].try_into().unwrap());
    let multisig = key(&observed["accounts"][0]);
    let creator = key(&observed["accounts"][2]);
    let built = proposal_create(&multisig, &creator, index, data[16] == 1);
    assert_eq!(built.data, data);
    assert_eq!(account_keys(&built), keys(&observed["accounts"]));
}

#[test]
fn add_proposer_compiles_with_one_signer() {
    let multisig = [7u8; 32];
    let creator = [8u8; 32];
    let instructions = [
        config_transaction_create(
            &multisig,
            &creator,
            2,
            &[ConfigAction::AddMember {
                key: [9; 32],
                mask: PERMISSION_INITIATE,
            }],
        ),
        proposal_create(&multisig, &creator, 2, false),
    ];
    let wire = compile_legacy(&creator, &instructions, &[5; 32]).unwrap();
    let decoded = arete_mcp::prepared_summary::decode_transaction(&wire).unwrap();
    assert_eq!(decoded.required_signatures, 1);
    assert_eq!(decoded.static_keys[0], b58(&creator));
    assert_eq!(decoded.instructions.len(), 2);
}
