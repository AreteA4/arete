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
| `a4 stack compose` | Compose ProgramSpecs and aliased LiveSpecs |
| `a4 up <manifest>` | Deploy an exact StackManifest |
| `a4 status` | Show project overview |
| `a4 stack list` | List all stacks |
| `a4 stack show <name>` | Show stack details |
| `a4 stack rollback <name>` | Rollback to previous version |

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

### `a4 stack push [name]` (legacy)

Push a legacy configured composite input. This compatibility path is retained
only through **August 31, 2026**:

```bash
a4 stack push                  # Push all
a4 stack push settlement-game  # Push one
```

### `a4 stack versions <name>`

Show version history:

```bash
a4 stack versions settlement-game --limit 10
```

### `a4 stack rollback <name>`

Rollback to a previous version:

```bash
a4 stack rollback settlement-game          # Previous version
a4 stack rollback settlement-game --to 2   # Specific version
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
```

Composite `.stack.json` is an input-only compatibility adapter through **August
31, 2026**. New deployments use the manifest path.

## Authentication

```bash
a4 auth login       # Login
a4 auth logout      # Logout
a4 auth whoami      # Verify with server
```

Credentials: `~/.arete/credentials.toml`

## SDK Generation

```bash
a4 install ore-stack-abc123 --ts              # Install a published hosted stack SDK
a4 install ore-stack-abc123 --rust            # Install a published hosted Rust stack SDK
a4 install program spl-token --ts             # Install a published hosted program SDK
a4 sdk list                                   # List available stacks
a4 sdk create --manifest .arete/MyStack.stack-manifest.json --ts
a4 sdk create --manifest .arete/MyStack.stack-manifest.json --rust
a4 sdk create --program-spec .arete/token.program-spec.json --program-only --ts
```

SDK generation writes local source and does not publish a package.

## Configuration

**File:** `arete.toml`

```toml
[project]
name = "my-project"

[sdk]
output_dir = "./generated"

# Legacy composite inputs (compatibility only through August 31, 2026)
[[stacks]]
name = "my-game"
stack = ".arete/SettlementGame.stack.json"
```

For most projects, you only need:

```toml
[project]
name = "my-project"
```

New SDK and deployment workflows pass an explicit StackManifest path.

## Endpoint and DNS Handoff

Live, Program Read, chain, and transaction endpoints are independent bindings.
Operators map them through their chosen DNS/CDN provider and publish generated
SDK packages manually. Hosted TypeScript composition output preserves the full
Solana gateway descriptors and exports a `create<StackName>HostedSession`
helper. That helper creates authenticated chain and transaction transports from
the generated bindings; the generic composition helper still requires explicit
transports.

## Environment Variables

| Variable | Description |
|----------|-------------|
| `ARETE_API_URL` | Override API endpoint |

## Troubleshooting

| Error | Solution |
|-------|----------|
| `Not authenticated` | Run `a4 auth login` |
| `Stack not found` | Check `a4 stack list` |
| `StackManifest not found` | Run `cargo build` and use the generated manifest path |
| `Build failed` | Check `a4 status` for build details |

## License

Apache-2.0
