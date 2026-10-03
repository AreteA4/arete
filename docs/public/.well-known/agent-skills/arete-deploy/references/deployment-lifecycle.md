# Stack Deployment Lifecycle

Read this reference for planning, creating, inspecting, stopping, or deleting a hosted stack deployment.

## Preflight

1. Build and validate the local ProgramSpec, LiveSpec, and StackManifest closure.
2. Record the exact StackManifest hash and selected views.
3. Confirm required programs are admitted and healthy.
4. Confirm authentication identity and target environment.
5. Decide production, named branch, or generated preview explicitly.

Run a server-aware dry run:

```bash
a4 up <manifest> --dry-run --json
```

Use `--local-only` only with `--dry-run` when intentionally validating without server checks. It is not evidence that hosted dependencies are ready.

Do not use `--allow-unverified-programs` unless the user accepts a deployment plan containing observed private programs and the risk is understood.

## Deploy

```bash
a4 up <manifest> --json
a4 up <manifest> --branch <branch> --json
a4 up <manifest> --preview --json
```

Branch and preview flags conflict; choose one. Preserve the returned deployment/resource identifiers and endpoint bindings.

A successful submission can still be building or preparing. Query `a4 status`, `a4 stack show`, or the command-reported status path until the requested terminal outcome is reached.

## Installed Stacks

`<manifest>` can also be the alias of an installed stack dependency:

```bash
a4 up <alias> --dry-run
a4 up <alias>
```

- The deployed closure is the one `arete.lock` pins, read from the registry cache that `a4 install` fills. Each cached artifact must still hash to its locked identity. A missing entry is fetched again at the locked release. `--dry-run --local-only` never fetches and instead asks for `a4 install`.
- The alias names the deployment unless `[authoring.stacks]` has an entry with the same name, which wins.
- A plain `a4 up` deploys only authored stacks. Name the installed stack explicitly.
- After a healthy production deployment, the CLI writes the returned endpoints into the dependency and runs `a4 install`:

  ```toml
  [dependencies.stacks.<alias>]
  source = { registry = "<package>" }
  version = "^1.0.0"
  endpoints = { live = { websocket = "wss://…", query = "https://…" } }
  ```

  `endpoints` has one entry per LiveSpec alias. For a definition-only stack it replaces the SDK's empty placeholders. For a hosted stack it replaces the managed stream endpoints, while the managed chain and transaction gateway stays. Removing the table and running `a4 install` returns the SDK to the stack's own endpoints.
- If recording or reinstalling fails, the deployment still stands and `arete.toml` is left as it was. Report the failure; do not hand-write endpoints the CLI did not return.
- After `a4 update` or a new version requirement moves the stack to a different StackManifest, `a4 install` notes that the recorded deployment still serves the previous one. Redeploy with `a4 up <alias>`, which needs the user's authorization.
- `--branch`, `--preview`, and `--json` deployments never edit `arete.toml`. Pass their endpoint to the client explicitly when the user wants the SDK to read one.

## Inspect History

```bash
a4 stack show <stack-name> --json
a4 stack versions <stack-name> --json
```

Use immutable version and artifact identity when comparing or diagnosing deployments. Do not assume the latest numeric version is the desired rollback target.

## Stop

```bash
a4 stack stop <stack-name> --branch <branch>
```

Omit `--branch` only when the intended target is production. Confirm the exact active deployment before stopping. `--force` bypasses confirmation and therefore requires already-established authorization.

## Delete

```bash
a4 stack delete <stack-name>
```

Deletion is destructive. Inspect the stack, versions, active branches, and dependent clients first. Do not delete as automatic cleanup after a failed deploy. Report whether deletion is reversible according to the current CLI/control-plane response.
