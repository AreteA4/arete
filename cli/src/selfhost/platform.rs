//! Platform keys and release asset names.
//!
//! The key is `<os>-<arch>` with os ∈ `darwin|linux|win32` and arch ∈
//! `arm64|x64`, identical to Node's `process.platform`/`process.arch` so the
//! npm bootstrapper and the Rust code agree. Asset name is `a4-<key>` plus
//! `.exe` on win32.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};

/// Platform key for the running binary, e.g. `darwin-arm64`.
pub fn platform_key() -> Result<&'static str> {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        "linux" => "linux",
        "windows" => "win32",
        other => return Err(anyhow!("Unsupported operating system: {other}")),
    };
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "x64",
        other => return Err(anyhow!("Unsupported architecture: {other}")),
    };
    Ok(match (os, arch) {
        ("darwin", "arm64") => "darwin-arm64",
        ("darwin", "x64") => "darwin-x64",
        ("linux", "x64") => "linux-x64",
        ("linux", "arm64") => "linux-arm64",
        ("win32", "x64") => "win32-x64",
        (os, arch) => return Err(anyhow!("No prebuilt a4 binary for {os}-{arch}")),
    })
}

/// Release asset name for a platform key (`a4-linux-x64`, `a4-win32-x64.exe`).
pub fn asset_name(platform_key: &str) -> String {
    if platform_key.starts_with("win32") {
        format!("a4-{platform_key}.exe")
    } else {
        format!("a4-{platform_key}")
    }
}

/// File name of the installed binary (`a4` or `a4.exe`).
pub fn binary_file_name() -> &'static str {
    if cfg!(windows) {
        "a4.exe"
    } else {
        "a4"
    }
}

/// Download base for one release: `https://github.com/AreteA4/arete/releases/download/a4-cli-v<ver>/`.
/// `A4_MANIFEST_BASE_URL` (a URL that already contains the version's directory,
/// or a template with `{version}`) overrides it for tests.
pub fn release_base_url(version: &str) -> String {
    match std::env::var("A4_MANIFEST_BASE_URL") {
        Ok(base) if !base.trim().is_empty() => {
            let base = base.trim_end_matches('/');
            if base.contains("{version}") {
                base.replace("{version}", version)
            } else {
                format!("{base}/a4-cli-v{version}")
            }
        }
        _ => format!("https://github.com/AreteA4/arete/releases/download/a4-cli-v{version}"),
    }
}

/// URL of the latest-version pointer (`A4_LATEST_URL` overrides for tests).
pub fn latest_url() -> String {
    std::env::var("A4_LATEST_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "https://docs.arete.run/a4/latest.json".to_string())
}

/// Default install directory: `$A4_INSTALL_DIR` → `$XDG_BIN_HOME` →
/// `~/.local/bin` (`%USERPROFILE%\.local\bin` on Windows).
pub fn default_install_dir() -> Result<PathBuf> {
    for key in ["A4_INSTALL_DIR", "XDG_BIN_HOME"] {
        if let Some(value) = std::env::var_os(key) {
            if !value.is_empty() {
                return Ok(PathBuf::from(value));
            }
        }
    }
    let home = dirs::home_dir().ok_or_else(|| anyhow!("Could not find home directory"))?;
    Ok(home.join(".local").join("bin"))
}

/// Split a PATH-like value into directories (empty entries dropped).
pub fn split_path(path_env: &OsStr) -> Vec<PathBuf> {
    std::env::split_paths(path_env)
        .filter(|entry| !entry.as_os_str().is_empty())
        .collect()
}

/// Whether `dir` appears in `path_env` (compared after normalising trailing
/// separators; symlinks are not resolved).
pub fn path_contains(path_env: &OsStr, dir: &Path) -> bool {
    let wanted = normalise(dir);
    split_path(path_env)
        .iter()
        .any(|entry| normalise(entry) == wanted)
}

/// Another `a4` that would shadow `install_dir/a4` on this PATH: the first
/// PATH entry *before* `install_dir` (or anywhere, when `install_dir` is not
/// on PATH) that contains an executable `a4`/`a4.exe` and is not
/// `install_dir` itself. Typical hits: `~/.cargo/bin/a4`, an npm global shim.
pub fn shadowing_binary(path_env: &OsStr, install_dir: &Path) -> Option<PathBuf> {
    let wanted = normalise(install_dir);
    for entry in split_path(path_env) {
        if normalise(&entry) == wanted {
            return None;
        }
        let candidate = entry.join(binary_file_name());
        if is_executable_file(&candidate) {
            return Some(candidate);
        }
    }
    None
}

fn normalise(path: &Path) -> PathBuf {
    let mut normalised = PathBuf::from(path);
    // Drop a single trailing separator ("/usr/bin/" == "/usr/bin").
    if let Some(stripped) = path.to_str().and_then(|value| {
        value
            .strip_suffix(std::path::MAIN_SEPARATOR)
            .filter(|rest| !rest.is_empty())
    }) {
        normalised = PathBuf::from(stripped);
    }
    normalised
}

#[cfg(unix)]
fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable_file(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|meta| meta.is_file())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn shadow_detection_stops_at_install_dir() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let cargo_bin = root.path().join("cargo-bin");
        let install_dir = root.path().join("local-bin");
        let later = root.path().join("later");
        for dir in [&cargo_bin, &install_dir, &later] {
            std::fs::create_dir_all(dir).unwrap();
            let bin = dir.join("a4");
            std::fs::write(&bin, "#!/bin/sh\n").unwrap();
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let joined =
            |dirs: &[&PathBuf]| std::env::join_paths(dirs.iter().map(|d| d.as_os_str())).unwrap();

        // Shadowed by cargo-bin, which comes first.
        let path = joined(&[&cargo_bin, &install_dir, &later]);
        assert_eq!(
            shadowing_binary(&path, &install_dir),
            Some(cargo_bin.join("a4"))
        );

        // install_dir first: nothing shadows it, even though `later` has an a4.
        let path = joined(&[&install_dir, &cargo_bin, &later]);
        assert_eq!(shadowing_binary(&path, &install_dir), None);

        // install_dir not on PATH at all: the first a4 anywhere wins.
        let path = joined(&[&later]);
        assert_eq!(
            shadowing_binary(&path, &install_dir),
            Some(later.join("a4"))
        );
        assert!(!path_contains(&path, &install_dir));
        assert!(path_contains(&joined(&[&install_dir]), &install_dir));
    }

    #[test]
    fn asset_names_follow_npm_layout() {
        assert_eq!(asset_name("linux-x64"), "a4-linux-x64");
        assert_eq!(asset_name("win32-x64"), "a4-win32-x64.exe");
    }

    #[test]
    fn platform_key_is_supported_on_ci_hosts() {
        let key = platform_key().unwrap();
        assert!([
            "darwin-arm64",
            "darwin-x64",
            "linux-x64",
            "linux-arm64",
            "win32-x64"
        ]
        .contains(&key));
    }
}
