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
mod http;
pub mod http_health;
pub mod http_server;
pub mod materialized_view;
#[cfg(feature = "otel")]
pub mod metrics;
pub mod mutation_batch;
pub mod program_runtime;
pub mod projector;
pub mod runtime;
pub mod sorted_cache;
pub mod telemetry;
pub mod view;
pub mod websocket;

pub use arete_auth::{
    AsyncVerifier, KeyLoader, Limits, SolanaGatewayAuthorization, SolanaGatewayAuthorizationError,
    SolanaGatewayScope, TargetKind, TokenVerifier, VerifyingKey, SCOPE_READ,
    SCOPE_TRANSACTION_INSPECT, SCOPE_TRANSACTION_SEND, SOLANA_GATEWAY_AUDIENCE,
};
pub use bus::{BusManager, BusMessage};
pub use cache::{EntityCache, EntityCacheConfig};
pub use config::{
    HealthConfig, HttpHealthConfig, HttpServerConfig, ReconnectionConfig, RuntimePlan,
    ServerConfig, TransactionConfig, WebSocketConfig, YellowstoneConfig,
};
pub use health::{HealthMonitor, SlotTracker, StreamStatus};
pub use http_health::HttpHealthServer;
pub use http_server::HttpServer;
pub use materialized_view::{MaterializedView, MaterializedViewRegistry, ViewEffect};
#[cfg(feature = "otel")]
pub use metrics::Metrics;
pub use mutation_batch::{EventContext, MutationBatch, SlotContext};
pub use program_runtime::{
    IdlContentHash, NormalizedIdlHash, ProgramAccountReaderFn, ProgramReleaseHash,
    ProgramRuntimeCatalog, ProgramRuntimeDefinition, ProgramSpecHash,
};
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
    RetryPolicy, SignedSessionAuthPlugin, SnapshotOptions, SocketIssueMessage,
    StaticTokenAuthPlugin, Subscription, SubscriptionQuery, WebSocketAuthPlugin,
    WebSocketRateLimiter, WebSocketServer, WebSocketUsageBatch, WebSocketUsageEmitter,
    WebSocketUsageEnvelope, WebSocketUsageEvent,
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
    pub program_runtime_definitions: Vec<ProgramRuntimeDefinition>,
    pub entity_specs: Vec<arete_interpreter::ast::SerializableStreamSpec>,
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
            program_runtime_definitions: Vec::new(),
            entity_specs: Vec::new(),
            views: Vec::new(),
        }
    }

    pub fn with_parser_setup(mut self, setup_fn: ParserSetupFn) -> Self {
        self.parser_setup = Some(setup_fn);
        self
    }

    pub fn with_program_runtime_definitions(
        mut self,
        definitions: Vec<ProgramRuntimeDefinition>,
    ) -> Self {
        for definition in &definitions {
            if !self.program_ids.contains(&definition.program_id) {
                self.program_ids.push(definition.program_id.clone());
            }
        }
        self.program_runtime_definitions = definitions;
        self
    }

    pub fn with_entity_specs(
        mut self,
        entity_specs: Vec<arete_interpreter::ast::SerializableStreamSpec>,
    ) -> Self {
        self.entity_specs = entity_specs;
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

    /// Build a standalone HTTP gateway without stack or live-stream capabilities.
    pub fn solana_gateway(target_id: impl Into<String>) -> SolanaGatewayBuilder {
        SolanaGatewayBuilder::new(target_id.into())
    }
}

/// Constrained builder for health, chain reads, and fixed transaction routes.
///
/// The final runtime plan is fixed when `build` or `start` is called. This
/// builder intentionally has no Spec, WebSocket, stack-query, program-read, or
/// live-runtime configuration surface.
pub struct SolanaGatewayBuilder {
    inner: ServerBuilder,
}

impl SolanaGatewayBuilder {
    fn new(target_id: String) -> Self {
        let mut inner = ServerBuilder::new();
        inner.config.http_health = Some(HttpHealthConfig::default());
        inner.config.runtime_plan = RuntimePlan::solana_gateway();
        inner.config.solana_gateway_target_id = Some(target_id);
        Self { inner }
    }

    /// Set the gateway HTTP bind address.
    pub fn bind(mut self, addr: impl Into<SocketAddr>) -> Self {
        self.inner.config.http_health = Some(HttpHealthConfig::new(addr));
        self
    }

    /// Set the auth plugin for chain and transaction requests.
    pub fn auth_plugin(mut self, plugin: Arc<dyn WebSocketAuthPlugin>) -> Self {
        self.inner.http_auth_plugin = Some(plugin);
        self
    }

    /// Configure the existing fixed `/transactions/v1/*` handlers.
    pub fn transactions_config(mut self, config: TransactionConfig) -> Self {
        self.inner.config.transactions = Some(config);
        self
    }

    fn finalize(mut self) -> Result<ServerBuilder> {
        if self
            .inner
            .config
            .solana_gateway_target_id
            .as_deref()
            .is_none_or(|target_id| target_id.trim().is_empty())
        {
            anyhow::bail!("the Solana gateway target ID must not be empty");
        }
        if let Some(config) = self.inner.config.transactions.as_ref() {
            config.validate()?;
            if !config.enabled {
                anyhow::bail!("Solana gateway transaction configuration must be enabled");
            }
        }
        self.inner.config.runtime_plan = RuntimePlan::solana_gateway();
        Ok(self.inner)
    }

    /// Build the reusable runtime without starting it.
    pub fn build(self) -> Result<Runtime> {
        self.finalize()?.build()
    }

    /// Start the gateway and wait for shutdown.
    pub async fn start(self) -> Result<()> {
        self.finalize()?.start().await
    }
}

/// Builder for configuring and creating a Arete server
pub struct ServerBuilder {
    spec: Option<Spec>,
    views: Option<ViewIndex>,
    materialized_views: Option<MaterializedViewRegistry>,
    config: ServerConfig,
    websocket_auth_plugin: Option<Arc<dyn WebSocketAuthPlugin>>,
    http_auth_plugin: Option<Arc<dyn WebSocketAuthPlugin>>,
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
            http_auth_plugin: None,
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
        self.config.runtime_plan.websocket = true;
        self.config.runtime_plan.live_runtime = true;
        self
    }

    /// Configure WebSocket server
    pub fn websocket_config(mut self, config: WebSocketConfig) -> Self {
        self.config.websocket = Some(config);
        self.config.runtime_plan.websocket = true;
        self.config.runtime_plan.live_runtime = true;
        self
    }

    /// Set a WebSocket auth plugin used to authorize inbound connections.
    pub fn websocket_auth_plugin(mut self, plugin: Arc<dyn WebSocketAuthPlugin>) -> Self {
        self.websocket_auth_plugin = Some(plugin);
        self
    }

    /// Set an HTTP auth plugin used to authorize inbound read requests.
    pub fn http_auth_plugin(mut self, plugin: Arc<dyn WebSocketAuthPlugin>) -> Self {
        self.http_auth_plugin = Some(plugin);
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
        self.config.runtime_plan.websocket = true;
        self.config.runtime_plan.live_runtime = true;
        self
    }

    /// Configure Yellowstone gRPC connection
    pub fn yellowstone(mut self, config: YellowstoneConfig) -> Self {
        self.config.yellowstone = Some(config);
        self.config.runtime_plan.live_runtime = true;
        self
    }

    /// Enable health monitoring with default configuration
    pub fn health_monitoring(mut self) -> Self {
        self.config.health = Some(HealthConfig::default());
        self.config.runtime_plan.health = true;
        self
    }

    /// Configure health monitoring
    pub fn health_config(mut self, config: HealthConfig) -> Self {
        self.config.health = Some(config);
        self.config.runtime_plan.health = true;
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

    /// Enable the HTTP server with default configuration (port 8081).
    ///
    /// This serves health endpoints plus stack-scoped HTTP reads.
    pub fn http(mut self) -> Self {
        self.config.http_health = Some(HttpHealthConfig::default());
        self.config.runtime_plan.health = true;
        self.config.runtime_plan.chain_reads = true;
        self.config.runtime_plan.program_reads = true;
        self.config.runtime_plan.stack_queries = true;
        self
    }

    /// Configure the HTTP server.
    pub fn http_config(mut self, config: crate::http_server::HttpServerConfig) -> Self {
        self.config.http_health = Some(config);
        self.config.runtime_plan.health = true;
        self.config.runtime_plan.chain_reads = true;
        self.config.runtime_plan.program_reads = true;
        self.config.runtime_plan.stack_queries = true;
        self
    }

    /// Configure and explicitly enable the fixed transaction HTTP routes.
    pub fn transactions_config(mut self, config: TransactionConfig) -> Self {
        self.config.runtime_plan.transactions = config.enabled;
        self.config.transactions = Some(config);
        self
    }

    /// Replace the inferred capability set with an explicit runtime plan.
    pub fn runtime_plan(mut self, plan: RuntimePlan) -> Self {
        self.config.runtime_plan = plan;
        self
    }

    /// Enable only health and release-pinned program reads over HTTP.
    pub fn program_reads(mut self) -> Self {
        if self.config.http_health.is_none() {
            self.config.http_health = Some(HttpHealthConfig::default());
        }
        self.config.runtime_plan.health = true;
        self.config.runtime_plan.program_reads = true;
        self
    }

    pub fn chain_reads(mut self) -> Self {
        if self.config.http_health.is_none() {
            self.config.http_health = Some(HttpHealthConfig::default());
        }
        self.config.runtime_plan.chain_reads = true;
        self
    }

    pub fn stack_queries(mut self) -> Self {
        if self.config.http_health.is_none() {
            self.config.http_health = Some(HttpHealthConfig::default());
        }
        self.config.runtime_plan.stack_queries = true;
        self
    }

    pub fn live_runtime(mut self) -> Self {
        self.config.runtime_plan.live_runtime = true;
        self
    }

    /// Set the bind address for the HTTP server.
    pub fn http_bind(mut self, addr: impl Into<SocketAddr>) -> Self {
        if let Some(http_config) = &mut self.config.http_health {
            http_config.bind_address = addr.into();
        } else {
            self.config.http_health = Some(HttpHealthConfig::new(addr.into()));
        }
        self.config.runtime_plan.health = true;
        self.config.runtime_plan.chain_reads = true;
        self.config.runtime_plan.program_reads = true;
        self.config.runtime_plan.stack_queries = true;
        self
    }

    /// Enable HTTP health server with default configuration (port 8081)
    pub fn http_health(self) -> Self {
        self.http()
    }

    /// Configure HTTP health server
    pub fn http_health_config(self, config: HttpHealthConfig) -> Self {
        self.http_config(config)
    }

    /// Set the bind address for HTTP health server
    pub fn health_bind(self, addr: impl Into<SocketAddr>) -> Self {
        self.http_bind(addr)
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

        if let Some(plugin) = self.http_auth_plugin {
            runtime = runtime.with_http_auth_plugin(plugin);
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
            runtime = runtime.with_spec(spec)?;
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
            let entity_wire_formats = spec
                .entity_specs
                .iter()
                .map(|entity_spec| {
                    (
                        entity_spec.state_name.clone(),
                        ViewSpec::wire_format_from_entity_spec(entity_spec),
                    )
                })
                .collect::<std::collections::HashMap<_, _>>();

            for entity_name in spec.bytecode.entities.keys() {
                let wire_format = entity_wire_formats
                    .get(entity_name)
                    .cloned()
                    .unwrap_or_default();
                index.add_spec(ViewSpec {
                    id: format!("{}/list", entity_name),
                    export: entity_name.clone(),
                    mode: Mode::List,
                    wire_format: wire_format.clone(),
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
                    wire_format: wire_format.clone(),
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
                    wire_format,
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

                    let wire_format = entity_wire_formats
                        .get(&export)
                        .cloned()
                        .unwrap_or_default();
                    let view_spec = ViewSpec::from_view_def(view_def, &export, wire_format);
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

        if let Some(plugin) = self.http_auth_plugin {
            runtime = runtime.with_http_auth_plugin(plugin);
        }

        if let Some(max_clients) = self.websocket_max_clients {
            runtime = runtime.with_websocket_max_clients(max_clients);
        }

        if let Some(registry) = materialized_registry {
            runtime = runtime.with_materialized_views(registry);
        }

        if let Some(spec) = self.spec {
            runtime = runtime.with_spec(spec)?;
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

    #[test]
    fn http_without_websocket_has_a_read_only_runtime_plan() {
        let builder = Server::builder().http();
        assert!(builder.config.runtime_plan.program_reads);
        assert!(!builder.config.runtime_plan.live_runtime_enabled());
    }

    #[test]
    fn websocket_and_http_preserve_all_in_one_runtime_behavior() {
        let builder = Server::builder().websocket().http();
        assert!(builder.config.runtime_plan.websocket);
        assert!(builder.config.runtime_plan.live_runtime_enabled());
    }

    #[test]
    fn explicit_hosted_plan_disables_program_reads_after_http_helpers() {
        let plan = RuntimePlan {
            health: true,
            chain_reads: true,
            program_reads: false,
            stack_queries: true,
            transactions: true,
            websocket: true,
            live_runtime: true,
        };
        let builder = Server::builder()
            .websocket()
            .http_health()
            .health_bind("[::]:8081".parse::<SocketAddr>().unwrap())
            .runtime_plan(plan);

        assert_eq!(builder.config.runtime_plan, plan);
    }

    #[test]
    fn solana_gateway_builder_excludes_stack_and_live_capabilities() {
        let builder = Server::solana_gateway("gateway-us-east-1");

        assert_eq!(
            builder.inner.config.runtime_plan,
            RuntimePlan::solana_gateway()
        );
        assert!(builder.inner.config.http_health.is_some());
        assert_eq!(
            builder.inner.config.solana_gateway_target_id.as_deref(),
            Some("gateway-us-east-1")
        );
        assert!(builder.inner.config.websocket.is_none());
        assert!(builder.inner.config.yellowstone.is_none());
        assert!(builder.inner.spec.is_none());
        assert!(builder.inner.views.is_none());
        assert!(builder.inner.materialized_views.is_none());
        assert!(!builder.inner.config.runtime_plan.websocket);
        assert!(!builder.inner.config.runtime_plan.live_runtime_enabled());
        assert!(!builder.inner.config.runtime_plan.stack_queries);
        assert!(!builder.inner.config.runtime_plan.program_reads);
    }

    #[test]
    fn solana_gateway_builder_rejects_invalid_gateway_configuration() {
        assert!(Server::solana_gateway("").build().is_err());
        assert!(Server::solana_gateway("gateway-us-east-1")
            .transactions_config(TransactionConfig::default())
            .build()
            .is_err());
    }

    #[test]
    fn builder_rejects_mismatched_program_release_definitions() {
        let bytecode = arete_interpreter::compiler::MultiEntityBytecode::new().build();
        let definition = ProgramRuntimeDefinition {
            program_id: "Program111".to_string(),
            program_spec_hash: ProgramSpecHash::from_digest([1; 32]),
            idl_content_hash: IdlContentHash::from_digest([2; 32]),
            normalized_idl_hash: NormalizedIdlHash::from_digest([3; 32]),
            program_release_hash: ProgramReleaseHash::from_digest([4; 32]),
            account_reader: Arc::new(|_, _| Ok(serde_json::Value::Null)),
        };
        let spec =
            Spec::new(bytecode, "Program111").with_program_runtime_definitions(vec![definition]);

        assert!(Server::builder().spec(spec).build().is_err());
    }
}
