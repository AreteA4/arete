# @usearete/automations

Server-side automation of Arete Protocol steps on a user's behalf: when a
step needs the user's signature, a backend prepares it, checks it against a
signing policy and the user's consent, signs it with a delegated signer, and
submits it. Arete never holds a key; the signer runs in your backend.

> **Status:** in development and not yet published. The signing policy,
> signers and runtime are here; signed consent proofs, spending windows, the
> Postgres store and the Privy adapter follow.

## Use the least powerful mechanism

Reach for delegated signing only when nothing simpler works:

| Situation | Mechanism |
|---|---|
| The program lets anyone advance it | A keeper step |
| The money is committed now | Deposit straight into the pot or escrow |
| A fixed recurring payment from a wallet the user keeps using | An on-chain allowance |
| The user's signature is needed each time, and the wallet is where their funds already live | Delegated signing under consent (this package) |

## The runtime

```ts
import { createAutomations, definePolicy, memoryStore, rpcSimulator, scheduleSlot } from '@usearete/automations';

const automations = createAutomations({
  client,                       // a secret-key protocol client
  signer,                       // a ProtocolSigner
  policy: definePolicy({ ... }),
  simulator: rpcSimulator(RPC_URL),
  store: memoryStore(),         // tests and development only
  cluster: 'mainnet',
});

// A protocol event: read state, return the steps to run. Handlers never prepare.
automations.on('round.settled', async (event, ctx) => {
  const player = await lookUpPlayer(event);
  return [ctx.step('claimAndEnter', { wallet: player.wallet, instance: event.instanceId })];
});

// Webhooks: verified, accepted once, answered straight away.
export const POST = automations.webhookHandler({ secret: process.env.ARETE_WEBHOOK_SECRET! });

// Cron: your scheduler dispatches; the slot id makes a double fire a no-op.
await automations.dispatch('weeklyBuy', {}, { id: scheduleSlot('weekly-buy', 'week') });

automations.start();            // a long-running worker, or `await automations.drain()` per request
```

Each trigger becomes a **delivery job** that runs its handler. Each step
request the handler returns becomes a **step job** (`${deliveryId}:${step}:${wallet}`, plus
`:${instance}` for a step on an instance),
so rerunning a handler never doubles a step. A step job:

1. is skipped without an active consent grant (`no_consent`) or inside its
   `cooldown`;
2. is rejected when `allowedSteps` or prepare says no (a normal outcome);
3. is prepared with idempotency key `${jobId}:${generation}` in the
   negotiated format, checked against the intent, simulated and checked
   against the policy (`refused(<rule>)`);
4. re-checks consent, is signed, and the signed bytes are checked and saved
   before submit;
5. is tracked to `completed`, `chain_failed`, or `expired` (prepared once
   more with a new key). An intent in `submitted_unknown` is read again
   later and never resubmitted blind.

Only one step job per wallet runs at a time, from claim until it settles.
After a crash, a job that had signed reads its intent before doing anything
else. `onOutcome`, `onRefused` and `onRejected` report final states.

A store must pass `runStoreConformance` from `@usearete/automations/testing`.

## The signing policy

Every prepared transaction is checked before it is signed. A refusal names
the rule:

```ts
import { checkPrepared, checkTarget, definePolicy, rpcSimulator, summarizePrepared } from '@usearete/automations';

const policy = definePolicy({
  deployments: [DEPLOYMENT_ID],
  clusters: ['mainnet'],              // mainnet only when listed
  programs: [VAULT_PROGRAM_ID],       // optional; runtime programs are always allowed
  maxSolPerTransaction: '0.01',       // SOL, wrapped SOL and closed token accounts included
  maxTokenAmounts: { [USDC]: '5000000000' }, // base units; a mint without a cap may not leave
});

checkTarget(policy, DEPLOYMENT_ID, 'mainnet');           // before preparing; null when allowed

const summary = await summarizePrepared(prepared, wallet, rpcSimulator(RPC_URL));
const refusal = checkPrepared(policy, { deploymentId: DEPLOYMENT_ID, prepared, summary, simulates: true });
if (refusal) throw new Error(`${refusal.rule}: ${refusal.reason}`);
```

- Outflows are measured by simulating the exact bytes, per transaction. A
  transaction whose effect on the signer cannot be simulated is refused.
- Instructions that hand control of the signer's funds to someone else
  (token approvals, authority changes, System assignments, nonce, stake and
  loader authority changes) are always refused, top-level and inner.
- The rules match the Arete MCP server's local signer. Both run the shared
  vectors in [`test-vectors/signer-policy/`](../../test-vectors/signer-policy/).

What a wallet provider can enforce on its side is narrower; see
[docs/privy-policy-expressiveness.md](docs/privy-policy-expressiveness.md).

## Signers

A `ProtocolSigner` fills the wallet's signature slot of each prepared
transaction and nothing else. The package does not trust it:

- Before signing, `checkPreparedAgainstIntent` confirms the prepared bytes
  are the ones the intent committed to (`messageSha256`), in the negotiated
  format, with the wallet as a required signer.
- After signing, `checkSignedTransactions` re-hashes every message against
  the intent, confirms no other signature slot changed, and verifies the
  wallet's signature. A signer that adds a compute-budget instruction or
  re-encodes a transaction is refused as `signer_modified`.

`negotiateTransactionFormat` picks the first format, in the deployment's
order (`v1`, then `v0`), that the signer supports.

```ts
import { keypairSigner } from '@usearete/automations/keypair';

const signer = keypairSigner(JSON.parse(readFileSync(DEV_WALLET, 'utf8'))); // dev and tests only
```

## Development

```bash
npm install
npm test         # unit tests and the shared signer-policy vectors
npm run typecheck
npm run lint
npm run build
```
