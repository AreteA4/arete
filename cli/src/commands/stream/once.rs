//! `a4 get`: read a view's current entities once and exit.
//!
//! Connects the way `a4 stream` does (same URL resolution and session
//! token), subscribes with a snapshot, waits for the snapshot to complete,
//! prints the entities as one JSON document and disconnects. For "what is
//! the current X" questions, where a stream that never ends is the wrong
//! tool.

use std::time::Duration;

use anyhow::{bail, Context, Result};
use arete_sdk::{Frame, ServerMessage, SnapshotEntity, Subscription, SubscriptionQuery};
use clap::Args;
use serde_json::{json, Value};

use super::filter::{self, Filter};
use super::session::{Notice, ReconnectPolicy, SessionHandler, StreamEnd, StreamSession};
use super::token;

#[derive(Args)]
pub struct GetArgs {
    /// View to read: EntityName/mode (e.g. OreRound/latest)
    pub view: String,

    /// Owned deployment or registry stack name (e.g. ore)
    #[arg(short, long)]
    pub stack: Option<String>,

    /// WebSocket URL override
    #[arg(long)]
    pub url: Option<String>,

    /// Read the one entity with this key (usually with an EntityName/state view)
    #[arg(short, long)]
    pub key: Option<String>,

    /// Filter expression: field=value, field>N, field~regex (repeatable, ANDed)
    #[arg(long = "where", value_name = "EXPR")]
    pub filters: Vec<String>,

    /// Fields to output (comma-separated dot paths, e.g. "id.round_id,state.motherlode")
    #[arg(long)]
    pub select: Option<String>,

    /// Most entities to print, in the view's order (`--limit 1` for the
    /// current entity of a sorted view like OreRound/latest)
    #[arg(long, default_value_t = 10, value_parser = clap::value_parser!(u32).range(1..))]
    pub limit: u32,

    /// Seconds to wait for the connection and snapshot before failing
    #[arg(long, value_name = "SECS", default_value_t = 15, value_parser = clap::value_parser!(u64).range(1..))]
    pub timeout: u64,
}

/// Units and the output shape, for `a4 get --help`.
pub const GET_HELP: &str = "\
Output:
  One JSON document on stdout:
    {\"view\", \"key\"?, \"total\", \"matching\", \"returned\", \"truncated\", \"entities\": [...]}
  \"entities\" holds at most --limit entities in the view's order. Without
  --where the server sends only --limit of them, so \"total\" is at most
  --limit; with --where, \"total\" counts the whole snapshot and \"matching\"
  the entities it kept. With --key, an entity that does not exist is an
  error.

Token amounts:
  Float fields a stack derives with ui_amount are whole token units; the
  integer fields they come from are raw base units (often string-encoded
  u64; for SOL, lamports). `a4 explore stack <stack> --views <view>` shows
  each scaled field's scale and decimals.

Examples:
  a4 get OreRound/latest --stack ore --limit 1
  a4 get OreRound/latest --stack ore --limit 1 --select id.round_id,state.total_deployed,treasury.motherlode
  a4 get OreMiner/state --stack ore --key <authority>";

pub fn run(args: GetArgs) -> Result<()> {
    let view = args.view.trim();
    if !view.contains('/') {
        bail!("<VIEW> must be EntityName/mode (e.g. OreRound/latest), got '{view}'");
    }
    let filter = Filter::parse(&args.filters)?;
    let select = args.select.as_deref().map(filter::parse_select);

    let url = super::resolve_ws_url(args.url.as_deref(), args.stack.as_deref())?;
    let (url, refresh) = token::ensure_hosted_ws_token(url)?;

    let rt = tokio::runtime::Runtime::new().context("Failed to create async runtime")?;
    let timeout = Duration::from_secs(args.timeout);
    // Without filters the server can cut the snapshot to --limit; with them
    // the whole snapshot is needed to find the matches.
    let take = (args.key.is_none() && filter.is_empty()).then_some(args.limit as usize);
    let rows = rt.block_on(read_snapshot(
        url,
        refresh,
        view,
        args.key.clone(),
        take,
        timeout,
    ))?;

    let output = shape_output(
        view,
        args.key.as_deref(),
        rows,
        &filter,
        select.as_deref(),
        args.limit as usize,
    )?;
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

/// Subscribe to `view` and return its first complete snapshot.
async fn read_snapshot(
    url: String,
    refresh: Option<token::SessionRefresh>,
    view: &str,
    key: Option<String>,
    take: Option<usize>,
    timeout: Duration,
) -> Result<Vec<SnapshotEntity>> {
    let deadline = tokio::time::Instant::now() + timeout;
    let subscription_id = format!("a4-get:{}", uuid::Uuid::new_v4().simple());
    let mut query = SubscriptionQuery::new(view);
    if let Some(key) = key {
        query = query.with_key(key);
    }
    if let Some(take) = take {
        query = query.with_take(take);
    }
    let subscription = Subscription::new(subscription_id.clone(), query).with_snapshot(true);
    // A dropped connection is retried a couple of times inside the timeout:
    // the new subscription takes a fresh snapshot.
    let policy = ReconnectPolicy::new(true, Some(2));
    let mut session = StreamSession::new(url, refresh, subscription, policy)?;

    let socket = tokio::time::timeout_at(deadline, session.connect())
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "Timed out after {}s connecting to {}",
                timeout.as_secs(),
                session.display_url()
            )
        })??;

    let mut collector = Collector::new(subscription_id);
    let stop = tokio::time::sleep_until(deadline);
    tokio::pin!(stop);
    let end = session.run(socket, &mut collector, stop).await?;

    if let Some(rows) = collector.snapshot {
        return Ok(rows);
    }
    if let Some(error) = collector.error {
        bail!("{error}");
    }
    match end {
        StreamEnd::Stopped => bail!(
            "Timed out after {}s waiting for the snapshot of {view}. Check the view id with \
             `a4 explore stack <stack>`, or raise --timeout.",
            timeout.as_secs()
        ),
        StreamEnd::Failed(failure) => bail!("{failure}"),
        StreamEnd::Lost(loss) => bail!("{loss}"),
        StreamEnd::Finished => {
            bail!("The server ended the subscription to {view} before its snapshot")
        }
    }
}

/// Collects one complete snapshot, across batches.
struct Collector {
    subscription_id: String,
    pending: Option<(String, Vec<SnapshotEntity>)>,
    snapshot: Option<Vec<SnapshotEntity>>,
    error: Option<String>,
}

impl Collector {
    fn new(subscription_id: String) -> Self {
        Self {
            subscription_id,
            pending: None,
            snapshot: None,
            error: None,
        }
    }
}

impl SessionHandler for Collector {
    fn on_message(&mut self, message: ServerMessage) -> Result<bool> {
        match message {
            ServerMessage::Error(error) => {
                let ours = error.subscription_id.as_deref() == Some(self.subscription_id.as_str());
                let text = format!("Server error [{}]: {}", error.code, error.message);
                if error.fatal || ours {
                    self.error = Some(text);
                    return Ok(true);
                }
                eprintln!("{text}");
                Ok(false)
            }
            ServerMessage::Frame(Frame::Snapshot {
                snapshot_id,
                data,
                complete,
                ..
            }) => {
                let (id, rows) = self
                    .pending
                    .get_or_insert_with(|| (snapshot_id.clone(), Vec::new()));
                if *id != snapshot_id {
                    // A new snapshot replaces one left incomplete.
                    *id = snapshot_id;
                    rows.clear();
                }
                for row in data {
                    match rows.iter_mut().find(|item| item.key == row.key) {
                        Some(existing) => *existing = row,
                        None => rows.push(row),
                    }
                }
                if complete {
                    self.snapshot = self.pending.take().map(|(_, rows)| rows);
                    return Ok(true);
                }
                Ok(false)
            }
            ServerMessage::Frame(Frame::Unsubscribed { .. }) => Ok(true),
            ServerMessage::Frame(_) => Ok(false),
        }
    }

    fn on_notice(&mut self, notice: Notice<'_>) {
        match notice {
            Notice::Reconnecting(reconnecting) => eprintln!("{reconnecting}"),
            Notice::Unparsed { binary, error } => eprintln!(
                "Warning: failed to parse {} frame: {error}",
                if binary { "binary" } else { "text" }
            ),
            Notice::RefreshRefused(_) | Notice::RefreshFailed(_) => {}
        }
    }

    fn on_reconnected(&mut self, _url: &str) -> Result<()> {
        self.pending = None;
        Ok(())
    }
}

/// The JSON document `a4 get` prints.
fn shape_output(
    view: &str,
    key: Option<&str>,
    rows: Vec<SnapshotEntity>,
    filter: &Filter,
    select: Option<&[Vec<String>]>,
    limit: usize,
) -> Result<Value> {
    let total = rows.len();
    if let Some(key) = key {
        if !rows.iter().any(|row| row.key == key) {
            bail!(
                "No entity with key '{key}' in {view}. Keyed reads usually need the entity's \
                 /state view; list keys with `a4 get <Entity>/list --stack <stack>`."
            );
        }
    }
    let matched: Vec<Value> = rows
        .into_iter()
        .filter(|row| key.is_none_or(|key| row.key == key))
        .map(|row| row.data)
        .filter(|data| filter.is_empty() || filter.matches(data))
        .collect();
    let matching = matched.len();
    let entities: Vec<Value> = matched
        .into_iter()
        .take(limit)
        .map(|data| match select {
            Some(fields) => filter::select_fields(&data, fields),
            None => data,
        })
        .collect();
    let mut output = json!({
        "view": view,
        "total": total,
        "matching": matching,
        "returned": entities.len(),
        "truncated": matching > entities.len(),
        "entities": entities,
    });
    if let Some(key) = key {
        output["key"] = json!(key);
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use arete_sdk::Mode;

    fn snapshot(id: &str, complete: bool, keys: &[(&str, u64)]) -> ServerMessage {
        ServerMessage::Frame(Frame::Snapshot {
            protocol_version: 2,
            subscription_id: "a4-get:test".to_string(),
            snapshot_id: id.to_string(),
            authoritative: true,
            mode: Mode::List,
            entity: "Round/latest".to_string(),
            key: None,
            data: keys
                .iter()
                .map(|(key, n)| SnapshotEntity {
                    key: (*key).to_string(),
                    data: json!({"id": key, "n": n}),
                })
                .collect(),
            complete,
        })
    }

    #[test]
    fn collects_every_batch_of_the_first_complete_snapshot() {
        let mut collector = Collector::new("a4-get:test".into());
        assert!(!collector
            .on_message(snapshot("s1", false, &[("a", 1), ("b", 2)]))
            .unwrap());
        assert!(collector
            .on_message(snapshot("s1", true, &[("b", 3), ("c", 4)]))
            .unwrap());
        let rows = collector.snapshot.unwrap();
        let keys: Vec<_> = rows.iter().map(|row| row.key.as_str()).collect();
        assert_eq!(keys, ["a", "b", "c"]);
        assert_eq!(rows[1].data["n"], 3);
    }

    #[test]
    fn an_empty_snapshot_completes_the_read() {
        let mut collector = Collector::new("a4-get:test".into());
        assert!(collector.on_message(snapshot("s1", true, &[])).unwrap());
        assert_eq!(collector.snapshot.unwrap().len(), 0);
    }

    #[test]
    fn a_reconnect_drops_a_partial_snapshot() {
        let mut collector = Collector::new("a4-get:test".into());
        collector
            .on_message(snapshot("s1", false, &[("a", 1)]))
            .unwrap();
        collector.on_reconnected("wss://example.test").unwrap();
        collector
            .on_message(snapshot("s2", true, &[("b", 2)]))
            .unwrap();
        let rows = collector.snapshot.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].key, "b");
    }

    fn rows() -> Vec<SnapshotEntity> {
        (1..=3)
            .map(|n| SnapshotEntity {
                key: n.to_string(),
                data: json!({"id": {"n": n}, "state": {"v": n * 10}}),
            })
            .collect()
    }

    #[test]
    fn output_filters_selects_and_limits() {
        let filter = Filter::parse(&["state.v>10".to_string()]).unwrap();
        let select = filter::parse_select("id.n");
        let output = shape_output("T/list", None, rows(), &filter, Some(&select), 1).unwrap();
        assert_eq!(
            output,
            json!({
                "view": "T/list",
                "total": 3,
                "matching": 2,
                "returned": 1,
                "truncated": true,
                "entities": [{"id.n": 2}],
            })
        );
    }

    #[test]
    fn a_missing_key_is_an_error() {
        let filter = Filter::parse(&[]).unwrap();
        let error = shape_output("T/state", Some("9"), rows(), &filter, None, 20).unwrap_err();
        assert!(error.to_string().contains("No entity with key '9'"));
        let found = shape_output("T/state", Some("2"), rows(), &filter, None, 20).unwrap();
        assert_eq!(found["entities"][0]["state"]["v"], 20);
        assert_eq!(found["key"], "2");
    }
}
