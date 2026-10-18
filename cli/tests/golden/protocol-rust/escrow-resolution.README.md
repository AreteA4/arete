# `escrow-resolution` protocol client (Rust)

Generated from protocol package `escrow-resolution` 0.1.0 (spec `escrowResolution` 0.1.0), installed as `escrow-resolution`. `a4 install` regenerates this folder, so do not edit it.

## Use

Add the crate as a path dependency: `escrow-resolution-protocol = { path = "…" }`.

```rust,ignore
use escrow_resolution_protocol::*;

// Servers: a secret key prepares every client-callable step.
let client = EscrowResolutionSecretClient::new(std::env::var("ARETE_API_KEY")?)?;
```

`EscrowResolutionPublishableClient` takes a publishable key and has only the `public` steps. Both clients dereference to `arete_sdk::protocol::ProtocolClient` for reads (`instance`, `list_instances`, `participant`, `allowed_steps`, and `actions` for what a wallet can do across the deployment), `submit`, `intent`, metadata (`update_metadata`, `sign_metadata_proof`) and `events` (the instance event stream). Never submit again after `submitted_unknown`: read the intent until it settles.

## Deployment

Calls the public deployment `7d0c5a52-2f8e-4c1b-9a37-1b6f3c2a9e10`. A protocol session is minted from your own key through the session endpoint; this folder holds no credential.

## Steps

| Step | Method | Scope | Authority | Access | Rejection codes |
|---|---|---|---|---|---|
| `open` | `prepare_open` | create | creator | secret | - |
| `refund` | `prepare_refund` | instance | role `resolver` | secret | `not_resolver`, `not_awaiting_resolution` |
| `release` | `prepare_release` | instance | role `resolver` | secret | `not_resolver`, `not_awaiting_resolution` |

Every prepare can also be rejected with `step_requires_secret_key`, `step_not_in_session_scope` or `step_not_client_callable`. Instance and participant steps can also be rejected with `instance_terminal` (the instance is in a terminal state) or `participant_terminal` (the wallet's participant is in a terminal state), each with `detail.state`.

## Events

- `escrow.awaitingResolution` (instance)
- `escrow.resolved` (instance)

Verify webhook deliveries with `verify_escrow_resolution_webhook`.

