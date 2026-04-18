//! Basic Example: Using Arete Server
//!
//! This example demonstrates how to use the arete-server crate
//! with a generated spec from transform-macros.
//!
//! # Prerequisites
//!
//! 1. You need a Solana program with an IDL file
//! 2. Create a spec module using the `#[stream_spec]` macro
//! 3. Generate the spec using the macro system
//!
//! # Basic Usage
//!
//! The simplest way to start a Arete server:
//!
//! ```no_run
//! use arete_server::Server;
//! # use transform::compiler::MultiEntityBytecode;
//! # use arete_server::Spec;
//!
//! # fn mock_spec() -> Spec {
//! #     Spec::new(MultiEntityBytecode::new().build(), "your_program_id")
//! # }
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     // Setup logging
//!     tracing_subscriber::fmt()
//!         .with_env_filter("info")
//!         .init();
//!
//!     // Start server with default WebSocket configuration
//!     Server::builder()
//!         .spec(mock_spec())
//!         .websocket()
//!         .bind("[::]:8877".parse()?)
//!         .start()
//!         .await
//! }
//! ```
//!
//! # Advanced Usage with Custom Configuration
//!
//! For more control over the server configuration:
//!
//! ```no_run
//! use arete_server::{Server, WebSocketConfig, YellowstoneConfig, HealthConfig};
//! use std::net::SocketAddr;
//! use std::time::Duration;
//! # use transform::compiler::MultiEntityBytecode;
//! # use arete_server::Spec;
//!
//! # fn mock_spec() -> Spec {
//! #     Spec::new(MultiEntityBytecode::new().build(), "your_program_id")
//! # }
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     // Setup logging with custom filter
//!     tracing_subscriber::fmt()
//!         .with_env_filter(
//!             std::env::var("RUST_LOG")
//!                 .unwrap_or_else(|_| "info,arete_server=debug".into())
//!         )
//!         .init();
//!
//!     // Configure WebSocket server
//!     let ws_config = WebSocketConfig::new("[::]:8877".parse()?);
//!     
//!     // Configure Yellowstone gRPC connection (if needed)
//!     let yellowstone_config = YellowstoneConfig {
//!         endpoint: std::env::var("YELLOWSTONE_ENDPOINT")
//!             .unwrap_or_else(|_| "http://localhost:10000".into()),
//!         x_token: std::env::var("YELLOWSTONE_X_TOKEN").ok(),
//!     };
//!
//!     // Configure health monitoring (optional)
//!     let health_config = HealthConfig::new()
//!         .with_heartbeat_interval(Duration::from_secs(30))
//!         .with_health_check_timeout(Duration::from_secs(10));
//!
//!     // Build and start server with custom configuration
//!     Server::builder()
//!         .spec(mock_spec())
//!         .websocket_config(ws_config)
//!         .yellowstone(yellowstone_config)
//!         .health_config(health_config)  // Enable health monitoring
//!         .start()
//!         .await
//! }
//! ```
//!
//! # Health Monitoring
//!
//! Arete Server includes built-in health monitoring for stream connections:
//!
//! ```no_run
//! use arete_server::Server;
//! # use transform::compiler::MultiEntityBytecode;
//! # use arete_server::Spec;
//!
//! # fn mock_spec() -> Spec {
//! #     Spec::new(MultiEntityBytecode::new().build(), "your_program_id")
//! # }
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     // Enable health monitoring with defaults (30s heartbeat, 10s timeout)
//!     Server::builder()
//!         .spec(mock_spec())
//!         .websocket()
//!         .health_monitoring()  // Simple one-liner to enable
//!         .bind("[::]:8877".parse()?)
//!         .start()
//!         .await
//! }
//! ```
//!
//! Health monitoring tracks:
//! - Stream connection status (Connected, Disconnected, Reconnecting, Error)
//! - Event staleness detection (warns if no events received for 2x heartbeat interval)
//! - Error counting and logging
//! - Connection duration tracking
//!
//! # Creating a Spec
//!
//! The spec is generated automatically by the `transform-macros` crate.
//! Here's how to set it up:
//!
//! ```ignore
//! // In your lib.rs or a separate spec module
//! use transform_macros::stream_spec;
//!
//! #[stream_spec(idl = "path/to/your/program.json")]
//! pub mod my_program_spec {
//!     use transform_macros::StreamSection;
//!     use serde::{Deserialize, Serialize};
//!
//!     #[entity(name = "MyEntity")]
//!     pub struct MyEntity {
//!         pub id: EntityId,
//!         pub data: EntityData,
//!     }
//!
//!     #[derive(Debug, Clone, Serialize, Deserialize, StreamSection)]
//!     pub struct EntityId {
//!         #[map(generated_sdk::accounts::MyAccount::id, primary_key, strategy = SetOnce)]
//!         pub id: u64,
//!     }
//!
//!     #[derive(Debug, Clone, Serialize, Deserialize, StreamSection)]
//!     pub struct EntityData {
//!         #[map(generated_sdk::accounts::MyAccount::data, strategy = LastWrite)]
//!         pub value: String,
//!     }
//! }
//! ```
//!
//! Then use it in your main:
//!
//! ```ignore
//! use arete_server::Server;
//! use my_crate::my_program_spec;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     Server::builder()
//!         .spec(my_program_spec::spec())  // Generated by the macro!
//!         .websocket()
//!         .bind("[::]:8877".parse()?)
//!         .start()
//!         .await
//! }
//! ```
//!
//! # Environment Variables
//!
//! - `RUST_LOG`: Set log level (e.g., `info`, `debug`, `trace`)
//! - `YELLOWSTONE_ENDPOINT`: Yellowstone gRPC endpoint URL
//! - `YELLOWSTONE_X_TOKEN`: Authentication token for Yellowstone
//!
//! # Running the Example
//!
//! Note: This example cannot run standalone as it requires a real program spec.
//! See the flip-atom implementation for a complete working example.

fn main() {
    println!("This example is for documentation purposes only.");
    println!("See the source code for usage patterns.");
    println!("\nFor a working example, check out:");
    println!("  main/backend/tenant-runtime/flip-atom/");
}
