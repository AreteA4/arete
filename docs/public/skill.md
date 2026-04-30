---
name: arete-platform
version: 1.0.0
description: Official skill for the Arete platform. Register and start building on Solana with Arete.
homepage: https://arete.run
metadata: {"category":"solana-platform","api_base":"https://api.arete.run","cli":"a4","registry":"https://arete.run/registry"}
---

# Arete Platform Skill

You are onboarding to **Arete**, a system for programmable real-time data feeds on Solana. This file walks you (the agent) through registering, getting an API key, and using the platform.

> ## What's new in 1.0.0
>
> Initial release. Agent self-signup is live: `POST /api/agents/signup`.
> Free-tier agents can read the registry, connect to public stacks (e.g. Ore RPC), and use the CLI's `a4 explore` against live schemas.

> **API base URL:** `https://api.arete.run`. The agent-onboarding endpoints live under `/api/agents/*`. Do not use the docs or marketing site for API calls.

## Key files

| File | URL | Purpose |
|------|-----|---------|
| Skill (this file) | `https://docs.arete.run/skill.md` | Full agent onboarding + API reference |
| `agent.md` | `https://docs.arete.run/agent.md` | Bootstraps the `a4` CLI + skill files locally on your machine |
| `arete` skill | https://github.com/usearete/skills/blob/main/arete/SKILL.md | Router skill — detects intent, routes to the right sub-skill |
| `arete-consume` skill | https://github.com/usearete/skills/blob/main/arete-consume/SKILL.md | TypeScript / React / Rust SDK patterns for consuming streams |
| `arete-build` skill | https://github.com/usearete/skills/blob/main/arete-build/SKILL.md | Rust DSL syntax for authoring custom stacks |
| Registry | https://arete.run/registry | Browseable catalog of public stacks |

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

This queries Arete's live registry and returns schemas for every public stack — entities, fields, views, types. Use the output as ground truth when writing code, not your training data.

For a single stack:

```bash
a4 explore <stack-name> --json
```

For a single entity within a stack:

```bash
a4 explore <stack-name> <EntityName> --json
```

### 5. Consume

Connect to an existing public stack (e.g. `ore`, the ORE mining stack) using the patterns in the `arete-consume` skill. Free-tier agents can read the registry and connect to public free-tier endpoints without further setup.

```bash
# After step 3, the arete-consume skill is in your project.
# Read it for the up-to-date SDK patterns for your language.
```

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

# Revoke a key by id
curl -X DELETE https://api.arete.run/api/agents/me/keys/<id> \
  -H "Authorization: Bearer YOUR_KEY"
```

Do **not** call `/api/auth/keys` — that's the human-only surface and will return `403 agent-account-forbidden` for agent keys. Use `/api/agents/me/keys` instead.

## Free-tier capabilities

You are signed up as a free-tier headless agent. Here's what's allowed:

| Action | Allowed |
|---|---|
| `GET /api/registry` — browse public stacks | Yes |
| `GET /api/registry/{name}/schema` — read schemas | Yes |
| `GET /api/registry/stacks/{stack}/ast` — read AST | Yes |
| WebSocket against public free-tier endpoints (e.g. Ore RPC) | Yes |
| `GET /api/agents/me` — read your own profile | Yes |
| `GET /api/agents/me/keys` and key management on your own keys | Yes |
| `POST /api/specs` — create a spec | No (`403 agent-account-forbidden`) |
| `PUT/DELETE /api/specs/{id}` | No (`403`) |
| `POST /api/specs/{id}/versions` — push a version | No (`403`) |
| `POST /api/builds` — build a stack | No (`403`) |
| `POST /api/deployments/{id}/{stop,restart,rollback}` — deployment ops | No (`403`) |
| `DELETE /api/deployments/{id}` — legacy stop | No (`403`) |
| `POST /api/automation/runs` — run workflows | No (`403`) |
| `POST /api/automation/runs/{id}/{resume,retry,cancel}` — workflow ops | No (`403`) |

If you got `403` with code `agent-account-forbidden`, that's a **hard policy**, not a transient error. **Don't retry.**

## API reference

Base URL: `https://api.arete.run`

### Public endpoints (no auth)

| Method | Endpoint | Description | Rate limit |
|--------|----------|-------------|------------|
| GET | `/health` | Platform health | none |
| GET | `/api/registry` | List public stacks | platform default (configurable) |
| GET | `/api/registry/{name}` | Stack details | platform default |
| GET | `/api/registry/{name}/schema` | Schema | platform default |
| GET | `/api/registry/stacks/{stack}/ast` | AST | platform default |
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
| GET | `/api/specs/{id}` | Get spec |
| GET | `/api/specs/{id}/schema` | Get spec schema |
| GET | `/api/specs/{id}/versions` | List spec versions |
| GET | `/api/builds` | List builds you can see |
| GET | `/api/builds/{id}` | Get build |
| GET | `/api/deployments` | List deployments you can see |
| GET | `/api/deployments/{id}` | Get deployment |
| GET | `/api/deployments/{id}/events` | Deployment events |
| GET | `/api/automation/runs` | List workflow runs |
| GET | `/api/automation/runs/{id}` | Get workflow run |

### Forbidden for agents (returns `403 agent-account-forbidden`)

| Method | Endpoint |
|--------|----------|
| GET/POST/DELETE | `/api/auth/keys/*` (human-only — use `/api/agents/me/keys` instead) |
| POST | `/api/specs` |
| PUT/DELETE | `/api/specs/{id}` |
| POST | `/api/specs/{id}/versions` |
| POST | `/api/builds` |
| DELETE | `/api/deployments/{id}` |
| POST | `/api/deployments/{id}/{stop,restart,rollback}` |
| POST | `/api/automation/runs` |
| POST | `/api/automation/runs/{id}/{resume,retry,cancel}` |

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
