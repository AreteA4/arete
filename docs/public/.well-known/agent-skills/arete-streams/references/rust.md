# Rust View Clients

Use the generated Rust crate or module and the runtime crate version selected by generation. Do not copy an SDK version from this file.

The runtime package is `arete-a4-sdk`; its Rust module is `arete_sdk`.

```rust
use futures_util::StreamExt;
use arete_sdk::prelude::*;
use my_generated_stack::MyStack;

let client = Arete::<MyStack>::builder()
    .publishable_key(publishable_key)
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

Treat names such as `MyStack` and the accessor paths above as shape examples. Inspect the generated crate for the exact types and constructor signatures. Rust state accessors currently take the canonical encoded key string even when TypeScript and Python expose structured key inputs.

Rust uses:

- `.listen()` for the canonical merged-value `use` stream.
- `.watch()` for raw operations.
- `.watch_rich()` for before/after updates.
- builder methods for filters, keys, pagination, partitions, and cursors.
- `u64`/`u128` for large unsigned values.

Import the stream extension trait required by the installed runtime. Propagate or classify `AreteError` rather than converting transport and schema errors to empty data.

Current reference: `https://docs.arete.run/sdks/rust/`.
