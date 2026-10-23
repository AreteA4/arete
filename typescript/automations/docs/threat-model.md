# Threat model

`@usearete/automations` signs transactions for users from your backend.
This document says which layer stops which failure, so you know what you
are relying on. It needs a review by a second engineer before each
release that changes a layer.

## Layers, strongest first

| Layer | Where it runs | Holds if your backend is compromised? |
|---|---|---|
| The protocol's on-chain rules | Solana | Yes |
| The wallet provider's policy (for example Privy) | The provider's enclave | Yes |
| The signing policy, checked on a simulation | Your backend | No |
| The user's consent and its limits | Your backend | No |
| The per-window ledger | Your backend | No |
| The exact-bytes checks around the signer | Your backend | No |
| Deduplication, idempotent prepare, reconciliation | Your backend and Arete | No |

**Arete never holds, stores or proxies a user's key.** It prepares
transactions, records intents and submits what your backend signed.

## Threats

**Backend compromise.** An attacker who controls your backend can skip
every check that runs there, including consent. What still bounds them is
the provider policy and the protocol's on-chain rules. With Privy, that
means:

- which programs the wallet may call;
- a cap on each direct SOL and `TransferChecked` transfer;
- the consent's expiry;
- no key export.

The transfer caps limit each transfer, not the total: they do not restrict
the destination, so an attacker can sign capped transfers to themselves,
one transaction after another, until the delegation expires or is removed.
They slow a drain; they do not bound it. Compute-unit prices are not
capped either, so fees can burn SOL. Privy also cannot bound what a
protocol program moves by CPI, cumulative spending, or anything it cannot
see in top-level instructions (see
[privy-policy-expressiveness.md](privy-policy-expressiveness.md)). So keep
the program allow-list tight, keep authorization keys in a secrets manager,
keep delegations short, and choose protocols whose steps can only move
funds between the user's own positions. Revoking a consent in your backend
does not change the provider policy: against a compromised backend, only
removing the delegation in the provider stops signing.

**Forged webhooks.** Every delivery's HMAC is verified over the raw body,
with a timestamp tolerance, before anything is parsed. A delivery for
another deployment is ignored.

**Replayed webhooks or dispatches.** A delivery id is accepted once. Step
job ids are deterministic per delivery, step, wallet and instance. Prepare
uses an idempotency key per job and generation.

**Double spend after a crash.** Signed bytes are saved before submit. A job
that had signed always reads its intent first. It submits the saved bytes
only if the intent was never submitted, and never signs again or submits
blind. A wallet stays locked while its intent is `submitted_unknown`.

**Consent forgery or replay.** A grant needs the wallet's Ed25519 signature
over a message rebuilt from the grant's fields, with a nonce the backend
issued and that works once. Changing any field, raising a limit or changing
a setting needs a new signature. Settings are signed but not enforced: the
runtime does not compare a step's arguments with them, so a handler must
read them from the grant (`ctx.consents.list`) rather than from elsewhere.
The domain in the message is text the user reads; wallets do not check it
against the page, as they would for Sign-In with Solana.

**Cap races.** Only one step job per wallet is active at a time, which the
Postgres store enforces with a unique index. The window check and the
ledger write therefore happen in one section per wallet. Ledger entries are
written with the signed bytes, before submit, and voided only when the
intent did not land. When Arete reports an intent expired, the runtime asks
the chain before preparing the step again: a transaction that landed keeps
its spending and ends the job, and one whose blockhash is still valid is
waited out. Durable-nonce transactions, which never expire, are refused.
Limits are per store: two services with separate stores for one deployment
each allow the full window.

**Revocation mid-flight.** Consent is read again immediately before
signing, with its limits. Bytes signed before a revocation can still be
submitted: normally within milliseconds, but after a crash or a failed
submit the saved bytes are submitted when the job resumes, without reading
consent again, until the intent expires. Nothing can be recalled after
submit.

**Signer tampering.** Before signing, the prepared bytes must hash to the
intent's `messageSha256`. After signing, every message must still match,
no other signature slot may change, and the wallet's signature must verify.
A signer that adds an instruction is refused.

**Wrong or stale triggers.** An oracle blip, a mispriced market or a
lagging view can fire a trigger. `stableFor` and hysteresis damp it at the
source, cooldowns limit how often a user is acted on, and handlers re-read
live state. Every step is still checked by `allowedSteps`, prepare and
simulation; a simulation that does not report inner instructions is
refused, since approvals made by CPI would not show. The worst case is a step the protocol allows within the user's
limits, so view-triggered steps must only move funds between the user's
own allowed positions.

**Users who can't stop it.** Revoking needs no signature and takes effect
before the next signature. Removing your app's delegation in the wallet
provider stops signing at once, whatever your backend does.

## What the runtime trusts

- **The simulator's RPC.** Every cap is checked on its simulation. An RPC
  that lies, serves another cluster or a stale fork defeats them. Use an
  RPC you control or trust, on the cluster the deployment runs on.
- **Arete's intent states.** They decide when a step is done and when a
  job may sign again; the chain is asked before signing an expired intent
  again.
- **The `clusters` label.** The cluster comes from the client (or the
  `cluster` option) and is not checked against the chain: `deployments` is
  what binds the policy to a deployment.
- **Only SOL and SPL tokens are measured.** An NFT, a compressed asset or a
  position a protocol holds for the user can move without counting against
  any cap, through any program the policy allows. `createAutomations`
  therefore needs a `programs` allow-list, and the consent message says
  what its limits count.
- **The webhook secret.** Anyone holding it can trigger any handler, so any
  consented step for any consenting wallet, at a time they choose, within
  consent, limits and cooldowns.

## Out of scope

- Fee sponsorship. Users pay their own fees, or you top them up.
- Steps whose later transactions depend on earlier ones in the same step.
  The policy refuses them because they cannot be simulated one at a time.
- Mainnet use with real funds before an operator has reviewed this model
  for your deployment.
