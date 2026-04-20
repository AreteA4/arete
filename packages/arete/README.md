# Arete CLI

Programmable data feeds for Solana. Arete CLI lets you deploy feeds for any Solana data to our cloud.

## Quick Start

```bash
npx @usearete/a4 create my-app
cd my-app
npm run dev
```

## Installation

### npm (recommended for JS/TS developers)

```bash
npm install -g @usearete/a4
```

### Cargo (for Rust developers)

```bash
cargo install a4-cli
```

## Commands

| Command | Description |
|---------|-------------|
| `a4 create [name]` | Scaffold a new app from a template |
| `a4 init` | Initialize a stack project |
| `a4 up` | Deploy your stack |
| `a4 status` | Show project overview |
| `a4 auth login` | Authenticate with Arete |

The npm package `@usearete/a4` and the Cargo crate `a4-cli` both install the same `a4` command.

## Documentation

- [Getting Started](https://docs.arete.run)
- [CLI Reference](https://docs.arete.run/cli/commands/)
- [Stack Development](https://docs.arete.run/building-stacks/workflow/)

## License

Apache-2.0
