use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use colored::Colorize;

use crate::project::installer;
use crate::project::manifest::{AuthoringProgramV1, AuthoringStackV1, ManifestV1};

pub fn init(config_path: &str) -> Result<()> {
    let path = Path::new(config_path);
    if path.exists() {
        anyhow::bail!(
            "Configuration file already exists: {}\nUse a different path or remove the existing file.",
            path.display()
        );
    }
    let root = path.parent().unwrap_or_else(|| Path::new("."));
    println!("{} Initializing Arete project...\n", "→".blue().bold());
    let discovered = discover_public_artifacts(root)?;
    println!(
        "{} Found {} StackManifest(s) and {} ProgramSpec(s)",
        "→".blue().bold(),
        discovered.stacks.len(),
        discovered.programs.len()
    );

    let project_name = prompt_project_name(root)?;
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

    let contents = manifest.to_toml_pretty()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, contents)
        .with_context(|| format!("Failed to write project manifest {}", path.display()))?;
    println!("{} Created {}", "✓".green().bold(), path.display());
    println!(
        "Run {} to validate local artifact closure and output ownership.",
        "a4 config validate".cyan()
    );
    Ok(())
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
struct DiscoveredArtifacts {
    stacks: BTreeMap<String, String>,
    programs: BTreeMap<String, String>,
}

fn discover_public_artifacts(root: &Path) -> Result<DiscoveredArtifacts> {
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

fn prompt_project_name(root: &Path) -> Result<String> {
    let default_name = root
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "my-project".to_string());
    print!("Project name [{}]: ", default_name.dimmed());
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let input = input.trim();
    Ok(if input.is_empty() {
        default_name
    } else {
        input.to_string()
    })
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
}
