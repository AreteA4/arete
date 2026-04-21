use crate::auth::{AuthConfig, AuthToken, TokenTransport};
use crate::config::{AreteConfig, ConnectionConfig};
use crate::connection::{ConnectionManager, ConnectionState};
use crate::entity::Stack;
use crate::error::{AreteError, SocketIssue};
use crate::frame::Frame;
use crate::store::{SharedStore, StoreConfig};
use crate::view::Views;
use std::future::Future;
use std::marker::PhantomData;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, mpsc};

/// Arete client with typed views access.
///
/// ```ignore
/// use arete_sdk::prelude::*;
/// use arete_stacks::ore::OreStack;
///
/// let a4 = Arete::<OreStack>::connect().await?;
/// let rounds = a4.views.latest().get().await;
/// ```
pub struct Arete<S: Stack> {
    connection: ConnectionManager,
    store: SharedStore,
    #[allow(dead_code)]
    config: AreteConfig,
    pub views: S::Views,
    _stack: PhantomData<S>,
}

impl<S: Stack> Arete<S> {
    /// Connect to the stack's default URL.
    pub async fn connect() -> Result<Self, AreteError> {
        Self::builder().connect().await
    }

    /// Connect with custom URL.
    pub async fn connect_url(url: &str) -> Result<Self, AreteError> {
        Self::builder().url(url).connect().await
    }

    /// Create a builder for custom configuration.
    pub fn builder() -> AreteBuilder<S> {
        AreteBuilder::new()
    }

    pub async fn connection_state(&self) -> ConnectionState {
        self.connection.state().await
    }

    pub async fn last_error(&self) -> Option<Arc<AreteError>> {
        self.connection.last_error().await
    }

    pub async fn last_socket_issue(&self) -> Option<SocketIssue> {
        self.connection.last_socket_issue().await
    }

    pub fn subscribe_socket_issues(&self) -> broadcast::Receiver<SocketIssue> {
        self.connection.subscribe_socket_issues()
    }

    pub async fn disconnect(&self) {
        self.connection.disconnect().await;
    }

    pub fn store(&self) -> &SharedStore {
        &self.store
    }
}

/// Builder for Arete with custom configuration.
pub struct AreteBuilder<S: Stack> {
    url: String,
    config: AreteConfig,
    _stack: PhantomData<S>,
}

impl<S: Stack> AreteBuilder<S> {
    fn new() -> Self {
        Self {
            url: S::url().to_string(),
            config: AreteConfig::default(),
            _stack: PhantomData,
        }
    }

    pub fn url(mut self, url: &str) -> Self {
        self.url = url.to_string();
        self
    }

    pub fn auto_reconnect(mut self, enabled: bool) -> Self {
        self.config.auto_reconnect = enabled;
        self
    }

    pub fn reconnect_intervals(mut self, intervals: Vec<Duration>) -> Self {
        self.config.reconnect_intervals = intervals;
        self
    }

    pub fn max_reconnect_attempts(mut self, max: u32) -> Self {
        self.config.max_reconnect_attempts = max;
        self
    }

    pub fn ping_interval(mut self, interval: Duration) -> Self {
        self.config.ping_interval = interval;
        self
    }

    pub fn initial_data_timeout(mut self, timeout: Duration) -> Self {
        self.config.initial_data_timeout = timeout;
        self
    }

    pub fn max_entries_per_view(mut self, max: usize) -> Self {
        self.config.max_entries_per_view = Some(max);
        self
    }

    pub fn unlimited_entries(mut self) -> Self {
        self.config.max_entries_per_view = None;
        self
    }

    pub fn auth(mut self, auth: AuthConfig) -> Self {
        self.config.auth = Some(auth);
        self
    }

    pub fn auth_token(mut self, token: impl Into<String>) -> Self {
        let auth = self
            .config
            .auth
            .take()
            .unwrap_or_default()
            .with_token(token);
        self.config.auth = Some(auth);
        self
    }

    pub fn publishable_key(mut self, publishable_key: impl Into<String>) -> Self {
        let auth = self
            .config
            .auth
            .take()
            .unwrap_or_default()
            .with_publishable_key(publishable_key);
        self.config.auth = Some(auth);
        self
    }

    /// Alias for `publishable_key` - use this for server-side code where
    /// the key could be either a secret key or a publishable key.
    pub fn api_key(self, api_key: impl Into<String>) -> Self {
        self.publishable_key(api_key)
    }

    pub fn token_endpoint(mut self, token_endpoint: impl Into<String>) -> Self {
        let auth = self
            .config
            .auth
            .take()
            .unwrap_or_default()
            .with_token_endpoint(token_endpoint);
        self.config.auth = Some(auth);
        self
    }

    pub fn token_endpoint_header(
        mut self,
        key: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        let auth = self
            .config
            .auth
            .take()
            .unwrap_or_default()
            .with_token_endpoint_header(key, value);
        self.config.auth = Some(auth);
        self
    }

    pub fn token_transport(mut self, transport: TokenTransport) -> Self {
        let auth = self
            .config
            .auth
            .take()
            .unwrap_or_default()
            .with_token_transport(transport);
        self.config.auth = Some(auth);
        self
    }

    pub fn get_token<F, Fut>(mut self, provider: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<AuthToken, AreteError>> + Send + 'static,
    {
        let auth = self
            .config
            .auth
            .take()
            .unwrap_or_default()
            .with_token_provider(provider);
        self.config.auth = Some(auth);
        self
    }

    pub async fn connect(self) -> Result<Arete<S>, AreteError> {
        let AreteBuilder {
            url,
            config,
            _stack: _,
        } = self;

        let store_config = StoreConfig {
            max_entries_per_view: config.max_entries_per_view,
        };
        let store = SharedStore::with_config(store_config);
        let store_clone = store.clone();

        let (frame_tx, mut frame_rx) = mpsc::channel::<Frame>(1000);

        let connection_config: ConnectionConfig = config.clone().into();
        let connection = ConnectionManager::new(url, connection_config, frame_tx).await?;

        tokio::spawn(async move {
            while let Some(frame) = frame_rx.recv().await {
                store_clone.apply_frame(frame).await;
            }
        });

        let view_builder = crate::view::ViewBuilder::new(
            connection.clone(),
            store.clone(),
            config.initial_data_timeout,
        );
        let views = S::Views::from_builder(view_builder);

        Ok(Arete {
            connection,
            store,
            config,
            views,
            _stack: PhantomData,
        })
    }
}
