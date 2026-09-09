# SDK content identity and generation provenance

An internal compiler/interpreter change can produce identical SDK code. Generation
must preserve SDK content identities in that case and still record which compiler
produced the output. A compiler fingerprint is provenance, not a compatibility test.

## Program definitions

The frozen `arete.sdk-definition/v1` projection and its existing Rust/TypeScript
functions remain supported. New TypeScript generation emits `SdkDefinitionV2`:

- `schema`: `arete.sdk-definition/v2`
- `inputKind`: `program-spec`
- `inputHash`: the exact ProgramSpec identity
- `target`: `typescript`
- `runtimeContract`: `@usearete/sdk/program-definition-v1`
- `outputTreeHash`: an `SdkOutputTree` identity over the content described below

The enclosing hash protocol remains `arete:h1:sdk-definition:sha256:...`.
The projection schema is included in the hash; V1 values are never reinterpreted
as V2 values. Both implementations have a shared V2 test vector, in addition to
the unchanged V1 corpus. Consumers such as React treat these identities as opaque
values; an old and a new identity do not accidentally share a cached client.

The TypeScript runtime contract specifies three virtual UTF-8 files, hashed using
the existing artifact-tree-v1 algorithm (exact bytes, sorted relative paths):

| Path | Content |
| --- | --- |
| `imports.ts` | The emitted imports |
| `declarations.ts` | The module's emitted declarations, including account schemas and instruction implementations |
| `definition.ts` | This program's emitted literal sections joined by LF, with `sdkDefinitionHash` absent |

Input `sdk_definition_hash` values are ignored and recomputed. Hash exclusion is
structural, before rendering: no regex removes arbitrary hash-looking strings.
Program IDs, ProgramSpec/IDL identities, PDA rules, schemas, instruction helpers,
and any gateway descriptor in the program literal participate. Source paths,
compiler source bytes, build versions, warnings, and diagnostics do not.

Release identities, read bindings, and stack wrappers retain their existing
separate roles. The complete SDK file tree below includes those emitted values.
Shared declarations are intentionally conservative: a change elsewhere in the
same generated module may change its program identities. This is content identity,
not proof of semantic equivalence or a language-independent public API identity.

The runtime contract must be versioned when the meaning of imported SDK helpers
changes for generated definitions, even if their names do not change. Changes to
the declared content projection also require a new contract. A runtime dependency
version alone is not silently treated as a semantic compatibility guarantee.

## Output inventory and provenance

Every CLI generation writes two metadata documents:

- `sdk-manifest.json`: stable schema version 1, input identity, extension identities,
  artifact inventory, and `sdkOutputTreeHash` over every listed payload file.
- `sdk-provenance.json`: the existing readable V2 provenance format, with compiler
  version/fingerprint and an additive `sdkOutputTreeHash` linking it to the payload.
  Its ownership inventory also includes `sdk-manifest.json` for project installs.

The stable manifest's inventory excludes itself and generation provenance. It
includes staged extensions, entrypoints, generated core modules, and any nested
content manifests. Paths and bytes are hashed exactly; duplicate paths, missing
files, reserved metadata paths, and symlinks are rejected. New content is hashed
before stale owned files are pruned. Unowned files remain untouched.

The example SDKs commit `sdk-manifest.json` and all payload files. Their compiler
provenance is ignored by Git and uploaded by CI as `example-sdk-provenance`.
Release automation no longer rewrites compiler versions in example provenance.
CI continues to regenerate twice and check the entire committed output, so actual
content changes still require regeneration and a commit. Compiler-only provenance
changes no longer fail this check. CLI/project consumers still receive the full
provenance and ownership document; it has not been removed from generated packages.

Deployment parity reads the committed content manifest by default. The existing
`ARETE_ORE_PROVENANCE_PATH` override also accepts old provenance documents.

## Validation

Rust and TypeScript tests verify shared V2 vectors, unchanged V1 identities,
content/target/runtime-contract sensitivity, and invalid inputs. Generator tests
verify that a changed emitted PDA implementation changes identity even with an
unchanged ProgramSpec, and that supplied source identities or different release
references cannot contaminate portable identity. CLI tests verify that compiler
changes leave the content manifest byte-identical, payload changes update its
identity, and invalid payload files cannot be recorded.

Local verification on Rust/Cargo 1.98.0 (aarch64-apple-darwin):

- Rust hash suite: 27 passed, including the shared V2 vector and unchanged V1 corpus.
- TypeScript hash suite: 202 passed; type checking and package ESM/CJS/type smokes passed.
- Interpreter suite: 193 passed, 2 existing ignored tests.
- CLI suite: 335 unit tests and 30 integration tests passed.
- Locked workspace check, Clippy with warnings denied, formatting and diff checks passed.
- Generated Rust program/stack consumer crates built against the local SDK.
- TypeScript SDK build and strict ORE TypeScript compilation passed.
- Adding an interpreter comment and regenerating preserved all 25 payload/manifest
  files byte-for-byte while all 3 compiler provenance fingerprints changed. The
  temporary comment was then removed.
- The TypeScript hash implementation independently verified all three content
  manifests emitted by the Rust CLI against the actual output files.
