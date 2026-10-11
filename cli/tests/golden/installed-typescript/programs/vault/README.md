# `vault` TypeScript program SDK reference

Generated from program package `vault` 1.0.0, installed as `vault`. `a4 install` regenerates this folder, so do not edit it.
This file lists only what is specific to this SDK. For how to use any Arete SDK (`createSession`, wallets, preparing and executing operations), see the `arete-programs` agent skill or https://docs.arete.run/using-stacks/transactions/.
Look parts up with `a4 sdk describe vault [--view <Entity/view>] [--read <name>] [--program <key>] [--json]` or the MCP tool `describe_sdk`.

## Import

```ts
import { VAULT_PROGRAM } from './generated/typescript/programs/vault/vault.js'; // from the project root
```

Register it as `vault`: `createSession({ programs: { vault: VAULT_PROGRAM } })` gives `session.programs.vault`. Paths below start there.

Program `2c35Vf2AKSi7mTvaNdhSrgE3ppGAEyeSSLWNRkxbrQQM`.

## Operations

Each returns a prepared operation to inspect or execute.

- `instructions.treasury.deposit(input: DepositToTreasuryInput)`

## Accounts and instructions

- Accounts (`accounts.<Name>.fetch(address)`): `Vault`
- Raw instructions (`raw.<name>`): `deposit`, `withdraw`, `payFee`
- Helpers: `addresses.treasury`, `defaults.treasuryDeposit`

