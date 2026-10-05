# Program Operation Lifecycle

Read this reference when deciding between account reads, raw instruction building, semantic preparation, transaction inspection, and execution.

## Program Reads

Generated account readers are bound to an exact Program Release and Program Read service:

- `fetch(address)` returns one decoded account or the language's missing value.
- `fetchMany` / `fetch_many` preserves per-address status.
- `exists(address)` checks existence without decoding a full account.

Do not substitute an arbitrary RPC decoder if an exact generated reader is available. Validate that Program Read is marked available in the descriptor.

## Address Derivation

Use generated `pdas` and `addresses` helpers. They encode exact seed order, serialization, program IDs, and bump behavior. Do not reconstruct seeds from prose unless auditing the generated helper itself.

## Raw Build

`raw.<instruction>.build(params)` mirrors the normalized IDL:

- Raw names and nested values retain the IDL wire shape.
- Account-name parameters override generated resolution.
- Argument-name parameters serialize instruction data.
- `resolve`-style input supplies values used only by PDA resolution.
- Missing required values and unknown parameters fail closed.

The result is unsigned instruction data suitable for inspection or composition. Building does not imply sending.

## Semantic Prepare

Generated extensions may expose:

- `instructions.<name>.prepare(input)` for one instruction.
- `transactions.<path>.prepare(input)` for one transaction containing multiple instructions.
- `flows.<path>.prepare(input)` for multiple transactions.

Prepared values carry names, artifacts, required signer addresses, generated errors, and composable instructions/transactions. Prefer them when they exist.

Preparation may perform network reads. Re-run preparation near execution when authoritative account state, price, nonce, or eligibility can become stale. A quote is a preview, not a reservation.

## Inspect

Inspection compiles and simulates an unsigned transaction. It may return fee, logs, program errors, and required context without wallet approval or submission.

Use inspection to present material effects and detect deterministic failures. Do not report simulation success as confirmation that the transaction was sent or will land unchanged.

## Execute

Execution validates required signer coverage, asks the configured wallet to sign, submits through the selected transaction transport, and returns receipts. Use:

- `client.transaction([...instructions], options)` for manually composed built instructions.
- `client.execute(prepared, options)` for prepared instructions, transactions, or flows.

One wallet can be shared by a multi-stack session; the connected client supplies the correct authenticated transaction transport for each invocation.

## Reconcile

When execution returns a confirmed slot, `waitForProcessedSlot` / `wait_for_processed_slot` can wait until the live stack has processed at least that slot. The watermark proves the stream caught up. It does not prove that a particular entity changed, so choose what to refresh from what the operation touches.

No tool infers this. Work it out while writing the code:

1. List the programs, instructions, and accounts the operation uses. In TypeScript, `describePreparedOperation(prepared)` from `@usearete/sdk` returns each transaction's instructions with their program ids and account keys. `a4 explore program <program-ref> --operation <operation> --json` lists the operation's required and derived accounts without preparing it.
2. List each installed stack's entities and views with `a4 explore stack <stack-ref> --json` (`--summary` for the short form). `a4 explore stack <stack-ref> <Entity> --json` shows one entity's program, primary keys, and fields.
3. Match them. A view is likely affected when its entity is indexed from a program the operation calls and represents an account the operation writes, or is keyed by one of its accounts or signers. For example, an operation that writes a board, a round, and a miner account affects the board, round, and miner entities' views.
4. Refresh exactly those views, plus any one-shot reads or quotes that depend on the same accounts. In React, pass them to `submit(input, { reconcile: { refresh: [...] } })`. Elsewhere, wait for the processed slot, then read them again with `get`.

Do not refresh every view by default, and do not treat a caught-up watermark as proof that the expected change is visible.
