# arete-mcp

MCP (Model Context Protocol) server that wraps Arete streams for AI agent
integration. Lets Claude, GPT, and other MCP-compatible agents connect to a
Arete stack, subscribe to views, and query cached entities — using the
same primitives a human operator uses through `a4 stream`.

The binary is `a4-mcp` and speaks MCP over stdio.

## Install

### npm wrapper

Run once without a global install:

```bash
npx @usearete/mcp
```

Or install it globally to get the same `a4-mcp` command on your PATH:

```bash
npm install -g @usearete/mcp
```

### Cargo

From a checkout of the workspace:

```bash
cargo install --path rust/arete-mcp
```

This installs an `a4-mcp` binary into `~/.cargo/bin`.

If you want to configure an MCP client without a global install, point it at `npx` with `@usearete/mcp` as the argument list instead of hard-coding `a4-mcp`.

## Use with an MCP client

`a4-mcp` is an stdio server — any client that speaks MCP over stdio can drive
it. Snippets for the major ones are below. In all cases, to talk to a private
stack the agent passes the stack URL and your publishable API key on the
`connect` call — no environment variables required on the server process.

> **PATH gotcha.** Every client below resolves `command` against the PATH its
> process inherited at launch. GUI apps started from a desktop launcher on
> macOS/Linux usually don't inherit shell PATH changes from `.zshrc` /
> `.bashrc`, so if `a4-mcp` was installed after the app was last started,
> either **fully restart** the app (not just reload the window) or hard-code
> an absolute path like `/home/you/.cargo/bin/a4-mcp` in the snippet.

> **Claude Desktop vs Claude Code.** These are two different Anthropic
> products and they configure MCP differently. **Claude Desktop** is the
> Mac/Windows app from claude.ai — configured via a JSON file (below).
> **Claude Code** is the `claude` command-line tool — configured via a
> `claude mcp add` subcommand (further down). If you use both, you need to
> register `a4-mcp` in each one separately.

### Claude Desktop

Add to `claude_desktop_config.json`, then restart the app.

```json
{
  "mcpServers": {
    "arete": {
      "command": "a4-mcp"
    }
  }
}
```

Without a global install, use the npm wrapper instead:

```json
{
  "mcpServers": {
    "arete": {
      "command": "npx",
      "args": ["-y", "@usearete/mcp"]
    }
  }
}
```

### Cursor

Global config at `~/.cursor/mcp.json` or workspace-scoped at
`.cursor/mcp.json` in the project root (commit-friendly). Restart Cursor or
toggle the server in **Settings → MCP** after edits. The first tool call
prompts for trust. Cursor does **not** interpolate `${env:VAR}` from the host
shell — any `env` values must be literal.

```json
{
  "mcpServers": {
    "arete": {
      "command": "a4-mcp",
      "args": []
    }
  }
}
```

Without a global install:

```json
{
  "mcpServers": {
    "arete": {
      "command": "npx",
      "args": ["-y", "@usearete/mcp"]
    }
  }
}
```

### VS Code

Native MCP support. Workspace-scoped config at `.vscode/mcp.json`, or open
the user-level file via the command palette: **MCP: Open User Configuration**.
Note the top-level key is `servers` (singular), not `mcpServers`. VS Code
hot-reloads the file — no restart needed — and prompts for trust before first
start. Tools surface in Copilot Chat's agent mode.

```json
{
  "servers": {
    "arete": {
      "command": "a4-mcp",
      "args": []
    }
  }
}
```

Without a global install:

```json
{
  "servers": {
    "arete": {
      "command": "npx",
      "args": ["-y", "@usearete/mcp"]
    }
  }
}
```

### Claude Code (Anthropic CLI)

Use the `claude mcp add` subcommand. The `--` separator is required so Claude
doesn't try to parse `a4-mcp`'s own flags. Scope is one of `local` (default,
current project, private), `project` (writes `.mcp.json` at repo root,
shared), or `user` (all projects on your machine):

```bash
claude mcp add --transport stdio arete --scope user -- a4-mcp
```

Without a global install:

```bash
claude mcp add --transport stdio arete --scope user -- npx -y @usearete/mcp
```

### Zed

Zed calls them "context servers". Add to `settings.json` via
`zed: open settings`. Zed reloads on save; no restart required. Status is
visible in the Agent Panel (green dot = active).

```json
{
  "context_servers": {
    "arete": {
      "command": "a4-mcp",
      "args": [],
      "env": {}
    }
  }
}
```

Without a global install:

```json
{
  "context_servers": {
    "arete": {
      "command": "npx",
      "args": ["-y", "@usearete/mcp"],
      "env": {}
    }
  }
}
```

### Windsurf (Codeium)

Config file: `~/.codeium/windsurf/mcp_config.json`. Same shape as Claude
Desktop. **Restart Windsurf after editing** — config is not hot-reloaded.
Cascade has a hard cap of 100 total tools across all enabled MCP servers, so
budget accordingly.

```json
{
  "mcpServers": {
    "arete": {
      "command": "a4-mcp",
      "args": []
    }
  }
}
```

Without a global install:

```json
{
  "mcpServers": {
    "arete": {
      "command": "npx",
      "args": ["-y", "@usearete/mcp"]
    }
  }
}
```

### Continue.dev

Continue uses YAML. Either inline in `config.yaml`:

```yaml
mcpServers:
  - name: arete
    type: stdio
    command: a4-mcp
    args: []
```

Without a global install:

```yaml
mcpServers:
  - name: arete
    type: stdio
    command: npx
    args: ["-y", "@usearete/mcp"]
```

…or as a standalone file at `.continue/mcpServers/arete.yaml`:

```yaml
name: arete
version: 0.0.1
schema: v1
mcpServers:
  - name: arete
    type: stdio
    command: a4-mcp
    args: []
```

Without a global install:

```yaml
name: arete
version: 0.0.1
schema: v1
mcpServers:
  - name: arete
    type: stdio
    command: npx
    args: ["-y", "@usearete/mcp"]
```

MCP tools in Continue are only available in **agent mode**, not chat or edit.

## Tool reference

All tools are stateful: a typical session calls `connect` once, then
`subscribe`, then queries the cache via `get_entity` / `list_entities` /
`get_recent` / `query_entities`.

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
`a4-mcp`. Instead, `a4-mcp` resolves the key itself using this precedence:

1. **Explicit `api_key` argument** on the `connect` call (override, useful
   for testing or multi-stack setups)
2. **`ARETE_API_KEY` env var** set in the MCP server's process
   environment — the recommended pattern for headless/CI use. Set it in
   `.vscode/mcp.json`'s `env` block, or via
   `claude mcp add -e ARETE_API_KEY=arete_... arete -- a4-mcp` (legacy `hsk_...` keys still work).
3. **`~/.arete/credentials.toml`** — the file managed by the CLI's
   `a4 auth login` command. Both schemas the CLI writes are supported:

   ```toml
   # New format (URL-keyed, written by recent `a4 auth login`):
   [keys]
   "https://api.arete.run" = "arete_..."

   # Legacy format (top-level key, still honored):
   api_key = "arete_..."  # older keys may use hsk_ / hspk_ prefixes
   ```

   The file lookup honors `ARETE_API_URL` if set; otherwise falls back
   to `https://api.arete.run`.

If none of the three produces a key **and** the target stack URL is a
hosted Arete stack (`*.stack.arete.run`), `connect` fails
with a descriptive error telling the agent exactly what to do. Self-hosted
/ custom WebSocket URLs are allowed to proceed without a key because they
may not require auth at all.

The `connect` tool response includes a `key_source` field identifying
which of the three lookup paths won (`explicit_argument`,
`env:ARETE_API_KEY`, `~/.arete/credentials.toml`, or `none`).
The key itself is never included in responses or log output.

### Subscription management

- `subscribe({ connection_id, view, key?, with_snapshot? })` — subscribe to a
  view (e.g. `PumpfunToken/list` or `OreRound/latest`). Optional `key` narrows
  to a single entity. Returns a `subscription_id`. The subscription is
  multiplexed over the existing WebSocket — no extra connections are opened.
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

### Querying the cache

Streamed entities land in an in-memory cache (the SDK's `SharedStore`, LRU
with a 10k-entry-per-view default). Every query tool below reads from that
cache and is bound to a `subscription_id` so the agent doesn't have to repeat
view names.

- `get_entity({ subscription_id, key })` — fetch one entity by key.
- `list_entities({ subscription_id })` — keys only (no values), to keep the
  response small even on 10k-entity views.
- `get_recent({ subscription_id, n })` — up to N entities. Order matches the
  view's sort config when configured, otherwise hash order — not strict
  insertion recency.
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

### Health

- `ping()` — returns `pong`. Used by clients to verify the server is up.

## Example session (JSON-RPC over stdio)

```jsonc
// 1. Open connection
{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"connect","arguments":{"url":"wss://demo.stack.arete.run"}}}
// → {"connection_id":"a1b2..."}

// 2. Subscribe to a view
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"subscribe","arguments":{"connection_id":"a1b2...","view":"OreRound/latest"}}}
// → {"subscription_id":"c3d4..."}

// 3. Query
{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"query_entities","arguments":{
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
RUST_LOG=hs_mcp=debug,arete_sdk=info a4-mcp
```

## Status

Tracks Linear issue HYP-189. Triggers (`add_trigger` / `get_triggered`) and an
HTTP/SSE transport are planned for v2.
