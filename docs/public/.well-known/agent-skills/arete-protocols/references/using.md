# Using an installed protocol

`a4 install protocol <ref> --ts` generates the client under
`generated/typescript/protocols/<alias>/` (`arete.protocols.<name>` in a
session) with the protocol's stacks and programs hoisted alongside it.
`--rust` generates a crate over `arete-a4-sdk` and `--python` a package over
`arete-sdk`; see [Rust](#rust) and [Python](#python) below. Installing a
public protocol records the shared deployment only; no credentials are
written.

Your own deployed protocol installs the same way: activation publishes an
owner-private package, so `a4 install protocol <slug> --ts` resolves it for
your account with no deployment flag (other accounts can't). Never edit the
generated files; `a4 install --check` in CI fails when a client is not the
published release `arete.lock` pins.

## Execution receipts

A signed execution receipt connects a completed transaction signature to its
protocol deployment/release, instance, step, message hash and assessed fees.
It is distinct from the build receipt and from a UI execution/history entry.
Keep the transaction signatures and execution/instance ids returned by the flow.
If a receipt is not yet available, inspect finality and intent status; retry the
read rather than sending the transaction again.

Read a receipt with the generated client's `getReceipt(signature)` and the
published key set with `getReceiptKeys()` (check the installed SDK for language
spellings), or use:

- `GET /api/protocol-receipts/<signature>`
- `GET /api/protocol-receipts/keys`

Private deployment receipts require the owner/integrator's authorized API key;
public receipts and the key set can be read anonymously. The current CLI has no
dedicated receipt command. Never expose a secret key in browser code or logs to
fetch a private receipt; use an authenticated backend when needed.

Fetching JSON does not verify its claims. In TypeScript, `verifyReceipt` checks
receipt signatures against a trusted receipt-key set; `verifyReceiptOnChain`
checks the message/memo claims against chain transaction evidence. Check the
installed SDK's exact input types and validation result, and establish trust in
the key set rather than accepting arbitrary keys attached to a receipt. Query
the transaction on the receipt's actual chain: a dev-environment signature must
be read from that fork's RPC. Verify deployment/release, instance, step and fees
against the requested operation as well.

An absent private receipt can mean insufficient access rather than a missing
transaction. Receipt issuance also depends on service configuration and the
execution path; a standalone local protocol runtime does not issue hosted
receipts. Preserve errors and report that distinction rather than inventing a
receipt or treating absence as permission to resubmit.

Applications display any prepared fee and verify completed fees through the
receipt. Users do not manage classifications or request approval for fees.
See [fees](authoring.md#fees).

## Keys and access

- **Browser:** a publishable key, bound to the app's origin. The client can
  read, subscribe, and prepare and submit `public` steps (typically
  participant steps such as contribute, claim, refund).
- **Backend:** a secret key. Needed for `secret` steps (by default the
  create step and role steps). Never ship a secret key to the browser.
- The generated client is typed by key class: on a publishable-key client a
  `secret` step does not exist, so calling it is a compile error.
- When the key comes from `ARETE_API_KEY` (or a token) rather than
  `auth.secretKey`, the TypeScript client can't tell its class and types
  itself as publishable, so `secret` steps are missing. On a backend pass
  `keyClass: "secret"`:

  ```ts
  const client = createProtocolClient(TIP_JAR, { keyClass: "secret" }); // key from ARETE_API_KEY
  ```

- Sessions are minted and refreshed by the SDK through the session endpoint;
  you do not handle tokens.

## Choose the environment

For development, `a4 dev up` generates a client with the deployment id in the
account's cloud dev environment. Use the account's credentials and the API
origin reported by the session; keep wallets and chain clients on the loopback
RPC. The session does not contain a reusable account secret key.

For production, deploy only when authorized, then restore the published client
with `a4 install --locked` and check it with `a4 install --check`. Read the
generated README for connection options. Backend credentials can come from
`ARETE_API_KEY`; `ARETE_API_URL` selects the API origin, not the chain RPC.
Never edit generated ids or put a secret key or session token in browser code.
See [local-dev.md](local-dev.md#the-apps-client).

## Reads and live state

```ts
const raise = arete.protocols.tokenRaise;
const page = await raise.instances.list({ state: "open" });
const instance = await raise.instances.get(id);         // canonical state, accounts, metadata
const steps = await raise.allowedSteps({ instance: id, wallet });
// [{ step: "contribute", allowed: false, reason: "deadline_passed", detail: { now, "params.deadline": ... } }]
const subscription = raise.events(id, { onEvent: (event) => { /* transitions and named events */ } });
// subscription.close() when done; the SDK resumes with Last-Event-ID after reconnects.
```

Render `allowedSteps` to drive the UI, and show `reason` with its `detail`
operands. To ask what one wallet can do across the deployment (create steps
and every instance it relates to), use `raise.actions({ wallet })`
(`useWalletActions` in React, `actions(...)` in Rust and Python); each step
carries its `scope` and typed `args` (amounts and keys are strings with a
`format` such as `u64` or `pubkey`). Canonical state comes from the API; the protocol stack adds live
fields and may lag.

## Prepare, sign, submit

```ts
const result = await raise.steps.contribute.prepare(
  { instance: id, wallet, amount: "1000000000" },
  { transactionFormat: "v0" },
);
if (!result.ok) {
  switch (result.rejection.rejected) { /* exhaustive over the step's guard codes */ }
  throw new Error(`Preparation rejected: ${result.rejection.rejected}`);
}
// Each entry is the full wire transaction (Arete's signatures filled in,
// the wallet's slots empty) as base64: hand it to the wallet unchanged.
const signed = await signWithWallet(result.transactions.map((tx) => tx.transactionBase64));
const state = await raise.submit(result.intent, signed); // { executionId, state, transactions }
```

- The client never signs. Prepare returns the exact transactions to sign,
  already co-signed by Arete (attestation memo, ephemeral keys); the wallet
  adds its signature (`signTransaction` on a `VersionedTransaction`) and the
  app passes the signed bytes to `submit`. Do not modify them; submit refuses
  any changed byte.
- Transactions are Solana Transaction V1 by default. Wallets don't sign V1
  yet, so pass `{ transactionFormat: "v0" }` whenever a wallet signs
  (including `a4 dev`); the intent may then contain several transactions to
  sign together.
- `submit` returns the intent's state. Poll `intents.get(executionId)` until
  it is `completed`, `chain_failed`, `expired` or `rejected`:

  ```ts
  let state = await raise.submit(result.intent, signed);
  while (!["completed", "chain_failed", "expired", "rejected"].includes(state.state)) {
    await new Promise((r) => setTimeout(r, 500));
    state = await raise.intents.get(result.intent.executionId);
  }
  // result.intent.instanceId is the instance a create step opened
  ```

- Intents expire (about 60 seconds or the blockhash lifetime). Prepare again
  with the same idempotency key to get a linked replacement.
- If submission ends `submitted_unknown`, never resend: poll the intent.

## Metadata

`raise.metadata.instance(id).update(patch)` for creator fields (secret key);
participant fields need a wallet proof: sign the message the SDK builds for
the patch with the participant's wallet.

## Webhooks

Register an endpoint and subscribe to named events with a secret key. The
signing secret is returned once. From the project:

```bash
a4 protocol webhook add --url https://app.example/arete/webhook   # every event the build declares; --events a,b narrows it
a4 protocol webhook list
a4 protocol webhook test <id>        # a signed test delivery now
a4 protocol webhook remove <id>
```

`--secret-only` prints just the secret, to pipe into the host's secret store.
Endpoints must be public https. Verify every delivery:

```ts
import { verifyWebhook } from "@usearete/sdk";
const event = verifyWebhook(secret, rawBody, headers); // throws on a bad signature or stale timestamp
```

Payloads carry ids and states only; read details with your own key. Dedupe by
event id. You receive events for instances your account created, plus shared
instances if you opted in.

## Squads proposer onboarding

Protocols with admin steps propose them to the team's Squads v4 multisig with
a proposer key that belongs to **your account** (one proposer for every
protocol you deploy). The proposer may only initiate; Arete never votes or
executes. The multisig is the deployment's: the protocol reads it from a
resource, and each deployment sets it in `[protocol.resources]`.

**Ask first.** Does the team already have a Squads v4 multisig? How do its
members sign: a hot keypair, a browser wallet, a hardware wallet (Ledger)?
Never ask for or handle a member's key.

- **No multisig yet**: the team creates one in the Squads app (on a dev
  environment, `a4 dev squads create --members alice,bob --threshold 1`).
- **Members sign in a browser wallet or a Ledger**: walk them through the
  Squads app. Members → Add member → paste the proposer
  (`a4 protocol squads proposer <deployment>`) → permission **Initiate**
  only → approve and execute the proposal. Then check the status.
- **A member can sign a raw transaction**: build the proposal with the CLI,
  the MCP tool or the client below; the member signs and sends it, then the
  team approves and executes it in Squads.

Steps, from the CLI (`--dev` for a dev environment):

```bash
a4 protocol squads proposer <deployment>                                  # the proposer's pubkey
a4 protocol squads add-proposer <deployment> --multisig <M> --member <member>   # unsigned config proposal
a4 protocol squads status <deployment> --multisig <M>                     # ready, or the problem
```

`add-proposer` prints the unsigned transaction; `--keypair <file> --rpc-url
<url>` signs and sends it as that member (only when the user asks and the
keypair is theirs). Deploy with `--no-activate` first when the deployment does
not exist yet. The MCP tools `protocol_squads_proposer`,
`protocol_squads_status` and `protocol_squads_add_proposer` return the same and
never sign.

On a dev environment the Squads app cannot see the chain, so `a4 dev squads`
stands in for it with dev wallets:

```bash
a4 protocol squads add-proposer --dev --multisig <M> --member alice --send
a4 dev squads show --multisig <M>                           # the config proposal's index
a4 dev squads approve --multisig <M> --index <n> --wallet alice
a4 dev squads execute --multisig <M> --index <n> --wallet alice
a4 dev fund <proposer> 1
```

Then fund the proposer: it pays the fee and rent of each proposal (about
0.01 SOL each). `status` reports `ready`, or the `problem`: `not_member`,
`permissions_invalid` or `proposer_unfunded`.

Until the proposer is an Initiate-only member, **activation refuses the
deployment** with a 409 whose message and `details` (`step`, `multisig`,
`proposer`, `problem`, `commands`) say what to run. Show the user the message
and follow it; deploy or activate again once `status` is `ready`. Once that
works, every protocol you deploy can use the same multisig with no further
onboarding.

From an app:

```ts
const raise = arete.protocols.tokenRaise;
const proposer = await raise.squads.proposer();     // { pubkey, permissions: "initiate", rootKeyId }
const status = await raise.squads.status(multisig); // { ready, member, mask, threshold, problem? }
if (!status.ready) {
  // An unsigned config proposal adding the proposer with Initiate only.
  // `member` is an existing member with Initiate who signs and pays.
  const built = await raise.squads.addProposer({ multisig, member });
  await sendWithWallet(built.transactionBase64);    // the member's wallet signs and sends it
  // The team then approves and executes proposal `built.proposal` in Squads.
}
```

`addProposer` returns a transaction for the team's member wallet to sign and
send itself; it is not submitted through `raise.submit`. Re-check `status`
until `ready` is true (`problem` is `not_member`, `permissions_invalid` or
`proposer_unfunded`).

If the proposer is removed after activation, admin proposals are parked and
retried with backoff for up to 24 hours, and the instance stream emits a
system event:

```ts
raise.events(id, {
  onEvent: (event) => {
    if (event.type === "arete.squads.proposerMissing") {
      // event.data: { step, multisig, proposer, problem, instanceId, ... }
      promptTeamToAddProposer(event.data);
    }
  },
});
```

## Protocol stack rows

Each deployment has a generated stack with three entities: **Instances**
(`ProtocolInstance`), **Activity** (`ProtocolActivity`) and **Participants**
(`ProtocolParticipant`, only when the spec has a participant machine), plus
per-instance views (`byId`, `byInstance`). Stream them with the stack SDK, or
locally with `a4 stream --dev`. Fields to rely on:

| Field | Meaning |
|---|---|
| `instance` | the instance's short id (base58 of the UUID's bytes); rows are keyed by it |
| `instance_id` | the instance UUID the API and `a4 dev step` use. `a4 stream` adds it next to `instance` (`id.instance_id` on instance rows) in every output but `--raw`; other consumers convert the short id themselves |
| Activity rows | one per transaction Arete prepared: `step`, `wallet`, `amount`, `attested`, `via`, `slot`, `timestamp`, and `activity_account`, the account the step's `stack.activity` entry names as `instanceAccount` |
| `<step>_protocol_count`, `<step>_protocol_amount` | under an Instances row's `activity`: the step's attested transactions and amounts, each counted under the step its attested memo names |
| `<step>_count`, `<step>_amount`, `<step>_direct_count` | under an Instances row's `activity`: every matching instruction (attested or not), and how many bypassed Arete and called the programs directly |
| `<step>_count`, `<step>_amount` | under a Participants row's `activity`: that participant's attested per-step counts and amounts |

- Direct (bypass) activity is matched only on accounts the instance owns:
  those the create step binds from its ephemeral keys. A step whose
  `instanceAccount` is a wallet, a role or a shared account is counted from
  attested activity only, so its direct count stays zero.
- Instances rows don't carry the machine state; read it with
  `instances.get(id)` (or watch it in `a4 stream --dev --tui`).
- Canonical state comes from the API; the stack adds live fields and may
  lag.

## Rust

```bash
a4 install protocol <protocol-ref> --rust
```

The crate defines `<Name>PublishableClient` (only `public` steps) and
`<Name>SecretClient` (every client-callable step; refuses a publishable key),
with a `prepare_<step>` method per step. Both dereference to
`arete_sdk::protocol::ProtocolClient` for `instance`, `list_instances`,
`allowed_steps`, `submit`, `intent`, metadata, `events` and `squads()`.

```rust
use futures::StreamExt;
use arete_sdk::protocol::{ProtocolEventsOptions, ProtocolPrepareOptions, ProtocolStreamEvent, ProtocolTransactionFormat};
use token_raise_protocol::*;

let client = TokenRaiseSecretClient::from_env()?; // ARETE_API_KEY, backend only
let args = TokenRaiseContributeArgs { amount: 1_000_000_000u64.into() }; // ProtocolInteger
let options = ProtocolPrepareOptions::default()
    .with_transaction_format(ProtocolTransactionFormat::V0);
match client.prepare_contribute(&instance_id, &wallet, &args, options).await? {
    ProtocolPrepareResult::Prepared(prepared) => {
        // Sign each `transaction_base64` unchanged with the wallet.
        let signed = sign_with_wallet(&prepared.transactions)?;
        let state = client.submit(&prepared.intent.execution_id, signed).await?;
        // On SubmittedUnknown: read client.intent(..) until it settles; never resend.
    }
    // `rejection.code` is typed by the step's guard-code enum; `detail` holds the operands.
    ProtocolPrepareResult::Rejected(rejection) => show_rejection(rejection.rejected(), rejection.detail),
}

let mut events = client.events(&instance_id, ProtocolEventsOptions::default());
while let Some(event) = events.next().await {
    match event? {
        ProtocolStreamEvent::Emit { data, .. } => { /* named event */ }
        ProtocolStreamEvent::SquadsProposerMissing { data, .. } => { /* onboard the proposer */ }
        _ => {}
    }
}

let payload = verify_token_raise_webhook(&secret, &raw_body, &headers, Default::default())?.payload;
```

Squads onboarding: `client.squads().proposer()`,
`client.squads().add_proposer(&ProtocolSquadsAddProposerRequest::new(multisig, member))`
and `client.squads().status(&multisig)`.

## Python

```bash
a4 install protocol <protocol-ref> --python
```

The package defines `<Name>Client` (publishable key, `public` steps only) and
`<Name>SecretClient` (secret key, every client-callable step), with
`client.steps.<step>(...)` per step.

```python
from arete import AuthConfig
from arete.protocol import SQUADS_PROPOSER_MISSING_EVENT
from token_raise_protocol import TokenRaiseSecretClient, verify_token_raise_webhook

async with TokenRaiseSecretClient(auth=AuthConfig(secret_key=key)) as client:
    result = await client.steps.contribute(
        instance=instance_id, wallet=wallet, args={"amount": "1000000000"},
        transaction_format="v0",
    )
    if not result.ok:
        handle_rejection(result.rejected, result.detail)   # guard code + operands
    else:
        signed = sign_with_wallet([tx["transactionBase64"] for tx in result.transactions])
        state = await client.submit(result, signed)        # never resend after submitted_unknown

    async with client.events(instance_id) as stream:
        async for event in stream:
            if event.type == "emit":
                ...                                         # event.event is the spec's event name
            elif event.type == SQUADS_PROPOSER_MISSING_EVENT:
                ...                                         # onboard the proposer

    status = await client.squads.status(multisig)
    if not status["ready"]:
        built = await client.squads.add_proposer(multisig=multisig, member=member)
        # the member's wallet signs and sends built["transactionBase64"]

payload = verify_token_raise_webhook(secret, raw_body, headers).payload
```

Names above are from a `token-raise` install; read the generated README in
the installed folder for the exact class, method and guard-code names.
