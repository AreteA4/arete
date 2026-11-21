//! `a4 sandbox`: list the sandboxes the account has been granted and choose
//! one, read a sandbox wallet's status (address, balances, granted actions),
//! history, holdings and rules, list the programs it can read, and check a
//! Rules v3 document before its owner signs it.
//!
//! The reads go to the Arete API's sandbox routes with a short-lived token
//! the account's API key mints for the sandbox (as the MCP `sandbox_*` tools
//! do), so they work for an agent the sandbox's owner has granted access.
//! The sandbox resolves as for the MCP tools: `--sandbox`, then the one
//! chosen with `a4 sandbox use` for this directory, then `ARETE_SANDBOX_ID`,
//! then the only active sandbox granted.
//!
//! `preview` and `invoke` are the one-shot forms of the MCP `sandbox_preview`
//! and `sandbox_invoke` tools, for hosts that cannot run a stdio MCP server:
//! the same `invoke`, `call` and `withdraw` actions, sent to the same routes
//! through the same client, with the same results and refusals (`--json`
//! prints them as the tools return them). `action` looks one up by id or
//! idempotency key, as `sandbox_action` does. `rules validate` reads a
//! local file and sends nothing.

use std::io::Read;
use std::path::PathBuf;

use anyhow::{Context, Result};
use arete_mcp::sandbox::{
    ledger, rules, ActionLookup, SandboxActionName, SandboxClient, SandboxRefusal,
};
use chrono::{DateTime, Duration, Utc};
use clap::{ArgGroup, Args, Subcommand, ValueEnum};
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
    /// The sandbox wallet: address, SOL and token balances, and the actions
    /// this agent is granted
    Status(SandboxTarget),
    /// The sandbox's actions, newest first (paginated, with filters)
    History(HistoryArgs),
    /// What the sandbox wallet holds now: SOL, tokens and program positions
    Holdings(HoldingsArgs),
    /// The sandbox's rules: show them, or check a Rules v3 file
    ///
    /// Rules are built and signed in the dashboard (Sandboxes → Rules,
    /// https://app.arete.run/dashboard/sandboxes); the CLI only shows and
    /// checks them.
    #[command(subcommand)]
    Rules(RulesCommands),
    /// The programs the sandbox can read: their accounts with `a4 program
    /// account decode` and their stacks with `a4 get` / `a4 stream`, each with
    /// --sandbox
    Reads(SandboxTarget),
    /// Dry-run one action: every check, nothing signed or recorded
    ///
    /// Builds the transaction and runs every check without signing, sending
    /// or recording anything. Takes the action and arguments `a4 sandbox
    /// invoke` takes: --call (a function `a4 sandbox rules show` lists),
    /// --invoke (named program instructions) or --withdraw (SOL to the
    /// owner). Prints whether it passes and what the wallet would lose, or
    /// the refusal with its code, rule, requestId and, for a call, the
    /// stages it got through. Exits 1 when it does not pass. --json prints
    /// the result as the MCP `sandbox_preview` tool returns it.
    ///
    /// Examples:
    ///   a4 sandbox preview --call <program> transactions.obligation.deposit \
    ///     --input '{"reserve":"<reserve>","amount":{"ui":"0.1"}}'
    ///   a4 sandbox preview --call <program> transactions.obligation.deposit --input @deposit.json
    ///   a4 sandbox preview --invoke @instructions.json
    ///   a4 sandbox preview --withdraw 50000000
    #[command(verbatim_doc_comment)]
    Preview(ActionArgs),
    /// Run one action from the sandbox wallet and wait for it to confirm
    ///
    /// Builds, checks, signs and sends the transaction. Takes the same
    /// action flags as `a4 sandbox preview`: preview first. Every run has an
    /// idempotency key (--idempotency-key, else generated), printed on
    /// stderr before the run is sent and again with its result or refusal,
    /// so an interrupted run can still be retried or looked up. The same request under the same key never
    /// runs twice and returns the original result (`replayed: true`), so
    /// after a timeout retry with the same key, never a new one, or look the
    /// action up with `a4 sandbox action --idempotency-key`. Exits 1 when its
    /// transaction failed; a refusal exits 77 (not granted or not signed
    /// in), 75 (rate limited) or 1. --json prints the result as the MCP
    /// `sandbox_invoke` tool returns it.
    ///
    /// Examples:
    ///   a4 sandbox invoke --call <program> transactions.obligation.deposit --input @deposit.json
    ///   a4 sandbox invoke --withdraw 50000000 --idempotency-key withdraw-2026-11-21
    #[command(verbatim_doc_comment)]
    Invoke(InvokeArgs),
    /// Look one action up by its id or the idempotency key it ran under
    ///
    /// Use it after a timeout instead of retrying under a new key: status,
    /// reject code, signature, what left the wallet and the guard pipeline.
    Action(ActionLookupArgs),
}

/// One action, as `a4 sandbox preview` and `a4 sandbox invoke` take it:
/// exactly one of --call, --invoke and --withdraw.
#[derive(Args, Debug, Clone, Default)]
#[command(group(
    ArgGroup::new("sandbox_action")
        .required(true)
        .args(["call", "invoke", "withdraw"])
))]
pub struct ActionArgs {
    #[command(flatten)]
    pub target: SandboxTarget,

    /// Run a function a program offers: the program and the function, as
    /// `a4 sandbox rules show` lists them with each function's input. Arete
    /// plans the instructions and fills the wallet and accounts.
    #[arg(long, num_args = 2, value_names = ["PROGRAM", "FUNCTION"])]
    pub call: Option<Vec<String>>,

    /// The --call function's input: JSON, @FILE, or - for stdin
    #[arg(
        long,
        value_name = "JSON|@FILE|-",
        requires = "call",
        conflicts_with_all = ["invoke", "withdraw"]
    )]
    pub input: Option<String>,

    /// Run named program instructions: the invoke arguments as JSON
    /// (`{"instructions":[{program, instruction, args?, accounts?}]}`, or the
    /// instructions array alone), @FILE, or - for stdin
    #[arg(long, value_name = "JSON|@FILE|-")]
    pub invoke: Option<String>,

    /// Send SOL to the owner's withdraw address (the only destination a
    /// withdraw can have): the amount in lamports
    #[arg(long, value_name = "LAMPORTS")]
    pub withdraw: Option<u64>,

    /// The transaction format (default: the API's, Solana v1)
    #[arg(long, value_enum)]
    pub transaction_format: Option<TransactionFormat>,

    /// The compute unit limit for --call or --invoke
    #[arg(long, value_name = "UNITS", conflicts_with = "withdraw")]
    pub compute_unit_limit: Option<u32>,

    /// The compute unit price for --call or --invoke, in micro-lamports
    #[arg(long, value_name = "MICRO_LAMPORTS", conflicts_with = "withdraw")]
    pub compute_unit_price: Option<u64>,
}

#[derive(Args, Debug, Clone, Default)]
pub struct InvokeArgs {
    #[command(flatten)]
    pub action: ActionArgs,

    /// Your key for this action (1 to 128 characters). Generated when left
    /// out; either way it is printed. Retry a timed-out run with the same key.
    #[arg(long, value_name = "KEY")]
    pub idempotency_key: Option<String>,
}

#[derive(Args, Debug, Clone, Default)]
#[command(group(
    ArgGroup::new("lookup")
        .required(true)
        .args(["action_id", "idempotency_key"])
))]
pub struct ActionLookupArgs {
    #[command(flatten)]
    pub target: SandboxTarget,

    /// The action's id, as `a4 sandbox invoke` printed it
    #[arg(value_name = "ACTION_ID")]
    pub action_id: Option<String>,

    /// Or the idempotency key it ran under
    #[arg(long, value_name = "KEY")]
    pub idempotency_key: Option<String>,
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionFormat {
    /// Solana v1 transactions: up to 4096 bytes and 64 accounts
    V1,
    /// The older v0 format: up to 1232 bytes
    V0,
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
    ///
    /// Rules are built and signed in the dashboard (Sandboxes → Rules,
    /// https://app.arete.run/dashboard/sandboxes); the CLI only shows and
    /// checks them. Only version 3 is supported.
    Validate {
        /// The rules file, or `-` for stdin
        file: PathBuf,
    },
}

pub fn run(command: SandboxCommands, config: &str, json: bool) -> Result<()> {
    match command {
        SandboxCommands::List => list(json),
        SandboxCommands::Use(args) => use_sandbox(args, json),
        SandboxCommands::Status(target) => status(target, json),
        SandboxCommands::History(args) => history(args, json),
        SandboxCommands::Holdings(args) => holdings(args, json),
        SandboxCommands::Rules(RulesCommands::Show(target)) => rules_show(target, json),
        SandboxCommands::Rules(RulesCommands::Validate { file }) => rules_validate(&file, json),
        SandboxCommands::Reads(target) => reads_show(target, config, json),
        SandboxCommands::Preview(args) => preview(args, json),
        SandboxCommands::Invoke(args) => invoke(args, json),
        SandboxCommands::Action(args) => action_lookup(args, json),
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
    let requested = requested.map(str::trim).filter(|wanted| !wanted.is_empty());
    if requested.is_none() && !reads_through_sandbox(usual, config) {
        return None;
    }
    Some(SandboxClient::from_env().targeting(requested))
}

/// Whether a read command whose own groups are `usual` goes through the
/// sandbox without --sandbox: the account's groups grant `sandboxes` and
/// none of them.
fn reads_through_sandbox(usual: &[arete_mcp::entitlements::Group], config: &str) -> bool {
    let surface = crate::entitlements::selected_account_surface(config);
    sandbox_only(&surface, usual)
}

/// Whether `surface` grants `sandboxes` and none of `usual`.
fn sandbox_only(
    surface: &arete_mcp::entitlements::Surface,
    usual: &[arete_mcp::entitlements::Group],
) -> bool {
    use arete_mcp::entitlements::Group;
    surface.allows(Group::Sandboxes) && !usual.iter().any(|group| surface.allows(*group))
}

/// A sandbox refusal of a read, printed like the other sandbox refusals.
pub fn read_refused(refusal: SandboxRefusal, json: bool) -> anyhow::Error {
    refused(refusal, json)
}

/// One line for a terminal, with the hint in CLI terms.
pub fn cli_message(refusal: &SandboxRefusal) -> String {
    refusal.message_with_hint(cli_hint(refusal))
}

/// For a key a sandbox grant binds, refused a direct read with `refusal`
/// (`sandbox-read-required`): a client pinned to the sandbox to read
/// through, the one chosen with `a4 sandbox use` (else `ARETE_SANDBOX_ID`,
/// else the only one the refusal lists). Else the refusal, printed with the
/// sandboxes it lists.
pub fn bound_reader(refusal: &SandboxRefusal, json: bool) -> Result<SandboxClient> {
    let (client, _) = SandboxClient::from_env()
        .read_through_bound(refusal)
        .map_err(|refusal| refused(refusal, json))?;
    Ok(client)
}

/// The refusal in `error` of a direct read by a key a sandbox grant binds
/// (`sandbox-read-required`), from the API client or a stream's mint.
pub fn read_required(error: &anyhow::Error) -> Option<SandboxRefusal> {
    if let Some(refusal) = error.downcast_ref::<SandboxRefusal>() {
        return (refusal.code() == arete_mcp::sandbox::READ_REQUIRED).then(|| refusal.clone());
    }
    let api = error.downcast_ref::<crate::api_client::ApiClientError>()?;
    SandboxRefusal::read_required(
        api.status.as_u16(),
        serde_json::to_value(&api.problem).ok()?,
    )
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
    resolved_client(runtime, target, json).map(|(client, resolved)| (client, resolved.sandbox_id))
}

/// A client for the sandbox `target` resolves to, and how it resolved.
fn resolved_client(
    runtime: &tokio::runtime::Runtime,
    target: &SandboxTarget,
    json: bool,
) -> Result<(SandboxClient, arete_mcp::sandbox::selection::Resolved)> {
    let client = SandboxClient::from_env().targeting(target.sandbox.as_deref());
    let resolved = runtime
        .block_on(client.resolve())
        .map_err(|refusal| refused(refusal, json))?;
    // Resolved once: the calls below name the id, with no second listing.
    let client = client.targeting(Some(&resolved.sandbox_id));
    Ok((client, resolved))
}

/// The hint for `refusal` in CLI terms, where the MCP hint names MCP tools.
pub fn cli_hint(refusal: &SandboxRefusal) -> Option<&'static str> {
    let read = refusal.kind == arete_mcp::sandbox::CallKind::Read;
    Some(match refusal.code().as_str() {
        "sandbox-program-not-allowlisted" if read => {
            "The sandbox can read only the programs in its rules: `a4 sandbox reads` lists them. Only the owner can add one, in the dashboard."
        }
        "sandbox-read-programs-unknown" => {
            "Arete could not tell which programs this stack decodes, so a sandbox cannot read it. Read the programs the sandbox allows (`a4 sandbox reads` lists them) with `a4 program account decode`."
        }
        "sandbox-read-target-unsupported" => {
            "A sandbox reads only program accounts (`a4 program account decode`) and stack views (`a4 get`, `a4 stream`) of the programs `a4 sandbox reads` lists."
        }
        arete_mcp::sandbox::READ_REQUIRED => {
            "This API key reads programs and stacks only through a sandbox. Choose one with `a4 sandbox use <id|name>` (remembered for this directory), or pass --sandbox <id>."
        }
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

/// `hint` with the MCP tool names it mentions replaced by the `a4 sandbox`
/// commands that do the same.
pub fn cli_terms(hint: &str) -> String {
    // Longest first, so a name is never matched inside a longer one.
    const TOOLS: [(&str, &str); 10] = [
        ("sandbox_holdings", "`a4 sandbox holdings`"),
        ("sandbox_history", "`a4 sandbox history`"),
        ("sandbox_preview", "`a4 sandbox preview`"),
        ("sandbox_invoke", "`a4 sandbox invoke`"),
        ("sandbox_action", "`a4 sandbox action`"),
        ("sandbox_status", "`a4 sandbox status`"),
        ("sandbox_select", "`a4 sandbox use`"),
        ("sandbox_rules", "`a4 sandbox rules show`"),
        ("sandbox_reads", "`a4 sandbox reads`"),
        ("sandbox_list", "`a4 sandbox list`"),
    ];
    let mut out = String::with_capacity(hint.len());
    let mut rest = hint;
    'scan: while !rest.is_empty() {
        let at_word_start = !out
            .chars()
            .last()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
        if at_word_start {
            for (tool, command) in TOOLS {
                // A tool name, alone or in backticks, not part of a longer word.
                for quoted in [true, false] {
                    let name = if quoted {
                        format!("`{tool}`")
                    } else {
                        tool.to_string()
                    };
                    let Some(after) = rest.strip_prefix(name.as_str()) else {
                        continue;
                    };
                    if !quoted
                        && after
                            .chars()
                            .next()
                            .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
                    {
                        continue;
                    }
                    out.push_str(command);
                    rest = after;
                    continue 'scan;
                }
            }
        }
        let next = rest.chars().next().unwrap_or_default();
        out.push(next);
        rest = &rest[next.len_utf8()..];
    }
    out.replace("an idempotencyKey", "an --idempotency-key")
        .replace("a new idempotencyKey", "a new --idempotency-key")
        .replace("the idempotencyKey", "the --idempotency-key")
}

/// Print a refusal the way API errors are printed (the problem as JSON on
/// stdout with --json, else the message and a hint on stderr) and exit
/// with 77 for auth refusals, 75 when rate limited, else 1.
fn refused(refusal: SandboxRefusal, json: bool) -> anyhow::Error {
    let shown = shown_refusal(&refusal);
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
    refusal_exit(&refusal)
}

/// The refusal as the MCP tools return it, with its hint in CLI terms.
fn shown_refusal(refusal: &SandboxRefusal) -> Value {
    let mut shown = refusal.to_json();
    let hint = cli_hint(refusal)
        .map(str::to_string)
        .or_else(|| shown["hint"].as_str().map(cli_terms));
    if let Some(hint) = hint {
        shown["hint"] = json!(hint);
    }
    shown
}

/// 77 for auth refusals, 75 when rate limited, else 1.
fn refusal_exit(refusal: &SandboxRefusal) -> anyhow::Error {
    let code = match refusal.status {
        401 | 403 => 77,
        429 => 75,
        _ => 1,
    };
    ExitCode(code).into()
}

// ---- preview, invoke and action ----------------------------------------------

impl ActionArgs {
    /// The action and its `args`, as the MCP tools take them.
    pub fn action(&self) -> Result<(SandboxActionName, Value)> {
        let (action, mut args) = if let Some(call) = &self.call {
            let [program, function] = call.as_slice() else {
                anyhow::bail!("--call takes the program and the function");
            };
            let mut args = json!({ "program": program, "function": function });
            if let Some(input) = &self.input {
                args["input"] = json_argument("--input", input)?;
            }
            (SandboxActionName::Call, args)
        } else if let Some(invoke) = &self.invoke {
            let args = match json_argument("--invoke", invoke)? {
                // The instructions alone.
                Value::Array(instructions) => json!({ "instructions": instructions }),
                Value::Object(args) => Value::Object(args),
                other => anyhow::bail!(
                    "--invoke takes a JSON object (`{{\"instructions\": [...]}}`) or an array of instructions, not {other}"
                ),
            };
            (SandboxActionName::Invoke, args)
        } else if let Some(lamports) = self.withdraw {
            // u64 amounts go as decimal strings.
            (
                SandboxActionName::Withdraw,
                json!({ "lamports": lamports.to_string() }),
            )
        } else {
            anyhow::bail!("give one of --call, --invoke or --withdraw");
        };
        let object = args.as_object_mut().expect("action args are an object");
        if let Some(format) = self.transaction_format {
            let format = match format {
                TransactionFormat::V1 => "v1",
                TransactionFormat::V0 => "v0",
            };
            object.insert("transactionFormat".into(), json!(format));
        }
        if let Some(limit) = self.compute_unit_limit {
            object.insert("computeUnitLimit".into(), json!(limit));
        }
        if let Some(price) = self.compute_unit_price {
            object.insert(
                "computeUnitPriceMicroLamports".into(),
                json!(price.to_string()),
            );
        }
        Ok((action, args))
    }
}

/// A JSON flag value: inline JSON, `@FILE`, or `-` for stdin.
fn json_argument(flag: &str, value: &str) -> Result<Value> {
    let (text, source) = if value == "-" {
        let mut text = String::new();
        std::io::stdin()
            .read_to_string(&mut text)
            .with_context(|| format!("Failed to read {flag} from stdin"))?;
        (text, "stdin".to_string())
    } else if let Some(path) = value.strip_prefix('@') {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read {flag} from {path}"))?;
        (text, path.to_string())
    } else {
        (value.to_string(), "the argument".to_string())
    };
    serde_json::from_str(&text).with_context(|| format!("{flag}: {source} is not valid JSON"))
}

/// A fresh idempotency key for a run that was not given one.
fn new_cli_idempotency_key() -> String {
    format!("a4-{}", uuid::Uuid::new_v4())
}

fn preview(args: ActionArgs, json: bool) -> Result<()> {
    let (action, body) = args.action()?;
    let client = SandboxClient::from_env().targeting(args.target.sandbox.as_deref());
    let result = runtime()?.block_on(client.preview(action, body));
    let body = result.map_err(|refusal| action_refused(&refusal, None, json))?;
    let passed = body["pass"] != false;
    if json {
        println!("{body}");
    } else {
        print!("{}", render_preview(&body, action));
    }
    if passed {
        Ok(())
    } else {
        Err(ExitCode(1).into())
    }
}

fn invoke(args: InvokeArgs, json: bool) -> Result<()> {
    let (action, body) = args.action.action()?;
    let key = args
        .idempotency_key
        .clone()
        .unwrap_or_else(new_cli_idempotency_key);
    // The key goes to stderr before anything is sent: a run interrupted
    // while it waits (Ctrl-C, a host's timeout) may still have sent the
    // transaction, and the key is how to retry it safely or look it up.
    // stdout stays the result alone.
    eprintln!("{}", key_notice(&key));
    let client = SandboxClient::from_env().targeting(args.action.target.sandbox.as_deref());
    let result = runtime()?.block_on(client.invoke(action, body, &key));
    let body = result.map_err(|refusal| action_refused(&refusal, Some(&key), json))?;
    let body = arete_mcp::sandbox::with_idempotency_key(body, Some(&key));
    if json {
        println!("{body}");
    } else {
        print!("{}", render_invoke(&body, action));
    }
    if body["status"] == "failed" {
        Err(ExitCode(1).into())
    } else {
        Ok(())
    }
}

/// What `a4 sandbox invoke` prints on stderr before it sends a run.
pub fn key_notice(key: &str) -> String {
    let key = shell_word(key);
    format!(
        "Idempotency key: {key} (if this run is interrupted, rerun it with --idempotency-key {key}, or look it up with `a4 sandbox action --idempotency-key {key}`; never rerun it under a new key)"
    )
}

fn action_lookup(args: ActionLookupArgs, json: bool) -> Result<()> {
    let lookup = match (args.action_id, args.idempotency_key) {
        (Some(id), _) => ActionLookup::Id(id),
        (None, Some(key)) => ActionLookup::IdempotencyKey(key),
        (None, None) => anyhow::bail!("give the action id or --idempotency-key"),
    };
    let client = SandboxClient::from_env().targeting(args.target.sandbox.as_deref());
    let body = runtime()?
        .block_on(client.action(&lookup))
        .map_err(|refusal| action_refused(&refusal, None, json))?;
    if json {
        println!("{body}");
    } else {
        print!("{}", render_action(&body));
    }
    Ok(())
}

/// Print an action's refusal: with --json, the refusal as the MCP tools
/// return it (and `idempotencyKey` for a run); else the message, what
/// refused it (rule, requestId, the stages a call got through, the guard
/// pipeline), the key and the hint, on stderr.
fn action_refused(
    refusal: &SandboxRefusal,
    idempotency_key: Option<&str>,
    json: bool,
) -> anyhow::Error {
    let shown = arete_mcp::sandbox::with_idempotency_key(shown_refusal(refusal), idempotency_key);
    if json {
        println!("{shown}");
    } else {
        eprint!("{}", render_refusal(&shown));
    }
    refusal_exit(refusal)
}

/// The human rendering of an action's refusal, `shown` as
/// [`shown_refusal`] gives it.
pub fn render_refusal(shown: &Value) -> String {
    let code = shown["code"].as_str().unwrap_or("sandbox-request-refused");
    let message = shown["error"]
        .as_str()
        .or_else(|| shown["message"].as_str())
        .unwrap_or("the Arete API refused the request");
    let mut out = format!("{} {message} ({code})\n", "Error:".red().bold());
    let row = |out: &mut String, label: &str, value: &str| {
        out.push_str(&format!("  {label:<16}{value}\n"));
    };
    for (label, key) in [
        ("Rule", "rule"),
        ("Program", "program"),
        ("Instruction", "instruction"),
        ("Mint", "mint"),
    ] {
        if let Some(value) = shown[key].as_str() {
            row(&mut out, label, value);
        }
    }
    if let Some(path) = shown["path"].as_u64() {
        row(&mut out, "Instruction #", &path.to_string());
    }
    for (label, key) in [
        ("Attempted", "attempted"),
        ("Cap", "cap"),
        ("Remaining", "remaining"),
    ] {
        if let Some(value) = amount(&shown[key]) {
            row(&mut out, label, &value.to_string());
        }
    }
    if let Some(stages) = shown["stages"].as_array() {
        let names: Vec<String> = stages
            .iter()
            .map(|stage| {
                let name = stage["name"]
                    .as_str()
                    .or_else(|| stage["stage"].as_str())
                    .or_else(|| stage.as_str())
                    .unwrap_or("?");
                match stage["code"].as_str() {
                    Some(code) => format!("{name} ({code})"),
                    None => name.to_string(),
                }
            })
            .collect();
        row(&mut out, "Stages", &names.join(" → "));
    }
    if let Some(stage) = shown["failedStage"].as_str() {
        let reason = shown["stages"]
            .as_array()
            .and_then(|stages| stages.last())
            .and_then(|stage| stage["reason"].as_str());
        match reason {
            Some(reason) => row(&mut out, "Refused at", &format!("{stage}: {reason}")),
            None => row(&mut out, "Refused at", stage),
        }
    }
    if let Some(pipeline) = shown["guardPipeline"].as_str() {
        row(&mut out, "Guard pipeline", pipeline);
    }
    if let Some(change) = amount(&shown["walletLamportsChange"]) {
        row(
            &mut out,
            "SOL change",
            &format!("{} SOL", signed_units(change, 9)),
        );
    }
    if shown["replayed"] == true {
        row(
            &mut out,
            "Replayed",
            "yes: the recorded answer for this key",
        );
    }
    if let Some(request) = shown["requestId"].as_str() {
        row(&mut out, "Request id", request);
    }
    if let Some(key) = shown["idempotencyKey"].as_str() {
        row(&mut out, "Idempotency key", key);
    }
    if let Some(hint) = shown["hint"].as_str() {
        out.push_str(hint);
        out.push('\n');
    }
    out
}

/// The wallet's change, what it spent and the fees, as preview and invoke
/// results report them.
fn wallet_rows(out: &mut String, body: &Value) {
    let row = |out: &mut String, label: &str, value: &str| {
        out.push_str(&format!("  {label:<16}{value}\n"));
    };
    let sol = |raw: i128| format!("{} SOL", units(raw, 9));
    if let Some(change) = amount(&body["walletLamportsChange"]) {
        row(
            out,
            "SOL change",
            &format!("{} SOL", signed_units(change, 9)),
        );
    }
    if let Some(spent) = amount(&body["outflowLamports"]) {
        row(out, "Spent", &sol(spent));
    }
    if let Some(fee) = amount(&body["networkFeeLamports"]) {
        row(out, "Network fee", &format!("{} (included)", sol(fee)));
    }
    if let Some(fee) = amount(&body["feeLamports"]).filter(|fee| *fee != 0) {
        row(out, "Protocol fees", &sol(fee));
    }
    if let Some(rent) = amount(&body["newAccountRentLamports"]).filter(|rent| *rent != 0) {
        let count = body["newAccounts"].as_array().map_or(0, Vec::len);
        row(
            out,
            "New account rent",
            &format!("{} ({count} accounts)", sol(rent)),
        );
    }
    if let Some(rent) = amount(&body["closedAccountRentLamports"]).filter(|rent| *rent != 0) {
        row(out, "Rent refunded", &sol(rent));
    }
    match &body["tokenOutflow"] {
        Value::Object(mints) => {
            for (mint, raw) in mints {
                if let Some(raw) = amount(raw) {
                    row(out, "Token out", &format!("{mint}  {raw} (raw)"));
                }
            }
        }
        Value::Array(entries) => {
            for entry in entries {
                if let (Some(mint), Some(raw)) = (
                    entry["mint"].as_str(),
                    amount(&entry["amount"]).or_else(|| amount(&entry["raw"])),
                ) {
                    row(out, "Token out", &format!("{mint}  {raw} (raw)"));
                }
            }
        }
        _ => {}
    }
    if let Some(pipeline) = body["guardPipeline"].as_str() {
        row(out, "Guard pipeline", pipeline);
    }
}

/// What the action names, for a heading: the function, the instructions or
/// the amount.
fn action_label(action: SandboxActionName, body: &Value) -> String {
    let function = body["plan"]["function"].as_str();
    match (action, function) {
        (SandboxActionName::Call, Some(function)) => format!("call {function}"),
        _ => action.as_str().to_string(),
    }
}

/// The human rendering of `a4 sandbox preview`.
pub fn render_preview(body: &Value, action: SandboxActionName) -> String {
    let passed = body["pass"] != false;
    let verdict = if passed {
        "passes".green().bold()
    } else {
        "does not pass".red().bold()
    };
    let mut out = format!(
        "{} {}: {verdict}\n",
        "Preview".bold(),
        action_label(action, body)
    );
    wallet_rows(&mut out, body);
    if let Some(instructions) = body["plan"]["instructions"].as_array() {
        out.push_str("  Plan\n");
        for (index, instruction) in instructions.iter().enumerate() {
            let program = instruction["program"]
                .as_str()
                .or_else(|| instruction["programId"].as_str())
                .unwrap_or("?");
            let name = instruction["instruction"]
                .as_str()
                .or_else(|| instruction["name"].as_str())
                .unwrap_or("?");
            out.push_str(&format!("    {index:>2}. {}  {name}\n", short(program)));
        }
    }
    if passed {
        out.push_str(
            "Nothing was signed or sent. Run it with `a4 sandbox invoke` and the same flags.\n",
        );
    } else if let Some(reason) = body["reason"].as_str().or_else(|| body["error"].as_str()) {
        out.push_str(&format!("{reason}\n"));
    }
    out
}

/// The human rendering of `a4 sandbox invoke`.
pub fn render_invoke(body: &Value, action: SandboxActionName) -> String {
    let status = body["status"].as_str().unwrap_or("?");
    let mut out = format!(
        "{} {}: {}\n",
        "Action".bold(),
        action_label(action, body),
        status_word(status)
    );
    let row = |out: &mut String, label: &str, value: &str| {
        out.push_str(&format!("  {label:<16}{value}\n"));
    };
    if let Some(id) = body["actionId"].as_str().or_else(|| body["id"].as_str()) {
        row(&mut out, "Action id", id);
    }
    if let Some(signature) = body["signature"].as_str() {
        row(&mut out, "Signature", signature);
    }
    wallet_rows(&mut out, body);
    if body["replayed"] == true {
        row(
            &mut out,
            "Replayed",
            "yes: the original result of this idempotency key",
        );
    }
    if let Some(key) = body["idempotencyKey"].as_str() {
        row(&mut out, "Idempotency key", key);
    }
    match status {
        "sent" => out.push_str(&format!(
            "Not confirmed yet. Look it up with `a4 sandbox action --idempotency-key {}`; never rerun it under a new key.\n",
            shell_word(body["idempotencyKey"].as_str().unwrap_or("<key>"))
        )),
        "failed" => out.push_str("The transaction landed but failed: nothing it did took effect beyond the network fee.\n"),
        _ => {}
    }
    out
}

/// An action's status, colored as in the history, unpadded.
fn status_word(status: &str) -> String {
    match status {
        "landed" => status.green().to_string(),
        "failed" | "rejected" => status.red().to_string(),
        _ => status.yellow().to_string(),
    }
}

/// The human rendering of `a4 sandbox action`.
pub fn render_action(body: &Value) -> String {
    let status = body["status"].as_str().unwrap_or("?");
    let id = body["id"]
        .as_str()
        .or_else(|| body["actionId"].as_str())
        .unwrap_or("?");
    let mut out = format!("{} {id}: {}\n", "Action".bold(), status_word(status));
    let row = |out: &mut String, label: &str, value: &str| {
        out.push_str(&format!("  {label:<16}{value}\n"));
    };
    for (label, key) in [
        ("Action", "action"),
        ("Created", "createdAt"),
        ("Reject code", "rejectCode"),
        ("Signature", "signature"),
    ] {
        if let Some(value) = body[key].as_str() {
            row(&mut out, label, value);
        }
    }
    wallet_rows(&mut out, body);
    if let Some(key) = body["idempotencyKey"].as_str() {
        row(&mut out, "Idempotency key", key);
    }
    out
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
        "bound-key" => "the only sandbox this API key reads through",
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

// ---- status -------------------------------------------------------------------

fn status(target: SandboxTarget, json: bool) -> Result<()> {
    let runtime = runtime()?;
    let client = SandboxClient::from_env().targeting(target.sandbox.as_deref());
    let body = runtime
        .block_on(client.status())
        .map_err(|refusal| refused(refusal, json))?;
    if json {
        println!("{body}");
        return Ok(());
    }
    // The wallet does not carry the sandbox's name; the listing does. Not
    // worth failing over: an API without the listing shows the id alone.
    let name = body["selection"]["sandboxId"]
        .as_str()
        .or_else(|| body["sandboxId"].as_str())
        .and_then(|id| {
            let grants = runtime.block_on(client.granted(false)).ok()?;
            grants
                .into_iter()
                .find(|grant| grant.sandbox_id == id)
                .and_then(|grant| grant.name)
        });
    print!("{}", render_status(&body, name.as_deref()));
    Ok(())
}

/// The human rendering of `a4 sandbox status`: the sandbox and how it was
/// chosen, the wallet, its balances and the granted actions. `name` is the
/// sandbox's name, when the listing gives one.
pub fn render_status(body: &Value, name: Option<&str>) -> String {
    let id = body["selection"]["sandboxId"]
        .as_str()
        .or_else(|| body["sandboxId"].as_str())
        .unwrap_or("?");
    let mut out = format!("{} {id}", "Sandbox".bold());
    if let Some(name) = name {
        out.push_str(&format!("  {}", name.bold()));
    }
    if let Some(source) = body["selection"]["source"].as_str() {
        out.push_str(&format!("  ({})", source_label(source)));
    }
    out.push_str("\n\n");
    let row = |out: &mut String, label: &str, value: &str| {
        out.push_str(&format!("  {label:<11}{value}\n"));
    };
    row(
        &mut out,
        "Wallet",
        body["walletAddress"].as_str().unwrap_or("?"),
    );
    if let Some(cluster) = body["cluster"].as_str() {
        let mut shown = cluster.to_string();
        // A dev environment's wallet holds forked, free funds.
        if let Some(environment) = body["environment"]
            .as_str()
            .filter(|environment| *environment != "mainnet")
        {
            shown.push_str(&format!(" ({environment})"));
        }
        row(&mut out, "Cluster", &shown);
    }
    let sol = amount(&body["balanceLamports"])
        .map(|lamports| units(lamports, 9))
        .unwrap_or_else(|| "?".into());
    row(&mut out, "SOL", &sol);

    let tokens = body["tokens"].as_array().cloned().unwrap_or_default();
    if tokens.is_empty() {
        row(&mut out, "Tokens", "none");
    } else {
        out.push_str("  Tokens\n");
        for token in &tokens {
            let held = match amount(&token["amount"]) {
                Some(raw) => match token["decimals"].as_u64() {
                    Some(decimals) => units(raw, decimals as u32),
                    None => format!("{raw} (raw)"),
                },
                None => "?".into(),
            };
            out.push_str(&format!(
                "    {}  {held}\n",
                token["mint"].as_str().unwrap_or("?")
            ));
        }
    }

    if let Some(actions) = body["actions"].as_array() {
        let actions: Vec<&str> = actions.iter().filter_map(Value::as_str).collect();
        let actions = if actions.is_empty() {
            "none".to_string()
        } else {
            actions.join(", ")
        };
        row(&mut out, "Actions", &actions);
    }
    if let Some(address) = body["withdrawAddress"].as_str() {
        row(&mut out, "Withdraws", &format!("to {address}"));
    }
    if let Some(revision) = body["rulesRevision"].as_u64() {
        row(&mut out, "Rules", &format!("revision {revision}"));
    }
    if let Some(fee) = amount(&body["feeLamports"]).filter(|fee| *fee != 0) {
        row(
            &mut out,
            "Arete fee",
            &format!("{} SOL per action", units(fee, 9)),
        );
    }
    if let Some(expires) = body["sandboxExpiresAt"].as_str() {
        row(&mut out, "Expires", expires);
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
    // The network fee, already part of the SOL change (and of
    // `outflowLamports`). It is null until the landing is read, and older
    // servers do not send it: then nothing is shown. `feeLamports` is the
    // protocol's own fees, which history items do not carry.
    if let Some(fee) = amount(&item["networkFeeLamports"]).filter(|fee| *fee != 0) {
        changes.push(format!("network fee {} SOL", units(fee, 9)));
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
    // Null or absent for actions recorded before it was kept.
    if let Some(pipeline) = item["guardPipeline"].as_str() {
        line.push_str(&format!("  {}", format!("guard {pipeline}").dimmed()));
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
        out.push_str("  Positions\n");
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
            let mut line = format!(
                "    {program}  {}  {}",
                position["label"].as_str().unwrap_or("account"),
                position["account"].as_str().unwrap_or("?")
            );
            let obligation = &position["obligation"];
            if let Some(market) = obligation["lendingMarket"].as_str() {
                line.push_str(&format!("  market {}", short(market)));
            }
            out.push_str(&line);
            out.push('\n');
            if obligation.is_object() {
                out.push_str(&obligation_lines(obligation));
            }
        }
    }
    out
}

/// A decoded lending obligation: one line per deposit (the cToken amount,
/// and what it is worth in the underlying token at the reserve's rate when
/// the reserve could be read) and one per borrow. Amounts are in whole units
/// when the decimals are known, else raw.
fn obligation_lines(obligation: &Value) -> String {
    let deposits = obligation["deposits"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let borrows = obligation["borrows"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if deposits.is_empty() && borrows.is_empty() {
        return "      no deposits or borrows\n".into();
    }
    let mut out = String::new();
    for deposit in &deposits {
        let decimals = deposit["underlyingDecimals"].as_u64().map(|d| d as u32);
        let mut line = format!(
            "      deposit  {}  {} cToken",
            short(deposit["reserve"].as_str().unwrap_or("?")),
            amount_text(&deposit["cTokenAmount"], decimals)
        );
        if !deposit["underlyingAmount"].is_null() {
            line.push_str(&format!(
                " ≈ {} {}",
                amount_text(&deposit["underlyingAmount"], decimals),
                mint_label(&deposit["underlyingMint"])
            ));
        }
        if let Some(rate) = deposit["exchangeRate"].as_str() {
            line.push_str(&format!(" @ {rate}"));
        }
        out.push_str(&line);
        out.push('\n');
    }
    for borrow in &borrows {
        let decimals = borrow["underlyingDecimals"].as_u64().map(|d| d as u32);
        out.push_str(&format!(
            "      borrow   {}  {} {}\n",
            short(borrow["reserve"].as_str().unwrap_or("?")),
            amount_text(&borrow["borrowedAmount"], decimals),
            mint_label(&borrow["underlyingMint"])
        ));
    }
    out
}

/// A raw amount in whole units with `decimals`, else raw and marked so. Text
/// that is not an integer is shown as it is.
fn amount_text(value: &Value, decimals: Option<u32>) -> String {
    match (amount(value), decimals) {
        (Some(raw), Some(decimals)) => units(raw, decimals),
        (Some(raw), None) => format!("{raw} (raw)"),
        (None, _) => value.as_str().unwrap_or("?").to_string(),
    }
}

/// A mint, shortened; `token` when the reserve could not be read.
fn mint_label(mint: &Value) -> String {
    mint.as_str().map(short).unwrap_or_else(|| "token".into())
}

// ---- rules --------------------------------------------------------------------

fn rules_show(target: SandboxTarget, json: bool) -> Result<()> {
    let runtime = runtime()?;
    let (client, _) = client(&runtime, &target, json)?;
    let body = match runtime.block_on(client.rules()) {
        Ok(body) => body,
        Err(refusal) if refusal.code() == rules::UNSUPPORTED_VERSION_CODE => {
            return Err(unsupported_rules(&refusal, json))
        }
        Err(refusal) => return Err(refused(refusal, json)),
    };
    if json {
        println!("{body}");
    } else {
        print!("{}", render_rules(&body));
    }
    Ok(())
}

/// Rules in a format before version 3: the one-line refusal, as JSON on
/// stdout with --json, else on stderr, and exit 1.
fn unsupported_rules(refusal: &SandboxRefusal, json: bool) -> anyhow::Error {
    if json {
        println!("{}", refusal.to_json());
    } else {
        eprintln!("{}", unsupported_rules_line(refusal));
    }
    ExitCode(1).into()
}

pub fn unsupported_rules_line(refusal: &SandboxRefusal) -> String {
    let shown = refusal.to_json();
    format!(
        "{} {}",
        "Error:".red().bold(),
        shown["error"].as_str().unwrap_or_default()
    )
}

// ---- reads --------------------------------------------------------------------

fn reads_show(target: SandboxTarget, config: &str, json: bool) -> Result<()> {
    use arete_mcp::entitlements::Group;
    let runtime = runtime()?;
    let (client, resolved) = resolved_client(&runtime, &target, json)?;
    let body = runtime
        .block_on(client.reads())
        .map_err(|refusal| refused(refusal, json))?;
    if json {
        println!("{body}");
    } else {
        // One lookup of the account's groups serves both commands.
        let surface = crate::entitlements::selected_account_surface(config);
        let implicit = Implicit {
            account: sandbox_only(&surface, &[Group::Stacks, Group::Streams]),
            stack: sandbox_only(&surface, &[Group::Streams]),
        };
        print!("{}", render_reads(&body, &resolved, implicit));
    }
    Ok(())
}

/// Which read commands go through the sandbox without --sandbox for this
/// account: `a4 program account decode`, and `a4 get` / `a4 stream`.
#[derive(Debug, Clone, Copy, Default)]
pub struct Implicit {
    pub account: bool,
    pub stack: bool,
}

/// The human rendering of the programs a sandbox can read. A suggested
/// command leaves out --sandbox only when it would read through this
/// sandbox without it: the account reads that way only through a sandbox
/// (`implicit`) and this one is chosen with `a4 sandbox use`.
pub fn render_reads(
    body: &Value,
    resolved: &arete_mcp::sandbox::selection::Resolved,
    implicit: Implicit,
) -> String {
    use arete_mcp::sandbox::selection::Source;
    let sandbox_id = resolved.sandbox_id.as_str();
    let programs = arete_mcp::program_read::readable_programs(body);
    let mut out = String::new();
    if programs.is_empty() {
        out.push_str(&format!(
            "Sandbox {sandbox_id} can read no programs: its rules enable none.\n"
        ));
        return out;
    }
    let version = rules::revision(body)
        .or_else(|| body["rulesVersion"].as_u64())
        .map(|revision| format!(" (rules revision {revision})"))
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
    let saved = matches!(resolved.source, Source::Saved | Source::Selected);
    let flag = |implicit: bool| match saved && implicit {
        true => String::new(),
        false => format!(" --sandbox {sandbox_id}"),
    };
    let (account, stack) = (flag(implicit.account), flag(implicit.stack));
    out.push_str(&format!(
        "\nRead an account:  a4 program account decode <address>{account}\n\
         Read a stack:     a4 get <View> --stack <stack>{stack}\n\
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
    let (shown, valid) = validation_output(&name, &text, json);
    print!("{shown}");
    if valid {
        Ok(())
    } else {
        Err(ExitCode(1).into())
    }
}

/// What `rules validate` prints for the rules `text` read from `name`, and
/// whether they are valid. A document in a format before version 3 gets one
/// line (one error with --json) instead of a check of every field.
pub fn validation_output(name: &str, text: &str, json: bool) -> (String, bool) {
    let document = serde_json::from_str::<Value>(text);
    let report = match &document {
        Ok(document) => rules::validate(document),
        Err(error) => rules::RulesReport {
            valid: false,
            errors: vec![rules::RulesIssue {
                path: String::new(),
                message: format!("not JSON: {error}"),
            }],
            ..Default::default()
        },
    };
    let shown = if json {
        format!("{}\n", serde_json::to_string(&report).unwrap_or_default())
    } else {
        match document.as_ref().ok().and_then(rules::unsupported_version) {
            Some(message) => format!("{name}: {}\n", message.red()),
            None => render_report(name, &report),
        }
    };
    (shown, report.valid)
}

/// The human rendering of a validation report.
pub fn render_report(name: &str, report: &rules::RulesReport) -> String {
    let source = match &report.checked {
        Some(member) => format!("the signed rules in the agent view (`{member}`); "),
        None => String::new(),
    };
    let mut out = if report.valid {
        let enabled = if report.enabled_programs.is_empty() {
            "no programs enabled".to_string()
        } else {
            format!("enabled: {}", report.enabled_programs.join(", "))
        };
        format!("{name}: {} ({source}{enabled})\n", "valid Rules v3".green())
    } else if report.checked.is_some() {
        format!(
            "{name}: {} in the signed rules of the agent view\n",
            format!(
                "{} error{}",
                report.errors.len(),
                if report.errors.len() == 1 { "" } else { "s" }
            )
            .red()
        )
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

/// The human rendering of the rules an agent sees: the programs with their
/// allowed instructions, the Rules v3 toggles, limits and allowlist when the
/// API gives them, and the baseline. The header names the format and, for
/// the agent's view, the revision (which signing is active): an agent
/// view's numbers are never shown as a format they are not.
pub fn render_rules(body: &Value) -> String {
    let rules = rules::find_document(body);
    let format = rules::format(rules).map(|format| format!(" v{format}"));
    let mut out = format!("{}{}", "Rules".bold(), format.unwrap_or_default());
    if rules::is_agent_view(body) {
        match rules::revision(body) {
            Some(revision) => {
                out.push_str(&format!(" (revision {revision}, as this agent sees them)"))
            }
            None => out.push_str(" (as this agent sees them)"),
        }
    }
    out.push('\n');
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
            line.push_str(&instruction_lines(program));
            line.push_str(&functions(program));
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

/// The functions a program entry offers (the agent view's `{ name,
/// summary, input? }` objects or a signed document's names), one per line,
/// or nothing when it lists none. A function with an `input` schema is
/// shown as a signature, its summary on the line below.
fn functions(program: &Value) -> String {
    let lines: Vec<String> = program["functions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|function| {
            let name = function.as_str().or_else(|| function["name"].as_str())?;
            let summary = function["summary"]
                .as_str()
                .map(str::trim)
                .filter(|summary| !summary.is_empty());
            Some(
                match (function_signature(name, &function["input"]), summary) {
                    (Some(signature), Some(summary)) => {
                        format!("{signature}\n               {summary}")
                    }
                    (Some(signature), None) => signature,
                    (None, Some(summary)) => format!("{name}  {summary}"),
                    (None, None) => name.to_string(),
                },
            )
        })
        .collect();
    if lines.is_empty() {
        return String::new();
    }
    let mut out = String::from("\n           functions (the `call` action):");
    for line in lines {
        out.push_str(&format!("\n             {line}"));
    }
    out
}

/// `name(field: type, optional?: type = default, ...)` from a function's
/// `input` schema (`{ fields: [{ name, type, required, fill?, default?,
/// values?, variants? }] }`), or `None` when it has none. Members Arete
/// fills (the wallet) are left out; an enum shows its variants and a member
/// with special values adds them (`amount | "max"`).
fn function_signature(name: &str, input: &Value) -> Option<String> {
    let fields = input["fields"].as_array()?;
    let members: Vec<String> = fields
        .iter()
        .filter(|field| field["fill"].is_null())
        .filter_map(|field| {
            let member = field["name"].as_str()?;
            let mut ty = match field["variants"].as_array() {
                Some(variants) if !variants.is_empty() => variants
                    .iter()
                    .map(|variant| variant.to_string())
                    .collect::<Vec<_>>()
                    .join(" | "),
                _ => field["type"].as_str().unwrap_or("?").to_string(),
            };
            for value in field["values"].as_array().into_iter().flatten() {
                if let Some(value) = value.get("value") {
                    ty.push_str(&format!(" | {value}"));
                }
            }
            let optional = if field["required"].as_bool() == Some(false) {
                "?"
            } else {
                ""
            };
            let default = match field.get("default") {
                Some(default) if !default.is_null() => format!(" = {default}"),
                _ => String::new(),
            };
            Some(format!("{member}{optional}: {ty}{default}"))
        })
        .collect();
    Some(format!("{name}({})", members.join(", ")))
}

/// What goes under a program's line about its instructions: the names on
/// one line, or, when the agent view details them, one line per allowed
/// instruction with its arguments.
fn instruction_lines(program: &Value) -> String {
    let Some(signatures) = instruction_signatures(program) else {
        return format!(
            "\n           instructions: {}",
            allowed_instructions(program)
        );
    };
    let mut out = String::from("\n           instructions:");
    if program["allowed"].as_str() == Some("any") {
        out.push_str(" any");
    }
    for signature in signatures {
        out.push_str(&format!("\n             {signature}"));
    }
    out
}

/// One `name(arg: type, ...)` per allowed instruction, when the agent view
/// details the program's instructions (`instructions: [{name, args}]`);
/// `None` for names alone (the signed document) or no details. An allowed
/// name without details is shown alone.
fn instruction_signatures(program: &Value) -> Option<Vec<String>> {
    let detailed: Vec<&Value> = program["instructions"]
        .as_array()?
        .iter()
        .filter(|entry| entry["name"].is_string() && entry["args"].is_array())
        .collect();
    if detailed.is_empty() {
        return None;
    }
    let types = program["types"].as_object();
    let signature = |name: &str| match detailed.iter().find(|entry| entry["name"] == name) {
        Some(entry) => instruction_signature(entry, types),
        None => name.to_string(),
    };
    let signatures: Vec<String> = match program["allowed"].as_array() {
        Some(allowed) => allowed
            .iter()
            .filter_map(|entry| entry.as_str().or_else(|| entry["name"].as_str()))
            .map(signature)
            .collect(),
        None => detailed
            .iter()
            .map(|entry| instruction_signature(entry, types))
            .collect(),
    };
    (!signatures.is_empty()).then_some(signatures)
}

/// How deep defined types are expanded in an argument's type: the
/// argument's own type and the defined types inside it; deeper ones are
/// shown by name.
const EXPAND_DEPTH: usize = 2;

/// `name(arg: type, ...)` for an instruction of the agent view.
fn instruction_signature(
    instruction: &Value,
    types: Option<&serde_json::Map<String, Value>>,
) -> String {
    let args: Vec<String> = instruction["args"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|arg| {
            format!(
                "{}: {}",
                arg["name"].as_str().unwrap_or("?"),
                type_label(&arg["type"], types, 0, &mut Vec::new())
            )
        })
        .collect();
    format!(
        "{}({})",
        instruction["name"].as_str().unwrap_or("?"),
        args.join(", ")
    )
}

/// An IDL type, compactly: `u64`, `Option<u8>`, `Vec<Pubkey>`, `[u8; 32]`,
/// and a defined type expanded from `types` (`Name { a: u8 }`,
/// `Name (A | B)`) up to [`EXPAND_DEPTH`], else by name. `expanding` holds
/// the defined types being expanded, so a recursive type stops at its name.
fn type_label(
    ty: &Value,
    types: Option<&serde_json::Map<String, Value>>,
    depth: usize,
    expanding: &mut Vec<String>,
) -> String {
    let Value::Object(object) = ty else {
        return ty.as_str().unwrap_or("?").to_string();
    };
    if let Some(defined) = object.get("defined") {
        let name = defined
            .as_str()
            .or_else(|| defined["name"].as_str())
            .unwrap_or("?");
        return defined_label(name, types, depth, expanding);
    }
    if let Some(generic) = object.get("generic").and_then(Value::as_str) {
        return generic.to_string();
    }
    if let Some(array) = object.get("array").and_then(Value::as_array) {
        let length = match array.get(1) {
            Some(Value::Number(length)) => length.to_string(),
            Some(other) => type_label(other, types, depth, expanding),
            None => "?".into(),
        };
        let item = array
            .first()
            .map(|item| type_label(item, types, depth, expanding))
            .unwrap_or_else(|| "?".into());
        return format!("[{item}; {length}]");
    }
    for (key, wrapper) in [("option", "Option"), ("coption", "COption"), ("vec", "Vec")] {
        if let Some(inner) = object.get(key) {
            return format!("{wrapper}<{}>", type_label(inner, types, depth, expanding));
        }
    }
    // Some other wrapper: `key<inner>`.
    match object.iter().next() {
        Some((key, inner)) if object.len() == 1 => {
            format!("{key}<{}>", type_label(inner, types, depth, expanding))
        }
        _ => "?".into(),
    }
}

/// A defined type by `name`: its definition from `types` when there is one
/// and the depth allows, else the name.
fn defined_label(
    name: &str,
    types: Option<&serde_json::Map<String, Value>>,
    depth: usize,
    expanding: &mut Vec<String>,
) -> String {
    let Some(definition) = types.and_then(|types| types.get(name)) else {
        return name.to_string();
    };
    if depth >= EXPAND_DEPTH || expanding.iter().any(|open| open == name) {
        return name.to_string();
    }
    expanding.push(name.to_string());
    let label = match definition["kind"].as_str() {
        Some("struct") => format!(
            "{name}{}",
            fields_label(&definition["fields"], types, depth + 1, expanding)
        ),
        Some("enum") => {
            let variants: Vec<String> = definition["variants"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|variant| {
                    format!(
                        "{}{}",
                        variant["name"].as_str().unwrap_or("?"),
                        fields_label(&variant["fields"], types, depth + 1, expanding)
                    )
                })
                .collect();
            if variants.is_empty() {
                name.to_string()
            } else {
                format!("{name} ({})", variants.join(" | "))
            }
        }
        Some("type") if !definition["alias"].is_null() => {
            type_label(&definition["alias"], types, depth, expanding)
        }
        _ => name.to_string(),
    };
    expanding.pop();
    label
}

/// A struct's or variant's fields: ` { a: u8, b: u64 }` when named,
/// `(u8, u64)` when a tuple, nothing when there are none.
fn fields_label(
    fields: &Value,
    types: Option<&serde_json::Map<String, Value>>,
    depth: usize,
    expanding: &mut Vec<String>,
) -> String {
    let fields = fields.as_array().cloned().unwrap_or_default();
    if fields.is_empty() {
        return String::new();
    }
    let named = fields.iter().all(|field| field["name"].is_string());
    let shown: Vec<String> = fields
        .iter()
        .map(|field| {
            if named {
                format!(
                    "{}: {}",
                    field["name"].as_str().unwrap_or("?"),
                    type_label(&field["type"], types, depth, expanding)
                )
            } else {
                type_label(field, types, depth, expanding)
            }
        })
        .collect();
    if named {
        format!(" {{ {} }}", shown.join(", "))
    } else {
        format!("({})", shown.join(", "))
    }
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
    use arete_mcp::sandbox::CallKind;

    const PUMP: &str = "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P";

    #[test]
    fn action_hints_name_the_cli_commands() {
        assert_eq!(
            cli_terms("Check sandbox_rules, then `sandbox_preview` and sandbox_invoke."),
            "Check `a4 sandbox rules show`, then `a4 sandbox preview` and `a4 sandbox invoke`."
        );
        assert_eq!(
            cli_terms("look the action up with sandbox_action and the idempotencyKey"),
            "look the action up with `a4 sandbox action` and the --idempotency-key"
        );
        // Not inside a longer word.
        assert_eq!(
            cli_terms("my_sandbox_rules and sandbox_rulesx"),
            "my_sandbox_rules and sandbox_rulesx"
        );
        assert_eq!(cli_terms("sandbox_select."), "`a4 sandbox use`.");

        // Every hint the MCP server gives an action refusal is in CLI terms.
        for code in [
            "sandbox-action-not-granted",
            "sandbox-intent-invalid",
            "sandbox-function-input-invalid",
            "sandbox-guard-insufficient-funds",
            "sandbox-duplicate-action",
            "sandbox-api-timeout",
            "sandbox-response-too-large",
            "sandbox-program-not-allowlisted",
        ] {
            for kind in [CallKind::Preview, CallKind::Action] {
                let refusal = SandboxRefusal {
                    status: 403,
                    body: json!({ "code": code, "error": "refused" }),
                    kind,
                };
                let shown = shown_refusal(&refusal);
                let hint = shown["hint"].as_str().unwrap();
                assert!(!hint.contains("sandbox_"), "{code}: {hint}");
                // Everything else is the MCP tools' refusal.
                let mut mcp = refusal.to_json();
                mcp["hint"] = shown["hint"].clone();
                assert_eq!(shown, mcp);
            }
        }
    }

    fn action_args(args: &[&str]) -> ActionArgs {
        #[derive(clap::Parser)]
        struct Cli {
            #[command(flatten)]
            action: ActionArgs,
        }
        let mut argv = vec!["a4"];
        argv.extend(args);
        <Cli as clap::Parser>::try_parse_from(argv).unwrap().action
    }

    #[test]
    fn action_flags_become_the_mcp_tools_action_and_args() {
        let (action, args) = action_args(&[
            "--call",
            "KLend",
            "transactions.obligation.deposit",
            "--input",
            r#"{"reserve":"R","amount":{"ui":"0.1"}}"#,
            "--transaction-format",
            "v0",
        ])
        .action()
        .unwrap();
        assert_eq!(action, SandboxActionName::Call);
        assert_eq!(
            args,
            json!({ "program": "KLend", "function": "transactions.obligation.deposit",
                    "input": { "reserve": "R", "amount": { "ui": "0.1" } },
                    "transactionFormat": "v0" })
        );

        let (action, args) = action_args(&[
            "--invoke",
            r#"{"instructions":[{"program":"P","instruction":"i"}],"computeUnitLimit":5}"#,
            "--compute-unit-price",
            "7",
        ])
        .action()
        .unwrap();
        assert_eq!(action, SandboxActionName::Invoke);
        assert_eq!(
            args,
            json!({ "instructions": [{ "program": "P", "instruction": "i" }],
                    "computeUnitLimit": 5, "computeUnitPriceMicroLamports": "7" })
        );

        let (action, args) = action_args(&["--withdraw", "18446744073709551615"])
            .action()
            .unwrap();
        assert_eq!(action, SandboxActionName::Withdraw);
        assert_eq!(args, json!({ "lamports": "18446744073709551615" }));

        // A call without --input leaves the input to the API.
        let (_, args) = action_args(&["--call", "P", "f"]).action().unwrap();
        assert_eq!(args, json!({ "program": "P", "function": "f" }));

        let error = action_args(&["--invoke", "\"text\""]).action().unwrap_err();
        assert!(
            error.to_string().contains("--invoke takes a JSON object"),
            "{error}"
        );
    }

    #[test]
    fn a_direct_read_refused_for_a_bound_key_is_recognised() {
        let bound = serde_json::json!({
            "error": "This agent key is bound to a sandbox, so it reads programs and stacks only through it",
            "code": "sandbox-read-required",
            "retryable": false,
            "sandboxes": [{"sandboxId": "6f1c2c43-6d3c-4a51-9b8e-0e3c1e2a7b10", "name": "Trading"}],
            "readRoute": "/api/sandbox/v1/sessions",
        });
        let api = crate::api_client::test_support::MockServer::json(403, &bound.to_string());
        let client = crate::api_client::ApiClient::with_base_url(api.base_url())
            .with_api_key("a4_sk_agent".into());
        let error = client
            .mint_target_session(&format!("{}/ws/sessions", api.base_url()), &json!({}))
            .unwrap_err();
        let refusal = read_required(&error).expect("the bound-key refusal");
        assert_eq!(refusal.status, 403);
        assert_eq!(
            refusal.to_json()["sandboxes"][0]["sandboxId"],
            "6f1c2c43-6d3c-4a51-9b8e-0e3c1e2a7b10"
        );
        let hint = cli_hint(&refusal).unwrap();
        assert!(
            hint.contains("a4 sandbox use") && hint.contains("--sandbox"),
            "{hint}"
        );

        // Any other refusal is left as it is.
        let other = crate::api_client::test_support::MockServer::json(
            403,
            r#"{"error":"no","code":"forbidden"}"#,
        );
        let error = crate::api_client::ApiClient::with_base_url(other.base_url())
            .with_api_key("a4_sk_agent".into())
            .mint_target_session(&format!("{}/ws/sessions", other.base_url()), &json!({}))
            .unwrap_err();
        assert!(read_required(&error).is_none());
    }

    #[test]
    fn read_refusals_point_to_the_cli_commands() {
        let refusal = |code: &str, kind| SandboxRefusal {
            status: 403,
            body: json!({
                "code": code,
                "error": "Program KLend is not in the sandbox's read rules",
                "programs": ["KLend"],
            }),
            kind,
        };
        let read = refusal(
            "sandbox-program-not-allowlisted",
            arete_mcp::sandbox::CallKind::Read,
        );
        let message = cli_message(&read);
        assert!(
            message.contains("`a4 sandbox reads` lists them"),
            "{message}"
        );
        assert!(!message.contains("sandbox_rules"), "{message}");
        assert!(!message.contains("sandbox_reads"), "{message}");
        // The message names the program; it is not repeated.
        assert!(!message.contains("not in the rules: KLend"), "{message}");
        for code in [
            "sandbox-read-programs-unknown",
            "sandbox-read-target-unsupported",
        ] {
            let hint = cli_hint(&refusal(code, arete_mcp::sandbox::CallKind::Read)).unwrap();
            assert!(hint.contains("a4 "), "{hint}");
        }
    }
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
        let cli_hint = |code: &str| {
            cli_hint(&SandboxRefusal {
                status: 403,
                body: json!({ "code": code }),
                kind: arete_mcp::sandbox::CallKind::Read,
            })
        };
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
            "rulesRevision": 7,
            "programs": [
                {"programId": "JUP6", "name": "Jupiter"},
                {"programId": "KLend"},
            ],
        });
        use arete_mcp::sandbox::selection::{Resolved, Source};
        let resolved = |source| Resolved {
            sandbox_id: "s".into(),
            source,
        };
        let both = Implicit {
            account: true,
            stack: true,
        };
        let text = render_reads(&body, &resolved(Source::OnlyGrant), both);
        assert!(
            text.contains("can read 2 programs (rules revision 7)"),
            "{text}"
        );
        assert!(text.contains("Jupiter"), "{text}");
        assert!(text.contains("  KLend\n"), "{text}");
        assert!(text.contains("decode <address> --sandbox s\n"), "{text}");
        // The sandbox saved for this directory needs no --sandbox, for the
        // commands that read through a sandbox by default.
        for saved in [Source::Saved, Source::Selected] {
            let text = render_reads(&body, &resolved(saved), both);
            assert!(!text.contains("--sandbox"), "{text}");
            assert!(text.contains("decode <address>\n"), "{text}");
        }
        // An account that reads stacks with its own key still needs it there.
        let account_only = Implicit {
            account: true,
            stack: false,
        };
        let text = render_reads(&body, &resolved(Source::Saved), account_only);
        assert!(text.contains("decode <address>\n"), "{text}");
        assert!(text.contains("--stack <stack> --sandbox s\n"), "{text}");
        let text = render_reads(&body, &resolved(Source::Saved), Implicit::default());
        assert_eq!(text.matches("--sandbox s").count(), 2, "{text}");
        let none = render_reads(&json!({"programs": []}), &resolved(Source::Env), both);
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

    /// Newer records carry the network fee (`networkFeeLamports`) and the
    /// guard pipeline that checked the action; older ones (the fixture
    /// above) carry neither and render as before, as does a record whose
    /// network fee is still null.
    #[test]
    fn history_shows_the_fee_and_guard_pipeline_when_recorded() {
        colored::control::set_override(false);
        let text = render_history(
            &fixture("history-page-fees.json"),
            &HistoryArgs::default(),
            None,
            "sbx",
        );
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines[2],
            "  2026-11-12 10:00:00  landed    Pump.fun.buy  -0.105005 SOL, network fee 0.000005 SOL, +1234.5 7xKX…gAsU  5Kt3…6uHq  guard execute-v1",
            "{text}"
        );
        let rejected = lines.iter().find(|line| line.contains("rejected")).unwrap();
        assert!(!rejected.contains("guard"), "{text}");
        // Sent, its landing not read yet: no network fee to show.
        let sent = lines.iter().find(|line| line.contains("sent ")).unwrap();
        assert!(!sent.contains("fee"), "{text}");
        assert!(sent.contains("guard execute-v1"), "{text}");

        let older = render_history(
            &fixture("history-page.json"),
            &HistoryArgs::default(),
            None,
            "sbx",
        );
        assert!(!older.contains("fee "), "{older}");
        assert!(!older.contains("guard "), "{older}");
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
        assert!(text.contains("  Positions\n"), "{text}");
        // A position without a decoded `obligation` keeps its one line.
        assert!(
            text.contains(
                "    KLen…gmjD  obligation  H4dH3hmyWZ1SbsMzYqMKdyFBsUuvHGbzvNNxGg8bPFuu\n    "
            ),
            "{text}"
        );
    }

    /// A decoded Kamino obligation: a line per deposit, in the underlying
    /// token's units with its value at the reserve's rate, and per borrow.
    /// A reserve that could not be read leaves its amounts raw.
    #[test]
    fn holdings_render_decoded_obligations() {
        colored::control::set_override(false);
        let text = render_holdings(&fixture("holdings.json"), "sbx");
        let expected = "    KLen…gmjD  Kamino obligation  5xLRsQnVdPbJJ1TVBf5RUzJ3cXcKqVgF6zU6aH5pyLzM  market 7u3H…5PfF\n\
             \x20     deposit  d4A2…Bc4Q  0.022755326 cToken ≈ 0.026416668 So11…1112 @ 1.160901249\n\
             \x20     deposit  Ga4r…auXp  5000 (raw) cToken\n\
             \x20     borrow   D6q6…gJ59  1.234567 EPjF…Dt1v\n";
        assert!(text.ends_with(expected), "{text}");

        let empty = json!({
            "sol": "0", "tokens": [],
            "positions": [{
                "program": "KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD", "account": "A",
                "kind": "kaminoObligation", "label": "Kamino obligation",
                "obligation": { "lendingMarket": null, "deposits": [], "borrows": [] }
            }]
        });
        let text = render_holdings(&empty, "sbx");
        assert!(
            text.ends_with("Kamino obligation  A\n      no deposits or borrows\n"),
            "{text}"
        );
    }

    #[test]
    fn status_renders_the_wallet_balances_and_grants() {
        colored::control::set_override(false);
        let text = render_status(&fixture("status.json"), Some("Bot"));
        assert_eq!(
            text,
            format!(
                "Sandbox 6f1c2c43-6d3c-4a51-9b8e-0e3c1e2a7b10  Bot  (saved for this directory)\n\
                 \n\
                 \x20 Wallet     9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM\n\
                 \x20 Cluster    mainnet-beta\n\
                 \x20 SOL        1.394995\n\
                 \x20 Tokens\n\
                 \x20   {MINT}  1234.5\n\
                 \x20 Actions    invoke, withdraw\n\
                 \x20 Withdraws  to 4Nd1mBQtrMJVYVfKf2PJy9NZUZdTAsp7D4xWLs4gDB4T\n\
                 \x20 Rules      revision 3\n"
            )
        );

        // A dev environment's wallet, nothing held or granted, a fee, and
        // no name or selection (an older server's answer).
        let mut body = fixture("status.json");
        body["environment"] = json!("devenv:e1");
        body["sandboxExpiresAt"] = json!("2026-11-16T00:00:00Z");
        body["tokens"] = json!([]);
        body["actions"] = json!([]);
        body["feeLamports"] = json!(1000);
        body.as_object_mut().unwrap().remove("selection");
        let text = render_status(&body, None);
        assert!(
            text.starts_with("Sandbox 6f1c2c43-6d3c-4a51-9b8e-0e3c1e2a7b10\n\n"),
            "{text}"
        );
        assert!(
            text.contains("  Cluster    mainnet-beta (devenv:e1)\n"),
            "{text}"
        );
        assert!(text.contains("  Tokens     none\n"), "{text}");
        assert!(text.contains("  Actions    none\n"), "{text}");
        assert!(
            text.contains("  Arete fee  0.000001 SOL per action\n"),
            "{text}"
        );
        assert!(
            text.contains("  Expires    2026-11-16T00:00:00Z\n"),
            "{text}"
        );
    }

    #[test]
    fn raw_is_dropped_unless_asked_for() {
        let page = fixture("history-page.json");
        assert!(page["items"][0].get("raw").is_some());
        assert!(ledger::without_raw(page)["items"][0].get("raw").is_none());
        // A decoded obligation is not raw data: it stays.
        let holdings = ledger::without_raw(fixture("holdings.json"));
        assert!(holdings["positions"][1].get("raw").is_none());
        assert_eq!(
            holdings["positions"][1]["obligation"]["deposits"][0]["cTokenAmount"],
            "22755326"
        );
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
    fn the_agent_view_renders_instructions_and_the_baseline() {
        colored::control::set_override(false);
        let text = render_rules(&json!({
            "view": "agent",
            "rulesFormat": 3,
            "revision": 1,
            "programs": [
                { "programId": PUMP, "name": "pump", "allowed": ["buy"],
                  "instructions": [{ "name": "buy", "args": [], "accounts": [] }] },
                { "programId": "S", "name": "spl", "allowed": "any", "instructions": [] }
            ],
            "baseline": [{ "name": "System" }, { "name": "Associated Token" }]
        }));
        assert!(
            text.starts_with("Rules v3 (revision 1, as this agent sees them)\n"),
            "{text}"
        );
        assert!(
            text.contains("           instructions:\n             buy()\n"),
            "{text}"
        );
        assert!(text.contains("instructions: any\n"), "{text}");
        assert!(
            text.contains("Always allowed: System, Associated Token"),
            "{text}"
        );
        assert!(!text.contains("Transfers"), "{text}");
        assert!(!text.contains("functions"), "{text}");
    }

    #[test]
    fn the_agent_view_renders_a_programs_functions() {
        colored::control::set_override(false);
        let text = render_rules(&json!({
            "view": "agent",
            "rulesFormat": 3,
            "revision": 1,
            "programs": [{
                "programId": "KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD",
                "name": "Kamino Lending",
                "allowed": ["depositReserveLiquidityAndObligationCollateralV2"],
                "functions": [
                    { "name": "transactions.obligation.deposit",
                      "summary": "Deposit into a reserve as obligation collateral" },
                    { "name": "transactions.obligation.withdraw" }
                ]
            }]
        }));
        assert!(
            text.contains(
                "           instructions: depositReserveLiquidityAndObligationCollateralV2\n           functions (the `call` action):\n"
            ),
            "{text}"
        );
        assert!(
            text.contains("\n             transactions.obligation.deposit  Deposit into a reserve as obligation collateral\n"),
            "{text}"
        );
        assert!(
            text.contains("\n             transactions.obligation.withdraw\n"),
            "{text}"
        );
    }

    #[test]
    fn functions_with_an_input_schema_render_as_signatures() {
        colored::control::set_override(false);
        let text = render_rules(&json!({
            "view": "agent",
            "rulesFormat": 3,
            "revision": 1,
            "programs": [{
                "programId": "KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD",
                "name": "Kamino Lending",
                "allowed": [],
                "functions": [
                    {
                        "name": "transactions.obligation.withdraw",
                        "summary": "Withdraw collateral from an obligation",
                        "input": {
                            "fields": [
                                { "name": "owner", "type": "pubkey", "required": false, "fill": "wallet" },
                                { "name": "reserve", "type": "pubkey", "required": true },
                                { "name": "amount", "type": "amount", "required": true,
                                  "units": "the reserve's liquidity mint",
                                  "values": [{ "value": "max", "meaning": "the whole deposit" }] },
                                { "name": "payer", "type": "pubkey", "required": false }
                            ],
                            "unknownFields": "refused"
                        }
                    },
                    {
                        "name": "transactions.position.addLiquidity",
                        "input": {
                            "fields": [
                                { "name": "width", "type": "i64", "required": true },
                                { "name": "strategy", "type": "enum", "required": false,
                                  "variants": ["spot", "curve", "bidAsk"], "default": "spot" },
                                { "name": "bps", "type": "i64", "required": false, "default": 10000 }
                            ]
                        }
                    },
                    // An API that sends no `input` keeps the name and summary.
                    { "name": "transactions.obligation.deposit", "summary": "Deposit" }
                ]
            }]
        }));
        assert!(
            text.contains(
                "\n             transactions.obligation.withdraw(reserve: pubkey, amount: amount | \"max\", payer?: pubkey)\n               Withdraw collateral from an obligation\n"
            ),
            "{text}"
        );
        assert!(
            text.contains(
                "\n             transactions.position.addLiquidity(width: i64, strategy?: \"spot\" | \"curve\" | \"bidAsk\" = \"spot\", bps?: i64 = 10000)\n"
            ),
            "{text}"
        );
        assert!(
            text.contains("\n             transactions.obligation.deposit  Deposit\n"),
            "{text}"
        );
        assert!(!text.contains("owner"), "{text}");
    }

    fn kamino_view(types: Option<Value>) -> Value {
        let mut program = json!({
            "programId": "KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD",
            "name": "Kamino Lend",
            "allowed": ["initObligation", "depositReserveLiquidity", "refreshReserve"],
            "instructions": [
                { "name": "initObligation", "discriminator": [251, 10, 231, 76, 27, 11, 159, 96],
                  "args": [{ "name": "args", "type": { "defined": "InitObligationArgs" } }],
                  "accounts": [] },
                { "name": "depositReserveLiquidity", "discriminator": [169, 201, 30, 126, 6, 205, 102, 68],
                  "args": [
                      { "name": "liquidityAmount", "type": "u64" },
                      { "name": "mode", "type": { "option": { "defined": { "name": "Mode" } } } },
                      { "name": "path", "type": { "vec": { "defined": "Hop" } } },
                      { "name": "seed", "type": { "array": ["u8", 32] } }
                  ],
                  "accounts": [] }
            ]
        });
        if let Some(types) = types {
            program["types"] = types;
        }
        json!({ "view": "agent", "rulesFormat": 3, "revision": 2, "programs": [program], "baseline": [] })
    }

    /// The agent view's instructions are listed one per line with their
    /// arguments, defined types expanded from `types`: a struct's fields, an
    /// enum's variants, the types inside them expanded once more, deeper
    /// ones by name.
    #[test]
    fn the_agent_view_lists_instruction_signatures() {
        colored::control::set_override(false);
        let types = json!({
            "InitObligationArgs": { "kind": "struct", "fields": [
                { "name": "tag", "type": "u8" }, { "name": "id", "type": "u8" }
            ] },
            "Mode": { "kind": "enum", "variants": [
                { "name": "Plain" },
                { "name": "Capped", "fields": [{ "name": "cap", "type": { "defined": "Cap" } }] },
                { "name": "Pair", "fields": ["u8", "u64"] }
            ] },
            "Cap": { "kind": "struct", "fields": [{ "name": "limit", "type": { "defined": "Limit" } }] },
            "Limit": { "kind": "struct", "fields": [{ "name": "lamports", "type": "u64" }] },
            "Hop": { "kind": "struct", "fields": [
                { "name": "next", "type": { "option": { "defined": "Hop" } } }
            ] }
        });
        let text = render_rules(&kamino_view(Some(types)));
        assert!(
            text.contains(
                "           instructions:\n\
                 \x20            initObligation(args: InitObligationArgs { tag: u8, id: u8 })\n\
                 \x20            depositReserveLiquidity(liquidityAmount: u64, \
                 mode: Option<Mode (Plain | Capped { cap: Cap { limit: Limit } } | Pair(u8, u64))>, \
                 path: Vec<Hop { next: Option<Hop> }>, seed: [u8; 32])\n\
                 \x20            refreshReserve\n"
            ),
            "{text}"
        );
        assert!(!text.contains("defined"), "{text}");
    }

    /// An older server sends no `types`: a defined type is shown by its
    /// name, never as raw JSON.
    #[test]
    fn without_types_a_defined_type_is_shown_by_name() {
        colored::control::set_override(false);
        let text = render_rules(&kamino_view(None));
        assert!(
            text.contains("initObligation(args: InitObligationArgs)\n"),
            "{text}"
        );
        assert!(
            text.contains("mode: Option<Mode>, path: Vec<Hop>, seed: [u8; 32])"),
            "{text}"
        );
        assert!(!text.contains("defined") && !text.contains('{'), "{text}");
    }

    /// What `rules show` printed before the API named its numbers: the
    /// revision of version 3 rules as `version`.
    fn old_agent_view() -> Value {
        json!({
            "version": 1,
            "requestId": "req-1",
            "baseline": [{ "name": "System" }],
            "programs": [{
                "programId": PUMP, "name": "Pump.fun", "allowed": ["buy"],
                "instructions": [{ "name": "buy", "args": [], "accounts": [] }]
            }]
        })
    }

    #[test]
    fn an_old_agent_view_shows_its_number_as_a_revision() {
        colored::control::set_override(false);
        let text = render_rules(&old_agent_view());
        assert!(
            text.starts_with("Rules (revision 1, as this agent sees them)\n"),
            "{text}"
        );
        assert!(!text.contains("Rules v1"), "{text}");
    }

    #[test]
    fn the_agent_view_is_not_validated_as_a_rules_file() {
        colored::control::set_override(false);
        let text = old_agent_view().to_string();
        let (plain, valid) = validation_output("stdin", &text, false);
        assert!(!valid, "still exits 1: it is not a rules document");
        assert!(plain.contains("agent's view"), "{plain}");
        assert!(plain.contains("signedRules"), "{plain}");
        assert!(!plain.contains("turned off"), "{plain}");
        assert!(!plain.contains("no longer supported"), "{plain}");
        assert_eq!(plain.lines().count(), 2, "{plain}");

        // With the signed document in it, that document is checked.
        let mut view = json!({
            "view": "agent",
            "rulesFormat": 3,
            "revision": 1,
            "programs": [],
            "baseline": [],
        });
        view["signedRules"] = fixture("rules-v3.json");
        let (plain, valid) = validation_output("stdin", &view.to_string(), false);
        assert!(valid, "{plain}");
        assert_eq!(
            plain,
            "stdin: valid Rules v3 (the signed rules in the agent view (`signedRules`); enabled: Pump.fun)\n"
        );
        view["signedRules"]["programs"][0]["enabled"] = json!("yes");
        let (plain, valid) = validation_output("stdin", &view.to_string(), false);
        assert!(!valid);
        assert!(
            plain.starts_with("stdin: 1 error in the signed rules of the agent view\n"),
            "{plain}"
        );
        assert!(plain.contains("signedRules.programs[0].enabled"), "{plain}");
    }

    const NO_LONGER: &str = "Rules version 2 is no longer supported. Build and sign v3 rules \
                             in the dashboard (Sandboxes → Rules).";

    #[test]
    fn a_version_2_file_is_refused_with_one_line() {
        colored::control::set_override(false);
        // A version 2 document: programs keyed by id, instruction objects.
        let text = json!({
            "version": 2,
            "programs": { PUMP: { "instructions": [{ "name": "buy" }] } }
        })
        .to_string();

        let (plain, valid) = validation_output("rules.json", &text, false);
        assert!(!valid);
        assert_eq!(plain, format!("rules.json: {NO_LONGER}\n"));

        let (shown, valid) = validation_output("rules.json", &text, true);
        assert!(!valid);
        assert_eq!(shown.lines().count(), 1, "{shown}");
        let report: Value = serde_json::from_str(&shown).unwrap();
        assert_eq!(report["valid"], false);
        assert_eq!(
            report["errors"],
            json!([{ "path": "version", "message": NO_LONGER }])
        );
        assert_eq!(report["warnings"], json!([]));
    }

    #[test]
    fn version_2_rules_from_the_api_are_refused_with_one_line() {
        colored::control::set_override(false);
        let refusal = SandboxRefusal {
            status: 0,
            body: json!({
                "code": rules::UNSUPPORTED_VERSION_CODE,
                "error": NO_LONGER,
                "retryable": false,
            }),
            kind: arete_mcp::sandbox::CallKind::Read,
        };
        assert_eq!(
            unsupported_rules_line(&refusal),
            format!("Error: {NO_LONGER}")
        );
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
