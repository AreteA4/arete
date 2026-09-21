# TypeScript Program SDKs

Use the exact generated exports. The names below illustrate the stable layers, not a promise that a particular installed program defines them.

```ts
import { createSession } from '@usearete/sdk';
import { MY_STACK } from './generated/my-stack';

const session = await createSession(
  { stacks: { app: MY_STACK } },
  { wallet, auth: { publishableKey } },
);

const program = session.stacks.app.programs.myProgram;

const [vault] = program.pdas.vault.deriveSync({ owner });
const account = await program.accounts.Vault.fetch(vault);

const prepared = await program.transactions.deposit.prepare({
  owner,
  amount: { ui: '10.5' },
});

const inspection = await session.stacks.app.inspectOperation(prepared);
console.log(inspection.transaction.feeLamports, inspection.programError);

// Execute only when the request authorizes signing and submission.
const receipt = await session.execute(prepared);
if (receipt.transaction.slot != null) {
  await session.stacks.app.waitForProcessedSlot(receipt.transaction.slot);
}
```

If no semantic operation exists, use the exact raw shape:

```ts
const instruction = program.raw.deposit.build({
  amount: 10_500_000n,
  owner,
  vault,
});
```

Inspect the generated types for exact names, nested account overrides, `resolve` inputs, and whether PDA derivation returns a tuple or structured result.

TypeScript rules:

- Semantic inputs are normally camelCase; raw IDL inputs retain their generated raw names.
- Large integer instruction values use `bigint` unless a semantic operation accepts a UI amount object.
- `createSession` is preferred for shared wallets and multiple stacks/programs.
- `Arete.connect` remains suitable for one stack.
- Close sessions in scripts and tests.

Current references: `https://docs.arete.run/sdks/typescript/` and `https://docs.arete.run/using-stacks/transactions/`.
