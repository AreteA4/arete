#!/usr/bin/env node

// @usearete/mcp is deprecated: the MCP server now ships inside the `a4`
// binary as `a4 mcp`. This shim only tells callers to update their config.
process.stderr.write(
  '@usearete/mcp is deprecated. Update your MCP config to run "a4 mcp" (see a4 init) or "npx @usearete/a4 mcp".\n'
);
process.exit(1);
