# Set up Arete

> Read this page verbatim. If a tool summarised it for you (for example
> WebFetch), run `curl -fsSL https://docs.arete.run/agent.md` and read the
> output instead. Copy commands exactly; do not paraphrase them.

## Do this first

Run these commands in order, in the project root. Each command is one line.

1. macOS / Linux: `curl -fsSL https://arete.run/install.sh | sh`
   Windows PowerShell: `irm https://arete.run/install.ps1 | iex`
2. `a4 init -y`
3. `a4 --profile agent auth signup --if-missing --json`
4. `a4 doctor --json`
5. `a4 explore catalog --vocabulary --json`

Before step 2: step 1 prints `A4_BIN=<absolute path>`. If `a4` is not found in
the current shell, run steps 2–5 with that absolute path in place of `a4`.

Then:

- Step 2 installs the Arete skills and MCP servers. Use the skills from then
  on; they hold the full workflows.
- Step 3 must report `"credentialStored": true`.
- Step 4 must report top-level `"status": "ok"`. On `"warn"`, apply each `fix`.
- If new skills or MCP tools are not visible, ask the user to restart the agent
  host, then run `a4 doctor --json` again.
- Step 5 lists current discovery categories. Use it with the capability list
  below to tell the user what Arete can do.

Never read, print, or copy credential files or keys. Never put a key in source.

## Capabilities

Arete can:

- discover relevant programs and stacks from intent through the public catalog;
- explore deployed live views through MCP or `a4 stream`;
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

`a4 init -y` writes `arete.toml`, managed project instructions, the five Arete
skills, and the Arete stream and documentation MCP configuration. It is
idempotent; do not rewrite config repeatedly. Use `--global` only when
user-scoped setup was requested.

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

Filter with `--kind program|stack`, `--mode read|build|subscribe`, and
`--target typescript|rust|python`. Results are brief by default; the JSON
`hint` says how to get `--full` fields or the next page. A catalog result is
not permission to invent missing delivery: respect its modes, SDK targets,
authentication, bindings, and install command.

### Route to a skill

| Skill                   | Use it for                                           |
| ----------------------- | ---------------------------------------------------- |
| `arete`                 | Discovery, descriptors, and project dependencies     |
| `arete-streams`         | Deployed views and live subscriptions                |
| `arete-programs`        | Account reads, PDAs, operations, and transactions    |
| `arete-stack-authoring` | Custom read models and portable artifacts            |
| `arete-deploy`          | Only explicitly authorized publication or deployment |

Use MCP for exploration and generated SDKs for shipped code. For a live view,
inspect the exact schema, connect with its descriptor, take a bounded sample,
answer with provenance, and disconnect. If no suitable view exists, explain the
gap. Do not construct endpoints.

### Install capabilities

    a4 install program <slug> --ts
    a4 install stack <slug> --ts
    a4 install --locked

Use another target only when the descriptor lists it. Do not edit generated
output. `arete.toml` records intent; `arete.lock` records exact resolution.

A stack includes the program SDKs for the programs its views index, at
`arete.programs.<name>`. Install a stack when the app needs live views, with or
without transactions; install a program on its own only when no stack you use
covers it. Never merge stack and program objects by hand.

### Authority boundaries

Reading, building, preparing, inspecting, signing, submitting, compiling,
publishing, and deploying are separate actions. Do each only when asked.

CLI field guide: https://docs.arete.run/skill.md
