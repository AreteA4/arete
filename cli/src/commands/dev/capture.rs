//! `a4 dev capture`: record a window of raw updates for some programs from
//! a Solana JSON-RPC node into an `arete.raw-capture/v1` file (`.arcap`),
//! with no Arete infrastructure involved. A capture is a portable record of
//! what a geyser subscription for the programs would have delivered; attach
//! it to a bug report or use it as a test fixture.

use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use arete_mcp::dev_capture::{capture_from_rpc, parse_window, HttpRpc, RpcCaptureOptions};
use clap::Args;
use colored::Colorize;
use serde_json::Value;

use super::{block_on, print, project_root, ProjectArgs};

/// The public mainnet RPC, the default capture source. It rate-limits
/// hard; `--rpc-url` takes your own.
pub const DEFAULT_CAPTURE_RPC: &str = "https://api.mainnet-beta.solana.com";

#[derive(Args, Debug, Clone)]
pub struct CaptureArgs {
    #[command(flatten)]
    pub project: ProjectArgs,
    /// A program to capture: its address, or the alias of a program (or the
    /// programs of a stack) in arete.lock; repeatable
    #[arg(long = "program", value_name = "ID|ALIAS", required = true)]
    pub programs: Vec<String>,
    /// Window start: a slot, an RFC 3339 time, or -10m/-2h relative to now
    #[arg(long, allow_hyphen_values = true)]
    pub from: String,
    /// Window end: a slot, a time, or +5m relative to --from (default: now)
    #[arg(long, allow_hyphen_values = true)]
    pub to: Option<String>,
    /// JSON-RPC node to read history from
    #[arg(long, env = "ARETE_CAPTURE_RPC_URL", default_value = DEFAULT_CAPTURE_RPC, value_name = "URL")]
    pub rpc_url: String,
    /// Also capture these accounts' current state as starting state
    /// (comma-separated or repeated)
    #[arg(long = "accounts", value_delimiter = ',', value_name = "PUBKEY")]
    pub accounts: Vec<String>,
    /// Don't capture the state of accounts the window's transactions touch
    #[arg(long)]
    pub no_touched_state: bool,
    /// Refuse windows with more transactions than this
    #[arg(long, default_value_t = 500)]
    pub max_transactions: usize,
    /// Pages of 1,000 signatures to read back through before giving up
    #[arg(long, default_value_t = 20)]
    pub max_signature_pages: usize,
    /// Starting-state accounts to fetch at most
    #[arg(long, default_value_t = 500)]
    pub max_state_accounts: usize,
    /// Don't read blocks to order transactions within a slot
    #[arg(long)]
    pub no_block_order: bool,
    /// Requests per second to the RPC node
    #[arg(long, default_value_t = 8.0)]
    pub rps: f64,
    /// Where to write the capture (`.arcap`)
    #[arg(short = 'o', long, value_name = "FILE")]
    pub output: PathBuf,
}

fn is_address(text: &str) -> bool {
    bs58::decode(text)
        .into_vec()
        .is_ok_and(|bytes| bytes.len() == 32)
}

/// Program addresses for `--program` values: addresses as they are,
/// aliases from arete.lock (a program's id, or a stack's programs).
fn resolve_programs(root: Option<&Path>, refs: &[String]) -> Result<Vec<String>> {
    let lock = root
        .map(|root| root.join("arete.lock"))
        .filter(|path| path.is_file())
        .map(crate::project::lockfile::ProjectLock::load)
        .transpose()?;
    let mut out: Vec<String> = Vec::new();
    for reference in refs {
        let ids: Vec<String> = if is_address(reference) {
            vec![reference.clone()]
        } else {
            let Some(dependency) = lock
                .as_ref()
                .and_then(|lock| lock.dependencies.iter().find(|d| d.alias == *reference))
            else {
                bail!("'{reference}' is not a program address or an alias in arete.lock");
            };
            let mut ids: Vec<String> = dependency.program_id.iter().cloned().collect();
            ids.extend(dependency.programs.iter().map(|p| p.program_id.clone()));
            if ids.is_empty() {
                bail!("'{reference}' in arete.lock names no program");
            }
            ids
        };
        for id in ids {
            if !out.contains(&id) {
                out.push(id);
            }
        }
    }
    Ok(out)
}

pub fn capture(args: CaptureArgs, json: bool) -> Result<()> {
    block_on(async move {
        let root = project_root(&args.project).ok();
        let programs = resolve_programs(root.as_deref(), &args.programs)?;
        if programs.is_empty() {
            bail!("name what to capture: --program <address|alias> (repeatable)");
        }
        let now = chrono::Utc::now().timestamp();
        let (from, to) = parse_window(&args.from, args.to.as_deref(), now)?;
        let mut options =
            RpcCaptureOptions::new(args.rpc_url.clone(), programs, from, to, &args.output);
        options.accounts = args.accounts.clone();
        options.touched_state = !args.no_touched_state;
        options.max_transactions = args.max_transactions;
        options.max_signature_pages = args.max_signature_pages;
        options.max_state_accounts = args.max_state_accounts;
        options.block_order = !args.no_block_order;
        options.requests_per_second = args.rps;
        options.tool_version = env!("CARGO_PKG_VERSION").to_string();
        let rpc = HttpRpc::new(args.rpc_url.clone(), args.rps)?;
        let mut progress = |line: &str| {
            if !json {
                eprintln!("{} {line}", "capture".dimmed());
            }
        };
        // The whole chain: the cause (a rate limit, a pruned transaction)
        // is what the reader needs.
        let summary = capture_from_rpc(&rpc, &options, &mut progress)
            .await
            .map_err(|error| anyhow::anyhow!("{error:#}"))?;
        let value = serde_json::to_value(&summary)?;
        print(json, &value, || print_written(&value));
        Ok(())
    })
}

fn print_written(result: &Value) {
    let header = &result["header"];
    println!(
        "Wrote {} ({} state accounts, {} updates: {} transactions, {} accounts; slots {}–{}; {})",
        result["path"].as_str().unwrap_or("?"),
        header["counts"]["state"],
        header["counts"]["updates"],
        header["counts"]["transactions"],
        header["counts"]["accounts"],
        header["slotRange"]["first"],
        header["slotRange"]["last"],
        header["contentHash"].as_str().unwrap_or("?"),
    );
    if let Some(note) = header["startingState"]["note"].as_str() {
        println!(
            "  starting state: {} ({note})",
            header["startingState"]["fidelity"].as_str().unwrap_or("?")
        );
    }
    for limit in header["limits"].as_array().into_iter().flatten() {
        println!("  limit: {}", limit.as_str().unwrap_or_default());
    }
    if let Some(skipped) = result["skippedFailed"].as_u64().filter(|n| *n > 0) {
        println!("  skipped {skipped} failed transactions (stacks never see them)");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn program_references_resolve_addresses_without_a_lock() {
        let id = "oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv".to_string();
        assert_eq!(
            resolve_programs(None, &[id.clone(), id.clone()]).unwrap(),
            vec![id]
        );
        assert!(resolve_programs(None, &["ore".into()]).is_err());
    }
}
