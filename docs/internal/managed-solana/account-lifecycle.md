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
key)` for each mapped entity. Submit the returned mutations in the ordered
`MutationBatch` stream with the deletion's `SlotContext` and snapshot guard before
advancing the source checkpoint. Custom sources use `Mutation::delete` directly.

The VM clears state, identity indexes, buffered inputs, deferred writes, resolver
targets and whole-entity requests. The projector removes current cached and sorted
rows before publishing `delete`. Predicate or window departure remains `remove`.
Append tapes retain historical records according to their configured retention.

Account recreation initializes fresh VM state and emits a complete creation.
Linked VMs mark creation automatically; custom sources call `Mutation::mark_created`.
The projector publishes a complete `upsert`, replacing a subscriber's previous
copy even when deletion and recreation occur before it reads either frame.

Cached deletion/recreation barriers survive snapshots and are bounded to eight
times the entity cache capacity per view. Sparse patches and whole resends cannot
restart a deleted lifetime. Ingestion integrations must retain durable ordering
beyond that bounded retention and validate bootstrap, deletion, reconnect and
recreation through their own source pipeline.

The first planned linked release containing this API is 0.29.0, with
`arete-solana-contracts` 0.1.0. Publication is pending; published 0.28.0 does not
contain these APIs or runtime fixes.
