use crate::bus::{BusManager, BusMessage, StateUpdate};
use crate::cache::{cmp_seq, EntityCache, SnapshotBatchConfig};
use crate::compression::maybe_compress;
use crate::shared_entity::{lookup, EntityFields, SharedEntity};
use crate::view::{ViewIndex, ViewSpec};
use crate::websocket::admission::{WebSocketAdmissionProvider, WebSocketConnectionPermit};
use crate::websocket::auth::{
    AuthContext, AuthDecision, AuthDeny, ConnectionAuthRequest, WebSocketAuthPlugin,
};
use crate::websocket::client_manager::{ClientManager, RateLimitConfig};
use crate::websocket::frame::{
    apply_wire_format, Frame, Mode, SortConfig, SortOrder, SourceFields, SubscribedFrame,
    UnsubscribedFrame, WireEntity, WireFormat,
};
use crate::websocket::subscription::{
    ClientMessage, RefreshAuthRequest, RefreshAuthResponse, SocketIssueMessage, Subscription,
    SubscriptionQuery, Unsubscription, PROTOCOL_VERSION,
};
use crate::websocket::usage::{UsageIdentity, WebSocketUsageEmitter, WebSocketUsageEvent};
use crate::WebSocketDeliveryConfig;
use anyhow::Result;
use bytes::Bytes;
use futures_util::StreamExt;
use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet, VecDeque};
use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, watch};
use tokio_tungstenite::{
    accept_hdr_async,
    tungstenite::{
        handshake::server::{ErrorResponse as HandshakeErrorResponse, Request, Response},
        http::{header::CONTENT_TYPE, StatusCode},
        Error as WsError,
    },
};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tracing::{debug, error, info, info_span, warn, Instrument};
use uuid::Uuid;

#[cfg(feature = "otel")]
use crate::metrics::Metrics;

#[derive(Clone, Default)]
struct WsMetrics {
    #[cfg(feature = "otel")]
    inner: Option<Arc<Metrics>>,
    #[cfg(test)]
    probe: Option<Arc<DeliveryProbe>>,
}

/// Test-only mirror of the delivery instruments recorded through
/// [`WsMetrics`], so the real-socket load harness can report them without the
/// `otel` feature. Each counter has the meaning of the metric named beside it.
#[cfg(test)]
#[derive(Debug, Default)]
pub(crate) struct DeliveryProbe {
    /// `arete.ws.connections.active`, by metering key
    pub(crate) active_connections: std::sync::Mutex<std::collections::BTreeMap<String, i64>>,
    /// `arete.ws.subscriptions.active`, by view and metering key
    pub(crate) active_subscriptions: std::sync::Mutex<std::collections::BTreeMap<String, i64>>,
    /// `arete.ws.messages.sent`
    pub(crate) messages_sent: std::sync::atomic::AtomicU64,
    /// `arete.ws.subscription.lagged`
    pub(crate) lag_events: std::sync::atomic::AtomicU64,
    /// `arete.ws.subscription.dropped_updates`
    pub(crate) dropped_updates: std::sync::atomic::AtomicU64,
    /// `arete.ws.subscription.resnapshots`
    pub(crate) resnapshots: std::sync::atomic::AtomicU64,
    /// `arete.ws.collection.coalesced_updates`
    pub(crate) coalesced_updates: std::sync::atomic::AtomicU64,
    /// `arete.ws.collection.coalesced_flushes`
    pub(crate) coalesced_flushes: std::sync::atomic::AtomicU64,
    /// `arete.ws.delivery.stopped`, by reason
    pub(crate) delivery_stopped: std::sync::Mutex<std::collections::BTreeMap<&'static str, u64>>,
}

#[cfg(test)]
impl DeliveryProbe {
    fn add(counter: &std::sync::atomic::AtomicU64, value: u64) {
        counter.fetch_add(value, std::sync::atomic::Ordering::Relaxed);
    }

    fn add_active(
        counts: &std::sync::Mutex<std::collections::BTreeMap<String, i64>>,
        key: String,
        delta: i64,
    ) {
        *counts
            .lock()
            .expect("delivery probe lock poisoned")
            .entry(key)
            .or_default() += delta;
    }
}

impl WsMetrics {
    #[cfg(feature = "otel")]
    fn new(inner: Option<Arc<Metrics>>) -> Self {
        Self {
            inner,
            #[cfg(test)]
            probe: None,
        }
    }

    #[cfg(test)]
    fn probe(&self, record: impl FnOnce(&DeliveryProbe)) {
        if let Some(probe) = &self.probe {
            record(probe);
        }
    }

    fn connection_opened(&self, metering_key: Option<&str>) {
        #[cfg(test)]
        self.probe(|probe| {
            DeliveryProbe::add_active(
                &probe.active_connections,
                metering_key.unwrap_or("<none>").to_string(),
                1,
            )
        });
        #[cfg(not(feature = "otel"))]
        let _ = metering_key;
        #[cfg(feature = "otel")]
        if let Some(metrics) = &self.inner {
            if let Some(metering_key) = metering_key {
                metrics.record_ws_connection_with_metering(metering_key);
            } else {
                metrics.record_ws_connection();
            }
        }
    }

    fn connection_closed(&self, duration_secs: f64, metering_key: Option<&str>) {
        #[cfg(test)]
        self.probe(|probe| {
            DeliveryProbe::add_active(
                &probe.active_connections,
                metering_key.unwrap_or("<none>").to_string(),
                -1,
            )
        });
        #[cfg(not(feature = "otel"))]
        let _ = (duration_secs, metering_key);
        #[cfg(feature = "otel")]
        if let Some(metrics) = &self.inner {
            if let Some(metering_key) = metering_key {
                metrics.record_ws_disconnection_with_metering(duration_secs, metering_key);
            } else {
                metrics.record_ws_disconnection(duration_secs);
            }
        }
    }

    fn message_received(&self, metering_key: Option<&str>) {
        #[cfg(not(feature = "otel"))]
        let _ = metering_key;
        #[cfg(feature = "otel")]
        if let Some(metrics) = &self.inner {
            if let Some(metering_key) = metering_key {
                metrics.record_ws_message_received_with_metering(metering_key);
            } else {
                metrics.record_ws_message_received();
            }
        }
    }

    fn message_sent(&self) {
        #[cfg(test)]
        self.probe(|probe| DeliveryProbe::add(&probe.messages_sent, 1));
        #[cfg(feature = "otel")]
        if let Some(metrics) = &self.inner {
            metrics.record_ws_message_sent();
        }
    }

    fn subscription_created(&self, view: &str, metering_key: Option<&str>) {
        #[cfg(test)]
        self.probe(|probe| {
            DeliveryProbe::add_active(
                &probe.active_subscriptions,
                format!("{view}|{}", metering_key.unwrap_or("<none>")),
                1,
            )
        });
        #[cfg(not(feature = "otel"))]
        let _ = (view, metering_key);
        #[cfg(feature = "otel")]
        if let Some(metrics) = &self.inner {
            if let Some(metering_key) = metering_key {
                metrics.record_subscription_created_with_metering(view, metering_key);
            } else {
                metrics.record_subscription_created(view);
            }
        }
    }

    fn subscription_removed(&self, view: &str, metering_key: Option<&str>) {
        #[cfg(test)]
        self.probe(|probe| {
            DeliveryProbe::add_active(
                &probe.active_subscriptions,
                format!("{view}|{}", metering_key.unwrap_or("<none>")),
                -1,
            )
        });
        #[cfg(not(feature = "otel"))]
        let _ = (view, metering_key);
        #[cfg(feature = "otel")]
        if let Some(metrics) = &self.inner {
            if let Some(metering_key) = metering_key {
                metrics.record_subscription_removed_with_metering(view, metering_key);
            } else {
                metrics.record_subscription_removed(view);
            }
        }
    }

    fn protocol_error(&self, code: &str) {
        #[cfg(not(feature = "otel"))]
        let _ = code;
        #[cfg(feature = "otel")]
        if let Some(metrics) = &self.inner {
            metrics.record_ws_protocol_error(code);
        }
    }

    fn subscription_lagged(&self, view: &str, skipped: u64) {
        #[cfg(test)]
        self.probe(|probe| {
            DeliveryProbe::add(&probe.lag_events, 1);
            DeliveryProbe::add(&probe.dropped_updates, skipped);
        });
        #[cfg(not(feature = "otel"))]
        let _ = (view, skipped);
        #[cfg(feature = "otel")]
        if let Some(metrics) = &self.inner {
            metrics.record_ws_subscription_lagged(view, skipped);
        }
    }

    fn subscription_resnapshot(&self, view: &str) {
        #[cfg(test)]
        self.probe(|probe| DeliveryProbe::add(&probe.resnapshots, 1));
        #[cfg(not(feature = "otel"))]
        let _ = view;
        #[cfg(feature = "otel")]
        if let Some(metrics) = &self.inner {
            metrics.record_ws_subscription_resnapshot(view);
        }
    }

    fn collection_coalesced(&self, view: &str, updates: u64) {
        #[cfg(test)]
        self.probe(|probe| {
            DeliveryProbe::add(&probe.coalesced_updates, updates);
            DeliveryProbe::add(&probe.coalesced_flushes, 1);
        });
        #[cfg(not(feature = "otel"))]
        let _ = (view, updates);
        #[cfg(feature = "otel")]
        if let Some(metrics) = &self.inner {
            metrics.record_ws_collection_coalesced(view, updates);
        }
    }

    fn delivery_stopped(&self, view: &str, reason: &'static str) {
        #[cfg(test)]
        self.probe(|probe| {
            *probe
                .delivery_stopped
                .lock()
                .expect("delivery probe lock poisoned")
                .entry(reason)
                .or_default() += 1;
        });
        #[cfg(not(feature = "otel"))]
        let _ = (view, reason);
        #[cfg(feature = "otel")]
        if let Some(metrics) = &self.inner {
            metrics.record_ws_delivery_stopped(view, reason);
        }
    }
}

async fn handle_refresh_auth(
    client_id: Uuid,
    refresh_req: &RefreshAuthRequest,
    client_manager: &ClientManager,
    auth_plugin: &Arc<dyn WebSocketAuthPlugin>,
) {
    let refresh_result: Result<AuthContext, String> = if let Some(signed_plugin) = auth_plugin
        .as_any()
        .downcast_ref::<crate::websocket::auth::SignedSessionAuthPlugin>()
    {
        signed_plugin
            .verify_refresh_token(&refresh_req.token)
            .await
            .map_err(|error| error.reason)
    } else {
        Err("In-band auth refresh not supported with current auth plugin".to_string())
    };

    let response = match refresh_result {
        Ok(new_context) => {
            let expires_at = new_context.expires_at;
            match client_manager
                .try_update_client_auth_async(client_id, new_context)
                .await
            {
                Ok(true) => RefreshAuthResponse {
                    success: true,
                    error: None,
                    expires_at: Some(expires_at),
                },
                Ok(false) => RefreshAuthResponse {
                    success: false,
                    error: Some("client-not-found".to_string()),
                    expires_at: None,
                },
                Err(deny) => RefreshAuthResponse {
                    success: false,
                    error: Some(deny.code.as_str().to_string()),
                    expires_at: None,
                },
            }
        }
        Err(error) => {
            let code = if error.contains("expired") {
                "token-expired"
            } else if error.contains("signature") {
                "token-invalid-signature"
            } else if error.contains("issuer") {
                "token-invalid-issuer"
            } else if error.contains("audience") {
                "token-invalid-audience"
            } else {
                "token-invalid"
            };
            RefreshAuthResponse {
                success: false,
                error: Some(code.to_string()),
                expires_at: None,
            }
        }
    };

    if let Ok(json) = serde_json::to_string(&response) {
        let _ = client_manager.send_text_to_client(client_id, json).await;
    }
}

async fn send_socket_issue(
    client_id: Uuid,
    client_manager: &ClientManager,
    deny: &AuthDeny,
    fatal: bool,
    subscription_id: Option<String>,
) {
    let message = SocketIssueMessage::from_auth_deny(deny, fatal, subscription_id);
    if let Ok(json) = serde_json::to_string(&message) {
        let _ = client_manager.send_text_to_client(client_id, json).await;
    }
}

async fn send_protocol_issue(
    client_id: Uuid,
    client_manager: &ClientManager,
    metrics: &WsMetrics,
    subscription_id: Option<String>,
    code: &str,
    message: impl Into<String>,
) {
    metrics.protocol_error(code);
    let issue = SocketIssueMessage::protocol(subscription_id, code, message);
    if let Ok(json) = serde_json::to_string(&issue) {
        let _ = client_manager.send_text_to_client(client_id, json).await;
    }
}

/// Send an already-built issue, for refusals that carry structured detail.
async fn send_prepared_issue(
    client_id: Uuid,
    client_manager: &ClientManager,
    metrics: &WsMetrics,
    issue: SocketIssueMessage,
) {
    metrics.protocol_error(&issue.code);
    if let Ok(json) = serde_json::to_string(&issue) {
        let _ = client_manager.send_text_to_client(client_id, json).await;
    }
}

fn key_class_label(key_class: arete_auth::KeyClass) -> &'static str {
    match key_class {
        arete_auth::KeyClass::Secret => "secret",
        arete_auth::KeyClass::Publishable => "publishable",
    }
}

#[derive(Clone)]
struct UsageEmitterHandle {
    emitter: Arc<dyn WebSocketUsageEmitter>,
    tasks: TaskTracker,
}

impl UsageEmitterHandle {
    fn new(emitter: Arc<dyn WebSocketUsageEmitter>) -> Self {
        Self {
            emitter,
            tasks: TaskTracker::new(),
        }
    }

    fn emit(&self, event: WebSocketUsageEvent) {
        let emitter = self.emitter.clone();
        self.tasks.spawn(async move {
            emitter.emit(event).await;
        });
    }

    async fn close_and_wait(&self) {
        self.tasks.close();
        self.tasks.wait().await;
    }
}

fn emit_usage_event(usage_emitter: &Option<UsageEmitterHandle>, event: WebSocketUsageEvent) {
    if let Some(emitter) = usage_emitter {
        emitter.emit(event);
    }
}

fn usage_identity_from_context(
    auth_context: Option<&AuthContext>,
) -> (UsageIdentity, Option<String>) {
    match auth_context {
        Some(context) => {
            let v2 = !context.is_legacy_policy();
            (
                UsageIdentity {
                    metering_key: Some(context.metering_key.clone()),
                    subject: Some(context.subject.clone()),
                    key_class: Some(key_class_label(context.key_class).to_string()),
                    actor_key: v2.then(|| context.actor_key.clone()).flatten(),
                    account_key: v2.then(|| context.account_key.clone()).flatten(),
                    consumer_key: v2.then(|| context.consumer_key.clone()).flatten(),
                    plan_code: v2.then(|| context.plan.clone()).flatten(),
                    policy_version: v2.then_some(context.policy_version).flatten(),
                },
                context.deployment_id.clone(),
            )
        }
        None => (UsageIdentity::default(), None),
    }
}

fn emit_update_sent_for_client(
    usage_emitter: &Option<UsageEmitterHandle>,
    client_manager: &ClientManager,
    client_id: Uuid,
    view_id: &str,
    bytes: usize,
) {
    let auth_context = client_manager.get_auth_context(client_id);
    let (identity, deployment_id) = usage_identity_from_context(auth_context.as_ref());
    emit_usage_event(
        usage_emitter,
        WebSocketUsageEvent::UpdateSent {
            client_id: client_id.to_string(),
            deployment_id,
            identity,
            view_id: view_id.to_string(),
            messages: 1,
            bytes: bytes as u64,
        },
    );
}

#[derive(Clone)]
struct SubscriptionContext {
    client_id: Uuid,
    client_manager: ClientManager,
    bus_manager: BusManager,
    entity_cache: EntityCache,
    view_index: Arc<ViewIndex>,
    usage_emitter: Option<UsageEmitterHandle>,
    journal: Option<Arc<crate::journal::EventJournal>>,
    metrics: WsMetrics,
    delivery: WebSocketDeliveryConfig,
    /// Cancelled when the server stops; every session ends through its normal
    /// cleanup path rather than being dropped mid-flight.
    shutdown: CancellationToken,
}

pub struct WebSocketServer {
    bind_addr: SocketAddr,
    client_manager: ClientManager,
    bus_manager: BusManager,
    entity_cache: EntityCache,
    view_index: Arc<ViewIndex>,
    max_clients: usize,
    auth_plugin: Arc<dyn WebSocketAuthPlugin>,
    usage_emitter: Option<UsageEmitterHandle>,
    rate_limit_config: Option<RateLimitConfig>,
    admission_provider: Option<Arc<dyn WebSocketAdmissionProvider>>,
    journal: Option<Arc<crate::journal::EventJournal>>,
    delivery: WebSocketDeliveryConfig,
    #[cfg(feature = "otel")]
    metrics: Option<Arc<Metrics>>,
}

impl WebSocketServer {
    #[cfg(feature = "otel")]
    pub fn new(
        bind_addr: SocketAddr,
        bus_manager: BusManager,
        entity_cache: EntityCache,
        view_index: Arc<ViewIndex>,
        metrics: Option<Arc<Metrics>>,
    ) -> Self {
        Self {
            bind_addr,
            client_manager: ClientManager::new(),
            bus_manager,
            entity_cache,
            view_index,
            max_clients: 10_000,
            auth_plugin: Arc::new(crate::websocket::auth::AllowAllAuthPlugin),
            usage_emitter: None,
            rate_limit_config: None,
            admission_provider: None,
            journal: None,
            delivery: WebSocketDeliveryConfig::default(),
            metrics,
        }
    }

    #[cfg(not(feature = "otel"))]
    pub fn new(
        bind_addr: SocketAddr,
        bus_manager: BusManager,
        entity_cache: EntityCache,
        view_index: Arc<ViewIndex>,
    ) -> Self {
        Self {
            bind_addr,
            client_manager: ClientManager::new(),
            bus_manager,
            entity_cache,
            view_index,
            max_clients: 10_000,
            auth_plugin: Arc::new(crate::websocket::auth::AllowAllAuthPlugin),
            usage_emitter: None,
            rate_limit_config: None,
            admission_provider: None,
            journal: None,
            delivery: WebSocketDeliveryConfig::default(),
        }
    }

    pub fn with_max_clients(mut self, max_clients: usize) -> Self {
        self.max_clients = max_clients;
        self
    }

    pub fn with_auth_plugin(mut self, auth_plugin: Arc<dyn WebSocketAuthPlugin>) -> Self {
        self.auth_plugin = auth_plugin;
        self
    }

    pub fn with_usage_emitter(mut self, usage_emitter: Arc<dyn WebSocketUsageEmitter>) -> Self {
        self.usage_emitter = Some(UsageEmitterHandle::new(usage_emitter));
        self
    }

    /// Serve replayable append subscriptions from the retained event journal.
    pub fn with_journal(mut self, journal: Arc<crate::journal::EventJournal>) -> Self {
        self.journal = Some(journal);
        self
    }

    pub fn with_rate_limit_config(mut self, config: RateLimitConfig) -> Self {
        self.rate_limit_config = Some(config);
        self
    }

    /// Install an embedder-owned provider for cross-runtime admission.
    pub fn with_admission_provider(
        mut self,
        provider: Arc<dyn WebSocketAdmissionProvider>,
    ) -> Self {
        self.admission_provider = Some(provider);
        self
    }

    pub fn with_delivery_config(mut self, config: WebSocketDeliveryConfig) -> Self {
        self.delivery = config;
        self
    }

    /// Bind the configured address and serve connections until the task is
    /// dropped. Equivalent to [`into_acceptor`](Self::into_acceptor) followed by
    /// [`ConnectionAcceptor::serve_listener`].
    pub async fn start(self) -> Result<()> {
        info!(
            "Starting WebSocket server on {} (max_clients: {})",
            self.bind_addr, self.max_clients
        );
        let listener = TcpListener::bind(&self.bind_addr).await?;
        let (acceptor, _cleanup) = self.into_acceptor();
        acceptor.serve_listener(listener).await
    }

    /// Split this server into the part that serves connections and the
    /// client-manager cleanup task, leaving the caller to own the listener.
    ///
    /// The cleanup handle is returned rather than detached so a caller that
    /// stops serving can stop it too.
    pub(crate) fn into_acceptor(self) -> (ConnectionAcceptor, tokio::task::JoinHandle<()>) {
        let mut client_manager = self
            .rate_limit_config
            .map(ClientManager::with_config)
            .unwrap_or(self.client_manager);
        if let Some(provider) = self.admission_provider {
            client_manager = client_manager.with_admission_provider(provider);
        }
        let cleanup = client_manager.start_cleanup_task();

        #[cfg(feature = "otel")]
        let metrics = WsMetrics::new(self.metrics.clone());
        #[cfg(not(feature = "otel"))]
        let metrics = WsMetrics::default();

        let acceptor = ConnectionAcceptor {
            client_manager,
            bus_manager: self.bus_manager,
            entity_cache: self.entity_cache,
            view_index: self.view_index,
            max_clients: self.max_clients,
            auth_plugin: self.auth_plugin,
            usage_emitter: self.usage_emitter,
            journal: self.journal,
            delivery: self.delivery,
            metrics,
            shutdown: CancellationToken::new(),
            sessions: TaskTracker::new(),
        };
        (acceptor, cleanup)
    }
}

/// Serves already-accepted TCP connections against one server's buses, cache
/// and views.
///
/// This is what [`WebSocketServer::start`] runs behind its listener, separated
/// so that a caller that owns the listener (an application that terminates
/// TLS itself, a test with an ephemeral port) can hand streams in without the
/// server binding anything.
#[derive(Clone)]
pub(crate) struct ConnectionAcceptor {
    client_manager: ClientManager,
    bus_manager: BusManager,
    entity_cache: EntityCache,
    view_index: Arc<ViewIndex>,
    max_clients: usize,
    auth_plugin: Arc<dyn WebSocketAuthPlugin>,
    usage_emitter: Option<UsageEmitterHandle>,
    journal: Option<Arc<crate::journal::EventJournal>>,
    delivery: WebSocketDeliveryConfig,
    metrics: WsMetrics,
    shutdown: CancellationToken,
    /// Sessions spawned by [`serve_listener`](Self::serve_listener), so a
    /// stop can wait for them. Sessions a caller serves on its own tasks are
    /// the caller's to wait for.
    sessions: TaskTracker,
}

impl ConnectionAcceptor {
    /// Mirror this acceptor's delivery instruments into `probe`. Must be set
    /// before serving: each session copies the metrics handle when it starts.
    #[cfg(test)]
    pub(crate) fn with_delivery_probe(mut self, probe: Arc<DeliveryProbe>) -> Self {
        self.metrics.probe = Some(probe);
        self
    }

    /// Number of clients currently connected to this server.
    pub(crate) fn client_count(&self) -> usize {
        self.client_manager.client_count()
    }

    /// End every session this acceptor is serving and stop accepting.
    ///
    /// Sessions notice on their next poll and leave through the same cleanup
    /// as a client disconnect, so the client manager, buses and usage events
    /// see an ordinary close.
    pub(crate) fn shutdown(&self) {
        self.shutdown.cancel();
        self.sessions.close();
    }

    /// Resolves once every listener-spawned session has finished cleaning
    /// up. Call after [`shutdown`](Self::shutdown).
    pub(crate) async fn wait_for_sessions(&self) {
        self.sessions.wait().await;
    }

    /// Stop accepting usage emissions and wait for every event spawned by a
    /// completed session to enter the emitter queue.
    pub(crate) async fn wait_for_usage_events(&self) {
        if let Some(emitter) = &self.usage_emitter {
            emitter.close_and_wait().await;
        }
    }

    /// Serve one accepted connection: WebSocket handshake, authentication,
    /// then the subscription session until the peer disconnects.
    ///
    /// Returns `Ok(())` without serving when the server is at its client
    /// limit, exactly as the listener loop does.
    pub(crate) async fn serve(&self, stream: TcpStream, remote_addr: SocketAddr) -> Result<()> {
        if self.client_manager.client_count() >= self.max_clients {
            warn!(
                "Rejecting connection from {}: max clients reached",
                remote_addr
            );
            return Ok(());
        }

        let context = SubscriptionContext {
            client_id: Uuid::nil(),
            client_manager: self.client_manager.clone(),
            bus_manager: self.bus_manager.clone(),
            entity_cache: self.entity_cache.clone(),
            view_index: self.view_index.clone(),
            usage_emitter: self.usage_emitter.clone(),
            journal: self.journal.clone(),
            delivery: self.delivery.clone(),
            metrics: self.metrics.clone(),
            shutdown: self.shutdown.clone(),
        };
        handle_connection(stream, context, remote_addr, self.auth_plugin.clone()).await
    }

    /// Accept from `listener` until [`shutdown`](Self::shutdown), serving each
    /// connection on its own task.
    pub(crate) async fn serve_listener(self, listener: TcpListener) -> Result<()> {
        loop {
            let accepted = tokio::select! {
                _ = self.shutdown.cancelled() => return Ok(()),
                accepted = listener.accept() => accepted,
            };
            match accepted {
                Ok((stream, addr)) => {
                    let acceptor = self.clone();
                    self.sessions.spawn(
                        async move {
                            if let Err(error) = acceptor.serve(stream, addr).await {
                                error!("WebSocket connection error: {}", error);
                            }
                        }
                        .instrument(info_span!("ws.connection", %addr)),
                    );
                }
                Err(error) => error!("Failed to accept connection: {}", error),
            }
        }
    }
}

#[derive(Debug, Clone)]
struct HandshakeReject {
    status: StatusCode,
    body: crate::websocket::auth::ErrorResponse,
    error_code: String,
    retry_after_secs: Option<u64>,
}

impl HandshakeReject {
    fn from_deny(deny: &AuthDeny) -> Self {
        let retry_after_secs = match deny.retry_policy {
            crate::websocket::auth::RetryPolicy::RetryAfter(duration) => Some(duration.as_secs()),
            _ => None,
        };
        Self {
            status: StatusCode::from_u16(deny.http_status).unwrap_or(StatusCode::UNAUTHORIZED),
            body: deny.to_error_response(),
            error_code: deny.code.to_string(),
            retry_after_secs,
        }
    }
}

fn build_handshake_error_response(
    response: &Response,
    reject: &HandshakeReject,
) -> HandshakeErrorResponse {
    let mut builder = Response::builder()
        .status(reject.status)
        .version(response.version())
        .header(CONTENT_TYPE, "application/json; charset=utf-8")
        .header("X-Error-Code", &reject.error_code)
        .header("Cache-Control", "no-store");
    if let Some(retry_after_secs) = reject.retry_after_secs {
        builder = builder.header("Retry-After", retry_after_secs.to_string());
    }
    let body = serde_json::to_string(&reject.body).unwrap_or_else(|_| {
        format!(
            r#"{{"error":"{}","message":"{}","code":"{}","retryable":false}}"#,
            reject.body.error, reject.body.message, reject.body.code
        )
    });
    builder
        .body(Some(body))
        .expect("handshake rejection response should build")
}

#[allow(clippy::result_large_err)]
async fn accept_authorized_connection(
    stream: TcpStream,
    remote_addr: SocketAddr,
    auth_plugin: Arc<dyn WebSocketAuthPlugin>,
    client_manager: ClientManager,
) -> Result<
    Option<(
        tokio_tungstenite::WebSocketStream<TcpStream>,
        AuthContext,
        Option<Arc<dyn WebSocketConnectionPermit>>,
    )>,
> {
    use std::sync::Mutex;

    type Admission =
        Result<(AuthContext, Option<Arc<dyn WebSocketConnectionPermit>>), HandshakeReject>;
    let capture: Arc<Mutex<Option<Admission>>> = Arc::new(Mutex::new(None));
    let capture_ref = capture.clone();
    let auth_plugin_ref = auth_plugin.clone();
    let manager_ref = client_manager.clone();

    let handshake_result = accept_hdr_async(stream, move |request: &Request, response| {
        let request = ConnectionAuthRequest::from_http_request(remote_addr, request);
        let result = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                match auth_plugin_ref.authorize(&request).await {
                    AuthDecision::Allow(context) => manager_ref
                        .check_connection_allowed(remote_addr, &Some(context.clone()))
                        .await
                        .and_then(|()| manager_ref.reserve_connection_admission(&context))
                        .map(|permit| (context, permit))
                        .map_err(|deny| HandshakeReject::from_deny(&deny)),
                    AuthDecision::Deny(deny) => Err(HandshakeReject::from_deny(&deny)),
                }
            })
        });
        *capture_ref.lock().expect("capture lock poisoned") = Some(result.clone());
        match result {
            Ok(_) => Ok(response),
            Err(reject) => Err(build_handshake_error_response(&response, &reject)),
        }
    })
    .await;

    let auth_result = capture.lock().expect("capture lock poisoned").take();
    match handshake_result {
        Ok(stream) => match auth_result {
            Some(Ok((context, permit))) => Ok(Some((stream, context, permit))),
            Some(Err(reject)) => Err(anyhow::anyhow!(
                "handshake unexpectedly succeeded after rejection: {}",
                reject.body.message
            )),
            None => Err(anyhow::anyhow!("no auth result captured during handshake")),
        },
        Err(WsError::Http(_)) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

async fn handle_connection(
    stream: TcpStream,
    mut context: SubscriptionContext,
    remote_addr: SocketAddr,
    auth_plugin: Arc<dyn WebSocketAuthPlugin>,
) -> Result<()> {
    // The handshake is raced against shutdown too: a peer that stalls it must
    // not keep a task alive after the server has stopped.
    let accepted = tokio::select! {
        _ = context.shutdown.cancelled() => return Ok(()),
        accepted = accept_authorized_connection(
            stream,
            remote_addr,
            auth_plugin.clone(),
            context.client_manager.clone(),
        ) => accepted?,
    };
    let Some((ws_stream, auth_context, admission_permit)) = accepted else {
        return Ok(());
    };

    let client_id = Uuid::new_v4();
    context.client_id = client_id;
    let connection_start = Instant::now();
    let (mut usage_identity, mut deployment_id) = usage_identity_from_context(Some(&auth_context));
    let mut metering_key = usage_identity.metering_key.clone();
    // Active gauges must be decremented with the same attributes used for
    // their increment. The signed usage identity may legitimately move to a
    // claimed owner's account during an in-band refresh, but that must not
    // leave the original account's active count stuck or make the new one
    // negative when this connection closes.
    let connection_metrics_metering_key = metering_key.clone();
    context
        .metrics
        .connection_opened(connection_metrics_metering_key.as_deref());

    let (ws_sender, mut ws_receiver) = ws_stream.split();
    context.client_manager.add_client_with_admission(
        client_id,
        ws_sender,
        Some(auth_context),
        remote_addr,
        admission_permit,
    );
    emit_usage_event(
        &context.usage_emitter,
        WebSocketUsageEvent::ConnectionEstablished {
            client_id: client_id.to_string(),
            remote_addr: remote_addr.to_string(),
            deployment_id: deployment_id.clone(),
            identity: usage_identity.clone(),
        },
    );

    let mut active_subscriptions: HashMap<String, ActiveSubscription> = HashMap::new();
    loop {
        let message = tokio::select! {
            _ = context.shutdown.cancelled() => break,
            next = ws_receiver.next() => match next {
                Some(message) => message,
                None => break,
            },
        };
        let message = match message {
            Ok(message) => message,
            Err(error) => {
                warn!("WebSocket error for client {}: {}", client_id, error);
                break;
            }
        };
        if message.is_close() {
            break;
        }
        context.client_manager.update_client_last_seen(client_id);
        if !message.is_text() {
            continue;
        }
        if let Err(deny) = context
            .client_manager
            .check_inbound_message_allowed(client_id)
        {
            send_socket_issue(client_id, &context.client_manager, &deny, true, None).await;
            break;
        }
        context.metrics.message_received(metering_key.as_deref());

        let text = match message.to_text() {
            Ok(text) => text,
            Err(_) => continue,
        };
        let client_message = match serde_json::from_str::<ClientMessage>(text) {
            Ok(message) => message,
            Err(parse_error) => {
                let subscription_id = extract_subscription_id(text);
                send_protocol_issue(
                    client_id,
                    &context.client_manager,
                    &context.metrics,
                    subscription_id,
                    "malformed-message",
                    format!("invalid protocol v2 message: {parse_error}"),
                )
                .await;
                continue;
            }
        };

        match client_message {
            ClientMessage::Subscribe(subscription) => {
                let subscription_id = subscription.subscription_id.clone();
                if let Err(message) = subscription.validate() {
                    send_protocol_issue(
                        client_id,
                        &context.client_manager,
                        &context.metrics,
                        Some(subscription_id),
                        "invalid-subscription",
                        message,
                    )
                    .await;
                    continue;
                }
                if let Err(deny) = context
                    .client_manager
                    .check_subscription_allowed(client_id)
                    .await
                {
                    send_socket_issue(
                        client_id,
                        &context.client_manager,
                        &deny,
                        false,
                        Some(subscription_id),
                    )
                    .await;
                    continue;
                }

                let cancel_token = CancellationToken::new();
                let added = match context
                    .client_manager
                    .try_add_client_subscription(
                        client_id,
                        subscription_id.clone(),
                        cancel_token.clone(),
                    )
                    .await
                {
                    Ok(added) => added,
                    Err(deny) => {
                        send_socket_issue(
                            client_id,
                            &context.client_manager,
                            &deny,
                            false,
                            Some(subscription_id),
                        )
                        .await;
                        continue;
                    }
                };
                if !added {
                    send_protocol_issue(
                        client_id,
                        &context.client_manager,
                        &context.metrics,
                        Some(subscription_id),
                        "duplicate-subscription-id",
                        "subscriptionId is already active on this connection",
                    )
                    .await;
                    continue;
                }

                let view = subscription.query.view.clone();
                if let Err(error) = attach_client_to_bus(&context, subscription, cancel_token).await
                {
                    context
                        .client_manager
                        .remove_client_subscription(client_id, &subscription_id)
                        .await;
                    // A refusal that already knows what to tell the client
                    // (an expired cursor, a changed epoch) keeps its own
                    // frame; anything else is a generic rejection.
                    match error.downcast::<RejectedSubscription>() {
                        Ok(rejected) => {
                            send_prepared_issue(
                                client_id,
                                &context.client_manager,
                                &context.metrics,
                                rejected.0,
                            )
                            .await;
                        }
                        Err(error) => {
                            send_protocol_issue(
                                client_id,
                                &context.client_manager,
                                &context.metrics,
                                Some(subscription_id),
                                "subscription-rejected",
                                error.to_string(),
                            )
                            .await;
                        }
                    }
                    continue;
                }

                active_subscriptions.insert(
                    subscription_id,
                    ActiveSubscription {
                        view: view.clone(),
                        metrics_metering_key: metering_key.clone(),
                    },
                );
                context
                    .metrics
                    .subscription_created(&view, metering_key.as_deref());
                emit_usage_event(
                    &context.usage_emitter,
                    WebSocketUsageEvent::SubscriptionCreated {
                        client_id: client_id.to_string(),
                        deployment_id: deployment_id.clone(),
                        identity: usage_identity.clone(),
                        view_id: view,
                    },
                );
            }
            ClientMessage::Unsubscribe(unsubscription) => {
                handle_unsubscribe(
                    &context,
                    unsubscription,
                    &mut active_subscriptions,
                    &deployment_id,
                    &usage_identity,
                )
                .await;
            }
            ClientMessage::Ping => debug!("Received ping from client {}", client_id),
            ClientMessage::RefreshAuth(request) => {
                handle_refresh_auth(client_id, &request, &context.client_manager, &auth_plugin)
                    .await;
                // A successful refresh replaces the verified context. Keep
                // every later usage event on the same current identity used
                // by snapshots and updates; on failure the manager still
                // returns the previous context.
                if let Some(current_auth) = context.client_manager.get_auth_context(client_id) {
                    (usage_identity, deployment_id) =
                        usage_identity_from_context(Some(&current_auth));
                    metering_key = usage_identity.metering_key.clone();
                }
            }
        }
    }

    context
        .client_manager
        .cancel_all_client_subscriptions(client_id)
        .await;
    context.client_manager.remove_client(client_id);
    if let Some(rate_limiter) = context.client_manager.rate_limiter().cloned() {
        rate_limiter.remove_client_buckets(client_id).await;
    }
    for subscription in active_subscriptions.values() {
        context.metrics.subscription_removed(
            &subscription.view,
            subscription.metrics_metering_key.as_deref(),
        );
        emit_usage_event(
            &context.usage_emitter,
            WebSocketUsageEvent::SubscriptionRemoved {
                client_id: client_id.to_string(),
                deployment_id: deployment_id.clone(),
                identity: usage_identity.clone(),
                view_id: subscription.view.clone(),
            },
        );
    }
    let duration = connection_start.elapsed().as_secs_f64();
    context
        .metrics
        .connection_closed(duration, connection_metrics_metering_key.as_deref());
    emit_usage_event(
        &context.usage_emitter,
        WebSocketUsageEvent::ConnectionClosed {
            client_id: client_id.to_string(),
            deployment_id,
            identity: usage_identity,
            duration_secs: Some(duration),
            subscription_count: u32::try_from(active_subscriptions.len()).unwrap_or(u32::MAX),
        },
    );
    Ok(())
}

struct ActiveSubscription {
    view: String,
    /// Attribute set used for the active-gauge increment. It is immutable
    /// even if a later token refresh moves usage to another billing account.
    metrics_metering_key: Option<String>,
}

async fn handle_unsubscribe(
    context: &SubscriptionContext,
    unsubscription: Unsubscription,
    active_subscriptions: &mut HashMap<String, ActiveSubscription>,
    deployment_id: &Option<String>,
    usage_identity: &UsageIdentity,
) {
    let subscription_id = unsubscription.subscription_id.clone();
    if let Err(message) = unsubscription.validate() {
        send_protocol_issue(
            context.client_id,
            &context.client_manager,
            &context.metrics,
            Some(subscription_id),
            "invalid-unsubscription",
            message,
        )
        .await;
        return;
    }

    if !context
        .client_manager
        .remove_client_subscription(context.client_id, &subscription_id)
        .await
    {
        send_protocol_issue(
            context.client_id,
            &context.client_manager,
            &context.metrics,
            Some(subscription_id),
            "unknown-subscription-id",
            "subscriptionId is not active on this connection",
        )
        .await;
        return;
    }

    let Some(subscription) = active_subscriptions.remove(&subscription_id) else {
        return;
    };
    let _ = send_control_frame(
        context,
        &UnsubscribedFrame::new(subscription_id),
        &subscription.view,
    );
    context.metrics.subscription_removed(
        &subscription.view,
        subscription.metrics_metering_key.as_deref(),
    );
    emit_usage_event(
        &context.usage_emitter,
        WebSocketUsageEvent::SubscriptionRemoved {
            client_id: context.client_id.to_string(),
            deployment_id: deployment_id.clone(),
            identity: usage_identity.clone(),
            view_id: subscription.view,
        },
    );
}

fn extract_subscription_id(text: &str) -> Option<String> {
    serde_json::from_str::<Value>(text)
        .ok()?
        .get("subscriptionId")?
        .as_str()
        .map(str::to_string)
}

struct SnapshotMetadata<'a> {
    subscription_id: &'a str,
    snapshot_id: &'a str,
    authoritative: bool,
    mode: Mode,
    view_id: &'a str,
    key: Option<&'a str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SnapshotPurpose {
    Initial,
    Recovery,
}

impl SnapshotPurpose {
    fn authoritative(self, subscription: &Subscription) -> bool {
        match self {
            Self::Initial => subscription.query.after.is_none(),
            // Recovery replaces the exact query membership even when `after`
            // made the initial snapshot incremental. A merge-only snapshot
            // cannot remove members whose delete frames were skipped.
            Self::Recovery => true,
        }
    }
}

/// One snapshot frame, serialized straight from the cached entities it
/// carries: the same JSON as a [`SnapshotFrame`] of
/// [`SnapshotEntity`](crate::websocket::frame::SnapshotEntity) rows with the
/// view's wire format applied, without building those rows.
///
/// [`SnapshotFrame`]: crate::websocket::frame::SnapshotFrame
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SnapshotBatch<'a> {
    protocol_version: u8,
    subscription_id: &'a str,
    snapshot_id: &'a str,
    authoritative: bool,
    mode: Mode,
    #[serde(rename = "entity")]
    export: &'a str,
    op: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    key: Option<&'a str>,
    data: SnapshotRows<'a>,
    complete: bool,
}

struct SnapshotRows<'a> {
    rows: &'a [(String, SharedEntity)],
    wire_format: &'a WireFormat,
}

impl Serialize for SnapshotRows<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeSeq as _;
        let mut rows = serializer.serialize_seq(Some(self.rows.len()))?;
        for (key, entity) in self.rows {
            rows.serialize_element(&SnapshotRow {
                key,
                data: WireEntity {
                    entity,
                    wire_format: self.wire_format,
                },
            })?;
        }
        rows.end()
    }
}

#[derive(Serialize)]
struct SnapshotRow<'a> {
    key: &'a str,
    data: WireEntity<'a>,
}

/// Split `rows` into the snapshot's frames: the first batch for a fast first
/// render, then larger ones. An empty snapshot is one empty, complete frame.
fn create_snapshot_batches<'a>(
    rows: &'a [(String, SharedEntity)],
    wire_format: &'a WireFormat,
    metadata: SnapshotMetadata<'a>,
    batch_config: &SnapshotBatchConfig,
) -> Vec<SnapshotBatch<'a>> {
    let batch = |data: &'a [(String, SharedEntity)], complete: bool| SnapshotBatch {
        protocol_version: PROTOCOL_VERSION,
        subscription_id: metadata.subscription_id,
        snapshot_id: metadata.snapshot_id,
        authoritative: metadata.authoritative,
        mode: metadata.mode,
        export: metadata.view_id,
        op: "snapshot",
        key: metadata.key,
        data: SnapshotRows {
            rows: data,
            wire_format,
        },
        complete,
    };
    if rows.is_empty() {
        return vec![batch(&[], true)];
    }

    let mut batches = Vec::new();
    let mut offset = 0;
    while offset < rows.len() {
        let configured_size = if offset == 0 {
            batch_config.initial_batch_size
        } else {
            batch_config.subsequent_batch_size
        };
        let end = (offset + configured_size.max(1)).min(rows.len());
        batches.push(batch(&rows[offset..end], end == rows.len()));
        offset = end;
    }
    batches
}

/// Send `rows` as one snapshot, serializing each frame from the cached
/// entities as it goes: no row is copied.
async fn send_snapshot_batches(
    context: &SubscriptionContext,
    subscription: &Subscription,
    rows: &[(String, SharedEntity)],
    view_spec: &ViewSpec,
    purpose: SnapshotPurpose,
    batch_config: &SnapshotBatchConfig,
) -> Result<()> {
    let snapshot_id = Uuid::new_v4().to_string();
    let authoritative = purpose.authoritative(subscription);
    let mode = view_spec.mode;
    let frames = create_snapshot_batches(
        rows,
        &view_spec.wire_format,
        SnapshotMetadata {
            subscription_id: &subscription.subscription_id,
            snapshot_id: &snapshot_id,
            authoritative,
            mode,
            view_id: &subscription.query.view,
            key: subscription.query.key.as_deref(),
        },
        batch_config,
    );

    for frame in frames {
        let rows = frame.data.rows.len() as u32;
        let json = serde_json::to_vec(&frame)?;
        let payload = maybe_compress(&json);
        let bytes = payload.as_bytes().len() as u64;
        context
            .client_manager
            .send_compressed_async(context.client_id, payload)
            .await
            .map_err(|error| anyhow::anyhow!("failed to send snapshot: {error}"))?;
        context.metrics.message_sent();

        let auth_context = context.client_manager.get_auth_context(context.client_id);
        let (identity, deployment_id) = usage_identity_from_context(auth_context.as_ref());
        emit_usage_event(
            &context.usage_emitter,
            WebSocketUsageEvent::SnapshotSent {
                client_id: context.client_id.to_string(),
                deployment_id,
                identity,
                view_id: subscription.query.view.clone(),
                rows,
                messages: 1,
                bytes,
            },
        );
    }
    Ok(())
}

fn extract_sort_config(view_spec: &ViewSpec) -> Option<SortConfig> {
    if let Some(sort) = view_spec
        .pipeline
        .as_ref()
        .and_then(|pipeline| pipeline.sort.as_ref())
    {
        return Some(SortConfig {
            field: sort.field_path.clone(),
            order: match sort.order {
                crate::materialized_view::SortOrder::Asc => SortOrder::Asc,
                crate::materialized_view::SortOrder::Desc => SortOrder::Desc,
            },
        });
    }
    (view_spec.mode == Mode::List).then(|| SortConfig {
        field: vec!["_seq".to_string()],
        order: SortOrder::Desc,
    })
}

fn send_control_frame<T: Serialize>(
    context: &SubscriptionContext,
    frame: &T,
    view_id: &str,
) -> Result<()> {
    let json = serde_json::to_vec(frame)?;
    let bytes = json.len();
    context
        .client_manager
        .send_to_client(context.client_id, Arc::new(Bytes::from(json)))
        .map_err(|error| anyhow::anyhow!("failed to send control frame: {error}"))?;
    context.metrics.message_sent();
    emit_update_sent_for_client(
        &context.usage_emitter,
        &context.client_manager,
        context.client_id,
        view_id,
        bytes,
    );
    Ok(())
}

fn send_subscribed_frame(
    context: &SubscriptionContext,
    subscription: &Subscription,
    view_spec: &ViewSpec,
) -> Result<()> {
    let frame = SubscribedFrame::new(
        subscription.subscription_id.clone(),
        subscription.query.clone(),
        view_spec.mode,
        extract_sort_config(view_spec),
    );
    send_control_frame(context, &frame, &subscription.query.view)
}

fn enforce_snapshot_limit(context: &SubscriptionContext, rows: usize) -> Result<()> {
    context
        .client_manager
        .check_snapshot_allowed(context.client_id, u32::try_from(rows).unwrap_or(u32::MAX))
        .map_err(|deny| anyhow::anyhow!(deny.reason))
}

async fn subscribe_state_then_snapshot<F, Fut, T>(
    bus_manager: &BusManager,
    view_id: &str,
    key: &str,
    snapshot: F,
) -> (watch::Receiver<StateUpdate>, u64, T)
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = T>,
{
    let mut receiver = bus_manager.get_or_create_state_bus(view_id, key).await;
    let published = receiver.borrow_and_update().published;
    let snapshot = snapshot().await;
    (receiver, published, snapshot)
}

async fn subscribe_list_then_snapshot<F, Fut, T>(
    bus_manager: &BusManager,
    view_id: &str,
    snapshot: F,
) -> (broadcast::Receiver<Arc<BusMessage>>, T)
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = T>,
{
    let receiver = bus_manager.get_or_create_list_bus(view_id).await;
    let snapshot = snapshot().await;
    (receiver, snapshot)
}

async fn attach_client_to_bus(
    context: &SubscriptionContext,
    mut subscription: Subscription,
    cancel_token: CancellationToken,
) -> Result<()> {
    let view_spec = context
        .view_index
        .get_view(&subscription.query.view)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("unknown view: {}", subscription.query.view))?;

    if view_spec.mode == Mode::State && !view_spec.is_derived() && subscription.query.key.is_none()
    {
        return Err(anyhow::anyhow!("state subscriptions require query.key"));
    }
    if view_spec.is_derived() && subscription.query.take.is_none() {
        subscription.query.take = view_spec
            .pipeline
            .as_ref()
            .and_then(|pipeline| pipeline.limit);
    }

    // A retained tape takes precedence for append views: it is the only
    // delivery that can honour a cursor. Without one, fall through to the
    // previous latest-state behaviour.
    let journal = context
        .journal
        .clone()
        .filter(|journal| journal.is_enabled() && view_spec.mode == Mode::Append);
    if let Some(journal) = journal {
        return attach_journal_subscription(
            context,
            subscription,
            view_spec,
            journal,
            cancel_token,
        )
        .await;
    }

    if view_spec.mode == Mode::State && !view_spec.is_derived() {
        attach_state_subscription(context, subscription, view_spec, cancel_token).await
    } else {
        attach_collection_subscription(context, subscription, view_spec, cancel_token).await
    }
}

async fn attach_state_subscription(
    context: &SubscriptionContext,
    subscription: Subscription,
    view_spec: ViewSpec,
    cancel_token: CancellationToken,
) -> Result<()> {
    let view_id = subscription.query.view.clone();
    let key = subscription.query.key.clone().unwrap_or_default();
    let query = subscription.query.clone();
    let cache = context.entity_cache.clone();
    let view_spec_for_snapshot = view_spec.clone();
    let (mut receiver, mut seen, initial) =
        subscribe_state_then_snapshot(&context.bus_manager, &view_id, &key, move || async move {
            load_query_entities(&cache, None, &view_spec_for_snapshot, &query, false).await
        })
        .await;

    let mut snapshot_entities = initial;
    if let Some(limit) = subscription.query.snapshot_limit {
        snapshot_entities.truncate(limit);
    }
    enforce_snapshot_limit(context, snapshot_entities.len())?;
    send_subscribed_frame(context, &subscription, &view_spec)?;
    // The client holds the entity only if the snapshot actually carried it.
    // A disabled snapshot, or one `snapshotLimit` cut to nothing, leaves it
    // without a copy, so its first change must be a full `upsert`: a patch
    // would have nothing to merge into.
    let delivered = subscription.snapshot.enabled && !snapshot_entities.is_empty();
    // The entity as the client last received it whole, which a catch-up is
    // measured against.
    let synced = if delivered {
        snapshot_entities
            .first()
            .map(|(_, entity)| Synced::Entity(entity.clone()))
    } else {
        None
    };
    if subscription.snapshot.enabled {
        send_snapshot_batches(
            context,
            &subscription,
            &snapshot_entities,
            &view_spec,
            SnapshotPurpose::Initial,
            &context.entity_cache.snapshot_config(),
        )
        .await?;
    }

    let task_context = context.clone();
    let subscription_id = subscription.subscription_id.clone();
    let query = subscription.query.clone();
    let view_spec_task = view_spec.clone();
    let span_view = view_id.clone();
    let span_key = key.clone();
    tokio::spawn(
        async move {
            // Whether the client holds this key, not whether it exists.
            let mut member = delivered;
            // The entity as the client last received it whole (its snapshot
            // row, an upsert or the last catch-up; see `Synced`). A catch-up
            // sends what changed since, and every field a patch forwarded in
            // between set (`touched`, see `mark_fields`): the entity may have
            // set it back, and a client may have dropped the patch.
            let mut synced = synced;
            let mut touched = Value::Null;
            // Whether the client lacks fields of frames the bus overwrote
            // before this task read them. The cached entity is sent in their
            // place, but while the cache lacks the key there is none, so the
            // debt waits for the next frame that finds it cached.
            let mut behind = false;
            // Whether, while behind, a frame the client missed replaced the
            // entity (a delete, or a whole upsert): its copy may then hold
            // fields the entity no longer has, which merging keeps, so the
            // catch-up must replace it.
            let mut replaced = false;
            loop {
                tokio::select! {
                    _ = cancel_token.cancelled() => break,
                    changed = receiver.changed() => {
                        if changed.is_err() {
                            break;
                        }
                        let (payload, published, replaced_at) = {
                            let update = receiver.borrow_and_update();
                            (update.payload.clone(), update.published, update.replaced)
                        };
                        // The bus keeps only the latest frame. If more than one
                        // was published since the last read, the earlier ones
                        // were overwritten and the latest patch alone would
                        // drop their fields, so send the cached entity instead.
                        behind |= published > seen + 1;
                        replaced |= behind && replaced_at > seen;
                        seen = published;
                        let metadata = source_frame_metadata(&payload);
                        if metadata.op == "delete" {
                            if !task_context.entity_cache
                                .deletion_is_current(&query.view, &key)
                                .await
                            {
                                behind = true;
                                replaced = true;
                                continue;
                            }
                            if member && send_membership_frame(
                                &task_context,
                                &subscription_id,
                                &view_spec_task,
                                "delete",
                                &key,
                                Value::Null,
                                metadata.seq,
                            ).is_err() {
                                break;
                            }
                            member = false;
                            behind = false;
                            replaced = false;
                            synced = None;
                            touched = Value::Null;
                            continue;
                        }

                        // A key missing from the cache was evicted there or
                        // never held whole, not deleted (deletes arrive
                        // above): the cache refused this patch and the whole
                        // entity is on its way (see `EntityResync`). The
                        // client's copy is still good, so a holder gets the
                        // patch and stays a holder; `remove` is only for an
                        // entity that stopped matching. If it is behind, it
                        // stays behind: the resend that brings the entity
                        // back carries the seq of the latest change, which it
                        // already has, so only the catch-up below reaches it.
                        let Some(cached) = task_context
                            .entity_cache
                            .get_shared(&view_spec_task.id, &key)
                            .await
                        else {
                            // A copy of a replaced entity takes no patch: it
                            // waits for the whole entity to replace it.
                            if member && !replaced {
                                record_forwarded(&payload, &metadata.op, &mut synced, &mut touched);
                                if send_scoped_source_payload(
                                    &task_context,
                                    &subscription_id,
                                    &query.view,
                                    payload,
                                ).is_err() {
                                    break;
                                }
                            }
                            continue;
                        };
                        let selected =
                            select_query_entities(vec![(key.clone(), cached)], &query, true, false);
                        let is_member = !selected.is_empty();
                        let caught_up = !std::mem::take(&mut behind);
                        let replace = std::mem::take(&mut replaced);
                        let result = match (member, selected.into_iter().next()) {
                            (true, Some(_)) if caught_up => {
                                record_forwarded(&payload, &metadata.op, &mut synced, &mut touched);
                                send_scoped_source_payload(
                                    &task_context,
                                    &subscription_id,
                                    &query.view,
                                    payload,
                                )
                            }
                            // The catch-up: a patch of the cached entity's
                            // fields that changed since the client last received
                            // it whole or that a forwarded patch set, which
                            // merges into what the client holds.
                            // If a missed frame replaced the entity, it is the
                            // whole entity as an upsert instead, which replaces
                            // the client's copy: merging would keep the fields of
                            // the entity that was deleted or replaced. Either
                            // carries no seq, because it is newer than anything
                            // this subscriber was sent and seqs are not ordered
                            // within a slot: account updates and instructions
                            // number themselves differently, so the latest
                            // patch's seq can sort below one already delivered.
                            (true, Some((entity_key, data))) => {
                                let wire_format = &view_spec_task.wire_format;
                                let base = synced
                                    .as_ref()
                                    .filter(|_| !replace)
                                    .and_then(|synced| synced.wire(wire_format));
                                let result = match base {
                                    Some(base) => match changed_fields(
                                        &base,
                                        &wire_value(&data, wire_format),
                                        &touched,
                                    ) {
                                        Some(body) => send_membership_frame(
                                            &task_context,
                                            &subscription_id,
                                            &view_spec_task,
                                            "patch",
                                            &entity_key,
                                            body,
                                            None,
                                        ),
                                        None => Ok(()),
                                    },
                                    // Nothing to measure against: the whole
                                    // entity, straight from the cache.
                                    None => send_entity_frame(
                                        &task_context,
                                        &subscription_id,
                                        &view_spec_task,
                                        if replace { "upsert" } else { "patch" },
                                        &entity_key,
                                        &data,
                                        None,
                                    ),
                                };
                                synced = Some(Synced::Entity(data));
                                touched = Value::Null;
                                result
                            }
                            (false, Some((entity_key, data))) => {
                                let result = send_entity_frame(
                                    &task_context,
                                    &subscription_id,
                                    &view_spec_task,
                                    "upsert",
                                    &entity_key,
                                    &data,
                                    metadata.seq.as_deref(),
                                );
                                synced = Some(Synced::Entity(data));
                                touched = Value::Null;
                                result
                            }
                            (true, None) => {
                                synced = None;
                                touched = Value::Null;
                                send_membership_frame(
                                &task_context,
                                &subscription_id,
                                &view_spec_task,
                                "remove",
                                &key,
                                Value::Null,
                                metadata.seq,
                                )
                            }
                            (false, None) => Ok(()),
                        };
                        if result.is_err() {
                            break;
                        }
                        member = is_member;
                    }
                }
            }
        }
        .instrument(info_span!("ws.subscribe.state", client_id = %context.client_id, view = %span_view, key = %span_key)),
    );
    Ok(())
}

async fn attach_collection_subscription(
    context: &SubscriptionContext,
    subscription: Subscription,
    view_spec: ViewSpec,
    cancel_token: CancellationToken,
) -> Result<()> {
    let view_id = subscription.query.view.clone();
    let source_view_id = view_spec
        .source_view
        .clone()
        .unwrap_or_else(|| view_id.clone());
    let (mut receiver, initial_membership) = subscribe_collection_then_snapshot(
        context,
        &source_view_id,
        &view_spec,
        &subscription.query,
    )
    .await;

    let mut snapshot_entities = initial_membership.clone();
    if let Some(limit) = subscription.query.snapshot_limit {
        snapshot_entities.truncate(limit);
    }
    enforce_snapshot_limit(context, snapshot_entities.len())?;
    send_subscribed_frame(context, &subscription, &view_spec)?;
    // Window members the client was never sent: everything past a
    // `snapshotLimit` cut, or the whole window when the snapshot is disabled.
    // They stay in the window (take/skip positions still count them) but the
    // client holds no copy, so none of them may ever be sent a patch.
    let delivered_rows = if subscription.snapshot.enabled {
        snapshot_entities.len()
    } else {
        0
    };
    let mut undelivered: HashSet<String> = initial_membership
        .iter()
        .skip(delivered_rows)
        .map(|(key, _)| key.clone())
        .collect();
    if subscription.snapshot.enabled {
        send_snapshot_batches(
            context,
            &subscription,
            &snapshot_entities,
            &view_spec,
            SnapshotPurpose::Initial,
            &context.entity_cache.snapshot_config(),
        )
        .await?;
    }

    let task_context = context.clone();
    let task_subscription = subscription.clone();
    let subscription_id = subscription.subscription_id.clone();
    let query = subscription.query.clone();
    let view_spec_task = view_spec.clone();
    let span_view = view_id.clone();
    tokio::spawn(
        async move {
            let mut current = initial_membership;
            // Append views are event tapes: collapsing two records would lose
            // observable history. Coalescing is only valid for latest-state
            // list membership, where a full final entity preserves meaning.
            let coalesce_ms = (view_spec_task.mode == Mode::List)
                .then(|| {
                    view_spec_task
                        .delivery
                        .coalesce_ms
                        .or(task_context.delivery.collection_coalesce_ms)
                        .filter(|milliseconds| *milliseconds > 0)
                })
                .flatten();
            let mut flush_interval = coalesce_ms.map(|milliseconds| {
                let period = Duration::from_millis(milliseconds);
                let mut interval = tokio::time::interval_at(
                    tokio::time::Instant::now() + period,
                    period,
                );
                interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                interval
            });
            let mut pending = HashMap::<String, Arc<BusMessage>>::new();
            let mut pending_updates = 0_u64;

            loop {
                tokio::select! {
                    _ = cancel_token.cancelled() => break,
                    _ = async {
                        flush_interval
                            .as_mut()
                            .expect("coalescing interval is guarded")
                            .tick()
                            .await;
                    }, if flush_interval.is_some() => {
                        if pending.is_empty() {
                            continue;
                        }
                        let sorted_caches = view_spec_task
                            .is_derived()
                            .then(|| task_context.view_index.sorted_caches());
                        let next = load_query_entities(
                            &task_context.entity_cache,
                            sorted_caches,
                            &view_spec_task,
                            &query,
                            false,
                        ).await;
                        if emit_coalesced_collection_delta(
                            &task_context,
                            &subscription_id,
                            &view_spec_task,
                            &current,
                            &next,
                            &pending,
                            &mut undelivered,
                        ).is_err() {
                            task_context.metrics.delivery_stopped(&view_id, "send-failed");
                            break;
                        }
                        task_context
                            .metrics
                            .collection_coalesced(&view_id, pending_updates);
                        current = next;
                        pending.clear();
                        pending_updates = 0;
                    }
                    received = receiver.recv() => {
                        let envelope = match received {
                            Ok(envelope) => envelope,
                            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                                task_context.metrics.subscription_lagged(&view_id, skipped);
                                warn!(
                                    "Subscription {} lagged by {} updates",
                                    subscription_id, skipped
                                );
                                if view_spec_task.mode == Mode::Append {
                                    let _ = send_control_frame(
                                        &task_context,
                                        &SocketIssueMessage::append_subscription_lagged(
                                            subscription_id.clone(),
                                            skipped,
                                        ),
                                        &view_id,
                                    );
                                    task_context.metrics.delivery_stopped(&view_id, "append-lagged-without-replay");
                                    break;
                                }
                                if !task_subscription.snapshot.enabled {
                                    let _ = send_control_frame(
                                        &task_context,
                                        &SocketIssueMessage::subscription_lagged(
                                            subscription_id.clone(),
                                            skipped,
                                        ),
                                        &view_id,
                                    );
                                    task_context.metrics.delivery_stopped(&view_id, "lagged-without-snapshot");
                                    break;
                                }
                                info!(
                                    "Subscription {} is recovering from an authoritative snapshot",
                                    subscription_id
                                );
                                match recover_collection_subscription(
                                    &task_context,
                                    &task_subscription,
                                    &view_spec_task,
                                    &source_view_id,
                                ).await {
                                    Ok((next_receiver, recovered)) => {
                                        receiver = next_receiver;
                                        current = recovered;
                                        // Recovery replaces the whole live
                                        // membership, untruncated.
                                        undelivered.clear();
                                        pending.clear();
                                        pending_updates = 0;
                                        task_context.metrics.subscription_resnapshot(&view_id);
                                        continue;
                                    }
                                    Err(error) => {
                                        warn!(
                                            "Subscription {} failed to recover from lag: {error:#}",
                                            subscription_id
                                        );
                                        task_context.metrics.delivery_stopped(&view_id, "resnapshot-failed");
                                        break;
                                    }
                                }
                            }
                            Err(broadcast::error::RecvError::Closed) => break,
                        };

                        apply_collection_source_event(
                            &task_context,
                            &source_view_id,
                            &view_spec_task,
                            &query,
                            &envelope,
                        ).await;

                        if flush_interval.is_some() {
                            pending_updates = pending_updates.saturating_add(1);
                            pending.insert(envelope.key.clone(), envelope);
                            continue;
                        }

                        let metadata = source_frame_metadata(&envelope.payload);
                        let sorted_caches = view_spec_task
                            .is_derived()
                            .then(|| task_context.view_index.sorted_caches());
                        let next = load_query_entities(
                            &task_context.entity_cache,
                            sorted_caches,
                            &view_spec_task,
                            &query,
                            false,
                        ).await;
                        if emit_collection_delta(
                            &task_context,
                            &subscription_id,
                            &view_spec_task,
                            &current,
                            &next,
                            &envelope,
                            &metadata,
                            &mut undelivered,
                        ).is_err() {
                            task_context.metrics.delivery_stopped(&view_id, "send-failed");
                            break;
                        }
                        current = next;
                    }
                }
            }
        }
        .instrument(info_span!("ws.subscribe.collection", client_id = %context.client_id, view = %span_view)),
    );
    Ok(())
}

async fn subscribe_collection_then_snapshot(
    context: &SubscriptionContext,
    source_view_id: &str,
    view_spec: &ViewSpec,
    query: &SubscriptionQuery,
) -> (
    broadcast::Receiver<Arc<BusMessage>>,
    Vec<(String, SharedEntity)>,
) {
    let cache = context.entity_cache.clone();
    let sorted_caches = view_spec
        .is_derived()
        .then(|| context.view_index.sorted_caches());
    let view_spec = view_spec.clone();
    let query = query.clone();
    subscribe_list_then_snapshot(&context.bus_manager, source_view_id, move || async move {
        load_query_entities(&cache, sorted_caches, &view_spec, &query, false).await
    })
    .await
}

async fn recover_collection_subscription(
    context: &SubscriptionContext,
    subscription: &Subscription,
    view_spec: &ViewSpec,
    source_view_id: &str,
) -> Result<(
    broadcast::Receiver<Arc<BusMessage>>,
    Vec<(String, SharedEntity)>,
)> {
    let (receiver, membership) =
        subscribe_collection_then_snapshot(context, source_view_id, view_spec, &subscription.query)
            .await;
    // `snapshotLimit` caps only the initial transfer. Recovery has to replace
    // the complete live membership; truncating it would make the replacement
    // authoritative while immediately omitting members the server still
    // considers current.
    enforce_snapshot_limit(context, membership.len())?;
    send_snapshot_batches(
        context,
        subscription,
        &membership,
        view_spec,
        SnapshotPurpose::Recovery,
        &context.entity_cache.snapshot_config(),
    )
    .await?;
    Ok((receiver, membership))
}

async fn apply_collection_source_event(
    context: &SubscriptionContext,
    source_view_id: &str,
    view_spec: &ViewSpec,
    query: &SubscriptionQuery,
    envelope: &BusMessage,
) {
    let metadata = source_frame_metadata(&envelope.payload);
    if metadata.op != "delete" {
        return;
    }
    // The projector changed the shared cache before publishing this frame.
    // A slow subscription must inspect that authoritative lifetime state, not
    // replay the delete against `_seq` and potentially erase a recreation.
    if !context
        .entity_cache
        .deletion_is_current(source_view_id, &envelope.key)
        .await
    {
        return;
    }
    if view_spec.is_derived() {
        let caches = context.view_index.sorted_caches();
        let mut guard = caches.write().await;
        if let Some(cache) = guard.get_mut(&query.view) {
            cache.remove(&envelope.key);
        }
    }
}

/// A subscription refused for a reason the client needs spelled out.
///
/// Attach paths that return this get the registration released by the
/// connection loop, the same as any other failure, while the client still
/// receives the specific error rather than a generic `subscription-rejected`.
/// Sending the frame and returning `Ok` instead would leave a registered
/// subscription with nothing attached: it would hold a slot against the
/// client's limit and make the advertised "resubscribe" remediation fail with
/// `duplicate-subscription-id`.
#[derive(Debug)]
pub(crate) struct RejectedSubscription(pub SocketIssueMessage);

impl RejectedSubscription {
    fn into_error(issue: SocketIssueMessage) -> anyhow::Error {
        anyhow::Error::new(Self(issue))
    }
}

impl std::fmt::Display for RejectedSubscription {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0.message)
    }
}

impl std::error::Error for RejectedSubscription {}

/// Deliver an append view as an event tape: replay the retained records after
/// the cursor, then forward live frames.
///
/// This deliberately does not recompute membership from the entity cache the
/// way [`attach_collection_subscription`] does. The cache folds each patch
/// into the resident entity, so a membership diff cannot express "these three
/// events happened"; the retained records can.
///
/// For the same reason records are never promoted to a full `upsert` the
/// first time this subscription sees their key, the way a window entry is.
/// The cache only holds the latest state, not the state as of a record's
/// offset, so substituting it would hand a consumer state from after its
/// cursor. Records go out exactly as retained, `patch` included: a tape is
/// events, and the consumer that resumes from a cursor holds whatever came
/// before it. Clients therefore apply frames that carry an `offset` as events
/// and do not discard them as partial entities (see "Partial entities" in
/// `docs/websocket-v2-protocol.md`).
async fn attach_journal_subscription(
    context: &SubscriptionContext,
    subscription: Subscription,
    view_spec: ViewSpec,
    journal: Arc<crate::journal::EventJournal>,
    cancel_token: CancellationToken,
) -> Result<()> {
    let view_id = view_spec.id.clone();
    let subscription_id = subscription.subscription_id.clone();

    // A tape has no membership window, so `take`/`skip` cannot mean what they
    // mean on a list view. Refuse them rather than accept and ignore them.
    if subscription.query.take.is_some()
        || subscription.query.skip.is_some()
        || subscription.query.snapshot_limit.is_some()
    {
        return Err(RejectedSubscription::into_error(SocketIssueMessage::protocol(
            Some(subscription_id),
            "invalid-subscription",
            format!(
                "take, skip and snapshotLimit are window options and do not apply to the replayable view {view_id}"
            ),
        )));
    }

    // Reject a malformed cursor rather than silently replaying from the start,
    // which would look like success and duplicate everything.
    let cursor = match subscription.query.after.as_deref() {
        Some(raw) => match crate::journal::Cursor::parse(raw) {
            Some(cursor) => Some(cursor),
            None => {
                return Err(RejectedSubscription::into_error(
                    SocketIssueMessage::protocol(
                        Some(subscription_id),
                        "invalid-cursor",
                        format!(
                            "`after` must be an {{epoch}}:{{offset}} replay cursor for view {view_id}, got {raw:?}"
                        ),
                    ),
                ));
            }
        },
        None => None,
    };

    // Subscribe before reading the journal so anything published during the
    // replay is still delivered; the offset filter below drops the overlap.
    let mut receiver = context.bus_manager.get_or_create_list_bus(&view_id).await;

    let replayed = match journal.replay_after(&view_id, cursor.as_ref()).await {
        Ok(records) => records,
        Err(error) => {
            return Err(RejectedSubscription::into_error(
                SocketIssueMessage::replay_refused(Some(subscription_id), &error),
            ));
        }
    };

    let frame = SubscribedFrame::new(
        subscription.subscription_id.clone(),
        subscription.query.clone(),
        view_spec.mode,
        extract_sort_config(&view_spec),
    )
    .with_replay_window(journal.window(&view_id).await);
    send_control_frame(context, &frame, &view_id)?;

    // Everything past the acknowledgement runs on its own task. The replay can
    // be long and applies real backpressure, and `attach_client_to_bus` is
    // awaited directly on the connection's inbound loop — doing it there would
    // block unsubscribe, auth refresh and pong for the whole replay.
    let task_context = context.clone();
    let task_subscription_id = subscription.subscription_id.clone();
    let task_query = subscription.query.clone();
    let task_epoch = journal.epoch().await;
    let span_view = view_id.clone();
    tokio::spawn(
        async move {
            let mut last_sent = cursor.map(|cursor| cursor.offset);
            // Frames that published while the replay was still running. The
            // bus is a bounded broadcast, so it has to be drained as we go or
            // a busy view laps us before the replay finishes.
            let mut pending: VecDeque<Arc<BusMessage>> = VecDeque::new();
            let mut lagged: Option<u64> = None;

            for record in replayed {
                if cancel_token.is_cancelled() {
                    return;
                }
                drain_available(&mut receiver, &mut pending, &mut lagged);
                if journal_record_matches(&task_query, &record)
                    && send_scoped_source_payload_async(
                        &task_context,
                        &task_subscription_id,
                        &span_view,
                        record.payload,
                    )
                    .await
                    .is_err()
                {
                    return;
                }
                // Advance past filtered records too: they were considered.
                last_sent = Some(record.offset);
            }

            // Flush what arrived during the replay before going live, so the
            // handover keeps offset order.
            while let Some(envelope) = pending.pop_front() {
                if !forward_live_frame(
                    &task_context,
                    &task_subscription_id,
                    &span_view,
                    &task_query,
                    &envelope,
                    &mut last_sent,
                )
                .await
                {
                    return;
                }
            }

            if let Some(skipped) = lagged {
                report_replay_gap(
                    &task_context,
                    &task_subscription_id,
                    &span_view,
                    &task_epoch,
                    skipped,
                    last_sent,
                );
                return;
            }

            loop {
                tokio::select! {
                    _ = cancel_token.cancelled() => break,
                    received = receiver.recv() => {
                        let envelope = match received {
                            Ok(envelope) => envelope,
                            // A lagged tape is a gap. Report it with the last
                            // offset delivered *before* the gap and stop
                            // delivering on this subscription.
                            //
                            // Continuing would hand the consumer frames from
                            // after the gap, advancing its checkpoint past
                            // the skipped records so they could never be
                            // replayed. Stopping is not a silent stall: the
                            // consumer has an explicit error and a cursor
                            // that recovers exactly what it missed.
                            //
                            // The registration is deliberately left alone.
                            // Its lifecycle belongs to the connection loop,
                            // which holds the only handle to
                            // `active_subscriptions`; releasing half of it
                            // here would desynchronise unsubscribe, the
                            // duplicate-ID gate and close-time usage.
                            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                                report_replay_gap(
                                    &task_context,
                                    &task_subscription_id,
                                    &span_view,
                                    &task_epoch,
                                    skipped,
                                    last_sent,
                                );
                                break;
                            }
                            Err(broadcast::error::RecvError::Closed) => break,
                        };

                        if !forward_live_frame(
                            &task_context,
                            &task_subscription_id,
                            &span_view,
                            &task_query,
                            &envelope,
                            &mut last_sent,
                        )
                        .await
                        {
                            break;
                        }
                    }
                }
            }
        }
        .instrument(info_span!(
            "ws.subscribe.replay",
            client_id = %context.client_id,
            view = %view_id
        )),
    );
    Ok(())
}

/// Take whatever the bus already has without waiting, so a long replay cannot
/// be lapped by a busy view.
fn drain_available(
    receiver: &mut broadcast::Receiver<Arc<BusMessage>>,
    pending: &mut VecDeque<Arc<BusMessage>>,
    lagged: &mut Option<u64>,
) {
    // Once a gap is known, everything still on the bus is on the far side of
    // it. Buffering it would put post-gap frames in front of the lag report,
    // advancing `last_sent` past the hole and making `recoverFrom` point
    // after the very records it is supposed to recover.
    if lagged.is_some() {
        return;
    }
    // Bounded so a view publishing faster than the client drains cannot turn
    // the buffer into an unbounded queue. Filling it is not itself a gap:
    // nothing has been skipped at that instant, the buffer simply stopped
    // accepting. Stop buffering and let the bus report the loss, with the
    // count it actually measures, when delivery reaches it.
    const MAX_PENDING: usize = 8_192;
    loop {
        if pending.len() >= MAX_PENDING {
            return;
        }
        match receiver.try_recv() {
            Ok(envelope) => pending.push_back(envelope),
            Err(broadcast::error::TryRecvError::Empty)
            | Err(broadcast::error::TryRecvError::Closed) => return,
            Err(broadcast::error::TryRecvError::Lagged(skipped)) => {
                *lagged = Some(skipped);
                return;
            }
        }
    }
}

/// Whether a live frame was already delivered by the replay that preceded it.
///
/// The bus is subscribed before the tape is read, so a record published in
/// between appears on both paths; without this the consumer sees it twice.
/// Advances the high-water mark as a side effect.
fn already_delivered(offset: Option<u64>, last_sent: &mut Option<u64>) -> bool {
    let Some(offset) = offset else {
        // A frame with no offset predates the tape, so it cannot have been
        // replayed and must not move the mark.
        return false;
    };
    if last_sent.is_some_and(|last| offset <= last) {
        return true;
    }
    *last_sent = Some(offset);
    false
}

/// Deliver one live frame, skipping anything the replay already sent.
///
/// Returns false when the subscription should end.
async fn forward_live_frame(
    context: &SubscriptionContext,
    subscription_id: &str,
    view_id: &str,
    query: &SubscriptionQuery,
    envelope: &Arc<BusMessage>,
    last_sent: &mut Option<u64>,
) -> bool {
    let metadata = source_frame_metadata(&envelope.payload);
    if already_delivered(metadata.offset, last_sent) {
        return true;
    }
    if !live_frame_matches(query, &envelope.key, &envelope.payload) {
        return true;
    }
    send_scoped_source_payload(context, subscription_id, view_id, envelope.payload.clone()).is_ok()
}

fn report_replay_gap(
    context: &SubscriptionContext,
    subscription_id: &str,
    view_id: &str,
    epoch: &crate::journal::JournalEpoch,
    skipped: u64,
    last_sent: Option<u64>,
) {
    warn!(
        "Replay subscription {} lagged past {} records; stopping with a recovery cursor",
        subscription_id, skipped
    );
    let recover_from = last_sent.map(|offset| crate::journal::Cursor {
        epoch: epoch.clone(),
        offset,
    });
    let _ = send_control_frame(
        context,
        &SocketIssueMessage::replay_lagged(
            Some(subscription_id.to_string()),
            skipped,
            recover_from,
        ),
        view_id,
    );
}

/// Apply the subscription's `key`, `partition` and `filters` to a retained
/// record. A replay must honour the same predicates a live subscription does.
fn journal_record_matches(
    query: &SubscriptionQuery,
    record: &crate::journal::JournalRecord,
) -> bool {
    live_frame_matches(query, &record.key, &record.payload)
}

fn live_frame_matches(query: &SubscriptionQuery, key: &str, payload: &[u8]) -> bool {
    if !query.matches_key(key) {
        return false;
    }
    if query.partition.is_none() && query.filters.is_empty() {
        return true;
    }
    let Some(data) = source_frame_data(payload) else {
        return false;
    };
    let data = &data;
    if let Some(partition) = &query.partition {
        if value_at_dot_path(data, "_partition").as_deref()
            != Some(&Value::String(partition.clone()))
        {
            return false;
        }
    }
    query
        .filters
        .iter()
        .all(|(path, expected)| value_at_dot_path(data, path).as_deref() == Some(expected))
}

/// Awaiting variant of [`send_scoped_source_payload`], for replays that can
/// exceed the client's send queue.
async fn send_scoped_source_payload_async(
    context: &SubscriptionContext,
    subscription_id: &str,
    view_id: &str,
    payload: Arc<Bytes>,
) -> Result<()> {
    let json = SourceFields::scoped(&payload, subscription_id)?;
    let compressed = maybe_compress(&json);
    let bytes = compressed.as_bytes().len();
    context
        .client_manager
        .send_compressed_async(context.client_id, compressed)
        .await
        .map_err(|error| anyhow::anyhow!("failed to send replayed frame: {error}"))?;
    context.metrics.message_sent();
    emit_update_sent_for_client(
        &context.usage_emitter,
        &context.client_manager,
        context.client_id,
        view_id,
        bytes,
    );
    Ok(())
}

#[derive(Default)]
struct SourceFrameMetadata {
    op: String,
    seq: Option<String>,
    offset: Option<u64>,
}

/// A source frame's `op`, `seq` and `offset`, read without building its
/// `data`.
fn source_frame_metadata(payload: &[u8]) -> SourceFrameMetadata {
    SourceFields::parse(payload)
        .map(|fields| SourceFrameMetadata {
            op: fields
                .value("op")
                .as_ref()
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            seq: fields
                .value("seq")
                .as_ref()
                .and_then(Value::as_str)
                .map(str::to_string),
            offset: fields.value("offset").as_ref().and_then(Value::as_u64),
        })
        .unwrap_or_default()
}

/// The `data` of a source frame, as a client receives it.
fn source_frame_data(payload: &[u8]) -> Option<Value> {
    SourceFields::parse(payload)?.value("data")
}

/// What a state subscriber last received whole, which a catch-up is
/// measured against. Both kinds are shared, with the entity cache or the bus,
/// so a subscriber keeps no copy of its own. A subscriber does keep the
/// version it was sent alive while the cache moves on; subscribers sent the
/// same version share it.
enum Synced {
    /// The cached entity, as sent in a snapshot row, an upsert or a catch-up.
    Entity(SharedEntity),
    /// A forwarded `upsert` source frame, whose `data` the client holds.
    Frame(Arc<Bytes>),
}

impl Synced {
    /// What the client holds, as it received it: in wire format.
    fn wire(&self, wire_format: &WireFormat) -> Option<Value> {
        match self {
            Synced::Entity(entity) => Some(wire_value(entity, wire_format)),
            Synced::Frame(frame) => source_frame_data(frame),
        }
    }
}

/// Tracks what a client holds after a frame forwarded to it as published. An
/// upsert hands it the entity whole, which a catch-up is then measured
/// against; a patch sets fields (see [`mark_fields`]).
fn record_forwarded(
    payload: &Arc<Bytes>,
    op: &str,
    synced: &mut Option<Synced>,
    touched: &mut Value,
) {
    if op == "upsert" {
        *synced = Some(Synced::Frame(payload.clone()));
        *touched = Value::Null;
    } else if let Some(data) = source_frame_data(payload) {
        mark_fields(touched, &data);
    }
}

/// Marks in `touched` the fields `patch` sets: objects field by field, any
/// other value whole (`true`). `null` marks nothing.
///
/// A client holds whatever a forwarded patch set, which the entity may have
/// set back before the next catch-up (or the client may have dropped the
/// patch), so a catch-up sends these fields even when they match the entity
/// the client last received whole.
fn mark_fields(touched: &mut Value, patch: &Value) {
    let Value::Object(fields) = patch else {
        *touched = Value::Bool(true);
        return;
    };
    if touched.is_null() {
        *touched = Value::Object(serde_json::Map::new());
    }
    let Value::Object(marked) = touched else {
        // Already marked whole.
        return;
    };
    for (field, value) in fields {
        mark_fields(marked.entry(field.clone()).or_insert(Value::Null), value);
    }
}

/// The fields of `current` that differ from `base` or that `touched` marks
/// (see [`mark_fields`]), as a patch that turns what the client holds into
/// `current` when merged: objects compare field by field, any other value is
/// sent whole, and a field `current` no longer has is sent as `null`. `None`
/// when nothing needs sending.
fn changed_fields(base: &Value, current: &Value, touched: &Value) -> Option<Value> {
    match (base, current, touched) {
        (_, _, Value::Bool(true)) => Some(current.clone()),
        (Value::Object(base), Value::Object(current), _) => {
            let marked = touched.as_object();
            let mut patch = serde_json::Map::new();
            for (field, value) in current {
                let touched = marked
                    .and_then(|marked| marked.get(field))
                    .unwrap_or(&Value::Null);
                match base.get(field) {
                    Some(old) => {
                        if let Some(changed) = changed_fields(old, value, touched) {
                            patch.insert(field.clone(), changed);
                        }
                    }
                    None => {
                        patch.insert(field.clone(), value.clone());
                    }
                }
            }
            for field in base
                .keys()
                .chain(marked.into_iter().flat_map(|marked| marked.keys()))
            {
                if !current.contains_key(field) {
                    patch.insert(field.clone(), Value::Null);
                }
            }
            (!patch.is_empty()).then_some(Value::Object(patch))
        }
        _ => (base != current || !touched.is_null()).then(|| current.clone()),
    }
}

fn send_scoped_source_payload(
    context: &SubscriptionContext,
    subscription_id: &str,
    view_id: &str,
    payload: Arc<Bytes>,
) -> Result<()> {
    let encoded = Arc::new(Bytes::from(SourceFields::scoped(
        &payload,
        subscription_id,
    )?));
    let bytes = encoded.len();
    context
        .client_manager
        .send_to_client(context.client_id, encoded)
        .map_err(|error| anyhow::anyhow!("failed to send live frame: {error}"))?;
    context.metrics.message_sent();
    emit_update_sent_for_client(
        &context.usage_emitter,
        &context.client_manager,
        context.client_id,
        view_id,
        bytes,
    );
    Ok(())
}

fn send_membership_frame(
    context: &SubscriptionContext,
    subscription_id: &str,
    view_spec: &ViewSpec,
    op: &str,
    key: &str,
    mut data: Value,
    seq: Option<String>,
) -> Result<()> {
    apply_wire_format(&mut data, &view_spec.wire_format);
    let frame = Frame::scoped(
        subscription_id,
        view_spec.mode,
        &view_spec.id,
        op,
        key,
        data,
        seq,
    );
    send_membership_json(context, view_spec, serde_json::to_vec(&frame)?)
}

/// A scoped frame whose data is a cached entity, serialized straight from
/// it: the same JSON as [`Frame::scoped`] with a formatted copy of the entity.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ScopedEntityFrame<'a> {
    protocol_version: u8,
    subscription_id: &'a str,
    mode: Mode,
    #[serde(rename = "entity")]
    export: &'a str,
    op: &'a str,
    key: &'a str,
    data: WireEntity<'a>,
    #[serde(skip_serializing_if = "Option::is_none")]
    seq: Option<&'a str>,
}

/// [`send_membership_frame`] for a cached entity, without copying it.
fn send_entity_frame(
    context: &SubscriptionContext,
    subscription_id: &str,
    view_spec: &ViewSpec,
    op: &str,
    key: &str,
    entity: &SharedEntity,
    seq: Option<&str>,
) -> Result<()> {
    let frame = ScopedEntityFrame {
        protocol_version: PROTOCOL_VERSION,
        subscription_id,
        mode: view_spec.mode,
        export: &view_spec.id,
        op,
        key,
        data: WireEntity {
            entity,
            wire_format: &view_spec.wire_format,
        },
        seq,
    };
    send_membership_json(context, view_spec, serde_json::to_vec(&frame)?)
}

fn send_membership_json(
    context: &SubscriptionContext,
    view_spec: &ViewSpec,
    json: Vec<u8>,
) -> Result<()> {
    let encoded = Arc::new(Bytes::from(json));
    let bytes = encoded.len();
    context
        .client_manager
        .send_to_client(context.client_id, encoded)
        .map_err(|error| anyhow::anyhow!("failed to send membership frame: {error}"))?;
    context.metrics.message_sent();
    emit_update_sent_for_client(
        &context.usage_emitter,
        &context.client_manager,
        context.client_id,
        &view_spec.id,
        bytes,
    );
    Ok(())
}

/// Send what one source mutation owes a subscriber, and keep `undelivered`
/// (window members the client holds no copy of) in step with what was sent.
#[allow(clippy::too_many_arguments)]
fn emit_collection_delta(
    context: &SubscriptionContext,
    subscription_id: &str,
    view_spec: &ViewSpec,
    current: &[(String, SharedEntity)],
    next: &[(String, SharedEntity)],
    envelope: &BusMessage,
    metadata: &SourceFrameMetadata,
    undelivered: &mut HashSet<String>,
) -> Result<()> {
    let current_keys: Vec<&str> = current.iter().map(|(key, _)| key.as_str()).collect();
    let next_keys: Vec<&str> = next.iter().map(|(key, _)| key.as_str()).collect();
    let next_set: HashSet<&str> = next_keys.iter().copied().collect();

    for key in current_keys
        .iter()
        .copied()
        .filter(|key| !next_set.contains(key))
    {
        // A key the client was never sent has nothing to remove.
        if undelivered.remove(key) {
            continue;
        }
        let op = if metadata.op == "delete" && key == envelope.key {
            "delete"
        } else {
            "remove"
        };
        send_membership_frame(
            context,
            subscription_id,
            view_spec,
            op,
            key,
            Value::Null,
            metadata.seq.clone(),
        )?;
    }

    for (key, data) in next.iter() {
        let in_window = current_keys.iter().any(|candidate| *candidate == key);
        match member_action(
            in_window,
            in_window && !undelivered.contains(key),
            key == &envelope.key,
            view_spec.is_derived(),
            &metadata.op,
        ) {
            MemberAction::Skip => {}
            MemberAction::ForwardPatch => send_scoped_source_payload(
                context,
                subscription_id,
                &view_spec.id,
                envelope.payload.clone(),
            )?,
            MemberAction::Upsert => {
                let seq = metadata
                    .seq
                    .as_deref()
                    .or_else(|| data.field("_seq").and_then(Value::as_str));
                send_entity_frame(
                    context,
                    subscription_id,
                    view_spec,
                    "upsert",
                    key,
                    data,
                    seq,
                )?;
                undelivered.remove(key);
            }
        }
    }
    Ok(())
}

/// Emit the net effect of every source mutation seen during one coalescing
/// interval. Full entities are used for changed members because intermediate
/// sparse patches were intentionally discarded and can no longer be merged
/// safely by a client.
fn emit_coalesced_collection_delta(
    context: &SubscriptionContext,
    subscription_id: &str,
    view_spec: &ViewSpec,
    current: &[(String, SharedEntity)],
    next: &[(String, SharedEntity)],
    pending: &HashMap<String, Arc<BusMessage>>,
    undelivered: &mut HashSet<String>,
) -> Result<()> {
    let changes = plan_coalesced_collection_delta(current, next, pending, undelivered);
    // Every key that left the window or was just upserted is settled: the
    // client either never needs it or now holds all of it.
    let next_keys: HashSet<&str> = next.iter().map(|(key, _)| key.as_str()).collect();
    undelivered.retain(|key| next_keys.contains(key.as_str()));
    for change in changes {
        if change.op == "upsert" {
            undelivered.remove(&change.key);
        }
        match &change.data {
            Some(entity) => send_entity_frame(
                context,
                subscription_id,
                view_spec,
                change.op,
                &change.key,
                entity,
                change.seq.as_deref(),
            )?,
            None => send_membership_frame(
                context,
                subscription_id,
                view_spec,
                change.op,
                &change.key,
                Value::Null,
                change.seq,
            )?,
        }
    }
    Ok(())
}

#[derive(Debug, PartialEq)]
struct CollectionChange {
    op: &'static str,
    key: String,
    /// The entity an `upsert` sends; `None` for a `remove` or `delete`.
    data: Option<SharedEntity>,
    seq: Option<String>,
}

/// Plan one coalescing interval. `undelivered` holds window members the
/// client was never sent; they get no `remove`/`delete` when they leave, and
/// like every changed key they are sent whole when they change.
fn plan_coalesced_collection_delta(
    current: &[(String, SharedEntity)],
    next: &[(String, SharedEntity)],
    pending: &HashMap<String, Arc<BusMessage>>,
    undelivered: &HashSet<String>,
) -> Vec<CollectionChange> {
    let current_by_key: HashMap<&str, &SharedEntity> = current
        .iter()
        .map(|(key, data)| (key.as_str(), data))
        .collect();
    let next_keys: HashSet<&str> = next.iter().map(|(key, _)| key.as_str()).collect();
    let latest_seq = pending
        .values()
        .filter_map(|envelope| source_frame_metadata(&envelope.payload).seq)
        .max_by(|left, right| cmp_seq(left, right));
    let mut changes = Vec::new();

    for (key, _) in current
        .iter()
        .filter(|(key, _)| !next_keys.contains(key.as_str()) && !undelivered.contains(key.as_str()))
    {
        let metadata = pending
            .get(key)
            .map(|envelope| source_frame_metadata(&envelope.payload));
        let op = if metadata
            .as_ref()
            .is_some_and(|metadata| metadata.op == "delete")
        {
            "delete"
        } else {
            "remove"
        };
        let seq = metadata
            .and_then(|metadata| metadata.seq)
            .or_else(|| latest_seq.clone());
        changes.push(CollectionChange {
            op,
            key: key.clone(),
            data: None,
            seq,
        });
    }

    for (key, data) in next {
        let changed = current_by_key
            .get(key.as_str())
            .is_none_or(|previous| *previous != data);
        if !changed && !pending.contains_key(key) {
            continue;
        }
        let seq = data
            .field("_seq")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| {
                pending
                    .get(key)
                    .and_then(|envelope| source_frame_metadata(&envelope.payload).seq)
            })
            .or_else(|| latest_seq.clone());
        changes.push(CollectionChange {
            op: "upsert",
            key: key.clone(),
            data: Some(data.clone()),
            seq,
        });
    }
    changes
}

/// What one in-window key owes a subscriber after a source mutation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MemberAction {
    /// Send nothing: the subscriber already holds this entity and its data did
    /// not change.
    Skip,
    /// Forward the source patch verbatim.
    ForwardPatch,
    /// Send the whole entity.
    Upsert,
}

/// Decide what to send for one key in the query window.
///
/// `in_window` says the key was in the window before this mutation; `held`
/// says the client also has a copy of it. The two differ after a snapshot
/// that `snapshotLimit` truncated or that was disabled: those keys count for
/// the window's positions, but the client was never sent them.
///
/// Only the mutated key's data changed, so a key that was already in the
/// window has at most moved index — and index is not the server's to
/// communicate. `subscribed` announces the window's sort (see
/// [`extract_sort_config`], which is `_seq` descending for a plain list) and
/// every SDK re-sorts locally from it, so resending an unchanged entity to
/// convey its new position is pure waste. This matters because on a
/// `_seq`-ordered list the mutated entity jumps to the front on *every*
/// mutation: keying the decision on position change meant rebroadcasting the
/// whole window each time, and the mutated entity itself never kept its
/// position long enough to have its patch forwarded.
fn member_action(
    in_window: bool,
    held: bool,
    is_mutated_key: bool,
    is_derived: bool,
    op: &str,
) -> MemberAction {
    if !is_mutated_key {
        // A key entering the window has no local state to merge into, so it
        // needs the whole entity. One already in the window is unchanged:
        // if the client holds it there is nothing new, and if it doesn't the
        // snapshot limit deliberately left it out, so it waits for its own
        // next change.
        return if in_window {
            MemberAction::Skip
        } else {
            MemberAction::Upsert
        };
    }
    // The mutated entity rides its own patch through untouched, but only when
    // the subscriber already holds a copy; a patch for a key the client never
    // received is not an entity. Derived views still send whole entities: the
    // patch on the bus is scoped to the source view, not this one (see
    // A4-150).
    if held && !is_derived && op != "delete" {
        MemberAction::ForwardPatch
    } else {
        MemberAction::Upsert
    }
}

/// A copy of `entity` with the view's wire format applied, for code that
/// needs it as a value (a catch-up diff) rather than serialized.
fn wire_value(entity: &SharedEntity, wire_format: &WireFormat) -> Value {
    let mut value = entity.to_value();
    apply_wire_format(&mut value, wire_format);
    value
}

async fn load_query_entities(
    entity_cache: &EntityCache,
    sorted_caches: Option<
        Arc<tokio::sync::RwLock<HashMap<String, crate::sorted_cache::SortedViewCache>>>,
    >,
    view_spec: &ViewSpec,
    query: &SubscriptionQuery,
    apply_snapshot_limit: bool,
) -> Vec<(String, SharedEntity)> {
    // Rows share their fields with the caches; nothing here copies an entity.
    let ordered = if let Some(sorted_caches) = sorted_caches {
        let mut caches = sorted_caches.write().await;
        caches
            .get_mut(&view_spec.id)
            .map(|cache| cache.ordered_entities())
    } else {
        None
    };
    let (entities, preordered) = if let Some(entities) = ordered {
        (entities, true)
    } else if view_spec.is_derived() {
        // Empty and filter-only pipelines have no sorted cache. Evaluate the
        // source rows, retaining the pipeline predicate before query selection.
        let mut entities = entity_cache
            .get_all_shared(view_spec.source_view.as_deref().unwrap_or(&view_spec.id))
            .await;
        if let Some(filter) = view_spec
            .pipeline
            .as_ref()
            .and_then(|pipeline| pipeline.filter.as_ref())
        {
            entities.retain(|(_, data)| filter.matches(data));
        }
        (entities, false)
    } else if view_spec.mode == Mode::State {
        let entity = match query.key.as_deref() {
            Some(key) => entity_cache
                .get_shared(&view_spec.id, key)
                .await
                .map(|data| vec![(key.to_string(), data)])
                .unwrap_or_default(),
            None => vec![],
        };
        (entity, true)
    } else {
        (entity_cache.get_all_shared(&view_spec.id).await, false)
    };
    let mut query = query.clone();
    if let Some(limit) = view_spec
        .pipeline
        .as_ref()
        .and_then(|pipeline| pipeline.limit)
    {
        query.take = Some(query.take.unwrap_or(limit).min(limit));
    }
    select_query_entities(entities, &query, preordered, apply_snapshot_limit)
}

fn select_query_entities<E: EntityFields>(
    mut entities: Vec<(String, E)>,
    query: &SubscriptionQuery,
    preordered: bool,
    apply_snapshot_limit: bool,
) -> Vec<(String, E)> {
    entities.retain(|(key, data)| query_matches_entity(query, key, data));
    if !preordered {
        entities.sort_by(|left, right| {
            let left_seq = left.1.field("_seq").and_then(Value::as_str).unwrap_or("");
            let right_seq = right.1.field("_seq").and_then(Value::as_str).unwrap_or("");
            let order = if query.after.is_some() {
                cmp_seq(left_seq, right_seq)
            } else {
                cmp_seq(right_seq, left_seq)
            };
            order.then_with(|| left.0.cmp(&right.0))
        });
    }

    let skip = query.skip.unwrap_or(0);
    let take = query.take.unwrap_or(usize::MAX);
    let mut selected: Vec<_> = entities.into_iter().skip(skip).take(take).collect();
    if apply_snapshot_limit {
        if let Some(limit) = query.snapshot_limit {
            selected.truncate(limit);
        }
    }
    selected
}

fn query_matches_entity<E: EntityFields + ?Sized>(
    query: &SubscriptionQuery,
    key: &str,
    data: &E,
) -> bool {
    if !query.matches_key(key) {
        return false;
    }
    if let Some(partition) = &query.partition {
        if value_at_dot_path(data, "_partition").as_deref()
            != Some(&Value::String(partition.clone()))
        {
            return false;
        }
    }
    if let Some(after) = &query.after {
        let Some(seq) = data.field("_seq").and_then(Value::as_str) else {
            return false;
        };
        if cmp_seq(seq, after) != std::cmp::Ordering::Greater {
            return false;
        }
    }
    query
        .filters
        .iter()
        .all(|(path, expected)| value_at_dot_path(data, path).as_deref() == Some(expected))
}

fn value_at_dot_path<'a, E: EntityFields + ?Sized>(
    entity: &'a E,
    path: &str,
) -> Option<std::borrow::Cow<'a, Value>> {
    lookup(entity, path.split('.'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::{EntityCacheConfig, PatchOrigin};
    use crate::view::{Delivery, Filters, Projection};
    use serde_json::json;
    use tokio::sync::oneshot;

    #[derive(Default)]
    struct DelayedUsageEmitter {
        started: tokio::sync::Notify,
        release: tokio::sync::Notify,
        events: tokio::sync::Mutex<Vec<WebSocketUsageEvent>>,
    }

    #[async_trait::async_trait]
    impl WebSocketUsageEmitter for DelayedUsageEmitter {
        async fn emit(&self, event: WebSocketUsageEvent) {
            self.started.notify_one();
            self.release.notified().await;
            self.events.lock().await.push(event);
        }
    }

    #[tokio::test]
    async fn usage_emitter_handle_waits_for_spawned_emissions() {
        let emitter = Arc::new(DelayedUsageEmitter::default());
        let handle = UsageEmitterHandle::new(emitter.clone());
        handle.emit(WebSocketUsageEvent::ConnectionEstablished {
            client_id: "client-1".to_string(),
            remote_addr: "127.0.0.1:1234".to_string(),
            deployment_id: Some("1".to_string()),
            identity: UsageIdentity::default(),
        });
        emitter.started.notified().await;

        let closing_handle = handle.clone();
        let closing = tokio::spawn(async move {
            closing_handle.close_and_wait().await;
        });
        tokio::task::yield_now().await;
        assert!(!closing.is_finished());

        emitter.release.notify_one();
        closing.await.expect("usage task wait should finish");
        assert_eq!(emitter.events.lock().await.len(), 1);
    }

    #[test]
    fn a_catch_up_carries_only_the_fields_that_changed() {
        let base = json!({
            "id": 7,
            "state": {"deployed": [1, 2, 3], "total": 6},
            "results": {"square": null},
            "gone": true,
        });
        let current = json!({
            "id": 7,
            "state": {"deployed": [1, 2, 4], "total": 6},
            "results": {"square": 3},
            "fresh": "x",
        });
        assert_eq!(
            changed_fields(&base, &current, &Value::Null),
            Some(json!({
                // An array that changed is sent whole.
                "state": {"deployed": [1, 2, 4]},
                "results": {"square": 3},
                "fresh": "x",
                // A field the entity no longer has is cleared.
                "gone": null,
            }))
        );
        assert_eq!(changed_fields(&current, &current, &Value::Null), None);
        // Anything that is not an object on both sides is compared whole.
        assert_eq!(
            changed_fields(&json!(null), &current, &Value::Null),
            Some(current.clone())
        );
    }

    #[test]
    fn a_catch_up_resends_the_fields_forwarded_patches_set() {
        let base = json!({"a": 1, "state": {"x": 1, "y": 1}, "list": [1]});
        let mut touched = Value::Null;
        mark_fields(&mut touched, &json!({"a": 2, "state": {"x": 2}}));
        mark_fields(&mut touched, &json!({"list": [2], "extra": true}));
        assert_eq!(
            touched,
            json!({"a": true, "state": {"x": true}, "list": true, "extra": true})
        );
        // The entity set `a` and `state.x` back and never kept `extra`: the
        // client still holds what the forwarded patches set, so a catch-up
        // compared with `base` alone would leave it stale.
        let current = json!({"a": 1, "state": {"x": 1, "y": 1}, "list": [1, 2]});
        assert_eq!(
            changed_fields(&base, &current, &Value::Null),
            Some(json!({"list": [1, 2]}))
        );
        assert_eq!(
            changed_fields(&base, &current, &touched),
            Some(json!({"a": 1, "state": {"x": 1}, "list": [1, 2], "extra": null}))
        );
        // A patch whose data is not an object marks everything.
        mark_fields(&mut touched, &json!(null));
        assert_eq!(
            changed_fields(&base, &current, &touched),
            Some(current.clone())
        );
    }

    fn list_spec() -> ViewSpec {
        ViewSpec {
            id: "Thing/list".to_string(),
            export: "Thing".to_string(),
            mode: Mode::List,
            wire_format: Default::default(),
            projection: Projection::all(),
            filters: Filters::all(),
            delivery: Delivery::default(),
            pipeline: None,
            source_view: None,
        }
    }

    /// Plan the whole window the way `emit_collection_delta` does, so a test
    /// can assert on what a subscriber is actually sent.
    fn plan_window(
        current_keys: &[&str],
        next_keys: &[&str],
        envelope_key: &str,
        is_derived: bool,
        op: &str,
    ) -> Vec<(String, MemberAction)> {
        plan_window_with_undelivered(current_keys, &[], next_keys, envelope_key, is_derived, op)
    }

    /// [`plan_window`] for a subscriber that was never sent `undelivered`.
    fn plan_window_with_undelivered(
        current_keys: &[&str],
        undelivered: &[&str],
        next_keys: &[&str],
        envelope_key: &str,
        is_derived: bool,
        op: &str,
    ) -> Vec<(String, MemberAction)> {
        next_keys
            .iter()
            .map(|key| {
                let in_window = current_keys.contains(key);
                let held = in_window && !undelivered.contains(key);
                (
                    (*key).to_string(),
                    member_action(in_window, held, *key == envelope_key, is_derived, op),
                )
            })
            .collect()
    }

    #[test]
    fn a_window_member_the_client_never_received_is_sent_whole() {
        // snapshotLimit cut the window [4, 3, 2, 1] to [4, 3]. "1" still
        // counts for positions, but a patch for it would reach a client with
        // nothing to merge into.
        let plan = plan_window_with_undelivered(
            &["4", "3", "2", "1"],
            &["2", "1"],
            &["1", "4", "3", "2"],
            "1",
            false,
            "patch",
        );
        assert_eq!(
            plan,
            vec![
                ("1".to_string(), MemberAction::Upsert),
                ("4".to_string(), MemberAction::Skip),
                ("3".to_string(), MemberAction::Skip),
                // Unchanged and never sent: it waits for its own change
                // rather than undoing the snapshot limit.
                ("2".to_string(), MemberAction::Skip),
            ]
        );
    }

    #[test]
    fn a_held_member_still_rides_its_patch_after_a_truncated_snapshot() {
        let plan = plan_window_with_undelivered(
            &["4", "3", "2", "1"],
            &["2", "1"],
            &["4", "3", "2", "1"],
            "4",
            false,
            "patch",
        );
        assert_eq!(plan[0], ("4".to_string(), MemberAction::ForwardPatch));
    }

    #[test]
    fn coalescing_sends_no_remove_for_a_key_the_client_never_received() {
        let current = vec![
            ("held".to_string(), json!({"count": 1, "_seq": "10:000001"})),
            (
                "unsent".to_string(),
                json!({"count": 1, "_seq": "10:000001"}),
            ),
            ("gone".to_string(), json!({"count": 1, "_seq": "10:000001"})),
        ];
        let next = vec![
            ("held".to_string(), json!({"count": 1, "_seq": "10:000001"})),
            (
                "unsent".to_string(),
                json!({"count": 2, "_seq": "10:000003"}),
            ),
        ];
        let pending = HashMap::from([
            (
                "unsent".to_string(),
                list_message("unsent", "patch", "10:000003"),
            ),
            (
                "gone".to_string(),
                list_message("gone", "delete", "10:000002"),
            ),
        ]);
        let undelivered = HashSet::from(["unsent".to_string(), "gone".to_string()]);

        assert_eq!(
            plan_coalesced_collection_delta(
                &shared(&current),
                &shared(&next),
                &pending,
                &undelivered
            ),
            vec![CollectionChange {
                op: "upsert",
                key: "unsent".to_string(),
                data: Some(SharedEntity::new(json!({"count": 2, "_seq": "10:000003"}))),
                seq: Some("10:000003".to_string()),
            }]
        );
    }

    #[test]
    fn a_reordered_window_sends_only_the_mutated_entity() {
        // A `_seq`-descending list: mutating "1" moves it to the front and
        // shifts every other key down one. Only "1" changed, so only "1" is
        // sent — and it rides its own patch, not a full entity.
        let current = ["4", "3", "2", "1"];
        let next = ["1", "4", "3", "2"];
        let plan = plan_window(&current, &next, "1", false, "patch");

        assert_eq!(
            plan,
            vec![
                ("1".to_string(), MemberAction::ForwardPatch),
                ("4".to_string(), MemberAction::Skip),
                ("3".to_string(), MemberAction::Skip),
                ("2".to_string(), MemberAction::Skip),
            ]
        );
    }

    #[test]
    fn a_key_entering_the_window_gets_the_whole_entity() {
        // "5" has no local state for the subscriber to merge a patch into.
        let plan = plan_window(&["4", "3"], &["5", "4", "3"], "5", false, "patch");
        assert_eq!(
            plan,
            vec![
                ("5".to_string(), MemberAction::Upsert),
                ("4".to_string(), MemberAction::Skip),
                ("3".to_string(), MemberAction::Skip),
            ]
        );
    }

    #[test]
    fn derived_views_still_send_whole_entities() {
        // The patch on the bus is scoped to the source view, so a derived
        // subscription cannot forward it verbatim (A4-150).
        let plan = plan_window(&["1", "2"], &["1", "2"], "1", true, "patch");
        assert_eq!(
            plan,
            vec![
                ("1".to_string(), MemberAction::Upsert),
                ("2".to_string(), MemberAction::Skip),
            ]
        );
    }

    #[test]
    fn a_delete_envelope_never_forwards_a_patch() {
        // A surviving key on a delete envelope carries no mergeable patch.
        let plan = plan_window(&["1", "2"], &["1", "2"], "1", false, "delete");
        assert_eq!(
            plan,
            vec![
                ("1".to_string(), MemberAction::Upsert),
                ("2".to_string(), MemberAction::Skip),
            ]
        );
    }

    #[test]
    fn an_unchanged_window_sends_one_frame_not_a_broadcast() {
        // The regression this guards: 500 members used to mean 500 full
        // entities on the wire for a single mutation.
        let keys: Vec<String> = (0..500).map(|index| index.to_string()).collect();
        let refs: Vec<&str> = keys.iter().map(String::as_str).collect();
        let plan = plan_window(&refs, &refs, "250", false, "patch");

        let sent = plan
            .iter()
            .filter(|(_, action)| *action != MemberAction::Skip)
            .count();
        assert_eq!(sent, 1);
        assert_eq!(plan[250].1, MemberAction::ForwardPatch);
    }

    fn shared(rows: &[(String, Value)]) -> Vec<(String, SharedEntity)> {
        rows.iter()
            .map(|(key, data)| (key.clone(), SharedEntity::new(data.clone())))
            .collect()
    }

    fn list_message(key: &str, op: &str, seq: &str) -> Arc<BusMessage> {
        Arc::new(BusMessage {
            key: key.to_string(),
            entity: "Thing/list".to_string(),
            payload: Arc::new(Bytes::from(
                serde_json::to_vec(&json!({
                    "entity": "Thing/list",
                    "op": op,
                    "key": key,
                    "seq": seq,
                    "data": {},
                }))
                .unwrap(),
            )),
        })
    }

    #[test]
    fn coalescing_emits_only_the_final_full_state_per_changed_key() {
        let current = vec![
            ("a".to_string(), json!({"count": 1, "_seq": "10:000001"})),
            ("b".to_string(), json!({"count": 1, "_seq": "10:000001"})),
            (
                "stable".to_string(),
                json!({"count": 1, "_seq": "10:000001"}),
            ),
        ];
        let next = vec![
            ("a".to_string(), json!({"count": 3, "_seq": "10:000004"})),
            ("c".to_string(), json!({"count": 1, "_seq": "10:000003"})),
            (
                "stable".to_string(),
                json!({"count": 1, "_seq": "10:000001"}),
            ),
        ];
        let pending = HashMap::from([
            ("a".to_string(), list_message("a", "patch", "10:000004")),
            ("b".to_string(), list_message("b", "delete", "10:000002")),
            ("c".to_string(), list_message("c", "patch", "10:000003")),
        ]);

        assert_eq!(
            plan_coalesced_collection_delta(
                &shared(&current),
                &shared(&next),
                &pending,
                &HashSet::new()
            ),
            vec![
                CollectionChange {
                    op: "delete",
                    key: "b".to_string(),
                    data: None,
                    seq: Some("10:000002".to_string()),
                },
                CollectionChange {
                    op: "upsert",
                    key: "a".to_string(),
                    data: Some(SharedEntity::new(json!({"count": 3, "_seq": "10:000004"}))),
                    seq: Some("10:000004".to_string()),
                },
                CollectionChange {
                    op: "upsert",
                    key: "c".to_string(),
                    data: Some(SharedEntity::new(json!({"count": 1, "_seq": "10:000003"}))),
                    seq: Some("10:000003".to_string()),
                },
            ]
        );
    }

    #[test]
    fn a_lagged_snapshotless_subscription_gets_a_fatal_retryable_error() {
        let issue = SocketIssueMessage::subscription_lagged("balances".to_string(), 42);
        assert_eq!(issue.code, "subscription-lagged");
        assert!(issue.retryable);
        assert!(issue.fatal);
        assert!(issue.message.contains("42"));
        assert!(issue
            .suggested_action
            .unwrap()
            .contains("snapshots enabled"));

        let append = SocketIssueMessage::append_subscription_lagged("trades".to_string(), 9);
        assert!(append.fatal);
        assert!(append.suggested_action.unwrap().contains("retained replay"));
    }

    #[test]
    fn snapshot_batches_share_identity_and_completion() {
        let entities = ["one", "two", "three"]
            .map(|key| (key.to_string(), SharedEntity::new(json!({"key": key}))));
        let wire_format = WireFormat::default();
        let batches = create_snapshot_batches(
            &entities,
            &wire_format,
            SnapshotMetadata {
                subscription_id: "sub-1",
                snapshot_id: "snapshot-1",
                authoritative: true,
                mode: Mode::List,
                view_id: "Thing/list",
                key: None,
            },
            &SnapshotBatchConfig {
                initial_batch_size: 2,
                subsequent_batch_size: 1,
            },
        );
        assert_eq!(batches.len(), 2);
        assert!(batches.iter().all(|batch| batch.subscription_id == "sub-1"));
        assert!(batches
            .iter()
            .all(|batch| batch.snapshot_id == "snapshot-1"));
        assert!(!batches[0].complete);
        assert!(batches[1].complete);
        assert!(batches.iter().all(|batch| batch.authoritative));
    }

    #[test]
    fn empty_incremental_snapshot_is_explicitly_non_authoritative() {
        let wire_format = WireFormat::default();
        let batches = create_snapshot_batches(
            &[],
            &wire_format,
            SnapshotMetadata {
                subscription_id: "sub-1",
                snapshot_id: "snapshot-1",
                authoritative: false,
                mode: Mode::State,
                view_id: "Thing/state",
                key: Some("missing"),
            },
            &SnapshotBatchConfig {
                initial_batch_size: 1,
                subsequent_batch_size: 1,
            },
        );
        assert_eq!(batches.len(), 1);
        assert!(!batches[0].authoritative);
        assert!(batches[0].complete);
        assert_eq!(batches[0].key, Some("missing"));
    }

    fn wide_ints() -> WireFormat {
        WireFormat {
            wide_int_paths: vec![
                vec!["amount".to_string()],
                vec!["trades".to_string(), "price".to_string()],
            ],
        }
    }

    fn sample_rows() -> Vec<(String, SharedEntity)> {
        (0..7)
            .map(|index| {
                (
                    index.to_string(),
                    SharedEntity::new(json!({
                        "_seq": format!("10:{index:012}"),
                        "_version": format!("e:{index}"),
                        "amount": u64::MAX - index,
                        "label": "π \"quoted\" \n",
                        "trades": [{"price": index, "side": "buy"}, {"price": -1}],
                    })),
                )
            })
            .collect()
    }

    /// Snapshot frames serialized from shared rows are byte for byte the
    /// frames built from formatted copies of the rows.
    #[test]
    fn snapshot_batches_serialize_as_frames_of_formatted_copies() {
        use crate::websocket::frame::{SnapshotEntity, SnapshotFrame};
        let rows = sample_rows();
        for (rows, key, wire_format, (initial, subsequent)) in [
            (&rows[..], None, wide_ints(), (2, 3)),
            (&rows[..], Some("3"), WireFormat::default(), (50, 100)),
            (&rows[..1], Some("0"), wide_ints(), (1, 1)),
            (&rows[..0], None, wide_ints(), (2, 3)),
        ] {
            let metadata = SnapshotMetadata {
                subscription_id: "sub-1",
                snapshot_id: "snapshot-1",
                authoritative: true,
                mode: Mode::List,
                view_id: "Thing/list",
                key,
            };
            let config = SnapshotBatchConfig {
                initial_batch_size: initial,
                subsequent_batch_size: subsequent,
            };
            let batches = create_snapshot_batches(rows, &wire_format, metadata, &config);
            let mut offset = 0;
            for batch in &batches {
                let copies = rows[offset..offset + batch.data.rows.len()]
                    .iter()
                    .map(|(key, entity)| SnapshotEntity {
                        key: key.clone(),
                        data: wire_value(entity, &wire_format),
                    })
                    .collect();
                offset += batch.data.rows.len();
                let frame = SnapshotFrame {
                    protocol_version: PROTOCOL_VERSION,
                    subscription_id: "sub-1".to_string(),
                    snapshot_id: "snapshot-1".to_string(),
                    authoritative: true,
                    mode: Mode::List,
                    export: "Thing/list".to_string(),
                    op: "snapshot",
                    key: key.map(str::to_string),
                    data: copies,
                    complete: batch.complete,
                };
                assert_eq!(
                    serde_json::to_string(batch).unwrap(),
                    serde_json::to_string(&frame).unwrap()
                );
            }
            assert_eq!(offset, rows.len());
        }
    }

    /// A membership frame serialized from a shared entity is byte for byte
    /// the frame built from a formatted copy of it.
    #[test]
    fn entity_frames_serialize_as_frames_of_formatted_copies() {
        for (_, entity) in sample_rows() {
            for (wire_format, seq) in [
                (wide_ints(), Some("10:000000000001")),
                (WireFormat::default(), None),
            ] {
                let frame = ScopedEntityFrame {
                    protocol_version: PROTOCOL_VERSION,
                    subscription_id: "sub-1",
                    mode: Mode::List,
                    export: "Thing/latest",
                    op: "upsert",
                    key: "7",
                    data: WireEntity {
                        entity: &entity,
                        wire_format: &wire_format,
                    },
                    seq,
                };
                let copy = Frame::scoped(
                    "sub-1",
                    Mode::List,
                    "Thing/latest",
                    "upsert",
                    "7",
                    wire_value(&entity, &wire_format),
                    seq.map(str::to_string),
                );
                assert_eq!(
                    serde_json::to_string(&frame).unwrap(),
                    serde_json::to_string(&copy).unwrap()
                );
            }
        }
    }

    #[test]
    fn lag_recovery_is_authoritative_for_an_after_query() {
        let subscription = Subscription {
            protocol_version: PROTOCOL_VERSION,
            subscription_id: "sub-1".to_string(),
            query: SubscriptionQuery {
                view: "Thing/list".to_string(),
                after: Some("40:000000000010".to_string()),
                ..Default::default()
            },
            snapshot: Default::default(),
        };

        assert!(!SnapshotPurpose::Initial.authoritative(&subscription));
        assert!(SnapshotPurpose::Recovery.authoritative(&subscription));
    }

    #[test]
    fn dot_path_filters_are_exact_and_type_sensitive() {
        let mut query = SubscriptionQuery {
            view: "Thing/list".to_string(),
            ..Default::default()
        };
        query
            .filters
            .insert("state.status".to_string(), json!("open"));
        query.filters.insert("metrics.count".to_string(), json!(2));
        assert!(query_matches_entity(
            &query,
            "one",
            &json!({"state": {"status": "open"}, "metrics": {"count": 2}}),
        ));
        assert!(!query_matches_entity(
            &query,
            "one",
            &json!({"state": {"status": "open"}, "metrics": {"count": "2"}}),
        ));
    }

    #[test]
    fn take_and_skip_define_independent_deterministic_windows() {
        let entities: Vec<_> = (1..=6)
            .map(|id| {
                (
                    id.to_string(),
                    json!({"id": id, "_seq": format!("10:{id:012}")}),
                )
            })
            .collect();
        let first = SubscriptionQuery {
            view: "Thing/list".to_string(),
            take: Some(2),
            skip: Some(0),
            ..Default::default()
        };
        let second = SubscriptionQuery {
            skip: Some(2),
            ..first.clone()
        };
        let first_keys: Vec<_> = select_query_entities(entities.clone(), &first, false, false)
            .into_iter()
            .map(|(key, _)| key)
            .collect();
        let second_keys: Vec<_> = select_query_entities(entities, &second, false, false)
            .into_iter()
            .map(|(key, _)| key)
            .collect();
        assert_eq!(first_keys, ["6", "5"]);
        assert_eq!(second_keys, ["4", "3"]);
    }

    #[tokio::test]
    async fn state_receiver_is_installed_before_snapshot_awaits() {
        let bus = BusManager::new();
        let (snapshot_started_tx, snapshot_started_rx) = oneshot::channel();
        let (release_snapshot_tx, release_snapshot_rx) = oneshot::channel();
        let bus_for_task = bus.clone();
        let task = tokio::spawn(async move {
            subscribe_state_then_snapshot(&bus_for_task, "Thing/state", "one", || async move {
                snapshot_started_tx.send(()).unwrap();
                release_snapshot_rx.await.unwrap();
            })
            .await
            .0
        });
        snapshot_started_rx.await.unwrap();
        bus.publish_state(
            "Thing/state",
            "one",
            Arc::new(Bytes::from_static(br#"{"op":"patch"}"#)),
        )
        .await;
        release_snapshot_tx.send(()).unwrap();
        let mut receiver = task.await.unwrap();
        receiver.changed().await.unwrap();
        assert!(!receiver.borrow().payload.is_empty());
    }

    async fn assert_list_receiver_precedes_snapshot(view: &'static str) {
        let bus = BusManager::new();
        let (snapshot_started_tx, snapshot_started_rx) = oneshot::channel();
        let (release_snapshot_tx, release_snapshot_rx) = oneshot::channel();
        let bus_for_task = bus.clone();
        let task = tokio::spawn(async move {
            subscribe_list_then_snapshot(&bus_for_task, view, || async move {
                snapshot_started_tx.send(()).unwrap();
                release_snapshot_rx.await.unwrap();
            })
            .await
            .0
        });
        snapshot_started_rx.await.unwrap();
        bus.publish_list(
            view,
            Arc::new(BusMessage {
                key: "one".to_string(),
                entity: view.to_string(),
                payload: Arc::new(Bytes::from_static(br#"{"op":"patch"}"#)),
            }),
        )
        .await;
        release_snapshot_tx.send(()).unwrap();
        let mut receiver = task.await.unwrap();
        assert_eq!(receiver.recv().await.unwrap().key, "one");
    }

    #[tokio::test]
    async fn list_receiver_is_installed_before_snapshot() {
        assert_list_receiver_precedes_snapshot("Thing/list").await;
    }

    #[tokio::test]
    async fn append_receiver_is_installed_before_snapshot() {
        assert_list_receiver_precedes_snapshot("Thing/append").await;
    }

    #[tokio::test]
    async fn derived_source_receiver_is_installed_before_snapshot() {
        assert_list_receiver_precedes_snapshot("Thing/list-source").await;
    }

    #[tokio::test]
    async fn unsorted_derived_reads_preserve_filters_order_and_snapshot_window() {
        use crate::materialized_view::{CompareOp, FilterConfig, ViewPipeline};
        let cache = EntityCache::new();
        for id in 1..=6 {
            cache
                .upsert(
                    "Thing/list",
                    &id.to_string(),
                    json!({
                        "_seq": format!("{}:0", id * 10), "active": id != 6,
                        "owner": "alice", "id": id
                    }),
                )
                .await;
        }
        let mut spec = list_spec();
        spec.id = "Thing/custom".into();
        spec.source_view = Some("Thing/list".into());
        spec.pipeline = Some(ViewPipeline::default());
        let caches = Arc::new(tokio::sync::RwLock::new(HashMap::new()));
        let mut query = SubscriptionQuery {
            view: spec.id.clone(),
            ..Default::default()
        };
        let rows = load_query_entities(&cache, Some(caches.clone()), &spec, &query, false).await;
        assert_eq!(
            rows.iter().map(|(key, _)| key.as_str()).collect::<Vec<_>>(),
            ["6", "5", "4", "3", "2", "1"]
        );
        spec.pipeline.as_mut().unwrap().filter = Some(FilterConfig {
            field_path: vec!["active".into()],
            op: CompareOp::Eq,
            value: json!(true),
        });
        query.filters.insert("owner".into(), json!("alice"));
        query.skip = Some(1);
        query.take = Some(3);
        query.snapshot_limit = Some(1);
        let rows = load_query_entities(&cache, Some(caches.clone()), &spec, &query, false).await;
        assert_eq!(
            rows.iter().map(|(key, _)| key.as_str()).collect::<Vec<_>>(),
            ["4", "3", "2"]
        );
        assert_eq!(
            load_query_entities(&cache, Some(caches.clone()), &spec, &query, true)
                .await
                .len(),
            1
        );
        query.skip = None;
        query.after = Some("20:0".into());
        let rows = load_query_entities(&cache, Some(caches.clone()), &spec, &query, false).await;
        assert_eq!(
            rows.iter().map(|(key, _)| key.as_str()).collect::<Vec<_>>(),
            ["3", "4", "5"]
        );
        query.filters.insert("owner".into(), json!("bob"));
        assert!(
            load_query_entities(&cache, Some(caches), &spec, &query, false)
                .await
                .is_empty()
        );
    }

    #[tokio::test]
    async fn snapshot_limit_does_not_change_live_take_skip_membership() {
        let cache = EntityCache::with_config(EntityCacheConfig {
            max_entities_per_view: 10,
            ..Default::default()
        });
        for id in 1..=4 {
            cache
                .upsert(
                    "Thing/list",
                    &id.to_string(),
                    json!({"_seq": format!("10:{id:012}")}),
                )
                .await;
        }
        let query = SubscriptionQuery {
            view: "Thing/list".to_string(),
            take: Some(3),
            skip: Some(1),
            snapshot_limit: Some(1),
            ..Default::default()
        };
        let live = load_query_entities(&cache, None, &list_spec(), &query, false).await;
        let snapshot = load_query_entities(&cache, None, &list_spec(), &query, true).await;
        assert_eq!(live.len(), 3);
        assert_eq!(snapshot.len(), 1);
    }

    #[test]
    fn fixture_manifest_covers_required_conformance_cases() {
        let manifest: Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/websocket-v2/manifest.json"
        ))
        .unwrap();
        let names: HashSet<_> = manifest["fixtures"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        for required in [
            "keyed-state.json",
            "list-windows.json",
            "filters.json",
            "multi-batch-authoritative.json",
            "empty-snapshot.json",
            "remove.json",
            "delete.json",
            "incremental-snapshot.json",
            "reconnect-replacement.json",
            "errors.json",
            "whole-entities.json",
        ] {
            assert!(names.contains(required), "missing fixture {required}");
        }

        for document in [
            include_str!("../../../../tests/fixtures/websocket-v2/keyed-state.json"),
            include_str!("../../../../tests/fixtures/websocket-v2/list-windows.json"),
            include_str!("../../../../tests/fixtures/websocket-v2/filters.json"),
            include_str!("../../../../tests/fixtures/websocket-v2/multi-batch-authoritative.json"),
            include_str!("../../../../tests/fixtures/websocket-v2/empty-snapshot.json"),
            include_str!("../../../../tests/fixtures/websocket-v2/remove.json"),
            include_str!("../../../../tests/fixtures/websocket-v2/delete.json"),
            include_str!("../../../../tests/fixtures/websocket-v2/incremental-snapshot.json"),
            include_str!("../../../../tests/fixtures/websocket-v2/reconnect-replacement.json"),
            include_str!("../../../../tests/fixtures/websocket-v2/errors.json"),
            include_str!("../../../../tests/fixtures/websocket-v2/whole-entities.json"),
        ] {
            let fixture: Value = serde_json::from_str(document).unwrap();
            assert!(fixture["name"].is_string());
        }
    }

    fn append_frame(key: &str, data: Value) -> Arc<Bytes> {
        let frame = json!({
            "entity": "Trade/append",
            "op": "patch",
            "key": key,
            "offset": 7,
            "data": data,
        });
        Arc::new(Bytes::from(serde_json::to_vec(&frame).unwrap()))
    }

    /// A replayable subscription must honour the same predicates a live
    /// collection subscription does; otherwise a filtered consumer receives
    /// events outside the query it asked for.
    #[test]
    fn replay_delivery_applies_key_partition_and_filters() {
        let matching = append_frame("pool1", json!({"_partition": "us", "side": "buy"}));
        let other_partition = append_frame("pool1", json!({"_partition": "eu", "side": "buy"}));
        let other_side = append_frame("pool1", json!({"_partition": "us", "side": "sell"}));

        let unfiltered = SubscriptionQuery {
            view: "Trade/append".to_string(),
            ..Default::default()
        };
        assert!(live_frame_matches(&unfiltered, "pool1", &matching));
        assert!(live_frame_matches(&unfiltered, "pool9", &matching));

        let keyed = SubscriptionQuery {
            view: "Trade/append".to_string(),
            key: Some("pool1".to_string()),
            ..Default::default()
        };
        assert!(live_frame_matches(&keyed, "pool1", &matching));
        assert!(!live_frame_matches(&keyed, "pool2", &matching));

        let partitioned = SubscriptionQuery {
            view: "Trade/append".to_string(),
            partition: Some("us".to_string()),
            ..Default::default()
        };
        assert!(live_frame_matches(&partitioned, "pool1", &matching));
        assert!(!live_frame_matches(&partitioned, "pool1", &other_partition));

        let filtered = SubscriptionQuery {
            view: "Trade/append".to_string(),
            filters: [("side".to_string(), json!("buy"))].into_iter().collect(),
            ..Default::default()
        };
        assert!(live_frame_matches(&filtered, "pool1", &matching));
        assert!(!live_frame_matches(&filtered, "pool1", &other_side));
    }

    #[test]
    fn a_frame_without_decodable_data_does_not_satisfy_a_filter() {
        let filtered = SubscriptionQuery {
            view: "Trade/append".to_string(),
            filters: [("side".to_string(), json!("buy"))].into_iter().collect(),
            ..Default::default()
        };
        let garbage = Arc::new(Bytes::from_static(b"not json"));
        assert!(!live_frame_matches(&filtered, "pool1", &garbage));
    }

    /// The recovery cursor must be the last offset delivered *before* the
    /// gap. Reporting the newest offset seen would step the consumer over
    /// the skipped records permanently.
    #[test]
    fn replay_lagged_recovers_from_before_the_gap() {
        let epoch = crate::journal::JournalEpoch::new();
        let issue = SocketIssueMessage::replay_lagged(
            Some("trades".to_string()),
            37,
            Some(crate::journal::Cursor {
                epoch: epoch.clone(),
                offset: 4180,
            }),
        );
        assert_eq!(issue.code, "replay-lagged");
        assert_eq!(issue.recover_from, Some(format!("{epoch}:4180")));
        assert!(
            issue.suggested_action.unwrap().contains("4180"),
            "the consumer is told exactly which cursor recovers the gap"
        );

        // Nothing delivered yet: there is no pre-gap offset, so the whole
        // retained window is the recovery.
        let from_scratch = SocketIssueMessage::replay_lagged(Some("trades".to_string()), 9, None);
        assert_eq!(from_scratch.recover_from, None);
        assert!(from_scratch
            .suggested_action
            .unwrap()
            .contains("without `after`"));
    }

    fn bus_message(key: &str) -> Arc<BusMessage> {
        Arc::new(BusMessage {
            key: key.to_string(),
            entity: "Trade/append".to_string(),
            payload: Arc::new(Bytes::from_static(b"{}")),
        })
    }

    /// A long replay must keep the bus drained. The broadcast buffer is
    /// bounded, so a busy view would otherwise lap the replay and the first
    /// live `recv` would return `Lagged` — telling the client to resubscribe,
    /// starting another long replay, which laps again.
    #[tokio::test]
    async fn draining_during_a_replay_keeps_a_busy_view_from_lapping_it() {
        let (sender, mut receiver) = broadcast::channel::<Arc<BusMessage>>(16);
        let mut pending = VecDeque::new();
        let mut lagged = None;

        // Publish more than the channel holds, draining as a replay would
        // between sends.
        for index in 0..48 {
            sender.send(bus_message(&format!("k{index}"))).unwrap();
            drain_available(&mut receiver, &mut pending, &mut lagged);
        }

        assert_eq!(lagged, None, "draining as we go means nothing is dropped");
        assert_eq!(pending.len(), 48, "every published frame is buffered");
    }

    #[tokio::test]
    async fn a_replay_that_never_drains_is_reported_as_a_gap() {
        let (sender, mut receiver) = broadcast::channel::<Arc<BusMessage>>(8);
        for index in 0..32 {
            sender.send(bus_message(&format!("k{index}"))).unwrap();
        }

        let mut pending = VecDeque::new();
        let mut lagged = None;
        drain_available(&mut receiver, &mut pending, &mut lagged);

        assert!(
            lagged.is_some(),
            "overflowing the bus is a gap, not silent truncation"
        );
    }

    /// Everything still on the bus after a gap is on the far side of it.
    /// Buffering it would put those frames in front of the lag report and
    /// advance the recovery cursor past the records it is meant to recover.
    #[tokio::test]
    async fn nothing_after_a_gap_is_buffered_ahead_of_the_report() {
        let (sender, mut receiver) = broadcast::channel::<Arc<BusMessage>>(8);
        let mut pending = VecDeque::new();
        let mut lagged = None;

        // Delivered and buffered normally.
        sender.send(bus_message("before")).unwrap();
        drain_available(&mut receiver, &mut pending, &mut lagged);
        assert_eq!(pending.len(), 1);

        // Overflow the bus: everything published from here is past the gap.
        for index in 0..32 {
            sender.send(bus_message(&format!("lost{index}"))).unwrap();
        }
        drain_available(&mut receiver, &mut pending, &mut lagged);
        assert!(lagged.is_some());

        // The bus still holds what survived the overflow, all of it past the
        // gap. A later iteration of the replay loop drains again.
        let buffered_at_gap = pending.len();
        drain_available(&mut receiver, &mut pending, &mut lagged);
        assert_eq!(
            pending.len(),
            buffered_at_gap,
            "post-gap frames must not join the pre-gap flush"
        );
        assert_eq!(pending.front().unwrap().key, "before");
    }
    /// The bus is subscribed before the tape is read, so a record published
    /// in that window arrives on both paths.
    #[test]
    fn the_seam_between_replay_and_live_neither_repeats_nor_skips() {
        let mut last_sent = Some(4211);

        assert!(
            already_delivered(Some(4211), &mut last_sent),
            "the record the replay ended on must not be sent twice"
        );
        assert!(already_delivered(Some(4100), &mut last_sent));
        assert_eq!(last_sent, Some(4211), "a duplicate never moves the mark");

        assert!(
            !already_delivered(Some(4212), &mut last_sent),
            "the next record is new"
        );
        assert_eq!(last_sent, Some(4212));

        // A frame with no offset comes from a view with no tape; it cannot
        // have been replayed, and must not disturb the mark.
        assert!(!already_delivered(None, &mut last_sent));
        assert_eq!(last_sent, Some(4212));
    }

    /// A subscription with no cursor has delivered nothing, so the first live
    /// frame is not a duplicate.
    #[test]
    fn a_fresh_subscription_delivers_its_first_live_frame() {
        let mut last_sent = None;
        assert!(!already_delivered(Some(0), &mut last_sent));
        assert_eq!(last_sent, Some(0));
    }

    /// End-to-end over a real socket: the pieces above are unit-tested
    /// individually, but the thing a consumer actually does — reconnect with
    /// a stored cursor and keep reading — only exists once a subscription is
    /// attached to a connection.
    mod over_a_socket {
        use super::*;
        use crate::journal::{EventJournal, JournalConfig};
        use crate::projector::Projector;
        use crate::{MutationBatch, SlotContext};
        use arete_interpreter::Mutation;
        use futures_util::{SinkExt, StreamExt};
        use std::time::Duration;
        use tokio::net::{TcpListener, TcpStream};
        use tokio::sync::mpsc;
        use tokio_tungstenite::tungstenite::Message;
        use tokio_tungstenite::{client_async, WebSocketStream};

        const RETAINED: u64 = 600;

        fn append_index() -> ViewIndex {
            let mut index = ViewIndex::new();
            index.add_spec(ViewSpec {
                id: "Trade/append".to_string(),
                export: "Trade".to_string(),
                mode: Mode::Append,
                wire_format: Default::default(),
                projection: Projection::all(),
                filters: Filters::all(),
                delivery: Delivery::default(),
                pipeline: None,
                source_view: None,
            });
            index
        }

        fn trade(index: u64) -> MutationBatch {
            MutationBatch::with_slot_context(
                vec![Mutation {
                    export: "Trade".to_string(),
                    key: json!(format!("pool{}", index % 4)),
                    patch: json!({"trade": index}),
                    append: vec![],
                }]
                .into_iter()
                .collect(),
                SlotContext::new(100 + index / 3, index % 3),
            )
        }

        struct Harness {
            addr: SocketAddr,
            journal: Arc<EventJournal>,
            tx: mpsc::Sender<MutationBatch>,
        }

        impl Harness {
            async fn start() -> Self {
                let view_index = Arc::new(append_index());
                let entity_cache = EntityCache::new();
                let bus_manager = BusManager::new();
                let journal = Arc::new(EventJournal::new(JournalConfig {
                    enabled: true,
                    max_bytes_per_view: u64::MAX,
                    max_records_per_view: 10_000,
                    max_age: Duration::from_secs(3_600),
                }));

                let (tx, rx) = mpsc::channel::<MutationBatch>(256);
                tokio::spawn(
                    Projector::new(
                        view_index.clone(),
                        bus_manager.clone(),
                        entity_cache.clone(),
                        rx,
                        #[cfg(feature = "otel")]
                        None,
                    )
                    .with_journal(journal.clone())
                    .run(),
                );

                let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
                let addr = listener.local_addr().unwrap();
                let server = WebSocketServer::new(
                    addr,
                    bus_manager,
                    entity_cache,
                    view_index,
                    #[cfg(feature = "otel")]
                    None,
                )
                .with_journal(journal.clone());
                let (acceptor, _cleanup) = server.into_acceptor();
                tokio::spawn(async move { acceptor.serve_listener(listener).await });

                Self { addr, journal, tx }
            }

            async fn publish(&self, range: std::ops::Range<u64>) {
                for index in range {
                    self.tx.send(trade(index)).await.unwrap();
                }
                let (ack, wait) = oneshot::channel();
                self.tx
                    .send(MutationBatch::flush_marker(ack))
                    .await
                    .unwrap();
                wait.await.unwrap();
            }

            async fn connect(&self) -> WebSocketStream<TcpStream> {
                let stream = TcpStream::connect(self.addr).await.unwrap();
                client_async(format!("ws://{}/", self.addr), stream)
                    .await
                    .unwrap()
                    .0
            }
        }

        async fn next_frame(socket: &mut WebSocketStream<TcpStream>) -> Value {
            loop {
                let message = tokio::time::timeout(Duration::from_secs(10), socket.next())
                    .await
                    .expect("the server answers within the timeout")
                    .expect("the stream stays open")
                    .expect("a readable frame");
                // Control and data frames arrive as binary; issue frames as
                // text. Both are JSON.
                let bytes = match &message {
                    Message::Text(text) => text.as_bytes(),
                    Message::Binary(bytes) => bytes.as_ref(),
                    _ => continue,
                };
                return serde_json::from_slice(bytes).expect("frames are JSON");
            }
        }

        /// Collect `count` event frames, ignoring anything else on the wire.
        async fn collect_trades(socket: &mut WebSocketStream<TcpStream>, count: usize) -> Vec<u64> {
            let mut offsets = Vec::with_capacity(count);
            while offsets.len() < count {
                let frame = next_frame(socket).await;
                assert_ne!(
                    frame["type"], "error",
                    "no error frame should interrupt delivery: {frame}"
                );
                if let Some(offset) = frame["offset"].as_u64() {
                    offsets.push(offset);
                }
            }
            offsets
        }

        /// The headline claim: reconnecting with a stored cursor delivers every
        /// event published since it, in order, and then continues live without
        /// a duplicate or a hole at the seam.
        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn a_reconnect_replays_from_a_cursor_and_continues_live() {
            let harness = Harness::start().await;
            harness.publish(0..RETAINED).await;

            let cursor = harness.journal.window("Trade/append").await;
            let stored = format!("{}:{}", cursor.epoch, 99);

            let mut socket = harness.connect().await;
            socket
                .send(Message::Text(
                    json!({
                        "type": "subscribe",
                        "protocolVersion": 2,
                        "subscriptionId": "trades",
                        "query": {"view": "Trade/append", "after": stored},
                    })
                    .to_string()
                    .into(),
                ))
                .await
                .unwrap();

            let ack = next_frame(&mut socket).await;
            assert_eq!(ack["op"], "subscribed", "unexpected ack: {ack}");
            assert_eq!(ack["replayWindow"]["next"], json!(RETAINED));

            // Well over the 500 a single read or buffer would cover.
            let replayed = collect_trades(&mut socket, (RETAINED - 100) as usize).await;
            assert_eq!(
                replayed,
                (100..RETAINED).collect::<Vec<_>>(),
                "every event after the cursor, in order, exactly once"
            );

            // Published only now, so these can only arrive over the live path.
            harness.publish(RETAINED..RETAINED + 40).await;
            let live = collect_trades(&mut socket, 40).await;
            assert_eq!(
                live,
                (RETAINED..RETAINED + 40).collect::<Vec<_>>(),
                "the live stream resumes exactly where the replay stopped"
            );

            socket.close(None).await.ok();
        }

        /// Events published *during* the replay must still arrive. The replay
        /// and the live subscription are separate reads of the same tape, and
        /// the seam between them is where a naive implementation drops or
        /// repeats.
        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn events_published_during_a_replay_are_not_lost() {
            let harness = Harness::start().await;
            harness.publish(0..RETAINED).await;

            let epoch = harness.journal.window("Trade/append").await.epoch;
            let mut socket = harness.connect().await;
            socket
                .send(Message::Text(
                    json!({
                        "type": "subscribe",
                        "protocolVersion": 2,
                        "subscriptionId": "trades",
                        "query": {"view": "Trade/append", "after": format!("{epoch}:0")},
                    })
                    .to_string()
                    .into(),
                ))
                .await
                .unwrap();
            let ack = next_frame(&mut socket).await;
            assert_eq!(ack["op"], "subscribed", "unexpected ack: {ack}");

            // Keep publishing while the replay is still draining.
            harness.publish(RETAINED..RETAINED + 200).await;

            let total = (RETAINED + 200 - 1) as usize;
            let delivered = collect_trades(&mut socket, total).await;
            assert_eq!(
                delivered,
                (1..RETAINED + 200).collect::<Vec<_>>(),
                "replay and live output join without a gap or a repeat"
            );

            socket.close(None).await.ok();
        }

        /// A cursor from another tape lifetime is refused rather than served
        /// as a continuation, and the refusal releases the subscription id.
        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn a_stale_epoch_is_refused_and_frees_the_subscription_id() {
            let harness = Harness::start().await;
            harness.publish(0..50).await;

            let mut socket = harness.connect().await;
            socket
                .send(Message::Text(
                    json!({
                        "type": "subscribe",
                        "protocolVersion": 2,
                        "subscriptionId": "trades",
                        "query": {
                            "view": "Trade/append",
                            "after": format!("{}:10", crate::journal::JournalEpoch::new()),
                        },
                    })
                    .to_string()
                    .into(),
                ))
                .await
                .unwrap();

            let error = next_frame(&mut socket).await;
            assert_eq!(error["type"], "error", "unexpected frame: {error}");
            assert_eq!(error["code"], "cursor-epoch-changed");

            // The documented recovery is to resubscribe without a cursor. That
            // only works if the refused attempt released the id.
            socket
                .send(Message::Text(
                    json!({
                        "type": "subscribe",
                        "protocolVersion": 2,
                        "subscriptionId": "trades",
                        "query": {"view": "Trade/append"},
                    })
                    .to_string()
                    .into(),
                ))
                .await
                .unwrap();
            let ack = next_frame(&mut socket).await;
            assert_eq!(
                ack["op"], "subscribed",
                "the refused id must be reusable: {ack}"
            );

            assert_eq!(collect_trades(&mut socket, 50).await.len(), 50);
            socket.close(None).await.ok();
        }

        const ROUND: &str = "Round/state";

        /// A state view with no projector: the test writes the cache and the
        /// bus itself, so it controls exactly when the subscriber can run.
        #[derive(Clone)]
        struct StateServer {
            addr: SocketAddr,
            bus_manager: BusManager,
            entity_cache: EntityCache,
        }

        impl StateServer {
            async fn start() -> Self {
                let mut index = ViewIndex::new();
                index.add_spec(ViewSpec {
                    id: ROUND.to_string(),
                    export: "Round".to_string(),
                    mode: Mode::State,
                    wire_format: Default::default(),
                    projection: Projection::all(),
                    filters: Filters::all(),
                    delivery: Delivery::default(),
                    pipeline: None,
                    source_view: None,
                });
                let bus_manager = BusManager::new();
                let entity_cache = EntityCache::new();
                let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
                let addr = listener.local_addr().unwrap();
                let server = WebSocketServer::new(
                    addr,
                    bus_manager.clone(),
                    entity_cache.clone(),
                    Arc::new(index),
                    #[cfg(feature = "otel")]
                    None,
                );
                let (acceptor, _cleanup) = server.into_acceptor();
                tokio::spawn(async move { acceptor.serve_listener(listener).await });
                Self {
                    addr,
                    bus_manager,
                    entity_cache,
                }
            }

            /// Publish a patch the way the projector does: stamp its `_seq`,
            /// write the cache, then the key's bus.
            async fn publish(&self, key: &str, mut patch: Value, seq: &str) {
                patch["_seq"] = Value::String(seq.to_string());
                self.entity_cache
                    .upsert_with_append(ROUND, key, patch.clone(), &[], PatchOrigin::Unknown)
                    .await;
                self.publish_frame(key, "patch", patch, seq).await;
            }

            /// A complete new lifetime after an explicit source deletion.
            async fn create(&self, key: &str, mut entity: Value, seq: &str) {
                entity["_seq"] = Value::String(seq.to_string());
                self.entity_cache
                    .upsert_with_append(ROUND, key, entity.clone(), &[], PatchOrigin::Creation)
                    .await;
                self.publish_frame(key, "upsert", entity, seq).await;
            }

            /// Publish a frame to the key's bus without touching the cache.
            async fn publish_frame(&self, key: &str, op: &str, data: Value, seq: &str) {
                let frame = json!({
                    "mode": "state",
                    "entity": ROUND,
                    "op": op,
                    "key": key,
                    "data": data,
                    "seq": seq,
                });
                self.bus_manager
                    .publish_state(ROUND, key, Arc::new(Bytes::from(frame.to_string())))
                    .await;
            }

            /// Subscribe to one key and read through its snapshot.
            async fn subscribe(&self, key: &str) -> WebSocketStream<TcpStream> {
                let stream = TcpStream::connect(self.addr).await.unwrap();
                let mut socket = client_async(format!("ws://{}/", self.addr), stream)
                    .await
                    .unwrap()
                    .0;
                socket
                    .send(Message::Text(
                        json!({
                            "type": "subscribe",
                            "protocolVersion": 2,
                            "subscriptionId": "round",
                            "query": {"view": ROUND, "key": key},
                        })
                        .to_string()
                        .into(),
                    ))
                    .await
                    .unwrap();
                let ack = next_frame(&mut socket).await;
                assert_eq!(ack["op"], "subscribed", "unexpected ack: {ack}");
                loop {
                    let frame = next_frame(&mut socket).await;
                    assert_eq!(frame["op"], "snapshot", "unexpected frame: {frame}");
                    if frame["complete"] == true {
                        return socket;
                    }
                }
            }
        }

        /// Two patches to one key, published before the subscriber reads the
        /// first, must both reach it. The bus holds only the latest frame, so
        /// forwarding it alone drops the fields of the one it replaced.
        // One worker, so tasks run one at a time. The handshake needs the
        // multi-threaded runtime.
        #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
        async fn a_state_subscriber_keeps_the_fields_of_an_overwritten_patch() {
            let server = StateServer::start().await;
            server
                .publish("7", json!({"id": 7}), "100:000000000001")
                .await;
            let mut socket = server.subscribe("7").await;

            // A lone patch is forwarded as it was published. This one is an
            // account update, numbered by its write version.
            server
                .publish(
                    "7",
                    json!({"results": {"slot_hash": "abc"}}),
                    "140:003836292257",
                )
                .await;
            let frame = next_frame(&mut socket).await;
            assert_eq!(frame["op"], "patch", "unexpected frame: {frame}");
            assert_eq!(frame["seq"], "140:003836292257");

            // Two instruction patches from later in the same slot, numbered by
            // transaction index. Both writes happen in one task that never
            // yields to the subscriber, so the second replaces the first on
            // the bus before the subscriber wakes.
            let writer = server.clone();
            tokio::spawn(async move {
                writer
                    .publish("7", json!({"entropy": {"seed": "def"}}), "140:000000001200")
                    .await;
                writer
                    .publish("7", json!({"total": 1}), "140:000000001200")
                    .await;
            })
            .await
            .unwrap();
            let frame = next_frame(&mut socket).await;
            assert_eq!(frame["op"], "patch", "unexpected frame: {frame}");
            // Everything that changed since the snapshot, the forwarded patch
            // included; `id` has not.
            assert_eq!(
                frame["data"],
                json!({
                    "results": {"slot_hash": "abc"},
                    "entropy": {"seed": "def"},
                    "total": 1,
                    "_seq": "140:000000001200",
                })
            );
            // The latest seq sorts below the one already delivered, and a
            // client would drop the frame as stale.
            assert!(frame.get("seq").is_none(), "unexpected seq: {frame}");

            // Having caught up, the subscriber forwards patches again.
            server
                .publish("7", json!({"total": 2}), "141:000000000002")
                .await;
            let frame = next_frame(&mut socket).await;
            assert_eq!(frame["op"], "patch", "unexpected frame: {frame}");
            assert_eq!(
                frame["data"],
                json!({"total": 2, "_seq": "141:000000000002"})
            );

            // The next catch-up is measured against the last one: only what
            // changed since it, not the fields that one already carried.
            let writer = server.clone();
            tokio::spawn(async move {
                writer
                    .publish("7", json!({"total": 3}), "142:000000000001")
                    .await;
                writer
                    .publish("7", json!({"flag": true}), "142:000000000001")
                    .await;
            })
            .await
            .unwrap();
            let frame = next_frame(&mut socket).await;
            assert_eq!(frame["op"], "patch", "unexpected frame: {frame}");
            assert!(frame.get("seq").is_none(), "unexpected seq: {frame}");
            assert_eq!(
                frame["data"],
                json!({"total": 3, "flag": true, "_seq": "142:000000000001"})
            );

            socket.close(None).await.ok();
        }

        /// A whole entity forwarded as it was published replaces the client's
        /// copy, so a later catch-up is measured against it.
        #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
        async fn a_forwarded_whole_entity_is_what_a_catch_up_is_measured_against() {
            let server = StateServer::start().await;
            server
                .publish("7", json!({"id": 7, "a": 1}), "100:000000000001")
                .await;
            let mut socket = server.subscribe("7").await;

            let whole = json!({"id": 7, "a": 1, "b": 2, "_seq": "101:000000000001"});
            server
                .entity_cache
                .store_whole(ROUND, "7", whole.clone())
                .await;
            server
                .publish_frame("7", "upsert", whole.clone(), "101:000000000001")
                .await;
            let frame = next_frame(&mut socket).await;
            assert_eq!(frame["op"], "upsert", "unexpected frame: {frame}");
            assert_eq!(frame["data"], whole);

            let writer = server.clone();
            tokio::spawn(async move {
                writer
                    .publish("7", json!({"c": 3}), "102:000000000001")
                    .await;
                writer
                    .publish("7", json!({"d": 4}), "102:000000000002")
                    .await;
            })
            .await
            .unwrap();
            let frame = next_frame(&mut socket).await;
            assert_eq!(frame["op"], "patch", "unexpected frame: {frame}");
            assert_eq!(
                frame["data"],
                json!({"c": 3, "d": 4, "_seq": "102:000000000002"}),
                "`b` came with the whole entity, so it is not sent again"
            );
            socket.close(None).await.ok();
        }

        /// A catch-up resends a field a forwarded patch set even when the
        /// entity has set it back to the value the client last received
        /// whole: the client still holds the forwarded value.
        #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
        async fn a_catch_up_restores_a_field_a_forwarded_patch_set() {
            let server = StateServer::start().await;
            server
                .publish("7", json!({"id": 7, "a": 1}), "100:000000000001")
                .await;
            let mut socket = server.subscribe("7").await;

            server
                .publish("7", json!({"a": 2}), "101:000000000001")
                .await;
            let frame = next_frame(&mut socket).await;
            assert_eq!(frame["op"], "patch", "unexpected frame: {frame}");
            assert_eq!(frame["data"], json!({"a": 2, "_seq": "101:000000000001"}));

            let writer = server.clone();
            tokio::spawn(async move {
                writer
                    .publish("7", json!({"a": 1}), "102:000000000001")
                    .await;
                writer
                    .publish("7", json!({"b": 1}), "102:000000000002")
                    .await;
            })
            .await
            .unwrap();
            let frame = next_frame(&mut socket).await;
            assert_eq!(frame["op"], "patch", "unexpected frame: {frame}");
            assert_eq!(
                frame["data"],
                json!({"a": 1, "b": 1, "_seq": "102:000000000002"}),
                "`a` matches the snapshot but the client holds 2"
            );
            socket.close(None).await.ok();
        }

        /// A key deleted and created again before the subscriber reads
        /// either frame: the bus keeps only the new entity's. Merged into the
        /// client's copy it would leave the deleted entity's fields in place,
        /// so the client gets the new entity whole, replacing its copy.
        #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
        async fn a_key_deleted_and_recreated_between_reads_replaces_the_clients_copy() {
            let server = StateServer::start().await;
            server
                .publish("7", json!({"id": 7, "old": true}), "100:000000000001")
                .await;
            let mut socket = server.subscribe("7").await;

            let writer = server.clone();
            tokio::spawn(async move {
                writer.entity_cache.remove(ROUND, "7").await;
                writer
                    .publish_frame("7", "delete", Value::Null, "101:000000000001")
                    .await;
                writer
                    .publish("7", json!({"id": 7, "fresh": true}), "101:000000000002")
                    .await;
            })
            .await
            .unwrap();
            let frame = next_frame(&mut socket).await;
            assert_eq!(frame["op"], "upsert", "unexpected frame: {frame}");
            assert_eq!(
                frame["data"],
                json!({"id": 7, "fresh": true, "_seq": "101:000000000002"})
            );
            assert!(frame.get("seq").is_none(), "unexpected seq: {frame}");

            // Deleted, with a patch overwritten before it: the delete is final.
            let writer = server.clone();
            tokio::spawn(async move {
                writer
                    .publish("7", json!({"n": 1}), "102:000000000001")
                    .await;
                writer.entity_cache.remove(ROUND, "7").await;
                writer
                    .publish_frame("7", "delete", Value::Null, "102:000000000002")
                    .await;
            })
            .await
            .unwrap();
            let frame = next_frame(&mut socket).await;
            assert_eq!(frame["op"], "delete", "unexpected frame: {frame}");

            // Created again: the client holds nothing, so it arrives whole.
            server
                .create("7", json!({"id": 7, "third": true}), "103:000000000001")
                .await;
            let frame = next_frame(&mut socket).await;
            assert_eq!(frame["op"], "upsert", "unexpected frame: {frame}");
            assert_eq!(frame["data"]["third"], true);
            assert!(frame["data"].get("fresh").is_none(), "{frame}");
            socket.close(None).await.ok();
        }

        /// A missed delete, then a patch the cache refused (it lacks the new
        /// entity): the client's copy is of the deleted entity, so the patch
        /// is not merged into it. The new entity replaces it once it is whole.
        #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
        async fn a_copy_of_a_deleted_entity_takes_no_patch_until_it_is_replaced() {
            let server = StateServer::start().await;
            server
                .publish("7", json!({"id": 7, "old": true}), "100:000000000001")
                .await;
            let mut socket = server.subscribe("7").await;

            let writer = server.clone();
            tokio::spawn(async move {
                writer.entity_cache.remove(ROUND, "7").await;
                writer
                    .publish_frame("7", "delete", Value::Null, "101:000000000001")
                    .await;
                writer
                    .publish_frame("7", "patch", json!({"n": 1}), "101:000000000002")
                    .await;
            })
            .await
            .unwrap();
            // Let the subscriber read the patch, and withhold it, before the
            // whole entity arrives. Nothing is sent, so there is no frame to
            // wait for.
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;

            let whole = json!({"id": 7, "n": 1, "_seq": "101:000000000002"});
            server
                .entity_cache
                .store_whole(ROUND, "7", whole.clone())
                .await;
            server
                .publish_frame("7", "upsert", whole.clone(), "101:000000000002")
                .await;
            let frame = next_frame(&mut socket).await;
            assert_eq!(frame["op"], "upsert", "unexpected frame: {frame}");
            assert_eq!(frame["data"], whole);
            socket.close(None).await.ok();
        }

        /// The source replaced the entity whole (an `upsert`, dropping a field)
        /// and then patched it, both before the subscriber read either. The
        /// patch alone, or the entity merged in, would keep the dropped field.
        #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
        async fn a_whole_entity_among_overwritten_frames_replaces_the_clients_copy() {
            let server = StateServer::start().await;
            server
                .publish("7", json!({"id": 7, "dropped": 1}), "100:000000000001")
                .await;
            let mut socket = server.subscribe("7").await;

            let writer = server.clone();
            tokio::spawn(async move {
                let whole = json!({"id": 7, "_seq": "101:000000000001"});
                writer
                    .entity_cache
                    .store_whole(ROUND, "7", whole.clone())
                    .await;
                writer
                    .publish_frame("7", "upsert", whole, "101:000000000001")
                    .await;
                writer
                    .publish("7", json!({"n": 1}), "101:000000000002")
                    .await;
            })
            .await
            .unwrap();
            let frame = next_frame(&mut socket).await;
            assert_eq!(frame["op"], "upsert", "unexpected frame: {frame}");
            assert_eq!(
                frame["data"],
                json!({"id": 7, "n": 1, "_seq": "101:000000000002"})
            );
            assert!(frame.get("seq").is_none(), "unexpected seq: {frame}");
            socket.close(None).await.ok();
        }

        /// Patches overwritten on the bus while the cache lacks the key (it
        /// evicted it, and refused them) leave nothing whole to send in their
        /// place: the holder gets the latest patch, as a holder of an evicted
        /// key does, and stays behind. The resend that brings the entity back
        /// carries the seq of the key's latest change, which the holder
        /// already has and would drop as stale, so it gets the cached entity
        /// without a seq instead: as an upsert, since the resend is the whole
        /// entity and replaces what the holder has.
        #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
        async fn a_holder_behind_on_an_evicted_key_catches_up_when_it_returns() {
            let server = StateServer::start().await;
            server
                .publish("7", json!({"id": 7}), "100:000000000001")
                .await;
            let mut socket = server.subscribe("7").await;
            server.entity_cache.remove(ROUND, "7").await;

            let writer = server.clone();
            tokio::spawn(async move {
                writer
                    .publish_frame("7", "patch", json!({"a": 1}), "101:000000000001")
                    .await;
                writer
                    .publish_frame("7", "patch", json!({"b": 2}), "101:000000000002")
                    .await;
            })
            .await
            .unwrap();
            let frame = next_frame(&mut socket).await;
            assert_eq!(frame["op"], "patch", "unexpected frame: {frame}");
            assert_eq!(frame["data"], json!({"b": 2}));
            assert_eq!(frame["seq"], "101:000000000002");

            // The resend: the whole entity, at the latest change's position.
            let whole = json!({"id": 7, "a": 1, "b": 2, "_seq": "101:000000000002"});
            server
                .entity_cache
                .store_whole(ROUND, "7", whole.clone())
                .await;
            server
                .publish_frame("7", "upsert", whole.clone(), "101:000000000002")
                .await;
            let frame = next_frame(&mut socket).await;
            assert_eq!(frame["op"], "upsert", "unexpected frame: {frame}");
            assert_eq!(frame["data"], whole);
            assert!(frame.get("seq").is_none(), "unexpected seq: {frame}");

            // Caught up: patches are forwarded again.
            server
                .publish("7", json!({"c": 3}), "102:000000000001")
                .await;
            let frame = next_frame(&mut socket).await;
            assert_eq!(frame["data"], json!({"c": 3, "_seq": "102:000000000001"}));
            assert_eq!(frame["seq"], "102:000000000001");
            socket.close(None).await.ok();
        }
    }

    /// A key only counts as held by the client once a frame actually carried
    /// it. A truncated or disabled snapshot leaves keys the client never saw,
    /// and their first change must arrive whole.
    mod partial_entities_over_a_socket {
        use super::*;
        use crate::projector::Projector;
        use crate::{MutationBatch, SlotContext};
        use arete_interpreter::{AccountPosition, Mutation};
        use futures_util::{SinkExt, StreamExt};
        use std::time::Duration;
        use tokio::net::{TcpListener, TcpStream};
        use tokio::sync::mpsc;
        use tokio_tungstenite::tungstenite::Message;
        use tokio_tungstenite::{client_async, WebSocketStream};

        fn thing_index() -> ViewIndex {
            let mut index = ViewIndex::new();
            index.add_spec(list_spec());
            index.add_spec(ViewSpec {
                id: "Thing/state".to_string(),
                mode: Mode::State,
                ..list_spec()
            });
            index
        }

        struct Harness {
            addr: SocketAddr,
            tx: mpsc::Sender<MutationBatch>,
            slot: std::sync::atomic::AtomicU64,
        }

        impl Harness {
            async fn start(delivery: WebSocketDeliveryConfig) -> Self {
                Self::start_with_cache(delivery, EntityCache::new()).await
            }

            async fn start_with_cache(
                delivery: WebSocketDeliveryConfig,
                entity_cache: EntityCache,
            ) -> Self {
                Self::start_with_index(delivery, entity_cache, thing_index()).await
            }

            async fn start_with_index(
                delivery: WebSocketDeliveryConfig,
                entity_cache: EntityCache,
                index: ViewIndex,
            ) -> Self {
                let view_index = Arc::new(index);
                let bus_manager = BusManager::new();
                let (tx, rx) = mpsc::channel::<MutationBatch>(64);
                // Unconstrained, so Tokio's cooperative budget never makes the
                // projector yield partway through a batch: a test on one
                // worker then knows no subscriber runs until the whole batch
                // is published (see
                // `snapshot_rows_patches_and_catch_ups_carry_the_version`).
                tokio::spawn(tokio::task::unconstrained(
                    Projector::new(
                        view_index.clone(),
                        bus_manager.clone(),
                        entity_cache.clone(),
                        rx,
                        #[cfg(feature = "otel")]
                        None,
                    )
                    .run(),
                ));
                let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
                let addr = listener.local_addr().unwrap();
                let server = WebSocketServer::new(
                    addr,
                    bus_manager,
                    entity_cache,
                    view_index,
                    #[cfg(feature = "otel")]
                    None,
                )
                .with_delivery_config(delivery);
                let (acceptor, _cleanup) = server.into_acceptor();
                tokio::spawn(async move { acceptor.serve_listener(listener).await });
                Self {
                    addr,
                    tx,
                    slot: std::sync::atomic::AtomicU64::new(100),
                }
            }

            /// Apply one source patch and wait until the projector published it.
            async fn patch(&self, key: &str, patch: Value) {
                self.mutate(Mutation {
                    export: "Thing".to_string(),
                    key: json!(key),
                    patch,
                    append: vec![],
                })
                .await;
            }

            async fn mutate(&self, mutation: Mutation) {
                let slot = self.slot.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                self.mutate_with_context(mutation, SlotContext::new(slot, 0))
                    .await;
            }

            async fn mutate_with_context(&self, mutation: Mutation, context: SlotContext) {
                self.tx
                    .send(MutationBatch::with_slot_context(
                        vec![mutation].into_iter().collect(),
                        context,
                    ))
                    .await
                    .unwrap();
                let (ack, wait) = oneshot::channel();
                self.tx
                    .send(MutationBatch::flush_marker(ack))
                    .await
                    .unwrap();
                wait.await.unwrap();
            }

            async fn subscribe(&self, query: Value, snapshot: bool) -> WebSocketStream<TcpStream> {
                let stream = TcpStream::connect(self.addr).await.unwrap();
                let mut socket = client_async(format!("ws://{}/", self.addr), stream)
                    .await
                    .unwrap()
                    .0;
                socket
                    .send(Message::Text(
                        json!({
                            "type": "subscribe",
                            "protocolVersion": 2,
                            "subscriptionId": "things",
                            "query": query,
                            "snapshot": {"enabled": snapshot},
                        })
                        .to_string()
                        .into(),
                    ))
                    .await
                    .unwrap();
                let ack = next_frame(&mut socket).await;
                assert_eq!(ack["op"], "subscribed", "unexpected ack: {ack}");
                assert_eq!(
                    ack["wholeEntities"],
                    json!(true),
                    "the ack advertises the whole-entity guarantee: {ack}"
                );
                socket
            }
        }

        async fn next_frame(socket: &mut WebSocketStream<TcpStream>) -> Value {
            loop {
                let message = tokio::time::timeout(Duration::from_secs(10), socket.next())
                    .await
                    .expect("the server answers within the timeout")
                    .expect("the stream stays open")
                    .expect("a readable frame");
                // Every frame here is far below the compression threshold.
                let bytes = match &message {
                    Message::Text(text) => text.as_bytes(),
                    Message::Binary(bytes) => bytes.as_ref(),
                    _ => continue,
                };
                return serde_json::from_slice(bytes).expect("frames are JSON");
            }
        }

        /// The snapshot rows, in order, once the snapshot completes.
        async fn snapshot_keys(socket: &mut WebSocketStream<TcpStream>) -> Vec<String> {
            let mut keys = Vec::new();
            loop {
                let frame = next_frame(socket).await;
                assert_eq!(frame["op"], "snapshot", "expected a snapshot: {frame}");
                keys.extend(
                    frame["data"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|row| row["key"].as_str().unwrap().to_string()),
                );
                if frame["complete"] == json!(true) {
                    return keys;
                }
            }
        }

        async fn seed(harness: &Harness) {
            for key in ["a", "b", "c"] {
                harness
                    .patch(key, json!({"name": format!("thing-{key}"), "count": 1}))
                    .await;
            }
        }

        fn derived_index() -> ViewIndex {
            use crate::materialized_view::{
                CompareOp, FilterConfig, SortConfig, SortOrder, ViewPipeline,
            };
            let mut index = thing_index();
            for name in ["empty", "filtered", "sorted"] {
                let mut pipeline = ViewPipeline::default();
                if name != "empty" {
                    pipeline.filter = Some(FilterConfig {
                        field_path: vec!["owner".into()],
                        op: CompareOp::Eq,
                        value: json!("alice"),
                    });
                    pipeline.limit = Some(2);
                }
                if name == "sorted" {
                    pipeline.sort = Some(SortConfig {
                        field_path: vec!["amount".into()],
                        order: SortOrder::Desc,
                    });
                }
                index.add_spec(ViewSpec {
                    id: format!("Thing/{name}"),
                    source_view: Some("Thing/list".into()),
                    pipeline: Some(pipeline),
                    ..list_spec()
                });
            }
            index
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn unsorted_derived_idle_bootstrap_and_reconnect_are_authoritative() {
            let harness = Harness::start_with_index(
                WebSocketDeliveryConfig::default(),
                EntityCache::new(),
                derived_index(),
            )
            .await;
            harness
                .patch("a", json!({"owner":"alice", "amount":1}))
                .await;
            harness.patch("b", json!({"owner":"bob", "amount":2})).await;
            harness
                .patch("c", json!({"owner":"alice", "amount":3}))
                .await;
            harness
                .patch("d", json!({"owner":"alice", "amount":4}))
                .await;
            for _ in 0..2 {
                for (view, expected) in [
                    ("empty", vec!["d", "c", "b", "a"]),
                    ("filtered", vec!["d", "c"]),
                    ("sorted", vec!["d", "c"]),
                ] {
                    let mut socket = harness
                        .subscribe(json!({"view": format!("Thing/{view}"), "take":100}), true)
                        .await;
                    assert_eq!(snapshot_keys(&mut socket).await, expected);
                    socket.close(None).await.unwrap();
                }
            }
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn source_delete_and_recreation_clear_all_views_across_reconnect() {
            let harness = Harness::start_with_index(
                WebSocketDeliveryConfig::default(),
                EntityCache::new(),
                derived_index(),
            )
            .await;
            harness
                .patch("a", json!({"owner":"alice", "amount":1, "oldField":true}))
                .await;
            let mut list = harness
                .subscribe(json!({"view":"Thing/filtered"}), true)
                .await;
            assert_eq!(snapshot_keys(&mut list).await, ["a"]);
            let mut state = harness
                .subscribe(json!({"view":"Thing/state", "key":"a"}), true)
                .await;
            assert_eq!(snapshot_keys(&mut state).await, ["a"]);
            // Leaving a predicate is view eviction, not chain deletion.
            harness.patch("a", json!({"owner":"bob"})).await;
            assert_eq!(next_frame(&mut list).await["op"], "remove");
            harness.patch("a", json!({"owner":"alice"})).await;
            assert_eq!(next_frame(&mut list).await["op"], "upsert");
            let mut deletion = Mutation::delete("Thing", json!("a"));
            deletion.mark_account_position(AccountPosition::new(100, 1001));
            harness.mutate(deletion).await;
            assert_eq!(next_frame(&mut list).await["op"], "delete");
            // State patches may precede its deletion on the socket.
            loop {
                if next_frame(&mut state).await["op"] == "delete" {
                    break;
                }
            }
            for view in [
                "Thing/list",
                "Thing/empty",
                "Thing/filtered",
                "Thing/sorted",
                "Thing/state",
            ] {
                let mut socket = harness
                    .subscribe(json!({"view":view, "key":"a"}), true)
                    .await;
                assert!(snapshot_keys(&mut socket).await.is_empty(), "{view}");
                socket.close(None).await.unwrap();
            }
            // A whole resend or sparse change cannot recreate a deleted row.
            harness
                .patch("a", json!({"owner":"alice", "oldField":true}))
                .await;
            let mut stale = Mutation {
                export: "Thing".into(),
                key: json!("a"),
                patch: json!({"owner":"alice", "oldField":true}),
                append: vec![],
            };
            stale.mark_whole_entity();
            harness.mutate(stale).await;
            let mut socket = harness
                .subscribe(json!({"view":"Thing/filtered"}), true)
                .await;
            assert!(snapshot_keys(&mut socket).await.is_empty());
            socket.close(None).await.unwrap();
            let mut recreated = Mutation {
                export: "Thing".into(),
                key: json!("a"),
                patch: json!({"owner":"alice", "amount":9}),
                append: vec![],
            };
            recreated.mark_created();
            recreated.mark_account_position(AccountPosition::new(100, 1002));
            harness.mutate(recreated).await;
            for view in [
                "Thing/list",
                "Thing/empty",
                "Thing/filtered",
                "Thing/sorted",
                "Thing/state",
            ] {
                let mut socket = harness
                    .subscribe(json!({"view":view, "key":"a"}), true)
                    .await;
                let frame = next_frame(&mut socket).await;
                assert_eq!(frame["data"][0]["key"], "a");
                assert_eq!(frame["data"][0]["data"]["amount"], 9);
                assert!(
                    frame["data"][0]["data"].get("oldField").is_none(),
                    "{frame}"
                );
                socket.close(None).await.unwrap();
            }
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn resolver_offsets_do_not_suppress_later_instruction_updates() {
            let harness = Harness::start(WebSocketDeliveryConfig::default()).await;
            let mut created = Mutation {
                export: "Thing".into(),
                key: json!("a"),
                patch: json!({"value":"created"}),
                append: vec![],
            };
            created.mark_created();
            created.mark_account_position(AccountPosition::new(100, 9));
            harness
                .mutate_with_context(created, SlotContext::account(100, 9))
                .await;

            harness
                .mutate_with_context(
                    Mutation {
                        export: "Thing".into(),
                        key: json!("a"),
                        patch: json!({"value":"resolver", "resolved":true}),
                        append: vec![],
                    },
                    SlotContext::resolver(100, 1_u64 << 63),
                )
                .await;
            harness
                .mutate_with_context(
                    Mutation {
                        export: "Thing".into(),
                        key: json!("a"),
                        patch: json!({"value":"instruction", "instruction":true}),
                        append: vec![],
                    },
                    SlotContext::instruction(100, 900),
                )
                .await;

            let mut socket = harness
                .subscribe(json!({"view":"Thing/state", "key":"a"}), true)
                .await;
            let frame = next_frame(&mut socket).await;
            assert_eq!(frame["op"], "snapshot", "{frame}");
            let row = &frame["data"][0]["data"];
            assert_eq!(row["value"], "instruction", "{frame}");
            assert_eq!(row["resolved"], true, "{frame}");
            assert_eq!(row["instruction"], true, "{frame}");

            // Recency still applies inside the instruction domain even after
            // activity from another source domain.
            harness
                .mutate_with_context(
                    Mutation {
                        export: "Thing".into(),
                        key: json!("a"),
                        patch: json!({"value":"stale", "stale":true}),
                        append: vec![],
                    },
                    SlotContext::instruction(100, 899),
                )
                .await;
            let mut reconnected = harness
                .subscribe(json!({"view":"Thing/state", "key":"a"}), true)
                .await;
            let frame = next_frame(&mut reconnected).await;
            let row = &frame["data"][0]["data"];
            assert_eq!(row["value"], "instruction", "{frame}");
            assert!(row.get("stale").is_none(), "{frame}");
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
        async fn projected_delete_and_recreation_between_reads_replace_state() {
            let harness = Harness::start(WebSocketDeliveryConfig::default()).await;
            harness.patch("a", json!({"old":true})).await;
            let mut socket = harness
                .subscribe(json!({"view":"Thing/state", "key":"a"}), true)
                .await;
            assert_eq!(snapshot_keys(&mut socket).await, ["a"]);
            let mut creation = Mutation {
                export: "Thing".into(),
                key: json!("a"),
                patch: json!({"fresh":true}),
                append: vec![],
            };
            creation.mark_created();
            // Both ready batches run before the watch receiver on this worker.
            for mutation in [Mutation::delete("Thing", json!("a")), creation] {
                let slot = harness
                    .slot
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                harness
                    .tx
                    .try_send(MutationBatch::with_slot_context(
                        vec![mutation].into_iter().collect(),
                        SlotContext::new(slot, 0),
                    ))
                    .unwrap();
            }
            let (ack, wait) = oneshot::channel();
            harness
                .tx
                .try_send(MutationBatch::flush_marker(ack))
                .unwrap();
            wait.await.unwrap();
            let frame = next_frame(&mut socket).await;
            assert_eq!(frame["op"], "upsert", "{frame}");
            assert_eq!(frame["data"]["fresh"], true);
            assert!(frame["data"].get("old").is_none(), "{frame}");
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn a_change_to_a_key_cut_by_snapshot_limit_is_an_upsert() {
            let harness = Harness::start(WebSocketDeliveryConfig::default()).await;
            seed(&harness).await;

            let mut socket = harness
                .subscribe(json!({"view": "Thing/list", "snapshotLimit": 1}), true)
                .await;
            // Newest first: only "c" was sent.
            assert_eq!(snapshot_keys(&mut socket).await, ["c"]);

            harness.patch("a", json!({"count": 2})).await;
            let frame = next_frame(&mut socket).await;
            assert_eq!(frame["key"], "a", "unexpected frame: {frame}");
            assert_eq!(frame["op"], "upsert", "a key never sent arrives whole");
            assert_eq!(frame["data"]["name"], "thing-a");
            assert_eq!(frame["data"]["count"], 2);

            // Now the client holds "a", so its next change is a patch again.
            harness.patch("a", json!({"count": 3})).await;
            let frame = next_frame(&mut socket).await;
            assert_eq!(frame["key"], "a");
            assert_eq!(frame["op"], "patch", "unexpected frame: {frame}");
            assert!(frame["data"].get("name").is_none());

            // A key the snapshot did carry still rides its patch.
            harness.patch("c", json!({"count": 2})).await;
            let frame = next_frame(&mut socket).await;
            assert_eq!(frame["key"], "c");
            assert_eq!(frame["op"], "patch", "unexpected frame: {frame}");
            socket.close(None).await.ok();
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn a_list_without_a_snapshot_upserts_each_key_on_its_first_change() {
            let harness = Harness::start(WebSocketDeliveryConfig::default()).await;
            seed(&harness).await;

            let mut socket = harness
                .subscribe(json!({"view": "Thing/list"}), false)
                .await;
            harness.patch("b", json!({"count": 2})).await;
            let frame = next_frame(&mut socket).await;
            assert_eq!(frame["key"], "b");
            assert_eq!(frame["op"], "upsert", "unexpected frame: {frame}");
            assert_eq!(frame["data"]["name"], "thing-b");

            harness.patch("b", json!({"count": 3})).await;
            let frame = next_frame(&mut socket).await;
            assert_eq!(frame["key"], "b");
            assert_eq!(frame["op"], "patch", "unexpected frame: {frame}");
            socket.close(None).await.ok();
        }

        /// Eviction from the server's bounded cache is not a change to the
        /// entity: a client holding it keeps getting its patches, never a
        /// `remove`, and the whole entity replaces its copy when it returns.
        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn a_state_holder_keeps_its_patches_when_the_cache_evicts_the_key() {
            let harness = Harness::start_with_cache(
                WebSocketDeliveryConfig::default(),
                EntityCache::with_config(crate::cache::EntityCacheConfig {
                    max_entities_per_view: 2,
                    ..Default::default()
                }),
            )
            .await;
            harness
                .patch("a", json!({"name": "thing-a", "count": 1}))
                .await;
            let mut socket = harness
                .subscribe(json!({"view": "Thing/state", "key": "a"}), true)
                .await;
            assert_eq!(snapshot_keys(&mut socket).await, ["a"]);

            // "b" and "c" push "a" out of the cache.
            for key in ["b", "c"] {
                harness
                    .patch(key, json!({"name": format!("thing-{key}"), "count": 1}))
                    .await;
            }
            harness.patch("a", json!({"count": 2})).await;
            let frame = next_frame(&mut socket).await;
            assert_eq!(frame["key"], "a", "unexpected frame: {frame}");
            assert_eq!(frame["op"], "patch", "the holder merges it: {frame}");
            assert_eq!(frame["data"]["count"], 2);

            // The whole entity comes back from the VM.
            let mut whole = Mutation {
                export: "Thing".to_string(),
                key: json!("a"),
                patch: json!({"name": "thing-a", "count": 2}),
                append: vec![],
            };
            whole.mark_whole_entity();
            harness.mutate(whole).await;
            let frame = next_frame(&mut socket).await;
            assert_eq!(frame["op"], "upsert", "unexpected frame: {frame}");
            assert_eq!(frame["data"]["name"], "thing-a");
            assert_eq!(frame["data"]["count"], 2);
            assert!(frame["data"]
                .get(arete_interpreter::WHOLE_ENTITY_MARKER)
                .is_none());

            harness.patch("a", json!({"count": 3})).await;
            let frame = next_frame(&mut socket).await;
            assert_eq!(frame["op"], "patch", "unexpected frame: {frame}");
            socket.close(None).await.ok();
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn a_state_view_without_a_snapshot_upserts_on_the_first_change() {
            let harness = Harness::start(WebSocketDeliveryConfig::default()).await;
            seed(&harness).await;

            let mut socket = harness
                .subscribe(json!({"view": "Thing/state", "key": "a"}), false)
                .await;
            harness.patch("a", json!({"count": 2})).await;
            let frame = next_frame(&mut socket).await;
            assert_eq!(frame["key"], "a");
            assert_eq!(frame["op"], "upsert", "unexpected frame: {frame}");
            assert_eq!(frame["data"]["name"], "thing-a");
            assert_eq!(frame["data"]["count"], 2);

            harness.patch("a", json!({"count": 3})).await;
            let frame = next_frame(&mut socket).await;
            assert_eq!(frame["op"], "patch", "unexpected frame: {frame}");
            socket.close(None).await.ok();
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn a_coalesced_list_sends_no_remove_for_a_key_it_never_sent() {
            let harness = Harness::start(WebSocketDeliveryConfig {
                collection_coalesce_ms: Some(10),
                ..Default::default()
            })
            .await;
            seed(&harness).await;

            // A one-row window holding "c", which the disabled snapshot never
            // sent.
            let mut socket = harness
                .subscribe(json!({"view": "Thing/list", "take": 1}), false)
                .await;
            // "a" jumps to the front: it enters the window whole and "c",
            // never sent, leaves it without a `remove`.
            harness.patch("a", json!({"count": 2})).await;
            let frame = next_frame(&mut socket).await;
            assert_eq!(frame["key"], "a", "unexpected frame: {frame}");
            assert_eq!(frame["op"], "upsert");
            assert_eq!(frame["data"]["name"], "thing-a");

            // "c" re-enters whole; "a", held, leaves with a `remove`.
            harness.patch("c", json!({"count": 2})).await;
            let mut frames = [next_frame(&mut socket).await, next_frame(&mut socket).await];
            frames.sort_by_key(|frame| frame["key"].as_str().unwrap().to_string());
            assert_eq!(frames[0]["key"], "a");
            assert_eq!(frames[0]["op"], "remove");
            assert_eq!(frames[1]["key"], "c");
            assert_eq!(frames[1]["op"], "upsert");
            assert_eq!(frames[1]["data"]["name"], "thing-c");
            socket.close(None).await.ok();
        }

        /// `(epoch, counter)` from an entity's `_version`.
        fn version_of(data: &Value) -> (String, u64) {
            let version = data["_version"]
                .as_str()
                .unwrap_or_else(|| panic!("no _version in {data}"));
            let (epoch, counter) = version.split_once(':').expect("epoch:counter");
            (epoch.to_string(), counter.parse().expect("decimal counter"))
        }

        /// Every entity a subscriber receives carries the projector's
        /// `_version`: a snapshot row, a forwarded patch, and a state
        /// subscription's catch-up, which sends the cached entity after the
        /// bus overwrote a patch.
        // One worker, and a projector the cooperative budget cannot interrupt:
        // within a batch it awaits only locks no other task holds, so it
        // publishes both changes before the subscriber can run, and the
        // second overwrites the first on the state bus.
        #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
        async fn snapshot_rows_patches_and_catch_ups_carry_the_version() {
            let harness = Harness::start(WebSocketDeliveryConfig::default()).await;
            harness.patch("7", json!({"name": "thing-7"})).await;

            let mut list = harness.subscribe(json!({"view": "Thing/list"}), true).await;
            let snapshot = next_frame(&mut list).await;
            assert_eq!(snapshot["op"], "snapshot", "unexpected frame: {snapshot}");
            let (epoch, _) = version_of(&snapshot["data"][0]["data"]);

            let mut state = harness
                .subscribe(json!({"view": "Thing/state", "key": "7"}), true)
                .await;
            let snapshot = next_frame(&mut state).await;
            assert_eq!(snapshot["op"], "snapshot", "unexpected frame: {snapshot}");
            let (state_epoch, seeded) = version_of(&snapshot["data"][0]["data"]);
            assert_eq!(state_epoch, epoch, "one projector, one epoch");

            harness.patch("7", json!({"count": 1})).await;
            let patch = next_frame(&mut list).await;
            assert_eq!(patch["op"], "patch", "unexpected frame: {patch}");
            assert_eq!(version_of(&patch["data"]).0, epoch);
            let patch = next_frame(&mut state).await;
            assert_eq!(patch["op"], "patch", "unexpected frame: {patch}");
            let (_, patched) = version_of(&patch["data"]);
            assert!(patched > seeded, "{patched} after {seeded}");

            // Two changes to the key in one batch.
            let slot = harness
                .slot
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let mutation = |patch: Value| Mutation {
                export: "Thing".to_string(),
                key: json!("7"),
                patch,
                append: vec![],
            };
            harness
                .tx
                .send(MutationBatch::with_slot_context(
                    vec![
                        mutation(json!({"count": 2})),
                        mutation(json!({"flag": true})),
                    ]
                    .into_iter()
                    .collect(),
                    SlotContext::new(slot, 0),
                ))
                .await
                .unwrap();

            let catch_up = next_frame(&mut state).await;
            assert_eq!(catch_up["op"], "patch", "unexpected frame: {catch_up}");
            assert!(catch_up.get("seq").is_none(), "a catch-up: {catch_up}");
            assert_eq!(catch_up["data"]["count"], 2);
            assert_eq!(catch_up["data"]["flag"], true);
            let (_, caught_up) = version_of(&catch_up["data"]);
            assert!(caught_up > patched, "{caught_up} after {patched}");

            list.close(None).await.ok();
            state.close(None).await.ok();
        }
    }

    /// Session tokens that expire while their socket is open.
    mod session_expiry {
        use super::*;
        use crate::websocket::auth::SignedSessionAuthPlugin;
        use arete_auth::{KeyClass, SessionClaims, SigningKey, TokenSigner, TokenVerifier};
        use futures_util::{SinkExt, StreamExt};
        use std::time::Duration;
        use tokio::net::{TcpListener, TcpStream};
        use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
        use tokio_tungstenite::tungstenite::protocol::CloseFrame;
        use tokio_tungstenite::tungstenite::Message;
        use tokio_tungstenite::{client_async, WebSocketStream};

        #[derive(Default)]
        struct RecordingUsageEmitter {
            events: tokio::sync::Mutex<Vec<WebSocketUsageEvent>>,
        }

        #[async_trait::async_trait]
        impl WebSocketUsageEmitter for RecordingUsageEmitter {
            async fn emit(&self, event: WebSocketUsageEvent) {
                self.events.lock().await.push(event);
            }
        }

        struct Server {
            addr: SocketAddr,
            signer: TokenSigner,
            usage: Arc<RecordingUsageEmitter>,
            metrics: Arc<DeliveryProbe>,
        }

        impl Server {
            async fn start() -> Self {
                let signing_key = SigningKey::generate();
                let verifier =
                    TokenVerifier::new(signing_key.verifying_key(), "test-issuer", "test-audience");
                let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
                let addr = listener.local_addr().unwrap();
                let usage = Arc::new(RecordingUsageEmitter::default());
                let server = WebSocketServer::new(
                    addr,
                    BusManager::new(),
                    EntityCache::new(),
                    Arc::new(ViewIndex::new()),
                    #[cfg(feature = "otel")]
                    None,
                )
                .with_auth_plugin(Arc::new(SignedSessionAuthPlugin::new(verifier)))
                .with_usage_emitter(usage.clone());
                let metrics = Arc::new(DeliveryProbe::default());
                let (acceptor, _cleanup) = server.into_acceptor();
                let acceptor = acceptor.with_delivery_probe(metrics.clone());
                tokio::spawn(async move { acceptor.serve_listener(listener).await });
                Self {
                    addr,
                    signer: TokenSigner::new(signing_key, "test-issuer"),
                    usage,
                    metrics,
                }
            }

            fn token(&self, ttl_seconds: u64) -> String {
                let claims = SessionClaims::builder("test-issuer", "test-subject", "test-audience")
                    .with_scope("read")
                    .with_key_class(KeyClass::Secret)
                    .with_ttl(ttl_seconds)
                    .build();
                self.signer.sign(claims).unwrap()
            }

            fn account_token(&self, ttl_seconds: u64, account: &str) -> String {
                let claims = SessionClaims::builder("test-issuer", account, "test-audience")
                    .with_scope("read")
                    .with_key_class(KeyClass::Secret)
                    .with_metering_key(account)
                    .with_plan("agent_trial")
                    .with_actor_key(account)
                    .with_account_key(account)
                    .with_consumer_key(format!("consumer:{account}"))
                    .with_policy_version(3)
                    .with_account_limits(Default::default())
                    .with_ttl(ttl_seconds)
                    .build();
                self.signer.sign(claims).unwrap()
            }

            async fn connect(&self, token: &str) -> WebSocketStream<TcpStream> {
                let stream = TcpStream::connect(self.addr).await.unwrap();
                client_async(format!("ws://{}/?hs_token={token}", self.addr), stream)
                    .await
                    .unwrap()
                    .0
            }
        }

        async fn send_json(socket: &mut WebSocketStream<TcpStream>, message: Value) {
            socket
                .send(Message::Text(message.to_string().into()))
                .await
                .unwrap();
        }

        /// The close frame, if the server closes the socket within `wait`.
        async fn close_within(
            socket: &mut WebSocketStream<TcpStream>,
            wait: Duration,
        ) -> Option<Option<CloseFrame>> {
            tokio::time::timeout(wait, async {
                while let Some(Ok(message)) = socket.next().await {
                    if let Message::Close(frame) = message {
                        return Some(frame);
                    }
                }
                Some(None)
            })
            .await
            .ok()
            .flatten()
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn an_expired_session_is_closed_with_the_reason() {
            let server = Server::start().await;
            let mut socket = server.connect(&server.token(2)).await;

            tokio::time::sleep(Duration::from_secs(3)).await;
            send_json(&mut socket, json!({"type": "ping"})).await;

            let frame = close_within(&mut socket, Duration::from_secs(5))
                .await
                .expect("the server closes the expired session")
                .expect("the close frame carries a reason");
            assert_eq!(frame.code, CloseCode::Policy);
            assert_eq!(
                frame.reason.as_str(),
                "token-expired: Authentication token expired"
            );
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn a_session_refreshed_in_band_outlives_its_first_token() {
            let server = Server::start().await;
            let mut socket = server.connect(&server.token(2)).await;

            send_json(
                &mut socket,
                json!({"type": "refresh_auth", "token": server.token(3_600)}),
            )
            .await;
            let reply = tokio::time::timeout(Duration::from_secs(5), async {
                while let Some(Ok(message)) = socket.next().await {
                    if let Message::Text(text) = message {
                        return serde_json::from_str::<Value>(text.as_str()).ok();
                    }
                }
                None
            })
            .await
            .expect("the server answers the refresh")
            .expect("the answer is JSON");
            assert_eq!(reply["success"], true, "refresh accepted: {reply}");

            tokio::time::sleep(Duration::from_secs(3)).await;
            send_json(&mut socket, json!({"type": "ping"})).await;
            assert!(
                close_within(&mut socket, Duration::from_millis(1_500))
                    .await
                    .is_none(),
                "the socket stays open on the refreshed token"
            );
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn usage_after_auth_refresh_uses_the_refreshed_account() {
            let server = Server::start().await;
            let mut socket = server
                .connect(&server.account_token(3_600, "account:1"))
                .await;

            send_json(
                &mut socket,
                json!({
                    "type": "refresh_auth",
                    "token": server.account_token(3_600, "account:2")
                }),
            )
            .await;
            let reply = tokio::time::timeout(Duration::from_secs(5), async {
                while let Some(Ok(message)) = socket.next().await {
                    if let Message::Text(text) = message {
                        return serde_json::from_str::<Value>(text.as_str()).ok();
                    }
                }
                None
            })
            .await
            .expect("the server answers the refresh")
            .expect("the answer is JSON");
            assert_eq!(reply["success"], true, "refresh accepted: {reply}");
            socket.close(None).await.unwrap();

            let account = tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    let events = server.usage.events.lock().await;
                    if let Some(account) = events.iter().find_map(|event| match event {
                        WebSocketUsageEvent::ConnectionClosed { identity, .. } => {
                            identity.account_key.clone()
                        }
                        _ => None,
                    }) {
                        return account;
                    }
                    drop(events);
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("connection close usage event");
            assert_eq!(account, "account:2");
            let active = server
                .metrics
                .active_connections
                .lock()
                .expect("delivery probe lock poisoned");
            assert_eq!(active.get("account:1"), Some(&0));
            assert_eq!(active.get("account:2"), None);
        }
    }
}
