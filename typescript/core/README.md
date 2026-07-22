# Arete TypeScript SDK

Pure TypeScript SDK for generated stack definitions, program SDKs, prepared operation execution, and chain reads.

Connected clients expose the typed six-route transaction relay as `client.transactions`. Wallet adapters receive it per invocation from `client.transaction` and `client.inspectOperation`, so a shared wallet can safely serve multiple clients. Sessions expose `session.transactions` and accept a `transactions` override. HTTP `u64` fields are decimal strings on the wire and `bigint` in the SDK; sends are never automatically retried.

## Installation

```bash
npm install @usearete/sdk
```

## Quick Start

```ts
import { createSession } from '@usearete/sdk';
import { MY_STACK } from './generated/my-stack';

const session = await createSession({
  stacks: { myStack: MY_STACK },
});

for await (const item of session.stacks.myStack.views.MyEntity.list.use()) {
  console.log(item);
}
```

## SDK Shapes

### Mental Model

Think of a generated stack as a **cartridge**: it packages one domain's typed views, queries, programs, reads, and flows, plus any cross-program addresses, constants, defaults, or math. "Cartridge" is only a mental model; the public API continues to use the existing generated stack objects and types.

An Arete session is the **console**. Insert one or more generated stacks under `session.stacks`, optionally add standalone programs, and use the console's shared `session.programs`, `session.chain`, and `session.execute(...)` surfaces. Programs packaged by a stack are automatically promoted to `session.programs` without creating another connection or runtime.

Every semantic operation has an explicit cardinality:

- `instruction` - exactly one Solana instruction
- `transaction` - exactly one atomic transaction with one or more instructions
- `flow` - one or more sequential transactions

Every semantic operation exposes one pure entrypoint:

- `.prepare(input)`

Every connected client or session exposes one semantic execution entrypoint:

- `.execute(prepared, options?)`

There is no semantic `.resolve()`, `.build()`, `.stage()`, `.plan()`, or `.send()` projection surface anymore.

### Connected Stack Cartridges

Access a connected generated stack through `session.stacks.<name>`. Its stable namespaces are:

- `views` - typed streaming, list, and state views
- `queries` - stack-level HTTP queries
- `programs` - owner-scoped program SDKs; the same connected objects are promoted to `session.programs`
- `addresses` - domain address derivations, when defined
- `constants` - domain constants and enums, when defined
- `defaults` - reusable domain defaults, when defined
- `math` - pure domain calculations, when defined
- `read` - connected, domain-oriented reads, when defined
- `flows` - stack-level multi-transaction operations, when defined

These are namespaces on the existing connected stack object; no separate cartridge class or type is introduced.

### Program SDKs

Each packaged, attached, or standalone program is canonically available under `session.programs.<name>` and exposes these stable namespaces. The owner-scoped `session.stacks.<name>.programs.<program>` path remains available when disambiguation is needed.

- `programId` - the deployed Solana program address
- `schemas` - generated validation schemas
- `pdas` - exact generated PDA definitions
- `addresses` - semantic address derivations, when defined
- `accounts` - typed point reads for program accounts
- `queries` - program-level HTTP queries
- `raw` - exact IDL instruction builders with synchronous `.build(...)`
- `instructions` - semantic one-instruction operations with `.prepare(...)`
- `transactions` - semantic one-transaction operations with `.prepare(...)`
- `flows` - program-local multi-transaction operations with `.prepare(...)`
- `constants` - program constants and enums
- `defaults` - reusable program defaults
- `math` - pure protocol calculations

### Raw Instructions

Use raw handlers when you want exact wire control and will compose the transaction yourself:

```ts
const ix = session.programs.splToken.raw.InitializeMint2.build({
  mint,
  decimals: 6,
  mint_authority: authority,
  freeze_authority: null,
});

await session.transaction([ix]);
```

Raw builders are the exact IDL escape hatch. Instruction names, account names, argument names, and nested objects retain the generated IDL shape, including snake_case where the IDL uses it.

### Semantic Operations

Use semantic operations for normal application code. Their `.prepare(...)` methods accept semantic, camelCase object inputs, derive routine addresses, normalize amounts, and return prepared artifacts. Use `raw` only when you deliberately need the exact IDL surface.

Instruction example:

```ts
const prepared = await session.programs.tokenMetadata.instructions.createMetadataAccountV3.prepare({
  mint,
  mintAuthority,
  payer,
  updateAuthority,
  name,
  symbol,
  uri,
});

console.log(prepared.kind); // 'instruction'
console.log(prepared.instruction);
console.log(prepared.artifacts);

await session.execute(prepared);
```

Transaction example:

```ts
const prepared = await session.programs.cpAmm.transactions.swap.exactIn.prepare({
  pool,
  payer,
  inputTokenMint,
  amountIn: { ui: '1.25' },
  minimumAmountOut: 1n,
});

console.log(prepared.kind); // 'transaction'
console.log(prepared.transaction.instructions.length);

await session.execute(prepared);
```

Flow example:

```ts
const prepared = await session.programs.presale.flows.escrow.depositPermissionless.prepare({
  presale,
  owner,
  maxAmount: { ui: '1000' },
});

console.log(prepared.kind); // 'flow'
console.log(prepared.plan.transactions.length);

await session.execute(prepared);
```

### Prepared Shapes

Prepared values are immutable and discriminated:

- `PreparedInstruction`
- `PreparedTransaction`
- `PreparedFlow`

All prepared values include:

- `kind`
- `name`
- `artifacts`
- `plan.transactions`

Instruction values also include `.instruction`, and transaction values include `.transaction`.

Prepared instructions can be composed directly into a transaction without
extracting `.instruction`:

```ts
const transaction = createPreparedTransaction({
  name: 'configureMint',
  instructions: [initializeMint, setAuthority],
  artifacts: { mint },
});
```

Prepared instructions and prepared transactions can also be flattened into one
atomic transaction without reaching into their transaction bodies:

```ts
const transaction = createPreparedTransaction({
  name: 'createAndConfigureMint',
  operations: [createMint, createMetadata, setAuthority],
  artifacts: { mint },
});
```

Only single-transaction operations are accepted by `operations`. A
`PreparedFlow` must retain its ordered transaction boundaries.

Child signer and error metadata is inherited unless the transaction explicitly
provides `requiredSignerAddresses` or `errors`.

Every successful execution receipt exposes its signatures in transaction order:

```ts
const receipt = await session.execute(prepared);
console.log(receipt.signatures);
```

Instruction and transaction receipts contain one signature. Flow receipts
contain one signature for each executed transaction.

Use `describePreparedOperation(prepared)` for a typed, JSON-safe description, or
`formatPreparedOperation(prepared)` for human-readable text.

```ts
const description = describePreparedOperation(prepared);
console.log(JSON.stringify(description, null, 2));
```

## Sessions

Use a session as the console for one or more generated stack cartridges and/or standalone programs behind one execution surface. Stack programs are promoted by reference; `session.programs.presale === session.stacks.presale.programs.presale`.

```ts
import { createSession, createSignerRegistry } from '@usearete/sdk';

const signerRegistry = createSignerRegistry([
  [creatorAddress, creatorSigner],
]);

const session = await createSession(
  {
    stacks: {
      squads: SQUADS_V4_STREAM_STACK,
      presale: METEORA_PRESALE_STREAM_STACK,
    },
    programs: {
      splToken: SPL_TOKEN_PROGRAM,
    },
  },
  {
    transport: 'http',
    endpoints: { http: 'http://127.0.0.1:8081' },
    wallet,
    signerRegistry,
  }
);

const prepared = await session.stacks.squads.flows.vaultProposal.prepare(...);
await session.execute(prepared);

// Signers can also be managed after session creation.
session.signerRegistry.register(memberAddress, memberSigner);
```

Equivalent entrypoints:

- `createSession(...)`
- `Arete.session(...)`

Session surface:

- `session.stacks.<name>` - connected generated stacks
- `session.programs.<name>` - connected packaged, attached, or standalone programs
- `session.chain` - canonical generic chain reads
- `session.signerRegistry`
- `session.transaction(...)`
- `session.execute(...)`

Prefer these connected paths in application code. `Arete.connect(STACK, ...)` remains available when a direct single-stack client is more convenient.

If multiple stacks package the same program key, the first stack in the session definition owns the promoted alias and the session emits a warning. Both owner-scoped stack paths remain available. An explicitly declared standalone program always owns its top-level key.

## Chain Reads

Generic chain reads use the console-level `session.chain` surface.

Available reads include:

- `exists(address)`
- `lamports(address)`
- `minimumBalanceForRentExemption(space)`
- `clock()`
- `account(address)`
- `mint(address)`
- `tokenAccount(address)`
- `balance({ owner, mint, tokenProgram? })`

Example:

```ts
const rentLamports = await session.chain.minimumBalanceForRentExemption(82);
const mintInfo = await session.chain.mint(mintAddress);
```

## Streaming Views

Views are still the main streaming surface.

```ts
for await (const update of session.stacks.myStack.views.settlementGame.list.watch()) {
  if (update.type === 'upsert') {
    console.log(update.key, update.data);
  }
}

const game = await session.stacks.myStack.views.settlementGame.state.get('game-123');
```

Every options object is a protocol v2 query with independent ordered membership. Different windows and filters on the same view can run concurrently, while equivalent normalized queries share one reference-counted wire subscription:

```ts
const rounds = session.stacks.ore.views.OreRound.latest;

const firstPage = rounds.watch({ take: 10 });
const secondPage = rounds.watch({ take: 10, skip: 10 });

for await (const update of firstPage) {
  if (update.type === 'remove') {
    console.log(`${update.key} left the first-page window`);
  }
}
```

Completed authoritative snapshots replace membership for their exact query after reconnect. Cursor queries created with `after` receive incremental snapshots and merge without pruning.

Low-level `QueryLease.refresh()` returns a `Promise<void>` that resolves after the refreshed subscription's next complete snapshot is committed. Registration, send, and subscription failures reject and are published on the lease's `QuerySnapshot.error`, with `isRefreshing` cleared. Subscriptions created with snapshots disabled resolve after the refresh request is sent.

### Connection recovery

`autoConnect` and `autoReconnect` default to `true` and control separate lifecycle phases. Set `autoConnect: false` to create a disconnected client that the application connects later. Set `autoReconnect: false` when the application wants to handle post-disconnect recovery itself:

```ts
const client = await Arete.connect(MY_STACK, {
  autoConnect: false,
  autoReconnect: false,
});
```

The same independent options are available per stack member in `createSession(...)` and on React's `AreteProvider`.

### Caller-supplied schema diagnostics

Core view schemas continue to filter rejected entities. React view hooks additionally accept `onSchemaValidationError`, which reports `{ view, key?, entity, error }` without changing accepted data.

## Safe amount parsing

`toRawAmount(input, decimals)` throws when user input is invalid. Form and API boundaries can use `safeToRawAmount(input, decimals)` instead:

```ts
import { safeToRawAmount } from '@usearete/sdk';

const result = safeToRawAmount({ ui: amountText }, 9);
if (!result.success) {
  console.error(result.error);
  return;
}

console.log(result.data); // bigint in raw base units
```

It returns `{ success: true, data } | { success: false, error }` and never throws for an invalid amount input.

## Update Types

```ts
type Update<T> =
  | { type: 'upsert'; key: string; data: T }
  | { type: 'patch'; key: string; data: Partial<T> }
  | { type: 'remove'; key: string }
  | { type: 'delete'; key: string };

type RichUpdate<T> =
  | { type: 'created'; key: string; data: T }
  | { type: 'updated'; key: string; before: T; after: T; patch?: unknown }
  | { type: 'removed'; key: string; lastKnown?: T }
  | { type: 'deleted'; key: string; lastKnown?: T };
```

`remove` means an entity left only this query's filter or window. `delete` means the source entity was deleted and is removed from every query for that view.

## License

MIT
