<!-- BEGIN:arete v1 -->
## Arete

This project uses Arete (real-time Solana data streams). The `a4` CLI is
the interface; the `arete`, `arete-consume` and `arete-build` skills hold
the detailed patterns.

- Health check first: `a4 doctor --json` (exit 0 = ready). If `a4` is
  missing: `curl -fsSL https://arete.run/install.sh | sh`
- Never guess schemas. Before writing Arete code run
  `a4 explore --json`, then `a4 explore stack <ref> --json` or
  `a4 explore <stack> <Entity> --json`.
- Generate clients, don't hand-write them: `a4 install <ref> --ts|--rust`.
- Account: `a4 auth signup` (agent) or `a4 auth login --key <a4_ak_…>`.
- Live data in your loop: the `arete` MCP server (`a4 mcp`) is configured;
  use it for exploration, use generated SDKs for shipped code.
- Never `cargo install a4-cli`; update with `a4 self update`.

Docs: https://docs.arete.run (agent entry: https://docs.arete.run/agent.md)
<!-- END:arete -->
