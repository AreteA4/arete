//! Instruction files: the `AGENTS.md` managed block, the `CLAUDE.md`
//! import line and the Gemini `context.fileName` setting (spec WP7 §2–4).

use std::path::PathBuf;

use serde_json::{json, Value};

use super::jsonc::JsonDoc;
use super::{display_path, read_optional, upsert_file, Env, ItemResult, Outcome};

/// Exact managed block (spec WP7 §2).
pub const BLOCK: &str = include_str!("templates/agents-block.md");
pub const BLOCK_VERSION: &str = "v1";
const BEGIN_PREFIX: &str = "<!-- BEGIN:arete";
const END_MARKER: &str = "<!-- END:arete -->";
pub const CLAUDE_IMPORT: &str = "@AGENTS.md";

/// State of the managed block in an `AGENTS.md`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockState {
    Missing,
    /// Present with a different (or no) version token.
    Stale(String),
    Current,
    /// A begin marker without a matching end marker (or vice versa); never
    /// rewritten automatically because the block boundary is unknown.
    Malformed(String),
}

pub const MALFORMED_FIX: &str =
    "fix the <!-- BEGIN:arete v1 --> / <!-- END:arete --> markers in AGENTS.md by hand, then rerun";

enum Span {
    Missing,
    Found(usize, usize, String),
    Malformed(String),
}

/// Line span (inclusive begin, inclusive end) of the managed block.
fn block_span(lines: &[&str]) -> Span {
    let begin = lines
        .iter()
        .position(|line| line.trim_start().starts_with(BEGIN_PREFIX));
    let end_from = |from: usize| {
        lines[from..]
            .iter()
            .position(|line| line.trim() == END_MARKER)
            .map(|offset| from + offset)
    };
    let Some(begin) = begin else {
        return match end_from(0) {
            Some(_) => Span::Malformed("end marker without a begin marker".to_string()),
            None => Span::Missing,
        };
    };
    let Some(end) = end_from(begin) else {
        return Span::Malformed("begin marker without an end marker".to_string());
    };
    let token = lines[begin]
        .trim()
        .trim_start_matches(BEGIN_PREFIX)
        .trim_end_matches("-->")
        .trim()
        .to_string();
    Span::Found(begin, end, token)
}

/// Inspect `content` for the managed block.
pub fn block_state(content: &str) -> BlockState {
    let lines: Vec<&str> = content.lines().collect();
    match block_span(&lines) {
        Span::Missing => BlockState::Missing,
        Span::Malformed(reason) => BlockState::Malformed(reason),
        Span::Found(begin, end, token) => {
            let current: Vec<&str> = lines[begin..=end].to_vec();
            if token == BLOCK_VERSION && current.join("\n") == BLOCK.trim_end_matches('\n') {
                BlockState::Current
            } else {
                BlockState::Stale(token)
            }
        }
    }
}

/// New `AGENTS.md` content: append the block (after a blank line) when the
/// markers are missing, otherwise replace between the markers. Everything
/// outside the markers is untouched. Errors when only one marker is present:
/// appending a second block would leave conflicting instructions.
pub fn upsert_block(existing: Option<&str>) -> anyhow::Result<String> {
    let block = BLOCK.trim_end_matches('\n');
    let Some(existing) = existing else {
        return Ok(format!("{block}\n"));
    };
    let lines: Vec<&str> = existing.lines().collect();
    match block_span(&lines) {
        Span::Found(begin, end, _) => {
            let mut out: Vec<&str> = Vec::with_capacity(lines.len());
            out.extend_from_slice(&lines[..begin]);
            out.extend(block.lines());
            out.extend_from_slice(&lines[end + 1..]);
            let mut text = out.join("\n");
            text.push('\n');
            Ok(text)
        }
        Span::Missing => {
            let body = existing.trim_end_matches(['\n', '\r']);
            if body.trim().is_empty() {
                Ok(format!("{block}\n"))
            } else {
                Ok(format!("{body}\n\n{block}\n"))
            }
        }
        Span::Malformed(reason) => {
            anyhow::bail!("AGENTS.md has a malformed Arete block ({reason}); {MALFORMED_FIX}")
        }
    }
}

/// Whether `CLAUDE.md` content imports `AGENTS.md`.
pub fn claude_md_ok(content: &str) -> bool {
    content.lines().any(|line| line.trim() == CLAUDE_IMPORT)
}

/// New `CLAUDE.md` content: exactly `@AGENTS.md\n` when missing, else the
/// import inserted as the first line when absent.
pub fn upsert_claude_md(existing: Option<&str>) -> String {
    match existing {
        None => format!("{CLAUDE_IMPORT}\n"),
        Some(content) if claude_md_ok(content) => content.to_string(),
        Some(content) => format!("{CLAUDE_IMPORT}\n{content}"),
    }
}

/// Desired Gemini `context.fileName` union.
const GEMINI_CONTEXT_FILES: [&str; 2] = ["AGENTS.md", "GEMINI.md"];

fn gemini_file_names(doc: &JsonDoc) -> Vec<String> {
    match doc.get(&["context", "fileName"]) {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| item.as_str().map(str::to_string))
            .collect(),
        Some(Value::String(single)) => vec![single],
        _ => Vec::new(),
    }
}

/// Whether Gemini settings content lists `AGENTS.md` as a context file.
pub fn gemini_context_ok(content: &str) -> bool {
    JsonDoc::parse(content)
        .map(|doc| {
            gemini_file_names(&doc)
                .iter()
                .any(|name| name == "AGENTS.md")
        })
        .unwrap_or(false)
}

/// New `.gemini/settings.json` content, or `None` when already complete.
pub fn upsert_gemini_context(existing: Option<&str>) -> anyhow::Result<Option<String>> {
    let doc = JsonDoc::parse(existing.unwrap_or(""))?;
    if !doc.root_is_object() {
        anyhow::bail!("root value is not an object");
    }
    let mut names = gemini_file_names(&doc);
    let before = names.len();
    for wanted in GEMINI_CONTEXT_FILES {
        if !names.iter().any(|name| name == wanted) {
            names.push(wanted.to_string());
        }
    }
    if names.len() == before && existing.is_some() {
        return Ok(None);
    }
    doc.set(&["context", "fileName"], &json!(names));
    Ok(Some(doc.render()))
}

pub fn agents_md_path(env: &Env) -> PathBuf {
    env.root.join("AGENTS.md")
}

pub fn claude_md_path(env: &Env) -> PathBuf {
    env.root.join("CLAUDE.md")
}

pub fn gemini_settings_path(env: &Env) -> PathBuf {
    env.root.join(".gemini").join("settings.json")
}

fn write_with(
    env: &Env,
    item: &str,
    path: PathBuf,
    dry_run: bool,
    desired: impl FnOnce(Option<&str>) -> anyhow::Result<Option<String>>,
) -> ItemResult {
    let shown = Some(display_path(env, &path));
    let existing = match read_optional(&path) {
        Ok(existing) => existing,
        Err(error) => {
            return ItemResult::new(item, Outcome::error(format!("{error:#}"), None), shown)
        }
    };
    let outcome = match desired(existing.as_deref()) {
        Ok(Some(content)) => upsert_file(&path, &content, dry_run),
        Ok(None) => Outcome::Unchanged,
        Err(error) => Outcome::error(
            format!("{}: {error:#}", path.display()),
            Some("fix the file by hand, then re-run a4 init".to_string()),
        ),
    };
    ItemResult::new(item, outcome, shown)
}

/// Writer: `AGENTS.md` managed block (`agents-md`).
pub fn write_agents_md(env: &Env, dry_run: bool) -> ItemResult {
    write_with(env, "agents-md", agents_md_path(env), dry_run, |existing| {
        upsert_block(existing).map(Some)
    })
}

/// Writer: `CLAUDE.md` import (`claude-md`).
pub fn write_claude_md(env: &Env, dry_run: bool) -> ItemResult {
    write_with(env, "claude-md", claude_md_path(env), dry_run, |existing| {
        Ok(Some(upsert_claude_md(existing)))
    })
}

/// Writer: Gemini `context.fileName` (`gemini-context`).
pub fn write_gemini_context(env: &Env, dry_run: bool) -> ItemResult {
    write_with(
        env,
        "gemini-context",
        gemini_settings_path(env),
        dry_run,
        upsert_gemini_context,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_has_markers_and_version() {
        assert!(BLOCK.starts_with("<!-- BEGIN:arete v1 -->\n"));
        assert!(BLOCK.ends_with("<!-- END:arete -->\n"));
        assert_eq!(block_state(BLOCK), BlockState::Current);
    }

    #[test]
    fn missing_block_is_appended_after_a_blank_line() {
        assert_eq!(upsert_block(None).unwrap(), BLOCK);
        assert_eq!(upsert_block(Some("")).unwrap(), BLOCK);
        let user = "# My project\n\nSome notes.\n";
        let updated = upsert_block(Some(user)).unwrap();
        assert_eq!(updated, format!("# My project\n\nSome notes.\n\n{BLOCK}"));
        assert_eq!(block_state(&updated), BlockState::Current);
        // Idempotent.
        assert_eq!(upsert_block(Some(&updated)).unwrap(), updated);
        // Missing trailing newline is normalised to exactly one blank line.
        assert_eq!(upsert_block(Some("x")).unwrap(), format!("x\n\n{BLOCK}"));
    }

    #[test]
    fn stale_block_is_replaced_and_surroundings_kept() {
        let stale = "above\n\n<!-- BEGIN:arete v0 -->\nold stuff\n<!-- END:arete -->\n\nbelow\n";
        assert_eq!(block_state(stale), BlockState::Stale("v0".into()));
        let updated = upsert_block(Some(stale)).unwrap();
        assert_eq!(updated, format!("above\n\n{BLOCK}\nbelow\n"));
        assert_eq!(block_state(&updated), BlockState::Current);
        // Same token but edited body counts as stale and is restored.
        let edited = BLOCK.replace("## Arete", "## Arete (edited)");
        assert!(matches!(block_state(&edited), BlockState::Stale(_)));
        assert_eq!(upsert_block(Some(&edited)).unwrap(), BLOCK);
    }

    #[test]
    fn claude_md_import_is_first_line() {
        assert_eq!(upsert_claude_md(None), "@AGENTS.md\n");
        assert_eq!(upsert_claude_md(Some("# Notes\n")), "@AGENTS.md\n# Notes\n");
        assert_eq!(
            upsert_claude_md(Some("x\n@AGENTS.md\ny\n")),
            "x\n@AGENTS.md\ny\n"
        );
        assert!(claude_md_ok("  @AGENTS.md  \n"));
        assert!(!claude_md_ok("see @AGENTS.md"));
    }

    #[test]
    fn gemini_context_is_a_union() {
        let created = upsert_gemini_context(None).unwrap().unwrap();
        assert!(gemini_context_ok(&created));
        let parsed: Value = serde_json::from_str(&created).unwrap();
        assert_eq!(
            parsed["context"]["fileName"],
            json!(["AGENTS.md", "GEMINI.md"])
        );

        let existing = r#"{"theme": "x", "context": {"fileName": ["GEMINI.md", "OTHER.md"]}}"#;
        let updated = upsert_gemini_context(Some(existing)).unwrap().unwrap();
        let parsed: Value = serde_json::from_str(&updated).unwrap();
        assert_eq!(
            parsed["context"]["fileName"],
            json!(["GEMINI.md", "OTHER.md", "AGENTS.md"])
        );
        assert_eq!(parsed["theme"], "x");
        assert_eq!(upsert_gemini_context(Some(&updated)).unwrap(), None);

        // A string value is promoted to an array.
        let single = r#"{"context": {"fileName": "GEMINI.md"}}"#;
        let updated = upsert_gemini_context(Some(single)).unwrap().unwrap();
        assert!(gemini_context_ok(&updated));
    }

    #[test]
    fn unterminated_block_is_reported_not_duplicated() {
        let partial = "# Mine\n\n<!-- BEGIN:arete v1 -->\n## Arete\nhalf a block\n";
        assert!(matches!(block_state(partial), BlockState::Malformed(_)));
        let error = upsert_block(Some(partial)).unwrap_err().to_string();
        assert!(error.contains("malformed"), "{error}");
        assert!(error.contains("by hand"), "{error}");

        let orphan_end = "text\n<!-- END:arete -->\n";
        assert!(matches!(block_state(orphan_end), BlockState::Malformed(_)));
        assert!(upsert_block(Some(orphan_end)).is_err());
    }
}
