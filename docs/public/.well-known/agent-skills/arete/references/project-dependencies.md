# Project Dependencies

Read this reference when changing `arete.toml`, resolving `arete.lock`, or maintaining generated SDK outputs.

## Model

- `arete.toml` is the human-authored manifest.
- `arete.lock` pins resolved stack and program identities.
- Generated SDK outputs are owned through recorded provenance.
- `[dependencies]` is for installable stack and program SDKs.
- `[authoring]` is for local ProgramSpec and StackManifest artifacts. Do not put legacy `.stack.json` inputs there.

Inspect an unfamiliar project before changing it:

```bash
a4 config validate
a4 sdk list
```

Use `a4 <command> --help` before relying on optional flags.

## Add and Reproduce

Add a dependency with the exact kind and reference returned by exploration:

```bash
a4 install stack <stack-ref> --ts
a4 install program <program-ref> --ts
```

Use `--alias` when the local name should differ. Use `--exact` when the project must save the exact selected version rather than a compatible requirement.

Reproduce the manifest without changing resolution:

```bash
a4 install --locked
```

Preview the complete install graph without writing:

```bash
a4 install --dry-run
```

## Update and Remove

Advance dependencies within their saved requirements:

```bash
a4 update
a4 update stack <alias>
a4 update program <alias>
```

Remove one dependency and its provenance-owned outputs:

```bash
a4 remove stack <alias>
a4 remove program <alias>
```

Use `--keep-output` only when the user wants to retain generated files after removing the dependency.

## Boundaries

- Do not manually combine a manifest and a lockfile from different resolutions.
- Do not edit generated outputs; change the source dependency or extension and regenerate.
- Do not consent to paths outside the project merely to bypass an error. `--allow-outside-project` requires an intentional project layout and user authorization.
- Do not delete unowned output. Let the installer use its provenance record.
- Commit the manifest and lockfile when the repository's dependency policy expects reproducible installs.
