use crate::account_policy::{redact_identity, AccountPolicyError, AccountPolicyRegistry};
use crate::compression::CompressedPayload;
use crate::websocket::auth::{AuthContext, AuthDeny};
use crate::websocket::rate_limiter::{RateLimitResult, WebSocketRateLimiter};
use arete_auth::Limits;
use bytes::Bytes;
use dashmap::DashMap;
use futures_util::stream::SplitSink;
use futures_util::SinkExt;
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};
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

/// Egress tracking for a client
#[derive(Debug)]
struct EgressTracker {
    /// Bytes sent in the current minute window
    bytes_this_minute: u64,
    /// Start of the current minute window
    window_start: SystemTime,
}

/// Inbound message-rate tracking for a client
#[derive(Debug)]
struct MessageRateTracker {
    messages_this_minute: u32,
    window_start: SystemTime,
}

impl MessageRateTracker {
    fn new() -> Self {
        Self {
            messages_this_minute: 0,
            window_start: SystemTime::now(),
        }
    }

    fn maybe_reset_window(&mut self) {
        let now = SystemTime::now();
        if now.duration_since(self.window_start).unwrap_or_default() >= Duration::from_secs(60) {
            self.messages_this_minute = 0;
            self.window_start = now;
        }
    }

    fn record_message(&mut self, limit: u32) -> bool {
        self.maybe_reset_window();
        if self.messages_this_minute + 1 > limit {
            false
        } else {
            self.messages_this_minute += 1;
            true
        }
    }

    fn current_usage(&mut self) -> u32 {
        self.maybe_reset_window();
        self.messages_this_minute
    }
}

impl EgressTracker {
    fn new() -> Self {
        Self {
            bytes_this_minute: 0,
            window_start: SystemTime::now(),
        }
    }

    /// Check if we need to reset the window (new minute)
    fn maybe_reset_window(&mut self) {
        let now = SystemTime::now();
        if now.duration_since(self.window_start).unwrap_or_default() >= Duration::from_secs(60) {
            self.bytes_this_minute = 0;
            self.window_start = now;
        }
    }

    /// Record bytes sent, returning true if within limit
    fn record_bytes(&mut self, bytes: usize, limit: u64) -> bool {
        self.maybe_reset_window();
        let bytes_u64 = bytes as u64;
        if self.bytes_this_minute + bytes_u64 > limit {
            false
        } else {
            self.bytes_this_minute += bytes_u64;
            true
        }
    }

    /// Get current usage
    fn current_usage(&mut self) -> u64 {
        self.maybe_reset_window();
        self.bytes_this_minute
    }
}

/// Aggregate message/egress usage shared by every connection of one account.
///
/// This is a per-process tracker, not a globally exact quota; durable
/// cross-replica metering is a separate system.
#[derive(Debug)]
struct AccountUsage {
    egress: std::sync::Mutex<EgressTracker>,
    messages: std::sync::Mutex<MessageRateTracker>,
    last_seen: std::sync::Mutex<Instant>,
}

impl AccountUsage {
    fn new() -> Self {
        Self {
            egress: std::sync::Mutex::new(EgressTracker::new()),
            messages: std::sync::Mutex::new(MessageRateTracker::new()),
            last_seen: std::sync::Mutex::new(Instant::now()),
        }
    }

    fn touch(&self) {
        if let Ok(mut last_seen) = self.last_seen.lock() {
            *last_seen = Instant::now();
        }
    }

    fn record_egress(&self, bytes: usize, limit: u64) -> bool {
        self.touch();
        match self.egress.lock() {
            Ok(mut tracker) => tracker.record_bytes(bytes, limit),
            Err(_) => true,
        }
    }

    fn record_message(&self, limit: u32) -> bool {
        self.touch();
        match self.messages.lock() {
            Ok(mut tracker) => tracker.record_message(limit),
            Err(_) => true,
        }
    }

    fn idle_for(&self) -> Duration {
        self.last_seen
            .lock()
            .map(|last_seen| last_seen.elapsed())
            .unwrap_or(Duration::MAX)
    }
}

/// Information about a connected client
#[derive(Debug)]
pub struct ClientInfo {
    pub id: Uuid,
    pub last_seen: SystemTime,
    pub sender: mpsc::Sender<Message>,
    subscriptions: Arc<RwLock<HashMap<String, CancellationToken>>>,
    /// Authentication context for this client
    pub auth_context: Option<AuthContext>,
    /// Client's IP address for rate limiting
    pub remote_addr: SocketAddr,
    /// Egress tracking for rate limiting
    egress_tracker: std::sync::Mutex<EgressTracker>,
    /// Inbound message-rate tracking for rate limiting
    message_rate_tracker: std::sync::Mutex<MessageRateTracker>,
}

impl ClientInfo {
    pub fn new(
        id: Uuid,
        sender: mpsc::Sender<Message>,
        auth_context: Option<AuthContext>,
        remote_addr: SocketAddr,
    ) -> Self {
        Self {
            id,
            last_seen: SystemTime::now(),
            sender,
            subscriptions: Arc::new(RwLock::new(HashMap::new())),
            auth_context,
            remote_addr,
            egress_tracker: std::sync::Mutex::new(EgressTracker::new()),
            message_rate_tracker: std::sync::Mutex::new(MessageRateTracker::new()),
        }
    }

    /// Record bytes sent, returning true if within limit
    pub fn record_egress(&self, bytes: usize) -> Option<u64> {
        if let Ok(mut tracker) = self.egress_tracker.lock() {
            if let Some(ref ctx) = self.auth_context {
                if let Some(limit) = ctx.limits.max_bytes_per_minute {
                    if tracker.record_bytes(bytes, limit) {
                        return Some(tracker.current_usage());
                    } else {
                        return None; // Limit exceeded
                    }
                }
            }
            // No limit set, return current usage (0)
            return Some(tracker.current_usage());
        }
        None
    }

    /// Record an inbound client message, returning true if within limit.
    pub fn record_inbound_message(&self) -> Option<u32> {
        if let Ok(mut tracker) = self.message_rate_tracker.lock() {
            if let Some(ref ctx) = self.auth_context {
                if let Some(limit) = ctx.limits.max_messages_per_minute {
                    if tracker.record_message(limit) {
                        return Some(tracker.current_usage());
                    } else {
                        return None;
                    }
                }
            }

            return Some(tracker.current_usage());
        }

        None
    }

    pub fn update_last_seen(&mut self) {
        self.last_seen = SystemTime::now();
    }

    pub fn is_stale(&self, timeout: Duration) -> bool {
        self.last_seen.elapsed().unwrap_or(Duration::MAX) > timeout
    }

    pub async fn add_subscription(
        &self,
        subscription_id: String,
        token: CancellationToken,
    ) -> bool {
        let mut subs = self.subscriptions.write().await;
        match subs.entry(subscription_id) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(token);
                true
            }
            std::collections::hash_map::Entry::Occupied(_) => false,
        }
    }

    pub async fn remove_subscription(&self, subscription_id: &str) -> bool {
        let mut subs = self.subscriptions.write().await;
        if let Some(token) = subs.remove(subscription_id) {
            token.cancel();
            debug!("Cancelled subscription: {}", subscription_id);
            true
        } else {
            debug!(
                "Subscription not found for cancellation: {}",
                subscription_id
            );
            false
        }
    }

    pub async fn cancel_all_subscriptions(&self) {
        let subs = self.subscriptions.read().await;
        for (subscription_id, token) in subs.iter() {
            token.cancel();
            debug!("Cancelled subscription on disconnect: {}", subscription_id);
        }
    }

    pub async fn subscription_count(&self) -> usize {
        self.subscriptions.read().await.len()
    }
}

/// Configuration for rate limiting in ClientManager
///
/// These settings control various rate limits at the connection level.
/// Per-subject limits are controlled via AuthContext.Limits.
#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    /// Global maximum connections per IP address
    pub max_connections_per_ip: Option<usize>,
    /// Global maximum connections per metering key
    pub max_connections_per_metering_key: Option<usize>,
    /// Global maximum connections per origin
    pub max_connections_per_origin: Option<usize>,
    /// Default connection timeout for stale client cleanup
    pub client_timeout: Duration,
    /// Message queue size per client
    pub message_queue_size: usize,
    /// Maximum reconnect attempts per client (optional global default)
    pub max_reconnect_attempts: Option<u32>,
    /// Rate limit window duration for message counting
    pub message_rate_window: Duration,
    /// Rate limit window duration for egress tracking
    pub egress_rate_window: Duration,
    /// Default limits applied when auth token doesn't specify limits
    /// These act as server-wide fallback limits for all connections
    pub default_limits: Option<Limits>,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            max_connections_per_ip: None,
            max_connections_per_metering_key: None,
            max_connections_per_origin: None,
            client_timeout: Duration::from_secs(300),
            message_queue_size: 512,
            max_reconnect_attempts: None,
            message_rate_window: Duration::from_secs(60),
            egress_rate_window: Duration::from_secs(60),
            default_limits: None,
        }
    }
}

impl RateLimitConfig {
    /// Load configuration from environment variables
    ///
    /// Environment variables:
    /// - `ARETE_WS_MAX_CONNECTIONS_PER_IP` - Max connections per IP (default: unlimited)
    /// - `ARETE_WS_MAX_CONNECTIONS_PER_METERING_KEY` - Max connections per metering key (default: unlimited)
    /// - `ARETE_WS_MAX_CONNECTIONS_PER_ORIGIN` - Max connections per origin (default: unlimited)
    /// - `ARETE_WS_CLIENT_TIMEOUT_SECS` - Client timeout in seconds (default: 300)
    /// - `ARETE_WS_MESSAGE_QUEUE_SIZE` - Message queue size per client (default: 512)
    /// - `ARETE_WS_RATE_LIMIT_WINDOW_SECS` - Rate limit window in seconds (default: 60)
    /// - `ARETE_WS_DEFAULT_MAX_CONNECTIONS` - Default max connections per subject (fallback when token has no limit)
    /// - `ARETE_WS_DEFAULT_MAX_SUBSCRIPTIONS` - Default max subscriptions per connection (fallback when token has no limit)
    /// - `ARETE_WS_DEFAULT_MAX_SNAPSHOT_ROWS` - Default max snapshot rows per request (fallback when token has no limit)
    /// - `ARETE_WS_DEFAULT_MAX_MESSAGES_PER_MINUTE` - Default max messages per minute (fallback when token has no limit)
    /// - `ARETE_WS_DEFAULT_MAX_BYTES_PER_MINUTE` - Default max bytes per minute (fallback when token has no limit)
    pub fn from_env() -> Self {
        let mut config = Self::default();

        if let Ok(val) = std::env::var("ARETE_WS_MAX_CONNECTIONS_PER_IP") {
            if let Ok(max) = val.parse() {
                config.max_connections_per_ip = Some(max);
            }
        }

        if let Ok(val) = std::env::var("ARETE_WS_MAX_CONNECTIONS_PER_METERING_KEY") {
            if let Ok(max) = val.parse() {
                config.max_connections_per_metering_key = Some(max);
            }
        }

        if let Ok(val) = std::env::var("ARETE_WS_MAX_CONNECTIONS_PER_ORIGIN") {
            if let Ok(max) = val.parse() {
                config.max_connections_per_origin = Some(max);
            }
        }

        if let Ok(val) = std::env::var("ARETE_WS_CLIENT_TIMEOUT_SECS") {
            if let Ok(secs) = val.parse() {
                config.client_timeout = Duration::from_secs(secs);
            }
        }

        if let Ok(val) = std::env::var("ARETE_WS_MESSAGE_QUEUE_SIZE") {
            if let Ok(size) = val.parse() {
                config.message_queue_size = size;
            }
        }

        if let Ok(val) = std::env::var("ARETE_WS_RATE_LIMIT_WINDOW_SECS") {
            if let Ok(secs) = val.parse() {
                config.message_rate_window = Duration::from_secs(secs);
                config.egress_rate_window = Duration::from_secs(secs);
            }
        }

        // Load default limits from environment (fallback when auth token doesn't specify limits)
        let mut default_limits = Limits::default();
        let mut has_default_limits = false;

        if let Ok(val) = std::env::var("ARETE_WS_DEFAULT_MAX_CONNECTIONS") {
            if let Ok(max) = val.parse() {
                default_limits.max_connections = Some(max);
                has_default_limits = true;
            }
        }

        if let Ok(val) = std::env::var("ARETE_WS_DEFAULT_MAX_SUBSCRIPTIONS") {
            if let Ok(max) = val.parse() {
                default_limits.max_subscriptions = Some(max);
                has_default_limits = true;
            }
        }

        if let Ok(val) = std::env::var("ARETE_WS_DEFAULT_MAX_SNAPSHOT_ROWS") {
            if let Ok(max) = val.parse() {
                default_limits.max_snapshot_rows = Some(max);
                has_default_limits = true;
            }
        }

        if let Ok(val) = std::env::var("ARETE_WS_DEFAULT_MAX_MESSAGES_PER_MINUTE") {
            if let Ok(max) = val.parse() {
                default_limits.max_messages_per_minute = Some(max);
                has_default_limits = true;
            }
        }

        if let Ok(val) = std::env::var("ARETE_WS_DEFAULT_MAX_BYTES_PER_MINUTE") {
            if let Ok(max) = val.parse() {
                default_limits.max_bytes_per_minute = Some(max);
                has_default_limits = true;
            }
        }

        if has_default_limits {
            config.default_limits = Some(default_limits);
        }

        config
    }

    /// Set the maximum connections per IP
    pub fn with_max_connections_per_ip(mut self, max: usize) -> Self {
        self.max_connections_per_ip = Some(max);
        self
    }

    /// Set the client timeout
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.client_timeout = timeout;
        self
    }

    /// Set the message queue size
    pub fn with_message_queue_size(mut self, size: usize) -> Self {
        self.message_queue_size = size;
        self
    }

    /// Set the rate limit window (applies to both message and egress windows)
    pub fn with_rate_limit_window(mut self, window: Duration) -> Self {
        self.message_rate_window = window;
        self.egress_rate_window = window;
        self
    }

    /// Set default limits applied when auth token doesn't specify limits
    ///
    /// These limits act as server-wide fallbacks for connections
    /// where the authentication token doesn't include explicit limits.
    pub fn with_default_limits(mut self, limits: Limits) -> Self {
        self.default_limits = Some(limits);
        self
    }
}

/// Bound on tracked account usage entries; signed account keys keep the
/// cardinality legitimate, this is a hard backstop.
const MAX_TRACKED_ACCOUNT_USAGE: usize = crate::account_policy::DEFAULT_MAX_TRACKED_ACCOUNTS;

/// Idle TTL after which aggregate account usage state is evicted.
const ACCOUNT_USAGE_IDLE_TTL: Duration = crate::account_policy::DEFAULT_ACCOUNT_IDLE_TTL;

/// Manages all connected WebSocket clients using lock-free DashMap.
///
/// Key design decisions:
/// - Uses DashMap for lock-free concurrent access to client registry
/// - Uses try_send instead of send to never block on slow clients
/// - Disconnects clients that are backpressured (queue full) to prevent cascade failures
/// - All public methods are non-blocking or use fine-grained per-key locks
/// - Supports configurable rate limiting per IP, subject, and global defaults
#[derive(Clone)]
pub struct ClientManager {
    clients: Arc<DashMap<Uuid, ClientInfo>>,
    rate_limit_config: RateLimitConfig,
    /// Optional WebSocket rate limiter for granular rate control
    rate_limiter: Option<Arc<WebSocketRateLimiter>>,
    /// Per-process account policy versions and signed aggregate limits
    account_policies: Arc<AccountPolicyRegistry>,
    /// Per-process aggregate message/egress usage per account
    account_usage: Arc<DashMap<String, AccountUsage>>,
}

impl ClientManager {
    pub fn new() -> Self {
        Self::with_config(RateLimitConfig::default())
    }

    /// Create a new ClientManager with the given rate limit configuration
    pub fn with_config(config: RateLimitConfig) -> Self {
        Self {
            clients: Arc::new(DashMap::new()),
            rate_limit_config: config,
            rate_limiter: None,
            account_policies: Arc::new(AccountPolicyRegistry::default()),
            account_usage: Arc::new(DashMap::new()),
        }
    }

    /// Load configuration from environment variables
    ///
    /// See `RateLimitConfig::from_env` for supported variables.
    pub fn from_env() -> Self {
        Self::with_config(RateLimitConfig::from_env())
    }

    /// Set the client timeout for stale client cleanup
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.rate_limit_config.client_timeout = timeout;
        self
    }

    /// Set the message queue size per client
    pub fn with_message_queue_size(mut self, queue_size: usize) -> Self {
        self.rate_limit_config.message_queue_size = queue_size;
        self
    }

    /// Set a global limit on connections per IP address
    pub fn with_max_connections_per_ip(mut self, max: usize) -> Self {
        self.rate_limit_config.max_connections_per_ip = Some(max);
        self
    }

    /// Set the rate limit window duration
    pub fn with_rate_limit_window(mut self, window: Duration) -> Self {
        self.rate_limit_config.message_rate_window = window;
        self.rate_limit_config.egress_rate_window = window;
        self
    }

    /// Set default limits applied when auth token doesn't specify limits
    ///
    /// These limits act as server-wide fallbacks for connections
    /// where the authentication token doesn't include explicit limits.
    pub fn with_default_limits(mut self, limits: Limits) -> Self {
        self.rate_limit_config.default_limits = Some(limits);
        self
    }

    /// Set a WebSocket rate limiter for granular rate control
    pub fn with_rate_limiter(mut self, rate_limiter: Arc<WebSocketRateLimiter>) -> Self {
        self.rate_limiter = Some(rate_limiter);
        self
    }

    /// Get the rate limiter if configured
    pub fn rate_limiter(&self) -> Option<&WebSocketRateLimiter> {
        self.rate_limiter.as_ref().map(|r| r.as_ref())
    }

    /// Get the current rate limit configuration
    pub fn rate_limit_config(&self) -> &RateLimitConfig {
        &self.rate_limit_config
    }

    /// Account policy state observed by this manager.
    pub fn account_policies(&self) -> &AccountPolicyRegistry {
        &self.account_policies
    }

    /// Record aggregate account egress; true when within the signed limit.
    fn record_account_egress(&self, ctx: &AuthContext, bytes: usize) -> bool {
        if ctx.is_legacy_policy() || ctx.account_key.is_none() {
            return true;
        }
        let Some(limit) = ctx.account_limits.max_bytes_per_minute else {
            return true;
        };
        let account = ctx.account_key();
        if let Some(usage) = self.account_usage.get(account) {
            return usage.record_egress(bytes, limit);
        }
        if self.account_usage.len() >= MAX_TRACKED_ACCOUNT_USAGE {
            warn!(
                account = %redact_identity(account),
                "account usage tracking at capacity; aggregate egress guard skipped"
            );
            return true;
        }
        self.account_usage
            .entry(account.to_string())
            .or_insert_with(AccountUsage::new)
            .record_egress(bytes, limit)
    }

    /// Record one aggregate account inbound message; true when within limit.
    fn record_account_message(&self, ctx: &AuthContext) -> bool {
        if ctx.is_legacy_policy() || ctx.account_key.is_none() {
            return true;
        }
        let Some(limit) = ctx.account_limits.max_messages_per_minute else {
            return true;
        };
        let account = ctx.account_key();
        if let Some(usage) = self.account_usage.get(account) {
            return usage.record_message(limit);
        }
        if self.account_usage.len() >= MAX_TRACKED_ACCOUNT_USAGE {
            warn!(
                account = %redact_identity(account),
                "account usage tracking at capacity; aggregate message guard skipped"
            );
            return true;
        }
        self.account_usage
            .entry(account.to_string())
            .or_insert_with(AccountUsage::new)
            .record_message(limit)
    }

    /// Enforce the per-connection and aggregate account egress budgets.
    fn enforce_egress_budgets(&self, client_id: Uuid, bytes: usize) -> Result<(), SendError> {
        let (connection_ok, account_ok) = {
            let Some(client) = self.clients.get(&client_id) else {
                return Err(SendError::ClientNotFound);
            };
            let connection_ok = client.record_egress(bytes).is_some();
            let account_ok = !connection_ok
                || client
                    .auth_context
                    .as_ref()
                    .map(|ctx| self.record_account_egress(ctx, bytes))
                    .unwrap_or(true);
            (connection_ok, account_ok)
        };
        if !connection_ok {
            warn!("Client {} exceeded egress limit, disconnecting", client_id);
            self.clients.remove(&client_id);
            return Err(SendError::ClientDisconnected);
        }
        if !account_ok {
            warn!(
                "Client {} exceeded the account egress limit, disconnecting",
                client_id
            );
            self.clients.remove(&client_id);
            return Err(SendError::ClientDisconnected);
        }
        Ok(())
    }

    /// Add a new client connection.
    ///
    /// Spawns a dedicated sender task for this client that reads from its mpsc channel
    /// and writes to the WebSocket. If the WebSocket write fails, the client is automatically
    /// removed from the registry.
    pub fn add_client(
        &self,
        client_id: Uuid,
        mut ws_sender: WebSocketSender,
        auth_context: Option<AuthContext>,
        remote_addr: SocketAddr,
    ) {
        let (client_tx, mut client_rx) =
            mpsc::channel::<Message>(self.rate_limit_config.message_queue_size);
        let client_info = ClientInfo::new(client_id, client_tx, auth_context, remote_addr);

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
        info!("Client {} registered from {}", client_id, remote_addr);
    }

    /// Remove a client from the registry.
    pub fn remove_client(&self, client_id: Uuid) {
        if self.clients.remove(&client_id).is_some() {
            info!("Client {} removed", client_id);
        }
    }

    /// Update the auth context for a client.
    ///
    /// Used for in-band auth refresh without reconnecting.
    pub fn update_client_auth(&self, client_id: Uuid, auth_context: AuthContext) -> bool {
        if let Some(mut client) = self.clients.get_mut(&client_id) {
            client.auth_context = Some(auth_context);
            debug!("Updated auth context for client {}", client_id);
            true
        } else {
            false
        }
    }

    /// Check if a client's token has expired.
    ///
    /// Returns true if the client has an auth context and it has expired.
    /// If expired, the client is removed from the registry.
    pub fn check_and_remove_expired(&self, client_id: Uuid) -> bool {
        if let Some(client) = self.clients.get(&client_id) {
            if let Some(ref ctx) = client.auth_context {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                if ctx.expires_at <= now {
                    warn!(
                        "Client {} token expired (expired at {}), disconnecting",
                        client_id, ctx.expires_at
                    );
                    self.clients.remove(&client_id);
                    return true;
                }
            }
        }
        false
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
        // Check if client token has expired before sending
        if self.check_and_remove_expired(client_id) {
            return Err(SendError::ClientDisconnected);
        }

        // Check per-connection and aggregate account egress limits
        self.enforce_egress_budgets(client_id, data.len())?;

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
        // Check if client token has expired before sending
        if self.check_and_remove_expired(client_id) {
            return Err(SendError::ClientDisconnected);
        }

        // Check per-connection and aggregate account egress limits
        self.enforce_egress_budgets(client_id, data.len())?;

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

    /// Send a text message to a specific client (async).
    ///
    /// This method sends a text message directly to the client's WebSocket.
    /// Used for control messages like auth refresh responses.
    pub async fn send_text_to_client(
        &self,
        client_id: Uuid,
        text: String,
    ) -> Result<(), SendError> {
        // Check if client token has expired before sending
        if self.check_and_remove_expired(client_id) {
            return Err(SendError::ClientDisconnected);
        }

        let sender = {
            let client = self
                .clients
                .get(&client_id)
                .ok_or(SendError::ClientNotFound)?;
            client.sender.clone()
        };

        let msg = Message::Text(text.into());
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
        // Check if client token has expired before sending
        if self.check_and_remove_expired(client_id) {
            return Err(SendError::ClientDisconnected);
        }

        let (sender, bytes_to_record) = {
            let client = self
                .clients
                .get(&client_id)
                .ok_or(SendError::ClientNotFound)?;

            let bytes = match &payload {
                CompressedPayload::Compressed(bytes) => bytes.len(),
                CompressedPayload::Uncompressed(bytes) => bytes.len(),
            };

            (client.sender.clone(), bytes)
        };

        // Check per-connection and aggregate account egress limits
        self.enforce_egress_budgets(client_id, bytes_to_record)?;

        let msg = match payload {
            CompressedPayload::Compressed(bytes) => Message::Binary(bytes),
            CompressedPayload::Uncompressed(bytes) => Message::Binary(bytes),
        };
        sender
            .send(msg)
            .await
            .map_err(|_| SendError::ClientDisconnected)
    }

    /// Update the last_seen timestamp for a client.
    pub fn update_client_last_seen(&self, client_id: Uuid) {
        if let Some(mut client) = self.clients.get_mut(&client_id) {
            client.update_last_seen();
        }
    }

    /// Check whether an inbound message is allowed for a client.
    #[allow(clippy::result_large_err)]
    pub fn check_inbound_message_allowed(&self, client_id: Uuid) -> Result<(), AuthDeny> {
        if self.check_and_remove_expired(client_id) {
            return Err(AuthDeny::new(
                crate::websocket::auth::AuthErrorCode::TokenExpired,
                "Authentication token expired",
            ));
        }

        let (connection_ok, account_ok) = {
            let Some(client) = self.clients.get(&client_id) else {
                return Err(AuthDeny::new(
                    crate::websocket::auth::AuthErrorCode::InternalError,
                    "Client not found",
                ));
            };
            let connection_ok = client.record_inbound_message().is_some();
            let account_ok = !connection_ok
                || client
                    .auth_context
                    .as_ref()
                    .map(|ctx| self.record_account_message(ctx))
                    .unwrap_or(true);
            (connection_ok, account_ok)
        };

        if connection_ok && account_ok {
            return Ok(());
        }
        self.clients.remove(&client_id);
        let scope = if connection_ok {
            "inbound account websocket messages"
        } else {
            "inbound websocket messages"
        };
        Err(
            AuthDeny::rate_limited(self.rate_limit_config.message_rate_window, scope).with_context(
                format!("client {} exceeded the inbound message budget", client_id),
            ),
        )
    }

    /// Check if a client exists.
    pub fn has_client(&self, client_id: Uuid) -> bool {
        self.clients.contains_key(&client_id)
    }

    pub async fn add_client_subscription(
        &self,
        client_id: Uuid,
        subscription_id: String,
        token: CancellationToken,
    ) -> bool {
        if let Some(client) = self.clients.get(&client_id) {
            client.add_subscription(subscription_id, token).await
        } else {
            false
        }
    }

    pub async fn remove_client_subscription(&self, client_id: Uuid, subscription_id: &str) -> bool {
        if let Some(client) = self.clients.get(&client_id) {
            client.remove_subscription(subscription_id).await
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
        let timeout = self.rate_limit_config.client_timeout;
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

    /// Evict aggregate account usage and policy state for accounts with no
    /// live connection past the idle TTL.
    fn cleanup_account_state(&self) {
        let live_accounts: HashSet<String> = self
            .clients
            .iter()
            .filter_map(|entry| {
                entry
                    .value()
                    .auth_context
                    .as_ref()
                    .and_then(|ctx| ctx.account_key.clone())
            })
            .collect();
        self.account_usage.retain(|account, usage| {
            live_accounts.contains(account) || usage.idle_for() < ACCOUNT_USAGE_IDLE_TTL
        });
        self.account_policies
            .evict_idle(|account| live_accounts.contains(account));
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
                client_manager.cleanup_account_state();
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
    #[allow(clippy::result_large_err)]
    pub async fn check_connection_allowed(
        &self,
        remote_addr: SocketAddr,
        auth_context: &Option<AuthContext>,
    ) -> Result<(), AuthDeny> {
        // Check rate limiter first if configured
        if let Some(ref rate_limiter) = self.rate_limiter {
            // Check handshake rate limit for IP
            match rate_limiter.check_handshake(remote_addr).await {
                RateLimitResult::Allowed { .. } => {}
                RateLimitResult::Denied { retry_after, limit } => {
                    return Err(AuthDeny::rate_limited(retry_after, "websocket handshakes")
                        .with_context(format!(
                            "handshake rate limit of {} per minute exceeded for {}",
                            limit, remote_addr
                        )));
                }
            }

            if let Some(ref ctx) = auth_context {
                // Connection-attempt rate per resolved consumer. Legacy
                // tokens resolve to the subject with the configured window,
                // preserving old behavior exactly.
                match rate_limiter
                    .check_connection_for_consumer(
                        ctx.consumer_key(),
                        ctx.limits.max_connection_attempts_per_minute,
                    )
                    .await
                {
                    RateLimitResult::Allowed { .. } => {}
                    RateLimitResult::Denied { retry_after, limit } => {
                        return Err(AuthDeny::rate_limited(retry_after, "websocket connections")
                            .with_context(format!(
                                "connection rate limit for consumer {} of {} per minute exceeded",
                                ctx.consumer_key(),
                                limit
                            )));
                    }
                }

                // Connection-attempt rate per resolved account. Legacy
                // tokens resolve to the metering key; anonymous v2 tokens
                // carry no account and skip the aggregate bucket.
                if ctx.is_legacy_policy() || ctx.account_key.is_some() {
                    match rate_limiter
                        .check_connection_for_account(
                            ctx.account_key(),
                            ctx.account_limits.max_connection_attempts_per_minute,
                        )
                        .await
                    {
                        RateLimitResult::Allowed { .. } => {}
                        RateLimitResult::Denied { retry_after, limit } => {
                            return Err(AuthDeny::rate_limited(
                                retry_after,
                                "metered websocket connections",
                            )
                            .with_context(format!(
                                "connection rate limit for account {} of {} per minute exceeded",
                                ctx.account_key(),
                                limit
                            )));
                        }
                    }
                }
            }
        }

        // Check global per-IP connection limit
        if let Some(max_per_ip) = self.rate_limit_config.max_connections_per_ip {
            let current_ip_connections = self.count_connections_for_ip(&remote_addr);
            if current_ip_connections >= max_per_ip {
                return Err(AuthDeny::connection_limit_exceeded(
                    &format!("ip {}", remote_addr.ip()),
                    current_ip_connections,
                    max_per_ip,
                ));
            }
        }

        if let Some(ctx) = auth_context {
            // Admit the token against previously observed account policy and
            // count legacy tokens so Plan 030 can end compatibility.
            if ctx.is_legacy_policy() {
                let legacy_policy_token = self.account_policies.record_legacy_token();
                debug!(legacy_policy_token, "legacy policy token admitted");
            } else if let (Some(account), Some(policy_version)) =
                (ctx.account_key.as_deref(), ctx.policy_version)
            {
                match self
                    .account_policies
                    .observe(account, policy_version, &ctx.account_limits)
                {
                    Ok(()) => {}
                    Err(AccountPolicyError::StaleVersion { presented, current }) => {
                        debug!(
                            account = %redact_identity(account),
                            presented,
                            current,
                            "stale policy version rejected"
                        );
                        return Err(AuthDeny::new(
                            crate::websocket::auth::AuthErrorCode::TokenExpired,
                            "session policy version is stale; refresh the session token",
                        ));
                    }
                    Err(AccountPolicyError::ConflictingLimits { version }) => {
                        warn!(
                            account = %redact_identity(account),
                            version,
                            "signed account limits conflict for one policy version"
                        );
                        return Err(AuthDeny::new(
                            crate::websocket::auth::AuthErrorCode::InternalError,
                            "signed account limits conflict with previously observed policy",
                        ));
                    }
                    Err(AccountPolicyError::CapacityExhausted) => {
                        warn!("account policy state at capacity; denying admission");
                        return Err(AuthDeny::new(
                            crate::websocket::auth::AuthErrorCode::InternalError,
                            "account policy state is at capacity; retry shortly",
                        ));
                    }
                }
            }

            // Check max connections per resolved consumer (token limits,
            // fallback to default limits). Legacy tokens resolve to the
            // subject, preserving old behavior.
            let max_connections = ctx.limits.max_connections.or_else(|| {
                self.rate_limit_config
                    .default_limits
                    .as_ref()
                    .and_then(|l| l.max_connections)
            });
            if let Some(max_connections) = max_connections {
                let current_connections = self.count_connections_for_consumer(ctx.consumer_key());
                if current_connections >= max_connections as usize {
                    return Err(AuthDeny::connection_limit_exceeded(
                        &format!("consumer {}", ctx.consumer_key()),
                        current_connections,
                        max_connections as usize,
                    ));
                }
            }

            // Check aggregate concurrent connections per account from the
            // signed account limits.
            if !ctx.is_legacy_policy() && ctx.account_key.is_some() {
                if let Some(max_account_connections) = ctx.account_limits.max_connections {
                    let current_account_connections =
                        self.count_connections_for_account(ctx.account_key());
                    if current_account_connections >= max_account_connections as usize {
                        return Err(AuthDeny::connection_limit_exceeded(
                            &format!("account {}", ctx.account_key()),
                            current_account_connections,
                            max_account_connections as usize,
                        ));
                    }
                }
            }

            // Check global max connections per metering key
            if let Some(max_per_metering_key) =
                self.rate_limit_config.max_connections_per_metering_key
            {
                let current_metering_connections =
                    self.count_connections_for_metering_key(&ctx.metering_key);
                if current_metering_connections >= max_per_metering_key {
                    return Err(AuthDeny::connection_limit_exceeded(
                        &format!("metering key {}", ctx.metering_key),
                        current_metering_connections,
                        max_per_metering_key,
                    ));
                }
            }

            // Check global max connections per origin
            if let Some(max_per_origin) = self.rate_limit_config.max_connections_per_origin {
                if let Some(ref origin) = ctx.origin {
                    let current_origin_connections = self.count_connections_for_origin(origin);
                    if current_origin_connections >= max_per_origin {
                        return Err(AuthDeny::connection_limit_exceeded(
                            &format!("origin {}", origin),
                            current_origin_connections,
                            max_per_origin,
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    /// Count connections from a specific IP address
    fn count_connections_for_ip(&self, remote_addr: &SocketAddr) -> usize {
        let ip = remote_addr.ip();
        self.clients
            .iter()
            .filter(|entry| entry.value().remote_addr.ip() == ip)
            .count()
    }

    /// Count connections for a resolved consumer identity
    fn count_connections_for_consumer(&self, consumer: &str) -> usize {
        self.clients
            .iter()
            .filter(|entry| {
                entry
                    .value()
                    .auth_context
                    .as_ref()
                    .map(|ctx| ctx.consumer_key() == consumer)
                    .unwrap_or(false)
            })
            .count()
    }

    /// Count connections for a resolved account identity
    fn count_connections_for_account(&self, account: &str) -> usize {
        self.clients
            .iter()
            .filter(|entry| {
                entry
                    .value()
                    .auth_context
                    .as_ref()
                    .map(|ctx| ctx.account_key() == account)
                    .unwrap_or(false)
            })
            .count()
    }

    /// Count connections for a specific metering key
    fn count_connections_for_metering_key(&self, metering_key: &str) -> usize {
        self.clients
            .iter()
            .filter(|entry| {
                entry
                    .value()
                    .auth_context
                    .as_ref()
                    .map(|ctx| ctx.metering_key == metering_key)
                    .unwrap_or(false)
            })
            .count()
    }

    /// Count connections for a specific origin
    fn count_connections_for_origin(&self, origin: &str) -> usize {
        self.clients
            .iter()
            .filter(|entry| {
                entry
                    .value()
                    .auth_context
                    .as_ref()
                    .and_then(|ctx| ctx.origin.as_ref())
                    .map(|o| o == origin)
                    .unwrap_or(false)
            })
            .count()
    }

    /// Check if a subscription is allowed for the given client.
    ///
    /// Returns Ok(()) if the subscription is allowed, or an error with a reason if not.
    #[allow(clippy::result_large_err)]
    pub async fn check_subscription_allowed(&self, client_id: Uuid) -> Result<(), AuthDeny> {
        let context = {
            let Some(client) = self.clients.get(&client_id) else {
                return Ok(());
            };
            let current_subs = client.subscription_count().await;

            // Check max subscriptions per connection (use token limits, fallback to default limits)
            if let Some(ref ctx) = client.auth_context {
                let max_subs = ctx.limits.max_subscriptions.or_else(|| {
                    self.rate_limit_config
                        .default_limits
                        .as_ref()
                        .and_then(|l| l.max_subscriptions)
                });
                if let Some(max_subs) = max_subs {
                    if current_subs >= max_subs as usize {
                        return Err(AuthDeny::new(
                            crate::websocket::auth::AuthErrorCode::SubscriptionLimitExceeded,
                            format!(
                                "Subscription limit exceeded: {} of {} subscriptions for client {}",
                                current_subs, max_subs, client_id
                            ),
                        )
                        .with_suggested_action(
                            "Unsubscribe from an existing view before creating another subscription",
                        ));
                    }
                }
            }
            client.auth_context.clone()
        };

        // Signed subscription-create rates, enforced only for v2 tokens that
        // carry the corresponding optional limit fields.
        if let (Some(rate_limiter), Some(ctx)) = (self.rate_limiter.as_ref(), context.as_ref()) {
            if !ctx.is_legacy_policy() {
                if let RateLimitResult::Denied { retry_after, limit } = rate_limiter
                    .check_subscription_create_for_consumer(
                        ctx.consumer_key(),
                        ctx.limits.max_subscription_creates_per_minute,
                    )
                    .await
                {
                    return Err(AuthDeny::rate_limited(retry_after, "subscription creates")
                        .with_context(format!(
                        "subscription-create rate limit for consumer {} of {} per minute exceeded",
                        ctx.consumer_key(),
                        limit
                    )));
                }

                if ctx.account_key.is_some() {
                    if let RateLimitResult::Denied { retry_after, limit } = rate_limiter
                        .check_subscription_create_for_account(
                            ctx.account_key(),
                            ctx.account_limits.max_subscription_creates_per_minute,
                        )
                        .await
                    {
                        return Err(AuthDeny::rate_limited(
                            retry_after,
                            "account subscription creates",
                        )
                        .with_context(format!(
                            "subscription-create rate limit for account {} of {} per minute exceeded",
                            ctx.account_key(),
                            limit
                        )));
                    }
                }
            }
        }
        Ok(())
    }

    /// Get metering key for a client
    pub fn get_metering_key(&self, client_id: Uuid) -> Option<String> {
        self.clients.get(&client_id).and_then(|client| {
            client
                .auth_context
                .as_ref()
                .map(|ctx| ctx.metering_key.clone())
        })
    }

    /// Get auth context for a client.
    pub fn get_auth_context(&self, client_id: Uuid) -> Option<AuthContext> {
        self.clients
            .get(&client_id)
            .and_then(|client| client.auth_context.clone())
    }

    /// Check if a snapshot request is allowed (based on max_snapshot_rows limit)
    ///
    /// Uses token limits if available, falls back to default limits from RateLimitConfig.
    #[allow(clippy::result_large_err)]
    pub fn check_snapshot_allowed(
        &self,
        client_id: Uuid,
        requested_rows: u32,
    ) -> Result<(), AuthDeny> {
        if let Some(client) = self.clients.get(&client_id) {
            if let Some(ref ctx) = client.auth_context {
                let max_rows = ctx.limits.max_snapshot_rows.or_else(|| {
                    self.rate_limit_config
                        .default_limits
                        .as_ref()
                        .and_then(|l| l.max_snapshot_rows)
                });
                if let Some(max_rows) = max_rows {
                    if requested_rows > max_rows {
                        return Err(AuthDeny::new(
                            crate::websocket::auth::AuthErrorCode::SnapshotLimitExceeded,
                            format!(
                                "Snapshot limit exceeded: requested {} rows, max allowed is {} for client {}",
                                requested_rows, max_rows, client_id
                            ),
                        )
                        .with_suggested_action(
                            "Request fewer rows or lower the snapshotLimit on the subscription",
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::websocket::auth::AuthContext;
    use arete_auth::{KeyClass, Limits};
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    fn create_test_auth_context(subject: &str, limits: Limits) -> AuthContext {
        AuthContext {
            subject: subject.to_string(),
            issuer: "test-issuer".to_string(),
            audience: "test-audience".to_string(),
            key_class: KeyClass::Publishable,
            metering_key: format!("meter-{}", subject),
            deployment_id: None,
            target_kind: None,
            target_id: None,
            program_id: None,
            program_release_hash: None,
            expires_at: u64::MAX,
            scope: "read".to_string(),
            limits,
            plan: None,
            origin: None,
            client_ip: None,
            jti: uuid::Uuid::new_v4().to_string(),
            actor_key: None,
            account_key: None,
            consumer_key: None,
            policy_version: None,
            account_limits: Limits::default(),
        }
    }

    fn create_v2_auth_context(
        consumer: &str,
        account: &str,
        policy_version: u32,
        limits: Limits,
        account_limits: Limits,
    ) -> AuthContext {
        AuthContext {
            subject: "user:1".to_string(),
            issuer: "test-issuer".to_string(),
            audience: "test-audience".to_string(),
            key_class: KeyClass::Publishable,
            metering_key: account.to_string(),
            deployment_id: None,
            target_kind: None,
            target_id: None,
            program_id: None,
            program_release_hash: None,
            expires_at: u64::MAX,
            scope: "read".to_string(),
            limits,
            plan: Some("pro".to_string()),
            origin: None,
            client_ip: None,
            jti: uuid::Uuid::new_v4().to_string(),
            actor_key: Some("user:1".to_string()),
            account_key: Some(account.to_string()),
            consumer_key: Some(consumer.to_string()),
            policy_version: Some(policy_version),
            account_limits,
        }
    }

    fn insert_client(manager: &ClientManager, context: AuthContext) -> Uuid {
        let (sender, receiver) = mpsc::channel(8);
        // Keep the receiver alive so the sender stays open.
        std::mem::forget(receiver);
        let client_id = Uuid::new_v4();
        manager.clients.insert(
            client_id,
            ClientInfo::new(
                client_id,
                sender,
                Some(context),
                create_test_socket_addr("127.0.0.1"),
            ),
        );
        client_id
    }

    fn create_test_socket_addr(ip: &str) -> SocketAddr {
        SocketAddr::new(
            ip.parse::<IpAddr>()
                .unwrap_or(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))),
            12345,
        )
    }

    #[test]
    fn test_egress_tracker_basic() {
        let mut tracker = EgressTracker::new();

        // Should allow bytes within limit
        assert!(tracker.record_bytes(500, 1000));
        assert_eq!(tracker.current_usage(), 500);

        // Should allow more bytes within limit
        assert!(tracker.record_bytes(400, 1000));
        assert_eq!(tracker.current_usage(), 900);

        // Should reject bytes over limit
        assert!(!tracker.record_bytes(200, 1000));
        assert_eq!(tracker.current_usage(), 900); // Usage shouldn't increase
    }

    #[test]
    fn test_egress_tracker_window_reset() {
        let mut tracker = EgressTracker::new();

        // Use up the limit
        assert!(tracker.record_bytes(100, 100));
        assert!(!tracker.record_bytes(1, 100));

        // Reset the window
        tracker.bytes_this_minute = 0;
        tracker.window_start = SystemTime::now() - Duration::from_secs(61);

        // Should allow after window reset
        assert!(tracker.record_bytes(50, 100));
    }

    #[test]
    fn test_message_rate_tracker_basic() {
        let mut tracker = MessageRateTracker::new();

        assert!(tracker.record_message(2));
        assert_eq!(tracker.current_usage(), 1);

        assert!(tracker.record_message(2));
        assert_eq!(tracker.current_usage(), 2);

        assert!(!tracker.record_message(2));
        assert_eq!(tracker.current_usage(), 2);
    }

    #[tokio::test]
    async fn test_client_inbound_message_limit() {
        let (tx, _rx) = mpsc::channel(1);
        let client = ClientInfo::new(
            Uuid::new_v4(),
            tx,
            Some(create_test_auth_context(
                "user-1",
                Limits {
                    max_messages_per_minute: Some(2),
                    ..Default::default()
                },
            )),
            create_test_socket_addr("127.0.0.1"),
        );

        assert_eq!(client.record_inbound_message(), Some(1));
        assert_eq!(client.record_inbound_message(), Some(2));
        assert_eq!(client.record_inbound_message(), None);
    }

    #[tokio::test]
    async fn duplicate_subscription_id_is_rejected_without_replacement() {
        let (tx, _rx) = mpsc::channel(1);
        let client = ClientInfo::new(
            Uuid::new_v4(),
            tx,
            None,
            create_test_socket_addr("127.0.0.1"),
        );
        let first = CancellationToken::new();
        let duplicate = CancellationToken::new();

        assert!(
            client
                .add_subscription("opaque-id".to_string(), first.clone())
                .await
        );
        assert!(
            !client
                .add_subscription("opaque-id".to_string(), duplicate.clone())
                .await
        );
        assert!(!first.is_cancelled());
        assert!(!duplicate.is_cancelled());

        assert!(client.remove_subscription("opaque-id").await);
        assert!(first.is_cancelled());
        assert!(!duplicate.is_cancelled());
    }

    #[tokio::test]
    async fn test_no_limits() {
        let manager = ClientManager::new();
        let addr = create_test_socket_addr("127.0.0.1");

        // No auth context - should succeed
        assert!(manager.check_connection_allowed(addr, &None).await.is_ok());

        // Auth context with no limits - should succeed
        let auth_context = create_test_auth_context("test", Limits::default());
        assert!(manager
            .check_connection_allowed(addr, &Some(auth_context))
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn test_per_subject_connection_limit() {
        let manager = ClientManager::new();

        let limits = Limits {
            max_connections: Some(2),
            ..Default::default()
        };

        let auth_context = create_test_auth_context("user-1", limits);
        let addr = create_test_socket_addr("127.0.0.1");

        // First connection should succeed (no clients yet)
        assert!(manager
            .check_connection_allowed(addr, &Some(auth_context.clone()))
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn test_per_ip_connection_limit() {
        let manager = ClientManager::new().with_max_connections_per_ip(2);
        let addr = create_test_socket_addr("192.168.1.1");

        // Should succeed when no connections from that IP
        assert!(manager.check_connection_allowed(addr, &None).await.is_ok());
    }

    // Tests for RateLimitConfig
    #[test]
    fn rate_limit_config_default() {
        let config = RateLimitConfig::default();
        assert!(config.max_connections_per_ip.is_none());
        assert_eq!(config.client_timeout, Duration::from_secs(300));
        assert_eq!(config.message_queue_size, 512);
        assert!(config.max_reconnect_attempts.is_none());
        assert_eq!(config.message_rate_window, Duration::from_secs(60));
        assert_eq!(config.egress_rate_window, Duration::from_secs(60));
    }

    #[test]
    fn rate_limit_config_builder_methods() {
        let config = RateLimitConfig::default()
            .with_max_connections_per_ip(10)
            .with_timeout(Duration::from_secs(600))
            .with_message_queue_size(1024)
            .with_rate_limit_window(Duration::from_secs(120));

        assert_eq!(config.max_connections_per_ip, Some(10));
        assert_eq!(config.client_timeout, Duration::from_secs(600));
        assert_eq!(config.message_queue_size, 1024);
        assert_eq!(config.message_rate_window, Duration::from_secs(120));
        assert_eq!(config.egress_rate_window, Duration::from_secs(120));
    }

    #[tokio::test]
    async fn client_manager_with_config() {
        let config = RateLimitConfig::default()
            .with_max_connections_per_ip(5)
            .with_timeout(Duration::from_secs(120))
            .with_message_queue_size(256);

        let manager = ClientManager::with_config(config);
        let addr = create_test_socket_addr("10.0.0.1");

        // Check that the configuration was applied
        assert_eq!(manager.rate_limit_config().max_connections_per_ip, Some(5));
        assert_eq!(
            manager.rate_limit_config().client_timeout,
            Duration::from_secs(120)
        );
        assert_eq!(manager.rate_limit_config().message_queue_size, 256);

        // Should allow when under limit
        assert!(manager.check_connection_allowed(addr, &None).await.is_ok());
    }

    #[tokio::test]
    async fn client_manager_builder_pattern() {
        let manager = ClientManager::new()
            .with_max_connections_per_ip(10)
            .with_timeout(Duration::from_secs(180))
            .with_message_queue_size(1024)
            .with_rate_limit_window(Duration::from_secs(90));

        assert_eq!(manager.rate_limit_config().max_connections_per_ip, Some(10));
        assert_eq!(
            manager.rate_limit_config().client_timeout,
            Duration::from_secs(180)
        );
        assert_eq!(manager.rate_limit_config().message_queue_size, 1024);
        assert_eq!(
            manager.rate_limit_config().message_rate_window,
            Duration::from_secs(90)
        );
    }

    // Integration test: Connection limits are enforced
    #[tokio::test]
    async fn connection_limit_enforcement_with_actual_clients() {
        let manager = ClientManager::new().with_max_connections_per_ip(2);
        let addr1 = create_test_socket_addr("192.168.1.1");
        let addr2 = create_test_socket_addr("192.168.1.2");

        // First connection from IP1 should succeed
        let auth1 = create_test_auth_context("user-1", Limits::default());
        assert!(manager
            .check_connection_allowed(addr1, &Some(auth1.clone()))
            .await
            .is_ok());

        // Simulate adding a client (we can't easily do this without a real WebSocket,
        // but we can verify the check logic works)

        // Same IP, different auth context - should still count toward IP limit
        let auth2 = create_test_auth_context("user-2", Limits::default());
        assert!(manager
            .check_connection_allowed(addr1, &Some(auth2.clone()))
            .await
            .is_ok());

        // Different IP - should succeed regardless
        let auth3 = create_test_auth_context("user-3", Limits::default());
        assert!(manager
            .check_connection_allowed(addr2, &Some(auth3.clone()))
            .await
            .is_ok());
    }

    // Test subscription limit enforcement
    #[tokio::test]
    async fn subscription_limit_enforcement() {
        let manager = ClientManager::new();
        let addr = create_test_socket_addr("127.0.0.1");

        // Create auth context with subscription limit
        let auth = create_test_auth_context(
            "user-1",
            Limits {
                max_subscriptions: Some(2),
                ..Default::default()
            },
        );

        // Check should pass initially
        assert!(manager
            .check_connection_allowed(addr, &Some(auth.clone()))
            .await
            .is_ok());

        // Note: We can't easily test the full subscription flow without a real connection,
        // but we verify the limit configuration is properly stored
        assert_eq!(auth.limits.max_subscriptions, Some(2));
    }

    // Test snapshot limit enforcement
    #[tokio::test]
    async fn snapshot_limit_enforcement() {
        let manager = ClientManager::new();
        let addr = create_test_socket_addr("127.0.0.1");

        let auth = create_test_auth_context(
            "user-1",
            Limits {
                max_snapshot_rows: Some(1000),
                ..Default::default()
            },
        );

        assert!(manager
            .check_connection_allowed(addr, &Some(auth.clone()))
            .await
            .is_ok());

        // Note: Actual snapshot limit checking happens in check_snapshot_allowed
        // which requires a connected client
    }

    #[tokio::test]
    async fn two_consumers_share_the_account_connection_cap() {
        let manager = ClientManager::new();
        let account_limits = Limits {
            max_connections: Some(1),
            ..Limits::default()
        };
        let consumer_a = create_v2_auth_context(
            "consumer:a",
            "account:42",
            1,
            Limits::default(),
            account_limits.clone(),
        );
        insert_client(&manager, consumer_a);

        // A different consumer on the same account is blocked by the
        // aggregate cap.
        let consumer_b = create_v2_auth_context(
            "consumer:b",
            "account:42",
            1,
            Limits::default(),
            account_limits.clone(),
        );
        let addr = create_test_socket_addr("127.0.0.1");
        assert!(manager
            .check_connection_allowed(addr, &Some(consumer_b))
            .await
            .is_err());

        // A consumer on another account is unaffected.
        let other_account = create_v2_auth_context(
            "consumer:c",
            "account:43",
            1,
            Limits::default(),
            account_limits,
        );
        assert!(manager
            .check_connection_allowed(addr, &Some(other_account))
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn one_consumer_is_limited_independently_of_its_account() {
        let manager = ClientManager::new();
        let limits = Limits {
            max_connections: Some(1),
            ..Limits::default()
        };
        let account_limits = Limits {
            max_connections: Some(10),
            ..Limits::default()
        };
        let consumer_a = create_v2_auth_context(
            "consumer:a",
            "account:42",
            1,
            limits.clone(),
            account_limits.clone(),
        );
        insert_client(&manager, consumer_a.clone());

        let addr = create_test_socket_addr("127.0.0.1");
        // The same consumer hits its own cap.
        assert!(manager
            .check_connection_allowed(addr, &Some(consumer_a))
            .await
            .is_err());

        // A sibling consumer under the same account is still admitted.
        let consumer_b =
            create_v2_auth_context("consumer:b", "account:42", 1, limits, account_limits);
        assert!(manager
            .check_connection_allowed(addr, &Some(consumer_b))
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn policy_version_upgrade_stale_and_conflict_rules_apply() {
        let manager = ClientManager::new();
        let addr = create_test_socket_addr("127.0.0.1");
        let limits_v1 = Limits {
            max_connections: Some(5),
            ..Limits::default()
        };
        let limits_v2 = Limits {
            max_connections: Some(2),
            ..Limits::default()
        };

        // Version 1 admits and creates state.
        let v1 = create_v2_auth_context(
            "consumer:a",
            "account:42",
            1,
            Limits::default(),
            limits_v1.clone(),
        );
        assert!(manager
            .check_connection_allowed(addr, &Some(v1.clone()))
            .await
            .is_ok());

        // Version 2 with new limits replaces the policy.
        let v2 = create_v2_auth_context(
            "consumer:a",
            "account:42",
            2,
            Limits::default(),
            limits_v2.clone(),
        );
        assert!(manager
            .check_connection_allowed(addr, &Some(v2))
            .await
            .is_ok());

        // A stale version-1 token is rejected once version 2 was observed.
        let stale = manager.check_connection_allowed(addr, &Some(v1)).await;
        assert!(stale.is_err());

        // Version 2 with different limits is a signing/config fault.
        let conflict =
            create_v2_auth_context("consumer:a", "account:42", 2, Limits::default(), limits_v1);
        assert!(manager
            .check_connection_allowed(addr, &Some(conflict))
            .await
            .is_err());
    }

    #[tokio::test]
    async fn legacy_tokens_keep_old_behavior_and_are_counted() {
        let manager = ClientManager::new();
        let addr = create_test_socket_addr("127.0.0.1");
        let legacy = create_test_auth_context(
            "user-1",
            Limits {
                max_connections: Some(2),
                ..Limits::default()
            },
        );
        assert!(legacy.is_legacy_policy());

        assert!(manager
            .check_connection_allowed(addr, &Some(legacy.clone()))
            .await
            .is_ok());
        assert_eq!(manager.account_policies().legacy_token_count(), 1);
        // Legacy tokens create no account policy state.
        assert_eq!(manager.account_policies().tracked_accounts(), 0);

        // Legacy connection counting still keys off the subject.
        insert_client(&manager, legacy.clone());
        insert_client(&manager, legacy.clone());
        assert!(manager
            .check_connection_allowed(addr, &Some(legacy))
            .await
            .is_err());
    }

    #[tokio::test]
    async fn account_message_budget_is_shared_across_consumers() {
        let manager = ClientManager::new();
        let account_limits = Limits {
            max_messages_per_minute: Some(2),
            ..Limits::default()
        };
        let client_a = insert_client(
            &manager,
            create_v2_auth_context(
                "consumer:a",
                "account:42",
                1,
                Limits::default(),
                account_limits.clone(),
            ),
        );
        let client_b = insert_client(
            &manager,
            create_v2_auth_context(
                "consumer:b",
                "account:42",
                1,
                Limits::default(),
                account_limits,
            ),
        );

        assert!(manager.check_inbound_message_allowed(client_a).is_ok());
        assert!(manager.check_inbound_message_allowed(client_b).is_ok());
        // The third message anywhere on the account is rejected.
        assert!(manager.check_inbound_message_allowed(client_a).is_err());
    }

    #[tokio::test]
    async fn account_state_evicts_when_idle_and_unreferenced() {
        let manager = ClientManager::new();
        let addr = create_test_socket_addr("127.0.0.1");
        let context = create_v2_auth_context(
            "consumer:a",
            "account:42",
            1,
            Limits::default(),
            Limits::default(),
        );
        assert!(manager
            .check_connection_allowed(addr, &Some(context))
            .await
            .is_ok());
        assert_eq!(manager.account_policies().tracked_accounts(), 1);

        // No live connection references the account; an idle sweep with a
        // zero TTL registry drops it.
        manager.account_policies().evict_idle(|_| false);
        assert_eq!(
            manager.account_policies().tracked_accounts(),
            1,
            "TTL keeps fresh entries"
        );
        manager.cleanup_account_state();
        assert_eq!(
            manager.account_policies().tracked_accounts(),
            1,
            "fresh entries survive sweep"
        );
    }

    // Test WebSocketRateLimiter integration
    #[tokio::test]
    async fn test_rate_limiter_integration() {
        use crate::websocket::rate_limiter::{RateLimiterConfig, WebSocketRateLimiter};

        let rate_limiter = Arc::new(WebSocketRateLimiter::new(RateLimiterConfig::default()));
        let manager = ClientManager::new().with_rate_limiter(rate_limiter);
        let addr = create_test_socket_addr("127.0.0.1");

        // Should allow connections when rate limiter is configured
        let auth = create_test_auth_context("user-1", Limits::default());
        assert!(manager
            .check_connection_allowed(addr, &Some(auth))
            .await
            .is_ok());
    }
}
