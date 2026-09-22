use std::net::SocketAddr;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use ipnet::IpNet;

pub use crate::health::HealthConfig;
pub use crate::http_health::HttpHealthConfig;
pub use crate::http_server::HttpServerConfig;

/// Runtime capabilities selected for one server process.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RuntimePlan {
    pub health: bool,
    pub chain_reads: bool,
    pub program_reads: bool,
    pub stack_queries: bool,
    pub transactions: bool,
    pub websocket: bool,
    pub live_runtime: bool,
}

impl RuntimePlan {
    /// Health, chain reads, and fixed transaction routes only.
    pub const fn solana_gateway() -> Self {
        Self {
            health: true,
            chain_reads: true,
            program_reads: false,
            stack_queries: false,
            transactions: true,
            websocket: false,
            live_runtime: false,
        }
    }

    pub fn http() -> Self {
        Self {
            health: true,
            chain_reads: true,
            program_reads: true,
            stack_queries: true,
            ..Self::default()
        }
    }

    pub fn program_reads() -> Self {
        Self {
            health: true,
            program_reads: true,
            ..Self::default()
        }
    }

    pub fn live_runtime_enabled(self) -> bool {
        self.live_runtime || self.websocket
    }
}

/// Explicit configuration for the fixed transaction relay routes.
#[derive(Clone, Debug)]
pub struct TransactionConfig {
    pub enabled: bool,
    pub rpc_url: Option<String>,
    /// Development-only: allow relay requests when no auth plugin is configured.
    pub allow_unauthenticated: bool,
    pub max_body_bytes: usize,
    pub max_transaction_bytes: usize,
    pub inspect_timeout: Duration,
    pub send_timeout: Duration,
    pub status_timeout: Duration,
    pub inspect_concurrency: usize,
    pub send_concurrency: usize,
    pub inspect_requests_per_minute: u32,
    pub send_requests_per_minute: u32,
    pub status_requests_per_minute: u32,
    pub trusted_proxy_cidrs: Vec<IpNet>,
    pub usage_enabled: bool,
    pub usage_endpoint: Option<String>,
    pub usage_token: Option<String>,
    pub usage_spool_capacity: usize,
}

impl Default for TransactionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            rpc_url: None,
            allow_unauthenticated: false,
            // Envelope for any relay body, not the transaction itself — `max_transaction_bytes`
            // below still bounds a submit at Solana's packet limit. Sized for the largest
            // legitimate body, which is a 256-signature status batch at roughly 23 KiB; 4 KiB
            // admitted only about 44 signatures and made the advertised batch unreachable.
            max_body_bytes: 32 * 1024,
            // SIMD-0296 raised the packet limit to 4096 bytes for v1 transactions. legacy and v0
            // stay at 1232, enforced by the cluster; lower this to cap submits below the network.
            max_transaction_bytes: 4096,
            inspect_timeout: Duration::from_secs(10),
            send_timeout: Duration::from_secs(15),
            status_timeout: Duration::from_secs(10),
            inspect_concurrency: 64,
            send_concurrency: 16,
            inspect_requests_per_minute: 600,
            send_requests_per_minute: 60,
            status_requests_per_minute: 600,
            trusted_proxy_cidrs: Vec::new(),
            usage_enabled: false,
            usage_endpoint: None,
            usage_token: None,
            usage_spool_capacity: 1_000,
        }
    }
}

impl TransactionConfig {
    /// Load transaction settings. Routes remain disabled unless explicitly enabled.
    pub fn from_env() -> Result<Self> {
        let mut config = Self::default();
        config.enabled = env_bool("ARETE_TRANSACTIONS_ENABLED")?.unwrap_or(false);
        config.rpc_url =
            first_nonempty(&["ARETE_TRANSACTION_RPC_URL", "SOLANA_RPC_URL", "RPC_URL"]);
        config.allow_unauthenticated =
            env_bool("ARETE_TRANSACTIONS_ALLOW_UNAUTHENTICATED")?.unwrap_or(false);
        config.max_body_bytes =
            env_parse("ARETE_TRANSACTION_MAX_BODY_BYTES")?.unwrap_or(config.max_body_bytes);
        config.max_transaction_bytes =
            env_parse("ARETE_TRANSACTION_MAX_BYTES")?.unwrap_or(config.max_transaction_bytes);
        config.inspect_timeout = Duration::from_millis(
            env_parse("ARETE_TRANSACTION_INSPECT_TIMEOUT_MS")?
                .unwrap_or(config.inspect_timeout.as_millis() as u64),
        );
        config.send_timeout = Duration::from_millis(
            env_parse("ARETE_TRANSACTION_SEND_TIMEOUT_MS")?
                .unwrap_or(config.send_timeout.as_millis() as u64),
        );
        config.status_timeout = Duration::from_millis(
            env_parse("ARETE_TRANSACTION_STATUS_TIMEOUT_MS")?
                .unwrap_or(config.status_timeout.as_millis() as u64),
        );
        config.inspect_concurrency = env_parse("ARETE_TRANSACTION_INSPECT_CONCURRENCY")?
            .unwrap_or(config.inspect_concurrency);
        config.send_concurrency =
            env_parse("ARETE_TRANSACTION_SEND_CONCURRENCY")?.unwrap_or(config.send_concurrency);
        config.inspect_requests_per_minute =
            env_parse("ARETE_TRANSACTION_INSPECT_REQUESTS_PER_MINUTE")?
                .unwrap_or(config.inspect_requests_per_minute);
        config.send_requests_per_minute = env_parse("ARETE_TRANSACTION_SEND_REQUESTS_PER_MINUTE")?
            .unwrap_or(config.send_requests_per_minute);
        config.status_requests_per_minute =
            env_parse("ARETE_TRANSACTION_STATUS_REQUESTS_PER_MINUTE")?
                .unwrap_or(config.status_requests_per_minute);
        config.trusted_proxy_cidrs = std::env::var("ARETE_TRUSTED_PROXY_CIDRS")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .map(|value| {
                value
                    .split(',')
                    .map(|cidr| cidr.trim().parse().context("invalid trusted proxy CIDR"))
                    .collect::<Result<Vec<_>>>()
            })
            .transpose()?
            .unwrap_or_default();
        config.usage_enabled = env_bool("ARETE_TRANSACTION_USAGE_ENABLED")?.unwrap_or(false);
        config.usage_endpoint = first_nonempty(&["ARETE_TRANSACTION_USAGE_ENDPOINT"]);
        config.usage_token = first_nonempty(&["ARETE_TRANSACTION_USAGE_TOKEN"]);
        config.usage_spool_capacity = env_parse("ARETE_TRANSACTION_USAGE_SPOOL_CAPACITY")?
            .unwrap_or(config.usage_spool_capacity);
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        if self.enabled && self.rpc_url.as_deref().unwrap_or_default().is_empty() {
            bail!("transactions are enabled but no transaction RPC URL is configured");
        }
        if self.max_body_bytes == 0
            || self.max_transaction_bytes == 0
            || self.inspect_concurrency == 0
            || self.send_concurrency == 0
        {
            bail!("transaction size and concurrency limits must be greater than zero");
        }
        if self.usage_enabled && (self.usage_endpoint.is_none() || self.usage_token.is_none()) {
            bail!("transaction usage requires an endpoint and token");
        }
        Ok(())
    }
}

fn first_nonempty(keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| std::env::var(key).ok())
        .filter(|value| !value.trim().is_empty())
}

pub(crate) fn env_parse<T>(key: &str) -> Result<Option<T>>
where
    T: std::str::FromStr,
    T::Err: std::error::Error + Send + Sync + 'static,
{
    std::env::var(key)
        .ok()
        .map(|value| value.parse().with_context(|| format!("invalid {key}")))
        .transpose()
}

pub(crate) fn env_bool(key: &str) -> Result<Option<bool>> {
    let Some(value) = std::env::var(key).ok() else {
        return Ok(None);
    };
    match value.to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" => Ok(Some(true)),
        "false" | "0" | "no" => Ok(Some(false)),
        _ => bail!("invalid {key}; expected true or false"),
    }
}

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
    /// Consecutive short-lived connections before a runtime with no snapshot
    /// to replay gives up on its checkpoint and subscribes live.
    ///
    /// The fallback trades data for availability: every slot between the
    /// checkpoint and the live tip is lost. `None` refuses that trade and
    /// keeps retrying from the checkpoint, which is what a recorder wants.
    pub live_fallback_attempts: Option<u32>,
}

impl Default for ReconnectionConfig {
    fn default() -> Self {
        Self {
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(60),
            max_attempts: None, // Infinite retries by default
            backoff_multiplier: 2.0,
            http2_keep_alive_interval: Some(Duration::from_secs(30)),
            live_fallback_attempts: Some(3),
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

    /// Never abandon the resume checkpoint, even if the provider keeps
    /// refusing it. Ingestion stalls rather than silently skipping slots.
    pub fn fail_closed(mut self) -> Self {
        self.live_fallback_attempts = None;
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

/// Buffering and latest-state delivery controls for WebSocket subscriptions.
///
/// These settings are separate from [`WebSocketConfig`] because embedded hosts
/// serve accepted connections without binding a listener of their own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebSocketDeliveryConfig {
    /// Source frames retained for each active list-view broadcast bus.
    pub list_bus_capacity: usize,
    /// Default fixed flush cadence for latest-state collection subscriptions.
    /// `None` preserves immediate delivery unless a view overrides it.
    pub collection_coalesce_ms: Option<u64>,
}

impl Default for WebSocketDeliveryConfig {
    fn default() -> Self {
        Self {
            // Shared per active view rather than allocated per client.
            list_bus_capacity: 8 * 1024,
            collection_coalesce_ms: None,
        }
    }
}

impl WebSocketDeliveryConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        let mut config = Self::default();
        if let Ok(value) = std::env::var("ARETE_WS_LIST_BUS_CAPACITY") {
            config.list_bus_capacity = value.parse().map_err(|_| {
                anyhow::anyhow!("ARETE_WS_LIST_BUS_CAPACITY must be a positive integer")
            })?;
        }
        if let Ok(value) = std::env::var("ARETE_WS_COLLECTION_COALESCE_MS") {
            let milliseconds: u64 = value.parse().map_err(|_| {
                anyhow::anyhow!("ARETE_WS_COLLECTION_COALESCE_MS must be a non-negative integer")
            })?;
            config.collection_coalesce_ms = (milliseconds > 0).then_some(milliseconds);
        }
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.list_bus_capacity > 0,
            "WebSocket list bus capacity must be greater than zero"
        );
        anyhow::ensure!(
            self.collection_coalesce_ms
                .is_none_or(|milliseconds| milliseconds <= 60_000),
            "WebSocket collection coalescing must not exceed 60000ms"
        );
        Ok(())
    }
}

/// The only wire protocol accepted by the WebSocket server.
pub const WEBSOCKET_PROTOCOL_VERSION: u8 = crate::websocket::subscription::PROTOCOL_VERSION;

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
    pub runtime_plan: RuntimePlan,
    pub websocket: Option<WebSocketConfig>,
    pub yellowstone: Option<YellowstoneConfig>,
    pub health: Option<HealthConfig>,
    pub http_health: Option<HttpHealthConfig>,
    pub reconnection: Option<ReconnectionConfig>,
    pub transactions: Option<TransactionConfig>,
    pub solana_gateway_target_id: Option<String>,
    pub program_read_binding_target_id: Option<String>,
    /// State snapshot settings. `None` falls back to `SnapshotConfig::from_env()`.
    pub snapshots: Option<crate::snapshot::SnapshotConfig>,
    /// Event journal settings. `None` falls back to `JournalConfig::from_env()`.
    ///
    /// Set this per runtime when several deployments share one process: the
    /// env vars are process-wide, so they cannot enable replay for one stack
    /// or size a busy stack differently from a quiet one.
    pub journal: Option<crate::journal::JournalConfig>,
    /// WebSocket buffering and latest-state delivery settings. `None` falls
    /// back to [`WebSocketDeliveryConfig::from_env`].
    pub websocket_delivery: Option<WebSocketDeliveryConfig>,
}

impl ServerConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_websocket(mut self, config: WebSocketConfig) -> Self {
        self.websocket = Some(config);
        self.runtime_plan.websocket = true;
        self.runtime_plan.live_runtime = true;
        self
    }

    pub fn with_websocket_delivery(mut self, config: WebSocketDeliveryConfig) -> Self {
        self.websocket_delivery = Some(config);
        self.runtime_plan.live_runtime = true;
        self
    }

    pub fn with_yellowstone(mut self, config: YellowstoneConfig) -> Self {
        self.yellowstone = Some(config);
        self.runtime_plan.live_runtime = true;
        self
    }

    pub fn with_health(mut self, config: HealthConfig) -> Self {
        self.health = Some(config);
        self.runtime_plan.health = true;
        self
    }

    pub fn with_http_health(mut self, config: HttpHealthConfig) -> Self {
        self.http_health = Some(config);
        self.runtime_plan.health = true;
        self.runtime_plan.chain_reads = true;
        self.runtime_plan.program_reads = true;
        self.runtime_plan.stack_queries = true;
        self
    }

    pub fn with_reconnection(mut self, config: ReconnectionConfig) -> Self {
        self.reconnection = Some(config);
        self
    }

    pub fn with_transactions(mut self, config: TransactionConfig) -> Self {
        self.runtime_plan.transactions = config.enabled;
        self.transactions = Some(config);
        self
    }

    pub fn with_runtime_plan(mut self, runtime_plan: RuntimePlan) -> Self {
        self.runtime_plan = runtime_plan;
        self
    }

    pub fn with_snapshots(mut self, config: crate::snapshot::SnapshotConfig) -> Self {
        self.snapshots = Some(config);
        self
    }
}
