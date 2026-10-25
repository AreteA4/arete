---
name: arete-deploy
description: Publish Solana ProgramSpecs and plan, deploy, inspect, stop, archive, promote, or delete hosted Arete resources. Use for a4 program lifecycle, a4 up, deployment branches/previews, hosted stack operations, and taking an Arete Protocol to a private hosted deployment with a4 protocol deploy (review, activation outcomes, the owner-private registry package). This skill performs external mutations only when the user's request authorizes them; use arete-stack-authoring for local DSL and artifact work and arete-protocols for protocol authoring and a4 dev.
metadata:
  version: "1.3.0"
  min-cli: ">=0.45.0"
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

## Deploy an Installed Stack

A stack installed with `a4 install stack` deploys by its dependency alias. This is how a definition-only catalog stack (one with no hosted stream) gets a stream of its own:

```bash
a4 up <alias> --dry-run
a4 up <alias>
```

`a4 up <alias>` deploys exactly the StackManifest, LiveSpecs, and ProgramSpecs `arete.lock` pins for that dependency; an `[authoring.stacks]` entry with the same name takes precedence. The lock must be fresh, so run `a4 install` first when it is not.

After a healthy production deployment, `a4 up <alias>` also edits the project: it records the deployment's endpoints under that dependency in `arete.toml` and reinstalls, so the generated SDK connects to the new deployment. Treat that as part of the mutation the user authorized. Branch, preview, and `--json` deployments leave `arete.toml` unchanged. See [references/deployment-lifecycle.md](references/deployment-lifecycle.md#installed-stacks).

## Protocol Deployment

`a4 protocol deploy` takes a protocol project (authored with
`arete-protocols`) to a private hosted deployment. It needs a secret key;
publishable keys are refused. Preview first:

```bash
a4 protocol deploy --dry-run                 # builds; files, expected release hash, slug, create vs reuse, programs to push
a4 protocol deploy --fee-policy fee-policy.json
a4 protocol deploy --no-activate --json
```

A real run builds the project (`--no-build` uploads the existing build and
refuses it when sources changed), registers each program installed from a
local IDL as the user's private program when Arete doesn't have that exact
ProgramSpec yet (and records it in `arete.lock`), uploads the release,
creates or reuses a private deployment (a new one needs a fee policy), and
activates it. All of that is one authorized mutation; a request to build,
check or run locally is not authorization to deploy.

| `status` | Meaning |
|---|---|
| `active` | Template-only protocol, private, trust tier `unreviewed`. Activation waited while the server prepared the deployment and its stack (`--wait-secs`). |
| `review-required` | The release has handler code, which needs Arete's review before it can be activated. Report the printed release hash and deployment id; do not try to activate it another way. |
| not active, with `pending` | The wait ran out while the server was still preparing; running deploy again resumes. |
| `dry-run` | Nothing changed. |

Public deployments are never self-serve. Present an `unreviewed` deployment
as private and unreviewed, never as public or reviewed.

On activation, Arete publishes an owner-private registry package of the
protocol, reported as `registryPackage`:

- `published` or `existing`: `a4 install protocol <slug> --ts` resolves it
  for this account (others can't), and deploy reinstalls the project's own
  client from it, so the client calls the new deployment.
- `skipped`, with `code` and `message`: the deployment is still active, but
  no package was published. For `package-version-immutable`, the spec
  `version` already pins other content: bump it and deploy again (with the
  user's go-ahead). Until a package exists, the project's client is
  generated from the deployed build, and `a4 install --check` fails.

Verify after deploying: the status, the deployment id and trust tier, the
`registryPackage` outcome, any pushed programs (`a4 program list --json`),
and that `arete.toml` and `arete.lock` changed only as deploy reported.

A protocol's automations deploy to the app's own host (Cloudflare, Vercel or
a Node container), not to Arete: `npx @usearete/automations add <host>` writes
the files and prints the steps, and `a4 protocol webhook add` registers the
endpoint. See the `arete-protocols` automations reference.

## Mutation Boundaries

The following require explicit scope from the user:

- agent signup or key creation;
- program push, archive, or promotion;
- protocol deploy (it may also push programs and publishes a private package), or deploying again after a version bump;
- registering, testing or removing a protocol webhook endpoint (`a4 protocol webhook`), or deploying automations to a host;
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
- for an installed stack, confirm `arete.toml` records the returned endpoints and the regenerated SDK uses them, or report why `a4 up` did not record them;
- distinguish a submitted operation from a ready deployment;
- do not silently clean up a failed resource if deletion/archive was not requested.
