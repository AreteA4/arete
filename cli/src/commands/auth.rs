use anyhow::Result;
use colored::Colorize;
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};

use crate::api_client::{AgentMeResponse, ApiClient};
use crate::config;
use crate::ui;

fn credentials_path() -> String {
    ApiClient::credentials_file_path()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|_| "~/.arete/credentials.toml".to_string())
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
            "trialAccessEnabled": identity.trial_access_enabled,
            "starterGuidance": identity.starter_guidance,
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
    slug: String,
    display_name: String,
    credentials_path: std::path::PathBuf,
}

/// Register with `POST /api/agents/signup` and store the issued key for
/// `api_url`. Refuses to overwrite existing credentials unless `force`.
fn perform_signup(
    client: &ApiClient,
    api_url: &str,
    name: Option<&str>,
    profile: &str,
    force: bool,
) -> Result<SignupOutcome> {
    if !force && ApiClient::load_optional_api_key_for_profile(api_url, Some(profile))?.is_some() {
        anyhow::bail!(
            "Credentials already exist for profile `{profile}` at {api_url}. Run: a4 auth status (or pass --force to replace them)"
        );
    }

    let response = client.agent_signup(name)?;
    ApiClient::save_api_key_for_profile(&response.api_key, Some(api_url), profile)?;
    let credentials_path = ApiClient::credentials_file_path()?;

    Ok(SignupOutcome {
        slug: response.slug,
        display_name: response.display_name,
        credentials_path,
    })
}

/// `a4 auth signup`: agent self-registration (WP9).
///
/// Both human and JSON modes omit the key after it has been stored.
pub fn signup(
    name: Option<String>,
    force: bool,
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
    let outcome = perform_signup(&client, &api_url, name.as_deref(), profile, force);
    if let Some(spinner) = spinner {
        spinner.finish_and_clear();
    }
    let outcome = outcome?;

    if json {
        let payload = serde_json::json!({
            "schemaVersion": 1,
            "slug": outcome.slug,
            "displayName": outcome.display_name,
            "credentialStored": true,
            "credentialPathKind": if std::env::var_os("ARETE_CREDENTIALS_PATH").is_some() { "override" } else { "default" },
            "profile": profile,
            "accountStatus": "active",
        });
        println!("{}", serde_json::to_string(&payload)?);
        return Ok(());
    }

    ui::print_success(&format!(
        "Registered agent {} ({})",
        outcome.slug.bold(),
        outcome.display_name
    ));
    println!("  Target API:  {}", api_url.yellow());
    println!("  Profile:     {}", profile);
    println!(
        "  Credentials: {}",
        outcome.credentials_path.display().to_string().dimmed()
    );
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

    const OK_BODY: &str =
        r#"{"slug":"agent-7f3a","display_name":"Robo","api_key":"a4_ak_fresh","message":"hi"}"#;

    #[test]
    fn signup_rejects_non_agent_profile_before_registration() {
        let error = signup(None, false, true, Some("human")).unwrap_err();
        assert!(error.to_string().contains("must use profile `agent`"));
    }

    #[test]
    fn signup_stores_key_for_api_url_and_reports_slug() {
        let sandbox = CredentialsSandbox::new();
        let server = MockServer::json(200, OK_BODY);
        let client = ApiClient::with_base_url(server.base_url());

        let outcome = perform_signup(&client, server.base_url(), Some("Robo"), "agent", false)
            .expect("signup succeeds");

        assert_eq!(outcome.slug, "agent-7f3a");
        assert_eq!(outcome.display_name, "Robo");
        assert_eq!(outcome.credentials_path, sandbox.credentials_path());
        assert_eq!(
            ApiClient::load_optional_api_key_for_url(server.base_url())
                .expect("credentials readable")
                .as_deref(),
            Some("a4_ak_fresh")
        );
        let stored = std::fs::read_to_string(sandbox.credentials_path()).unwrap();
        assert!(stored.contains("[profiles.agent.keys]"), "{stored}");
        assert!(stored.contains("a4_ak_fresh"), "{stored}");
        let body: serde_json::Value =
            serde_json::from_str(&server.request().body).expect("json body");
        assert_eq!(body, serde_json::json!({"display_name": "Robo"}));
    }

    #[test]
    fn signup_refuses_to_replace_existing_credentials_unless_forced() {
        let _sandbox = CredentialsSandbox::new();
        let server = MockServer::json(200, OK_BODY);
        let api_url = server.base_url().to_string();
        ApiClient::save_api_key("a4_ak_old", Some(&api_url)).expect("seed credentials");
        let client = ApiClient::with_base_url(&api_url);

        let err = perform_signup(&client, &api_url, None, "agent", false).expect_err("must refuse");
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

        perform_signup(&client, &api_url, None, "agent", true).expect("--force replaces");
        assert_eq!(
            ApiClient::load_optional_api_key_for_url(&api_url)
                .unwrap()
                .as_deref(),
            Some("a4_ak_fresh")
        );
        let body: serde_json::Value =
            serde_json::from_str(&server.request().body).expect("json body");
        assert_eq!(
            body,
            serde_json::json!({}),
            "display_name omitted when None"
        );
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
    fn signup_preserves_rate_limit_problem_and_stores_nothing() {
        let _sandbox = CredentialsSandbox::new();
        let server = MockServer::json(
            429,
            r#"{"schemaVersion":1,"error":"slow down","code":"rate_limit_exceeded","retryable":true,"retryAfterSeconds":60}"#,
        );
        let client = ApiClient::with_base_url(server.base_url());

        let err =
            perform_signup(&client, server.base_url(), None, "agent", false).expect_err("429");
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
    }
}
