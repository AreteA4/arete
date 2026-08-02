# arete-hash

Authoritative typed artifact identities and canonical hashing for Arete.

The crate implements the versioned `arete:h1` protocol, including strict hash
identifier parsing, JCS canonicalization, framed tuples, artifact trees, IDL
projections, and program release projections.

The internal `arete.decoder-fixtures/v1` projection hashes as
`decoder-fixture-set`. Cases are validated and sorted by stable ID before JCS
hashing; exact account bytes and optional private diagnostics participate in
identity. The kind is marked `internal-only` and is not a public artifact
identity. Error expectations use the stable public account decode categories;
private diagnostics contain only trailing-byte and candidate counts.

The shared conformance vectors live in `../test-vectors/hash-v1.json` and are
also consumed by `@usearete/hash`.

## License

Apache-2.0
