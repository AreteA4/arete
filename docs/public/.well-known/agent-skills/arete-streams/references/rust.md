# Rust View Clients

Use the generated Rust crate or module and the runtime crate version selected by generation. Do not copy an SDK version from this file.

The runtime package is `arete-a4-sdk`; its Rust module is `arete_sdk`.

```rust
use futures_util::StreamExt;
use arete_sdk::prelude::*;
use my_generated_stack::MyStack;

// No auth option: connect() uses ARETE_API_KEY if set, else your a4 CLI login.
let client = Arete::<MyStack>::builder()
    .connect()
    .await?;

let current = client
    .views
    .position
    .state()
    .get(&owner)
    .await;

let mut updates = client.views.position.list().listen();
if let Some(position) = updates.next().await {
    println!("{position:?}");
}

client.disconnect().await;
```

With no auth option, `connect()` uses `ARETE_API_KEY` if set, and otherwise the agent or secret key from the active `a4` login; if several `a4` profiles hold a key, set `ARETE_PROFILE`. To pass a key explicitly, use `.secret_key(std::env::var("ARETE_API_KEY")?)` with an agent key (`a4_ak_...`) or secret key (`a4_sk_...`). `publishable_key` is for origin-bound keys used by browser apps; a publishable key passed to `secret_key` is refused with `AreteError::InvalidConfig`.

Treat names such as `MyStack` and the accessor paths above as shape examples. Inspect the generated crate for the exact types and constructor signatures. Rust state accessors currently take the canonical encoded key string even when TypeScript and Python expose structured key inputs.

Rust uses:

- `.listen()` for the canonical merged-value `use` stream.
- `.watch()` for raw operations.
- `.watch_rich()` for before/after updates.
- builder methods for filters, keys, pagination, partitions, and cursors.
- `u64`/`u128` for large unsigned values.

Import the stream extension trait required by the installed runtime. Propagate or classify `AreteError` rather than converting transport and schema errors to empty data.

Current reference: `https://docs.arete.run/sdks/rust/`.
