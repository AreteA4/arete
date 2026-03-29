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

    /// Check if a request is allowed and record it
    fn check_and_record(&mut self, now: Instant) -> RateLimitResult {
        // Remove expired requests outside the window
        let cutoff = now - self.window.window_duration;
        self.requests.retain(|&t| t > cutoff);

        let limit = self.window.max_requests + self.window.burst;
        let current_count = self.requests.len() as u32;

        if current_count >= limit {
            // Calculate retry after time
            if let Some(oldest) = self.requests.first() {
                let retry_after = (*oldest + self.window.window_duration).saturating_duration_since(now);
                RateLimitResult::Denied {
                    retry_after,
                    limit: self.window.max_requests,
                }
            } else {
                RateLimitResult::Denied {
                    retry_after: self.window.window_duration,
                    limit: self.window.max_requests,
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

    /// Peek at current state without recording
    fn peek(&self, now: Instant) -> RateLimitResult {
        let cutoff = now - self.window.window_duration;
        let valid_requests: Vec<_> = self.requests.iter().filter(|&&t| t > cutoff).copied().collect();
        
        let limit = self.window.max_requests + self.window.burst;
        let current_count = valid_requests.len() as u32;

        if current_count >= limit {
            if let Some(oldest) = valid_requests.first() {
                let retry_after = (*oldest + self.window.window_duration).saturating_duration_since(now);
                RateLimitResult::Denied {
                    retry_after,
                    limit: self.window.max_requests,
                }
            } else {
                RateLimitResult::Denied {
                    retry_after: self.window.window_duration,
                    limit: self.window.max_requests,
                }
            }
        } else {
            let reset_at = now + self.window.window_duration;
            RateLimitResult::Allowed {
                remaining: limit - current_count,
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
    /// Rate limit for connection attempts per subject
    pub connections_per_subject: RateLimitWindow,
    /// Rate limit for connection attempts per metering key
    pub connections_per_metering_key: RateLimitWindow,
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
            connections_per_subject: RateLimitWindow::new(30, Duration::from_secs(60)).with_burst(5),
            connections_per_metering_key: RateLimitWindow::new(100, Duration::from_secs(60)).with_burst(20),
            subscriptions_per_connection: RateLimitWindow::new(120, Duration::from_secs(60)).with_burst(10),
            messages_per_connection: RateLimitWindow::new(1000, Duration::from_secs(60)).with_burst(100),
            snapshots_per_connection: RateLimitWindow::new(30, Duration::from_secs(60)).with_burst(5),
            enabled: true,
        }
    }
}

impl RateLimiterConfig {
    /// Load configuration from environment variables
    pub fn from_env() -> Self {
        let mut config = Self::default();

        // Handshake rate limit
        if let (Ok(max), Ok(secs)) = (
            std::env::var("HYPERSTACK_RATE_LIMIT_HANDSHAKE_PER_IP_MAX"),
            std::env::var("HYPERSTACK_RATE_LIMIT_HANDSHAKE_PER_IP_WINDOW_SECS"),
        ) {
            if let (Ok(max), Ok(secs)) = (max.parse(), secs.parse()) {
                config.handshake_per_ip = RateLimitWindow::new(max, Duration::from_secs(secs));
            }
        }

        // Connections per subject
        if let (Ok(max), Ok(secs)) = (
            std::env::var("HYPERSTACK_RATE_LIMIT_CONNECTIONS_PER_SUBJECT_MAX"),
            std::env::var("HYPERSTACK_RATE_LIMIT_CONNECTIONS_PER_SUBJECT_WINDOW_SECS"),
        ) {
            if let (Ok(max), Ok(secs)) = (max.parse(), secs.parse()) {
                config.connections_per_subject = RateLimitWindow::new(max, Duration::from_secs(secs));
            }
        }

        // Connections per metering key
        if let (Ok(max), Ok(secs)) = (
            std::env::var("HYPERSTACK_RATE_LIMIT_CONNECTIONS_PER_METERING_KEY_MAX"),
            std::env::var("HYPERSTACK_RATE_LIMIT_CONNECTIONS_PER_METERING_KEY_WINDOW_SECS"),
        ) {
            if let (Ok(max), Ok(secs)) = (max.parse(), secs.parse()) {
                config.connections_per_metering_key = RateLimitWindow::new(max, Duration::from_secs(secs));
            }
        }

        // Subscriptions per connection
        if let (Ok(max), Ok(secs)) = (
            std::env::var("HYPERSTACK_RATE_LIMIT_SUBSCRIPTIONS_PER_CONNECTION_MAX"),
            std::env::var("HYPERSTACK_RATE_LIMIT_SUBSCRIPTIONS_PER_CONNECTION_WINDOW_SECS"),
        ) {
            if let (Ok(max), Ok(secs)) = (max.parse(), secs.parse()) {
                config.subscriptions_per_connection = RateLimitWindow::new(max, Duration::from_secs(secs));
            }
        }

        // Messages per connection
        if let (Ok(max), Ok(secs)) = (
            std::env::var("HYPERSTACK_RATE_LIMIT_MESSAGES_PER_CONNECTION_MAX"),
            std::env::var("HYPERSTACK_RATE_LIMIT_MESSAGES_PER_CONNECTION_WINDOW_SECS"),
        ) {
            if let (Ok(max), Ok(secs)) = (max.parse(), secs.parse()) {
                config.messages_per_connection = RateLimitWindow::new(max, Duration::from_secs(secs));
            }
        }

        // Snapshots per connection
        if let (Ok(max), Ok(secs)) = (
            std::env::var("HYPERSTACK_RATE_LIMIT_SNAPSHOTS_PER_CONNECTION_MAX"),
            std::env::var("HYPERSTACK_RATE_LIMIT_SNAPSHOTS_PER_CONNECTION_WINDOW_SECS"),
        ) {
            if let (Ok(max), Ok(secs)) = (max.parse(), secs.parse()) {
                config.snapshots_per_connection = RateLimitWindow::new(max, Duration::from_secs(secs));
            }
        }

        // Enable/disable
        if let Ok(enabled) = std::env::var("HYPERSTACK_RATE_LIMITING_ENABLED") {
            config.enabled = enabled.parse().unwrap_or(true);
        }

        config
    }

    /// Disable rate limiting (useful for testing)
    pub fn disabled() -> Self {
        let mut config = Self::default();
        config.enabled = false;
        config
    }
}

/// Multi-tenant rate limiter with per-key tracking
#[derive(Debug)]
pub struct WebSocketRateLimiter {
    config: RateLimiterConfig,
    /// Per-IP handshake rate limits
    ip_buckets: Arc<RwLock<HashMap<String, RateLimitBucket>>>,
    /// Per-subject connection rate limits
    subject_buckets: Arc<RwLock<HashMap<String, RateLimitBucket>>>,
    /// Per-metering-key connection rate limits
    metering_key_buckets: Arc<RwLock<HashMap<String, RateLimitBucket>>>,
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
            subject_buckets: Arc::new(RwLock::new(HashMap::new())),
            metering_key_buckets: Arc::new(RwLock::new(HashMap::new())),
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
        
        let result = bucket.check_and_record(Instant::now());
        
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

    /// Check if connection is allowed for the given subject
    pub async fn check_connection_for_subject(&self, subject: &str) -> RateLimitResult {
        if !self.config.enabled {
            return RateLimitResult::Allowed {
                remaining: u32::MAX,
                reset_at: Instant::now() + Duration::from_secs(60),
            };
        }

        let mut buckets = self.subject_buckets.write().await;
        let bucket = buckets
            .entry(subject.to_string())
            .or_insert_with(|| RateLimitBucket::new(self.config.connections_per_subject));
        
        bucket.check_and_record(Instant::now())
    }

    /// Check if connection is allowed for the given metering key
    pub async fn check_connection_for_metering_key(&self, metering_key: &str) -> RateLimitResult {
        if !self.config.enabled {
            return RateLimitResult::Allowed {
                remaining: u32::MAX,
                reset_at: Instant::now() + Duration::from_secs(60),
            };
        }

        let mut buckets = self.metering_key_buckets.write().await;
        let bucket = buckets
            .entry(metering_key.to_string())
            .or_insert_with(|| RateLimitBucket::new(self.config.connections_per_metering_key));
        
        bucket.check_and_record(Instant::now())
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
        
        bucket.check_and_record(Instant::now())
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
        
        bucket.check_and_record(Instant::now())
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
        
        bucket.check_and_record(Instant::now())
    }

    /// Clean up stale buckets to prevent memory growth
    pub async fn cleanup_stale_buckets(&self) {
        let now = Instant::now();
        let cutoff = now - Duration::from_secs(300); // 5 minutes

        // Clean up IP buckets
        {
            let mut buckets = self.ip_buckets.write().await;
            buckets.retain(|_, bucket| {
                // Keep if any request is recent
                bucket.peek(now);
                !bucket.requests.is_empty()
            });
        }

        // Clean up subject buckets
        {
            let mut buckets = self.subject_buckets.write().await;
            buckets.retain(|_, bucket| {
                bucket.peek(now);
                !bucket.requests.is_empty()
            });
        }

        // Clean up metering key buckets
        {
            let mut buckets = self.metering_key_buckets.write().await;
            buckets.retain(|_, bucket| {
                bucket.peek(now);
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
            subject_buckets: Arc::clone(&self.subject_buckets),
            metering_key_buckets: Arc::clone(&self.metering_key_buckets),
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
            connections_per_subject: RateLimitWindow::new(30, Duration::from_secs(60)).with_burst(5),
            connections_per_metering_key: RateLimitWindow::new(100, Duration::from_secs(60)).with_burst(20),
            subscriptions_per_connection: RateLimitWindow::new(120, Duration::from_secs(60)).with_burst(10),
            messages_per_connection: RateLimitWindow::new(1000, Duration::from_secs(60)).with_burst(100),
            snapshots_per_connection: RateLimitWindow::new(30, Duration::from_secs(60)).with_burst(5),
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
                    assert_eq!(remaining, 4 - i, "Request {} should have {} remaining", i, 4 - i);
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
    async fn test_subject_rate_limiting() {
        let config = RateLimiterConfig {
            connections_per_subject: RateLimitWindow::new(3, Duration::from_secs(60)),
            ..test_config()
        };
        let limiter = WebSocketRateLimiter::new(config);

        // First 3 connections allowed
        for i in 0..3 {
            let result = limiter.check_connection_for_subject("user-123").await;
            assert!(
                matches!(result, RateLimitResult::Allowed { remaining, .. } if remaining == 2 - i),
                "Connection {} should be allowed",
                i
            );
        }

        // Fourth denied
        let result = limiter.check_connection_for_subject("user-123").await;
        assert!(
            matches!(result, RateLimitResult::Denied { .. }),
            "Fourth connection should be denied"
        );

        // Different subject should still work
        let result = limiter.check_connection_for_subject("user-456").await;
        assert!(
            matches!(result, RateLimitResult::Allowed { .. }),
            "Different subject should be allowed"
        );
    }
}
