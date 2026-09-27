use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use toml_edit::{value, Array, DocumentMut, InlineTable, Item, Table};

use crate::api_client::ApiClient;
use crate::commands::public_artifacts::{load_local_artifact_stack_with_roots, LocalArtifactStack};
use crate::commands::sdk::{
    generate_project_composed_stack, generate_project_local_program, generate_project_local_stack,
    generate_project_registry_dependency, verify_resolved_stack_delivery, ProjectGenerationOptions,
};

use super::composition::{self, ComposedStack};
use super::lockfile::{LockedDependency, LockedLiveSpec, LockedProgram};
use super::manifest::{
    DependencyKind, DependencyOutputsV1, DependencySourceV1, DependencyV1, InstallTarget,
    ManifestV1, PathSourceV1, RegistrySourceV1, StackEndpointsV1, WorkspaceSourceV1,
};
use super::paths::ProjectPaths;
use super::registry_cache;
use super::resolver::{
    RegistryDependencyRequest, RegistryResolveRequest, ResolvedRegistryDependency,
};
use super::runtime;
use super::{InstallPlan, ProjectLock, ProjectManifest, GENERATOR_CONTRACT, RESOLVER_CONTRACT};

const INSTALL_JOURNAL: &str = ".arete/install-journal.json";

static JSON_OUTPUT: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

/// Records the parsed global `--json` flag once, before any command runs.
/// Project installs are reached from several commands, so they read it here
/// rather than through every call chain.
pub(crate) fn set_json_output(json: bool) {
    let _ = JSON_OUTPUT.set(json);
}

/// Whether this invocation reports its result as JSON. When it does, stdout
/// carries only the JSON result and progress moves to stderr.
pub(crate) fn json_output() -> bool {
    JSON_OUTPUT.get().copied().unwrap_or(false)
}

/// One human progress or summary line: stdout, or stderr under `--json`.
fn status_line(line: impl std::fmt::Display) {
    if json_output() {
        eprintln!("{line}");
    } else {
        println!("{line}");
    }
}
/// A hosted stack's exact delivery is transiently unavailable. It is not a
/// lock integrity failure: the locked release still resolves.
const DELIVERY_NOT_READY: &str = "delivery-not-ready";

#[derive(Debug, Clone, Copy, Default)]
pub struct InstallOptions<'a> {
    pub locked: bool,
    pub allow_outside_project: bool,
    pub dry_run: bool,
    pub update: Option<UpdateSelection<'a>>,
}

#[derive(Debug, Clone, Copy)]
pub struct UpdateSelection<'a> {
    pub kind: Option<DependencyKind>,
    pub alias: Option<&'a str>,
}

#[derive(Debug, Default)]
pub struct AddDependencyOptions {
    pub alias: Option<String>,
    pub exact: bool,
    pub target: Option<InstallTarget>,
    pub output: Option<String>,
    pub typescript_package: Option<String>,
    pub module: bool,
    pub allow_outside_project: bool,
}

#[derive(Debug, Default)]
pub struct NoSaveDependencyOptions {
    pub alias: Option<String>,
    pub target: Option<InstallTarget>,
    pub output: Option<String>,
    pub typescript_package: Option<String>,
    pub rust_crate_prefix: Option<String>,
    pub module: bool,
}

#[derive(Debug, Default)]
pub struct RemoveDependencyOptions {
    pub keep_output: bool,
    pub allow_outside_project: bool,
}

pub fn install_without_saving(
    kind: DependencyKind,
    package_spec: &str,
    options: NoSaveDependencyOptions,
) -> Result<()> {
    let (package, requirement) = split_package_requirement(package_spec)?;
    let alias = select_local_alias(kind, &package, options.alias.as_deref())?;
    let target = options.target.unwrap_or(InstallTarget::TypeScript);
    let invocation_root = fs::canonicalize(std::env::current_dir()?)?;
    let output = options.output.map(PathBuf::from).unwrap_or_else(|| {
        let suffix = match target {
            InstallTarget::TypeScript => alias.clone(),
            InstallTarget::Rust => format!("{alias}-stack"),
            InstallTarget::Python => format!("{alias}-py"),
        };
        PathBuf::from("generated").join(suffix)
    });
    let output = if output.is_absolute() {
        output
    } else {
        invocation_root.join(output)
    };
    let temporary_root = invocation_root
        .join(".arete")
        .join(format!("no-save-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&temporary_root)?;

    let result = (|| -> Result<()> {
        let mut manifest = ManifestV1::new(format!("no-save-{alias}"));
        manifest.project.private = true;
        manifest.install.allow_outside_project = true;
        manifest.sdk.targets = vec![target];
        if let Some(package_name) = options.typescript_package {
            manifest.sdk.typescript.package = package_name;
        }
        if let Some(crate_prefix) = options.rust_crate_prefix {
            manifest.sdk.rust.crate_prefix = crate_prefix;
        }
        match target {
            InstallTarget::Rust => manifest.sdk.rust.module_mode = options.module,
            InstallTarget::Python => manifest.sdk.python.module_mode = options.module,
            InstallTarget::TypeScript if options.module => {
                bail!("--module requires --rust or --python")
            }
            InstallTarget::TypeScript => {}
        }
        let mut outputs = DependencyOutputsV1::default();
        let output = output.to_string_lossy().into_owned();
        match target {
            InstallTarget::TypeScript => outputs.typescript = Some(output),
            InstallTarget::Rust => outputs.rust = Some(output),
            InstallTarget::Python => outputs.python = Some(output),
        }
        let dependency = DependencyV1 {
            source: DependencySourceV1::Registry(RegistrySourceV1 { registry: package }),
            version: Some(requirement.unwrap_or_else(|| "*".into())),
            targets: Some(vec![target]),
            outputs,
            endpoints: BTreeMap::new(),
        };
        let requested_alias = alias.clone();
        match kind {
            DependencyKind::Stack => manifest.dependencies.stacks.insert(alias, dependency),
            DependencyKind::Program => manifest.dependencies.programs.insert(alias, dependency),
        };
        manifest.validate()?;
        let manifest_path = temporary_root.join("arete.toml");
        fs::write(&manifest_path, manifest.to_toml_pretty()?)?;
        install_project_requesting(
            &manifest_path,
            InstallOptions {
                allow_outside_project: true,
                ..InstallOptions::default()
            },
            &[(kind, requested_alias)],
        )
    })();
    let _ = fs::remove_dir_all(&temporary_root);
    result
}

pub fn add_and_install(
    manifest_path: impl AsRef<Path>,
    kind: DependencyKind,
    package_spec: &str,
    options: AddDependencyOptions,
) -> Result<()> {
    let manifest_path = manifest_path.as_ref();
    let original = fs::read(manifest_path)
        .with_context(|| format!("Failed to read {}", manifest_path.display()))?;
    let mut manifest = ProjectManifest::load(manifest_path)?.document;
    let (package, supplied_requirement) = split_package_requirement(package_spec)?;
    // The remote lookup (`package`) is sent to the registry unchanged; the
    // local alias is a deterministic cross-language identifier derived from it
    // unless the user chose one explicitly. Both are validated before any
    // file, manifest, or lock is written.
    // One package occupies exactly one local alias. Look the package up first,
    // always, including when --alias was supplied: keying only on the alias
    // lets the same package be installed twice under two names, which produces
    // duplicate manifest entries, duplicate lock entries, and two generated
    // outputs for one dependency.
    //
    // The derived default alias also changed (it now lower-cases and separates
    // on non-alphanumeric runs, so `My-Program` yields `my-program` and
    // `@scope/name` yields `scope-name`, where the old default took the last
    // path segment), so a project written by an older CLI stores a key the new
    // default would not reproduce.
    let declared_alias = {
        let entries: Box<dyn Iterator<Item = (&String, &DependencyV1)>> = match kind {
            DependencyKind::Stack => Box::new(manifest.dependencies.stacks.iter()),
            DependencyKind::Program => Box::new(manifest.dependencies.programs.iter()),
        };
        entries
            .filter(|(_, entry)| match &entry.source {
                DependencySourceV1::Registry(RegistrySourceV1 { registry }) => {
                    registry.eq_ignore_ascii_case(&package)
                }
                _ => false,
            })
            .map(|(alias, _)| alias.clone())
            .next()
    };
    let alias = match (declared_alias, options.alias.as_deref()) {
        // Already declared, and the caller asked for a different local name.
        // Renaming would orphan the existing lock entry and generated output,
        // so say what is already there instead of adding a second dependency.
        (Some(declared), Some(requested)) if declared != requested => bail!(
            "arete.toml already declares {kind} '{package}' under the local alias '{declared}'; \
             remove that entry first if you want to install it as '{requested}', or re-run \
             without --alias to keep '{declared}'"
        ),
        // Already declared: keep the stored alias. It is pre-existing project
        // state, but it still has to be a legal identifier in every generated
        // language, and a project written before aliases were validated can
        // hold one that is not.
        (Some(declared), _) => {
            super::alias::validate_local_alias(&declared, kind).map_err(|error| {
                anyhow::anyhow!(
                    "arete.toml already declares {kind} '{package}' under the local alias \
                     '{declared}', which is not a portable identifier ({error}). Rename that \
                     entry to a portable alias and reinstall."
                )
            })?;
            declared
        }
        (None, explicit) => select_local_alias(kind, &package, explicit)?,
    };
    let existing = match kind {
        DependencyKind::Stack => manifest.dependencies.stacks.get(&alias),
        DependencyKind::Program => manifest.dependencies.programs.get(&alias),
    };
    if let Some(existing) = existing {
        match &existing.source {
            DependencySourceV1::Registry(RegistrySourceV1 { registry })
                if registry.eq_ignore_ascii_case(&package) => {}
            _ => bail!(
                "{kind} alias '{alias}' already names a different dependency in arete.toml; pass --alias <name> to choose another local alias for '{package}'"
            ),
        }
    }
    let requirement = match supplied_requirement {
        Some(requirement) => {
            semver::VersionReq::parse(&requirement)
                .with_context(|| format!("Invalid semantic version requirement '{requirement}'"))?;
            requirement
        }
        None => resolve_saved_requirement(kind, &alias, &package, options.exact)?,
    };
    let targets = options.target.map(|target| vec![target]);
    let mut outputs = DependencyOutputsV1::default();
    if let (Some(target), Some(output)) = (options.target, options.output.as_ref()) {
        match target {
            InstallTarget::TypeScript => outputs.typescript = Some(output.clone()),
            InstallTarget::Rust => outputs.rust = Some(output.clone()),
            InstallTarget::Python => outputs.python = Some(output.clone()),
        }
    } else if options.output.is_some() {
        bail!("--output requires exactly one of --ts, --rust, or --python");
    }
    // Installing a declared package again keeps the deployment `a4 up`
    // recorded for it.
    let endpoints = existing
        .map(|existing| existing.endpoints.clone())
        .unwrap_or_default();
    let dependency = DependencyV1 {
        source: DependencySourceV1::Registry(RegistrySourceV1 { registry: package }),
        version: Some(requirement),
        targets,
        outputs,
        endpoints,
    };
    match kind {
        DependencyKind::Stack => {
            manifest
                .dependencies
                .stacks
                .insert(alias.clone(), dependency.clone());
        }
        DependencyKind::Program => {
            manifest
                .dependencies
                .programs
                .insert(alias.clone(), dependency.clone());
        }
    }
    if let Some(package) = options.typescript_package.as_ref() {
        manifest.sdk.typescript.package = package.clone();
    }
    if options.module {
        match options.target {
            Some(InstallTarget::Rust) => manifest.sdk.rust.module_mode = true,
            Some(InstallTarget::Python) => manifest.sdk.python.module_mode = true,
            _ => bail!("--module requires --rust or --python"),
        }
    }
    manifest.validate()?;
    let replacement_manifest_hash = manifest.resolution_hash()?;
    let replacement = render_manifest_addition(
        &original,
        kind,
        &alias,
        &dependency,
        options.typescript_package.as_deref(),
        options.module.then_some(options.target).flatten(),
    )?;
    write_manifest_atomic(manifest_path, replacement.as_bytes())?;
    let result = install_project_requesting(
        manifest_path,
        InstallOptions {
            allow_outside_project: options.allow_outside_project,
            ..InstallOptions::default()
        },
        &[(kind, alias.clone())],
    );
    if result.is_err() {
        let install_committed =
            ProjectLock::load_optional(manifest_path.with_file_name("arete.lock"))
                .ok()
                .flatten()
                .is_some_and(|lock| lock.is_fresh(&replacement_manifest_hash));
        if !install_committed {
            write_manifest_atomic(manifest_path, &original)?;
        }
    }
    result
}

pub fn remove_and_install(
    manifest_path: impl AsRef<Path>,
    kind: DependencyKind,
    alias: &str,
    options: RemoveDependencyOptions,
) -> Result<()> {
    let manifest_path = manifest_path.as_ref();
    let original = fs::read(manifest_path)
        .with_context(|| format!("Failed to read {}", manifest_path.display()))?;
    let mut existing = ProjectManifest::load(manifest_path)?;
    recover_interrupted_install(&existing.root)?;
    existing = ProjectManifest::load(manifest_path)?;
    if existing.dependency(kind, alias).is_none() {
        bail!("No {kind} dependency named '{alias}'");
    }

    let old_plan = InstallPlan::build(&existing, options.allow_outside_project)?;
    let removals = if options.keep_output {
        Vec::new()
    } else {
        old_plan
            .for_dependency(kind, alias)
            .map(|output| RemovalOutput {
                final_path: output.path.clone(),
                kind,
                alias: alias.to_string(),
                target: output.target,
            })
            .collect()
    };
    for output in &removals {
        if output.final_path.exists() {
            reject_unowned_files(&output.final_path)?;
            validate_project_output_ownership(output)?;
        }
    }

    match kind {
        DependencyKind::Stack => {
            existing.document.dependencies.stacks.remove(alias);
        }
        DependencyKind::Program => {
            existing.document.dependencies.programs.remove(alias);
        }
    }
    existing.document.validate()?;
    let replacement_manifest_hash = existing.document.resolution_hash()?;
    let replacement = render_manifest_removal(&original, kind, alias)?;
    write_manifest_atomic(manifest_path, replacement.as_bytes())?;

    let result = ProjectManifest::load(manifest_path).and_then(|manifest| {
        install_loaded_project(
            manifest,
            InstallOptions {
                allow_outside_project: options.allow_outside_project,
                ..InstallOptions::default()
            },
            removals,
            &[],
        )
    });
    if result.is_err() {
        let install_committed =
            ProjectLock::load_optional(manifest_path.with_file_name("arete.lock"))
                .ok()
                .flatten()
                .is_some_and(|lock| lock.is_fresh(&replacement_manifest_hash));
        if !install_committed {
            write_manifest_atomic(manifest_path, &original)?;
        }
    }
    result?;
    status_line(format!("Removed {kind} dependency '{alias}'"));
    Ok(())
}

fn render_manifest_addition(
    original: &[u8],
    kind: DependencyKind,
    alias: &str,
    dependency: &DependencyV1,
    typescript_package: Option<&str>,
    module_target: Option<InstallTarget>,
) -> Result<String> {
    let mut document = parse_editable_manifest(original)?;
    let kind_key = match kind {
        DependencyKind::Stack => "stacks",
        DependencyKind::Program => "programs",
    };
    insert_manifest_item(
        document.as_item_mut(),
        &["dependencies", kind_key, alias],
        dependency_manifest_item(dependency),
    )?;
    if let Some(package) = typescript_package {
        insert_manifest_item(
            document.as_item_mut(),
            &["sdk", "typescript", "package"],
            value(package),
        )?;
    }
    match module_target {
        Some(InstallTarget::Rust) => insert_manifest_item(
            document.as_item_mut(),
            &["sdk", "rust", "module_mode"],
            value(true),
        )?,
        Some(InstallTarget::Python) => insert_manifest_item(
            document.as_item_mut(),
            &["sdk", "python", "module_mode"],
            value(true),
        )?,
        Some(InstallTarget::TypeScript) | None => {}
    }
    Ok(document.to_string())
}

/// Writes `[authoring.stacks.<name>]` as the composition `entry`, keeping
/// the rest of arete.toml (and any other keys of the entry, such as
/// `deployment_name`) as written. With `dependency`, also declares
/// `[dependencies.stacks.<dependency>] source = { workspace = "<name>" }`
/// when no stack dependency reads the composition yet, and installs; a
/// failed install restores arete.toml.
pub(crate) fn save_composition(
    manifest_path: &Path,
    name: &str,
    entry: &super::manifest::AuthoringStackV1,
    dependency: Option<&str>,
) -> Result<Option<String>> {
    let original = fs::read(manifest_path)
        .with_context(|| format!("Failed to read {}", manifest_path.display()))?;
    let loaded = ProjectManifest::load(manifest_path)?;
    let mut manifest = loaded.document;
    if manifest
        .authoring
        .stacks
        .get(name)
        .is_some_and(|existing| !existing.is_composed())
    {
        bail!(
            "[authoring.stacks.{name}] already names a StackManifest file; remove it or compose under another --name"
        );
    }
    let mut document = parse_editable_manifest(&original)?;
    render_composition(&mut document, name, entry)?;
    let merged = manifest
        .authoring
        .stacks
        .entry(name.to_string())
        .or_insert_with(|| entry.clone());
    merged.live = entry.live.clone();
    merged.programs = entry.programs.clone();

    let declared = manifest
        .dependencies
        .stacks
        .iter()
        .find(|(_, candidate)| {
            matches!(&candidate.source, DependencySourceV1::Workspace(WorkspaceSourceV1 { workspace }) if workspace == name)
        })
        .map(|(alias, _)| alias.clone());
    let added = match (dependency, declared) {
        (Some(alias), None) => {
            if manifest.dependencies.stacks.contains_key(alias) {
                bail!(
                    "arete.toml already declares stack '{alias}' from another source; declare `source = {{ workspace = \"{name}\" }}` under another alias and run `a4 install`"
                );
            }
            super::alias::validate_local_alias(alias, DependencyKind::Stack)?;
            let workspace = DependencyV1 {
                source: DependencySourceV1::Workspace(WorkspaceSourceV1 {
                    workspace: name.to_string(),
                }),
                version: None,
                targets: None,
                outputs: DependencyOutputsV1::default(),
                endpoints: BTreeMap::new(),
            };
            insert_manifest_item(
                document.as_item_mut(),
                &["dependencies", "stacks", alias],
                dependency_manifest_item(&workspace),
            )?;
            manifest
                .dependencies
                .stacks
                .insert(alias.to_string(), workspace);
            Some(alias.to_string())
        }
        (_, declared) => declared,
    };
    manifest.validate()?;
    let replacement = document.to_string();
    // What was rendered must read back as what was validated.
    let reparsed: ManifestV1 =
        toml::from_str(&replacement).context("The composed arete.toml does not parse")?;
    reparsed.validate()?;
    if reparsed.resolution_hash()? != manifest.resolution_hash()? {
        bail!("The composed arete.toml does not read back as composed; nothing was written");
    }
    write_manifest_atomic(manifest_path, replacement.as_bytes())?;
    let Some(alias) = dependency.and(added.clone()) else {
        return Ok(added);
    };
    let replacement_manifest_hash = manifest.resolution_hash()?;
    let result = install_project_requesting(
        manifest_path,
        InstallOptions::default(),
        &[(DependencyKind::Stack, alias)],
    );
    if result.is_err() {
        let install_committed =
            ProjectLock::load_optional(manifest_path.with_file_name("arete.lock"))
                .ok()
                .flatten()
                .is_some_and(|lock| lock.is_fresh(&replacement_manifest_hash));
        if !install_committed {
            write_manifest_atomic(manifest_path, &original)?;
        }
    }
    result.map(|()| added)
}

/// Replaces the `live` and `programs` of `[authoring.stacks.<name>]`:
///
/// ```toml
/// [authoring.stacks.ore-plus-token]
/// live.ore = { stack = "ore", version = "^1.0.0", views = ["OreRound/latest"] }
/// programs = [{ package = "spl-token", version = "^4.0.0" }]
/// ```
fn render_composition(
    document: &mut DocumentMut,
    name: &str,
    entry: &super::manifest::AuthoringStackV1,
) -> Result<()> {
    let exists = document
        .get("authoring")
        .and_then(|authoring| authoring.get("stacks"))
        .and_then(|stacks| stacks.get(name))
        .is_some();
    if !exists {
        insert_manifest_item(
            document.as_item_mut(),
            &["authoring", "stacks", name],
            Item::Table(Table::new()),
        )?;
    }
    let item = document
        .get_mut("authoring")
        .and_then(|authoring| authoring.get_mut("stacks"))
        .and_then(|stacks| stacks.get_mut(name))
        .ok_or_else(|| anyhow::anyhow!("Failed to edit [authoring.stacks.{name}]"))?;
    if let Some(inline) = item.as_inline_table().cloned() {
        *item = Item::Table(inline.into_table());
    }
    let table = item
        .as_table_mut()
        .ok_or_else(|| anyhow::anyhow!("[authoring.stacks.{name}] is not a table"))?;
    table.remove("live");
    table.remove("programs");

    let mut live = Table::new();
    live.set_dotted(true);
    for (alias, part) in &entry.live {
        let mut inline = InlineTable::new();
        for (key, value) in [
            ("stack", &part.stack),
            ("version", &part.version),
            ("live_alias", &part.live_alias),
            ("path", &part.path),
        ] {
            if let Some(value) = value {
                inline.insert(key, value.clone().into());
            }
        }
        if let Some(views) = &part.views {
            let mut array: Array = views.iter().map(String::as_str).collect();
            array.fmt();
            inline.insert("views", array.into());
        }
        inline.fmt();
        live.insert(alias, value(inline));
    }
    table.insert("live", Item::Table(live));
    if !entry.programs.is_empty() {
        let mut programs = Array::new();
        for program in &entry.programs {
            let mut inline = InlineTable::new();
            for (key, value) in [
                ("package", &program.package),
                ("version", &program.version),
                ("path", &program.path),
            ] {
                if let Some(value) = value {
                    inline.insert(key, value.clone().into());
                }
            }
            inline.fmt();
            programs.push(inline);
        }
        programs.fmt();
        table.insert("programs", value(programs));
    }
    Ok(())
}

fn render_manifest_removal(original: &[u8], kind: DependencyKind, alias: &str) -> Result<String> {
    let mut document = parse_editable_manifest(original)?;
    let kind_key = match kind {
        DependencyKind::Stack => "stacks",
        DependencyKind::Program => "programs",
    };
    remove_manifest_item(document.as_item_mut(), &["dependencies", kind_key, alias])?;
    Ok(document.to_string())
}

fn parse_editable_manifest(contents: &[u8]) -> Result<DocumentMut> {
    let source = std::str::from_utf8(contents).context("Project manifest is not UTF-8")?;
    source
        .parse::<DocumentMut>()
        .context("Failed to parse project manifest for editing")
}

fn insert_manifest_item(current: &mut Item, path: &[&str], item: Item) -> Result<()> {
    let (key, remaining) = path
        .split_first()
        .ok_or_else(|| anyhow::anyhow!("Cannot insert an empty manifest path"))?;
    let table = current
        .as_table_like_mut()
        .ok_or_else(|| anyhow::anyhow!("Manifest path parent is not a table"))?;
    if remaining.is_empty() {
        table.insert(key, item);
        return Ok(());
    }
    if !table.contains_key(key) {
        let mut child = Table::new();
        child.set_implicit(true);
        table.insert(key, Item::Table(child));
    }
    let child = table
        .get_mut(key)
        .ok_or_else(|| anyhow::anyhow!("Failed to create manifest table '{key}'"))?;
    insert_manifest_item(child, remaining, item)
}

fn remove_manifest_item(current: &mut Item, path: &[&str]) -> Result<bool> {
    let (key, remaining) = path
        .split_first()
        .ok_or_else(|| anyhow::anyhow!("Cannot remove an empty manifest path"))?;
    let table = current
        .as_table_like_mut()
        .ok_or_else(|| anyhow::anyhow!("Manifest path parent is not a table"))?;
    if remaining.is_empty() {
        table
            .remove(key)
            .ok_or_else(|| anyhow::anyhow!("Manifest entry '{key}' disappeared while editing"))?;
    } else {
        let child_is_empty = {
            let child = table.get_mut(key).ok_or_else(|| {
                anyhow::anyhow!("Manifest table '{key}' disappeared while editing")
            })?;
            remove_manifest_item(child, remaining)?
        };
        if child_is_empty {
            table.remove(key);
        }
    }
    Ok(table.is_empty())
}

fn dependency_manifest_item(dependency: &DependencyV1) -> Item {
    let mut table = Table::new();
    let mut source = InlineTable::new();
    match &dependency.source {
        DependencySourceV1::Registry(RegistrySourceV1 { registry }) => {
            source.insert("registry", registry.clone().into());
        }
        DependencySourceV1::Path(PathSourceV1 { path }) => {
            source.insert("path", path.clone().into());
        }
        DependencySourceV1::Workspace(WorkspaceSourceV1 { workspace }) => {
            source.insert("workspace", workspace.clone().into());
        }
    }
    source.fmt();
    table.insert("source", value(source));
    if let Some(version) = dependency.version.as_ref() {
        table.insert("version", value(version.clone()));
    }
    if let Some(targets) = dependency.targets.as_ref() {
        let mut targets: Array = targets.iter().map(|target| target.as_str()).collect();
        targets.fmt();
        table.insert("targets", value(targets));
    }
    let mut outputs = InlineTable::new();
    if let Some(output) = dependency.outputs.typescript.as_ref() {
        outputs.insert("typescript", output.clone().into());
    }
    if let Some(output) = dependency.outputs.rust.as_ref() {
        outputs.insert("rust", output.clone().into());
    }
    if let Some(output) = dependency.outputs.python.as_ref() {
        outputs.insert("python", output.clone().into());
    }
    if !outputs.is_empty() {
        outputs.fmt();
        table.insert("outputs", value(outputs));
    }
    if !dependency.endpoints.is_empty() {
        table.insert("endpoints", endpoints_manifest_item(&dependency.endpoints));
    }
    Item::Table(table)
}

/// `endpoints = { <live> = { websocket = "...", query = "..." } }`, inline so
/// it fits a dependency written as a table or as an inline table.
fn endpoints_manifest_item(endpoints: &BTreeMap<String, StackEndpointsV1>) -> Item {
    let mut table = InlineTable::new();
    for (live, endpoint) in endpoints {
        let mut entry = InlineTable::new();
        entry.insert("websocket", endpoint.websocket.clone().into());
        entry.insert("query", endpoint.query.clone().into());
        entry.fmt();
        table.insert(live, entry.into());
    }
    table.fmt();
    value(table)
}

/// arete.toml with one stack dependency's `endpoints` replaced, leaving every
/// other byte as written.
fn render_stack_endpoints(
    original: &[u8],
    alias: &str,
    endpoints: &BTreeMap<String, StackEndpointsV1>,
) -> Result<String> {
    let mut document = parse_editable_manifest(original)?;
    let path = ["dependencies", "stacks", alias, "endpoints"];
    if endpoints.is_empty() {
        let dependency = document
            .as_item_mut()
            .get_mut("dependencies")
            .and_then(|item| item.get_mut("stacks"))
            .and_then(|item| item.get_mut(alias))
            .and_then(Item::as_table_like_mut);
        if let Some(dependency) = dependency {
            dependency.remove("endpoints");
        }
    } else {
        insert_manifest_item(
            document.as_item_mut(),
            &path,
            endpoints_manifest_item(endpoints),
        )?;
    }
    Ok(document.to_string())
}

/// Points an installed stack's generated SDK at the user's own deployment:
/// records `endpoints` for it in arete.toml and reinstalls, restoring
/// arete.toml if the install does not commit. Returns false when arete.toml
/// already records these endpoints and arete.lock is fresh.
pub(crate) fn record_stack_endpoints_and_install(
    manifest_path: impl AsRef<Path>,
    alias: &str,
    endpoints: BTreeMap<String, StackEndpointsV1>,
) -> Result<bool> {
    let manifest_path = manifest_path.as_ref();
    let original = fs::read(manifest_path)
        .with_context(|| format!("Failed to read {}", manifest_path.display()))?;
    let loaded = ProjectManifest::load(manifest_path)?;
    let mut manifest = loaded.document;
    let dependency = manifest
        .dependencies
        .stacks
        .get_mut(alias)
        .ok_or_else(|| anyhow::anyhow!("arete.toml declares no stack '{alias}'"))?;
    let lock_fresh = ProjectLock::load_optional(loaded.root.join("arete.lock"))?
        .is_some_and(|lock| lock.is_fresh(&loaded.manifest_hash));
    if dependency.endpoints == endpoints && lock_fresh {
        return Ok(false);
    }
    dependency.endpoints = endpoints.clone();
    manifest.validate()?;
    let replacement_manifest_hash = manifest.resolution_hash()?;
    let replacement = render_stack_endpoints(&original, alias, &endpoints)?;
    write_manifest_atomic(manifest_path, replacement.as_bytes())?;
    let result = install_project(manifest_path, InstallOptions::default());
    if result.is_err() {
        let install_committed =
            ProjectLock::load_optional(manifest_path.with_file_name("arete.lock"))
                .ok()
                .flatten()
                .is_some_and(|lock| lock.is_fresh(&replacement_manifest_hash));
        if !install_committed {
            write_manifest_atomic(manifest_path, &original)?;
        }
    }
    result.map(|()| true)
}

fn split_package_requirement(value: &str) -> Result<(String, Option<String>)> {
    if value.trim().is_empty() {
        bail!("Package name cannot be empty");
    }
    if let Some(position) = value.rfind('@').filter(|position| *position > 0) {
        let package = value[..position].to_string();
        let requirement = value[position + 1..].to_string();
        if requirement.is_empty() {
            bail!("Package requirement after '@' cannot be empty");
        }
        Ok((package, Some(requirement)))
    } else {
        Ok((value.to_string(), None))
    }
}

fn resolve_saved_requirement(
    kind: DependencyKind,
    alias: &str,
    package: &str,
    exact: bool,
) -> Result<String> {
    let request = RegistryResolveRequest {
        manifest_version: 1,
        dependencies: vec![RegistryDependencyRequest {
            kind,
            alias: alias.into(),
            package: package.into(),
            requirement: "*".into(),
            locked_package_release_hash: None,
            locked_programs: Vec::new(),
        }],
        targets: vec![InstallTarget::TypeScript],
        generator_contract: GENERATOR_CONTRACT.into(),
    };
    let response = ApiClient::new()?
        .resolve_registry_dependencies(&request)
        .map_err(|error| describe_resolver_error(error, kind, package, false))?;
    if response.resolver_contract != RESOLVER_CONTRACT || response.dependencies.len() != 1 {
        bail!("Registry returned an invalid single-package resolver response");
    }
    let resolved = &response.dependencies[0];
    if resolved.alias() != alias || resolved.package() != package {
        bail!("Registry response did not match requested package '{package}'");
    }
    verify_resolved_kind_and_contract(kind, resolved)?;
    verify_resolved_extensions(resolved, &[InstallTarget::TypeScript])?;
    verify_resolved_release_identity(resolved)?;
    verify_resolved_stack_delivery(resolved)?;
    let version = match resolved {
        ResolvedRegistryDependency::Stack { version, .. }
        | ResolvedRegistryDependency::Program { version, .. } => version,
    };
    let version = semver::Version::parse(version)?;
    Ok(if exact {
        format!("={version}")
    } else {
        format!("^{version}")
    })
}

fn write_manifest_atomic(path: &Path, contents: &[u8]) -> Result<()> {
    let temporary = path.with_extension(format!("toml.{}.tmp", uuid::Uuid::new_v4()));
    fs::write(&temporary, contents)?;
    fs::rename(&temporary, path)?;
    Ok(())
}

pub fn validate_project(
    manifest_path: impl AsRef<Path>,
    allow_outside_project: bool,
) -> Result<(ProjectManifest, InstallPlan, Option<ProjectLock>)> {
    let manifest = ProjectManifest::load(manifest_path)?;
    let plan = InstallPlan::build(&manifest, allow_outside_project)?;
    validate_local_closure(&manifest)?;
    let lock = ProjectLock::load_optional(manifest.root.join("arete.lock"))?;
    Ok((manifest, plan, lock))
}

pub fn install_project(manifest_path: impl AsRef<Path>, options: InstallOptions<'_>) -> Result<()> {
    install_project_requesting(manifest_path, options, &[])
}

/// Install the project, reporting `requested` as what this invocation asked
/// for; every other dependency is reported as regenerated.
fn install_project_requesting(
    manifest_path: impl AsRef<Path>,
    options: InstallOptions<'_>,
    requested: &[(DependencyKind, String)],
) -> Result<()> {
    let manifest = ProjectManifest::load(manifest_path)?;
    install_loaded_project(manifest, options, Vec::new(), requested)
}

fn install_loaded_project(
    manifest: ProjectManifest,
    options: InstallOptions<'_>,
    removals: Vec<RemovalOutput>,
    requested: &[(DependencyKind, String)],
) -> Result<()> {
    recover_interrupted_install(&manifest.root)?;
    let plan = InstallPlan::build(&manifest, options.allow_outside_project)?;
    let lock_path = manifest.root.join("arete.lock");
    let previous_lock = ProjectLock::load_optional(&lock_path)?;
    if options.locked {
        let lock = previous_lock.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "--locked requires committed lockfile {}",
                lock_path.display()
            )
        })?;
        if !lock.is_fresh(&manifest.manifest_hash) {
            bail!(
                "arete.lock is stale for {}; --locked changed nothing",
                manifest.path.display()
            );
        }
    }
    validate_local_closure(&manifest)?;
    if options.dry_run {
        print_plan(&plan, previous_lock.as_ref(), &manifest);
        return Ok(());
    }
    if let Some(selection) = options.update {
        validate_update_selection(&manifest, selection)?;
    }

    let resolved = resolve_dependencies(&manifest, previous_lock.as_ref(), options.update)?;
    let prospective_lock = build_lock(&manifest, &resolved)?;
    // `--locked` never rewrites arete.lock: it installs exactly what the lock
    // pins, which for a lock written before program SDK identities were
    // resolved leaves only the program package releases implied.
    let prospective_lock = match previous_lock.as_ref() {
        Some(previous) if options.locked => {
            check_locked_resolution(previous, &prospective_lock, &resolved)?;
            previous.clone()
        }
        _ => prospective_lock,
    };

    let staging_root = manifest
        .root
        .join(".arete")
        .join(format!("install-staging-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&staging_root)?;
    let staged = match generate_all(&manifest, &plan, &resolved, &staging_root) {
        Ok(staged) => staged,
        Err(error) => {
            let _ = fs::remove_dir_all(&staging_root);
            return Err(error);
        }
    };
    commit_install(
        &manifest.root,
        &lock_path,
        &prospective_lock,
        staged,
        removals,
        &staging_root,
    )?;
    // `a4 up <name>` deploys a composed stack from these, checked against
    // the lock just written.
    for dependency in &resolved {
        if let ResolvedProjectDependency::ComposedStack { composed, .. } = dependency {
            composition::write_composition_artifacts(&manifest.root, composed)?;
        }
    }
    let mut requested = requested.to_vec();
    if let Some(selection) = options.update {
        requested.extend(updated_dependencies(&manifest, selection));
    }
    let report = InstallReport {
        lockfile: lock_path.display().to_string(),
        installed: prospective_lock.dependencies.len(),
        requested: Vec::new(),
        regenerated: Vec::new(),
        shared: shared_program_sdks(&resolved),
        auth: Vec::new(),
        runtime: runtime::typescript_runtime_for_outputs(
            plan.outputs
                .iter()
                .filter(|output| output.target == InstallTarget::TypeScript)
                .map(|output| output.path.as_path()),
        ),
        notes: redeploy_notes(&manifest, previous_lock.as_ref(), &prospective_lock)
            .into_iter()
            .chain(composition_notes(&resolved))
            .collect(),
    }
    .with_dependencies(&requested, previous_lock.as_ref(), &prospective_lock)
    .with_auth(&requested, &resolved);
    report.emit()
}

/// What each composed stack's resolution says: explicit program SDKs that
/// replace a source stack's, and live views without hosted delivery.
fn composition_notes(resolved: &[ResolvedProjectDependency]) -> Vec<String> {
    let mut notes = Vec::new();
    for dependency in resolved {
        if let ResolvedProjectDependency::ComposedStack { composed, .. } = dependency {
            for note in &composed.notes {
                if !notes.contains(note) {
                    notes.push(note.clone());
                }
            }
        }
    }
    notes
}

/// The registry dependencies an `a4 update` selection advanced.
fn updated_dependencies(
    manifest: &ProjectManifest,
    selection: UpdateSelection<'_>,
) -> Vec<(DependencyKind, String)> {
    manifest
        .dependencies()
        .filter(|(kind, alias, dependency)| {
            updatable(manifest, *kind, dependency)
                && selection.kind.is_none_or(|selected| selected == *kind)
                && selection
                    .alias
                    .is_none_or(|selected| selected == alias.as_str())
        })
        .map(|(kind, alias, _)| (kind, alias.clone()))
        .collect()
}

/// What one install did: the human summary and the `--json` result.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct InstallReport {
    lockfile: String,
    installed: usize,
    /// The dependencies this invocation asked for.
    requested: Vec<ReportedDependency>,
    /// Every other dependency: installs regenerate the whole project.
    regenerated: Vec<RegeneratedDependency>,
    /// Standalone program dependencies a stack dependency also provides.
    shared: Vec<SharedProgramSdk>,
    /// Account requirements of the hosted stacks this install reports on.
    auth: Vec<StackAuthRequirements>,
    /// The packages the generated TypeScript needs at run time, at the CLI's
    /// lockstep version. Printed, never written to package.json.
    runtime: Vec<runtime::RuntimePackage>,
    notes: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReportedDependency {
    kind: DependencyKind,
    alias: String,
    source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    package_release_hash: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RegeneratedDependency {
    kind: DependencyKind,
    alias: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<String>,
    reason: String,
}

/// A program installed on its own that a stack dependency also provides.
/// The same program package release generates the same program SDK in both
/// outputs; a different release (or a stack that embeds only the core
/// program) generates a different one, and both stay installed.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SharedProgramSdk {
    program: String,
    program_id: String,
    version: String,
    package_release_hash: String,
    stack: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    stack_program_package: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stack_program_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stack_package_release_hash: Option<String>,
    same_release: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct StackAuthRequirements {
    stack: String,
    live_views: Vec<LiveViewAuth>,
    #[serde(skip_serializing_if = "Option::is_none")]
    chain: Option<CapabilityAuth>,
    #[serde(skip_serializing_if = "Option::is_none")]
    transactions: Option<CapabilityAuth>,
    browser: &'static str,
    readiness: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LiveViewAuth {
    alias: String,
    websocket_auth_policy: String,
    query_auth_policy: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CapabilityAuth {
    required: bool,
    mode: String,
    scopes: Vec<String>,
    accepted_key_classes: Vec<String>,
    transaction_entitlement_required: bool,
}

const BROWSER_KEY_REQUIREMENT: &str =
    "Browser apps need a publishable key bound to the app's origin.";
const READINESS_CHECK: &str = "Run `a4 doctor` to check this account's readiness.";

impl InstallReport {
    fn with_dependencies(
        mut self,
        requested: &[(DependencyKind, String)],
        previous: Option<&ProjectLock>,
        next: &ProjectLock,
    ) -> Self {
        let is_requested = |entry: &LockedDependency| {
            requested
                .iter()
                .any(|(kind, alias)| *kind == entry.kind && *alias == entry.alias)
        };
        for entry in &next.dependencies {
            if is_requested(entry) {
                self.requested.push(ReportedDependency {
                    kind: entry.kind,
                    alias: entry.alias.clone(),
                    source: entry.source.clone(),
                    version: entry.version.clone(),
                    package_release_hash: entry.package_release_hash.clone(),
                });
                continue;
            }
            let before = previous.and_then(|lock| {
                lock.dependencies.iter().find(|candidate| {
                    candidate.kind == entry.kind && candidate.alias == entry.alias
                })
            });
            self.regenerated.push(RegeneratedDependency {
                kind: entry.kind,
                alias: entry.alias.clone(),
                version: entry.version.clone(),
                reason: regeneration_reason(before, entry),
            });
        }
        self
    }

    /// Auth requirements of the requested hosted stacks, or of every hosted
    /// stack when nothing in particular was requested.
    fn with_auth(
        mut self,
        requested: &[(DependencyKind, String)],
        resolved: &[ResolvedProjectDependency],
    ) -> Self {
        let reported = |alias: &str| {
            requested.is_empty()
                || requested
                    .iter()
                    .any(|(kind, requested)| *kind == DependencyKind::Stack && requested == alias)
        };
        for dependency in resolved {
            if let ResolvedProjectDependency::ComposedStack {
                alias, composed, ..
            } = dependency
            {
                if reported(alias) {
                    if let Some(requirements) = composed_auth_requirements(alias, composed) {
                        self.auth.push(requirements);
                    }
                }
                continue;
            }
            let ResolvedProjectDependency::Registry { resolved, .. } = dependency else {
                continue;
            };
            let ResolvedRegistryDependency::Stack {
                alias,
                delivery: Some(delivery),
                ..
            } = resolved.as_ref()
            else {
                continue;
            };
            if !reported(alias) {
                continue;
            }
            if let Some(requirements) = stack_auth_requirements(alias, delivery) {
                self.auth.push(requirements);
            }
        }
        self
    }

    fn emit(&self) -> Result<()> {
        if json_output() {
            println!("{}", serde_json::to_string_pretty(self)?);
            return Ok(());
        }
        println!(
            "Installed {} dependencies and wrote {}",
            self.installed, self.lockfile
        );
        for dependency in &self.requested {
            println!(
                "Requested:   {}",
                describe_dependency(
                    dependency.kind,
                    &dependency.alias,
                    dependency.version.as_deref(),
                    &dependency.source
                )
            );
        }
        for dependency in &self.regenerated {
            let version = dependency
                .version
                .as_deref()
                .map(|version| format!("@{version}"))
                .unwrap_or_default();
            println!(
                "Regenerated: {} {}{version} ({})",
                dependency.kind, dependency.alias, dependency.reason
            );
        }
        for shared in &self.shared {
            println!("{}", describe_shared(shared));
        }
        if !self.runtime.is_empty() {
            println!(
                "Runtime:     the generated TypeScript needs these packages (a4 does not change package.json):"
            );
            println!(
                "             {}",
                runtime::npm_install_command(&self.runtime)
            );
        }
        for note in &self.notes {
            println!("{note}");
        }
        for auth in &self.auth {
            print_auth_requirements(auth);
        }
        Ok(())
    }
}

fn describe_dependency(
    kind: DependencyKind,
    alias: &str,
    version: Option<&str>,
    source: &str,
) -> String {
    match version {
        Some(version) => format!("{kind} {alias}@{version}"),
        None => format!("{kind} {alias} ({source})"),
    }
}

fn regeneration_reason(previous: Option<&LockedDependency>, next: &LockedDependency) -> String {
    let Some(previous) = previous else {
        return "newly locked".into();
    };
    if previous == next {
        return "inputs unchanged".into();
    }
    if previous.package_release_hash != next.package_release_hash {
        return match (&previous.version, &next.version) {
            (Some(before), Some(after)) if before != after => {
                format!("resolved {after}, was {before}")
            }
            _ => "resolved a different release".into(),
        };
    }
    if previous.source != next.source {
        return "source changed".into();
    }
    if previous.targets != next.targets {
        return "targets changed".into();
    }
    if previous.programs != next.programs {
        return "program SDK releases changed".into();
    }
    "resolved inputs changed".into()
}

fn describe_shared(shared: &SharedProgramSdk) -> String {
    let program = format!("program {}@{}", shared.program, shared.version);
    if shared.same_release {
        return format!(
            "Shared:      {program} is also provided by stack {} (the same program SDK release, generated identically in both)",
            shared.stack
        );
    }
    match (&shared.stack_program_package, &shared.stack_program_version) {
        (Some(package), Some(version)) => format!(
            "Note:        stack {} provides program {package}@{version}, a different release than {program}; both are generated and stay separate",
            shared.stack
        ),
        _ => format!(
            "Note:        stack {} embeds its own core SDK for program {}, not a program SDK release; {program} is generated separately",
            shared.stack, shared.program_id
        ),
    }
}

/// Every standalone registry program that a registry stack in the same
/// install also provides, matched by on-chain program.
fn shared_program_sdks(resolved: &[ResolvedProjectDependency]) -> Vec<SharedProgramSdk> {
    let registry = resolved
        .iter()
        .filter_map(|dependency| match dependency {
            ResolvedProjectDependency::Registry { resolved, .. } => Some(resolved.as_ref()),
            _ => None,
        })
        .collect::<Vec<_>>();
    let mut shared = Vec::new();
    for dependency in &registry {
        let ResolvedRegistryDependency::Program {
            alias,
            version,
            package_release_hash,
            install,
            ..
        } = dependency
        else {
            continue;
        };
        for stack in &registry {
            let ResolvedRegistryDependency::Stack {
                alias: stack_alias,
                programs,
                ..
            } = stack
            else {
                continue;
            };
            for embedded in programs
                .iter()
                .filter(|embedded| embedded.definition.program_id == install.definition.program_id)
            {
                let reference = embedded.program_package.as_ref();
                shared.push(SharedProgramSdk {
                    program: alias.clone(),
                    program_id: install.definition.program_id.clone(),
                    version: version.clone(),
                    package_release_hash: package_release_hash.clone(),
                    stack: stack_alias.clone(),
                    stack_program_package: reference.map(|reference| reference.package.clone()),
                    stack_program_version: reference.map(|reference| reference.version.clone()),
                    stack_package_release_hash: reference
                        .map(|reference| reference.package_release_hash.clone()),
                    same_release: reference.is_some_and(|reference| {
                        &reference.package_release_hash == package_release_hash
                    }),
                });
            }
        }
    }
    shared
}

fn capability_auth(
    binding: &crate::api_client::RegistryCapabilityInstallBinding,
) -> CapabilityAuth {
    CapabilityAuth {
        required: binding.auth.required,
        mode: binding.auth.mode.clone(),
        scopes: binding.auth.scopes.clone(),
        accepted_key_classes: binding.auth.accepted_key_classes.clone(),
        transaction_entitlement_required: binding.auth.transaction_entitlement_required,
    }
}

/// A hosted stack's account requirements, from its install descriptor. A
/// definition-only stack is served by whoever deploys it, so it has none.
fn stack_auth_requirements(
    alias: &str,
    delivery: &super::resolver::ResolvedStackDelivery,
) -> Option<StackAuthRequirements> {
    let super::resolver::ResolvedStackDelivery::Hosted {
        live_bindings,
        chain_binding,
        transaction_binding,
        ..
    } = delivery
    else {
        return None;
    };
    Some(StackAuthRequirements {
        stack: alias.to_string(),
        live_views: live_bindings
            .iter()
            .map(|live| LiveViewAuth {
                alias: live.alias.clone(),
                websocket_auth_policy: live.binding.websocket_auth_policy.clone(),
                query_auth_policy: live.binding.query_auth_policy.clone(),
            })
            .collect(),
        chain: chain_binding.as_deref().map(capability_auth),
        transactions: transaction_binding.as_deref().map(capability_auth),
        browser: BROWSER_KEY_REQUIREMENT,
        readiness: READINESS_CHECK,
    })
}

/// A composed stack's account requirements: those of the hosted deployments
/// its live aliases read, and of the managed gateway they share. With no
/// hosted alias it is served by whoever deploys it, so it has none.
fn composed_auth_requirements(
    alias: &str,
    composed: &ComposedStack,
) -> Option<StackAuthRequirements> {
    if composed.hosted.is_empty() {
        return None;
    }
    Some(StackAuthRequirements {
        stack: alias.to_string(),
        live_views: composed
            .hosted
            .iter()
            .map(|live| LiveViewAuth {
                alias: live.descriptor.alias.clone(),
                websocket_auth_policy: live.descriptor.binding.websocket_auth_policy.clone(),
                query_auth_policy: live.descriptor.binding.query_auth_policy.clone(),
            })
            .collect(),
        chain: composed.chain_binding.as_ref().map(capability_auth),
        transactions: composed.transaction_binding.as_ref().map(capability_auth),
        browser: BROWSER_KEY_REQUIREMENT,
        readiness: READINESS_CHECK,
    })
}

fn print_auth_requirements(auth: &StackAuthRequirements) {
    println!("Auth for stack {}:", auth.stack);
    let policies = auth
        .live_views
        .iter()
        .flat_map(|live| [&live.websocket_auth_policy, &live.query_auth_policy])
        .collect::<BTreeSet<_>>();
    if !policies.is_empty() {
        println!(
            "  Live views: {}",
            policies
                .into_iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    for (label, capability) in [
        ("Chain reads", &auth.chain),
        ("Transactions", &auth.transactions),
    ] {
        let Some(capability) = capability else {
            continue;
        };
        let mut line = format!(
            "  {label}: {} keys; scopes {}",
            capability.accepted_key_classes.join(", "),
            capability.scopes.join(", ")
        );
        if capability.transaction_entitlement_required {
            line.push_str("; the account needs the transaction entitlement");
        }
        println!("{line}");
    }
    println!("  {}", auth.browser);
    println!("  {}", auth.readiness);
}

/// A stack whose recorded deployment (`endpoints`) now resolves to a
/// different StackManifest: the deployment still serves the previous one
/// until `a4 up <alias>` redeploys it. Its endpoints stay valid, since the
/// deployment keeps its name.
fn redeploy_notes(
    manifest: &ProjectManifest,
    previous: Option<&ProjectLock>,
    next: &ProjectLock,
) -> Vec<String> {
    let Some(previous) = previous else {
        return Vec::new();
    };
    let stack_manifest = |lock: &ProjectLock, alias: &str| {
        lock.dependencies
            .iter()
            .find(|entry| entry.kind == DependencyKind::Stack && entry.alias == alias)
            .and_then(|entry| entry.stack_manifest_hash.clone())
    };
    manifest
        .document
        .dependencies
        .stacks
        .iter()
        .filter(|(_, dependency)| !dependency.endpoints.is_empty())
        .filter_map(|(alias, dependency)| {
            let before = stack_manifest(previous, alias)?;
            let after = stack_manifest(next, alias)?;
            (before != after).then(|| {
                // `a4 up <name>` prefers an [authoring.stacks] entry of the
                // same name, so that command would not redeploy this stack,
                // unless the entry is the composed stack it installs.
                let redeploy = match composed_workspace(manifest, DependencyKind::Stack, dependency)
                {
                    Some(workspace) => format!("Run `a4 up {workspace}` to redeploy it"),
                    None if manifest.document.authoring.stacks.contains_key(alias) => format!(
                        "Rename it or the [authoring.stacks] entry '{alias}' (which `a4 up {alias}` deploys instead), then redeploy it"
                    ),
                    None => format!("Run `a4 up {alias}` to redeploy it"),
                };
                format!(
                    "note: stack '{alias}' now resolves StackManifest {after}, but the deployment its SDK reads was deployed from {before}. {redeploy}."
                )
            })
        })
        .collect()
}

fn validate_update_selection(
    manifest: &ProjectManifest,
    selection: UpdateSelection<'_>,
) -> Result<()> {
    if let Some(alias) = selection.alias {
        let kind = selection
            .kind
            .ok_or_else(|| anyhow::anyhow!("An update alias requires its dependency kind"))?;
        let dependency = manifest
            .dependency(kind, alias)
            .ok_or_else(|| anyhow::anyhow!("No {kind} dependency named '{alias}'"))?;
        if !updatable(manifest, kind, dependency) {
            bail!("Dependency '{alias}' is local and has no registry version to update");
        }
    } else if let Some(kind) = selection.kind {
        let count = manifest
            .dependencies()
            .filter(|(candidate, _, dependency)| {
                *candidate == kind && updatable(manifest, kind, dependency)
            })
            .count();
        if count == 0 {
            bail!("Project has no registry {kind} dependencies to update");
        }
    }
    Ok(())
}

fn print_plan(plan: &InstallPlan, lock: Option<&ProjectLock>, manifest: &ProjectManifest) {
    let status = if lock.is_some_and(|lock| lock.is_fresh(&manifest.manifest_hash)) {
        "fresh"
    } else if lock.is_some() {
        "stale"
    } else {
        "missing"
    };
    println!("Lock: {status}");
    for output in &plan.outputs {
        println!(
            "{} {} {} -> {}",
            output.kind,
            output.alias,
            output.target,
            output.path.display()
        );
    }
}

enum ResolvedProjectDependency {
    LocalStack {
        alias: String,
        source: String,
        targets: Vec<InstallTarget>,
        manifest_path: PathBuf,
        artifact_roots: Vec<PathBuf>,
        stack: LocalArtifactStack,
    },
    LocalProgram {
        alias: String,
        source: String,
        targets: Vec<InstallTarget>,
        program_spec_path: PathBuf,
        program_spec: arete_artifacts::ProgramSpecArtifact,
    },
    Registry {
        kind: DependencyKind,
        source: String,
        requirement: String,
        targets: Vec<InstallTarget>,
        resolved: Box<ResolvedRegistryDependency>,
    },
    /// A workspace dependency on a composed `[authoring.stacks]` entry.
    ComposedStack {
        alias: String,
        source: String,
        targets: Vec<InstallTarget>,
        composed: Box<ComposedStack>,
    },
}

impl ResolvedProjectDependency {
    fn kind(&self) -> DependencyKind {
        match self {
            Self::LocalStack { .. } | Self::ComposedStack { .. } => DependencyKind::Stack,
            Self::LocalProgram { .. } => DependencyKind::Program,
            Self::Registry { kind, .. } => *kind,
        }
    }

    fn alias(&self) -> &str {
        match self {
            Self::LocalStack { alias, .. }
            | Self::LocalProgram { alias, .. }
            | Self::ComposedStack { alias, .. } => alias,
            Self::Registry { resolved, .. } => resolved.alias(),
        }
    }

    fn targets(&self) -> &[InstallTarget] {
        match self {
            Self::LocalStack { targets, .. }
            | Self::LocalProgram { targets, .. }
            | Self::Registry { targets, .. }
            | Self::ComposedStack { targets, .. } => targets,
        }
    }
}

/// Whether `dependency` is a workspace stack on a composed authoring entry,
/// and which one.
fn composed_workspace<'a>(
    manifest: &'a ProjectManifest,
    kind: DependencyKind,
    dependency: &'a DependencyV1,
) -> Option<&'a str> {
    match &dependency.source {
        DependencySourceV1::Workspace(WorkspaceSourceV1 { workspace })
            if kind == DependencyKind::Stack
                && manifest
                    .document
                    .authoring
                    .stacks
                    .get(workspace)
                    .is_some_and(super::manifest::AuthoringStackV1::is_composed) =>
        {
            Some(workspace)
        }
        _ => None,
    }
}

/// Whether `a4 update` advances `dependency`: a registry dependency, or a
/// composed stack, whose registry parts it re-resolves.
fn updatable(manifest: &ProjectManifest, kind: DependencyKind, dependency: &DependencyV1) -> bool {
    matches!(&dependency.source, DependencySourceV1::Registry(_))
        || composed_workspace(manifest, kind, dependency).is_some()
}

fn resolve_dependencies(
    manifest: &ProjectManifest,
    previous_lock: Option<&ProjectLock>,
    update: Option<UpdateSelection<'_>>,
) -> Result<Vec<ResolvedProjectDependency>> {
    let paths = ProjectPaths::new(&manifest.root, false, false)?;
    let previous = previous_lock
        .into_iter()
        .flat_map(|lock| &lock.dependencies)
        .map(|entry| ((entry.kind, entry.alias.as_str()), entry))
        .collect::<BTreeMap<_, _>>();
    let mut registry_requests = Vec::new();
    let mut resolved = Vec::new();

    for (kind, alias, dependency) in manifest.dependencies() {
        let targets = dependency.selected_targets(&manifest.document.sdk).to_vec();
        match &dependency.source {
            DependencySourceV1::Registry(RegistrySourceV1 { registry }) => {
                let requirement = dependency.version.clone().expect("validated version");
                let unlocked = update.is_some_and(|selection| {
                    selection.kind.is_none_or(|selected| selected == kind)
                        && selection.alias.is_none_or(|selected| selected == alias)
                });
                let reusable = previous
                    .get(&(kind, alias.as_str()))
                    .copied()
                    .filter(|entry| {
                        !unlocked
                            && entry.kind == kind
                            && entry.source == dependency.source.stable_description()
                            && entry.requirement.as_deref() == Some(requirement.as_str())
                            && entry.targets == targets
                    });
                registry_requests.push(RegistryDependencyRequest {
                    kind,
                    alias: alias.clone(),
                    package: registry.clone(),
                    requirement,
                    locked_package_release_hash: reusable
                        .and_then(|entry| entry.package_release_hash.clone()),
                    locked_programs: reusable
                        .map(|entry| entry.programs.clone())
                        .unwrap_or_default(),
                });
            }
            DependencySourceV1::Path(PathSourceV1 { path }) => resolved.push(
                resolve_path_dependency(&paths, kind, alias, dependency, path, targets)?,
            ),
            DependencySourceV1::Workspace(WorkspaceSourceV1 { workspace })
                if composed_workspace(manifest, kind, dependency).is_some() =>
            {
                let unlocked = update.is_some_and(|selection| {
                    selection.kind.is_none_or(|selected| selected == kind)
                        && selection.alias.is_none_or(|selected| selected == alias)
                });
                let source = dependency.source.stable_description();
                let reusable = previous
                    .get(&(kind, alias.as_str()))
                    .copied()
                    .filter(|entry| {
                        !unlocked && entry.source == source && entry.targets == targets
                    });
                let composed =
                    composition::resolve_composition(manifest, workspace, alias, reusable)?;
                resolved.push(ResolvedProjectDependency::ComposedStack {
                    alias: alias.clone(),
                    source,
                    targets,
                    composed: Box::new(composed),
                });
            }
            DependencySourceV1::Workspace(WorkspaceSourceV1 { workspace }) => {
                resolved.push(resolve_workspace_dependency(
                    manifest, &paths, kind, alias, dependency, workspace, targets,
                )?)
            }
        }
    }

    if !registry_requests.is_empty() {
        resolved.extend(resolve_registry_requests(manifest, registry_requests)?);
    }
    resolved.sort_by(|left, right| (left.kind(), left.alias()).cmp(&(right.kind(), right.alias())));
    Ok(resolved)
}

/// Resolves registry dependencies in one resolver batch and verifies each
/// response against its request (and, when locked, the pinned release).
fn resolve_registry_requests(
    manifest: &ProjectManifest,
    registry_requests: Vec<RegistryDependencyRequest>,
) -> Result<Vec<ResolvedProjectDependency>> {
    let responses = resolve_registry_batch(
        manifest.document.manifest_version,
        &manifest.document.sdk.targets,
        &registry_requests,
        None,
    )?;
    Ok(registry_requests
        .into_iter()
        .zip(responses)
        .map(|(request, response)| {
            let dependency = manifest
                .dependency(request.kind, &request.alias)
                .expect("request came from manifest");
            ResolvedProjectDependency::Registry {
                kind: request.kind,
                source: dependency.source.stable_description(),
                requirement: request.requirement,
                targets: dependency.selected_targets(&manifest.document.sdk).to_vec(),
                resolved: Box::new(response),
            }
        })
        .collect())
}

/// Resolves registry requests in one resolver batch and verifies each
/// response against its request (and, when locked, the pinned release).
/// `composition` names the composed stack dependency the requests are parts
/// of, so a lock failure names the command that advances it.
pub(crate) fn resolve_registry_batch(
    manifest_version: u32,
    targets: &[InstallTarget],
    registry_requests: &[RegistryDependencyRequest],
    composition: Option<&str>,
) -> Result<Vec<ResolvedRegistryDependency>> {
    let mut resolved = Vec::with_capacity(registry_requests.len());
    let request = RegistryResolveRequest {
        manifest_version,
        dependencies: registry_requests.to_vec(),
        targets: targets.to_vec(),
        generator_contract: GENERATOR_CONTRACT.into(),
    };
    // A batch failure names one package only when the batch *is* one
    // package. Attributing a multi-dependency failure to the first entry
    // reported the wrong package and, with a batch-wide `locked` flag,
    // could tell the user to `a4 update` a dependency that is not locked.
    let single = match registry_requests {
        [only] => Some((
            only.kind,
            only.package.clone(),
            only.locked_package_release_hash.is_some(),
        )),
        _ => None,
    };
    let response = ApiClient::new()?
        .resolve_registry_dependencies(&request)
        .map_err(|error| match &single {
            Some((kind, package, locked)) => {
                describe_resolver_error(error, *kind, package, *locked)
            }
            None => describe_resolver_batch_error(error, registry_requests),
        })?;
    if response.resolver_contract != RESOLVER_CONTRACT {
        bail!(
            "Registry returned resolver contract '{}'; expected '{}'",
            response.resolver_contract,
            RESOLVER_CONTRACT
        );
    }
    if response.dependencies.len() != registry_requests.len() {
        bail!("Registry resolver did not return exactly one dependency per request");
    }
    for (request, response) in registry_requests.iter().zip(response.dependencies) {
        if response.alias() != request.alias {
            bail!(
                "Resolver response order mismatch: expected '{}', received '{}'",
                request.alias,
                response.alias()
            );
        }
        if response.package() != request.package {
            bail!(
                "Resolver response package mismatch for '{}': expected '{}', received '{}'",
                request.alias,
                request.package,
                response.package()
            );
        }
        verify_resolved_kind_and_contract(request.kind, &response)?;
        verify_resolved_extensions(&response, targets)?;
        verify_resolved_release_identity(&response)?;
        verify_resolved_stack_delivery(&response)?;
        if let Some(locked) = &request.locked_package_release_hash {
            if response.package_release_hash() != locked {
                let (subject, update) = match composition {
                    Some(alias) => (
                        format!(
                            "{} '{}' of composed stack '{alias}'",
                            request.kind, request.package
                        ),
                        format!("a4 update stack {alias}"),
                    ),
                    None => (
                        format!("'{}'", request.alias),
                        format!("a4 update {} {}", request.kind, request.alias),
                    ),
                };
                bail!(
                    "Registry resolved {subject} to release {} but arete.lock pins {}; run `{update}` to advance intentionally",
                    response.package_release_hash(),
                    locked,
                );
            }
            verify_locked_program_sdks(request, &response)?;
        }
        resolved.push(response);
    }
    Ok(resolved)
}

/// A locked stack release pins the program SDK release it references for
/// each program. Resolving the same stack release to a different program
/// package release would mean the reference moved underneath the lock, so it
/// is an integrity failure rather than an update.
fn verify_locked_program_sdks(
    request: &RegistryDependencyRequest,
    response: &ResolvedRegistryDependency,
) -> Result<()> {
    let ResolvedRegistryDependency::Stack { programs, .. } = response else {
        return Ok(());
    };
    for locked in &request.locked_programs {
        let Some(pinned) = &locked.package_release_hash else {
            continue;
        };
        let resolved = programs
            .iter()
            .find(|program| {
                program.definition.program_id == locked.program_id
                    && program.definition.program_spec_hash == locked.program_spec_hash
            })
            .and_then(|program| program.program_package.as_ref())
            .map(|package| package.package_release_hash.as_str());
        if resolved != Some(pinned.as_str()) {
            bail!(
                "arete.lock integrity failure for stack '{}': its locked release pins program {} to program SDK release {pinned}, but the registry now returns {}. Nothing was changed; run `a4 update stack {}` only if you intend to advance",
                request.alias,
                locked.program_id,
                resolved.unwrap_or("no program SDK release"),
                request.alias
            );
        }
    }
    Ok(())
}

/// Restores the registry cache entries of one installed dependency by
/// resolving exactly the release `arete.lock` pins. `a4 up <alias>` deploys an
/// installed stack from the cache, so a cleared cache is refilled here rather
/// than by a full `a4 install`.
pub(crate) fn restore_locked_registry_cache(
    manifest: &ProjectManifest,
    locked: &LockedDependency,
) -> Result<()> {
    let dependency = manifest
        .dependency(locked.kind, &locked.alias)
        .ok_or_else(|| anyhow::anyhow!("arete.toml has no {} '{}'", locked.kind, locked.alias))?;
    let DependencySourceV1::Registry(RegistrySourceV1 { registry }) = &dependency.source else {
        bail!(
            "{} '{}' is local and has no registry artifacts to restore",
            locked.kind,
            locked.alias
        );
    };
    let package_release_hash = locked.package_release_hash.clone().ok_or_else(|| {
        anyhow::anyhow!(
            "arete.lock pins no release for {} '{}'",
            locked.kind,
            locked.alias
        )
    })?;
    let request = RegistryDependencyRequest {
        kind: locked.kind,
        alias: locked.alias.clone(),
        package: registry.clone(),
        requirement: dependency.version.clone().expect("validated version"),
        locked_package_release_hash: Some(package_release_hash),
        locked_programs: locked.programs.clone(),
    };
    for dependency in resolve_registry_requests(manifest, vec![request])? {
        if let ResolvedProjectDependency::Registry { resolved, .. } = &dependency {
            cache_registry_dependency(resolved)?;
        }
    }
    Ok(())
}

fn resolve_path_dependency(
    paths: &ProjectPaths,
    kind: DependencyKind,
    alias: &str,
    dependency: &DependencyV1,
    path: &str,
    targets: Vec<InstallTarget>,
) -> Result<ResolvedProjectDependency> {
    let source = dependency.source.stable_description();
    match kind {
        DependencyKind::Stack => {
            let manifest_path = paths.input(path, "StackManifest dependency")?;
            let artifact_roots = vec![manifest_path
                .parent()
                .ok_or_else(|| anyhow::anyhow!("StackManifest has no parent"))?
                .to_path_buf()];
            let stack = load_local_artifact_stack_with_roots(&manifest_path, &artifact_roots)?;
            Ok(ResolvedProjectDependency::LocalStack {
                alias: alias.into(),
                source,
                targets,
                manifest_path,
                artifact_roots,
                stack,
            })
        }
        DependencyKind::Program => {
            let program_spec_path = paths.input(path, "ProgramSpec dependency")?;
            let program_spec = load_program_spec(&program_spec_path)?;
            Ok(ResolvedProjectDependency::LocalProgram {
                alias: alias.into(),
                source,
                targets,
                program_spec_path,
                program_spec,
            })
        }
    }
}

fn resolve_workspace_dependency(
    manifest: &ProjectManifest,
    paths: &ProjectPaths,
    kind: DependencyKind,
    alias: &str,
    dependency: &DependencyV1,
    workspace: &str,
    targets: Vec<InstallTarget>,
) -> Result<ResolvedProjectDependency> {
    let source = dependency.source.stable_description();
    match kind {
        DependencyKind::Stack => {
            let authored = &manifest.document.authoring.stacks[workspace];
            let manifest_path =
                paths.input(authored.manifest_path(workspace)?, "authored StackManifest")?;
            let artifact_roots = if authored.artifact_roots.is_empty() {
                vec![manifest_path
                    .parent()
                    .ok_or_else(|| anyhow::anyhow!("StackManifest has no parent"))?
                    .to_path_buf()]
            } else {
                authored
                    .artifact_roots
                    .iter()
                    .map(|root| paths.input_directory(root, "authoring artifact root"))
                    .collect::<Result<Vec<_>>>()?
            };
            let stack = load_local_artifact_stack_with_roots(&manifest_path, &artifact_roots)?;
            Ok(ResolvedProjectDependency::LocalStack {
                alias: alias.into(),
                source,
                targets,
                manifest_path,
                artifact_roots,
                stack,
            })
        }
        DependencyKind::Program => {
            let authored = &manifest.document.authoring.programs[workspace];
            let program_spec_path = paths.input(&authored.program_spec, "authored ProgramSpec")?;
            let program_spec = load_program_spec(&program_spec_path)?;
            Ok(ResolvedProjectDependency::LocalProgram {
                alias: alias.into(),
                source,
                targets,
                program_spec_path,
                program_spec,
            })
        }
    }
}

fn load_program_spec(path: &Path) -> Result<arete_artifacts::ProgramSpecArtifact> {
    let bytes =
        fs::read(path).with_context(|| format!("Failed to read ProgramSpec {}", path.display()))?;
    Ok(arete_artifacts::load_program_spec(&bytes)
        .with_context(|| format!("Invalid ProgramSpec {}", path.display()))?
        .artifact)
}

fn verify_resolved_kind_and_contract(
    expected: DependencyKind,
    resolved: &ResolvedRegistryDependency,
) -> Result<()> {
    let (actual, contract) = match resolved {
        ResolvedRegistryDependency::Stack {
            generator_contract, ..
        } => (DependencyKind::Stack, generator_contract),
        ResolvedRegistryDependency::Program {
            generator_contract, ..
        } => (DependencyKind::Program, generator_contract),
    };
    if actual != expected {
        bail!("Registry resolver returned kind '{actual}' for requested '{expected}'");
    }
    if contract != GENERATOR_CONTRACT {
        bail!("Registry requires unsupported generator contract '{contract}'");
    }
    Ok(())
}

fn verify_resolved_extensions(
    resolved: &ResolvedRegistryDependency,
    targets: &[InstallTarget],
) -> Result<()> {
    let extensions = match resolved {
        ResolvedRegistryDependency::Stack { sdk_extensions, .. }
        | ResolvedRegistryDependency::Program { sdk_extensions, .. } => sdk_extensions,
    };
    verify_extension_list(extensions, targets, "")?;
    // A stack's programs carry their program SDKs' extensions under the same
    // rules: requested targets only, one per target, exact content identity.
    if let ResolvedRegistryDependency::Stack { programs, .. } = resolved {
        for program in programs {
            if let Some(extensions) = &program.sdk_extensions {
                verify_extension_list(
                    extensions,
                    targets,
                    &format!(" for program '{}'", program.install_name),
                )?;
            }
        }
    }
    Ok(())
}

fn verify_extension_list(
    extensions: &[super::resolver::ResolvedSdkExtension],
    targets: &[InstallTarget],
    owner: &str,
) -> Result<()> {
    let requested = targets
        .iter()
        .map(|target| target.as_str())
        .collect::<BTreeSet<_>>();
    let mut seen = BTreeSet::new();
    for extension in extensions {
        if !requested.contains(extension.target.as_str()) {
            bail!(
                "Registry returned an unrequested '{}' SDK extension{owner}",
                extension.target
            );
        }
        if !seen.insert(extension.target.as_str()) {
            bail!(
                "Registry returned more than one '{}' SDK extension{owner}",
                extension.target
            );
        }
        if extension.content_hash.len() != 64
            || !extension
                .content_hash
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            || extension.artifact.artifact_hash != extension.content_hash
        {
            bail!(
                "Registry returned an invalid '{}' SDK extension identity{owner}",
                extension.target
            );
        }
    }
    Ok(())
}

/// Choose the local alias for a registry dependency: the explicit `--alias`
/// when given (validated for every generated language), otherwise the
/// deterministic alias derived from the unchanged remote lookup.
fn select_local_alias(
    kind: DependencyKind,
    package: &str,
    explicit: Option<&str>,
) -> Result<String> {
    match explicit {
        Some(alias) => {
            super::alias::validate_local_alias(alias, kind)?;
            Ok(alias.to_string())
        }
        None => Ok(super::alias::derive_local_alias(package)),
    }
}

/// Every resolved package must carry an immutable release identity, and a
/// stack must pin every constituent program by exact release. Floating or
/// incomplete responses are rejected before anything is generated.
fn verify_resolved_release_identity(resolved: &ResolvedRegistryDependency) -> Result<()> {
    let release = resolved.package_release_hash();
    if !is_package_release_hash(release) {
        bail!(
            "Registry returned an invalid immutable release identity '{release}' for '{}'",
            resolved.package()
        );
    }
    if let ResolvedRegistryDependency::Program {
        package,
        package_release_hash,
        install,
        ..
    } = resolved
    {
        // A direct program install pins the same hosted program identity a
        // stack member does, so it gets the same check: without this only the
        // package-level hash was validated and an empty or inconsistent
        // program release could reach the generated SDK and arete.lock.
        if install.release.program_release_hash.trim().is_empty()
            || install.release.program_spec_hash != install.definition.program_spec_hash
        {
            bail!("Registry returned program '{package}' without an exact release identity");
        }
        // The package being installed is the program SDK: a descriptor that
        // names another package release contradicts the resolution.
        if let Some(reference) = &install.program_package {
            if &reference.package_release_hash != package_release_hash {
                bail!(
                    "Registry returned program '{package}' whose descriptor names program package release {} instead of {package_release_hash}",
                    reference.package_release_hash
                );
            }
        }
    }
    if let ResolvedRegistryDependency::Stack {
        package,
        stack_manifest,
        programs,
        ..
    } = resolved
    {
        let declared = stack_manifest
            .pointer("/payload/programs")
            .and_then(|programs| programs.as_array())
            .map(|programs| programs.len())
            .unwrap_or(0);
        if declared != programs.len() {
            bail!(
                "Registry returned {} exact program releases for stack '{package}' but its StackManifest declares {declared}; refusing an incomplete stack package",
                programs.len()
            );
        }
        for program in programs {
            if program.release.program_release_hash.trim().is_empty()
                || program.release.program_spec_hash != program.definition.program_spec_hash
            {
                bail!(
                    "Registry returned program '{}' for stack '{package}' without an exact release identity",
                    program.definition.program_id
                );
            }
            if let Some(reference) = &program.program_package {
                if reference.package.trim().is_empty()
                    || reference.version.trim().is_empty()
                    || !is_package_release_hash(&reference.package_release_hash)
                {
                    bail!(
                        "Registry returned program '{}' for stack '{package}' with an invalid program package release identity",
                        program.definition.program_id
                    );
                }
            }
        }
    }
    Ok(())
}

fn is_package_release_hash(value: &str) -> bool {
    [
        "arete:registry-package-release:v1:sha256:",
        "arete:registry-package-release:v2:sha256:",
    ]
    .iter()
    .any(|prefix| {
        value.strip_prefix(prefix).is_some_and(|digest| {
            digest.len() == 64
                && digest
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        })
    })
}

/// Describe a failure for a multi-dependency resolve. The transport reports
/// one status for the whole batch, so no single dependency can be blamed and
/// the remedy is stated over the set that was actually requested.
fn describe_resolver_batch_error(
    error: anyhow::Error,
    requests: &[RegistryDependencyRequest],
) -> anyhow::Error {
    let names = requests
        .iter()
        .map(|request| format!("{} '{}'", request.kind, request.package))
        .collect::<Vec<_>>()
        .join(", ");
    let locked = requests
        .iter()
        .filter(|request| request.locked_package_release_hash.is_some())
        .map(|request| format!("{} '{}'", request.kind, request.package))
        .collect::<Vec<_>>();
    let Some(http) = error.downcast_ref::<crate::api_client::ApiHttpError>() else {
        return error.context(format!("Failed to resolve {names} through the registry"));
    };
    match http.status {
        401 => {
            anyhow::anyhow!("Resolving {names} requires a login; run `a4 auth login` and try again")
        }
        403 => anyhow::anyhow!("This account is not entitled to resolve one or more of {names}"),
        404 => anyhow::anyhow!("One or more of {names} is unavailable to this account or unknown"),
        409 if http.code.as_deref() == Some(DELIVERY_NOT_READY) => anyhow::anyhow!(
            "A hosted stack among {names} is published but its live delivery is not currently \
             ready; nothing was installed and arete.lock is unchanged. Retry shortly ({http})"
        ),
        409 if !locked.is_empty() => anyhow::anyhow!(
            "The registry could not honor the exact lock for one or more of {}; this is an \
             integrity failure, so nothing was installed. Run `a4 update` for the affected \
             dependency once you have confirmed the intended release.",
            locked.join(", ")
        ),
        _ => error.context(format!("Failed to resolve {names} through the registry")),
    }
}

/// Translate a resolver transport failure into an actionable message without
/// creating an existence oracle: unknown names and packages owned by another
/// account produce the same text, and no lookup ever falls back to a
/// differently scoped endpoint.
fn describe_resolver_error(
    error: anyhow::Error,
    kind: DependencyKind,
    package: &str,
    locked: bool,
) -> anyhow::Error {
    let Some(http) = error.downcast_ref::<crate::api_client::ApiHttpError>() else {
        return error.context(format!(
            "Failed to resolve {kind} '{package}' through the registry"
        ));
    };
    match http.status {
        401 => anyhow::anyhow!(
            "Registry resolution for {kind} '{package}' requires a login: run `a4 auth login`, then retry ({http})"
        ),
        403 => anyhow::anyhow!(
            "This account is not entitled to install {kind} '{package}' ({http})"
        ),
        404 => anyhow::anyhow!(
            "{kind} '{package}' is unavailable to this account or unknown; check the name, or log in as the owner if it is private ({http})"
        ),
        409 if http.code.as_deref() == Some(DELIVERY_NOT_READY) => anyhow::anyhow!(
            "{kind} '{package}' is published but its live delivery is not currently ready; nothing was installed and arete.lock is unchanged. Retry shortly ({http})"
        ),
        409 if locked => anyhow::anyhow!(
            "arete.lock integrity failure for {kind} '{package}': the locked release is no longer resolvable. Nothing was changed; run `a4 update {kind} <alias>` only if you intend to advance ({http})"
        ),
        409 => anyhow::anyhow!(
            "Registry could not satisfy {kind} '{package}' ({http})"
        ),
        _ => anyhow::Error::from(http.clone()),
    }
}

fn build_lock(
    manifest: &ProjectManifest,
    resolved: &[ResolvedProjectDependency],
) -> Result<ProjectLock> {
    let mut lock = ProjectLock::empty(manifest.manifest_hash.clone());
    for dependency in resolved {
        lock.dependencies.push(match dependency {
            ResolvedProjectDependency::LocalStack {
                alias,
                source,
                targets,
                stack,
                ..
            } => LockedDependency {
                kind: DependencyKind::Stack,
                alias: alias.clone(),
                source: source.clone(),
                requirement: None,
                version: None,
                package_release_hash: None,
                stack_manifest_hash: Some(stack.manifest_hash.clone()),
                program_id: None,
                program_spec_hash: None,
                program_release_hash: None,
                live_specs: stack
                    .live_specs
                    .iter()
                    .map(|(alias, live)| LockedLiveSpec {
                        alias: alias.clone(),
                        artifact_hash: live.artifact_hash.to_string(),
                    })
                    .collect(),
                programs: stack
                    .program_specs
                    .iter()
                    .map(|program| LockedProgram {
                        program_id: program.payload.program_id.clone(),
                        program_spec_hash: program.artifact_hash.to_string(),
                        program_release_hash: None,
                        package_release_hash: None,
                        sdk_extension_hashes: Vec::new(),
                    })
                    .collect(),
                parts: Vec::new(),
                sdk_extension_hashes: Vec::new(),
                targets: targets.clone(),
                generator_contract: GENERATOR_CONTRACT.into(),
            },
            ResolvedProjectDependency::LocalProgram {
                alias,
                source,
                targets,
                program_spec,
                ..
            } => LockedDependency {
                kind: DependencyKind::Program,
                alias: alias.clone(),
                source: source.clone(),
                requirement: None,
                version: None,
                package_release_hash: None,
                stack_manifest_hash: None,
                program_id: Some(program_spec.payload.program_id.clone()),
                program_spec_hash: Some(program_spec.artifact_hash.to_string()),
                program_release_hash: None,
                live_specs: Vec::new(),
                programs: Vec::new(),
                parts: Vec::new(),
                sdk_extension_hashes: Vec::new(),
                targets: targets.clone(),
                generator_contract: GENERATOR_CONTRACT.into(),
            },
            ResolvedProjectDependency::Registry {
                kind,
                source,
                requirement,
                targets,
                resolved,
            } => registry_lock(*kind, source, requirement, targets, resolved),
            ResolvedProjectDependency::ComposedStack {
                alias,
                source,
                targets,
                composed,
            } => composed.lock_entry(alias, source, targets),
        });
    }
    lock.normalize_and_validate()?;
    Ok(lock)
}

/// Checks that `next`, resolved now, installs exactly what the existing lock
/// `previous` pins. They must be equal, except for a stack program locked by
/// a CLI that did not resolve program SDK identities: it pins no program
/// package release, which the immutable stack package release the lock pins
/// implies. Its SDK extension hashes must still be exactly the ones
/// generation uses for the project's targets, or the lock does not pin what
/// would be generated.
fn check_locked_resolution(
    previous: &ProjectLock,
    next: &ProjectLock,
    resolved: &[ResolvedProjectDependency],
) -> Result<()> {
    const DIFFERS: &str = "--locked resolution differs from arete.lock; no output was changed";
    if previous == next {
        return Ok(());
    }
    if previous.lock_version != next.lock_version
        || previous.manifest_hash != next.manifest_hash
        || previous.resolver_contract != next.resolver_contract
        || previous.dependencies.len() != next.dependencies.len()
    {
        bail!(DIFFERS);
    }
    for (before, after) in previous.dependencies.iter().zip(&next.dependencies) {
        if before == after {
            continue;
        }
        let stack_programs = resolved.iter().find_map(|dependency| match dependency {
            ResolvedProjectDependency::Registry {
                kind: DependencyKind::Stack,
                resolved,
                ..
            } if resolved.alias() == after.alias => match resolved.as_ref() {
                ResolvedRegistryDependency::Stack { programs, .. } => Some(programs),
                ResolvedRegistryDependency::Program { .. } => None,
            },
            _ => None,
        });
        let Some(stack_programs) = stack_programs else {
            bail!(DIFFERS);
        };
        let unenriched = |dependency: &LockedDependency| LockedDependency {
            programs: Vec::new(),
            ..dependency.clone()
        };
        if unenriched(before) != unenriched(after) || before.programs.len() != after.programs.len()
        {
            bail!(DIFFERS);
        }
        for (was, now) in before.programs.iter().zip(&after.programs) {
            let Some(program) = stack_programs.iter().find(|program| {
                program.definition.program_id == now.program_id
                    && program.definition.program_spec_hash == now.program_spec_hash
            }) else {
                bail!(DIFFERS);
            };
            let generated = after
                .targets
                .iter()
                .filter_map(|target| {
                    super::resolver::program_extension_for_target(program, *target).transpose()
                })
                .map(|extension| extension.map(|extension| extension_lock_hash(&extension)))
                .collect::<Result<BTreeSet<_>>>()?;
            // Older CLIs recorded the legacy single extension whatever its
            // target; for a target the project does not generate it pins
            // nothing.
            let ungenerated_legacy = program
                .definition
                .extensions
                .as_ref()
                .filter(|extension| {
                    !super::resolver::legacy_extension_target(extension)
                        .is_some_and(|target| after.targets.contains(&target))
                })
                .map(extension_lock_hash);
            match locked_program_match(was, now, &generated, ungenerated_legacy.as_deref()) {
                LockedProgramMatch::Same => {}
                LockedProgramMatch::Differs => bail!(DIFFERS),
                LockedProgramMatch::Unpinned => bail!(
                    "arete.lock was written before program SDK pins and does not record exactly the SDK extensions that stack '{}' generates for program {}. Run `a4 install` once to record the program SDK pins, then commit arete.lock; --locked changed nothing",
                    after.alias,
                    now.program_id
                ),
            }
        }
    }
    Ok(())
}

/// How a locked stack program compares with its resolution.
#[derive(Debug, PartialEq, Eq)]
enum LockedProgramMatch {
    /// The lock pins what is resolved.
    Same,
    /// The lock pins something else.
    Differs,
    /// The lock predates program SDK identities and its SDK extension
    /// hashes are not exactly the ones generation uses.
    Unpinned,
}

/// Compares a locked stack program with its resolution. `generated` holds
/// the SDK extension hashes generation uses for the project's targets, where
/// a legacy single extension counts for its own target; `ungenerated_legacy`
/// is the legacy extension's hash when that target is not generated.
///
/// A lock that predates program SDK identities pins no program package
/// release and matches when it pins the same program and Program Release
/// and exactly the generated extensions: more would leave generation using
/// extension content the lock never recorded.
fn locked_program_match(
    was: &LockedProgram,
    now: &LockedProgram,
    generated: &BTreeSet<String>,
    ungenerated_legacy: Option<&str>,
) -> LockedProgramMatch {
    if was == now {
        return LockedProgramMatch::Same;
    }
    if was.package_release_hash.is_some()
        || was.program_id != now.program_id
        || was.program_spec_hash != now.program_spec_hash
        || was.program_release_hash != now.program_release_hash
    {
        return LockedProgramMatch::Differs;
    }
    let pinned = was
        .sdk_extension_hashes
        .iter()
        .map(String::as_str)
        .filter(|hash| Some(*hash) != ungenerated_legacy)
        .collect::<BTreeSet<_>>();
    if pinned == generated.iter().map(String::as_str).collect() {
        LockedProgramMatch::Same
    } else {
        LockedProgramMatch::Unpinned
    }
}

fn registry_lock(
    kind: DependencyKind,
    source: &str,
    requirement: &str,
    targets: &[InstallTarget],
    resolved: &ResolvedRegistryDependency,
) -> LockedDependency {
    match resolved {
        ResolvedRegistryDependency::Stack {
            alias,
            version,
            package_release_hash,
            stack_manifest_hash,
            live_specs,
            programs,
            sdk_extensions,
            ..
        } => LockedDependency {
            kind,
            alias: alias.clone(),
            source: source.into(),
            requirement: Some(requirement.into()),
            version: Some(version.clone()),
            package_release_hash: Some(package_release_hash.clone()),
            stack_manifest_hash: Some(stack_manifest_hash.clone()),
            program_id: None,
            program_spec_hash: None,
            program_release_hash: None,
            live_specs: live_specs
                .iter()
                .map(|live| LockedLiveSpec {
                    alias: live.alias.clone(),
                    artifact_hash: live.artifact_hash.clone(),
                })
                .collect(),
            programs: programs
                .iter()
                .map(|program| LockedProgram {
                    program_id: program.definition.program_id.clone(),
                    program_spec_hash: program.definition.program_spec_hash.clone(),
                    program_release_hash: Some(program.release.program_release_hash.clone()),
                    package_release_hash: program
                        .program_package
                        .as_ref()
                        .map(|package| package.package_release_hash.clone()),
                    sdk_extension_hashes: super::resolver::program_extension_artifacts(
                        program, targets,
                    )
                    .into_iter()
                    .map(extension_lock_hash)
                    .collect(),
                })
                .collect(),
            parts: Vec::new(),
            sdk_extension_hashes: sdk_extensions
                .iter()
                .filter(|extension| {
                    targets
                        .iter()
                        .any(|target| target.as_str() == extension.target)
                })
                .map(|extension| extension.content_hash.clone())
                .collect(),
            targets: targets.to_vec(),
            generator_contract: GENERATOR_CONTRACT.into(),
        },
        ResolvedRegistryDependency::Program {
            alias,
            version,
            package_release_hash,
            install,
            sdk_extensions,
            ..
        } => LockedDependency {
            kind,
            alias: alias.clone(),
            source: source.into(),
            requirement: Some(requirement.into()),
            version: Some(version.clone()),
            package_release_hash: Some(package_release_hash.clone()),
            stack_manifest_hash: None,
            program_id: Some(install.definition.program_id.clone()),
            program_spec_hash: Some(install.definition.program_spec_hash.clone()),
            program_release_hash: Some(install.release.program_release_hash.clone()),
            live_specs: Vec::new(),
            programs: Vec::new(),
            parts: Vec::new(),
            sdk_extension_hashes: sdk_extensions
                .iter()
                .filter(|extension| {
                    targets
                        .iter()
                        .any(|target| target.as_str() == extension.target)
                })
                .map(|extension| extension.content_hash.clone())
                .collect(),
            targets: targets.to_vec(),
            generator_contract: GENERATOR_CONTRACT.into(),
        },
    }
}

fn extension_lock_hash(extension: &crate::api_client::RegistrySdkExtensionArtifact) -> String {
    extension
        .sdk_extension_hash
        .clone()
        .unwrap_or_else(|| extension.artifact_hash.clone())
}

fn validate_local_closure(manifest: &ProjectManifest) -> Result<()> {
    let paths = ProjectPaths::new(&manifest.root, false, false)?;
    for (kind, alias, dependency) in manifest.dependencies() {
        match &dependency.source {
            DependencySourceV1::Registry(_) => {}
            DependencySourceV1::Path(PathSourceV1 { path }) => {
                resolve_path_dependency(
                    &paths,
                    kind,
                    alias,
                    dependency,
                    path,
                    dependency.selected_targets(&manifest.document.sdk).to_vec(),
                )?;
            }
            DependencySourceV1::Workspace(WorkspaceSourceV1 { workspace })
                if composed_workspace(manifest, kind, dependency).is_some() =>
            {
                composition::validate_composition_files(manifest, workspace)?;
            }
            DependencySourceV1::Workspace(WorkspaceSourceV1 { workspace }) => {
                resolve_workspace_dependency(
                    manifest,
                    &paths,
                    kind,
                    alias,
                    dependency,
                    workspace,
                    dependency.selected_targets(&manifest.document.sdk).to_vec(),
                )?;
            }
        }
    }
    Ok(())
}

struct StagedOutput {
    final_path: PathBuf,
    staged_path: PathBuf,
}

struct RemovalOutput {
    final_path: PathBuf,
    kind: DependencyKind,
    alias: String,
    target: InstallTarget,
}

fn generate_all(
    manifest: &ProjectManifest,
    plan: &InstallPlan,
    resolved: &[ResolvedProjectDependency],
    staging_root: &Path,
) -> Result<Vec<StagedOutput>> {
    let by_alias = resolved
        .iter()
        .map(|dependency| ((dependency.kind(), dependency.alias()), dependency))
        .collect::<BTreeMap<_, _>>();
    let mut staged = Vec::with_capacity(plan.outputs.len());
    for (position, output) in plan.outputs.iter().enumerate() {
        let dependency = by_alias
            .get(&(output.kind, output.alias.as_str()))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "No resolution for {} dependency '{}'",
                    output.kind,
                    output.alias
                )
            })?;
        if !dependency.targets().contains(&output.target) {
            bail!(
                "Install plan selected an undeclared target for '{}'",
                output.alias
            );
        }
        status_line(format!(
            "Generating {} {} ({})",
            output.kind, output.alias, output.target
        ));
        let staged_path = staging_root
            .join("outputs")
            .join(format!("{position:04}"))
            .join("output");
        let options = ProjectGenerationOptions {
            alias: &output.alias,
            target: output.target,
            output: &staged_path,
            typescript_package: &manifest.document.sdk.typescript.package,
            rust_module: manifest.document.sdk.rust.module_mode,
            python_module: manifest.document.sdk.python.module_mode,
            stack_endpoints: manifest
                .dependency(output.kind, &output.alias)
                .map(|dependency| &dependency.endpoints),
        };
        match dependency {
            ResolvedProjectDependency::LocalStack {
                manifest_path,
                artifact_roots,
                ..
            } => generate_project_local_stack(manifest_path, artifact_roots, options)?,
            ResolvedProjectDependency::LocalProgram {
                program_spec_path, ..
            } => generate_project_local_program(program_spec_path, options)?,
            ResolvedProjectDependency::Registry { resolved, .. } => {
                generate_project_registry_dependency(resolved, options)?
            }
            ResolvedProjectDependency::ComposedStack { composed, .. } => {
                generate_project_composed_stack(composed, options)?
            }
        }
        attach_project_provenance(
            &staged_path,
            dependency,
            &manifest.manifest_hash,
            output.target,
        )?;
        staged.push(StagedOutput {
            final_path: output.path.clone(),
            staged_path,
        });
    }
    for dependency in resolved {
        match dependency {
            ResolvedProjectDependency::Registry { resolved, .. } => {
                cache_registry_dependency(resolved)?;
            }
            ResolvedProjectDependency::ComposedStack { composed, .. } => {
                for part in &composed.registry {
                    cache_registry_dependency(part)?;
                }
            }
            _ => {}
        }
    }
    Ok(staged)
}

pub(crate) fn cache_registry_dependency(resolved: &ResolvedRegistryDependency) -> Result<()> {
    match resolved {
        ResolvedRegistryDependency::Stack {
            stack_manifest_hash,
            stack_manifest,
            live_specs,
            programs,
            sdk_extensions,
            ..
        } => {
            cache_immutable_json("stack-manifest", stack_manifest_hash, stack_manifest)?;
            for live in live_specs {
                cache_immutable_json("live-spec", &live.artifact_hash, &live.artifact)?;
            }
            for program in programs {
                cache_program_install(program)?;
            }
            for extension in sdk_extensions {
                cache_immutable_json(
                    "sdk-extension",
                    &extension.content_hash,
                    &serde_json::to_value(&extension.artifact)?,
                )?;
            }
        }
        ResolvedRegistryDependency::Program {
            install,
            sdk_extensions,
            ..
        } => {
            cache_program_install(install)?;
            for extension in sdk_extensions {
                cache_immutable_json(
                    "sdk-extension",
                    &extension.content_hash,
                    &serde_json::to_value(&extension.artifact)?,
                )?;
            }
        }
    }
    Ok(())
}

fn cache_program_install(
    install: &crate::api_client::RegistryProgramInstallResponse,
) -> Result<()> {
    cache_immutable_json(
        "program-spec",
        &install.definition.program_spec_hash,
        &install.definition.program_spec,
    )?;
    if let Some(extension) = &install.definition.extensions {
        cache_immutable_json(
            "sdk-extension",
            &extension.artifact_hash,
            &serde_json::to_value(extension)?,
        )?;
    }
    for extension in install.sdk_extensions.iter().flatten() {
        cache_immutable_json(
            "sdk-extension",
            &extension.content_hash,
            &serde_json::to_value(&extension.artifact)?,
        )?;
    }
    Ok(())
}

fn cache_immutable_json(kind: &str, hash: &str, value: &serde_json::Value) -> Result<()> {
    let path = registry_cache::file(&registry_cache::root()?, kind, hash)?;
    let directory = path.parent().expect("cache files live in a kind directory");
    fs::create_dir_all(directory)?;
    if path.exists() {
        let cached = fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
        if cached.as_ref() == Some(value) {
            return Ok(());
        }
        fs::remove_file(&path).with_context(|| {
            format!(
                "Failed to evict corrupt registry cache entry {}",
                path.display()
            )
        })?;
    }
    let temporary = directory.join(format!(".{hash}.{}.tmp", uuid::Uuid::new_v4()));
    fs::write(&temporary, serde_json::to_vec(value)?)?;
    fs::rename(&temporary, &path)?;
    Ok(())
}

fn attach_project_provenance(
    output: &Path,
    dependency: &ResolvedProjectDependency,
    manifest_hash: &str,
    target: InstallTarget,
) -> Result<()> {
    let path = output.join("sdk-provenance.json");
    let contents = fs::read_to_string(&path).with_context(|| {
        format!(
            "Generated {target} output for '{}' omitted sdk-provenance.json",
            dependency.alias()
        )
    })?;
    let mut value: serde_json::Value = serde_json::from_str(&contents)
        .with_context(|| format!("Generated invalid provenance {}", path.display()))?;
    let package_release_hash = match dependency {
        ResolvedProjectDependency::Registry { resolved, .. } => Some(match resolved.as_ref() {
            ResolvedRegistryDependency::Stack {
                package_release_hash,
                ..
            }
            | ResolvedRegistryDependency::Program {
                package_release_hash,
                ..
            } => package_release_hash,
        }),
        _ => None,
    };
    value["project"] = serde_json::json!({
        "rootAlias": dependency.alias(),
        "dependencyKind": dependency.kind(),
        "packageReleaseHash": package_release_hash,
        "manifestHash": manifest_hash,
        "generatorContract": GENERATOR_CONTRACT,
        "target": target,
    });
    fs::write(
        &path,
        format!("{}\n", serde_json::to_string_pretty(&value)?),
    )?;
    validate_provenance_inventory(output, &value)
}

fn validate_provenance_inventory(output: &Path, value: &serde_json::Value) -> Result<()> {
    let artifacts = value["artifacts"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("sdk-provenance.json lacks an artifacts array"))?;
    for artifact in artifacts {
        let relative = artifact
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("provenance artifact path is not a string"))?;
        let path = output.join(relative);
        if !path.is_file() {
            bail!(
                "Generated provenance references missing file {}",
                path.display()
            );
        }
    }
    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct InstallJournal {
    expected_lock_sha256: String,
    staging_root: PathBuf,
    entries: Vec<JournalEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct JournalEntry {
    final_path: PathBuf,
    staged_path: Option<PathBuf>,
    backup_path: PathBuf,
    had_previous: bool,
    committed: bool,
}

fn commit_install(
    project_root: &Path,
    lock_path: &Path,
    lock: &ProjectLock,
    staged: Vec<StagedOutput>,
    removals: Vec<RemovalOutput>,
    staging_root: &Path,
) -> Result<()> {
    let expected_lock_sha256 = sha256(lock.canonical_toml()?.as_bytes());
    let backup_root = staging_root.join("backups");
    fs::create_dir_all(&backup_root)?;
    let mut journal = InstallJournal {
        expected_lock_sha256,
        staging_root: staging_root.to_path_buf(),
        entries: Vec::with_capacity(staged.len() + removals.len()),
    };
    for (position, output) in staged.into_iter().enumerate() {
        if output.final_path.exists() {
            reject_unowned_files(&output.final_path)?;
        }
        journal.entries.push(JournalEntry {
            had_previous: output.final_path.exists(),
            final_path: output.final_path,
            staged_path: Some(output.staged_path),
            backup_path: backup_root.join(format!("{position:04}")),
            committed: false,
        });
    }
    let staged_count = journal.entries.len();
    for (offset, output) in removals.into_iter().enumerate() {
        if !output.final_path.exists() {
            continue;
        }
        reject_unowned_files(&output.final_path)?;
        validate_project_output_ownership(&output)?;
        journal.entries.push(JournalEntry {
            had_previous: true,
            final_path: output.final_path,
            staged_path: None,
            backup_path: backup_root.join(format!("{:04}", staged_count + offset)),
            committed: false,
        });
    }
    write_journal(project_root, &journal)?;

    let result = (|| -> Result<()> {
        for position in 0..journal.entries.len() {
            let entry = &journal.entries[position];
            if let Some(parent) = entry.final_path.parent() {
                fs::create_dir_all(parent)?;
            }
            if entry.had_previous {
                fs::rename(&entry.final_path, &entry.backup_path)
                    .with_context(|| format!("Failed to back up {}", entry.final_path.display()))?;
            }
            if let Some(staged_path) = &entry.staged_path {
                fs::rename(staged_path, &entry.final_path).with_context(|| {
                    format!(
                        "Failed to commit {} to {} (outputs on another filesystem are unsupported)",
                        staged_path.display(),
                        entry.final_path.display()
                    )
                })?;
            }
            journal.entries[position].committed = true;
            write_journal(project_root, &journal)?;
        }
        lock.write_atomic(lock_path)?;
        Ok(())
    })();
    if let Err(error) = result {
        rollback_journal(&journal)?;
        remove_journal(project_root)?;
        let _ = fs::remove_dir_all(staging_root);
        return Err(error);
    }
    remove_journal(project_root)?;
    let _ = fs::remove_dir_all(staging_root);
    Ok(())
}

fn validate_project_output_ownership(output: &RemovalOutput) -> Result<()> {
    let provenance_path = output.final_path.join("sdk-provenance.json");
    let value: serde_json::Value = serde_json::from_slice(&fs::read(&provenance_path)?)
        .with_context(|| format!("Invalid ownership provenance {}", provenance_path.display()))?;
    let project = value["project"].as_object().ok_or_else(|| {
        anyhow::anyhow!(
            "SDK output {} has no project ownership provenance",
            output.final_path.display()
        )
    })?;
    let expected = [
        ("rootAlias", output.alias.as_str()),
        ("dependencyKind", output.kind.as_str()),
        ("target", output.target.as_str()),
        ("generatorContract", GENERATOR_CONTRACT),
    ];
    for (field, expected) in expected {
        if project.get(field).and_then(serde_json::Value::as_str) != Some(expected) {
            bail!(
                "Refusing to remove SDK output {}: project provenance field '{field}' does not match '{expected}'",
                output.final_path.display()
            );
        }
    }
    if !project
        .get("manifestHash")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|hash| hash.starts_with("arete-manifest-v1:"))
    {
        bail!(
            "Refusing to remove SDK output {}: project provenance has no valid installation manifest identity",
            output.final_path.display()
        );
    }
    Ok(())
}

fn reject_unowned_files(output: &Path) -> Result<()> {
    if output.is_symlink() || !output.is_dir() {
        bail!(
            "SDK output {} already exists and is not an owned directory",
            output.display()
        );
    }
    let provenance_path = output.join("sdk-provenance.json");
    let contents = fs::read_to_string(&provenance_path).with_context(|| {
        format!(
            "SDK output {} exists without ownership provenance",
            output.display()
        )
    })?;
    let value: serde_json::Value = serde_json::from_str(&contents)?;
    let mut owned = value["artifacts"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Existing provenance has no artifacts array"))?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| anyhow::anyhow!("Existing provenance has a non-string artifact"))
        })
        .collect::<Result<BTreeSet<_>>>()?;
    owned.insert("sdk-provenance.json".into());
    let mut actual = BTreeSet::new();
    collect_relative_files(output, output, &mut actual)?;
    actual.insert("sdk-provenance.json".into());
    let extras = actual.difference(&owned).cloned().collect::<Vec<_>>();
    if !extras.is_empty() {
        bail!(
            "SDK output {} contains files not owned by provenance: {}",
            output.display(),
            extras.join(", ")
        );
    }
    Ok(())
}

fn collect_relative_files(
    root: &Path,
    directory: &Path,
    files: &mut BTreeSet<String>,
) -> Result<()> {
    for entry in fs::read_dir(directory)
        .with_context(|| format!("Failed to inspect output {}", directory.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_relative_files(root, &path, files)?;
        } else if path.file_name().and_then(|name| name.to_str()) != Some("sdk-provenance.json") {
            files.insert(
                path.strip_prefix(root)?
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
    }
    Ok(())
}

fn write_journal(project_root: &Path, journal: &InstallJournal) -> Result<()> {
    let path = project_root.join(INSTALL_JOURNAL);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension(format!("json.{}.tmp", uuid::Uuid::new_v4()));
    fs::write(&temporary, serde_json::to_vec_pretty(journal)?)?;
    fs::rename(&temporary, path)?;
    Ok(())
}

fn remove_journal(project_root: &Path) -> Result<()> {
    let path = project_root.join(INSTALL_JOURNAL);
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

fn recover_interrupted_install(project_root: &Path) -> Result<()> {
    let path = project_root.join(INSTALL_JOURNAL);
    if !path.exists() {
        return Ok(());
    }
    let journal: InstallJournal = serde_json::from_slice(&fs::read(&path)?)
        .with_context(|| format!("Invalid install journal {}", path.display()))?;
    let lock_matches = fs::read(project_root.join("arete.lock"))
        .ok()
        .is_some_and(|contents| sha256(&contents) == journal.expected_lock_sha256);
    if lock_matches {
        for entry in &journal.entries {
            if entry.backup_path.exists() {
                remove_path(&entry.backup_path)?;
            }
        }
    } else {
        rollback_journal(&journal)?;
    }
    remove_journal(project_root)?;
    if journal.staging_root.exists() {
        fs::remove_dir_all(&journal.staging_root)?;
    }
    status_line("Recovered an interrupted Arete install");
    Ok(())
}

fn rollback_journal(journal: &InstallJournal) -> Result<()> {
    for entry in journal.entries.iter().rev() {
        let staged_output_was_moved = entry.committed
            || entry
                .staged_path
                .as_ref()
                .is_some_and(|staged_path| !staged_path.exists());
        if staged_output_was_moved && entry.final_path.exists() {
            remove_path(&entry.final_path)?;
        }
        if entry.had_previous && entry.backup_path.exists() {
            if let Some(parent) = entry.final_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::rename(&entry.backup_path, &entry.final_path)?;
        }
    }
    Ok(())
}

fn remove_path(path: &Path) -> Result<()> {
    if path.is_dir() && !path.is_symlink() {
        fs::remove_dir_all(path)?;
    } else if path.exists() || path.is_symlink() {
        fs::remove_file(path)?;
    }
    Ok(())
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn removal_fixture() -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("arete-remove-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let manifest_path = root.join("arete.toml");
        fs::write(
            &manifest_path,
            r#"
manifest_version = 1

# This comment should survive dependency edits.

[project]
name = "remove-test"

[sdk]
targets = ["typescript"]

[sdk.typescript]
output_dir = "./generated/typescript"

[dependencies.programs.demo]
source = { registry = "demo" }
version = "^1.0.0"
"#,
        )
        .unwrap();
        let manifest = ProjectManifest::load(&manifest_path).unwrap();
        let output = root.join("generated/typescript/programs/demo");
        fs::create_dir_all(&output).unwrap();
        fs::write(output.join("index.ts"), "export const demo = true;\n").unwrap();
        fs::write(
            output.join("sdk-provenance.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "artifacts": ["index.ts"],
                "project": {
                    "rootAlias": "demo",
                    "dependencyKind": "program",
                    "packageReleaseHash": null,
                    "manifestHash": manifest.manifest_hash,
                    "generatorContract": GENERATOR_CONTRACT,
                    "target": "typescript"
                }
            }))
            .unwrap(),
        )
        .unwrap();
        (root, manifest_path)
    }

    #[test]
    fn remove_prunes_manifest_lock_and_owned_output_without_resolution() {
        let (root, manifest_path) = removal_fixture();
        remove_and_install(
            &manifest_path,
            DependencyKind::Program,
            "demo",
            RemoveDependencyOptions::default(),
        )
        .unwrap();

        let manifest = ProjectManifest::load(&manifest_path).unwrap();
        assert!(manifest.document.dependencies.programs.is_empty());
        let source = fs::read_to_string(&manifest_path).unwrap();
        assert!(source.contains("# This comment should survive dependency edits."));
        assert!(!source.contains("[dependencies"));
        assert!(!source.contains("[install]"));
        assert!(!source.contains("[sdk.rust]"));
        assert!(!source.contains("[sdk.python]"));
        assert!(!source.contains("[authoring"));
        let lock = ProjectLock::load_optional(root.join("arete.lock"))
            .unwrap()
            .unwrap();
        assert!(lock.dependencies.is_empty());
        assert!(!root.join("generated/typescript/programs/demo").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn remove_accepts_owned_output_after_unrelated_manifest_change() {
        let (root, manifest_path) = removal_fixture();
        let installed_hash = ProjectManifest::load(&manifest_path).unwrap().manifest_hash;
        let source = fs::read_to_string(&manifest_path).unwrap();
        let updated = source.replace(
            "output_dir = \"./generated/typescript\"",
            "output_dir = \"./generated/typescript\"\npackage = \"@example/changed\"",
        );
        fs::write(&manifest_path, updated).unwrap();
        assert_ne!(
            ProjectManifest::load(&manifest_path).unwrap().manifest_hash,
            installed_hash
        );

        remove_and_install(
            &manifest_path,
            DependencyKind::Program,
            "demo",
            RemoveDependencyOptions::default(),
        )
        .unwrap();

        assert!(!root.join("generated/typescript/programs/demo").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn remove_refuses_output_with_unowned_files_and_restores_manifest() {
        let (root, manifest_path) = removal_fixture();
        fs::write(
            root.join("generated/typescript/programs/demo/hand-written.ts"),
            "export const keep = true;\n",
        )
        .unwrap();

        let error = remove_and_install(
            &manifest_path,
            DependencyKind::Program,
            "demo",
            RemoveDependencyOptions::default(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("files not owned by provenance"));
        let manifest = ProjectManifest::load(&manifest_path).unwrap();
        assert!(manifest.document.dependencies.programs.contains_key("demo"));
        assert!(root.join("generated/typescript/programs/demo").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn adding_dependency_preserves_manifest_format_and_omits_empty_properties() {
        let original = br#"manifest_version = 1

# Keep this project-level comment.
[project]
name = "format-test"
private = true

[sdk]
targets = ["typescript"]

[sdk.typescript]
output_dir = "./generated/typescript"

[dependencies.stacks.existing]
source = { registry = "existing" }
version = "^1.0.0"
"#;
        let dependency = DependencyV1 {
            source: DependencySourceV1::Registry(RegistrySourceV1 {
                registry: "demo".into(),
            }),
            version: Some("^2.0.0".into()),
            targets: Some(vec![InstallTarget::TypeScript]),
            outputs: DependencyOutputsV1::default(),
            endpoints: BTreeMap::new(),
        };

        let rendered = render_manifest_addition(
            original,
            DependencyKind::Program,
            "demo",
            &dependency,
            None,
            None,
        )
        .unwrap();

        assert!(rendered.contains("# Keep this project-level comment."));
        assert!(rendered.contains("source = { registry = \"existing\" }"));
        assert!(rendered.contains("[dependencies.programs.demo]"));
        assert!(rendered.contains("source = { registry = \"demo\" }"));
        assert!(!rendered.contains("[install]"));
        assert!(!rendered.contains("[sdk.rust]"));
        assert!(!rendered.contains("[sdk.python]"));
        assert!(!rendered.contains("outputs"));
        assert!(!rendered.contains("[authoring"));

        let parsed: ManifestV1 = toml::from_str(&rendered).unwrap();
        parsed.validate().unwrap();
        assert!(parsed.dependencies.programs.contains_key("demo"));
    }
}

#[cfg(test)]
mod private_install_tests {
    //! Owner-private registry installs through the manifest resolver: one
    //! endpoint for saved and `--no-save` flows, portable local aliases, exact
    //! immutable locks, deterministic reinstall, explicit update, actionable
    //! errors without an existence oracle, and atomic project updates.

    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::MutexGuard;

    use serde_json::{json, Value};

    use super::*;
    use crate::api_client::test_support::{MockServer, ENV_LOCK};

    /// Every project resolution opts into stack delivery.
    const RESOLVE_PATH: &str = "/api/registry/v1/resolve?include=delivery,program-sdks";
    const OWNER_KEY: &str = "a4_sk_private_install_owner";

    /// Serialises the process-global API URL and credentials for one test.
    struct RegistrySandbox {
        _guard: MutexGuard<'static, ()>,
        dir: tempfile::TempDir,
        server: MockServer,
    }

    impl RegistrySandbox {
        fn new(responses: Vec<(u16, String)>, authenticated: bool) -> Self {
            let guard = ENV_LOCK
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let dir = tempfile::tempdir().expect("tempdir");
            let server = MockServer::json_sequence(responses);
            std::env::set_var("ARETE_API_URL", server.base_url());
            let credentials = dir.path().join("credentials.toml");
            if authenticated {
                fs::write(
                    &credentials,
                    format!("[keys]\n\"{}\" = \"{OWNER_KEY}\"\n", server.base_url()),
                )
                .unwrap();
            } else {
                fs::write(&credentials, "[keys]\n").unwrap();
            }
            std::env::set_var("ARETE_CREDENTIALS_PATH", &credentials);
            std::env::set_var("ARETE_TELEMETRY_DISABLED", "1");
            RegistrySandbox {
                _guard: guard,
                dir,
                server,
            }
        }

        fn project(&self) -> PathBuf {
            let root = self.dir.path().join("project");
            fs::create_dir_all(&root).unwrap();
            let manifest = root.join("arete.toml");
            fs::write(
                &manifest,
                r#"manifest_version = 1

[project]
name = "private-install"
private = true

[sdk]
targets = ["typescript"]

[sdk.typescript]
output_dir = "./generated/typescript"
"#,
            )
            .unwrap();
            manifest
        }

        fn request(&self) -> crate::api_client::test_support::ReceivedRequest {
            self.server.request()
        }
    }

    impl Drop for RegistrySandbox {
        fn drop(&mut self) {
            std::env::remove_var("ARETE_API_URL");
            std::env::remove_var("ARETE_CREDENTIALS_PATH");
        }
    }

    fn program_spec_json(program_id: &str, name: &str) -> (Value, String) {
        let idl = format!(
            r#"{{"address":"{program_id}","metadata":{{"name":"{name}","version":"1.0.0","spec":"0.1.0"}},"instructions":[{{"name":"ping","discriminator":[1,2,3,4,5,6,7,8],"accounts":[{{"name":"payer","isMut":true,"isSigner":true}}],"args":[]}}],"accounts":[],"types":[],"events":[],"errors":[]}}"#
        );
        let document = arete_hash::CanonicalIdlDocument::parse(idl.as_bytes(), None).unwrap();
        let artifact = arete_artifacts::ProgramSpecArtifact::new(
            arete_hash::ProgramSpecV1::from_document(&document),
        )
        .unwrap();
        let hash = artifact.artifact_hash.to_string();
        (serde_json::to_value(&artifact).unwrap(), hash)
    }

    fn release_hash(marker: char) -> String {
        format!(
            "arete:registry-package-release:v2:sha256:{}",
            marker.to_string().repeat(64)
        )
    }

    fn program_install(program_id: &str, name: &str, release: &str) -> Value {
        let (program_spec, spec_hash) = program_spec_json(program_id, name);
        let idl_payload = program_spec["payload"]["idlSnapshot"].clone();
        json!({
            "installName": name,
            "displayName": name,
            "definition": {
                "programId": program_id,
                "programSpecHash": spec_hash,
                "idlContentHash": program_spec["payload"]["idlContentHash"],
                "normalizedIdlHash": program_spec["payload"]["normalizedIdlHash"],
                "idlPayload": idl_payload,
                "programSpec": program_spec,
                "extensions": null
            },
            "release": {
                "programReleaseHash": release,
                "programSpecHash": spec_hash
            },
            "transport": {
                "kind": "hosted-binding",
                "binding": {
                    "endpoint": "https://reads.example.test/private/",
                    "programReadBindingId": "prb_00000000000000000000000000000077",
                    "auth": {
                        "required": true,
                        "mode": "signed_session",
                        "sessionEndpoint": "https://api.example.test/ws/sessions",
                        "targetKind": "program-read-binding",
                        "targetId": "prb_00000000000000000000000000000077",
                        "acceptedKeyClasses": ["publishable", "secret"]
                    }
                }
            },
            "chainBinding": gateway_binding(vec!["read"], vec!["anonymous", "publishable", "secret"], false),
            "transactionBinding": gateway_binding(vec!["transaction:inspect", "transaction:send"], vec!["publishable", "secret"], true)
        })
    }

    /// Managed Solana gateway capability binding: hosted installs must carry
    /// both the chain-read and transaction descriptors.
    fn gateway_binding(
        scopes: Vec<&str>,
        accepted_key_classes: Vec<&str>,
        entitlement: bool,
    ) -> Value {
        json!({
            "endpoint": "https://solana.example.test/gateway/",
            "authPolicy": "signed_session",
            "solanaGatewayBindingId": "sgb_00000000000000000000000000000001",
            "cluster": "mainnet-beta",
            "region": "us-west-1",
            "auth": {
                "required": true,
                "mode": "signed_session",
                "sessionEndpoint": "https://api.example.test/ws/sessions",
                "jwksUrl": "https://api.example.test/.well-known/jwks.json",
                "tokenTransport": "bearer",
                "audience": "arete:solana-gateway",
                "targetKind": "solana-gateway-binding",
                "targetId": "sgb_00000000000000000000000000000001",
                "scopes": scopes,
                "acceptedKeyClasses": accepted_key_classes,
                "transactionEntitlementRequired": entitlement,
            }
        })
    }

    fn program_resolution(alias: &str, package: &str, version: &str, marker: char) -> String {
        json!({
            "resolverContract": RESOLVER_CONTRACT,
            "dependencies": [{
                "kind": "program",
                "alias": alias,
                "package": package,
                "version": version,
                "packageReleaseHash": release_hash(marker),
                "generatorContract": GENERATOR_CONTRACT,
                "install": program_install("Vote111111111111111111111111111111111111111", "vote_program", &format!("arete:h1:program-release:sha256:{}", marker.to_string().repeat(64))),
                "sdkExtensions": []
            }]
        })
        .to_string()
    }

    fn not_found(alias: &str, package: &str) -> String {
        json!({
            "error": format!("Dependency '{alias}' cannot access program package '{package}'")
        })
        .to_string()
    }

    fn request_dependency(request: &crate::api_client::test_support::ReceivedRequest) -> Value {
        let body: Value = serde_json::from_str(&request.body).expect("json body");
        body["dependencies"][0].clone()
    }

    fn lock_of(manifest: &Path) -> ProjectLock {
        ProjectLock::load_optional(manifest.with_file_name("arete.lock"))
            .unwrap()
            .expect("lock written")
    }

    #[test]
    fn saved_install_by_owner_alias_uses_the_resolver_with_a_portable_alias_and_exact_lock() {
        // Two resolver calls: version selection (`*`), then the saved `^0.1.0` install.
        let sandbox = RegistrySandbox::new(
            vec![
                (
                    200,
                    program_resolution("my-private-program", "My-Private_Program", "0.1.0", 'a'),
                ),
                (
                    200,
                    program_resolution("my-private-program", "My-Private_Program", "0.1.0", 'a'),
                ),
            ],
            true,
        );
        let manifest = sandbox.project();
        add_and_install(
            &manifest,
            DependencyKind::Program,
            "My-Private_Program",
            AddDependencyOptions::default(),
        )
        .expect("owner-private install should succeed");

        let first = sandbox.request();
        assert!(
            first
                .request_line
                .starts_with(&format!("POST {RESOLVE_PATH} ")),
            "{}",
            first.request_line
        );
        assert_eq!(
            first.header("authorization"),
            Some(format!("Bearer {OWNER_KEY}").as_str())
        );
        let dependency = request_dependency(&first);
        assert_eq!(
            dependency["package"], "My-Private_Program",
            "remote lookup is sent unchanged"
        );
        assert_eq!(
            dependency["alias"], "my-private-program",
            "local alias is normalized"
        );
        assert_eq!(dependency["requirement"], "*");
        let second = sandbox.request();
        let dependency = request_dependency(&second);
        assert_eq!(dependency["requirement"], "^0.1.0");
        assert!(
            dependency.get("lockedPackageReleaseHash").is_none()
                || dependency["lockedPackageReleaseHash"].is_null()
        );

        let toml = fs::read_to_string(&manifest).unwrap();
        assert!(
            toml.contains("[dependencies.programs.my-private-program]"),
            "{toml}"
        );
        assert!(toml.contains("registry = \"My-Private_Program\""), "{toml}");
        let lock = lock_of(&manifest);
        assert_eq!(lock.dependencies.len(), 1);
        assert_eq!(lock.dependencies[0].alias, "my-private-program");
        assert_eq!(
            lock.dependencies[0].package_release_hash.as_deref(),
            Some(release_hash('a').as_str())
        );
        assert_eq!(lock.dependencies[0].version.as_deref(), Some("0.1.0"));
        assert!(manifest
            .with_file_name("generated/typescript/programs/my-private-program")
            .is_dir());
    }

    #[test]
    fn stable_reference_and_stack_name_lookups_derive_valid_aliases() {
        let sandbox = RegistrySandbox::new(
            vec![
                (
                    200,
                    program_resolution(
                        "upr-abcdefghijklmnopqrstuvwxyz012345",
                        "upr_ABCDEFGHIJKLMNOPQRSTUVWXYZ012345",
                        "0.1.0",
                        'b',
                    ),
                ),
                (
                    200,
                    program_resolution(
                        "upr-abcdefghijklmnopqrstuvwxyz012345",
                        "upr_ABCDEFGHIJKLMNOPQRSTUVWXYZ012345",
                        "0.1.0",
                        'b',
                    ),
                ),
            ],
            true,
        );
        let manifest = sandbox.project();
        add_and_install(
            &manifest,
            DependencyKind::Program,
            "upr_ABCDEFGHIJKLMNOPQRSTUVWXYZ012345",
            AddDependencyOptions::default(),
        )
        .expect("install by stable reference should succeed");
        let dependency = request_dependency(&sandbox.request());
        assert_eq!(
            dependency["package"],
            "upr_ABCDEFGHIJKLMNOPQRSTUVWXYZ012345"
        );
        assert_eq!(dependency["alias"], "upr-abcdefghijklmnopqrstuvwxyz012345");
        let toml = fs::read_to_string(&manifest).unwrap();
        assert!(
            toml.contains("[dependencies.programs.upr-abcdefghijklmnopqrstuvwxyz012345]"),
            "{toml}"
        );
    }

    #[test]
    fn no_save_install_uses_the_same_resolver_endpoint() {
        let sandbox = RegistrySandbox::new(
            vec![(
                200,
                program_resolution("plan004-shared", "Plan004-Shared", "0.1.0", 'c'),
            )],
            true,
        );
        let output = sandbox.dir.path().join("no-save-output");
        let cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(sandbox.dir.path()).unwrap();
        let result = install_without_saving(
            DependencyKind::Program,
            "Plan004-Shared",
            NoSaveDependencyOptions {
                alias: None,
                target: Some(InstallTarget::TypeScript),
                output: Some(output.to_string_lossy().into_owned()),
                typescript_package: None,
                rust_crate_prefix: None,
                module: false,
            },
        );
        std::env::set_current_dir(cwd).unwrap();
        result.expect("--no-save install should succeed");
        let request = sandbox.request();
        assert!(request
            .request_line
            .starts_with(&format!("POST {RESOLVE_PATH} ")));
        let dependency = request_dependency(&request);
        assert_eq!(dependency["package"], "Plan004-Shared");
        assert_eq!(dependency["alias"], "plan004-shared");
        assert!(output.is_dir());
        assert!(
            !sandbox.dir.path().join("arete.toml").exists(),
            "--no-save writes no manifest"
        );
    }

    #[test]
    fn reinstall_honors_the_exact_lock_and_update_advances_it() {
        let sandbox = RegistrySandbox::new(
            vec![
                (
                    200,
                    program_resolution("plan004-shared", "plan004-shared", "0.1.0", 'd'),
                ),
                (
                    200,
                    program_resolution("plan004-shared", "plan004-shared", "0.1.0", 'd'),
                ),
                // Reinstall: the server honors the lock even though 0.1.1 exists.
                (
                    200,
                    program_resolution("plan004-shared", "plan004-shared", "0.1.0", 'd'),
                ),
                // Explicit update: unlocked request selects the newer revision.
                (
                    200,
                    program_resolution("plan004-shared", "plan004-shared", "0.1.1", 'e'),
                ),
            ],
            true,
        );
        let manifest = sandbox.project();
        add_and_install(
            &manifest,
            DependencyKind::Program,
            "plan004-shared",
            AddDependencyOptions::default(),
        )
        .unwrap();
        sandbox.request();
        sandbox.request();
        let locked_before = lock_of(&manifest);

        install_project(&manifest, InstallOptions::default()).expect("reinstall");
        let reinstall = request_dependency(&sandbox.request());
        assert_eq!(
            reinstall["lockedPackageReleaseHash"],
            release_hash('d'),
            "reinstall sends the exact lock"
        );
        assert_eq!(lock_of(&manifest), locked_before, "reinstall never floats");

        install_project(
            &manifest,
            InstallOptions {
                update: Some(UpdateSelection {
                    kind: Some(DependencyKind::Program),
                    alias: Some("plan004-shared"),
                }),
                ..InstallOptions::default()
            },
        )
        .expect("explicit update");
        let update = request_dependency(&sandbox.request());
        assert!(
            update.get("lockedPackageReleaseHash").is_none()
                || update["lockedPackageReleaseHash"].is_null(),
            "update drops the lock: {update}"
        );
        let updated = lock_of(&manifest);
        assert_eq!(updated.dependencies[0].version.as_deref(), Some("0.1.1"));
        assert_eq!(
            updated.dependencies[0].package_release_hash.as_deref(),
            Some(release_hash('e').as_str())
        );
    }

    #[test]
    fn a_direct_program_install_requires_an_exact_program_release_identity() {
        // The stack path already rejected a program without an exact release.
        // A direct program install pins the same hosted identity into the
        // generated SDK and arete.lock, so it must reject the same shapes.
        let package = "Plan004-Program";
        let alias = super::super::alias::derive_local_alias(package);
        for (label, mutate) in [
            (
                "empty program release hash",
                Box::new(|value: &mut Value| {
                    value["dependencies"][0]["install"]["release"]["programReleaseHash"] =
                        json!("   ");
                }) as Box<dyn Fn(&mut Value)>,
            ),
            (
                "release spec hash disagreeing with the definition",
                Box::new(|value: &mut Value| {
                    value["dependencies"][0]["install"]["release"]["programSpecHash"] =
                        json!("arete:h1:program-spec:sha256:{}".replace("{}", &"f".repeat(64),));
                }) as Box<dyn Fn(&mut Value)>,
            ),
        ] {
            let mut body: Value =
                serde_json::from_str(&program_resolution(&alias, package, "0.1.0", 'a')).unwrap();
            mutate(&mut body);
            let sandbox = RegistrySandbox::new(vec![(200, body.to_string())], true);
            let manifest = sandbox.project();
            let original = fs::read(&manifest).unwrap();
            let error = add_and_install(
                &manifest,
                DependencyKind::Program,
                package,
                AddDependencyOptions::default(),
            )
            .expect_err("an inexact program release must be rejected");
            let text = format!("{error:#}");
            assert!(
                text.contains("without an exact release identity"),
                "{label}: {text}"
            );
            assert_eq!(
                fs::read(&manifest).unwrap(),
                original,
                "{label}: manifest untouched"
            );
            assert!(
                !manifest.with_file_name("arete.lock").exists(),
                "{label}: no lock written"
            );
        }
    }

    #[test]
    fn reinstalling_a_dependency_reuses_its_existing_alias_instead_of_duplicating() {
        // The derived default alias changed: it now lower-cases and separates
        // on non-alphanumeric runs, where the old default took the last path
        // segment. A project written before that change stores the old key,
        // and reinstalling must find it rather than adding a second entry.
        let package = "owner/vote-program";
        let legacy_alias = "vote-program"; // what the previous default produced
        let derived = super::super::alias::derive_local_alias(package);
        assert_ne!(
            legacy_alias, derived,
            "fixture is only meaningful if the derived alias changed"
        );

        let sandbox = RegistrySandbox::new(
            vec![
                (200, program_resolution(legacy_alias, package, "0.1.0", 'a')),
                (200, program_resolution(legacy_alias, package, "0.1.0", 'a')),
            ],
            true,
        );
        let manifest = sandbox.project();
        let mut document: toml_edit::DocumentMut =
            fs::read_to_string(&manifest).unwrap().parse().unwrap();
        document["dependencies"]["programs"][legacy_alias]["source"]["registry"] =
            toml_edit::value(package);
        document["dependencies"]["programs"][legacy_alias]["version"] = toml_edit::value("^0.1.0");
        fs::write(&manifest, document.to_string()).unwrap();

        add_and_install(
            &manifest,
            DependencyKind::Program,
            package,
            AddDependencyOptions::default(),
        )
        .expect("reinstall of an existing dependency succeeds");

        let after: toml_edit::DocumentMut = fs::read_to_string(&manifest).unwrap().parse().unwrap();
        let programs = after["dependencies"]["programs"]
            .as_table_like()
            .expect("programs table");
        assert!(
            programs.contains_key(legacy_alias),
            "the existing alias is kept"
        );
        assert!(
            !programs.contains_key(derived.as_str()),
            "reinstall must not add a second entry under the newly derived alias"
        );
        assert_eq!(programs.len(), 1, "exactly one dependency entry");
    }

    #[test]
    fn an_explicit_alias_cannot_install_an_already_declared_package_twice() {
        // Duplicate detection is keyed on the package, not the alias, so
        // --alias cannot smuggle a second copy of a dependency into the
        // project under a different local name.
        let package = "Plan004-Shared";
        let declared_alias = super::super::alias::derive_local_alias(package);
        let sandbox = RegistrySandbox::new(
            vec![
                (
                    200,
                    program_resolution(&declared_alias, package, "0.1.0", 'a'),
                ),
                (
                    200,
                    program_resolution(&declared_alias, package, "0.1.0", 'a'),
                ),
            ],
            true,
        );
        let manifest = sandbox.project();
        add_and_install(
            &manifest,
            DependencyKind::Program,
            package,
            AddDependencyOptions::default(),
        )
        .expect("first install succeeds");
        let after_first = fs::read(&manifest).unwrap();

        let error = add_and_install(
            &manifest,
            DependencyKind::Program,
            package,
            AddDependencyOptions {
                alias: Some("something-else".into()),
                ..AddDependencyOptions::default()
            },
        )
        .expect_err("a second alias for the same package must be refused");
        let text = format!("{error:#}");
        assert!(text.contains("already declares"), "{text}");
        assert!(text.contains(&declared_alias), "{text}");
        assert_eq!(
            fs::read(&manifest).unwrap(),
            after_first,
            "the refused install leaves the manifest untouched"
        );

        let document: toml_edit::DocumentMut =
            fs::read_to_string(&manifest).unwrap().parse().unwrap();
        let programs = document["dependencies"]["programs"]
            .as_table_like()
            .expect("programs table");
        assert_eq!(programs.len(), 1, "exactly one dependency entry");
    }

    #[test]
    fn a_legacy_non_portable_alias_never_produces_a_duplicate_dependency() {
        // Aliases were not validated before, so a project can hold one that is
        // not a legal identifier in every generated language. Whatever the
        // outcome, reinstalling must never leave the project with two entries
        // for the same package.
        let package = "Plan004-Shared";
        let legacy_alias = "Plan004-Shared";
        let derived = super::super::alias::derive_local_alias(package);
        assert!(
            super::super::alias::validate_local_alias(legacy_alias, DependencyKind::Program)
                .is_err()
        );

        let sandbox = RegistrySandbox::new(
            vec![
                (200, program_resolution(legacy_alias, package, "0.1.0", 'a')),
                (200, program_resolution(legacy_alias, package, "0.1.0", 'a')),
            ],
            true,
        );
        let manifest = sandbox.project();
        let mut document: toml_edit::DocumentMut =
            fs::read_to_string(&manifest).unwrap().parse().unwrap();
        document["dependencies"]["programs"][legacy_alias]["source"]["registry"] =
            toml_edit::value(package);
        document["dependencies"]["programs"][legacy_alias]["version"] = toml_edit::value("^0.1.0");
        fs::write(&manifest, document.to_string()).unwrap();

        let _ = add_and_install(
            &manifest,
            DependencyKind::Program,
            package,
            AddDependencyOptions::default(),
        );

        let after: toml_edit::DocumentMut = fs::read_to_string(&manifest).unwrap().parse().unwrap();
        let programs = after["dependencies"]["programs"]
            .as_table_like()
            .expect("programs table");
        assert!(
            !(programs.contains_key(legacy_alias) && programs.contains_key(derived.as_str())),
            "a legacy alias must never end up alongside a newly derived duplicate"
        );
    }

    #[test]
    fn unknown_and_cross_owner_not_found_are_identical_and_never_fall_back() {
        let messages = ["Plan004-Shared", "Does-Not_Exist"].map(|package| {
            let alias = super::super::alias::derive_local_alias(package);
            let sandbox = RegistrySandbox::new(vec![(404, not_found(&alias, package))], true);
            let manifest = sandbox.project();
            let original = fs::read(&manifest).unwrap();
            let error = add_and_install(
                &manifest,
                DependencyKind::Program,
                package,
                AddDependencyOptions::default(),
            )
            .expect_err("404 must fail");
            // Exactly one resolver request and no direct/public endpoint fallback.
            let request = sandbox.request();
            assert!(request
                .request_line
                .starts_with(&format!("POST {RESOLVE_PATH} ")));
            assert_eq!(fs::read(&manifest).unwrap(), original, "manifest untouched");
            assert!(
                !manifest.with_file_name("arete.lock").exists(),
                "no lock written"
            );
            let text = format!("{error:#}");
            assert!(
                text.contains("unavailable to this account or unknown"),
                "{text}"
            );
            assert!(
                !text.contains("registry-package-release") && !text.contains("0.1."),
                "no package metadata leaks: {text}"
            );
            text.replace(&alias, "<alias>")
                .replace(package, "<package>")
        });
        assert_eq!(
            messages[0], messages[1],
            "unknown and cross-owner errors are indistinguishable"
        );
    }

    #[test]
    fn unauthenticated_lookup_of_a_private_package_asks_for_login() {
        let sandbox = RegistrySandbox::new(
            vec![(401, json!({"error": "Authentication required"}).to_string())],
            false,
        );
        let manifest = sandbox.project();
        let error = add_and_install(
            &manifest,
            DependencyKind::Program,
            "Plan004-Shared",
            AddDependencyOptions::default(),
        )
        .expect_err("401 must fail");
        let request = sandbox.request();
        assert!(request.header("authorization").is_none());
        let text = format!("{error:#}");
        assert!(text.contains("a4 auth login"), "{text}");
        assert!(!text.contains(OWNER_KEY));
    }

    #[test]
    fn lock_integrity_failures_do_not_float_to_latest() {
        let sandbox = RegistrySandbox::new(
            vec![
                (200, program_resolution("plan004-shared", "plan004-shared", "0.1.0", 'f')),
                (200, program_resolution("plan004-shared", "plan004-shared", "0.1.0", 'f')),
                (409, json!({"error": "Dependency 'plan004-shared': locked package release hash does not exist for this package"}).to_string()),
            ],
            true,
        );
        let manifest = sandbox.project();
        add_and_install(
            &manifest,
            DependencyKind::Program,
            "plan004-shared",
            AddDependencyOptions::default(),
        )
        .unwrap();
        sandbox.request();
        sandbox.request();
        let lock_before = fs::read(manifest.with_file_name("arete.lock")).unwrap();
        let error =
            install_project(&manifest, InstallOptions::default()).expect_err("integrity failure");
        sandbox.request();
        let text = format!("{error:#}");
        assert!(text.contains("integrity failure"), "{text}");
        assert!(text.contains("a4 update"), "{text}");
        assert_eq!(
            fs::read(manifest.with_file_name("arete.lock")).unwrap(),
            lock_before,
            "lock unchanged"
        );
    }

    #[test]
    fn incomplete_stack_responses_are_rejected_and_leave_the_project_intact() {
        let stack_manifest = json!({
            "kind": "stack-manifest",
            "artifactVersion": "2",
            "artifactHash": format!("arete:h1:stack-manifest:sha256:{}", "9".repeat(64)),
            "payload": {
                "schema": "arete.stack-manifest/v2",
                "name": "Demo",
                "programs": [
                    {"programId": "Vote111111111111111111111111111111111111111", "artifactHash": "arete:h1:program-spec:sha256:one"},
                    {"programId": "Stake11111111111111111111111111111111111111", "artifactHash": "arete:h1:program-spec:sha256:two"}
                ],
                "liveSpecs": [],
                "selectedViews": []
            }
        });
        let incomplete = json!({
            "resolverContract": RESOLVER_CONTRACT,
            "dependencies": [{
                "kind": "stack",
                "alias": "demo-stack-a1b2",
                "package": "Demo-Stack-a1b2",
                "version": "0.1.0",
                "packageReleaseHash": release_hash('9'),
                "generatorContract": GENERATOR_CONTRACT,
                "stackManifestHash": stack_manifest["artifactHash"],
                "stackManifest": stack_manifest,
                "liveSpecs": [],
                // Only one of the two declared programs is pinned: floating.
                "programs": [program_install("Vote111111111111111111111111111111111111111", "vote_program", &format!("arete:h1:program-release:sha256:{}", "9".repeat(64)))],
                "sdkExtensions": []
            }]
        })
        .to_string();
        let sandbox = RegistrySandbox::new(vec![(200, incomplete)], true);
        let manifest = sandbox.project();
        let original = fs::read(&manifest).unwrap();
        let error = add_and_install(
            &manifest,
            DependencyKind::Stack,
            "Demo-Stack-a1b2",
            AddDependencyOptions::default(),
        )
        .expect_err("incomplete stack must be rejected");
        sandbox.request();
        let text = format!("{error:#}");
        assert!(text.contains("incomplete stack package"), "{text}");
        assert_eq!(
            fs::read(&manifest).unwrap(),
            original,
            "manifest restored byte-for-byte"
        );
        assert!(!manifest.with_file_name("arete.lock").exists());
        assert!(!manifest.with_file_name("generated").exists());
    }

    const HOSTED_WS: &str = "wss://ore.stack.example.test";
    const HOSTED_HTTP: &str = "https://ore.stack.example.test";

    fn ore_fixture(name: &str) -> Value {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("cli crate lives in the repo root")
            .join("stacks/ore/.arete")
            .join(name);
        serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
    }

    /// The exact release descriptor the resolver returns for one of the Ore
    /// fixture's programs.
    fn fixture_program_install(name: &str, program_spec: Value, marker: char) -> Value {
        let spec_hash = program_spec["artifactHash"].clone();
        let mut install = program_install(
            program_spec["payload"]["programId"].as_str().unwrap(),
            name,
            &format!(
                "arete:h1:program-release:sha256:{}",
                marker.to_string().repeat(64)
            ),
        );
        install["definition"]["programSpecHash"] = spec_hash.clone();
        install["definition"]["idlContentHash"] = program_spec["payload"]["idlContentHash"].clone();
        install["definition"]["normalizedIdlHash"] =
            program_spec["payload"]["normalizedIdlHash"].clone();
        install["definition"]["idlPayload"] = program_spec["payload"]["idlSnapshot"].clone();
        install["definition"]["programSpec"] = program_spec;
        install["release"]["programSpecHash"] = spec_hash;
        install
    }

    fn hosted_delivery(websocket: &str, query: &str, generation: i64) -> Value {
        let live = ore_fixture("OreStream.live-spec.json");
        json!({
            "mode": "hosted",
            "deploymentReleaseHash": format!("arete:h1:deployment-release:sha256:{}", "d".repeat(64)),
            "liveBindings": [{
                "alias": "live",
                "liveSpecHash": live["artifactHash"],
                "binding": {
                    "deploymentId": 7,
                    "websocketEndpoint": websocket,
                    "queryEndpoint": query,
                    "websocketAuthPolicy": "signed_session",
                    "queryAuthPolicy": "signed_session",
                    "observedGeneration": generation
                }
            }],
            "chainBinding": gateway_binding(vec!["read"], vec!["anonymous", "publishable", "secret"], false),
            "transactionBinding": gateway_binding(vec!["transaction:inspect", "transaction:send"], vec!["publishable", "secret"], true)
        })
    }

    /// The Ore fixture as a resolved registry stack, with `delivery` exactly
    /// as given (absent when `None`).
    fn ore_stack_dependency(delivery: Option<Value>) -> Value {
        let manifest = ore_fixture("OreStream.stack-manifest.json");
        let live = ore_fixture("OreStream.live-spec.json");
        let mut dependency = json!({
            "kind": "stack",
            "alias": "ore",
            "package": "ore",
            "version": "1.0.0",
            "packageReleaseHash": release_hash('5'),
            "generatorContract": GENERATOR_CONTRACT,
            "stackManifestHash": manifest["artifactHash"],
            "stackManifest": manifest,
            "liveSpecs": [{"alias": "live", "artifactHash": live["artifactHash"], "artifact": live}],
            "programs": [
                fixture_program_install("ore", ore_fixture("ore.program-spec.json"), 'a'),
                fixture_program_install("entropy", ore_fixture("entropy.program-spec.json"), 'b')
            ],
            "sdkExtensions": []
        });
        if let Some(delivery) = delivery {
            dependency["delivery"] = delivery;
        }
        dependency
    }

    fn resolution(dependencies: Vec<Value>) -> String {
        json!({"resolverContract": RESOLVER_CONTRACT, "dependencies": dependencies}).to_string()
    }

    /// A project that already declares the Ore stack for every SDK target.
    fn stack_project(sandbox: &RegistrySandbox, extra: &str) -> PathBuf {
        let root = sandbox.dir.path().join("stack-project");
        fs::create_dir_all(&root).unwrap();
        let manifest = root.join("arete.toml");
        fs::write(
            &manifest,
            format!(
                r#"manifest_version = 1

[project]
name = "hosted-install"

[sdk]
targets = ["typescript", "rust", "python"]

[dependencies.stacks.ore]
source = {{ registry = "ore" }}
version = "^1.0.0"
{extra}"#
            ),
        )
        .unwrap();
        manifest
    }

    /// Every generated text file for one target, by path relative to the
    /// target's output root.
    fn generated_files(manifest: &Path, target: &str) -> BTreeMap<String, String> {
        fn collect(root: &Path, path: &Path, files: &mut BTreeMap<String, String>) {
            for entry in fs::read_dir(path).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    collect(root, &path, files);
                } else if let Ok(contents) = fs::read_to_string(&path) {
                    files.insert(
                        path.strip_prefix(root).unwrap().display().to_string(),
                        contents,
                    );
                }
            }
        }
        let root = manifest.with_file_name("generated").join(target);
        let mut files = BTreeMap::new();
        collect(&root, &root, &mut files);
        files
    }

    /// Every generated file for one target, concatenated.
    fn generated_text(manifest: &Path, target: &str) -> String {
        generated_files(manifest, target)
            .into_values()
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Whether the generated TypeScript stack definition itself carries a
    /// managed gateway (program modules carry their own read bindings).
    fn has_stack_gateway(manifest: &Path) -> bool {
        generated_files(manifest, "typescript")
            .get("stacks/ore/ore-core.ts")
            .expect("stack definition")
            .lines()
            .any(|line| line.starts_with("  gateway: "))
    }

    fn assert_project_untouched(manifest: &Path, original: &[u8]) {
        assert_eq!(fs::read(manifest).unwrap(), original, "manifest unchanged");
        assert!(!manifest.with_file_name("arete.lock").exists(), "no lock");
        assert!(
            !manifest.with_file_name("generated").exists(),
            "no generated output"
        );
    }

    #[test]
    fn hosted_stack_install_generates_exact_endpoints_for_every_target() {
        let sandbox = RegistrySandbox::new(
            vec![(
                200,
                resolution(vec![ore_stack_dependency(Some(hosted_delivery(
                    HOSTED_WS,
                    HOSTED_HTTP,
                    4,
                )))]),
            )],
            false,
        );
        let manifest = stack_project(&sandbox, "");
        install_project(&manifest, InstallOptions::default()).expect("hosted install");

        let request = sandbox.request();
        assert!(
            request
                .request_line
                .starts_with(&format!("POST {RESOLVE_PATH} ")),
            "{}",
            request.request_line
        );
        let stack_manifest = ore_fixture("OreStream.stack-manifest.json");
        let manifest_hash = stack_manifest["artifactHash"].as_str().unwrap();
        let live_hash = ore_fixture("OreStream.live-spec.json")["artifactHash"]
            .as_str()
            .unwrap()
            .to_string();
        for target in ["typescript", "rust", "python"] {
            let text = generated_text(&manifest, target);
            assert!(text.contains(HOSTED_WS), "{target}: websocket endpoint");
            assert!(text.contains(HOSTED_HTTP), "{target}: query endpoint");
            assert!(
                text.contains("https://solana.example.test/gateway/"),
                "{target}: gateway endpoint"
            );
            assert!(
                text.contains("https://api.example.test/ws/sessions"),
                "{target}: session endpoint"
            );
            assert!(
                text.contains("https://api.example.test/.well-known/jwks.json"),
                "{target}: JWKS"
            );
            assert!(text.contains(manifest_hash), "{target}: StackManifest");
            for marker in ['a', 'b'] {
                let release = format!(
                    "arete:h1:program-release:sha256:{}",
                    marker.to_string().repeat(64)
                );
                assert!(text.contains(&release), "{target}: program release");
            }
            assert!(!text.contains("TODO: Set"), "{target}: placeholder");
            assert!(!text.contains("ws: ''"), "{target}: empty ws");
            assert!(!text.contains("http: ''"), "{target}: empty http");
            assert!(!text.contains("ws=\"\""), "{target}: empty ws");
        }
        assert!(has_stack_gateway(&manifest), "hosted stack gateway");
        let locked = lock_of(&manifest);
        assert_eq!(
            locked.dependencies[0].live_specs[0].artifact_hash,
            live_hash
        );
        assert_eq!(
            locked.dependencies[0].stack_manifest_hash.as_deref(),
            Some(manifest_hash)
        );
        let lock = fs::read_to_string(manifest.with_file_name("arete.lock")).unwrap();
        for transport in [
            "ore.stack.example.test",
            "solana.example.test",
            "deployment-release",
        ] {
            assert!(!lock.contains(transport), "lock carries {transport}");
        }
    }

    #[test]
    fn definition_only_stack_install_keeps_placeholders_and_no_gateway() {
        let sandbox = RegistrySandbox::new(
            vec![(
                200,
                resolution(vec![ore_stack_dependency(Some(
                    json!({"mode": "definition-only"}),
                ))]),
            )],
            false,
        );
        let manifest = stack_project(&sandbox, "");
        install_project(&manifest, InstallOptions::default()).expect("definition-only install");
        let typescript = generated_text(&manifest, "typescript");
        assert!(typescript.contains("ws: '', // TODO"), "{typescript}");
        assert!(
            !has_stack_gateway(&manifest),
            "a definition-only stack exposes no hosted gateway"
        );
        assert!(generated_text(&manifest, "rust").contains("TODO: Set URL"));
        assert!(generated_text(&manifest, "python").contains("ws=\"\""));
    }

    const MINE_WS: &str = "wss://mine.example.test";
    const MINE_HTTP: &str = "https://mine.example.test";

    fn mine_endpoints() -> BTreeMap<String, StackEndpointsV1> {
        BTreeMap::from([(
            "live".to_string(),
            StackEndpointsV1 {
                websocket: MINE_WS.into(),
                query: MINE_HTTP.into(),
            },
        )])
    }

    fn endpoints_line(live: &str) -> String {
        format!(
            "endpoints = {{ {live} = {{ websocket = \"{MINE_WS}\", query = \"{MINE_HTTP}\" }} }}\n"
        )
    }

    fn definition_only_resolution() -> (u16, String) {
        (
            200,
            resolution(vec![ore_stack_dependency(Some(
                json!({"mode": "definition-only"}),
            ))]),
        )
    }

    #[test]
    fn a_definition_only_stack_reads_the_deployment_arete_toml_records() {
        let sandbox = RegistrySandbox::new(vec![definition_only_resolution()], false);
        let manifest = stack_project(&sandbox, &endpoints_line("live"));
        install_project(&manifest, InstallOptions::default()).expect("install");
        for target in ["typescript", "rust", "python"] {
            let text = generated_text(&manifest, target);
            assert!(text.contains(MINE_WS), "{target}: websocket endpoint");
            assert!(!text.contains("TODO: Set"), "{target}: placeholder");
        }
        let typescript = generated_text(&manifest, "typescript");
        assert!(
            typescript.contains(&format!("ws: '{MINE_WS}'")),
            "{typescript}"
        );
        assert!(
            typescript.contains(&format!("http: '{MINE_HTTP}'")),
            "{typescript}"
        );
        assert!(
            !has_stack_gateway(&manifest),
            "a deployment of a definition-only stack adds no managed gateway"
        );
    }

    #[test]
    fn recorded_endpoints_replace_a_hosted_stream_and_keep_its_gateway() {
        let sandbox = RegistrySandbox::new(
            vec![(
                200,
                resolution(vec![ore_stack_dependency(Some(hosted_delivery(
                    HOSTED_WS,
                    HOSTED_HTTP,
                    4,
                )))]),
            )],
            false,
        );
        let manifest = stack_project(&sandbox, &endpoints_line("live"));
        install_project(&manifest, InstallOptions::default()).expect("install");
        let typescript = generated_text(&manifest, "typescript");
        assert!(
            typescript.contains(&format!("ws: '{MINE_WS}'")),
            "{typescript}"
        );
        assert!(
            typescript.contains(&format!("http: '{MINE_HTTP}'")),
            "{typescript}"
        );
        assert!(has_stack_gateway(&manifest), "the managed gateway stays");
        for target in ["rust", "python"] {
            assert!(
                generated_text(&manifest, target).contains(MINE_WS),
                "{target}: websocket endpoint"
            );
        }
    }

    #[test]
    fn recorded_endpoints_must_name_every_live_spec() {
        let sandbox = RegistrySandbox::new(vec![definition_only_resolution()], false);
        let manifest = stack_project(&sandbox, &endpoints_line("other"));
        let original = fs::read(&manifest).unwrap();
        let error = install_project(&manifest, InstallOptions::default())
            .expect_err("endpoints for an unknown LiveSpec");
        assert!(
            format!("{error:#}").contains("its StackManifest has LiveSpecs [live]"),
            "{error:#}"
        );
        assert_project_untouched(&manifest, &original);
    }

    #[test]
    fn stack_endpoints_are_recorded_in_place_and_installed() {
        let sandbox = RegistrySandbox::new(
            vec![definition_only_resolution(), definition_only_resolution()],
            false,
        );
        let manifest = stack_project(&sandbox, "");
        let written = fs::read_to_string(&manifest).unwrap();
        fs::write(&manifest, format!("# kept as written\n{written}")).unwrap();
        install_project(&manifest, InstallOptions::default()).expect("install");

        assert!(record_stack_endpoints_and_install(&manifest, "ore", mine_endpoints()).unwrap());
        let text = fs::read_to_string(&manifest).unwrap();
        assert!(text.starts_with("# kept as written\n"), "{text}");
        let project = ProjectManifest::load(&manifest).unwrap();
        assert_eq!(
            project.document.dependencies.stacks["ore"].endpoints,
            mine_endpoints()
        );
        assert!(lock_of(&manifest).is_fresh(&project.manifest_hash));
        assert!(generated_text(&manifest, "typescript").contains(&format!("ws: '{MINE_WS}'")));

        // Recording the same deployment again needs no install.
        assert!(!record_stack_endpoints_and_install(&manifest, "ore", mine_endpoints()).unwrap());
        // Rewriting the dependency entry (as installing it again does) keeps
        // the recorded deployment.
        let rendered = render_manifest_addition(
            text.as_bytes(),
            DependencyKind::Stack,
            "ore",
            &project.document.dependencies.stacks["ore"],
            None,
            None,
        )
        .unwrap();
        assert!(rendered.contains(MINE_WS), "{rendered}");
    }

    #[test]
    fn a_stack_manifest_change_under_a_recorded_deployment_asks_for_a_redeploy() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("arete.toml");
        fs::write(
            &path,
            format!(
                "manifest_version = 1\n[project]\nname = \"notes\"\n\n\
                 [dependencies.stacks.ore]\nsource = {{ registry = \"ore\" }}\nversion = \"^1.0.0\"\n{}\n\
                 [dependencies.stacks.plain]\nsource = {{ registry = \"plain\" }}\nversion = \"^1.0.0\"\n",
                endpoints_line("live")
            ),
        )
        .unwrap();
        let manifest = ProjectManifest::load(&path).unwrap();
        let entry = |alias: &str, marker: char| LockedDependency {
            kind: DependencyKind::Stack,
            alias: alias.into(),
            source: format!("registry:{alias}"),
            requirement: Some("^1.0.0".into()),
            version: Some("1.0.0".into()),
            package_release_hash: Some(format!(
                "arete:registry-package-release:v2:sha256:{}",
                marker.to_string().repeat(64)
            )),
            stack_manifest_hash: Some(format!(
                "arete:h1:stack-manifest:sha256:{}",
                marker.to_string().repeat(64)
            )),
            program_id: None,
            program_spec_hash: None,
            program_release_hash: None,
            live_specs: Vec::new(),
            programs: Vec::new(),
            parts: Vec::new(),
            sdk_extension_hashes: Vec::new(),
            targets: vec![InstallTarget::TypeScript],
            generator_contract: GENERATOR_CONTRACT.into(),
        };
        let lock = |ore: char, plain: char| {
            let mut lock = ProjectLock::empty(manifest.manifest_hash.clone());
            lock.dependencies = vec![entry("ore", ore), entry("plain", plain)];
            lock
        };

        assert!(redeploy_notes(&manifest, None, &lock('a', 'a')).is_empty());
        assert!(
            redeploy_notes(&manifest, Some(&lock('a', 'a')), &lock('a', 'b')).is_empty(),
            "a stack without a recorded deployment needs no redeploy"
        );
        let notes = redeploy_notes(&manifest, Some(&lock('a', 'a')), &lock('b', 'a'));
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(notes[0].contains("stack 'ore'"), "{}", notes[0]);
        assert!(notes[0].contains("Run `a4 up ore`"), "{}", notes[0]);

        // An authored stack of the same name is what `a4 up ore` deploys.
        let mut shadowed = manifest.clone();
        shadowed.document.authoring.stacks.insert(
            "ore".into(),
            toml::from_str("manifest = \"./.arete/Ore.stack-manifest.json\"").unwrap(),
        );
        let notes = redeploy_notes(&shadowed, Some(&lock('a', 'a')), &lock('b', 'a'));
        assert!(!notes[0].contains("Run `a4 up ore`"), "{}", notes[0]);
        assert!(
            notes[0].contains("[authoring.stacks] entry 'ore'"),
            "{}",
            notes[0]
        );
    }

    #[test]
    fn a_failed_endpoint_install_leaves_arete_toml_as_it_was() {
        let sandbox = RegistrySandbox::new(
            vec![
                definition_only_resolution(),
                (409, json!({"error": "conflict"}).to_string()),
            ],
            false,
        );
        let manifest = stack_project(&sandbox, "");
        install_project(&manifest, InstallOptions::default()).expect("install");
        let original = fs::read(&manifest).unwrap();
        record_stack_endpoints_and_install(&manifest, "ore", mine_endpoints())
            .expect_err("the resolver refused");
        assert_eq!(fs::read(&manifest).unwrap(), original);
    }

    #[test]
    fn a_stack_without_a_delivery_mode_is_refused_before_anything_is_written() {
        let sandbox = RegistrySandbox::new(
            vec![(200, resolution(vec![ore_stack_dependency(None)]))],
            false,
        );
        let manifest = stack_project(&sandbox, "");
        let original = fs::read(&manifest).unwrap();
        let error = install_project(&manifest, InstallOptions::default())
            .expect_err("a registry that ignored include=delivery must be refused");
        sandbox.request();
        assert!(
            format!("{error:#}").contains("does not support stack delivery resolution"),
            "{error:#}"
        );
        assert_project_untouched(&manifest, &original);
    }

    #[test]
    fn malformed_hosted_bindings_are_refused_before_anything_is_written() {
        let hosted = || hosted_delivery(HOSTED_WS, HOSTED_HTTP, 4);
        let mut cases: Vec<(&str, Value, &str)> = Vec::new();
        let mut partial = hosted();
        partial["liveBindings"][0]["binding"]
            .as_object_mut()
            .unwrap()
            .remove("queryEndpoint");
        cases.push(("partial binding", partial, "queryEndpoint"));
        let mut blank = hosted();
        blank["liveBindings"][0]["binding"]["websocketEndpoint"] = json!(" ");
        cases.push(("blank endpoint", blank, "incomplete"));
        let mut blank_policy = hosted();
        blank_policy["liveBindings"][0]["binding"]["queryAuthPolicy"] = json!("");
        cases.push(("blank policy", blank_policy, "incomplete"));
        let mut deployment = hosted();
        deployment["liveBindings"][0]["binding"]["deploymentId"] = json!(0);
        cases.push(("non-positive deployment", deployment, "incomplete"));
        let mut generation = hosted();
        generation["liveBindings"][0]["binding"]["observedGeneration"] = json!(0);
        cases.push(("non-positive generation", generation, "incomplete"));
        let mut reordered = hosted();
        reordered["liveBindings"][0]["alias"] = json!("other");
        cases.push(("wrong alias", reordered, "alias/order mismatch"));
        let mut mismatched = hosted();
        mismatched["liveBindings"][0]["liveSpecHash"] =
            json!(format!("arete:h1:live-spec:sha256:{}", "0".repeat(64)));
        cases.push(("wrong hash", mismatched, "hash mismatch"));
        let mut duplicate = hosted();
        let binding = duplicate["liveBindings"][0].clone();
        duplicate["liveBindings"]
            .as_array_mut()
            .unwrap()
            .push(binding);
        cases.push(("duplicate binding", duplicate, "do not exactly cover"));
        let mut missing = hosted();
        missing["liveBindings"] = json!([]);
        cases.push(("no binding", missing, "do not exactly cover"));
        let mut half_gateway = hosted();
        half_gateway["transactionBinding"] = Value::Null;
        cases.push((
            "partial gateway",
            half_gateway,
            "only one managed Solana gateway",
        ));
        let mut no_gateway = hosted();
        no_gateway["chainBinding"] = Value::Null;
        no_gateway["transactionBinding"] = Value::Null;
        cases.push(("no gateway", no_gateway, "omitted managed Solana gateway"));
        let mut blank_gateway = hosted();
        blank_gateway["chainBinding"]["endpoint"] = json!(" ");
        cases.push((
            "blank gateway endpoint",
            blank_gateway,
            "chain binding with no endpoint",
        ));
        let mut relative_gateway = hosted();
        relative_gateway["transactionBinding"]["endpoint"] = json!("/gateway/");
        cases.push((
            "relative gateway endpoint",
            relative_gateway,
            "endpoint is not an absolute HTTP(S) URL",
        ));
        let mut blank_jwks = hosted();
        blank_jwks["chainBinding"]["auth"]["jwksUrl"] = json!("");
        cases.push(("blank gateway JWKS", blank_jwks, "no auth.jwksUrl"));
        let mut foreign_target = hosted();
        foreign_target["transactionBinding"]["auth"]["targetId"] = json!("sgb_other");
        cases.push((
            "foreign gateway target",
            foreign_target,
            "session target does not name the binding",
        ));
        let mut release = hosted();
        release["deploymentReleaseHash"] = json!("");
        cases.push(("blank release", release, "no exact deployment release"));
        let mut unknown = hosted();
        unknown["websocketUrl"] = json!(HOSTED_WS);
        cases.push(("unknown field", unknown, "websocketUrl"));
        cases.push((
            "unknown mode",
            json!({"mode": "self-hosted"}),
            "self-hosted",
        ));

        for (case, delivery, expected) in cases {
            let sandbox = RegistrySandbox::new(
                vec![(200, resolution(vec![ore_stack_dependency(Some(delivery))]))],
                false,
            );
            let manifest = stack_project(&sandbox, "");
            let original = fs::read(&manifest).unwrap();
            let error = install_project(&manifest, InstallOptions::default()).expect_err(case);
            sandbox.request();
            assert!(format!("{error:#}").contains(expected), "{case}: {error:#}");
            assert_project_untouched(&manifest, &original);
        }
    }

    #[test]
    fn locked_reinstall_refreshes_transport_without_touching_the_lock() {
        let moved_ws = "wss://ore.moved.example.test";
        let moved_http = "https://ore.moved.example.test";
        let sandbox = RegistrySandbox::new(
            vec![
                (
                    200,
                    resolution(vec![ore_stack_dependency(Some(hosted_delivery(
                        HOSTED_WS,
                        HOSTED_HTTP,
                        4,
                    )))]),
                ),
                (
                    200,
                    resolution(vec![ore_stack_dependency(Some(hosted_delivery(
                        moved_ws, moved_http, 5,
                    )))]),
                ),
            ],
            false,
        );
        let manifest = stack_project(&sandbox, "");
        install_project(&manifest, InstallOptions::default()).expect("first install");
        sandbox.request();
        let lock_path = manifest.with_file_name("arete.lock");
        let lock_before = fs::read(&lock_path).unwrap();
        assert!(generated_text(&manifest, "typescript").contains(HOSTED_WS));

        install_project(
            &manifest,
            InstallOptions {
                locked: true,
                ..InstallOptions::default()
            },
        )
        .expect("locked reinstall");
        let second = sandbox.request();
        assert_eq!(
            request_dependency(&second)["lockedPackageReleaseHash"],
            release_hash('5')
        );
        assert_eq!(fs::read(&lock_path).unwrap(), lock_before, "lock unchanged");
        for target in ["typescript", "rust", "python"] {
            let text = generated_text(&manifest, target);
            assert!(text.contains(moved_ws), "{target}: refreshed endpoint");
            assert!(!text.contains(HOSTED_WS), "{target}: stale endpoint");
        }
    }

    #[test]
    fn stacks_and_programs_resolve_with_delivery_in_one_batch_request() {
        let program =
            serde_json::from_str::<Value>(&program_resolution("vote", "vote", "0.1.0", 'c'))
                .unwrap()["dependencies"][0]
                .clone();
        let sandbox = RegistrySandbox::new(
            vec![(
                200,
                resolution(vec![
                    ore_stack_dependency(Some(hosted_delivery(HOSTED_WS, HOSTED_HTTP, 4))),
                    program,
                ]),
            )],
            false,
        );
        let manifest = stack_project(
            &sandbox,
            "\n[dependencies.programs.vote]\nsource = { registry = \"vote\" }\nversion = \"^0.1.0\"\n",
        );
        install_project(&manifest, InstallOptions::default()).expect("batch install");
        let request = sandbox.request();
        assert!(
            request
                .request_line
                .starts_with(&format!("POST {RESOLVE_PATH} ")),
            "{}",
            request.request_line
        );
        let body: Value = serde_json::from_str(&request.body).unwrap();
        assert_eq!(
            body["dependencies"]
                .as_array()
                .unwrap()
                .iter()
                .map(|dependency| (dependency["kind"].clone(), dependency["alias"].clone()))
                .collect::<Vec<_>>(),
            vec![
                (json!("stack"), json!("ore")),
                (json!("program"), json!("vote"))
            ]
        );
        assert_eq!(lock_of(&manifest).dependencies.len(), 2);
    }

    #[test]
    fn delivery_not_ready_is_not_reported_as_a_lock_integrity_failure() {
        let error = describe_resolver_error(
            crate::api_client::ApiHttpError {
                status: 409,
                status_text: "409 Conflict".into(),
                message: "Stack 'ore' is not currently ready".into(),
                code: Some(DELIVERY_NOT_READY.into()),
            }
            .into(),
            DependencyKind::Stack,
            "ore",
            true,
        );
        let text = format!("{error:#}");
        assert!(text.contains("not currently ready"), "{text}");
        assert!(!text.contains("integrity"), "{text}");
    }

    #[test]
    fn explicit_alias_must_be_portable_before_anything_is_written() {
        // The canned response must never be consumed: alias validation fails first.
        let sandbox = RegistrySandbox::new(
            vec![(500, json!({"error": "must not be called"}).to_string())],
            true,
        );
        let manifest = sandbox.project();
        let original = fs::read(&manifest).unwrap();
        for alias in ["Default", "class", "1abc", "bad--alias"] {
            let error = add_and_install(
                &manifest,
                DependencyKind::Program,
                "plan004-shared",
                AddDependencyOptions {
                    alias: Some(alias.into()),
                    ..AddDependencyOptions::default()
                },
            )
            .expect_err("invalid alias must fail before resolution");
            assert!(format!("{error:#}").contains("alias"), "{error:#}");
        }
        assert_eq!(fs::read(&manifest).unwrap(), original);
    }

    // ---------------------------------------------------------------------
    // Program SDKs carried by stacks: identity, extensions, and overlap.
    // ---------------------------------------------------------------------

    const ORE_PROGRAM_ID: &str = "oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv";
    const VOTE_PROGRAM_ID: &str = "Vote111111111111111111111111111111111111111";

    fn ore_program_spec_hash() -> String {
        ore_fixture("ore.program-spec.json")["artifactHash"]
            .as_str()
            .unwrap()
            .to_string()
    }

    /// One SDK extension as the resolver returns it: `{ target, contentHash,
    /// artifact }` with the artifact's hash equal to the content hash.
    fn sdk_extension(
        target: &str,
        marker: char,
        input: (&str, &str),
        entry: &str,
        contents: &str,
        language: Option<&str>,
    ) -> Value {
        let hash = marker.to_string().repeat(64);
        let mut manifest = json!({
            "entry": entry,
            "files": [entry],
            "inputKind": input.0,
            "inputHash": input.1,
            "sdkRange": null
        });
        if let Some(language) = language {
            manifest["language"] = json!(language);
        }
        json!({
            "target": target,
            "contentHash": hash,
            "artifact": {
                "artifactHash": hash,
                "manifest": manifest,
                "files": { entry: contents },
                "createdAt": "2026-09-25T00:00:00Z"
            }
        })
    }

    const ORE_PROGRAM_EXTENSION: &str = "import { defineProgramExtensions } from '@usearete/sdk';\nimport type { ORE } from './ore-core.js';\n\nexport default defineProgramExtensions<typeof ORE>()({\n  createOperations: () => ({\n    transactions: {\n      mining: {\n        deployWithCheckpoint: {},\n      },\n    },\n  }),\n});\n";

    fn ore_program_extension(target: &str, marker: char) -> Value {
        let spec = ore_program_spec_hash();
        let (entry, language) = match target {
            "rust" => ("extensions.rs", Some("rust")),
            "python" => ("extensions.py", Some("python")),
            _ => ("ore-extensions.ts", None),
        };
        sdk_extension(
            target,
            marker,
            ("program-spec", &spec),
            entry,
            ORE_PROGRAM_EXTENSION,
            language,
        )
    }

    /// The Ore stack whose `ore` program references the `ore` program
    /// package release `marker`, with that release's extensions.
    fn ore_stack_with_program_sdk(marker: char, version: &str, extensions: Vec<Value>) -> Value {
        let mut dependency = ore_stack_dependency(Some(hosted_delivery(HOSTED_WS, HOSTED_HTTP, 4)));
        let ore = &mut dependency["programs"][0];
        ore["installName"] = json!("ore");
        ore["programPackage"] = json!({
            "package": "ore",
            "version": version,
            "packageReleaseHash": release_hash(marker)
        });
        ore["sdkExtensions"] = json!(extensions);
        dependency
    }

    /// The standalone `ore` program package release `marker`, exactly as the
    /// stack's reference resolves it.
    fn ore_program_dependency(marker: char, version: &str, extensions: Vec<Value>) -> Value {
        json!({
            "kind": "program",
            "alias": "ore",
            "package": "ore",
            "version": version,
            "packageReleaseHash": release_hash(marker),
            "generatorContract": GENERATOR_CONTRACT,
            "install": fixture_program_install("ore", ore_fixture("ore.program-spec.json"), 'a'),
            "sdkExtensions": extensions
        })
    }

    fn typescript_project(sandbox: &RegistrySandbox, dependencies: &str) -> PathBuf {
        let root = sandbox.dir.path().join("typescript-project");
        fs::create_dir_all(&root).unwrap();
        let manifest = root.join("arete.toml");
        fs::write(
            &manifest,
            format!(
                "manifest_version = 1\n\n[project]\nname = \"program-sdks\"\n\n[sdk]\ntargets = [\"typescript\"]\n{dependencies}"
            ),
        )
        .unwrap();
        manifest
    }

    const ORE_STACK_DEPENDENCY: &str =
        "\n[dependencies.stacks.ore]\nsource = { registry = \"ore\" }\nversion = \"^1.0.0\"\n";
    const ORE_PROGRAM_DEPENDENCY: &str =
        "\n[dependencies.programs.ore]\nsource = { registry = \"ore\" }\nversion = \"^1.0.0\"\n";

    #[test]
    fn a_referenced_program_sdk_is_generated_with_its_extension_identity_and_lock() {
        let sandbox = RegistrySandbox::new(
            vec![(
                200,
                resolution(vec![ore_stack_with_program_sdk(
                    '7',
                    "1.0.2",
                    vec![ore_program_extension("typescript", 'e')],
                )]),
            )],
            false,
        );
        let manifest = stack_project(&sandbox, "");
        install_project(&manifest, InstallOptions::default()).expect("install");
        let request = sandbox.request();
        assert!(
            request
                .request_line
                .starts_with("POST /api/registry/v1/resolve?include=delivery,program-sdks "),
            "{}",
            request.request_line
        );

        let typescript = generated_files(&manifest, "typescript");
        let entry = &typescript["stacks/ore/programs/ore/__arete-program.ts"];
        assert!(
            entry.contains("export const ORE_PROGRAM = withProgramIdentity(\n  withProgramRead(\n    extendProgram(ORE_PROGRAM_CORE, programExtensions),"),
            "{entry}"
        );
        assert!(entry.contains("import programExtensions from './ore-extensions.js';"));
        // The semantic operation path: the stack's `programs.ore` is the
        // program SDK module, whose extension defines the operation.
        let stack_entry = &typescript["stacks/ore/ore.ts"];
        assert!(
            stack_entry
                .contains("import hostedOreProgram from './programs/ore/__arete-program.js';"),
            "{stack_entry}"
        );
        assert!(
            stack_entry.contains("    ore: hostedOreProgram,"),
            "{stack_entry}"
        );
        assert!(typescript["stacks/ore/programs/ore/ore-extensions.ts"]
            .contains("transactions: {\n      mining: {\n        deployWithCheckpoint"));
        // Identity: the program package release, in every generated language.
        // TypeScript stamps it on the finished program SDK in its entry,
        // after the package's own extension; core definitions carry none.
        let identity = format!("{{ packageReleaseHash: '{}' }}", release_hash('7'));
        assert!(entry.contains(&identity), "{entry}");
        for core in [
            "stacks/ore/programs/ore/ore-core.ts",
            "stacks/ore/ore-core.ts",
            "stacks/ore/programs/entropy/entropy-core.ts",
        ] {
            assert!(!typescript[core].contains("packageReleaseHash"), "{core}");
        }
        // A program without a package release has no identity to stamp.
        assert!(
            !typescript["stacks/ore/programs/entropy/__arete-program.ts"]
                .contains("packageReleaseHash")
        );
        assert!(generated_text(&manifest, "rust").contains(&format!(
            "pub const PACKAGE_RELEASE_HASH: &str = \"{}\";",
            release_hash('7')
        )));
        let python = generated_text(&manifest, "python");
        assert!(python.contains(&format!(
            "ORE_PACKAGE_RELEASE_HASH = \"{}\"",
            release_hash('7')
        )));
        assert!(python.contains("    package_release_hash=ORE_PACKAGE_RELEASE_HASH,\n"));
        assert!(!python.contains("ENTROPY_PACKAGE_RELEASE_HASH"));
        let provenance: Value =
            serde_json::from_str(&typescript["stacks/ore/sdk-provenance.json"]).unwrap();
        assert!(
            provenance["programExtensions"]["ore"].is_object(),
            "{provenance}"
        );

        // The lock records the program SDK release and its extension, and
        // round-trips through its canonical form.
        let lock = lock_of(&manifest);
        let programs = &lock.dependencies[0].programs;
        let ore = programs
            .iter()
            .find(|program| program.program_id == ORE_PROGRAM_ID)
            .unwrap();
        assert_eq!(
            ore.package_release_hash.as_deref(),
            Some(release_hash('7').as_str())
        );
        assert_eq!(ore.sdk_extension_hashes, vec!["e".repeat(64)]);
        let entropy = programs
            .iter()
            .find(|program| program.program_id != ORE_PROGRAM_ID)
            .unwrap();
        assert_eq!(entropy.package_release_hash, None);
        let text = fs::read_to_string(manifest.with_file_name("arete.lock")).unwrap();
        assert!(text.contains(&format!("package_release_hash = \"{}\"", release_hash('7'))));
        let mut reparsed: ProjectLock = toml::from_str(&text).unwrap();
        reparsed.normalize_and_validate().unwrap();
        assert_eq!(reparsed, lock);
    }

    #[test]
    fn a_locked_install_refuses_a_moved_program_sdk_release() {
        let first = resolution(vec![ore_stack_with_program_sdk('7', "1.0.2", vec![])]);
        let moved = resolution(vec![ore_stack_with_program_sdk('8', "1.0.3", vec![])]);
        let sandbox = RegistrySandbox::new(
            vec![(200, first), (200, moved.clone()), (200, moved)],
            false,
        );
        let manifest = stack_project(&sandbox, "");
        install_project(&manifest, InstallOptions::default()).expect("first install");
        sandbox.request();
        let lock_path = manifest.with_file_name("arete.lock");
        let before = fs::read(&lock_path).unwrap();

        for locked in [true, false] {
            let error = install_project(
                &manifest,
                InstallOptions {
                    locked,
                    ..InstallOptions::default()
                },
            )
            .expect_err("the stack release's program SDK reference moved");
            sandbox.request();
            let text = format!("{error:#}");
            assert!(text.contains("integrity failure"), "{text}");
            assert!(text.contains(&release_hash('8')), "{text}");
            assert_eq!(fs::read(&lock_path).unwrap(), before, "lock unchanged");
        }
    }

    #[test]
    fn a_stack_and_a_standalone_install_of_one_release_generate_identical_program_sdks() {
        let extension = || vec![ore_program_extension("typescript", 'e')];
        let sandbox = RegistrySandbox::new(
            vec![(
                200,
                resolution(vec![
                    ore_stack_with_program_sdk('7', "1.0.2", extension()),
                    ore_program_dependency('7', "1.0.2", extension()),
                ]),
            )],
            false,
        );
        let manifest = typescript_project(
            &sandbox,
            &format!("{ORE_STACK_DEPENDENCY}{ORE_PROGRAM_DEPENDENCY}"),
        );
        install_project(&manifest, InstallOptions::default()).expect("install both");
        let files = generated_files(&manifest, "typescript");
        for (in_stack, standalone) in [
            (
                "stacks/ore/programs/ore/__arete-program.ts",
                "programs/ore/ore.ts",
            ),
            (
                "stacks/ore/programs/ore/ore-core.ts",
                "programs/ore/ore-core.ts",
            ),
            (
                "stacks/ore/programs/ore/ore-extensions.ts",
                "programs/ore/ore-extensions.ts",
            ),
            (
                "stacks/ore/programs/ore/extensions.json",
                "programs/ore/extensions.json",
            ),
        ] {
            assert_eq!(
                files[in_stack], files[standalone],
                "{in_stack} and {standalone} must be byte-identical"
            );
        }
        assert!(files["programs/ore/ore.ts"].contains(&format!(
            "{{ packageReleaseHash: '{}' }}",
            release_hash('7')
        )));
        let lock = lock_of(&manifest);
        assert_eq!(lock.dependencies.len(), 2);
    }

    #[test]
    fn a_standalone_program_sdk_carries_its_package_release_in_every_language() {
        let sandbox = RegistrySandbox::new(
            vec![(
                200,
                resolution(vec![ore_program_dependency('7', "1.0.2", vec![])]),
            )],
            false,
        );
        let root = sandbox.dir.path().join("program-project");
        fs::create_dir_all(&root).unwrap();
        let manifest = root.join("arete.toml");
        fs::write(
            &manifest,
            format!(
                "manifest_version = 1\n\n[project]\nname = \"program-identity\"\n\n[sdk]\ntargets = [\"typescript\", \"rust\", \"python\"]\n{ORE_PROGRAM_DEPENDENCY}"
            ),
        )
        .unwrap();
        install_project(&manifest, InstallOptions::default()).expect("program install");
        let release = release_hash('7');
        assert!(generated_text(&manifest, "typescript")
            .contains(&format!("{{ packageReleaseHash: '{release}' }}")));
        let rust = generated_text(&manifest, "rust");
        assert!(rust.contains(&format!(
            "    fn package_release_hash() -> Option<&'static str> {{\n        Some(\"{release}\")\n    }}"
        )), "{rust}");
        assert!(rust.contains(&format!(
            "pub const PACKAGE_RELEASE_HASH: &str = \"{release}\";"
        )));
        let python = generated_text(&manifest, "python");
        assert!(python.contains(&format!("ORE_PACKAGE_RELEASE_HASH = \"{release}\"")));
        assert!(python.contains("    package_release_hash=ORE_PACKAGE_RELEASE_HASH,\n"));
    }

    fn resolved_registry(dependency: Value, kind: DependencyKind) -> ResolvedProjectDependency {
        ResolvedProjectDependency::Registry {
            kind,
            source: "registry:ore".into(),
            requirement: "^1.0.0".into(),
            targets: vec![InstallTarget::TypeScript],
            resolved: Box::new(serde_json::from_value(dependency).unwrap()),
        }
    }

    #[test]
    fn the_install_report_names_shared_program_sdks_and_different_releases() {
        let same = vec![
            resolved_registry(
                ore_stack_with_program_sdk('7', "1.0.2", vec![]),
                DependencyKind::Stack,
            ),
            resolved_registry(
                ore_program_dependency('7', "1.0.2", vec![]),
                DependencyKind::Program,
            ),
        ];
        let shared = shared_program_sdks(&same);
        assert_eq!(shared.len(), 1);
        assert!(shared[0].same_release);
        assert_eq!(
            describe_shared(&shared[0]),
            "Shared:      program ore@1.0.2 is also provided by stack ore (the same program SDK release, generated identically in both)"
        );

        let different = vec![
            resolved_registry(
                ore_stack_with_program_sdk('6', "1.0.1", vec![]),
                DependencyKind::Stack,
            ),
            resolved_registry(
                ore_program_dependency('7', "1.0.2", vec![]),
                DependencyKind::Program,
            ),
        ];
        let shared = shared_program_sdks(&different);
        assert_eq!(shared.len(), 1);
        assert!(!shared[0].same_release);
        assert_eq!(
            describe_shared(&shared[0]),
            "Note:        stack ore provides program ore@1.0.1, a different release than program ore@1.0.2; both are generated and stay separate"
        );

        // A legacy stack embeds the core program, with no program SDK release.
        let legacy = vec![
            resolved_registry(
                ore_stack_dependency(Some(hosted_delivery(HOSTED_WS, HOSTED_HTTP, 4))),
                DependencyKind::Stack,
            ),
            resolved_registry(
                ore_program_dependency('7', "1.0.2", vec![]),
                DependencyKind::Program,
            ),
        ];
        let shared = shared_program_sdks(&legacy);
        assert!(!shared[0].same_release);
        assert!(describe_shared(&shared[0]).contains("its own core SDK"));
    }

    #[test]
    fn the_install_report_json_separates_requested_regenerated_shared_and_auth() {
        let stack = resolved_registry(
            ore_stack_with_program_sdk('7', "1.0.2", vec![]),
            DependencyKind::Stack,
        );
        let program = resolved_registry(
            ore_program_dependency('7', "1.0.2", vec![]),
            DependencyKind::Program,
        );
        let resolved = vec![stack, program];
        let locked =
            |alias: &str, kind: DependencyKind, version: &str, marker: char| LockedDependency {
                kind,
                alias: alias.into(),
                source: format!("registry:{alias}"),
                requirement: Some("^1.0.0".into()),
                version: Some(version.into()),
                package_release_hash: Some(release_hash(marker)),
                stack_manifest_hash: (kind == DependencyKind::Stack)
                    .then(|| format!("arete:h1:stack-manifest:sha256:{}", "1".repeat(64))),
                program_id: (kind == DependencyKind::Program).then(|| ORE_PROGRAM_ID.into()),
                program_spec_hash: (kind == DependencyKind::Program).then(ore_program_spec_hash),
                program_release_hash: None,
                live_specs: Vec::new(),
                programs: Vec::new(),
                parts: Vec::new(),
                sdk_extension_hashes: Vec::new(),
                targets: vec![InstallTarget::TypeScript],
                generator_contract: GENERATOR_CONTRACT.into(),
            };
        let manifest_hash = format!("arete-manifest-v1:{:064x}", 3);
        let mut previous = ProjectLock::empty(manifest_hash.clone());
        previous.dependencies = vec![locked("ore", DependencyKind::Stack, "1.0.1", '5')];
        let mut next = ProjectLock::empty(manifest_hash);
        next.dependencies = vec![
            locked("ore", DependencyKind::Stack, "1.0.1", '5'),
            locked("ore", DependencyKind::Program, "1.0.2", '7'),
        ];
        let requested = [(DependencyKind::Program, "ore".to_string())];
        let report = InstallReport {
            lockfile: "arete.lock".into(),
            installed: 2,
            requested: Vec::new(),
            regenerated: Vec::new(),
            shared: shared_program_sdks(&resolved),
            auth: Vec::new(),
            runtime: runtime::typescript_runtime_set(&BTreeSet::from(["react".to_string()])),
            notes: Vec::new(),
        }
        .with_dependencies(&requested, Some(&previous), &next)
        .with_auth(&requested, &resolved);
        let value = serde_json::to_value(&report).unwrap();
        assert_eq!(value["requested"][0]["kind"], "program");
        assert_eq!(value["requested"][0]["alias"], "ore");
        assert_eq!(value["requested"][0]["version"], "1.0.2");
        assert_eq!(value["regenerated"][0]["kind"], "stack");
        assert_eq!(value["regenerated"][0]["reason"], "inputs unchanged");
        assert_eq!(value["shared"][0]["sameRelease"], true);
        assert_eq!(value["shared"][0]["stack"], "ore");
        // A program request reports no stack's auth; a stack request does.
        assert_eq!(value["auth"], json!([]));
        let version = env!("CARGO_PKG_VERSION");
        assert_eq!(
            value["runtime"],
            json!([
                {"package": "@usearete/sdk", "version": version},
                {"package": "@usearete/react", "version": version},
                {"package": "zod", "version": runtime::ZOD_RANGE},
            ])
        );

        let stack_request = [(DependencyKind::Stack, "ore".to_string())];
        let report = InstallReport {
            lockfile: "arete.lock".into(),
            installed: 2,
            requested: Vec::new(),
            regenerated: Vec::new(),
            shared: Vec::new(),
            auth: Vec::new(),
            runtime: Vec::new(),
            notes: Vec::new(),
        }
        .with_auth(&stack_request, &resolved);
        let auth = serde_json::to_value(&report).unwrap()["auth"][0].clone();
        assert_eq!(auth["stack"], "ore");
        assert_eq!(
            auth["liveViews"][0]["websocketAuthPolicy"],
            "signed_session"
        );
        assert_eq!(
            auth["transactions"]["acceptedKeyClasses"],
            json!(["publishable", "secret"])
        );
        assert_eq!(
            auth["transactions"]["scopes"],
            json!(["transaction:inspect", "transaction:send"])
        );
        assert_eq!(auth["transactions"]["transactionEntitlementRequired"], true);
        assert_eq!(auth["chain"]["transactionEntitlementRequired"], false);
        assert!(auth["browser"]
            .as_str()
            .unwrap()
            .contains("publishable key"));
        assert!(auth["readiness"].as_str().unwrap().contains("a4 doctor"));
    }

    #[test]
    fn regeneration_reasons_name_what_changed() {
        let base = LockedDependency {
            kind: DependencyKind::Stack,
            alias: "ore".into(),
            source: "registry:ore".into(),
            requirement: Some("^1.0.0".into()),
            version: Some("1.0.1".into()),
            package_release_hash: Some(release_hash('1')),
            stack_manifest_hash: Some(format!("arete:h1:stack-manifest:sha256:{}", "1".repeat(64))),
            program_id: None,
            program_spec_hash: None,
            program_release_hash: None,
            live_specs: Vec::new(),
            programs: Vec::new(),
            parts: Vec::new(),
            sdk_extension_hashes: Vec::new(),
            targets: vec![InstallTarget::TypeScript],
            generator_contract: GENERATOR_CONTRACT.into(),
        };
        assert_eq!(regeneration_reason(None, &base), "newly locked");
        assert_eq!(regeneration_reason(Some(&base), &base), "inputs unchanged");
        let mut advanced = base.clone();
        advanced.version = Some("1.0.2".into());
        advanced.package_release_hash = Some(release_hash('2'));
        assert_eq!(
            regeneration_reason(Some(&base), &advanced),
            "resolved 1.0.2, was 1.0.1"
        );
        let mut retargeted = base.clone();
        retargeted.targets = vec![InstallTarget::TypeScript, InstallTarget::Rust];
        assert_eq!(
            regeneration_reason(Some(&base), &retargeted),
            "targets changed"
        );
    }

    #[test]
    fn a_program_sdk_extension_rust_stacks_cannot_include_fails_loudly() {
        let sandbox = RegistrySandbox::new(
            vec![(
                200,
                resolution(vec![ore_stack_with_program_sdk(
                    '7',
                    "1.0.2",
                    vec![
                        ore_program_extension("typescript", 'e'),
                        ore_program_extension("rust", 'f'),
                    ],
                )]),
            )],
            false,
        );
        let manifest = stack_project(&sandbox, "");
        let original = fs::read(&manifest).unwrap();
        let error = install_project(&manifest, InstallOptions::default())
            .expect_err("a Rust program SDK extension cannot be dropped silently");
        sandbox.request();
        let text = format!("{error:#}");
        assert!(
            text.contains("cannot include per-program extensions yet"),
            "{text}"
        );
        assert!(text.contains("a4 install program ore --rust"), "{text}");
        assert_project_untouched(&manifest, &original);
    }

    #[test]
    fn a_program_sdk_extension_for_an_unrequested_target_is_refused() {
        let sandbox = RegistrySandbox::new(
            vec![(
                200,
                resolution(vec![ore_stack_with_program_sdk(
                    '7',
                    "1.0.2",
                    vec![ore_program_extension("rust", 'f')],
                )]),
            )],
            false,
        );
        let manifest = typescript_project(&sandbox, ORE_STACK_DEPENDENCY);
        let original = fs::read(&manifest).unwrap();
        let error = install_project(&manifest, InstallOptions::default())
            .expect_err("the registry answered a target nobody asked for");
        sandbox.request();
        assert!(
            format!("{error:#}").contains("unrequested 'rust' SDK extension for program 'ore'"),
            "{error:#}"
        );
        assert_project_untouched(&manifest, &original);
    }

    /// A StackManifest grouping `live_aliases` (each the Ore LiveSpec) with
    /// `programs`, as a resolved registry stack with hosted delivery.
    fn composed_ore_stack(
        name: &str,
        live_aliases: &[&str],
        programs: Vec<(Value, Value)>,
        sdk_extensions: Vec<Value>,
    ) -> Value {
        let live_value = ore_fixture("OreStream.live-spec.json");
        let live: arete_artifacts::LiveSpecArtifactV2 =
            serde_json::from_value(live_value.clone()).unwrap();
        let program_specs = programs
            .iter()
            .map(|(spec, _)| serde_json::from_value(spec.clone()).unwrap())
            .collect::<Vec<arete_artifacts::ProgramSpecArtifact>>();
        let views = [
            "OreRound/latest",
            "OreRound/state",
            "OreBoard/state",
            "OreMiner/state",
        ];
        let manifest = arete_artifacts::compose_stack_manifest_v2(
            name,
            &program_specs,
            live_aliases
                .iter()
                .map(|alias| (alias.to_string(), &live))
                .collect(),
            live_aliases
                .iter()
                .flat_map(|alias| {
                    views.iter().map(|view| arete_artifacts::SelectedViewV2 {
                        live_alias: alias.to_string(),
                        view_id: view.to_string(),
                    })
                })
                .collect(),
        )
        .unwrap();
        let mut delivery = hosted_delivery(HOSTED_WS, HOSTED_HTTP, 4);
        let binding = delivery["liveBindings"][0].clone();
        delivery["liveBindings"] = json!(live_aliases
            .iter()
            .enumerate()
            .map(|(position, alias)| {
                let mut binding = binding.clone();
                binding["alias"] = json!(alias);
                binding["binding"]["deploymentId"] = json!(7 + position);
                binding["binding"]["websocketEndpoint"] =
                    json!(format!("wss://{alias}.stack.example.test"));
                binding
            })
            .collect::<Vec<_>>());
        json!({
            "kind": "stack",
            "alias": "ore",
            "package": "ore",
            "version": "1.0.0",
            "packageReleaseHash": release_hash('5'),
            "generatorContract": GENERATOR_CONTRACT,
            "stackManifestHash": manifest.artifact_hash.to_string(),
            "stackManifest": serde_json::to_value(&manifest).unwrap(),
            "liveSpecs": live_aliases
                .iter()
                .map(|alias| json!({"alias": alias, "artifactHash": live_value["artifactHash"], "artifact": live_value}))
                .collect::<Vec<_>>(),
            "programs": programs.into_iter().map(|(_, install)| install).collect::<Vec<_>>(),
            "sdkExtensions": sdk_extensions,
            "delivery": delivery
        })
    }

    fn ore_programs_with_sdk(marker: char) -> Vec<(Value, Value)> {
        let mut ore = fixture_program_install("ore", ore_fixture("ore.program-spec.json"), 'a');
        ore["programPackage"] = json!({
            "package": "ore",
            "version": "1.0.2",
            "packageReleaseHash": release_hash(marker)
        });
        ore["sdkExtensions"] = json!([ore_program_extension("typescript", 'e')]);
        vec![
            (ore_fixture("ore.program-spec.json"), ore),
            (
                ore_fixture("entropy.program-spec.json"),
                fixture_program_install("entropy", ore_fixture("entropy.program-spec.json"), 'b'),
            ),
        ]
    }

    #[test]
    fn a_single_live_stack_generates_its_independent_programs_for_every_target() {
        let (vote_spec, _) = program_spec_json(VOTE_PROGRAM_ID, "vote_program");
        let mut programs = ore_programs_with_sdk('7');
        programs.push((
            vote_spec,
            program_install(
                VOTE_PROGRAM_ID,
                "vote_program",
                &format!("arete:h1:program-release:sha256:{}", "c".repeat(64)),
            ),
        ));
        let stack = composed_ore_stack("OreWithVote", &["live"], programs, vec![]);
        let sandbox = RegistrySandbox::new(vec![(200, resolution(vec![stack]))], false);
        let manifest = stack_project(&sandbox, "");
        install_project(&manifest, InstallOptions::default())
            .expect("a single-live stack with an independent program installs");
        let typescript = generated_files(&manifest, "typescript");
        assert!(typescript.contains_key("stacks/ore/programs/vote-program/__arete-program.ts"));
        assert!(typescript["stacks/ore/ore.ts"].contains("voteProgram: hostedVoteProgramProgram,"));
        assert!(typescript["stacks/ore/programs/ore/__arete-program.ts"].contains("extendProgram("));
        for target in ["rust", "python"] {
            assert!(
                generated_text(&manifest, target).contains(VOTE_PROGRAM_ID),
                "{target}: independent program"
            );
        }
        assert_eq!(lock_of(&manifest).dependencies[0].programs.len(), 3);
    }

    const COMPOSITION_EXTENSION: &str = "import { defineProgramExtensions, defineStackExtensions } from '@usearete/sdk';\nimport type { ORE_MULTI_SESSION_DEFINITION_CORE } from './ore.js';\n\nexport const alphaStackExtensions = defineStackExtensions<typeof ORE_MULTI_SESSION_DEFINITION_CORE.stacks.alpha>()({\n  constants: { board: 25 },\n});\n\nexport const entropyProgramExtensions = defineProgramExtensions<typeof ORE_MULTI_SESSION_DEFINITION_CORE.programs.entropy>()({\n  constants: { tag: 'entropy' },\n});\n";

    fn composition_extension(manifest_hash: &str, contents: &str) -> Value {
        sdk_extension(
            "typescript",
            'd',
            ("stack-manifest", manifest_hash),
            "ore-extensions.ts",
            contents,
            None,
        )
    }

    #[test]
    fn a_multi_live_stack_keeps_program_sdk_extensions_and_applies_a_stack_extension() {
        let mut stack = composed_ore_stack(
            "OreMulti",
            &["alpha", "beta"],
            ore_programs_with_sdk('7'),
            vec![],
        );
        let manifest_hash = stack["stackManifestHash"].as_str().unwrap().to_string();
        stack["sdkExtensions"] =
            json!([composition_extension(&manifest_hash, COMPOSITION_EXTENSION)]);
        let sandbox = RegistrySandbox::new(vec![(200, resolution(vec![stack]))], false);
        let manifest = typescript_project(&sandbox, ORE_STACK_DEPENDENCY);
        install_project(&manifest, InstallOptions::default()).expect("multi-live install");

        let files = generated_files(&manifest, "typescript");
        assert!(files["stacks/ore/programs/ore/__arete-program.ts"]
            .contains("extendProgram(ORE_PROGRAM_CORE, programExtensions)"));
        let session = &files["stacks/ore/ore.ts"];
        for expected in [
            "import { createSession, extendPrograms, extendStack, type CompositionSessionOptions } from '@usearete/sdk';",
            "import OreProgramSdk from './programs/ore/__arete-program.js';",
            "import EntropyProgramSdk from './programs/entropy/__arete-program.js';",
            "import { alphaStackExtensions, entropyProgramExtensions } from './ore-extensions.js';",
            "export const ORE_MULTI_SESSION_DEFINITION_CORE = {",
            "    alpha: { ...AlphaStack, programs: { ...AlphaStack.programs, ore: OreProgramSdk, entropy: EntropyProgramSdk } },",
            "    ore: OreProgramSdk,",
            "    alpha: extendStack(ORE_MULTI_SESSION_DEFINITION_CORE.stacks.alpha, alphaStackExtensions),",
            "  programs: extendPrograms(ORE_MULTI_SESSION_DEFINITION_CORE.programs, {\n    entropy: entropyProgramExtensions,\n  }),",
            "export type OreMultiSessionDefinition = typeof ORE_MULTI_SESSION_DEFINITION;",
        ] {
            assert!(session.contains(expected), "missing {expected}\n{session}");
        }
        assert!(files.contains_key("stacks/ore/ore-extensions.ts"));
        let provenance: Value =
            serde_json::from_str(&files["stacks/ore/sdk-provenance.json"]).unwrap();
        assert!(
            provenance["programExtensions"]["ore"].is_object(),
            "{provenance}"
        );
        assert_eq!(
            provenance["extensions"]["contentSha256"]
                .as_str()
                .map(str::len),
            Some(64),
            "{provenance}"
        );
    }

    #[test]
    fn a_stack_extension_cannot_extend_a_program_that_brings_its_own_extension() {
        let contents = "import { defineProgramExtensions } from '@usearete/sdk';\nimport type { ORE_MULTI_SESSION_DEFINITION_CORE } from './ore.js';\n\nexport const oreProgramExtensions = defineProgramExtensions<typeof ORE_MULTI_SESSION_DEFINITION_CORE.programs.ore>()({});\n";
        let mut stack = composed_ore_stack(
            "OreMulti",
            &["alpha", "beta"],
            ore_programs_with_sdk('7'),
            vec![],
        );
        let manifest_hash = stack["stackManifestHash"].as_str().unwrap().to_string();
        stack["sdkExtensions"] = json!([composition_extension(&manifest_hash, contents)]);
        let sandbox = RegistrySandbox::new(vec![(200, resolution(vec![stack]))], false);
        let manifest = typescript_project(&sandbox, ORE_STACK_DEPENDENCY);
        let original = fs::read(&manifest).unwrap();
        let error = install_project(&manifest, InstallOptions::default())
            .expect_err("the program would be extended twice");
        sandbox.request();
        assert!(format!("{error:#}").contains("extended twice"), "{error:#}");
        assert_project_untouched(&manifest, &original);
    }

    // ---------------------------------------------------------------------
    // Composed stacks: registry parts, lock, generation and delivery.
    // ---------------------------------------------------------------------

    const TOKEN_PROGRAM_ID: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";

    /// A program package release `marker` of `package`, whose program is
    /// `name` at `program_id`.
    fn program_package(
        package: &str,
        program_id: &str,
        name: &str,
        marker: char,
        version: &str,
    ) -> Value {
        let mut install = program_install(
            program_id,
            name,
            &format!(
                "arete:h1:program-release:sha256:{}",
                marker.to_string().repeat(64)
            ),
        );
        install["installName"] = json!(package);
        json!({
            "kind": "program",
            "alias": crate::project::alias::derive_local_alias(package),
            "package": package,
            "version": version,
            "packageReleaseHash": release_hash(marker),
            "generatorContract": GENERATOR_CONTRACT,
            "install": install,
            "sdkExtensions": []
        })
    }

    fn token_program(marker: char) -> Value {
        program_package("spl-token", TOKEN_PROGRAM_ID, "spl_token", marker, "4.0.1")
    }

    /// The ore stack (hosted, its `ore` program at program SDK release `ore`
    /// with semantic operations) and the spl-token program package release
    /// `token`, as the resolver answers the composition below.
    fn composed_resolution(ore: char, token: char) -> String {
        resolution(vec![
            ore_stack_with_program_sdk(
                ore,
                "1.0.2",
                vec![ore_program_extension("typescript", 'e')],
            ),
            token_program(token),
        ])
    }

    const COMPOSED_STACK: &str = r#"
[authoring.stacks.ore-plus-token]
live.ore = { stack = "ore", version = "^1", views = ["OreRound/latest", "OreMiner/state"] }
programs = [{ package = "spl-token", version = "^4" }]

[dependencies.stacks.ore-plus-token]
source = { workspace = "ore-plus-token" }
targets = ["typescript"]
"#;

    fn source_manifest_hash() -> String {
        ore_fixture("OreStream.stack-manifest.json")["artifactHash"]
            .as_str()
            .unwrap()
            .to_string()
    }

    fn request_body(request: &crate::api_client::test_support::ReceivedRequest) -> Value {
        serde_json::from_str(&request.body).expect("json body")
    }

    fn live_part(alias: &str, stack: Value) -> composition::LivePart {
        composition::LivePart {
            alias: alias.into(),
            views: None,
            source: composition::LivePartSource::Registry {
                requirement: "^1".into(),
                live_alias: None,
                resolved: serde_json::from_value(stack).unwrap(),
            },
        }
    }

    #[test]
    fn a_composition_of_stack_views_and_a_program_sdk_resolves_locks_and_generates() {
        let sandbox = RegistrySandbox::new(vec![(200, composed_resolution('7', '8'))], false);
        let manifest = typescript_project(&sandbox, COMPOSED_STACK);
        install_project(&manifest, InstallOptions::default()).expect("composed install");

        // One resolver batch carries every registry part.
        let request = request_body(&sandbox.request());
        let parts = request["dependencies"].as_array().unwrap();
        assert_eq!(parts.len(), 2, "{request}");
        assert_eq!(
            (
                &parts[0]["kind"],
                &parts[0]["package"],
                &parts[0]["requirement"]
            ),
            (&json!("stack"), &json!("ore"), &json!("^1"))
        );
        assert_eq!(
            (
                &parts[1]["kind"],
                &parts[1]["package"],
                &parts[1]["requirement"]
            ),
            (&json!("program"), &json!("spl-token"), &json!("^4"))
        );

        let files = generated_files(&manifest, "typescript");
        let root = "stacks/ore-plus-token";
        let entry = &files[&format!("{root}/ore-plus-token.ts")];
        assert!(entry.contains("    ore: hostedOreProgram,"), "{entry}");
        assert!(
            entry.contains("    splToken: hostedSplTokenProgram,"),
            "{entry}"
        );
        // `programs.ore` is the ORE program SDK the ore stack references,
        // with its semantic operations.
        assert!(files[&format!("{root}/programs/ore/__arete-program.ts")]
            .contains("extendProgram(ORE_PROGRAM_CORE, programExtensions)"));
        assert!(files[&format!("{root}/programs/ore/ore-extensions.ts")]
            .contains("deployWithCheckpoint"));
        assert!(
            files[&format!("{root}/programs/ore/__arete-program.ts")].contains(&format!(
                "{{ packageReleaseHash: '{}' }}",
                release_hash('7')
            ))
        );
        // `programs.splToken` is the spl-token program package release.
        assert!(
            files[&format!("{root}/programs/spl-token/__arete-program.ts")].contains(&format!(
                "{{ packageReleaseHash: '{}' }}",
                release_hash('8')
            ))
        );
        // The live views read the ore stack's hosted deployment, and each
        // session names the version that deployment serves.
        let core = &files[&format!("{root}/ore-plus-token-core.ts")];
        assert!(core.contains(&format!("ws: '{HOSTED_WS}'")), "{core}");
        assert!(core.contains(&format!("http: '{HOSTED_HTTP}'")), "{core}");
        assert!(
            core.contains(&format!(
                "  release: {{\n    stackManifestHash: '{}',\n    liveAlias: 'live',\n  }},",
                source_manifest_hash()
            )),
            "{core}"
        );
        assert!(core.lines().any(|line| line.starts_with("  gateway: ")));
        assert!(core.contains("OreRound/latest") && core.contains("OreMiner/state"));
        assert!(
            !core.contains("OreTreasury/state"),
            "only the selected views"
        );

        let lock = lock_of(&manifest);
        let locked = &lock.dependencies[0];
        assert_eq!(locked.alias, "ore-plus-token");
        assert_eq!(locked.source, "workspace:ore-plus-token");
        let composed_hash = locked.stack_manifest_hash.clone().unwrap();
        assert_ne!(composed_hash, source_manifest_hash());
        assert_eq!(locked.live_specs.len(), 1);
        assert_eq!(locked.live_specs[0].alias, "ore");
        let program = |id: &str| {
            locked
                .programs
                .iter()
                .find(|program| program.program_id == id)
                .unwrap()
        };
        assert_eq!(
            program(ORE_PROGRAM_ID).package_release_hash,
            Some(release_hash('7'))
        );
        assert_eq!(
            program(ORE_PROGRAM_ID).sdk_extension_hashes,
            vec!["e".repeat(64)]
        );
        assert_eq!(
            program(TOKEN_PROGRAM_ID).package_release_hash,
            Some(release_hash('8'))
        );
        assert_eq!(locked.programs.len(), 3, "ore, entropy and spl-token");
        let stack_part = &locked.parts[0];
        assert_eq!(stack_part.source, "registry:ore");
        assert_eq!(stack_part.live.as_deref(), Some("ore"));
        assert_eq!(stack_part.requirement.as_deref(), Some("^1"));
        assert_eq!(stack_part.version.as_deref(), Some("1.0.0"));
        assert_eq!(stack_part.package_release_hash, Some(release_hash('5')));
        assert_eq!(stack_part.stack_manifest_hash, Some(source_manifest_hash()));
        assert_eq!(stack_part.live_alias.as_deref(), Some("live"));
        assert_eq!(stack_part.artifact_hash, locked.live_specs[0].artifact_hash);
        let token_part = &locked.parts[1];
        assert_eq!(token_part.source, "registry:spl-token");
        assert_eq!(token_part.version.as_deref(), Some("4.0.1"));
        assert_eq!(token_part.package_release_hash, Some(release_hash('8')));
        assert_eq!(token_part.program_id.as_deref(), Some(TOKEN_PROGRAM_ID));

        // `a4 up ore-plus-token` deploys the StackManifest the lock pins.
        let written: Value = serde_json::from_slice(
            &fs::read(manifest.with_file_name(
                ".arete/compositions/ore-plus-token/ore-plus-token.stack-manifest.json",
            ))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(written["artifactHash"], json!(composed_hash));
    }

    #[test]
    fn a_composition_generates_for_every_target_naming_the_source_release() {
        let sandbox = RegistrySandbox::new(
            vec![(
                200,
                resolution(vec![
                    ore_stack_with_program_sdk('7', "1.0.2", vec![]),
                    token_program('8'),
                ]),
            )],
            false,
        );
        let manifest = typescript_project(
            &sandbox,
            &COMPOSED_STACK.replace("targets = [\"typescript\"]\n", ""),
        );
        let text = fs::read_to_string(&manifest).unwrap().replace(
            "targets = [\"typescript\"]",
            "targets = [\"typescript\", \"rust\", \"python\"]",
        );
        fs::write(&manifest, text).unwrap();
        install_project(&manifest, InstallOptions::default()).expect("every target");
        let hash = source_manifest_hash();
        let rust = generated_text(&manifest, "rust");
        assert!(rust.contains(HOSTED_WS), "rust: endpoint");
        assert!(
            rust.contains(&format!(
                "fn stack_manifest_hash() -> Option<&'static str> {{\n        Some(\"{hash}\")\n    }}\n\n    fn live_alias() -> Option<&'static str> {{\n        Some(\"live\")\n    }}"
            )),
            "rust: source release"
        );
        assert!(rust.contains(TOKEN_PROGRAM_ID), "rust: spl-token");
        let python = generated_text(&manifest, "python");
        assert!(python.contains(HOSTED_WS), "python: endpoint");
        assert!(
            python.contains(&format!(
                "    release=StackRelease(\n        stack_manifest_hash=\"{hash}\",\n        live_alias=\"live\",\n    ),"
            )),
            "python: source release"
        );
        assert!(python.contains(TOKEN_PROGRAM_ID), "python: spl-token");
    }

    #[test]
    fn composed_views_must_exist_and_be_served_by_the_source_stack() {
        for (views, expected) in [
            (
                "[\"OreRound/missing\"]",
                "which its LiveSpec does not define",
            ),
            (
                "[\"OreTreasury/state\"]",
                "which stack 'ore' does not serve",
            ),
        ] {
            // The source stack serves four of the LiveSpec's views.
            let stack =
                composed_ore_stack("OreStream", &["live"], ore_programs_with_sdk('7'), vec![]);
            let sandbox = RegistrySandbox::new(vec![(200, resolution(vec![stack]))], false);
            let manifest = typescript_project(
                &sandbox,
                &format!(
                    "\n[authoring.stacks.ore-views]\nlive.ore = {{ stack = \"ore\", views = {views} }}\n\n[dependencies.stacks.ore-views]\nsource = {{ workspace = \"ore-views\" }}\n"
                ),
            );
            let original = fs::read(&manifest).unwrap();
            let error = install_project(&manifest, InstallOptions::default())
                .expect_err("a view the source does not serve");
            sandbox.request();
            assert!(format!("{error:#}").contains(expected), "{error:#}");
            assert_project_untouched(&manifest, &original);
        }

        // A served subset composes.
        let stack = composed_ore_stack("OreStream", &["live"], ore_programs_with_sdk('7'), vec![]);
        let sandbox = RegistrySandbox::new(vec![(200, resolution(vec![stack]))], false);
        let manifest = typescript_project(
            &sandbox,
            "\n[authoring.stacks.ore-views]\nlive.ore = { stack = \"ore\", views = [\"OreBoard/state\"] }\n\n[dependencies.stacks.ore-views]\nsource = { workspace = \"ore-views\" }\n",
        );
        install_project(&manifest, InstallOptions::default()).expect("a served subset");
        let core = &generated_files(&manifest, "typescript")["stacks/ore-views/ore-views-core.ts"];
        assert!(core.contains("OreBoard/state") && !core.contains("OreRound/latest"));
    }

    #[test]
    fn a_locked_composition_detects_drift_and_update_advances_it() {
        let sandbox = RegistrySandbox::new(
            vec![
                (200, composed_resolution('7', '8')),
                (200, composed_resolution('7', '8')),
                // spl-token moved to another release.
                (200, composed_resolution('7', '9')),
                // Same ore stack release, but its program SDK moved.
                (200, composed_resolution('6', '8')),
                (200, composed_resolution('7', '9')),
            ],
            false,
        );
        let manifest = typescript_project(&sandbox, COMPOSED_STACK);
        install_project(&manifest, InstallOptions::default()).expect("install");
        sandbox.request();
        let lock_path = manifest.with_file_name("arete.lock");
        let before = fs::read(&lock_path).unwrap();
        let locked = InstallOptions {
            locked: true,
            ..InstallOptions::default()
        };

        install_project(&manifest, locked).expect("nothing moved");
        let request = request_body(&sandbox.request());
        // Every registry part is requested at exactly its locked release.
        assert_eq!(
            request["dependencies"][0]["lockedPackageReleaseHash"],
            json!(release_hash('5'))
        );
        assert_eq!(
            request["dependencies"][1]["lockedPackageReleaseHash"],
            json!(release_hash('8'))
        );
        assert_eq!(fs::read(&lock_path).unwrap(), before);

        for expected in [
            "program 'spl-token' of composed stack 'ore-plus-token'",
            "integrity failure for composed stack 'ore-plus-token'",
        ] {
            let error = install_project(&manifest, locked).expect_err("a part moved");
            sandbox.request();
            let text = format!("{error:#}");
            assert!(text.contains(expected), "{text}");
            assert!(text.contains("a4 update stack ore-plus-token"), "{text}");
            assert_eq!(fs::read(&lock_path).unwrap(), before, "lock unchanged");
        }

        install_project(
            &manifest,
            InstallOptions {
                update: Some(UpdateSelection {
                    kind: Some(DependencyKind::Stack),
                    alias: Some("ore-plus-token"),
                }),
                ..InstallOptions::default()
            },
        )
        .expect("update");
        let request = request_body(&sandbox.request());
        assert!(
            request["dependencies"][1]
                .get("lockedPackageReleaseHash")
                .is_none(),
            "{request}"
        );
        let lock = lock_of(&manifest);
        let token = lock.dependencies[0]
            .parts
            .iter()
            .find(|part| part.kind == DependencyKind::Program)
            .unwrap();
        assert_eq!(token.package_release_hash, Some(release_hash('9')));
    }

    #[test]
    fn a_composed_alias_without_hosted_delivery_is_definition_only_and_says_so() {
        let mut stack = ore_stack_with_program_sdk('7', "1.0.2", vec![]);
        stack["delivery"] = json!({"mode": "definition-only"});
        let sandbox = RegistrySandbox::new(
            vec![(200, resolution(vec![stack.clone(), token_program('8')]))],
            false,
        );
        let manifest = typescript_project(&sandbox, COMPOSED_STACK);
        install_project(&manifest, InstallOptions::default()).expect("definition-only alias");
        let core = &generated_files(&manifest, "typescript")
            ["stacks/ore-plus-token/ore-plus-token-core.ts"];
        assert!(core.contains("ws: '', // TODO"), "{core}");
        assert!(!core.contains("stackManifestHash"), "{core}");
        assert!(!core.lines().any(|line| line.starts_with("  gateway: ")));

        let composed =
            composition::compose_parts("ore-plus-token", vec![live_part("ore", stack)], vec![])
                .unwrap();
        assert!(composed.hosted.is_empty());
        assert!(
            composed
                .notes
                .iter()
                .any(|note| note.contains("definition-only")
                    && note.contains("a4 up ore-plus-token")),
            "{:?}",
            composed.notes
        );
    }

    #[test]
    fn a_hosted_alias_keeps_its_source_delivery_beside_a_definition_only_one() {
        let sandbox = RegistrySandbox::new(
            vec![(
                200,
                resolution(vec![ore_stack_with_program_sdk(
                    '7',
                    "1.0.2",
                    vec![ore_program_extension("typescript", 'e')],
                )]),
            )],
            false,
        );
        let manifest = typescript_project(
            &sandbox,
            "\n[authoring.stacks.ore-mix]\nlive.ore = { stack = \"ore\" }\nlive.local = { path = \"./artifacts/local.live-spec.json\" }\n\n[dependencies.stacks.ore-mix]\nsource = { workspace = \"ore-mix\" }\n",
        );
        let artifacts = manifest.with_file_name("artifacts");
        fs::create_dir_all(&artifacts).unwrap();
        fs::write(
            artifacts.join("local.live-spec.json"),
            serde_json::to_vec(&ore_fixture("OreStream.live-spec.json")).unwrap(),
        )
        .unwrap();
        install_project(&manifest, InstallOptions::default()).expect("mixed composition");
        let files = generated_files(&manifest, "typescript");
        let ore = &files["stacks/ore-mix/ore-stack.ts"];
        assert!(ore.contains(&format!("ws: '{HOSTED_WS}'")), "{ore}");
        assert!(ore.contains(&format!(
            "  release: {{\n    stackManifestHash: '{}',\n    liveAlias: 'live',\n  }},",
            source_manifest_hash()
        )));
        let local = &files["stacks/ore-mix/local-stack.ts"];
        assert!(
            !local.contains(HOSTED_WS) && !local.contains("stackManifestHash"),
            "{local}"
        );
        let session = &files["stacks/ore-mix/ore-mix.ts"];
        assert!(session.contains("\"release\": {"), "{session}");
        assert!(session.contains("createOreMixHostedSession"), "{session}");
        // The one LiveSpec both aliases use is written once for `a4 up`.
        let written = fs::read_dir(manifest.with_file_name(".arete/compositions/ore-mix"))
            .unwrap()
            .filter(|entry| {
                entry
                    .as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".live-spec.json")
            })
            .count();
        assert_eq!(written, 1);
        let parts = &lock_of(&manifest).dependencies[0].parts;
        assert!(parts
            .iter()
            .any(|part| part.source == "path:artifacts/local.live-spec.json"));
    }

    #[test]
    fn an_explicit_program_sdk_replaces_the_one_the_source_stack_brings() {
        let stack = ore_stack_with_program_sdk('7', "1.0.2", vec![]);
        let composed = composition::compose_parts(
            "ore-newer",
            vec![live_part("ore", stack.clone())],
            vec![composition::ProgramPart::Registry {
                requirement: "^1".into(),
                resolved: serde_json::from_value(ore_program_dependency(
                    '8',
                    "1.0.3",
                    vec![ore_program_extension("typescript", 'f')],
                ))
                .unwrap(),
            }],
        )
        .unwrap();
        assert!(
            composed.notes.iter().any(|note| note.contains(
                "the `programs` entry ore@1.0.3 replaces the program SDK stack 'ore' brings (ore@1.0.2)"
            )),
            "{:?}",
            composed.notes
        );
        let ore = composed
            .programs
            .iter()
            .find(|program| program.definition.program_id == ORE_PROGRAM_ID)
            .unwrap();
        assert_eq!(
            ore.program_package.as_ref().unwrap().package_release_hash,
            release_hash('8')
        );
        assert_eq!(composed.programs.len(), 2, "ore (explicit) and entropy");

        // A program whose ProgramSpec differs from the one the views index
        // cannot replace it.
        let error = composition::compose_parts(
            "ore-other",
            vec![live_part("ore", stack)],
            vec![composition::ProgramPart::Registry {
                requirement: "^1".into(),
                resolved: serde_json::from_value(program_package(
                    "ore",
                    ORE_PROGRAM_ID,
                    "ore",
                    '9',
                    "2.0.0",
                ))
                .unwrap(),
            }],
        )
        .unwrap_err();
        assert!(
            format!("{error:#}").contains("cannot be read with that program"),
            "{error:#}"
        );
    }

    #[test]
    fn compose_writes_the_authoring_entry_and_can_install_it() {
        let sandbox = RegistrySandbox::new(
            vec![
                (200, composed_resolution('7', '8')),
                (200, composed_resolution('7', '8')),
                (200, composed_resolution('7', '8')),
            ],
            false,
        );
        let root = sandbox.dir.path().join("compose-project");
        fs::create_dir_all(&root).unwrap();
        let manifest = root.join("arete.toml");
        fs::write(
            &manifest,
            "manifest_version = 1\n\n# Keep this comment.\n[project]\nname = \"compose\"\n\n[sdk]\ntargets = [\"typescript\"]\n",
        )
        .unwrap();
        let config = manifest.display().to_string();
        let compose = |install: bool| {
            crate::commands::public_artifacts::compose(
                crate::commands::public_artifacts::ComposeArgs {
                    config_path: &config,
                    name: "ore-plus-token",
                    programs: &["spl-token@^4".to_string()],
                    lives: &["ore".to_string()],
                    artifact_dirs: &[],
                    selected_views: &[
                        "ore=OreRound/latest".to_string(),
                        "ore=OreMiner/state".to_string(),
                    ],
                    output: None,
                    install,
                },
            )
        };
        compose(false).expect("compose into arete.toml");
        let request = request_body(&sandbox.request());
        assert_eq!(request["dependencies"][0]["package"], "ore");
        assert_eq!(request["dependencies"][0]["requirement"], "*");
        assert_eq!(request["dependencies"][1]["requirement"], "^4");
        let text = fs::read_to_string(&manifest).unwrap();
        assert!(text.contains("# Keep this comment."), "{text}");
        let written = "[authoring.stacks.ore-plus-token]\nlive.ore = { stack = \"ore\", version = \"^1.0.0\", views = [\"OreRound/latest\", \"OreMiner/state\"] }\nprograms = [{ package = \"spl-token\", version = \"^4\" }]\n";
        assert!(text.contains(written), "{text}");
        assert!(!text.contains("[dependencies"), "{text}");
        assert!(!manifest.with_file_name("arete.lock").exists());

        // Composing it again with --install updates the entry in place,
        // declares the dependency and installs it.
        compose(true).expect("compose and install");
        sandbox.request();
        sandbox.request();
        let text = fs::read_to_string(&manifest).unwrap();
        assert_eq!(
            text.matches("[authoring.stacks.ore-plus-token]").count(),
            1,
            "{text}"
        );
        assert!(text.contains(written), "{text}");
        assert!(
            text.contains(
                "[dependencies.stacks.ore-plus-token]\nsource = { workspace = \"ore-plus-token\" }\n"
            ),
            "{text}"
        );
        let lock = lock_of(&manifest);
        assert_eq!(lock.dependencies[0].source, "workspace:ore-plus-token");
        assert!(generated_files(&manifest, "typescript")
            .contains_key("stacks/ore-plus-token/ore-plus-token.ts"));
    }

    // ---------------------------------------------------------------------
    // Review fixes: composition edits, older locks, registry extensionApi.
    // ---------------------------------------------------------------------

    const ORE_WITH_OVERRIDE: &str = r#"
[authoring.stacks.ore-sdk]
live.ore = { stack = "ore", version = "^1" }
programs = [{ package = "ore", version = "^1" }]

[dependencies.stacks.ore-sdk]
source = { workspace = "ore-sdk" }
"#;

    #[test]
    fn removing_an_explicit_program_re_resolves_it_from_the_stack() {
        // The stack references ore program SDK release '7' (or, second, only
        // embeds the core program); the explicit entry pins release '8'.
        for stack in [
            ore_stack_with_program_sdk('7', "1.0.2", vec![]),
            ore_stack_dependency(Some(hosted_delivery(HOSTED_WS, HOSTED_HTTP, 4))),
        ] {
            let sandbox = RegistrySandbox::new(
                vec![
                    (
                        200,
                        resolution(vec![
                            stack.clone(),
                            ore_program_dependency('8', "1.0.3", vec![]),
                        ]),
                    ),
                    (200, resolution(vec![stack.clone()])),
                    (200, resolution(vec![stack.clone()])),
                ],
                false,
            );
            let manifest = typescript_project(&sandbox, ORE_WITH_OVERRIDE);
            install_project(&manifest, InstallOptions::default()).expect("with the override");
            sandbox.request();
            let ore = |lock: &ProjectLock| {
                lock.dependencies[0]
                    .programs
                    .iter()
                    .find(|program| program.program_id == ORE_PROGRAM_ID)
                    .unwrap()
                    .package_release_hash
                    .clone()
            };
            assert_eq!(ore(&lock_of(&manifest)), Some(release_hash('8')));

            let text = fs::read_to_string(&manifest)
                .unwrap()
                .replace("programs = [{ package = \"ore\", version = \"^1\" }]\n", "");
            fs::write(&manifest, text).unwrap();
            install_project(&manifest, InstallOptions::default())
                .expect("removing the override is an edit, not drift");
            let request = request_body(&sandbox.request());
            assert_eq!(
                request["dependencies"][0]["lockedPackageReleaseHash"],
                json!(release_hash('5')),
                "the unchanged stack part stays locked"
            );
            let expected = stack["programs"][0]["programPackage"]["packageReleaseHash"]
                .as_str()
                .map(str::to_string);
            let lock = lock_of(&manifest);
            assert_eq!(ore(&lock), expected);
            assert!(lock.dependencies[0]
                .parts
                .iter()
                .all(|part| part.kind == DependencyKind::Stack));
            install_project(
                &manifest,
                InstallOptions {
                    locked: true,
                    ..InstallOptions::default()
                },
            )
            .expect("and the new lock is exact");
            sandbox.request();
        }
    }

    #[test]
    fn a_moved_program_sdk_under_an_unchanged_locked_part_fails_even_when_parts_are_added() {
        let memo = || program_package("memo", VOTE_PROGRAM_ID, "vote_program", 'c', "1.0.0");
        let sandbox = RegistrySandbox::new(
            vec![
                (200, composed_resolution('7', '8')),
                (
                    200,
                    resolution(vec![
                        ore_stack_with_program_sdk('6', "1.0.2", vec![]),
                        token_program('8'),
                        memo(),
                    ]),
                ),
            ],
            false,
        );
        let manifest = typescript_project(&sandbox, COMPOSED_STACK);
        install_project(&manifest, InstallOptions::default()).expect("install");
        sandbox.request();
        let before = fs::read(manifest.with_file_name("arete.lock")).unwrap();
        let text = fs::read_to_string(&manifest).unwrap().replace(
            "programs = [{ package = \"spl-token\", version = \"^4\" }]",
            "programs = [{ package = \"spl-token\", version = \"^4\" }, { package = \"memo\" }]",
        );
        fs::write(&manifest, text).unwrap();
        let error = install_project(&manifest, InstallOptions::default())
            .expect_err("the locked ore stack release now references another program SDK");
        sandbox.request();
        let text = format!("{error:#}");
        assert!(
            text.contains("integrity failure for composed stack 'ore-plus-token'"),
            "{text}"
        );
        assert!(text.contains(&release_hash('6')), "{text}");
        assert_eq!(
            fs::read(manifest.with_file_name("arete.lock")).unwrap(),
            before
        );
    }

    /// arete.lock as a CLI that did not resolve program SDK identities wrote
    /// it: no program package releases, and only legacy extension hashes.
    fn without_program_sdk_identities(lock: &str) -> String {
        lock.lines()
            .filter(|line| {
                !line.starts_with(
                    "package_release_hash = \"arete:registry-package-release:v2:sha256:7",
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
            + "\n"
    }

    #[test]
    fn a_locked_install_accepts_a_lock_written_before_program_sdk_identities() {
        let response = || {
            (
                200,
                resolution(vec![ore_stack_with_program_sdk(
                    '7',
                    "1.0.2",
                    vec![ore_program_extension("typescript", 'e')],
                )]),
            )
        };
        let sandbox = RegistrySandbox::new(vec![response(), response(), response()], false);
        let manifest = typescript_project(&sandbox, ORE_STACK_DEPENDENCY);
        install_project(&manifest, InstallOptions::default()).expect("install");
        sandbox.request();
        let lock_path = manifest.with_file_name("arete.lock");
        let older = without_program_sdk_identities(&fs::read_to_string(&lock_path).unwrap());
        assert!(!older.contains(&release_hash('7')), "{older}");
        fs::write(&lock_path, &older).unwrap();

        install_project(
            &manifest,
            InstallOptions {
                locked: true,
                ..InstallOptions::default()
            },
        )
        .expect("the older lock pins nothing differently");
        sandbox.request();
        assert_eq!(
            fs::read_to_string(&lock_path).unwrap(),
            older,
            "--locked does not rewrite the lock"
        );

        install_project(&manifest, InstallOptions::default()).expect("a plain install");
        sandbox.request();
        assert!(fs::read_to_string(&lock_path)
            .unwrap()
            .contains(&format!("package_release_hash = \"{}\"", release_hash('7'))));
    }

    #[test]
    fn a_locked_install_refuses_an_older_lock_missing_a_target_s_program_sdk_extension() {
        let extensions = |targets: &[(&str, char)]| {
            resolution(vec![ore_stack_with_program_sdk(
                '7',
                "1.0.2",
                targets
                    .iter()
                    .map(|(target, marker)| ore_program_extension(target, *marker))
                    .collect(),
            )])
        };
        let sandbox = RegistrySandbox::new(
            vec![
                (200, extensions(&[("typescript", 'e')])),
                (200, extensions(&[("typescript", 'e'), ("python", 'c')])),
            ],
            false,
        );
        let manifest = typescript_project(&sandbox, ORE_STACK_DEPENDENCY);
        install_project(&manifest, InstallOptions::default()).expect("install");
        sandbox.request();

        // The project now generates Python too, under a lock as an older a4
        // wrote it: no program SDK release, only the TypeScript extension.
        let text = fs::read_to_string(&manifest).unwrap().replace(
            "targets = [\"typescript\"]",
            "targets = [\"typescript\", \"python\"]",
        );
        fs::write(&manifest, text).unwrap();
        let mut lock = lock_of(&manifest);
        lock.manifest_hash = ProjectManifest::load(&manifest).unwrap().manifest_hash;
        let stack = &mut lock.dependencies[0];
        stack.targets = vec![InstallTarget::TypeScript, InstallTarget::Python];
        for program in &mut stack.programs {
            program.package_release_hash = None;
        }
        let lock_path = manifest.with_file_name("arete.lock");
        lock.write_atomic(&lock_path).unwrap();
        let before = fs::read(&lock_path).unwrap();

        let error = install_project(
            &manifest,
            InstallOptions {
                locked: true,
                ..InstallOptions::default()
            },
        )
        .expect_err("the Python extension would be generated unpinned");
        sandbox.request();
        let text = format!("{error:#}");
        assert!(
            text.contains(&format!(
                "does not record exactly the SDK extensions that stack 'ore' generates for program {ORE_PROGRAM_ID}"
            )),
            "{text}"
        );
        assert!(
            text.contains("Run `a4 install` once to record the program SDK pins"),
            "{text}"
        );
        assert_eq!(fs::read(&lock_path).unwrap(), before, "lock unchanged");
    }

    #[test]
    fn an_older_lock_matches_exactly_what_is_generated() {
        use LockedProgramMatch::{Differs, Same, Unpinned};
        let program = |package: Option<char>, extensions: &[&String]| LockedProgram {
            program_id: ORE_PROGRAM_ID.into(),
            program_spec_hash: ore_program_spec_hash(),
            program_release_hash: Some("release".into()),
            package_release_hash: package.map(release_hash),
            sdk_extension_hashes: extensions.iter().map(|hash| hash.to_string()).collect(),
        };
        let generated = |hashes: &[&String]| {
            hashes
                .iter()
                .map(|hash| hash.to_string())
                .collect::<BTreeSet<_>>()
        };
        let (typescript, python, legacy) = ("e".repeat(64), "c".repeat(64), "d".repeat(64));
        let now = program(Some('7'), &[&typescript]);
        let generates_typescript = generated(&[&typescript]);
        // The package release is implied by the locked stack release.
        assert_eq!(
            locked_program_match(
                &program(None, &[&typescript]),
                &now,
                &generates_typescript,
                None
            ),
            Same
        );
        // Extensions generated for a target the lock never recorded, or no
        // longer generated although recorded, are not pinned.
        let both = program(Some('7'), &[&typescript, &python]);
        assert_eq!(
            locked_program_match(
                &program(None, &[&typescript]),
                &both,
                &generated(&[&typescript, &python]),
                None
            ),
            Unpinned
        );
        assert_eq!(
            locked_program_match(&program(None, &[]), &now, &generates_typescript, None),
            Unpinned
        );
        assert_eq!(
            locked_program_match(
                &program(None, &[&typescript, &python]),
                &now,
                &generates_typescript,
                None
            ),
            Unpinned
        );
        // The legacy single extension counts as its own target's hash: it
        // pins that target, and nothing when the target is not generated.
        assert_eq!(
            locked_program_match(
                &program(None, &[&legacy]),
                &program(Some('7'), &[&legacy]),
                &generated(&[&legacy]),
                None
            ),
            Same
        );
        assert_eq!(
            locked_program_match(
                &program(None, &[&legacy]),
                &program(Some('7'), &[]),
                &generated(&[]),
                Some(legacy.as_str())
            ),
            Same
        );
        assert_eq!(
            locked_program_match(
                &program(None, &[&legacy]),
                &now,
                &generates_typescript,
                None
            ),
            Unpinned
        );
        // What the lock does pin still has to match.
        assert_eq!(
            locked_program_match(
                &program(Some('6'), &[&typescript]),
                &now,
                &generates_typescript,
                None
            ),
            Differs
        );
        let mut moved = program(None, &[&typescript]);
        moved.program_release_hash = Some("other".into());
        assert_eq!(
            locked_program_match(&moved, &now, &generates_typescript, None),
            Differs
        );
    }

    /// A project whose `@usearete/sdk` provides extension API `api`.
    fn install_typescript_sdk(manifest: &Path, api: u32) {
        let package = manifest.with_file_name("node_modules/@usearete/sdk");
        fs::create_dir_all(&package).unwrap();
        fs::write(
            package.join("package.json"),
            format!(
                r#"{{"name":"@usearete/sdk","version":"0.23.0","arete":{{"extensionApi":{api}}}}}"#
            ),
        )
        .unwrap();
    }

    fn with_extension_api(mut extension: Value, api: u32) -> Value {
        extension["extensionApi"] = json!(api);
        extension
    }

    #[test]
    fn a_registry_extension_api_is_checked_against_the_installed_sdk() {
        let program = |api: u32| {
            resolution(vec![ore_program_dependency(
                '7',
                "1.0.2",
                vec![with_extension_api(
                    ore_program_extension("typescript", 'e'),
                    api,
                )],
            )])
        };
        let sandbox = RegistrySandbox::new(vec![(200, program(2)), (200, program(1))], false);
        let manifest = typescript_project(&sandbox, ORE_PROGRAM_DEPENDENCY);
        install_typescript_sdk(&manifest, 1);
        let original = fs::read(&manifest).unwrap();
        let error = install_project(&manifest, InstallOptions::default())
            .expect_err("the extension needs another extension API");
        sandbox.request();
        let text = format!("{error:#}");
        assert!(
            text.contains("require extension API 2, but the installed @usearete/sdk 0.23.0 provides extension API 1"),
            "{text}"
        );
        assert_eq!(fs::read(&manifest).unwrap(), original);
        assert!(!manifest.with_file_name("arete.lock").exists());

        install_project(&manifest, InstallOptions::default()).expect("a matching extension API");
        sandbox.request();
        // The staged manifest carries it, where `a4 doctor` reads it.
        let staged: Value = serde_json::from_str(
            &generated_files(&manifest, "typescript")["programs/ore/extensions.json"],
        )
        .unwrap();
        assert_eq!(staged["extensionApi"], 1);
    }

    #[test]
    fn a_stack_program_extension_api_is_checked_too() {
        let sandbox = RegistrySandbox::new(
            vec![(
                200,
                resolution(vec![ore_stack_with_program_sdk(
                    '7',
                    "1.0.2",
                    vec![with_extension_api(
                        ore_program_extension("typescript", 'e'),
                        3,
                    )],
                )]),
            )],
            false,
        );
        let manifest = typescript_project(&sandbox, ORE_STACK_DEPENDENCY);
        install_typescript_sdk(&manifest, 1);
        let error = install_project(&manifest, InstallOptions::default())
            .expect_err("the stack's program SDK extension needs another extension API");
        sandbox.request();
        assert!(
            format!("{error:#}").contains("require extension API 3"),
            "{error:#}"
        );

        // An entry that contradicts its own manifest is refused.
        let mut extension = with_extension_api(ore_program_extension("typescript", 'e'), 2);
        extension["artifact"]["manifest"]["extensionApi"] = json!(1);
        let resolved: crate::project::resolver::ResolvedSdkExtension =
            serde_json::from_value(extension).unwrap();
        let error = resolved.artifact_with_contract().unwrap_err().to_string();
        assert!(error.contains("its manifest declares 1"), "{error}");
        let resolved: crate::project::resolver::ResolvedSdkExtension = serde_json::from_value(
            with_extension_api(ore_program_extension("typescript", 'e'), 2),
        )
        .unwrap();
        assert_eq!(
            resolved
                .artifact_with_contract()
                .unwrap()
                .manifest
                .extension_api
                .map(std::num::NonZeroU32::get),
            Some(2)
        );
        assert_eq!(resolved.artifact.manifest.extension_api, None);
    }
}
