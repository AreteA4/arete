# Authoring a protocol

`defineProtocol` (from `@usearete/protocol`) lowers to the canonical
`arete.protocol-spec/v1` JSON. Write it in `protocol.ts` and default-export it.

For a new project, start from `a4 protocol init <dir>`: a template-only tip
jar with the System Program installed as `@programs/system`, a manifest,
lock, `package.json` and `tsconfig.json` ready for `npm install`,
`npx arete-protocol check` and `a4 dev up` (see
[local-dev.md](local-dev.md#start-a-project)). Replace its steps rather than
writing the project files by hand.

## Shape

```ts
import { builtins, defineProtocol, ix } from "@usearete/protocol";
import meteoraPresale from "@programs/meteora-presale"; // a4 install program meteora-presale --ts

export default defineProtocol({
  name: "tokenRaise",
  version: "1.0.0",
  programs: { meteoraPresale, ...builtins("system", "splToken") },
  params: { /* JSON-Schema subset: instance parameters, set by the create step's args */ },
  roles: { /* e.g. resolver: { description } */ },
  instanceVisibility: "account", // or "shared"
  reads: { presale: { kind: "account", program: "meteoraPresale", account: "Presale", address: "instance.accounts.presale", optional: false } },
  machines: { instance: { /* initial, states, derive, reads */ }, participant: { /* optional */ } },
  steps: { /* see below */ },
  reactions: [ /* see below */ ],
  events: { "raise.succeeded": { machine: "instance" } },
  metadata: { instance: { fields: { name: { schema: { type: "string", maxLength: 64 }, writer: "creator", filterable: true } }, maxBytes: 4096 } },
});
```

## Programs

Install every program the protocol calls or reads, then import it:

```bash
a4 install program meteora-presale --ts                        # a registry program
a4 install program ./idl/my_program.json --as my-program --ts  # a local IDL (add --program-id <address> if the IDL has none)
```

- `programs` takes the imported modules and `builtins(...)` (`system`,
  `splToken`, `associatedToken`, `computeBudget`, `memo`). The key you give a
  module in `programs` is its alias in expressions (`pda("meteoraPresale",
  ...)`, `reads`, `stack`).
- The module carries the program id, the ProgramSpec hash and where it was
  installed from. `arete-protocol build` derives the generated stack's
  program sources from it (a registry install at its exact version, a local
  IDL copied into the build) and checks each hash against `arete.lock`.
- A local IDL install is for programs not in the registry yet; its
  ProgramSpec hash is computed from the IDL file, the same hash the registry
  would give that IDL. Keep the IDL in the project: `a4 protocol deploy`
  registers it as your private program before the release (Arete checks
  templates only against programs it has) and records the result in
  `arete.lock`. After editing the IDL, run `a4 install` before deploying.

Rules the checker enforces:

- Exactly one `create` step. It declares `bind`: the instance accounts (from
  ephemeral keys, derived addresses, args) and every role's wallet.
- Every state is reachable; every non-terminal state has an exit; terminal
  states have none.
- Steps move only to **recorded** states; derivation rules target only
  **derived** states.
- Client-callable steps declare `access: "public" | "secret"` (defaults:
  the create step and role steps are `secret`, other wallet steps `public`).
  Keeper and Squads steps have no access.
- A step whose authority is a role carries the guard
  `wallet == instance.roles.<role>`.
- Every declared event is emitted by an `emit` reaction.

## Expressions

Guards, derivations, reaction conditions, read addresses and templates use
the stack-source expression language. Names in scope depend on where the
expression is:

| Name | Available |
|---|---|
| `now`, `params.*`, `resources.*` | everywhere |
| `deployment.id/attestation/squadsProposer` | everywhere, including the create step |
| `args.*`, `wallet`, `ephemeral.<declared>` | step expressions |
| `instance.id/state/accounts.*/roles.*` | everywhere except the create step |
| `participant.id/state/wallet` | participant steps and machine |
| `reads.<declared>.*` | where the read is declared (never inside another read's address) |

The `deployment` root names the deployment an expression runs under and the
platform keys it holds:

| Field | Value |
|---|---|
| `deployment.id` | the deployment id (UUID) |
| `deployment.attestation` | the deployment's attestation pubkey |
| `deployment.squadsProposer` | the Squads proposer pubkey for the calling integrator account; `null` when no step uses `squadsProposer` authority |

Use these instead of hard-coding keys: each deployment (including every local
`a4 dev` redeploy) has its own attestation key, and the proposer differs per
integrator account. Handlers get the same values as `input.deployment`.

Integer amounts are exact: `u64` arguments and parameters are decimal strings
that compare and add as integers. Address helpers: `pda("<program>",
"<pda name>", seed...)` and `ata(owner, mint)`.

## Steps

```ts
contribute: {
  scope: "participant",
  authority: { kind: "participant" },
  access: "public",
  args: { type: "object", properties: { amount: { type: "string", format: "u64" } }, required: ["amount"], additionalProperties: false },
  guards: [
    { code: "raise_not_open", expr: 'instance.state == "open"' },
    { code: "deadline_passed", expr: "now < params.deadline" },
  ],
  reads: ["presale"],
  fee: { kind: "input", amountArg: "amount", asset: { kind: "native" } },
  glue: { atas: [{ owner: "wallet", mint: "params.quoteMint" }], wrapSol: { amount: "args.amount" }, unwrapSol: false },
  build: [
    ix(meteoraPresale, "deposit", {
      args: { max_amount: "args.amount", remaining_account_info: "resources.noRemainingAccounts" },
      accounts: { presale: "instance.accounts.presale", payer: "wallet" /* ... */ },
    }),
  ],
}
```

`ix` takes IDL names as written there. TypeScript rejects an unknown
instruction, argument or account, a missing argument, and a missing account
the program can't derive (an account may be left out when the IDL marks it
optional or its address is a fixed program/sysvar or a PDA of supplied
seeds). Values stay expressions. `remainingAccounts: [{ pubkey, isWritable }]`
appends accounts the IDL doesn't name.

Guard codes become the step's typed rejection union in generated clients;
make them short, stable and specific (`deadline_passed`, not `error_1`).

Terminal states close steps without a guard. On a terminal instance, prepare
rejects every step with `instance_terminal`. When a wallet's participant
state is terminal, prepare rejects that wallet's participant steps with
`participant_terminal`; instance steps are unaffected. Both carry
`detail: { state }`, `allowedSteps` reports them the same way, and
`instance_terminal` wins when both apply. To keep a participant step
callable after its participant reached a terminal state, list that state in
one of the step's participant transitions, for example a self-loop
`{ machine: "participant", from: ["left"], to: "left" }`. Transitions do not
otherwise gate a step: from a non-terminal state, guard on
`participant.state` or `instance.state` yourself.

Arete adds the fee transfer, compute budget, ATA creation, wSOL wrap and
unwrap, and the attested memo. Never emit fee or memo instructions yourself.

## Fees

Arete includes any applicable platform fee in the prepared transaction. Authors
return the intended program instructions; they do not construct platform fee
transfers. Applications show the actual prepared fee before signing, separately
from network fees and rent, and can verify it using execution receipts.

Fee assessment is best effort. Unclassified instructions do not prevent an
otherwise valid operation from running. When Arete cannot safely establish the
operation's fee basis or collect its fee, it adds no platform fee for now.
Authors and users do not need to manage classifications or seek fee approval
for each protocol. Do not hard-code a guessed rate or charge separately in the
app; use the returned transaction and receipt.

A step named `refund` is not automatically exempt. Genuine refund treatment
depends on reviewed context; returning an arbitrary amount to an earlier sender
is not proof of a refund. Normal protocol validation, authority checks and
simulation still apply regardless of whether a platform fee is charged.

See [execution receipts](using.md#execution-receipts) for checking completed fees.

## The generated stack

Each deployment gets a generated stack (Instances, Activity and
Participants rows; see [using.md](using.md#protocol-stack-rows)). Tell it what
to track with `stack`:

```ts
stack: {
  instanceAccounts: { presale: { program: "meteoraPresale", account: "Presale" } }, // bound account -> its account type
  instanceFields: { totalDeposit: { account: "presale", field: "total_deposit" } },  // live fields read from it
  activity: [
    { step: "contribute", program: "meteoraPresale", instruction: "deposit",
      instanceAccount: "presale", walletAccount: "owner", amountArg: "max_amount" },
  ],
},
```

- Each `activity` rule maps one program instruction to a step, by IDL
  names: `instanceAccount` is the instruction account that identifies the
  instance (it becomes `activity_account` on Activity rows), `walletAccount`
  the acting wallet, and `amountArg` the argument summed into
  `<step>_amount` / `<step>_protocol_amount`.
- Several steps may list the same instruction; every rule for one
  instruction must name the same `instanceAccount`, `walletAccount` and
  `amountArg`, and each occurrence counts once, under the step its attested
  memo names.
- Activity that bypasses Arete is counted only on accounts the instance
  owns (bound in the create step from its ephemeral keys). Point
  `instanceAccount` at such an account when bypass activity matters; a
  wallet, role or shared account gets attested activity only.

## Handlers (escape hatch)

```ts
create: {
  /* ... */
  handler: async (input, ctx) => {
    const presale = await ctx.solana.account(input.instance?.accounts.presale ?? "", presaleDecoder);
    return wirePreparedOperation(/* instructions built with @programs/<name> builders */);
  },
}
```

- The `ctx` methods are the only data path: `solana.account`,
  `solana.accounts`, `solana.tokenAccountsByOwner` (owner must be the wallet
  or a bound instance account) and `stacks.view` (advisory only).
- Build instructions with the installed program module's builders
  (`import { depositInstruction } from "@programs/meteora-presale"`) and
  `buildInstruction` from `@usearete/sdk/program`, never hand-written byte
  offsets. Import from `@usearete/sdk/program`, not `@usearete/sdk`: the root
  entry carries the network client, which a handler bundle rejects.
- Return instructions only; no signers, fees or memos.
- Test handlers by replaying a recorded provider snapshot:
  `import { runHandler } from "@usearete/protocol/testing"`.

## Reactions

| Trigger | Fires |
|---|---|
| `at` | at a time (expression) while the machine is in given states |
| `when` | when a condition becomes true in given states |
| `onTransition` | on entering given states |
| `onConfirmed` | when a step's intent confirms |

| Action | Does |
|---|---|
| `runKeeperStep` | an Arete keeper prepares, signs and sends a permissionless step |
| `proposeSquads` | proposes an admin step to the deployment's Squads multisig |
| `emit` | publishes a named event to SSE and webhook subscriptions |

### Admin steps and Squads

A `proposeSquads` reaction proposes an admin step to the team's Squads v4
multisig with the integrator's proposer key. Arete only proposes; the team
votes and executes in Squads.

The multisig is part of the **deployment config**, never the protocol's
own: declare it as a resource and read it with `resources.<name>`. A
`squadsProposer` step whose `multisig` reads anything else (the instance,
args, params, the wallet, or a hard-coded address) fails `arete-protocol
check` with `squads-multisig-not-from-resources`, and the platform refuses
the spec on upload. A protocol never creates a multisig in its create step.

```ts
import squads from "@programs/squads";   // a4 install program squads-multisig-program --as squads --ts

const vault = 'pda("squads", "vault", resources.treasury.multisig, 0)';

resources: {
  treasury: {
    schema: {
      type: "object",
      properties: { multisig: { type: "string", format: "pubkey" } },
      required: ["multisig"],
      additionalProperties: false,
    },
    description: "The team's Squads v4 multisig.",
  },
},
steps: {
  pay: {
    scope: "instance",
    authority: { kind: "squadsProposer", multisig: "resources.treasury.multisig" },
    build: [ix(system, "transfer", {
      args: { lamports: "params.amount" },
      accounts: { fundingAccount: vault, recipientAccount: "instance.roles.recipient" },
    })],
  },
},
reactions: [{ id: "proposePayout", trigger: { kind: "onTransition", machine: "instance", from: [], to: ["requested"] },
              action: { kind: "proposeSquads", step: "pay" } }],
```

Each deployment sets the value (`[protocol.resources] treasury =
"treasury.json"`, `{ "multisig": "<address>" }`). The step acts from vault 0
unless it sets `vaultIndex` (1 to 255); the `vault` expression the
instructions use must name the same index. Give protocols that share a
multisig their own `vaultIndex` to keep their funds apart.

The multisig must include `deployment.squadsProposer` as a member with the
**Initiate** permission only (mask `1`); onboarding it is the user's job (see
[using.md](using.md)). Never give the proposer Vote or Execute, and never
make it a threshold signer. Activation refuses a deployment whose multisig
does not have the proposer. If the proposer is removed later, proposals are
parked and retried, and the instance stream emits
`arete.squads.proposerMissing`.

Use `runKeeperStep` or `proposeSquads` when no off-chain decision is needed.
When one is (resolve a market, accept an offer, approve KYC), `emit` an event,
let the role holder's backend decide, and have it prepare the role step with
its own wallet.
