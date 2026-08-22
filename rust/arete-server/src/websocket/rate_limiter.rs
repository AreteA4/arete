use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{debug, warn};

/// Rate limit window configuration
#[derive(Debug, Clone, Copy)]
pub struct RateLimitWindow {
    /// Maximum number of requests allowed in the window
    pub max_requests: u32,
    /// Window duration
    pub window_duration: Duration,
    /// Burst allowance (extra requests allowed temporarily)
    pub burst: u32,
}

impl RateLimitWindow {
    /// Create a new rate limit window
    pub fn new(max_requests: u32, window_duration: Duration) -> Self {
        Self {
            max_requests,
            window_duration,
            burst: 0,
        }
    }

    /// Add burst allowance
    pub fn with_burst(mut self, burst: u32) -> Self {
        self.burst = burst;
        self
    }
}

impl Default for RateLimitWindow {
    fn default() -> Self {
        Self {
            max_requests: 100,
            window_duration: Duration::from_secs(60),
            burst: 10,
        }
    }
}

/// Rate limit result
#[derive(Debug, Clone)]
pub enum RateLimitResult {
    /// Request is allowed
    Allowed { remaining: u32, reset_at: Instant },
    /// Request is denied due to rate limiting
    Denied { retry_after: Duration, limit: u32 },
}

/// A single rate limit bucket using sliding window algorithm
#[derive(Debug)]
struct RateLimitBucket {
    /// Request timestamps in the current window
    requests: Vec<Instant>,
    /// Window configuration
    window: RateLimitWindow,
}

impl RateLimitBucket {
    fn new(window: RateLimitWindow) -> Self {
        Self {
            requests: Vec::with_capacity((window.max_requests + window.burst) as usize),
            window,
        }
    }

    fn prune_expired(&mut self, now: Instant) {
        let cutoff = now - self.window.window_duration;
        self.requests.retain(|&t| t > cutoff);
    }

    /// Check if a request is allowed and record it.
    ///
    /// `limit_override` replaces the configured max+burst with a signed
    /// per-token limit (no additional burst) when present.
    fn check_and_record(&mut self, now: Instant, limit_override: Option<u32>) -> RateLimitResult {
        self.prune_expired(now);

        let limit = limit_override.unwrap_or(self.window.max_requests + self.window.burst);
        let current_count = self.requests.len() as u32;

        if current_count >= limit {
            let reported_limit = limit_override.unwrap_or(self.window.max_requests);
            // Calculate retry after time
            if let Some(oldest) = self.requests.first() {
                let retry_after =
                    (*oldest + self.window.window_duration).saturating_duration_since(now);
                RateLimitResult::Denied {
                    retry_after,
                    limit: reported_limit,
                }
            } else {
                RateLimitResult::Denied {
                    retry_after: self.window.window_duration,
                    limit: reported_limit,
                }
            }
        } else {
            self.requests.push(now);
            let reset_at = now + self.window.window_duration;
            RateLimitResult::Allowed {
                remaining: limit - current_count - 1,
                reset_at,
            }
        }
    }
}

/// Rate limiter configuration per key type
#[derive(Debug, Clone)]
pub struct RateLimiterConfig {
    /// Rate limit for handshake attempts per IP
    pub handshake_per_ip: RateLimitWindow,
    /// Rate limit for connection attempts per resolved consumer
    /// (falls back to the token subject for legacy tokens)
    pub connections_per_consumer: RateLimitWindow,
    /// Rate limit for connection attempts per resolved account
    /// (falls back to the metering key for legacy tokens)
    pub connections_per_account: RateLimitWindow,
    /// Rate limit for subscription requests per connection
    pub subscriptions_per_connection: RateLimitWindow,
    /// Rate limit for messages per connection
    pub messages_per_connection: RateLimitWindow,
    /// Rate limit for snapshot requests per connection
    pub snapshots_per_connection: RateLimitWindow,
    /// Enable rate limiting (can be disabled for testing)
    pub enabled: bool,
}

impl Default for RateLimiterConfig {
    fn default() -> Self {
        Self {
            handshake_per_ip: RateLimitWindow::new(60, Duration::from_secs(60)).with_burst(10),
            connections_per_consumer: RateLimitWindow::new(30, Duration::from_secs(60))
                .with_burst(5),
            connections_per_account: RateLimitWindow::new(100, Duration::from_secs(60))
                .with_burst(20),
            subscriptions_per_connection: RateLimitWindow::new(120, Duration::from_secs(60))
                .with_burst(10),
            messages_per_connection: RateLimitWindow::new(1000, Duration::from_secs(60))
                .with_burst(100),
            snapshots_per_connection: RateLimitWindow::new(30, Duration::from_secs(60))
                .with_burst(5),
            enabled: true,
        }
    }
}

impl RateLimiterConfig {
    /// Load a rate limit window from `{prefix}_MAX` and `{prefix}_WINDOW_SECS`.
    fn window_from_env(prefix: &str) -> Option<RateLimitWindow> {
        let max = std::env::var(format!("{prefix}_MAX")).ok()?.parse().ok()?;
        let secs = std::env::var(format!("{prefix}_WINDOW_SECS"))
            .ok()?
            .parse()
            .ok()?;
        Some(RateLimitWindow::new(max, Duration::from_secs(secs)))
    }

    /// Load a window from its current env prefix, falling back to a
    /// deprecated alias with a startup warning.
    fn window_from_env_with_alias(prefix: &str, deprecated: &str) -> Option<RateLimitWindow> {
        if let Some(window) = Self::window_from_env(prefix) {
            return Some(window);
        }
        let window = Self::window_from_env(deprecated)?;
        tracing::warn!(
            deprecated_prefix = deprecated,
            replacement_prefix = prefix,
            "deprecated rate-limit environment variables are set; \
             rename them before the alias is removed"
        );
        Some(window)
    }

    /// Load configuration from environment variables
    pub fn from_env() -> Self {
        let mut config = Self::default();

        // Handshake rate limit
        if let Some(window) = Self::window_from_env("ARETE_RATE_LIMIT_HANDSHAKE_PER_IP") {
            config.handshake_per_ip = window;
        }

        // Connection attempts per resolved consumer
        // (deprecated alias: per-subject variables)
        if let Some(window) = Self::window_from_env_with_alias(
            "ARETE_RATE_LIMIT_CONNECTIONS_PER_CONSUMER",
            "ARETE_RATE_LIMIT_CONNECTIONS_PER_SUBJECT",
        ) {
            config.connections_per_consumer = window;
        }

        // Connection attempts per resolved account
        // (deprecated alias: per-metering-key variables)
        if let Some(window) = Self::window_from_env_with_alias(
            "ARETE_RATE_LIMIT_CONNECTIONS_PER_ACCOUNT",
            "ARETE_RATE_LIMIT_CONNECTIONS_PER_METERING_KEY",
        ) {
            config.connections_per_account = window;
        }

        // Subscriptions per connection
        if let Some(window) = Self::window_from_env("ARETE_RATE_LIMIT_SUBSCRIPTIONS_PER_CONNECTION")
        {
            config.subscriptions_per_connection = window;
        }

        // Messages per connection
        if let Some(window) = Self::window_from_env("ARETE_RATE_LIMIT_MESSAGES_PER_CONNECTION") {
            config.messages_per_connection = window;
        }

        // Snapshots per connection
        if let Some(window) = Self::window_from_env("ARETE_RATE_LIMIT_SNAPSHOTS_PER_CONNECTION") {
            config.snapshots_per_connection = window;
        }

        // Enable/disable
        if let Ok(enabled) = std::env::var("ARETE_RATE_LIMITING_ENABLED") {
            config.enabled = enabled.parse().unwrap_or(true);
        }

        config
    }

    /// Disable rate limiting (useful for testing)
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            ..Default::default()
        }
    }
}

/// Multi-tenant rate limiter with per-key tracking
#[derive(Debug)]
pub struct WebSocketRateLimiter {
    config: RateLimiterConfig,
    /// Per-IP handshake rate limits
    ip_buckets: Arc<RwLock<HashMap<String, RateLimitBucket>>>,
    /// Per-consumer connection rate limits (subject for legacy tokens)
    consumer_buckets: Arc<RwLock<HashMap<String, RateLimitBucket>>>,
    /// Per-account connection rate limits (metering key for legacy tokens)
    account_buckets: Arc<RwLock<HashMap<String, RateLimitBucket>>>,
    /// Per-consumer subscription-create rate limits (signed limit only)
    consumer_subscription_buckets: Arc<RwLock<HashMap<String, RateLimitBucket>>>,
    /// Per-account subscription-create rate limits (signed limit only)
    account_subscription_buckets: Arc<RwLock<HashMap<String, RateLimitBucket>>>,
    /// Per-connection subscription rate limits
    subscription_buckets: Arc<RwLock<HashMap<uuid::Uuid, RateLimitBucket>>>,
    /// Per-connection message rate limits
    message_buckets: Arc<RwLock<HashMap<uuid::Uuid, RateLimitBucket>>>,
    /// Per-connection snapshot rate limits
    snapshot_buckets: Arc<RwLock<HashMap<uuid::Uuid, RateLimitBucket>>>,
}

impl WebSocketRateLimiter {
    /// Create a new rate limiter with the given configuration
    pub fn new(config: RateLimiterConfig) -> Self {
        Self {
            config,
            ip_buckets: Arc::new(RwLock::new(HashMap::new())),
            consumer_buckets: Arc::new(RwLock::new(HashMap::new())),
            account_buckets: Arc::new(RwLock::new(HashMap::new())),
            consumer_subscription_buckets: Arc::new(RwLock::new(HashMap::new())),
            account_subscription_buckets: Arc::new(RwLock::new(HashMap::new())),
            subscription_buckets: Arc::new(RwLock::new(HashMap::new())),
            message_buckets: Arc::new(RwLock::new(HashMap::new())),
            snapshot_buckets: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Check if handshake is allowed from the given IP
    pub async fn check_handshake(&self, addr: SocketAddr) -> RateLimitResult {
        if !self.config.enabled {
            return RateLimitResult::Allowed {
                remaining: u32::MAX,
                reset_at: Instant::now() + Duration::from_secs(60),
            };
        }

        let ip = addr.ip().to_string();
        let mut buckets = self.ip_buckets.write().await;
        let bucket = buckets
            .entry(ip.clone())
            .or_insert_with(|| RateLimitBucket::new(self.config.handshake_per_ip));

        let result = bucket.check_and_record(Instant::now(), None);

        match &result {
            RateLimitResult::Denied { retry_after, limit } => {
                warn!(
                    ip = %ip,
                    retry_after_secs = retry_after.as_secs(),
                    limit = limit,
                    "Rate limit exceeded for handshake"
                );
            }
            RateLimitResult::Allowed { remaining, .. } => {
                debug!(
                    ip = %ip,
                    remaining = remaining,
                    "Handshake rate limit check passed"
                );
            }
        }

        result
    }

    fn allowed_unlimited() -> RateLimitResult {
        RateLimitResult::Allowed {
            remaining: u32::MAX,
            reset_at: Instant::now() + Duration::from_secs(60),
        }
    }

    async fn check_keyed_bucket(
        &self,
        buckets: &RwLock<HashMap<String, RateLimitBucket>>,
        window: RateLimitWindow,
        key: &str,
        limit_override: Option<u32>,
    ) -> RateLimitResult {
        let mut buckets = buckets.write().await;
        let bucket = buckets
            .entry(key.to_string())
            .or_insert_with(|| RateLimitBucket::new(window));
        bucket.check_and_record(Instant::now(), limit_override)
    }

    /// Check if a connection attempt is allowed for the resolved consumer.
    ///
    /// `limit_override` carries the signed
    /// `limits.max_connection_attempts_per_minute` when present; the
    /// configured window applies otherwise.
    pub async fn check_connection_for_consumer(
        &self,
        consumer: &str,
        limit_override: Option<u32>,
    ) -> RateLimitResult {
        if !self.config.enabled {
            return Self::allowed_unlimited();
        }
        self.check_keyed_bucket(
            &self.consumer_buckets,
            self.config.connections_per_consumer,
            consumer,
            limit_override,
        )
        .await
    }

    /// Check if a connection attempt is allowed for the resolved account.
    ///
    /// `limit_override` carries the signed
    /// `account_limits.max_connection_attempts_per_minute` when present; the
    /// configured window applies otherwise.
    pub async fn check_connection_for_account(
        &self,
        account: &str,
        limit_override: Option<u32>,
    ) -> RateLimitResult {
        if !self.config.enabled {
            return Self::allowed_unlimited();
        }
        self.check_keyed_bucket(
            &self.account_buckets,
            self.config.connections_per_account,
            account,
            limit_override,
        )
        .await
    }

    /// Check the signed per-consumer subscription-create rate.
    ///
    /// Enforced only when the token carries
    /// `limits.max_subscription_creates_per_minute`; a `None` limit is
    /// allowed without creating bucket state.
    pub async fn check_subscription_create_for_consumer(
        &self,
        consumer: &str,
        limit: Option<u32>,
    ) -> RateLimitResult {
        let Some(limit) = limit else {
            return Self::allowed_unlimited();
        };
        if !self.config.enabled {
            return Self::allowed_unlimited();
        }
        self.check_keyed_bucket(
            &self.consumer_subscription_buckets,
            RateLimitWindow::new(limit, Duration::from_secs(60)),
            consumer,
            Some(limit),
        )
        .await
    }

    /// Check the signed per-account subscription-create rate.
    ///
    /// Enforced only when the token carries
    /// `account_limits.max_subscription_creates_per_minute`; a `None` limit
    /// is allowed without creating bucket state.
    pub async fn check_subscription_create_for_account(
        &self,
        account: &str,
        limit: Option<u32>,
    ) -> RateLimitResult {
        let Some(limit) = limit else {
            return Self::allowed_unlimited();
        };
        if !self.config.enabled {
            return Self::allowed_unlimited();
        }
        self.check_keyed_bucket(
            &self.account_subscription_buckets,
            RateLimitWindow::new(limit, Duration::from_secs(60)),
            account,
            Some(limit),
        )
        .await
    }

    /// Check if connection is allowed for the given subject
    #[deprecated(note = "use check_connection_for_consumer with the resolved consumer identity")]
    pub async fn check_connection_for_subject(&self, subject: &str) -> RateLimitResult {
        self.check_connection_for_consumer(subject, None).await
    }

    /// Check if connection is allowed for the given metering key
    #[deprecated(note = "use check_connection_for_account with the resolved account identity")]
    pub async fn check_connection_for_metering_key(&self, metering_key: &str) -> RateLimitResult {
        self.check_connection_for_account(metering_key, None).await
    }

    /// Check if subscription is allowed for the given connection
    pub async fn check_subscription(&self, client_id: uuid::Uuid) -> RateLimitResult {
        if !self.config.enabled {
            return RateLimitResult::Allowed {
                remaining: u32::MAX,
                reset_at: Instant::now() + Duration::from_secs(60),
            };
        }

        let mut buckets = self.subscription_buckets.write().await;
        let bucket = buckets
            .entry(client_id)
            .or_insert_with(|| RateLimitBucket::new(self.config.subscriptions_per_connection));

        bucket.check_and_record(Instant::now(), None)
    }

    /// Check if message is allowed for the given connection
    pub async fn check_message(&self, client_id: uuid::Uuid) -> RateLimitResult {
        if !self.config.enabled {
            return RateLimitResult::Allowed {
                remaining: u32::MAX,
                reset_at: Instant::now() + Duration::from_secs(60),
            };
        }

        let mut buckets = self.message_buckets.write().await;
        let bucket = buckets
            .entry(client_id)
            .or_insert_with(|| RateLimitBucket::new(self.config.messages_per_connection));

        bucket.check_and_record(Instant::now(), None)
    }

    /// Check if snapshot is allowed for the given connection
    pub async fn check_snapshot(&self, client_id: uuid::Uuid) -> RateLimitResult {
        if !self.config.enabled {
            return RateLimitResult::Allowed {
                remaining: u32::MAX,
                reset_at: Instant::now() + Duration::from_secs(60),
            };
        }

        let mut buckets = self.snapshot_buckets.write().await;
        let bucket = buckets
            .entry(client_id)
            .or_insert_with(|| RateLimitBucket::new(self.config.snapshots_per_connection));

        bucket.check_and_record(Instant::now(), None)
    }

    /// Clean up stale buckets to prevent memory growth
    pub async fn cleanup_stale_buckets(&self) {
        let now = Instant::now();

        // Clean up IP buckets
        {
            let mut buckets = self.ip_buckets.write().await;
            buckets.retain(|_, bucket| {
                bucket.prune_expired(now);
                !bucket.requests.is_empty()
            });
        }

        // Clean up consumer/account connection and subscription-create buckets
        for buckets in [
            &self.consumer_buckets,
            &self.account_buckets,
            &self.consumer_subscription_buckets,
            &self.account_subscription_buckets,
        ] {
            let mut buckets = buckets.write().await;
            buckets.retain(|_, bucket| {
                bucket.prune_expired(now);
                !bucket.requests.is_empty()
            });
        }

        // Clean up connection-specific buckets for disconnected clients
        // These should be explicitly removed when clients disconnect
    }

    /// Remove all rate limit buckets for a disconnected client
    pub async fn remove_client_buckets(&self, client_id: uuid::Uuid) {
        let mut sub_buckets = self.subscription_buckets.write().await;
        sub_buckets.remove(&client_id);
        drop(sub_buckets);

        let mut msg_buckets = self.message_buckets.write().await;
        msg_buckets.remove(&client_id);
        drop(msg_buckets);

        let mut snap_buckets = self.snapshot_buckets.write().await;
        snap_buckets.remove(&client_id);
    }

    /// Start a background task to periodically clean up stale buckets
    pub fn start_cleanup_task(&self) {
        let limiter = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            loop {
                interval.tick().await;
                limiter.cleanup_stale_buckets().await;
            }
        });
    }
}

impl Clone for WebSocketRateLimiter {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            ip_buckets: Arc::clone(&self.ip_buckets),
            consumer_buckets: Arc::clone(&self.consumer_buckets),
            account_buckets: Arc::clone(&self.account_buckets),
            consumer_subscription_buckets: Arc::clone(&self.consumer_subscription_buckets),
            account_subscription_buckets: Arc::clone(&self.account_subscription_buckets),
            subscription_buckets: Arc::clone(&self.subscription_buckets),
            message_buckets: Arc::clone(&self.message_buckets),
            snapshot_buckets: Arc::clone(&self.snapshot_buckets),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> RateLimiterConfig {
        RateLimiterConfig {
            enabled: true,
            handshake_per_ip: RateLimitWindow::new(60, Duration::from_secs(60)).with_burst(10),
            connections_per_consumer: RateLimitWindow::new(30, Duration::from_secs(60))
                .with_burst(5),
            connections_per_account: RateLimitWindow::new(100, Duration::from_secs(60))
                .with_burst(20),
            subscriptions_per_connection: RateLimitWindow::new(120, Duration::from_secs(60))
                .with_burst(10),
            messages_per_connection: RateLimitWindow::new(1000, Duration::from_secs(60))
                .with_burst(100),
            snapshots_per_connection: RateLimitWindow::new(30, Duration::from_secs(60))
                .with_burst(5),
        }
    }

    #[tokio::test]
    async fn test_rate_limiter_allows_within_limit() {
        let config = RateLimiterConfig {
            handshake_per_ip: RateLimitWindow::new(5, Duration::from_secs(60)),
            ..test_config()
        };
        let limiter = WebSocketRateLimiter::new(config);

        let addr: SocketAddr = "127.0.0.1:12345".parse().unwrap();

        // Should allow first 5 requests
        for i in 0..5 {
            let result = limiter.check_handshake(addr).await;
            match result {
                RateLimitResult::Allowed { remaining, .. } => {
                    assert_eq!(
                        remaining,
                        4 - i,
                        "Request {} should have {} remaining",
                        i,
                        4 - i
                    );
                }
                RateLimitResult::Denied { .. } => {
                    panic!("Request {} should be allowed", i);
                }
            }
        }
    }

    #[tokio::test]
    async fn test_rate_limiter_denies_over_limit() {
        let config = RateLimiterConfig {
            handshake_per_ip: RateLimitWindow::new(2, Duration::from_secs(60)),
            ..test_config()
        };
        let limiter = WebSocketRateLimiter::new(config);

        let addr: SocketAddr = "127.0.0.1:12345".parse().unwrap();

        // First 2 should be allowed
        limiter.check_handshake(addr).await;
        limiter.check_handshake(addr).await;

        // Third should be denied
        let result = limiter.check_handshake(addr).await;
        assert!(
            matches!(result, RateLimitResult::Denied { .. }),
            "Third request should be denied"
        );
    }

    #[tokio::test]
    async fn test_rate_limiter_with_burst() {
        let config = RateLimiterConfig {
            handshake_per_ip: RateLimitWindow::new(2, Duration::from_secs(60)).with_burst(2),
            ..test_config()
        };
        let limiter = WebSocketRateLimiter::new(config);

        let addr: SocketAddr = "127.0.0.1:12345".parse().unwrap();

        // First 4 should be allowed (2 base + 2 burst)
        for i in 0..4 {
            let result = limiter.check_handshake(addr).await;
            assert!(
                matches!(result, RateLimitResult::Allowed { .. }),
                "Request {} should be allowed with burst",
                i
            );
        }

        // Fifth should be denied
        let result = limiter.check_handshake(addr).await;
        assert!(
            matches!(result, RateLimitResult::Denied { .. }),
            "Fifth request should be denied"
        );
    }

    #[tokio::test]
    async fn test_rate_limiter_disabled() {
        let limiter = WebSocketRateLimiter::new(RateLimiterConfig::disabled());

        let addr: SocketAddr = "127.0.0.1:12345".parse().unwrap();

        // Should allow unlimited when disabled
        for _ in 0..100 {
            let result = limiter.check_handshake(addr).await;
            assert!(
                matches!(result, RateLimitResult::Allowed { .. }),
                "Should be allowed when disabled"
            );
        }
    }

    #[tokio::test]
    async fn test_consumer_rate_limiting() {
        let config = RateLimiterConfig {
            connections_per_consumer: RateLimitWindow::new(3, Duration::from_secs(60)),
            ..test_config()
        };
        let limiter = WebSocketRateLimiter::new(config);

        // First 3 connections allowed
        for i in 0..3 {
            let result = limiter
                .check_connection_for_consumer("user-123", None)
                .await;
            assert!(
                matches!(result, RateLimitResult::Allowed { remaining, .. } if remaining == 2 - i),
                "Connection {} should be allowed",
                i
            );
        }

        // Fourth denied
        let result = limiter
            .check_connection_for_consumer("user-123", None)
            .await;
        assert!(
            matches!(result, RateLimitResult::Denied { .. }),
            "Fourth connection should be denied"
        );

        // Different consumer should still work
        let result = limiter
            .check_connection_for_consumer("user-456", None)
            .await;
        assert!(
            matches!(result, RateLimitResult::Allowed { .. }),
            "Different consumer should be allowed"
        );
    }

    #[tokio::test]
    async fn signed_limits_override_configured_connection_windows() {
        let limiter = WebSocketRateLimiter::new(test_config());

        // The signed account limit (2/min) wins over the configured window.
        for _ in 0..2 {
            assert!(matches!(
                limiter
                    .check_connection_for_account("account:42", Some(2))
                    .await,
                RateLimitResult::Allowed { .. }
            ));
        }
        assert!(matches!(
            limiter
                .check_connection_for_account("account:42", Some(2))
                .await,
            RateLimitResult::Denied { .. }
        ));

        // A different account is unaffected.
        assert!(matches!(
            limiter
                .check_connection_for_account("account:43", Some(2))
                .await,
            RateLimitResult::Allowed { .. }
        ));
    }

    #[tokio::test]
    async fn subscription_create_limits_apply_only_when_signed() {
        let limiter = WebSocketRateLimiter::new(test_config());

        // No signed limit: allowed and no state is created.
        for _ in 0..10 {
            assert!(matches!(
                limiter
                    .check_subscription_create_for_account("account:42", None)
                    .await,
                RateLimitResult::Allowed { .. }
            ));
        }
        assert!(limiter.account_subscription_buckets.read().await.is_empty());

        // Signed limit of 1/min: second create denied, other accounts fine.
        assert!(matches!(
            limiter
                .check_subscription_create_for_account("account:42", Some(1))
                .await,
            RateLimitResult::Allowed { .. }
        ));
        assert!(matches!(
            limiter
                .check_subscription_create_for_account("account:42", Some(1))
                .await,
            RateLimitResult::Denied { .. }
        ));
        assert!(matches!(
            limiter
                .check_subscription_create_for_consumer("consumer:a", Some(1))
                .await,
            RateLimitResult::Allowed { .. }
        ));
    }

    #[tokio::test]
    async fn test_cleanup_stale_buckets_removes_expired_buckets() {
        let limiter = WebSocketRateLimiter::new(test_config());
        let stale_request = Instant::now() - Duration::from_secs(600);

        {
            let mut buckets = limiter.ip_buckets.write().await;
            let mut bucket = RateLimitBucket::new(limiter.config.handshake_per_ip);
            bucket.requests.push(stale_request);
            buckets.insert("127.0.0.1".to_string(), bucket);
        }

        {
            let mut buckets = limiter.consumer_buckets.write().await;
            let mut bucket = RateLimitBucket::new(limiter.config.connections_per_consumer);
            bucket.requests.push(stale_request);
            buckets.insert("user-123".to_string(), bucket);
        }

        {
            let mut buckets = limiter.account_buckets.write().await;
            let mut bucket = RateLimitBucket::new(limiter.config.connections_per_account);
            bucket.requests.push(stale_request);
            buckets.insert("account-123".to_string(), bucket);
        }

        limiter.cleanup_stale_buckets().await;

        assert!(limiter.ip_buckets.read().await.is_empty());
        assert!(limiter.consumer_buckets.read().await.is_empty());
        assert!(limiter.account_buckets.read().await.is_empty());
    }

    #[tokio::test]
    async fn test_remove_client_buckets_clears_connection_specific_state() {
        let limiter = WebSocketRateLimiter::new(test_config());
        let client_id = uuid::Uuid::new_v4();

        let _ = limiter.check_subscription(client_id).await;
        let _ = limiter.check_message(client_id).await;
        let _ = limiter.check_snapshot(client_id).await;

        assert!(limiter
            .subscription_buckets
            .read()
            .await
            .contains_key(&client_id));
        assert!(limiter
            .message_buckets
            .read()
            .await
            .contains_key(&client_id));
        assert!(limiter
            .snapshot_buckets
            .read()
            .await
            .contains_key(&client_id));

        limiter.remove_client_buckets(client_id).await;

        assert!(!limiter
            .subscription_buckets
            .read()
            .await
            .contains_key(&client_id));
        assert!(!limiter
            .message_buckets
            .read()
            .await
            .contains_key(&client_id));
        assert!(!limiter
            .snapshot_buckets
            .read()
            .await
            .contains_key(&client_id));
    }
}
