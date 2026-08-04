# Python SDK Alignment — full parity with the canonical surface

Status: **implemented 2026-08-04** (all modules, codegen, CLI, and extensions landed;
see §6 for the shipped punch list and §5 for the final documented divergences).
Companion to `sdk-core-api.md` (the canonical
surface) and `sdk-api-surface.md` §2/§3 (wire + TS byte-level contracts). Goal: the
Python SDK (`python/arete-sdk`, package `arete`, dist `arete-sdk`) becomes a full
idiomatic projection of the core API — the Python sibling of the Rust phase 1+2 effort.

## 1. Current state (surveyed 2026-08-04)

`python/arete-sdk/arete/` is a **legacy protocol-v1 client** (~1,800 lines):
`AreteClient(url).subscribe("Entity/list", key)` sends `{type:"subscribe", view, key}` —
no `protocolVersion`, no `subscriptionId`, no query object, no canonical identity, no
refcounting. `Store` predates the v2 snapshot model (no snapshot staging, no
authoritative replacement, no `remove` op, LRU eviction the server no longer expects).
`auth.py` is the one modern module (publishable key / token endpoint / provider,
`hs_token`, expiry refresh) and is kept. Nothing exists for programs, chain,
transactions, reads, wallet/operations, sessions, extensions, or codegen.

**Consequence: this is a rewrite behind a mostly-new public surface, not a refactor.**
Version stays 0.x; the legacy `AreteClient` surface is not preserved (unpublished SDK,
no compatibility constraint).

## 2. Python idiom decisions (the projection rules)

- **Async-first**: `asyncio` + `websockets` + `httpx`. No sync facade (non-goal;
  revisit on demand). Streams are `AsyncIterator`s consumed with `async for`; breaking
  the loop (or `aclose()`) releases the lease.
- **Options-objects → keyword arguments** everywhere
  (`.use(take=10, filters={"state.status": "open"})`). Unknown kwargs are `TypeError`s
  — fail-closed for free.
- **Verbs keep TS names in snake_case**: `use`, `watch`, `watch_rich`, `get`,
  `get_sync`, `get_one`. Python has no keyword clash, so no Rust-style rename.
- **Namespaces are attributes**: `a4.views.ore_round.latest`, `a4.programs.ore.raw`.
  Generated bindings provide real typed attributes; dynamic stacks fall back to
  `__getattr__`.
- **snake_case payloads pass through untransformed** — the wire's entity casing is
  already Python's. u64 decimal strings → `int` (arbitrary precision). Generated
  converters (plain functions + dataclasses) replace zod; no pydantic dependency.
- **`get_sync` absent-vs-empty**: no active equivalent subscription → module-level
  `arete.UNSET` sentinel; subscribed-but-empty → `[]` / `None`. (TS uses
  `undefined` vs `null`; Python needs an explicit sentinel because `None` is taken.)
- **Errors**: exception hierarchy rooted at `AreteError`
  (`AuthError`, `SubscriptionError`, `ChainError`, `TransactionTransportError`,
  `ReadRequestError`, `InstructionError`, `OperationExecutionError`, `SessionError`).
  The four-state transaction **outcome model is data, not exceptions**:
  `execute` returns receipts; failures raise `OperationExecutionError` carrying the
  `TransactionFailureOutcome` (status ∈ `not-submitted | submitted-unknown |
  chain-failed`, plus phase), mirroring TS exactly.
- **Wallet adapter is a `Protocol`** (structural): any object with
  `async def sign_and_send(instructions, options, context) -> SendResult` works.
- **Interfaces** (`ChainClient`, `TransactionTransport`, `ProgramReadTransport`) are
  `Protocol`s with `Http*` default implementations.
- **Dependencies**: `websockets>=12`, `httpx>=0.27`, stdlib otherwise. base58 and the
  ed25519 on-curve check are vendored pure-Python (`instructions/_curve.py`) — no
  `solders`/`nacl` requirement. `from __future__ import annotations` everywhere
  (3.9 support needs no `typing_extensions`).
- **Store stays internal** (like Rust): the six view verbs are the public read surface.

## 3. Module map (strict file ownership; python/arete-sdk/arete/)

| File(s) | Contents (mirror of) | Owner |
|---|---|---|
| `wire.py` | protocol-v2 frames: envelopes, gzip, server frames, error envelope, `Seq` parsing/compare, u64-string helpers (TS `frame.ts`, `types.ts`) | core |
| `subscription.py` | canonical subscription identity (exact TS canonical JSON: field order, sorted filter keys), refcounted `SubscriptionRegistry`, stable ids across reconnect (TS `subscription.ts`) | core |
| `store.py` | v2 store: snapshot staging + authoritative replacement, patch/append deep-merge, remove/delete, server sort (TS `store.ts`/`query-store.ts`, Rust `SharedStore`) | core |
| `connection.py` | WS manager v2: connect/reconnect, ping, `refresh_auth`, resubscribe, socket issues, frame dispatch (TS `connection.ts` WS half) | core |
| `views.py` | `ViewsNamespace`, `ListViewHandle`/`StateViewHandle`: the six verbs, kwarg options, lease-releasing async generators (TS `views.ts`) | core |
| `auth.py` | existing module + targeted tokens (`program-read-binding`, `solana-gateway-binding`), LRU cap 32 keyed `(target_kind, target_id, release_hash, sorted_scopes)` | http |
| `http.py` | `HttpAuthClient`: authed JSON fetch, refresh-replay-once, predispatch marker, `derive_http_endpoint` (TS `connection.ts` HTTP half) | http |
| `chain.py` | `ChainClient` Protocol + `HttpChainClient` (9 routes) (TS `chain.ts`) | http |
| `transactions.py` | `TransactionTransport` Protocol + `HttpTransactionTransport` (6 routes, full error body) (TS `transactions.ts`) | http |
| `gateway.py` | `sgb_` bindings → chain + transaction transports, per-capability targeted tokens (TS `solana-gateway.ts`) | http |
| `amounts.py`, `spl.py` | amount parse/format/resolve; ATA derivation, token-program resolution (TS `amounts.ts`, `spl.ts`) | http |
| `instructions/` (`__init__.py`, `args.py`, `accounts.py`, `pda.py`, `_curve.py`, `handler.py`, `errors.py`) | borsh arg serializer (byte-identical), seed serializer, `derive_pda`, topo-sorted `resolve_accounts`, `InstructionHandler.build` with TS `splitParams` semantics, `ErrorMetadata` + `parse_program_error` (TS `instructions/`, Rust `instruction/`) | ix |
| `read.py`, `program_read_transport.py` | read defs, `AccountReader` (`fetch/fetch_many/exists`), query executors; v1 transport + descriptor validation (TS `read.ts`, `program-read-transport.ts`) | read |
| `wallet.py` | `WalletAdapter` Protocol, `SendOptions/SendResult/WalletExecutionContext` (TS `wallet/types.ts`) | exec |
| `operations.py` | `Prepared{Instruction,Transaction,Flow}` + composition, `execute_prepared_operation`, receipts, callbacks, outcome model, `SignerRegistry` (TS `operations.ts`, `signer-registry.ts`) | exec |
| `client.py` | `Arete`: `connect` classmethod, async context manager, views/programs/chain/transactions/wallet/`transaction`/`execute`, `processed_slot`, `wait_for_processed_slot`, http-only transport mode | exec |
| `stack.py` | stack binding model consumed by generated code: `StackDef`, `ViewDef` (state/list, key fields), `ProgramDef`, read descriptors; `program.py`-style `ProgramBuilder` runtime | exec |
| `session.py` | multi-stack session per canonical §9 (TS `session.ts`) | exec |
| `errors.py` | exception hierarchy | core |

Tests in `python/arete-sdk/tests/` mirror module names; instruction tests port the
byte-level vectors from `typescript/core/src/instructions` tests /
`rust/arete-a4-sdk` instruction tests so all three serializers are proven identical.
Tooling: `pytest` + `pytest-asyncio` (dev extra), `python -m pytest` from
`python/arete-sdk`.

## 4. Codegen (`interpreter/src/python.rs`) and CLI

Mirror of `rust.rs`, emitting a Python package next to the TS/Rust outputs:

- `<stack>/__init__.py` — `<STACK_NAME>_STACK: StackDef` binding (endpoints, views,
  programs, read descriptors) + re-exports.
- `<stack>/models.py` — entity dataclasses + converter functions
  (snake_case fields, u64→int, nested structs, patch converters).
- `<stack>/views.py` — typed view namespaces (`OreRoundViews.state/latest/…`) with
  typed state keys.
- `<stack>/programs.py` — per-program: `<Ix>Params` TypedDicts, raw builders
  (kwargs → `InstructionHandler.build`), `pdas.<name>.derive(...)` functions,
  `accounts.<name>` `AccountReader`s typed to generated account dataclasses,
  `PROGRAM_ID`, release-identity consts + `read_descriptor()`, error tables.
  Unsupported instructions are skipped with doc-comment notes (never miscompiled).
- `sdk_version` matches the current generator version; provenance hashes embedded.

CLI (`cli/src/commands/sdk.rs`): `--python` on `a4 sdk create` / `a4 install` /
`a4 sdk sync`, resolving language the same way `--rust` does. Extensions manifest
`language: "python"`: flat `.py` bundles, entry `extensions.py` by convention,
pin-validate → stage verbatim → wire via explicit imports in the generated package
`__init__.py` (manifest-driven, no source regex) → `sdk-provenance.json`. Same
registry gap as Rust: hosted bundles need the backend language dimension.

Example: `examples/ore-python` — generated package + `main.py` demoing views
streaming, offline instruction build, PDA derivation, and (connectivity-guarded)
chain clock + account read. Regeneration helper test alongside
`regenerate_ore_example` in the interpreter.

## 5. Documented divergences (idiom, never semantics)

- `arete.UNSET` sentinel for `get_sync` absence (Python has no `undefined`).
- Kwargs replace options objects; `TypeError` is the fail-closed unknown-option error.
- No display/`stringifyBigints` port (`int` is precise); no SSR/storage adapters; no
  React layer; store internal.
- Sessions use dynamic attribute access (`session.stacks.ore`) — runtime-keyed like
  Rust, spelled like TS.
- Pure-Python curve check is ~10× slower than native; PDA derivation is still
  sub-millisecond and offline-safe. Acceptable; optional native acceleration later.

Additional divergences recorded during implementation (idiom or spec-conform, never
semantics):

- **`get` subscribes**: acquires a lease, awaits snapshot resolution, releases —
  per canonical §4 ("awaits an equivalent lease's snapshot"). TS `get` only reads an
  already-active subscription; the Python shape is the more useful projection and
  `get_sync` covers the read-existing case.
- **Raw builder params use verbatim IDL wire-name keys** (camelCase arg names as in the
  IDL), matching the TS `raw` layer exactly; generated `<Ix>Params` TypedDicts spell the
  wire names. Namespace keys (`raw.<ix>`, `pdas.<name>`, `accounts.<name>`) are
  snake_case per the idiom matrix.
- `Update` names its discriminator `op` (`upsert|patch|remove|delete` — the wire
  taxonomy); `RichUpdate.type` keeps `created|updated|removed|deleted`.
- All four client envelopes carry `protocolVersion: 2` per the spec; TS currently sends
  bare `ping`/`refresh_auth` (documented byte divergence from TS, conformance with the
  protocol doc).
- Instruction-runtime strictness follows Rust where TS is loose (integer range checks,
  no silent truncation); byte output is identical for all valid inputs. On-curve edge
  semantics mirror `@noble/ed25519`.
- Codegen: composite/unsupported state keys degrade to a positional scalar key with a
  `# [arete codegen]` note (never miscompiled, never blocks generation); `program_reads`
  on a `StackDef` is all-or-nothing (partial specs degrade to `{}` with per-program
  omission comments); u128/i128 params are `int`.
- CLI: default Python output dir suffix is `-py` (avoids no-flag sync collision with the
  Rust `-stack` default); `-p/--package-name` doubles as the generated distribution name
  (documented in help; Rust uses a dedicated `--crate-name`).
- **`connect_http_url` falls back to `StackDef.endpoints.http`** when the `http_url`
  option is absent (TS uses the explicit option only; Rust prefers explicit > generated >
  derived-from-ws). Python sits between the two: strictly looser than TS, so it only
  turns previously-erroring configs into working ones, and it means a generated stack
  carrying `endpoints.http` supports local-http program reads with no extra option.
- **`get`/`get_one` are bounded** by a default snapshot timeout
  (`DEFAULT_INITIAL_DATA_TIMEOUT = 5.0`, per-call `timeout=`, `None` opts out; client-level
  `Arete.connect(initial_data_timeout=…)`), raising `InitialDataTimeoutError`. Rust
  `initial_data_timeout` semantics. TS returns `[]` immediately because its `get` never
  subscribes; Python's subscribing `get` must not hang. The timeout is not part of the
  query, so subscription identity/dedup is unchanged.
- **Failure classification is data, not exception classes.** The TS
  `normalizeTransactionError` ladder is ported in full (deterministic program-error match
  → carried outcome → wallet rejection → non-deterministic match → recovered signature ⇒
  `submitted-unknown`/`confirmation` → fallback), but TS's `InstructionError` /
  `TransactionExecutionError` class-identity steps collapse into "the carried
  `TransactionFailureOutcome` wins", plus a rule TS lacks: a resolved `program_error` is
  never downgraded to a synthetic `CustomError<code>` when `errors` is empty.
- **`wallet=` is the reserved signer-fallback option** on raw builders
  (`RawInstruction.build` and generated builders), matching TS `BuildOptions.wallet`.
  Spelling it `payer=` would shadow the IDL account name `payer`, which must stay
  reachable as an account override. Reserved set: `wallet`, `accounts`,
  `remaining_accounts`; audited against every IDL in `stacks/**` — `payer` was the only
  collision, and none of the reserved names appear as account or arg names.
- **Capture/event envelopes are exposed, not unwrapped** (TS parity): a `#[capture]`
  field is `Optional[CaptureWrapper[T]]` carrying `timestamp` / `account_address` /
  `slot` / `signature` alongside `data`, since that provenance is unrecoverable
  elsewhere. Attribute spelling stays snake_case where TS transforms to camelCase.
- **Strict/loose converter split** mirrors TS's Schema vs PatchSchema: IDL struct
  converters (`*_from_wire` for resolved account types) require non-optional fields and
  raise, which is what makes `read.py`'s `SCHEMA_VALIDATION` guard reachable; entity /
  section converters and every `*_patch_from_wire` stay loose because wire payloads are
  legitimately partial.
- **Collation**: `subscription.collation_key` / `locale_compare` implement JS
  `localeCompare` (canonical §2 ordering rule) in pure stdlib, used for filter-key sorting
  and store ordering. Verified byte-for-byte against Node ICU over the full printable-ASCII
  block (the real input domain: base58 keys, dotted filter paths), case-as-tertiary,
  accents-as-secondary, canonical equivalence, and ignorable control characters.
  Approximated (sign may differ from ICU, none reachable from wire data): non-ASCII
  punctuation/symbols and non-Latin scripts order by code point within their band,
  non-ASCII digits sort after ASCII digits, undecomposable Latin letters
  (`ð þ ı ŋ`) fall to the letter band's tail, and contractions beyond a small fold table
  (`ß æ œ ø đ ł ŧ`) are unmodelled. `tests/test_collation.py` re-runs the whole table
  against a live `node` when present, so the captured values cannot rot.
- **Wallet-rejection heuristic scope**: TS runs its rejection regex over every thrown
  value; Python skips it for `WalletError` (adapters own their classification, so an
  outcome-less `WalletError` is `not-submitted`/`send` rather than `/wallet`) and applies
  the full heuristic to all other exceptions. Both agree on the status; only the phase
  differs.

## 6. Punch list — implemented 2026-08-04

1. ✅ Core: `wire.py`, `subscription.py`, `store.py`, `connection.py`, `views.py`,
   `errors.py` + 117 tests (canonical-identity byte-equality vs TS, snapshot authority,
   remove-vs-delete, lease refcounting, hermetic loopback connection tests).
2. ✅ Instruction runtime + 91 tests porting the TS/Rust byte vectors plus 12 golden
   PDA address+bump fixtures (proving base58/sha256/on-curve byte-identical).
3. ✅ HTTP: `http.py`, `auth.py` targeted tokens, `chain.py`, `transactions.py`,
   `gateway.py`, `amounts.py`, `spl.py` + 88 tests (also migrated legacy `auth.py`
   off aiohttp, which was never a declared dependency).
4. ✅ Reads: `read.py`, `program_read_transport.py` + 61 tests driven by the embedded
   `program-read-contract-v1` fixture and the Rust descriptor-validation table.
5. ✅ Execution: `wallet.py`, `operations.py`, `stack.py`, `extensions.py`,
   `client.py`, `session.py`, curated `arete` exports — full suite 492 tests.
6. ✅ Codegen `interpreter/src/python.rs` (16 tests; interpreter suite 147) +
   CLI `--python` on create/install/sync, extensions `language: "python"`, `python-ore`
   template, provenance (15 new CLI tests; suite 126) + regenerated
   `examples/ore-python` with an offline smoke test.
7. ✅ Package: pyproject deps/extras (0.4.0), README rewrite, `__init__.py` exports.
   ☐ Follow-up: `docs/src/content/docs/sdks/python.mdx` public page (docs team);
   hosted registry `sdk_extension_contents` language dimension (backend — same gap
   as Rust).
