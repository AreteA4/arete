# ORE React Example

The smallest useful ORE path is one provider, one explicit `useArete(stack)` call, and two keyed state views. The Board is authoritative for the active round, so subscribe to it first and use its `roundId` to key the Round subscription:

```tsx
import { AreteProvider, useArete } from '@usearete/react';
import { ORE_STREAM_STACK } from './generated/ore-stack';

// Optional: without a key the app connects anonymously with shared
// low-trust limits; a key attributes usage to your own quota.
const publishableKey = import.meta.env.VITE_ARETE_PUBLISHABLE_KEY;

function CurrentRound() {
  const arete = useArete(ORE_STREAM_STACK);
  const board = arete.views.OreBoard.state.use({
    address: arete.addresses.board(),
  });
  const roundId = board.data?.state.roundId ?? undefined;
  const round = arete.views.OreRound.state.use(
    roundId === undefined ? undefined : { roundId },
  );

  return <p>Round {round.data?.id.roundId?.toString() ?? 'loading'}</p>;
}

export default function App() {
  return (
    <AreteProvider
      autoConnect
      stack={ORE_STREAM_STACK}
      auth={publishableKey ? { publishableKey } : undefined}
    >
      <CurrentRound />
    </AreteProvider>
  );
}
```

The stack remains on the provider so its endpoint overrides apply to matching calls. The provider cache shares one client and store for every equivalent stack/options pair, whether the stack is explicit or provider-default. Components repeat `useArete(ORE_STREAM_STACK)` intentionally so readers can see the data dependency without tracing provider configuration or ambient TypeScript registration.

## What to read

1. `src/App.tsx` wires `AreteProvider` to a Wallet Standard wallet through `useSolanaWalletAdapter()` from `@usearete/adapter-web3js/react`.
2. `src/components/RecentRounds.tsx` is the simplest consumer: one sorted list view with loading, error, empty, and populated states.
3. `src/components/OreDashboard.tsx` follows Board to keyed Round, subscribes to Treasury and the connected wallet's Miner, and rolls resource status up with `summarizeStatuses`.
4. `src/components/RewardsPanel.tsx` combines a wallet-keyed one-shot read with a composed claim transaction.
5. `src/components/DeploymentPanel.tsx` is the advanced path: a debounced quote, generated transaction mutation, and explicit stream reconciliation targets.
6. `src/components/ConnectionBadge.tsx` shows connection status, non-fatal `socketIssue` details, and manual retry.
7. `src/components/BlockGrid.tsx` and `StatsPanel.tsx` are presentational components fed by stream data.

The application keeps the complete ORE UI around that path: a responsive 5x5 board, live statistics, recent-round history, wallet positions, SOL reward claims, manual deployment quotes and writes, connection recovery, and light/dark themes.

## Data authority

- `OreBoard/state`, keyed by `{ address }`, is the authoritative singleton for the current `roundId`.
- `OreRound/state`, keyed by `{ roundId }`, supplies the board and statistics for that exact active round. Its `estimatedExpiresAtUnix` countdown is derived from the authoritative `OreBoard.endSlot`; the internal slot used for that calculation is not part of the emitted SDK shape.
- `OreRound/latest` is a sorted history view. `RecentRounds` uses it for the recent-round list, but it must not choose the active round because delivery timing can briefly differ from Board rollover timing.
- `OreTreasury/state`, keyed by `{ address }`, is the singleton treasury (motherlode).
- `OreMiner/state`, keyed by `{ authority }`, is disabled by passing `undefined` until a wallet connects. A miner deployment is displayed only when its snapshot `roundId` matches the Board round — that check is domain logic, not SDK plumbing.

Two guarantees make the app code short, and both are worth knowing:

1. **Keyed subscriptions and reads only expose data for the arguments you passed.** When a key or read argument changes, `data` is `undefined` (and `isLoading` true) until the fresh result arrives. Components never re-verify that returned data matches their inputs.
2. **Hooks are safe to call before the client connects.** Views, reads, and mutation hooks all exist immediately, so no conditional hook calls are needed. Each view hook result carries a `status` (`'disabled' | 'connecting' | 'subscribing' | 'ready' | 'error'`) for precise loading UI.

One constraint to know: streamed entities contain bigints, and React's dev-mode performance track `JSON.stringify`s changed props, which crashes on bigint arrays (fixed in React 19.3). The preferred fix is at the stack layer: ORE ships UI-denominated fields alongside raw values. Both Round and Miner expose `deployedPerSquareUi`, and Miner also exposes `totalDeployed`, so presentational components receive plain numbers while raw fields remain available for exact transaction logic.

`OreDashboard`, `DeploymentPanel`, `RewardsPanel`, `RecentRounds`, and `ConnectionBadge` each call `useArete(ORE_STREAM_STACK)`. `AreteProvider` caches one connected client per stack and endpoint, so those calls share a single socket and subscription store — components can grab `useArete` wherever they need it instead of receiving props from a top-level owner. A retry removes that shared client for every consumer, exposes one connecting window, and then installs one replacement client.

Components self-subscribe when they own an independent data need. `BlockGrid`
and `StatsPanel` stay presentational because they render the same active Round
already owned by `OreDashboard`; passing those values as props avoids duplicate
domain joins without introducing a data layer.

The app runs under React Strict Mode. The provider lifecycle is tested to leave
one active shared client after Strict Mode's development mount cycle and to
disconnect every superseded client.

The full dashboard remains mounted during initial loading, connection or query failure, and reconnection. `OreDashboard` aggregates the connection, Board, active Round, Treasury, and Miner with `summarizeStatuses`, including which committed streams are being resynchronized; `RecentRounds` separately renders loading, error, empty, and populated history states.

## Read and write boundaries

`safeToRawAmount` turns the deployment form's decimal input into a discriminated success/error result before a quote is requested. `arete.read.quoteManualDeployment.use(input, { debounceMs: 300 })` is then a one-shot, argument-keyed preview. Imperative and hook reads share the `arete.read` namespace. The quote reads raw Miner and Automation accounts and calculates principal allocation, exclusions, and checkpoint reserve. Its detailed `status` distinguishes disabled, connecting, loading, refreshing, ready, and error states; `isPending`, `isReady`, and `isEmpty` cover common UI branches. It is not a reservation or a guarantee that the round will remain current.

`deployWithCheckpoint.useMutation()` prepares the operation again before asking the wallet to sign. Preparation reads the raw Board account and rejects an explicit stale `roundId`. The app never retries or resubmits a transaction automatically.

The mutation's `phase` field is its discriminated status — `'preparing' | 'awaiting-wallet' | 'submitted' | 'confirmed' | 'reconciling' | 'reconciled' | 'confirmed-unreconciled' | ...` — use it for busy labels; use `displayError` and `reconciliationError` for messages. When `canRetryReconciliation` is true, `retryReconciliation()` repeats only watermark/refresh work and never rebuilds, signs, or resubmits the transaction.

After chain confirmation, generated program mutation hooks wait for the stack's processed-slot watermark by default. This proves that the stream has processed at least the confirmed receipt slot; it does not prove that every view changed. The `reconcile.refresh` option then refreshes targets — it accepts view hook objects, view/read hook results, or plain callbacks:

```tsx
reconcile: {
  refresh: [
    arete.views.OreBoard.state,   // refreshes the dashboard's active subscription
    arete.views.OreRound.state,
    arete.views.OreMiner.state,
    quoteRead,                    // re-runs the one-shot read
  ],
},
```

View hook refresh targets resolve through the subscription registry: they refresh that view's *active* subscriptions (optionally narrowed with a key, e.g. `arete.views.OreRound.state.refresh({ roundId })`) and are a no-op when nothing is subscribed. That is why `DeploymentPanel` needs no subscriptions of its own. An active view refresh resolves after its next complete snapshot is committed; registration, send, snapshot, and timeout failures reject. A reconciliation failure leaves the transaction confirmed and reports `confirmed-unreconciled`; it does not roll back or fail the landed transaction. Both transaction panels retry reconciliation before allowing another submission.

## Monorepo setup

This example links the local core, React, and web3.js adapter packages. Build them before installing the app in a clean checkout:

```bash
(cd typescript/core && npm ci && npm run build)
(cd typescript/react && npm ci && npm install ../core --no-save --package-lock=false && npm run build)
(cd typescript/adapters/web3js && npm ci && npm install ../../core --no-save --package-lock=false && npm run build)
```

## Run it

```bash
cd examples/ore-react
npm install
npm run dev
```

Open [localhost:5173](http://localhost:5173). Read-only viewing uses the hosted ORE stack at `wss://ore.stack.arete.run` without a wallet or credentials — the app mints anonymous sessions that share low-trust rate limits per IP/origin.

To attribute usage to your own quota instead of the shared anonymous limits, create `.env.local` with a publishable key from [the Arete dashboard](https://arete.run/dashboard):

```bash
VITE_ARETE_PUBLISHABLE_KEY=hspk_...
```

Optional endpoint and authentication overrides go in an untracked `.env.local`. See `.env.example` for the complete list.

## Generated SDK boundary

Everything under `src/generated/` is generated output. Do not edit it by hand.

The inputs are:

- `stacks/ore/src/stack.rs` and its IDLs, compiled to ProgramSpec, LiveSpec, and
  StackManifest artifacts under `stacks/ore/.arete/`
- `stacks/ore/extensions/`, which adds ORE-specific reads, math, addresses, and semantic transactions
- the monorepo CLI and TypeScript generator

Regenerate every ORE example SDK from the repository root:

```bash
bash scripts/generate-example-sdks.sh
```

The script first builds the standalone ORE crate against this checkout's local Arete source, then generates React, TypeScript, and Rust outputs. `sdk-manifest.json` records the canonical input, extension identities, artifact inventory, and generated content hash. It is committed alongside the generated code. `sdk-provenance.json` records compiler provenance separately and is uploaded by CI. Generation is deterministic and CI runs it twice before accepting the committed output.

After deploying ORE, compare the public deployment's AST and extension provenance without credentials:

```bash
bash scripts/check-ore-deployment-parity.sh
```

This parity check is intentionally post-deploy and is not part of normal pull-request CI.

## Tests

```bash
npm run lint
npm run typecheck
npm test
npm run build
npm run test:e2e
```

Playwright runs disconnected, read-only browser coverage. Any controlled mainnet write smoke test must remain manual and use a disposable, tightly funded wallet.

Component tests mock `useArete` results to exercise ORE-specific rendering and transaction policy deterministically. Provider, subscription, keyed-view, read, and reconciliation lifecycle behavior is covered in `typescript/react/src/*test.ts`; use that controlled-client pattern when an application needs an integration test across the real provider boundary. Browser tests deliberately use unreachable local endpoints, so they verify disconnected production UI without depending on hosted data or credentials.
