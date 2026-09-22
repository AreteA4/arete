# Rust Program SDKs

Use the generated Rust crate/module and inspect its typed parameter structs. The runtime package is `arete-a4-sdk`, imported as `arete_sdk`.

```rust
use arete_sdk::prelude::*;
use my_generated_stack::programs::my_program::{self, DepositParams};

// Pure/offline generated builder.
let instruction = my_program::deposit(DepositParams {
    owner,
    amount: 10_500_000,
    ..Default::default()
})?;

// The connected generated program exposes release-addressed readers.
let vault = client.programs.my_program.vault_accounts()?.fetch(&vault_address).await?;
```

Generated PDA functions encode exact seeds:

```rust
let (vault_address, bump) = my_program::pdas::vault(&owner)?;
```

Treat names and signatures above as shape examples; use the generated module as the compiler-checked source of truth.

For execution, configure an `Arc<dyn WalletAdapter>` and use `client.transaction(&[instruction], options)` or `client.execute(&prepared, options)`. A `SignerRegistry` covers additional signers and fails closed when required signer addresses are unavailable.

Preserve `OperationExecutionError` and its structured outcome. Do not flatten `submitted-unknown` into a generic retryable error.

Current OSS reference: `https://github.com/AreteA4/arete/tree/main/rust/arete-a4-sdk`.
