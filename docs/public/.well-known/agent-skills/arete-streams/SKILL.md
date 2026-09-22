---
name: arete-streams
description: Query or subscribe to deployed Arete stack views from TypeScript, React, Rust, Python, the a4 CLI, or the Arete MCP server. Use for dashboards, bots, backends, current-state reads, live entity updates, view filtering, or stream debugging. Do not use for program accounts or transaction construction; use arete-programs for those.
metadata:
  version: "1.1.0"
  min-cli: ">=0.20.4"
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
```

Use the descriptor's exact `installRef`, identities, authentication policies, selected views, SDK targets, and `installCommand`. A failed descriptor is not permission to fall back to an unpinned deployment.

## Choose the Consumer Surface

- Use `a4 stream` or the configured Arete MCP server for investigation during an agent run.
- Use the generated SDK for application code, durable automation, tests, or anything committed to the project.
- Use HTTP-only connection mode only for point-in-time reads; view subscriptions must fail fast without WebSocket transport.

Useful CLI probes include:

```bash
a4 stream <Entity>/<view> --stack <stack-ref> --first
a4 stream <Entity>/<view> --stack <stack-ref> --where '<field>=<value>' --take 10
a4 stream <Entity>/<view> --stack <stack-ref> --ops snapshot,upsert,patch,remove,delete --duration 15
```

Run `a4 stream --help` for the current filtering, selection, cursor, history, snapshot, and TUI options. Do not invent MCP tool names; inspect the configured server's exposed tools.

## Install and Inspect Generated Code

Prefer the descriptor's `installCommand`. Otherwise add the exact stack dependency for the project language:

```bash
a4 install stack <stack-ref> --ts
a4 install stack <stack-ref> --rust
a4 install stack <stack-ref> --python
```

Inspect the generated exports and types before coding. Generated names are the application API; raw descriptor field paths remain useful for CLI filters and diagnostics.

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

- Hosted reads commonly require a publishable key, including browser reads.
- A read-only view does not require a wallet.
- Browser publishable keys must be origin-bound and may appear in client configuration.
- Never embed an Arete API key, wallet secret, private key, or unrestricted token in generated or browser code.

If a publishable key must be created or its origins changed, that is an external account mutation. Do it only when requested and use the current `a4 auth keys --help` surface.

## Verify Behavior

Validate more than compilation:

1. Confirm the generated dependency identity matches the explored descriptor.
2. Exercise a bounded first read or first update with an explicit timeout.
3. Check state keys and numeric types such as `bigint`, `u64`, or Python `int`.
4. Exercise empty and error states; an absent subscription is not the same as an empty result.
5. Close or release streams, sessions, and clients in scripts and tests.

Do not keep an unbounded live command running merely to prove connectivity.
