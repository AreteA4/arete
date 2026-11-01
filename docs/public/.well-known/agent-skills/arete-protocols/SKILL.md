---
name: arete-protocols
description: Author Arete Protocols with defineProtocol (machines, guards, instruction templates, reviewed handlers, reactions, fees), scaffold them with a4 protocol init, iterate with a4 dev on an Arete cloud fork (generated client, a4 stream --dev), deploy privately with a4 protocol deploy and install the result with a4 install protocol, use installed protocols from TypeScript, Rust or Python apps (instances, allowedSteps, prepare, sign, submit, metadata, events, webhooks, Squads multisig setup), and act on a protocol as an agent with a wallet (a4 protocol actions, the MCP protocol tools, an opt-in signer policy), and automate user-signed steps from an app backend with @usearete/automations (triggers, schedules, consent, spending limits, signers, deploying to Cloudflare, Vercel or a Node host). Use when composing existing Solana programs into a hosted multi-step flow or transaction workflow, or when an app calls a protocol. Do not use for a single program call (arete-programs), stack views (arete-streams) or stack authoring (arete-stack-authoring).
metadata:
  version: "1.12.0"
  min-cli: ">=0.51.0"
---

# Arete Protocols

An Arete Protocol composes existing, audited programs into a hosted flow that
apps call as an API: instances and participants with state machines, guards
that explain rejections, steps that build exact transactions users sign
themselves, and reactions (timers, keeper steps, Squads proposals, emitted
events). There is no custom on-chain code. Users sign the program transactions with
their own wallets; platform fees can apply.

Two jobs, two references:

- **Author** a protocol: [references/authoring.md](references/authoring.md).
- **Use** an installed protocol from an app: [references/using.md](references/using.md).

Start, iterate locally and deploy your own protocol with
[references/local-dev.md](references/local-dev.md), including CLI profiles,
environment selection, execution checks and what persists across restores.

For transaction fees returned by prepare, read [fees](references/authoring.md#fees).
For completed transactions, see [execution receipts](references/using.md#execution-receipts).

Act on a protocol as an agent with a wallet (what can it do, prepare, check,
sign, submit, and the optional signer):
[references/agents.md](references/agents.md).

Run steps for users from the app's backend when something happens, inside
limits each user signed (refunds, recurring buys, claim and re-enter,
rebalancing): [references/automations.md](references/automations.md).

Before handing a release to a reviewer, walk
[references/review-checklist.md](references/review-checklist.md).

Expressions (functions, operators, names in scope, what is not supported):
[references/expressions.md](references/expressions.md). Rejections with their
cause and fix: [references/troubleshooting.md](references/troubleshooting.md).
Builtin programs and glue: [references/builtins.md](references/builtins.md).
Design patterns (allow-lists, choosing a venue, per-user accounts, state-only
steps, a PDA the IDL lacks, dev-only app features, dev and production builds):
[references/patterns.md](references/patterns.md).
Check a spec against the server before deploying: `npx arete-protocol check`
with a secret key in `ARETE_API_KEY`, or `a4 protocol deploy --dry-run`.

## Start a Project

```bash
a4 protocol init my-protocol && cd my-protocol
a4 init -y
npm install
npx arete-protocol check
a4 dev up
```

`a4 protocol init` scaffolds a template-only tip jar with the System Program
installed as `@programs/system` from the registry, plus `arete.toml`,
`arete.lock`, `package.json` and `tsconfig.json`, and `automations/index.ts`
(wired in as `[dev.automations]`, so `a4 dev up` runs it). If the registry
install fails, init stops with the error and never substitutes a local IDL.
`a4 init -y` then adds the agent instructions, skills and MCP config. Start
from it instead of writing project files by hand.

## Ground Every Program Fact

Never guess program ids, instruction names, argument units, account roles or
PDA seeds. Resolve them from the exact generated program surface first (the
`arete` and `arete-programs` skills):

```bash
a4 explore catalog --query "<intent>" --kind program --json
a4 explore program <program-ref> --json
a4 install program <program-ref> --ts                         # from the registry
a4 install program ./idl/my_program.json --as my-program --ts # an unpublished program, from its IDL
```

Then install, import, `ix`: `a4 install program --ts` generates the program
module, records its ProgramSpec hash in `arete.lock` and maps
`@programs/<name>` in `tsconfig.json`. `protocol.ts` imports the module and
lists it in `programs`; never write a program id, a ProgramSpec hash or a
stack source by hand. `ix(program, "<instruction>", { args, accounts })` makes
an unknown instruction, argument or account name a TypeScript error, and
`arete-protocol check`/`build` fail when a module no longer matches
`arete.lock` (run `a4 install` or `a4 update program <name>`, then rebuild).

## Prefer Templates over Handler Code

Write each step's `build` as a declarative instruction template. Reach for a
handler only when a template cannot express the step (looping over accounts,
branching on decoded state, rebuilding remaining accounts from a stored
message), and say why in the step's description. Handlers run only after a
human review signs the release; they read data only through the typed `ctx`
and have no network, filesystem, clock, randomness, environment or secrets.

## Check and Build Locally

```bash
npx arete-protocol check protocol.ts
npx arete-protocol build protocol.ts --out .arete/protocol
```

`check` validates the spec, reports unknown expression references with
positions and flags forbidden APIs in handler code. `build` writes the spec,
the handler bundle (only if a step has a handler), the generated stack
source, the release and a build receipt. Two clean builds must produce
identical bytes.

## Explain the Model Accurately

- **Recorded states** move only when a step confirms. **Derived states** are
  re-evaluated from chain reads and time after every observation.
- **Guards** are UX: they explain why a step is not allowed. Programs enforce
  money; anyone can bypass Arete and call the programs directly, and the
  protocol stack labels that activity `direct`.
- **Signer classes**: `wallet` (participant or a named role wallet held by
  whoever performs the role), `ephemeral`, `attestation` (Arete's memo
  co-signer), `keeper` (only provably permissionless steps) and
  `squadsProposer` (proposes, never votes or executes).
- **Platform keys**: `deployment.id`, `deployment.attestation` and
  `deployment.squadsProposer` name the deployment and the keys it holds, in
  every expression including the create step.
- **Trust tier**: `reviewed` (a reviewer signed the release) or
  `unreviewed` (template-only, private deployments only). Construction and
  simulation do not prove economic intent.

## Run Locally, Then Deploy

```bash
a4 dev up                 # cloud fork, local RPC/UI; watch + redeploy
a4 dev wallet creator     # local signing key, funded on the dev fork
a4 dev step <step> --args '<json>' --wallet <name>
a4 dev warp +2h
a4 stream --dev           # the protocol's Instances, Activity and Participants; --tui to browse
a4 protocol deploy --dry-run
```

- `a4 dev up` regenerates the client in
  `generated/<lang>/protocols/<alias>/` and serves an edge on loopback. Run the
  app with `a4 dev run -- <command>` (`ARETE_ENVIRONMENT=dev`): every installed
  SDK then uses the environment, with no credential in the app. Unset, the
  same app is production. See
  [local-dev.md](references/local-dev.md#the-apps-client-one-switch).
- The environment persists across CLI restarts. `a4 dev down` stops local
  forwarding; `a4 dev sleep` sleeps the environment. Reset, restore and delete
  alter cloud state: explain the effect and require the user's authorization.
- Keep `.arete/dev/` (session file, dev wallet keypairs) out of git, browser
  bundles, logs and chat.
- Before shipping: `a4 dev promote --check` lists what exists only in the
  environment, with the command that publishes each.
- `a4 protocol deploy` builds first (`--no-build` refuses a stale build),
  pushes programs installed from a local IDL as your private programs, and
  activates. Deploying is an external mutation: only when the user asks.
- **Template-only** protocols activate privately at trust tier `unreviewed`.
- **Handler** protocols stop at `review-required`: they need Arete's review
  before they can be activated. Share the printed release hash; owners never
  activate handler code.
- **Public** deployments are not self-serve. Self-serve is always private.
- Activation publishes an owner-private package (`registryPackage`:
  `published`, `existing` or `skipped`; on `skipped` with
  `package-version-immutable`, bump the spec `version`). Deploy reinstalls
  the project's own client from it; other projects of the account run
  `a4 install protocol <slug> --ts`. In CI, `a4 install --check` fails while
  a client is still a local build; `a4 install --locked` restores the
  published one.

See [references/local-dev.md](references/local-dev.md).

## Using an Installed Protocol

```bash
a4 explore protocol <protocol-ref> --json
a4 install protocol <protocol-ref> --ts   # or --rust / --python
```

A public protocol needs no deployment in your account: the app calls the
shared deployment with its own key. Browser code uses a publishable key and
can call only `public` steps; `secret` steps (by default the create step and
role steps) need a backend with a secret key. The generated clients enforce
this: TypeScript by type, Rust and Python with separate publishable and
secret client classes. A TypeScript backend that takes its key from
`ARETE_API_KEY` passes `keyClass: "secret"`, or the secret steps are
missing from its type.

The client never signs: `prepare` returns the step's transactions, the
user's wallet signs them unchanged (ask for `transactionFormat: "v0"`;
wallets don't sign Transaction V1 yet), `submit` relays them, and the app
polls the intent until `completed`, `chain_failed`, `expired` or `rejected`.
Never resend after `submitted_unknown`.

Protocols with admin steps propose them to the team's Squads multisig through
a proposer key that belongs to the deploying account. The multisig is a
resource in the deployment config, and production deployments are refused
(`protocol-squads-setup-required`) until it is attached to the account and
active (the proposer is an Initiate-only member). Set it up in this order:

1. Declare the multisig as a resource (`resources.<name>.multisig`), never
   read from the instance or built by a create step.
2. Ask the user whether they already have a Squads multisig and how its
   members sign (hardware wallet, browser wallet, or a keypair file).
3. Send them to https://app.arete.run/dashboard/squads (any browser wallet,
   Ledger included; it attaches an existing multisig or creates one), or, for
   an existing multisig, the Squads app: Members → Add member → paste the
   proposer → permission Initiate only → approve and execute.
4. Fund the proposer with a little SOL (it pays each proposal's fee and
   rent), put the active multisig's address in the deployment config, and
   deploy again.

Arete builds no Squads transactions. Only when the user wants it in code (a
script or a hot-wallet member), use the Arete setup helpers of the Squads v4
program SDK (`a4 install program squads-v4`); the member's wallet signs.
Check with `a4 squads status` (or the `squads_status` MCP tool) and handle
the `arete.squads.proposerMissing` event. See
[references/using.md](references/using.md).

## Agents with a Wallet

```bash
a4 protocol actions <protocol-or-deployment> --wallet <pubkey> --json
a4 protocol actions --dev --wallet <dev-wallet>
```

From the MCP server (`a4 mcp`): `protocol_actions` (or `dev_actions` in an
`a4 dev` session) → `protocol_prepare` → check the `summary` → sign every
transaction with the wallet outside the server → `protocol_submit` →
`protocol_intent` if it had not settled. Add `dev: true` to target the local
session.

- Several steps can be allowed at once; when the user's intent does not pick
  one, ask.
- Before signing, check the summary against the user's intent: programs
  invoked, accounts written, the SOL and tokens leaving the wallet, and
  simulation errors. Stop and report on any mismatch.
- Args are typed by their schema: amounts and keys are strings with a
  `format` (`u64`, `pubkey`).
- The server signs only with a signer the user configured (`[signer]` in
  `~/.arete/mcp.toml` or `ARETE_MCP_SIGNER_CONFIG`): a keypair plus a
  policy (deployment allow-list and a SOL cap required; programs, token
  caps and clusters optional; mainnet only when listed). Then
  `protocol_execute` prepares, checks the policy, signs and submits; a
  refusal names the rule and nothing is signed.

See [references/agents.md](references/agents.md).

## Automate Steps for Users

When a step needs the user's own signature but should run without them
(a refund when a raise fails, a weekly buy, moving a deposit when rates
change), the app's backend can run it with `@usearete/automations`:

```ts
// automations/index.ts: run by `a4 dev up` ([dev.automations] entry), and in
// production by a host adapter (`npx @usearete/automations add <host>`)
export default defineAutomations(TOKEN_RAISE_PROTOCOL, (automations) => {
  automations.on("raise.failed", async (event, ctx) => {
    const grants = await ctx.consents.list({ step: "refund", instanceId: event.instanceId });
    return grants.map((g) => ctx.step("refund", { wallet: g.wallet, instance: event.instanceId }));
  });
});
```

- Recommend the least powerful mechanism first: a `keeper` step if anyone may
  advance it, a direct deposit if the money is committed now, an on-chain
  allowance for a fixed recurring payment. Automations are for steps the user
  must sign, from a wallet that already holds their funds.
- Develop in the dev environment, then mainnet (never devnet): `a4 dev up`
  runs the definition, forwards the environment's events as signed
  deliveries and reloads on save; `a4 dev consent` opts a dev wallet in.
- Triggers: signed webhooks for emitted events (`webhookHandler`), `follow`
  for one instance's stream, schedules declared with
  `automations.schedule(name, { every })`, and `fromView`, `fromViewRows` or
  `fromAppendView` for live data. Handlers return steps; they never prepare
  or sign.
- Nothing runs without a consent the user signed (`consent.proofMessage`,
  then `consent.grant`) that states the steps, per-asset limits, settings and
  expiry. Every step is checked against the consent, the protocol, the
  committed transactions, a simulation and the signing policy, then signed by
  the app's signer (`keypairSigner` for dev, `privySigner` for Privy server
  wallets) and submitted.
- Production keeps state in Postgres (`postgresStore`, migrations from a
  deploy step) and is safe across retries, duplicate triggers and replicas.
- Deploy with `npx @usearete/automations add cloudflare|vercel|node`: it writes
  the runtime wiring and the host's entry and config, and prints the steps
  (`a4 install --locked`, migrations, secrets, the host's deploy, then
  `a4 protocol webhook add` for the endpoint). Each deploy and the webhook
  registration needs the user's request.

See [references/automations.md](references/automations.md).

## Refuse

Do not, and do not help to:

- upload or run unreviewed handler code, or describe process limits as a sandbox;
- give a handler network access, secrets, environment variables or files;
- sign a review, create a public deployment, activate a deployment that has
  handler code, or change production flags or keys (these are Arete's
  decisions; self-serve activation covers only private template-only
  deployments);
- deploy, push programs or publish without the user's request;
- give Arete's Squads proposer the Vote or Execute permission;
- create a multisig in a protocol's steps, or read a Squads step's multisig
  from anything but a resource;
- push a team toward code or raw transactions instead of the dashboard or
  the Squads app;
- hold, generate or sign with a role wallet's key on Arete's behalf;
- resend a transaction whose submission outcome is unknown;
- put an API key in browser code, or a secret key anywhere but a backend;
- configure the MCP signer, widen its policy, or list mainnet in it without
  the user asking; read, print or copy a signer keypair;
- sign or submit a prepared step whose summary does not match what the user
  asked for, or retry a policy refusal by loosening the policy;
- have Arete hold, store or proxy a user's key for automations;
- deploy automations to a host or register a webhook endpoint without the
  user's request, or ship a build that still bundles the dev client (run
  `a4 install --locked` first);
- automate a step for a wallet without that user's signed consent, beyond
  its steps, limits or expiry, or pre-tick or bundle the consent with
  sign-in or a deposit;
- loosen an automation's signing policy or a provider policy to get a
  refused step through, or present a provider policy as bounding amounts it
  lists as `unenforced`;
- reach for delegated signing when a keeper step, a direct deposit or an
  on-chain allowance would do.

## Verify

- `arete-protocol check` passes with no findings.
- Every guard has a stable code and its rejection reads well with operands.
- Every template step builds the same instruction a reference handler would.
- Two builds give byte-identical artifacts.
- For apps: a publishable-key client compiles without calling any `secret`
  step, rejections are handled exhaustively, and ambiguous submissions are
  never retried.
- The app ran against `a4 dev` with only `ARETE_API_URL` and `ARETE_API_KEY`
  changed, and `a4 install --check` passes before committing.
