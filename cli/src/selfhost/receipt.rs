//! Install receipt: `~/.arete/receipt.json`.
//!
//! Written by `a4 self install`, updated by `a4 self update`, read by
//! `a4 doctor`, `a4 init` (absolute MCP command path) and the npm
//! bootstrapper (`packages/arete/bin/a4.js`). Field names are camelCase and
//! part of the public contract in `docs/internal/agent-first-onboarding.md`.

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub const RECEIPT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Receipt {
    pub schema_version: u32,
    /// CLI version that was installed (`CARGO_PKG_VERSION` at install time).
    pub version: String,
    /// Absolute path of the installed binary.
    pub binary: PathBuf,
    /// Directory containing `binary`.
    pub install_dir: PathBuf,
    /// Platform key, e.g. `darwin-arm64`.
    pub platform: String,
    /// `sh | ps1 | npm | manual | self-update`.
    pub source: String,
    /// Whether checksum + minisign signature were verified at install time.
    pub verified: bool,
    /// Whether the installer edited PATH (rc files / registry).
    pub modify_path: bool,
    /// RFC 3339 UTC timestamp.
    pub installed_at: String,
}

/// `~/.arete` (or `$ARETE_HOME` when set; test hook, undocumented).
pub fn arete_home() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("ARETE_HOME") {
        if !path.is_empty() {
            return Ok(PathBuf::from(path));
        }
    }
    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Could not find home directory"))?;
    Ok(home.join(".arete"))
}

/// Path of the receipt file.
pub fn receipt_path() -> Result<PathBuf> {
    Ok(arete_home()?.join("receipt.json"))
}

impl Receipt {
    /// Load the receipt; `Ok(None)` when it does not exist.
    pub fn load() -> Result<Option<Receipt>> {
        let path = receipt_path()?;
        match fs::read_to_string(&path) {
            Ok(content) => {
                let receipt: Receipt = serde_json::from_str(&content).with_context(|| {
                    format!("Failed to parse install receipt {}", path.display())
                })?;
                Ok(Some(receipt))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error).with_context(|| format!("Failed to read {}", path.display())),
        }
    }

    /// Write the receipt (creates `~/.arete`).
    pub fn save(&self) -> Result<PathBuf> {
        let path = receipt_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create {}", parent.display()))?;
        }
        let content = serde_json::to_string_pretty(self)?;
        fs::write(&path, format!("{content}\n"))
            .with_context(|| format!("Failed to write install receipt {}", path.display()))?;
        Ok(path)
    }

    /// Remove the receipt if present.
    pub fn delete() -> Result<()> {
        let path = receipt_path()?;
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => {
                Err(error).with_context(|| format!("Failed to remove {}", path.display()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receipt_round_trips_with_camel_case_keys() {
        let receipt = Receipt {
            schema_version: RECEIPT_SCHEMA_VERSION,
            version: "0.13.0".into(),
            binary: PathBuf::from("/Users/x/.local/bin/a4"),
            install_dir: PathBuf::from("/Users/x/.local/bin"),
            platform: "darwin-arm64".into(),
            source: "sh".into(),
            verified: true,
            modify_path: true,
            installed_at: "2026-09-10T12:00:00Z".into(),
        };
        let json = serde_json::to_string(&receipt).unwrap();
        assert!(json.contains("\"schemaVersion\":1"));
        assert!(json.contains("\"installDir\""));
        assert!(json.contains("\"modifyPath\""));
        assert!(json.contains("\"installedAt\""));
        let parsed: Receipt = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, receipt);
    }
}
