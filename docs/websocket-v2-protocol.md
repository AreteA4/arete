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

For ordinary list and append views, full snapshots are ordered by `_seq` descending and incremental snapshots by `_seq` ascending. Entity key is the deterministic tie breaker. Derived views retain their declared sort order. An entity whose sort field is missing or `null` cannot be ranked, so it sorts after every entity that has one in both directions — `desc` reverses only the comparison between two present values — and it never displaces a ranked entity from a `take` window. Filters run before `skip` and `take`; the same filtered window determines both snapshot rows and live membership.

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
  },
  "wholeEntities": true
}
```

The echoed query is the effective query. For example, a derived view can fill in its declared limit as `take` when the request omitted one.

`wholeEntities: true` states that this subscription follows the whole-entity rules under [Live Frames](#live-frames): the server sends every key whole, as a snapshot row or `upsert`, before any `patch` for it. Servers that predate the rules omit the field. A client drops a patch for a key it does not hold only when the acknowledgement carries it (see [Partial entities](#partial-entities)). Clients ignore acknowledgement fields they do not recognise, so the field is safe to send to older clients.

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
replay request, instead of one row per surviving entity. See the retention
notes below for the one case a restart can still duplicate.

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

One duplication case remains. A restart that resumes the stream from the
snapshot's watermark re-decodes the transactions in the overlap, and those
events are appended to the tape a second time under fresh offsets. A
consumer reading across such a restart sees them twice, and cannot tell
them apart. Delivery is exactly once within one server lifetime.

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

A source change to a key the client already holds is forwarded as `patch`. The server sends the whole entity as `upsert` whenever a key becomes a member of the subscription as far as the server knows:

- when it enters the query window — it starts passing the filters, or a change to it or to another entity moves it into the `take`/`skip` range;
- on its first change after a snapshot that `snapshotLimit` truncated before it, or after a subscription with the snapshot disabled — those keys are in the window but the client was never sent them;
- on every change to a derived view, whose source patches do not apply to it.

A key that only moves position inside the window is not resent: the client orders members from the `sort` in `subscribed`. A key the client was never sent gets no `remove` or `delete` when it leaves the window.

### Partial entities

A `patch` for a key the client does not hold is not an entity. When the subscription's acknowledgement carries `wholeEntities: true`, clients discard such a patch without storing it, tracking its `seq`, or granting it membership; the entity appears with the next full `upsert`, which the rules above guarantee for every key the server has not sent. Without the field the server may send a key's first change as a `patch` (after a truncated or disabled snapshot), so clients store that patch as the entity, as they always have. A `remove` or `delete` for a key the client does not hold has nothing to act on.

The guarantee covers keys the server has not sent. A client that drops an entity it was sent, for example to stay within a local entry limit, still counts as holding it for the server, which keeps sending patches and sends no `upsert` until the key leaves and re-enters the window or the subscription is re-established. The client discards those patches, so the entity stays missing locally until then. Keeping such limits above the subscription's size is the client's responsibility, and the SDKs report each patch they drop for a key they evicted.

Frames that carry an `offset` are exempt. They are records from a [replayable append view](#replayable-append-views): events delivered verbatim in offset order, never promoted to `upsert`. The server keeps only each entity's latest state, not its state as of a given offset, so it has nothing faithful to substitute, and a consumer resuming from a cursor already holds whatever came before it. Clients apply them as received.

The server holds itself to the same rule. A source patch carries only the fields that changed, so for a key the server's bounded entity cache does not hold it is stored only when the state machine that produced it marks it as the entity's creation. Any other patch for such a key — one the cache evicted, however long ago, or one the state machine kept across a restore — is not stored. The server then asks the state machine for the whole entity, which follows with the state machine's next batch of changes, whether or not that batch changes the key: the key returns to snapshots and windows, and clients receive it as an `upsert`. That `upsert` is not a change: it carries the `seq` of the key's latest change, and the key keeps that position in recency (`_seq`) order instead of taking the batch's. Until then the key is out of the view's snapshots. If the state machine no longer holds the entity it has nothing to send; its next change to the key starts the entity afresh and is stored as its creation. A state subscription that holds it keeps receiving its patches and is never sent `remove` for an eviction. A list view's cache bound is also the extent of its windows, so an evicted key leaves list windows the way a key past `take` does, with a `remove`, and re-enters as an `upsert`. Such a patch never adds the key to a derived view either; a derived view that still holds the whole entity (derived views are bounded by sort position, not recency) merges the change into its copy. Snapshot rows and `upsert` data are always whole entities, as far as the server's state machine holds them: it keeps a bounded number of entities too (2,500 per entity type by default), and an entity it has dropped restarts from the fields its next update sets.

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

Deterministic shared examples live in `tests/fixtures/websocket-v2`. The manifest covers keyed state, independent list windows, exact filters, authoritative multi-batch and empty snapshots, query-scoped remove, global delete, incremental snapshots, reconnect replacement, error envelopes, replay cursors and gaps, and patches for keys the client does not hold with and without `wholeEntities`.
