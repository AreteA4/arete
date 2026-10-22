# Automating steps for users

Some protocol steps need the user's own signature: a deposit from their
wallet, a refund to it, a claim, a rebalance between positions they own.
Without help those steps wait until the user opens your app.
`@usearete/automations` lets your backend run them for the user, inside
limits the user signed, with a key the user delegated to you. Arete never
holds the key.

## First, use the least powerful mechanism

Delegated signing is the most powerful tool here, and the one with the most
to go wrong. Use something weaker when it works:

| Situation | Mechanism | Needs this package? |
|---|---|---|
| The program lets anyone advance it | A keeper step | No |
| The money is committed now | Deposit straight into the pot or escrow | No |
| A fixed recurring payment from a wallet the user keeps using | An on-chain allowance (subscriptions, SPL `approve`, a Squads spending limit) | No |
| The user's signature is needed each time, the action is more than a fixed transfer, and the wallet is where the user's funds already live | Delegated signing under consent | **Yes** |

Delegated signing through an embedded wallet only makes sense when that
wallet is the user's primary wallet. If the user has to fund a wallet just
so you can automate it, they have already committed the money, and a direct
deposit is simpler and safer.

## How it fits together

```
user's app ──signs consent──► your backend ──────────────► Arete (prepare, submit)
                                │  @usearete/automations      │
Arete webhooks / streams ──────►│  ├─ triggers → handlers     │
your cron, live views ─────────►│  ├─ step jobs (one per wallet at a time)
                                │  ├─ policy + simulation     │
                                │  ├─ consent limits + ledger │
                                │  └─ signer ──► wallet provider (holds the key, enforces its own policy)
                                └─ store (Postgres, your database)
```

1. **Triggers.** A webhook, an instance event stream, your own cron
   (`dispatch` with a `scheduleSlot` id), or a live view source becomes a
   delivery, accepted once.
2. **Handlers** turn a trigger into step requests, usually by fanning out
   over the wallets that consented (`ctx.consents.list`). Handlers read
   state and return requests; they never prepare or sign.
3. **Step jobs** run one at a time per wallet. Each checks consent,
   cooldown, the policy's deployment and cluster, and whether the protocol
   allows the step. It then prepares the step, checks the prepared bytes
   against the intent, simulates them, and checks the policy and the
   consent's limits. Finally it signs, checks the signed bytes, saves them,
   submits, and tracks the intent.
4. **Outcomes** (`completed`, `chain_failed`, `expired`, `skipped`,
   `rejected`, `refused`, `failed`) reach your hooks, so your app can tell
   the user.

## What you set up

- **A signing policy** (`definePolicy`): the deployments, clusters and
  programs your automations may touch, and per-transaction caps on what may
  leave a wallet. It applies to every user.
- **A signer.** Use the keypair signer for development and dev wallets
  only. In production, use a wallet provider whose own policy bounds what
  your key may sign, for example Privy with `privyPolicyFor`.
- **A store.** Use the Postgres store in your existing database, migrated
  from your deploy step.
- **A simulator** (`rpcSimulator(RPC_URL)`). The policy refuses anything it
  cannot measure.
- **Consent.** Build a proof message, have the user sign it, and grant it.
  See [the consent UX guide](consent-ux.md).
- **Workers.** In a long-running server, `automations.start()`. In a
  serverless function, `await automations.drain()` per request or from a
  cron.

Read [the threat model](threat-model.md) before you ship: it says what each
layer protects against, and what it does not.

See `examples/token-raise/` for a complete backend that refunds
contributors automatically when a raise fails.
