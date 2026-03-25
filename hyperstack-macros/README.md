# hyperstack-macros

[![crates.io](https://img.shields.io/crates/v/hyperstack-macros.svg)](https://crates.io/crates/hyperstack-macros)
[![docs.rs](https://docs.rs/hyperstack-macros/badge.svg)](https://docs.rs/hyperstack-macros)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

Procedural macros for defining HyperStack streams.

## Overview

This crate provides the `#[hyperstack]` attribute macro that transforms annotated Rust structs into full streaming pipeline specifications, including:

- State struct generation with field accessors
- Handler creation functions for event processing
- IDL/Proto parser integration for Solana programs
- Automatic AST serialization for deployment

## Installation

```toml
[dependencies]
hyperstack-macros = "0.2"
```

## Usage

### IDL-based Stream

```rust
use hyperstack_macros::{hyperstack, Stream};

#[hyperstack(idl = "idl.json")]
pub mod my_stream {
    #[entity(name = "MyEntity")]
    #[derive(Stream)]
    struct Entity {
        #[map(from = "MyAccount", field = "value")]
        pub value: u64,
        
        #[map(from = "MyAccount", field = "owner")]
        pub owner: String,
    }
}
```

### Proto-based Stream

```rust
#[hyperstack(proto = ["events.proto"])]
pub mod my_stream {
    // entity structs
}
```

## Supported Attributes

| Attribute | Description |
|-----------|-------------|
| `#[map(...)]` | Map from account fields |
| `#[from_instruction(...)]` | Map from instruction fields |
| `#[event(...)]` | Capture instruction events |
| `#[snapshot(...)]` | Capture entire source data |
| `#[aggregate(...)]` | Aggregate field values |
| `#[computed(...)]` | Computed fields from other fields |
| `#[derive_from(...)]` | Derive values from instructions |

## Generated Output

The macro generates:

- `{EntityName}State` struct with all fields
- `fields::` module with field accessors
- `create_spec()` function returning `TypedStreamSpec`
- Handler creation functions for each source

## Diagnostics

The macro now validates most authoring mistakes before code generation. Common failures include:

- unknown account, instruction, or field references in `#[map]`, `#[event]`, and `#[derive_from]`
- invalid resolver inputs, unsupported resolver-backed field types, and malformed URL templates
- invalid view `sort_by` fields and computed-field dependency cycles
- invalid `pdas!` programs, seed accounts, and seed argument types

Most diagnostics include either a `Did you mean: ...?` suggestion or a short list of available values.

## Troubleshooting

- `unknown ... on entity ...`: check the field path against the generated state shape; nested fields must use `section.field`
- `unknown ... in instructions/accounts/...`: the IDL lookup failed; verify the SDK path or instruction/account spelling
- `invalid strategy ...`: use one of the listed strategy values exactly as shown in the error
- `unknown resolver ...` or `unknown resolver-backed type ...`: use a supported resolver name or change the target field type to a supported resolver-backed type
- `computed fields contain a dependency cycle ...`: break the cycle by making one field depend only on stored state, not another computed field in the loop

## Testing

Useful commands while working on macro diagnostics:

```bash
cargo test -p hyperstack-macros
cargo test -p hyperstack-idl
cargo check --manifest-path stacks/ore/Cargo.toml
```

The macro crate includes both `trybuild` UI tests under `hyperstack-macros/tests/ui/` and higher-level dynamic compile-failure tests under `hyperstack-macros/tests/phase*_dynamic.rs`.

## License

Apache-2.0
