//! Connection registry: tracks open WebSocket connections to HyperStack stacks.
//!
//! Each `connect` MCP call creates a new [`ConnectionEntry`]; each `disconnect`
//! removes one. The registry is a `DashMap` so per-entry locking does not
//! contend across tools running concurrently.
//!
//! Each connection owns one [`SharedStore`] (from `hyperstack-sdk`) into which
//! the ingest task applies every inbound `Frame`. The store is keyed by view
//! internally, so a single connection can hold many subscribed views without
//! needing a second WebSocket. Subscription bookkeeping (which `subscription_id`
//! maps to which `(view, key)` on which connection) lives in
//! [`crate::subscriptions`].

use std::sync::Arc;

use dashmap::DashMap;
use hyperstack_sdk::{
    AuthConfig, ConnectionConfig, ConnectionManager, ConnectionState, HyperStackError, SharedStore,
};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use uuid::Uuid;

/// Channel buffer for inbound frames. The ingest task drains this and applies
/// each frame to the connection's `SharedStore`. 1024 gives ample headroom for
/// burst snapshots without backpressuring the WebSocket reader.
const FRAME_CHANNEL_CAPACITY: usize = 1024;

/// Opaque identifier returned to the MCP client. Hex UUID v4.
pub type ConnectionId = String;

/// One open WebSocket connection to a HyperStack stack.
pub struct ConnectionEntry {
    pub id: ConnectionId,
    pub url: String,
    pub manager: ConnectionManager,
    /// Per-connection cache. All subscribed views on this connection land here,
    /// keyed by view name internally. Shared with query tools via `Arc`.
    #[allow(dead_code)] // Read by query tools landing in step 4.
    pub store: Arc<SharedStore>,
    /// Background task that drains the frame channel and applies each frame
    /// to `store`. Aborted on disconnect.
    ingest_task: JoinHandle<()>,
}

impl ConnectionEntry {
    pub async fn state(&self) -> ConnectionState {
        self.manager.state().await
    }
}

impl Drop for ConnectionEntry {
    fn drop(&mut self) {
        self.ingest_task.abort();
    }
}

/// Server-wide registry of open connections.
#[derive(Clone, Default)]
pub struct ConnectionRegistry {
    inner: Arc<DashMap<ConnectionId, Arc<ConnectionEntry>>>,
}

impl ConnectionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Open a new connection. `api_key` becomes a publishable-key auth token
    /// when present; absent means the stack must be public.
    pub async fn connect(
        &self,
        url: String,
        api_key: Option<String>,
    ) -> Result<ConnectionId, HyperStackError> {
        let mut config = ConnectionConfig::default();
        if let Some(key) = api_key {
            config.auth = Some(AuthConfig::default().with_publishable_key(key));
        }

        let (frame_tx, mut frame_rx) = mpsc::channel(FRAME_CHANNEL_CAPACITY);
        let manager = ConnectionManager::new(url.clone(), config, frame_tx).await?;

        // TODO(HYP-189): SDK's StoreConfig defaults to 10k entries/view.
        // Revisit per-subscription overrides once we have real agent usage data.
        let store = Arc::new(SharedStore::new());
        let store_for_task = store.clone();
        let ingest_task = tokio::spawn(async move {
            while let Some(frame) = frame_rx.recv().await {
                store_for_task.apply_frame(frame).await;
            }
        });

        let id = Uuid::new_v4().simple().to_string();
        let entry = Arc::new(ConnectionEntry {
            id: id.clone(),
            url,
            manager,
            store,
            ingest_task,
        });
        self.inner.insert(id.clone(), entry);
        Ok(id)
    }

    /// Look up a connection by id.
    pub fn get(&self, id: &str) -> Option<Arc<ConnectionEntry>> {
        self.inner.get(id).map(|e| e.clone())
    }

    /// Disconnect and remove a connection. Returns false if id not found.
    pub async fn disconnect(&self, id: &str) -> bool {
        let Some((_, entry)) = self.inner.remove(id) else {
            return false;
        };
        entry.manager.disconnect().await;
        // Drop of the last Arc aborts the drain task via ConnectionEntry::drop.
        true
    }

    /// Snapshot of all open connections for `list_connections`.
    pub fn list(&self) -> Vec<Arc<ConnectionEntry>> {
        self.inner.iter().map(|e| e.value().clone()).collect()
    }
}
