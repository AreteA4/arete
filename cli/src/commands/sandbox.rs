//! `a4 sandbox`: list the sandboxes the account has been granted and choose
//! one, read a sandbox wallet's history, holdings and rules, list the
//! programs it can read, and check a Rules v3 document before its owner
//! signs it.
//!
//! The reads go to the Arete API's sandbox routes with a short-lived token
//! the account's API key mints for the sandbox (as the MCP `sandbox_*` tools
//! do), so they work for an agent the sandbox's owner has granted access.
//! The sandbox resolves as for the MCP tools: `--sandbox`, then the one
//! chosen with `a4 sandbox use` for this directory, then `ARETE_SANDBOX_ID`,
//! then the only active sandbox granted. They never run an action: actions
//! go through the MCP `sandbox_invoke` tool. `rules validate` reads a local
//! file and sends nothing.

use std::io::Read;
use std::path::PathBuf;

use anyhow::{Context, Result};
use arete_mcp::sandbox::{ledger, rules, SandboxClient, SandboxRefusal};
use chrono::{DateTime, Duration, Utc};
use clap::{Args, Subcommand};
use colored::Colorize;
use serde_json::{json, Value};

use crate::ui::ExitCode;

/// History calls `--all` makes before it stops and prints the cursor to
/// carry on from.
const MAX_ALL_CALLS: usize = 50;

#[derive(Subcommand, Debug)]
pub enum SandboxCommands {
    /// The sandboxes this account has been granted, and which one is used
    List,
    /// Choose the sandbox the other commands use in this directory
    Use(UseArgs),
    /// The sandbox's actions, newest first (paginated, with filters)
    History(HistoryArgs),
    /// What the sandbox wallet holds now: SOL, tokens and program positions
    Holdings(HoldingsArgs),
    /// The sandbox's rules: show them, or check a Rules v3 file
    #[command(subcommand)]
    Rules(RulesCommands),
    /// The programs the sandbox can read: their accounts with `a4 program
    /// account decode` and their stacks with `a4 get` / `a4 stream`, each with
    /// --sandbox
    Reads(SandboxTarget),
}

#[derive(Args, Debug, Clone, Default)]
pub struct SandboxTarget {
    /// The sandbox id or name (default: the one chosen with `a4 sandbox use`,
    /// else ARETE_SANDBOX_ID, else the only sandbox granted)
    #[arg(long, value_name = "ID|NAME")]
    pub sandbox: Option<String>,
}

#[derive(Args, Debug, Clone, Default)]
pub struct UseArgs {
    /// The sandbox id or name, as `a4 sandbox list` shows it
    #[arg(value_name = "ID|NAME", required_unless_present = "clear")]
    pub sandbox: Option<String>,

    /// Forget the choice for this directory
    #[arg(long, conflicts_with = "sandbox")]
    pub clear: bool,
}

#[derive(Args, Debug, Clone, Default)]
pub struct HistoryArgs {
    #[command(flatten)]
    pub target: SandboxTarget,

    /// Actions per page, 1 to 100
    #[arg(long, default_value_t = ledger::DEFAULT_LIMIT)]
    pub limit: u32,

    /// Carry on from a previous page's `nextCursor`
    #[arg(long)]
    pub cursor: Option<String>,

    /// Only actions with this status (landed, failed, rejected, sent, ...)
    #[arg(long)]
    pub status: Option<String>,

    /// Only actions that called this program (id or name)
    #[arg(long)]
    pub program: Option<String>,

    /// Only actions that moved this token mint
    #[arg(long)]
    pub mint: Option<String>,

    /// Only actions since this time: RFC 3339, or an age such as 30m, 24h, 7d
    #[arg(long, value_name = "TIME")]
    pub since: Option<String>,

    /// Read every page, not just one
    #[arg(long)]
    pub all: bool,

    /// Keep each action's raw parsed transaction in --json output
    #[arg(long)]
    pub raw: bool,
}

#[derive(Args, Debug, Clone, Default)]
pub struct HoldingsArgs {
    #[command(flatten)]
    pub target: SandboxTarget,

    /// Keep each position's raw account data in --json output
    #[arg(long)]
    pub raw: bool,
}

#[derive(Subcommand, Debug)]
pub enum RulesCommands {
    /// The rules the sandbox runs under, as this agent sees them
    Show(SandboxTarget),
    /// Check a Rules v3 JSON file (`-` for stdin) without sending it anywhere
    Validate {
        /// The rules file, or `-` for stdin
        file: PathBuf,
    },
}

pub fn run(command: SandboxCommands, json: bool) -> Result<()> {
    match command {
        SandboxCommands::List => list(json),
        SandboxCommands::Use(args) => use_sandbox(args, json),
        SandboxCommands::History(args) => history(args, json),
        SandboxCommands::Holdings(args) => holdings(args, json),
        SandboxCommands::Rules(RulesCommands::Show(target)) => rules_show(target, json),
        SandboxCommands::Rules(RulesCommands::Validate { file }) => rules_validate(&file, json),
        SandboxCommands::Reads(target) => reads_show(target, json),
    }
}

/// The sandbox a read command (`program account decode`, `get`, `stream`)
/// reads through, before it resolves: `requested` (an id or a granted
/// sandbox's name), else, when the account's command groups grant
/// `sandboxes` and none of the command's `usual` groups, the sandbox the
/// other `a4 sandbox` commands use (chosen with `a4 sandbox use`, else
/// ARETE_SANDBOX_ID, else the only one granted). None: the account's own key
/// reads. The caller resolves it with [`SandboxClient::pin`], and a sandbox
/// that does not resolve is refused with the resolver's hint, never read
/// with the account's key.
pub fn read_sandbox(
    requested: Option<&str>,
    usual: &[arete_mcp::entitlements::Group],
    config: &str,
) -> Option<SandboxClient> {
    use arete_mcp::entitlements::Group;
    let requested = requested.map(str::trim).filter(|wanted| !wanted.is_empty());
    if requested.is_none() {
        let surface = crate::entitlements::account_surface(None, None, config);
        let only_sandbox =
            surface.allows(Group::Sandboxes) && !usual.iter().any(|group| surface.allows(*group));
        if !only_sandbox {
            return None;
        }
    }
    Some(SandboxClient::from_env().targeting(requested))
}

/// A sandbox refusal of a read, printed like the other sandbox refusals.
pub fn read_refused(refusal: SandboxRefusal, json: bool) -> anyhow::Error {
    refused(refusal, json)
}

fn runtime() -> Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("Failed to start the async runtime")
}

/// A client for the sandbox `target` resolves to, and its id.
fn client(
    runtime: &tokio::runtime::Runtime,
    target: &SandboxTarget,
    json: bool,
) -> Result<(SandboxClient, String)> {
    let client = SandboxClient::from_env().targeting(target.sandbox.as_deref());
    let resolved = runtime
        .block_on(client.resolve())
        .map_err(|refusal| refused(refusal, json))?;
    // Resolved once: the calls below name the id, with no second listing.
    let client = client.targeting(Some(&resolved.sandbox_id));
    Ok((client, resolved.sandbox_id))
}

/// The hint for `code` in CLI terms, where the MCP hint names MCP tools.
fn cli_hint(code: &str) -> Option<&'static str> {
    Some(match code {
        "sandbox-none-granted" => {
            "No sandbox owner has granted this agent access yet. Ask the owner to grant it access in the dashboard, then run `a4 sandbox list`."
        }
        "sandbox-selection-required" => {
            "Choose one with `a4 sandbox use <id|name>` (remembered for this directory), or pass --sandbox."
        }
        "sandbox-unknown" => "`a4 sandbox list` shows the sandboxes this agent may use.",
        "sandbox-not-configured" => {
            "This Arete API cannot list the sandboxes granted to this agent: pass --sandbox <id> or set ARETE_SANDBOX_ID."
        }
        "sandbox-grant-missing" | "sandbox-consent-unsigned" => {
            "The sandbox's owner has not granted this agent access to this sandbox (or must grant it again). `a4 sandbox list` shows the ones it may use and `a4 sandbox use` picks one; or ask the owner to grant access in the dashboard."
        }
        _ => return None,
    })
}

/// Print a refusal the way API errors are printed (the problem as JSON on
/// stdout with --json, else the message and a hint on stderr) and exit
/// with 77 for auth refusals, 75 when rate limited, else 1.
fn refused(refusal: SandboxRefusal, json: bool) -> anyhow::Error {
    let mut shown = refusal.to_json();
    if let Some(hint) = shown["code"].as_str().and_then(cli_hint) {
        shown["hint"] = json!(hint);
    }
    if json {
        println!("{shown}");
    } else {
        let code = shown["code"].as_str().unwrap_or("sandbox-request-failed");
        let message = shown["error"]
            .as_str()
            .or_else(|| shown["message"].as_str())
            .unwrap_or("the Arete API refused the request");
        eprintln!("{} {message} ({code})", "Error:".red().bold());
        for sandbox in shown["sandboxes"].as_array().into_iter().flatten() {
            eprintln!("  {}", sandbox_line(sandbox));
        }
        if let Some(hint) = shown["hint"].as_str() {
            eprintln!("{hint}");
        } else if refusal.status == 404 {
            eprintln!(
                "This Arete API does not serve sandbox history and holdings yet, or the sandbox id is wrong."
            );
        }
    }
    let code = match refusal.status {
        401 | 403 => 77,
        429 => 75,
        _ => 1,
    };
    ExitCode(code).into()
}

// ---- list and use -------------------------------------------------------------

fn list(json: bool) -> Result<()> {
    let client = SandboxClient::from_env();
    let mut body = runtime()?
        .block_on(client.list())
        .map_err(|refusal| refused(refusal, json))?;
    // The listing's note names the MCP tools; say it in CLI terms.
    match list_note(&body) {
        Some(note) => body["note"] = json!(note),
        None => {
            if let Some(object) = body.as_object_mut() {
                object.remove("note");
            }
        }
    }
    if json {
        println!("{body}");
    } else {
        print!("{}", render_list(&body));
    }
    Ok(())
}

fn use_sandbox(args: UseArgs, json: bool) -> Result<()> {
    let client = SandboxClient::from_env();
    let body = runtime()?
        .block_on(client.select(args.sandbox.as_deref(), args.clear))
        .map_err(|refusal| refused(refusal, json))?;
    if json {
        println!("{body}");
        return Ok(());
    }
    let saved = body["saved"] == true;
    if body["cleared"] == true {
        println!("Forgot the sandbox chosen for this directory.");
        match body["current"].as_object() {
            Some(_) => println!(
                "Sandbox commands now use {} ({}).",
                body["current"]["sandboxId"].as_str().unwrap_or("?"),
                source_label(body["current"]["source"].as_str().unwrap_or(""))
            ),
            None => println!("Pass --sandbox, or choose again with `a4 sandbox use`."),
        }
    } else {
        println!(
            "{} {}",
            "Using".green().bold(),
            sandbox_line(&body["selected"])
        );
        if saved {
            println!("Remembered for this directory (by `a4 sandbox` and `a4 mcp`).");
        }
    }
    if !saved {
        println!(
            "{} the choice could not be saved, so it lasted only for this command.",
            "Warning:".yellow().bold()
        );
    }
    if body["note"].is_string() {
        println!(
            "{}",
            "Its actions are refused until the owner resumes it.".yellow()
        );
    }
    Ok(())
}

/// What `a4 sandbox list` should say beneath the list, if anything.
pub fn list_note(body: &Value) -> Option<String> {
    let sandboxes = body["sandboxes"].as_array().cloned().unwrap_or_default();
    let current = body["current"]["sandboxId"].as_str();
    let entry = |id: &str| {
        sandboxes
            .iter()
            .find(|sandbox| sandbox["sandboxId"].as_str() == Some(id))
    };
    match current {
        Some(id) => match entry(id) {
            None => Some(format!(
                "The sandbox in use ({id}, from {}) is not granted to this agent, so its commands are refused. Choose a listed one with `a4 sandbox use`.",
                source_label(body["current"]["source"].as_str().unwrap_or(""))
            )),
            Some(sandbox) if sandbox["status"] != "active" => Some(
                "The sandbox in use is paused: its actions are refused until the owner resumes it."
                    .into(),
            ),
            Some(_) => None,
        },
        None if sandboxes.is_empty() => Some(
            "No sandbox is granted to this agent yet. Ask a sandbox owner to grant it access in the dashboard."
                .into(),
        ),
        None if sandboxes.iter().all(|sandbox| sandbox["status"] != "active") => Some(
            "Every sandbox granted to this agent is paused. Only an owner can resume one.".into(),
        ),
        None => Some(
            "Several sandboxes are granted: choose one with `a4 sandbox use <id|name>`, or pass --sandbox."
                .into(),
        ),
    }
}

/// Where the sandbox in use came from, in words.
fn source_label(source: &str) -> &'static str {
    match source {
        "argument" => "--sandbox",
        "selected" => "chosen with `a4 sandbox use`",
        "saved" => "saved for this directory",
        "env:ARETE_SANDBOX_ID" => "ARETE_SANDBOX_ID",
        "only-grant" => "the only sandbox granted",
        _ => "unknown",
    }
}

/// One sandbox on one line: id, name, status, cluster, actions, wallet.
fn sandbox_line(sandbox: &Value) -> String {
    let mut line = sandbox["sandboxId"].as_str().unwrap_or("?").to_string();
    if let Some(name) = sandbox["name"].as_str() {
        line.push_str(&format!("  {}", name.bold()));
    }
    if let Some(status) = sandbox["status"].as_str() {
        let status = if status == "active" {
            status.green().to_string()
        } else {
            status.yellow().to_string()
        };
        line.push_str(&format!("  {status}"));
    }
    if let Some(cluster) = sandbox["cluster"].as_str() {
        line.push_str(&format!("  {cluster}"));
    }
    if let Some(actions) = sandbox["actions"].as_array() {
        let actions: Vec<&str> = actions.iter().filter_map(Value::as_str).collect();
        let actions = if actions.is_empty() {
            "none".to_string()
        } else {
            actions.join(", ")
        };
        line.push_str(&format!("  actions: {actions}"));
    }
    if let Some(wallet) = sandbox["walletAddress"].as_str() {
        line.push_str(&format!("  wallet: {wallet}"));
    }
    line
}

/// The human rendering of `a4 sandbox list`.
pub fn render_list(body: &Value) -> String {
    let sandboxes = body["sandboxes"].as_array().cloned().unwrap_or_default();
    let mut out = format!("{}\n\n", "Sandboxes granted to this agent".bold());
    if sandboxes.is_empty() {
        out.push_str("  none\n");
    }
    for sandbox in &sandboxes {
        let mark = if sandbox["current"] == true { "*" } else { " " };
        out.push_str(&format!("{mark} {}\n", sandbox_line(sandbox)));
        if let Some(expires) = sandbox["grantExpiresAt"].as_str() {
            out.push_str(&format!("    grant expires {expires}\n"));
        }
    }
    if let Some(id) = body["current"]["sandboxId"].as_str() {
        out.push_str(&format!(
            "\nIn use: {id} ({})\n",
            source_label(body["current"]["source"].as_str().unwrap_or(""))
        ));
    }
    if let Some(note) = body["note"].as_str() {
        out.push_str(&format!("\n{note}\n"));
    }
    out
}

// ---- history ------------------------------------------------------------------

fn history(args: HistoryArgs, json: bool) -> Result<()> {
    let runtime = runtime()?;
    let (client, sandbox_id) = client(&runtime, &args.target, json)?;
    let query = ledger::HistoryQuery {
        cursor: args.cursor.clone(),
        limit: Some(args.limit),
        status: args.status.clone(),
        program: args.program.clone(),
        mint: args.mint.clone(),
        since: args
            .since
            .as_deref()
            .map(|text| parse_since(text, Utc::now()))
            .transpose()?,
    };
    let body = if args.all {
        let mut query = query.clone();
        let mut items = Vec::new();
        let mut calls = 0;
        loop {
            let page = runtime
                .block_on(client.history(&query))
                .map_err(|refusal| refused(refusal, json))?;
            calls += 1;
            items.extend(page["items"].as_array().cloned().unwrap_or_default());
            let next = next_cursor(&page);
            if next.is_none() || calls >= MAX_ALL_CALLS {
                break json!({ "items": items, "nextCursor": next });
            }
            query.cursor = next;
        }
    } else {
        runtime
            .block_on(client.history(&query))
            .map_err(|refusal| refused(refusal, json))?
    };
    let body = if args.raw {
        body
    } else {
        ledger::without_raw(body)
    };
    if json {
        println!("{body}");
    } else {
        print!("{}", render_history(&body, &args, query.since, &sandbox_id));
    }
    Ok(())
}

fn next_cursor(page: &Value) -> Option<String> {
    page["nextCursor"]
        .as_str()
        .filter(|cursor| !cursor.is_empty())
        .map(str::to_string)
}

/// `--since`: an RFC 3339 time, or an age before `now` (`90s`, `30m`, `24h`,
/// `7d`).
pub fn parse_since(text: &str, now: DateTime<Utc>) -> Result<DateTime<Utc>> {
    let text = text.trim();
    if let Ok(time) = DateTime::parse_from_rfc3339(text) {
        return Ok(time.with_timezone(&Utc));
    }
    let bad = || {
        anyhow::anyhow!(
            "--since takes an RFC 3339 time (2026-11-15T00:00:00Z) or an age such as 30m, 24h or 7d, not {text:?}"
        )
    };
    // The unit is the last character, which may be more than one byte.
    let split = text
        .char_indices()
        .next_back()
        .map(|(index, _)| index)
        .ok_or_else(bad)?;
    let (amount, unit) = text.split_at(split);
    let amount: i64 = amount.parse().map_err(|_| bad())?;
    let age = match unit {
        "s" => Duration::try_seconds(amount),
        "m" => Duration::try_minutes(amount),
        "h" => Duration::try_hours(amount),
        "d" => Duration::try_days(amount),
        _ => None,
    }
    .filter(|age| *age >= Duration::zero())
    .ok_or_else(bad)?;
    now.checked_sub_signed(age).ok_or_else(bad)
}

/// The human rendering of a history page. `since` is the cutoff `--since`
/// resolved to.
pub fn render_history(
    body: &Value,
    args: &HistoryArgs,
    since: Option<DateTime<Utc>>,
    sandbox_id: &str,
) -> String {
    let items = body["items"].as_array().cloned().unwrap_or_default();
    let mut out = format!(
        "{} {}  history, newest first\n\n",
        "Sandbox".bold(),
        sandbox_id
    );
    if items.is_empty() {
        out.push_str("  No actions");
        if filtered(args) {
            out.push_str(" matched the filters");
        }
        out.push_str(".\n");
    }
    for item in &items {
        out.push_str(&history_row(item));
    }
    if let Some(cursor) = next_cursor(body) {
        out.push_str(&format!(
            "\nMore: a4 sandbox history --cursor {cursor}{}\n",
            repeat_flags(args, since)
        ));
    }
    out
}

fn filtered(args: &HistoryArgs) -> bool {
    args.status.is_some() || args.program.is_some() || args.mint.is_some() || args.since.is_some()
}

/// The flags to repeat with `--cursor` for the next page. `--since` is
/// repeated as the time it resolved to, never as an age (`24h`), so later
/// pages keep the same cutoff.
fn repeat_flags(args: &HistoryArgs, since: Option<DateTime<Utc>>) -> String {
    let mut flags = String::new();
    if let Some(sandbox) = &args.target.sandbox {
        flags.push_str(&format!(" --sandbox {sandbox}"));
    }
    if args.limit != ledger::DEFAULT_LIMIT {
        flags.push_str(&format!(" --limit {}", args.limit));
    }
    let since = since.map(|since| since.to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
    for (flag, value) in [
        ("--status", &args.status),
        ("--program", &args.program),
        ("--mint", &args.mint),
        ("--since", &since),
    ] {
        if let Some(value) = value {
            flags.push_str(&format!(" {flag} {}", shell_word(value)));
        }
    }
    flags
}

fn shell_word(value: &str) -> String {
    if !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.:+/".contains(c))
    {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', r"'\''"))
    }
}

fn history_row(item: &Value) -> String {
    let time = item["createdAt"]
        .as_str()
        .map(|text| match DateTime::parse_from_rfc3339(text) {
            Ok(time) => time
                .with_timezone(&Utc)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string(),
            Err(_) => text.to_string(),
        })
        .unwrap_or_else(|| "?".into());
    let status = item["status"].as_str().unwrap_or("?");
    let mut line = format!(
        "  {time}  {}  {}",
        status_label(status),
        instruction_labels(item)
    );
    let mut changes = Vec::new();
    if let Some(delta) = amount(&item["solDelta"]).filter(|delta| *delta != 0) {
        changes.push(format!("{} SOL", signed_units(delta, 9)));
    }
    for delta in item["tokenDeltas"].as_array().into_iter().flatten() {
        let Some(raw) = amount(&delta["delta"]).filter(|raw| *raw != 0) else {
            continue;
        };
        let decimals = delta["decimals"].as_u64().unwrap_or(0) as u32;
        changes.push(format!(
            "{} {}",
            signed_units(raw, decimals),
            short(delta["mint"].as_str().unwrap_or("?"))
        ));
    }
    if !changes.is_empty() {
        line.push_str(&format!("  {}", changes.join(", ")));
    }
    if let Some(signature) = item["signature"].as_str() {
        line.push_str(&format!("  {}", short(signature).dimmed()));
    }
    line.push('\n');
    if let Some(events) = event_labels(item) {
        line.push_str(&format!("      events: {events}\n"));
    }
    if let Some(reason) = item["rejectCode"]
        .as_str()
        .or_else(|| item["error"]["code"].as_str())
        .or_else(|| item["error"].as_str())
    {
        line.push_str(&format!("      {reason}\n"));
    }
    line
}

/// The Anchor events an action emitted, in order: each event's name (the
/// emitting program when no IDL event matched), marked `(not decoded)` when
/// its fields did not decode. A run of the same label is shown once with
/// its count. `None` when the item has no events.
fn event_labels(item: &Value) -> Option<String> {
    let mut runs: Vec<(String, usize)> = Vec::new();
    for event in item["events"].as_array().into_iter().flatten() {
        let mut label = match event["name"].as_str() {
            Some(name) => name.to_string(),
            None => short(event["program"].as_str().unwrap_or("?")),
        };
        if event["fields"].get("__parse_error").is_some() {
            label.push_str(" (not decoded)");
        }
        match runs.last_mut() {
            Some((last, count)) if *last == label => *count += 1,
            _ => runs.push((label, 1)),
        }
    }
    if runs.is_empty() {
        return None;
    }
    Some(
        runs.into_iter()
            .map(|(label, count)| match count {
                1 => label,
                n => format!("{label} ×{n}"),
            })
            .collect::<Vec<_>>()
            .join(", "),
    )
}

fn status_label(status: &str) -> String {
    let padded = format!("{status:<8}");
    match status {
        "landed" => padded.green().to_string(),
        "failed" | "rejected" => padded.red().to_string(),
        _ => padded.yellow().to_string(),
    }
}

/// `program.instruction` for each decoded instruction (the program's name
/// when the API gives one), else the programs called.
fn instruction_labels(item: &Value) -> String {
    let labels: Vec<String> = item["instructions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|instruction| match instruction {
            Value::String(name) => Some(name.clone()),
            Value::Object(object) => {
                let name = object
                    .get("name")
                    .or_else(|| object.get("instruction"))
                    .and_then(Value::as_str)?;
                let program = object
                    .get("programName")
                    .or_else(|| object.get("program"))
                    .and_then(Value::as_str)
                    .map(|program| {
                        if rules::is_pubkey(program) {
                            short(program)
                        } else {
                            program.to_string()
                        }
                    });
                Some(match program {
                    Some(program) => format!("{program}.{name}"),
                    None => name.to_string(),
                })
            }
            _ => None,
        })
        .collect();
    if !labels.is_empty() {
        return labels.join(", ");
    }
    let programs: Vec<String> = ledger::item_programs(item)
        .map(|program| {
            if rules::is_pubkey(program) {
                short(program)
            } else {
                program.to_string()
            }
        })
        .collect();
    if programs.is_empty() {
        "-".into()
    } else {
        programs.join(", ")
    }
}

// ---- holdings -----------------------------------------------------------------

fn holdings(args: HoldingsArgs, json: bool) -> Result<()> {
    let runtime = runtime()?;
    let (client, sandbox_id) = client(&runtime, &args.target, json)?;
    let body = runtime
        .block_on(client.holdings())
        .map_err(|refusal| refused(refusal, json))?;
    let body = if args.raw {
        body
    } else {
        ledger::without_raw(body)
    };
    if json {
        println!("{body}");
    } else {
        print!("{}", render_holdings(&body, &sandbox_id));
    }
    Ok(())
}

/// The human rendering of holdings.
pub fn render_holdings(body: &Value, sandbox_id: &str) -> String {
    let mut out = format!("{} {}  holdings", "Sandbox".bold(), sandbox_id);
    if let Some(slot) = body["asOfSlot"].as_u64() {
        out.push_str(&format!(" at slot {slot}"));
    }
    out.push_str("\n\n");
    let sol = amount(&body["sol"])
        .map(|lamports| units(lamports, 9))
        .unwrap_or_else(|| "?".into());
    out.push_str(&format!("  SOL        {sol}\n"));

    let tokens = body["tokens"].as_array().cloned().unwrap_or_default();
    if tokens.is_empty() {
        out.push_str("  Tokens     none\n");
    } else {
        out.push_str("  Tokens\n");
        for token in &tokens {
            let decimals = token["decimals"].as_u64().unwrap_or(0) as u32;
            let held = amount(&token["amount"])
                .map(|raw| units(raw, decimals))
                .unwrap_or_else(|| "?".into());
            let bought = if token["acquiredBySandbox"].as_bool() == Some(true) {
                "  bought by the sandbox (may always be sold)"
            } else {
                ""
            };
            out.push_str(&format!(
                "    {}  {held}{bought}\n",
                token["mint"].as_str().unwrap_or("?")
            ));
        }
    }

    let positions = body["positions"].as_array().cloned().unwrap_or_default();
    if !positions.is_empty() {
        out.push_str("  Positions (raw accounts)\n");
        for position in &positions {
            let program = position["program"]
                .as_str()
                .map(|program| {
                    if rules::is_pubkey(program) {
                        short(program)
                    } else {
                        program.to_string()
                    }
                })
                .unwrap_or_else(|| "?".into());
            out.push_str(&format!(
                "    {program}  {}  {}\n",
                position["label"].as_str().unwrap_or("account"),
                position["account"].as_str().unwrap_or("?")
            ));
        }
    }
    out
}

// ---- rules --------------------------------------------------------------------

fn rules_show(target: SandboxTarget, json: bool) -> Result<()> {
    let runtime = runtime()?;
    let (client, _) = client(&runtime, &target, json)?;
    let body = runtime
        .block_on(client.rules())
        .map_err(|refusal| refused(refusal, json))?;
    if json {
        println!("{body}");
    } else {
        print!("{}", render_rules(&body));
    }
    Ok(())
}

// ---- reads --------------------------------------------------------------------

fn reads_show(target: SandboxTarget, json: bool) -> Result<()> {
    let runtime = runtime()?;
    let (client, sandbox_id) = client(&runtime, &target, json)?;
    let body = runtime
        .block_on(client.reads())
        .map_err(|refusal| refused(refusal, json))?;
    if json {
        println!("{body}");
    } else {
        print!("{}", render_reads(&body, &sandbox_id));
    }
    Ok(())
}

/// The human rendering of the programs a sandbox can read.
pub fn render_reads(body: &Value, sandbox_id: &str) -> String {
    let programs = arete_mcp::program_read::readable_programs(body);
    let mut out = String::new();
    if programs.is_empty() {
        out.push_str(&format!(
            "Sandbox {sandbox_id} can read no programs: its rules enable none.\n"
        ));
        return out;
    }
    let version = body["rulesVersion"]
        .as_u64()
        .map(|version| format!(" (rules v{version})"))
        .unwrap_or_default();
    out.push_str(&format!(
        "Sandbox {sandbox_id} can read {} program{}{version}:\n",
        programs.len(),
        if programs.len() == 1 { "" } else { "s" }
    ));
    for (id, name) in &programs {
        match name {
            Some(name) => out.push_str(&format!("  {name:<24} {id}\n")),
            None => out.push_str(&format!("  {id}\n")),
        }
    }
    out.push_str(&format!(
        "\nRead an account:  a4 program account decode <address> --sandbox {sandbox_id}\n\
         Read a stack:     a4 get <View> --stack <stack> --sandbox {sandbox_id}\n\
         A stack opens only when every program it decodes is listed here.\n"
    ));
    out
}

fn rules_validate(file: &std::path::Path, json: bool) -> Result<()> {
    let (name, text) = if file.as_os_str() == "-" {
        let mut text = String::new();
        std::io::stdin()
            .read_to_string(&mut text)
            .context("Failed to read the rules from stdin")?;
        ("stdin".to_string(), text)
    } else {
        (
            file.display().to_string(),
            std::fs::read_to_string(file)
                .with_context(|| format!("Failed to read {}", file.display()))?,
        )
    };
    let report = match serde_json::from_str::<Value>(&text) {
        Ok(document) => rules::validate(&document),
        Err(error) => rules::RulesReport {
            valid: false,
            errors: vec![rules::RulesIssue {
                path: String::new(),
                message: format!("not JSON: {error}"),
            }],
            ..Default::default()
        },
    };
    if json {
        println!("{}", serde_json::to_string(&report)?);
    } else {
        print!("{}", render_report(&name, &report));
    }
    if report.valid {
        Ok(())
    } else {
        Err(ExitCode(1).into())
    }
}

/// The human rendering of a validation report.
pub fn render_report(name: &str, report: &rules::RulesReport) -> String {
    let mut out = if report.valid {
        let enabled = if report.enabled_programs.is_empty() {
            "no programs enabled".to_string()
        } else {
            format!("enabled: {}", report.enabled_programs.join(", "))
        };
        format!("{name}: {} ({enabled})\n", "valid Rules v3".green())
    } else {
        format!(
            "{name}: {}\n",
            format!(
                "{} error{}",
                report.errors.len(),
                if report.errors.len() == 1 { "" } else { "s" }
            )
            .red()
        )
    };
    for (label, issues) in [
        ("error".red().to_string(), &report.errors),
        ("warning".yellow().to_string(), &report.warnings),
    ] {
        for issue in issues {
            let path = if issue.path.is_empty() {
                "(document)"
            } else {
                &issue.path
            };
            out.push_str(&format!("  {label}  {path}: {}\n", issue.message));
        }
    }
    out
}

/// The human rendering of the rules an agent sees: Rules v3 fields when the
/// API gives them, and the allowed instructions either way.
pub fn render_rules(body: &Value) -> String {
    let rules = rules::find_document(body);
    let mut out = match rules["version"].as_u64() {
        Some(version) => format!("{} v{version}\n", "Rules".bold()),
        None => format!("{}\n", "Rules".bold()),
    };
    if let Some(reason) = body["unavailable"]
        .get("message")
        .or_else(|| body["unavailable"].get("error"))
        .and_then(Value::as_str)
        .or_else(|| body["unavailable"].as_str())
    {
        out.push_str(&format!("  {} {reason}\n", "unavailable:".red()));
    }

    let programs = rules["programs"].as_array().cloned().unwrap_or_default();
    if programs.is_empty() {
        out.push_str("  Programs   none\n");
    } else {
        out.push_str("  Programs\n");
        for program in &programs {
            let state = match program["enabled"].as_bool() {
                Some(false) => "off".dimmed().to_string(),
                _ => "on ".green().to_string(),
            };
            let mut line = format!(
                "    {state}  {}  {}",
                program["name"].as_str().unwrap_or("?"),
                program["programId"].as_str().unwrap_or("?")
            );
            line.push_str(&format!(
                "\n           instructions: {}",
                allowed_instructions(program)
            ));
            if let Some(limit) = program["spendLimit"].as_object() {
                let lamports = limit
                    .get("lamports")
                    .and_then(amount)
                    .map(|lamports| units(lamports, 9))
                    .unwrap_or_else(|| "?".into());
                let window = limit
                    .get("windowSeconds")
                    .and_then(Value::as_u64)
                    .map(window_label)
                    .unwrap_or_else(|| "?".into());
                line.push_str(&format!(
                    "\n           spend limit: {lamports} SOL per {window}"
                ));
            }
            out.push_str(&line);
            out.push('\n');
        }
    }

    if let Some(allowlist) = rules["transferAllowlist"].as_array() {
        out.push_str(
            "  Transfers out of the programs may go to the owner's wallet, the sandbox's own accounts",
        );
        if allowlist.is_empty() {
            out.push_str(" (no allowlist)\n");
        } else {
            out.push_str(" and:\n");
            for address in allowlist {
                out.push_str(&format!("    {}\n", address.as_str().unwrap_or("?")));
            }
        }
    }

    let baseline: Vec<&str> = body["baseline"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|program| program["name"].as_str().or_else(|| program.as_str()))
        .collect();
    if !baseline.is_empty() {
        out.push_str(&format!("  Always allowed: {}\n", baseline.join(", ")));
    }
    out
}

/// The instructions a program entry allows: a v3 `instructions` list of
/// names, the agent view's `allowed` (`"any"` or names) or its detailed
/// `instructions`, else the registry's safe set.
fn allowed_instructions(program: &Value) -> String {
    if program["allowed"].as_str() == Some("any") {
        return "any".into();
    }
    for key in ["allowed", "instructions"] {
        if let Some(list) = program[key].as_array() {
            let names: Vec<&str> = list
                .iter()
                .filter_map(|entry| entry.as_str().or_else(|| entry["name"].as_str()))
                .collect();
            if !names.is_empty() {
                return names.join(", ");
            }
            if key == "instructions" {
                return "none".into();
            }
        }
    }
    "the registry's safe set".into()
}

fn window_label(seconds: u64) -> String {
    match seconds {
        86_400 => "day".into(),
        3_600 => "hour".into(),
        60 => "minute".into(),
        s if s % 86_400 == 0 => format!("{} days", s / 86_400),
        s if s % 3_600 == 0 => format!("{} hours", s / 3_600),
        s if s % 60 == 0 => format!("{} minutes", s / 60),
        s => format!("{s} seconds"),
    }
}

// ---- amounts ------------------------------------------------------------------

/// An integer amount given as a JSON number or a decimal string.
fn amount(value: &Value) -> Option<i128> {
    match value {
        Value::Number(number) => number
            .as_i64()
            .map(i128::from)
            .or_else(|| number.as_u64().map(i128::from)),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

/// `raw` in whole units with `decimals` places, trailing zeros trimmed.
pub fn units(raw: i128, decimals: u32) -> String {
    let negative = raw < 0;
    let magnitude = raw.unsigned_abs();
    let scale = 10u128.pow(decimals.min(38));
    let whole = magnitude / scale;
    let fraction = magnitude % scale;
    let mut text = whole.to_string();
    if fraction > 0 {
        let digits = format!("{fraction:0width$}", width = decimals as usize);
        text.push('.');
        text.push_str(digits.trim_end_matches('0'));
    }
    if negative {
        format!("-{text}")
    } else {
        text
    }
}

fn signed_units(raw: i128, decimals: u32) -> String {
    if raw > 0 {
        format!("+{}", units(raw, decimals))
    } else {
        units(raw, decimals)
    }
}

/// `abcd…wxyz` for a long address or signature.
fn short(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    if chars.len() <= 12 {
        return value.to_string();
    }
    let head: String = chars[..4].iter().collect();
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("{head}…{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const PUMP: &str = "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P";
    const MINT: &str = "7xKXtg2CW87d97TXJSDpbD5jBkheTqA83TZRuJosgAsU";

    fn fixture(name: &str) -> Value {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/sandbox")
            .join(name);
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    fn granted_listing(current: Value) -> Value {
        json!({
            "sandboxes": [
                { "sandboxId": "6f1c2c43-6d3c-4a51-9b8e-0e3c1e2a7b10", "name": "Bot",
                  "walletAddress": "W1", "cluster": "mainnet-beta", "status": "active",
                  "actions": ["invoke", "withdraw"], "grantExpiresAt": "2026-12-01T00:00:00Z",
                  "current": current["sandboxId"] == "6f1c2c43-6d3c-4a51-9b8e-0e3c1e2a7b10" },
                { "sandboxId": "0b5a3e8e-2f7d-4c1a-8e2b-7f3c9d1e4a20", "name": null,
                  "walletAddress": null, "cluster": "devnet", "status": "paused",
                  "actions": [], "grantExpiresAt": null, "current": false },
            ],
            "current": current,
        })
    }

    #[test]
    fn the_list_marks_the_sandbox_in_use_and_where_it_came_from() {
        colored::control::set_override(false);
        let body = granted_listing(json!({
            "sandboxId": "6f1c2c43-6d3c-4a51-9b8e-0e3c1e2a7b10", "source": "saved"
        }));
        assert_eq!(list_note(&body), None);
        let text = render_list(&body);
        assert!(text.contains(
            "* 6f1c2c43-6d3c-4a51-9b8e-0e3c1e2a7b10  Bot  active  mainnet-beta  actions: invoke, withdraw  wallet: W1"
        ), "{text}");
        assert!(
            text.contains("    grant expires 2026-12-01T00:00:00Z"),
            "{text}"
        );
        assert!(
            text.contains("  0b5a3e8e-2f7d-4c1a-8e2b-7f3c9d1e4a20  paused  devnet  actions: none"),
            "{text}"
        );
        assert!(
            text.contains(
                "In use: 6f1c2c43-6d3c-4a51-9b8e-0e3c1e2a7b10 (saved for this directory)"
            ),
            "{text}"
        );
    }

    #[test]
    fn the_list_says_what_to_do_in_cli_terms() {
        let several = granted_listing(Value::Null);
        let mut several_active = several.clone();
        several_active["sandboxes"][1]["status"] = json!("active");
        let note = list_note(&several_active).unwrap();
        assert!(note.contains("a4 sandbox use"), "{note}");
        assert!(!note.contains("sandbox_select"), "{note}");

        let mut all_paused = several.clone();
        all_paused["sandboxes"][0]["status"] = json!("paused");
        assert!(list_note(&all_paused).unwrap().contains("paused"));
        let none = json!({ "sandboxes": [], "current": null });
        assert!(list_note(&none).unwrap().contains("dashboard"));
        assert!(render_list(&none).contains("  none"));

        let stale = granted_listing(json!({
            "sandboxId": "9d4f6a1b-3c2e-4b8a-a1f0-5e6d7c8b9a01", "source": "env:ARETE_SANDBOX_ID"
        }));
        let note = list_note(&stale).unwrap();
        assert!(
            note.contains("not granted") && note.contains("ARETE_SANDBOX_ID"),
            "{note}"
        );
    }

    #[test]
    fn selection_refusals_get_cli_hints() {
        for code in [
            "sandbox-none-granted",
            "sandbox-selection-required",
            "sandbox-unknown",
            "sandbox-not-configured",
            "sandbox-grant-missing",
        ] {
            let hint = cli_hint(code).unwrap();
            assert!(
                hint.contains("a4 sandbox") || hint.contains("--sandbox"),
                "{code}: {hint}"
            );
            assert!(!hint.contains("sandbox_"), "{code}: {hint}");
        }
        assert_eq!(cli_hint("sandbox-paused"), None);
    }

    #[test]
    fn reads_render_the_programs_and_how_to_read_them() {
        let body = json!({
            "sandboxId": "s",
            "rulesVersion": 7,
            "programs": [
                {"programId": "JUP6", "name": "Jupiter"},
                {"programId": "KLend"},
            ],
        });
        let text = render_reads(&body, "s");
        assert!(text.contains("can read 2 programs (rules v7)"), "{text}");
        assert!(text.contains("Jupiter"), "{text}");
        assert!(text.contains("  KLend\n"), "{text}");
        assert!(text.contains("--sandbox s"), "{text}");
        let none = render_reads(&json!({"programs": []}), "s");
        assert!(none.contains("can read no programs"), "{none}");
    }

    #[test]
    fn amounts_render_in_whole_units() {
        assert_eq!(units(1_500_000_000, 9), "1.5");
        assert_eq!(units(-5_000, 9), "-0.000005");
        assert_eq!(units(42, 0), "42");
        assert_eq!(signed_units(1_234_500_000, 6), "+1234.5");
        assert_eq!(amount(&json!("-105000000")), Some(-105_000_000));
        assert_eq!(amount(&json!(7)), Some(7));
        assert_eq!(amount(&json!("x")), None);
    }

    #[test]
    fn since_takes_a_time_or_an_age() {
        let now = DateTime::parse_from_rfc3339("2026-11-15T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(
            parse_since("24h", now).unwrap().to_rfc3339(),
            "2026-11-14T12:00:00+00:00"
        );
        assert_eq!(
            parse_since("7d", now).unwrap().to_rfc3339(),
            "2026-11-08T12:00:00+00:00"
        );
        assert_eq!(
            parse_since("2026-11-01T00:00:00Z", now)
                .unwrap()
                .to_rfc3339(),
            "2026-11-01T00:00:00+00:00"
        );
        for bad in ["", "h", "-1h", "3w", "yesterday", "1日", "日"] {
            assert!(parse_since(bad, now).is_err(), "{bad}");
        }
    }

    #[test]
    fn history_renders_one_line_per_action_and_the_next_page() {
        colored::control::set_override(false);
        let args = HistoryArgs {
            limit: 2,
            status: Some("landed".into()),
            target: SandboxTarget {
                sandbox: Some("sbx".into()),
            },
            ..Default::default()
        };
        let text = render_history(&fixture("history-page.json"), &args, None, "sbx");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "Sandbox sbx  history, newest first");
        assert!(
            lines[2].starts_with(
                "  2026-11-12 10:00:00  landed    Pump.fun.buy  -0.105005 SOL, +1234.5 7xKX…gAsU"
            ),
            "{text}"
        );
        assert_eq!(
            lines[3],
            "      events: TradeEvent ×2, MAyh…MD4e (not decoded), CreateEvent (not decoded)",
            "{text}"
        );
        assert!(
            lines[4].contains("rejected  ") && lines[4].contains("Pump.fun.sell"),
            "{text}"
        );
        assert!(lines[5].contains("sandbox-guard-trial-failed"), "{text}");
        assert!(
            text.ends_with(
                "More: a4 sandbox history --cursor eyJpZCI6ImEyIn0 --sandbox sbx --limit 2 --status landed\n"
            ),
            "{text}"
        );
    }

    #[test]
    fn an_empty_filtered_page_says_so() {
        colored::control::set_override(false);
        let args = HistoryArgs {
            mint: Some(MINT.into()),
            ..Default::default()
        };
        let text = render_history(
            &json!({ "items": [], "nextCursor": null }),
            &args,
            None,
            "s",
        );
        assert!(text.contains("No actions matched the filters."), "{text}");
        assert!(!text.contains("More:"), "{text}");
    }

    #[test]
    fn holdings_render_balances_and_positions() {
        colored::control::set_override(false);
        let text = render_holdings(&fixture("holdings.json"), "sbx");
        assert!(
            text.starts_with("Sandbox sbx  holdings at slot 312345678\n"),
            "{text}"
        );
        assert!(text.contains("  SOL        1.394995\n"), "{text}");
        assert!(
            text.contains(&format!(
                "    {MINT}  1234.5  bought by the sandbox (may always be sold)\n"
            )),
            "{text}"
        );
        assert!(text.contains("Positions (raw accounts)"), "{text}");
        assert!(text.contains("KLen…gmjD  obligation"), "{text}");
    }

    #[test]
    fn raw_is_dropped_unless_asked_for() {
        let page = fixture("history-page.json");
        assert!(page["items"][0].get("raw").is_some());
        assert!(ledger::without_raw(page)["items"][0].get("raw").is_none());
    }

    #[test]
    fn v3_rules_render_toggles_limits_and_the_allowlist() {
        colored::control::set_override(false);
        let text = render_rules(&fixture("rules-v3.json"));
        assert!(text.starts_with("Rules v3\n"), "{text}");
        assert!(text.contains(&format!("    on   Pump.fun  {PUMP}\n           instructions: buy, sell\n           spend limit: 1 SOL per day\n")), "{text}");
        assert!(text.contains("    off  Meteora DLMM"), "{text}");
        assert!(
            text.contains("instructions: the registry's safe set"),
            "{text}"
        );
        assert!(
            text.contains("9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM"),
            "{text}"
        );
    }

    #[test]
    fn earlier_agent_rules_still_render() {
        colored::control::set_override(false);
        let text = render_rules(&json!({
            "version": 2,
            "programs": [
                { "programId": PUMP, "name": "pump", "allowed": ["buy"],
                  "instructions": [{ "name": "buy", "args": [], "accounts": [] }] },
                { "programId": "S", "name": "spl", "allowed": "any", "instructions": [] }
            ],
            "baseline": [{ "name": "System" }, { "name": "Associated Token" }]
        }));
        assert!(text.contains("instructions: buy\n"), "{text}");
        assert!(text.contains("instructions: any\n"), "{text}");
        assert!(
            text.contains("Always allowed: System, Associated Token"),
            "{text}"
        );
        assert!(!text.contains("Transfers"), "{text}");
    }

    #[test]
    fn validation_reports_render_errors_and_warnings() {
        colored::control::set_override(false);
        let good = rules::validate(&fixture("rules-v3.json"));
        assert_eq!(
            render_report("rules.json", &good),
            "rules.json: valid Rules v3 (enabled: Pump.fun)\n"
        );
        let mut bad = fixture("rules-v3.json");
        bad["version"] = json!(4);
        bad["programs"][0]["spendLimit"]["windowSeconds"] = json!(0);
        let text = render_report("rules.json", &rules::validate(&bad));
        assert!(text.starts_with("rules.json: 2 errors\n"), "{text}");
        assert!(
            text.contains("  error  version: must be 3, not 4\n"),
            "{text}"
        );
        assert!(
            text.contains("  error  programs[0].spendLimit.windowSeconds:"),
            "{text}"
        );
    }

    #[test]
    fn the_next_page_keeps_the_resolved_cutoff() {
        let args = HistoryArgs {
            limit: ledger::DEFAULT_LIMIT,
            since: Some("24h".into()),
            ..Default::default()
        };
        let since = parse_since(
            "24h",
            DateTime::parse_from_rfc3339("2026-11-15T12:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
        )
        .unwrap();
        let text = render_history(
            &json!({ "items": [], "nextCursor": "c2" }),
            &args,
            Some(since),
            "s",
        );
        assert!(
            text.ends_with("More: a4 sandbox history --cursor c2 --since 2026-11-14T12:00:00Z\n"),
            "{text}"
        );
        assert!(!text.contains("24h"), "{text}");
    }

    #[test]
    fn next_page_flags_are_shell_safe() {
        let args = HistoryArgs {
            limit: ledger::DEFAULT_LIMIT,
            status: Some("it's".into()),
            program: Some("Pump.fun".into()),
            ..Default::default()
        };
        assert_eq!(
            repeat_flags(&args, None),
            " --status 'it'\\''s' --program Pump.fun"
        );
    }
}
