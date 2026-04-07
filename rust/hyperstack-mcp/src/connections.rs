//! Connection registry: tracks open WebSocket connections to HyperStack stacks.
//!
//! Each `connect` MCP call creates a new [`ConnectionEntry`]; each `disconnect`
//! removes one. The registry is a `DashMap` so per-entry locking does not
//! contend across tools running concurrently.
//!
//! Subscriptions are intentionally **not** modeled here yet — that lands in
//! step 3 of HYP-189. For now each entry only owns its `ConnectionManager`
//! plus the `frame_rx` task handle so that v1 can verify connect/disconnect
//! end-to-end against a real stack before any data flows.

use std::sync::Arc;

use dashmap::DashMap;
use hyperstack_sdk::{
    AuthConfig, ConnectionConfig, ConnectionManager, ConnectionState, Frame, HyperStackError,
};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use uuid::Uuid;

/// Channel buffer for inbound frames before any subscription consumer is wired up.
/// In step 3 the ingest task will route these into per-subscription stores.
const FRAME_CHANNEL_CAPACITY: usize = 1024;

/// Opaque identifier returned to the MCP client. Hex UUID v4.
pub type ConnectionId = String;

/// One open WebSocket connection to a HyperStack stack.
pub struct ConnectionEntry {
    pub id: ConnectionId,
    pub url: String,
    pub manager: ConnectionManager,
    /// Background task that drains the frame channel. Until subscriptions
    /// land in step 3 this just discards frames so the channel never fills.
    /// Aborted on disconnect.
    drain_task: JoinHandle<()>,
}

impl ConnectionEntry {
    pub async fn state(&self) -> ConnectionState {
        self.manager.state().await
    }
}

impl Drop for ConnectionEntry {
    fn drop(&mut self) {
        self.drain_task.abort();
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

        let (frame_tx, mut frame_rx) = mpsc::channel::<Frame>(FRAME_CHANNEL_CAPACITY);
        let manager = ConnectionManager::new(url.clone(), config, frame_tx).await?;

        // TODO(HYP-189 step 3): replace this drain with the per-subscription
        // ingest task that routes frames into SharedStores.
        let drain_task = tokio::spawn(async move {
            while frame_rx.recv().await.is_some() {
                // discard
            }
        });

        let id = Uuid::new_v4().simple().to_string();
        let entry = Arc::new(ConnectionEntry {
            id: id.clone(),
            url,
            manager,
            drain_task,
        });
        self.inner.insert(id.clone(), entry);
        Ok(id)
    }

    /// Look up a connection by id. Used by subscribe/query tools in step 3+.
    #[allow(dead_code)]
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
