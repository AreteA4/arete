//! Update nudge: `a4 0.14.0 is available (you have 0.13.0). Run: a4 self update`.
//!
//! Runs from `main()` after every command, before telemetry is flushed. It
//! is throttled to one check per 24 h through `~/.arete/update-check.json`
//! (`{ "checkedAt", "latest" }`), only speaks on a TTY stderr, and stays
//! silent under `CI`, `--json`, `A4_NO_UPDATE_CHECK`, for the commands
//! `self`/`upgrade`/`mcp`/`stream`, and on any error. There is no background
//! auto-update; the nudge is the only unsolicited update behaviour.
//!
//! The clock and the fetch are injected into [`run_nudge`] so the throttle
//! is unit-testable without network or sleeping.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

use super::latest::{fetch_latest, is_newer};
use super::receipt::arete_home;

/// Minimum interval between two checks.
pub fn check_interval() -> chrono::Duration {
    chrono::Duration::hours(24)
}
/// Network timeout for the check; a slow docs site must not slow commands.
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(2);

/// Commands that never nudge (they own the update flow or stdout/stderr).
const QUIET_COMMANDS: &[&str] = &["self", "upgrade", "mcp", "stream"];

/// Contents of `~/.arete/update-check.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheck {
    /// RFC 3339 UTC timestamp of the last fetch.
    pub checked_at: String,
    /// Version the pointer carried at that time.
    pub latest: String,
}

/// Path of the throttle file.
pub fn update_check_path() -> Result<PathBuf> {
    Ok(arete_home()?.join("update-check.json"))
}

/// The nudge message for a newer version.
pub fn message(latest: &str, current: &str) -> String {
    format!("a4 {latest} is available (you have {current}). Run: a4 self update")
}

/// Whether the environment allows a nudge at all (no I/O).
pub fn allowed(command_name: &str, json: bool, stderr_is_tty: bool) -> bool {
    if json || !stderr_is_tty {
        return false;
    }
    if QUIET_COMMANDS.contains(&command_name) {
        return false;
    }
    if std::env::var_os("CI").is_some() {
        return false;
    }
    if std::env::var_os("A4_NO_UPDATE_CHECK").is_some_and(|value| !value.is_empty()) {
        return false;
    }
    true
}

fn load_state(path: &Path) -> Option<UpdateCheck> {
    let content = fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

fn save_state(path: &Path, state: &UpdateCheck) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }
    let content = serde_json::to_string_pretty(state)?;
    fs::write(path, format!("{content}\n"))
        .with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(())
}

/// Throttled check. Returns the message to print, if any.
///
/// * `now` is the injected clock.
/// * `current` is the running version.
/// * `fetch` retrieves the latest version string (network); it is only
///   called when the last check is older than [`check_interval`].
/// * `state_path` is the throttle file.
///
/// Any error (unreadable state, failed fetch, unparsable versions) yields
/// `None`; the caller stays silent.
pub fn run_nudge(
    now: DateTime<Utc>,
    current: &str,
    fetch: impl FnOnce() -> Result<String>,
    state_path: &Path,
) -> Option<String> {
    if let Some(state) = load_state(state_path) {
        if let Ok(checked_at) = DateTime::parse_from_rfc3339(&state.checked_at) {
            let checked_at = checked_at.with_timezone(&Utc);
            if now - checked_at < check_interval() && now >= checked_at {
                return None;
            }
        }
    }
    let latest = fetch().ok()?;
    let state = UpdateCheck {
        checked_at: now.to_rfc3339_opts(SecondsFormat::Secs, true),
        latest: latest.clone(),
    };
    // A failed write only means we may check again next time; not fatal.
    let _ = save_state(state_path, &state);
    if is_newer(&latest, current)? {
        Some(message(&latest, current))
    } else {
        None
    }
}

/// Entry point used by `main()`. Never fails, never prints on error.
pub fn maybe_nudge(command_name: &str, json: bool) {
    use std::io::IsTerminal;
    if !allowed(command_name, json, std::io::stderr().is_terminal()) {
        return;
    }
    let Ok(state_path) = update_check_path() else {
        return;
    };
    let fetch = || fetch_latest(FETCH_TIMEOUT).map(|pointer| pointer.version);
    if let Some(text) = run_nudge(Utc::now(), env!("CARGO_PKG_VERSION"), fetch, &state_path) {
        eprintln!("{text}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn at(rfc3339: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(rfc3339)
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn nudges_once_then_throttles_for_24h() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("update-check.json");
        let fetches = Cell::new(0);
        let fetch = || {
            fetches.set(fetches.get() + 1);
            Ok("0.14.0".to_string())
        };

        let t0 = at("2026-09-10T12:00:00Z");
        assert_eq!(
            run_nudge(t0, "0.13.0", fetch, &state),
            Some("a4 0.14.0 is available (you have 0.13.0). Run: a4 self update".to_string())
        );
        assert_eq!(fetches.get(), 1);
        let saved: UpdateCheck =
            serde_json::from_str(&fs::read_to_string(&state).unwrap()).unwrap();
        assert_eq!(saved.checked_at, "2026-09-10T12:00:00Z");
        assert_eq!(saved.latest, "0.14.0");

        // 23 h later: throttled, no fetch, no message.
        assert_eq!(
            run_nudge(t0 + chrono::Duration::hours(23), "0.13.0", fetch, &state),
            None
        );
        assert_eq!(fetches.get(), 1);

        // 25 h later: checks again.
        assert!(run_nudge(t0 + chrono::Duration::hours(25), "0.13.0", fetch, &state).is_some());
        assert_eq!(fetches.get(), 2);
    }

    #[test]
    fn silent_when_current_or_on_error() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("update-check.json");
        let now = at("2026-09-10T12:00:00Z");
        assert_eq!(
            run_nudge(now, "0.14.0", || Ok("0.14.0".to_string()), &state),
            None
        );
        // State was still recorded, so the next call within 24 h does not fetch.
        assert!(state.exists());
        let failing = || Err(anyhow::anyhow!("offline"));
        let later = now + chrono::Duration::hours(30);
        assert_eq!(run_nudge(later, "0.13.0", failing, &state), None);
        // Corrupt state is treated as "never checked".
        fs::write(&state, "{ not json").unwrap();
        assert!(run_nudge(later, "0.13.0", || Ok("9.0.0".to_string()), &state).is_some());
    }

    #[test]
    fn allowed_respects_json_tty_and_quiet_commands() {
        assert!(!allowed("init", true, true));
        assert!(!allowed("init", false, false));
        for quiet in ["self", "upgrade", "mcp", "stream"] {
            assert!(!allowed(quiet, false, true), "{quiet} must not nudge");
        }
        // The environment-dependent branch is exercised by the integration test.
    }
}
