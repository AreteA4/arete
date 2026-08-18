use crate::bus::BusManager;
use crate::cache::EntityCache;
use crate::config::ServerConfig;
use crate::config::TransactionConfig;
use crate::health::HealthMonitor;
use crate::http_server::HttpServer;
use crate::materialized_view::MaterializedViewRegistry;
use crate::mutation_batch::MutationBatch;
use crate::program_runtime::ProgramRuntimeCatalog;
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
    program_runtime_catalog: ProgramRuntimeCatalog,
    materialized_views: Option<MaterializedViewRegistry>,
    websocket_auth_plugin: Option<Arc<dyn WebSocketAuthPlugin>>,
    http_auth_plugin: Option<Arc<dyn WebSocketAuthPlugin>>,
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
            program_runtime_catalog: ProgramRuntimeCatalog::default(),
            materialized_views: None,
            websocket_auth_plugin: None,
            http_auth_plugin: None,
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
            program_runtime_catalog: ProgramRuntimeCatalog::default(),
            materialized_views: None,
            websocket_auth_plugin: None,
            http_auth_plugin: None,
            websocket_usage_emitter: None,
            websocket_max_clients: None,
            websocket_rate_limit_config: None,
        }
    }

    pub fn with_spec(mut self, spec: Spec) -> Result<Self> {
        self.program_runtime_catalog =
            ProgramRuntimeCatalog::try_new(spec.program_runtime_definitions.clone())?;
        self.spec = Some(spec);
        Ok(self)
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

    pub fn with_http_auth_plugin(mut self, http_auth_plugin: Arc<dyn WebSocketAuthPlugin>) -> Self {
        self.http_auth_plugin = Some(http_auth_plugin);
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
    pub fn with_websocket_rate_limit_config(mut self, config: RateLimitConfig) -> Self {
        self.websocket_rate_limit_config = Some(config);
        self
    }

    /// Return the immutable capability plan selected by the builder.
    pub fn plan(&self) -> crate::RuntimePlan {
        self.config.runtime_plan
    }

    pub async fn run(self) -> Result<()> {
        info!("Starting Arete runtime");

        let plan = self.config.runtime_plan;
        let transaction_config = if plan.transactions {
            match self.config.transactions.clone() {
                Some(config) => config,
                None => TransactionConfig::from_env()?,
            }
        } else {
            TransactionConfig::default()
        };
        if plan.transactions && !transaction_config.enabled {
            anyhow::bail!(
                "the runtime plan enables transactions but transaction configuration is disabled"
            );
        }
        let program_runtime_catalog = self.program_runtime_catalog.clone();

        let health_monitor = if plan.health {
            self.config
                .health
                .as_ref()
                .map(|health_config| HealthMonitor::new(health_config.clone()))
        } else {
            None
        };
        if let Some(monitor) = &health_monitor {
            let _health_task = monitor.start().await;
            info!("Health monitoring enabled");
        }

        let mut projector_handle = None;
        let mut ws_handle = None;
        let mut parser_handle = None;
        let mut bus_cleanup_handle = None;
        let mut stats_handle = None;
        let mut mutations_tx_guard = None;
        let mut snapshot_service: Option<Arc<crate::snapshot::SnapshotService>> = None;
        let mut snapshot_manager_handle = None;

        if plan.live_runtime_enabled() {
            let (mutations_tx, mutations_rx) = mpsc::channel::<MutationBatch>(1024);
            mutations_tx_guard = Some(mutations_tx.clone());
            let bus_manager = BusManager::new();
            let entity_cache = EntityCache::new();

            // Restore state from the latest snapshot (when enabled) before the
            // WebSocket server spawns, so the first client's snapshot-on-subscribe
            // is already warm. The VM portion is stashed for the generated
            // runtime to hydrate before it connects to Yellowstone.
            if let Some(spec) = self.spec.as_ref() {
                let snapshot_config = match self.config.snapshots.clone() {
                    Some(config) => Some(config),
                    None => match crate::snapshot::SnapshotConfig::from_env() {
                        Ok(config) => Some(config),
                        Err(e) => {
                            error!("Invalid snapshot configuration; snapshots disabled: {e:#}");
                            None
                        }
                    },
                };
                if let Some(snapshot_config) = snapshot_config.filter(|c| c.enabled) {
                    match crate::snapshot::SnapshotService::initialize(
                        snapshot_config,
                        spec,
                        entity_cache.clone(),
                        &self.view_index,
                        mutations_tx.clone(),
                    )
                    .await
                    {
                        Ok(service) => {
                            snapshot_manager_handle = Some(service.spawn());
                            snapshot_service = Some(service);
                        }
                        Err(e) => {
                            error!("Failed to initialize snapshots; continuing without: {e:#}")
                        }
                    }
                }
            }

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

            projector_handle = Some(tokio::spawn(
                async move {
                    projector.run().await;
                }
                .instrument(info_span!("projector")),
            ));

            if plan.websocket {
                if let Some(ws_config) = &self.config.websocket {
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
                    ws_handle = Some(tokio::spawn(
                        async move {
                            if let Err(e) = ws_server.start().await {
                                error!("WebSocket server error: {}", e);
                            }
                        }
                        .instrument(info_span!("ws.server", %bind_addr)),
                    ));
                }
            }

            if let Some(spec) = self.spec.as_ref() {
                if let Some(parser_setup) = spec.parser_setup.clone() {
                    let program_id = spec
                        .program_ids
                        .first()
                        .cloned()
                        .unwrap_or_else(|| "unknown".to_string());
                    info!("Starting parser runtime for program: {}", program_id);
                    let health = health_monitor.clone();
                    let reconnection_config = self.config.reconnection.clone().unwrap_or_default();
                    parser_handle = Some(tokio::spawn(
                        async move {
                            if let Err(e) =
                                parser_setup(mutations_tx, health, reconnection_config).await
                            {
                                error!("Vixen parser runtime error: {}", e);
                            }
                        }
                        .instrument(info_span!("vixen.parser", %program_id)),
                    ));
                } else {
                    info!("Spec provided but no parser_setup configured - skipping parser runtime");
                }
            } else {
                info!("No spec provided - running in websocket-only mode");
            }

            let cleanup_bus = bus_manager.clone();
            bus_cleanup_handle = Some(tokio::spawn(
                async move {
                    let mut interval = tokio::time::interval(Duration::from_secs(60));
                    loop {
                        interval.tick().await;
                        let state_cleaned = cleanup_bus.cleanup_stale_state_buses().await;
                        let list_cleaned = cleanup_bus.cleanup_stale_list_buses().await;
                        if state_cleaned > 0 || list_cleaned > 0 {
                            let (state_count, list_count) = cleanup_bus.bus_counts().await;
                            info!(
                                "Bus cleanup: removed {} state, {} list buses. Current: {} state, {} list",
                                state_cleaned, list_cleaned, state_count, list_count
                            );
                        }
                    }
                }
                .instrument(info_span!("bus.cleanup")),
            ));

            stats_handle = Some(tokio::spawn(
                async move {
                    let mut interval = tokio::time::interval(Duration::from_secs(30));
                    loop {
                        interval.tick().await;
                        let (_state_buses, _list_buses) = bus_manager.bus_counts().await;
                        let _cache_stats = entity_cache.stats().await;
                    }
                }
                .instrument(info_span!("stats.reporter")),
            ));
        } else {
            info!(
                "Live runtime disabled; projection and Yellowstone resources were not initialized"
            );
        }

        // Run the HTTP server on a dedicated OS thread with its own single-threaded
        // tokio runtime so liveness remains responsive under projection load.
        let _http_health_handle = if let Some(http_health_config) = &self.config.http_health {
            let mut http_server = HttpServer::new(http_health_config.bind_address)
                .with_runtime_plan(plan)
                .with_program_runtime_catalog(program_runtime_catalog);
            if let Some(target_id) = self.config.program_read_binding_target_id.clone() {
                http_server = http_server.with_program_read_binding_target(target_id);
            }
            if let Some(target_id) = self.config.solana_gateway_target_id.clone() {
                http_server = http_server.with_solana_gateway_target(target_id);
            }
            if let Some(monitor) = health_monitor.clone() {
                http_server = http_server.with_health_monitor(monitor);
            }
            if let Some(plugin) = self
                .http_auth_plugin
                .clone()
                .or_else(|| self.websocket_auth_plugin.clone())
            {
                http_server = http_server.with_auth_plugin(plugin);
            }
            if plan.transactions && transaction_config.enabled {
                http_server = http_server.with_transaction_config(transaction_config.clone());
            }
            #[cfg(feature = "otel")]
            {
                http_server = http_server.with_metrics(self.metrics.clone());
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

        info!("Arete runtime is running. Press Ctrl+C to stop.");

        async fn wait_for_task(handle: Option<tokio::task::JoinHandle<()>>) {
            if let Some(handle) = handle {
                let _ = handle.await;
            } else {
                std::future::pending().await
            }
        }

        tokio::select! {
            _ = wait_for_task(ws_handle) => info!("WebSocket server task completed"),
            _ = wait_for_task(projector_handle) => info!("Projector task completed"),
            _ = wait_for_task(parser_handle) => info!("Parser runtime task completed"),
            _ = wait_for_task(bus_cleanup_handle) => info!("Bus cleanup task completed"),
            _ = wait_for_task(stats_handle) => info!("Stats reporter task completed"),
            _ = shutdown_signal() => {}
        }

        // Final snapshot while the projector is still draining, so planned
        // deploys restart near-lossless. Bounded to fit inside the platform's
        // termination grace period.
        if let Some(service) = snapshot_service.take() {
            if let Some(handle) = snapshot_manager_handle.take() {
                handle.abort();
            }
            if service.config().snapshot_on_shutdown {
                info!("Taking final snapshot before shutdown");
                match tokio::time::timeout(
                    Duration::from_secs(20),
                    service.snapshot_now(crate::snapshot::SnapshotTrigger::Shutdown),
                )
                .await
                {
                    Ok(Ok(_)) => {}
                    Ok(Err(e)) => error!("Shutdown snapshot failed: {e:#}"),
                    Err(_) => error!("Shutdown snapshot timed out"),
                }
            }
        }
        drop(mutations_tx_guard);

        info!("Shutting down Arete runtime");
        Ok(())
    }
}
