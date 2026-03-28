//! Hyperstack Reference Authentication Server
//!
//! This is a reference implementation of an authentication server for Hyperstack.
//! It provides:
//! - Token minting endpoint (POST /ws/sessions)
//! - JWKS endpoint (GET /.well-known/jwks.json)
//! - Health check (GET /health)
//! - Key management for secret and publishable keys

use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    Router,
    routing::{get, post},
};
use tower_http::cors::CorsLayer;
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;

mod config;
mod error;
mod handlers;
mod keys;
mod middleware;
mod models;
mod server;

use config::Config;
use server::AppState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;

    info!("Starting Hyperstack Auth Server...");

    // Load configuration
    let config = Config::from_env()?;
    info!("Configuration loaded successfully");

    // Bind address (before config is moved)
    let addr: SocketAddr = format!("{}:{}", config.host, config.port).parse()?;

    // Create application state
    let state = Arc::new(AppState::new(config).await?);
    info!("Application state initialized");

    // Build router
    let app = Router::new()
        .route("/ws/sessions", post(handlers::mint_token))
        .route("/.well-known/jwks.json", get(handlers::jwks))
        .route("/health", get(handlers::health))
        .layer(CorsLayer::permissive())
        .with_state(state);
    info!("Listening on {}", addr);

    // Start server
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
