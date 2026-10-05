//! `a4 doctor`: read-only health check of everything `a4 init` writes plus
//! the environment; `--fix` re-runs the init writers for failing agent checks.
//!
//! Spec: `docs/internal/agent-first-onboarding.md` (WP8).

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
    api_error_details, AccountCapabilities, ApiClient, CAPABILITY_CREATE_DEPLOYMENT,
    CAPABILITY_TRANSACTION_INSPECT, CAPABILITY_TRANSACTION_SEND,
};
use crate::project::manifest::InstallTarget;
use crate::project::runtime;
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
    let summary = match status {
        Status::Ok => "ok".green().bold().to_string(),
        Status::Warn => "warn".yellow().bold().to_string(),
        Status::Fail => "fail".red().bold().to_string(),
        Status::Info | Status::Unknown => "ok".green().bold().to_string(),
    };
    out.push_str(&format!("\nStatus: {summary}\n"));
    out
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

    // account.transactions / account.deploy
    let authoring = facts.as_ref().map(|f| f.authoring_stacks).unwrap_or(0);
    checks.extend(match &key {
        None => vec![
            Check::info("account.transactions", "skipped (no credentials)", None),
            Check::info("account.deploy", "skipped (no credentials)", None),
        ],
        Some(key) => account_checks(account_capabilities(key), authoring),
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

/// A neutral, user-facing reason account capabilities could not be read.
pub(crate) fn account_unavailable_reason(error: &anyhow::Error) -> String {
    match api_error_details(error).map(|error| error.status) {
        Some(404) => "this API does not report account capabilities yet".to_string(),
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
/// project uses.
fn project_agent_ids(detection: &Detection) -> Vec<String> {
    detection
        .agents
        .iter()
        .filter(|agent| agent.how != How::Home)
        .map(|agent| agent.id.clone())
        .collect()
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
    fn aggregate_status_prefers_fail_then_warn() {
        let checks = vec![Check::ok("a", ""), Check::info("b", "", None)];
        assert_eq!(aggregate(&checks), Status::Ok);
        let checks = vec![Check::ok("a", ""), Check::warn("b", "", None)];
        assert_eq!(aggregate(&checks), Status::Warn);
        let checks = vec![Check::warn("a", "", None), Check::fail("b", "", None)];
        assert_eq!(aggregate(&checks), Status::Fail);
    }

    #[test]
    fn unknown_status_is_neutral_in_the_aggregate() {
        let checks = vec![Check::ok("a", ""), Check::unknown("b", "")];
        assert_eq!(aggregate(&checks), Status::Ok);
        let json = serde_json::to_value(&checks[1]).unwrap();
        assert_eq!(json["status"], "unknown");
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
        let checks = account_checks(
            Err("this API does not report account capabilities yet".into()),
            3,
        );
        assert!(checks.iter().all(|check| check.status == Status::Unknown));
        assert_eq!(
            checks[0].detail,
            "unknown: this API does not report account capabilities yet"
        );
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
