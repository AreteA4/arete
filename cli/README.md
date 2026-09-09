# a4-cli

[![crates.io](https://img.shields.io/crates/v/a4-cli.svg)](https://crates.io/crates/a4-cli)
[![docs.rs](https://docs.rs/a4-cli/badge.svg)](https://docs.rs/a4-cli)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

Command-line tool for building, deploying, and managing Arete stream stacks.

## Installation

Prebuilt, signed binary. No Rust toolchain or account required:

```bash
curl -fsSL https://arete.run/install.sh | sh        # macOS / Linux
irm https://arete.run/install.ps1 | iex             # Windows PowerShell
npx @usearete/a4 install                            # if you prefer npm
```

The binary is installed to `~/.local/bin/a4` (override with `A4_INSTALL_DIR`),
the installer adds that directory to your shell profile (skip with
`A4_NO_MODIFY_PATH=1`) and prints `A4_BIN=<absolute path>` as its last-but-one
line. If `a4` is not found in an already-open shell, use that path or run
`export PATH="$HOME/.local/bin:$PATH"`.

```bash
a4 self update            # update in place (alias: a4 upgrade)
a4 self update --check    # exit 10 if a newer version exists
a4 self uninstall         # remove the binary, receipt, and PATH lines
```

### Building from source

Only for unreleased builds. A Cargo-built binary cannot `a4 self update`;
rebuild with `cargo install a4-cli --force` instead.

```bash
cargo install a4-cli
# or from a checkout:
git clone https://github.com/AreteA4/arete.git
cd arete
cargo install --path cli
```

## Quick Start

```bash
# Set up the project: arete.toml, AGENTS.md block, CLAUDE.md import,
# agent skills, and MCP config for every detected coding agent
a4 init -y

# Verify (exit 0 = ready; each check carries a fix)
a4 doctor --json

# Discover live data (no account needed)
a4 explore --json
a4 explore stack ore --json

# Need an account (deploying, knowledge layer)?
a4 auth signup                      # register as an agent, stores the key
a4 auth login --key <a4_ak_...>     # or use a human-issued key

# Build explicit artifacts and deploy the exact manifest
cargo build
a4 up .arete/MyStack.stack-manifest.json
```

The deployment returns operational bindings for the exact StackManifest.

## Command Overview

| Command | Description |
|---------|-------------|
| `a4 init` | Set up the project: manifest, `AGENTS.md`, skills, MCP config |
| `a4 doctor` | Check the install, project, auth, network, and agent setup |
| `a4 install program <alias\|upr_...>` / `a4 install stack <atom-name>` | Install an owner-private program or stack through the manifest resolver (login required; exact lock; portable local alias) |
| `a4 self install\|update\|uninstall` | Manage the `a4` binary (`a4 upgrade` = `self update`) |
| `a4 mcp` | Run the stream MCP server over stdio |
| `a4 auth signup` | Register an agent account and store the key |
| `a4 program build <idl>` | Build a portable ProgramSpec |
| `a4 program push <idl-or-program-spec>` | Upload an owner-private hosted ProgramSpec |
| `a4 program status <upr-id>` | Inspect admission and runtime health |
| `a4 stack compose` | Compose ProgramSpecs and aliased LiveSpecs |
| `a4 up <manifest>` | Deploy an exact StackManifest |
| `a4 status` | Show project overview |
| `a4 install` | Resolve and install the project dependency graph |
| `a4 update [kind] [alias]` | Advance selected registry dependencies |
| `a4 remove <kind> <alias>` | Remove dependency intent, lock, and owned output |
| `a4 stack list` | List all stacks |
| `a4 stack show <name>` | Show stack details |

## Private Program Uploads

Uploads are explicit and never happen as a side effect of `a4 up`:

```bash
a4 program push ./idl.json --program-id <PUBKEY> --alias my-program --wait
a4 install program my-program --ts
# The stable ID returned by push is an unambiguous fallback:
a4 install program upr_... --ts
a4 program list
# Continue when the previous page prints "Next cursor":
a4 program list --cursor upc_...
a4 program status upr_... --watch
a4 program events upr_...
# Continue when the previous page prints "Next cursor":
a4 program events upr_... --after uev_...
a4 program archive upr_... --yes
a4 program promote upr_... --make-idl-public
```

Every upload begins owner-private. Promotion consent means the baseline IDL may
be reviewed and committed to a public OSS repository; it does not grant a
managed or public release automatically. Archival retains immutable content
while references exist. Private installs require the credentials saved by
`a4 auth login` and resolve only the caller's exact alias or `upr_...` ID. They
do not appear in `a4 explore programs`. Managed registry names take precedence
over private aliases, so use the stable ID if an alias collides.

## Daily Workflow

```bash
# Make changes to your stack, rebuild
cargo build

# Deploy
a4 up .arete/MyStack.stack-manifest.json

# Check status
a4 status
```

## Stack Commands

### `a4 stack list`

List all stacks with deployment status:

```
STACK              STATUS     VERSION  URL
settlement-game    active     v3       wss://settlement-game.stack.arete.run
token-tracker      active     v1       wss://token-tracker.stack.arete.run
```

### `a4 stack show <name>`

Show detailed information:

```bash
a4 stack show settlement-game
```

Shows: entity info, deployment status, version history, recent builds.

### `a4 stack versions <name>`

Show version history:

```bash
a4 stack versions settlement-game --limit 10
```

### `a4 stack delete <name>`

Durably destroy a stack:

```bash
a4 stack delete settlement-game
```

The command submits one server-side destroy operation and waits for its terminal
result. It removes stack-owned Kubernetes runtime resources and mutable stack
metadata. Deployment tombstones and immutable build, composition, operation,
event, and usage history remain available for audit. If the CLI times out, the
server operation continues; rerunning with `--force` safely reuses or retries
the durable operation and only skips the local name confirmation.

## Deployment

### `a4 up <manifest>`

Deploy one exact local StackManifest:

```bash
a4 up .arete/MyStack.stack-manifest.json
a4 up .arete/MyStack.stack-manifest.json --branch staging
a4 up .arete/MyStack.stack-manifest.json --preview
a4 up .arete/MyStack.stack-manifest.json --allow-unverified-programs
```

The last flag is explicit consent to persist a V2 deployment plan containing
owner-private, observed-executable programs. It is never inferred from upload
and does not make a program global or public.

## Authentication

```bash
a4 auth signup [name]           # Register an agent (5 per hour per IP); --force to replace saved credentials
a4 auth signup --json           # Also prints "apiKey" (a secret) for ARETE_API_KEY in sub-processes
a4 auth login --key <a4_ak_...> # Use a human-issued key; prompts only in an interactive terminal
a4 auth logout
a4 auth status
a4 auth whoami                  # Verify with server
```

Credentials: `~/.arete/credentials.toml`

## Agent Setup

```bash
a4 init -y                # idempotent; every write reports created | updated | unchanged
a4 init -y --global       # skills and MCP config for your user instead of the project
a4 init -y --dry-run --json
a4 doctor                 # one line per check: ok | warn | fail | info
a4 doctor --fix           # re-runs the init writers for every agents.* warning
```

`a4 init` writes `arete.toml`, a managed block in `AGENTS.md`, an `@AGENTS.md`
import in `CLAUDE.md`, the five Arete workflow skills
(via `npx skills add AreteA4/skills`; skipped when `npx` is missing), and the
`arete` (`a4 mcp`) and `arete-docs` (`https://docs.arete.run/mcp`) MCP servers
for every detected agent. `a4 doctor` exits 0 for `ok`/`warn`, 1 for `fail`.

## MCP Server

```bash
a4 mcp
```

Stream MCP server over stdio (registry discovery, knowledge layer, live entity
reads). `a4 init` writes the config; the manual shape for Claude Code is
`{"mcpServers":{"arete":{"type":"stdio","command":"a4","args":["mcp"]},"arete-docs":{"type":"http","url":"https://docs.arete.run/mcp"}}}`.

## Registry Exploration

Exploration uses the same deployment-pinned install descriptors as `a4
install`, so the reported StackManifest, LiveSpec, AST, and Program Release
identities are the ones an installation will consume.

```bash
a4 explore                              # List stacks
a4 explore programs                     # List complete installable programs
a4 explore stack ore --json             # Exact stack descriptor summary
a4 explore program spl-token --json     # Accounts, instructions, and release
```

Legacy stack forms remain valid:

```bash
a4 explore ore
a4 explore ore OreRound
```

Every JSON explore response includes `schemaVersion`. Stack exploration shows
LiveSpec aliases without flattening multi-live compositions and includes only
the views selected by the exact StackManifest. If descriptor assembly fails,
the command reports the deployment/publication problem instead of falling back
to a different AST.

## SDK Generation

```bash
a4 install ore-stack-abc123 --ts              # Install a published hosted stack SDK
a4 install ore-stack-abc123 --rust            # Install a published hosted Rust stack SDK
a4 install program spl-token --ts             # Install a published hosted program SDK
a4 install program my-program --ts             # Install your ready owner-private program
a4 sdk list                                   # List available stacks
a4 sdk create --manifest .arete/MyStack.stack-manifest.json --ts
a4 sdk create --manifest .arete/MyStack.stack-manifest.json --rust
a4 sdk create --program-spec .arete/token.program-spec.json --program-only --ts
```

SDK generation writes local source and does not publish a package.

## Configuration

**File:** `arete.toml`

```toml
manifest_version = 1

[project]
name = "my-project"
private = true

[sdk]
targets = ["typescript", "rust"]

[sdk.typescript]
output_dir = "./generated/typescript"
package = "@myorg/my-sdk"

[dependencies.stacks.ore]
source = { registry = "ore" }
version = "^1.0.0"

[authoring.stacks.local]
manifest = "./.arete/SettlementGame.stack-manifest.json"
artifact_roots = ["./.arete"]
```

Default outputs are separated by dependency kind. TypeScript installs use
`<output_dir>/stacks/<alias>` and `<output_dir>/programs/<alias>`; Rust and
Python use the same kind directories with `<alias>-stack` and
`<alias>-program` leaf names (including any configured prefix). A stack and a
program may therefore use the same local alias. Explicit dependency `outputs`
remain exact path overrides.

Install every declared dependency and write a deterministic lockfile with:

```bash
a4 install
a4 install --locked
a4 update stack ore
a4 remove stack ore
```

`a4 remove` deletes only SDK output carrying matching project provenance and
refuses directories containing unowned files. Pass `--keep-output` to retain
the generated directory while removing the manifest and lock entries.

## Endpoint and DNS Handoff

Live, Program Read, chain, and transaction endpoints are independent bindings.
Operators map them through their chosen DNS/CDN provider and publish generated
SDK packages manually. Hosted TypeScript, Python, and Rust installs preserve
the full Solana gateway descriptors. Their ordinary clients select the hosted
chain and transaction transports automatically; explicit transports are
overrides. TypeScript compositions also retain a
`create<StackName>HostedSession` convenience helper. Local/self-hosted output
does not contain hosted bindings and keeps using explicitly configured or
tenant-local transports.

## Environment Variables

| Variable | Description |
|----------|-------------|
| `ARETE_API_URL` | Override API endpoint |
| `ARETE_API_KEY` | API key; takes precedence over the credentials file |
| `ARETE_CREDENTIALS_PATH` | Override the credentials file (useful for isolated local testing) |
| `A4_INSTALL_DIR` | Install directory for `a4 self install` (default `~/.local/bin`) |
| `A4_NO_MODIFY_PATH=1` | Do not edit shell profiles or the Windows PATH on install |
| `A4_NO_UPDATE_CHECK=1` | Disable the once-per-day update notice |
| `A4_NON_INTERACTIVE=1` | Never prompt; missing inputs are errors that name the flag to pass |
| `DO_NOT_TRACK=1` | Disable telemetry |

## Troubleshooting

| Error | Solution |
|-------|----------|
| `Not authenticated` | Run `a4 auth signup` (or `a4 auth login --key <a4_ak_...>`) |
| `a4: command not found` | Run `export PATH="$HOME/.local/bin:$PATH"` or use the `A4_BIN=` path the installer printed |
| `a4 was not installed by the Arete installer` | Reinstall with `curl -fsSL https://arete.run/install.sh \| sh` |
| `Stack not found` | Check `a4 stack list` |
| `StackManifest not found` | Run `cargo build` and use the generated manifest path |
| `Build failed` | Check `a4 status` for build details |

## License

Apache-2.0

## Neutral workspace agent renderer

`a4 workspace-agents --request REQUEST.json` accepts schema version 1: an explicit target, selected harness IDs, complete local skill source directories, neutral MCP definitions, instructions, and existing configuration with owned entries. `--protocol-info` reports supported schemas. It returns planned files/copies, shared configuration entries and activation requirements as JSON. The caller owns validation, filesystem transactions, exclusion receipts, and native readiness checks; rendering does not write the target or create a consumer `arete.toml`.

The renderer reuses the public MCP writers for Codex, Claude Code, OpenCode, oh-my-pi and Cursor. It does not load private workspace manifests or preferences. Ordinary public commands ignore the optional inert `arete-dev.toml`; `ARETE_DEV_HOME` never redirects the public receipt directory selected by `ARETE_HOME` or its default `~/.arete`.
