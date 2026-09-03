//! `a4 mcp`: the Arete stream MCP server over stdio, folded into the `a4`
//! binary so no MCP config needs npm.
//!
//! Spec: `docs/internal/agent-first-onboarding.md` (WP6).

use anyhow::{Context, Result};
use clap::Args;

#[derive(Args, Debug, Clone)]
pub struct McpArgs {
    /// Serve over stdio (the only transport today; accepted for forward-compat)
    #[arg(long, hide = true)]
    pub stdio: bool,
}

/// Serve MCP over stdio. Never prints to stdout except MCP frames.
pub fn run(args: McpArgs) -> Result<()> {
    let _ = args;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("Failed to start the async runtime for a4 mcp")?;
    runtime.block_on(arete_mcp::serve_stdio())
}
