# Account lifecycle integration

`AccountTombstone` is an authoritative chain deletion notification containing an
address, slot and write version. It describes an account's lifetime independently
of any application entity or view. All integer quantities use decimal u64 strings
on the wire. `account-deletion.json` in the canonical fixture bundle specifies the
DTO, mutation and source frame.

Ingestion integrations recognize authoritative deletion before normal owner or
layout filtering. Empty data alone does not establish deletion. They enforce a
durable slot/write-version watermark and explicitly map the account to live entity
export/primary keys; closing an input account does not implicitly delete historical
events or aggregates.

For VM-backed sources, hold the snapshot processing barrier and VM lock, call
`VmContext::discard_account(address)`, then call `delete_entity(bytecode, export,
key)` for each mapped entity. Call `Mutation::mark_account_position` with the
deletion's `AccountPosition::new(slot, write_version)` on each returned mutation,
then submit them in the ordered `MutationBatch` stream with
`SlotContext::account(slot, write_version)` and the snapshot guard before advancing
the source checkpoint. Custom
sources use `Mutation::delete` and attach the same account position.

Apply that marker to every address-keyed entity mutation authoritatively owned by
an account update, including the complete creation. Do not apply it to aggregate
mutations, instruction mutations, background resolver results or resends. Continue
to put the source position in `SlotContext`: it supplies `_seq` for recency and
client cursors, but it is not a lifetime clock. Generated ingestion uses
`SlotContext::account`, `SlotContext::instruction` and `SlotContext::resolver`;
custom producers use `SlotContext::with_domain`. Recency checkpoints are retained
independently per domain. Account `write_version`, instruction `txn_index` and
resolver counters are unrelated within one slot.
When a queued account update is replayed while handling an instruction, retain
the queued update's original account position on its mutations; do not substitute
the enclosing instruction's `txn_index`.

The VM clears state, identity indexes, buffered inputs, deferred writes, resolver
targets and whole-entity requests. The projector removes current cached and sorted
rows before publishing `delete`. Predicate or window departure remains `remove`.
Append tapes retain historical records according to their configured retention.

Account recreation initializes fresh VM state and emits a complete creation.
Linked VMs mark creation automatically; custom sources call `Mutation::mark_created`.
The projector publishes a complete `upsert`, replacing a subscriber's previous
copy even when deletion and recreation occur before it reads either frame.

Cached live/deleted account checkpoints and producer-local recency cursors survive
snapshots and are bounded to eight times the entity cache capacity per view. They
reject old account writes and stale tombstones without comparing them to
instruction or resolver activity. Sparse patches,
background resolver results and whole resends cannot restart a deleted lifetime;
a recreation is a complete replacement, not a merge. Ingestion integrations must
retain durable ordering beyond that bounded retention and validate bootstrap,
deletion, reconnect and recreation through their own source pipeline.

The first planned linked release containing this API is 0.29.0, with
`arete-solana-contracts` 0.1.0. Publication is pending; published 0.28.0 does not
contain these APIs or runtime fixes.
