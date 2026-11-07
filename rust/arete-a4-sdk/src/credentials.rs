//! `a4` CLI credential profiles.
//!
//! The `a4` CLI stores the keys it logs in with in a TOML credentials file
//! (`~/.arete/credentials.toml`, or `ARETE_CREDENTIALS_PATH`), keyed by
//! profile and API URL. This module owns the parsing and lookup rules so the
//! CLI, the MCP server and this SDK resolve profiles identically.
//!
//! The SDK uses [`cli_profile_secret_key`] as the last step of its server-side
//! credential chain: explicit auth option, then `ARETE_API_KEY`, then the
//! active `a4` profile. Only the key stored for [`DEFAULT_API_URL`] is used,
//! because that is the API the SDK's default token endpoint talks to: a key
//! the CLI stored for another API URL (`--api-url` / `ARETE_API_URL`) is never
//! sent anywhere else. Only secret-class keys (agent `a4_ak_` or secret
//! `a4_sk_`) are accepted. Any problem (no file, unreadable, malformed,
//! ambiguous) means "no key", never an error.
//!
//! Not part of the stable SDK surface.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{anyhow, Result};
use serde::Deserialize;

use crate::auth::{classify_api_key, ApiKeyClass};

/// The API the SDK's default hosted token endpoint belongs to.
pub const DEFAULT_API_URL: &str = "https://api.arete.run";
pub const AGENT_PROFILE: &str = "agent";
pub const HUMAN_PROFILE: &str = "human";
/// Selects a named credential profile (same variable the CLI reads).
pub const ENV_VAR_PROFILE: &str = "ARETE_PROFILE";
/// Overrides the credentials file location (same variable the CLI reads).
pub const ENV_VAR_CREDENTIALS_PATH: &str = "ARETE_CREDENTIALS_PATH";
/// Project file, relative to the working directory, that pins the
/// low-privilege agent profile (written by `a4 init`).
pub const PROJECT_AUTH_RELATIVE_PATH: &str = ".arete/auth.toml";

/// Credentials file location: `ARETE_CREDENTIALS_PATH` when set and
/// non-empty, otherwise `~/.arete/credentials.toml`.
pub fn credentials_path(read_env: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    if let Some(path) = read_env(ENV_VAR_CREDENTIALS_PATH).filter(|path| !path.is_empty()) {
        return Some(PathBuf::from(path));
    }
    dirs::home_dir().map(|home| home.join(".arete").join("credentials.toml"))
}

/// Outcome of the project profile file lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ProjectProfile {
    /// No project file.
    Absent,
    /// The file pins this profile.
    Selected(String),
    /// The file exists but is unreadable, malformed, or selects anything
    /// other than the agent profile. The CLI refuses to run here; the SDK
    /// uses no profile key.
    Invalid,
}

fn project_profile(content: Option<Result<String, std::io::Error>>) -> ProjectProfile {
    let content = match content {
        None => return ProjectProfile::Absent,
        Some(Err(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            return ProjectProfile::Absent
        }
        Some(Err(_)) => return ProjectProfile::Invalid,
        Some(Ok(content)) => content,
    };
    let Ok(value) = toml::from_str::<toml::Value>(&content) else {
        return ProjectProfile::Invalid;
    };
    match value.get("default_profile").and_then(toml::Value::as_str) {
        Some(AGENT_PROFILE) => ProjectProfile::Selected(AGENT_PROFILE.to_string()),
        _ => ProjectProfile::Invalid,
    }
}

static WARNED_AMBIGUOUS_PROFILE: AtomicBool = AtomicBool::new(false);

/// Where [`cli_profile_secret_key_with`] reads the outside world from.
pub(crate) struct ProfileSources<'a> {
    pub read_env: &'a dyn Fn(&str) -> Option<String>,
    /// Read a file to a string.
    pub read_file: &'a dyn Fn(&Path) -> Result<String, std::io::Error>,
    /// Working directory holding the optional project profile file. `None`
    /// means the project file cannot be checked, so no key is used.
    pub cwd: Option<PathBuf>,
    pub credentials_path: Option<PathBuf>,
}

/// The secret-class key of the active `a4` CLI profile for
/// [`DEFAULT_API_URL`], if any. Never fails.
pub fn cli_profile_secret_key() -> Option<String> {
    let read_env = |name: &str| std::env::var(name).ok();
    let credentials_path = credentials_path(read_env);
    cli_profile_secret_key_with(&ProfileSources {
        read_env: &read_env,
        read_file: &|path: &Path| std::fs::read_to_string(path),
        cwd: std::env::current_dir().ok(),
        credentials_path,
    })
}

/// Profile selection mirrors the CLI: the project file in the working
/// directory (agent only), then `ARETE_PROFILE`, then the single profile that
/// holds a key for the API URL.
pub(crate) fn cli_profile_secret_key_with(sources: &ProfileSources<'_>) -> Option<String> {
    // The project file can pin the agent profile; if it cannot be checked,
    // choose nothing rather than risk a profile the project excludes.
    let Some(cwd) = sources.cwd.as_ref() else {
        tracing::debug!("working directory unknown; not using an a4 login key");
        return None;
    };
    let project = Some((sources.read_file)(&cwd.join(PROJECT_AUTH_RELATIVE_PATH)));
    let profile = match project_profile(project) {
        ProjectProfile::Selected(profile) => Some(profile),
        ProjectProfile::Invalid => {
            tracing::debug!("a4 project profile file is invalid; not using an a4 login key");
            return None;
        }
        ProjectProfile::Absent => match (sources.read_env)(ENV_VAR_PROFILE) {
            None => None,
            Some(profile) => match validate_profile_name(&profile) {
                Ok(profile) => Some(profile.to_string()),
                Err(_) => {
                    tracing::debug!("{ENV_VAR_PROFILE} is invalid; not using an a4 login key");
                    return None;
                }
            },
        },
    };

    let path = sources.credentials_path.as_ref()?;
    let content = match (sources.read_file)(path) {
        Ok(content) => content,
        Err(error) => {
            if error.kind() != std::io::ErrorKind::NotFound {
                tracing::debug!("could not read a4 credentials: {}", error.kind());
            }
            return None;
        }
    };
    let key = match lookup_credentials(&content, DEFAULT_API_URL, profile.as_deref()) {
        Ok(lookup) => lookup.key?,
        Err(error) => {
            if let Some(CredentialsFileError::AmbiguousProfile { profiles, .. }) =
                error.downcast_ref::<CredentialsFileError>()
            {
                if !WARNED_AMBIGUOUS_PROFILE.swap(true, Ordering::Relaxed) {
                    tracing::warn!(
                        "More than one a4 login profile holds a key ({}); not choosing one. Set \
                         {ENV_VAR_PROFILE} (for example `agent`) or ARETE_API_KEY.",
                        profiles.join(", ")
                    );
                }
            } else {
                tracing::debug!("could not use a4 credentials: {error}");
            }
            return None;
        }
    };
    if classify_api_key(&key) != ApiKeyClass::Secret {
        tracing::debug!("a4 login key is not an agent or secret key; ignoring it");
        return None;
    }
    Some(key)
}

/// Why [`lookup_credentials`] could not use a credentials file. Messages
/// name profiles and API URLs, never key values.
///
/// `path` is `None` until a caller that knows where the file lives attaches
/// it with [`attach_credentials_path`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialsFileError {
    /// The file is not valid TOML, or is not laid out as credential profiles.
    Malformed {
        path: Option<PathBuf>,
        detail: String,
    },
    /// More than one profile holds a key for the API URL and none was
    /// selected. No key is chosen.
    AmbiguousProfile {
        path: Option<PathBuf>,
        api_url: String,
        profiles: Vec<String>,
    },
}

impl CredentialsFileError {
    fn path(&self) -> Option<&Path> {
        match self {
            Self::Malformed { path, .. } | Self::AmbiguousProfile { path, .. } => path.as_deref(),
        }
    }
}

impl std::fmt::Display for CredentialsFileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let file = match self.path() {
            Some(path) => format!("credentials file {}", path.display()),
            None => "credentials file".to_string(),
        };
        match self {
            Self::Malformed { detail, .. } => write!(
                f,
                "Failed to parse {file}: {detail}. Fix the file, or set \
                 {ENV_VAR_CREDENTIALS_PATH} to use a different one."
            ),
            Self::AmbiguousProfile {
                api_url, profiles, ..
            } => write!(
                f,
                "No profile selected and more than one profile in the {file} holds a key \
                 for {api_url}: {}. Choose one with `--profile <name>` (for example \
                 `--profile {}`) or {ENV_VAR_PROFILE}, set `default_profile = \"{AGENT_PROFILE}\"` \
                 in the project's {PROJECT_AUTH_RELATIVE_PATH}, or point \
                 {ENV_VAR_CREDENTIALS_PATH} at a file with one profile.",
                profiles.join(", "),
                profiles
                    .iter()
                    .find(|profile| *profile == AGENT_PROFILE)
                    .or(profiles.first())
                    .map(String::as_str)
                    .unwrap_or(AGENT_PROFILE),
            ),
        }
    }
}

impl std::error::Error for CredentialsFileError {}

/// Name `path` in a [`CredentialsFileError`] returned by
/// [`lookup_credentials`]. Other errors pass through unchanged.
pub fn attach_credentials_path(error: anyhow::Error, path: &Path) -> anyhow::Error {
    match error.downcast::<CredentialsFileError>() {
        Ok(CredentialsFileError::Malformed { detail, .. }) => CredentialsFileError::Malformed {
            path: Some(path.to_path_buf()),
            detail,
        }
        .into(),
        Ok(CredentialsFileError::AmbiguousProfile {
            api_url, profiles, ..
        }) => CredentialsFileError::AmbiguousProfile {
            path: Some(path.to_path_buf()),
            api_url,
            profiles,
        }
        .into(),
        Err(error) => error,
    }
}

/// Check that a credentials document parses, with the same contents-free
/// error [`lookup_credentials`] gives.
pub fn validate_credentials_file(content: &str) -> Result<(), CredentialsFileError> {
    parse_credentials_file(content).map(|_| ())
}

/// Parse a credentials document without echoing its contents: TOML errors
/// render with the offending source line, which can hold a key.
fn parse_credentials_file(content: &str) -> Result<CredentialsFile, CredentialsFileError> {
    let value = toml::from_str::<toml::Value>(content).map_err(|error| {
        let location = error
            .span()
            .map(|span| {
                let before = &content[..span.start.min(content.len())];
                let line = before.matches('\n').count() + 1;
                let column = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
                format!("line {line}, column {column}: ")
            })
            .unwrap_or_default();
        CredentialsFileError::Malformed {
            path: None,
            detail: format!("{location}{}", error.message()),
        }
    })?;
    // Shape errors quote the offending value, which can be a key, so they
    // are summarised instead.
    value
        .try_into::<CredentialsFile>()
        .map_err(|_| CredentialsFileError::Malformed {
            path: None,
            detail: "expected `[profiles.<name>.keys]` tables mapping API URLs to key strings"
                .to_string(),
        })
}

/// One profile-aware lookup from a parsed credentials document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialLookup {
    pub key: Option<String>,
    /// The named profile used. `None` means a legacy `[keys]`/`api_key` entry.
    pub profile: Option<String>,
    /// Whether `key` is stored for the looked-up API URL (a profile or
    /// `[keys]` entry). False for the legacy top-level `api_key`, which is
    /// stored for no URL, and when there is no key.
    pub url_bound: bool,
}

/// Validate a profile name accepted on the CLI and in credentials files.
pub fn validate_profile_name(profile: &str) -> Result<&str> {
    let profile = profile.trim();
    if profile.is_empty()
        || profile.len() > 64
        || !profile
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err(anyhow!(
            "invalid Arete profile `{profile}`; use 1-64 letters, numbers, hyphens, or underscores"
        ));
    }
    Ok(profile)
}

/// Infer the safe built-in profile from a key prefix.
pub fn inferred_profile_for_key(key: &str) -> &'static str {
    if key.trim().starts_with("a4_ak_") {
        AGENT_PROFILE
    } else {
        HUMAN_PROFILE
    }
}

/// Ensure reserved profiles cannot accidentally contain the other principal's
/// credential. Custom profiles remain available for advanced use.
pub fn validate_key_for_profile(profile: &str, key: &str) -> Result<()> {
    let profile = validate_profile_name(profile)?;
    let is_agent_key = key.trim().starts_with("a4_ak_");
    match profile {
        AGENT_PROFILE if !is_agent_key => Err(anyhow!(
            "profile `agent` requires an a4_ak_* agent credential"
        )),
        HUMAN_PROFILE if is_agent_key => Err(anyhow!(
            "profile `human` cannot contain an a4_ak_* agent credential"
        )),
        _ => Ok(()),
    }
}

/// Parse a credentials document with optional named-profile selection.
///
/// When no profile is requested, exactly one matching named profile is used.
/// Multiple matches are rejected instead of silently choosing a potentially
/// more privileged credential. Legacy URL-keyed credentials remain supported.
///
/// A malformed file or an ambiguous selection fails with a
/// [`CredentialsFileError`] (see [`attach_credentials_path`]).
pub fn lookup_credentials(
    content: &str,
    api_url: &str,
    requested_profile: Option<&str>,
) -> Result<CredentialLookup> {
    let parsed = parse_credentials_file(content)?;

    if let Some(profile) = requested_profile {
        let profile = validate_profile_name(profile)?;
        if let Some(key) = parsed
            .profiles
            .as_ref()
            .and_then(|profiles| profiles.get(profile))
            .and_then(|profile| find_url_key(&profile.keys, api_url))
        {
            validate_key_for_profile(profile, &key)?;
            return Ok(CredentialLookup {
                key: Some(key),
                profile: Some(profile.to_string()),
                url_bound: true,
            });
        }

        if let Some((key, url_bound)) = legacy_url_key(&parsed, api_url) {
            let compatible = match profile {
                AGENT_PROFILE => key.starts_with("a4_ak_"),
                HUMAN_PROFILE => !key.starts_with("a4_ak_"),
                _ => false,
            };
            if compatible {
                return Ok(CredentialLookup {
                    key: Some(key),
                    profile: None,
                    url_bound,
                });
            }
        }

        return Ok(CredentialLookup {
            key: None,
            profile: Some(profile.to_string()),
            url_bound: false,
        });
    }

    let mut matches = parsed
        .profiles
        .as_ref()
        .into_iter()
        .flat_map(|profiles| profiles.iter())
        .filter_map(|(name, profile)| {
            find_url_key(&profile.keys, api_url).map(|key| (name.clone(), key))
        })
        .collect::<Vec<_>>();
    matches.sort_by(|left, right| left.0.cmp(&right.0));
    match matches.as_slice() {
        [] => {}
        [(profile, key)] => {
            validate_key_for_profile(profile, key)?;
            return Ok(CredentialLookup {
                key: Some(key.clone()),
                profile: Some(profile.clone()),
                url_bound: true,
            });
        }
        _ => {
            return Err(CredentialsFileError::AmbiguousProfile {
                path: None,
                api_url: api_url.to_string(),
                profiles: matches.into_iter().map(|(profile, _)| profile).collect(),
            }
            .into());
        }
    }

    let legacy = legacy_url_key(&parsed, api_url);
    Ok(CredentialLookup {
        url_bound: legacy.as_ref().is_some_and(|(_, url_bound)| *url_bound),
        key: legacy.map(|(key, _)| key),
        profile: None,
    })
}

/// Normalize an API URL for credential lookup with the same rule
/// `RegistryClient` applies to its request destination (trim whitespace,
/// drop trailing slashes), plus the host rules the registry origin check
/// (`is_arete_origin`) applies: case-fold the scheme and host (both are
/// case-insensitive per RFC 3986) and drop the host's trailing dot (the
/// FQDN root label — `api.arete.run.` IS `api.arete.run`). So an
/// `ARETE_API_URL` of `https://API.Arete.Run.` must still find the key
/// stored under `https://api.arete.run`. Userinfo and path are left
/// untouched — they are case-sensitive. Both sides of the `[keys]` match
/// go through this so `ARETE_API_URL="https://api.arete.run/"` still finds
/// the key stored under `"https://api.arete.run"` and vice versa.
pub fn normalize_api_url(url: &str) -> String {
    let trimmed = url.trim().trim_end_matches('/');
    let (scheme, rest) = match trimmed.split_once("://") {
        Some((scheme, rest)) => (scheme.to_ascii_lowercase(), rest),
        None => (String::new(), trimmed),
    };
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    // Keep any `user:pass@` userinfo verbatim; the host starts after it.
    let host_start = authority.rfind('@').map_or(0, |i| i + 1);
    let host_port = &authority[host_start..];
    // Split the port off so the host rules cannot touch it. Bracketed IPv6
    // literals end at ']'; anything else carries at most one ':'.
    let (host, port) = if host_port.starts_with('[') {
        match host_port.find(']') {
            Some(end) => host_port.split_at(end + 1),
            None => (host_port, ""),
        }
    } else {
        match host_port.rfind(':') {
            Some(colon) => host_port.split_at(colon),
            None => (host_port, ""),
        }
    };
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    let mut out = String::with_capacity(trimmed.len());
    if !scheme.is_empty() {
        out.push_str(&scheme);
        out.push_str("://");
    }
    out.push_str(&authority[..host_start]);
    out.push_str(&host);
    out.push_str(port);
    out.push_str(&rest[authority_end..]);
    out
}

fn find_url_key(keys: &HashMap<String, String>, api_url: &str) -> Option<String> {
    let wanted = normalize_api_url(api_url);
    let exact = keys
        .get(api_url)
        .or_else(|| keys.get(wanted.as_str()))
        .map(|key| key.trim())
        .filter(|key| !key.is_empty());
    exact.map(str::to_string).or_else(|| {
        let mut candidates = keys
            .iter()
            .filter(|(url, _)| normalize_api_url(url) == wanted)
            .collect::<Vec<_>>();
        candidates.sort_by_key(|(url, _)| url.as_str());
        candidates
            .into_iter()
            .map(|(_, key)| key.trim())
            .find(|key| !key.is_empty())
            .map(str::to_string)
    })
}

/// The legacy key for `api_url`: a `[keys]` entry (stored for that URL,
/// so `true`), else the top-level `api_key` (stored for no URL, `false`).
fn legacy_url_key(parsed: &CredentialsFile, api_url: &str) -> Option<(String, bool)> {
    if let Some(key) = parsed
        .keys
        .as_ref()
        .and_then(|keys| find_url_key(keys, api_url))
    {
        return Some((key, true));
    }
    parsed
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(|key| (key.to_string(), false))
}

#[derive(Default, Deserialize)]
struct CredentialsFile {
    profiles: Option<HashMap<String, CredentialProfile>>,
    keys: Option<HashMap<String, String>>,
    api_key: Option<String>,
}

#[derive(Default, Deserialize)]
struct CredentialProfile {
    #[serde(default)]
    keys: HashMap<String, String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Error, ErrorKind};

    const AGENT: &str = "a4_ak_agentkey";
    const HUMAN: &str = "a4_sk_humankey";

    /// Hermetic sources: inline env vars and files keyed by path.
    struct Fixture {
        vars: HashMap<String, String>,
        files: HashMap<PathBuf, Result<String, ErrorKind>>,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                vars: HashMap::new(),
                files: HashMap::new(),
            }
        }
        fn var(mut self, name: &str, value: &str) -> Self {
            self.vars.insert(name.into(), value.into());
            self
        }
        fn credentials(mut self, content: &str) -> Self {
            self.files
                .insert(PathBuf::from("/home/creds.toml"), Ok(content.into()));
            self
        }
        fn project(mut self, content: Result<&str, ErrorKind>) -> Self {
            self.files.insert(
                PathBuf::from("/work").join(PROJECT_AUTH_RELATIVE_PATH),
                content.map(str::to_string),
            );
            self
        }
        fn key(&self) -> Option<String> {
            self.key_in(Some(PathBuf::from("/work")))
        }
        fn key_in(&self, cwd: Option<PathBuf>) -> Option<String> {
            let read_env = |name: &str| self.vars.get(name).cloned();
            let read_file = |path: &Path| match self.files.get(path) {
                Some(Ok(content)) => Ok(content.clone()),
                Some(Err(kind)) => Err(Error::from(*kind)),
                None => Err(Error::from(ErrorKind::NotFound)),
            };
            cli_profile_secret_key_with(&ProfileSources {
                read_env: &read_env,
                read_file: &read_file,
                cwd,
                credentials_path: Some(PathBuf::from("/home/creds.toml")),
            })
        }
    }

    fn both_profiles() -> String {
        format!(
            "[profiles.agent.keys]\n\"https://api.arete.run\" = \"{AGENT}\"\n\
             [profiles.human.keys]\n\"https://api.arete.run\" = \"{HUMAN}\"\n"
        )
    }

    #[test]
    fn single_profile_is_used_without_selection() {
        let file = format!("[profiles.agent.keys]\n\"https://api.arete.run/\" = \"{AGENT}\"\n");
        assert_eq!(
            Fixture::new().credentials(&file).key().as_deref(),
            Some(AGENT)
        );
    }

    #[test]
    fn ambiguous_profiles_without_selection_give_no_key() {
        assert_eq!(Fixture::new().credentials(&both_profiles()).key(), None);
    }

    #[test]
    fn arete_profile_selects_the_profile() {
        let fixture = Fixture::new().credentials(&both_profiles());
        let fixture = fixture.var(ENV_VAR_PROFILE, "human");
        assert_eq!(fixture.key().as_deref(), Some(HUMAN));
        let fixture = fixture.var(ENV_VAR_PROFILE, "agent");
        assert_eq!(fixture.key().as_deref(), Some(AGENT));
        let fixture = fixture.var(ENV_VAR_PROFILE, "missing");
        assert_eq!(fixture.key(), None);
        let fixture = fixture.var(ENV_VAR_PROFILE, "not a name!");
        assert_eq!(fixture.key(), None);
    }

    #[test]
    fn project_file_pins_agent_over_arete_profile() {
        let fixture = Fixture::new()
            .credentials(&both_profiles())
            .var(ENV_VAR_PROFILE, "human")
            .project(Ok("default_profile = \"agent\"\n"));
        assert_eq!(fixture.key().as_deref(), Some(AGENT));
    }

    #[test]
    fn invalid_project_file_gives_no_key() {
        for project in [
            Ok("default_profile = \"human\"\n"),
            Ok("not toml {{"),
            Ok("other = 1\n"),
            Err(ErrorKind::PermissionDenied),
        ] {
            let fixture = Fixture::new()
                .credentials(&both_profiles())
                .var(ENV_VAR_PROFILE, "agent")
                .project(project);
            assert_eq!(fixture.key(), None, "{project:?}");
        }
    }

    #[test]
    fn unknown_working_directory_gives_no_key() {
        let fixture = Fixture::new()
            .credentials(&both_profiles())
            .var(ENV_VAR_PROFILE, "human");
        assert_eq!(fixture.key_in(None), None);
    }

    #[test]
    fn login_keys_go_only_to_the_arete_api() {
        use crate::auth::{check_credential_destination, AccountCredential};
        // ARETE_API_URL never widens where the login key goes.
        let env =
            |name: &str| (name == "ARETE_API_URL").then(|| "https://other.arete.run".to_string());
        let allowed = |url: &str| {
            check_credential_destination(AccountCredential::A4Login, url, Some(url), &env).is_ok()
        };
        assert!(allowed("https://api.arete.run/ws/sessions"));
        assert!(allowed("https://API.arete.run.:443/x"));
        for url in [
            "http://api.arete.run/ws/sessions",
            "https://api.arete.run:8443/ws/sessions",
            "https://api.arete.run.evil.example/",
            "https://user@api.arete.run/",
            "https://other.arete.run/",
            "not a url",
        ] {
            assert!(!allowed(url), "{url}");
        }
    }

    #[test]
    fn only_the_default_api_url_key_is_used() {
        let file = format!("[profiles.agent.keys]\n\"http://localhost:3000\" = \"{AGENT}\"\n");
        let fixture = Fixture::new()
            .credentials(&file)
            .var("ARETE_API_URL", "http://localhost:3000");
        assert_eq!(fixture.key(), None);
    }

    #[test]
    fn non_secret_keys_are_ignored() {
        for key in ["a4_pk_publishable", "custom-key"] {
            let file = format!("api_key = \"{key}\"\n");
            assert_eq!(Fixture::new().credentials(&file).key(), None, "{key}");
        }
        let legacy = format!("[keys]\n\"https://api.arete.run\" = \"{HUMAN}\"\n");
        assert_eq!(
            Fixture::new().credentials(&legacy).key().as_deref(),
            Some(HUMAN)
        );
    }

    #[test]
    fn missing_unreadable_or_malformed_files_give_no_key() {
        assert_eq!(Fixture::new().key(), None);
        assert_eq!(Fixture::new().credentials("not toml {{{").key(), None);
        assert_eq!(Fixture::new().credentials("").key(), None);
        let mut unreadable = Fixture::new();
        unreadable.files.insert(
            PathBuf::from("/home/creds.toml"),
            Err(ErrorKind::PermissionDenied),
        );
        assert_eq!(unreadable.key(), None);
    }

    fn lookup_error(content: &str, profile: Option<&str>) -> CredentialsFileError {
        lookup_credentials(content, DEFAULT_API_URL, profile)
            .unwrap_err()
            .downcast::<CredentialsFileError>()
            .expect("typed credentials error")
    }

    #[test]
    fn syntax_errors_are_malformed_and_do_not_echo_the_file() {
        let content = format!("[profiles.agent.keys]\n\"https://api.arete.run\" = \"{AGENT}\n");
        let error = lookup_error(&content, None);
        assert!(
            matches!(error, CredentialsFileError::Malformed { .. }),
            "{error:?}"
        );
        let message = error.to_string();
        assert!(
            message.starts_with("Failed to parse credentials file: line 2"),
            "{message}"
        );
        assert!(!message.contains(AGENT), "{message}");
    }

    #[test]
    fn layout_errors_are_malformed_and_do_not_echo_values() {
        let content = format!("[profiles]\nagent = \"{AGENT}\"\n");
        let message = lookup_error(&content, Some(AGENT_PROFILE)).to_string();
        assert!(
            message.contains("expected `[profiles.<name>.keys]`"),
            "{message}"
        );
        assert!(!message.contains(AGENT), "{message}");
    }

    #[test]
    fn ambiguous_profiles_name_the_profiles_path_and_fix() {
        let error = lookup_error(&both_profiles(), None);
        assert_eq!(
            error,
            CredentialsFileError::AmbiguousProfile {
                path: None,
                api_url: DEFAULT_API_URL.to_string(),
                profiles: vec![AGENT_PROFILE.to_string(), HUMAN_PROFILE.to_string()],
            }
        );
        let error = attach_credentials_path(error.into(), Path::new("/home/creds.toml"));
        let message = error.to_string();
        for expected in [
            "No profile selected",
            "credentials file /home/creds.toml",
            "for https://api.arete.run: agent, human",
            "`--profile <name>`",
            "`--profile agent`",
            ENV_VAR_PROFILE,
            PROJECT_AUTH_RELATIVE_PATH,
            ENV_VAR_CREDENTIALS_PATH,
        ] {
            assert!(message.contains(expected), "{expected}: {message}");
        }
        assert!(!message.contains("parse"), "{message}");
        assert!(
            !message.contains(AGENT) && !message.contains(HUMAN),
            "{message}"
        );
    }

    #[test]
    fn attach_credentials_path_names_malformed_files_and_passes_other_errors() {
        let malformed = lookup_credentials("[keys", DEFAULT_API_URL, None).unwrap_err();
        let message = attach_credentials_path(malformed, Path::new("/x/creds.toml")).to_string();
        assert!(
            message.starts_with("Failed to parse credentials file /x/creds.toml: "),
            "{message}"
        );

        let other = anyhow!("profile `agent` requires an a4_ak_* agent credential");
        let message = attach_credentials_path(other, Path::new("/x/creds.toml")).to_string();
        assert_eq!(
            message,
            "profile `agent` requires an a4_ak_* agent credential"
        );
    }

    #[test]
    fn explicit_profile_resolves_an_otherwise_ambiguous_file() {
        for (profile, key) in [(AGENT_PROFILE, AGENT), (HUMAN_PROFILE, HUMAN)] {
            let lookup = lookup_credentials(&both_profiles(), DEFAULT_API_URL, Some(profile))
                .expect("selected profile");
            assert_eq!(lookup.key.as_deref(), Some(key));
            assert_eq!(lookup.profile.as_deref(), Some(profile));
        }
    }

    #[test]
    fn credentials_path_honours_override() {
        let path = credentials_path(|name| {
            (name == ENV_VAR_CREDENTIALS_PATH).then(|| "/custom/creds.toml".to_string())
        });
        assert_eq!(path, Some(PathBuf::from("/custom/creds.toml")));
        let default = credentials_path(|_| Some(String::new()));
        if let Some(default) = default {
            assert!(default.ends_with(".arete/credentials.toml"));
        }
    }
}
