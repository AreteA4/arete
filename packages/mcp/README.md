# @usearete/mcp (deprecated)

**@usearete/mcp is deprecated. Update your MCP config to run "a4 mcp" (see a4 init) or "npx @usearete/a4 mcp".**

The Arete MCP server now ships inside the `a4` CLI binary as `a4 mcp`. This
package no longer downloads or runs anything: its `a4-mcp` bin prints the
deprecation notice above to stderr and exits 1.

## Migrate

1. Install `a4` (one of):

   ```bash
   curl -fsSL https://arete.run/install.sh | sh
   npx @usearete/a4 install
   ```

2. Let `a4 init` write the MCP config for your agent (Claude Code, Cursor,
   Codex, OpenCode, Gemini CLI, Copilot, ...):

   ```bash
   a4 init -y
   ```

   Or edit the config yourself. Claude Code / Copilot CLI (`.mcp.json`):

   ```json
   {
     "mcpServers": {
       "arete": { "type": "stdio", "command": "a4", "args": ["mcp"] },
       "arete-docs": { "type": "http", "url": "https://docs.arete.run/mcp" }
     }
   }
   ```

   OpenCode (`opencode.json`):

   ```json
   {
     "$schema": "https://opencode.ai/config.json",
     "mcp": {
       "arete": { "type": "local", "command": ["a4", "mcp"], "enabled": true },
       "arete-docs": { "type": "remote", "url": "https://docs.arete.run/mcp", "enabled": true }
     }
   }
   ```

   Without a global `a4` on PATH, `npx -y @usearete/a4 mcp` works in place of
   `a4 mcp`.

3. Remove `@usearete/mcp` from your dependencies and MCP configs.

Authentication is unchanged: run `a4 auth signup` (or `a4 auth login --key
<a4_ak_...>`) once, or set `ARETE_API_KEY` in the server's environment. Never
pass the key as a tool-call argument.

## Documentation

- [Arete docs](https://docs.arete.run)
- [MCP server reference](https://github.com/AreteA4/arete/tree/main/rust/arete-mcp)

## License

Apache-2.0
