mod client;
mod filter;
mod output;
mod snapshot;

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

    /// Auto-stop recording after N seconds (used with --save)
    #[arg(long)]
    pub duration: Option<u64>,

    /// Replay a previously saved snapshot file instead of connecting live
    #[arg(long, conflicts_with = "url")]
    pub load: Option<String>,
}

pub fn run(args: StreamArgs, config_path: &str) -> Result<()> {
    // --load mode: replay from file, no WebSocket needed
    if let Some(load_path) = &args.load {
        let player = snapshot::SnapshotPlayer::load(load_path)?;
        let default_view = player.header.view.clone();
        let view = args.view.as_deref().unwrap_or(&default_view);
        let rt = tokio::runtime::Runtime::new().context("Failed to create async runtime")?;
        return rt.block_on(client::replay(player, view, &args));
    }

    let view = args.view.as_deref().unwrap_or_else(|| {
        eprintln!("Error: <VIEW> argument is required (e.g. OreRound/latest)");
        std::process::exit(1);
    });

    let url = resolve_url(&args, config_path, view)?;

    eprintln!("Connecting to {} ...", url);
    eprintln!("Subscribing to {} ...", view);

    let rt = tokio::runtime::Runtime::new().context("Failed to create async runtime")?;
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
        // Try matching against all stacks
        for stack in &config.stacks {
            if let Some(url) = &stack.url {
                return Ok(url.clone());
            }
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
