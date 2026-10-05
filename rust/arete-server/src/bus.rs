use bytes::Bytes;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, watch, RwLock};

/// Message sent through the event bus
#[derive(Debug, Clone)]
pub struct BusMessage {
    pub key: String,
    pub entity: String,
    pub payload: Arc<Bytes>,
}

/// The latest frame published to one state key, and how many frames the key
/// has published in total.
///
/// A state bus keeps only the latest frame, so frames published faster than a
/// subscriber reads them are overwritten. The count lets the subscriber tell:
/// if it moved by more than one since its last read, frames were skipped.
#[derive(Debug, Clone, Default)]
pub struct StateUpdate {
    pub published: u64,
    /// `published` as of the latest frame that replaced the entity instead
    /// of patching it: an `upsert` or a `delete`. If it is past a
    /// subscriber's last read, a client's copy may hold fields the entity no
    /// longer has, which no patch removes. Zero until one is published.
    pub replaced: u64,
    pub payload: Arc<Bytes>,
}

/// Whether a state frame replaces its entity (`upsert`, `delete`) rather than
/// patching it. A frame that does not parse is taken for a patch.
fn replaces_entity(frame: &[u8]) -> bool {
    #[derive(serde::Deserialize)]
    struct Frame<'a> {
        #[serde(borrow)]
        op: std::borrow::Cow<'a, str>,
    }
    serde_json::from_slice::<Frame<'_>>(frame)
        .is_ok_and(|frame| matches!(frame.op.as_ref(), "upsert" | "delete"))
}

#[derive(Clone)]
#[allow(clippy::type_complexity)]
pub struct BusManager {
    state_buses: Arc<RwLock<HashMap<(String, String), watch::Sender<StateUpdate>>>>,
    list_buses: Arc<RwLock<HashMap<String, broadcast::Sender<Arc<BusMessage>>>>>,
    broadcast_capacity: usize,
}

impl BusManager {
    pub fn new() -> Self {
        Self::with_capacity(1000)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            state_buses: Arc::new(RwLock::new(HashMap::new())),
            list_buses: Arc::new(RwLock::new(HashMap::new())),
            broadcast_capacity: capacity,
        }
    }

    /// Get or create a state bus (latest-value semantics)
    /// Each (view_id, key) pair gets its own watch channel
    pub async fn get_or_create_state_bus(
        &self,
        view_id: &str,
        key: &str,
    ) -> watch::Receiver<StateUpdate> {
        let mut buses = self.state_buses.write().await;
        let entry = (view_id.to_string(), key.to_string());

        let tx = buses
            .entry(entry)
            .or_insert_with(|| watch::channel(StateUpdate::default()).0)
            .clone();

        tx.subscribe()
    }

    pub async fn get_or_create_list_bus(
        &self,
        view_id: &str,
    ) -> broadcast::Receiver<Arc<BusMessage>> {
        let mut buses = self.list_buses.write().await;

        let tx = buses
            .entry(view_id.to_string())
            .or_insert_with(|| broadcast::channel(self.broadcast_capacity).0)
            .clone();

        tx.subscribe()
    }

    /// Publish to a state bus (latest-value)
    pub async fn publish_state(&self, view_id: &str, key: &str, frame: Arc<Bytes>) {
        let buses = self.state_buses.read().await;
        if let Some(tx) = buses.get(&(view_id.to_string(), key.to_string())) {
            let replaces = replaces_entity(&frame);
            tx.send_modify(|update| {
                update.published += 1;
                if replaces {
                    update.replaced = update.published;
                }
                update.payload = frame;
            });
        }
    }

    pub async fn publish_list(&self, view_id: &str, message: Arc<BusMessage>) {
        let buses = self.list_buses.read().await;
        if let Some(tx) = buses.get(view_id) {
            let _ = tx.send(message);
        }
    }

    pub async fn cleanup_stale_state_buses(&self) -> usize {
        let mut buses = self.state_buses.write().await;
        let before = buses.len();

        buses.retain(|_, tx| tx.receiver_count() > 0);

        before - buses.len()
    }

    pub async fn cleanup_stale_list_buses(&self) -> usize {
        let mut buses = self.list_buses.write().await;
        let before = buses.len();

        buses.retain(|_, tx| tx.receiver_count() > 0);

        before - buses.len()
    }

    pub async fn bus_counts(&self) -> (usize, usize) {
        let state_count = self.state_buses.read().await.len();
        let list_count = self.list_buses.read().await.len();
        (state_count, list_count)
    }
}

impl Default for BusManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(op: &str) -> Arc<Bytes> {
        Arc::new(Bytes::from(format!(
            r#"{{"mode":"state","entity":"Round/state","op":"{op}","key":"7","data":{{}}}}"#
        )))
    }

    /// The bus remembers where the latest whole replacement (an upsert or a
    /// delete) sits among the frames it published, so a subscriber that
    /// skipped frames can tell whether a patch will do.
    #[tokio::test]
    async fn a_state_bus_records_its_latest_replacing_frame() {
        let bus = BusManager::new();
        let receiver = bus.get_or_create_state_bus("Round/state", "7").await;
        bus.publish_state("Round/state", "7", frame("patch")).await;
        assert_eq!(receiver.borrow().replaced, 0);
        bus.publish_state("Round/state", "7", frame("upsert")).await;
        bus.publish_state("Round/state", "7", frame("patch")).await;
        assert_eq!(receiver.borrow().replaced, 2);
        bus.publish_state("Round/state", "7", frame("delete")).await;
        assert_eq!(receiver.borrow().replaced, 4);
        assert_eq!(receiver.borrow().published, 4);
        bus.publish_state(
            "Round/state",
            "7",
            Arc::new(Bytes::from_static(b"not json")),
        )
        .await;
        assert_eq!(receiver.borrow().replaced, 4, "unparsed frames are patches");
    }
}
