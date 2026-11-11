//! `arete-mcp` — MCP server wrapping Arete streams for AI agent integration.
//!
//! The server ships inside the `a4` CLI as `a4 mcp`, which calls
//! [`serve_stdio`]. There is no standalone binary; `a4 init` writes the MCP
//! config that launches it.

pub mod authority;
pub mod catalog_view;
mod connections;
pub mod credentials;
pub mod descriptor;
pub mod dev;
pub mod dev_capture;
pub mod dev_env;
pub mod entitlements;
pub mod extension_reads;
pub mod field_amounts;
mod filter;
pub mod oneshot;
pub mod prepared_summary;
pub mod program_read;
pub mod protocol;
pub mod protocol_agent;
pub mod protocol_api;
pub mod protocol_listing;
mod recovery;
mod registry;
pub mod sandbox;
pub mod sdk_reference;
pub mod server;
pub mod signer;
#[cfg(test)]
mod signer_vectors;
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
    // List only the account's entitlement groups. Without a key, or when the
    // API can't be reached or reports no groups, every tool is listed and
    // the API refuses what the account may not use.
    let api_url = std::env::var("ARETE_API_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| credentials::DEFAULT_API_URL.to_string());
    let surface = entitlements::fetch_surface(&api_url).await;
    let server = match sdk_references {
        Some(source) => AreteMcp::new().with_sdk_references(source),
        None => AreteMcp::new(),
    }
    .with_surface(&surface);
    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
