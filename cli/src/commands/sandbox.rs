//! `a4 sandbox`: read a sandbox wallet's history, holdings and rules, and
//! check a Rules v3 document before its owner signs it.
//!
//! The reads go to the Arete API's sandbox routes with a short-lived token
//! the account's API key mints for the sandbox (as the MCP `sandbox_*` tools
//! do), so they work for an agent the sandbox's owner has granted access.
//! They never run an action: actions go through the MCP `sandbox_invoke`
//! tool. `rules validate` reads a local file and sends nothing.

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
    /// The sandbox's actions, newest first (paginated, with filters)
    History(HistoryArgs),
    /// What the sandbox wallet holds now: SOL, tokens and program positions
    Holdings(HoldingsArgs),
    /// The sandbox's rules: show them, or check a Rules v3 file
    #[command(subcommand)]
    Rules(RulesCommands),
}

#[derive(Args, Debug, Clone, Default)]
pub struct SandboxTarget {
    /// The sandbox id (default: ARETE_SANDBOX_ID)
    #[arg(long, value_name = "ID")]
    pub sandbox: Option<String>,
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
        SandboxCommands::History(args) => history(args, json),
        SandboxCommands::Holdings(args) => holdings(args, json),
        SandboxCommands::Rules(RulesCommands::Show(target)) => rules_show(target, json),
        SandboxCommands::Rules(RulesCommands::Validate { file }) => rules_validate(&file, json),
    }
}

fn runtime() -> Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("Failed to start the async runtime")
}

fn client(target: &SandboxTarget) -> Result<SandboxClient> {
    let client = SandboxClient::from_env_for(target.sandbox.clone());
    if !client.configured() {
        anyhow::bail!(
            "No sandbox: pass --sandbox <id> or set ARETE_SANDBOX_ID to the sandbox id shown on the dashboard"
        );
    }
    Ok(client)
}

/// Print a refusal the way API errors are printed (the problem as JSON on
/// stdout with --json, else the message and a hint on stderr) and exit
/// with 77 for auth refusals, 75 when rate limited, else 1.
fn refused(refusal: SandboxRefusal, json: bool) -> anyhow::Error {
    let shown = refusal.to_json();
    if json {
        println!("{shown}");
    } else {
        let code = shown["code"].as_str().unwrap_or("sandbox-request-failed");
        let message = shown["error"]
            .as_str()
            .or_else(|| shown["message"].as_str())
            .unwrap_or("the Arete API refused the request");
        eprintln!("{} {message} ({code})", "Error:".red().bold());
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

// ---- history ------------------------------------------------------------------

fn history(args: HistoryArgs, json: bool) -> Result<()> {
    let client = client(&args.target)?;
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
    let runtime = runtime()?;
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
        print!(
            "{}",
            render_history(
                &body,
                &continuation(&args, query.since),
                client.sandbox_id().unwrap_or("?")
            )
        );
    }
    Ok(())
}

/// `args` for the next-page command: a relative `--since` (`24h`) is
/// repeated as the time it resolved to, so later pages keep the same cutoff.
fn continuation(args: &HistoryArgs, since: Option<DateTime<Utc>>) -> HistoryArgs {
    let mut next = args.clone();
    if let Some(since) = since {
        next.since = Some(since.to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
    }
    next
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

/// The human rendering of a history page.
pub fn render_history(body: &Value, args: &HistoryArgs, sandbox_id: &str) -> String {
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
            repeat_flags(args)
        ));
    }
    out
}

fn filtered(args: &HistoryArgs) -> bool {
    args.status.is_some() || args.program.is_some() || args.mint.is_some() || args.since.is_some()
}

/// The flags to repeat with `--cursor` for the next page.
fn repeat_flags(args: &HistoryArgs) -> String {
    let mut flags = String::new();
    if let Some(sandbox) = &args.target.sandbox {
        flags.push_str(&format!(" --sandbox {sandbox}"));
    }
    if args.limit != ledger::DEFAULT_LIMIT {
        flags.push_str(&format!(" --limit {}", args.limit));
    }
    for (flag, value) in [
        ("--status", &args.status),
        ("--program", &args.program),
        ("--mint", &args.mint),
        ("--since", &args.since),
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
    if let Some(reason) = item["rejectCode"]
        .as_str()
        .or_else(|| item["error"]["code"].as_str())
        .or_else(|| item["error"].as_str())
    {
        line.push_str(&format!("      {reason}\n"));
    }
    line
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
    let client = client(&args.target)?;
    let body = runtime()?
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
        print!(
            "{}",
            render_holdings(&body, client.sandbox_id().unwrap_or("?"))
        );
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
    let client = client(&target)?;
    let body = runtime()?
        .block_on(client.rules())
        .map_err(|refusal| refused(refusal, json))?;
    if json {
        println!("{body}");
    } else {
        print!("{}", render_rules(&body));
    }
    Ok(())
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
        let text = render_history(&fixture("history-page.json"), &args, "sbx");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "Sandbox sbx  history, newest first");
        assert!(
            lines[2].starts_with(
                "  2026-11-12 10:00:00  landed    Pump.fun.buy  -0.105005 SOL, +1234.5 7xKX…gAsU"
            ),
            "{text}"
        );
        assert!(
            lines[3].contains("rejected  ") && lines[3].contains("Pump.fun.sell"),
            "{text}"
        );
        assert!(lines[4].contains("sandbox-guard-trial-failed"), "{text}");
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
        let text = render_history(&json!({ "items": [], "nextCursor": null }), &args, "s");
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
        assert_eq!(
            repeat_flags(&continuation(&args, Some(since))),
            " --since 2026-11-14T12:00:00Z"
        );
    }

    #[test]
    fn next_page_flags_are_shell_safe() {
        let args = HistoryArgs {
            limit: ledger::DEFAULT_LIMIT,
            program: Some("Pump.fun".into()),
            since: Some("it's".into()),
            ..Default::default()
        };
        assert_eq!(
            repeat_flags(&args),
            " --program Pump.fun --since 'it'\\''s'"
        );
    }
}
