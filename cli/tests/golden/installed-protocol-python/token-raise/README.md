# `token-raise` protocol client (Python)

Generated from protocol package `token-raise` 1.0.0 (spec `tokenRaise` 1.0.0), installed as `token-raise`. `a4 install` regenerates this folder, so do not edit it.

A token raise on Meteora Presale with a soft cap, team finalisation through Squads, and refunds.

## Import

```python
from arete import AuthConfig
from token_raise_protocol import TokenRaiseClient, TokenRaiseSecretClient, TOKEN_RAISE_PROTOCOL

# A publishable key prepares `public` steps only.
async with TokenRaiseClient(auth=AuthConfig(publishable_key=key)) as client:
    result = await client.steps.<step>(wallet=wallet, ...)
    if result.ok:
        state = await client.submit(result.intent, signed_transactions)

# A secret key (servers) prepares every client-callable step.
client = TokenRaiseSecretClient(auth=AuthConfig(secret_key=key))
```

`TOKEN_RAISE_PROTOCOL.stacks` and `.programs` are the protocol's stacks and programs, ready for `arete.create_session(stacks=..., programs=...)`.

## Deployment

Calls the public deployment `7d0c5a52-2f8e-4c1b-9a37-1b6f3c2a9e10`. A protocol session is minted from your own key through the session endpoint; this folder holds no credential.

## Steps

| Step | Method | Scope | Access | Rejection codes |
|---|---|---|---|---|
| `claim` | `steps.claim` | participant | public | `raise_not_finalized` |
| `contribute` | `steps.contribute` | participant | public | `raise_not_open`, `deadline_passed`, `wallet_cap_exceeded`, `hard_cap_exceeded` |
| `create` | `steps.create` | create | secret | `soft_cap_above_hard_cap` |
| `finalize` | - | instance | not client-callable | `raise_not_succeeded` |
| `refund` | `steps.refund` | participant | public | `raise_not_failed` |

Every prepare can also be rejected with `step_requires_secret_key`, `step_not_in_session_scope` or `step_not_client_callable`. Instance and participant steps can also be rejected with `instance_terminal` (the instance is in a terminal state) or `participant_terminal` (the wallet's participant is in a terminal state), each with `detail.state`.

## Events

- `contribution.confirmed` (participant): A contribution confirmed on chain.
- `raise.failed` (instance)
- `raise.succeeded` (instance)

Verify webhook deliveries with `verify_token_raise_webhook(secret, raw_body, headers)`.

## Stacks and programs

- `STACKS["meteoraPresale"]`
- `STACKS["mirror"]`
- `PROGRAMS["meteoraPresale"]`
- `PROGRAMS["squadsV4"]`
