# Managed Solana contracts

Shared server/SDK types for `managed-solana/v1`: contextual account reads, indexed
owner token inventory and native position discovery. Exact u64 quantities use
decimal strings on the wire. Indexed provenance never asserts chain commitment.
`AccountTombstone` defines authoritative chain deletion independently of view
eviction; ingestion owns recognition, ordering and entity mapping.

The versioned schemas and fixtures are published as the `managed-solana-v1.tar.gz`
release asset with a SHA-256 checksum before the dependent SDK/server releases.
Bundle 1.0.0's `manifest.json` is the canonical route/method, defaults and error
manifest. The planned linked release is 0.29.0; published 0.28.0 lacks these APIs.
Source fixtures live in
`tests/fixtures/managed-solana-v1` at the repository root. See
`docs/internal/managed-solana/README.md` for routes, migration and handoff.
