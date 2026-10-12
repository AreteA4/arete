# `vault` TypeScript SDK reference

Generated from stack package `vault` 1.0.0, installed as `vault`. `a4 install` regenerates this folder, so do not edit it.
This file lists only what is specific to this SDK. For how to use any Arete SDK (`createSession`, auth, `get`/`getOne`/`use`/`watch`, preparing and executing operations), see the `arete-streams` and `arete-programs` agent skills or https://docs.arete.run/sdks/typescript/.
Look parts up with `a4 sdk describe vault [--view <Entity/view>] [--read <name>] [--program <key>] [--json]` or the MCP tool `describe_sdk`.

## Contents

- Views: `Vault/state`, `Vault/list`
- Programs: `programs.vault`

To print one part with the row fields it uses, run `a4 sdk describe vault --read <name>` or `--view <Entity/view>`. The sections below list everything.

## Import

```ts
import { VAULT_STREAM_STACK } from './generated/typescript/stacks/vault/vault.js'; // from the project root
```

The path is relative to the importing file: from `src/`, import `'../generated/typescript/stacks/vault/vault.js'`. TypeScript resolves the `.js` to `vault.ts`.

Register it as `vault`: `createSession({ stacks: { vault: VAULT_STREAM_STACK } })` gives `session.stacks.vault`; in React, `useArete(VAULT_STREAM_STACK)`. Paths below start there. Row types (`Vault`) are exported from the same module (declared in `vault-core.ts`).

## Example

A script in `src/` that reads once, prints the result and exits (run it with `npx tsx`):

```ts
import { createSession } from '@usearete/sdk';
import { VAULT_STREAM_STACK } from '../generated/typescript/stacks/vault/vault.js';

const session = await createSession({ stacks: { vault: VAULT_STREAM_STACK } });
try {
  const result = await session.stacks.vault.views.Vault.list.getOne();
  console.log(result);
} finally {
  session.close();
}
```

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

