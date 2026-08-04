# Rust SDK Alignment — Phase 2 (full functional parity)

Status: **implemented 2026-08-04** (all modules landed; see `sdk-api-surface.md` §5.5 for
the summary of what shipped and the documented divergences).

Addendum (same day): **devex extension pipeline parity** also landed. The shared
`extensions.json` manifest gained an optional `language` field (`"rust"`; absent =
TypeScript, byte-identical round-trip for existing TS manifests). Rust bundles are flat
`.rs` files (entry `extensions.rs` by convention); `a4 sdk create --rust --extensions`,
`a4 install --rust --extensions`, and `a4 sdk sync --rust` resolve → pin-validate
(stack-manifest / program-spec input hashes, same hard errors as TS) → stage verbatim →
wire into the generated module (`pub mod <stem>;` per file + `pub use <entry>::*;` at the
stack module root — explicit manifest-driven wiring, no TS-style source regex) → write
`sdk-provenance.json`. Sync re-reads the output-dir manifest with pins intact (fixing the
TS silent-unpin sharp edge for the Rust path). Reference bundle: the `OreDevex` trait in
`examples/ore-rust/src/generated/ore/{devex,extensions}.rs`. Registry gap: hosted Rust
bundles need a language dimension on the backend `sdk_extension_contents` before the
hosted resolution rung can match (the CLI already deserializes the field).

Companion to `sdk-api-surface.md` (§2 wire formats, §3 TS surface are the contracts).
Goal: the Rust SDK becomes functionally identical to `@usearete/sdk` using Rust idioms.
TS sources of truth: `typescript/core/src/{chain,transactions,amounts,spl,read,
program-read-transport,operations,signer-registry,session,solana-gateway,account-loader}.ts`
and `wallet/types.ts`.

## Cross-cutting design decisions (all modules follow these)

- **Async traits**: use the `async-trait` crate for object-safe traits
  (`ChainClient`, `TransactionTransport`, `WalletAdapter`); store as `Arc<dyn …>`.
- **Errors**: each module gets a small `thiserror` enum; anything crossing the client
  boundary converts into `AreteError` via `From`. Port the TS transaction outcome model
  exactly: `TransactionOutcome`/`TransactionFailureOutcome` discriminated by status
  (`confirmed | not-submitted | submitted-unknown | chain-failed`) with phase.
- **u64s on the wire are decimal strings** (validated, bounded); Rust surface uses
  `u64`/`u128` natives.
- **Options objects → builder or struct-with-Default**: request options are plain
  structs with `Default` (e.g. `SendOptions`), not builders, mirroring serde needs.
- **HTTP auth**: `http::HttpAuthClient` is the shared token machinery (built from
  `AuthConfig` + fetch): strategy order token > provider > token_endpoint > hosted
  default; targeted tokens (`program-read-binding`, `solana-gateway-binding`) with an
  LRU cache (cap 32) keyed by `(target_kind, target_id, release_hash, sorted_scopes)`;
  refresh-on-401 replay at most once, `send` scope requires the
  `X-Arete-Upstream-Attempted: false` marker (see §2.2/§6 of the surface doc).
- **No `stringifyBigints`/display port** — Rust types are already precise; skip.

## Module map (files are pre-stubbed; each implementer owns only its files)

| File | Contents (mirror of) |
|---|---|
| `src/http.rs` | `HttpAuthClient`, `AuthTokenRequest`/`AuthTokenTarget`, authed JSON fetch with refresh-replay (TS `connection.ts` HTTP-token half + `readJson`) |
| `src/chain.rs` | `trait ChainClient` + `HttpChainClient` (all 9 `/chain/*` routes), `derive_http_endpoint` |
| `src/transactions.rs` | `trait TransactionTransport` + `HttpTransactionTransport` (6 `/transactions/v1/*` routes), `TransactionTransportError` |
| `src/amounts.rs` | `AmountInput` (`Raw(u64-ish)/Ui(String)`), `parse_ui_amount_to_raw`, `format_raw_to_ui`, `to_raw_amount`, `get_mint_decimals`, `resolve_amount(s)_to_raw` |
| `src/spl.rs` | program-address consts, `derive_associated_token_account`, `resolve_token_program_address` |
| `src/wallet.rs` | `trait WalletAdapter` (`sign_and_send(instructions, options, context)`), `SendOptions`, `SendResult`, `WalletExecutionContext`, optional `inspect_transaction` |
| `src/operations.rs` | `PreparedInstruction/Transaction/Flow` (+`PreparedOperation` enum), `create_prepared_*`, prepend/append composition, `execute_prepared_operation`, receipts, callbacks, `OperationExecutionError`, `SignerRegistry` |
| `src/read.rs` | `ProgramAccountReadDef`, `ProgramQueryDef`, `StackQueryDef`, `AccountReader<T>` (`fetch/fetch_many/exists`), `ReadRequestError` |
| `src/program_read_transport.rs` | program-read-http/v1 transport (local-http + hosted-binding), descriptor types + validation (release hashes, `prb_` id regex, https/localhost rule) |
| `src/session.rs` | `Session` (multi-stack/program), member options, program promotion, composition mode |
| `src/gateway.rs` | hosted Solana gateway transports (`sgb_` bindings → ChainClient + TransactionTransport) |

## Client integration (after modules land)

- `AreteBuilder`: `http_url(…)`, `transport(Transport::Ws|Http)` (http skips the socket;
  view subscriptions error `WEBSOCket_DISABLED`-equivalent), `wallet(Arc<dyn WalletAdapter>)`,
  `chain(Arc<dyn ChainClient>)`, `transactions(Arc<dyn TransactionTransport>)`.
- `Arete<S>`: `chain()`, `transactions()`, `wallet()/set_wallet()`,
  `transaction(&[BuiltInstruction], TransactionOptions) -> Result<ExecutionResult>`,
  `execute(prepared, ExecuteOptions) -> Result<OperationReceipt>`.
- `Stack` trait gains `fn http_url() -> &'static str { "" }` (default derives from
  `url()` like TS `deriveHttpEndpoint`).
- `ProgramBuilder` carries `Option<ProgramReadTransport>` + program release descriptor so
  generated programs expose `accounts` readers; generated `Programs::from_builder`
  threads it.

## Codegen additions (interpreter/src/rust.rs)

- Per program: `accounts` reader accessors typed to the already-generated raw account
  structs (`programs::ore::accounts::miner(&client) -> AccountReader<Miner>`-shaped, or
  fields on `<Name>Program`), from the IDL account list.
- Program read descriptors: release/spec hashes from `SerializableStackSpec::program_specs`
  (`ProgramSpecV1::hash()`), transport `local-http` by default.
- Stack/program queries when present in the spec.
- `Stack::http_url()` emission from arete.toml/live-spec endpoints when known.
- Regenerate `examples/ore-rust`; extend the demo (chain clock + account read guarded by
  connectivity errors being non-fatal).

## Sessions semantics (port of session.ts, §11 of surface doc)

Every member gets its own `Arete` client; standalone programs become synthetic HTTP-only
stacks; program promotion by reference with first-stack-wins warning; composition mode
requires explicit chain+transactions and forbids endpoint fallback; execution host is the
first connected member; `set_wallet` fans out; `close()` disconnects all.
Rust shape: `Session::builder().stack("ore", OreStack).program("spl", …).connect().await`
returning a struct with typed accessor generics is NOT feasible without codegen — use a
runtime-keyed API (`session.stack::<OreStack>("ore")`) documented as the Rust idiom.
