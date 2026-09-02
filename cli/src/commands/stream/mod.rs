mod client;
mod filter;
mod output;
mod snapshot;
mod store;
mod token;
#[cfg(feature = "tui")]
mod tui;

use anyhow::{bail, Context, Result};
use arete_sdk::{Subscription, SubscriptionQuery};
use clap::Args;

use crate::api_client::ApiClient;

#[derive(Args)]
pub struct StreamArgs {
    /// View to subscribe to: EntityName/mode (e.g. OreRound/latest)
    pub view: Option<String>,

    /// Entity key to watch (for state-mode subscriptions)
    #[arg(short, long)]
    pub key: Option<String>,

    /// WebSocket URL override
    #[arg(long)]
    pub url: Option<String>,

    /// Stack name (resolves URL from arete.toml)
    #[arg(short, long)]
    pub stack: Option<String>,

    /// Output raw WebSocket frames instead of merged entities
    #[arg(long)]
    pub raw: bool,

    /// NO_DNA agent-friendly envelope format
    #[arg(long)]
    pub no_dna: bool,

    /// Filter expression: field=value, field>N, field~regex (repeatable, ANDed).
    /// Note: field? treats null as absent (returns false for null values)
    #[arg(long = "where", value_name = "EXPR")]
    pub filters: Vec<String>,

    /// Select specific fields to output (comma-separated dot paths). Nested paths are
    /// flattened to literal keys, e.g. --select "info.name" outputs {"info.name": "..."}
    #[arg(long)]
    pub select: Option<String>,

    /// Exit after first entity matches filter criteria
    #[arg(long)]
    pub first: bool,

    /// Filter by operation type (comma-separated: snapshot,upsert,patch,remove,delete).
    /// Snapshot entities are always tracked for state merging but only emitted
    /// when "snapshot" is in the allowed set.
    #[arg(long)]
    pub ops: Option<String>,

    /// Show running count of entities/updates only
    #[arg(long)]
    pub count: bool,

    /// Maximum size of the live query window
    #[arg(long)]
    pub take: Option<u32>,

    /// Skip N matching entities in the live query window
    #[arg(long)]
    pub skip: Option<u32>,

    /// Disable initial snapshot
    #[arg(long)]
    pub no_snapshot: bool,

    /// Resume from cursor (seq value)
    #[arg(long)]
    pub after: Option<String>,

    /// Record frames to a JSON snapshot file
    #[arg(long)]
    pub save: Option<String>,

    /// Auto-stop the stream after N seconds
    #[arg(long)]
    pub duration: Option<u64>,

    /// Replay a previously saved snapshot file instead of connecting live
    #[arg(
        long,
        conflicts_with = "url",
        conflicts_with = "tui",
        conflicts_with = "duration"
    )]
    pub load: Option<String>,

    /// Show update history for the specified --key entity
    #[arg(long)]
    pub history: bool,

    /// Show entity at a specific history index (0 = latest)
    #[arg(long)]
    pub at: Option<usize>,

    /// Show diff between consecutive updates
    #[arg(long)]
    pub diff: bool,

    /// Interactive TUI mode
    #[arg(long, short = 'i')]
    pub tui: bool,
}

pub fn run(args: StreamArgs, config_path: &str) -> Result<()> {
    // --load mode: replay from file, no WebSocket needed
    // (--load + --tui conflict is enforced by clap at the arg level)
    if let Some(load_path) = &args.load {
        let player = snapshot::SnapshotPlayer::load(load_path)?;
        let default_view = player.header.view.clone();
        let view = args.view.as_deref().unwrap_or(&default_view);
        let rt = tokio::runtime::Runtime::new().context("Failed to create async runtime")?;
        return rt.block_on(client::replay(player, view, &args));
    }

    let view = match args.view.as_deref() {
        Some(v) => v,
        None => bail!("<VIEW> argument is required (e.g. OreRound/latest)"),
    };

    let url = resolve_url(&args, config_path, view)?;
    let url = token::ensure_hosted_ws_token(url)?;

    let rt = tokio::runtime::Runtime::new().context("Failed to create async runtime")?;

    if args.tui {
        if args.duration.is_some() {
            bail!("--duration has no effect in TUI mode; stop with 'q' or Ctrl+C.");
        }
        if args.count {
            bail!("--count is incompatible with TUI mode.");
        }
        if args.save.is_some() {
            bail!("--save is not yet supported in TUI mode; use 's' inside the TUI to save.");
        }
        if args.history || args.at.is_some() || args.diff {
            bail!("--history/--at/--diff are not supported in TUI mode; use h/l keys to browse history.");
        }
        if args.raw {
            bail!("--raw is incompatible with TUI mode; omit --tui to use raw output.");
        }
        if args.no_dna {
            bail!("--no-dna is incompatible with TUI mode; omit --tui to use NO_DNA output.");
        }
        if !args.filters.is_empty() {
            bail!("--where is not supported in TUI mode; use '/' inside the TUI to filter.");
        }
        if args.select.is_some() {
            bail!("--select is not supported in TUI mode.");
        }
        if args.ops.is_some() {
            bail!("--ops is not supported in TUI mode.");
        }
        if args.first {
            bail!("--first is not supported in TUI mode.");
        }
        #[cfg(feature = "tui")]
        {
            return rt.block_on(tui::run_tui(url, view, &args));
        }
        #[cfg(not(feature = "tui"))]
        {
            bail!(
                "TUI mode requires the 'tui' feature.\n\
                 Install with: cargo install a4-cli --features tui"
            );
        }
    }

    eprintln!(
        "Connecting to {} ...",
        token::redact_hs_token_for_display(&url)
    );

    rt.block_on(client::stream(url, view, &args))
}

pub fn build_subscription(view: &str, args: &StreamArgs) -> Subscription {
    let mut query = SubscriptionQuery::new(view);
    if let Some(key) = &args.key {
        query = query.with_key(key);
    }
    if let Some(take) = args.take {
        query = query.with_take(take as usize);
    }
    if let Some(skip) = args.skip {
        query = query.with_skip(skip as usize);
    }
    if let Some(after) = &args.after {
        query = query.after(after);
    }
    Subscription::new(format!("a4-cli:{}", uuid::Uuid::new_v4().simple()), query)
        .with_snapshot(!args.no_snapshot)
}

fn validate_ws_url(url: &str) -> Result<()> {
    if !url.starts_with("ws://") && !url.starts_with("wss://") {
        bail!("Invalid URL scheme. Expected ws:// or wss://, got: {}", url);
    }
    Ok(())
}

fn resolve_url(args: &StreamArgs, _config_path: &str, _view: &str) -> Result<String> {
    // 1. Explicit --url
    if let Some(url) = &args.url {
        validate_ws_url(url)?;
        return Ok(url.clone());
    }

    // 2. Explicit hosted stack name
    if let Some(stack_name) = &args.stack {
        let install = ApiClient::new()?.get_registry_stack_install(stack_name, None)?;
        let url = install.websocket_url.ok_or_else(|| {
            anyhow::anyhow!(
                "Hosted stack '{stack_name}' has no single WebSocket endpoint; pass --url for the desired live binding"
            )
        })?;
        validate_ws_url(&url)?;
        return Ok(url);
    }

    bail!(
        "Could not determine WebSocket URL.\n\n\
         Specify one of:\n  \
         --url wss://your-stack.stack.arete.run\n  \
         --stack <registry-name>  (resolves the hosted endpoint)",
    )
}
