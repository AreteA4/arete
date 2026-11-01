# `subscriptions` TypeScript program SDK reference

Generated from program package `subscriptions` 1.0.13, installed as `subscriptions`. `a4 install` regenerates this folder, so do not edit it.
This file lists only what is specific to this SDK. For how to use any Arete SDK (`createSession`, wallets, preparing and executing operations), see the `arete-programs` agent skill or https://docs.arete.run/using-stacks/transactions/.
Look parts up with `a4 sdk describe subscriptions [--view <Entity/view>] [--read <name>] [--program <key>] [--json]` or the MCP tool `describe_sdk`.

## Import

```ts
import { SUBSCRIPTIONS_PROGRAM } from './generated/typescript/programs/subscriptions/subscriptions.js'; // from the project root
```

The path is relative to the importing file: from `src/`, import `'../generated/typescript/programs/subscriptions/subscriptions.js'`. TypeScript resolves the `.js` to `subscriptions.ts`.

Register it as `subscriptions`: `createSession({ programs: { subscriptions: SUBSCRIPTIONS_PROGRAM } })` gives `session.programs.subscriptions`. Paths below start there.

Program `De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44`.

## Accounts and instructions

- Accounts (`accounts.<Name>.fetch(address)`): `fixedDelegation`, `plan`, `recurringDelegation`, `subscriptionAuthority`, `subscriptionDelegation`
- Raw instructions (`raw.<name>`): `initSubscriptionAuthority`, `createFixedDelegation`, `createRecurringDelegation`, `revokeDelegation`, `transferFixed`, `transferRecurring`, `closeSubscriptionAuthority`, `createPlan`, `updatePlan`, `deletePlan`, `transferSubscription`, `subscribe`, `cancelSubscription`, `resumeSubscription`, `revokeSubscriptionAuthority`, `revokeAbandonedDelegation`, `revokeAbandonedSubscription`, `cancelSubscriptionNow`
- PDAs (`pdas.<name>`): `eventAuthority`, `fixedDelegation`, `plan`, `recurringDelegation`, `subscriptionAuthority`, `subscriptionDelegation`
- Helpers: `constants.program`, `addresses.eventAuthority`, `addresses.subscriptionAuthority`, `addresses.plan`, `addresses.delegation`, `addresses.fixedDelegation`, `addresses.recurringDelegation`, `addresses.subscription`, `math.isExpired`, `math.fixedRemainingAmount`, `math.recurringRemainingAmount`

