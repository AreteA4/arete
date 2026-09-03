# Arete Agent API Reference

This is the hosted platform API reference, not an installed Agent Skill and not
the bootstrap flow. For setup, follow `https://docs.arete.run/agent.md`. For
day-to-day work, use the five skills installed by `a4 init`; they retrieve exact
descriptors and generated APIs instead of treating this page as static product
context.

API base URL: `https://api.arete.run`. Agent endpoints live under
`/api/agents/*`. Do not send API credentials to the docs or marketing sites.

## Key files

| File | URL | Purpose |
|------|-----|---------|
| API reference (this file) | `https://docs.arete.run/skill.md` | Hosted agent identity, key, MCP, and API contracts |
| `agent.md` | `https://docs.arete.run/agent.md` | Bootstraps the `a4` CLI + skill files locally on your machine |
| `arete` skill | https://github.com/AreteA4/skills/blob/main/skills/arete/SKILL.md | Capability discovery, exact descriptors, and project dependencies |
| `arete-streams` skill | https://github.com/AreteA4/skills/blob/main/skills/arete-streams/SKILL.md | Typed view reads and subscriptions in TypeScript, React, Rust, and Python |
| `arete-programs` skill | https://github.com/AreteA4/skills/blob/main/skills/arete-programs/SKILL.md | Program reads, PDAs, instructions, semantic operations, and transaction safety |
| `arete-stack-authoring` skill | https://github.com/AreteA4/skills/blob/main/skills/arete-stack-authoring/SKILL.md | Read-model design, join proof, Rust DSL, and portable artifacts |
| `arete-deploy` skill | https://github.com/AreteA4/skills/blob/main/skills/arete-deploy/SKILL.md | Program publication and hosted deployment lifecycle |
| Registry | https://arete.run/registry | Browseable catalog of public stacks |
| Docs MCP server | `https://docs.arete.run/mcp` | HTTP MCP — `search_docs`, `fetch_page` over these docs |
| Stream MCP server | `a4 mcp` | stdio MCP — connect/subscribe/query live stack entities |
| CLI installer | `https://arete.run/install.sh` / `https://arete.run/install.ps1` | Prebuilt, signed `a4` binary; no Rust needed |

## Security

Your API key (`a4_ak_*`) is a secret.

- Only send it to `https://api.arete.run`.
- Never include it in commit messages, public repos, chat logs, or forum posts.
- It is shown **exactly once** at signup. There is no recovery flow — if you lose it, register a new agent.
- Do not share the key with another agent or process. If you need a second principal, register a second agent.
- Treat your API key like a password.

## Agent identity

### 1. Register

Preferred: complete setup through `https://docs.arete.run/agent.md`, then let the
CLI register you and store the key:

```bash
a4 auth signup                    # optional: a4 auth signup my-agent
a4 auth signup --json             # prints {"slug","displayName","credentialsPath","apiKey"}
```

`a4 auth signup` calls the endpoint below, saves the key to `~/.arete/credentials.toml`,
and prints your slug and the credentials path. It never prints the key in human mode.
With `--json` the output includes `apiKey` — a secret; use it only to set
`ARETE_API_KEY` for a sub-process, never log it. If credentials already exist for
this API URL it refuses unless you pass `--force` (check with `a4 auth status`).
Same rate limit as the raw call: on `429` it tells you to retry later or use a
human-issued key via `a4 auth login --key <a4_ak_…>`.

Fallback without the CLI, the raw HTTP call:

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

## Local tooling and workflows

This reference deliberately does not duplicate setup, discovery, SDK generation,
or deployment instructions. Follow `https://docs.arete.run/agent.md` until
`a4 doctor --json` exits 0, then use the installed workflow skill that matches the
task. Use the `arete` MCP server (`a4 mcp`) for live data inside an agent loop; use
generated SDKs for shipped application code.

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

`a4 init -y` registers it as `arete-docs` alongside the stream server. Manual: `claude mcp add --transport http arete-docs https://docs.arete.run/mcp`

### Stream MCP (stdio)

Discover registry resources and read live stack entities from inside your own agent
loop, without generating an SDK.

The server is built into the `a4` binary:

```bash
a4 mcp
```

`a4 init -y` writes this config for every detected agent. Manual shape (Claude Code `.mcp.json`):

```json
{"mcpServers":{"arete":{"type":"stdio","command":"a4","args":["mcp"]},"arete-docs":{"type":"http","url":"https://docs.arete.run/mcp"}}}
```

Use the absolute path from `A4_BIN=` as `command` when the host does not inherit your
shell PATH. `npx -y @usearete/mcp` is deprecated.

Discovery tools — read-only against the public registry, no auth required. Casing is not
uniform: `explore_stacks` and `explore_stack_schema` return snake_case (`websocket_url`,
`primary_keys`, `rust_type`), while `explore_stack`, `explore_programs` and
`explore_program` return camelCase (`websocketUrl`, `installName`, `programSpecHash`).
`resolve_artifact` has a camelCase envelope over a stored payload that may be snake_case
inside. `a4 explore --json` is camelCase throughout. Read the keys you actually receive.

| Tool | Purpose |
|------|---------|
| `explore_stacks` | List stacks. The `websocket_url` is what `connect` takes |
| `explore_stack` | Pinned install descriptor for one stack |
| `explore_stack_schema` | Entity/view schema — the view ids `subscribe` accepts |
| `explore_programs` | List installable standalone programs |
| `explore_program` | Pinned install descriptor for one program |
| `resolve_artifact` | Fetch a content-addressed artifact by kind and hash |

Knowledge tools — read-only against `/api/registry/knowledge/*`. Unlike the explore
tools these **require an API key** (`ARETE_API_KEY`, or the file `a4 auth signup` /
`a4 auth login` writes); without one they fail up front with an actionable error rather than
returning a public subset.

| Tool | Purpose |
|------|---------|
| `search_knowledge` | Search the curated knowledge layer by intent (`query`), or filter by `concept` / `category` slug. Results carry `read` / `build` / `subscribe` coverage flags |
| `get_protocol` | One protocol: programs with roles, related protocols, public stacks streaming it, per-concept coverage |
| `get_program_knowledge` | One program's reviewed annotations. `section` is `summary` (default), `instructions`, `accounts`, or `surface` (SDK operations with bindings) |
| `get_recipe` | One cross-protocol recipe with resolved surface refs and an example path (catalog is growing) |
| `list_concepts` | Concept and category vocabularies — the slugs `search_knowledge` filters accept |

Typical flow for "monitor swaps": `search_knowledge({ query: "monitor swaps" })` → read
each result's coverage → `subscribe` set? `get_protocol` names the stack, then
`explore_stack_schema` + `connect` + `subscribe` below → `read`/`build` only?
`get_program_knowledge({ program, section: "surface" })` lists the SDK operations to
call from generated code.

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
`ARETE_API_KEY`, then the credentials file written by `a4 auth signup` / `a4 auth login`. Prefer omitting
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
| GET | `/api/registry/knowledge/vocabulary` | Knowledge layer concept and category vocabularies |
| GET | `/api/registry/knowledge/search?q=&concept=&category=&limit=` | Search the knowledge layer by intent or slug filter (at least one param) |
| GET | `/api/registry/knowledge/protocols/:slug` | Curated protocol knowledge with per-concept coverage |
| GET | `/api/registry/knowledge/programs/:slug?section=` | Program annotations: `summary`, `instructions`, `accounts`, or `surface` |
| GET | `/api/registry/knowledge/recipes/:slug` | Cross-protocol recipe with resolved surface refs |
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

## Keeping the Surface Current

Use `a4 doctor --json`, exact `a4 explore` output, current CLI help, and the docs
MCP before relying on a copied endpoint example. This page is a reference, not a
versioned activation prompt.

## Support

- Website: https://arete.run
- Docs: https://docs.arete.run
- Agent API reference (this): https://docs.arete.run/skill.md
- `agent.md` (local tooling install): https://docs.arete.run/agent.md
- Source: https://github.com/usearete

Good luck. Build something useful.
