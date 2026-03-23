use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use hyperstack_sdk::{
    deep_merge_with_append, parse_frame, parse_snapshot_entities, try_parse_subscribed_frame,
    ClientMessage, Frame, Operation, Subscription,
};
use std::collections::{HashMap, HashSet};
use tokio_tungstenite::{connect_async, tungstenite::Message};

use super::filter::{self, Filter};
use super::output::{self, OutputMode};
use super::snapshot::{SnapshotPlayer, SnapshotRecorder};
use super::store::EntityStore;
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
}

fn build_state(args: &StreamArgs, view: &str, url: &str) -> Result<StreamState> {
    let filter = Filter::parse(&args.filters)?;
    let select_fields = args.select.as_deref().map(filter::parse_select);
    let allowed_ops = args.ops.as_deref().map(|ops| {
        ops.split(',')
            .map(|s| s.trim().to_lowercase())
            .collect::<HashSet<_>>()
    });

    let output_mode = if args.raw {
        OutputMode::Raw
    } else if args.no_dna {
        output::emit_no_dna_event("connected", view, &serde_json::json!({"url": url}), 0, 0)?;
        OutputMode::NoDna
    } else {
        OutputMode::Merged
    };

    let recorder = args.save.as_ref().map(|_| SnapshotRecorder::new(view, url));

    let use_store = args.history || args.at.is_some() || args.diff;
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
    })
}

pub async fn stream(url: String, view: &str, args: &StreamArgs) -> Result<()> {
    let (ws, _) = connect_async(&url)
        .await
        .with_context(|| format!("Failed to connect to {}", url))?;

    eprintln!("Connected.");

    let (mut ws_tx, mut ws_rx) = ws.split();

    // Build subscription
    let mut sub = Subscription::new(view);
    if let Some(key) = &args.key {
        sub = sub.with_key(key.clone());
    }
    if let Some(take) = args.take {
        sub = sub.with_take(take);
    }
    if let Some(skip) = args.skip {
        sub = sub.with_skip(skip);
    }
    if args.no_snapshot {
        sub = sub.with_snapshot(false);
    }
    if let Some(after) = &args.after {
        sub = sub.after(after.clone());
    }

    // Send subscribe message
    let msg = serde_json::to_string(&ClientMessage::Subscribe(sub))
        .context("Failed to serialize subscribe message")?;
    ws_tx
        .send(Message::Text(msg))
        .await
        .context("Failed to send subscribe message")?;

    let mut state = build_state(args, view, &url)?;

    // Ping interval
    let ping_period = std::time::Duration::from_secs(30);
    let mut ping_interval = tokio::time::interval_at(tokio::time::Instant::now() + ping_period, ping_period);

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
    // When --no-snapshot, treat as if snapshot was already received so
    // snapshot_complete fires on the first live frame
    let mut received_snapshot = args.no_snapshot;

    loop {
        tokio::select! {
            msg = ws_rx.next() => {
                match msg {
                    Some(Ok(Message::Binary(bytes))) => {
                        match parse_frame(&bytes) {
                            Ok(frame) => {
                                if frame.operation() == Operation::Subscribed {
                                    eprintln!("Subscribed to {}", view);
                                    continue;
                                }
                                let was_snapshot = frame.is_snapshot();
                                if was_snapshot { received_snapshot = true; }
                                if process_frame(frame, view, &mut state)? {
                                    break;
                                }
                                if !was_snapshot && received_snapshot && !snapshot_complete {
                                    snapshot_complete = true;
                                    if let OutputMode::NoDna = state.output_mode {
                                        output::emit_no_dna_event(
                                            "snapshot_complete", view,
                                            &serde_json::json!({"entity_count": state.entity_count}),
                                            state.update_count, state.entity_count,
                                        )?;
                                    }
                                }
                            }
                            Err(e) => {
                                if try_parse_subscribed_frame(&bytes).is_some() {
                                    eprintln!("Subscribed to {}", view);
                                } else {
                                    eprintln!("Warning: failed to parse binary frame: {}", e);
                                }
                            }
                        }
                    }
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
                            if value.get("op").and_then(|v| v.as_str()) == Some("subscribed") {
                                eprintln!("Subscribed to {}", view);
                                continue;
                            }
                        }
                        match serde_json::from_str::<Frame>(&text) {
                            Ok(frame) => {
                                let was_snapshot = frame.is_snapshot();
                                if was_snapshot { received_snapshot = true; }
                                if process_frame(frame, view, &mut state)? {
                                    break;
                                }
                                if !was_snapshot && received_snapshot && !snapshot_complete {
                                    snapshot_complete = true;
                                    if let OutputMode::NoDna = state.output_mode {
                                        output::emit_no_dna_event(
                                            "snapshot_complete", view,
                                            &serde_json::json!({"entity_count": state.entity_count}),
                                            state.update_count, state.entity_count,
                                        )?;
                                    }
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

    if let OutputMode::NoDna = state.output_mode {
        output::emit_no_dna_event(
            "disconnected", view,
            &serde_json::json!(null),
            state.update_count, state.entity_count,
        )?;
    }

    // Output history/at/diff after stream ends (for non-interactive agent use)
    output_history_if_requested(&state, args)?;

    Ok(())
}

/// Replay frames from a saved snapshot file through the same processing pipeline.
pub async fn replay(player: SnapshotPlayer, view: &str, args: &StreamArgs) -> Result<()> {
    let mut state = build_state(args, view, &player.header.url)?;

    for snapshot_frame in &player.frames {
        if process_frame(snapshot_frame.frame.clone(), view, &mut state)? {
            break;
        }
    }

    if let OutputMode::NoDna = state.output_mode {
        output::emit_no_dna_event(
            "disconnected", view,
            &serde_json::json!(null),
            state.update_count, state.entity_count,
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

/// Process a frame. Returns true if the stream should end (--first matched).
fn process_frame(
    frame: Frame,
    view: &str,
    state: &mut StreamState,
) -> Result<bool> {
    // Record frame if --save is active
    if let Some(recorder) = &mut state.recorder {
        recorder.record(&frame);
    }

    let op = frame.operation();
    let op_str = &frame.op;

    // Filter by operation type
    if let Some(allowed) = &state.allowed_ops {
        if op != Operation::Snapshot && !allowed.contains(op_str.to_lowercase().as_str()) {
            return Ok(false);
        }
    }

    if let OutputMode::Raw = state.output_mode {
        if !state.filter.is_empty() && !state.filter.matches(&frame.data) {
            return Ok(false);
        }
        state.update_count += 1;
        if state.count_only {
            output::print_count(state.update_count)?;
        } else {
            output::print_raw_frame(&frame)?;
        }
        return Ok(state.first);
    }

    match op {
        Operation::Snapshot => {
            let snapshot_entities = parse_snapshot_entities(&frame.data);
            for entity in snapshot_entities {
                state.entities.insert(entity.key.clone(), entity.data.clone());
                if let Some(store) = &mut state.store {
                    store.upsert(&entity.key, entity.data.clone(), "snapshot", None);
                }
                state.entity_count = state.entities.len() as u64;
                if emit_entity(state, view, &entity.key, "snapshot", &entity.data)? {
                    return Ok(true);
                }
            }
        }
        Operation::Upsert | Operation::Create => {
            state.entities.insert(frame.key.clone(), frame.data.clone());
            if let Some(store) = &mut state.store {
                store.upsert(&frame.key, frame.data.clone(), op_str, frame.seq.clone());
            }
            state.entity_count = state.entities.len() as u64;
            if emit_entity(state, view, &frame.key, op_str, &frame.data)? {
                return Ok(true);
            }
        }
        Operation::Patch => {
            if let Some(store) = &mut state.store {
                store.patch(&frame.key, &frame.data, &frame.append, frame.seq.clone());
            }
            let entry = state.entities
                .entry(frame.key.clone())
                .or_insert_with(|| serde_json::json!({}));
            deep_merge_with_append(entry, &frame.data, &frame.append, "");
            let merged = entry.clone();
            state.entity_count = state.entities.len() as u64;
            if emit_entity(state, view, &frame.key, "patch", &merged)? {
                return Ok(true);
            }
        }
        Operation::Delete => {
            // Filter against last-known state before removing
            let last_state = state.entities.remove(&frame.key).unwrap_or(serde_json::json!(null));
            if let Some(store) = &mut state.store {
                store.delete(&frame.key);
            }
            state.entity_count = state.entities.len() as u64;

            if !state.filter.is_empty() && !state.filter.matches(&last_state) {
                return Ok(false);
            }

            state.update_count += 1;
            if state.count_only {
                output::print_count(state.update_count)?;
            } else {
                match state.output_mode {
                    OutputMode::NoDna => output::emit_no_dna_event(
                        "entity_update", view,
                        &serde_json::json!({"key": frame.key, "op": "delete", "data": null}),
                        state.update_count, state.entity_count,
                    )?,
                    _ => output::print_delete(view, &frame.key)?,
                }
            }
            if state.first {
                return Ok(true);
            }
        }
        Operation::Subscribed => {}
    }

    Ok(false)
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
                "entity_update", view,
                &serde_json::json!({"key": key, "op": op, "data": output_data}),
                state.update_count, state.entity_count,
            )?,
            _ => output::print_entity_update(view, key, op, &output_data)?,
        }
    }

    if state.first {
        return Ok(true);
    }

    Ok(false)
}
