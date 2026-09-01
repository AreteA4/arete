//! `a4 know` — query the curated knowledge layer.
//!
//! Thin client of the registry knowledge API
//! (`/api/registry/knowledge/*`). Every route requires an API key, so all
//! subcommands go through the same `ApiClient` credential resolution as the
//! other authenticated commands (`ARETE_API_URL` +
//! `~/.arete/credentials.toml`); an unauthenticated call fails with the
//! standard "Run 'a4 auth login' first" error before any request is sent.
//!
//! The API returns raw JSON that `--json` prints verbatim (pretty-printed).
//! The readable rendering parses that JSON *leniently* — every field is
//! optional and unknown fields are ignored — because the knowledge payloads
//! are additive over time and a new server field must never break an older
//! CLI. Where the contract leaves casing open, both snake_case and camelCase
//! are accepted via serde aliases.

use std::fmt::Write as _;

use anyhow::{bail, Result};
use colored::Colorize;
use serde::Deserialize;
use serde_json::Value;

use crate::api_client::ApiClient;

/// Sections accepted by `a4 know program --section`, mirroring
/// `GET /api/registry/knowledge/programs/{slug}?section=...`.
const SECTIONS: [&str; 4] = ["summary", "instructions", "accounts", "surface"];

// ============================================================================
// Subcommand entry points
// ============================================================================

pub fn search(
    query: Option<&str>,
    concept: Option<&str>,
    category: Option<&str>,
    limit: Option<usize>,
    json: bool,
) -> Result<()> {
    let query = non_empty(query);
    let concept = non_empty(concept);
    let category = non_empty(category);
    if query.is_none() && concept.is_none() && category.is_none() {
        bail!(
            "Provide at least one of --query, --concept, or --category. \
             Run `a4 know concepts` to list concept and category slugs."
        );
    }
    if let Some(concept) = concept {
        validate_slug(concept, "--concept")?;
    }
    if let Some(category) = category {
        validate_slug(category, "--category")?;
    }

    let value = ApiClient::new()?.knowledge_search(query, concept, category, limit)?;
    emit(&value, json, render_search)
}

pub fn protocol(slug: &str, json: bool) -> Result<()> {
    let slug = validate_slug(slug, "protocol slug")?;
    let value = ApiClient::new()?.knowledge_protocol(slug)?;
    emit(&value, json, render_protocol)
}

pub fn program(slug: &str, section: Option<&str>, json: bool) -> Result<()> {
    let slug = validate_slug(slug, "program slug")?;
    let section = validate_section(section)?;
    let value = ApiClient::new()?.knowledge_program(slug, section)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&value)?);
        return Ok(());
    }
    print!("{}", render_program(&value, section.unwrap_or("summary")));
    Ok(())
}

pub fn recipe(slug: &str, json: bool) -> Result<()> {
    let slug = validate_slug(slug, "recipe slug")?;
    let value = ApiClient::new()?.knowledge_recipe(slug)?;
    emit(&value, json, render_recipe)
}

pub fn concepts(json: bool) -> Result<()> {
    let value = ApiClient::new()?.knowledge_vocabulary()?;
    emit(&value, json, render_vocabulary)
}

fn emit(value: &Value, json: bool, render: impl Fn(&Value) -> String) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(value)?);
    } else {
        print!("{}", render(value));
    }
    Ok(())
}

// ============================================================================
// Input validation
// ============================================================================

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|s| !s.is_empty())
}

/// Validate a slug destined for a URL path segment or slug filter.
///
/// Same reasoning as the MCP server's registry client: rejecting instead of
/// percent-encoding turns "you passed a path or URL where a bare slug
/// belongs" into a clear error instead of a confusing 404 — and since these
/// requests carry a bearer token, a `\`/`..`-shaped segment must never reach
/// URL normalization, where it could walk out of `/api/registry/knowledge/`.
fn validate_slug<'a>(value: &'a str, what: &str) -> Result<&'a str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        bail!("{what} must not be empty");
    }
    if let Some(bad) = trimmed
        .chars()
        .find(|c| c.is_whitespace() || c.is_control() || "/\\?#%&".contains(*c))
    {
        bail!(
            "{what} contains an invalid character {bad:?}. Pass a bare slug \
             (e.g. `meteora-damm`), not a URL or path."
        );
    }
    if trimmed.chars().all(|c| c == '.') {
        bail!(
            "{what} must not be a relative path segment. Pass a bare slug \
             (e.g. `meteora-damm`), not a URL or path."
        );
    }
    Ok(trimmed)
}

fn validate_section(section: Option<&str>) -> Result<Option<&str>> {
    match non_empty(section) {
        None => Ok(None),
        Some(section) if SECTIONS.contains(&section) => Ok(Some(section)),
        Some(other) => bail!(
            "--section must be one of: {}. Got `{other}`.",
            SECTIONS.join(", ")
        ),
    }
}

// ============================================================================
// Lenient response models (readable rendering only; --json bypasses these)
// ============================================================================

#[derive(Debug, Default, Deserialize)]
struct SearchResponse {
    #[serde(default, alias = "matchedConcepts")]
    matched_concepts: Vec<String>,
    #[serde(default)]
    results: Vec<SearchResult>,
}

#[derive(Debug, Default, Deserialize)]
struct SearchResult {
    #[serde(default, rename = "type")]
    result_type: String,
    #[serde(default)]
    slug: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    summary: Option<String>,
    /// Owning protocol slug for program/recipe/view results.
    #[serde(default)]
    protocol: Option<String>,
    #[serde(default)]
    coverage: Coverage,
    #[serde(default)]
    score: Option<f64>,
}

#[derive(Debug, Default, Deserialize)]
struct Coverage {
    #[serde(default)]
    read: bool,
    #[serde(default)]
    build: bool,
    #[serde(default)]
    subscribe: bool,
}

#[derive(Debug, Default, Deserialize)]
struct VocabularyResponse {
    #[serde(default)]
    concepts: Vec<ConceptEntry>,
    #[serde(default)]
    categories: Vec<CategoryEntry>,
}

#[derive(Debug, Default, Deserialize)]
struct ConceptEntry {
    #[serde(default)]
    slug: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    synonyms: Vec<String>,
    #[serde(default)]
    related: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
struct CategoryEntry {
    #[serde(default)]
    slug: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct ProtocolResponse {
    #[serde(default)]
    slug: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    categories: Vec<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    links: serde_json::Map<String, Value>,
    #[serde(default)]
    programs: Vec<ProtocolProgram>,
    #[serde(default)]
    related: Vec<ProtocolRelation>,
    #[serde(default)]
    stacks: Vec<ProtocolStack>,
    #[serde(default)]
    coverage: Vec<CoverageRow>,
}

#[derive(Debug, Default, Deserialize)]
struct ProtocolProgram {
    #[serde(default)]
    slug: String,
    #[serde(default, alias = "programId")]
    program_id: String,
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    linked: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
struct ProtocolRelation {
    #[serde(default)]
    protocol: String,
    #[serde(default)]
    relation: Option<String>,
    #[serde(default)]
    note: Option<String>,
    #[serde(default)]
    evidence: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct ProtocolStack {
    #[serde(default)]
    stack: String,
    #[serde(default)]
    entities: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
struct CoverageRow {
    #[serde(default)]
    concept: String,
    #[serde(default)]
    read: bool,
    #[serde(default)]
    build: bool,
    #[serde(default)]
    subscribe: bool,
}

#[derive(Debug, Default, Deserialize)]
struct ProgramHeader {
    #[serde(default)]
    slug: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    protocol: Option<String>,
    #[serde(default, alias = "programId")]
    program_id: Option<String>,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    provenance: Option<Provenance>,
    #[serde(default)]
    counts: serde_json::Map<String, Value>,
}

#[derive(Debug, Default, Deserialize)]
struct Provenance {
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    reviewed: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
struct AnnotationEntry {
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    concepts: Vec<String>,
    /// Dotted arg paths → meaning (instructions).
    #[serde(default)]
    args: serde_json::Map<String, Value>,
    /// Dotted field paths → meaning (accounts).
    #[serde(default)]
    fields: serde_json::Map<String, Value>,
    /// Account name → meaning (instructions).
    #[serde(default)]
    accounts: serde_json::Map<String, Value>,
}

#[derive(Debug, Default, Deserialize)]
struct RecipeResponse {
    #[serde(default)]
    slug: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    concepts: Vec<String>,
    #[serde(default)]
    protocols: Vec<String>,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    steps: Vec<Value>,
    #[serde(default)]
    example: Option<String>,
    #[serde(
        default,
        alias = "surfaceRefs",
        alias = "resolvedSurfaceRefs",
        alias = "resolved_surface_refs"
    )]
    surface_refs: Vec<Value>,
}

// ============================================================================
// Rendering
// ============================================================================

fn coverage_flags(read: bool, build: bool, subscribe: bool) -> String {
    let flags: Vec<&str> = [(read, "read"), (build, "build"), (subscribe, "subscribe")]
        .iter()
        .filter_map(|(on, label)| on.then_some(*label))
        .collect();
    if flags.is_empty() {
        "none".into()
    } else {
        flags.join(", ")
    }
}

fn parse_lenient<T: Default + for<'de> Deserialize<'de>>(value: &Value) -> T {
    serde_json::from_value(value.clone()).unwrap_or_default()
}

fn render_search(value: &Value) -> String {
    let parsed: SearchResponse = parse_lenient(value);
    let mut text = String::new();
    if !parsed.matched_concepts.is_empty() {
        let _ = writeln!(
            text,
            "\n{} {}",
            "Matched concepts:".bold(),
            parsed.matched_concepts.join(", ").green()
        );
    }
    if parsed.results.is_empty() {
        let _ = writeln!(
            text,
            "\n{}",
            "No results. Try a broader --query, or `a4 know concepts` for filter slugs.".yellow()
        );
        return text;
    }
    let _ = writeln!(text, "\n{}", "Results".bold());
    let _ = writeln!(text, "{}", "-".repeat(60).dimmed());
    for result in &parsed.results {
        let _ = writeln!(
            text,
            "  [{}] {}  {}",
            result.result_type.cyan(),
            result.slug.green().bold(),
            result.name
        );
        let _ = writeln!(
            text,
            "    coverage: {}",
            coverage_flags(
                result.coverage.read,
                result.coverage.build,
                result.coverage.subscribe
            )
        );
        if let Some(protocol) = &result.protocol {
            if protocol != &result.slug {
                let _ = writeln!(text, "    protocol: {protocol}");
            }
        }
        if let Some(score) = result.score {
            let _ = writeln!(text, "    score: {score:.2}");
        }
        if let Some(summary) = &result.summary {
            let _ = writeln!(text, "    {}", summary.trim().dimmed());
        }
        text.push('\n');
    }
    let _ = writeln!(
        text,
        "{}",
        "Tip: `a4 know protocol <slug>`, `a4 know program <slug>`, or `a4 know recipe <slug>` for detail"
            .dimmed()
    );
    text
}

fn render_vocabulary(value: &Value) -> String {
    let parsed: VocabularyResponse = parse_lenient(value);
    let mut text = String::new();
    let _ = writeln!(text, "\n{}", "Concepts".bold());
    let _ = writeln!(text, "{}", "-".repeat(60).dimmed());
    if parsed.concepts.is_empty() {
        let _ = writeln!(text, "  none");
    }
    for concept in &parsed.concepts {
        let _ = writeln!(text, "  {}  {}", concept.slug.green().bold(), concept.name);
        if let Some(description) = &concept.description {
            let _ = writeln!(text, "    {}", description.trim().dimmed());
        }
        if !concept.synonyms.is_empty() {
            let _ = writeln!(text, "    synonyms: {}", concept.synonyms.join(", "));
        }
        if !concept.related.is_empty() {
            let _ = writeln!(text, "    related: {}", concept.related.join(", "));
        }
    }
    let _ = writeln!(text, "\n{}", "Categories".bold());
    let _ = writeln!(text, "{}", "-".repeat(60).dimmed());
    if parsed.categories.is_empty() {
        let _ = writeln!(text, "  none");
    }
    for category in &parsed.categories {
        let _ = writeln!(
            text,
            "  {}  {}",
            category.slug.green().bold(),
            category.name
        );
        if let Some(description) = &category.description {
            let _ = writeln!(text, "    {}", description.trim().dimmed());
        }
    }
    let _ = writeln!(
        text,
        "\n{}",
        "Tip: filter searches with `a4 know search --concept <slug>` or `--category <slug>`"
            .dimmed()
    );
    text
}

fn render_protocol(value: &Value) -> String {
    let parsed: ProtocolResponse = parse_lenient(value);
    let mut text = String::new();
    let _ = writeln!(
        text,
        "\n{} {} ({})",
        "Protocol:".bold(),
        parsed.name.green().bold(),
        parsed.slug
    );
    if !parsed.categories.is_empty() {
        let _ = writeln!(text, "  Categories: {}", parsed.categories.join(", "));
    }
    if let Some(status) = &parsed.status {
        let _ = writeln!(text, "  Status: {status}");
    }
    if let Some(description) = &parsed.description {
        let _ = writeln!(text, "  {}", description.trim().dimmed());
    }
    if !parsed.links.is_empty() {
        let _ = writeln!(text, "\n{}", "Links".bold());
        for (name, url) in &parsed.links {
            if let Some(url) = url.as_str() {
                let _ = writeln!(text, "  {name}: {}", url.cyan());
            }
        }
    }
    let _ = writeln!(text, "\n{}", "Programs".bold());
    if parsed.programs.is_empty() {
        let _ = writeln!(text, "  none");
    }
    for program in &parsed.programs {
        let mut annotations = Vec::new();
        if let Some(role) = &program.role {
            annotations.push(role.clone());
        }
        if program.linked == Some(false) {
            annotations.push("unlinked".into());
        }
        let suffix = if annotations.is_empty() {
            String::new()
        } else {
            format!("  [{}]", annotations.join(", "))
        };
        let _ = writeln!(
            text,
            "  {}  {}{}",
            program.slug.green().bold(),
            program.program_id.cyan(),
            suffix
        );
    }
    if !parsed.related.is_empty() {
        let _ = writeln!(text, "\n{}", "Related protocols".bold());
        for related in &parsed.related {
            let relation = related.relation.as_deref().unwrap_or("related-to");
            let _ = writeln!(
                text,
                "  {}  {}",
                related.protocol.green().bold(),
                relation.cyan()
            );
            if let Some(note) = &related.note {
                let _ = writeln!(text, "    {}", note.trim().dimmed());
            }
            if let Some(evidence) = &related.evidence {
                let _ = writeln!(text, "    evidence: {evidence}");
            }
        }
    }
    if !parsed.stacks.is_empty() {
        let _ = writeln!(text, "\n{}", "Live stacks".bold());
        for stack in &parsed.stacks {
            let _ = writeln!(
                text,
                "  {}  entities: {}",
                stack.stack.green().bold(),
                stack.entities.join(", ")
            );
        }
    }
    if !parsed.coverage.is_empty() {
        let _ = writeln!(text, "\n{}", "Coverage".bold());
        for row in &parsed.coverage {
            let _ = writeln!(
                text,
                "  {}: {}",
                row.concept.green(),
                coverage_flags(row.read, row.build, row.subscribe)
            );
        }
    }
    let _ = writeln!(
        text,
        "\n{}",
        "Tip: `a4 know program <slug> --section instructions` for instruction semantics".dimmed()
    );
    text
}

fn render_program(value: &Value, section: &str) -> String {
    match section {
        "instructions" => render_annotation_map(value, "instructions", "Instructions"),
        "accounts" => render_annotation_map(value, "accounts", "Accounts"),
        // The surface section is SDK extension-surface entries with bindings;
        // its shape is owned by the surface artifacts, so pretty JSON is the
        // honest rendering rather than a lossy summary.
        "surface" => format!(
            "\n{}\n{}\n",
            "Surface entries".bold(),
            serde_json::to_string_pretty(
                value
                    .get("surface")
                    .or_else(|| value.get("entries"))
                    .unwrap_or(value)
            )
            .unwrap_or_else(|_| value.to_string())
        ),
        _ => render_program_summary(value),
    }
}

fn render_program_summary(value: &Value) -> String {
    let parsed: ProgramHeader = parse_lenient(value);
    let mut text = String::new();
    let _ = writeln!(
        text,
        "\n{} {}{}",
        "Program:".bold(),
        parsed.slug.green().bold(),
        parsed
            .name
            .as_deref()
            .map(|name| format!(" ({name})"))
            .unwrap_or_default()
    );
    if let Some(protocol) = &parsed.protocol {
        let _ = writeln!(text, "  Protocol: {protocol}");
    }
    if let Some(program_id) = &parsed.program_id {
        let _ = writeln!(text, "  Program ID: {}", program_id.cyan());
    }
    if let Some(summary) = &parsed.summary {
        let _ = writeln!(text, "  {}", summary.trim().dimmed());
    }
    if let Some(provenance) = &parsed.provenance {
        let source = provenance.source.as_deref().unwrap_or("unknown");
        let reviewed = match provenance.reviewed {
            Some(true) => "reviewed",
            Some(false) => "unreviewed",
            None => "review status unknown",
        };
        let model = provenance
            .model
            .as_deref()
            .map(|model| format!(", model {model}"))
            .unwrap_or_default();
        let _ = writeln!(text, "  Provenance: {source} ({reviewed}{model})");
    }
    if !parsed.counts.is_empty() {
        let _ = writeln!(text, "\n{}", "Counts".bold());
        for (name, count) in &parsed.counts {
            let _ = writeln!(text, "  {name}: {count}");
        }
    }
    let _ = writeln!(
        text,
        "\n{}",
        "Tip: --section instructions | accounts | surface for detail".dimmed()
    );
    text
}

fn render_annotation_map(value: &Value, key: &str, heading: &str) -> String {
    let mut text = String::new();
    let _ = writeln!(text, "\n{}", heading.bold());
    let _ = writeln!(text, "{}", "-".repeat(60).dimmed());
    let Some(map) = value.get(key).and_then(Value::as_object) else {
        let _ = writeln!(text, "  none");
        return text;
    };
    if map.is_empty() {
        let _ = writeln!(text, "  none");
        return text;
    }
    for (name, entry_value) in map {
        let entry: AnnotationEntry = parse_lenient(entry_value);
        let _ = writeln!(text, "  {}", name.green().bold());
        if let Some(summary) = &entry.summary {
            let _ = writeln!(text, "    {}", summary.trim());
        }
        if !entry.concepts.is_empty() {
            let _ = writeln!(text, "    concepts: {}", entry.concepts.join(", ").cyan());
        }
        for (label, details) in [
            ("args", &entry.args),
            ("accounts", &entry.accounts),
            ("fields", &entry.fields),
        ] {
            if details.is_empty() {
                continue;
            }
            let _ = writeln!(text, "    {label}:");
            for (path, meaning) in details {
                let meaning = meaning
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| meaning.to_string());
                let _ = writeln!(text, "      {path}: {}", meaning.trim().dimmed());
            }
        }
    }
    text
}

fn render_recipe(value: &Value) -> String {
    let parsed: RecipeResponse = parse_lenient(value);
    let mut text = String::new();
    let _ = writeln!(
        text,
        "\n{} {}",
        "Recipe:".bold(),
        parsed
            .title
            .as_deref()
            .unwrap_or(&parsed.slug)
            .green()
            .bold()
    );
    let _ = writeln!(text, "  Slug: {}", parsed.slug);
    if let Some(mode) = &parsed.mode {
        let _ = writeln!(text, "  Mode: {mode}");
    }
    if !parsed.protocols.is_empty() {
        let _ = writeln!(text, "  Protocols: {}", parsed.protocols.join(", ").cyan());
    }
    if !parsed.concepts.is_empty() {
        let _ = writeln!(text, "  Concepts: {}", parsed.concepts.join(", "));
    }
    if let Some(summary) = &parsed.summary {
        let _ = writeln!(text, "  {}", summary.trim().dimmed());
    }
    let _ = writeln!(text, "\n{}", "Steps".bold());
    if parsed.steps.is_empty() {
        let _ = writeln!(text, "  none");
    }
    for (index, step) in parsed.steps.iter().enumerate() {
        let number = index + 1;
        match step {
            Value::String(step_text) => {
                let _ = writeln!(text, "  {number}. {}", step_text.trim());
            }
            Value::Object(step) => {
                let step_text = ["text", "summary", "description", "step"]
                    .iter()
                    .find_map(|key| step.get(*key).and_then(Value::as_str))
                    .unwrap_or("(unnamed step)");
                let _ = writeln!(text, "  {number}. {}", step_text.trim());
                if let Some(reference) = step.get("ref") {
                    let sdk = reference.get("sdk").and_then(Value::as_str);
                    let path = reference.get("path").and_then(Value::as_str);
                    match (sdk, path) {
                        (Some(sdk), Some(path)) => {
                            let _ = writeln!(text, "     ref: {} → {path}", sdk.cyan());
                        }
                        _ => {
                            let _ = writeln!(text, "     ref: {reference}");
                        }
                    }
                }
            }
            other => {
                let _ = writeln!(text, "  {number}. {other}");
            }
        }
    }
    if let Some(example) = &parsed.example {
        let _ = writeln!(text, "\n{}\n  {}", "Example".bold(), example.cyan());
    }
    if !parsed.surface_refs.is_empty() {
        let _ = writeln!(text, "\n{}", "Resolved surface refs".bold());
        let _ = writeln!(
            text,
            "{}",
            serde_json::to_string_pretty(&parsed.surface_refs)
                .unwrap_or_else(|_| "  (unrenderable)".into())
        );
    }
    text
}

// ============================================================================
// Tests — fixtures are constructed from the Registry API contract v1
// response shapes; no live server is involved.
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn search_fixture() -> Value {
        json!({
            "matched_concepts": ["swap"],
            "results": [
                {
                    "type": "protocol",
                    "slug": "meteora-damm",
                    "name": "Meteora DAMM v2",
                    "summary": "Meteora's dynamic AMM (constant-product with dynamic fees).",
                    "protocol": "meteora-damm",
                    "coverage": { "read": true, "build": true, "subscribe": true },
                    "score": 12.5
                },
                {
                    "type": "recipe",
                    "slug": "execute-presale-purchase-via-squads",
                    "name": "Execute a Meteora presale purchase through a Squads multisig",
                    "summary": "Wrap the prepared purchase as a Squads vault transaction.",
                    "protocol": "meteora-presale",
                    "coverage": { "read": false, "build": true, "subscribe": false },
                    "score": 4.0
                }
            ]
        })
    }

    #[test]
    fn search_rendering_shows_concepts_results_and_coverage_flags() {
        let text = render_search(&search_fixture());
        assert!(text.contains("Matched concepts"), "{text}");
        assert!(text.contains("swap"), "{text}");
        assert!(text.contains("meteora-damm"), "{text}");
        assert!(text.contains("Meteora DAMM v2"), "{text}");
        assert!(text.contains("read, build, subscribe"), "{text}");
        // Build-only coverage renders just the modes that are on.
        assert!(text.contains("coverage: build"), "{text}");
        assert!(text.contains("[recipe]"), "{text}");
    }

    #[test]
    fn search_rendering_survives_missing_and_unknown_fields() {
        // A new server may add fields and omit optional ones; neither may
        // break rendering.
        let text = render_search(&json!({
            "results": [{ "type": "program", "slug": "bare", "brand_new_field": {"x": 1} }]
        }));
        assert!(text.contains("bare"), "{text}");
        assert!(text.contains("coverage: none"), "{text}");

        let empty = render_search(&json!({ "matched_concepts": [], "results": [] }));
        assert!(empty.contains("No results"), "{empty}");
    }

    #[test]
    fn vocabulary_rendering_lists_both_namespaces() {
        let text = render_vocabulary(&json!({
            "concepts": [{
                "slug": "swap",
                "name": "Swap",
                "description": "Exchange one token for another.",
                "synonyms": ["trade", "exchange"],
                "related": ["add-liquidity"]
            }],
            "categories": [{
                "slug": "dex",
                "name": "DEX",
                "description": "Decentralized exchange or AMM."
            }]
        }));
        assert!(text.contains("Concepts"), "{text}");
        assert!(text.contains("swap"), "{text}");
        assert!(text.contains("synonyms: trade, exchange"), "{text}");
        assert!(text.contains("related: add-liquidity"), "{text}");
        assert!(text.contains("Categories"), "{text}");
        assert!(text.contains("dex"), "{text}");
    }

    #[test]
    fn protocol_rendering_covers_programs_relations_stacks_and_coverage() {
        let text = render_protocol(&json!({
            "slug": "meteora-damm",
            "name": "Meteora DAMM v2",
            "description": "Dynamic AMM.",
            "categories": ["dex"],
            "status": "curated",
            "links": { "website": "https://meteora.ag" },
            "programs": [{
                "slug": "meteora-cp-amm",
                "program_id": "cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG",
                "role": "core",
                "linked": true
            }],
            "related": [{
                "protocol": "squads-v4",
                "relation": "composes-with",
                "note": "Admin actions run through a Squads vault.",
                "evidence": "recipes/execute-presale-purchase-via-squads"
            }],
            "stacks": [{ "stack": "meteora-damm-stream", "entities": ["MeteoraPool"] }],
            "coverage": [{ "concept": "swap", "read": true, "build": true, "subscribe": false }]
        }));
        assert!(text.contains("Meteora DAMM v2"), "{text}");
        assert!(text.contains("Categories: dex"), "{text}");
        assert!(text.contains("meteora-cp-amm"), "{text}");
        assert!(
            text.contains("cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG"),
            "{text}"
        );
        assert!(text.contains("[core]"), "{text}");
        assert!(text.contains("squads-v4"), "{text}");
        assert!(text.contains("composes-with"), "{text}");
        assert!(text.contains("meteora-damm-stream"), "{text}");
        assert!(text.contains("swap: read, build"), "{text}");
    }

    #[test]
    fn protocol_rendering_accepts_camel_case_program_ids() {
        // §12 writes `program_id`; the authored YAML uses `programId`. Both
        // must render until the server pins one.
        let text = render_protocol(&json!({
            "slug": "p",
            "name": "P",
            "programs": [{ "slug": "x", "programId": "Prog111" }]
        }));
        assert!(text.contains("Prog111"), "{text}");
    }

    #[test]
    fn program_summary_rendering_shows_header_provenance_and_counts() {
        let text = render_program(
            &json!({
                "slug": "meteora-cp-amm",
                "name": "cp_amm",
                "protocol": "meteora-damm",
                "programId": "cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG",
                "summary": "Constant-product AMM program.",
                "provenance": { "source": "llm-draft", "model": "some-model", "reviewed": true },
                "counts": { "instructions": 12, "accounts": 4 }
            }),
            "summary",
        );
        assert!(text.contains("meteora-cp-amm"), "{text}");
        assert!(text.contains("cp_amm"), "{text}");
        assert!(text.contains("llm-draft"), "{text}");
        assert!(text.contains("reviewed, model some-model"), "{text}");
        assert!(text.contains("instructions: 12"), "{text}");
    }

    #[test]
    fn program_instruction_rendering_shows_annotation_maps() {
        let text = render_program(
            &json!({
                "slug": "meteora-cp-amm",
                "instructions": {
                    "swap2": {
                        "summary": "Swap with exact-in or exact-out semantics.",
                        "concepts": ["swap"],
                        "args": { "params.amount": "Amount in base units." },
                        "accounts": { "pool": "The pool being traded against." }
                    }
                }
            }),
            "instructions",
        );
        assert!(text.contains("swap2"), "{text}");
        assert!(text.contains("exact-in or exact-out"), "{text}");
        assert!(text.contains("concepts: swap"), "{text}");
        assert!(
            text.contains("params.amount: Amount in base units."),
            "{text}"
        );
        assert!(
            text.contains("pool: The pool being traded against."),
            "{text}"
        );
    }

    #[test]
    fn program_accounts_rendering_shows_field_meanings() {
        let text = render_program(
            &json!({
                "accounts": {
                    "Pool": {
                        "summary": "One trading pair.",
                        "concepts": ["swap"],
                        "fields": { "sqrt_price": "Current price as a Q64.64 square root." }
                    }
                }
            }),
            "accounts",
        );
        assert!(text.contains("Pool"), "{text}");
        assert!(text.contains("sqrt_price: Current price"), "{text}");
    }

    #[test]
    fn program_rendering_reports_absent_sections_as_none() {
        let text = render_program(&json!({ "slug": "x" }), "instructions");
        assert!(text.contains("none"), "{text}");
    }

    #[test]
    fn recipe_rendering_shows_steps_refs_and_example() {
        let text = render_recipe(&json!({
            "slug": "execute-presale-purchase-via-squads",
            "title": "Execute a Meteora presale purchase through a Squads multisig",
            "summary": "Prepare the purchase, then wrap it for the multisig.",
            "concepts": ["multisig-execute"],
            "protocols": ["meteora-presale", "squads-v4"],
            "mode": "build",
            "steps": [
                {
                    "text": "Prepare the purchase with the presale SDK; do not send it.",
                    "ref": { "sdk": "sdks/meteora-presale", "path": "transactions.purchase.create" }
                },
                "Members approve; anyone executes once threshold is met."
            ],
            "example": "jurassic/recipes/presale.ts"
        }));
        assert!(text.contains("Squads multisig"), "{text}");
        assert!(
            text.contains("Protocols: meteora-presale, squads-v4"),
            "{text}"
        );
        assert!(text.contains("1. Prepare the purchase"), "{text}");
        assert!(text.contains("sdks/meteora-presale"), "{text}");
        assert!(text.contains("transactions.purchase.create"), "{text}");
        assert!(text.contains("2. Members approve"), "{text}");
        assert!(text.contains("jurassic/recipes/presale.ts"), "{text}");
    }

    #[test]
    fn slug_validation_rejects_path_and_url_shapes() {
        // Same failure modes the MCP registry client guards: URL-parser
        // normalization must never see `\` or `..` from a user-supplied slug.
        for bad in [
            "",
            "   ",
            "a slug",
            "stacks/ore",
            r"..\..\agents\me",
            "..",
            ".",
            "slug?x=1",
            "slug#frag",
            "%2e%2e",
        ] {
            assert!(
                validate_slug(bad, "slug").is_err(),
                "expected {bad:?} to be refused"
            );
        }
        assert_eq!(
            validate_slug(" meteora-damm ", "slug").unwrap(),
            "meteora-damm"
        );
    }

    #[test]
    fn section_validation_matches_the_contract() {
        for section in SECTIONS {
            assert_eq!(validate_section(Some(section)).unwrap(), Some(section));
        }
        assert_eq!(validate_section(None).unwrap(), None);
        assert_eq!(validate_section(Some("")).unwrap(), None);
        let err = validate_section(Some("idl")).unwrap_err().to_string();
        assert!(err.contains("must be one of"), "{err}");
        assert!(
            err.contains("summary, instructions, accounts, surface"),
            "{err}"
        );
    }

    #[test]
    fn search_requires_at_least_one_filter() {
        let err = search(None, Some("  "), None, None, false)
            .unwrap_err()
            .to_string();
        assert!(err.contains("at least one of"), "{err}");
        assert!(err.contains("a4 know concepts"), "{err}");
    }

    #[test]
    fn search_rejects_malformed_slug_filters_before_any_request() {
        let err = search(None, Some("swap/../x"), None, None, false)
            .unwrap_err()
            .to_string();
        assert!(err.contains("invalid character"), "{err}");
    }
}
