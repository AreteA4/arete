//! `a4 dev squads`: Squads v4 multisigs on the project's cloud dev
//! environment, so a protocol whose steps are proposed to a multisig can be
//! tested end to end without the Squads app (which cannot see a fork).
//!
//! `create` makes a multisig whose members all hold every permission (and,
//! with `--proposer`, the environment's proposer with Initiate only);
//! `add-proposer` opens a config proposal adding the proposer to an existing
//! multisig; `approve` votes on a proposal (activating a draft first);
//! `execute` runs an approved config transaction and `vault-execute` an
//! approved vault transaction (such as a protocol step's proposal). Every
//! transaction is built here and signed by dev wallets. These commands only
//! ever talk to the project's dev environment: there is no RPC option to
//! point them anywhere else.

use anyhow::{anyhow, bail, Context, Result};
use arete_mcp::dev::{validate_wallet_name, DevClient, DevWallet};
use clap::{Args, Subcommand};
use colored::Colorize;
use ed25519_dalek::SigningKey;
use serde_json::{json, Value};

use super::ProjectArgs;
use crate::squads::tx::{compile_legacy, compile_v0, Chain, LookupTable};
use crate::squads::{self as v4, b58, parse_key, Instruction, Key, ProposalStatus};

/// Compute units for executing a vault transaction (the stored
/// instructions run inside it).
const VAULT_EXECUTE_UNITS: u32 = 1_400_000;

#[derive(Subcommand, Debug)]
pub enum DevSquadsCommands {
    /// Create a Squads v4 multisig whose members hold every permission
    /// (mask 7); prints the multisig and its vault 0
    Create(CreateArgs),
    /// Open a config proposal adding a Squads proposer to a multisig as an
    /// Initiate-only member (mask 1); a dev wallet that can initiate creates
    /// and pays
    AddProposer(AddProposerArgs),
    /// Show a multisig: members, threshold, last transaction index, vault 0
    Show(ShowArgs),
    /// Approve a proposal with a dev wallet (a draft is activated first)
    Approve(ProposalArgs),
    /// Execute an approved config transaction, such as the add-proposer
    /// proposal
    Execute(ProposalArgs),
    /// Execute an approved vault transaction, such as a protocol step's
    /// proposal
    VaultExecute(ProposalArgs),
}

#[derive(Args, Debug, Clone)]
pub struct CreateArgs {
    #[command(flatten)]
    pub project: ProjectArgs,
    /// Members: dev wallet names (created and funded on first use) or
    /// public keys, comma-separated
    #[arg(
        long,
        value_delimiter = ',',
        required = true,
        value_name = "NAME|PUBKEY,..."
    )]
    pub members: Vec<String>,
    /// Approvals a transaction needs (1 to the number of members)
    #[arg(long)]
    pub threshold: u16,
    /// Dev wallet that creates and pays (default: the first member, when it
    /// is a dev wallet)
    #[arg(long)]
    pub wallet: Option<String>,
    /// Also add this Squads proposer (`a4 protocol squads proposer --dev`)
    /// as a member with the Initiate permission only; it does not vote or
    /// count toward the threshold
    #[arg(long, value_name = "PUBKEY")]
    pub proposer: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct AddProposerArgs {
    #[command(flatten)]
    pub project: ProjectArgs,
    /// The multisig account
    #[arg(long, value_name = "ADDRESS")]
    pub multisig: String,
    /// The Squads proposer to add (`a4 protocol squads proposer --dev`)
    #[arg(long, value_name = "PUBKEY")]
    pub proposer: String,
    /// The dev wallet (a member with the Initiate permission) that creates
    /// and pays for the proposal
    #[arg(long)]
    pub wallet: String,
}

#[derive(Args, Debug, Clone)]
pub struct ShowArgs {
    #[command(flatten)]
    pub project: ProjectArgs,
    /// The multisig account
    #[arg(long, value_name = "ADDRESS")]
    pub multisig: String,
}

#[derive(Args, Debug, Clone)]
pub struct ProposalArgs {
    #[command(flatten)]
    pub project: ProjectArgs,
    /// The multisig account
    #[arg(long, value_name = "ADDRESS")]
    pub multisig: String,
    /// The transaction index (`transactionIndex` of `a4 dev squads
    /// add-proposer`, or the multisig's latest from `a4 dev squads show`)
    #[arg(long)]
    pub index: u64,
    /// The dev wallet (a multisig member) that signs
    #[arg(long)]
    pub wallet: String,
}

pub async fn run(command: DevSquadsCommands, json: bool) -> Result<()> {
    let (project, render): (&ProjectArgs, fn(&Value)) = match &command {
        DevSquadsCommands::Create(args) => (&args.project, render_create),
        DevSquadsCommands::AddProposer(args) => (&args.project, render_add_proposer),
        DevSquadsCommands::Show(args) => (&args.project, render_show),
        DevSquadsCommands::Approve(args) => (&args.project, render_approve),
        DevSquadsCommands::Execute(args) | DevSquadsCommands::VaultExecute(args) => {
            (&args.project, render_execute)
        }
    };
    let client = super::client(project)?;
    let chain = Chain::Dev(&client);
    let result = match &command {
        DevSquadsCommands::Create(args) => create(&client, &chain, args).await?,
        DevSquadsCommands::AddProposer(args) => add_proposer(&client, &chain, args).await?,
        DevSquadsCommands::Show(args) => show(&chain, &parse_key(&args.multisig)?).await?,
        DevSquadsCommands::Approve(args) => approve(&client, &chain, args).await?,
        DevSquadsCommands::Execute(args) => execute(&client, &chain, args, false).await?,
        DevSquadsCommands::VaultExecute(args) => execute(&client, &chain, args, true).await?,
    };
    super::print(json, &result, || render(&result));
    Ok(())
}

/// A dev wallet that already exists (funded again when empty).
async fn existing_wallet(client: &DevClient, name: &str) -> Result<DevWallet> {
    let name = name.trim();
    if validate_wallet_name(name).is_err()
        || !client
            .project
            .wallets_dir()
            .join(format!("{name}.json"))
            .is_file()
    {
        bail!("'{name}' is not a dev wallet under .arete/dev/wallets/ (`a4 dev wallet {name}` creates one)");
    }
    client.wallet(name).await
}

/// A member: an existing dev wallet, a public key, or a new dev wallet.
async fn member(client: &DevClient, value: &str) -> Result<(Option<String>, Key)> {
    let value = value.trim();
    let is_wallet = validate_wallet_name(value).is_ok()
        && client
            .project
            .wallets_dir()
            .join(format!("{value}.json"))
            .is_file();
    if !is_wallet {
        if let Ok(key) = parse_key(value) {
            return Ok((None, key));
        }
    }
    let wallet = client
        .wallet(value)
        .await
        .with_context(|| format!("'{value}' is neither a public key nor a dev wallet name"))?;
    Ok((Some(value.to_string()), wallet.pubkey_bytes()))
}

/// Compile `instructions` paid by `payer` (a v0 transaction loading from
/// `tables` when there are any), sign with it and `extra`, send through the
/// dev environment and wait for confirmation.
async fn send(
    chain: &Chain<'_>,
    payer: &DevWallet,
    instructions: &[Instruction],
    extra: &[&SigningKey],
    tables: &[LookupTable],
) -> Result<String> {
    let blockhash = chain.latest_blockhash().await?;
    let mut transaction = if tables.is_empty() {
        compile_legacy(&payer.pubkey_bytes(), instructions, &blockhash)?
    } else {
        compile_v0(&payer.pubkey_bytes(), instructions, &blockhash, tables)?
    };
    transaction = payer.sign_transaction(&transaction)?;
    for key in extra {
        transaction = arete_mcp::dev::sign_transaction(&transaction, key)?;
    }
    chain.send_and_confirm(&transaction).await
}

async fn multisig_account(chain: &Chain<'_>, multisig: &Key) -> Result<v4::Multisig> {
    v4::Multisig::decode(&chain.squads_account(multisig, "multisig").await?)
}

async fn proposal_account(
    chain: &Chain<'_>,
    multisig: &Key,
    index: u64,
) -> Result<Option<v4::Proposal>> {
    match chain.account(&v4::proposal_pda(multisig, index)).await? {
        Some((data, owner)) if owner == v4::PROGRAM_ID => v4::Proposal::decode(&data).map(Some),
        Some(_) => bail!("the proposal account of transaction {index} is not a Squads proposal"),
        None => Ok(None),
    }
}

fn mask_names(mask: u8) -> String {
    let mut names = Vec::new();
    for (bit, name) in [
        (v4::PERMISSION_INITIATE, "initiate"),
        (v4::PERMISSION_VOTE, "vote"),
        (v4::PERMISSION_EXECUTE, "execute"),
    ] {
        if mask & bit != 0 {
            names.push(name);
        }
    }
    if names.is_empty() {
        "none".into()
    } else {
        names.join(", ")
    }
}

/// `a4 dev squads create`.
async fn create(client: &DevClient, chain: &Chain<'_>, args: &CreateArgs) -> Result<Value> {
    let mut members: Vec<(Option<String>, Key)> = Vec::new();
    for value in &args.members {
        let (name, key) = member(client, value).await?;
        if members.iter().any(|(_, known)| *known == key) {
            bail!("{} is listed twice in --members", b58(&key));
        }
        members.push((name, key));
    }
    if args.threshold == 0 || args.threshold as usize > members.len() {
        bail!(
            "--threshold must be between 1 and the number of members ({})",
            members.len()
        );
    }
    let proposer = args.proposer.as_deref().map(parse_key).transpose()?;
    if let Some(proposer) = &proposer {
        if members.iter().any(|(_, key)| key == proposer) {
            bail!(
                "{} is both in --members and --proposer; the proposer is added with Initiate only, so leave it out of --members",
                b58(proposer)
            );
        }
    }
    let payer_name = match &args.wallet {
        Some(wallet) => wallet.clone(),
        None => members[0].0.clone().ok_or_else(|| {
            anyhow!("The first member is not a dev wallet; pass --wallet <dev wallet> to create and pay")
        })?,
    };
    let payer = existing_wallet(client, &payer_name).await?;
    let config = v4::ProgramConfig::decode(
        &chain
            .squads_account(&v4::program_config_pda(), "program config")
            .await?,
    )?;
    let mut seed = [0u8; 32];
    getrandom::getrandom(&mut seed).map_err(|error| anyhow!("no OS randomness: {error}"))?;
    let create_key = SigningKey::from_bytes(&seed);
    let create_key_bytes = *create_key.verifying_key().as_bytes();
    let mut keyed: Vec<(Key, u8)> = members
        .iter()
        .map(|(_, key)| (*key, v4::PERMISSION_ALL))
        .collect();
    if let Some(proposer) = proposer {
        keyed.push((proposer, v4::PERMISSION_INITIATE));
    }
    keyed.sort();
    let instruction = v4::multisig_create_v2(&v4::CreateMultisig {
        treasury: config.treasury,
        create_key: create_key_bytes,
        creator: payer.pubkey_bytes(),
        threshold: args.threshold,
        members: &keyed,
        time_lock: 0,
    });
    let signature = send(chain, &payer, &[instruction], &[&create_key], &[]).await?;
    let multisig = v4::multisig_pda(&create_key_bytes);
    let vault = v4::vault_pda(&multisig, 0);
    let project = project_flag(&args.project);
    let next = match &proposer {
        Some(proposer) => format!(
            "Use it as the protocol's Squads multisig: the proposer is already an Initiate-only member. `a4 protocol squads status --dev{project} --multisig {}` checks it (`a4 dev fund{project} {} 1` if it is unfunded).",
            b58(&multisig),
            b58(proposer)
        ),
        None => format!(
            "Use it as the protocol's Squads multisig. `a4 dev squads add-proposer{project} --multisig {} --proposer <proposer> --wallet {payer_name}` proposes adding the proposer (`a4 protocol squads proposer --dev` prints it), or pass --proposer to create.",
            b58(&multisig)
        ),
    };
    let mut listed: Vec<Value> = members
        .iter()
        .map(|(name, key)| {
            json!({
                "pubkey": b58(key),
                "wallet": name,
                "mask": v4::PERMISSION_ALL,
            })
        })
        .collect();
    if let Some(proposer) = &proposer {
        listed.push(json!({
            "pubkey": b58(proposer),
            "wallet": Value::Null,
            "mask": v4::PERMISSION_INITIATE,
            "proposer": true,
        }));
    }
    Ok(json!({
        "multisig": b58(&multisig),
        "vault": b58(&vault),
        "createKey": b58(&create_key_bytes),
        "threshold": args.threshold,
        "members": listed,
        "proposer": proposer.as_ref().map(b58),
        "creator": payer.pubkey(),
        "creationFeeLamports": config.multisig_creation_fee,
        "signature": signature,
        "next": next,
    }))
}

/// ` --project DIR` when a project was named, for follow-up commands.
fn project_flag(project: &ProjectArgs) -> String {
    project
        .project
        .as_deref()
        .map(|dir| format!(" --project {}", dir.display()))
        .unwrap_or_default()
}

/// `a4 dev squads add-proposer`: a config transaction with exactly one
/// `AddMember { proposer, mask 1 }` and its proposal, at the multisig's
/// next transaction index, created and paid by a dev wallet that can
/// initiate.
async fn add_proposer(
    client: &DevClient,
    chain: &Chain<'_>,
    args: &AddProposerArgs,
) -> Result<Value> {
    let multisig = parse_key(&args.multisig)?;
    let proposer = parse_key(&args.proposer)?;
    let account = multisig_account(chain, &multisig).await?;
    if account.is_controlled() {
        bail!(
            "This multisig has a config authority ({}), so members cannot change it by proposal; its config authority adds the proposer directly",
            b58(&account.config_authority)
        );
    }
    match account.mask_of(&proposer) {
        Some(v4::PERMISSION_INITIATE) => bail!(
            "{} is already a member with Initiate only (mask 1); there is nothing to add",
            args.proposer.trim()
        ),
        Some(mask) => bail!(
            "{} is already a member with permissions {} (mask {mask}); remove it first with a config proposal, then add it with Initiate only",
            args.proposer.trim(),
            mask_names(mask)
        ),
        None => {}
    }
    let wallet = existing_wallet(client, &args.wallet).await?;
    let mask = account.mask_of(&wallet.pubkey_bytes()).ok_or_else(|| {
        anyhow!(
            "The dev wallet '{}' ({}) is not a member of this multisig",
            args.wallet,
            wallet.pubkey()
        )
    })?;
    if mask & v4::PERMISSION_INITIATE == 0 {
        bail!(
            "The dev wallet '{}' cannot initiate on this multisig (permissions: {}); pass a member with Initiate",
            args.wallet,
            mask_names(mask)
        );
    }
    let index = account
        .transaction_index
        .checked_add(1)
        .ok_or_else(|| anyhow!("the multisig's transaction index is exhausted"))?;
    let creator = wallet.pubkey_bytes();
    let instructions = [
        v4::config_transaction_create(
            &multisig,
            &creator,
            index,
            &[v4::ConfigAction::AddMember {
                key: proposer,
                mask: v4::PERMISSION_INITIATE,
            }],
        ),
        v4::proposal_create(&multisig, &creator, index, false),
    ];
    let signature = send(chain, &wallet, &instructions, &[], &[]).await?;
    let project = project_flag(&args.project);
    let multisig_text = b58(&multisig);
    let wallet_name = args.wallet.trim();
    Ok(json!({
        "multisig": multisig_text,
        "proposer": b58(&proposer),
        "mask": v4::PERMISSION_INITIATE,
        "transactionIndex": index,
        "configTransaction": b58(&v4::transaction_pda(&multisig, index)),
        "proposal": b58(&v4::proposal_pda(&multisig, index)),
        "wallet": wallet.pubkey(),
        "threshold": account.threshold,
        "signature": signature,
        "next": format!(
            "Config proposal {index} is open. Approve it with enough members (`a4 dev squads approve{project} --multisig {multisig_text} --index {index} --wallet {wallet_name}`), execute it (`a4 dev squads execute{project} --multisig {multisig_text} --index {index} --wallet {wallet_name}`), then `a4 protocol squads status --dev{project} --multisig {multisig_text}` checks the proposer."
        ),
    }))
}

/// `a4 dev squads show`.
async fn show(chain: &Chain<'_>, multisig: &Key) -> Result<Value> {
    let account = multisig_account(chain, multisig).await?;
    Ok(json!({
        "multisig": b58(multisig),
        "vault": b58(&v4::vault_pda(multisig, 0)),
        "threshold": account.threshold,
        "timeLock": account.time_lock,
        "transactionIndex": account.transaction_index,
        "staleTransactionIndex": account.stale_transaction_index,
        "configAuthority": account.is_controlled().then(|| b58(&account.config_authority)),
        "members": account.members.iter().map(|(key, mask)| json!({
            "pubkey": b58(key),
            "mask": mask,
            "permissions": mask_names(*mask),
        })).collect::<Vec<_>>(),
    }))
}

/// `a4 dev squads approve`.
async fn approve(client: &DevClient, chain: &Chain<'_>, args: &ProposalArgs) -> Result<Value> {
    let multisig = parse_key(&args.multisig)?;
    let account = multisig_account(chain, &multisig).await?;
    let wallet = existing_wallet(client, &args.wallet).await?;
    let mask = account.mask_of(&wallet.pubkey_bytes()).ok_or_else(|| {
        anyhow!(
            "The dev wallet '{}' ({}) is not a member of this multisig",
            args.wallet,
            wallet.pubkey()
        )
    })?;
    if mask & v4::PERMISSION_VOTE == 0 {
        bail!(
            "The dev wallet '{}' cannot vote on this multisig (permissions: {})",
            args.wallet,
            mask_names(mask)
        );
    }
    let proposal = proposal_account(chain, &multisig, args.index)
        .await?
        .ok_or_else(|| {
            anyhow!(
                "Transaction {} of {} has no proposal yet",
                args.index,
                args.multisig
            )
        })?;
    let mut instructions = Vec::new();
    match proposal.status {
        ProposalStatus::Draft => {
            if mask & v4::PERMISSION_INITIATE == 0 {
                bail!(
                    "Proposal {} is a draft and the dev wallet '{}' cannot activate it (no Initiate permission)",
                    args.index,
                    args.wallet
                );
            }
            instructions.push(v4::proposal_activate(
                &multisig,
                &wallet.pubkey_bytes(),
                args.index,
            ));
        }
        ProposalStatus::Active => {}
        other => bail!(
            "Proposal {} is {}; only an active (or draft) proposal takes votes",
            args.index,
            other.as_str()
        ),
    }
    if proposal.approved.contains(&wallet.pubkey_bytes()) {
        bail!(
            "The dev wallet '{}' already approved proposal {}",
            args.wallet,
            args.index
        );
    }
    instructions.push(v4::proposal_approve(
        &multisig,
        &wallet.pubkey_bytes(),
        args.index,
    ));
    let signature = send(chain, &wallet, &instructions, &[], &[]).await?;
    let after = proposal_account(chain, &multisig, args.index)
        .await?
        .ok_or_else(|| anyhow!("The proposal disappeared after the vote"))?;
    let status = after.status.as_str();
    Ok(json!({
        "multisig": args.multisig,
        "index": args.index,
        "proposal": b58(&v4::proposal_pda(&multisig, args.index)),
        "wallet": wallet.pubkey(),
        "status": status,
        "approvals": after.approved.len(),
        "threshold": account.threshold,
        "signature": signature,
        "next": if after.status == ProposalStatus::Approved {
            format!("Approved. Execute it with `a4 dev squads execute` (config) or `a4 dev squads vault-execute` (vault) --multisig {} --index {} --wallet <member>.", args.multisig, args.index)
        } else {
            format!("{} of {} approvals; approve with another member's dev wallet.", after.approved.len(), account.threshold)
        },
    }))
}

/// `a4 dev squads execute` (config) and `vault-execute`.
async fn execute(
    client: &DevClient,
    chain: &Chain<'_>,
    args: &ProposalArgs,
    vault: bool,
) -> Result<Value> {
    let multisig = parse_key(&args.multisig)?;
    let account = multisig_account(chain, &multisig).await?;
    let wallet = existing_wallet(client, &args.wallet).await?;
    let mask = account.mask_of(&wallet.pubkey_bytes()).unwrap_or(0);
    if mask & v4::PERMISSION_EXECUTE == 0 {
        bail!(
            "The dev wallet '{}' cannot execute on this multisig (it is not a member with Execute)",
            args.wallet
        );
    }
    let proposal = proposal_account(chain, &multisig, args.index)
        .await?
        .ok_or_else(|| {
            anyhow!(
                "Transaction {} of {} has no proposal",
                args.index,
                args.multisig
            )
        })?;
    if proposal.status != ProposalStatus::Approved {
        bail!(
            "Proposal {} is {} ({} of {} approvals); it must be approved first (`a4 dev squads approve`)",
            args.index,
            proposal.status.as_str(),
            proposal.approved.len(),
            account.threshold
        );
    }
    let transaction_key = v4::transaction_pda(&multisig, args.index);
    let data = chain
        .squads_account(&transaction_key, "transaction")
        .await?;
    let is_vault = data.starts_with(&v4::account_discriminator("VaultTransaction"));
    let is_config = data.starts_with(&v4::account_discriminator("ConfigTransaction"));
    let member = wallet.pubkey_bytes();
    let (instructions, details, lookup_tables) = match (vault, is_vault, is_config) {
        (false, _, true) => {
            let config = v4::ConfigTransaction::decode(&data)?;
            let actions: Vec<String> = config.actions.iter().map(|a| a.describe()).collect();
            (
                vec![v4::config_transaction_execute(&multisig, &member, args.index)],
                json!({ "kind": "config", "actions": actions }),
                Vec::new(),
            )
        }
        (true, true, _) => {
            let transaction = v4::VaultTransaction::decode(&data)?;
            let mut tables = Vec::new();
            for lookup in &transaction.message.address_table_lookups {
                let (table, _) = chain.account(&lookup.account_key).await?.ok_or_else(|| {
                    anyhow!(
                        "The lookup table {} does not exist",
                        b58(&lookup.account_key)
                    )
                })?;
                tables.push(v4::lookup_table_addresses(&table)?);
            }
            let remaining = v4::vault_execute_accounts(&transaction_key, &transaction, &tables)?;
            // The outer transaction loads the proposal's accounts through the
            // same tables, or a large proposal would not fit.
            let lookup_tables: Vec<LookupTable> = transaction
                .message
                .address_table_lookups
                .iter()
                .zip(tables)
                .map(|(lookup, addresses)| LookupTable {
                    key: lookup.account_key,
                    addresses,
                })
                .collect();
            (
                vec![
                    v4::compute_unit_limit(VAULT_EXECUTE_UNITS),
                    v4::vault_transaction_execute(&multisig, &member, args.index, remaining),
                ],
                json!({
                    "kind": "vault",
                    "vault": b58(&v4::vault_pda(&multisig, transaction.vault_index)),
                    "instructions": transaction.message.instructions.len(),
                }),
                lookup_tables,
            )
        }
        (false, true, _) => bail!(
            "Transaction {} is a vault transaction; run `a4 dev squads vault-execute`",
            args.index
        ),
        (true, _, true) => bail!(
            "Transaction {} is a config transaction; run `a4 dev squads execute`",
            args.index
        ),
        _ => bail!(
            "Transaction {} is neither a config nor a vault transaction (batches are not supported)",
            args.index
        ),
    };
    let signature = send(chain, &wallet, &instructions, &[], &lookup_tables).await?;
    let mut result = json!({
        "multisig": args.multisig,
        "index": args.index,
        "transaction": b58(&transaction_key),
        "wallet": wallet.pubkey(),
        "signature": signature,
        "next": if vault {
            "Executed. Read the protocol's state again (`a4 dev instance <id>`).".to_string()
        } else {
            format!("Executed. `a4 protocol squads status --dev{} --multisig {}` checks the proposer.", project_flag(&args.project), args.multisig)
        },
    });
    if let (Some(object), Some(details)) = (result.as_object_mut(), details.as_object()) {
        object.extend(details.clone());
    }
    Ok(result)
}

fn render_create(result: &Value) {
    println!(
        "{} {}",
        "multisig".bold(),
        result["multisig"].as_str().unwrap_or_default().green()
    );
    println!(
        "  vault 0    {}",
        result["vault"].as_str().unwrap_or_default()
    );
    println!(
        "  threshold  {} of {}",
        result["threshold"],
        result["members"].as_array().map(Vec::len).unwrap_or(0)
    );
    for member in result["members"].as_array().into_iter().flatten() {
        println!(
            "  member     {}{} ({})",
            member["pubkey"].as_str().unwrap_or_default(),
            member["wallet"]
                .as_str()
                .map(|name| format!(" [{name}]"))
                .or_else(|| (member["proposer"] == true).then(|| " [proposer]".to_string()))
                .unwrap_or_default(),
            mask_names(member["mask"].as_u64().unwrap_or(0) as u8)
        );
    }
    println!(
        "  signature  {}",
        result["signature"].as_str().unwrap_or_default()
    );
    println!("\n{}", result["next"].as_str().unwrap_or_default());
}

fn render_add_proposer(result: &Value) {
    println!(
        "{} config proposal {} on {}",
        "add-proposer".bold(),
        result["transactionIndex"],
        result["multisig"].as_str().unwrap_or_default()
    );
    println!(
        "  proposer   {} (Initiate only, mask 1)",
        result["proposer"].as_str().unwrap_or_default()
    );
    println!(
        "  proposal   {}",
        result["proposal"].as_str().unwrap_or_default()
    );
    println!(
        "  signature  {}",
        result["signature"].as_str().unwrap_or_default()
    );
    println!("\n{}", result["next"].as_str().unwrap_or_default());
}

fn render_show(result: &Value) {
    println!(
        "{} {}",
        "multisig".bold(),
        result["multisig"].as_str().unwrap_or_default()
    );
    println!(
        "  vault 0            {}",
        result["vault"].as_str().unwrap_or_default()
    );
    println!("  threshold          {}", result["threshold"]);
    println!("  time lock          {}s", result["timeLock"]);
    println!("  transaction index  {}", result["transactionIndex"]);
    if let Some(authority) = result["configAuthority"].as_str() {
        println!("  config authority   {authority}");
    }
    for member in result["members"].as_array().into_iter().flatten() {
        println!(
            "  member             {} (mask {}: {})",
            member["pubkey"].as_str().unwrap_or_default(),
            member["mask"],
            member["permissions"].as_str().unwrap_or_default()
        );
    }
}

fn render_approve(result: &Value) {
    println!(
        "{} proposal {} -> {} ({} of {} approvals)",
        "approve".bold(),
        result["index"],
        result["status"].as_str().unwrap_or_default().green(),
        result["approvals"],
        result["threshold"]
    );
    println!(
        "  signature  {}",
        result["signature"].as_str().unwrap_or_default()
    );
    println!("\n{}", result["next"].as_str().unwrap_or_default());
}

fn render_execute(result: &Value) {
    println!(
        "{} {} transaction {}",
        "executed".bold().green(),
        result["kind"].as_str().unwrap_or_default(),
        result["index"]
    );
    for action in result["actions"].as_array().into_iter().flatten() {
        println!("  {}", action.as_str().unwrap_or_default());
    }
    println!(
        "  signature  {}",
        result["signature"].as_str().unwrap_or_default()
    );
    println!("\n{}", result["next"].as_str().unwrap_or_default());
}

#[cfg(test)]
pub(crate) mod chain_tests;

#[cfg(test)]
mod tests {
    use super::*;

    use clap::Parser;

    #[derive(Parser, Debug)]
    struct Cli {
        #[command(subcommand)]
        command: DevSquadsCommands,
    }

    fn parse(args: &[&str]) -> Result<DevSquadsCommands, clap::Error> {
        let mut argv = vec!["squads"];
        argv.extend_from_slice(args);
        Cli::try_parse_from(argv).map(|cli| cli.command)
    }

    #[test]
    fn arguments_parse() {
        let DevSquadsCommands::Create(args) =
            parse(&["create", "--members", "alice,bob,carol", "--threshold", "2"]).unwrap()
        else {
            panic!("not create");
        };
        assert_eq!(args.members, ["alice", "bob", "carol"]);
        assert_eq!(args.threshold, 2);
        assert_eq!(args.wallet, None);
        assert!(parse(&["create", "--threshold", "1"]).is_err());
        let DevSquadsCommands::Create(args) = parse(&[
            "create",
            "--members",
            "alice",
            "--threshold",
            "1",
            "--proposer",
            "P",
        ])
        .unwrap() else {
            panic!("not create");
        };
        assert_eq!(args.proposer.as_deref(), Some("P"));

        let DevSquadsCommands::AddProposer(args) = parse(&[
            "add-proposer",
            "--multisig",
            "M",
            "--proposer",
            "P",
            "--wallet",
            "alice",
        ])
        .unwrap() else {
            panic!("not add-proposer");
        };
        assert_eq!(
            (
                args.multisig.as_str(),
                args.proposer.as_str(),
                args.wallet.as_str()
            ),
            ("M", "P", "alice")
        );
        assert!(parse(&["add-proposer", "--multisig", "M", "--wallet", "alice"]).is_err());
        assert!(parse(&["add-proposer", "--multisig", "M", "--proposer", "P"]).is_err());

        let DevSquadsCommands::VaultExecute(args) = parse(&[
            "vault-execute",
            "--multisig",
            "M",
            "--index",
            "3",
            "--wallet",
            "alice",
            "--project",
            "app",
        ])
        .unwrap() else {
            panic!("not vault-execute");
        };
        assert_eq!(args.index, 3);
        assert_eq!(
            args.project.project.as_deref(),
            Some(std::path::Path::new("app"))
        );
        for command in ["approve", "execute"] {
            assert!(parse(&[command, "--multisig", "M", "--index", "1"]).is_err());
            assert!(parse(&[command, "--multisig", "M", "--wallet", "w"]).is_err());
        }
        // Only the dev environment: there is no RPC option.
        assert!(parse(&[
            "approve",
            "--multisig",
            "M",
            "--index",
            "1",
            "--wallet",
            "w",
            "--rpc-url",
            "https://api.mainnet-beta.solana.com"
        ])
        .is_err());
    }

    #[test]
    fn permissions_are_named() {
        assert_eq!(mask_names(7), "initiate, vote, execute");
        assert_eq!(mask_names(1), "initiate");
        assert_eq!(mask_names(0), "none");
    }
}
