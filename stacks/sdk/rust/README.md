# arete-stacks

[![crates.io](https://img.shields.io/crates/v/arete-stacks.svg)](https://crates.io/crates/arete-stacks)
[![docs.rs](https://docs.rs/arete-stacks/badge.svg)](https://docs.rs/arete-stacks)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

Protocol stacks for Arete - ready-to-use Solana data streams.

## Installation

```toml
[dependencies]
arete-stacks = "0.2"
```

Or with specific features:

```toml
[dependencies]
arete-stacks = { version = "0.2", default-features = false, features = ["pumpfun"] }
```

## Features

| Feature | Default | Description |
|---------|---------|-------------|
| `pumpfun` | Yes | PumpFun token streaming |
| `full` | No | Enables all stacks |

## Usage

```rust
use arete_sdk::prelude::*;
use arete_stacks::pumpfun::{PumpfunToken, PumpfunTokenEntity};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let a4 = Arete::connect("wss://mainnet.arete.xyz").await?;
    
    // List all tokens
    let tokens = a4.list::<PumpfunTokenEntity>().await;
    println!("Found {} tokens", tokens.len());
    
    // Watch for real-time updates
    let mut stream = a4.watch::<PumpfunTokenEntity>();
    while let Some(update) = stream.next().await {
        match update {
            Update::Upsert { key, data } => {
                println!("Token {}: {:?}", key, data.info.name);
            }
            Update::Delete { key } => {
                println!("Token {} removed", key);
            }
            _ => {}
        }
    }
    
    Ok(())
}
```

## Available Stacks

### PumpFun Token Stack

Real-time streaming data for PumpFun tokens on Solana.

```rust
use arete_stacks::pumpfun::{PumpfunToken, PumpfunTokenEntity};
```

**Entity: `PumpfunToken`**

| Field | Type | Description |
|-------|------|-------------|
| `id` | `PumpfunTokenId` | Token identifiers (mint, bonding curve) |
| `info` | `PumpfunTokenInfo` | Token metadata (name, symbol, URI) |
| `reserves` | `PumpfunTokenReserves` | Current reserve state and pricing |
| `trading` | `PumpfunTokenTrading` | Trading statistics and metrics |
| `events` | `PumpfunTokenEvents` | Recent buy/sell/create events |

## Dependencies

This crate depends on [`arete-sdk`](https://crates.io/crates/arete-sdk) for the core streaming functionality.

## License

MIT
