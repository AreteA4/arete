//! Result reporting shared by `a4 init` and `a4 doctor --fix`: per-item
//! outcomes, the `--json` object and the human one-line-per-item rendering.

use colored::Colorize;
use serde::Serialize;

use super::detect::DetectedAgent;

/// Outcome of one writer. Every writer is an upsert.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Created,
    Updated,
    Unchanged,
    Skipped { reason: String, fix: Option<String> },
    Error { reason: String, fix: Option<String> },
}

impl Outcome {
    pub fn skipped(reason: impl Into<String>, fix: Option<String>) -> Self {
        Self::Skipped {
            reason: reason.into(),
            fix,
        }
    }

    pub fn error(reason: impl Into<String>, fix: Option<String>) -> Self {
        Self::Error {
            reason: reason.into(),
            fix,
        }
    }

    /// Status string; `created`/`updated` become `would-*` in a dry run.
    pub fn status(&self, dry_run: bool) -> &'static str {
        match (self, dry_run) {
            (Self::Created, false) => "created",
            (Self::Created, true) => "would-create",
            (Self::Updated, false) => "updated",
            (Self::Updated, true) => "would-update",
            (Self::Unchanged, _) => "unchanged",
            (Self::Skipped { .. }, _) => "skipped",
            (Self::Error { .. }, _) => "error",
        }
    }

    pub fn is_error(&self) -> bool {
        matches!(self, Self::Error { .. })
    }

    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Skipped { reason, .. } | Self::Error { reason, .. } => Some(reason),
            _ => None,
        }
    }

    pub fn fix(&self) -> Option<&str> {
        match self {
            Self::Skipped { fix, .. } | Self::Error { fix, .. } => fix.as_deref(),
            _ => None,
        }
    }
}

/// One line of the init report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemResult {
    /// `arete.toml`, `agents-md`, `claude-md`, `gemini-context`, `skills`, `mcp:<id>`.
    pub item: String,
    pub outcome: Outcome,
    /// Path written (relative to the project root when beneath it).
    pub path: Option<String>,
}

impl ItemResult {
    pub fn new(item: impl Into<String>, outcome: Outcome, path: Option<String>) -> Self {
        Self {
            item: item.into(),
            outcome,
            path,
        }
    }

    fn to_json(&self, dry_run: bool) -> serde_json::Value {
        let mut object = serde_json::Map::new();
        object.insert("item".into(), self.item.clone().into());
        object.insert("status".into(), self.outcome.status(dry_run).into());
        if let Some(path) = &self.path {
            object.insert("path".into(), path.clone().into());
        }
        if let Some(reason) = self.outcome.reason() {
            object.insert("reason".into(), reason.into());
        }
        if let Some(fix) = self.outcome.fix() {
            object.insert("fix".into(), fix.into());
        }
        serde_json::Value::Object(object)
    }

    /// Human rendering: `✓ created   AGENTS.md`.
    pub fn render(&self, dry_run: bool, width: usize) -> String {
        let status = self.outcome.status(dry_run);
        let (symbol, status_text) = match &self.outcome {
            Outcome::Created => ("+".green().bold(), status.green()),
            Outcome::Updated => ("~".green().bold(), status.green()),
            Outcome::Unchanged => ("=".dimmed(), status.dimmed()),
            Outcome::Skipped { .. } => ("-".yellow().bold(), status.yellow()),
            Outcome::Error { .. } => ("✗".red().bold(), status.red().bold()),
        };
        let padding = " ".repeat(width.saturating_sub(status.len()));
        let mut line = format!(
            "{symbol} {status_text}{padding} {}",
            self.path.as_deref().unwrap_or(&self.item)
        );
        if let Some(reason) = self.outcome.reason() {
            line.push_str(&format!(" ({reason})"));
        }
        if let Some(fix) = self.outcome.fix() {
            line.push_str(&format!(" → {}", fix.cyan()));
        }
        line
    }
}

/// The `a4 init --json` object (spec WP7).
#[derive(Debug, Clone)]
pub struct InitReport {
    pub dry_run: bool,
    pub detected: Vec<DetectedAgent>,
    pub universal: bool,
    pub selected: Vec<String>,
    pub results: Vec<ItemResult>,
    pub warnings: Vec<String>,
    pub next: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct InitReportJson<'a> {
    schema_version: u32,
    dry_run: bool,
    detected_agents: &'a [DetectedAgent],
    selected_agents: &'a [String],
    results: Vec<serde_json::Value>,
    warnings: &'a [String],
    next: &'a [String],
}

impl InitReport {
    pub fn has_errors(&self) -> bool {
        self.results.iter().any(|result| result.outcome.is_error())
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(InitReportJson {
            schema_version: 1,
            dry_run: self.dry_run,
            detected_agents: &self.detected,
            selected_agents: &self.selected,
            results: self
                .results
                .iter()
                .map(|result| result.to_json(self.dry_run))
                .collect(),
            warnings: &self.warnings,
            next: &self.next,
        })
        .expect("init report serialises")
    }

    /// Human rendering: one line per item, then warnings and `Next:` lines.
    pub fn render(&self) -> String {
        let mut out = String::new();
        let mut detected: Vec<String> = self
            .detected
            .iter()
            .map(|agent| format!("{} ({})", agent.id, agent.how))
            .collect();
        if self.universal {
            detected.push("universal (.agents/)".to_string());
        }
        out.push_str(&format!(
            "{} Detected agents: {}\n",
            "→".blue().bold(),
            if detected.is_empty() {
                "none".dimmed().to_string()
            } else {
                detected.join(", ")
            }
        ));
        if self.dry_run {
            out.push_str(&format!(
                "{} Dry run: nothing is written\n",
                "→".blue().bold()
            ));
        }
        let width = self
            .results
            .iter()
            .map(|result| result.outcome.status(self.dry_run).len())
            .max()
            .unwrap_or(9);
        for result in &self.results {
            out.push_str(&result.render(self.dry_run, width));
            out.push('\n');
        }
        for warning in &self.warnings {
            out.push_str(&format!("{} {warning}\n", "!".yellow().bold()));
        }
        if !self.next.is_empty() {
            out.push_str("Next:\n");
            for next in &self.next {
                out.push_str(&format!("  {}\n", next.cyan()));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_follow_the_spec_vocabulary() {
        assert_eq!(Outcome::Created.status(false), "created");
        assert_eq!(Outcome::Created.status(true), "would-create");
        assert_eq!(Outcome::Updated.status(true), "would-update");
        assert_eq!(Outcome::Unchanged.status(true), "unchanged");
        assert_eq!(Outcome::skipped("x", None).status(true), "skipped");
        assert_eq!(Outcome::error("x", None).status(false), "error");
    }

    #[test]
    fn json_result_only_carries_present_fields() {
        let result = ItemResult::new(
            "skills",
            Outcome::skipped(
                "npx not found",
                Some("npx skills add AreteA4/skills".into()),
            ),
            None,
        );
        let json = result.to_json(false);
        assert_eq!(json["status"], "skipped");
        assert_eq!(json["reason"], "npx not found");
        assert_eq!(json["fix"], "npx skills add AreteA4/skills");
        assert!(json.get("path").is_none());

        let result = ItemResult::new("agents-md", Outcome::Created, Some("AGENTS.md".into()));
        let json = result.to_json(true);
        assert_eq!(json["status"], "would-create");
        assert_eq!(json["path"], "AGENTS.md");
        assert!(json.get("reason").is_none());
    }

    #[test]
    fn report_json_has_schema_version_and_exact_keys() {
        let report = InitReport {
            dry_run: false,
            detected: vec![],
            universal: false,
            selected: vec![],
            results: vec![],
            warnings: vec!["w".into()],
            next: vec!["a4 doctor --json".into()],
        };
        let json = report.to_json();
        let keys: Vec<&String> = json.as_object().unwrap().keys().collect();
        assert_eq!(
            keys,
            [
                "detectedAgents",
                "dryRun",
                "next",
                "results",
                "schemaVersion",
                "selectedAgents",
                "warnings"
            ]
            .iter()
            .map(|k| k.to_string())
            .collect::<Vec<_>>()
            .iter()
            .collect::<Vec<_>>()
        );
        assert_eq!(json["schemaVersion"], 1);
    }
}
