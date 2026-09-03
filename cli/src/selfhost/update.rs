//! `a4 self update [VERSION] [--check] [--dry-run]` and its alias `a4 upgrade`.
//!
//! Only binaries installed by the Arete installer (i.e. with a receipt) can
//! self-update; `cargo install` users are told to use cargo. The target is
//! the explicit `VERSION` or the docs-site `latest.json` pointer. Downloads
//! go to `~/.arete/downloads/<version>/`; the signature over `checksums.txt`
//! and the asset's SHA-256 are mandatory here (the running binary is the
//! trust root, and it carries the release public key). The swap uses the
//! `self-replace` crate so it works while this very binary is running, on
//! Windows too.
//!
//! Spec: `docs/internal/agent-first-onboarding.md` WP4, steps 1–4.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use colored::Colorize;
use serde_json::json;

use super::latest::{fetch_latest, is_newer};
use super::manifest::{download_client, download_release_file, fetch_manifest};
use super::platform::{asset_name, platform_key};
use super::receipt::{arete_home, Receipt};
use super::{current_version, install_binary, keys, now_rfc3339, same_file, verify, UpdateArgs};
use crate::ui::{symbols, ExitCode};

/// Exit code for `--check` when a newer release exists.
pub const UPDATE_AVAILABLE_EXIT_CODE: i32 = 10;

const LATEST_TIMEOUT: Duration = Duration::from_secs(10);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(300);

pub fn run(args: UpdateArgs, json: bool) -> Result<()> {
    // 1. Receipt.
    let receipt = match Receipt::load()? {
        Some(receipt) => receipt,
        None => bail!("{}", not_installed_message()),
    };
    let current = current_version();

    // 2. Target version.
    let explicit = args.version.is_some();
    let target = match args.version.as_deref() {
        Some(version) => {
            let version = version.trim().trim_start_matches('v').to_string();
            semver::Version::parse(&version).with_context(|| {
                format!("{version:?} is not a semver version (example: 0.13.0)")
            })?;
            version
        }
        None => {
            fetch_latest(LATEST_TIMEOUT)
                .context("Could not determine the latest a4 version")?
                .version
        }
    };
    let update_available = if explicit {
        target != current
    } else {
        is_newer(&target, current).unwrap_or(false)
    };

    // 3. --check.
    if args.check {
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schemaVersion": 1,
                    "current": current,
                    "latest": target,
                    "updateAvailable": update_available,
                }))?
            );
        } else if update_available {
            eprintln!("a4 {target} is available (you have {current}). Run: a4 self update");
        } else {
            eprintln!("a4 {current} is up to date");
        }
        if update_available {
            return Err(ExitCode(UPDATE_AVAILABLE_EXIT_CODE).into());
        }
        return Ok(());
    }

    if !update_available {
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schemaVersion": 1,
                    "status": "unchanged",
                    "previous": current,
                    "version": current,
                    "binary": receipt.binary,
                    "dryRun": args.dry_run,
                }))?
            );
        }
        eprintln!(
            "{} a4 {current} is up to date",
            symbols::SUCCESS.green().bold()
        );
        return Ok(());
    }

    // 4. Download + verify.
    let platform = platform_key()?;
    let asset = asset_name(platform);
    let download_dir = arete_home()?.join("downloads").join(&target);
    fs::create_dir_all(&download_dir)
        .with_context(|| format!("Failed to create {}", download_dir.display()))?;
    let outcome = download_and_verify(&target, &asset, &download_dir);
    let asset_path = match outcome {
        Ok(path) => path,
        Err(error) => {
            let _ = fs::remove_dir_all(&download_dir);
            return Err(error);
        }
    };

    if args.dry_run {
        let _ = fs::remove_dir_all(&download_dir);
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schemaVersion": 1,
                    "status": "dry-run",
                    "previous": current,
                    "version": target,
                    "binary": receipt.binary,
                    "asset": asset,
                    "dryRun": true,
                }))?
            );
        }
        eprintln!(
            "{} Dry run: would replace {} (a4 {current}) with a4 {target} ({asset}); signature and sha256 verified",
            symbols::SUCCESS.green().bold(),
            receipt.binary.display()
        );
        return Ok(());
    }

    // Replace, update the receipt, clean up.
    replace_binary(&receipt.binary, &asset_path)?;
    let _ = fs::remove_dir_all(&download_dir);
    let updated = Receipt {
        version: target.clone(),
        source: "self-update".to_string(),
        verified: true,
        installed_at: now_rfc3339(),
        ..receipt.clone()
    };
    updated.save()?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "schemaVersion": 1,
                "status": "updated",
                "previous": current,
                "version": target,
                "binary": receipt.binary,
                "dryRun": false,
            }))?
        );
    }
    eprintln!(
        "{} Updated a4 {current} -> {target} at {}",
        symbols::SUCCESS.green().bold(),
        receipt.binary.display()
    );
    Ok(())
}

/// Exact error for binaries without a receipt; adds the cargo hint only when
/// the running binary lives under `~/.cargo/bin`.
pub fn not_installed_message() -> String {
    let mut message = "a4 was not installed by the Arete installer. Reinstall with: curl -fsSL https://arete.run/install.sh | sh".to_string();
    if running_from_cargo_bin() {
        message.push_str(" (this binary is under ~/.cargo/bin; to update it instead run: cargo install a4-cli --force)");
    }
    message
}

fn running_from_cargo_bin() -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let cargo_bin = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".cargo")))
        .map(|cargo| cargo.join("bin"));
    match cargo_bin {
        Some(cargo_bin) => {
            exe.starts_with(&cargo_bin) || exe.to_string_lossy().contains("/.cargo/bin/")
        }
        None => false,
    }
}

/// Download manifest, asset, checksums and signature for `version` into
/// `dir`, verify, and return the asset path.
fn download_and_verify(version: &str, asset: &str, dir: &Path) -> Result<PathBuf> {
    let client = download_client(DOWNLOAD_TIMEOUT)?;
    eprintln!(
        "{} Downloading a4 {version} ({asset})",
        symbols::ARROW.blue().bold()
    );
    let manifest = fetch_manifest(&client, version)?;
    let asset_path = dir.join(asset);
    let checksums_path = dir.join(&manifest.checksums);
    let signature_path = dir.join(&manifest.signature);
    download_release_file(&client, version, asset, &asset_path)?;
    download_release_file(&client, version, &manifest.checksums, &checksums_path)?;
    download_release_file(&client, version, &manifest.signature, &signature_path)?;
    verify::verify_release_asset(
        &checksums_path,
        &signature_path,
        &asset_path,
        asset,
        keys::MINISIGN_PUBLIC_KEY,
    )?;
    eprintln!(
        "{} Verified minisign signature and sha256 for {asset}",
        symbols::SUCCESS.green().bold()
    );
    Ok(asset_path)
}

/// Put `new_binary` at `target`. When `target` is the running executable the
/// `self-replace` crate swaps it safely (rename-aside on Windows); otherwise
/// a plain tmp-copy + atomic rename is enough.
fn replace_binary(target: &Path, new_binary: &Path) -> Result<()> {
    let running = std::env::current_exe().ok();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(new_binary, fs::Permissions::from_mode(0o755))
            .with_context(|| format!("Failed to chmod {}", new_binary.display()))?;
    }
    if running.as_deref().is_some_and(|exe| same_file(exe, target)) {
        self_replace::self_replace(new_binary)
            .with_context(|| format!("Failed to replace {}", target.display()))?;
        return Ok(());
    }
    install_binary(new_binary, target)
}
