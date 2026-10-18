//! The dev event feed: a bounded in-memory buffer of numbered events with a
//! broadcast for live readers.
//!
//! Every event has a sequence number (`seq`, from 1) unique within one
//! `a4 dev up` run, which a random `boot` id names. The SSE id of an event is
//! `<boot>:<seq>`, so a reader that reconnects with `Last-Event-ID` resumes
//! after the last event it saw, and one that reconnects to a new run (a new
//! boot) starts over. Readers that fell behind the buffer get a
//! `stream.reset` event first: they should reload `GET /dev/v1/protocol`.
//!
//! Ephemeral events (`chain.clock` ticks) go to live readers only: they take
//! no sequence number and never fill the buffer.

use std::collections::VecDeque;
use std::sync::Mutex;

use serde::Serialize;
use serde_json::{json, Value};
use tokio::sync::broadcast;

/// Events kept for replay.
pub const BUFFER_CAPACITY: usize = 4096;
/// The page size of a JSON read.
pub const DEFAULT_PAGE: usize = 200;
pub const MAX_PAGE: usize = 1000;

/// One event of the feed.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DevEvent {
    /// 0 for ephemeral events.
    pub seq: u64,
    #[serde(rename = "type")]
    pub kind: String,
    /// RFC 3339, when `a4 dev` published it.
    pub at: String,
    pub data: Value,
}

impl DevEvent {
    pub fn ephemeral(&self) -> bool {
        self.seq == 0
    }
}

/// Where a reader is: the run and the last sequence number it saw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cursor {
    pub boot: Option<String>,
    pub seq: u64,
}

impl Cursor {
    /// `<boot>:<seq>` or a bare `<seq>`.
    pub fn parse(raw: &str) -> Option<Self> {
        let raw = raw.trim();
        match raw.rsplit_once(':') {
            Some((boot, seq)) if !boot.is_empty() => Some(Self {
                boot: Some(boot.to_string()),
                seq: seq.parse().ok()?,
            }),
            Some(_) => None,
            None => Some(Self {
                boot: None,
                seq: raw.parse().ok()?,
            }),
        }
    }
}

/// What a reader gets back for a cursor.
#[derive(Debug, Clone, PartialEq)]
pub struct Replay {
    pub events: Vec<DevEvent>,
    /// Events after the cursor were dropped from the buffer, or the cursor
    /// is from another run: the reader should reload its state.
    pub reset: bool,
    /// More events are buffered after the last one returned.
    pub more: bool,
    /// The newest sequence number when the replay was taken (under the same
    /// lock), so a reader with no events can resume from here without
    /// skipping one published since.
    pub head: u64,
}

struct Inner {
    next: u64,
    buffer: VecDeque<DevEvent>,
}

pub struct EventHub {
    boot: String,
    capacity: usize,
    inner: Mutex<Inner>,
    live: broadcast::Sender<DevEvent>,
}

impl EventHub {
    pub fn new() -> Self {
        Self::with_capacity(BUFFER_CAPACITY)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        let mut bytes = [0u8; 6];
        let _ = getrandom::getrandom(&mut bytes);
        let boot: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
        let (live, _) = broadcast::channel(1024);
        Self {
            boot,
            capacity: capacity.max(1),
            inner: Mutex::new(Inner {
                next: 1,
                buffer: VecDeque::new(),
            }),
            live,
        }
    }

    pub fn boot(&self) -> &str {
        &self.boot
    }

    /// The SSE id of `event`.
    pub fn event_id(&self, event: &DevEvent) -> String {
        format!("{}:{}", self.boot, event.seq)
    }

    /// The cursor after the newest event.
    pub fn head(&self) -> String {
        let inner = self.lock();
        self.cursor_at(inner.next - 1)
    }

    /// The cursor after sequence number `seq` of this run.
    pub fn cursor_at(&self, seq: u64) -> String {
        format!("{}:{}", self.boot, seq)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Number, buffer and broadcast an event.
    pub fn publish(&self, kind: &str, data: Value) -> DevEvent {
        let mut inner = self.lock();
        let event = DevEvent {
            seq: inner.next,
            kind: kind.to_string(),
            at: now(),
            data,
        };
        inner.next += 1;
        inner.buffer.push_back(event.clone());
        while inner.buffer.len() > self.capacity {
            inner.buffer.pop_front();
        }
        // Sent under the lock, so live readers see sequence order.
        let _ = self.live.send(event.clone());
        event
    }

    /// Broadcast an event to live readers only.
    pub fn publish_ephemeral(&self, kind: &str, data: Value) {
        let _ = self.live.send(DevEvent {
            seq: 0,
            kind: kind.to_string(),
            at: now(),
            data,
        });
    }

    /// The buffered events after `cursor`, at most `limit`.
    pub fn since(&self, cursor: Option<&Cursor>, limit: usize) -> Replay {
        let inner = self.lock();
        self.since_locked(&inner, cursor, limit)
    }

    fn since_locked(&self, inner: &Inner, cursor: Option<&Cursor>, limit: usize) -> Replay {
        let oldest = inner.buffer.front().map(|event| event.seq);
        let (after, mut reset) = match cursor {
            None => (0, false),
            Some(cursor) if cursor.boot.as_deref().is_some_and(|boot| boot != self.boot) => {
                (0, true)
            }
            // A cursor from the future (another run without a boot id).
            Some(cursor) if cursor.seq >= inner.next => (0, true),
            Some(cursor) => (cursor.seq, false),
        };
        if let Some(oldest) = oldest {
            if after + 1 < oldest && cursor.is_some() {
                reset = true;
            }
        }
        let mut events: Vec<DevEvent> = inner
            .buffer
            .iter()
            .filter(|event| event.seq > after)
            .take(limit.saturating_add(1))
            .cloned()
            .collect();
        let more = events.len() > limit;
        events.truncate(limit);
        Replay {
            events,
            reset,
            more,
            head: inner.next - 1,
        }
    }

    /// The replay after `cursor` and a receiver for everything published
    /// after it, taken atomically.
    pub fn subscribe(&self, cursor: Option<&Cursor>) -> (Replay, broadcast::Receiver<DevEvent>) {
        let inner = self.lock();
        let replay = self.since_locked(&inner, cursor, usize::MAX - 1);
        (replay, self.live.subscribe())
    }

    /// A receiver for live events only.
    pub fn listen(&self) -> broadcast::Receiver<DevEvent> {
        self.live.subscribe()
    }
}

impl Default for EventHub {
    fn default() -> Self {
        Self::new()
    }
}

pub fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// The `stream.reset` event a reader gets when its cursor is gone.
pub fn reset_event(hub: &EventHub) -> DevEvent {
    DevEvent {
        seq: 0,
        kind: "stream.reset".into(),
        at: now(),
        data: json!({ "boot": hub.boot(), "reason": "The events after your cursor are no longer buffered; reload the protocol and instances." }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_buffers_and_replays_after_a_cursor() {
        let hub = EventHub::with_capacity(3);
        for index in 0..5 {
            hub.publish("step.prepared", json!({ "index": index }));
        }
        // Only the last three are kept.
        let all = hub.since(None, 100);
        assert_eq!(
            all.events.iter().map(|e| e.seq).collect::<Vec<_>>(),
            vec![3, 4, 5]
        );
        assert!(!all.reset);

        let cursor = Cursor::parse(&format!("{}:4", hub.boot())).unwrap();
        let after = hub.since(Some(&cursor), 100);
        assert_eq!(after.events.len(), 1);
        assert_eq!(after.events[0].seq, 5);
        assert!(!after.reset);

        // Seq 1 is gone: the reader missed seq 2 and must reload.
        let stale = hub.since(Some(&Cursor::parse("1").unwrap()), 100);
        assert!(stale.reset);
        assert_eq!(stale.events.len(), 3);

        // Another run's cursor starts over.
        let other = hub.since(Some(&Cursor::parse("abc:4").unwrap()), 100);
        assert!(other.reset);
        assert_eq!(other.events.len(), 3);

        // Paging.
        let page = hub.since(None, 2);
        assert_eq!(page.events.len(), 2);
        assert!(page.more);
        assert_eq!(hub.head(), format!("{}:5", hub.boot()));
    }

    #[test]
    fn ephemeral_events_reach_live_readers_only() {
        let hub = EventHub::new();
        let (replay, mut receiver) = hub.subscribe(None);
        assert!(replay.events.is_empty());
        hub.publish_ephemeral("chain.clock", json!({ "slot": 1 }));
        hub.publish("build.started", json!({}));
        let first = receiver.try_recv().unwrap();
        assert!(first.ephemeral());
        assert_eq!(receiver.try_recv().unwrap().seq, 1);
        assert_eq!(hub.since(None, 10).events.len(), 1);
    }

    #[test]
    fn parses_cursors() {
        assert_eq!(
            Cursor::parse("b00t:12"),
            Some(Cursor {
                boot: Some("b00t".into()),
                seq: 12
            })
        );
        assert_eq!(Cursor::parse("7").unwrap().seq, 7);
        assert!(Cursor::parse(":7").is_none());
        assert!(Cursor::parse("x:y").is_none());
        assert!(Cursor::parse("").is_none());
    }
}
