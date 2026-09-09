//! Coding-agent integration shared by `a4 init` and `a4 doctor`: agent
//! detection, the `AGENTS.md` managed block, skills installation and MCP
//! config writers.
//!
//! Spec: `docs/internal/agent-first-onboarding.md` (WP7, WP8).

pub mod agents_md;
pub mod detect;
pub mod jsonc;
pub mod mcp_config;
pub mod report;
pub mod skills;
pub mod workspace;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

pub use report::{ItemResult, Outcome};

/// Every agent id in table order (the order `init` processes them).
pub const AGENT_IDS: [&str; 14] = [
    "claude-code",
    "cursor",
    "codex",
    "opencode",
    "gemini-cli",
    "vscode",
    "copilot-cli",
    "windsurf",
    "cline",
    "zed",
    "amp",
    "kiro",
    "roo",
    "goose",
];

/// Whether `id` is a known agent id.
pub fn is_agent_id(id: &str) -> bool {
    AGENT_IDS.contains(&id)
}

/// Process environment as seen by detection and the writers, made
/// injectable so unit tests never touch process globals.
#[derive(Debug, Clone)]
pub struct Env {
    /// Project root: the parent of `--config` (default `arete.toml`).
    pub root: PathBuf,
    /// Home directory, if known.
    pub home: Option<PathBuf>,
    /// Environment variables.
    pub vars: BTreeMap<String, String>,
}

impl Env {
    /// Snapshot of the real process environment for `root`.
    pub fn from_process(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            home: dirs::home_dir(),
            vars: std::env::vars().collect(),
        }
    }

    /// Build an environment for tests.
    #[cfg(test)]
    pub fn new(root: impl Into<PathBuf>, home: Option<PathBuf>, vars: &[(&str, &str)]) -> Self {
        Self {
            root: root.into(),
            home,
            vars: vars
                .iter()
                .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
                .collect(),
        }
    }

    /// Non-empty environment variable.
    pub fn var(&self, key: &str) -> Option<&str> {
        self.vars
            .get(key)
            .map(String::as_str)
            .filter(|value| !value.trim().is_empty())
    }

    /// Home directory or an error naming the fix.
    pub fn home(&self) -> Result<&Path> {
        self.home
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("Could not find home directory (set HOME)"))
    }

    /// `$CODEX_HOME` or `~/.codex`.
    pub fn codex_home(&self) -> Option<PathBuf> {
        self.var("CODEX_HOME")
            .map(PathBuf::from)
            .or_else(|| self.home.as_ref().map(|home| home.join(".codex")))
    }

    /// `$XDG_CONFIG_HOME` or `~/.config`.
    pub fn xdg_config_home(&self) -> Option<PathBuf> {
        self.var("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| self.home.as_ref().map(|home| home.join(".config")))
    }

    /// `PATH` of this environment.
    pub fn path_env(&self) -> std::ffi::OsString {
        std::ffi::OsString::from(self.var("PATH").unwrap_or_default())
    }
}

/// Path shown in reports: relative to the project root when beneath it,
/// otherwise absolute.
pub fn display_path(env: &Env, path: &Path) -> String {
    let relative = path
        .strip_prefix(&env.root)
        .map(Path::to_path_buf)
        .ok()
        .or_else(|| {
            let root = fs::canonicalize(&env.root).ok()?;
            let full = fs::canonicalize(path).ok()?;
            full.strip_prefix(&root).ok().map(Path::to_path_buf)
        });
    relative
        .as_deref()
        .unwrap_or(path)
        .to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, "/")
}

/// Read a file; `Ok(None)` when it does not exist.
pub fn read_optional(path: &Path) -> Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(content) => Ok(Some(content)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("Failed to read {}", path.display())),
    }
}

/// Write `content` to `path` if it differs, reporting the outcome. Creates
/// parent directories. With `dry_run` nothing is written.
pub fn upsert_file(path: &Path, content: &str, dry_run: bool) -> Outcome {
    let existing = match read_optional(path) {
        Ok(existing) => existing,
        Err(error) => return Outcome::error(format!("{error:#}"), None),
    };
    if existing.as_deref() == Some(content) {
        return Outcome::Unchanged;
    }
    let outcome = if existing.is_some() {
        Outcome::Updated
    } else {
        Outcome::Created
    };
    if dry_run {
        return outcome;
    }
    if let Some(parent) = path.parent() {
        if let Err(error) = fs::create_dir_all(parent) {
            return Outcome::error(
                format!("Failed to create {}: {error}", parent.display()),
                None,
            );
        }
    }
    match fs::write(path, content) {
        Ok(()) => outcome,
        Err(error) => Outcome::error(format!("Failed to write {}: {error}", path.display()), None),
    }
}

/// Find an executable on `path_env` (`name` plus `.cmd`/`.exe` on Windows).
pub fn find_on_path(path_env: &std::ffi::OsStr, name: &str) -> Option<PathBuf> {
    let candidates: Vec<String> = if cfg!(windows) {
        vec![
            format!("{name}.cmd"),
            format!("{name}.exe"),
            name.to_string(),
        ]
    } else {
        vec![name.to_string()]
    };
    for dir in std::env::split_paths(path_env).filter(|d| !d.as_os_str().is_empty()) {
        for candidate in &candidates {
            let full = dir.join(candidate);
            if is_executable(&full) {
                return Some(full);
            }
        }
    }
    None
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path)
        .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    fs::metadata(path)
        .map(|meta| meta.is_file())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_reports_created_unchanged_updated_and_respects_dry_run() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/file.txt");
        assert_eq!(upsert_file(&path, "a\n", true), Outcome::Created);
        assert!(!path.exists(), "dry run must not write");
        assert_eq!(upsert_file(&path, "a\n", false), Outcome::Created);
        assert_eq!(upsert_file(&path, "a\n", false), Outcome::Unchanged);
        assert_eq!(upsert_file(&path, "b\n", true), Outcome::Updated);
        assert_eq!(fs::read_to_string(&path).unwrap(), "a\n");
        assert_eq!(upsert_file(&path, "b\n", false), Outcome::Updated);
        assert_eq!(fs::read_to_string(&path).unwrap(), "b\n");
    }

    #[test]
    fn env_lookups_fall_back_to_home() {
        let env = Env::new("/proj", Some(PathBuf::from("/home/u")), &[("EMPTY", "  ")]);
        assert_eq!(env.var("EMPTY"), None);
        assert_eq!(env.codex_home(), Some(PathBuf::from("/home/u/.codex")));
        assert_eq!(
            env.xdg_config_home(),
            Some(PathBuf::from("/home/u/.config"))
        );
        let env = Env::new(
            "/proj",
            Some(PathBuf::from("/home/u")),
            &[("CODEX_HOME", "/c"), ("XDG_CONFIG_HOME", "/x")],
        );
        assert_eq!(env.codex_home(), Some(PathBuf::from("/c")));
        assert_eq!(env.xdg_config_home(), Some(PathBuf::from("/x")));
    }

    #[test]
    fn display_path_is_relative_inside_root() {
        let env = Env::new("/proj", None, &[]);
        assert_eq!(
            display_path(&env, Path::new("/proj/.cursor/mcp.json")),
            ".cursor/mcp.json"
        );
        assert_eq!(
            display_path(&env, Path::new("/home/u/.claude.json")),
            "/home/u/.claude.json"
        );
    }
}
