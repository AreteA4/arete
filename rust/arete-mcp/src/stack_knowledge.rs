//! Curated field descriptions from a stack's catalog knowledge.
//!
//! A stack published to the catalog carries a knowledge document: a summary
//! for each entity and view, and descriptions keyed by field path, such as
//! which of two similar fields a live UI should show. The registry serves it
//! at `GET /api/registry/v1/catalog/entries/stack/<slug>/knowledge`; the
//! explore surfaces attach it to the entities, views and fields they report.
//!
//! The document is optional context. A stack with no catalog entry (private,
//! composed or local), a registry that does not serve the route, or any
//! transport failure means there is simply nothing to attach, never an error.
//! It is attached only to the exact StackManifest it was published for (see
//! [`StackKnowledge::belongs_to`]), so guidance for one version of a stack
//! never describes another.
//!
//! Knowledge paths are camelCase (`results.preRevealWinningSquare`) while a
//! schema may report `results.pre_reveal_winning_square` or the camelCase
//! form, so paths are compared through [`normalize_path`].

use std::time::Duration;

use serde_json::{json, Map, Value};

use crate::descriptor::EntityField;

/// How long the optional knowledge lookup may take in total, connecting
/// included, before explore prints what it already has without it. For a
/// schema this also bounds reading the StackManifest the knowledge is
/// checked against.
pub const LOOKUP_TIMEOUT: Duration = Duration::from_secs(2);

/// Whether a stack could have catalog knowledge, so that looking it up is
/// worth a request. Knowledge is published for public and global catalog
/// stacks, under their package slug, so a private stack, or a name that is
/// not a package slug (a display name, or anything that is not one path
/// segment), is skipped without a request.
pub fn may_have_catalog_knowledge(slug: &str, visibility: Option<&str>) -> bool {
    let bytes = slug.as_bytes();
    let package_slug = !bytes.is_empty()
        && bytes.len() <= 128
        && bytes[0].is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'));
    package_slug && visibility.is_none_or(|visibility| !visibility.eq_ignore_ascii_case("private"))
}

/// One path segment, compared case-insensitively with `_` removed.
fn normalize_segment(segment: &str) -> String {
    segment
        .chars()
        .filter(|character| *character != '_')
        .flat_map(char::to_lowercase)
        .collect()
}

/// A field path in the form used to join knowledge to a schema: split on
/// `.`, each segment lowercased with `_` removed, so
/// `results.preRevealWinningSquare` and `results.pre_reveal_winning_square`
/// normalize to the same string.
pub fn normalize_path(path: &str) -> String {
    path.trim()
        .split('.')
        .map(normalize_segment)
        .collect::<Vec<_>>()
        .join(".")
}

/// Whether two names or paths refer to the same thing after normalization.
pub fn same_path(left: &str, right: &str) -> bool {
    normalize_path(left) == normalize_path(right)
}

fn text(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

/// The knowledge of one entity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityKnowledge {
    pub name: String,
    pub summary: Option<String>,
    /// `(view name, summary)`, e.g. `("latest", "The current round.")`.
    views: Vec<(String, String)>,
    /// `(knowledge path, description)`.
    fields: Vec<(String, String)>,
}

impl EntityKnowledge {
    fn parse(name: &str, value: &Value) -> Self {
        let pairs = |key: &str, read: fn(&Value) -> Option<String>| {
            value
                .get(key)
                .and_then(Value::as_object)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|(name, item)| Some((name.clone(), read(item)?)))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        };
        Self {
            name: name.to_string(),
            summary: text(value.get("summary")),
            views: pairs("views", |view| text(view.get("summary"))),
            fields: pairs("fields", |description| text(Some(description))),
        }
    }

    /// The curated description of the field at `path` (either casing).
    pub fn field_description(&self, path: &str) -> Option<&str> {
        let wanted = normalize_path(path);
        self.fields
            .iter()
            .find(|(candidate, _)| normalize_path(candidate) == wanted)
            .map(|(_, description)| description.as_str())
    }

    /// The summary of a view, by its name (`latest`) or its id
    /// (`OreRound/latest`).
    pub fn view_summary(&self, view: &str) -> Option<&str> {
        let name = view.rsplit('/').next().unwrap_or(view);
        self.views
            .iter()
            .find(|(candidate, _)| same_path(candidate, name))
            .map(|(_, summary)| summary.as_str())
    }

    /// Set `description` on every field the knowledge describes.
    pub fn describe_fields(&self, fields: &mut [EntityField]) {
        for field in fields {
            if let Some(description) = self.field_description(&field.path) {
                field.description = Some(description.to_string());
            }
        }
    }

    /// `[{path, description}]` for the described fields among `paths`, in
    /// their order, reported with the schema's own path.
    pub fn field_descriptions<'a>(&self, paths: impl IntoIterator<Item = &'a str>) -> Vec<Value> {
        paths
            .into_iter()
            .filter_map(|path| {
                let description = self.field_description(path)?;
                Some(json!({ "path": path, "description": description }))
            })
            .collect()
    }
}

/// A stack's published knowledge document, read leniently.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackKnowledge {
    /// The knowledge document's slug, which may differ from the package's.
    pub slug: String,
    pub document_hash: String,
    /// The StackManifest the catalog entry publishes, when the registry
    /// reports it.
    pub stack_manifest_hash: Option<String>,
    entities: Vec<EntityKnowledge>,
}

impl StackKnowledge {
    /// Read the registry's catalog entry knowledge response. `None` when it
    /// is not a stack document or has no identity.
    pub fn from_response(response: &Value) -> Option<Self> {
        if response.get("kind").and_then(Value::as_str) != Some("stack") {
            return None;
        }
        let document_hash = text(response.get("documentHash"))?;
        let entities = response
            .get("entities")
            .and_then(Value::as_object)
            .map(|entities| {
                entities
                    .iter()
                    .filter(|(_, entity)| entity.is_object())
                    .map(|(name, entity)| EntityKnowledge::parse(name, entity))
                    .collect()
            })
            .unwrap_or_default();
        Some(Self {
            slug: text(response.get("slug")).unwrap_or_default(),
            document_hash,
            stack_manifest_hash: text(response.get("stackManifestHash")),
            entities,
        })
    }

    /// Whether this knowledge was published for the stack whose
    /// StackManifest is `stack_manifest_hash`. It must say so: knowledge that
    /// names no StackManifest, or another one, describes a stack whose
    /// fields this one may not share.
    pub fn belongs_to(&self, stack_manifest_hash: &str) -> bool {
        !stack_manifest_hash.is_empty()
            && self.stack_manifest_hash.as_deref() == Some(stack_manifest_hash)
    }

    /// The knowledge of the entity named `name` (either casing).
    pub fn entity(&self, name: &str) -> Option<&EntityKnowledge> {
        self.entities
            .iter()
            .find(|entity| same_path(&entity.name, name))
    }

    /// Where the descriptions come from, with keys in `key_case`.
    pub fn source(&self, key_case: KeyCase) -> Value {
        let mut out = Map::new();
        out.insert("slug".into(), json!(self.slug));
        let hash_key = match key_case {
            KeyCase::Camel => "documentHash",
            KeyCase::Snake => "document_hash",
        };
        out.insert(hash_key.into(), json!(self.document_hash));
        Value::Object(out)
    }
}

/// Key casing of the surface knowledge is attached to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyCase {
    Camel,
    Snake,
}

/// Attach knowledge to a registry schema response
/// (`GET /api/registry/<stack>/schema`): `summary` on each described entity
/// and view, `description` on each described field, and a top-level
/// `knowledge` naming the document. Keys stay snake_case like the response.
pub fn describe_schema(response: &mut Value, knowledge: &StackKnowledge) {
    let Some(entities) = response
        .pointer_mut("/schema/entities")
        .and_then(Value::as_array_mut)
    else {
        return;
    };
    for entity in entities {
        let Some(name) = entity.get("name").and_then(Value::as_str) else {
            continue;
        };
        let Some(entity_knowledge) = knowledge.entity(name) else {
            continue;
        };
        let Some(entity) = entity.as_object_mut() else {
            continue;
        };
        if let Some(summary) = &entity_knowledge.summary {
            entity.insert("summary".into(), json!(summary));
        }
        for field in entity
            .get_mut("fields")
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
        {
            let description = field
                .get("path")
                .and_then(Value::as_str)
                .and_then(|path| entity_knowledge.field_description(path));
            if let (Some(description), Some(field)) = (description, field.as_object_mut()) {
                field.insert("description".into(), json!(description));
            }
        }
        for view in entity
            .get_mut("views")
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
        {
            let summary = view
                .get("id")
                .and_then(Value::as_str)
                .and_then(|id| entity_knowledge.view_summary(id));
            if let (Some(summary), Some(view)) = (summary, view.as_object_mut()) {
                view.insert("summary".into(), json!(summary));
            }
        }
    }
    if let Some(response) = response.as_object_mut() {
        response.insert("knowledge".into(), knowledge.source(KeyCase::Snake));
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn ore_knowledge() -> Value {
        json!({
            "kind": "stack",
            "package": "ore",
            "version": "1.2.0",
            "stackManifestHash": "manifest-exact",
            "schemaVersion": "arete.knowledge-stack/v1",
            "documentHash": "arete:h1:knowledge-document:sha256:aa",
            "slug": "ore-stream",
            "stackName": "OreStream",
            "summary": "Live ORE v3 mining state.",
            "entities": {
                "OreRound": {
                    "summary": "One mining round.",
                    "concepts": ["mining"],
                    "views": {
                        "latest": {"summary": "The current round."},
                        "list": {"concepts": ["mining"]}
                    },
                    "fields": {
                        "results.preRevealWinningSquare": "The winning square before reveal; show this in a live UI.",
                        "results.winningSquare": "Only set once the next round opens.",
                        "state.blank": "  "
                    }
                },
                "OreBoard": "not an object"
            },
            "futureKey": {"ignored": true}
        })
    }

    #[test]
    fn paths_normalize_across_camel_and_snake_case() {
        assert_eq!(
            normalize_path("results.preRevealWinningSquare"),
            "results.prerevealwinningsquare"
        );
        assert_eq!(
            normalize_path("results.pre_reveal_winning_square"),
            "results.prerevealwinningsquare"
        );
        assert!(same_path(
            "results.pre_reveal_winning_square",
            "results.preRevealWinningSquare"
        ));
        assert!(same_path("OreRound", "ore_round"));
        // Segments never merge across the separator.
        assert!(!same_path("results.winningSquare", "resultsWinning.square"));
        assert!(!same_path(
            "results.winningSquare",
            "results.winningSquares"
        ));
        assert!(!same_path("state.winningSquare", "results.winningSquare"));
    }

    #[test]
    fn knowledge_parses_leniently_and_joins_by_normalized_path() {
        let knowledge = StackKnowledge::from_response(&ore_knowledge()).unwrap();
        assert_eq!(knowledge.slug, "ore-stream");
        assert_eq!(
            knowledge.stack_manifest_hash.as_deref(),
            Some("manifest-exact")
        );
        assert!(knowledge.entity("OreBoard").is_none());
        let round = knowledge.entity("ore_round").unwrap();
        assert_eq!(round.summary.as_deref(), Some("One mining round."));
        assert_eq!(
            round.field_description("results.pre_reveal_winning_square"),
            Some("The winning square before reveal; show this in a live UI.")
        );
        assert_eq!(
            round.field_description("results.preRevealWinningSquare"),
            round.field_description("results.pre_reveal_winning_square")
        );
        assert_eq!(round.field_description("state.blank"), None);
        assert_eq!(round.field_description("results.rng"), None);
        assert_eq!(
            round.view_summary("OreRound/latest"),
            Some("The current round.")
        );
        assert_eq!(round.view_summary("latest"), Some("The current round."));
        assert_eq!(round.view_summary("OreRound/list"), None);

        let mut fields = vec![
            EntityField {
                section: "results".into(),
                path: "results.winning_square".into(),
                rust_type: "Option<u8>".into(),
                nullable: true,
                description: None,
                amount: None,
            },
            EntityField {
                section: "results".into(),
                path: "results.rng".into(),
                rust_type: "Option<u64>".into(),
                nullable: true,
                description: None,
                amount: None,
            },
        ];
        round.describe_fields(&mut fields);
        assert_eq!(
            fields[0].description.as_deref(),
            Some("Only set once the next round opens.")
        );
        assert_eq!(fields[1].description, None);
        assert_eq!(
            serde_json::to_value(&fields[1]).unwrap(),
            json!({"section": "results", "path": "results.rng", "rustType": "Option<u64>", "nullable": true}),
            "an undescribed field serializes exactly as before"
        );
        assert_eq!(
            round.field_descriptions(["results.rng", "results.pre_reveal_winning_square"]),
            vec![json!({
                "path": "results.pre_reveal_winning_square",
                "description": "The winning square before reveal; show this in a live UI."
            })]
        );
    }

    #[test]
    fn only_stack_documents_with_an_identity_are_knowledge() {
        let mut program = ore_knowledge();
        program["kind"] = json!("program");
        assert!(StackKnowledge::from_response(&program).is_none());
        let mut anonymous = ore_knowledge();
        anonymous.as_object_mut().unwrap().remove("documentHash");
        assert!(StackKnowledge::from_response(&anonymous).is_none());
        assert!(StackKnowledge::from_response(&json!({"error": "not found"})).is_none());
        assert!(StackKnowledge::from_response(&Value::Null).is_none());
    }

    #[test]
    fn knowledge_is_only_looked_up_where_the_catalog_can_have_it() {
        for slug in ["ore", "ore-stream", "meteora_damm", "pump.fun", "Ore2"] {
            assert!(may_have_catalog_knowledge(slug, Some("public")), "{slug}");
            assert!(may_have_catalog_knowledge(slug, Some("global")), "{slug}");
            assert!(may_have_catalog_knowledge(slug, None), "{slug}");
        }
        assert!(!may_have_catalog_knowledge("ore", Some("private")));
        assert!(!may_have_catalog_knowledge("ore", Some("Private")));
        for slug in [
            "",
            "Ore Mining",
            "ore/stream",
            "..",
            "-ore",
            ".ore",
            "ore?x=1",
            "ore%2F",
        ] {
            assert!(
                !may_have_catalog_knowledge(slug, Some("public")),
                "{slug:?}"
            );
        }
        assert!(!may_have_catalog_knowledge(&"a".repeat(129), None));
    }

    #[test]
    fn knowledge_belongs_only_to_the_stack_manifest_it_names() {
        let knowledge = StackKnowledge::from_response(&ore_knowledge()).unwrap();
        assert!(knowledge.belongs_to("manifest-exact"));
        assert!(!knowledge.belongs_to("another-manifest"));
        assert!(!knowledge.belongs_to(""));
        let mut unpinned = ore_knowledge();
        unpinned
            .as_object_mut()
            .unwrap()
            .remove("stackManifestHash");
        let unpinned = StackKnowledge::from_response(&unpinned).unwrap();
        assert!(
            !unpinned.belongs_to("manifest-exact"),
            "knowledge that names no StackManifest cannot be checked"
        );
    }

    #[test]
    fn schema_responses_gain_snake_case_descriptions_and_summaries() {
        let knowledge = StackKnowledge::from_response(&ore_knowledge()).unwrap();
        let mut schema = json!({
            "name": "ore",
            "schema": {"stack_name": "OreStream", "entities": [
                {
                    "name": "OreRound",
                    "primary_keys": ["id.round_id"],
                    "fields": [
                        {"path": "results.pre_reveal_winning_square", "rust_type": "Option<u8>", "nullable": true, "section": "results"},
                        {"path": "results.rng", "rust_type": "Option<u64>", "nullable": true, "section": "results"}
                    ],
                    "views": [
                        {"id": "OreRound/latest", "mode": "single", "pipeline": []},
                        {"id": "OreRound/list", "mode": "list", "pipeline": []}
                    ]
                },
                {"name": "OreMiner", "primary_keys": [], "fields": [], "views": []}
            ]}
        });
        describe_schema(&mut schema, &knowledge);
        let round = &schema["schema"]["entities"][0];
        assert_eq!(round["summary"], "One mining round.");
        assert_eq!(
            round["fields"][0]["description"],
            "The winning square before reveal; show this in a live UI."
        );
        assert!(round["fields"][1].get("description").is_none());
        assert_eq!(round["views"][0]["summary"], "The current round.");
        assert!(round["views"][1].get("summary").is_none());
        assert!(schema["schema"]["entities"][1].get("summary").is_none());
        assert_eq!(
            schema["knowledge"],
            json!({"slug": "ore-stream", "document_hash": "arete:h1:knowledge-document:sha256:aa"})
        );
    }
}
