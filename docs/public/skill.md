# Arete CLI Guide for Agents

Arete is operated through the `a4` CLI. Use it for setup, authentication,
catalog discovery, exact descriptors, project dependencies, SDK generation,
live-data inspection, and hosted lifecycle commands. Do not recreate its HTTP
requests from documentation.

For a new project, start with the complete bootstrap page:

```text
https://docs.arete.run/agent.md
```

## Set up and verify

```bash
curl -fsSL https://arete.run/install.sh | sh
a4 init -y
a4 doctor --json
```

`a4 init` configures the project, installs the Arete skills, and registers the
Arete stream and documentation MCP servers for detected coding agents. If the
current agent process does not see new skills or tools, ask the user to reload
or restart the host, then run `a4 doctor --json` again.

Follow the exact `fix` reported for a required doctor failure. Use current
`a4 <command> --help` output when a command surface differs from this guide.

## Work from intent

Search the live catalog instead of guessing a program, stack, or endpoint:

```bash
a4 explore catalog --query "<user intent>" --json
a4 explore catalog --vocabulary --json
```

Useful filters are:

- `--kind program|stack`
- `--mode read|build|subscribe`
- `--target typescript|rust|python`
- `--concept <slug>` and `--category <slug>` from the vocabulary

Catalog results report which delivery modes are currently available. A known
program does not imply that every read, build, or subscription surface is ready.

## Inspect the exact capability

Before installing or writing code, inspect the selected descriptor:

```bash
a4 explore catalog program <slug> --json
a4 explore catalog stack <slug> --json
```

The descriptor supplies exact identities, bindings, authentication, SDK
targets, and the install command. Treat it as the contract. If a project already
contains a direct reference, these compatibility forms inspect the same kind of
contract:

```bash
a4 explore program <program-ref> --json
a4 explore stack <stack-ref> --json
```

For curated protocol meaning and generated operation names, use the knowledge
commands when available:

```bash
a4 know search --query "<intent>" --json
a4 know program <program-slug> --section surface --json
```

Use knowledge to understand why a capability fits. Return to the exact
descriptor before installation.

## Explore live data

`a4 init` configures the Arete MCP server for agent-led exploration. Use the
tools exposed by the current server to inspect schemas and read views
(`read_view` reads a view's current entities in one call), or connect,
subscribe, query a bounded sample, and disconnect.

To answer "what is the current X", read the view once instead of writing a
script:

```bash
a4 get <Entity>/<view> --stack <stack-ref> --limit 1
a4 get <Entity>/<view> --stack <stack-ref> --select <field>,<field> --limit 10
a4 get <Entity>/state --stack <stack-ref> --key <key>
```

To watch it change:

```bash
a4 stream <Entity>/<view> --stack <stack-ref> --take 10 --duration 15
```

Before reporting token amounts, check each field's `amount` in
`a4 explore stack <stack-ref> --views <Entity>/<view>`: `scale: "ui"` values
are whole tokens, `scale: "raw"` values are base units (divide by
`10^decimals`; for SOL these are lamports).

Use MCP, `a4 get` or `a4 stream` for investigation. Use a generated SDK for application
code.

## Install an exact dependency

Prefer the install command reported by the descriptor:

```bash
a4 install program <slug> --ts
a4 install stack <slug> --ts
a4 install --locked
```

A stack includes the program SDKs for the programs its views index, at
`arete.programs.<name>`, and they are the same SDKs a standalone program install
gives. Install a stack when the app needs live views, with or without
transactions; install a program on its own only when no stack you use covers
it. Never merge stack and program objects by hand.

Use `--rust` or `--python` only when the descriptor lists that target. A saved
install updates:

- `arete.toml`, which records dependency intent;
- `arete.lock`, which pins exact resolution; and
- generated SDK output, which should be regenerated rather than edited.

Inspect generated exports and types before writing application code. They are
the authority for names, parameters, and return shapes.

## Authenticate during setup

The canonical agent bootstrap creates or verifies a restricted account before
hosted discovery. Use the CLI and keep the generated credential in its named
profile:

```bash
a4 --profile agent auth signup --if-missing --json
a4 --profile agent auth status
a4 --profile agent auth whoami --json
```

Let the CLI store and resolve credentials. Do not implement key or session
management against platform endpoints, and do not request a human `a4_sk_*`
credential for agent work.

The CLI and the Arete MCP server load stored credentials for you. Do not search
for, read, print, or copy credential files or keys into code, prompts, tool
arguments, or transcripts; use `auth status` and `auth whoami` to inspect
authentication.

SDK code for servers, agents, and local scripts authenticates with an agent or
secret key: pass it as `secretKey` (TypeScript) or `secret_key` (Python, Rust),
or set no auth option and provide `ARETE_API_KEY` in the environment. Read the
key from the environment, never from source. Anything shipped to a browser
needs an origin-bound publishable key instead (`publishableKey` /
`publishable_key`), created per origin with
`a4 auth keys create-publishable --origin <scheme://host[:port]>`. The
TypeScript SDK refuses `secretKey` in a browser.

## Use the installed task skill

`a4 init` installs five focused skills:

| Skill                   | Use it for                                           |
| ----------------------- | ---------------------------------------------------- |
| `arete`                 | Discovery, descriptors, and project dependencies     |
| `arete-streams`         | Deployed views and subscriptions                     |
| `arete-programs`        | Account reads, PDAs, operations, and transactions    |
| `arete-stack-authoring` | Custom read models and portable artifacts            |
| `arete-deploy`          | Explicitly requested publication and deployment work |

The task skill provides workflow detail. The current descriptor, generated
types, and CLI output provide the changing technical surface.

## Source-of-truth order

When information differs, use:

1. generated types and the exact installed descriptor;
2. current `a4 <command> --help` and `--json` output;
3. current Arete documentation; and
4. general examples or skill guidance.

The normal loop is:

```text
doctor → discover → inspect → explore → install → build with generated types
```
