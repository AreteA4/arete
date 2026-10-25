# Automating protocol steps for users

`@usearete/automations` runs protocol steps that need a user's own signature
from the app's backend, when something happens, inside limits the user
signed, with a key the user delegated to the app. Arete never holds, stores
or proxies that key. Use it for products a protocol cannot offer alone:
automatic refunds when a raise fails, recurring buys, claim and re-enter,
moving a deposit to the best rate, repaying before liquidation.

Docs: [docs.arete.run/automations/overview](https://docs.arete.run/automations/overview/).
Resolve exact signatures from the package's types; this reference is the
workflow and the safety rules.

## Pick the least powerful mechanism

Delegated signing is the most powerful option and the most can go wrong with
it. Recommend it only when nothing simpler works:

| Situation | Use |
| --- | --- |
| Anyone may advance the step | A `keeper` step in the protocol (see [authoring](authoring.md)) |
| The money is committed now | A deposit straight into the pot or escrow |
| A fixed recurring payment from a wallet the user keeps using | An on-chain allowance (a subscription program, SPL `approve`, a Squads spending limit) |
| The user must sign each time, the action is more than a fixed transfer, and the wallet holds their funds | Automations |

If users would have to fund a wallet only so the app can automate it, they
have already committed the money: a direct deposit is simpler.

## Set up

Handlers live in a definition, apart from keys, store and URLs, so the same
code runs under `a4 dev` and in production. `a4 protocol init` already
scaffolds one: `automations/index.ts` (a daily recurring tip and a handler
for `tipJar.opened`), `[dev.automations]` in `arete.toml`, and
`@usearete/automations` and `tsx` in `package.json`. Edit that file rather
than starting another. In an older project:

```bash
npm install @usearete/automations @usearete/sdk
npm install --save-dev tsx
```

```ts
// automations.ts
import { defineAutomations } from "@usearete/automations";
import { TOKEN_RAISE_PROTOCOL } from "./generated/typescript/protocols/token-raise/token-raise.js";

export default defineAutomations(
  TOKEN_RAISE_PROTOCOL,
  (automations) => {
    automations.on("raise.failed", async (event, ctx) => { /* return ctx.step(...) per wallet */ });
  },
  { policy: { programs: [RAISE_PROGRAM_ID], maxSolPerTransaction: "0.001" } }, // what may be signed
);
```

```toml
# arete.toml
[dev.automations]
entry = "automations.ts"   # or: forward = "http://localhost:3000/api/arete-webhook"
```

`a4 dev up` then runs the definition against the dev environment (a fork of
mainnet): the client it generated (target `dev`), a signer for the dev
wallets, simulation on its local RPC, a store under
`.arete/dev/automations/`, and the environment's events as signed webhook
deliveries (dev environments send no webhooks). It restarts on save.
`a4 dev consent <wallet> --steps refund` opts a dev wallet in; then drive the
protocol with `a4 dev step` and `a4 dev warp`. With `forward`, it delivers
the signed events to the app's own webhook route instead, signed with
`.arete/dev/automations/webhook-secret`.

Arete runs dev environments and mainnet only. Never set up devnet or
testnet for automations.

In production the backend builds the runtime and uses the definition:

```ts
import { createProtocolClient } from "@usearete/sdk";
import { createAutomations, definePolicy, rpcSimulator } from "@usearete/automations";
import { postgresStore } from "@usearete/automations/postgres";
import definition from "./automations";

const client = createProtocolClient(TOKEN_RAISE_PROTOCOL, {
  auth: { secretKey: process.env.ARETE_SECRET_KEY! }, // backend only
});

const automations = createAutomations({
  client,
  signer,                                  // a wallet provider (Privy); keypairSigner only for tests
  policy: definePolicy({
    ...definition.policy!,
    deployments: [client.deploymentId],    // required
    clusters: ["mainnet"],                 // mainnet only when listed
  }),
  simulator: rpcSimulator(process.env.RPC_URL!),
  store: postgresStore(pool),
  consent: { domain: "app.example" },
  onOutcome: (outcome) => notifyUser(outcome),
});
await automations.use(definition);
```

The target comes from the client: `dev` for the client `a4 dev` generated,
`mainnet` after `a4 install --locked` restores the published one. The policy
allows `dev` by default and mainnet only when listed. It is a library in the
app's backend, next to the app's own database. It needs a secret-key client,
so it never runs in a browser.

## Handlers and fan-out

A handler receives a trigger and returns step requests, usually one per
wallet that opted in. It reads whatever it needs to decide but never
prepares, signs or submits; the library does that after its checks.

```ts
automations.on("raise.failed", async (event, ctx) => {
  const grants = await ctx.consents.list({ step: "refund", instanceId: event.instanceId });
  return grants.map((grant) =>
    ctx.step("refund", { wallet: grant.wallet, instance: event.instanceId }),
  );
});
```

- `ctx.consents.list({ step, instanceId? })` returns active grants with each
  user's own `params` (settings). With `instanceId`, a grant scoped to that
  instance wins over one for every instance.
- `ctx.step(step, { wallet, instance?, args? }, { cooldown? })`; event and
  step names are typed from the generated client.
- Each request becomes its own job keyed by trigger, step, wallet and
  instance: a handler that reruns never doubles a step, and one user's
  refusal does not affect the others.
- A trigger means "go and look", not the truth. Read current state (for
  example `client.participants.get(...)`) instead of trusting a payload's
  amounts. Every step is checked again before signing.

## Triggers

| Trigger | API |
| --- | --- |
| A protocol event (`emit` in a reaction), production | `automations.webhookHandler({ secret })`; Express: `expressWebhookHandler` behind `express.raw({ type: "application/json" })` |
| A protocol event under `a4 dev` | forwarded by `a4 dev up` as signed deliveries; nothing to set up |
| One instance's events | `automations.follow(instanceId)` |
| A schedule | `automations.schedule("weeklyBuy", { every: "week" })` in the definition, plus `on("weeklyBuy", ...)` |
| A whole live view (which market is best) | `fromView(view, { name, key, derive, changed, stableFor })` |
| One signal per row (this position's health) | `fromViewRows(view, { name, key, derive, filter, ... })` |
| Each row appended to an append view | `fromAppendView(view, { name, ... })` |

- Triggers dedupe on their id: webhook retries, a double cron and restarted
  streams are harmless.
- `every` is a UTC period (`minute`, `hour`, `day`, `week`, `month`) or an
  interval of at least a second (`"15m"`). `automations.tick()` dispatches
  each due slot once, however many replicas or crons call it; the host
  adapters call it for you. Under `a4 dev up` schedules follow the dev
  environment's clock, so `a4 dev warp` runs them. For per-user schedules,
  run daily and filter grants on their `params`. A schedule's slot already
  runs once per period: do not add a `cooldown` to repeat that, it only
  stops a wallet with several grants and breaks warping.
- Outside a definition, `dispatch(name, payload, { id: scheduleSlot(...) })`
  from your own cron does the same for one slot.
- Run a view source with `automations.source(src)` on a long-running server
  (add `{ exclusive: true }` to run it on one replica). On serverless,
  `use(definition, { sources: "poll" })` registers list sources for
  `tick()` to poll; append sources need a long-running server.
- `stableFor` makes a value hold before it counts (ignores an oracle blip);
  put hysteresis such as a minimum edge in `derive`. A step's `cooldown`
  limits how often a user is acted on. View-driven automations usually want
  both.
- Views hold current state: a value that changed and changed back while
  nothing watched is not seen. Pass `observedAt` when replicas can see a
  view at different times.

## Consent and limits

Nothing runs for a wallet until its owner signs a consent message
(`arete.automation-consent/v1`) that states the steps, per-asset limits,
settings and expiry.

```ts
const fields = {
  wallet,
  deploymentId: client.deploymentId,
  steps: ["buy"],
  limits: {
    perExecution: { [SOL]: "10000", [USDC_MINT]: "50000000" },
    perWindow: { [USDC_MINT]: { amount: "200000000", window: "30d" } },
  },
  params: { amount: "50000000", weekday: "fri" },
  expiresAt: new Date("2027-01-01"),
};
const proof = await automations.consent.proofMessage(fields);
// the user's wallet signs proof.bytes with signMessage, in the app
const { grant, warnings } = await automations.consent.grant({
  ...fields, nonce: proof.nonce, issuedAt: proof.issuedAt, signature,
});
await automations.consent.revoke({ wallet, steps: ["buy"] }); // or all steps
```

- Limits are per asset: `SOL` in lamports, a mint in base units. An asset
  without a `perExecution` limit may not leave the wallet at all, so include
  `SOL` for fees. `perWindow` caps all steps together over a rolling window.
- `grant` rebuilds the message from the fields and verifies the signature;
  the nonce works once and expires after ten minutes. Raising a limit or
  changing a setting needs a new signature.
- Treat `warnings` (no delegated access, no provider-side policy) as setup
  the user still has to finish.
- Revoking needs no signature, so expose it only to the signed-in user.
- Present it honestly: the product's words, human amounts next to base
  units, its own screen and signature (never pre-ticked or bundled with
  sign-in), an end date that fits the feature, an easy off switch, and a
  notification for every outcome.

## What every step passes

1. Active consent for the step, and not inside its cooldown (else `skipped`).
2. The protocol allows the step for this wallet now (else `rejected`).
3. The prepared transactions match the intent's committed message hashes.
4. Simulation, then the signing policy: allowed deployment, cluster and
   programs; nothing hands control of the wallet's funds to anyone (approvals,
   authority changes, reassignments, nonce, stake and upgrade authorities,
   including inside CPIs); outflows within `maxSolPerTransaction`,
   `maxTokenAmounts` and the user's limits. Unsimulatable means refused.
5. After signing: only the wallet's signature changed, and it verifies.

The policy rules are the MCP signer policy's (see [agents](agents.md)), tested
against the same shared cases.

## Signers

- `keypairSigner(secretKey)` from `/keypair`: dev wallets and tests only.
- `privySigner({ privy, walletIdFor, authorizationPrivateKey })` from
  `/privy`: Privy server wallets where the user added the app's authorization
  key. It signs (never sends) `v0` transactions; a Privy denial ends the step
  `refused` with rule `provider_policy`.
- `privyPolicyFor(policy, grant)` builds the provider-side policy to attach to
  the user's wallet. Privy sees only top-level instructions, so for a
  protocol step it can bound programs and expiry, not amounts; show its
  `unenforced` list to the user rather than implying more protection.
- A custom signer (KMS, Turnkey) implements `ProtocolSigner` and throws
  `SignerRefusal` when the provider declines, so the step is refused rather
  than retried.

## Production

- `postgresStore(pool)` keeps everything in its own schema
  (`arete_automation` by default); one schema per deployment. Migrations
  never run at startup: call `store.migrate()` from a deploy step, or export
  them with `npx @usearete/automations migrations --out db/migrations`. Run with
  a role limited to rows (`npx @usearete/automations grants --role <role>`).
- `memoryStore()` is for tests. A custom store must pass
  `runStoreConformance` from `/testing`.
- Workers: `automations.start({ concurrency })` and `await automations.stop()`
  on a server; on serverless, `automations.drain({ maxJobs })` after
  accepting a trigger and from a scheduled function. Several replicas are
  safe: one claim per job, one active step per wallet.
- Outcomes: `completed`, `chain_failed`, `expired`, `skipped`, `rejected`
  (the protocol said no; normal), `refused` (policy, limit, exact bytes or
  provider), `failed` (kept failing before signing). Each has `code` and
  `reason`.
- After signing, the signed bytes are saved before submitting. Recovery
  reads the intent first and never re-signs or blindly resubmits; while an
  outcome is unknown, that wallet's other steps wait.

## Deploy to a host

Three layers keep the definition host-agnostic:

| Layer | File | Who writes it |
| --- | --- | --- |
| Definition (handlers, schedules, policy) | `automations/index.ts` | init, then you |
| Runtime: `runtime(env)` returns client, signer, store, simulator, policy | `automations/runtime.ts` | `npx @usearete/automations add` |
| Host entry and config | `worker.ts` + `wrangler.jsonc`, `api/arete/[route].ts` + `vercel.json`, or `server.ts` + `Dockerfile` | `npx @usearete/automations add` |

```bash
npx @usearete/automations add cloudflare          # or vercel, node; --signer privy for Privy wallets
```

It never overwrites a file and prints the remaining steps in order: install
the dependencies it lists, `a4 protocol deploy` if the release is not
deployed, `a4 install --locked` (bundle the published client, not the one
`a4 dev` generated), `DATABASE_URL=... npx @usearete/automations migrate`, set
the secrets in the host, deploy with the host's own CLI, then register the
webhook and store its secret:

```bash
a4 protocol webhook add --url https://<worker>.workers.dev/arete/webhook --secret-only \
  | npx wrangler secret put ARETE_WEBHOOK_SECRET
a4 protocol webhook test <id>                 # a signed test delivery; ids from `a4 protocol webhook list`
```

- Every adapter serves `POST …/webhook` (verified against
  `ARETE_WEBHOOK_SECRET` before any runtime is built), `GET|POST …/tick`
  (`Authorization: Bearer <CRON_SECRET>`) and `GET …/health`.
- Cloudflare builds a runtime per invocation and closes it after;
  `wrangler.jsonc` has a one-minute cron for `tick`. Vercel reuses one per
  instance; `vercel.json` has a cron for `/api/arete/tick`, and environment
  variables reach only new deployments, so deploy again after adding the
  webhook secret. Node runs workers, live sources (append sources included)
  and its own tick; no cron needed.
- Set `ARETE_CLUSTER=mainnet` in the host: a build whose client still
  targets the dev environment then fails every request and says to run
  `a4 install --locked`.
- `npx @usearete/automations manifest` prints the definition's events,
  schedules and sources; `a4 protocol webhook add` subscribes to the
  build's events by default (`--events` narrows it).
- For another framework (Hono, Express, Bun, Deno), call
  `createAutomations(await runtime(env))`, then `automations.use(definition)`,
  and route requests to `webhookHandler` and `tick` yourself.

Deploying the protocol, deploying to a host and registering a webhook are
each external changes: do them only when the user asked.

## Safety model

The protocol's on-chain rules and the wallet provider's policy hold even if
the backend is compromised. The signing policy, consent limits, spending
windows and exact-bytes checks run in the backend and do not. Say so when
advising on design: keep the authorization key in a secrets manager, keep the
program allow-list tight, and prefer protocols whose automated steps only move
funds between the user's own positions.

## Verify

- The handler returns steps only for wallets with a grant for that step, and
  reads current state instead of trusting the trigger's amounts.
- Schedules are declared with `automations.schedule` (or dispatched with a
  `scheduleSlot` id); view sources use `stableFor`, and view-driven steps
  that repeat use a `cooldown`.
- The consent's limits include `SOL` for fees and every asset the step moves,
  and the UI shows human amounts and a revoke control.
- The policy lists only the deployment, cluster and programs the steps need;
  mainnet only when the user asked.
- Handlers are in a `defineAutomations` module run with `a4 dev up`
  (`[dev.automations]`) before production; tests use `memoryStore()`, and
  every outcome reaches `onOutcome`.
- Before a host deploy: `a4 install --locked` ran, the host sets
  `ARETE_CLUSTER=mainnet`, migrations ran from a deploy step, and
  `GET …/health` reports the expected deployment and cluster.
