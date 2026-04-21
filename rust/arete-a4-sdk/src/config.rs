use crate::auth::AuthConfig;
use crate::store::DEFAULT_MAX_ENTRIES_PER_VIEW;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct AreteConfig {
    pub auto_reconnect: bool,
    pub reconnect_intervals: Vec<Duration>,
    pub max_reconnect_attempts: u32,
    pub ping_interval: Duration,
    pub initial_data_timeout: Duration,
    pub max_entries_per_view: Option<usize>,
    pub auth: Option<AuthConfig>,
}

impl Default for AreteConfig {
    fn default() -> Self {
        Self {
            auto_reconnect: true,
            reconnect_intervals: vec![
                Duration::from_secs(1),
                Duration::from_secs(2),
                Duration::from_secs(4),
                Duration::from_secs(8),
                Duration::from_secs(16),
            ],
            max_reconnect_attempts: 5,
            ping_interval: Duration::from_secs(15),
            initial_data_timeout: Duration::from_secs(5),
            max_entries_per_view: Some(DEFAULT_MAX_ENTRIES_PER_VIEW),
            auth: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ConnectionConfig {
    pub auto_reconnect: bool,
    pub reconnect_intervals: Vec<Duration>,
    pub max_reconnect_attempts: u32,
    pub ping_interval: Duration,
    pub auth: Option<AuthConfig>,
}

impl From<AreteConfig> for ConnectionConfig {
    fn from(config: AreteConfig) -> Self {
        Self {
            auto_reconnect: config.auto_reconnect,
            reconnect_intervals: config.reconnect_intervals,
            max_reconnect_attempts: config.max_reconnect_attempts,
            ping_interval: config.ping_interval,
            auth: config.auth,
        }
    }
}

impl Default for ConnectionConfig {
    fn default() -> Self {
        AreteConfig::default().into()
    }
}
