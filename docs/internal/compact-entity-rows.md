# Compact entity rows

Entity rows are held in memory as `serde_json::Value` trees, and a tree
costs several times the JSON it stands for: 9x for a row made of arrays of
small objects. This note records where those bytes go, measures four
other representations against the operations the runtime performs on rows,
and picks one. It then describes the first slice, which packs the VM's
state-table rows, and lists the slices that remain.

## Where rows live

| Holder | Rows | Representation before this work |
| --- | --- | --- |
| VM state table (`interpreter`, `StateTable`) | up to 2,500 per entity (`DEFAULT_MAX_STATE_TABLE_ENTRIES`) | `DashMap<Value, Value>` |
| Server entity cache (`arete-server`, `EntityCache`) | up to 500 per view, one copy shared by an export's views (`SharedEntity`) | `Arc<Value>` plus each view's `_version` |
| Derived sorted views (`SortedViewCache`) | bounded by the view | share the entity cache's `Arc<Value>` |

A VM state table can hold five times as many rows per entity as a cached
view, so for entities that fill both, the VM holds most of the row memory.

## Where the bytes go

One row of each fixture shape (described under [Method](#method)). Bytes
are rounded to jemalloc size classes. Map node bytes are the measured total
minus every other category.

| Part | Position (13.4 KB JSON, 124 KB heap) | Pool (50.1 KB JSON, 188 KB heap) | Token (0.6 KB JSON, 3.1 KB heap) |
| --- | ---: | ---: | ---: |
| Map nodes (`BTreeMap` leaves, including their 32 B value slots) | 92,160 B, 74% (144 objects) | 131,840 B, 70% (206 objects) | 2,560 B, 83% (4 objects) |
| Key strings (one heap `String` per key per object) | 5,400 B, 4% (444 keys) | 14,776 B, 8% (1,332 keys) | 248 B, 8% (23 keys) |
| All-digit strings (u64 and u128 amounts) | 7,680 B, 6% (411) | 3,208 B, 2% (222) | 0 |
| Other strings (addresses, signatures, labels) | 2,064 B, 2% (230) | 31,384 B, 17% (690) | 280 B, 9% (9) |
| Array buffers (32 B per element) | 16,704 B, 13% (144 arrays) | 7,168 B, 4% (2 arrays) | 0 |
| Numbers and booleans | inline in their slot (8) | inline in their slot (413) | inline in their slot (11) |

The map nodes dominate. A `BTreeMap<String, Value>` leaf has room for 11
entries whatever the object holds, so it is 632 bytes (640 after rounding)
for a 4-field fee record as much as for an 11-field section. Repeated keys
and numbers carried as strings cost far less than the nodes.
A cloned and later freed tree is also 1,373 (position) or 2,452 (pool)
separate allocations. The allocator churns through these on every copy and
fragments its size classes.

## Candidates

Each candidate was prototyped in a standalone harness (not committed)
against the same fixtures, with a check that every one round-trips and
serializes to the same bytes as the original `Value`.

- **(a)** `serde_json::Value` with `preserve_order` (IndexMap-backed objects).
- **(b)** Interned keys: `Arc<str>` from a per-table interner, objects as a
  sorted `Box<[(Key, V)]>`, arrays as `Box<[V]>`. Strings and numbers stay
  as in serde_json.
- **(c)** A compact typed tree: `u32` key symbols from a per-table symbol
  table, sorted boxed objects, native numbers, strings up to 22 bytes inline,
  and canonical u128 decimal strings stored as the integer. 24 bytes per
  slot.
- **(d1)** JSON bytes: the row serialized with `serde_json::to_vec`, parsed
  when needed.
- **(d2)** Packed bytes: one binary buffer per row holding the row's keys
  once in a per-row dictionary, varint integers, and all-digit strings at
  two digits per byte. Three variants were measured: keys inline in each
  object, a leading dictionary, and a trailing dictionary with key
  prediction for arrays of same-shaped objects ("tuned"). The prototype
  also stored a byte length per container so readers could skip subtrees.

### Memory per row

Bytes per row rounded to jemalloc size classes, averaged over 100 rows of
each shape. "Shared" is per-table state amortized over all rows (interner
or symbol table), not included in the row figure.

| Representation | Position | Pool | Token | Allocations per position row |
| --- | ---: | ---: | ---: | ---: |
| `Value` (BTreeMap, as today) | 124,309 B (9.1x JSON) | 188,381 B (3.8x) | 3,102 B (5.2x) | 1,373 |
| (a) `Value` with `preserve_order` | 141,771 B (10.4x) | 325,139 B (6.5x) | 4,309 B (7.3x) | 1,517 |
| (b) interned keys, sorted boxed maps | 40,034 B (2.9x), shared 1.7 KB | 95,549 B (1.9x) | 1,298 B (2.2x) | 929 |
| (c) compact typed tree | 26,657 B (2.0x), shared 1.6 KB | 76,904 B (1.5x) | 965 B (1.6x) | 294 |
| (d1) JSON bytes | 14,338 B (1.0x) | 57,346 B (1.1x) | 642 B (1.1x) | 1 |
| (d2) packed, inline keys | 10,242 B (0.7x) | 49,154 B (1.0x) | 573 B (1.0x) | 1 |
| (d2) packed, key dictionary | 7,170 B (0.5x) | 40,962 B (0.8x) | 642 B (1.1x) | 1 |

### CPU per operation

Microseconds per call for one position / one pool row, measured as thread
CPU time (the fastest of 41 batches) in a release build with the system
allocator. "Merge array" replaces the position's 70-element `fees` array,
and for the pool appends one swap to a 100-element array, dropping the
oldest. Both use the projector's deep-merge rules. "Pack" builds the
representation from a `Value` and "unpack" rebuilds the `Value`. For the
`Value` baseline, both columns show a deep clone.

| Operation | `Value` | (b) interned | (c) compact tree | (d1) JSON bytes | (d2) packed, tuned |
| --- | ---: | ---: | ---: | ---: | ---: |
| get a field by path | 0.06 / 0.03 | 0.08 / 0.05 | 0.11 / 0.05 | 130 / 254 | 0.46 / 0.46 |
| set a field by path | 0.14 / 0.09 | 0.07 / 0.06 | 0.12 / 0.07 | 162 / 311 | 5.5 / 7.7 |
| deep-merge a 3-field patch | 0.49 / 0.98 | 0.54 / 1.07 | 0.64 / 0.90 | 152 / 324 | 5.5 / 8.0 |
| merge array (replace / append and truncate) | 33.7 / 1.05 | 25.5 / 0.77 | 15.2 / 0.58 | 209 / 287 | 9.8 / 6.9 |
| serialize to JSON | 12.8 / 45.3 | 14.7 / 46.9 | 26.3 / 52.9 | 0.25 / 0.95 | 29.1 / 66.7 |
| clone | 73 / 164 | 57 / 82 | 23 / 49 | 0.26 / 0.91 | 0.13 / 0.66 |
| pack from `Value` | 76 / 167 | 72 / 113 | 47 / 83 | 18 / 47 | 17 / 37 |
| unpack to `Value` | 76 / 167 | 100 / 194 | 103 / 185 | 138 / 262 | 113 / 227 |

The packed candidate's get, set and merge work on the bytes directly. Get
skips sibling subtrees. Set and merge stream the row into a new buffer,
copying untouched subtrees as bytes, so they cost time in proportion to
the row rather than the patch. Serialization walks the bytes through
serde_json's serializer.

With jemalloc the allocation-bound operations are faster, and the parsing
in unpack accounts for more of its cost. On the position row, clone takes
50 µs, packed unpack 58 µs and pack 19 µs. On the pool row they take 95,
134 and 39 µs. Building unpacked objects with `FromIterator`, which the
committed codec does, took about 10% off the pool unpack.

`preserve_order` is ruled out on two counts. It is larger (IndexMap keeps
a hash index and per-entry hashes beside the entries), and it would
change the wire. Objects would serialize in insertion order instead of
sorted key order, and the cargo feature applies to every crate in a build.

## How rows are used

**VM.** A handler runs on one entity at a time.

- `ReadOrInitState` takes the row out of the table into a register.
- Opcodes (`SetField`, `SetFieldIfNull`, `SetFieldMax`, `AppendToArray`
  with truncation, `Transform`, ...) change the `Value` in that register.
- `EvaluateComputedFields` runs a generated closure over the
  `&mut Value`. These closures come from `arete-macros` and are compiled
  into every stack.
- `UpdateState` writes the row back.
- `EmitMutation` builds the patch (the dirty paths) or, for a creation,
  the whole row from the register.
- Instruction hooks read and change the state register after the event.
- Resolver results and deferred `when` writes read a row, change it and
  write it back.
- Whole-entity requests, `get_entity_state` and snapshot dumps read
  copies of rows.

Everything between read and write needs a real `Value`.

**Server.**

- `EntityCache::upsert_views` deep-merges each patch into the shared
  fields, appending to and truncating arrays at `append` paths.
- Sorted views and filters read single fields through
  `EntityFields::field`, which returns `&Value`.
- Snapshot and query delivery turn the entity into an owned `Value`, apply
  the projection and wire format, and serialize it.
- Dumps copy each entity, and hydration rebuilds shared copies.

What each option would touch:

- **(b) and (c)** pay off only where code works on them natively. In the
  VM that means every opcode, the resolver and deferred paths, and the
  generated computed-field closures (a code generation change in every
  stack). In the server it means the merge, `EntityFields` (which returns
  references) and wire formatting. Even then they are 3-6x larger than
  (d2).
- **(d1)** is a boundary-only change like (d2), but twice its size. It is
  also slower to unpack, since it parses JSON text.
- **(d2)** can be introduced behind existing APIs. Each holder converts at
  the boundary where code needs a `Value`. The VM unpacks on read and
  packs on write. The server can merge and serialize on the bytes
  directly.

## Decision

Rows at rest are packed bytes (d2) with a per-row key dictionary:

- **Smallest.** A position row takes 0.5x its JSON, 17x smaller than the
  `Value` tree.
- **One allocation per row.** The allocator stops churning through
  thousands of small blocks per copy.
- **Cheapest boundary.** Packing writes one buffer from a borrowed
  `Value`, cheaper than the clone it replaces. Unpacking performs the
  allocations a clone performs, plus parsing.
- **Lossless.** Unpacking returns an equal `Value`: the same field order,
  the same strings, and numbers in the same representation (unsigned,
  negative or float). Every serialization downstream is therefore
  byte-identical. A number that cannot be stored exactly as an integer or
  float (only possible with serde_json's `arbitrary_precision`) is kept as
  text.
- **No shared state.** The dictionary is per row, so no interner outlives
  or leaks across tables. Dropping a stack's tables frees everything.
- **Free to change.** The format exists only in memory and is never
  persisted, so it can change without migrations.

## First slice: VM state tables

This change stores state-table rows as `PackedRow` (`interpreter/src/packed_row.rs`):

- `StateTable` keeps `DashMap<Value, PackedRow>`.
  - `take_and_touch`, `get_and_touch` and the new `get` unpack.
  - `insert_with_eviction(key, impl Borrow<Value>)` packs from a reference,
    so a caller that keeps the entity no longer copies it.
- `UpdateState` packs from the state register. The register keeps the
  entity, so `EmitMutation`, `CopyRegister` and instruction hooks read it
  there as before.
  - Moving the register into the table was an optimization to avoid
    copying it. Nothing is copied any more, so the optimization is
    removed. `retain_state_register(false)` is now a deprecated no-op.
- Resolver results, deferred `when` writes and the failed-handler path
  pack by reference instead of cloning.
- Snapshot dumps unpack each row into the existing `StateTableSnapshot`,
  and restores pack them again. The snapshot format is unchanged.
- `VmCacheStats::state_table_row_bytes` and `StateTable::row_bytes` report
  the packed bytes, so an embedder can watch them.

### Measured

`cargo test -p arete-interpreter --test state_table_memory -- --nocapture`
counts live heap bytes per row in a state table, including the row's key
and the table's bookkeeping. The file uses only the public API, so it ran
unchanged on `main`.

| Row | JSON | as `Value` | table row on `main` | table row now | now / `Value` |
| --- | ---: | ---: | ---: | ---: | ---: |
| position | 13,676 B | 116,579 B | 116,808 B | 6,443 B | 0.055 |
| pool | 50,056 B | 175,369 B | 175,606 B | 35,693 B | 0.204 |
| token | 562 B | 2,945 B | 3,183 B | 748 B | 0.254 |

The ignored test in the same file times `VmContext::process_event` in a
release build. The workloads are a position account handler that maps 17
fields onto the row, and a swap instruction handler that appends to a
pool's recent swaps and sets three fields. The figures below are medians
over ten interleaved runs of each build, with the fastest run in
parentheses. The machine was shared and loaded throughout, so single runs
varied by up to 50%.

| | `main` | this change |
| --- | ---: | ---: |
| VM heap holding 2,500 positions | 277.0 MiB | 15.4 MiB |
| position update, spread over 2,500 rows | 543 µs (507) | 641 µs (495) |
| position update, one row | 571 µs (474) | 581 µs (480) |
| swap appended to one of 20 pools (175 KiB rows) | 228 µs (188) | 344 µs (251) |

### The CPU trade-off

On `main`, a handler moves its row out of the table, and `UpdateState`
clones the register back in (the register is retained by default, for
instruction hooks). Packed, the handler unpacks the row instead of moving
it, and `UpdateState` packs instead of cloning. The extra cost per event
is roughly the unpack's overhead over a clone, 15-40%, plus a pack, about
a third of a clone. That is about 0.6-0.7 of a clone of the row.

- Account handlers on large rows already do several row-sized copies per
  event (decoding the account, cloning mapped fields, building the patch),
  so the share stays modest.
- A cheap handler on a very large row, such as appending one swap to a
  175 KiB pool, pays most: about 50% per event.
- Small rows pay a microsecond or two.

A cache of recently written rows kept unpacked would win back the unpack
for hot rows. Keeping it in step with the table is subtle, though.
Instruction hooks change the state register after the event without
writing it back, so the cache cannot reuse the register and would need its
own copy. That copy costs the clone this change removed. It is left out
until production CPU profiles show it is needed.

## Why the output is unchanged

Every consumer of a row reads the `Value` that `unpack` returns, and that
value equals the one packed:

- Map iteration order is kept by writing fields in the order the map
  yields them and rebuilding the map from them in that order (`BTreeMap`
  sorts them anyway; an IndexMap keeps them).
- Strings are copied byte for byte. All-digit strings keep their exact
  digits, leading zeros included.
- Numbers keep their serde_json representation: `PosInt`, `NegInt` or
  `Float`, each written only if converting it back gives an equal
  `Number`, and otherwise kept as text.

Patches, creations, whole-entity resends and snapshots are built from
these values exactly as before. Unit tests check the round trip and the
serialized bytes for scalars at every encoding boundary (0, 239, 240,
`u64::MAX`, `i64::MIN`, `-0.0`, `1.0` against `1`, escapes, non-ASCII
text, digit strings with leading zeros and of 1,000 digits), for nested
and empty containers, and for arrays whose objects change shape. The full
interpreter and `arete-server` suites pass unchanged apart from the
replaced register-move tests.

## Remaining slices

1. **Server entity caches.**
   - Store `Arc<PackedRow>` in `SharedEntity`, so the 500-per-view caches
     and derived sorted views shrink by the same factor as the VM.
   - Merge patches by streaming into a new buffer, copying untouched
     subtrees as bytes (as the prototype did), so a merge no longer
     rebuilds trees. This needs container byte lengths in the format.
   - Serialize frames straight from the bytes through serde_json, which
     gives identical output.
   - Read sort and filter fields from the bytes.
   - `EntityFields::field` returns `&Value` today and would return an
     owned or `Cow` value.
   - Wire formatting either runs on the unpacked value or learns the
     packed form.

   This touches the subscriber and snapshot copy paths that other work is
   changing now, so it should follow that work.
2. **Snapshots without unpacking.**
   - Serialize packed rows directly into `StateTableSnapshot` and the
     cache dump, with a `Serialize` that emits what the `Value` would.
     The dump then no longer allocates every row's tree under the VM
     lock.
   - Restore packs rows straight from the decoded snapshot.
3. **Other `Value` stores in the VM.** These are `last_account_data` (the
   latest account per PDA, unbounded), queued account updates and
   instruction events, and resolver cache values. They are smaller
   per-entry but follow the same pattern.
4. **Optional: work on packed rows directly.** Handlers that only set
   scalars or append to arrays could apply to the bytes without unpacking
   the row. That would remove the CPU cost above for the hot-row case.
   Only worth doing if production profiles show it matters.

## Risks

- **CPU.**
  - Measured above: up to about 50% more per event for cheap handlers on
    very large rows, and modest elsewhere.
  - `VmCacheStats::state_table_row_bytes` shows the memory side in
    production, and the `duration_ms` on each event's canonical log line
    shows the CPU side.
- **API.**
  - `StateTable::data` is no longer public (breaking; no known users
    outside the crate).
  - `insert_with_eviction` takes `impl Borrow<Value>` (source-compatible).
  - `retain_state_register` is deprecated and does nothing.
- **Peak memory.**
  - A handler still works on a full `Value` of its row, so one unpacked
    row per VM is live at a time.
  - Snapshot dumps still unpack every row, as they cloned every row
    before, until slice 2.
- **Robustness.**
  - Unpacking trusts the bytes, which only `PackedRow::pack` writes into
    a private, immutable buffer. A malformed buffer would panic rather
    than return wrong data. There is no `unsafe` code, and UTF-8 is
    validated on unpack.
  - Recursion depth matches what cloning or dropping the same `Value`
    already needs.
  - Rows must be under 4 GiB.

## Method

- **Machine and toolchain.** Apple M2 (8 cores), 24 GB, macOS 15.0, rustc
  1.98.0. The machine was a shared workstation running other builds, with
  a load average of 10-60. Timings are therefore minimums or medians, and
  base and branch runs were interleaved.
- **Fixtures.** Rows are deterministic, from a seeded generator. The same
  shapes are in `interpreter/tests/state_table_memory.rs`:
  - *position*: a DEX liquidity position with `fees` (70 records of four
    u128-as-string fields), `rewards` (70 records holding two 2-element
    arrays) and `liquidity_shares` (70 u128 strings), plus `id`, `state`
    and `metrics` sections. About 40% of the large amounts are `"0"`.
  - *pool*: `recent_swaps` (100 records of 8 fields, with an 88-character
    signature and an address each) and `recent_liquidity` (100 records of
    5 fields), plus `id`, `state`, `config` and `metrics` sections.
  - *token*: 23 scalars in three sections.
- **Memory.** Memory is counted by a global allocator that tracks live
  requested bytes. The prototype also tracked bytes rounded to jemalloc's
  size classes and the live allocation count.
- **CPU in the prototype.** The prototype timed thread CPU time over
  batches of about 1 ms and took the fastest of 41.
- **CPU in the committed test.** The VM timings come from
  `cargo test --release -p arete-interpreter --test state_table_memory -- --ignored --nocapture`,
  taking the fastest of five 2,000-event rounds per run.
