<!-- BEGIN:arete v2 -->
## Arete

This project uses Arete for Solana protocols, typed views and program
operations. The `a4` CLI is the interface; the six installed skills hold the
detailed workflows: `arete` (discovery and dependencies), `arete-streams`
(live views), `arete-programs` (accounts and transactions),
`arete-protocols` (authoring, running and calling protocols),
`arete-stack-authoring` (custom read models) and `arete-deploy` (publication
and deployment).

What Arete can do, for Solana programs and data: compose existing programs
into protocols with typed steps, guards and roles, develop them in cloud
mainnet forks (`a4 dev`), and call them from apps and agents; find programs
and live-data stacks from intent in a public catalog; read live views once
(`a4 get`, MCP `read_view`) or stream them; generate typed TypeScript/React,
Rust and Python SDKs; read program accounts and chain state; derive PDAs and
build instructions, transactions and multi-step flows, signed and submitted
through application wallets; and author and deploy custom live read models
(stacks).

- Health check first: `a4 doctor --summary --json` (ready when top-level
  `status` is `"ok"`; exit 0 can include warnings; `checks` lists only what
  needs attention, and an `info` check there is optional unless you need
  that capability). If `a4` is missing:
  `curl -fsSL https://arete.run/install.sh | sh`
- New protocol: `a4 create <dir> --template protocol` (a tip jar with
  automations), or `--template protocol-app` (plus a React app on the
  protocol client and an automations server with a Privy signer). Then
  `cd <dir> && a4 init -y && a4 dev up`; the `arete-protocols` skill has the
  workflow and its design patterns.
- Catalog or knowledge: the catalog (`a4 explore catalog --query "..."
  --json`) finds installable programs and stacks; knowledge
  (`a4 know search --query "..." --json`) explains what an on-chain protocol
  does and what its instructions mean. Installs always come from the catalog
  descriptor (`a4 explore stack <ref> --json`,
  `a4 explore program <ref> --json`), never from knowledge results.
- One-off live read, no code: `a4 explore stack <ref> --json` lists the
  views, then `a4 get <Entity>/<view> --stack <ref> --limit 1 --select
  <paths>` (or the MCP `read_view` tool). `<Entity>/latest` is the current
  state. One healthy `a4 doctor` is enough, and a single read needs no skill
  reading.
- Never guess schemas or SDK methods. Generate clients from the explored
  descriptor with `a4 install stack <ref> --ts` or
  `a4 install program <ref> --ts`; use `--rust` or `--python` only when the
  descriptor advertises that target.
- Before writing SDK code, look up the part you use in the installed SDK's
  reference: `a4 sdk describe <alias> --read <name>` or `--view
  <Entity/view>` (or the `describe_sdk` MCP tool) prints it with the row
  fields it uses; the `README.md` in its generated folder indexes every part
  at the top. SDK rows use camelCase paths (`id.roundId`); `a4 get` and the
  MCP tools print wire names (`id.round_id`).
- A stack includes the program SDKs for the programs its views index, at
  `arete.programs.<name>`; install a program separately only when no stack
  you use covers it.
- Live data without a protocol? `a4 create <dir> --template react-ore`
  scaffolds the ORE live-data example (also `typescript-ore`, `rust-ore`,
  `python-ore`).
- Account: `a4 --profile agent auth signup` stores a restricted `a4_ak_*` key
  in the `agent` profile. Never request or use the human `a4_sk_*` key.
- Live data in your loop: the `arete` MCP server
  (`a4 --profile agent mcp`) is configured;
  use it for exploration, use generated SDKs for shipped code.
- MCP servers load when the host restarts. Until then, use the `a4` CLI
  equivalents (`a4 get`, `a4 know`, `a4 explore`, `a4 protocol actions`,
  `a4 dev step`). Don't wait for a restart.
- Building or preparing does not authorize transaction submission or hosted
  deployment. Keep external mutations within the user's request.
- Never `cargo install a4-cli`; update with `a4 self update`.

Docs: https://docs.arete.run (agent entry: https://docs.arete.run/agent.md)
<!-- END:arete -->
