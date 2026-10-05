---
name: arete
description: Discover and install exact Arete stacks or program SDKs for a Solana application. Use for generic Arete setup, capability discovery, choosing between a stack, a standalone program SDK, or a composed stack, or managing arete.toml dependencies. For view code use arete-streams; for program operations use arete-programs; for Rust stack definitions or composing stacks use arete-stack-authoring; for hosted publication or deployment use arete-deploy.
metadata:
  version: "1.3.1"
  min-cli: ">=0.25.1"
---

# Discover and Install Arete Capabilities

Use this skill to turn an application intent into an exact, installed Arete dependency. Do not guess protocols, stack names, program surfaces, view names, or generated APIs.

## Platform Model

Arete has two building blocks:

- **Live views**: entities and views that a hosted runtime maintains. Query them at a point in time or subscribe to their updates.
- **Program SDKs**: one program's typed accounts, reads, PDAs, raw instruction builders, and semantic `instructions`, `transactions`, and `flows`.

A **stack** is a named group of live views and program SDKs. A default stack includes the program SDKs for the programs its views index. A composed stack groups views and program SDKs that you choose.

Catalog results describe coverage in three modes:

- `subscribe`: typed point-in-time and live views from a hosted stack.
- `read`: typed program-account and chain reads.
- `build`: typed program instructions, transactions, and flows.

An exact stack descriptor binds selected views, programs, authentication policies, endpoints, and SDK targets. A program descriptor binds a normalized IDL, ProgramSpec, Program Release, Program Read service, and supported SDK targets.

## Health Gate

When environment setup is relevant, run:

```bash
a4 doctor --json
```

Follow the reported `fix` commands when a required check is not ready. If `a4` is missing and setup is within scope, use the current bootstrap instructions at `https://docs.arete.run/agent.md`; do not substitute a Cargo installation from memory.

Do not run setup on every Arete task. A healthy project, a usable generated dependency, or a task that only asks for explanation does not need repeated onboarding.

## Discovery Workflow

Start from the user's intent, not from a remembered public stack:

```bash
a4 explore catalog --query "<intent>" --json
a4 explore catalog --vocabulary --json
```

Use `--concept` or `--category` only with slugs returned by the vocabulary.
Narrow by `--kind program|stack`, `--mode read|build|subscribe`, and
`--target typescript|rust|python` when the request already determines those
constraints.

Read each result's coverage modes, then route by what the application needs:

- **Live views, with or without operations**: a stack. Its program SDKs come with it (see [Stacks include their program SDKs](#stacks-include-their-program-sdks)). Use `arete-streams` for view code and `arete-programs` for the stack's operations.
- **Reads and operations only**: a program SDK. Use `arete-programs`.
- **Views and program SDKs that no stack groups**: compose a stack from them (see [Compose a stack](#compose-a-stack)), or install each one and hold them in one session.
- A stack result without `subscribe` coverage is definition-only. It installs as a typed SDK and a pinned StackManifest, but nothing is hosted. The user can deploy their own copy after installing it (see `arete-deploy`); do not present it as a live stream until then.
- No suitable hosted capability and the user wants a custom feed: specify the missing read model, then use `arete-stack-authoring`.
- Publication or hosted lifecycle work: use `arete-deploy`.

Inspect the selected catalog result as an exact install descriptor:

```bash
a4 explore catalog stack <stack-slug> --json
a4 explore catalog program <program-slug> --json
```

If the user or project already supplies a direct reference, these compatibility
forms resolve the same descriptor contract:

```bash
a4 explore stack <stack-ref> --json
a4 explore program <program-ref> --json
```

Keep exploration output small when you only need part of a descriptor:

```bash
a4 explore stack <stack-ref> --summary --json
a4 explore stack <stack-ref> --views <Entity>/<view> --json
a4 explore stack <stack-ref> --operation <operation> --json
a4 explore program <program-ref> --operation <operation> --json
a4 explore program <program-ref> --section instructions --json
```

`--summary` lists a stack's entities with their view ids, its program SDKs, endpoints, and auth requirements. `--operation` takes a semantic path such as `transactions.<group>.<name>`, an operation id, or a raw instruction name. On a stack it searches the stack's program SDKs. Semantic paths need an API key; without one, only raw instruction names resolve.

Catalog contents and capability delivery can change. Do not maintain a static
list of public programs or stacks, infer an endpoint, or treat a search result
as proof that an unreported mode is available.

For reviewed protocol context and callable operation semantics, use the
knowledge layer when available:

```bash
a4 know search --query "<intent>" --json
a4 know program <program-slug> --section surface --json
a4 know program <program-slug> --section instructions --json
a4 know program <program-slug> --section accounts --json
```

Return to the exact catalog descriptor before installation. Treat a descriptor
refusal as “not currently installable.” Do not fall back to an arbitrary latest
IDL, AST, deployment, or release.

## Install the Exact Dependency

Prefer the `installCommand` returned by the exact descriptor. In a project, a
saved dependency updates `arete.toml`, resolves `arete.lock`, and generates
provenance-owned output:

```bash
a4 install stack <stack-ref> --ts
a4 install program <program-ref> --ts
```

Choose `--ts`, `--rust`, or `--python` from the existing project language and the descriptor's `sdkTargets`. Standalone program packaging may support fewer targets than a program bundled in a stack; trust the descriptor and command output.

Use `--no-save` only for a genuinely disposable, one-package generation. Do not hand-edit generated SDKs.

A definition-only stack's generated SDK has empty endpoints until the project records a deployment of it. Deploying it with `a4 up <alias>` (an external mutation; see `arete-deploy`) records the deployment in `arete.toml` and regenerates the SDK.

For project dependency configuration, locked installs, updates, removals, and output ownership, read [references/project-dependencies.md](references/project-dependencies.md).

### Stacks include their program SDKs

A default stack includes the program SDKs for the programs its views index. Installing the stack gives you each one at `arete.programs.<name>` (`session.programs.<name>` in a session), using the stack's auth and transaction transport. These are the same SDKs a standalone `a4 install program` produces, from the same program package release, so you do not need a separate program install to get a stack's operations. Take `<name>` from the generated types.

- Install a program on its own when you only need reads and transactions.
- If no stack groups the views and program SDKs you need, compose one.
- Installing a stack and the same program standalone is still valid. When both come from the same program package release, the install output lists the program under `Shared`, and at runtime they are one program.
- Never merge stack and program objects by hand, for example by spreading them into one object. Pass stacks and programs to one session, or use the stack's own `programs`.

To confirm that a stack provides an operation before adding another package for it, run `a4 explore stack <stack-ref> --operation <operation> --json`.

Report the requirements before writing code that sends transactions. The install output lists the stack's auth requirements, and `auth` in `a4 explore stack <stack-ref> --summary --json` gives the same. Browser apps need a publishable key bound to the app's origin. When transactions need an account entitlement, `a4 doctor --json` reports the account's `account.transactions` readiness.

### Compose a stack

When no stack groups the live views and program SDKs you need, compose one from published parts:

```bash
a4 stack compose --name <name> --live <stack-ref> --program <program-ref> --install --ts
```

This writes `[authoring.stacks.<name>]` in `arete.toml`, declares the stack as a dependency with the SDK target you pass (`--ts`, `--rust`, or `--python`, as for `a4 install`), and installs it. The programs the views index come with them, as their stack's program SDKs. A composed stack does not carry a source stack's stack extension, such as its `read` functions. For view selection, local files, deployment, and limits, use `arete-stack-authoring`.

## Source-of-Truth Order

When sources differ, use this precedence:

1. Generated types and the installed descriptor identity.
2. Current `a4 <command> --help` and `--json` output.
3. Current Arete documentation.
4. These workflow instructions.

Never preserve a stale example merely because it appears in an existing application. Identify the mismatch and migrate it to the installed surface.
