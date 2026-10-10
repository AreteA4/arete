//! Client-side shaping of catalog and knowledge responses.
//!
//! Catalog entries carry identity hashes, program id lists and release notes
//! that an agent rarely needs while choosing what to install. These helpers
//! trim a response to a compact form or to an explicit field list. They work
//! on the decoded JSON, so they apply to any server version: unknown keys are
//! passed through by [`compact`] and silently skipped by [`project`].
//!
//! Shared by the MCP tools (`search_catalog`, `get_catalog_entry`,
//! `list_catalog_vocabulary`, `list_concepts`, `explore_programs`) and the
//! CLI (`a4 explore catalog`, `a4 explore programs`, `a4 know search`).
//!
//! List, search and vocabulary responses are brief by default; callers opt
//! out with `full` / `--full` or choose keys with `fields` / `--fields`, and a
//! top-level `hint` string says how.
//!
//! Shaped (non-`full`) output also gains two derived keys, computed only from
//! fields the server already sends:
//!
//! - `live` on stack entries: whether the stack has a hosted stream to
//!   subscribe to now (see [`is_live_stack`]). A stack with `live: false` is
//!   definition-only.
//! - `related` on search results: the other results of the same page that
//!   share the entry's `protocol`, as `kind:slug` (e.g. the program a stack
//!   streams). Program, stack and protocol slugs often differ for the same
//!   protocol, so this saves a lookup per result.

use serde_json::{Map, Value};

/// Fields kept by the CLI `--brief` preset. Paths missing from an entry are
/// skipped, so the same list serves catalog entries (`kind`, `modes`,
/// `delivery`) and knowledge results (`type`, `coverage`).
pub const BRIEF_FIELDS: &[&str] = &[
    "kind",
    "type",
    "slug",
    "name",
    "version",
    "protocol",
    "summary",
    "modes",
    "live",
    "related",
    "sdkTargets",
    "coverage",
    "delivery.status",
    "delivery.health",
];

/// The catalog access mode a hosted stream is published under.
const SUBSCRIBE_MODE: &str = "subscribe";

/// The delivery kind of a stack whose stream is hosted.
const DEPLOYED_STACK_DELIVERY: &str = "deployed-stack";

/// What `live` and `related` mean, for hints and help text.
pub const LIVE_HINT: &str = "`live`: true = a hosted stream to subscribe to now; false = \
     definition-only (install its SDK to build and read, or deploy it to stream). `related`: \
     other results for the same protocol, as `kind:slug`.";

/// Page size used for catalog searches when the caller passes no limit. The
/// server's own default is larger; a smaller first page keeps discovery
/// output short, and `nextCursor` continues it.
pub const DEFAULT_SEARCH_LIMIT: usize = 10;

/// Entries per kind in the overview a catalog search without filters
/// returns (the server itself requires a filter).
pub const OVERVIEW_LIMIT: usize = 5;

/// Kinds listed by the unfiltered catalog overview, in order.
pub const OVERVIEW_KINDS: [&str; 2] = ["program", "stack"];

/// Merge one search page per kind into an overview: the pages' results in
/// order, and `nextCursors` keyed by kind for the kinds that have another
/// page (omitted when none do).
pub fn merge_overview(pages: &[(&str, Value)]) -> Value {
    let mut results = Vec::new();
    let mut cursors = Map::new();
    for (kind, page) in pages {
        if let Some(Value::Array(items)) = page.get("results") {
            results.extend(items.iter().cloned());
        }
        if let Some(cursor) = next_cursor(page) {
            cursors.insert(kind.to_string(), Value::String(cursor.to_string()));
        }
    }
    let mut out = Map::new();
    out.insert("results".to_string(), Value::Array(results));
    if !cursors.is_empty() {
        out.insert("nextCursors".to_string(), Value::Object(cursors));
    }
    Value::Object(out)
}

/// Whether a catalog entry is a stack with a hosted stream. `None` for
/// anything that is not a stack (programs have no stream).
///
/// A stack is live when its evidenced `modes` include `subscribe`, the mode
/// `--mode subscribe` / `mode: "subscribe"` filters on. An entry that carries
/// no `modes` falls back to its delivery: `delivery.kind` `deployed-stack`
/// is a hosted deployment. A stack with neither is definition-only.
pub fn is_live_stack(entry: &Value) -> Option<bool> {
    if entry.get("kind").and_then(Value::as_str) != Some("stack") {
        return None;
    }
    if let Some(modes) = entry.get("modes").and_then(Value::as_array) {
        return Some(
            modes
                .iter()
                .any(|mode| mode.as_str() == Some(SUBSCRIBE_MODE)),
        );
    }
    Some(
        entry
            .pointer("/delivery/kind")
            .and_then(Value::as_str)
            .is_some_and(|kind| kind == DEPLOYED_STACK_DELIVERY),
    )
}

/// Add the derived `live` key to a stack entry (see [`is_live_stack`]). A key
/// the server already sends is kept.
pub fn annotate_entry(entry: &Value) -> Value {
    let mut out = entry.clone();
    if let (Some(live), Value::Object(map)) = (is_live_stack(entry), &mut out) {
        map.entry("live").or_insert(Value::Bool(live));
    }
    out
}

/// Whether a search page (or a bare result list) holds a stack entry, so
/// its hint should say what `live` means.
pub fn has_stacks(value: &Value) -> bool {
    let results = match value {
        Value::Array(items) => items.as_slice(),
        _ => value
            .get("results")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[]),
    };
    results.iter().any(|entry| is_live_stack(entry).is_some())
}

/// `kind:slug` (or `type:slug` for knowledge results) of one result.
fn result_ref(entry: &Value) -> Option<String> {
    let kind = entry
        .get("kind")
        .or_else(|| entry.get("type"))
        .and_then(Value::as_str)?;
    let slug = entry.get("slug").and_then(Value::as_str)?;
    Some(format!("{kind}:{slug}"))
}

/// Annotate every result of a page with `live` and, where other results of
/// the page share its `protocol`, `related` (their `kind:slug`, in page
/// order). Linking uses only the page: a related package outside it is not
/// looked up.
pub fn annotate_results(results: &[Value]) -> Vec<Value> {
    let refs: Vec<(Option<&str>, Option<String>)> = results
        .iter()
        .map(|entry| {
            (
                entry
                    .get("protocol")
                    .and_then(Value::as_str)
                    .filter(|protocol| !protocol.is_empty()),
                result_ref(entry),
            )
        })
        .collect();
    results
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let mut out = annotate_entry(entry);
            let (protocol, own) = &refs[index];
            let Some(protocol) = protocol else {
                return out;
            };
            let mut related: Vec<Value> = Vec::new();
            for (other_index, (other_protocol, other_ref)) in refs.iter().enumerate() {
                let Some(other_ref) = other_ref else { continue };
                if other_index == index || other_protocol != &Some(*protocol) {
                    continue;
                }
                if own.as_ref() == Some(other_ref)
                    || related.iter().any(|seen| seen.as_str() == Some(other_ref))
                {
                    continue;
                }
                related.push(Value::String(other_ref.clone()));
            }
            if let (false, Value::Object(map)) = (related.is_empty(), &mut out) {
                map.entry("related").or_insert(Value::Array(related));
            }
            out
        })
        .collect()
}

/// The problem code the registry answers a stack install with when the stack
/// is a catalog package with no hosted deployment.
pub const DEFINITION_ONLY_CODE: &str = "stack-definition-only";

/// The catalog facts a definition-only answer repeats: slug, version,
/// protocol, summary and modes. A single catalog entry carries its summary
/// and protocols under `knowledge`, a search result at the top level.
fn entry_brief(entry: &Value) -> Value {
    let knowledge = entry.get("knowledge").unwrap_or(&Value::Null);
    let text = |value: Option<&Value>| value.and_then(Value::as_str).map(str::to_string);
    let mut out = Map::new();
    for key in ["slug", "version"] {
        if let Some(value) = text(entry.get(key)) {
            out.insert(key.into(), Value::String(value));
        }
    }
    let protocol = text(entry.get("protocol"))
        .or_else(|| text(knowledge.get("protocol")))
        .or_else(|| text(knowledge.pointer("/protocols/0")));
    if let Some(protocol) = protocol {
        out.insert("protocol".into(), Value::String(protocol));
    }
    if let Some(summary) = text(entry.get("summary")).or_else(|| text(knowledge.get("summary"))) {
        out.insert("summary".into(), Value::String(summary));
    }
    if let Some(modes) = entry.get("modes") {
        out.insert("modes".into(), modes.clone());
    }
    Value::Object(out)
}

/// The structured answer to exploring a stack that has no hosted stream,
/// instead of an error: the registry refused its install descriptor with
/// [`DEFINITION_ONLY_CODE`] (`registry_says`), or the catalog lists a stack
/// under `reference` that is not live (`catalog_entry`, the stack's catalog
/// entry when it could be read). `None` when neither holds, so the original
/// error stands.
///
/// `detail` is the registry's message; `find_live` says how to search for a
/// live stack in the caller's interface (CLI flags or MCP arguments).
pub fn definition_only_stack(
    reference: &str,
    registry_says: bool,
    detail: Option<&str>,
    catalog_entry: Option<&Value>,
    find_live: &str,
) -> Option<Value> {
    let catalog_live = catalog_entry.and_then(is_live_stack);
    if !registry_says && catalog_live != Some(false) {
        return None;
    }
    let slug = catalog_entry
        .and_then(|entry| entry.get("slug"))
        .and_then(Value::as_str)
        .unwrap_or(reference);
    let mut out = Map::new();
    out.insert("kind".into(), Value::String("stack-definition-only".into()));
    out.insert("stack".into(), Value::String(slug.to_string()));
    out.insert("live".into(), Value::Bool(false));
    if let Some(detail) = detail.filter(|detail| !detail.is_empty()) {
        out.insert("detail".into(), Value::String(detail.to_string()));
    }
    if let Some(entry) = catalog_entry {
        out.insert("catalog".into(), entry_brief(entry));
    }
    out.insert(
        "next".into(),
        serde_json::json!([
            format!(
                "`a4 install stack {slug} --ts` installs its typed SDK and program SDKs: build \
                 transactions and read accounts now; its views have no endpoint until it is deployed"
            ),
            "`a4 up <alias>` (the dependency alias `a4 install` printed) deploys it yourself so \
             its views stream"
                .to_string(),
            find_live.to_string(),
        ]),
    );
    Some(Value::Object(out))
}

/// How a catalog response should be shaped before it is returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shape {
    /// The server response, untouched.
    Full,
    /// Drop identity hashes, release notes, program id lists and empty
    /// deprecation markers.
    Compact,
    /// Keep only these top-level keys or dotted paths (e.g. `delivery.status`).
    Fields(Vec<String>),
}

impl Shape {
    /// Resolve the MCP arguments: an explicit field list wins, then
    /// `full: true`, otherwise compact.
    pub fn from_args(fields: Vec<String>, full: bool) -> Self {
        if !fields.is_empty() {
            Shape::Fields(fields)
        } else if full {
            Shape::Full
        } else {
            Shape::Compact
        }
    }
}

/// Split comma-separated field lists into trimmed, de-duplicated paths.
pub fn parse_fields<S: AsRef<str>>(values: &[S]) -> Vec<String> {
    crate::descriptor::split_list(values)
}

/// The brief preset as owned paths.
pub fn brief_fields() -> Vec<String> {
    BRIEF_FIELDS.iter().map(|field| field.to_string()).collect()
}

/// Whether a key is dropped from compact output.
fn is_bulky_key(key: &str, value: &Value) -> bool {
    key.ends_with("Hash")
        || key.ends_with("Hashes")
        || key == "releaseNotes"
        || key == "programIds"
        || (key == "deprecation" && value.is_null())
}

/// Recursively drop bulky keys from objects (including objects inside arrays).
pub fn compact(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .filter(|(key, value)| !is_bulky_key(key, value))
                .map(|(key, value)| (key.clone(), compact(value)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(compact).collect()),
        other => other.clone(),
    }
}

/// Keep only `fields` from one entry. Dotted paths keep the nested value
/// under the same nesting (`delivery.status` -> `{"delivery":{"status":..}}`).
/// Non-object entries are returned unchanged.
pub fn project(value: &Value, fields: &[String]) -> Value {
    let Value::Object(source) = value else {
        return value.clone();
    };
    let mut out = Map::new();
    for field in fields {
        let segments: Vec<&str> = field.split('.').filter(|s| !s.is_empty()).collect();
        if segments.is_empty() {
            continue;
        }
        let mut current = source;
        let mut found = None;
        for (index, segment) in segments.iter().enumerate() {
            match current.get(*segment) {
                Some(Value::Object(next)) if index + 1 < segments.len() => current = next,
                Some(leaf) if index + 1 == segments.len() => found = Some(leaf.clone()),
                _ => break,
            }
        }
        if let Some(leaf) = found {
            insert_path(&mut out, &segments, leaf);
        }
    }
    Value::Object(out)
}

fn insert_path(out: &mut Map<String, Value>, segments: &[&str], leaf: Value) {
    let (last, parents) = segments.split_last().expect("non-empty path");
    let mut target = out;
    for segment in parents {
        let slot = target
            .entry(segment.to_string())
            .or_insert_with(|| Value::Object(Map::new()));
        if !slot.is_object() {
            *slot = Value::Object(Map::new());
        }
        target = slot.as_object_mut().expect("object slot");
    }
    target.insert(last.to_string(), leaf);
}

fn shape_one(value: &Value, shape: &Shape) -> Value {
    match shape {
        Shape::Full => value.clone(),
        Shape::Compact => compact(value),
        Shape::Fields(fields) => project(value, fields),
    }
}

/// Shape a search response: each item of its `results` array is shaped and
/// the envelope (`matchedConcepts`, `nextCursor`, ...) is kept. A bare array
/// is treated as the result list.
pub fn shape_search(value: &Value, shape: &Shape) -> Value {
    if *shape == Shape::Full {
        return value.clone();
    }
    match value {
        Value::Object(map) => {
            let mut out = map.clone();
            if let Some(Value::Array(results)) = map.get("results") {
                out.insert(
                    "results".to_string(),
                    Value::Array(
                        annotate_results(results)
                            .iter()
                            .map(|item| shape_one(item, shape))
                            .collect(),
                    ),
                );
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(
            annotate_results(items)
                .iter()
                .map(|item| shape_one(item, shape))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Shape a single catalog entry response. Shaped (non-`full`) stack entries
/// gain `live`.
pub fn shape_entry(value: &Value, shape: &Shape) -> Value {
    match shape {
        Shape::Full => value.clone(),
        _ => shape_one(&annotate_entry(value), shape),
    }
}

/// The brief preset as a [`Shape`].
pub fn brief() -> Shape {
    Shape::Fields(brief_fields())
}

/// Attach a top-level `hint` string to an object response. Other values are
/// returned unchanged. Readers that ignore unknown keys are unaffected.
pub fn with_hint(value: Value, hint: impl Into<String>) -> Value {
    match value {
        Value::Object(mut map) => {
            map.insert("hint".to_string(), Value::String(hint.into()));
            Value::Object(map)
        }
        other => other,
    }
}

/// The `nextCursor` of a search page, when there is another page.
pub fn next_cursor(value: &Value) -> Option<&str> {
    value
        .get("nextCursor")
        .and_then(Value::as_str)
        .filter(|cursor| !cursor.is_empty())
}

/// Whether a search page that has no cursor (knowledge search) may have been
/// cut at `limit`: it returned at least that many results.
pub fn may_have_more(value: &Value, limit: usize) -> bool {
    value
        .get("results")
        .and_then(Value::as_array)
        .is_some_and(|results| results.len() >= limit)
}

/// Keep only `slug` and `name` of each concept and category of a vocabulary
/// response; descriptions, synonyms, related slugs and snapshot hashes are
/// dropped. Lists other than `concepts` and `categories` are dropped too.
pub fn compact_vocabulary(value: &Value) -> Value {
    let Value::Object(source) = value else {
        return value.clone();
    };
    let keep = ["slug".to_string(), "name".to_string()];
    let mut out = Map::new();
    for key in ["concepts", "categories"] {
        if let Some(Value::Array(items)) = source.get(key) {
            out.insert(
                key.to_string(),
                Value::Array(items.iter().map(|item| project(item, &keep)).collect()),
            );
        }
    }
    Value::Object(out)
}

/// Shape a raw JSON body. Bodies that are not JSON are returned unchanged.
pub fn shape_body(body: String, shape: &Shape, search: bool) -> String {
    if *shape == Shape::Full {
        return body;
    }
    match serde_json::from_str::<Value>(&body) {
        Ok(value) => {
            let shaped = if search {
                shape_search(&value, shape)
            } else {
                shape_entry(&value, shape)
            };
            serde_json::to_string(&shaped).unwrap_or(body)
        }
        Err(_) => body,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entry() -> Value {
        json!({
            "kind": "program",
            "slug": "ore",
            "name": "ORE",
            "version": "1.0.0",
            "summary": "ORE mining.",
            "modes": ["build", "read"],
            "packageReleaseHash": "sha256:aa",
            "bundleHash": "sha256:bb",
            "setHash": "sha256:cc",
            "programIds": ["oreV3"],
            "releaseNotes": "long text",
            "deprecation": null,
            "delivery": {"kind": "program-read", "status": "active", "health": "ready", "identityHash": "x"},
            "capabilities": [{"mode": "build", "operationId": "op", "specHash": "y"}]
        })
    }

    #[test]
    fn compact_drops_hashes_release_notes_and_null_deprecation() {
        let out = compact(&entry());
        for key in [
            "packageReleaseHash",
            "bundleHash",
            "setHash",
            "programIds",
            "releaseNotes",
            "deprecation",
        ] {
            assert!(out.get(key).is_none(), "{key} should be dropped");
        }
        assert_eq!(out["slug"], "ore");
        assert_eq!(out["delivery"]["status"], "active");
        assert!(out["delivery"].get("identityHash").is_none());
        assert!(out["capabilities"][0].get("specHash").is_none());
        assert_eq!(out["capabilities"][0]["operationId"], "op");
    }

    #[test]
    fn compact_keeps_an_actual_deprecation() {
        let mut value = entry();
        value["deprecation"] = json!({"message": "use ore-v2"});
        assert_eq!(compact(&value)["deprecation"]["message"], "use ore-v2");
    }

    #[test]
    fn project_keeps_top_level_and_dotted_paths_and_skips_missing() {
        let fields = parse_fields(&["slug, name", "delivery.status,missing,modes.0"]);
        let out = project(&entry(), &fields);
        assert_eq!(
            out,
            json!({"slug": "ore", "name": "ORE", "delivery": {"status": "active"}})
        );
    }

    #[test]
    fn search_shaping_keeps_envelope() {
        let body = json!({
            "matchedConcepts": ["swap"],
            "nextCursor": "c1",
            "results": [entry()]
        });
        let out = shape_search(&body, &Shape::Fields(brief_fields()));
        assert_eq!(out["nextCursor"], "c1");
        assert_eq!(out["matchedConcepts"][0], "swap");
        let first = out["results"][0].as_object().unwrap();
        assert_eq!(first["kind"], "program");
        assert_eq!(
            first["delivery"],
            json!({"status": "active", "health": "ready"})
        );
        assert!(!first.contains_key("packageReleaseHash"));
        assert!(!first.contains_key("capabilities"));

        assert_eq!(shape_search(&body, &Shape::Full), body);
    }

    #[test]
    fn shape_from_args_prefers_fields_then_full() {
        assert_eq!(Shape::from_args(vec![], false), Shape::Compact);
        assert_eq!(Shape::from_args(vec![], true), Shape::Full);
        assert_eq!(
            Shape::from_args(vec!["slug".into()], true),
            Shape::Fields(vec!["slug".into()])
        );
    }

    #[test]
    fn vocabulary_compacts_to_slugs_and_names() {
        let vocabulary = json!({
            "concepts": [{"slug": "swap", "name": "Swap", "description": "d", "synonyms": ["trade"], "related": ["dex"]}],
            "categories": [{"slug": "dex", "name": "DEX", "description": "d"}],
            "sets": ["arete:h1:catalog-publication-set:sha256:aa"]
        });
        assert_eq!(
            compact_vocabulary(&vocabulary),
            json!({
                "concepts": [{"slug": "swap", "name": "Swap"}],
                "categories": [{"slug": "dex", "name": "DEX"}]
            })
        );
    }

    #[test]
    fn overview_merges_pages_and_keys_cursors_by_kind() {
        let programs = json!({"results": [{"slug": "a"}], "nextCursor": "p1", "sets": []});
        let stacks = json!({"results": [{"slug": "b"}]});
        assert_eq!(
            merge_overview(&[("program", programs), ("stack", stacks.clone())]),
            json!({"results": [{"slug": "a"}, {"slug": "b"}], "nextCursors": {"program": "p1"}})
        );
        assert_eq!(
            merge_overview(&[("stack", stacks)]),
            json!({"results": [{"slug": "b"}]})
        );
    }

    #[test]
    fn full_pages_may_have_more() {
        let page = json!({"results": [{}, {}]});
        assert!(may_have_more(&page, 2));
        assert!(!may_have_more(&page, 3));
        assert!(!may_have_more(&json!({}), 1));
    }

    #[test]
    fn hints_attach_to_objects_and_cursors_are_read() {
        let page = json!({"results": [], "nextCursor": "c1"});
        assert_eq!(next_cursor(&page), Some("c1"));
        assert_eq!(next_cursor(&json!({"nextCursor": ""})), None);
        assert_eq!(with_hint(page, "more")["hint"], "more");
        assert_eq!(with_hint(json!([1]), "more"), json!([1]));
    }

    #[test]
    fn shape_body_passes_non_json_through() {
        assert_eq!(
            shape_body("not json".to_string(), &Shape::Compact, true),
            "not json"
        );
        let shaped = shape_body(entry().to_string(), &Shape::Compact, false);
        assert!(!shaped.contains("bundleHash"));
    }

    fn launchpad_page() -> Value {
        json!({
            "results": [
                {"kind": "program", "slug": "jurassic-fi-token-sale", "protocol": "jurassic",
                 "modes": ["build", "read"],
                 "delivery": {"kind": "program-read", "status": "active", "health": "ready"}},
                {"kind": "stack", "slug": "jurassic-launchpad", "protocol": "jurassic",
                 "modes": ["build", "read", "subscribe"],
                 "delivery": {"kind": "deployed-stack", "status": "active", "health": "ready"}},
                {"kind": "stack", "slug": "pumpfun", "protocol": "pump-fun", "modes": ["build", "read"]},
                {"kind": "program", "slug": "pumpfun", "protocol": "pump-fun", "modes": ["build", "read"]},
                {"kind": "stack", "slug": "solo", "protocol": "solo", "modes": ["build", "read"]}
            ]
        })
    }

    #[test]
    fn stacks_are_live_only_with_a_hosted_stream() {
        let page = launchpad_page();
        let results = page["results"].as_array().unwrap();
        assert_eq!(is_live_stack(&results[0]), None, "programs have no stream");
        assert_eq!(is_live_stack(&results[1]), Some(true));
        assert_eq!(is_live_stack(&results[2]), Some(false));
        // Without `modes`, a hosted delivery decides.
        assert_eq!(
            is_live_stack(&json!({"kind": "stack", "delivery": {"kind": "deployed-stack"}})),
            Some(true)
        );
        assert_eq!(is_live_stack(&json!({"kind": "stack"})), Some(false));
        // `modes` wins over delivery when both are present.
        assert_eq!(
            is_live_stack(
                &json!({"kind": "stack", "modes": ["read"], "delivery": {"kind": "deployed-stack"}})
            ),
            Some(false)
        );
    }

    #[test]
    fn brief_search_marks_live_stacks_and_links_results_by_protocol() {
        let out = shape_search(&launchpad_page(), &brief());
        let results = out["results"].as_array().unwrap();
        assert!(results[0].get("live").is_none());
        assert_eq!(results[0]["related"], json!(["stack:jurassic-launchpad"]));
        assert_eq!(results[1]["live"], true);
        assert_eq!(
            results[1]["related"],
            json!(["program:jurassic-fi-token-sale"])
        );
        assert_eq!(results[2]["live"], false);
        assert_eq!(results[2]["related"], json!(["program:pumpfun"]));
        assert_eq!(results[3]["related"], json!(["stack:pumpfun"]));
        assert_eq!(results[4]["live"], false);
        assert!(
            results[4].get("related").is_none(),
            "nothing shares its protocol"
        );

        // An explicit field list keeps only what it names; `full` is untouched.
        let fields = shape_search(&launchpad_page(), &Shape::Fields(vec!["slug".into()]));
        assert_eq!(fields["results"][1], json!({"slug": "jurassic-launchpad"}));
        let live = shape_search(
            &launchpad_page(),
            &Shape::Fields(vec!["slug".into(), "live".into()]),
        );
        assert_eq!(
            live["results"][1],
            json!({"slug": "jurassic-launchpad", "live": true})
        );
        assert_eq!(
            shape_search(&launchpad_page(), &Shape::Full),
            launchpad_page()
        );
    }

    #[test]
    fn shaped_entries_gain_live_but_full_entries_do_not() {
        let stack = launchpad_page()["results"][2].clone();
        assert_eq!(shape_entry(&stack, &Shape::Compact)["live"], false);
        assert!(shape_entry(&stack, &Shape::Full).get("live").is_none());
        let program = launchpad_page()["results"][0].clone();
        assert!(shape_entry(&program, &Shape::Compact).get("live").is_none());
    }

    #[test]
    fn definition_only_answers_come_from_the_registry_code_or_the_catalog() {
        let entry = json!({"kind": "stack", "slug": "pumpfun", "version": "1.1.4",
            "protocol": "pump-fun", "summary": "Pump.", "modes": ["build", "read"],
            "packageReleaseHash": "sha256:aa"});
        let answer = definition_only_stack("pumpfun", false, None, Some(&entry), "find").unwrap();
        assert_eq!(answer["kind"], "stack-definition-only");
        assert_eq!(answer["live"], false);
        assert_eq!(answer["catalog"]["protocol"], "pump-fun");
        assert!(answer["catalog"].get("packageReleaseHash").is_none());
        assert!(answer["next"][0]
            .as_str()
            .unwrap()
            .contains("a4 install stack pumpfun --ts"));
        assert_eq!(answer["next"][2], "find");

        let registry =
            definition_only_stack("pumpfun", true, Some("no hosted stream"), None, "f").unwrap();
        assert_eq!(registry["detail"], "no hosted stream");
        assert!(registry.get("catalog").is_none());

        // A live catalog stack, a program, or nothing at all keeps the error.
        // A single entry: summary and protocols under `knowledge`, no modes.
        let single = json!({"kind": "stack", "slug": "pumpfun", "version": "1.1.4",
            "knowledge": {"summary": "Pump.", "protocols": ["pump-fun"]}});
        let answer = definition_only_stack("pumpfun", false, None, Some(&single), "f").unwrap();
        assert_eq!(
            answer["catalog"],
            json!({"slug": "pumpfun", "version": "1.1.4", "protocol": "pump-fun", "summary": "Pump."})
        );

        let mut live = entry.clone();
        live["modes"] = json!(["subscribe"]);
        assert!(definition_only_stack("pumpfun", false, None, Some(&live), "f").is_none());
        let program = json!({"kind": "program", "slug": "pumpfun"});
        assert!(definition_only_stack("pumpfun", false, None, Some(&program), "f").is_none());
        assert!(definition_only_stack("pumpfun", false, None, None, "f").is_none());
    }
}
