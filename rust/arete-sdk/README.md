# arete-sdk

[![crates.io](https://img.shields.io/crates/v/arete-sdk.svg)](https://crates.io/crates/arete-sdk)
[![docs.rs](https://docs.rs/arete-sdk/badge.svg)](https://docs.rs/arete-sdk)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

Rust client SDK for connecting to Arete streaming servers.

## Installation

```toml
[dependencies]
arete-sdk = "0.2"
```

### TLS Options

By default, the SDK uses `rustls` for TLS. You can switch to native TLS:

```toml
[dependencies]
arete-sdk = { version = "0.1", default-features = false, features = ["native-tls"] }
```

## Quick Start

```rust
use arete_sdk::prelude::*;
use my_stack::{PumpfunToken, PumpfunTokenEntity};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let a4 = Arete::connect("wss://mainnet.arete.xyz").await?;
    
    // List all entities
    let tokens = a4.list::<PumpfunTokenEntity>().await;
    println!("Found {} tokens", tokens.len());
    
    // Watch for real-time updates (lazy - no .await needed)
    let mut stream = a4.watch::<PumpfunTokenEntity>();
    while let Some(update) = stream.next().await {
        match update {
            Update::Upsert { key, data } => println!("Updated {}", key),
            Update::Patch { key, data } => println!("Patched {}", key),
            Update::Delete { key } => println!("Deleted {}", key),
        }
    }
    
    Ok(())
}
```

The `prelude` module re-exports all commonly needed types including `StreamExt`, so you don't need separate imports from `futures_util`.

## Lazy Streams with Chainable Operators

Streams are **lazy** - calling `watch()` returns immediately without subscribing. The subscription happens automatically on first poll. This enables ergonomic method chaining:

```rust
use std::collections::HashSet;

let watchlist: HashSet<String> = /* tokens to watch */;

let mut price_alerts = a4
    .watch_rich::<PumpfunTokenEntity>()
    .filter(move |u| watchlist.contains(u.key()))
    .filter_map(|update| match update {
        RichUpdate::Updated { before, after, .. } => {
            let prev = before.trading.last_trade_price.flatten().unwrap_or(0.0);
            let curr = after.trading.last_trade_price.flatten().unwrap_or(0.0);
            if prev > 0.0 {
                let pct = (curr - prev) / prev * 100.0;
                if pct.abs() > 0.1 {
                    return Some((after.info.name.clone(), pct));
                }
            }
            None
        }
        _ => None,
    });

while let Some((name, pct)) = price_alerts.next().await {
    println!("[PRICE] {:?} changed by {:.2}%", name, pct);
}
```

### Available Stream Operators

| Operator | Description |
|----------|-------------|
| `.filter(predicate)` | Keep only updates matching the predicate |
| `.filter_map(f)` | Filter and transform in one step |
| `.map(f)` | Transform each update |

All operators are chainable and return streams that support the same operators.

## Views API

The Views API provides a unified interface for accessing state, list, and derived views. This is the recommended way to access views as it provides consistent ergonomics across all view types.

```rust
use arete_sdk::prelude::*;
use my_stack::{OreRound, OreRoundViews};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let a4 = Arete::connect("wss://mainnet.arete.xyz").await?;
    
    // Get a views accessor
    let views = a4.views::<OreRoundViews>();
    
    // Access derived view (e.g., "latest" round)
    let latest = views.latest().get().await;
    println!("Latest round: {:?}", latest);
    
    // Access list view
    let all_rounds = views.list().get().await;
    println!("Found {} rounds", all_rounds.len());
    
    // Access state view by key
    let specific = views.state().get("round_key").await;
    
    // Watch derived view for updates
    let mut stream = views.latest().watch();
    while let Some(update) = stream.next().await {
        println!("Latest round updated: {:?}", update);
    }
    
    Ok(())
}
```

### View Types

| View Type | Access Pattern | Returns |
|-----------|---------------|---------|
| State | `views.state().get(key)` | `Option<T>` |
| List | `views.list().get()` | `Vec<T>` |
| Derived Single | `views.{name}().get()` | `Option<T>` |
| Derived Collection | `views.{name}().get()` | `Vec<T>` |

All view types support `.watch()` for streaming updates.

## API Reference

### Arete Client

```rust
// Simple connection
let a4 = Arete::connect("wss://example.com").await?;

// With configuration
let a4 = Arete::builder()
    .url("wss://example.com")
    .auto_reconnect(true)
    .max_reconnect_attempts(10)
    .ping_interval(Duration::from_secs(30))
    .initial_data_timeout(Duration::from_secs(5))
    .connect()
    .await?;
```

### Core Methods

| Method | Returns | Description |
|--------|---------|-------------|
| `get::<E>(key).await` | `Option<T>` | Get a single entity by key |
| `list::<E>().await` | `Vec<T>` | Get all entities of type E |
| `watch::<E>()` | `EntityStream<T>` | Stream all updates (lazy) |
| `watch_key::<E>(key)` | `EntityStream<T>` | Stream updates for a specific key (lazy) |
| `watch_keys::<E>(&[keys])` | `EntityStream<T>` | Stream updates for multiple keys (lazy) |
| `watch_rich::<E>()` | `RichEntityStream<T>` | Stream with before/after values (lazy) |
| `watch_key_rich::<E>(key)` | `RichEntityStream<T>` | Rich stream for specific key (lazy) |
| `connection_state().await` | `ConnectionState` | Get current connection state |
| `disconnect().await` | `()` | Close the connection |

### Update Types

When streaming with `watch()`, you receive `Update<T>` variants:

```rust
pub enum Update<T> {
    Upsert { key: String, data: T },  // Full entity update
    Patch { key: String, data: T },   // Partial update (merged)
    Delete { key: String },           // Entity removed
}
```

Helper methods: `key()`, `data()`, `is_delete()`, `has_data()`, `into_data()`, `into_key()`, `map(f)`

### Rich Updates (Before/After Diffs)

For tracking changes over time, use `watch_rich()`:

```rust
pub enum RichUpdate<T> {
    Created { key: String, data: T },
    Updated { key: String, before: T, after: T, patch: Option<Value> },
    Deleted { key: String, last_known: Option<T> },
}
```

The `Updated` variant includes `patch` - the raw JSON of changed fields, useful for checking what specifically changed:

```rust
if update.has_patch_field("trading") {
    // The trading field was modified
}
```

## Understanding `Option<Option<T>>` Fields

Generated entity types often have fields typed as `Option<Option<T>>`. This represents the **patch semantics** of Arete updates:

| Value | Meaning |
|-------|---------|
| `None` | Field was **not included** in this update (no change) |
| `Some(None)` | Field was **explicitly set to null** |
| `Some(Some(value))` | Field has a **concrete value** |

This distinction matters for partial updates (patches). When the server sends a patch, only changed fields are included. An absent field means "keep the previous value", while an explicit `null` means "clear this field".

### Working with `Option<Option<T>>`

```rust
// Access a nested optional field
let price = token.trading.last_trade_price.flatten().unwrap_or(0.0);

// Check if field was explicitly set (vs absent from patch)
match &token.reserves.current_price_sol {
    None => println!("Price not in this update"),
    Some(None) => println!("Price explicitly cleared"),
    Some(Some(price)) => println!("Price: {}", price),
}

// Compare values in before/after
if before.trading.last_trade_price != after.trading.last_trade_price {
    println!("Price changed!");
}
```

## Generating a Rust SDK

Use the Arete CLI to generate a typed Rust SDK from your spec:

```bash
# Generate SDK crate
a4 sdk create rust settlement-game

# With custom output directory
a4 sdk create rust settlement-game --output ./crates/game-sdk

# With custom crate name
a4 sdk create rust settlement-game --crate-name game-sdk
```

This generates a crate with:

```
generated/settlement-game-stack/
├── Cargo.toml
└── src/
    ├── lib.rs      # Re-exports
    ├── types.rs    # Data structs (with Option<Option<T>> for patchable fields)
    └── entity.rs   # Entity trait implementations
```

Add the generated crate to your `Cargo.toml`:

```toml
[dependencies]
arete-sdk = "0.2"
settlement-game-stack = { path = "./generated/settlement-game-stack" }
```

## Connection Management

### Auto-Reconnection

The SDK automatically reconnects on connection loss with configurable backoff:

```rust
let a4 = Arete::builder()
    .url("wss://example.com")
    .auto_reconnect(true)
    .reconnect_intervals(vec![
        Duration::from_secs(1),
        Duration::from_secs(2),
        Duration::from_secs(5),
        Duration::from_secs(10),
    ])
    .max_reconnect_attempts(20)
    .connect()
    .await?;
```

### Connection State

```rust
let state = a4.connection_state().await;
match state {
    ConnectionState::Connected => println!("Connected"),
    ConnectionState::Connecting => println!("Connecting..."),
    ConnectionState::Reconnecting { attempt } => println!("Reconnecting (attempt {})", attempt),
    ConnectionState::Disconnected => println!("Disconnected"),
    ConnectionState::Error => println!("Error"),
}
```

## Streaming Modes

| Mode | View | Description |
|------|------|-------------|
| State | `Entity/state` | Single shared state object |
| List | `Entity/list` | All entities, key-value lookups |
| Append | `Entity/append` | Append-only event log |

## License

MIT
