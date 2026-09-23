# WebSocket list-delivery load baseline

This records how latest-state list delivery behaves over a real socket while
each live list update is still its own frame. It is local evidence for
comparing one implementation with the next. None of the wall-clock numbers
here is an SLO or a capacity promise.

It was recorded again after two harness fixes. `publishedUpdates` now leaves
out updates still queued for the projector when a deadline cuts a burst
short. A client that the server drops after it converges now reconnects under
its scenario's policy, and must hold the final state again from a fresh
snapshot.

The harness is `rust/arete-server/src/websocket/load_tests.rs`. It starts the
real `Projector`, `BusManager`, `EntityCache`, `ViewIndex`, and
`WebSocketServer` on an ephemeral port. It publishes deterministic
`MutationBatch`es and reads each socket until the client's merged state equals
the publisher's model.

## Baseline identity

| | |
| --- | --- |
| Delivery code | `3039d71b6438394fba086969bb93d4d9f95f4be6` (lag recovery, PR #248) |
| Measured at | Harness commit `b07d3d50`, on merge commit `b491e798`. `git diff 3039d71b b491e798 -- rust interpreter arete-hash Cargo.lock` is empty, so the delivery code is exactly `3039d71b`. |
| Production changes | None. `server.rs` gains only a `#[cfg(test)]` `DeliveryProbe` that mirrors existing `WsMetrics` calls. |
| Measured | 2026-09-23 19:15–19:29 UTC: three debug runs, then three release runs |
| Machine | Apple M2 (8 logical CPUs), 24 GB, macOS 15.0 (Darwin 24.0.0 arm64) |
| Toolchain | rustc 1.98.0 (88d9e12ae 2026-08-18) |
| Runtime | Server tokio runtime with 4 workers; clients on a separate 4-worker runtime |
| Background load | A developer workstation running a browser and other agent sessions. The 1-minute load average at run start was 2.6–7.4 for the debug runs and 6.9–11.3 for the release runs. |

## Commands

```bash
# Always-run correctness gate (part of `cargo test -p arete-server websocket:: --lib`)
cargo test -p arete-server websocket::load_tests::smoke -- --nocapture

# The four capacity profiles: the canonical baseline command
cargo test -p arete-server websocket::load_tests::profiles -- --ignored --nocapture

# The same, keeping summaries, log, and machine context under target/websocket-delivery-load/
scripts/run-websocket-delivery-load.sh            # debug, canonical
scripts/run-websocket-delivery-load.sh --release  # optimized, supplementary
```

Each scenario prints one JSON line that begins with `{"scenario":`.

## Delivery configuration under test

Every profile uses one coalesced configuration, sized so that a reader that
keeps draining never lags:

| Setting | Value | Source |
| --- | --- | --- |
| `collectionCoalesceMs` | 100 | `ARETE_WS_COLLECTION_COALESCE_MS` |
| `listBusCapacity` | 16,384 frames | `ARETE_WS_LIST_BUS_CAPACITY` |
| `messageQueueSize` | 2,048 messages per client | `ARETE_WS_MESSAGE_QUEUE_SIZE` |
| Snapshot batches | 50 rows, then 100 per frame | `EntityCacheConfig` defaults |
| Projector channel | 1,024 batches | `runtime.rs` |
| Usage emitter | Attached (in-memory channel) | A metered server emits usage for every message |
| `cacheEntitiesPerView` | Seed row count (3,000 or 10,000) | Harness choice; see below |

The entity cache is sized explicitly so the list can hold every seeded row: a
list with thousands of initial rows. **A runtime with the default cache cannot
produce that shape.** `runtime.rs` builds `EntityCache::new()`, which retains
500 entities per view. With that cap, a 3,000-key list does not have a 3,000-row window.
Membership churns through LRU eviction instead (a `remove` plus an `upsert` per
re-entry). Because `EntityCache` stores a cache-miss patch as the whole entry,
a sparse patch to an evicted key becomes a partial row. These profiles do not
measure that path; it needs its own decision and scenario.

## Workload

Every run generates the same fixtures:

- **Rows.** Token-account-shaped entities: `id.address`, a `state` object
  (`mint`, `owner`, `amount`, `delegate`, `state`, `isNative`,
  `delegatedAmount`, `closeAuthority`), `tokenAmount` (`decimals`,
  `uiAmountString`), `lastUpdatedSlot`, and the injected `_seq`. The view marks
  `state.amount`, `state.delegatedAmount`, and `lastUpdatedSlot` as wide
  integers, so they are strings on the wire. Addresses are base58 SHA-256
  digests of fixed labels, so no fixture names a real account. A seeded row is
  about 429 JSON bytes.
- **Source updates.** A sparse balance change: `state.amount`,
  `tokenAmount.uiAmountString`, and `lastUpdatedSlot`, about 113 JSON bytes.
  Each update is one `MutationBatch` with one mutation and a unique,
  increasing `SlotContext`, so every update has a unique `_seq`.
- **Seeding.** Every row is published and flushed through the projector before
  any client subscribes, so each subscriber's initial snapshot carries all of
  them.
- **Pacing.** The burst is paced at 10,000 source updates per second, released
  in 10 ms ticks of 100. `ARETE_WS_LOAD_SOURCE_RATE=0` publishes as fast as
  the projector accepts. Pacing shapes load; it never
  decides an outcome.

## Scenarios

| Scenario | JSON name | Rows | Source updates | Key pattern | Subscribers | Reader |
| --- | --- | ---: | ---: | --- | ---: | --- |
| Hot keys | `hot-keys` | 3,000 | 30,000 | 80% of updates on the first 300 keys, 20% on any key (deterministic splitmix64 draw) | 1 | Continuously draining |
| Unique burst | `unique-burst` | 10,000 | 10,000 | Every key exactly once, in key order | 1 | Continuously draining |
| Slow reader | `slow-reader` | 3,000 | 30,000 | Hot keys | 1 | Stops reading for 2,000 ms at burst start, then drains |
| Fan-out | `fan-out` | 3,000 | 30,000 | Hot keys | 16 | Continuously draining |

A subscriber that observes an explicit close reconnects and resubscribes once
per close, up to eight times, as an SDK would. Its fresh authoritative snapshot
must still reach the exact final state. The smoke test (128 rows, 2,000 hot-key
updates, 2 subscribers) forbids disconnects.

The server runtime has 4 worker threads. In `profiles`, clients run on a
separate 4-thread runtime so decoding never occupies a server worker; they
still share the machine's CPUs.

Local profiling overrides (each is recorded in the summary's
`config.overrides`): `ARETE_WS_LOAD_HOT_KEYS`, `ARETE_WS_LOAD_HOT_UPDATES`,
`ARETE_WS_LOAD_UNIQUE_UPDATES`, `ARETE_WS_LOAD_FANOUT_SUBSCRIBERS`,
`ARETE_WS_LOAD_SLOW_READER_PAUSE_MS`, `ARETE_WS_LOAD_SOURCE_RATE`,
`ARETE_WS_LOAD_TIMEOUT_SECS` (default 180), `ARETE_WS_LOAD_SERVER_THREADS`,
`ARETE_WS_LOAD_CLIENT_THREADS`, and `ARETE_WS_LOAD_ONLY` (comma-separated
scenario names). A baseline or comparison run sets none of them.

## How a run ends

Nothing sleeps to decide an outcome:

1. Seed and burst each end with a projector flush marker, so the harness knows
   the cache and bus hold every update.
2. A client has converged when its incrementally maintained mismatch count
   reaches zero against the model, and its current connection has completed a
   snapshot. Every update carries a unique `_seq`, so no earlier frame can
   complete the final state by accident.
3. The runner then waits, up to the deadline, until the server's own
   `coalesced_updates` counter equals source updates × subscribers. That means
   every subscription has flushed everything it received. Only then does it
   shut the server down.

   A lag or a stopped delivery discards pending updates, and once the server
   reports either, that count can never be reached. The runner then waits a
   one-second grace period instead. The summary records the path in
   `quiescence`: `flushed`, `immediate` (uncoalesced delivery, where
   convergence already ends the traffic), or `grace`.
4. The runner signals `stopping` and shuts the server down. Each converged
   client keeps reading until the server's close frame, and everything it
   reads after converging is counted as `trailingMessages`. If such a frame
   moves the state away from the model, the run fails. A close before
   `stopping` is the server dropping that client, and is recorded as a
   disconnect. A scenario that allows reconnecting then reconnects the
   client, which must hold the final state again from a fresh snapshot; the
   runner waits for every client to hold it again before it stops the server.
5. Every wait is bounded by the scenario deadline (180 s). A deadline produces
   a failing summary with mismatch samples, not a hang.

A run fails on any protocol error or abrupt socket failure. It also fails on
duplicate terminal state (a second `subscribed`, a snapshot that continues
after completing, or interleaved snapshots). It fails on a disconnect the
scenario forbids, on a final digest that differs from the model, or on a
server that does not stop cleanly. Wall-clock values are reported and never
asserted.

## Summary fields

All counts are summed over subscribers unless a field says otherwise. Each
summary also carries a `clients` array with the same counters for each
subscriber.

| Field | Meaning |
| --- | --- |
| `sourceUpdates` | Burst mutations the workload defines (seeding excluded) |
| `publishedUpdates` | Burst mutations the projector applied; below `sourceUpdates` only when the deadline cut the burst short, and then updates still queued for the projector are not counted |
| `uniqueKeys` | Distinct keys the burst touched |
| `deliveredOperations` | Live entity operations received (`upsert`, `patch`, `remove`, `delete`). One entity frame each while live frames are unbatched; one item each if they are batched; never a batch envelope |
| `coalescedSourceUpdates` | Server `arete.ws.collection.coalesced_updates`: source updates consumed by timed flushes, summed over subscriptions |
| `server.coalescedFlushes` | Server `arete.ws.collection.coalesced_flushes` |
| `websocketMessages` | Physical text or binary WebSocket messages received, of every kind |
| `liveMessages`, `snapshotMessages`, `controlMessages` | The same messages split by kind |
| `trailingMessages` | Messages received after the client already held the exact final state |
| `wireBytes` | Payload bytes as received (gzip frames at compressed size), excluding WebSocket framing |
| `decodedBytes` | JSON bytes after decompression |
| `liveWireBytes`, `snapshotWireBytes` | `wireBytes` split by kind |
| `snapshotRows` | Rows carried by snapshot frames (initial and recovery) |
| `frameEncodings` | Text, uncompressed binary JSON, and gzip binary message counts |
| `resnapshots` | Lag-recovery snapshots completed by clients; `server.resnapshots` is the server's count |
| `server.lagEvents`, `server.droppedUpdates` | Server `arete.ws.subscription.lagged` and `...dropped_updates` |
| `server.deliveryStopped` | Server `arete.ws.delivery.stopped`, by reason |
| `server.messagesSent` | Server `arete.ws.messages.sent` |
| `usage` | Totals from `WebSocketUsageEvent`s (`UpdateSent`, `SnapshotSent`, connections) |
| `protocolErrors`, `disconnects`, `reconnects` | Client-observed |
| `outcomes` | Per client: `drained`, `recovered` (lag resnapshot), `reconnected` (explicit close, then a fresh subscription, before or after converging), `closed-after-converging` (the server dropped it after it converged, in a scenario that forbids reconnecting; also a failure), or `failed` |
| `firstLiveUpdateMs` | Slowest subscriber's first live operation, measured from the first burst publish |
| `drainMs` | Slowest subscriber's arrival at the exact final state, measured from the first burst publish |
| `drainAfterPublishMs` | `drainMs` minus the time the projector took to apply the whole burst |
| `perSubscriberSourceUpdate` | Delivered operations, live messages, and live bytes ÷ (`sourceUpdates` × `subscribers`) |
| `perDeliveredOperation` | Live messages and live bytes ÷ `deliveredOperations` |
| `expectedStateDigest`, `finalStateDigest` | SHA-256 over sorted keys and canonical (key-sorted) JSON rows. `finalStateDigest` is null unless every subscriber converged to the same digest |

The smoke test also asserts exact accounting:
- server `messagesSent` equals client `websocketMessages`;
- usage messages and bytes equal client messages and wire bytes;
- snapshot rows and frames match the seed;
- `coalescedSourceUpdates` equals source updates × subscribers.

In every profile, a subscriber that drains without a disconnect or recovery
must satisfy `uniqueKeys ≤ deliveredOperations ≤ 2 × sourceUpdates`. The upper
bound is not `sourceUpdates`. A flush sends a key when it has a pending update,
and also when the shared cache already holds a value the subscription has not
yet read from its bus (see the findings below). Each source update can cause at
most one of each.

## Results

Each run's summaries, log, and context were written by
`scripts/run-websocket-delivery-load.sh` to
`target/websocket-delivery-load/rerun-{debug,release}-{1,2,3}.*`. The raw
summaries for debug run 1 (canonical) and release run 1 are at the end of this
document.

Every debug run exits 101, because fan-out did not converge before its
deadline (finding 5). Every release run exits 0. In all six runs, every other
scenario converged to the expected digest with no protocol errors. Two
expected digests hold for every scenario and run:
`sha256:ee4d907d…6056c` (3,000-row scenarios) and `sha256:4476a15f…532de`
(unique burst). Server `messagesSent` equals client `websocketMessages` in
every scenario that converged.

### Debug build: the canonical baseline

#### Counts from run 1

| Scenario | Subs | Source updates | Applied | Unique keys | Delivered ops | Coalesced source updates | Flushes | WS messages (live / snapshot / trailing) | Wire bytes | Recovery snapshots | Disconnects | Outcomes |
|---|---:|---:|---:|---:|---:|---:|---:|---|---:|---:|---:|---|
| Hot keys | 1 | 30,000 | 30,000 | 2,652 | 14,049 | 30,000 | 31 | 14,081 (14,049 / 31 / 38) | 9,185,779 | 0 | 0 | drained ×1 |
| Unique burst | 1 | 10,000 | 10,000 | 10,000 | 10,007 | 10,000 | 5 | 10,109 (10,007 / 101 / 7) | 7,284,625 | 0 | 0 | drained ×1 |
| Slow reader | 1 | 30,000 | 30,000 | 2,652 | 7,012 | 15,896 | 15 | 7,076 (7,012 / 62 / 0) | 5,081,834 | 0 | 1 | reconnected ×1 |
| Fan-out | 16 | 30,000 | 3,551 | 2,652 | 81,835 | 57,075 | 18,548 | 82,347 (81,835 / 496 / 0) | 56,397,535 | 0 | 0 | failed ×16 |

#### Ratios and wall clock over 3 runs (min / median / max, or one value when all agree)

| Scenario | Delivered ops ÷ subscriber source update | Live messages ÷ subscriber source update | Live bytes ÷ subscriber source update | Live messages ÷ delivered op | Live bytes ÷ delivered op | Trailing messages |
|---|---|---|---|---|---|---|
| Hot keys | 0.468 / 0.468 / 0.469 | 0.468 / 0.468 / 0.469 | 297 / 297 / 297 | 1.000 | 633.6 / 633.6 / 633.6 | 36 / 38 / 38 |
| Unique burst | 1.000 / 1.001 / 1.131 | 1.000 / 1.001 / 1.131 | 634 / 634 / 717 | 1.000 | 633.6 / 633.6 / 633.6 | 0 / 7 / 992 |
| Slow reader | 0.233 / 0.234 / 0.235 | 0.233 / 0.234 / 0.235 | 148 / 148 / 149 | 1.000 | 633.6 / 633.6 / 633.6 | 0 / 5 / 15 |
| Fan-out | 0.130 / 0.171 / 0.179 | 0.130 / 0.171 / 0.179 | 82 / 108 / 113 | 1.000 | 633.6 / 633.6 / 633.6 | 0 |

| Scenario | Applied updates | Achieved source rate (/s) | First live update ms | Drain ms | Drain after publish ms | Slow-reader close after messages | Outcomes |
|---|---|---|---|---|---|---|---|
| Hot keys | 30,000 | 9,837 / 9,845 / 9,846 | 120 / 121 / 122 | 3,120 / 3,121 / 3,121 | 71 / 72 / 74 | — | drained ×1; drained ×1; drained ×1 |
| Unique burst | 10,000 | 3,833 / 7,966 / 9,215 | 155 / 158 / 158 | 1,433 / 1,440 / 2,864 | 185 / 255 / 348 | — | drained ×1; drained ×1; drained ×1 |
| Slow reader | 30,000 | 9,917 / 9,995 / 10,022 | 2,008 / 2,008 / 2,008 | 3,089 / 3,092 / 3,095 | 66 / 88 / 102 | 3,303 / 3,304 / 3,304 | reconnected ×1; reconnected ×1; reconnected ×1 |
| Fan-out | 2,519 / 3,551 / 3,996 | 14 / 20 / 22 | 237 / 315 / 326 | — | — | — | failed ×16; failed ×16; failed ×16 |

Fan-out per subscriber (each run): drain ms min–max, delivered ops min–max:
- run 1: no subscriber converged, delivered ops 4,927–5,322
- run 2: no subscriber converged, delivered ops 5,129–5,549
- run 3: no subscriber converged, delivered ops 3,711–4,046

`perSubscriberSourceUpdate` divides by `sourceUpdates`, so it is not
comparable for fan-out runs cut short by the deadline. Per *applied* update,
fan-out delivered 1.34–1.54 operations per subscriber in debug.

### Release build: supplementary

#### Counts from run 1

| Scenario | Subs | Source updates | Applied | Unique keys | Delivered ops | Coalesced source updates | Flushes | WS messages (live / snapshot / trailing) | Wire bytes | Recovery snapshots | Disconnects | Outcomes |
|---|---:|---:|---:|---:|---:|---:|---:|---|---:|---:|---:|---|
| Hot keys | 1 | 30,000 | 30,000 | 2,652 | 13,733 | 30,000 | 30 | 13,765 (13,733 / 31 / 0) | 8,985,582 | 0 | 0 | drained ×1 |
| Unique burst | 1 | 10,000 | 10,000 | 10,000 | 10,000 | 10,000 | 10 | 10,102 (10,000 / 101 / 0) | 7,280,227 | 0 | 0 | drained ×1 |
| Slow reader | 1 | 30,000 | 30,000 | 2,652 | 7,641 | 16,500 | 17 | 7,705 (7,641 / 62 / 0) | 5,479,821 | 0 | 1 | reconnected ×1 |
| Fan-out | 16 | 30,000 | 30,000 | 2,652 | 336,156 | 480,000 | 3,388 | 336,668 (336,156 / 496 / 482) | 217,538,574 | 0 | 0 | drained ×16 |

#### Ratios and wall clock over 3 runs (min / median / max, or one value when all agree)

| Scenario | Delivered ops ÷ subscriber source update | Live messages ÷ subscriber source update | Live bytes ÷ subscriber source update | Live messages ÷ delivered op | Live bytes ÷ delivered op | Trailing messages |
|---|---|---|---|---|---|---|
| Hot keys | 0.458 | 0.458 | 290 | 1.000 | 633.6 | 0 |
| Unique burst | 1.000 | 1.000 | 634 | 1.000 | 633.6 | 0 |
| Slow reader | 0.254 / 0.255 / 0.255 | 0.254 / 0.255 / 0.255 | 161 / 161 / 162 | 1.000 | 633.6 / 633.6 / 633.6 | 0 |
| Fan-out | 0.700 / 0.700 / 0.749 | 0.700 / 0.700 / 0.749 | 443 / 444 / 475 | 1.000 | 633.6 / 633.6 / 633.6 | 9 / 9 / 482 |

| Scenario | Applied updates | Achieved source rate (/s) | First live update ms | Drain ms | Drain after publish ms | Slow-reader close after messages | Outcomes |
|---|---|---|---|---|---|---|---|
| Hot keys | 30,000 | 10,018 / 10,019 / 10,020 | 109 / 110 / 112 | 3,019 / 3,022 / 3,022 | 25 / 27 / 28 | — | drained ×1; drained ×1; drained ×1 |
| Unique burst | 10,000 | 10,058 / 10,061 / 10,065 | 121 / 125 / 127 | 1,032 / 1,033 / 1,033 | 39 / 39 / 39 | — | drained ×1; drained ×1; drained ×1 |
| Slow reader | 30,000 | 10,021 / 10,022 / 10,024 | 2,002 / 2,004 / 2,004 | 3,086 / 3,087 / 3,090 | 92 / 94 / 97 | 3,304 / 3,307 / 3,323 | reconnected ×1; reconnected ×1; reconnected ×1 |
| Fan-out | 30,000 | 1,139 / 1,343 / 1,543 | 140 / 152 / 166 | 19,546 / 22,451 / 26,447 | 97 / 101 / 120 | — | drained ×16; drained ×16; drained ×16 |

Fan-out per subscriber (each run): drain ms min–max, delivered ops min–max:
- run 1: drain 22,345–22,451 ms, delivered ops 20,570–21,301
- run 2: drain 26,383–26,447 ms, delivered ops 22,082–23,054
- run 3: drain 19,484–19,546 ms, delivered ops 20,542–21,529

## Findings

### 1. Delivered operations per source update

- **Hot keys.** 0.468–0.469 (debug) and 0.458 (release) delivered operations
  per source update. The 100 ms window folds repeated writes to the 300 hot keys.
  In each window, most of the 20% spread across all keys is still a distinct
  key.
- **Unique burst.** Coalescing cannot collapse anything here, so the floor is
  1.0. Release delivers exactly 1.000; debug delivers 1.000–1.131 because of
  redundant sends (finding 4).
- **Slow reader.** 0.233–0.235 (debug) and 0.254–0.255 (release). This is lower because
  the reconnect snapshot, not live operations, carries most of the final state.
- **Fan-out.** In release, where every subscriber converged, 0.70–0.75
  operations per source update per subscriber: ingest ran at 1.1–1.5k
  updates/s, so each 100 ms window held about 110–150 updates across 2,652
  keys and coalescing had little to fold. In debug the ingest rate collapses
  (finding 5), and each subscriber received 1.34–1.54 operations per applied
  update, more than one because of redundant sends (finding 4).

### 2. Messages and bytes

- **One message per operation.** Every run sends exactly one WebSocket message
  per delivered operation. Nothing is batched yet.
- **Always a full upsert.** Every coalesced live operation is a full `upsert`;
  `operations.patch` is 0 in every summary. Each costs 633.6 bytes: the ~430
  byte row plus a ~200 byte frame envelope. The source change behind it is a
  113-byte patch. Hot keys therefore ship about 290–297 live bytes per source
  update, and unique burst 634–717.
- **Compression.** Live frames are uncompressed binary JSON. Only snapshot
  frames cross the 1 KiB threshold and are gzipped (101 frames carry 10,000
  rows in about 944 KB).

### 3. Slow reader: explicit close, reconnect, exact state

In all six runs the slow reader followed the same sequence:
1. It read 3,303–3,323 messages: the subscribed frame, 31 snapshot frames,
   about 1,220 live frames already buffered by loopback TCP, and the 2,048 the
   server queue held when it filled.
2. It received a close frame without a status.
3. The server recorded exactly one `deliveryStopped: send-failed`.

There was no lag, no recovery snapshot, and no silent hang. The client
reconnected, and its fresh authoritative snapshot reached the exact final
state 3.09 s after the burst started, 66–102 ms after the burst finished.
For a paused reader the 2,048-message queue is the limit; the 16,384-frame bus
never lagged. This is not a correctness failure.

### 4. Redundant sends when the cache is ahead of the bus

A flush reloads the whole window from the shared `EntityCache`. The projector
writes the cache before it publishes to the bus, so a flush can see values
whose bus messages this subscription has not yet received. The flush sends
those keys with their newer value (`changed` against `current`). When the bus
messages arrive, the keys are pending again and are sent a second time with
the same value.

The harness checks that no such frame ever moves a client backwards. The cost
grows with flush latency:
- Debug unique burst (a 10,000-row window, 5–20 flushes) received 0–992
  messages after it already held the final state, up to 1.13 operations per
  update.
- Debug fan-out delivered 1.34–1.54 operations per applied update per
  subscriber for the same reason.
- Release sends none in unique burst and 9–482 in fan-out.

Delta planning currently mixes cache state with bus position, and a future
patch composer must not inherit that.

### 5. Fan-out: per-subscriber window work throttles ingest

With 16 subscribers on the 3,000-row window, the projector could not apply the
burst at the paced 10,000 updates/s:

| Build | Applied | Rate (/s) | Converged runs |
| --- | --- | --- | --- |
| Debug | 2,519–3,996 of 30,000 in the 180 s deadline | 14–22 | 0 of 3 |
| Release | 30,000 of 30,000 in 19.4–26.4 s | 1,139–1,543 | 3 of 3 |

The scenario is sensitive to machine load. When this baseline was first
recorded, at 1-minute load averages of 13–48, release converged in 1 of 3
runs at 61–176 updates/s (those figures also counted queued updates as
applied). Here, at 6.9–11.3, all three converged. Debug fails either way.

In debug, the server also stopped delivery to 8–10 of the 16 subscriptions
with `send-failed`: their 2,048-message queues filled. Those clients were
still reading their backlog when the deadline ended the run, so they report a
timeout rather than a disconnect. Neither build lagged or took a recovery
snapshot, and the 16,384-frame bus had spare capacity. The server's workers
did not.

macOS `sample` taken during fan-out on the same server code when this
baseline was first recorded (10 s in debug, 5 s in release) shows the four
server workers 93–94% busy. Almost all of
that is the per-subscription collection task:

| Share of collection-task samples | Debug | Release |
| --- | ---: | ---: |
| `plan_coalesced_collection_delta` (diff every row against the previous window) | 40.5% | 14.4% |
| Dropping the previous window's cloned rows | 32.7% | 49.8% |
| `load_query_entities`: clone every row under the cache read lock | 17.8% | 27.5% |
| `load_query_entities`: filter and sort | 8.4% | 3.5% |
| `send_membership_frame` (encode and enqueue) | 0.5% | 1.8% |
| `Projector::run`, all server samples | 5 of 24,996 | 36 of 46,320 |

Every flush costs time proportional to the window, not to the number of
changes. The task clones, sorts, and diffs all 3,000 rows, then frees the
previous copy. With 16 subscriptions this saturates the workers. The
projector's cache writes then queue behind both the flush readers and the busy
workers.

A slower ingest rate does not make flushes cheaper, so the system does not
recover. The debug runs show the loop at its limit: 17,992–18,548 flushes in
180 s, averaging 3.5–4.7 operations each. The release runs made 2,970–4,040
flushes, averaging 89–113 operations each.

Consequences:
- **Batching cannot fix fan-out.** Batching and patch composition act on frame
  encoding, which is 0.5–2% of this CPU. Fan-out cannot converge in debug
  unless flush work stops scaling with the window.
- **Repeated query work is already the limit.** Loading and sorting alone are
  26–31% of the collection task, and whole-window work is 95–99% of it.

### 6. Latency on converged scenarios

- **First live update.** It arrived 109–158 ms after the burst started, one
  100 ms flush plus processing.
- **Hot keys, slow reader, and release unique burst.** Clients converged
  25–102 ms after the projector applied the last update.
- **Debug unique burst.** It took 185–348 ms, because each flush of its
  10,000-row window is slow in debug.
- **Release fan-out.** All 16 subscribers converged 97–120 ms after the
  projector applied the last update.

These numbers describe this machine and build only.

## Caveats

- **Debug build.** The canonical command is a plain `cargo test`,
  which builds unoptimized. Debug multiplies CPU-bound costs (cloning,
  comparing, and dropping `serde_json::Value` trees). That changes counts that
  depend on timing: flush cadence, redundant sends, and fan-out ingest (14–22
  versus 1,139–1,543 updates/s). Compare a debug run only with a debug run, and a
  release run only with a release run.
- **One machine, loopback.** Server, publisher, and clients share one host and
  one process. There is no network latency or loss. Kernel socket buffering
  decides how much a paused reader absorbs before the server queue fills; that
  differs between macOS and Linux.
- **Scheduling noise.** Flush boundaries depend on the tokio scheduler, so
  delivered-operation counts vary between runs even though the workload is
  identical. Repeat runs and compare ranges, not single values.
- **Not a deployment model.** The default cache caps list views at 500
  entities (see above). Real traffic is not paced uniformly, and real clients
  are remote SDKs, not a harness decoder.
- **Local evidence only.** Nothing here is an SLO. Absolute rates and latencies
  belong to this machine and build.

## Comparison procedure

A change to live delivery, such as batching live frames, appends an
after-table to this document. It must not replace the baseline.

1. Start from the change's branch with this harness unchanged, apart from the
   decoding described in the next step. Do not change scenario names, sizes,
   pacing, rows, patches, key patterns, delivery values, deadlines, or thread
   counts. A payload-shape change needs a new scenario name; keep the old one
   too.
2. Teach `ClientView::apply` to accept the batch envelope. Validate the whole
   envelope before applying any item. Count the envelope as one
   `websocketMessage`/`liveMessage`, count each item as one
   `deliveredOperation`, and apply items in order. Keep every field in this
   document. Add `composedPatchItems`, `collapsedSourcePatches`, and
   `compositionFallbacks` alongside, and bump `summaryVersion` to 2.
3. On the same machine class, with no `ARETE_WS_LOAD_*` overrides, run
   `scripts/run-websocket-delivery-load.sh` three times, then
   `scripts/run-websocket-delivery-load.sh --release` three times. Keep the
   `.jsonl` and `.context` files.
4. Check correctness before any number:
   - smoke passes;
   - every profile that converged here converges, with the same
     `expectedStateDigest` it reports here;
   - `protocolErrors` is 0;
   - no frame after convergence moves the state.

   A composed patch that is correct for the model but differs in shape from
   today's upserts is expected.
5. Compare against the tables above, field by field and on the same build:
   - `perSubscriberSourceUpdate.deliveredOperations` shows how much the
     composer collapses patches;
   - `perSubscriberSourceUpdate.liveMessages` and `.liveWireBytes` show the
     batching gain;
   - `perDeliveredOperation.liveWireBytes` shows the patch-versus-upsert size;
   - `trailingMessages` shows the cache-ahead redundancy;
   - slow-reader `outcomes`, `disconnects`, and `server.deliveryStopped`
     show queue pressure;
   - fan-out `publishedUpdates` and `achievedSourceRatePerSec` show ingest
     under fan-out.

   Report min/median/max over the three runs.
6. Append the after-table under a heading naming the change's commit. Say which
   differences exceed the run-to-run ranges recorded here.

## Raw summaries

<details>
<summary>Debug run 1 (canonical), harness <code>b07d3d50</code></summary>

```json
{"scenario":"hot-keys","summaryVersion":1,"config":{"collectionCoalesceMs":100,"listBusCapacity":16384,"messageQueueSize":2048,"cacheEntitiesPerView":3000,"snapshotInitialBatch":50,"snapshotSubsequentBatch":100,"keyPattern":"hot","targetSourceRatePerSec":10000,"readPauseMs":0,"onDisconnect":"reconnect","serverThreads":4,"clientThreads":4,"build":"debug","overrides":[]},"fixture":{"seedRowJsonBytes":429,"sourcePatchJsonBytes":113},"seedRows":3000,"subscribers":1,"sourceUpdates":30000,"uniqueKeys":2652,"deliveredOperations":14049,"operations":{"upsert":14049,"patch":0,"remove":0,"delete":0},"coalescedSourceUpdates":30000,"websocketMessages":14081,"wireBytes":9185779,"decodedBytes":10386299,"liveMessages":14049,"liveWireBytes":8901662,"snapshotMessages":31,"snapshotWireBytes":283952,"snapshotRows":3000,"controlMessages":1,"trailingMessages":38,"frameEncodings":{"text":0,"binaryJson":14050,"binaryGzip":31},"resnapshots":0,"protocolErrors":0,"disconnects":0,"reconnects":0,"outcomes":{"drained":1},"server":{"messagesSent":14081,"resnapshots":0,"lagEvents":0,"droppedUpdates":0,"coalescedFlushes":31,"deliveryStopped":{}},"usage":{"updateMessages":14050,"updateBytes":8901827,"snapshotMessages":31,"snapshotRows":3000,"snapshotBytes":283952,"connections":1,"connectionsClosed":1},"publishedUpdates":30000,"publishMs":3047.0,"achievedSourceRatePerSec":9846.0,"quiescence":"flushed","firstLiveUpdateMs":120.2,"drainMs":3121.3,"drainAfterPublishMs":74.3,"perSubscriberSourceUpdate":{"deliveredOperations":0.4683,"liveMessages":0.4683,"liveWireBytes":296.7221},"perDeliveredOperation":{"liveMessages":1.0,"liveWireBytes":633.6153},"expectedStateDigest":"sha256:ee4d907d69ef14696f948d44f2bcf8049ffec47b2b136803978448826e16056c","finalStateDigest":"sha256:ee4d907d69ef14696f948d44f2bcf8049ffec47b2b136803978448826e16056c","converged":true,"problems":[],"clients":[{"client":0,"outcome":"drained","converged":true,"deliveredOperations":14049,"operations":{"upsert":14049,"patch":0,"remove":0,"delete":0},"websocketMessages":14081,"wireBytes":9185779,"decodedBytes":10386299,"liveMessages":14049,"liveWireBytes":8901662,"snapshotMessages":31,"snapshotWireBytes":283952,"snapshotRows":3000,"controlMessages":1,"trailingMessages":38,"frameEncodings":{"text":0,"binaryJson":14050,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":[],"firstLiveUpdateMs":120.2,"drainMs":3121.3,"finalStateDigest":"sha256:ee4d907d69ef14696f948d44f2bcf8049ffec47b2b136803978448826e16056c","finalRows":3000,"mismatchedRows":0,"mismatchSample":[]}]}
{"scenario":"unique-burst","summaryVersion":1,"config":{"collectionCoalesceMs":100,"listBusCapacity":16384,"messageQueueSize":2048,"cacheEntitiesPerView":10000,"snapshotInitialBatch":50,"snapshotSubsequentBatch":100,"keyPattern":"unique","targetSourceRatePerSec":10000,"readPauseMs":0,"onDisconnect":"reconnect","serverThreads":4,"clientThreads":4,"build":"debug","overrides":[]},"fixture":{"seedRowJsonBytes":429,"sourcePatchJsonBytes":113},"seedRows":10000,"subscribers":1,"sourceUpdates":10000,"uniqueKeys":10000,"deliveredOperations":10007,"operations":{"upsert":10007,"patch":0,"remove":0,"delete":0},"coalescedSourceUpdates":10000,"websocketMessages":10109,"wireBytes":7284625,"decodedBytes":11288496,"liveMessages":10007,"liveWireBytes":6340518,"snapshotMessages":101,"snapshotWireBytes":943942,"snapshotRows":10000,"controlMessages":1,"trailingMessages":7,"frameEncodings":{"text":0,"binaryJson":10008,"binaryGzip":101},"resnapshots":0,"protocolErrors":0,"disconnects":0,"reconnects":0,"outcomes":{"drained":1},"server":{"messagesSent":10109,"resnapshots":0,"lagEvents":0,"droppedUpdates":0,"coalescedFlushes":5,"deliveryStopped":{}},"usage":{"updateMessages":10008,"updateBytes":6340683,"snapshotMessages":101,"snapshotRows":10000,"snapshotBytes":943942,"connections":1,"connectionsClosed":1},"publishedUpdates":10000,"publishMs":1255.3,"achievedSourceRatePerSec":7966.0,"quiescence":"flushed","firstLiveUpdateMs":155.4,"drainMs":1440.5,"drainAfterPublishMs":185.2,"perSubscriberSourceUpdate":{"deliveredOperations":1.0007,"liveMessages":1.0007,"liveWireBytes":634.0518},"perDeliveredOperation":{"liveMessages":1.0,"liveWireBytes":633.6083},"expectedStateDigest":"sha256:4476a15f7d9f9b14aa3c1a6a76b156aefbc4de6a574412b6bd2a433c06b532de","finalStateDigest":"sha256:4476a15f7d9f9b14aa3c1a6a76b156aefbc4de6a574412b6bd2a433c06b532de","converged":true,"problems":[],"clients":[{"client":0,"outcome":"drained","converged":true,"deliveredOperations":10007,"operations":{"upsert":10007,"patch":0,"remove":0,"delete":0},"websocketMessages":10109,"wireBytes":7284625,"decodedBytes":11288496,"liveMessages":10007,"liveWireBytes":6340518,"snapshotMessages":101,"snapshotWireBytes":943942,"snapshotRows":10000,"controlMessages":1,"trailingMessages":7,"frameEncodings":{"text":0,"binaryJson":10008,"binaryGzip":101},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":[],"firstLiveUpdateMs":155.4,"drainMs":1440.5,"finalStateDigest":"sha256:4476a15f7d9f9b14aa3c1a6a76b156aefbc4de6a574412b6bd2a433c06b532de","finalRows":10000,"mismatchedRows":0,"mismatchSample":[]}]}
{"scenario":"slow-reader","summaryVersion":1,"config":{"collectionCoalesceMs":100,"listBusCapacity":16384,"messageQueueSize":2048,"cacheEntitiesPerView":3000,"snapshotInitialBatch":50,"snapshotSubsequentBatch":100,"keyPattern":"hot","targetSourceRatePerSec":10000,"readPauseMs":2000,"onDisconnect":"reconnect","serverThreads":4,"clientThreads":4,"build":"debug","overrides":[]},"fixture":{"seedRowJsonBytes":429,"sourcePatchJsonBytes":113},"seedRows":3000,"subscribers":1,"sourceUpdates":30000,"uniqueKeys":2652,"deliveredOperations":7012,"operations":{"upsert":7012,"patch":0,"remove":0,"delete":0},"coalescedSourceUpdates":15896,"websocketMessages":7076,"wireBytes":5081834,"decodedBytes":7412132,"liveMessages":7012,"liveWireBytes":4442894,"snapshotMessages":62,"snapshotWireBytes":638610,"snapshotRows":6000,"controlMessages":2,"trailingMessages":0,"frameEncodings":{"text":0,"binaryJson":7014,"binaryGzip":62},"resnapshots":0,"protocolErrors":0,"disconnects":1,"reconnects":1,"outcomes":{"reconnected":1},"server":{"messagesSent":7076,"resnapshots":0,"lagEvents":0,"droppedUpdates":0,"coalescedFlushes":15,"deliveryStopped":{"send-failed":1}},"usage":{"updateMessages":7014,"updateBytes":4443224,"snapshotMessages":62,"snapshotRows":6000,"snapshotBytes":638610,"connections":2,"connectionsClosed":2},"publishedUpdates":30000,"publishMs":3001.6,"achievedSourceRatePerSec":9995.0,"quiescence":"grace","firstLiveUpdateMs":2007.9,"drainMs":3089.1,"drainAfterPublishMs":87.5,"perSubscriberSourceUpdate":{"deliveredOperations":0.2337,"liveMessages":0.2337,"liveWireBytes":148.0965},"perDeliveredOperation":{"liveMessages":1.0,"liveWireBytes":633.6129},"expectedStateDigest":"sha256:ee4d907d69ef14696f948d44f2bcf8049ffec47b2b136803978448826e16056c","finalStateDigest":"sha256:ee4d907d69ef14696f948d44f2bcf8049ffec47b2b136803978448826e16056c","converged":true,"problems":[],"clients":[{"client":0,"outcome":"reconnected","converged":true,"deliveredOperations":7012,"operations":{"upsert":7012,"patch":0,"remove":0,"delete":0},"websocketMessages":7076,"wireBytes":5081834,"decodedBytes":7412132,"liveMessages":7012,"liveWireBytes":4442894,"snapshotMessages":62,"snapshotWireBytes":638610,"snapshotRows":6000,"controlMessages":2,"trailingMessages":0,"frameEncodings":{"text":0,"binaryJson":7014,"binaryGzip":62},"initialSnapshots":2,"resnapshots":0,"disconnects":1,"disconnectDetails":[{"atMs":2103.6,"reason":"close without a status","messagesBefore":3303}],"reconnects":1,"protocolErrors":[],"failures":[],"firstLiveUpdateMs":2007.9,"drainMs":3089.1,"finalStateDigest":"sha256:ee4d907d69ef14696f948d44f2bcf8049ffec47b2b136803978448826e16056c","finalRows":3000,"mismatchedRows":0,"mismatchSample":[]}]}
{"scenario":"fan-out","summaryVersion":1,"config":{"collectionCoalesceMs":100,"listBusCapacity":16384,"messageQueueSize":2048,"cacheEntitiesPerView":3000,"snapshotInitialBatch":50,"snapshotSubsequentBatch":100,"keyPattern":"hot","targetSourceRatePerSec":10000,"readPauseMs":0,"onDisconnect":"reconnect","serverThreads":4,"clientThreads":4,"build":"debug","overrides":[]},"fixture":{"seedRowJsonBytes":429,"sourcePatchJsonBytes":113},"seedRows":3000,"subscribers":16,"sourceUpdates":30000,"uniqueKeys":2652,"deliveredOperations":81835,"operations":{"upsert":81835,"patch":0,"remove":0,"delete":0},"coalescedSourceUpdates":57075,"websocketMessages":82347,"wireBytes":56397535,"decodedBytes":75606167,"liveMessages":81835,"liveWireBytes":51851975,"snapshotMessages":496,"snapshotWireBytes":4542920,"snapshotRows":48000,"controlMessages":16,"trailingMessages":0,"frameEncodings":{"text":0,"binaryJson":81851,"binaryGzip":496},"resnapshots":0,"protocolErrors":0,"disconnects":0,"reconnects":0,"outcomes":{"failed":16},"server":{"messagesSent":82640,"resnapshots":0,"lagEvents":0,"droppedUpdates":0,"coalescedFlushes":18548,"deliveryStopped":{"send-failed":9}},"usage":{"updateMessages":82144,"updateBytes":52040280,"snapshotMessages":496,"snapshotRows":48000,"snapshotBytes":4542920,"connections":16,"connectionsClosed":16},"publishedUpdates":3551,"publishMs":179061.9,"achievedSourceRatePerSec":20.0,"quiescence":"grace","firstLiveUpdateMs":325.8,"drainMs":null,"drainAfterPublishMs":null,"perSubscriberSourceUpdate":{"deliveredOperations":0.1705,"liveMessages":0.1705,"liveWireBytes":108.0249},"perDeliveredOperation":{"liveMessages":1.0,"liveWireBytes":633.6161},"expectedStateDigest":"sha256:ee4d907d69ef14696f948d44f2bcf8049ffec47b2b136803978448826e16056c","finalStateDigest":null,"converged":false,"problems":["only 3551 of 30000 burst updates were applied before the deadline","client 0: timed out with 2556 rows still different from the final state","client 1: timed out with 2556 rows still different from the final state","client 2: timed out with 2556 rows still different from the final state","client 3: timed out with 2556 rows still different from the final state","client 4: timed out with 2556 rows still different from the final state","client 5: timed out with 2556 rows still different from the final state","client 6: timed out with 2556 rows still different from the final state","client 7: timed out with 2556 rows still different from the final state","client 8: timed out with 2556 rows still different from the final state","client 9: timed out with 2556 rows still different from the final state","client 10: timed out with 2556 rows still different from the final state","client 11: timed out with 2556 rows still different from the final state","client 12: timed out with 2556 rows still different from the final state","client 13: timed out with 2556 rows still different from the final state","client 14: timed out with 2556 rows still different from the final state","client 15: timed out with 2556 rows still different from the final state","0 of 16 subscribers reached the exact final state","subscribers do not share the expected final digest"],"clients":[{"client":0,"outcome":"failed","converged":false,"deliveredOperations":5291,"operations":{"upsert":5291,"patch":0,"remove":0,"delete":0},"websocketMessages":5323,"wireBytes":3636536,"decodedBytes":4837071,"liveMessages":5291,"liveWireBytes":3352434,"snapshotMessages":31,"snapshotWireBytes":283937,"snapshotRows":3000,"controlMessages":1,"trailingMessages":0,"frameEncodings":{"text":0,"binaryJson":5292,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":["timed out with 2556 rows still different from the final state"],"firstLiveUpdateMs":177.6,"drainMs":null,"finalStateDigest":"sha256:af0d9807029dd8b945b0b11c7123e8bd816b2c491be7ad3f8cfc2d986d50a588","finalRows":3000,"mismatchedRows":2556,"mismatchSample":["3JCdWShx19dSV35ZwC5otuELajEpaD9v2ps9nUEsWmHo: expected _seq \"300100074:000000000110\" got \"300100006:000000000020\"","8VGDRFbKt7ZcX91mnKRDTW46HoznibRL9tTNZgzpGQ2a: expected _seq \"300100024:000000000150\" got \"300000006:000000000303\"","98cLznipcGSbBDCDkRSUVET5oxogy5BbAEHMmNj9VNBN: expected _seq \"300100074:000000000285\" got \"300100008:000000000193\"","9ML5P95txjsi43eXhWc6URcFhffp2aXg4CstN11h4gMk: expected _seq \"300100037:000000000399\" got \"300100006:000000000034\"","HAKx19UuadyBcDLrNJG8rxqpX92Wo8gqXA26grDULxmJ: expected _seq \"300100052:000000000160\" got \"300000000:000000000377\""]},{"client":1,"outcome":"failed","converged":false,"deliveredOperations":5113,"operations":{"upsert":5113,"patch":0,"remove":0,"delete":0},"websocketMessages":5145,"wireBytes":3523793,"decodedBytes":4724324,"liveMessages":5113,"liveWireBytes":3239687,"snapshotMessages":31,"snapshotWireBytes":283941,"snapshotRows":3000,"controlMessages":1,"trailingMessages":0,"frameEncodings":{"text":0,"binaryJson":5114,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":["timed out with 2556 rows still different from the final state"],"firstLiveUpdateMs":152.3,"drainMs":null,"finalStateDigest":"sha256:028e5e1b553aaa3d74f3ef8881d36bafd403ad7e2ae9b65cbedff888707de9c7","finalRows":3000,"mismatchedRows":2556,"mismatchSample":["3JCdWShx19dSV35ZwC5otuELajEpaD9v2ps9nUEsWmHo: expected _seq \"300100074:000000000110\" got \"300100006:000000000020\"","8VGDRFbKt7ZcX91mnKRDTW46HoznibRL9tTNZgzpGQ2a: expected _seq \"300100024:000000000150\" got \"300000006:000000000303\"","98cLznipcGSbBDCDkRSUVET5oxogy5BbAEHMmNj9VNBN: expected _seq \"300100074:000000000285\" got \"300100008:000000000193\"","9ML5P95txjsi43eXhWc6URcFhffp2aXg4CstN11h4gMk: expected _seq \"300100037:000000000399\" got \"300100006:000000000034\"","HAKx19UuadyBcDLrNJG8rxqpX92Wo8gqXA26grDULxmJ: expected _seq \"300100052:000000000160\" got \"300000000:000000000377\""]},{"client":2,"outcome":"failed","converged":false,"deliveredOperations":5130,"operations":{"upsert":5130,"patch":0,"remove":0,"delete":0},"websocketMessages":5162,"wireBytes":3534585,"decodedBytes":4735077,"liveMessages":5130,"liveWireBytes":3250440,"snapshotMessages":31,"snapshotWireBytes":283980,"snapshotRows":3000,"controlMessages":1,"trailingMessages":0,"frameEncodings":{"text":0,"binaryJson":5131,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":["timed out with 2556 rows still different from the final state"],"firstLiveUpdateMs":183.7,"drainMs":null,"finalStateDigest":"sha256:af0d9807029dd8b945b0b11c7123e8bd816b2c491be7ad3f8cfc2d986d50a588","finalRows":3000,"mismatchedRows":2556,"mismatchSample":["3JCdWShx19dSV35ZwC5otuELajEpaD9v2ps9nUEsWmHo: expected _seq \"300100074:000000000110\" got \"300100006:000000000020\"","8VGDRFbKt7ZcX91mnKRDTW46HoznibRL9tTNZgzpGQ2a: expected _seq \"300100024:000000000150\" got \"300000006:000000000303\"","98cLznipcGSbBDCDkRSUVET5oxogy5BbAEHMmNj9VNBN: expected _seq \"300100074:000000000285\" got \"300100008:000000000193\"","9ML5P95txjsi43eXhWc6URcFhffp2aXg4CstN11h4gMk: expected _seq \"300100037:000000000399\" got \"300100006:000000000034\"","HAKx19UuadyBcDLrNJG8rxqpX92Wo8gqXA26grDULxmJ: expected _seq \"300100052:000000000160\" got \"300000000:000000000377\""]},{"client":3,"outcome":"failed","converged":false,"deliveredOperations":5161,"operations":{"upsert":5161,"patch":0,"remove":0,"delete":0},"websocketMessages":5193,"wireBytes":3554199,"decodedBytes":4754736,"liveMessages":5161,"liveWireBytes":3270099,"snapshotMessages":31,"snapshotWireBytes":283935,"snapshotRows":3000,"controlMessages":1,"trailingMessages":0,"frameEncodings":{"text":0,"binaryJson":5162,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":["timed out with 2556 rows still different from the final state"],"firstLiveUpdateMs":71.6,"drainMs":null,"finalStateDigest":"sha256:af0d9807029dd8b945b0b11c7123e8bd816b2c491be7ad3f8cfc2d986d50a588","finalRows":3000,"mismatchedRows":2556,"mismatchSample":["3JCdWShx19dSV35ZwC5otuELajEpaD9v2ps9nUEsWmHo: expected _seq \"300100074:000000000110\" got \"300100006:000000000020\"","8VGDRFbKt7ZcX91mnKRDTW46HoznibRL9tTNZgzpGQ2a: expected _seq \"300100024:000000000150\" got \"300000006:000000000303\"","98cLznipcGSbBDCDkRSUVET5oxogy5BbAEHMmNj9VNBN: expected _seq \"300100074:000000000285\" got \"300100008:000000000193\"","9ML5P95txjsi43eXhWc6URcFhffp2aXg4CstN11h4gMk: expected _seq \"300100037:000000000399\" got \"300100006:000000000034\"","HAKx19UuadyBcDLrNJG8rxqpX92Wo8gqXA26grDULxmJ: expected _seq \"300100052:000000000160\" got \"300000000:000000000377\""]},{"client":4,"outcome":"failed","converged":false,"deliveredOperations":5061,"operations":{"upsert":5061,"patch":0,"remove":0,"delete":0},"websocketMessages":5093,"wireBytes":3490871,"decodedBytes":4691396,"liveMessages":5061,"liveWireBytes":3206759,"snapshotMessages":31,"snapshotWireBytes":283947,"snapshotRows":3000,"controlMessages":1,"trailingMessages":0,"frameEncodings":{"text":0,"binaryJson":5062,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":["timed out with 2556 rows still different from the final state"],"firstLiveUpdateMs":185.1,"drainMs":null,"finalStateDigest":"sha256:028e5e1b553aaa3d74f3ef8881d36bafd403ad7e2ae9b65cbedff888707de9c7","finalRows":3000,"mismatchedRows":2556,"mismatchSample":["3JCdWShx19dSV35ZwC5otuELajEpaD9v2ps9nUEsWmHo: expected _seq \"300100074:000000000110\" got \"300100006:000000000020\"","8VGDRFbKt7ZcX91mnKRDTW46HoznibRL9tTNZgzpGQ2a: expected _seq \"300100024:000000000150\" got \"300000006:000000000303\"","98cLznipcGSbBDCDkRSUVET5oxogy5BbAEHMmNj9VNBN: expected _seq \"300100074:000000000285\" got \"300100008:000000000193\"","9ML5P95txjsi43eXhWc6URcFhffp2aXg4CstN11h4gMk: expected _seq \"300100037:000000000399\" got \"300100006:000000000034\"","HAKx19UuadyBcDLrNJG8rxqpX92Wo8gqXA26grDULxmJ: expected _seq \"300100052:000000000160\" got \"300000000:000000000377\""]},{"client":5,"outcome":"failed","converged":false,"deliveredOperations":4997,"operations":{"upsert":4997,"patch":0,"remove":0,"delete":0},"websocketMessages":5029,"wireBytes":3450285,"decodedBytes":4650831,"liveMessages":4997,"liveWireBytes":3166194,"snapshotMessages":31,"snapshotWireBytes":283926,"snapshotRows":3000,"controlMessages":1,"trailingMessages":0,"frameEncodings":{"text":0,"binaryJson":4998,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":["timed out with 2556 rows still different from the final state"],"firstLiveUpdateMs":322.7,"drainMs":null,"finalStateDigest":"sha256:af0d9807029dd8b945b0b11c7123e8bd816b2c491be7ad3f8cfc2d986d50a588","finalRows":3000,"mismatchedRows":2556,"mismatchSample":["3JCdWShx19dSV35ZwC5otuELajEpaD9v2ps9nUEsWmHo: expected _seq \"300100074:000000000110\" got \"300100006:000000000020\"","8VGDRFbKt7ZcX91mnKRDTW46HoznibRL9tTNZgzpGQ2a: expected _seq \"300100024:000000000150\" got \"300000006:000000000303\"","98cLznipcGSbBDCDkRSUVET5oxogy5BbAEHMmNj9VNBN: expected _seq \"300100074:000000000285\" got \"300100008:000000000193\"","9ML5P95txjsi43eXhWc6URcFhffp2aXg4CstN11h4gMk: expected _seq \"300100037:000000000399\" got \"300100006:000000000034\"","HAKx19UuadyBcDLrNJG8rxqpX92Wo8gqXA26grDULxmJ: expected _seq \"300100052:000000000160\" got \"300000000:000000000377\""]},{"client":6,"outcome":"failed","converged":false,"deliveredOperations":5079,"operations":{"upsert":5079,"patch":0,"remove":0,"delete":0},"websocketMessages":5111,"wireBytes":3502247,"decodedBytes":4702776,"liveMessages":5079,"liveWireBytes":3218139,"snapshotMessages":31,"snapshotWireBytes":283943,"snapshotRows":3000,"controlMessages":1,"trailingMessages":0,"frameEncodings":{"text":0,"binaryJson":5080,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":["timed out with 2556 rows still different from the final state"],"firstLiveUpdateMs":325.8,"drainMs":null,"finalStateDigest":"sha256:af0d9807029dd8b945b0b11c7123e8bd816b2c491be7ad3f8cfc2d986d50a588","finalRows":3000,"mismatchedRows":2556,"mismatchSample":["3JCdWShx19dSV35ZwC5otuELajEpaD9v2ps9nUEsWmHo: expected _seq \"300100074:000000000110\" got \"300100006:000000000020\"","8VGDRFbKt7ZcX91mnKRDTW46HoznibRL9tTNZgzpGQ2a: expected _seq \"300100024:000000000150\" got \"300000006:000000000303\"","98cLznipcGSbBDCDkRSUVET5oxogy5BbAEHMmNj9VNBN: expected _seq \"300100074:000000000285\" got \"300100008:000000000193\"","9ML5P95txjsi43eXhWc6URcFhffp2aXg4CstN11h4gMk: expected _seq \"300100037:000000000399\" got \"300100006:000000000034\"","HAKx19UuadyBcDLrNJG8rxqpX92Wo8gqXA26grDULxmJ: expected _seq \"300100052:000000000160\" got \"300000000:000000000377\""]},{"client":7,"outcome":"failed","converged":false,"deliveredOperations":4996,"operations":{"upsert":4996,"patch":0,"remove":0,"delete":0},"websocketMessages":5028,"wireBytes":3449554,"decodedBytes":4650155,"liveMessages":4996,"liveWireBytes":3165518,"snapshotMessages":31,"snapshotWireBytes":283871,"snapshotRows":3000,"controlMessages":1,"trailingMessages":0,"frameEncodings":{"text":0,"binaryJson":4997,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":["timed out with 2556 rows still different from the final state"],"firstLiveUpdateMs":144.9,"drainMs":null,"finalStateDigest":"sha256:af0d9807029dd8b945b0b11c7123e8bd816b2c491be7ad3f8cfc2d986d50a588","finalRows":3000,"mismatchedRows":2556,"mismatchSample":["3JCdWShx19dSV35ZwC5otuELajEpaD9v2ps9nUEsWmHo: expected _seq \"300100074:000000000110\" got \"300100006:000000000020\"","8VGDRFbKt7ZcX91mnKRDTW46HoznibRL9tTNZgzpGQ2a: expected _seq \"300100024:000000000150\" got \"300000006:000000000303\"","98cLznipcGSbBDCDkRSUVET5oxogy5BbAEHMmNj9VNBN: expected _seq \"300100074:000000000285\" got \"300100008:000000000193\"","9ML5P95txjsi43eXhWc6URcFhffp2aXg4CstN11h4gMk: expected _seq \"300100037:000000000399\" got \"300100006:000000000034\"","HAKx19UuadyBcDLrNJG8rxqpX92Wo8gqXA26grDULxmJ: expected _seq \"300100052:000000000160\" got \"300000000:000000000377\""]},{"client":8,"outcome":"failed","converged":false,"deliveredOperations":5322,"operations":{"upsert":5322,"patch":0,"remove":0,"delete":0},"websocketMessages":5354,"wireBytes":3656192,"decodedBytes":4856719,"liveMessages":5322,"liveWireBytes":3372082,"snapshotMessages":31,"snapshotWireBytes":283945,"snapshotRows":3000,"controlMessages":1,"trailingMessages":0,"frameEncodings":{"text":0,"binaryJson":5323,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":["timed out with 2556 rows still different from the final state"],"firstLiveUpdateMs":133.8,"drainMs":null,"finalStateDigest":"sha256:af0d9807029dd8b945b0b11c7123e8bd816b2c491be7ad3f8cfc2d986d50a588","finalRows":3000,"mismatchedRows":2556,"mismatchSample":["3JCdWShx19dSV35ZwC5otuELajEpaD9v2ps9nUEsWmHo: expected _seq \"300100074:000000000110\" got \"300100006:000000000020\"","8VGDRFbKt7ZcX91mnKRDTW46HoznibRL9tTNZgzpGQ2a: expected _seq \"300100024:000000000150\" got \"300000006:000000000303\"","98cLznipcGSbBDCDkRSUVET5oxogy5BbAEHMmNj9VNBN: expected _seq \"300100074:000000000285\" got \"300100008:000000000193\"","9ML5P95txjsi43eXhWc6URcFhffp2aXg4CstN11h4gMk: expected _seq \"300100037:000000000399\" got \"300100006:000000000034\"","HAKx19UuadyBcDLrNJG8rxqpX92Wo8gqXA26grDULxmJ: expected _seq \"300100052:000000000160\" got \"300000000:000000000377\""]},{"client":9,"outcome":"failed","converged":false,"deliveredOperations":5045,"operations":{"upsert":5045,"patch":0,"remove":0,"delete":0},"websocketMessages":5077,"wireBytes":3480711,"decodedBytes":4681250,"liveMessages":5045,"liveWireBytes":3196613,"snapshotMessages":31,"snapshotWireBytes":283933,"snapshotRows":3000,"controlMessages":1,"trailingMessages":0,"frameEncodings":{"text":0,"binaryJson":5046,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":["timed out with 2556 rows still different from the final state"],"firstLiveUpdateMs":183.5,"drainMs":null,"finalStateDigest":"sha256:af0d9807029dd8b945b0b11c7123e8bd816b2c491be7ad3f8cfc2d986d50a588","finalRows":3000,"mismatchedRows":2556,"mismatchSample":["3JCdWShx19dSV35ZwC5otuELajEpaD9v2ps9nUEsWmHo: expected _seq \"300100074:000000000110\" got \"300100006:000000000020\"","8VGDRFbKt7ZcX91mnKRDTW46HoznibRL9tTNZgzpGQ2a: expected _seq \"300100024:000000000150\" got \"300000006:000000000303\"","98cLznipcGSbBDCDkRSUVET5oxogy5BbAEHMmNj9VNBN: expected _seq \"300100074:000000000285\" got \"300100008:000000000193\"","9ML5P95txjsi43eXhWc6URcFhffp2aXg4CstN11h4gMk: expected _seq \"300100037:000000000399\" got \"300100006:000000000034\"","HAKx19UuadyBcDLrNJG8rxqpX92Wo8gqXA26grDULxmJ: expected _seq \"300100052:000000000160\" got \"300000000:000000000377\""]},{"client":10,"outcome":"failed","converged":false,"deliveredOperations":5008,"operations":{"upsert":5008,"patch":0,"remove":0,"delete":0},"websocketMessages":5040,"wireBytes":3457218,"decodedBytes":4657777,"liveMessages":5008,"liveWireBytes":3173140,"snapshotMessages":31,"snapshotWireBytes":283913,"snapshotRows":3000,"controlMessages":1,"trailingMessages":0,"frameEncodings":{"text":0,"binaryJson":5009,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":["timed out with 2556 rows still different from the final state"],"firstLiveUpdateMs":188.1,"drainMs":null,"finalStateDigest":"sha256:028e5e1b553aaa3d74f3ef8881d36bafd403ad7e2ae9b65cbedff888707de9c7","finalRows":3000,"mismatchedRows":2556,"mismatchSample":["3JCdWShx19dSV35ZwC5otuELajEpaD9v2ps9nUEsWmHo: expected _seq \"300100074:000000000110\" got \"300100006:000000000020\"","8VGDRFbKt7ZcX91mnKRDTW46HoznibRL9tTNZgzpGQ2a: expected _seq \"300100024:000000000150\" got \"300000006:000000000303\"","98cLznipcGSbBDCDkRSUVET5oxogy5BbAEHMmNj9VNBN: expected _seq \"300100074:000000000285\" got \"300100008:000000000193\"","9ML5P95txjsi43eXhWc6URcFhffp2aXg4CstN11h4gMk: expected _seq \"300100037:000000000399\" got \"300100006:000000000034\"","HAKx19UuadyBcDLrNJG8rxqpX92Wo8gqXA26grDULxmJ: expected _seq \"300100052:000000000160\" got \"300000000:000000000377\""]},{"client":11,"outcome":"failed","converged":false,"deliveredOperations":5122,"operations":{"upsert":5122,"patch":0,"remove":0,"delete":0},"websocketMessages":5154,"wireBytes":3529489,"decodedBytes":4730033,"liveMessages":5122,"liveWireBytes":3245396,"snapshotMessages":31,"snapshotWireBytes":283928,"snapshotRows":3000,"controlMessages":1,"trailingMessages":0,"frameEncodings":{"text":0,"binaryJson":5123,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":["timed out with 2556 rows still different from the final state"],"firstLiveUpdateMs":202.6,"drainMs":null,"finalStateDigest":"sha256:af0d9807029dd8b945b0b11c7123e8bd816b2c491be7ad3f8cfc2d986d50a588","finalRows":3000,"mismatchedRows":2556,"mismatchSample":["3JCdWShx19dSV35ZwC5otuELajEpaD9v2ps9nUEsWmHo: expected _seq \"300100074:000000000110\" got \"300100006:000000000020\"","8VGDRFbKt7ZcX91mnKRDTW46HoznibRL9tTNZgzpGQ2a: expected _seq \"300100024:000000000150\" got \"300000006:000000000303\"","98cLznipcGSbBDCDkRSUVET5oxogy5BbAEHMmNj9VNBN: expected _seq \"300100074:000000000285\" got \"300100008:000000000193\"","9ML5P95txjsi43eXhWc6URcFhffp2aXg4CstN11h4gMk: expected _seq \"300100037:000000000399\" got \"300100006:000000000034\"","HAKx19UuadyBcDLrNJG8rxqpX92Wo8gqXA26grDULxmJ: expected _seq \"300100052:000000000160\" got \"300000000:000000000377\""]},{"client":12,"outcome":"failed","converged":false,"deliveredOperations":5290,"operations":{"upsert":5290,"patch":0,"remove":0,"delete":0},"websocketMessages":5322,"wireBytes":3635964,"decodedBytes":4836468,"liveMessages":5290,"liveWireBytes":3351831,"snapshotMessages":31,"snapshotWireBytes":283968,"snapshotRows":3000,"controlMessages":1,"trailingMessages":0,"frameEncodings":{"text":0,"binaryJson":5291,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":["timed out with 2556 rows still different from the final state"],"firstLiveUpdateMs":45.5,"drainMs":null,"finalStateDigest":"sha256:af0d9807029dd8b945b0b11c7123e8bd816b2c491be7ad3f8cfc2d986d50a588","finalRows":3000,"mismatchedRows":2556,"mismatchSample":["3JCdWShx19dSV35ZwC5otuELajEpaD9v2ps9nUEsWmHo: expected _seq \"300100074:000000000110\" got \"300100006:000000000020\"","8VGDRFbKt7ZcX91mnKRDTW46HoznibRL9tTNZgzpGQ2a: expected _seq \"300100024:000000000150\" got \"300000006:000000000303\"","98cLznipcGSbBDCDkRSUVET5oxogy5BbAEHMmNj9VNBN: expected _seq \"300100074:000000000285\" got \"300100008:000000000193\"","9ML5P95txjsi43eXhWc6URcFhffp2aXg4CstN11h4gMk: expected _seq \"300100037:000000000399\" got \"300100006:000000000034\"","HAKx19UuadyBcDLrNJG8rxqpX92Wo8gqXA26grDULxmJ: expected _seq \"300100052:000000000160\" got \"300000000:000000000377\""]},{"client":13,"outcome":"failed","converged":false,"deliveredOperations":4927,"operations":{"upsert":4927,"patch":0,"remove":0,"delete":0},"websocketMessages":4959,"wireBytes":3405949,"decodedBytes":4606458,"liveMessages":4927,"liveWireBytes":3121821,"snapshotMessages":31,"snapshotWireBytes":283963,"snapshotRows":3000,"controlMessages":1,"trailingMessages":0,"frameEncodings":{"text":0,"binaryJson":4928,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":["timed out with 2556 rows still different from the final state"],"firstLiveUpdateMs":208.6,"drainMs":null,"finalStateDigest":"sha256:29155a8bdfc367b6a4556d3074b51b5dc8462210d96437afd9c013bbc6854418","finalRows":3000,"mismatchedRows":2556,"mismatchSample":["3JCdWShx19dSV35ZwC5otuELajEpaD9v2ps9nUEsWmHo: expected _seq \"300100074:000000000110\" got \"300100006:000000000020\"","8VGDRFbKt7ZcX91mnKRDTW46HoznibRL9tTNZgzpGQ2a: expected _seq \"300100024:000000000150\" got \"300000006:000000000303\"","98cLznipcGSbBDCDkRSUVET5oxogy5BbAEHMmNj9VNBN: expected _seq \"300100074:000000000285\" got \"300100008:000000000193\"","9ML5P95txjsi43eXhWc6URcFhffp2aXg4CstN11h4gMk: expected _seq \"300100037:000000000399\" got \"300100006:000000000034\"","HAKx19UuadyBcDLrNJG8rxqpX92Wo8gqXA26grDULxmJ: expected _seq \"300100052:000000000160\" got \"300000000:000000000377\""]},{"client":14,"outcome":"failed","converged":false,"deliveredOperations":5162,"operations":{"upsert":5162,"patch":0,"remove":0,"delete":0},"websocketMessages":5194,"wireBytes":3554797,"decodedBytes":4755366,"liveMessages":5162,"liveWireBytes":3270729,"snapshotMessages":31,"snapshotWireBytes":283903,"snapshotRows":3000,"controlMessages":1,"trailingMessages":0,"frameEncodings":{"text":0,"binaryJson":5163,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":["timed out with 2556 rows still different from the final state"],"firstLiveUpdateMs":94.5,"drainMs":null,"finalStateDigest":"sha256:af0d9807029dd8b945b0b11c7123e8bd816b2c491be7ad3f8cfc2d986d50a588","finalRows":3000,"mismatchedRows":2556,"mismatchSample":["3JCdWShx19dSV35ZwC5otuELajEpaD9v2ps9nUEsWmHo: expected _seq \"300100074:000000000110\" got \"300100006:000000000020\"","8VGDRFbKt7ZcX91mnKRDTW46HoznibRL9tTNZgzpGQ2a: expected _seq \"300100024:000000000150\" got \"300000006:000000000303\"","98cLznipcGSbBDCDkRSUVET5oxogy5BbAEHMmNj9VNBN: expected _seq \"300100074:000000000285\" got \"300100008:000000000193\"","9ML5P95txjsi43eXhWc6URcFhffp2aXg4CstN11h4gMk: expected _seq \"300100037:000000000399\" got \"300100006:000000000034\"","HAKx19UuadyBcDLrNJG8rxqpX92Wo8gqXA26grDULxmJ: expected _seq \"300100052:000000000160\" got \"300000000:000000000377\""]},{"client":15,"outcome":"failed","converged":false,"deliveredOperations":5131,"operations":{"upsert":5131,"patch":0,"remove":0,"delete":0},"websocketMessages":5163,"wireBytes":3535145,"decodedBytes":4735730,"liveMessages":5131,"liveWireBytes":3251093,"snapshotMessages":31,"snapshotWireBytes":283887,"snapshotRows":3000,"controlMessages":1,"trailingMessages":0,"frameEncodings":{"text":0,"binaryJson":5132,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":["timed out with 2556 rows still different from the final state"],"firstLiveUpdateMs":325.7,"drainMs":null,"finalStateDigest":"sha256:af0d9807029dd8b945b0b11c7123e8bd816b2c491be7ad3f8cfc2d986d50a588","finalRows":3000,"mismatchedRows":2556,"mismatchSample":["3JCdWShx19dSV35ZwC5otuELajEpaD9v2ps9nUEsWmHo: expected _seq \"300100074:000000000110\" got \"300100006:000000000020\"","8VGDRFbKt7ZcX91mnKRDTW46HoznibRL9tTNZgzpGQ2a: expected _seq \"300100024:000000000150\" got \"300000006:000000000303\"","98cLznipcGSbBDCDkRSUVET5oxogy5BbAEHMmNj9VNBN: expected _seq \"300100074:000000000285\" got \"300100008:000000000193\"","9ML5P95txjsi43eXhWc6URcFhffp2aXg4CstN11h4gMk: expected _seq \"300100037:000000000399\" got \"300100006:000000000034\"","HAKx19UuadyBcDLrNJG8rxqpX92Wo8gqXA26grDULxmJ: expected _seq \"300100052:000000000160\" got \"300000000:000000000377\""]}]}
```

</details>

<details>
<summary>Release run 1, harness <code>b07d3d50</code></summary>

```json
{"scenario":"hot-keys","summaryVersion":1,"config":{"collectionCoalesceMs":100,"listBusCapacity":16384,"messageQueueSize":2048,"cacheEntitiesPerView":3000,"snapshotInitialBatch":50,"snapshotSubsequentBatch":100,"keyPattern":"hot","targetSourceRatePerSec":10000,"readPauseMs":0,"onDisconnect":"reconnect","serverThreads":4,"clientThreads":4,"build":"release","overrides":[]},"fixture":{"seedRowJsonBytes":429,"sourcePatchJsonBytes":113},"seedRows":3000,"subscribers":1,"sourceUpdates":30000,"uniqueKeys":2652,"deliveredOperations":13733,"operations":{"upsert":13733,"patch":0,"remove":0,"delete":0},"coalescedSourceUpdates":30000,"websocketMessages":13765,"wireBytes":8985582,"decodedBytes":10186101,"liveMessages":13733,"liveWireBytes":8701464,"snapshotMessages":31,"snapshotWireBytes":283953,"snapshotRows":3000,"controlMessages":1,"trailingMessages":0,"frameEncodings":{"text":0,"binaryJson":13734,"binaryGzip":31},"resnapshots":0,"protocolErrors":0,"disconnects":0,"reconnects":0,"outcomes":{"drained":1},"server":{"messagesSent":13765,"resnapshots":0,"lagEvents":0,"droppedUpdates":0,"coalescedFlushes":30,"deliveryStopped":{}},"usage":{"updateMessages":13734,"updateBytes":8701629,"snapshotMessages":31,"snapshotRows":3000,"snapshotBytes":283953,"connections":1,"connectionsClosed":1},"publishedUpdates":30000,"publishMs":2994.7,"achievedSourceRatePerSec":10018.0,"quiescence":"flushed","firstLiveUpdateMs":111.8,"drainMs":3022.0,"drainAfterPublishMs":27.3,"perSubscriberSourceUpdate":{"deliveredOperations":0.4578,"liveMessages":0.4578,"liveWireBytes":290.0488},"perDeliveredOperation":{"liveMessages":1.0,"liveWireBytes":633.6171},"expectedStateDigest":"sha256:ee4d907d69ef14696f948d44f2bcf8049ffec47b2b136803978448826e16056c","finalStateDigest":"sha256:ee4d907d69ef14696f948d44f2bcf8049ffec47b2b136803978448826e16056c","converged":true,"problems":[],"clients":[{"client":0,"outcome":"drained","converged":true,"deliveredOperations":13733,"operations":{"upsert":13733,"patch":0,"remove":0,"delete":0},"websocketMessages":13765,"wireBytes":8985582,"decodedBytes":10186101,"liveMessages":13733,"liveWireBytes":8701464,"snapshotMessages":31,"snapshotWireBytes":283953,"snapshotRows":3000,"controlMessages":1,"trailingMessages":0,"frameEncodings":{"text":0,"binaryJson":13734,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":[],"firstLiveUpdateMs":111.8,"drainMs":3022.0,"finalStateDigest":"sha256:ee4d907d69ef14696f948d44f2bcf8049ffec47b2b136803978448826e16056c","finalRows":3000,"mismatchedRows":0,"mismatchSample":[]}]}
{"scenario":"unique-burst","summaryVersion":1,"config":{"collectionCoalesceMs":100,"listBusCapacity":16384,"messageQueueSize":2048,"cacheEntitiesPerView":10000,"snapshotInitialBatch":50,"snapshotSubsequentBatch":100,"keyPattern":"unique","targetSourceRatePerSec":10000,"readPauseMs":0,"onDisconnect":"reconnect","serverThreads":4,"clientThreads":4,"build":"release","overrides":[]},"fixture":{"seedRowJsonBytes":429,"sourcePatchJsonBytes":113},"seedRows":10000,"subscribers":1,"sourceUpdates":10000,"uniqueKeys":10000,"deliveredOperations":10000,"operations":{"upsert":10000,"patch":0,"remove":0,"delete":0},"coalescedSourceUpdates":10000,"websocketMessages":10102,"wireBytes":7280227,"decodedBytes":11284061,"liveMessages":10000,"liveWireBytes":6336083,"snapshotMessages":101,"snapshotWireBytes":943979,"snapshotRows":10000,"controlMessages":1,"trailingMessages":0,"frameEncodings":{"text":0,"binaryJson":10001,"binaryGzip":101},"resnapshots":0,"protocolErrors":0,"disconnects":0,"reconnects":0,"outcomes":{"drained":1},"server":{"messagesSent":10102,"resnapshots":0,"lagEvents":0,"droppedUpdates":0,"coalescedFlushes":10,"deliveryStopped":{}},"usage":{"updateMessages":10001,"updateBytes":6336248,"snapshotMessages":101,"snapshotRows":10000,"snapshotBytes":943979,"connections":1,"connectionsClosed":1},"publishedUpdates":10000,"publishMs":994.0,"achievedSourceRatePerSec":10061.0,"quiescence":"flushed","firstLiveUpdateMs":124.9,"drainMs":1032.8,"drainAfterPublishMs":38.8,"perSubscriberSourceUpdate":{"deliveredOperations":1.0,"liveMessages":1.0,"liveWireBytes":633.6083},"perDeliveredOperation":{"liveMessages":1.0,"liveWireBytes":633.6083},"expectedStateDigest":"sha256:4476a15f7d9f9b14aa3c1a6a76b156aefbc4de6a574412b6bd2a433c06b532de","finalStateDigest":"sha256:4476a15f7d9f9b14aa3c1a6a76b156aefbc4de6a574412b6bd2a433c06b532de","converged":true,"problems":[],"clients":[{"client":0,"outcome":"drained","converged":true,"deliveredOperations":10000,"operations":{"upsert":10000,"patch":0,"remove":0,"delete":0},"websocketMessages":10102,"wireBytes":7280227,"decodedBytes":11284061,"liveMessages":10000,"liveWireBytes":6336083,"snapshotMessages":101,"snapshotWireBytes":943979,"snapshotRows":10000,"controlMessages":1,"trailingMessages":0,"frameEncodings":{"text":0,"binaryJson":10001,"binaryGzip":101},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":[],"firstLiveUpdateMs":124.9,"drainMs":1032.8,"finalStateDigest":"sha256:4476a15f7d9f9b14aa3c1a6a76b156aefbc4de6a574412b6bd2a433c06b532de","finalRows":10000,"mismatchedRows":0,"mismatchSample":[]}]}
{"scenario":"slow-reader","summaryVersion":1,"config":{"collectionCoalesceMs":100,"listBusCapacity":16384,"messageQueueSize":2048,"cacheEntitiesPerView":3000,"snapshotInitialBatch":50,"snapshotSubsequentBatch":100,"keyPattern":"hot","targetSourceRatePerSec":10000,"readPauseMs":2000,"onDisconnect":"reconnect","serverThreads":4,"clientThreads":4,"build":"release","overrides":[]},"fixture":{"seedRowJsonBytes":429,"sourcePatchJsonBytes":113},"seedRows":3000,"subscribers":1,"sourceUpdates":30000,"uniqueKeys":2652,"deliveredOperations":7641,"operations":{"upsert":7641,"patch":0,"remove":0,"delete":0},"coalescedSourceUpdates":16500,"websocketMessages":7705,"wireBytes":5479821,"decodedBytes":7810627,"liveMessages":7641,"liveWireBytes":4841409,"snapshotMessages":62,"snapshotWireBytes":638082,"snapshotRows":6000,"controlMessages":2,"trailingMessages":0,"frameEncodings":{"text":0,"binaryJson":7643,"binaryGzip":62},"resnapshots":0,"protocolErrors":0,"disconnects":1,"reconnects":1,"outcomes":{"reconnected":1},"server":{"messagesSent":7705,"resnapshots":0,"lagEvents":0,"droppedUpdates":0,"coalescedFlushes":17,"deliveryStopped":{"send-failed":1}},"usage":{"updateMessages":7643,"updateBytes":4841739,"snapshotMessages":62,"snapshotRows":6000,"snapshotBytes":638082,"connections":2,"connectionsClosed":2},"publishedUpdates":30000,"publishMs":2993.4,"achievedSourceRatePerSec":10022.0,"quiescence":"grace","firstLiveUpdateMs":2004.4,"drainMs":3090.2,"drainAfterPublishMs":96.8,"perSubscriberSourceUpdate":{"deliveredOperations":0.2547,"liveMessages":0.2547,"liveWireBytes":161.3803},"perDeliveredOperation":{"liveMessages":1.0,"liveWireBytes":633.6093},"expectedStateDigest":"sha256:ee4d907d69ef14696f948d44f2bcf8049ffec47b2b136803978448826e16056c","finalStateDigest":"sha256:ee4d907d69ef14696f948d44f2bcf8049ffec47b2b136803978448826e16056c","converged":true,"problems":[],"clients":[{"client":0,"outcome":"reconnected","converged":true,"deliveredOperations":7641,"operations":{"upsert":7641,"patch":0,"remove":0,"delete":0},"websocketMessages":7705,"wireBytes":5479821,"decodedBytes":7810627,"liveMessages":7641,"liveWireBytes":4841409,"snapshotMessages":62,"snapshotWireBytes":638082,"snapshotRows":6000,"controlMessages":2,"trailingMessages":0,"frameEncodings":{"text":0,"binaryJson":7643,"binaryGzip":62},"initialSnapshots":2,"resnapshots":0,"disconnects":1,"disconnectDetails":[{"atMs":2044.4,"reason":"close without a status","messagesBefore":3307}],"reconnects":1,"protocolErrors":[],"failures":[],"firstLiveUpdateMs":2004.4,"drainMs":3090.2,"finalStateDigest":"sha256:ee4d907d69ef14696f948d44f2bcf8049ffec47b2b136803978448826e16056c","finalRows":3000,"mismatchedRows":0,"mismatchSample":[]}]}
{"scenario":"fan-out","summaryVersion":1,"config":{"collectionCoalesceMs":100,"listBusCapacity":16384,"messageQueueSize":2048,"cacheEntitiesPerView":3000,"snapshotInitialBatch":50,"snapshotSubsequentBatch":100,"keyPattern":"hot","targetSourceRatePerSec":10000,"readPauseMs":0,"onDisconnect":"reconnect","serverThreads":4,"clientThreads":4,"build":"release","overrides":[]},"fixture":{"seedRowJsonBytes":429,"sourcePatchJsonBytes":113},"seedRows":3000,"subscribers":16,"sourceUpdates":30000,"uniqueKeys":2652,"deliveredOperations":336156,"operations":{"upsert":336156,"patch":0,"remove":0,"delete":0},"coalescedSourceUpdates":480000,"websocketMessages":336668,"wireBytes":217538574,"decodedBytes":236747110,"liveMessages":336156,"liveWireBytes":212992918,"snapshotMessages":496,"snapshotWireBytes":4543016,"snapshotRows":48000,"controlMessages":16,"trailingMessages":482,"frameEncodings":{"text":0,"binaryJson":336172,"binaryGzip":496},"resnapshots":0,"protocolErrors":0,"disconnects":0,"reconnects":0,"outcomes":{"drained":16},"server":{"messagesSent":336668,"resnapshots":0,"lagEvents":0,"droppedUpdates":0,"coalescedFlushes":3388,"deliveryStopped":{}},"usage":{"updateMessages":336172,"updateBytes":212995558,"snapshotMessages":496,"snapshotRows":48000,"snapshotBytes":4543016,"connections":16,"connectionsClosed":16},"publishedUpdates":30000,"publishMs":22331.5,"achievedSourceRatePerSec":1343.0,"quiescence":"flushed","firstLiveUpdateMs":152.1,"drainMs":22451.1,"drainAfterPublishMs":119.6,"perSubscriberSourceUpdate":{"deliveredOperations":0.7003,"liveMessages":0.7003,"liveWireBytes":443.7352},"perDeliveredOperation":{"liveMessages":1.0,"liveWireBytes":633.6133},"expectedStateDigest":"sha256:ee4d907d69ef14696f948d44f2bcf8049ffec47b2b136803978448826e16056c","finalStateDigest":"sha256:ee4d907d69ef14696f948d44f2bcf8049ffec47b2b136803978448826e16056c","converged":true,"problems":[],"clients":[{"client":0,"outcome":"drained","converged":true,"deliveredOperations":21125,"operations":{"upsert":21125,"patch":0,"remove":0,"delete":0},"websocketMessages":21157,"wireBytes":13669132,"decodedBytes":14869692,"liveMessages":21125,"liveWireBytes":13385055,"snapshotMessages":31,"snapshotWireBytes":283912,"snapshotRows":3000,"controlMessages":1,"trailingMessages":1,"frameEncodings":{"text":0,"binaryJson":21126,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":[],"firstLiveUpdateMs":105.8,"drainMs":22447.5,"finalStateDigest":"sha256:ee4d907d69ef14696f948d44f2bcf8049ffec47b2b136803978448826e16056c","finalRows":3000,"mismatchedRows":0,"mismatchSample":[]},{"client":1,"outcome":"drained","converged":true,"deliveredOperations":21021,"operations":{"upsert":21021,"patch":0,"remove":0,"delete":0},"websocketMessages":21053,"wireBytes":13603229,"decodedBytes":14803784,"liveMessages":21021,"liveWireBytes":13319147,"snapshotMessages":31,"snapshotWireBytes":283917,"snapshotRows":3000,"controlMessages":1,"trailingMessages":68,"frameEncodings":{"text":0,"binaryJson":21022,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":[],"firstLiveUpdateMs":139.4,"drainMs":22394.1,"finalStateDigest":"sha256:ee4d907d69ef14696f948d44f2bcf8049ffec47b2b136803978448826e16056c","finalRows":3000,"mismatchedRows":0,"mismatchSample":[]},{"client":2,"outcome":"drained","converged":true,"deliveredOperations":20651,"operations":{"upsert":20651,"patch":0,"remove":0,"delete":0},"websocketMessages":20683,"wireBytes":13368849,"decodedBytes":14569380,"liveMessages":20651,"liveWireBytes":13084743,"snapshotMessages":31,"snapshotWireBytes":283941,"snapshotRows":3000,"controlMessages":1,"trailingMessages":1,"frameEncodings":{"text":0,"binaryJson":20652,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":[],"firstLiveUpdateMs":95.6,"drainMs":22383.2,"finalStateDigest":"sha256:ee4d907d69ef14696f948d44f2bcf8049ffec47b2b136803978448826e16056c","finalRows":3000,"mismatchedRows":0,"mismatchSample":[]},{"client":3,"outcome":"drained","converged":true,"deliveredOperations":21151,"operations":{"upsert":21151,"patch":0,"remove":0,"delete":0},"websocketMessages":21183,"wireBytes":13685684,"decodedBytes":14886206,"liveMessages":21151,"liveWireBytes":13401569,"snapshotMessages":31,"snapshotWireBytes":283950,"snapshotRows":3000,"controlMessages":1,"trailingMessages":68,"frameEncodings":{"text":0,"binaryJson":21152,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":[],"firstLiveUpdateMs":97.9,"drainMs":22410.1,"finalStateDigest":"sha256:ee4d907d69ef14696f948d44f2bcf8049ffec47b2b136803978448826e16056c","finalRows":3000,"mismatchedRows":0,"mismatchSample":[]},{"client":4,"outcome":"drained","converged":true,"deliveredOperations":21078,"operations":{"upsert":21078,"patch":0,"remove":0,"delete":0},"websocketMessages":21110,"wireBytes":13639329,"decodedBytes":14839880,"liveMessages":21078,"liveWireBytes":13355243,"snapshotMessages":31,"snapshotWireBytes":283921,"snapshotRows":3000,"controlMessages":1,"trailingMessages":1,"frameEncodings":{"text":0,"binaryJson":21079,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":[],"firstLiveUpdateMs":65.8,"drainMs":22391.4,"finalStateDigest":"sha256:ee4d907d69ef14696f948d44f2bcf8049ffec47b2b136803978448826e16056c","finalRows":3000,"mismatchedRows":0,"mismatchSample":[]},{"client":5,"outcome":"drained","converged":true,"deliveredOperations":21026,"operations":{"upsert":21026,"patch":0,"remove":0,"delete":0},"websocketMessages":21058,"wireBytes":13606470,"decodedBytes":14807003,"liveMessages":21026,"liveWireBytes":13322366,"snapshotMessages":31,"snapshotWireBytes":283939,"snapshotRows":3000,"controlMessages":1,"trailingMessages":67,"frameEncodings":{"text":0,"binaryJson":21027,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":[],"firstLiveUpdateMs":107.9,"drainMs":22451.1,"finalStateDigest":"sha256:ee4d907d69ef14696f948d44f2bcf8049ffec47b2b136803978448826e16056c","finalRows":3000,"mismatchedRows":0,"mismatchSample":[]},{"client":6,"outcome":"drained","converged":true,"deliveredOperations":20978,"operations":{"upsert":20978,"patch":0,"remove":0,"delete":0},"websocketMessages":21010,"wireBytes":13576042,"decodedBytes":14776564,"liveMessages":20978,"liveWireBytes":13291927,"snapshotMessages":31,"snapshotWireBytes":283950,"snapshotRows":3000,"controlMessages":1,"trailingMessages":38,"frameEncodings":{"text":0,"binaryJson":20979,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":[],"firstLiveUpdateMs":137.1,"drainMs":22432.2,"finalStateDigest":"sha256:ee4d907d69ef14696f948d44f2bcf8049ffec47b2b136803978448826e16056c","finalRows":3000,"mismatchedRows":0,"mismatchSample":[]},{"client":7,"outcome":"drained","converged":true,"deliveredOperations":21013,"operations":{"upsert":21013,"patch":0,"remove":0,"delete":0},"websocketMessages":21045,"wireBytes":13598234,"decodedBytes":14798753,"liveMessages":21013,"liveWireBytes":13314116,"snapshotMessages":31,"snapshotWireBytes":283953,"snapshotRows":3000,"controlMessages":1,"trailingMessages":68,"frameEncodings":{"text":0,"binaryJson":21014,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":[],"firstLiveUpdateMs":144.1,"drainMs":22395.5,"finalStateDigest":"sha256:ee4d907d69ef14696f948d44f2bcf8049ffec47b2b136803978448826e16056c","finalRows":3000,"mismatchedRows":0,"mismatchSample":[]},{"client":8,"outcome":"drained","converged":true,"deliveredOperations":21018,"operations":{"upsert":21018,"patch":0,"remove":0,"delete":0},"websocketMessages":21050,"wireBytes":13601438,"decodedBytes":14801946,"liveMessages":21018,"liveWireBytes":13317309,"snapshotMessages":31,"snapshotWireBytes":283964,"snapshotRows":3000,"controlMessages":1,"trailingMessages":11,"frameEncodings":{"text":0,"binaryJson":21019,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":[],"firstLiveUpdateMs":103.2,"drainMs":22345.0,"finalStateDigest":"sha256:ee4d907d69ef14696f948d44f2bcf8049ffec47b2b136803978448826e16056c","finalRows":3000,"mismatchedRows":0,"mismatchSample":[]},{"client":9,"outcome":"drained","converged":true,"deliveredOperations":20736,"operations":{"upsert":20736,"patch":0,"remove":0,"delete":0},"websocketMessages":20768,"wireBytes":13422718,"decodedBytes":14623258,"liveMessages":20736,"liveWireBytes":13138621,"snapshotMessages":31,"snapshotWireBytes":283932,"snapshotRows":3000,"controlMessages":1,"trailingMessages":68,"frameEncodings":{"text":0,"binaryJson":20737,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":[],"firstLiveUpdateMs":152.1,"drainMs":22401.3,"finalStateDigest":"sha256:ee4d907d69ef14696f948d44f2bcf8049ffec47b2b136803978448826e16056c","finalRows":3000,"mismatchedRows":0,"mismatchSample":[]},{"client":10,"outcome":"drained","converged":true,"deliveredOperations":21136,"operations":{"upsert":21136,"patch":0,"remove":0,"delete":0},"websocketMessages":21168,"wireBytes":13676186,"decodedBytes":14876712,"liveMessages":21136,"liveWireBytes":13392075,"snapshotMessages":31,"snapshotWireBytes":283946,"snapshotRows":3000,"controlMessages":1,"trailingMessages":10,"frameEncodings":{"text":0,"binaryJson":21137,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":[],"firstLiveUpdateMs":91.1,"drainMs":22360.1,"finalStateDigest":"sha256:ee4d907d69ef14696f948d44f2bcf8049ffec47b2b136803978448826e16056c","finalRows":3000,"mismatchedRows":0,"mismatchSample":[]},{"client":11,"outcome":"drained","converged":true,"deliveredOperations":21235,"operations":{"upsert":21235,"patch":0,"remove":0,"delete":0},"websocketMessages":21267,"wireBytes":13738888,"decodedBytes":14939396,"liveMessages":21235,"liveWireBytes":13454759,"snapshotMessages":31,"snapshotWireBytes":283964,"snapshotRows":3000,"controlMessages":1,"trailingMessages":1,"frameEncodings":{"text":0,"binaryJson":21236,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":[],"firstLiveUpdateMs":9.8,"drainMs":22381.2,"finalStateDigest":"sha256:ee4d907d69ef14696f948d44f2bcf8049ffec47b2b136803978448826e16056c","finalRows":3000,"mismatchedRows":0,"mismatchSample":[]},{"client":12,"outcome":"drained","converged":true,"deliveredOperations":21301,"operations":{"upsert":21301,"patch":0,"remove":0,"delete":0},"websocketMessages":21333,"wireBytes":13780657,"decodedBytes":14981212,"liveMessages":21301,"liveWireBytes":13496575,"snapshotMessages":31,"snapshotWireBytes":283917,"snapshotRows":3000,"controlMessages":1,"trailingMessages":1,"frameEncodings":{"text":0,"binaryJson":21302,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":[],"firstLiveUpdateMs":9.6,"drainMs":22395.1,"finalStateDigest":"sha256:ee4d907d69ef14696f948d44f2bcf8049ffec47b2b136803978448826e16056c","finalRows":3000,"mismatchedRows":0,"mismatchSample":[]},{"client":13,"outcome":"drained","converged":true,"deliveredOperations":20570,"operations":{"upsert":20570,"patch":0,"remove":0,"delete":0},"websocketMessages":20602,"wireBytes":13317564,"decodedBytes":14518081,"liveMessages":20570,"liveWireBytes":13033444,"snapshotMessages":31,"snapshotWireBytes":283955,"snapshotRows":3000,"controlMessages":1,"trailingMessages":11,"frameEncodings":{"text":0,"binaryJson":20571,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":[],"firstLiveUpdateMs":110.0,"drainMs":22356.7,"finalStateDigest":"sha256:ee4d907d69ef14696f948d44f2bcf8049ffec47b2b136803978448826e16056c","finalRows":3000,"mismatchedRows":0,"mismatchSample":[]},{"client":14,"outcome":"drained","converged":true,"deliveredOperations":21038,"operations":{"upsert":21038,"patch":0,"remove":0,"delete":0},"websocketMessages":21070,"wireBytes":13614096,"decodedBytes":14814618,"liveMessages":21038,"liveWireBytes":13329981,"snapshotMessages":31,"snapshotWireBytes":283950,"snapshotRows":3000,"controlMessages":1,"trailingMessages":68,"frameEncodings":{"text":0,"binaryJson":21039,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":[],"firstLiveUpdateMs":97.2,"drainMs":22434.4,"finalStateDigest":"sha256:ee4d907d69ef14696f948d44f2bcf8049ffec47b2b136803978448826e16056c","finalRows":3000,"mismatchedRows":0,"mismatchSample":[]},{"client":15,"outcome":"drained","converged":true,"deliveredOperations":21079,"operations":{"upsert":21079,"patch":0,"remove":0,"delete":0},"websocketMessages":21111,"wireBytes":13640058,"decodedBytes":14840625,"liveMessages":21079,"liveWireBytes":13355988,"snapshotMessages":31,"snapshotWireBytes":283905,"snapshotRows":3000,"controlMessages":1,"trailingMessages":0,"frameEncodings":{"text":0,"binaryJson":21080,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":[],"firstLiveUpdateMs":102.1,"drainMs":22379.1,"finalStateDigest":"sha256:ee4d907d69ef14696f948d44f2bcf8049ffec47b2b136803978448826e16056c","finalRows":3000,"mismatchedRows":0,"mismatchSample":[]}]}
```

</details>
