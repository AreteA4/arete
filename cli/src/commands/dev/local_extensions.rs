//! Local SDK extensions under `a4 dev up`: each registry program or stack
//! dependency with `extensions = { path = "<dir>" }` in arete.toml has that
//! directory watched, and a change regenerates only that dependency's SDK
//! with the bundle as it is now (arete.lock records it as
//! `local_extension`).

use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::project::installer::{install_local_extensions, LocalExtensionInstall};
use crate::project::manifest::DependencyKind;
use crate::project::ProjectManifest;

/// Each dependency with a local SDK extension, and the bundle directory.
fn directories(root: &Path) -> Vec<(DependencyKind, String, PathBuf)> {
    let Ok(manifest) = ProjectManifest::load(root.join("arete.toml")) else {
        return Vec::new();
    };
    manifest
        .dependencies()
        .filter_map(|(kind, alias, dependency)| {
            let extensions = dependency.extensions.as_ref()?;
            Some((kind, alias.clone(), root.join(&extensions.path)))
        })
        .collect()
}

/// The files of every local SDK extension directory now.
pub fn snapshot(root: &Path) -> super::watch::Snapshot {
    let mut snapshot = super::watch::Snapshot::new();
    for (_, _, dir) in directories(root) {
        super::watch::walk(&dir, &mut snapshot, 0);
    }
    snapshot
}

/// The dependencies whose local SDK extension directory holds a changed
/// path, and whether every changed path is in one.
pub fn changed(root: &Path, changed: &[PathBuf]) -> (Vec<(DependencyKind, String)>, bool) {
    let directories = directories(root);
    let mut selected = Vec::new();
    let mut only_extensions = !changed.is_empty();
    for path in changed {
        let mut inside = false;
        for (kind, alias, dir) in &directories {
            if path.starts_with(dir) {
                inside = true;
                if !selected
                    .iter()
                    .any(|(k, a): &(DependencyKind, String)| k == kind && a == alias)
                {
                    selected.push((*kind, alias.clone()));
                }
            }
        }
        only_extensions &= inside;
    }
    (selected, only_extensions)
}

/// Regenerate the SDKs of `selected` with their local SDK extensions.
pub fn regenerate(
    root: &Path,
    selected: &[(DependencyKind, String)],
) -> Result<Vec<LocalExtensionInstall>> {
    if selected.is_empty() {
        return Ok(Vec::new());
    }
    install_local_extensions(&root.join("arete.toml"), selected)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = r#"manifest_version = 1

[project]
name = "app"

[dependencies.programs.ore]
source = { registry = "ore" }
version = "^1.0.0"
extensions = { path = "extensions/ore" }

[dependencies.stacks.plain]
source = { registry = "plain" }
version = "^1.0.0"
"#;

    #[test]
    fn watches_and_selects_only_extension_directories() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("arete.toml"), MANIFEST).unwrap();
        std::fs::create_dir_all(root.join("extensions/ore")).unwrap();
        std::fs::write(root.join("extensions/ore/index.ts"), "export {};").unwrap();
        std::fs::write(root.join("README.md"), "a").unwrap();

        let before = snapshot(root);
        assert_eq!(
            before.keys().cloned().collect::<Vec<_>>(),
            [root.join("extensions/ore/index.ts")]
        );

        std::fs::write(root.join("extensions/ore/index.ts"), "export const a = 1;").unwrap();
        let paths = super::super::watch::changed_paths(&before, &snapshot(root));
        assert_eq!(
            changed(root, &paths),
            (vec![(DependencyKind::Program, "ore".to_string())], true)
        );

        let mixed = vec![
            root.join("extensions/ore/index.ts"),
            root.join("protocol.ts"),
        ];
        assert_eq!(
            changed(root, &mixed),
            (vec![(DependencyKind::Program, "ore".to_string())], false)
        );
        assert_eq!(
            changed(root, &[root.join("protocol.ts")]),
            (Vec::new(), false)
        );
    }
}
