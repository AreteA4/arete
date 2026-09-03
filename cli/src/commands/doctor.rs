//! `a4 doctor`: read-only health check of everything `a4 init` writes plus
//! the environment; `--fix` re-runs the init writers for failing agent checks.
//!
//! Spec: `docs/internal/agent-first-onboarding.md` (WP8).

use std::path::Path;
use std::time::Duration;

use anyhow::Result;
use clap::Args;
use colored::Colorize;
use serde::Serialize;

use crate::agents::agents_md::{self, BlockState};
use crate::agents::detect::{detect, Detection};
use crate::agents::mcp_config::{self, McpState, Scope};
use crate::agents::skills;
use crate::agents::{find_on_path, read_optional, Env};
use crate::api_client::ApiClient;
use crate::selfhost::{latest, platform, receipt::Receipt};
use crate::ui;

use super::init::{self, InitPlan, Selection};

#[derive(Args, Debug, Clone)]
pub struct DoctorArgs {
    /// Re-run the init writers for every failing agents.* check
    #[arg(long)]
    pub fix: bool,
}

const NETWORK_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Ok,
    Warn,
    Fail,
    Info,
}

impl Status {
    fn as_str(self) -> &'static str {
        match self {
            Status::Ok => "ok",
            Status::Warn => "warn",
            Status::Fail => "fail",
            Status::Info => "info",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Check {
    pub id: String,
    pub status: Status,
    pub detail: String,
    pub fix: Option<String>,
}

impl Check {
    fn new(id: &str, status: Status, detail: impl Into<String>, fix: Option<String>) -> Self {
        Self {
            id: id.to_string(),
            status,
            detail: detail.into(),
            fix,
        }
    }
    fn ok(id: &str, detail: impl Into<String>) -> Self {
        Self::new(id, Status::Ok, detail, None)
    }
    fn info(id: &str, detail: impl Into<String>, fix: Option<String>) -> Self {
        Self::new(id, Status::Info, detail, fix)
    }
    fn warn(id: &str, detail: impl Into<String>, fix: Option<String>) -> Self {
        Self::new(id, Status::Warn, detail, fix)
    }
    fn fail(id: &str, detail: impl Into<String>, fix: Option<String>) -> Self {
        Self::new(id, Status::Fail, detail, fix)
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DoctorJson<'a> {
    schema_version: u32,
    status: &'static str,
    checks: &'a [Check],
}

/// Aggregate: `fail` if any check fails, else `warn` if any warns, else `ok`.
pub fn aggregate(checks: &[Check]) -> Status {
    if checks.iter().any(|check| check.status == Status::Fail) {
        Status::Fail
    } else if checks.iter().any(|check| check.status == Status::Warn) {
        Status::Warn
    } else {
        Status::Ok
    }
}

pub fn run(args: DoctorArgs, config_path: &str, json: bool) -> Result<()> {
    let env = Env::from_process(init::project_root(config_path));
    let config = Path::new(config_path);
    let mut checks = run_checks(&env, config);

    if args.fix {
        let fixable: Vec<&Check> = checks
            .iter()
            .filter(|check| {
                check.id.starts_with("agents.")
                    && matches!(check.status, Status::Warn | Status::Fail)
            })
            .collect();
        if fixable.is_empty() {
            eprintln!("{} Nothing to fix.", "→".blue().bold());
        } else {
            let plan = InitPlan {
                dry_run: false,
                force: false,
                name: None,
                global: false,
                skills_ref: None,
                selection: Selection::List(detect(&env).ids()),
                manifest: false,
                agents_md: fixable.iter().any(|check| {
                    matches!(
                        check.id.as_str(),
                        "agents.agents-md" | "agents.claude-md" | "agents.gemini-context"
                    )
                }),
                skills: fixable.iter().any(|check| check.id.ends_with(".skills")),
                mcp: fixable.iter().any(|check| check.id.ends_with(".mcp")),
            };
            let report = init::execute(&env, config, &plan)?;
            if json {
                eprint!("{}", report.render());
            } else {
                println!("{} Fixing {} check(s)…", "→".blue().bold(), fixable.len());
                print!("{}", report.render());
                println!();
            }
            checks = run_checks(&env, config);
        }
    }

    let status = aggregate(&checks);
    if json {
        let output = DoctorJson {
            schema_version: 1,
            status: status.as_str(),
            checks: &checks,
        };
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        print!("{}", render(&checks, status));
    }
    if status == Status::Fail {
        return Err(ui::ExitCode(1).into());
    }
    Ok(())
}

fn render(checks: &[Check], status: Status) -> String {
    let mut out = String::new();
    let width = checks.iter().map(|check| check.id.len()).max().unwrap_or(0);
    for check in checks {
        let (symbol, label) = match check.status {
            Status::Ok => ("✓".green().bold(), "ok  ".green()),
            Status::Warn => ("!".yellow().bold(), "warn".yellow()),
            Status::Fail => ("✗".red().bold(), "fail".red().bold()),
            Status::Info => ("i".blue().bold(), "info".blue()),
        };
        out.push_str(&format!(
            "{symbol} {label} {:<width$}  {}\n",
            check.id,
            check.detail,
            width = width
        ));
    }
    let fixes: Vec<&Check> = checks
        .iter()
        .filter(|check| check.fix.is_some() && check.status != Status::Ok)
        .collect();
    if !fixes.is_empty() {
        out.push_str("\nFixes:\n");
        for check in fixes {
            out.push_str(&format!(
                "  {} {}: {}\n",
                "•".dimmed(),
                check.id,
                check.fix.as_deref().unwrap_or_default().cyan()
            ));
        }
    }
    let summary = match status {
        Status::Ok => "ok".green().bold().to_string(),
        Status::Warn => "warn".yellow().bold().to_string(),
        Status::Fail => "fail".red().bold().to_string(),
        Status::Info => "ok".green().bold().to_string(),
    };
    out.push_str(&format!("\nStatus: {summary}\n"));
    out
}

/// Loaded manifest facts the checks need.
struct ProjectFacts {
    name: String,
    dependencies: usize,
    authoring_stacks: usize,
    lock_fresh: Option<bool>,
}

/// Run every check from the WP8 table, in order.
pub fn run_checks(env: &Env, config_path: &Path) -> Vec<Check> {
    let mut checks = Vec::new();
    let receipt = Receipt::load().ok().flatten();
    let path_env = env.path_env();
    let current = env!("CARGO_PKG_VERSION");

    // cli.version
    checks.push(cli_version(receipt.as_ref(), current));

    // cli.install
    checks.push(cli_install(receipt.as_ref(), &path_env));

    // cli.path
    checks.push(cli_path(receipt.as_ref(), &path_env));

    // project.manifest / project.lock
    let facts = match project_manifest(config_path) {
        Ok(facts) => {
            checks.push(Check::ok(
                "project.manifest",
                format!("{} ({})", config_path.display(), facts.name),
            ));
            Some(facts)
        }
        Err(check) => {
            checks.push(check);
            None
        }
    };
    checks.push(match &facts {
        None => Check::info("project.lock", "not checked (no valid manifest)", None),
        Some(ProjectFacts {
            lock_fresh: Some(true),
            ..
        }) => Check::ok("project.lock", "arete.lock is fresh"),
        Some(ProjectFacts {
            lock_fresh: Some(false),
            ..
        }) => Check::warn(
            "project.lock",
            "arete.lock is stale (manifest changed)",
            Some("a4 install".to_string()),
        ),
        Some(ProjectFacts {
            lock_fresh: None,
            dependencies: 0,
            ..
        }) => Check::ok("project.lock", "no dependencies, no lock needed"),
        Some(_) => Check::warn(
            "project.lock",
            "arete.lock missing",
            Some("a4 install".to_string()),
        ),
    });

    // auth.credentials / auth.whoami
    let api_url = crate::config::get_api_url(None);
    let key = env.var("ARETE_API_KEY").map(str::to_string).or_else(|| {
        ApiClient::load_optional_api_key_for_url(&api_url)
            .ok()
            .flatten()
    });
    match &key {
        Some(_) => checks.push(Check::ok(
            "auth.credentials",
            format!("API key configured for {api_url}"),
        )),
        None => checks.push(Check::info(
            "auth.credentials",
            format!("no API key for {api_url} (not needed for a4 explore)"),
            Some("a4 auth signup".to_string()),
        )),
    }
    checks.push(match &key {
        None => Check::info("auth.whoami", "skipped (no credentials)", None),
        Some(key) => auth_whoami(key),
    });

    // net.api / net.docs-mcp
    checks.push(net_api(&api_url));
    checks.push(net_docs_mcp(env));

    // tools.node / tools.rust
    checks.push(match find_on_path(&path_env, "npx") {
        Some(npx) => Check::ok("tools.node", npx.display().to_string()),
        None => Check::info(
            "tools.node",
            "npx not found (needed only for skills)",
            Some("install Node.js, then: npx skills add AreteA4/skills".to_string()),
        ),
    });
    let authoring = facts.as_ref().map(|f| f.authoring_stacks).unwrap_or(0);
    checks.push(match find_on_path(&path_env, "cargo") {
        Some(cargo) => Check::ok("tools.rust", cargo.display().to_string()),
        None if authoring > 0 => Check::warn(
            "tools.rust",
            format!("cargo not found; arete.toml has {authoring} authoring stack(s)"),
            Some("curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh".to_string()),
        ),
        None => Check::info(
            "tools.rust",
            "cargo not found (needed only for stack authoring)",
            None,
        ),
    });

    // agents.*
    let detection = detect(env);
    checks.extend(agent_checks(env, &detection));
    checks
}

fn cli_version(receipt: Option<&Receipt>, current: &str) -> Check {
    let id = "cli.version";
    let receipt_note = if receipt.is_some() {
        String::new()
    } else {
        " (no install receipt: not installed via a4 self install)".to_string()
    };
    if std::env::var("A4_NO_UPDATE_CHECK").is_ok_and(|v| v == "1") {
        return Check::info(
            id,
            format!("{current}; update check disabled{receipt_note}"),
            None,
        );
    }
    match latest::fetch_latest(NETWORK_TIMEOUT) {
        Ok(pointer) => match latest::is_newer(&pointer.version, current) {
            Some(true) => Check::warn(
                id,
                format!("{current} (latest {}){receipt_note}", pointer.version),
                Some("a4 self update".to_string()),
            ),
            _ if receipt.is_some() => Check::ok(id, format!("{current} (latest)")),
            _ => Check::info(id, format!("{current} (latest){receipt_note}"), None),
        },
        Err(error) => Check::info(
            id,
            format!(
                "{current}; could not check latest ({}){receipt_note}",
                root_cause(&error)
            ),
            None,
        ),
    }
}

fn cli_install(receipt: Option<&Receipt>, path_env: &std::ffi::OsStr) -> Check {
    let id = "cli.install";
    let install_fix = "curl -fsSL https://arete.run/install.sh | sh".to_string();
    let Some(receipt) = receipt else {
        return Check::info(
            id,
            format!(
                "no install receipt; running {}",
                std::env::current_exe()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|_| "unknown binary".to_string())
            ),
            Some(install_fix),
        );
    };
    if !receipt.binary.exists() {
        return Check::warn(
            id,
            format!("receipt binary {} is missing", receipt.binary.display()),
            Some(install_fix),
        );
    }
    let same = match (
        std::env::current_exe().and_then(std::fs::canonicalize),
        std::fs::canonicalize(&receipt.binary),
    ) {
        (Ok(current), Ok(installed)) => current == installed,
        _ => false,
    };
    if !same {
        return Check::warn(
            id,
            format!(
                "running {} but the installed binary is {}",
                std::env::current_exe()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|_| "unknown".to_string()),
                receipt.binary.display()
            ),
            Some(format!(
                "use {} (check PATH order)",
                receipt.binary.display()
            )),
        );
    }
    if let Some(shadow) = platform::shadowing_binary(path_env, &receipt.install_dir) {
        return Check::warn(
            id,
            format!(
                "{} is shadowed by {}",
                receipt.binary.display(),
                shadow.display()
            ),
            Some(format!(
                "remove {} (e.g. cargo uninstall a4-cli or npm rm -g @usearete/a4)",
                shadow.display()
            )),
        );
    }
    Check::ok(id, receipt.binary.display().to_string())
}

fn cli_path(receipt: Option<&Receipt>, path_env: &std::ffi::OsStr) -> Check {
    let id = "cli.path";
    let install_dir = match receipt {
        Some(receipt) => receipt.install_dir.clone(),
        None => match platform::default_install_dir() {
            Ok(dir) => dir,
            Err(error) => return Check::info(id, format!("{error:#}"), None),
        },
    };
    if platform::path_contains(path_env, &install_dir) {
        return Check::ok(id, format!("{} is on PATH", install_dir.display()));
    }
    let export = if cfg!(windows) {
        format!("$env:Path = \"{};$env:Path\"", install_dir.display())
    } else {
        format!("export PATH=\"{}:$PATH\"", install_dir.display())
    };
    if receipt.is_some() {
        Check::warn(
            id,
            format!("{} is not on PATH", install_dir.display()),
            Some(export),
        )
    } else {
        Check::info(
            id,
            format!(
                "{} is not on PATH (no install receipt)",
                install_dir.display()
            ),
            Some(export),
        )
    }
}

fn project_manifest(config_path: &Path) -> std::result::Result<ProjectFacts, Check> {
    let id = "project.manifest";
    if !config_path.exists() {
        return Err(Check::fail(
            id,
            format!("{} not found", config_path.display()),
            Some("a4 init".to_string()),
        ));
    }
    match crate::project::installer::validate_project(config_path, true) {
        Ok((manifest, _plan, lock)) => Ok(ProjectFacts {
            name: manifest.document.project.name.clone(),
            dependencies: manifest.dependencies().count(),
            authoring_stacks: manifest.document.authoring.stacks.len(),
            lock_fresh: lock.map(|lock| lock.is_fresh(&manifest.manifest_hash)),
        }),
        Err(error) => Err(Check::fail(
            id,
            format!("{}: {}", config_path.display(), root_cause(&error)),
            Some("a4 config validate".to_string()),
        )),
    }
}

fn auth_whoami(key: &str) -> Check {
    let id = "auth.whoami";
    let client = match ApiClient::new() {
        Ok(client) => client.with_api_key(key.to_string()),
        Err(error) => return Check::fail(id, format!("{error:#}"), None),
    };
    // Agent keys answer GET /api/agents/me; user keys fall back to a
    // key-scoped listing.
    let result = client
        .agent_me()
        .map(|me| {
            me.get("name")
                .or_else(|| me.get("id"))
                .and_then(|v| v.as_str())
                .map(|name| format!("agent {name}"))
                .unwrap_or_else(|| "agent key accepted".to_string())
        })
        .or_else(|_| client.list_specs().map(|_| "API key accepted".to_string()));
    match result {
        Ok(detail) => Check::ok(id, detail),
        Err(error) => Check::fail(
            id,
            format!("API key rejected: {}", root_cause(&error)),
            Some("a4 auth login --key <a4_ak_...> (or a4 auth signup)".to_string()),
        ),
    }
}

fn http_client() -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .timeout(NETWORK_TIMEOUT)
        .user_agent(format!("a4/{}", env!("CARGO_PKG_VERSION")))
        .build()?)
}

fn net_api(api_url: &str) -> Check {
    let id = "net.api";
    let url = format!("{}/api/registry", api_url.trim_end_matches('/'));
    let response = http_client().and_then(|client| Ok(client.get(&url).send()?));
    match response {
        Ok(response) if response.status().as_u16() == 200 => Check::ok(id, format!("{url} → 200")),
        Ok(response) => Check::fail(
            id,
            format!("{url} → HTTP {}", response.status()),
            Some("check ARETE_API_URL / --api-url, then retry".to_string()),
        ),
        Err(error) => Check::fail(
            id,
            format!("{url}: {}", root_cause(&error)),
            Some("check your network connection, then retry".to_string()),
        ),
    }
}

fn net_docs_mcp(env: &Env) -> Check {
    let id = "net.docs-mcp";
    // `A4_DOCS_MCP_URL` is a test hook (undocumented).
    let url = env
        .var("A4_DOCS_MCP_URL")
        .unwrap_or(mcp_config::DOCS_MCP_URL)
        .to_string();
    let response = http_client().and_then(|client| Ok(client.head(&url).send()?));
    match response {
        Ok(response) if response.status().is_server_error() => Check::warn(
            id,
            format!("{url} → HTTP {}", response.status()),
            Some("the docs MCP server is unavailable; the `arete-docs` server will not answer until it recovers".to_string()),
        ),
        Ok(response) => Check::ok(id, format!("{url} → {}", response.status())),
        Err(error) => Check::warn(
            id,
            format!("{url}: {}", root_cause(&error)),
            Some("check your network connection".to_string()),
        ),
    }
}

fn agent_checks(env: &Env, detection: &Detection) -> Vec<Check> {
    let mut checks = Vec::new();
    let detected: Vec<String> = detection
        .agents
        .iter()
        .map(|agent| format!("{} ({})", agent.id, agent.how))
        .collect();
    let mut detail = if detected.is_empty() {
        "none".to_string()
    } else {
        detected.join(", ")
    };
    if detection.universal {
        detail.push_str(", universal (.agents/)");
    }
    checks.push(Check::info("agents.detected", detail, None));

    let command = mcp_config::command_from_receipt();
    for agent in &detection.agents {
        let id = agent.id.as_str();
        // agents.<id>.mcp
        let check_id = format!("agents.{id}.mcp");
        let (scope, state) = match mcp_config::check(env, id, Scope::Project, &command) {
            McpState::Skipped(_) => (
                Scope::Global,
                mcp_config::check(env, id, Scope::Global, &command),
            ),
            state => (Scope::Project, state),
        };
        checks.push(match (scope, state) {
            (_, McpState::Ok) => Check::ok(&check_id, "arete + arete-docs servers configured"),
            (Scope::Project, McpState::Missing(detail)) => {
                Check::warn(&check_id, detail, Some("a4 doctor --fix".to_string()))
            }
            (Scope::Global, McpState::Missing(detail)) => Check::info(
                &check_id,
                format!("{detail} ({id} reads MCP config from the user scope only)"),
                Some(format!(
                    "a4 init --global --agents {id} --no-manifest --no-agents-md --no-skills"
                )),
            ),
            (_, McpState::Skipped(reason)) => {
                Check::info(&check_id, format!("skipped ({reason})"), None)
            }
            (_, McpState::Error(detail)) => Check::warn(
                &check_id,
                detail,
                Some("fix the file by hand, then: a4 doctor".to_string()),
            ),
        });

        // agents.<id>.skills
        if let Some(name) = skills::skills_agent_name(id) {
            let check_id = format!("agents.{id}.skills");
            let missing = skills::missing_skills(env, id, false);
            let missing_global = skills::missing_skills(env, id, true);
            checks.push(if missing.is_empty() || missing_global.is_empty() {
                Check::ok(&check_id, "arete, arete-consume, arete-build installed")
            } else {
                Check::warn(
                    &check_id,
                    format!("missing skills: {}", missing.join(", ")),
                    Some(format!("npx skills add AreteA4/skills --agent {name}")),
                )
            });
        }
    }

    // agents.agents-md
    let agents_md_content = read_optional(&agents_md::agents_md_path(env))
        .ok()
        .flatten();
    checks.push(
        match agents_md_content.as_deref().map(agents_md::block_state) {
            Some(BlockState::Current) => {
                Check::ok("agents.agents-md", "AGENTS.md has the v1 Arete block")
            }
            Some(BlockState::Stale(token)) => Check::warn(
                "agents.agents-md",
                format!(
                    "AGENTS.md block is stale ({})",
                    if token.is_empty() {
                        "no version".to_string()
                    } else {
                        token
                    }
                ),
                Some("a4 doctor --fix".to_string()),
            ),
            Some(BlockState::Missing) => Check::warn(
                "agents.agents-md",
                "AGENTS.md lacks the Arete block",
                Some("a4 doctor --fix".to_string()),
            ),
            None => Check::warn(
                "agents.agents-md",
                "AGENTS.md missing",
                Some("a4 doctor --fix".to_string()),
            ),
        },
    );

    if detection.contains("claude-code") {
        let content = read_optional(&agents_md::claude_md_path(env))
            .ok()
            .flatten();
        checks.push(match content {
            Some(content) if agents_md::claude_md_ok(&content) => {
                Check::ok("agents.claude-md", "CLAUDE.md imports @AGENTS.md")
            }
            Some(_) => Check::warn(
                "agents.claude-md",
                "CLAUDE.md does not import @AGENTS.md",
                Some("a4 doctor --fix".to_string()),
            ),
            None => Check::warn(
                "agents.claude-md",
                "CLAUDE.md missing",
                Some("a4 doctor --fix".to_string()),
            ),
        });
    }

    if detection.contains("gemini-cli") {
        let content = read_optional(&agents_md::gemini_settings_path(env))
            .ok()
            .flatten();
        checks.push(match content {
            Some(content) if agents_md::gemini_context_ok(&content) => Check::ok(
                "agents.gemini-context",
                ".gemini/settings.json context.fileName includes AGENTS.md",
            ),
            _ => Check::warn(
                "agents.gemini-context",
                ".gemini/settings.json context.fileName lacks AGENTS.md",
                Some("a4 doctor --fix".to_string()),
            ),
        });
    }

    if env.root.join(".codex/config.toml").exists() {
        checks.push(match mcp_config::codex_project_trusted(env) {
            Some(true) => Check::ok("agents.codex-trust", "project trusted in ~/.codex/config.toml"),
            _ => Check::info(
                "agents.codex-trust",
                ".codex/config.toml exists but the project is not trusted in ~/.codex/config.toml (Codex ignores it until trusted)",
                Some("run `codex` in this directory once and accept the trust prompt".to_string()),
            ),
        });
    }

    checks
}

fn root_cause(error: &anyhow::Error) -> String {
    error.root_cause().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggregate_status_prefers_fail_then_warn() {
        let checks = vec![Check::ok("a", ""), Check::info("b", "", None)];
        assert_eq!(aggregate(&checks), Status::Ok);
        let checks = vec![Check::ok("a", ""), Check::warn("b", "", None)];
        assert_eq!(aggregate(&checks), Status::Warn);
        let checks = vec![Check::warn("a", "", None), Check::fail("b", "", None)];
        assert_eq!(aggregate(&checks), Status::Fail);
    }

    #[test]
    fn json_shape_matches_spec() {
        let checks = vec![Check::ok("cli.version", "0.13.0 (latest)")];
        let output = DoctorJson {
            schema_version: 1,
            status: aggregate(&checks).as_str(),
            checks: &checks,
        };
        let json = serde_json::to_value(output).unwrap();
        assert_eq!(json["schemaVersion"], 1);
        assert_eq!(json["status"], "ok");
        assert_eq!(json["checks"][0]["id"], "cli.version");
        assert_eq!(json["checks"][0]["status"], "ok");
        assert_eq!(json["checks"][0]["detail"], "0.13.0 (latest)");
        assert!(json["checks"][0]["fix"].is_null());
        assert_eq!(json["checks"][0].as_object().unwrap().len(), 4);
    }

    #[test]
    fn agent_checks_on_empty_project_warn_about_agents_md_only() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("proj");
        let home = dir.path().join("home");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&home).unwrap();
        let env = Env::new(&root, Some(home), &[]);
        let checks = agent_checks(&env, &detect(&env));
        let ids: Vec<&str> = checks.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, vec!["agents.detected", "agents.agents-md"]);
        assert_eq!(checks[1].status, Status::Warn);
        assert_eq!(checks[1].fix.as_deref(), Some("a4 doctor --fix"));
    }
}
