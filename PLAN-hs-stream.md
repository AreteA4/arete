# Plan: `hs stream` — Live WebSocket Stream CLI + TUI

## Context

Users can deploy stacks with `hs up` but have no CLI way to observe live stream data without writing code. This is the highest-leverage missing UX feature. The goal is a single `hs stream` command that:

1. Connects to a deployed stack's WebSocket and streams entity data to stdout (pipe-friendly NDJSON)
2. Supports rich filtering, `--first` triggers, raw vs merged output, NO_DNA agent format
3. Provides an interactive TUI for exploring entities, time-traveling through updates
4. Ensures every TUI action has a non-interactive CLI equivalent for agent consumption
5. Supports saving/loading snapshot recordings

## Key Existing Infrastructure

| Component | Location | Reuse |
|-----------|----------|-------|
| CLI entry point | [main.rs](cli/src/main.rs) | Add `Stream` variant to `Commands` enum |
| Config with WS URLs | [config.rs](cli/src/config.rs) `StackConfig.url` | Resolve WebSocket URL from `hyperstack.toml` |
| SDK Frame types | [frame.rs](rust/hyperstack-sdk/src/frame.rs) | `Frame`, `parse_frame`, `parse_snapshot_entities`, gzip handling |
| SDK Subscription | [subscription.rs](rust/hyperstack-sdk/src/subscription.rs) | `ClientMessage`, `Subscription` types |
| SDK Connection | [connection.rs](rust/hyperstack-sdk/src/connection.rs) | Pattern reference for WS connect + reconnect loop |
| SDK Store + merge | [store.rs](rust/hyperstack-sdk/src/store.rs) | `deep_merge_with_append` (currently private — needs `pub`) |
| SDK SharedStore | [store.rs](rust/hyperstack-sdk/src/store.rs) | `StoreUpdate` type with `previous`/`patch` fields |

## Command Design

```
hs stream <VIEW> [OPTIONS]

ARGS:
  <VIEW>                 Entity/view to subscribe: EntityName/mode (e.g. OreRound/latest)

CONNECTION:
  --url <URL>            WebSocket URL override
  --stack <NAME>         Stack name (resolves URL from hyperstack.toml)
  --key <KEY>            Entity key for state-mode subscriptions

OUTPUT MODE (mutually exclusive):
  --raw                  Raw WebSocket frames (no merge)
  --no-dna               NO_DNA agent-friendly envelope format
  [default]              Merged entity NDJSON

FILTERING:
  --where <EXPR>         Filter: field=value, field>N, field~regex (repeatable, ANDed)
  --select <FIELDS>      Project specific fields (comma-separated dot paths)
  --first                Exit after first entity matches filter
  --ops <OPS>            Filter by operation type: upsert,patch,delete

SUBSCRIPTION:
  --take <N>             Max entities in snapshot
  --skip <N>             Skip N entities
  --no-snapshot          Skip initial snapshot
  --after <SEQ>          Resume from cursor (seq value)

RECORDING:
  --save <FILE>          Record frames to JSON file
  --duration <SECS>      Auto-stop recording after N seconds
  --load <FILE>          Replay a saved recording (no WS connection)

TUI:
  --tui                  Interactive terminal UI

HISTORY (non-interactive agent equivalents):
  --history              Show update history for --key entity
  --at <N>               Show entity at history index (0=latest)
  --diff                 Show diff between consecutive updates
  --count                Show running count of updates only
```

### URL Resolution Priority
1. `--url wss://...` — explicit
2. `--stack my-stack` — lookup `StackConfig.url` via `config.find_stack()`
3. Auto-match entity name from view to a stack in config
4. Error with list of available stacks

### NO_DNA Format (`--no-dna`)
When `NO_DNA` env var is set OR `--no-dna` flag is used:
- Each line is a JSON envelope: `{"schema":"no-dna/v1", "tool":"hs-stream", "action":"entity_update"|"connected"|"snapshot_complete"|"error", "data":{...}, "meta":{update_count, entities_tracked, connected}}`
- No spinners, no color, no interactive prompts
- Lifecycle events (connected, snapshot_complete, disconnected) emitted as structured events

### Filter DSL (`--where`)
```
field=value        exact match (string or number auto-coercion)
field!=value       not equal
field>N            greater than (numeric)
field>=N           greater or equal
field<N            less than
field<=N           less or equal
field~regex        regex match
field!~regex       regex not match
field.nested=x     dot-path into nested JSON
field?             field exists (not null)
field!?            field is null/missing
```

## Architecture

### Async Boundary
The CLI is synchronous today. The `stream` command creates a one-shot tokio runtime:
```rust
tokio::runtime::Runtime::new()?.block_on(stream_main(args))
```
This isolates async to the stream command only.

### Two Execution Paths

**Lightweight path** (default, `--raw`, `--no-dna`):
```
connect_async(url) → subscribe → frame loop → [filter] → [format] → stdout
```
- No store, no history tracking, minimal memory
- Each frame is processed and immediately printed or discarded
- For merged mode: maintains a simple `HashMap<String, Value>` for patch merging

**Store path** (`--tui`, `--history`, `--save`):
```
connect_async(url) → subscribe → frame loop → EntityStore → [filter] → output/TUI
```
- `EntityStore` tracks full entity state + history ring buffer per entity
- History capped at configurable max (default 1000 entries per entity)

### EntityStore (new, in CLI)
```rust
struct EntityStore {
    entities: HashMap<String, EntityRecord>,
    max_history: usize,
}
struct EntityRecord {
    current: Value,
    history: VecDeque<HistoryEntry>,  // ring buffer
}
struct HistoryEntry {
    timestamp: DateTime<Utc>,
    seq: Option<String>,
    op: Operation,
    state: Value,        // full entity after this update
    patch: Option<Value>, // raw patch for patch ops
}
```

### TUI Architecture
Uses `ratatui` + `crossterm` (behind `tui` feature flag).

**Layout:**
```
┌──────────────────────────────────────────────────────┐
│ hs stream OreRound/latest              [connected]   │
├─────────────────┬────────────────────────────────────┤
│ Entities        │ Entity Detail                      │
│                 │                                    │
│ > round_42      │ {                                  │
│   round_43      │   "roundId": 42,                   │
│   round_44      │   "rewards": "1.5 SOL",            │
│                 │   ...                              │
│                 │ }                                  │
├─────────────────┴────────────────────────────────────┤
│ History: [|<] [<] update 3/7 [>] [>|]               │
├──────────────────────────────────────────────────────┤
│ Filter: --where roundId>40   Updates: 127            │
└──────────────────────────────────────────────────────┘
```

**Key bindings:**
- `j/k`/arrows: navigate entity list
- `Enter`: focus detail (full width), `Esc`: back
- `h/l`/left/right: step through history (time travel)
- `Home/End`: oldest/newest history
- `d`: toggle diff view
- `/`: type filter expression
- `r`: toggle raw/merged
- `s`: save snapshot
- `p`: pause/resume
- `q`: quit

**Event loop:** `tokio::select!` over crossterm events (16ms tick) + WS frame channel (`mpsc`).

### TUI ↔ Agent Equivalence Table

| TUI Action | Agent CLI |
|---|---|
| Browse entity list | `hs stream View/list` (prints all entities) |
| Select entity by key | `hs stream View/mode --key <key>` |
| View detail | Default merged output |
| Time travel to step N | `--history --at N --key <key>` |
| Show diff | `--diff --key <key>` |
| Filter | `--where "field=value"` |
| Raw frames | `--raw` |
| Save dataset | `--save file.json --duration 30` |
| Load replay | `--load file.json` |
| Count updates | `--count` |
| First match | `--first --where "field=value"` |

### Snapshot File Format
```json
{
  "version": 1,
  "view": "OreRound/latest",
  "url": "wss://...",
  "captured_at": "2026-03-23T10:00:00Z",
  "duration_ms": 30000,
  "frame_count": 147,
  "frames": [
    {"ts": 1711180800000, "frame": {"mode": "list", "entity": "OreRound/latest", "op": "upsert", "...": "..."}},
    "..."
  ]
}
```
- `--load file.json` replays through the same merge/filter/output pipeline
- `--load file.json --tui` enables TUI replay with time travel

## File Structure

```
cli/src/
  main.rs                        # Add Stream to Commands enum
  commands/
    mod.rs                       # Add pub mod stream
    stream/
      mod.rs                     # Entry point, URL resolution, tokio runtime, dispatch
      client.rs                  # WebSocket connect, subscribe, frame loop (reuses SDK types)
      store.rs                   # EntityStore with history + deep_merge_with_append
      filter.rs                  # --where DSL parser and evaluator
      output.rs                  # Formatters: ndjson, no_dna, raw
      snapshot.rs                # --save/--load file I/O and replay
      tui/
        mod.rs                   # TuiApp state + main event loop
        ui.rs                    # ratatui layout rendering
        widgets.rs               # Entity list, JSON viewer, timeline bar
```

## Dependencies to Add (cli/Cargo.toml)

```toml
# Async (only used by stream command)
tokio = { version = "1.0", features = ["rt-multi-thread", "sync", "time", "macros", "signal"] }
futures-util = { version = "0.3", features = ["sink"] }
tokio-tungstenite = { version = "0.21", default-features = false, features = ["connect", "rustls-tls-webpki-roots"] }

# Reuse SDK frame/subscription types
hyperstack-sdk = { path = "../rust/hyperstack-sdk", version = "0.5.10" }

# TUI (behind feature flag)
ratatui = { version = "0.29", optional = true }
crossterm = { version = "0.28", optional = true }

[features]
tui = ["ratatui", "crossterm"]
```

### SDK Change Required
Make `deep_merge_with_append` public in [store.rs](rust/hyperstack-sdk/src/store.rs:119):
```rust
pub fn deep_merge_with_append(...)  // was fn (private)
```

## Implementation Phases

### Phase 1: Core Streaming (MVP)
1. Add `Stream` variant to `Commands` enum in `main.rs`
2. Create `commands/stream/mod.rs` — arg parsing, URL resolution, tokio runtime entry
3. Implement `client.rs` — direct WS connection using `tokio-tungstenite`, subscribe, frame receive loop
4. Implement `output.rs` — NDJSON line output to stdout
5. Wire `--raw` mode (frame → JSON line → stdout)
6. Wire merged mode with inline `HashMap<String, Value>` + `deep_merge_with_append`
7. Make `deep_merge_with_append` pub in SDK

### Phase 2: Filtering + Flags
8. Implement `filter.rs` — parse `--where` expressions, evaluate against `serde_json::Value`
9. Wire `--first` (disconnect + exit 0 after first filter match)
10. Wire `--select` (project fields from output)
11. Wire `--ops` (filter by operation type)
12. Implement `--no-dna` output envelope format
13. Wire `--count` mode

### Phase 3: Recording
14. Implement `snapshot.rs` — `--save` appends timestamped frames to buffer, writes on Ctrl+C or `--duration`
15. Implement `--load` replay through same pipeline

### Phase 4: History + Store
16. Implement `store.rs` — `EntityStore` with history ring buffer
17. Wire `--history`, `--at`, `--diff` flags for non-interactive history access

### Phase 5: TUI
18. Add `ratatui`/`crossterm` behind `tui` feature flag
19. Implement `tui/mod.rs` — `TuiApp` state, `select!` event loop
20. Implement `tui/ui.rs` — three-panel layout
21. Implement `tui/widgets.rs` — entity list, JSON viewer with syntax highlighting, timeline bar
22. Wire entity navigation, detail view, history stepping, diff view, filter input, pause/resume, save

## Verification

### Unit Tests
- `filter.rs`: All operators, nested paths, type coercion, null handling
- `store.rs`: Merge behavior, history ring buffer, eviction
- `output.rs`: NDJSON format, NO_DNA envelope, field projection
- `snapshot.rs`: Save/load round-trip, replay ordering

### Integration Tests
- Mock WS server (tokio-tungstenite server in test) sends scripted frame sequence
- Verify `--raw` outputs valid NDJSON
- Verify merged mode correctly patches entities
- Verify `--where` excludes non-matching
- Verify `--first` exits after match
- Verify `--save`/`--load` round-trip

### Manual E2E
- `hs stream OreRound/latest --stack ore` against a live deployment
- Pipe to `jq '.rewards'` — verify valid JSON per line
- `hs stream OreRound/latest --raw | head -5` — verify `head` causes clean exit
- `hs stream OreRound/latest --tui` — interactive exploration
- `hs stream OreRound/latest --save test.json --duration 10 && hs stream --load test.json --tui`
