//! `arete-mcp` — MCP server wrapping Arete streams for AI agent integration.
//!
//! The server ships inside the `a4` CLI as `a4 mcp`, which calls
//! [`serve_stdio`]. There is no standalone binary; `a4 init` writes the MCP
//! config that launches it.

mod connections;
mod credentials;
mod filter;
mod registry;
pub mod server;
mod subscriptions;

pub use server::AreteMcp;

use rmcp::{transport::stdio, ServiceExt};

/// Run the Arete MCP server over stdio until the client disconnects.
///
/// Installs a `tracing` subscriber that writes to **stderr** (stdout carries
/// MCP frames only), then serves [`AreteMcp`] on the process's stdin/stdout.
/// Must be called inside a Tokio runtime.
pub async fn serve_stdio() -> anyhow::Result<()> {
    // Logs go to stderr so they don't pollute the stdio MCP transport on stdout.
    let _ = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .try_init();

    tracing::info!("starting arete mcp stdio server");
    let service = AreteMcp::new().serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
