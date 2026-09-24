# WebSocket Protocol v2

Protocol v2 is the server wire contract for view subscriptions. It is intentionally breaking: the server does not accept legacy bare subscriptions or `view:key` unsubscriptions.

## Subscribe

Every subscribe request has a client-selected opaque `subscriptionId`, the fixed `protocolVersion`, one canonical `query`, and explicit snapshot options:

```json
{
  "type": "subscribe",
  "protocolVersion": 2,
  "subscriptionId": "rounds:page-1",
  "query": {
    "view": "OreRound/latest",
    "key": "optional-key",
    "partition": "optional-partition",
    "filters": {
      "state.status": "open"
    },
    "take": 10,
    "skip": 0,
    "after": "0f8c2b31-6a4e-4f0b-9a77-1d2c3e4f5a6b:4211",
    "snapshotLimit": 100
  },
  "snapshot": {
    "enabled": true
  }
}
```

Only `query.view` is always required. A non-derived state view also requires `query.key`. Omitted `snapshot` defaults to `{ "enabled": true }`.

`subscriptionId` is opaque to the server. It must be 1 to 128 bytes, have no leading or trailing whitespace, and contain no control characters. It does not need to be a UUID. IDs must be unique among active subscriptions on one connection; the same ID may be reused after unsubscribe or on a replacement connection.

Unknown fields are rejected. The canonical query fields are exactly:

- `view`: registered server view ID.
- `key`: exact entity-key match.
- `partition`: exact match against the entity's reserved top-level `_partition` value.
- `filters`: JSON exact-match predicates keyed by dot path. Matching is case-sensitive and type-sensitive. Missing paths do not match.
- `take`: positive size of the live query window.
- `skip`: number of matching ordered entities omitted before `take`.
- `after`: resume cursor. On an append view backed by the event journal this is a replay offset (see [Replayable append views](#replayable-append-views)); on every other view it is an exclusive `_seq` cursor whose initial snapshot is incremental.
- `snapshotLimit`: positive cap applied only to initial snapshot rows. It does not alter live `take`/`skip` membership.

For ordinary list and append views, full snapshots are ordered by `_seq` descending and incremental snapshots by `_seq` ascending. Entity key is the deterministic tie breaker. Derived views retain their declared sort order. Filters run before `skip` and `take`; the same filtered window determines both snapshot rows and live membership.

## Acknowledgements

The server acknowledges a valid subscription before sending its snapshot:

```json
{
  "protocolVersion": 2,
  "subscriptionId": "rounds:page-1",
  "op": "subscribed",
  "query": {
    "view": "OreRound/latest",
    "take": 10,
    "skip": 0
  },
  "mode": "list",
  "sort": {
    "field": ["id", "roundId"],
    "order": "desc"
  }
}
```

The echoed query is the effective query. For example, a derived view can fill in its declared limit as `take` when the request omitted one.

## Snapshots

Every snapshot batch includes its subscription and snapshot identities:

```json
{
  "protocolVersion": 2,
  "subscriptionId": "rounds:page-1",
  "snapshotId": "0f7e9f1d-5b55-4b5a-a87f-b9b55e59e64b",
  "authoritative": true,
  "mode": "list",
  "entity": "OreRound/latest",
  "op": "snapshot",
  "data": [
    { "key": "100", "data": { "id": { "roundId": "100" } } }
  ],
  "complete": true
}
```

All batches for one snapshot share `snapshotId`, `subscriptionId`, and `authoritative`. `complete: false` means another batch follows. Exactly one final batch has `complete: true`, including an empty snapshot.

- `authoritative: true` means the completed snapshot replaces all local state for this subscription.
- `authoritative: false` means the snapshot is incremental and must be merged. The initial snapshot for a query containing `after` produces this form.
- `key` is present on keyed snapshots, including a completed empty state snapshot.

If a latest-state subscription falls behind the server's bounded live bus, the
server re-registers its receiver and sends a full authoritative recovery
snapshot. Recovery is authoritative even when the original query contains
`after`, so deletions skipped during the gap prune stale client membership.
Because `snapshotLimit` applies only to the initial transfer, it does not
truncate this recovery snapshot.

Receiver registration happens before snapshot capture for state, list, append, and derived-source subscriptions. Updates published while a snapshot is being built or sent remain pending for live delivery after the snapshot. The implementation does not use timing sleeps for this handoff.

## Replayable append views

When the server runs with an event journal (`ARETE_JOURNAL_ENABLED=true`), an
append view is delivered as an event tape rather than a membership
projection. Every retained event is replayed in order, exactly once per
replay request, instead of one row per surviving entity.

Each live frame on such a view carries an `offset`: a dense, monotonic,
per-view cursor assigned when the event is retained.

```json
{
  "protocolVersion": 2,
  "subscriptionId": "trades",
  "entity": "Trade/append",
  "op": "patch",
  "key": "pool1",
  "offset": 4211,
  "seq": "381471241:000000000007",
  "data": { "amount": 125 }
}
```

`offset` is the position within the tape. The cursor you send back as `after`
is `{epoch}:{offset}`, where the epoch comes from the acknowledgement's
`replayWindow`.

`seq` is not usable as a replay cursor: its second component is the
transaction index, so every event decoded from one transaction shares a
`seq`. Offsets are scoped to one view and are not comparable across views.

The acknowledgement advertises the window the view can still serve:

```json
{
  "protocolVersion": 2,
  "subscriptionId": "trades",
  "op": "subscribed",
  "mode": "append",
  "replayWindow": {
    "epoch": "0f8c2b31-6a4e-4f0b-9a77-1d2c3e4f5a6b",
    "earliest": 3200,
    "next": 4212
  }
}
```

`earliest` is the oldest retained offset; `next` is the offset the next event
will take, so a consumer holding `next - 1` is fully caught up. To resume at
offset 4211 you would send `after: "0f8c2b31-...:4211"`.

The official SDKs assemble this for you: every update from a replayable view
carries a `cursor` field holding exactly that string, and each SDK resumes
from the last one it delivered when a dropped socket reconnects. Store the
cursor in the same transaction as the data it came with, and a restart picks
up where the write did.

### Epochs

The epoch identifies one tape lifetime. Offsets restart at zero whenever a
tape is built without restoring one — snapshots disabled, a rejected or
corrupt blob, any cold start — and because offsets are dense, an old cursor
would otherwise land inside the new window and replay unrelated events as a
continuation of the stream you were reading.

A cursor whose epoch is not the view's current epoch is refused with
`cursor-epoch-changed`. Discard it and resubscribe without `after`. A restore
from a snapshot adopts the snapshot's epoch, so cursors held across a normal
restart stay valid.

`after` is exclusive. A cursor below `earliest` is refused with
`cursor-expired`, which repeats the window so the consumer knows what it
lost:

```json
{
  "type": "error",
  "code": "cursor-expired",
  "retryable": false,
  "replayWindow": { "epoch": "0f8c2b31-...", "earliest": 3200, "next": 4212 }
}
```

Recover by resubscribing **without** `after`, which replays the whole
retained window. Do not resubscribe with `after` set to `earliest`: because
`after` is exclusive that skips the oldest retained record.

A cursor at or above `next` is refused with `cursor-unknown` rather than
treated as caught up — accepting an offset the view has never issued would
suppress delivery until its offsets reached that value. An `after` that is
not an `{epoch}:{offset}` cursor at all is refused with `invalid-cursor`.

### Gaps

If a restore hydrates state but starts the stream live, events between the
retained tape and the first live append are lost. Offsets stay dense across
that hole, so the window alone would look continuous. Replaying across it is
refused with `replay-gap`, and `replayWindow.gapAfter` marks the last offset
before the hole. Resubscribe without `after` to accept the gap, or from a
cursor after it.

If delivery falls behind the server's fan-out buffer, the gap is reported as
`replay-lagged` and **delivery on that subscription stops**. The error
carries `recoverFrom`: the cursor for the last record delivered *before* the
gap.

```json
{
  "type": "error",
  "code": "replay-lagged",
  "recoverFrom": "0f8c2b31-...:4180"
}
```

Unsubscribe, then resubscribe with `after` set to `recoverFrom` to replay
the skipped records; they are recoverable as long as they are still
retained. `recoverFrom` is absent when nothing had been delivered yet, in
which case resubscribe without `after`.

Delivery stops rather than continuing because frames from after the gap
would advance the consumer's checkpoint past the skipped records, making
them unrecoverable. The subscription stays registered until you unsubscribe,
so reuse of the same `subscriptionId` requires unsubscribing first.

### Query options and retention

`take`, `skip` and `snapshotLimit` describe a membership window and do not
apply to a tape; a replayable subscription requesting any of them is refused
with `invalid-subscription`. `key`, `partition` and `filters` are honoured,
on replayed and live records alike.

Retention is bounded by bytes, record count and age, whichever bites first.
The byte bound is the one to budget against: frame size is stack-dependent,
so a record count cannot be reasoned about against a memory limit. The
retained tape is captured in the state snapshot, so the advertised window
survives a normal restart.

The tape is not the only journal-related allocation. While a replay is
draining, the server buffers live frames for that subscription so a busy view
cannot lap it — bounded, but per replaying subscription rather than per view,
so it scales with client count. Payload bytes are shared with the retained
records; the overhead is the envelopes. A restart that has every client
reconnect at once is where that peaks.

A restart that was not clean retires cursors. A snapshot taken at shutdown is
exact, so cursors held across it stay valid; a periodic snapshot is behind
what was published, and restoring one rewinds the tape below offsets that
already went out. Those offsets get re-issued for different records, so the
restore mints a new epoch and every cursor from before it is refused with
`cursor-epoch-changed` rather than silently served at the wrong place.

A stream that resumes — after a restart, or after a reconnect inside one
process — re-decodes the slot it restarted at, so the events the tape already
holds for that slot arrive again. They are recognised and not retained a
second time: an event's decode site is a pure function of the transaction, so
it reproduces exactly, while the payload cannot be compared because events
carry a wall-clock timestamp. Events from that slot which had *not* been
retained when the stream stopped are new and do land, so the overlap is
deduplicated rather than skipped.

Two sources are exempt. Events derived from a resolver result are built
outside the decode path and carry no decode site, and on the scheduler path
their position comes from a process-local counter rather than from the
stream — neither half of that identity reproduces, so those events can still
be retained twice across a resume. Account-driven events have no decode site
either; a re-delivered account write is normally dropped before it reaches
the tape, by a version check that is itself bounded in capacity, so it is not
an absolute guarantee.

## Live Frames

All live frames include `protocolVersion` and `subscriptionId`:

```json
{
  "protocolVersion": 2,
  "subscriptionId": "rounds:page-1",
  "mode": "list",
  "entity": "OreRound/latest",
  "op": "upsert",
  "key": "101",
  "data": { "id": { "roundId": "101" } },
  "seq": "1235:000000000001"
}
```

Source patches remain `patch`. A full entity entering or moving inside a query window is `upsert`.

`remove` and `delete` are deliberately different:

- `remove`: evict the key from this subscription only. The entity left a filter or `take`/`skip` window but still exists in the source view.
- `delete`: the entity was deleted from the source view. Consumers may remove it from every local query for that source.

Derived live frames carry the source envelope's `seq` on both membership removals and full upserts. If a source envelope has no sequence, the derived frame may use the selected entity's `_seq`; otherwise `seq` is omitted because the server has no ordering cursor to preserve.

## SDK Examples

The SDKs construct protocol v2 subscriptions. Applications normally use generated view APIs instead of sending the JSON envelopes directly.

### TypeScript

Each options object defines an independent query. These two streams can coexist on the same view without sharing list membership:

```ts
const rounds = session.stacks.ore.views.OreRound.latest;

const firstPage = rounds.watch({ take: 10 });
const secondPage = rounds.watch({ take: 10, skip: 10 });

for await (const update of firstPage) {
  if (update.type === 'remove') {
    console.log(`${update.key} left the first page`);
  }
  if (update.type === 'delete') {
    console.log(`${update.key} was deleted from OreRound/latest`);
  }
}
```

Equivalent normalized queries share one wire subscription and are reference-counted. Breaking the final consuming loop releases that subscription.

### React

React hooks read exact query membership and keep committed data visible while an authoritative reconnect snapshot is loading:

```tsx
const firstPage = arete.views.OreRound.latest.use({ take: 10 });
const secondPage = arete.views.OreRound.latest.use({ take: 10, skip: 10 });

if (firstPage.isLoading) return <p>Loading rounds...</p>;

return (
  <>
    {firstPage.isRefreshing && <p>Refreshing first page...</p>}
    <RoundList rounds={firstPage.data ?? []} />
    <RoundList rounds={secondPage.data ?? []} />
  </>
);
```

### Rust

Rust stream builders put filters, windows, partitions, cursors, and snapshot limits on the wire. Streams are lazy, so polling starts the subscription:

```rust
let mut open_orders = Box::pin(
    a4.views.order.list()
        .watch()
        .filter("state.status", "open")
        .take(10)
        .skip(20)
        .partition("solana-mainnet")
        .with_snapshot_limit(100),
);

while let Some(update) = open_orders.next().await {
    match update {
        Update::Upsert { key, data } => println!("upserted {key}: {data:?}"),
        Update::Patch { key, data } => println!("patched {key}: {data:?}"),
        Update::Remove { key } => println!("{key} left this query"),
        Update::Delete { key } => println!("{key} was deleted globally"),
    }
}
```

The Rust SDK retains a stable opaque subscription ID across reconnects, atomically replaces membership after a complete authoritative snapshot, and sends `unsubscribe` when the final equivalent stream is dropped.

## Unsubscribe

Cancellation is by `subscriptionId`, not by view and key:

```json
{
  "type": "unsubscribe",
  "protocolVersion": 2,
  "subscriptionId": "rounds:page-1"
}
```

Success is acknowledged:

```json
{
  "protocolVersion": 2,
  "subscriptionId": "rounds:page-1",
  "op": "unsubscribed"
}
```

## Errors

Protocol and subscription errors are non-fatal unless explicitly marked otherwise. They include `subscriptionId`; it is `null` only when the malformed request did not contain a usable ID.

```json
{
  "type": "error",
  "protocolVersion": 2,
  "subscriptionId": "rounds:page-1",
  "error": "duplicate-subscription-id",
  "message": "subscriptionId is already active on this connection",
  "code": "duplicate-subscription-id",
  "retryable": false,
  "fatal": false
}
```

Stable protocol codes include `malformed-message`, `invalid-subscription`, `invalid-unsubscription`, `duplicate-subscription-id`, `unknown-subscription-id`, `subscription-rejected`, `cursor-expired`, `cursor-epoch-changed`, `cursor-unknown`, `invalid-cursor`, `replay-gap`, and `replay-lagged`. Authentication, quota, and rate-limit errors keep their existing codes and use the same v2 envelope.

## Conformance Fixtures

Deterministic shared examples live in `tests/fixtures/websocket-v2`. The manifest covers keyed state, independent list windows, exact filters, authoritative multi-batch and empty snapshots, query-scoped remove, global delete, incremental snapshots, reconnect replacement, and error envelopes.
