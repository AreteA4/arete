use super::subscription::Subscription;
use crate::compression::CompressedPayload;
use crate::websocket::auth::AuthContext;
use bytes::Bytes;
use dashmap::DashMap;
use futures_util::stream::SplitSink;
use futures_util::SinkExt;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, RwLock};
use tokio_tungstenite::{tungstenite::Message, WebSocketStream};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};
use uuid::Uuid;

pub type WebSocketSender = SplitSink<WebSocketStream<TcpStream>, Message>;

/// Error type for send operations
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendError {
    /// Client not found in registry
    ClientNotFound,
    /// Client's message queue is full - client was disconnected
    ClientBackpressured,
    /// Client's channel is closed - client was disconnected
    ClientDisconnected,
}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SendError::ClientNotFound => write!(f, "client not found"),
            SendError::ClientBackpressured => write!(f, "client backpressured and disconnected"),
            SendError::ClientDisconnected => write!(f, "client disconnected"),
        }
    }
}

impl std::error::Error for SendError {}

/// Information about a connected client
#[derive(Debug)]
pub struct ClientInfo {
    pub id: Uuid,
    pub subscription: Option<Subscription>,
    pub last_seen: SystemTime,
    pub sender: mpsc::Sender<Message>,
    subscriptions: Arc<RwLock<HashMap<String, CancellationToken>>>,
    /// Authentication context for this client
    pub auth_context: Option<AuthContext>,
}

impl ClientInfo {
    pub fn new(id: Uuid, sender: mpsc::Sender<Message>, auth_context: Option<AuthContext>) -> Self {
        Self {
            id,
            subscription: None,
            last_seen: SystemTime::now(),
            sender,
            subscriptions: Arc::new(RwLock::new(HashMap::new())),
            auth_context,
        }
    }

    pub fn update_last_seen(&mut self) {
        self.last_seen = SystemTime::now();
    }

    pub fn is_stale(&self, timeout: Duration) -> bool {
        self.last_seen.elapsed().unwrap_or(Duration::MAX) > timeout
    }

    pub async fn add_subscription(&self, sub_key: String, token: CancellationToken) -> bool {
        let mut subs = self.subscriptions.write().await;
        if let Some(old_token) = subs.insert(sub_key.clone(), token) {
            old_token.cancel();
            debug!("Replaced existing subscription: {}", sub_key);
            false
        } else {
            true
        }
    }

    pub async fn remove_subscription(&self, sub_key: &str) -> bool {
        let mut subs = self.subscriptions.write().await;
        if let Some(token) = subs.remove(sub_key) {
            token.cancel();
            debug!("Cancelled subscription: {}", sub_key);
            true
        } else {
            debug!("Subscription not found for cancellation: {}", sub_key);
            false
        }
    }

    pub async fn cancel_all_subscriptions(&self) {
        let subs = self.subscriptions.read().await;
        for (sub_key, token) in subs.iter() {
            token.cancel();
            debug!("Cancelled subscription on disconnect: {}", sub_key);
        }
    }

    pub async fn subscription_count(&self) -> usize {
        self.subscriptions.read().await.len()
    }
}

/// Manages all connected WebSocket clients using lock-free DashMap.
///
/// Key design decisions:
/// - Uses DashMap for lock-free concurrent access to client registry
/// - Uses try_send instead of send to never block on slow clients
/// - Disconnects clients that are backpressured (queue full) to prevent cascade failures
/// - All public methods are non-blocking or use fine-grained per-key locks
#[derive(Clone)]
pub struct ClientManager {
    clients: Arc<DashMap<Uuid, ClientInfo>>,
    client_timeout: Duration,
    message_queue_size: usize,
}

impl ClientManager {
    pub fn new() -> Self {
        Self {
            clients: Arc::new(DashMap::new()),
            client_timeout: Duration::from_secs(300),
            message_queue_size: 512,
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.client_timeout = timeout;
        self
    }

    pub fn with_message_queue_size(mut self, queue_size: usize) -> Self {
        self.message_queue_size = queue_size;
        self
    }

    /// Add a new client connection.
    ///
    /// Spawns a dedicated sender task for this client that reads from its mpsc channel
    /// and writes to the WebSocket. If the WebSocket write fails, the client is automatically
    /// removed from the registry.
    pub fn add_client(&self, client_id: Uuid, mut ws_sender: WebSocketSender, auth_context: Option<AuthContext>) {
        let (client_tx, mut client_rx) = mpsc::channel::<Message>(self.message_queue_size);
        let client_info = ClientInfo::new(client_id, client_tx, auth_context);

        let clients_ref = self.clients.clone();
        tokio::spawn(async move {
            while let Some(message) = client_rx.recv().await {
                if let Err(e) = ws_sender.send(message).await {
                    warn!("Failed to send message to client {}: {}", client_id, e);
                    break;
                }
            }
            clients_ref.remove(&client_id);
            debug!("WebSocket sender task for client {} stopped", client_id);
        });

        self.clients.insert(client_id, client_info);
        info!("Client {} registered", client_id);
    }

    /// Remove a client from the registry.
    pub fn remove_client(&self, client_id: Uuid) {
        if self.clients.remove(&client_id).is_some() {
            info!("Client {} removed", client_id);
        }
    }

    /// Get the current number of connected clients.
    ///
    /// This is lock-free and returns an approximate count (may be slightly stale
    /// under high concurrency, which is fine for max_clients checks).
    pub fn client_count(&self) -> usize {
        self.clients.len()
    }

    /// Send data to a specific client (non-blocking).
    ///
    /// This method NEVER blocks. If the client's queue is full, the client is
    /// considered too slow and is disconnected to prevent cascade failures.
    /// Use this for live streaming updates.
    ///
    /// For initial snapshots where you expect to send many messages at once,
    /// use `send_to_client_async` instead which will wait for queue space.
    pub fn send_to_client(&self, client_id: Uuid, data: Arc<Bytes>) -> Result<(), SendError> {
        let sender = {
            let client = self
                .clients
                .get(&client_id)
                .ok_or(SendError::ClientNotFound)?;
            client.sender.clone()
        };

        let msg = Message::Binary((*data).clone());
        match sender.try_send(msg) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Full(_)) => {
                warn!(
                    "Client {} backpressured (queue full), disconnecting",
                    client_id
                );
                self.clients.remove(&client_id);
                Err(SendError::ClientBackpressured)
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                debug!("Client {} channel closed", client_id);
                self.clients.remove(&client_id);
                Err(SendError::ClientDisconnected)
            }
        }
    }

    /// Send data to a specific client (async, waits for queue space).
    ///
    /// This method will wait if the client's queue is full, allowing the client
    /// time to catch up. Use this for initial snapshots where you need to send
    /// many messages at once.
    ///
    /// For live streaming updates, use `send_to_client` instead which will
    /// disconnect slow clients rather than blocking.
    pub async fn send_to_client_async(
        &self,
        client_id: Uuid,
        data: Arc<Bytes>,
    ) -> Result<(), SendError> {
        let sender = {
            let client = self
                .clients
                .get(&client_id)
                .ok_or(SendError::ClientNotFound)?;
            client.sender.clone()
        };

        let msg = Message::Binary((*data).clone());
        sender
            .send(msg)
            .await
            .map_err(|_| SendError::ClientDisconnected)
    }

    /// Send a potentially compressed payload to a client (async).
    ///
    /// Compressed payloads are sent as binary frames (raw gzip).
    /// Uncompressed payloads are sent as text frames (JSON).
    pub async fn send_compressed_async(
        &self,
        client_id: Uuid,
        payload: CompressedPayload,
    ) -> Result<(), SendError> {
        let sender = {
            let client = self
                .clients
                .get(&client_id)
                .ok_or(SendError::ClientNotFound)?;
            client.sender.clone()
        };

        let msg = match payload {
            CompressedPayload::Compressed(bytes) => Message::Binary(bytes),
            CompressedPayload::Uncompressed(bytes) => Message::Binary(bytes),
        };
        sender
            .send(msg)
            .await
            .map_err(|_| SendError::ClientDisconnected)
    }

    /// Update the subscription for a client.
    pub fn update_subscription(&self, client_id: Uuid, subscription: Subscription) -> bool {
        if let Some(mut client) = self.clients.get_mut(&client_id) {
            client.subscription = Some(subscription);
            client.update_last_seen();
            debug!("Updated subscription for client {}", client_id);
            true
        } else {
            warn!(
                "Failed to update subscription for unknown client {}",
                client_id
            );
            false
        }
    }

    /// Update the last_seen timestamp for a client.
    pub fn update_client_last_seen(&self, client_id: Uuid) {
        if let Some(mut client) = self.clients.get_mut(&client_id) {
            client.update_last_seen();
        }
    }

    /// Get the subscription for a client.
    pub fn get_subscription(&self, client_id: Uuid) -> Option<Subscription> {
        self.clients
            .get(&client_id)
            .and_then(|c| c.subscription.clone())
    }

    /// Check if a client exists.
    pub fn has_client(&self, client_id: Uuid) -> bool {
        self.clients.contains_key(&client_id)
    }

    pub async fn add_client_subscription(
        &self,
        client_id: Uuid,
        sub_key: String,
        token: CancellationToken,
    ) -> bool {
        if let Some(client) = self.clients.get(&client_id) {
            client.add_subscription(sub_key, token).await
        } else {
            false
        }
    }

    pub async fn remove_client_subscription(&self, client_id: Uuid, sub_key: &str) -> bool {
        if let Some(client) = self.clients.get(&client_id) {
            client.remove_subscription(sub_key).await
        } else {
            false
        }
    }

    pub async fn cancel_all_client_subscriptions(&self, client_id: Uuid) {
        if let Some(client) = self.clients.get(&client_id) {
            client.cancel_all_subscriptions().await;
        }
    }

    /// Remove stale clients that haven't been seen within the timeout period.
    pub fn cleanup_stale_clients(&self) -> usize {
        let timeout = self.client_timeout;
        let mut stale_clients = Vec::new();

        for entry in self.clients.iter() {
            if entry.value().is_stale(timeout) {
                stale_clients.push(*entry.key());
            }
        }

        let removed_count = stale_clients.len();
        for client_id in stale_clients {
            self.clients.remove(&client_id);
            info!("Removed stale client {}", client_id);
        }

        removed_count
    }

    /// Start a background task that periodically cleans up stale clients.
    pub fn start_cleanup_task(&self) {
        let client_manager = self.clone();

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));

            loop {
                interval.tick().await;
                let removed = client_manager.cleanup_stale_clients();
                if removed > 0 {
                    info!("Cleaned up {} stale clients", removed);
                }
            }
        });
    }

    /// ENFORCEMENT HOOKS
    /// 
    /// These methods provide hooks for enforcing limits based on auth context.
    /// They check limits before allowing operations and return errors if limits are exceeded.

    /// Check if a connection is allowed for the given auth context.
    /// 
    /// Returns Ok(()) if the connection is allowed, or an error with a reason if not.
    pub fn check_connection_allowed(&self, auth_context: &Option<AuthContext>) -> Result<(), String> {
        if let Some(ctx) = auth_context {
            // Check max connections per subject
            if let Some(max_connections) = ctx.limits.max_connections {
                let current_connections = self.count_connections_for_subject(&ctx.subject);
                if current_connections >= max_connections as usize {
                    return Err(format!(
                        "Connection limit exceeded: {} of {} connections for subject {}",
                        current_connections, max_connections, ctx.subject
                    ));
                }
            }
        }
        Ok(())
    }

    /// Count connections for a specific subject
    fn count_connections_for_subject(&self, subject: &str) -> usize {
        self.clients
            .iter()
            .filter(|entry| {
                entry
                    .value()
                    .auth_context
                    .as_ref()
                    .map(|ctx| ctx.subject == subject)
                    .unwrap_or(false)
            })
            .count()
    }

    /// Check if a subscription is allowed for the given client.
    /// 
    /// Returns Ok(()) if the subscription is allowed, or an error with a reason if not.
    pub async fn check_subscription_allowed(&self, client_id: Uuid) -> Result<(), String> {
        if let Some(client) = self.clients.get(&client_id) {
            let current_subs = client.subscription_count().await;
            
            // Check max subscriptions per connection from auth context
            if let Some(ref ctx) = client.auth_context {
                if let Some(max_subs) = ctx.limits.max_subscriptions {
                    if current_subs >= max_subs as usize {
                        return Err(format!(
                            "Subscription limit exceeded: {} of {} subscriptions for client {}",
                            current_subs, max_subs, client_id
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    /// Get metering key for a client
    pub fn get_metering_key(&self, client_id: Uuid) -> Option<String> {
        self.clients
            .get(&client_id)
            .and_then(|client| {
                client
                    .auth_context
                    .as_ref()
                    .map(|ctx| ctx.metering_key.clone())
            })
    }

    /// Check if a snapshot request is allowed (based on max_snapshot_rows limit)
    pub fn check_snapshot_allowed(&self, client_id: Uuid, requested_rows: u32) -> Result<(), String> {
        if let Some(client) = self.clients.get(&client_id) {
            if let Some(ref ctx) = client.auth_context {
                if let Some(max_rows) = ctx.limits.max_snapshot_rows {
                    if requested_rows > max_rows {
                        return Err(format!(
                            "Snapshot limit exceeded: requested {} rows, max allowed is {} for client {}",
                            requested_rows, max_rows, client_id
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}

impl Default for ClientManager {
    fn default() -> Self {
        Self::new()
    }
}
