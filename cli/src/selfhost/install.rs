//! `a4 self install`: the downloaded binary finishes its own installation.
//!
//! Bootstrappers (`install.sh`, `install.ps1`, `npx @usearete/a4`) only
//! download and hand over; everything after that lives here so all three
//! behave identically: optional signature + checksum verification of the
//! running binary, copy into the install dir, receipt, PATH edits, shadow
//! warning, and the two machine-readable stdout lines agents parse:
//!
//! ```text
//! A4_BIN=/Users/x/.local/bin/a4
//! export PATH="$HOME/.local/bin:$PATH"
//! ```
//!
//! Spec: `docs/internal/agent-first-onboarding.md` WP2, steps 1–7.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use colored::Colorize;
use serde_json::{json, Value};

use super::latest::is_newer;
use super::path_edit;
use super::platform::{
    asset_name, binary_file_name, default_install_dir, platform_key, shadowing_binary,
};
use super::receipt::{Receipt, RECEIPT_SCHEMA_VERSION};
use super::{
    absolute, current_version, home_dir, install_binary, now_rfc3339, path_edits_disabled,
    print_final_lines, removal_command, same_file, shell_basename, InstallArgs,
};
use super::{keys, verify};
use crate::ui::symbols;

const SOURCES: &[&str] = &["sh", "ps1", "npm", "manual"];

pub fn run(args: InstallArgs, json: bool) -> Result<()> {
    if !SOURCES.contains(&args.source.as_str()) {
        bail!(
            "Unknown --source {:?}. Pass one of: {}",
            args.source,
            SOURCES.join(", ")
        );
    }
    let src = std::env::current_exe().context("Failed to locate the running a4 binary")?;
    let src = fs::canonicalize(&src).unwrap_or(src);
    let platform = platform_key()?;
    let asset = asset_name(platform);

    // 1. Verify (only when the bootstrapper handed us the release files).
    let verified = match (&args.checksums, &args.signature) {
        (Some(checksums), Some(signature)) => {
            verify::verify_release_asset(
                checksums,
                signature,
                &src,
                &asset,
                keys::MINISIGN_PUBLIC_KEY,
            )?;
            log(&format!(
                "{} Verified minisign signature and sha256 for {asset}",
                symbols::SUCCESS.green().bold()
            ));
            true
        }
        _ => false,
    };

    // 2. Install dir.
    let install_dir = match args.install_dir {
        Some(dir) => absolute(&dir)?,
        None => absolute(&default_install_dir()?)?,
    };
    fs::create_dir_all(&install_dir)
        .with_context(|| format!("Failed to create {}", install_dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&install_dir, fs::Permissions::from_mode(0o755));
    }
    let target = install_dir.join(binary_file_name());

    // 3. Copy (skip when re-running from the installed binary itself).
    let previous = Receipt::load().unwrap_or(None);
    let status = if target.exists() && same_file(&src, &target) {
        "unchanged"
    } else {
        if target.exists() && !args.force {
            if let Some(previous) = previous.as_ref().filter(|r| same_path(&r.binary, &target)) {
                if is_newer(&previous.version, current_version()) == Some(true) {
                    bail!(
                        "a4 {} at {} is newer than this binary ({}). Pass --force to overwrite it.",
                        previous.version,
                        target.display(),
                        current_version()
                    );
                }
            }
        }
        install_binary(&src, &target)?;
        "installed"
    };

    // 4. Receipt.
    let modify_path = !(args.no_modify_path || path_edits_disabled());
    let receipt = Receipt {
        schema_version: RECEIPT_SCHEMA_VERSION,
        version: current_version().to_string(),
        binary: target.clone(),
        install_dir: install_dir.clone(),
        platform: platform.to_string(),
        source: args.source.clone(),
        verified,
        modify_path,
        installed_at: now_rfc3339(),
    };
    let receipt_path = receipt.save()?;

    // 5. PATH.
    let home = home_dir()?;
    let mut path_modified: Vec<PathBuf> = Vec::new();
    let env_disabled = std::env::var_os("A4_NO_MODIFY_PATH").is_some_and(|v| !v.is_empty());
    if !args.no_modify_path && !env_disabled {
        if modify_path {
            path_modified.extend(modify_system_path(&install_dir, &home)?);
        }
        // `$GITHUB_PATH` is the CI-specific PATH mechanism: it is honoured even
        // though rc-file edits are skipped under `CI`.
        if let Some(github_path) = path_edit::append_github_path(&install_dir)? {
            path_modified.push(github_path);
        }
    }

    // 6. Shadowing.
    let path_env = std::env::var_os("PATH").unwrap_or_default();
    let shadowed_by =
        shadowing_binary(&path_env, &install_dir).filter(|other| !same_file(other, &target));

    // 7. Output.
    if json {
        let mut object = match serde_json::to_value(&receipt)? {
            Value::Object(map) => map,
            _ => unreachable!("receipt serialises to an object"),
        };
        object.insert("status".into(), json!(status));
        object.insert("receipt".into(), json!(receipt_path));
        object.insert("pathModified".into(), json!(path_modified));
        object.insert("shadowedBy".into(), json!(shadowed_by));
        println!("{}", serde_json::to_string_pretty(&Value::Object(object))?);
    }
    match status {
        "installed" => log(&format!(
            "{} Installed a4 {} to {}",
            symbols::SUCCESS.green().bold(),
            receipt.version,
            target.display()
        )),
        _ => log(&format!(
            "{} a4 {} is already installed at {}",
            symbols::SUCCESS.green().bold(),
            receipt.version,
            target.display()
        )),
    }
    if !verified {
        log(&format!(
            "  {}",
            "Not verified: no --checksums/--signature given (recorded verified: false)".dimmed()
        ));
    }
    if !modify_path {
        log(&format!(
            "  {}",
            "PATH edits skipped (--no-modify-path, A4_NO_MODIFY_PATH or CI)".dimmed()
        ));
    } else if path_modified.is_empty() {
        log(&format!(
            "  {}",
            "PATH already configured; no rc files changed".dimmed()
        ));
    } else {
        let files: Vec<String> = path_modified
            .iter()
            .map(|p| display_home(p, &home))
            .collect();
        log(&format!(
            "  {}",
            format!("PATH: added to {}", files.join(", ")).dimmed()
        ));
    }
    if let Some(other) = &shadowed_by {
        log(&format!(
            "{} Another a4 at {} comes first on PATH and will shadow {}. Remove it with: {}",
            symbols::WARNING.yellow().bold(),
            other.display(),
            target.display(),
            removal_command(other)
        ));
    }
    log(&format!(
        "  {}",
        "Open a new shell, or run the export line below, to use a4".dimmed()
    ));
    print_final_lines(&target, &install_dir, &home);
    Ok(())
}

/// rc files (Unix) or the user PATH (Windows). Returns what was modified.
fn modify_system_path(install_dir: &Path, home: &Path) -> Result<Vec<PathBuf>> {
    #[cfg(unix)]
    {
        path_edit::add_to_rc_files(install_dir, home, shell_basename().as_deref())
    }
    #[cfg(windows)]
    {
        let _ = (home, shell_basename());
        if path_edit::windows::add_to_user_path(install_dir)? {
            Ok(vec![PathBuf::from("HKCU\\Environment\\Path")])
        } else {
            Ok(Vec::new())
        }
    }
}

fn same_path(a: &Path, b: &Path) -> bool {
    a == b || same_file(a, b)
}

fn display_home(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

/// Human output goes to stderr; stdout is reserved for JSON and the final
/// `A4_BIN=` / `export PATH=` lines.
fn log(message: &str) {
    eprintln!("{message}");
}
