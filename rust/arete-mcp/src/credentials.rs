//! Shared API-key resolution for the MCP server and `a4` CLI.
//!
//! Agents should never have to paste API keys into tool calls — the key would
//! end up in the model's context window, chat transcript, and JSON-RPC wire.
//! Instead, `connect` resolves its api key through the following precedence:
//!
//! 1. **Explicit `api_key` argument** on the `connect` tool call (override,
//!    still supported for testing and multi-stack scenarios, but constrained
//!    to the selected built-in profile when one is set).
//! 2. When `ARETE_PROFILE` is selected, that named profile in the credentials
//!    file. A selected profile deliberately outranks `ARETE_API_KEY`, so an
//!    ambient human key cannot override an MCP pinned to the agent profile.
//! 3. Without a selected profile, **`ARETE_API_KEY`**.
//! 4. **`~/.arete/credentials.toml`** (or `ARETE_CREDENTIALS_PATH`). Named
//!    `[profiles.<name>.keys]`, URL-keyed `[keys]`, and legacy top-level
//!    `api_key` schemas are supported.
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

// Profile parsing and lookup live in the SDK so the CLI, this server and SDK
// scripts resolve `a4` logins identically.
pub use arete_sdk::credentials::{
    inferred_profile_for_key, lookup_credentials, normalize_api_url, validate_key_for_profile,
    validate_profile_name, CredentialLookup, AGENT_PROFILE, DEFAULT_API_URL,
    ENV_VAR_CREDENTIALS_PATH, ENV_VAR_PROFILE, HUMAN_PROFILE,
};

pub const ENV_VAR_API_KEY: &str = "ARETE_API_KEY";
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
}

fn system_credentials_path() -> Option<PathBuf> {
    arete_sdk::credentials::credentials_path(|name| std::env::var(name).ok())
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
            // Agent-visible: name the source, never where credentials live.
            KeySource::CredentialsFile => "a4-login",
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
    let selected_profile = env
        .var(ENV_VAR_PROFILE)
        .map(|profile| validate_profile_name(&profile).map(str::to_string))
        .transpose()?;

    // 1. Explicit argument wins. Trim to protect against accidental whitespace.
    if let Some(k) = explicit
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        if let Some(profile) = selected_profile.as_deref() {
            validate_key_for_profile(profile, k)?;
        }
        return Ok(ResolvedKey {
            key: Some(k.to_string()),
            source: KeySource::Explicit,
        });
    }

    // 2. A selected profile is fail-closed and deliberately wins over an
    // ambient ARETE_API_KEY. Generated MCP config always selects `agent`.
    if let (Some(profile), Some(content)) = (selected_profile.as_deref(), env.credentials_file()) {
        let api_url = selected_api_url(env);
        let lookup = lookup_credentials(&content, &api_url, Some(profile))?;
        if let Some(key) = lookup.key {
            return Ok(ResolvedKey {
                key: Some(key),
                source: KeySource::CredentialsFile,
            });
        }
    }

    // 3. The legacy environment override is used only when no profile was
    // selected. This preserves compatibility without allowing a human key to
    // override an explicitly agent-scoped process.
    if selected_profile.is_none() {
        if let Some(k) = env.var(ENV_VAR_API_KEY) {
            let k = k.trim().to_string();
            if !k.is_empty() {
                return Ok(ResolvedKey {
                    key: Some(k),
                    source: KeySource::EnvVar,
                });
            }
        }
    }

    // 4. Credentials file. The lookup URL mirrors `RegistryClient::new`
    // exactly: trim, strip trailing slashes, and fall back to the default
    // when the env var is effectively empty — otherwise an
    // `ARETE_API_URL` of whitespace/slashes would send requests to the
    // default host while looking up credentials under an empty key.
    if selected_profile.is_none() {
        if let Some(content) = env.credentials_file() {
            let api_url = selected_api_url(env);
            if let Some(k) = lookup_credentials(&content, &api_url, None)?.key {
                return Ok(ResolvedKey {
                    key: Some(k),
                    source: KeySource::CredentialsFile,
                });
            }
        }
    }

    // Nothing found. Decide whether that's fatal.
    if is_hosted_websocket_url(url) {
        // Agent-visible: say what was checked, never where credentials live.
        let api_url = selected_api_url(env);
        let checked = match selected_profile.as_deref() {
            Some(profile) => format!("the a4 login (profile `{profile}`, API {api_url})"),
            None => format!("{ENV_VAR_API_KEY} or the a4 login (API {api_url})"),
        };
        Err(anyhow!(
            "No Arete API key found in {checked} for hosted stack `{url}`. Run \
             `a4 auth signup` (agent) or `a4 auth login`, or set {ENV_VAR_API_KEY}."
        ))
    } else {
        Ok(ResolvedKey {
            key: None,
            source: KeySource::None,
        })
    }
}

fn selected_api_url<E: Env>(env: &E) -> String {
    env.var(ENV_VAR_API_URL)
        .map(|u| normalize_api_url(&u))
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| DEFAULT_API_URL.to_string())
}

/// Whether the URL points at a Arete-hosted WebSocket endpoint.
/// Defers to the SDK so the set of hosted suffixes is decided in one place;
/// this used to keep its own copy of the constant because the SDK's check was
/// `pub(crate)`, which meant the two could drift.
fn is_hosted_websocket_url(url: &str) -> bool {
    let rest = url
        .strip_prefix("wss://")
        .or_else(|| url.strip_prefix("ws://"))
        .unwrap_or(url);
    let host_end = rest.find(['/', ':', '?', '#']).unwrap_or(rest.len());
    arete_sdk::is_hosted_websocket_host(&rest[..host_end])
}

#[cfg(test)]
fn parse_credentials_content(content: &str, api_url: &str) -> Option<String> {
    lookup_credentials(content, api_url, None)
        .ok()
        .and_then(|lookup| lookup.key)
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
    fn selected_agent_profile_wins_over_ambient_human_key() {
        let env = TestEnv::default()
            .with_var(ENV_VAR_PROFILE, AGENT_PROFILE)
            .with_var(ENV_VAR_API_KEY, "a4_sk_human")
            .with_credentials(
                "[profiles.agent.keys]\n\
                 \"https://api.arete.run\" = \"a4_ak_agent\"\n\
                 [profiles.human.keys]\n\
                 \"https://api.arete.run\" = \"a4_sk_human_file\"",
            );
        let resolved = resolve_with(&env, None, "wss://foo.stack.arete.run").unwrap();
        assert_eq!(resolved.source, KeySource::CredentialsFile);
        assert_eq!(resolved.key.as_deref(), Some("a4_ak_agent"));
    }

    #[test]
    fn selected_agent_profile_does_not_fall_back_to_human_credentials() {
        let env = TestEnv::default()
            .with_var(ENV_VAR_PROFILE, AGENT_PROFILE)
            .with_var(ENV_VAR_API_KEY, "a4_sk_human")
            .with_credentials(
                "[profiles.human.keys]\n\
                 \"https://api.arete.run\" = \"a4_sk_human_file\"",
            );
        let error = resolve_with(&env, None, "wss://foo.stack.arete.run").unwrap_err();
        assert!(error.to_string().contains("profile `agent`"));
    }

    #[test]
    fn selected_agent_profile_rejects_explicit_human_key() {
        let env = TestEnv::default().with_var(ENV_VAR_PROFILE, AGENT_PROFILE);
        let error = resolve_with(
            &env,
            Some("a4_sk_human".to_string()),
            "wss://foo.stack.arete.run",
        )
        .unwrap_err();
        assert!(error.to_string().contains("requires an a4_ak_*"));
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
    fn url_keyed_lookup_folds_env_url_host_case() {
        // DNS hosts and URL schemes are case-insensitive (RFC 3986): a
        // case-variant ARETE_API_URL must still find the stored key.
        let env = TestEnv::default()
            .with_var(ENV_VAR_API_URL, "HTTPS://API.Arete.Run")
            .with_credentials("[keys]\n\"https://api.arete.run\" = \"a4_sk_url\"");
        let r = resolve_with(&env, None, "").unwrap();
        assert_eq!(r.source, KeySource::CredentialsFile);
        assert_eq!(r.key.as_deref(), Some("a4_sk_url"));
    }

    #[test]
    fn url_keyed_lookup_folds_file_key_host_case() {
        // Symmetric case: the credentials.toml entry carries the loud casing.
        let env = TestEnv::default()
            .with_var(ENV_VAR_API_URL, "https://api.arete.run")
            .with_credentials("[keys]\n\"https://API.Arete.Run\" = \"a4_sk_url\"");
        let r = resolve_with(&env, None, "").unwrap();
        assert_eq!(r.source, KeySource::CredentialsFile);
        assert_eq!(r.key.as_deref(), Some("a4_sk_url"));
    }

    #[test]
    fn url_keyed_lookup_keeps_path_case_sensitive() {
        // The host folds, but the path is case-sensitive: `/API` must not
        // match a key stored under `/api`.
        let env = TestEnv::default()
            .with_var(ENV_VAR_API_URL, "http://LOCALHOST:3000/API")
            .with_credentials(
                "[keys]\n\
                 \"http://localhost:3000/API\" = \"a4_sk_upper\"\n\
                 \"http://localhost:3000/api\" = \"a4_sk_lower\"",
            );
        let r = resolve_with(&env, None, "").unwrap();
        assert_eq!(r.key.as_deref(), Some("a4_sk_upper"));
    }

    #[test]
    fn url_keyed_lookup_strips_fqdn_trailing_dot() {
        // `api.arete.run.` is the fully-qualified spelling of
        // `api.arete.run` (trailing dot = DNS root label). The registry
        // origin check strips it; the credentials lookup must follow or the
        // stored key is missed.
        let env = TestEnv::default()
            .with_var(ENV_VAR_API_URL, "https://api.arete.run.")
            .with_credentials("[keys]\n\"https://api.arete.run\" = \"a4_sk_url\"");
        let r = resolve_with(&env, None, "").unwrap();
        assert_eq!(r.source, KeySource::CredentialsFile);
        assert_eq!(r.key.as_deref(), Some("a4_sk_url"));
    }

    #[test]
    fn url_keyed_lookup_strips_fqdn_trailing_dot_in_file_key() {
        // Symmetric case: the credentials.toml entry carries the trailing dot.
        let env = TestEnv::default()
            .with_var(ENV_VAR_API_URL, "https://api.arete.run")
            .with_credentials("[keys]\n\"https://api.arete.run.\" = \"a4_sk_url\"");
        let r = resolve_with(&env, None, "").unwrap();
        assert_eq!(r.source, KeySource::CredentialsFile);
        assert_eq!(r.key.as_deref(), Some("a4_sk_url"));
    }

    #[test]
    fn url_keyed_lookup_strips_trailing_dot_before_port() {
        // The dot belongs to the host; an explicit port must survive.
        let env = TestEnv::default()
            .with_var(ENV_VAR_API_URL, "http://localhost.:3000")
            .with_credentials("[keys]\n\"http://localhost:3000\" = \"a4_sk_local\"");
        let r = resolve_with(&env, None, "").unwrap();
        assert_eq!(r.key.as_deref(), Some("a4_sk_local"));
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
    fn url_keyed_lookup_prefers_normalized_destination_spelling() {
        // Two spellings of one destination: requests go to the normalized
        // URL, so the entry stored under exactly that spelling must win
        // regardless of HashMap iteration order.
        let env = TestEnv::default()
            .with_var(ENV_VAR_API_URL, "https://api.arete.run/")
            .with_credentials(
                "[keys]\n\
                 \"https://api.arete.run\" = \"a4_sk_bare\"\n\
                 \"https://api.arete.run/\" = \"a4_sk_slash\"",
            );
        for _ in 0..8 {
            let r = resolve_with(&env, None, "").unwrap();
            assert_eq!(r.key.as_deref(), Some("a4_sk_bare"));
        }
    }

    #[test]
    fn url_keyed_lookup_breaks_normalized_ties_deterministically() {
        // No exact spelling matches; among normalized-equal entries the
        // sorted-first raw key is chosen, launch after launch.
        let env = TestEnv::default()
            .with_var(ENV_VAR_API_URL, "https://api.arete.run//")
            .with_credentials(
                "[keys]\n\
                 \"https://api.arete.run/\" = \"a4_sk_slash\"\n\
                 \"https://api.arete.run\" = \"a4_sk_bare\"",
            );
        for _ in 0..8 {
            let r = resolve_with(&env, None, "").unwrap();
            assert_eq!(r.key.as_deref(), Some("a4_sk_bare"));
        }
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
    fn named_profiles_require_selection_when_more_than_one_matches() {
        let content = "[profiles.agent.keys]\n\
                       \"https://api.arete.run\" = \"a4_ak_agent\"\n\
                       [profiles.human.keys]\n\
                       \"https://api.arete.run\" = \"a4_sk_human\"";
        let error = lookup_credentials(content, DEFAULT_API_URL, None).unwrap_err();
        assert!(error.to_string().contains("multiple Arete profiles"));
        assert!(error.to_string().contains("ARETE_PROFILE"));
    }

    #[test]
    fn named_profile_selection_returns_only_requested_principal() {
        let content = "[profiles.agent.keys]\n\
                       \"https://api.arete.run\" = \"a4_ak_agent\"\n\
                       [profiles.human.keys]\n\
                       \"https://api.arete.run\" = \"a4_sk_human\"";
        let agent = lookup_credentials(content, DEFAULT_API_URL, Some(AGENT_PROFILE)).unwrap();
        let human = lookup_credentials(content, DEFAULT_API_URL, Some(HUMAN_PROFILE)).unwrap();
        assert_eq!(agent.key.as_deref(), Some("a4_ak_agent"));
        assert_eq!(agent.profile.as_deref(), Some(AGENT_PROFILE));
        assert_eq!(human.key.as_deref(), Some("a4_sk_human"));
        assert_eq!(human.profile.as_deref(), Some(HUMAN_PROFILE));
    }

    #[test]
    fn reserved_profiles_reject_the_wrong_key_kind() {
        let content = "[profiles.agent.keys]\n\
                       \"https://api.arete.run\" = \"a4_sk_human\"";
        let error = lookup_credentials(content, DEFAULT_API_URL, Some(AGENT_PROFILE)).unwrap_err();
        assert!(error.to_string().contains("requires an a4_ak_*"));
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
    fn unparseable_file_is_an_error() {
        let env = TestEnv::default().with_credentials("not valid toml {{{");
        let error = resolve_with(&env, None, "wss://self.hosted.example").unwrap_err();
        assert!(error
            .to_string()
            .contains("failed to parse credentials file"));
    }

    // ── Hosted vs self-hosted failure modes ─────────────────────────────────

    #[test]
    fn hosted_url_without_any_key_is_error() {
        let env = TestEnv::default();
        let err = resolve_with(&env, None, "wss://any.stack.arete.run").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("No Arete API key found"), "{msg}");
        assert!(msg.contains("ARETE_API_KEY or the a4 login"), "{msg}");
        assert!(msg.contains("https://api.arete.run"), "{msg}");
        assert!(msg.contains("a4 auth login"), "{msg}");
        assert!(msg.contains("a4 auth signup"), "{msg}");
        // Never reveal where credentials are stored.
        assert!(!msg.contains(".arete/"), "{msg}");
        assert!(!msg.contains("credentials"), "{msg}");

        let env = TestEnv::default().with_var(ENV_VAR_PROFILE, "agent");
        let msg = resolve_with(&env, None, "wss://any.stack.arete.run")
            .unwrap_err()
            .to_string();
        assert!(msg.contains("profile `agent`"), "{msg}");
        assert!(!msg.contains(".arete/"), "{msg}");
    }

    #[test]
    fn key_sources_never_name_a_path() {
        assert_eq!(KeySource::CredentialsFile.as_str(), "a4-login");
        for source in [
            KeySource::Explicit,
            KeySource::EnvVar,
            KeySource::CredentialsFile,
            KeySource::None,
        ] {
            assert!(!source.as_str().contains('/'), "{}", source.as_str());
        }
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
