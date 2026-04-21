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

## Documentation

- [MCP usage guide](https://github.com/AreteA4/arete/tree/main/rust/arete-mcp)
- [Arete docs](https://docs.arete.run)

## License

Apache-2.0
