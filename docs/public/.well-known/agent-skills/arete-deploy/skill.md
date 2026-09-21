---
name: arete-deploy
description: Publish Solana ProgramSpecs and plan, deploy, inspect, stop, archive, promote, or delete hosted Arete resources. Use for a4 program lifecycle, a4 up, deployment branches/previews, and hosted stack operations. This skill performs external mutations only when the user's request authorizes them; use arete-stack-authoring for local DSL and artifact work.
metadata:
  version: "1.0.0"
  min-cli: ">=0.13.0"
---

# Publish and Operate Hosted Arete Resources

Operate on exact content-addressed artifacts and explicit hosted resource identifiers. Separate read-only planning from external mutation, and do not infer deployment authority from a request to build or validate local code.

## Health and Authentication

When environment health is relevant:

```bash
a4 doctor --json
a4 auth status --json
a4 auth whoami --json
```

Follow current CLI fixes. If the user explicitly wants an agent account and none exists, `a4 auth signup` can create and store one. Do not print, commit, or paste returned secret credentials into source, logs, or browser configuration.

Use publishable, origin-bound keys for browser clients. Account API keys and publishable keys are not interchangeable.

## Start Read-Only

Resolve exact targets before any mutation:

```bash
a4 program list --json
a4 program status <user-program-id> --json
a4 program events <user-program-id> --json
a4 stack list --json
a4 stack show <stack-name> --json
a4 stack versions <stack-name> --json
a4 status --json
```

Do not operate on a display label when an immutable hash or returned resource ID is required. Preserve opaque cursors and IDs exactly.

## Program Publication

Program publication has distinct steps:

1. Build or validate a local ProgramSpec.
2. Push it as an owner-private hosted program.
3. Wait for or inspect admission and health.
4. Optionally request reviewed promotion with explicit public-IDL consent.
5. Optionally archive the registration while immutable content remains.

Read [references/program-publication.md](references/program-publication.md) before pushing, promoting, or archiving.

## Stack Deployment

Deploy the exact reviewed StackManifest, not a legacy stack name guessed from a crate:

```bash
a4 up .arete/<Stack>.stack-manifest.json --dry-run
a4 up .arete/<Stack>.stack-manifest.json
```

Use a dry run first unless the user explicitly requested an immediate repeat of an already-reviewed deployment. Compare the plan's StackManifest, ProgramSpec, and LiveSpec identities to local artifacts.

Branches and previews are separate deployments:

```bash
a4 up .arete/<Stack>.stack-manifest.json --branch staging --dry-run
a4 up .arete/<Stack>.stack-manifest.json --preview --dry-run
```

Run `a4 up --help` for the installed surface and read [references/deployment-lifecycle.md](references/deployment-lifecycle.md) before a live deploy, stop, or delete.

## Mutation Boundaries

The following require explicit scope from the user:

- agent signup or key creation;
- program push, archive, or promotion;
- live stack deployment or replacement;
- branch/preview creation;
- deployment stop;
- stack deletion.

A dry run, list, show, status, version, or event query is read-only. It can be used to resolve ambiguity before asking for missing authority.

Never add `--force`, `--yes`, public-IDL consent, `--allow-unverified-programs`, or an equivalent bypass solely to avoid a prompt or failed precondition. Each flag represents a decision that must already be within the user's request.

## Verify and Report

After a mutation:

- verify the response echoes the intended artifact/resource identity;
- wait only when requested or needed for the requested outcome, using bounded status polling;
- inspect final admission, build, deployment, endpoint, and health state;
- report branch/preview identity and returned endpoints without exposing credentials;
- distinguish a submitted operation from a ready deployment;
- do not silently clean up a failed resource if deletion/archive was not requested.
