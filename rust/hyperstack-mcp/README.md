# hyperstack-mcp

MCP (Model Context Protocol) server that wraps HyperStack streams for AI agent
integration. Lets Claude, GPT, and other MCP-compatible agents connect to a
HyperStack stack, subscribe to views, and query cached entities — using the
same primitives a human operator uses through `hs stream`.

The binary is `hs-mcp` and speaks MCP over stdio.

## Install

From a checkout of the workspace:

```bash
cargo install --path rust/hyperstack-mcp
```

This installs an `hs-mcp` binary into `~/.cargo/bin`.

## Use with Claude Desktop

Add the following to your `claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "hyperstack": {
      "command": "hs-mcp"
    }
  }
}
```

Restart Claude Desktop. The agent will now have access to all `hyperstack-mcp`
tools listed below. To talk to a private stack, the agent passes the stack URL
and your publishable API key on the `connect` call — no environment variables
required.

## Tool reference

All tools are stateful: a typical session calls `connect` once, then
`subscribe`, then queries the cache via `get_entity` / `list_entities` /
`get_recent` / `query_entities`.

### Connection management

- `connect({ url, api_key? })` — open a WebSocket to a HyperStack stack.
  Returns a `connection_id`. `api_key` is treated as a publishable key.
- `disconnect({ connection_id })` — close a connection. Also drops every
  subscription bound to it.
- `list_connections()` — id, URL, current connection state.

### Subscription management

- `subscribe({ connection_id, view, key?, with_snapshot? })` — subscribe to a
  view (e.g. `OreRound/latest`). Optional `key` narrows to a single entity.
  Returns a `subscription_id`. The subscription is multiplexed over the
  existing WebSocket — no extra connections are opened.
- `unsubscribe({ subscription_id })` — cancel.
- `list_subscriptions({ connection_id? })` — list active subscriptions,
  optionally filtered by connection.

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

  - `where: string[]` — the same predicate DSL as `hs stream --where`:
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
{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"connect","arguments":{"url":"wss://demo.stack.usehyperstack.com"}}}
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
RUST_LOG=hs_mcp=debug,hyperstack_sdk=info hs-mcp
```

## Status

Tracks Linear issue HYP-189. Triggers (`add_trigger` / `get_triggered`) and an
HTTP/SSE transport are planned for v2.
