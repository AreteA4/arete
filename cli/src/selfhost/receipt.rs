//! Install receipt: `~/.arete/receipt.json`.
//!
//! Written by `a4 self install`, updated by `a4 self update`, read by
//! `a4 doctor`, `a4 init` (absolute MCP command path) and the npm
//! bootstrapper (`packages/arete/bin/a4.js`). The [`Receipt`] struct defines
//! the public field contract; serialized field names are camelCase.

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
    ///
    /// `a4 self update` sets this to `self-update`. The original installer
    /// channel is [`Self::install_source`].
    pub source: String,
    /// Original installer channel (`sh`, `ps1`, `npm`, `manual`, `cargo`).
    /// Preserved when `source` becomes `self-update`. Absent on receipts
    /// written before this field existed; those still expose the channel
    /// through [`Self::install_channel`] while `source` is one of the
    /// installer values.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_source: Option<String>,
    /// Whether checksum + minisign signature were verified at install time.
    pub verified: bool,
    /// Whether the installer edited PATH (rc files / registry).
    pub modify_path: bool,
    /// RFC 3339 UTC timestamp.
    pub installed_at: String,
}

pub use crate::arete_home::arete_home;

fn is_install_channel(source: &str) -> bool {
    matches!(source, "sh" | "ps1" | "npm" | "manual" | "cargo")
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
        crate::arete_home::ensure_private_dir(
            path.parent()
                .ok_or_else(|| anyhow::anyhow!("Receipt path must have a parent directory"))?,
            true,
        )?;
        let content = serde_json::to_string_pretty(self)?;
        fs::write(&path, format!("{content}\n"))
            .with_context(|| format!("Failed to write install receipt {}", path.display()))?;
        Ok(path)
    }

    /// Installer channel to report after updates. `self-update` is not a channel.
    pub fn install_channel(&self) -> Option<&str> {
        if let Some(channel) = self
            .install_source
            .as_deref()
            .filter(|value| is_install_channel(value))
        {
            return Some(channel);
        }
        if is_install_channel(&self.source) {
            Some(self.source.as_str())
        } else {
            None
        }
    }

    /// Receipt written by `a4 self update`. Keeps the original install channel.
    pub fn after_self_update(&self, version: String, installed_at: String) -> Self {
        let install_source = self.install_channel().map(str::to_string);
        Self {
            version,
            source: "self-update".to_string(),
            install_source,
            verified: true,
            installed_at,
            ..self.clone()
        }
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
            install_source: None,
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
        assert!(!json.contains("installSource"));
    }

    #[test]
    fn self_update_keeps_the_original_install_channel() {
        let installed = Receipt {
            schema_version: RECEIPT_SCHEMA_VERSION,
            version: "0.13.0".into(),
            binary: PathBuf::from("/usr/local/bin/a4"),
            install_dir: PathBuf::from("/usr/local/bin"),
            platform: "linux-x64".into(),
            source: "npm".into(),
            install_source: None,
            verified: true,
            modify_path: false,
            installed_at: "2026-09-10T12:00:00Z".into(),
        };
        assert_eq!(installed.install_channel(), Some("npm"));

        let updated = installed.after_self_update("0.14.0".into(), "2026-10-01T00:00:00Z".into());
        assert_eq!(updated.source, "self-update");
        assert_eq!(updated.install_source.as_deref(), Some("npm"));
        assert_eq!(updated.install_channel(), Some("npm"));
        assert_eq!(updated.version, "0.14.0");
        assert_eq!(updated.binary, installed.binary);

        let again = updated.after_self_update("0.15.0".into(), "2026-10-02T00:00:00Z".into());
        assert_eq!(again.install_channel(), Some("npm"));
    }

    #[test]
    fn old_self_update_receipt_without_a_channel_parses() {
        let parsed: Receipt = serde_json::from_str(
            r#"{"schemaVersion":1,"version":"0.13.0","binary":"/bin/a4","installDir":"/bin","platform":"linux-x64","source":"self-update","verified":true,"modifyPath":false,"installedAt":"2026-09-10T12:00:00Z"}"#,
        )
        .unwrap();
        assert!(parsed.install_source.is_none());
        assert_eq!(parsed.install_channel(), None);
    }
}
