# @usearete/automations

Server-side automation of Arete Protocol steps on a user's behalf: when a
step needs the user's signature, a backend prepares it, checks it against a
signing policy and the user's consent, signs it with a delegated signer, and
submits it. Arete never holds a key; the signer runs in your backend.

> **Status:** in development and not yet published. The signing policy,
> signers, runtime, consent, view sources and the Postgres store are here;
> the Privy adapter and a reference example follow.

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

## Consent

Nothing runs for a wallet without a grant the wallet signed:

```ts
const automations = createAutomations({ ...options, consent: { domain: 'app.example' } });

// 1. Your backend builds the message; it issues a single-use nonce.
const proof = await automations.consent.proofMessage({
  wallet,
  deploymentId: DCA_DEPLOYMENT,
  steps: ['buy'],
  limits: {
    perExecution: { SOL: '10000', [USDC]: '50000000' },          // lamports / base units, per run
    perWindow: { [USDC]: { amount: '200000000', window: '30d' } },
  },
  params: { amount: '50000000', weekday: 'fri' },                // the user's settings, signed too
  expiresAt: new Date('2027-01-01'),
});

// 2. The user's wallet signs `proof.bytes` (signMessage).
// 3. Your backend rebuilds the message from the same fields and checks the signature.
const { grant, warnings } = await automations.consent.grant({ ...fields, nonce: proof.nonce, issuedAt: proof.issuedAt, signature });

await automations.consent.revoke({ wallet });                    // no signature needed
```

- The message is human-readable and bound to your domain. Changing any
  field, raising a limit or changing a setting needs a new signature.
- An asset the step moves without a per-run limit is refused
  (`consent_limit`); going past a window is refused (`consent_window`). The
  window counts a step from the moment it is signed and stops counting it
  if its intent never lands. Only one step per wallet runs at a time, so
  concurrent workers cannot overspend a window.
- A grant scoped to an instance ends when the instance is terminal.
- `grant` warns when the signer reports no provider-side policy. The
  signature records what the user agreed to; it does not stop a compromised
  backend. The wallet provider's policy and the protocol's on-chain rules do.

Handlers fan out over the wallets that consented:

```ts
automations.on('weeklyBuy', async (_trigger, ctx) => {
  const grants = await ctx.consents.list({ step: 'buy' });
  return grants
    .filter((grant) => grant.params.weekday === today)
    .map((grant) => ctx.step('buy', { wallet: grant.wallet, args: { amount: grant.params.amount } }));
});
```

## View sources

A view source watches a live view and dispatches when a condition becomes
true, for example when the best lending rate moves to another market:

```ts
import { fromView, fromViewRows } from '@usearete/automations';

const bestRate = fromView(client.views['jupiter-lend-rates'].markets.list, {
  name: 'bestRateChanged',
  derive: (markets) => pickBest(markets, { minEdgeBps: 25 }),   // hysteresis lives in derive
  changed: (previous, next) => previous !== undefined && previous !== next,
  stableFor: '2m',                                               // a new value must hold 2 minutes
});

automations.on('bestRateChanged', async (trigger, ctx) => { /* fan out over consenting wallets */ });

automations.source(bestRate);                                    // long-running: subscribe
export async function GET() { await automations.poll(bestRate); } // serverless cron: one snapshot per call
```

`fromViewRows(view, { name, key, derive, filter })` keeps one signal per
row, for example "this wallet's position crossed below a health of 1.15".
`fromAppendView(view, { name })` dispatches one trigger per appended row and
resumes from its stored cursor.

Exactly once comes from the store: each key's last stable value is
advanced by compare-and-set and the delivery is accepted in the same step.
Replicas watching one view produce one delivery per transition; after a
restart, a change made while every replica was down fires once; A → B → A
→ B fires three times. A value that changed and changed back while nothing
was watching is not seen. A trigger means "go and look": handlers read live
state before returning steps.

## The Postgres store

```ts
import { postgresStore } from '@usearete/automations/postgres';

const store = postgresStore(pool);            // your pg Pool; opens no connections of its own
await store.migrate();                        // once, from a deploy step, with a role that may create tables
```

- Everything lives in one schema, `arete_automation` by default
  (`postgresStore(pool, { schema })`). Nothing outside it is created,
  altered or read.
- Migrations never run at startup. Run `store.migrate()` (transactional,
  behind an advisory lock, so concurrent callers run one at a time), or
  export the SQL for your own tool:
  `npx arete-automations migrations --out db/migrations`. A store whose
  database is behind refuses to run (`SchemaVersionError`).
- Use two roles: one that migrates, and a runtime role limited to rows.
  `npx arete-automations grants --role app_runtime` prints its grants.
- Pass a client instead of a pool to write inside your own transaction, for
  example a consent grant with the user record:
  `store.using(client).consent.put(grant)`.
- Jobs are claimed with `FOR UPDATE SKIP LOCKED`; a partial unique index
  keeps one active step per wallet. Tested on Postgres 15, 16 and 17.

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
