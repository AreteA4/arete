use crate::bus::{BusManager, BusMessage};
use crate::cache::{cmp_seq, EntityCache, SnapshotBatchConfig};
use crate::compression::maybe_compress;
use crate::view::{ViewIndex, ViewSpec};
use crate::websocket::auth::{
    AuthContext, AuthDecision, AuthDeny, ConnectionAuthRequest, WebSocketAuthPlugin,
};
use crate::websocket::client_manager::{ClientManager, RateLimitConfig};
use crate::websocket::frame::{
    apply_wire_format, Frame, Mode, SnapshotEntity, SnapshotFrame, SortConfig, SortOrder,
    SubscribedFrame, UnsubscribedFrame,
};
use crate::websocket::subscription::{
    ClientMessage, RefreshAuthRequest, RefreshAuthResponse, SocketIssueMessage, Subscription,
    SubscriptionQuery, Unsubscription, PROTOCOL_VERSION,
};
use crate::websocket::usage::{WebSocketUsageEmitter, WebSocketUsageEvent};
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
            if client_manager.update_client_auth(client_id, new_context) {
                RefreshAuthResponse {
                    success: true,
                    error: None,
                    expires_at: Some(expires_at),
                }
            } else {
                RefreshAuthResponse {
                    success: false,
                    error: Some("client-not-found".to_string()),
                    expires_at: None,
                }
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

fn emit_usage_event(
    usage_emitter: &Option<Arc<dyn WebSocketUsageEmitter>>,
    event: WebSocketUsageEvent,
) {
    if let Some(emitter) = usage_emitter.clone() {
        tokio::spawn(async move {
            emitter.emit(event).await;
        });
    }
}

fn usage_identity(
    auth_context: Option<&AuthContext>,
) -> (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
) {
    match auth_context {
        Some(context) => (
            Some(context.metering_key.clone()),
            Some(context.subject.clone()),
            Some(key_class_label(context.key_class).to_string()),
            context.deployment_id.clone(),
        ),
        None => (None, None, None, None),
    }
}

fn emit_update_sent_for_client(
    usage_emitter: &Option<Arc<dyn WebSocketUsageEmitter>>,
    client_manager: &ClientManager,
    client_id: Uuid,
    view_id: &str,
    bytes: usize,
) {
    let auth_context = client_manager.get_auth_context(client_id);
    let (metering_key, subject, _, deployment_id) = usage_identity(auth_context.as_ref());
    emit_usage_event(
        usage_emitter,
        WebSocketUsageEvent::UpdateSent {
            client_id: client_id.to_string(),
            deployment_id,
            metering_key,
            subject,
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
    usage_emitter: Option<Arc<dyn WebSocketUsageEmitter>>,
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
    usage_emitter: Option<Arc<dyn WebSocketUsageEmitter>>,
    rate_limit_config: Option<RateLimitConfig>,
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
        self.usage_emitter = Some(usage_emitter);
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
        let client_manager = self
            .rate_limit_config
            .map(ClientManager::with_config)
            .unwrap_or(self.client_manager);
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
    usage_emitter: Option<Arc<dyn WebSocketUsageEmitter>>,
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
) -> Result<Option<(tokio_tungstenite::WebSocketStream<TcpStream>, AuthContext)>> {
    use std::sync::Mutex;

    let capture: Arc<Mutex<Option<Result<AuthContext, HandshakeReject>>>> =
        Arc::new(Mutex::new(None));
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
                        .map(|()| context)
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
            Some(Ok(context)) => Ok(Some((stream, context))),
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
    let Some((ws_stream, auth_context)) = accepted else {
        return Ok(());
    };

    let client_id = Uuid::new_v4();
    context.client_id = client_id;
    let connection_start = Instant::now();
    let (metering_key, subject, key_class, deployment_id) = usage_identity(Some(&auth_context));
    context.metrics.connection_opened(metering_key.as_deref());

    let (ws_sender, mut ws_receiver) = ws_stream.split();
    context
        .client_manager
        .add_client(client_id, ws_sender, Some(auth_context), remote_addr);
    emit_usage_event(
        &context.usage_emitter,
        WebSocketUsageEvent::ConnectionEstablished {
            client_id: client_id.to_string(),
            remote_addr: remote_addr.to_string(),
            deployment_id: deployment_id.clone(),
            metering_key: metering_key.clone(),
            subject: subject.clone(),
            key_class,
        },
    );

    let mut active_subscriptions: HashMap<String, String> = HashMap::new();
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
                if !context
                    .client_manager
                    .add_client_subscription(
                        client_id,
                        subscription_id.clone(),
                        cancel_token.clone(),
                    )
                    .await
                {
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

                active_subscriptions.insert(subscription_id, view.clone());
                context
                    .metrics
                    .subscription_created(&view, metering_key.as_deref());
                emit_usage_event(
                    &context.usage_emitter,
                    WebSocketUsageEvent::SubscriptionCreated {
                        client_id: client_id.to_string(),
                        deployment_id: deployment_id.clone(),
                        metering_key: metering_key.clone(),
                        subject: subject.clone(),
                        view_id: view,
                    },
                );
            }
            ClientMessage::Unsubscribe(unsubscription) => {
                handle_unsubscribe(
                    &context,
                    unsubscription,
                    &mut active_subscriptions,
                    metering_key.as_deref(),
                    &deployment_id,
                    &metering_key,
                    &subject,
                )
                .await;
            }
            ClientMessage::Ping => debug!("Received ping from client {}", client_id),
            ClientMessage::RefreshAuth(request) => {
                handle_refresh_auth(client_id, &request, &context.client_manager, &auth_plugin)
                    .await;
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
    for view in active_subscriptions.values() {
        context
            .metrics
            .subscription_removed(view, metering_key.as_deref());
        emit_usage_event(
            &context.usage_emitter,
            WebSocketUsageEvent::SubscriptionRemoved {
                client_id: client_id.to_string(),
                deployment_id: deployment_id.clone(),
                metering_key: metering_key.clone(),
                subject: subject.clone(),
                view_id: view.clone(),
            },
        );
    }
    let duration = connection_start.elapsed().as_secs_f64();
    context
        .metrics
        .connection_closed(duration, metering_key.as_deref());
    emit_usage_event(
        &context.usage_emitter,
        WebSocketUsageEvent::ConnectionClosed {
            client_id: client_id.to_string(),
            deployment_id,
            metering_key,
            subject,
            duration_secs: Some(duration),
            subscription_count: u32::try_from(active_subscriptions.len()).unwrap_or(u32::MAX),
        },
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn handle_unsubscribe(
    context: &SubscriptionContext,
    unsubscription: Unsubscription,
    active_subscriptions: &mut HashMap<String, String>,
    metrics_metering_key: Option<&str>,
    deployment_id: &Option<String>,
    usage_metering_key: &Option<String>,
    subject: &Option<String>,
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

    let Some(view) = active_subscriptions.remove(&subscription_id) else {
        return;
    };
    let _ = send_control_frame(context, &UnsubscribedFrame::new(subscription_id), &view);
    context
        .metrics
        .subscription_removed(&view, metrics_metering_key);
    emit_usage_event(
        &context.usage_emitter,
        WebSocketUsageEvent::SubscriptionRemoved {
            client_id: context.client_id.to_string(),
            deployment_id: deployment_id.clone(),
            metering_key: usage_metering_key.clone(),
            subject: subject.clone(),
            view_id: view,
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

fn create_snapshot_batches(
    entities: &[SnapshotEntity],
    metadata: SnapshotMetadata<'_>,
    batch_config: &SnapshotBatchConfig,
) -> Vec<SnapshotFrame> {
    if entities.is_empty() {
        return vec![SnapshotFrame {
            protocol_version: PROTOCOL_VERSION,
            subscription_id: metadata.subscription_id.to_string(),
            snapshot_id: metadata.snapshot_id.to_string(),
            authoritative: metadata.authoritative,
            mode: metadata.mode,
            export: metadata.view_id.to_string(),
            op: "snapshot",
            key: metadata.key.map(str::to_string),
            data: vec![],
            complete: true,
        }];
    }

    let mut batches = Vec::new();
    let mut offset = 0;
    while offset < entities.len() {
        let configured_size = if offset == 0 {
            batch_config.initial_batch_size
        } else {
            batch_config.subsequent_batch_size
        };
        let end = (offset + configured_size.max(1)).min(entities.len());
        batches.push(SnapshotFrame {
            protocol_version: PROTOCOL_VERSION,
            subscription_id: metadata.subscription_id.to_string(),
            snapshot_id: metadata.snapshot_id.to_string(),
            authoritative: metadata.authoritative,
            mode: metadata.mode,
            export: metadata.view_id.to_string(),
            op: "snapshot",
            key: metadata.key.map(str::to_string),
            data: entities[offset..end].to_vec(),
            complete: end == entities.len(),
        });
        offset = end;
    }
    batches
}

async fn send_snapshot_batches(
    context: &SubscriptionContext,
    subscription: &Subscription,
    entities: &[SnapshotEntity],
    mode: Mode,
    purpose: SnapshotPurpose,
    batch_config: &SnapshotBatchConfig,
) -> Result<()> {
    let snapshot_id = Uuid::new_v4().to_string();
    let authoritative = purpose.authoritative(subscription);
    let frames = create_snapshot_batches(
        entities,
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
        let rows = frame.data.len() as u32;
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
        let (metering_key, subject, _, deployment_id) = usage_identity(auth_context.as_ref());
        emit_usage_event(
            &context.usage_emitter,
            WebSocketUsageEvent::SnapshotSent {
                client_id: context.client_id.to_string(),
                deployment_id,
                metering_key,
                subject,
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
) -> (watch::Receiver<Arc<Bytes>>, T)
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = T>,
{
    let mut receiver = bus_manager.get_or_create_state_bus(view_id, key).await;
    receiver.borrow_and_update();
    let snapshot = snapshot().await;
    (receiver, snapshot)
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
    let (mut receiver, initial) =
        subscribe_state_then_snapshot(&context.bus_manager, &view_id, &key, move || async move {
            load_query_entities(&cache, None, &view_spec_for_snapshot, &query, false).await
        })
        .await;

    let mut snapshot_entities = initial.clone();
    if let Some(limit) = subscription.query.snapshot_limit {
        snapshot_entities.truncate(limit);
    }
    enforce_snapshot_limit(context, snapshot_entities.len())?;
    send_subscribed_frame(context, &subscription, &view_spec)?;
    if subscription.snapshot.enabled {
        send_snapshot_batches(
            context,
            &subscription,
            &to_wire_snapshot_entities(snapshot_entities, &view_spec),
            view_spec.mode,
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
            let mut member = !initial.is_empty();
            loop {
                tokio::select! {
                    _ = cancel_token.cancelled() => break,
                    changed = receiver.changed() => {
                        if changed.is_err() {
                            break;
                        }
                        let payload = receiver.borrow().clone();
                        let metadata = source_frame_metadata(&payload);
                        if metadata.op == "delete" {
                            task_context.entity_cache.remove(&query.view, &key).await;
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
                            continue;
                        }

                        let selected = load_query_entities(
                            &task_context.entity_cache,
                            None,
                            &view_spec_task,
                            &query,
                            false,
                        ).await;
                        let is_member = !selected.is_empty();
                        let result = match (member, is_member) {
                            (true, true) => send_scoped_source_payload(
                                &task_context,
                                &subscription_id,
                                &query.view,
                                payload,
                            ),
                            (false, true) => {
                                let (entity_key, data) = selected.into_iter().next().unwrap();
                                send_membership_frame(
                                    &task_context,
                                    &subscription_id,
                                    &view_spec_task,
                                    "upsert",
                                    &entity_key,
                                    data,
                                    metadata.seq,
                                )
                            }
                            (true, false) => send_membership_frame(
                                &task_context,
                                &subscription_id,
                                &view_spec_task,
                                "remove",
                                &key,
                                Value::Null,
                                metadata.seq,
                            ),
                            (false, false) => Ok(()),
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
    if subscription.snapshot.enabled {
        send_snapshot_batches(
            context,
            &subscription,
            &to_wire_snapshot_entities(snapshot_entities, &view_spec),
            view_spec.mode,
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
) -> (broadcast::Receiver<Arc<BusMessage>>, Vec<(String, Value)>) {
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
) -> Result<(broadcast::Receiver<Arc<BusMessage>>, Vec<(String, Value)>)> {
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
        &to_wire_snapshot_entities(membership.clone(), view_spec),
        view_spec.mode,
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
    // A slow subscription can observe an old delete after the projector has
    // already recreated the key. Never let subscriber-local lag erase newer
    // shared cache state.
    let current = context
        .entity_cache
        .get(source_view_id, &envelope.key)
        .await;
    if source_delete_is_stale(current.as_ref(), metadata.seq.as_deref()) {
        return;
    }
    context
        .entity_cache
        .remove(source_view_id, &envelope.key)
        .await;
    if view_spec.is_derived() {
        let caches = context.view_index.sorted_caches();
        let mut guard = caches.write().await;
        if let Some(cache) = guard.get_mut(&query.view) {
            cache.remove(&envelope.key);
        }
    }
}

fn source_delete_is_stale(current: Option<&Value>, delete_seq: Option<&str>) -> bool {
    match (current, delete_seq) {
        (Some(current), Some(delete_seq)) => current
            .get("_seq")
            .and_then(Value::as_str)
            .is_some_and(|current_seq| {
                cmp_seq(current_seq, delete_seq) == std::cmp::Ordering::Greater
            }),
        _ => false,
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
    let Ok(frame) = serde_json::from_slice::<Value>(payload) else {
        return false;
    };
    let Some(data) = frame.get("data") else {
        return false;
    };
    if let Some(partition) = &query.partition {
        if value_at_dot_path(data, "_partition") != Some(&Value::String(partition.clone())) {
            return false;
        }
    }
    query
        .filters
        .iter()
        .all(|(path, expected)| value_at_dot_path(data, path) == Some(expected))
}

/// Awaiting variant of [`send_scoped_source_payload`], for replays that can
/// exceed the client's send queue.
async fn send_scoped_source_payload_async(
    context: &SubscriptionContext,
    subscription_id: &str,
    view_id: &str,
    payload: Arc<Bytes>,
) -> Result<()> {
    let mut value: Value = serde_json::from_slice(&payload)?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("source frame is not an object"))?;
    object.insert("protocolVersion".to_string(), Value::from(PROTOCOL_VERSION));
    object.insert(
        "subscriptionId".to_string(),
        Value::String(subscription_id.to_string()),
    );
    let json = serde_json::to_vec(&value)?;
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

fn source_frame_metadata(payload: &[u8]) -> SourceFrameMetadata {
    serde_json::from_slice::<Value>(payload)
        .ok()
        .map(|value| SourceFrameMetadata {
            op: value
                .get("op")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            seq: value.get("seq").and_then(Value::as_str).map(str::to_string),
            offset: value.get("offset").and_then(Value::as_u64),
        })
        .unwrap_or_default()
}

fn send_scoped_source_payload(
    context: &SubscriptionContext,
    subscription_id: &str,
    view_id: &str,
    payload: Arc<Bytes>,
) -> Result<()> {
    let mut value: Value = serde_json::from_slice(&payload)?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("source frame is not an object"))?;
    object.insert("protocolVersion".to_string(), Value::from(PROTOCOL_VERSION));
    object.insert(
        "subscriptionId".to_string(),
        Value::String(subscription_id.to_string()),
    );
    let encoded = Arc::new(Bytes::from(serde_json::to_vec(&value)?));
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
    let encoded = Arc::new(Bytes::from(serde_json::to_vec(&frame)?));
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

fn emit_collection_delta(
    context: &SubscriptionContext,
    subscription_id: &str,
    view_spec: &ViewSpec,
    current: &[(String, Value)],
    next: &[(String, Value)],
    envelope: &BusMessage,
    metadata: &SourceFrameMetadata,
) -> Result<()> {
    let current_keys: Vec<&str> = current.iter().map(|(key, _)| key.as_str()).collect();
    let next_keys: Vec<&str> = next.iter().map(|(key, _)| key.as_str()).collect();
    let next_set: HashSet<&str> = next_keys.iter().copied().collect();

    for key in current_keys
        .iter()
        .copied()
        .filter(|key| !next_set.contains(key))
    {
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
        let was_member = current_keys.iter().any(|candidate| *candidate == key);
        match member_action(
            was_member,
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
                    .clone()
                    .or_else(|| data.get("_seq").and_then(Value::as_str).map(str::to_string));
                send_membership_frame(
                    context,
                    subscription_id,
                    view_spec,
                    "upsert",
                    key,
                    data.clone(),
                    seq,
                )?;
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
    current: &[(String, Value)],
    next: &[(String, Value)],
    pending: &HashMap<String, Arc<BusMessage>>,
) -> Result<()> {
    for change in plan_coalesced_collection_delta(current, next, pending) {
        send_membership_frame(
            context,
            subscription_id,
            view_spec,
            change.op,
            &change.key,
            change.data,
            change.seq,
        )?;
    }
    Ok(())
}

#[derive(Debug, PartialEq)]
struct CollectionChange {
    op: &'static str,
    key: String,
    data: Value,
    seq: Option<String>,
}

fn plan_coalesced_collection_delta(
    current: &[(String, Value)],
    next: &[(String, Value)],
    pending: &HashMap<String, Arc<BusMessage>>,
) -> Vec<CollectionChange> {
    let current_by_key: HashMap<&str, &Value> = current
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
        .filter(|(key, _)| !next_keys.contains(key.as_str()))
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
            data: Value::Null,
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
            .get("_seq")
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
            data: data.clone(),
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
/// Only the mutated key's data changed, so a key that was already a member has
/// at most moved index — and index is not the server's to communicate.
/// `subscribed` announces the window's sort (see [`extract_sort_config`], which
/// is `_seq` descending for a plain list) and every SDK re-sorts locally from
/// it, so resending an unchanged entity to convey its new position is pure
/// waste. This matters because on a `_seq`-ordered list the mutated entity
/// jumps to the front on *every* mutation: keying the decision on position
/// change meant rebroadcasting the whole window each time, and the mutated
/// entity itself never kept its position long enough to have its patch
/// forwarded.
fn member_action(
    was_member: bool,
    is_mutated_key: bool,
    is_derived: bool,
    op: &str,
) -> MemberAction {
    if !is_mutated_key {
        // A key entering the window has no local state to merge into, so it
        // needs the whole entity; one already held is unchanged.
        return if was_member {
            MemberAction::Skip
        } else {
            MemberAction::Upsert
        };
    }
    // The mutated entity rides its own patch through untouched, but only when
    // the subscriber already holds a copy. Derived views still send whole
    // entities: the patch on the bus is scoped to the source view, not this
    // one (see A4-150).
    if was_member && !is_derived && op != "delete" {
        MemberAction::ForwardPatch
    } else {
        MemberAction::Upsert
    }
}

fn to_wire_snapshot_entities(
    entities: Vec<(String, Value)>,
    view_spec: &ViewSpec,
) -> Vec<SnapshotEntity> {
    entities
        .into_iter()
        .map(|(key, mut data)| {
            apply_wire_format(&mut data, &view_spec.wire_format);
            SnapshotEntity { key, data }
        })
        .collect()
}

async fn load_query_entities(
    entity_cache: &EntityCache,
    sorted_caches: Option<
        Arc<tokio::sync::RwLock<HashMap<String, crate::sorted_cache::SortedViewCache>>>,
    >,
    view_spec: &ViewSpec,
    query: &SubscriptionQuery,
    apply_snapshot_limit: bool,
) -> Vec<(String, Value)> {
    let (entities, preordered) = if let Some(sorted_caches) = sorted_caches {
        let mut caches = sorted_caches.write().await;
        let entities = caches
            .get_mut(&view_spec.id)
            .map(|cache| cache.get_all_ordered())
            .unwrap_or_default();
        (entities, true)
    } else if view_spec.mode == Mode::State {
        let entity = match query.key.as_deref() {
            Some(key) => entity_cache
                .get(&view_spec.id, key)
                .await
                .map(|data| vec![(key.to_string(), data)])
                .unwrap_or_default(),
            None => vec![],
        };
        (entity, true)
    } else {
        (entity_cache.get_all(&view_spec.id).await, false)
    };
    select_query_entities(entities, query, preordered, apply_snapshot_limit)
}

fn select_query_entities(
    mut entities: Vec<(String, Value)>,
    query: &SubscriptionQuery,
    preordered: bool,
    apply_snapshot_limit: bool,
) -> Vec<(String, Value)> {
    entities.retain(|(key, data)| query_matches_entity(query, key, data));
    if !preordered {
        entities.sort_by(|left, right| {
            let left_seq = left.1.get("_seq").and_then(Value::as_str).unwrap_or("");
            let right_seq = right.1.get("_seq").and_then(Value::as_str).unwrap_or("");
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

fn query_matches_entity(query: &SubscriptionQuery, key: &str, data: &Value) -> bool {
    if !query.matches_key(key) {
        return false;
    }
    if let Some(partition) = &query.partition {
        if value_at_dot_path(data, "_partition") != Some(&Value::String(partition.clone())) {
            return false;
        }
    }
    if let Some(after) = &query.after {
        let Some(seq) = data.get("_seq").and_then(Value::as_str) else {
            return false;
        };
        if cmp_seq(seq, after) != std::cmp::Ordering::Greater {
            return false;
        }
    }
    query
        .filters
        .iter()
        .all(|(path, expected)| value_at_dot_path(data, path) == Some(expected))
}

fn value_at_dot_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.')
        .try_fold(value, |current, segment| current.get(segment))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::EntityCacheConfig;
    use crate::view::{Delivery, Filters, Projection};
    use serde_json::json;
    use tokio::sync::oneshot;

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
        next_keys
            .iter()
            .map(|key| {
                let was_member = current_keys.contains(key);
                (
                    (*key).to_string(),
                    member_action(was_member, *key == envelope_key, is_derived, op),
                )
            })
            .collect()
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
            plan_coalesced_collection_delta(&current, &next, &pending),
            vec![
                CollectionChange {
                    op: "delete",
                    key: "b".to_string(),
                    data: Value::Null,
                    seq: Some("10:000002".to_string()),
                },
                CollectionChange {
                    op: "upsert",
                    key: "a".to_string(),
                    data: json!({"count": 3, "_seq": "10:000004"}),
                    seq: Some("10:000004".to_string()),
                },
                CollectionChange {
                    op: "upsert",
                    key: "c".to_string(),
                    data: json!({"count": 1, "_seq": "10:000003"}),
                    seq: Some("10:000003".to_string()),
                },
            ]
        );
    }

    #[test]
    fn a_delayed_delete_cannot_erase_a_newer_recreated_entity() {
        let recreated = json!({"balance": 2, "_seq": "10:000004"});
        assert!(source_delete_is_stale(Some(&recreated), Some("10:000002")));
        assert!(!source_delete_is_stale(Some(&recreated), Some("10:000004")));
        assert!(!source_delete_is_stale(Some(&recreated), None));
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
        let entities = ["one", "two", "three"].map(|key| SnapshotEntity {
            key: key.to_string(),
            data: json!({"key": key}),
        });
        let batches = create_snapshot_batches(
            &entities,
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
        let batches = create_snapshot_batches(
            &[],
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
        assert_eq!(batches[0].key.as_deref(), Some("missing"));
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
        assert!(!receiver.borrow().is_empty());
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
                    occurrence: None,
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

        struct Server {
            addr: SocketAddr,
            signer: TokenSigner,
        }

        impl Server {
            async fn start() -> Self {
                let signing_key = SigningKey::generate();
                let verifier =
                    TokenVerifier::new(signing_key.verifying_key(), "test-issuer", "test-audience");
                let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
                let addr = listener.local_addr().unwrap();
                let server = WebSocketServer::new(
                    addr,
                    BusManager::new(),
                    EntityCache::new(),
                    Arc::new(ViewIndex::new()),
                    #[cfg(feature = "otel")]
                    None,
                )
                .with_auth_plugin(Arc::new(SignedSessionAuthPlugin::new(verifier)));
                let (acceptor, _cleanup) = server.into_acceptor();
                tokio::spawn(async move { acceptor.serve_listener(listener).await });
                Self {
                    addr,
                    signer: TokenSigner::new(signing_key, "test-issuer"),
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
    }
}
