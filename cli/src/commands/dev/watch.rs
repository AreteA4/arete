//! A polling watcher over the protocol sources: `protocol.ts` (and any
//! other `protocol.*` file), `handlers/`, `idl/` and `arete.lock`, so an
//! `a4 install` or `a4 update` of a program the protocol imports redeploys
//! it, and each local SDK extension directory arete.toml names. Polling
//! keeps it free of platform file-event APIs; a project is small.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Path → (modified time, size).
pub type Snapshot = BTreeMap<PathBuf, (Option<SystemTime>, u64)>;

const WATCHED_DIRS: [&str; 2] = ["handlers", "idl"];
const WATCHED_FILES: [&str; 1] = ["arete.lock"];
const SKIPPED: [&str; 4] = ["node_modules", ".arete", ".git", "target"];

fn record(path: &Path, snapshot: &mut Snapshot) {
    if let Ok(metadata) = std::fs::metadata(path) {
        snapshot.insert(
            path.to_path_buf(),
            (metadata.modified().ok(), metadata.len()),
        );
    }
}

pub(super) fn walk(dir: &Path, snapshot: &mut Snapshot, depth: usize) {
    if depth > 8 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        if SKIPPED.iter().any(|skip| name == *skip) {
            continue;
        }
        match entry.file_type() {
            Ok(kind) if kind.is_dir() => walk(&path, snapshot, depth + 1),
            Ok(kind) if kind.is_file() => record(&path, snapshot),
            _ => {}
        }
    }
}

/// The watched files of `root` now.
pub fn snapshot(root: &Path) -> Snapshot {
    let mut snapshot = source_snapshot(root);
    snapshot.extend(super::local_extensions::snapshot(root));
    snapshot
}

/// The watched files that rebuild the protocol or the project's stacks:
/// [`snapshot`] without the local SDK extension directories.
pub fn source_snapshot(root: &Path) -> Snapshot {
    let mut snapshot = Snapshot::new();
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            if name.to_string_lossy().starts_with("protocol.") && entry.path().is_file() {
                record(&entry.path(), &mut snapshot);
            }
        }
    }
    for dir in WATCHED_DIRS {
        walk(&root.join(dir), &mut snapshot, 0);
    }
    for file in WATCHED_FILES {
        record(&root.join(file), &mut snapshot);
    }
    snapshot.extend(super::custom_stacks::snapshot(root));
    snapshot
}

/// Take `relative`'s current state into `snapshot`, so a write this
/// process made is not seen as a change.
pub fn refresh(snapshot: &mut Snapshot, root: &Path, relative: &str) {
    let path = root.join(relative);
    snapshot.remove(&path);
    if path.is_file() {
        record(&path, snapshot);
    }
}

/// Files added, removed or modified between two snapshots.
pub fn changed_paths(before: &Snapshot, after: &Snapshot) -> Vec<PathBuf> {
    let mut changed: Vec<PathBuf> = after
        .iter()
        .filter(|(path, stamp)| before.get(*path) != Some(*stamp))
        .map(|(path, _)| path.clone())
        .collect();
    changed.extend(
        before
            .keys()
            .filter(|path| !after.contains_key(*path))
            .cloned(),
    );
    changed.sort();
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sees_protocol_handler_and_idl_changes_only() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("protocol.ts"), "a").unwrap();
        std::fs::create_dir_all(root.join("handlers/nested")).unwrap();
        std::fs::create_dir_all(root.join("node_modules/x")).unwrap();
        std::fs::create_dir_all(root.join("idl")).unwrap();
        std::fs::write(root.join("handlers/nested/a.ts"), "a").unwrap();
        std::fs::write(root.join("README.md"), "a").unwrap();
        let before = snapshot(root);
        assert_eq!(before.len(), 2);

        std::fs::write(root.join("README.md"), "changed").unwrap();
        std::fs::write(root.join("node_modules/x/i.js"), "x").unwrap();
        assert!(changed_paths(&before, &snapshot(root)).is_empty());

        std::fs::write(root.join("handlers/nested/a.ts"), "longer").unwrap();
        std::fs::write(root.join("idl/p.json"), "{}").unwrap();
        let after = snapshot(root);
        let changed = changed_paths(&before, &after);
        assert_eq!(changed.len(), 2, "{changed:?}");

        std::fs::remove_file(root.join("protocol.ts")).unwrap();
        assert_eq!(
            changed_paths(&after, &snapshot(root)),
            vec![root.join("protocol.ts")]
        );
    }

    #[test]
    fn sees_arete_lock_changes() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("arete.toml"), "a").unwrap();
        let before = snapshot(root);
        assert_eq!(before.len(), 1);
        std::fs::write(root.join("arete.lock"), "lock_version = 1").unwrap();
        let after = snapshot(root);
        assert_eq!(
            changed_paths(&before, &after),
            vec![root.join("arete.lock")]
        );
        std::fs::write(root.join("arete.lock"), "lock_version = 1\n# updated").unwrap();
        assert_eq!(
            changed_paths(&after, &snapshot(root)),
            vec![root.join("arete.lock")]
        );
    }

    #[test]
    fn a_refreshed_file_is_not_a_change() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("arete.lock"), "lock_version = 1").unwrap();
        let mut before = snapshot(root);
        std::fs::write(root.join("arete.lock"), "lock_version = 1\n# local_build").unwrap();
        refresh(&mut before, root, "arete.lock");
        assert!(changed_paths(&before, &snapshot(root)).is_empty());
    }
}
