# Builtin programs

`builtins(...)` returns typed modules for the runtime programs every protocol
may use without installing them. `ix` checks their instructions like an
installed program's: each builtin carries its program spec, and the server
checks the template against it.

| Builtin | Program | Registry package | Also used by |
|---|---|---|---|
| `system` | `11111111111111111111111111111111` | `system-program` | glue (account creation inside ATA creation) |
| `splToken` | `TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA` | `spl-token` | glue `wrapSol` / `unwrapSol` |
| `associatedToken` | `ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL` | `spl-ata` | glue `atas`, and `ata(owner, mint)` addresses |
| `computeBudget` | `ComputeBudget111111111111111111111111111111` | | compute limits the runtime adds |
| `memo` | `MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr` | | the attested memo on every step transaction |

Each builtin has one name. Other names are errors that name the builtin:
`spl-ata` and `splAta` are `associatedToken`, `spl-token` is `splToken`,
`system-program` is `system`, `spl-memo` is `memo` and `compute-budget` is
`computeBudget`.

```ts
const { associatedToken } = builtins("associatedToken");

ix(associatedToken, "createIdempotent", {
  accounts: { fundingAddress: "wallet" /* … */ },
});
```

## Signers

SPL Token's `owner` and `authority` accounts are signers: pass a key the step
signs with, such as `wallet` or an ephemeral key. Templates cannot use a
multisig token authority, because its members would sign as remaining
accounts, and remaining accounts never sign.

## Glue or a template

Glue is still the shortest way to do the common cases:

- `glue.atas: [{ owner, mint }]` creates each associated token account
  idempotently before the step.
- `glue.wrapSol: { amount }` wraps SOL into the fee payer's wSOL account, and
  `glue.unwrapSol: true` closes it after.
- `ata(owner, mint)` derives an associated token account address without any
  template.

Use `ix(<builtin>, ...)` for anything else, such as a token transfer or
closing an account in the step's own transaction.

Installing the registry package instead (`a4 install program spl-token`)
still works, and is what you want when an app also calls the program
directly through its program SDK.
