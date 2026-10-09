//! The per-user Arete home directory (`~/.arete`).
//!
//! The home directory holds credentials, so every code path that creates it
//! must create it owner-only. Use [`ensure_arete_home`] (or
//! [`ensure_private_dir`]) instead of `fs::create_dir_all` for it.

use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

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

/// Create the Arete home directory if needed and make sure only the current
/// user can access it. Returns its path.
pub fn ensure_arete_home() -> Result<PathBuf> {
    let home = arete_home()?;
    ensure_private_dir(&home, true)?;
    Ok(home)
}

/// Ensure `path` is a directory only its owner can access (mode 700 on unix).
///
/// A missing directory is created with mode 700. An existing directory whose
/// mode grants group or other access is tightened to 700 when
/// `tighten_existing` is set, the current user owns it, and `path` itself is
/// not a symlink (so a shared directory the link points at is never changed
/// implicitly); otherwise this fails with the `chmod` command that fixes it.
pub fn ensure_private_dir(path: &Path, tighten_existing: bool) -> Result<()> {
    #[cfg(unix)]
    {
        unix::ensure_private_dir(path, tighten_existing)
    }

    #[cfg(not(unix))]
    {
        let _ = tighten_existing;
        fs::create_dir_all(path)
            .with_context(|| format!("Failed to create directory {}", path.display()))
    }
}

#[cfg(unix)]
mod unix {
    use super::*;
    use std::io::ErrorKind;
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};

    pub(super) fn ensure_private_dir(path: &Path, tighten_existing: bool) -> Result<()> {
        let created = create_private_dir(path)?;
        let metadata = fs::metadata(path)
            .with_context(|| format!("Failed to inspect directory {}", path.display()))?;
        if !metadata.is_dir() {
            anyhow::bail!("{} exists but is not a directory", path.display());
        }
        let mode = metadata.permissions().mode() & 0o777;
        if created && mode != 0o700 {
            // The process umask may have masked bits off the requested mode.
            set_owner_only(path)?;
            return Ok(());
        }
        if mode & 0o077 == 0 {
            return Ok(());
        }

        let owned = metadata.uid() == current_uid();
        // Never chmod through a symlink: the target may be a shared directory
        // this CLI did not create. Leave that decision to the user.
        let is_symlink = fs::symlink_metadata(path)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(true);
        if tighten_existing && owned && !is_symlink && set_owner_only(path).is_ok() {
            eprintln!(
                "note: restricted {} to mode 700 (was {mode:o}) because it holds credentials",
                path.display()
            );
            return Ok(());
        }

        anyhow::bail!(
            "Directory permissions are too broad ({mode:o}) for {}; it holds credentials and must be accessible only by you. Fix it with:\n  chmod 700 {}",
            path.display(),
            shell_quote(path)
        );
    }

    /// Create `path` (and missing parents) with mode 700 on the leaf.
    /// Returns whether this call created the leaf directory.
    fn create_private_dir(path: &Path) -> Result<bool> {
        if path.is_dir() {
            return Ok(false);
        }
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory {}", parent.display()))?;
        }
        match fs::DirBuilder::new().mode(0o700).create(path) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => Ok(false),
            Err(error) => {
                Err(error).with_context(|| format!("Failed to create directory {}", path.display()))
            }
        }
    }

    fn set_owner_only(path: &Path) -> Result<()> {
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .with_context(|| format!("Failed to set {} to mode 700", path.display()))
    }

    fn current_uid() -> u32 {
        // SAFETY: geteuid has no preconditions and cannot fail.
        unsafe { libc::geteuid() }
    }

    fn shell_quote(path: &Path) -> String {
        let text = path.display().to_string();
        let safe = text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'.' | b'_' | b'-' | b'~'));
        if safe && !text.is_empty() {
            text
        } else {
            format!("'{}'", text.replace('\'', r"'\''"))
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn mode(path: &Path) -> u32 {
            fs::metadata(path).unwrap().permissions().mode() & 0o777
        }

        #[test]
        fn creates_missing_directory_owner_only() {
            let root = tempfile::tempdir().unwrap();
            let dir = root.path().join("nested").join(".arete");

            ensure_private_dir(&dir, true).unwrap();

            assert_eq!(mode(&dir), 0o700);
        }

        #[test]
        fn tightens_existing_broad_directory_owned_by_user() {
            let root = tempfile::tempdir().unwrap();
            let dir = root.path().join(".arete");
            fs::create_dir(&dir).unwrap();
            fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();

            ensure_private_dir(&dir, true).unwrap();

            assert_eq!(mode(&dir), 0o700);
        }

        #[test]
        fn refuses_broad_directory_without_tightening_and_prints_chmod() {
            let root = tempfile::tempdir().unwrap();
            let dir = root.path().join("my creds");
            fs::create_dir(&dir).unwrap();
            fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();

            let error = ensure_private_dir(&dir, false).unwrap_err().to_string();

            assert!(error.contains("too broad (755)"), "{error}");
            assert!(
                error.contains(&format!("chmod 700 '{}'", dir.display())),
                "{error}"
            );
            assert_eq!(mode(&dir), 0o755);
        }

        #[test]
        fn does_not_tighten_through_symlink() {
            let root = tempfile::tempdir().unwrap();
            let shared = root.path().join("shared");
            fs::create_dir(&shared).unwrap();
            fs::set_permissions(&shared, fs::Permissions::from_mode(0o755)).unwrap();
            let link = root.path().join(".arete");
            std::os::unix::fs::symlink(&shared, &link).unwrap();

            let error = ensure_private_dir(&link, true).unwrap_err().to_string();

            assert!(error.contains("too broad (755)"), "{error}");
            assert_eq!(mode(&shared), 0o755);
        }

        #[test]
        fn accepts_symlink_to_private_directory() {
            let root = tempfile::tempdir().unwrap();
            let private = root.path().join("private");
            fs::create_dir(&private).unwrap();
            fs::set_permissions(&private, fs::Permissions::from_mode(0o700)).unwrap();
            let link = root.path().join(".arete");
            std::os::unix::fs::symlink(&private, &link).unwrap();

            ensure_private_dir(&link, true).unwrap();

            assert_eq!(mode(&private), 0o700);
        }

        #[test]
        fn leaves_private_directory_untouched() {
            let root = tempfile::tempdir().unwrap();
            let dir = root.path().join(".arete");
            fs::create_dir(&dir).unwrap();
            fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();

            ensure_private_dir(&dir, false).unwrap();

            assert_eq!(mode(&dir), 0o700);
        }

        #[test]
        fn shell_quote_only_quotes_when_needed() {
            assert_eq!(shell_quote(Path::new("/home/a/.arete")), "/home/a/.arete");
            assert_eq!(shell_quote(Path::new("/tmp/it's")), r"'/tmp/it'\''s'");
        }
    }
}
