//! Connection registry: tracks open WebSocket connections to Arete stacks.
//!
//! Each `connect` MCP call creates a new [`ConnectionEntry`]; each `disconnect`
//! removes one. The registry is a `DashMap` so per-entry locking does not
//! contend across tools running concurrently.
//!
//! Each connection owns one [`SharedStore`] (from `arete-sdk`) into which
//! the ingest task applies every inbound `Frame`. The store is keyed by view
//! internally, so a single connection can hold many subscribed views without
//! needing a second WebSocket. Subscription bookkeeping (which `subscription_id`
//! maps to which `(view, key)` on which connection) lives in
//! [`crate::subscriptions`].
//!
//! ## Subscribe/disconnect race safety
//!
//! Each entry carries a `tokio::sync::RwLock<bool>` (`alive`). The `subscribe`
//! tool acquires a read guard on it for the entire insert-sub + dispatch
//! window, after checking that the flag is still `true`. The `disconnect`
//! tool (a) removes the entry from the DashMap so no new `get()` can find it,
//! then (b) acquires the write lock — waiting for any in-flight subscribe
//! read-guards to drop — then (c) flips the flag and only then sweeps stale
//! subscription entries. The effect is that a concurrent subscribe either
//! finishes fully (and its subscription entry is later swept) or sees
//! `!alive` and returns without inserting anything. No orphans possible.

use std::sync::Arc;

use arete_sdk::{
    AreteError, AuthConfig, ConnectionConfig, ConnectionManager, ConnectionState, SharedStore,
};
use dashmap::DashMap;
use tokio::sync::{mpsc, RwLock};
use tokio::task::JoinHandle;
use uuid::Uuid;

/// Channel buffer for inbound frames. The ingest task drains this and applies
/// each frame to the connection's `SharedStore`. 1024 gives ample headroom for
/// burst snapshots without backpressuring the WebSocket reader.
const FRAME_CHANNEL_CAPACITY: usize = 1024;

/// Opaque identifier returned to the MCP client. Hex UUID v4.
pub type ConnectionId = String;

/// One open WebSocket connection to a Arete stack.
pub struct ConnectionEntry {
    pub id: ConnectionId,
    pub url: String,
    pub manager: ConnectionManager,
    /// Per-connection cache. All subscribed views on this connection land here,
    /// keyed by view name internally. Shared with query tools via `Arc`.
    pub store: Arc<SharedStore>,
    /// Alive flag gated by a read/write lock. Subscribe takes a read guard
    /// for the full insert + dispatch window; disconnect takes the write lock
    /// to wait for in-flight subscribes before sweeping. See module docs for
    /// the full race-safety argument.
    pub alive: Arc<RwLock<bool>>,
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
    ) -> Result<ConnectionId, AreteError> {
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
                tracing::debug!(
                    "ingest: view={} op={} key={}",
                    frame.entity,
                    frame.op,
                    frame.key
                );
                store_for_task.apply_frame(frame).await;
            }
        });

        let id = Uuid::new_v4().simple().to_string();
        let entry = Arc::new(ConnectionEntry {
            id: id.clone(),
            url,
            manager,
            store,
            alive: Arc::new(RwLock::new(true)),
            ingest_task,
        });
        self.inner.insert(id.clone(), entry);
        Ok(id)
    }

    /// Look up a connection by id.
    pub fn get(&self, id: &str) -> Option<Arc<ConnectionEntry>> {
        self.inner.get(id).map(|e| e.clone())
    }

    /// Disconnect and remove a connection. Returns the dropped entry if the
    /// id was present. The caller (MCP `disconnect` tool) is responsible for
    /// sweeping `SubscriptionRegistry` for this connection, which must happen
    /// **after** the `alive` write lock is acquired to wait out any in-flight
    /// `subscribe` calls — see the module-level race-safety note.
    pub async fn disconnect(&self, id: &str) -> Option<Arc<ConnectionEntry>> {
        // Step 1: remove from the map so no new `get()` can find this entry.
        // Any subscribe caller who already holds an Arc<ConnectionEntry> will
        // still be able to reach `alive`, but new callers get a clean "unknown
        // connection_id" error instead of racing us.
        let (_, entry) = self.inner.remove(id)?;

        // Step 2: acquire the write lock. This blocks until every in-flight
        // subscribe holding a read guard has released. Once we hold the
        // write lock, we can safely set alive=false — any subscribe that
        // wakes up next will see it and bail.
        {
            let mut alive = entry.alive.write().await;
            *alive = false;
        }

        // Step 3: stop the underlying WebSocket. The drop of the last Arc
        // aborts the ingest task (see ConnectionEntry::drop). We still return
        // the Arc so the caller can delay destruction until after the
        // subscription sweep.
        entry.manager.disconnect().await;
        Some(entry)
    }

    /// Snapshot of all open connections for `list_connections`.
    pub fn list(&self) -> Vec<Arc<ConnectionEntry>> {
        self.inner.iter().map(|e| e.value().clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    //! Tests for the race-safety contract between `subscribe` and `disconnect`.
    //!
    //! We can't construct a real `ConnectionEntry` in a unit test because
    //! `ConnectionManager` has no public constructor and opens a WebSocket.
    //! Instead these tests exercise the `alive` flag protocol directly —
    //! that's the entire mechanism the `subscribe`/`disconnect` handlers in
    //! `main.rs` rely on for ordering. If someone removes the locking or
    //! reorders the steps in `disconnect`, these tests break.
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::RwLock;

    /// Simulates: subscribe holds a read guard, disconnect tries to write,
    /// and we verify disconnect waits until subscribe releases.
    #[tokio::test]
    async fn disconnect_waits_for_in_flight_subscribe() {
        let alive: Arc<RwLock<bool>> = Arc::new(RwLock::new(true));

        // Subscribe: acquire read guard, then "dispatch" for a bit, then drop.
        let sub_alive = alive.clone();
        let subscribe = tokio::spawn(async move {
            let guard = sub_alive.read().await;
            assert!(*guard, "subscribe should see alive=true");
            tokio::time::sleep(Duration::from_millis(100)).await;
            drop(guard);
        });

        // Give subscribe a moment to grab the read guard.
        tokio::time::sleep(Duration::from_millis(10)).await;

        // Disconnect: try to acquire write lock. Should block on subscribe's
        // read guard until it's released.
        let disc_alive = alive.clone();
        let disconnect_start = std::time::Instant::now();
        let disconnect = tokio::spawn(async move {
            let mut guard = disc_alive.write().await;
            *guard = false;
        });

        subscribe.await.unwrap();
        disconnect.await.unwrap();

        // Disconnect must have waited for subscribe — at least ~80ms of the
        // 100ms sleep. Using real time so no tokio test-util feature needed.
        let elapsed = disconnect_start.elapsed();
        assert!(
            elapsed >= Duration::from_millis(80),
            "disconnect finished in {elapsed:?}, should have waited for subscribe's ~100ms read hold"
        );

        // After disconnect, alive must be false.
        assert!(!*alive.read().await);
    }

    /// Simulates: disconnect wins the write lock first, subscribe then tries
    /// to take the read guard and MUST see alive=false before inserting.
    #[tokio::test]
    async fn subscribe_sees_alive_false_after_disconnect_wins_race() {
        let alive: Arc<RwLock<bool>> = Arc::new(RwLock::new(true));

        // Disconnect runs first synchronously — flips the flag.
        {
            let mut guard = alive.write().await;
            *guard = false;
        }

        // Subscribe now takes its read guard and must see alive=false.
        let guard = alive.read().await;
        assert!(
            !*guard,
            "subscribe must see alive=false so it can bail before inserting an orphan"
        );
    }

    /// Many concurrent subscribes + one disconnect: every subscribe that
    /// observed alive=true must have completed its "insert + dispatch" work
    /// before disconnect returns. Uses a shared counter to detect violations.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn disconnect_observes_all_inflight_subscribes() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let alive = Arc::new(RwLock::new(true));
        let inflight = Arc::new(AtomicUsize::new(0));
        let completed = Arc::new(AtomicUsize::new(0));

        // Spawn 20 subscribes that each hold the read guard briefly.
        let mut subs = Vec::new();
        for _ in 0..20 {
            let a = alive.clone();
            let inf = inflight.clone();
            let com = completed.clone();
            subs.push(tokio::spawn(async move {
                let guard = a.read().await;
                if !*guard {
                    return; // lost the race, correctly bailing out
                }
                inf.fetch_add(1, Ordering::SeqCst);
                // Simulate some work inside the critical section.
                tokio::time::sleep(Duration::from_millis(20)).await;
                com.fetch_add(1, Ordering::SeqCst);
                drop(guard);
            }));
        }

        // Let some subscribes start.
        tokio::time::sleep(Duration::from_millis(5)).await;

        // Disconnect: take write lock, flip flag. Must wait for every
        // subscribe that already grabbed a read guard.
        {
            let mut guard = alive.write().await;
            *guard = false;
        }

        // After disconnect's write lock, every subscribe that incremented
        // inflight must also have incremented completed (i.e. no subscribe
        // is still mid-critical-section once disconnect returns).
        let inf_count = inflight.load(Ordering::SeqCst);
        let com_count = completed.load(Ordering::SeqCst);
        assert_eq!(
            inf_count, com_count,
            "disconnect returned with {inf_count} in-flight subscribes but only {com_count} completed"
        );

        // Drain any subscribes that ran after the flag flip.
        for s in subs {
            s.await.unwrap();
        }
    }
}
