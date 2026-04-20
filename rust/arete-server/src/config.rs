use std::net::SocketAddr;
use std::time::Duration;

pub use crate::health::HealthConfig;
pub use crate::http_health::HttpHealthConfig;

/// Configuration for gRPC stream reconnection with exponential backoff
#[derive(Clone, Debug)]
pub struct ReconnectionConfig {
    /// Initial delay before first reconnection attempt
    pub initial_delay: Duration,
    /// Maximum delay between reconnection attempts
    pub max_delay: Duration,
    /// Maximum number of reconnection attempts (None = infinite)
    pub max_attempts: Option<u32>,
    /// Multiplier for exponential backoff (typically 2.0)
    pub backoff_multiplier: f64,
    /// HTTP/2 keep-alive interval to prevent silent disconnects
    pub http2_keep_alive_interval: Option<Duration>,
}

impl Default for ReconnectionConfig {
    fn default() -> Self {
        Self {
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(60),
            max_attempts: None, // Infinite retries by default
            backoff_multiplier: 2.0,
            http2_keep_alive_interval: Some(Duration::from_secs(30)),
        }
    }
}

impl ReconnectionConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_initial_delay(mut self, delay: Duration) -> Self {
        self.initial_delay = delay;
        self
    }

    pub fn with_max_delay(mut self, delay: Duration) -> Self {
        self.max_delay = delay;
        self
    }

    pub fn with_max_attempts(mut self, attempts: u32) -> Self {
        self.max_attempts = Some(attempts);
        self
    }

    pub fn with_backoff_multiplier(mut self, multiplier: f64) -> Self {
        self.backoff_multiplier = multiplier;
        self
    }

    pub fn with_http2_keep_alive_interval(mut self, interval: Duration) -> Self {
        self.http2_keep_alive_interval = Some(interval);
        self
    }

    /// Calculate the next backoff duration given the current one
    pub fn next_backoff(&self, current: Duration) -> Duration {
        let next_secs = current.as_secs_f64() * self.backoff_multiplier;
        let capped_secs = next_secs.min(self.max_delay.as_secs_f64());
        Duration::from_secs_f64(capped_secs)
    }
}

/// WebSocket server configuration
#[derive(Clone, Debug)]
pub struct WebSocketConfig {
    pub bind_address: SocketAddr,
}

impl Default for WebSocketConfig {
    fn default() -> Self {
        Self {
            bind_address: "[::]:8877".parse().expect("valid socket address"),
        }
    }
}

impl WebSocketConfig {
    pub fn new(bind_address: impl Into<SocketAddr>) -> Self {
        Self {
            bind_address: bind_address.into(),
        }
    }
}

/// Yellowstone gRPC configuration
#[derive(Clone, Debug)]
pub struct YellowstoneConfig {
    pub endpoint: String,
    pub x_token: Option<String>,
}

impl YellowstoneConfig {
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            x_token: None,
        }
    }

    pub fn with_token(mut self, token: impl Into<String>) -> Self {
        self.x_token = Some(token.into());
        self
    }
}

/// Main server configuration
#[derive(Clone, Debug, Default)]
pub struct ServerConfig {
    pub websocket: Option<WebSocketConfig>,
    pub yellowstone: Option<YellowstoneConfig>,
    pub health: Option<HealthConfig>,
    pub http_health: Option<HttpHealthConfig>,
    pub reconnection: Option<ReconnectionConfig>,
}

impl ServerConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_websocket(mut self, config: WebSocketConfig) -> Self {
        self.websocket = Some(config);
        self
    }

    pub fn with_yellowstone(mut self, config: YellowstoneConfig) -> Self {
        self.yellowstone = Some(config);
        self
    }

    pub fn with_health(mut self, config: HealthConfig) -> Self {
        self.health = Some(config);
        self
    }

    pub fn with_http_health(mut self, config: HttpHealthConfig) -> Self {
        self.http_health = Some(config);
        self
    }

    pub fn with_reconnection(mut self, config: ReconnectionConfig) -> Self {
        self.reconnection = Some(config);
        self
    }
}
