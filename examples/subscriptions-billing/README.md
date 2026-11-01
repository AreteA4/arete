# Subscriptions Billing

An Arete protocol for recurring token billing, wrapping the Solana Foundation
[`subscriptions`](https://github.com/solana-foundation/subscriptions) program.
A merchant publishes a plan, subscribers approve it, and the merchant or one
puller wallet collects each period's amount. Every step is an instruction
template; there is no handler code.

> [!WARNING]
> The live flow doesn't work yet. Arete's protocol reads currently fail on this
> program's accounts with `account-decode-failed`, so an instance never leaves
> its create step. `arete-protocol check` and `npm test` pass;
> `scripts/verify-fork.sh` stops at "the instance never appeared".

| File | What it is |
| --- | --- |
| `protocol.ts` | the protocol definition |
| `generated/typescript/programs/subscriptions/` | the program's SDK, from the registry, imported as `@programs/subscriptions` |
| `arete.toml`, `arete.lock` | the dependency and its pinned version |
| `tests/spec.test.mjs` | offline checks of each step's authority and guards |
| `scripts/verify-fork.sh` | the whole flow on an `a4 dev` fork, by hand |
| `scripts/chain-values.ts` | reads the values clients must pass back (see below) |

## The model

One protocol instance is one on-chain plan.

- `createPlan` is the only create step. It binds
  `pda("subscriptions", "plan", wallet, args.planId)`, so a second create for
  the same merchant and `planId` fails on chain.
- `planId`, `mint`, `amount` and `periodHours` are params, so they never change.
  To change a price, create another plan.
- Plans created outside Arete can't be adopted.
- Mints are SPL Token mints: the program's `createPlan` fixes the token program.

| Who | What |
| --- | --- |
| Merchant (role) | the wallet that creates the plan; `updatePlan`, `collect` |
| Puller (role) | `pullers[0]`; `collectAsPuller`. The default pubkey when the plan has none, and then only the merchant collects |
| Subscriber (participant) | any wallet; `initAuthority`, `subscribe`, `cancel`, `resume` |

Your backend holds the puller's key. Arete holds no merchant or puller key.
`pullers[1..3]` must stay the default pubkey: one puller per plan for now.

| Step | Instruction | Notes |
| --- | --- | --- |
| `createPlan` | `createPlan` | the first destination must be set: `collect` checks against the list |
| `updatePlan` | `updatePlan` | status, end time and metadata only. The pullers must match the chain |
| `collect` | `transferSubscription` | one period's amount, to one of the plan's destinations |
| `collectAsPuller` | `transferSubscription` | same args; the wallet must still be on the plan's pullers |
| `initAuthority` | `initSubscriptionAuthority` | once per wallet and mint; creates the token account if missing |
| `subscribe` | `subscribe` | needs the authority |
| `cancel` | `cancelSubscription` | runs to the end of the current period |
| `resume` | `resumeSubscription` | before that period ends |

The instance goes `open` → `active` ⇄ `sunset`, derived from the plan's status.
A participant goes `prospect` → `subscribed` ⇄ `canceled`. No state is terminal,
because a terminal instance would reject `cancel`. The program itself never
takes a plan out of sunset.

## Client-supplied structs

Templates pass each instruction argument as one expression, and expressions
can't build a struct. So `planData`, `updatePlanData`, `subscribeData`,
`resumeData` and `transferData` are step args: the client fills them in, and
guards refuse any field that disagrees with the params or the chain. Nothing a
client sends can change more than the program and the guards allow.

Clients read the chain for these values; `scripts/chain-values.ts` shows how,
and `scripts/verify-fork.sh` passes them to each step.

| Arg | Rules |
| --- | --- |
| `createPlan` `planData` | `planId`, `mint`, `terms.amount`, `terms.periodHours` repeat the top-level args. `terms.createdAt` is ignored (the program sets it): pass `"0"` |
| `destinations`, `pullers` | exactly 4 items, padded with the default pubkey |
| `metadataUri` | UTF-8 bytes, zero-padded to exactly 128 items |
| `updatePlanData` | `pullers` and every `expected*` field come from the plan as read |
| `subscribeData` | `planBump` and `expected*` come from the plan and the wallet's authority |
| `resumeData` | `expectedExpiresAtTs` comes from the subscription |
| `transferData` | `amount` and `mint` match the params; `delegator` is `subscriber` |

## Develop

```sh
npm install
npm run typecheck
npx arete-protocol check          # with ARETE_API_KEY, runs the server's checks too
a4 protocol deploy --dry-run      # the same server checks with your a4 login; changes nothing
npm test                          # offline
./scripts/verify-fork.sh          # on an a4 dev fork
```

`verify-fork.sh` creates the environment if needed and uses fresh wallets each
run: merchant, puller, subscriber and a stranger. It runs the flow, the
rejections that keep the roles apart (`not_merchant`, `not_puller`, a subscriber
calling `updatePlan`) and `a4 protocol actions` for each wallet.

## Webhooks

Registering a webhook endpoint is per account, not a protocol step:
`a4 protocol webhook add`. This protocol declares no events yet.
