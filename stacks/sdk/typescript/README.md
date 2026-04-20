# arete-stacks

[![npm](https://img.shields.io/npm/v/arete-stacks.svg)](https://www.npmjs.com/package/arete-stacks)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

Protocol stacks for Arete - ready-to-use Solana data streams.

## Installation

```bash
npm install @usearete/stacks
```

## Usage

### With @usearete/react

```tsx
import { useArete } from '@usearete/react';
import { PUMPFUNTOKEN_STACK } from '@usearete/stacks/pumpfun';

function TokenList() {
  const a4 = useArete(PUMPFUNTOKEN_STACK);
  const tokens = a4.views.pumpfunToken.list.useWatch();

  return (
    <ul>
      {tokens.map((token) => (
        <li key={token.id?.mint}>
          {token.info?.name} - {token.info?.symbol}
        </li>
      ))}
    </ul>
  );
}
```

### With @usearete/sdk (framework-agnostic)

```typescript
import { Arete } from '@usearete/sdk';
import { PUMPFUNTOKEN_STACK, PumpfunToken } from '@usearete/stacks/pumpfun';

const a4 = await Arete.connect('wss://mainnet.arete.xyz', {
  stack: PUMPFUNTOKEN_STACK,
});

// Stream all token updates
for await (const update of a4.views.pumpfunToken.list.watch()) {
  if (update.type === 'upsert') {
    console.log('Token updated:', update.data.info?.name);
  }
}
```

## Available Stacks

### PumpFun Token Stack

Real-time streaming data for PumpFun tokens on Solana.

```typescript
import { PUMPFUNTOKEN_STACK, PumpfunToken } from '@usearete/stacks/pumpfun';
```

**Entity: `PumpfunToken`**

| Field | Type | Description |
|-------|------|-------------|
| `id` | `PumpfunTokenId` | Token identifiers (mint, bonding curve) |
| `info` | `PumpfunTokenInfo` | Token metadata (name, symbol, URI) |
| `reserves` | `PumpfunTokenReserves` | Current reserve state and pricing |
| `trading` | `PumpfunTokenTrading` | Trading statistics and metrics |
| `events` | `PumpfunTokenEvents` | Recent buy/sell/create events |

## Peer Dependencies

This package requires one of:

- `@usearete/react` - For React applications
- `@usearete/sdk` - For framework-agnostic usage

```bash
# For React
npm install @usearete/react arete-stacks

# For vanilla TypeScript/JavaScript
npm install @usearete/sdk arete-stacks
```

## License

MIT
