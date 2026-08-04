# SDK API Surface & Wire Formats — TypeScript ⇄ Rust Alignment

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
          "after":"1234:000000000010","snapshotLimit":100},
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

Still open (tracked as roadmap, see §5.5): execution layer (wallet/transaction/receipts),
HTTP program reads + chain client, sessions, stack runtime extensions (`read`/`flows`/
`math`/`addresses`) in Rust, semantic `prepare()` operations, and refreshing the public
`docs/src/content/docs/sdks/rust.mdx` + `.claude/skills/arete-consume` references.
