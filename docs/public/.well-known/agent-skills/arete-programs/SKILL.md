---
name: arete-programs
description: Use generated Arete program SDKs, standalone or included in a stack, to read accounts, derive PDAs, build instructions, prepare semantic operations, inspect transactions, or execute them through a wallet. Use for Solana protocol integrations and transaction workflows. Do not use for stack views or subscriptions; use arete-streams. Do not publish a program or deploy a stack; use arete-deploy.
metadata:
  version: "1.2.1"
  min-cli: ">=0.25.0"
---

# Build with Arete Program SDKs

Use the exact generated program surface. Do not infer instruction arguments, account roles, PDA seeds, semantic operation names, or signer requirements from a raw IDL or remembered protocol API.

## Resolve the Program Surface

Start from intent when the protocol or operation is not already pinned:

```bash
a4 explore catalog --query "<intent>" --kind program --json
a4 explore catalog program <program-slug> --json
```

The exact catalog descriptor determines program identity, available `read` and
`build` modes, SDK targets, Program Read delivery, authentication, and the
install command. Do not infer a missing delivery mode from catalog knowledge.

Inspect reviewed operation, instruction, and account semantics as needed:

```bash
a4 know program <program-slug> --section surface --json
a4 know program <program-slug> --section instructions --json
a4 know program <program-slug> --section accounts --json
```

For one operation's input, required and derived accounts, signers, and errors, ask for only that operation:

```bash
a4 explore program <program-ref> --operation <operation> --json
a4 explore stack <stack-ref> --operation <operation> --json
```

`<operation>` is a semantic path such as `transactions.<group>.<name>`, an operation id, or a raw instruction name. On a stack, the command searches the stack's program SDKs.

If the project already pins a direct program or stack reference, inspect it
through the compatibility descriptor form:

```bash
a4 explore program <program-ref> --json
a4 explore stack <stack-ref> --json
```

Use `surface` for semantic operation names and generated bindings. Use the
descriptor for exact account and instruction shapes, identities, SDK targets,
Program Read availability, authentication, and installation. A missing curated
recipe means “not documented,” not automatically “unsupported.”

## Stacks Include Their Program SDKs

A stack groups live views and program SDKs. A default stack includes the program SDKs for the programs its views index. Installing the stack gives you each one at `arete.programs.<name>` (`session.programs.<name>` in a session), using the stack's auth and transaction transport. These are the same SDKs a standalone `a4 install program` produces, from the same program package release, so you do not need a separate program install to get a stack's operations.

- The application uses a stack, or needs live views too: install the stack and use its program SDKs.
- Only reads and transactions: install the program on its own.
- No stack groups the views and program SDKs you need: compose one (see `arete`).
- Never merge stack and program objects by hand. Pass stacks and programs to one session, or use the stack's own `programs`.

A project may install a stack and the same program standalone. When both come from the same program package release, they are one program at runtime.

## Install and Inspect Generated Code

Prefer the descriptor's `installCommand`. A standalone program currently may support fewer language targets than the same program bundled in a stack. Trust `sdkTargets` rather than assuming parity.

```bash
a4 install program <program-ref> --ts
a4 install stack <stack-ref> --ts
```

Inspect the generated module before coding. Program clients normally expose these layers:

| Layer | Purpose |
| --- | --- |
| `programId`, `schemas` | Exact identity and generated data shapes |
| `pdas`, `addresses` | Typed deterministic address derivation |
| `accounts`, `queries` | Typed Program Read access |
| `raw.<instruction>.build` | Exact IDL-shaped instruction construction |
| `instructions.<name>.prepare` | Semantic single-instruction operation |
| `transactions.<path>.prepare` | Semantic multi-instruction transaction |
| `flows.<path>.prepare` | Semantic multi-transaction flow |
| `constants`, `defaults`, `math` | Generated or extension-provided domain helpers |

Not every program defines every semantic namespace. Do not recreate a missing semantic operation under a guessed name.

## Prefer the Highest Safe Layer

Use semantic `prepare` operations for application code when available. They can normalize application values, derive routine addresses, re-read authoritative accounts, and return a portable prepared value with signer and error metadata.

Use `raw.<instruction>.build` when the caller intentionally needs the IDL wire shape or manual composition. Raw builders are pure and offline, but use exact raw names and units. They fail closed on unknown parameters and missing required values.

Use account readers for release-addressed state. Use generic `chain` reads only for data that is not a program account surface.

For the complete prepare/build/inspect/execute lifecycle, read [references/operation-lifecycle.md](references/operation-lifecycle.md).

## Separate Construction from Execution

The following are different authority levels:

1. Explore or read account data.
2. Derive addresses or build an unsigned instruction.
3. Prepare and inspect an unsigned operation.
4. Ask a wallet to sign.
5. Submit and confirm on-chain.

A request to “build,” “prepare,” “show,” or “inspect” a transaction does not authorize signing or submission. Preserve that boundary in code and during agent execution.

Before implementing any wallet or submission path, read [references/transaction-safety.md](references/transaction-safety.md).

## Implement by Project Language

Read only the relevant reference:

- [TypeScript](references/typescript.md)
- [React](references/react.md)
- [Rust](references/rust.md)
- [Python](references/python.md)

Match the project's existing wallet ecosystem. Add an Arete adapter for that ecosystem rather than replacing the application's wallet stack without need.

## Verify

- Confirm the installed ProgramSpec and Program Release identities match exploration.
- Test pure builders and PDA derivation without a live wallet where possible.
- Assert important program IDs, writable accounts, signer accounts, token programs, units, and slippage limits.
- Inspect or simulate before a real send when the transport supports it.
- In tests, use a mock/custom transaction transport and prove that ambiguous submission is never retried.
- After confirmation, refresh exactly the views and reads the operation affects, worked out from its accounts and each installed stack's entities (see the Reconcile section of [references/operation-lifecycle.md](references/operation-lifecycle.md#reconcile)). A caught-up processed-slot watermark alone does not prove an entity changed.
