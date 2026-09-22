# Program Publication

Read this reference before pushing, promoting, or archiving a hosted program.

## Build Locally

Normalize an IDL into a ProgramSpec when needed:

```bash
a4 program build idl/program.json \
  --program-id <program-id-if-absent> \
  --output artifacts/program.program-spec.json
```

If the input is already a ProgramSpec, validate its kind, program ID, and artifact hash. Do not pass a conflicting program ID.

## Push Private Program

```bash
a4 program push artifacts/program.program-spec.json --wait --json
```

The push creates an owner-private registration and admission job. Preserve the returned `upr_...` identifier. The response must echo the uploaded program ID and ProgramSpec hash.

Use `--idempotency-key` only with a canonical UUID deliberately persisted for retrying the same logical upload. Reusing a key for different content is incorrect.

If `--wait` times out, query status; do not immediately create a second registration:

```bash
a4 program status <user-program-id> --watch --json
a4 program events <user-program-id> --json
```

Ready admission should include an exact Program Release hash and Program Read binding. Treat failed or unhealthy admission as a diagnostic result, not an invitation to fabricate an install descriptor.

## Promotion

```bash
a4 program promote <user-program-id> --make-idl-public --json
```

Promotion requests review and permits the baseline IDL to enter public OSS distribution. The consent flag must reflect explicit authorization from the rights holder; never add it automatically.

Promotion is not the same as immediate public availability. Report the returned promotion request and status.

## Archive

```bash
a4 program archive <user-program-id> --yes --json
```

Archival changes the registration lifecycle but retains immutable content. Confirm the exact program ID and current state first. Do not use `--yes` unless archival itself is authorized.
