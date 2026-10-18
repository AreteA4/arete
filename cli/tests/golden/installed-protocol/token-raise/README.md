# `token-raise` protocol client

Generated from protocol package `token-raise` 1.0.0 (spec `tokenRaise` 1.0.0), installed as `token-raise`. `a4 install` regenerates this folder, so do not edit it.

A token raise on Meteora Presale with a soft cap, team finalisation through Squads, and refunds.

## Import

```ts
import { createProtocolClient } from '@usearete/sdk';
import { TOKEN_RAISE_PROTOCOL } from './generated/typescript/protocols/token-raise/token-raise.js'; // from the project root

// Browser: a publishable key prepares `public` steps only.
const client = createProtocolClient(TOKEN_RAISE_PROTOCOL, { auth: { publishableKey } });
```

Or register it in a session: `createSession({ protocols: { tokenRaise: TOKEN_RAISE_PROTOCOL } })` gives `session.protocols.tokenRaise`, and promotes the protocol's stacks and programs into `session.stacks` and `session.programs`.

## Deployment

Calls the public deployment `7d0c5a52-2f8e-4c1b-9a37-1b6f3c2a9e10`. A protocol session is minted from your own key through the session endpoint; this folder holds no credential.

## Steps

| Step | Scope | Authority | Access | Rejection codes |
|---|---|---|---|---|
| `claim` | participant | participant | public | `raise_not_finalized` |
| `contribute` | participant | participant | public | `raise_not_open`, `deadline_passed`, `wallet_cap_exceeded`, `hard_cap_exceeded` |
| `create` | create | creator | secret | `soft_cap_above_hard_cap` |
| `finalize` | instance | Squads proposal | not client-callable | `raise_not_succeeded` |
| `refund` | participant | participant | public | `raise_not_failed` |

Every prepare can also be rejected with `step_requires_secret_key`, `step_not_in_session_scope` or `step_not_client_callable`. Instance and participant steps can also be rejected with `instance_terminal` (the instance is in a terminal state) or `participant_terminal` (the wallet's participant is in a terminal state), each with `detail.state`.

## Events

- `contribution.confirmed` (participant): A contribution confirmed on chain.
- `raise.failed` (instance)
- `raise.succeeded` (instance)

## Stacks and programs

- `stacks.meteoraPresale`
- `stacks.mirror`
- `programs.meteoraPresale`
- `programs.squadsV4`
