use anyhow::{Context, Result};
use colored::Colorize;
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};

use crate::api_client::{api_error_details, AgentMeResponse, ApiClient, PendingAgentSignup};
use crate::config;
use crate::ui;

fn credentials_path() -> String {
    ApiClient::credentials_file_path()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|_| "~/.arete/credentials.toml".to_string())
}

fn trial_remaining_seconds(expires_at: Option<&str>) -> Option<i64> {
    let expires_at = chrono::DateTime::parse_from_rfc3339(expires_at?).ok()?;
    Some(
        expires_at
            .signed_duration_since(chrono::Utc::now())
            .num_seconds()
            .max(0),
    )
}

fn format_trial_remaining(expires_at: Option<&str>) -> Option<String> {
    let seconds = trial_remaining_seconds(expires_at)?;
    Some(format_remaining_seconds(seconds))
}

fn format_remaining_seconds(seconds: i64) -> String {
    if seconds == 0 {
        return "expired".to_string();
    }
    let days = seconds / 86_400;
    let hours = (seconds % 86_400) / 3_600;
    let minutes = (seconds % 3_600) / 60;
    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {minutes}m")
    } else if minutes > 0 {
        format!("{minutes}m")
    } else {
        format!("{seconds}s")
    }
}

fn signup_expiry_matches(identity: Option<&str>, signup: &str) -> bool {
    // PostgreSQL stores timestamps at microsecond precision and rounds the
    // signup response's nanoseconds in either direction. Allow one complete
    // microsecond instead of truncating both values, which fails when the
    // database rounds across a microsecond boundary.
    const DATABASE_TIMESTAMP_TOLERANCE_NANOS: i64 = 1_000;

    let Some(identity) = identity else {
        return false;
    };
    let Ok(identity) = chrono::DateTime::parse_from_rfc3339(identity) else {
        return false;
    };
    let Ok(signup) = chrono::DateTime::parse_from_rfc3339(signup) else {
        return false;
    };
    identity
        .signed_duration_since(signup)
        .num_nanoseconds()
        .is_some_and(|delta| {
            (-DATABASE_TIMESTAMP_TOLERANCE_NANOS..=DATABASE_TIMESTAMP_TOLERANCE_NANOS)
                .contains(&delta)
        })
}

pub fn login(api_key: Option<String>, requested_profile: Option<&str>) -> Result<()> {
    let api_url = config::get_api_url(None);

    let api_key = if let Some(key) = api_key {
        key
    } else {
        if !ui::interactive() {
            anyhow::bail!(
                "Missing --key and no terminal to prompt on. Pass: a4 auth login --profile human --key <a4_sk_...> (or register as an agent: a4 auth signup)"
            );
        }
        println!("{}", "Login to Arete".bold());
        println!();
        println!("Target API: {}", api_url.yellow());
        println!();
        print!("API Key: ");
        io::stdout().flush()?;

        let mut key = String::new();
        io::stdin().read_line(&mut key)?;
        key.trim().to_string()
    };

    if api_key.is_empty() {
        anyhow::bail!("API key cannot be empty");
    }

    let profile = requested_profile
        .map(str::to_string)
        .unwrap_or_else(|| arete_mcp::credentials::inferred_profile_for_key(&api_key).to_string());
    arete_mcp::credentials::validate_key_for_profile(&profile, &api_key)?;

    // Verify before writing so a typo cannot overwrite a valid stored key.
    let spinner = ui::create_spinner("Verifying API key...");
    let client = ApiClient::with_base_url(&api_url).with_api_key(api_key.clone());

    let verification = if api_key.starts_with("a4_ak_") {
        client.agent_me().map(|_| ())
    } else {
        client.list_specs().map(|_| ())
    };
    match verification {
        Ok(_) => {
            let saved = ApiClient::save_api_key_for_profile(&api_key, Some(&api_url), &profile);
            spinner.finish_and_clear();
            saved?;
            ui::print_success("API key saved and verified!");
            println!();
            println!("  Profile:     {}", profile);
            println!("  Credentials: {}", credentials_path().dimmed());
            println!();
            println!("You are now ready to use Arete!");
        }
        Err(e) => {
            spinner.finish_and_clear();
            anyhow::bail!("Invalid API key: {}", e);
        }
    }

    Ok(())
}

pub fn logout() -> Result<()> {
    let api_url = config::get_api_url(None);

    let spinner = ui::create_spinner("Logging out...");

    // Delete only the selected/resolved profile. Removing every principal is
    // intentionally reserved for the explicit `logout-all` command.
    let result = ApiClient::delete_api_key_for_url(&api_url);
    spinner.finish_and_clear();
    result?;
    ui::print_success(&format!("Logged out from {}", api_url));
    println!("  The selected credential has been removed from this device.");

    Ok(())
}

pub fn logout_all() -> Result<()> {
    let spinner = ui::create_spinner("Logging out from all environments...");

    ApiClient::delete_all_api_keys()?;

    spinner.finish_and_clear();
    ui::print_success("Logged out from all environments!");
    println!("  All credentials have been removed from this device.");

    Ok(())
}

pub fn status(json: bool) -> Result<()> {
    let api_url = config::get_api_url(None);
    let selected_profile = ApiClient::selected_profile()?;

    if json {
        let key = ApiClient::load_optional_api_key_for_url(&api_url)?;
        let direct_env_key = selected_profile.is_none()
            && std::env::var("ARETE_API_KEY").is_ok_and(|value| !value.trim().is_empty());
        let principal_kind = match key.as_deref() {
            Some(key) if key.starts_with("a4_ak_") => "agent",
            Some(_) => "human",
            None => "unknown",
        };
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "schemaVersion": 1,
                "authenticated": key.is_some(),
                "principalKind": principal_kind,
                "credentialSource": if direct_env_key { "env:ARETE_API_KEY" } else if key.is_some() { "credentials_file" } else { "none" },
                "profile": selected_profile,
                "agent": null,
            }))?
        );
        return Ok(());
    }

    println!("{}", "Authentication Status".bold());
    println!();
    println!("Current target API: {}", api_url.yellow());
    println!(
        "Credential profile: {}",
        selected_profile.as_deref().unwrap_or("auto").yellow()
    );
    println!();

    // Try to load key for current URL
    match ApiClient::load_api_key_for_url(&api_url) {
        Ok(api_key) => {
            println!(
                "{} {}",
                ui::symbols::SUCCESS.green().bold(),
                "Authenticated".green().bold()
            );
            println!();
            println!(
                "  API key: {}...{}",
                &api_key[..8.min(api_key.len())],
                if api_key.len() > 12 {
                    &api_key[api_key.len() - 4..]
                } else {
                    ""
                }
            );
            println!("  Credentials: {}", credentials_path().dimmed());
            println!(
                "  Principal:   {}",
                if api_key.starts_with("a4_ak_") {
                    "agent"
                } else {
                    "human"
                }
            );
            println!();
            println!(
                "  Run {} to verify with the server.",
                "a4 auth whoami".cyan()
            );
        }
        Err(_) => {
            println!(
                "{} {}",
                ui::symbols::FAILURE.red().bold(),
                "Not authenticated".red().bold()
            );
            println!();
            println!("Run 'a4 auth login' to authenticate.");
        }
    }

    // List all stored credentials
    match ApiClient::list_credentials() {
        Ok(creds) if !creds.is_empty() => {
            println!();
            println!("{}", "Stored credentials:".dimmed());
            for credential in creds {
                let is_current = credential.api_url == api_url
                    || (api_url.contains("localhost")
                        && (credential.api_url.contains("localhost")
                            || credential.api_url.contains("127.0.0.1")));
                let marker = if is_current { "→ " } else { "  " };
                let profile = credential.profile.as_deref().unwrap_or("legacy");
                println!(
                    "{}[{}] {} {} ({})",
                    marker,
                    profile,
                    credential.api_url,
                    if is_current {
                        "(current)".green()
                    } else {
                        "".normal()
                    },
                    credential.masked_key,
                );
            }
        }
        _ => {}
    }

    Ok(())
}

pub fn whoami(json: bool) -> Result<()> {
    let api_url = config::get_api_url(None);

    let api_key = match ApiClient::load_api_key_for_url(&api_url) {
        Ok(key) => key,
        Err(_) => {
            ui::print_error("Not authenticated");
            println!();
            println!("  Run {} to authenticate.", "a4 auth login".cyan());
            return Ok(());
        }
    };

    let spinner = (!json).then(|| ui::create_spinner("Verifying authentication..."));
    let client = ApiClient::new()?;

    if api_key.starts_with("a4_ak_") {
        let identity = client.agent_me()?;
        if json {
            print_agent_identity_json(&identity)?;
            return Ok(());
        }
        if let Some(spinner) = spinner {
            spinner.finish_and_clear();
        }
        println!(
            "{} {}",
            ui::symbols::SUCCESS.green().bold(),
            "Authenticated agent".green().bold()
        );
        println!();
        println!("  Slug:        {}", identity.slug);
        println!("  Name:        {}", identity.display_name);
        println!("  Claim state: {}", identity.claim_state);
        println!("  Status:      {}", identity.status);
        if let Some(plan) = &identity.plan {
            println!("  Plan:        {plan}");
        }
        if let Some(expires_at) = &identity.entitlement_expires_at {
            println!("  Expires:     {expires_at}");
            if let Some(remaining) = format_trial_remaining(Some(expires_at)) {
                println!("  Remaining:   {remaining}");
            }
        }
        if let Some(usage) = &identity.usage {
            println!(
                "  Usage:       {}",
                if usage.exhausted {
                    "exhausted".red().bold()
                } else {
                    "active".green().normal()
                }
            );
            for meter in &usage.meters {
                println!(
                    "    {:<25} {:>12} / {}{}",
                    meter.meter,
                    meter.consumed,
                    meter.allowance,
                    if meter.exhausted { " (exhausted)" } else { "" }
                );
            }
        }
        println!("  Target API:  {}", api_url.yellow());
        println!("  Credentials: {}", credentials_path().dimmed());
        return Ok(());
    }

    match client.list_specs() {
        Ok(specs) => {
            if let Some(spinner) = spinner.as_ref() {
                spinner.finish_and_clear();
            }
            if json {
                println!(
                    "{}",
                    serde_json::to_string(&serde_json::json!({
                        "schemaVersion": 1,
                        "principalKind": "human",
                        "credentialSource": "credentials_file",
                        "stackCount": specs.len(),
                    }))?
                );
                return Ok(());
            }
            println!(
                "{} {}",
                ui::symbols::SUCCESS.green().bold(),
                "Authenticated".green().bold()
            );
            println!();
            println!(
                "  API key: {}...{}",
                &api_key[..8.min(api_key.len())],
                &api_key[api_key.len().saturating_sub(4)..]
            );
            println!("  Stacks: {}", specs.len());
            println!("  Target API: {}", api_url.yellow());
            println!("  Credentials: {}", credentials_path().dimmed());
        }
        Err(e) => {
            if let Some(spinner) = spinner.as_ref() {
                spinner.finish_and_clear();
            }
            ui::print_error("API key invalid or expired");
            println!();
            println!("  Error: {}", e);
            println!();
            println!("  Run {} to re-authenticate.", "a4 auth login".cyan());
        }
    }

    Ok(())
}

fn print_agent_identity_json(identity: &AgentMeResponse) -> Result<()> {
    println!(
        "{}",
        serde_json::to_string(&serde_json::json!({
            "schemaVersion": 1,
            "principalKind": "agent",
            "slug": identity.slug,
            "displayName": identity.display_name,
            "status": identity.status,
            "claimState": identity.claim_state,
            "plan": identity.plan,
            "entitlementExpiresAt": identity.entitlement_expires_at,
            "trialRemainingSeconds": trial_remaining_seconds(identity.entitlement_expires_at.as_deref()),
            "trialAccessEnabled": identity.trial_access_enabled,
            "starterGuidance": identity.starter_guidance,
            "usage": identity.usage,
            "credentialSource": "credentials_file",
        }))?
    );
    Ok(())
}

pub fn claim_link(json: bool) -> Result<()> {
    let client = ApiClient::new()?;
    let ready = client.agent_claim_link()?;
    let expected_origin = ApiClient::configured_claim_app_origin()?;
    if !ready.action.is_safe_claim_url_for_origin(&expected_origin) {
        anyhow::bail!("Server returned an unsafe agent claim URL");
    }

    if json {
        println!("{}", serde_json::to_string(&ready)?);
    } else {
        println!("A human owner must open this link to claim the agent:");
        println!("{}", ready.action.url);
        println!("Expires: {}", ready.action.expires_at);
        println!("The agent must not open or complete this link itself.");
    }
    Ok(())
}

// ============================================================================
// Publishable Key Management
// ============================================================================

pub fn list_keys(json: bool) -> Result<()> {
    let client = ApiClient::new()?;

    let spinner = (!json).then(|| ui::create_spinner("Fetching API keys..."));
    let result = client.list_api_keys();
    if let Some(spinner) = spinner {
        spinner.finish_and_clear();
    }

    match result {
        Ok(keys) => {
            if json {
                let keys = keys
                    .iter()
                    .map(|key| {
                        serde_json::json!({
                            "id": key.id,
                            "name": key.name,
                            "keyClass": key.key_class,
                            "origins": key.origin_allowlist.clone().unwrap_or_default(),
                            "expiresAt": key.expires_at,
                            "lastUsedAt": key.last_used_at,
                            "createdAt": key.created_at,
                        })
                    })
                    .collect::<Vec<_>>();
                let payload = serde_json::json!({ "schemaVersion": 1, "keys": keys });
                println!("{}", serde_json::to_string_pretty(&payload)?);
                return Ok(());
            }

            if keys.is_empty() {
                println!("{}", "No API keys found.".yellow());
                println!();
                println!(
                    "  Run {} to create a publishable key for browser use.",
                    "a4 auth keys create-publishable".cyan()
                );
                return Ok(());
            }

            println!("{}", "API Keys:".bold());
            println!();

            for key in keys {
                let key_type = match key.key_class.as_str() {
                    "publishable" => "publishable".green(),
                    "secret" => "secret".cyan(),
                    _ => key.key_class.normal(),
                };

                println!(
                    "  {} {}",
                    "•".bold(),
                    key.name.unwrap_or_else(|| "Unnamed".to_string())
                );
                println!("    ID:    {}", key.id);
                println!("    Type:  {}", key_type);

                if let Some(origins) = key.origin_allowlist {
                    if !origins.is_empty() {
                        println!("    Origins: {}", origins.join(", "));
                    }
                }

                if let Some(expires) = key.expires_at {
                    println!(
                        "    Expires: {}",
                        expires.split('T').next().unwrap_or(&expires)
                    );
                }

                if let Some(last_used) = key.last_used_at {
                    println!(
                        "    Last used: {}",
                        last_used.split('T').next().unwrap_or(&last_used)
                    );
                }

                println!();
            }
        }
        Err(error) => {
            let message = format!("Failed to list keys: {error}");
            return Err(error.context(message));
        }
    }

    Ok(())
}

/// Options for `a4 auth keys create-publishable`.
pub struct CreatePublishableArgs {
    pub name: Option<String>,
    pub origins: Vec<String>,
    pub expiry_days: Option<i64>,
    /// Write only the key's environment variable into this file.
    pub env_file: Option<String>,
}

/// The browser framework a project uses, which decides the environment
/// variable its bundler exposes to client code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Framework {
    NextJs,
    Vite,
    Generic,
}

impl Framework {
    fn id(self) -> &'static str {
        match self {
            Framework::NextJs => "nextjs",
            Framework::Vite => "vite",
            Framework::Generic => "generic",
        }
    }

    fn env_var(self) -> &'static str {
        match self {
            Framework::NextJs => "NEXT_PUBLIC_ARETE_PUBLISHABLE_KEY",
            Framework::Vite => "VITE_ARETE_PUBLISHABLE_KEY",
            Framework::Generic => "ARETE_PUBLISHABLE_KEY",
        }
    }

    fn how(self) -> &'static str {
        match self {
            Framework::NextJs => "Next.js exposes NEXT_PUBLIC_* variables to browser code",
            Framework::Vite => "Vite exposes VITE_* variables to browser code as import.meta.env",
            Framework::Generic => "read it from the environment and pass it as auth.publishableKey",
        }
    }
}

const NEXT_CONFIGS: [&str; 5] = [
    "next.config.js",
    "next.config.mjs",
    "next.config.cjs",
    "next.config.ts",
    "next.config.mts",
];
const VITE_CONFIGS: [&str; 6] = [
    "vite.config.js",
    "vite.config.mjs",
    "vite.config.cjs",
    "vite.config.ts",
    "vite.config.mts",
    "vite.config.cts",
];

/// Detect the framework from config files and package.json dependencies in
/// the project directory. Next.js wins over Vite, because a Next.js app may
/// carry Vite as a test dependency.
fn detect_framework(root: &Path) -> Framework {
    let package_deps = std::fs::read_to_string(root.join("package.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .map(|package| {
            ["dependencies", "devDependencies"]
                .iter()
                .filter_map(|key| package.get(*key).and_then(serde_json::Value::as_object))
                .flat_map(|deps| deps.keys().cloned())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let has_dep = |name: &str| package_deps.iter().any(|dep| dep == name);
    let has_file = |names: &[&str]| names.iter().any(|name| root.join(name).is_file());
    if has_file(&NEXT_CONFIGS) || has_dep("next") {
        Framework::NextJs
    } else if has_file(&VITE_CONFIGS) || has_dep("vite") {
        Framework::Vite
    } else {
        Framework::Generic
    }
}

/// Check the one origin a publishable key allows. The browser sends
/// `scheme://host[:port]` with no path, so anything else could never match.
fn validate_origins(origins: &[String]) -> Result<String> {
    let origin = match origins {
        [origin] => origin.trim(),
        [] => anyhow::bail!(
            "A publishable key needs exactly one origin (e.g. https://example.com or http://localhost:5173)"
        ),
        many => anyhow::bail!(
            "A publishable key allows exactly one origin; got {} ({}). Create one key per origin.",
            many.len(),
            many.join(", ")
        ),
    };
    if !origin.starts_with("https://") && !origin.starts_with("http://") {
        anyhow::bail!("Invalid origin '{origin}'. Origins must start with https:// or http://");
    }
    let parsed = url::Url::parse(origin)
        .map_err(|error| anyhow::anyhow!("Invalid origin '{origin}': {error}"))?;
    let normalized = parsed.origin().ascii_serialization();
    if normalized != origin {
        anyhow::bail!(
            "Invalid origin '{origin}'. Browsers send the origin as scheme://host[:port] with no path or trailing slash; use --origin {normalized}"
        );
    }
    Ok(normalized)
}

/// Resolve `--env-file`. A relative path is taken from the project directory
/// and must stay inside it, including through symlinked directories, and must
/// not name a symlink; it resolves to its real location. An absolute path is
/// an explicit choice and is used as given, following a symlink to the file
/// it points to.
fn resolve_env_file(root: &Path, raw: &str) -> Result<PathBuf> {
    let raw = raw.trim();
    if raw.is_empty() {
        anyhow::bail!("--env-file must not be empty");
    }
    let path = Path::new(raw);
    if path.is_absolute() {
        if is_symlink(path) {
            return std::fs::canonicalize(path).map_err(|error| {
                anyhow::anyhow!("--env-file {raw} is a symlink that cannot be followed: {error}")
            });
        }
        return Ok(path.to_path_buf());
    }
    let project = std::fs::canonicalize(root).map_err(|error| {
        anyhow::anyhow!(
            "Failed to resolve the project directory {}: {error}",
            root.display()
        )
    })?;
    let outside = || {
        anyhow::anyhow!(
            "--env-file {raw} points outside the project directory ({}). Pass an absolute path to write there explicitly.",
            project.display()
        )
    };
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => parts.push(part),
            Component::CurDir => {}
            _ => return Err(outside()),
        }
    }
    let Some(file_name) = parts.pop() else {
        anyhow::bail!("--env-file {raw} must name a file");
    };
    // The lexical check alone would follow a symlinked directory out of the
    // project: resolve the directory and check where it really is.
    let parent = parts
        .iter()
        .fold(root.to_path_buf(), |directory, part| directory.join(part));
    let directory = match std::fs::canonicalize(&parent) {
        Ok(directory) => directory,
        Err(error) if error.kind() == io::ErrorKind::NotFound => anyhow::bail!(
            "--env-file {raw}: directory {} does not exist",
            parent.display()
        ),
        Err(error) => anyhow::bail!("Failed to resolve --env-file {raw}: {error}"),
    };
    if !directory.starts_with(&project) {
        return Err(outside());
    }
    let resolved = directory.join(file_name);
    if is_symlink(&resolved) {
        anyhow::bail!(
            "--env-file {raw} is a symlink; pass the absolute path of the file it points to"
        );
    }
    Ok(resolved)
}

fn is_symlink(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink())
}

/// Read the current env file (if any) before the key is created, so a path
/// problem fails without creating a key that could then be lost.
fn read_env_file(path: &Path) -> Result<Option<String>> {
    if path.is_dir() {
        anyhow::bail!("--env-file {} is a directory", path.display());
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if !parent.is_dir() {
        anyhow::bail!(
            "--env-file {}: directory {} does not exist",
            path.display(),
            parent.display()
        );
    }
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(anyhow::anyhow!(
            "Failed to read --env-file {}: {error}",
            path.display()
        )),
    }
}

/// Replace the env file with `content` atomically: write a temporary file
/// beside it and rename it over the original, so a failed write leaves every
/// other variable in place. An existing file keeps its permissions (env files
/// are often private); a new one gets the default mode.
fn write_env_file(path: &Path, content: &str) -> io::Result<()> {
    let directory = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut temporary_name = std::ffi::OsString::from(".");
    temporary_name.push(path.file_name().unwrap_or_default());
    temporary_name.push(format!(".{}.tmp", uuid::Uuid::new_v4()));
    let temporary = directory.join(temporary_name);
    let permissions = match std::fs::metadata(path) {
        Ok(metadata) => Some(metadata.permissions()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    let result = (|| {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        // Private until the original's permissions are applied.
        #[cfg(unix)]
        if permissions.is_some() {
            std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(content.as_bytes())?;
        if let Some(permissions) = permissions {
            file.set_permissions(permissions)?;
        }
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

/// How an env file changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EnvChange {
    Created,
    Replaced,
    Appended,
    Unchanged,
}

impl EnvChange {
    fn as_str(self) -> &'static str {
        match self {
            EnvChange::Created => "created",
            EnvChange::Replaced => "replaced",
            EnvChange::Appended => "appended",
            EnvChange::Unchanged => "unchanged",
        }
    }
}

/// Whether `line` assigns `name` (`NAME=…` or `export NAME=…`, any leading
/// whitespace).
fn assigns(line: &str, name: &str) -> bool {
    let line = line.trim_start();
    let line = line
        .strip_prefix("export")
        .filter(|rest| rest.starts_with(char::is_whitespace))
        .map(str::trim_start)
        .unwrap_or(line);
    line.strip_prefix(name)
        .is_some_and(|rest| rest.trim_start().starts_with('='))
}

/// Set `name=value` in env-file text, touching no other line: every existing
/// assignment of `name` is rewritten in place (keeping `export` and
/// indentation); otherwise one line is appended.
fn upsert_env_var(existing: Option<&str>, name: &str, value: &str) -> (String, EnvChange) {
    let Some(existing) = existing else {
        return (format!("{name}={value}\n"), EnvChange::Created);
    };
    let mut output = String::with_capacity(existing.len() + name.len() + value.len() + 2);
    let mut replaced = false;
    for line in existing.split_inclusive('\n') {
        let (body, ending) = match line.strip_suffix("\r\n") {
            Some(body) => (body, "\r\n"),
            None => match line.strip_suffix('\n') {
                Some(body) => (body, "\n"),
                None => (line, ""),
            },
        };
        if assigns(body, name) {
            let indent_len = body.len() - body.trim_start().len();
            let exported = body.trim_start().starts_with("export");
            output.push_str(&body[..indent_len]);
            if exported {
                output.push_str("export ");
            }
            output.push_str(&format!("{name}={value}{ending}"));
            replaced = true;
        } else {
            output.push_str(line);
        }
    }
    if replaced {
        let change = if output == existing {
            EnvChange::Unchanged
        } else {
            EnvChange::Replaced
        };
        return (output, change);
    }
    let newline = if existing.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    if !output.is_empty() && !output.ends_with('\n') {
        output.push_str(newline);
    }
    output.push_str(&format!("{name}={value}{newline}"));
    (output, EnvChange::Appended)
}

pub fn create_publishable_key(
    args: CreatePublishableArgs,
    config_path: &str,
    json: bool,
) -> Result<()> {
    let origin = validate_origins(&args.origins)?;
    let root = super::init::project_root(config_path);
    let framework = detect_framework(&root);
    let env_var = framework.env_var();
    let env_target = match args.env_file.as_deref() {
        Some(raw) => {
            let path = resolve_env_file(&root, raw)?;
            let existing = read_env_file(&path)?;
            Some((path, existing))
        }
        None => None,
    };

    let client = ApiClient::new()?;
    let spinner = (!json).then(|| ui::create_spinner("Creating publishable key..."));
    let result =
        client.create_publishable_key(args.name.clone(), vec![origin.clone()], args.expiry_days);
    if let Some(spinner) = spinner {
        spinner.finish_and_clear();
    }
    let response = match result {
        Ok(response) => response,
        Err(error) => {
            let message = format!("Failed to create publishable key: {error}");
            return Err(error.context(message));
        }
    };

    // The key exists now and is shown only once: report it even if writing
    // the env file fails, then fail the command.
    let written = env_target.map(|(path, existing)| {
        let (content, change) = upsert_env_var(existing.as_deref(), env_var, &response.key);
        let result = if change == EnvChange::Unchanged {
            Ok(())
        } else {
            write_env_file(&path, &content)
        };
        (path, change, result)
    });
    let (env_file, env_change, write_error) = match written {
        Some((path, change, Ok(()))) => (Some(path), Some(change), None),
        Some((path, _, Err(error))) => (
            None,
            None,
            Some(anyhow::anyhow!(
                "Created the key, but failed to write {}: {error}. Set {env_var} yourself from the key above.",
                path.display()
            )),
        ),
        None => (None, None, None),
    };
    let name = response.name.clone().or(args.name);

    if json {
        let payload = serde_json::json!({
            "schemaVersion": 1,
            "id": response.id,
            "name": name,
            "keyClass": response.key_class,
            "origins": [origin],
            "expiresAt": response.expires_at,
            "key": response.key,
            "framework": framework.id(),
            "envVar": env_var,
            "envFile": env_file.as_ref().map(|path| path.display().to_string()),
            "envFileChange": env_change.map(EnvChange::as_str),
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else {
        println!("{}", "✓ Publishable key created".green().bold());
        println!();
        println!(
            "{}",
            "Save this key now; it won't be shown again."
                .yellow()
                .bold()
        );
        println!();
        if let Some(name) = &name {
            println!("  Name:       {}", name);
        }
        println!("  Key ID:     {}", response.id);
        println!("  Type:       {}", "publishable".green());
        println!("  Origin:     {}", origin);
        println!(
            "  Expires:    {}",
            response
                .expires_at
                .split('T')
                .next()
                .unwrap_or(&response.expires_at)
        );
        println!();
        println!("  {}", "Publishable Key:".bold());
        println!("  {}", response.key.green().bold());
        println!();
        match (&env_file, env_change) {
            (Some(path), Some(change)) => println!(
                "  {} {env_var} in {} ({})",
                match change {
                    EnvChange::Created | EnvChange::Appended => "Wrote",
                    EnvChange::Replaced => "Replaced",
                    EnvChange::Unchanged => "Kept",
                },
                path.display(),
                framework.how()
            ),
            _ => println!(
                "  Set {} in your environment ({}), or rerun with --env-file .env.local",
                format!("{env_var}=<key>").cyan(),
                framework.how()
            ),
        }
        println!();
        println!(
            "{}",
            "This key is safe in browser code. Each publishable key allows exactly one origin;"
                .dimmed()
        );
        println!(
            "{}",
            "create another key for every other origin (e.g. production and localhost).".dimmed()
        );
    }

    match write_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

// ============================================================================
// Agent self-registration (WP9)
// ============================================================================

/// What `a4 auth signup` produced, before any printing.
#[derive(Debug)]
struct SignupOutcome {
    identity: AgentMeResponse,
    credentials_path: std::path::PathBuf,
    created: bool,
    idempotent: Option<bool>,
}

fn generate_agent_credential() -> String {
    let random = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    format!("a4_ak_{}", &random[..40])
}

fn pending_signup() -> PendingAgentSignup {
    PendingAgentSignup {
        credential: generate_agent_credential(),
        idempotency_key: uuid::Uuid::new_v4().to_string(),
    }
}

/// Register with `POST /api/agents/signup/v2`. Pending state is persisted
/// before the network request and reused after timeouts or process restarts.
fn perform_signup(
    client: &ApiClient,
    api_url: &str,
    name: Option<&str>,
    profile: &str,
    force: bool,
    if_missing: bool,
) -> Result<SignupOutcome> {
    if force && if_missing {
        anyhow::bail!("--force and --if-missing cannot be used together");
    }
    let _lock = ApiClient::lock_credentials()?;
    let existing = ApiClient::load_optional_api_key_for_profile(api_url, Some(profile))?;
    if let Some(existing) = existing {
        if if_missing {
            match ApiClient::with_base_url(api_url)
                .with_api_key(existing)
                .agent_me()
            {
                Ok(identity) => {
                    ApiClient::clear_pending_agent_signup(api_url, profile)?;
                    return Ok(SignupOutcome {
                        identity,
                        credentials_path: ApiClient::credentials_file_path()?,
                        created: false,
                        idempotent: None,
                    });
                }
                Err(error)
                    if matches!(
                        api_error_details(&error).map(|details| details.status),
                        Some(401 | 403)
                    ) =>
                {
                    anyhow::bail!(
                        "The stored `{profile}` credential for {api_url} is invalid or disabled ({error}). It was left untouched; rerun `a4 auth signup --force` to replace it"
                    );
                }
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!(
                            "Could not verify the stored `{profile}` credential for {api_url}; it was left untouched. Retry `a4 auth signup --if-missing`"
                        )
                    });
                }
            }
        } else if !force {
            anyhow::bail!(
                "Credentials already exist for profile `{profile}` at {api_url}. Run: a4 auth status (or pass --force to replace them)"
            );
        }
    }

    let pending = match ApiClient::load_pending_agent_signup(api_url, profile)? {
        Some(pending) => pending,
        None => {
            let pending = pending_signup();
            ApiClient::save_pending_agent_signup(api_url, profile, &pending)?;
            pending
        }
    };
    let response = client
        .agent_signup(name, &pending.credential, &pending.idempotency_key)
        .with_context(|| {
            "Agent signup did not complete. Pending state was retained; retry the same command safely"
        })?;
    if response.schema_version != 1 {
        anyhow::bail!(
            "Agent signup returned unsupported schema version {}; pending state was retained",
            response.schema_version
        );
    }
    let identity = ApiClient::with_base_url(api_url)
        .with_api_key(pending.credential.clone())
        .agent_me()
        .with_context(|| {
            "The new agent credential could not be verified. Pending state was retained; retry the same command safely"
        })?;
    if identity.slug != response.slug
        || identity.plan.as_deref() != Some(response.plan.as_str())
        || !signup_expiry_matches(
            identity.entitlement_expires_at.as_deref(),
            &response.entitlement_expires_at,
        )
        || identity.claim_state != response.claim_state
    {
        anyhow::bail!(
            "Agent signup verification did not match the signup response; pending state was retained"
        );
    }
    ApiClient::promote_pending_agent_signup(api_url, profile, &pending)?;
    let credentials_path = ApiClient::credentials_file_path()?;

    Ok(SignupOutcome {
        identity,
        credentials_path,
        created: true,
        idempotent: Some(response.idempotent),
    })
}

fn signup_json_payload(outcome: &SignupOutcome, profile: &str) -> serde_json::Value {
    serde_json::json!({
        "schemaVersion": 1,
        "slug": outcome.identity.slug,
        "displayName": outcome.identity.display_name,
        "createdAt": outcome.identity.created_at,
        "credentialStored": true,
        "credentialPathKind": if std::env::var_os("ARETE_CREDENTIALS_PATH").is_some() { "override" } else { "default" },
        "profile": profile,
        "accountStatus": outcome.identity.status,
        "plan": outcome.identity.plan,
        "entitlementExpiresAt": outcome.identity.entitlement_expires_at,
        "trialRemainingSeconds": trial_remaining_seconds(outcome.identity.entitlement_expires_at.as_deref()),
        "claimState": outcome.identity.claim_state,
        "trialAccessEnabled": outcome.identity.trial_access_enabled,
        "starterGuidance": outcome.identity.starter_guidance,
        "usage": outcome.identity.usage,
        "created": outcome.created,
        "idempotent": outcome.idempotent,
    })
}

/// `a4 auth signup`: agent self-registration (WP9).
///
/// Both human and JSON modes omit the key after it has been stored.
pub fn signup(
    name: Option<String>,
    force: bool,
    if_missing: bool,
    json: bool,
    requested_profile: Option<&str>,
) -> Result<()> {
    let api_url = config::get_api_url(None);
    let profile = requested_profile.unwrap_or(arete_mcp::credentials::AGENT_PROFILE);
    if profile != arete_mcp::credentials::AGENT_PROFILE {
        anyhow::bail!(
            "a4 auth signup creates an agent credential and must use profile `agent`; pass --profile agent or unset ARETE_PROFILE"
        );
    }
    let client = ApiClient::with_base_url(&api_url);

    let spinner = (!json).then(|| ui::create_spinner("Registering agent..."));
    let outcome = perform_signup(
        &client,
        &api_url,
        name.as_deref(),
        profile,
        force,
        if_missing,
    );
    if let Some(spinner) = spinner {
        spinner.finish_and_clear();
    }
    let outcome = outcome?;

    if json {
        let payload = signup_json_payload(&outcome, profile);
        println!("{}", serde_json::to_string(&payload)?);
        return Ok(());
    }

    ui::print_success(&format!(
        "{} agent {} ({})",
        if outcome.created {
            "Registered"
        } else {
            "Verified existing"
        },
        outcome.identity.slug.bold(),
        outcome.identity.display_name
    ));
    println!("  Target API:  {}", api_url.yellow());
    println!("  Profile:     {}", profile);
    println!(
        "  Credentials: {}",
        outcome.credentials_path.display().to_string().dimmed()
    );
    if let Some(plan) = &outcome.identity.plan {
        println!("  Plan:        {plan}");
    }
    if let Some(expires_at) = &outcome.identity.entitlement_expires_at {
        println!("  Expires:     {expires_at}");
        if let Some(remaining) = format_trial_remaining(Some(expires_at)) {
            println!("  Remaining:   {remaining}");
        }
    }
    println!();
    println!("Next: {}", "a4 explore --json".cyan());
    Ok(())
}

#[cfg(test)]
mod publishable_tests {
    use super::*;
    use crate::api_client::test_support::{MockServer, ENV_LOCK};

    #[test]
    fn framework_detection_prefers_next_then_vite() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(detect_framework(dir.path()), Framework::Generic);
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"devDependencies":{"vite":"^5"}}"#,
        )
        .unwrap();
        assert_eq!(detect_framework(dir.path()), Framework::Vite);
        assert_eq!(Framework::Vite.env_var(), "VITE_ARETE_PUBLISHABLE_KEY");
        std::fs::write(dir.path().join("next.config.mjs"), "export default {}").unwrap();
        assert_eq!(detect_framework(dir.path()), Framework::NextJs);
        assert_eq!(
            Framework::NextJs.env_var(),
            "NEXT_PUBLIC_ARETE_PUBLISHABLE_KEY"
        );
        assert_eq!(Framework::Generic.env_var(), "ARETE_PUBLISHABLE_KEY");
    }

    #[test]
    fn exactly_one_bare_origin_is_accepted() {
        assert_eq!(
            validate_origins(&["http://localhost:5173".into()]).unwrap(),
            "http://localhost:5173"
        );
        let many = validate_origins(&["https://a.test".into(), "https://b.test".into()])
            .unwrap_err()
            .to_string();
        assert!(many.contains("exactly one origin"), "{many}");
        assert!(validate_origins(&[]).is_err());
        assert!(validate_origins(&["example.com".into()]).is_err());
        let slash = validate_origins(&["https://example.com/".into()])
            .unwrap_err()
            .to_string();
        assert!(slash.contains("--origin https://example.com"), "{slash}");
    }

    #[test]
    fn env_file_paths_stay_in_the_project_unless_absolute() {
        let dir = tempfile::tempdir().unwrap();
        let project = std::fs::canonicalize(dir.path()).unwrap();
        assert_eq!(
            resolve_env_file(dir.path(), ".env.local").unwrap(),
            project.join(".env.local")
        );
        std::fs::create_dir_all(dir.path().join("apps/web")).unwrap();
        assert_eq!(
            resolve_env_file(dir.path(), "./apps/web/.env").unwrap(),
            project.join("apps/web/.env")
        );
        assert!(resolve_env_file(dir.path(), "../.env").is_err());
        assert!(resolve_env_file(dir.path(), "web/../../.env").is_err());
        assert!(resolve_env_file(dir.path(), " ").is_err());
        assert!(resolve_env_file(dir.path(), ".").is_err());
        let absolute = dir.path().join("elsewhere.env");
        assert_eq!(
            resolve_env_file(Path::new("/unrelated"), absolute.to_str().unwrap()).unwrap(),
            absolute
        );
    }

    #[cfg(unix)]
    #[test]
    fn env_file_paths_cannot_leave_the_project_through_a_symlink() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let project = std::fs::canonicalize(dir.path()).unwrap();
        symlink(outside.path(), dir.path().join("web")).unwrap();
        for raw in ["web/.env", "./web/.env"] {
            let error = resolve_env_file(dir.path(), raw).unwrap_err().to_string();
            assert!(error.contains("outside the project directory"), "{error}");
        }
        std::fs::create_dir_all(dir.path().join("apps/site")).unwrap();
        symlink(dir.path().join("apps"), dir.path().join("linked")).unwrap();
        assert_eq!(
            resolve_env_file(dir.path(), "linked/site/.env").unwrap(),
            project.join("apps/site/.env"),
            "a symlinked directory that stays inside the project is fine"
        );
        symlink(outside.path().join(".env"), dir.path().join(".env.local")).unwrap();
        let error = resolve_env_file(dir.path(), ".env.local")
            .unwrap_err()
            .to_string();
        assert!(error.contains("is a symlink"), "{error}");
        // An absolute path is explicit: it follows the link.
        std::fs::write(outside.path().join(".env"), "A=1\n").unwrap();
        let linked = dir.path().join(".env.local");
        assert_eq!(
            resolve_env_file(dir.path(), linked.to_str().unwrap()).unwrap(),
            std::fs::canonicalize(outside.path().join(".env")).unwrap()
        );
    }

    #[cfg(unix)]
    #[test]
    fn rewriting_an_env_file_keeps_its_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".env.local");
        for mode in [0o600, 0o640, 0o604] {
            std::fs::write(&path, "SECRET=keep\n").unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
            write_env_file(&path, "SECRET=keep\nA=1\n").unwrap();
            assert_eq!(
                std::fs::read_to_string(&path).unwrap(),
                "SECRET=keep\nA=1\n"
            );
            let actual = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(actual, mode, "{actual:o} != {mode:o}");
        }
        let entries = std::fs::read_dir(dir.path()).unwrap().count();
        assert_eq!(entries, 1, "no temporary file is left behind");
    }

    #[test]
    fn env_upsert_touches_only_the_variable() {
        let name = "VITE_ARETE_PUBLISHABLE_KEY";
        assert_eq!(
            upsert_env_var(None, name, "hspk_new"),
            (format!("{name}=hspk_new\n"), EnvChange::Created)
        );
        let existing = "SECRET=keep\r\n  export VITE_ARETE_PUBLISHABLE_KEY = hspk_old\r\nVITE_ARETE_PUBLISHABLE_KEY_OTHER=x\r\n";
        let (content, change) = upsert_env_var(Some(existing), name, "hspk_new");
        assert_eq!(change, EnvChange::Replaced);
        assert_eq!(
            content,
            "SECRET=keep\r\n  export VITE_ARETE_PUBLISHABLE_KEY=hspk_new\r\nVITE_ARETE_PUBLISHABLE_KEY_OTHER=x\r\n"
        );
        let (content, change) = upsert_env_var(Some("A=1"), name, "hspk_new");
        assert_eq!(change, EnvChange::Appended);
        assert_eq!(content, format!("A=1\n{name}=hspk_new\n"));
        let same = format!("{name}=hspk_new\n");
        assert_eq!(
            upsert_env_var(Some(&same), name, "hspk_new").1,
            EnvChange::Unchanged
        );
    }

    struct Sandbox {
        _guard: std::sync::MutexGuard<'static, ()>,
        dir: tempfile::TempDir,
    }

    impl Sandbox {
        fn new(server: &MockServer) -> Self {
            let guard = ENV_LOCK
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let dir = tempfile::tempdir().unwrap();
            let credentials = dir.path().join("credentials.toml");
            std::fs::write(
                &credentials,
                format!("[keys]\n\"{}\" = \"a4_sk_owner\"\n", server.base_url()),
            )
            .unwrap();
            std::env::set_var("ARETE_API_URL", server.base_url());
            std::env::set_var("ARETE_CREDENTIALS_PATH", &credentials);
            std::fs::write(
                dir.path().join("package.json"),
                r#"{"dependencies":{"vite":"^5"}}"#,
            )
            .unwrap();
            Self { _guard: guard, dir }
        }

        fn config(&self) -> String {
            self.dir.path().join("arete.toml").display().to_string()
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            std::env::remove_var("ARETE_API_URL");
            std::env::remove_var("ARETE_CREDENTIALS_PATH");
        }
    }

    fn args(env_file: Option<&str>) -> CreatePublishableArgs {
        CreatePublishableArgs {
            name: Some("web".into()),
            origins: vec!["http://localhost:5173".into()],
            expiry_days: None,
            env_file: env_file.map(str::to_string),
        }
    }

    #[test]
    fn api_failure_is_an_error_and_writes_nothing() {
        let server = MockServer::json(
            400,
            r#"{"error":"A publishable key must have exactly one allowed origin","code":"origin-allowlist-too-many"}"#,
        );
        let sandbox = Sandbox::new(&server);
        let env = sandbox.dir.path().join(".env.local");
        std::fs::write(&env, "OTHER=1\n").unwrap();
        let error = create_publishable_key(args(Some(".env.local")), &sandbox.config(), true)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("Failed to create publishable key")
                && error.contains("origin-allowlist-too-many"),
            "{error}"
        );
        assert_eq!(std::fs::read_to_string(&env).unwrap(), "OTHER=1\n");
    }

    #[test]
    fn created_key_is_written_to_the_env_file_for_the_detected_framework() {
        let server = MockServer::json(
            201,
            r#"{"id":7,"key":"hspk_fresh","name":"web","key_class":"publishable","expires_at":"2027-09-25T00:00:00Z","message":"ok"}"#,
        );
        let sandbox = Sandbox::new(&server);
        let env = sandbox.dir.path().join(".env.local");
        std::fs::write(&env, "OTHER=1\nVITE_ARETE_PUBLISHABLE_KEY=hspk_old\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&env, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        create_publishable_key(args(Some(".env.local")), &sandbox.config(), true).unwrap();
        assert_eq!(
            std::fs::read_to_string(&env).unwrap(),
            "OTHER=1\nVITE_ARETE_PUBLISHABLE_KEY=hspk_fresh\n"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&env).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "a private env file stays private");
        }
        let body: serde_json::Value = serde_json::from_str(&server.request().body).unwrap();
        assert_eq!(
            body["origin_allowlist"],
            serde_json::json!(["http://localhost:5173"])
        );
    }

    #[test]
    fn a_bad_env_file_path_fails_before_any_key_is_created() {
        let server = MockServer::json(500, r#"{"error":"must not be called"}"#);
        let sandbox = Sandbox::new(&server);
        let error = create_publishable_key(args(Some("missing-dir/.env")), &sandbox.config(), true)
            .unwrap_err()
            .to_string();
        assert!(error.contains("does not exist"), "{error}");
        let error = create_publishable_key(args(Some("../.env")), &sandbox.config(), true)
            .unwrap_err()
            .to_string();
        assert!(error.contains("outside the project"), "{error}");
        #[cfg(unix)]
        {
            let outside = tempfile::tempdir().unwrap();
            std::os::unix::fs::symlink(outside.path(), sandbox.dir.path().join("web")).unwrap();
            let error = create_publishable_key(args(Some("web/.env")), &sandbox.config(), true)
                .unwrap_err()
                .to_string();
            assert!(error.contains("outside the project"), "{error}");
            assert!(!outside.path().join(".env").exists());
        }
    }
}

#[cfg(test)]
mod signup_tests {
    use super::*;
    use crate::api_client::test_support::MockServer;
    use crate::api_client::ApiClientError;
    use std::sync::MutexGuard;

    /// `ARETE_CREDENTIALS_PATH` is process-global; serialise the tests that set it.
    use crate::api_client::test_support::ENV_LOCK;

    struct CredentialsSandbox {
        _guard: MutexGuard<'static, ()>,
        dir: tempfile::TempDir,
    }

    impl CredentialsSandbox {
        fn new() -> Self {
            let guard = ENV_LOCK
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let dir = tempfile::tempdir().expect("tempdir");
            std::env::set_var(
                "ARETE_CREDENTIALS_PATH",
                dir.path().join("creds").join("credentials.toml"),
            );
            CredentialsSandbox { _guard: guard, dir }
        }

        fn credentials_path(&self) -> std::path::PathBuf {
            self.dir.path().join("creds").join("credentials.toml")
        }
    }

    impl Drop for CredentialsSandbox {
        fn drop(&mut self) {
            std::env::remove_var("ARETE_CREDENTIALS_PATH");
        }
    }

    const OK_BODY: &str = r#"{"schemaVersion":1,"slug":"agent-7f3a","displayName":"Robo","createdAt":"2026-09-29T00:00:00Z","plan":"agent_trial","entitlementExpiresAt":"2026-10-06T00:00:00Z","claimState":"unclaimed","idempotent":false}"#;
    const ME_BODY: &str = r#"{"slug":"agent-7f3a","display_name":"Robo","status":"active","created_at":"2026-09-29T00:00:00Z","last_seen_at":null,"claimState":"unclaimed","plan":"agent_trial","entitlementExpiresAt":"2026-10-06T00:00:00Z","trialAccessEnabled":true,"starterGuidance":"Use starter stacks.","usage":{"window":"entitlement","windowStart":"2026-09-29T00:00:00Z","windowEnd":"2026-10-06T00:00:00Z","meters":[{"meter":"messages","consumed":7,"allowance":25,"remaining":18,"exhausted":false}],"exhausted":false}}"#;

    #[test]
    fn signup_rejects_non_agent_profile_before_registration() {
        let error = signup(None, false, false, true, Some("human")).unwrap_err();
        assert!(error.to_string().contains("must use profile `agent`"));
    }

    #[test]
    fn signup_stores_key_for_api_url_and_reports_slug() {
        let sandbox = CredentialsSandbox::new();
        let server =
            MockServer::json_sequence(vec![(200, OK_BODY.to_string()), (200, ME_BODY.to_string())]);
        let client = ApiClient::with_base_url(server.base_url());

        let outcome = perform_signup(
            &client,
            server.base_url(),
            Some("Robo"),
            "agent",
            false,
            false,
        )
        .expect("signup succeeds");

        assert_eq!(outcome.identity.slug, "agent-7f3a");
        assert_eq!(outcome.identity.display_name, "Robo");
        let usage = outcome.identity.usage.as_ref().expect("usage summary");
        assert!(!usage.exhausted);
        assert_eq!(usage.meters[0].remaining, 18);
        assert_eq!(outcome.credentials_path, sandbox.credentials_path());
        let stored_key = ApiClient::load_optional_api_key_for_url(server.base_url())
            .expect("credentials readable")
            .expect("key stored");
        let stored = std::fs::read_to_string(sandbox.credentials_path()).unwrap();
        assert!(stored.contains("[profiles.agent.keys]"), "{stored}");
        assert!(!stored.contains("pendingSignup"), "{stored}");
        let signup_request = server.request();
        let body: serde_json::Value =
            serde_json::from_str(&signup_request.body).expect("json body");
        assert_eq!(body["displayName"], "Robo");
        assert_eq!(body["credential"], stored_key);
        assert!(uuid::Uuid::parse_str(body["idempotencyKey"].as_str().unwrap()).is_ok());
        let me_request = server.request();
        assert_eq!(me_request.request_line, "GET /api/agents/me HTTP/1.1");
        assert_eq!(
            me_request.header("authorization"),
            Some(format!("Bearer {stored_key}").as_str())
        );
    }

    #[test]
    fn signup_accepts_database_precision_in_the_verified_expiry() {
        let _sandbox = CredentialsSandbox::new();
        let signup = OK_BODY.replace("2026-10-06T00:00:00Z", "2026-10-06T00:00:00.000000499Z");
        let server = MockServer::json_sequence(vec![(200, signup), (200, ME_BODY.to_string())]);
        let client = ApiClient::with_base_url(server.base_url());

        let outcome = perform_signup(
            &client,
            server.base_url(),
            Some("Robo"),
            "agent",
            false,
            false,
        )
        .expect("PostgreSQL microsecond precision verifies");

        assert!(outcome.created);
        assert_eq!(outcome.identity.slug, "agent-7f3a");
    }

    #[test]
    fn signup_accepts_database_precision_when_expiry_rounds_up() {
        let _sandbox = CredentialsSandbox::new();
        let signup = OK_BODY.replace("2026-10-06T00:00:00Z", "2026-10-06T00:00:00.000000501Z");
        let identity = ME_BODY.replace("2026-10-06T00:00:00Z", "2026-10-06T00:00:00.000001Z");
        let server = MockServer::json_sequence(vec![(200, signup), (200, identity)]);
        let client = ApiClient::with_base_url(server.base_url());

        let outcome = perform_signup(
            &client,
            server.base_url(),
            Some("Robo"),
            "agent",
            false,
            false,
        )
        .expect("PostgreSQL upward microsecond rounding verifies");

        assert!(outcome.created);
        assert_eq!(outcome.identity.slug, "agent-7f3a");
    }

    #[test]
    fn signup_refuses_to_replace_existing_credentials_unless_forced() {
        let _sandbox = CredentialsSandbox::new();
        let server =
            MockServer::json_sequence(vec![(200, OK_BODY.to_string()), (200, ME_BODY.to_string())]);
        let api_url = server.base_url().to_string();
        ApiClient::save_api_key("a4_ak_old", Some(&api_url)).expect("seed credentials");
        let credentials_path = ApiClient::credentials_file_path().unwrap();
        let mut duplicate = std::fs::read_to_string(&credentials_path).unwrap();
        duplicate.push_str(&format!("\"{api_url}/\" = \"a4_ak_stale\"\n"));
        std::fs::write(&credentials_path, duplicate).unwrap();
        let client = ApiClient::with_base_url(&api_url);

        let err = perform_signup(&client, &api_url, None, "agent", false, false)
            .expect_err("must refuse");
        assert_eq!(
            err.to_string(),
            format!(
                "Credentials already exist for profile `agent` at {api_url}. Run: a4 auth status (or pass --force to replace them)"
            )
        );
        assert_eq!(
            ApiClient::load_optional_api_key_for_url(&api_url)
                .unwrap()
                .as_deref(),
            Some("a4_ak_old"),
            "refusal must not touch the stored key"
        );

        perform_signup(&client, &api_url, None, "agent", true, false).expect("--force replaces");
        let replacement = ApiClient::load_optional_api_key_for_url(&api_url)
            .unwrap()
            .expect("replacement key");
        assert_ne!(replacement, "a4_ak_old");
        assert_eq!(
            ApiClient::load_optional_api_key_for_profile(&format!("{api_url}/"), Some("agent"))
                .unwrap()
                .as_deref(),
            Some(replacement.as_str()),
            "equivalent URL spellings must not retain a stale key"
        );
        let stored = std::fs::read_to_string(ApiClient::credentials_file_path().unwrap()).unwrap();
        let credentials = stored.parse::<toml::Value>().unwrap();
        let profile = credentials["profiles"]["agent"].as_table().unwrap();
        let active_keys = profile["keys"].as_table().unwrap();
        assert_eq!(active_keys.len(), 1, "{stored}");
        assert_eq!(
            active_keys.get(&api_url).and_then(toml::Value::as_str),
            Some(replacement.as_str()),
            "{stored}"
        );
        let backup_keys = profile["backupKeys"].as_table().unwrap();
        assert_eq!(
            backup_keys.get(&api_url).and_then(toml::Value::as_str),
            Some("a4_ak_old"),
            "{stored}"
        );
        assert_eq!(
            backup_keys
                .get(&format!("{api_url}/"))
                .and_then(toml::Value::as_str),
            Some("a4_ak_stale"),
            "{stored}"
        );
        let body: serde_json::Value =
            serde_json::from_str(&server.request().body).expect("json body");
        assert!(body.get("displayName").is_none());
        assert_eq!(body["credential"], replacement);
        let _ = server.request();
    }

    #[test]
    fn agent_and_human_profiles_coexist_without_implicit_selection() {
        let _sandbox = CredentialsSandbox::new();
        let api_url = "https://api.arete.run";
        ApiClient::save_api_key_for_profile("a4_ak_agent", Some(api_url), "agent").unwrap();
        ApiClient::save_api_key_for_profile("a4_sk_human", Some(api_url), "human").unwrap();

        assert_eq!(
            ApiClient::load_optional_api_key_for_profile(api_url, Some("agent"))
                .unwrap()
                .as_deref(),
            Some("a4_ak_agent")
        );
        assert_eq!(
            ApiClient::load_optional_api_key_for_profile(api_url, Some("human"))
                .unwrap()
                .as_deref(),
            Some("a4_sk_human")
        );
        assert!(ApiClient::load_optional_api_key_for_profile(api_url, None).is_err());

        ApiClient::delete_api_key_for_profile(api_url, Some("agent")).unwrap();
        assert_eq!(
            ApiClient::load_optional_api_key_for_profile(api_url, Some("agent")).unwrap(),
            None
        );
        assert_eq!(
            ApiClient::load_optional_api_key_for_profile(api_url, Some("human"))
                .unwrap()
                .as_deref(),
            Some("a4_sk_human")
        );
    }

    #[test]
    fn signup_preserves_rate_limit_problem_and_pending_retry_state() {
        let _sandbox = CredentialsSandbox::new();
        let server = MockServer::json(
            429,
            r#"{"schemaVersion":1,"error":"slow down","code":"rate_limit_exceeded","retryable":true,"retryAfterSeconds":60}"#,
        );
        let client = ApiClient::with_base_url(server.base_url());

        let err = perform_signup(&client, server.base_url(), None, "agent", false, false)
            .expect_err("429");
        let api_error = err
            .downcast_ref::<ApiClientError>()
            .expect("typed API error");
        assert_eq!(
            api_error.problem.code.as_deref(),
            Some("rate_limit_exceeded")
        );
        assert_eq!(api_error.retry_after_seconds(), Some(60));
        assert_eq!(
            ApiClient::load_optional_api_key_for_url(server.base_url()).unwrap(),
            None
        );
        assert!(
            ApiClient::load_pending_agent_signup(server.base_url(), "agent")
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn signup_reuses_pending_state_after_a_lost_response() {
        let _sandbox = CredentialsSandbox::new();
        let server = MockServer::json_sequence(vec![
            (
                500,
                r#"{"error":"response lost","code":"temporary","retryable":true}"#.to_string(),
            ),
            (200, OK_BODY.to_string()),
            (200, ME_BODY.to_string()),
        ]);
        let client = ApiClient::with_base_url(server.base_url());

        perform_signup(
            &client,
            server.base_url(),
            Some("Robo"),
            "agent",
            false,
            false,
        )
        .expect_err("first response is lost");
        let first: serde_json::Value =
            serde_json::from_str(&server.request().body).expect("first request JSON");
        let pending = ApiClient::load_pending_agent_signup(server.base_url(), "agent")
            .unwrap()
            .expect("pending state retained");
        assert_eq!(first["credential"], pending.credential);
        assert_eq!(first["idempotencyKey"], pending.idempotency_key);

        let outcome = perform_signup(
            &client,
            server.base_url(),
            Some("Robo"),
            "agent",
            false,
            false,
        )
        .expect("retry succeeds");
        assert!(outcome.created);
        let second: serde_json::Value =
            serde_json::from_str(&server.request().body).expect("second request JSON");
        assert_eq!(second["credential"], first["credential"]);
        assert_eq!(second["idempotencyKey"], first["idempotencyKey"]);
        let _ = server.request();
        assert!(
            ApiClient::load_pending_agent_signup(server.base_url(), "agent")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn signup_if_missing_verifies_and_reuses_an_existing_agent() {
        let _sandbox = CredentialsSandbox::new();
        let server = MockServer::json(200, ME_BODY);
        let api_url = server.base_url();
        ApiClient::save_api_key_for_profile("a4_ak_existing", Some(api_url), "agent").unwrap();
        ApiClient::save_pending_agent_signup(
            api_url,
            "agent",
            &PendingAgentSignup {
                credential: "a4_ak_0123456789012345678901234567890123456789".to_string(),
                idempotency_key: "11111111-1111-4111-8111-111111111111".to_string(),
            },
        )
        .unwrap();
        let client = ApiClient::with_base_url(api_url);

        let outcome = perform_signup(&client, api_url, None, "agent", false, true)
            .expect("existing key is reused");

        assert!(!outcome.created);
        assert_eq!(outcome.identity.slug, "agent-7f3a");
        assert_eq!(server.request().request_line, "GET /api/agents/me HTTP/1.1");
        assert!(ApiClient::load_pending_agent_signup(api_url, "agent")
            .unwrap()
            .is_none());
        assert_eq!(
            ApiClient::load_optional_api_key_for_profile(api_url, Some("agent"))
                .unwrap()
                .as_deref(),
            Some("a4_ak_existing")
        );
    }

    #[test]
    fn signup_if_missing_never_replaces_an_invalid_key_implicitly() {
        let _sandbox = CredentialsSandbox::new();
        let server = MockServer::json(401, r#"{"error":"disabled","code":"agent-key-disabled"}"#);
        let api_url = server.base_url();
        ApiClient::save_api_key_for_profile("a4_ak_disabled", Some(api_url), "agent").unwrap();
        let client = ApiClient::with_base_url(api_url);

        let error = perform_signup(&client, api_url, None, "agent", false, true)
            .expect_err("invalid key needs explicit force");

        assert!(error.to_string().contains("--force"));
        assert!(error.to_string().contains("left untouched"));
        assert_eq!(
            ApiClient::load_optional_api_key_for_profile(api_url, Some("agent"))
                .unwrap()
                .as_deref(),
            Some("a4_ak_disabled")
        );
    }

    #[test]
    fn signup_if_missing_keeps_a_key_on_transient_verification_failure() {
        let _sandbox = CredentialsSandbox::new();
        let server = MockServer::json(
            503,
            r#"{"error":"temporarily unavailable","code":"service-unavailable"}"#,
        );
        let api_url = server.base_url();
        ApiClient::save_api_key_for_profile("a4_ak_existing", Some(api_url), "agent").unwrap();
        let client = ApiClient::with_base_url(api_url);

        let error = perform_signup(&client, api_url, None, "agent", false, true)
            .expect_err("transient verification failure is surfaced");

        assert!(error.to_string().contains("Retry"), "{error:#}");
        assert!(!error.to_string().contains("--force"), "{error:#}");
        assert_eq!(
            ApiClient::load_optional_api_key_for_profile(api_url, Some("agent"))
                .unwrap()
                .as_deref(),
            Some("a4_ak_existing")
        );
    }

    #[test]
    fn short_positive_trial_time_is_not_reported_as_zero_minutes() {
        assert_eq!(format_remaining_seconds(1), "1s");
        assert_eq!(format_remaining_seconds(59), "59s");
        assert_eq!(format_remaining_seconds(60), "1m");
    }

    #[test]
    fn signup_json_never_contains_secrets_or_an_absolute_credentials_path() {
        let outcome = SignupOutcome {
            identity: serde_json::from_str(ME_BODY).unwrap(),
            credentials_path: std::path::PathBuf::from("/Users/example/.arete/credentials.toml"),
            created: true,
            idempotent: Some(false),
        };
        let encoded = serde_json::to_string(&signup_json_payload(&outcome, "agent")).unwrap();

        assert!(!encoded.contains("credential\""), "{encoded}");
        assert!(!encoded.contains("idempotency"), "{encoded}");
        assert!(!encoded.contains("/Users/example"), "{encoded}");
        assert!(encoded.contains("starterGuidance"), "{encoded}");
        assert!(encoded.contains("entitlementExpiresAt"), "{encoded}");
        assert!(encoded.contains("\"usage\""), "{encoded}");
    }

    #[test]
    fn credentials_lock_serializes_concurrent_signup_processes() {
        let _sandbox = CredentialsSandbox::new();
        let first = ApiClient::lock_credentials().expect("first lock");
        let (sender, receiver) = std::sync::mpsc::channel();
        let join = std::thread::spawn(move || {
            let second = ApiClient::lock_credentials().expect("second lock");
            sender.send(()).unwrap();
            drop(second);
        });

        assert!(receiver
            .recv_timeout(std::time::Duration::from_millis(100))
            .is_err());
        drop(first);
        receiver
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("second process acquires the released lock");
        join.join().unwrap();
    }

    #[test]
    fn generated_agent_credentials_match_the_server_contract() {
        let credential = generate_agent_credential();
        let body = credential.strip_prefix("a4_ak_").expect("agent prefix");
        assert_eq!(body.len(), 40);
        assert!(body.bytes().all(|byte| byte.is_ascii_alphanumeric()));
    }
}
