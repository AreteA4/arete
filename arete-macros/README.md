# arete-macros

[![crates.io](https://img.shields.io/crates/v/arete-macros.svg)](https://crates.io/crates/arete-macros)
[![docs.rs](https://docs.rs/arete-macros/badge.svg)](https://docs.rs/arete-macros)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

Procedural macros for defining Arete streams.

## Overview

This crate provides the `#[arete]` attribute macro that transforms annotated Rust structs into full streaming pipeline specifications, including:

- State struct generation with field accessors
- Handler creation functions for event processing
- IDL/Proto parser integration for Solana programs
- Automatic AST serialization for deployment

## Installation

```toml
[dependencies]
arete-macros = "0.2"
```

## Usage

### IDL-based Stream

```rust
use arete_macros::{arete, Stream};

#[arete(idl = "idl.json")]
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
#[arete(proto = ["events.proto"])]
pub mod my_stream {
    // entity structs
}
```

## Supported Attributes

| Attribute | Description |
|-----------|-------------|
| `#[map(...)]` | Map from account, instruction, or IDL event fields |
| `#[from_instruction(...)]` | Map from instruction fields |
| `#[event(...)]` | Capture structured payloads from instructions or IDL events |
| `#[snapshot(...)]` | Capture entire source data |
| `#[aggregate(...)]` | Aggregate field values |
| `#[computed(...)]` | Computed fields from other fields |
| `#[derive_from(...)]` | Derive values from instructions or IDL events |

## Event-backed authoring

IDL events are valid mapping sources anywhere you can reference a generated SDK path. Use `..._sdk::events::EventName` as the `from =` source and `..._sdk::events::EventName::field_name` for field paths.

Typical patterns:

- `#[map(my_program_sdk::events::TradeExecuted::amount, strategy = LastWrite)]`
- `#[map(my_program_sdk::events::TradeExecuted::__signature, primary_key, strategy = SetOnce)]`
- `#[aggregate(from = my_program_sdk::events::TradeExecuted, field = amount, strategy = Sum)]`
- `#[derive_from(from = my_program_sdk::events::TradeExecuted, field = amount, strategy = LastWrite)]`
- `#[event(from = my_program_sdk::events::TradeExecuted, fields = [id, amount])]`

When an IDL omits inline event fields, the macro resolves them from the event's backing type or a same-name struct in `types[]`. Event sources do not expose instruction accounts, so `accounts::...` is not valid on an event source.

The reserved fields `__signature`, `__slot`, and `__timestamp` come from the
runtime update context rather than the IDL payload. They can be mapped onto
ordinary entity fields, and a context-backed field can be used as an embedded
primary key. A transaction signature identifies the transaction, so use a
compound action identity when one transaction may emit multiple records of the
same entity type.

## Generated Output

The macro generates:

- `{EntityName}State` struct with all fields
- `fields::` module with field accessors
- `create_spec()` function returning `TypedStreamSpec`
- Handler creation functions for each source

## Diagnostics

The macro now validates most authoring mistakes before code generation. Common failures include:

- unknown account, instruction, event, or field references in `#[map]`, `#[event]`, and `#[derive_from]`
- invalid resolver inputs, unsupported resolver-backed field types, and malformed URL templates
- invalid view `sort_by` fields and computed-field dependency cycles
- invalid `pdas!` programs, seed accounts, and seed argument types

Most diagnostics include either a `Did you mean: ...?` suggestion or a short list of available values.

## Troubleshooting

- `unknown ... on entity ...`: check the field path against the generated state shape; nested fields must use `section.field`
- `unknown ... in instructions/accounts/events/...`: the IDL lookup failed; verify the SDK path or source spelling
- `invalid strategy ...`: use one of the listed strategy values exactly as shown in the error
- `unknown resolver ...` or `unknown resolver-backed type ...`: use a supported resolver name or change the target field type to a supported resolver-backed type
- `computed fields contain a dependency cycle ...`: break the cycle by making one field depend only on stored state, not another computed field in the loop

## Testing

Useful commands while working on macro diagnostics:

```bash
cargo test -p arete-macros
cargo test -p arete-idl
cargo check --manifest-path stacks/ore/Cargo.toml
```

The macro crate includes both `trybuild` UI tests under `arete-macros/tests/ui/` and higher-level dynamic compile-failure tests under `arete-macros/tests/phase*_dynamic.rs`.

## License

Apache-2.0
