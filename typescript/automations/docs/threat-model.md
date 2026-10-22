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
- caps on direct SOL and `TransferChecked` transfers;
- the consent's expiry;
- no key export.

Privy cannot bound what a protocol program moves by CPI, cumulative
spending, or anything it cannot see in top-level instructions (see
[privy-policy-expressiveness.md](privy-policy-expressiveness.md)). So keep
the program allow-list tight, keep authorization keys in a secrets manager,
and choose protocols whose steps can only move funds between the user's
own positions.

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
a setting needs a new signature.

**Cap races.** Only one step job per wallet is active at a time, which the
Postgres store enforces with a unique index. The window check and the
ledger write therefore happen in one section per wallet. Ledger entries are
written with the signed bytes, before submit, and voided only when the
intent never lands.

**Revocation mid-flight.** Consent is read again immediately before
signing, with its limits. A revocation between signing and submit is a
window of milliseconds and is accepted. Nothing can be recalled after
submit.

**Signer tampering.** Before signing, the prepared bytes must hash to the
intent's `messageSha256`. After signing, every message must still match,
no other signature slot may change, and the wallet's signature must verify.
A signer that adds an instruction is refused.

**Wrong or stale triggers.** An oracle blip, a mispriced market or a
lagging view can fire a trigger. `stableFor` and hysteresis damp it at the
source, cooldowns limit how often a user is acted on, and handlers re-read
live state. Every step is still checked by `allowedSteps`, prepare and
simulation. The worst case is a step the protocol allows within the user's
limits, so view-triggered steps must only move funds between the user's
own allowed positions.

**Users who can't stop it.** Revoking needs no signature and takes effect
before the next signature. Removing your app's delegation in the wallet
provider stops signing at once, whatever your backend does.

## Out of scope

- Fee sponsorship. Users pay their own fees, or you top them up.
- Steps whose later transactions depend on earlier ones in the same step.
  The policy refuses them because they cannot be simulated one at a time.
- Mainnet use with real funds before an operator has reviewed this model
  for your deployment.
