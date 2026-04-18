# Arete React SDK

React SDK for real-time Solana program data streaming from Arete.

Built on top of [`@usearete/sdk`](https://www.npmjs.com/package/@usearete/sdk), the pure TypeScript core SDK.

## Installation

```bash
npm install @usearete/react
```

> **Not using React?** Use [`@usearete/sdk`](../core/README.md) directly for Vue, Svelte, Node.js, or vanilla JavaScript.

## Usage

### Basic Setup

```tsx
import { AreteProvider, useArete, defineStack } from '@usearete/react';

const myStack = defineStack({
  // Your stack configuration
});

function App() {
  return (
    <AreteProvider config={{ /* your config */ }}>
      <MyComponent />
    </AreteProvider>
  );
}

function MyComponent() {
  const stack = useArete(myStack);
  // Use your stack
}
```

### Core Features

- **Real-time Data Streaming**: Subscribe to Solana program state changes
- **React Integration**: Hooks-based API for easy integration with React applications
- **State Management**: Built-in state management with Zustand
- **Type Safety**: Full TypeScript support with comprehensive type definitions
- **View Definitions**: Create state and list views for your data
- **Transaction Handling**: Define and execute transactions with hooks
- **Single Item Queries**: Type-safe single item fetching with `take: 1` or `useOne()`

### API

#### Providers

- `AreteProvider` - Root provider for Arete configuration

#### Hooks

- `useArete` - Main hook for accessing stack functionality
- `useAreteContext` - Access the runtime context directly

#### View Methods

- `.use()` - Subscribe to view data (returns `T[]` for lists, `T` for state)
- `.use({ take: 1 })` - Subscribe to single item with type narrowing (returns `T | undefined`)
- `.useOne()` - Convenience method for single item queries (returns `T | undefined`)

#### Factory Functions

- `defineStack` - Define a new stack configuration
- `createStateView` - Create a state view
- `createListView` - Create a list view
- `createRuntime` - Create a runtime instance

#### Utilities

- `ConnectionManager` - Manage WebSocket connections

## Relationship with @usearete/sdk

This package depends on and re-exports the core `@usearete/sdk` package. The core SDK provides:

- `Arete` - Main client class
- `ConnectionManager` - WebSocket connection handling
- `EntityStore` - State management
- AsyncIterable-based streaming APIs

The React SDK adds:

- `AreteProvider` - React context provider
- `useArete` - Main hook for accessing stacks
- `useConnectionState` - Connection monitoring hook
- `defineStack`, `createStateView`, `createListView` - React-friendly factories

If you need low-level access, you can import directly from the core:

```typescript
import { Arete, ConnectionManager } from '@usearete/react';
// or
import { Arete, ConnectionManager } from '@usearete/sdk';
```

## License

MIT

## Author

Arete Team
