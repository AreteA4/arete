use std::path::{Component, Path, PathBuf};

use anyhow::{bail, Context, Result};

#[derive(Debug, Clone)]
pub struct ProjectPaths {
    root: PathBuf,
    manifest_allows_outside: bool,
    cli_allows_outside: bool,
}

impl ProjectPaths {
    pub fn new(
        root: impl AsRef<Path>,
        manifest_allows_outside: bool,
        cli_allows_outside: bool,
    ) -> Result<Self> {
        let root = std::fs::canonicalize(root.as_ref()).with_context(|| {
            format!("Failed to resolve project root {}", root.as_ref().display())
        })?;
        Ok(Self {
            root,
            manifest_allows_outside,
            cli_allows_outside,
        })
    }

    pub fn input(&self, relative: &str, kind: &str) -> Result<PathBuf> {
        let joined = self.join_manifest_relative(relative, kind)?;
        let canonical = std::fs::canonicalize(&joined)
            .with_context(|| format!("Failed to resolve {kind} {}", joined.display()))?;
        if !canonical.starts_with(&self.root) {
            bail!("{kind} {} escapes the project root", joined.display());
        }
        Ok(canonical)
    }

    pub fn input_directory(&self, relative: &str, kind: &str) -> Result<PathBuf> {
        let canonical = self.input(relative, kind)?;
        if !canonical.is_dir() {
            bail!("{kind} {} is not a directory", canonical.display());
        }
        Ok(canonical)
    }

    pub fn output(&self, configured: &str, description: &str) -> Result<PathBuf> {
        if configured.trim().is_empty() {
            bail!("{description} cannot be empty");
        }
        let supplied = Path::new(configured);
        let absolute = if supplied.is_absolute() {
            normalize_absolute(supplied)?
        } else {
            normalize_absolute(&self.root.join(supplied))?
        };
        if !absolute.starts_with(&self.root)
            && !(self.manifest_allows_outside && self.cli_allows_outside)
        {
            if self.manifest_allows_outside {
                bail!(
                    "{description} {} is outside the project; repeat with --allow-outside-project",
                    absolute.display()
                );
            }
            bail!(
                "{description} {} is outside the project; set install.allow_outside_project = true and repeat with --allow-outside-project",
                absolute.display()
            );
        }
        Ok(absolute)
    }

    fn join_manifest_relative(&self, relative: &str, kind: &str) -> Result<PathBuf> {
        let supplied = Path::new(relative);
        if relative.trim().is_empty() || supplied.is_absolute() {
            bail!("{kind} must be a non-empty manifest-relative path");
        }
        if supplied
            .components()
            .any(|component| matches!(component, Component::ParentDir))
        {
            bail!("{kind} cannot contain parent traversal: {relative}");
        }
        normalize_absolute(&self.root.join(supplied))
    }
}

pub fn normalize_absolute(path: &Path) -> Result<PathBuf> {
    if !path.is_absolute() {
        bail!("path {} is not absolute", path.display());
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::Normal(segment) => normalized.push(segment),
            Component::ParentDir => {
                if !normalized.pop() {
                    bail!(
                        "path {} traverses above its filesystem root",
                        path.display()
                    );
                }
            }
        }
    }
    Ok(normalized)
}

/// `project_root/relative`, created if missing, when every directory below
/// `project_root` on the way is a real directory: a symlink there (a
/// repository can ship one) would send writes outside the project.
pub(crate) fn real_project_dir(project_root: &Path, relative: &str) -> Result<PathBuf> {
    let mut directory = project_root.to_path_buf();
    for component in Path::new(relative).components() {
        let Component::Normal(part) = component else {
            bail!("{relative} is not a plain relative path");
        };
        directory.push(part);
        match std::fs::symlink_metadata(&directory) {
            Ok(metadata) if metadata.file_type().is_symlink() => bail!(
                "Refusing to write through {}: it is a symlink",
                directory.display()
            ),
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => bail!(
                "Refusing to write through {}: it is not a directory",
                directory.display()
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                match std::fs::create_dir(&directory) {
                    Ok(()) => {}
                    // Created concurrently: check it again.
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                        let metadata = std::fs::symlink_metadata(&directory)?;
                        if metadata.file_type().is_symlink() || !metadata.is_dir() {
                            bail!(
                                "Refusing to write through {}: it is not a directory",
                                directory.display()
                            );
                        }
                    }
                    Err(error) => {
                        return Err(anyhow::Error::from(error)
                            .context(format!("Failed to create {}", directory.display())))
                    }
                }
            }
            Err(error) => {
                return Err(anyhow::Error::from(error)
                    .context(format!("Failed to read {}", directory.display())))
            }
        }
    }
    Ok(directory)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_dirs_are_created_as_real_directories() {
        let root = tempfile::tempdir().unwrap();
        let created = real_project_dir(root.path(), ".arete/compositions").unwrap();
        assert_eq!(created, root.path().join(".arete/compositions"));
        assert!(created.is_dir());
        assert_eq!(
            real_project_dir(root.path(), ".arete/compositions").unwrap(),
            created
        );
        for bad in ["../outside", "/tmp", "a/../b"] {
            assert!(real_project_dir(root.path(), bad).is_err(), "{bad}");
        }
        std::fs::write(root.path().join("file"), "").unwrap();
        assert!(real_project_dir(root.path(), "file/sub").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn project_dirs_refuse_a_symlink_on_the_way() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join(".arete")).unwrap();
        let error = real_project_dir(root.path(), ".arete/compositions").unwrap_err();
        assert!(error.to_string().contains("symlink"), "{error}");
        assert!(!outside.path().join("compositions").exists());
    }

    #[test]
    fn outside_outputs_require_both_manifest_and_cli_consent() {
        let root = std::env::temp_dir().join(format!("arete-paths-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let outside = root.parent().unwrap().join("outside-sdk");

        let manifest_only = ProjectPaths::new(&root, true, false).unwrap();
        assert!(manifest_only
            .output(outside.to_str().unwrap(), "SDK output")
            .unwrap_err()
            .to_string()
            .contains("--allow-outside-project"));

        let both = ProjectPaths::new(&root, true, true).unwrap();
        assert_eq!(
            both.output(outside.to_str().unwrap(), "SDK output")
                .unwrap(),
            outside
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
