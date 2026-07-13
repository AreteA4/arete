# Arete React SDK

React hooks and provider for consuming Arete stacks in React applications.

Built on top of [`@usearete/sdk`](https://www.npmjs.com/package/@usearete/sdk), the framework-agnostic TypeScript core SDK.

## Installation

```bash
npm install @usearete/react react zustand
```

> Not using React? Use [`@usearete/sdk`](../core/README.md) directly.

## Quick Start

```tsx
import { AreteProvider, useArete } from '@usearete/react';
import { ORE_STREAM_STACK } from './generated/ore-stack';

function Dashboard() {
  const { views, isLoading, error } = useArete(ORE_STREAM_STACK, {
    url: 'ws://localhost:8877',
    httpUrl: 'http://localhost:8877',
  });

  const { data: latestRound } = views.OreRound.latest.useOne();

  if (isLoading) {
    return <div>Connecting...</div>;
  }

  if (error) {
    return <div>{error.message}</div>;
  }

  return <pre>{JSON.stringify(latestRound, null, 2)}</pre>;
}

export default function App() {
  return (
    <AreteProvider
      autoConnect={true}
      auth={{
        // publishableKey: 'hspk_...',
      }}
    >
      <Dashboard />
    </AreteProvider>
  );
}
```

## Provider

`AreteProvider` manages connected clients for descendant hooks.

Supported props:

- `autoConnect`
- `wallet`
- `auth`
- `fetch`
- `validateFrames`
- `reconnectIntervals`
- `maxReconnectAttempts`
- `maxEntriesPerView`
- `flushIntervalMs`

These are provider-wide defaults. Endpoint overrides stay on the hook call.

## `useArete(stack, options?)`

`useArete` returns the connected React surface for a stack:

- `views`
- `programs`
- `queries`
- `chain`
- `client`
- `connectionState`
- `isConnected`
- `isLoading`
- `error`

Supported hook options:

- `url`
- `httpUrl`
- `transport`
- `programs`

Notes:

- `transport: 'http'` disables streaming view subscriptions, but HTTP-backed surfaces like `queries`, `chain`, and program reads still work.
- If you pass attached `programs`, keep that object stable with a module constant or `useMemo` so React does not create a fresh client on every render.

## Views

View hooks return a `ViewHookResult<T>` object:

```ts
type ViewHookResult<T> = {
  data: T | undefined;
  isLoading: boolean;
  error?: Error;
  refresh: () => void;
};
```

List views support:

- `.use()`
- `.use({ take: 1 })`
- `.useOne()`

State views support:

- `.use(key)`

## Programs, Queries, and Chain

`useArete` mirrors the connected core client surface:

- `programs.<program>.raw.<instruction>` for raw typed instructions
- `programs.<program>.plan` / `programs.<program>.instructions` for semantic instructions
- `programs.<program>.accounts` and `programs.<program>.queries` for HTTP reads
- `queries` for stack-level HTTP queries
- `chain` for chain helpers

Raw instruction hooks preserve `.execute`, `.build`, and `.useMutation()`.

Semantic instruction hooks preserve:

- `.execute`
- `.send`
- `.resolve`
- `.plan`
- `.build`
- `.stage`
- `.useMutation()`

## Low-Level Hooks

The React SDK also exports:

- `useConnectionState`
- `useView`
- `useEntity`
- `useAreteContext`

These accept the same client lookup overrides when you need to target a non-default client.

## Relationship with `@usearete/sdk`

`@usearete/react` re-exports selected core APIs and types for convenience, but it is not a complete mirror of the core package.

If you need the full low-level surface, import directly from `@usearete/sdk`.

## License

MIT
