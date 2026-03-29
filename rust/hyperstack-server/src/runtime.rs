use crate::bus::BusManager;
use crate::cache::EntityCache;
use crate::config::ServerConfig;
use crate::health::HealthMonitor;
use crate::http_health::HttpHealthServer;
use crate::materialized_view::MaterializedViewRegistry;
use crate::mutation_batch::MutationBatch;
use crate::projector::Projector;
use crate::view::ViewIndex;
use crate::websocket::client_manager::RateLimitConfig;
use crate::websocket::WebSocketServer;
use crate::Spec;
use crate::WebSocketAuthPlugin;
use crate::WebSocketUsageEmitter;
use anyhow::Result;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{error, info, info_span, Instrument};

#[cfg(feature = "otel")]
use crate::metrics::Metrics;

/// Wait for shutdown signal (SIGINT on all platforms, SIGTERM on Unix)
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("Failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            info!("Received SIGINT (Ctrl+C), initiating shutdown");
        }
        _ = terminate => {
            info!("Received SIGTERM, initiating graceful shutdown");
        }
    }
}

pub struct Runtime {
    config: ServerConfig,
    view_index: Arc<ViewIndex>,
    spec: Option<Spec>,
    materialized_views: Option<MaterializedViewRegistry>,
    websocket_auth_plugin: Option<Arc<dyn WebSocketAuthPlugin>>,
    websocket_usage_emitter: Option<Arc<dyn WebSocketUsageEmitter>>,
    websocket_max_clients: Option<usize>,
    websocket_rate_limit_config: Option<RateLimitConfig>,
    #[cfg(feature = "otel")]
    metrics: Option<Arc<Metrics>>,
}

impl Runtime {
    #[cfg(feature = "otel")]
    pub fn new(config: ServerConfig, view_index: ViewIndex, metrics: Option<Arc<Metrics>>) -> Self {
        Self {
            config,
            view_index: Arc::new(view_index),
            spec: None,
            materialized_views: None,
            websocket_auth_plugin: None,
            websocket_usage_emitter: None,
            websocket_max_clients: None,
            websocket_rate_limit_config: None,
            metrics,
        }
    }

    #[cfg(not(feature = "otel"))]
    pub fn new(config: ServerConfig, view_index: ViewIndex) -> Self {
        Self {
            config,
            view_index: Arc::new(view_index),
            spec: None,
            materialized_views: None,
            websocket_auth_plugin: None,
            websocket_usage_emitter: None,
            websocket_max_clients: None,
            websocket_rate_limit_config: None,
        }
    }

    pub fn with_spec(mut self, spec: Spec) -> Self {
        self.spec = Some(spec);
        self
    }

    pub fn with_materialized_views(mut self, registry: MaterializedViewRegistry) -> Self {
        self.materialized_views = Some(registry);
        self
    }

    pub fn with_websocket_auth_plugin(
        mut self,
        websocket_auth_plugin: Arc<dyn WebSocketAuthPlugin>,
    ) -> Self {
        self.websocket_auth_plugin = Some(websocket_auth_plugin);
        self
    }

    pub fn with_websocket_usage_emitter(
        mut self,
        websocket_usage_emitter: Arc<dyn WebSocketUsageEmitter>,
    ) -> Self {
        self.websocket_usage_emitter = Some(websocket_usage_emitter);
        self
    }

    pub fn with_websocket_max_clients(mut self, websocket_max_clients: usize) -> Self {
        self.websocket_max_clients = Some(websocket_max_clients);
        self
    }

    /// Configure rate limiting for WebSocket connections.
    ///
    /// This sets global rate limits such as maximum connections per IP,
    /// timeouts, and rate windows. Per-subject limits are controlled
    /// via AuthContext.Limits from the authentication token.
    pub fn with_websocket_rate_limit_config(
        mut self,
        config: RateLimitConfig,
    ) -> Self {
        self.websocket_rate_limit_config = Some(config);
        self
    }

    pub async fn run(self) -> Result<()> {
        info!("Starting HyperStack runtime");

        let (mutations_tx, mutations_rx) = mpsc::channel::<MutationBatch>(1024);

        let bus_manager = BusManager::new();
        let entity_cache = EntityCache::new();

        let health_monitor = if let Some(health_config) = &self.config.health {
            let monitor = HealthMonitor::new(health_config.clone());
            let _health_task = monitor.start().await;
            info!("Health monitoring enabled");
            Some(monitor)
        } else {
            None
        };

        #[cfg(feature = "otel")]
        let projector = Projector::new(
            self.view_index.clone(),
            bus_manager.clone(),
            entity_cache.clone(),
            mutations_rx,
            self.metrics.clone(),
        );
        #[cfg(not(feature = "otel"))]
        let projector = Projector::new(
            self.view_index.clone(),
            bus_manager.clone(),
            entity_cache.clone(),
            mutations_rx,
        );

        let projector_handle = tokio::spawn(
            async move {
                projector.run().await;
            }
            .instrument(info_span!("projector")),
        );

        let ws_handle = if let Some(ws_config) = &self.config.websocket {
            #[cfg(feature = "otel")]
            let mut ws_server = WebSocketServer::new(
                ws_config.bind_address,
                bus_manager.clone(),
                entity_cache.clone(),
                self.view_index.clone(),
                self.metrics.clone(),
            );
            #[cfg(not(feature = "otel"))]
            let mut ws_server = WebSocketServer::new(
                ws_config.bind_address,
                bus_manager.clone(),
                entity_cache.clone(),
                self.view_index.clone(),
            );

            if let Some(max_clients) = self.websocket_max_clients {
                ws_server = ws_server.with_max_clients(max_clients);
            }

            if let Some(plugin) = self.websocket_auth_plugin.clone() {
                ws_server = ws_server.with_auth_plugin(plugin);
            }

            if let Some(emitter) = self.websocket_usage_emitter.clone() {
                ws_server = ws_server.with_usage_emitter(emitter);
            }

            if let Some(rate_limit_config) = self.websocket_rate_limit_config {
                ws_server = ws_server.with_rate_limit_config(rate_limit_config);
            }

            let bind_addr = ws_config.bind_address;
            Some(tokio::spawn(
                async move {
                    if let Err(e) = ws_server.start().await {
                        error!("WebSocket server error: {}", e);
                    }
                }
                .instrument(info_span!("ws.server", %bind_addr)),
            ))
        } else {
            None
        };

        let parser_handle = if let Some(spec) = self.spec {
            if let Some(parser_setup) = spec.parser_setup {
                let program_id = spec
                    .program_ids
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "unknown".to_string());
                info!("Starting Vixen parser runtime for program: {}", program_id);
                let tx = mutations_tx.clone();
                let health = health_monitor.clone();
                let reconnection_config = self.config.reconnection.clone().unwrap_or_default();
                Some(tokio::spawn(
                    async move {
                        if let Err(e) = parser_setup(tx, health, reconnection_config).await {
                            error!("Vixen parser runtime error: {}", e);
                        }
                    }
                    .instrument(info_span!("vixen.parser", %program_id)),
                ))
            } else {
                info!("Spec provided but no parser_setup configured - skipping Vixen runtime");
                None
            }
        } else {
            info!("No spec provided - running in websocket-only mode");
            None
        };

        // Run the HTTP health server on a dedicated OS thread with its own single-threaded
        // tokio runtime. This isolates it from the main runtime so that liveness probes
        // always respond even when the event processing pipeline saturates worker threads
        // (e.g. due to std::sync::Mutex contention on VmContext under high throughput).
        let _http_health_handle = if let Some(http_health_config) = &self.config.http_health {
            let mut http_server = HttpHealthServer::new(http_health_config.bind_address);
            if let Some(monitor) = health_monitor.clone() {
                http_server = http_server.with_health_monitor(monitor);
            }

            let bind_addr = http_health_config.bind_address;
            let join_handle = std::thread::Builder::new()
                .name("health-server".into())
                .spawn(move || {
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .expect("Failed to create health server runtime");
                    rt.block_on(async move {
                        let _span = info_span!("http.health", %bind_addr).entered();
                        if let Err(e) = http_server.start().await {
                            error!("HTTP health server error: {}", e);
                        }
                    });
                })
                .expect("Failed to spawn health server thread");
            info!(
                "HTTP health server running on dedicated thread at {}",
                bind_addr
            );
            Some(join_handle)
        } else {
            None
        };

        let bus_cleanup_handle = {
            let bus = bus_manager.clone();
            tokio::spawn(
                async move {
                    let mut interval = tokio::time::interval(Duration::from_secs(60));
                    loop {
                        interval.tick().await;
                        let state_cleaned = bus.cleanup_stale_state_buses().await;
                        let list_cleaned = bus.cleanup_stale_list_buses().await;
                        if state_cleaned > 0 || list_cleaned > 0 {
                            let (state_count, list_count) = bus.bus_counts().await;
                            info!(
                                "Bus cleanup: removed {} state, {} list buses. Current: {} state, {} list",
                                state_cleaned, list_cleaned, state_count, list_count
                            );
                        }
                    }
                }
                .instrument(info_span!("bus.cleanup")),
            )
        };

        let stats_handle = {
            let bus = bus_manager.clone();
            let cache = entity_cache.clone();
            tokio::spawn(
                async move {
                    let mut interval = tokio::time::interval(Duration::from_secs(30));
                    loop {
                        interval.tick().await;
                        let (_state_buses, _list_buses) = bus.bus_counts().await;
                        let _cache_stats = cache.stats().await;
                    }
                }
                .instrument(info_span!("stats.reporter")),
            )
        };

        info!("HyperStack runtime is running. Press Ctrl+C to stop.");

        // Wait for any task to complete (or handle shutdown signals)
        tokio::select! {
            _ = async {
                if let Some(handle) = ws_handle {
                    handle.await
                } else {
                    std::future::pending().await
                }
            } => {
                info!("WebSocket server task completed");
            }
            _ = projector_handle => {
                info!("Projector task completed");
            }
            _ = async {
                if let Some(handle) = parser_handle {
                    handle.await
                } else {
                    std::future::pending().await
                }
            } => {
                info!("Parser runtime task completed");
            }
            _ = bus_cleanup_handle => {
                info!("Bus cleanup task completed");
            }
            _ = stats_handle => {
                info!("Stats reporter task completed");
            }
            _ = shutdown_signal() => {}
        }

        info!("Shutting down HyperStack runtime");
        Ok(())
    }
}
