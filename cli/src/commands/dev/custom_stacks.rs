//! Compile the project's stacks and deploy them into its Arete cloud dev
//! environment. Stack Source uses the account compile API; Rust DSL crates
//! emit public artifacts with cargo check. Both use the same environment
//! deployment API and release selection as other stacks.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};

use super::api::DevApi;
use super::runtime::LogSink;
use crate::commands::public_artifacts::{load_local_artifact_stack_with_roots, LocalArtifactStack};

const STACKS_DIR: &str = "stacks";
const SOURCE_SUFFIXES: [&str; 2] = [".stack.yaml", ".stack.yml"];
const COMPILE_REQUEST_SCHEMA: &str = "arete.stack-compilation-request/v1";
/// Where one stack comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StackOrigin {
    /// A Stack Source YAML file.
    Source { path: PathBuf },
    /// An `[authoring.stacks]` entry's StackManifest, built by its crate
    /// (`crate_dir`) or, without one, loaded as it is.
    Manifest {
        entry: String,
        manifest: PathBuf,
        artifact_roots: Vec<PathBuf>,
        crate_dir: Option<PathBuf>,
    },
}

/// One of the project's own stacks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomStack {
    /// Its name in the environment.
    pub name: String,
    pub origin: StackOrigin,
}

impl CustomStack {
    /// `stack-source`, `rust` or `artifacts`.
    pub fn kind(&self) -> &'static str {
        match &self.origin {
            StackOrigin::Source { .. } => "stack-source",
            StackOrigin::Manifest {
                crate_dir: Some(_), ..
            } => "rust",
            StackOrigin::Manifest { .. } => "artifacts",
        }
    }

    /// Where it comes from, relative to `root`.
    pub fn source_label(&self, root: &Path) -> String {
        match &self.origin {
            StackOrigin::Source { path } => relative(root, path),
            StackOrigin::Manifest {
                entry, crate_dir, ..
            } => match crate_dir {
                Some(dir) if dir != root => {
                    format!("[authoring.stacks.{entry}] ({})", relative(root, dir))
                }
                _ => format!("[authoring.stacks.{entry}]"),
            },
        }
    }

    /// The directories and files a watcher should look at for this stack.
    pub fn watch_targets(&self, inputs: &BTreeSet<PathBuf>) -> (Vec<PathBuf>, Vec<PathBuf>) {
        let mut files: Vec<PathBuf> = inputs.iter().cloned().collect();
        let mut dirs = Vec::new();
        match &self.origin {
            StackOrigin::Source { path } => files.push(path.clone()),
            StackOrigin::Manifest {
                manifest,
                crate_dir,
                ..
            } => match crate_dir {
                Some(dir) => {
                    dirs.push(dir.join("src"));
                    dirs.push(dir.join("idl"));
                    files.push(dir.join("Cargo.toml"));
                    files.push(dir.join("build.rs"));
                }
                None => files.push(manifest.clone()),
            },
        }
        (dirs, files)
    }
}

/// `path` relative to `root`, with `/` separators, or as it is.
pub fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .map(|relative| {
            relative
                .components()
                .map(|component| component.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/")
        })
        .unwrap_or_else(|_| path.display().to_string())
}

/// `^[a-z0-9][a-z0-9-]{0,63}$`: the environment's stack names.
pub fn valid_stack_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 64
        && (bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit())
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
}

/// A file or entry name as a stack name: lowercase, `_`, `.` and spaces as
/// `-`. `None` when nothing usable is left.
pub fn stack_name_from(text: &str) -> Option<String> {
    let mut name = String::new();
    for character in text.chars() {
        let character = character.to_ascii_lowercase();
        if character.is_ascii_lowercase() || character.is_ascii_digit() {
            name.push(character);
        } else if matches!(character, '-' | '_' | '.' | ' ') && !name.ends_with('-') {
            name.push('-');
        }
    }
    let name = name.trim_matches('-').to_string();
    let name: String = name.chars().take(64).collect();
    valid_stack_name(&name).then_some(name)
}

/// The stack file name without its suffix, if it is a Stack Source file.
fn source_stem(path: &Path) -> Option<&str> {
    let name = path.file_name()?.to_str()?;
    SOURCE_SUFFIXES
        .iter()
        .find_map(|suffix| name.strip_suffix(suffix))
        .filter(|stem| !stem.is_empty())
}

/// What [`detect`] found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Detection {
    pub stacks: Vec<CustomStack>,
    /// Stacks or entries that were skipped, and why.
    pub notes: Vec<String>,
}

impl Detection {
    pub fn is_empty(&self) -> bool {
        self.stacks.is_empty()
    }

    fn push(&mut self, stack: CustomStack, root: &Path) {
        if let Some(existing) = self.stacks.iter().find(|other| other.name == stack.name) {
            self.notes.push(format!(
                "Skipped {}: stack '{}' already comes from {}; rename one of them",
                stack.source_label(root),
                stack.name,
                existing.source_label(root)
            ));
            return;
        }
        self.stacks.push(stack);
    }
}

/// Find the project's own stacks (see the module docs for the rules).
pub fn detect(root: &Path) -> Result<Detection> {
    let mut detection = Detection::default();
    let mut sources = Vec::new();
    let stacks_dir = root.join(STACKS_DIR);
    if stacks_dir.is_dir() {
        let mut entries = read_dir_sorted(&stacks_dir)?;
        let mut nested = Vec::new();
        for path in entries.drain(..) {
            if path.is_dir() {
                nested.extend(read_dir_sorted(&path)?.into_iter().filter(|p| p.is_file()));
            } else if path.is_file() {
                sources.push(path);
            }
        }
        sources.extend(nested);
        sources.retain(|path| source_stem(path).is_some());
    }

    let manifest = read_manifest(root)?;
    for listed in dev_stack_paths(&manifest)? {
        let path = root.join(&listed);
        if source_stem(&path).is_none() {
            detection.notes.push(format!(
                "Skipped '{listed}' in [dev] stacks: Stack Source files end in .stack.yaml"
            ));
            continue;
        }
        if !path.is_file() {
            detection
                .notes
                .push(format!("Skipped '{listed}' in [dev] stacks: no such file"));
            continue;
        }
        if !sources.contains(&path) {
            sources.push(path);
        }
    }
    for path in sources {
        let stem = source_stem(&path).unwrap_or_default();
        match stack_name_from(stem) {
            Some(name) => detection.push(
                CustomStack {
                    name,
                    origin: StackOrigin::Source { path },
                },
                root,
            ),
            None => detection.notes.push(format!(
                "Skipped {}: '{stem}' gives no stack name (letters, digits and '-')",
                relative(root, &path)
            )),
        }
    }

    let authored = manifest
        .as_ref()
        .and_then(|value| value.get("authoring"))
        .and_then(|value| value.get("stacks"))
        .and_then(toml::Value::as_table);
    for (entry, table) in authored.into_iter().flatten() {
        let Some(manifest_path) = table.get("manifest").and_then(toml::Value::as_str) else {
            detection.notes.push(format!(
                "Skipped [authoring.stacks.{entry}]: a composed stack (live views of published stacks) runs on Arete; `a4 up {entry}` deploys it"
            ));
            continue;
        };
        let Some(name) = stack_name_from(entry) else {
            detection.notes.push(format!(
                "Skipped [authoring.stacks.{entry}]: the name gives no stack name"
            ));
            continue;
        };
        let manifest_path = normalize(&root.join(manifest_path));
        let roots: Vec<PathBuf> = table
            .get("artifact_roots")
            .and_then(toml::Value::as_array)
            .map(|roots| {
                roots
                    .iter()
                    .filter_map(toml::Value::as_str)
                    .map(|root_path| normalize(&root.join(root_path)))
                    .collect()
            })
            .filter(|roots: &Vec<PathBuf>| !roots.is_empty())
            .unwrap_or_else(|| {
                vec![manifest_path
                    .parent()
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| root.to_path_buf())]
            });
        let crate_dir = crate_dir_of(root, &manifest_path);
        detection.push(
            CustomStack {
                name,
                origin: StackOrigin::Manifest {
                    entry: entry.clone(),
                    manifest: manifest_path,
                    artifact_roots: roots,
                    crate_dir,
                },
            },
            root,
        );
    }
    Ok(detection)
}

fn read_dir_sorted(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = std::fs::read_dir(dir)
        .with_context(|| format!("Failed to read {}", dir.display()))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| !name.starts_with('.'))
        })
        .collect::<Vec<_>>();
    paths.sort();
    Ok(paths)
}

/// `a/./b/../c` → `a/c`, without touching the file system.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// The nearest directory at or above `manifest`'s with a `Cargo.toml`, not
/// above `root`.
fn crate_dir_of(root: &Path, manifest: &Path) -> Option<PathBuf> {
    let mut dir = manifest.parent()?;
    loop {
        if dir.join("Cargo.toml").is_file() {
            return Some(dir.to_path_buf());
        }
        if dir == root {
            return None;
        }
        dir = dir.parent().filter(|parent| parent.starts_with(root))?;
    }
}

fn read_manifest(root: &Path) -> Result<Option<toml::Value>> {
    let path = root.join("arete.toml");
    match std::fs::read_to_string(&path) {
        Ok(text) => toml::from_str(&text)
            .map(Some)
            .map_err(|error| anyhow!("Invalid {}: {error}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("Failed to read {}", path.display())),
    }
}

/// `[dev] stacks = ["path/to/x.stack.yaml", ...]`.
fn dev_stack_paths(manifest: &Option<toml::Value>) -> Result<Vec<String>> {
    let Some(stacks) = manifest
        .as_ref()
        .and_then(|value| value.get("dev"))
        .and_then(|dev| dev.get("stacks"))
    else {
        return Ok(Vec::new());
    };
    let paths = stacks
        .as_array()
        .ok_or_else(|| anyhow!("[dev] stacks must be a list of Stack Source paths"))?;
    paths
        .iter()
        .map(|path| {
            path.as_str()
                .map(str::to_string)
                .ok_or_else(|| anyhow!("[dev] stacks must be a list of Stack Source paths"))
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Stack Source bundles and compile requests

/// A Stack Source file and the IDLs it names, as the compiler reads them:
/// paths relative to the source file's directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceBundle {
    /// The source file's name (the bundle root is its directory).
    pub entrypoint: String,
    /// `(bundle path, content)`, the entrypoint first.
    pub files: Vec<(String, String)>,
    /// Bundle path → where the file was read, relative to the project.
    pub origins: BTreeMap<String, String>,
    /// The files read, absolute.
    pub inputs: BTreeSet<PathBuf>,
    /// The source as JSON, when it parses.
    pub document: Option<Value>,
}

impl SourceBundle {
    fn content(&self, bundle_path: &str) -> Option<&str> {
        self.files
            .iter()
            .find(|(path, _)| path == bundle_path)
            .map(|(_, content)| content.as_str())
    }
}

/// A relative POSIX bundle path without `.`/`..` segments, or `None`.
fn bundle_path(reference: &str) -> Option<String> {
    let reference = reference.strip_prefix("./").unwrap_or(reference);
    if reference.is_empty() || reference.starts_with('/') || reference.contains('\\') {
        return None;
    }
    let segments: Vec<&str> = reference.split('/').collect();
    if segments
        .iter()
        .any(|segment| segment.is_empty() || *segment == "." || *segment == "..")
    {
        return None;
    }
    Some(segments.join("/"))
}

/// Read `source` and the local IDLs its `programs` name. An IDL path is read
/// beside the source file, else from the project root (so a stack in
/// `stacks/` can name the project's `./idl/x.json`). Unreadable YAML or a
/// missing IDL is left for the compiler to report with its location.
pub fn assemble_bundle(source: &Path, project_root: &Path) -> Result<SourceBundle> {
    let entrypoint = source
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow!("{} has no UTF-8 file name", source.display()))?
        .to_string();
    let text = std::fs::read_to_string(source)
        .with_context(|| format!("Failed to read {}", source.display()))?;
    let base = source.parent().unwrap_or(project_root);
    let mut bundle = SourceBundle {
        entrypoint: entrypoint.clone(),
        files: vec![(entrypoint.clone(), text.clone())],
        origins: BTreeMap::from([(entrypoint, relative(project_root, source))]),
        inputs: BTreeSet::from([source.to_path_buf()]),
        document: None,
    };
    let document: Option<Value> = serde_yaml::from_str::<serde_yaml::Value>(&text)
        .ok()
        .and_then(|value| serde_json::to_value(value).ok());
    let references: Vec<String> = document
        .as_ref()
        .and_then(|document| document.get("programs"))
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter_map(|(_, program)| program["source"].get("idl")?.as_str().map(str::to_string))
        .collect();
    for reference in references {
        let Some(path) = bundle_path(&reference) else {
            continue;
        };
        if bundle.content(&path).is_some() {
            continue;
        }
        let candidates = [base.join(&path), project_root.join(&path)];
        // Watch both places, so creating either one rebuilds the stack.
        bundle.inputs.extend(candidates.iter().cloned());
        let Some(found) = candidates.iter().find(|candidate| candidate.is_file()) else {
            continue;
        };
        let content = std::fs::read_to_string(found)
            .with_context(|| format!("Failed to read {}", found.display()))?;
        bundle
            .origins
            .insert(path.clone(), relative(project_root, found));
        bundle.files.push((path, content));
    }
    bundle.document = document;
    Ok(bundle)
}

/// The `/api/stack-compilations/compile` request body.
pub fn compile_request(bundle: &SourceBundle) -> Value {
    json!({
        "schema": COMPILE_REQUEST_SCHEMA,
        "entrypoint": bundle.entrypoint,
        "files": bundle
            .files
            .iter()
            .map(|(path, content)| json!({ "path": path, "content": content }))
            .collect::<Vec<_>>(),
        "mode": "compile",
    })
}

/// Files that should trigger a rebuild, including local IDLs and Rust
/// sources; generated artifacts and Cargo target directories are excluded.
pub fn snapshot(root: &Path) -> super::watch::Snapshot {
    let mut snapshot = super::watch::Snapshot::new();
    let record = |path: &Path, snapshot: &mut super::watch::Snapshot| {
        if let Ok(metadata) = std::fs::metadata(path) {
            snapshot.insert(
                path.to_path_buf(),
                (metadata.modified().ok(), metadata.len()),
            );
        }
    };
    record(&root.join("arete.toml"), &mut snapshot);
    let Ok(detection) = detect(root) else {
        return snapshot;
    };

    for stack in detection.stacks {
        let inputs = match &stack.origin {
            StackOrigin::Source { path } => assemble_bundle(path, root)
                .map(|bundle| bundle.inputs)
                .unwrap_or_default(),
            _ => BTreeSet::new(),
        };
        let (dirs, files) = stack.watch_targets(&inputs);
        for file in files {
            record(&file, &mut snapshot);
        }
        for dir in dirs {
            super::watch::walk(&dir, &mut snapshot, 0);
        }
    }
    snapshot
}

/// Compile and deploy each detected stack. The caller keeps the previous
/// session endpoints when a rebuild fails.
pub async fn deploy(
    root: &Path,
    api: &arete_mcp::dev_env::EnvApi,
    environment: &str,
    log: &LogSink,
) -> Result<Vec<super::api::EnvStack>> {
    let detection = detect(root)?;
    for note in &detection.notes {
        log.line(note);
    }
    if detection.is_empty() {
        return Ok(Vec::new());
    }
    let mut names = BTreeSet::new();
    let mut metadata = Vec::new();
    for stack in detection.stacks {
        log.line(&format!(
            "Building {} ({}, {})",
            stack.name,
            stack.kind(),
            stack.source_label(root)
        ));
        let (artifacts, source_idls) = match &stack.origin {
            StackOrigin::Source { path } => {
                let bundle = assemble_bundle(path, root)?;
                let response = api
                    .http()
                    .post(format!("{}/api/stack-compilations/compile", api.origin()))
                    .bearer_auth(api.key())
                    .json(&compile_request(&bundle))
                    .send()
                    .await?;
                let status = response.status();
                let response: Value = response.json().await?;
                if response["valid"] == false {
                    let diagnostics = response["diagnostics"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|d| {
                            let file = d["file"].as_str().unwrap_or(&bundle.entrypoint);
                            let file = bundle.origins.get(file).map(String::as_str).unwrap_or(file);
                            format!(
                                "{}:{}:{}: {}",
                                file,
                                d.pointer("/range/start/line").unwrap_or(&Value::Null),
                                d.pointer("/range/start/column").unwrap_or(&Value::Null),
                                d["message"].as_str().unwrap_or("compile error")
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    bail!("Stack {} did not compile\n{diagnostics}", stack.name);
                }
                if !status.is_success() {
                    bail!(
                        "Compiling {} failed ({status}): {response}",
                        relative(root, path)
                    );
                }
                let out = &response["artifacts"];
                let manifest: arete_artifacts::StackManifestArtifactV2 =
                    serde_json::from_value(out["stackManifest"].clone())?;
                let live: arete_artifacts::LiveSpecArtifactV2 =
                    serde_json::from_value(out["liveSpec"].clone())?;
                let alias = manifest
                    .payload
                    .live_specs
                    .first()
                    .context("Compiled stack has no LiveSpec")?
                    .alias
                    .clone();
                (
                    LocalArtifactStack {
                        manifest_path: path.clone(),
                        manifest_hash: manifest.artifact_hash.to_string(),
                        program_specs: serde_json::from_value(out["programSpecs"].clone())?,
                        live_specs: vec![(alias, live)],
                        stack_manifest: manifest,
                    },
                    out.get("sourceIdls").cloned().unwrap_or(json!({})),
                )
            }
            StackOrigin::Manifest {
                manifest,
                artifact_roots,
                crate_dir,
                ..
            } => {
                if let Some(dir) = crate_dir {
                    let output = tokio::process::Command::new("cargo")
                        .args(["check", "--locked", "--message-format=short"])
                        .current_dir(dir)
                        .output()
                        .await
                        .context("Rust stacks need cargo on PATH")?;
                    log.line(&String::from_utf8_lossy(&output.stderr));
                    if !output.status.success() {
                        bail!(
                            "Rust stack {} did not compile: {}",
                            stack.name,
                            String::from_utf8_lossy(&output.stderr)
                        );
                    }
                }
                (
                    load_local_artifact_stack_with_roots(manifest, artifact_roots)?,
                    json!({}),
                )
            }
        };
        for (index, (alias, _)) in artifacts.live_specs.iter().enumerate() {
            let name = crate::commands::up::child_spec_name(&stack.name, alias, index);
            names.insert(name.clone());
            let views: Vec<_> = artifacts
                .stack_manifest
                .payload
                .selected_views
                .iter()
                .filter(|view| view.live_alias == *alias)
                .map(|view| view.view_id.clone())
                .collect();
            metadata.push(json!({ "name": name, "views": views }));
        }
        let origin = api.origin().to_string();
        let key = api.key().to_string();
        let environment = environment.to_string();
        let name = stack.name;
        tokio::task::spawn_blocking(move || {
            crate::commands::up::deploy_environment_stack(
                &origin,
                &key,
                &environment,
                &name,
                artifacts,
                source_idls,
            )
        })
        .await??;
    }
    std::fs::write(
        root.join(".arete/dev/stacks.json"),
        serde_json::to_vec_pretty(&metadata)?,
    )?;
    Ok(DevApi::new(api.origin(), api.key())
        .env_stacks(environment)
        .await?
        .into_iter()
        .filter_map(|entry| super::api::env_stack(&entry))
        .filter(|stack| names.contains(&stack.name))
        .collect())
}
