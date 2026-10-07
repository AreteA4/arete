use anyhow::Result;
use arete_sdk::{deep_merge_with_append, Frame, ServerMessage, SnapshotEntity};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use super::filter::{self, Filter};
use super::output::{self, OutputMode};
use super::session::{Notice, ReconnectPolicy, SessionHandler, StreamEnd, StreamSession};
use super::snapshot::{SnapshotPlayer, SnapshotRecorder};
use super::store::EntityStore;
use super::token;
use super::StreamArgs;

struct StreamState {
    entities: HashMap<String, serde_json::Value>,
    store: Option<EntityStore>,
    filter: Filter,
    select_fields: Option<Vec<Vec<String>>>,
    allowed_ops: Option<HashSet<String>>,
    output_mode: OutputMode,
    first: bool,
    count_only: bool,
    update_count: u64,
    entity_count: u64,
    last_count_render: Option<Instant>,
    recorder: Option<SnapshotRecorder>,
    pending_snapshot: Option<PendingSnapshot>,
    out: output::StdoutWriter,
}

struct PendingSnapshot {
    id: String,
    authoritative: bool,
    rows: Vec<SnapshotEntity>,
}

fn build_state(args: &StreamArgs, view: &str, url: &str) -> Result<StreamState> {
    let filter = Filter::parse(&args.filters)?;
    let select_fields = args.select.as_deref().map(filter::parse_select);
    let allowed_ops = args.ops.as_deref().map(|ops| {
        ops.split(',')
            .map(|s| {
                let s = s.trim().to_lowercase();
                // Normalize "create" → "upsert" to match op normalization at comparison time
                if s == "create" {
                    "upsert".to_string()
                } else {
                    s
                }
            })
            .collect::<HashSet<_>>()
    });

    let output_mode = if args.raw {
        OutputMode::Raw
    } else if args.no_dna {
        OutputMode::NoDna
    } else {
        OutputMode::Merged
    };

    let recorder = args.save.as_ref().map(|_| SnapshotRecorder::new(view, url));

    let use_store = args.history || args.at.is_some() || args.diff;
    if use_store && args.key.is_none() {
        eprintln!("Warning: --history/--at/--diff require --key; history will not be output.");
    }
    let store = if use_store {
        Some(EntityStore::new())
    } else {
        None
    };

    Ok(StreamState {
        entities: HashMap::new(),
        store,
        filter,
        select_fields,
        allowed_ops,
        output_mode,
        first: args.first,
        count_only: args.count,
        update_count: 0,
        entity_count: 0,
        last_count_render: None,
        recorder,
        pending_snapshot: None,
        out: output::StdoutWriter::new(),
    })
}

pub async fn stream(
    url: String,
    refresh: Option<token::SessionRefresh>,
    view: &str,
    args: &StreamArgs,
) -> Result<()> {
    let policy = super::reconnect_policy(args);
    stream_with_policy(url, refresh, view, args, policy).await
}

async fn stream_with_policy(
    url: String,
    refresh: Option<token::SessionRefresh>,
    view: &str,
    args: &StreamArgs,
    policy: ReconnectPolicy,
) -> Result<()> {
    // Validate args and build state before connecting (fails fast on bad --where regex etc.)
    let mut state = build_state(args, view, &url)?;

    let mut session =
        StreamSession::new(url, refresh, super::build_subscription(view, args), policy)?;
    let socket = session.connect().await?;

    eprintln!("Connected.");

    // Emit NoDna connected event only after successful WebSocket handshake
    if let OutputMode::NoDna = state.output_mode {
        output::emit_no_dna_event(
            &mut state.out,
            "connected",
            view,
            &serde_json::json!({"url": session.display_url()}),
            0,
            0,
        )?;
    }

    // Ctrl+C, or --duration (as a select! arm for precise timing). The
    // session polls this across reconnects, so --duration is wall time.
    let duration = args.duration;
    let stop = async move {
        let duration_elapsed = async {
            match duration {
                Some(secs) => tokio::time::sleep(Duration::from_secs(secs)).await,
                None => std::future::pending::<()>().await,
            }
        };
        tokio::select! {
            () = duration_elapsed => eprintln!("Duration reached, stopping..."),
            _ = tokio::signal::ctrl_c() => eprintln!("\nDisconnecting..."),
        }
    };
    tokio::pin!(stop);

    let mut handler = Output {
        state: &mut state,
        view,
        no_snapshot: args.no_snapshot,
        snapshot_complete: false,
    };
    let failure = match session.run(socket, &mut handler, stop).await? {
        StreamEnd::Stopped | StreamEnd::Finished => None,
        StreamEnd::Lost(loss) => {
            eprintln!("{loss}");
            None
        }
        StreamEnd::Failed(failure) => Some(failure),
    };

    // Save snapshot if --save was specified
    if let (Some(save_path), Some(recorder)) = (&args.save, &state.recorder) {
        recorder.save(save_path)?;
    }

    // Clear the overwriting count line before post-stream output
    if state.count_only {
        finalize_count(&mut state)?;
    }

    if let OutputMode::NoDna = state.output_mode {
        output::emit_no_dna_event(
            &mut state.out,
            "disconnected",
            view,
            &serde_json::json!(null),
            state.update_count,
            state.entity_count,
        )?;
    }

    // Output history/at/diff after stream ends (for non-interactive agent use)
    output_history_if_requested(&state, args)?;

    if let Some(failure) = failure {
        anyhow::bail!("{failure}");
    }
    Ok(())
}

/// Where a live stream's messages go: the merged/raw/NO_DNA output on stdout,
/// warnings on stderr.
struct Output<'a> {
    state: &'a mut StreamState,
    view: &'a str,
    no_snapshot: bool,
    snapshot_complete: bool,
}

impl SessionHandler for Output<'_> {
    fn on_message(&mut self, message: ServerMessage) -> Result<bool> {
        handle_server_message(
            message,
            self.view,
            self.state,
            &mut self.snapshot_complete,
            self.no_snapshot,
        )
    }

    fn on_notice(&mut self, notice: Notice<'_>) {
        let line = match notice {
            Notice::Unparsed { binary, error } => format!(
                "Warning: failed to parse {} frame: {}",
                if binary { "binary" } else { "text" },
                error
            ),
            Notice::RefreshRefused(reason) => format!(
                "Warning: the server refused the refreshed session token ({}); \
                 the stream ends when the current token expires.",
                reason.unwrap_or("no reason given")
            ),
            Notice::RefreshFailed(error) => {
                format!("Warning: could not refresh the session token, retrying: {error:#}")
            }
            Notice::Reconnecting(reconnecting) => reconnecting.to_string(),
        };
        self.print_notice(&line);
    }

    fn on_reconnected(&mut self, url: &str) -> Result<()> {
        // The new subscription starts over with its own snapshot. Entities
        // merged from the old connection, or a snapshot it left half
        // delivered, would otherwise mix stale state into the output.
        self.state.entities.clear();
        self.state.entity_count = 0;
        self.state.pending_snapshot = None;
        self.snapshot_complete = false;

        self.print_notice(&format!("Reconnected; resubscribed to {}", self.view));
        if let OutputMode::NoDna = self.state.output_mode {
            output::emit_no_dna_event(
                &mut self.state.out,
                "reconnected",
                self.view,
                &serde_json::json!({"url": url}),
                self.state.update_count,
                self.state.entity_count,
            )?;
        }
        Ok(())
    }
}

impl Output<'_> {
    /// Print a line to stderr, below the running count when --count is
    /// drawing one there.
    fn print_notice(&mut self, line: &str) {
        if self.state.count_only && self.state.last_count_render.take().is_some() {
            output::finalize_count();
        }
        eprintln!("{line}");
    }
}

/// Replay frames from a saved snapshot file through the same processing pipeline.
pub async fn replay(player: SnapshotPlayer, view: &str, args: &StreamArgs) -> Result<()> {
    let mut state = build_state(args, view, &player.header.url)?;

    // Emit NoDna connected event with replay source indicator
    if let OutputMode::NoDna = state.output_mode {
        output::emit_no_dna_event(
            &mut state.out,
            "connected",
            view,
            &serde_json::json!({"url": player.header.url, "source": "replay"}),
            0,
            0,
        )?;
    }

    let mut snapshot_complete = false;

    for snapshot_frame in &player.frames {
        if handle_server_message(
            ServerMessage::Frame(snapshot_frame.frame.clone()),
            view,
            &mut state,
            &mut snapshot_complete,
            args.no_snapshot,
        )? {
            break;
        }
    }

    if state.count_only {
        finalize_count(&mut state)?;
    }

    if let OutputMode::NoDna = state.output_mode {
        output::emit_no_dna_event(
            &mut state.out,
            "disconnected",
            view,
            &serde_json::json!(null),
            state.update_count,
            state.entity_count,
        )?;
    }

    output_history_if_requested(&state, args)?;

    eprintln!("Replay complete: {} updates processed.", state.update_count);
    Ok(())
}

/// After the stream ends, output --history / --at / --diff results for the specified --key.
fn output_history_if_requested(state: &StreamState, args: &StreamArgs) -> Result<()> {
    let store = match &state.store {
        Some(s) => s,
        None => return Ok(()),
    };

    let key = match &args.key {
        Some(k) => k.as_str(),
        None => {
            if args.history || args.at.is_some() || args.diff {
                eprintln!("Warning: --history/--at/--diff require --key to specify which entity");
            }
            return Ok(());
        }
    };

    if args.diff && args.history {
        eprintln!("Warning: --history is ignored when --diff is specified. Remove --diff to see full history.");
    }

    if args.diff {
        let index = args.at.unwrap_or(0);
        if let Some(diff) = store.diff_at(key, index) {
            let line = serde_json::to_string_pretty(&diff)?;
            println!("{}", line);
        } else {
            eprintln!("No history entry at index {} for key '{}'", index, key);
        }
    } else if let Some(index) = args.at {
        if let Some(entry) = store.at(key, index) {
            let output = serde_json::json!({
                "key": key,
                "index": index,
                "op": entry.op,
                "seq": entry.seq,
                "state": entry.state,
            });
            let line = serde_json::to_string_pretty(&output)?;
            println!("{}", line);
        } else {
            eprintln!("No history entry at index {} for key '{}'", index, key);
        }
    } else if args.history {
        if let Some(history) = store.history(key) {
            let line = serde_json::to_string_pretty(&history)?;
            println!("{}", line);
        } else {
            eprintln!("No history found for key '{}'", key);
        }
    }

    Ok(())
}

fn handle_server_message(
    message: ServerMessage,
    view: &str,
    state: &mut StreamState,
    snapshot_complete: &mut bool,
    no_snapshot: bool,
) -> Result<bool> {
    match message {
        ServerMessage::Error(error) => {
            eprintln!("Server error [{}]: {}", error.code, error.message);
            Ok(error.fatal)
        }
        ServerMessage::Frame(Frame::Subscribed { .. }) => {
            eprintln!("Subscribed to {}", view);
            Ok(false)
        }
        ServerMessage::Frame(Frame::Unsubscribed { .. }) => {
            eprintln!("Unsubscribed from {}", view);
            Ok(true)
        }
        ServerMessage::Frame(frame) => {
            let completes_snapshot = matches!(&frame, Frame::Snapshot { complete: true, .. });
            let first_live_without_snapshot = no_snapshot
                && !*snapshot_complete
                && matches!(
                    &frame,
                    Frame::Upsert { .. }
                        | Frame::Patch { .. }
                        | Frame::Remove { .. }
                        | Frame::Delete { .. }
                );
            let stop = process_frame(frame, view, state)?;
            if (completes_snapshot || first_live_without_snapshot) && !*snapshot_complete {
                *snapshot_complete = true;
                if let OutputMode::NoDna = state.output_mode {
                    output::emit_no_dna_event(
                        &mut state.out,
                        "snapshot_complete",
                        view,
                        &serde_json::json!({"entity_count": state.entity_count}),
                        state.update_count,
                        state.entity_count,
                    )?;
                }
            }
            Ok(stop)
        }
    }
}

/// Process a frame. Returns true if the stream should end (--first matched).
fn process_frame(frame: Frame, view: &str, state: &mut StreamState) -> Result<bool> {
    // Record frame if --save is active
    if let Some(recorder) = &mut state.recorder {
        recorder.record(&frame);
    }

    let op = match &frame {
        Frame::Snapshot { .. } => "snapshot",
        Frame::Upsert { .. } => "upsert",
        Frame::Patch { .. } => "patch",
        Frame::Remove { .. } => "remove",
        Frame::Delete { .. } => "delete",
        Frame::Subscribed { .. } | Frame::Unsubscribed { .. } => return Ok(false),
    };

    // Check if this op type is allowed by --ops (but always process snapshots
    // for entity state — just suppress their output)
    let ops_allowed = match &state.allowed_ops {
        Some(allowed) => allowed.contains(op),
        None => true,
    };

    if let OutputMode::Raw = state.output_mode {
        if !ops_allowed {
            return Ok(false);
        }
        // Note: in raw mode, --where filters against the raw frame.data which is
        // an array for snapshot frames. Field-level filters (e.g. --where "info.name=X")
        // will not match snapshot batch arrays — use merged mode for field filtering.
        let raw = serde_json::to_value(&frame)?;
        let data = raw.get("data").cloned().unwrap_or(serde_json::Value::Null);
        if !state.filter.is_empty() && !state.filter.matches(&data) {
            return Ok(false);
        }
        state.update_count += 1;
        if state.count_only {
            render_count_if_due(state)?;
        } else {
            output::print_raw_frame(&mut state.out, &frame)?;
        }
        return Ok(state.first);
    }

    match frame {
        Frame::Snapshot {
            snapshot_id,
            authoritative,
            data,
            complete,
            ..
        } => {
            let pending = state
                .pending_snapshot
                .get_or_insert_with(|| PendingSnapshot {
                    id: snapshot_id.clone(),
                    authoritative,
                    rows: Vec::new(),
                });
            if pending.id != snapshot_id || pending.authoritative != authoritative {
                anyhow::bail!("snapshot batches changed snapshotId or authoritative mode");
            }
            for row in data {
                if let Some(existing) = pending.rows.iter_mut().find(|item| item.key == row.key) {
                    *existing = row;
                } else {
                    pending.rows.push(row);
                }
            }
            if !complete {
                return Ok(false);
            }

            let snapshot = state
                .pending_snapshot
                .take()
                .expect("snapshot stage exists");
            if snapshot.authoritative {
                let retained: HashSet<&str> =
                    snapshot.rows.iter().map(|row| row.key.as_str()).collect();
                let removed: Vec<String> = state
                    .entities
                    .keys()
                    .filter(|key| !retained.contains(key.as_str()))
                    .cloned()
                    .collect();
                for key in removed {
                    state.entities.remove(&key);
                    if let Some(store) = &mut state.store {
                        store.remove(&key, "remove", None);
                    }
                }
            }

            for entity in snapshot.rows {
                // Always populate entity state (needed for correct patch merging).
                // entity_count is a running tally — NoDna entity_update events during
                // snapshot delivery report the count at that point, not the final total.
                // The final count is available in the snapshot_complete event.
                state
                    .entities
                    .insert(entity.key.clone(), entity.data.clone());
                state.entity_count = state.entities.len() as u64;
                if let Some(store) = &mut state.store {
                    store.upsert(&entity.key, entity.data.clone(), "snapshot", None);
                }
                // --first: exits on the first matching entity (even within a snapshot batch).
                // update_count will be 1 in the emitted event, which is correct.
                if ops_allowed && emit_entity(state, view, &entity.key, "snapshot", &entity.data)? {
                    return Ok(true);
                }
            }
            state.entity_count = state.entities.len() as u64;
        }
        Frame::Upsert { key, data, seq, .. } => {
            state.entities.insert(key.clone(), data.clone());
            if let Some(store) = &mut state.store {
                store.upsert(&key, data.clone(), "upsert", seq);
            }
            state.entity_count = state.entities.len() as u64;
            if ops_allowed && emit_entity(state, view, &key, "upsert", &data)? {
                return Ok(true);
            }
        }
        Frame::Patch {
            key,
            data,
            append,
            seq,
            ..
        } => {
            if let Some(store) = &mut state.store {
                store.patch(&key, &data, &append, seq);
            }
            let entry = state
                .entities
                .entry(key.clone())
                .or_insert_with(|| serde_json::json!({}));
            deep_merge_with_append(entry, &data, &append, "");
            let merged = entry.clone();
            state.entity_count = state.entities.len() as u64;
            if ops_allowed && emit_entity(state, view, &key, "patch", &merged)? {
                return Ok(true);
            }
        }
        Frame::Remove { key, seq, .. } => {
            return process_removal(key, seq, "remove", view, state, ops_allowed)
        }
        Frame::Delete { key, seq, .. } => {
            return process_removal(key, seq, "delete", view, state, ops_allowed)
        }
        Frame::Subscribed { .. } | Frame::Unsubscribed { .. } => {}
    }

    Ok(false)
}

fn process_removal(
    key: String,
    seq: Option<String>,
    op: &str,
    view: &str,
    state: &mut StreamState,
    ops_allowed: bool,
) -> Result<bool> {
    // If the entity was never seen (for example with --no-snapshot), field
    // filters cannot evaluate its previous state and suppress the event.
    let last_state = state
        .entities
        .remove(&key)
        .unwrap_or(serde_json::Value::Null);
    if let Some(store) = &mut state.store {
        store.remove(&key, op, seq);
    }
    state.entity_count = state.entities.len() as u64;

    if !ops_allowed || (!state.filter.is_empty() && !state.filter.matches(&last_state)) {
        return Ok(false);
    }

    state.update_count += 1;
    if state.count_only {
        render_count_if_due(state)?;
    } else {
        match state.output_mode {
            OutputMode::NoDna => output::emit_no_dna_event(
                &mut state.out,
                "entity_update",
                view,
                &serde_json::json!({"key": key, "op": op, "data": null}),
                state.update_count,
                state.entity_count,
            )?,
            _ => output::print_removal(&mut state.out, view, &key, op)?,
        }
    }
    Ok(state.first)
}

/// Emit an entity through filter + select + output. Returns true if --first should trigger.
fn emit_entity(
    state: &mut StreamState,
    view: &str,
    key: &str,
    op: &str,
    data: &serde_json::Value,
) -> Result<bool> {
    if !state.filter.is_empty() && !state.filter.matches(data) {
        return Ok(false);
    }

    state.update_count += 1;

    let output_data = match &state.select_fields {
        Some(fields) => filter::select_fields(data, fields),
        None => data.clone(),
    };

    if state.count_only {
        render_count_if_due(state)?;
    } else {
        match state.output_mode {
            OutputMode::NoDna => output::emit_no_dna_event(
                &mut state.out,
                "entity_update",
                view,
                &serde_json::json!({"key": key, "op": op, "data": output_data}),
                state.update_count,
                state.entity_count,
            )?,
            _ => output::print_entity_update(&mut state.out, view, key, op, &output_data)?,
        }
    }

    if state.first {
        return Ok(true);
    }

    Ok(false)
}

/// A terminal redraw performs a blocking write and flush. At high feed rates,
/// doing that once per frame can make `--count` itself the slow consumer, so
/// keep accounting exact while rendering at a human-visible cadence.
fn render_count_if_due(state: &mut StreamState) -> Result<()> {
    const COUNT_RENDER_INTERVAL: Duration = Duration::from_millis(100);
    let now = Instant::now();
    if state
        .last_count_render
        .is_none_or(|last| now.duration_since(last) >= COUNT_RENDER_INTERVAL)
    {
        output::print_count(state.update_count)?;
        state.last_count_render = Some(now);
    }
    Ok(())
}

fn finalize_count(state: &mut StreamState) -> Result<()> {
    output::print_count(state.update_count)?;
    output::finalize_count();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use arete_sdk::Mode;

    fn state() -> StreamState {
        StreamState {
            entities: HashMap::new(),
            store: None,
            filter: Filter {
                predicates: Vec::new(),
            },
            select_fields: None,
            allowed_ops: Some(HashSet::new()),
            output_mode: OutputMode::Merged,
            first: false,
            count_only: false,
            update_count: 0,
            entity_count: 0,
            last_count_render: None,
            recorder: None,
            pending_snapshot: None,
            out: output::StdoutWriter::new(),
        }
    }

    fn snapshot(id: &str, authoritative: bool, complete: bool, keys: &[&str]) -> Frame {
        Frame::Snapshot {
            protocol_version: 2,
            subscription_id: "cli:test".to_string(),
            snapshot_id: id.to_string(),
            authoritative,
            mode: Mode::List,
            entity: "Thing/list".to_string(),
            key: None,
            data: keys
                .iter()
                .map(|key| SnapshotEntity {
                    key: (*key).to_string(),
                    data: serde_json::json!({"id": key}),
                })
                .collect(),
            complete,
        }
    }

    #[test]
    fn stages_snapshot_batches_and_replaces_authoritative_membership() {
        let mut state = state();

        process_frame(
            snapshot("initial", true, false, &["1", "2"]),
            "Thing/list",
            &mut state,
        )
        .unwrap();
        assert!(state.entities.is_empty());

        process_frame(
            snapshot("initial", true, true, &["3"]),
            "Thing/list",
            &mut state,
        )
        .unwrap();
        assert_eq!(state.entities.len(), 3);

        process_frame(
            snapshot("replacement", true, true, &["3"]),
            "Thing/list",
            &mut state,
        )
        .unwrap();
        assert_eq!(
            state.entities.keys().cloned().collect::<Vec<_>>(),
            vec!["3".to_string()]
        );
    }

    #[test]
    fn remove_evicts_query_membership() {
        let mut state = state();
        process_frame(
            snapshot("initial", true, true, &["1"]),
            "Thing/list",
            &mut state,
        )
        .unwrap();

        process_frame(
            Frame::Remove {
                protocol_version: 2,
                subscription_id: "cli:test".to_string(),
                mode: Mode::List,
                entity: "Thing/list".to_string(),
                key: "1".to_string(),
                data: serde_json::Value::Null,
                seq: Some("2:1".to_string()),
                offset: None,
            },
            "Thing/list",
            &mut state,
        )
        .unwrap();
        assert!(state.entities.is_empty());
    }

    #[test]
    fn a_reconnect_drops_state_from_the_old_connection() {
        let mut state = state();
        process_frame(
            snapshot("initial", true, true, &["1", "2"]),
            "Thing/list",
            &mut state,
        )
        .unwrap();
        // The old connection drops halfway through another snapshot.
        process_frame(
            snapshot("recovery", true, false, &["3"]),
            "Thing/list",
            &mut state,
        )
        .unwrap();
        let mut output = Output {
            state: &mut state,
            view: "Thing/list",
            no_snapshot: false,
            snapshot_complete: true,
        };

        output.on_reconnected("ws://localhost/").unwrap();

        assert!(!output.snapshot_complete);
        assert!(state.entities.is_empty());
        assert_eq!(state.entity_count, 0);
        assert!(state.pending_snapshot.is_none());

        // The new connection's snapshot is staged on its own.
        process_frame(
            snapshot("fresh", false, true, &["4"]),
            "Thing/list",
            &mut state,
        )
        .unwrap();
        assert_eq!(
            state.entities.keys().cloned().collect::<Vec<_>>(),
            vec!["4".to_string()]
        );
    }

    /// A hosted stream against a local server standing in for the stack.
    mod over_a_socket {
        use super::*;
        use crate::api_client::test_support::MockServer;
        use futures_util::{SinkExt, StreamExt};
        use serde_json::{json, Value};
        use tokio::net::TcpListener;
        use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
        use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
        use tokio_tungstenite::tungstenite::protocol::CloseFrame;
        use tokio_tungstenite::tungstenite::Message;
        use tokio_tungstenite::WebSocketStream;

        use crate::commands::stream::session::tests::upsert;

        fn stream_args(view: &str) -> StreamArgs {
            stream_args_from(&[view])
        }

        fn stream_args_from(args: &[&str]) -> StreamArgs {
            #[derive(clap::Parser)]
            struct Cli {
                #[command(flatten)]
                args: StreamArgs,
            }
            <Cli as clap::Parser>::try_parse_from(std::iter::once("a4").chain(args.iter().copied()))
                .expect("stream args parse")
                .args
        }

        /// The policy `args` ask for, with waits short enough for a test.
        fn quick(args: &StreamArgs) -> ReconnectPolicy {
            ReconnectPolicy {
                initial_delay: Duration::from_millis(10),
                max_delay: Duration::from_millis(50),
                ..crate::commands::stream::reconnect_policy(args)
            }
        }

        type ServerSocket = WebSocketStream<tokio::net::TcpStream>;

        /// Accept the next connection, returning it with the path and query
        /// it was opened with.
        // The handshake callback's error type is tungstenite's, not ours.
        #[allow(clippy::result_large_err)]
        async fn accept(listener: &TcpListener) -> (ServerSocket, String) {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut target = String::new();
            let socket = tokio_tungstenite::accept_hdr_async(
                tcp,
                |request: &Request, response: Response| {
                    target = request.uri().to_string();
                    Ok(response)
                },
            )
            .await
            .unwrap();
            (socket, target)
        }

        /// The next text message the client sends, as JSON.
        async fn next_json(socket: &mut ServerSocket) -> Value {
            loop {
                match socket.next().await {
                    Some(Ok(Message::Text(text))) => return serde_json::from_str(&text).unwrap(),
                    Some(Ok(_)) => continue,
                    other => panic!("the client went away: {other:?}"),
                }
            }
        }

        fn unix_now() -> u64 {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn a_hosted_stream_refreshes_its_token_on_the_open_socket() {
            let mint = MockServer::json(
                200,
                &json!({"token": "second", "expires_at": unix_now() + 3_600}).to_string(),
            );
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("ws://{}/", listener.local_addr().unwrap());
            let server = tokio::spawn(async move {
                let (tcp, _) = listener.accept().await.unwrap();
                let mut socket = tokio_tungstenite::accept_async(tcp).await.unwrap();
                let mut received = Vec::new();
                while let Some(Ok(message)) = socket.next().await {
                    let Message::Text(text) = message else {
                        continue;
                    };
                    let message: Value = serde_json::from_str(&text).unwrap();
                    let refreshed = message["type"] == "refresh_auth";
                    received.push(message);
                    if refreshed {
                        break;
                    }
                }
                socket
                    .send(Message::Text(
                        json!({"success": true, "expiresAt": unix_now() + 3_600}).to_string(),
                    ))
                    .await
                    .unwrap();
                // Then end the session the way an expired token would.
                socket
                    .close(Some(CloseFrame {
                        code: CloseCode::Policy,
                        reason: "token-expired: Authentication token expired".into(),
                    }))
                    .await
                    .unwrap();
                received
            });

            let refresh = token::SessionRefresh::for_test(
                format!("{}/ws/sessions", mint.base_url()),
                &url,
                unix_now() + 2,
            );
            let args = stream_args("Ore/list");
            let result = tokio::time::timeout(
                Duration::from_secs(20),
                stream(url.clone(), Some(refresh), "Ore/list", &args),
            )
            .await
            .expect("the stream ends within the timeout");

            let error = result.expect_err("a policy close fails the stream");
            assert!(
                format!("{error:#}").contains("token-expired"),
                "the reason reaches the user: {error:#}"
            );
            let received = server.await.unwrap();
            assert_eq!(received[0]["type"], "subscribe");
            assert_eq!(received.last().unwrap()["token"], "second");
            assert_eq!(mint.request().request_line, "POST /ws/sessions HTTP/1.1");
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn without_reconnect_a_plain_close_ends_the_stream_cleanly() {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("ws://{}/", listener.local_addr().unwrap());
            let server = tokio::spawn(async move {
                let (tcp, _) = listener.accept().await.unwrap();
                let mut socket = tokio_tungstenite::accept_async(tcp).await.unwrap();
                let _subscribe = socket.next().await;
                socket.close(None).await.unwrap();
            });

            let args = stream_args_from(&["Ore/list", "--no-reconnect"]);
            let result = tokio::time::timeout(
                Duration::from_secs(10),
                stream(url, None, "Ore/list", &args),
            )
            .await
            .expect("the stream ends within the timeout");

            assert!(result.is_ok(), "{result:?}");
            server.await.unwrap();
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn a_dropped_connection_reconnects_and_resubscribes() {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("ws://{}/", listener.local_addr().unwrap());
            let server = tokio::spawn(async move {
                let (mut socket, _) = accept(&listener).await;
                let first = next_json(&mut socket).await;
                let id = first["subscriptionId"].as_str().unwrap().to_string();
                socket.send(upsert(&id, "1")).await.unwrap();
                // Go away the way a crashed server does: no close frame.
                drop(socket);

                let (mut socket, _) = accept(&listener).await;
                let second = next_json(&mut socket).await;
                socket.send(upsert(&id, "2")).await.unwrap();
                // Wait for the client to leave.
                while let Some(Ok(_)) = socket.next().await {}
                (first, second)
            });

            // Only the entity sent after the reconnect ends the stream.
            let args = stream_args_from(&["Ore/list", "--first", "--where", "id=2", "--take", "5"]);
            let result = tokio::time::timeout(
                Duration::from_secs(10),
                stream_with_policy(url, None, "Ore/list", &args, quick(&args)),
            )
            .await
            .expect("the stream ends within the timeout");

            assert!(result.is_ok(), "{result:?}");
            let (first, second) = server.await.unwrap();
            assert_eq!(first["type"], "subscribe");
            assert_eq!(first["query"]["take"], 5);
            assert_eq!(second, first, "the same subscription is sent again");
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn a_hosted_stream_mints_a_new_token_to_reconnect() {
            let mint = MockServer::json(
                200,
                &json!({"token": "second", "expires_at": unix_now() + 3_600}).to_string(),
            );
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base_url = format!("ws://{}/", listener.local_addr().unwrap());
            let server = tokio::spawn(async move {
                let (mut socket, first_target) = accept(&listener).await;
                let first = next_json(&mut socket).await;
                let id = first["subscriptionId"].as_str().unwrap().to_string();
                drop(socket);

                let (mut socket, second_target) = accept(&listener).await;
                let _second = next_json(&mut socket).await;
                socket.send(upsert(&id, "1")).await.unwrap();
                while let Some(Ok(_)) = socket.next().await {}
                (first_target, second_target)
            });

            // The first token is far from expiry, so only the reconnect mints.
            let refresh = token::SessionRefresh::for_test(
                format!("{}/ws/sessions", mint.base_url()),
                &base_url,
                unix_now() + 3_600,
            );
            let args = stream_args_from(&["Ore/list", "--first"]);
            let result = tokio::time::timeout(
                Duration::from_secs(10),
                stream_with_policy(
                    format!("{base_url}?hs_token=first"),
                    Some(refresh),
                    "Ore/list",
                    &args,
                    quick(&args),
                ),
            )
            .await
            .expect("the stream ends within the timeout");

            assert!(result.is_ok(), "{result:?}");
            let (first_target, second_target) = server.await.unwrap();
            assert_eq!(first_target, "/?hs_token=first");
            assert_eq!(second_target, "/?hs_token=second");
            assert_eq!(
                serde_json::from_str::<Value>(&mint.request().body).unwrap(),
                json!({"websocket_url": base_url})
            );
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn a_policy_close_is_not_retried() {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("ws://{}/", listener.local_addr().unwrap());
            let server = tokio::spawn(async move {
                let (mut socket, _) = accept(&listener).await;
                let _subscribe = next_json(&mut socket).await;
                socket
                    .close(Some(CloseFrame {
                        code: CloseCode::Policy,
                        reason: "token-expired: Authentication token expired".into(),
                    }))
                    .await
                    .unwrap();
                // Any reconnect would arrive well within this.
                tokio::time::timeout(Duration::from_millis(500), listener.accept())
                    .await
                    .is_ok()
            });

            let args = stream_args("Ore/list");
            let result = tokio::time::timeout(
                Duration::from_secs(10),
                stream_with_policy(url, None, "Ore/list", &args, quick(&args)),
            )
            .await
            .expect("the stream ends within the timeout");

            let error = result.expect_err("a policy close fails the stream");
            assert!(format!("{error:#}").contains("token-expired"), "{error:#}");
            assert!(!server.await.unwrap(), "the client did not reconnect");
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn the_stream_gives_up_after_max_reconnects() {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("ws://{}/", listener.local_addr().unwrap());
            let server = tokio::spawn(async move {
                let (mut socket, _) = accept(&listener).await;
                let _subscribe = next_json(&mut socket).await;
                // The server goes away for good: every reconnect is refused.
                drop(socket);
                drop(listener);
            });

            let args = stream_args_from(&["Ore/list", "--max-reconnects", "2"]);
            let result = tokio::time::timeout(
                Duration::from_secs(10),
                stream_with_policy(url, None, "Ore/list", &args, quick(&args)),
            )
            .await
            .expect("the stream ends within the timeout");

            let error = result.expect_err("giving up fails the stream");
            let message = format!("{error:#}");
            assert!(
                message.starts_with("gave up after 2 reconnect attempts: could not connect: "),
                "{message}"
            );
            assert_eq!(
                message.matches("refused").count(),
                1,
                "the cause is named once: {message}"
            );
            server.await.unwrap();
        }
    }
}
