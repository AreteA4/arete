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
        resolved: Box<ResolvedRegistryDependency>,
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
        // A batch failure names one package only when the batch *is* one
        // package. Attributing a multi-dependency failure to the first entry
        // reported the wrong package and, with a batch-wide `locked` flag,
        // could tell the user to `a4 update` a dependency that is not locked.
        let single = match registry_requests.as_slice() {
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
                None => describe_resolver_batch_error(error, &registry_requests),
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
            verify_resolved_release_identity(&response)?;
            if let Some(locked) = &request.locked_package_release_hash {
                if response.package_release_hash() != locked {
                    bail!(
                        "Registry resolved '{}' to release {} but arete.lock pins {}; run `a4 update {} {}` to advance intentionally",
                        request.alias,
                        response.package_release_hash(),
                        locked,
                        request.kind,
                        request.alias
                    );
                }
            }
            let dependency = manifest
                .dependency(request.kind, &request.alias)
                .expect("request came from manifest");
            resolved.push(ResolvedProjectDependency::Registry {
                kind: request.kind,
                source: dependency.source.stable_description(),
                requirement: request.requirement,
                targets: dependency.selected_targets(&manifest.document.sdk).to_vec(),
                resolved: Box::new(response),
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
        package, install, ..
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

    const RESOLVE_PATH: &str = "/api/registry/v1/resolve";
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
}
