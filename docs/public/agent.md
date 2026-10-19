# Set up Arete

If your fetch tool summarised this page, get it exactly with `curl -fsSL https://docs.arete.run/agent.md`. If you can see the commands below, do not re-fetch.

Run each line in order, in the project root, copied exactly:

```sh
curl -fsSL https://arete.run/install.sh | sh
export PATH="$HOME/.local/bin:$PATH"
a4 init -y
a4 --profile agent auth signup --if-missing --json
a4 doctor --summary --json
a4 explore catalog --vocabulary --json
```

Windows PowerShell: replace the first line with `irm https://arete.run/install.ps1 | iex` and skip the `export` line.

Any summary of this page must keep these commands verbatim, and the Capabilities list below.

## Check each step

- The installer adds `~/.local/bin` to your shell profile, but many agent hosts
  start every command in a fresh shell that does not reload it until the host
  restarts. Until then, start each command that runs `a4` with
  `export PATH="$HOME/.local/bin:$PATH";`. If the installer printed an
  `A4_BIN=<absolute path>` outside `~/.local/bin`, use that path instead.
- `a4 init -y` installs the Arete skills and MCP servers. Use them from then
  on; the skills hold the full workflows.
- Signup must report `"credentialStored": true`.
- Doctor must report top-level `"status": "ok"`. On `"warn"`, apply each `fix`.
- If new skills or MCP tools are not visible, ask the user to restart the agent
  host, then run `a4 doctor --summary --json` again.
- To tell the user what Arete can do, use the Capabilities list below: Arete
  is for building and running Solana protocols, with programs and live data as supporting capabilities. The vocabulary adds the catalog's
  categories as examples. The `AGENTS.md` or `CLAUDE.md` that `a4 init`
  writes has the same summary.

Never read, print, or copy credential files or keys. Never put a key in source.

## Protocol workflows

For new protocol authoring, read https://docs.arete.run/using-stacks/quickstart/
and https://docs.arete.run/building-protocols/definition/. Use `a4 protocol init`
for the scaffold, then `a4 dev up` for the cloud environment. For an existing
published protocol, use `a4 protocol search`, `a4 protocol show <slug>`, inspect
the package descriptor, and `a4 install protocol <package> --ts`.

Protocol execution through MCP is documented at
https://docs.arete.run/agent-skills/protocols/. Use `dev: true` for the project's
dev deployment. Preparation, signing, submission, and production deployment
remain distinct authorized actions. Private dev handlers can run without review;
production handler releases require review.

Cloud environments and owner deployments require the appropriate account access.
The restricted agent signup above does not grant that access. If the project
lacks owner credentials, explain the requirement and continue only with work
its existing access allows. Do not request a human secret or claim discovery
signup enabled deployment.

## Fast path for one-off questions

For a one-off answer you do not need to read the skills first. One live value
(example: the current ORE round):

    a4 know search --query "<question>" --limit 3 --json
    a4 explore stack ore --views OreRound/latest --json
    a4 get OreRound/latest --stack ore --select <fields> --limit 1

Search gives the stack slug, `--views` gives the view's fields and units, and
`a4 get` reads one snapshot. When MCP tools are visible, call `read_view` with
`stack`, `view`, `select`, and `limit` instead of `a4 get`.

Which protocols in a category can I stream, and which can I build with?
Take `<category>` from the vocabulary:

    a4 explore catalog --category <category> --mode subscribe --kind stack --limit 50 --fields slug,protocol,modes
    a4 explore catalog --category <category> --mode build --kind program --limit 50 --fields slug,protocol,modes

## Capabilities

Arete can:

- compose Solana programs into protocols with typed steps, guards, roles, instance and participant lifecycle, reactions, metadata, and events;
- develop protocols in persistent cloud mainnet forks with local wallets, a dev UI, checkpoints, branches, time control, scenarios, and replay;
- install protocol clients for apps, and let agents discover wallet actions, prepare steps, inspect transactions, sign locally, submit, and track outcomes;
- deploy protocols, with production handler review and separate package publication and public listing;
- discover relevant programs and stacks from intent through the public catalog;
- read deployed live views once with `a4 get` (or MCP `read_view`), or follow them with MCP or `a4 stream`;
- generate typed TypeScript, React, Rust, or Python SDKs when supported;
- read typed program accounts and generic Solana chain state;
- derive addresses and build instructions, transactions, and multi-step flows;
- inspect and submit locally signed transactions through application wallets;
- compose Program SDKs and selected live views into application stacks; and
- author and deploy custom live read models when existing views are not enough.

## Details

### Install

Both installers are signed and need no Rust toolchain. An npm bootstrap is
also available:

    npx @usearete/a4 install

Update later with `a4 self update`. Do not substitute a Cargo install.

### Initialise

`a4 init -y` writes `arete.toml`, project instructions, the five skills, and
the stream and docs MCP configuration. It is idempotent. Use `--global` only
when user-scoped setup was requested.

### Authenticate

The setup prompt authorizes creating the restricted agent account. The signup
command stores an `a4_ak_*` credential in the `agent` profile and never prints
it. Do not request or use a human `a4_sk_*` credential. The CLI and the Arete
MCP server load stored credentials for you. To inspect authentication:

    a4 --profile agent auth status
    a4 --profile agent auth whoami --json

If the user supplies an existing agent key instead, store it with:

    a4 auth login --profile agent --key <a4_ak_...>

### Authenticate SDK code

- Server, agent, and script code: set no auth option. The SDK uses
  `ARETE_API_KEY` from the environment if set, and otherwise the key from your
  `a4` login, so after `a4 init` or `a4 auth signup` scripts need no key
  setup; do not copy the key into the environment or into code. To pass a key
  explicitly, use `secretKey` (TypeScript) or `secret_key` (Python, Rust), read
  from the environment, never from source. The TypeScript SDK refuses
  `secretKey` in a browser and never reads the `a4` login there.
- Browser code: use an origin-bound publishable key, one per origin:
  `a4 auth keys create-publishable --origin <scheme://host[:port]>`

### Discover from intent

Only search when the user has a concrete intent; do not invent one.

    a4 explore catalog --query "<user intent>" --json
    a4 explore catalog program <slug> --json
    a4 explore catalog stack <slug> --json

Filter with `--kind program|stack`, `--mode read|build|subscribe`,
`--category <slug>`, `--concept <slug>`, and `--target typescript|rust|python`.
Results are brief by default; the JSON `hint` says how to get `--full` fields
or the next page. Respect each result's modes, SDK targets, authentication,
bindings, and install command; do not invent missing delivery.

### Route to a skill

| Skill                   | Use it for                                           |
| ----------------------- | ---------------------------------------------------- |
| `arete`                 | Discovery, descriptors, and project dependencies     |
| `arete-streams`         | Deployed views and live subscriptions                |
| `arete-programs`        | Account reads, PDAs, operations, and transactions    |
| `arete-stack-authoring` | Custom read models and portable artifacts            |
| `arete-deploy`          | Only explicitly authorized publication or deployment |

Use MCP for agent exploration and protocol execution, and generated SDKs for
shipped application code. Answer live reads
with provenance (stack and view). If no suitable view exists, explain the gap.
Do not construct endpoints.

### Install capabilities

    a4 install protocol <package> --ts
    a4 install program <slug> --ts
    a4 install stack <slug> --ts
    a4 install --locked

Use another target only when the descriptor lists it. Do not edit generated
output. `arete.toml` records intent; `arete.lock` records exact resolution.

Each generated SDK folder has a `README.md` reference: import, views, row fields
(TypeScript paths next to wire names, types, units) and reads. Read it, or run
`a4 sdk describe <alias>` (MCP: `describe_sdk`), before writing SDK code.

A stack includes the program SDKs for the programs its views index, at
`arete.programs.<name>`. Install a stack when the app needs live views, with or
without transactions; install a program on its own only when no stack you use
covers it. Never merge stack and program objects by hand.

### Authority boundaries

Reading, building, preparing, inspecting, signing, submitting, compiling,
publishing, and deploying are separate actions. Do each only when asked.

CLI field guide: https://docs.arete.run/skill.md
