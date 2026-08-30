//! `a4-mcp` — MCP server wrapping Arete streams for AI agent integration.
//!
//! See HYP-189 for the design. This binary speaks the Model Context Protocol
//! over stdio and exposes tools for AI agents to connect to Arete stacks,
//! subscribe to views, and query cached entities. See `connections.rs` for the
//! per-connection registry.

mod connections;
mod credentials;
mod filter;
mod registry;
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

use arete_sdk::{Subscription, SubscriptionQuery};
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
use crate::registry::RegistryClient;
use crate::subscriptions::SubscriptionRegistry;

#[derive(Clone)]
pub struct AreteMcp {
    tool_router: ToolRouter<AreteMcp>,
    connections: ConnectionRegistry,
    subscriptions: SubscriptionRegistry,
    registry: RegistryClient,
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
pub struct ExploreStackArgs {
    /// Bare stack reference as listed by `explore_stacks` (e.g. `ore`).
    /// Not a URL and not a path.
    pub stack: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ExploreProgramArgs {
    /// Bare program reference as listed by `explore_programs`
    /// (e.g. `spl-token`), or a program ID.
    pub program: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ResolveArtifactArgs {
    /// One of `program-spec`, `live-spec`, `stack-manifest`.
    pub kind: String,
    /// Artifact hash taken from an install descriptor.
    pub hash: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchKnowledgeArgs {
    /// Free-text intent to search for (e.g. `monitor swaps`). Matched against
    /// concept names and synonyms first, then protocols/programs/recipes via
    /// full-text search. At least one of `query`, `concept`, `category` is
    /// required.
    #[serde(default)]
    pub query: Option<String>,
    /// Concept slug to filter by (e.g. `swap`). Discover slugs with
    /// `list_concepts`.
    #[serde(default)]
    pub concept: Option<String>,
    /// Category slug to filter by (e.g. `dex`). Discover slugs with
    /// `list_concepts`.
    #[serde(default)]
    pub category: Option<String>,
    /// Maximum number of results. Accepts either an integer (`5`) or a
    /// string-encoded integer (`"5"`) because LLM tool-call arguments
    /// sometimes stringify numbers.
    #[serde(default, deserialize_with = "lenient::opt_usize")]
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetProtocolArgs {
    /// Bare protocol slug (e.g. `meteora-damm`), as returned by
    /// `search_knowledge`. Not a URL and not a path.
    pub protocol: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetProgramKnowledgeArgs {
    /// Bare program slug (e.g. `meteora-cp-amm`), as listed by
    /// `search_knowledge` or a protocol's `programs`. Not a URL and not a
    /// path.
    pub program: String,
    /// Which part of the annotations to fetch: `summary` (default),
    /// `instructions`, `accounts`, or `surface`.
    #[serde(default)]
    pub section: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetRecipeArgs {
    /// Bare recipe slug (e.g. `execute-presale-purchase-via-squads`). Not a
    /// URL and not a path.
    pub recipe: String,
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
            registry: RegistryClient::new(),
        }
    }

    #[tool(description = "Health check. Returns \"pong\" if the server is alive.")]
    async fn ping(&self) -> Result<CallToolResult, McpError> {
        Ok(CallToolResult::success(vec![Content::text("pong")]))
    }

    #[tool(description = "List stacks available in the Arete registry. \
                          Start here: the returned `websocket_url` is what `connect` \
                          takes, and `entities` tells you which `<EntityName>/<view>` \
                          ids to look for.\n\n\
                          NOTE: casing is not uniform across these tools. This endpoint \
                          and `explore_stack_schema` return snake_case \
                          (`websocket_url`, `primary_keys`, `rust_type`); \
                          `explore_stack`, `explore_programs` and `explore_program` \
                          return camelCase (`websocketUrl`, `installName`, \
                          `programSpecHash`). `resolve_artifact` has a camelCase \
                          envelope over a stored payload that may be snake_case inside. \
                          Read the keys you actually get rather than assuming one \
                          convention, and note `a4 explore --json` is camelCase \
                          throughout, so it does not match this tool field-for-field.\n\n\
                          No auth required — public stacks are always listed. If an \
                          api key is resolvable (ARETE_API_KEY or `a4 auth login`), \
                          global stacks are included too.")]
    async fn explore_stacks(&self) -> Result<CallToolResult, McpError> {
        registry_result(self.registry.list_stacks().await)
    }

    #[tool(
        description = "Fetch the pinned install descriptor for one stack: the exact \
                          StackManifest, AST, LiveSpec, view, and Program Release \
                          identities that `a4 install` would consume.\n\n\
                          Pass a bare stack reference (e.g. `ore`), not a URL."
    )]
    async fn explore_stack(
        &self,
        Parameters(args): Parameters<ExploreStackArgs>,
    ) -> Result<CallToolResult, McpError> {
        registry_result(self.registry.stack_install(&args.stack).await)
    }

    #[tool(
        description = "Fetch the entity and view schema for one stack — field paths, \
                          types, primary keys, and the view ids `subscribe` accepts.\n\n\
                          Use this to resolve a `<EntityName>/<view>` id before calling \
                          subscribe, instead of guessing from a template."
    )]
    async fn explore_stack_schema(
        &self,
        Parameters(args): Parameters<ExploreStackArgs>,
    ) -> Result<CallToolResult, McpError> {
        registry_result(self.registry.stack_schema(&args.stack).await)
    }

    #[tool(
        description = "List standalone Solana programs installable from the Arete \
                          registry, independent of any stack. No auth required."
    )]
    async fn explore_programs(&self) -> Result<CallToolResult, McpError> {
        registry_result(self.registry.list_programs().await)
    }

    #[tool(
        description = "Fetch the pinned install descriptor for one standalone program: \
                          program identity and hashes, accounts, instructions, events, \
                          types, and Program Read availability.\n\n\
                          Pass a bare program reference (e.g. `spl-token`), not a URL."
    )]
    async fn explore_program(
        &self,
        Parameters(args): Parameters<ExploreProgramArgs>,
    ) -> Result<CallToolResult, McpError> {
        registry_result(self.registry.program_install(&args.program).await)
    }

    #[tool(
        description = "Fetch a content-addressed artifact by hash. `kind` must be one \
                          of `program-spec`, `live-spec`, or `stack-manifest`; the hash \
                          comes from an install descriptor returned by explore_stack or \
                          explore_program.\n\n\
                          Large artifacts are refused rather than truncated — use the \
                          `a4` CLI for those."
    )]
    async fn resolve_artifact(
        &self,
        Parameters(args): Parameters<ResolveArtifactArgs>,
    ) -> Result<CallToolResult, McpError> {
        registry_result(self.registry.artifact(&args.kind, &args.hash).await)
    }

    #[tool(
        description = "Search the curated Solana knowledge layer for protocols, programs, \
                          stacks, and recipes that serve an intent. Start here when you \
                          need to find which protocols/programs/stacks serve an intent \
                          like 'monitor swaps' or 'execute through a multisig'.\n\n\
                          `query` is free text, matched against concept names and \
                          synonyms first, then protocols/programs/recipes via full-text \
                          search. `concept` and `category` filter by exact slug — \
                          discover slugs with `list_concepts`. At least one of the three \
                          is required.\n\n\
                          Each result carries coverage flags: `read` (fetch on-chain \
                          account state), `build` (construct transactions), `subscribe` \
                          (stream live entities from a hosted stack) — pick the mode you \
                          need, then drill in with get_protocol, get_program_knowledge, \
                          or get_recipe.\n\n\
                          AUTH: unlike the explore_* tools, this requires an Arete API \
                          key (`ARETE_API_KEY` env var, or the file `a4 auth login` \
                          writes)."
    )]
    async fn search_knowledge(
        &self,
        Parameters(args): Parameters<SearchKnowledgeArgs>,
    ) -> Result<CallToolResult, McpError> {
        registry_result(
            self.registry
                .knowledge_search(
                    args.query.as_deref(),
                    args.concept.as_deref(),
                    args.category.as_deref(),
                    args.limit,
                )
                .await,
        )
    }

    #[tool(
        description = "Fetch curated knowledge for one protocol by slug (e.g. \
                          `meteora-damm`): description, categories, links, its on-chain \
                          programs with roles (core/periphery/deprecated), related \
                          protocols (composes-with, wraps, graduates-to, ...), the \
                          public stacks streaming its entities, and per-concept coverage \
                          (read/build/subscribe).\n\n\
                          Use after search_knowledge to decide how to integrate a \
                          protocol; follow `programs[].slug` into get_program_knowledge \
                          for instruction-level detail.\n\n\
                          Pass a bare slug, not a URL. Requires an API key \
                          (`a4 auth login`)."
    )]
    async fn get_protocol(
        &self,
        Parameters(args): Parameters<GetProtocolArgs>,
    ) -> Result<CallToolResult, McpError> {
        registry_result(self.registry.knowledge_protocol(&args.protocol).await)
    }

    #[tool(
        description = "Fetch curated, human-reviewed annotations for one Solana program \
                          by slug (e.g. `meteora-cp-amm`). `section` selects what comes \
                          back:\n\
                          - `summary` (default) — program header, provenance, and counts\n\
                          - `instructions` — per-instruction semantics: what each \
                          instruction does, argument and account meanings, concepts\n\
                          - `accounts` — account-type semantics and field meanings\n\
                          - `surface` — the ingested SDK extension surface for this \
                          program (callable operations with bindings)\n\n\
                          Sections keep responses under the 512 KiB tool-result cap — \
                          fetch only the section you need. Pass a bare slug, not a URL. \
                          Requires an API key (`a4 auth login`)."
    )]
    async fn get_program_knowledge(
        &self,
        Parameters(args): Parameters<GetProgramKnowledgeArgs>,
    ) -> Result<CallToolResult, McpError> {
        registry_result(
            self.registry
                .knowledge_program(&args.program, args.section.as_deref())
                .await,
        )
    }

    #[tool(
        description = "Fetch one cross-protocol recipe by slug (e.g. \
                          `execute-presale-purchase-via-squads`): an ordered, curated \
                          sequence of steps for a multi-protocol pattern (e.g. wrap a \
                          prepared transaction in a Squads multisig), each step \
                          referencing a real SDK surface entry (resolved in the \
                          response), plus a path to working example code.\n\n\
                          Use when search_knowledge returns a `recipe` result or a \
                          protocol's `related` edges cite one as evidence.\n\n\
                          Pass a bare slug, not a URL. Requires an API key \
                          (`a4 auth login`)."
    )]
    async fn get_recipe(
        &self,
        Parameters(args): Parameters<GetRecipeArgs>,
    ) -> Result<CallToolResult, McpError> {
        registry_result(self.registry.knowledge_recipe(&args.recipe).await)
    }

    #[tool(
        description = "List the controlled vocabularies of the knowledge layer: concept \
                          slugs (actions/observables like `swap` or `add-liquidity`, \
                          with synonyms and related concepts) and category slugs \
                          (protocol classifications like `dex` or `launchpad`).\n\n\
                          Call this first when you want to filter search_knowledge by \
                          `concept`/`category`, or to map a user's phrasing onto a \
                          canonical concept slug.\n\n\
                          Requires an API key (`a4 auth login`)."
    )]
    async fn list_concepts(&self) -> Result<CallToolResult, McpError> {
        registry_result(self.registry.knowledge_vocabulary().await)
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

        let subscription_id = self.subscriptions.next_id();
        let mut query = SubscriptionQuery::new(&args.view);
        query.key = args.key.clone();
        let mut sub = Subscription::new(&subscription_id, query);
        if let Some(snap) = args.with_snapshot {
            sub = sub.with_snapshot(snap);
        }
        let lease = conn.manager.subscribe(sub).await.map_err(|error| {
            McpError::internal_error(format!("failed to subscribe: {error}"), None)
        })?;
        let entry = self.subscriptions.insert(
            subscription_id,
            args.connection_id.clone(),
            args.view.clone(),
            args.key.clone(),
            lease,
        );
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

        drop(entry);
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
        let (store, wire_subscription_id, view) =
            self.resolve_subscription(&args.subscription_id)?;

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
        let raw: Vec<serde_json::Value> = store.list_for_subscription(&wire_subscription_id).await;
        let total_scanned = raw.len();
        let mut matched: Vec<serde_json::Value> = Vec::new();
        for value in raw {
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
        let (store, wire_subscription_id, view) =
            self.resolve_subscription(&args.subscription_id)?;
        let value: Option<serde_json::Value> = store
            .get_for_subscription(&wire_subscription_id, &args.key)
            .await;
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
        let (store, wire_subscription_id, view) =
            self.resolve_subscription(&args.subscription_id)?;
        let all_keys = store.keys_for_subscription(&wire_subscription_id).await;
        let total_cached = all_keys.len();
        let keys: Vec<String> = all_keys.into_iter().take(QUERY_LIMIT_MAX).collect();
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

    #[tool(description = "Return up to N entities from a subscription's exact \
                          ordered query membership.")]
    async fn get_recent(
        &self,
        Parameters(args): Parameters<GetRecentArgs>,
    ) -> Result<CallToolResult, McpError> {
        let (store, wire_subscription_id, view) =
            self.resolve_subscription(&args.subscription_id)?;
        let n = args.n.min(QUERY_LIMIT_MAX);
        let all: Vec<serde_json::Value> = store.list_for_subscription(&wire_subscription_id).await;
        let total = all.len();
        let recent: Vec<serde_json::Value> = all.into_iter().take(n).collect();
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

/// Render a registry lookup as a tool result.
///
/// Registry failures split cleanly by cause: a bad `stack`/`program`/`kind`
/// argument is the agent's to fix and maps to `invalid_params`, while a
/// transport failure or a non-2xx from the platform is not, and maps to
/// `internal_error`. Getting this split wrong matters — agents retry
/// `invalid_params` with different arguments and give up on `internal_error`.
///
/// The body is emitted exactly as the registry sent it. `registry` already bounds
/// it to 512 KiB, and re-serializing would break that bound rather than preserve
/// it: JSON number formatting is not length-preserving, so `1e9` becomes
/// `1000000000.0` and an array of them grows over 3x — enough to carry a legal
/// body well past the cap. Pretty-printing is worse again. Passing the accepted
/// bytes through keeps the advertised bound true by construction and makes the
/// documented raw pass-through actually raw.
fn registry_result(result: anyhow::Result<String>) -> Result<CallToolResult, McpError> {
    match result {
        Ok(body) => Ok(CallToolResult::success(vec![Content::text(body)])),
        Err(error) => {
            let message = error.to_string();
            let caller_fixable = message.contains("must not be empty")
                || message.contains("invalid character")
                || message.contains("must not be a relative path segment")
                || message.contains("unknown artifact kind")
                || message.contains("requires at least one of")
                || message.contains("must be one of");
            Err(if caller_fixable {
                McpError::invalid_params(message, None)
            } else {
                McpError::internal_error(message, None)
            })
        }
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
    ) -> Result<(std::sync::Arc<arete_sdk::SharedStore>, String, String), McpError> {
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
        Ok((
            conn.store.clone(),
            sub.wire_subscription_id.clone(),
            sub.view.clone(),
        ))
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

#[cfg(test)]
mod registry_result_tests {
    use super::registry_result;
    use rmcp::model::ErrorCode;

    /// Every rejection a bad path segment can produce must classify as
    /// `invalid_params`. This is the difference between an agent retrying with
    /// a corrected reference and giving up: agents treat `internal_error` as
    /// "the server is broken, stop". The check is substring-based, so a reworded
    /// validation error silently falls through to `internal_error` — this test
    /// is what catches that.
    #[test]
    fn argument_rejections_are_invalid_params() {
        let client = crate::registry::RegistryClient::new();
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();

        for hash in [
            "",                    // empty
            "ore/../../agents",    // invalid character
            r"..\..\..\agents\me", // backslash traversal
            "..",                  // dot-only segment
        ] {
            // `path_segment` rejects before any request, so this never leaves
            // the process and needs no network.
            let err = registry_result(rt.block_on(client.artifact("program-spec", hash)))
                .expect_err("expected {hash:?} to be refused");
            assert_eq!(
                err.code,
                ErrorCode::INVALID_PARAMS,
                "{hash:?} should be caller-fixable, got: {}",
                err.message
            );
        }

        let err = registry_result(rt.block_on(client.artifact("not-a-kind", "abc")))
            .expect_err("unknown kind should be refused");
        assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
    }

    /// The knowledge tools' client-side validations must classify the same
    /// way: an empty search, a bad section, or a path-shaped slug are all the
    /// agent's to fix, and each fires before credential resolution or any
    /// network request, so this test is hermetic.
    #[test]
    fn knowledge_argument_rejections_are_invalid_params() {
        let client = crate::registry::RegistryClient::new();
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();

        let cases: Vec<(&str, Result<String, anyhow::Error>)> = vec![
            (
                "empty search",
                rt.block_on(client.knowledge_search(None, None, None, Some(5))),
            ),
            (
                "bad section",
                rt.block_on(client.knowledge_program("meteora-cp-amm", Some("everything"))),
            ),
            (
                "traversal protocol slug",
                rt.block_on(client.knowledge_protocol(r"..\..\agents\me")),
            ),
            (
                "traversal recipe slug",
                rt.block_on(client.knowledge_recipe("a/../b")),
            ),
        ];
        for (label, result) in cases {
            let err = registry_result(result).expect_err("expected the argument to be refused");
            assert_eq!(
                err.code,
                ErrorCode::INVALID_PARAMS,
                "{label} should be caller-fixable, got: {}",
                err.message
            );
        }
    }

    /// The tool result must be the registry's bytes, unchanged.
    ///
    /// Re-serializing a parsed `Value` would silently break both the documented
    /// raw pass-through and the 512 KiB bound, because JSON number formatting is
    /// not length-preserving: `1e9` comes back as `1000000000.0`, over 3x longer.
    /// An array of those turns a legal body into an oversized tool result.
    #[test]
    fn body_passes_through_verbatim() {
        let body = r#"{"big":1e9,"kept":"as-sent"}"#.to_string();
        let result = registry_result(Ok(body.clone())).expect("should succeed");
        let rendered = serde_json::to_string(&result).expect("result serializes");

        assert!(
            rendered.contains("1e9"),
            "number must survive as sent: {rendered}"
        );
        assert!(
            !rendered.contains("1000000000.0"),
            "number must not be reformatted: {rendered}"
        );
    }
}
