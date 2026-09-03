//! `a4 self uninstall`: remove the installed binary, the receipt and the PATH
//! lines `a4 self install` added. Credentials (`~/.arete/credentials.toml`),
//! telemetry settings and cached templates stay; the command lists what it
//! left behind so a human can finish the job deliberately.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use colored::Colorize;
use serde_json::json;

use super::path_edit;
use super::platform::{binary_file_name, default_install_dir};
use super::receipt::{arete_home, receipt_path, Receipt};
use super::{home_dir, same_file, UninstallArgs};
use crate::ui::symbols;

pub fn run(_args: UninstallArgs, json: bool) -> Result<()> {
    let receipt = Receipt::load()?;
    let (binary, install_dir) = match &receipt {
        Some(receipt) => (receipt.binary.clone(), receipt.install_dir.clone()),
        None => {
            let dir = default_install_dir()?;
            (dir.join(binary_file_name()), dir)
        }
    };

    let mut removed: Vec<PathBuf> = Vec::new();
    let mut path_restored: Vec<PathBuf> = Vec::new();

    if binary.exists() {
        remove_binary(&binary)?;
        removed.push(binary.clone());
    }

    let receipt_file = receipt_path()?;
    if receipt_file.exists() {
        Receipt::delete()?;
        removed.push(receipt_file);
    }

    let home = home_dir()?;
    path_restored.extend(restore_system_path(&home, &install_dir)?);

    let arete = arete_home()?;
    let mut left_behind: Vec<PathBuf> = Vec::new();
    for name in [
        "credentials.toml",
        "templates",
        "telemetry.toml",
        "update-check.json",
    ] {
        let path = arete.join(name);
        if path.exists() {
            left_behind.push(path);
        }
    }
    if arete.exists() && left_behind.is_empty() {
        left_behind.push(arete.clone());
    }

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "schemaVersion": 1,
                "removed": removed,
                "pathRestored": path_restored,
                "leftBehind": left_behind,
            }))?
        );
    }
    if removed.is_empty() {
        eprintln!(
            "{} Nothing to remove: no a4 at {} and no receipt",
            symbols::WARNING.yellow().bold(),
            binary.display()
        );
    } else {
        for path in &removed {
            eprintln!(
                "{} Removed {}",
                symbols::SUCCESS.green().bold(),
                display_home(path, &home)
            );
        }
    }
    for path in &path_restored {
        eprintln!(
            "{} Removed PATH line from {}",
            symbols::SUCCESS.green().bold(),
            display_home(path, &home)
        );
    }
    if !left_behind.is_empty() {
        eprintln!(
            "  {}",
            "Left behind (delete manually if unwanted):".dimmed()
        );
        for path in &left_behind {
            eprintln!("    {}", display_home(path, &home).dimmed());
        }
    }
    Ok(())
}

/// Undo the rc-file (Unix) or user PATH (Windows) edits made by install.
fn restore_system_path(home: &Path, install_dir: &Path) -> Result<Vec<PathBuf>> {
    #[cfg(unix)]
    {
        let _ = install_dir;
        path_edit::remove_from_rc_files(home)
    }
    #[cfg(windows)]
    {
        let _ = home;
        if path_edit::windows::remove_from_user_path(install_dir)? {
            Ok(vec![PathBuf::from("HKCU\\Environment\\Path")])
        } else {
            Ok(Vec::new())
        }
    }
}

/// Delete the binary, even when it is the running executable.
fn remove_binary(binary: &Path) -> Result<()> {
    let running = std::env::current_exe().ok();
    let is_self = running.as_deref().is_some_and(|exe| same_file(exe, binary));
    if is_self && cfg!(windows) {
        self_replace::self_delete_at(binary)
            .with_context(|| format!("Failed to schedule removal of {}", binary.display()))?;
        return Ok(());
    }
    fs::remove_file(binary).with_context(|| format!("Failed to remove {}", binary.display()))
}

fn display_home(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}
