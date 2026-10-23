//! `a4 dev squads`: Squads v4 multisigs on the project's cloud dev
//! environment, so a protocol whose steps are proposed to a multisig can be
//! tested end to end without the Squads app (which cannot see a fork).
//!
//! `create` makes a multisig whose members all hold every permission;
//! `approve` votes on a proposal (activating a draft first); `execute` runs
//! an approved config transaction (such as the add-proposer proposal
//! `a4 protocol squads add-proposer --dev` opens) and `vault-execute` an
//! approved vault transaction (such as a protocol step's proposal). Dev
//! wallets sign. These commands only ever talk to the project's dev
//! environment: there is no RPC option to point them anywhere else.

use anyhow::{anyhow, bail, Context, Result};
use arete_mcp::dev::{validate_wallet_name, DevClient, DevWallet};
use clap::{Args, Subcommand};
use colored::Colorize;
use ed25519_dalek::SigningKey;
use serde_json::{json, Value};

use super::ProjectArgs;
use crate::squads::tx::{compile_legacy, Chain};
use crate::squads::{self as v4, b58, parse_key, Instruction, Key, ProposalStatus};

/// Compute units for executing a vault transaction (the stored
/// instructions run inside it).
const VAULT_EXECUTE_UNITS: u32 = 1_400_000;

#[derive(Subcommand, Debug)]
pub enum DevSquadsCommands {
    /// Create a Squads v4 multisig whose members hold every permission
    /// (mask 7); prints the multisig and its vault 0
    Create(CreateArgs),
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
    /// The transaction index (`transactionIndex` of add-proposer, or the
    /// multisig's latest from `a4 dev squads show`)
    #[arg(long)]
    pub index: u64,
    /// The dev wallet (a multisig member) that signs
    #[arg(long)]
    pub wallet: String,
}

pub async fn run(command: DevSquadsCommands, json: bool) -> Result<()> {
    let (project, render): (&ProjectArgs, fn(&Value)) = match &command {
        DevSquadsCommands::Create(args) => (&args.project, render_create),
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

/// Compile `instructions` paid by `payer`, sign with it and `extra`, send
/// through the dev environment and wait for confirmation.
async fn send(
    chain: &Chain<'_>,
    payer: &DevWallet,
    instructions: &[Instruction],
    extra: &[&SigningKey],
) -> Result<String> {
    let blockhash = chain.latest_blockhash().await?;
    let mut transaction = compile_legacy(&payer.pubkey_bytes(), instructions, &blockhash)?;
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
    keyed.sort();
    let instruction = v4::multisig_create_v2(&v4::CreateMultisig {
        treasury: config.treasury,
        create_key: create_key_bytes,
        creator: payer.pubkey_bytes(),
        threshold: args.threshold,
        members: &keyed,
        time_lock: 0,
    });
    let signature = send(chain, &payer, &[instruction], &[&create_key]).await?;
    let multisig = v4::multisig_pda(&create_key_bytes);
    let vault = v4::vault_pda(&multisig, 0);
    Ok(json!({
        "multisig": b58(&multisig),
        "vault": b58(&vault),
        "createKey": b58(&create_key_bytes),
        "threshold": args.threshold,
        "members": members.iter().map(|(name, key)| json!({
            "pubkey": b58(key),
            "wallet": name,
            "mask": v4::PERMISSION_ALL,
        })).collect::<Vec<_>>(),
        "creator": payer.pubkey(),
        "creationFeeLamports": config.multisig_creation_fee,
        "signature": signature,
        "next": format!(
            "Use it as the protocol's Squads multisig. `a4 protocol squads add-proposer --dev --multisig {} --member {} --send` proposes adding the proposer.",
            b58(&multisig),
            payer_name
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
    let signature = send(chain, &wallet, &instructions, &[]).await?;
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
    let (instructions, details) = match (vault, is_vault, is_config) {
        (false, _, true) => {
            let config = v4::ConfigTransaction::decode(&data)?;
            let actions: Vec<String> = config.actions.iter().map(|a| a.describe()).collect();
            (
                vec![v4::config_transaction_execute(&multisig, &member, args.index)],
                json!({ "kind": "config", "actions": actions }),
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
    let signature = send(chain, &wallet, &instructions, &[]).await?;
    let mut result = json!({
        "multisig": args.multisig,
        "index": args.index,
        "transaction": b58(&transaction_key),
        "wallet": wallet.pubkey(),
        "signature": signature,
        "next": if vault {
            "Executed. Read the protocol's state again (`a4 dev instance <id>`).".to_string()
        } else {
            format!("Executed. `a4 protocol squads status --dev --multisig {}` checks the proposer.", args.multisig)
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
            "  member     {}{} (initiate, vote, execute)",
            member["pubkey"].as_str().unwrap_or_default(),
            member["wallet"]
                .as_str()
                .map(|name| format!(" [{name}]"))
                .unwrap_or_default()
        );
    }
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
