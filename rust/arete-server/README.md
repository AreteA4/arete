# arete-server

[![crates.io](https://img.shields.io/crates/v/arete-server.svg)](https://crates.io/crates/arete-server)
[![docs.rs](https://docs.rs/arete-server/badge.svg)](https://docs.rs/arete-server)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

WebSocket server and projection handlers for Arete streaming pipelines.

## Overview

This crate provides a builder API for creating Arete servers that:

- Process Solana blockchain data via Yellowstone gRPC
- Parse and transform data using generated IDL parsers and the Arete VM
- Stream entity updates over WebSockets to connected clients
- Support multiple streaming modes (State, List, Append)
- Monitor stream health and connectivity status

## Installation

```toml
[dependencies]
arete-server = "0.2"
```

## Quick Start

```rust
use arete_server::{Server, Spec};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    Server::builder()
        .spec(my_spec())
        .websocket()
        .bind("[::]:8877".parse()?)
        .health_monitoring()
        .start()
        .await
}
```

### WebSocket Auth Plugins

`arete-server` now supports pluggable WebSocket auth. By default, all
connections are allowed.

```rust
use std::sync::Arc;

use arete_server::{Server, StaticTokenAuthPlugin};

Server::builder()
    .spec(my_spec())
    .websocket()
    .websocket_auth_plugin(Arc::new(StaticTokenAuthPlugin::new([
        "dev-secret".to_string(),
    ])))
    .start()
    .await?;
```

The built-in `StaticTokenAuthPlugin` accepts either:

- `Authorization: Bearer <token>`
- `?token=<token>` query param

### With Configuration

```rust
use arete_server::{Server, WebSocketConfig, HealthConfig};
use std::time::Duration;

Server::builder()
    .spec(my_spec())
    .websocket_config(WebSocketConfig {
        bind_addr: "[::]:8877".into(),
        max_clients: 1000,
        message_queue_size: 100,
    })
    .health_config(HealthConfig::new()
        .with_heartbeat_interval(Duration::from_secs(30))
        .with_health_check_timeout(Duration::from_secs(10)))
    .start()
    .await
```

## Architecture

```
┌─────────────────────┐
│  Yellowstone gRPC   │
└──────────┬──────────┘
           │
           ▼
┌─────────────────────┐
│  Vixen Runtime      │  ← Generated from IDL
└──────────┬──────────┘
           │
           ▼
┌──────────────────────┐
│  Arete VM       │  ← Processes bytecode
└──────────┬───────────┘
           │
           ▼
┌──────────────────────┐
│  Projector           │  ← Mutations → Frames
└──────────┬───────────┘
           │
           ▼
┌──────────────────────┐
│  BusManager          │  ← Pub/Sub routing
└──────────┬───────────┘
           │
           ▼
┌──────────────────────┐
│  WebSocket Server    │  ← Streams to clients
└──────────────────────┘
```

## Feature Flags

| Feature | Default | Description |
|---------|---------|-------------|
| `otel` | No | OpenTelemetry integration for metrics and distributed tracing |

## Health Monitoring

Built-in health monitoring tracks stream connectivity and detects issues:

- **Connection Status Tracking** - Monitors stream states (Connected, Disconnected, Reconnecting, Error)
- **Event Staleness Detection** - Warns when connected but not receiving events
- **Error Counting** - Tracks and logs error frequency for alerting
- **Connection Duration** - Records uptime for debugging stability issues

## Module Structure

```
arete-server/
├── src/
│   ├── lib.rs              # Server & ServerBuilder API
│   ├── bus.rs              # Event bus manager
│   ├── config.rs           # Configuration types
│   ├── runtime.rs          # Runtime orchestrator
│   ├── projector.rs        # Mutation → Frame transformation
│   ├── health.rs           # Health monitoring
│   ├── view/               # View registry & specs
│   └── websocket/          # WebSocket infrastructure
├── Cargo.toml
└── README.md
```

## Dependencies

- `tokio` - Async runtime
- `tokio-tungstenite` - WebSocket support
- `yellowstone-vixen` - Yellowstone gRPC integration
- `arete-interpreter` - Arete VM and bytecode

## License

Apache-2.0
