use anyhow::{Context, Result};
use arete_sdk::{
    deep_merge_with_append, parse_server_message, ClientMessage, Frame, ServerMessage,
    SnapshotEntity,
};
use futures_util::{SinkExt, StreamExt};
use std::collections::{HashMap, HashSet};
use tokio_tungstenite::{connect_async, tungstenite::Message};

use super::filter::{self, Filter};
use super::output::{self, OutputMode};
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
        recorder,
        pending_snapshot: None,
        out: output::StdoutWriter::new(),
    })
}

pub async fn stream(url: String, view: &str, args: &StreamArgs) -> Result<()> {
    // Validate args and build state before connecting (fails fast on bad --where regex etc.)
    let mut state = build_state(args, view, &url)?;

    let (ws, _) = connect_async(&url).await.map_err(|err| {
        let redacted = token::redact_hs_token_for_display(&url);
        let hint = if token::is_hosted_arete_cloud_url(&url) {
            "\nHint: hosted stacks need a valid `hs_token` (the CLI adds one after `a4 auth login`). \
             On some systems, TLS uses the OS trust store — if this persists, report the error above."
        } else {
            ""
        };
        anyhow::anyhow!("Failed to connect to {}: {}{}", redacted, err, hint)
    })?;

    eprintln!("Connected.");

    // Emit NoDna connected event only after successful WebSocket handshake
    if let OutputMode::NoDna = state.output_mode {
        output::emit_no_dna_event(
            &mut state.out,
            "connected",
            view,
            &serde_json::json!({"url": token::redact_hs_token_for_display(&url)}),
            0,
            0,
        )?;
    }

    let (mut ws_tx, mut ws_rx) = ws.split();

    // Build and send subscription
    let sub = super::build_subscription(view, args);
    let msg = serde_json::to_string(&ClientMessage::Subscribe(sub))
        .context("Failed to serialize subscribe message")?;
    ws_tx
        .send(Message::Text(msg))
        .await
        .context("Failed to send subscribe message")?;

    // Ping interval
    let ping_period = std::time::Duration::from_secs(30);
    let mut ping_interval =
        tokio::time::interval_at(tokio::time::Instant::now() + ping_period, ping_period);

    // Duration timer for --save --duration (as a select! arm for precise timing)
    let duration_future = async {
        if let Some(secs) = args.duration {
            tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
        } else {
            std::future::pending::<()>().await;
        }
    };
    tokio::pin!(duration_future);

    // Handle Ctrl+C
    let shutdown = tokio::signal::ctrl_c();
    tokio::pin!(shutdown);

    let mut snapshot_complete = false;

    loop {
        tokio::select! {
            msg = ws_rx.next() => {
                match msg {
                    Some(Ok(Message::Binary(bytes))) => {
                        match parse_server_message(&bytes) {
                            Ok(message) => {
                                if handle_server_message(
                                    message,
                                    view,
                                    &mut state,
                                    &mut snapshot_complete,
                                    args.no_snapshot,
                                )? {
                                    break;
                                }
                            }
                            Err(e) => eprintln!("Warning: failed to parse binary frame: {}", e),
                        }
                    }
                    Some(Ok(Message::Text(text))) => {
                        match parse_server_message(text.as_bytes()) {
                            Ok(message) => {
                                if handle_server_message(
                                    message,
                                    view,
                                    &mut state,
                                    &mut snapshot_complete,
                                    args.no_snapshot,
                                )? {
                                    break;
                                }
                            }
                            Err(e) => eprintln!("Warning: failed to parse text frame: {}", e),
                        }
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        let _ = ws_tx.send(Message::Pong(payload)).await;
                    }
                    Some(Ok(Message::Close(_))) => {
                        eprintln!("Connection closed by server.");
                        break;
                    }
                    Some(Err(e)) => {
                        eprintln!("WebSocket error: {}", e);
                        break;
                    }
                    None => {
                        eprintln!("Connection closed.");
                        break;
                    }
                    _ => {}
                }
            }
            _ = ping_interval.tick() => {
                if let Ok(msg) = serde_json::to_string(&ClientMessage::Ping) {
                    let _ = ws_tx.send(Message::Text(msg)).await;
                }
            }
            _ = &mut duration_future => {
                eprintln!("Duration reached, stopping...");
                let _ = ws_tx.close().await;
                break;
            }
            _ = &mut shutdown => {
                eprintln!("\nDisconnecting...");
                let _ = ws_tx.close().await;
                break;
            }
        }
    }

    // Save snapshot if --save was specified
    if let (Some(save_path), Some(recorder)) = (&args.save, &state.recorder) {
        recorder.save(save_path)?;
    }

    // Clear the overwriting count line before post-stream output
    if state.count_only {
        output::finalize_count();
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

    Ok(())
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
        output::finalize_count();
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
            output::print_count(state.update_count)?;
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
        output::print_count(state.update_count)?;
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
        output::print_count(state.update_count)?;
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
            },
            "Thing/list",
            &mut state,
        )
        .unwrap();
        assert!(state.entities.is_empty());
    }
}
