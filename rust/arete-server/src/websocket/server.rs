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
use anyhow::Result;
use bytes::Bytes;
use futures_util::StreamExt;
use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;
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
use tracing::{debug, error, info, info_span, warn, Instrument};
use uuid::Uuid;

#[cfg(feature = "otel")]
use crate::metrics::Metrics;

#[derive(Clone, Default)]
struct WsMetrics {
    #[cfg(feature = "otel")]
    inner: Option<Arc<Metrics>>,
}

impl WsMetrics {
    #[cfg(feature = "otel")]
    fn new(inner: Option<Arc<Metrics>>) -> Self {
        Self { inner }
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
    metrics: WsMetrics,
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

    pub fn with_rate_limit_config(mut self, config: RateLimitConfig) -> Self {
        self.rate_limit_config = Some(config);
        self
    }

    pub async fn start(self) -> Result<()> {
        info!(
            "Starting WebSocket server on {} (max_clients: {})",
            self.bind_addr, self.max_clients
        );
        let listener = TcpListener::bind(&self.bind_addr).await?;
        let client_manager = self
            .rate_limit_config
            .map(ClientManager::with_config)
            .unwrap_or(self.client_manager);
        client_manager.start_cleanup_task();

        #[cfg(feature = "otel")]
        let metrics = WsMetrics::new(self.metrics.clone());
        #[cfg(not(feature = "otel"))]
        let metrics = WsMetrics::default();

        loop {
            match listener.accept().await {
                Ok((stream, addr)) => {
                    if client_manager.client_count() >= self.max_clients {
                        warn!("Rejecting connection from {}: max clients reached", addr);
                        continue;
                    }

                    let context = SubscriptionContext {
                        client_id: Uuid::nil(),
                        client_manager: client_manager.clone(),
                        bus_manager: self.bus_manager.clone(),
                        entity_cache: self.entity_cache.clone(),
                        view_index: self.view_index.clone(),
                        usage_emitter: self.usage_emitter.clone(),
                        metrics: metrics.clone(),
                    };
                    let auth_plugin = self.auth_plugin.clone();
                    tokio::spawn(
                        async move {
                            if let Err(error) =
                                handle_connection(stream, context, addr, auth_plugin).await
                            {
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
    let Some((ws_stream, auth_context)) = accept_authorized_connection(
        stream,
        remote_addr,
        auth_plugin.clone(),
        context.client_manager.clone(),
    )
    .await?
    else {
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
    while let Some(message) = ws_receiver.next().await {
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
                    send_protocol_issue(
                        client_id,
                        &context.client_manager,
                        &context.metrics,
                        Some(subscription_id),
                        "subscription-rejected",
                        error.to_string(),
                    )
                    .await;
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
    batch_config: &SnapshotBatchConfig,
) -> Result<()> {
    let snapshot_id = Uuid::new_v4().to_string();
    let authoritative = subscription.query.after.is_none();
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
    let query = subscription.query.clone();
    let cache = context.entity_cache.clone();
    let sorted_caches = view_spec
        .is_derived()
        .then(|| context.view_index.sorted_caches());
    let view_spec_for_snapshot = view_spec.clone();
    let (mut receiver, initial_membership) =
        subscribe_list_then_snapshot(&context.bus_manager, &source_view_id, move || async move {
            load_query_entities(
                &cache,
                sorted_caches,
                &view_spec_for_snapshot,
                &query,
                false,
            )
            .await
        })
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
            &context.entity_cache.snapshot_config(),
        )
        .await?;
    }

    let task_context = context.clone();
    let subscription_id = subscription.subscription_id.clone();
    let query = subscription.query.clone();
    let view_spec_task = view_spec.clone();
    let span_view = view_id.clone();
    tokio::spawn(
        async move {
            let mut current = initial_membership;
            loop {
                tokio::select! {
                    _ = cancel_token.cancelled() => break,
                    received = receiver.recv() => {
                        let envelope = match received {
                            Ok(envelope) => envelope,
                            Err(broadcast::error::RecvError::Lagged(_)) => {
                                warn!("Subscription {} lagged; closing to preserve membership correctness", subscription_id);
                                break;
                            }
                            Err(broadcast::error::RecvError::Closed) => break,
                        };
                        let metadata = source_frame_metadata(&envelope.payload);
                        if metadata.op == "delete" {
                            task_context.entity_cache.remove(&source_view_id, &envelope.key).await;
                            if view_spec_task.is_derived() {
                                let caches = task_context.view_index.sorted_caches();
                                let mut guard = caches.write().await;
                                if let Some(cache) = guard.get_mut(&query.view) {
                                    cache.remove(&envelope.key);
                                }
                            }
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
                        if emit_collection_delta(
                            &task_context,
                            &subscription_id,
                            &view_spec_task,
                            &current,
                            &next,
                            &envelope,
                            &metadata,
                        ).is_err() {
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

#[derive(Default)]
struct SourceFrameMetadata {
    op: String,
    seq: Option<String>,
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

    for (position, (key, data)) in next.iter().enumerate() {
        let previous_position = current_keys.iter().position(|candidate| *candidate == key);
        let changed_position = previous_position != Some(position);
        if previous_position.is_none() || changed_position || key == &envelope.key {
            let can_forward_patch = !view_spec.is_derived()
                && previous_position == Some(position)
                && key == &envelope.key
                && metadata.op != "delete";
            if can_forward_patch {
                send_scoped_source_payload(
                    context,
                    subscription_id,
                    &view_spec.id,
                    envelope.payload.clone(),
                )?;
            } else {
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
}
