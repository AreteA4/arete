# a4-cli

[![crates.io](https://img.shields.io/crates/v/a4-cli.svg)](https://crates.io/crates/a4-cli)
[![docs.rs](https://docs.rs/a4-cli/badge.svg)](https://docs.rs/a4-cli)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

Command-line tool for building, deploying, and managing Arete stream stacks.

## Installation

```bash
cargo install a4-cli
```

### From Source

```bash
git clone https://github.com/AreteA4/arete.git
cd arete
cargo install --path cli
```

## Quick Start

```bash
# Initialize project
a4 init

# Authenticate
a4 auth login

# Build explicit artifacts and deploy the exact manifest
cargo build
a4 up .arete/MyStack.stack-manifest.json
```

The deployment returns operational bindings for the exact StackManifest.

## Command Overview

| Command | Description |
|---------|-------------|
| `a4 init` | Initialize project |
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

Delete a stack:

```bash
a4 stack delete settlement-game
```

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
a4 auth login       # Login
a4 auth logout      # Logout
a4 auth whoami      # Verify with server
```

Credentials: `~/.arete/credentials.toml`

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
| `ARETE_CREDENTIALS_PATH` | Override the credentials file (useful for isolated local testing) |

## Troubleshooting

| Error | Solution |
|-------|----------|
| `Not authenticated` | Run `a4 auth login` |
| `Stack not found` | Check `a4 stack list` |
| `StackManifest not found` | Run `cargo build` and use the generated manifest path |
| `Build failed` | Check `a4 status` for build details |

## License

Apache-2.0
