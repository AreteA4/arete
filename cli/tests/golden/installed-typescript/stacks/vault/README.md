# `vault` TypeScript SDK reference

Generated from stack package `vault` 1.0.0, installed as `vault`. `a4 install` regenerates this folder, so do not edit it.
This file lists only what is specific to this SDK. For how to use any Arete SDK (`createSession`, auth, `get`/`getOne`/`use`/`watch`, preparing and executing operations), see the `arete-streams` and `arete-programs` agent skills or https://docs.arete.run/sdks/typescript/.
Look parts up with `a4 sdk describe vault [--view <Entity/view>] [--read <name>] [--program <key>] [--json]` or the MCP tool `describe_sdk`.

## Import

```ts
import { VAULT_STREAM_STACK } from './generated/typescript/stacks/vault/vault.js'; // from the project root
```

Register it as `vault`: `createSession({ stacks: { vault: VAULT_STREAM_STACK } })` gives `session.stacks.vault`; in React, `useArete(VAULT_STREAM_STACK)`. Paths below start there. Row types (`Vault`) are exported from the same module (declared in `vault-core.ts`).

## Field names and types

- Rows use the paths below, which match the wire names in `a4 get`, `a4 stream` and the MCP `read_view` tool.
- `bigint` fields are 64-bit integers: compare them with `n` literals, and convert them (`Number()`, `String()`) before mixing them with numbers or passing them to `JSON.stringify`.
- A nullable field is null until the stack has seen its data: check it rather than defaulting it, so a wrong path does not pass silently.

## Entities and views

### Vault

Views: `views.Vault.state` (state, key `{ address: string }`), `views.Vault.list` (list).

Row `Vault`:

```text
balance.amount  bigint | null
id.address      string
```

Helpers on `session.stacks.vault`: `defaults.limits`.

## Programs

### `programs.vault` (program `2c35Vf2AKSi7mTvaNdhSrgE3ppGAEyeSSLWNRkxbrQQM`)

Reach it as `session.stacks.vault.programs.vault`. Descriptions: `programs/vault/README.md`, or `a4 sdk describe vault --program vault`.

#### Operations

Each returns a prepared operation to inspect or execute.

- `instructions.treasury.deposit(input: DepositToTreasuryInput)`

#### Accounts and instructions

- Accounts (`accounts.<Name>.fetch(address)`): `Vault`
- Raw instructions (`raw.<name>`): `deposit`, `withdraw`, `payFee`
- Helpers: `addresses.treasury`, `defaults.treasuryDeposit`

