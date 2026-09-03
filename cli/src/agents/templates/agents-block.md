<!-- BEGIN:arete v2 -->
## Arete

This project uses Arete for typed Solana views and program operations. The
`a4` CLI is the interface; the installed `arete`, `arete-streams`,
`arete-programs`, `arete-stack-authoring`, and `arete-deploy` skills hold the
detailed workflows.

- Health check first: `a4 doctor --json` (exit 0 = ready). If `a4` is
  missing: `curl -fsSL https://arete.run/install.sh | sh`
- Start from intent with `a4 know search --query "..." --json`, then inspect
  exact descriptors with `a4 explore stack <ref> --json` or
  `a4 explore program <ref> --json`.
- Never guess schemas or SDK methods. Generate clients from the explored
  descriptor with `a4 install stack <ref> --ts` or
  `a4 install program <ref> --ts`; use `--rust` or `--python` only when the
  descriptor advertises that target.
- Account: `a4 auth signup` (agent) or `a4 auth login --key <a4_ak_…>`.
- Live data in your loop: the `arete` MCP server (`a4 mcp`) is configured;
  use it for exploration, use generated SDKs for shipped code.
- Building or preparing does not authorize transaction submission or hosted
  deployment. Keep external mutations within the user's request.
- Never `cargo install a4-cli`; update with `a4 self update`.

Docs: https://docs.arete.run (agent entry: https://docs.arete.run/agent.md)
<!-- END:arete -->
