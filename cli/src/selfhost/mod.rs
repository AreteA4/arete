//! Self-hosting: `a4 self install|update|uninstall`, `a4 upgrade`, the
//! install receipt and the update nudge.
//!
//! Spec: `docs/internal/agent-first-onboarding.md` (WP2, WP4). Module map:
//!
//! * [`install`]   – `a4 self install` (verify, copy, receipt, PATH, output)
//! * [`uninstall`] – `a4 self uninstall`
//! * [`update`]    – `a4 self update` / `a4 upgrade`
//! * [`nudge`]     – throttled "a4 X is available" hint after other commands
//! * [`verify`]    – minisign + sha256 checks (key injected for tests)
//! * [`manifest`]  – `manifest.json` schema and release download helpers
//! * [`path_edit`] – rc-file / Windows user PATH edits and their removal
//! * [`receipt`]   – `~/.arete/receipt.json`
//! * [`latest`]    – `latest.json` pointer
//! * [`platform`]  – platform keys, asset names, URLs, PATH scanning
//! * [`keys`]      – the embedded release public key
//!
//! Shared helpers for the three commands (atomic binary copy, same-file
//! test, final stdout lines) live at the bottom of this file.

#![allow(dead_code)]

pub mod install;
pub mod keys;
pub mod latest;
pub mod manifest;
pub mod nudge;
pub mod path_edit;
pub mod platform;
pub mod receipt;
pub mod uninstall;
pub mod update;
pub mod verify;

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use clap::{Args, Subcommand};

#[derive(Subcommand)]
pub enum SelfCommands {
    /// Finish installing this binary into the install dir and write the receipt
    Install(InstallArgs),

    /// Download, verify and install a newer (or explicitly chosen) release
    Update(UpdateArgs),

    /// Remove the installed binary, receipt and PATH edits
    Uninstall(UninstallArgs),
}

#[derive(Args, Debug, Clone)]
pub struct InstallArgs {
    /// Install directory (default: $A4_INSTALL_DIR, $XDG_BIN_HOME, ~/.local/bin)
    #[arg(long, value_name = "DIR")]
    pub install_dir: Option<PathBuf>,

    /// Do not edit shell rc files / the Windows registry PATH
    #[arg(long)]
    pub no_modify_path: bool,

    /// How this binary was obtained: sh, ps1, npm or manual
    #[arg(long, value_name = "SOURCE", default_value = "manual")]
    pub source: String,

    /// checksums.txt downloaded with this binary (verified with --signature)
    #[arg(long, value_name = "FILE", requires = "signature")]
    pub checksums: Option<PathBuf>,

    /// checksums.txt.minisig for --checksums
    #[arg(long, value_name = "FILE", requires = "checksums")]
    pub signature: Option<PathBuf>,

    /// Overwrite an existing binary even if it is newer
    #[arg(long)]
    pub force: bool,
}

#[derive(Args, Debug, Clone)]
pub struct UpdateArgs {
    /// Target version (default: latest); downgrades are allowed when explicit
    pub version: Option<String>,

    /// Only report whether an update is available (exit 10 when it is)
    #[arg(long)]
    pub check: bool,

    /// Download and verify, then print the plan without replacing the binary
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Args, Debug, Clone)]
pub struct UninstallArgs {}

/// Dispatch for `a4 self <cmd>` and `a4 upgrade`.
pub fn run(command: SelfCommands, json: bool) -> Result<()> {
    match command {
        SelfCommands::Install(args) => install::run(args, json),
        SelfCommands::Update(args) => update::run(args, json),
        SelfCommands::Uninstall(args) => uninstall::run(args, json),
    }
}

/// Once-per-24h "a4 X is available" hint on stderr. Called from `main()`
/// after the command ran and before telemetry is flushed. Never fails, never
/// prints when `--json`, `CI`, `A4_NO_UPDATE_CHECK` or a non-TTY stderr.
pub fn maybe_nudge(command_name: &str, json: bool) {
    nudge::maybe_nudge(command_name, json);
}

// ============================================================================
// Shared helpers
// ============================================================================

/// The running CLI version.
pub(crate) fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// RFC 3339 UTC timestamp with second precision and a `Z` suffix.
pub(crate) fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// `$HOME` (`%USERPROFILE%`).
pub(crate) fn home_dir() -> Result<PathBuf> {
    dirs::home_dir().ok_or_else(|| anyhow!("Could not find home directory"))
}

/// Basename of `$SHELL` (`zsh`, `bash`, `fish`), if set.
pub(crate) fn shell_basename() -> Option<String> {
    let shell = std::env::var_os("SHELL")?;
    Path::new(&shell)
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
}

/// Whether PATH edits are disabled by the environment (`A4_NO_MODIFY_PATH=1`
/// or `CI`), independent of `--no-modify-path`.
pub(crate) fn path_edits_disabled() -> bool {
    std::env::var_os("A4_NO_MODIFY_PATH").is_some_and(|v| !v.is_empty())
        || std::env::var_os("CI").is_some()
}

/// Make a path absolute against the current directory (no symlink resolution).
pub(crate) fn absolute(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    let cwd = std::env::current_dir().context("Failed to read the current directory")?;
    Ok(cwd.join(path))
}

/// Whether two paths name the same file (device + inode on Unix; canonical
/// path elsewhere). False when either does not exist.
#[cfg(unix)]
pub(crate) fn same_file(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (fs::metadata(a), fs::metadata(b)) {
        (Ok(ma), Ok(mb)) => ma.dev() == mb.dev() && ma.ino() == mb.ino(),
        _ => false,
    }
}

#[cfg(not(unix))]
pub(crate) fn same_file(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => false,
    }
}

/// Copy `src` to `target` via `a4.tmp-<pid>` in the same directory and an
/// atomic rename. On Windows, when the target is a running executable the
/// rename fails, so the old file is renamed aside (`a4.old-<pid>.exe`, which
/// Windows allows for running images) and removed once it is no longer busy.
pub(crate) fn install_binary(src: &Path, target: &Path) -> Result<()> {
    let dir = target
        .parent()
        .ok_or_else(|| anyhow!("{} has no parent directory", target.display()))?;
    let pid = std::process::id();
    let tmp = dir.join(format!("a4.tmp-{pid}"));
    fs::copy(src, &tmp)
        .with_context(|| format!("Failed to copy {} to {}", src.display(), tmp.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o755))
            .with_context(|| format!("Failed to chmod {}", tmp.display()))?;
    }
    match fs::rename(&tmp, target) {
        Ok(()) => Ok(()),
        Err(error) => {
            #[cfg(windows)]
            {
                if target.exists() {
                    let aside = dir.join(format!("a4.old-{pid}.exe"));
                    if fs::rename(target, &aside).is_ok() && fs::rename(&tmp, target).is_ok() {
                        if fs::remove_file(&aside).is_err() {
                            // Still running: let self-replace schedule the delete.
                            let _ = self_replace::self_delete_at(&aside);
                        }
                        return Ok(());
                    }
                }
            }
            let _ = fs::remove_file(&tmp);
            Err(error).with_context(|| format!("Failed to move {} into place", target.display()))
        }
    }
}

/// The removal command for a shadowing `a4` (never executed, only printed).
pub(crate) fn removal_command(other: &Path) -> String {
    let text = other.to_string_lossy();
    if text.contains(".cargo") {
        "cargo uninstall a4-cli".to_string()
    } else if text.contains("node_modules") || text.contains("npm") || text.contains(".nvm") {
        "npm uninstall -g @usearete/a4".to_string()
    } else if cfg!(windows) {
        format!("Remove-Item \"{text}\"")
    } else {
        format!("rm \"{text}\"")
    }
}

/// The two stdout lines every install prints last, even with `--json`.
/// Agents parse `A4_BIN=`; the export line makes the binary usable in the
/// current shell, whose PATH was snapshotted before any rc edit.
pub(crate) fn print_final_lines(binary: &Path, install_dir: &Path, home: &Path) {
    println!("A4_BIN={}", binary.display());
    if cfg!(windows) {
        println!("{}", path_edit::powershell_line(install_dir, home));
    } else {
        println!(
            "{}",
            path_edit::posix_export_line(&path_edit::shell_dir(install_dir, home))
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_binary_copies_atomically_and_is_executable() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        fs::write(&src, "#!/bin/sh\necho hi\n").unwrap();
        let target = dir.path().join("bin").join(platform::binary_file_name());
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        install_binary(&src, &target).unwrap();
        assert_eq!(fs::read(&target).unwrap(), fs::read(&src).unwrap());
        assert!(!dir
            .path()
            .join("bin")
            .join(format!("a4.tmp-{}", std::process::id()))
            .exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&target).unwrap().permissions().mode() & 0o777,
                0o755
            );
        }
        // Overwrite works too.
        fs::write(&src, "v2").unwrap();
        install_binary(&src, &target).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"v2");
        assert!(same_file(&target, &target));
        assert!(!same_file(&target, &src));
    }

    #[test]
    fn removal_commands_match_origin() {
        assert_eq!(
            removal_command(Path::new("/Users/x/.cargo/bin/a4")),
            "cargo uninstall a4-cli"
        );
        assert_eq!(
            removal_command(Path::new("/usr/local/lib/node_modules/.bin/a4")),
            "npm uninstall -g @usearete/a4"
        );
        assert!(removal_command(Path::new("/opt/bin/a4")).contains("/opt/bin/a4"));
    }
}
