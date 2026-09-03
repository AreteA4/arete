//! `a4 init`: agent-first project setup.
//!
//! Writes `arete.toml`, the `AGENTS.md` managed block, the `CLAUDE.md`
//! import, the Arete skills (via `npx skills`) and MCP config for every
//! detected (or selected) coding agent. Every write is an upsert and the
//! command is safe to run repeatedly. The only prompt is the agent picker,
//! shown on a TTY when `--agents` is absent (never under `-y`, `--json`
//! still gets clean stdout because the picker lives on stderr).
//!
//! Spec: `docs/internal/agent-first-onboarding.md` (WP7).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::Args;
use dialoguer::{console::Term, theme::ColorfulTheme, MultiSelect};

use crate::agents::detect::{detect, Detection};
use crate::agents::mcp_config::{self, Scope};
use crate::agents::report::InitReport;
use crate::agents::skills::{self, SkillsOptions};
use crate::agents::{
    agents_md, display_path, read_optional, upsert_file, Env, ItemResult, Outcome, AGENT_IDS,
};
use crate::ui;

#[derive(Args, Debug, Clone)]
pub struct InitArgs {
    /// Show what would change without writing anything
    #[arg(long)]
    pub dry_run: bool,

    /// Rewrite the [project] block of an existing arete.toml
    #[arg(long)]
    pub force: bool,

    /// Project name (default: directory basename)
    #[arg(long)]
    pub name: Option<String>,

    /// Agents to configure: comma-separated ids, `all`, or `none` (default: detected)
    #[arg(long, value_name = "LIST")]
    pub agents: Option<String>,

    /// Install skills and MCP config for your user instead of this project
    #[arg(long)]
    pub global: bool,

    /// Skip arete.toml
    #[arg(long)]
    pub no_manifest: bool,

    /// Skip the AGENTS.md / CLAUDE.md block
    #[arg(long)]
    pub no_agents_md: bool,

    /// Skip installing the Arete skills (npx skills)
    #[arg(long)]
    pub no_skills: bool,

    /// Skip MCP server configuration
    #[arg(long)]
    pub no_mcp: bool,

    /// Git ref of AreteA4/skills to install (default: main)
    #[arg(long, value_name = "REF")]
    pub skills_ref: Option<String>,
}

/// `--agents` value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    /// Default: every detected agent (agent-independent set when none).
    Detected,
    All,
    None,
    List(Vec<String>),
}

impl Selection {
    /// Parse `--agents` (`all`, `none`, or a comma-separated id list).
    pub fn parse(value: Option<&str>) -> Result<Self> {
        let Some(value) = value else {
            return Ok(Self::Detected);
        };
        match value.trim() {
            "all" => Ok(Self::All),
            "none" | "" => Ok(Self::None),
            list => {
                let mut ids = Vec::new();
                for id in list.split(',').map(str::trim).filter(|id| !id.is_empty()) {
                    if !crate::agents::is_agent_id(id) {
                        anyhow::bail!(
                            "Unknown agent id `{id}`. Valid ids: {} (or `all`, `none`)",
                            AGENT_IDS.join(", ")
                        );
                    }
                    if !ids.iter().any(|existing| existing == id) {
                        ids.push(id.to_string());
                    }
                }
                Ok(Self::List(ids))
            }
        }
    }
}

/// What one `init` run should do. Built from `InitArgs` by `a4 init` and
/// from failing checks by `a4 doctor --fix`.
#[derive(Debug, Clone)]
pub struct InitPlan {
    pub dry_run: bool,
    pub force: bool,
    pub name: Option<String>,
    pub global: bool,
    pub skills_ref: Option<String>,
    pub selection: Selection,
    pub manifest: bool,
    pub agents_md: bool,
    pub skills: bool,
    pub mcp: bool,
}

impl InitPlan {
    fn from_args(args: &InitArgs) -> Result<Self> {
        Ok(Self {
            dry_run: args.dry_run,
            force: args.force,
            name: args.name.clone(),
            global: args.global,
            skills_ref: args.skills_ref.clone(),
            selection: Selection::parse(args.agents.as_deref())?,
            manifest: !args.no_manifest,
            agents_md: !args.no_agents_md,
            skills: !args.no_skills,
            mcp: !args.no_mcp,
        })
    }
}

/// Project root for a `--config` path: its parent directory.
pub fn project_root(config_path: &str) -> PathBuf {
    Path::new(config_path)
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf()
}

pub fn run(args: InitArgs, config_path: &str, json: bool) -> Result<()> {
    let mut plan = InitPlan::from_args(&args)?;
    let env = Env::from_process(project_root(config_path));
    if plan.selection == Selection::Detected && ui::interactive() {
        plan.selection = pick_agents(&detect(&env))?;
    }
    let report = execute(&env, Path::new(config_path), &plan)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report.to_json())?);
    } else {
        print!("{}", report.render());
    }
    if report.has_errors() {
        return Err(ui::ExitCode(1).into());
    }
    Ok(())
}

/// Pre-checked state for every entry of `AGENT_IDS` in the interactive
/// picker: the detected agents, or `claude-code` alone when nothing was
/// detected (the same agent the non-interactive fallback configures).
pub fn picker_defaults(detected: &[String]) -> Vec<bool> {
    let any_known = AGENT_IDS
        .iter()
        .any(|id| detected.iter().any(|detected| detected == id));
    AGENT_IDS
        .iter()
        .map(|id| {
            if any_known {
                detected.iter().any(|detected| detected == id)
            } else {
                *id == "claude-code"
            }
        })
        .collect()
}

/// Interactive agent picker (TTY only, `--agents` absent). Draws on stderr
/// so `--json` stdout stays machine-readable. An empty selection is
/// `--agents none`.
fn pick_agents(detection: &Detection) -> Result<Selection> {
    let labels: Vec<String> = AGENT_IDS
        .iter()
        .map(
            |id| match detection.agents.iter().find(|agent| agent.id == *id) {
                Some(agent) => format!("{id}  (detected: {})", agent.how),
                None => (*id).to_string(),
            },
        )
        .collect();
    let picked = MultiSelect::with_theme(&ColorfulTheme::default())
        .with_prompt("Configure Arete for these coding agents")
        .items(&labels)
        .defaults(&picker_defaults(&detection.ids()))
        .interact_on(&Term::stderr())
        .context("Failed to read agent selection (pass --agents <list|all|none> or -y)")?;
    let ids: Vec<String> = picked
        .into_iter()
        .map(|index| AGENT_IDS[index].to_string())
        .collect();
    Ok(if ids.is_empty() {
        Selection::None
    } else {
        Selection::List(ids)
    })
}

/// Resolve the agents to configure. Returns the ids and whether this is
/// the agent-independent fallback (nothing detected, no `--agents`).
fn select(detection: &Detection, selection: &Selection) -> (Vec<String>, bool) {
    match selection {
        Selection::Detected if detection.agents.is_empty() => (Vec::new(), true),
        Selection::Detected => (detection.ids(), false),
        Selection::All => (AGENT_IDS.iter().map(|id| id.to_string()).collect(), false),
        Selection::None => (Vec::new(), false),
        Selection::List(ids) => (ids.clone(), false),
    }
}

/// Run the writers in `plan` and build the report.
pub fn execute(env: &Env, config_path: &Path, plan: &InitPlan) -> Result<InitReport> {
    let detection = detect(env);
    let (selected, fallback) = select(&detection, &plan.selection);
    let mut warnings = Vec::new();
    if fallback {
        warnings.push("No coding agent detected; wrote agent-independent files only.".to_string());
    }
    let has = |id: &str| selected.iter().any(|selected| selected == id);
    let mut results = Vec::new();

    if plan.manifest {
        results.push(write_manifest(env, config_path, plan));
    }

    if plan.agents_md {
        results.push(agents_md::write_agents_md(env, plan.dry_run));
        if fallback || has("claude-code") {
            results.push(agents_md::write_claude_md(env, plan.dry_run));
        }
        if has("gemini-cli") {
            results.push(agents_md::write_gemini_context(env, plan.dry_run));
        }
    }

    // Skills run before the MCP writers so a failure here never blocks them.
    let agent_ids: Vec<String> = if fallback {
        vec!["claude-code".to_string()]
    } else {
        selected.clone()
    };
    if plan.skills && (fallback || !selected.is_empty()) {
        let options = SkillsOptions {
            agent_ids: agent_ids.clone(),
            skills_ref: plan.skills_ref.clone(),
            global: plan.global,
            timeout: skills::SKILLS_TIMEOUT,
        };
        results.push(skills::install(env, &options, plan.dry_run));
    }

    if plan.mcp {
        let command = mcp_config::command_from_receipt();
        let scope = if plan.global {
            Scope::Global
        } else {
            Scope::Project
        };
        for id in &agent_ids {
            let (result, warning) = mcp_config::write(env, id, scope, &command, plan.dry_run);
            results.push(result);
            if let Some(warning) = warning {
                if !warnings.contains(&warning) {
                    warnings.push(warning);
                }
            }
        }
    }

    Ok(InitReport {
        dry_run: plan.dry_run,
        detected: detection.agents.clone(),
        universal: detection.universal,
        selected,
        results,
        warnings,
        next: vec![
            "a4 doctor --json".to_string(),
            "a4 explore --json".to_string(),
        ],
    })
}

/// Writer: `arete.toml` (create; `unchanged` if present; `--force`
/// rewrites only `[project]`).
fn write_manifest(env: &Env, config_path: &Path, plan: &InitPlan) -> ItemResult {
    let item = "arete.toml";
    let shown = Some(display_path(env, config_path));
    let existing = match read_optional(config_path) {
        Ok(existing) => existing,
        Err(error) => {
            return ItemResult::new(item, Outcome::error(format!("{error:#}"), None), shown)
        }
    };
    let content = match existing {
        None => super::config::new_manifest_contents(&env.root, plan.name.clone())
            .with_context(|| format!("Failed to build {}", config_path.display())),
        Some(_) if !plan.force => return ItemResult::new(item, Outcome::Unchanged, shown),
        Some(text) => {
            let name = plan
                .name
                .clone()
                .unwrap_or_else(|| super::config::default_project_name(&env.root));
            super::config::rewrite_project_table(&text, &name)
        }
    };
    match content {
        Ok(content) => ItemResult::new(
            item,
            upsert_file(config_path, &content, plan.dry_run),
            shown,
        ),
        Err(error) => ItemResult::new(
            item,
            Outcome::error(format!("{error:#}"), Some("a4 config validate".to_string())),
            shown,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn selection_parses_lists_all_none_and_rejects_unknown() {
        assert_eq!(Selection::parse(None).unwrap(), Selection::Detected);
        assert_eq!(Selection::parse(Some("all")).unwrap(), Selection::All);
        assert_eq!(Selection::parse(Some("none")).unwrap(), Selection::None);
        assert_eq!(
            Selection::parse(Some("cursor, claude-code,cursor")).unwrap(),
            Selection::List(vec!["cursor".into(), "claude-code".into()])
        );
        let error = Selection::parse(Some("emacs")).unwrap_err().to_string();
        assert!(error.contains("Unknown agent id `emacs`"));
        assert!(error.contains("claude-code"));
    }

    #[test]
    fn picker_defaults_precheck_detected_or_claude_code() {
        let index = |id: &str| AGENT_IDS.iter().position(|known| *known == id).unwrap();
        let checked = |defaults: &[bool]| -> Vec<usize> {
            defaults
                .iter()
                .enumerate()
                .filter_map(|(i, on)| on.then_some(i))
                .collect()
        };

        let defaults = picker_defaults(&["gemini-cli".to_string(), "cursor".to_string()]);
        assert_eq!(defaults.len(), AGENT_IDS.len());
        assert_eq!(
            checked(&defaults),
            vec![index("cursor"), index("gemini-cli")],
            "table order, detected only"
        );

        assert_eq!(checked(&picker_defaults(&[])), vec![index("claude-code")]);
        assert_eq!(
            checked(&picker_defaults(&["emacs".to_string()])),
            vec![index("claude-code")],
            "unknown ids fall back like an empty detection"
        );
    }

    #[test]
    fn project_root_is_config_parent() {
        assert_eq!(project_root("arete.toml"), PathBuf::from("."));
        assert_eq!(project_root("sub/dir/arete.toml"), PathBuf::from("sub/dir"));
    }

    fn plan(selection: Selection) -> InitPlan {
        InitPlan {
            dry_run: false,
            force: false,
            name: None,
            global: false,
            skills_ref: None,
            selection,
            manifest: true,
            agents_md: true,
            skills: false,
            mcp: true,
        }
    }

    #[test]
    fn fallback_writes_the_agent_independent_set_with_a_warning() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("proj");
        let home = dir.path().join("home");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&home).unwrap();
        let env = Env::new(&root, Some(home), &[]);
        let config = root.join("arete.toml");
        let report = execute(&env, &config, &plan(Selection::Detected)).unwrap();
        assert!(report.selected.is_empty());
        assert_eq!(
            report.warnings,
            vec!["No coding agent detected; wrote agent-independent files only."]
        );
        let items: Vec<&str> = report.results.iter().map(|r| r.item.as_str()).collect();
        assert_eq!(
            items,
            vec!["arete.toml", "agents-md", "claude-md", "mcp:claude-code"]
        );
        assert!(report.results.iter().all(|r| r.outcome == Outcome::Created));
        assert!(config.exists());
        assert!(root.join(".mcp.json").exists());
        assert_eq!(
            fs::read_to_string(root.join("CLAUDE.md")).unwrap(),
            "@AGENTS.md\n"
        );

        // Second run: claude-code is now detected (CLAUDE.md, .mcp.json) and everything is unchanged.
        let report = execute(&env, &config, &plan(Selection::Detected)).unwrap();
        assert_eq!(report.selected, vec!["claude-code"]);
        assert!(report.warnings.is_empty());
        assert!(
            report
                .results
                .iter()
                .all(|r| r.outcome == Outcome::Unchanged),
            "{:?}",
            report.results
        );
    }

    #[test]
    fn force_rewrites_project_name_only() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("proj");
        fs::create_dir_all(&root).unwrap();
        let env = Env::new(&root, None, &[]);
        let config = root.join("arete.toml");
        let mut p = plan(Selection::None);
        p.name = Some("first".into());
        execute(&env, &config, &p).unwrap();
        let text = fs::read_to_string(&config).unwrap();
        assert!(text.contains("name = \"first\""));
        fs::write(
            &config,
            format!("{text}\n[sdk]\ntargets = [\"typescript\"]\n"),
        )
        .unwrap();

        p.name = Some("second".into());
        let report = execute(&env, &config, &p).unwrap();
        assert_eq!(report.results[0].outcome, Outcome::Unchanged, "no --force");
        p.force = true;
        let report = execute(&env, &config, &p).unwrap();
        assert_eq!(report.results[0].outcome, Outcome::Updated);
        let text = fs::read_to_string(&config).unwrap();
        assert!(text.contains("name = \"second\""));
        assert!(text.contains("[sdk]\ntargets = [\"typescript\"]"));
        // `--agents none` writes no agent items at all.
        let items: Vec<&str> = report.results.iter().map(|r| r.item.as_str()).collect();
        assert_eq!(items, vec!["arete.toml", "agents-md"]);
    }
}
