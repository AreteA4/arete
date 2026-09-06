# arete-a4-sdk

[![crates.io](https://img.shields.io/crates/v/arete-a4-sdk.svg)](https://crates.io/crates/arete-a4-sdk)
[![docs.rs](https://docs.rs/arete-a4-sdk/badge.svg)](https://docs.rs/arete-a4-sdk)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

Rust client SDK for Arete real-time Solana data stacks. The API mirrors the
TypeScript SDK (`@usearete/sdk`) — same views, program SDK, and stack binding —
expressed with native Rust idioms: builder chains instead of options objects,
`impl Stream` instead of async iterables, and typed generated structs instead of
phantom-typed object literals.

## Installation

```toml
[dependencies]
arete-sdk = { package = "arete-a4-sdk", version = "0.13.0" } # x-release-please-version
```

By default the SDK uses `rustls` for TLS. Switch to native TLS with
`default-features = false, features = ["native-tls"]`.

## Quick start

Generated stack code (from `a4 sdk create` / `a4 sdk sync`) provides a `Stack`
type binding views and programs:

```rust
use arete_sdk::prelude::*;
use ore_stack::{OreStreamStack, OreRound};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let a4 = Arete::<OreStreamStack>::builder()
        .api_key("hspk_your_key")
        .connect()
        .await?;

    // Fetch a snapshot.
    let rounds: Vec<OreRound> = a4.views.ore_round.latest().get().await;
    let latest = a4.views.ore_round.latest().get_one().await;

    // Stream merged entities (patches applied; removes/deletes filtered out).
    let mut stream = a4.views.ore_round.latest().listen().take(1);
    while let Some(round) = stream.next().await {
        println!("round: {round:?}");
    }

    Ok(())
}
```

## Views

Every entity exposes `state()` (keyed) plus `list()` and any derived views.
All subscriptions accept the same server-side query options as protocol v2:

```rust
// List/derived views
let mut open_orders = a4.views.order.list()
    .watch()                          // impl Stream<Item = Update<T>>
    .filter("state.status", "open")   // server-side exact-match filter
    .take(10)                         // query window (wire `take`, not StreamExt::take)
    .skip(20)
    .partition("solana-mainnet")
    .with_snapshot_limit(100);

while let Some(update) = open_orders.next().await {
    match update {
        Update::Upsert { key, data } => println!("upserted {key}: {data:?}"),
        Update::Patch { key, data } => println!("patched {key}: {data:?}"),
        Update::Remove { key } => println!("{key} left this query"),
        Update::Delete { key } => println!("{key} was deleted globally"),
    }
}

// Keyed state views take the same options
let mut miner = a4.views.ore_miner.state()
    .listen("miner-authority-address")
    .with_snapshot(true);

// Rich diffs with before/after values
let mut diffs = a4.views.ore_round.state().watch_rich("42");
```

`listen` yields `T`, `watch` yields `Update<T>`, `watch_rich` yields
`RichUpdate<T>`. Streams are lazy (the subscription is established on first
poll), reference-counted (equivalent queries share one wire subscription), and
unsubscribe when dropped. Chainable client-side operators `.filter(pred)`,
`.filter_map(f)`, and `.map(f)` are available on all entity streams.

## Program SDK

Generated program modules expose pure, typed instruction builders — the Rust
mirror of the TypeScript `client.programs.<name>.raw.<ix>.build(params)` layer.
No network access, no wallet: the output is a `BuiltInstruction` you hand to
your own signing/sending stack.

```rust
use ore_stack::programs::ore::{self, DeployParams};

// Through the connected client…
let ix = a4.programs.ore.deploy(DeployParams {
    signer: Some("signer-address".into()),
    round_id: 42,
    ..Default::default()
})?;

// …or standalone, no connection required.
let ix = ore::deploy(params)?;
let (miner_pda, _bump) = ore::pdas::miner("authority-address")?;

// ix: BuiltInstruction { program_id: Pubkey, accounts: Vec<BuiltAccountMeta>, data: Vec<u8> }
```

PDA accounts are derived automatically from the IDL seeds (literals, argument
references, account references), known addresses are filled in, and unknown
parameters fail closed. The lower-level schema-driven
`arete_sdk::InstructionHandler` is available for advanced composition.

Generated programs also expose release-addressed HTTP account readers:

```rust
let board = a4.programs.ore.board_accounts()?.fetch(&board_address).await?;
```

## Execution

Wire a wallet and send transactions or prepared operations:

```rust
let a4 = Arete::<MyStack>::builder()
    .api_key("hspk_…")
    .wallet(my_wallet)                    // Arc<dyn WalletAdapter>
    .connect()
    .await?;

let receipt = a4.transaction(&[ix], TransactionOptions::default()).await?;

let prepared = create_prepared_transaction(/* compose instructions/operations */)?;
let receipt = a4.execute(&prepared, ExecuteOptions::default()).await?;
```

Failures classify into the same four-state outcome model as TypeScript
(`confirmed | not-submitted | submitted-unknown | chain-failed`) with program
errors resolved against IDL metadata. A `SignerRegistry` covers multi-signer
flows with fail-closed validation before dispatch.

## Chain reads and transaction relay

```rust
let clock = a4.chain().clock().await?;
let balance = a4.chain().native_balance(&address, Default::default()).await?;
let blockhash = a4.transactions().latest_blockhash(Default::default()).await?;
```

`transport(Transport::Http)` skips the WebSocket entirely (point reads and
execution keep working; view subscriptions fail fast). Hosted deployments can
construct gateway transports from generated `sgb_` bindings via
`create_hosted_solana_gateway_transports`.

## Sessions

```rust
let session = Session::builder()
    .stack::<OreStack>("ore")
    .stack::<OtherStack>("other")
    .wallet(wallet)
    .connect()
    .await?;

let ore = session.stack::<OreStack>("ore")?;      // typed accessor
session.execute(&prepared, Default::default()).await?;
session.close().await;
```

## Authentication

```rust
let a4 = Arete::<MyStack>::builder()
    .publishable_key("hspk_…")                 // hosted stacks
    // .auth_token("jwt…")                     // pre-minted token
    // .token_endpoint("https://…/ws/sessions")
    // .get_token(|| async { … })              // custom provider
    .connect()
    .await?;
```

Hosted `*.stack.arete.run` endpoints mint session tokens automatically from a
publishable key; tokens refresh before expiry.

## Connection lifecycle

```rust
a4.connection_state().await;          // Disconnected | Connecting | Connected | Reconnecting | Error
let mut issues = a4.subscribe_socket_issues();
a4.disconnect().await;
```

Reconnects are automatic (configurable via the builder); subscriptions are
replayed with stable subscription IDs, and an authoritative snapshot atomically
replaces local state after reconnect.

## TypeScript equivalence

| TypeScript (`@usearete/sdk`) | Rust |
|---|---|
| `a4.views.OreRound.latest.use({ take: 1 })` | `a4.views.ore_round.latest().listen().take(1)` |
| `a4.views.OreRound.latest.watch(opts)` | `a4.views.ore_round.latest().watch()…` |
| `a4.views.OreRound.state.get({ roundId })` | `a4.views.ore_round.state().get("42").await` |
| `a4.programs.ore.raw.deploy.build(params)` | `a4.programs.ore.deploy(params)?` |
| `Update.type: 'upsert' \| 'patch' \| 'remove' \| 'delete'` | `Update::{Upsert, Patch, Remove, Delete}` |

See `docs/internal/sdk-api-surface.md` in the repository for the full mapping
and wire-format reference.

## License

MIT
