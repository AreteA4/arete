//! Shared UI utilities for consistent terminal output.
//!
//! This module provides standardized formatting, spinners, progress bars,
//! and output helpers to ensure a consistent look and feel across all CLI commands.

#![allow(dead_code)]

use colored::Colorize;
use indicatif::{ProgressBar, ProgressStyle};
use std::time::Duration;

/// Standard symbols used throughout the CLI for consistent visual language.
pub mod symbols {
    /// Arrow for action/progress indicators
    pub const ARROW: &str = "→";
    /// Checkmark for success
    pub const SUCCESS: &str = "✓";
    /// X mark for failure
    pub const FAILURE: &str = "✗";
    /// Warning/attention indicator
    pub const WARNING: &str = "!";
    /// Bullet point
    pub const BULLET: &str = "•";
    /// Active/running indicator
    pub const ACTIVE: &str = "●";
    /// Inactive/stopped indicator
    pub const INACTIVE: &str = "○";
    /// In-progress indicator
    pub const IN_PROGRESS: &str = "◐";
    /// Up arrow for uploads/pushes
    pub const UPLOAD: &str = "↑";
    /// Plus for additions
    pub const PLUS: &str = "+";
    /// Equals for unchanged
    pub const EQUALS: &str = "=";
}

/// Print a step header with the action arrow.
pub fn print_step(message: &str) {
    println!("{} {}", symbols::ARROW.blue().bold(), message);
}

/// Print a numbered step header.
pub fn print_numbered_step(num: u32, message: &str) {
    println!("\n{} {}", num.to_string().blue().bold(), message);
}

/// Print a success message.
pub fn print_success(message: &str) {
    println!("{} {}", symbols::SUCCESS.green().bold(), message);
}

/// Print a failure message.
pub fn print_error(message: &str) {
    println!("{} {}", symbols::FAILURE.red().bold(), message);
}

/// Print a warning message.
pub fn print_warning(message: &str) {
    println!("{} {}", symbols::WARNING.yellow().bold(), message);
}

/// Print a dimmed info line (indented).
pub fn print_info(message: &str) {
    println!("  {}", message.dimmed());
}

/// Print a section header.
pub fn print_section(title: &str) {
    println!();
    println!("{}", title.bold());
    println!("{}", "─".repeat(50).dimmed());
}

/// Print a horizontal divider.
pub fn print_divider() {
    println!("{}", "━".repeat(50).dimmed());
}

/// Create a spinner with a consistent style.
pub fn create_spinner(message: &str) -> ProgressBar {
    let spinner = ProgressBar::new_spinner();
    spinner.set_style(
        ProgressStyle::default_spinner()
            .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏")
            .template("{spinner:.blue} {msg}")
            .expect("Invalid spinner template"),
    );
    spinner.set_message(message.to_string());
    spinner.enable_steady_tick(Duration::from_millis(80));
    spinner
}

/// Create a progress bar with a consistent style.
pub fn create_progress_bar(total: u64, message: &str) -> ProgressBar {
    let bar = ProgressBar::new(total);
    bar.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.blue} [{bar:30.green/dim}] {pos}% {msg}")
            .expect("Invalid progress bar template")
            .progress_chars("█░░"),
    );
    bar.set_message(message.to_string());
    bar.enable_steady_tick(Duration::from_millis(100));
    bar
}

/// Create a progress bar for build watching.
pub fn create_build_progress_bar() -> ProgressBar {
    let bar = ProgressBar::new(100);
    bar.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.blue} [{bar:30.green/dim}] {pos}% {msg}")
            .expect("Invalid progress bar template")
            .progress_chars("█░░"),
    );
    bar.enable_steady_tick(Duration::from_millis(100));
    bar
}

/// Format a build status with consistent coloring.
pub fn format_build_status(status: &str) -> String {
    match status.to_lowercase().as_str() {
        "pending" => status.yellow().to_string(),
        "uploading" => status.yellow().to_string(),
        "queued" => status.yellow().to_string(),
        "building" => status.blue().to_string(),
        "pushing" => status.blue().to_string(),
        "deploying" => status.blue().to_string(),
        "completed" => status.green().bold().to_string(),
        "failed" => status.red().bold().to_string(),
        "cancelled" => status.dimmed().to_string(),
        _ => status.to_string(),
    }
}

/// Format a deployment status with consistent coloring.
pub fn format_deployment_status(status: &str) -> String {
    match status.to_lowercase().as_str() {
        "active" => "active".green().to_string(),
        "updating" => "updating".yellow().to_string(),
        "stopped" => "stopped".dimmed().to_string(),
        "failed" => "failed".red().to_string(),
        _ => status.to_string(),
    }
}

/// Humanize build phase names for user-friendly display.
pub fn humanize_phase(phase: &str) -> &str {
    match phase.to_uppercase().as_str() {
        "SUBMITTED" => "Queued",
        "PROVISIONING" => "Starting build environment",
        "DOWNLOAD_SOURCE" => "Preparing",
        "INSTALL" => "Installing dependencies",
        "PRE_BUILD" => "Preparing build",
        "BUILD" => "Building",
        "POST_BUILD" => "Finalizing build",
        "UPLOAD_ARTIFACTS" => "Publishing image",
        "FINALIZING" => "Deploying",
        "COMPLETED" => "Completed",
        _ => phase,
    }
}

/// Format a relative time string from a timestamp.
pub fn format_relative_time(timestamp: &str) -> Option<String> {
    let parsed = chrono::DateTime::parse_from_rfc3339(timestamp).ok()?;
    let now = chrono::Utc::now();
    let duration = now.signed_duration_since(parsed);

    let seconds = duration.num_seconds();
    if seconds < 0 {
        return Some("just now".to_string());
    }

    if seconds < 60 {
        return Some("just now".to_string());
    }

    let minutes = duration.num_minutes();
    if minutes < 60 {
        return Some(format!(
            "{} minute{} ago",
            minutes,
            if minutes == 1 { "" } else { "s" }
        ));
    }

    let hours = duration.num_hours();
    if hours < 24 {
        return Some(format!(
            "{} hour{} ago",
            hours,
            if hours == 1 { "" } else { "s" }
        ));
    }

    let days = duration.num_days();
    if days < 30 {
        return Some(format!(
            "{} day{} ago",
            days,
            if days == 1 { "" } else { "s" }
        ));
    }

    Some(parsed.format("%Y-%m-%d").to_string())
}

/// Get the current time formatted for log output.
pub fn timestamp_now() -> String {
    chrono::Local::now().format("%H:%M:%S").to_string()
}

/// Default timeout for build polling in seconds.
pub const DEFAULT_POLL_TIMEOUT_SECS: u64 = 1800; // 30 minutes

/// Default polling interval in milliseconds.
pub const DEFAULT_POLL_INTERVAL_MS: u64 = 500;

/// Polling interval for status checks in seconds.
pub const STATUS_POLL_INTERVAL_SECS: u64 = 3;

// ============================================================================
// Interactivity and process exit control
// ============================================================================

/// Whether prompts may be shown.
///
/// False when `--non-interactive` or `-y/--yes` was passed (main() mirrors
/// them into `A4_NON_INTERACTIVE=1` / `A4_YES=1` so child processes inherit
/// them), when `A4_NON_INTERACTIVE=1` or `CI` is set in the environment, or
/// when stdin is not a terminal. Every prompt in the CLI must be guarded by
/// this; when it is false and a required input is missing, commands return an
/// error naming the exact flags to pass instead.
pub fn interactive() -> bool {
    use std::io::IsTerminal;
    if env_flag("A4_NON_INTERACTIVE") || env_flag("A4_YES") {
        return false;
    }
    if std::env::var_os("CI").is_some() {
        return false;
    }
    std::io::stdin().is_terminal()
}

/// Whether `-y/--yes` was passed (mirrored into `A4_YES=1` by main()).
pub fn assume_yes() -> bool {
    env_flag("A4_YES")
}

fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .map(|value| {
            let value = value.trim();
            value == "1" || value.eq_ignore_ascii_case("true") || value.eq_ignore_ascii_case("yes")
        })
        .unwrap_or(false)
}

/// Error carrying an explicit process exit code.
///
/// Return `Err(ExitCode(n).into())` from a command to make `main()` exit with
/// `n` without printing the generic `Error:` line (the command has already
/// reported everything it wants to). Used for `a4 self update --check` (10 =
/// update available) and `a4 doctor` (1 = a failing check).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExitCode(pub i32);

impl std::fmt::Display for ExitCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "exit code {}", self.0)
    }
}

impl std::error::Error for ExitCode {}

#[cfg(test)]
mod interactivity_tests {
    use super::*;

    #[test]
    fn env_flag_accepts_common_truthy_spellings() {
        let key = "A4_TEST_ENV_FLAG_UNIQUE";
        for (value, expected) in [
            ("1", true),
            ("true", true),
            ("YES", true),
            ("0", false),
            ("", false),
        ] {
            std::env::set_var(key, value);
            assert_eq!(env_flag(key), expected, "value {value:?}");
        }
        std::env::remove_var(key);
        assert!(!env_flag(key));
    }
}
