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

If your MCP client supports a command plus args, you can run the wrapper through `npx` without a global install:

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

If you install the package globally, it provides the same `a4-mcp` command as the Rust binary.

## Documentation

- [MCP usage guide](https://github.com/AreteA4/arete/tree/main/rust/arete-mcp)
- [Arete docs](https://docs.arete.run)

## License

Apache-2.0
