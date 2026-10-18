# `@usearete/protocol`

Author, check and build Arete Protocols: declarative protocol specs with
instance and participant state machines, guarded steps, reactions and named
events, plus the reviewed handler escape hatch.

The package contains:

- **Contracts.** TypeScript models and strict validators for every public
  protocol contract (`arete.protocol-spec/v1`, the stack source, release,
  release review, build receipt, resource, provider snapshot, execution
  intent and the handler result). They mirror the Rust validators in
  `arete-hash` exactly: the same check order, error `code` and JSON Pointer
  `path`, and the same hashes. Both languages run every vector in
  [`test-vectors/protocol-v1.json`](../../test-vectors/protocol-v1.json).
- **`defineProtocol`**, the authoring API, which lowers to the canonical spec,
  with `ix` for typed instruction templates of installed programs and
  `builtins` for builtin programs.
- **The handler ABI** (`ProtocolHandlerV1`, `ProtocolContextV1`,
  `PrepareInputV1`) and `wirePreparedOperation`.
- **Events and webhooks**: event payload types and `verifyWebhook`.
- **`@usearete/protocol/testing`**: replay a recorded provider snapshot into a
  handler.
- **The `arete-protocol` CLI**: `check`, `build`, and `serve-handlers`, the
  local handler host for development.

## Trust model

Most protocols need no code. A step's `build` is a list of instruction
templates whose args and accounts are expressions; the platform resolves
them itself.

Some programs read trailing accounts that their IDL does not name (Anchor's
`remaining_accounts`). A template instruction lists those in
`remainingAccounts`, in order. They are appended after the IDL's accounts.
Each `pubkey` is an account expression, checked like the named accounts.
Remaining accounts never sign, so only `isSigner: false` is accepted.

```ts
build: [
  ix(meteoraPresale, "creator_withdraw", {
    args: { remaining_accounts_info: "resources.noRemainingAccounts" },
    accounts: { presale: "instance.accounts.presale", owner: "instance.accounts.vault" /* … */ },
    remainingAccounts: [
      { pubkey: "instance.accounts.quoteVault", isWritable: true },
      { pubkey: "params.quoteMint" }, // isWritable defaults to false
    ],
  }),
],
```

In the canonical spec each entry is `{ pubkey, isWritable, isSigner }`. The
field is omitted when the list is empty.

A step that a template can't express may name a **handler**. Handlers are
the escape hatch, and they are deliberately narrow:

- Handlers run only after human review. A release runs on a protocol host
  only when its bundle hash matches and a review signature over the release
  hash verifies against review keys the host trusts
  (`verifyReleaseReview` checks the same signature).
- A handler reaches data **only** through the typed `ctx`: Solana account
  reads, token accounts by owner, and stack views. It has no network,
  filesystem, clock, randomness, environment or secrets. Every `ctx` call is
  recorded as the provider snapshot.
- A handler returns portable instructions only. It never returns signers,
  fees, memos or compiled messages; trusted platform code adds those, and
  rejects programs and signers the step did not declare.
- API guards are for UX. Programs enforce money.

The static checker in this package (`checkHandlerSource`, run by
`arete-protocol build`) is author feedback, not a sandbox: it rejects direct
use of `fetch`, `WebSocket`, `process`, `eval`, `new Function`, dynamic
`import()`, `require` and Node built-ins so you learn early what review and
the host will refuse.

Running untrusted, self-service handler code (WASM or another isolation
sandbox) is deferred; until then every handler is reviewed.

Deployments carry a **trust tier** (`TrustTier`):

- `reviewed`: a reviewer signed the release. Required for any release with
  handlers and for every public deployment.
- `unreviewed`: no review. Only a template-only release on a private
  deployment may run at this tier, so no author code runs unreviewed. A hosted
  service may let the owner activate such a deployment without an operator.

A review document always attests `reviewed`; `unreviewed` is the absence of a
review, so `validateProtocolReleaseReview` rejects it.

## Authoring

Install the programs the protocol uses, with their TypeScript SDKs:

```sh
a4 install program meteora-presale --ts                        # from the registry
a4 install program ./idl/my_program.json --as my-program --ts  # from a local IDL
```

Then import them. `a4 install` maps `@programs/*` in `tsconfig.json` to the
installed programs:

```ts
import { builtins, defineProtocol, ix } from "@usearete/protocol";
import meteoraPresale from "@programs/meteora-presale";

export default defineProtocol({
  name: "tokenRaise",
  version: "1.0.0",
  instanceVisibility: "account",
  programs: { meteoraPresale, ...builtins("system", "splToken") },
  params: { deadline: "unixTimestamp", softCap: "u64" },
  machines: { instance: { initial: "open", states: { open: "recorded", closed: { kind: "derived", terminal: true } } } },
  steps: {
    contribute: {
      scope: "participant",
      authority: "participant",
      args: { amount: "u64" },
      guards: { deadline_passed: "now < params.deadline" },
      build: [
        ix(meteoraPresale, "deposit", {
          args: { max_amount: "args.amount", remaining_account_info: "resources.noRemainingAccounts" },
          accounts: { presale: "instance.accounts.presale", payer: "wallet" /* … */ },
        }),
      ],
    },
  },
});
```

The definition never restates a hash, a program id or where an IDL comes
from. Handlers import builders from the same `@programs/<alias>` module.

Terminal states close steps without a guard. Prepare rejects every step of a
terminal instance with `instance_terminal`, and the participant steps of a
wallet whose participant state is terminal with `participant_terminal`
(instance steps are unaffected); both carry `detail: { state }`,
`instance_terminal` wins when both apply, and `allowedSteps` reports them the
same way. A participant step stays callable from a terminal participant state
only when one of its participant transitions lists that state in `from`, for
example the self-loop `{ machine: "participant", from: ["left"], to: "left" }`
(a self-loop is not an exit, so the checker accepts it).

`lowerProtocol(definition)` returns `{ spec, handlers }`. It fills every
default so the output is canonical: empty collections, `access: "secret"` on
create and role steps and `"public"` on other wallet steps (none on keeper and
Squads steps), `ix` as an alias for `instruction`, and a step with `handler: fn`
becomes `{ kind: "handler", export, programs }`. Object-shorthand args and
params (`{ amount: "u64" }`) make every property required, sorted.
`RejectionCode<typeof protocol, "contribute">` is the union of that step's
guard codes.

### Programs

`programs` takes installed program modules and `builtins(...)` entries
(`builtins("system", "splToken")` is
`{ system: { builtin: "system" }, splToken: { builtin: "splToken" } }`).
A module lowers to `{ kind: "programSpec", programId, programSpecHash }`,
plus `programReleaseHash` when it was installed from the registry. The raw
`{ programId, programSpecHash, programReleaseHash? }` and `{ builtin }`
forms are still accepted.

`a4 install program --ts` generates the module. Besides its `programId`
and `programSpecHash`, its default export carries two plain properties, the
only ones this package reads (it never imports the SDK runtime). The module
is detected by `programInstall`, not by class:

```ts
programInstall: {
  alias: "meteora-presale",                 // the arete.toml dependency alias
  programReleaseHash?: "arete:h1:program-release:sha256:…", // registry installs only
  source:
    | { kind: "registry", registry: "meteora-presale", version: "1.2.3" } // exact installed version
    | { kind: "idl", idl: "idl/my_program.json" },                       // project-relative
}
instructionShapes: {
  creator_withdraw: {                        // IDL instruction name
    args: ["remaining_accounts_info"],       // IDL argument names
    accounts: {                              // IDL account names, in order
      presale: { optional: false, derive: null },
      presale_authority: { optional: false, derive: [] },
      // …
    },
  },
}
```

`derive` is `null` for an account a template must supply, otherwise the
accounts its address derives from (`[]` for a fixed address or a PDA of
literal and argument seeds).

`ix(program, instruction, { args, accounts, remainingAccounts? })` returns a
template instruction. Names are IDL names, as written there. TypeScript
rejects an unknown instruction, argument or account, a missing argument, and
a missing account the program can't derive. An account may be left out when
the IDL marks it optional, or when its address is derivable: a fixed address
(a program or sysvar), or a PDA whose account seeds are supplied or
derivable in turn. Values stay expressions.

The template's `program` is the alias the module has in `programs`;
`defineProtocol` refuses a module that isn't there. When the protocol is
lowered, every template of an installed program (an `ix` one, or a plain
object naming the alias) is checked against the module's runtime
`instructionShapes`, which catches plain JavaScript and stale declarations,
including a left-out PDA whose seed accounts are missing.

The generated stack reads each installed program from where it was
installed: a registry module as `{ registry, version: "=<exact>" }`, an IDL
module as `{ idl: "./idl/<file>" }`, with the build copying the IDL to
`idl/<file>` next to `protocol-stack-source.json`. A program given by raw
hash gets the generator's default (`{ registry: <kebab-case alias> }`).

`arete-protocol check` and `build` read `arete.lock` next to the nearest
`arete.toml`. Each installed module's `programSpecHash`, and its
`programReleaseHash` when the lock has one, must match the lock entry for
its arete.toml alias; otherwise the build fails, naming the program, with
the fix: run `a4 install` (or `a4 update program <alias>`) and rebuild.
Without an `arete.lock` nothing is checked and the CLI prints a warning.

[`fixtures/token-raise/protocol.ts`](fixtures/token-raise/protocol.ts)
lowers to [`token-raise.protocol-spec.json`](../../test-vectors/protocol/token-raise.protocol-spec.json)
exactly.

## Handlers

```ts
import { wirePreparedOperation, type PrepareInputV1, type ProtocolContextV1 } from "@usearete/protocol";

export async function createLaunch(input: PrepareInputV1<CreateArgs>, ctx: ProtocolContextV1) {
  const mint = await ctx.solana.account(input.args.quoteMint);
  // … build instructions with a generated program SDK …
  return wirePreparedOperation([{ name: "createLaunch", instructions }], {
    programs: ["meteoraPresale", "system"],
    input,
  });
}
```

`wirePreparedOperation` takes core-SDK `BuiltInstruction`s and returns the
`arete.protocol-handler/v1` wire object. It rejects, with stable
`WireOperationError` codes: any `signers` property, empty flows or
transactions, programs outside the step's declared `programs`, signer keys
other than the wallet, the intent's ephemeral keys and declared extra
signers, and more than 8 transactions, 64 instructions per transaction, 256
accounts per instruction or 10 KiB of instruction data.

Test handlers against recorded snapshots:

```ts
import { runHandler } from "@usearete/protocol/testing";

const operation = await runHandler(createLaunch, input, recordedCalls);
```

`createReplayContext(calls)` replays the ordered `ctx` call log and throws
`ReplayDivergenceError` on the first call whose method or canonical
arguments differ, or when calls are left over.

## CLI

```sh
arete-protocol check [protocol.ts]
arete-protocol build [protocol.ts] --out .arete/protocol [--server-release]
```

The entry module default-exports `defineProtocol(...)`. `check` lowers and
validates the spec and runs the expression reference and static checks.
`build` also bundles the handlers (only when a step has one) with pinned
esbuild into one deterministic ESM file and writes, as canonical JSON with
no trailing newline:

| File | Contents |
|---|---|
| `protocol-spec.json` | `arete.protocol-spec/v1` |
| `handlers.mjs` | the handler bundle, exporting each handler by its export name (absent for template-only protocols) |
| `protocol-stack-source.json` | `arete.protocol-stack-source/v1`, interim profile |
| `protocol-release.json` | `arete.protocol-release/v1` |
| `build-receipt.json` | `arete.protocol-build-receipt/v1`: source-tree, lockfile, dependency and compiler identities |
| `build-sources.json` | local only, never uploaded: each project source and its SHA-256, the files behind the receipt's `sourceTreeSha256`. `a4 protocol deploy --no-build` reads it to refuse a stale build and name the changed files |
| `machine-map.json` | states, edges and steps for UIs |
| `anatomy.json` | each step's instruction slots, with the glue and runtime slots the platform injects |
| `idl/<file>` | the IDL of each program installed from a local IDL, which the stack source names |

Artifacts contain no wall time and no machine paths; paths are relative to
the project root (`--root`, default the entry's directory). Two clean
builds of the same project are byte-identical. The bundle rejects
unresolved imports, dynamic imports and Node built-ins.

esbuild is a dependency of this package (pinned at 0.28.2), so a plain
`npm install` is enough: the project needs no esbuild of its own, and a
`file:` or linked install of `@usearete/protocol` works without
`--install-links`. The bundle records no module paths, so its bytes (and
the handler bundle hash) do not depend on where the project or its
packages are checked out, or on whether they are copies or symlinks.

When several steps list the same instruction in `stack.activity` (two steps
that both pay through one transfer, say), the generated stack counts each
occurrence once. Without the attested memo those steps are read as one
group (`close_or_open_count`); with it, a participant's per-step amounts
follow the step the memo names. Every rule for one instruction must name
the same `instanceAccount`, `walletAccount` and `amountArg`.

### Uploading a build

`build` ends by printing the **upload set**: `protocol-spec.json`, plus
`handlers.mjs` when a step has a handler. A hosted service takes exactly those
files (for example through `a4 protocol deploy`) and derives the stack
source, the release and a receipt itself with `deriveServerRelease` (Rust:
`arete_hash::protocol::derive_server_release`). The server-derived release
hash differs from the local `protocol-release.json` hash only through:

- the compiler identity: `arete-protocol-server` at
  `SERVER_DERIVATION_VERSION`, instead of this package and its esbuild;
- the stack source: the server's generator identity and the generator's
  default program sources (where the local build reads each installed
  program is local only).

`--server-release` prints the hash the server will derive, so you can check
it before uploading and quote it to a reviewer. Reviews of uploaded releases
sign that server-derived hash. The shared vectors in
`test-vectors/protocol-release/` keep the TypeScript and Rust derivations
identical.

The expression check covers the stack-source expression syntax. It confirms
each reference is in scope at its use site (`params.*`, `args.*` on steps,
`wallet`, `instance.accounts.<bound>`, `instance.roles.<role>`,
`ephemeral.<declared>`, `reads.<declared>.*`, `deployment.*`, …) and
reports `unknown-reference` with a span. It is author feedback only; the
platform type-checks every expression.

### Local handler host: `serve-handlers`

```sh
ARETE_HANDLER_HOST_TOKEN=… arete-protocol serve-handlers \
  --review-key <keyId>=<base58 pubkey> --ctx-url http://127.0.0.1:<port> \
  [--listen 127.0.0.1:0] [--token-file <path>] [--timeout-ms 10000]
```

`a4 dev` starts this for you when a protocol has handlers. It runs only
reviewed handler bundles: a release whose review signature doesn't verify
against a `--review-key` is refused. Handlers run with the same ABI, the same
`ctx` methods and the same result validation as on a hosted protocol host.

- `--review-key` is repeatable and names the review keys the host trusts.
- `--ctx-url` is the handler context API's base URL; handler `ctx` calls go back
  to it.
- The token comes from `--token-file` (one trailing newline is stripped) or
  from `ARETE_HANDLER_HOST_TOKEN`. Every request must carry it in the
  `x-arete-handler-host-token` header.
- `--listen` only accepts loopback addresses.
- `--timeout-ms` bounds each handler invocation (10 s by default).

When the host is listening it prints a single line to stdout:

```json
{"event":"ready","schema":"arete.handler-host-ready/v1","url":"http://127.0.0.1:<port>"}
```

Logs go to stderr as JSON lines. SIGINT and SIGTERM let in-flight requests
finish, then the host exits.

The host is for development only. Handlers run in the host's own Node
process, so a handler that blocks the event loop synchronously is not
preempted by the timeout.

### Platform keys: `deployment`

The `deployment` root names the deployment an expression runs under and the
platform keys it holds. It is valid in every expression, including the
create step:

| Field | Type | Value |
|---|---|---|
| `deployment.id` | string | the deployment id (UUID) |
| `deployment.attestation` | pubkey | the deployment's attestation key |
| `deployment.squadsProposer` | pubkey or `null` | the Squads proposer for the calling integrator account; `null` when no step uses `squadsProposer` authority |

Handlers receive the same values as `input.deployment`
(`{ id, attestation, squadsProposer }`). A create step that makes a Squads
multisig adds `deployment.squadsProposer` as a member with the Initiate
permission only (mask `1`), so the platform can propose admin steps but
never vote or execute.

## Webhooks

An `emit` reaction publishes a named event. Payloads carry ids and states
only (`ProtocolEventPayloadV1`):

```json
{ "id": "…", "event": "raise.succeeded", "deploymentId": "…", "instanceId": "…",
  "participantId": null, "state": "succeeded", "occurredAt": "2026-10-13T00:00:00Z" }
```

Each delivery carries three headers:

| Header | Value |
|---|---|
| `arete-webhook-id` | the delivery id, stable across retries |
| `arete-webhook-timestamp` | Unix seconds when the delivery was signed |
| `arete-webhook-signature` | `v1=<hex>`: lowercase hex HMAC-SHA256 over `${timestamp}.${body}` keyed with the endpoint secret. Several space-separated entries may appear while a secret rotates; any match is enough |

```ts
import { verifyWebhook } from "@usearete/protocol";

const { payload } = verifyWebhook(secret, rawBody, request.headers);
```

Pass the raw body exactly as received. Verification compares in constant
time and rejects timestamps more than 300 seconds from now (`options.now`
and `options.toleranceSeconds` override). It uses `@noble/hashes`, so it
runs in browsers, Node and edge runtimes.

## Development

The published package depends on `@usearete/hash` by version range; in this
repository the lockfile links `../hash`, so build it first:

```sh
(cd ../hash && npm ci && npm run build)
npm ci
npm test && npm run typecheck && npm run lint && npm run build
```

Releases bump the range with each `@usearete/hash` release, and the release
workflow publishes `@usearete/hash` before this package.
