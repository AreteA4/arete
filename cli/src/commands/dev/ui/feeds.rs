//! The producers of the dev event feed `a4 dev up` serves.
//!
//! - [`instances`] polls the deployment's instance list through the Arete
//!   API and reports the state changes it sees (`instance.transitioned`,
//!   without causes).
//! - [`history`] reports the steps `a4 dev step`, the `dev_step` MCP tool and
//!   the dev UI run (`.arete/dev/history.jsonl`): their outcomes, and the
//!   guard rejections that leave no execution behind. Steps an app runs on
//!   the deployment show up as the instance changes they cause.
//! - [`clock`] samples the environment's clock (ephemeral `chain.clock`
//!   ticks, and a buffered `chain.clock` with `jump: true` when the clock
//!   leaps, as after a warp).
//! - [`environment`] follows the environment's lifecycle events from the
//!   Arete API (`env.changed`: woken, slept, checkpointed, restored,
//!   rebased, reset, ...).
//! - [`stack_rows`] streams the protocol stack's primary views and
//!   summarises each changed row as `stack.row`.

use std::collections::BTreeMap;
use std::io::{Read, Seek, SeekFrom};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{bail, Result};
use arete_mcp::dev_env::EnvRpc;
use serde_json::{json, Map, Value};

use super::events::EventHub;
use super::UiState;
use crate::commands::stream::{client as stream_client, dev as stream_dev, protocols, session};

const HISTORY_POLL: Duration = Duration::from_millis(500);
const INSTANCES_POLL: Duration = Duration::from_secs(2);
const CLOCK_POLL: Duration = Duration::from_secs(5);
const ENVIRONMENT_POLL: Duration = Duration::from_secs(5);
/// A clock that moved this much more than wall time did leapt.
const CLOCK_JUMP_SECONDS: i64 = 30;
const SUMMARY_FIELDS: usize = 12;
const SUMMARY_TEXT: usize = 96;

/// The instance states the fallback compares, by id.
pub fn instance_changes(
    before: &BTreeMap<String, String>,
    now: &[Value],
) -> (BTreeMap<String, String>, Vec<Value>) {
    let mut states = BTreeMap::new();
    let mut changes = Vec::new();
    for instance in now {
        let (Some(id), Some(to)) = (instance["id"].as_str(), instance["state"].as_str()) else {
            continue;
        };
        states.insert(id.to_string(), to.to_string());
        let from = before.get(id);
        if from.map(String::as_str) != Some(to) {
            changes.push(json!({
                "deploymentId": instance["deploymentId"],
                "instanceId": id,
                "machine": "instance",
                "from": from,
                "to": to,
                "cause": Value::Null,
                "inferred": true,
                "at": instance["updatedAt"],
            }));
        }
    }
    (states, changes)
}

/// Diff the active deployment's instance list.
pub async fn instances(state: Arc<UiState>) {
    let poll = INSTANCES_POLL;
    let mut known: Option<BTreeMap<String, String>> = None;
    let mut deployment: Option<String> = None;
    loop {
        tokio::time::sleep(poll).await;
        let Ok(client) = state.dev_client() else {
            continue;
        };
        if client.session.deployment_id != deployment {
            deployment = client.session.deployment_id.clone();
            known = None;
        }
        let Ok(instances) = client.list_instances(None, 1000).await else {
            continue;
        };
        let (states, changes) =
            instance_changes(known.as_ref().unwrap_or(&BTreeMap::new()), &instances);
        if known.is_some() {
            for change in changes {
                state.hub.publish("instance.transitioned", change);
            }
        }
        known = Some(states);
    }
}

/// The event a `history.jsonl` line is worth: rejections always (they leave
/// no intent); outcomes when `outcomes`.
pub fn history_event(line: &Value, outcomes: bool) -> Option<(&'static str, Value)> {
    let result = &line["result"];
    let state = result["state"].as_str()?;
    let data = json!({
        "executionId": result["executionId"],
        "instanceId": result["instanceId"].as_str().or(line["instance"].as_str()),
        "step": line["step"],
        "state": state,
        "args": line["args"],
        "devWallet": line["wallet"],
        "rejected": result["rejected"],
        "error": result["message"],
        "hint": result["hint"],
        "source": "history",
    });
    if state == "rejected" && result["executionId"].is_null() {
        return Some(("step.failed", data));
    }
    if !outcomes {
        return None;
    }
    Some((
        if state == "completed" {
            "step.confirmed"
        } else {
            "step.failed"
        },
        data,
    ))
}

/// Tail `.arete/dev/history.jsonl` from its current end.
pub async fn history(state: Arc<UiState>) {
    let path = state.project.history_path();
    let mut offset = std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
    loop {
        tokio::time::sleep(HISTORY_POLL).await;
        let Ok(mut file) = std::fs::File::open(&path) else {
            continue;
        };
        let len = file.metadata().map(|meta| meta.len()).unwrap_or(0);
        if len < offset {
            offset = 0;
        }
        if len == offset || file.seek(SeekFrom::Start(offset)).is_err() {
            continue;
        }
        let mut text = String::new();
        if file.read_to_string(&mut text).is_err() {
            continue;
        }
        let complete = text.rfind('\n').map(|index| index + 1).unwrap_or(0);
        let outcomes = true;
        for line in text[..complete].lines() {
            let Ok(entry) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            if let Some((kind, data)) = history_event(&entry, outcomes) {
                state.hub.publish(kind, data);
            }
        }
        offset += complete as u64;
    }
}

/// Sample the environment's slot and clock.
pub async fn clock(state: Arc<UiState>, rpc: Arc<EnvRpc>) {
    let mut last: Option<(i64, Instant)> = None;
    loop {
        if let Ok((slot, unix)) = arete_mcp::dev_env::read_clock(&rpc).await {
            let data = json!({ "slot": slot, "unixTimestamp": unix });
            state.set_chain(data.clone());
            if let Some((before, at)) = last {
                let wall = at.elapsed().as_secs() as i64;
                if unix - before > wall + CLOCK_JUMP_SECONDS {
                    state.hub.publish(
                        "chain.clock",
                        json!({ "slot": slot, "unixTimestamp": unix, "before": before, "jump": true }),
                    );
                }
            }
            state.hub.publish_ephemeral("chain.clock", data);
            last = Some((unix, Instant::now()));
        }
        tokio::time::sleep(CLOCK_POLL).await;
    }
}

/// The `env.changed` event of one lifecycle event of the API.
pub fn environment_event(event: &Value) -> Value {
    json!({
        "kind": event["kind"],
        "detail": event["detail"],
        "at": event["createdAt"],
        "eventId": event["id"],
    })
}

/// Follow the environment's lifecycle events from now on.
pub async fn environment(state: Arc<UiState>, rpc: Arc<EnvRpc>) {
    let api = rpc.api().clone();
    let env = rpc.env().to_string();
    // Skip what happened before this session.
    let mut cursor: Option<i64> = None;
    loop {
        match api.events(&env, cursor, Some(500)).await {
            Ok(page) => match page["next"].as_i64() {
                Some(next) if Some(next) != cursor => cursor = Some(next),
                _ => break,
            },
            Err(_) => {
                tokio::time::sleep(ENVIRONMENT_POLL).await;
            }
        }
    }
    loop {
        tokio::time::sleep(ENVIRONMENT_POLL).await;
        let Ok(page) = api.events(&env, cursor, Some(100)).await else {
            continue;
        };
        for event in page["events"].as_array().into_iter().flatten() {
            state.hub.publish("env.changed", environment_event(event));
        }
        if let Some(next) = page["next"].as_i64() {
            cursor = Some(next);
        }
    }
}

fn short(value: &Value) -> Value {
    match value {
        Value::String(text) if text.chars().count() > SUMMARY_TEXT => {
            json!(format!(
                "{}…",
                text.chars().take(SUMMARY_TEXT).collect::<String>()
            ))
        }
        other => other.clone(),
    }
}

/// The scalar fields of a row, one level deep (`a.b`), at most
/// [`SUMMARY_FIELDS`] of them.
pub fn summarise_row(data: &Value) -> Value {
    let mut out = Map::new();
    let Some(object) = data.as_object() else {
        return short(data);
    };
    for (key, value) in object {
        if out.len() >= SUMMARY_FIELDS {
            break;
        }
        // Delivery bookkeeping, not row data.
        if key.starts_with('_') {
            continue;
        }
        match value {
            Value::Object(inner) => {
                for (inner_key, inner_value) in inner {
                    if out.len() >= SUMMARY_FIELDS {
                        break;
                    }
                    if !inner_value.is_object() && !inner_value.is_array() {
                        out.insert(format!("{key}.{inner_key}"), short(inner_value));
                    }
                }
            }
            Value::Array(items) => {
                out.insert(key.clone(), json!(format!("[{} items]", items.len())));
            }
            other => {
                out.insert(key.clone(), short(other));
            }
        }
    }
    Value::Object(out)
}

/// The `stack.row` event of one live frame.
pub fn stack_row_event(stack: &str, view: &str, frame: &arete_sdk::ServerFrame) -> Option<Value> {
    use arete_sdk::ServerFrame;
    let (op, entity, key, data) = match frame {
        ServerFrame::Upsert {
            entity, key, data, ..
        } => ("upsert", entity, key, data),
        ServerFrame::Patch {
            entity, key, data, ..
        } => ("patch", entity, key, data),
        ServerFrame::Remove {
            entity, key, data, ..
        } => ("remove", entity, key, data),
        ServerFrame::Delete {
            entity, key, data, ..
        } => ("delete", entity, key, data),
        _ => return None,
    };
    let mut row = data.clone();
    protocols::add_instance_id(entity, &mut row);
    let instance = row
        .get("instance_id")
        .or_else(|| row.get("id").and_then(|id| id.get("instance_id")))
        .cloned()
        .or_else(|| protocols::instance_uuid(key).map(Value::String))
        .unwrap_or(Value::Null);
    Some(json!({
        "stack": stack,
        "view": view,
        "entity": entity,
        "op": op,
        "key": key,
        "instanceId": instance,
        "fields": summarise_row(data),
    }))
}

struct RowHandler {
    hub: Arc<EventHub>,
    stack: String,
    views: BTreeMap<String, String>,
}

impl session::SessionHandler for RowHandler {
    fn on_message(&mut self, message: arete_sdk::ServerMessage) -> Result<bool> {
        if let arete_sdk::ServerMessage::Frame(frame) = message {
            let view = self
                .views
                .get(frame.subscription_id())
                .cloned()
                .unwrap_or_default();
            if let Some(data) = stack_row_event(&self.stack, &view, &frame) {
                self.hub.publish("stack.row", data);
            }
        }
        Ok(false)
    }

    fn on_notice(&mut self, notice: session::Notice<'_>) {
        if let session::Notice::Redeployed { from, to } = notice {
            self.hub
                .publish("stack.redeployed", json!({ "from": from, "to": to }));
        }
    }

    fn on_reconnected(&mut self, _url: &str) -> Result<()> {
        Ok(())
    }
}

/// Stream the protocol stack's primary views as `stack.row` events, waiting
/// for a stack when there is none yet.
pub async fn stack_rows(state: Arc<UiState>) {
    loop {
        if let Err(error) = stream_stack_rows(&state).await {
            state.set_stack_error(Some(format!("{error:#}")));
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

async fn stream_stack_rows(state: &Arc<UiState>) -> Result<()> {
    let runtime = stream_dev::DevRuntime::open(Some(&state.project.root))?;
    let listing = runtime.listing().await?;
    let stack = stream_dev::select_stack(&listing.stacks, None, &runtime.session)?.clone();
    let views = protocols::primary_views(&stack);
    if views.is_empty() {
        bail!("the protocol stack lists no views");
    }
    let mut refresh = runtime.refresh_for(&stack);
    let url = refresh.renew().await?;
    let feeds: Vec<stream_client::Feed> = views
        .iter()
        .map(|entry| stream_client::Feed::new(&entry.raw, Some(stack.name.clone())))
        .collect();
    let subscriptions: Vec<arete_sdk::Subscription> = feeds
        .iter()
        .map(|feed| {
            let query = arete_sdk::SubscriptionQuery::new(&feed.view);
            arete_sdk::Subscription::new(
                format!("a4-dev-ui:{}", uuid::Uuid::new_v4().simple()),
                query,
            )
            .with_snapshot(false)
        })
        .collect();
    let mut handler = RowHandler {
        hub: state.hub.clone(),
        stack: stack.name.clone(),
        views: subscriptions
            .iter()
            .zip(&views)
            .map(|(subscription, entry)| {
                (subscription.subscription_id.clone(), entry.friendly.clone())
            })
            .collect(),
    };
    let follow = stream_dev::watch(runtime.clone(), None, stack.identity());
    let mut stream = session::StreamSession::with_subscriptions(
        url,
        Some(refresh),
        subscriptions,
        session::ReconnectPolicy::new(true, None),
    )?
    .follow(Some(follow));
    let socket = stream.connect().await?;
    state.set_stack_error(None);
    let stop = std::future::pending::<()>();
    tokio::pin!(stop);
    stream.run(socket, &mut handler, stop).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fallback_reports_state_changes() {
        let before = BTreeMap::from([("a".to_string(), "open".to_string())]);
        let (states, changes) = instance_changes(
            &before,
            &[
                json!({ "id": "a", "state": "closed", "deploymentId": "d" }),
                json!({ "id": "b", "state": "open", "deploymentId": "d" }),
            ],
        );
        assert_eq!(states.len(), 2);
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0]["from"], "open");
        assert_eq!(changes[0]["to"], "closed");
        assert_eq!(changes[1]["from"], Value::Null);
        assert_eq!(changes[1]["inferred"], true);
    }

    #[test]
    fn history_reports_rejections_and_fallback_outcomes() {
        let rejected = json!({ "step": "close", "wallet": "bob", "args": {},
            "result": { "executionId": null, "instanceId": null, "state": "rejected", "rejected": "not_recipient" } });
        let (kind, data) = history_event(&rejected, false).unwrap();
        assert_eq!(kind, "step.failed");
        assert_eq!(data["rejected"], "not_recipient");
        let done = json!({ "step": "open", "wallet": "a", "result": { "executionId": "e", "state": "completed" } });
        assert!(history_event(&done, false).is_none());
        assert_eq!(history_event(&done, true).unwrap().0, "step.confirmed");
    }

    #[test]
    fn stack_rows_carry_the_instance_id() {
        let uuid = uuid::Uuid::from_u128(7);
        let short = bs58::encode(uuid.as_bytes()).into_string();
        let frame: arete_sdk::ServerFrame = serde_json::from_value(json!({
            "op": "upsert", "protocolVersion": 1, "subscriptionId": "s", "mode": "list",
            "entity": "ProtocolActivity/list", "key": "sig",
            "data": { "instance": short, "amount": "5", "_seq": "1:0" }
        }))
        .unwrap();
        let event = stack_row_event("jar", "Activity", &frame).unwrap();
        assert_eq!(event["instanceId"], uuid.to_string());
        assert_eq!(event["op"], "upsert");
        assert!(event["fields"].get("_seq").is_none());
    }

    #[test]
    fn summarises_rows() {
        let summary = summarise_row(&json!({
            "id": { "instance": "abc", "deep": { "x": 1 } },
            "state": "open",
            "history": [1, 2, 3],
            "memo": "x".repeat(200),
        }));
        assert_eq!(summary["id.instance"], "abc");
        assert!(summary.get("id.deep").is_none());
        assert_eq!(summary["history"], "[3 items]");
        assert!(summary["memo"].as_str().unwrap().ends_with('…'));
    }

    #[test]
    fn lifecycle_events_become_env_changed() {
        let event = environment_event(&json!({
            "id": 7, "kind": "slept", "detail": { "reason": "idle" }, "createdAt": "2026-10-17T00:00:00Z"
        }));
        assert_eq!(event["kind"], "slept");
        assert_eq!(event["detail"]["reason"], "idle");
        assert_eq!(event["eventId"], 7);
    }
}
