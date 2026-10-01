# Managed Solana capabilities v1

This implements the OSS portion of the liquidity-management plan against the
0.28.0 source baseline. Ship these changes in the next compatible linked
SDK/server/compiler release; installed 0.28.0 packages do not contain these new
APIs. The contract identifier is `managed-solana/v1`, independently versioned
from SDK package versions. The existing `program-read-http/v1` transport remains
available; the new contextual/query operations are additive.

## Frozen wire contracts

The schemas, capability descriptor, example binding descriptors and deterministic
fixtures are in [`tests/fixtures/managed-solana-v1`](../../../tests/fixtures/managed-solana-v1).
Run `scripts/package-managed-solana-fixtures.sh` to build the release asset.
The release workflow uploads this bundle and publishes `arete-solana-contracts`
before its dependent SDK/server releases. Platform implementations can target the
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
reads to budget inventory. The existing `balance(owner, mint)` selects one
matching token account; it does not aggregate a wallet's accounts.

All u64 quantities use decimal strings on the wire. Rust exposes u64, TypeScript
bigint and Python int. Transaction meta's fee, pre/post lamports and resource
integers remain decimal strings in the retained JSON metadata; token amounts
already use exact decimal strings. Other retained integer fields outside the
JavaScript exact range are also strings. Account indexes remain numbers.

## Provider and platform handoff

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
The platform owns the implementation and supported release/type/filter matrix.
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
u128/i128 payload values remain exact. Python readers preserve payload JSON, and
resolved enum aliases carrying payloads use Any rather than incorrectly claiming
str. Python generation does not yet offer fully typed IDL-only account models;
use its AccountReader with an explicit parser or raw mapping. Mixed named/tuple
enum layouts are rejected rather than emitted as valid-looking unit variants.

Generated account decoders expose `try_from_bytes_exact`. It rejects trailing
bytes with `unsupported_layout`; existing `try_from_bytes` retains its padded
account behavior. A platform release limited to fixed layouts must select the
strict decoder or enforce account length before decoding. The fixed-position
fixture isolates the 70-entry resize boundary. It is a synthetic acceptance
fixture, not a curated production Meteora IDL. Extended Meteora positions remain
unsupported until stack definitions and platform decoder policy declare support.

## Validation and release assembly

`scripts/check-managed-liquidity.sh` validates contracts against fake HTTP servers
and an injected index, schema/fixture parity in all languages, actual fees/token
balances for legacy/v0/v1 successful/failed transactions, and absent metadata.
It also compiles generated Rust program/stack subscription interfaces and runs
generated models, instruction builders and readers through a mock managed service,
executes generated TypeScript schemas, and independently checks managed decoder
payloads, unknown tags, truncation and strict fixed-layout boundaries. It needs no
live chain, hosted service, provider credentials or protocol stack deployment.

The fixture bundle, integration harness and migration notes are ready for
platform and stack consumers. Publish the linked release group after CI; both
release and recovery upload the reproducible fixture asset before publishing
dependent packages. Real hosted
conformance, curated protocol regeneration and binding availability checks remain
release-assembly work before bot implementation.
