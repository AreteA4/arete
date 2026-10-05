# Managed Solana capabilities v1

These contracts give Gateway and SDK consumers paginated token-account discovery,
account reads with their actual chain context, and complete available transaction
execution data. Wallet services, portfolio tools, indexers and transaction
workflows can use the same contracts across Rust, TypeScript and Python.
Generated account models and decoders preserve enum payloads and exact integers
so applications can read program state without losing layout or value information.

The changes target the 0.28.0 source baseline. Ship them in the next compatible
linked SDK/server/compiler release; installed 0.28.0 packages do not contain these new
APIs. The contract identifier is `managed-solana/v1`, independently versioned
from SDK package versions. The existing `program-read-http/v1` transport remains
available; the new contextual/query operations are additive.

## Frozen wire contracts

The schemas, capability descriptor, example binding descriptors and deterministic
fixtures are in [`tests/fixtures/managed-solana-v1`](../../../tests/fixtures/managed-solana-v1).
Run `scripts/package-managed-solana-fixtures.sh` to build the release asset.
The release workflow uploads this bundle and publishes `arete-solana-contracts`
before its dependent SDK/server releases. Service implementers can target the
bundle before those releases finish.

| Operation | Route | Request | Response |
| --- | --- | --- | --- |
| Owner token inventory | `POST /chain/v1/owner-token-accounts` | `owner`, optional `mint`, `tokenProgram`, `limit`, `cursor` | `items`, `nextCursor`, `discovery` |
| Raw contextual single | `POST /chain/v1/account` | `address`, optional `options` | `{context, value}`; absent account is `value: null` |
| Raw contextual batch | `POST /chain/v1/accounts` | `addresses`, optional `options` | `{context, value}`; aligned array with null missing entries |
| Typed contextual single | `POST /v1/releases/<release>/accounts/<type>/<address>/context` | optional `options` | `{context, value}`; missing stays null |
| Typed contextual batch | `POST /v1/releases/<release>/accounts/<type>/context` | `addresses`, optional `options` | `{context, value: {items}}`; aligned ok/missing/error entries |
| Native position discovery | `POST /v1/releases/<release>/accounts/<type>/query` | owner and/or pool, optional limit/cursor | addresses, nextCursor, discovery |
| Actual transaction | `POST /transactions/v1/get` | signature, optional confirmed/finalized commitment and maximum supported version | compatible summary plus transaction, meta, version, metadataAvailable |

`options` carries `commitment` (processed/confirmed/finalized, default confirmed)
and `minContextSlot` (decimal u64 string). `context.slot` is the actual RPC slot.
One nonempty batch uses one underlying `getMultipleAccounts` call. Empty batches
return null context because no chain read occurred. Separate requests have
separate contexts; minimum slot does not imply an atomic snapshot.

Batch and page limits are 100 and are further bounded by a session's address
limit on the server. Cursors are opaque, nonempty and at most 2048 UTF-8 bytes.
Providers must bind cursors to owner/mint/token-program filters and reject
reuse under changed filters with `invalid_cursor`. Both optional token filters
mean intersection. Invalid public keys, page bounds and unknown request options
fail. Native queries require at least owner or pool and an exact release/type.

Discovery records include provider source, actual RFC3339 `observedAt` and an
optional real index watermark. An absent watermark is unknown. Discovery does
not assert a chain commitment. Discover addresses, then use contextual verified
reads to verify account state at the required minimum slot. The existing
`balance(owner, mint)` selects one matching token account; it does not aggregate
a wallet's accounts.

All u64 quantities use decimal strings on the wire. Rust exposes u64, TypeScript
bigint and Python int. Transaction meta's fee, pre/post lamports and resource
integers remain decimal strings in the retained JSON metadata; token amounts
already use exact decimal strings. Other retained integer fields outside the
JavaScript exact range are also strings. Account indexes remain numbers.

## Provider integration

Implement `arete_server::token_discovery::OwnerTokenAccountsProvider` and pass it
to `Server::solana_gateway(...).owner_token_accounts_provider(...)`, the general
server builder or `HttpHealthServer`. Discovery works without an RPC URL.
The shared server validates page size, filters, account identities/state and
provenance. Providers own index access, credentials, cursor integrity and actual
observation time. Unsupported discovery returns HTTP 501 with
`unsupported_capability`; bad cursors return HTTP 400 with `invalid_cursor`.

The shared Program Read handlers implement contextual reads with release
authorization and decoder ownership checks. Native position querying is a
transport contract: the shared server explicitly returns `unsupported_capability`.
Hosted services supply the implementation and supported release/type/filter matrix.
SDK readers expose `query_positions` / `queryPositions`; the generic query
executor does not advertise availability of a native endpoint.

Existing binding identities and read/auth scopes are retained. The capability
descriptor describes contract support; it is not proof of hosted deployment.

## Migration and account layout support

Old raw account routes and typed fetch/exists methods remain available. Use the
new `account_with_context` / `accountWithContext`, `accounts_with_context` /
`accountsWithContext`, and typed `fetch_with_context` / `fetchWithContext` APIs
when read freshness matters. No RPC-dependent helper is needed for these reads.
New batches reject malformed cardinality and address alignment rather than
padding or shifting results. Rust custom ChainClient implementations inherit
unsupported defaults for the additive methods; TypeScript/Python implementations
should implement them or report unsupported capability explicitly.

Transaction summary fields remain present. New clients accept summaries from
older relays: missing new fields mean unknown. New relays distinguish an unseen
transaction (null outer transaction) from unavailable execution metadata
(`metadataAvailable: false`, `meta: null`, empty summary accounts). Do not treat
unknown execution error/fees as success/zero. An estimated fee is a separate API.
Rust consumers constructing `ConfirmedTransaction` literals must initialize its
four new optional fields. Custom TypeScript/Python transports must expose `get`.

TypeScript schemas and the Rust managed decoder now preserve unit, named and
tuple enum payloads, including Orca's `Initialized(DynamicTickData)` variant.
Rust IDL-only program and stack generation emits account models and readers;
u128/i128 payload values remain exact. The current Rust stack generator also
keeps mapped Orca live-model u128/i128 fields at their wide integer types; an
artifact generated from an older source pin must not be used to infer narrowing
in the planned 0.29.0 output. Python readers preserve payload JSON, and
resolved enum aliases carrying payloads use Any rather than incorrectly claiming
str. Python generation does not yet offer fully typed IDL-only account models;
use its AccountReader with an explicit parser or raw mapping. Mixed named/tuple
enum layouts are rejected rather than emitted as valid-looking unit variants.

Generated account decoders expose `try_from_bytes_exact`. It rejects trailing
bytes with `unsupported_layout`; existing `try_from_bytes` retains its padded
account behavior. A service limited to fixed layouts must select the
strict decoder or enforce account length before decoding. The fixed-position
fixture isolates the 70-entry resize boundary. It is a synthetic acceptance
fixture, not a curated production Meteora IDL. Extended Meteora positions remain
unsupported until stack definitions and decoder policy declare support.

## Validation and release assembly

`scripts/check-managed-solana.sh` validates contracts against fake HTTP servers
and an injected index, schema/fixture parity in all languages, actual fees/token
balances for legacy/v0/v1 successful/failed transactions, and absent metadata.
It also compiles generated Rust program/stack subscription interfaces and runs
generated models, instruction builders and readers through a mock managed service,
executes generated TypeScript schemas, and independently checks managed decoder
payloads, unknown tags, truncation and strict fixed-layout boundaries. It needs no
live chain, hosted service, provider credentials or protocol stack deployment.

The fixture bundle, integration harness and migration notes are ready for
service and stack consumers. Publish the linked release group after CI; both
release and recovery upload the reproducible fixture asset before publishing
dependent packages. Integrations that depend on hosted discovery or native
queries must verify provider conformance, supported account layouts and binding
availability before enabling those capabilities.

## Release and canonical contract

Published SDK/server/compiler **0.28.0 does not contain these APIs or runtime
fixes**. The first planned linked release is **0.29.0**, with
`arete-solana-contracts` **0.1.0**. Both are pending publication; a checkout whose
package manifests still say 0.28.0 is not evidence that the registry release
contains this work. Consumers must use the merged revision or the published
0.29.0 release before enabling the integrations.

`tests/fixtures/managed-solana-v1/manifest.json` is the canonical route/method,
request/response schema, default and error manifest for fixture bundle **1.0.0**.
Release automation attaches `managed-solana-v1.tar.gz` and its SHA-256 checksum to
`arete-solana-contracts-v0.1.0`. The archive includes HTTP examples, exact integer
and unknown metadata cases, the account deletion boundary, and complete pinned
Orca Whirlpool/SPL Token IDLs with source hashes. Routes remain
`managed-solana/v1`; Service implementations align their routes and envelopes to these fixtures.

Empty and filter-only derived views read the source cache when there is no
sorted cache. Pipeline filters run before subscription filters; unsorted results
use numeric `_seq` order (descending by default, ascending for `after`), with key
ties. `skip`/`take` still select the live window; a pipeline limit caps `take`.
`snapshotLimit` truncates initial delivery only. Reconnect replaces the client
snapshot, including an empty result. Sorted views keep their configured order.

Source deletion uses `Mutation::delete`; VM-backed sources use
`VmContext::delete_entity` plus `discard_account`. Account-owned entity mutations
and deletes also carry `Mutation::mark_account_position(AccountPosition::new(slot,
write_version))`; instruction, resolver and resend mutations do not. Use
`SlotContext::account`, `SlotContext::instruction` and `SlotContext::resolver`
for generated ingestion, or `SlotContext::with_domain` in a custom producer.
`SlotContext` still controls `_seq` recency within its declared domain, but never
account lifetime ordering or recency in a different domain. The projector
removes cached and sorted rows before publishing `delete`. Predicate/window
departure remains `remove`. Deleted rows accept only a newer complete creation
marked `mark_created`, so resends and sparse patches cannot resurrect them.
Snapshot payloads retain explicit live/deleted account checkpoints and independent
source-domain recency cursors, bounded to eight times the entity cache capacity
per view. Ingestion owners must retain
durable ingestion deduplication and resume watermarks beyond that cache retention.

See [Account lifecycle integration](account-lifecycle.md) for the public ingestion
boundary. The validation harness compiles every instruction and account reader
from the complete public Orca Whirlpool IDL and the public Anchor SPL Token IDL.
The SPL adapter explicitly declares native account layouts and option encodings;
its source and transformation are bundled for reproducibility.

Decoder regressions construct an Orca `DynamicTickArray` containing 88 enum values
with exact u128/i128 payloads and independently pack a 165-byte SPL Token account
from its public layout. Both are constructed test data, not captured accounts.
`production-idls/provenance.json` records only public upstream sources and hashes.

IDLs can explicitly declare discriminator-free account layouts with
`arete.account_untagged=true` in account docs and an empty discriminator. Omitting a
discriminator without that declaration retains Anchor derivation. Untagged managed
reads select the requested account type and require an exact layout. Generic
untagged parsing rejects ambiguous layouts.
