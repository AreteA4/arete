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

# Verify (ready when the JSON status is "ok")
a4 doctor --json

# Discover live data (no account needed)
a4 explore --json
a4 explore stack ore --json

# Need an account (deploying, knowledge layer)?
a4 auth signup                                  # restricted agent profile
a4 auth login --profile human --key <a4_sk_...> # separate human profile

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
| `a4 protocol deploy [dir]` | Upload a built protocol and deploy it privately |
| `a4 dev up` | Develop against your Arete cloud dev environment: serve it on 127.0.0.1, deploy the protocol, redeploy on change |
| `a4 dev status\|list\|checkpoint\|restore\|branch\|rebase\|reset\|fund\|pin\|follow\|warp\|…` | Manage the dev environment |
| `a4 dev step\|wallet\|instance\|replay\|sdk\|down` | Drive the protocol with local dev wallets; stop `a4 dev up` |
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
a4 auth signup [name]                       # Register an agent; --force replaces its profile key
a4 auth signup --json                       # Metadata only; never prints the secret
a4 auth login --profile human --key <a4_sk_...> # Save a separate human key
a4 auth logout
a4 auth status
a4 auth whoami                  # Verify with server
```

Credentials: named profiles in `~/.arete/credentials.toml`. `a4 init` writes
the non-secret `.arete/auth.toml` project default (`agent`) and pins generated
MCP config to `a4 --profile agent mcp`. Humans opt in with
`a4 --profile human <command>`.

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
for the agent running it and agents set up in the project (others with
`--agents <id>`). `a4 doctor` exits 0 for `ok`/`warn`, 1 for `fail`.

## MCP Server

```bash
a4 mcp
```

Stream MCP server over stdio (registry discovery, knowledge layer, live entity
reads). `a4 init` writes the config; the manual shape for Claude Code is
`{"mcpServers":{"arete":{"type":"stdio","command":"a4","args":["--profile","agent","mcp"]},"arete-docs":{"type":"http","url":"https://docs.arete.run/mcp"}}}`.

## Registry Exploration

Exploration uses the same deployment-pinned install descriptors as `a4
install`, so the reported StackManifest, LiveSpec, AST, and Program Release
identities are the ones an installation will consume.

```bash
a4 explore                              # List stacks
a4 explore programs                     # List complete installable programs
a4 explore stack ore --json             # Exact stack descriptor summary
a4 explore program spl-token --json     # Accounts, instructions, and release
a4 explore protocol token-raise --json  # Steps, access, rejection codes, events, hoisted packages
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

## Protocols

A protocol package is a typed client for an Arete Protocol: its steps
(prepare, sign, submit), instances, participants, metadata and events, plus
the stacks and programs it builds on.

```bash
a4 explore protocol token-raise --json   # Spec summary: machines, steps, access, rejection codes, events
a4 install protocol token-raise --ts     # Add [dependencies.protocols.token-raise] and generate the client
a4 install protocol token-raise --rust   # The same client for Rust (arete-a4-sdk)
a4 install protocol token-raise --python # The same client for Python (arete-sdk)
a4 update protocol token-raise
a4 remove protocol token-raise
```

Protocols are registry dependencies and generate TypeScript, Rust or Python clients:

```toml
[dependencies.protocols.token-raise]
source = { registry = "token-raise" }
version = "^1.0.0"
targets = ["typescript"]
```

`a4 install protocol` writes `<output_dir>/protocols/<alias>/`:

| Path | Contents |
|---|---|
| `<alias>.ts` | `<NAME>_PROTOCOL` (the default export) and its generated types: params, each step's args and guard-code union, machine states, metadata fields, event names, the webhook payload and `verify<Name>Webhook` |
| `protocol-spec.json` | the exact `arete.protocol-spec/v1` |
| `stacks/<stack>/` | each stack the protocol hoists, generated as `a4 install stack` generates it |
| `programs/<program>/` | each program the protocol declares that none of its stacks bundles (except unpinned programs, below) |
| `README.md`, `sdk-manifest.json`, `sdk-provenance.json` | reference and ownership |

Only steps an app can call (wallet steps) are in the client. Each records its
`access`: a `public` step can be prepared from a browser with a publishable
key; a `secret` step only with a secret key, and calling it from a
publishable-key client is a type error. A protocol's programs are hoisted the
way a stack hoists its programs: one program SDK per program, served by the
first stack that bundles that release; two different releases under one key
fail with `PROGRAM_KEY_CONFLICT`.

A private protocol package can leave a program **unpinned**: the protocol was
built against a program installed from a local IDL
(`a4 install program ./idl/x.json --as <alias>`), so no program package exists
to hoist. The install skips it: no SDK is generated for it and the client's
`programs` doesn't list it. The client's `README.md` lists each unpinned
program (key, program id, ProgramSpec hash), and `a4 install` prints a note
naming this project's own program when one with the same ProgramSpec hash is
installed, or how to install it otherwise. Preparing and submitting steps
never needs a program SDK; install the program yourself only if the app reads
or calls it directly, and pass it under the protocol's key to share it in a
session (`createSession({ programs: { <key>: <PROGRAM> }, protocols: { … } })`).

Installing a public protocol records the publisher's shared deployment (its
id and the hosted API URLs) and nothing else: no credential is written to
`arete.toml`, `arete.lock` or the generated code. The app's own key mints a
protocol session at runtime (`POST /ws/sessions` with
`targetKind: "protocol-deployment"`).

`--python` writes a Python package to
`<python output_dir>/protocols/<alias>-protocol/` (`<alias>_protocol/` holds
the module; in module mode the directory is the module itself). It defines
`<NAME>_PROTOCOL`, `TypedDict`s for params, step args and metadata,
`Literal`s for states, events and guard codes, and two clients over
`arete.protocol.ProtocolClient`: `<Name>Client` carries only `public` steps,
and `<Name>SecretClient` carries every client-callable step and refuses a
publishable key. Hoisted stacks and programs are generated under `stacks/`
and `programs/` as module-mode subpackages.

`--rust` writes a crate to `<rust output_dir>/protocols/<alias>-protocol/`
(package `<alias>-protocol`; in module mode the directory holds `mod.rs`). It
defines `<NAME>_PROTOCOL`, structs for params, step args and metadata
(wide integers as `ProtocolInteger` decimal strings), enums for states, events
and each step's guard codes, `verify_<name>_webhook`, and two clients over
`arete_sdk::protocol::ProtocolClient`: `<Name>PublishableClient` has a
`prepare_<step>` method for each `public` step only, and `<Name>SecretClient`
has one for every client-callable step and refuses a publishable key. A
prepare resolves with the intent and transactions, or a rejection typed by the
step's guard-code enum. Hoisted stacks and programs are generated under
`stacks/` and `programs/` as path-dependency crates (modules in module mode)
and re-exported as `stacks::<key>` and `programs::<key>`.

### Deploying your own protocol

`a4 protocol deploy [dir]` builds your protocol with `arete-protocol build`
(from `@usearete/protocol`) and takes it to a private deployment. It uploads
only `.arete/protocol/protocol-spec.json`, plus `handlers.mjs` when a step has
a handler; the server derives the stack source and release from them.

```bash
a4 protocol deploy                       # the protocol in the current directory
a4 protocol deploy ./my-protocol         # builds first, then deploys
a4 protocol deploy --no-build            # upload the existing build, if it is current
a4 protocol deploy --dry-run             # print the plan; changes nothing
a4 protocol deploy \
  --fee-policy fee-policy.json \
  --resource launchDefaults=launch-defaults.json
a4 protocol deploy --no-activate --json
```

The command:

1. Builds the project (`npx --no arete-protocol build`), unless you pass
   `--no-build`. With `--no-build` it uploads the existing `.arete/protocol/`
   output, and refuses it when a source changed since that build: the build
   records each source file's hash (`build-sources.json`, behind the receipt's
   `sourceTreeSha256`), and the error names the changed and deleted files.
2. Validates the spec locally and prints the release hash the server should
   derive. If the server returns a different hash, it warns and continues
   with the server's release.
3. Finds your protocol by slug or creates it. The slug defaults to the spec
   name in kebab case (`tokenRaise` becomes `token-raise`).
4. Uploads the release. Re-uploading an identical release is a no-op.
5. Reuses a private deployment of the same release if one exists. Otherwise
   it stores the fee policy, publishes a version of every resource the spec
   declares, and creates the deployment.
6. Activates the deployment, unless it is already active or you passed
   `--no-activate`.
7. When the deployment is active and the project installs its own protocol
   (`[dependencies.protocols]` with this slug, which `a4 dev up` adds), updates
   that client to the new deployment: `a4 update protocol <alias>` from the
   registry, or, while the registry has no package release for the deployment
   yet, the client is generated from the deployed build with the new
   deployment id and `arete.lock` marks it `local_build` (the command says so).
   Run `a4 install` once the package is published.

What happens at activation depends on the protocol:

- **Template-only protocols** (no handler code) activate as private
  deployments at trust tier `unreviewed`. While the server's template check is
  `pending`, activation waits until the server has the program specs the
  templates target.
- **Protocols with handler code** can't run until an operator reviews the
  release. The command creates the deployment, prints the release hash to
  share with the operator, and exits 0 with status `review-required`. The
  operator reviews the release and activates the deployment; owners never
  activate handler code themselves.
- **Public deployments** can only be created by an operator. Self-serve
  deployments are always private.

| Flag | Description |
|---|---|
| `--no-build` | Upload the existing `.arete/protocol/` output instead of building; refused when the sources changed since that build |
| `--dry-run` | Print the files, expected release hash, slug, and whether a deployment would be created or reused. Reads the server only when a secret key is configured |
| `--slug <slug>`, `--name <name>` | Override the protocol slug (`^[a-z0-9][a-z0-9-]{0,62}$`) and display name |
| `--fee-policy <file>` | Fee policy document for a new deployment |
| `--resource <name>=<file>` | JSON value for a declared resource (repeatable) |
| `--no-activate` | Create or reuse the deployment without activating it |
| `--wait-secs <n>` | Keep re-trying activation while the server is still preparing the deployment, for example deploying its stack (default 300; 0 activates once) |
| `--json` | Print the result as JSON: `status`, `protocol`, `release`, `deployment`, `client` (the installed client update, when there is one), `warnings` |

The command needs a secret API key; publishable keys are refused. A new
deployment needs a fee policy, which is a JSON document like this:

```json
{
  "schema": "arete.protocol-fee-policy/v1",
  "steps": {}
}
```

Arete fills in the platform fee destination, so `platformDestination` is
optional. A policy that names one is still accepted.

Flags override the optional `[protocol]` section of the project's
`arete.toml`. Paths in that section are relative to the project directory, and
paths passed as flags are relative to the current directory.

```toml
[protocol]
slug = "token-raise"
name = "Token Raise"
fee_policy = "fee-policy.json"
access = "private"            # the only accepted value

[protocol.resources]
launchDefaults = "resources/launch-defaults.json"
```

A `[protocol.resources]` entry is either a path to a JSON file (a string) or
the value itself as an inline TOML value, such as
`launchDefaults = { tokenDecimals = 6 }`.

### Cloud dev environments (`a4 dev`)

`a4 dev up` develops a project against an Arete cloud dev environment: a
persistent fork of mainnet that Arete hosts for your account. It creates or
wakes the project's environment (remembered in `.arete/dev/environment.json`),
serves it on `127.0.0.1` (JSON-RPC on the port base, WebSocket on +1, with
the environment token added, so wallets, the Solana CLI and web3 clients
need no token), runs the dev UI (+2), and, in a protocol project, rebuilds
with `arete-protocol build` and deploys each new release as a private
deployment bound to the environment, disabling the previous one once the new
one is active. Signing stays local. Ctrl-C stops the local endpoints; the
environment keeps its state and sleeps when idle.

```bash
a4 dev up                                             # create or wake, serve on 127.0.0.1, deploy and watch
a4 dev up --env staging --no-deploy                   # another environment, no protocol deploy
a4 dev up --fee-policy fee-policy.json --resource limits=limits.json  # as for `a4 protocol deploy`
a4 dev status --json                                  # the environment, the session, the chain, a stack stream URL
a4 dev list                                           # your environments and quotas
a4 dev fund default 50                                # SOL for a dev wallet (or any address)
a4 dev fund <owner> --mint <mint> --amount 1000000    # a token balance
a4 dev warp +2h                                       # the clock (also slot:<n>, epoch:<n>)
a4 dev checkpoint funded && a4 dev restore funded     # save and restore the whole state
a4 dev branch funded experiment --use                 # a new environment from a checkpoint
a4 dev rebase                                         # refetch every account you never wrote
a4 dev pin <pubkey> / a4 dev follow <pubkey>          # keep an account as it is, or live
a4 dev scenario crash.json                            # arete.scenario/v1 overrides with expectations
a4 dev preview <base64 tx>                            # simulate: compute units, accounts before and after
a4 dev repro <signature> -o repro.json                # the minimal state to reproduce a transaction
a4 dev token                                          # an environment token and its RPC URLs
a4 stream --dev                                       # the environment's protocols, counts and views
a4 dev wallet recipient                               # a dev wallet's address (created and funded)
a4 dev step open --args '{"recipient":"<address>","amount":"1000"}' --wallet creator
a4 dev step close --instance <id> --wallet recipient
a4 dev instance <id>
a4 dev replay .arete/dev/history.jsonl                # run the recorded steps again (skips failed ones)
a4 dev sdk --ts --rust                                # regenerate the installed client from the local build now
a4 dev down [--sleep] [--purge]
a4 dev delete --yes
```

Every environment command takes `--env <name|id>` (default `ARETE_DEV_ENV`,
else the project's) and `--json`, and works without `a4 dev up` running.
`.arete/dev/session.json` (written while `a4 dev up` runs) names the
environment, the local RPC and WebSocket URLs, the environment's own RPC
URL, the deployment and the dev UI; it holds no key and no token.

Every `a4 dev step` is recorded in `.arete/dev/history.jsonl`. `a4 dev
replay` skips entries recorded as rejected or failed unless
`--include-failed`, and logs its own runs to `.arete/dev/replays.jsonl`.
After every deploy, `a4 dev up` generates the project's own protocol client
from the local build where `a4 install protocol` puts it
(`generated/<lang>/protocols/<alias>/`), calling the deployment in the dev
environment, adding the `[dependencies.protocols.<slug>]` entry before the
first publish, and marks it `local_build` in `arete.lock`. `a4 install
--locked` restores the published client and `a4 install --check` fails while
a local build is installed. Releases with handler code run after an Arete
review, in a dev environment too; `a4 dev up` activates the waiting
deployment once it is reviewed. The full guide is in
`docs/src/content/docs/cli/local-development.mdx`.

### Lockfile

A protocol entry pins its package release, the protocol release and spec, the
deployment it calls, each hoisted stack (with its LiveSpecs and programs) and
its own programs:

```toml
[[dependency]]
kind = "protocol"
alias = "token-raise"
source = "registry:token-raise"
requirement = "^1.0.0"
version = "1.0.0"
package_release_hash = "arete:registry-package-release:v2:sha256:…"
targets = ["typescript"]
generator_contract = "sdk-generator-v1"

[[dependency.programs]]          # programs no stack bundles
program_id = "…"
program_spec_hash = "arete:h1:program-spec:sha256:…"
program_release_hash = "arete:h1:program-release:sha256:…"
package_release_hash = "arete:registry-package-release:v2:sha256:…"

[dependency.protocol]
protocol_release_hash = "arete:h1:protocol-release:sha256:…"
protocol_spec_hash = "arete:h1:protocol-spec:sha256:…"
deployment_id = "…"
deployment_access = "public"

[[dependency.stacks]]            # one per stack the spec declares
alias = "meteoraPresale"
package = "meteora-presale"
version = "1.1.0"
package_release_hash = "arete:registry-package-release:v2:sha256:…"
stack_manifest_hash = "arete:h1:stack-manifest:sha256:…"

[[dependency.stacks.live_specs]]
alias = "live"
artifact_hash = "arete:h1:live-spec:sha256:…"

[[dependency.stacks.programs]]
program_id = "…"
program_spec_hash = "arete:h1:program-spec:sha256:…"
program_release_hash = "arete:h1:program-release:sha256:…"
package_release_hash = "arete:registry-package-release:v2:sha256:…"
```

Stack and program entries are unchanged by protocol support.

### Registry contract

`POST /api/registry/v1/resolve` accepts `kind: "protocol"` dependencies and
answers each with a protocol resolution that mirrors a stack resolution: the
package release, plus the protocol, plus nested `stacks` (each a complete
`kind: "stack"` resolution, aliased by the spec's stack key, carrying its own
`programs`, `liveSpecs` and `delivery`) and `programs` (exact program
releases, like a stack's `programs`, each with its `programPackage`).
`GET /api/registry/protocols/{ref}/install` returns the same object for one
package, which `a4 explore protocol` summarises.

```json
{
  "kind": "protocol",
  "alias": "token-raise",
  "package": "token-raise",
  "version": "1.0.0",
  "packageReleaseHash": "arete:registry-package-release:v2:sha256:…",
  "generatorContract": "sdk-generator-v1",
  "protocol": {
    "protocolReleaseHash": "arete:h1:protocol-release:sha256:…",
    "protocolSpecHash": "arete:h1:protocol-spec:sha256:…",
    "protocolSpec": { "schema": "arete.protocol-spec/v1", "…": "…" },
    "deployment": {
      "deploymentId": "…",
      "access": "public",
      "apiUrl": "https://api.arete.run/api",
      "sessionEndpoint": "https://api.arete.run/ws/sessions"
    },
    "liveStack": { "stack": "tokenRaiseStack", "view": "TokenRaiseInstance/state", "keyAccount": "presale" }
  },
  "stacks": [{ "kind": "stack", "alias": "meteoraPresale", "…": "a stack resolution" }],
  "programs": [{ "installName": "squads_v4", "programPackage": { "…": "…" }, "…": "a program release" }]
}
```

- `protocolSpec` may be omitted; the CLI then reads
  `GET /api/registry/artifacts/protocol-spec/{protocolSpecHash}`. Either way
  the document must hash to `protocolSpecHash`.
- `stacks` must be exactly the spec's `stacks`, each at the StackManifest the
  spec pins.
- every `programSpec` program the spec declares must be bundled by one of the
  stacks, listed in `programs` (matched by program id and ProgramSpec hash),
  or listed in `unpinnedPrograms`; `programs` lists nothing else.
- `unpinnedPrograms` is optional (private packages only): programs with no
  program package, each `{ "key", "programId", "programSpecHash" }` exactly as
  the spec pins it. Nothing is generated for them, and a program can't be both
  unpinned and resolved.
- `deployment.access` is `public` (the publisher's shared deployment, called
  with the integrator's own key) or `private` (the owner's own).
- `liveStack` is optional: the hoisted stack whose view row carries an
  instance's live fields, keyed by `instance.accounts.<keyAccount>`.
- A resolution never carries a credential.

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
| `Not authenticated` | Run `a4 auth signup` or `a4 auth login --profile human --key <a4_sk_...>` |
| `a4: command not found` | Run `export PATH="$HOME/.local/bin:$PATH"` or use the `A4_BIN=` path the installer printed |
| `a4 was not installed by the Arete installer` | Reinstall with `curl -fsSL https://arete.run/install.sh \| sh` |
| `Stack not found` | Check `a4 stack list` |
| `StackManifest not found` | Run `cargo build` and use the generated manifest path |
| `Build failed` | Check `a4 status` for build details |

## License

Apache-2.0
