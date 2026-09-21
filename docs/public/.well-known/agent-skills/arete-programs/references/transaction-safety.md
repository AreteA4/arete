# Transaction Safety

Read this reference before adding wallet signing, transaction submission, automatic retry, or post-send reconciliation.

## Authority Boundary

- Reading, deriving, building, preparing, inspecting, signing, and submitting are separate actions.
- Do not sign or submit unless the user's request authorizes an on-chain write.
- Do not deploy or operate hosted resources merely because a transaction integration needs credentials.
- Never request, log, store, or transmit a wallet private key or seed phrase.

## Before Signing

Verify the exact:

- cluster and program ID;
- instruction/semantic operation;
- writable and signer accounts;
- token mint and token program;
- raw versus UI amount conversion and decimals;
- fees, price/slippage bounds, deadlines, and recipient;
- prepared artifacts and required signer addresses.

Prefer semantic preparation, then inspect/simulate where supported. Show material effects to the approving user or wallet surface.

## Submission Contract

Sign locally. Submit a signed transaction at most once. Do not implement Arete-to-direct fallback after an operation starts, because a failed hosted response can still mean the transaction reached Solana.

Preserve the four terminal outcome states:

| Outcome | Meaning | Safe response |
| --- | --- | --- |
| `confirmed` | Reached the requested commitment | Reconcile application state |
| `not-submitted` | Definitely did not reach submission | Fix the cause; retry only under explicit policy |
| `submitted-unknown` | May have reached Solana | Query the known signature; never blindly resend |
| `chain-failed` | Solana reported an on-chain error | Surface the parsed error; do not resend unchanged |

Store the request ID and signature in diagnostics, but never secrets or signed transaction bytes unless the application explicitly requires protected audit storage.

## Authentication

Hosted transaction scopes are distinct:

- `read`
- `transaction:inspect`
- `transaction:send`

Use the least scope required. Browser publishable keys must be origin-bound. A wallet signs locally; Arete must not receive private signing material.

## React Reconciliation

Generated mutations can reconcile after confirmation by waiting for the processed-slot watermark and refreshing selected live results. A reconciliation timeout after confirmation is `confirmed-unreconciled`; it must not turn a confirmed chain result into a failed transaction or trigger resubmission.
