# @usearete/adapter-kit

A [`WalletAdapter`](https://github.com/AreteA4/arete/blob/main/typescript/core/src/wallet/types.ts) for the Arete SDK backed by [`@solana/kit`](https://github.com/anza-xyz/kit).

The adapter owns v0 transaction construction, signing, submission, confirmation, and unsigned RPC inspection. It supports `@solana/kit` 2.3 and requires Node.js 20.18 or newer for Node consumers.

`transport: 'auto'` uses the connected Arete client's authenticated HTTP transaction transport per invocation. Standalone auto mode uses configured `rpc` and `rpcSubscriptions`; `transport: 'direct'` requires both. A custom `TransactionTransport` can be supplied instead. The Arete path performs no subscription calls and never falls back or resubmits.

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

const prepared = await client.programs.myProgram.instructions.buy.prepare({
  amount: 1_000_000n,
  maxSolCost: 100_000_000n,
  mint: 'So11111111111111111111111111111111111111112',
});

const inspection = await client.inspectOperation(prepared);
console.log(inspection.transaction.feeLamports, inspection.transaction.logs);

const receipt = await client.execute(prepared);
console.log(receipt.signature, receipt.slot);
```

## Signers

`signer` and `additionalSigners` must be `@solana/kit` `TransactionSigner` implementations. The primary signer is the default fee payer. Configured additional signers are published through `wallet.signerAddresses` so Arete can validate prepared operations before execution.

Per-send `signers`, `additionalSigners`, and `feePayer` values can satisfy transaction-specific signer requirements. A fee-payer override is itself a `TransactionSigner`. The adapter invokes signers only from `signAndSend`; `inspectTransaction` compiles an unsigned transaction and never calls a signer or submits to the network.

## Outcomes

Submission failures throw `KitTransactionExecutionError` with an Arete-compatible `outcome`:

- `not-submitted` for build, signer, and known preflight failures.
- `submitted-unknown` when confirmation fails and one signature-status query cannot prove the result.
- `chain-failed` when that status query reports an on-chain error.

The adapter never rebuilds, retries, or resubmits a transaction. Known signatures and landed slots are preserved on results and errors.
