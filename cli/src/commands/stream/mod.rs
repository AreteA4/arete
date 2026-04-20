mod client;
mod filter;
mod output;
mod snapshot;
mod store;
mod token;
#[cfg(feature = "tui")]
mod tui;

use anyhow::{bail, Context, Result};
use clap::Args;
use hyperstack_sdk::Subscription;

use crate::config::HyperstackConfig;

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

    /// Stack name (resolves URL from hyperstack.toml)
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

    /// Filter by operation type (comma-separated: snapshot,upsert,patch,delete).
    /// "upsert" also matches "create". Snapshot entities are always tracked for
    /// state merging but only emitted when "snapshot" is in the allowed set
    #[arg(long)]
    pub ops: Option<String>,

    /// Show running count of entities/updates only
    #[arg(long)]
    pub count: bool,

    /// Max entities in snapshot
    #[arg(long)]
    pub take: Option<u32>,

    /// Skip N entities in snapshot
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
                 Install with: cargo install hyperstack-cli --features tui"
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
    let mut sub = Subscription::new(view);
    if let Some(key) = &args.key {
        sub = sub.with_key(key.clone());
    }
    if let Some(take) = args.take {
        sub = sub.with_take(take);
    }
    if let Some(skip) = args.skip {
        sub = sub.with_skip(skip);
    }
    if args.no_snapshot {
        sub = sub.with_snapshot(false);
    }
    if let Some(after) = &args.after {
        sub = sub.after(after.clone());
    }
    sub
}

fn validate_ws_url(url: &str) -> Result<()> {
    if !url.starts_with("ws://") && !url.starts_with("wss://") {
        bail!("Invalid URL scheme. Expected ws:// or wss://, got: {}", url);
    }
    Ok(())
}

fn resolve_url(args: &StreamArgs, config_path: &str, view: &str) -> Result<String> {
    // 1. Explicit --url
    if let Some(url) = &args.url {
        validate_ws_url(url)?;
        return Ok(url.clone());
    }

    let config = HyperstackConfig::load_optional(config_path)?;

    // 2. Explicit --stack name
    if let Some(stack_name) = &args.stack {
        if let Some(config) = &config {
            if let Some(stack) = config.find_stack(stack_name) {
                if let Some(url) = &stack.url {
                    validate_ws_url(url)?;
                    return Ok(url.clone());
                }
                bail!(
                    "Stack '{}' found in config but has no url set.\n\
                     Set it in hyperstack.toml or use --url to specify the WebSocket URL.",
                    stack_name
                );
            }
        }
        bail!(
            "Stack '{}' not found in {}.\n\
             Available stacks: {}",
            stack_name,
            config_path,
            list_stacks(config.as_ref()),
        );
    }

    // 3. Auto-match entity name from view
    let entity_name = view.split('/').next().unwrap_or(view);
    if let Some(config) = &config {
        if let Some(stack) = config.find_stack(entity_name) {
            if let Some(url) = &stack.url {
                validate_ws_url(url)?;
                return Ok(url.clone());
            }
        }
        // Only auto-select if there's exactly one stack with a URL (unambiguous)
        let stacks_with_urls: Vec<_> = config.stacks.iter().filter(|s| s.url.is_some()).collect();
        if stacks_with_urls.len() == 1 {
            let stack = stacks_with_urls[0];
            let name = stack.name.as_deref().unwrap_or(&stack.stack);
            let url = stack.url.clone().unwrap();
            validate_ws_url(&url)?;
            eprintln!("Using stack '{}' (only stack with a URL)", name);
            return Ok(url);
        }
    }

    bail!(
        "Could not determine WebSocket URL.\n\n\
         Specify one of:\n  \
         --url wss://your-stack.stack.usehyperstack.com\n  \
         --stack <name>  (resolves from hyperstack.toml)\n\n\
         Available stacks: {}",
        list_stacks(config.as_ref()),
    )
}

fn list_stacks(config: Option<&HyperstackConfig>) -> String {
    match config {
        Some(config) if !config.stacks.is_empty() => config
            .stacks
            .iter()
            .map(|s| s.name.as_deref().unwrap_or(&s.stack).to_string())
            .collect::<Vec<_>>()
            .join(", "),
        _ => "(none — create hyperstack.toml with [[stacks]] entries)".to_string(),
    }
}
