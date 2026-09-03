//! PATH edits made by `a4 self install` and reverted by `a4 self uninstall`.
//!
//! Unix: append one marked line to `~/.profile` (created if missing) and,
//! depending on `$SHELL`, `~/.zshrc`, `~/.bashrc` + `~/.bash_profile`, or
//! `~/.config/fish/conf.d/a4.fish` (never created except `.profile`; the
//! fish file is created because conf.d is the documented place). Files that
//! already put the install dir on PATH are left alone, so the edit is
//! idempotent. Every appended line ends with [`MARKER`] so uninstall can
//! remove exactly what install added.
//!
//! Windows: the user PATH in `HKCU\Environment` is read and written through
//! `[Environment]::GetEnvironmentVariable/SetEnvironmentVariable(..., 'User')`
//! in PowerShell, which also broadcasts `WM_SETTINGCHANGE`; no registry crate
//! is needed.
//!
//! GitHub Actions: `$GITHUB_PATH` gets the install dir appended so later
//! steps of the same job see it.
//!
//! All edits are skipped by the caller when `--no-modify-path`,
//! `A4_NO_MODIFY_PATH=1` or `CI` is set.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Trailing comment on every line this module appends.
pub const MARKER: &str = "# added by a4 self install";

/// Which syntax an rc file uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RcKind {
    /// POSIX `sh`, bash, zsh.
    Posix,
    /// fish.
    Fish,
}

/// One rc file the installer may touch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RcTarget {
    pub path: PathBuf,
    pub kind: RcKind,
    /// Create the file when it does not exist (`~/.profile` and the fish
    /// `conf.d` drop-in only).
    pub create: bool,
}

/// Render `install_dir` for a shell line: `$HOME/...` when it lives under
/// `home`, otherwise the absolute path.
pub fn shell_dir(install_dir: &Path, home: &Path) -> String {
    match install_dir.strip_prefix(home) {
        Ok(rest) if !rest.as_os_str().is_empty() => {
            let rest = rest.to_string_lossy().replace('\\', "/");
            format!("$HOME/{rest}")
        }
        _ => install_dir.to_string_lossy().replace('\\', "/"),
    }
}

/// The POSIX line, without the marker: `export PATH="$HOME/.local/bin:$PATH"`.
pub fn posix_export_line(shell_dir: &str) -> String {
    format!("export PATH=\"{shell_dir}:$PATH\"")
}

/// The fish line, without the marker: `fish_add_path -g $HOME/.local/bin`.
pub fn fish_line(shell_dir: &str) -> String {
    format!("fish_add_path -g {shell_dir}")
}

/// The PowerShell line printed for agents on Windows.
pub fn powershell_line(install_dir: &Path, home: &Path) -> String {
    let dir = match install_dir.strip_prefix(home) {
        Ok(rest) if !rest.as_os_str().is_empty() => {
            format!("$HOME\\{}", rest.to_string_lossy().replace('/', "\\"))
        }
        _ => install_dir.to_string_lossy().to_string(),
    };
    format!("$env:Path = \"{dir};$env:Path\"")
}

fn rc_line(kind: RcKind, shell_dir: &str) -> String {
    match kind {
        RcKind::Posix => format!("{} {MARKER}", posix_export_line(shell_dir)),
        RcKind::Fish => format!("{} {MARKER}", fish_line(shell_dir)),
    }
}

/// rc files to edit for `home` and the basename of `$SHELL` (`None` when
/// unset). `~/.profile` is always included.
pub fn rc_targets(home: &Path, shell_basename: Option<&str>) -> Vec<RcTarget> {
    let mut targets = vec![RcTarget {
        path: home.join(".profile"),
        kind: RcKind::Posix,
        create: true,
    }];
    match shell_basename {
        Some("zsh") => targets.push(RcTarget {
            path: home.join(".zshrc"),
            kind: RcKind::Posix,
            create: false,
        }),
        Some("bash") => {
            targets.push(RcTarget {
                path: home.join(".bashrc"),
                kind: RcKind::Posix,
                create: false,
            });
            targets.push(RcTarget {
                path: home.join(".bash_profile"),
                kind: RcKind::Posix,
                create: false,
            });
        }
        Some("fish") => targets.push(RcTarget {
            path: home
                .join(".config")
                .join("fish")
                .join("conf.d")
                .join("a4.fish"),
            kind: RcKind::Fish,
            create: true,
        }),
        _ => {}
    }
    targets
}

/// Whether `content` already has an active (non-comment) line that puts a
/// directory whose path contains `dir_fragment` (e.g. `.local/bin`) on PATH.
pub fn already_on_path(content: &str, dir_fragment: &str) -> bool {
    content.lines().any(|line| {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            return false;
        }
        line.contains(dir_fragment) && (line.contains("PATH") || line.contains("fish_add_path"))
    })
}

/// Fragment used to recognise an existing PATH line for `install_dir`: the
/// path relative to `home` (`.local/bin`), or the full path otherwise.
fn dir_fragment(install_dir: &Path, home: &Path) -> String {
    match install_dir.strip_prefix(home) {
        Ok(rest) if !rest.as_os_str().is_empty() => rest.to_string_lossy().replace('\\', "/"),
        _ => install_dir.to_string_lossy().replace('\\', "/"),
    }
}

/// Append the PATH line to one rc file unless it already puts the dir on
/// PATH. Returns `Ok(true)` when the file was modified.
pub fn ensure_path_line(target: &RcTarget, install_dir: &Path, home: &Path) -> Result<bool> {
    let existing = match fs::read_to_string(&target.path) {
        Ok(content) => Some(content),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(error).with_context(|| format!("Failed to read {}", target.path.display()))
        }
    };
    let content = match existing {
        Some(content) => content,
        None if target.create => String::new(),
        None => return Ok(false),
    };
    if already_on_path(&content, &dir_fragment(install_dir, home)) {
        return Ok(false);
    }
    if let Some(parent) = target.path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }
    let line = rc_line(target.kind, &shell_dir(install_dir, home));
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&target.path)
        .with_context(|| format!("Failed to open {}", target.path.display()))?;
    let separator = if content.is_empty() || content.ends_with('\n') {
        ""
    } else {
        "\n"
    };
    writeln!(file, "{separator}{line}")
        .with_context(|| format!("Failed to write {}", target.path.display()))?;
    Ok(true)
}

/// Apply the Unix rc-file edits. Returns the files that were modified.
pub fn add_to_rc_files(
    install_dir: &Path,
    home: &Path,
    shell_basename: Option<&str>,
) -> Result<Vec<PathBuf>> {
    let mut modified = Vec::new();
    for target in rc_targets(home, shell_basename) {
        if ensure_path_line(&target, install_dir, home)? {
            modified.push(target.path);
        }
    }
    Ok(modified)
}

/// Remove every line carrying [`MARKER`] from an rc file. Returns `Ok(true)`
/// when the file changed. Missing files are ignored.
pub fn remove_marker_lines(path: &Path) -> Result<bool> {
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(error).with_context(|| format!("Failed to read {}", path.display()))
        }
    };
    if !content.contains(MARKER) {
        return Ok(false);
    }
    let kept: Vec<&str> = content
        .lines()
        .filter(|line| !line.trim_end().ends_with(MARKER))
        .collect();
    let mut rewritten = kept.join("\n");
    if !rewritten.is_empty() {
        rewritten.push('\n');
    }
    fs::write(path, rewritten).with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(true)
}

/// Revert the rc-file edits for every shell (not just the current one, since
/// `$SHELL` may have changed since install). Returns the files changed.
pub fn remove_from_rc_files(home: &Path) -> Result<Vec<PathBuf>> {
    let mut changed = Vec::new();
    let mut paths: Vec<PathBuf> = Vec::new();
    for shell in [Some("zsh"), Some("bash"), Some("fish")] {
        for target in rc_targets(home, shell) {
            if !paths.contains(&target.path) {
                paths.push(target.path);
            }
        }
    }
    for path in paths {
        if remove_marker_lines(&path)? {
            changed.push(path);
        }
    }
    Ok(changed)
}

/// Append `install_dir` to the file named by `$GITHUB_PATH`, if set. Returns
/// the file path when it was written.
pub fn append_github_path(install_dir: &Path) -> Result<Option<PathBuf>> {
    let Some(github_path) = std::env::var_os("GITHUB_PATH").filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };
    let github_path = PathBuf::from(github_path);
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&github_path)
        .with_context(|| format!("Failed to open {}", github_path.display()))?;
    writeln!(file, "{}", install_dir.display())
        .with_context(|| format!("Failed to write {}", github_path.display()))?;
    Ok(Some(github_path))
}

/// Windows user PATH (`HKCU\Environment\Path`) via PowerShell.
#[cfg(windows)]
pub mod windows {
    use std::path::Path;
    use std::process::Command;

    use anyhow::{anyhow, Context, Result};

    use crate::selfhost::platform::path_contains;

    fn powershell(script: &str) -> Result<String> {
        let output = Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", script])
            .output()
            .context("Failed to run powershell to edit the user PATH")?;
        if !output.status.success() {
            return Err(anyhow!(
                "powershell failed while editing the user PATH: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout)
            .trim_end()
            .to_string())
    }

    /// Current value of the user PATH.
    pub fn read_user_path() -> Result<String> {
        powershell("[Environment]::GetEnvironmentVariable('Path','User')")
    }

    fn set_user_path(value: &str) -> Result<()> {
        let escaped = value.replace('\'', "''");
        powershell(&format!(
            "[Environment]::SetEnvironmentVariable('Path','{escaped}','User')"
        ))?;
        Ok(())
    }

    /// Append `install_dir` to the user PATH unless present. Returns whether
    /// the registry value changed.
    pub fn add_to_user_path(install_dir: &Path) -> Result<bool> {
        let current = read_user_path()?;
        if path_contains(std::ffi::OsStr::new(&current), install_dir) {
            return Ok(false);
        }
        let dir = install_dir.to_string_lossy();
        let updated = if current.trim().is_empty() {
            dir.to_string()
        } else {
            format!("{};{dir}", current.trim_end_matches(';'))
        };
        set_user_path(&updated)?;
        Ok(true)
    }

    /// Remove `install_dir` from the user PATH. Returns whether it changed.
    pub fn remove_from_user_path(install_dir: &Path) -> Result<bool> {
        let current = read_user_path()?;
        if !path_contains(std::ffi::OsStr::new(&current), install_dir) {
            return Ok(false);
        }
        let wanted = install_dir
            .to_string_lossy()
            .trim_end_matches('\\')
            .to_ascii_lowercase();
        let kept: Vec<&str> = current
            .split(';')
            .filter(|entry| {
                !entry.trim().is_empty()
                    && entry.trim().trim_end_matches('\\').to_ascii_lowercase() != wanted
            })
            .collect();
        set_user_path(&kept.join(";"))?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn shell_dir_uses_home_variable() {
        let home = Path::new("/Users/x");
        assert_eq!(
            shell_dir(&home.join(".local/bin"), home),
            "$HOME/.local/bin"
        );
        assert_eq!(shell_dir(Path::new("/opt/a4/bin"), home), "/opt/a4/bin");
        assert_eq!(
            posix_export_line("$HOME/.local/bin"),
            "export PATH=\"$HOME/.local/bin:$PATH\""
        );
        assert_eq!(
            fish_line("$HOME/.local/bin"),
            "fish_add_path -g $HOME/.local/bin"
        );
    }

    #[test]
    fn profile_is_created_and_edit_is_idempotent() {
        let home = home();
        let install_dir = home.path().join(".local").join("bin");
        let zshrc = home.path().join(".zshrc");
        fs::write(&zshrc, "alias ll='ls -l'").unwrap(); // no trailing newline

        let first = add_to_rc_files(&install_dir, home.path(), Some("zsh")).unwrap();
        assert_eq!(first, vec![home.path().join(".profile"), zshrc.clone()]);
        let profile = fs::read_to_string(home.path().join(".profile")).unwrap();
        assert_eq!(
            profile,
            "export PATH=\"$HOME/.local/bin:$PATH\" # added by a4 self install\n"
        );
        let zshrc_after = fs::read_to_string(&zshrc).unwrap();
        assert_eq!(
            zshrc_after,
            "alias ll='ls -l'\nexport PATH=\"$HOME/.local/bin:$PATH\" # added by a4 self install\n"
        );

        let second = add_to_rc_files(&install_dir, home.path(), Some("zsh")).unwrap();
        assert!(second.is_empty());
        assert_eq!(fs::read_to_string(&zshrc).unwrap(), zshrc_after);
        assert_eq!(
            fs::read_to_string(home.path().join(".profile")).unwrap(),
            profile
        );
        // .bashrc was never created.
        assert!(!home.path().join(".bashrc").exists());
    }

    #[test]
    fn existing_local_bin_line_means_no_append() {
        let home = home();
        let install_dir = home.path().join(".local").join("bin");
        let bashrc = home.path().join(".bashrc");
        let original = "# comment mentioning .local/bin PATH\nPATH=\"$HOME/.local/bin:$PATH\"\n";
        fs::write(&bashrc, original).unwrap();
        let modified = add_to_rc_files(&install_dir, home.path(), Some("bash")).unwrap();
        assert_eq!(modified, vec![home.path().join(".profile")]);
        assert_eq!(fs::read_to_string(&bashrc).unwrap(), original);
        // A commented-out line does not count.
        fs::write(&bashrc, "# export PATH=\"$HOME/.local/bin:$PATH\"\n").unwrap();
        let modified = add_to_rc_files(&install_dir, home.path(), Some("bash")).unwrap();
        assert_eq!(modified, vec![bashrc.clone()]);
        assert!(!home.path().join(".bash_profile").exists());
    }

    #[test]
    fn fish_drop_in_is_created_with_fish_add_path() {
        let home = home();
        let install_dir = home.path().join(".local").join("bin");
        let modified = add_to_rc_files(&install_dir, home.path(), Some("fish")).unwrap();
        let fish = home.path().join(".config/fish/conf.d/a4.fish");
        assert!(modified.contains(&fish));
        assert_eq!(
            fs::read_to_string(&fish).unwrap(),
            "fish_add_path -g $HOME/.local/bin # added by a4 self install\n"
        );
        assert!(add_to_rc_files(&install_dir, home.path(), Some("fish"))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn uninstall_removes_only_marked_lines() {
        let home = home();
        let install_dir = home.path().join(".local").join("bin");
        let zshrc = home.path().join(".zshrc");
        fs::write(
            &zshrc,
            "alias ll='ls -l'\nexport PATH=\"$HOME/bin:$PATH\"\n",
        )
        .unwrap();
        add_to_rc_files(&install_dir, home.path(), Some("zsh")).unwrap();
        let changed = remove_from_rc_files(home.path()).unwrap();
        assert!(changed.contains(&zshrc));
        assert!(changed.contains(&home.path().join(".profile")));
        assert_eq!(
            fs::read_to_string(&zshrc).unwrap(),
            "alias ll='ls -l'\nexport PATH=\"$HOME/bin:$PATH\"\n"
        );
        assert_eq!(
            fs::read_to_string(home.path().join(".profile")).unwrap(),
            ""
        );
        assert!(remove_from_rc_files(home.path()).unwrap().is_empty());
    }

    #[test]
    fn custom_install_dir_is_written_verbatim() {
        let home = home();
        let install_dir = Path::new("/opt/a4/bin");
        add_to_rc_files(install_dir, home.path(), None).unwrap();
        assert_eq!(
            fs::read_to_string(home.path().join(".profile")).unwrap(),
            "export PATH=\"/opt/a4/bin:$PATH\" # added by a4 self install\n"
        );
    }
}
