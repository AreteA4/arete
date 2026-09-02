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
    generate_project_local_program, generate_project_local_stack,
    generate_project_registry_dependency, ProjectGenerationOptions,
};

use super::lockfile::{LockedDependency, LockedLiveSpec, LockedProgram};
use super::manifest::{
    DependencyKind, DependencyOutputsV1, DependencySourceV1, DependencyV1, InstallTarget,
    ManifestV1, PathSourceV1, RegistrySourceV1, WorkspaceSourceV1,
};
use super::paths::ProjectPaths;
use super::resolver::{
    RegistryDependencyRequest, RegistryResolveRequest, ResolvedRegistryDependency,
};
use super::{InstallPlan, ProjectLock, ProjectManifest, GENERATOR_CONTRACT, RESOLVER_CONTRACT};

const INSTALL_JOURNAL: &str = ".arete/install-journal.json";

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
    let alias = options
        .alias
        .unwrap_or_else(|| package.rsplit('/').next().unwrap_or(&package).to_string());
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
        };
        match kind {
            DependencyKind::Stack => manifest.dependencies.stacks.insert(alias, dependency),
            DependencyKind::Program => manifest.dependencies.programs.insert(alias, dependency),
        };
        manifest.validate()?;
        let manifest_path = temporary_root.join("arete.toml");
        fs::write(&manifest_path, manifest.to_toml_pretty()?)?;
        install_project(
            &manifest_path,
            InstallOptions {
                allow_outside_project: true,
                ..InstallOptions::default()
            },
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
    let alias = options
        .alias
        .unwrap_or_else(|| package.rsplit('/').next().unwrap_or(&package).to_string());
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
    let dependency = DependencyV1 {
        source: DependencySourceV1::Registry(RegistrySourceV1 { registry: package }),
        version: Some(requirement),
        targets,
        outputs,
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
    let result = install_project(
        manifest_path,
        InstallOptions {
            allow_outside_project: options.allow_outside_project,
            ..InstallOptions::default()
        },
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
                manifest_hash: existing.manifest_hash.clone(),
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
    println!("Removed {kind} dependency '{alias}'");
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
    Item::Table(table)
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
        }],
        targets: vec![InstallTarget::TypeScript],
        generator_contract: GENERATOR_CONTRACT.into(),
    };
    let response = ApiClient::new()?.resolve_registry_dependencies(&request)?;
    if response.resolver_contract != RESOLVER_CONTRACT || response.dependencies.len() != 1 {
        bail!("Registry returned an invalid single-package resolver response");
    }
    let resolved = &response.dependencies[0];
    if resolved.alias() != alias || resolved.package() != package {
        bail!("Registry response did not match requested package '{package}'");
    }
    verify_resolved_kind_and_contract(kind, resolved)?;
    verify_resolved_extensions(resolved, &[InstallTarget::TypeScript])?;
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
    let manifest = ProjectManifest::load(manifest_path)?;
    install_loaded_project(manifest, options, Vec::new())
}

fn install_loaded_project(
    manifest: ProjectManifest,
    options: InstallOptions<'_>,
    removals: Vec<RemovalOutput>,
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
    if options.locked && previous_lock.as_ref() != Some(&prospective_lock) {
        bail!("--locked resolution differs from arete.lock; no output was changed");
    }

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
    println!(
        "Installed {} dependencies and wrote {}",
        prospective_lock.dependencies.len(),
        lock_path.display()
    );
    Ok(())
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
        if !matches!(&dependency.source, DependencySourceV1::Registry(_)) {
            bail!("Dependency '{alias}' is local and has no registry version to update");
        }
    } else if let Some(kind) = selection.kind {
        let count = manifest
            .dependencies()
            .filter(|(candidate, _, dependency)| {
                *candidate == kind && matches!(&dependency.source, DependencySourceV1::Registry(_))
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
        resolved: ResolvedRegistryDependency,
    },
}

impl ResolvedProjectDependency {
    fn kind(&self) -> DependencyKind {
        match self {
            Self::LocalStack { .. } => DependencyKind::Stack,
            Self::LocalProgram { .. } => DependencyKind::Program,
            Self::Registry { kind, .. } => *kind,
        }
    }

    fn alias(&self) -> &str {
        match self {
            Self::LocalStack { alias, .. } | Self::LocalProgram { alias, .. } => alias,
            Self::Registry { resolved, .. } => resolved.alias(),
        }
    }

    fn targets(&self) -> &[InstallTarget] {
        match self {
            Self::LocalStack { targets, .. }
            | Self::LocalProgram { targets, .. }
            | Self::Registry { targets, .. } => targets,
        }
    }
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
                });
            }
            DependencySourceV1::Path(PathSourceV1 { path }) => resolved.push(
                resolve_path_dependency(&paths, kind, alias, dependency, path, targets)?,
            ),
            DependencySourceV1::Workspace(WorkspaceSourceV1 { workspace }) => {
                resolved.push(resolve_workspace_dependency(
                    manifest, &paths, kind, alias, dependency, workspace, targets,
                )?)
            }
        }
    }

    if !registry_requests.is_empty() {
        let request = RegistryResolveRequest {
            manifest_version: manifest.document.manifest_version,
            dependencies: registry_requests.clone(),
            targets: manifest.document.sdk.targets.clone(),
            generator_contract: GENERATOR_CONTRACT.into(),
        };
        let response = ApiClient::new()?.resolve_registry_dependencies(&request)?;
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
        for (request, response) in registry_requests.into_iter().zip(response.dependencies) {
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
            verify_resolved_extensions(&response, &manifest.document.sdk.targets)?;
            let dependency = manifest
                .dependency(request.kind, &request.alias)
                .expect("request came from manifest");
            resolved.push(ResolvedProjectDependency::Registry {
                kind: request.kind,
                source: dependency.source.stable_description(),
                requirement: request.requirement,
                targets: dependency.selected_targets(&manifest.document.sdk).to_vec(),
                resolved: response,
            });
        }
    }
    resolved.sort_by(|left, right| (left.kind(), left.alias()).cmp(&(right.kind(), right.alias())));
    Ok(resolved)
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
            let manifest_path = paths.input(&authored.manifest, "authored StackManifest")?;
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
    let requested = targets
        .iter()
        .map(|target| target.as_str())
        .collect::<BTreeSet<_>>();
    let mut seen = BTreeSet::new();
    for extension in extensions {
        if !requested.contains(extension.target.as_str()) {
            bail!(
                "Registry returned an unrequested '{}' SDK extension",
                extension.target
            );
        }
        if !seen.insert(extension.target.as_str()) {
            bail!(
                "Registry returned more than one '{}' SDK extension",
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
                "Registry returned an invalid '{}' SDK extension identity",
                extension.target
            );
        }
    }
    Ok(())
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
                        sdk_extension_hashes: Vec::new(),
                    })
                    .collect(),
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
        });
    }
    lock.normalize_and_validate()?;
    Ok(lock)
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
                    sdk_extension_hashes: program
                        .definition
                        .extensions
                        .iter()
                        .map(extension_lock_hash)
                        .collect(),
                })
                .collect(),
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
    manifest_hash: String,
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
        if let ResolvedProjectDependency::Registry { resolved, .. } = dependency {
            cache_registry_dependency(resolved)?;
        }
    }
    Ok(staged)
}

fn cache_registry_dependency(resolved: &ResolvedRegistryDependency) -> Result<()> {
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
    Ok(())
}

fn cache_immutable_json(kind: &str, hash: &str, value: &serde_json::Value) -> Result<()> {
    if hash.is_empty()
        || !hash
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b':' | b'-' | b'_'))
    {
        bail!("Cannot cache invalid {kind} identity '{hash}'");
    }
    let directory = dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("Could not determine Arete cache directory"))?
        .join(".arete")
        .join("cache")
        .join("registry")
        .join("v1")
        .join(kind);
    fs::create_dir_all(&directory)?;
    let path = directory.join(format!("{hash}.json"));
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
        ResolvedProjectDependency::Registry { resolved, .. } => Some(match resolved {
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
        ("manifestHash", output.manifest_hash.as_str()),
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
    println!("Recovered an interrupted Arete install");
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
