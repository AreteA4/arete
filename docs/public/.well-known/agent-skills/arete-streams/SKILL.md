---
name: arete-streams
description: Query or subscribe to deployed Arete stack views from TypeScript, React, Rust, Python, the a4 CLI, or the Arete MCP server. Use for dashboards, bots, backends, current-state reads, live entity updates, view filtering, or stream debugging. Do not use for program accounts or transaction construction; use arete-programs for those.
metadata:
  version: "1.6.0"
  min-cli: ">=0.35.0"
---

# Query and Subscribe to Arete Views

Implement against an exact generated stack binding. Never infer entity names, view modes, keys, field paths, or types from examples or training data.

## Resolve the Exact Surface

If the environment may not be ready, run `a4 doctor --json` and follow any
required fix. When a stack is not already pinned, discover subscribe-ready
stacks by intent and inspect the exact result:

```bash
a4 explore catalog --query "<intent>" --kind stack --mode subscribe --json
a4 explore catalog stack <stack-slug> --json
```

Then inspect the descriptor and relevant entity before writing code. A direct
reference supplied by the user or project can use the compatibility form:

```bash
a4 explore stack <stack-ref> --json
a4 explore stack <stack-ref> <Entity> --json
a4 explore stack <stack-ref> --full --json
```

The first prints a compact summary (views, amount units, `read.*` helpers, install command); `--full` prints the descriptor. Use the descriptor's exact `installRef`, identities, authentication policies, selected views, SDK targets, and `installCommand`. A failed descriptor is not permission to fall back to an unpinned deployment.

## Choose the Consumer Surface

- To check a current value once, use `a4 get` (or MCP `read_view`) instead of writing a script. It reads the view's current entities, prints one JSON document, and exits.
- Use `a4 stream` or the configured Arete MCP server to follow a view during an agent run. MCP cache reads wait for the subscription's snapshot and report `ready`.
- Use the generated SDK for application code, durable automation, tests, or anything committed to the project.
- Use HTTP-only connection mode only for point-in-time reads; view subscriptions must fail fast without WebSocket transport.

Useful CLI probes include:

```bash
a4 get <Entity>/<view> --stack <stack-ref> --limit 1
a4 get <Entity>/<view> --stack <stack-ref> --select <field>,<field> --limit 10
a4 get <Entity>/state --stack <stack-ref> --key <key>
a4 stream <Entity>/<view> --stack <stack-ref> --first
a4 stream <Entity>/<view> --stack <stack-ref> --where '<field>=<value>' --take 10
a4 stream <Entity>/<view> --stack <stack-ref> --ops snapshot,upsert,patch,remove,delete --duration 15
```

`a4 get` also takes `--where` and `--timeout`. Run `a4 stream --help` for the current filtering, selection, cursor, history, snapshot, and TUI options. Do not invent MCP tool names; inspect the configured server's exposed tools.

Before reporting token amounts, check each field's `amount` in `a4 explore stack <stack-ref> --views <Entity>/<view> --json` (or MCP `explore_stack_schema`): `scale: "ui"` values are whole tokens, `scale: "raw"` values are base units (divide by `10^decimals`; for SOL these are lamports), and `counterpart` names the same amount at the other scale. When the stack does not fix the decimals, `amount` has `decimalsFrom` instead of `decimals`: read the decimals from that field (for example the mint's metadata) before converting, and if it is unavailable, report the raw amount rather than guessing.

## Install and Inspect Generated Code

Prefer the descriptor's `installCommand`. Otherwise add the exact stack dependency for the project language:

```bash
a4 install stack <stack-ref> --ts
a4 install stack <stack-ref> --rust
a4 install stack <stack-ref> --python
```

For TypeScript in a directory with no `package.json`, add `--setup` (`a4 install stack <stack-ref> --ts --setup`, or `a4 install --setup`): it creates an ES module `package.json` and `tsconfig.json` and installs the runtime and dev dependencies. It never replaces existing files.

Inspect the generated exports and types before coding. Generated names are the application API; raw descriptor field paths remain useful for CLI filters and diagnostics.

If the generated stack definition has empty endpoints, the stack is definition-only and has no deployment yet. Nothing can stream from it until one exists. Deploying it is an external mutation handled by `arete-deploy`; once deployed, the project records the endpoints and the SDK is regenerated. Never invent an endpoint to fill the gap.

## Select the Correct View Operation

Every language expresses the same view semantics:

| Need | Canonical operation |
| --- | --- |
| Merged live entities | `use` (`listen` in Rust) |
| Raw membership/update operations | `watch` |
| Before/after diffs | `watchRich` / `watch_rich` |
| Await one snapshot | `get` |
| Read an existing local lease without waiting | `getSync` / `get_sync` |
| First item from a list | `getOne` / `get_one` |

State views require the generated key shape or language-specific key representation. List and custom views do not. Inspect the generated accessor instead of assuming every language accepts the same key form.

For derived "current X" state, such as a protocol's current round, check whether the stack defines a read (`read.*`) before combining several views and chain state yourself. The [TypeScript](references/typescript.md) and [React](references/react.md) references show how to use one.

For update taxonomy, snapshot authority, query identity, and absence semantics, read [references/view-semantics.md](references/view-semantics.md).

## Implement by Project Language

Read only the relevant reference:

- [TypeScript](references/typescript.md)
- [React](references/react.md)
- [Rust](references/rust.md)
- [Python](references/python.md)

Match the existing project language and framework. Do not migrate frameworks merely to follow an example.

## Authentication and Secrets

Use the authentication policy returned by the descriptor.

- Hosted reads commonly require a key, including browser reads.
- A read-only view does not require a wallet.
- Servers, agents, and local scripts: set no auth option. The SDKs use `ARETE_API_KEY` if set, and otherwise the key from the active `a4` login, so after `a4 init` or `a4 auth signup` scripts need no key setup. Do not copy the key into code or the environment. If several `a4` profiles hold a key, set `ARETE_PROFILE` (for example `agent`). The login key is only sent to the default Arete API.
- To pass a key explicitly, use `secretKey` (TypeScript) or `secret_key` (Python, Rust) with an agent or secret key read from the environment, never from source.
- On a 401 or missing-key error, run `a4 auth signup` or `a4 auth login`, or set `ARETE_PROFILE` or `ARETE_API_KEY`.
- Anything shipped to a browser uses an origin-bound publishable key (`publishableKey` / `publishable_key`), which may appear in client configuration. The TypeScript SDK throws if `secretKey` is used in a browser.
- Never embed an Arete API key, wallet secret, private key, or unrestricted token in generated or browser code.

If a publishable key must be created or its origins changed, that is an external account mutation. Do it only when requested: `a4 auth keys create-publishable --origin <scheme://host[:port]>` creates one, and `a4 auth keys --help` lists the rest.

## Verify Behavior

Validate more than compilation:

1. Confirm the generated dependency identity matches the explored descriptor.
2. Exercise a bounded first read or first update with an explicit timeout.
3. Check state keys, numeric types such as `bigint`, `u64`, or Python `int`, and each amount field's `amount` scale.
4. Exercise empty and error states; an absent subscription is not the same as an empty result.
5. Close or release streams, sessions, and clients in scripts and tests.

Do not keep an unbounded live command running merely to prove connectivity.
