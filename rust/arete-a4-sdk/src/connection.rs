use crate::auth::{
    build_websocket_url, hosted_auth_required_error, parse_jwt_expiry, token_is_expiring,
    token_refresh_delay, AuthConfig, AuthToken, ResolvedAuthStrategy, TokenEndpointRequest,
    TokenEndpointResponse, TokenTransport, MIN_REFRESH_DELAY_SECONDS,
};
use crate::config::ConnectionConfig;
use crate::error::{AreteError, SocketIssue};
use crate::frame::{parse_server_message, ProtocolErrorFrame, ServerMessage};
use crate::store::SharedStore;
use crate::subscription::{
    ClientMessage, SnapshotOptions, Subscription, SubscriptionQuery, SubscriptionRegistry,
    Unsubscription,
};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{broadcast, mpsc, oneshot, RwLock};
use tokio::time::{sleep, Sleep};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{
        client::IntoClientRequest,
        http::{HeaderName, HeaderValue},
        Message,
    },
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
    Release(String),
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
    pub partition: Option<String>,
    pub filters: BTreeMap<String, Value>,
    pub take: Option<usize>,
    pub skip: Option<usize>,
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
    command_tx: mpsc::UnboundedSender<ConnectionCommand>,
    last_error: Arc<RwLock<Option<Arc<AreteError>>>>,
    last_socket_issue: Arc<RwLock<Option<SocketIssue>>>,
    socket_issue_tx: broadcast::Sender<SocketIssue>,
    store: SharedStore,
    /// HTTP-only mode: no socket is ever opened and subscription acquisition
    /// fails with [`AreteError::WebSocketDisabled`].
    disabled: bool,
}

#[derive(Clone)]
pub struct ConnectionManager {
    inner: Arc<ConnectionManagerInner>,
}

pub struct SubscriptionLease {
    subscription_id: String,
    command_tx: mpsc::UnboundedSender<ConnectionCommand>,
    released: bool,
}

impl SubscriptionLease {
    pub fn subscription_id(&self) -> &str {
        &self.subscription_id
    }

    pub fn release(mut self) {
        self.release_inner();
    }

    fn release_inner(&mut self) {
        if self.released {
            return;
        }
        self.released = true;
        if let Err(error) = self
            .command_tx
            .send(ConnectionCommand::Release(self.subscription_id.clone()))
        {
            tracing::warn!(
                subscription_id = %self.subscription_id,
                %error,
                "failed to queue protocol v2 unsubscribe"
            );
        }
    }
}

impl Drop for SubscriptionLease {
    fn drop(&mut self) {
        self.release_inner();
    }
}

impl ConnectionManager {
    pub async fn new(
        url: String,
        config: ConnectionConfig,
        store: SharedStore,
    ) -> Result<Self, AreteError> {
        let (command_tx, command_rx) = mpsc::unbounded_channel();
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
            store: store.clone(),
            disabled: false,
        };

        spawn_connection_loop(
            url,
            state,
            subscriptions,
            config,
            store,
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

    /// Construct a manager for HTTP-only clients (`Transport::Http`).
    ///
    /// No connection task is spawned and no socket is ever opened. Any
    /// attempt to acquire a subscription fails fast with
    /// [`AreteError::WebSocketDisabled`] (recorded as the manager's last
    /// error), so view streams terminate instead of hanging. This is the
    /// least invasive HTTP-only construction path: the rest of the client
    /// (store, views wiring) is built exactly as in WebSocket mode.
    pub fn new_disabled(config: ConnectionConfig, store: SharedStore) -> Self {
        let (command_tx, _command_rx) = mpsc::unbounded_channel();
        let (socket_issue_tx, _) = broadcast::channel(1);
        Self {
            inner: Arc::new(ConnectionManagerInner {
                url: String::new(),
                state: Arc::new(RwLock::new(ConnectionState::Disconnected)),
                subscriptions: Arc::new(RwLock::new(SubscriptionRegistry::new())),
                config,
                command_tx,
                last_error: Arc::new(RwLock::new(None)),
                last_socket_issue: Arc::new(RwLock::new(None)),
                socket_issue_tx,
                store,
                disabled: true,
            }),
        }
    }

    /// Whether this manager was constructed HTTP-only (no socket).
    pub fn is_disabled(&self) -> bool {
        self.inner.disabled
    }

    async fn reject_when_disabled(&self) -> Result<(), AreteError> {
        if !self.inner.disabled {
            return Ok(());
        }
        let error = AreteError::WebSocketDisabled;
        set_last_error(&self.inner.last_error, error.clone()).await;
        Err(error)
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

    pub async fn ensure_subscription(
        &self,
        view: &str,
        key: Option<&str>,
    ) -> Result<SubscriptionLease, AreteError> {
        self.ensure_subscription_with_opts(view, key, SubscriptionOptions::default())
            .await
    }

    pub async fn ensure_subscription_with_opts(
        &self,
        view: &str,
        key: Option<&str>,
        opts: SubscriptionOptions,
    ) -> Result<SubscriptionLease, AreteError> {
        let mut query = SubscriptionQuery::new(view);
        query.key = key.map(str::to_string);
        query.partition = opts.partition;
        query.filters = opts.filters;
        query.take = opts.take;
        query.skip = opts.skip;
        query.after = opts.after;
        query.snapshot_limit = opts.snapshot_limit;
        self.acquire_query(
            query,
            SnapshotOptions {
                enabled: opts.with_snapshot.unwrap_or(true),
            },
        )
        .await
    }

    pub async fn acquire_query(
        &self,
        query: SubscriptionQuery,
        snapshot: SnapshotOptions,
    ) -> Result<SubscriptionLease, AreteError> {
        self.reject_when_disabled().await?;
        let (subscription, is_new) = self
            .inner
            .subscriptions
            .write()
            .await
            .acquire(query, snapshot)?;
        self.finish_acquire(subscription, is_new).await
    }

    pub async fn subscribe(
        &self,
        subscription: Subscription,
    ) -> Result<SubscriptionLease, AreteError> {
        self.reject_when_disabled().await?;
        let (subscription, is_new) = self
            .inner
            .subscriptions
            .write()
            .await
            .acquire_explicit(subscription)?;
        self.finish_acquire(subscription, is_new).await
    }

    async fn finish_acquire(
        &self,
        subscription: Subscription,
        is_new: bool,
    ) -> Result<SubscriptionLease, AreteError> {
        if is_new {
            if let Err(error) = self
                .inner
                .store
                .register_subscription(
                    &subscription.subscription_id,
                    subscription.query.clone(),
                    subscription.snapshot.enabled,
                )
                .await
            {
                self.inner
                    .subscriptions
                    .write()
                    .await
                    .remove(&subscription.subscription_id);
                return Err(error);
            }
            if let Err(error) = self
                .inner
                .command_tx
                .send(ConnectionCommand::Subscribe(subscription.clone()))
            {
                self.inner
                    .subscriptions
                    .write()
                    .await
                    .remove(&subscription.subscription_id);
                self.inner
                    .store
                    .unregister_subscription(&subscription.subscription_id)
                    .await;
                return Err(AreteError::ChannelError(error.to_string()));
            }
        }
        Ok(SubscriptionLease {
            subscription_id: subscription.subscription_id,
            command_tx: self.inner.command_tx.clone(),
            released: false,
        })
    }

    pub async fn unsubscribe(&self, unsubscription: Unsubscription) -> Result<(), AreteError> {
        unsubscription.validate()?;
        self.inner
            .command_tx
            .send(ConnectionCommand::Unsubscribe(unsubscription))
            .map_err(|error| AreteError::ChannelError(error.to_string()))
    }

    pub async fn disconnect(&self) {
        let _ = self.inner.command_tx.send(ConnectionCommand::Disconnect);
    }
}

struct RuntimeAuthState {
    websocket_url: String,
    config: Option<AuthConfig>,
    handshake_headers: HashMap<String, String>,
    current_token: Option<String>,
    token_expiry: Option<u64>,
    http_client: reqwest::Client,
}

impl RuntimeAuthState {
    fn new(
        websocket_url: String,
        config: Option<AuthConfig>,
        handshake_headers: HashMap<String, String>,
    ) -> Self {
        Self {
            websocket_url,
            config,
            handshake_headers,
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

        // Apply user-supplied handshake headers first. The SDK-managed
        // Authorization header (Bearer transport, below) overwrites any
        // user-provided `authorization` to keep auth state consistent.
        for (key, value) in &self.handshake_headers {
            let header_name = HeaderName::from_bytes(key.as_bytes())
                .map_err(|error| AreteError::ConnectionFailed(error.to_string()))?;
            let header_value = HeaderValue::from_str(value)
                .map_err(|error| AreteError::ConnectionFailed(error.to_string()))?;
            request.headers_mut().insert(header_name, header_value);
        }

        if self.token_transport() == TokenTransport::Bearer {
            if let Some(token) = token {
                let header_value = HeaderValue::from_str(&format!("Bearer {token}"))
                    .map_err(|error| AreteError::ConnectionFailed(error.to_string()))?;
                request.headers_mut().insert("Authorization", header_value);
            }
        }

        // Auto-forward Origin from token_endpoint_headers when no explicit
        // handshake Origin was set. Browsers add Origin automatically; for
        // origin-bound publishable keys, Rust clients otherwise have to set
        // the same Origin twice (mint + upgrade). HTTP header names are
        // case-insensitive on the wire, so match the user's key regardless
        // of its capitalisation.
        if !request.headers().contains_key("origin") {
            if let Some(origin) = self.config.as_ref().and_then(|c| {
                c.token_endpoint_headers
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("origin"))
                    .map(|(_, v)| v.as_str())
            }) {
                let header_value = HeaderValue::from_str(origin)
                    .map_err(|error| AreteError::ConnectionFailed(error.to_string()))?;
                request.headers_mut().insert("Origin", header_value);
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
    store: SharedStore,
    mut command_rx: mpsc::UnboundedReceiver<ConnectionCommand>,
    last_error: Arc<RwLock<Option<Arc<AreteError>>>>,
    last_socket_issue: Arc<RwLock<Option<SocketIssue>>>,
    socket_issue_tx: broadcast::Sender<SocketIssue>,
    initial_connect_tx: oneshot::Sender<Result<(), AreteError>>,
) {
    tokio::spawn(async move {
        let mut auth_state = RuntimeAuthState::new(
            url.clone(),
            config.auth.clone(),
            config.handshake_headers.clone(),
        );
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
                        store.begin_refresh(&sub.subscription_id).await;
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
                                        match process_server_payload(&bytes, &store).await {
                                            Ok(Some(issue)) => {
                                                record_socket_issue(&last_socket_issue, &socket_issue_tx, issue.clone()).await;
                                                let error = AreteError::from_socket_issue(issue);
                                                let is_fatal = error.socket_issue().is_some_and(|issue| issue.fatal);
                                                set_last_error(&last_error, error).await;
                                                if is_fatal {
                                                    break;
                                                }
                                            }
                                            Ok(None) => {}
                                            Err(error) => {
                                                set_last_error(&last_error, error).await;
                                                break;
                                            }
                                        }
                                    }
                                    Some(Ok(Message::Text(text))) => {
                                        if let Some(refresh_response) = parse_refresh_auth_response(&text) {
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
                                        } else {
                                            match process_server_payload(text.as_bytes(), &store).await {
                                                Ok(Some(issue)) => {
                                                    record_socket_issue(&last_socket_issue, &socket_issue_tx, issue.clone()).await;
                                                    let error = AreteError::from_socket_issue(issue);
                                                    if error.should_refresh_token() && auth_state.has_refreshable_auth() {
                                                        auth_state.clear_cached_token();
                                                        force_token_refresh = true;
                                                        immediate_reconnect = true;
                                                    }
                                                    let is_fatal = error.socket_issue().is_some_and(|issue| issue.fatal);
                                                    set_last_error(&last_error, error).await;
                                                    if is_fatal {
                                                        break;
                                                    }
                                                }
                                                Ok(None) => {}
                                                Err(error) => {
                                                    set_last_error(&last_error, error).await;
                                                    break;
                                                }
                                            }
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
                                        let client_msg = ClientMessage::Subscribe(sub);
                                        if let Ok(msg) = serde_json::to_string(&client_msg) {
                                            let _ = ws_tx.send(Message::Text(msg)).await;
                                        }
                                    }
                                    Some(ConnectionCommand::Release(subscription_id)) => {
                                        let unsub = subscriptions.write().await.release(&subscription_id);
                                        if let Some(unsub) = unsub {
                                            store.unregister_subscription(&subscription_id).await;
                                            let client_msg = ClientMessage::Unsubscribe(unsub);
                                            if let Ok(msg) = serde_json::to_string(&client_msg) {
                                                let _ = ws_tx.send(Message::Text(msg)).await;
                                            }
                                        }
                                    }
                                    Some(ConnectionCommand::Unsubscribe(unsub)) => {
                                        subscriptions.write().await.remove(&unsub.subscription_id);
                                        store.unregister_subscription(&unsub.subscription_id).await;
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
                } else if !error.should_retry()
                    && error.socket_issue().is_none_or(|issue| issue.fatal)
                {
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

async fn process_server_payload(
    payload: &[u8],
    store: &SharedStore,
) -> Result<Option<SocketIssue>, AreteError> {
    match parse_server_message(payload)? {
        ServerMessage::Frame(frame) => {
            store.apply_frame(frame).await?;
            Ok(None)
        }
        ServerMessage::Error(error) => Ok(Some(protocol_error_to_socket_issue(error))),
    }
}

fn protocol_error_to_socket_issue(error: ProtocolErrorFrame) -> SocketIssue {
    let message = if error.message.is_empty() {
        error.code.clone()
    } else {
        error.message
    };
    let error_name = if error.error.is_empty() {
        error.code.clone()
    } else {
        error.error
    };
    SocketIssue {
        protocol_version: error.protocol_version,
        subscription_id: error.subscription_id,
        error: error_name,
        message,
        wire_code: error.code.clone(),
        code: crate::error::AuthErrorCode::from_wire(&error.code),
        retryable: error.retryable,
        retry_after: error.retry_after,
        suggested_action: error.suggested_action,
        docs_url: error.docs_url,
        fatal: error.fatal,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn auth_state_with(
        config: Option<AuthConfig>,
        handshake_headers: HashMap<String, String>,
    ) -> RuntimeAuthState {
        RuntimeAuthState::new(
            "wss://demo.stack.arete.run/socket".to_string(),
            config,
            handshake_headers,
        )
    }

    #[test]
    fn handshake_header_is_applied_to_upgrade_request() {
        let mut headers = HashMap::new();
        headers.insert("Origin".to_string(), "https://example.gg".to_string());
        headers.insert("X-Custom".to_string(), "value".to_string());
        let state = auth_state_with(None, headers);

        let request = state
            .build_request(None)
            .expect("request should build with handshake headers");

        assert_eq!(
            request
                .headers()
                .get("origin")
                .and_then(|v| v.to_str().ok()),
            Some("https://example.gg")
        );
        assert_eq!(
            request
                .headers()
                .get("x-custom")
                .and_then(|v| v.to_str().ok()),
            Some("value")
        );
    }

    #[test]
    fn origin_is_auto_forwarded_from_token_endpoint_header() {
        let auth = AuthConfig::default()
            .with_publishable_key("hspk_test")
            .with_token_endpoint_header("Origin", "https://example.gg");
        let state = auth_state_with(Some(auth), HashMap::new());

        let request = state
            .build_request(None)
            .expect("request should build with auto-forwarded origin");

        assert_eq!(
            request
                .headers()
                .get("origin")
                .and_then(|v| v.to_str().ok()),
            Some("https://example.gg")
        );
    }

    #[test]
    fn explicit_handshake_origin_overrides_auto_forward() {
        let auth = AuthConfig::default()
            .with_token_endpoint_header("Origin", "https://from-token-endpoint.example");
        let mut handshake = HashMap::new();
        handshake.insert("Origin".to_string(), "https://explicit.example".to_string());
        let state = auth_state_with(Some(auth), handshake);

        let request = state
            .build_request(None)
            .expect("request should build with explicit origin");

        assert_eq!(
            request
                .headers()
                .get("origin")
                .and_then(|v| v.to_str().ok()),
            Some("https://explicit.example")
        );
    }

    #[test]
    fn no_origin_set_when_neither_handshake_nor_token_endpoint_provides_one() {
        let state = auth_state_with(Some(AuthConfig::default()), HashMap::new());

        let request = state
            .build_request(None)
            .expect("request should build without origin");

        assert!(request.headers().get("origin").is_none());
    }

    #[test]
    fn origin_auto_forward_is_case_insensitive() {
        // User supplied a lowercase `origin` key to `token_endpoint_header`.
        // HTTP header names are case-insensitive on the wire, so the
        // auto-forward must find this regardless of capitalisation.
        let auth =
            AuthConfig::default().with_token_endpoint_header("origin", "https://lower.example");
        let state = auth_state_with(Some(auth), HashMap::new());

        let request = state
            .build_request(None)
            .expect("request should build with lowercase-origin auto-forward");

        assert_eq!(
            request
                .headers()
                .get("origin")
                .and_then(|v| v.to_str().ok()),
            Some("https://lower.example")
        );
    }

    #[test]
    fn bearer_authorization_overrides_user_handshake_authorization() {
        // The SDK manages the Bearer Authorization header. A user-provided
        // `authorization` handshake header must not silently replace it.
        let auth = AuthConfig::default().with_token_transport(TokenTransport::Bearer);
        let mut handshake = HashMap::new();
        handshake.insert(
            "authorization".to_string(),
            "Bearer user-supplied".to_string(),
        );
        let state = auth_state_with(Some(auth), handshake);

        let request = state
            .build_request(Some("sdk-managed-token"))
            .expect("request should build with SDK-managed Bearer");

        assert_eq!(
            request
                .headers()
                .get("authorization")
                .and_then(|v| v.to_str().ok()),
            Some("Bearer sdk-managed-token")
        );
    }
}
