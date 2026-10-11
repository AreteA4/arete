<!-- BEGIN:arete v2 -->
## Arete

This project uses Arete for typed Solana views and program operations. The
`a4` CLI is the interface; the installed `arete`, `arete-streams`,
`arete-programs`, `arete-stack-authoring`, and `arete-deploy` skills hold the
detailed workflows.

- Health check first: `a4 doctor --json` (ready when top-level `status` is
  `"ok"`; exit 0 can include warnings). If `a4` is
  missing: `curl -fsSL https://arete.run/install.sh | sh`
- Start from intent with `a4 know search --query "..." --json`, then inspect
  exact descriptors with `a4 explore stack <ref> --json` or
  `a4 explore program <ref> --json`.
- Never guess schemas or SDK methods. Generate clients from the explored
  descriptor with `a4 install stack <ref> --ts` or
  `a4 install program <ref> --ts`; use `--rust` or `--python` only when the
  descriptor advertises that target.
- Before writing SDK code, read the installed SDK's reference: the
  `README.md` in its generated folder, `a4 sdk describe <alias>`, or the
  `describe_sdk` MCP tool. SDK rows use camelCase paths (`id.roundId`); `a4
  get` and the MCP tools print wire names (`id.round_id`).
- A stack includes the program SDKs for the programs its views index, at
  `arete.programs.<name>`; install a program separately only when no stack
  you use covers it.
- Starting a new app? `a4 create <dir> --template react-ore` scaffolds a
  working example in a new directory (also `typescript-ore`, `rust-ore`,
  `python-ore`).
- Account: `a4 --profile agent auth signup` stores a restricted `a4_ak_*` key
  in the `agent` profile. Never request or use the human `a4_sk_*` key.
- Live data in your loop: the `arete` MCP server
  (`a4 --profile agent mcp`) is configured;
  use it for exploration, use generated SDKs for shipped code.
- Building or preparing does not authorize transaction submission or hosted
  deployment. Keep external mutations within the user's request.
- Never `cargo install a4-cli`; update with `a4 self update`.

Docs: https://docs.arete.run (agent entry: https://docs.arete.run/agent.md)
<!-- END:arete -->
