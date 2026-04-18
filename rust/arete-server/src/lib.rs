//! # arete-server
//!
//! WebSocket server and projection handlers for Arete streaming pipelines.
//!
//! This crate provides a builder API for creating Arete servers that:
//!
//! - Process Solana blockchain data via Yellowstone gRPC
//! - Transform data using the Arete VM
//! - Stream entity updates over WebSockets to connected clients
//! - Support multiple streaming modes (State, List, Append)
//!
//! ## Quick Start
//!
//! ```rust,ignore
//! use arete_server::{Server, Spec};
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     Server::builder()
//!         .spec(my_spec())
//!         .websocket()
//!         .bind("[::]:8877".parse()?)
//!         .health_monitoring()
//!         .start()
//!         .await
//! }
//! ```
//!
//! ## Feature Flags
//!
//! - `otel` - OpenTelemetry integration for metrics and distributed tracing

pub mod bus;
pub mod cache;
pub mod compression;
pub mod config;
pub mod health;
pub mod http_health;
pub mod materialized_view;
#[cfg(feature = "otel")]
pub mod metrics;
pub mod mutation_batch;
pub mod projector;
pub mod runtime;
pub mod sorted_cache;
pub mod telemetry;
pub mod view;
pub mod websocket;

pub use bus::{BusManager, BusMessage};
pub use cache::{EntityCache, EntityCacheConfig};
pub use config::{
    HealthConfig, HttpHealthConfig, ReconnectionConfig, ServerConfig, WebSocketConfig,
    YellowstoneConfig,
};
pub use health::{HealthMonitor, SlotTracker, StreamStatus};
pub use http_health::HttpHealthServer;
pub use arete_auth::{AsyncVerifier, KeyLoader, Limits, TokenVerifier, VerifyingKey};
pub use materialized_view::{MaterializedView, MaterializedViewRegistry, ViewEffect};
#[cfg(feature = "otel")]
pub use metrics::Metrics;
pub use mutation_batch::{EventContext, MutationBatch, SlotContext};
pub use projector::Projector;
pub use runtime::Runtime;
pub use telemetry::{init as init_telemetry, TelemetryConfig};
#[cfg(feature = "otel")]
pub use telemetry::{init_with_otel, TelemetryGuard};
pub use view::{Delivery, Filters, Projection, ViewIndex, ViewSpec};
pub use websocket::{
    AllowAllAuthPlugin, AuthContext, AuthDecision, AuthDeny, AuthErrorDetails, ChannelUsageEmitter,
    ClientInfo, ClientManager, ConnectionAuthRequest, ErrorResponse, Frame, HttpUsageEmitter, Mode,
    RateLimitConfig, RateLimitResult, RateLimiterConfig, RefreshAuthRequest, RefreshAuthResponse,
    RetryPolicy, SignedSessionAuthPlugin, SocketIssueMessage, StaticTokenAuthPlugin, Subscription,
    WebSocketAuthPlugin, WebSocketRateLimiter, WebSocketServer, WebSocketUsageBatch,
    WebSocketUsageEmitter, WebSocketUsageEnvelope, WebSocketUsageEvent,
};

use anyhow::Result;
use arete_interpreter::ast::ViewDef;
use std::net::SocketAddr;
use std::sync::Arc;

/// Type alias for a parser setup function.
pub type ParserSetupFn = Arc<
    dyn Fn(
            tokio::sync::mpsc::Sender<MutationBatch>,
            Option<HealthMonitor>,
            ReconnectionConfig,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send>>
        + Send
        + Sync,
>;

/// Specification for a Arete server
/// Contains bytecode, parsers, and program information
pub struct Spec {
    pub bytecode: arete_interpreter::compiler::MultiEntityBytecode,
    pub program_ids: Vec<String>,
    pub parser_setup: Option<ParserSetupFn>,
    pub views: Vec<ViewDef>,
}

impl Spec {
    pub fn new(
        bytecode: arete_interpreter::compiler::MultiEntityBytecode,
        program_id: impl Into<String>,
    ) -> Self {
        Self {
            bytecode,
            program_ids: vec![program_id.into()],
            parser_setup: None,
            views: Vec::new(),
        }
    }

    pub fn with_parser_setup(mut self, setup_fn: ParserSetupFn) -> Self {
        self.parser_setup = Some(setup_fn);
        self
    }

    pub fn with_views(mut self, views: Vec<ViewDef>) -> Self {
        self.views = views;
        self
    }
}

/// Main server interface with fluent builder API
pub struct Server;

impl Server {
    /// Create a new server builder
    pub fn builder() -> ServerBuilder {
        ServerBuilder::new()
    }
}

/// Builder for configuring and creating a Arete server
pub struct ServerBuilder {
    spec: Option<Spec>,
    views: Option<ViewIndex>,
    materialized_views: Option<MaterializedViewRegistry>,
    config: ServerConfig,
    websocket_auth_plugin: Option<Arc<dyn WebSocketAuthPlugin>>,
    websocket_usage_emitter: Option<Arc<dyn WebSocketUsageEmitter>>,
    websocket_max_clients: Option<usize>,
    websocket_rate_limit_config: Option<crate::websocket::client_manager::RateLimitConfig>,
    #[cfg(feature = "otel")]
    metrics: Option<Arc<Metrics>>,
}

impl ServerBuilder {
    fn new() -> Self {
        Self {
            spec: None,
            views: None,
            materialized_views: None,
            config: ServerConfig::new(),
            websocket_auth_plugin: None,
            websocket_usage_emitter: None,
            websocket_max_clients: None,
            websocket_rate_limit_config: None,
            #[cfg(feature = "otel")]
            metrics: None,
        }
    }

    /// Set the specification (bytecode, parsers, program_ids)
    pub fn spec(mut self, spec: Spec) -> Self {
        self.spec = Some(spec);
        self
    }

    /// Set custom view index
    pub fn views(mut self, views: ViewIndex) -> Self {
        self.views = Some(views);
        self
    }

    /// Enable metrics collection (requires 'otel' feature)
    #[cfg(feature = "otel")]
    pub fn metrics(mut self, metrics: Metrics) -> Self {
        self.metrics = Some(Arc::new(metrics));
        self
    }

    /// Enable WebSocket server with default configuration
    pub fn websocket(mut self) -> Self {
        self.config.websocket = Some(WebSocketConfig::default());
        self
    }

    /// Configure WebSocket server
    pub fn websocket_config(mut self, config: WebSocketConfig) -> Self {
        self.config.websocket = Some(config);
        self
    }

    /// Set a WebSocket auth plugin used to authorize inbound connections.
    pub fn websocket_auth_plugin(mut self, plugin: Arc<dyn WebSocketAuthPlugin>) -> Self {
        self.websocket_auth_plugin = Some(plugin);
        self
    }

    /// Set an async usage emitter for billing-grade websocket usage events.
    pub fn websocket_usage_emitter(mut self, emitter: Arc<dyn WebSocketUsageEmitter>) -> Self {
        self.websocket_usage_emitter = Some(emitter);
        self
    }

    /// Set the maximum number of concurrent WebSocket clients.
    pub fn websocket_max_clients(mut self, max_clients: usize) -> Self {
        self.websocket_max_clients = Some(max_clients);
        self
    }

    /// Configure rate limiting for WebSocket connections.
    ///
    /// This sets global rate limits such as maximum connections per IP,
    /// timeouts, and rate windows. Per-subject limits are controlled
    /// via AuthContext.Limits from the authentication token.
    pub fn websocket_rate_limit_config(
        mut self,
        config: crate::websocket::client_manager::RateLimitConfig,
    ) -> Self {
        self.websocket_rate_limit_config = Some(config);
        self
    }

    /// Set the bind address for WebSocket server
    pub fn bind(mut self, addr: impl Into<SocketAddr>) -> Self {
        if let Some(ws_config) = &mut self.config.websocket {
            ws_config.bind_address = addr.into();
        } else {
            self.config.websocket = Some(WebSocketConfig::new(addr.into()));
        }
        self
    }

    /// Configure Yellowstone gRPC connection
    pub fn yellowstone(mut self, config: YellowstoneConfig) -> Self {
        self.config.yellowstone = Some(config);
        self
    }

    /// Enable health monitoring with default configuration
    pub fn health_monitoring(mut self) -> Self {
        self.config.health = Some(HealthConfig::default());
        self
    }

    /// Configure health monitoring
    pub fn health_config(mut self, config: HealthConfig) -> Self {
        self.config.health = Some(config);
        self
    }

    /// Enable reconnection with default configuration
    pub fn reconnection(mut self) -> Self {
        self.config.reconnection = Some(ReconnectionConfig::default());
        self
    }

    /// Configure reconnection behavior
    pub fn reconnection_config(mut self, config: ReconnectionConfig) -> Self {
        self.config.reconnection = Some(config);
        self
    }

    /// Enable HTTP health server with default configuration (port 8081)
    pub fn http_health(mut self) -> Self {
        self.config.http_health = Some(HttpHealthConfig::default());
        self
    }

    /// Configure HTTP health server
    pub fn http_health_config(mut self, config: HttpHealthConfig) -> Self {
        self.config.http_health = Some(config);
        self
    }

    /// Set the bind address for HTTP health server
    pub fn health_bind(mut self, addr: impl Into<SocketAddr>) -> Self {
        if let Some(http_config) = &mut self.config.http_health {
            http_config.bind_address = addr.into();
        } else {
            self.config.http_health = Some(HttpHealthConfig::new(addr.into()));
        }
        self
    }

    pub async fn start(self) -> Result<()> {
        let (view_index, materialized_registry) =
            Self::build_view_index_and_registry(self.views, self.materialized_views, &self.spec);

        #[cfg(feature = "otel")]
        let mut runtime = Runtime::new(self.config, view_index, self.metrics);
        #[cfg(not(feature = "otel"))]
        let mut runtime = Runtime::new(self.config, view_index);

        if let Some(plugin) = self.websocket_auth_plugin {
            runtime = runtime.with_websocket_auth_plugin(plugin);
        }

        if let Some(emitter) = self.websocket_usage_emitter {
            runtime = runtime.with_websocket_usage_emitter(emitter);
        }

        if let Some(max_clients) = self.websocket_max_clients {
            runtime = runtime.with_websocket_max_clients(max_clients);
        }

        if let Some(rate_limit_config) = self.websocket_rate_limit_config {
            runtime = runtime.with_websocket_rate_limit_config(rate_limit_config);
        }

        if let Some(registry) = materialized_registry {
            runtime = runtime.with_materialized_views(registry);
        }

        if let Some(spec) = self.spec {
            runtime = runtime.with_spec(spec);
        }

        runtime.run().await
    }

    fn build_view_index_and_registry(
        views: Option<ViewIndex>,
        materialized_views: Option<MaterializedViewRegistry>,
        spec: &Option<Spec>,
    ) -> (ViewIndex, Option<MaterializedViewRegistry>) {
        let mut index = views.unwrap_or_default();
        let mut registry = materialized_views;

        if let Some(ref spec) = spec {
            for entity_name in spec.bytecode.entities.keys() {
                index.add_spec(ViewSpec {
                    id: format!("{}/list", entity_name),
                    export: entity_name.clone(),
                    mode: Mode::List,
                    projection: Projection::all(),
                    filters: Filters::all(),
                    delivery: Delivery::default(),
                    pipeline: None,
                    source_view: None,
                });

                index.add_spec(ViewSpec {
                    id: format!("{}/state", entity_name),
                    export: entity_name.clone(),
                    mode: Mode::State,
                    projection: Projection::all(),
                    filters: Filters::all(),
                    delivery: Delivery::default(),
                    pipeline: None,
                    source_view: None,
                });

                index.add_spec(ViewSpec {
                    id: format!("{}/append", entity_name),
                    export: entity_name.clone(),
                    mode: Mode::Append,
                    projection: Projection::all(),
                    filters: Filters::all(),
                    delivery: Delivery::default(),
                    pipeline: None,
                    source_view: None,
                });
            }

            if !spec.views.is_empty() {
                let reg = registry.get_or_insert_with(MaterializedViewRegistry::new);

                for view_def in &spec.views {
                    let export = match &view_def.source {
                        arete_interpreter::ast::ViewSource::Entity { name } => name.clone(),
                        arete_interpreter::ast::ViewSource::View { id } => {
                            id.split('/').next().unwrap_or(id).to_string()
                        }
                    };

                    let view_spec = ViewSpec::from_view_def(view_def, &export);
                    let pipeline = view_spec.pipeline.clone().unwrap_or_default();
                    let source_id = view_spec.source_view.clone().unwrap_or_default();
                    tracing::debug!(
                        view_id = %view_def.id,
                        source = %source_id,
                        "Registering derived view"
                    );

                    index.add_spec(view_spec);

                    let materialized =
                        MaterializedView::new(view_def.id.clone(), source_id, pipeline);
                    reg.register(materialized);
                }
            }
        }

        (index, registry)
    }

    pub fn build(self) -> Result<Runtime> {
        let (view_index, materialized_registry) =
            Self::build_view_index_and_registry(self.views, self.materialized_views, &self.spec);

        #[cfg(feature = "otel")]
        let mut runtime = Runtime::new(self.config, view_index, self.metrics);
        #[cfg(not(feature = "otel"))]
        let mut runtime = Runtime::new(self.config, view_index);

        if let Some(plugin) = self.websocket_auth_plugin {
            runtime = runtime.with_websocket_auth_plugin(plugin);
        }

        if let Some(max_clients) = self.websocket_max_clients {
            runtime = runtime.with_websocket_max_clients(max_clients);
        }

        if let Some(registry) = materialized_registry {
            runtime = runtime.with_materialized_views(registry);
        }

        if let Some(spec) = self.spec {
            runtime = runtime.with_spec(spec);
        }
        Ok(runtime)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builder_pattern() {
        let _builder = Server::builder()
            .websocket()
            .bind("[::]:8877".parse::<SocketAddr>().unwrap());
    }

    #[test]
    fn test_spec_creation() {
        let bytecode = arete_interpreter::compiler::MultiEntityBytecode::new().build();
        let spec = Spec::new(bytecode, "test_program");
        assert_eq!(
            spec.program_ids.first().map(String::as_str),
            Some("test_program")
        );
    }
}
