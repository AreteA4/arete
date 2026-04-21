//! `a4-mcp` — MCP server wrapping Arete streams for AI agent integration.
//!
//! See HYP-189 for the design. This binary speaks the Model Context Protocol
//! over stdio and exposes tools for AI agents to connect to Arete stacks,
//! subscribe to views, and query cached entities. See `connections.rs` for the
//! per-connection registry.

mod connections;
mod credentials;
mod filter;
mod subscriptions;

/// LLM-friendly deserializers that accept both the typed form and a string
/// encoding of the typed form. LLMs frequently emit `"5"` instead of `5` when
/// filling out tool-call arguments; strict serde refuses the coercion, which
/// produces `invalid type: string "5"` errors that make the agent think the
/// tool is broken. Using these helpers on numeric fields makes the schema
/// forgiving without losing validation on bad input.
mod lenient {
    use serde::{de::Error, Deserialize, Deserializer};
    use serde_json::Value;

    fn value_to_usize<E: Error>(v: Value) -> Result<Option<usize>, E> {
        match v {
            Value::Null => Ok(None),
            Value::Number(n) => n
                .as_u64()
                .map(|u| Some(u as usize))
                .ok_or_else(|| E::custom(format!("expected non-negative integer, got {n}"))),
            Value::String(s) => {
                let t = s.trim();
                if t.is_empty() {
                    Ok(None)
                } else {
                    t.parse::<usize>()
                        .map(Some)
                        .map_err(|e| E::custom(format!("expected integer, got {s:?}: {e}")))
                }
            }
            other => Err(E::custom(format!(
                "expected integer or numeric string, got {other}"
            ))),
        }
    }

    pub fn usize<'de, D: Deserializer<'de>>(d: D) -> Result<usize, D::Error> {
        let v = Value::deserialize(d)?;
        match value_to_usize::<D::Error>(v)? {
            Some(n) => Ok(n),
            None => Err(D::Error::custom(
                "expected integer, got null or empty string",
            )),
        }
    }

    pub fn opt_usize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<usize>, D::Error> {
        let v = Value::deserialize(d)?;
        value_to_usize::<D::Error>(v)
    }

    #[cfg(test)]
    mod tests {
        use serde::Deserialize;

        #[derive(Deserialize)]
        struct S {
            #[serde(deserialize_with = "super::usize")]
            n: usize,
            #[serde(default, deserialize_with = "super::opt_usize")]
            limit: Option<usize>,
        }

        fn parse(json: &str) -> serde_json::Result<S> {
            serde_json::from_str(json)
        }

        #[test]
        fn accepts_int() {
            let s = parse(r#"{"n": 10, "limit": 5}"#).unwrap();
            assert_eq!(s.n, 10);
            assert_eq!(s.limit, Some(5));
        }

        #[test]
        fn accepts_string() {
            let s = parse(r#"{"n": "10", "limit": "5"}"#).unwrap();
            assert_eq!(s.n, 10);
            assert_eq!(s.limit, Some(5));
        }

        #[test]
        fn opt_accepts_null_and_missing() {
            let s1 = parse(r#"{"n": 3, "limit": null}"#).unwrap();
            assert_eq!(s1.limit, None);
            let s2 = parse(r#"{"n": 3}"#).unwrap();
            assert_eq!(s2.limit, None);
            let s3 = parse(r#"{"n": 3, "limit": ""}"#).unwrap();
            assert_eq!(s3.limit, None);
        }

        #[test]
        fn rejects_nonsense() {
            assert!(parse(r#"{"n": "not a number"}"#).is_err());
            assert!(parse(r#"{"n": true}"#).is_err());
        }
    }
}

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
pub struct AreteMcp {
    tool_router: ToolRouter<AreteMcp>,
    connections: ConnectionRegistry,
    subscriptions: SubscriptionRegistry,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ConnectArgs {
    /// WebSocket URL of the Arete stack
    /// (e.g. `wss://your-stack.stack.arete.run`).
    pub url: String,
    /// Optional explicit API key (override). If omitted, the server resolves
    /// the key from the `ARETE_API_KEY` env var, then from
    /// `~/.arete/credentials.toml` (the file managed by `a4 auth login`).
    /// Prefer leaving this blank in agent calls so the key does not enter
    /// the model context or chat transcript.
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
    /// How many recent entities to return. Hard cap is 1000. Accepts either
    /// an integer (`5`) or a string-encoded integer (`"5"`) because LLM
    /// tool-call arguments sometimes stringify numbers.
    #[serde(deserialize_with = "lenient::usize")]
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
    /// `a4 stream --where` flag: `field=value`, `field>N`, `field~regex`,
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
    /// Accepts either an integer (`5`) or a string-encoded integer (`"5"`)
    /// because LLM tool-call arguments sometimes stringify numbers.
    #[serde(default, deserialize_with = "lenient::opt_usize")]
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
    /// Where the api key came from for this connect call. One of
    /// `explicit_argument`, `env:ARETE_API_KEY`,
    /// `~/.arete/credentials.toml`, or `none`. Never contains the key
    /// itself — this field is safe to log and to expose to the agent.
    /// Only populated on `connect`; omitted from `list_connections` because
    /// we don't store per-connection credential provenance.
    #[serde(skip_serializing_if = "Option::is_none")]
    key_source: Option<&'static str>,
}

#[tool_router]
impl AreteMcp {
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

    #[tool(description = "Open a WebSocket connection to a Arete stack. \
                          Returns a connection_id used by subscribe and query tools.\n\n\
                          AUTH: Prefer omitting `api_key` in agent calls — the \
                          server resolves it automatically from (1) explicit arg, \
                          (2) `ARETE_API_KEY` env var, (3) \
                          `~/.arete/credentials.toml` (managed by \
                          `a4 auth login`). Passing the key as an argument puts it \
                          in the model context and chat transcript, which is \
                          usually not what you want. The response includes a \
                          `key_source` field so you can see which lookup path \
                          produced the credential (never the key itself).")]
    async fn connect(
        &self,
        Parameters(args): Parameters<ConnectArgs>,
    ) -> Result<CallToolResult, McpError> {
        let resolved = credentials::resolve(args.api_key, &args.url)
            .map_err(|e| McpError::invalid_params(e.to_string(), None))?;

        let id = self
            .connections
            .connect(args.url.clone(), resolved.key)
            .await
            .map_err(|e| McpError::internal_error(format!("connect failed: {e}"), None))?;

        let info = ConnectionInfo {
            connection_id: id,
            url: args.url,
            state: "Connecting".to_string(),
            key_source: Some(resolved.source.as_str()),
        };
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string(&info).unwrap_or_default(),
        )]))
    }

    #[tool(description = "Close an open Arete connection by id. \
                          Also drops every subscription bound to that connection.")]
    async fn disconnect(
        &self,
        Parameters(args): Parameters<DisconnectArgs>,
    ) -> Result<CallToolResult, McpError> {
        // ConnectionRegistry::disconnect removes the entry from the
        // DashMap, then acquires the per-entry write lock to wait out any
        // in-flight `subscribe` calls holding a read guard. Only after that
        // is it safe to sweep the SubscriptionRegistry — otherwise a
        // subscribe that was mid-flight could insert a new entry after our
        // sweep and leave an orphan. See connections.rs module docs.
        let entry = self.connections.disconnect(&args.connection_id).await;
        if entry.is_some() {
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

    #[tool(description = "Subscribe to a Arete view on an existing connection. \
                          Streamed entities land in an in-memory cache that the query \
                          tools (get_entity, list_entities, get_recent, query_entities) \
                          read from.\n\n\
                          VIEW NAMING: A view name ALWAYS has the shape \
                          `EntityName/mode` — an entity name, a slash, and a mode. \
                          Pass the full string, never just the mode. Concrete \
                          examples:\n\
                          - `PumpfunToken/list`   (pump.fun tokens, list view)\n\
                          - `PumpfunToken/state`  (pump.fun tokens, per-key state)\n\
                          - `PumpfunToken/append` (pump.fun tokens, append-only events)\n\
                          - `OreRound/latest`     (ore rounds, custom view)\n\n\
                          Every entity in a stack auto-generates three built-in modes:\n\
                          - `/list`   — ordered recent-items list, sorted by _seq desc. \
                          Best default for 'show me recent X' queries.\n\
                          - `/state`  — per-key current-state cache. May legitimately \
                          be empty if entities have not written state yet.\n\
                          - `/append` — append-only event stream of every write.\n\
                          Stacks may also expose custom view modes (like `/latest` \
                          in the ore stack); custom names can only be learned from the \
                          stack's source or docs.\n\n\
                          IF YOUR CACHE STAYS EMPTY after subscribing and waiting a \
                          few seconds, the most likely cause is wrong mode choice — \
                          try `EntityName/list` before concluding the stack is empty. \
                          If the view name you passed did not include a slash and an \
                          entity name, that is a bug — always prepend the entity.\n\n\
                          Returns { subscription_id, connection_id, view, key }.")]
    async fn subscribe(
        &self,
        Parameters(args): Parameters<SubscribeArgs>,
    ) -> Result<CallToolResult, McpError> {
        validate_view_name(&args.view)?;

        let conn = self.connections.get(&args.connection_id).ok_or_else(|| {
            McpError::invalid_params(
                format!("unknown connection_id: {}", args.connection_id),
                None,
            )
        })?;

        // Race protection against a concurrent `disconnect`. We hold the
        // read guard for the full insert-sub + dispatch window. Disconnect
        // takes the write lock before sweeping subscriptions, so it will
        // wait for us to finish; if it won the write lock first, `*alive`
        // is now `false` and we bail without inserting anything. See
        // `connections.rs` module docs for the full argument.
        let alive_guard = conn.alive.read().await;
        if !*alive_guard {
            return Err(McpError::invalid_params(
                format!(
                    "connection {} was disconnected concurrently; subscription not created",
                    args.connection_id
                ),
                None,
            ));
        }

        let entry = self.subscriptions.insert(
            args.connection_id.clone(),
            args.view.clone(),
            args.key.clone(),
        );

        let mut sub = entry.to_sdk_subscription();
        if let Some(snap) = args.with_snapshot {
            sub = sub.with_snapshot(snap);
        }
        conn.manager.subscribe(sub).await;
        // Guard explicitly dropped at end of scope; keeping it named ensures
        // the compiler won't reorder it before the dispatch.
        drop(alive_guard);

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
            conn.manager
                .unsubscribe(entry.to_sdk_unsubscription())
                .await;
        }
        Ok(CallToolResult::success(vec![Content::text("unsubscribed")]))
    }

    #[tool(
        description = "Filter and project entities cached for a subscription. \
                          Accepts both a string-DSL `where` (CLI-compatible) and \
                          structured `filters` (LLM-friendly). Both are ANDed. \
                          `select` projects fields by dot-path. `limit` defaults \
                          to 100 and is capped at 1000.\n\n\
                          If this returns 0 entities, the view may be empty on this \
                          deployment — consider resubscribing with a different mode \
                          suffix (e.g. /list instead of /state); see the `subscribe` \
                          tool description for the mode reference."
    )]
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
        let limit = args
            .limit
            .unwrap_or(QUERY_LIMIT_DEFAULT)
            .min(QUERY_LIMIT_MAX);

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
                          Returns keys only — use get_entity for values. \
                          Hard-capped at 1000 keys per response to protect the \
                          stdio transport; `total_cached` reports the true cache \
                          size and `truncated` is true when the cap was hit. Use \
                          query_entities with a filter if you need to page through \
                          a larger cache.\n\n\
                          If this returns 0 keys, the view may be empty on this \
                          deployment — consider resubscribing with a different mode \
                          suffix (e.g. /list instead of /state); see the `subscribe` \
                          tool description for the mode reference.")]
    async fn list_entities(
        &self,
        Parameters(args): Parameters<ListEntitiesArgs>,
    ) -> Result<CallToolResult, McpError> {
        let (store, view) = self.resolve_subscription(&args.subscription_id)?;
        let raw = store.all_raw(&view).await;
        let total_cached = raw.len();
        let keys: Vec<String> = raw.into_keys().take(QUERY_LIMIT_MAX).collect();
        let truncated = total_cached > keys.len();
        let payload = serde_json::json!({
            "view": view,
            "total_cached": total_cached,
            "returned": keys.len(),
            "truncated": truncated,
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

    #[tool(description = "List all currently open Arete connections.")]
    async fn list_connections(&self) -> Result<CallToolResult, McpError> {
        let mut out = Vec::new();
        for entry in self.connections.list() {
            out.push(ConnectionInfo {
                connection_id: entry.id.clone(),
                url: entry.url.clone(),
                state: format!("{:?}", entry.state().await),
                key_source: None,
            });
        }
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string(&out).unwrap_or_default(),
        )]))
    }
}

/// Validate that a subscribe `view` argument has the expected
/// `<EntityName>/<mode>` shape. Catches two real agent failure modes:
///
/// 1. Empty or whitespace — sometimes emitted as a retry after an initial
///    "missing field view" error.
/// 2. Only the mode — e.g. `"list"` or `"state"`. This happens when weaker
///    LLMs read the tool description's `<EntityName>/list` template and strip
///    the placeholder, leaving just the suffix. The Arete server will
///    actually accept a single-segment view name and return zero data, which
///    the agent then misreads as "the stack is empty".
fn validate_view_name(view: &str) -> Result<(), McpError> {
    let trimmed = view.trim();
    if trimmed.is_empty() {
        return Err(McpError::invalid_params(
            "`view` must be a non-empty string shaped like `PumpfunToken/list` \
             or `OreRound/latest`. See the subscribe tool description for \
             the naming convention."
                .to_string(),
            None,
        ));
    }
    let Some((entity, mode)) = trimmed.split_once('/') else {
        return Err(McpError::invalid_params(
            format!(
                "`view` must be shaped like `<EntityName>/<mode>` (e.g. \
                 `PumpfunToken/list`). Got `{view}` — looks like only the \
                 mode portion. Prepend the entity name from the stack's \
                 source (e.g. `PumpfunToken/{view}`)."
            ),
            None,
        ));
    };
    if entity.trim().is_empty() || mode.trim().is_empty() {
        return Err(McpError::invalid_params(
            format!(
                "`view` must have non-empty entity and mode halves, got \
                 `{view}`. Example: `PumpfunToken/list`."
            ),
            None,
        ));
    }
    Ok(())
}

impl AreteMcp {
    /// Resolve a `subscription_id` to its connection's `SharedStore` and the
    /// view name to query inside it. Returns an MCP `invalid_params` error if
    /// either the subscription or its underlying connection is gone.
    fn resolve_subscription(
        &self,
        subscription_id: &str,
    ) -> Result<(std::sync::Arc<arete_sdk::SharedStore>, String), McpError> {
        let sub = self.subscriptions.get(subscription_id).ok_or_else(|| {
            McpError::invalid_params(format!("unknown subscription_id: {subscription_id}"), None)
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

impl Default for AreteMcp {
    fn default() -> Self {
        Self::new()
    }
}

#[tool_handler]
impl ServerHandler for AreteMcp {
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

    tracing::info!("starting a4-mcp stdio server");
    let service = AreteMcp::new().serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod view_validation_tests {
    use super::validate_view_name;

    #[test]
    fn accepts_standard_views() {
        assert!(validate_view_name("PumpfunToken/list").is_ok());
        assert!(validate_view_name("PumpfunToken/state").is_ok());
        assert!(validate_view_name("PumpfunToken/append").is_ok());
        assert!(validate_view_name("OreRound/latest").is_ok());
    }

    #[test]
    fn rejects_empty_and_whitespace() {
        assert!(validate_view_name("").is_err());
        assert!(validate_view_name("   ").is_err());
    }

    #[test]
    fn rejects_mode_only_without_entity_prefix() {
        // The key regression: agents sometimes emit just "list" after stripping
        // the `<EntityName>` placeholder in the tool description.
        let err = validate_view_name("list").unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("EntityName"),
            "error should explain the format: {msg}"
        );
        assert!(
            msg.contains("PumpfunToken/list"),
            "error should suggest a concrete fix: {msg}"
        );
    }

    #[test]
    fn rejects_entity_without_mode() {
        assert!(validate_view_name("PumpfunToken/").is_err());
        assert!(validate_view_name("PumpfunToken").is_err());
    }

    #[test]
    fn rejects_empty_entity_with_mode() {
        assert!(validate_view_name("/list").is_err());
    }
}
