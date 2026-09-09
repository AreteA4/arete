# `@usearete/hash`

Typed, domain-separated Arete artifact identities. The package implements hash
protocol v1 and is byte-for-byte conformant with `test-vectors/hash-v1.json`.

```ts
import { parseIdlV1, hashArtifactTree } from "@usearete/hash";

const idl = parseIdlV1(idlBytes);
console.log(idl.hashes.content);

const output = hashArtifactTree("sdk-output-tree", [
  { path: "src/index.ts", bytes: new TextEncoder().encode("export {};\n") },
]);
```

`idl-source` preserves exact input bytes. JSON identities use the Arete JCS v1
profile, which rejects duplicate keys, malformed UTF-8, unsafe integer tokens,
non-finite numbers, and non-JSON values. Artifact trees sort canonical POSIX
paths by raw UTF-8 bytes and never normalize file contents.

The package root exports the address-free `arete.decoder-fixtures/v2` DTO,
strict parser, validator, and hash function. It also exports strict
`arete.solana-executable-identity/v1` and hosted managed
`arete.program-release/v2` contracts. OSS-generated Program Release V1 hashes
remain byte-compatible and use the same `program-release`/`h1` domain.
The root also exports strict hosted-private `arete.program-release/v3`
creation, parsing, validation, and hashing helpers. V3 fixes
`executablePolicy` to `observed` and deliberately has no ownership or
visibility fields.

## SDK content identity

`SdkDefinitionV2` identifies generated content, its input, target, and runtime
contract independently of compiler provenance. The V1 projection remains frozen.
See [the content identity contract](../../docs/internal/sdk-content-identity.md).
