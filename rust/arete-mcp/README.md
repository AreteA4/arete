# arete-mcp

MCP (Model Context Protocol) server that wraps Arete streams for AI agent
integration. Lets Claude, GPT, and other MCP-compatible agents discover what
stacks and programs exist, query the curated knowledge layer, connect to a
Arete stack, subscribe to views, and query cached entities — using the same
primitives a human operator uses through `a4 explore`, `a4 know`, and
`a4 stream`.

**The server ships inside the `a4` CLI as `a4 mcp`.** It speaks MCP over
stdio; stdout carries MCP frames only and logs go to stderr. This crate is a
library (`arete_mcp::serve_stdio()`) that the `a4` binary links; you do not
install it separately.

> **Removed:** the standalone `a4-mcp` binary no longer exists as of this
> version, and the `@usearete/mcp` npm package is deprecated. Update any MCP
> config that runs `a4-mcp` or `npx -y @usearete/mcp` to run `a4 mcp` instead
> (or `npx -y @usearete/a4 mcp` when `a4` is not on PATH); `a4 init` does this
> for you.

## Install

Install `a4`:

```bash
curl -fsSL https://arete.run/install.sh | sh    # or: npx @usearete/a4 install
```

Then check the server answers:

```bash
a4 mcp --help
```

## Configure your agent

`a4 init` writes the MCP config for the agents it detects in the project
(Claude Code, Cursor, Codex, OpenCode, Gemini CLI, Copilot CLI, ...), plus
`arete.toml`, `AGENTS.md` and skills. It is idempotent; `--global` targets
the per-user config instead of the project:

```bash
a4 init -y            # project scope
a4 init -y --global   # user scope
a4 doctor             # verify what init wrote
```

If you prefer to edit the config by hand, the shapes are below. Every client
resolves `command` against the PATH its process inherited at launch; GUI apps
started from a desktop launcher may need a full restart after installing
`a4`, or an absolute path such as `/home/you/.local/bin/a4`.

### Claude Code / Copilot CLI (`.mcp.json`)

```json
{
  "mcpServers": {
    "arete": { "type": "stdio", "command": "a4", "args": ["--profile", "agent", "mcp"] },
    "arete-docs": { "type": "http", "url": "https://docs.arete.run/mcp" }
  }
}
```

Or via the CLI: `claude mcp add --transport stdio arete --scope project -- a4 mcp`.

### OpenCode (`opencode.json`)

```json
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "arete": { "type": "local", "command": ["a4", "--profile", "agent", "mcp"], "enabled": true },
    "arete-docs": { "type": "remote", "url": "https://docs.arete.run/mcp", "enabled": true }
  }
}
```

### Cursor (`.cursor/mcp.json`), Claude Desktop, Windsurf

```json
{
  "mcpServers": {
    "arete": { "command": "a4", "args": ["--profile", "agent", "mcp"] }
  }
}
```

### VS Code (`.vscode/mcp.json`)

```json
{
  "servers": {
    "arete": { "command": "a4", "args": ["--profile", "agent", "mcp"] }
  }
}
```

### Zed (`settings.json`)

```json
{
  "context_servers": {
    "arete": { "command": "a4", "args": ["--profile", "agent", "mcp"], "env": {} }
  }
}
```

### Continue.dev (`config.yaml`)

```yaml
mcpServers:
  - name: arete
    type: stdio
    command: a4
    args: ["--profile", "agent", "mcp"]
```

## Authentication

Run `a4 auth signup` once for a restricted agent credential. Human keys from
https://arete.run/keys are stored separately with `a4 auth login --profile
human --key <a4_sk_...>`. Generated MCP config runs `a4 --profile agent mcp`,
which resolves the agent profile even if a human `ARETE_API_KEY` is ambient.
Never pass a key as a tool-call argument. See [Authentication](#authentication-1)
under the tool reference for the exact precedence.

## Tool reference

A typical session starts with discovery — `search_knowledge` to find which
protocols serve an intent (API key required), `explore_stacks` to see what
exists, `explore_stack_schema` to get the view ids. To read a view's current
entities once, it then calls `read_view`, which connects, waits for the
snapshot and disconnects in one call. To follow updates it calls `connect` once,
then `subscribe`, then queries the cache via `get_entity` / `list_entities` /
`get_recent` / `query_entities`. Everything from `connect` onward is stateful.

| Group | Tools | Auth |
|-------|-------|------|
| Discovery | `explore_stacks`, `explore_stack`, `explore_stack_schema`, `explore_programs`, `explore_program`, `explore_protocol`, `resolve_artifact` | None required; a resolved key widens `explore_stacks` |
| One-shot read | `read_view` | Key resolved automatically; required for hosted stacks |
| Knowledge | `search_knowledge`, `get_protocol`, `get_program_knowledge`, `get_recipe`, `list_concepts` | **API key required** — fails up front without one |
| Connection | `connect`, `disconnect`, `list_connections` | Key resolved automatically; required for hosted stacks |
| Subscription | `subscribe`, `unsubscribe`, `list_subscriptions` | Per connection |
| Cache | `get_entity`, `list_entities`, `get_recent`, `query_entities` | Per subscription |
| Protocols | `protocol_check`, `protocol_actions`, `protocol_instances`, `protocol_instance`, `protocol_prepare`, `protocol_submit`, `protocol_intent`, `protocol_receipt`, `protocol_squads_proposer`, `protocol_squads_status`, `squads_status`, `squads_attach`, `protocol_execute` (only with a configured signer) | Key resolved automatically; sent only to Arete origins |
| Health | `ping` | None |

### Which tools are listed

At startup `a4 mcp` asks the Arete API which command groups the account may
use (`commandGroups` on `GET /api/auth/me`, with the resolved key) and lists
only their tools. `ping`, `account_status` and `create_claim_link` are
always listed.

| Group | Tools |
|-------|-------|
| `sandboxes` | `sandbox_*` (also need `ARETE_SANDBOX_ID`) |
| `streams` | connection, subscription and cache tools, and `read_view` |
| `registry` | `explore_*`, `resolve_artifact`, the catalog and knowledge tools, `describe_sdk` |
| `protocols` | `protocol_*`, `squads_*` |
| `dev` | `dev_*` |

With no key, when the API can't be reached, or when it reports no groups,
every tool is listed as before. Hiding is never the gate: the API refuses
calls the account may not make, whether the tool is listed or not.

### Discovery

Read-only lookups against the public `/api/registry/*` endpoints. No signup
required: public stacks and programs are visible to unauthenticated callers. When
a key happens to resolve (see [Authentication](#authentication)), it is attached
so `explore_stacks` also returns global stacks — but a missing key is never an
error here.

- `explore_stacks({ fields?, full? })` — stacks in the registry. The
  `websocket_url` in each entry is what `connect` takes; `entities` tells you
  what to look for in the schema. Brief by default (`name`, `description`,
  `websocket_url`, `entities`, `visibility`, `serviceClass`,
  `websocket_auth.required`); `full: true` returns every field with the registry's
  own key spelling. The brief and `fields` forms report these keys, and every field
  inside `websocket_auth` and `http_auth`, as snake_case whichever spelling the
  registry sends. `fields` accepts either spelling.
- `explore_stack({ stack, summary?, views?, full? })` — one stack from its pinned
  install descriptor. By default a compact summary: entities with their view ids,
  program SDKs, endpoints, and auth requirements (key classes, scopes, origin-bound
  browser keys, transaction entitlement). `views` returns only those views with
  their entity schemas; `full: true` returns the whole descriptor `a4 install`
  consumes.
- `explore_stack_schema({ stack })` — entity and view schema: field paths, types,
  primary keys, and the `<EntityName>/<view>` ids `subscribe` accepts. Use this
  instead of guessing a view id from the template. When the stack is published
  in the catalog, fields carry a curated `description` and entities and views a
  `summary`, with usage guidance such as which of two similar fields a live UI
  should show (`explore_stack` summaries list them as `fieldDescriptions`, and
  its `views` attach them to fields). `knowledge` names the source document.
  Descriptions are attached only when the knowledge was published for the
  StackManifest the registry serves for the stack. Stacks without a catalog
  entry return their schema unchanged.
- `explore_programs({ fields?, full? })` — standalone Solana programs
  installable independent of any stack. Release and spec hashes are dropped
  unless `full: true`.
- `explore_program({ program, operationId?, sections?, full? })` — one program
  from its pinned install descriptor. By default a compact summary: identity,
  account/instruction/event/type names, semantic SDK operations, transports.
  `operationId` returns one operation (generated paths, input, required and
  derived accounts, signers, transaction count, errors, transport, usage);
  `sections` returns `accounts`, `events`, `instructions`, `operations` or
  `types` in detail; `full: true` returns the whole descriptor. Semantic
  operations come from the knowledge surface and need an API key.
- `explore_protocol({ protocol, full? })` — one protocol package from its
  pinned install descriptor. By default a summary: release and spec identity,
  the deployment its client calls (`deploymentId`, `access`), the instance and
  participant machines, each step with its scope, authority, whether a client
  can call it, its `access` (`public` steps can be prepared with a publishable
  key, `secret` steps need a secret key), args and guard (rejection) codes,
  the access and state (`instance_terminal`, `participant_terminal`) rejection
  codes, events, metadata fields, and the stacks and
  programs it hoists; `full: true` returns the whole descriptor. The same
  summary backs `a4 explore protocol <ref> --json` (`arete_mcp::protocol`).
- `resolve_artifact({ kind, hash })` — fetch a content-addressed artifact.
  `kind` is one of `program-spec`, `live-spec`, `stack-manifest`,
  `protocol-spec`; the hash comes from `explore_stack`, `explore_program` or
  `explore_protocol`.

Summaries, sections, views and operations are cut from the full descriptor on
the client (`arete_mcp::descriptor`, shared with `a4 explore`). `full: true`
bodies are the registry's JSON, passed through unchanged. Any tool result over
512 KB is refused rather than truncated. Install descriptors are read up to
8 MB so the summary, views and WebSocket URL of a stack with large programs
still come back; only `full: true` on such a descriptor is refused, with the
arguments that return less. Past 8 MB, `explore_stack_schema` still lists the
view ids and `read_view` accepts the stack's `url` (`websocket_url` from
`explore_stacks`).

**Key casing is not uniform.** `explore_stacks` and `explore_stack_schema` return
snake_case (`websocket_url`, `stack_name`, `primary_keys`, `rust_type`), except
that `explore_stacks` with `full: true` passes through the registry's own spelling,
which may be camelCase (`websocketUrl`, `websocketAuth`).
`explore_stack`, `explore_programs` and `explore_program` return camelCase
(`websocketUrl`, `installName`, `programSpecHash`, `stackManifestHash`) — the CLI
deserializes those three with `deny_unknown_fields`, so their shape is pinned.
`resolve_artifact` returns a camelCase envelope (`artifactHash`, `artifactVersion`,
`kind`, `payload`) wrapping a stored payload that may be snake_case inside, such as an
embedded IDL's `program_id` and `is_signer`. Separately, `a4 explore --json` is
camelCase throughout. Read the keys you actually receive rather than assuming one
convention.

These tools cover discovery only. SDK generation (`a4 install`) and transaction
construction live in the CLI and the generated SDKs, not here.

### Knowledge layer

Read-only lookups against `/api/registry/knowledge/*` — the curated
knowledge layer that maps an intent ("monitor swaps", "execute through a
multisig") to protocols, programs, stacks, and recipes, and says what the
platform can do for each: `read` (fetch on-chain account state), `build`
(construct transactions), `subscribe` (stream live entities from a hosted
stack).

**Unlike the discovery tools, every knowledge tool requires an API key.** The
key resolves through the same precedence as `connect` (`ARETE_API_KEY`, then
the `a4` login), and when none resolves the call fails
immediately with an error pointing at `a4 auth login` — there is no public
subset. The same HTTPS-origin rule as discovery applies: a key is only sent to
`https://arete.run`, `https://*.arete.run`, or a loopback address.

- `search_knowledge({ query?, concept?, category?, limit?, fields?, full? })` —
  search by free-text intent (matched against concept names and synonyms
  first, then protocols/programs/recipes via full-text search) or filter by
  exact `concept` / `category` slug. At least one of the three is required.
  Each result carries `coverage: { read, build, subscribe }`. Brief by default
  (type, slug, name, protocol, summary, coverage; 10 results) with a `hint`;
  `full: true` adds `score` and `coverage_via`.
- `get_protocol({ protocol })` — one protocol by slug (e.g. `meteora-damm`):
  description, categories, links, programs with roles
  (core/periphery/deprecated), related protocols, the public stacks streaming
  its entities, and per-concept coverage. Follow `programs[].slug` into
  `get_program_knowledge`.
- `get_program_knowledge({ program, section? })` — curated annotations for one
  program by slug (e.g. `meteora-cp-amm`). `section` is `summary` (default),
  `instructions`, `accounts`, or `surface` — the ingested SDK extension surface
  (callable operations with bindings). Sections keep responses under the
  512 KiB cap; fetch only what you need.
- `get_recipe({ recipe })` — one cross-protocol recipe by slug: an ordered
  sequence of steps, each referencing a real SDK surface entry (resolved in the
  response), plus a path to example code. The catalog is growing.
- `list_concepts({ full? })` — concept and category slugs with their names.
  `full: true` adds descriptions, synonyms and related concepts. Call first to
  map a user's phrasing onto a canonical slug or to pick `search_knowledge`
  filters.

Slugs are bare identifiers, not URLs or paths; the client rejects anything with
`/`, `\`, `?`, `#`, `%`, `&`, or whitespace before a request is made. The
`surface` section is how an agent learns which typed SDK operations exist for a
program — read it instead of reading SDK source for method names. CLI mirror:
`a4 know search|protocol|program|recipe|concepts [--json]`.

### Connection management

- `connect({ url, api_key? })` — open a WebSocket to a Arete stack.
  Returns `{ connection_id, url, state, key_source }`. `api_key` is an
  **optional override** — the server resolves it automatically via the
  precedence described in [Authentication](#authentication) below. Prefer
  omitting it in agent calls.
- `disconnect({ connection_id })` — close a connection. Also drops every
  subscription bound to it.
- `list_connections()` — id, URL, current connection state.

#### Authentication

An agent should **never** pass a Arete API key as a tool-call argument
in normal operation: the key would end up in the model's context window,
chat transcript, and the JSON-RPC stdio traffic between the client and
`a4 mcp`. Instead, `a4 mcp` resolves the key itself using this precedence:

1. **Explicit `api_key` argument** on the `connect` call (override, useful
   for testing or multi-stack setups; it must match a selected built-in profile)
2. When **`ARETE_PROFILE` is selected**, that named profile of the `a4`
   login. A selected profile deliberately outranks `ARETE_API_KEY`.
3. Without a selected profile, **`ARETE_API_KEY`**, then the single `a4` login
   profile holding a key (`a4 auth signup` stores the `agent` profile,
   `a4 auth login --profile human` the `human` one).

   The login lookup honors `ARETE_API_URL` if set; otherwise falls back
   to `https://api.arete.run`.

If none of the three produces a key **and** the target stack URL is a
hosted Arete stack (`*.stack.arete.run`), `connect` fails
with a descriptive error telling the agent exactly what to do. Self-hosted
/ custom WebSocket URLs are allowed to proceed without a key because they
may not require auth at all.

The `connect` tool response includes a `key_source` field identifying
which of the three lookup paths won (`explicit_argument`,
`env:ARETE_API_KEY`, `a4-login`, or `none`).
The key itself is never included in responses or log output.

### Subscription management

- `subscribe({ connection_id, view, key?, with_snapshot? })` — subscribe to a
  view (e.g. `PumpfunToken/list` or `OreRound/latest`). Optional `key` narrows
  to a single entity. Returns a `subscription_id`. The subscription is
  multiplexed over the existing WebSocket — no extra connections are opened.
  The response also carries `next: { tool: "get_recent", arguments: {
  subscription_id, n } }`, the call that reads the new subscription.
- `unsubscribe({ subscription_id })` — cancel.
- `list_subscriptions({ connection_id? })` — list active subscriptions,
  optionally filtered by connection.

#### View naming

Views are named `<EntityName>/<mode>`. Every entity in a Arete stack
auto-generates three built-in modes:

| Mode     | Shape                                      | Best for                |
|----------|--------------------------------------------|-------------------------|
| `state`  | keyed cache of the latest state per entity | "get this specific one" |
| `list`   | ordered recent-items list (sorted `_seq`)  | "show me recent N"      |
| `append` | append-only stream of every write          | event-log consumers     |

The `state` view can legitimately be empty on a deployment even when entities
are streaming, because it only materializes when entities explicitly write
state. If a `subscribe` + wait loop returns zero entities, try
`<EntityName>/list` before assuming the stack is down.

Stacks may also declare **custom views** with non-standard suffixes (for
example `OreRound/latest` in the ore stack). Custom view names are not
discoverable via the MCP protocol — the stack author must document them out
of band, or the agent must be told by its user. If you're the author, list
them in whatever README the agent has access to.

### Reading a view once

`read_view({ stack | url, view, key?, where?, filters?, select?, limit?, timeout_secs? })`
connects, subscribes, waits for the view's snapshot, returns its entities and
disconnects; nothing outlives the call. `stack` is a bare registry reference
(`ore`) whose single WebSocket endpoint is looked up; `url` overrides it.
`where`, `filters` and `select` work as in `query_entities`. `limit` defaults to
10 (max 1000) and entities come in the view's order, so `limit: 1` on a sorted
view such as `OreRound/latest` is its current entity. Without filters the
server sends only `limit` entities. `timeout_secs` defaults to 15 (max 60). The
result is `{ view, key?, total, matching, returned, truncated, entities }`. An
unknown view, a `key` with no entity, or a timeout is an error.

### Token amounts

`explore_stack_schema` and `explore_stack` views attach `amount` to the fields
a stack scales with `ui_amount(decimals)`: `{ scale: "ui" | "raw", decimals?,
decimalsFrom?, counterpart? }`. `ui` is whole token units (raw / 10^decimals),
`raw` the integer base units the computation reads (for SOL, lamports). The
scale is read from the LiveSpec's computed fields, never guessed from names.

### Querying the cache

Each read waits up to 5 seconds for the subscription's snapshot and reports
`ready`, so a read straight after `subscribe` is not spuriously empty.

Streamed entities land in the SDK's `SharedStore`, which keeps normalized
entities separate from protocol v2 query membership. Every query tool below
reads the exact membership bound to its `subscription_id`, so keyed or
windowed subscriptions cannot leak entities from another query on the same
view. Equivalent subscriptions share one reference-counted wire subscription.

- `get_entity({ subscription_id, key })` — fetch one entity by key.
- `list_entities({ subscription_id })` — keys only (no values), to keep the
  response small even on 10k-entity views.
- `get_recent({ subscription_id, n? })` — the first `n` entities in the
  subscription's server-defined order. `n` defaults to 10 (max 1000) and
  `limit` is accepted as an alias. A malformed call reports every missing or
  invalid field at once, e.g. `{ "subscription_id": "sub_1", "n": 10 }`.
- `query_entities({ subscription_id, where?, filters?, select?, limit? })` —
  filter and project. Supports two filter inputs at once, ANDed together:

  - `where: string[]` — the same predicate DSL as `a4 stream --where`:
    - `field=value`, `field!=value`
    - `field>N`, `field>=N`, `field<N`, `field<=N`
    - `field~regex`, `field!~regex`
    - `field?` (exists), `field!?` (does not exist)
    - Nested fields use dot-paths: `user.name=alice`
  - `filters: Predicate[]` — structured form, easier for LLMs to generate
    without escaping bugs:
    ```json
    [
      { "path": "user.age", "op": "gt", "value": 18 },
      { "path": "name",     "op": "eq", "value": "alice" },
      { "path": "email",    "op": "exists" }
    ]
    ```
    `op` is one of `eq`, `not_eq`, `gt`, `gte`, `lt`, `lte`, `regex`,
    `not_regex`, `exists`, `not_exists`.
  - `select` — comma-separated dot-paths for field projection. Omit to return
    full entities. Collisions are avoided by using the full path as the key
    (e.g. `select: "a.id,b.id"` returns `{"a.id": ..., "b.id": ...}`).
  - `limit` — defaults to 100, hard-capped at 1000. Caps every response so
    the stdio transport is never asked to ship 10k entities at once.

### Cloud dev environments (`a4 dev`)

These tools act on an Arete cloud dev environment (a persistent mainnet fork
Arete hosts for the account): the project's (`.arete/dev/environment.json`
at or above the working directory, or `project`), `ARETE_DEV_ENV`, or the
one `environment` names. They never start or stop processes; `a4 dev up`
creates or wakes the project's environment and serves it locally. The account key goes only to `https://api.arete.run`, the `ARETE_API_URL`
origin, or the API your a4 login is stored for: a project file naming any
other API gets no key, and the tool fails before sending anything.

- `dev_status({ environment?, project? })` — the environment, whether `a4 dev
  up` serves it, the session's URLs and deployment, the chain's slot and
  clock, and a stack stream URL with a token.
- `dev_environments()` — the account's environments and the plan's quotas.
- `dev_wake`, `dev_sleep({ environment?, project? })`;
  `dev_delete({ confirm, … })` and `dev_reset({ confirm, … })`, where
  `confirm` repeats the environment's name.
- `dev_checkpoint({ action?: create|list|delete, checkpoint?, … })`,
  `dev_branch({ checkpoint, name, … })`, `dev_restore({ checkpoint, … })`.
- `dev_rebase({ keep?, … })`, `dev_reset_account({ account, include_owned?, … })`,
  `dev_pin({ account, include_owned?, undo?, … })`,
  `dev_follow({ account, undo?, … })` — freshness.
- `dev_fund({ account, sol?, mint?, amount?, token_program?, … })` — a SOL or
  token balance; `account` is a public key or a dev wallet name.
- `dev_warp({ to, … })` — `+2h`, Unix seconds, `slot:<n>` or `epoch:<n>`.
- `dev_scenario({ scenario, dry_run?, … })`, `dev_preview({ transaction, … })`,
  `dev_repro({ signature, output?, … })`.
- `dev_environment_events({ after?, limit?, … })` — lifecycle events.
- `dev_step({ step, instance?, args?, wallet?, project? })` — prepare on the
  project's dev deployment, sign locally with a dev wallet, submit and wait.
- `dev_instance({ instance, wallet?, project? })` and
  `dev_actions({ wallet, instance?, cursor?, project? })`.
- `dev_events({ after?, limit?, waitMs?, types?, project? })` — the running
  `a4 dev up`'s event feed since a cursor; pass the returned `cursor` as
  `after`.
- `dev_capture({ programs, from, to?, output, … })` — a capture from a Solana
  RPC node.
- File paths (`output`, a `scenario` file) are relative to the server's
  working directory: absolute paths, `..` and symlinks are refused.

### Protocols

Act on a protocol deployment: find what a wallet can do, prepare a step,
sign it outside the server, submit it. Name the deployment with `deployment`
or a package reference `protocol`; `dev: true` targets the project's dev deployment.

- `protocol_check({ dir? | spec? })` — runs Arete's upload checks (expression
  types, templates against their pinned program specs) on a built release
  without uploading it. `passed`, or the upload's `code` and `errors`, each
  with the spec `path`, `code` and `message`. Stores nothing; needs a secret
  key.
- `protocol_actions({ wallet, deployment? | protocol?, instance?, cursor? })` —
  create steps and the instances the wallet relates to, each step with
  `allowed`, `reason`, `scope`, typed `args` and `argsDependent`.
- `protocol_instances({ deployment? | protocol?, state?, cursor? })` and
  `protocol_instance({ instance, wallet? })`.
- `protocol_prepare({ step, wallet, instance?, args?, deployment? | protocol? })` —
  unsigned base64 transactions and a `summary` (programs invoked, accounts
  written, signatures still required, the wallet's simulated SOL and token
  changes when an RPC is configured, simulation errors). Guard and access
  rejections are results.
- `protocol_submit({ executionId, signedTransactions, wait?, timeoutSeconds? })`
  and `protocol_intent({ executionId })`. Both carry `receiptUrl` when Arete
  issues receipts for the deployment.
- `protocol_receipt({ signature })` — fetches the signed execution receipt of
  a transaction on Arete, checks its signature against the published receipt
  keys, and, with `ARETE_MCP_RPC_URL` or a signer `rpcUrl`, checks it against
  the transaction itself (message hash, memo, deployment, attestation
  signer). Local `a4 dev` executions get no receipt.
- `protocol_squads_proposer({ deployment? | protocol? })` and
  `protocol_squads_status({ multisig, ... })` — bring-your-own Squads v4
  multisig onboarding for steps whose authority is `squadsProposer`: the
  account's proposer (Initiate only) and whether it can propose to the
  multisig (`not_member`, `permissions_invalid`, `proposer_unfunded`) with
  its SOL balance. Arete builds no Squads transactions: `next` points at the
  dashboard or the Squads app, or, in `a4 dev`, at
  `a4 dev squads add-proposer`.
- `squads_status({ multisig? })` and `squads_attach({ multisig, label? })` —
  the account's Squads multisigs. A production deployment of a protocol with
  Squads steps is refused (`protocol-squads-setup-required`) unless its
  multisig is an `active` attachment. Answers lead with the dashboard
  (https://app.arete.run/dashboard/squads, any browser wallet, Ledger
  included) and the Squads-app steps (Members → Add member → the proposer →
  Initiate only → approve and execute). Arete builds no Squads transactions:
  code that creates a multisig or adds the proposer uses the squads-v4
  program SDK (`a4 install program squads-v4`) and its Arete setup helpers.
- `protocol_execute(...)` — listed only when a signer is configured in the
  `[signer]` table of `~/.arete/mcp.toml` (or the file
  `ARETE_MCP_SIGNER_CONFIG` names): prepares, checks the signer's policy
  (`deployments`, `maxSolPerTransaction`, optional `programs`,
  `maxTokenAmounts`, `clusters`), signs, submits and waits. A refusal names
  the rule. The keypair never appears in output, errors or logs. The caps
  hold per transaction and in total over the step; a transaction that uses
  an account an earlier transaction of the step writes is refused, since it
  is simulated against state from before that write. `programs` covers CPI
  callees (from the simulation's inner instructions; unresolved ones are
  refused). Without `maxTokenAmounts` no token may leave the
  signer; wrapped SOL and the lamports of a token account closed to another
  account count against `maxSolPerTransaction`. Token approvals, authority
  changes, System `Assign`/`AssignWithSeed`/`Allocate`/`AllocateWithSeed`
  of the signer or an account derived from it, nonce
  and stake authority changes or withdrawals, and upgradeable-loader
  instructions the signer takes part in are refused (`rule: "authority"`),
  top-level always and inner when the RPC's simulation returns inner
  instructions. A transaction writing more accounts than the simulation
  reports on (64) is refused as unknown. With `dev: true` the signer signs
  only for the dev environment's own deployment, a `project`
  argument must be at or below the server's working directory, the signer
  is funded in the dev environment when it has no SOL, and simulation uses
  the signer's `rpcUrl` unless the policy lists `dev` in `clusters`
  (the local RPC endpoint of a running `a4 dev up` is used when the signer
  has none).

### Sandbox wallets

A sandbox is a hosted wallet whose owner decides which programs and
instructions it may use, which actions an agent may run and how much may
leave it. Arete builds, checks, signs and sends every transaction: the agent
names an action and its arguments, and never sees a private key, a raw
transaction to sign or an RPC URL. Owner operations (rules, caps, grants,
pause, revoke) live in the dashboard and are not reachable from these tools.

**Auth.** Listed only when `ARETE_SANDBOX_ID` names a sandbox. The agent
uses its own account's secret API key (`ARETE_API_KEY`, or its a4 login) to
mint a short-lived sandbox token at `POST /ws/sessions` with
`{ "targetKind": "sandbox", "targetId": "<sandbox id>" }`. The mint succeeds
only while the sandbox's owner grants this agent access; the token carries a
`sandbox:<action>` scope per granted action and is the only credential the
sandbox routes see. The server mints it on first use and again when it
expires; a revoked grant fails closed. Neither the key nor the token is ever
returned in a tool result.

**Agent loop:** discover → preview → invoke.

- `sandbox_status()` — wallet address, SOL and token balances (raw
  amounts), the actions this agent was granted, and, under version 2 rules,
  caps (lamports and per mint, per action and per window) with what the
  window has spent. Start here.
- `sandbox_rules()` — the allowed programs and, for each, the allowed
  instructions with their argument types and accounts.
- `sandbox_derive({ kind: "ata" | "pda", ... })` — a program-derived
  address (`programId`, `seeds`), computed by this server with no network
  call, or an associated token account (`mint`, optional `owner`,
  `tokenProgram`), which the API derives under the mint's token program as
  the chain reports it. PDA seeds are `{ kind, value }` with `kind`
  `string`, `pubkey`, `u8`, `u16`, `u32` or `u64` (integers as numbers or
  decimal strings, little-endian), or `{ kind: "bytes", hex }`: at most 15
  seeds of at most 32 bytes each.
- `sandbox_preview({ action, args })` — a dry run of the same action
  `sandbox_invoke` takes: builds the transaction and runs every check
  without signing or recording anything. Returns `pass` with what the wallet
  would lose and the transaction it built, or the refusal. `withdraw` has no
  dry run and is refused with `sandbox-preview-unavailable`.
- `sandbox_invoke({ action, args, idempotencyKey? })` — runs one action.
  This is the only tool that moves funds; there are no per-action tools.
- `sandbox_action({ actionId | idempotencyKey })` — one action's status,
  for looking up a timed-out call instead of retrying it.
- `sandbox_history({ cursor?, limit?, status?, program?, mint?, since?, includeRaw? })`
  — read-only. Every action the wallet ran, newest first: `id`, `createdAt`,
  `status`, `signature`, `slot`, the `programs` and decoded `instructions`,
  `solDelta`, `tokenDeltas` (`mint`, raw `delta`, `decimals`) and the
  `accounts` touched. Page with `cursor` (the previous `nextCursor`) and
  `limit` (1 to 100, default 20). The filters are applied by this MCP
  server to the pages it reads: a filtered read scans up to 10
  pages until `limit` items match, and `nextCursor` carries on after the
  last page scanned. Each item's `raw` parsed transaction (with pre and post
  balances) is left out unless `includeRaw` is true.
- `sandbox_holdings({ includeRaw? })` — read-only. SOL, tokens (raw
  `amount`, `decimals`, and `acquiredBySandbox` for tokens the sandbox
  bought, which it may always sell) and program `positions` as labeled raw
  accounts, with `asOfSlot`.

The two reads call `GET /api/sandbox/v1/sandboxes/<id>/actions?limit&cursor`
and `GET /api/sandbox/v1/sandboxes/<id>/holdings` with the same sandbox
token. Like the other `sandbox_*` tools they are listed only with a sandbox
configured; they run no action.

**Rules v3.** Under version 3 rules the owner turns each supported program
on or off, may narrow its instructions (else the registry's safe set) and
may set a per-program spend limit (`lamports` over `windowSeconds`). Funds
may leave the enabled programs only to the owner's wallet, the owner's
`transferAllowlist` or the sandbox's own accounts, and tokens the sandbox
bought may always be sold, at a loss too. There are no global per-action or
per-window caps under version 3. `a4 sandbox rules validate <file>` checks a
Rules v3 document locally.

Actions (an action the owner did not grant is refused before anything is
sent, with code `sandbox-action-not-granted` and rule `session-grant`):

| `action`   | `args` |
| ---------- | ------ |
| `invoke`   | `{ instructions: [{ program, instruction, args?, accounts?, remainingAccounts?: [{ address, writable? }] }], computeUnitLimit?, computeUnitPriceMicroLamports?, transactionFormat? }` — 1 to 8 named instructions, exactly as `sandbox_rules` lists them; the wallet is the only signer |
| `withdraw` | `{ lamports, transactionFormat? }` — SOL to the owner's withdraw address, the only destination a withdraw can have |
| `swap`     | `{ inputMint, outputMint, amount, slippageBps?, computeUnitLimit?, computeUnitPriceMicroLamports? }` — exact input, in the input mint's smallest unit, through the venues the owner allows |

Pass u64 amounts as decimal strings. Every `sandbox_invoke` has an
idempotency key (yours, or generated); it is returned on success and on
error, and a key never runs twice. After a timeout, look the action up with
`sandbox_action` and retry only with the same key.

Actions build Solana v1 transactions (SIMD-0385) by default: up to 4096
bytes and 64 accounts, no address lookup tables, with the compute budget in
the message itself. `transactionFormat: "v0"` builds the older format
instead (up to 1232 bytes). `computeUnitPriceMicroLamports` is a per-unit
price in both; a v1 transaction pays the same total as its priority fee.
A transaction too large for its format is refused with rule
`transaction-size`.

A refusal is a tool error whose JSON carries the API's `code`, the `rule`
that refused (`program-allowlist`, `instruction-allowlist`, `outflow-cap`,
`window-cap`, `no-authority-change`, `session-grant`, `swap-venues`, ...),
what triggered it (`program`, `instruction`, `mint`, `attempted`, `cap`,
`remaining`), a `hint`, and for `sandbox_invoke` the `idempotencyKey`.

Every sandbox tool error has `status`, `code`, `retryable` and `hint`, even
when no API answered (`status: 0`) or something in front of it answered
with no readable body. The retry rule:

- `retryable: false`: the same call fails again; the `hint` says what to
  change. A wallet that cannot pay is `sandbox-guard-insufficient-funds`
  with `balanceLamports` (and `shortfallLamports` when known).
- `retryable: true` on a read (`sandbox_status`, `sandbox_rules`,
  `sandbox_derive`, `sandbox_action`, `sandbox_history`,
  `sandbox_holdings`) or a `sandbox_preview`: nothing was changed; retry
  the same call after `retryAfterSeconds` when present.
- `retryable: true` on `sandbox_invoke`: the action may have run unless the
  code says the guard refused it. Look it up with `sandbox_action` and the
  `idempotencyKey` before retrying.

Transport codes: `sandbox-api-unreachable` (no connection),
`sandbox-api-timeout` (no answer in time, from this server or the API),
`sandbox-api-unavailable` (502/503), `sandbox-internal-error` (500) and
`sandbox-response-unreadable`.

### Health

- `ping()` — returns `pong`. Used by clients to verify the server is up.

## Example session (JSON-RPC over stdio)

```jsonc
// 1. Discover what exists
{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"explore_stacks","arguments":{}}}
// → [{"name":"ore","websocket_url":"wss://ore-r4xj7y.stack.arete.run","entities":["OreRound",...]}, ...]

// 2. Get the exact view ids for the stack you picked
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"explore_stack_schema","arguments":{"stack":"ore"}}}
// → entities, fields, primary keys, and view ids like "OreRound/latest"

// 3. Open connection
{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"connect","arguments":{"url":"wss://ore-r4xj7y.stack.arete.run"}}}
// → {"connection_id":"a1b2..."}

// 4. Subscribe to a view
{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"subscribe","arguments":{"connection_id":"a1b2...","view":"OreRound/latest"}}}
// → {"subscription_id":"c3d4..."}

// 5. Query
{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"query_entities","arguments":{
  "subscription_id":"c3d4...",
  "filters":[{"path":"reward","op":"gt","value":1000}],
  "select":"key,reward,winner",
  "limit":20
}}}
```

## Logging

Logs are written to **stderr** so they never interfere with the stdio MCP
transport on stdout. Set the standard `RUST_LOG` env var to control verbosity:

```bash
RUST_LOG=hs_mcp=debug,arete_sdk=info a4 mcp
```

## Environment

| Variable        | Purpose                                                                                      |
| --------------- | -------------------------------------------------------------------------------------------- |
| `ARETE_API_KEY` | API key for `connect` and the knowledge tools (required), and for widening `explore_stacks`   |
| `ARETE_API_URL` | API base for the discovery tools and credentials lookup. Defaults to `https://api.arete.run`  |
| `ARETE_MCP_RPC_URL` | Solana RPC the protocol tools simulate prepared transactions against (signer balance changes) |
| `ARETE_MCP_SIGNER_CONFIG` | A TOML file with a `[signer]` table, instead of `~/.arete/mcp.toml`                    |
| `ARETE_SANDBOX_ID` | The sandbox an owner granted this agent; lists the `sandbox_*` tools. The agent's API key mints a scoped sandbox token for it, sent only to `ARETE_API_URL` when it is an Arete HTTPS or loopback origin |
| `RUST_LOG`      | Log verbosity (stderr only)                                                                   |

A resolved API key is attached to discovery requests **only when `ARETE_API_URL`
names an Arete origin over HTTPS** — `https://arete.run` or any
`https://*.arete.run` subdomain. Plain HTTP is permitted only for loopback
(`localhost`, `127.0.0.1`, `[::1]`), where the request never leaves the machine.
Anything else — including `http://api.arete.run`, which would put the key on the
wire in cleartext — gets an unauthenticated request instead. Every endpoint the
discovery tools touch is public, so you still get the public result set.

## Status

Tracks Linear issue HYP-189. Triggers (`add_trigger` / `get_triggered`) and an
HTTP/SSE transport are planned for v2.
