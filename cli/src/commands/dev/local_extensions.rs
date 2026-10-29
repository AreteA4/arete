//! Local SDK extensions under `a4 dev up`: each registry program or stack
//! dependency with `extensions = { path = "<dir>" }` in arete.toml has its
//! bundle's files watched (never the whole directory, which may hold the
//! generated SDK itself), and a change regenerates only that dependency's SDK
//! with the bundle as it is now (arete.lock records it as
//! `local_extension`).

use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::project::installer::{install_local_extensions, LocalExtensionInstall};
use crate::project::manifest::DependencyKind;
use crate::project::ProjectManifest;

/// The files a bundle in `dir` is read from: its `extensions.json`, each
/// file that manifest lists, and the entries read without one.
fn bundle_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = [
        "extensions.json",
        "index.ts",
        "extensions.rs",
        "extensions.py",
    ]
    .iter()
    .map(|name| dir.join(name))
    .collect();
    let listed = std::fs::read(dir.join("extensions.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
    if let Some(listed) = listed
        .as_ref()
        .and_then(|manifest| manifest["files"].as_array())
    {
        files.extend(
            listed
                .iter()
                .filter_map(|file| file.as_str())
                .map(|file| dir.join(file)),
        );
    }
    files.sort();
    files.dedup();
    files
}

/// Each dependency with a local SDK extension, and its bundle's files.
fn bundles(root: &Path) -> Vec<(DependencyKind, String, Vec<PathBuf>)> {
    let Ok(manifest) = ProjectManifest::load(root.join("arete.toml")) else {
        return Vec::new();
    };
    manifest
        .dependencies()
        .filter_map(|(kind, alias, dependency)| {
            let extensions = dependency.extensions.as_ref()?;
            Some((
                kind,
                alias.clone(),
                bundle_files(&root.join(&extensions.path)),
            ))
        })
        .collect()
}

/// The files of every local SDK extension bundle now.
pub fn snapshot(root: &Path) -> super::watch::Snapshot {
    let mut snapshot = super::watch::Snapshot::new();
    for (_, _, files) in bundles(root) {
        for file in files {
            if let Ok(metadata) = std::fs::metadata(&file) {
                if metadata.is_file() {
                    snapshot.insert(file, (metadata.modified().ok(), metadata.len()));
                }
            }
        }
    }
    snapshot
}

/// The dependencies whose local SDK extension bundle has a changed file,
/// and whether every changed path is only an extension file: one that
/// exists now and that the protocol and stack watch does not also cover (a
/// bundle may share a directory with them). A removed file counts as a
/// source change, to be safe.
pub fn changed(root: &Path, changed: &[PathBuf]) -> (Vec<(DependencyKind, String)>, bool) {
    let bundles = bundles(root);
    let sources = super::watch::source_snapshot(root);
    let mut selected = Vec::new();
    let mut only_extensions = !changed.is_empty();
    for path in changed {
        let mut inside = false;
        for (kind, alias, files) in &bundles {
            if files.contains(path) {
                inside = true;
                if !selected
                    .iter()
                    .any(|(k, a): &(DependencyKind, String)| k == kind && a == alias)
                {
                    selected.push((*kind, alias.clone()));
                }
            }
        }
        only_extensions &= inside && path.is_file() && !sources.contains_key(path);
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

    #[test]
    fn a_bundle_sharing_the_protocol_directory_still_rebuilds_the_protocol() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join("arete.toml"),
            MANIFEST.replace("extensions/ore", "."),
        )
        .unwrap();
        std::fs::write(root.join("index.ts"), "export {};").unwrap();
        std::fs::write(root.join("protocol.ts"), "a").unwrap();
        let ore = vec![(DependencyKind::Program, "ore".to_string())];
        assert_eq!(changed(root, &[root.join("index.ts")]), (ore.clone(), true));
        assert_eq!(
            changed(root, &[root.join("protocol.ts")]),
            (Vec::new(), false)
        );
        // A removed bundle file may have been a source file too: rebuild.
        std::fs::remove_file(root.join("index.ts")).unwrap();
        assert_eq!(changed(root, &[root.join("index.ts")]), (ore, false));
    }

    #[test]
    fn the_generated_sdk_beside_a_bundle_is_not_watched() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join("arete.toml"),
            MANIFEST.replace("extensions/ore", "."),
        )
        .unwrap();
        std::fs::write(
            root.join("extensions.json"),
            r#"{"entry":"ore-extensions.ts","files":["ore-extensions.ts"]}"#,
        )
        .unwrap();
        std::fs::write(root.join("ore-extensions.ts"), "export {};").unwrap();
        std::fs::create_dir_all(root.join("generated/typescript/programs/ore")).unwrap();
        std::fs::write(
            root.join("generated/typescript/programs/ore/ore-extensions.ts"),
            "export {};",
        )
        .unwrap();
        assert_eq!(
            snapshot(root).keys().cloned().collect::<Vec<_>>(),
            [root.join("extensions.json"), root.join("ore-extensions.ts")]
        );
        assert_eq!(
            changed(
                root,
                &[root.join("generated/typescript/programs/ore/ore-extensions.ts")]
            ),
            (Vec::new(), false)
        );
    }
}
