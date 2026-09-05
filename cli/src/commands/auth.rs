use anyhow::Result;
use colored::Colorize;
use std::io::{self, Write};

use crate::api_client::ApiClient;
use crate::config;
use crate::ui;

fn credentials_path() -> String {
    dirs::home_dir()
        .map(|home| {
            home.join(".arete")
                .join("credentials.toml")
                .display()
                .to_string()
        })
        .unwrap_or_else(|| "~/.arete/credentials.toml".to_string())
}

pub fn login(api_key: Option<String>) -> Result<()> {
    let api_url = config::get_api_url(None);

    let api_key = if let Some(key) = api_key {
        key
    } else {
        if !ui::interactive() {
            anyhow::bail!(
                "Missing --key and no terminal to prompt on. Pass: a4 auth login --key <a4_ak_...> (or register as an agent: a4 auth signup)"
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

    // Save the key with URL
    ApiClient::save_api_key(&api_key, Some(&api_url))?;

    // Verify the key works
    let spinner = ui::create_spinner("Verifying API key...");
    let client = ApiClient::new()?;

    match client.list_specs() {
        Ok(_) => {
            spinner.finish_and_clear();
            ui::print_success("API key saved and verified!");
            println!();
            println!("  Credentials: {}", credentials_path().dimmed());
            println!();
            println!("You are now ready to use Arete!");
        }
        Err(e) => {
            spinner.finish_and_clear();
            // Remove invalid key
            let _ = ApiClient::delete_api_key_for_url(&api_url);
            anyhow::bail!("Invalid API key: {}", e);
        }
    }

    Ok(())
}

pub fn logout() -> Result<()> {
    let api_url = config::get_api_url(None);

    let spinner = ui::create_spinner("Logging out...");

    // Delete specific URL credentials
    match ApiClient::delete_api_key_for_url(&api_url) {
        Ok(_) => {
            spinner.finish_and_clear();
            ui::print_success(&format!("Logged out from {}", api_url));
            println!("  Your credentials have been removed from this device.");
        }
        Err(_) => {
            // Try to delete all if specific fails
            let _ = ApiClient::delete_all_api_keys();
            spinner.finish_and_clear();
            ui::print_success("Logged out successfully");
            println!("  Your credentials have been removed from this device.");
        }
    }

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

pub fn status() -> Result<()> {
    let api_url = config::get_api_url(None);

    println!("{}", "Authentication Status".bold());
    println!();
    println!("Current target API: {}", api_url.yellow());
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
            for (url, _masked_key) in creds {
                let is_current = url == api_url
                    || (api_url.contains("localhost")
                        && (url.contains("localhost") || url.contains("127.0.0.1")));
                let marker = if is_current { "→ " } else { "  " };
                println!(
                    "{}{} {}",
                    marker,
                    url,
                    if is_current {
                        "(current)".green()
                    } else {
                        "".normal()
                    }
                );
            }
        }
        _ => {}
    }

    Ok(())
}

pub fn whoami() -> Result<()> {
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

    let spinner = ui::create_spinner("Verifying authentication...");
    let client = ApiClient::new()?;

    match client.list_specs() {
        Ok(specs) => {
            spinner.finish_and_clear();
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
            spinner.finish_and_clear();
            ui::print_error("API key invalid or expired");
            println!();
            println!("  Error: {}", e);
            println!();
            println!("  Run {} to re-authenticate.", "a4 auth login".cyan());
        }
    }

    Ok(())
}

// ============================================================================
// Publishable Key Management
// ============================================================================

pub fn list_keys() -> Result<()> {
    let client = ApiClient::new()?;

    let spinner = ui::create_spinner("Fetching API keys...");

    match client.list_api_keys() {
        Ok(keys) => {
            spinner.finish_and_clear();

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
        Err(e) => {
            spinner.finish_and_clear();
            ui::print_error(&format!("Failed to list keys: {}", e));
        }
    }

    Ok(())
}

pub fn create_publishable_key(
    name: Option<String>,
    origins: Vec<String>,
    expiry_days: Option<i64>,
) -> Result<()> {
    // Validate origins
    if origins.is_empty() {
        anyhow::bail!("At least one origin is required for publishable keys (e.g., https://example.com or http://localhost:5173)");
    }

    for origin in &origins {
        if !origin.starts_with("https://") && !origin.starts_with("http://") {
            anyhow::bail!(
                "Invalid origin '{}'. Origins must start with https:// or http://",
                origin
            );
        }
    }

    let client = ApiClient::new()?;

    let spinner = ui::create_spinner("Creating publishable key...");

    match client.create_publishable_key(name.clone(), origins.clone(), expiry_days) {
        Ok(response) => {
            spinner.finish_and_clear();

            println!(
                "{}",
                "✓ Publishable key created successfully!".green().bold()
            );
            println!();
            println!(
                "{}",
                "⚠️  IMPORTANT: Save this key now - it won't be shown again!"
                    .yellow()
                    .bold()
            );
            println!();

            if let Some(name) = &name {
                println!("  Name:       {}", name);
            }
            println!("  Key ID:     {}", response.id);
            println!("  Type:       {}", "publishable".green());
            println!("  Origins:    {}", origins.join(", "));
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
            println!(
                "{}",
                "This key is safe to use in browser/client-side code.".dimmed()
            );
            println!(
                "{}",
                "It can only access WebSocket endpoints from the allowed origins.".dimmed()
            );
        }
        Err(e) => {
            spinner.finish_and_clear();
            ui::print_error(&format!("Failed to create key: {}", e));
        }
    }

    Ok(())
}

// ============================================================================
// Agent self-registration (WP9)
// ============================================================================

/// What `a4 auth signup` produced, before any printing.
#[derive(Debug)]
struct SignupOutcome {
    slug: String,
    display_name: String,
    api_key: String,
    credentials_path: std::path::PathBuf,
    message: Option<String>,
}

/// Register with `POST /api/agents/signup` and store the issued key for
/// `api_url`. Refuses to overwrite existing credentials unless `force`.
fn perform_signup(
    client: &ApiClient,
    api_url: &str,
    name: Option<&str>,
    force: bool,
) -> Result<SignupOutcome> {
    if !force && ApiClient::load_optional_api_key_for_url(api_url)?.is_some() {
        anyhow::bail!(
            "Credentials already exist for {api_url}. Run: a4 auth status (or pass --force to replace them)"
        );
    }

    let response = client.agent_signup(name)?;
    ApiClient::save_api_key(&response.api_key, Some(api_url))?;
    let credentials_path = ApiClient::credentials_file_path()?;

    Ok(SignupOutcome {
        slug: response.slug,
        display_name: response.display_name,
        api_key: response.api_key,
        credentials_path,
        message: response.message,
    })
}

/// `a4 auth signup`: agent self-registration (WP9).
///
/// Human mode never prints the key; `--json` does (the agent may need it for
/// `ARETE_API_KEY` in a sub-process) and says so on stderr.
pub fn signup(name: Option<String>, force: bool, json: bool) -> Result<()> {
    let api_url = config::get_api_url(None);
    let client = ApiClient::new()?;

    let spinner = (!json).then(|| ui::create_spinner("Registering agent..."));
    let outcome = perform_signup(&client, &api_url, name.as_deref(), force);
    if let Some(spinner) = spinner {
        spinner.finish_and_clear();
    }
    let outcome = outcome?;

    if json {
        let payload = serde_json::json!({
            "schemaVersion": 1,
            "slug": outcome.slug,
            "displayName": outcome.display_name,
            "credentialsPath": outcome.credentials_path.display().to_string(),
            "apiKey": outcome.api_key,
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
        eprintln!(
            "note: apiKey is a secret; it is already stored in {}. Do not paste it into logs or chat.",
            outcome.credentials_path.display()
        );
        return Ok(());
    }

    ui::print_success(&format!(
        "Registered agent {} ({})",
        outcome.slug.bold(),
        outcome.display_name
    ));
    println!("  Target API:  {}", api_url.yellow());
    println!(
        "  Credentials: {}",
        outcome.credentials_path.display().to_string().dimmed()
    );
    if let Some(message) = outcome.message.filter(|m| !m.trim().is_empty()) {
        println!("  {}", message.dimmed());
    }
    println!();
    println!("Next: {}", "a4 explore --json".cyan());
    Ok(())
}

#[cfg(test)]
mod signup_tests {
    use super::*;
    use crate::api_client::test_support::MockServer;
    use crate::api_client::SIGNUP_RATE_LIMIT_MESSAGE;
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
    fn signup_stores_key_for_api_url_and_reports_slug() {
        let sandbox = CredentialsSandbox::new();
        let server = MockServer::json(200, OK_BODY);
        let client = ApiClient::with_base_url(server.base_url());

        let outcome = perform_signup(&client, server.base_url(), Some("Robo"), false)
            .expect("signup succeeds");

        assert_eq!(outcome.slug, "agent-7f3a");
        assert_eq!(outcome.display_name, "Robo");
        assert_eq!(outcome.api_key, "a4_ak_fresh");
        assert_eq!(outcome.message.as_deref(), Some("hi"));
        assert_eq!(outcome.credentials_path, sandbox.credentials_path());
        assert_eq!(
            ApiClient::load_optional_api_key_for_url(server.base_url())
                .expect("credentials readable")
                .as_deref(),
            Some("a4_ak_fresh")
        );
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

        let err = perform_signup(&client, &api_url, None, false).expect_err("must refuse");
        assert_eq!(
            err.to_string(),
            format!(
                "Credentials already exist for {api_url}. Run: a4 auth status (or pass --force to replace them)"
            )
        );
        assert_eq!(
            ApiClient::load_optional_api_key_for_url(&api_url)
                .unwrap()
                .as_deref(),
            Some("a4_ak_old"),
            "refusal must not touch the stored key"
        );

        let outcome = perform_signup(&client, &api_url, None, true).expect("--force replaces");
        assert_eq!(outcome.api_key, "a4_ak_fresh");
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
    fn signup_surfaces_rate_limit_message_and_stores_nothing() {
        let _sandbox = CredentialsSandbox::new();
        let server = MockServer::json(429, r#"{"error":"slow down"}"#);
        let client = ApiClient::with_base_url(server.base_url());

        let err = perform_signup(&client, server.base_url(), None, false).expect_err("429");
        assert_eq!(err.to_string(), SIGNUP_RATE_LIMIT_MESSAGE);
        assert_eq!(
            ApiClient::load_optional_api_key_for_url(server.base_url()).unwrap(),
            None
        );
    }
}
