# Arete

Real-time streaming data pipelines for Solana - transform on-chain events into typed state projections.

[![CI](https://github.com/AreteA4/arete/actions/workflows/ci.yml/badge.svg)](https://github.com/AreteA4/arete/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0%2FMIT-blue.svg)](#license)

## Packages

| Package | Language | Registry | Description |
|---------|----------|----------|-------------|
| arete | Rust | crates.io | Umbrella crate re-exporting all components |
| arete-interpreter | Rust | crates.io | AST transformation runtime and VM |
| arete-macros | Rust | crates.io | Proc-macros for stream definitions |
| arete-server | Rust | crates.io | WebSocket server and projection handlers |
| arete-sdk | Rust | crates.io | Rust client SDK |
| a4-cli | Rust | crates.io | CLI tool for SDK generation |
| arete-idl | Rust | crates.io | IDL parsing and type system |
| arete-hash | Rust | crates.io | Typed artifact identity and canonical hashing protocol |
| arete-artifacts | Rust | crates.io | Versioned public artifact schemas and legacy normalization |
| @usearete/hash | TypeScript | npm | TypeScript implementation of the artifact identity protocol |
| @usearete/sdk | TypeScript | npm | Pure TypeScript SDK (framework-agnostic) |
| @usearete/react | TypeScript | npm | React SDK with hooks |
| @usearete/adapter-kit | TypeScript | npm | Wallet adapter for `@solana/kit` |
| @usearete/adapter-web3js | TypeScript | npm | Wallet adapter for `@solana/web3.js` |
| arete-sdk | Python | PyPI | Python client SDK *(work in progress - not yet published)* |

## Quick Start

### Rust
Add to your `Cargo.toml`:
```toml
[dependencies]
arete = "0.5"
```

### TypeScript (Core)
```bash
npm install @usearete/sdk
```

### TypeScript / React
```bash
npm install @usearete/react @usearete/sdk zod
```

Generated React consumers use the React hooks, core SDK types, and generated Zod schemas. `zustand` is a normal dependency of `@usearete/react`; applications do not install it separately.

Hosted browser stacks require authentication. For the hosted ORE stack, set `VITE_ARETE_PUBLISHABLE_KEY` and pass it to the provider as `auth={{ publishableKey }}`. Read-only viewing does not require a wallet, but it does require the publishable key.

### Python
> **Note:** The Python SDK is a work in progress and has not yet been published to PyPI.

```bash
# Coming soon
pip install arete-sdk
```

## Artifact and Runtime Model

Arete keeps portable behavior separate from the infrastructure that serves it:

| Concept | What it represents |
|---------|--------------------|
| **ProgramSpec** | Portable program identity: program ID, normalized public IDL, account and instruction definitions, PDAs, and compatibility hashes. It contains no endpoint or managed decoder binding. |
| **LiveSpec** | Entities, mappings, handlers, computed fields, resolvers, and views over exact ProgramSpecs. |
| **StackManifest** | A client-facing composition of ProgramSpecs and aliased LiveSpecs, including the exact selected views. It contains no deployment URL. |
| **Program Release** | A hosted, immutable binding from one ProgramSpec to managed decoder behavior. Changing decoder semantics creates a new release. |
| **Deployment** | A hosted runtime prepared for one exact StackManifest. Images, regions, replicas, and rollout state belong here rather than in portable artifacts. |
| **Binding** | An operational endpoint and authentication attachment for a live deployment, Program Read release, chain reader, or transaction relay. Bindings can change without changing portable artifact hashes. |

The Rust DSL writes authoritative artifacts directly during compilation:

```bash
cargo build

# Typical outputs:
# .arete/<program>.program-spec.json
# .arete/<StackName>.live-spec.json
# .arete/<StackName>.stack-manifest.json
```

An IDL-only module generates ProgramSpecs, a zero-live StackManifest, and a
program-only `spec()` with generated account readers:

```rust
use arete::prelude::*;

#[arete(idl = ["idl/my_program.json"])]
mod my_program {}

let spec = my_program::spec();
```

Local `spec()` generation derives release fingerprints automatically from each
ProgramSpec and the generated decoder-engine contract. An OSS server does not
select or publish a hosted Program Release.

Use the explicit artifacts for CLI workflows:

```bash
# Create a standalone ProgramSpec directly from an IDL.
a4 program build ./idl/my_program.json \
  --output ./.arete/my_program.program-spec.json

# Compose one or more LiveSpecs under stable aliases. Repeating
# --selected-view creates the exact ordered client allowlist.
a4 stack compose --name my-app \
  --live core=./.arete/Core.live-spec.json \
  --artifact-dir ./.arete \
  --selected-view core=Position/list \
  --output ./.arete/MyApp.stack-manifest.json

# Generate local source, or deploy the exact manifest.
a4 sdk create --manifest ./.arete/MyApp.stack-manifest.json --ts
a4 sdk create --manifest ./.arete/MyApp.stack-manifest.json --rust
a4 up ./.arete/MyApp.stack-manifest.json

# Install a hosted SDK pinned to a published Program Release and read binding.
a4 install program spl-token --ts

# After an owner-private upload reaches ready, install it by alias or stable ID.
a4 program push ./idl/my_program.json --program-id <PUBKEY> --alias my-program --wait
a4 install program my-program --ts
a4 install program upr_... --ts

# Inspect the same deployment-pinned descriptors before installing.
a4 explore stack ore --json
a4 explore programs --json
a4 explore program spl-token --json
```

Owner-private install lookups use the credentials saved by `a4 auth login` and
are intentionally absent from registry discovery. A managed catalog name wins
an alias collision; the returned `upr_...` ID always identifies the owner's
registration unambiguously.

A composed client keeps each aliased LiveSpec's live transport and each
program's Program Read transport independent. Chain reads and transaction
submission are separate transports as well; do not infer one endpoint from
another.

`a4 sdk create` writes local source only. Publishing generated packages to npm
or crates.io is an explicit operator action. Likewise, deployment produces
endpoint bindings, not a required DNS provider: operators hand those endpoints
to their chosen DNS/CDN provider and manage records and certificates there.

## Repository Structure

- `arete/`: Main umbrella crate
- `interpreter/`: AST transformation runtime and VM
- `arete-macros/`: Proc-macros for stream definitions
- `arete-idl/`: IDL parsing and type system
- `rust/arete-server/`: WebSocket server and projection handlers
- `rust/arete-a4-sdk/`: Rust client SDK
- `cli/`: CLI tool for SDK generation
- `typescript/core/`: Pure TypeScript SDK
- `typescript/react/`: React SDK with hooks
- `typescript/adapters/`: Wallet adapters for `@solana/kit` and `@solana/web3.js`
- `python/arete-sdk/`: Python client SDK
- `stacks/`: Stack implementations and local SDK generation config
- `packages/`: Additional packages
- `examples/`: Example projects

## Releasing

This repo uses [release-please](https://github.com/googleapis/release-please) for automated releases.

### How it works

1. Make commits using [conventional commit](https://www.conventionalcommits.org/) format:
   - `feat: add new feature` - triggers minor version bump
   - `fix: resolve bug` - triggers patch version bump
   - `feat!: breaking change` - triggers major version bump
   - `chore:`, `docs:`, `refactor:` - no version bump

2. Push to `main` - release-please automatically creates/updates a Release PR

3. Merge the Release PR - this:
   - Updates `CHANGELOG.md` in affected packages
   - Bumps versions in `Cargo.toml`, `package.json`, `pyproject.toml`
   - Creates a GitHub Release with a unified version tag
   - Triggers publish workflows to crates.io, npm, and PyPI

### Configuration

| File | Purpose |
|------|---------|
| `release-please-config.json` | Package definitions and release settings |
| `.release-please-manifest.json` | Tracks current version of each package |

The first Python release also requires a PyPI pending trusted publisher for
project `arete-sdk`, owner/repository `AreteA4/arete`, workflow
`release-please.yml`, and environment `oss-pypi-publication`. The release job
uses GitHub OIDC and does not accept a long-lived PyPI token.

### Synchronized Versions

All core packages (Rust, TypeScript, and Python) are kept at the same version number using the `linked-versions` plugin. When any package receives a version bump, all packages are updated to the highest version in the group. This ensures compatibility when using packages individually.

> **Note:** `arete-idl` is currently versioned independently.

### Tag format

Tags follow the pattern `v{version}` (e.g., `v0.5.10`). Since all packages are version-synchronized, a single tag represents all packages in the release.

## Development

### Prerequisites

- **Rust**: 1.70+ (install via [rustup](https://rustup.rs/))
- **Node.js**: 16+ (for TypeScript SDK)
- **Python**: 3.9+ (for Python SDK)

### Building from Source

```bash
# Clone the repository
git clone https://github.com/AreteA4/arete.git
cd arete

# Build all Rust packages
cargo build --workspace

# Build TypeScript SDKs
cd typescript/core && npm install && npm run build
cd ../react && npm install && npm run build

# Install Python SDK in development mode
cd python/arete-sdk && pip install -e .
```

### Running Tests

```bash
# Rust tests
cargo test --workspace

# Rust linting
cargo clippy --workspace -- -D warnings

# TypeScript tests
cd typescript/core && npm test
cd ../react && npm test

# Python tests
cd python/arete-sdk && pytest
```

### Project Structure

```
arete/
├── arete/          # Rust umbrella crate
├── interpreter/         # AST transformation runtime and VM
├── arete-macros/   # Proc-macros for stream definitions
├── arete-idl/      # IDL parsing and type system
├── cli/                 # CLI tool (a4-cli)
├── rust/
│   ├── arete-sdk/      # Rust client SDK
│   └── arete-server/   # WebSocket server
├── typescript/
│   ├── core/            # Pure TypeScript SDK (@usearete/sdk)
│   ├── react/           # React SDK (@usearete/react)
│   └── adapters/        # Solana wallet adapters
├── python/arete-sdk/   # Python client SDK
├── stacks/              # Stack implementations and SDKs
├── packages/            # Additional packages
├── examples/            # Example projects
└── docs/                # Documentation (MDX)
```

## Documentation

- [Concepts Overview](docs/concepts/overview.mdx) - Architecture and core concepts
- [Stack API](docs/concepts/stack-api.mdx) - Client-side API reference
- [CLI Commands](docs/cli/commands.mdx) - CLI usage guide
- [React SDK](docs/src/content/docs/sdks/react.mdx) - Getting started with React

## Contributing

We welcome contributions! Here's how to get started:

### Development Workflow

1. Fork the repository
2. Create a feature branch (`git checkout -b feat/my-feature`)
3. Make your changes
4. Run tests (`cargo test --workspace`)
5. Commit using [conventional commits](https://www.conventionalcommits.org/) format
6. Open a pull request

### Commit Message Format

We use conventional commits for automated releases:

| Prefix | Purpose | Version Bump |
|--------|---------|--------------|
| `feat:` | New feature | Minor |
| `fix:` | Bug fix | Patch |
| `feat!:` or `fix!:` | Breaking change | Major |
| `docs:` | Documentation only | None |
| `chore:` | Maintenance | None |
| `refactor:` | Code refactoring | None |

### Code Style

- **Rust**: Follow `rustfmt` defaults, pass `clippy` with no warnings
- **TypeScript**: Follow ESLint configuration in `typescript/`
- **Python**: Follow PEP 8

## License

This project uses a dual license approach:

- **Rust infrastructure** (arete, interpreter, arete-macros, server, cli): [Apache-2.0](arete/LICENSE)
- **Client SDKs** (TypeScript, Python, Rust SDK): [MIT](typescript/react/LICENSE)
