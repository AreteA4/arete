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
use crate::websocket::server::ConnectionAcceptor;
use crate::websocket::WebSocketServer;
use crate::Spec;
use crate::WebSocketAuthPlugin;
use crate::WebSocketUsageEmitter;
use anyhow::Result;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, info_span, warn, Instrument};

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

    /// Start everything and block until a shutdown signal, then stop cleanly.
    ///
    /// This is [`spawn`](Self::spawn) followed by waiting for SIGINT/SIGTERM
    /// (or for a core task to exit) and [`RuntimeHandle::shutdown`]. Callers
    /// that embed the server in a larger process should use `spawn` directly
    /// and decide for themselves when to stop.
    pub async fn run(self) -> Result<()> {
        let mut handle = self.spawn().await?;
        info!("Arete runtime is running. Press Ctrl+C to stop.");

        tokio::select! {
            _ = handle.exited() => {}
            _ = shutdown_signal() => {}
        }

        handle.shutdown().await
    }

    /// Start the runtime's tasks and return a handle that owns them.
    ///
    /// Everything `run` starts is started here: projector, parser, snapshot
    /// manager, bus cleanup, stats, and - only when configured - the WebSocket
    /// listener and the HTTP health server. Nothing here installs signal
    /// handlers or blocks; the handle is how the caller waits, serves
    /// connections it accepted itself, and shuts the runtime down.
    ///
    /// A handle owns its runtime's channel, buses, cache and snapshot state
    /// outright; nothing is shared through process globals, so a test or an
    /// application can start and stop runtimes independently.
    pub async fn spawn(self) -> Result<RuntimeHandle> {
        info!("Starting Arete runtime");

        let plan = self.config.runtime_plan;
        // Resolved before anything starts: an invalid value must stop the
        // runtime, not leave it serving from a stream at a level nobody asked
        // for.
        let commitment = match self.config.commitment {
            Some(commitment) => commitment,
            None => crate::Commitment::from_env()?,
        };
        info!(yellowstone_commitment = %commitment, "Ingesting at Yellowstone commitment");
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
        let mut background = Vec::new();
        if let Some(monitor) = &health_monitor {
            background.push(monitor.start().await);
            info!("Health monitoring enabled");
        }

        let mut projector_handle = None;
        let mut ws_handle = None;
        let mut parser_handle = None;
        let mut mutations_tx_guard = None;
        let mut snapshot_service: Option<Arc<crate::snapshot::SnapshotService>> = None;
        let mut snapshot_manager_handle = None;
        let mut snapshot_runtime = None;
        let mut acceptor = None;
        let mut entity_cache_handle = None;

        if plan.live_runtime_enabled() {
            let (mutations_tx, mutations_rx) = mpsc::channel::<MutationBatch>(1024);
            mutations_tx_guard = Some(mutations_tx.clone());
            let bus_manager = BusManager::new();
            let entity_cache = EntityCache::new();
            entity_cache_handle = Some(entity_cache.clone());

            // Retained event tape for replayable append subscriptions. The
            // builder wins over the process env so one host can enable replay
            // for a single deployment and size it independently. A bad
            // configuration disables replay rather than failing startup, the
            // same posture snapshots take below.
            let journal_config = match self.config.journal.clone() {
                Some(config) => config,
                None => match crate::journal::JournalConfig::from_env() {
                    Ok(config) => config,
                    Err(e) => {
                        error!("Invalid journal configuration; event replay disabled: {e:#}");
                        crate::journal::JournalConfig::default()
                    }
                },
            };
            let journal = Arc::new(crate::journal::EventJournal::new(journal_config));
            if journal.is_enabled() {
                info!(
                    max_bytes_per_view = journal.config().max_bytes_per_view,
                    max_records_per_view = journal.config().max_records_per_view,
                    max_age_secs = journal.config().max_age.as_secs(),
                    "Event replay enabled for append views"
                );
            }

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
                if let Some(mut snapshot_config) = snapshot_config.filter(|c| c.enabled) {
                    // The runtime's level, whatever the snapshot config carried:
                    // a snapshot records the level it was taken at and restore
                    // compares against this one.
                    snapshot_config.commitment = commitment;
                    match crate::snapshot::SnapshotService::initialize(
                        snapshot_config,
                        spec,
                        entity_cache.clone(),
                        &self.view_index,
                        journal.clone(),
                        mutations_tx.clone(),
                    )
                    .await
                    {
                        Ok(service) => {
                            snapshot_runtime = Some(service.runtime());
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
            let projector = match snapshot_runtime.clone() {
                Some(runtime) => projector.with_snapshot_runtime(runtime),
                None => projector,
            };
            let projector = projector.with_journal(journal.clone());

            // The projector runs for the lifetime of the server. Giving the
            // task a span would make that span the parent of every batch
            // whose producer does not carry an explicit context, creating an
            // unbounded trace. `Projector::run` instead enters the bounded
            // batch span before it processes and logs each batch.
            projector_handle = Some(tokio::spawn(async move {
                projector.run().await;
            }));

            // The connection-serving half of the WebSocket server exists
            // whenever there is a live runtime, so a caller that owns its own
            // listener can hand in the connections it accepts. The listener
            // is bound only when a WebSocket address is configured.
            let bind_address = self
                .config
                .websocket
                .as_ref()
                .map(|ws_config| ws_config.bind_address)
                .unwrap_or_else(|| SocketAddr::from(([0, 0, 0, 0], 0)));
            #[cfg(feature = "otel")]
            let mut ws_server = WebSocketServer::new(
                bind_address,
                bus_manager.clone(),
                entity_cache.clone(),
                self.view_index.clone(),
                self.metrics.clone(),
            );
            #[cfg(not(feature = "otel"))]
            let mut ws_server = WebSocketServer::new(
                bind_address,
                bus_manager.clone(),
                entity_cache.clone(),
                self.view_index.clone(),
            );

            ws_server = ws_server.with_journal(journal.clone());
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
            let (connection_acceptor, cleanup_handle) = ws_server.into_acceptor();
            background.push(cleanup_handle);

            if plan.websocket && self.config.websocket.is_some() {
                let listener_acceptor = connection_acceptor.clone();
                ws_handle = Some(tokio::spawn(
                    async move {
                        info!("Starting WebSocket server on {}", bind_address);
                        let listener = match TcpListener::bind(&bind_address).await {
                            Ok(listener) => listener,
                            Err(e) => {
                                error!("WebSocket server error: {}", e);
                                return;
                            }
                        };
                        if let Err(e) = listener_acceptor.serve_listener(listener).await {
                            error!("WebSocket server error: {}", e);
                        }
                    }
                    .instrument(info_span!("ws.server", %bind_address)),
                ));
            }
            acceptor = Some(connection_acceptor);

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
                    let parser_snapshot_runtime = snapshot_runtime.clone();
                    let parser_journal = journal.clone();
                    parser_handle = Some(tokio::spawn(
                        async move {
                            let parser = async move {
                                parser_setup(mutations_tx, health, reconnection_config).await
                            };
                            let scoped = async move {
                                match parser_snapshot_runtime {
                                    Some(runtime) => runtime.scope(parser).await,
                                    None => parser.await,
                                }
                            };
                            // The tape is in scope even with snapshots off, so
                            // a runtime that abandons its checkpoint can still
                            // mark the hole it just created.
                            let result = commitment.scope(parser_journal.scope(scoped)).await;
                            if let Err(e) = result {
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
            background.push(tokio::spawn(
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

            background.push(tokio::spawn(
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
        let http_shutdown = CancellationToken::new();
        let http_health_thread = if let Some(http_health_config) = &self.config.http_health {
            let mut http_server = HttpServer::new(http_health_config.bind_address)
                .with_runtime_plan(plan)
                .with_program_runtime_catalog(program_runtime_catalog)
                .with_shutdown(http_shutdown.clone());
            if let Some(target_id) = self.config.program_read_binding_target_id.clone() {
                http_server = http_server.with_program_read_binding_target(target_id);
            }
            if let Some(target_id) = self.config.solana_gateway_target_id.clone() {
                http_server = http_server.with_solana_gateway_target(target_id);
            }
            if plan.live_runtime_enabled() {
                http_server = http_server.with_commitment(commitment);
            }
            if let Some(monitor) = health_monitor.clone() {
                http_server = http_server.with_health_monitor(monitor);
            }
            if let Some(runtime) = snapshot_runtime.clone() {
                http_server = http_server.with_snapshot_runtime(runtime);
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

        Ok(RuntimeHandle {
            plan,
            health_monitor,
            snapshot_runtime,
            snapshot_service,
            snapshot_manager_handle,
            mutations_tx: mutations_tx_guard,
            projector_handle,
            parser_handle,
            ws_handle,
            background,
            acceptor,
            entity_cache: entity_cache_handle,
            http_shutdown,
            http_health_thread,
        })
    }
}

/// A running [`Runtime`], owned by whoever called [`Runtime::spawn`].
///
/// Dropping the handle does **not** stop the runtime; the tasks it owns keep
/// running on the tokio runtime. Call [`shutdown`](Self::shutdown) to stop
/// them and release the memory they hold. This is deliberate: the same
/// semantics as dropping a `JoinHandle`, and what lets `run` hand the handle
/// across a `select!`.
pub struct RuntimeHandle {
    plan: crate::RuntimePlan,
    health_monitor: Option<HealthMonitor>,
    snapshot_runtime: Option<crate::snapshot::SnapshotRuntime>,
    snapshot_service: Option<Arc<crate::snapshot::SnapshotService>>,
    snapshot_manager_handle: Option<JoinHandle<()>>,
    mutations_tx: Option<mpsc::Sender<MutationBatch>>,
    projector_handle: Option<JoinHandle<()>>,
    parser_handle: Option<JoinHandle<()>>,
    ws_handle: Option<JoinHandle<()>>,
    background: Vec<JoinHandle<()>>,
    acceptor: Option<ConnectionAcceptor>,
    entity_cache: Option<EntityCache>,
    http_shutdown: CancellationToken,
    http_health_thread: Option<std::thread::JoinHandle<()>>,
}

/// Serves caller-accepted connections against one runtime. Obtained from
/// [`RuntimeHandle::connection_server`]; cheap to clone into the task that
/// owns each connection.
#[derive(Clone)]
pub struct ConnectionServer(ConnectionAcceptor);

impl ConnectionServer {
    /// Serve one accepted connection until the peer disconnects or the
    /// runtime shuts down. See [`RuntimeHandle::serve_connection`].
    pub async fn serve(&self, stream: TcpStream, remote_addr: SocketAddr) -> Result<()> {
        self.0.serve(stream, remote_addr).await
    }

    /// Number of WebSocket clients currently connected to the runtime.
    pub fn client_count(&self) -> usize {
        self.0.client_count()
    }
}

/// How long `shutdown` waits for listener-spawned sessions to finish their
/// cleanup after being told to stop.
const SESSION_DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

/// How long `shutdown` lets the projector drain queued batches after the
/// producers have stopped.
const PROJECTOR_DRAIN_TIMEOUT: Duration = Duration::from_secs(10);

/// Bound on the final snapshot, chosen to fit inside the platform's
/// termination grace period.
const SHUTDOWN_SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(20);

impl RuntimeHandle {
    /// The capability plan this runtime was started with.
    pub fn plan(&self) -> crate::RuntimePlan {
        self.plan
    }

    /// Whether this runtime should take traffic: the stream is healthy and,
    /// after a snapshot restore, the parser has caught back up to the slot
    /// tip. The same test the HTTP `/ready` endpoint applies.
    pub async fn is_ready(&self) -> bool {
        let stream_ready = match self.health_monitor.as_ref() {
            Some(monitor) => monitor.is_healthy().await,
            None => true,
        };
        let snapshot_ready = self
            .snapshot_runtime
            .as_ref()
            .is_none_or(crate::snapshot::SnapshotRuntime::resume_gate_ready);
        stream_ready && snapshot_ready
    }

    /// Number of WebSocket clients currently connected to this runtime.
    pub fn client_count(&self) -> usize {
        self.acceptor
            .as_ref()
            .map(ConnectionAcceptor::client_count)
            .unwrap_or(0)
    }

    /// What this runtime's entity cache holds: the entities kept per view
    /// for snapshot-on-subscribe. For an embedder accounting for the
    /// runtime's memory. `None` when the runtime has no live runtime.
    pub async fn entity_cache_stats(&self) -> Option<crate::cache::CacheStats> {
        match &self.entity_cache {
            Some(cache) => Some(cache.stats().await),
            None => None,
        }
    }

    /// Serve a TCP connection the caller accepted, as this runtime's
    /// WebSocket server would have: handshake, authentication, then the
    /// subscription session until the peer disconnects.
    ///
    /// Resolves when the session ends, or when the runtime shuts down. Fails
    /// if the runtime has no live runtime (no buses to subscribe to). Callers
    /// that serve from an accept loop should take a
    /// [`connection_server`](Self::connection_server), which is cheap to clone
    /// into each connection's task.
    pub async fn serve_connection(&self, stream: TcpStream, remote_addr: SocketAddr) -> Result<()> {
        match self.connection_server() {
            Some(server) => server.serve(stream, remote_addr).await,
            None => anyhow::bail!("this runtime has no live runtime to serve connections from"),
        }
    }

    /// A clonable handle that serves connections against this runtime.
    ///
    /// `None` when the runtime has no live runtime. Sessions served through a
    /// clone are ended by [`shutdown`](Self::shutdown) like any other.
    pub fn connection_server(&self) -> Option<ConnectionServer> {
        self.acceptor.clone().map(ConnectionServer)
    }

    /// Resolves when a core task - projector, parser or WebSocket listener -
    /// exits on its own. `run` treats that as a reason to shut down.
    pub async fn exited(&mut self) {
        async fn wait(handle: Option<&mut JoinHandle<()>>) {
            match handle {
                Some(handle) => {
                    let _ = handle.await;
                }
                None => std::future::pending().await,
            }
        }

        tokio::select! {
            _ = wait(self.ws_handle.as_mut()) => info!("WebSocket server task completed"),
            _ = wait(self.projector_handle.as_mut()) => info!("Projector task completed"),
            _ = wait(self.parser_handle.as_mut()) => info!("Parser runtime task completed"),
        }
    }

    /// Stop the runtime: take the final snapshot if configured, stop
    /// producing, let the projector drain, close sessions, then stop
    /// everything else.
    ///
    /// Order matters. The final snapshot is taken *first*, while the parser
    /// is still running: capture takes the barrier exclusively, so it waits
    /// for every in-flight update to reach the projector and records one
    /// consistent cut. Aborting the parser before that could cut an update
    /// between its VM write and its batch, and the snapshot would keep the
    /// write without the projection. Only then is the parser aborted; anything
    /// it produces after the snapshot is simply not restored. The sender is
    /// dropped so the projector exits once its queue is empty, bounded by a
    /// timeout after which it is aborted and awaited. Sessions are ended
    /// through their normal cleanup, the HTTP health server is signalled and
    /// its thread joined, and the remaining tasks are aborted.
    pub async fn shutdown(mut self) -> Result<()> {
        // Final snapshot before anything stops, so planned stops restart
        // near-lossless. Bounded to fit inside a termination grace period.
        if let Some(service) = self.snapshot_service.take() {
            if let Some(handle) = self.snapshot_manager_handle.take() {
                handle.abort();
            }
            if service.config().snapshot_on_shutdown {
                info!("Taking final snapshot before shutdown");
                match tokio::time::timeout(
                    SHUTDOWN_SNAPSHOT_TIMEOUT,
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
        if let Some(handle) = self.snapshot_manager_handle.take() {
            handle.abort();
        }

        if let Some(parser) = self.parser_handle.take() {
            parser.abort();
            let _ = parser.await;
        }
        if let Some(acceptor) = &self.acceptor {
            acceptor.shutdown();
        }
        if let Some(ws) = self.ws_handle.take() {
            let _ = ws.await;
        }
        if let Some(acceptor) = &self.acceptor {
            if tokio::time::timeout(SESSION_DRAIN_TIMEOUT, acceptor.wait_for_sessions())
                .await
                .is_err()
            {
                warn!(
                    "Sessions did not finish within {:?} of shutdown",
                    SESSION_DRAIN_TIMEOUT
                );
            }
        }

        drop(self.mutations_tx.take());
        if let Some(mut projector) = self.projector_handle.take() {
            if tokio::time::timeout(PROJECTOR_DRAIN_TIMEOUT, &mut projector)
                .await
                .is_err()
            {
                warn!(
                    "Projector did not drain within {:?}; aborting it",
                    PROJECTOR_DRAIN_TIMEOUT
                );
                projector.abort();
                let _ = projector.await;
            }
        }

        for handle in self.background.drain(..) {
            handle.abort();
        }

        self.http_shutdown.cancel();
        if let Some(thread) = self.http_health_thread.take() {
            if let Err(e) = tokio::task::spawn_blocking(move || thread.join()).await {
                error!("Health server thread join failed: {e}");
            }
        }

        info!("Shutting down Arete runtime");
        Ok(())
    }
}
