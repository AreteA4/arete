//! `a4 squads`: the Squads v4 multisigs your Arete account has attached.
//!
//! A protocol's admin steps are proposed to a multisig named in its
//! deployment config, and a production deployment is refused unless that
//! multisig is an `active` attachment of your account: Arete's proposer is
//! a member with the Initiate permission only. The dashboard page
//! (<https://app.arete.run/dashboard/squads>) sets this up with any browser
//! wallet, hardware wallets included, and the Squads app's own "add member"
//! flow works too. These commands read the setup and attach a multisig by
//! address; they print the dashboard link first and build no transactions.
//! Scripts and hot wallets use the squads-v4 program SDK's Arete setup
//! helpers (`a4 install program squads-v4`).
//!
//! Dev environments keep their own proposer: use `a4 dev squads` there.

use anyhow::{Context, Result};
use arete_mcp::protocol_agent::{ProtocolAgent, SQUADS_PROGRAM_SDK_HINT};
use clap::{Args, Subcommand};
use colored::Colorize;
use serde_json::Value;

const DASHBOARD_URL: &str = "https://app.arete.run/dashboard/squads";

#[derive(Subcommand, Debug)]
pub enum SquadsCommands {
    /// The account's Squads setup, or one attached multisig in detail
    Status(StatusArgs),
    /// The account's attached multisigs and their state
    List,
    /// Attach an existing Squads v4 multisig by address
    Attach(AttachArgs),
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
    };
    if json {
        println!("{result}");
    } else {
        print!("{}", render(&result));
    }
    Ok(())
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
    out.push_str(&format!("{SQUADS_PROGRAM_SDK_HINT}\n\n"));
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
    for proposal in result["proposals"].as_array().into_iter().flatten() {
        out.push_str(&format!(
            "  proposal #{} {} {} ({})\n",
            proposal["transactionIndex"],
            str_or(&proposal["step"], "?"),
            str_or(&proposal["status"], "?"),
            str_or(&proposal["squadsUrl"], "")
        ));
    }
    out.push('\n');
    out.push_str(&squads_app_lines(result));
    out.push_str(&format!("{}\n", str_or(&result["next"], "")));
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn pending_output_leads_with_the_dashboard_then_the_squads_app() {
        colored::control::set_override(false);
        let result = json!({
            "address": "Ms1", "state": "pending", "problem": "not_member",
            "dashboardUrl": "https://app.arete.run/dashboard/squads/Ms1",
            "proposer": "Prop1",
            "squadsAppSteps": ["Members → Add member.", "Permission: Initiate only."],
            "next": "Add the proposer."
        });
        for text in [render_attach(&result), render_detail(&result)] {
            assert!(
                text.starts_with("Dashboard: https://app.arete.run/dashboard/squads/Ms1"),
                "{text}"
            );
            let app = text.find("Members → Add member").unwrap();
            let sdk = text.find("squads-v4 program SDK").unwrap();
            assert!(app < sdk, "{text}");
            assert!(!text.contains("--keypair"), "{text}");
        }
    }

    #[test]
    fn active_output_has_no_squads_app_steps() {
        colored::control::set_override(false);
        let result = json!({
            "address": "Ms1", "state": "active",
            "dashboardUrl": "https://app.arete.run/dashboard/squads/Ms1",
            "deploymentConfigSnippet": "multisig = \"Ms1\"",
            "next": "Active."
        });
        let text = render_detail(&result);
        assert!(!text.contains("Squads app:"), "{text}");
        assert!(text.contains("Deployment config:"), "{text}");
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
