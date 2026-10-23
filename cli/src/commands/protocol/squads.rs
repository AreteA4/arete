//! `a4 protocol squads`: bring your own Squads v4 multisig.
//!
//! A step whose authority is `squadsProposer` is proposed to the team's
//! Squads v4 multisig by a proposer key Arete holds for the account (one key
//! per account, shared by its deployments). The proposer may only initiate:
//! the team adds it to the multisig as a member with exactly the Initiate
//! permission (mask 1) and funds it with a little SOL, since it pays the fee
//! and rent of every proposal; the members still approve and execute.
//!
//! - `proposer` prints the proposer (and its balance when an RPC answers);
//! - `status` says whether it can propose to a multisig yet;
//! - `add-proposer` fetches the unsigned transaction that creates a config
//!   proposal adding it, for an existing member with Initiate to sign and
//!   send, and with `--keypair` (or `--send` on a dev environment) signs and
//!   sends it.
//!
//! The deployment is named as `a4 protocol actions` names it, through the
//! same protocol tools the MCP server's `protocol_squads_*` tools use.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use arete_mcp::dev::{DevClient, DevProject};
use arete_mcp::protocol_agent::{ProtocolAgent, SQUADS_PROPOSER_LOW_LAMPORTS};
use base64::Engine as _;
use clap::{Args, Subcommand};
use colored::Colorize;
use ed25519_dalek::SigningKey;
use serde_json::{json, Value};

use crate::squads::tx::Chain;

#[derive(Subcommand, Debug)]
pub enum SquadsCommands {
    /// Print the account's Squads proposer for a deployment, and its SOL
    /// balance when an RPC is available
    Proposer(ProposerArgs),
    /// Whether the proposer is an Initiate-only member of a multisig, and
    /// what to do when it is not
    Status(StatusArgs),
    /// Fetch the unsigned transaction that proposes adding the proposer to
    /// a multisig (Initiate only); `--keypair` or `--send` signs and sends it
    AddProposer(AddProposerArgs),
}

/// The deployment, named as `a4 protocol actions` names it.
#[derive(Args, Debug, Clone, Default)]
pub struct SquadsTarget {
    /// Protocol package reference (e.g. `token-raise`) or deployment id;
    /// optional with `--dev`, which defaults to the session's deployment
    pub target: Option<String>,

    /// Ask the local `a4 dev` session instead of Arete
    #[arg(long)]
    pub dev: bool,

    /// The `a4 dev` project directory (implies `--dev`)
    #[arg(long, value_name = "DIR")]
    pub project: Option<String>,
}

impl SquadsTarget {
    fn is_dev(&self) -> bool {
        self.dev || self.project.is_some()
    }

    fn tool_target(&self) -> Result<arete_mcp::protocol_agent::TargetArgs> {
        super::protocol_target(self.target.as_deref(), self.dev, self.project.as_deref())
    }

    /// The target as command-line words, for the next command to run.
    fn words(&self) -> String {
        let mut words = Vec::new();
        if let Some(target) = self.target.as_deref().filter(|t| !t.trim().is_empty()) {
            words.push(target.trim().to_string());
        }
        if let Some(project) = &self.project {
            words.push(format!("--project {project}"));
        } else if self.dev {
            words.push("--dev".into());
        }
        words.join(" ")
    }

    fn dev_client(&self) -> Result<DevClient> {
        DevProject::discover(self.project.as_deref().map(Path::new)).and_then(DevClient::open)
    }
}

#[derive(Args, Debug, Clone, Default)]
pub struct ProposerArgs {
    #[command(flatten)]
    pub target: SquadsTarget,

    /// RPC URL to read the proposer's balance from (default
    /// ARETE_MCP_RPC_URL; with `--dev`, the dev environment)
    #[arg(long, value_name = "URL")]
    pub rpc_url: Option<String>,
}

#[derive(Args, Debug, Clone, Default)]
pub struct StatusArgs {
    #[command(flatten)]
    pub target: SquadsTarget,

    /// The team's Squads v4 multisig account (not one of its vaults)
    #[arg(long, value_name = "ADDRESS")]
    pub multisig: String,

    /// RPC URL to read the proposer's balance from (default
    /// ARETE_MCP_RPC_URL; with `--dev`, the dev environment)
    #[arg(long, value_name = "URL")]
    pub rpc_url: Option<String>,
}

#[derive(Args, Debug, Clone, Default)]
pub struct AddProposerArgs {
    #[command(flatten)]
    pub target: SquadsTarget,

    /// The team's Squads v4 multisig account (not one of its vaults)
    #[arg(long, value_name = "ADDRESS")]
    pub multisig: String,

    /// A multisig member with the Initiate permission, who signs, sends and
    /// pays (with `--dev`, also a dev wallet name)
    #[arg(long, value_name = "PUBKEY")]
    pub member: String,

    /// Sign with this Solana keypair file (it must be `--member`) and send
    /// the transaction through `--rpc-url` (with `--dev`, the dev
    /// environment)
    #[arg(long, value_name = "FILE", conflicts_with = "send")]
    pub keypair: Option<PathBuf>,

    /// With `--dev`: sign with the dev wallet `--member` names and send
    /// through the dev environment
    #[arg(long)]
    pub send: bool,

    /// The RPC that `--keypair` sends through (not with `--dev`)
    #[arg(long, value_name = "URL")]
    pub rpc_url: Option<String>,

    /// Transaction format: `v0` or `v1` (default: the server's)
    #[arg(long, value_name = "FORMAT", value_parser = ["v0", "v1"])]
    pub transaction_format: Option<String>,
}

pub fn run(command: SquadsCommands, json: bool) -> Result<()> {
    let agent = ProtocolAgent::new(None);
    match command {
        SquadsCommands::Proposer(args) => {
            let result = block_on(fetch_proposer(&agent, &args))?;
            output(json, &result, render_proposer)
        }
        SquadsCommands::Status(args) => {
            let result = block_on(fetch_status(&agent, &args))?;
            output(json, &result, render_status)
        }
        SquadsCommands::AddProposer(args) => {
            let result = block_on(add_proposer(&agent, &args))?;
            output(json, &result, render_add_proposer)
        }
    }
}

fn output(json: bool, result: &Value, render: fn(&Value) -> String) -> Result<()> {
    if json {
        println!("{result}");
    } else {
        print!("{}", render(result));
    }
    Ok(())
}

fn block_on<T>(future: impl std::future::Future<Output = Result<T>>) -> Result<T> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("Failed to start the async runtime")?
        .block_on(future)
}

/// `--rpc-url` for a balance read: refused with `--dev`, whose environment
/// answers instead.
fn balance_rpc(target: &SquadsTarget, rpc_url: Option<&str>) -> Result<Option<String>> {
    match rpc_url.map(str::trim).filter(|url| !url.is_empty()) {
        Some(_) if target.is_dev() => {
            bail!("--rpc-url cannot be combined with --dev: the dev environment's RPC is used")
        }
        Some(url) => Ok(Some(url.to_string())),
        None => Ok(None),
    }
}

/// `a4 protocol squads proposer`.
pub async fn fetch_proposer(agent: &ProtocolAgent, args: &ProposerArgs) -> Result<Value> {
    let rpc = balance_rpc(&args.target, args.rpc_url.as_deref())?;
    let mut result = agent
        .squads_proposer(&args.target.tool_target()?, rpc.as_deref())
        .await?;
    result["next"] = json!(format!(
        "Add {} to your Squads v4 multisig as a member with only the Initiate permission (mask 1), and fund it with a little SOL: it pays the fee and rent of every proposal. `{}` proposes adding it.",
        result["pubkey"].as_str().unwrap_or("the proposer"),
        add_proposer_command(&args.target, None)
    ));
    Ok(result)
}

/// `a4 protocol squads status`.
pub async fn fetch_status(agent: &ProtocolAgent, args: &StatusArgs) -> Result<Value> {
    let rpc = balance_rpc(&args.target, args.rpc_url.as_deref())?;
    let mut result = agent
        .squads_status(&args.target.tool_target()?, &args.multisig, rpc.as_deref())
        .await?;
    result["next"] = json!(status_next(&args.target, &result));
    Ok(result)
}

fn add_proposer_command(target: &SquadsTarget, multisig: Option<&str>) -> String {
    let words = target.words();
    format!(
        "a4 protocol squads add-proposer{}{} --multisig {} --member <member with Initiate>",
        if words.is_empty() { "" } else { " " },
        words,
        multisig.unwrap_or("<multisig>")
    )
}

/// What to do after a status answer, as commands.
pub fn status_next(target: &SquadsTarget, status: &Value) -> String {
    let multisig = status["multisig"].as_str();
    if status["ready"] == true {
        return "Ready: the proposer can propose to this multisig. Keep it funded; it pays the fee and rent of every proposal.".into();
    }
    match status["problem"].as_str() {
        Some("proposer_unfunded") => {
            let proposer = status["proposer"].as_str().unwrap_or("<proposer>");
            let needs = status["proposerMinLamports"]
                .as_u64()
                .map(|minimum| format!(" (one proposal needs {})", sol(minimum)))
                .unwrap_or_default();
            if target.is_dev() {
                format!(
                    "The proposer is a member but cannot pay for a proposal{needs}. Fund it with `a4 dev fund {proposer}`."
                )
            } else {
                format!(
                    "The proposer is a member but cannot pay for a proposal{needs}. Send SOL to {proposer}; it pays the fee and rent of every proposal."
                )
            }
        }
        Some("permissions_invalid") => format!(
            "The proposer is a member with permission mask {} instead of exactly Initiate (1). Remove it with a config proposal in Squads, then add it again with `{}`.",
            status["mask"],
            add_proposer_command(target, multisig)
        ),
        _ => format!(
            "Add it with `{}`; the member signs and sends that transaction, then your members approve and execute the config proposal in Squads.",
            add_proposer_command(target, multisig)
        ),
    }
}

/// Load a Solana CLI keypair file (a JSON array of 64 bytes).
fn load_keypair(path: &Path) -> Result<SigningKey> {
    let bytes: Vec<u8> = serde_json::from_slice(
        &std::fs::read(path).with_context(|| format!("Failed to read {}", path.display()))?,
    )
    .with_context(|| format!("{} is not a Solana keypair file", path.display()))?;
    if bytes.len() != 64 {
        bail!("{} must hold 64 bytes", path.display());
    }
    let key = SigningKey::from_bytes(&bytes[..32].try_into().expect("checked length"));
    if key.verifying_key().as_bytes() != &bytes[32..] {
        bail!(
            "{}: the public key does not match the secret key",
            path.display()
        );
    }
    Ok(key)
}

/// How `add-proposer` signs, decided before anything is fetched.
enum Signing {
    /// Print the transaction only.
    None,
    Keypair {
        key: Box<SigningKey>,
        rpc: Option<String>,
    },
    /// The dev wallet `--member` names.
    DevWallet,
}

fn signing(args: &AddProposerArgs) -> Result<Signing> {
    let dev = args.target.is_dev();
    let rpc = args
        .rpc_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty());
    if dev && rpc.is_some() {
        bail!("--rpc-url cannot be combined with --dev: the dev environment's RPC is used");
    }
    if args.send {
        if !dev {
            bail!("--send signs with a dev wallet and needs --dev; elsewhere pass --keypair and --rpc-url");
        }
        return Ok(Signing::DevWallet);
    }
    match &args.keypair {
        None if rpc.is_some() => bail!("--rpc-url is where --keypair sends; pass --keypair too"),
        None => Ok(Signing::None),
        Some(path) => {
            if !dev && rpc.is_none() {
                bail!("--keypair sends the transaction and needs --rpc-url");
            }
            Ok(Signing::Keypair {
                key: Box::new(load_keypair(path)?),
                rpc: rpc.map(str::to_string),
            })
        }
    }
}

/// `a4 protocol squads add-proposer`.
pub async fn add_proposer(agent: &ProtocolAgent, args: &AddProposerArgs) -> Result<Value> {
    let signing = signing(args)?;
    let mut result = agent
        .squads_add_proposer(
            &args.target.tool_target()?,
            &args.multisig,
            &args.member,
            args.transaction_format.as_deref(),
        )
        .await?;
    let member = result["member"].as_str().unwrap_or_default().to_string();
    if let Signing::Keypair { key, .. } = &signing {
        let signer = bs58::encode(key.verifying_key().as_bytes()).into_string();
        if signer != member {
            bail!("--keypair holds {signer}, but --member is {member}: the member signs");
        }
    }
    let transaction = base64::engine::general_purpose::STANDARD
        .decode(result["transactionBase64"].as_str().unwrap_or_default())
        .context("The add-proposer transaction is not base64")?;
    let dev = if args.target.is_dev() && !matches!(signing, Signing::None) {
        Some(args.target.dev_client()?)
    } else {
        None
    };
    let signed = match &signing {
        Signing::None => None,
        Signing::Keypair { key, .. } => Some(arete_mcp::dev::sign_transaction(&transaction, key)?),
        Signing::DevWallet => {
            let client = dev.as_ref().expect("--send needs --dev");
            let path = client
                .project
                .wallets_dir()
                .join(format!("{}.json", args.member.trim()));
            if !path.is_file() {
                bail!(
                    "--send signs with the dev wallet --member names, and '{}' is not a dev wallet",
                    args.member
                );
            }
            Some(
                client
                    .wallet(args.member.trim())
                    .await?
                    .sign_transaction(&transaction)?,
            )
        }
    };
    let index = result["transactionIndex"].clone();
    let multisig = result["multisig"].as_str().unwrap_or_default().to_string();
    if let Some(signed) = signed {
        let chain = match (&signing, &dev) {
            (_, Some(client)) => Chain::Dev(client),
            (Signing::Keypair { rpc: Some(url), .. }, None) => Chain::url(url),
            _ => unreachable!("signing checked the RPC"),
        };
        let signature = chain.send_and_confirm(&signed).await?;
        result["sent"] = json!(true);
        result["signature"] = json!(signature);
        result["next"] = json!(if dev.is_some() {
            let wallet = args.member.trim();
            format!(
                "Config proposal {index} is open. Approve it with enough members (`a4 dev squads approve --multisig {multisig} --index {index} --wallet {wallet}`), then execute it (`a4 dev squads execute --multisig {multisig} --index {index} --wallet {wallet}`)."
            )
        } else {
            format!(
                "Config proposal {index} is open. Your members approve and execute it in Squads; then `a4 protocol squads status{} --multisig {multisig}` reports ready.",
                prefixed(&args.target.words())
            )
        });
    } else {
        result["sent"] = json!(false);
        result["next"] = json!(format!(
            "{member} signs and sends this transaction (it pays the fee and the proposal's rent). Then your members approve and execute config proposal {index} in Squads, and `a4 protocol squads status{} --multisig {multisig}` reports ready.",
            prefixed(&args.target.words())
        ));
    }
    Ok(result)
}

fn prefixed(words: &str) -> String {
    if words.is_empty() {
        String::new()
    } else {
        format!(" {words}")
    }
}

fn sol(lamports: u64) -> String {
    let text = format!("{:.9}", lamports as f64 / 1e9);
    let text = text.trim_end_matches('0').trim_end_matches('.');
    format!("{text} SOL")
}

fn render_balance(result: &Value) -> String {
    let minimum = result["proposerMinLamports"].as_u64();
    let needs = minimum
        .map(|minimum| format!(" (one proposal needs {})", sol(minimum)))
        .unwrap_or_default();
    match result["proposerLamports"].as_u64() {
        Some(lamports) if lamports < minimum.unwrap_or(SQUADS_PROPOSER_LOW_LAMPORTS) => format!(
            "{}{} {}\n",
            sol(lamports).yellow(),
            needs.yellow(),
            "low: fund it, or proposals fail".yellow()
        ),
        Some(lamports) => format!("{}{needs}\n", sol(lamports)),
        None => format!(
            "unavailable ({})\n",
            result["balanceNote"]
                .as_str()
                .unwrap_or("no RPC")
                .trim_start_matches("The balance was not read: ")
        ),
    }
}

/// The human rendering of `proposer`.
pub fn render_proposer(result: &Value) -> String {
    let mut out = format!(
        "Squads proposer for deployment {}\n",
        result["deploymentId"].as_str().unwrap_or("?")
    );
    out.push_str(&format!(
        "  pubkey       {}\n",
        result["pubkey"].as_str().unwrap_or("?").bold()
    ));
    out.push_str("  permissions  Initiate only (mask 1)\n");
    out.push_str(&format!("  balance      {}", render_balance(result)));
    out.push_str(&format!(
        "\n{}\n",
        result["next"].as_str().unwrap_or_default()
    ));
    out
}

/// The human rendering of `status`.
pub fn render_status(result: &Value) -> String {
    let yes_no = |value: &Value| {
        if *value == true {
            "yes".green().to_string()
        } else {
            "no".red().to_string()
        }
    };
    let mut out = format!(
        "Squads proposer {} on multisig {}\n",
        result["proposer"].as_str().unwrap_or("?"),
        result["multisig"].as_str().unwrap_or("?")
    );
    out.push_str(&format!("  ready      {}\n", yes_no(&result["ready"])));
    out.push_str(&format!(
        "  member     {}{}\n",
        yes_no(&result["member"]),
        result["mask"]
            .as_u64()
            .map(|mask| format!(" (mask {mask})"))
            .unwrap_or_default()
    ));
    if let Some(problem) = result["problem"].as_str() {
        out.push_str(&format!("  problem    {problem}\n"));
    }
    out.push_str(&format!("  threshold  {}\n", result["threshold"]));
    out.push_str(&format!("  balance    {}", render_balance(result)));
    out.push_str(&format!(
        "\n{}\n",
        result["next"].as_str().unwrap_or_default()
    ));
    out
}

/// The human rendering of `add-proposer`.
pub fn render_add_proposer(result: &Value) -> String {
    let mut out = format!(
        "Add-proposer proposal for multisig {}\n",
        result["multisig"].as_str().unwrap_or("?")
    );
    for (label, value) in [
        (
            "proposer",
            format!(
                "{} (Initiate only)",
                result["proposer"].as_str().unwrap_or("?")
            ),
        ),
        (
            "member",
            format!(
                "{} signs, sends and pays",
                result["member"].as_str().unwrap_or("?")
            ),
        ),
        (
            "proposal",
            format!(
                "{} (transaction {})",
                result["proposal"].as_str().unwrap_or("?"),
                result["transactionIndex"]
            ),
        ),
        (
            "config tx",
            result["configTransaction"]
                .as_str()
                .unwrap_or("?")
                .to_string(),
        ),
    ] {
        out.push_str(&format!("  {label:<11}{value}\n"));
    }
    if let Some(signature) = result["signature"].as_str() {
        out.push_str(&format!("  {:<11}{}\n", "sent", signature.green()));
    } else {
        out.push_str(&format!(
            "\nUnsigned transaction ({}, base64):\n{}\n",
            result["transactionFormat"].as_str().unwrap_or("?"),
            result["transactionBase64"].as_str().unwrap_or_default()
        ));
    }
    out.push_str(&format!(
        "\n{}\n",
        result["next"].as_str().unwrap_or_default()
    ));
    out
}

#[cfg(test)]
mod tests;
