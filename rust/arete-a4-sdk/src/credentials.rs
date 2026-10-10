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

/// Start of the [`lookup_credentials`] error for more than one matching
/// profile.
const AMBIGUOUS_PROFILES_PREFIX: &str = "multiple Arete profiles match";

static WARNED_AMBIGUOUS_PROFILE: AtomicBool = AtomicBool::new(false);

/// True when `url` is on the API the `a4` login key was stored for
/// ([`DEFAULT_API_URL`]). A login key is never sent anywhere else, such as a
/// session endpoint a stack names on another host.
pub(crate) fn is_login_key_destination(url: &str) -> bool {
    let (Ok(url), Ok(api)) = (url::Url::parse(url), url::Url::parse(DEFAULT_API_URL)) else {
        return false;
    };
    let host = url
        .host_str()
        .map(|host| host.trim_end_matches('.').to_ascii_lowercase());
    url.scheme() == "https"
        && host.as_deref() == api.host_str()
        && url.port_or_known_default() == Some(443)
        && url.username().is_empty()
        && url.password().is_none()
}

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
            if error.to_string().starts_with(AMBIGUOUS_PROFILES_PREFIX) {
                if !WARNED_AMBIGUOUS_PROFILE.swap(true, Ordering::Relaxed) {
                    tracing::warn!(
                        "More than one a4 login profile holds a key; not choosing one. Set \
                         {ENV_VAR_PROFILE} (for example `agent`) or ARETE_API_KEY."
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

/// One profile-aware lookup from a parsed credentials document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialLookup {
    pub key: Option<String>,
    /// The named profile used. `None` means a legacy `[keys]`/`api_key` entry.
    pub profile: Option<String>,
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
pub fn lookup_credentials(
    content: &str,
    api_url: &str,
    requested_profile: Option<&str>,
) -> Result<CredentialLookup> {
    let parsed: CredentialsFile = toml::from_str(content)
        .map_err(|error| anyhow!("failed to parse credentials file: {error}"))?;

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
            });
        }

        if let Some(key) = legacy_url_key(&parsed, api_url) {
            let compatible = match profile {
                AGENT_PROFILE => key.starts_with("a4_ak_"),
                HUMAN_PROFILE => !key.starts_with("a4_ak_"),
                _ => false,
            };
            if compatible {
                return Ok(CredentialLookup {
                    key: Some(key),
                    profile: None,
                });
            }
        }

        return Ok(CredentialLookup {
            key: None,
            profile: Some(profile.to_string()),
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
            });
        }
        _ => {
            let profiles = matches
                .iter()
                .map(|(profile, _)| profile.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(anyhow!(
                "{AMBIGUOUS_PROFILES_PREFIX} {api_url}: {profiles}; set {ENV_VAR_PROFILE} or pass --profile"
            ));
        }
    }

    Ok(CredentialLookup {
        key: legacy_url_key(&parsed, api_url),
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

fn legacy_url_key(parsed: &CredentialsFile, api_url: &str) -> Option<String> {
    if let Some(key) = parsed
        .keys
        .as_ref()
        .and_then(|keys| find_url_key(keys, api_url))
    {
        return Some(key);
    }
    parsed
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(str::to_string)
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
        assert!(is_login_key_destination(
            "https://api.arete.run/ws/sessions"
        ));
        assert!(is_login_key_destination("https://API.arete.run.:443/x"));
        for url in [
            "http://api.arete.run/ws/sessions",
            "https://api.arete.run:8443/ws/sessions",
            "https://api.arete.run.evil.example/",
            "https://user@api.arete.run/",
            "https://other.arete.run/",
            "not a url",
        ] {
            assert!(!is_login_key_destination(url), "{url}");
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
