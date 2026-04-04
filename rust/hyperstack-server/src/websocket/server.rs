use crate::bus::BusManager;
use crate::cache::{cmp_seq, EntityCache, SnapshotBatchConfig};
use crate::compression::maybe_compress;
use crate::view::{ViewIndex, ViewSpec};
use crate::websocket::auth::{
    AuthContext, AuthDecision, AuthDeny, ConnectionAuthRequest, WebSocketAuthPlugin,
};
use crate::websocket::client_manager::{ClientManager, RateLimitConfig};
use crate::websocket::frame::{
    transform_large_u64_to_strings, Frame, Mode, SnapshotEntity, SnapshotFrame, SortConfig,
    SortOrder, SubscribedFrame,
};
use crate::websocket::subscription::{
    ClientMessage, RefreshAuthRequest, RefreshAuthResponse, SocketIssueMessage, Subscription,
};
use crate::websocket::usage::{WebSocketUsageEmitter, WebSocketUsageEvent};
use anyhow::Result;
use bytes::Bytes;
use futures_util::StreamExt;
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::Arc;
#[cfg(feature = "otel")]
use std::time::Instant;

use tokio::net::{TcpListener, TcpStream};
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

/// Helper function to handle refresh_auth messages
async fn handle_refresh_auth(
    client_id: Uuid,
    refresh_req: &RefreshAuthRequest,
    client_manager: &ClientManager,
    auth_plugin: &Arc<dyn WebSocketAuthPlugin>,
) {
    // Try to verify the new token using the auth plugin
    // We need to downcast to SignedSessionAuthPlugin to use verify_refresh_token
    let refresh_result: Result<AuthContext, String> = {
        // Try to downcast to SignedSessionAuthPlugin
        if let Some(signed_plugin) = auth_plugin
            .as_any()
            .downcast_ref::<crate::websocket::auth::SignedSessionAuthPlugin>(
        ) {
            signed_plugin
                .verify_refresh_token(&refresh_req.token)
                .await
                .map_err(|e| e.reason)
        } else {
            Err("In-band auth refresh not supported with current auth plugin".to_string())
        }
    };

    match refresh_result {
        Ok(new_context) => {
            let expires_at = new_context.expires_at;
            if client_manager.update_client_auth(client_id, new_context) {
                info!(
                    "Client {} refreshed auth successfully, expires at {}",
                    client_id, expires_at
                );

                // Send success response
                let response = RefreshAuthResponse {
                    success: true,
                    error: None,
                    expires_at: Some(expires_at),
                };
                if let Ok(json) = serde_json::to_string(&response) {
                    let _ = client_manager.send_text_to_client(client_id, json).await;
                }
            } else {
                warn!("Client {} not found when refreshing auth", client_id);

                // Send failure response - client not found
                let response = RefreshAuthResponse {
                    success: false,
                    error: Some("client-not-found".to_string()),
                    expires_at: None,
                };
                if let Ok(json) = serde_json::to_string(&response) {
                    let _ = client_manager.send_text_to_client(client_id, json).await;
                }
            }
        }
        Err(err) => {
            warn!("Client {} auth refresh failed: {}", client_id, err);

            // Send failure response with machine-readable error code
            let error_code = if err.contains("expired") {
                "token-expired"
            } else if err.contains("signature") {
                "token-invalid-signature"
            } else if err.contains("issuer") {
                "token-invalid-issuer"
            } else if err.contains("audience") {
                "token-invalid-audience"
            } else {
                "token-invalid"
            };

            let response = RefreshAuthResponse {
                success: false,
                error: Some(error_code.to_string()),
                expires_at: None,
            };
            if let Ok(json) = serde_json::to_string(&response) {
                let _ = client_manager.send_text_to_client(client_id, json).await;
            }
        }
    }
}

async fn send_socket_issue(
    client_id: Uuid,
    client_manager: &ClientManager,
    deny: &AuthDeny,
    fatal: bool,
) {
    let message = SocketIssueMessage::from_auth_deny(deny, fatal);
    match serde_json::to_string(&message) {
        Ok(json) => {
            let _ = client_manager.send_text_to_client(client_id, json).await;
        }
        Err(error) => {
            warn!(error = %error, client_id = %client_id, "failed to serialize socket issue message");
        }
    }
}

fn auth_deny_from_subscription_error(reason: &str) -> Option<AuthDeny> {
    if reason.starts_with("Snapshot limit exceeded:") {
        Some(AuthDeny::new(
            crate::websocket::auth::AuthErrorCode::SnapshotLimitExceeded,
            reason,
        ))
    } else {
        None
    }
}

fn key_class_label(key_class: hyperstack_auth::KeyClass) -> &'static str {
    match key_class {
        hyperstack_auth::KeyClass::Secret => "secret",
        hyperstack_auth::KeyClass::Publishable => "publishable",
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
        Some(ctx) => (
            Some(ctx.metering_key.clone()),
            Some(ctx.subject.clone()),
            Some(key_class_label(ctx.key_class).to_string()),
            ctx.deployment_id.clone(),
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

struct SubscriptionContext<'a> {
    client_id: Uuid,
    client_manager: &'a ClientManager,
    bus_manager: &'a BusManager,
    entity_cache: &'a EntityCache,
    view_index: &'a ViewIndex,
    usage_emitter: &'a Option<Arc<dyn WebSocketUsageEmitter>>,
    #[cfg(feature = "otel")]
    metrics: Option<Arc<Metrics>>,
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
            max_clients: 10000,
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
            max_clients: 10000,
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

    /// Configure rate limiting for the WebSocket server.
    ///
    /// This allows setting global rate limits that apply to all connections,
    /// such as maximum connections per IP, timeouts, and rate windows.
    /// Per-subject limits are controlled via AuthContext.Limits from the auth token.
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
        info!("WebSocket server listening on {}", self.bind_addr);

        // Apply rate limit configuration if provided
        let client_manager = if let Some(config) = self.rate_limit_config {
            ClientManager::with_config(config)
        } else {
            self.client_manager
        };

        client_manager.start_cleanup_task();

        loop {
            match listener.accept().await {
                Ok((stream, addr)) => {
                    let client_count = client_manager.client_count();
                    if client_count >= self.max_clients {
                        warn!(
                            "Rejecting connection from {} - max clients ({}) reached",
                            addr, self.max_clients
                        );
                        drop(stream);
                        continue;
                    }

                    info!(
                        "New WebSocket connection from {} ({}/{} clients)",
                        addr,
                        client_count + 1,
                        self.max_clients
                    );
                    let client_manager = client_manager.clone();
                    let bus_manager = self.bus_manager.clone();
                    let entity_cache = self.entity_cache.clone();
                    let view_index = self.view_index.clone();
                    #[cfg(feature = "otel")]
                    let metrics = self.metrics.clone();

                    let auth_plugin = self.auth_plugin.clone();
                    let usage_emitter = self.usage_emitter.clone();

                    tokio::spawn(
                        async move {
                            #[cfg(feature = "otel")]
                            let result = handle_connection(
                                stream,
                                client_manager,
                                bus_manager,
                                entity_cache,
                                view_index,
                                addr,
                                auth_plugin,
                                usage_emitter,
                                metrics,
                            )
                            .await;
                            #[cfg(not(feature = "otel"))]
                            let result = handle_connection(
                                stream,
                                client_manager,
                                bus_manager,
                                entity_cache,
                                view_index,
                                addr,
                                auth_plugin,
                                usage_emitter,
                            )
                            .await;

                            if let Err(e) = result {
                                error!("WebSocket connection error: {}", e);
                            }
                        }
                        .instrument(info_span!("ws.connection", %addr)),
                    );
                }
                Err(e) => {
                    error!("Failed to accept connection: {}", e);
                }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::websocket::auth::{AuthDeny, AuthErrorCode};
    use std::time::Duration;

    #[test]
    fn handshake_error_response_serializes_json_and_retry_after() {
        let response = Response::builder()
            .status(StatusCode::SWITCHING_PROTOCOLS)
            .body(())
            .unwrap();
        let deny = AuthDeny::rate_limited(Duration::from_secs(7), "websocket handshakes");
        let reject = HandshakeReject::from_deny(&deny);

        let handshake_response = build_handshake_error_response(&response, &reject);
        assert_eq!(handshake_response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            handshake_response.headers().get("X-Error-Code").unwrap(),
            "rate-limit-exceeded"
        );
        assert_eq!(
            handshake_response.headers().get("Retry-After").unwrap(),
            "7"
        );

        let body = handshake_response.into_body().unwrap();
        assert!(body.contains("rate-limit-exceeded"));
        assert!(body.contains("retryable"));
    }

    #[test]
    fn handshake_error_response_preserves_non_retryable_auth_denies() {
        let response = Response::builder()
            .status(StatusCode::SWITCHING_PROTOCOLS)
            .body(())
            .unwrap();
        let deny = AuthDeny::new(AuthErrorCode::OriginMismatch, "origin mismatch");
        let reject = HandshakeReject::from_deny(&deny);

        let handshake_response = build_handshake_error_response(&response, &reject);
        assert_eq!(handshake_response.status(), StatusCode::FORBIDDEN);
        assert!(handshake_response.headers().get("Retry-After").is_none());
    }
}

#[allow(clippy::result_large_err)]
async fn accept_authorized_connection(
    stream: TcpStream,
    remote_addr: SocketAddr,
    auth_plugin: Arc<dyn WebSocketAuthPlugin>,
    client_manager: ClientManager,
) -> Result<Option<(tokio_tungstenite::WebSocketStream<TcpStream>, AuthContext)>> {
    use std::sync::Mutex;

    let auth_result_capture: Arc<Mutex<Option<Result<AuthContext, HandshakeReject>>>> =
        Arc::new(Mutex::new(None));
    let auth_result_ref = auth_result_capture.clone();
    let auth_plugin_ref = auth_plugin.clone();
    let client_manager_for_auth = client_manager.clone();

    let handshake_result = accept_hdr_async(stream, move |request: &Request, response| {
        let connection_request = ConnectionAuthRequest::from_http_request(remote_addr, request);

        let auth_result = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                match auth_plugin_ref.authorize(&connection_request).await {
                    AuthDecision::Allow(ctx) => {
                        match client_manager_for_auth
                            .check_connection_allowed(remote_addr, &Some(ctx.clone()))
                            .await
                        {
                            Ok(()) => Ok(ctx),
                            Err(deny) => Err(HandshakeReject::from_deny(&deny)),
                        }
                    }
                    AuthDecision::Deny(deny) => Err(HandshakeReject::from_deny(&deny)),
                }
            })
        });

        let mut capture_lock = auth_result_ref.lock().expect("capture lock poisoned");
        *capture_lock = Some(auth_result.clone());

        match auth_result {
            Ok(_) => Ok(response),
            Err(reject) => Err(build_handshake_error_response(&response, &reject)),
        }
    })
    .await;

    let auth_result = {
        let mut guard = auth_result_capture.lock().expect("capture lock poisoned");
        guard.take()
    };

    match handshake_result {
        Ok(ws_stream) => match auth_result {
            Some(Ok(ctx)) => {
                info!("WebSocket connection authorized for {}", remote_addr);
                Ok(Some((ws_stream, ctx)))
            }
            Some(Err(reject)) => Err(anyhow::anyhow!(
                "handshake unexpectedly succeeded after rejection: {}",
                reject.body.message
            )),
            None => Err(anyhow::anyhow!(
                "no auth result captured for authorized connection {}",
                remote_addr
            )),
        },
        Err(WsError::Http(_)) => {
            match auth_result {
                Some(Err(reject)) => {
                    warn!(
                        "WebSocket connection rejected during handshake for {}: {}",
                        remote_addr, reject.body.message
                    );
                }
                Some(Ok(_)) => {
                    warn!(
                        "WebSocket handshake failed after auth success for {}",
                        remote_addr
                    );
                }
                None => {
                    warn!(
                        "WebSocket handshake rejected for {} without captured auth result",
                        remote_addr
                    );
                }
            }
            Ok(None)
        }
        Err(err) => Err(err.into()),
    }
}

#[cfg(feature = "otel")]
async fn handle_connection(
    stream: TcpStream,
    client_manager: ClientManager,
    bus_manager: BusManager,
    entity_cache: EntityCache,
    view_index: Arc<ViewIndex>,
    remote_addr: std::net::SocketAddr,
    auth_plugin: Arc<dyn WebSocketAuthPlugin>,
    usage_emitter: Option<Arc<dyn WebSocketUsageEmitter>>,
    metrics: Option<Arc<Metrics>>,
) -> Result<()> {
    let Some((ws_stream, auth_context)) = accept_authorized_connection(
        stream,
        remote_addr,
        auth_plugin.clone(),
        client_manager.clone(),
    )
    .await?
    else {
        return Ok(());
    };

    let client_id = Uuid::new_v4();
    let connection_start = Instant::now();

    let auth_context = Some(auth_context);
    let auth_context_ref = Some(&auth_context);
    let (usage_metering_key, usage_subject, usage_key_class, usage_deployment_id) =
        usage_identity(auth_context_ref);

    // Extract metering key from auth context for metrics attribution
    let metering_key = auth_context.as_ref().map(|ctx| ctx.metering_key.clone());

    if let Some(ref m) = metrics {
        if let Some(ref mk) = metering_key {
            m.record_ws_connection_with_metering(mk);
        } else {
            m.record_ws_connection();
        }
    }

    info!("WebSocket connection established for client {}", client_id);

    emit_usage_event(
        &usage_emitter,
        WebSocketUsageEvent::ConnectionEstablished {
            client_id: client_id.to_string(),
            remote_addr: remote_addr.to_string(),
            deployment_id: usage_deployment_id.clone(),
            metering_key: usage_metering_key.clone(),
            subject: usage_subject.clone(),
            key_class: usage_key_class,
        },
    );

    let (ws_sender, mut ws_receiver) = ws_stream.split();

    // Add client with auth context and IP tracking
    client_manager.add_client(client_id, ws_sender, auth_context, remote_addr);

    let ctx = SubscriptionContext {
        client_id,
        client_manager: &client_manager,
        bus_manager: &bus_manager,
        entity_cache: &entity_cache,
        view_index: &view_index,
        usage_emitter: &usage_emitter,
        metrics: metrics.clone(),
    };

    let mut active_subscriptions: HashMap<String, String> = HashMap::new();

    loop {
        tokio::select! {
            ws_msg = ws_receiver.next() => {
                match ws_msg {
                    Some(Ok(msg)) => {
                        if msg.is_close() {
                            info!("Client {} requested close", client_id);
                            break;
                        }

                        client_manager.update_client_last_seen(client_id);

                        if msg.is_text() {
                            if let Err(deny) = client_manager.check_inbound_message_allowed(client_id) {
                                warn!("Inbound message rejected for client {}: {}", client_id, deny.reason);
                                send_socket_issue(client_id, &client_manager, &deny, true).await;
                                break;
                            }

                            if let Some(ref m) = metrics {
                                if let Some(ref mk) = metering_key {
                                    m.record_ws_message_received_with_metering(mk);
                                } else {
                                    m.record_ws_message_received();
                                }
                            }

                            if let Ok(text) = msg.to_text() {
                                debug!("Received text message from client {}: {}", client_id, text);

                                if let Ok(client_msg) = serde_json::from_str::<ClientMessage>(text) {
                                    match client_msg {
                                        ClientMessage::Subscribe(subscription) => {
                                            let view_id = subscription.view.clone();
                                            let sub_key = subscription.sub_key();

                                            // Check subscription limits
                                            if let Err(deny) = client_manager.check_subscription_allowed(client_id).await {
                                                warn!("Subscription rejected for client {}: {}", client_id, deny.reason);
                                                send_socket_issue(client_id, &client_manager, &deny, false).await;
                                                continue;
                                            }

                                            client_manager.update_subscription(client_id, subscription.clone());

                                            let cancel_token = CancellationToken::new();
                                            let is_new = client_manager.add_client_subscription(
                                                client_id,
                                                sub_key.clone(),
                                                cancel_token.clone(),
                                            ).await;

                                            if !is_new {
                                                debug!("Client {} already subscribed to {}, ignoring duplicate", client_id, sub_key);
                                                continue;
                                            }

                                            if let Err(err) = attach_client_to_bus(&ctx, subscription, cancel_token).await {
                                                warn!(
                                                    "Subscription rejected for client {} on {}: {}",
                                                    client_id, view_id, err
                                                );
                                                if let Some(deny) = auth_deny_from_subscription_error(&err.to_string()) {
                                                    send_socket_issue(client_id, &client_manager, &deny, false).await;
                                                }
                                                let _ = client_manager
                                                    .remove_client_subscription(client_id, &sub_key)
                                                    .await;
                                                continue;
                                            }

                                            if let Some(ref m) = metrics {
                                                if let Some(ref mk) = metering_key {
                                                    m.record_subscription_created_with_metering(&view_id, mk);
                                                } else {
                                                    m.record_subscription_created(&view_id);
                                                }
                                            }
                                            active_subscriptions.insert(sub_key, view_id.clone());
                                            emit_usage_event(
                                                &usage_emitter,
                                                WebSocketUsageEvent::SubscriptionCreated {
                                                    client_id: client_id.to_string(),
                                                    deployment_id: usage_deployment_id.clone(),
                                                    metering_key: usage_metering_key.clone(),
                                                    subject: usage_subject.clone(),
                                                    view_id,
                                                },
                                            );
                                        }
                                        ClientMessage::Unsubscribe(unsub) => {
                                            let sub_key = unsub.sub_key();
                                            let removed = client_manager
                                                .remove_client_subscription(client_id, &sub_key)
                                                .await;

                                            if removed {
                                                info!("Client {} unsubscribed from {}", client_id, sub_key);
                                                active_subscriptions.remove(&sub_key);
                                                if let Some(ref m) = metrics {
                                                    if let Some(ref mk) = metering_key {
                                                        m.record_subscription_removed_with_metering(&unsub.view, mk);
                                                    } else {
                                                        m.record_subscription_removed(&unsub.view);
                                                    }
                                                }
                                                emit_usage_event(
                                                    &usage_emitter,
                                                    WebSocketUsageEvent::SubscriptionRemoved {
                                                        client_id: client_id.to_string(),
                                                        deployment_id: usage_deployment_id.clone(),
                                                        metering_key: usage_metering_key.clone(),
                                                        subject: usage_subject.clone(),
                                                        view_id: unsub.view.clone(),
                                                    },
                                                );
                                            }
                                        }
                                        ClientMessage::Ping => {
                                            debug!("Received ping from client {}", client_id);
                                        }
                                        ClientMessage::RefreshAuth(refresh_req) => {
                                            debug!("Received refresh_auth from client {}", client_id);
                                            handle_refresh_auth(client_id, &refresh_req, &client_manager, &auth_plugin).await;
                                        }
                                    }
                                } else if let Ok(subscription) = serde_json::from_str::<Subscription>(text) {
                                    let view_id = subscription.view.clone();
                                    let sub_key = subscription.sub_key();

                                    if let Err(deny) = client_manager.check_subscription_allowed(client_id).await {
                                        warn!("Subscription rejected for client {}: {}", client_id, deny.reason);
                                        send_socket_issue(client_id, &client_manager, &deny, false).await;
                                        continue;
                                    }

                                    client_manager.update_subscription(client_id, subscription.clone());

                                    let cancel_token = CancellationToken::new();
                                    let is_new = client_manager.add_client_subscription(
                                        client_id,
                                        sub_key.clone(),
                                        cancel_token.clone(),
                                    ).await;

                                    if !is_new {
                                        debug!("Client {} already subscribed to {}, ignoring duplicate", client_id, sub_key);
                                        continue;
                                    }

                                    if let Err(err) = attach_client_to_bus(&ctx, subscription, cancel_token).await {
                                        warn!(
                                            "Subscription rejected for client {} on {}: {}",
                                            client_id, view_id, err
                                        );
                                        if let Some(deny) = auth_deny_from_subscription_error(&err.to_string()) {
                                            send_socket_issue(client_id, &client_manager, &deny, false).await;
                                        }
                                        let _ = client_manager
                                            .remove_client_subscription(client_id, &sub_key)
                                            .await;
                                        continue;
                                    }

                                    if let Some(ref m) = metrics {
                                        if let Some(ref mk) = metering_key {
                                            m.record_subscription_created_with_metering(&view_id, mk);
                                        } else {
                                            m.record_subscription_created(&view_id);
                                        }
                                    }
                                    active_subscriptions.insert(sub_key, view_id.clone());
                                    emit_usage_event(
                                        &usage_emitter,
                                        WebSocketUsageEvent::SubscriptionCreated {
                                            client_id: client_id.to_string(),
                                            deployment_id: usage_deployment_id.clone(),
                                            metering_key: usage_metering_key.clone(),
                                            subject: usage_subject.clone(),
                                            view_id,
                                        },
                                    );
                                } else {
                                    debug!("Received non-subscription message from client {}: {}", client_id, text);
                                }
                            }
                        }
                    }
                    Some(Err(e)) => {
                        warn!("WebSocket error for client {}: {}", client_id, e);
                        break;
                    }
                    None => {
                        debug!("WebSocket stream ended for client {}", client_id);
                        break;
                    }
                }
            }
        }
    }

    client_manager
        .cancel_all_client_subscriptions(client_id)
        .await;
    client_manager.remove_client(client_id);
    if let Some(rate_limiter) = client_manager.rate_limiter().cloned() {
        rate_limiter.remove_client_buckets(client_id).await;
    }

    if let Some(ref m) = metrics {
        let duration_secs = connection_start.elapsed().as_secs_f64();
        if let Some(ref mk) = metering_key {
            m.record_ws_disconnection_with_metering(duration_secs, mk);
            for view_id in active_subscriptions.values() {
                m.record_subscription_removed_with_metering(view_id, mk);
            }
        } else {
            m.record_ws_disconnection(duration_secs);
            for view_id in active_subscriptions.values() {
                m.record_subscription_removed(view_id);
            }
        }
    }

    for view_id in active_subscriptions.values() {
        emit_usage_event(
            &usage_emitter,
            WebSocketUsageEvent::SubscriptionRemoved {
                client_id: client_id.to_string(),
                deployment_id: usage_deployment_id.clone(),
                metering_key: usage_metering_key.clone(),
                subject: usage_subject.clone(),
                view_id: view_id.clone(),
            },
        );
    }

    emit_usage_event(
        &usage_emitter,
        WebSocketUsageEvent::ConnectionClosed {
            client_id: client_id.to_string(),
            deployment_id: usage_deployment_id,
            metering_key: usage_metering_key,
            subject: usage_subject,
            duration_secs: Some(connection_start.elapsed().as_secs_f64()),
            subscription_count: u32::try_from(active_subscriptions.len()).unwrap_or(u32::MAX),
        },
    );

    info!("Client {} disconnected", client_id);

    Ok(())
}

#[cfg(not(feature = "otel"))]
#[allow(clippy::too_many_arguments)]
async fn handle_connection(
    stream: TcpStream,
    client_manager: ClientManager,
    bus_manager: BusManager,
    entity_cache: EntityCache,
    view_index: Arc<ViewIndex>,
    remote_addr: std::net::SocketAddr,
    auth_plugin: Arc<dyn WebSocketAuthPlugin>,
    usage_emitter: Option<Arc<dyn WebSocketUsageEmitter>>,
) -> Result<()> {
    let Some((ws_stream, auth_context)) = accept_authorized_connection(
        stream,
        remote_addr,
        auth_plugin.clone(),
        client_manager.clone(),
    )
    .await?
    else {
        return Ok(());
    };

    let client_id = Uuid::new_v4();
    let auth_context_ref = Some(&auth_context);
    let (usage_metering_key, usage_subject, usage_key_class, usage_deployment_id) =
        usage_identity(auth_context_ref);

    let auth_context = Some(auth_context);

    info!("WebSocket connection established for client {}", client_id);

    emit_usage_event(
        &usage_emitter,
        WebSocketUsageEvent::ConnectionEstablished {
            client_id: client_id.to_string(),
            remote_addr: remote_addr.to_string(),
            deployment_id: usage_deployment_id.clone(),
            metering_key: usage_metering_key.clone(),
            subject: usage_subject.clone(),
            key_class: usage_key_class,
        },
    );

    let (ws_sender, mut ws_receiver) = ws_stream.split();

    // Add client with auth context and IP tracking
    client_manager.add_client(client_id, ws_sender, auth_context, remote_addr);

    let ctx = SubscriptionContext {
        client_id,
        client_manager: &client_manager,
        bus_manager: &bus_manager,
        entity_cache: &entity_cache,
        view_index: &view_index,
        usage_emitter: &usage_emitter,
    };

    let mut active_subscriptions: HashMap<String, String> = HashMap::new();

    loop {
        tokio::select! {
            ws_msg = ws_receiver.next() => {
                match ws_msg {
                    Some(Ok(msg)) => {
                        if msg.is_close() {
                            info!("Client {} requested close", client_id);
                            break;
                        }

                        client_manager.update_client_last_seen(client_id);

                        if msg.is_text() {
                            if let Err(deny) = client_manager.check_inbound_message_allowed(client_id) {
                                warn!("Inbound message rejected for client {}: {}", client_id, deny.reason);
                                send_socket_issue(client_id, &client_manager, &deny, true).await;
                                break;
                            }

                            if let Ok(text) = msg.to_text() {
                                debug!("Received text message from client {}: {}", client_id, text);

                                if let Ok(client_msg) = serde_json::from_str::<ClientMessage>(text) {
                                    match client_msg {
                                        ClientMessage::Subscribe(subscription) => {
                                            let view_id = subscription.view.clone();
                                            if let Err(deny) = client_manager.check_subscription_allowed(client_id).await {
                                                warn!("Subscription rejected for client {}: {}", client_id, deny.reason);
                                                send_socket_issue(client_id, &client_manager, &deny, false).await;
                                                continue;
                                            }

                                            let sub_key = subscription.sub_key();
                                            client_manager.update_subscription(client_id, subscription.clone());

                                            let cancel_token = CancellationToken::new();
                                            let is_new = client_manager.add_client_subscription(
                                                client_id,
                                                sub_key.clone(),
                                                cancel_token.clone(),
                                            ).await;

                                            if !is_new {
                                                debug!("Client {} already subscribed to {}, ignoring duplicate", client_id, sub_key);
                                                continue;
                                            }

                                            if let Err(err) = attach_client_to_bus(&ctx, subscription, cancel_token).await {
                                                warn!(
                                                    "Subscription rejected for client {} on {}: {}",
                                                    client_id,
                                                    sub_key,
                                                    err
                                                );
                                                if let Some(deny) = auth_deny_from_subscription_error(&err.to_string()) {
                                                    send_socket_issue(client_id, &client_manager, &deny, false).await;
                                                }
                                                let _ = client_manager
                                                    .remove_client_subscription(client_id, &sub_key)
                                                    .await;
                                            } else {
                                                active_subscriptions.insert(sub_key, view_id.clone());
                                                emit_usage_event(
                                                    &usage_emitter,
                                                    WebSocketUsageEvent::SubscriptionCreated {
                                                        client_id: client_id.to_string(),
                                                        deployment_id: usage_deployment_id.clone(),
                                                        metering_key: usage_metering_key.clone(),
                                                        subject: usage_subject.clone(),
                                                        view_id,
                                                    },
                                                );
                                            }
                                        }
                                        ClientMessage::Unsubscribe(unsub) => {
                                            let sub_key = unsub.sub_key();
                                            let removed = client_manager
                                                .remove_client_subscription(client_id, &sub_key)
                                                .await;

                                            if removed {
                                                info!("Client {} unsubscribed from {}", client_id, sub_key);
                                                active_subscriptions.remove(&sub_key);
                                                emit_usage_event(
                                                    &usage_emitter,
                                                    WebSocketUsageEvent::SubscriptionRemoved {
                                                        client_id: client_id.to_string(),
                                                        deployment_id: usage_deployment_id.clone(),
                                                        metering_key: usage_metering_key.clone(),
                                                        subject: usage_subject.clone(),
                                                        view_id: unsub.view.clone(),
                                                    },
                                                );
                                            }
                                        }
                                        ClientMessage::Ping => {
                                            debug!("Received ping from client {}", client_id);
                                        }
                                        ClientMessage::RefreshAuth(refresh_req) => {
                                            debug!("Received refresh_auth from client {}", client_id);
                                            handle_refresh_auth(client_id, &refresh_req, &client_manager, &auth_plugin).await;
                                        }
                                    }
                                } else if let Ok(subscription) = serde_json::from_str::<Subscription>(text) {
                                    let view_id = subscription.view.clone();
                                    if let Err(deny) = client_manager.check_subscription_allowed(client_id).await {
                                        warn!("Subscription rejected for client {}: {}", client_id, deny.reason);
                                        send_socket_issue(client_id, &client_manager, &deny, false).await;
                                        continue;
                                    }

                                    let sub_key = subscription.sub_key();
                                    client_manager.update_subscription(client_id, subscription.clone());

                                    let cancel_token = CancellationToken::new();
                                    let is_new = client_manager.add_client_subscription(
                                        client_id,
                                        sub_key.clone(),
                                        cancel_token.clone(),
                                    ).await;

                                    if !is_new {
                                        debug!("Client {} already subscribed to {}, ignoring duplicate", client_id, sub_key);
                                        continue;
                                    }

                                    if let Err(err) = attach_client_to_bus(&ctx, subscription, cancel_token).await {
                                        warn!(
                                            "Subscription rejected for client {} on {}: {}",
                                            client_id,
                                            sub_key,
                                            err
                                        );
                                        if let Some(deny) = auth_deny_from_subscription_error(&err.to_string()) {
                                            send_socket_issue(client_id, &client_manager, &deny, false).await;
                                        }
                                        let _ = client_manager
                                            .remove_client_subscription(client_id, &sub_key)
                                            .await;
                                    } else {
                                        active_subscriptions.insert(sub_key, view_id.clone());
                                        emit_usage_event(
                                            &usage_emitter,
                                            WebSocketUsageEvent::SubscriptionCreated {
                                                client_id: client_id.to_string(),
                                                deployment_id: usage_deployment_id.clone(),
                                                metering_key: usage_metering_key.clone(),
                                                subject: usage_subject.clone(),
                                                view_id,
                                            },
                                        );
                                    }
                                } else {
                                    debug!("Received non-subscription message from client {}: {}", client_id, text);
                                }
                            }
                        }
                    }
                    Some(Err(e)) => {
                        warn!("WebSocket error for client {}: {}", client_id, e);
                        break;
                    }
                    None => {
                        debug!("WebSocket stream ended for client {}", client_id);
                        break;
                    }
                }
            }
        }
    }

    client_manager
        .cancel_all_client_subscriptions(client_id)
        .await;
    client_manager.remove_client(client_id);
    if let Some(rate_limiter) = client_manager.rate_limiter().cloned() {
        rate_limiter.remove_client_buckets(client_id).await;
    }

    for view_id in active_subscriptions.values() {
        emit_usage_event(
            &usage_emitter,
            WebSocketUsageEvent::SubscriptionRemoved {
                client_id: client_id.to_string(),
                deployment_id: usage_deployment_id.clone(),
                metering_key: usage_metering_key.clone(),
                subject: usage_subject.clone(),
                view_id: view_id.clone(),
            },
        );
    }

    emit_usage_event(
        &usage_emitter,
        WebSocketUsageEvent::ConnectionClosed {
            client_id: client_id.to_string(),
            deployment_id: usage_deployment_id,
            metering_key: usage_metering_key,
            subject: usage_subject,
            duration_secs: None,
            subscription_count: u32::try_from(active_subscriptions.len()).unwrap_or(u32::MAX),
        },
    );

    info!("Client {} disconnected", client_id);

    Ok(())
}

async fn send_snapshot_batches(
    client_id: Uuid,
    entities: &[SnapshotEntity],
    mode: Mode,
    view_id: &str,
    client_manager: &ClientManager,
    usage_emitter: &Option<Arc<dyn WebSocketUsageEmitter>>,
    batch_config: &SnapshotBatchConfig,
    #[cfg(feature = "otel")] metrics: Option<&Arc<Metrics>>,
) -> Result<()> {
    let total = entities.len();
    if total == 0 {
        return Ok(());
    }

    let mut offset = 0;
    let mut batch_num = 0;

    while offset < total {
        let batch_size = if batch_num == 0 {
            batch_config.initial_batch_size
        } else {
            batch_config.subsequent_batch_size
        };

        let end = (offset + batch_size).min(total);
        let batch_data: Vec<SnapshotEntity> = entities[offset..end].to_vec();
        let rows_in_batch = batch_data.len() as u32;
        let is_complete = end >= total;

        let snapshot_frame = SnapshotFrame {
            mode,
            export: view_id.to_string(),
            op: "snapshot",
            data: batch_data,
            complete: is_complete,
        };

        if let Ok(json_payload) = serde_json::to_vec(&snapshot_frame) {
            let payload = maybe_compress(&json_payload);
            let payload_bytes = payload.as_bytes().len() as u64;
            if client_manager
                .send_compressed_async(client_id, payload)
                .await
                .is_err()
            {
                return Err(anyhow::anyhow!("Failed to send snapshot batch"));
            }
            #[cfg(feature = "otel")]
            if let Some(m) = metrics {
                m.record_ws_message_sent();
            }

            let auth_context = client_manager.get_auth_context(client_id);
            let (metering_key, subject, _, deployment_id) = usage_identity(auth_context.as_ref());
            emit_usage_event(
                usage_emitter,
                WebSocketUsageEvent::SnapshotSent {
                    client_id: client_id.to_string(),
                    deployment_id,
                    metering_key,
                    subject,
                    view_id: view_id.to_string(),
                    rows: rows_in_batch,
                    messages: 1,
                    bytes: payload_bytes,
                },
            );
        }

        offset = end;
        batch_num += 1;
    }

    debug!(
        "Sent {} snapshot batches ({} entities) for {} to client {}",
        batch_num, total, view_id, client_id
    );

    Ok(())
}

fn extract_sort_config(view_spec: &ViewSpec) -> Option<SortConfig> {
    if let Some(sort) = view_spec.pipeline.as_ref().and_then(|p| p.sort.as_ref()) {
        return Some(SortConfig {
            field: sort.field_path.clone(),
            order: match sort.order {
                crate::materialized_view::SortOrder::Asc => SortOrder::Asc,
                crate::materialized_view::SortOrder::Desc => SortOrder::Desc,
            },
        });
    }

    if view_spec.mode == Mode::List {
        return Some(SortConfig {
            field: vec!["_seq".to_string()],
            order: SortOrder::Desc,
        });
    }

    None
}

fn send_subscribed_frame(
    client_id: Uuid,
    view_id: &str,
    view_spec: &ViewSpec,
    client_manager: &ClientManager,
    usage_emitter: &Option<Arc<dyn WebSocketUsageEmitter>>,
) -> Result<()> {
    let sort_config = extract_sort_config(view_spec);
    let subscribed_frame = SubscribedFrame::new(view_id.to_string(), view_spec.mode, sort_config);

    let json_payload = serde_json::to_vec(&subscribed_frame)?;
    let payload_bytes = json_payload.len() as u64;
    let payload = Arc::new(Bytes::from(json_payload));
    client_manager
        .send_to_client(client_id, payload)
        .map_err(|e| anyhow::anyhow!("Failed to send subscribed frame: {:?}", e))?;

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
            bytes: payload_bytes,
        },
    );

    Ok(())
}

fn enforce_snapshot_limit(ctx: &SubscriptionContext<'_>, rows: usize) -> Result<()> {
    let requested_rows = u32::try_from(rows).unwrap_or(u32::MAX);
    ctx.client_manager
        .check_snapshot_allowed(ctx.client_id, requested_rows)
        .map_err(|deny| anyhow::anyhow!(deny.reason))
}

#[cfg(feature = "otel")]
async fn attach_client_to_bus(
    ctx: &SubscriptionContext<'_>,
    subscription: Subscription,
    cancel_token: CancellationToken,
) -> Result<()> {
    let view_id = &subscription.view;

    let view_spec = match ctx.view_index.get_view(view_id) {
        Some(spec) => spec.clone(),
        None => {
            return Err(anyhow::anyhow!("Unknown view ID: {}", view_id));
        }
    };

    send_subscribed_frame(
        ctx.client_id,
        view_id,
        &view_spec,
        ctx.client_manager,
        ctx.usage_emitter,
    )?;

    let is_derived_with_sort = view_spec.is_derived()
        && view_spec
            .pipeline
            .as_ref()
            .map(|p| p.sort.is_some())
            .unwrap_or(false);

    if is_derived_with_sort {
        return attach_derived_view_subscription_otel(ctx, subscription, view_spec, cancel_token)
            .await;
    }

    match view_spec.mode {
        Mode::State => {
            let key = subscription.key.as_deref().unwrap_or("");

            let mut rx = ctx.bus_manager.get_or_create_state_bus(view_id, key).await;

            // Check if we should send snapshot (defaults to true for backward compatibility)
            let should_send_snapshot = subscription.with_snapshot.unwrap_or(true);

            if should_send_snapshot {
                if let Some(mut cached_entity) = ctx.entity_cache.get(view_id, key).await {
                    transform_large_u64_to_strings(&mut cached_entity);
                    let snapshot_entities = vec![SnapshotEntity {
                        key: key.to_string(),
                        data: cached_entity,
                    }];
                    enforce_snapshot_limit(ctx, snapshot_entities.len())?;
                    let batch_config = ctx.entity_cache.snapshot_config();
                    send_snapshot_batches(
                        ctx.client_id,
                        &snapshot_entities,
                        view_spec.mode,
                        view_id,
                        ctx.client_manager,
                        ctx.usage_emitter,
                        &batch_config,
                        #[cfg(feature = "otel")]
                        ctx.metrics.as_ref(),
                    )
                    .await?;
                    rx.borrow_and_update();
                } else if !rx.borrow().is_empty() {
                    let data = rx.borrow_and_update().clone();
                    let data_len = data.len();
                    if ctx
                        .client_manager
                        .send_to_client(ctx.client_id, data)
                        .is_ok()
                    {
                        emit_update_sent_for_client(
                            ctx.usage_emitter,
                            ctx.client_manager,
                            ctx.client_id,
                            view_id,
                            data_len,
                        );
                    }
                }
            } else {
                info!(
                    "Client {} subscribed to {} without snapshot",
                    ctx.client_id, view_id
                );
                rx.borrow_and_update();
            }

            let client_id = ctx.client_id;
            let client_mgr = ctx.client_manager.clone();
            let usage_emitter = ctx.usage_emitter.clone();
            let metrics_clone = ctx.metrics.clone();
            let view_id_clone = view_id.clone();
            let key_clone = key.to_string();
            tokio::spawn(
                async move {
                    loop {
                        tokio::select! {
                            _ = cancel_token.cancelled() => {
                                debug!("State subscription cancelled for client {}", client_id);
                                break;
                            }
                            result = rx.changed() => {
                                if result.is_err() {
                                    break;
                                }
                                let data = rx.borrow().clone();
                                if client_mgr.send_to_client(client_id, data).is_err() {
                                    break;
                                }
                                if let Some(ref m) = metrics_clone {
                                    m.record_ws_message_sent();
                                }
                                let data_len = data.len();
                                emit_update_sent_for_client(
                                    &usage_emitter,
                                    &client_mgr,
                                    client_id,
                                    &view_id_clone,
                                    data_len,
                                );
                            }
                        }
                    }
                }
                .instrument(info_span!("ws.subscribe.state", %client_id, view = %view_id_clone, key = %key_clone)),
            );
        }
        Mode::List | Mode::Append => {
            let mut rx = ctx.bus_manager.get_or_create_list_bus(view_id).await;

            // Check if we should send snapshot (defaults to true for backward compatibility)
            let should_send_snapshot = subscription.with_snapshot.unwrap_or(true);

            if should_send_snapshot {
                // Determine which entities to send based on cursor
                let mut snapshots = if let Some(ref cursor) = subscription.after {
                    ctx.entity_cache
                        .get_after(view_id, cursor, subscription.snapshot_limit)
                        .await
                } else {
                    ctx.entity_cache.get_all(view_id).await
                };

                // Sort by _seq descending only when there is no cursor (to get most-recent N from full cache)
                if let Some(limit) = subscription.snapshot_limit {
                    if subscription.after.is_none() {
                        snapshots.sort_by(|a, b| {
                            let sa = a.1.get("_seq").and_then(|s| s.as_str()).unwrap_or("");
                            let sb = b.1.get("_seq").and_then(|s| s.as_str()).unwrap_or("");
                            cmp_seq(sb, sa) // descending: most-recent N
                        });
                        snapshots.truncate(limit);
                    }
                }

                let snapshot_entities: Vec<SnapshotEntity> = snapshots
                    .into_iter()
                    .filter(|(key, _)| subscription.matches_key(key))
                    .map(|(key, mut data)| {
                        transform_large_u64_to_strings(&mut data);
                        SnapshotEntity { key, data }
                    })
                    .collect();

                if !snapshot_entities.is_empty() {
                    enforce_snapshot_limit(ctx, snapshot_entities.len())?;
                    let batch_config = ctx.entity_cache.snapshot_config();
                    send_snapshot_batches(
                        ctx.client_id,
                        &snapshot_entities,
                        view_spec.mode,
                        view_id,
                        ctx.client_manager,
                        ctx.usage_emitter,
                        &batch_config,
                        #[cfg(feature = "otel")]
                        ctx.metrics.as_ref(),
                    )
                    .await?;
                }
            } else {
                info!(
                    "Client {} subscribed to {} without snapshot",
                    ctx.client_id, view_id
                );
            }

            let client_id = ctx.client_id;
            let client_mgr = ctx.client_manager.clone();
            let usage_emitter = ctx.usage_emitter.clone();
            let sub = subscription.clone();
            let metrics_clone = ctx.metrics.clone();
            let view_id_clone = view_id.clone();
            let mode = view_spec.mode;
            tokio::spawn(
                async move {
                    loop {
                        tokio::select! {
                            _ = cancel_token.cancelled() => {
                                debug!("List subscription cancelled for client {}", client_id);
                                break;
                            }
                            result = rx.recv() => {
                                match result {
                                    Ok(envelope) => {
                                        if sub.matches(&envelope.entity, &envelope.key) {
                                            if client_mgr
                                                .send_to_client(client_id, envelope.payload.clone())
                                                .is_err()
                                            {
                                                break;
                                            }
                                            if let Some(ref m) = metrics_clone {
                                                m.record_ws_message_sent();
                                            }
                                            emit_update_sent_for_client(
                                                &usage_emitter,
                                                &client_mgr,
                                                client_id,
                                                &view_id_clone,
                                                envelope.payload.len(),
                                            );
                                        }
                                    }
                                    Err(_) => break,
                                }
                            }
                        }
                    }
                }
                .instrument(info_span!("ws.subscribe.list", %client_id, view = %view_id_clone, mode = ?mode)),
            );
        }
    }

    info!(
        "Client {} subscribed to {} (mode: {:?})",
        ctx.client_id, view_id, view_spec.mode
    );

    Ok(())
}

#[cfg(feature = "otel")]
async fn attach_derived_view_subscription_otel(
    ctx: &SubscriptionContext<'_>,
    subscription: Subscription,
    view_spec: ViewSpec,
    cancel_token: CancellationToken,
) -> Result<()> {
    let view_id = &subscription.view;
    let pipeline_limit = view_spec
        .pipeline
        .as_ref()
        .and_then(|p| p.limit)
        .unwrap_or(100);
    let take = subscription.take.unwrap_or(pipeline_limit);
    let skip = subscription.skip.unwrap_or(0);
    let is_single = take == 1;

    let source_view_id = match &view_spec.source_view {
        Some(s) => s.clone(),
        None => {
            return Err(anyhow::anyhow!(
                "Derived view {} has no source_view",
                view_id
            ));
        }
    };

    let sorted_caches = ctx.view_index.sorted_caches();
    let initial_window: Vec<(String, serde_json::Value)> = {
        let mut caches = sorted_caches.write().await;
        if let Some(cache) = caches.get_mut(view_id) {
            cache.get_window(skip, take)
        } else {
            warn!("No sorted cache for derived view {}", view_id);
            vec![]
        }
    };

    let initial_keys: HashSet<String> = initial_window.iter().map(|(k, _)| k.clone()).collect();

    if !initial_window.is_empty() {
        let snapshot_entities: Vec<SnapshotEntity> = initial_window
            .into_iter()
            .map(|(key, mut data)| {
                transform_large_u64_to_strings(&mut data);
                SnapshotEntity { key, data }
            })
            .collect();

        enforce_snapshot_limit(ctx, snapshot_entities.len())?;
        let batch_config = ctx.entity_cache.snapshot_config();
        send_snapshot_batches(
            ctx.client_id,
            &snapshot_entities,
            view_spec.mode,
            view_id,
            ctx.client_manager,
            ctx.usage_emitter,
            &batch_config,
            ctx.metrics.as_ref(),
        )
        .await?;
    }

    let mut rx = ctx
        .bus_manager
        .get_or_create_list_bus(&source_view_id)
        .await;

    let client_id = ctx.client_id;
    let client_mgr = ctx.client_manager.clone();
    let usage_emitter = ctx.usage_emitter.clone();
    let view_id_clone = view_id.clone();
    let view_id_span = view_id.clone();
    let sorted_caches_clone = sorted_caches;
    let metrics_clone = ctx.metrics.clone();
    let frame_mode = view_spec.mode;

    tokio::spawn(
        async move {
            let mut current_window_keys = initial_keys;

            loop {
                tokio::select! {
                    _ = cancel_token.cancelled() => {
                        debug!("Derived view subscription cancelled for client {}", client_id);
                        break;
                    }
                    result = rx.recv() => {
                        match result {
                            Ok(_envelope) => {
                                let new_window: Vec<(String, serde_json::Value)> = {
                                    let mut caches = sorted_caches_clone.write().await;
                                    if let Some(cache) = caches.get_mut(&view_id_clone) {
                                        cache.get_window(skip, take)
                                    } else {
                                        continue;
                                    }
                                };

                                let new_keys: HashSet<String> =
                                    new_window.iter().map(|(k, _)| k.clone()).collect();

                                if is_single {
                                    if let Some((new_key, data)) = new_window.first() {
                                        for old_key in current_window_keys.difference(&new_keys) {
                                            let delete_frame = Frame {
                                            seq: None,
                                                mode: frame_mode,
                                                export: view_id_clone.clone(),
                                                op: "delete",
                                                key: old_key.clone(),
                                                data: serde_json::Value::Null,
                                                append: vec![],
                                            };
                                            if let Ok(json) = serde_json::to_vec(&delete_frame) {
                                                let payload = Arc::new(Bytes::from(json));
                                                let payload_len = payload.len();
                                                if client_mgr.send_to_client(client_id, payload).is_err() {
                                                    return;
                                                }
                                                if let Some(ref m) = metrics_clone {
                                                    m.record_ws_message_sent();
                                                }
                                                emit_update_sent_for_client(
                                                    &usage_emitter,
                                                    &client_mgr,
                                                    client_id,
                                                    &view_id_clone,
                                                    payload_len,
                                                );
                                            }
                                        }

                                        let mut transformed_data = data.clone();
                                        transform_large_u64_to_strings(&mut transformed_data);
                                        let frame = Frame {
                                            seq: None,
                                            mode: frame_mode,
                                            export: view_id_clone.clone(),
                                            op: "upsert",
                                            key: new_key.clone(),
                                            data: transformed_data,
                                            append: vec![],
                                        };

                                        if let Ok(json) = serde_json::to_vec(&frame) {
                                            let payload = Arc::new(Bytes::from(json));
                                            let payload_len = payload.len();
                                            if client_mgr.send_to_client(client_id, payload).is_err() {
                                                return;
                                            }
                                            if let Some(ref m) = metrics_clone {
                                                m.record_ws_message_sent();
                                            }
                                            emit_update_sent_for_client(
                                                &usage_emitter,
                                                &client_mgr,
                                                client_id,
                                                &view_id_clone,
                                                payload_len,
                                            );
                                        }
                                    }
                                } else {
                                    for key in current_window_keys.difference(&new_keys) {
                                        let delete_frame = Frame {
                                            seq: None,
                                            mode: frame_mode,
                                            export: view_id_clone.clone(),
                                            op: "delete",
                                            key: key.clone(),
                                            data: serde_json::Value::Null,
                                            append: vec![],
                                        };
                                        if let Ok(json) = serde_json::to_vec(&delete_frame) {
                                            let payload = Arc::new(Bytes::from(json));
                                            let payload_len = payload.len();
                                            if client_mgr.send_to_client(client_id, payload).is_err() {
                                                return;
                                            }
                                            if let Some(ref m) = metrics_clone {
                                                m.record_ws_message_sent();
                                            }
                                            emit_update_sent_for_client(
                                                &usage_emitter,
                                                &client_mgr,
                                                client_id,
                                                &view_id_clone,
                                                payload_len,
                                            );
                                        }
                                    }

                                    for (key, data) in &new_window {
                                        let mut transformed_data = data.clone();
                                        transform_large_u64_to_strings(&mut transformed_data);
                                        let frame = Frame {
                                            seq: None,
                                            mode: frame_mode,
                                            export: view_id_clone.clone(),
                                            op: "upsert",
                                            key: key.clone(),
                                            data: transformed_data,
                                            append: vec![],
                                        };
                                        if let Ok(json) = serde_json::to_vec(&frame) {
                                            let payload = Arc::new(Bytes::from(json));
                                            let payload_len = payload.len();
                                            if client_mgr.send_to_client(client_id, payload).is_err() {
                                                return;
                                            }
                                            if let Some(ref m) = metrics_clone {
                                                m.record_ws_message_sent();
                                            }
                                            emit_update_sent_for_client(
                                                &usage_emitter,
                                                &client_mgr,
                                                client_id,
                                                &view_id_clone,
                                                payload_len,
                                            );
                                        }
                                    }
                                }

                                current_window_keys = new_keys;
                            }
                            Err(_) => break,
                        }
                    }
                }
            }
        }
        .instrument(info_span!("ws.subscribe.derived", %client_id, view = %view_id_span)),
    );

    info!(
        "Client {} subscribed to derived view {} (take={}, skip={})",
        ctx.client_id, view_id, take, skip
    );

    Ok(())
}

#[cfg(not(feature = "otel"))]
async fn attach_client_to_bus(
    ctx: &SubscriptionContext<'_>,
    subscription: Subscription,
    cancel_token: CancellationToken,
) -> Result<()> {
    let view_id = &subscription.view;

    let view_spec = match ctx.view_index.get_view(view_id) {
        Some(spec) => spec.clone(),
        None => {
            return Err(anyhow::anyhow!("Unknown view ID: {}", view_id));
        }
    };

    send_subscribed_frame(
        ctx.client_id,
        view_id,
        &view_spec,
        ctx.client_manager,
        ctx.usage_emitter,
    )?;

    let is_derived_with_sort = view_spec.is_derived()
        && view_spec
            .pipeline
            .as_ref()
            .map(|p| p.sort.is_some())
            .unwrap_or(false);

    if is_derived_with_sort {
        return attach_derived_view_subscription(ctx, subscription, view_spec, cancel_token).await;
    }

    match view_spec.mode {
        Mode::State => {
            let key = subscription.key.as_deref().unwrap_or("");

            let mut rx = ctx.bus_manager.get_or_create_state_bus(view_id, key).await;

            // Check if we should send snapshot (defaults to true for backward compatibility)
            let should_send_snapshot = subscription.with_snapshot.unwrap_or(true);

            if should_send_snapshot {
                if let Some(mut cached_entity) = ctx.entity_cache.get(view_id, key).await {
                    transform_large_u64_to_strings(&mut cached_entity);
                    let snapshot_entities = vec![SnapshotEntity {
                        key: key.to_string(),
                        data: cached_entity,
                    }];
                    enforce_snapshot_limit(ctx, snapshot_entities.len())?;
                    let batch_config = ctx.entity_cache.snapshot_config();
                    send_snapshot_batches(
                        ctx.client_id,
                        &snapshot_entities,
                        view_spec.mode,
                        view_id,
                        ctx.client_manager,
                        ctx.usage_emitter,
                        &batch_config,
                    )
                    .await?;
                    rx.borrow_and_update();
                } else if !rx.borrow().is_empty() {
                    let data = rx.borrow_and_update().clone();
                    let data_len = data.len();
                    if ctx
                        .client_manager
                        .send_to_client(ctx.client_id, data)
                        .is_ok()
                    {
                        emit_update_sent_for_client(
                            ctx.usage_emitter,
                            ctx.client_manager,
                            ctx.client_id,
                            view_id,
                            data_len,
                        );
                    }
                }
            } else {
                info!(
                    "Client {} subscribed to {} without snapshot",
                    ctx.client_id, view_id
                );
                rx.borrow_and_update();
            }

            let client_id = ctx.client_id;
            let client_mgr = ctx.client_manager.clone();
            let usage_emitter = ctx.usage_emitter.clone();
            let view_id_clone = view_id.clone();
            let view_id_span = view_id.clone();
            let key_clone = key.to_string();
            tokio::spawn(
                async move {
                    loop {
                        tokio::select! {
                            _ = cancel_token.cancelled() => {
                                debug!("State subscription cancelled for client {}", client_id);
                                break;
                            }
                            result = rx.changed() => {
                                if result.is_err() {
                                    break;
                                }
                                let data = rx.borrow().clone();
                                let data_len = data.len();
                                if client_mgr.send_to_client(client_id, data).is_err() {
                                    break;
                                }
                                emit_update_sent_for_client(
                                    &usage_emitter,
                                    &client_mgr,
                                    client_id,
                                    &view_id_clone,
                                    data_len,
                                );
                            }
                        }
                    }
                }
                .instrument(info_span!("ws.subscribe.state", %client_id, view = %view_id_span, key = %key_clone)),
            );
        }
        Mode::List | Mode::Append => {
            let mut rx = ctx.bus_manager.get_or_create_list_bus(view_id).await;

            // Check if we should send snapshot (defaults to true for backward compatibility)
            let should_send_snapshot = subscription.with_snapshot.unwrap_or(true);

            if should_send_snapshot {
                // Determine which entities to send based on cursor
                let mut snapshots = if let Some(ref cursor) = subscription.after {
                    ctx.entity_cache
                        .get_after(view_id, cursor, subscription.snapshot_limit)
                        .await
                } else {
                    ctx.entity_cache.get_all(view_id).await
                };

                // Sort by _seq descending only when there is no cursor (to get most-recent N from full cache)
                if let Some(limit) = subscription.snapshot_limit {
                    if subscription.after.is_none() {
                        snapshots.sort_by(|a, b| {
                            let sa = a.1.get("_seq").and_then(|s| s.as_str()).unwrap_or("");
                            let sb = b.1.get("_seq").and_then(|s| s.as_str()).unwrap_or("");
                            cmp_seq(sb, sa) // descending: most-recent N
                        });
                        snapshots.truncate(limit);
                    }
                }

                let snapshot_entities: Vec<SnapshotEntity> = snapshots
                    .into_iter()
                    .filter(|(key, _)| subscription.matches_key(key))
                    .map(|(key, mut data)| {
                        transform_large_u64_to_strings(&mut data);
                        SnapshotEntity { key, data }
                    })
                    .collect();

                if !snapshot_entities.is_empty() {
                    enforce_snapshot_limit(ctx, snapshot_entities.len())?;
                    let batch_config = ctx.entity_cache.snapshot_config();
                    send_snapshot_batches(
                        ctx.client_id,
                        &snapshot_entities,
                        view_spec.mode,
                        view_id,
                        ctx.client_manager,
                        ctx.usage_emitter,
                        &batch_config,
                    )
                    .await?;
                }
            } else {
                info!(
                    "Client {} subscribed to {} without snapshot",
                    ctx.client_id, view_id
                );
            }

            let client_id = ctx.client_id;
            let client_mgr = ctx.client_manager.clone();
            let usage_emitter = ctx.usage_emitter.clone();
            let sub = subscription.clone();
            let view_id_clone = view_id.clone();
            let view_id_span = view_id.clone();
            let mode = view_spec.mode;
            tokio::spawn(
                async move {
                    loop {
                        tokio::select! {
                            _ = cancel_token.cancelled() => {
                                debug!("List subscription cancelled for client {}", client_id);
                                break;
                            }
                            result = rx.recv() => {
                                match result {
                                    Ok(envelope) => {
                                        if sub.matches(&envelope.entity, &envelope.key)
                                            && client_mgr
                                                .send_to_client(client_id, envelope.payload.clone())
                                                .is_err()
                                        {
                                            break;
                                        } else if sub.matches(&envelope.entity, &envelope.key) {
                                            emit_update_sent_for_client(
                                                &usage_emitter,
                                                &client_mgr,
                                                client_id,
                                                &view_id_clone,
                                                envelope.payload.len(),
                                            );
                                        }
                                    }
                                    Err(_) => break,
                                }
                            }
                        }
                    }
                }
                .instrument(
                    info_span!("ws.subscribe.list", %client_id, view = %view_id_span, mode = ?mode),
                ),
            );
        }
    }

    info!(
        "Client {} subscribed to {} (mode: {:?})",
        ctx.client_id, view_id, view_spec.mode
    );

    Ok(())
}

#[cfg(not(feature = "otel"))]
async fn attach_derived_view_subscription(
    ctx: &SubscriptionContext<'_>,
    subscription: Subscription,
    view_spec: ViewSpec,
    cancel_token: CancellationToken,
) -> Result<()> {
    let view_id = &subscription.view;
    let pipeline_limit = view_spec
        .pipeline
        .as_ref()
        .and_then(|p| p.limit)
        .unwrap_or(100);
    let take = subscription.take.unwrap_or(pipeline_limit);
    let skip = subscription.skip.unwrap_or(0);
    let is_single = take == 1;

    let source_view_id = match &view_spec.source_view {
        Some(s) => s.clone(),
        None => {
            return Err(anyhow::anyhow!(
                "Derived view {} has no source_view",
                view_id
            ));
        }
    };

    let sorted_caches = ctx.view_index.sorted_caches();
    let initial_window: Vec<(String, serde_json::Value)> = {
        let mut caches = sorted_caches.write().await;
        if let Some(cache) = caches.get_mut(view_id) {
            cache.get_window(skip, take)
        } else {
            warn!("No sorted cache for derived view {}", view_id);
            vec![]
        }
    };

    let initial_keys: HashSet<String> = initial_window.iter().map(|(k, _)| k.clone()).collect();

    if !initial_window.is_empty() {
        let snapshot_entities: Vec<SnapshotEntity> = initial_window
            .into_iter()
            .map(|(key, mut data)| {
                transform_large_u64_to_strings(&mut data);
                SnapshotEntity { key, data }
            })
            .collect();

        enforce_snapshot_limit(ctx, snapshot_entities.len())?;
        let batch_config = ctx.entity_cache.snapshot_config();
        send_snapshot_batches(
            ctx.client_id,
            &snapshot_entities,
            view_spec.mode,
            view_id,
            ctx.client_manager,
            ctx.usage_emitter,
            &batch_config,
        )
        .await?;
    }

    let mut rx = ctx
        .bus_manager
        .get_or_create_list_bus(&source_view_id)
        .await;

    let client_id = ctx.client_id;
    let client_mgr = ctx.client_manager.clone();
    let usage_emitter = ctx.usage_emitter.clone();
    let view_id_clone = view_id.clone();
    let view_id_span = view_id.clone();
    let sorted_caches_clone = sorted_caches;
    let frame_mode = view_spec.mode;

    tokio::spawn(
        async move {
            let mut current_window_keys = initial_keys;

            loop {
                tokio::select! {
                    _ = cancel_token.cancelled() => {
                        debug!("Derived view subscription cancelled for client {}", client_id);
                        break;
                    }
                    result = rx.recv() => {
                        match result {
                            Ok(_envelope) => {
                                let new_window: Vec<(String, serde_json::Value)> = {
                                    let mut caches = sorted_caches_clone.write().await;
                                    if let Some(cache) = caches.get_mut(&view_id_clone) {
                                        cache.get_window(skip, take)
                                    } else {
                                        continue;
                                    }
                                };

                                let new_keys: HashSet<String> =
                                    new_window.iter().map(|(k, _)| k.clone()).collect();

                                if is_single {
                                    if let Some((new_key, data)) = new_window.first() {
                                        for old_key in current_window_keys.difference(&new_keys) {
                                            let delete_frame = Frame {
                                            seq: None,
                                                mode: frame_mode,
                                                export: view_id_clone.clone(),
                                                op: "delete",
                                                key: old_key.clone(),
                                                data: serde_json::Value::Null,
                                                append: vec![],
                                            };
                                            if let Ok(json) = serde_json::to_vec(&delete_frame) {
                                                let payload = Arc::new(Bytes::from(json));
                                                let payload_len = payload.len();
                                                if client_mgr.send_to_client(client_id, payload).is_err() {
                                                    return;
                                                }
                                                emit_update_sent_for_client(
                                                    &usage_emitter,
                                                    &client_mgr,
                                                    client_id,
                                                    &view_id_clone,
                                                    payload_len,
                                                );
                                            }
                                        }

                                        let mut transformed_data = data.clone();
                                        transform_large_u64_to_strings(&mut transformed_data);
                                        let frame = Frame {
                                            seq: None,
                                            mode: frame_mode,
                                            export: view_id_clone.clone(),
                                            op: "upsert",
                                            key: new_key.clone(),
                                            data: transformed_data,
                                            append: vec![],
                                        };
                                        if let Ok(json) = serde_json::to_vec(&frame) {
                                            let payload = Arc::new(Bytes::from(json));
                                            let payload_len = payload.len();
                                            if client_mgr.send_to_client(client_id, payload).is_err() {
                                                return;
                                            }
                                            emit_update_sent_for_client(
                                                &usage_emitter,
                                                &client_mgr,
                                                client_id,
                                                &view_id_clone,
                                                payload_len,
                                            );
                                        }
                                    }
                                } else {
                                    for key in current_window_keys.difference(&new_keys) {
                                        let delete_frame = Frame {
                                            seq: None,
                                            mode: frame_mode,
                                            export: view_id_clone.clone(),
                                            op: "delete",
                                            key: key.clone(),
                                            data: serde_json::Value::Null,
                                            append: vec![],
                                        };
                                        if let Ok(json) = serde_json::to_vec(&delete_frame) {
                                            let payload = Arc::new(Bytes::from(json));
                                            let payload_len = payload.len();
                                            if client_mgr.send_to_client(client_id, payload).is_err() {
                                                return;
                                            }
                                            emit_update_sent_for_client(
                                                &usage_emitter,
                                                &client_mgr,
                                                client_id,
                                                &view_id_clone,
                                                payload_len,
                                            );
                                        }
                                    }

                                    for (key, data) in &new_window {
                                        let mut transformed_data = data.clone();
                                        transform_large_u64_to_strings(&mut transformed_data);
                                        let frame = Frame {
                                            seq: None,
                                            mode: frame_mode,
                                            export: view_id_clone.clone(),
                                            op: "upsert",
                                            key: key.clone(),
                                            data: transformed_data,
                                            append: vec![],
                                        };
                                        if let Ok(json) = serde_json::to_vec(&frame) {
                                            let payload = Arc::new(Bytes::from(json));
                                            let payload_len = payload.len();
                                            if client_mgr.send_to_client(client_id, payload).is_err() {
                                                return;
                                            }
                                            emit_update_sent_for_client(
                                                &usage_emitter,
                                                &client_mgr,
                                                client_id,
                                                &view_id_clone,
                                                payload_len,
                                            );
                                        }
                                    }
                                }

                                current_window_keys = new_keys;
                            }
                            Err(_) => break,
                        }
                    }
                }
            }
        }
        .instrument(info_span!("ws.subscribe.derived", %client_id, view = %view_id_span)),
    );

    info!(
        "Client {} subscribed to derived view {} (take={}, skip={})",
        ctx.client_id, view_id, take, skip
    );

    Ok(())
}
