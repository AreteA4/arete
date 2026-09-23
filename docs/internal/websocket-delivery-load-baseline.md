# WebSocket list-delivery load baseline

This records how latest-state list delivery behaves over a real socket while
each live list update is still its own frame. It is local evidence for
comparing one implementation with the next. None of the wall-clock numbers
here is an SLO or a capacity promise.

The harness is `rust/arete-server/src/websocket/load_tests.rs`. It starts the
real `Projector`, `BusManager`, `EntityCache`, `ViewIndex`, and
`WebSocketServer` on an ephemeral port. It publishes deterministic
`MutationBatch`es and reads each socket until the client's merged state equals
the publisher's model.

## Baseline identity

| | |
| --- | --- |
| Delivery code | `3039d71b6438394fba086969bb93d4d9f95f4be6` (lag recovery, PR #248) |
| Measured at | Harness commit `c6d27cea`, on merge commit `b491e798`. `git diff 3039d71b b491e798 -- rust interpreter arete-hash Cargo.lock` is empty, so the delivery code is exactly `3039d71b`. |
| Production changes | None. `server.rs` gains only a `#[cfg(test)]` `DeliveryProbe` that mirrors existing `WsMetrics` calls. |
| Measured | 2026-09-23 03:07–03:27 UTC: three debug runs, then three release runs |
| Machine | Apple M2 (8 logical CPUs), 24 GB, macOS 15.0 (Darwin 24.0.0 arm64) |
| Toolchain | rustc 1.98.0 (88d9e12ae 2026-08-18) |
| Runtime | Server tokio runtime with 4 workers; clients on a separate 4-worker runtime |
| Background load | A developer workstation running a backup client, a browser, and other agent sessions. The 1-minute load average at run start was 5.8–9.1 for the debug runs and 13–48 for the release runs. |

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
   reaches zero against the model. Every update carries a unique `_seq`, so no
   earlier frame can complete the final state by accident.
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
   disconnect.
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
| `publishedUpdates` | Burst mutations the projector acknowledged; below `sourceUpdates` only when the deadline cut the burst short |
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
| `outcomes` | Per client: `drained`, `recovered` (lag resnapshot), `reconnected` (explicit close, then a fresh subscription), `closed-after-converging` (the server dropped it before the run ended), or `failed` |
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
`target/websocket-delivery-load/baseline-{debug,release}-{1,2,3}.*`. The raw
summaries for debug run 1 (canonical) and release run 3 are at the end of this
document.

Every debug run exits 101, and so did release runs 1 and 2, because fan-out
did not converge before its deadline (finding 5). In all six runs, every other
scenario converged to the expected digest with no protocol errors. Two
expected digests hold for every scenario and run:
`sha256:ee4d907d…6056c` (3,000-row scenarios) and `sha256:4476a15f…532de`
(unique burst). Server `messagesSent` equals client `websocketMessages` in
every scenario that converged.

### Debug build: the canonical baseline

#### Counts from run 1

| Scenario | Subs | Source updates | Applied | Unique keys | Delivered ops | Coalesced source updates | Flushes | WS messages (live / snapshot / trailing) | Wire bytes | Recovery snapshots | Disconnects | Outcomes |
|---|---:|---:|---:|---:|---:|---:|---:|---|---:|---:|---:|---|
| Hot keys | 1 | 30,000 | 30,000 | 2,652 | 13,955 | 30,000 | 31 | 13,987 (13,955 / 31 / 29) | 9,126,199 | 0 | 0 | drained ×1 |
| Unique burst | 1 | 10,000 | 10,000 | 10,000 | 13,898 | 10,000 | 8 | 14,000 (13,898 / 101 / 3,898) | 9,749,937 | 0 | 0 | drained ×1 |
| Slow reader | 1 | 30,000 | 30,000 | 2,652 | 6,973 | 15,842 | 15 | 7,037 (6,973 / 62 / 0) | 5,057,282 | 0 | 1 | reconnected ×1 |
| Fan-out | 16 | 30,000 | 4,906 | 2,652 | 85,100 | 62,245 | 16,483 | 85,612 (85,100 / 496 / 0) | 58,466,327 | 0 | 0 | failed ×16 |

#### Ratios and wall clock over 3 runs (min / median / max, or one value when all agree)

| Scenario | Delivered ops ÷ subscriber source update | Live messages ÷ subscriber source update | Live bytes ÷ subscriber source update | Live messages ÷ delivered op | Live bytes ÷ delivered op | Trailing messages |
|---|---|---|---|---|---|---|
| Hot keys | 0.465 / 0.481 / 0.488 | 0.465 / 0.481 / 0.488 | 295 / 305 / 309 | 1.000 | 633.6 / 633.6 / 633.6 | 3 / 17 / 29 |
| Unique burst | 1.017 / 1.390 / 1.920 | 1.017 / 1.390 / 1.920 | 645 / 881 / 1,216 | 1.000 | 633.6 / 633.6 / 633.6 | 43 / 3,898 / 9,192 |
| Slow reader | 0.232 / 0.233 / 0.235 | 0.232 / 0.233 / 0.235 | 147 / 148 / 149 | 1.000 | 633.6 / 633.6 / 633.6 | 0 / 0 / 13 |
| Fan-out | 0.079 / 0.114 / 0.177 | 0.079 / 0.114 / 0.177 | 50 / 72 / 112 | 1.000 | 633.6 / 633.6 / 633.6 | 0 |

| Scenario | Applied updates | Achieved source rate (/s) | First live update ms | Drain ms | Drain after publish ms | Slow-reader close after messages | Outcomes |
|---|---|---|---|---|---|---|---|
| Hot keys | 30,000 | 9,146 / 9,417 / 9,847 | 121 / 122 / 123 | 3,118 / 3,237 / 3,335 | 52 / 55 / 72 | — | drained ×1; drained ×1; drained ×1 |
| Unique burst | 10,000 | 6,749 / 10,031 / 10,061 | 156 / 158 / 159 | 1,141 / 1,353 / 1,655 | 147 / 173 / 356 | — | drained ×1; drained ×1; drained ×1 |
| Slow reader | 30,000 | 9,919 / 9,922 / 10,003 | 2,006 / 2,006 / 2,007 | 3,092 / 3,097 / 3,106 | 68 / 83 / 98 | 3,303 / 3,304 / 3,304 | reconnected ×1; reconnected ×1; reconnected ×1 |
| Fan-out | 2,725 / 3,335 / 4,906 | 15 / 19 / 27 | 296 / 304 / 399 | — | — | — | failed ×16; failed ×16; failed ×16 |

Fan-out per subscriber (each run): drain ms min–max, delivered ops min–max:
- run 1: no subscriber converged, delivered ops 4,953–5,555
- run 2: no subscriber converged, delivered ops 3,258–3,528
- run 3: no subscriber converged, delivered ops 2,242–2,505

`perSubscriberSourceUpdate` divides by `sourceUpdates`, so it is not
comparable for fan-out runs cut short by the deadline. Per *applied* update,
fan-out delivered 0.87–1.08 operations per subscriber in debug.

### Release build: supplementary

#### Counts from run 1

| Scenario | Subs | Source updates | Applied | Unique keys | Delivered ops | Coalesced source updates | Flushes | WS messages (live / snapshot / trailing) | Wire bytes | Recovery snapshots | Disconnects | Outcomes |
|---|---:|---:|---:|---:|---:|---:|---:|---|---:|---:|---:|---|
| Hot keys | 1 | 30,000 | 30,000 | 2,652 | 13,744 | 30,000 | 30 | 13,776 (13,744 / 31 / 0) | 8,992,407 | 0 | 0 | drained ×1 |
| Unique burst | 1 | 10,000 | 10,000 | 10,000 | 10,218 | 10,000 | 11 | 10,320 (10,218 / 101 / 43) | 7,418,330 | 0 | 0 | drained ×1 |
| Slow reader | 1 | 30,000 | 30,000 | 2,652 | 7,658 | 16,600 | 17 | 7,722 (7,658 / 62 / 0) | 5,490,410 | 0 | 1 | reconnected ×1 |
| Fan-out | 16 | 30,000 | 10,938 | 2,652 | 169,079 | 158,590 | 15,911 | 169,591 (169,079 / 496 / 0) | 111,675,943 | 0 | 0 | failed ×16 |

#### Ratios and wall clock over 3 runs (min / median / max, or one value when all agree)

| Scenario | Delivered ops ÷ subscriber source update | Live messages ÷ subscriber source update | Live bytes ÷ subscriber source update | Live messages ÷ delivered op | Live bytes ÷ delivered op | Trailing messages |
|---|---|---|---|---|---|---|
| Hot keys | 0.456 / 0.458 / 0.459 | 0.456 / 0.458 / 0.459 | 289 / 290 / 291 | 1.000 | 633.6 / 633.6 / 633.6 | 0 / 0 / 7 |
| Unique burst | 1.000 / 1.022 / 1.031 | 1.000 / 1.022 / 1.031 | 634 / 647 / 653 | 1.000 | 633.6 / 633.6 / 633.6 | 0 / 43 / 43 |
| Slow reader | 0.253 / 0.255 / 0.259 | 0.253 / 0.255 / 0.259 | 160 / 162 / 164 | 1.000 | 633.6 / 633.6 / 633.6 | 0 |
| Fan-out | 0.352 / 0.944 / 1.049 | 0.352 / 0.944 / 1.049 | 223 / 598 / 665 | 1.000 | 633.6 / 633.6 / 633.6 | 0 / 0 / 16 |

| Scenario | Applied updates | Achieved source rate (/s) | First live update ms | Drain ms | Drain after publish ms | Slow-reader close after messages | Outcomes |
|---|---|---|---|---|---|---|---|
| Hot keys | 30,000 | 10,018 / 10,022 / 10,024 | 107 / 108 / 110 | 3,029 / 3,036 / 3,063 | 36 / 41 / 69 | — | drained ×1; drained ×1; drained ×1 |
| Unique burst | 10,000 | 9,313 / 10,061 / 10,072 | 128 / 140 / 146 | 1,036 / 1,059 / 1,126 | 44 / 52 / 65 | — | drained ×1; drained ×1; drained ×1 |
| Slow reader | 30,000 | 9,958 / 10,021 / 10,023 | 2,002 / 2,002 / 2,004 | 3,055 / 3,091 / 3,102 | 61 / 90 / 98 | 3,304 | reconnected ×1; reconnected ×1; reconnected ×1 |
| Fan-out | 10,938 / 26,041 / 30,000 | 61 / 145 / 176 | 128 / 202 / 255 | 170,670 (run 3 only) | 122 (run 3 only) | — | failed ×16; failed ×16; drained ×16 |

Fan-out per subscriber (each run): drain ms min–max, delivered ops min–max:
- run 1: no subscriber converged, delivered ops 10,204–10,833
- run 2: no subscriber converged, delivered ops 27,625–29,075
- run 3: drain 170,623–170,670 ms, delivered ops 30,845–31,926

Per applied update, fan-out delivered 0.97–1.09 operations per subscriber in
release.

## Findings

### 1. Delivered operations per source update

- **Hot keys.** 0.47–0.49 (debug) and 0.46 (release) delivered operations per
  source update. The 100 ms window folds repeated writes to the 300 hot keys.
  In each window, most of the 20% spread across all keys is still a distinct
  key.
- **Unique burst.** Coalescing cannot collapse anything here, so the floor is
  1.0. Release delivers 1.00–1.03; debug delivers 1.02–1.92 because of
  redundant sends (finding 4).
- **Slow reader.** 0.23 (debug) and 0.25–0.26 (release). This is lower because
  the reconnect snapshot, not live operations, carries most of the final state.
- **Fan-out.** With 16 subscribers the ingest rate collapses (finding 5). Each
  100 ms window then holds only a few updates, and coalescing has almost
  nothing to fold: 0.87–1.09 operations per applied update per subscriber.

### 2. Messages and bytes

- **One message per operation.** Every run sends exactly one WebSocket message
  per delivered operation. Nothing is batched yet.
- **Always a full upsert.** Every coalesced live operation is a full `upsert`;
  `operations.patch` is 0 in every summary. Each costs 633.6 bytes: the ~430
  byte row plus a ~200 byte frame envelope. The source change behind it is a
  113-byte patch. Hot keys therefore ship about 290–310 live bytes per source
  update, and unique burst 634–1,216.
- **Compression.** Live frames are uncompressed binary JSON. Only snapshot
  frames cross the 1 KiB threshold and are gzipped (101 frames carry 10,000
  rows in about 944 KB).

### 3. Slow reader: explicit close, reconnect, exact state

In all six runs the slow reader followed the same sequence:
1. It read 3,303–3,304 messages: the subscribed frame, 31 snapshot frames,
   about 1,220 live frames already buffered by loopback TCP, and the 2,048 the
   server queue held when it filled.
2. It received a close frame without a status.
3. The server recorded exactly one `deliveryStopped: send-failed`.

There was no lag, no recovery snapshot, and no silent hang. The client
reconnected, and its fresh authoritative snapshot reached the exact final
state 3.06–3.11 s after the burst started, 61–98 ms after the burst finished.
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
- Debug unique burst (a 10,000-row window, 6–8 flushes of roughly 140–280 ms
  each) received 43–9,192 messages after it already held the final state, up
  to 1.92 operations per update.
- Release sends 0–43 such messages.

Delta planning currently mixes cache state with bus position, and a future
patch composer must not inherit that.

### 5. Fan-out: per-subscriber window work throttles ingest

With 16 subscribers on the 3,000-row window, the projector could not apply the
burst at the paced 10,000 updates/s:

| Build | Applied in the 180 s deadline | Rate (/s) | Converged runs |
| --- | --- | --- | --- |
| Debug | 2,725–4,906 of 30,000 | 15–27 | 0 of 3 |
| Release | 10,938–30,000 of 30,000 | 61–176 | 1 of 3 (170.7 s) |

The same server code was also run four times in release on earlier harness
revisions. Three of those runs converged in 46–105 s (286–651 updates/s); the
fourth managed 84 updates/s. Across all seven release runs, the rate fell as
the machine's background load rose (the recorded runs started at load averages
35 and 48 failed; the one at 13 converged). The scenario sits on a cliff edge.

There was no lag, recovery snapshot, or disconnect, and the bus and queues had
spare capacity. The server's workers did not.

macOS `sample` taken during fan-out on the same server code (10 s in debug,
5 s in release) shows the four server workers 93–94% busy. Almost all of
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
recover. The debug runs show the loop at its limit: 8,356–16,483 flushes in
180 s, averaging 4–5 operations each.

Consequences:
- **Batching cannot fix fan-out.** Batching and patch composition act on frame
  encoding, which is 0.5–2% of this CPU. Fan-out cannot converge in debug
  unless flush work stops scaling with the window.
- **Repeated query work is already the limit.** Loading and sorting alone are
  26–31% of the collection task, and whole-window work is 95–99% of it.

### 6. Latency on converged scenarios

- **First live update.** It arrived 107–160 ms after the burst started, one
  100 ms flush plus processing.
- **Hot keys, slow reader, and release unique burst.** Clients converged
  36–98 ms after the projector applied the last update.
- **Debug unique burst.** It took 147–356 ms, because each flush of its
  10,000-row window is slow in debug.

These numbers describe this machine and build only.

## Caveats

- **Debug build.** The canonical command is a plain `cargo test`,
  which builds unoptimized. Debug multiplies CPU-bound costs (cloning,
  comparing, and dropping `serde_json::Value` trees). That changes counts that
  depend on timing: flush cadence, redundant sends, and fan-out ingest (15–27
  versus 61–651 updates/s). Compare a debug run only with a debug run, and a
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
<summary>Debug run 1 (canonical), harness <code>c6d27cea</code></summary>

```json
{"scenario":"hot-keys","summaryVersion":1,"config":{"collectionCoalesceMs":100,"listBusCapacity":16384,"messageQueueSize":2048,"cacheEntitiesPerView":3000,"snapshotInitialBatch":50,"snapshotSubsequentBatch":100,"keyPattern":"hot","targetSourceRatePerSec":10000,"readPauseMs":0,"onDisconnect":"reconnect","serverThreads":4,"clientThreads":4,"build":"debug","overrides":[]},"fixture":{"seedRowJsonBytes":429,"sourcePatchJsonBytes":113},"seedRows":3000,"subscribers":1,"sourceUpdates":30000,"uniqueKeys":2652,"deliveredOperations":13955,"operations":{"upsert":13955,"patch":0,"remove":0,"delete":0},"coalescedSourceUpdates":30000,"websocketMessages":13987,"wireBytes":9126199,"decodedBytes":10326731,"liveMessages":13955,"liveWireBytes":8842094,"snapshotMessages":31,"snapshotWireBytes":283940,"snapshotRows":3000,"controlMessages":1,"trailingMessages":29,"frameEncodings":{"text":0,"binaryJson":13956,"binaryGzip":31},"resnapshots":0,"protocolErrors":0,"disconnects":0,"reconnects":0,"outcomes":{"drained":1},"server":{"messagesSent":13987,"resnapshots":0,"lagEvents":0,"droppedUpdates":0,"coalescedFlushes":31,"deliveryStopped":{}},"usage":{"updateMessages":13956,"updateBytes":8842259,"snapshotMessages":31,"snapshotRows":3000,"snapshotBytes":283940,"connections":1,"connectionsClosed":1},"publishedUpdates":30000,"publishMs":3046.7,"achievedSourceRatePerSec":9847.0,"quiescence":"flushed","firstLiveUpdateMs":122.0,"drainMs":3118.3,"drainAfterPublishMs":71.6,"perSubscriberSourceUpdate":{"deliveredOperations":0.4652,"liveMessages":0.4652,"liveWireBytes":294.7365},"perDeliveredOperation":{"liveMessages":1.0,"liveWireBytes":633.6148},"expectedStateDigest":"sha256:ee4d907d69ef14696f948d44f2bcf8049ffec47b2b136803978448826e16056c","finalStateDigest":"sha256:ee4d907d69ef14696f948d44f2bcf8049ffec47b2b136803978448826e16056c","converged":true,"problems":[],"clients":[{"client":0,"outcome":"drained","converged":true,"deliveredOperations":13955,"operations":{"upsert":13955,"patch":0,"remove":0,"delete":0},"websocketMessages":13987,"wireBytes":9126199,"decodedBytes":10326731,"liveMessages":13955,"liveWireBytes":8842094,"snapshotMessages":31,"snapshotWireBytes":283940,"snapshotRows":3000,"controlMessages":1,"trailingMessages":29,"frameEncodings":{"text":0,"binaryJson":13956,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":[],"firstLiveUpdateMs":122.0,"drainMs":3118.3,"finalStateDigest":"sha256:ee4d907d69ef14696f948d44f2bcf8049ffec47b2b136803978448826e16056c","finalRows":3000,"mismatchedRows":0,"mismatchSample":[]}]}
{"scenario":"unique-burst","summaryVersion":1,"config":{"collectionCoalesceMs":100,"listBusCapacity":16384,"messageQueueSize":2048,"cacheEntitiesPerView":10000,"snapshotInitialBatch":50,"snapshotSubsequentBatch":100,"keyPattern":"unique","targetSourceRatePerSec":10000,"readPauseMs":0,"onDisconnect":"reconnect","serverThreads":4,"clientThreads":4,"build":"debug","overrides":[]},"fixture":{"seedRowJsonBytes":429,"sourcePatchJsonBytes":113},"seedRows":10000,"subscribers":1,"sourceUpdates":10000,"uniqueKeys":10000,"deliveredOperations":13898,"operations":{"upsert":13898,"patch":0,"remove":0,"delete":0},"coalescedSourceUpdates":10000,"websocketMessages":14000,"wireBytes":9749937,"decodedBytes":13753833,"liveMessages":13898,"liveWireBytes":8805855,"snapshotMessages":101,"snapshotWireBytes":943917,"snapshotRows":10000,"controlMessages":1,"trailingMessages":3898,"frameEncodings":{"text":0,"binaryJson":13899,"binaryGzip":101},"resnapshots":0,"protocolErrors":0,"disconnects":0,"reconnects":0,"outcomes":{"drained":1},"server":{"messagesSent":14000,"resnapshots":0,"lagEvents":0,"droppedUpdates":0,"coalescedFlushes":8,"deliveryStopped":{}},"usage":{"updateMessages":13899,"updateBytes":8806020,"snapshotMessages":101,"snapshotRows":10000,"snapshotBytes":943917,"connections":1,"connectionsClosed":1},"publishedUpdates":10000,"publishMs":994.0,"achievedSourceRatePerSec":10061.0,"quiescence":"flushed","firstLiveUpdateMs":156.5,"drainMs":1141.2,"drainAfterPublishMs":147.2,"perSubscriberSourceUpdate":{"deliveredOperations":1.3898,"liveMessages":1.3898,"liveWireBytes":880.5855},"perDeliveredOperation":{"liveMessages":1.0,"liveWireBytes":633.6059},"expectedStateDigest":"sha256:4476a15f7d9f9b14aa3c1a6a76b156aefbc4de6a574412b6bd2a433c06b532de","finalStateDigest":"sha256:4476a15f7d9f9b14aa3c1a6a76b156aefbc4de6a574412b6bd2a433c06b532de","converged":true,"problems":[],"clients":[{"client":0,"outcome":"drained","converged":true,"deliveredOperations":13898,"operations":{"upsert":13898,"patch":0,"remove":0,"delete":0},"websocketMessages":14000,"wireBytes":9749937,"decodedBytes":13753833,"liveMessages":13898,"liveWireBytes":8805855,"snapshotMessages":101,"snapshotWireBytes":943917,"snapshotRows":10000,"controlMessages":1,"trailingMessages":3898,"frameEncodings":{"text":0,"binaryJson":13899,"binaryGzip":101},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":[],"firstLiveUpdateMs":156.5,"drainMs":1141.2,"finalStateDigest":"sha256:4476a15f7d9f9b14aa3c1a6a76b156aefbc4de6a574412b6bd2a433c06b532de","finalRows":10000,"mismatchedRows":0,"mismatchSample":[]}]}
{"scenario":"slow-reader","summaryVersion":1,"config":{"collectionCoalesceMs":100,"listBusCapacity":16384,"messageQueueSize":2048,"cacheEntitiesPerView":3000,"snapshotInitialBatch":50,"snapshotSubsequentBatch":100,"keyPattern":"hot","targetSourceRatePerSec":10000,"readPauseMs":2000,"onDisconnect":"reconnect","serverThreads":4,"clientThreads":4,"build":"debug","overrides":[]},"fixture":{"seedRowJsonBytes":429,"sourcePatchJsonBytes":113},"seedRows":3000,"subscribers":1,"sourceUpdates":30000,"uniqueKeys":2652,"deliveredOperations":6973,"operations":{"upsert":6973,"patch":0,"remove":0,"delete":0},"coalescedSourceUpdates":15842,"websocketMessages":7037,"wireBytes":5057282,"decodedBytes":7387411,"liveMessages":6973,"liveWireBytes":4418181,"snapshotMessages":62,"snapshotWireBytes":638771,"snapshotRows":6000,"controlMessages":2,"trailingMessages":0,"frameEncodings":{"text":0,"binaryJson":6975,"binaryGzip":62},"resnapshots":0,"protocolErrors":0,"disconnects":1,"reconnects":1,"outcomes":{"reconnected":1},"server":{"messagesSent":7037,"resnapshots":0,"lagEvents":0,"droppedUpdates":0,"coalescedFlushes":15,"deliveryStopped":{"send-failed":1}},"usage":{"updateMessages":6975,"updateBytes":4418511,"snapshotMessages":62,"snapshotRows":6000,"snapshotBytes":638771,"connections":2,"connectionsClosed":2},"publishedUpdates":30000,"publishMs":2999.2,"achievedSourceRatePerSec":10003.0,"quiescence":"grace","firstLiveUpdateMs":2007.2,"drainMs":3097.1,"drainAfterPublishMs":97.9,"perSubscriberSourceUpdate":{"deliveredOperations":0.2324,"liveMessages":0.2324,"liveWireBytes":147.2727},"perDeliveredOperation":{"liveMessages":1.0,"liveWireBytes":633.6126},"expectedStateDigest":"sha256:ee4d907d69ef14696f948d44f2bcf8049ffec47b2b136803978448826e16056c","finalStateDigest":"sha256:ee4d907d69ef14696f948d44f2bcf8049ffec47b2b136803978448826e16056c","converged":true,"problems":[],"clients":[{"client":0,"outcome":"reconnected","converged":true,"deliveredOperations":6973,"operations":{"upsert":6973,"patch":0,"remove":0,"delete":0},"websocketMessages":7037,"wireBytes":5057282,"decodedBytes":7387411,"liveMessages":6973,"liveWireBytes":4418181,"snapshotMessages":62,"snapshotWireBytes":638771,"snapshotRows":6000,"controlMessages":2,"trailingMessages":0,"frameEncodings":{"text":0,"binaryJson":6975,"binaryGzip":62},"initialSnapshots":2,"resnapshots":0,"disconnects":1,"disconnectDetails":[{"atMs":2106.7,"reason":"close without a status","messagesBefore":3303}],"reconnects":1,"protocolErrors":[],"failures":[],"firstLiveUpdateMs":2007.2,"drainMs":3097.1,"finalStateDigest":"sha256:ee4d907d69ef14696f948d44f2bcf8049ffec47b2b136803978448826e16056c","finalRows":3000,"mismatchedRows":0,"mismatchSample":[]}]}
{"scenario":"fan-out","summaryVersion":1,"config":{"collectionCoalesceMs":100,"listBusCapacity":16384,"messageQueueSize":2048,"cacheEntitiesPerView":3000,"snapshotInitialBatch":50,"snapshotSubsequentBatch":100,"keyPattern":"hot","targetSourceRatePerSec":10000,"readPauseMs":0,"onDisconnect":"reconnect","serverThreads":4,"clientThreads":4,"build":"debug","overrides":[]},"fixture":{"seedRowJsonBytes":429,"sourcePatchJsonBytes":113},"seedRows":3000,"subscribers":16,"sourceUpdates":30000,"uniqueKeys":2652,"deliveredOperations":85100,"operations":{"upsert":85100,"patch":0,"remove":0,"delete":0},"coalescedSourceUpdates":62245,"websocketMessages":85612,"wireBytes":58466327,"decodedBytes":77674774,"liveMessages":85100,"liveWireBytes":53920582,"snapshotMessages":496,"snapshotWireBytes":4543105,"snapshotRows":48000,"controlMessages":16,"trailingMessages":0,"frameEncodings":{"text":0,"binaryJson":85116,"binaryGzip":496},"resnapshots":0,"protocolErrors":0,"disconnects":0,"reconnects":0,"outcomes":{"failed":16},"server":{"messagesSent":86020,"resnapshots":0,"lagEvents":0,"droppedUpdates":0,"coalescedFlushes":16483,"deliveryStopped":{"send-failed":9}},"usage":{"updateMessages":85524,"updateBytes":54181778,"snapshotMessages":496,"snapshotRows":48000,"snapshotBytes":4543105,"connections":16,"connectionsClosed":16},"publishedUpdates":4906,"publishMs":179106.7,"achievedSourceRatePerSec":27.0,"quiescence":"grace","firstLiveUpdateMs":296.2,"drainMs":null,"drainAfterPublishMs":null,"perSubscriberSourceUpdate":{"deliveredOperations":0.1773,"liveMessages":0.1773,"liveWireBytes":112.3345},"perDeliveredOperation":{"liveMessages":1.0,"liveWireBytes":633.6144},"expectedStateDigest":"sha256:ee4d907d69ef14696f948d44f2bcf8049ffec47b2b136803978448826e16056c","finalStateDigest":null,"converged":false,"problems":["only 4906 of 30000 burst updates were applied before the deadline","client 0: timed out with 2545 rows still different from the final state","client 1: timed out with 2545 rows still different from the final state","client 2: timed out with 2545 rows still different from the final state","client 3: timed out with 2545 rows still different from the final state","client 4: timed out with 2545 rows still different from the final state","client 5: timed out with 2545 rows still different from the final state","client 6: timed out with 2545 rows still different from the final state","client 7: timed out with 2545 rows still different from the final state","client 8: timed out with 2545 rows still different from the final state","client 9: timed out with 2545 rows still different from the final state","client 10: timed out with 2545 rows still different from the final state","client 11: timed out with 2545 rows still different from the final state","client 12: timed out with 2545 rows still different from the final state","client 13: timed out with 2545 rows still different from the final state","client 14: timed out with 2545 rows still different from the final state","client 15: timed out with 2545 rows still different from the final state","0 of 16 subscribers reached the exact final state","subscribers do not share the expected final digest"],"clients":[{"client":0,"outcome":"failed","converged":false,"deliveredOperations":5153,"operations":{"upsert":5153,"patch":0,"remove":0,"delete":0},"websocketMessages":5185,"wireBytes":3549102,"decodedBytes":4749651,"liveMessages":5153,"liveWireBytes":3265014,"snapshotMessages":31,"snapshotWireBytes":283923,"snapshotRows":3000,"controlMessages":1,"trailingMessages":0,"frameEncodings":{"text":0,"binaryJson":5154,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":["timed out with 2545 rows still different from the final state"],"firstLiveUpdateMs":250.9,"drainMs":null,"finalStateDigest":"sha256:504b31e521dd8f3f316a7b10223844941cf315cf99caf6ce97e5f4f40208a6a4","finalRows":3000,"mismatchedRows":2545,"mismatchSample":["A894uNxVevPPr9WLPPaxpf9xdAgNn7kdDZbmgSYCJ8dG: expected _seq \"300100062:000000000271\" got \"300000002:000000000283\"","BDdR4aGkZuqzweTZtBytfb91LbByuB9kWrT4MRzmorsA: expected _seq \"300100073:000000000104\" got \"300100008:000000000124\"","CVtGdewUcGSo2UfKhtsQbRwoCU8jfhnRAQ4xzyU27Vss: expected _seq \"300100064:000000000081\" got \"300000002:000000000218\"","Fda5CYUMixRJVdRWLYdJhzGk7eD9VTzjgz4s2EobusRd: expected _seq \"300100074:000000000123\" got \"300000004:000000000209\"","HVmd9Et9v2JyZmGp4DSQ1ATmfaQHu1BnvWJBGnuFFD9j: expected _seq \"300100073:000000000056\" got \"300100009:000000000197\""]},{"client":1,"outcome":"failed","converged":false,"deliveredOperations":5555,"operations":{"upsert":5555,"patch":0,"remove":0,"delete":0},"websocketMessages":5587,"wireBytes":3803803,"decodedBytes":5004364,"liveMessages":5555,"liveWireBytes":3519727,"snapshotMessages":31,"snapshotWireBytes":283911,"snapshotRows":3000,"controlMessages":1,"trailingMessages":0,"frameEncodings":{"text":0,"binaryJson":5556,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":["timed out with 2545 rows still different from the final state"],"firstLiveUpdateMs":60.5,"drainMs":null,"finalStateDigest":"sha256:504b31e521dd8f3f316a7b10223844941cf315cf99caf6ce97e5f4f40208a6a4","finalRows":3000,"mismatchedRows":2545,"mismatchSample":["A894uNxVevPPr9WLPPaxpf9xdAgNn7kdDZbmgSYCJ8dG: expected _seq \"300100062:000000000271\" got \"300000002:000000000283\"","BDdR4aGkZuqzweTZtBytfb91LbByuB9kWrT4MRzmorsA: expected _seq \"300100073:000000000104\" got \"300100008:000000000124\"","CVtGdewUcGSo2UfKhtsQbRwoCU8jfhnRAQ4xzyU27Vss: expected _seq \"300100064:000000000081\" got \"300000002:000000000218\"","Fda5CYUMixRJVdRWLYdJhzGk7eD9VTzjgz4s2EobusRd: expected _seq \"300100074:000000000123\" got \"300000004:000000000209\"","HVmd9Et9v2JyZmGp4DSQ1ATmfaQHu1BnvWJBGnuFFD9j: expected _seq \"300100073:000000000056\" got \"300100009:000000000197\""]},{"client":2,"outcome":"failed","converged":false,"deliveredOperations":5537,"operations":{"upsert":5537,"patch":0,"remove":0,"delete":0},"websocketMessages":5569,"wireBytes":3792397,"decodedBytes":4992950,"liveMessages":5537,"liveWireBytes":3508313,"snapshotMessages":31,"snapshotWireBytes":283919,"snapshotRows":3000,"controlMessages":1,"trailingMessages":0,"frameEncodings":{"text":0,"binaryJson":5538,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":["timed out with 2545 rows still different from the final state"],"firstLiveUpdateMs":85.2,"drainMs":null,"finalStateDigest":"sha256:504b31e521dd8f3f316a7b10223844941cf315cf99caf6ce97e5f4f40208a6a4","finalRows":3000,"mismatchedRows":2545,"mismatchSample":["A894uNxVevPPr9WLPPaxpf9xdAgNn7kdDZbmgSYCJ8dG: expected _seq \"300100062:000000000271\" got \"300000002:000000000283\"","BDdR4aGkZuqzweTZtBytfb91LbByuB9kWrT4MRzmorsA: expected _seq \"300100073:000000000104\" got \"300100008:000000000124\"","CVtGdewUcGSo2UfKhtsQbRwoCU8jfhnRAQ4xzyU27Vss: expected _seq \"300100064:000000000081\" got \"300000002:000000000218\"","Fda5CYUMixRJVdRWLYdJhzGk7eD9VTzjgz4s2EobusRd: expected _seq \"300100074:000000000123\" got \"300000004:000000000209\"","HVmd9Et9v2JyZmGp4DSQ1ATmfaQHu1BnvWJBGnuFFD9j: expected _seq \"300100073:000000000056\" got \"300100009:000000000197\""]},{"client":3,"outcome":"failed","converged":false,"deliveredOperations":5437,"operations":{"upsert":5437,"patch":0,"remove":0,"delete":0},"websocketMessages":5469,"wireBytes":3729064,"decodedBytes":4929613,"liveMessages":5437,"liveWireBytes":3444976,"snapshotMessages":31,"snapshotWireBytes":283923,"snapshotRows":3000,"controlMessages":1,"trailingMessages":0,"frameEncodings":{"text":0,"binaryJson":5438,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":["timed out with 2545 rows still different from the final state"],"firstLiveUpdateMs":84.6,"drainMs":null,"finalStateDigest":"sha256:f5e5b4824e88e127e4c679dd49c7e00d53672e3a3bf026ce93d26a88fd1382cd","finalRows":3000,"mismatchedRows":2545,"mismatchSample":["A894uNxVevPPr9WLPPaxpf9xdAgNn7kdDZbmgSYCJ8dG: expected _seq \"300100062:000000000271\" got \"300000002:000000000283\"","BDdR4aGkZuqzweTZtBytfb91LbByuB9kWrT4MRzmorsA: expected _seq \"300100073:000000000104\" got \"300100008:000000000124\"","CVtGdewUcGSo2UfKhtsQbRwoCU8jfhnRAQ4xzyU27Vss: expected _seq \"300100064:000000000081\" got \"300000002:000000000218\"","Fda5CYUMixRJVdRWLYdJhzGk7eD9VTzjgz4s2EobusRd: expected _seq \"300100074:000000000123\" got \"300000004:000000000209\"","HVmd9Et9v2JyZmGp4DSQ1ATmfaQHu1BnvWJBGnuFFD9j: expected _seq \"300100073:000000000056\" got \"300100009:000000000197\""]},{"client":4,"outcome":"failed","converged":false,"deliveredOperations":5352,"operations":{"upsert":5352,"patch":0,"remove":0,"delete":0},"websocketMessages":5384,"wireBytes":3675169,"decodedBytes":4875723,"liveMessages":5352,"liveWireBytes":3391086,"snapshotMessages":31,"snapshotWireBytes":283918,"snapshotRows":3000,"controlMessages":1,"trailingMessages":0,"frameEncodings":{"text":0,"binaryJson":5353,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":["timed out with 2545 rows still different from the final state"],"firstLiveUpdateMs":268.0,"drainMs":null,"finalStateDigest":"sha256:504b31e521dd8f3f316a7b10223844941cf315cf99caf6ce97e5f4f40208a6a4","finalRows":3000,"mismatchedRows":2545,"mismatchSample":["A894uNxVevPPr9WLPPaxpf9xdAgNn7kdDZbmgSYCJ8dG: expected _seq \"300100062:000000000271\" got \"300000002:000000000283\"","BDdR4aGkZuqzweTZtBytfb91LbByuB9kWrT4MRzmorsA: expected _seq \"300100073:000000000104\" got \"300100008:000000000124\"","CVtGdewUcGSo2UfKhtsQbRwoCU8jfhnRAQ4xzyU27Vss: expected _seq \"300100064:000000000081\" got \"300000002:000000000218\"","Fda5CYUMixRJVdRWLYdJhzGk7eD9VTzjgz4s2EobusRd: expected _seq \"300100074:000000000123\" got \"300000004:000000000209\"","HVmd9Et9v2JyZmGp4DSQ1ATmfaQHu1BnvWJBGnuFFD9j: expected _seq \"300100073:000000000056\" got \"300100009:000000000197\""]},{"client":5,"outcome":"failed","converged":false,"deliveredOperations":5197,"operations":{"upsert":5197,"patch":0,"remove":0,"delete":0},"websocketMessages":5229,"wireBytes":3577154,"decodedBytes":4777496,"liveMessages":5197,"liveWireBytes":3292859,"snapshotMessages":31,"snapshotWireBytes":284130,"snapshotRows":3000,"controlMessages":1,"trailingMessages":0,"frameEncodings":{"text":0,"binaryJson":5198,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":["timed out with 2545 rows still different from the final state"],"firstLiveUpdateMs":278.8,"drainMs":null,"finalStateDigest":"sha256:f5e5b4824e88e127e4c679dd49c7e00d53672e3a3bf026ce93d26a88fd1382cd","finalRows":3000,"mismatchedRows":2545,"mismatchSample":["A894uNxVevPPr9WLPPaxpf9xdAgNn7kdDZbmgSYCJ8dG: expected _seq \"300100062:000000000271\" got \"300000002:000000000283\"","BDdR4aGkZuqzweTZtBytfb91LbByuB9kWrT4MRzmorsA: expected _seq \"300100073:000000000104\" got \"300100008:000000000124\"","CVtGdewUcGSo2UfKhtsQbRwoCU8jfhnRAQ4xzyU27Vss: expected _seq \"300100064:000000000081\" got \"300000002:000000000218\"","Fda5CYUMixRJVdRWLYdJhzGk7eD9VTzjgz4s2EobusRd: expected _seq \"300100074:000000000123\" got \"300000004:000000000209\"","HVmd9Et9v2JyZmGp4DSQ1ATmfaQHu1BnvWJBGnuFFD9j: expected _seq \"300100073:000000000056\" got \"300100009:000000000197\""]},{"client":6,"outcome":"failed","converged":false,"deliveredOperations":5166,"operations":{"upsert":5166,"patch":0,"remove":0,"delete":0},"websocketMessages":5198,"wireBytes":3557380,"decodedBytes":4757902,"liveMessages":5166,"liveWireBytes":3273265,"snapshotMessages":31,"snapshotWireBytes":283950,"snapshotRows":3000,"controlMessages":1,"trailingMessages":0,"frameEncodings":{"text":0,"binaryJson":5167,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":["timed out with 2545 rows still different from the final state"],"firstLiveUpdateMs":229.2,"drainMs":null,"finalStateDigest":"sha256:f5e5b4824e88e127e4c679dd49c7e00d53672e3a3bf026ce93d26a88fd1382cd","finalRows":3000,"mismatchedRows":2545,"mismatchSample":["A894uNxVevPPr9WLPPaxpf9xdAgNn7kdDZbmgSYCJ8dG: expected _seq \"300100062:000000000271\" got \"300000002:000000000283\"","BDdR4aGkZuqzweTZtBytfb91LbByuB9kWrT4MRzmorsA: expected _seq \"300100073:000000000104\" got \"300100008:000000000124\"","CVtGdewUcGSo2UfKhtsQbRwoCU8jfhnRAQ4xzyU27Vss: expected _seq \"300100064:000000000081\" got \"300000002:000000000218\"","Fda5CYUMixRJVdRWLYdJhzGk7eD9VTzjgz4s2EobusRd: expected _seq \"300100074:000000000123\" got \"300000004:000000000209\"","HVmd9Et9v2JyZmGp4DSQ1ATmfaQHu1BnvWJBGnuFFD9j: expected _seq \"300100073:000000000056\" got \"300100009:000000000197\""]},{"client":7,"outcome":"failed","converged":false,"deliveredOperations":5462,"operations":{"upsert":5462,"patch":0,"remove":0,"delete":0},"websocketMessages":5494,"wireBytes":3744871,"decodedBytes":4945448,"liveMessages":5462,"liveWireBytes":3460811,"snapshotMessages":31,"snapshotWireBytes":283895,"snapshotRows":3000,"controlMessages":1,"trailingMessages":0,"frameEncodings":{"text":0,"binaryJson":5463,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":["timed out with 2545 rows still different from the final state"],"firstLiveUpdateMs":255.2,"drainMs":null,"finalStateDigest":"sha256:504b31e521dd8f3f316a7b10223844941cf315cf99caf6ce97e5f4f40208a6a4","finalRows":3000,"mismatchedRows":2545,"mismatchSample":["A894uNxVevPPr9WLPPaxpf9xdAgNn7kdDZbmgSYCJ8dG: expected _seq \"300100062:000000000271\" got \"300000002:000000000283\"","BDdR4aGkZuqzweTZtBytfb91LbByuB9kWrT4MRzmorsA: expected _seq \"300100073:000000000104\" got \"300100008:000000000124\"","CVtGdewUcGSo2UfKhtsQbRwoCU8jfhnRAQ4xzyU27Vss: expected _seq \"300100064:000000000081\" got \"300000002:000000000218\"","Fda5CYUMixRJVdRWLYdJhzGk7eD9VTzjgz4s2EobusRd: expected _seq \"300100074:000000000123\" got \"300000004:000000000209\"","HVmd9Et9v2JyZmGp4DSQ1ATmfaQHu1BnvWJBGnuFFD9j: expected _seq \"300100073:000000000056\" got \"300100009:000000000197\""]},{"client":8,"outcome":"failed","converged":false,"deliveredOperations":5356,"operations":{"upsert":5356,"patch":0,"remove":0,"delete":0},"websocketMessages":5388,"wireBytes":3677741,"decodedBytes":4878281,"liveMessages":5356,"liveWireBytes":3393644,"snapshotMessages":31,"snapshotWireBytes":283932,"snapshotRows":3000,"controlMessages":1,"trailingMessages":0,"frameEncodings":{"text":0,"binaryJson":5357,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":["timed out with 2545 rows still different from the final state"],"firstLiveUpdateMs":296.2,"drainMs":null,"finalStateDigest":"sha256:504b31e521dd8f3f316a7b10223844941cf315cf99caf6ce97e5f4f40208a6a4","finalRows":3000,"mismatchedRows":2545,"mismatchSample":["A894uNxVevPPr9WLPPaxpf9xdAgNn7kdDZbmgSYCJ8dG: expected _seq \"300100062:000000000271\" got \"300000002:000000000283\"","BDdR4aGkZuqzweTZtBytfb91LbByuB9kWrT4MRzmorsA: expected _seq \"300100073:000000000104\" got \"300100008:000000000124\"","CVtGdewUcGSo2UfKhtsQbRwoCU8jfhnRAQ4xzyU27Vss: expected _seq \"300100064:000000000081\" got \"300000002:000000000218\"","Fda5CYUMixRJVdRWLYdJhzGk7eD9VTzjgz4s2EobusRd: expected _seq \"300100074:000000000123\" got \"300000004:000000000209\"","HVmd9Et9v2JyZmGp4DSQ1ATmfaQHu1BnvWJBGnuFFD9j: expected _seq \"300100073:000000000056\" got \"300100009:000000000197\""]},{"client":9,"outcome":"failed","converged":false,"deliveredOperations":5319,"operations":{"upsert":5319,"patch":0,"remove":0,"delete":0},"websocketMessages":5351,"wireBytes":3654338,"decodedBytes":4854834,"liveMessages":5319,"liveWireBytes":3370197,"snapshotMessages":31,"snapshotWireBytes":283976,"snapshotRows":3000,"controlMessages":1,"trailingMessages":0,"frameEncodings":{"text":0,"binaryJson":5320,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":["timed out with 2545 rows still different from the final state"],"firstLiveUpdateMs":247.9,"drainMs":null,"finalStateDigest":"sha256:504b31e521dd8f3f316a7b10223844941cf315cf99caf6ce97e5f4f40208a6a4","finalRows":3000,"mismatchedRows":2545,"mismatchSample":["A894uNxVevPPr9WLPPaxpf9xdAgNn7kdDZbmgSYCJ8dG: expected _seq \"300100062:000000000271\" got \"300000002:000000000283\"","BDdR4aGkZuqzweTZtBytfb91LbByuB9kWrT4MRzmorsA: expected _seq \"300100073:000000000104\" got \"300100008:000000000124\"","CVtGdewUcGSo2UfKhtsQbRwoCU8jfhnRAQ4xzyU27Vss: expected _seq \"300100064:000000000081\" got \"300000002:000000000218\"","Fda5CYUMixRJVdRWLYdJhzGk7eD9VTzjgz4s2EobusRd: expected _seq \"300100074:000000000123\" got \"300000004:000000000209\"","HVmd9Et9v2JyZmGp4DSQ1ATmfaQHu1BnvWJBGnuFFD9j: expected _seq \"300100073:000000000056\" got \"300100009:000000000197\""]},{"client":10,"outcome":"failed","converged":false,"deliveredOperations":5376,"operations":{"upsert":5376,"patch":0,"remove":0,"delete":0},"websocketMessages":5408,"wireBytes":3690414,"decodedBytes":4890952,"liveMessages":5376,"liveWireBytes":3406315,"snapshotMessages":31,"snapshotWireBytes":283934,"snapshotRows":3000,"controlMessages":1,"trailingMessages":0,"frameEncodings":{"text":0,"binaryJson":5377,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":["timed out with 2545 rows still different from the final state"],"firstLiveUpdateMs":240.0,"drainMs":null,"finalStateDigest":"sha256:504b31e521dd8f3f316a7b10223844941cf315cf99caf6ce97e5f4f40208a6a4","finalRows":3000,"mismatchedRows":2545,"mismatchSample":["A894uNxVevPPr9WLPPaxpf9xdAgNn7kdDZbmgSYCJ8dG: expected _seq \"300100062:000000000271\" got \"300000002:000000000283\"","BDdR4aGkZuqzweTZtBytfb91LbByuB9kWrT4MRzmorsA: expected _seq \"300100073:000000000104\" got \"300100008:000000000124\"","CVtGdewUcGSo2UfKhtsQbRwoCU8jfhnRAQ4xzyU27Vss: expected _seq \"300100064:000000000081\" got \"300000002:000000000218\"","Fda5CYUMixRJVdRWLYdJhzGk7eD9VTzjgz4s2EobusRd: expected _seq \"300100074:000000000123\" got \"300000004:000000000209\"","HVmd9Et9v2JyZmGp4DSQ1ATmfaQHu1BnvWJBGnuFFD9j: expected _seq \"300100073:000000000056\" got \"300100009:000000000197\""]},{"client":11,"outcome":"failed","converged":false,"deliveredOperations":4953,"operations":{"upsert":4953,"patch":0,"remove":0,"delete":0},"websocketMessages":4985,"wireBytes":3422376,"decodedBytes":4622893,"liveMessages":4953,"liveWireBytes":3138256,"snapshotMessages":31,"snapshotWireBytes":283955,"snapshotRows":3000,"controlMessages":1,"trailingMessages":0,"frameEncodings":{"text":0,"binaryJson":4954,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":["timed out with 2545 rows still different from the final state"],"firstLiveUpdateMs":169.8,"drainMs":null,"finalStateDigest":"sha256:504b31e521dd8f3f316a7b10223844941cf315cf99caf6ce97e5f4f40208a6a4","finalRows":3000,"mismatchedRows":2545,"mismatchSample":["A894uNxVevPPr9WLPPaxpf9xdAgNn7kdDZbmgSYCJ8dG: expected _seq \"300100062:000000000271\" got \"300000002:000000000283\"","BDdR4aGkZuqzweTZtBytfb91LbByuB9kWrT4MRzmorsA: expected _seq \"300100073:000000000104\" got \"300100008:000000000124\"","CVtGdewUcGSo2UfKhtsQbRwoCU8jfhnRAQ4xzyU27Vss: expected _seq \"300100064:000000000081\" got \"300000002:000000000218\"","Fda5CYUMixRJVdRWLYdJhzGk7eD9VTzjgz4s2EobusRd: expected _seq \"300100074:000000000123\" got \"300000004:000000000209\"","HVmd9Et9v2JyZmGp4DSQ1ATmfaQHu1BnvWJBGnuFFD9j: expected _seq \"300100073:000000000056\" got \"300100009:000000000197\""]},{"client":12,"outcome":"failed","converged":false,"deliveredOperations":5380,"operations":{"upsert":5380,"patch":0,"remove":0,"delete":0},"websocketMessages":5412,"wireBytes":3692961,"decodedBytes":4893490,"liveMessages":5380,"liveWireBytes":3408853,"snapshotMessages":31,"snapshotWireBytes":283943,"snapshotRows":3000,"controlMessages":1,"trailingMessages":0,"frameEncodings":{"text":0,"binaryJson":5381,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":["timed out with 2545 rows still different from the final state"],"firstLiveUpdateMs":174.5,"drainMs":null,"finalStateDigest":"sha256:504b31e521dd8f3f316a7b10223844941cf315cf99caf6ce97e5f4f40208a6a4","finalRows":3000,"mismatchedRows":2545,"mismatchSample":["A894uNxVevPPr9WLPPaxpf9xdAgNn7kdDZbmgSYCJ8dG: expected _seq \"300100062:000000000271\" got \"300000002:000000000283\"","BDdR4aGkZuqzweTZtBytfb91LbByuB9kWrT4MRzmorsA: expected _seq \"300100073:000000000104\" got \"300100008:000000000124\"","CVtGdewUcGSo2UfKhtsQbRwoCU8jfhnRAQ4xzyU27Vss: expected _seq \"300100064:000000000081\" got \"300000002:000000000218\"","Fda5CYUMixRJVdRWLYdJhzGk7eD9VTzjgz4s2EobusRd: expected _seq \"300100074:000000000123\" got \"300000004:000000000209\"","HVmd9Et9v2JyZmGp4DSQ1ATmfaQHu1BnvWJBGnuFFD9j: expected _seq \"300100073:000000000056\" got \"300100009:000000000197\""]},{"client":13,"outcome":"failed","converged":false,"deliveredOperations":5241,"operations":{"upsert":5241,"patch":0,"remove":0,"delete":0},"websocketMessages":5273,"wireBytes":3604905,"decodedBytes":4805415,"liveMessages":5241,"liveWireBytes":3320778,"snapshotMessages":31,"snapshotWireBytes":283962,"snapshotRows":3000,"controlMessages":1,"trailingMessages":0,"frameEncodings":{"text":0,"binaryJson":5242,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":["timed out with 2545 rows still different from the final state"],"firstLiveUpdateMs":278.6,"drainMs":null,"finalStateDigest":"sha256:504b31e521dd8f3f316a7b10223844941cf315cf99caf6ce97e5f4f40208a6a4","finalRows":3000,"mismatchedRows":2545,"mismatchSample":["A894uNxVevPPr9WLPPaxpf9xdAgNn7kdDZbmgSYCJ8dG: expected _seq \"300100062:000000000271\" got \"300000002:000000000283\"","BDdR4aGkZuqzweTZtBytfb91LbByuB9kWrT4MRzmorsA: expected _seq \"300100073:000000000104\" got \"300100008:000000000124\"","CVtGdewUcGSo2UfKhtsQbRwoCU8jfhnRAQ4xzyU27Vss: expected _seq \"300100064:000000000081\" got \"300000002:000000000218\"","Fda5CYUMixRJVdRWLYdJhzGk7eD9VTzjgz4s2EobusRd: expected _seq \"300100074:000000000123\" got \"300000004:000000000209\"","HVmd9Et9v2JyZmGp4DSQ1ATmfaQHu1BnvWJBGnuFFD9j: expected _seq \"300100073:000000000056\" got \"300100009:000000000197\""]},{"client":14,"outcome":"failed","converged":false,"deliveredOperations":5526,"operations":{"upsert":5526,"patch":0,"remove":0,"delete":0},"websocketMessages":5558,"wireBytes":3785436,"decodedBytes":4986011,"liveMessages":5526,"liveWireBytes":3501374,"snapshotMessages":31,"snapshotWireBytes":283897,"snapshotRows":3000,"controlMessages":1,"trailingMessages":0,"frameEncodings":{"text":0,"binaryJson":5527,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":["timed out with 2545 rows still different from the final state"],"firstLiveUpdateMs":170.0,"drainMs":null,"finalStateDigest":"sha256:504b31e521dd8f3f316a7b10223844941cf315cf99caf6ce97e5f4f40208a6a4","finalRows":3000,"mismatchedRows":2545,"mismatchSample":["A894uNxVevPPr9WLPPaxpf9xdAgNn7kdDZbmgSYCJ8dG: expected _seq \"300100062:000000000271\" got \"300000002:000000000283\"","BDdR4aGkZuqzweTZtBytfb91LbByuB9kWrT4MRzmorsA: expected _seq \"300100073:000000000104\" got \"300100008:000000000124\"","CVtGdewUcGSo2UfKhtsQbRwoCU8jfhnRAQ4xzyU27Vss: expected _seq \"300100064:000000000081\" got \"300000002:000000000218\"","Fda5CYUMixRJVdRWLYdJhzGk7eD9VTzjgz4s2EobusRd: expected _seq \"300100074:000000000123\" got \"300000004:000000000209\"","HVmd9Et9v2JyZmGp4DSQ1ATmfaQHu1BnvWJBGnuFFD9j: expected _seq \"300100073:000000000056\" got \"300100009:000000000197\""]},{"client":15,"outcome":"failed","converged":false,"deliveredOperations":5090,"operations":{"upsert":5090,"patch":0,"remove":0,"delete":0},"websocketMessages":5122,"wireBytes":3509216,"decodedBytes":4709751,"liveMessages":5090,"liveWireBytes":3225114,"snapshotMessages":31,"snapshotWireBytes":283937,"snapshotRows":3000,"controlMessages":1,"trailingMessages":0,"frameEncodings":{"text":0,"binaryJson":5091,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":["timed out with 2545 rows still different from the final state"],"firstLiveUpdateMs":237.2,"drainMs":null,"finalStateDigest":"sha256:504b31e521dd8f3f316a7b10223844941cf315cf99caf6ce97e5f4f40208a6a4","finalRows":3000,"mismatchedRows":2545,"mismatchSample":["A894uNxVevPPr9WLPPaxpf9xdAgNn7kdDZbmgSYCJ8dG: expected _seq \"300100062:000000000271\" got \"300000002:000000000283\"","BDdR4aGkZuqzweTZtBytfb91LbByuB9kWrT4MRzmorsA: expected _seq \"300100073:000000000104\" got \"300100008:000000000124\"","CVtGdewUcGSo2UfKhtsQbRwoCU8jfhnRAQ4xzyU27Vss: expected _seq \"300100064:000000000081\" got \"300000002:000000000218\"","Fda5CYUMixRJVdRWLYdJhzGk7eD9VTzjgz4s2EobusRd: expected _seq \"300100074:000000000123\" got \"300000004:000000000209\"","HVmd9Et9v2JyZmGp4DSQ1ATmfaQHu1BnvWJBGnuFFD9j: expected _seq \"300100073:000000000056\" got \"300100009:000000000197\""]}]}
```

</details>

<details>
<summary>Release run 3 (the release run whose fan-out converged), harness <code>c6d27cea</code></summary>

```json
{"scenario":"hot-keys","summaryVersion":1,"config":{"collectionCoalesceMs":100,"listBusCapacity":16384,"messageQueueSize":2048,"cacheEntitiesPerView":3000,"snapshotInitialBatch":50,"snapshotSubsequentBatch":100,"keyPattern":"hot","targetSourceRatePerSec":10000,"readPauseMs":0,"onDisconnect":"reconnect","serverThreads":4,"clientThreads":4,"build":"release","overrides":[]},"fixture":{"seedRowJsonBytes":429,"sourcePatchJsonBytes":113},"seedRows":3000,"subscribers":1,"sourceUpdates":30000,"uniqueKeys":2652,"deliveredOperations":13673,"operations":{"upsert":13673,"patch":0,"remove":0,"delete":0},"coalescedSourceUpdates":30000,"websocketMessages":13705,"wireBytes":8947541,"decodedBytes":10148076,"liveMessages":13673,"liveWireBytes":8663439,"snapshotMessages":31,"snapshotWireBytes":283937,"snapshotRows":3000,"controlMessages":1,"trailingMessages":0,"frameEncodings":{"text":0,"binaryJson":13674,"binaryGzip":31},"resnapshots":0,"protocolErrors":0,"disconnects":0,"reconnects":0,"outcomes":{"drained":1},"server":{"messagesSent":13705,"resnapshots":0,"lagEvents":0,"droppedUpdates":0,"coalescedFlushes":30,"deliveryStopped":{}},"usage":{"updateMessages":13674,"updateBytes":8663604,"snapshotMessages":31,"snapshotRows":3000,"snapshotBytes":283937,"connections":1,"connectionsClosed":1},"publishedUpdates":30000,"publishMs":2993.4,"achievedSourceRatePerSec":10022.0,"quiescence":"flushed","firstLiveUpdateMs":108.0,"drainMs":3062.7,"drainAfterPublishMs":69.3,"perSubscriberSourceUpdate":{"deliveredOperations":0.4558,"liveMessages":0.4558,"liveWireBytes":288.7813},"perDeliveredOperation":{"liveMessages":1.0,"liveWireBytes":633.6165},"expectedStateDigest":"sha256:ee4d907d69ef14696f948d44f2bcf8049ffec47b2b136803978448826e16056c","finalStateDigest":"sha256:ee4d907d69ef14696f948d44f2bcf8049ffec47b2b136803978448826e16056c","converged":true,"problems":[],"clients":[{"client":0,"outcome":"drained","converged":true,"deliveredOperations":13673,"operations":{"upsert":13673,"patch":0,"remove":0,"delete":0},"websocketMessages":13705,"wireBytes":8947541,"decodedBytes":10148076,"liveMessages":13673,"liveWireBytes":8663439,"snapshotMessages":31,"snapshotWireBytes":283937,"snapshotRows":3000,"controlMessages":1,"trailingMessages":0,"frameEncodings":{"text":0,"binaryJson":13674,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":[],"firstLiveUpdateMs":108.0,"drainMs":3062.7,"finalStateDigest":"sha256:ee4d907d69ef14696f948d44f2bcf8049ffec47b2b136803978448826e16056c","finalRows":3000,"mismatchedRows":0,"mismatchSample":[]}]}
{"scenario":"unique-burst","summaryVersion":1,"config":{"collectionCoalesceMs":100,"listBusCapacity":16384,"messageQueueSize":2048,"cacheEntitiesPerView":10000,"snapshotInitialBatch":50,"snapshotSubsequentBatch":100,"keyPattern":"unique","targetSourceRatePerSec":10000,"readPauseMs":0,"onDisconnect":"reconnect","serverThreads":4,"clientThreads":4,"build":"release","overrides":[]},"fixture":{"seedRowJsonBytes":429,"sourcePatchJsonBytes":113},"seedRows":10000,"subscribers":1,"sourceUpdates":10000,"uniqueKeys":10000,"deliveredOperations":10000,"operations":{"upsert":10000,"patch":0,"remove":0,"delete":0},"coalescedSourceUpdates":10000,"websocketMessages":10102,"wireBytes":7280232,"decodedBytes":11284061,"liveMessages":10000,"liveWireBytes":6336083,"snapshotMessages":101,"snapshotWireBytes":943984,"snapshotRows":10000,"controlMessages":1,"trailingMessages":0,"frameEncodings":{"text":0,"binaryJson":10001,"binaryGzip":101},"resnapshots":0,"protocolErrors":0,"disconnects":0,"reconnects":0,"outcomes":{"drained":1},"server":{"messagesSent":10102,"resnapshots":0,"lagEvents":0,"droppedUpdates":0,"coalescedFlushes":10,"deliveryStopped":{}},"usage":{"updateMessages":10001,"updateBytes":6336248,"snapshotMessages":101,"snapshotRows":10000,"snapshotBytes":943984,"connections":1,"connectionsClosed":1},"publishedUpdates":10000,"publishMs":992.8,"achievedSourceRatePerSec":10072.0,"quiescence":"flushed","firstLiveUpdateMs":128.3,"drainMs":1036.3,"drainAfterPublishMs":43.5,"perSubscriberSourceUpdate":{"deliveredOperations":1.0,"liveMessages":1.0,"liveWireBytes":633.6083},"perDeliveredOperation":{"liveMessages":1.0,"liveWireBytes":633.6083},"expectedStateDigest":"sha256:4476a15f7d9f9b14aa3c1a6a76b156aefbc4de6a574412b6bd2a433c06b532de","finalStateDigest":"sha256:4476a15f7d9f9b14aa3c1a6a76b156aefbc4de6a574412b6bd2a433c06b532de","converged":true,"problems":[],"clients":[{"client":0,"outcome":"drained","converged":true,"deliveredOperations":10000,"operations":{"upsert":10000,"patch":0,"remove":0,"delete":0},"websocketMessages":10102,"wireBytes":7280232,"decodedBytes":11284061,"liveMessages":10000,"liveWireBytes":6336083,"snapshotMessages":101,"snapshotWireBytes":943984,"snapshotRows":10000,"controlMessages":1,"trailingMessages":0,"frameEncodings":{"text":0,"binaryJson":10001,"binaryGzip":101},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":[],"firstLiveUpdateMs":128.3,"drainMs":1036.3,"finalStateDigest":"sha256:4476a15f7d9f9b14aa3c1a6a76b156aefbc4de6a574412b6bd2a433c06b532de","finalRows":10000,"mismatchedRows":0,"mismatchSample":[]}]}
{"scenario":"slow-reader","summaryVersion":1,"config":{"collectionCoalesceMs":100,"listBusCapacity":16384,"messageQueueSize":2048,"cacheEntitiesPerView":3000,"snapshotInitialBatch":50,"snapshotSubsequentBatch":100,"keyPattern":"hot","targetSourceRatePerSec":10000,"readPauseMs":2000,"onDisconnect":"reconnect","serverThreads":4,"clientThreads":4,"build":"release","overrides":[]},"fixture":{"seedRowJsonBytes":429,"sourcePatchJsonBytes":113},"seedRows":3000,"subscribers":1,"sourceUpdates":30000,"uniqueKeys":2652,"deliveredOperations":7765,"operations":{"upsert":7765,"patch":0,"remove":0,"delete":0},"coalescedSourceUpdates":16800,"websocketMessages":7829,"wireBytes":5557710,"decodedBytes":7889228,"liveMessages":7765,"liveWireBytes":4919998,"snapshotMessages":62,"snapshotWireBytes":637382,"snapshotRows":6000,"controlMessages":2,"trailingMessages":0,"frameEncodings":{"text":0,"binaryJson":7767,"binaryGzip":62},"resnapshots":0,"protocolErrors":0,"disconnects":1,"reconnects":1,"outcomes":{"reconnected":1},"server":{"messagesSent":7829,"resnapshots":0,"lagEvents":0,"droppedUpdates":0,"coalescedFlushes":17,"deliveryStopped":{"send-failed":1}},"usage":{"updateMessages":7767,"updateBytes":4920328,"snapshotMessages":62,"snapshotRows":6000,"snapshotBytes":637382,"connections":2,"connectionsClosed":2},"publishedUpdates":30000,"publishMs":2993.6,"achievedSourceRatePerSec":10021.0,"quiescence":"grace","firstLiveUpdateMs":2002.0,"drainMs":3054.7,"drainAfterPublishMs":61.1,"perSubscriberSourceUpdate":{"deliveredOperations":0.2588,"liveMessages":0.2588,"liveWireBytes":163.9999},"perDeliveredOperation":{"liveMessages":1.0,"liveWireBytes":633.6121},"expectedStateDigest":"sha256:ee4d907d69ef14696f948d44f2bcf8049ffec47b2b136803978448826e16056c","finalStateDigest":"sha256:ee4d907d69ef14696f948d44f2bcf8049ffec47b2b136803978448826e16056c","converged":true,"problems":[],"clients":[{"client":0,"outcome":"reconnected","converged":true,"deliveredOperations":7765,"operations":{"upsert":7765,"patch":0,"remove":0,"delete":0},"websocketMessages":7829,"wireBytes":5557710,"decodedBytes":7889228,"liveMessages":7765,"liveWireBytes":4919998,"snapshotMessages":62,"snapshotWireBytes":637382,"snapshotRows":6000,"controlMessages":2,"trailingMessages":0,"frameEncodings":{"text":0,"binaryJson":7767,"binaryGzip":62},"initialSnapshots":2,"resnapshots":0,"disconnects":1,"disconnectDetails":[{"atMs":2020.1,"reason":"close without a status","messagesBefore":3304}],"reconnects":1,"protocolErrors":[],"failures":[],"firstLiveUpdateMs":2002.0,"drainMs":3054.7,"finalStateDigest":"sha256:ee4d907d69ef14696f948d44f2bcf8049ffec47b2b136803978448826e16056c","finalRows":3000,"mismatchedRows":0,"mismatchSample":[]}]}
{"scenario":"fan-out","summaryVersion":1,"config":{"collectionCoalesceMs":100,"listBusCapacity":16384,"messageQueueSize":2048,"cacheEntitiesPerView":3000,"snapshotInitialBatch":50,"snapshotSubsequentBatch":100,"keyPattern":"hot","targetSourceRatePerSec":10000,"readPauseMs":0,"onDisconnect":"reconnect","serverThreads":4,"clientThreads":4,"build":"release","overrides":[]},"fixture":{"seedRowJsonBytes":429,"sourcePatchJsonBytes":113},"seedRows":3000,"subscribers":16,"sourceUpdates":30000,"uniqueKeys":2652,"deliveredOperations":503653,"operations":{"upsert":503653,"patch":0,"remove":0,"delete":0},"coalescedSourceUpdates":480000,"websocketMessages":504165,"wireBytes":323665264,"decodedBytes":342873751,"liveMessages":503653,"liveWireBytes":319119559,"snapshotMessages":496,"snapshotWireBytes":4543065,"snapshotRows":48000,"controlMessages":16,"trailingMessages":16,"frameEncodings":{"text":0,"binaryJson":503669,"binaryGzip":496},"resnapshots":0,"protocolErrors":0,"disconnects":0,"reconnects":0,"outcomes":{"drained":16},"server":{"messagesSent":504165,"resnapshots":0,"lagEvents":0,"droppedUpdates":0,"coalescedFlushes":20509,"deliveryStopped":{}},"usage":{"updateMessages":503669,"updateBytes":319122199,"snapshotMessages":496,"snapshotRows":48000,"snapshotBytes":4543065,"connections":16,"connectionsClosed":16},"publishedUpdates":30000,"publishMs":170547.7,"achievedSourceRatePerSec":176.0,"quiescence":"flushed","firstLiveUpdateMs":128.0,"drainMs":170669.6,"drainAfterPublishMs":121.9,"perSubscriberSourceUpdate":{"deliveredOperations":1.0493,"liveMessages":1.0493,"liveWireBytes":664.8324},"perDeliveredOperation":{"liveMessages":1.0,"liveWireBytes":633.61},"expectedStateDigest":"sha256:ee4d907d69ef14696f948d44f2bcf8049ffec47b2b136803978448826e16056c","finalStateDigest":"sha256:ee4d907d69ef14696f948d44f2bcf8049ffec47b2b136803978448826e16056c","converged":true,"problems":[],"clients":[{"client":0,"outcome":"drained","converged":true,"deliveredOperations":31926,"operations":{"upsert":31926,"patch":0,"remove":0,"delete":0},"websocketMessages":31958,"wireBytes":20512707,"decodedBytes":21713228,"liveMessages":31926,"liveWireBytes":20228591,"snapshotMessages":31,"snapshotWireBytes":283951,"snapshotRows":3000,"controlMessages":1,"trailingMessages":1,"frameEncodings":{"text":0,"binaryJson":31927,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":[],"firstLiveUpdateMs":52.0,"drainMs":170654.1,"finalStateDigest":"sha256:ee4d907d69ef14696f948d44f2bcf8049ffec47b2b136803978448826e16056c","finalRows":3000,"mismatchedRows":0,"mismatchSample":[]},{"client":1,"outcome":"drained","converged":true,"deliveredOperations":31092,"operations":{"upsert":31092,"patch":0,"remove":0,"delete":0},"websocketMessages":31124,"wireBytes":19984316,"decodedBytes":21184877,"liveMessages":31092,"liveWireBytes":19700240,"snapshotMessages":31,"snapshotWireBytes":283911,"snapshotRows":3000,"controlMessages":1,"trailingMessages":1,"frameEncodings":{"text":0,"binaryJson":31093,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":[],"firstLiveUpdateMs":111.8,"drainMs":170641.6,"finalStateDigest":"sha256:ee4d907d69ef14696f948d44f2bcf8049ffec47b2b136803978448826e16056c","finalRows":3000,"mismatchedRows":0,"mismatchSample":[]},{"client":2,"outcome":"drained","converged":true,"deliveredOperations":31489,"operations":{"upsert":31489,"patch":0,"remove":0,"delete":0},"websocketMessages":31521,"wireBytes":20235905,"decodedBytes":21436444,"liveMessages":31489,"liveWireBytes":19951807,"snapshotMessages":31,"snapshotWireBytes":283933,"snapshotRows":3000,"controlMessages":1,"trailingMessages":1,"frameEncodings":{"text":0,"binaryJson":31490,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":[],"firstLiveUpdateMs":113.4,"drainMs":170669.6,"finalStateDigest":"sha256:ee4d907d69ef14696f948d44f2bcf8049ffec47b2b136803978448826e16056c","finalRows":3000,"mismatchedRows":0,"mismatchSample":[]},{"client":3,"outcome":"drained","converged":true,"deliveredOperations":31403,"operations":{"upsert":31403,"patch":0,"remove":0,"delete":0},"websocketMessages":31435,"wireBytes":20181232,"decodedBytes":21381761,"liveMessages":31403,"liveWireBytes":19897124,"snapshotMessages":31,"snapshotWireBytes":283943,"snapshotRows":3000,"controlMessages":1,"trailingMessages":1,"frameEncodings":{"text":0,"binaryJson":31404,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":[],"firstLiveUpdateMs":128.0,"drainMs":170652.0,"finalStateDigest":"sha256:ee4d907d69ef14696f948d44f2bcf8049ffec47b2b136803978448826e16056c","finalRows":3000,"mismatchedRows":0,"mismatchSample":[]},{"client":4,"outcome":"drained","converged":true,"deliveredOperations":31116,"operations":{"upsert":31116,"patch":0,"remove":0,"delete":0},"websocketMessages":31148,"wireBytes":19999567,"decodedBytes":21200083,"liveMessages":31116,"liveWireBytes":19715446,"snapshotMessages":31,"snapshotWireBytes":283956,"snapshotRows":3000,"controlMessages":1,"trailingMessages":1,"frameEncodings":{"text":0,"binaryJson":31117,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":[],"firstLiveUpdateMs":96.9,"drainMs":170638.5,"finalStateDigest":"sha256:ee4d907d69ef14696f948d44f2bcf8049ffec47b2b136803978448826e16056c","finalRows":3000,"mismatchedRows":0,"mismatchSample":[]},{"client":5,"outcome":"drained","converged":true,"deliveredOperations":31742,"operations":{"upsert":31742,"patch":0,"remove":0,"delete":0},"websocketMessages":31774,"wireBytes":20396188,"decodedBytes":21596645,"liveMessages":31742,"liveWireBytes":20112008,"snapshotMessages":31,"snapshotWireBytes":284015,"snapshotRows":3000,"controlMessages":1,"trailingMessages":1,"frameEncodings":{"text":0,"binaryJson":31743,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":[],"firstLiveUpdateMs":76.4,"drainMs":170639.0,"finalStateDigest":"sha256:ee4d907d69ef14696f948d44f2bcf8049ffec47b2b136803978448826e16056c","finalRows":3000,"mismatchedRows":0,"mismatchSample":[]},{"client":6,"outcome":"drained","converged":true,"deliveredOperations":31835,"operations":{"upsert":31835,"patch":0,"remove":0,"delete":0},"websocketMessages":31867,"wireBytes":20455112,"decodedBytes":21655632,"liveMessages":31835,"liveWireBytes":20170995,"snapshotMessages":31,"snapshotWireBytes":283952,"snapshotRows":3000,"controlMessages":1,"trailingMessages":1,"frameEncodings":{"text":0,"binaryJson":31836,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":[],"firstLiveUpdateMs":77.2,"drainMs":170640.0,"finalStateDigest":"sha256:ee4d907d69ef14696f948d44f2bcf8049ffec47b2b136803978448826e16056c","finalRows":3000,"mismatchedRows":0,"mismatchSample":[]},{"client":7,"outcome":"drained","converged":true,"deliveredOperations":31903,"operations":{"upsert":31903,"patch":0,"remove":0,"delete":0},"websocketMessages":31935,"wireBytes":20498151,"decodedBytes":21698675,"liveMessages":31903,"liveWireBytes":20214038,"snapshotMessages":31,"snapshotWireBytes":283948,"snapshotRows":3000,"controlMessages":1,"trailingMessages":1,"frameEncodings":{"text":0,"binaryJson":31904,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":[],"firstLiveUpdateMs":115.5,"drainMs":170654.4,"finalStateDigest":"sha256:ee4d907d69ef14696f948d44f2bcf8049ffec47b2b136803978448826e16056c","finalRows":3000,"mismatchedRows":0,"mismatchSample":[]},{"client":8,"outcome":"drained","converged":true,"deliveredOperations":31521,"operations":{"upsert":31521,"patch":0,"remove":0,"delete":0},"websocketMessages":31553,"wireBytes":20256100,"decodedBytes":21456647,"liveMessages":31521,"liveWireBytes":19972010,"snapshotMessages":31,"snapshotWireBytes":283925,"snapshotRows":3000,"controlMessages":1,"trailingMessages":1,"frameEncodings":{"text":0,"binaryJson":31522,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":[],"firstLiveUpdateMs":61.4,"drainMs":170637.9,"finalStateDigest":"sha256:ee4d907d69ef14696f948d44f2bcf8049ffec47b2b136803978448826e16056c","finalRows":3000,"mismatchedRows":0,"mismatchSample":[]},{"client":9,"outcome":"drained","converged":true,"deliveredOperations":30845,"operations":{"upsert":30845,"patch":0,"remove":0,"delete":0},"websocketMessages":30877,"wireBytes":19827723,"decodedBytes":21028301,"liveMessages":30845,"liveWireBytes":19543664,"snapshotMessages":31,"snapshotWireBytes":283894,"snapshotRows":3000,"controlMessages":1,"trailingMessages":1,"frameEncodings":{"text":0,"binaryJson":30846,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":[],"firstLiveUpdateMs":19.8,"drainMs":170626.8,"finalStateDigest":"sha256:ee4d907d69ef14696f948d44f2bcf8049ffec47b2b136803978448826e16056c","finalRows":3000,"mismatchedRows":0,"mismatchSample":[]},{"client":10,"outcome":"drained","converged":true,"deliveredOperations":31546,"operations":{"upsert":31546,"patch":0,"remove":0,"delete":0},"websocketMessages":31578,"wireBytes":20271956,"decodedBytes":21472477,"liveMessages":31546,"liveWireBytes":19987840,"snapshotMessages":31,"snapshotWireBytes":283951,"snapshotRows":3000,"controlMessages":1,"trailingMessages":1,"frameEncodings":{"text":0,"binaryJson":31547,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":[],"firstLiveUpdateMs":35.7,"drainMs":170635.2,"finalStateDigest":"sha256:ee4d907d69ef14696f948d44f2bcf8049ffec47b2b136803978448826e16056c","finalRows":3000,"mismatchedRows":0,"mismatchSample":[]},{"client":11,"outcome":"drained","converged":true,"deliveredOperations":31394,"operations":{"upsert":31394,"patch":0,"remove":0,"delete":0},"websocketMessages":31426,"wireBytes":20175660,"decodedBytes":21376194,"liveMessages":31394,"liveWireBytes":19891557,"snapshotMessages":31,"snapshotWireBytes":283938,"snapshotRows":3000,"controlMessages":1,"trailingMessages":1,"frameEncodings":{"text":0,"binaryJson":31395,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":[],"firstLiveUpdateMs":59.9,"drainMs":170640.5,"finalStateDigest":"sha256:ee4d907d69ef14696f948d44f2bcf8049ffec47b2b136803978448826e16056c","finalRows":3000,"mismatchedRows":0,"mismatchSample":[]},{"client":12,"outcome":"drained","converged":true,"deliveredOperations":31521,"operations":{"upsert":31521,"patch":0,"remove":0,"delete":0},"websocketMessages":31553,"wireBytes":20256191,"decodedBytes":21456713,"liveMessages":31521,"liveWireBytes":19972076,"snapshotMessages":31,"snapshotWireBytes":283950,"snapshotRows":3000,"controlMessages":1,"trailingMessages":1,"frameEncodings":{"text":0,"binaryJson":31522,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":[],"firstLiveUpdateMs":103.6,"drainMs":170648.5,"finalStateDigest":"sha256:ee4d907d69ef14696f948d44f2bcf8049ffec47b2b136803978448826e16056c","finalRows":3000,"mismatchedRows":0,"mismatchSample":[]},{"client":13,"outcome":"drained","converged":true,"deliveredOperations":31530,"operations":{"upsert":31530,"patch":0,"remove":0,"delete":0},"websocketMessages":31562,"wireBytes":20261812,"decodedBytes":21462354,"liveMessages":31530,"liveWireBytes":19977717,"snapshotMessages":31,"snapshotWireBytes":283930,"snapshotRows":3000,"controlMessages":1,"trailingMessages":1,"frameEncodings":{"text":0,"binaryJson":31531,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":[],"firstLiveUpdateMs":91.4,"drainMs":170664.4,"finalStateDigest":"sha256:ee4d907d69ef14696f948d44f2bcf8049ffec47b2b136803978448826e16056c","finalRows":3000,"mismatchedRows":0,"mismatchSample":[]},{"client":14,"outcome":"drained","converged":true,"deliveredOperations":31200,"operations":{"upsert":31200,"patch":0,"remove":0,"delete":0},"websocketMessages":31232,"wireBytes":20052718,"decodedBytes":21253244,"liveMessages":31200,"liveWireBytes":19768607,"snapshotMessages":31,"snapshotWireBytes":283946,"snapshotRows":3000,"controlMessages":1,"trailingMessages":1,"frameEncodings":{"text":0,"binaryJson":31201,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":[],"firstLiveUpdateMs":83.8,"drainMs":170654.7,"finalStateDigest":"sha256:ee4d907d69ef14696f948d44f2bcf8049ffec47b2b136803978448826e16056c","finalRows":3000,"mismatchedRows":0,"mismatchSample":[]},{"client":15,"outcome":"drained","converged":true,"deliveredOperations":31590,"operations":{"upsert":31590,"patch":0,"remove":0,"delete":0},"websocketMessages":31622,"wireBytes":20299926,"decodedBytes":21500476,"liveMessages":31590,"liveWireBytes":20015839,"snapshotMessages":31,"snapshotWireBytes":283922,"snapshotRows":3000,"controlMessages":1,"trailingMessages":1,"frameEncodings":{"text":0,"binaryJson":31591,"binaryGzip":31},"initialSnapshots":1,"resnapshots":0,"disconnects":0,"disconnectDetails":[],"reconnects":0,"protocolErrors":[],"failures":[],"firstLiveUpdateMs":43.1,"drainMs":170622.6,"finalStateDigest":"sha256:ee4d907d69ef14696f948d44f2bcf8049ffec47b2b136803978448826e16056c","finalRows":3000,"mismatchedRows":0,"mismatchSample":[]}]}
```

</details>
