//! Privacy-respecting telemetry for the Arete CLI.
//!
//! Anonymous usage tracking to help improve the CLI:
//! - Opt-out with a transparent first-run disclosure on a TTY
//! - No PII (paths, usernames, secrets, stack names, deployment ids)
//! - Non-blocking: telemetry never adds latency to commands
//! - Minimal data: only what's needed to answer specific questions
//!
//! Users can disable telemetry via:
//! - `a4 telemetry disable`
//! - `DO_NOT_TRACK=1` environment variable
//! - `ARETE_TELEMETRY_DISABLED=1` environment variable
//!
//! CI runs are not sent. That is the existing opt-out, and it keeps test
//! runs from posting. Detection is a truthy `CI` (`true`, `1`, `yes`) or
//! another common CI variable (`GITHUB_ACTIONS`, `GITLAB_CI`, `CIRCLECI`,
//! `TRAVIS`, `BUILDKITE`, `TF_BUILD`, `CIRRUS_CI`, `APPVEYOR`, `DRONE`,
//! `WOODPECKER`, `CODEBUILD_BUILD_ID`, `BITBUCKET_BUILD_NUMBER`,
//! `TEAMCITY_VERSION`, `JENKINS_URL`). The docs-site privacy notice does not
//! govern this CLI; the env vars above are the CLI contract. `a4 telemetry
//! status` says when CI suppression is active.
//!
//! Ingested events include `is_ci=false` (CI runs never arrive, so there is
//! no `is_ci=true` series) and `install_source` of `curl`, `npm`, `cargo`,
//! `agent`, or `unknown`. Agent-driven processes are real usage and are
//! still sent. Filter them in PostHog with `install_source != agent`, and
//! combine with `is_ci = false` when a board should show only non-CI events.
//!
//! The PostHog project key is the public write-only `phc_` key already
//! embedded below. Do not rotate it here.

use anyhow::{Context, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::Duration;
use uuid::Uuid;

static PENDING_EVENTS: OnceLock<Mutex<Vec<JoinHandle<()>>>> = OnceLock::new();
static SELECTED_TEMPLATE: OnceLock<Mutex<Option<String>>> = OnceLock::new();
static INSTALL_SOURCE: OnceLock<String> = OnceLock::new();

fn get_pending_events() -> &'static Mutex<Vec<JoinHandle<()>>> {
    PENDING_EVENTS.get_or_init(|| Mutex::new(Vec::new()))
}

fn template_slot() -> &'static Mutex<Option<String>> {
    SELECTED_TEMPLATE.get_or_init(|| Mutex::new(None))
}

fn spawn_send(event: TelemetryEvent) {
    let handle = std::thread::spawn(move || {
        let _ = send_event(&event);
    });
    if let Ok(mut pending) = get_pending_events().lock() {
        pending.push(handle);
    }
}

/// PostHog public project API key (write-only, safe to embed in client code)
const POSTHOG_API_KEY: &str = "phc_PUDsD0lYcsjoXfMRYhH9k91xDbuxzAyw6ZD0tg3OUHz";

const POSTHOG_ENDPOINT: &str = "https://eu.i.posthog.com/capture/";

pub const TELEMETRY_DOCS_URL: &str = "https://arete.run/telemetry";

const CI_ENV: &[&str] = &[
    "CI",
    "GITHUB_ACTIONS",
    "GITLAB_CI",
    "CIRCLECI",
    "TRAVIS",
    "BUILDKITE",
    "TF_BUILD",
    "CIRRUS_CI",
    "APPVEYOR",
    "DRONE",
    "WOODPECKER",
    "CODEBUILD_BUILD_ID",
    "BITBUCKET_BUILD_NUMBER",
    "TEAMCITY_VERSION",
    "JENKINS_URL",
];

/// Env signals from the coding-agent detector. Presence only — no project
/// or home directory scan, so this stays non-invasive.
const AGENT_ENV: &[&str] = &[
    "CLAUDECODE",
    "CURSOR_AGENT",
    "CODEX_THREAD_ID",
    "CODEX_SANDBOX",
    "CODEX_SANDBOX_NETWORK_DISABLED",
    "OPENCODE_CLIENT",
    "GEMINI_CLI",
];

// ============================================================================
// Configuration
// ============================================================================

/// Telemetry configuration stored in ~/.arete/telemetry.json
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryConfig {
    /// Whether telemetry is enabled (default: true)
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    /// Anonymous UUID for this installation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anonymous_id: Option<String>,

    /// Whether the consent banner has been shown
    #[serde(default)]
    pub consent_shown: bool,

    /// When the consent banner was shown
    #[serde(skip_serializing_if = "Option::is_none")]
    pub consent_shown_at: Option<String>,

    /// Whether this install has already emitted `first_run`.
    ///
    /// Written only when the anonymous id is created. Existing configs are
    /// not rewritten to set this flag: that save could restore `enabled: true`
    /// over a concurrent `a4 telemetry disable`.
    #[serde(default)]
    pub first_run_recorded: bool,
}

fn default_enabled() -> bool {
    true
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            anonymous_id: None,
            consent_shown: false,
            consent_shown_at: None,
            first_run_recorded: false,
        }
    }
}

impl TelemetryConfig {
    /// Load telemetry config from disk, creating default if missing
    pub fn load() -> Self {
        Self::load_from_path(&config_path()).unwrap_or_default()
    }

    fn load_from_path(path: &PathBuf) -> Result<Self> {
        let contents = fs::read_to_string(path)?;
        let config: TelemetryConfig = serde_json::from_str(&contents)?;
        Ok(config)
    }

    /// Save telemetry config to disk
    pub fn save(&self) -> Result<()> {
        let path = config_path();
        crate::arete_home::ensure_arete_home()?;

        let contents = serde_json::to_string_pretty(self)?;
        fs::write(&path, contents)
            .with_context(|| format!("Failed to write telemetry config: {:?}", path))?;

        Ok(())
    }

    /// Get or create the anonymous ID in memory. Does not write the file.
    /// Callers persist through [`mutate_config`], which reloads under a lock
    /// so a concurrent opt-out is not overwritten.
    pub fn get_or_create_anonymous_id(&mut self) -> String {
        if let Some(ref id) = self.anonymous_id {
            return id.clone();
        }

        let id = Uuid::new_v4().to_string();
        self.anonymous_id = Some(id.clone());
        id
    }
}

/// Exclusive lock around every read-modify-write of `telemetry.json`.
/// The lock file stays on disk; the kernel releases it if the process exits.
struct TelemetryConfigLock {
    file: fs::File,
}

impl Drop for TelemetryConfigLock {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.file);
    }
}

fn lock_telemetry_config() -> Result<TelemetryConfigLock> {
    use fs2::FileExt;

    crate::arete_home::ensure_arete_home()?;
    let lock_path = config_path().with_file_name(".telemetry.json.lock");
    let mut options = fs::OpenOptions::new();
    options.create(true).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(&lock_path)
        .with_context(|| format!("Failed to open telemetry lock {}", lock_path.display()))?;
    file.lock_exclusive()
        .with_context(|| format!("Failed to lock telemetry config {}", lock_path.display()))?;
    Ok(TelemetryConfigLock { file })
}

/// Reload `telemetry.json`, apply `mutate`, and save, while holding the
/// config lock. `mutate` must not assume a previously loaded copy: `enabled`
/// and the other fields are whatever is on disk now.
fn mutate_config(mutate: impl FnOnce(&mut TelemetryConfig)) -> Result<()> {
    let _lock = lock_telemetry_config()?;
    let mut config = TelemetryConfig::load();
    mutate(&mut config);
    config.save()
}

/// Get the path to the telemetry config file
fn config_path() -> PathBuf {
    crate::arete_home::arete_home()
        .unwrap_or_else(|_| PathBuf::from(".arete"))
        .join("telemetry.json")
}

// ============================================================================
// Collection Logic
// ============================================================================

/// Whether this process looks like CI.
///
/// `CI` must be truthy (`true`, `1`, `yes`, or any other value that is not
/// empty/`false`/`0`/`no`/`off`). Vendor variables count when they are set
/// to a non-empty, non-false value. The values themselves are never sent.
pub fn in_ci() -> bool {
    ci_signaled(|key| std::env::var(key).ok())
}

fn ci_signaled(mut var: impl FnMut(&str) -> Option<String>) -> bool {
    CI_ENV
        .iter()
        .any(|key| var(key).is_some_and(|value| is_truthy(&value)))
}

fn is_truthy(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty()
        && !matches!(
            value.to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "off"
        )
}

/// Check if telemetry should be collected.
///
/// Returns false if:
/// - `DO_NOT_TRACK=1` is set (standard)
/// - `ARETE_TELEMETRY_DISABLED=1` is set
/// - Telemetry is disabled in config
/// - CI environment is detected
pub fn should_collect() -> bool {
    // `cargo test` compiles this binary with cfg(test). Those unit tests call
    // create/init paths and must not post to PostHog. Integration tests spawn
    // the normal binary (cfg(test) is off) and opt out with DO_NOT_TRACK or CI.
    if cfg!(test) || env_disables_telemetry() || in_ci() {
        return false;
    }

    TelemetryConfig::load().enabled
}

fn env_disables_telemetry() -> bool {
    env_flag_set("DO_NOT_TRACK") || env_flag_set("ARETE_TELEMETRY_DISABLED")
}

fn env_flag_set(key: &str) -> bool {
    std::env::var(key)
        .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Enable telemetry collection
pub fn enable() -> Result<()> {
    mutate_config(|config| {
        config.enabled = true;
    })
}

/// Disable telemetry collection
pub fn disable() -> Result<()> {
    mutate_config(|config| {
        config.enabled = false;
    })
}

pub fn status() -> (bool, Option<String>) {
    let config = TelemetryConfig::load();
    let effective_enabled = should_collect();
    (effective_enabled, config.anonymous_id)
}

pub fn flush() {
    if let Ok(mut pending) = get_pending_events().lock() {
        for handle in pending.drain(..) {
            let _ = handle.join();
        }
    }
}

/// Coarse install/invoker channel: `curl`, `npm`, `cargo`, `agent`, or `unknown`.
///
/// Agent environment variables win, because that is the noise source to
/// filter. Otherwise the install receipt's original channel (`sh`/`ps1` →
/// curl, `npm`, `cargo`) wins, including after `a4 self update` rewrites
/// `source` to `self-update`. A missing channel falls back to the current
/// executable path (`node_modules` → npm, `.cargo` → cargo) without sending
/// the path.
pub fn install_source_label() -> &'static str {
    INSTALL_SOURCE
        .get_or_init(|| detect_install_source().to_string())
        .as_str()
}

fn detect_install_source() -> &'static str {
    let agent = AGENT_ENV.iter().any(|key| {
        std::env::var(key)
            .ok()
            .is_some_and(|value| is_truthy(&value))
    });
    let channel = crate::selfhost::receipt::Receipt::load()
        .ok()
        .flatten()
        .and_then(|receipt| receipt.install_channel().map(str::to_string));
    classify_install_source(agent, channel.as_deref(), exe_install_kind())
}

#[derive(Clone, Copy)]
enum ExeInstallKind {
    Npm,
    Cargo,
    Other,
}

fn exe_install_kind() -> ExeInstallKind {
    let Ok(path) = std::env::current_exe() else {
        return ExeInstallKind::Other;
    };
    let mut npm = false;
    let mut cargo = false;
    for component in path.components() {
        match component.as_os_str().to_string_lossy().as_ref() {
            "node_modules" => npm = true,
            ".cargo" => cargo = true,
            _ => {}
        }
    }
    if npm {
        ExeInstallKind::Npm
    } else if cargo {
        ExeInstallKind::Cargo
    } else {
        ExeInstallKind::Other
    }
}

fn classify_install_source(
    agent: bool,
    receipt: Option<&str>,
    exe: ExeInstallKind,
) -> &'static str {
    if agent {
        return "agent";
    }
    match receipt {
        Some("sh" | "ps1") => "curl",
        Some("npm") => "npm",
        Some("cargo") => "cargo",
        Some("manual" | "self-update") | None => match exe {
            ExeInstallKind::Npm => "npm",
            ExeInstallKind::Cargo => "cargo",
            ExeInstallKind::Other => "unknown",
        },
        Some(_) => "unknown",
    }
}

/// `first_run` is only for an install that does not yet have an anonymous id.
///
/// Existing configs are not rewritten to set `first_run_recorded`. That
/// backfill saved the whole file and could put `enabled: true` back after
/// `a4 telemetry disable` ran between the load and the save.
fn should_emit_first_run(anonymous_id: Option<&str>) -> bool {
    anonymous_id.is_none()
}

/// Assign an id on a config that was just loaded. Leaves `enabled` alone.
/// Returns whether this call created the id.
fn assign_anonymous_id(config: &mut TelemetryConfig) -> bool {
    if config.anonymous_id.is_some() {
        return false;
    }
    let _ = config.get_or_create_anonymous_id();
    config.first_run_recorded = true;
    true
}

struct Identity {
    anonymous_id: String,
    emit_first_run: bool,
    /// A concurrent disable won before the new id was saved.
    opted_out: bool,
}

fn prepare_identity() -> Identity {
    let loaded = TelemetryConfig::load();
    if !should_emit_first_run(loaded.anonymous_id.as_deref()) {
        return Identity {
            anonymous_id: loaded
                .anonymous_id
                .unwrap_or_else(|| Uuid::new_v4().to_string()),
            emit_first_run: false,
            opted_out: false,
        };
    }

    let mut created_id = None;
    let mut created = false;
    let mut opted_out = false;
    if mutate_config(|config| {
        created = assign_anonymous_id(config);
        created_id = config.anonymous_id.clone();
        opted_out = !config.enabled;
    })
    .is_err()
    {
        // Could not lock the file. Do not write an unlocked copy.
        return Identity {
            anonymous_id: Uuid::new_v4().to_string(),
            emit_first_run: true,
            opted_out: false,
        };
    }

    Identity {
        anonymous_id: created_id.unwrap_or_else(|| Uuid::new_v4().to_string()),
        emit_first_run: created && !opted_out,
        opted_out,
    }
}

/// Template chosen during this process, if `template_selected` ran.
pub fn selected_template() -> Option<String> {
    template_slot().lock().ok().and_then(|guard| guard.clone())
}

fn remember_template(template: &str) {
    if let Ok(mut slot) = template_slot().lock() {
        *slot = Some(template.to_string());
    }
}

// ============================================================================
// Consent Banner
// ============================================================================

/// Show the consent banner if it hasn't been shown yet.
/// This is non-blocking and prints to stderr to avoid breaking pipes.
pub fn show_consent_banner_if_needed() {
    use std::io::IsTerminal;

    // Agents and pipes never see the banner; it is only for humans at a TTY.
    if !std::io::stderr().is_terminal() {
        return;
    }

    if TelemetryConfig::load().consent_shown {
        return;
    }

    // Print banner to stderr (doesn't break stdout pipes)
    let banner = r#"
┌─────────────────────────────────────────────────────────────┐
│                                                             │
│  Arete collects anonymous usage data to improve the    │
│  CLI. No personal information or project details are sent.  │
│                                                             │
│  Disable anytime:  a4 telemetry disable                     │
│  Learn more:       https://arete.run/telemetry      │
│                                                             │
└─────────────────────────────────────────────────────────────┘
"#;

    let _ = writeln!(std::io::stderr(), "{}", banner);

    // Reload under the lock so this write cannot put `enabled: true` back
    // over a disable that landed after the banner was printed.
    let _ = mutate_config(|config| {
        config.consent_shown = true;
        config.consent_shown_at = Some(chrono::Utc::now().to_rfc3339());
    });

    record_first_run();
}

// ============================================================================
// Event Types
// ============================================================================

/// Telemetry event sent to PostHog
#[derive(Debug, Clone, Serialize)]
pub struct TelemetryEvent {
    /// Event type (e.g., "command_executed", "first_run")
    pub event: String,

    /// Command that was run (e.g., "create", "up", "auth login")
    pub command: String,

    /// CLI version
    pub cli_version: String,

    /// Operating system
    pub os: String,

    /// CPU architecture
    pub arch: String,

    /// Whether the command succeeded
    pub success: bool,

    /// Error code if command failed (not the full error message)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,

    /// Sanitized failure category (`auth_required`, `network_error`, …).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,

    /// Command duration in milliseconds
    pub duration_ms: u64,

    /// Template name (for `a4 create` only) or SDK language.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,

    /// `single` or `all` for `stack_deployed`. Never a stack name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stack_scope: Option<String>,

    /// Coarse protocol deployment status (`active`, `review-required`, …).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deployment_status: Option<String>,

    /// `project` or `global` for `mcp_connected`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcp_scope: Option<String>,

    /// `curl`, `npm`, `cargo`, `agent`, or `unknown`.
    pub install_source: String,

    /// Always false on ingested events. CI runs are not sent.
    pub is_ci: bool,

    /// Anonymous installation ID
    pub anonymous_id: String,

    /// Session ID (unique per CLI invocation)
    pub session_id: String,

    /// ISO 8601 timestamp
    pub timestamp: String,
}

/// Get a session ID for the current CLI invocation.
/// This is unique per process and helps group events from the same command.
fn get_session_id() -> &'static str {
    static SESSION_ID: OnceLock<String> = OnceLock::new();
    SESSION_ID.get_or_init(|| Uuid::new_v4().to_string())
}

struct Capture<'a> {
    event: &'a str,
    command: &'a str,
    success: bool,
    error_code: Option<&'a str>,
    duration: Duration,
    template: Option<&'a str>,
    reason: Option<&'a str>,
    stack_scope: Option<&'a str>,
    deployment_status: Option<&'a str>,
    mcp_scope: Option<&'a str>,
}

fn capture(spec: Capture<'_>) {
    if !should_collect() {
        return;
    }

    let identity = prepare_identity();
    if identity.opted_out {
        return;
    }
    if identity.emit_first_run {
        spawn_send(build_event(
            &identity.anonymous_id,
            Capture {
                event: "first_run",
                command: "first_run",
                success: true,
                error_code: None,
                duration: Duration::ZERO,
                template: None,
                reason: None,
                stack_scope: None,
                deployment_status: None,
                mcp_scope: None,
            },
        ));
    }
    if spec.event != "first_run" {
        spawn_send(build_event(&identity.anonymous_id, spec));
    }
}

fn build_event(anonymous_id: &str, spec: Capture<'_>) -> TelemetryEvent {
    let reason = spec
        .reason
        .map(sanitize_reason)
        .or_else(|| spec.error_code.map(sanitize_reason));
    TelemetryEvent {
        event: sanitize_command(spec.event),
        command: sanitize_command(spec.command),
        cli_version: env!("CARGO_PKG_VERSION").to_string(),
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        success: spec.success,
        error_code: spec.error_code.map(sanitize_reason),
        reason,
        duration_ms: spec.duration.as_millis() as u64,
        template: spec.template.map(sanitize_label),
        stack_scope: spec.stack_scope.and_then(sanitize_scope),
        deployment_status: spec.deployment_status.map(sanitize_deployment_status),
        mcp_scope: spec.mcp_scope.and_then(sanitize_mcp_scope),
        install_source: install_source_label().to_string(),
        is_ci: in_ci(),
        anonymous_id: anonymous_id.to_string(),
        session_id: get_session_id().to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
    }
}

fn sanitize_command(value: &str) -> String {
    if !value.is_empty()
        && value.len() <= 64
        && value.chars().all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || character == '_'
                || character == '-'
                || character == ' '
        })
    {
        value.to_string()
    } else {
        "unknown".to_string()
    }
}

fn sanitize_label(value: &str) -> String {
    if !value.is_empty()
        && value.len() <= 64
        && value.chars().all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || character == '_'
                || character == '-'
        })
    {
        value.to_string()
    } else {
        "unknown".to_string()
    }
}

fn sanitize_reason(value: &str) -> String {
    match value {
        "unknown_error" | "unknown_template" | "already_exists" | "not_found" | "build_failed"
        | "auth_required" | "network_error" | "permission_denied" | "config_error"
        | "api_error" => value.to_string(),
        _ => "unknown_error".to_string(),
    }
}

fn sanitize_scope(value: &str) -> Option<String> {
    match value {
        "single" | "all" => Some(value.to_string()),
        _ => None,
    }
}

fn sanitize_deployment_status(value: &str) -> String {
    match value {
        "active" | "activating" | "disabled" | "review-required" | "pending" | "dry-run" => {
            value.to_string()
        }
        _ => "other".to_string(),
    }
}

fn sanitize_mcp_scope(value: &str) -> Option<String> {
    match value {
        "project" | "global" => Some(value.to_string()),
        _ => None,
    }
}

// ============================================================================
// Event Recording
// ============================================================================

/// Record a command execution event.
///
/// This is fire-and-forget: spawns a thread to send the event and returns immediately.
/// Errors are silently ignored to ensure telemetry never affects CLI performance.
pub fn record_command(
    command: &str,
    success: bool,
    error_code: Option<&str>,
    duration: Duration,
    extra: Option<HashMap<String, String>>,
) {
    let template = extra
        .as_ref()
        .and_then(|values| values.get("template"))
        .map(String::as_str);
    // Keep the owned template alive for the capture borrow.
    let owned = template.map(str::to_string);
    capture(Capture {
        event: "command_executed",
        command,
        success,
        error_code,
        duration,
        template: owned.as_deref(),
        reason: None,
        stack_scope: None,
        deployment_status: None,
        mcp_scope: None,
    });
}

pub fn record_first_run() {
    capture(Capture {
        event: "first_run",
        command: "first_run",
        success: true,
        error_code: None,
        duration: Duration::ZERO,
        template: None,
        reason: None,
        stack_scope: None,
        deployment_status: None,
        mcp_scope: None,
    });
}

pub fn record_template_selected(template: &str) {
    let template = sanitize_label(template);
    remember_template(&template);
    capture(Capture {
        event: "template_selected",
        command: "create",
        success: true,
        error_code: None,
        duration: Duration::ZERO,
        template: Some(&template),
        reason: None,
        stack_scope: None,
        deployment_status: None,
        mcp_scope: None,
    });
}

pub fn record_create_completed(template: &str, duration: Duration) {
    let template = sanitize_label(template);
    capture(Capture {
        event: "create_completed",
        command: "create",
        success: true,
        error_code: None,
        duration,
        template: Some(&template),
        reason: None,
        stack_scope: None,
        deployment_status: None,
        mcp_scope: None,
    });
}

pub fn record_create_failed(template: Option<&str>, reason: &str, duration: Duration) {
    let stored = selected_template();
    let template = template.or(stored.as_deref()).map(sanitize_label);
    let reason = sanitize_reason(reason);
    capture(Capture {
        event: "create_failed",
        command: "create",
        success: false,
        error_code: Some(&reason),
        duration,
        template: template.as_deref(),
        reason: Some(&reason),
        stack_scope: None,
        deployment_status: None,
        mcp_scope: None,
    });
}

/// A stack deploy succeeded. `scope` is `single` or `all`. The stack name is
/// not recorded.
pub fn record_stack_deployed(scope: &str, duration: Duration) {
    capture(Capture {
        event: "stack_deployed",
        command: "up",
        success: true,
        error_code: None,
        duration,
        template: None,
        reason: None,
        stack_scope: Some(scope),
        deployment_status: None,
        mcp_scope: None,
    });
}

pub fn record_up_failed(reason: &str, duration: Duration) {
    let reason = sanitize_reason(reason);
    capture(Capture {
        event: "up_failed",
        command: "up",
        success: false,
        error_code: Some(&reason),
        duration,
        template: None,
        reason: Some(&reason),
        stack_scope: None,
        deployment_status: None,
        mcp_scope: None,
    });
}

/// `a4 protocol deploy` finished without error. Distinct from `stack_deployed`.
/// Status is a coarse token; deployment ids and slugs are not sent.
pub fn record_protocol_deployed(status: &str, duration: Duration) {
    capture(Capture {
        event: "protocol_deployed",
        command: "protocol deploy",
        success: true,
        error_code: None,
        duration,
        template: None,
        reason: None,
        stack_scope: None,
        deployment_status: Some(status),
        mcp_scope: None,
    });
}

/// `a4 init` wrote or updated an agent MCP config.
pub fn record_mcp_connected(scope: &str) {
    capture(Capture {
        event: "mcp_connected",
        command: "init",
        success: true,
        error_code: None,
        duration: Duration::ZERO,
        template: None,
        reason: None,
        stack_scope: None,
        deployment_status: None,
        mcp_scope: Some(scope),
    });
}

pub fn record_sdk_generated(language: &str) {
    let language = sanitize_label(language);
    capture(Capture {
        event: "sdk_generated",
        command: "sdk",
        success: true,
        error_code: None,
        duration: Duration::ZERO,
        template: Some(&language),
        reason: None,
        stack_scope: None,
        deployment_status: None,
        mcp_scope: None,
    });
}

// ============================================================================
// Network
// ============================================================================

/// PostHog capture request format
#[derive(Serialize)]
struct PostHogCapture<'a> {
    api_key: &'static str,
    event: &'a str,
    distinct_id: &'a str,
    properties: &'a TelemetryEvent,
    timestamp: &'a str,
}

/// Send an event to PostHog.
/// This is a blocking call - should only be called from a spawned thread.
fn send_event(event: &TelemetryEvent) -> Result<()> {
    // Scrub any potential PII before sending
    let event = scrub_event(event);

    let payload = PostHogCapture {
        api_key: POSTHOG_API_KEY,
        event: &event.event,
        distinct_id: &event.anonymous_id,
        properties: &event,
        timestamp: &event.timestamp,
    };

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;

    client
        .post(POSTHOG_ENDPOINT)
        .json(&payload)
        .send()
        .context("Failed to send telemetry")?;

    Ok(())
}

/// Scrub any potential PII from the event before sending.
/// This is a safety net - we shouldn't be collecting PII in the first place.
fn scrub_event(event: &TelemetryEvent) -> TelemetryEvent {
    let mut event = event.clone();
    event.command = scrub_text(&event.command);
    event.event = scrub_text(&event.event);
    event.error_code = event.error_code.as_deref().map(scrub_text);
    event.reason = event.reason.as_deref().map(scrub_text);
    event.template = event.template.as_deref().map(scrub_text);
    event.stack_scope = event.stack_scope.as_deref().map(scrub_text);
    event.deployment_status = event.deployment_status.as_deref().map(scrub_text);
    event.mcp_scope = event.mcp_scope.as_deref().map(scrub_text);
    event.install_source = scrub_text(&event.install_source);
    event
}

fn scrub_text(value: &str) -> String {
    let mut value = value.to_string();
    for pattern in scrubbers() {
        value = pattern.replace_all(&value, "[REDACTED]").to_string();
    }
    value
}

fn scrubbers() -> &'static [Regex] {
    static SCRUBBERS: OnceLock<Vec<Regex>> = OnceLock::new();
    SCRUBBERS.get_or_init(|| {
        [
            r"/Users/[^/\s]+",
            r"/home/[^/\s]+",
            r"C:\\Users\\[^\\]+",
            r"[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}",
            r"(?i)\ba4_(?:sk|pk)_[A-Za-z0-9_\-]+",
            r"(?i)\bphc_[A-Za-z0-9]+",
            r"(?i)\bbearer\s+[A-Za-z0-9._\-+/=]+",
        ]
        .into_iter()
        .filter_map(|pattern| Regex::new(pattern).ok())
        .collect()
    })
}

// ============================================================================
// Error Codes
// ============================================================================

/// Extract a categorized error code from an error.
/// Returns a short code like "auth_failed", "network_error", etc.
/// Never returns the full error message (may contain PII).
pub fn extract_error_code(error: &anyhow::Error) -> Option<String> {
    Some(categorize_error(&error.to_string()))
}

/// Sanitized failure category safe to send as `reason`.
pub fn failure_reason(error: &anyhow::Error) -> String {
    sanitize_reason(&categorize_error(&error.to_string()))
}

fn categorize_error(message: &str) -> String {
    let msg = message.to_lowercase();

    if msg.contains("unknown template") {
        return "unknown_template".to_string();
    }
    if msg.contains("already exists") {
        return "already_exists".to_string();
    }
    if msg.contains("no stacks") {
        return "not_found".to_string();
    }
    if msg.contains("build failed") || msg.contains("compilation") {
        return "build_failed".to_string();
    }
    if msg.contains("not authenticated") || msg.contains("auth") {
        return "auth_required".to_string();
    }
    if msg.contains("network") || msg.contains("connection") || msg.contains("timeout") {
        return "network_error".to_string();
    }
    if msg.contains("not found") {
        return "not_found".to_string();
    }
    if msg.contains("permission") || msg.contains("denied") {
        return "permission_denied".to_string();
    }
    if msg.contains("config") || msg.contains("parse") || msg.contains("invalid") {
        return "config_error".to_string();
    }
    if msg.contains("api error") {
        return "api_error".to_string();
    }

    "unknown_error".to_string()
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_event() -> TelemetryEvent {
        TelemetryEvent {
            event: "test".to_string(),
            command: "Error at /Users/john/project".to_string(),
            cli_version: "0.1.0".to_string(),
            os: "darwin".to_string(),
            arch: "arm64".to_string(),
            success: false,
            error_code: Some("Error in /home/jane/code a4_sk_supersecret".to_string()),
            reason: Some("token bearer abc.def for jane@example.com".to_string()),
            duration_ms: 100,
            template: Some("/Users/john/template".to_string()),
            stack_scope: None,
            deployment_status: None,
            mcp_scope: None,
            install_source: "unknown".to_string(),
            is_ci: false,
            anonymous_id: "test-id".to_string(),
            session_id: "session-id".to_string(),
            timestamp: "2024-01-01T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn test_default_config() {
        let config = TelemetryConfig::default();
        assert!(config.enabled);
        assert!(config.anonymous_id.is_none());
        assert!(!config.consent_shown);
        assert!(!config.first_run_recorded);
    }

    #[test]
    fn test_scrub_event_removes_paths_and_secrets() {
        let scrubbed = scrub_event(&sample_event());
        assert!(!scrubbed.command.contains("john"));
        assert!(!scrubbed.error_code.as_ref().unwrap().contains("jane"));
        assert!(!scrubbed.error_code.as_ref().unwrap().contains("a4_sk_"));
        assert!(!scrubbed.reason.as_ref().unwrap().contains("abc.def"));
        assert!(!scrubbed.reason.as_ref().unwrap().contains("example.com"));
        assert!(!scrubbed.template.as_ref().unwrap().contains("john"));
        assert!(scrubbed.command.contains("[REDACTED]"));
    }

    #[test]
    fn test_error_code_extraction() {
        let auth_error = anyhow::anyhow!("Not authenticated. Run 'a4 auth login' first.");
        assert_eq!(
            extract_error_code(&auth_error),
            Some("auth_required".to_string())
        );

        let network_error = anyhow::anyhow!("Connection timeout");
        assert_eq!(
            extract_error_code(&network_error),
            Some("network_error".to_string())
        );
    }

    #[test]
    fn failure_reason_is_a_category_not_the_message() {
        let error =
            anyhow::anyhow!("Not authenticated token a4_sk_secretvalue at /Users/jane/project");
        let reason = failure_reason(&error);
        assert_eq!(reason, "auth_required");
        assert!(!reason.contains("a4_sk"));
        assert!(!reason.contains("jane"));
        assert!(!reason.contains('/'));
    }

    #[test]
    fn failure_categories_for_create_and_up() {
        assert_eq!(
            categorize_error("Unknown template: foo"),
            "unknown_template"
        );
        assert_eq!(
            categorize_error("Directory 'app' already exists"),
            "already_exists"
        );
        assert_eq!(categorize_error("No stacks found to deploy"), "not_found");
        assert_eq!(categorize_error("Build failed: rustc"), "build_failed");
    }

    #[test]
    fn labels_and_reasons_reject_paths_and_secrets() {
        assert_eq!(sanitize_label("/tmp/project"), "unknown");
        assert_eq!(sanitize_label("react-ore"), "react-ore");
        assert_eq!(sanitize_reason("a4_sk_secret"), "unknown_error");
        assert_eq!(sanitize_reason("network_error"), "network_error");
        assert_eq!(sanitize_deployment_status("active"), "active");
        assert_eq!(sanitize_deployment_status("id-dep_123"), "other");
        assert_eq!(sanitize_mcp_scope("project").as_deref(), Some("project"));
        assert_eq!(sanitize_mcp_scope("claude-code"), None);
        assert_eq!(sanitize_scope("single").as_deref(), Some("single"));
        assert_eq!(sanitize_scope("my-stack"), None);
    }

    #[test]
    fn install_source_classification() {
        assert_eq!(
            classify_install_source(true, Some("sh"), ExeInstallKind::Cargo),
            "agent"
        );
        assert_eq!(
            classify_install_source(false, Some("sh"), ExeInstallKind::Other),
            "curl"
        );
        assert_eq!(
            classify_install_source(false, Some("ps1"), ExeInstallKind::Other),
            "curl"
        );
        assert_eq!(
            classify_install_source(false, Some("npm"), ExeInstallKind::Other),
            "npm"
        );
        assert_eq!(
            classify_install_source(false, Some("cargo"), ExeInstallKind::Other),
            "cargo"
        );
        assert_eq!(
            classify_install_source(false, None, ExeInstallKind::Cargo),
            "cargo"
        );
        assert_eq!(
            classify_install_source(false, Some("manual"), ExeInstallKind::Npm),
            "npm"
        );
        assert_eq!(
            classify_install_source(false, Some("self-update"), ExeInstallKind::Other),
            "unknown"
        );
        assert_eq!(
            classify_install_source(false, None, ExeInstallKind::Other),
            "unknown"
        );
        assert_eq!(
            classify_install_source(false, Some("git"), ExeInstallKind::Cargo),
            "unknown"
        );
    }

    #[test]
    fn ci_detection_requires_a_truthy_value() {
        let truthy = |key: &str| {
            Some(match key {
                "CI" => "true".to_string(),
                _ => String::new(),
            })
        };
        assert!(ci_signaled(truthy));

        let github = |key: &str| (key == "GITHUB_ACTIONS").then(|| "true".to_string());
        assert!(ci_signaled(github));

        let falsey = |key: &str| (key == "CI").then(|| "false".to_string());
        assert!(!ci_signaled(falsey));

        let unset = |_key: &str| None;
        assert!(!ci_signaled(unset));

        let empty = |key: &str| (key == "CI").then(String::new);
        assert!(!ci_signaled(empty));
    }

    #[test]
    fn first_run_emits_only_for_a_new_anonymous_id() {
        assert!(should_emit_first_run(None));
        assert!(!should_emit_first_run(Some("already")));
    }

    #[test]
    fn assigning_an_id_does_not_turn_telemetry_back_on() {
        let mut config = TelemetryConfig {
            enabled: false,
            ..TelemetryConfig::default()
        };
        assert!(assign_anonymous_id(&mut config));
        assert!(!config.enabled);
        assert!(config.first_run_recorded);
        assert!(config.anonymous_id.is_some());
        let id = config.anonymous_id.clone();
        assert!(!assign_anonymous_id(&mut config));
        assert_eq!(config.anonymous_id, id);
        assert!(!config.enabled);
    }

    #[test]
    fn old_config_without_first_run_flag_still_parses() {
        let parsed: TelemetryConfig =
            serde_json::from_str(r#"{"enabled":true,"anonymous_id":"abc","consent_shown":true}"#)
                .unwrap();
        assert!(!parsed.first_run_recorded);
        assert_eq!(parsed.anonymous_id.as_deref(), Some("abc"));
        assert!(!should_emit_first_run(parsed.anonymous_id.as_deref()));
    }
}
