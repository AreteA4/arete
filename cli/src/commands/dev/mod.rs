//! `a4 dev`: develop against an Arete cloud dev environment.
//!
//! A cloud dev environment is a persistent mainnet fork Arete hosts for the
//! account: accounts nobody wrote follow mainnet, accounts you wrote are
//! yours, and it sleeps when idle and wakes on the next call with its state
//! intact. Signing stays local: dev wallets, the MCP local signer and
//! browser wallets sign on this machine.
//!
//! `a4 dev up` creates or wakes the project's environment, serves it on
//! loopback (a local RPC endpoint that adds the environment token, see
//! [`proxy`]), runs the dev UI against it and, in a protocol project, builds
//! the protocol and deploys it into the environment, redeploying on change
//! ([`up`], [`deploy`]). The project remembers its environment in
//! `.arete/dev/environment.json`; while `a4 dev up` runs it also writes
//! `.arete/dev/session.json`.
//!
//! The environment commands (`list`, `status`, `wake`, `sleep`, `delete`,
//! `checkpoint`, `branch`, `restore`, `rebase`, `reset`, `fund`, `pin`,
//! `follow`, `warp`, `pause`, `resume`, `scenario`, `preview`, `repro`,
//! `events`, `token`, `use`) are in [`env`]; the protocol commands (`step`,
//! `wallet`, `instance`, `replay`, `sdk`) drive the deployment through
//! [`arete_mcp::dev`], which the MCP server's `dev_*` tools share.

pub mod api;
pub mod automations;
pub mod capture;
pub mod custom_stacks;
pub mod deploy;
pub mod env;
pub mod proxy;
pub mod runtime;
pub mod sdk_gen;
pub mod tools;
pub mod ui;
pub mod up;
pub mod watch;

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use arete_mcp::dev::{
    parse_replay, DevClient, DevProject, ReplayOptions, StepProgress, StepRequest,
};
use clap::{Args, Subcommand};
use colored::Colorize;
use serde_json::{json, Value};

use crate::api_client::ApiClient;
use env::EnvArgs;

#[derive(Subcommand, Debug)]
pub enum DevCommands {
    /// Internal isolated admission worker. Credentials arrive on stdin.
    #[command(name = "__admit-local-programs", hide = true)]
    AdmitLocalPrograms,
    /// Create or wake this project's cloud dev environment, serve it on
    /// 127.0.0.1 (RPC and WebSocket), run the dev UI and, in a protocol
    /// project, deploy the protocol into it and redeploy on change
    Up(UpArgs),
    /// Stop a running `a4 dev up` for this project (the environment keeps
    /// its state and sleeps when idle)
    Down(DownArgs),
    /// Show the environment, the running session and the chain
    Status(EnvArgs),
    /// List your dev environments and your plan's quotas
    List,
    /// Point this project at another of your environments
    Use(UseArgs),
    /// Wake a sleeping environment
    Wake(EnvArgs),
    /// Put the environment to sleep now (its state is kept)
    Sleep(EnvArgs),
    /// Delete the environment with its state and checkpoints
    Delete(EnvArgs),
    /// Save a checkpoint of the environment's state; --list, --delete
    Checkpoint(CheckpointArgs),
    /// Create a new environment from a checkpoint
    Branch(BranchArgs),
    /// Restore a checkpoint into the environment (later writes are undone)
    Restore(RestoreArgs),
    /// Refetch every account the environment never wrote from mainnet, and
    /// move its clock to mainnet's
    Rebase(RebaseArgs),
    /// Reset the environment to a fresh fork (or one account with --account)
    Reset(ResetArgs),
    /// Set an account's SOL balance, or its token balance with --mint
    Fund(FundArgs),
    /// Pin an account: never refreshed from mainnet, kept by rebase
    Pin(PinArgs),
    /// Follow an account: refetched from mainnet every block
    Follow(FollowArgs),
    /// Move the environment's clock forward (+2h, a Unix timestamp,
    /// slot:<n>, epoch:<n>)
    #[command(alias = "time-travel")]
    Warp(WarpArgs),
    /// Pause the environment's clock
    Pause(EnvArgs),
    /// Resume the environment's clock
    Resume(EnvArgs),
    /// Run a scenario (arete.scenario/v1): account overrides at relative
    /// slots, with expectations
    Scenario(ScenarioArgs),
    /// Preview a transaction without committing it: compute units and
    /// every account before and after
    Preview(PreviewArgs),
    /// Export the minimal state to reproduce one transaction sent in the
    /// environment
    Repro(ReproArgs),
    /// Show the environment's lifecycle events (woken, slept, checkpointed, ...)
    Events(EventsArgs),
    /// Mint an environment token and print the environment's RPC and
    /// WebSocket URLs
    Token(EnvArgs),
    /// Prepare, sign (with a dev wallet), submit and settle one protocol step
    Step(StepArgs),
    /// Show a dev wallet's address and balance (created and funded on
    /// first use), e.g. to pass it as a step argument
    Wallet(WalletArgs),
    /// Read one protocol instance
    Instance(InstanceArgs),
    /// Run a recorded sequence of protocol steps again (history.jsonl)
    Replay(ReplayArgs),
    /// Record raw updates for programs from a Solana RPC node into a
    /// capture (.arcap)
    Capture(capture::CaptureArgs),
    /// Regenerate this protocol's installed client from the local build now
    /// (`a4 dev up` does it after every deploy)
    Sdk(SdkArgs),
    /// Opt a dev wallet in to the project's automations ([dev.automations]):
    /// it signs a consent for the steps, as a user's wallet would
    Consent(ConsentArgs),
}

#[derive(Args, Debug, Clone)]
pub struct ConsentArgs {
    #[command(flatten)]
    pub project: ProjectArgs,
    /// A dev wallet name or address
    pub wallet: String,
    /// The steps it may run, comma-separated (with --revoke: only these)
    #[arg(long, value_delimiter = ',', required_unless_present = "revoke")]
    pub steps: Vec<String>,
    /// The most of an asset one step may move out: `SOL=<lamports>` or
    /// `<mint>=<base units>`; repeat per asset (default SOL=10000000, fees only)
    #[arg(long = "limit", value_name = "ASSET=AMOUNT")]
    pub limits: Vec<String>,
    /// A rolling limit over all steps: `<asset>=<amount>/<window>`, e.g.
    /// `SOL=100000000/30d`; repeatable
    #[arg(long = "window", value_name = "ASSET=AMOUNT/WINDOW")]
    pub windows: Vec<String>,
    /// A setting the handlers read from the grant's params: `key=value`; repeatable
    #[arg(long = "param", value_name = "KEY=VALUE")]
    pub params: Vec<String>,
    /// Only this instance (default: every instance of the deployment)
    #[arg(long)]
    pub instance: Option<String>,
    /// Days until the consent ends
    #[arg(long, default_value_t = 30)]
    pub days: u64,
    /// Revoke the wallet's consent instead
    #[arg(long)]
    pub revoke: bool,
}

#[derive(Args, Debug, Clone, Default)]
pub struct ProjectArgs {
    /// Project directory (default: the current directory, or the nearest
    /// parent with .arete/dev)
    #[arg(long, value_name = "DIR")]
    pub project: Option<PathBuf>,
}

#[derive(Args, Debug, Clone)]
pub struct UpArgs {
    #[command(flatten)]
    pub project: ProjectArgs,

    /// The environment to use, by name or id; created when no environment
    /// has that name (default: the project's, else one named after
    /// [protocol].slug or the project directory)
    #[arg(long = "env", env = "ARETE_DEV_ENV", value_name = "NAME|ID")]
    pub env: Option<String>,

    /// What a new environment starts from (a template name)
    #[arg(long, value_name = "NAME")]
    pub template: Option<String>,

    /// Fee policy document (JSON, `arete.protocol-fee-policy/v1`) for new
    /// deployments (default: [protocol].fee_policy in arete.toml, else no
    /// fees)
    #[arg(long, value_name = "FILE")]
    pub fee_policy: Option<PathBuf>,

    /// Value of a resource the spec declares, as NAME=FILE (JSON); repeatable
    /// (default: [protocol.resources] in arete.toml)
    #[arg(long = "resource", value_name = "NAME=FILE")]
    pub resources: Vec<String>,

    /// First of three consecutive local ports (RPC, its WebSocket, dev UI);
    /// 0 picks free ports
    #[arg(long, default_value_t = 48_700, value_name = "N")]
    pub port_base: u16,

    /// Deploy once and don't watch the project
    #[arg(long)]
    pub no_watch: bool,

    /// Don't deploy the protocol or project stacks (serve the environment and UI only)
    #[arg(long)]
    pub no_deploy: bool,

    /// Don't generate this protocol's installed client from the local build
    #[arg(long)]
    pub no_sdk: bool,

    /// Open the dev UI in the browser once the session is ready
    #[arg(long, conflicts_with = "no_ui")]
    pub open: bool,

    /// Don't serve the dev UI or its event feed (the `dev_events` MCP tool
    /// reads that feed)
    #[arg(long)]
    pub no_ui: bool,

    /// Serve the dev UI's files from this Vite dev server instead of the
    /// ones built into a4 (UI development)
    #[arg(long, value_name = "URL", hide = true)]
    pub ui_dev_url: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct DownArgs {
    #[command(flatten)]
    pub project: ProjectArgs,
    /// Also put the environment to sleep now
    #[arg(long)]
    pub sleep: bool,
    /// Also delete .arete/dev (the environment choice, deploy state,
    /// wallets, history); the environment itself stays (`a4 dev delete`)
    #[arg(long)]
    pub purge: bool,
}

#[derive(Args, Debug, Clone)]
pub struct UseArgs {
    #[command(flatten)]
    pub project: ProjectArgs,
    /// The environment, by name or id
    pub env: String,
}

#[derive(Args, Debug, Clone)]
pub struct CheckpointArgs {
    #[command(flatten)]
    pub env: EnvArgs,
    /// A name for the checkpoint
    #[arg(conflicts_with_all = ["list", "delete"])]
    pub name: Option<String>,
    /// List the environment's checkpoints
    #[arg(long, conflicts_with = "delete")]
    pub list: bool,
    /// Delete a checkpoint (its id or name)
    #[arg(long, value_name = "CHECKPOINT")]
    pub delete: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct BranchArgs {
    #[command(flatten)]
    pub env: EnvArgs,
    /// The checkpoint to branch from (its id or name)
    pub checkpoint: String,
    /// The new environment's name
    pub name: String,
    /// Point this project at the new environment
    #[arg(long = "use")]
    pub switch: bool,
}

#[derive(Args, Debug, Clone)]
pub struct RestoreArgs {
    #[command(flatten)]
    pub env: EnvArgs,
    /// The checkpoint to restore (its id or name)
    pub checkpoint: String,
}

#[derive(Args, Debug, Clone)]
pub struct RebaseArgs {
    #[command(flatten)]
    pub env: EnvArgs,
    /// Also leave this account alone (repeatable)
    #[arg(long, value_name = "PUBKEY")]
    pub keep: Vec<String>,
}

#[derive(Args, Debug, Clone)]
pub struct ResetArgs {
    #[command(flatten)]
    pub env: EnvArgs,
    /// Reset only this account: its next read refetches it from mainnet
    #[arg(long, value_name = "PUBKEY")]
    pub account: Option<String>,
    /// With --account: also the accounts it owns
    #[arg(long, requires = "account")]
    pub include_owned: bool,
}

#[derive(Args, Debug, Clone)]
pub struct FundArgs {
    #[command(flatten)]
    pub env: EnvArgs,
    /// A base58 public key, or a dev wallet name (created on first use)
    pub account: String,
    /// The SOL balance to set (default 100)
    #[arg(conflicts_with = "mint")]
    pub sol: Option<String>,
    /// Set a token balance of this mint instead
    #[arg(long, value_name = "MINT", requires = "amount")]
    pub mint: Option<String>,
    /// With --mint: the balance in base units
    #[arg(long, value_name = "BASE_UNITS")]
    pub amount: Option<String>,
    /// With --mint: the token program (default SPL Token; pass the
    /// Token-2022 program id for Token-2022 mints)
    #[arg(long, value_name = "PROGRAM", requires = "mint")]
    pub token_program: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct PinArgs {
    #[command(flatten)]
    pub env: EnvArgs,
    /// The account (base58)
    pub account: String,
    /// Also the accounts it owns
    #[arg(long, conflicts_with = "unpin")]
    pub include_owned: bool,
    /// Unpin it
    #[arg(long)]
    pub unpin: bool,
}

#[derive(Args, Debug, Clone)]
pub struct FollowArgs {
    #[command(flatten)]
    pub env: EnvArgs,
    /// The account (base58)
    pub account: String,
    /// Stop following it
    #[arg(long)]
    pub stop: bool,
}

#[derive(Args, Debug, Clone)]
pub struct WarpArgs {
    #[command(flatten)]
    pub env: EnvArgs,
    /// +<n>[s|m|h|d], a Unix timestamp in seconds, slot:<n> or epoch:<n>
    #[arg(allow_hyphen_values = true)]
    pub to: String,
}

#[derive(Args, Debug, Clone)]
pub struct ScenarioArgs {
    #[command(flatten)]
    pub env: EnvArgs,
    /// The scenario document (a JSON file, or `@file`)
    pub file: String,
    /// Only encode the writes; change nothing
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Args, Debug, Clone)]
pub struct PreviewArgs {
    #[command(flatten)]
    pub env: EnvArgs,
    /// The transaction, base64 (or `@file` holding it)
    pub transaction: String,
}

#[derive(Args, Debug, Clone)]
pub struct ReproArgs {
    #[command(flatten)]
    pub env: EnvArgs,
    /// The transaction's signature
    pub signature: String,
    /// Where to write the bundle (default repro-<signature>.json)
    #[arg(short = 'o', long, value_name = "FILE")]
    pub output: Option<PathBuf>,
}

#[derive(Args, Debug, Clone)]
pub struct EventsArgs {
    #[command(flatten)]
    pub env: EnvArgs,
    /// Only events after this event id
    #[arg(long)]
    pub after: Option<i64>,
    /// At most this many (1-500)
    #[arg(long)]
    pub limit: Option<u32>,
    /// Keep printing new events
    #[arg(long, short = 'f')]
    pub follow: bool,
}

#[derive(Args, Debug, Clone)]
pub struct StepArgs {
    #[command(flatten)]
    pub project: ProjectArgs,
    /// Step name from the protocol spec
    pub step: String,
    /// Instance id (instance steps); omit for create steps
    #[arg(long)]
    pub instance: Option<String>,
    /// Step args as a JSON object
    #[arg(long, default_value = "{}", value_name = "JSON")]
    pub args: String,
    /// Dev wallet that signs (.arete/dev/wallets/<name>.json; created and
    /// funded on first use)
    #[arg(long, default_value = "default")]
    pub wallet: String,
}

#[derive(Args, Debug, Clone)]
pub struct WalletArgs {
    #[command(flatten)]
    pub project: ProjectArgs,
    /// Dev wallet name (.arete/dev/wallets/<name>.json)
    #[arg(default_value = "default")]
    pub name: String,
}

#[derive(Args, Debug, Clone)]
pub struct InstanceArgs {
    #[command(flatten)]
    pub project: ProjectArgs,
    /// Instance id
    pub id: String,
    /// Evaluate allowedSteps for this wallet (a dev wallet name or a public key)
    #[arg(long)]
    pub wallet: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct SdkArgs {
    #[command(flatten)]
    pub project: ProjectArgs,
    /// TypeScript client
    #[arg(long)]
    pub ts: bool,
    /// Rust client (a crate)
    #[arg(long)]
    pub rust: bool,
    /// Python client (a package)
    #[arg(long)]
    pub python: bool,
}

#[derive(Args, Debug, Clone)]
pub struct ReplayArgs {
    #[command(flatten)]
    pub project: ProjectArgs,
    /// Protocol steps: .arete/dev/history.jsonl, a JSON array of steps, or
    /// {"schema":"arete.dev-replay/v1","steps":[…]}
    pub file: PathBuf,
    /// Also run entries recorded as rejected or failed (skipped by default)
    #[arg(long)]
    pub include_failed: bool,
}

pub fn run(command: DevCommands, json: bool) -> Result<()> {
    match command {
        DevCommands::AdmitLocalPrograms => up::admission_worker(),
        DevCommands::Up(args) => up(args, json),
        DevCommands::Down(args) => down(args, json),
        DevCommands::Status(args) => block_on(status(args, json)),
        DevCommands::List => env::list(json),
        DevCommands::Use(args) => env::use_env(args.project, args.env, json),
        DevCommands::Wake(args) => env::wake(args, json),
        DevCommands::Sleep(args) => env::sleep(args, json),
        DevCommands::Delete(args) => env::delete(args, json),
        DevCommands::Checkpoint(args) => {
            env::checkpoint(args.env, args.name, args.list, args.delete, json)
        }
        DevCommands::Branch(args) => {
            env::branch(args.env, args.checkpoint, args.name, args.switch, json)
        }
        DevCommands::Restore(args) => env::restore(args.env, args.checkpoint, json),
        DevCommands::Rebase(args) => env::rebase(args.env, args.keep, json),
        DevCommands::Reset(args) => env::reset(args.env, args.account, args.include_owned, json),
        DevCommands::Fund(args) => env::fund(
            args.env,
            args.account,
            args.sol,
            args.mint,
            args.amount,
            args.token_program,
            json,
        ),
        DevCommands::Pin(args) => {
            env::pin(args.env, args.account, args.include_owned, args.unpin, json)
        }
        DevCommands::Follow(args) => env::follow(args.env, args.account, args.stop, json),
        DevCommands::Warp(args) => env::warp(args.env, args.to, json),
        DevCommands::Pause(args) => env::clock(args, true, json),
        DevCommands::Resume(args) => env::clock(args, false, json),
        DevCommands::Scenario(args) => env::scenario(args.env, args.file, args.dry_run, json),
        DevCommands::Preview(args) => env::preview(args.env, args.transaction, json),
        DevCommands::Repro(args) => env::repro(args.env, args.signature, args.output, json),
        DevCommands::Events(args) => {
            env::events(args.env, args.after, args.limit, args.follow, json)
        }
        DevCommands::Token(args) => env::token(args, json),
        DevCommands::Step(args) => block_on(async move {
            let client = client(&args.project)?;
            let step_args: Value =
                serde_json::from_str(&args.args).context("--args must be a JSON object")?;
            if !step_args.is_object() {
                bail!("--args must be a JSON object");
            }
            let request = StepRequest {
                step: args.step,
                instance: args.instance,
                args: step_args,
                wallet: args.wallet,
                result: None,
            };
            let work = client.step_with_progress(&request, |phase| {
                eprintln!("{}", step_progress_message(phase));
            });
            let result =
                finish_step(work, json, &mut std::io::stdout(), &mut std::io::stderr()).await?;
            if result["state"] != "completed" {
                std::process::exit(1);
            }
            Ok(())
        }),
        DevCommands::Wallet(args) => block_on(async move {
            let client = client(&args.project)?;
            let wallet = client.wallet(&args.name).await.map_err(env::explain)?;
            let balance = client
                .rpc(
                    "getBalance",
                    json!([wallet.pubkey(), { "commitment": "processed" }]),
                )
                .await?
                .get("value")
                .cloned()
                .unwrap_or(Value::Null);
            let result = json!({
                "name": args.name,
                "pubkey": wallet.pubkey(),
                "lamports": balance,
                "keypair": client.project.wallets_dir().join(format!("{}.json", args.name)).display().to_string(),
            });
            print(json, &result, || {
                println!(
                    "{} {} ({} lamports)",
                    args.name.bold(),
                    result["pubkey"].as_str().unwrap_or_default(),
                    balance
                )
            });
            Ok(())
        }),
        DevCommands::Instance(args) => block_on(async move {
            let result = client(&args.project)?
                .instance(&args.id, args.wallet.as_deref())
                .await?;
            print(json, &result, || {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&result).unwrap_or_default()
                )
            });
            Ok(())
        }),
        DevCommands::Capture(args) => capture::capture(args, json),
        DevCommands::Sdk(args) => sdk(args, json),
        DevCommands::Consent(args) => consent(args, json),
        DevCommands::Replay(args) => block_on(async move {
            let text = std::fs::read_to_string(&args.file)
                .with_context(|| format!("Failed to read {}", args.file.display()))?;
            let steps = parse_replay(&text)?;
            if steps.is_empty() {
                bail!("{} has no steps", args.file.display());
            }
            let options = ReplayOptions {
                include_failed: args.include_failed,
            };
            let result = client(&args.project)?.replay(&steps, options).await?;
            print(json, &result, || {
                for skipped in result["skipped"].as_array().into_iter().flatten() {
                    println!(
                        "{} {} (entry {}, recorded {}; --include-failed runs it)",
                        "skip".dimmed(),
                        skipped["step"].as_str().unwrap_or_default(),
                        skipped["entry"],
                        skipped["recordedState"].as_str().unwrap_or("failed")
                    );
                }
                for step in result["results"].as_array().into_iter().flatten() {
                    print_step(step);
                }
            });
            if result["completed"] != true {
                std::process::exit(1);
            }
            Ok(())
        }),
    }
}

fn block_on<T, F: std::future::Future<Output = Result<T>>>(future: F) -> Result<T> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("Failed to start the async runtime")?
        .block_on(future)
}

fn print(json: bool, value: &Value, human: impl FnOnce()) {
    if json {
        println!("{value}");
    } else {
        human();
    }
}

/// Await and print exactly one step outcome. Keep the timer and output
/// routing together so JSON output cannot be contaminated by heartbeats.
async fn finish_step(
    work: impl std::future::Future<Output = Result<Value>>,
    json: bool,
    stdout: &mut impl std::io::Write,
    stderr: &mut impl std::io::Write,
) -> Result<Value> {
    let started = tokio::time::Instant::now();
    let period = std::time::Duration::from_secs(5);
    let mut heartbeat = tokio::time::interval_at(started + period, period);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    tokio::pin!(work);
    let result = loop {
        tokio::select! {
            // If completion and a tick become ready together, stop first.
            biased;
            result = &mut work => break result.map_err(env::explain)?,
            _ = heartbeat.tick() => writeln!(
                stderr,
                "Still waiting ({}s). Do not resubmit while this execution is pending.",
                started.elapsed().as_secs()
            )?,
        }
    };
    if let Some(message) = step_outcome_message(&result) {
        writeln!(stderr, "{message}")?;
    }
    if json {
        serde_json::to_writer(&mut *stdout, &result)?;
        writeln!(stdout)?;
    } else {
        print_step(&result);
    }
    Ok(result)
}

fn step_progress_message(phase: StepProgress<'_>) -> String {
    match phase {
        StepProgress::Preparing => "Preparing and simulating step…".into(),
        StepProgress::Signing => "Signing with dev wallet…".into(),
        StepProgress::Submitting { execution_id } => {
            format!("Submitting execution {execution_id}…")
        }
        StepProgress::Confirming { execution_id } => {
            format!("Waiting for confirmation of {execution_id}…")
        }
    }
}

#[cfg(test)]
mod step_progress_tests {
    use super::*;

    #[derive(Clone, Default)]
    struct Output(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl Output {
        fn text(&self) -> String {
            String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
        }
    }

    impl std::io::Write for Output {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[tokio::test(start_paused = true)]
    async fn slow_step_heartbeats_go_only_to_stderr_until_one_json_result() {
        let stdout = Output::default();
        let stderr = Output::default();
        let (mut out, mut err) = (stdout.clone(), stderr.clone());
        let task = tokio::spawn(async move {
            finish_step(
                async {
                    tokio::time::sleep(std::time::Duration::from_secs(12)).await;
                    Ok(json!({"state": "completed", "executionId": "e-1"}))
                },
                true,
                &mut out,
                &mut err,
            )
            .await
            .unwrap()
        });
        tokio::task::yield_now().await;
        tokio::time::advance(std::time::Duration::from_secs(4)).await;
        tokio::task::yield_now().await;
        assert!(stderr.text().is_empty(), "no immediate or early heartbeat");
        assert!(stdout.text().is_empty(), "no result before completion");
        for elapsed in [5, 10] {
            tokio::time::advance(std::time::Duration::from_secs(if elapsed == 5 {
                1
            } else {
                5
            }))
            .await;
            tokio::task::yield_now().await;
            assert!(stderr
                .text()
                .contains(&format!("Still waiting ({elapsed}s)")));
            assert!(
                stdout.text().is_empty(),
                "progress must not reach JSON stdout"
            );
        }
        tokio::time::advance(std::time::Duration::from_secs(2)).await;
        let result = task.await.unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&stdout.text()).unwrap(),
            result
        );
        assert_eq!(stdout.text().lines().count(), 1);
        let finished = stderr.text();
        assert_eq!(finished.lines().count(), 2);
        tokio::time::advance(std::time::Duration::from_secs(20)).await;
        assert_eq!(stderr.text(), finished, "no heartbeat after completion");
    }

    #[tokio::test(start_paused = true)]
    async fn completion_at_a_tick_wins_and_expiry_stays_out_of_json_stdout() {
        let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
        let result = finish_step(
            async {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                Ok(json!({"state": "expired"}))
            },
            true,
            &mut stdout,
            &mut stderr,
        )
        .await
        .unwrap();
        let err = String::from_utf8(stderr).unwrap();
        assert!(
            !err.contains("Still waiting"),
            "completion must win at the tick boundary"
        );
        assert!(err.contains("Execution expired before confirmation"));
        let out = String::from_utf8(stdout).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&out).unwrap(), result);
        assert_eq!(out.lines().count(), 1);
    }

    #[test]
    fn expiry_is_not_presented_as_a_successful_payment() {
        let message = step_outcome_message(&json!({"state": "expired"})).unwrap();
        assert!(message.contains("not a completed payment"));
        assert!(message.contains("inspect"));
        assert!(step_outcome_message(&json!({"state": "completed"})).is_none());
        assert!(step_progress_message(StepProgress::Submitting {
            execution_id: "public-id"
        })
        .contains("public-id"));
    }
}

fn step_outcome_message(result: &Value) -> Option<&'static str> {
    (result["state"] == "expired").then_some(
        "Execution expired before confirmation. This is not a completed payment; inspect its transaction status before trying again."
    )
}

fn print_step(result: &Value) {
    let state = result["state"].as_str().unwrap_or("unknown");
    let marker = if state == "completed" {
        state.green()
    } else {
        state.red()
    };
    println!(
        "{} {} by {} -> {}",
        "step".bold(),
        result["step"].as_str().unwrap_or_default(),
        result["wallet"]["name"].as_str().unwrap_or_default(),
        marker
    );
    for (label, key) in [
        ("instance", "instanceId"),
        ("execution", "executionId"),
        ("rejected", "rejected"),
    ] {
        if let Some(value) = result[key].as_str() {
            println!("  {label:<10} {value}");
        }
    }
    for transaction in result["transactions"].as_array().into_iter().flatten() {
        if let Some(signature) = transaction["signature"].as_str() {
            println!("  signature  {signature}");
        }
        if let Some(error) = transaction["error"].as_str() {
            println!("  error      {error}");
        }
    }
}

/// `a4 dev down`.
fn down(args: DownArgs, json: bool) -> Result<()> {
    let root = project_root(&args.project)?;
    let selection = DevProject::new(&root).load_selection().ok().flatten();
    let report = up::down(&root, args.purge)?;
    let mut slept = None;
    if args.sleep {
        if let Some(selection) = &selection {
            slept = Some(block_on(async {
                let api = arete_mcp::dev_env::EnvApi::for_origin(
                    reqwest::Client::new(),
                    &selection.api_url,
                )?;
                api.sleep(&selection.id).await.map_err(env::explain)
            })?);
        }
    }
    let mut report = report;
    report["slept"] = json!(slept.as_ref().map(arete_mcp::dev_env::public_view));
    print(json, &report, || {
        let stopped = report["stopped"].as_array().map(Vec::len).unwrap_or(0);
        if stopped == 0 {
            println!("No a4 dev up was running.");
        } else {
            println!("Stopped a4 dev up.");
        }
        if let Some(env) = &slept {
            println!("{}", arete_mcp::dev_env::describe_env(env));
        }
        if args.purge {
            println!(
                "Deleted .arete/dev (the environment itself stays; `a4 dev delete` deletes it)."
            );
        }
    });
    Ok(())
}

/// `a4 dev sdk`: regenerate this protocol's installed client from the
/// local build now. `--ts`/`--rust`/`--python` choose the targets when
/// arete.toml does not install the protocol yet (else `[dev] sdk`, else
/// TypeScript); an installed dependency keeps its own.
/// The JSON `a4 dev consent` sends to the running automations.
fn consent_request(args: &ConsentArgs, now_ms: u128) -> Result<Value> {
    fn pair<'a>(text: &'a str, what: &str) -> Result<(&'a str, &'a str)> {
        text.split_once('=')
            .filter(|(key, value)| !key.is_empty() && !value.is_empty())
            .with_context(|| format!("{what} `{text}` is not KEY=VALUE"))
    }
    let mut per_execution = serde_json::Map::new();
    for limit in &args.limits {
        let (asset, amount) = pair(limit, "--limit")?;
        per_execution.insert(asset.into(), json!(amount));
    }
    if per_execution.is_empty() && !args.revoke {
        per_execution.insert("SOL".into(), json!("10000000"));
    }
    let mut per_window = serde_json::Map::new();
    for window in &args.windows {
        let (asset, rest) = pair(window, "--window")?;
        let (amount, window) = rest
            .split_once('/')
            .with_context(|| format!("--window `{window}` is not ASSET=AMOUNT/WINDOW"))?;
        per_window.insert(asset.into(), json!({ "amount": amount, "window": window }));
    }
    let mut params = serde_json::Map::new();
    for param in &args.params {
        let (key, value) = pair(param, "--param")?;
        params.insert(key.into(), json!(value));
    }
    let mut limits = json!({ "perExecution": per_execution });
    if !per_window.is_empty() {
        limits["perWindow"] = Value::Object(per_window);
    }
    Ok(json!({
        "wallet": args.wallet,
        "steps": args.steps,
        "limits": limits,
        "params": params,
        "instanceId": args.instance,
        "expiresAt": now_ms + u128::from(args.days) * 86_400_000,
        "revoke": args.revoke,
    }))
}

/// `a4 dev consent`: sign a consent grant with a dev wallet, through the
/// automations `a4 dev up` runs.
fn consent(args: ConsentArgs, json: bool) -> Result<()> {
    let root = project_root(&args.project)?;
    let (port, token) = automations::control(&root)?;
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis();
    let request = consent_request(&args, now_ms)?;
    let response = reqwest::blocking::Client::new()
        .post(format!("http://127.0.0.1:{port}/consent"))
        .bearer_auth(token)
        .json(&request)
        .send()
        .context("Could not reach the automations `a4 dev up` runs")?;
    let ok = response.status().is_success();
    let body: Value = response.json().unwrap_or(Value::Null);
    if !ok {
        bail!(
            "{}",
            body["error"]
                .as_str()
                .unwrap_or("the automations refused the consent")
        );
    }
    if json {
        println!("{body}");
    } else if args.revoke {
        println!(
            "Revoked {} consent grant(s) of {}",
            body["revoked"],
            body["wallet"].as_str().unwrap_or(&args.wallet)
        );
    } else {
        println!(
            "{} consented to {} until {} days from now",
            body["grant"]["wallet"].as_str().unwrap_or(&args.wallet),
            args.steps.join(", "),
            args.days
        );
        for warning in body["warnings"].as_array().into_iter().flatten() {
            if let Some(warning) = warning.as_str() {
                println!("  note: {warning}");
            }
        }
    }
    Ok(())
}

fn sdk(args: SdkArgs, json: bool) -> Result<()> {
    use crate::project::manifest::InstallTarget;
    let root = project_root(&args.project)?;
    let mut targets = Vec::new();
    for (chosen, target) in [
        (args.ts, InstallTarget::TypeScript),
        (args.rust, InstallTarget::Rust),
        (args.python, InstallTarget::Python),
    ] {
        if chosen {
            targets.push(target);
        }
    }
    if targets.is_empty() {
        targets = sdk_gen::configured_targets(&root)?;
    }
    let protocol = sdk_gen::LocalProtocol::from_project(&root)?;
    let api_base = DevProject::new(&root)
        .load_selection()
        .ok()
        .flatten()
        .map(|selection| selection.api_url)
        .unwrap_or_else(ApiClient::configured_api_url);
    let report = sdk_gen::sync_installed_client(&root, &protocol, &targets, &api_base)?;
    print(json, &serde_json::to_value(&report)?, || {
        if report.added_dependency {
            println!(
                "{} added [dependencies.protocols.{}] to arete.toml",
                "ok".green(),
                report.install.alias
            );
        }
        println!("{} {}", "ok".green(), report.summary());
        println!(
            "{}",
            "The app calls the deployment in your dev environment with your Arete key. `a4 install --locked` restores the published client.".dimmed()
        );
    });
    Ok(())
}

fn project_root(args: &ProjectArgs) -> Result<PathBuf> {
    let root = match &args.project {
        Some(path) => path.clone(),
        None => std::env::current_dir()?,
    };
    std::fs::canonicalize(&root).with_context(|| format!("{} does not exist", root.display()))
}

fn client(args: &ProjectArgs) -> Result<DevClient> {
    DevClient::open(DevProject::discover(args.project.as_deref())?)
}

/// `a4 dev status`.
async fn status(args: EnvArgs, json: bool) -> Result<()> {
    let mut report = if args.env.is_none() {
        match DevProject::discover(args.project.project.as_deref()).and_then(DevClient::open) {
            Ok(client) => {
                let mut report = client.status().await;
                let state = up::load_state(&client.project);
                report["up"] = match state {
                    Some(state) if runtime::pid_alive(state.pid) => {
                        json!({ "pid": state.pid, "ports": state.ports })
                    }
                    _ => Value::Null,
                };
                let deploy = deploy::DeployState::load(&client.project.deploy_state_path());
                report["deployments"] = json!(deploy
                    .deployment_states()
                    .into_iter()
                    .map(|(id, state)| json!({ "id": id, "state": state }))
                    .collect::<Vec<_>>());
                report
            }
            Err(error) => {
                if json {
                    println!(
                        "{}",
                        json!({ "environment": null, "error": format!("{error:#}") })
                    );
                    return Ok(());
                }
                return Err(error);
            }
        }
    } else {
        let rpc = args.rpc().await?;
        let view = rpc.api().get_env(rpc.env()).await.map_err(env::explain)?;
        json!({ "environment": arete_mcp::dev_env::public_view(&view) })
    };
    if report["environment"]["state"] == "running" && report.get("chain").is_none() {
        if let Ok(rpc) = args.rpc().await {
            if let Ok((slot, unix)) = arete_mcp::dev_env::read_clock(&rpc).await {
                report["chain"] = json!({ "healthy": true, "slot": slot, "unixTimestamp": unix });
            }
        }
    }
    print(json, &report, || print_status(&report));
    Ok(())
}

fn print_status(report: &Value) {
    let env = &report["environment"];
    if let Some(error) = env["error"].as_str() {
        println!("{} {}", "environment".bold(), error.red());
    } else {
        println!(
            "{} {}",
            "environment".bold(),
            arete_mcp::dev_env::describe_env(env)
        );
        if let Some(slept) = env["sleptAt"].as_str() {
            if env["state"] == "sleeping" {
                println!("  slept       {slept} (any call or `a4 dev up` wakes it)");
            }
        }
        if let Some(bytes) = env["storageBytes"].as_i64() {
            println!("  storage     {bytes} bytes");
        }
    }
    let session = &report["session"];
    match (&report["up"], session["rpcUrl"].as_str()) {
        (up, Some(rpc)) if !up.is_null() && !rpc.is_empty() => {
            println!("  a4 dev up   running (pid {})", up["pid"]);
            println!("  RPC         {rpc}");
            println!(
                "  WebSocket   {}",
                session["rpcWsUrl"].as_str().unwrap_or_default()
            );
            if let Some(ui) = session["uiUrl"].as_str() {
                println!("  UI          {ui}");
            }
        }
        _ => {
            if !session.is_null() {
                println!("  a4 dev up   not running");
            }
        }
    }
    if let Some(url) = session["environmentRpcUrl"]
        .as_str()
        .filter(|url| !url.is_empty())
    {
        println!("  Env RPC     {url}   (needs a token: `a4 dev token`)");
    }
    if let Some(deployment) = session["deploymentId"].as_str() {
        println!("  deployment  {deployment} (active)");
    }
    for earlier in report["deployments"].as_array().into_iter().flatten() {
        let state = earlier["state"].as_str().unwrap_or_default();
        if state == "active" {
            continue;
        }
        let state = if state == "disabled" {
            state.dimmed()
        } else {
            format!("{state} (disable failed; retried on the next deploy)").yellow()
        };
        println!(
            "              {} {state}",
            earlier["id"].as_str().unwrap_or_default()
        );
    }
    if let Some(url) = report["stack"]["streamUrl"].as_str() {
        println!(
            "  stream      a4 stream --dev   (lists the protocol's views; --tui browses them; mints its own tokens)"
        );
        println!("  stream URL  {url}");
    } else if let Some(error) = report["stack"]["error"].as_str() {
        println!("  stream      {}", error.red());
    }
    let chain = &report["chain"];
    if chain["healthy"] == true {
        println!(
            "  chain       slot {} clock {}",
            chain["slot"], chain["unixTimestamp"]
        );
    } else if let Some(error) = chain["error"].as_str() {
        println!("  chain       {}", error.red());
    }
}

fn up(args: UpArgs, json: bool) -> Result<()> {
    let project_root = project_root(&args.project)?;
    let has_protocol = project_root.join(deploy::ENTRY).is_file();
    let mut checks = Vec::new();
    let mut warnings = Vec::new();
    if has_protocol && !args.no_deploy {
        let node = tools::check_node()?;
        // Fail before anything starts when the project can't build.
        let arete_protocol = crate::commands::protocol_bin::resolve(&project_root)?;
        checks.push(json!({ "tool": "node", "version": node.detail }));
        checks.push(json!({ "tool": "arete-protocol", "path": arete_protocol }));
        warnings.extend(node.warning);
    }
    let settings = if has_protocol {
        deploy_settings(&project_root, &args)?
    } else {
        deploy::DeploySettings::default()
    };
    let api_url = ApiClient::configured_api_url();
    let key = ApiClient::load_optional_api_key_for_url(&api_url)?.ok_or_else(|| {
        anyhow::anyhow!(
            "Cloud dev environments need an Arete secret key: run `a4 auth login` (or `a4 auth signup`), or set ARETE_API_KEY."
        )
    })?;
    let api = arete_mcp::dev_env::EnvApi::new(reqwest::Client::new(), &api_url, &key)?;
    if json {
        println!(
            "{}",
            json!({ "event": "checks", "tools": checks, "warnings": warnings, "api": api.origin() })
        );
    } else {
        for check in &checks {
            println!(
                "{} {} {}",
                "ok".green(),
                check["tool"].as_str().unwrap_or_default(),
                check["version"]
                    .as_str()
                    .or_else(|| check["path"].as_str())
                    .unwrap_or_default()
            );
        }
        for warning in &warnings {
            eprintln!("{} {warning}", "warning:".yellow().bold());
        }
    }
    let config = up::UpConfig {
        project_root: project_root.clone(),
        protocol: has_protocol,
        api,
        environment: args.env.clone(),
        template: args.template.clone(),
        settings,
        port_base: args.port_base,
        watch: !args.no_watch,
        deploy: !args.no_deploy,
        sync_sdk: has_protocol && !args.no_sdk && !args.no_deploy,
        sdk_targets: if has_protocol {
            sdk_gen::configured_targets(&project_root)?
        } else {
            Vec::new()
        },
        json,
        ui: !args.no_ui,
        open_ui: args.open,
        ui_dev_url: args.ui_dev_url.clone(),
    };
    block_on(up::run(config))
}

/// Flags over the `[protocol]` section of `arete.toml`; config paths
/// resolve against the project, flag paths against the working directory.
fn deploy_settings(
    project_root: &std::path::Path,
    args: &UpArgs,
) -> Result<deploy::DeploySettings> {
    let config = crate::commands::protocol::load_protocol_config(project_root)?;
    let mut resources: std::collections::BTreeMap<String, PathBuf> = config
        .resources
        .iter()
        .map(|(name, path)| (name.clone(), project_root.join(path)))
        .collect();
    for entry in &args.resources {
        let (name, path) = entry
            .split_once('=')
            .filter(|(name, path)| !name.is_empty() && !path.is_empty())
            .ok_or_else(|| anyhow::anyhow!("--resource expects NAME=FILE, got '{entry}'"))?;
        resources.insert(name.to_string(), PathBuf::from(path));
    }
    if let Some(slug) = &config.slug {
        crate::commands::protocol::validate_slug(slug)?;
    }
    Ok(deploy::DeploySettings {
        slug: config.slug,
        name: config.name,
        fee_policy: args
            .fee_policy
            .clone()
            .or_else(|| config.fee_policy.map(|path| project_root.join(path))),
        resources,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use std::path::Path;

    #[derive(Parser, Debug)]
    struct Cli {
        #[command(subcommand)]
        command: DevCommands,
    }

    fn parse(args: &[&str]) -> Result<DevCommands, clap::Error> {
        let mut argv = vec!["dev"];
        argv.extend_from_slice(args);
        Cli::try_parse_from(argv).map(|cli| cli.command)
    }

    #[test]
    fn builds_a_consent_request() {
        let DevCommands::Consent(args) = parse(&[
            "consent",
            "alice",
            "--steps",
            "refund,claim",
            "--limit",
            "SOL=5000",
            "--window",
            "SOL=20000/30d",
            "--param",
            "weekday=fri",
            "--days",
            "2",
        ])
        .unwrap() else {
            panic!("not consent");
        };
        assert_eq!(
            consent_request(&args, 1_000).unwrap(),
            json!({
                "wallet": "alice",
                "steps": ["refund", "claim"],
                "limits": {
                    "perExecution": { "SOL": "5000" },
                    "perWindow": { "SOL": { "amount": "20000", "window": "30d" } },
                },
                "params": { "weekday": "fri" },
                "instanceId": null,
                "expiresAt": 1_000 + 2 * 86_400_000u64,
                "revoke": false,
            })
        );
        // Fees only by default; --revoke needs no steps.
        let DevCommands::Consent(args) = parse(&["consent", "alice", "--steps", "refund"]).unwrap()
        else {
            panic!("not consent");
        };
        assert_eq!(
            consent_request(&args, 0).unwrap()["limits"],
            json!({ "perExecution": { "SOL": "10000000" } })
        );
        assert!(parse(&["consent", "alice"]).is_err());
        let DevCommands::Consent(args) = parse(&["consent", "alice", "--revoke"]).unwrap() else {
            panic!("not consent");
        };
        assert_eq!(consent_request(&args, 0).unwrap()["revoke"], true);
        let DevCommands::Consent(bad) =
            parse(&["consent", "alice", "--steps", "x", "--limit", "SOL"]).unwrap()
        else {
            panic!("not consent");
        };
        assert!(consent_request(&bad, 0).is_err());
    }

    #[test]
    fn parses_up_flags() {
        let DevCommands::Up(args) = parse(&["up"]).unwrap() else {
            panic!("expected up");
        };
        assert_eq!(args.port_base, 48_700);
        assert!(!args.no_watch && !args.no_deploy && !args.no_ui);

        let DevCommands::Up(args) = parse(&[
            "up",
            "--project",
            "/tmp/p",
            "--env",
            "staging",
            "--template",
            "dex",
            "--no-deploy",
            "--port-base",
            "0",
        ])
        .unwrap() else {
            panic!("expected up");
        };
        assert_eq!(args.project.project.as_deref(), Some(Path::new("/tmp/p")));
        assert_eq!(args.env.as_deref(), Some("staging"));
        assert_eq!(args.template.as_deref(), Some("dex"));
        assert!(args.no_deploy);
        assert_eq!(args.port_base, 0);
        // The local runtime's flags are gone.
        for gone in [
            "--arete-dev-bin",
            "--geyser-plugin",
            "--rpc-url",
            "--offline",
            "--accept-eula",
        ] {
            assert!(parse(&["up", gone, "x"]).is_err(), "{gone}");
        }
        assert!(parse(&["up", "--open", "--no-ui"]).is_err());
    }

    #[test]
    fn parses_environment_commands() {
        assert!(matches!(parse(&["list"]).unwrap(), DevCommands::List));
        assert!(matches!(
            parse(&["status", "--env", "app"]).unwrap(),
            DevCommands::Status(EnvArgs { ref env, .. }) if env.as_deref() == Some("app")
        ));
        for command in ["wake", "sleep", "delete", "pause", "resume", "token"] {
            assert!(parse(&[command]).is_ok(), "{command}");
        }
        assert!(matches!(
            parse(&["checkpoint", "funded"]).unwrap(),
            DevCommands::Checkpoint(CheckpointArgs { ref name, list: false, .. }) if name.as_deref() == Some("funded")
        ));
        assert!(matches!(
            parse(&["checkpoint", "--list"]).unwrap(),
            DevCommands::Checkpoint(CheckpointArgs { list: true, .. })
        ));
        assert!(parse(&["checkpoint", "x", "--list"]).is_err());
        assert!(matches!(
            parse(&["checkpoint", "--delete", "ckpt_ab"]).unwrap(),
            DevCommands::Checkpoint(CheckpointArgs { ref delete, .. }) if delete.as_deref() == Some("ckpt_ab")
        ));
        assert!(matches!(
            parse(&["branch", "funded", "app-b", "--use"]).unwrap(),
            DevCommands::Branch(BranchArgs { switch: true, ref name, .. }) if name == "app-b"
        ));
        assert!(parse(&["branch", "funded"]).is_err());
        assert!(matches!(
            parse(&["restore", "funded"]).unwrap(),
            DevCommands::Restore(_)
        ));
        assert!(matches!(
            parse(&["rebase", "--keep", "A", "--keep", "B"]).unwrap(),
            DevCommands::Rebase(RebaseArgs { ref keep, .. }) if keep.len() == 2
        ));
        assert!(matches!(
            parse(&["reset", "--account", "A", "--include-owned"]).unwrap(),
            DevCommands::Reset(ResetArgs {
                include_owned: true,
                ..
            })
        ));
        assert!(parse(&["reset", "--include-owned"]).is_err());
        assert!(matches!(
            parse(&["fund", "default", "5"]).unwrap(),
            DevCommands::Fund(FundArgs { ref sol, .. }) if sol.as_deref() == Some("5")
        ));
        assert!(matches!(
            parse(&["fund", "default", "--mint", "M", "--amount", "100"]).unwrap(),
            DevCommands::Fund(FundArgs { ref mint, .. }) if mint.as_deref() == Some("M")
        ));
        assert!(parse(&["fund", "default", "--mint", "M"]).is_err());
        assert!(parse(&["fund", "default", "5", "--mint", "M", "--amount", "1"]).is_err());
        assert!(matches!(
            parse(&["pin", "A", "--unpin"]).unwrap(),
            DevCommands::Pin(PinArgs { unpin: true, .. })
        ));
        assert!(parse(&["pin", "A", "--unpin", "--include-owned"]).is_err());
        assert!(matches!(
            parse(&["follow", "A", "--stop"]).unwrap(),
            DevCommands::Follow(FollowArgs { stop: true, .. })
        ));
        assert!(matches!(
            parse(&["warp", "+2h"]).unwrap(),
            DevCommands::Warp(WarpArgs { ref to, .. }) if to == "+2h"
        ));
        assert!(matches!(
            parse(&["time-travel", "slot:5"]).unwrap(),
            DevCommands::Warp(_)
        ));
        assert!(matches!(
            parse(&["scenario", "crash.json", "--dry-run"]).unwrap(),
            DevCommands::Scenario(ScenarioArgs { dry_run: true, .. })
        ));
        assert!(matches!(
            parse(&["repro", "sig", "-o", "r.json"]).unwrap(),
            DevCommands::Repro(_)
        ));
        assert!(matches!(
            parse(&["events", "-f", "--after", "3"]).unwrap(),
            DevCommands::Events(EventsArgs {
                follow: true,
                after: Some(3),
                ..
            })
        ));
        assert!(matches!(
            parse(&["use", "app-b"]).unwrap(),
            DevCommands::Use(_)
        ));
        assert!(matches!(
            parse(&["down", "--sleep", "--purge"]).unwrap(),
            DevCommands::Down(DownArgs {
                sleep: true,
                purge: true,
                ..
            })
        ));
        // Local runtime commands are gone.
        for gone in [
            "fetch",
            "logs",
            "set-account",
            "inspect",
            "state",
            "diff",
            "stack",
        ] {
            assert!(parse(&[gone]).is_err(), "{gone}");
        }
    }

    #[test]
    fn parses_protocol_commands() {
        let DevCommands::Step(step) = parse(&[
            "step",
            "open",
            "--args",
            "{\"amount\":\"5\"}",
            "--wallet",
            "bob",
        ])
        .unwrap() else {
            panic!("expected step");
        };
        assert_eq!(step.step, "open");
        assert_eq!(step.wallet, "bob");
        assert!(step.instance.is_none());
        assert!(matches!(
            parse(&["replay", "h.jsonl", "--include-failed"]).unwrap(),
            DevCommands::Replay(ReplayArgs {
                include_failed: true,
                ..
            })
        ));
        assert!(matches!(
            parse(&["wallet", "bob"]).unwrap(),
            DevCommands::Wallet(WalletArgs { ref name, .. }) if name == "bob"
        ));
        assert!(matches!(
            parse(&["instance", "i-1"]).unwrap(),
            DevCommands::Instance(_)
        ));
        assert!(matches!(
            parse(&["sdk", "--ts", "--python"]).unwrap(),
            DevCommands::Sdk(SdkArgs {
                ts: true,
                rust: false,
                python: true,
                ..
            })
        ));
        assert!(matches!(
            parse(&[
                "capture",
                "--program",
                "oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv",
                "--from",
                "-2m",
                "-o",
                "ore.arcap"
            ])
            .unwrap(),
            DevCommands::Capture(ref args) if args.from == "-2m"
        ));
        assert!(parse(&["capture", "--program", "x", "-o", "a.arcap"]).is_err());
    }
}
