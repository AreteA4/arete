# @usearete/adapter-kit

A reference [`WalletAdapter`](../../core/src/wallet/types.ts) for the Arete SDK, backed by [`@solana/kit`](https://github.com/anza-xyz/kit) (the functional successor to `@solana/web3.js`).

The Arete core SDK is intentionally RPC-free: it only constructs `BuiltInstruction` objects. This adapter owns blockhash fetching, transaction message construction, signing, sending, and confirmation.

## Install

```bash
npm install @usearete/adapter-kit @solana/kit @usearete/sdk
```

## Usage

```ts
import { Arete } from '@usearete/sdk';
import { createWalletAdapter } from '@usearete/adapter-kit';
import {
  createSolanaRpc,
  createSolanaRpcSubscriptions,
  createKeyPairSignerFromBytes,
} from '@solana/kit';
import { MY_STACK } from './generated/my-stack';

const rpc = createSolanaRpc('https://api.devnet.solana.com');
const rpcSubscriptions = createSolanaRpcSubscriptions('wss://api.devnet.solana.com');
const signer = await createKeyPairSignerFromBytes(secretKeyBytes);

const wallet = createWalletAdapter({ rpc, rpcSubscriptions, signer });
const client = await Arete.connect(MY_STACK, { wallet });

const { signature } = await client.instructions.buy({
  amount: 1_000_000n,
  maxSolCost: 100_000_000n,
  mint: 'So11111111111111111111111111111111111111112',
});
```
