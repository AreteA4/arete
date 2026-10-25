//! `a4 doctor`: read-only health check of everything `a4 init` writes plus
//! the environment; `--fix` re-runs the init writers for failing agent checks.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use clap::Args;
use colored::Colorize;
use serde::Serialize;

use crate::agents::agents_md::{self, BlockState};
use crate::agents::detect::{detect, Detection, How};
use crate::agents::mcp_config::{self, McpState, Scope};
use crate::agents::skills;
use crate::agents::{find_on_path, read_optional, Env};
#[cfg(test)]
use crate::api_client::ApiHttpError;
use crate::api_client::{
    api_error_details, AccountCapabilities, AccountEntitlements, ApiClient,
    RegistryProgramInstallResponse, RegistryProgramInstallTransport, CAPABILITY_CREATE_DEPLOYMENT,
    CAPABILITY_TRANSACTION_INSPECT, CAPABILITY_TRANSACTION_SEND,
};
use crate::project::manifest::{DependencyKind, DependencySourceV1, InstallTarget};
use crate::project::runtime;
use crate::selfhost::{latest, platform, receipt::Receipt};
use crate::ui;

use super::init::{self, InitPlan, Selection};

#[derive(Args, Debug, Clone)]
pub struct DoctorArgs {
    /// Re-run the init writers for every failing agents.* check
    #[arg(long)]
    pub fix: bool,

    /// Print the overall status first, then only the checks that need
    /// attention (warn, fail, unknown, or info with a fix) with their fixes;
    /// count the ok and purely informational ones
    #[arg(long)]
    pub summary: bool,
}

const NETWORK_TIMEOUT: Duration = Duration::from_secs(2);
/// A live probe mints a session and makes one read: allow it longer.
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Ok,
    Warn,
    Fail,
    Info,
    /// The check could not be decided (e.g. the server does not report the
    /// fact yet). Neutral: it never changes the aggregate status.
    Unknown,
}

impl Status {
    fn as_str(self) -> &'static str {
        match self {
            Status::Ok => "ok",
            Status::Warn => "warn",
            Status::Fail => "fail",
            Status::Info => "info",
            Status::Unknown => "unknown",
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
    fn unknown(id: &str, detail: impl Into<String>) -> Self {
        Self::new(id, Status::Unknown, detail, None)
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DoctorJson<'a> {
    schema_version: u32,
    status: &'static str,
    /// With `--summary`: how many checks were ok and left out of `checks`.
    #[serde(skip_serializing_if = "Option::is_none")]
    ok_count: Option<usize>,
    /// With `--summary`: how many `info` checks with nothing to do were left
    /// out of `checks`.
    #[serde(skip_serializing_if = "Option::is_none")]
    info_count: Option<usize>,
    checks: &'a [Check],
}

/// What `--summary` left out of the listed checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Omitted {
    ok: usize,
    info: usize,
}

/// Whether `--summary` lists a check: everything that is not ok, except an
/// `info` check with no fix, which needs nothing from the reader.
fn needs_attention(check: &Check) -> bool {
    match check.status {
        Status::Ok => false,
        Status::Info => check.fix.is_some(),
        Status::Warn | Status::Fail | Status::Unknown => true,
    }
}

/// The checks `--summary` lists, and what it left out.
fn summarize(checks: &[Check]) -> (Vec<Check>, Omitted) {
    let shown = checks
        .iter()
        .filter(|check| needs_attention(check))
        .cloned()
        .collect();
    let count = |status| {
        checks
            .iter()
            .filter(|check| check.status == status && !needs_attention(check))
            .count()
    };
    let omitted = Omitted {
        ok: count(Status::Ok),
        info: count(Status::Info),
    };
    (shown, omitted)
}

/// Aggregate: `fail` if any check fails, else `warn` if any warns, else
/// `unknown` if any check could not be decided, else `ok`.
pub fn aggregate(checks: &[Check]) -> Status {
    if checks.iter().any(|check| check.status == Status::Fail) {
        Status::Fail
    } else if checks.iter().any(|check| check.status == Status::Warn) {
        Status::Warn
    } else if checks.iter().any(|check| check.status == Status::Unknown) {
        Status::Unknown
    } else {
        Status::Ok
    }
}

/// Fix for a project config's portable `a4` that isn't on PATH. It names a
/// machine setting; the writers leave the shared file alone.
const NOT_ON_PATH_FIX: &str = "put a4 on PATH (open a new shell after installing), then: a4 doctor";

/// Whether `--fix` re-runs the writers for `check`: a failing agent check,
/// except one only a PATH change can clear.
fn is_fixable(check: &Check) -> bool {
    (check.id.starts_with("agents.") || check.id == "project.auth-profile")
        && matches!(check.status, Status::Warn | Status::Fail)
        && check.fix.as_deref() != Some(NOT_ON_PATH_FIX)
}

pub fn run(args: DoctorArgs, config_path: &str, json: bool) -> Result<()> {
    let env = Env::from_process(init::project_root(config_path));
    let config = Path::new(config_path);
    let mut checks = run_checks(&env, config);

    if args.fix {
        let fixable: Vec<&Check> = checks.iter().filter(|check| is_fixable(check)).collect();
        if fixable.is_empty() {
            eprintln!("{} Nothing to fix.", "→".blue().bold());
        } else {
            let plan = InitPlan {
                dry_run: false,
                force: false,
                name: None,
                global: false,
                skills_ref: None,
                // Agents found only in the home directory are not set up in
                // this project, so their checks never fail and --fix leaves
                // them alone.
                selection: Selection::List(project_agent_ids(&detect(&env))),
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
    let (shown, omitted) = if args.summary {
        let (shown, omitted) = summarize(&checks);
        (shown, Some(omitted))
    } else {
        (checks.clone(), None)
    };
    if json {
        let output = DoctorJson {
            schema_version: 1,
            status: status.as_str(),
            ok_count: omitted.map(|omitted| omitted.ok),
            info_count: omitted.map(|omitted| omitted.info),
            checks: &shown,
        };
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        print!("{}", render(&shown, status, omitted));
    }
    if status == Status::Fail {
        return Err(ui::ExitCode(1).into());
    }
    Ok(())
}

/// The checks as text. A summary (`omitted` set) leads with the status and
/// what it left out; the full report ends with the status.
fn render(checks: &[Check], status: Status, omitted: Option<Omitted>) -> String {
    let mut out = String::new();
    if let Some(omitted) = omitted {
        out.push_str(&format!(
            "Status: {}  ({} ok, {} informational hidden; run without --summary for every check)\n",
            status_label(status),
            omitted.ok,
            omitted.info
        ));
        if !checks.is_empty() {
            out.push('\n');
        }
    }
    let width = checks.iter().map(|check| check.id.len()).max().unwrap_or(0);
    for check in checks {
        let (symbol, label) = match check.status {
            Status::Ok => ("✓".green().bold(), "ok  ".green()),
            Status::Warn => ("!".yellow().bold(), "warn".yellow()),
            Status::Fail => ("✗".red().bold(), "fail".red().bold()),
            Status::Info => ("i".blue().bold(), "info".blue()),
            Status::Unknown => ("?".dimmed(), "unkn".dimmed()),
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
    if omitted.is_none() {
        out.push_str(&format!("\nStatus: {}\n", status_label(status)));
    }
    out
}

/// The aggregate status, coloured.
fn status_label(status: Status) -> String {
    match status {
        Status::Ok => "ok".green().bold().to_string(),
        Status::Warn => "warn".yellow().bold().to_string(),
        Status::Fail => "fail".red().bold().to_string(),
        Status::Unknown => "unknown".dimmed().bold().to_string(),
        Status::Info => "ok".green().bold().to_string(),
    }
}

/// Loaded manifest facts the checks need.
struct ProjectFacts {
    name: String,
    root: PathBuf,
    dependencies: usize,
    authoring_stacks: usize,
    lock_fresh: Option<bool>,
    /// Where the project's TypeScript SDKs are generated.
    typescript_outputs: Vec<PathBuf>,
    /// What arete.toml uses that the account must be entitled to.
    usage: ProjectUsage,
}

/// The parts of arete.toml that need an entitlement.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ProjectUsage {
    /// Registry program dependencies, `(alias, registry reference)`: they
    /// read through Program Read.
    registry_programs: Vec<(String, String)>,
    /// Program dependencies of any source.
    programs: usize,
    /// A `[protocol]` table or a protocol dependency.
    protocol: bool,
    /// A `[dev]` table.
    dev: bool,
    /// Authoring stacks, deployed with `a4 up`.
    authoring_stacks: usize,
}

impl ProjectUsage {
    /// Whether the project uses `surface`, and how, for the warning.
    fn uses(&self, surface: &str) -> Option<String> {
        match surface {
            "program_read" if self.programs > 0 => Some(format!(
                "arete.toml has {} program dependenc{}",
                self.programs,
                if self.programs == 1 { "y" } else { "ies" }
            )),
            "protocol_prepare" | "protocol_submit" | "create_protocol" if self.protocol => {
                Some("arete.toml has a protocol".to_string())
            }
            "dev_environments" if self.dev => Some("arete.toml has a [dev] section".to_string()),
            "deploy_stack" if self.authoring_stacks > 0 => Some(format!(
                "arete.toml has {} authoring stack(s) for `a4 up`",
                self.authoring_stacks
            )),
            _ => None,
        }
    }
}

/// Run every check from the WP8 table, in order.
pub fn run_checks(env: &Env, config_path: &Path) -> Vec<Check> {
    let mut checks = Vec::new();
    let detection = detect(env);
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
    checks.push(project_auth_profile(config_path, &detection));

    // sdk.runtime
    checks.push(match &facts {
        None => Check::info("sdk.runtime", "not checked (no valid manifest)", None),
        Some(facts) => sdk_runtime(&facts.root, &facts.typescript_outputs, current),
    });

    // auth.credentials / auth.whoami
    let api_url = crate::config::get_api_url(None);
    let selected_profile = ApiClient::selected_profile().ok().flatten();
    let key = if selected_profile.is_some() {
        ApiClient::load_optional_api_key_for_url(&api_url)
            .ok()
            .flatten()
    } else {
        env.var("ARETE_API_KEY").map(str::to_string).or_else(|| {
            ApiClient::load_optional_api_key_for_url(&api_url)
                .ok()
                .flatten()
        })
    };
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

    // account.* from the server's entitlements (older APIs: account.transactions
    // and account.deploy from the capability list), then the live probes.
    let authoring = facts.as_ref().map(|f| f.authoring_stacks).unwrap_or(0);
    let usage = facts
        .as_ref()
        .map(|facts| facts.usage.clone())
        .unwrap_or_default();
    match &key {
        None => checks.extend([
            Check::info("account.entitlements", "skipped (no credentials)", None),
            Check::info("probe.program-read", "skipped (no credentials)", None),
            Check::info("probe.chain-read", "skipped (no credentials)", None),
        ]),
        Some(key) => {
            let entitlements =
                match keyed_client(key).and_then(|client| client.account_entitlements()) {
                    Ok(entitlements) => {
                        checks.extend(entitlement_checks(&entitlements, &usage));
                        Some(entitlements)
                    }
                    Err(error) if api_error_details(&error).is_some_and(|e| e.status == 404) => {
                        checks.extend(account_checks(account_capabilities(key), authoring));
                        None
                    }
                    Err(error) => {
                        checks.push(Check::unknown(
                            "account.entitlements",
                            format!("unknown: {}", account_unavailable_reason(&error)),
                        ));
                        None
                    }
                };
            checks.extend(probe_checks(key, &usage, entitlements.as_ref()));
        }
    }

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
    checks.extend(agent_checks(env, &detection));
    checks
}

fn project_auth_profile(config_path: &Path, detection: &Detection) -> Check {
    let id = "project.auth-profile";
    match crate::config::get_project_auth_profile(&config_path.display().to_string()) {
        Ok(Some(profile)) => Check::ok(id, format!("default profile is {profile}")),
        Ok(None) if detection.agents.is_empty() && !detection.universal => {
            Check::info(id, "not configured (no coding agent detected)", None)
        }
        Ok(None) => Check::warn(
            id,
            format!("{} is missing", crate::config::PROJECT_AUTH_RELATIVE_PATH),
            Some("a4 doctor --fix".to_string()),
        ),
        Err(error) => Check::fail(
            id,
            format!("{error:#}"),
            Some("a4 doctor --fix".to_string()),
        ),
    }
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
        Ok((manifest, plan, lock)) => Ok(ProjectFacts {
            usage: ProjectUsage {
                registry_programs: manifest
                    .dependencies()
                    .filter(|(kind, _, _)| *kind == DependencyKind::Program)
                    .filter_map(|(_, alias, dependency)| match &dependency.source {
                        DependencySourceV1::Registry(source) => {
                            Some((alias.clone(), source.registry.clone()))
                        }
                        _ => None,
                    })
                    .collect(),
                programs: manifest
                    .dependencies()
                    .filter(|(kind, _, _)| *kind == DependencyKind::Program)
                    .count(),
                protocol: manifest.document.protocol.is_some()
                    || manifest
                        .dependencies()
                        .any(|(kind, _, _)| kind == DependencyKind::Protocol),
                dev: manifest.document.dev.is_some(),
                authoring_stacks: manifest.document.authoring.stacks.len(),
            },
            name: manifest.document.project.name.clone(),
            root: manifest.root.clone(),
            dependencies: manifest.dependencies().count(),
            authoring_stacks: manifest.document.authoring.stacks.len(),
            lock_fresh: lock.map(|lock| lock.is_fresh(&manifest.manifest_hash)),
            typescript_outputs: plan
                .outputs
                .into_iter()
                .filter(|output| output.target == InstallTarget::TypeScript)
                .map(|output| output.path)
                .collect(),
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
        .map(|me| format!("agent {} ({})", me.slug, me.claim_state))
        .or_else(|_| client.list_specs().map(|_| "API key accepted".to_string()));
    match result {
        Ok(detail) => Check::ok(id, detail),
        Err(error) => Check::fail(
            id,
            format!("API key rejected: {}", root_cause(&error)),
            Some(
                "a4 auth signup (agent) or a4 auth login --profile human --key <a4_sk_...>"
                    .to_string(),
            ),
        ),
    }
}

/// The caller's account capabilities from `GET /api/auth/me`, or a neutral
/// reason they could not be read. Never an error for the doctor as a whole:
/// servers without the endpoint answer 404.
fn account_capabilities(key: &str) -> std::result::Result<AccountCapabilities, String> {
    let client = ApiClient::new()
        .map_err(|error| format!("{error:#}"))?
        .with_api_key(key.to_string());
    client
        .account_capabilities()
        .map_err(|error| account_unavailable_reason(&error))
}

fn keyed_client(key: &str) -> Result<ApiClient> {
    Ok(ApiClient::new()?.with_api_key(key.to_string()))
}

/// The check id for a surface: `account.transaction-send`.
fn surface_check_id(surface: &str) -> String {
    format!("account.{}", surface.replace('_', "-"))
}

/// What a surface lets the account do, for the check's detail.
fn surface_label(surface: &str) -> String {
    match surface {
        "program_read" => "program reads".to_string(),
        "chain_read" => "chain reads".to_string(),
        "transaction_inspect" => "raw transaction inspect".to_string(),
        "transaction_send" => "raw transaction send".to_string(),
        "protocol_prepare" => "protocol prepare".to_string(),
        "protocol_submit" => "protocol submit".to_string(),
        "deploy_stack" => "stack deploys".to_string(),
        "create_protocol" => "creating protocols".to_string(),
        "dev_environments" => "dev environments".to_string(),
        other => other.replace('_', " "),
    }
}

/// `dev tier free: 1 environment allowed, 0 in use`, from the quotas the
/// server attached to `dev_environments`.
fn dev_tier_detail(
    entitlements: &AccountEntitlements,
    quotas: Option<&serde_json::Value>,
) -> Option<String> {
    let tier = entitlements.dev_tier.as_deref().or_else(|| {
        quotas
            .and_then(|quotas| quotas.get("tier"))
            .and_then(serde_json::Value::as_str)
    })?;
    let max = quotas
        .and_then(|quotas| quotas.pointer("/limits/maxEnvironments"))
        .and_then(serde_json::Value::as_u64);
    let used = quotas
        .and_then(|quotas| quotas.pointer("/usage/environments"))
        .and_then(serde_json::Value::as_u64);
    Some(match (max, used) {
        (Some(max), Some(used)) => format!(
            "dev tier {tier}: {max} environment{} allowed, {used} in use",
            if max == 1 { "" } else { "s" }
        ),
        (Some(max), None) => format!(
            "dev tier {tier}: {max} environment{} allowed",
            if max == 1 { "" } else { "s" }
        ),
        _ => format!("dev tier {tier}"),
    })
}

/// One check per surface the server reports, after an `account.plan` line.
/// A surface the account can't use is `info` with the server's reason and
/// unlock, or `warn` when arete.toml uses it.
fn entitlement_checks(entitlements: &AccountEntitlements, usage: &ProjectUsage) -> Vec<Check> {
    let mut checks = Vec::with_capacity(entitlements.surfaces.len() + 1);
    let mut plan = match (&entitlements.plan, &entitlements.account_kind) {
        (Some(plan), Some(kind)) => format!("plan {plan} ({kind} account)"),
        (Some(plan), None) => format!("plan {plan}"),
        (None, Some(kind)) => format!("{kind} account"),
        (None, None) => "account".to_string(),
    };
    if let Some(tier) = &entitlements.dev_tier {
        plan.push_str(&format!("; dev tier {tier}"));
    }
    checks.push(Check::ok("account.plan", plan));
    for surface in &entitlements.surfaces {
        let id = surface_check_id(&surface.surface);
        let label = surface_label(&surface.surface);
        if surface.allowed {
            let detail = if surface.surface == "dev_environments" {
                dev_tier_detail(entitlements, surface.quotas.as_ref())
                    .unwrap_or_else(|| format!("can use {label}"))
            } else {
                format!("can use {label}")
            };
            checks.push(Check::ok(&id, detail));
            continue;
        }
        let reason = surface
            .reason
            .clone()
            .unwrap_or_else(|| format!("your account can't use {label}"));
        let fix = surface.unlock.clone();
        checks.push(match usage.uses(&surface.surface) {
            Some(used) => Check::warn(&id, format!("{reason}; {used}"), fix),
            None => Check::info(&id, reason, fix),
        });
    }
    checks
}

/// What a probe reads: one program's Program Read metadata and one chain
/// account through the gateway, both from the program's install descriptor.
#[derive(Debug, Clone, PartialEq)]
struct ProbeTargets {
    program: String,
    program_read: Option<ProbeRequest>,
    chain_read: Option<ProbeRequest>,
}

#[derive(Debug, Clone, PartialEq)]
struct ProbeRequest {
    session_endpoint: String,
    session_request: serde_json::Value,
    /// `GET` without a body, `POST` with one.
    url: String,
    body: Option<serde_json::Value>,
}

impl ProbeTargets {
    fn from_install(alias: &str, install: &RegistryProgramInstallResponse) -> Self {
        let RegistryProgramInstallTransport::HostedBinding { binding } = &install.transport;
        let program_read = binding
            .auth
            .get("sessionEndpoint")
            .and_then(serde_json::Value::as_str)
            .map(|session_endpoint| ProbeRequest {
                session_endpoint: session_endpoint.to_string(),
                session_request: serde_json::json!({
                    "scopes": ["read"],
                    "targetKind": "program-read-binding",
                    "targetId": binding.program_read_binding_id,
                    "programReleaseHash": install.release.program_release_hash,
                }),
                url: format!(
                    "{}/v1/releases/{}",
                    binding.endpoint.trim_end_matches('/'),
                    install.release.program_release_hash
                ),
                body: None,
            });
        let chain_read = install.chain_binding.as_ref().map(|chain| ProbeRequest {
            session_endpoint: chain.auth.session_endpoint.clone(),
            session_request: serde_json::json!({
                "scopes": ["read"],
                "targetKind": chain.auth.target_kind,
                "targetId": chain.auth.target_id,
            }),
            url: format!("{}/chain/v1/account", chain.endpoint.trim_end_matches('/')),
            body: Some(serde_json::json!({
                "address": install.definition.program_id,
                "options": {"commitment": "confirmed"},
            })),
        });
        Self {
            program: alias.to_string(),
            program_read,
            chain_read,
        }
    }
}

/// The two live probes. They read through the first registry program in
/// arete.toml, and are skipped when there is none or the account can't use
/// the surface.
fn probe_checks(
    key: &str,
    usage: &ProjectUsage,
    entitlements: Option<&AccountEntitlements>,
) -> Vec<Check> {
    let ids = ["probe.program-read", "probe.chain-read"];
    let Some((alias, reference)) = usage.registry_programs.first() else {
        return ids
            .iter()
            .map(|id| {
                Check::info(
                    id,
                    "skipped (arete.toml has no registry program to read)",
                    None,
                )
            })
            .collect();
    };
    let client = match keyed_client(key) {
        Ok(client) => client,
        Err(error) => {
            return ids
                .iter()
                .map(|id| Check::unknown(id, format!("unknown: {}", root_cause(&error))))
                .collect()
        }
    };
    let targets = match client.get_registry_program_install(reference, None) {
        Ok(install) => ProbeTargets::from_install(alias, &install),
        Err(error) => {
            return ids
                .iter()
                .map(|id| {
                    Check::unknown(
                        id,
                        format!("unknown: could not resolve {alias}: {}", root_cause(&error)),
                    )
                })
                .collect()
        }
    };
    let denied = |surface: &str| {
        entitlements
            .and_then(|entitlements| entitlements.surface(surface))
            .is_some_and(|surface| !surface.allowed)
    };
    vec![
        if denied("program_read") {
            Check::info(ids[0], "skipped (program reads are not in your plan)", None)
        } else {
            run_probe(
                &client,
                ids[0],
                "Program Read",
                &targets.program,
                targets.program_read.as_ref(),
            )
        },
        if denied("chain_read") {
            Check::info(ids[1], "skipped (chain reads are not in your plan)", None)
        } else {
            run_probe(
                &client,
                ids[1],
                "chain read",
                &targets.program,
                targets.chain_read.as_ref(),
            )
        },
    ]
}

/// Mint a session for the probe's target and make its one read. Reports the
/// status and code it got; a retryable failure is named as one.
fn run_probe(
    client: &ApiClient,
    id: &str,
    what: &str,
    program: &str,
    request: Option<&ProbeRequest>,
) -> Check {
    let Some(request) = request else {
        return Check::info(
            id,
            format!("skipped ({program} has no hosted {what} binding)"),
            None,
        );
    };
    let token =
        match client.mint_target_session(&request.session_endpoint, &request.session_request) {
            Ok(token) => token,
            Err(error) => {
                return match api_error_details(&error) {
                    Some(api) => probe_failure(
                        id,
                        &format!("{what} session for {program}"),
                        api.status,
                        api.code,
                        false,
                    ),
                    None => Check::warn(
                        id,
                        format!("{what} session for {program}: {}", root_cause(&error)),
                        Some("check your network connection, then: a4 doctor".to_string()),
                    ),
                }
            }
        };
    let http = match reqwest::blocking::Client::builder()
        .timeout(PROBE_TIMEOUT)
        .user_agent(format!("a4/{}", env!("CARGO_PKG_VERSION")))
        .build()
    {
        Ok(http) => http,
        Err(error) => return Check::unknown(id, format!("unknown: {error}")),
    };
    let builder = match &request.body {
        Some(body) => http.post(&request.url).json(body),
        None => http.get(&request.url),
    };
    match builder.bearer_auth(token).send() {
        Ok(response) if response.status().is_success() => Check::ok(
            id,
            format!("{what} for {program} → {}", response.status().as_u16()),
        ),
        Ok(response) => {
            let status = response.status().as_u16();
            let body: serde_json::Value = response.json().unwrap_or_default();
            let code = body
                .pointer("/error/code")
                .or_else(|| body.get("code"))
                .and_then(serde_json::Value::as_str);
            let retryable = body
                .get("retryable")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            probe_failure(
                id,
                &format!("{what} for {program}"),
                status,
                code,
                retryable,
            )
        }
        Err(error) => Check::warn(
            id,
            format!(
                "{what} for {program}: {}",
                root_cause(&anyhow::Error::from(error))
            ),
            Some("check your network connection, then: a4 doctor".to_string()),
        ),
    }
}

/// A probe that got an error status. 429, 5xx and errors the server marks
/// retryable are Arete-side or transient: they say so and never read as a
/// plan problem.
fn probe_failure(id: &str, what: &str, status: u16, code: Option<&str>, retryable: bool) -> Check {
    let code = code.map(|code| format!(" {code}")).unwrap_or_default();
    if retryable || status == 429 || status >= 500 {
        Check::warn(
            id,
            format!("{what} → HTTP {status}{code} (retryable: an Arete-side or transient failure, not your plan)"),
            Some("retry in a few minutes; if it persists, report the code above to Arete".to_string()),
        )
    } else {
        Check::warn(
            id,
            format!("{what} → HTTP {status}{code}"),
            Some("see the account.* checks above for what your plan includes".to_string()),
        )
    }
}

/// What an API without the account endpoints answers.
const NOT_REPORTED: &str = "this API does not report account capabilities yet";

/// A neutral, user-facing reason account capabilities could not be read.
pub(crate) fn account_unavailable_reason(error: &anyhow::Error) -> String {
    match api_error_details(error).map(|error| error.status) {
        Some(404) => NOT_REPORTED.to_string(),
        Some(401 | 403) => "the API key was not accepted for account details".to_string(),
        _ => format!(
            "could not read account capabilities ({})",
            root_cause(error)
        ),
    }
}

/// `account.transactions` (needs `transaction_inspect` and
/// `transaction_send`) and `account.deploy` (needs `create_deployment`).
/// Missing capabilities are `info`, like other optional tooling, except that
/// deploy is `warn` when arete.toml has authoring stacks for `a4 up`.
fn account_checks(
    account: std::result::Result<AccountCapabilities, String>,
    authoring_stacks: usize,
) -> Vec<Check> {
    let account = match account {
        Ok(account) => account,
        // An API that doesn't report capabilities has answered: that is
        // information, not an undecided check.
        Err(reason) if reason == NOT_REPORTED => {
            return vec![
                Check::info("account.transactions", reason.clone(), None),
                Check::info("account.deploy", reason, None),
            ]
        }
        Err(reason) => {
            return vec![
                Check::unknown("account.transactions", format!("unknown: {reason}")),
                Check::unknown("account.deploy", format!("unknown: {reason}")),
            ]
        }
    };
    let described = |detail: String| match (&account.account_kind, &account.plan) {
        (Some(kind), Some(plan)) => format!("{detail} ({kind} account, plan {plan})"),
        (Some(kind), None) => format!("{detail} ({kind} account)"),
        (None, Some(plan)) => format!("{detail} (plan {plan})"),
        (None, None) => detail,
    };

    let missing = account.missing(&[CAPABILITY_TRANSACTION_INSPECT, CAPABILITY_TRANSACTION_SEND]);
    let transactions = if missing.is_empty() {
        Check::ok(
            "account.transactions",
            described("can inspect and send transactions".to_string()),
        )
    } else {
        Check::info(
            "account.transactions",
            described(format!(
                "your account doesn't have transaction access yet (missing {}); reads and subscriptions are unaffected",
                missing.join(", ")
            )),
            None,
        )
    };

    let deploy = if account.missing(&[CAPABILITY_CREATE_DEPLOYMENT]).is_empty() {
        Check::ok("account.deploy", described("can deploy stacks".to_string()))
    } else {
        let detail = described(format!(
            "your account can't deploy stacks yet (missing {CAPABILITY_CREATE_DEPLOYMENT})"
        ));
        if authoring_stacks > 0 {
            Check::warn(
                "account.deploy",
                format!(
                    "{detail}; arete.toml has {authoring_stacks} authoring stack(s) for `a4 up`"
                ),
                None,
            )
        } else {
            Check::info("account.deploy", detail, None)
        }
    };
    vec![transactions, deploy]
}

/// `sdk.runtime`: the TypeScript runtime installed for the project's
/// TypeScript outputs. Every problem is a warning except an extension whose
/// `extensionApi` differs from the installed `@usearete/sdk`'s, which fails:
/// that code cannot run. Not applicable without TypeScript outputs or
/// `node_modules`.
fn sdk_runtime(root: &Path, typescript_outputs: &[PathBuf], cli_version: &str) -> Check {
    let id = "sdk.runtime";
    if typescript_outputs.is_empty() {
        return Check::info(id, "not applicable (no TypeScript outputs)", None);
    }
    let shown = |path: &Path| {
        path.strip_prefix(root)
            .unwrap_or(path)
            .display()
            .to_string()
    };
    let has_node_modules = |output: &PathBuf| {
        output
            .ancestors()
            .any(|ancestor| ancestor.join("node_modules").is_dir())
    };
    let outputs: Vec<&PathBuf> = typescript_outputs
        .iter()
        .filter(|output| has_node_modules(output))
        .collect();
    if outputs.is_empty() {
        return Check::info(id, "not applicable (no node_modules)", None);
    }

    // The code was generated for the runtime release of the CLI that
    // generated it; fixes target that release.
    let generators: BTreeSet<String> = outputs
        .iter()
        .filter_map(|output| generator_version(output))
        .collect();
    let expected = match generators.iter().collect::<Vec<_>>().as_slice() {
        [only] => only.as_str(),
        _ => cli_version,
    };

    let mut problems = Vec::new();
    let mut failed = false;
    let mut installed_summary = BTreeSet::new();
    let mut fix_packages = BTreeSet::new();
    for output in &outputs {
        fix_packages.extend(runtime::app_dependencies(output));
        let Some(sdk_dir) = runtime::resolve_npm_package(output, runtime::TYPESCRIPT_SDK) else {
            problems.push(format!(
                "{} is not installed for {}",
                runtime::TYPESCRIPT_SDK,
                shown(output)
            ));
            continue;
        };
        let installed: Vec<(&str, runtime::InstalledRuntime)> = runtime::TYPESCRIPT_LOCKSTEP
            .iter()
            .filter_map(|package| {
                let dir = runtime::resolve_npm_package(output, package)?;
                Some((*package, runtime::read_installed_npm_package(&dir)?))
            })
            .collect();
        for (package, _) in &installed {
            if *package == runtime::TYPESCRIPT_REACT {
                fix_packages.insert("react".to_string());
            } else if *package == runtime::TYPESCRIPT_ADAPTER_WEB3JS {
                fix_packages.insert("@solana/web3.js".to_string());
            } else if *package == runtime::TYPESCRIPT_ADAPTER_KIT {
                fix_packages.insert("@solana/kit".to_string());
            }
        }
        let Some(sdk) = installed
            .iter()
            .find(|(package, _)| *package == runtime::TYPESCRIPT_SDK)
            .map(|(_, sdk)| sdk.clone())
        else {
            problems.push(format!(
                "{} at {} has no readable package.json",
                runtime::TYPESCRIPT_SDK,
                shown(&sdk_dir)
            ));
            continue;
        };
        for (package, found) in &installed {
            let api = match (package, found.extension_api) {
                (&runtime::TYPESCRIPT_SDK, Some(api)) => format!(" (extension API {api})"),
                _ => String::new(),
            };
            installed_summary.insert(format!("{package} {}{api}", found.version));
        }

        // One version across the lockstep packages.
        let versions: BTreeSet<&str> = installed
            .iter()
            .map(|(_, found)| found.version.as_str())
            .collect();
        if versions.len() > 1 {
            problems.push(format!(
                "runtime packages differ in version ({})",
                installed
                    .iter()
                    .map(|(package, found)| format!("{package} {}", found.version))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }

        // One copy of @usearete/sdk.
        let copies = sdk_copies(&sdk_dir, &installed);
        if copies.len() > 1 {
            problems.push(format!(
                "{} copies of {} ({})",
                copies.len(),
                runtime::TYPESCRIPT_SDK,
                copies
                    .iter()
                    .map(|(path, version)| format!("{} {version}", shown(path)))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }

        // Generated extensions run on this SDK's extension API.
        for (manifest, declared) in declared_extension_apis(output) {
            match sdk.extension_api {
                Some(provided) if provided == declared => {}
                Some(provided) => {
                    failed = true;
                    problems.push(format!(
                        "{} requires extension API {declared}, but {} {} provides {provided}",
                        shown(&manifest),
                        runtime::TYPESCRIPT_SDK,
                        sdk.version
                    ));
                }
                None => problems.push(format!(
                    "{} requires extension API {declared}, but {} {} predates extension API versioning",
                    shown(&manifest),
                    runtime::TYPESCRIPT_SDK,
                    sdk.version
                )),
            }
        }

        // The runtime release the code was generated for.
        if let Some(generator) = generator_version(output) {
            if generator != sdk.version {
                problems.push(format!(
                    "{} was generated by a4 {generator}; the installed runtime is {}",
                    shown(output),
                    sdk.version
                ));
            }
        }
    }

    if problems.is_empty() {
        return Check::ok(
            id,
            installed_summary.into_iter().collect::<Vec<_>>().join(", "),
        );
    }
    // Outputs sharing one node_modules report its problems once.
    let mut seen = BTreeSet::new();
    problems.retain(|problem| seen.insert(problem.clone()));
    let fix =
        runtime::npm_install_command(&runtime::typescript_runtime_set_at(&fix_packages, expected));
    let fix = if expected == cli_version {
        fix
    } else {
        format!(
            "{fix} (the release the SDKs were generated for), or run `a4 install` to regenerate them for {cli_version}"
        )
    };
    let detail = problems.join("; ");
    if failed {
        Check::fail(id, detail, Some(fix))
    } else {
        Check::warn(id, detail, Some(fix))
    }
}

/// The version of the a4 CLI that generated a TypeScript output.
fn generator_version(output: &Path) -> Option<String> {
    let provenance: serde_json::Value =
        serde_json::from_slice(&std::fs::read(output.join("sdk-provenance.json")).ok()?).ok()?;
    (provenance.pointer("/generator/name")?.as_str()? == env!("CARGO_PKG_NAME"))
        .then(|| {
            provenance
                .pointer("/generator/version")?
                .as_str()
                .map(str::to_string)
        })
        .flatten()
}

/// Every copy of `@usearete/sdk` installed under the node_modules that holds
/// `sdk_dir`: the resolved one, copies nested under the other runtime
/// packages, and pnpm's per-version store entries.
fn sdk_copies(
    sdk_dir: &Path,
    installed: &[(&str, runtime::InstalledRuntime)],
) -> Vec<(PathBuf, String)> {
    let mut copies = BTreeMap::new();
    let mut record = |dir: PathBuf| {
        if let Some(found) = runtime::read_installed_npm_package(&dir) {
            let canonical = std::fs::canonicalize(&dir).unwrap_or(dir.clone());
            copies.entry(canonical).or_insert((dir, found.version));
        }
    };
    record(sdk_dir.to_path_buf());
    for (package, found) in installed {
        if let Some(package_dir) = found.location.parent() {
            if *package != runtime::TYPESCRIPT_SDK {
                record(
                    package_dir
                        .join("node_modules")
                        .join(runtime::TYPESCRIPT_SDK),
                );
            }
        }
    }
    // node_modules/@usearete/sdk -> node_modules
    if let Some(node_modules) = sdk_dir.parent().and_then(Path::parent) {
        if let Ok(entries) = std::fs::read_dir(node_modules.join(".pnpm")) {
            for entry in entries.filter_map(|entry| entry.ok()) {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with("@usearete+sdk@") {
                    record(
                        entry
                            .path()
                            .join("node_modules")
                            .join(runtime::TYPESCRIPT_SDK),
                    );
                }
            }
        }
    }
    copies.into_values().collect()
}

/// The `extensionApi` each generated extension manifest in a TypeScript
/// output declares (the stack's own and each program SDK module's).
fn declared_extension_apis(output: &Path) -> Vec<(PathBuf, u32)> {
    let mut manifests = vec![output.join("extensions.json")];
    if let Ok(programs) = std::fs::read_dir(output.join("programs")) {
        let mut nested = programs
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path().join("extensions.json"))
            .collect::<Vec<_>>();
        nested.sort();
        manifests.extend(nested);
    }
    manifests
        .into_iter()
        .filter_map(|manifest| {
            let value: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&manifest).ok()?).ok()?;
            let declared = u32::try_from(value.get("extensionApi")?.as_u64()?).ok()?;
            Some((manifest, declared))
        })
        .collect()
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

/// Detected agents with a project or environment signal: the ones this
/// project uses, as `a4 init -y` selects them.
fn project_agent_ids(detection: &Detection) -> Vec<String> {
    detection.default_ids()
}

/// A context file `agent` reads is missing or lacks AGENTS.md. For an agent
/// found only in the home directory this is information: `--fix` leaves
/// such agents alone, so a warning would outlive the fix.
fn context_check(detection: &Detection, agent: &str, id: &str, detail: &str) -> Check {
    let home_only = detection
        .agents
        .iter()
        .any(|detected| detected.id == agent && detected.how == How::Home);
    if home_only {
        Check::info(
            id,
            format!("{detail} ({agent} is installed but not set up in this project)"),
            Some(format!(
                "a4 init --agents {agent} --no-manifest --no-skills --no-mcp"
            )),
        )
    } else {
        Check::warn(id, detail, Some("a4 doctor --fix".to_string()))
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
        // Installed on this machine, but with no sign of use in this project.
        let home_only = agent.how == How::Home;
        let not_set_up = format!("{id} is installed but not set up in this project");
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
            (Scope::Project, McpState::Missing(detail)) if home_only => Check::info(
                &check_id,
                format!("{detail} ({not_set_up})"),
                Some(format!(
                    "a4 init --agents {id} --no-manifest --no-agents-md --no-skills"
                )),
            ),
            (Scope::Project, McpState::Missing(detail)) => {
                Check::warn(&check_id, detail, Some("a4 doctor --fix".to_string()))
            }
            (_, McpState::NotOnPath(detail)) => {
                Check::warn(&check_id, detail, Some(NOT_ON_PATH_FIX.to_string()))
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
                Check::ok(
                    &check_id,
                    format!("{} installed", skills::SKILL_NAMES.join(", ")),
                )
            } else if home_only {
                Check::info(
                    &check_id,
                    format!("missing skills: {} ({not_set_up})", missing.join(", ")),
                    Some(format!("npx skills add AreteA4/skills --agent {name}")),
                )
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
                Check::ok("agents.agents-md", "AGENTS.md has the v2 Arete block")
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
            Some(BlockState::Malformed(reason)) => Check::warn(
                "agents.agents-md",
                format!("AGENTS.md Arete block is malformed ({reason})"),
                Some(agents_md::MALFORMED_FIX.to_string()),
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
            Some(_) => context_check(
                detection,
                "claude-code",
                "agents.claude-md",
                "CLAUDE.md does not import @AGENTS.md",
            ),
            None => context_check(
                detection,
                "claude-code",
                "agents.claude-md",
                "CLAUDE.md missing",
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
            _ => context_check(
                detection,
                "gemini-cli",
                "agents.gemini-context",
                ".gemini/settings.json context.fileName lacks AGENTS.md",
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
    fn fix_leaves_a_portable_a4_that_is_not_on_path_alone() {
        let not_on_path = Check::warn(
            "agents.claude-code.mcp",
            ".mcp.json: `arete` server runs `a4`, which is not on PATH",
            Some(NOT_ON_PATH_FIX.to_string()),
        );
        assert!(!is_fixable(&not_on_path));
        let missing = Check::warn(
            "agents.claude-code.mcp",
            ".mcp.json missing",
            Some("a4 doctor --fix".to_string()),
        );
        assert!(is_fixable(&missing));
    }

    #[test]
    fn summary_lists_what_needs_attention_and_counts_the_rest() {
        let checks = vec![
            Check::ok("cli.version", "a4 0.38.0"),
            Check::info(
                "agents.codex-trust",
                "trust the project",
                Some("run codex".into()),
            ),
            Check::info("toolchain.cargo", "cargo not found", None),
            Check::warn("net.api", "slow", None),
            Check::unknown("account.deploy", "not reported"),
            Check::ok("auth.whoami", "agent"),
        ];
        let (shown, omitted) = summarize(&checks);
        let ids: Vec<&str> = shown.iter().map(|check| check.id.as_str()).collect();
        assert_eq!(ids, ["agents.codex-trust", "net.api", "account.deploy"]);
        assert_eq!(omitted, Omitted { ok: 2, info: 1 });
        let json = serde_json::to_value(DoctorJson {
            schema_version: 1,
            status: aggregate(&checks).as_str(),
            ok_count: Some(omitted.ok),
            info_count: Some(omitted.info),
            checks: &shown,
        })
        .unwrap();
        assert_eq!(json["status"], "warn");
        assert_eq!(json["okCount"], 2);
        assert_eq!(json["infoCount"], 1);
        assert_eq!(json["checks"].as_array().unwrap().len(), 3);

        colored::control::set_override(false);
        let text = render(&shown, aggregate(&checks), Some(omitted));
        assert!(
            text.starts_with("Status: warn  (2 ok, 1 informational hidden;"),
            "{text}"
        );
        assert!(text.contains("agents.codex-trust: run codex"), "{text}");
        assert!(
            !text.contains("cli.version") && !text.contains("toolchain.cargo"),
            "{text}"
        );
        assert_eq!(text.matches("Status:").count(), 1, "{text}");

        let full = render(&checks, aggregate(&checks), None);
        assert!(full.ends_with("\nStatus: warn\n"), "{full}");
        assert!(full.contains("toolchain.cargo"), "{full}");
    }

    #[test]
    fn full_output_has_no_ok_count() {
        let json = serde_json::to_value(DoctorJson {
            schema_version: 1,
            status: "ok",
            ok_count: None,
            info_count: None,
            checks: &[],
        })
        .unwrap();
        assert!(json.get("okCount").is_none() && json.get("infoCount").is_none());
    }

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
    fn an_undecided_check_makes_the_aggregate_unknown_not_ok() {
        let checks = vec![Check::ok("a", ""), Check::unknown("b", "")];
        assert_eq!(aggregate(&checks), Status::Unknown);
        let json = serde_json::to_value(&checks[1]).unwrap();
        assert_eq!(json["status"], "unknown");
        assert!(status_label(Status::Unknown).contains("unknown"));
        // A warning or failure still wins.
        let checks = vec![Check::unknown("a", ""), Check::warn("b", "", None)];
        assert_eq!(aggregate(&checks), Status::Warn);
    }

    fn account(capabilities: &[&str]) -> AccountCapabilities {
        AccountCapabilities {
            account_kind: Some("agent".into()),
            plan: None,
            capabilities: capabilities.iter().map(|c| c.to_string()).collect(),
        }
    }

    #[test]
    fn account_checks_report_ready_capabilities() {
        let checks = account_checks(
            Ok(account(&[
                "transaction_inspect",
                "transaction_send",
                "create_deployment",
            ])),
            0,
        );
        assert_eq!(checks[0].id, "account.transactions");
        assert_eq!(checks[0].status, Status::Ok);
        assert_eq!(
            checks[0].detail,
            "can inspect and send transactions (agent account)"
        );
        assert_eq!(checks[1].id, "account.deploy");
        assert_eq!(checks[1].status, Status::Ok);
    }

    #[test]
    fn account_checks_name_missing_capabilities_without_failing() {
        let checks = account_checks(Ok(account(&["transaction_inspect"])), 0);
        assert_eq!(checks[0].status, Status::Info);
        assert!(
            checks[0].detail.contains("missing transaction_send")
                && !checks[0].detail.contains("transaction_inspect,"),
            "{}",
            checks[0].detail
        );
        assert_eq!(checks[1].status, Status::Info);
        assert!(checks[1].detail.contains("missing create_deployment"));

        // Deploying matters once arete.toml has something to deploy.
        let checks = account_checks(Ok(account(&[])), 2);
        assert!(checks[0]
            .detail
            .contains("missing transaction_inspect, transaction_send"));
        assert_eq!(checks[1].status, Status::Warn);
        assert!(checks[1].detail.contains("2 authoring stack(s)"));
        assert_ne!(aggregate(&checks), Status::Fail);
    }

    #[test]
    fn account_checks_are_unknown_when_the_server_cannot_say() {
        let checks = account_checks(Err("could not read account capabilities (boom)".into()), 3);
        assert!(checks.iter().all(|check| check.status == Status::Unknown));
        assert_eq!(
            checks[0].detail,
            "unknown: could not read account capabilities (boom)"
        );
        assert_eq!(aggregate(&checks), Status::Unknown);
        // An API without the endpoint has answered: information.
        let checks = account_checks(Err(NOT_REPORTED.into()), 3);
        assert!(checks.iter().all(|check| check.status == Status::Info));
        assert_eq!(aggregate(&checks), Status::Ok);

        let not_found: anyhow::Error = ApiHttpError {
            status: 404,
            status_text: "404 Not Found".into(),
            message: "not found".into(),
            code: None,
            upgrade_command: None,
        }
        .into();
        assert_eq!(
            account_unavailable_reason(&not_found),
            "this API does not report account capabilities yet"
        );
        assert!(
            account_unavailable_reason(&anyhow::anyhow!("connection refused"))
                .contains("connection refused")
        );
    }

    fn entitlements(plan: &str, surfaces: serde_json::Value) -> AccountEntitlements {
        serde_json::from_value(serde_json::json!({
            "schemaVersion": 1,
            "accountKind": "human",
            "plan": plan,
            "devTier": "free",
            "surfaces": surfaces,
        }))
        .unwrap()
    }

    fn check<'a>(checks: &'a [Check], id: &str) -> &'a Check {
        checks
            .iter()
            .find(|check| check.id == id)
            .unwrap_or_else(|| panic!("no {id}"))
    }

    #[test]
    fn pro_entitlements_show_raw_transactions_and_a_free_dev_tier() {
        let pro = entitlements(
            "pro",
            serde_json::json!([
                {"surface": "transaction_inspect", "allowed": true},
                {"surface": "transaction_send", "allowed": true},
                {"surface": "dev_environments", "allowed": true, "quotas": {
                    "tier": "free", "limits": {"maxEnvironments": 1}, "usage": {"environments": 0}
                }},
            ]),
        );
        let checks = entitlement_checks(&pro, &ProjectUsage::default());
        assert_eq!(
            check(&checks, "account.plan").detail,
            "plan pro (human account); dev tier free"
        );
        assert_eq!(
            check(&checks, "account.transaction-inspect").status,
            Status::Ok
        );
        assert_eq!(
            check(&checks, "account.transaction-send").status,
            Status::Ok
        );
        let dev = check(&checks, "account.dev-environments");
        assert_eq!(dev.status, Status::Ok);
        assert_eq!(dev.detail, "dev tier free: 1 environment allowed, 0 in use");
        assert_eq!(aggregate(&checks), Status::Ok);
    }

    #[test]
    fn denied_surfaces_carry_the_reason_and_unlock_and_warn_when_used() {
        let free = entitlements(
            "free",
            serde_json::json!([
                {"surface": "transaction_send", "allowed": false,
                 "reason": "Your plan does not include raw transaction send", "unlock": "Upgrade to Pro"},
                {"surface": "program_read", "allowed": false, "reason": "public stacks only", "unlock": "Upgrade"},
                {"surface": "dev_environments", "allowed": false, "reason": "turned off"},
            ]),
        );
        let checks = entitlement_checks(&free, &ProjectUsage::default());
        let send = check(&checks, "account.transaction-send");
        assert_eq!(send.status, Status::Info);
        assert_eq!(
            send.detail,
            "Your plan does not include raw transaction send"
        );
        assert_eq!(send.fix.as_deref(), Some("Upgrade to Pro"));
        // With an unlock, --summary lists it.
        assert!(needs_attention(send));
        assert_eq!(aggregate(&checks), Status::Ok);

        let usage = ProjectUsage {
            programs: 2,
            dev: true,
            ..ProjectUsage::default()
        };
        let checks = entitlement_checks(&free, &usage);
        let read = check(&checks, "account.program-read");
        assert_eq!(read.status, Status::Warn);
        assert!(
            read.detail
                .ends_with("arete.toml has 2 program dependencies"),
            "{}",
            read.detail
        );
        assert_eq!(
            check(&checks, "account.dev-environments").status,
            Status::Warn
        );
        // Raw send isn't something arete.toml declares.
        assert_eq!(
            check(&checks, "account.transaction-send").status,
            Status::Info
        );
        assert_eq!(aggregate(&checks), Status::Warn);
    }

    #[test]
    fn probe_targets_come_from_the_program_install_descriptor() {
        let install: RegistryProgramInstallResponse = serde_json::from_value(serde_json::json!({
            "installName": "klend",
            "displayName": "klend",
            "definition": {
                "programId": "KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD",
                "programSpecHash": "spec", "idlContentHash": "c", "normalizedIdlHash": "n",
                "idlPayload": {},
                "programSpec": {"artifactVersion": "1.0.0", "kind": "program-spec", "artifactHash": "spec", "payload": {}},
                "extensions": null
            },
            "release": {"programReleaseHash": "rel", "programSpecHash": "spec"},
            "transport": {"kind": "hosted-binding", "binding": {
                "endpoint": "https://reads.example.test/",
                "programReadBindingId": "prb_1",
                "auth": {"sessionEndpoint": "https://api.example.test/ws/sessions",
                         "targetKind": "program-read-binding", "targetId": "prb_1"}
            }},
            "chainBinding": {
                "endpoint": "https://gw.example.test",
                "authPolicy": "signed_session",
                "solanaGatewayBindingId": "sgb_1",
                "cluster": "mainnet-beta",
                "region": "us",
                "auth": {
                    "required": true, "mode": "signed_session",
                    "sessionEndpoint": "https://api.example.test/ws/sessions",
                    "jwksUrl": "https://api.example.test/ws/.well-known/jwks.json",
                    "tokenTransport": "bearer", "audience": "aud",
                    "targetKind": "solana-gateway-binding", "targetId": "sgb_1",
                    "scopes": ["read"], "acceptedKeyClasses": ["secret"],
                    "transactionEntitlementRequired": false
                }
            }
        }))
        .unwrap();
        let targets = ProbeTargets::from_install("klend", &install);
        let read = targets.program_read.unwrap();
        assert_eq!(read.url, "https://reads.example.test/v1/releases/rel");
        assert_eq!(read.session_request["programReleaseHash"], "rel");
        assert_eq!(read.session_request["targetId"], "prb_1");
        assert!(read.body.is_none());
        let chain = targets.chain_read.unwrap();
        assert_eq!(chain.url, "https://gw.example.test/chain/v1/account");
        assert_eq!(
            chain.session_request["targetKind"],
            "solana-gateway-binding"
        );
        assert_eq!(
            chain.body.unwrap()["address"],
            "KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD"
        );
    }

    fn probe_against(responses: Vec<(u16, String)>) -> Check {
        use crate::api_client::test_support::MockServer;
        let server = MockServer::json_sequence(responses);
        let base = server.base_url().to_string();
        let client = ApiClient::with_base_url(&base).with_api_key("a4_sk_probe".into());
        let request = ProbeRequest {
            session_endpoint: format!("{base}/ws/sessions"),
            session_request: serde_json::json!({"scopes": ["read"]}),
            url: format!("{base}/v1/releases/rel"),
            body: None,
        };
        let check = run_probe(
            &client,
            "probe.program-read",
            "Program Read",
            "klend",
            Some(&request),
        );
        let mint = server.request();
        assert_eq!(mint.request_line, "POST /ws/sessions HTTP/1.1");
        check
    }

    #[test]
    fn a_failing_program_read_limiter_is_a_named_retryable_warning() {
        let check = probe_against(vec![
            (200, r#"{"token":"tok"}"#.into()),
            (503, r#"{"error":{"code":"RATE_LIMIT_UNAVAILABLE"}}"#.into()),
        ]);
        assert_eq!(check.status, Status::Warn);
        assert!(
            check.detail.contains("HTTP 503 RATE_LIMIT_UNAVAILABLE"),
            "{}",
            check.detail
        );
        assert!(check.detail.contains("retryable"), "{}", check.detail);
        assert!(check.detail.contains("not your plan"), "{}", check.detail);
        let checks = vec![Check::ok("cli.version", ""), check];
        assert_ne!(aggregate(&checks), Status::Ok);
        let (shown, _) = summarize(&checks);
        assert_eq!(shown[0].id, "probe.program-read");
    }

    #[test]
    fn a_working_probe_reports_its_status() {
        let check = probe_against(vec![
            (200, r#"{"token":"tok"}"#.into()),
            (200, r#"{"readiness":"ready"}"#.into()),
        ]);
        assert_eq!(check.status, Status::Ok);
        assert_eq!(check.detail, "Program Read for klend → 200");
    }

    #[test]
    fn a_refused_probe_session_names_status_and_code() {
        let check = probe_against(vec![(
            500,
            r#"{"error":"boom","code":"internal_error"}"#.into(),
        )]);
        assert_eq!(check.status, Status::Warn);
        assert!(
            check.detail.contains("HTTP 500 internal_error"),
            "{}",
            check.detail
        );
        assert!(check.detail.contains("retryable"), "{}", check.detail);
    }

    #[test]
    fn probes_are_skipped_without_a_registry_program() {
        let checks = probe_checks("a4_sk_unused", &ProjectUsage::default(), None);
        assert_eq!(checks.len(), 2);
        assert!(checks.iter().all(|check| check.status == Status::Info));
    }

    fn write(path: &Path, contents: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    fn npm_package(root: &Path, dir: &str, name: &str, version: &str, api: Option<u32>) {
        let arete = api
            .map(|api| format!(r#","arete":{{"extensionApi":{api}}}"#))
            .unwrap_or_default();
        write(
            &root.join(dir).join("package.json"),
            &format!(r#"{{"name":"{name}","version":"{version}"{arete}}}"#),
        );
    }

    /// A project with one TypeScript output generated by a4 `generator`.
    fn runtime_project(generator: &str) -> (tempfile::TempDir, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        write(
            &temp.path().join("package.json"),
            r#"{"dependencies":{"react":"^19"}}"#,
        );
        let output = temp.path().join("src/arete/ore");
        write(
            &output.join("sdk-provenance.json"),
            &format!(r#"{{"generator":{{"name":"a4-cli","version":"{generator}"}}}}"#),
        );
        (temp, output)
    }

    #[test]
    fn sdk_runtime_is_not_applicable_without_outputs_or_node_modules() {
        let (temp, output) = runtime_project("0.23.0");
        let check = sdk_runtime(temp.path(), &[], "0.23.0");
        assert_eq!(check.status, Status::Info);
        assert_eq!(check.detail, "not applicable (no TypeScript outputs)");
        let check = sdk_runtime(temp.path(), &[output], "0.23.0");
        assert_eq!(check.status, Status::Info);
        assert_eq!(check.detail, "not applicable (no node_modules)");
    }

    #[test]
    fn sdk_runtime_accepts_one_matching_set() {
        let (temp, output) = runtime_project("0.23.0");
        let root = temp.path();
        npm_package(
            root,
            "node_modules/@usearete/sdk",
            "@usearete/sdk",
            "0.23.0",
            Some(1),
        );
        npm_package(
            root,
            "node_modules/@usearete/react",
            "@usearete/react",
            "0.23.0",
            None,
        );
        write(&output.join("extensions.json"), r#"{"extensionApi":1}"#);
        let check = sdk_runtime(root, &[output], "0.23.0");
        assert_eq!(check.status, Status::Ok, "{}", check.detail);
        assert_eq!(
            check.detail,
            "@usearete/react 0.23.0, @usearete/sdk 0.23.0 (extension API 1)"
        );
    }

    #[test]
    fn sdk_runtime_warns_about_mixed_versions_duplicates_and_generator_drift() {
        let (temp, output) = runtime_project("0.23.0");
        let root = temp.path();
        npm_package(
            root,
            "node_modules/@usearete/sdk",
            "@usearete/sdk",
            "0.22.1",
            Some(1),
        );
        npm_package(
            root,
            "node_modules/@usearete/react",
            "@usearete/react",
            "0.23.0",
            None,
        );
        npm_package(
            root,
            "node_modules/@usearete/react/node_modules/@usearete/sdk",
            "@usearete/sdk",
            "0.23.0",
            Some(1),
        );
        let check = sdk_runtime(root, &[output], "0.23.0");
        assert_eq!(check.status, Status::Warn, "{}", check.detail);
        assert!(
            check.detail.contains(
                "runtime packages differ in version (@usearete/sdk 0.22.1, @usearete/react 0.23.0)"
            ),
            "{}",
            check.detail
        );
        assert!(
            check.detail.contains("2 copies of @usearete/sdk (node_modules/@usearete/react/node_modules/@usearete/sdk 0.23.0, node_modules/@usearete/sdk 0.22.1)")
                || check.detail.contains("2 copies of @usearete/sdk (node_modules/@usearete/sdk 0.22.1, node_modules/@usearete/react/node_modules/@usearete/sdk 0.23.0)"),
            "{}",
            check.detail
        );
        assert!(
            check.detail.contains(
                "src/arete/ore was generated by a4 0.23.0; the installed runtime is 0.22.1"
            ),
            "{}",
            check.detail
        );
        assert_eq!(
            check.fix.as_deref(),
            Some(r#"npm install @usearete/sdk@0.23.0 @usearete/react@0.23.0 "zod@^3.24.1""#)
        );
        assert_ne!(aggregate(&[check]), Status::Fail);
    }

    #[test]
    fn sdk_runtime_fails_only_for_an_extension_api_mismatch() {
        let (temp, output) = runtime_project("0.24.0");
        let root = temp.path();
        npm_package(
            root,
            "node_modules/@usearete/sdk",
            "@usearete/sdk",
            "0.24.0",
            Some(2),
        );
        write(
            &output.join("programs/ore/extensions.json"),
            r#"{"entry":"index.ts","extensionApi":1}"#,
        );
        let check = sdk_runtime(root, std::slice::from_ref(&output), "0.25.0");
        assert_eq!(check.status, Status::Fail);
        assert!(
            check.detail.contains(
                "src/arete/ore/programs/ore/extensions.json requires extension API 1, but @usearete/sdk 0.24.0 provides 2"
            ),
            "{}",
            check.detail
        );
        assert!(
            check
                .fix
                .as_deref()
                .unwrap()
                .contains("or run `a4 install` to regenerate them for 0.25.0"),
            "{:?}",
            check.fix
        );

        // An SDK that predates the contract only warns.
        npm_package(
            root,
            "node_modules/@usearete/sdk",
            "@usearete/sdk",
            "0.24.0",
            None,
        );
        let check = sdk_runtime(root, &[output], "0.24.0");
        assert_eq!(check.status, Status::Warn);
        assert!(check.detail.contains("predates extension API versioning"));
    }

    #[test]
    fn json_shape_matches_spec() {
        let checks = vec![Check::ok("cli.version", "0.13.0 (latest)")];
        let output = DoctorJson {
            schema_version: 1,
            status: aggregate(&checks).as_str(),
            ok_count: None,
            info_count: None,
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
    fn agents_found_only_in_the_home_directory_are_information() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("proj");
        let home = dir.path().join("home");
        std::fs::create_dir_all(root.join(".cursor")).unwrap();
        std::fs::create_dir_all(home.join(".cursor")).unwrap();
        std::fs::create_dir_all(home.join(".gemini")).unwrap();
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        let env = Env::new(&root, Some(home), &[]);
        let detection = detect(&env);
        assert_eq!(project_agent_ids(&detection), vec!["cursor".to_string()]);

        let checks = agent_checks(&env, &detection);
        let status = |id: &str| {
            checks
                .iter()
                .find(|check| check.id == id)
                .unwrap_or_else(|| panic!("{id}"))
                .status
        };
        // Cursor has a project directory: its missing MCP config is a problem.
        assert_eq!(status("agents.cursor.mcp"), Status::Warn);
        // Gemini is only installed: nothing about this project is wrong.
        assert_eq!(status("agents.gemini-cli.mcp"), Status::Info);
        let gemini = checks
            .iter()
            .find(|check| check.id == "agents.gemini-cli.mcp")
            .unwrap();
        assert!(gemini.detail.contains("not set up in this project"));
        assert_eq!(
            gemini.fix.as_deref(),
            Some("a4 init --agents gemini-cli --no-manifest --no-agents-md --no-skills")
        );
        let skills = checks
            .iter()
            .find(|check| check.id == "agents.gemini-cli.skills")
            .expect("Gemini skills check should exist");
        assert_eq!(skills.status, Status::Info);

        // --fix skips home-only agents, so their context files are not
        // warnings either: CLAUDE.md and the Gemini context setting.
        assert_eq!(status("agents.claude-code.mcp"), Status::Info);
        for id in ["agents.claude-md", "agents.gemini-context"] {
            assert_eq!(status(id), Status::Info, "{id}");
        }
        let claude_md = checks
            .iter()
            .find(|check| check.id == "agents.claude-md")
            .unwrap();
        assert_eq!(
            claude_md.fix.as_deref(),
            Some("a4 init --agents claude-code --no-manifest --no-skills --no-mcp")
        );
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
