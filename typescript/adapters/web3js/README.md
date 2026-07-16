# @usearete/adapter-web3js

An Arete [`WalletAdapter`](https://www.npmjs.com/package/@usearete/sdk) backed by [`@solana/web3.js`](https://github.com/solana-labs/solana-web3.js).

The adapter compiles static-key v0 transactions, signs once, submits once, and confirms without rebuilding or resubmitting. It also supports unsigned fee estimation and simulation for Arete operation inspection.

See the [transaction guide](../../../docs/src/content/docs/using-stacks/transactions.mdx)
for complete application setup, scopes, failure handling, and troubleshooting.

`transport: 'auto'` uses the connected Arete client's authenticated transaction transport per invocation. Standalone auto mode uses `connection` when provided. Use `transport: 'direct'` to require direct Solana RPC, or pass a `TransactionTransport` object. A resolved Arete operation never falls back to direct RPC after an error.

```ts
const wallet = createWalletAdapter({ signer, transport: 'auto' })
const client = await Arete.connect(stack, { wallet })
await client.transaction(instructions)
```

## Install

```bash
npm install @usearete/adapter-web3js @solana/web3.js @usearete/sdk
```

Node.js 18 or newer is supported. ESM and CommonJS entry points are included. Browser builds do not require an ambient `Buffer` global; the package imports its browser-compatible implementation explicitly.

## Node Signer

```ts
import { Connection, Keypair } from '@solana/web3.js';
import { Arete } from '@usearete/sdk';
import { createKeypairWalletAdapter } from '@usearete/adapter-web3js';
import { MY_STACK } from './generated/my-stack';

const connection = new Connection('https://api.devnet.solana.com', 'confirmed');
const keypair = Keypair.fromSecretKey(/* secret key bytes */);
const wallet = createKeypairWalletAdapter({ connection, keypair });
const client = await Arete.connect(MY_STACK, { wallet });

const { signature, slot } = await client.instructions.buy({
  amount: 1_000_000n,
  maxSolCost: 100_000_000n,
  mint: 'So11111111111111111111111111111111111111112',
});
```

Configured `additionalSigners` are used when their addresses are required and are all published through `wallet.signerAddresses`. Per-send signers can be supplied through `send.signers` or `send.additionalSigners`.

## Browser Wallets

`createWalletAdapter` accepts a wallet-adapter-style signer whose `signTransaction` method receives and returns a web3.js `VersionedTransaction`:

```ts
const wallet = createWalletAdapter({
  connection,
  signer: {
    publicKey,
    signTransaction,
    supportedTransactionVersions: walletAdapter.supportedTransactionVersions,
  },
});
```

The wallet must be connected, expose a non-null `PublicKey`, and support transaction version `0`. If `supportedTransactionVersions` is `null` or excludes `0`, the adapter rejects before prompting or sending. If that property is omitted, the supplied `signTransaction` implementation is responsible for accepting v0 transactions.

Raw Wallet Standard `solana:signTransaction` features operate on byte-array request and response objects; they do not directly satisfy this interface. Bridge those feature calls to web3.js `VersionedTransaction` serialization/deserialization, or use a wallet-adapter integration that already exposes `signTransaction`.

Address lookup tables are not currently accepted by this adapter. Transactions use a v0 message with static account keys.

## Inspection

Arete can inspect a prepared instruction or single-transaction operation without signing, submission, or a wallet prompt:

```ts
const prepared = await client.operations.deploy.prepare(params);
const inspection = await client.inspectOperation(prepared, {
  commitment: 'confirmed',
  minContextSlot,
});

console.log(inspection.transaction.feeLamports);
console.log(inspection.transaction.logs);
console.log(inspection.transaction.computeUnitsConsumed);
console.log(inspection.transaction.contextSlot);
console.log(inspection.programError);
```

Inspection compiles an unsigned v0 transaction, calls `getFeeForMessage`, and simulates with signature verification disabled. Arete core enriches simulation failures with the prepared operation's IDL error metadata. Multi-transaction flows are rejected by core rather than partially simulated.

## Failure Outcomes

Adapter failures expose the structured outcome consumed by `getTransactionFailureOutcome`:

- `not-submitted`: build, missing signer, wallet rejection, unsupported v0 wallet, or definite preflight rejection.
- `submitted-unknown`: a signed transaction may have been submitted, but one status lookup could not prove the requested commitment.
- `chain-failed`: confirmation or the status lookup reports an on-chain error.

Known signatures and landed slots are preserved. After an uncertain send or confirmation error, the adapter performs one signature-status lookup and never sends the transaction again.
