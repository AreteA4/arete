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

The `decoder-fixture-set` kind remains available to internal conformance
tooling, but its fixture DTOs and validators are deliberately not exported from
the package root.
