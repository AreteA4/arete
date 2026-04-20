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

#[derive(Clone)]
#[allow(clippy::type_complexity)]
pub struct BusManager {
    state_buses: Arc<RwLock<HashMap<(String, String), watch::Sender<Arc<Bytes>>>>>,
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
    ) -> watch::Receiver<Arc<Bytes>> {
        let mut buses = self.state_buses.write().await;
        let entry = (view_id.to_string(), key.to_string());

        let tx = buses
            .entry(entry)
            .or_insert_with(|| {
                let empty = Arc::new(Bytes::new());
                watch::channel(empty).0
            })
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
            let _ = tx.send(frame);
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
