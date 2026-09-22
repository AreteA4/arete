//! Bounded append-only record of published events, per view.
//!
//! The entity cache is a latest-state projection: `deep_merge_with_append`
//! folds each patch into the resident entity, so an intermediate occurrence is
//! unrecoverable however large the cache is. Resuming a consumer therefore
//! cannot be served from it — a replayable subscription needs the events
//! themselves, kept in arrival order.
//!
//! # Cursors
//!
//! Each view has its own dense monotonic offset, assigned here at append time.
//! The wire `_seq` (`{slot}:{slot_index:012}`) is deliberately *not* used as
//! the replay cursor: `slot_index` is `txn_index` for instruction updates, so
//! every event decoded from one transaction shares it and "replay each
//! occurrence exactly once" is not expressible. An append-assigned offset is
//! unique and gap-free by construction.
//!
//! A cursor is `{epoch}:{offset}`. The epoch identifies one tape lifetime.
//! Offsets restart at zero whenever a tape is built without restoring one
//! (snapshots disabled, a rejected or corrupt blob, any cold start), and dense
//! offsets make a stale cursor indistinguishable from a live one — so without
//! the epoch, a cursor from a previous lifetime would eventually land inside
//! the new window and replay unrelated events as a continuation. The epoch
//! makes that fail closed.
//!
//! Offsets are per view and are not comparable across views.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

const DEFAULT_MAX_RECORDS_PER_VIEW: usize = 10_000;
const DEFAULT_MAX_BYTES_PER_VIEW: u64 = 32 * 1024 * 1024;
const DEFAULT_MAX_AGE: Duration = Duration::from_secs(15 * 60);

/// Per-record bookkeeping charged against the byte bound alongside the frame:
/// the key `String`, the `Arc` control block, `Bytes` header and `VecDeque`
/// slot. Approximate by design — the bound exists to be budgetable, not exact.
const RECORD_OVERHEAD_BYTES: u64 = 120;

/// Retention bounds for the event journal. Whichever bound bites first wins.
#[derive(Clone, Debug)]
pub struct JournalConfig {
    /// Master opt-in. Disabled leaves append views on their previous
    /// latest-state delivery.
    pub enabled: bool,
    /// Retained bytes per view, counting frames plus per-record overhead.
    ///
    /// This is the bound to budget against: frame size is stack-dependent, so
    /// a record count cannot be reasoned about against a memory limit.
    pub max_bytes_per_view: u64,
    /// Retained records per view. A backstop against pathologically small
    /// frames; the byte bound is the one that should normally bite.
    pub max_records_per_view: usize,
    /// Records older than this are dropped even when the size bounds are not
    /// reached, so a quiet view does not advertise a stale replay window.
    pub max_age: Duration,
}

impl Default for JournalConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_bytes_per_view: DEFAULT_MAX_BYTES_PER_VIEW,
            max_records_per_view: DEFAULT_MAX_RECORDS_PER_VIEW,
            max_age: DEFAULT_MAX_AGE,
        }
    }
}

impl JournalConfig {
    /// Load from `ARETE_JOURNAL_*`. Stays disabled unless
    /// `ARETE_JOURNAL_ENABLED=true`.
    ///
    /// An embedder hosting several deployments in one process should use
    /// [`crate::ServerBuilder::journal`] instead, which takes these as the
    /// default and overrides them per runtime.
    pub fn from_env() -> anyhow::Result<Self> {
        let mut config = Self::default();
        config.enabled = crate::config::env_bool("ARETE_JOURNAL_ENABLED")?.unwrap_or(false);
        config.max_bytes_per_view = crate::config::env_parse("ARETE_JOURNAL_MAX_BYTES")?
            .unwrap_or(config.max_bytes_per_view);
        config.max_records_per_view = crate::config::env_parse("ARETE_JOURNAL_MAX_RECORDS")?
            .unwrap_or(config.max_records_per_view);
        config.max_age = Duration::from_secs(
            crate::config::env_parse("ARETE_JOURNAL_MAX_AGE_SECS")?
                .unwrap_or(config.max_age.as_secs()),
        );
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        if !self.enabled {
            return Ok(());
        }
        if self.max_bytes_per_view == 0 {
            anyhow::bail!("ARETE_JOURNAL_MAX_BYTES must be greater than zero");
        }
        if self.max_records_per_view == 0 {
            anyhow::bail!("ARETE_JOURNAL_MAX_RECORDS must be greater than zero");
        }
        if self.max_age.is_zero() {
            anyhow::bail!("ARETE_JOURNAL_MAX_AGE_SECS must be greater than zero");
        }
        Ok(())
    }
}

/// Identifies one tape lifetime, so a cursor minted by a previous one cannot
/// be mistaken for a live offset.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct JournalEpoch(String);

impl JournalEpoch {
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for JournalEpoch {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for JournalEpoch {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// A cursor as it travels on the wire: `{epoch}:{offset}`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cursor {
    pub epoch: JournalEpoch,
    pub offset: u64,
}

impl Cursor {
    pub fn parse(raw: &str) -> Option<Self> {
        let (epoch, offset) = raw.rsplit_once(':')?;
        if epoch.is_empty() {
            return None;
        }
        Some(Self {
            epoch: JournalEpoch(epoch.to_string()),
            offset: offset.parse().ok()?,
        })
    }
}

impl std::fmt::Display for Cursor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}:{}", self.epoch, self.offset)
    }
}

/// One published event, retained verbatim.
///
/// `payload` is the exact bytes the projector published, so a replay is
/// byte-identical to what a live subscriber received rather than a
/// re-projection of current state.
#[derive(Clone, Debug)]
pub struct JournalRecord {
    pub offset: u64,
    pub key: String,
    pub payload: Arc<Bytes>,
    /// Unix seconds, for the age bound.
    pub appended_at: i64,
}

impl JournalRecord {
    fn charged_bytes(&self) -> u64 {
        self.payload.len() as u64 + self.key.len() as u64 + RECORD_OVERHEAD_BYTES
    }
}

/// The offsets a view can currently serve.
///
/// `earliest` is the oldest retained offset, so a cursor below it has fallen
/// out of the window and cannot be honoured. `next` is the offset the next
/// append will take, so a consumer at `next - 1` is fully caught up.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayWindow {
    pub epoch: JournalEpoch,
    pub earliest: u64,
    pub next: u64,
    /// When set, records after this offset were lost before the tape resumed:
    /// the stream restarted live rather than from the snapshot's watermark.
    /// Replay across this boundary is refused rather than presented as
    /// continuous.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gap_after: Option<u64>,
}

impl ReplayWindow {
    pub fn is_empty(&self) -> bool {
        self.earliest >= self.next
    }

    /// The cursor a caught-up consumer would hold.
    pub fn latest_cursor(&self) -> Option<Cursor> {
        (self.next > self.earliest).then(|| Cursor {
            epoch: self.epoch.clone(),
            offset: self.next - 1,
        })
    }
}

/// Why a replay could not be served.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplayError {
    /// The cursor was minted by a previous tape lifetime. Its offsets mean
    /// nothing here, and dense offsets make that undetectable without the
    /// epoch.
    EpochMismatch(ReplayWindow),
    /// The cursor is older than the oldest retained record.
    CursorExpired(ReplayWindow),
    /// The cursor is one this view has never issued. Refused rather than
    /// treated as caught up, which would suppress delivery until the view's
    /// offsets reached it.
    CursorBeyondWindow(ReplayWindow),
    /// Serving this cursor would cross a known hole in the tape.
    GapCrossed(ReplayWindow),
}

impl ReplayError {
    pub fn window(&self) -> &ReplayWindow {
        match self {
            Self::EpochMismatch(window)
            | Self::CursorExpired(window)
            | Self::CursorBeyondWindow(window)
            | Self::GapCrossed(window) => window,
        }
    }
}

/// Durable form of one retained record.
///
/// `payload` persists as a JSON string rather than a byte array: the frame is
/// already UTF-8 JSON, and `serde_json` writes `Vec<u8>` as an array of
/// decimal integers, which inflates it roughly fourfold before compression and
/// compresses worse than the text it came from. Escaping costs a little; a
/// `RawValue` would cost nothing at all and is the upgrade if this ever shows
/// up in a profile.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PersistedRecord {
    pub offset: u64,
    pub key: String,
    #[serde(with = "frame_text")]
    pub payload: Arc<Bytes>,
    pub appended_at: i64,
}

/// Serialize retained frames as JSON text, straight out of the shared buffer.
///
/// Serializing borrows the bytes, so a snapshot dump clones the `Arc` rather
/// than deep-copying every payload inside the consistency guard.
mod frame_text {
    use super::*;
    use serde::{Deserializer, Serializer};

    pub fn serialize<S: Serializer>(
        payload: &Arc<Bytes>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let text = std::str::from_utf8(payload)
            .map_err(|_| serde::ser::Error::custom("retained frame is not UTF-8"))?;
        serializer.serialize_str(text)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Arc<Bytes>, D::Error> {
        let text = String::deserialize(deserializer)?;
        Ok(Arc::new(Bytes::from(text.into_bytes())))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PersistedViewJournal {
    pub next_offset: u64,
    pub records: Vec<PersistedRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gap_after: Option<u64>,
}

/// The retained tape for every view, carried in the snapshot payload.
///
/// ponytail: this rides inside the existing whole-state snapshot rather than
/// its own segment files. That buys the snapshot's consistency cut for free —
/// the tape and the entity cache are dumped under one barrier, so a restore
/// can never leave the cache ahead of the tape — at the cost of rewriting the
/// retained tape on every snapshot. Retention is bounded, so the cost is
/// bounded too; if a deployment raises the bounds far enough that whole-blob
/// rewrites hurt, the upgrade path is append-only segment objects alongside
/// the snapshot.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct JournalSnapshot {
    /// Absent in snapshots written before cursors carried an epoch; such a
    /// tape is discarded on restore rather than adopted under a new epoch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epoch: Option<JournalEpoch>,
    pub views: HashMap<String, PersistedViewJournal>,
}

#[derive(Debug, Default)]
struct ViewJournal {
    /// Offset the next append takes. Never rewound, including after pruning.
    next_offset: u64,
    /// Retained records, oldest first.
    records: VecDeque<JournalRecord>,
    /// Running total of `charged_bytes`, maintained on append and prune.
    retained_bytes: u64,
    /// Set when records after this offset were lost; see [`ReplayWindow`].
    gap_after: Option<u64>,
}

impl ViewJournal {
    fn window(&self, epoch: &JournalEpoch) -> ReplayWindow {
        ReplayWindow {
            epoch: epoch.clone(),
            earliest: self
                .records
                .front()
                .map(|record| record.offset)
                .unwrap_or(self.next_offset),
            next: self.next_offset,
            gap_after: self.gap_after,
        }
    }

    fn pop_oldest(&mut self) {
        if let Some(record) = self.records.pop_front() {
            self.retained_bytes = self.retained_bytes.saturating_sub(record.charged_bytes());
        }
    }

    fn prune(&mut self, config: &JournalConfig, now: i64) {
        // Timestamps are whole seconds, so a record that has reached exactly
        // `max_age` is retired. Comparing against a `now - max_age` cutoff
        // strictly would keep it for another whole second.
        let max_age = config.max_age.as_secs() as i64;
        while self
            .records
            .front()
            .is_some_and(|record| record.appended_at.saturating_add(max_age) <= now)
        {
            self.pop_oldest();
        }
        while self.records.len() > config.max_records_per_view {
            self.pop_oldest();
        }
        // Always keep one record, so a view whose frames exceed the byte bound
        // on their own still has a window rather than silently retaining
        // nothing.
        while self.retained_bytes > config.max_bytes_per_view && self.records.len() > 1 {
            self.pop_oldest();
        }
        // A hole that has fallen out of the window no longer constrains
        // anything that can still be replayed.
        if let (Some(gap), Some(oldest)) = (self.gap_after, self.records.front()) {
            if oldest.offset > gap {
                self.gap_after = None;
            }
        }
    }
}

/// Per-view append-only event log with bounded retention.
#[derive(Debug)]
pub struct EventJournal {
    epoch: RwLock<JournalEpoch>,
    views: RwLock<HashMap<String, ViewJournal>>,
    config: JournalConfig,
}

impl EventJournal {
    pub fn new(config: JournalConfig) -> Self {
        Self {
            epoch: RwLock::new(JournalEpoch::new()),
            views: RwLock::new(HashMap::new()),
            config,
        }
    }

    pub fn config(&self) -> &JournalConfig {
        &self.config
    }

    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    pub async fn epoch(&self) -> JournalEpoch {
        self.epoch.read().await.clone()
    }

    /// Append one published event, building its frame from the offset it is
    /// about to take.
    ///
    /// Reserving and committing happen under one lock so the offset embedded
    /// in the published frame is always the offset the record takes. A live
    /// subscriber checkpoints that value, so a mismatch would hand it a cursor
    /// that means something else.
    pub async fn append_with<E>(
        &self,
        view_id: &str,
        key: &str,
        build_frame: impl FnOnce(u64) -> Result<Arc<Bytes>, E>,
    ) -> Result<(u64, Arc<Bytes>), E> {
        self.append_with_at(view_id, key, unix_now(), build_frame)
            .await
    }

    async fn append_with_at<E>(
        &self,
        view_id: &str,
        key: &str,
        now: i64,
        build_frame: impl FnOnce(u64) -> Result<Arc<Bytes>, E>,
    ) -> Result<(u64, Arc<Bytes>), E> {
        let mut views = self.views.write().await;
        let journal = views.entry(view_id.to_string()).or_default();
        let offset = journal.next_offset;
        let payload = build_frame(offset)?;

        journal.next_offset += 1;
        let record = JournalRecord {
            offset,
            key: key.to_string(),
            payload: payload.clone(),
            appended_at: now,
        };
        journal.retained_bytes = journal
            .retained_bytes
            .saturating_add(record.charged_bytes());
        journal.records.push_back(record);
        journal.prune(&self.config, now);
        Ok((offset, payload))
    }

    /// Record that events were lost before the tape resumed.
    ///
    /// Called when a restore hydrates state but starts the stream live: the
    /// retained records stay valid, but everything between them and the first
    /// live append is missing, and dense offsets would otherwise present that
    /// hole as continuous.
    pub async fn mark_gap(&self) {
        let mut views = self.views.write().await;
        for journal in views.values_mut() {
            if journal.next_offset > 0 {
                journal.gap_after = Some(journal.next_offset - 1);
            }
        }
    }

    /// Offsets this view can currently serve.
    ///
    /// Prunes first: the age bound has to hold for a view that has gone
    /// quiet, otherwise expired records stay advertised and replayable.
    pub async fn window(&self, view_id: &str) -> ReplayWindow {
        let epoch = self.epoch.read().await.clone();
        let mut views = self.views.write().await;
        let now = unix_now();
        match views.get_mut(view_id) {
            Some(journal) => {
                journal.prune(&self.config, now);
                journal.window(&epoch)
            }
            None => ReplayWindow {
                epoch,
                earliest: 0,
                next: 0,
                gap_after: None,
            },
        }
    }

    /// Every retained record strictly after `cursor`, in offset order.
    ///
    /// A consumer that has seen nothing passes `None`, which replays the whole
    /// retained window.
    pub async fn replay_after(
        &self,
        view_id: &str,
        cursor: Option<&Cursor>,
    ) -> Result<Vec<JournalRecord>, ReplayError> {
        let epoch = self.epoch.read().await.clone();
        let mut views = self.views.write().await;
        let now = unix_now();
        let Some(journal) = views.get_mut(view_id) else {
            return Ok(Vec::new());
        };
        journal.prune(&self.config, now);
        let window = journal.window(&epoch);

        if let Some(cursor) = cursor {
            // A cursor from another tape lifetime says nothing about this one.
            if cursor.epoch != epoch {
                return Err(ReplayError::EpochMismatch(window));
            }
            let offset = cursor.offset;
            // Below the window: the records are gone.
            if offset.saturating_add(1) < window.earliest {
                return Err(ReplayError::CursorExpired(window));
            }
            // Above it: a cursor the view has never issued. Accepting it would
            // silently suppress every later record until the offsets caught
            // up, so refuse it rather than appear to work.
            if window.next == 0 || offset >= window.next {
                return Err(ReplayError::CursorBeyondWindow(window));
            }
            // Resuming from before a known hole would present the records
            // after it as an unbroken continuation.
            if window.gap_after.is_some_and(|gap| offset <= gap) {
                return Err(ReplayError::GapCrossed(window));
            }
        }

        let first_wanted = cursor
            .map(|cursor| cursor.offset.saturating_add(1))
            .unwrap_or(window.earliest);
        Ok(journal
            .records
            .iter()
            .filter(|record| record.offset >= first_wanted)
            .cloned()
            .collect())
    }

    /// Durable form of the retained tape, for the snapshot payload.
    ///
    /// Payload `Arc`s are cloned, not the bytes, so this stays cheap inside
    /// the snapshot's consistency guard.
    pub async fn dump(&self) -> JournalSnapshot {
        let epoch = self.epoch.read().await.clone();
        let views = self.views.read().await;
        JournalSnapshot {
            epoch: Some(epoch),
            views: views
                .iter()
                .map(|(view_id, journal)| {
                    (
                        view_id.clone(),
                        PersistedViewJournal {
                            next_offset: journal.next_offset,
                            gap_after: journal.gap_after,
                            records: journal
                                .records
                                .iter()
                                .map(|record| PersistedRecord {
                                    offset: record.offset,
                                    key: record.key.clone(),
                                    payload: record.payload.clone(),
                                    appended_at: record.appended_at,
                                })
                                .collect(),
                        },
                    )
                })
                .collect(),
        }
    }

    /// Restore a dumped tape, so the advertised replay window survives a
    /// restart and a consumer's cursor stays meaningful.
    ///
    /// Adopts the snapshot's epoch: the offsets continue the same lifetime, so
    /// cursors held across the restart remain valid. A snapshot without an
    /// epoch predates cursor epochs and is discarded — adopting it under a
    /// fresh epoch would be indistinguishable from a cold start anyway, and
    /// adopting its offsets under this tape's epoch would validate cursors
    /// that should fail.
    ///
    /// Retention is re-applied on load: a snapshot restored after a long
    /// outage must not advertise records the age bound has already retired.
    pub async fn hydrate(&self, snapshot: JournalSnapshot) {
        if !self.config.enabled {
            return;
        }
        let Some(epoch) = snapshot.epoch else {
            return;
        };
        let now = unix_now();
        *self.epoch.write().await = epoch;
        let mut views = self.views.write().await;
        for (view_id, persisted) in snapshot.views {
            let records: VecDeque<JournalRecord> = persisted
                .records
                .into_iter()
                .map(|record| JournalRecord {
                    offset: record.offset,
                    key: record.key,
                    payload: record.payload,
                    appended_at: record.appended_at,
                })
                .collect();
            let retained_bytes = records.iter().map(JournalRecord::charged_bytes).sum();
            let mut journal = ViewJournal {
                next_offset: persisted.next_offset,
                records,
                retained_bytes,
                gap_after: persisted.gap_after,
            };
            journal.prune(&self.config, now);
            views.insert(view_id, journal);
        }
    }

    /// Retained record count per view, for snapshot diagnostics.
    pub async fn entry_counts(&self) -> Vec<(String, u64)> {
        let views = self.views.read().await;
        views
            .iter()
            .map(|(view_id, journal)| (view_id.clone(), journal.records.len() as u64))
            .collect()
    }

    /// Retained bytes per view, for capacity reporting.
    pub async fn retained_bytes(&self) -> Vec<(String, u64)> {
        let views = self.views.read().await;
        views
            .iter()
            .map(|(view_id, journal)| (view_id.clone(), journal.retained_bytes))
            .collect()
    }
}

pub(crate) fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(max_records: usize, max_age_secs: u64) -> JournalConfig {
        JournalConfig {
            enabled: true,
            max_bytes_per_view: u64::MAX,
            max_records_per_view: max_records,
            max_age: Duration::from_secs(max_age_secs),
        }
    }

    fn frame(body: &str) -> Arc<Bytes> {
        Arc::new(Bytes::from(format!(r#"{{"data":"{body}"}}"#)))
    }

    async fn append(journal: &EventJournal, view: &str, key: &str, body: &str) -> u64 {
        journal
            .append_with(view, key, |_offset| {
                Ok::<_, std::convert::Infallible>(frame(body))
            })
            .await
            .unwrap()
            .0
    }

    async fn cursor_at(journal: &EventJournal, offset: u64) -> Cursor {
        Cursor {
            epoch: journal.epoch().await,
            offset,
        }
    }

    #[tokio::test]
    async fn offsets_are_dense_and_replay_is_ordered_and_exclusive() {
        let journal = EventJournal::new(config(100, 600));
        for index in 0..5 {
            let offset = append(&journal, "Trade/append", &format!("key{index}"), "x").await;
            assert_eq!(offset, index, "offsets are dense and monotonic");
        }

        let from_one = cursor_at(&journal, 1).await;
        let replayed = journal
            .replay_after("Trade/append", Some(&from_one))
            .await
            .unwrap();
        assert_eq!(
            replayed.iter().map(|r| r.offset).collect::<Vec<_>>(),
            [2, 3, 4]
        );

        let all = journal.replay_after("Trade/append", None).await.unwrap();
        assert_eq!(all.len(), 5);

        let caught_up = cursor_at(&journal, 4).await;
        assert!(journal
            .replay_after("Trade/append", Some(&caught_up))
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn the_published_frame_carries_the_offset_the_record_takes() {
        let journal = EventJournal::new(config(100, 600));
        // The frame is built from the offset under the same lock that commits
        // it, so the two can never disagree.
        for expected in 0..3u64 {
            let (offset, payload) = journal
                .append_with("Trade/append", "pool", |offset| {
                    Ok::<_, std::convert::Infallible>(Arc::new(Bytes::from(format!(
                        r#"{{"offset":{offset}}}"#
                    ))))
                })
                .await
                .unwrap();
            assert_eq!(offset, expected);
            assert_eq!(
                String::from_utf8(payload.to_vec()).unwrap(),
                format!(r#"{{"offset":{expected}}}"#)
            );
        }
    }

    #[tokio::test]
    async fn a_cursor_from_a_previous_tape_lifetime_fails_closed() {
        let first = EventJournal::new(config(100, 600));
        for index in 0..10 {
            append(&first, "Trade/append", &format!("key{index}"), "x").await;
        }
        let stale = cursor_at(&first, 5).await;

        // A cold start: new tape, offsets restart at zero.
        let second = EventJournal::new(config(100, 600));
        for index in 0..10 {
            append(&second, "Trade/append", &format!("key{index}"), "x").await;
        }

        // Offset 5 is squarely inside the new window, so without the epoch
        // this would silently replay unrelated events as a continuation.
        let window = second.window("Trade/append").await;
        assert!(stale.offset < window.next && stale.offset >= window.earliest);

        let error = second
            .replay_after("Trade/append", Some(&stale))
            .await
            .expect_err("a cursor from another lifetime is not a valid offset");
        assert!(matches!(error, ReplayError::EpochMismatch(_)));
    }

    #[tokio::test]
    async fn a_restored_tape_keeps_its_epoch_so_cursors_survive_restart() {
        let first = EventJournal::new(config(100, 600));
        for index in 0..10 {
            append(&first, "Trade/append", &format!("key{index}"), "x").await;
        }
        let held = cursor_at(&first, 4).await;
        let dumped = first.dump().await;

        let restored = EventJournal::new(config(100, 600));
        restored.hydrate(dumped).await;

        assert_eq!(restored.epoch().await, held.epoch);
        let replayed = restored
            .replay_after("Trade/append", Some(&held))
            .await
            .expect("a cursor from the restored lifetime is still valid");
        assert_eq!(replayed.len(), 5);
    }

    #[tokio::test]
    async fn a_pre_epoch_snapshot_is_discarded_rather_than_adopted() {
        let journal = EventJournal::new(config(100, 600));
        let legacy = JournalSnapshot {
            epoch: None,
            views: HashMap::from([(
                "Trade/append".to_string(),
                PersistedViewJournal {
                    next_offset: 500,
                    records: Vec::new(),
                    gap_after: None,
                },
            )]),
        };
        journal.hydrate(legacy).await;
        assert!(journal.window("Trade/append").await.is_empty());
        assert_eq!(journal.window("Trade/append").await.next, 0);
    }

    #[tokio::test]
    async fn replay_across_a_known_gap_is_refused() {
        let journal = EventJournal::new(config(100, 600));
        for index in 0..5 {
            append(&journal, "Trade/append", &format!("key{index}"), "x").await;
        }

        // The stream restarted live: events after offset 4 were never retained.
        journal.mark_gap().await;
        for index in 5..8 {
            append(&journal, "Trade/append", &format!("key{index}"), "x").await;
        }

        let window = journal.window("Trade/append").await;
        assert_eq!(window.gap_after, Some(4));

        let before_gap = cursor_at(&journal, 2).await;
        let error = journal
            .replay_after("Trade/append", Some(&before_gap))
            .await
            .expect_err("crossing the hole would look continuous");
        assert!(matches!(error, ReplayError::GapCrossed(_)));

        // After the hole the tape is trustworthy again.
        let after_gap = cursor_at(&journal, 5).await;
        assert_eq!(
            journal
                .replay_after("Trade/append", Some(&after_gap))
                .await
                .unwrap()
                .len(),
            2
        );
    }

    #[tokio::test]
    async fn the_byte_bound_trims_before_the_record_bound() {
        let journal = EventJournal::new(JournalConfig {
            enabled: true,
            // Room for roughly two records once overhead is charged.
            max_bytes_per_view: 2 * (RECORD_OVERHEAD_BYTES + 40),
            max_records_per_view: 10_000,
            max_age: Duration::from_secs(600),
        });
        for index in 0..20 {
            append(&journal, "Trade/append", "k", "payload-body").await;
            let _ = index;
        }

        let window = journal.window("Trade/append").await;
        assert_eq!(window.next, 20);
        assert!(
            window.next - window.earliest <= 3,
            "the byte bound trims well before 10,000 records, got {window:?}"
        );
        assert!(!window.is_empty(), "a window is always left to serve");
    }

    #[tokio::test]
    async fn a_cursor_below_the_window_is_expired_and_reports_the_window() {
        let journal = EventJournal::new(config(3, 600));
        for index in 0..10 {
            append(&journal, "Trade/append", &format!("key{index}"), "x").await;
        }

        let window = journal.window("Trade/append").await;
        assert_eq!(window.earliest, 7);
        assert_eq!(window.next, 10);

        let stale = cursor_at(&journal, 2).await;
        let error = journal
            .replay_after("Trade/append", Some(&stale))
            .await
            .expect_err("a cursor before the window cannot be honoured");
        assert_eq!(error, ReplayError::CursorExpired(window));
    }

    #[tokio::test]
    async fn age_retention_drops_records_the_count_bound_would_keep() {
        let journal = EventJournal::new(config(1_000, 60));
        let now = unix_now();

        journal
            .append_with_at("Trade/append", "old", now - 600, |_| {
                Ok::<_, std::convert::Infallible>(frame("x"))
            })
            .await
            .unwrap();
        journal
            .append_with_at("Trade/append", "fresh", now, |_| {
                Ok::<_, std::convert::Infallible>(frame("x"))
            })
            .await
            .unwrap();

        let window = journal.window("Trade/append").await;
        assert_eq!(window.earliest, 1);
        assert_eq!(window.next, 2);
    }

    #[tokio::test]
    async fn an_unknown_view_replays_nothing_rather_than_failing() {
        let journal = EventJournal::new(config(100, 600));
        let cursor = cursor_at(&journal, 7).await;
        assert!(journal
            .replay_after("Missing/append", Some(&cursor))
            .await
            .unwrap()
            .is_empty());
        assert!(journal.window("Missing/append").await.is_empty());
    }

    #[test]
    fn cursors_round_trip_through_the_wire_form() {
        let cursor = Cursor {
            epoch: JournalEpoch("8a1f-epoch".to_string()),
            offset: 4211,
        };
        let rendered = cursor.to_string();
        assert_eq!(rendered, "8a1f-epoch:4211");
        assert_eq!(Cursor::parse(&rendered), Some(cursor));

        // A bare offset is not a cursor: it carries no lifetime.
        assert_eq!(Cursor::parse("4211"), None);
        assert_eq!(Cursor::parse(":4211"), None);
        assert_eq!(Cursor::parse("epoch:not-a-number"), None);
    }

    #[test]
    fn persisted_frames_round_trip_as_text_not_byte_arrays() {
        let record = PersistedRecord {
            offset: 1,
            key: "pool".to_string(),
            payload: Arc::new(Bytes::from_static(br#"{"data":{"amount":5}}"#)),
            appended_at: 100,
        };
        let json = serde_json::to_string(&record).unwrap();
        assert!(
            json.contains(r#""payload":"{\"data\":{\"amount\":5}}""#),
            "frames persist as text, not a decimal byte array: {json}"
        );

        let restored: PersistedRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.payload, record.payload);
    }
}
