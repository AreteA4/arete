//! `a4 squads`: the Squads v4 multisigs your Arete account has attached.
//!
//! A protocol's admin steps are proposed to a multisig named in its
//! deployment config, and a production deployment is refused unless that
//! multisig is an `active` attachment of your account: Arete's proposer is
//! a member with the Initiate permission only. The dashboard page
//! (<https://app.arete.run/dashboard/squads>) sets this up with any browser
//! wallet, hardware wallets included, and the Squads app's own "add member"
//! flow works too. These commands print that link first; the unsigned
//! transaction they print after it is for hot wallets, and `--keypair` signs
//! it locally and sends it through Arete.
//!
//! Dev environments keep their own proposer: use `a4 dev squads` there.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use arete_mcp::protocol_agent::ProtocolAgent;
use base64::Engine as _;
use clap::{Args, Subcommand};
use colored::Colorize;
use serde_json::{json, Value};

const DASHBOARD_URL: &str = "https://app.arete.run/dashboard/squads";

#[derive(Subcommand, Debug)]
pub enum SquadsCommands {
    /// The account's Squads setup, or one attached multisig in detail
    Status(StatusArgs),
    /// The account's attached multisigs and their state
    List,
    /// Attach an existing Squads v4 multisig by address
    Attach(AttachArgs),
    /// Create a Squads v4 multisig with Arete's proposer as an Initiate-only
    /// member (prints the unsigned transaction; `--keypair` signs and sends)
    Create(CreateArgs),
    /// Propose adding Arete's proposer (Initiate only) to an attached
    /// multisig (prints the unsigned transaction; `--keypair` signs and sends)
    AddProposer(AddProposerArgs),
    /// Approve a config proposal Arete built (hot-wallet members)
    Approve(ProposalArgs),
    /// Execute an approved config proposal Arete built (hot-wallet members)
    Execute(ProposalArgs),
}

#[derive(Args, Debug, Clone, Default)]
pub struct StatusArgs {
    /// An attached multisig to show in detail (default: the whole account)
    pub multisig: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct AttachArgs {
    /// The Squads v4 multisig account (not one of its vaults)
    pub multisig: String,

    /// A label for the dashboard (1 to 64 characters)
    #[arg(long)]
    pub label: Option<String>,
}

/// How a prepared transaction is signed.
#[derive(Args, Debug, Clone, Default)]
pub struct SigningArgs {
    /// Sign with this Solana keypair file (a hot wallet; it must be the
    /// transaction's signer) and send it through Arete
    #[arg(long, value_name = "FILE")]
    pub keypair: Option<PathBuf>,

    /// Transaction format: `v0` (default) or `v1`
    #[arg(long, value_name = "FORMAT", value_parser = ["v0", "v1"])]
    pub transaction_format: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct CreateArgs {
    /// A member as `PUBKEY` (every permission) or
    /// `PUBKEY:initiate+vote+execute`; repeat for each member. Do not list
    /// Arete's proposer: it is added with Initiate only
    #[arg(long = "member", required = true, value_name = "PUBKEY[:PERMS]")]
    pub members: Vec<String>,

    /// Approvals a proposal needs (1 to the number of voting members)
    #[arg(long)]
    pub threshold: u16,

    /// The member who signs and pays (creation fee and rent)
    #[arg(long, value_name = "PUBKEY")]
    pub creator: String,

    /// A label for the dashboard (1 to 64 characters)
    #[arg(long)]
    pub label: Option<String>,

    #[command(flatten)]
    pub signing: SigningArgs,
}

#[derive(Args, Debug, Clone)]
pub struct AddProposerArgs {
    /// The attached Squads v4 multisig
    pub multisig: String,

    /// A multisig member with the Initiate permission who signs and pays
    #[arg(long, value_name = "PUBKEY")]
    pub member: String,

    #[command(flatten)]
    pub signing: SigningArgs,
}

#[derive(Args, Debug, Clone)]
pub struct ProposalArgs {
    /// The attached Squads v4 multisig
    pub multisig: String,

    /// The config proposal's transaction index (`a4 squads status <multisig>`)
    #[arg(long)]
    pub index: u64,

    /// The member who signs (it must be able to vote, or to execute)
    #[arg(long, value_name = "PUBKEY")]
    pub member: String,

    #[command(flatten)]
    pub signing: SigningArgs,
}

pub fn run(command: SquadsCommands, json: bool) -> Result<()> {
    let agent = ProtocolAgent::new(None);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("Failed to start the async runtime")?;
    let (result, render): (Value, fn(&Value) -> String) = match command {
        SquadsCommands::Status(args) => match args.multisig.as_deref() {
            None => (
                runtime.block_on(agent.account_squads_status(None))?,
                render_status,
            ),
            Some(multisig) => (
                runtime.block_on(agent.account_squads_status(Some(multisig)))?,
                render_detail,
            ),
        },
        SquadsCommands::List => (
            runtime.block_on(agent.account_squads_status(None))?,
            render_list,
        ),
        SquadsCommands::Attach(args) => (
            runtime.block_on(agent.account_squads_attach(&args.multisig, args.label.as_deref()))?,
            render_attach,
        ),
        SquadsCommands::Create(args) => {
            let members = args
                .members
                .iter()
                .map(|member| parse_member(member))
                .collect::<Result<Vec<_>>>()?;
            let key = signing_key(&args.signing)?;
            let prepared = runtime.block_on(agent.account_squads_create(
                members,
                args.threshold,
                &args.creator,
                args.label.as_deref(),
                args.signing.transaction_format.as_deref(),
            ))?;
            (
                runtime.block_on(sign_and_send(&agent, prepared, key.as_ref()))?,
                render_prepared,
            )
        }
        SquadsCommands::AddProposer(args) => {
            let key = signing_key(&args.signing)?;
            let prepared = runtime.block_on(agent.account_squads_add_proposer(
                &args.multisig,
                &args.member,
                args.signing.transaction_format.as_deref(),
            ))?;
            (
                runtime.block_on(sign_and_send(&agent, prepared, key.as_ref()))?,
                render_prepared,
            )
        }
        SquadsCommands::Approve(args) => {
            let key = signing_key(&args.signing)?;
            let prepared = runtime.block_on(agent.account_squads_approve(
                &args.multisig,
                args.index,
                &args.member,
                args.signing.transaction_format.as_deref(),
            ))?;
            (
                runtime.block_on(sign_and_send(&agent, prepared, key.as_ref()))?,
                render_prepared,
            )
        }
        SquadsCommands::Execute(args) => {
            let key = signing_key(&args.signing)?;
            let prepared = runtime.block_on(agent.account_squads_execute(
                &args.multisig,
                args.index,
                &args.member,
                args.signing.transaction_format.as_deref(),
            ))?;
            (
                runtime.block_on(sign_and_send(&agent, prepared, key.as_ref()))?,
                render_prepared,
            )
        }
    };
    if json {
        println!("{result}");
    } else {
        print!("{}", render(&result));
    }
    Ok(())
}

/// `PUBKEY` (every permission) or `PUBKEY:initiate+vote+execute`.
pub fn parse_member(value: &str) -> Result<(String, Vec<String>)> {
    let (pubkey, permissions) = match value.trim().split_once(':') {
        Some((pubkey, permissions)) => (pubkey.trim(), permissions.trim()),
        None => (value.trim(), "initiate+vote+execute"),
    };
    if pubkey.is_empty() {
        bail!("--member '{value}' names no public key");
    }
    let permissions: Vec<String> = permissions
        .split('+')
        .map(|permission| permission.trim().to_ascii_lowercase())
        .filter(|permission| !permission.is_empty())
        .collect();
    if permissions.is_empty() {
        bail!("--member '{value}' gives no permissions");
    }
    for permission in &permissions {
        if !matches!(permission.as_str(), "initiate" | "vote" | "execute") {
            bail!(
                "--member '{value}': permission `{permission}` must be initiate, vote or execute"
            );
        }
    }
    Ok((pubkey.to_string(), permissions))
}

fn signing_key(args: &SigningArgs) -> Result<Option<ed25519_dalek::SigningKey>> {
    args.keypair
        .as_deref()
        .map(super::protocol::squads::load_keypair)
        .transpose()
}

/// With a keypair, sign the prepared transaction locally (only the
/// keypair's own signature slot) and send it through Arete.
async fn sign_and_send(
    agent: &ProtocolAgent,
    mut prepared: Value,
    key: Option<&ed25519_dalek::SigningKey>,
) -> Result<Value> {
    let Some(key) = key else {
        prepared["sent"] = json!(false);
        return Ok(prepared);
    };
    let holder = bs58::encode(key.verifying_key().as_bytes()).into_string();
    let signer = prepared["signer"].as_str().unwrap_or_default().to_string();
    if holder != signer {
        bail!("--keypair holds {holder}, but this transaction is signed by {signer}");
    }
    let transaction = base64::engine::general_purpose::STANDARD
        .decode(prepared["transactionBase64"].as_str().unwrap_or_default())
        .context("The prepared transaction is not base64")?;
    let signed = arete_mcp::dev::sign_transaction(&transaction, key)?;
    let transaction_id = prepared["transactionId"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let mut sent = agent
        .account_squads_submit(&transaction_id, &signed)
        .await?;
    // A pending answer is re-checked by submitting the same body again.
    for _ in 0..3 {
        if sent["status"] != "pending" {
            break;
        }
        sent = agent
            .account_squads_submit(&transaction_id, &signed)
            .await?;
    }
    prepared["sent"] = json!(true);
    prepared["submit"] = sent.clone();
    if let Some(next) = sent["next"].as_str() {
        prepared["next"] = json!(format!(
            "{next} {}",
            prepared["next"].as_str().unwrap_or_default()
        ));
    }
    Ok(prepared)
}

fn dashboard_line(url: &str) -> String {
    format!(
        "{} {}\n  any browser wallet, Ledger included\n\n",
        "Dashboard:".bold(),
        url.cyan()
    )
}

fn state_label(state: &str) -> String {
    match state {
        "active" => state.green().to_string(),
        "archived" => state.red().to_string(),
        other => other.yellow().to_string(),
    }
}

fn str_or<'a>(value: &'a Value, fallback: &'a str) -> &'a str {
    value.as_str().unwrap_or(fallback)
}

/// The human rendering of `status` (whole account).
pub fn render_status(result: &Value) -> String {
    let mut out = dashboard_line(str_or(&result["dashboardUrl"], DASHBOARD_URL));
    out.push_str(&format!(
        "Squads setup   {}\n",
        str_or(&result["setup"], "?").bold()
    ));
    out.push_str(&format!(
        "  proposer     {} (Initiate only)\n",
        str_or(&result["proposer"]["pubkey"], "?")
    ));
    if let Some(lamports) = result["proposer"]["lamports"].as_u64() {
        out.push_str(&format!(
            "  balance      {} SOL (information only)\n",
            lamports as f64 / 1e9
        ));
    }
    out.push_str(&list_rows(result));
    for notification in result["notifications"].as_array().into_iter().flatten() {
        out.push_str(&format!(
            "  {} {} since {}: {} ({})\n",
            "archived".red(),
            str_or(&notification["multisig"], "?"),
            str_or(&notification["since"], "?"),
            str_or(
                &notification["problemDescription"],
                str_or(&notification["problem"], "the proposer is gone")
            ),
            str_or(&notification["dashboardUrl"], DASHBOARD_URL)
        ));
    }
    out.push_str(&format!("\n{}\n", str_or(&result["next"], "")));
    out
}

fn list_rows(result: &Value) -> String {
    let multisigs = result["multisigs"].as_array().cloned().unwrap_or_default();
    if multisigs.is_empty() {
        return "  multisigs    none attached\n".into();
    }
    let mut out = String::from("  multisigs\n");
    for multisig in &multisigs {
        out.push_str(&format!(
            "    {}  {}{}{}\n",
            str_or(&multisig["address"], "?"),
            state_label(str_or(&multisig["state"], "?")),
            multisig["label"]
                .as_str()
                .map(|label| format!("  {label}"))
                .unwrap_or_default(),
            multisig["problem"]
                .as_str()
                .map(|problem| format!("  ({problem})"))
                .unwrap_or_default()
        ));
    }
    out
}

/// The human rendering of `list`.
pub fn render_list(result: &Value) -> String {
    let mut out = dashboard_line(str_or(&result["dashboardUrl"], DASHBOARD_URL));
    out.push_str(&list_rows(result));
    out
}

fn attachment_lines(result: &Value) -> String {
    let mut out = format!(
        "Multisig {}  {}\n",
        str_or(&result["address"], "?").bold(),
        state_label(str_or(&result["state"], "?"))
    );
    if let Some(label) = result["label"].as_str() {
        out.push_str(&format!("  label        {label}\n"));
    }
    if let Some(problem) = result["problem"].as_str() {
        out.push_str(&format!(
            "  problem      {}\n",
            result["problemDescription"].as_str().unwrap_or(problem)
        ));
    }
    if let Some(threshold) = result["threshold"].as_u64() {
        out.push_str(&format!("  threshold    {threshold}\n"));
    }
    for member in result["members"].as_array().into_iter().flatten() {
        let permissions: Vec<&str> = member["permissions"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        out.push_str(&format!(
            "  member       {} ({})\n",
            str_or(&member["pubkey"], "?"),
            permissions.join(", ")
        ));
    }
    if let Some(url) = result["squadsUrl"].as_str() {
        out.push_str(&format!("  squads app   {url}\n"));
    }
    if let Some(snippet) = result["deploymentConfigSnippet"]
        .as_str()
        .filter(|_| result["state"] == "active")
    {
        out.push_str(&format!("\nDeployment config:\n{snippet}\n"));
    }
    out
}

fn squads_app_lines(result: &Value) -> String {
    let steps: Vec<&str> = result["squadsAppSteps"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    if steps.is_empty() {
        return String::new();
    }
    let mut out = format!("{}\n", "Or in the Squads app:".bold());
    for (index, step) in steps.iter().enumerate() {
        out.push_str(&format!("  {}. {step}\n", index + 1));
    }
    out.push('\n');
    out
}

/// The human rendering of `status <multisig>`.
pub fn render_detail(result: &Value) -> String {
    let mut out = dashboard_line(str_or(&result["dashboardUrl"], DASHBOARD_URL));
    out.push_str(&attachment_lines(result));
    for deployment in result["deployments"].as_array().into_iter().flatten() {
        let steps: Vec<&str> = deployment["steps"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        out.push_str(&format!(
            "  deployment   {} {} ({}; steps {})\n",
            str_or(
                &deployment["protocolName"],
                str_or(&deployment["protocolSlug"], "")
            ),
            str_or(&deployment["id"], "?"),
            str_or(&deployment["status"], "?"),
            if steps.is_empty() {
                "-".to_string()
            } else {
                steps.join(", ")
            }
        ));
    }
    for proposal in result["configProposals"].as_array().into_iter().flatten() {
        out.push_str(&format!(
            "  config #{}   {} {}\n",
            proposal["transactionIndex"],
            str_or(&proposal["kind"], "?"),
            str_or(&proposal["status"], "?")
        ));
    }
    for proposal in result["proposals"].as_array().into_iter().flatten() {
        out.push_str(&format!(
            "  proposal #{} {} {} ({})\n",
            proposal["transactionIndex"],
            str_or(&proposal["step"], "?"),
            str_or(&proposal["status"], "?"),
            str_or(&proposal["squadsUrl"], "")
        ));
    }
    out.push_str(&format!("\n{}\n", str_or(&result["next"], "")));
    out
}

/// The human rendering of `attach`.
pub fn render_attach(result: &Value) -> String {
    let mut out = dashboard_line(str_or(&result["dashboardUrl"], DASHBOARD_URL));
    out.push_str(&attachment_lines(result));
    out.push('\n');
    out.push_str(&squads_app_lines(result));
    out.push_str(&format!("{}\n", str_or(&result["next"], "")));
    out
}

/// The human rendering of a prepared (and maybe sent) transaction.
pub fn render_prepared(result: &Value) -> String {
    let dashboard = result["dashboardUrl"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| match result["multisig"].as_str() {
            Some(multisig) => format!("{DASHBOARD_URL}/{multisig}"),
            None => DASHBOARD_URL.to_string(),
        });
    let mut out = dashboard_line(&dashboard);
    out.push_str(&squads_app_lines(result));
    out.push_str(&format!(
        "{} for multisig {}\n",
        str_or(&result["kind"], "Transaction").bold(),
        str_or(&result["multisig"], "?")
    ));
    out.push_str(&format!(
        "  signer       {} (signs and pays)\n",
        str_or(&result["signer"], "?")
    ));
    if let Some(index) = result["transactionIndex"].as_u64() {
        out.push_str(&format!("  index        {index}\n"));
    }
    if let Some(proposal) = result["proposal"].as_str() {
        out.push_str(&format!("  proposal     {proposal}\n"));
    }
    if result["sent"] == true {
        let submit = &result["submit"];
        let status = str_or(&submit["status"], "?");
        let status = match status {
            "confirmed" => status.green().to_string(),
            "pending" => status.yellow().to_string(),
            _ => status.red().to_string(),
        };
        out.push_str(&format!(
            "  sent         {status} {}\n",
            str_or(&submit["signature"], "")
        ));
        if let Some(error) = submit.get("error").filter(|error| !error.is_null()) {
            out.push_str(&format!("  error        {error}\n"));
        }
    } else {
        out.push_str(&format!(
            "\nFor a hot wallet only. Unsigned transaction ({}, base64; transactionId {}):\n{}\n",
            str_or(&result["transactionFormat"], "?"),
            str_or(&result["transactionId"], "?"),
            str_or(&result["transactionBase64"], "")
        ));
        out.push_str(
            "Sign it with the signer and pass --keypair <file> to send it through Arete.\n",
        );
    }
    out.push_str(&format!("\n{}\n", str_or(&result["next"], "")));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn members_default_to_every_permission() {
        assert_eq!(
            parse_member("Abc").unwrap(),
            (
                "Abc".to_string(),
                vec!["initiate".into(), "vote".into(), "execute".into()]
            )
        );
        assert_eq!(
            parse_member("Abc:vote+Execute").unwrap().1,
            vec!["vote".to_string(), "execute".to_string()]
        );
        assert!(parse_member("Abc:admin").is_err());
        assert!(parse_member(":vote").is_err());
    }

    #[test]
    fn prepared_output_leads_with_the_dashboard_then_the_squads_app() {
        colored::control::set_override(false);
        let result = json!({
            "kind": "addProposer", "multisig": "Ms1", "signer": "Member1",
            "transactionIndex": 3, "proposal": "P1", "transactionId": "tx-1",
            "transactionFormat": "v0", "transactionBase64": "AQID", "sent": false,
            "dashboardUrl": "https://app.arete.run/dashboard/squads/Ms1",
            "squadsAppSteps": ["Members → Add member.", "Permission: Initiate only."],
            "next": "Approve and execute it."
        });
        let text = render_prepared(&result);
        let dashboard = text
            .find("https://app.arete.run/dashboard/squads/Ms1")
            .unwrap();
        let app = text.find("Members → Add member").unwrap();
        let raw = text.find("AQID").unwrap();
        assert!(dashboard < app && app < raw, "{text}");
        assert!(text.starts_with("Dashboard:"));
    }

    #[test]
    fn status_lists_attachments_after_the_dashboard() {
        colored::control::set_override(false);
        let result = json!({
            "setup": "needsAttention",
            "proposer": { "pubkey": "Prop1", "permissions": "initiate", "lamports": null },
            "dashboardUrl": DASHBOARD_URL,
            "multisigs": [{ "address": "Ms1", "state": "pending", "problem": "not_member", "label": null }],
            "next": "Add the proposer."
        });
        let text = render_status(&result);
        assert!(
            text.starts_with(&format!("Dashboard: {DASHBOARD_URL}")),
            "{text}"
        );
        assert!(text.contains("Ms1  pending  (not_member)"), "{text}");
    }
}
