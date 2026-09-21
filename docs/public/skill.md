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
tools exposed by the current server to inspect schemas, connect, subscribe,
query a bounded sample, and disconnect.

The CLI can inspect a deployed view directly:

```bash
a4 stream <Entity>/<view> --stack <stack-ref> --first
a4 stream <Entity>/<view> --stack <stack-ref> --take 10 --duration 15
```

Use MCP or `a4 stream` for investigation. Use a generated SDK for application
code.

## Install an exact dependency

Prefer the install command reported by the descriptor:

```bash
a4 install program <slug> --ts
a4 install stack <slug> --ts
a4 install --locked
```

Use `--rust` or `--python` only when the descriptor lists that target. A saved
install updates:

- `arete.toml`, which records dependency intent;
- `arete.lock`, which pins exact resolution; and
- generated SDK output, which should be regenerated rather than edited.

Inspect generated exports and types before writing application code. They are
the authority for names, parameters, and return shapes.

## Authenticate when requested

Public catalog discovery does not require an account. When a hosted capability
reports that authentication is required, use the CLI:

```bash
a4 auth status
a4 auth signup
a4 auth login --key <a4_ak_...>
```

Let the CLI store and resolve credentials. Do not implement key or session
management against platform endpoints.

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
