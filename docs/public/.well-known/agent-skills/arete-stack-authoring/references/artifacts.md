# Portable Artifacts

Read this reference when validating build output, composing independently authored packages, or handing an artifact to SDK generation or deployment.

## Artifact Roles

- **ProgramSpec**: endpoint-free program identity, normalized IDL, accounts, instructions, types, and PDAs.
- **LiveSpec**: entity, mapping, handler, resolver, and view behavior over exact ProgramSpecs.
- **StackManifest**: client composition of aliased LiveSpecs, ProgramSpecs, and selected views. It contains no deployment URL.

Artifacts are content-addressed. Do not edit their JSON after generation; rebuild from source.

`cargo build` normally emits files under `.arete/`:

```text
<program>.program-spec.json
<Stack>.live-spec.json
<Stack>.stack-manifest.json
```

## Program-Only Artifact

Normalize an IDL without a live projection:

```bash
a4 program build idl/program.json --output artifacts/program.program-spec.json
```

Use this for standalone program SDKs, Program Read, or later composition.

## Compose Independent Live Packages

```bash
a4 stack compose \
  --name my-app \
  --program ./artifacts/program.program-spec.json \
  --live markets=./artifacts/Markets.live-spec.json \
  --artifact-dir ./artifacts \
  --selected-view markets=Market/list \
  --output ./artifacts/MyApp.stack-manifest.json
```

Use stable aliases. `--selected-view` is an exact ordered allowlist; omit it only when every view should be exposed. Run `a4 stack compose --help` for current repeatable flag syntax.

## Validate Locally

```bash
a4 config validate
a4 sdk create --manifest ./artifacts/MyApp.stack-manifest.json --ts
```

The manifest directory is the default artifact search root. Add explicit `--artifact-dir` roots for dependencies outside it. Avoid symlink, parent-traversal, ambiguous duplicate, or unapproved artifact roots.

Confirm that generated code contains only intended views/programs and records provenance. A deterministic clean rebuild should reproduce the same hashes from the same source inputs.

Deployment consumes the exact reviewed StackManifest but is not part of local authoring.
