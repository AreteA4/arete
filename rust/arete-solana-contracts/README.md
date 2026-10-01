# Managed Solana contracts

Shared server/SDK types for `managed-solana/v1`: contextual account reads, indexed
owner token inventory and native position discovery. Exact u64 quantities use
decimal strings on the wire. Indexed provenance never asserts chain commitment.

The versioned schemas and fixtures are published as the `managed-solana-v1.tar.gz`
release asset before the dependent SDK/server releases. Source fixtures live in
`tests/fixtures/managed-solana-v1` at the repository root. See
`docs/internal/managed-solana/README.md` for routes, migration and handoff.
