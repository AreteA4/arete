//! Privacy-respecting telemetry for the Arete CLI.
//!
//! This module implements anonymous usage tracking to help improve the CLI.
//! Key principles:
//! - Opt-out with transparent first-run disclosure
//! - No PII (paths, usernames, secrets) ever collected
//! - Non-blocking: telemetry never adds latency to commands
//! - Minimal data: only what's needed to answer specific questions
//!
//! Users can disable telemetry via:
//! - `a4 telemetry disable`
//! - `DO_NOT_TRACK=1` environment variable
//! - `ARETE_TELEMETRY_DISABLED=1` environment variable

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

fn get_pending_events() -> &'static Mutex<Vec<JoinHandle<()>>> {
    PENDING_EVENTS.get_or_init(|| Mutex::new(Vec::new()))
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
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create config directory: {:?}", parent))?;
        }

        let contents = serde_json::to_string_pretty(self)?;
        fs::write(&path, contents)
            .with_context(|| format!("Failed to write telemetry config: {:?}", path))?;

        Ok(())
    }

    /// Get or create the anonymous ID
    pub fn get_or_create_anonymous_id(&mut self) -> String {
        if let Some(ref id) = self.anonymous_id {
            return id.clone();
        }

        let id = Uuid::new_v4().to_string();
        self.anonymous_id = Some(id.clone());
        let _ = self.save(); // Best effort save
        id
    }
}

/// Get the path to the telemetry config file
fn config_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".arete")
        .join("telemetry.json")
}

// ============================================================================
// Collection Logic
// ============================================================================

/// Check if telemetry should be collected.
///
/// Returns false if:
/// - `DO_NOT_TRACK=1` is set (standard)
/// - `ARETE_TELEMETRY_DISABLED=1` is set
/// - Telemetry is disabled in config
/// - CI environment is detected
pub fn should_collect() -> bool {
    // Check standard DO_NOT_TRACK
    if std::env::var("DO_NOT_TRACK")
        .map(|v| v == "1" || v.to_lowercase() == "true")
        .unwrap_or(false)
    {
        return false;
    }

    // Check Arete-specific disable
    if std::env::var("ARETE_TELEMETRY_DISABLED")
        .map(|v| v == "1" || v.to_lowercase() == "true")
        .unwrap_or(false)
    {
        return false;
    }

    // Check CI environments (don't track CI runs)
    if std::env::var("CI").is_ok() || std::env::var("GITHUB_ACTIONS").is_ok() {
        return false;
    }

    // Check local config
    let config = TelemetryConfig::load();
    config.enabled
}

/// Enable telemetry collection
pub fn enable() -> Result<()> {
    let mut config = TelemetryConfig::load();
    config.enabled = true;
    config.save()
}

/// Disable telemetry collection
pub fn disable() -> Result<()> {
    let mut config = TelemetryConfig::load();
    config.enabled = false;
    config.save()
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

// ============================================================================
// Consent Banner
// ============================================================================

/// Show the consent banner if it hasn't been shown yet.
/// This is non-blocking and prints to stderr to avoid breaking pipes.
pub fn show_consent_banner_if_needed() {
    let mut config = TelemetryConfig::load();

    if config.consent_shown {
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

    config.consent_shown = true;
    config.consent_shown_at = Some(chrono::Utc::now().to_rfc3339());
    let _ = config.save();

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

    /// Command duration in milliseconds
    pub duration_ms: u64,

    /// Template name (for `a4 create` only)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,

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
    if !should_collect() {
        return;
    }

    let mut config = TelemetryConfig::load();
    let anonymous_id = config.get_or_create_anonymous_id();

    let event = TelemetryEvent {
        event: "command_executed".to_string(),
        command: command.to_string(),
        cli_version: env!("CARGO_PKG_VERSION").to_string(),
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        success,
        error_code: error_code.map(|s| s.to_string()),
        duration_ms: duration.as_millis() as u64,
        template: extra.as_ref().and_then(|e| e.get("template").cloned()),
        anonymous_id,
        session_id: get_session_id().to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
    };

    spawn_send(event);
}

pub fn record_first_run() {
    if !should_collect() {
        return;
    }

    let mut config = TelemetryConfig::load();
    let anonymous_id = config.get_or_create_anonymous_id();

    let event = TelemetryEvent {
        event: "first_run".to_string(),
        command: String::new(),
        cli_version: env!("CARGO_PKG_VERSION").to_string(),
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        success: true,
        error_code: None,
        duration_ms: 0,
        template: None,
        anonymous_id,
        session_id: get_session_id().to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
    };

    spawn_send(event);
}

pub fn record_template_selected(template: &str) {
    if !should_collect() {
        return;
    }

    let mut config = TelemetryConfig::load();
    let anonymous_id = config.get_or_create_anonymous_id();

    let event = TelemetryEvent {
        event: "create_template_selected".to_string(),
        command: "create".to_string(),
        cli_version: env!("CARGO_PKG_VERSION").to_string(),
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        success: true,
        error_code: None,
        duration_ms: 0,
        template: Some(template.to_string()),
        anonymous_id,
        session_id: get_session_id().to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
    };

    spawn_send(event);
}

pub fn record_create_completed(template: &str, duration: Duration) {
    if !should_collect() {
        return;
    }

    let mut config = TelemetryConfig::load();
    let anonymous_id = config.get_or_create_anonymous_id();

    let event = TelemetryEvent {
        event: "create_completed".to_string(),
        command: "create".to_string(),
        cli_version: env!("CARGO_PKG_VERSION").to_string(),
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        success: true,
        error_code: None,
        duration_ms: duration.as_millis() as u64,
        template: Some(template.to_string()),
        anonymous_id,
        session_id: get_session_id().to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
    };

    spawn_send(event);
}

pub fn record_stack_deployed(stack_name: &str, duration: Duration) {
    if !should_collect() {
        return;
    }

    let mut config = TelemetryConfig::load();
    let anonymous_id = config.get_or_create_anonymous_id();

    let mut event = TelemetryEvent {
        event: "stack_deployed".to_string(),
        command: "up".to_string(),
        cli_version: env!("CARGO_PKG_VERSION").to_string(),
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        success: true,
        error_code: None,
        duration_ms: duration.as_millis() as u64,
        template: None,
        anonymous_id,
        session_id: get_session_id().to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
    };

    // Store stack count (not name) to avoid leaking project info
    event.template = Some(format!(
        "stack_count:{}",
        if stack_name.is_empty() { "all" } else { "1" }
    ));

    spawn_send(event);
}

pub fn record_sdk_generated(language: &str) {
    if !should_collect() {
        return;
    }

    let mut config = TelemetryConfig::load();
    let anonymous_id = config.get_or_create_anonymous_id();

    let event = TelemetryEvent {
        event: "sdk_generated".to_string(),
        command: "sdk".to_string(),
        cli_version: env!("CARGO_PKG_VERSION").to_string(),
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        success: true,
        error_code: None,
        duration_ms: 0,
        template: Some(language.to_string()),
        anonymous_id,
        session_id: get_session_id().to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
    };

    spawn_send(event);
}

pub fn record_stack_rollback(success: bool) {
    if !should_collect() {
        return;
    }

    let mut config = TelemetryConfig::load();
    let anonymous_id = config.get_or_create_anonymous_id();

    let event = TelemetryEvent {
        event: "stack_rollback".to_string(),
        command: "stack rollback".to_string(),
        cli_version: env!("CARGO_PKG_VERSION").to_string(),
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        success,
        error_code: None,
        duration_ms: 0,
        template: None,
        anonymous_id,
        session_id: get_session_id().to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
    };

    spawn_send(event);
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

    // Patterns that might contain usernames or sensitive paths
    let patterns = [
        r"/Users/[^/\s]+",                                 // macOS home dirs
        r"/home/[^/\s]+",                                  // Linux home dirs
        r"C:\\Users\\[^\\]+",                              // Windows home dirs
        r"[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}", // Email addresses
    ];

    for pattern in &patterns {
        if let Ok(re) = Regex::new(pattern) {
            event.command = re.replace_all(&event.command, "[REDACTED]").to_string();
            if let Some(ref code) = event.error_code {
                event.error_code = Some(re.replace_all(code, "[REDACTED]").to_string());
            }
        }
    }

    event
}

// ============================================================================
// Error Codes
// ============================================================================

/// Extract a categorized error code from an error.
/// Returns a short code like "auth_failed", "network_error", etc.
/// Never returns the full error message (may contain PII).
pub fn extract_error_code(error: &anyhow::Error) -> Option<String> {
    let msg = error.to_string().to_lowercase();

    // Categorize common errors
    if msg.contains("not authenticated") || msg.contains("auth") {
        return Some("auth_required".to_string());
    }
    if msg.contains("network") || msg.contains("connection") || msg.contains("timeout") {
        return Some("network_error".to_string());
    }
    if msg.contains("not found") {
        return Some("not_found".to_string());
    }
    if msg.contains("permission") || msg.contains("denied") {
        return Some("permission_denied".to_string());
    }
    if msg.contains("config") || msg.contains("parse") || msg.contains("invalid") {
        return Some("config_error".to_string());
    }
    if msg.contains("api error") {
        return Some("api_error".to_string());
    }

    // Generic fallback - don't expose error message
    Some("unknown_error".to_string())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = TelemetryConfig::default();
        assert!(config.enabled);
        assert!(config.anonymous_id.is_none());
        assert!(!config.consent_shown);
    }

    #[test]
    fn test_scrub_event_removes_paths() {
        let event = TelemetryEvent {
            event: "test".to_string(),
            command: "Error at /Users/john/project".to_string(),
            cli_version: "0.1.0".to_string(),
            os: "darwin".to_string(),
            arch: "arm64".to_string(),
            success: false,
            error_code: Some("Error in /home/jane/code".to_string()),
            duration_ms: 100,
            template: None,
            anonymous_id: "test-id".to_string(),
            session_id: "session-id".to_string(),
            timestamp: "2024-01-01T00:00:00Z".to_string(),
        };

        let scrubbed = scrub_event(&event);
        assert!(!scrubbed.command.contains("john"));
        assert!(!scrubbed.error_code.as_ref().unwrap().contains("jane"));
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
}
