//! `arete-mcp` — MCP server wrapping Arete streams for AI agent integration.
//!
//! The server ships inside the `a4` CLI as `a4 mcp`, which calls
//! [`serve_stdio`]. There is no standalone binary; `a4 init` writes the MCP
//! config that launches it.

pub mod catalog_view;
mod connections;
pub mod credentials;
pub mod descriptor;
pub mod extension_reads;
pub mod field_amounts;
mod filter;
pub mod oneshot;
mod recovery;
mod registry;
pub mod sdk_reference;
pub mod server;
pub mod stack_knowledge;
mod subscriptions;

pub use server::AreteMcp;

use rmcp::{transport::stdio, ServiceExt};

/// Run the Arete MCP server over stdio until the client disconnects.
///
/// Installs a `tracing` subscriber that writes to **stderr** (stdout carries
/// MCP frames only), then serves [`AreteMcp`] on the process's stdin/stdout.
/// Must be called inside a Tokio runtime.
pub async fn serve_stdio() -> anyhow::Result<()> {
    serve_stdio_with(None).await
}

/// [`serve_stdio`], answering `describe_sdk` from `sdk_references`.
pub async fn serve_stdio_with(
    sdk_references: Option<std::sync::Arc<dyn sdk_reference::SdkReferenceSource>>,
) -> anyhow::Result<()> {
    // Logs go to stderr so they don't pollute the stdio MCP transport on stdout.
    let _ = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .try_init();

    tracing::info!("starting arete mcp stdio server");
    let server = match sdk_references {
        Some(source) => AreteMcp::new().with_sdk_references(source),
        None => AreteMcp::new(),
    };
    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
