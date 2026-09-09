# Solana transaction observations in ingestion

A4-251 migrates ingestion to published Shipstern and retains the V1 inline
configuration exposed by its shared instruction context. The exact dependency
set is:

| Package | Version |
| --- | --- |
| `shipstern` | `=0.9.0` |
| `shipstern-core` | `=0.9.0` |
| `shipstern-yellowstone-grpc-source` | `=0.9.0` |
| `yellowstone-grpc-client` | `=13.2.1` |
| `yellowstone-grpc-proto` | `=12.7.0` |

All five packages resolve from crates.io. The workspace and standalone ORE
locks use this tuple, with one protobuf type family at Arete's boundaries.
Shipstern's client constraint is `>=13.1, <13.3`. `shipstern-proto` is not needed
by this feature graph. The old `arete::runtime::yellowstone_vixen*` names remain
aliases for the corresponding `shipstern*` exports, without duplicate packages.

## Metadata contract

`arete::transaction_metadata::instruction_update_context` produces an
`UpdateContext` with the existing slot, signature and transaction index. If
Shipstern retained a config, the context contains
`metadata["solana_transaction"]`, for example:

```json
{
  "version": 1,
  "config": {
    "priority_fee_lamports": "18446744073709551615",
    "compute_unit_limit": 0
  }
}
```

A present empty config produces `{"version":1,"config":{}}`. Omitted config
fields stay omitted and explicit zeros stay present. `priority_fee` on the
protobuf maps to `priority_fee_lamports`, serialized as a decimal u64 string.
The remaining optional config fields are `loaded_accounts_data_size_limit`
and `heap_size`. These observations do not insert effective defaults or
normalize fees.

`UpdateContext::solana_transaction` and
`InstructionContext::solana_transaction` return typed observations. Missing
metadata returns `Ok(None)`; malformed saved metadata returns an error.
`UpdateContext::with_solana_transaction` supports authoritative observations
from other readers, including explicit `"legacy"`, `0` and `1` versions.

Shipstern 0.9.0 discards the source version flag. Without config, the ingestion
helper leaves the metadata key absent. It does not classify legacy versus v0
from missing config, lookup tables or account counts. RPC/codec consumers
should retain explicit versions available from their own source. Exact
legacy/v0 classification after Shipstern normalization is the deferred,
non-blocking A4-259 follow-up.

The generated handlers retain their originating context for hooks, CPI/log
events and immediate async resolver application. Queued instructions retain
their full context while awaiting PDA mappings. Nested account processing
uses an account context and restores the surrounding instruction context.
Ordinary event wrappers, program data, state schemas and instruction arguments
have no new transaction fields.

## Connection and replay behavior

Arete's managed source continues to own reconnect backoff and processed-slot
checkpoints. The independent slot subscription never sets that checkpoint.
The new Shipstern reconnect options are explicitly disabled.

Client 13.2.1's high-level subscription unconditionally injects slot/block-meta
filters and wraps updates in its dedup stream, even with reconnect disabled.
Arete therefore subscribes through the client's public `geyser` service with
its exact request and keeps the request stream open. Connection setup, TLS,
authentication, compression and keepalive still use the client builder.
Arete's existing VM replay/dedup behavior receives the original update order.
The source also exits when its receiver closes during an idle upstream stream.

## Verification and release handoff

Local verification uses Rust `1.98.0 (88d9e12ae 2026-08-18)` and Cargo
`1.98.0 (797e8a9bc 2026-08-05)` on aarch64-apple-darwin. The implementation was
initially checked against the A4-251 revised baseline `a30cd285`. It is now
reconciled with `main` at `542dd39b`, preserving the linked **0.15.0** versions,
release lockfile updates, and the SDK simulation-budget/V1 option contract from
PR #198. The ingestion tuple above remains pinned independently.

| Pre-publication gate | Result |
| --- | --- |
| `cargo tree --locked -p arete` | Passed; exact registry tuple above, one proto family |
| `cargo check --workspace --locked` | Passed |
| `cargo test --locked -p arete-interpreter transaction_metadata` | Passed; 4 tests |
| `cargo test --locked -p arete-interpreter --lib` | Passed; 196 tests, 2 existing ignored tests |
| `cargo test --locked -p arete-macros --test transaction_v1_runtime` | Passed; 4 tests |
| `cargo test --locked -p arete-macros --test phase1_runtime` | Passed; 1 test |
| `cargo test --locked -p arete-macros --test artifact_native_v2` | Passed; 6 tests |
| `cargo test --locked -p arete-a4-sdk` | Passed; 0.15.0 SDK contract and integration tests |
| `cargo build --manifest-path stacks/ore/Cargo.toml --locked` | Passed |
| `cargo check --manifest-path examples/ore-server/Cargo.toml --locked` | Passed |
| `bash scripts/check-generated-rust-crates.sh --mode local` | Passed |
| `bash scripts/check-generated-ingestion-runtime.sh --mode local` | Passed |
| `bash scripts/generate-example-sdks.sh` (twice) | Passed; identical output across runs, refreshed compiler/SDK-definition hashes only |
| Local TypeScript SDK build and ORE TypeScript strict compile | Passed |
| TypeScript wallet/transaction/client/session contract tests | Passed; 81 tests |
| Linked release version and npm lockfile synchronization checks | Passed; linked components remain at 0.15.0 |
| `cargo clippy --workspace -- -D warnings` | Passed |
| `git diff --check` | Passed |

The generated-runtime tests serialize and decode protobuf field 7 before
Shipstern conversion, then execute both emitted handler variants. They cover
empty/nonempty V1 config, zeros, maximum fees, unknown legacy/v0 controls,
hooks, nested accounts, CPI/log routing, bundled multi-program dispatch,
duplicate replay and unchanged application state. The transport test runs a
local tonic Geyser server and checks exact filters, disconnect/resume,
duplicate delivery, bounded downstream capacity, idle cancellation and the
independent SlotHashes subscription.

The ingestion consumer script compiles and executes a public `#[arete]`
consumer. Local mode uses this checkout. Registry mode pins the checkout's
exact Arete package version and rejects path/Git dependencies throughout the
resolved graph. CI runs local mode; the release workflow runs registry mode
after publishing Arete.

**Implementation PR:** [AreteA4/arete#195](https://github.com/AreteA4/arete/pull/195).

**Release status:** this ingestion change is unpublished. The existing 0.15.0
release on `main` predates #195 and does not satisfy its registry acceptance gate.
After merging and publishing a release that includes #195, record the actual
release version and run:

```sh
bash scripts/check-generated-ingestion-runtime.sh --mode registry
```

A4-252 must wait for that successful registry result and adopt the actual
published versions. This ingestion release can precede the SDK adapters;
A4-256 owns the joined SDK lifecycle acceptance and A4-257 owns hosted rollout.

Provider readiness remains separate from these local checks. Require
Yellowstone geyser **15.1.1 or later**, or an equivalent provider verified to
retain V1 config before the wire. A provider that silently drops it fails V1
readiness. This minimum is documented in the
[Solana V1 support matrix](https://solana.com/upgrades/larger-transaction-sizes).
No live provider or funded transaction was used for these tests.

References: [A4-251](https://linear.app/arete-a4/issue/A4-251),
[parent A4-247](https://linear.app/arete-a4/issue/A4-247),
[Shipstern V1 config preservation](https://github.com/solana-rpc/shipstern/commit/d9408248476376bfa40dce8b18d24f2e099ce4bf).
