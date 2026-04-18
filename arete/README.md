# arete

[![crates.io](https://img.shields.io/crates/v/arete.svg)](https://crates.io/crates/arete)
[![docs.rs](https://docs.rs/arete/badge.svg)](https://docs.rs/arete)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

Real-time streaming data pipelines for Solana - transform on-chain events into typed state projections.

## Installation

```toml
[dependencies]
arete = "0.2"
```

Or with all features:

```toml
[dependencies]
arete = { version = "0.1", features = ["full"] }
```

## Features

| Feature | Default | Description |
|---------|---------|-------------|
| `interpreter` | ✅ | AST transformation runtime and VM |
| `macros` | ✅ | Proc-macros for defining streams |
| `server` | ✅ | WebSocket server and projection handlers |
| `sdk` | ❌ | Rust client for connecting to Arete servers |
| `full` | ❌ | Enables all features |

## Sub-crates

This is an umbrella crate that re-exports:

- [`arete-interpreter`](https://crates.io/crates/arete-interpreter) - AST transformation runtime
- [`arete-macros`](https://crates.io/crates/arete-macros) - Stream definition macros
- [`arete-server`](https://crates.io/crates/arete-server) - WebSocket server
- [`arete-sdk`](https://crates.io/crates/arete-sdk) - Rust client SDK

## Usage

```rust
use arete_macros::arete;

// Define a stream
#[arete(idl = "idl.json")]
pub mod my_stream {
    #[entity(name = "MyEntity")]
    #[derive(Stream)]
    struct MyEntity {
        #[map(from = Account::field, primary_key)]
        pub id: String,
    }
}
```

## License

Apache-2.0
