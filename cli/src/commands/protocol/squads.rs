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
//! - `status` says whether it can propose to a multisig yet.
//!
//! Arete builds no Squads transactions. Production multisigs are set up on
//! the dashboard or in the Squads app (`a4 squads`); on a dev environment,
//! `a4 dev squads add-proposer` builds the config proposal locally and a dev
//! wallet signs it.
//!
//! The deployment is named as `a4 protocol actions` names it, through the
//! same protocol tools the MCP server's `protocol_squads_*` tools use.

use anyhow::{bail, Context, Result};
use arete_mcp::protocol_agent::{ProtocolAgent, SQUADS_PROPOSER_LOW_LAMPORTS};
use clap::{Args, Subcommand};
use colored::Colorize;
use serde_json::{json, Value};

#[derive(Subcommand, Debug)]
pub enum SquadsCommands {
    /// Print the account's Squads proposer for a deployment, and its SOL
    /// balance when an RPC is available
    Proposer(ProposerArgs),
    /// Whether the proposer is an Initiate-only member of a multisig, and
    /// what to do when it is not
    Status(StatusArgs),
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
        // A malformed ARETE_ENVIRONMENT fails in `tool_target`.
        self.dev || self.project.is_some() || arete_mcp::dev::environment_is_dev().unwrap_or(false)
    }

    fn tool_target(&self) -> Result<arete_mcp::protocol_agent::TargetArgs> {
        super::protocol_target(self.target.as_deref(), self.dev, self.project.as_deref())
    }

    /// ` --project DIR` when a project was named, for follow-up `a4 dev`
    /// commands; empty otherwise.
    fn project_flag(&self) -> String {
        self.project
            .as_deref()
            .map(|project| format!(" --project {project}"))
            .unwrap_or_default()
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
    let proposer = result["pubkey"]
        .as_str()
        .unwrap_or("<proposer>")
        .to_string();
    result["next"] = json!(format!(
        "Add {proposer} to your Squads v4 multisig as a member with only the Initiate permission (mask 1), and fund it with a little SOL: it pays the fee and rent of every proposal. {}",
        add_proposer_hint(&args.target, None, &proposer)
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

/// How to add the proposer: on a dev environment the local
/// `a4 dev squads add-proposer`; otherwise the dashboard or the Squads app.
fn add_proposer_hint(target: &SquadsTarget, multisig: Option<&str>, proposer: &str) -> String {
    if target.is_dev() {
        format!(
            "`a4 dev squads add-proposer{} --multisig {} --proposer {proposer} --wallet <a dev wallet that can initiate>` proposes adding it; then approve and execute with `a4 dev squads approve` / `execute`.",
            target.project_flag(),
            multisig.unwrap_or("<multisig>")
        )
    } else {
        format!(
            "Set it up at {} (any browser wallet, Ledger included), or in the Squads app: Members → Add member → paste the proposer → permission Initiate only → approve and execute.",
            match multisig {
                Some(multisig) => format!("https://app.arete.run/dashboard/squads/{multisig}"),
                None => "https://app.arete.run/dashboard/squads".to_string(),
            }
        )
    }
}

/// What to do after a status answer, as commands.
pub fn status_next(target: &SquadsTarget, status: &Value) -> String {
    let multisig = status["multisig"].as_str();
    let proposer = status["proposer"].as_str().unwrap_or("<proposer>");
    if status["ready"] == true {
        return "Ready: the proposer can propose to this multisig. Keep it funded; it pays the fee and rent of every proposal.".into();
    }
    match status["problem"].as_str() {
        Some("proposer_unfunded") => {
            let needs = status["proposerMinLamports"]
                .as_u64()
                .map(|minimum| format!(" (one proposal needs {})", sol(minimum)))
                .unwrap_or_default();
            if target.is_dev() {
                format!(
                    "The proposer is a member but cannot pay for a proposal{needs}. Fund it with `a4 dev fund{} {proposer}`.",
                    target.project_flag()
                )
            } else {
                format!(
                    "The proposer is a member but cannot pay for a proposal{needs}. Send SOL to {proposer}; it pays the fee and rent of every proposal."
                )
            }
        }
        Some("permissions_invalid") => format!(
            "The proposer is a member with permission mask {} instead of exactly Initiate (1). Remove it with a config proposal in Squads, then add it again. {}",
            status["mask"],
            add_proposer_hint(target, multisig, proposer)
        ),
        _ => format!(
            "The proposer is not a member yet. {}",
            add_proposer_hint(target, multisig, proposer)
        ),
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

#[cfg(test)]
mod tests;
