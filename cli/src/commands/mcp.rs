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
    // Tokio writes stdout from a blocking thread, and a write to a pipe the
    // client keeps open but never reads blocks it for good. The session has
    // given up on such a reply by now, but dropping the runtime would wait
    // for that thread forever: wait a moment for blocking work, then leave
    // whatever is still stuck to the end of the process.
    runtime.shutdown_timeout(SHUTDOWN_GRACE);
    served
}

/// How long the end of `a4 mcp` waits for blocking work still running,
/// such as a write to a stdout nobody reads.
const SHUTDOWN_GRACE: std::time::Duration = std::time::Duration::from_secs(1);
