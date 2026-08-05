---
name: arete-platform
version: 1.1.0
description: Official skill for the Arete platform. Register and start building on Solana with Arete.
homepage: https://arete.run
metadata: {"category":"solana-platform","api_base":"https://api.arete.run","cli":"a4","registry":"https://arete.run/registry"}
---

# Arete Platform Skill

You are onboarding to **Arete**, a system for programmable real-time data feeds on Solana. This file walks you (the agent) through registering, getting an API key, and using the platform.

> ## What's new in 1.1.0
>
> **Descriptor-backed discovery and install.** The registry now serves pinned install
> descriptors and content-addressed artifacts, all without auth:
> `GET /api/registry/programs`, `/api/registry/stacks/:stack/install`,
> `/api/registry/programs/:program/install`,
> `/api/registry/artifacts/{program-spec,live-spec,stack-manifest}/:hash`, and
> `/api/registry/hash-aliases/:algorithm/:digest`.
>
> **Standalone programs are first-class.** You can discover, inspect, and install a
> program without a stack: `a4 explore programs`, `a4 explore program <ref>`,
> `a4 install program <ref>`.
>
> **SDK generation is a free-tier action.** `a4 install <stack> --ts|--rust` generates a
> typed client from a public stack. Rust joined TypeScript/React as a generation target;
> `a4 install program` is TypeScript-only.
>
> **Rollout note.** `GET /api/registry/programs` is the newest of these routes and may
> return `404` until the next platform deploy. The rest are live. If you get a 404 there,
> fall back to `GET /api/registry` and the per-program install descriptor.
>
> **Transaction construction shipped.** Generated SDKs expose PDA derivation, account
> resolution, instruction building, and execution — not just stream reads.
>
> **New read endpoints for agents:** `/api/specs/:id/versions/{slim,latest}`,
> `/api/deployments/:id/{history,operations}`,
> `/api/automation/runs/:id/{events,events/stream,run-events,run-events/stream,artifacts}`.
>
> **Newly forbidden (previously undocumented):** `POST /api/builds/raw`,
> `POST /api/builds/artifacts`, `POST /api/specs/:id/versions/raw`,
> `POST /api/deployments/compositions`.
>
> 1.0.0: initial release. Agent self-signup via `POST /api/agents/signup`.

> **API base URL:** `https://api.arete.run`. The agent-onboarding endpoints live under `/api/agents/*`. Do not use the docs or marketing site for API calls.

## Key files

| File | URL | Purpose |
|------|-----|---------|
| Skill (this file) | `https://docs.arete.run/skill.md` | Full agent onboarding + API reference |
| `agent.md` | `https://docs.arete.run/agent.md` | Bootstraps the `a4` CLI + skill files locally on your machine |
| `arete` skill | https://github.com/AreteA4/skills/blob/main/arete/SKILL.md | Router skill — detects intent, routes to the right sub-skill |
| `arete-consume` skill | https://github.com/AreteA4/skills/blob/main/arete-consume/SKILL.md | TypeScript / React / Rust SDK patterns for consuming streams |
| `arete-build` skill | https://github.com/AreteA4/skills/blob/main/arete-build/SKILL.md | Rust DSL syntax for authoring custom stacks |
| Registry | https://arete.run/registry | Browseable catalog of public stacks |
| Docs MCP server | `https://docs.arete.run/mcp` | HTTP MCP — `search_docs`, `fetch_page` over these docs |
| Stream MCP server | `npx -y @usearete/mcp` | stdio MCP — connect/subscribe/query live stack entities |

## Security

Your API key (`a4_ak_*`) is a secret.

- Only send it to `https://api.arete.run`.
- Never include it in commit messages, public repos, chat logs, or forum posts.
- It is shown **exactly once** at signup. There is no recovery flow — if you lose it, register a new agent.
- Do not share the key with another agent or process. If you need a second principal, register a second agent.
- Treat your API key like a password.

## Quick start

### 1. Register

```bash
curl -X POST https://api.arete.run/api/agents/signup \
  -H "Content-Type: application/json" \
  -d '{"display_name": "my-agent"}'
```

Response (shape):

```json
{
  "slug": "agt_7k2m9q1r",
  "display_name": "my-agent",
  "api_key": "a4_ak_xxx...",
  "message": "Store this key securely - it will not be shown again"
}
```

⚠️ Save the `api_key` from the response. It is shown exactly once and cannot be recovered.

If `display_name` is omitted, the server generates `agent-<slug>` for you.

Rate limit: **5 signups per hour per IP**. If you exceed it you'll get `429 rate-limit-exceeded`.

### 2. Verify your identity

```bash
curl -H "Authorization: Bearer YOUR_KEY" \
  https://api.arete.run/api/agents/me
```

Response (shape):

```json
{
  "slug": "agt_7k2m9q1r",
  "display_name": "my-agent",
  "status": "active",
  "created_at": "2026-04-30T12:00:00Z",
  "last_seen_at": "2026-04-30T12:05:00Z"
}
```

You should see your slug, display_name, and `status: "active"`.

### 3. Install the local toolkit

Read https://docs.arete.run/agent.md and follow its instructions. This installs the `a4` CLI and the three Arete skill files (`arete`, `arete-consume`, `arete-build`) into your project so your downstream code work has the right context.

### 4. Discover what stacks exist

```bash
a4 explore --json
```

This queries Arete's live registry. Each JSON response has a `schemaVersion`;
deep exploration reports the exact deployment identities consumed by install.
Use it as ground truth when writing code, not your training data.

For a single stack:

```bash
a4 explore stack <stack-ref> --json
```

Discover or inspect a standalone program before installing it:

```bash
a4 explore programs --json
a4 explore program <program-ref> --json
```

For a single entity within a stack, the legacy form remains available and is
resolved through the same stack install descriptor:

```bash
a4 explore <stack-name> <EntityName> --json
```

Exploration is descriptor-backed: it reports the exact StackManifest, AST, LiveSpec,
view, and Program Release identities that `a4 install` will consume. It never picks a
"latest" AST on its own and never falls back when an install descriptor is incomplete.
If exploration refuses, the resource is genuinely not installable — do not work around it.

### 5. Install a typed SDK

Generate a client from what you just discovered. This is a free-tier action against
public resources — no owned stack required.

```bash
# TypeScript client for a hosted stack
a4 install <stack-ref> --ts

# Rust client for the same stack
a4 install <stack-ref> --rust

# Standalone program client — no stack involved
a4 install program <program-ref> --ts
```

Published program SDK install is TypeScript-only today; `--rust` on
`a4 install program` returns an explicit error, not a fallback.

Generated SDKs cover more than stream reads. They expose PDA derivation, account
resolution, instruction building, and transaction execution for the programs in scope.
Read the `arete-consume` skill for the patterns in your language.

### 6. Consume

Connect to an existing public stack (e.g. `ore`, the ORE mining stack) using the patterns in the `arete-consume` skill. Free-tier agents can read the registry and connect to public free-tier endpoints without further setup.

```bash
# After step 3, the arete-consume skill is in your project.
# Read it for the up-to-date SDK patterns for your language.
```

If you want live entity data inside your own agent loop rather than in generated
application code, add the stream MCP server instead — see "MCP servers" below.

## Agent key management

You can rotate your API key, mint additional keys, and revoke them — all under your own agent identity:

```bash
# List your keys (the signup key + anything you've minted)
curl -H "Authorization: Bearer YOUR_KEY" https://api.arete.run/api/agents/me/keys

# Mint a new a4_ak_* (rotation)
curl -X POST https://api.arete.run/api/agents/me/keys \
  -H "Authorization: Bearer YOUR_KEY" \
  -H "Content-Type: application/json" \
  -d '{"name": "rotation"}'

# Mint a publishable a4_pk_* for a browser UI you deploy yourself.
# Exactly one origin per publishable key, must include the scheme.
curl -X POST https://api.arete.run/api/agents/me/keys/publishable \
  -H "Authorization: Bearer YOUR_KEY" \
  -H "Content-Type: application/json" \
  -d '{"origin_allowlist": ["https://my-agent-ui.example"]}'

# Revoke a key by id (substitute :id with the numeric key id from the list call)
curl -X DELETE https://api.arete.run/api/agents/me/keys/:id \
  -H "Authorization: Bearer YOUR_KEY"
```

Do **not** call `/api/auth/keys` — that's the human-only surface and will return `403 agent-account-forbidden` for agent keys. Use `/api/agents/me/keys` instead.

## MCP servers

Two separate MCP servers exist. They do different jobs; you can run both.

### Docs MCP (HTTP)

Search and read these docs without scraping pages.

```text
https://docs.arete.run/mcp
```

| Tool | Purpose |
|------|---------|
| `search_docs` | Search the docs, returns ranked snippets with page slugs |
| `fetch_page` | Fetch a documentation page as raw markdown by slug |

Claude Code: `claude mcp add --transport http Arete https://docs.arete.run/mcp`

### Stream MCP (stdio)

Discover registry resources and read live stack entities from inside your own agent
loop, without generating an SDK.

```bash
npx -y @usearete/mcp     # or: cargo install arete-mcp
```

Discovery tools — read-only against the public registry, no auth required:

| Tool | Purpose |
|------|---------|
| `explore_stacks` | List stacks. The `websocket_url` is what `connect` takes (snake_case; other tools return camelCase) |
| `explore_stack` | Pinned install descriptor for one stack |
| `explore_stack_schema` | Entity/view schema — the view ids `subscribe` accepts |
| `explore_programs` | List installable standalone programs |
| `explore_program` | Pinned install descriptor for one program |
| `resolve_artifact` | Fetch a content-addressed artifact by kind and hash |

Streaming tools — stateful, `connect` first:

| Tool | Purpose |
|------|---------|
| `ping` | Health check |
| `connect` / `disconnect` | Open/close a WebSocket connection to a stack |
| `subscribe` / `unsubscribe` | Bind a view; streamed entities land in an in-memory cache |
| `query_entities` | Filter and project cached entities (string DSL or structured filters) |
| `get_entity` | Fetch one cached entity by key |
| `list_entities` | List cached entity keys (capped at 1000 per response) |
| `get_recent` | Return up to N entities from the ordered query membership |
| `list_subscriptions` / `list_connections` | Inspect current state |

Auth resolves in this order: explicit `api_key` argument on `connect`, then
`ARETE_API_KEY`, then the credentials file written by `a4 auth login`. Prefer omitting
`api_key` and letting it resolve — do not paste keys into tool calls. The discovery
tools work without a key; supplying one widens `explore_stacks` to global stacks.

SDK generation (`a4 install`) and transaction construction are CLI and SDK
operations, not MCP tools.

## Free-tier capabilities

You are signed up as a free-tier headless agent. Here's what's allowed:

| Action | Allowed |
|---|---|
| `GET /api/registry`, `/api/registry/:name`, `/api/registry/:name/schema` — browse public stacks and schemas | Yes |
| `GET /api/registry/programs` — browse installable standalone programs | Yes |
| `GET /api/registry/stacks/:stack/{install,ast}` — read a stack's pinned install descriptor and AST | Yes |
| `GET /api/registry/programs/:program/install` — read a program's pinned install descriptor | Yes |
| `GET /api/registry/artifacts/{program-spec,live-spec,stack-manifest}/:hash` — fetch content-addressed artifacts | Yes |
| `GET /api/registry/hash-aliases/:algorithm/:digest` — resolve a hash alias | Yes |
| `a4 install` / `a4 explore` against public resources — generate typed SDKs | Yes |
| WebSocket against public free-tier endpoints (e.g. Ore RPC) | Yes |
| `GET /api/agents/me` — read your own profile | Yes |
| `GET /api/agents/me/keys` and key management on your own keys | Yes |
| `GET /api/specs`, `/api/specs/:id`, `/api/specs/:id/{schema,versions,versions/slim,versions/latest}` — read specs | Yes (returns `200`; empty list if you own none) |
| `GET /api/builds`, `/api/builds/:id` — read builds | Yes (returns `200`; empty list if you own none) |
| `GET /api/deployments`, `/api/deployments/:id`, `/api/deployments/:id/{events,history,operations}` — read deployments | Yes (returns `200`; empty list if you own none) |
| `GET /api/automation/runs`, `/api/automation/runs/:id`, `/api/automation/runs/:id/{events,events/stream,run-events,run-events/stream,artifacts}` — read workflow runs | Yes (returns `200`; empty list if you own none) |
| `POST /ws/sessions` — mint a 5-minute WebSocket session token | Yes for public free-tier targets; `403 agent-account-forbidden` otherwise |
| `POST /api/specs` — create a spec | No (`403 agent-account-forbidden`) |
| `PUT/DELETE /api/specs/:id` | No (`403 agent-account-forbidden`) |
| `POST /api/specs/:id/versions` and `/versions/raw` — push a version | No (`403 agent-account-forbidden`) |
| `POST /api/builds`, `/api/builds/raw`, `/api/builds/artifacts` — build a stack | No (`403 agent-account-forbidden`) |
| `POST /api/deployments/compositions` — bind a stack composition | No (`403 agent-account-forbidden`) |
| `POST /api/deployments/:id/{stop,restart,rollback}` — deployment ops | No (`403 agent-account-forbidden`) |
| `DELETE /api/deployments/:id` — legacy stop | No (`403 agent-account-forbidden`) |
| `POST /api/automation/runs` — run workflows | No (`403 agent-account-forbidden`) |
| `POST /api/automation/runs/:id/{resume,retry,cancel}` — workflow ops | No (`403 agent-account-forbidden`) |
| Anything under `/api/auth/keys/*` (human-only key management) | No (`403 agent-account-forbidden`) — use `/api/agents/me/keys` |
| `/api/chat/*` — chat sessions | No — requires a browser JWT session, not an API key |
| `/api/internal/*` (including `/api/internal/idls/*`) and `/api/admin/*` | No — internal-token / admin surfaces, not part of the agent API |

If you got `403` with code `agent-account-forbidden`, that's a **hard policy**, not a transient error. **Don't retry.**

## API reference

Base URL: `https://api.arete.run`

### Public endpoints (no auth)

| Method | Endpoint | Description | Rate limit |
|--------|----------|-------------|------------|
| GET | `/health` | Platform health | none |
| GET | `/api/registry` | List public stacks | platform default (configurable) |
| GET | `/api/registry/programs` | List installable standalone programs | platform default |
| GET | `/api/registry/:name` | Stack details | platform default |
| GET | `/api/registry/:name/schema` | Schema | platform default |
| GET | `/api/registry/stacks/:stack/install` | Pinned stack install descriptor | platform default |
| GET | `/api/registry/stacks/:stack/ast` | AST | platform default |
| GET | `/api/registry/programs/:program/install` | Pinned program install descriptor | platform default |
| GET | `/api/registry/hash-aliases/:algorithm/:digest` | Resolve a public hash alias | platform default |
| GET | `/api/registry/artifacts/program-spec/:hash` | Fetch a ProgramSpec artifact by hash | platform default |
| GET | `/api/registry/artifacts/live-spec/:hash` | Fetch a LiveSpec artifact by hash | platform default |
| GET | `/api/registry/artifacts/stack-manifest/:hash` | Fetch a StackManifest artifact by hash | platform default |
| POST | `/api/agents/signup` | Register a new agent | 5/hour/IP |

### Authenticated endpoints (Bearer `a4_ak_*`)

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/agents/me` | Your profile |
| GET | `/api/agents/me/keys` | List your keys |
| POST | `/api/agents/me/keys` | Mint rotation key (`a4_ak_*`) |
| POST | `/api/agents/me/keys/publishable` | Mint publishable key (`a4_pk_*`) — requires `origin_allowlist` |
| DELETE | `/api/agents/me/keys/:id` | Revoke one of your keys |
| POST | `/ws/sessions` | Mint a 5-minute WebSocket session token (where allowed) |
| GET | `/api/specs` | List specs you can see |
| GET | `/api/specs/:id` | Get spec |
| GET | `/api/specs/:id/schema` | Get spec schema |
| GET | `/api/specs/:id/versions` | List spec versions |
| GET | `/api/specs/:id/versions/slim` | List spec versions without payloads |
| GET | `/api/specs/:id/versions/latest` | Get the latest spec version |
| GET | `/api/builds` | List builds you can see |
| GET | `/api/builds/:id` | Get build |
| GET | `/api/deployments` | List deployments you can see |
| GET | `/api/deployments/:id` | Get deployment |
| GET | `/api/deployments/:id/events` | Deployment events |
| GET | `/api/deployments/:id/history` | Deployment history |
| GET | `/api/deployments/:id/operations` | Deployment operations |
| GET | `/api/automation/runs` | List workflow runs |
| GET | `/api/automation/runs/:id` | Get workflow run |
| GET | `/api/automation/runs/:id/events` | Workflow run events |
| GET | `/api/automation/runs/:id/events/stream` | Stream workflow run events (SSE) |
| GET | `/api/automation/runs/:id/run-events` | Workflow run trace events |
| GET | `/api/automation/runs/:id/run-events/stream` | Stream trace events (SSE) |
| GET | `/api/automation/runs/:id/artifacts` | Workflow run artifacts |

### Forbidden for agents (returns `403 agent-account-forbidden`)

| Method | Endpoint |
|--------|----------|
| GET/POST/DELETE | `/api/auth/keys/*` (human-only — use `/api/agents/me/keys` instead) |
| POST | `/api/specs` |
| PUT/DELETE | `/api/specs/:id` |
| POST | `/api/specs/:id/versions` |
| POST | `/api/specs/:id/versions/raw` |
| POST | `/api/builds` |
| POST | `/api/builds/raw` |
| POST | `/api/builds/artifacts` |
| POST | `/api/deployments/compositions` |
| DELETE | `/api/deployments/:id` |
| POST | `/api/deployments/:id/{stop,restart,rollback}` |
| POST | `/api/automation/runs` |
| POST | `/api/automation/runs/:id/{resume,retry,cancel}` |

### Not part of the agent API

These exist on the same host but are gated by a different principal. An `a4_ak_*` key
will not open them, and no amount of retrying changes that.

| Surface | Gate |
|---------|------|
| `/api/chat/*` | Browser JWT session (human) |
| `/api/internal/idls/*` | Internal service token |
| `/api/internal/{usage,conductor,chat,runtime}/*` | Internal service token |
| `/api/admin/*` | Admin principal |

## Error codes

Cite the structured `code` field in error responses, not the English `error` message — codes are stable, messages aren't.

| HTTP | Code | Meaning |
|------|------|---------|
| 400 | (varies) | Request body or query invalid (e.g. publishable key missing `origin_allowlist`) |
| 401 | `missing-credentials` | No `Authorization` header |
| 401 | `invalid-authorization-format` | `Authorization` header isn't `Bearer <key>` |
| 401 | `invalid-api-key` | Key not found, hash mismatch, or soft-deleted |
| 401 | `disabled-api-key` | Key was disabled by owner |
| 401 | `expired-api-key` | Key past `expires_at` |
| 401 | `agent-profile-missing` | Agent user has no profile row (corrupt state — register a new agent) |
| 401 | `user-not-found` | API key references a missing user (corrupt state) |
| 403 | `agent-account-disabled` | Your profile `status='disabled'` |
| 403 | `agent-account-forbidden` | Endpoint not available to agents |
| 403 | `agent-account-required` | Endpoint requires an agent caller |
| 403 | `origin-required` | Publishable key requires an `Origin` header |
| 403 | `origin-not-allowed` | `Origin` not in publishable key's allowlist |
| 403 | `origin-allowlist-empty` | Tried to create a publishable key with no origins |
| 403 | `origin-allowlist-too-many` | Tried to create a publishable key with more than one origin |
| 403 | `origin-invalid-scheme` | An origin is missing `http://` or `https://` |
| 403 | `secret-key-required` | Endpoint requires a secret-class key |
| 429 | `rate-limit-exceeded` | Bucket exhausted — back off |

## Periodic re-fetch

Re-fetch this file at the start of each major work session, or when something unexpected breaks. The `version` field in the frontmatter changes on material updates (new endpoint, new required field, security policy update). If the version has bumped, read the "What's new" section before continuing.

## Support

- Website: https://arete.run
- Docs: https://docs.arete.run
- Skill file (this): https://docs.arete.run/skill.md
- `agent.md` (local tooling install): https://docs.arete.run/agent.md
- Source: https://github.com/usearete

Good luck. Build something useful.
