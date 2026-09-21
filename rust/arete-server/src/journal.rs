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
//! Offsets are per view and are not comparable across views.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

const DEFAULT_MAX_RECORDS_PER_VIEW: usize = 10_000;
const DEFAULT_MAX_AGE: Duration = Duration::from_secs(15 * 60);

/// Retention bounds for the event journal. Whichever bound bites first wins.
#[derive(Clone, Debug)]
pub struct JournalConfig {
    /// Master opt-in. Disabled leaves append views on their previous
    /// latest-state delivery.
    pub enabled: bool,
    /// Retained records per view.
    pub max_records_per_view: usize,
    /// Records older than this are dropped even when the count bound is not
    /// reached, so a quiet view does not advertise a stale replay window.
    pub max_age: Duration,
}

impl Default for JournalConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_records_per_view: DEFAULT_MAX_RECORDS_PER_VIEW,
            max_age: DEFAULT_MAX_AGE,
        }
    }
}

impl JournalConfig {
    /// Load from `ARETE_JOURNAL_*`. Stays disabled unless
    /// `ARETE_JOURNAL_ENABLED=true`.
    pub fn from_env() -> anyhow::Result<Self> {
        let mut config = Self::default();
        config.enabled = crate::config::env_bool("ARETE_JOURNAL_ENABLED")?.unwrap_or(false);
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
        if self.enabled && self.max_records_per_view == 0 {
            anyhow::bail!("ARETE_JOURNAL_MAX_RECORDS must be greater than zero");
        }
        if self.enabled && self.max_age.is_zero() {
            anyhow::bail!("ARETE_JOURNAL_MAX_AGE_SECS must be greater than zero");
        }
        Ok(())
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

/// The offsets a view can currently serve.
///
/// `earliest` is the oldest retained offset, so a cursor below it has fallen
/// out of the window and cannot be honoured. `next` is the offset the next
/// append will take, so a consumer at `next` is fully caught up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayWindow {
    pub earliest: u64,
    pub next: u64,
}

impl ReplayWindow {
    pub fn is_empty(&self) -> bool {
        self.earliest >= self.next
    }
}

/// Why a replay could not be served.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplayError {
    /// The cursor is older than the oldest retained record. Carries the
    /// window so the consumer can decide where to restart.
    CursorExpired(ReplayWindow),
    /// The cursor is one this view has never issued. Refused rather than
    /// treated as caught up, which would suppress delivery until the view's
    /// offsets reached it.
    CursorBeyondWindow(ReplayWindow),
}

/// Durable form of one view's retained tape.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PersistedRecord {
    pub offset: u64,
    pub key: String,
    /// The published frame bytes, verbatim.
    pub payload: Vec<u8>,
    pub appended_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PersistedViewJournal {
    pub next_offset: u64,
    pub records: Vec<PersistedRecord>,
}

/// The retained tape for every view, carried in the snapshot payload.
///
/// ponytail: this rides inside the existing whole-state snapshot rather than
/// its own segment files. That buys the snapshot's consistency cut for free —
/// the tape and the entity cache are dumped under one barrier, so a restore
/// can never leave the cache ahead of the tape — at the cost of rewriting the
/// retained tape on every snapshot. Retention is bounded, so the cost is
/// bounded too; if a deployment raises `ARETE_JOURNAL_MAX_RECORDS` far enough
/// that whole-blob rewrites hurt, the upgrade path is append-only segment
/// objects alongside the snapshot.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct JournalSnapshot {
    pub views: HashMap<String, PersistedViewJournal>,
}

#[derive(Debug, Default)]
struct ViewJournal {
    /// Offset the next append takes. Never rewound, including after pruning.
    next_offset: u64,
    /// Retained records, oldest first.
    records: VecDeque<JournalRecord>,
}

impl ViewJournal {
    fn window(&self) -> ReplayWindow {
        ReplayWindow {
            earliest: self
                .records
                .front()
                .map(|record| record.offset)
                .unwrap_or(self.next_offset),
            next: self.next_offset,
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
            self.records.pop_front();
        }
        while self.records.len() > config.max_records_per_view {
            self.records.pop_front();
        }
    }
}

/// Per-view append-only event log with bounded retention.
#[derive(Debug)]
pub struct EventJournal {
    views: RwLock<HashMap<String, ViewJournal>>,
    config: JournalConfig,
}

impl EventJournal {
    pub fn new(config: JournalConfig) -> Self {
        Self {
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

    /// Append one published event and return the offset it was given.
    ///
    /// `payload` must be the bytes that were published for this event; replay
    /// re-sends them unchanged.
    pub async fn append(&self, view_id: &str, key: &str, payload: Arc<Bytes>) -> u64 {
        self.append_at(view_id, key, payload, unix_now()).await
    }

    async fn append_at(&self, view_id: &str, key: &str, payload: Arc<Bytes>, now: i64) -> u64 {
        let mut views = self.views.write().await;
        let journal = views.entry(view_id.to_string()).or_default();
        let offset = journal.next_offset;
        journal.next_offset += 1;
        journal.records.push_back(JournalRecord {
            offset,
            key: key.to_string(),
            payload,
            appended_at: now,
        });
        journal.prune(&self.config, now);
        offset
    }

    /// The offset the next append to this view will take.
    ///
    /// Reserved before the frame is serialized so the offset can be embedded
    /// in the published payload. The projector applies one mutation at a time,
    /// so the reserved offset is the one `append` then assigns.
    pub async fn next_offset(&self, view_id: &str) -> u64 {
        let views = self.views.read().await;
        views
            .get(view_id)
            .map(|journal| journal.next_offset)
            .unwrap_or(0)
    }

    /// Offsets this view can currently serve.
    ///
    /// Prunes first: the age bound has to hold for a view that has gone
    /// quiet, otherwise expired records stay advertised and replayable.
    pub async fn window(&self, view_id: &str) -> ReplayWindow {
        let mut views = self.views.write().await;
        let now = unix_now();
        match views.get_mut(view_id) {
            Some(journal) => {
                journal.prune(&self.config, now);
                journal.window()
            }
            None => ReplayWindow {
                earliest: 0,
                next: 0,
            },
        }
    }

    /// Every retained record strictly after `cursor`, in offset order.
    ///
    /// `cursor` is the last offset the consumer already has, so a consumer
    /// that has seen nothing passes `None`.
    pub async fn replay_after(
        &self,
        view_id: &str,
        cursor: Option<u64>,
    ) -> Result<Vec<JournalRecord>, ReplayError> {
        let mut views = self.views.write().await;
        let now = unix_now();
        let Some(journal) = views.get_mut(view_id) else {
            return Ok(Vec::new());
        };
        journal.prune(&self.config, now);
        let window = journal.window();

        if let Some(cursor) = cursor {
            // Below the window: the records are gone.
            if cursor.saturating_add(1) < window.earliest {
                return Err(ReplayError::CursorExpired(window));
            }
            // Above it: a cursor the view has never issued. Accepting it would
            // silently suppress every later record until the offsets caught
            // up, so refuse it rather than appear to work.
            if cursor >= window.next && window.next > 0 {
                return Err(ReplayError::CursorBeyondWindow(window));
            }
            if window.next == 0 {
                return Err(ReplayError::CursorBeyondWindow(window));
            }
        }

        let first_wanted = cursor
            .map(|cursor| cursor.saturating_add(1))
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
    /// Records are cloned; `payload` becomes owned bytes so serialization
    /// happens outside the journal lock.
    pub async fn dump(&self) -> JournalSnapshot {
        let views = self.views.read().await;
        JournalSnapshot {
            views: views
                .iter()
                .map(|(view_id, journal)| {
                    (
                        view_id.clone(),
                        PersistedViewJournal {
                            next_offset: journal.next_offset,
                            records: journal
                                .records
                                .iter()
                                .map(|record| PersistedRecord {
                                    offset: record.offset,
                                    key: record.key.clone(),
                                    payload: record.payload.to_vec(),
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
    /// Retention is re-applied on load: a snapshot restored after a long
    /// outage must not advertise records the age bound has already retired.
    pub async fn hydrate(&self, snapshot: JournalSnapshot) {
        if !self.config.enabled {
            return;
        }
        let now = unix_now();
        let mut views = self.views.write().await;
        for (view_id, persisted) in snapshot.views {
            let mut journal = ViewJournal {
                next_offset: persisted.next_offset,
                records: persisted
                    .records
                    .into_iter()
                    .map(|record| JournalRecord {
                        offset: record.offset,
                        key: record.key,
                        payload: Arc::new(Bytes::from(record.payload)),
                        appended_at: record.appended_at,
                    })
                    .collect(),
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

    /// Drop everything retained for a view. Used when a view is torn down.
    pub async fn clear(&self, view_id: &str) {
        self.views.write().await.remove(view_id);
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
            max_records_per_view: max_records,
            max_age: Duration::from_secs(max_age_secs),
        }
    }

    fn payload(body: &str) -> Arc<Bytes> {
        Arc::new(Bytes::copy_from_slice(body.as_bytes()))
    }

    #[tokio::test]
    async fn offsets_are_dense_and_replay_is_ordered_and_exclusive() {
        let journal = EventJournal::new(config(100, 600));
        for index in 0..5 {
            let offset = journal
                .append("Trade/append", &format!("key{index}"), payload("frame"))
                .await;
            assert_eq!(offset, index, "offsets are dense and monotonic");
        }

        // A cursor is the last offset already seen, so replay starts after it.
        let replayed = journal.replay_after("Trade/append", Some(1)).await.unwrap();
        assert_eq!(
            replayed.iter().map(|r| r.offset).collect::<Vec<_>>(),
            [2, 3, 4]
        );

        // No cursor replays the whole retained window.
        let all = journal.replay_after("Trade/append", None).await.unwrap();
        assert_eq!(all.len(), 5);

        // Caught up is empty, not an error.
        let caught_up = journal.replay_after("Trade/append", Some(4)).await.unwrap();
        assert!(caught_up.is_empty());
    }

    #[tokio::test]
    async fn replay_survives_more_records_than_the_entity_cache_holds() {
        // The entity cache caps at 500 per view; a journal consumer must be
        // able to resume across more events than that.
        let journal = EventJournal::new(config(5_000, 600));
        for index in 0..1_200u64 {
            journal
                .append("Trade/append", &format!("key{index}"), payload("frame"))
                .await;
        }

        let replayed = journal.replay_after("Trade/append", Some(499)).await.unwrap();
        assert_eq!(replayed.len(), 700, "every event after the cursor replays");
        assert_eq!(replayed.first().unwrap().offset, 500);
        assert_eq!(replayed.last().unwrap().offset, 1_199);
    }

    #[tokio::test]
    async fn a_cursor_below_the_window_is_expired_and_reports_the_window() {
        let journal = EventJournal::new(config(3, 600));
        for index in 0..10 {
            journal
                .append("Trade/append", &format!("key{index}"), payload("frame"))
                .await;
        }

        let window = journal.window("Trade/append").await;
        assert_eq!(
            window,
            ReplayWindow {
                earliest: 7,
                next: 10
            },
            "count retention drops the oldest records but never rewinds offsets"
        );

        let error = journal
            .replay_after("Trade/append", Some(2))
            .await
            .expect_err("a cursor before the window cannot be honoured");
        assert_eq!(error, ReplayError::CursorExpired(window));

        // The boundary cursor is still serviceable: offset 6 means "I have 6",
        // and 7 is still retained.
        let boundary = journal.replay_after("Trade/append", Some(6)).await.unwrap();
        assert_eq!(boundary.first().unwrap().offset, 7);
    }

    #[tokio::test]
    async fn age_retention_drops_records_the_count_bound_would_keep() {
        let journal = EventJournal::new(config(1_000, 60));
        let now = unix_now();

        journal
            .append_at("Trade/append", "old", payload("frame"), now - 600)
            .await;
        journal
            .append_at("Trade/append", "fresh", payload("frame"), now)
            .await;

        let window = journal.window("Trade/append").await;
        assert_eq!(
            window,
            ReplayWindow {
                earliest: 1,
                next: 2
            },
            "the aged record is gone even though the count bound was not reached"
        );
    }

    #[tokio::test]
    async fn an_unknown_view_replays_nothing_rather_than_failing() {
        let journal = EventJournal::new(config(100, 600));
        assert!(journal
            .replay_after("Missing/append", Some(7))
            .await
            .unwrap()
            .is_empty());
        assert!(journal.window("Missing/append").await.is_empty());
    }
}
