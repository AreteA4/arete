//! Client-side shaping of catalog and knowledge responses.
//!
//! Catalog entries carry identity hashes, program id lists and release notes
//! that an agent rarely needs while choosing what to install. These helpers
//! trim a response to a compact form or to an explicit field list. They work
//! on the decoded JSON, so they apply to any server version: unknown keys are
//! passed through by [`compact`] and silently skipped by [`project`].
//!
//! Shared by the MCP tools (`search_catalog`, `get_catalog_entry`) and the
//! CLI (`a4 explore catalog --fields/--brief`, `a4 know search`).

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
    "sdkTargets",
    "coverage",
    "delivery.status",
    "delivery.health",
];

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
                    Value::Array(results.iter().map(|item| shape_one(item, shape)).collect()),
                );
            }
            Value::Object(out)
        }
        Value::Array(items) => {
            Value::Array(items.iter().map(|item| shape_one(item, shape)).collect())
        }
        other => other.clone(),
    }
}

/// Shape a single catalog entry response.
pub fn shape_entry(value: &Value, shape: &Shape) -> Value {
    shape_one(value, shape)
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
    fn shape_body_passes_non_json_through() {
        assert_eq!(
            shape_body("not json".to_string(), &Shape::Compact, true),
            "not json"
        );
        let shaped = shape_body(entry().to_string(), &Shape::Compact, false);
        assert!(!shaped.contains("bundleHash"));
    }
}
