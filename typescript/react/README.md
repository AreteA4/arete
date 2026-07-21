# Arete React SDK

React hooks and provider for consuming Arete stacks in React applications.

Built on top of [`@usearete/sdk`](https://www.npmjs.com/package/@usearete/sdk), the framework-agnostic TypeScript core SDK.

## Installation

```bash
npm install @usearete/react @usearete/sdk zod
```

`@usearete/react` installs `zustand` as a normal dependency, so do not install it separately. React remains an application dependency. Generated consumers install the core SDK and Zod because generated stack files import their types and schemas directly.

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
- `stackOptions` — default client lookup options (`url`, `httpUrl`, `transport`, `programs`) for the provider stack, including explicit `useArete(providerStack)` calls
- `autoConnect` — defaults to `true`; controls only whether the provider starts the initial connection
- `autoReconnect` — defaults to `true`; controls only recovery after an established connection is lost
- `wallet`
- `auth`
- `fetch`
- `validateFrames` — set `false` to suppress rejected-frame warnings; generated schemas still normalize and validate entities
- `onFrameValidationError` — structured callback for generated-schema rejections
- `reconnectIntervals`
- `maxReconnectAttempts`
- `maxEntriesPerView`
- `flushIntervalMs`

These are provider-wide defaults. Per-call overrides stay on the hook call and always win. Connection settings are captured when a shared client is created; changing provider props does not mutate that client. `wallet` updates are synchronized in place. For other changed settings, call `retry()` to replace the shared client using the latest provider configuration.

The lifecycle flags are intentionally independent. `autoConnect={false}` leaves initial connection ownership to the application. It does not disable automatic recovery for a connection the application establishes, but `canRetry` remains false because provider-owned replacement attempts would also stay disconnected; use the client lifecycle directly. `autoReconnect={false}` does not suppress the initial connection; it disables only automatic recovery after that connection is lost.

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

The mutation `phase` field is the discriminated status to branch on for busy labels (`'preparing'`, `'awaiting-wallet'`, `'submitted'`, `'confirmed'`, `'reconciling'`, `'reconciled'`, plus the failure outcomes); `status` remains `'pending'` until the mutation settles. Generated operation hooks publish each completed receipt and signature during `submitted`, before the final operation receipt resolves. `onConfirmed` runs immediately after chain confirmation; `onSuccess` runs only after successful reconciliation (or immediately when reconciliation is disabled/skipped). Reconciliation failure, including a refreshed snapshot timeout, is exposed through `reconciliationError`, does not call `onSuccess`, and does not reject the confirmed `submit()` result. The confirmed transaction remains landed. `retryReconciliation()` repeats only post-confirmation watermark/refresh work using the saved receipts and targets; it never rebuilds, signs, or submits. Convenience booleans (`isPreparing`, `isAwaitingWallet`, `isReconciling`, `canRetryReconciliation`) remain available.

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
