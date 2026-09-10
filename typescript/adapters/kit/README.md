# @usearete/adapter-kit

A [`WalletAdapter`](https://github.com/AreteA4/arete/blob/main/typescript/core/src/wallet/types.ts) for the Arete SDK backed by [`@solana/kit`](https://github.com/anza-xyz/kit).

The adapter owns legacy, v0 and transaction-V1 (SIMD-0385) construction, signing, submission, confirmation, and unsigned RPC inspection. It supports `@solana/kit` 8 — the first release that constructs V1 messages and carries the resource budget as a typed message config — and requires Node.js 20.18 or newer for Node consumers.

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

## Transaction versions and resources

An omitted `transactionVersion` still compiles v0. `'legacy'`, `0` and `1` are all built; anything else is refused before a signer is reached, never downgraded.

One typed `resources` contract covers every version: `computeUnitLimit`, `loadedAccountsDataSizeLimit`, `heapSize`, `priorityFeeLamports` (V1 only, total lamports) and `computeUnitPriceMicroLamports` (legacy/v0 only, per compute unit). Using a fee against the wrong version is an error, not a conversion. V1 carries the budget in the message config; legacy/v0 carry the same budget as the `ComputeBudget` instructions kit prepends — so a hand-built `ComputeBudget` instruction is rejected with a pointer to the option that replaces it. Address lookup tables are refused for every version.

```ts
const receipt = await client.execute(prepared, {
  send: {
    transactionVersion: 1,
    resources: { priorityFeeLamports: 5_000n },
  },
});
```

V1's `computeUnitLimit` and `loadedAccountsDataSizeLimit` are always resolved before signing, because an omitted V1 budget requests the *minimum* rather than a default (SIMD-0385) — a message without them could only fail on chain. An explicit value is used verbatim and never raised. An omitted one is measured: the adapter simulates a provisional unsigned message declaring the protocol maxima (1,400,000 CU and 64 MiB) with signature verification off, then derives the budget with 20% compute headroom and one 32 KiB page of loaded-data headroom, bounded by those same maxima. Only a metric the simulation never reports is refused, naming the option to pass instead.

`inspectTransaction` builds that same provisional message, never signs or submits, and returns its `transactionVersion` and applied `resources` alongside the fee and simulation metrics — so its output is what you pin the budgets with.

Final wire bytes are checked against 1232 bytes for legacy/v0 and 4096 for V1. The V1 structural caps (12 signatures, 64 accounts, 64 top-level instructions) are enforced by kit's own compiler, one step earlier and equally before signing.

## External wallets (Wallet Standard)

`createWalletStandardSigner` bridges a Wallet Standard account into the kit signer boundary as a byte-oriented handoff: the app selects the format, the wallet decodes the version out of the bytes, and Arete relays what comes back without converting versions.

```ts
import { createWalletStandardSigner } from '@usearete/adapter-kit';

const signer = createWalletStandardSigner({ wallet, account, chain: 'solana:devnet' });
const adapter = createWalletAdapter({ transport: 'auto', signer });
```

The account must advertise `solana:signTransaction`, and that feature must advertise the version being signed. `solana:signAndSendTransaction` is never read as evidence of sign-only support: it cannot hand signed bytes back for Arete to relay. A wallet that advertises only `legacy`/`0` refuses a V1 transaction before the user is prompted. Returned bytes are decoded with kit's V1-capable codec and rejected if the wallet changed the message or omitted the account's signature.

**External-wallet V1 is unverified against a released wallet.** As of Kit 8, the upstream Wallet Standard `signTransaction` feature still types `supportedTransactionVersions` as `legacy | 0`, and Anza's Phantom adapter advertises only `legacy`/`0`. The bridge is covered by controlled wallet fixtures and real codecs; a smoke test against an actual released wallet is tracked separately. Local keypair signers are verified end to end.

## Outcomes

Submission failures throw `KitTransactionExecutionError` with an Arete-compatible `outcome`:

- `not-submitted` for build, signer, and known preflight failures.
- `submitted-unknown` when confirmation fails and one signature-status query cannot prove the result.
- `chain-failed` when that status query reports an on-chain error.

The adapter never rebuilds, retries, or resubmits a transaction. On the Arete path, `confirmationTimeoutMs` is one deadline covering submission **and** confirmation, including whatever request is in flight, so a relay that stops answering yields `submitted-unknown` rather than hanging. The signature derived from the signed bytes is authoritative throughout: a relay reporting a different one is logged as a diagnostic, never polled for and never returned as the transaction submitted.
