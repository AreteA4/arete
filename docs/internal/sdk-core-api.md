# Arete SDK Core API — Canonical Surface Specification

Status: **canonical spec**. This document defines the language-neutral API surface that
every Arete client SDK expresses. Each language SDK is an idiomatic projection of this
one surface: **same nouns, same shapes, same semantics — native idiom**.

Relationship to other documents:

- `docs/websocket-v2-protocol.md` — normative WebSocket wire protocol.
- `docs/internal/sdk-api-surface.md` — the recorded TypeScript surface (§3) and wire
  formats (§2), plus the TS ⇄ Rust alignment history. §2/§3 remain the byte-level
  contracts; this document is the language-neutral statement of the surface they encode.
- `docs/internal/sdk-rust-alignment-phase2.md` — Rust projection design + divergences.
- `docs/internal/sdk-python-alignment.md` — Python projection design + divergences.

A user who learns the model in one language must be able to carry it to any other:
the nouns (`views`, `programs`, `chain`, `transactions`, `read`, sessions, extensions),
the verbs (`use/watch/get`, `build/prepare/execute`, `fetch/exists`), and the semantics
(update taxonomy, snapshot authority, fail-closed builds, outcome model) never change —
only the spelling changes per language.

---

## 1. Core nouns

| Noun | Definition |
|---|---|
| **Stack** | A deployed set of entities + views + bundled program SDKs reachable at `endpoints.ws` / `endpoints.http`. Generated code produces a *stack binding* that types a client. |
| **Stack binding** | Generated, portable description of one stack: name, endpoints, view definitions, entity schemas, program definitions (instruction handlers, PDAs, accounts, provenance hashes), program-read descriptors. Pure data + pure functions; no I/O. |
| **View** | A named server query surface `"<Entity>/<view>"`. `state` views are keyed (one entity per key, typed key fields); `list` views (incl. derived views such as `latest`) are ordered collections. |
| **Program SDK** | Generated client for one Solana program bundled with a stack: raw instruction builders, PDA factories, account readers, semantic operations (instructions / transactions / flows), error metadata, and extension namespaces. |
| **Client** | A connected handle binding one stack: `views`, `programs`, `chain`, `transactions`, `wallet`, `execute`, connection lifecycle, store access. |
| **Session** | A multi-stack/multi-program composition: per-member clients, shared wallet fan-out, one execution host. |
| **Prepared operation** | A portable, not-yet-executed instruction / transaction / flow value carrying name, artifacts, required signer addresses, and error metadata. Composable. |
| **Extension** | Author-written code attached to a stack (`read`, `flows`, `addresses`, `constants`, `defaults`, `math`) or a program (semantic `operations`), pinned to exact artifact hashes and staged verbatim into generated output. |
| **Binding (operational)** | Endpoint + auth attachment for a deployment, program-read release (`prb_…`), or Solana gateway (`sgb_…`). Bindings change without changing portable artifact hashes. |

## 2. Wire contracts (all SDKs speak exactly these)

1. **WebSocket protocol v2** — `docs/websocket-v2-protocol.md`. JSON frames (server may
   gzip; magic `1f 8b`), `protocolVersion: 2` on every message. Envelope fields are
   camelCase; entity payloads are snake_case. Client sends
   `subscribe { subscriptionId, query, snapshot }`, `unsubscribe`, `ping`,
   `refresh_auth`. Query canonical field order:
   `view, key, partition, filters, take, skip, after, snapshotLimit`; filter keys
   sorted; unknown fields rejected. Server sends `subscribed` (effective query, mode,
   sort), `snapshot` batches (`snapshotId`, `authoritative`, `complete`), live frames
   `upsert | patch | remove | delete` (`key`, `data`, `seq "<slot>:<index>"`,
   `append` dot-paths), `unsubscribed`, and the structured error envelope
   (`code`, `retryable`, `retry_after`, `suggested_action`, `docs_url`, `fatal`).
2. **Program reads** — `program-read-http/v1`:
   `GET /v1/releases/<releaseHash>/accounts/<Account>/<address>` (`null` if missing),
   `…/exists` → `{exists}`, batch `POST …/accounts/<Account>` with per-item
   `ok | missing | error` statuses.
3. **Chain routes** — `/chain/{exists,lamports,rent-exemption,clock,accounts,mints,token-accounts}`
   (GET) and `/chain/{native-balance,balances}` (POST).
4. **Transaction relay** —
   `POST <base>/transactions/v1/{latest-blockhash,fee,simulate,send,signature-status,block-height}`.
5. **Auth** — token endpoint exchange (`Authorization: Bearer <publishableKey>` →
   `{token, expires_at}`), WS token via `?hs_token=` or Bearer upgrade header, refresh at
   `exp − 60s`; targeted tokens for program-read and gateway bindings; on 401 refresh and
   replay **once**. For `send`-scoped requests the replay is additionally gated on the
   **response** header `X-Arete-Upstream-Attempted: false` — the server's proof that it
   did not dispatch upstream, so replaying cannot double-submit. The marker is written by
   the server and only ever read by clients; SDKs must never send it on a request.

**Numeric rule**: `u64`/`u128` are decimal strings on the wire, native
arbitrary/64-bit integers in every SDK (`bigint` / `u64` / `int`). `seq` compares slot
numerically, index lexicographically.

**Ordering rule**: wherever the surface sorts strings — canonical subscription identity
(filter keys), and store ordering including the key tie-break — SDKs use JavaScript
`String.prototype.localeCompare` semantics (ICU default collation: case as a tertiary
difference with lowercase first, accents as a secondary difference), **not** code-point
order. Code-point sorting silently diverges on mixed-case keys, which base58 addresses
produce constantly.

## 3. Client & connection

Canonical operations (spelling per §10):

- `connect(stack, options)` → connected client. Options: `url`, `http_url`, `transport`
  (`websocket` default | `http` — http-only mode fails view subscriptions fast),
  `auth`, `wallet`, `programs`, `execution`, reconnect tuning.
- Lifecycle: `connect / disconnect / is_connected / connection_state`, observation hooks
  `on_connection_state_change / on_frame / on_socket_issue`.
- Slot cursor: `processed_slot`, `wait_for_processed_slot(slot)` — the reconciliation
  primitive used after writes.
- Reconnect keeps subscription identity stable: active leases re-subscribe with the same
  subscription ids; snapshot re-sync uses authoritative replacement (§5).

## 4. Views

Every view exposes the same six verbs; all take the same query options
(`filters`, `take`, `skip`, `partition`, `after`, `snapshot_limit`, `with_snapshot`,
optional schema/parser override):

| Verb | Meaning | Result shape |
|---|---|---|
| `use` | Live stream of **merged entities**: patches applied, `remove`/`delete` filtered out of yields | stream of `T` |
| `watch` | Raw update stream (taxonomy below) | stream of `Update<T>` |
| `watch_rich` | Update stream with before/after diffs | stream of `RichUpdate<T>` |
| `get` | One-shot read: awaits an equivalent lease's snapshot | `T[]` (list) / `T?` (state) |
| `get_sync` | Non-blocking read of an existing equivalent subscription; *absent subscription* ≠ *empty result* | `T[]?` / `T??` |
| `get_one` | First element convenience on list views | `T?` |

State views take a **typed key** (generated from the entity's key fields) plus the same
options. Dropping/breaking the stream releases the refcounted lease.

`get` and `get_one` send their options on the read's subscription exactly as the
streaming verbs do, then release it after the snapshot: a list
`get({ filters: { 'state.authority': a } })` subscribes with
`query: {view, filters: {"state.authority": a}}` and `snapshot: {enabled: true}`, and a
`get` without options sends the bare `{view}` query (state: `{view, key}`). They also
take a per-read bound on the snapshot wait that is not sent (TS `timeoutMs`, Python
`timeout`, Rust `GetOptions::timeout`; default the client's initial-data timeout). Rust's
`get()` takes no options, so the options are a `GetOptions` value with the stream
builders' method names: `list().get_with(GetOptions::new().filter(path, value).take(10))`
and `state().get_with(key, options)`.

**Update taxonomy** (identical everywhere): `upsert` (the whole entity — sent whenever a
key becomes a member of the subscription as far as the server knows: entering the
window, its first change after a truncated or disabled snapshot, every change on a
derived view), `patch` (partial merge into an entity the client holds; `append` paths
concatenate arrays), `remove` (left *this query's* window), `delete` (deleted from the
source view globally). A key that only moves position is not resent; clients order
members from the declared sort (§5).

**Subscription identity**: canonical JSON of `{query, snapshot}`. Equivalent queries
share one wire subscription; leases are reference-counted; ids stay stable across
reconnect.

## 5. Store & snapshot semantics

- Snapshot batches sharing a `snapshotId` are staged; on the final `complete: true`
  batch, `authoritative: true` **replaces** membership, `authoritative: false` (cursor
  resumes via `after`) **merges**.
- Patches deep-merge with `append`-path array concatenation.
- **No partial entities**: a live `patch` for a key the store does not hold — never
  received, or evicted locally (e.g. by a max-entries bound) — is not an entity. When
  the subscription's `subscribed` ack carries `wholeEntities: true`, it is discarded: no
  storage write, no tracked sequence, no membership, no update emitted. The entity
  appears with the next full `upsert`, which such a server guarantees for every key it
  has not sent (`docs/websocket-v2-protocol.md`, "Partial entities"). Without the field
  (an older server, which may send a key's first change as a patch) the patch is stored
  as the entity, as before. SDKs may apply the latest ack on a connection to all its
  subscriptions (TS) or each ack to its own subscription (Rust, Python); one connection
  talks to one server, so the two agree. Frames carrying `offset` are exempt: they are
  records from a replayable append view — events, applied as received (the server
  never promotes them to `upsert`, and a consumer resuming from a cursor holds what
  came before it). The same `offset` exemption applies to the stale-sequence guard
  below, because every event decoded from one transaction shares a `seq`.
- **Local eviction**: the server still counts a key the store evicted as held, so it
  keeps sending patches and no `upsert` until the key re-enters the window or the
  subscription is re-established; those patches are discarded and the entity stays
  missing until then. The protocol has no per-key re-fetch. SDKs that evict report each
  such drop distinctly (TS: diagnostic `reason: 'evicted-key'`, code
  `PATCH_FOR_EVICTED_KEY`, plus one `console.warn` per view; Rust: a `warn!` on the
  first per view), and keep the local limit's documentation explicit that it must
  exceed the subscription's size. Rust evicts only keys no subscription's membership
  references, so there the case needs membership and storage to disagree.
- **Stale-frame guard**: a live `upsert`/`patch` that is not newer than the cached entity
  must not write storage. It still grants query membership and emits an update carrying
  the *cached* value, so a query seeing the key for the first time converges on the
  authoritative version rather than the stale one. Snapshot rows deliberately bypass this
  guard; snapshot authority (above) governs them instead. Frames with an `offset` are tape
  records and bypass it too. "Newer" is decided by:
  - **`_version`** in the frame's data, when the server stamps one (`{epoch}:{counter}`,
    the server's merge order). Stale when the client tracks a version of the **same
    epoch** for the key and the frame's counter is not greater (numeric compare, so equal
    counts as duplicate: one frame routed to two queries of a view applies once). A frame
    with a version is never stale against no tracked version, another epoch, or a version
    that does not parse.
  - **`seq`** otherwise, for servers that predate versions: stale when `compare <= 0`
    against the tracked sequence. Frames without a sequence are never stale.
  `seq` cannot decide when the server sends versions: every update decoded from one
  transaction shares one, and within a slot account updates and instructions number
  themselves differently, so a later frame can carry a lower `seq`. An unsequenced or
  unversioned write must **not** clear the tracked sequence or version — doing so
  disarms the guard until the next frame that carries one. Versions are tracked beside
  sequences (TS: a non-enumerable `__version`; Python and Rust: a sibling map), and
  snapshot rows record theirs. The shared `frame-versions.json` fixture pins the rule.
- Ordering follows the server-declared `sort` from the `subscribed` ack. Entities tied on
  the sort field break the tie on entity key, and that tie-break is **always ascending** —
  the `desc` negation applies to the sort-field comparison only. Both comparisons use the
  §2 ordering rule. An entity whose sort value is **missing or `null`** cannot be ranked:
  it sorts **after** every entity that has one in both directions (the `desc` negation
  does not apply to it), and such entities order among themselves by the key tie-break.
  The server applies the same rule when it fills derived-view windows, so an unranked
  entity never displaces a ranked one.
- The store is an internal engine detail; languages may expose it (TS `store`) or keep
  it private (Rust/Python) — the six view verbs are the public contract.

## 6. Program SDKs

Layered, lowest to highest; every layer is present in every SDK:

1. **`raw.<ix>.build(params)`** — pure instruction building. Params are IDL wire shape:
   account-name keys override addresses, arg-name keys serialize, `resolve` feeds
   PDA-only seeds. Resolution classes: `signer | known | pda | userProvided`; PDA seeds
   (`literal | bytes | argRef | accountRef`) resolve in topological order. An explicit
   address (a params key, or the build's `accounts`, which wins) is used for every
   class, never re-derived, and seeds the PDAs that reference it; it must be a base58
   32-byte public key (`Invalid account override for "<name>": …`). A `signer` is
   caller-provided unless its kind is `wallet` (TS `signerKind: 'wallet'`, Rust
   `AccountResolution::WalletSigner`, Python `signer_kind="wallet"`): only a wallet
   signer falls back to the build's wallet (`payer`), and generated handlers declare
   every signer caller-provided (TS `'provided'`, Rust `AccountResolution::Signer`,
   Python `signer_kind="provided"`). Args
   serialize via the shared borsh layout
   (`u8…u128, i8…i128, f32/f64, bool, string, pubkey, bytes, vec, option, array,
   hashMap, struct, enum`). **Fail closed**: unknown param or missing non-option arg is
   an error. Result: `BuiltInstruction { program_id, accounts: AccountMeta[], data }`.
   Escape hatch: the underlying `InstructionHandler`
   (programId + discriminator + account metas + arg schemas + error metadata) is
   reachable and buildable directly.
2. **`pdas.<name>.derive(seeds…)`** — typed PDA factories over
   `find_program_address`: every PDA the ProgramSpec declares, plus, in a stack's
   program module, the stack's own (which win on a shared name). Python omits PDAs
   whose program is itself a seed reference.
3. **`accounts.<Account>.fetch / fetch_many / exists`** — HTTP program reads (§8)
   returning typed decoded accounts (`null`/absent for missing; batch preserves
   per-item status). Every account of the ProgramSpec has one, in a standalone
   program SDK and in a stack's program module. Rust and Python decode into models
   derived from the IDL, typed the way the Program Read API encodes each value:
   - **Integers** decode from JSON numbers or decimal strings (`u64` above 2^53 and
     every `u128`/`i128` travel as strings) at any depth — inside options, vectors,
     fixed arrays, tuples and maps (Rust `serde_utils::deserialize_wire_option` with
     the `serde_utils::wire` shape markers where the flat helpers do not reach).
     Integer kinds follow the flat convention (`u8`/`u16` widen to `u64`), and byte
     arrays decode as integer lists.
   - **Defined types.** Every IDL struct, tuple struct and enum an account reaches
     gets a model of its own, named after the IDL type (`AdminConfig`; in Python a
     dataclass with strict `admin_config_from_wire` and `admin_config_patch_from_wire`
     converters). A struct is an object keyed by its field names (snake_case fields;
     Rust models also accept the IDL's own camelCase key), a tuple struct an object
     keyed `field_0`, `field_1`, …; an inline tuple is an array (a Rust tuple, a
     Python `tuple`), a map a string-keyed object (`BTreeMap<String, T>`,
     `Dict[str, T]`). A recursive reference (and a tuple of more than eight
     elements) reads as raw JSON.
   - **Enums.** A unit variant is its name (`"Full"`), a data variant a one-key object
     from its name to its fields, tuple fields keyed `field_<index>`
     (`{"Partial": {"num_signatures": 5}}`, `{"Address": {"field_0": "…"}}`). Rust
     declares an externally tagged serde `enum` (`VerificationLevel::Partial {
     num_signatures }`, tuple variants as `Address { field_0 }`); Python keeps the
     value's wire shape — a `str`, or a one-key `dict` whose value is the variant's
     dataclass (`{"Partial": VerificationLevelPartial(num_signatures=5)}`, typed
     `VerificationLevel = Union[str, Dict[str, VerificationLevelPartial]]`, a
     unit-only enum `str`), and `verification_level_from_wire` rejects any other
     value. TypeScript types an enum as the union of its variant names, and its
     schema accepts unit variants only.
   - **Naming.** A standalone program SDK declares each model under its *stable
     name*: the account's or type's name in PascalCase (an account's with an
     `Account` suffix, a type's with a `Type` suffix, where that is taken). A stack
     shares a model an identical declaration already has (across programs too) and
     renames a different one the way TypeScript does: prefixed with the program name
     (`BetaFee`), then also with the `Account`/`Type` role (`OreTreasuryAccount`,
     where the ORE entity takes `OreTreasury`). A field the flat convention of the
     stack SDKs' captured-account types already expresses (a scalar, or an option or
     array of scalars) renders exactly as there, so a stack whose entity maps an
     account with the same fields reads into that same type. Entity types are
     unchanged: they keep nested defined types, tuples and maps as JSON values, so an
     account that has them reads into a model of its own beside the entity's type
     (the ORE stack's captured `Treasury` and its `OreTreasuryAccount` reader model).
   An account whose layout is an enum reads as raw JSON. No SDK generates program
   queries (`queries` stays empty; the runtimes execute hand-written ones).
4. **`instructions.<name>.prepare(input)`** — semantic single-instruction operations →
   `PreparedInstruction`.
5. **`transactions.<path>.prepare(input)`** — semantic multi-instruction operations →
   `PreparedTransaction`.
6. **`flows.<path>.prepare(input)`** — multi-transaction operations → `PreparedFlow`.
7. **Error metadata** — generated `ErrorMetadata { code, name, msg }` +
   `parse_program_error(code)`.
8. **Extension namespaces** — `addresses`, `constants`, `defaults`, `math`, plus
   program `read` and `operations` created with access to the fully connected
   program (§9, Rust and Python extension bundles).

Prepared values carry `name`, `artifacts`, `required_signer_addresses`, `errors`, and
per transaction the signer material it was prepared with (`signers`, such as a created
account's keypair: TS `signers`, Rust `PreparedTransactionBody::signers` set with
`with_signers`, Python `signers=`), and compose (prepend/append;
`create_prepared_transaction({operations})`, which keeps its parts' signers).

## 7. Execution

- `client.transaction(instructions, options)` — wrap built instructions and execute.
- `client.execute(prepared, options)` — run a prepared operation through the wallet:
  fail-closed signer validation (`SignerRegistry`), per-transaction callbacks
  (`on_transaction_start`, …), receipts with signatures. Each send hands the adapter
  the transaction's `signers`, then the registry's (TS and Python `signers`, Rust
  `WalletExecutionContext::signers`), and their addresses count toward validation. A
  Rust signer that carries its key implements `Signer::sign_transaction_message` (the
  Solana adapter's `SolanaOperationSigner`); the Solana adapters sign a required
  signature they do not own with it.
- **Outcome model** (identical in every SDK): four terminal statuses
  `confirmed | not-submitted | submitted-unknown | chain-failed`, each with the phase
  that produced it.
- **Wallet adapter**: one interface — `sign_and_send(instructions, options, context)`
  (+ optional `inspect_transaction`); adapters for platform wallet ecosystems live in
  separate packages.
- Post-write reconciliation: `wait_for_processed_slot` bridges writes to view state.

## 8. Program reads (`client.read` / program `accounts`)

Descriptor-driven: release hash + transport (`local-http` or hosted `prb_…` binding,
https-or-localhost rule), validated before use. Surfaces: typed `AccountReader<T>`
(`fetch/fetch_many/exists`), stack-level and program-level query executors, targeted
auth tokens per binding.

## 9. Chain, transaction relay, sessions, extensions

- **Chain** (`client.chain`): `exists`, `lamports`, `rent_exemption`, `clock`,
  `accounts`, `mints`, `token_accounts`, `native_balance`, `balances` — the nine routes,
  pluggable via a `ChainClient` interface (hosted gateway `sgb_…` bindings provide one).
- **Transactions** (`client.transactions`): `latest_blockhash`, `fee`, `simulate`,
  `send`, `signature_status`, `block_height` via a `TransactionTransport` interface,
  with the shared structured error body.
- **Amounts/SPL helpers**: raw⇄UI amount conversion pinned to mint decimals, associated
  token account derivation, token-program resolution.
- **Sessions**: `create_session({stacks, programs}, options)` — each member gets its own
  client; standalone programs become synthetic HTTP-only stacks; programs bundled by
  stacks are promoted by reference to the top-level program namespace, under the program
  identity rule below; composition mode requires explicit chain + transactions (no
  endpoint fallback) and promotes nothing, so its top level holds only the explicit
  programs; execution host is the first connected member; `set_wallet` fans out;
  `close` disconnects all.
- **Program identity**: a program SDK's identity is the package release it was
  generated from (`packageReleaseHash` / `package_release_hash`; absent for local
  builds). It describes exactly the generated SDK, so it is set last: generated entries
  apply the package's own extension (and TS read descriptor) first, then stamp the
  identity (`withProgramIdentity` / `with_program_identity`; Python stamps it in the
  package `__init__.py` after applying the bundle's `PROGRAM_EXTENSIONS`, or in the
  `ProgramDef` when the package has no extension; inside a stack package, in the
  program's `program_sdks/<program>` subpackage). Applying an extension outside generated code
  (`extendProgram`, `extendPrograms`, TS `withProgramRead`; Python `extend_program`,
  `extend_programs`) drops it. Programs are matched by identity, never by name
  (`compareProgramIdentity` / `compare_program_identity` → `same | unproven |
  different`), wherever a key can name two programs: session promotion and attaching
  programs to a stack (`withPrograms`, `ConnectOptions.programs`, per-member
  `programs`, `useArete(…, {programs})`):
  - **Both identities known** → equal (or the same object) is **the same program**: one
    program, no warning; a standalone session program that a stack already provides is
    served by that stack's connected instance (no second member), provided it reads the
    same way: it has no explicit session `programReads.<key>` / `program_reads[<key>]`
    descriptor, or that descriptor resolves to the one the stack reads through. A
    differing descriptor would be silently replaced by the stack's, so that pair is
    handled as **unproven** below: the standalone program takes the key with its own
    read configuration, with one warning. Different → `PROGRAM_KEY_CONFLICT`.
  - **At least one unknown, same `programSpecHash`** → **unproven**: the explicitly
    attached program wins for the key it is attached under, with one warning (TS
    `console.warn`, Python `logging`, once per distinct message) saying the programs
    could not be proven identical. A standalone session program takes
    `session.programs.<key>`; attached to a stack (`withPrograms` and the rest above),
    it replaces the stack's program in that client's `programs.<key>`. Two stacks with
    unproven copies promote the first stack's program, with one warning.
  - **Otherwise** (different or missing `programSpecHash`) → `PROGRAM_KEY_CONFLICT` when
    the session or stack is composed; the message names the stack and the key and gives
    the fix: use the stack-scoped program (`session.stacks.<stack>.programs.<key>`), or
    attach the standalone program under a different key. Two stacks providing different
    programs under one key → the session is still created and both stay reachable
    through their stacks; the top-level key is not promoted, and reading it fails with
    `PROGRAM_KEY_CONFLICT` naming both stacks, the key and the stack-scoped paths.
  - A session's stack-scoped access (`session.stacks.<stack>.programs.<key>`) is never
    replaced by a session-level program, and a stack definition's `programs` are never
    modified.
- **Extension API contract**: every SDK exports `EXTENSION_API_VERSION` (currently `1`),
  the version of the extension-authoring surface that generated code and published
  extensions import: stack/program extension definition and composition helpers (TS
  `defineStackExtensions`, `defineProgramExtensions`, `extendProgram`, `extendPrograms`,
  `extendStack`; Python `extend_program`, `extend_programs`, `extend_stack`,
  `program_extensions_of`, `stack_extensions_of`; Rust the `Programs` / `ProgramSdk` /
  `StackWithPrograms` binding traits and `ProgramContext` / `ProgramAccessor`), program read attachment
  (TS `withProgramRead` and its equivalents), and the instruction helpers generated code
  imports (instruction handlers, PDA derivation, the borsh layout, prepared-operation
  constructors). It bumps **only on a breaking change** to that surface; additive changes
  keep it. Each package records it in its manifest: TypeScript `package.json`
  `arete.extensionApi`, Rust `Cargo.toml` `[package.metadata.arete] extension-api`,
  Python `pyproject.toml` `[tool.arete] extension-api`. An extension records the
  `extensionApi` it targets; `sdkRange` stays as a floor for older CLIs.
- **Extensions pipeline**: one `extensions.json` manifest
  (`entry`, `files`, `inputKind`, `inputHash`, `sdkRange`, optional `language` —
  absent = TypeScript, `"rust"`, `"python"` — and optional `extensionApi`, a positive
  integer). A declared `extensionApi` must equal the installed runtime's (TS
  `node_modules/@usearete/sdk/package.json`; Rust the path/`[patch]` crate or the
  `Cargo.lock` release in Cargo's source cache; Python the project virtualenv's
  `arete/extensions.py`): a mismatch is a hard error, a runtime without the value warns,
  and nothing is checked when the runtime is not installed. Without `extensionApi` the
  `sdkRange` comparison stays warning-only. `extensionApi` is hashed into the bundle's
  `contentSha256` only when declared, so existing bundle identities are unchanged.
  CLI (`a4 sdk create/install/sync`) resolves → pin-validates against stack-manifest / program-spec hashes (hard errors on
  mismatch) → stages files verbatim → wires them into the generated module using the
  language's explicit wiring convention → records `sdk-provenance.json`. A hosted
  bundle's manifest `language` must be the target it was served for (absent =
  TypeScript): anything else is a hard error, never a silent drop, and its hash is
  never written to `arete.lock`. A hosted Rust or Python bundle's staged bytes must
  re-hash to the resolved `contentHash` (the registry's rule: lowercase hex SHA-256 of
  the compact JSON `{"files": {path: source}, "manifest": {"entry", "files",
  "inputHash", "inputKind", "language", "sdkRange"}}`, keys sorted, absent values
  `null`, `language` present only for Rust and Python, `extensionApi` never hashed).
- **Stack SDKs embed program extensions**: every language generates each stack
  program whose program package release carries its own extension for that target
  with that extension, so `a4 install stack <s> --<lang>` and
  `a4 install program <p> --<lang>` expose the same program surface: TypeScript as a
  program SDK module (`programs/<program>/`), Rust in the program's module of
  `programs.rs` (bundle files under `programs/<program>/`), Python as a
  `program_sdks/<program>/` subpackage. Multi-live Rust and Python compositions refuse a
  program extension rather than drop it.
- **Rust and Python extension bundles**: flat files beside the TypeScript bundle they
  mirror; entry `extensions.rs` / `extensions.py`; `extensions.json` carries
  `"language": "rust" | "python"`. Module names reserved for generated code: `entity`,
  `types`, `mod`, `lib`, `programs`, `models`, `views`, `generated` (Rust program
  bundles also `pdas`; Python also `__init__`, and stack bundles `program_sdks`).
  Bindings follow the TypeScript surface path, converted per segment
  (`math.formatAmount` → Rust `math::format_amount`, Python `math.format_amount`;
  `constants.X` → Rust `constants::UPPER_SNAKE`); the **SDK root** is a standalone
  program crate's root, a stack crate's `programs::<program>` module, or the stack
  crate's root for a stack bundle.
  - **Rust.** Pure namespaces (`addresses`, `constants`, `defaults`, `math`) are
    modules of free functions, constants and statics. Context namespaces (`read`,
    `instructions`, `transactions`, `flows`) are modules of `async` free functions that
    take their context first: program bundles
    `&arete_sdk::ProgramContext<'_, <Name>Program>`, stack bundles' `read`
    `&Arete<<Stack>>`. Inputs derive `serde::Deserialize`; results are
    `arete_sdk::operations::Prepared*` or `serde::Serialize` values. No traits, macros
    or `async-trait`; dependencies are `std`, `arete_sdk`, `serde`, `serde_json`.
    `ProgramContext` exposes `chain()` (the client's `ChainClient`), `wallet()` (its
    current wallet, read at call time) and `program()` (the generated accessor: typed
    instruction builders and an `<account>_accounts()` reader for every account of the
    program's ProgramSpec, in a standalone program crate and inside a stack alike).
    Every generated accessor implements
    `ProgramAccessor` and has `context()`: `client.programs.<p>.context()` for a
    standalone crate connected as `Arete::<ProgramStack<…>>` or through a `Session`,
    `a4.programs.<p>.context()` inside a stack client; `ProgramContext::new(&accessor)`
    when an instruction named `context` takes the method name, and
    `ProgramContext::from_parts` / `ProgramBuilder::with_chain` / `with_wallet` for
    fixtures. Generated items are imported through `super::generated`, a re-export
    module the generator emits beside the bundle: at a standalone program crate's root
    it re-exports the program module's items (`PROGRAM_ID`, `*Params`, builders,
    `pdas`, `<Name>Program`, read descriptors) and the generated types; inside a stack
    it is `programs::<program>::generated`, re-exporting the program module and the
    stack's types; at a stack crate's root it re-exports the stack binding, the types
    and `programs`. A program module's `generated` also re-exports each account model,
    and each model of the IDL types they reach (§6), explicitly under its *stable
    name*, the one the standalone program crate declares it under
    (`generated::Vault`, `generated::AdminConfig`), even where the stack's `types.rs`
    renamed it around an entity or another program's type (`VaultVault`). Other bundle files are imported as `super::<module>`. The generator
    declares `pub mod <file>;` per staged file and re-exports the entry
    (`pub use extensions::*`); the entry declares no file modules, and a namespace is a
    bundle file named after it or an inline module (`pub mod math { … }` or
    `pub use super::ore_math as math;`).
  - **Python.** A program bundle's entry exports `PROGRAM_EXTENSIONS = {…}` with any of
    `addresses`, `constants`, `defaults`, `math`, `pdas`, `create_operations(ctx)`,
    `create_read(ctx)`; a stack bundle's entry exports `STACK_EXTENSIONS = {…}` with
    any of `addresses`, `constants`, `defaults`, `math`, `read_arg_counts`,
    `create_read(client)`, `create_flows(client)`. The generated `__init__.py` imports
    the bundle files as submodules (nothing is star-imported, so entries never patch
    generated names) and applies the mapping —
    `extend_program(programs.<P>_PROGRAM, **program_extensions_of(entry))` then the
    identity stamp, or `extend_stack(<STACK>, **stack_extensions_of(entry))`. The CLI
    refuses an entry that never names its export; `program_extensions_of` /
    `stack_extensions_of` raise `ImportError` for a missing export and `TypeError` for
    unknown keys. `ctx` is `ProgramOperationContext` (`chain`, `wallet`, `program`).
    `create_read` returns a mapping of async callables, composed base-first like
    `create_operations` and surfaced as the connected program's `read` namespace
    before `create_operations` runs (TS `createRead`). Bundles import generated items
    from `.programs` and `.models` and sibling files relatively; inside a stack the
    bundle sits in `program_sdks/<program>/`, whose `programs.py` / `models.py`
    re-export the stack's, so the same relative imports resolve. That `models.py` also
    aliases each account and IDL type model the stack renamed (with its converters,
    and an enum's variant dataclasses) back to its stable name, so
    `from .models import Vault` names the account model in both.
- **Extension runtime helpers**: the SDK functions and types every Rust and Python
  extension port uses, so the ports match the TypeScript extension they mirror.
  **Porting rule: use the SDK helpers, never reimplement them** in a bundle (no
  bundle-local amount decoder, clock struct, hash, base58 codec or error type that
  only exists to change an error's text).
  - **Amount inputs** (TypeScript `AmountInput`, `bigint | { ui } | { raw }`). Rust
    `arete_sdk::AmountInput` implements `Deserialize`, so an input field is typed
    `AmountInput`: a bare integer is raw, `{ "raw": … }` takes an integer, an
    integral number or a string as JavaScript `BigInt(…)` reads it, and `{ "ui": … }`
    takes a string, or a number kept as JavaScript's `String(number)` text; an object
    carries exactly one key, and raw values are unsigned (`u128`). It serializes as
    `{ "raw": "<decimal>" }` / `{ "ui": "<text>" }`. Python
    `arete.decode_amount_input(value)` applies the same rules (raising `TypeError`
    for a wrong shape, `ValueError` with the `BigInt` message for an unconvertible raw
    value; a negative raw `int` decodes, as a TypeScript bigint does) and returns
    `{"raw": int}` or `{"ui": str}`; `arete.AmountInput` is the type alias. Resolve
    either with the amount helpers (`to_raw_amount`, `resolve_amount`, …).
  - **Filtered view reads.** A stack extension's TypeScript
    `client.views.<Entity>.list.get({ filters: { '<path>': value } })` is Rust
    `a4.views.<entity>.list().get_with(GetOptions::new().filter("<path>", value)).await`
    and Python `await client.views.<Entity>.list.get(filters={"<path>": value})`: the
    same paths and values, so the host query, and the view fixture it replays
    (`args: [{"filters": {…}}]`), is the TypeScript one. Keep the TypeScript
    extension's client-side check of the returned rows as well.
  - **Chain clock.** Rust `arete_sdk::ChainClock` implements `Serialize` as the
    TypeScript `ChainClock` (`slot`, `epoch`, `leaderScheduleEpoch`,
    `unixTimestamp`; an absent optional field omitted), so a result carries it as is.
  - **Errors.** Rust bundles return `AreteError::InvalidInput(message)` (or
    `AreteError::invalid_input(message)`) for input and validation errors: its
    `Display` is the message alone, so it carries the TypeScript text exactly.
    `AmountError` (except a failed chain read), `InstructionError` and `Base58Error`
    convert to it with `?`. `AreteError::InvalidConfig` prefixes
    `Invalid configuration:` and is for client configuration only. Python bundles
    raise `ValueError`, or `TypeError` for a wrong type, with the TypeScript text.
  - **Hashing and base58.** Rust `arete_sdk::{keccak256, sha256}(&[&[u8]]) -> [u8; 32]`
    and Python `arete.keccak256(*parts) -> bytes` / `arete.sha256(*parts) -> bytes`
    hash the concatenation of their parts (Solana's `keccak::hashv` / `hash::hashv`;
    Keccak-256 is the original Keccak padding, not `hashlib.sha3_256`). Base58 is
    TypeScript `encodeBase58` / `decodeBase58`: Rust
    `arete_sdk::encode_base58(&[u8]) -> String` and
    `decode_base58(&str) -> Result<Vec<u8>, Base58Error>`, Python
    `arete.encode_base58(bytes) -> str` and `arete.decode_base58(str) -> bytes`
    (`ValueError`); both fail with the TypeScript text
    `Invalid base58 character: <c>`.
  - **Instruction artifacts.** A TypeScript semantic instruction carries its built
    instruction as `artifacts.instruction`, a TypeScript `BuiltInstruction`:
    `{ programId, keys: [{ pubkey, isSigner, isWritable }], data }`. The Rust and
    Python `BuiltInstruction` keep their own field names (`program_id`, `accounts`),
    so a port builds that shape with `to_artifact()`, never by hand: Rust
    `instruction.to_artifact()` returns a `serde_json::Value` (artifacts are JSON, so
    `data` is an array of byte values, as TypeScript's `toJsonValue` encodes it), and
    Python `instruction.to_artifact()` returns a dict whose `data` is `bytes` (the
    TypeScript artifact's `Uint8Array`; `to_json_value` lists its byte values, as
    TypeScript does). Mirror `artifacts: { instruction }` with
    `json!({ "instruction": instruction.to_artifact() })` /
    `{"instruction": instruction.to_artifact()}`.

## 10. Idiom matrix

The projection rules per language; anything not listed follows the host language's
standard style (casing, error, async, and options conventions).

| Canonical | TypeScript (`@usearete/sdk`) | React (`@usearete/react`) | Rust (`arete-sdk`) | Python (`arete-sdk`) |
|---|---|---|---|---|
| connect | `await Arete.connect(STACK, {opts})` | `useArete(stack)` under a provider | `Arete::<Stack>::builder()…connect().await` | `await Arete.connect(STACK, **opts)` / `async with` |
| view access | `a4.views.OreRound.latest` | `arete.views.OreRound.latest` | `a4.views.ore_round.latest()` | `a4.views.ore_round.latest` |
| `use` | `.use(opts)` → `AsyncIterable<T>` | `.use(opts)` → status-discriminated hook result | `.listen()` + builder methods → `impl Stream<Item=T>` | `.use(**opts)` → `AsyncIterator[T]` |
| `watch` / `watch_rich` | `.watch(opts)` / `.watchRich(opts)` | *(covered by hook statuses)* | `.watch()` / `.watch_rich()` + builders | `.watch(**opts)` / `.watch_rich(**opts)` |
| `get` / `get_sync` / `get_one` | `await .get(opts)` / `.getSync(opts)` / list-first | `.useOne(...)` | `.get().await` or `.get_with(GetOptions::new()…).await` / `.get_sync()` / `.get_one().await` | `await .get(**opts)` / `.get_sync(**opts)` / `await .get_one(**opts)` |
| state key | `.state.use({roundId: 42n}, opts)` | same | `.state().listen(key)` + builders | `.state.use(round_id=42, **opts)` |
| query options | options object | options object | builder chain (`.take(10).filter(…)`) | keyword arguments |
| raw build | `ore.raw.deploy.build(params)` | same (via `useMutation` for execution) | `a4.programs.ore.deploy(DeployParams{…})` (typed struct, `deny_unknown_fields`) | `ore.raw.deploy.build(**params)` (kwargs, fail-closed) |
| prepare | `ore.instructions.deploy.prepare(input)` | `…deploy.useMutation()` | `prepare` on generated ops | `ore.instructions.deploy.prepare(**input)` |
| PDA | `ore.pdas.miner.deriveSync({accounts})` | same | `ore::pdas::miner(authority)` | `ore.pdas.miner.derive(authority=…)` |
| accounts | `ore.accounts.Miner.fetch(addr)` | `…use()` read hooks | `…miner_accounts().fetch(addr)` | `await ore.accounts.miner.fetch(addr)` |
| execute | `await a4.execute(prepared, opts)` | mutation phase machine + reconciliation | `a4.execute(prepared, opts).await` | `await a4.execute(prepared, **opts)` |
| errors | thrown `Error` subclasses / result objects | status unions | `Result<_, thiserror enums>` → `AreteError` | `AreteError` exception hierarchy |
| absent vs empty | `undefined` vs `null`/`[]` | `isPending` vs `isEmpty` | `None` vs `Some(empty)` (nested `Option`) | sentinel `UNSET` vs `None`/`[]` (see Python doc) |
| session | `createSession({stacks, programs})` → `session.stacks.<k>` | provider-level | `session.stack::<OreStack>("ore")` (runtime-keyed) | `create_session(stacks={…})` → `session.stacks.<k>` |
| program identity | `ProgramSdkDefinition.packageReleaseHash`; `PROGRAM_KEY_CONFLICT` error code | same | `ProgramSdk::package_release_hash()`; `same_program::<A, B>()` (typed paths make key conflicts unrepresentable — see the Rust doc) | `ProgramDef.package_release_hash`; `ProgramKeyConflictError` (`code="PROGRAM_KEY_CONFLICT"`) |
| extension API | `EXTENSION_API_VERSION`; `package.json` `arete.extensionApi` | — | `arete_sdk::EXTENSION_API_VERSION`; `[package.metadata.arete] extension-api` | `arete.EXTENSION_API_VERSION`; `[tool.arete] extension-api` |
| program extension context | `ProgramOperationContext` (`chain`, `wallet`, `program`) passed to `createOperations` / `createRead` | same | `ProgramContext<'_, P>` (`chain()`, `wallet()`, `program()`) from `a4.programs.ore.context()`, first argument of extension functions | `ProgramOperationContext` passed to `create_operations` / `create_read` |
| extension bundle | default export of `defineProgramExtensions` / `defineStackExtensions` | same | namespace modules of free functions, `super::generated` imports | `PROGRAM_EXTENSIONS` / `STACK_EXTENSIONS` mapping |
| wire payload casing | snake_case → camelCase transform (zod) | same | snake_case → snake_case (serde) | snake_case natively — no transform |
| u64 | `bigint` | `bigint` | `u64`/`u128` | `int` |
| IDL enum (account reads) | variant-name union (unit variants) | same | externally tagged `enum` (`Level::Partial { num_signatures }`) | `str` or `{"Partial": LevelPartial(...)}` |
| validation | zod schemas + patch schemas | zod | serde typed structs | generated converters (typed dataclasses; u64-string → int) |

Documented per-language divergences live in the language alignment docs. Divergences
must be *idiom*, never *semantics*: a language may rename a verb to avoid a keyword
(Rust `listen` for `use`) or swap options-objects for builders/kwargs, but may not
change the update taxonomy, snapshot authority, fail-closed behavior, outcome statuses,
or wire formats.

## 11. Conformance checklist

A language SDK claims alignment when it implements, with these exact semantics:

1. Protocol v2 client: canonical subscription identity, refcounted leases, stable ids
   across reconnect, gzip frames, structured error envelope, `refresh_auth`.
2. The six view verbs with the full query-option set on both list and state views.
3. Store semantics: snapshot staging + authoritative replacement, patch/append merge,
   no partial entities (patches for unheld keys discarded under an ack's
   `wholeEntities`, stored without it; tape records exempt), server sort with unranked
   entities last.
4. Instruction runtime: shared borsh layout (byte-identical — port the test vectors),
   topo-sorted account resolution, PDA derivation, fail-closed builds, error metadata.
5. Program SDK layers 1–8 (§6) generated from the stack artifacts by the interpreter's
   language backend.
6. HTTP surfaces: auth token machinery (strategy order token > provider >
   token_endpoint > hosted default; targeted tokens w/ LRU; refresh-replay-once;
   predispatch marker), chain (9 routes), transaction relay (6 routes), program reads
   (v1 contract).
7. Execution: prepared operations + composition, wallet adapter interface, signer
   validation, four-state outcome model, receipts, `wait_for_processed_slot`.
8. Sessions with the §9 semantics, including the program identity rule.
9. Extensions: manifest `language` dimension, pin validation, verbatim staging,
   language-native wiring, provenance file, exported `EXTENSION_API_VERSION` recorded
   in the package manifest.
10. Generated example app in `examples/` regenerated from the live codegen.
