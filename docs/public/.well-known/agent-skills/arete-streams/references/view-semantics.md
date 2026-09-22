# View Semantics

Read this reference when choosing view operations, implementing updates, or debugging state convergence.

## View Modes

- A `state` view represents at most one entity for a generated typed key.
- A `list` view represents an ordered collection.
- Custom views are list or state views with stack-defined filtering, sorting, or projection.

The exact server identifier is `<Entity>/<view>`. Generated SDKs expose language-native accessors for the same identifier.

## Operations

- `use`: emits the merged current entity after patches; filters `remove` and `delete` from the value stream.
- `watch`: emits raw `upsert`, `patch`, `remove`, and `delete` operations.
- `watchRich`: emits created/updated/removed/deleted values with before/after state where available.
- `get`: opens or reuses an equivalent lease and waits for its snapshot.
- `getSync`: reads an already active equivalent subscription without waiting.
- `getOne`: returns the first item from a list view.

Dropping or breaking a stream releases its lease. Equivalent normalized queries share a wire subscription and are reference-counted.

## Update Taxonomy

- `upsert`: a complete entity entered or changed in this query window.
- `patch`: a partial update merged into the stored entity. Declared append paths concatenate arrays.
- `remove`: the entity left this query's current window; it may still exist globally.
- `delete`: the entity was deleted from the source view.

Never treat `remove` as global deletion.

## Snapshots and Cursors

Snapshot batches are staged until complete. A complete authoritative snapshot replaces membership for that query. An incremental cursor snapshot merges and does not prune unseen keys.

Query identity includes the normalized query and snapshot behavior. Filters, keys, pagination windows, partitions, cursors, and snapshot options can therefore create different leases over the same view.

Use an `after` cursor only as an opaque sequence returned by the platform. Do not parse or synthesize one.

## Ordering and Windows

The server declares view ordering. A `take`/`skip` query defines live window membership, not merely a client-side slice. An item moving out of the window produces `remove`; another item can enter with `upsert`.

Do not re-sort unless the application intentionally needs a different presentation order.

## Absence and Empty Results

- A settled empty list is different from a list that has not received its first snapshot.
- A missing state entity is different from no active state subscription.
- React exposes status fields for these distinctions.
- Rust may use nested `Option`; Python uses an unset sentinel separately from `None`.

Preserve those states in application loading and error logic instead of collapsing them with truthiness checks.
