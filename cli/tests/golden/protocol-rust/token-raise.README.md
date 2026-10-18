# `token-raise` protocol client (Rust)

Generated from protocol package `token-raise` 1.0.0 (spec `tokenRaise` 1.0.0), installed as `token-raise`. `a4 install` regenerates this folder, so do not edit it.

A token raise on Meteora Presale with a soft cap, team finalisation through Squads, and refunds.

## Use

Add the crate as a path dependency: `token-raise-protocol = { path = "…" }`.

```rust,ignore
use token_raise_protocol::*;

// Servers: a secret key prepares every client-callable step.
let client = TokenRaiseSecretClient::new(std::env::var("ARETE_API_KEY")?)?;
match client
    .prepare_claim(&instance_id, &wallet, ProtocolPrepareOptions::default())
    .await?
{
    ProtocolPrepareResult::Prepared(prepared) => {
        // Sign `prepared.transactions` with the wallet, then:
        let state = client.submit(&prepared.intent.execution_id, signed).await?;
    }
    ProtocolPrepareResult::Rejected(rejection) => println!("rejected: {}", rejection.rejected()),
}
```

`TokenRaisePublishableClient` takes a publishable key and has only the `public` steps. Both clients dereference to `arete_sdk::protocol::ProtocolClient` for reads (`instance`, `list_instances`, `participant`, `allowed_steps`, and `actions` for what a wallet can do across the deployment), `submit`, `intent`, metadata (`update_metadata`, `sign_metadata_proof`) and `events` (the instance event stream). Never submit again after `submitted_unknown`: read the intent until it settles.

## Deployment

Calls the public deployment `7d0c5a52-2f8e-4c1b-9a37-1b6f3c2a9e10`. A protocol session is minted from your own key through the session endpoint; this folder holds no credential.

## Steps

| Step | Method | Scope | Authority | Access | Rejection codes |
|---|---|---|---|---|---|
| `claim` | `prepare_claim` | participant | participant | public | `raise_not_finalized` |
| `contribute` | `prepare_contribute` | participant | participant | public | `raise_not_open`, `deadline_passed`, `wallet_cap_exceeded`, `hard_cap_exceeded` |
| `create` | `prepare_create` | create | creator | secret | `soft_cap_above_hard_cap` |
| `finalize` | - | instance | Squads proposal | not client-callable | `raise_not_succeeded` |
| `refund` | `prepare_refund` | participant | participant | public | `raise_not_failed` |

Every prepare can also be rejected with `step_requires_secret_key`, `step_not_in_session_scope` or `step_not_client_callable`. Instance and participant steps can also be rejected with `instance_terminal` (the instance is in a terminal state) or `participant_terminal` (the wallet's participant is in a terminal state), each with `detail.state`.

## Events

- `contribution.confirmed` (participant): A contribution confirmed on chain.
- `raise.failed` (instance)
- `raise.succeeded` (instance)

Verify webhook deliveries with `verify_token_raise_webhook`.

