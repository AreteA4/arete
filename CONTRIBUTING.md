# Contributing to Arete

Arete is building the real-time data layer for Solana applications.

**Why contribute?**

- Work on something that genuinely helps builders ship faster on Solana
- Your PRs ship to production — no contribution graveyard here
- Help keep Solana tooling current as the ecosystem evolves

We welcome contributions of all kinds: code, documentation, bug reports, and ideas.

## Getting Help

Stuck? Have questions? Here's how to reach us:

- **GitHub Discussions**: For longer-form questions and RFC-style proposals
- **Twitter/X**: [@areteHQ](https://twitter.com/areteHQ) — announcements and updates

Don't be shy. We were all beginners once.

## Your First Contribution

New to Arete? Start here:

1. **Find an issue**: Look for issues labeled [`good first issue`](https://github.com/AreteA4/arete/labels/good%20first%20issue) or [`help wanted`](https://github.com/AreteA4/arete/labels/help%20wanted)
2. **Claim it**: Comment on the issue to let us know you're working on it
4. **Submit your PR**: We'll review it and work with you to get it merged

**New to open source entirely?** Check out [How to Contribute to Open Source](https://opensource.guide/how-to-contribute/) — it's a great primer.

### What to Expect

- **Issue response**: We aim to respond within 48 hours
- **PR review**: Within 1 week for small changes, longer for substantial work
- **Merge timeline**: After approval, typically within a few days

We're a small team, so please be patient. We appreciate every contribution and will never leave you hanging without communication.

## Ways to Contribute

Not sure where to start? Here are some ideas:

| Type | Examples | Good for |
|------|----------|----------|
| **Code** | Bug fixes, features, optimizations | Developers comfortable with Rust/TS/Python |
| **Documentation** | Tutorials, API docs, examples, typo fixes | Great first contribution |
| **Testing** | Write tests, report bugs, verify fixes | Learning the codebase |
| **Examples** | Build demo apps using Arete | Showcasing what's possible |
| **Community** | Answer questions, review PRs, share feedback | Experienced contributors |

Documentation contributions are especially valuable — they help everyone and don't require deep codebase knowledge.

## Code of Conduct

We are committed to providing a welcoming and harassment-free experience for everyone. We expect all participants to:

- Be respectful and inclusive
- Accept constructive criticism gracefully
- Focus on what's best for the community
- Show empathy toward others

Unacceptable behavior includes harassment, trolling, personal attacks, and publishing others' private information. If you experience or witness unacceptable behavior.

For the full text, see our [Code of Conduct](CODE_OF_CONDUCT.md).

## Architecture Overview

Understanding how the pieces fit together:

```
┌─────────────────┐     ┌──────────┐     ┌─────────────┐     ┌─────────────────┐
│ Declarative     │ ──▶ │ Compiler │ ──▶ │  Bytecode   │ ──▶ │   Interpreter   │
│ Spec (Rust)     │     │          │     │             │     │   (Runtime VM)  │
└─────────────────┘     └──────────┘     └─────────────┘     └────────┬────────┘
                                                                      │
                                                                      ▼
                                                             ┌─────────────────┐
                                                             │  Real-time Data │
                                                             │      Feeds      │
                                                             └────────┬────────┘
                                                                      │
                         ┌────────────────────────────────────────────┼────────────────────────────────────────────┐
                         │                                            │                                            │
                         ▼                                            ▼                                            ▼
                ┌─────────────────┐                          ┌─────────────────┐                          ┌─────────────────┐
                │  TypeScript SDK │                          │    Rust SDK     │                          │   Python SDK    │
                │  (Core + React) │                          │                 │                          │                 │
                └─────────────────┘                          └─────────────────┘                          └─────────────────┘
```

**Key components:**

| Component | What it does | Language |
|-----------|--------------|----------|
| `arete-macros/` | Proc-macros for defining data streams declaratively | Rust |
| `interpreter/` | Executes bytecode, manages subscriptions and transforms | Rust |
| `cli/` | Generates SDKs from compiled specs | Rust |
| `typescript/` | Client SDKs for browser/Node.js apps | TypeScript |
| `python/` | Client SDK for Python apps | Python |
| `rust/` | Rust SDK and server components | Rust |

## Prerequisites

### Required

- **Rust 1.70+** — Install via [rustup](https://rustup.rs/)
- **Node.js 18+** and npm — For TypeScript SDKs
- **Python 3.9+** — For Python SDK

### Optional (Helpful for Deeper Contributions)

Familiarity with these helps but isn't required for all contributions:

- [Solana Developer Docs](https://solana.com/docs) — Blockchain basics
- [Anchor Framework](https://www.anchor-lang.com/) — Common Solana development framework
- Geyser plugins — How Solana streams account updates

Don't know Solana? No problem. Documentation, SDK, and CLI contributions don't require blockchain knowledge.

## Development Setup

### Fork & Clone

```bash
# 1. Fork the repository on GitHub

# 2. Clone your fork
git clone https://github.com/YOUR-USERNAME/arete.git
cd arete

# 3. Add upstream remote
git remote add upstream https://github.com/AreteA4/arete.git

# 4. Keep your fork updated
git fetch upstream
git checkout main
git merge upstream/main
```

### Building from Source

```bash
# Build all Rust packages
cargo build --workspace

# Build TypeScript SDKs
cd typescript/core && npm install && npm run build
cd ../react && npm install && npm run build

# Install Python SDK in development mode
cd python/arete-sdk && pip install -e .
```

### Development Commands

#### Rust

| Action | Command |
|--------|---------|
| Build | `cargo build --workspace` |
| Test | `cargo test --workspace` |
| Lint | `cargo clippy --workspace -- -D warnings` |
| Format | `cargo fmt --all` |

#### TypeScript

Located in `typescript/core` and `typescript/react`:

| Action | Command |
|--------|---------|
| Install | `npm install` |
| Build | `npm run build` |
| Test | `npm test` |
| Lint | `npm run lint` |

#### Python

Located in `python/arete-sdk`:

| Action | Command |
|--------|---------|
| Install | `pip install -e .` |
| Test | `pytest` |
| Lint | `ruff check .` |

### Troubleshooting

<details>
<summary><strong>Rust build fails with missing dependencies</strong></summary>

Ensure you have the latest stable Rust:
```bash
rustup update stable
rustup default stable
```

On macOS, you may need Xcode command line tools:
```bash
xcode-select --install
```
</details>

<details>
<summary><strong>TypeScript tests fail</strong></summary>

Clear node_modules and reinstall:
```bash
rm -rf node_modules package-lock.json
npm install
```
</details>

<details>
<summary><strong>Python import errors</strong></summary>

Ensure you're using a virtual environment:
```bash
python -m venv .venv
source .venv/bin/activate  # or `.venv\Scripts\activate` on Windows
pip install -e .
```
</details>

## Code Style

### Rust

- Run `cargo fmt --all` before committing
- Ensure `cargo clippy --workspace -- -D warnings` passes with no warnings
- Follow standard Rust naming conventions (snake_case for functions, CamelCase for types)

### TypeScript

- Follow the ESLint configuration in `typescript/`
- Use Prettier for formatting (`npm run format`)
- Prefer explicit types over `any`

### Python

- Follow PEP 8 guidelines
- Use type hints for function signatures
- Run `ruff check .` for linting

## Conventional Commits

We use [Conventional Commits](https://www.conventionalcommits.org/) to automate releases via `release-please`.

### Format

```
<type>[optional scope]: <description>

[optional body]

[optional footer(s)]
```

### Types

| Prefix | Use for | Version Bump |
|--------|---------|--------------|
| `feat` | New features | Minor |
| `fix` | Bug fixes | Patch |
| `feat!` or `fix!` | Breaking changes | Major |
| `docs` | Documentation only | None |
| `chore` | Maintenance, deps | None |
| `refactor` | Code restructuring | None |
| `test` | Adding/fixing tests | None |
| `perf` | Performance improvements | Patch |

### Examples

```bash
feat: add support for custom projection handlers
fix: resolve race condition in websocket reconnection
feat!: change stream subscription API (breaking)
docs: add tutorial for React integration
chore: update dependencies
test: add integration tests for Python SDK
```

## Pull Request Process

### Before You Start

1. **Check existing issues/PRs** — Someone might already be working on it
2. **Open an issue first** for substantial changes — Let's discuss before you invest time
3. **Small PRs are better** — Easier to review, faster to merge

### Creating Your PR

1. **Create a feature branch** from `main`:
   ```bash
   git checkout -b feat/your-feature
   # or: fix/bug-description, docs/what-you-documented
   ```

2. **Make your changes** with clear, atomic commits

3. **Ensure quality**:
   ```bash
   # Rust
   cargo fmt --all && cargo clippy --workspace -- -D warnings && cargo test --workspace

   # TypeScript
   npm run lint && npm test

   # Python
   ruff check . && pytest
   ```

4. **Push and open PR**:
   ```bash
   git push origin feat/your-feature
   ```

5. **Fill out the PR template** with:
   - What the PR does
   - Why it's needed
   - Related issue number (e.g., "Fixes #123")
   - Any breaking changes

### Review Process

- All CI checks must pass
- At least one maintainer approval required
- We may request changes — this is collaborative, not adversarial
- Once approved, a maintainer will merge

## Issue Guidelines

### Bug Reports

Please include:

- **Steps to reproduce** — Minimal example if possible
- **Expected vs actual behavior**
- **Environment** — OS, Rust/Node/Python version, Arete version
- **Error messages** — Full stack trace if available

### Feature Requests

Please include:

- **Problem statement** — What are you trying to solve?
- **Proposed solution** — How do you envision it working?
- **Alternatives considered** — What else did you think about?
- **Use case** — Real-world scenario where this helps

## Project Structure

```
arete/
├── arete/           # Main umbrella crate
├── interpreter/          # AST transformation runtime and VM
├── arete-macros/    # Proc-macros for stream definitions
├── rust/                 # Rust SDK and server components
├── cli/                  # CLI tool for SDK generation
├── typescript/
│   ├── core/            # Core TypeScript SDK
│   └── react/           # React hooks and components
├── python/
│   └── arete-sdk/  # Python client SDK
└── docs/                # Documentation source files
```

## Recognition

All contributors are recognized:

- Added to [CONTRIBUTORS.md](CONTRIBUTORS.md)
- Mentioned in release notes for significant contributions

## License

By contributing, you agree that your contributions will be licensed under:

- **Rust Infrastructure** (interpreter, macros, server): Apache-2.0
- **Client SDKs** (TypeScript, Python): MIT

See [LICENSE-APACHE](LICENSE-APACHE) and [LICENSE-MIT](LICENSE-MIT) for details.

---

**Thank you for contributing to Arete!**
