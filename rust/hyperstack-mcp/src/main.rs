//! `hs-mcp` — MCP server wrapping HyperStack streams for AI agent integration.
//!
//! See HYP-189 for the design. This binary speaks the Model Context Protocol
//! over stdio and exposes tools for AI agents to connect to HyperStack stacks,
//! subscribe to views, and query cached entities. See `connections.rs` for the
//! per-connection registry.

mod connections;
mod subscriptions;

use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::*,
    schemars, tool, tool_handler, tool_router,
    transport::stdio,
    ErrorData as McpError, ServerHandler, ServiceExt,
};
use serde::{Deserialize, Serialize};

use crate::connections::ConnectionRegistry;
use crate::subscriptions::SubscriptionRegistry;

#[derive(Clone)]
pub struct HyperstackMcp {
    tool_router: ToolRouter<HyperstackMcp>,
    connections: ConnectionRegistry,
    subscriptions: SubscriptionRegistry,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ConnectArgs {
    /// WebSocket URL of the HyperStack stack
    /// (e.g. `wss://your-stack.stack.usehyperstack.com`).
    pub url: String,
    /// Optional publishable API key for authenticated stacks.
    #[serde(default)]
    pub api_key: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DisconnectArgs {
    /// Connection ID returned from a previous `connect` call.
    pub connection_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SubscribeArgs {
    /// Connection ID returned from `connect`.
    pub connection_id: String,
    /// View name to subscribe to (e.g. `OreRound/latest`).
    pub view: String,
    /// Optional entity key to narrow the subscription to a single record.
    #[serde(default)]
    pub key: Option<String>,
    /// Whether to request the initial snapshot. Defaults to true.
    #[serde(default)]
    pub with_snapshot: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct UnsubscribeArgs {
    /// Subscription ID returned from a previous `subscribe` call.
    pub subscription_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListSubscriptionsArgs {
    /// Optional connection_id filter — only list subscriptions for that connection.
    #[serde(default)]
    pub connection_id: Option<String>,
}

#[derive(Debug, Serialize)]
struct SubscriptionInfo {
    subscription_id: String,
    connection_id: String,
    view: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    key: Option<String>,
}

#[derive(Debug, Serialize)]
struct ConnectionInfo {
    connection_id: String,
    url: String,
    state: String,
}

#[tool_router]
impl HyperstackMcp {
    pub fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
            connections: ConnectionRegistry::new(),
            subscriptions: SubscriptionRegistry::new(),
        }
    }

    #[tool(description = "Health check. Returns \"pong\" if the server is alive.")]
    async fn ping(&self) -> Result<CallToolResult, McpError> {
        Ok(CallToolResult::success(vec![Content::text("pong")]))
    }

    #[tool(description = "Open a WebSocket connection to a HyperStack stack. \
                          Returns a connection_id used by subscribe and query tools.")]
    async fn connect(
        &self,
        Parameters(args): Parameters<ConnectArgs>,
    ) -> Result<CallToolResult, McpError> {
        let id = self
            .connections
            .connect(args.url.clone(), args.api_key)
            .await
            .map_err(|e| McpError::internal_error(format!("connect failed: {e}"), None))?;
        let info = ConnectionInfo {
            connection_id: id,
            url: args.url,
            state: "Connecting".to_string(),
        };
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string(&info).unwrap_or_default(),
        )]))
    }

    #[tool(description = "Close an open HyperStack connection by id. \
                          Also drops every subscription bound to that connection.")]
    async fn disconnect(
        &self,
        Parameters(args): Parameters<DisconnectArgs>,
    ) -> Result<CallToolResult, McpError> {
        let removed = self.connections.disconnect(&args.connection_id).await;
        if removed {
            self.subscriptions
                .remove_for_connection(&args.connection_id);
            Ok(CallToolResult::success(vec![Content::text("disconnected")]))
        } else {
            Err(McpError::invalid_params(
                format!("unknown connection_id: {}", args.connection_id),
                None,
            ))
        }
    }

    #[tool(description = "Subscribe to a HyperStack view on an existing connection. \
                          Streamed entities are cached for query tools to read. \
                          Returns a subscription_id.")]
    async fn subscribe(
        &self,
        Parameters(args): Parameters<SubscribeArgs>,
    ) -> Result<CallToolResult, McpError> {
        let conn = self.connections.get(&args.connection_id).ok_or_else(|| {
            McpError::invalid_params(
                format!("unknown connection_id: {}", args.connection_id),
                None,
            )
        })?;

        let entry =
            self.subscriptions
                .insert(args.connection_id.clone(), args.view.clone(), args.key.clone());

        let mut sub = entry.to_sdk_subscription();
        if let Some(snap) = args.with_snapshot {
            sub = sub.with_snapshot(snap);
        }
        conn.manager.subscribe(sub).await;

        let info = SubscriptionInfo {
            subscription_id: entry.id.clone(),
            connection_id: entry.connection_id.clone(),
            view: entry.view.clone(),
            key: entry.key.clone(),
        };
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string(&info).unwrap_or_default(),
        )]))
    }

    #[tool(description = "Cancel a subscription by id.")]
    async fn unsubscribe(
        &self,
        Parameters(args): Parameters<UnsubscribeArgs>,
    ) -> Result<CallToolResult, McpError> {
        let entry = self
            .subscriptions
            .remove(&args.subscription_id)
            .ok_or_else(|| {
                McpError::invalid_params(
                    format!("unknown subscription_id: {}", args.subscription_id),
                    None,
                )
            })?;

        if let Some(conn) = self.connections.get(&entry.connection_id) {
            conn.manager.unsubscribe(entry.to_sdk_unsubscription()).await;
        }
        Ok(CallToolResult::success(vec![Content::text("unsubscribed")]))
    }

    #[tool(description = "List active subscriptions, optionally filtered by connection_id.")]
    async fn list_subscriptions(
        &self,
        Parameters(args): Parameters<ListSubscriptionsArgs>,
    ) -> Result<CallToolResult, McpError> {
        let out: Vec<SubscriptionInfo> = self
            .subscriptions
            .list(args.connection_id.as_deref())
            .into_iter()
            .map(|e| SubscriptionInfo {
                subscription_id: e.id.clone(),
                connection_id: e.connection_id.clone(),
                view: e.view.clone(),
                key: e.key.clone(),
            })
            .collect();
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string(&out).unwrap_or_default(),
        )]))
    }

    #[tool(description = "List all currently open HyperStack connections.")]
    async fn list_connections(&self) -> Result<CallToolResult, McpError> {
        let mut out = Vec::new();
        for entry in self.connections.list() {
            out.push(ConnectionInfo {
                connection_id: entry.id.clone(),
                url: entry.url.clone(),
                state: format!("{:?}", entry.state().await),
            });
        }
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string(&out).unwrap_or_default(),
        )]))
    }
}

impl Default for HyperstackMcp {
    fn default() -> Self {
        Self::new()
    }
}

#[tool_handler]
impl ServerHandler for HyperstackMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                env!("CARGO_PKG_NAME"),
                env!("CARGO_PKG_VERSION"),
            ))
            .with_protocol_version(ProtocolVersion::V_2024_11_05)
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Logs go to stderr so they don't pollute the stdio MCP transport on stdout.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    tracing::info!("starting hs-mcp stdio server");
    let service = HyperstackMcp::new().serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
