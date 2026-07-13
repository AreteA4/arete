# @usearete/adapter-web3js

A reference [`WalletAdapter`](../../core/src/wallet/types.ts) for the Arete SDK, backed by [`@solana/web3.js`](https://github.com/solana-labs/solana-web3.js).

The Arete core SDK is intentionally RPC-free: it only constructs `BuiltInstruction` objects. This adapter owns everything network-related: fetching a recent blockhash, compiling a v0 message, signing, sending, and confirming.

## Install

```bash
npm install @usearete/adapter-web3js @solana/web3.js @usearete/sdk
```

## Usage (Node / scripts / bots)

```ts
import { Arete } from '@usearete/sdk';
import { createKeypairWalletAdapter } from '@usearete/adapter-web3js';
import { Connection, Keypair } from '@solana/web3.js';
import { MY_STACK } from './generated/my-stack';

const connection = new Connection('https://api.devnet.solana.com', 'confirmed');
const keypair = Keypair.fromSecretKey(/* ... */);
const wallet = createKeypairWalletAdapter({ connection, keypair });

const client = await Arete.connect(MY_STACK, { wallet });

// Single flat params object: instruction args + any user-provided accounts.
const { signature } = await client.instructions.buy({
  amount: 1_000_000n,
  maxSolCost: 100_000_000n,
  mint: 'So11111111111111111111111111111111111111112',
});
```

## Usage (browser / wallet-standard)

```ts
import { createWalletAdapter } from '@usearete/adapter-web3js';

const wallet = createWalletAdapter({
  connection,
  signer: {
    publicKey: walletStandardAccount.publicKey,
    signTransaction: (tx) => walletStandardSigner.signTransaction(tx),
  },
});
```

## Batching

```ts
const buy = client.instructions.buy.build({ amount: 1000n, mint });
const stake = client.instructions.stake.build({ amount: 1000n });
const { signature } = await client.transaction([buy, stake]);
```
