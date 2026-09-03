use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use colored::Colorize;

use crate::project::installer;
use crate::project::manifest::{AuthoringProgramV1, AuthoringStackV1, ManifestV1};

/// Build the manifest `a4 init` writes for `root`: `[project]` from `name`
/// plus every discovered public artifact under `[authoring]`.
pub(crate) fn build_manifest(root: &Path, name: Option<String>) -> Result<ManifestV1> {
    let discovered = discover_public_artifacts(root)?;
    let project_name = name.unwrap_or_else(|| default_project_name(root));
    let mut manifest = ManifestV1::new(project_name);
    manifest.authoring.stacks = discovered
        .stacks
        .into_iter()
        .map(|(alias, artifact)| {
            let artifact_root = Path::new(&artifact)
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .to_string_lossy()
                .into_owned();
            (
                alias,
                AuthoringStackV1 {
                    manifest: artifact,
                    artifact_roots: vec![if artifact_root.is_empty() {
                        ".".into()
                    } else {
                        artifact_root
                    }],
                    deployment_name: None,
                },
            )
        })
        .collect();
    manifest.authoring.programs = discovered
        .programs
        .into_iter()
        .map(|(alias, program_spec)| (alias, AuthoringProgramV1 { program_spec }))
        .collect();
    manifest.validate()?;
    Ok(manifest)
}

/// Content of a fresh `arete.toml` for `root`.
pub(crate) fn new_manifest_contents(root: &Path, name: Option<String>) -> Result<String> {
    build_manifest(root, name)?.to_toml_pretty()
}

/// `--force`: rewrite only the `[project]` table of an existing manifest,
/// leaving every other table (and comments) as they are.
pub(crate) fn rewrite_project_table(existing: &str, name: &str) -> Result<String> {
    let mut doc: toml_edit::DocumentMut = existing
        .parse()
        .context("Existing arete.toml is not valid TOML")?;
    let project = doc
        .entry("project")
        .or_insert(toml_edit::Item::Table(toml_edit::Table::new()));
    let table = project
        .as_table_mut()
        .ok_or_else(|| anyhow::anyhow!("`project` in arete.toml is not a table"))?;
    // Only `name` is rewritten; every other [project] field (and any table
    // decoration) survives a --force run.
    table.insert("name", toml_edit::value(name));
    if !table.contains_key("private") {
        table.insert("private", toml_edit::value(false));
    }
    Ok(doc.to_string())
}

pub fn validate(config_path: &str) -> Result<()> {
    println!("{} Validating project manifest...", "→".blue().bold());
    let (manifest, plan, lock) = installer::validate_project(config_path, true).context(
        "Failed to validate project manifest. Run `a4 init` to create manifest_version = 1.",
    )?;
    println!("{} Project manifest is valid", "✓".green().bold());
    println!("  Project: {}", manifest.document.project.name.bold());
    println!("  Dependencies: {}", manifest.dependencies().count());
    println!("  Planned outputs: {}", plan.outputs.len());
    match lock {
        Some(lock) if lock.is_fresh(&manifest.manifest_hash) => {
            println!("  Lock: {}", "fresh".green())
        }
        Some(_) => println!("  Lock: {}", "stale".yellow()),
        None => println!("  Lock: {}", "missing".yellow()),
    }
    Ok(())
}

#[derive(Default)]
pub(crate) struct DiscoveredArtifacts {
    pub(crate) stacks: BTreeMap<String, String>,
    pub(crate) programs: BTreeMap<String, String>,
}

pub(crate) fn discover_public_artifacts(root: &Path) -> Result<DiscoveredArtifacts> {
    let root = fs::canonicalize(root)
        .with_context(|| format!("Failed to resolve project directory {}", root.display()))?;
    let mut files = Vec::new();
    discover_files(&root, &root, 0, &mut files)?;
    files.sort();
    let mut discovered = DiscoveredArtifacts::default();
    for file in files {
        let relative = file
            .strip_prefix(&root)
            .expect("discovery stays beneath root")
            .to_string_lossy()
            .replace(std::path::MAIN_SEPARATOR, "/");
        let name = file
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        if let Some(stem) = name.strip_suffix(".stack-manifest.json") {
            insert_discovered(&mut discovered.stacks, portable_alias(stem), relative);
        } else if let Some(stem) = name.strip_suffix(".program-spec.json") {
            insert_discovered(&mut discovered.programs, portable_alias(stem), relative);
        }
    }
    Ok(discovered)
}

fn discover_files(
    root: &Path,
    directory: &Path,
    depth: usize,
    files: &mut Vec<PathBuf>,
) -> Result<()> {
    if depth > 4 {
        return Ok(());
    }
    let mut entries = fs::read_dir(directory)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(std::fs::DirEntry::path);
    for entry in entries {
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default();
            if matches!(name, "target" | "node_modules" | ".git")
                || (name.starts_with('.') && name != ".arete")
            {
                continue;
            }
            discover_files(root, &path, depth + 1, files)?;
        } else if metadata.is_file()
            && path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    name.ends_with(".stack-manifest.json") || name.ends_with(".program-spec.json")
                })
        {
            debug_assert!(path.starts_with(root));
            files.push(path);
        }
    }
    Ok(())
}

fn insert_discovered(map: &mut BTreeMap<String, String>, base: String, path: String) {
    let mut alias = base.clone();
    let mut suffix = 2;
    while map.contains_key(&alias) {
        alias = format!("{base}-{suffix}");
        suffix += 1;
    }
    map.insert(alias, path);
}

fn portable_alias(value: &str) -> String {
    let mut alias = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    while alias.contains("--") {
        alias = alias.replace("--", "-");
    }
    alias = alias.trim_matches('-').to_string();
    if alias.is_empty() {
        "artifact".into()
    } else {
        alias.chars().take(64).collect()
    }
}

/// Project name when `--name` is absent: the directory basename, never a prompt.
pub fn default_project_name(root: &Path) -> String {
    fs::canonicalize(root)
        .ok()
        .as_deref()
        .unwrap_or(root)
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "my-project".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_never_treats_stack_json_as_an_arete_artifact() {
        let root = std::env::temp_dir().join(format!("arete-init-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join(".arete")).unwrap();
        fs::write(root.join(".arete/Legacy.stack.json"), "{}").unwrap();
        fs::write(root.join(".arete/Exact.stack-manifest.json"), "{}").unwrap();
        let discovered = discover_public_artifacts(&root).unwrap();
        assert_eq!(discovered.stacks.len(), 1);
        assert!(discovered.stacks.contains_key("exact"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn force_rewrites_only_the_project_table() {
        let existing = "# keep\nmanifest_version = 1\n\n[project]\nname = \"old\"\nprivate = true\nextra = 1\n\n[sdk]\ntargets = [\"typescript\"]\n";
        let rewritten = rewrite_project_table(existing, "new").unwrap();
        assert!(rewritten.contains("# keep"));
        assert!(rewritten.contains("name = \"new\""));
        assert!(rewritten.contains("private = true"));
        // Other [project] fields survive --force.
        assert!(rewritten.contains("extra = 1"));
        assert!(rewritten.contains("[sdk]\ntargets = [\"typescript\"]"));
    }
}
