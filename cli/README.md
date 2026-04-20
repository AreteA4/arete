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
# Initialize project (auto-discovers AST files)
a4 init

# Authenticate
a4 auth login

# Deploy
a4 up
```

That's it! Your stack is deployed and you'll see the WebSocket URL.

## Command Overview

| Command | Description |
|---------|-------------|
| `a4 init` | Initialize project |
| `a4 up [stack]` | Deploy (push + build + deploy) |
| `a4 status` | Show project overview |
| `a4 stack list` | List all stacks |
| `a4 stack show <name>` | Show stack details |
| `a4 stack rollback <name>` | Rollback to previous version |

## Daily Workflow

```bash
# Make changes to your stack, rebuild
cargo build

# Deploy
a4 up

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

### `a4 stack push [name]`

Push local stacks to remote without deploying:

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

### `a4 up [stack-name]`

The happy path - push, build, and deploy in one command:

```bash
a4 up                              # Deploy all
a4 up settlement-game              # Deploy one
a4 up settlement-game --branch staging  # Branch deploy
a4 up settlement-game --preview    # Preview deployment
```

## Authentication

```bash
a4 auth register    # Create new account
a4 auth login       # Login
a4 auth logout      # Logout
a4 auth whoami      # Verify with server
```

Credentials: `~/.arete/credentials.toml`

## SDK Generation

```bash
a4 sdk list                                   # List available stacks
a4 sdk create typescript settlement-game      # Generate TypeScript SDK
a4 sdk create rust settlement-game            # Generate Rust SDK
```

## Configuration

**File:** `arete.toml`

```toml
[project]
name = "my-project"

[sdk]
output_dir = "./generated"

# Stacks - auto-discovered from .arete/*.ast.json
# Define explicitly for custom naming:
[[stacks]]
name = "my-game"
ast = "SettlementGame"
```

For most projects, you only need:

```toml
[project]
name = "my-project"
```

The CLI auto-discovers stacks from `.arete/*.ast.json` files.

## WebSocket URLs

| Type | Pattern |
|------|---------|
| Production | `wss://{stack-name}.stack.arete.run` |
| Branch | `wss://{stack-name}-{branch}.stack.arete.run` |

## Environment Variables

| Variable | Description |
|----------|-------------|
| `ARETE_API_URL` | Override API endpoint |

## Troubleshooting

| Error | Solution |
|-------|----------|
| `Not authenticated` | Run `a4 auth login` |
| `Stack not found` | Check `a4 stack list` |
| `AST file not found` | Run `cargo build` to generate AST |
| `Build failed` | Check `a4 status` for build details |

## License

Apache-2.0
