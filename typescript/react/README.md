# Arete React SDK

React hooks and provider for consuming Arete stacks in React applications.

Built on top of [`@usearete/sdk`](https://www.npmjs.com/package/@usearete/sdk), the framework-agnostic TypeScript core SDK.

## Installation

```bash
npm install @usearete/react @usearete/sdk zod
```

`@usearete/react` installs `zustand` as a normal dependency, so do not install it separately. React and `@usearete/sdk` are peer dependencies: the application owns exactly one copy of the core SDK, on the same release as `@usearete/react`, so error and outcome checks see one set of classes. Generated consumers install the core SDK and Zod because generated stack files import their types and schemas directly.

### Hooks linting

Arete's fluent `.use()`, `.useOne()`, and `.useMutation()` calls are real React
hooks, but the standard Hooks rule cannot identify deeply nested member calls.
The package ships `@usearete/react/eslint-plugin.cjs` with an
`arete/fluent-hooks` rule for flat ESLint configurations. The ORE template also
configures the equivalent bundled rule for ESLint 8. Use it together with
`react-hooks/rules-of-hooks`, not instead of the standard rule.

```js
// eslint.config.js
import areteHooks from '@usearete/react/eslint-plugin.cjs';

export default [{
  plugins: { arete: areteHooks },
  rules: { 'arete/fluent-hooks': 'error' },
}];
```

> Not using React? Use [`@usearete/sdk`](../core/README.md) directly.

## Quick Start

Keep the generated stack explicit at the provider and consumer boundaries. Equivalent calls share one provider-managed client and store:

```tsx
import { AreteProvider, useArete } from '@usearete/react';
import { ORE_STREAM_STACK } from './generated/ore-stack';

const publishableKey = import.meta.env.VITE_ARETE_PUBLISHABLE_KEY;
if (!publishableKey) throw new Error('VITE_ARETE_PUBLISHABLE_KEY is required');

function Dashboard() {
  const arete = useArete(ORE_STREAM_STACK);
  const board = arete.views.OreBoard.state.use({
    address: arete.addresses.board(),
  });
  const roundId = board.data?.state.roundId;
  const round = arete.views.OreRound.state.use(
    roundId == null ? undefined : { roundId },
  );

  if (arete.status === 'error') {
    return <button onClick={() => void arete.retry()}>Reconnect</button>;
  }
  if (board.status !== 'ready' || round.status !== 'ready') {
    return <p>Connecting...</p>;
  }

  return (
    <div>
      {arete.socketIssue && <p>{arete.socketIssue.message}</p>}
      <p>Round {round.data?.id.roundId?.toString() ?? 'unavailable'}</p>
    </div>
  );
}

export default function App() {
  return (
    <AreteProvider
      autoConnect
      stack={ORE_STREAM_STACK}
      auth={{ publishableKey }}
    >
      <Dashboard />
    </AreteProvider>
  );
}
```

The hosted ORE stack requires this publishable key for authentication and rate limiting. Read-only browser viewing does not require a wallet, but it is not unauthenticated.

Single-stack applications can remove the repeated constant without ambient types by binding the React API once:

```tsx
import { createAreteReact } from '@usearete/react';
import { ORE_STREAM_STACK } from './generated/ore-stack';

export const {
  Provider: OreProvider,
  useArete: useOre,
} = createAreteReact(ORE_STREAM_STACK);
```

`useOre()` remains explicit about the application dependency. Multi-stack apps can keep using `AreteProvider` and `useArete(stack)` directly.

Provider-default `useArete()` calls remain available. For full type inference, register the stack type once (e.g. in `src/arete.d.ts`):

```ts
import type { OreStreamStack } from './generated/ore-stack';

declare module '@usearete/react' {
  interface AreteDefaultStackRegistry {
    defaultStack: OreStreamStack;
  }
}
```

Multi-stack apps can keep passing the stack explicitly: `useArete(stack, options)` always wins over the provider default.

## Provider

`AreteProvider` manages connected clients for descendant hooks.

Supported props:

- `stack` — default stack for argument-less `useArete()` calls
- `stackOptions` — default client lookup options (`url`, `httpUrl`, `transport`, `transactions`, `programs`) for the provider stack, including explicit `useArete(providerStack)` calls
- `autoConnect` — defaults to `true`; controls only whether the provider starts the initial connection
- `autoReconnect` — defaults to `true`; controls only recovery after an established connection is lost
- `wallet`
- `auth` (see [Signing users in with their Arete login](#signing-users-in-with-their-arete-login))
- `fetch`
- `validateFrames` — set `false` to suppress rejected-frame warnings; generated schemas still normalize and validate entities
- `onFrameValidationError` — structured callback for frames that were not stored: schema rejections (`reason: 'schema'`) and patches for keys the client holds no entity for (`reason: 'unknown-key'`)
- `reconnectIntervals`
- `maxReconnectAttempts`
- `maxEntriesPerView`
- `flushIntervalMs`

These are provider-wide defaults. Per-call overrides stay on the hook call and always win. Connection settings are captured when a shared client is created; changing provider props does not mutate that client. `wallet` updates are synchronized in place. For other changed settings, call `retry()` to replace the shared client using the latest provider configuration.

The lifecycle flags are intentionally independent. `autoConnect={false}` leaves initial connection ownership to the application. It does not disable automatic recovery for a connection the application establishes, but `canRetry` remains false because provider-owned replacement attempts would also stay disconnected; use the client lifecycle directly. `autoReconnect={false}` does not suppress the initial connection; it disables only automatic recovery after that connection is lost.

### Signing users in with their Arete login

An app whose users sign in to Arete (a dashboard built on the SDKs) passes their login as an API credential. The SDK sends it only to the Arete API and its session endpoint; stack, gateway and program read connections mint their own sessions with it:

```tsx
// Module level, so its identity is stable.
const auth = { tokenKind: 'api', tokenEndpoint: '/api/arete-token', tokenEndpointCredentials: 'same-origin' } as const;

<AreteProvider auth={auth}>
  <App />
</AreteProvider>;
```

`getToken` works the same (`{ tokenKind: 'api', getToken: async () => session.accessToken }`). `usePlatformHttpClient({ apiUrl })` returns a client for the Arete API's account routes with that same auth, to hand to SDK extensions that take an `http` client.

## Sessions: `AreteSessionProvider` and `useAreteSession`

`AreteProvider` manages stack clients. For a [`createSession`](../core/README.md) session (programs, stacks and protocols behind one auth and one wallet), put an `AreteSessionProvider` above the components that use it:

```tsx
import { AreteSessionProvider, useAreteSession, useOperationMutation } from '@usearete/react';
import { SQUADS_V4_PROGRAM } from './sdk/squads-v4';

// Module level, so their identity is stable.
const SQUADS = { programs: { squads: SQUADS_V4_PROGRAM } };
const auth = { tokenKind: 'api', tokenEndpoint: '/api/arete-token', tokenEndpointCredentials: 'same-origin' } as const;

<AreteSessionProvider definition={SQUADS} options={{ auth }} wallet={wallet}>
  <Squads />
</AreteSessionProvider>;

function Squads() {
  const { session, status, error } = useAreteSession<typeof SQUADS>();
  const squads = session?.programs.squads; // typed from the definition
  …
}
```

`useAreteSession()` returns `{ session, status, error, ready, retry }`. `status` is `opening`, `open` or `error`; `session` is `null` until it is `open`. `ready()` resolves with the session once it is open, waiting through a reopen; it is stable, so it can be a `useOperationMutation` executor. `retry()` opens it again after a failure. Type it with the definition (`useAreteSession<typeof SQUADS>()`) or with a session type (`useAreteSession<MySession>()`). It throws outside an `AreteSessionProvider`.

How the provider manages the session:

- **Opened once, closed on unmount.** Given `definition` (and `options`, as for `createSession(definition, options)`), the provider opens the session when it mounts and closes it when it unmounts.
- **Auth from `AreteProvider`.** `options.auth` and `options.fetch` default to the enclosing `AreteProvider`'s, so one `auth` drives both.
- **A change reopens it.** When the definition or the options change, the old session is closed at once and a new one is opened; `session` is `null` until the new one is ready, so nothing runs against a session opened with credentials that no longer apply. A run waiting on `ready()` gets the new session. Options are compared by shape two levels deep: an inline `options={{ auth: { tokenKind: 'api', … } }}` with the same fields is the same options. Functions (`getToken`, `fetch`) compare by identity, so keep them stable (module level or `useCallback`).
- **The wallet does not reopen it.** `wallet` is a prop, not an option: changing it calls `session.setWallet`.
- **An existing session.** Pass `session={mySession}` instead of `definition` to share a session the app opened. The provider never closes it, and sets `wallet` on it only when the `wallet` prop is passed. `session={null}` while the app is still opening it leaves the provider `opening`.

Under React StrictMode in development the provider opens, closes and reopens the session once on mount, like any effect.

### Stacks in a session

A stack member of the session (`session.stacks.<key>`) is a connected client of its own. Pass it to `useArete` for the same views, programs, reads and mutation hooks `useArete(stack)` gives, typed for that stack, without a second connection:

```tsx
import { AreteSessionProvider, useArete, useAreteSession } from '@usearete/react';
import { ORE_STREAM_STACK } from './generated/ore-stack';

const APP = { stacks: { ore: ORE_STREAM_STACK } };

<AreteSessionProvider definition={APP} options={{ auth }} wallet={wallet}>
  <Round />
</AreteSessionProvider>;

function Round() {
  const { session } = useAreteSession<typeof APP>();
  // `null` while the session opens: the stack definition gives the placeholders.
  const ore = useArete(session?.stacks.ore, { stack: APP.stacks.ore });
  const round = ore.views.OreRound.latest.use();
  const deploy = ore.programs.ore.instructions.deploy.useMutation();
  …
}
```

- **One connection.** The hooks read the member's own connection and store; nothing connects again, and an enclosing `AreteProvider` opens nothing for it.
- **The session owns it.** Unmounting never disconnects the member. The provider closes it with the session.
- **While the session opens** the member is `null`, and the result is the one `useArete(stack)` gives before its client exists: `status: 'connecting'`, `client: null`, placeholder views and programs (addresses, PDAs and constants from the definition). Pass the stack definition as `stack` for this; TypeScript asks for it whenever the member can be `null` or `undefined`. If the session fails to open, `status` is `'error'` and `retry()` reopens it.
- **A reopen** (new auth or options) gives a new member: the hooks drop the old one and subscribe on the new one, with nothing carried over.
- **The wallet is the session's.** Mutations sign with the member's wallet, which the provider's `wallet` prop (`session.setWallet`) sets. `AreteProvider`'s `wallet` does not reach session members.
- With a member that is never `null` (`useArete(session.stacks.ore)`, for a session the app opened), `stack` can be left out. Pass `transport: 'http'` for a member the session connects over HTTP, so mutations do not wait for a streamed slot.

## `useArete(stack?, options?)`

`useArete` returns the connected React surface for a stack. With no arguments it resolves the provider's `stack`/`stackOptions`. Passing the same stack explicitly keeps the provider's `stackOptions`; per-call options override them, while another explicit stack is isolated from the provider stack's options. It throws a descriptive error when no stack is passed and no default is configured.

- `views`
- `programs`
- `queries`
- `read` — callable stack reads with `.use(...args, options?)` React hooks
- `chain`
- `client`
- `reads` — deprecated alias of `read`
- `status` — preferred connection state for UI; includes the initial client-creation window
- `connectionState` — raw state from the connected client
- `isConnected`
- `isLoading`
- `canRetry` — true when `retry()` can start a new shared connection attempt
- `error`
- `socketIssue` — the latest structured server WebSocket issue, if any
- `retry()` — replaces the failed shared client; every matching `useArete` consumer observes the connecting window and adopts the same replacement

Supported hook options:

- `url`
- `httpUrl`
- `transport`
- `transactions` — explicit transaction transport when execution uses a different service from `httpUrl`
- `programs`

All options are optional. By default the client connects to the endpoints embedded in the generated stack definition (`stack.endpoints`), so `useArete(STACK)` with no options is the common case.

Notes:

- `transport: 'http'` disables streaming view subscriptions, but HTTP-backed surfaces like `queries`, `chain`, and program reads still work.
- Attached `programs` are keyed by value (program name and id): passing a fresh object literal each render still resolves to the same client.
- Repeated `useArete` calls with the same stack object and options share one provider-managed client and store. It is safe for sibling components to call the hook independently.

## Views

View hooks return a discriminated `ViewHookResult<T>`. `status` and `isEmpty`
narrow the associated data and error fields:

```ts
if (round.status === 'error') {
  console.error(round.error); // Error
} else if (round.status === 'ready' && !round.isEmpty) {
  console.log(round.data.id.roundId); // entity, not undefined
}
```

`status` distinguishes "no client yet" (`connecting`) from "subscribed, waiting for the first frame" (`subscribing`). `isPending` covers both states, `isReady` marks a usable ready result, and `isEmpty` distinguishes a ready hook with no entity or list entries from loading. Errors may retain previously committed data, but `isReady` becomes false. `isLoading` remains for compatibility.

Use the connection `status` together with `socketIssue` and `retry()` for recovery UI:

```tsx
const arete = useArete(ORE_STREAM_STACK);

if (arete.canRetry) {
  return <button onClick={() => void arete.retry()}>Reconnect</button>;
}

return <ConnectionBadge status={arete.status} issue={arete.socketIssue} />;
```

The view hook objects themselves are refresh targets — they refresh that view's *active* subscriptions without you holding a hook result, and are a no-op when nothing is subscribed:

```ts
await arete.views.OreRound.state.refresh({ roundId }); // one keyed subscription
await arete.views.OreRound.state.refresh();            // every active subscription of the view
await arete.views.OreRound.latest.refresh();           // list views

// Handy in mutation reconciliation — the component doesn't need its own
// subscriptions just to refresh them:
claim.submit(args, {
  reconcile: { refresh: [arete.views.OreBoard.state, arete.views.OreMiner.state, preview] },
});
```

When several resources feed one status line, aggregate them with `summarizeStatuses` instead of hand-rolling loading/error lists:

```tsx
const streams = summarizeStatuses({ Board: board, Round: round, Miner: authority && miner });
// streams.loading  → names still loading, e.g. ['Board']
// streams.refreshing → names resynchronizing committed data
// streams.errors   → 'Name: message' strings
// streams.isLoading / streams.isRefreshing / streams.hasError
```

Falsy entries (`authority && miner`) are skipped.

List views support:

- `.use()`
- `.use({ take: 1 })`
- `.useOne()`

Put list query configuration (`take`, `skip`, `key`, `partition`, `filters`, `after`, `withSnapshot`, `snapshotLimit`, `schema`, and `onSchemaValidationError`) in the first params object. For compatibility, fields also accepted by view options are honored from the second argument when absent from params; params win when both are supplied. Keep `enabled` and `initialData` in the second argument. State hooks accept one entity as `initialData`, list hooks accept an entity array, and `useOne` accepts one entity.

`onSchemaValidationError(diagnostic)` observes entities rejected by a caller-supplied schema. The diagnostic is `{ view, key?, entity, error }`. Accepted entities and existing filtering behavior are unchanged.

Protocol v2 gives each parameter set independent ordered membership. Different windows and filters on one view can coexist, while equivalent normalized queries share one reference-counted wire subscription:

```tsx
const firstPage = views.OreRound.latest.use({ take: 10 });
const secondPage = views.OreRound.latest.use({ take: 10, skip: 10 });

if (firstPage.isLoading) return <p>Loading...</p>;

return (
  <>
    {firstPage.isRefreshing && <p>Refreshing...</p>}
    <RoundList rounds={firstPage.data ?? []} />
    <RoundList rounds={secondPage.data ?? []} />
  </>
);
```

During reconnect, hooks preserve their committed data and set `isRefreshing`. A completed authoritative snapshot then atomically replaces membership for that exact query, including an empty result.

`await result.refresh()` resolves after the refreshed subscription's next complete snapshot is committed. Registration, send, and subscription failures reject, clear `isRefreshing`, and appear in `result.error`. A view refresh with no active matching subscription remains a no-op.

On a replayable append view, `result.cursor` is the `{epoch}:{offset}` position of the last event delivered to that hook. Persist it with whatever you derive from the data and pass it back as `after` to resume; `after` is exclusive and is never a `_seq` value. State and list views project membership rather than a tape, so their hooks have no cursor. A server refusal of a cursor (`cursor-expired`, `cursor-epoch-changed`, `cursor-unknown`, `invalid-cursor`, `replay-gap`, `replay-lagged`) arrives in `result.error` under its wire `code`, each with a different recovery.

State views support:

- `.use(key)` — subscribe to one keyed entity using the generated key type (for example `{ roundId: bigint }`)
- `.use(undefined)` — disabled: no subscription, `{ data: undefined, isLoading: false }`. Useful while a wallet is not connected: `views.OreMiner.state.use(authority ? { authority } : undefined)`.

Keyed hooks only expose data for the key you passed: when the key changes, `data` becomes `undefined` (and `isLoading` true) until the new key's snapshot arrives. Components never need to re-verify returned data against their inputs.

`initialData` is trusted as immediately usable ready data. The first live store update or completed empty snapshot replaces it; an empty list settles as `[]` and an absent state entity settles as `undefined`. Do not seed transaction-authorizing data unless the application is willing to trust that seed.

> **BigInt props in development:** entities contain bigints, and React ≤19.2's dev-mode performance track calls `JSON.stringify` on changed props — it crashes on bigint *arrays and objects* (scalar bigints are fine), wedging the app after the first live update. Prefer stack-provided UI fields when they exist (e.g. `deployedPerSquareUi` next to `deployedPerSquare`), use `stringifyBigints(value)` for a typed structural bigint→string conversion of the rest (raw account snapshots are the common case), or upgrade to React 19.3+ where this is fixed.

## Reads

Stacks can expose one-shot read functions (e.g. RPC-backed previews). Each one gets a React hook:

```tsx
const preview = arete.read.solClaimPreview.use(authority);
const quote = arete.read.quoteManualDeployment.use(
  input,
  { debounceMs: 300 },
);
```

- Arguments form the cache key — the read re-runs when they change, and the previous arguments' data is never exposed while the fresh result is loading.
- The hook is disabled while any required argument is `null`/`undefined`, so no `enabled` flag or non-null assertions are needed.
- `.use(...args, { debounceMs, initialData })` adds React-only behavior with options last. Debounce applies to automatic argument changes; `refresh()` always runs immediately. When options follow an omitted optional read argument, pass `undefined` in that argument's position.
- `status` is `'disabled' | 'connecting' | 'loading' | 'ready' | 'refreshing' | 'error'`. The existing loading and refreshing booleans remain available.
- `isPending`, `isReady`, and `isEmpty` cover the common UI branches without comparing status strings.
- `preview.refresh()` re-runs the read on demand, typically after a mutation (see below).

The same functions remain callable for imperative access: `await arete.read.solClaimPreview(authority)`. The old `arete.reads` namespace remains as a deprecated alias.

## Programs, Queries, and Chain

`useArete` mirrors the connected core client surface:

- `programs.<program>.raw.<instruction>` for raw typed instructions
- `programs.<program>.instructions`, `.transactions`, and `.flows` for semantic operations
- `programs.<program>.accounts` and `programs.<program>.queries` for HTTP reads
- `queries` for stack-level HTTP queries
- `chain` for chain helpers

Raw instruction hooks preserve `.execute`, `.build`, and `.useMutation()`.

Semantic operation hooks preserve:

- `.execute`
- `.prepare`
- `.useMutation()`

Mutation hooks expose `mutate(args, options)` for event handlers. It records
errors in hook state and does not return a promise. Use the equivalent rejecting
`mutateAsync(args, options)` or `submit(args, options)` when the caller must
await or compose the result.

Program and read hooks are safe to call while the client is still connecting: mutations throw "Arete client is not connected" if submitted early, and reads stay disabled.

### Running an operation or flow: `useOperationMutation`

`useOperationMutation(prepare, { executor?, wallet, signers? })` runs any prepared instruction, transaction or flow through anything with `execute(prepared, options)`: a connected client or a session, so it works without a stack. Inside an [`AreteSessionProvider`](#sessions-aretesessionprovider-and-usearetesession) `executor` can be left out: the hook runs on the provider's session, waiting for it to open. `prepare` returns the operation, or `{ operation, signers }` when it creates signers (a create key):

```tsx
import { generateKeyPairSigner } from '@solana/kit';
import { useOperationMutation } from '@usearete/react';

const create = useOperationMutation(
  async (input: { members: string[]; threshold: number }) => {
    const createKey = await generateKeyPairSigner();
    const operation = await squads.flows.arete.createMultisig.prepare({ ...input, createKey: createKey.address });
    return { operation, signers: [createKey] };
  },
  { executor: session, wallet },
);

<button disabled={create.isBusy} onClick={() => create.mutate(input)}>Create</button>;
{create.isAwaitingWallet && <p>Approve the request in your wallet</p>}
{create.error?.kind === 'wallet-rejected' && <p>You declined the request.</p>}
```

`status` walks `idle` → `preparing` → `signing` → `sending` → `confirming` → `done` (or `error`), once per transaction of a flow (`transactionIndex` of `transactionCount`). `sending` and `confirming` come from wallets that report their activity, as the `@usearete/adapter-kit` adapters do; any other wallet stays `signing` until it confirms. `prepared`, `artifacts`, `result` (the receipt) and `signatures` hold the latest run. `executor` may be a function that resolves it (a session opened on first use). A second run while one is in progress is refused with `busy`, so a double click never opens two wallet prompts. `run()` rejects; `mutate()` keeps the error in state.

**No wallet, no run.** The run signs with the `wallet` given to `run(input, { wallet })` or to the hook. Without one (`wallet` is `undefined` while no wallet is connected) it fails with `not-ready` before `prepare` is called, so nothing is prepared or signed. The executor's own wallet (`session.wallet`, the provider's) is used only with `allowExecutorWallet: true`.

`error` is an `OperationMutationError`; switch on its `kind`:

| `kind` | Meaning |
| --- | --- |
| `busy` | A run of this hook is already in progress; nothing was started. |
| `not-ready` | No executor, or no wallet; nothing was prepared or signed. |
| `prepare-failed` | `prepare` threw; nothing was signed. |
| `wallet-rejected` | The user declined or closed the wallet prompt. |
| `wallet-feature-unsupported` | The wallet cannot sign without sending (no `solana:signTransaction`). |
| `wallet-chain-unsupported` | The wallet does not sign for the chain it was asked to. |
| `wallet-version-unsupported` | The wallet signs, but not this transaction version. |
| `wallet-modified` | The wallet changed the transaction, so it was not sent. |
| `not-submitted` | Failed before anything reached the chain. |
| `submitted-unknown` | Sent, but the confirmation did not arrive; check the chain before trying again. |
| `chain-failed` | Failed on chain; see `programError`. |

It also carries the transaction `failure`, `programError` and the signatures that confirmed before it. `toOperationMutationError(error)` classifies a failure from an imperative `session.execute` the same way.

### Wallet Standard wallets: `@usearete/react/wallet-standard`

`useWalletStandardWallet(account, { chain })` turns the account a user connected with `@wallet-standard/react` into the wallet Arete signs with. Sign only: the wallet signs and Arete relays the bytes through the client's or session's transaction transport. It needs `@usearete/adapter-kit` (with `@solana/kit`) and `@wallet-standard/ui-registry`, optional peers of this package.

```tsx
import { useWalletStandardWallet } from '@usearete/react/wallet-standard';

const account = connectedWallet?.accounts[0] ?? null; // a UiWalletAccount
const { wallet, isPromptOpen, error } = useWalletStandardWallet(account, { chain: 'solana:mainnet' });

<AreteProvider wallet={wallet}>…</AreteProvider>;
```

`wallet` is `undefined` until an account is connected, and when it cannot sign for Arete. `error` then says why: a `WalletStandardSignerError` (exported from this entry) whose `code` is

- `sign_transaction_unsupported`: the wallet lacks `solana:signTransaction` (it only signs and sends in one step);
- `chain_unsupported`: the account does not list `chain` (a devnet account asked for `solana:mainnet`);
- `account_unavailable`: the account could not be read, or its wallet is no longer registered.

A declined prompt is not an `error` here: it fails the run (`wallet-rejected`, with a `user_rejected` signer error in its `cause` chain) and the wallet stays usable. `isPromptOpen` is true while the wallet's approval prompt is open, and only for the current account. A raw `{ wallet, account }` pair from the Wallet Standard registry is accepted too.

With a session provider:

```tsx
const { wallet, error } = useWalletStandardWallet(account, { chain: 'solana:mainnet' });

<AreteSessionProvider definition={SQUADS} options={{ auth }} wallet={wallet}>…</AreteSessionProvider>;

// below it
const propose = useOperationMutation(prepareProposal, { wallet });
if (error?.code === 'chain_unsupported') return <p>Switch your wallet to mainnet.</p>;
```

## Protocols

Protocol clients come from `@usearete/sdk` (`createProtocolClient(DEFINITION, { auth })` or `session.protocols.<key>`; see its README). Create one per key outside render (or memoize it) and pass it to these hooks:

```tsx
import { createProtocolClient } from '@usearete/sdk';
import { useAllowedSteps, useProtocolEvents, useProtocolInstance } from '@usearete/react';
import TOKEN_RAISE from './generated/typescript/protocols/token-raise/token-raise';

const raise = createProtocolClient(TOKEN_RAISE, { auth: { publishableKey: 'a4_pk_...' } });

function Raise({ id, wallet }: { id: string; wallet?: string }) {
  const { instance, live, error, isLoading } = useProtocolInstance(raise, id, { wallet });
  const steps = useAllowedSteps(raise, { instance: id, wallet }, { refreshOnEvents: true });
  useProtocolEvents(raise, id, {
    onEvent: (event) => {
      if (event.type === 'emit' && event.event === 'raise.succeeded') celebrate();
    },
  });

  if (isLoading) return <Spinner />;
  const contribute = steps.data?.find((step) => step.step === 'contribute');
  return (
    <button
      disabled={!contribute?.allowed}
      onClick={() => raise.steps.contribute.prepare({ instance: id, wallet: wallet!, amount: '1000000' })}
    >
      {instance?.state} {contribute?.reason}
    </button>
  );
}
```

- `useProtocolInstance(client, instanceId, { wallet? })` returns the canonical instance, refreshed on every event of the instance's stream, plus `live` (the protocol stack's row when the client belongs to a session whose protocol ships a stack). The stream opens while mounted.
- `useAllowedSteps(client, { instance, wallet })` and `useProtocolParticipant(client, { instance, wallet })` are one-shot reads with the same `status`, `refresh()` and argument-keyed behavior as other read hooks; they stay disabled until every argument is set. `refreshOnEvents: true` re-reads on stream events.
- `useWalletActions(client, { wallet, instance?, cursor?, limit? })` reads `client.actions`: the create steps and one page of instances the wallet relates to, with each step's availability, reason and typed args. It is disabled until `wallet` is set and re-reads when any argument changes (pass `data.nextCursor` as `cursor` to page). With `instance` set, `refreshOnEvents: true` re-reads on that instance's stream events.
- `useProtocolEvents(client, instanceId, { onEvent, onError?, lastEventId? })` follows the stream with the latest callbacks, reopening only when the client or instance changes.

All of them keep the generated types: instance and participant states, step names, guard codes and event names. A publishable-key client has only the `public` steps, so a secret step does not compile in browser code.

## Safe amount parsing

`safeToRawAmount(input, decimals)` is re-exported by `@usearete/react` for form validation. Unlike `toRawAmount`, it reports invalid input without throwing:

```tsx
import { safeToRawAmount } from '@usearete/react';

const parsed = safeToRawAmount({ ui: amountText }, 9);
if (!parsed.success) {
  setAmountError(String(parsed.error));
  return;
}

const rawAmount = parsed.data;
```

The return type is `{ success: true, data } | { success: false, error }`.

## Default reconciliation

Mutations from generated program hooks **reconcile against the stream by default**: after the transaction confirms on-chain, the hook waits until the stack has processed the highest confirmed receipt slot before settling (30s timeout). HTTP-only mutations and results without a receipt slot skip this step. The processed-slot watermark does not promise that a particular entity changed; use `refresh` for the views or one-shot reads that must be re-read.

Per-submit options:

```tsx
// Opt out entirely:
deploy.submit(args, { reconcile: false });

// Keep the default, but also refresh views and one-shot reads afterwards.
// `refresh` accepts view hook objects, view/read hook results, or plain
// callbacks:
claim.submit(args, {
  reconcile: { refresh: [arete.views.OreBoard.state, miner, preview] },
});

// Custom timeout:
deploy.submit(args, { reconcile: { timeoutMs: 10_000 } });

// Full control — a function replaces the default:
deploy.submit(args, { reconcile: async (context) => { /* ... */ } });
```

The mutation `phase` field is the discriminated status to branch on. Its values are exactly `'idle'`, `'preparing'`, `'awaiting-wallet'`, `'submitted'`, `'confirmed'`, `'reconciling'`, `'reconciled'`, `'confirmed-unreconciled'`, `'not-submitted'`, `'submitted-unknown'` and `'chain-failed'`. There is no `'submitting'` or `'confirming'` phase: `'awaiting-wallet'` covers signing, sending and confirming, because the wallet adapter does all three in one call. `status` remains `'pending'` until the mutation settles. Generated operation hooks publish each completed receipt and signature during `submitted`, before the final operation receipt resolves. `onConfirmed` runs immediately after chain confirmation; `onSuccess` runs only after successful reconciliation (or immediately when reconciliation is disabled/skipped). Reconciliation failure, including a refreshed snapshot timeout, is exposed through `reconciliationError`, does not call `onSuccess`, and does not reject the confirmed `submit()` result. The confirmed transaction remains landed. `retryReconciliation()` repeats only post-confirmation watermark/refresh work using the saved receipts and targets; it never rebuilds, signs, or submits. Convenience booleans (`isPreparing`, `isAwaitingWallet`, `isReconciling`, `canRetryReconciliation`) remain available.

## Migration notes (0.3 → next)

- `AreteProvider` accepts a default `stack` and `stackOptions`; `useArete()` may then be called with no arguments. Register the stack type via `AreteDefaultStackRegistry` for full inference.
- View hook results expose a `status` field (`'disabled' | 'connecting' | 'subscribing' | 'ready' | 'error'`).
- View hook objects (`arete.views.X.state` / `.list`) expose `refresh(key?)` and can be passed directly in `reconcile: { refresh: [...] }`.
- New `summarizeStatuses(namedResults)` status aggregator; `summarizeViews` remains as a deprecated alias.
- New `stringifyBigints(value)` display helper (also re-exported from `@usearete/sdk`).
- `UseAreteResult`'s second type parameter defaults to `undefined`.
- The duplicate low-level `useView` and `useEntity` hooks were removed. Use generated `arete.views.<Entity>.<view>.use(...)` hooks.

## Migration notes (0.2 → 0.3)

- **Generated-hook mutations reconcile by default.** `submit()` now resolves after stream reconciliation instead of right after confirmation. Timeouts settle as `confirmed-unreconciled` rather than failing. Pass `reconcile: false` to restore the old behavior.
- **State view keys are required and generated.** Pass the generated key object, or explicitly pass `undefined`/`null` to disable the hook. Zero-argument state subscriptions are no longer supported, and malformed keys throw the shared core serializer error.
- **Disabled view hooks report `isLoading: false`.** Previously `enabled: false` still started in a loading state.
- New additive `UseMutationResult` fields: `isPreparing`, `isAwaitingWallet`, `isReconciling`.
- New mutation callback: `onConfirmed`, for work that must run after chain confirmation even if stream reconciliation later fails.
- `useArete` now exposes `socketIssue` and deduplicated `retry()`.
- Stack reads expose imperative calls and hooks together under `arete.read.<name>`; `arete.reads` remains a deprecated alias.
- Repeated `useArete` calls now share a provider-managed client when stack and option identities match; remove component-local connection ownership workarounds.

## Testing

`@usearete/react/testing` re-exports every `@usearete/sdk/testing` helper and adds:

- `createAreteTestConfig(harness, props?)` — `<AreteProvider>` props that connect every client to a `createWebSocketHarness()` server, with no reconnects or batching delay.
- `createMutationResultFixture(phase, overrides?)` — a complete, consistent `useMutation()` result for any of the `MUTATION_PHASES`, for rendering phase-specific UI without a wallet.

```tsx
import { createAreteTestConfig, createWebSocketHarness } from '@usearete/react/testing';

const ws = createWebSocketHarness();
render(<AreteProvider {...createAreteTestConfig(ws)}><Dashboard /></AreteProvider>);
const subscription = await ws.waitForSubscription('OreRound/latest'); // flush renders between polls
ws.serve(subscription, [{ key: '1', data: round }]);
```

## Low-Level Hooks

The React SDK also exports:

- `useConnectionState`
- `useAreteContext`

These accept the same client lookup overrides when you need to target a non-default client.

## Relationship with `@usearete/sdk`

`@usearete/react` re-exports selected core APIs and types for convenience, but it is not a complete mirror of the core package.

If you need the full low-level surface, import directly from `@usearete/sdk`.

## License

MIT
