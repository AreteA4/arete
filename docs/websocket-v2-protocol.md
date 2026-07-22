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
    "after": "1234:000000000010",
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
- `after`: exclusive `_seq` cursor. Its initial snapshot is incremental.
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
- `authoritative: false` means the snapshot is incremental and must be merged. A query containing `after` produces this form.
- `key` is present on keyed snapshots, including a completed empty state snapshot.

Receiver registration happens before snapshot capture for state, list, append, and derived-source subscriptions. Updates published while a snapshot is being built or sent remain pending for live delivery after the snapshot. The implementation does not use timing sleeps for this handoff.

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

Stable protocol codes include `malformed-message`, `invalid-subscription`, `invalid-unsubscription`, `duplicate-subscription-id`, `unknown-subscription-id`, and `subscription-rejected`. Authentication, quota, and rate-limit errors keep their existing codes and use the same v2 envelope.

## Conformance Fixtures

Deterministic shared examples live in `tests/fixtures/websocket-v2`. The manifest covers keyed state, independent list windows, exact filters, authoritative multi-batch and empty snapshots, query-scoped remove, global delete, incremental snapshots, reconnect replacement, and error envelopes.
