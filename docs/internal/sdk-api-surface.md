# SDK API Surface & Wire Formats — TypeScript ⇄ Rust Alignment

> The language-neutral canonical surface now lives in `sdk-core-api.md`; this document
> remains the recorded TypeScript surface (§3), wire formats (§2), and the TS ⇄ Rust
> alignment history. Python's projection is `sdk-python-alignment.md`.

Status: reference + alignment spec. Sources of truth surveyed 2026-08-04:
`typescript/core` (`@usearete/sdk` 0.4.1), `typescript/react` (`@usearete/react` 0.4.1),
`rust/arete-a4-sdk` (0.4.1), `docs/websocket-v2-protocol.md`, generated output in
`examples/ore-typescript/src/generated` and `examples/ore-rust/src/generated`.

This document has three jobs:

1. Record the **public API surface** of the TypeScript core and React SDKs.
2. Record the **wire formats** both SDKs speak.
3. Define the **Rust alignment**: what the Rust SDK must expose so that the three core
   surfaces — **views**, **program SDK**, and **stack binding** — feel the same as
   TypeScript while staying idiomatic Rust.

---

## 1. Shared concepts

| Concept | Definition |
|---|---|
| **Stack** | A deployed set of entities + views + bundled program SDKs, reachable at `endpoints.ws` / `endpoints.http`. Generated code binds a stack to a client. |
| **View** | A named server query surface `"<Entity>/<view>"`. `state` views are keyed (one entity per key); `list` views (incl. derived views like `latest`) are ordered collections. |
| **Program SDK** | Generated client for a Solana program bundled with the stack: raw instruction builders, PDA factories, account readers, and semantic operations. |
| **Update taxonomy** | `upsert` (full entity entered/changed in window), `patch` (partial merge), `remove` (left *this query's* window), `delete` (deleted from the source view globally). |
| **Subscription identity** | The canonical JSON of `{query, snapshot}`. Equivalent queries share one wire subscription; leases are reference-counted. |
| **Replay cursor** | `"{epoch}:{offset}"`, carried on every update from a journal-backed `append` view and sent back as `query.after`. `epoch` identifies one tape lifetime (it comes from the ack's `replayWindow`), `offset` the position within it. `seq` is **not** a cursor: its second component is the transaction index, so events decoded from one transaction share it. Absent on `state`/`list` views, which project membership and have no per-event identity. |
| **Gap** | Any point where a consumer's sequence of cursors would be discontinuous: a refused cursor (`cursor-expired`, `cursor-epoch-changed`, `cursor-unknown`, `invalid-cursor`), a server-side discontinuity (`replay-gap`), server fan-out lag (`replay-lagged`), or the SDK's own bounded queue evicting a cursor-bearing update. Every SDK ends the stream at the last delivered cursor rather than continuing past it; a dropped update on a cursorless view is not a gap. |

---

## 2. Wire formats

### 2.1 WebSocket protocol v2 (canonical spec: `docs/websocket-v2-protocol.md`)

All frames are JSON; the server may send gzip-compressed binary (magic `1f 8b`). Every
message carries `protocolVersion: 2`. Wire JSON is **camelCase envelope fields** with
**snake_case entity payloads** (`round_id`, `_seq`).

Client → server:

```json
{"type":"subscribe","protocolVersion":2,"subscriptionId":"<opaque ≤128B>",
 "query":{"view":"Order/list","key":"…","partition":"…",
          "filters":{"state.status":"open"},"take":10,"skip":0,
          "after":"0f8c2b31-…-1d2c3e4f5a6b:4211","snapshotLimit":100},
 "snapshot":{"enabled":true}}
{"type":"unsubscribe","protocolVersion":2,"subscriptionId":"…"}
{"type":"ping"}
{"type":"refresh_auth","token":"<jwt>"}
```

Only `query.view` is required (state views also need `key`). Canonical field order:
`view, key, partition, filters, take, skip, after, snapshotLimit`; filter keys sorted.
Unknown fields are rejected.

Server → client (`op`-tagged):

- `subscribed` — ack with the **effective** query, `mode ∈ state|append|list`, optional
  `sort: {field: [path…], order: asc|desc}`.
- `snapshot` — batches sharing `snapshotId`; `authoritative: true` replaces membership on
  the final `complete: true` batch, `authoritative: false` (from `after` cursors) merges.
- `upsert | patch | remove | delete` — live frames with `key`, `data`, optional
  `seq: "<slot>:<index>"`, and `append: [dot.paths]` whose arrays concatenate on patch.
  `upsert` is always a whole entity and arrives whenever a key becomes a member as far
  as the server knows; a `patch` for a key the client does not hold is discarded
  (frames with `offset` — tape records — excepted). See the protocol doc, "Partial
  entities".
- `unsubscribed` — ack.
- error envelope: `{"type":"error", "subscriptionId": string|null, "code": "<kebab>",
  "message", "retryable", "retry_after"?, "suggested_action"?, "docs_url"?, "fatal"}`.

`u64` values are decimal strings on the wire. `seq` slots compare as integers, index
lexicographically.

### 2.2 HTTP surfaces (TypeScript today; Rust roadmap)

- **Program reads** (`program-read-http/v1`):
  `GET /v1/releases/<releaseHash>/accounts/<Account>/<address>` (`null` when missing),
  `…/exists` → `{"exists":bool}`, `POST …/accounts/<Account>` `{"addresses":[…]}` →
  `{"items":[{address,status:"ok",value}|{address,status:"missing"}|{address,status:"error",error:{code}}]}`.
- **Chain routes**: `/chain/exists|lamports|rent-exemption|clock|accounts|mints|token-accounts`
  (GET) and `/chain/native-balance|balances` (POST, u64s as decimal strings).
  Managed contextual reads and owner enumeration are additive `/chain/v1/*` POST routes;
  see [managed Solana v1](managed-solana/README.md) for the frozen contracts.
- **Transaction relay**: `POST <base>/transactions/v1/{latest-blockhash,fee,simulate,send,signature-status,block-height}`.
- **Auth**: `POST <tokenEndpoint>` `{"websocket_url": "...", "scopes": ["read"]}` (+
  `Authorization: Bearer <publishableKey>`) → `{"token","expires_at"}`; WS token in
  `?hs_token=` (default) or `Authorization: Bearer` upgrade header. Refresh at `exp − 60s`.

---

## 3. TypeScript core surface (`@usearete/sdk`)

### 3.1 Client

```ts
const a4 = await Arete.connect(ORE_STREAM_STACK, { url?, httpUrl?, transport?, auth?,
  wallet?, storage?, programs?, execution?, autoConnect?, autoReconnect?, … });
// a4: ConnectedArete<TStack> = Arete<TStack> & { addresses?, constants?, defaults?, math?, read?, flows? }
```

Instance surface: `views`, `programs`, `queries`, `chain`, `transactions`, `wallet` /
`setWallet`, `transaction(instructions, opts?)`, `execute(prepared, opts?)`,
`inspectOperation`, `connect/disconnect/isConnected/connectionState`,
`onConnectionStateChange/onFrame/onSocketIssue`, `store`, `processedSlot`,
`waitForProcessedSlot(slot)`. Multi-stack: `createSession({stacks, programs}, opts)` →
`session.stacks.<k>`, `session.programs.<k>`, `session.execute(...)`.

### 3.2 Views

Views are **property-accessed** typed namespaces; each method takes an options object:

```ts
a4.views.OreRound.latest.use({ take: 10, skip: 0, filters, partition, after,
                               snapshotLimit, withSnapshot, schema })  // AsyncIterable<T>
a4.views.OreRound.latest.watch(opts)      // AsyncIterable<Update<T>>
a4.views.OreRound.latest.watchRich(opts)  // AsyncIterable<RichUpdate<T>>
await a4.views.OreRound.latest.get(opts)  // T[]   (reads an existing lease's snapshot)
a4.views.OreRound.latest.getSync(opts)    // T[] | undefined

a4.views.OreRound.state.use({ roundId: 42n }, opts)   // typed key via keyFields
await a4.views.OreRound.state.get({ roundId: 42n })   // T | null
```

Semantics: `use` = merged entities (patches applied, removes/deletes filtered);
`watch` = raw `Update`; `watchRich` = before/after diffs; breaking the `for await` loop
releases the refcounted lease. `getSync` returns `undefined` when no equivalent active
subscription exists, `null`/`[]` when subscribed-but-absent.

### 3.3 Program SDK

```ts
const ore = a4.programs.ore;
ore.raw.deploy.build(params)                 // pure → BuiltInstruction (IDL-shaped params)
ore.instructions.deploy.prepare(input)       // semantic → PreparedInstruction
ore.transactions.mining.deployWithCheckpoint.prepare(input)  // → PreparedTransaction
ore.flows.<path>.prepare(input)              // → PreparedFlow (multi-tx)
ore.accounts.Miner.fetch(address) / fetchMany / exists       // HTTP program reads
ore.pdas.miner.deriveSync({ accounts: { authority } })       // PDA factories
ore.addresses / constants / defaults / math                  // extension namespaces
await a4.execute(prepared, { onTransactionStart, … })        // → receipt with signatures
```

Building blocks: `InstructionHandler` (programId + discriminator + `AccountMeta[]` +
`ArgSchema[]` + `ErrorMetadata[]`), `buildInstruction(handler, params)` — pure, resolves
accounts (`signer | known | pda | userProvided`, topo-sorted PDA seeds
`literal | bytes | argRef | accountRef`), borsh-serializes args
(`u8…u128, i8…i128, f32/f64, bool, string, pubkey, bytes, vec, option, array, hashMap,
struct, enum`), and fails closed on unknown params / missing non-option args.
`PreparedInstruction/Transaction/Flow` carry `name`, `artifacts`,
`requiredSignerAddresses`, `errors` and compose (`createPreparedTransaction({operations})`).

### 3.4 Stack binding & extensions

Generated core stack = `as const` object: `name`, `endpoints`, `views` (phantom-typed
`stateView<T, TKey>()`/`listView<T>()` defs), `schemas`/`patchSchemas` (zod, snake_case →
camelCase transform, u64→bigint), `programs` (with `rawInstructions`, `pdas`, `accounts`,
provenance hashes), `programReads` descriptors. Extensions attach via
`extendStack`/`extendProgram` and surface on the connected client as `read`, `flows`,
`addresses`, `constants`, `defaults`, `math`; operations attach via
`createOperations(context)` with access to the fully connected program.

Program definitions may carry `packageReleaseHash`; `createSession`, `withPrograms`,
`ConnectOptions.programs` and `useArete(…, {programs})` match programs by it (canonical
§9 "Program identity") and report `PROGRAM_KEY_CONFLICT` instead of warning. The
package exports `EXTENSION_API_VERSION` and records it as `arete.extensionApi` in
`package.json` (canonical §9 "Extension API contract").

### 3.5 React (`@usearete/react`)

Same namespaces, hook-terminal: `useArete(stack)` → `arete.views.X.state.use(key, opts)` /
`.list.use(params, opts)` / `.useOne(...)` (status-discriminated results:
`disabled|connecting|subscribing|ready|error`, `isPending/isReady/isEmpty/isRefreshing`),
`arete.programs.<p>.{raw|instructions|transactions|flows}.<path>.useMutation()`
(phase machine incl. reconciliation against `waitForProcessedSlot`),
`arete.read.<name>.use(...)`, `summarizeStatuses`, `createAreteReact(stack)` for bound
`useOre()` hooks. No Suspense; discriminated status unions everywhere.

---

## 4. Rust SDK surface (current, 0.4.x)

```rust
let a4 = Arete::<OreStreamStack>::builder().api_key(KEY).connect().await?;
a4.views.ore_round.latest()          // ViewHandle<OreRound>
    .watch()                         // WatchBuilder<T>: impl Stream<Item = Update<T>>
    .filter("state.status", "open").take(10).skip(20)
    .partition("p").after(cursor).with_snapshot(false).with_snapshot_limit(100);
a4.views.ore_round.latest().listen() // UseBuilder<T>: impl Stream<Item = T>
a4.views.ore_round.latest().get().await          // Vec<T>
a4.views.ore_round.state().get("key").await      // Option<T>
```

Present and aligned: protocol v2 wire structs (`Subscription`, `SubscriptionQuery`,
`ServerFrame`, canonical identity + refcounted `SubscriptionRegistry`, stable IDs across
reconnect), `Update`/`RichUpdate` enums, lazy stream builders, `SharedStore` with
snapshot staging/authoritative replacement, auth (publishable key / token endpoint /
provider, `hs_token` or Bearer, JWT expiry refresh), reconnect + `SocketIssue`s,
`serde_utils` string-or-number integer deserializers.

Generated code (interpreter/src/rust.rs): `OreStreamStack` (`Stack` impl), per-entity
`*EntityViews` with `state()`, `list()`, derived views. **Nothing else.**

### Gaps vs TypeScript (from the survey)

1. **No program SDK at all** — no instruction building, PDAs, borsh, accounts, programs
   namespace, prepared operations, errors metadata.
2. **State views take no query options** (`listen/watch/watch_rich(key)` return raw
   streams; no `with_snapshot/after/partition/…`).
3. No single-item conveniences (`get_one`), no error-carrying fetch (`get` swallows
   failures into `Vec::new()`/`None`).
4. `ViewHandle::get` ignores query options; `get_sync` only matches the default query.
5. Codegen emits empty `url()` for deployed stacks in module mode; hard-codes
   `sdk_version "0.3"`.
6. Misc: `watch_keys` has no `listen_keys` sibling; `new_lazy` dead parameter; README and
   crate docs describe a removed API.

---

## 5. Rust alignment design

Guiding rule: **same nouns, same shape, native idiom**. TS options-objects become Rust
builder chains (already the established pattern); TS `AsyncIterable` becomes
`impl Stream`; TS `Promise<T | null>` becomes `async -> Result<Option<T>>` where errors
are real; phantom generic `ViewDef`s become generated typed structs.

### 5.1 Surface mapping (the contract)

| TypeScript | Rust (target) |
|---|---|
| `a4.views.OreRound.latest.use(opts)` | `a4.views.ore_round.latest().listen()` + builder methods |
| `….watch(opts)` / `.watchRich(opts)` | `….watch()` / `….watch_rich()` + builder methods |
| `await ….get(opts)` / `.getSync(opts)` | `await ….get()` / `….get_sync()` (+ builders honoring options) |
| `views.X.state.use(key, opts)` | `views.x.state().listen(key)` **returning a builder** with the same option set |
| `views.X.list.useOne(params)` (React) / `get` first | `views.x.list().get_one().await -> Option<T>` |
| `a4.programs.ore.raw.deploy.build(params)` | `a4.programs.ore.deploy(params) -> Result<BuiltInstruction>` (typed params struct) |
| `handler.build` low-level | `InstructionHandler::build(&self, params: Value, opts) -> Result<BuiltInstruction>` |
| `ore.pdas.miner.deriveSync({accounts})` | `ore::pdas::miner(authority: &str) -> Result<Pubkey>` (generated typed fns) |
| `PROGRAM/stack extension namespaces` | future: generated inherent impls / feature-gated modules |
| `BuiltInstruction {programId, keys, data}` | `BuiltInstruction { program_id: Pubkey, accounts: Vec<BuiltAccountMeta>, data: Vec<u8> }` (+ `From` into `solana_sdk::Instruction` shape) |
| `ErrorMetadata` + `parseInstructionError` | `ErrorMetadata` + `parse_program_error(code)` |
| fail-closed unknown params | serde `deny_unknown_fields` on generated param structs |

### 5.2 New SDK modules (`rust/arete-a4-sdk`)

- `instruction/` — runtime the generated code targets:
  - `BuiltAccountMeta { pubkey, is_signer, is_writable }`, `BuiltInstruction`.
  - `AccountMeta { name, is_signer, is_writable, resolution, is_optional }` with
    `AccountResolution::{Signer, Known(address), Pda(PdaConfig), UserProvided}`.
  - `PdaConfig { program_id: Option<String>, seeds: Vec<PdaSeed> }`,
    `PdaSeed::{Literal(String), Bytes(Vec<u8>), ArgRef{arg, arg_type}, AccountRef(name)}`.
  - `ArgType` schema enum mirroring the TS borsh serializer, plus
    `serialize_args(discriminator, args: &Map, schema) -> Result<Vec<u8>>` implementing
    the same borsh layout (fail-closed: missing non-option arg or unknown param errors).
  - `derive_pda(seeds, program_id)` via `solana-pubkey` `find_program_address`.
  - `resolve_accounts(metas, args, overrides, payer) -> Result<Vec<ResolvedAccount>>`
    with topo-sorted PDA resolution, exactly mirroring TS `resolveAccounts`.
  - `InstructionHandler { program_id, discriminator, accounts, args, errors }` +
    `build(&self, params: serde_json::Value, opts: BuildOptions) -> Result<BuiltInstruction>`.
    Params use IDL wire shape (same as TS `raw`): account-name keys override addresses,
    arg-name keys serialize, `resolve` map feeds PDA-only seeds.
  - `ErrorMetadata { code, name, msg }` + lookup.
- `program.rs` — `Programs` trait (`from_builder`, like `Views`) so the client can expose
  `pub programs: S::Programs`; `()` implements both `Views` and `Programs` for empty
  stacks.
- `view.rs` additions — `StateView::{listen,watch,watch_rich}` return the existing
  builders (with the key applied) so keyed subscriptions accept
  `with_snapshot/after/partition/filter/snapshot_limit`; `ViewHandle::get_one()`.

### 5.3 Stack trait (breaking, coordinated with codegen)

```rust
pub trait Stack: Sized + Send + Sync + 'static {
    type Views: Views;
    type Programs: Programs;      // NEW; `()` for program-less stacks
    fn name() -> &'static str;
    fn url() -> &'static str;
}
```

### 5.4 Generated Rust (interpreter/src/rust.rs)

Per program in the spec (`SerializableStackSpec::instructions` grouped by `program_id`,
names from the IDL snapshot):

```rust
pub mod programs {
    pub mod ore {
        pub struct DeployParams { pub signer: String, pub round_id: u64, … }  // typed, deny_unknown_fields
        pub fn deploy(params: DeployParams) -> Result<BuiltInstruction, InstructionError>;
        pub fn deploy_handler() -> &'static InstructionHandler;               // escape hatch
        pub mod pdas { pub fn miner(authority: &str) -> Result<Pubkey, …>; }
        pub const PROGRAM_ID: &str = "oreV3…";
    }
}
pub struct OreStreamStackPrograms { pub ore: OreProgram, … }   // bound on the client
impl OreProgram { pub fn deploy(&self, params: DeployParams) -> Result<BuiltInstruction, …> }
```

`a4.programs.ore.deploy(params)` therefore mirrors `a4.programs.ore.raw.deploy.build(params)`
— the Rust surface starts at the raw layer (semantic operations/`prepare` come with the
execution layer later; Rust has no wallet/transaction transport yet).

### 5.5 Phase 2 — full functional parity (implemented 2026-08-04)

Everything deferred from the first pass now exists in Rust (see
`sdk-rust-alignment-phase2.md` for the design and module map):

- `http` — shared token machinery (`HttpAuthClient`/`TokenSource`, targeted tokens with
  LRU cache, refresh-replay-once, predispatch-marker gating).
- `chain` — `ChainClient` trait + `HttpChainClient` over all nine `/chain/*` routes.
- `transactions` — `TransactionTransport` trait + `HttpTransactionTransport` over the six
  `/transactions/v1/*` routes with the full TS error body.
- `wallet` + `operations` — `WalletAdapter`, prepared instruction/transaction/flow
  values with composition helpers, receipts, fail-closed signer validation,
  `SignerRegistry`, and the four-state transaction outcome model.
- `read` + `program_read_transport` — program-read-http/v1 (fetch/fetchMany/exists),
  descriptor validation (release hashes, `prb_` bindings), typed `AccountReader<T>`,
  stack/program query executors.
- Client integration — `AreteBuilder::{http_url, transport(Http|WebSocket), wallet,
  chain, transactions, signer_registry}`, `Arete::{chain, transactions, wallet/set_wallet,
  transaction, execute}`, HTTP-only mode failing view subscriptions fast, and
  `Stack::http_url()` (empty → derived from the ws URL — documented divergence).
- `session` — multi-stack sessions with typed runtime-keyed accessors
  (`session.stack::<OreStack>("ore")`), wallet fan-out, first-member execution host,
  composition-mode chain/transaction overrides.
- `gateway` — hosted Solana gateway bindings (`sgb_`) → chain + transaction transports
  with per-capability targeted tokens.
- Codegen — generated programs carry the runtime (`ProgramBuilder`), release-identity
  consts + `read_descriptor()`, and typed `*_accounts()` readers.

Remaining intentional divergences are documented in the module docs (runtime-keyed
sessions instead of type-level maps, wallet-classified failures instead of error
duck-typing, sync observation callbacks, `serde_json::Value` artifacts). Not ported by
design: `stringifyBigints`/display (Rust types are precise), SSR helpers, storage
adapters (Rust's `SharedStore` remains internal), and the React layer (browser-only).

---

## 6. Punch list — implemented 2026-08-04

1. ✅ `arete_sdk::instruction` module (schema-driven borsh serializer, seed serializer,
   PDA derivation via `solana-pubkey`, topo-sorted account resolver, `InstructionHandler`
   with TS `splitParams` semantics) — 60 unit tests port the TS byte-level vectors.
2. ✅ `Programs` trait + `ProgramBuilder`; `Stack::Programs` associated type (`()` for
   program-less stacks); `Arete::programs` public field.
3. ✅ `StateView::{listen, watch, watch_rich}` now return the option builders (keyed
   subscriptions accept `with_snapshot/after/partition/filter/take/skip/snapshot_limit`);
   `ViewHandle::get_one()`; `ViewHandle::listen_keys()`.
4. ✅ Codegen (`interpreter/src/rust.rs`): generated `programs.rs` with per-program
   modules (typed `<Ix>Params` structs, `<ix>()` builders, `<ix>_handler()` escape hatch,
   `pdas::` fns, `PROGRAM_ID`), `<Stack>StackPrograms` wiring, `type Programs` in both
   Stack templates; generated `sdk_version` bumped to `0.4`; unsupported instructions are
   skipped with doc-comment notes (never miscompiled). URL emission verified — the stale
   empty `url()` in the checked-in example predated existing plumbing.
5. ✅ `examples/ore-rust` regenerated (correct `url()`, `programs.rs` for ore + entropy)
   with a `[patch.crates-io]` override onto the local SDK; `main.rs` demos offline
   instruction building + `a4.programs.ore.deploy(...)`. Regeneration helper:
   `cargo test -p arete-interpreter regenerate_ore_example -- --ignored`.

### String ordering — fixed 2026-08-04

Rust used byte/code-point order where the canonical ordering rule (`sdk-core-api.md` §2)
requires JS `localeCompare` semantics — the same defect Python carried until the same day.
`rust/arete-a4-sdk/src/collation.rs` now provides `collation_key` / `locale_compare`
(`CollationKey`'s derived `Ord` is level-by-level, so `sort_by_key` ≡ `sort_by`), ported
from the Python reference with an identical fidelity envelope: cross-checked over 8,465
pairs with **0 mismatches vs Python**, and the same 34 documented deviations from Node ICU
(all `ð`/`ı`/`ŋ`, the recorded "undecomposable Latin, no fold entry" approximation).
Dependencies are `unicode-normalization` + `unicode-properties`, both already resolved in
the workspace lockfile; no ICU crate.

- `subscription.rs` — `filters` stays a public `BTreeMap` (preserving
  insertion-order-independent identity); a `serialize_with` hook emits entries in
  collation order, which fixes canonical identity and wire key order together. A
  `CanonicalValue` wrapper extends the same ordering to nested object keys inside filter
  values, matching TS `canonicalJsonValue`.
- `store.rs` — collation applies to the string sort-field branch, the `to_string()`
  fallback, and the key tie-break (keys are decorated with their `CollationKey` once per
  sort rather than recompared).

**Second divergence found and fixed in the same expression**: Rust applied the `desc`
negation to the key tie-break, so descending lists tie-broke in reverse. TS
(`query-store.ts:387`) applies the tie-break *after* the negation, making it always
ascending; Python already matched TS. Rust now does too.

Residual, documented: for two distinct keys that *collate equal* (e.g. NFC vs NFD
spellings used as separate filter paths), Rust's stable sort falls back to BTreeMap byte
order while TS/Python fall back to insertion order — Rust is strictly more deterministic;
unreachable from real filter paths.

### Stale-sequence guard — added to Rust 2026-08-04

The Rust store had **no** duplicate/stale-sequence guard for live `upsert`/`patch`
frames: `apply_frame` destructured `ServerFrame::Upsert`/`::Patch` with `..`, discarding
the `seq` the frame carries, and `apply_live` wrote storage unconditionally — so an
older-sequence frame could overwrite newer cached data, which the TS and Python suites
both forbid. Ported from `typescript/core/src/frame-processor.ts:623-712` and
`python/arete-sdk/arete/store.py:341-391`: `frame::compare_seq` (slot compared as a digit
string, so long slots cannot overflow a `u64` parse; only the second `:` segment is the
index), per-key sequences in `ViewData::seqs` (a sibling map — **not** injected into the
`serde_json::Value`, which would leak into user data and break typed deserialization),
and the guard itself. A stale frame still grants query membership and emits an update
carrying the **cached** value, exactly as TS does; snapshot rows still bypass the guard,
which is the authoritative-replacement semantics.

**Unsequenced upserts — reconciled across all three SDKs 2026-08-04.** On an upsert with
no sequence (neither `frame.seq` nor a payload `_seq`), TypeScript used to drop the
entity's tracked sequence — a side effect of carrying `__seq` on the entity object it
replaces, not a designed behavior. That disarmed the guard for that key until the next
sequenced frame, letting a later older frame overwrite newer data. The giveaway was
internal inconsistency: the *patch* branch of the same function already fell back to
`getInternalSeq(existing)`, while *upsert* did not. Reachable in practice — the server's
`seq` is `Option<String>` at every layer (`rust/arete-server/src/websocket/frame.rs:93`).

All three now retain the sequence: `frame-processor.ts` upsert gained
`?? this.getInternalSeq(previousValue)`, Python already did this
(`store.py::_set_entity` writes only when `seq is not None`), and Rust matches
(`ViewData::set_seq`). Regression tests in all three drive `seq 50:…09` → unsequenced →
`seq 50:…01` and assert the unsequenced value survives; each fails if its fallback is
removed.

### Frame versions — all three SDKs

The `seq` guard above dropped real updates. `seq` is `slot:index`, but the index is the
transaction index for instruction updates and the account write version for account
updates, and every update decoded from one transaction shares it. So an instruction
patch that followed an account patch in the same slot sorted older, and a transaction's
second patch was an exact duplicate. Replaying one live ORE round through the TypeScript
SDK applied 44 patches and discarded 41 that carried new data.

The equal-`seq` rule could not simply be loosened: queries on one view share storage,
and it is what applies a patch routed to two subscriptions once (the conformance test
"normalizes one sequenced patch once while routing it to multiple queries"). The server
now stamps `_version` (`{epoch}:{counter}`, its merge order) into frame data, and all
three SDKs guard on it when present, comparing counters only within an epoch, and fall
back to `seq` without one. The helpers are `isStaleVersion` (`frame-processor.ts`),
`is_stale_version` (`python/arete-sdk/arete/wire.py`) and `frame::is_stale_version`
(Rust); tracked versions live beside tracked sequences and follow their lifecycle.

### Resolved-field typing — fixed 2026-08-04

`RustCompiler::field_type_to_rust` never consulted `field.resolved_type` (the
`resolved_name_map` was plumbed in only to *name* emitted structs), so every
resolved-struct field fell through to `serde_json::Value` and the generated `Board` /
`Treasury` / `Miner` / `Automation` structs were referenced by nothing. `EventWrapper<T>`
was emitted and reserved but likewise unreachable — dead code, exactly as in Python
before its fix.

Now fixed: `capture_field_targets` + `wrapper_kind_for` ported from
`interpreter/src/python.rs`, so `#[capture]` fields emit `CaptureWrapper<T>` (exposing
`timestamp` / `account_address` / `slot` / `signature` beside `data`, matching TS and
Python), `is_event` / array-of-instruction fields emit `EventWrapper<T>`, and plain
resolved fields emit the bare struct. `CaptureWrapper` joined the reserved type-name set;
both wrapper literals collapsed into one `WRAPPER_TYPES` const; wrapper `slot` moved to
`Option<u64>` with the string-or-number deserializer (the wire sends u64 as a decimal
string, which the previous dead `Option<f64>` would have rejected). `regenerate_ore_example`
was also not writing `types.rs`, making regeneration a silent no-op for this file — fixed.

Two adjacent typing gaps closed in the same pass:

- **Scalar arrays** — `BaseType::Array` mapped unconditionally to `Vec<serde_json::Value>`;
  `integer_kind` / `inner_type` were never read. Now `rust_scalar_field_shape` +
  `rust_scalar_array_element` (ports of the Python fix) derive the type *and* the
  `serde_utils` deserializer from one call, so a typed `Vec<u64>` can't be paired with a
  scalar deserializer. Reuses the existing `deserialize_option_vec_*` matrix; element
  kinds widen exactly as scalar integers already do (so `Vec<u8>` → `Vec<u64>`; a
  byte-accurate `Vec<u8>` would need a new SDK deserializer and a policy change).
- **Builtin resolver types** — `SlotHashBytes` / `TokenMetadata` now emit as structs.
  This needed the TS `add_unmapped_fields` override ported too: a computed field keeps the
  user's declared type in the section (`ResolvedSlotHash`), and only
  `spec.field_mappings["results.expires_at_slot_hash"]` records the real resolver output
  type. `KeccakRngValue` is deliberately **not** ported from TS — TS models it as `string`
  only because a u64 rides the wire as a decimal string; typing it that way in Rust would
  regress `results.rng` from `u64` to `String`. The builtin branch is therefore gated on a
  local struct table, not on the resolver registry alone.

Note for whoever owns it: `examples/ore-typescript/src/generated/ore-stack-core.ts`
predates commit `ff44e714` and is due a regeneration pass.

Still open (tracked as roadmap, see §5.5): execution layer (wallet/transaction/receipts),
HTTP program reads + chain client, sessions, stack runtime extensions (`read`/`flows`/
`math`/`addresses`) in Rust, semantic `prepare()` operations, and refreshing the public
`docs/src/content/docs/sdks/rust.mdx` plus the `arete-streams` and `arete-programs` skill references.

### No partial entities — server and all three SDKs, 2026-09-25

A `patch` for a key the client does not hold is not an entity (canonical §5; protocol
doc "Partial entities"). Server: only keys a frame actually carried count as held, so
after a `snapshotLimit`-truncated or disabled snapshot a key's first change is a full
`upsert`, and keys never sent get no `remove`/`delete` (state, list, derived and
coalesced paths); every `subscribed` ack says so with `wholeEntities: true`.
Replayable append views keep delivering records verbatim; clients exempt frames with
`offset`.

The server's bounded entity cache refuses a source patch for a key it evicted instead
of storing the fragment — the cause of partial, id-less entities at the top of
`OreRound/latest`, together with derived windows ranking a missing sort value first
(now last in both directions). Refusing alone would freeze out an active key for good,
so the projector asks the VM for the whole entity (`WholeEntityRequests`, linked when
the generated runtime calls `snapshot::register_runtime`, snapshots enabled or not).
The VM answers every pending request at the end of its next `process_event` or
`apply_resolver_result` call, whatever that call changed (2026-09-27; before, a request
waited for the key's own next mutation): after the call's mutations it appends one per
requested entity, carrying the entity as its state table then holds it, marked with
`Mutation::mark_whole_entity` (`WholeEntity::Resent`). It rides in the VM's ordered
output, so it follows every patch emitted before it. A request for an entity the VM no
longer holds is dropped: the VM's next mutation for the key starts a new row, marked
created. The projector stores a resend in every view of the export and publishes it to
list and state views as an `upsert` (append views and the journal never see it). A
resend is not a change, so it keeps the `_seq` of the key's latest change rather than
its batch's (2026-09-28): `EntityResync` remembers, per requested key, the position of
the refused patch and of any later change it sees for the key, bounded at 4,096 like
the requests, and stamps the resend's cache entry, derived views and `upsert` with it.
The projector owns this because positions are assigned per batch, after the VM emits;
it sees each key's changes in stream order. A resend without a remembered position (past
the bound, or a whole entity from a source without a VM) takes its batch's. A
state subscriber that holds an evicted key keeps receiving its patches and is never
sent `remove` for an eviction.

Telling an evicted key from a new one does not depend on remembering evictions
(2026-09-27): the linked VM also marks each entity's first mutation since its
state-table row was created (`Mutation::mark_created`, `WholeEntity::Created`), and
that patch is the whole row — more than the change when a handler that emits nothing
(an instruction hook, `emit: false`) started the row. The flag rides on the table's
recency entry, so it is dropped with the row; rows restored from a snapshot count as
emitted. With a VM linked the cache stores a key it lacks only from a creation, merges a
creation into a key it holds (as clients do), and refuses any other patch for a key it
lacks — evicted however long ago, or never held, as after a restore or legacy migration
— and requests the whole entity, which follows in the VM's next batch. The flag is not
persisted in snapshots: a row a non-emitting handler started just before a snapshot
restores as emitted, so its first update is refused and the row appears whole with the
VM's next batch. So the guarantee holds whatever
`max_entities_per_view` is against the VM's table bound. A creation is still an
ordinary `patch` frame and journal record; only a resend is a state-only `upsert`. A VM
without requests marks nothing, so its output is unchanged for other consumers.

Without a linked VM the cache falls back to remembering evictions: each view remembers
8× its cache bound of evicted keys (4,000 by default), allocated on first eviction, and
refuses patches for them (one warning per projector); restores mark every VM key the
cache lacks as evicted; a key forgotten after that many *distinct* later evictions is
taken for new. A view fed by a linked VM drops that memory with its first patch and
keeps none. A mutation source without a VM can mark whole entities itself; otherwise
its evicted keys stay out until a source delete.

Bounds: requests are capped at 4,096 (asking again is safe). Entities are whole as far
as the VM holds them: a key the VM itself dropped restarts from what its next update
sets, and that update's mutation is a creation again.

SDKs discard a patch for an unheld key only under `wholeEntities`, so they keep working
against older servers. TypeScript (`frame-processor.ts`, the latest ack per
connection), Rust (`store.rs::apply_live`, per subscription) and Python
(`store.py::_handle_entity`, per subscription) implement the same rule, pinned by the
shared `whole-entities.json` fixture, and sort unranked entities last. TypeScript and
Rust report a patch dropped for a key they evicted locally (`evicted-key` /
`PATCH_FOR_EVICTED_KEY`; a `warn!`), since the server still counts that key as held.

