# React View Clients

Use this reference for React applications. Keep generated stack imports at module scope so provider and client cache identity remains stable.

## Dependencies

```bash
npm install @usearete/react @usearete/sdk zod
```

React is an application peer dependency. Do not install `zustand` separately; `@usearete/react` carries it.

## Provider and Hook

```tsx
import { AreteProvider, useArete } from '@usearete/react';
import { MY_STACK } from './generated/my-stack';

function Positions({ owner }: { owner: string }) {
  const arete = useArete(MY_STACK);
  const position = arete.views.Position.state.use({ owner });

  if (arete.status === 'error') {
    return <button onClick={() => void arete.retry()}>Reconnect</button>;
  }
  if (position.status !== 'ready') return <p>Loading…</p>;
  if (!position.data) return <p>No position</p>;
  return <pre>{JSON.stringify(position.data, null, 2)}</pre>;
}

export function App() {
  const publishableKey = import.meta.env.VITE_ARETE_PUBLISHABLE_KEY;
  if (!publishableKey) throw new Error('VITE_ARETE_PUBLISHABLE_KEY is required');

  return (
    <AreteProvider stack={MY_STACK} auth={{ publishableKey }}>
      <Positions owner="…" />
    </AreteProvider>
  );
}
```

Use `.use()` for a live state/list result and `.useOne()` for the first list result. Pass `undefined` to a state hook when a dependent key is not ready; do not call hooks conditionally.

## Status and Lifecycle

Render from status fields rather than treating `data` truthiness as loading state. Account for loading, ready-empty, refreshing, connection error, and validation error states. Use `retry()` for an explicit provider-managed replacement attempt.

Repeated `useArete` calls share a client when stack and option identities match. Keep program objects and options stable with module constants or `useMemo`.

For single-stack applications, generated or `createAreteReact`-bound provider/hook pairs can remove repeated stack arguments. Follow the installed generated exports rather than assuming their names.

Enable the `arete/fluent-hooks` ESLint rule when the project uses fluent `.use()`, `.useOne()`, or `.useMutation()` hooks; generic hooks linting cannot identify every nested fluent call.

Current reference: `https://docs.arete.run/sdks/react/`.
