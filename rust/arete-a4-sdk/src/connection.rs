use crate::auth::{
    build_websocket_url, hosted_auth_required_error, parse_jwt_expiry, token_is_expiring,
    token_refresh_delay, AuthConfig, AuthToken, ResolvedAuthStrategy, TokenEndpointRequest,
    TokenEndpointResponse, TokenTransport, MIN_REFRESH_DELAY_SECONDS,
};
use crate::config::ConnectionConfig;
use crate::error::{AreteError, SocketIssue, SocketIssuePayload};
use crate::frame::{parse_frame, Frame};
use crate::subscription::{ClientMessage, Subscription, SubscriptionRegistry, Unsubscription};
use futures_util::{SinkExt, StreamExt};
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{broadcast, mpsc, oneshot, RwLock};
use tokio::time::{sleep, Sleep};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, http::HeaderValue, Message},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
    Reconnecting { attempt: u32 },
    Error,
}

pub enum ConnectionCommand {
    Subscribe(Subscription),
    Unsubscribe(Unsubscription),
    Disconnect,
}

#[derive(Debug, serde::Deserialize)]
struct RefreshAuthResponseMessage {
    success: bool,
    error: Option<String>,
    expires_at: Option<u64>,
}

#[derive(Debug, Clone, Default)]
pub struct SubscriptionOptions {
    pub take: Option<u32>,
    pub skip: Option<u32>,
    pub with_snapshot: Option<bool>,
    pub after: Option<String>,
    pub snapshot_limit: Option<usize>,
}

struct ConnectionManagerInner {
    #[allow(dead_code)]
    url: String,
    state: Arc<RwLock<ConnectionState>>,
    subscriptions: Arc<RwLock<SubscriptionRegistry>>,
    #[allow(dead_code)]
    config: ConnectionConfig,
    command_tx: mpsc::Sender<ConnectionCommand>,
    last_error: Arc<RwLock<Option<Arc<AreteError>>>>,
    last_socket_issue: Arc<RwLock<Option<SocketIssue>>>,
    socket_issue_tx: broadcast::Sender<SocketIssue>,
}

#[derive(Clone)]
pub struct ConnectionManager {
    inner: Arc<ConnectionManagerInner>,
}

impl ConnectionManager {
    pub async fn new(
        url: String,
        config: ConnectionConfig,
        frame_tx: mpsc::Sender<Frame>,
    ) -> Result<Self, AreteError> {
        let (command_tx, command_rx) = mpsc::channel(100);
        let (initial_connect_tx, initial_connect_rx) = oneshot::channel();
        let state = Arc::new(RwLock::new(ConnectionState::Disconnected));
        let subscriptions = Arc::new(RwLock::new(SubscriptionRegistry::new()));
        let last_error = Arc::new(RwLock::new(None));
        let last_socket_issue = Arc::new(RwLock::new(None));
        let (socket_issue_tx, _) = broadcast::channel(100);

        let inner = ConnectionManagerInner {
            url: url.clone(),
            state: state.clone(),
            subscriptions: subscriptions.clone(),
            config: config.clone(),
            command_tx,
            last_error: last_error.clone(),
            last_socket_issue: last_socket_issue.clone(),
            socket_issue_tx: socket_issue_tx.clone(),
        };

        spawn_connection_loop(
            url,
            state,
            subscriptions,
            config,
            frame_tx,
            command_rx,
            last_error,
            last_socket_issue,
            socket_issue_tx,
            initial_connect_tx,
        );

        let manager = Self {
            inner: Arc::new(inner),
        };

        match initial_connect_rx.await {
            Ok(Ok(())) => Ok(manager),
            Ok(Err(error)) => Err(error),
            Err(_) => Err(AreteError::ConnectionFailed(
                "Connection task ended before initial connect completed".to_string(),
            )),
        }
    }

    pub async fn state(&self) -> ConnectionState {
        *self.inner.state.read().await
    }

    pub async fn last_error(&self) -> Option<Arc<AreteError>> {
        self.inner.last_error.read().await.clone()
    }

    pub async fn last_socket_issue(&self) -> Option<SocketIssue> {
        self.inner.last_socket_issue.read().await.clone()
    }

    pub fn subscribe_socket_issues(&self) -> broadcast::Receiver<SocketIssue> {
        self.inner.socket_issue_tx.subscribe()
    }

    pub async fn ensure_subscription(&self, view: &str, key: Option<&str>) {
        self.ensure_subscription_with_opts(view, key, SubscriptionOptions::default())
            .await
    }

    pub async fn ensure_subscription_with_opts(
        &self,
        view: &str,
        key: Option<&str>,
        opts: SubscriptionOptions,
    ) {
        let sub = Subscription {
            view: view.to_string(),
            key: key.map(|s| s.to_string()),
            partition: None,
            filters: None,
            take: opts.take,
            skip: opts.skip,
            with_snapshot: opts.with_snapshot,
            after: opts.after,
            snapshot_limit: opts.snapshot_limit,
        };

        if !self.inner.subscriptions.read().await.contains(&sub) {
            let _ = self
                .inner
                .command_tx
                .send(ConnectionCommand::Subscribe(sub))
                .await;
        }
    }

    pub async fn subscribe(&self, sub: Subscription) {
        let _ = self
            .inner
            .command_tx
            .send(ConnectionCommand::Subscribe(sub))
            .await;
    }

    pub async fn unsubscribe(&self, unsub: Unsubscription) {
        let _ = self
            .inner
            .command_tx
            .send(ConnectionCommand::Unsubscribe(unsub))
            .await;
    }

    pub async fn disconnect(&self) {
        let _ = self
            .inner
            .command_tx
            .send(ConnectionCommand::Disconnect)
            .await;
    }
}

struct RuntimeAuthState {
    websocket_url: String,
    config: Option<AuthConfig>,
    current_token: Option<String>,
    token_expiry: Option<u64>,
    http_client: reqwest::Client,
}

impl RuntimeAuthState {
    fn new(websocket_url: String, config: Option<AuthConfig>) -> Self {
        Self {
            websocket_url,
            config,
            current_token: None,
            token_expiry: None,
            http_client: reqwest::Client::new(),
        }
    }

    fn token_transport(&self) -> TokenTransport {
        self.config
            .as_ref()
            .map(|config| config.token_transport)
            .unwrap_or_default()
    }

    fn has_refreshable_auth(&self) -> bool {
        self.config
            .as_ref()
            .is_some_and(|config| config.has_refreshable_auth(&self.websocket_url))
    }

    fn clear_cached_token(&mut self) {
        self.current_token = None;
        self.token_expiry = None;
    }

    fn refresh_timer(&self) -> Option<Pin<Box<Sleep>>> {
        let delay = token_refresh_delay(self.token_expiry, current_unix_timestamp())?;
        Some(Box::pin(sleep(Duration::from_secs(delay))))
    }

    async fn resolve_token(&mut self, force_refresh: bool) -> Result<Option<String>, AreteError> {
        if !force_refresh {
            if let Some(token) = self.current_token.clone() {
                if !token_is_expiring(self.token_expiry, current_unix_timestamp()) {
                    return Ok(Some(token));
                }
            }
        }

        let Some(config) = self.config.as_ref() else {
            if crate::auth::is_hosted_arete_websocket_url(&self.websocket_url) {
                return Err(hosted_auth_required_error());
            }
            return Ok(None);
        };

        let strategy = config.resolve_strategy(&self.websocket_url);
        match strategy {
            ResolvedAuthStrategy::None => {
                if crate::auth::is_hosted_arete_websocket_url(&self.websocket_url) {
                    Err(hosted_auth_required_error())
                } else {
                    Ok(None)
                }
            }
            ResolvedAuthStrategy::StaticToken(token) => {
                self.set_token(AuthToken::new(token)).map(Some)
            }
            ResolvedAuthStrategy::TokenProvider(provider) => {
                let token = provider().await?;
                self.set_token(token).map(Some)
            }
            ResolvedAuthStrategy::TokenEndpoint(endpoint) => {
                let token = self.fetch_token_from_endpoint(&endpoint).await?;
                self.set_token(token).map(Some)
            }
        }
    }

    fn set_token(&mut self, token: AuthToken) -> Result<String, AreteError> {
        let token_value = token.token.trim().to_string();
        if token_value.is_empty() {
            return Err(AreteError::WebSocket {
                message: "Authentication provider returned an empty token".to_string(),
                code: None,
            });
        }

        let expires_at = token.expires_at.or_else(|| parse_jwt_expiry(&token_value));
        if expires_at.is_some() && token_is_expiring(expires_at, current_unix_timestamp()) {
            return Err(AreteError::WebSocket {
                message: "Authentication token is expired".to_string(),
                code: Some(crate::error::AuthErrorCode::TokenExpired),
            });
        }

        self.current_token = Some(token_value.clone());
        self.token_expiry = expires_at;
        Ok(token_value)
    }

    async fn fetch_token_from_endpoint(
        &self,
        token_endpoint: &str,
    ) -> Result<AuthToken, AreteError> {
        let mut request = self
            .http_client
            .post(token_endpoint)
            .json(&TokenEndpointRequest {
                websocket_url: &self.websocket_url,
            });

        if let Some(config) = self.config.as_ref() {
            if let Some(publishable_key) = config.publishable_key.as_ref() {
                request = request.header("Authorization", format!("Bearer {}", publishable_key));
            }

            for (key, value) in &config.token_endpoint_headers {
                request = request.header(key, value);
            }
        }

        let response = request.send().await.map_err(|error| {
            AreteError::ConnectionFailed(format!("Token endpoint request failed: {error}"))
        })?;
        let status = response.status();
        let header_code = response
            .headers()
            .get("X-Error-Code")
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let fallback_message = status.canonical_reason().map(str::to_string);
        let body = response.bytes().await.map_err(|error| {
            AreteError::ConnectionFailed(format!("Failed to read token endpoint response: {error}"))
        })?;

        if !status.is_success() {
            return Err(AreteError::from_auth_response(
                status.as_u16(),
                header_code.as_deref(),
                Some(body.as_ref()),
                fallback_message.as_deref(),
            ));
        }

        let response: TokenEndpointResponse = serde_json::from_slice(body.as_ref())?;
        let token = response.into_auth_token();
        if token.token.trim().is_empty() {
            return Err(AreteError::WebSocket {
                message: "Token endpoint did not return a token".to_string(),
                code: None,
            });
        }

        Ok(token)
    }

    fn build_request(
        &self,
        token: Option<&str>,
    ) -> Result<tokio_tungstenite::tungstenite::http::Request<()>, AreteError> {
        let url = build_websocket_url(&self.websocket_url, token, self.token_transport())?;
        let mut request = url
            .into_client_request()
            .map_err(|error| AreteError::ConnectionFailed(error.to_string()))?;

        if self.token_transport() == TokenTransport::Bearer {
            if let Some(token) = token {
                let header_value = HeaderValue::from_str(&format!("Bearer {token}"))
                    .map_err(|error| AreteError::ConnectionFailed(error.to_string()))?;
                request.headers_mut().insert("Authorization", header_value);
            }
        }

        Ok(request)
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn_connection_loop(
    url: String,
    state: Arc<RwLock<ConnectionState>>,
    subscriptions: Arc<RwLock<SubscriptionRegistry>>,
    config: ConnectionConfig,
    frame_tx: mpsc::Sender<Frame>,
    mut command_rx: mpsc::Receiver<ConnectionCommand>,
    last_error: Arc<RwLock<Option<Arc<AreteError>>>>,
    last_socket_issue: Arc<RwLock<Option<SocketIssue>>>,
    socket_issue_tx: broadcast::Sender<SocketIssue>,
    initial_connect_tx: oneshot::Sender<Result<(), AreteError>>,
) {
    tokio::spawn(async move {
        let mut auth_state = RuntimeAuthState::new(url.clone(), config.auth.clone());
        let mut reconnect_attempt: u32 = 0;
        let mut should_run = true;
        let mut initial_connect_tx = Some(initial_connect_tx);
        let mut force_token_refresh = false;
        let mut immediate_reconnect = false;

        while should_run {
            *state.write().await = ConnectionState::Connecting;

            let token = match auth_state.resolve_token(force_token_refresh).await {
                Ok(token) => {
                    force_token_refresh = false;
                    token
                }
                Err(error) => {
                    set_last_error(&last_error, error.clone()).await;
                    *state.write().await = ConnectionState::Error;
                    report_initial_failure(&mut initial_connect_tx, error);
                    break;
                }
            };

            let request = match auth_state.build_request(token.as_deref()) {
                Ok(request) => request,
                Err(error) => {
                    set_last_error(&last_error, error.clone()).await;
                    *state.write().await = ConnectionState::Error;
                    report_initial_failure(&mut initial_connect_tx, error);
                    break;
                }
            };

            match connect_async(request).await {
                Ok((ws, _)) => {
                    clear_last_error(&last_error).await;
                    *last_socket_issue.write().await = None;
                    *state.write().await = ConnectionState::Connected;
                    reconnect_attempt = 0;
                    immediate_reconnect = false;
                    report_initial_success(&mut initial_connect_tx);

                    let (mut ws_tx, mut ws_rx) = ws.split();
                    let subs = subscriptions.read().await.all();
                    for sub in subs {
                        let client_msg = ClientMessage::Subscribe(sub);
                        if let Ok(msg) = serde_json::to_string(&client_msg) {
                            let _ = ws_tx.send(Message::Text(msg)).await;
                        }
                    }

                    let ping_interval = config.ping_interval;
                    let mut ping_timer = tokio::time::interval(ping_interval);
                    let mut refresh_timer = auth_state.refresh_timer();

                    loop {
                        tokio::select! {
                            msg = ws_rx.next() => {
                                match msg {
                                    Some(Ok(Message::Binary(bytes))) => {
                                        if let Ok(frame) = parse_frame(&bytes) {
                                            let _ = frame_tx.send(frame).await;
                                        }
                                    }
                                    Some(Ok(Message::Text(text))) => {
                                        if let Some(issue) = parse_socket_issue_message(&text) {
                                            record_socket_issue(&last_socket_issue, &socket_issue_tx, issue.clone()).await;

                                            let error = AreteError::from_socket_issue(issue);
                                            if error.should_refresh_token() && auth_state.has_refreshable_auth() {
                                                auth_state.clear_cached_token();
                                                force_token_refresh = true;
                                                immediate_reconnect = true;
                                            }

                                            let is_fatal = error
                                                .socket_issue()
                                                .map(|issue| issue.fatal)
                                                .unwrap_or(false);
                                            set_last_error(&last_error, error).await;

                                            if is_fatal {
                                                break;
                                            }
                                        } else if let Some(refresh_response) = parse_refresh_auth_response(&text) {
                                            if refresh_response.success {
                                                if let Some(expires_at) = refresh_response.expires_at {
                                                    auth_state.token_expiry = Some(expires_at);
                                                }
                                                refresh_timer = auth_state.refresh_timer();
                                            } else {
                                                let error = refresh_response_error(refresh_response);
                                                if error.should_refresh_token() && auth_state.has_refreshable_auth() {
                                                    auth_state.clear_cached_token();
                                                    force_token_refresh = true;
                                                }
                                                immediate_reconnect = true;
                                                set_last_error(&last_error, error).await;
                                                break;
                                            }
                                        } else if let Ok(frame) = serde_json::from_str::<Frame>(&text) {
                                            let _ = frame_tx.send(frame).await;
                                        }
                                    }
                                    Some(Ok(Message::Ping(payload))) => {
                                        let _ = ws_tx.send(Message::Pong(payload)).await;
                                    }
                                    Some(Ok(Message::Close(frame))) => {
                                        if let Some(frame) = frame.as_ref() {
                                            let reason = frame.reason.to_string();
                                            if let Some(error) = AreteError::from_close_reason(&reason) {
                                                if error.should_refresh_token() && auth_state.has_refreshable_auth() {
                                                    auth_state.clear_cached_token();
                                                    force_token_refresh = true;
                                                    immediate_reconnect = true;
                                                }
                                                set_last_error(&last_error, error).await;
                                            }
                                        }
                                        break;
                                    }
                                    Some(Err(error)) => {
                                        let parsed_error = AreteError::from_tungstenite(error);
                                        if parsed_error.should_refresh_token() && auth_state.has_refreshable_auth() {
                                            auth_state.clear_cached_token();
                                            force_token_refresh = true;
                                            immediate_reconnect = true;
                                        }
                                        set_last_error(&last_error, parsed_error).await;
                                        break;
                                    }
                                    None => {
                                        break;
                                    }
                                    _ => {}
                                }
                            }
                            cmd = command_rx.recv() => {
                                match cmd {
                                    Some(ConnectionCommand::Subscribe(sub)) => {
                                        subscriptions.write().await.add(sub.clone());
                                        let client_msg = ClientMessage::Subscribe(sub);
                                        if let Ok(msg) = serde_json::to_string(&client_msg) {
                                            let _ = ws_tx.send(Message::Text(msg)).await;
                                        }
                                    }
                                    Some(ConnectionCommand::Unsubscribe(unsub)) => {
                                        let sub = Subscription {
                                            view: unsub.view.clone(),
                                            key: unsub.key.clone(),
                                            partition: None,
                                            filters: None,
                                            take: None,
                                            skip: None,
                                            with_snapshot: None,
                                            after: None,
                                            snapshot_limit: None,
                                        };
                                        subscriptions.write().await.remove(&sub);
                                        let client_msg = ClientMessage::Unsubscribe(unsub);
                                        if let Ok(msg) = serde_json::to_string(&client_msg) {
                                            let _ = ws_tx.send(Message::Text(msg)).await;
                                        }
                                    }
                                    Some(ConnectionCommand::Disconnect) => {
                                        let _ = ws_tx.close().await;
                                        *state.write().await = ConnectionState::Disconnected;
                                        should_run = false;
                                        break;
                                    }
                                    None => {
                                        should_run = false;
                                        break;
                                    }
                                }
                            }
                            _ = ping_timer.tick() => {
                                if let Ok(msg) = serde_json::to_string(&ClientMessage::Ping) {
                                    let _ = ws_tx.send(Message::Text(msg)).await;
                                }
                            }
                            _ = wait_for_refresh_timer(&mut refresh_timer) => {
                                let previous_token = auth_state.current_token.clone();
                                match auth_state.resolve_token(true).await {
                                    Ok(Some(token)) => {
                                        refresh_timer = auth_state.refresh_timer();
                                        if previous_token.as_deref() != Some(token.as_str()) {
                                            match serde_json::to_string(&ClientMessage::RefreshAuth { token }) {
                                                Ok(message) => {
                                                    if ws_tx.send(Message::Text(message)).await.is_err() {
                                                        immediate_reconnect = true;
                                                        break;
                                                    }
                                                }
                                                Err(error) => {
                                                    tracing::warn!("Failed to serialize auth refresh message: {}", error);
                                                    refresh_timer = Some(Box::pin(sleep(Duration::from_secs(MIN_REFRESH_DELAY_SECONDS))));
                                                }
                                            }
                                        }
                                    }
                                    Ok(None) => {
                                        refresh_timer = None;
                                    }
                                    Err(error) => {
                                        tracing::warn!("Failed to refresh auth token in background: {}", error);
                                        refresh_timer = Some(Box::pin(sleep(Duration::from_secs(MIN_REFRESH_DELAY_SECONDS))));
                                    }
                                }
                            }
                        }
                    }
                }
                Err(error) => {
                    let parsed_error = AreteError::from_tungstenite(error);
                    if parsed_error.should_refresh_token() && auth_state.has_refreshable_auth() {
                        auth_state.clear_cached_token();
                        force_token_refresh = true;
                        immediate_reconnect = true;
                    }
                    tracing::error!("Connection failed: {}", parsed_error);
                    set_last_error(&last_error, parsed_error).await;
                }
            }

            if !should_run {
                break;
            }

            let latest_error = last_error.read().await.clone();
            if let Some(error) = latest_error.as_deref() {
                if error.should_refresh_token() && auth_state.has_refreshable_auth() {
                    auth_state.clear_cached_token();
                    force_token_refresh = true;
                    immediate_reconnect = true;
                } else if !error.should_retry() {
                    *state.write().await = ConnectionState::Error;
                    report_initial_failure(&mut initial_connect_tx, error.clone());
                    break;
                }
            }

            if !config.auto_reconnect {
                *state.write().await = ConnectionState::Error;
                let error = latest_error
                    .as_deref()
                    .cloned()
                    .unwrap_or(AreteError::ConnectionClosed);
                report_initial_failure(&mut initial_connect_tx, error);
                break;
            }

            if reconnect_attempt >= config.max_reconnect_attempts {
                *state.write().await = ConnectionState::Error;
                let error =
                    latest_error
                        .as_deref()
                        .cloned()
                        .unwrap_or(AreteError::MaxReconnectAttempts(
                            config.max_reconnect_attempts,
                        ));
                set_last_error(&last_error, error.clone()).await;
                report_initial_failure(&mut initial_connect_tx, error);
                break;
            }

            let delay = if immediate_reconnect {
                Duration::from_millis(0)
            } else {
                config
                    .reconnect_intervals
                    .get(reconnect_attempt as usize)
                    .copied()
                    .unwrap_or_else(|| {
                        config
                            .reconnect_intervals
                            .last()
                            .copied()
                            .unwrap_or(Duration::from_secs(16))
                    })
            };

            *state.write().await = ConnectionState::Reconnecting {
                attempt: reconnect_attempt,
            };
            reconnect_attempt += 1;

            if !delay.is_zero() {
                tracing::info!(
                    "Reconnecting in {:?} (attempt {})",
                    delay,
                    reconnect_attempt
                );
                sleep(delay).await;
            }
        }

        if let Some(tx) = initial_connect_tx.take() {
            let error = last_error
                .read()
                .await
                .as_deref()
                .cloned()
                .unwrap_or(AreteError::ConnectionClosed);
            let _ = tx.send(Err(error));
        }
    });
}

async fn set_last_error(last_error: &Arc<RwLock<Option<Arc<AreteError>>>>, error: AreteError) {
    *last_error.write().await = Some(Arc::new(error));
}

async fn clear_last_error(last_error: &Arc<RwLock<Option<Arc<AreteError>>>>) {
    *last_error.write().await = None;
}

async fn record_socket_issue(
    last_socket_issue: &Arc<RwLock<Option<SocketIssue>>>,
    socket_issue_tx: &broadcast::Sender<SocketIssue>,
    issue: SocketIssue,
) {
    *last_socket_issue.write().await = Some(issue.clone());
    let _ = socket_issue_tx.send(issue);
}

async fn wait_for_refresh_timer(timer: &mut Option<Pin<Box<Sleep>>>) {
    if let Some(timer) = timer.as_mut() {
        timer.as_mut().await;
    } else {
        futures_util::future::pending::<()>().await;
    }
}

fn report_initial_success(
    initial_connect_tx: &mut Option<oneshot::Sender<Result<(), AreteError>>>,
) {
    if let Some(tx) = initial_connect_tx.take() {
        let _ = tx.send(Ok(()));
    }
}

fn report_initial_failure(
    initial_connect_tx: &mut Option<oneshot::Sender<Result<(), AreteError>>>,
    error: AreteError,
) {
    if let Some(tx) = initial_connect_tx.take() {
        let _ = tx.send(Err(error));
    }
}

fn current_unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn parse_socket_issue_message(text: &str) -> Option<SocketIssue> {
    let payload = serde_json::from_str::<SocketIssuePayload>(text).ok()?;
    if payload.is_socket_issue() {
        Some(payload.into_socket_issue())
    } else {
        None
    }
}

fn parse_refresh_auth_response(text: &str) -> Option<RefreshAuthResponseMessage> {
    let payload = serde_json::from_str::<RefreshAuthResponseMessage>(text).ok()?;
    Some(payload)
}

fn refresh_response_error(response: RefreshAuthResponseMessage) -> AreteError {
    let code = response
        .error
        .as_deref()
        .and_then(crate::error::AuthErrorCode::from_wire);
    let message = response
        .error
        .unwrap_or_else(|| "Authentication refresh failed".to_string());

    AreteError::WebSocket { message, code }
}
