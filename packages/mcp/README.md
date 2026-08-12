# Arete MCP Server

Model Context Protocol server for Arete streams.

## Quick Start

```bash
npx @usearete/mcp
```

That launches the `a4-mcp` stdio server.

## Installation

### npm

```bash
npm install -g @usearete/mcp
```

### Cargo

```bash
cargo install arete-mcp
```

## Usage with MCP clients

Use the npm wrapper through `npx` if you do not want a global install.

### Claude Desktop

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

### Claude Code

```bash
claude mcp add --transport stdio arete --scope user -- npx -y @usearete/mcp
```

If you install the package globally, it provides the same `a4-mcp` command as the Rust binary.

## Tools

**Discovery** — read-only lookups against the public registry. No auth required.

| Tool                   | Purpose                                                          |
| ---------------------- | ---------------------------------------------------------------- |
| `explore_stacks`       | List stacks. Returns the `websocketUrl` that `connect` takes.    |
| `explore_stack`        | Pinned install descriptor for one stack                          |
| `explore_stack_schema` | Entity/view schema — the `<EntityName>/<view>` ids for subscribe |
| `explore_programs`     | List installable standalone programs                             |
| `explore_program`      | Pinned install descriptor for one program                        |
| `resolve_artifact`     | Fetch a content-addressed artifact by kind and hash              |

**Streaming** — stateful; `connect` first, then `subscribe`, then query.

| Tool                                  | Purpose                                              |
| ------------------------------------- | ---------------------------------------------------- |
| `connect` / `disconnect`              | Open and close a WebSocket to a stack                |
| `subscribe` / `unsubscribe`           | Bind a view; entities land in an in-memory cache     |
| `query_entities`                      | Filter and project cached entities                   |
| `get_entity`                          | Fetch one cached entity by key                       |
| `list_entities`                       | List cached keys (capped at 1000 per response)       |
| `get_recent`                          | Up to N entities from the ordered query membership   |
| `list_subscriptions` / `list_connections` | Inspect current state                            |
| `ping`                                | Health check                                         |

Do not pass `api_key` in tool calls. Run `a4 auth login` once, or set
`ARETE_API_KEY` in the server's environment.

## Documentation

- [MCP usage guide](https://github.com/AreteA4/arete/tree/main/rust/arete-mcp)
- [Arete docs](https://docs.arete.run)

## License

Apache-2.0
