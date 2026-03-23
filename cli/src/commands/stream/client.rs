use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use hyperstack_sdk::{
    deep_merge_with_append, parse_frame, parse_snapshot_entities, ClientMessage, Frame, Operation,
    Subscription,
};
use std::collections::HashMap;
use tokio_tungstenite::{connect_async, tungstenite::Message};

use super::output;
use super::StreamArgs;

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

    // Entity state for merged mode
    let mut entities: HashMap<String, serde_json::Value> = HashMap::new();

    // Ping interval
    let mut ping_interval = tokio::time::interval(std::time::Duration::from_secs(30));

    // Handle Ctrl+C
    let shutdown = tokio::signal::ctrl_c();
    tokio::pin!(shutdown);

    loop {
        tokio::select! {
            msg = ws_rx.next() => {
                match msg {
                    Some(Ok(Message::Binary(bytes))) => {
                        match parse_frame(&bytes) {
                            Ok(frame) => process_frame(frame, args, &mut entities)?,
                            Err(e) => eprintln!("Warning: failed to parse frame: {}", e),
                        }
                    }
                    Some(Ok(Message::Text(text))) => {
                        // Check for subscribed frame
                        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
                            if value.get("op").and_then(|v| v.as_str()) == Some("subscribed") {
                                eprintln!("Subscribed to {}", view);
                                continue;
                            }
                        }
                        match serde_json::from_str::<Frame>(&text) {
                            Ok(frame) => process_frame(frame, args, &mut entities)?,
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
            _ = &mut shutdown => {
                eprintln!("\nDisconnecting...");
                let _ = ws_tx.close().await;
                break;
            }
        }
    }

    Ok(())
}

fn process_frame(
    frame: Frame,
    args: &StreamArgs,
    entities: &mut HashMap<String, serde_json::Value>,
) -> Result<()> {
    if args.raw {
        return output::print_raw_frame(&frame);
    }

    let op = frame.operation();

    match op {
        Operation::Snapshot => {
            let snapshot_entities = parse_snapshot_entities(&frame.data);
            for entity in snapshot_entities {
                entities.insert(entity.key.clone(), entity.data.clone());
                output::print_entity_update(&frame.entity, &entity.key, "snapshot", &entity.data)?;
            }
        }
        Operation::Upsert | Operation::Create => {
            entities.insert(frame.key.clone(), frame.data.clone());
            output::print_entity_update(&frame.entity, &frame.key, &frame.op, &frame.data)?;
        }
        Operation::Patch => {
            let entry = entities
                .entry(frame.key.clone())
                .or_insert_with(|| serde_json::json!({}));
            deep_merge_with_append(entry, &frame.data, &frame.append, "");
            let merged = entry.clone();
            output::print_entity_update(&frame.entity, &frame.key, "patch", &merged)?;
        }
        Operation::Delete => {
            entities.remove(&frame.key);
            output::print_delete(&frame.entity, &frame.key)?;
        }
        Operation::Subscribed => {
            // Handled in the text message branch
        }
    }

    Ok(())
}
