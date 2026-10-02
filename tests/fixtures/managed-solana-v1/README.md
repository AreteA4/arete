# Managed Solana v1 fixture bundle

Contract: `managed-solana/v1`. Bundle version: **1.0.0**.

Start with `manifest.json` for canonical routes, HTTP methods, schema references,
defaults, limits, availability and error codes/envelopes. `schema.json` defines
requests and responses. `wire-cases.json` adds HTTP success/error examples; human
error messages are diagnostic and not normative. Existing capability and binding
descriptors identify the same contract independently of package versions.

The bundle covers exact integers, null/empty reads, aligned batch errors, indexed
discovery provenance and full legacy/versioned transaction responses. An unknown
index watermark or unavailable execution metadata must remain unknown.

`account-deletion.json` defines the chain-to-runtime tombstone boundary, rather
than another HTTP operation. `delete` ends an account-derived entity's lifetime;
`remove` evicts it from a view. Recreation requires a complete marked creation.
Ingestion owners enforce ordering and explicitly map accounts to entities. See
`docs/internal/managed-solana/account-lifecycle.md` in the OSS source for the
public integration sequence and cache retention limits.

`production-idls/provenance.json` pins public Orca Whirlpool and Anchor SPL Token
IDL sources and hashes. The complete SPL source and its small native-layout
adapter are included. Decoder account bytes are independently constructed from
these public definitions; no captured account data is included.

The first planned linked SDK/server/compiler release is **0.29.0**, with
`arete-solana-contracts` **0.1.0**; publication is pending. Published **0.28.0 does
not contain the new APIs or runtime fixes**. Release and recovery workflows attach
`managed-solana-v1.tar.gz` and its SHA-256 checksum to the contracts release before
publishing dependent packages. `scripts/package-managed-solana-fixtures.sh`
reproduces the archive from this directory.
