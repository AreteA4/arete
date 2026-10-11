//! The Arete MCP tool server: `AreteMcp` and its tool definitions.
//!
//! See HYP-189 for the design. The server speaks the Model Context Protocol
//! over stdio and exposes tools for AI agents to connect to Arete stacks,
//! subscribe to views, and query cached entities. See `connections.rs` for the
//! per-connection registry. Entry point: [`crate::serve_stdio`].

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

    pub fn opt_usize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<usize>, D::Error> {
        let v = Value::deserialize(d)?;
        value_to_usize::<D::Error>(v)
    }

    /// Parse an already-decoded value with the same leniency, for
    /// hand-written deserializers that collect several errors.
    pub fn value_usize(v: Value) -> Result<Option<usize>, serde_json::Error> {
        value_to_usize::<serde_json::Error>(v)
    }

    #[cfg(test)]
    mod tests {
        use serde::Deserialize;

        #[derive(Deserialize)]
        struct S {
            #[serde(default, deserialize_with = "super::opt_usize")]
            n: Option<usize>,
            #[serde(default, deserialize_with = "super::opt_usize")]
            limit: Option<usize>,
        }

        fn parse(json: &str) -> serde_json::Result<S> {
            serde_json::from_str(json)
        }

        #[test]
        fn accepts_int() {
            let s = parse(r#"{"n": 10, "limit": 5}"#).unwrap();
            assert_eq!(s.n, Some(10));
            assert_eq!(s.limit, Some(5));
        }

        #[test]
        fn accepts_string() {
            let s = parse(r#"{"n": "10", "limit": "5"}"#).unwrap();
            assert_eq!(s.n, Some(10));
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

use arete_sdk::{ApiProblemV1, AreteError, Subscription, SubscriptionQuery};
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::*,
    schemars, tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler,
};
use serde::{Deserialize, Serialize};

use crate::connections::ConnectionRegistry;
use crate::filter::{Filter, StructuredPredicate};
use crate::recovery::{RecoveryApiError, RecoveryClient};
use crate::registry::{RegistryClient, MAX_RESPONSE_BYTES};
use crate::stack_knowledge::{self, StackKnowledge, LOOKUP_TIMEOUT};
use crate::subscriptions::SubscriptionRegistry;
use crate::{catalog_view, credentials, descriptor, filter};

#[derive(Clone)]
pub struct AreteMcp {
    tool_router: ToolRouter<AreteMcp>,
    connections: ConnectionRegistry,
    subscriptions: SubscriptionRegistry,
    registry: RegistryClient,
    recovery: RecoveryClient,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ConnectArgs {
    /// WebSocket URL of the Arete stack
    /// (e.g. `wss://your-stack.stack.arete.run`).
    pub url: String,
    /// Optional explicit API key (override). If omitted, the server resolves
    /// the key from the `ARETE_API_KEY` env var, then from the a4 login
    /// (`a4 auth signup` / `a4 auth login`).
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

/// One string or a list of strings. Agents pass either shape; comma-separated
/// entries inside a string are split too.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(untagged)]
pub enum StringList {
    One(String),
    Many(Vec<String>),
}

impl StringList {
    fn into_vec(self) -> Vec<String> {
        match self {
            StringList::One(value) => vec![value],
            StringList::Many(values) => values,
        }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ExploreStackArgs {
    /// Bare stack reference as listed by `explore_stacks` (e.g. `ore`).
    /// Not a URL and not a path.
    pub stack: String,
    /// Return the compact summary: whether the stack is live, entities with
    /// their subscribable view ids and token-amount fields, the stack's
    /// `read.*` helpers, program SDKs, stream endpoints and stream auth. This
    /// is the default; `false` is the same as `full: true`.
    #[serde(default)]
    pub summary: Option<bool>,
    /// Only these selected views, each with its entity's field schema. View
    /// ids like `OreRound/latest`; prefix `alias:` when several LiveSpecs
    /// select the same id. A list or a comma-separated string.
    #[serde(default)]
    pub views: Option<StringList>,
    /// Return the whole pinned install descriptor (hundreds of KB for real
    /// stacks: identity hashes, every auth surface, SDK extension sources)
    /// instead of the summary.
    #[serde(default)]
    pub full: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ExploreStackSchemaArgs {
    /// Bare stack reference as listed by `explore_stacks` (e.g. `ore`).
    /// Not a URL and not a path.
    pub stack: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ExploreProgramArgs {
    /// Bare program reference as listed by `explore_programs`
    /// (e.g. `spl-token`), or a program ID.
    pub program: String,
    /// Return one operation: a semantic path such as
    /// `transactions.mining.deployWithCheckpoint`, a full operation id, a
    /// generated binding, or a raw IDL instruction name such as `deploy`.
    #[serde(default, rename = "operationId", alias = "operation_id")]
    pub operation_id: Option<String>,
    /// Return these sections in detail: `accounts`, `events`,
    /// `instructions` (with error codes), `operations` (every SDK surface
    /// entry), `types`. A list or a comma-separated string.
    #[serde(default)]
    pub sections: Option<StringList>,
    /// Return the whole pinned install descriptor (IDL, ProgramSpec and SDK
    /// extension sources) instead of the summary.
    #[serde(default)]
    pub full: Option<bool>,
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
    /// Maximum number of results, 10 by default. Accepts either an integer
    /// (`5`) or a string-encoded integer (`"5"`) because LLM tool-call
    /// arguments sometimes stringify numbers.
    #[serde(default, deserialize_with = "lenient::opt_usize")]
    pub limit: Option<usize>,
    /// Keep only these fields of each result: top-level keys or dotted paths
    /// such as `coverage.read`. A list or a comma-separated string. Replaces
    /// the brief default field set.
    #[serde(default)]
    pub fields: Option<StringList>,
    /// Return each result as the server sent it, including `score` and
    /// `coverage_via`. By default each result keeps only type, slug, name,
    /// protocol, summary and coverage.
    #[serde(default)]
    pub full: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchCatalogArgs {
    /// Free-text intent (e.g. `monitor swaps`). Matched against concept
    /// names and synonyms first, then curated knowledge via full-text
    /// search. At least one filter is required.
    #[serde(default)]
    pub query: Option<String>,
    /// Concept slug to require (e.g. `swap`). Discover slugs with
    /// `list_catalog_vocabulary`.
    #[serde(default)]
    pub concept: Option<String>,
    /// Category slug to filter by (e.g. `dex`).
    #[serde(default)]
    pub category: Option<String>,
    /// `program` or `stack`.
    #[serde(default)]
    pub kind: Option<String>,
    /// Required access mode: `build`, `read`, or `subscribe`.
    #[serde(default)]
    pub mode: Option<String>,
    /// Required verified SDK target: `typescript`, `rust`, or `python`.
    #[serde(default)]
    pub target: Option<String>,
    /// Maximum number of results (integer or string-encoded integer).
    /// Defaults to 10; continue with `cursor`.
    #[serde(default, deserialize_with = "lenient::opt_usize")]
    pub limit: Option<usize>,
    /// `nextCursor` from a previous `search_catalog` page. Repeat the same
    /// filters to fetch the next page; the server rejects a cursor once the
    /// active catalog changes.
    #[serde(default)]
    pub cursor: Option<String>,
    /// Keep only these fields of each result: top-level keys or dotted paths
    /// such as `delivery.status`. A list or a comma-separated string, e.g.
    /// `"slug,kind,name,version,modes,delivery.health"`. Replaces the brief
    /// default field set.
    #[serde(default)]
    pub fields: Option<StringList>,
    /// Return each result as the server sent it, including `concepts`,
    /// `score`, identity hashes (`packageReleaseHash`, `bundleHash`,
    /// `setHash`), `programIds` and release notes, but without the derived
    /// `live` and `related`. By default each result keeps only kind, slug,
    /// name, version, protocol, summary, modes, live (stacks), related,
    /// sdkTargets and delivery status/health.
    #[serde(default)]
    pub full: Option<bool>,
}

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
pub struct VocabularyArgs {
    /// Return every concept and category with its description, synonyms and
    /// related slugs. By default each item keeps only `slug` and `name`.
    #[serde(default)]
    pub full: Option<bool>,
}

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
pub struct ExploreStacksArgs {
    /// Keep only these fields of each stack: top-level keys or dotted paths
    /// such as `websocket_auth.mode`. A list or a comma-separated string.
    /// Replaces the brief default field set.
    #[serde(default)]
    pub fields: Option<StringList>,
    /// Return each stack as the server sent it, including `http_url`,
    /// `subdomain` and the full `websocket_auth`/`http_auth` objects. By
    /// default each stack keeps only `name`, `description`, `websocket_url`,
    /// `entities`, `visibility`, `serviceClass` and `websocket_auth.required`.
    #[serde(default)]
    pub full: Option<bool>,
}

/// Fields `explore_stacks` keeps by default: enough to pick a stack and
/// `connect` to it. `explore_stack` reports the auth requirements in full.
const STACK_LIST_BRIEF_FIELDS: &[&str] = &[
    "name",
    "description",
    "websocket_url",
    "entities",
    "visibility",
    "serviceClass",
    "websocket_auth.required",
];

fn stack_list_shape(fields: Option<StringList>, full: Option<bool>) -> catalog_view::Shape {
    match catalog_shape(fields, full) {
        catalog_view::Shape::Compact => catalog_view::Shape::Fields(
            STACK_LIST_BRIEF_FIELDS
                .iter()
                .map(|field| field.to_string())
                .collect(),
        ),
        shape => shape,
    }
}

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
pub struct ExploreProgramsArgs {
    /// Keep only these fields of each program: top-level keys such as
    /// `installName,programId,sdkTargets`. A list or a comma-separated string.
    #[serde(default)]
    pub fields: Option<StringList>,
    /// Return each program as the server sent it, including its release and
    /// spec hashes. By default hash fields are dropped.
    #[serde(default)]
    pub full: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetCatalogEntryArgs {
    /// `program` or `stack`.
    pub kind: String,
    /// Bare package slug as returned by `search_catalog` (e.g. `ore`). Not a
    /// URL and not a path.
    pub slug: String,
    /// Keep only these fields of the entry: top-level keys or dotted paths
    /// such as `delivery.status`. A list or a comma-separated string.
    #[serde(default)]
    pub fields: Option<StringList>,
    /// Return the entry as the server sent it, including identity hashes
    /// (`packageReleaseHash`, `bundleHash`, `setHash`, ...), `programIds` and
    /// release notes. The entry is compact by default.
    #[serde(default)]
    pub full: Option<bool>,
}

fn catalog_shape(fields: Option<StringList>, full: Option<bool>) -> catalog_view::Shape {
    let fields = catalog_view::parse_fields(&fields.map(StringList::into_vec).unwrap_or_default());
    catalog_view::Shape::from_args(fields, full == Some(true))
}

/// Search results default to the brief field set rather than the compact
/// entry shape: a page of results is for choosing, not for installing.
fn search_shape(fields: Option<StringList>, full: Option<bool>) -> catalog_view::Shape {
    match catalog_shape(fields, full) {
        catalog_view::Shape::Compact => catalog_view::brief(),
        shape => shape,
    }
}

const SEARCH_BRIEF_HINT: &str = "Brief fields per result. Pass `full: true` for every field \
     (concepts, score, identity hashes) or `fields` to choose keys; drill in with \
     `get_catalog_entry`.";

/// How to find a live stack, for the definition-only answer.
const FIND_LIVE_HINT: &str = "For a stack that streams now, call `search_catalog` with \
     `kind: \"stack\", mode: \"subscribe\"` (results with `live: true`).";

/// Shape a `search_catalog` body and attach a `hint` naming how to get more
/// fields and the next page. `full` bodies are returned as sent.
fn search_body(body: String, shape: &catalog_view::Shape) -> String {
    if *shape == catalog_view::Shape::Full {
        return body;
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&body) else {
        return body;
    };
    serde_json::to_string(&search_value(&value, shape, None)).unwrap_or(body)
}

/// Shape a search page (or the unfiltered overview) and attach its `hint`.
/// `overview` leads the hint when the page is the overview.
fn search_value(
    value: &serde_json::Value,
    shape: &catalog_view::Shape,
    overview: Option<&str>,
) -> serde_json::Value {
    let mut parts: Vec<&str> = overview.into_iter().collect();
    if *shape != catalog_view::Shape::Full {
        parts.push(SEARCH_BRIEF_HINT);
        if catalog_view::has_stacks(value) {
            parts.push(catalog_view::LIVE_HINT);
        }
    }
    if catalog_view::next_cursor(value).is_some() {
        parts.push("More results: pass `nextCursor` as `cursor` with the same filters.");
    }
    let shaped = catalog_view::shape_search(value, shape);
    if parts.is_empty() {
        shaped
    } else {
        catalog_view::with_hint(shaped, parts.join(" "))
    }
}

/// The hint leading the unfiltered `search_catalog` overview.
fn overview_hint(limit: usize, paged: bool) -> String {
    let mut hint = format!(
        "Catalog overview (no filters): up to {limit} programs and {limit} stacks. Narrow \
         with `query`, `concept` or `category` (slugs: `list_catalog_vocabulary`), or \
         `kind`."
    );
    if paged {
        hint.push_str(" Page one kind with `kind` and `cursor` set to `nextCursors.<kind>`.");
    }
    hint
}

/// Whether a catalog search names no filter at all.
fn unfiltered(args: &SearchCatalogArgs) -> bool {
    [
        &args.query,
        &args.concept,
        &args.category,
        &args.kind,
        &args.mode,
        &args.target,
    ]
    .iter()
    .all(|value| value.as_deref().is_none_or(|value| value.trim().is_empty()))
}

const KNOWLEDGE_BRIEF_HINT: &str = "Brief fields per result. Pass `full: true` for every \
     field (score, coverage_via) or `fields` to choose keys; drill in with `get_protocol`, \
     `get_program_knowledge` or `get_recipe`.";

/// Shape a `search_knowledge` body and attach a `hint`. Knowledge search has
/// no cursor, so a page cut at `limit` says to raise it.
fn knowledge_search_body(body: String, shape: &catalog_view::Shape, limit: usize) -> String {
    // `full` returns the response as sent, without a hint.
    if *shape == catalog_view::Shape::Full {
        return body;
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&body) else {
        return body;
    };
    let mut parts = vec![KNOWLEDGE_BRIEF_HINT.to_string()];
    if catalog_view::may_have_more(&value, limit) {
        parts.push(format!(
            "There may be more results: raise `limit` above {limit}."
        ));
    }
    let shaped = catalog_view::shape_search(&value, shape);
    serde_json::to_string(&catalog_view::with_hint(shaped, parts.join(" "))).unwrap_or(body)
}

/// Compact a vocabulary body to slugs and names unless `full` is set.
fn vocabulary_body(body: String, full: bool) -> String {
    if full {
        return body;
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&body) else {
        return body;
    };
    let compact = catalog_view::with_hint(
        catalog_view::compact_vocabulary(&value),
        "Slugs and names only. Pass `full: true` for descriptions, synonyms and related slugs.",
    );
    serde_json::to_string(&compact).unwrap_or(body)
}

/// Shape a stack or program list body (a JSON array). The list stays an
/// array, so a hint cannot be attached; the tool descriptions document
/// `full`/`fields`.
fn list_body(body: String, shape: &catalog_view::Shape) -> String {
    catalog_view::shape_body(body, shape, true)
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

/// Arguments for `get_recent`. Deserialization is hand-written so a bad call
/// reports every missing or invalid field at once, with an example, instead of
/// serde's first failure only.
#[derive(Debug, schemars::JsonSchema)]
pub struct GetRecentArgs {
    /// Subscription ID returned from `subscribe` (required).
    pub subscription_id: String,
    /// How many entities to return. Optional, defaults to 10, hard cap 1000.
    /// `limit` is accepted as an alias. Accepts either an integer (`5`) or a
    /// string-encoded integer (`"5"`) because LLM tool-call arguments
    /// sometimes stringify numbers.
    #[serde(default, alias = "limit")]
    pub n: Option<usize>,
}

const GET_RECENT_DEFAULT: usize = 10;
const GET_RECENT_EXAMPLE: &str = r#"{"subscription_id": "sub_1", "n": 10}"#;

impl<'de> Deserialize<'de> for GetRecentArgs {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;

        let value = serde_json::Value::deserialize(deserializer)?;
        let serde_json::Value::Object(map) = value else {
            return Err(D::Error::custom(format!(
                "get_recent arguments must be an object, e.g. {GET_RECENT_EXAMPLE}"
            )));
        };
        let mut problems = Vec::new();
        let subscription_id = match map.get("subscription_id") {
            Some(serde_json::Value::String(id)) if !id.trim().is_empty() => Some(id.clone()),
            Some(serde_json::Value::String(_)) => {
                problems.push("`subscription_id` must not be empty".to_string());
                None
            }
            Some(other) => {
                problems.push(format!("`subscription_id` must be a string, got {other}"));
                None
            }
            None => {
                problems
                    .push("missing `subscription_id` (the id returned by `subscribe`)".to_string());
                None
            }
        };
        let n = match (map.get("n"), map.get("limit")) {
            (Some(_), Some(_)) => {
                problems.push("pass either `n` or its alias `limit`, not both".to_string());
                None
            }
            (Some(value), None) | (None, Some(value)) => {
                let name = if map.contains_key("n") { "n" } else { "limit" };
                match lenient::value_usize(value.clone()) {
                    Ok(n) => n,
                    Err(error) => {
                        problems.push(format!("`{name}`: {error}"));
                        None
                    }
                }
            }
            (None, None) => None,
        };
        if !problems.is_empty() {
            return Err(D::Error::custom(format!(
                "invalid get_recent arguments: {}. Example: {GET_RECENT_EXAMPLE}",
                problems.join("; ")
            )));
        }
        Ok(GetRecentArgs {
            subscription_id: subscription_id.expect("validated above"),
            n,
        })
    }
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

/// Default and ceiling for how long `read_view` waits for its snapshot.
const READ_VIEW_TIMEOUT_DEFAULT_SECS: usize = 15;
const READ_VIEW_TIMEOUT_MAX_SECS: usize = 60;
const READ_VIEW_LIMIT_DEFAULT: usize = 10;

/// How long a cache read on a subscription whose snapshot has not arrived
/// yet waits for it, so a read straight after `subscribe` is not empty.
const SNAPSHOT_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ReadViewArgs {
    /// Bare stack reference as listed by `explore_stacks` (e.g. `ore`); its
    /// WebSocket endpoint is looked up in the registry. Pass `stack` or
    /// `url`.
    #[serde(default)]
    pub stack: Option<String>,
    /// WebSocket URL of the stack, instead of `stack` (e.g. a stack with
    /// several endpoints, or a local `ws://localhost:8878`).
    #[serde(default)]
    pub url: Option<String>,
    /// View id shaped `EntityName/mode`, e.g. `OreRound/latest`.
    pub view: String,
    /// Entity key, to read one entity (usually with an `EntityName/state`
    /// view).
    #[serde(default)]
    pub key: Option<String>,
    /// String-DSL filters, ANDed: `field=value`, `field>N`, `field~regex`,
    /// `field?`, `field!?`, `field!=value`, `field!~re`.
    #[serde(default)]
    pub r#where: Vec<String>,
    /// Structured filter predicates, ANDed with `where`.
    #[serde(default)]
    pub filters: Vec<StructuredPredicate>,
    /// Comma-separated dot-paths to project from each entity
    /// (e.g. `id.round_id,state.motherlode`).
    #[serde(default)]
    pub select: Option<String>,
    /// Most entities to return. Defaults to 10, capped at 1000.
    #[serde(default, deserialize_with = "lenient::opt_usize")]
    pub limit: Option<usize>,
    /// Seconds to wait for the connection and snapshot. Defaults to 15,
    /// capped at 60.
    #[serde(default, deserialize_with = "lenient::opt_usize")]
    pub timeout_secs: Option<usize>,
}

#[derive(Debug, Serialize)]
struct SubscriptionInfo {
    subscription_id: String,
    connection_id: String,
    view: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    key: Option<String>,
}

/// `subscribe` result: the subscription plus a `next` hint naming the call
/// that reads its entities.
#[derive(Debug, Serialize)]
struct SubscribeResponse {
    #[serde(flatten)]
    info: SubscriptionInfo,
    next: NextCall,
}

#[derive(Debug, Serialize)]
struct NextCall {
    tool: &'static str,
    arguments: serde_json::Value,
}

fn subscribe_response(info: SubscriptionInfo) -> SubscribeResponse {
    let arguments = serde_json::json!({
        "subscription_id": info.subscription_id,
        "n": GET_RECENT_DEFAULT,
    });
    SubscribeResponse {
        info,
        next: NextCall {
            tool: "get_recent",
            arguments,
        },
    }
}

#[derive(Debug, Serialize)]
struct ConnectionInfo {
    connection_id: String,
    url: String,
    state: String,
    /// Where the api key came from for this connect call. One of
    /// `explicit_argument`, `env:ARETE_API_KEY`, `a4-login`, or `none`.
    /// Never names where credentials are stored. Never contains the key
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
            recovery: RecoveryClient::new(),
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
                          global stacks are included too.\n\n\
                          Each stack is brief by default: `name`, `description`, \
                          `websocket_url`, `entities`, `visibility`, `serviceClass` and \
                          `websocket_auth.required`. Pass `full: true` for every field \
                          (`http_url`, `subdomain`, full `websocket_auth`/`http_auth`), \
                          or `fields` to choose keys; `explore_stack` reports one \
                          stack's auth requirements in full.")]
    async fn explore_stacks(
        &self,
        Parameters(args): Parameters<ExploreStacksArgs>,
    ) -> Result<CallToolResult, McpError> {
        let shape = stack_list_shape(args.fields, args.full);
        let body = self
            .registry_body(self.registry.list_stacks().await)
            .await?;
        bounded_result(list_body(body, &shape))
    }

    #[tool(
        description = "List explicitly curated `serviceClass=starter` stacks using the existing Arete registry. These are the authenticated stacks eligible for an agent trial. Public stacks remain available regardless of service class; use `explore_stacks` to see them. This tool is read-only and never creates an account or changes trial state."
    )]
    async fn explore_starter_stacks(&self) -> Result<CallToolResult, McpError> {
        self.registry_result(self.registry.list_starter_stacks().await)
            .await
    }

    #[tool(
        description = "Describe one stack from its pinned install descriptor.\n\n\
                          By default returns a compact summary (a few KB): `live` (whether \
                          it has a hosted stream), entities with their subscribable view \
                          ids and token-amount fields (`amountFields`: `scale` `ui` or \
                          `raw`, `decimals`), the stack's `read.*` helpers from its SDK \
                          extension (e.g. `read.currentRound()`), the program SDKs it \
                          carries, stream endpoints, and stream auth (accepted key \
                          classes, whether transactions need an entitlement).\n\n\
                          `views: [\"OreRound/latest\", \"OreMiner/state\"]` returns only \
                          those views with their entity field schemas. When the stack is \
                          published in the catalog, entities and views carry a curated \
                          `summary` and view fields a `description` with usage guidance, \
                          e.g. which of two similar fields a live UI should show. \
                          `full: true` returns the whole descriptor `a4 install` consumes \
                          (identity hashes, every auth surface and endpoint, \
                          StackManifest, LiveSpecs, programs, extension sources) — \
                          hundreds of KB, so ask for it only when you need those.\n\n\
                          A definition-only stack (a catalog package with no hosted \
                          stream) is not an error: the result has `kind: \
                          \"stack-definition-only\"`, `live: false` and `next` steps \
                          (install its SDK, or deploy it yourself).\n\n\
                          Pass a bare stack reference (e.g. `ore`), not a URL."
    )]
    async fn explore_stack(
        &self,
        Parameters(args): Parameters<ExploreStackArgs>,
    ) -> Result<CallToolResult, McpError> {
        let views = args
            .views
            .map(StringList::into_vec)
            .map(|views| descriptor::split_list(&views))
            .unwrap_or_default();
        let full = args.full == Some(true) || args.summary == Some(false);
        if full && !views.is_empty() {
            return Err(McpError::invalid_params(
                "`full` returns the whole descriptor; drop it (and `summary: false`) to select `views`"
                    .to_string(),
                None,
            ));
        }
        let body = match self.registry.stack_install(&args.stack).await {
            Ok(body) => body,
            Err(error) => {
                if let Some(answer) = self.definition_only(&args.stack, &error).await {
                    return shaped_result(&answer);
                }
                self.registry_body(Err(error)).await?
            }
        };
        if full {
            return full_descriptor_result(
                body,
                "Drop `full` for the summary (entities, views, endpoints and auth), or pass \
                 `views` for view fields.",
            );
        }
        let stack = parse_descriptor(Ok(body))?;
        // Guidance is pinned to the StackManifest this descriptor serves; a
        // descriptor that names none gets none.
        let knowledge = match stack
            .get("stackManifestHash")
            .and_then(serde_json::Value::as_str)
        {
            Some(stack_manifest_hash) => {
                let slug = stack
                    .get("name")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(&args.stack);
                let visibility = stack.get("visibility").and_then(serde_json::Value::as_str);
                self.stack_knowledge(slug, visibility, stack_manifest_hash)
                    .await
            }
            None => None,
        };
        let shaped = if views.is_empty() {
            let mut summary = descriptor::stack_summary(&stack, knowledge.as_ref());
            summary["next"] = serde_json::json!(
                "Pass `views` (e.g. [\"OreRound/latest\"], several at once) for view fields with descriptions and token-amount units; subscribe or read_view with those ids. Program operations: explore_program { program, operationId }. `full: true` for identity hashes, every auth surface and SDK metadata."
            );
            summary
        } else {
            descriptor::stack_views(&stack, &views, knowledge.as_ref())
                .map_err(|error| McpError::invalid_params(error.to_string(), None))?
        };
        shaped_result(&shaped)
    }

    #[tool(
        description = "Fetch the entity and view schema for one stack — field paths, \
                          types, primary keys, and the view ids `subscribe` accepts.\n\n\
                          Use this to resolve a `<EntityName>/<view>` id before calling \
                          subscribe, instead of guessing from a template.\n\n\
                          When the stack is published in the catalog, fields carry a \
                          curated `description` and entities and views a `summary`. \
                          Descriptions carry usage guidance, e.g. which of two similar \
                          fields a live UI should show or when a field fills in; read \
                          them before choosing fields. They are attached only when the \
                          knowledge was published for the StackManifest the registry \
                          serves for this stack. `knowledge` names the document they \
                          come from.\n\n\
                          Token amounts: a field the stack scales carries `amount`: \
                          `scale: \"ui\"` is whole token units (raw / 10^decimals, a \
                          float), `scale: \"raw\"` is integer base units (often a \
                          string-encoded u64; for SOL, lamports), with `decimals` (or \
                          `decimalsFrom`, the field holding them) and `counterpart`, the \
                          same amount at the other scale. Which token a field counts \
                          is in its `description` when the catalog has one. To read \
                          values, use `read_view`. A definition-only stack (no hosted \
                          stream) returns `kind: \"stack-definition-only\"` with `next` \
                          steps instead of an error."
    )]
    async fn explore_stack_schema(
        &self,
        Parameters(args): Parameters<ExploreStackSchemaArgs>,
    ) -> Result<CallToolResult, McpError> {
        let body = match self.registry.stack_schema(&args.stack).await {
            Ok(body) => body,
            Err(error) => {
                if let Some(answer) = self.definition_only(&args.stack, &error).await {
                    return shaped_result(&answer);
                }
                self.registry_body(Err(error)).await?
            }
        };
        let schema: serde_json::Value = parse_descriptor(Ok(body.clone()))?;
        // The response names the catalog package it resolved to.
        let slug = schema
            .get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(&args.stack);
        // The install descriptor pins the knowledge to the served
        // StackManifest and carries the LiveSpecs that say how token
        // amounts are scaled. Both lookups only ever add to the output.
        let deadline = std::time::Instant::now() + LOOKUP_TIMEOUT;
        let descriptor = self
            .registry
            .stack_install_within(&args.stack, Some(LOOKUP_TIMEOUT))
            .await
            .ok();
        let knowledge = self
            .schema_knowledge(slug, descriptor.as_deref(), deadline)
            .await;
        let descriptor = descriptor.and_then(|body| serde_json::from_str(&body).ok());
        described_schema_result(body, schema, knowledge.as_ref(), descriptor.as_ref())
    }

    #[tool(
        description = "List standalone Solana programs installable from the Arete \
                          registry, independent of any stack. No auth required.\n\n\
                          Each program keeps its install name, display name, program id \
                          and SDK targets; release and spec hashes are dropped by \
                          default. Pass `full: true` for every field, or `fields` (e.g. \
                          `\"installName,sdkTargets\"`) to keep only those keys. Drill in \
                          with `explore_program`."
    )]
    async fn explore_programs(
        &self,
        Parameters(args): Parameters<ExploreProgramsArgs>,
    ) -> Result<CallToolResult, McpError> {
        let shape = catalog_shape(args.fields, args.full);
        let body = self
            .registry_body(self.registry.list_programs().await)
            .await?;
        bounded_result(list_body(body, &shape))
    }

    #[tool(
        description = "Describe one standalone program from its pinned install \
                          descriptor.\n\n\
                          By default returns a compact summary: identity, the names of \
                          its accounts, instructions, events and types with the first \
                          line of each instruction's and account's docs \
                          (`descriptions`), the PDAs it derives with their seeds \
                          (`pdas`), its semantic SDK operations (e.g. \
                          `transactions.mining.deployWithCheckpoint`), and its Program \
                          Read and transaction transports.\n\n\
                          `operationId` returns one operation: generated paths, input \
                          type, required versus derived accounts, signers, transaction \
                          count, program errors, transport and scopes, and a minimal \
                          usage line. It accepts a semantic path, a full operation id, or \
                          a raw IDL instruction name (`deploy`). `sections` returns \
                          `accounts`, `events`, `instructions`, `operations` or `types` \
                          in detail. `full: true` returns the whole descriptor (identity \
                          hashes, IDL, ProgramSpec, SDK extension sources) — hundreds of \
                          KB.\n\n\
                          Semantic operations come from the knowledge surface, which \
                          needs an API key (`a4 auth login`); without one, raw IDL \
                          instructions still resolve. Pass a bare program reference \
                          (e.g. `spl-token`), not a URL."
    )]
    async fn explore_program(
        &self,
        Parameters(args): Parameters<ExploreProgramArgs>,
    ) -> Result<CallToolResult, McpError> {
        let operation = args
            .operation_id
            .map(|operation| operation.trim().to_string())
            .filter(|operation| !operation.is_empty());
        let sections = descriptor::parse_sections(
            &args.sections.map(StringList::into_vec).unwrap_or_default(),
            "sections",
        )
        .map_err(|error| McpError::invalid_params(error.to_string(), None))?;
        let full = args.full == Some(true);
        if full && (operation.is_some() || !sections.is_empty()) {
            return Err(McpError::invalid_params(
                "`full` returns the whole descriptor; drop it to select `operationId` or `sections`"
                    .to_string(),
                None,
            ));
        }
        if operation.is_some() && !sections.is_empty() {
            return Err(McpError::invalid_params(
                "pass either `operationId` or `sections`, not both".to_string(),
                None,
            ));
        }
        let body = self
            .registry_body(self.registry.program_install(&args.program).await)
            .await?;
        if full {
            return full_descriptor_result(
                body,
                "Drop `full` for the summary, or pass `operationId` or `sections` for detail.",
            );
        }
        let program = parse_descriptor(Ok(body))?;
        let needs_surface = sections.is_empty() || sections.iter().any(|s| s == "operations");
        let surface = if needs_surface {
            self.program_surface(&program).await
        } else {
            Err("not requested".to_string())
        };
        let surface_state = surface.as_ref().map_err(String::as_str);
        let shaped = if let Some(operation) = operation {
            descriptor::program_operation(&program, surface_state, &operation)
                .map_err(|error| McpError::invalid_params(error.to_string(), None))?
        } else if !sections.is_empty() {
            descriptor::program_sections(&program, surface_state, &sections)
        } else {
            let mut summary = descriptor::program_summary(&program, surface_state);
            summary["next"] = serde_json::json!(
                "Pass `operationId` for one operation, `sections` (accounts, events, instructions, operations, types) for detail, or `full: true` for the whole descriptor with identity hashes."
            );
            summary
        };
        shaped_result(&shaped)
    }

    #[tool(
        description = "Fetch a content-addressed artifact by hash. `kind` must be one \
                          of `program-spec`, `live-spec`, or `stack-manifest`; the hash \
                          comes from the full explore_stack or explore_program descriptor \
                          (`full: true`: `stackManifestHash`, `liveSpecHash`, \
                          `programSpecHash`).\n\n\
                          Large artifacts are refused rather than truncated — use the \
                          `a4` CLI for those."
    )]
    async fn resolve_artifact(
        &self,
        Parameters(args): Parameters<ResolveArtifactArgs>,
    ) -> Result<CallToolResult, McpError> {
        self.registry_result(self.registry.artifact(&args.kind, &args.hash).await)
            .await
    }

    #[tool(
        description = "Search the active public Arete catalog for installable programs \
                          and stacks by intent. Start here: every result is a catalog \
                          entry with a verified SDK target, curated knowledge, and \
                          evidenced capabilities (`modes`: build/read/subscribe), the \
                          exact `version` to install, and a sanitized `delivery.health` \
                          (`ready` or `degraded`).\n\n\
                          `query` is free text; `concept`/`category` filter by slug \
                          (see `list_catalog_vocabulary`); `kind`, `mode`, and `target` \
                          narrow to what you can actually use. Without any filter the \
                          tool returns an overview: up to 5 programs and 5 stacks (or \
                          `limit` of each), with `nextCursors.program`/`nextCursors.stack` \
                          to page one kind. Pages hold 10 results unless you pass `limit`; when a \
                          page returns `nextCursor`, pass it back as `cursor` with the \
                          same filters to continue. Drill in with `get_catalog_entry`.\n\n\
                          Results are brief by default: each keeps only `kind`, `slug`, \
                          `name`, `version`, `protocol`, `summary`, `modes`, `sdkTargets` \
                          and `delivery.status`/`delivery.health`, plus two derived keys: \
                          `live` on stacks (`true`: a hosted stream to subscribe to now, \
                          i.e. `modes` include `subscribe`; `false`: definition-only, \
                          install its SDK or deploy it yourself; `mode: \"subscribe\"` \
                          returns only live stacks) and `related`, the other results on \
                          the page with the same `protocol` as `kind:slug` (program, \
                          stack and protocol slugs often differ). The response carries a \
                          top-level `hint`. Pass `fields` (e.g. \
                          `\"slug,kind,name,version,modes,delivery.health\"`, dotted paths \
                          allowed) to keep only those keys, or `full: true` for every \
                          field as the server sent it.\n\n\
                          No credential is required; an API key widens results to \
                          global-visibility entries."
    )]
    async fn search_catalog(
        &self,
        Parameters(args): Parameters<SearchCatalogArgs>,
    ) -> Result<CallToolResult, McpError> {
        let is_unfiltered = unfiltered(&args);
        let shape = search_shape(args.fields, args.full);
        if is_unfiltered {
            if args.cursor.as_deref().is_some_and(|c| !c.trim().is_empty()) {
                return Err(McpError::invalid_params(
                    "`cursor` continues a search: repeat the filters of the page that returned it \
                     (e.g. `kind: \"program\"` with `nextCursors.program`)"
                        .to_string(),
                    None,
                ));
            }
            let limit = args.limit.unwrap_or(catalog_view::OVERVIEW_LIMIT);
            let mut pages = Vec::new();
            for kind in catalog_view::OVERVIEW_KINDS {
                let body = self
                    .registry_body(
                        self.registry
                            .catalog_search(
                                None,
                                None,
                                None,
                                Some(kind),
                                None,
                                None,
                                Some(limit),
                                None,
                            )
                            .await,
                    )
                    .await?;
                pages.push((kind, parse_descriptor(Ok(body))?));
            }
            let overview = catalog_view::merge_overview(&pages);
            let hint = overview_hint(limit, overview.get("nextCursors").is_some());
            return shaped_result(&search_value(&overview, &shape, Some(&hint)));
        }
        let body = self
            .registry_body(
                self.registry
                    .catalog_search(
                        args.query.as_deref(),
                        args.concept.as_deref(),
                        args.category.as_deref(),
                        args.kind.as_deref(),
                        args.mode.as_deref(),
                        args.target.as_deref(),
                        Some(args.limit.unwrap_or(catalog_view::DEFAULT_SEARCH_LIMIT)),
                        args.cursor.as_deref(),
                    )
                    .await,
            )
            .await?;
        bounded_result(search_body(body, &shape))
    }

    #[tool(
        description = "Fetch one active catalog entry by kind and slug: exact package \
                          version, the curated knowledge summary, verified SDK targets, \
                          capabilities keyed by stable language-neutral `operationId` \
                          values (e.g. `program/<programId>/raw-instruction/deploy`), and \
                          sanitized delivery state. Use after `search_catalog`; install \
                          the exact version shown with \
                          `a4 install <kind> <slug>@=<version>`.\n\n\
                          The entry is compact by default: identity hashes \
                          (`packageReleaseHash`, `bundleHash`, `setHash`, ...), \
                          `programIds` and release notes are dropped, and a stack gains \
                          `live` (whether it has a hosted stream). Pass `full: true` \
                          to get them (e.g. to confirm the lockfile records the same \
                          `packageReleaseHash`), or `fields` (dotted paths allowed) to \
                          keep only specific keys."
    )]
    async fn get_catalog_entry(
        &self,
        Parameters(args): Parameters<GetCatalogEntryArgs>,
    ) -> Result<CallToolResult, McpError> {
        self.catalog_result(
            self.registry.catalog_entry(&args.kind, &args.slug).await,
            &catalog_shape(args.fields, args.full),
            false,
        )
        .await
    }

    #[tool(
        description = "List the concept and category vocabularies of the active \
                          catalog snapshot. Use it to map a user's phrasing onto \
                          `search_catalog` concept/category slugs. No credential required.\n\n\
                          By default each concept and category keeps only `slug` and \
                          `name`. Pass `full: true` for descriptions, synonyms and related \
                          slugs when a name alone does not settle the mapping."
    )]
    async fn list_catalog_vocabulary(
        &self,
        Parameters(args): Parameters<VocabularyArgs>,
    ) -> Result<CallToolResult, McpError> {
        let body = self
            .registry_body(self.registry.catalog_vocabulary().await)
            .await?;
        bounded_result(vocabulary_body(body, args.full == Some(true)))
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
                          Results are brief by default (10 of them unless you pass \
                          `limit`): each keeps `type`, `slug`, `name`, `protocol`, \
                          `summary` and `coverage`, and the response carries a top-level \
                          `hint`. Pass `full: true` for every field (`score`, \
                          `coverage_via`) or `fields` to choose keys.\n\n\
                          AUTH: unlike the explore_* tools, this requires an Arete API \
                          key (`ARETE_API_KEY` env var, or the file `a4 auth login` \
                          writes)."
    )]
    async fn search_knowledge(
        &self,
        Parameters(args): Parameters<SearchKnowledgeArgs>,
    ) -> Result<CallToolResult, McpError> {
        let shape = search_shape(args.fields, args.full);
        let limit = args.limit.unwrap_or(catalog_view::DEFAULT_SEARCH_LIMIT);
        let body = self
            .registry_body(
                self.registry
                    .knowledge_search(
                        args.query.as_deref(),
                        args.concept.as_deref(),
                        args.category.as_deref(),
                        Some(limit),
                    )
                    .await,
            )
            .await?;
        bounded_result(knowledge_search_body(body, &shape, limit))
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
        self.registry_result(self.registry.knowledge_protocol(&args.protocol).await)
            .await
    }

    #[tool(
        description = "Fetch curated annotations for one Solana program by slug \
                          (e.g. `meteora-cp-amm`). `section` selects what comes \
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
        self.registry_result(
            self.registry
                .knowledge_program(&args.program, args.section.as_deref())
                .await,
        )
        .await
    }

    #[tool(description = "Fetch one cross-protocol recipe by slug (e.g. \
                          `execute-presale-purchase-via-squads`): an ordered, curated \
                          sequence of steps for a multi-protocol pattern (e.g. wrap a \
                          prepared transaction in a Squads multisig), each step \
                          referencing a real SDK surface entry (resolved in the \
                          response), plus a path to working example code.\n\n\
                          Use when search_knowledge returns a `recipe` result or a \
                          protocol's `related` edges cite one as evidence.\n\n\
                          Pass a bare slug, not a URL. Requires an API key \
                          (`a4 auth login`).")]
    async fn get_recipe(
        &self,
        Parameters(args): Parameters<GetRecipeArgs>,
    ) -> Result<CallToolResult, McpError> {
        self.registry_result(self.registry.knowledge_recipe(&args.recipe).await)
            .await
    }

    #[tool(
        description = "List the controlled vocabularies of the knowledge layer: concept \
                          slugs (actions/observables like `swap` or `add-liquidity`) and \
                          category slugs (protocol classifications like `dex` or \
                          `launchpad`).\n\n\
                          Call this first when you want to filter search_knowledge by \
                          `concept`/`category`, or to map a user's phrasing onto a \
                          canonical concept slug. By default each item keeps only `slug` \
                          and `name`; pass `full: true` for descriptions, synonyms and \
                          related concepts.\n\n\
                          Requires an API key (`a4 auth login`)."
    )]
    async fn list_concepts(
        &self,
        Parameters(args): Parameters<VocabularyArgs>,
    ) -> Result<CallToolResult, McpError> {
        let body = self
            .registry_body(self.registry.knowledge_vocabulary().await)
            .await?;
        bounded_result(vocabulary_body(body, args.full == Some(true)))
    }

    #[tool(
        description = "Show the authenticated agent account, including slug, status, plan, entitlement expiry, claim state, whether trial access is enabled, and starter-stack guidance. The API key is resolved from ARETE_API_KEY or the a4 login and is never returned."
    )]
    async fn account_status(&self) -> Result<CallToolResult, McpError> {
        match self.recovery.account_status().await {
            Ok(status) => Ok(CallToolResult::success(vec![Content::text(
                serde_json::to_string(&status).unwrap_or_default(),
            )])),
            Err(error) => Err(self.recovery_error(error).await),
        }
    }

    #[tool(
        description = "Create a short-lived ownership link for the authenticated agent. Returns MCP URL elicitation so the link can be handed to a human. The agent must not open or complete the link itself."
    )]
    async fn create_claim_link(&self) -> Result<CallToolResult, McpError> {
        match self.recovery.create_claim_link().await {
            Ok(ready) => Err(claim_url_elicitation(&ready.action)),
            Err(error) => Err(self.recovery_error(error).await),
        }
    }

    #[tool(description = "Open a WebSocket connection to a Arete stack. \
                          Returns a connection_id used by subscribe and query tools.\n\n\
                          AUTH: Prefer omitting `api_key` in agent calls — the \
                          server resolves it automatically from (1) explicit arg, \
                          (2) `ARETE_API_KEY` env var, (3) the a4 login \
                          (`a4 auth signup` / `a4 auth login`). Passing the key as an argument puts it \
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

        let id = match self
            .connections
            .connect(args.url.clone(), resolved.key)
            .await
        {
            Ok(id) => id,
            Err(error) => return Err(self.sdk_error(error).await),
        };

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

    #[tool(
        description = "Read the current entities of a view once: connects, subscribes, \
                          waits for the snapshot, returns the entities and disconnects. \
                          Use this to answer \"what is the current X\" (e.g. the current \
                          ORE round: `{\"stack\": \"ore\", \"view\": \"OreRound/latest\", \
                          \"limit\": 1}`) \
                          instead of connect + subscribe + get_recent, and instead of \
                          writing a script. Use `subscribe` only to follow changes over \
                          time.\n\n\
                          Pass `stack` (a bare reference like `ore`) or `url`. Auth is \
                          resolved like `connect`'s, so omit any key. `key` reads one \
                          entity; `where`/`filters` and `select` work as in \
                          `query_entities`; `limit` defaults to 10 (entities come in \
                          the view's order, so `limit: 1` on a sorted view like \
                          `OreRound/latest` is its current entity).\n\n\
                          Token amounts: a float field is usually in whole token units \
                          and an integer (often string-encoded) one in raw base units. \
                          `explore_stack_schema` reports each scaled field's `amount` \
                          (`scale`: `ui` or `raw`, `decimals`) — check it before \
                          converting or labelling values."
    )]
    async fn read_view(
        &self,
        Parameters(args): Parameters<ReadViewArgs>,
    ) -> Result<CallToolResult, McpError> {
        validate_view_name(&args.view)?;
        let mut compiled = Filter::parse(&args.r#where)
            .map_err(|e| McpError::invalid_params(format!("invalid where: {e}"), None))?;
        let structured = Filter::from_structured(&args.filters)
            .map_err(|e| McpError::invalid_params(format!("invalid filters: {e}"), None))?;
        compiled.extend(structured);
        let select_paths = args.select.as_deref().map(filter::parse_select);
        let limit = args
            .limit
            .unwrap_or(READ_VIEW_LIMIT_DEFAULT)
            .clamp(1, QUERY_LIMIT_MAX);
        let timeout = std::time::Duration::from_secs(
            args.timeout_secs
                .unwrap_or(READ_VIEW_TIMEOUT_DEFAULT_SECS)
                .clamp(1, READ_VIEW_TIMEOUT_MAX_SECS) as u64,
        );

        // One deadline covers the stack lookup, the connection and the
        // snapshot.
        let deadline = tokio::time::Instant::now() + timeout;
        let url =
            match (args.url, args.stack) {
                (Some(url), _) if !url.trim().is_empty() => url.trim().to_string(),
                (_, Some(stack)) if !stack.trim().is_empty() => {
                    self.stack_websocket_url(&stack, deadline).await?
                }
                _ => return Err(McpError::invalid_params(
                    "pass `stack` (a bare reference like `ore`, from `explore_stacks`) or `url` \
                     (the stack's WebSocket URL)"
                        .to_string(),
                    None,
                )),
            };
        let resolved = credentials::resolve(None, &url)
            .map_err(|e| McpError::invalid_params(e.to_string(), None))?;

        // Without filters the server can cut the snapshot to `limit`; with
        // them the whole snapshot is needed to find the matches.
        let take = (args.key.is_none() && compiled.is_empty()).then_some(limit);
        let entities = match crate::oneshot::read_view(
            url.clone(),
            resolved.key,
            args.view.trim(),
            args.key.clone(),
            take,
            deadline,
        )
        .await
        {
            Ok(entities) => entities,
            Err(crate::oneshot::ReadError::Sdk(error)) => return Err(self.sdk_error(error).await),

            Err(crate::oneshot::ReadError::TimedOut { waiting_for }) => {
                return Err(McpError::internal_error(
                    format!(
                        "timed out after {}s waiting for {waiting_for} of `{}`. Check the view id \
                         with explore_stack_schema (it must be `EntityName/mode`), or raise \
                         `timeout_secs`.",
                        timeout.as_secs(),
                        args.view
                    ),
                    None,
                ))
            }
        };

        let total = entities.len();
        let mut matched = Vec::new();
        for value in entities {
            if !compiled.is_empty() && !compiled.matches(&value) {
                continue;
            }
            matched.push(match &select_paths {
                Some(paths) => filter::select_fields(&value, paths),
                None => value,
            });
        }
        let matching = matched.len();
        matched.truncate(limit);
        if let Some(key) = &args.key {
            if total == 0 {
                return Err(McpError::invalid_params(
                    format!(
                        "no entity with key `{key}` in `{}`. Keyed reads usually need the \
                         entity's `/state` view; check the key format with read_view on \
                         `EntityName/list`.",
                        args.view
                    ),
                    None,
                ));
            }
        }
        let mut payload = serde_json::json!({
            "view": args.view.trim(),
            "total": total,
            "matching": matching,
            "returned": matched.len(),
            "truncated": matching > matched.len(),
            "entities": matched,
        });
        if let Some(key) = &args.key {
            payload["key"] = serde_json::json!(key);
        }
        // A large view can exceed the tool-result cap; `select` or a
        // smaller `limit` narrows it.
        shaped_result(&payload)
    }

    #[tool(description = "Subscribe to a Arete view on an existing connection. \
                          Streamed entities land in an in-memory cache that the query \
                          tools (get_entity, list_entities, get_recent, query_entities) \
                          read from. To read a view's current value once, use \
                          `read_view` instead: it needs no connection or subscription.\n\n\
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
            serde_json::to_string(&subscribe_response(info)).unwrap_or_default(),
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
                          to 100 and is capped at 1000. Waits up to 5s for the \
                          subscription's snapshot (`ready`).\n\n\
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
        let ready = store
            .wait_for_subscription_ready(&wire_subscription_id, SNAPSHOT_WAIT)
            .await;

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
            "ready": ready,
            "total_scanned": total_scanned,
            "returned": matched.len(),
            "limit_applied": limit,
            "entities": matched,
        });
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string(&payload).unwrap_or_default(),
        )]))
    }

    #[tool(
        description = "Fetch a single entity by key from a subscription's cache. \
                          Waits up to 5s for the subscription's snapshot (`ready`). To \
                          read one entity without a subscription, use `read_view` with \
                          `key`."
    )]
    async fn get_entity(
        &self,
        Parameters(args): Parameters<GetEntityArgs>,
    ) -> Result<CallToolResult, McpError> {
        let (store, wire_subscription_id, view) =
            self.resolve_subscription(&args.subscription_id)?;
        let ready = store
            .wait_for_subscription_ready(&wire_subscription_id, SNAPSHOT_WAIT)
            .await;
        let value: Option<serde_json::Value> = store
            .get_for_subscription(&wire_subscription_id, &args.key)
            .await;
        let payload = serde_json::json!({
            "view": view,
            "ready": ready,
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
        let ready = store
            .wait_for_subscription_ready(&wire_subscription_id, SNAPSHOT_WAIT)
            .await;
        let all_keys = store.keys_for_subscription(&wire_subscription_id).await;
        let total_cached = all_keys.len();
        let keys: Vec<String> = all_keys.into_iter().take(QUERY_LIMIT_MAX).collect();
        let truncated = total_cached > keys.len();
        let payload = serde_json::json!({
            "view": view,
            "ready": ready,
            "total_cached": total_cached,
            "returned": keys.len(),
            "truncated": truncated,
            "keys": keys,
        });
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string(&payload).unwrap_or_default(),
        )]))
    }

    #[tool(description = "Return up to `n` entities from a subscription's exact \
                          ordered query membership.\n\n\
                          Parameters: `subscription_id` (required, string returned by \
                          `subscribe`); `n` (optional integer, default 10, max 1000; \
                          `limit` is accepted as an alias). \
                          Example: {\"subscription_id\": \"sub_1\", \"n\": 10}\n\n\
                          Waits up to 5s for the subscription's snapshot; `ready: false` \
                          means it has not arrived (check the view id). For a one-off \
                          read without a subscription, use `read_view`.")]
    async fn get_recent(
        &self,
        Parameters(args): Parameters<GetRecentArgs>,
    ) -> Result<CallToolResult, McpError> {
        let (store, wire_subscription_id, view) =
            self.resolve_subscription(&args.subscription_id)?;
        let ready = store
            .wait_for_subscription_ready(&wire_subscription_id, SNAPSHOT_WAIT)
            .await;
        let n = args.n.unwrap_or(GET_RECENT_DEFAULT).min(QUERY_LIMIT_MAX);
        let all: Vec<serde_json::Value> = store.list_for_subscription(&wire_subscription_id).await;
        let total = all.len();
        let recent: Vec<serde_json::Value> = all.into_iter().take(n).collect();
        let payload = serde_json::json!({
            "view": view,
            "ready": ready,
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
        Err(error) => Err(registry_error(error)),
    }
}

/// Classify a registry client failure: argument rejections are the caller's
/// to fix (`invalid_params`); everything else is `internal_error`.
fn registry_error(error: anyhow::Error) -> McpError {
    let message = error.to_string();
    let caller_fixable = message.contains("must not be empty")
        || message.contains("invalid character")
        || message.contains("must not be a relative path segment")
        || message.contains("unknown artifact kind")
        || message.contains("requires at least one of")
        || message.contains("must be one of");
    if caller_fixable {
        McpError::invalid_params(message, None)
    } else {
        McpError::internal_error(message, None)
    }
}

/// Parse a registry descriptor for client-side shaping. Registry failures keep
/// the classification [`registry_result`] gives them.
fn parse_descriptor(body: anyhow::Result<String>) -> Result<serde_json::Value, McpError> {
    let body = body.map_err(registry_error)?;
    serde_json::from_str(&body)
        .map_err(|error| McpError::internal_error(format!("invalid registry JSON: {error}"), None))
}

/// A whole install descriptor (`full: true`), passed through unchanged.
///
/// Descriptors are read under the larger
/// [`MAX_DESCRIPTOR_BYTES`](crate::registry::MAX_DESCRIPTOR_BYTES) cap so they
/// can be shaped, so the tool-result cap is applied here, to the bytes that
/// would be returned. Exceeding it is the caller's to fix: `narrower` names the
/// arguments that return less.
fn full_descriptor_result(body: String, narrower: &str) -> Result<CallToolResult, McpError> {
    if body.len() > MAX_RESPONSE_BYTES {
        return Err(McpError::invalid_params(
            format!(
                "the full descriptor is {} bytes, over the {MAX_RESPONSE_BYTES} byte limit for a \
                 single tool result. {narrower}",
                body.len()
            ),
            None,
        ));
    }
    Ok(CallToolResult::success(vec![Content::text(body)]))
}

/// A shaped (summary, sections, views or operation) result. It is cut from a
/// descriptor that was already bounded, but the 512 KiB cap is re-checked on
/// the bytes actually returned so the advertised bound holds for every tool.
fn shaped_result(value: &serde_json::Value) -> Result<CallToolResult, McpError> {
    let text = serde_json::to_string(value)
        .map_err(|error| McpError::internal_error(error.to_string(), None))?;
    bounded_result(text)
}

/// Return reshaped text (a brief list, search page or vocabulary) only if it
/// still fits the 512 KiB tool-result cap. Re-serializing can expand what
/// the registry sent (numbers, an added `hint`), so the cap is re-checked on
/// the bytes actually returned.
fn bounded_result(text: String) -> Result<CallToolResult, McpError> {
    if text.len() > MAX_RESPONSE_BYTES {
        return Err(McpError::internal_error(
            format!(
                "shaped response is {} bytes, over the {MAX_RESPONSE_BYTES} byte limit for a single \
                 tool result. Narrow it (fewer `views`, `sections` or `fields`, a `select`, a smaller `limit`, \
                 or no `full`), or use `a4 explore` on the command line.",
                text.len()
            ),
            None,
        ));
    }
    Ok(CallToolResult::success(vec![Content::text(text)]))
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
    /// The knowledge surface for a program descriptor, or why it is
    /// unavailable (no API key, no knowledge for the program, or knowledge
    /// that describes a different program id).
    async fn program_surface(
        &self,
        program: &serde_json::Value,
    ) -> Result<descriptor::ProgramSurface, String> {
        let install_name = program
            .get("installName")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| "the descriptor names no install name".to_string())?;
        let program_id = program
            .pointer("/definition/programId")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| "the descriptor names no program id".to_string())?;
        let body = self
            .registry
            .knowledge_program(install_name, Some("surface"))
            .await
            .map_err(|error| error.to_string())?;
        let response: serde_json::Value =
            serde_json::from_str(&body).map_err(|error| error.to_string())?;
        descriptor::ProgramSurface::from_knowledge(&response, program_id)
            .map_err(|error| error.to_string())
    }

    async fn sdk_error(&self, error: AreteError) -> McpError {
        if let Some(problem) = error.api_problem() {
            return self.problem_error(problem, None, true).await;
        }
        if let Some(issue) = error.socket_issue() {
            let problem = ApiProblemV1 {
                schema_version: Some(1),
                error: issue.message.clone(),
                code: Some(issue.wire_code.clone()),
                retryable: Some(issue.retryable),
                request_id: None,
                retry_after_seconds: issue.retry_after,
                usage: issue.usage.clone(),
                action: issue.action.clone(),
                extra: Default::default(),
            };
            return self.problem_error(&problem, None, true).await;
        }

        let data = serde_json::json!({
            "code": error.auth_code().map(|code| code.as_wire()),
            "retryable": error.should_retry(),
            "retryAfterSeconds": error.retry_after(),
        });
        McpError::internal_error("Arete connection failed", Some(data))
    }

    async fn registry_body(&self, result: anyhow::Result<String>) -> Result<String, McpError> {
        match result {
            Ok(body) => Ok(body),
            Err(error) => {
                if let Some(api_error) = error.downcast_ref::<crate::registry::RegistryApiError>() {
                    return Err(self
                        .problem_error(&api_error.problem, Some(api_error.status.as_u16()), true)
                        .await);
                }
                Err(registry_error(error))
            }
        }
    }

    async fn registry_result(
        &self,
        result: anyhow::Result<String>,
    ) -> Result<CallToolResult, McpError> {
        let body = self.registry_body(result).await?;
        Ok(CallToolResult::success(vec![Content::text(body)]))
    }

    async fn catalog_result(
        &self,
        result: anyhow::Result<String>,
        shape: &catalog_view::Shape,
        search: bool,
    ) -> Result<CallToolResult, McpError> {
        let body = self.registry_body(result).await?;
        bounded_result(catalog_view::shape_body(body, shape, search))
    }

    async fn recovery_error(&self, error: anyhow::Error) -> McpError {
        if let Some(api_error) = error.downcast_ref::<RecoveryApiError>() {
            return self
                .problem_error(&api_error.problem, Some(api_error.status.as_u16()), false)
                .await;
        }
        McpError::internal_error(
            "Arete account request failed",
            Some(serde_json::json!({ "code": "account-request-failed" })),
        )
    }

    async fn problem_error(
        &self,
        problem: &ApiProblemV1,
        status: Option<u16>,
        allow_materialize: bool,
    ) -> McpError {
        if allow_materialize
            && problem
                .recovery_action()
                .is_some_and(|action| action.is_claim_agent_materializer())
        {
            return match self.recovery.create_claim_link().await {
                Ok(ready) => claim_url_elicitation(&ready.action),
                Err(_) => {
                    structured_problem_error(problem, status, Some("claim-materialization-failed"))
                }
            };
        }
        structured_problem_error(problem, status, None)
    }

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

impl AreteMcp {
    /// The definition-only answer for a stack whose install descriptor or
    /// schema lookup failed: the registry refused it as definition-only, or
    /// it was not found and the catalog lists a stack under the reference
    /// that is not live (one catalog request, on the failure path only).
    /// `None` keeps the original error.
    async fn definition_only(
        &self,
        stack: &str,
        error: &anyhow::Error,
    ) -> Option<serde_json::Value> {
        let api_error = error.downcast_ref::<crate::registry::RegistryApiError>()?;
        let registry_says =
            api_error.problem.code.as_deref() == Some(catalog_view::DEFINITION_ONLY_CODE);
        if !registry_says && api_error.status != reqwest::StatusCode::NOT_FOUND {
            return None;
        }
        let entry = self
            .registry
            .catalog_entry("stack", stack.trim())
            .await
            .ok()
            .and_then(|body| serde_json::from_str::<serde_json::Value>(&body).ok());
        catalog_view::definition_only_stack(
            stack.trim(),
            registry_says,
            // A not-found message would only mislead next to the answer.
            registry_says.then_some(api_error.problem.error.as_str()),
            entry.as_ref(),
            FIND_LIVE_HINT,
        )
    }

    /// The catalog knowledge of the stack published as `slug`, when there is
    /// one and it was published for `stack_manifest_hash`, the StackManifest
    /// being described. Any failure, including a registry without the route,
    /// means no knowledge. A stack that cannot have catalog knowledge is not
    /// looked up, and the lookup is abandoned after [`LOOKUP_TIMEOUT`].
    async fn stack_knowledge(
        &self,
        slug: &str,
        visibility: Option<&str>,
        stack_manifest_hash: &str,
    ) -> Option<StackKnowledge> {
        if !stack_knowledge::may_have_catalog_knowledge(slug, visibility) {
            return None;
        }
        let body = self
            .registry
            .catalog_entry_knowledge("stack", slug, LOOKUP_TIMEOUT)
            .await
            .ok()?;
        stack_knowledge_from_body(&body, stack_manifest_hash)
    }

    /// The single WebSocket endpoint the registry serves for `stack`.
    /// Abandoned at `deadline`.
    async fn stack_websocket_url(
        &self,
        stack: &str,
        deadline: tokio::time::Instant,
    ) -> Result<String, McpError> {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let lookup = self
            .registry
            .stack_install_within(stack.trim(), Some(remaining));
        let body = match tokio::time::timeout_at(deadline, lookup).await {
            Ok(result) => self.registry_body(result).await?,
            Err(_) => {
                return Err(McpError::internal_error(
                    format!(
                        "timed out after {}s looking up stack `{stack}` in the registry; \
                         retry, raise `timeout_secs`, or pass `url`",
                        remaining.as_secs()
                    ),
                    None,
                ))
            }
        };
        let descriptor: serde_json::Value = parse_descriptor(Ok(body))?;
        descriptor
            .get("websocketUrl")
            .and_then(serde_json::Value::as_str)
            .filter(|url| !url.is_empty())
            .map(str::to_string)
            .ok_or_else(|| {
                McpError::invalid_params(
                    format!(
                        "stack `{stack}` has no single WebSocket endpoint; pass `url` with one of \
                         the endpoints explore_stack lists"
                    ),
                    None,
                )
            })
    }

    /// The catalog knowledge for a schema response. The schema names no
    /// StackManifest, so the guidance is pinned to the StackManifest of
    /// `descriptor`, the install descriptor the registry serves for the same
    /// stack. It is omitted when that descriptor could not be read, names no
    /// StackManifest, or names another one, and when the knowledge cannot
    /// be fetched before `deadline`.
    async fn schema_knowledge(
        &self,
        slug: &str,
        descriptor: Option<&str>,
        deadline: std::time::Instant,
    ) -> Option<StackKnowledge> {
        if !stack_knowledge::may_have_catalog_knowledge(slug, None) {
            return None;
        }
        let descriptor = descriptor?;
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return None;
        }
        let knowledge = self
            .registry
            .catalog_entry_knowledge("stack", slug, remaining)
            .await
            .ok()?;
        schema_knowledge_from_bodies(&knowledge, descriptor)
    }
}

/// A knowledge response, kept only when it was published for
/// `stack_manifest_hash`.
fn stack_knowledge_from_body(body: &str, stack_manifest_hash: &str) -> Option<StackKnowledge> {
    let response = serde_json::from_str::<serde_json::Value>(body).ok()?;
    StackKnowledge::from_response(&response)
        .filter(|knowledge| knowledge.belongs_to(stack_manifest_hash))
}

/// A knowledge response, kept only when it was published for the
/// StackManifest `descriptor` (an install descriptor) serves.
fn schema_knowledge_from_bodies(knowledge: &str, descriptor: &str) -> Option<StackKnowledge> {
    let descriptor = serde_json::from_str::<serde_json::Value>(descriptor).ok()?;
    let stack_manifest_hash = descriptor
        .get("stackManifestHash")
        .and_then(serde_json::Value::as_str)?;
    stack_knowledge_from_body(knowledge, stack_manifest_hash)
}

/// A stack schema response (`body`, parsed as `schema`) with its catalog
/// knowledge and the token-amount scale of its fields (read from the
/// install `descriptor`) attached. With neither the registry's bytes pass
/// through unchanged.
fn described_schema_result(
    body: String,
    mut schema: serde_json::Value,
    knowledge: Option<&StackKnowledge>,
    descriptor: Option<&serde_json::Value>,
) -> Result<CallToolResult, McpError> {
    let amounts = descriptor
        .is_some_and(|descriptor| descriptor::annotate_schema_amounts(&mut schema, descriptor));
    if let Some(knowledge) = knowledge {
        stack_knowledge::describe_schema(&mut schema, knowledge);
    } else if !amounts {
        return registry_result(Ok(body));
    }
    shaped_result(&schema)
}

fn claim_url_elicitation(action: &arete_sdk::ReadyRecoveryAction) -> McpError {
    McpError::url_elicitation_required(
        "A human owner must claim this agent. The agent must not open this URL itself.",
        Some(serde_json::json!({
            "url": action.url,
            "elicitationId": action.elicitation_id,
            "expiresAt": action.expires_at,
            "actionType": action.action_type,
        })),
    )
}

fn structured_problem_error(
    problem: &ApiProblemV1,
    status: Option<u16>,
    secondary_code: Option<&str>,
) -> McpError {
    let mut data = serde_json::to_value(problem).unwrap_or_else(|_| serde_json::json!({}));
    if let Some(object) = data.as_object_mut() {
        if let Some(status) = status {
            object.insert("status".to_string(), serde_json::json!(status));
        }
        if let Some(secondary_code) = secondary_code {
            object.insert(
                "secondaryCode".to_string(),
                serde_json::json!(secondary_code),
            );
        }
    }
    McpError::internal_error(problem.error.clone(), Some(data))
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
mod explore_args_tests {
    use super::*;
    use rmcp::model::ErrorCode;

    #[test]
    fn explore_program_accepts_camel_and_snake_operation_ids_and_either_list_shape() {
        let args: ExploreProgramArgs = serde_json::from_value(serde_json::json!({
            "program": "ore",
            "operationId": "transactions.mining.deployWithCheckpoint",
            "sections": "accounts,types"
        }))
        .unwrap();
        assert_eq!(
            args.operation_id.as_deref(),
            Some("transactions.mining.deployWithCheckpoint")
        );
        assert_eq!(
            descriptor::parse_sections(&args.sections.unwrap().into_vec(), "sections").unwrap(),
            vec!["accounts", "types"]
        );

        let args: ExploreProgramArgs = serde_json::from_value(serde_json::json!({
            "program": "ore",
            "operation_id": "deploy",
            "sections": ["instructions"],
            "full": false
        }))
        .unwrap();
        assert_eq!(args.operation_id.as_deref(), Some("deploy"));
        assert_eq!(args.sections.unwrap().into_vec(), vec!["instructions"]);
    }

    #[test]
    fn explore_stack_defaults_to_the_summary() {
        let args: ExploreStackArgs =
            serde_json::from_value(serde_json::json!({"stack": "ore"})).unwrap();
        assert!(args.summary.is_none() && args.views.is_none() && args.full.is_none());
        let args: ExploreStackArgs = serde_json::from_value(serde_json::json!({
            "stack": "ore",
            "views": ["OreRound/latest", "OreMiner/list"]
        }))
        .unwrap();
        assert_eq!(args.views.unwrap().into_vec().len(), 2);
    }

    #[tokio::test]
    async fn conflicting_explore_arguments_are_invalid_params_before_any_request() {
        let server = AreteMcp::new();
        let err = server
            .explore_stack(Parameters(ExploreStackArgs {
                stack: "ore".into(),
                summary: None,
                views: Some(StringList::One("OreRound/latest".into())),
                full: Some(true),
            }))
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::INVALID_PARAMS);

        for args in [
            ExploreProgramArgs {
                program: "ore".into(),
                operation_id: Some("deploy".into()),
                sections: None,
                full: Some(true),
            },
            ExploreProgramArgs {
                program: "ore".into(),
                operation_id: Some("deploy".into()),
                sections: Some(StringList::One("accounts".into())),
                full: None,
            },
            ExploreProgramArgs {
                program: "ore".into(),
                operation_id: None,
                sections: Some(StringList::Many(vec!["idl".into()])),
                full: None,
            },
        ] {
            let err = server.explore_program(Parameters(args)).await.unwrap_err();
            assert_eq!(err.code, ErrorCode::INVALID_PARAMS, "{}", err.message);
        }
    }

    /// A registry that answers each request path from `routes` (status and
    /// body; anything else is a 404), on a background thread.
    fn canned_registry(routes: Vec<(&'static str, u16, String)>) -> AreteMcp {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let mut request = Vec::new();
                let mut buffer = [0u8; 4096];
                while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                    match stream.read(&mut buffer) {
                        Ok(0) | Err(_) => break,
                        Ok(read) => request.extend_from_slice(&buffer[..read]),
                    }
                }
                let request = String::from_utf8_lossy(&request);
                let target = request.split_whitespace().nth(1).unwrap_or("");
                let target = target.split('?').next().unwrap_or("");
                let (status, body) = routes
                    .iter()
                    .find(|(route, _, _)| *route == target)
                    .map(|(_, status, body)| (*status, body.clone()))
                    .unwrap_or((404, r#"{"error":"not found"}"#.to_string()));
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        AreteMcp {
            registry: RegistryClient::with_base_url(&format!("http://{address}")),
            ..AreteMcp::new()
        }
    }

    fn pumpfun_entry(modes: serde_json::Value) -> String {
        serde_json::json!({
            "kind": "stack", "slug": "pumpfun", "version": "1.1.4", "protocol": "pump-fun",
            "summary": "Pump bonding curves.", "modes": modes, "bundleHash": "sha256:aa"
        })
        .to_string()
    }

    #[tokio::test]
    async fn definition_only_stacks_are_a_structured_answer_not_an_error() {
        let refusal = r#"{"schemaVersion":1,"error":"Stack 'pumpfun' is a definition-only package with no hosted stream","code":"stack-definition-only","retryable":false}"#;
        let server = canned_registry(vec![
            (
                "/api/registry/stacks/pumpfun/install",
                409,
                refusal.to_string(),
            ),
            (
                "/api/registry/v1/catalog/entries/stack/pumpfun",
                200,
                pumpfun_entry(serde_json::json!(["build", "read"])),
            ),
        ]);
        let result = server
            .explore_stack(Parameters(ExploreStackArgs {
                stack: "pumpfun".into(),
                summary: None,
                views: None,
                full: None,
            }))
            .await
            .unwrap();
        let answer: serde_json::Value = serde_json::from_str(&result_text(&result)).unwrap();
        assert_eq!(answer["kind"], "stack-definition-only");
        assert_eq!(answer["live"], false);
        assert!(answer["detail"]
            .as_str()
            .unwrap()
            .contains("definition-only"));
        assert_eq!(answer["catalog"]["protocol"], "pump-fun");
        assert!(answer["next"][2]
            .as_str()
            .unwrap()
            .contains("mode: \"subscribe\""));

        // Not found, but the catalog lists it as a definition-only stack.
        let result = server
            .explore_stack_schema(Parameters(ExploreStackSchemaArgs {
                stack: "pumpfun".into(),
            }))
            .await
            .unwrap();
        let answer: serde_json::Value = serde_json::from_str(&result_text(&result)).unwrap();
        assert_eq!(answer["kind"], "stack-definition-only");
        assert!(
            answer.get("detail").is_none(),
            "a not-found message would mislead"
        );
    }

    #[tokio::test]
    async fn a_missing_live_stack_keeps_its_error() {
        let server = canned_registry(vec![(
            "/api/registry/v1/catalog/entries/stack/pumpfun",
            200,
            pumpfun_entry(serde_json::json!(["build", "read", "subscribe"])),
        )]);
        let error = server
            .explore_stack(Parameters(ExploreStackArgs {
                stack: "pumpfun".into(),
                summary: None,
                views: None,
                full: None,
            }))
            .await
            .unwrap_err();
        assert!(error.message.contains("not found"), "{}", error.message);
    }

    /// A stack install descriptor padded past the tool-result cap by an
    /// embedded program artifact, as a stack with large IDLs is.
    fn oversized_stack_descriptor(padding: usize) -> String {
        serde_json::json!({
            "name": "big",
            "stack": "big-abc",
            "websocketUrl": "wss://big.test",
            "liveSpecs": [{
                "alias": "live",
                "artifact": {"payload": {"entities": [{
                    "state_name": "Round",
                    "identity": {"primary_keys": ["id.round_id"]},
                    "sections": [{"name": "id", "fields": [
                        {"field_name": "round_id", "rust_type_name": "u64", "is_optional": false}
                    ]}],
                    "views": [{"id": "Round/latest", "output": "Collection"}]
                }]}},
                "binding": {"websocketEndpoint": "wss://big.test"}
            }],
            "stackManifest": {"payload": {"selectedViews": [
                {"liveAlias": "live", "viewId": "Round/latest"}
            ]}},
            "programs": [{"installName": "big-program", "idl": {"docs": "x".repeat(padding)}}]
        })
        .to_string()
    }

    #[tokio::test]
    async fn descriptors_over_the_tool_result_cap_are_still_shaped() {
        let descriptor = oversized_stack_descriptor(MAX_RESPONSE_BYTES + 100 * 1024);
        assert!(descriptor.len() > MAX_RESPONSE_BYTES);
        let server = canned_registry(vec![("/api/registry/stacks/big/install", 200, descriptor)]);

        let summary = server
            .explore_stack(Parameters(ExploreStackArgs {
                stack: "big".into(),
                summary: None,
                views: None,
                full: None,
            }))
            .await
            .unwrap();
        let text = result_text(&summary);
        assert!(text.len() < 64 * 1024, "summary is {} bytes", text.len());
        assert!(text.contains("Round"), "{text}");

        let views = server
            .explore_stack(Parameters(ExploreStackArgs {
                stack: "big".into(),
                summary: None,
                views: Some(StringList::Many(vec!["Round/latest".into()])),
                full: None,
            }))
            .await
            .unwrap();
        assert!(result_text(&views).contains("Round/latest"));

        // read_view resolves the WebSocket URL from the same descriptor.
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        assert_eq!(
            server.stack_websocket_url("big", deadline).await.unwrap(),
            "wss://big.test"
        );

        // The whole descriptor cannot be one tool result: the caller is told
        // which arguments return less.
        let error = server
            .explore_stack(Parameters(ExploreStackArgs {
                stack: "big".into(),
                summary: None,
                views: None,
                full: Some(true),
            }))
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::INVALID_PARAMS, "{}", error.message);
        assert!(error.message.contains("`views`"), "{}", error.message);
    }

    #[tokio::test]
    async fn descriptors_over_the_read_cap_point_at_in_session_alternatives() {
        let descriptor = oversized_stack_descriptor(crate::registry::MAX_DESCRIPTOR_BYTES + 1024);
        let server = canned_registry(vec![("/api/registry/stacks/big/install", 200, descriptor)]);
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        let error = server
            .stack_websocket_url("big", deadline)
            .await
            .unwrap_err();
        assert!(error.message.contains("`url`"), "{}", error.message);
        assert!(
            error.message.contains("explore_stack_schema"),
            "{}",
            error.message
        );
    }

    fn result_text(result: &CallToolResult) -> String {
        serde_json::to_value(result).unwrap()["content"][0]["text"]
            .as_str()
            .unwrap()
            .to_string()
    }

    /// A registry `GET /api/registry/ore/schema` response.
    fn ore_schema() -> String {
        serde_json::json!({
            "name": "ore",
            "subdomain": "ore-abc123",
            "websocket_url": "wss://ore-abc123.stack.arete.run",
            "schema": {"stack_name": "OreStream", "entities": [{
                "name": "OreRound",
                "primary_keys": ["id.round_id"],
                "fields": [
                    {"path": "results.pre_reveal_winning_square", "rust_type": "Option<u8>", "nullable": true, "section": "results"},
                    {"path": "results.winning_square", "rust_type": "Option<u8>", "nullable": true, "section": "results"},
                    {"path": "results.rng", "rust_type": "Option<u64>", "nullable": true, "section": "results"}
                ],
                "views": [{"id": "OreRound/latest", "mode": "single", "pipeline": []}]
            }]}
        })
        .to_string()
    }

    #[test]
    fn stack_schema_gains_catalog_field_descriptions_only_when_served() {
        let body = ore_schema();
        let schema: serde_json::Value = serde_json::from_str(&body).unwrap();

        // No knowledge (no catalog entry, or a registry without the route):
        // the registry's bytes pass through unchanged.
        let result = described_schema_result(body.clone(), schema.clone(), None, None).unwrap();
        assert_eq!(result_text(&result), body);

        let knowledge = crate::stack_knowledge::tests::ore_knowledge().to_string();
        let descriptor = serde_json::json!({"name": "ore", "stackManifestHash": "manifest-exact"});
        let knowledge = schema_knowledge_from_bodies(&knowledge, &descriptor.to_string()).unwrap();
        let result = described_schema_result(body, schema, Some(&knowledge), None).unwrap();
        let described: serde_json::Value = serde_json::from_str(&result_text(&result)).unwrap();
        let round = &described["schema"]["entities"][0];
        assert_eq!(round["summary"], "One mining round.");
        assert_eq!(
            round["fields"][0],
            serde_json::json!({
                "path": "results.pre_reveal_winning_square",
                "rust_type": "Option<u8>",
                "nullable": true,
                "section": "results",
                "description": "The winning square before reveal; show this in a live UI."
            })
        );
        assert_eq!(
            round["fields"][1]["description"],
            "Only set once the next round opens."
        );
        assert!(round["fields"][2].get("description").is_none());
        assert_eq!(round["views"][0]["summary"], "The current round.");
        assert_eq!(described["knowledge"]["slug"], "ore-stream");
        assert_eq!(
            described["websocket_url"],
            "wss://ore-abc123.stack.arete.run"
        );
    }

    #[test]
    fn unusable_knowledge_responses_mean_no_knowledge() {
        let knowledge = crate::stack_knowledge::tests::ore_knowledge().to_string();
        assert!(stack_knowledge_from_body(&knowledge, "manifest-exact").is_some());
        assert!(
            stack_knowledge_from_body(&knowledge, "another-manifest").is_none(),
            "knowledge published for another StackManifest is not attached"
        );
        let mut unpinned = crate::stack_knowledge::tests::ore_knowledge();
        unpinned
            .as_object_mut()
            .unwrap()
            .remove("stackManifestHash");
        assert!(
            stack_knowledge_from_body(&unpinned.to_string(), "manifest-exact").is_none(),
            "knowledge that names no StackManifest cannot be checked"
        );
        assert!(stack_knowledge_from_body("<html>not json</html>", "manifest-exact").is_none());
        assert!(stack_knowledge_from_body(r#"{"error":"not found"}"#, "manifest-exact").is_none());
    }

    /// A registry address that records whether anything connected.
    fn watched_registry() -> (std::net::TcpListener, AreteMcp) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let server = AreteMcp {
            registry: RegistryClient::with_base_url(&format!(
                "http://{}",
                listener.local_addr().unwrap()
            )),
            ..AreteMcp::new()
        };
        (listener, server)
    }

    #[tokio::test]
    async fn stacks_that_cannot_have_catalog_knowledge_are_not_looked_up() {
        let (listener, server) = watched_registry();
        assert!(server
            .stack_knowledge("vault", Some("private"), "manifest-exact")
            .await
            .is_none());
        assert!(server
            .stack_knowledge("Vault Stack", Some("global"), "manifest-exact")
            .await
            .is_none());
        assert!(server
            .schema_knowledge(
                "Vault Stack",
                Some(r#"{"stackManifestHash":"manifest-exact"}"#),
                std::time::Instant::now() + LOOKUP_TIMEOUT,
            )
            .await
            .is_none());
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock,
            "no request was made"
        );
    }

    #[test]
    fn schema_guidance_is_pinned_to_the_served_stack_manifest() {
        let knowledge = crate::stack_knowledge::tests::ore_knowledge().to_string();
        let descriptor = |manifest: serde_json::Value| {
            serde_json::json!({"name": "ore", "stackManifestHash": manifest}).to_string()
        };
        // The descriptor serves the StackManifest the knowledge was
        // published for.
        assert!(
            schema_knowledge_from_bodies(&knowledge, &descriptor("manifest-exact".into()))
                .is_some()
        );
        // Another StackManifest, or one that cannot be read, gets no
        // guidance.
        for unproven in [
            descriptor("another-manifest".into()),
            descriptor(serde_json::Value::Null),
            serde_json::json!({"name": "ore"}).to_string(),
            "<html>not json</html>".to_string(),
        ] {
            assert!(
                schema_knowledge_from_bodies(&knowledge, &unproven).is_none(),
                "{unproven}"
            );
        }
    }

    #[test]
    fn shaped_results_are_compact_and_bounded() {
        let result = shaped_result(&serde_json::json!({"kind": "stack-summary"})).unwrap();
        let rendered = serde_json::to_string(&result).unwrap();
        assert!(
            rendered.contains(r#"{\"kind\":\"stack-summary\"}"#),
            "{rendered}"
        );

        let oversized = serde_json::json!({"blob": "x".repeat(MAX_RESPONSE_BYTES + 1)});
        let err = shaped_result(&oversized).unwrap_err();
        assert!(err.message.contains("byte limit"), "{}", err.message);

        // Reshaped list, search and vocabulary text is held to the same cap.
        let err = bounded_result("x".repeat(MAX_RESPONSE_BYTES + 1)).unwrap_err();
        assert!(err.message.contains("byte limit"), "{}", err.message);
        assert!(bounded_result("[]".to_string()).is_ok());
    }

    #[test]
    fn registry_failures_keep_their_classification_when_shaping() {
        let err = parse_descriptor(Err(anyhow::anyhow!("stack must not be empty"))).unwrap_err();
        assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
        let err = parse_descriptor(Err(anyhow::anyhow!("registry returned 500"))).unwrap_err();
        assert_eq!(err.code, ErrorCode::INTERNAL_ERROR);
        let err = parse_descriptor(Ok("<html>".into())).unwrap_err();
        assert_eq!(err.code, ErrorCode::INTERNAL_ERROR);
    }
}

#[cfg(test)]
mod recovery_error_tests {
    use super::{claim_url_elicitation, structured_problem_error};
    use arete_sdk::{ApiProblemV1, ReadyRecoveryAction};

    #[test]
    fn ready_claim_uses_standard_url_elicitation_with_only_handoff_metadata() {
        let error = claim_url_elicitation(&ReadyRecoveryAction {
            action_type: "claim_agent".to_string(),
            url: "https://arete.run/claim#opaque-secret".to_string(),
            elicitation_id: "0199-agent-claim".to_string(),
            expires_at: "2026-09-22T12:30:00Z".to_string(),
        });

        assert_eq!(error.code.0, -32042);
        let data = error.data.expect("URL elicitation data");
        assert_eq!(data["url"], "https://arete.run/claim#opaque-secret");
        assert_eq!(data["elicitationId"], "0199-agent-claim");
        assert_eq!(data["expiresAt"], "2026-09-22T12:30:00Z");
        assert_eq!(data["actionType"], "claim_agent");
        assert_eq!(data.as_object().expect("object").len(), 4);
    }

    #[test]
    fn structured_problem_keeps_contract_and_secondary_failure_code() {
        let problem: ApiProblemV1 = serde_json::from_value(serde_json::json!({
            "schemaVersion": 1,
            "error": "Claim this agent",
            "code": "agent-claim-required",
            "retryable": false,
            "futureField": "preserved"
        }))
        .expect("problem");

        let error =
            structured_problem_error(&problem, Some(403), Some("claim-materialization-failed"));
        let data = error.data.expect("structured problem data");
        assert_eq!(data["schemaVersion"], 1);
        assert_eq!(data["code"], "agent-claim-required");
        assert_eq!(data["retryable"], false);
        assert_eq!(data["status"], 403);
        assert_eq!(data["secondaryCode"], "claim-materialization-failed");
        assert_eq!(data["futureField"], "preserved");
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

#[cfg(test)]
mod ergonomics_tests {
    use super::*;

    #[test]
    fn get_recent_defaults_n_and_accepts_limit_alias() {
        let args: GetRecentArgs =
            serde_json::from_value(serde_json::json!({ "subscription_id": "sub_1" })).unwrap();
        assert_eq!(args.n, None);
        let args: GetRecentArgs = serde_json::from_value(serde_json::json!({
            "subscription_id": "sub_1",
            "limit": "5"
        }))
        .unwrap();
        assert_eq!(args.n, Some(5));
        let args: GetRecentArgs = serde_json::from_value(serde_json::json!({
            "subscription_id": "sub_1",
            "n": 3
        }))
        .unwrap();
        assert_eq!(args.n, Some(3));
    }

    #[test]
    fn get_recent_reports_every_problem_with_an_example() {
        let err = serde_json::from_value::<GetRecentArgs>(serde_json::json!({ "n": "many" }))
            .unwrap_err()
            .to_string();
        assert!(err.contains("missing `subscription_id`"), "{err}");
        assert!(err.contains("`n`"), "{err}");
        assert!(err.contains("Example:"), "{err}");

        let err = serde_json::from_value::<GetRecentArgs>(serde_json::json!({
            "subscription_id": 7,
            "n": 1,
            "limit": 2
        }))
        .unwrap_err()
        .to_string();
        assert!(err.contains("must be a string"), "{err}");
        assert!(err.contains("not both"), "{err}");
    }

    #[test]
    fn get_recent_schema_marks_only_subscription_id_required() {
        let schema = serde_json::to_value(schemars::schema_for!(GetRecentArgs)).unwrap();
        assert_eq!(schema["required"], serde_json::json!(["subscription_id"]));
        assert!(schema["properties"].get("n").is_some());
    }

    #[test]
    fn subscribe_response_includes_get_recent_hint() {
        let value = serde_json::to_value(subscribe_response(SubscriptionInfo {
            subscription_id: "sub_9".into(),
            connection_id: "conn_1".into(),
            view: "OreRound/latest".into(),
            key: None,
        }))
        .unwrap();
        assert_eq!(value["subscription_id"], "sub_9");
        assert_eq!(value["view"], "OreRound/latest");
        assert_eq!(value["next"]["tool"], "get_recent");
        assert_eq!(value["next"]["arguments"]["subscription_id"], "sub_9");
        assert_eq!(value["next"]["arguments"]["n"], GET_RECENT_DEFAULT);
    }

    #[test]
    fn catalog_args_accept_fields_and_full() {
        let args: SearchCatalogArgs = serde_json::from_value(serde_json::json!({
            "query": "swaps",
            "fields": "slug, delivery.status"
        }))
        .unwrap();
        assert_eq!(
            catalog_shape(args.fields, args.full),
            catalog_view::Shape::Fields(vec!["slug".into(), "delivery.status".into()])
        );
        let args: GetCatalogEntryArgs = serde_json::from_value(serde_json::json!({
            "kind": "program",
            "slug": "ore",
            "full": true
        }))
        .unwrap();
        assert_eq!(
            catalog_shape(args.fields, args.full),
            catalog_view::Shape::Full
        );
        let args: GetCatalogEntryArgs =
            serde_json::from_value(serde_json::json!({ "kind": "stack", "slug": "ore" })).unwrap();
        assert_eq!(
            catalog_shape(args.fields, args.full),
            catalog_view::Shape::Compact
        );
    }

    #[test]
    fn unfiltered_catalog_search_is_an_overview_with_a_hint() {
        let args: SearchCatalogArgs =
            serde_json::from_value(serde_json::json!({ "query": "  ", "limit": 3 })).unwrap();
        assert!(unfiltered(&args));
        let args: SearchCatalogArgs =
            serde_json::from_value(serde_json::json!({ "kind": "stack" })).unwrap();
        assert!(!unfiltered(&args));

        let overview = catalog_view::merge_overview(&[
            (
                "program",
                serde_json::json!({"results": [{"kind": "program", "slug": "a", "score": 1}], "nextCursor": "p1"}),
            ),
            (
                "stack",
                serde_json::json!({"results": [{"kind": "stack", "slug": "b"}]}),
            ),
        ]);
        let hint = overview_hint(5, true);
        let out = search_value(&overview, &catalog_view::brief(), Some(&hint));
        assert_eq!(
            out["results"][0],
            serde_json::json!({"kind": "program", "slug": "a"})
        );
        assert_eq!(out["nextCursors"]["program"], "p1");
        let hint = out["hint"].as_str().unwrap();
        assert!(hint.starts_with("Catalog overview"));
        assert!(hint.contains("nextCursors.<kind>") && hint.contains("full: true"));
        assert!(!overview_hint(5, false).contains("nextCursors"));
    }

    #[test]
    fn knowledge_search_defaults_to_brief_results_with_a_hint() {
        let args: SearchKnowledgeArgs =
            serde_json::from_value(serde_json::json!({ "query": "swaps" })).unwrap();
        let shape = search_shape(args.fields, args.full);
        let body = serde_json::json!({
            "matched_concepts": ["swap"],
            "results": [{
                "type": "program", "slug": "raydium-cp-swap", "name": "raydium_cp_swap",
                "protocol": "raydium", "summary": "AMM.", "score": 6.1,
                "coverage": {"read": true, "build": true, "subscribe": false},
                "coverage_via": {"read": ["raydium-cp-swap"]}
            }]
        })
        .to_string();
        let out: serde_json::Value =
            serde_json::from_str(&knowledge_search_body(body.clone(), &shape, 10)).unwrap();
        let first = out["results"][0].as_object().unwrap();
        assert!(!first.contains_key("score") && !first.contains_key("coverage_via"));
        assert_eq!(first["coverage"]["read"], true);
        assert_eq!(out["matched_concepts"][0], "swap");
        let hint = out["hint"].as_str().unwrap();
        assert!(hint.contains("full: true") && !hint.contains("raise `limit`"));

        let out: serde_json::Value =
            serde_json::from_str(&knowledge_search_body(body.clone(), &shape, 1)).unwrap();
        assert!(out["hint"]
            .as_str()
            .unwrap()
            .contains("raise `limit` above 1"));
        let full = search_shape(None, Some(true));
        assert_eq!(knowledge_search_body(body.clone(), &full, 10), body);
    }

    #[test]
    fn search_defaults_to_brief_results_with_a_hint() {
        let args: SearchCatalogArgs =
            serde_json::from_value(serde_json::json!({ "query": "swaps" })).unwrap();
        let shape = search_shape(args.fields, args.full);
        assert_eq!(shape, catalog_view::brief());
        let body = serde_json::json!({
            "matchedConcepts": ["swap"],
            "nextCursor": "c1",
            "results": [{
                "kind": "program", "slug": "ore", "name": "ORE", "version": "1.0.0",
                "summary": "ORE mining.", "modes": ["build"], "sdkTargets": ["typescript"],
                "score": 2.5, "concepts": ["mining"], "packageReleaseHash": "sha256:aa",
                "delivery": {"kind": "program-read", "status": "active", "health": "ready"}
            }]
        })
        .to_string();
        let out: serde_json::Value =
            serde_json::from_str(&search_body(body.clone(), &shape)).unwrap();
        assert_eq!(out["nextCursor"], "c1");
        let first = out["results"][0].as_object().unwrap();
        assert_eq!(first["slug"], "ore");
        assert!(!first.contains_key("score"));
        assert!(!first.contains_key("packageReleaseHash"));
        let hint = out["hint"].as_str().unwrap();
        assert!(hint.contains("full: true") && hint.contains("cursor"));

        let full = search_shape(None, Some(true));
        assert_eq!(search_body(body.clone(), &full), body);
        let fields = search_shape(Some(StringList::One("slug".into())), None);
        let out: serde_json::Value = serde_json::from_str(&search_body(body, &fields)).unwrap();
        assert_eq!(out["results"][0], serde_json::json!({"slug": "ore"}));
    }

    #[test]
    fn vocabularies_default_to_slugs_and_names() {
        let args: VocabularyArgs = serde_json::from_value(serde_json::json!({})).unwrap();
        let body = serde_json::json!({
            "concepts": [{"slug": "swap", "name": "Swap", "synonyms": ["trade"]}],
            "categories": [{"slug": "dex", "name": "DEX", "description": "d"}]
        })
        .to_string();
        let out: serde_json::Value =
            serde_json::from_str(&vocabulary_body(body.clone(), args.full == Some(true))).unwrap();
        assert_eq!(
            out["concepts"],
            serde_json::json!([{"slug": "swap", "name": "Swap"}])
        );
        assert!(out["hint"].as_str().unwrap().contains("full: true"));
        assert_eq!(vocabulary_body(body.clone(), true), body);
    }

    #[test]
    fn stack_lists_default_to_brief_fields() {
        let body = serde_json::json!([{
            "name": "ore", "description": "ORE.", "subdomain": "ore-1",
            "websocket_url": "wss://ore", "http_url": "https://ore",
            "websocket_auth": {"required": true, "mode": "signed_session"},
            "http_auth": {"required": true},
            "entities": ["OreRound"], "visibility": "public", "serviceClass": "standard"
        }])
        .to_string();
        let args: ExploreStacksArgs = serde_json::from_value(serde_json::json!({})).unwrap();
        let out: serde_json::Value = serde_json::from_str(&list_body(
            body.clone(),
            &stack_list_shape(args.fields, args.full),
        ))
        .unwrap();
        assert_eq!(
            out,
            serde_json::json!([{
                "name": "ore", "description": "ORE.", "websocket_url": "wss://ore",
                "entities": ["OreRound"], "visibility": "public", "serviceClass": "standard",
                "websocket_auth": {"required": true}
            }])
        );
        assert_eq!(
            list_body(body.clone(), &stack_list_shape(None, Some(true))),
            body
        );
    }

    #[test]
    fn program_lists_drop_hashes_unless_full() {
        let body = serde_json::json!([{
            "installName": "spl-token", "programId": "Tok", "sdkTargets": ["rust"],
            "programReleaseHash": "arete:h1:program-release:sha256:aa",
            "programSpecHash": "arete:h1:program-spec:sha256:bb"
        }])
        .to_string();
        let args: ExploreProgramsArgs = serde_json::from_value(serde_json::json!({})).unwrap();
        let out: serde_json::Value = serde_json::from_str(&list_body(
            body.clone(),
            &catalog_shape(args.fields, args.full),
        ))
        .unwrap();
        assert_eq!(
            out,
            serde_json::json!([{"installName": "spl-token", "programId": "Tok", "sdkTargets": ["rust"]}])
        );
        assert_eq!(list_body(body.clone(), &catalog_view::Shape::Full), body);
    }
}
