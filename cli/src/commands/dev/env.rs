//! The environment commands of `a4 dev`: list, status, wake, sleep, delete,
//! checkpoints, branch, restore, rebase, reset, the account and clock tools
//! (fund, pin, follow, warp, pause, resume), scenarios, previews, repro
//! exports, the lifecycle event feed and environment tokens.
//!
//! Each one acts on `--env <name or id>`, else `ARETE_DEV_ENV`, else the
//! project's environment (`.arete/dev/environment.json`, which `a4 dev up`
//! writes), through [`arete_mcp::dev_env`], the same client the MCP `dev_*`
//! tools use.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use arete_mcp::dev::{DevProject, DevWallet};
use arete_mcp::dev_env::{
    self, describe_env, public_view, resolve_env, EnvApi, EnvRpc, EnvSelection,
};
use clap::Args;
use colored::Colorize;
use serde_json::{json, Value};

use super::{block_on, print, ProjectArgs};

/// Which environment a command acts on.
#[derive(Args, Debug, Clone, Default)]
pub struct EnvArgs {
    #[command(flatten)]
    pub project: ProjectArgs,
    /// The environment, by name or id (default: ARETE_DEV_ENV, else the
    /// project's environment from `a4 dev up`)
    #[arg(long = "env", value_name = "NAME|ID")]
    pub env: Option<String>,
}

impl EnvArgs {
    pub async fn rpc(&self) -> Result<EnvRpc> {
        resolve_env(self.project.project.as_deref(), self.env.as_deref()).await
    }
}

/// An API error with what to do about it.
pub fn explain(error: anyhow::Error) -> anyhow::Error {
    match error
        .downcast_ref::<dev_env::ApiFailure>()
        .and_then(|failure| failure.hint())
    {
        Some(hint) => error.context(hint),
        None => error,
    }
}

/// Ask before something that cannot be undone: `-y/--yes` skips the
/// question; without a terminal it is required.
pub fn confirm(question: &str) -> Result<()> {
    if crate::ui::assume_yes() {
        return Ok(());
    }
    if !crate::ui::interactive() {
        bail!("{question} Pass --yes to confirm (this cannot be undone).");
    }
    let confirmed = dialoguer::Confirm::new()
        .with_prompt(question)
        .default(false)
        .interact()
        .context("Failed to read the answer")?;
    if !confirmed {
        bail!("Cancelled");
    }
    Ok(())
}

fn print_env(json: bool, env: &Value) {
    let view = public_view(env);
    print(json, &view, || println!("{}", describe_env(&view)));
}

/// A dev wallet name of the project, as its public key; a public key as is.
pub fn account_key(account: &str, project: &ProjectArgs) -> Result<String> {
    if dev_env::is_pubkey(account) {
        return Ok(account.to_string());
    }
    arete_mcp::dev::validate_wallet_name(account).with_context(|| {
        format!("'{account}' is neither a base58 public key nor a dev wallet name")
    })?;
    let project = DevProject::discover(project.project.as_deref())?;
    let (wallet, created) = DevWallet::load_or_create(&project.wallets_dir(), account)?;
    if created {
        eprintln!(
            "{} created dev wallet {account} ({})",
            "ok".green(),
            wallet.pubkey()
        );
    }
    Ok(wallet.pubkey())
}

// ---------------------------------------------------------------------------
// Lifecycle
// ---------------------------------------------------------------------------

/// `a4 dev list`.
pub fn list(json: bool) -> Result<()> {
    block_on(async move {
        let api = EnvApi::from_environment(reqwest::Client::new())?;
        let environments = api.list().await.map_err(explain)?;
        let quotas = api.quotas().await.ok();
        let current = DevProject::discover(None)
            .ok()
            .and_then(|project| project.load_selection().ok().flatten())
            .map(|selection| selection.id);
        let report = json!({
            "environments": environments.iter().map(public_view).collect::<Vec<_>>(),
            "quotas": quotas,
            "project": current,
        });
        print(json, &report, || {
            if environments.is_empty() {
                println!("No dev environments yet. `a4 dev up` in a project creates one.");
            }
            for env in &environments {
                let marker = if env["id"].as_str() == current.as_deref() {
                    "*"
                } else {
                    " "
                };
                println!("{marker} {}", describe_env(env));
            }
            if let Some(quotas) = &quotas {
                let limits = &quotas["limits"];
                let usage = &quotas["usage"];
                println!(
                    "{}",
                    format!(
                        "{} of {} environments, {} of {} storage bytes ({} tier)",
                        usage["environments"],
                        limits["maxEnvironments"],
                        usage["storageBytes"],
                        limits["maxStorageBytes"],
                        quotas["tier"].as_str().unwrap_or("?")
                    )
                    .dimmed()
                );
            }
        });
        Ok(())
    })
}

/// `a4 dev wake`.
pub fn wake(args: EnvArgs, json: bool) -> Result<()> {
    block_on(async move {
        let rpc = args.rpc().await?;
        let env = rpc.api().wake(rpc.env()).await.map_err(explain)?;
        print_env(json, &env);
        Ok(())
    })
}

/// `a4 dev sleep`.
pub fn sleep(args: EnvArgs, json: bool) -> Result<()> {
    block_on(async move {
        let rpc = args.rpc().await?;
        let env = rpc.api().sleep(rpc.env()).await.map_err(explain)?;
        print_env(json, &env);
        Ok(())
    })
}

/// `a4 dev delete`: the environment, its state and its checkpoints.
pub fn delete(args: EnvArgs, json: bool) -> Result<()> {
    block_on(async move {
        let rpc = args.rpc().await?;
        let env = rpc.api().get_env(rpc.env()).await.map_err(explain)?;
        let name = env["name"].as_str().unwrap_or(rpc.env()).to_string();
        confirm(&format!(
            "Delete dev environment {name} ({}) with its state and checkpoints?",
            rpc.env()
        ))?;
        rpc.api().delete(rpc.env()).await.map_err(explain)?;
        // Forget it in the project that pointed at it.
        if let Ok(project) = DevProject::discover(args.project.project.as_deref()) {
            if project
                .load_selection()
                .ok()
                .flatten()
                .is_some_and(|selection| selection.id == rpc.env())
            {
                let _ = std::fs::remove_file(project.environment_path());
            }
        }
        let report = json!({ "deleted": rpc.env(), "name": name });
        print(json, &report, || {
            println!("Deleted dev environment {name} ({})", rpc.env())
        });
        Ok(())
    })
}

/// `a4 dev use <name|id>`: point the project at another environment.
pub fn use_env(project: ProjectArgs, reference: String, json: bool) -> Result<()> {
    block_on(async move {
        let api = EnvApi::from_environment(reqwest::Client::new())?;
        let env = api
            .find(&reference)
            .await
            .map_err(explain)?
            .with_context(|| format!("You have no dev environment named '{reference}'"))?;
        let root = super::project_root(&project)?;
        let selection = EnvSelection::new(
            env["id"].as_str().unwrap_or_default(),
            env["name"].as_str().unwrap_or_default(),
            api.origin(),
        );
        selection.save(&DevProject::new(&root).environment_path())?;
        print(json, &serde_json::to_value(&selection)?, || {
            println!(
                "{} this project now develops against {}",
                "ok".green(),
                describe_env(&env)
            );
            println!(
                "{}",
                "A running `a4 dev up` keeps its environment until it restarts.".dimmed()
            );
        });
        Ok(())
    })
}

// ---------------------------------------------------------------------------
// Checkpoints
// ---------------------------------------------------------------------------

/// `a4 dev checkpoint [NAME]`, `--list`, `--delete <ID|NAME>`.
pub fn checkpoint(
    args: EnvArgs,
    name: Option<String>,
    list: bool,
    delete: Option<String>,
    json: bool,
) -> Result<()> {
    block_on(async move {
        let rpc = args.rpc().await?;
        let api = rpc.api();
        if list {
            let checkpoints = api.checkpoints(rpc.env()).await.map_err(explain)?;
            print(json, &json!({ "checkpoints": checkpoints }), || {
                if checkpoints.is_empty() {
                    println!("No checkpoints yet. `a4 dev checkpoint <name>` saves one.");
                }
                for checkpoint in &checkpoints {
                    println!(
                        "{} {} {} bytes, {}",
                        checkpoint["name"].as_str().unwrap_or("-").bold(),
                        checkpoint["id"].as_str().unwrap_or("?"),
                        checkpoint["bytes"],
                        checkpoint["createdAt"].as_str().unwrap_or("?")
                    );
                }
            });
            return Ok(());
        }
        if let Some(checkpoint) = delete {
            api.delete_checkpoint(rpc.env(), &checkpoint)
                .await
                .map_err(explain)?;
            print(json, &json!({ "deleted": checkpoint }), || {
                println!("Deleted checkpoint {checkpoint}")
            });
            return Ok(());
        }
        let created = api
            .checkpoint(rpc.env(), name.as_deref())
            .await
            .map_err(explain)?;
        print(json, &created, || {
            println!(
                "{} checkpoint {} ({}, {} bytes)",
                "ok".green(),
                created["name"].as_str().unwrap_or("-"),
                created["id"].as_str().unwrap_or("?"),
                created["bytes"]
            );
            println!(
                "{}",
                "Restore it with `a4 dev restore <name>`, or start a new environment from it with `a4 dev branch <name> <new-env>`.".dimmed()
            );
        });
        Ok(())
    })
}

/// `a4 dev branch <CHECKPOINT> <NAME>`.
pub fn branch(
    args: EnvArgs,
    checkpoint: String,
    name: String,
    switch: bool,
    json: bool,
) -> Result<()> {
    block_on(async move {
        let rpc = args.rpc().await?;
        let env = rpc
            .api()
            .branch(rpc.env(), &checkpoint, &name)
            .await
            .map_err(explain)?;
        if switch {
            let root = super::project_root(&args.project)?;
            EnvSelection::new(
                env["id"].as_str().unwrap_or_default(),
                env["name"].as_str().unwrap_or_default(),
                rpc.api().origin(),
            )
            .save(&DevProject::new(&root).environment_path())?;
        }
        print(json, &public_view(&env), || {
            println!("{} branched {}", "ok".green(), describe_env(&env));
            if switch {
                println!("This project now develops against it.");
            } else {
                println!(
                    "{}",
                    format!("`a4 dev use {name}` points this project at it.").dimmed()
                );
            }
        });
        Ok(())
    })
}

/// `a4 dev restore <CHECKPOINT>`.
pub fn restore(args: EnvArgs, checkpoint: String, json: bool) -> Result<()> {
    block_on(async move {
        let rpc = args.rpc().await?;
        let env = rpc
            .api()
            .restore(rpc.env(), &checkpoint)
            .await
            .map_err(explain)?;
        print(json, &public_view(&env), || {
            println!(
                "{} restored {checkpoint} into {}",
                "ok".green(),
                describe_env(&env)
            )
        });
        Ok(())
    })
}

/// `a4 dev rebase [--keep PUBKEY]`.
pub fn rebase(args: EnvArgs, keep: Vec<String>, json: bool) -> Result<()> {
    for account in &keep {
        if !dev_env::is_pubkey(account) {
            bail!("--keep {account} is not a base58 public key");
        }
    }
    block_on(async move {
        let rpc = args.rpc().await?;
        let result = rpc.api().rebase(rpc.env(), &keep).await.map_err(explain)?;
        print(json, &result, || {
            println!(
                "{} refreshed {} accounts from mainnet, kept {} (written or pinned); slot {}",
                "ok".green(),
                result["reset"],
                result["kept"],
                result["slot"]
            )
        });
        Ok(())
    })
}

/// `a4 dev reset`: a fresh fork; with `--account`, drop one account.
pub fn reset(
    args: EnvArgs,
    account: Option<String>,
    include_owned: bool,
    json: bool,
) -> Result<()> {
    block_on(async move {
        let rpc = args.rpc().await?;
        if let Some(account) = account {
            let result = rpc
                .reset_account(&account, include_owned)
                .await
                .map_err(explain)?;
            print(
                json,
                &json!({ "account": account, "result": result }),
                || {
                    println!(
                        "{} {account} will be read from mainnet again on its next use",
                        "ok".green()
                    )
                },
            );
            return Ok(());
        }
        let env = rpc.api().get_env(rpc.env()).await.map_err(explain)?;
        let name = env["name"].as_str().unwrap_or(rpc.env()).to_string();
        confirm(&format!(
            "Reset dev environment {name} to a fresh fork of mainnet (every account it wrote, funded or pinned is dropped; checkpoints stay)?"
        ))?;
        let result = rpc.api().reset(rpc.env()).await.map_err(explain)?;
        print(json, &result, || {
            println!(
                "{} {name} is a fresh fork again (slot {})",
                "ok".green(),
                result["slot"]
            )
        });
        Ok(())
    })
}

// ---------------------------------------------------------------------------
// Accounts and the clock
// ---------------------------------------------------------------------------

/// `a4 dev fund <ACCOUNT> [SOL]`, or a token balance with `--mint`.
pub fn fund(
    args: EnvArgs,
    account: String,
    sol: Option<String>,
    mint: Option<String>,
    amount: Option<String>,
    token_program: Option<String>,
    json: bool,
) -> Result<()> {
    let pubkey = account_key(&account, &args.project)?;
    block_on(async move {
        let rpc = args.rpc().await?;
        let report = match (mint, amount) {
            (Some(mint), Some(amount)) => {
                let result = rpc
                    .set_token_balance(&pubkey, &mint, &amount, token_program.as_deref())
                    .await
                    .map_err(explain)?;
                json!({ "account": pubkey, "mint": mint, "amount": amount, "result": result })
            }
            (Some(_), None) => bail!("--mint needs --amount <base units>"),
            (None, Some(_)) => bail!("--amount needs --mint <mint>"),
            (None, None) => {
                let lamports = dev_env::sol_to_lamports(sol.as_deref().unwrap_or("100"))?;
                let result = rpc.fund(&pubkey, lamports).await.map_err(explain)?;
                json!({ "account": pubkey, "lamports": lamports, "result": result })
            }
        };
        print(json, &report, || {
            if let Some(mint) = report["mint"].as_str() {
                println!(
                    "{} {account} holds {} base units of {mint}",
                    "ok".green(),
                    report["amount"].as_str().unwrap_or("?")
                );
            } else {
                println!(
                    "{} {account} ({pubkey}) has {} SOL",
                    "ok".green(),
                    report["lamports"].as_u64().unwrap_or_default() as f64 / 1e9
                );
            }
        });
        Ok(())
    })
}

/// `a4 dev pin <PUBKEY>` / `--unpin`.
pub fn pin(
    args: EnvArgs,
    account: String,
    include_owned: bool,
    unpin: bool,
    json: bool,
) -> Result<()> {
    block_on(async move {
        let rpc = args.rpc().await?;
        let result = rpc
            .pin(&account, include_owned, unpin)
            .await
            .map_err(explain)?;
        print(
            json,
            &json!({ "account": account, "result": result }),
            || {
                if unpin {
                    println!("{} unpinned {account}", "ok".green());
                } else {
                    println!(
                        "{} pinned {account}: never refreshed from mainnet, kept by rebase",
                        "ok".green()
                    );
                }
            },
        );
        Ok(())
    })
}

/// `a4 dev follow <PUBKEY>` / `--stop`.
pub fn follow(args: EnvArgs, account: String, stop: bool, json: bool) -> Result<()> {
    block_on(async move {
        let rpc = args.rpc().await?;
        let result = rpc.follow(&account, stop).await.map_err(explain)?;
        print(
            json,
            &json!({ "account": account, "result": result }),
            || {
                if stop {
                    println!("{} stopped following {account}", "ok".green());
                } else {
                    println!(
                        "{} following {account}: refetched from mainnet every block",
                        "ok".green()
                    );
                }
            },
        );
        Ok(())
    })
}

/// `a4 dev warp <TO>`.
pub fn warp(args: EnvArgs, to: String, json: bool) -> Result<()> {
    block_on(async move {
        let rpc = args.rpc().await?;
        let result = dev_env::warp_to(&rpc, &to).await.map_err(explain)?;
        print(json, &result, || {
            println!(
                "Clock: slot {} ({}) -> slot {} ({})",
                result["before"]["slot"],
                result["before"]["unixTimestamp"],
                result["after"]["slot"],
                result["after"]["unixTimestamp"]
            )
        });
        Ok(())
    })
}

/// `a4 dev pause` / `a4 dev resume`.
pub fn clock(args: EnvArgs, pause: bool, json: bool) -> Result<()> {
    block_on(async move {
        let rpc = args.rpc().await?;
        let result = if pause {
            rpc.pause().await
        } else {
            rpc.resume().await
        }
        .map_err(explain)?;
        print(json, &json!({ "paused": pause, "result": result }), || {
            println!(
                "{} the clock is {}",
                "ok".green(),
                if pause { "paused" } else { "running" }
            )
        });
        Ok(())
    })
}

// ---------------------------------------------------------------------------
// Scenarios, previews, repro exports
// ---------------------------------------------------------------------------

fn read_json_arg(value: &str, what: &str) -> Result<Value> {
    let text = match value.strip_prefix('@') {
        Some(path) => {
            std::fs::read_to_string(path).with_context(|| format!("Failed to read {path}"))?
        }
        None if Path::new(value).is_file() => {
            std::fs::read_to_string(value).with_context(|| format!("Failed to read {value}"))?
        }
        None => value.to_string(),
    };
    serde_json::from_str(&text).with_context(|| format!("The {what} is not JSON"))
}

/// `a4 dev scenario <FILE>`.
pub fn scenario(args: EnvArgs, file: String, dry_run: bool, json: bool) -> Result<()> {
    let document = read_json_arg(&file, "scenario")?;
    block_on(async move {
        let rpc = args.rpc().await?;
        let result = rpc.scenario(document, dry_run).await.map_err(explain)?;
        print(json, &result, || print_scenario(&result, dry_run));
        if !dry_run && result["passed"] == false {
            std::process::exit(1);
        }
        Ok(())
    })
}

fn print_scenario(result: &Value, dry_run: bool) {
    let name = result["scenario"].as_str().unwrap_or("scenario");
    for step in result["steps"].as_array().into_iter().flatten() {
        println!(
            "{} slot +{}{}",
            "step".bold(),
            step["slot"],
            step["label"]
                .as_str()
                .map(|label| format!(" {label}"))
                .unwrap_or_default()
        );
        for write in step["writes"].as_array().into_iter().flatten() {
            for field in write["fields"].as_array().into_iter().flatten() {
                println!(
                    "  {} {}.{}: {} -> {}",
                    write["account"].as_str().unwrap_or("?"),
                    write["accountType"].as_str().unwrap_or("?"),
                    field["path"].as_str().unwrap_or("?"),
                    field["before"],
                    field["after"]
                );
            }
        }
        for expectation in step["expectations"].as_array().into_iter().flatten() {
            let mark = if expectation["passed"] == true {
                "pass".green()
            } else {
                "fail".red()
            };
            println!(
                "  {mark} {} {} {} {} (actual {})",
                expectation["account"].as_str().unwrap_or("?"),
                expectation["field"].as_str().unwrap_or("?"),
                expectation["op"].as_str().unwrap_or("?"),
                expectation["value"],
                expectation["actual"]
            );
        }
    }
    if dry_run {
        println!("{name}: encoded, nothing written (--dry-run)");
    } else if result["passed"] == true {
        println!(
            "{} {name} passed (digest {})",
            "ok".green(),
            result["digest"].as_str().unwrap_or("?")
        );
    } else {
        println!("{} {name} failed", "fail".red());
    }
}

/// `a4 dev preview <TX>`: base64, or `@file` holding it.
pub fn preview(args: EnvArgs, transaction: String, json: bool) -> Result<()> {
    let transaction = match transaction.strip_prefix('@') {
        Some(path) => std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read {path}"))?
            .trim()
            .to_string(),
        None => transaction.trim().to_string(),
    };
    block_on(async move {
        let rpc = args.rpc().await?;
        let result = rpc.preview(&transaction).await.map_err(explain)?;
        print(json, &result, || {
            let error = &result["error"];
            println!(
                "{} {} compute units at slot {}",
                if error.is_null() {
                    "ok".green()
                } else {
                    "fails".red()
                },
                result["computeUnits"],
                result["slot"]
            );
            if !error.is_null() {
                println!("  error: {error}");
            }
            for instruction in result["instructions"].as_array().into_iter().flatten() {
                println!(
                    "  #{} {} {} CU",
                    instruction["index"],
                    instruction["programId"].as_str().unwrap_or("?"),
                    instruction["computeUnits"]
                );
            }
            for account in result["accounts"].as_array().into_iter().flatten() {
                println!(
                    "  {} {}",
                    account["pubkey"].as_str().unwrap_or("?"),
                    account["change"].as_str().unwrap_or("")
                );
            }
        });
        Ok(())
    })
}

/// `a4 dev repro <SIGNATURE>`.
pub fn repro(args: EnvArgs, signature: String, output: Option<PathBuf>, json: bool) -> Result<()> {
    block_on(async move {
        let rpc = args.rpc().await?;
        let result = rpc.repro(&signature).await.map_err(explain)?;
        let output = output.unwrap_or_else(|| {
            PathBuf::from(format!(
                "repro-{}.json",
                &signature[..signature.len().min(12)]
            ))
        });
        std::fs::write(&output, serde_json::to_vec_pretty(&result)?)
            .with_context(|| format!("Failed to write {}", output.display()))?;
        let summary = json!({
            "output": output.display().to_string(),
            "format": result["format"],
            "signature": result["signature"],
            "slot": result["slot"],
            "accounts": result["accounts"].as_object().map(|accounts| accounts.len()),
        });
        print(json, &summary, || {
            println!(
                "{} wrote {} ({} accounts before slot {})",
                "ok".green(),
                output.display(),
                summary["accounts"],
                summary["slot"]
            )
        });
        Ok(())
    })
}

// ---------------------------------------------------------------------------
// Events and tokens
// ---------------------------------------------------------------------------

fn print_event(event: &Value) {
    println!(
        "{} {} {}",
        event["createdAt"].as_str().unwrap_or("?").dimmed(),
        event["kind"].as_str().unwrap_or("?").bold(),
        event["detail"]
    );
}

/// `a4 dev events [--after N] [-f]`: the environment's lifecycle events.
pub fn events(
    args: EnvArgs,
    after: Option<i64>,
    limit: Option<u32>,
    follow: bool,
    json: bool,
) -> Result<()> {
    block_on(async move {
        let rpc = args.rpc().await?;
        let mut cursor = after;
        loop {
            let page = rpc
                .api()
                .events(rpc.env(), cursor, limit)
                .await
                .map_err(explain)?;
            let events = page["events"].as_array().cloned().unwrap_or_default();
            if json && !follow {
                println!("{page}");
                return Ok(());
            }
            for event in &events {
                if json {
                    println!("{event}");
                } else {
                    print_event(event);
                }
            }
            if let Some(next) = page["next"].as_i64() {
                cursor = Some(next);
            }
            if !follow {
                return Ok(());
            }
            if events.is_empty() {
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(3)) => {}
                    _ = tokio::signal::ctrl_c() => return Ok(()),
                }
            }
        }
    })
}

/// `a4 dev token`: a fresh environment token and the URLs it opens.
pub fn token(args: EnvArgs, json: bool) -> Result<()> {
    block_on(async move {
        let rpc = args.rpc().await?;
        let token = rpc.token(true).await.map_err(explain)?;
        let report = json!({
            "environment": rpc.env(),
            "token": token.token,
            "expiresAt": token.expires_at,
            "rpcUrl": token.rpc_url,
            "wsUrl": dev_env::ws_url_of(&token.rpc_url),
        });
        print(json, &report, || {
            println!("RPC   {}", token.rpc_url);
            println!("WS    {}", dev_env::ws_url_of(&token.rpc_url));
            println!("Token {}", token.token);
            println!(
                "{}",
                format!(
                    "Send it as `Authorization: Bearer <token>`; it expires at {} (Unix seconds). While `a4 dev up` runs, its local RPC URL needs no token.",
                    token.expires_at
                )
                .dimmed()
            );
        });
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use axum::http::{HeaderMap, StatusCode};
    use axum::response::IntoResponse;
    use axum::routing::post;
    use axum::{Json, Router};
    use serde_json::json;

    use super::*;

    #[derive(Default)]
    struct Counts {
        mints: AtomicUsize,
        calls: AtomicUsize,
    }

    /// The token route mints `token-<n>`; the environment answers its
    /// first call with 401 (an expired token), the next with 503 and
    /// `Retry-After: 1` (waking), then the result, for `token-2` only.
    #[tokio::test]
    async fn calls_renew_the_token_and_wait_out_a_wake() {
        let counts = Arc::new(Counts::default());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let rpc_url = format!("{origin}/rpc/v1/devenv/x");
        let app = Router::new()
            .route(
                "/api/dev/environments/:env/token",
                post({
                    let counts = counts.clone();
                    move || async move {
                        let n = counts.mints.fetch_add(1, Ordering::SeqCst) + 1;
                        Json(json!({ "token": format!("token-{n}"), "expiresAt": 4_000_000_000i64, "rpcUrl": rpc_url }))
                    }
                }),
            )
            .route(
                "/rpc/v1/devenv/x",
                post({
                    let counts = counts.clone();
                    move |headers: HeaderMap| async move {
                        let call = counts.calls.fetch_add(1, Ordering::SeqCst);
                        let bearer = headers["authorization"].to_str().unwrap().to_string();
                        match call {
                            0 => StatusCode::UNAUTHORIZED.into_response(),
                            1 => (StatusCode::SERVICE_UNAVAILABLE, [("retry-after", "1")], "waking")
                                .into_response(),
                            _ if bearer == "Bearer token-2" => {
                                Json(json!({ "jsonrpc": "2.0", "id": 1, "result": 42 }))
                                    .into_response()
                            }
                            _ => StatusCode::UNAUTHORIZED.into_response(),
                        }
                    }
                }),
            );
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        let api = EnvApi::new(reqwest::Client::new(), &origin, "a4_sk_x").unwrap();
        let rpc = EnvRpc::new(api, "denv_01hzx3k8m9q2r4s5t6v7w8x9yz").unwrap();
        assert_eq!(rpc.call("getSlot", json!([])).await.unwrap(), json!(42));
        assert_eq!(counts.mints.load(Ordering::SeqCst), 2);
        assert_eq!(counts.calls.load(Ordering::SeqCst), 3);

        // A JSON-RPC error is an error naming the method.
        let refused = rpc.fund("not-a-key", 1).await.unwrap_err().to_string();
        assert!(refused.contains("not a base58 public key"), "{refused}");
    }

    #[test]
    fn api_errors_carry_hints() {
        let error = explain(anyhow::Error::new(dev_env::ApiFailure {
            status: 403,
            code: Some("devenv-environment-quota".into()),
            message: "Your plan allows 1 environment".into(),
        }));
        let text = format!("{error:#}");
        assert!(text.contains("a4 dev up --env"), "{text}");
        assert!(text.contains("devenv-environment-quota"), "{text}");
    }
}
