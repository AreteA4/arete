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
