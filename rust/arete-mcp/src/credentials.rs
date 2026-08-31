//! API key resolution for the `connect` tool.
//!
//! Agents should never have to paste API keys into tool calls — the key would
//! end up in the model's context window, chat transcript, and JSON-RPC wire.
//! Instead, `connect` resolves its api key through the following precedence:
//!
//! 1. **Explicit `api_key` argument** on the `connect` tool call (override,
//!    still supported for testing and multi-stack scenarios).
//! 2. **`ARETE_API_KEY` environment variable**, set once when launching
//!    the MCP server (e.g. in `.vscode/mcp.json`'s `env` block or via
//!    `claude mcp add -e ARETE_API_KEY=...`).
//! 3. **`~/.arete/credentials.toml`**, the file managed by
//!    `a4 auth login`. Two schemas are supported:
//!    - **New format:** `[keys]` table keyed by API URL
//!      (`https://api.arete.run`). Honors `ARETE_API_URL` for
//!      the lookup key.
//!    - **Legacy format:** a top-level `api_key = "..."` key. This is what
//!      older `a4 auth login` versions wrote and what many users still have.
//!
//! If none of the three produces a key **and** the target WebSocket URL is a
//! hosted Arete stack (ends in `.stack.arete.run`), this module
//! returns a descriptive error so the agent can tell the user what to do.
//! Self-hosted / custom stacks are allowed to proceed without a key because
//! they may not require auth at all.
//!
//! The resolver reads the outside world through the [`Env`] trait. Production
//! code uses [`SystemEnv`], which touches `std::env` and the filesystem.
//! Tests construct a `TestEnv` (see the tests module) with inline values so
//! they do not mutate process-global state — unit tests run in parallel by
//! default and `std::env::set_var` races are a real footgun.

use std::fs;
use std::path::PathBuf;

use anyhow::{anyhow, Result};
use serde::Deserialize;

const HOSTED_WEBSOCKET_SUFFIX: &str = ".stack.arete.run";
const DEFAULT_API_URL: &str = "https://api.arete.run";
const ENV_VAR_API_KEY: &str = "ARETE_API_KEY";
const ENV_VAR_API_URL: &str = "ARETE_API_URL";

/// Ambient environment the resolver reads from. Production code uses
/// [`SystemEnv`]; tests construct an inline struct implementing this trait.
pub trait Env {
    /// Look up an environment variable.
    fn var(&self, key: &str) -> Option<String>;
    /// Return the raw contents of the credentials file, if it exists and is
    /// readable. Errors (missing file, permission denied, bad UTF-8) map to
    /// `None` — the caller decides whether absence is fatal.
    fn credentials_file(&self) -> Option<String>;
    /// Display path for the credentials file, used only in error messages.
    /// Must never be called on test data in a way that leaks real paths.
    fn credentials_file_path_display(&self) -> String;
}

/// The real implementation used in the shipped binary.
pub struct SystemEnv;

impl Env for SystemEnv {
    fn var(&self, key: &str) -> Option<String> {
        std::env::var(key).ok()
    }

    fn credentials_file(&self) -> Option<String> {
        fs::read_to_string(system_credentials_path()?).ok()
    }

    fn credentials_file_path_display(&self) -> String {
        system_credentials_path()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "~/.arete/credentials.toml".to_string())
    }
}

fn system_credentials_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".arete").join("credentials.toml"))
}

/// Describes where a resolved api key came from. Useful for log lines and the
/// `connect` tool response so users can see which source won without revealing
/// the key itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeySource {
    Explicit,
    EnvVar,
    CredentialsFile,
    None,
}

impl KeySource {
    pub fn as_str(self) -> &'static str {
        match self {
            KeySource::Explicit => "explicit_argument",
            KeySource::EnvVar => "env:ARETE_API_KEY",
            KeySource::CredentialsFile => "~/.arete/credentials.toml",
            KeySource::None => "none",
        }
    }
}

/// Result of a key lookup. The `key` is `None` only for self-hosted URLs
/// where proceeding without auth is legitimate.
#[derive(Debug, Clone)]
pub struct ResolvedKey {
    pub key: Option<String>,
    pub source: KeySource,
}

/// Resolve the api key to use for a `connect` call to `url`. Thin wrapper
/// around [`resolve_with`] that uses [`SystemEnv`].
pub fn resolve(explicit: Option<String>, url: &str) -> Result<ResolvedKey> {
    resolve_with(&SystemEnv, explicit, url)
}

/// Generic resolver parametrized over the ambient environment. Tests use this
/// directly with a `TestEnv` to avoid touching process-global state.
pub fn resolve_with<E: Env>(env: &E, explicit: Option<String>, url: &str) -> Result<ResolvedKey> {
    // 1. Explicit argument wins. Trim to protect against accidental whitespace.
    if let Some(k) = explicit
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        return Ok(ResolvedKey {
            key: Some(k.to_string()),
            source: KeySource::Explicit,
        });
    }

    // 2. Environment variable.
    if let Some(k) = env.var(ENV_VAR_API_KEY) {
        let k = k.trim().to_string();
        if !k.is_empty() {
            return Ok(ResolvedKey {
                key: Some(k),
                source: KeySource::EnvVar,
            });
        }
    }

    // 3. Credentials file. The lookup URL mirrors `RegistryClient::new`
    // exactly: trim, strip trailing slashes, and fall back to the default
    // when the env var is effectively empty — otherwise an
    // `ARETE_API_URL` of whitespace/slashes would send requests to the
    // default host while looking up credentials under an empty key.
    if let Some(content) = env.credentials_file() {
        let api_url = env
            .var(ENV_VAR_API_URL)
            .map(|u| normalize_api_url(&u).to_string())
            .filter(|u| !u.is_empty())
            .unwrap_or_else(|| DEFAULT_API_URL.to_string());
        if let Some(k) = parse_credentials_content(&content, &api_url) {
            return Ok(ResolvedKey {
                key: Some(k),
                source: KeySource::CredentialsFile,
            });
        }
    }

    // Nothing found. Decide whether that's fatal.
    if is_hosted_websocket_url(url) {
        let file = env.credentials_file_path_display();
        Err(anyhow!(
            "no Arete api key found for hosted stack `{url}`. \
             Tried: explicit `api_key` argument, `{ENV_VAR_API_KEY}` env var, and {file}. \
             Fix: run `a4 auth login`, or set `{ENV_VAR_API_KEY}=a4_sk_...` (or legacy `hsk_...`) in your MCP \
             server environment (e.g. `.vscode/mcp.json` `env` block), or pass \
             `api_key` explicitly on the connect call."
        ))
    } else {
        Ok(ResolvedKey {
            key: None,
            source: KeySource::None,
        })
    }
}

/// Whether the URL points at a Arete-hosted WebSocket endpoint.
/// Mirrors `arete_sdk::auth::is_hosted_arete_websocket_url`, which is
/// `pub(crate)` in the SDK and not reachable from here. Kept in sync with the
/// SDK's `HOSTED_WEBSOCKET_SUFFIX` constant.
fn is_hosted_websocket_url(url: &str) -> bool {
    let rest = url
        .strip_prefix("wss://")
        .or_else(|| url.strip_prefix("ws://"))
        .unwrap_or(url);
    let host_end = rest.find(['/', ':', '?', '#']).unwrap_or(rest.len());
    rest[..host_end].ends_with(HOSTED_WEBSOCKET_SUFFIX)
}

/// Parse a credentials.toml body and return a key if either supported schema
/// matches. Pure function — easy to unit-test without touching the filesystem.
/// Normalize an API URL for credential lookup with the same rule
/// `RegistryClient` applies to its request destination (trim whitespace,
/// drop trailing slashes). Both sides of the `[keys]` match go through this
/// so `ARETE_API_URL="https://api.arete.run/"` still finds the key stored
/// under `"https://api.arete.run"` and vice versa.
fn normalize_api_url(url: &str) -> &str {
    url.trim().trim_end_matches('/')
}

fn parse_credentials_content(content: &str, api_url: &str) -> Option<String> {
    let wanted = normalize_api_url(api_url);
    // Try the new format first.
    if let Ok(parsed) = toml::from_str::<NewFormat>(content) {
        if let Some(keys) = parsed.keys {
            let matched = keys
                .iter()
                .find(|(url, _)| normalize_api_url(url) == wanted)
                .map(|(_, key)| key.trim())
                .filter(|k| !k.is_empty());
            if let Some(k) = matched {
                return Some(k.to_string());
            }
        }
    }

    // Fall back to legacy top-level `api_key = "..."`.
    if let Ok(parsed) = toml::from_str::<LegacyFormat>(content) {
        if let Some(k) = parsed.api_key {
            let k = k.trim().to_string();
            if !k.is_empty() {
                return Some(k);
            }
        }
    }

    None
}

#[derive(Deserialize)]
struct NewFormat {
    keys: Option<std::collections::HashMap<String, String>>,
}

#[derive(Deserialize)]
struct LegacyFormat {
    api_key: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Hermetic environment for tests. No global state — each test builds one
    /// inline, so `cargo test` can run them in parallel without racing.
    #[derive(Default)]
    struct TestEnv {
        vars: HashMap<String, String>,
        credentials: Option<String>,
    }

    impl TestEnv {
        fn with_var(mut self, key: &str, value: &str) -> Self {
            self.vars.insert(key.to_string(), value.to_string());
            self
        }

        fn with_credentials(mut self, content: &str) -> Self {
            self.credentials = Some(content.to_string());
            self
        }
    }

    impl Env for TestEnv {
        fn var(&self, key: &str) -> Option<String> {
            self.vars.get(key).cloned()
        }
        fn credentials_file(&self) -> Option<String> {
            self.credentials.clone()
        }
        fn credentials_file_path_display(&self) -> String {
            "<test:~/.arete/credentials.toml>".to_string()
        }
    }

    // ── Precedence ──────────────────────────────────────────────────────────

    #[test]
    fn explicit_argument_wins_over_env_and_file() {
        let env = TestEnv::default()
            .with_var(ENV_VAR_API_KEY, "a4_sk_from_env")
            .with_credentials("api_key = \"a4_sk_from_file\"");
        let r = resolve_with(
            &env,
            Some("a4_sk_explicit".into()),
            "wss://foo.stack.arete.run",
        )
        .unwrap();
        assert_eq!(r.source, KeySource::Explicit);
        assert_eq!(r.key.as_deref(), Some("a4_sk_explicit"));
    }

    #[test]
    fn env_var_wins_over_file() {
        let env = TestEnv::default()
            .with_var(ENV_VAR_API_KEY, "a4_sk_from_env")
            .with_credentials("api_key = \"a4_sk_from_file\"");
        let r = resolve_with(&env, None, "wss://foo.stack.arete.run").unwrap();
        assert_eq!(r.source, KeySource::EnvVar);
        assert_eq!(r.key.as_deref(), Some("a4_sk_from_env"));
    }

    #[test]
    fn url_keyed_lookup_normalizes_env_url() {
        // ARETE_API_URL with trailing slash + whitespace must still find the
        // key stored under the normalized URL (RegistryClient normalizes the
        // request destination the same way).
        let env = TestEnv::default()
            .with_var(ENV_VAR_API_URL, " https://api.arete.run/ ")
            .with_credentials("[keys]\n\"https://api.arete.run\" = \"a4_sk_url\"");
        let r = resolve_with(&env, None, "").unwrap();
        assert_eq!(r.source, KeySource::CredentialsFile);
        assert_eq!(r.key.as_deref(), Some("a4_sk_url"));
    }

    #[test]
    fn url_keyed_lookup_normalizes_file_key() {
        // Symmetric case: the credentials.toml entry carries the trailing
        // slash instead.
        let env = TestEnv::default()
            .with_var(ENV_VAR_API_URL, "https://api.arete.run")
            .with_credentials("[keys]\n\"https://api.arete.run/\" = \"a4_sk_url\"");
        let r = resolve_with(&env, None, "").unwrap();
        assert_eq!(r.source, KeySource::CredentialsFile);
        assert_eq!(r.key.as_deref(), Some("a4_sk_url"));
    }

    #[test]
    fn url_keyed_lookup_falls_back_to_default_on_effectively_empty_env_url() {
        // Whitespace/slashes-only ARETE_API_URL makes RegistryClient target
        // the default host; the credentials lookup must follow it there
        // instead of searching under an empty key.
        let env = TestEnv::default()
            .with_var(ENV_VAR_API_URL, "  / ")
            .with_credentials(&format!("[keys]\n\"{DEFAULT_API_URL}\" = \"a4_sk_url\""));
        let r = resolve_with(&env, None, "").unwrap();
        assert_eq!(r.source, KeySource::CredentialsFile);
        assert_eq!(r.key.as_deref(), Some("a4_sk_url"));
    }

    #[test]
    fn file_used_when_nothing_else_available() {
        let env = TestEnv::default().with_credentials("api_key = \"a4_sk_from_file\"");
        let r = resolve_with(&env, None, "wss://foo.stack.arete.run").unwrap();
        assert_eq!(r.source, KeySource::CredentialsFile);
        assert_eq!(r.key.as_deref(), Some("a4_sk_from_file"));
    }

    #[test]
    fn whitespace_only_explicit_falls_through_to_env() {
        let env = TestEnv::default().with_var(ENV_VAR_API_KEY, "a4_sk_from_env");
        let r = resolve_with(&env, Some("  ".into()), "wss://foo.stack.arete.run").unwrap();
        assert_eq!(r.source, KeySource::EnvVar);
    }

    #[test]
    fn whitespace_only_env_falls_through_to_file() {
        let env = TestEnv::default()
            .with_var(ENV_VAR_API_KEY, "   ")
            .with_credentials("api_key = \"a4_sk_from_file\"");
        let r = resolve_with(&env, None, "wss://foo.stack.arete.run").unwrap();
        assert_eq!(r.source, KeySource::CredentialsFile);
    }

    // ── File schemas ────────────────────────────────────────────────────────

    #[test]
    fn parses_legacy_top_level_api_key() {
        assert_eq!(
            parse_credentials_content("api_key = \"a4_sk_legacy\"\n", DEFAULT_API_URL).as_deref(),
            Some("a4_sk_legacy")
        );
    }

    #[test]
    fn parses_new_url_keyed_table() {
        let content = "[keys]\n\
                       \"https://api.arete.run\" = \"a4_sk_new\"\n";
        assert_eq!(
            parse_credentials_content(content, "https://api.arete.run").as_deref(),
            Some("a4_sk_new")
        );
    }

    #[test]
    fn new_format_respects_api_url_selector() {
        let content = "[keys]\n\
                       \"https://api.arete.run\" = \"a4_sk_prod\"\n\
                       \"http://localhost:3000\" = \"a4_sk_local\"\n";
        assert_eq!(
            parse_credentials_content(content, "http://localhost:3000").as_deref(),
            Some("a4_sk_local")
        );
    }

    #[test]
    fn new_format_returns_none_when_url_not_listed() {
        let content = "[keys]\n\
                       \"https://api.arete.run\" = \"a4_sk_prod\"\n";
        assert_eq!(
            parse_credentials_content(content, "https://not-listed.example"),
            None
        );
    }

    #[test]
    fn env_api_url_overrides_default_lookup() {
        let env = TestEnv::default()
            .with_var(ENV_VAR_API_URL, "http://localhost:3000")
            .with_credentials(
                "[keys]\n\"http://localhost:3000\" = \"a4_sk_local\"\n\
                 \"https://api.arete.run\" = \"a4_sk_prod\"\n",
            );
        let r = resolve_with(&env, None, "wss://foo.stack.arete.run").unwrap();
        assert_eq!(r.source, KeySource::CredentialsFile);
        assert_eq!(r.key.as_deref(), Some("a4_sk_local"));
    }

    #[test]
    fn empty_file_returns_none() {
        let env = TestEnv::default().with_credentials("");
        let r = resolve_with(&env, None, "wss://self.hosted.example").unwrap();
        assert_eq!(r.source, KeySource::None);
        assert!(r.key.is_none());
    }

    #[test]
    fn unparseable_file_returns_none() {
        let env = TestEnv::default().with_credentials("not valid toml {{{");
        let r = resolve_with(&env, None, "wss://self.hosted.example").unwrap();
        assert_eq!(r.source, KeySource::None);
    }

    // ── Hosted vs self-hosted failure modes ─────────────────────────────────

    #[test]
    fn hosted_url_without_any_key_is_error() {
        let env = TestEnv::default();
        let err = resolve_with(&env, None, "wss://any.stack.arete.run").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("no Arete api key"), "{msg}");
        assert!(msg.contains("a4 auth login"), "{msg}");
        assert!(msg.contains("ARETE_API_KEY"), "{msg}");
        // Should include the test env's display path, not a real $HOME.
        assert!(msg.contains("<test:"), "{msg}");
    }

    #[test]
    fn self_hosted_url_without_key_is_ok_with_none() {
        let env = TestEnv::default();
        let r = resolve_with(&env, None, "wss://my.self.hosted.example").unwrap();
        assert_eq!(r.source, KeySource::None);
        assert!(r.key.is_none());
    }

    // ── Host detection ──────────────────────────────────────────────────────

    #[test]
    fn hosted_url_detection() {
        assert!(is_hosted_websocket_url("wss://foo.stack.arete.run"));
        assert!(is_hosted_websocket_url("wss://a-b-c.stack.arete.run"));
        assert!(is_hosted_websocket_url("wss://a.stack.arete.run/socket"));
        assert!(is_hosted_websocket_url("wss://a.stack.arete.run:443"));
        assert!(!is_hosted_websocket_url("wss://example.com"));
        assert!(!is_hosted_websocket_url("ws://localhost:8878"));
        assert!(!is_hosted_websocket_url("not a url"));
    }
}
