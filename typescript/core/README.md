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

## Authentication

Pick the key by where the code runs:

```ts
// Servers, agents and local scripts (Node, Bun, Deno, workers, SSR):
// an agent key (a4_ak_...) or secret key (a4_sk_...).
const server = await createSession(
  { stacks: { myStack: MY_STACK } },
  { auth: { secretKey: process.env.ARETE_API_KEY } },
);

// Anything shipped to a browser: an origin-bound publishable key (a4_pk_...).
// Create one with `a4 auth keys create-publishable --origin <scheme://host[:port]>`.
const browser = await createSession(
  { stacks: { myStack: MY_STACK } },
  { auth: { publishableKey: import.meta.env.VITE_ARETE_PUBLISHABLE_KEY } },
);
```

Outside a browser, when no `auth` option is set, the SDK uses `ARETE_API_KEY`
if it is set, and otherwise the agent or secret key from your `a4` login
(`a4 auth login`, `a4 auth signup` or `a4 init`). So `createSession({ stacks })`
is enough in a script on a machine where you are logged in with `a4`. The login
is read in Node (20.16+ / 22.3+), Bun and Deno (only with read permission
already granted), never in a browser, and browser bundles contain no Node
imports for it. If several `a4` profiles hold a key, set `ARETE_PROFILE` to pick
one. `secretKey` throws in a browser, as does a secret or agent key passed as
`publishableKey`; a publishable key passed as `secretKey` is refused everywhere.
Keys are never included in error messages or logs.

### Arete API credentials (`tokenKind: 'api'`)

By default `token`, `getToken` and `tokenEndpoint` hand out Arete session
tokens. An app whose users are signed in to Arete can hand out the user's own
Arete API credential instead:

```ts
const session = await createSession(
  { stacks: { myStack: MY_STACK } },
  {
    auth: {
      tokenKind: 'api',
      tokenEndpoint: '/api/arete-credential', // returns the user's credential
      tokenEndpointCredentials: 'include',
    },
  },
);
```

The SDK sends that credential only to Arete API routes and to session
endpoints. For the Solana gateway, Program Read, protocol and hosted stack
transports it mints binding-scoped sessions at each binding's session endpoint,
caches them per target and scope, and sends only those to the transport. When
a session endpoint refuses the credential with a 401, it drops it, fetches the
credential again once and retries; other requests wait for the new one, and a
late 401 for a credential already replaced does not fetch again. With no
`token`, `getToken` or `tokenEndpoint` the credential falls back to
`ARETE_API_KEY`, then the `a4` login. `getToken` is called as `getToken({ scopes: [],
forceRefresh })`; `tokenEndpoint` is POSTed `{"scopes": [], "forceRefresh":
false}` and answers with the credential as a string or `{ token, expiresAt }`.

A session (or an `Arete.connect` client) shares one credential source across
every member, transport and its Arete API client, so the app is asked once per
credential expiry. A member with its own `auth` gets its own.

Every `getToken` request carries `forceRefresh`, `true` when the SDK needs a
new token (the last was refused, expired or did not cover the scopes). A
provider with its own cache must bypass that cache when it is set. With
`tokenKind: 'api'`, `tokenEndpoint` bodies carry it too.

### Where account credentials go

An account credential (a secret or agent key from `auth.secretKey`,
`ARETE_API_KEY` or the `a4` login, or any `tokenKind: 'api'` token) is sent
only to trusted Arete API origins, however it was supplied:
`https://api.arete.run`, the `ARETE_API_URL` origin (outside browsers), and the
origin of an `apiUrl` you set in code. An explicit `apiUrl` covers only
credentials you pass in `auth`. An `ARETE_API_KEY` key goes to
`https://api.arete.run` and `ARETE_API_URL`, and an `a4` login key goes only to
`https://api.arete.run`.

A session endpoint named by a stack, binding or generated descriptor, or by a
`sessionEndpoint` override, is never trusted for being named, even with
`apiUrl` set. It must be on a trusted origin itself. Otherwise nothing is
sent, and the call rejects with an `AreteError` whose code is
`UNTRUSTED_CREDENTIAL_DESTINATION`; its `details` carry `destination`,
`credential` and `trustedOrigins`. To use an API you run, set `apiUrl` (or
`ARETE_API_URL`) to its origin. Publishable keys and Arete session tokens are
not account credentials and are unaffected.

### Arete API client

`session.platform` and `client.platform` are an authenticated Arete API client
when `auth` carries an API credential (a secret key, or `tokenKind: 'api'`),
and `undefined` otherwise. If that credential may not go to `apiUrl` (an
`ARETE_API_KEY` or `a4` login key with `apiUrl` on another origin), the client
still exists and each request rejects with `UNTRUSTED_CREDENTIAL_DESTINATION`;
`createPlatformHttpClient` throws it instead. `createSession` and `Arete.connect` options take
`apiUrl` (default `https://api.arete.run`) and `platform` (a client to use
instead). To call the API without a session:

```ts
import { createPlatformHttpClient } from '@usearete/sdk';

const platform = createPlatformHttpClient({ auth: { secretKey: process.env.ARETE_API_KEY } });
// A hypothetical route, for illustration only.
const { status, body } = await platform.request<{ items: string[] }>({ path: '/example/items' });
```

`createPlatformHttpClient({ apiUrl?, auth?, fetch? })` reads `secretKey`,
`token`, `getToken` and `tokenEndpoint` as API credentials (outside browsers an
unset `auth` falls back to `ARETE_API_KEY`, then the `a4` login), never mints a
session, and refuses publishable keys with `INVALID_CONFIG`. `request({ path,
method?, body?, signal? })` resolves to `{ status, body }`, retries once after
a 401 when the credential comes from `getToken` or `tokenEndpoint`, and rejects
a non-2xx answer with an `AreteError` whose `details` carry `status`,
`wireErrorCode`, `body` and `apiProblem`.

### Arete API calls from SDK extensions

Program extensions receive the client's Arete API client as `context.platform`
in `ProgramOperationContext` (stack extensions read `client.platform`). It is
there only with an API credential: a secret key on a server, or
`tokenKind: 'api'`. A publishable key identifies a visitor to an app, never the
account, so with one `context.platform` is `undefined`; public data reaches the
browser through the program's own session transports. Check for it and say
what the app must configure:

```ts
import { AreteError, defineProgramExtensions } from '@usearete/sdk/program';

export default defineProgramExtensions<typeof VAULT>()({
  createRead(context) {
    return {
      async watchedVaults() {
        if (!context.platform) {
          throw new AreteError(
            "read.watchedVaults calls the Arete API: use a secret key on a server or auth.tokenKind 'api'",
            'PLATFORM_CLIENT_REQUIRED',
          );
        }
        // A hypothetical route, for illustration only.
        const { body } = await context.platform.request<{ addresses: string[] }>({
          path: '/example/watchlist',
        });
        return context.program.accounts.Vault.fetchMany(body.addresses);
      },
    };
  },
});
```

`PlatformHttpClient` and its request and response types are exported as types
from `@usearete/sdk/program`, so extension code never imports the client
runtime.

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
- `read` - optional program-oriented convenience reads supplied by the SDK
- `raw` - exact IDL instruction builders with synchronous `.build(...)`
- `instructions` - semantic one-instruction operations with `.prepare(...)`
- `transactions` - semantic one-transaction operations with `.prepare(...)`
- `flows` - program-local multi-transaction operations with `.prepare(...)`
- `constants` - program constants and enums
- `defaults` - reusable program defaults
- `math` - pure protocol calculations

### Program Read Transports

Generated account readers use a `ProgramReadDescriptor` for each program. Local
generation emits an explicit `local-http` descriptor, so the endpoint must come
from the connection call:

```ts
const client = await Arete.connect(LOCAL_STACK, {
  url: 'ws://localhost:8877',
  httpUrl: 'http://localhost:8877',
});
```

Installed hosted program SDKs instead carry a complete `hosted-binding`
descriptor opaquely inside the exported program cartridge. Passing that one
program object to `createSession` or `ConnectOptions.programs` automatically
activates its release-pinned account reads; consumers do not import or register
the descriptor separately. Hosted account reads ignore `httpUrl` and stack HTTP
endpoints. The advanced `programReads` override remains available, but it must
replace the complete descriptor; release, binding, and endpoint fields are never
patched independently.

Stack and program queries remain stack-HTTP operations. They continue to use
`httpUrl` or `stack.endpoints.http` and do not use a hosted Program Read binding.
Composition sessions accept only complete hosted descriptors or complete hosted
overrides, and never infer Program Read endpoints from their members.

### Hosted Solana Gateway Transports

Hosted stack and program installs embed complete, independent `chain` and
`transactions` gateway bindings. `Arete.connect`, standalone program sessions,
and generated composition sessions select those authenticated transports by
default instead of using a LiveSpec query endpoint. Explicit transports always
remain available as overrides:

```ts
import { createHostedSolanaGatewayTransports } from '@usearete/sdk';
import { MY_STACK_HOSTED_BINDINGS } from './generated/my-stack';

const { chain, transactions } = createHostedSolanaGatewayTransports(
  {
    chain: MY_STACK_HOSTED_BINDINGS.chain,
    transactions: MY_STACK_HOSTED_BINDINGS.transactions,
  },
  {
    auth: { publishableKey: import.meta.env.VITE_ARETE_PUBLISHABLE_KEY },
  }
);
```

The helper mints tokens for the exact `solana-gateway-binding` target. `read`,
`transaction:inspect`, and `transaction:send` use separate scope caches. An
authentication failure is refreshed at most once, and transaction requests are
replayed only when the gateway explicitly reports that upstream dispatch did
not begin. Chain and transaction descriptors may intentionally share the same
endpoint and binding ID.

Generated hosted compositions additionally retain
`create<StackName>HostedSession(options)` as an explicit convenience helper.
The ordinary `create<StackName>Session(options)` now uses the embedded gateway
too. Local/self-hosted compositions carry no gateway descriptor and continue to
require caller-supplied `chain` and `transactions` transports.

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

### Building without a connection: `@usearete/sdk/program`

`@usearete/sdk/program` is the subset that builds data and never connects: instruction handlers (`createInstructionHandler`, `buildInstruction`), PDA seeds, account read descriptors, operation metadata and amount conversion. It reaches no client, WebSocket, `fetch`, `process`, `Buffer` or Node built-in, so it bundles into code that runs without network or file access, such as a protocol's handlers:

```ts
import { buildInstruction } from '@usearete/sdk/program';
import { pingInstruction } from '@programs/vote';

const ix = buildInstruction(pingInstruction, { payer: wallet });
```

It also carries what a program's curated extensions build with: the SPL and system program addresses, `deriveAssociatedTokenAccount`, PDA seed and base58 helpers, the prepared-operation builders (`createPreparedInstruction`, `createPreparedTransaction`, `createPreparedFlow`) and the operation markers (`instructionOperation`, `transactionOperation`, `flowOperation`). Extensions for a program module import from `@usearete/sdk/program` only, so a handler that imports the module still bundles.

The root `@usearete/sdk` entry re-exports the same values, so a program built through one entry works with the other.

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

- `session.stacks.<name>` - connected generated stacks; each exposes `stack` (its definition, with attached programs) and `transport` (`'ws'` when it opened the streaming WebSocket, `'http'` otherwise), as every connected client does
- `session.programs.<name>` - connected packaged, attached, or standalone programs
- `session.protocols.<name>` - protocol clients
- `session.chain` - canonical generic chain reads
- `session.wallet` and `session.setWallet(wallet)` - the shared wallet
- `session.signerRegistry`
- `session.transaction(...)`
- `session.execute(...)`

In React, `AreteSessionProvider` and `useAreteSession` from `@usearete/react` own the session; see [Compose a session in React](https://docs.arete.run/sdks/react-sessions/).

Prefer these connected paths in application code. `Arete.connect(STACK, ...)` remains available when a direct single-stack client is more convenient.

Programs are matched by identity, not by name (`compareProgramIdentity(a, b)` returns `'same' | 'unproven' | 'different'`). A program SDK's identity is its `packageReleaseHash`, the program package release a registry-installed SDK was generated from; local builds have none.

- Both have one: equal hashes (or the same object) are the same program. A standalone program that a stack also provides is served by that stack's connected instance: one client, no warning. Different hashes throw `AreteError` with code `PROGRAM_KEY_CONFLICT` before anything connects.
- At least one has none, with the same `programSpecHash`: the explicitly attached program takes the key, with one `console.warn`. A standalone session program takes `session.programs.<key>`, and `session.stacks.<name>.programs.<key>` keeps the stack's. Two stacks with such copies promote the first stack's.
- Anything else (a different or missing `programSpecHash`) throws `PROGRAM_KEY_CONFLICT`; use `session.stacks.<name>.programs.<key>` or attach the standalone program under another key. Two stacks bundling different programs under one key both stay reachable through their stacks, and reading `session.programs.<key>` throws `PROGRAM_KEY_CONFLICT` naming them.

The same rule applies to `withPrograms`, `ConnectOptions.programs`, a session member's `programs`, and React's `useArete(stack, { programs })`, where the attached program replaces the stack's for that client. `isSameProgramSdk(a, b)` is `compareProgramIdentity(a, b) === 'same'`.

`extendProgram`, `extendPrograms`, and `withProgramRead` drop `packageReleaseHash`, because a program changed outside its generated SDK is no longer provably that SDK. Generated entries stamp it last with `withProgramIdentity(program, { packageReleaseHash })`, after the package's own extension.

Generated stack and program objects carry their runtime extensions under registry symbols, so `{ ...MY_STACK }` keeps `read`, `flows`, program operations and read descriptors, while `Object.keys` and JSON never list them. `EXTENSION_API_VERSION` (also `arete.extensionApi` in this package's `package.json`) versions that extension contract and changes only on a breaking change.

## Protocols

A protocol is a hosted state machine over Solana programs: an instance moves through states as wallets call its steps, guards decide whether a step may run, and the hosted API builds each step's transactions for the caller's wallet to sign. Install one as a typed module:

```bash
a4 install protocol token-raise --ts
```

This writes `generated/typescript/protocols/token-raise/token-raise.ts`. Its default export (`TOKEN_RAISE_PROTOCOL`) carries the deployment it talks to, the client-callable steps with their guard codes, the events, the metadata fields, and the stacks and programs the protocol uses. It never contains credentials. The module also exports the types (`TokenRaiseProtocolTypes`, `TokenRaiseContributeArgs`, ...) and `verifyTokenRaiseWebhook`.

### Clients and keys

```ts
import { createProtocolClient } from '@usearete/sdk';
import TOKEN_RAISE from './generated/typescript/protocols/token-raise/token-raise';

// Browser: publishable key, bound to your origins.
const raise = createProtocolClient(TOKEN_RAISE, { auth: { publishableKey: 'a4_pk_...' } });

// Server: secret key (or ARETE_API_KEY).
const admin = createProtocolClient(TOKEN_RAISE, { auth: { secretKey: process.env.ARETE_SECRET_KEY! } });
```

Every step is either `public` or `secret`. The client is typed by key class: a publishable-key client's `steps` holds only the `public` steps, so calling a secret step does not compile; a secret-key client has every client-callable step. With `token`, `getToken`, `tokenEndpoint` or `ARETE_API_KEY`, pass `keyClass: 'secret'` to get the secret type. The runtime enforces the same rule: a publishable client asked for a secret step resolves to the `step_requires_secret_key` rejection without a network call. Steps only a keeper or a multisig can run are never on the client.

Sessions are minted for the protocol deployment and scoped to what the client does. Reads, metadata, allowed steps and event streams use a read-only session. The first `prepare` or `submit` mints a session that can prepare the client's steps (the `public` steps for publishable keys, or `options.steps`; every step for secret keys) and submit. Read-only pages therefore never mint a session that can prepare. Expired sessions are re-minted and the request retried once.

### Prepare, sign, submit

```ts
const result = await raise.steps.contribute.prepare({
  instance: instanceId,
  wallet: walletAddress,
  amount: 1_000_000n, // u64 and wider integers take a bigint or a decimal string
});

if (!result.ok) {
  switch (result.rejection.rejected) {
    case 'raise_not_open':
    case 'deadline_passed':
    case 'wallet_cap_exceeded':
    case 'hard_cap_exceeded':
      return showGuard(result.rejection); // `guard` and `detail` explain the evaluation
    case 'step_requires_secret_key':
    case 'step_not_in_session_scope':
    case 'step_not_client_callable':
      return showAccess(result.rejection);
    case 'instance_terminal': // the instance has ended
    case 'participant_terminal': // this wallet's participant has ended
      return showFinished(result.rejection); // `detail.state` names the terminal state
    default: {
      const unreachable: never = result.rejection.rejected;
      return unreachable;
    }
  }
}

// result.transactions: partially signed transactions (base64), one per intent transaction.
const signed = await wallet.signAllTransactions(result.transactions.map(decode));
const state = await raise.submit(result.intent, signed.map(serialize)); // base64 strings or bytes
await raise.intents.get(state.executionId);
```

`create`-scope steps take `{ wallet, ...args }`; instance and participant steps take `{ instance, wallet, ...args }`. `prepare` options: `transactionFormat` (`'v1'` default, `'v0'` may return several transactions), `idempotencyKey` (generated when absent) and `clientContext`. A rejection's `rejected` is the step's guard-code union plus the access codes and the state codes (`instance_terminal`, `participant_terminal`, each with `detail.state`), so a `switch` over it is exhaustive. Other failures throw `AreteError`.

### Instances, participants and events

```ts
const page = await raise.instances.list({ state: 'open', metadata: { ticker: 'ABC' }, limit: 20 });
const instance = await raise.instances.get(id, { wallet }); // allowedSteps evaluated for `wallet`
const steps = await raise.allowedSteps({ instance: id, wallet }); // [{ step, allowed, reason?, detail? }]
const me = await raise.participants.get({ instance: id, wallet });

// Everything `wallet` can do now: create steps plus the instances it relates to.
const actions = await raise.actions({ wallet, limit: 20 }); // { createSteps, instances, nextCursor }
for (const { instanceId, relation, steps } of actions.instances) {
  // relation: ['participant' | 'creator' | `role:${name}`]; steps: [{ step, allowed, reason?, scope?, args? }]
}

const subscription = raise.events(id, {
  onEvent(event) {
    if (event.type === 'emit') console.log(event.event); // 'raise.succeeded' | ...
  },
  onError: console.error,
});
subscription.close();
```

`actions({ wallet, instance?, cursor?, limit? })` reads which steps a wallet can call across the deployment: `createSteps`, evaluated without an instance (guards that read the arguments or the instance-to-be are listed in `argsDependent`), and one page of the instances the wallet relates to, each with its `state`, `relation`, `participantState` and `steps`. Each step carries its `scope` and its typed `args` (`{ name, optional, type, format?, description? }`: the JSON type, with a `format` such as `u64` or `pubkey` for Solana amounts and keys), so an agent or UI can build the call without the spec. Pass `nextCursor` back as `cursor` for the next page (`limit` is at most 50), or `instance` to read one instance.

`events` reads the instance's Server-Sent Events stream over `fetch` (so the session token is sent as a header), delivering `transition`, `participant`, `observation` and `emit` events. The server ends streams periodically and when the session expires; the client reconnects with `Last-Event-ID` and a fresh token, so no event is skipped. Pass `lastEventId` to resume a stream you stored.

`instances.use(id)` returns a handle for `useSyncExternalStore`-style consumers: `get()` returns `{ instance, live }`, refreshed from the hosted API on every stream event. Inside a session, when the protocol ships its own stack, `live` is that stack's row for the instance.

### Metadata

Creator-writer fields need a secret-key client. Participant-writer fields need a signature from the participant's wallet:

```ts
import { protocolMetadataProof } from '@usearete/sdk';

const metadata = raise.metadata.participant({ instance: id, wallet });
const patch = { note: 'hello' }; // JSON merge patch; null removes a field
const proof = metadata.proofMessage(patch, { wallet }); // expires in 4 minutes by default
const signature = await walletAdapter.signMessage(proof.bytes); // 64-byte Ed25519
await metadata.update(patch, {
  walletProof: protocolMetadataProof({ wallet, signature, expiresAt: proof.expiresAt }),
});
```

The signed message is six lines: `arete.protocol-metadata/v1`, the deployment id, the instance id, the wallet, the lowercase hex SHA-256 of the patch's RFC 8785 canonical JSON (`canonicalizeJson`), and `expiresAt` in unix seconds (at most 300 seconds ahead). `protocolMetadataProofMessage` builds it without a client.

### Sessions

```ts
const session = await createSession(
  { protocols: { tokenRaise: TOKEN_RAISE } },
  { auth: { publishableKey: 'a4_pk_...' } }
);
session.protocols.tokenRaise.steps.contribute;
session.stacks.meteoraPresale; // hoisted from the protocol, connected once
```

Each protocol's stacks join `session.stacks` and its programs join `session.programs`. A stack or program the session already has (the same object or the same `packageReleaseHash`) is connected once and shared, even across protocols; a different one under a taken key throws `STACK_KEY_CONFLICT` or `PROGRAM_KEY_CONFLICT` before anything connects. `session.protocols.<key>.stacks` and `.programs` are the session's connected members. Protocol clients are secret-typed when `auth.secretKey` is given or `protocolKeyClass: 'secret'` is set; per-protocol options go in `options.protocols.<key>`.

### Webhooks

Endpoints subscribed to a protocol's events receive signed POSTs. Verify them with the raw body:

```ts
import { verifyTokenRaiseWebhook } from './generated/typescript/protocols/token-raise/token-raise';

const { id, payload } = verifyTokenRaiseWebhook(secret, rawBody, request.headers);
// payload.event: 'contribution.confirmed' | 'raise.failed' | 'raise.succeeded'
```

`verifyWebhook` checks the `arete-webhook-id`, `arete-webhook-timestamp` and `arete-webhook-signature` headers (`v1=<hex HMAC-SHA256 of "<timestamp>.<body>">`, several entries while a secret rotates) with a 300-second tolerance, and throws `WebhookVerificationError` with a `code` on failure.

## Chain Reads

Generic chain reads use the console-level `session.chain` surface.

Available reads include:

- `exists(address)`
- `lamports(address)`
- `minimumBalanceForRentExemption(space)`
- `clock()`
- `account(address)`
- `accounts(addresses)` - batched `account(address)`, up to 100 per call, results aligned with the input order
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
const latest = await session.stacks.ore.views.OreRound.latest.getOne();
```

`get` and `getOne` open (or reuse) an equivalent subscription, wait for its initial snapshot, and release it. They reject with `InitialDataTimeoutError` after `timeoutMs` (5000 by default; `null` waits forever). `getSync` only reads a subscription that is already active and returns `undefined` when there is none.

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
  | { type: 'upsert'; key: string; data: T; cursor?: string }
  | { type: 'patch'; key: string; data: Partial<T>; cursor?: string }
  | { type: 'remove'; key: string; cursor?: string }
  | { type: 'delete'; key: string; cursor?: string };

type RichUpdate<T> =
  | { type: 'created'; key: string; data: T; cursor?: string }
  | { type: 'updated'; key: string; before: T; after: T; patch?: unknown; cursor?: string }
  | { type: 'removed'; key: string; lastKnown?: T; cursor?: string }
  | { type: 'deleted'; key: string; lastKnown?: T; cursor?: string };
```

`remove` means an entity left only this query's filter or window. `delete` means the source entity was deleted and is removed from every query for that view.

A `patch` for a key the client holds no entity for (never received, or evicted by `maxEntriesPerView`) is discarded rather than stored as a partial entity; the entity appears with the server's next full `upsert`. Each discard is reported to `onFrameValidationError` with `reason: 'unknown-key'`. Replayable append-view records (frames with an `offset`) are events and are always applied.

## Testing

`@usearete/sdk/testing` provides supported, dependency-light test helpers:

- `createWebSocketHarness()` — a scripted WebSocket server; pass `harness.websocketFactory` as `auth.websocketFactory` and answer subscriptions with `frames.*` builders.
- `createFrameHarness()` — the store engine without a socket.
- `createFakeTransactionTransport()` — a recording relay that can fail or stall.
- `createWalletFixture()` — a recording wallet with scripted outcomes; `createTransactionOutcomeFixtures()` covers every status and phase.
- `createFetchStub(routes)` — a routed, recording `fetch`.

```ts
import { createWebSocketHarness, createWalletFixture } from '@usearete/sdk/testing';

const ws = createWebSocketHarness();
const client = await Arete.connect(MY_STACK, {
  auth: { websocketFactory: ws.websocketFactory },
  wallet: createWalletFixture(),
});
```

## Replay cursors

Updates from an append view backed by the server's journal carry `cursor`, the
`{epoch}:{offset}` position of the event. Store it with the data you derive
from that update and pass it back as `after` to resume exactly where you
stopped — `after` is exclusive, and it is never a `_seq` value. State and list
views project membership rather than a tape, so their updates have no cursor.

```ts
for await (const update of session.stacks.myStack.views.Trade.list.watch()) {
  await db.apply(update, update.cursor); // one transaction: data + position
}
```

Reconnects resume from the last cursor delivered on each subscription. If a
record is lost locally the stream ends with a `StreamGapError` carrying the
last cursor delivered before the loss, rather than skipping records silently.
Server-side refusals (`cursor-expired`, `cursor-epoch-changed`,
`cursor-unknown`, `invalid-cursor`, `replay-gap`, `replay-lagged`) end the
stream with their wire code; `isReplayErrorCode` identifies them, and the
failing frame — including `replayWindow` and `recoverFrom` — is on the error's
`details`.

## License

MIT
