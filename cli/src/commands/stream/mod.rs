mod client;
mod filter;
mod output;
mod snapshot;
mod store;
#[cfg(feature = "tui")]
mod tui;

use anyhow::{bail, Context, Result};
use clap::Args;

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

    /// Filter expression: field=value, field>N, field~regex (repeatable, ANDed)
    #[arg(long = "where", value_name = "EXPR")]
    pub filters: Vec<String>,

    /// Select specific fields to output (comma-separated dot paths)
    #[arg(long)]
    pub select: Option<String>,

    /// Exit after first entity matches filter criteria
    #[arg(long)]
    pub first: bool,

    /// Filter by operation type (comma-separated: upsert,patch,delete)
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
    #[arg(long, conflicts_with = "url")]
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
    if let Some(load_path) = &args.load {
        if args.tui {
            bail!("--tui is not yet supported with --load. Use non-interactive replay for now.");
        }
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

    let rt = tokio::runtime::Runtime::new().context("Failed to create async runtime")?;

    if args.tui {
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

    eprintln!("Connecting to {} ...", url);
    eprintln!("Subscribing to {} ...", view);

    rt.block_on(client::stream(url, view, &args))
}

fn resolve_url(args: &StreamArgs, config_path: &str, view: &str) -> Result<String> {
    // 1. Explicit --url
    if let Some(url) = &args.url {
        return Ok(url.clone());
    }

    let config = HyperstackConfig::load_optional(config_path)?;

    // 2. Explicit --stack name
    if let Some(stack_name) = &args.stack {
        if let Some(config) = &config {
            if let Some(stack) = config.find_stack(stack_name) {
                if let Some(url) = &stack.url {
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
                return Ok(url.clone());
            }
        }
        // Only auto-select if there's exactly one stack with a URL (unambiguous)
        let stacks_with_urls: Vec<_> = config.stacks.iter().filter(|s| s.url.is_some()).collect();
        if stacks_with_urls.len() == 1 {
            let stack = stacks_with_urls[0];
            let name = stack.name.as_deref().unwrap_or(&stack.stack);
            eprintln!("Using stack '{}' (only stack with a URL)", name);
            return Ok(stack.url.clone().unwrap());
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
            .map(|s| {
                s.name
                    .as_deref()
                    .unwrap_or(&s.stack)
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join(", "),
        _ => "(none — create hyperstack.toml with [[stacks]] entries)".to_string(),
    }
}
