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
- Serve stack-scoped HTTP reads for `/chain/*` and `/programs/*`
- Optionally relay bounded transaction operations through fixed `/transactions/v1/*` routes
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

### Auth Plugins

`arete-server` supports pluggable auth for both WebSocket connections and HTTP
read endpoints. By default, all connections and HTTP reads are allowed.
When WebSocket auth is configured, HTTP reads inherit that plugin unless an
HTTP-specific plugin is configured.

```rust
use std::sync::Arc;

use arete_server::{Server, StaticTokenAuthPlugin};

Server::builder()
    .spec(my_spec())
    .websocket()
    .websocket_auth_plugin(Arc::new(StaticTokenAuthPlugin::new([
        "dev-secret".to_string(),
    ])))
    .http_auth_plugin(Arc::new(StaticTokenAuthPlugin::new([
        "dev-secret".to_string(),
    ])))
    .start()
    .await?;
```

The built-in `StaticTokenAuthPlugin` accepts either:

- `Authorization: Bearer <token>`
- `?token=<token>` query param

For signed sessions, the same short-lived JWT can be used for both WebSocket
and HTTP read requests. HTTP reads use bearer-only transport.

HTTP reads require the exact `read` scope. Transaction inspection routes
require `transaction:inspect`; submission requires `transaction:send`. These
whitespace-delimited scopes are independent and do not imply one another.

### Transaction Relay

The complete self-hosting and operations guide is available in
[`docs/src/content/docs/a4-server/transaction-relay.mdx`](../../docs/src/content/docs/a4-server/transaction-relay.mdx).

The transaction relay is disabled by default even if an RPC URL exists. For a
self-hosted server started with `.http()`, enable it with:

```bash
ARETE_TRANSACTIONS_ENABLED=true
ARETE_TRANSACTION_RPC_URL=https://your-solana-rpc.example
```

RPC URL precedence is `ARETE_TRANSACTION_RPC_URL`, `SOLANA_RPC_URL`, then
`RPC_URL`; transaction traffic never falls back to `ARETE_READ_RPC_URL`.
Relevant clamps include `ARETE_TRANSACTION_MAX_BODY_BYTES`,
`ARETE_TRANSACTION_MAX_BYTES`, `ARETE_TRANSACTION_INSPECT_TIMEOUT_MS`,
`ARETE_TRANSACTION_SEND_TIMEOUT_MS`, `ARETE_TRANSACTION_STATUS_TIMEOUT_MS`,
`ARETE_TRANSACTION_INSPECT_CONCURRENCY`, and
`ARETE_TRANSACTION_SEND_CONCURRENCY`. Per-process request clamps use
`ARETE_TRANSACTION_INSPECT_REQUESTS_PER_MINUTE`,
`ARETE_TRANSACTION_SEND_REQUESTS_PER_MINUTE`, and
`ARETE_TRANSACTION_STATUS_REQUESTS_PER_MINUTE`. `ARETE_TRUSTED_PROXY_CIDRS` is a
comma-separated CIDR list; forwarded client IPs are ignored unless the direct
peer is trusted.

Optional usage delivery is configured with
`ARETE_TRANSACTION_USAGE_ENABLED`, `ARETE_TRANSACTION_USAGE_ENDPOINT`,
`ARETE_TRANSACTION_USAGE_TOKEN`, and
`ARETE_TRANSACTION_USAGE_SPOOL_CAPACITY`. Delivery uses a bounded in-memory
queue and bounded retries, never blocks transaction responses, and emits only
metering identity, operation, result, byte count, and latency metadata.

The routes map statically to Solana RPC methods:

| Route | Required scope |
| --- | --- |
| `/transactions/v1/latest-blockhash` | `transaction:inspect` |
| `/transactions/v1/fee` | `transaction:inspect` |
| `/transactions/v1/simulate` | `transaction:inspect` |
| `/transactions/v1/send` | `transaction:send` |
| `/transactions/v1/signature-status` | `transaction:inspect` |
| `/transactions/v1/block-height` | `transaction:inspect` |

Clients compile and sign locally. The server accepts only bounded unsigned
inspection payloads and fully signed wire transactions. A send is dispatched
once with provider retries disabled and is never automatically replayed. For
hosted V1 deployments, keep transaction-enabled runtimes at one replica
because admission counters are process-local.

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
