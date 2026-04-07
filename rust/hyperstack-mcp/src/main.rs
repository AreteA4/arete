//! `hs-mcp` — MCP server wrapping HyperStack streams for AI agent integration.
//!
//! See HYP-189 for the design. This binary speaks the Model Context Protocol
//! over stdio and exposes tools for AI agents to connect to HyperStack stacks,
//! subscribe to views, and query cached entities. See `connections.rs` for the
//! per-connection registry.

mod connections;
mod filter;
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
use crate::filter::{Filter, StructuredPredicate};
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

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetEntityArgs {
    /// Subscription ID returned from `subscribe`.
    pub subscription_id: String,
    /// Entity key to fetch.
    pub key: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListEntitiesArgs {
    /// Subscription ID returned from `subscribe`.
    pub subscription_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetRecentArgs {
    /// Subscription ID returned from `subscribe`.
    pub subscription_id: String,
    /// How many recent entities to return. Hard cap is 1000.
    pub n: usize,
}

/// Hard ceiling on entities returned by any single query tool call.
/// Protects the stdio transport from runaway agents that ask for everything.
const QUERY_LIMIT_MAX: usize = 1000;
const QUERY_LIMIT_DEFAULT: usize = 100;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct QueryEntitiesArgs {
    /// Subscription ID returned from `subscribe`.
    pub subscription_id: String,
    /// String-DSL filter expressions, ANDed together. Same syntax as the
    /// `hs stream --where` flag: `field=value`, `field>N`, `field~regex`,
    /// `field?` (exists), `field!?` (not exists), `field!=value`, `field!~re`.
    #[serde(default)]
    pub r#where: Vec<String>,
    /// Structured filter predicates, ANDed with `where`. LLM-friendly form
    /// that avoids escaping pitfalls in the string DSL.
    #[serde(default)]
    pub filters: Vec<StructuredPredicate>,
    /// Comma-separated dot-paths to project from each matching entity.
    /// If omitted, returns the full entity.
    #[serde(default)]
    pub select: Option<String>,
    /// Maximum number of entities to return. Defaults to 100, capped at 1000.
    #[serde(default)]
    pub limit: Option<usize>,
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

    #[tool(description = "Filter and project entities cached for a subscription. \
                          Accepts both a string-DSL `where` (CLI-compatible) and \
                          structured `filters` (LLM-friendly). Both are ANDed. \
                          `select` projects fields by dot-path. `limit` defaults \
                          to 100 and is capped at 1000.")]
    async fn query_entities(
        &self,
        Parameters(args): Parameters<QueryEntitiesArgs>,
    ) -> Result<CallToolResult, McpError> {
        let (store, view) = self.resolve_subscription(&args.subscription_id)?;

        let mut compiled = Filter::parse(&args.r#where)
            .map_err(|e| McpError::invalid_params(format!("invalid where: {e}"), None))?;
        let structured = Filter::from_structured(&args.filters)
            .map_err(|e| McpError::invalid_params(format!("invalid filters: {e}"), None))?;
        compiled.extend(structured);

        let select_paths = args.select.as_deref().map(filter::parse_select);
        let limit = args.limit.unwrap_or(QUERY_LIMIT_DEFAULT).min(QUERY_LIMIT_MAX);

        // Snapshot raw entries under the read lock, then filter/project outside
        // the lock to keep the critical section short.
        let raw = store.all_raw(&view).await;
        let total_scanned = raw.len();
        let mut matched: Vec<serde_json::Value> = Vec::new();
        for (_key, value) in raw {
            if !compiled.is_empty() && !compiled.matches(&value) {
                continue;
            }
            let projected = match &select_paths {
                Some(paths) => filter::select_fields(&value, paths),
                None => value,
            };
            matched.push(projected);
            if matched.len() >= limit {
                break;
            }
        }

        let payload = serde_json::json!({
            "view": view,
            "total_scanned": total_scanned,
            "returned": matched.len(),
            "limit_applied": limit,
            "entities": matched,
        });
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string(&payload).unwrap_or_default(),
        )]))
    }

    #[tool(description = "Fetch a single entity by key from a subscription's cache.")]
    async fn get_entity(
        &self,
        Parameters(args): Parameters<GetEntityArgs>,
    ) -> Result<CallToolResult, McpError> {
        let (store, view) = self.resolve_subscription(&args.subscription_id)?;
        let value: Option<serde_json::Value> = store.get(&view, &args.key).await;
        let payload = serde_json::json!({
            "view": view,
            "key": args.key,
            "found": value.is_some(),
            "data": value,
        });
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string(&payload).unwrap_or_default(),
        )]))
    }

    #[tool(description = "List entity keys currently cached for a subscription. \
                          Returns keys only — use get_entity for values.")]
    async fn list_entities(
        &self,
        Parameters(args): Parameters<ListEntitiesArgs>,
    ) -> Result<CallToolResult, McpError> {
        let (store, view) = self.resolve_subscription(&args.subscription_id)?;
        let raw = store.all_raw(&view).await;
        let keys: Vec<String> = raw.into_keys().collect();
        let payload = serde_json::json!({
            "view": view,
            "count": keys.len(),
            "keys": keys,
        });
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string(&payload).unwrap_or_default(),
        )]))
    }

    #[tool(description = "Return up to N entities from a subscription's cache. \
                          Order matches the view's sort config when configured, \
                          otherwise hash order — not strict insertion recency.")]
    async fn get_recent(
        &self,
        Parameters(args): Parameters<GetRecentArgs>,
    ) -> Result<CallToolResult, McpError> {
        let (store, view) = self.resolve_subscription(&args.subscription_id)?;
        let n = args.n.min(QUERY_LIMIT_MAX);
        let all: Vec<serde_json::Value> = store.list(&view).await;
        // TODO(HYP-189): SharedStore exposes no native "last N inserted" view.
        // For sorted views the tail is meaningful; for unsorted views it is
        // hash order. If real usage needs strict recency, add a per-view ring
        // buffer in the ingest task.
        let total = all.len();
        let recent: Vec<serde_json::Value> = all.into_iter().rev().take(n).collect();
        let payload = serde_json::json!({
            "view": view,
            "total_cached": total,
            "returned": recent.len(),
            "entities": recent,
        });
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string(&payload).unwrap_or_default(),
        )]))
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

impl HyperstackMcp {
    /// Resolve a `subscription_id` to its connection's `SharedStore` and the
    /// view name to query inside it. Returns an MCP `invalid_params` error if
    /// either the subscription or its underlying connection is gone.
    fn resolve_subscription(
        &self,
        subscription_id: &str,
    ) -> Result<(std::sync::Arc<hyperstack_sdk::SharedStore>, String), McpError> {
        let sub = self.subscriptions.get(subscription_id).ok_or_else(|| {
            McpError::invalid_params(
                format!("unknown subscription_id: {subscription_id}"),
                None,
            )
        })?;
        let conn = self.connections.get(&sub.connection_id).ok_or_else(|| {
            McpError::internal_error(
                format!(
                    "subscription {} references unknown connection_id {}",
                    sub.id, sub.connection_id
                ),
                None,
            )
        })?;
        Ok((conn.store.clone(), sub.view.clone()))
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
