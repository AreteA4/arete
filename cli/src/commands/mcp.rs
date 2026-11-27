//! `a4 mcp`: the Arete stream MCP server over stdio, folded into the `a4`
//! binary so no MCP config needs npm.

use anyhow::{Context, Result};
use clap::Args;

#[derive(Args, Debug, Clone)]
pub struct McpArgs {
    /// Serve over stdio (the only transport today; accepted for forward-compat)
    #[arg(long, hide = true)]
    pub stdio: bool,
}

/// How long the runtime may take to stop once the session is over. Replies
/// were already given their time to drain; what is left is a write to a
/// stdout the client does not read, or a read of a stdin it keeps open, each
/// blocked in a thread that dropping the runtime would wait on for good.
const SHUTDOWN_GRACE: std::time::Duration = std::time::Duration::from_secs(1);

/// Serve MCP over stdio. Never prints to stdout except MCP frames.
pub fn run(args: McpArgs) -> Result<()> {
    let _ = args;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("Failed to start the async runtime for a4 mcp")?;
    let served = runtime.block_on(arete_mcp::serve_stdio_with(Some(std::sync::Arc::new(
        crate::project::sdk_references::ProjectSdkReferences,
    ))));
    runtime.shutdown_timeout(SHUTDOWN_GRACE);
    served
}
