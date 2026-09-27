//! Composed stacks: an `[authoring.stacks.<name>]` entry that groups live
//! views and program SDKs from published stacks, program packages and local
//! artifact files.
//!
//! Resolution fetches every registry part through the project resolver in
//! one batch, composes a StackManifest from the parts with the same rules as
//! `a4 stack compose` (the programs the views require come with them, extra
//! programs are independent, selected views must exist), and pins each part
//! in `arete.lock`. Generation then uses each program's program SDK and, for
//! each live alias whose source stack is hosted, that stack's delivery.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use arete_artifacts::{LiveSpecArtifactV2, ProgramSpecArtifact, SelectedViewV2};
use arete_interpreter::public_artifacts::StackRelease;

use crate::api_client::{
    RegistryCapabilityInstallBinding, RegistryLiveSpecInstallDescriptor,
    RegistryProgramInstallResponse, RegistryProgramPackageReference,
};
use crate::commands::sdk::{program_spec_artifact_from_registry, verify_registry_stack_artifacts};

use super::lockfile::{LockedDependency, LockedPart};
use super::manifest::{
    AuthoringStackV1, ComposedLiveSource, ComposedProgramSource, DependencyKind, InstallTarget,
    ProjectManifest,
};
use super::paths::ProjectPaths;
use super::resolver::{
    RegistryDependencyRequest, ResolvedRegistryDependency, ResolvedStackDelivery,
};

/// Where `a4 install` writes a composed stack's artifacts, for `a4 up`.
pub(crate) const COMPOSITIONS_DIR: &str = ".arete/compositions";

/// A composed stack, resolved and composed.
#[derive(Debug, Clone)]
pub(crate) struct ComposedStack {
    /// The `[authoring.stacks]` entry name, also the StackManifest name.
    pub name: String,
    pub stack_manifest: arete_artifacts::StackManifestArtifactV2,
    /// In StackManifest order.
    pub live_specs: Vec<(String, LiveSpecArtifactV2)>,
    /// In StackManifest order.
    pub program_specs: Vec<ProgramSpecArtifact>,
    /// The program SDK each program is generated from, for every program
    /// that has one: a program package release, or the program a published
    /// stack brings. A program from a local ProgramSpec file has none.
    pub programs: Vec<RegistryProgramInstallResponse>,
    /// The live aliases whose source stack is hosted, with its delivery.
    pub hosted: Vec<HostedLive>,
    /// The managed Solana gateway the hosted sources share, if they do.
    pub chain_binding: Option<RegistryCapabilityInstallBinding>,
    pub transaction_binding: Option<RegistryCapabilityInstallBinding>,
    pub parts: Vec<LockedPart>,
    /// What the install output says about this composition.
    pub notes: Vec<String>,
    /// The registry responses the parts came from, cached with the install.
    pub registry: Vec<ResolvedRegistryDependency>,
}

/// A composed live alias read from its source stack's hosted deployment.
#[derive(Debug, Clone)]
pub(crate) struct HostedLive {
    /// The binding, under the composed alias.
    pub descriptor: RegistryLiveSpecInstallDescriptor,
    /// What the deployment serves: the source stack's StackManifest and live
    /// alias. Sessions name it, exactly as the source stack's own SDK does.
    pub release: StackRelease,
}

impl ComposedStack {
    pub fn manifest_hash(&self) -> String {
        self.stack_manifest.artifact_hash.to_string()
    }

    /// The arete.lock entry of a dependency on this composed stack.
    pub fn lock_entry(
        &self,
        alias: &str,
        source: &str,
        targets: &[InstallTarget],
    ) -> LockedDependency {
        let descriptors = self
            .programs
            .iter()
            .map(|program| (program.definition.program_spec_hash.as_str(), program))
            .collect::<BTreeMap<_, _>>();
        LockedDependency {
            kind: DependencyKind::Stack,
            alias: alias.to_string(),
            source: source.to_string(),
            requirement: None,
            version: None,
            package_release_hash: None,
            stack_manifest_hash: Some(self.manifest_hash()),
            program_id: None,
            program_spec_hash: None,
            program_release_hash: None,
            live_specs: self
                .live_specs
                .iter()
                .map(|(alias, live)| super::lockfile::LockedLiveSpec {
                    alias: alias.clone(),
                    artifact_hash: live.artifact_hash.to_string(),
                })
                .collect(),
            programs: self
                .program_specs
                .iter()
                .map(|program| {
                    let hash = program.artifact_hash.to_string();
                    let descriptor = descriptors.get(hash.as_str());
                    super::lockfile::LockedProgram {
                        program_id: program.payload.program_id.clone(),
                        program_release_hash: descriptor
                            .map(|install| install.release.program_release_hash.clone()),
                        package_release_hash: descriptor
                            .and_then(|install| install.program_package.as_ref())
                            .map(|package| package.package_release_hash.clone()),
                        sdk_extension_hashes: descriptor
                            .map(|install| {
                                super::resolver::program_extension_artifacts(install, targets)
                                    .into_iter()
                                    .map(|extension| {
                                        extension
                                            .sdk_extension_hash
                                            .clone()
                                            .unwrap_or_else(|| extension.artifact_hash.clone())
                                    })
                                    .collect()
                            })
                            .unwrap_or_default(),
                        program_spec_hash: hash,
                    }
                })
                .collect(),
            parts: self.parts.clone(),
            sdk_extension_hashes: Vec::new(),
            targets: targets.to_vec(),
            generator_contract: super::GENERATOR_CONTRACT.into(),
        }
    }
}

/// One live alias of a composition, with its source resolved.
pub(crate) struct LivePart {
    pub alias: String,
    pub views: Option<Vec<String>>,
    pub source: LivePartSource,
}

pub(crate) enum LivePartSource {
    Registry {
        requirement: String,
        live_alias: Option<String>,
        resolved: ResolvedRegistryDependency,
    },
    File {
        /// `path:<manifest-relative file>`, or the file as given.
        source: String,
        artifact: LiveSpecArtifactV2,
    },
}

/// One program of a composition, with its source resolved.
pub(crate) enum ProgramPart {
    Registry {
        requirement: String,
        resolved: Box<ResolvedRegistryDependency>,
    },
    File {
        source: String,
        artifact: Box<ProgramSpecArtifact>,
    },
}

/// A program chosen for the composed stack.
struct ChosenProgram {
    spec: ProgramSpecArtifact,
    descriptor: Option<RegistryProgramInstallResponse>,
    /// How the install output names where it came from.
    label: String,
}

fn program_label(install: &RegistryProgramInstallResponse) -> String {
    match &install.program_package {
        Some(package) => format!("{}@{}", package.package, package.version),
        None => "its core program".to_string(),
    }
}

/// Compose `lives` and `programs` into one stack named `name`.
pub(crate) fn compose_parts(
    name: &str,
    lives: Vec<LivePart>,
    programs: Vec<ProgramPart>,
) -> Result<ComposedStack> {
    if lives.is_empty() {
        bail!(
            "composed stack '{name}' has no live views; a group of programs alone is installed with [dependencies.programs]"
        );
    }
    let mut notes = Vec::new();
    let mut parts = Vec::new();
    let mut registry = Vec::new();
    let mut live_specs = Vec::<(String, LiveSpecArtifactV2)>::new();
    let mut selected = Vec::new();
    let mut hosted = Vec::new();
    let mut gateways = Vec::new();
    // Program SDKs the source stacks bring, by ProgramSpec hash.
    let mut brought = BTreeMap::<String, Vec<(RegistryProgramInstallResponse, String)>>::new();
    let mut sources = BTreeSet::new();

    for live in lives {
        let LivePart {
            alias,
            views,
            source,
        } = live;
        match source {
            LivePartSource::Registry {
                requirement,
                live_alias,
                resolved,
            } => {
                let ResolvedRegistryDependency::Stack {
                    package,
                    version,
                    package_release_hash,
                    stack_manifest_hash,
                    stack_manifest,
                    live_specs: source_lives,
                    programs: source_programs,
                    sdk_extensions,
                    delivery,
                    ..
                } = &resolved
                else {
                    bail!("live '{alias}' of composed stack '{name}' did not resolve to a stack");
                };
                let source_stack = verify_registry_stack_artifacts(
                    stack_manifest_hash,
                    stack_manifest,
                    source_lives,
                    source_programs,
                )
                .with_context(|| format!("stack '{package}' (live '{alias}')"))?;
                let available = source_stack
                    .live_specs
                    .iter()
                    .map(|(alias, _)| alias.as_str())
                    .collect::<Vec<_>>();
                let (source_alias, artifact) = match (&live_alias, source_stack.live_specs.as_slice())
                {
                    (Some(wanted), lives) => lives
                        .iter()
                        .find(|(alias, _)| alias == wanted)
                        .cloned()
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "live '{alias}' of composed stack '{name}' names live_alias '{wanted}', but stack '{package}' has [{}]",
                                available.join(", ")
                            )
                        })?,
                    (None, [only]) => only.clone(),
                    (None, []) => bail!(
                        "live '{alias}' of composed stack '{name}': stack '{package}' has no live views"
                    ),
                    (None, _) => bail!(
                        "live '{alias}' of composed stack '{name}': stack '{package}' has live views [{}]; set `live_alias` to choose one",
                        available.join(", ")
                    ),
                };
                if !sources.insert((package.clone(), source_alias.clone())) {
                    bail!(
                        "composed stack '{name}' reads live '{source_alias}' of stack '{package}' under more than one alias; compose it once"
                    );
                }
                let served = source_stack
                    .stack_manifest
                    .payload
                    .selected_views
                    .iter()
                    .filter(|view| view.live_alias == source_alias)
                    .map(|view| view.view_id.clone())
                    .collect::<Vec<_>>();
                let binding = match delivery.as_deref() {
                    Some(ResolvedStackDelivery::Hosted {
                        live_bindings,
                        chain_binding,
                        transaction_binding,
                        ..
                    }) => {
                        let binding = live_bindings
                            .iter()
                            .find(|binding| {
                                binding.alias == source_alias
                                    && binding.live_spec_hash == artifact.artifact_hash.to_string()
                            })
                            .ok_or_else(|| {
                                anyhow::anyhow!(
                                    "stack '{package}' is hosted but has no delivery for live '{source_alias}'"
                                )
                            })?;
                        gateways.push((
                            alias.clone(),
                            chain_binding.as_deref().cloned(),
                            transaction_binding.as_deref().cloned(),
                        ));
                        Some(binding.binding.clone())
                    }
                    _ => None,
                };
                let views = select_views(
                    name,
                    &alias,
                    &artifact,
                    views,
                    (!served.is_empty()).then_some(served.as_slice()),
                    binding.is_some().then_some(package.as_str()),
                )?;
                selected.extend(views.into_iter().map(|view_id| SelectedViewV2 {
                    live_alias: alias.clone(),
                    view_id,
                }));
                match binding {
                    Some(binding) => hosted.push(HostedLive {
                        descriptor: RegistryLiveSpecInstallDescriptor {
                            alias: alias.clone(),
                            live_spec_hash: artifact.artifact_hash.to_string(),
                            artifact: serde_json::to_value(&artifact)
                                .context("Failed to encode a composed LiveSpec")?,
                            binding,
                        },
                        release: StackRelease {
                            stack_manifest_hash: stack_manifest_hash.clone(),
                            live_alias: source_alias.clone(),
                        },
                    }),
                    None => notes.push(format!(
                        "note: live '{alias}' comes from stack '{package}', which has no hosted delivery, so its SDK has no endpoints (definition-only). Deploy it with `a4 up {name}`, or record endpoints for it under the dependency in arete.toml."
                    )),
                }
                if !sdk_extensions.is_empty() {
                    notes.push(format!(
                        "note: stack '{package}' has a stack extension; composed stack '{name}' includes its live views and program SDKs, not that extension."
                    ));
                }
                for program in source_programs {
                    brought
                        .entry(program.definition.program_spec_hash.clone())
                        .or_default()
                        .push((program.clone(), package.clone()));
                }
                parts.push(LockedPart {
                    kind: DependencyKind::Stack,
                    source: format!("registry:{package}"),
                    live: Some(alias.clone()),
                    requirement: Some(requirement),
                    version: Some(version.clone()),
                    package_release_hash: Some(package_release_hash.clone()),
                    stack_manifest_hash: Some(stack_manifest_hash.clone()),
                    live_alias: Some(source_alias),
                    program_id: None,
                    artifact_hash: artifact.artifact_hash.to_string(),
                });
                live_specs.push((alias, artifact));
                registry.push(resolved);
            }
            LivePartSource::File { source, artifact } => {
                let views = select_views(name, &alias, &artifact, views, None, None)?;
                selected.extend(views.into_iter().map(|view_id| SelectedViewV2 {
                    live_alias: alias.clone(),
                    view_id,
                }));
                notes.push(format!(
                    "note: live '{alias}' comes from a LiveSpec file, so its SDK has no endpoints (definition-only). Deploy it with `a4 up {name}`, or record endpoints for it under the dependency in arete.toml."
                ));
                parts.push(LockedPart {
                    kind: DependencyKind::Stack,
                    source,
                    live: Some(alias.clone()),
                    requirement: None,
                    version: None,
                    package_release_hash: None,
                    stack_manifest_hash: None,
                    live_alias: None,
                    program_id: None,
                    artifact_hash: artifact.artifact_hash.to_string(),
                });
                live_specs.push((alias, artifact));
            }
        }
    }

    let mut explicit = Vec::<ChosenProgram>::new();
    for program in programs {
        match program {
            ProgramPart::Registry {
                requirement,
                resolved,
            } => {
                let ResolvedRegistryDependency::Program {
                    package,
                    version,
                    package_release_hash,
                    install,
                    sdk_extensions,
                    ..
                } = resolved.as_ref()
                else {
                    bail!("program '{}' of composed stack '{name}' did not resolve to a program package", resolved.package());
                };
                // The package being composed is the program SDK, with the
                // same identity and extensions a standalone install has.
                let mut install = (**install).clone();
                install.program_package = Some(RegistryProgramPackageReference {
                    package: package.clone(),
                    version: version.clone(),
                    package_release_hash: package_release_hash.clone(),
                });
                install.sdk_extensions = Some(sdk_extensions.clone());
                let spec = program_spec_artifact_from_registry(&install)?;
                parts.push(LockedPart {
                    kind: DependencyKind::Program,
                    source: format!("registry:{package}"),
                    live: None,
                    requirement: Some(requirement),
                    version: Some(version.clone()),
                    package_release_hash: Some(package_release_hash.clone()),
                    stack_manifest_hash: None,
                    live_alias: None,
                    program_id: Some(spec.payload.program_id.clone()),
                    artifact_hash: spec.artifact_hash.to_string(),
                });
                explicit.push(ChosenProgram {
                    spec,
                    label: format!("{package}@{version}"),
                    descriptor: Some(install),
                });
                registry.push(*resolved);
            }
            ProgramPart::File { source, artifact } => {
                parts.push(LockedPart {
                    kind: DependencyKind::Program,
                    source: source.clone(),
                    live: None,
                    requirement: None,
                    version: None,
                    package_release_hash: None,
                    stack_manifest_hash: None,
                    live_alias: None,
                    program_id: Some(artifact.payload.program_id.clone()),
                    artifact_hash: artifact.artifact_hash.to_string(),
                });
                explicit.push(ChosenProgram {
                    spec: *artifact,
                    descriptor: None,
                    label: source.trim_start_matches("path:").to_string(),
                });
            }
        }
    }
    let mut program_ids = BTreeSet::new();
    for program in &explicit {
        if !program_ids.insert(program.spec.payload.program_id.as_str()) {
            bail!(
                "composed stack '{name}' lists program {} more than once (last as {})",
                program.spec.payload.program_id,
                program.label
            );
        }
    }

    // The programs the live views require, in first-use order: an explicit
    // entry for the same program wins over the program SDK a source stack
    // brings; otherwise the source stacks' program SDK is used.
    let mut required = Vec::<(String, String, String)>::new();
    let mut required_hashes = BTreeSet::new();
    for (alias, live) in &live_specs {
        for requirement in &live.payload.programs {
            let hash = requirement.program_spec_hash.to_string();
            if required_hashes.insert(hash.clone()) {
                required.push((hash, requirement.program_id.clone(), alias.clone()));
            }
        }
    }
    let mut added = Vec::<ChosenProgram>::new();
    for (hash, program_id, alias) in required {
        let candidates = brought.get(&hash).map(Vec::as_slice).unwrap_or_default();
        if let Some(chosen) = explicit
            .iter()
            .find(|program| program.spec.payload.program_id == program_id)
        {
            if chosen.spec.artifact_hash.to_string() != hash {
                bail!(
                    "live '{alias}' of composed stack '{name}' requires program {program_id} as ProgramSpec {hash}, but the `programs` entry {} is ProgramSpec {}; its views cannot be read with that program. Use a release of it with the same ProgramSpec, or remove the entry to keep the one the stack brings",
                    chosen.label,
                    chosen.spec.artifact_hash
                );
            }
            let ours = chosen
                .descriptor
                .as_ref()
                .and_then(|install| install.program_package.as_ref())
                .map(|package| package.package_release_hash.as_str());
            let mut noted = BTreeSet::new();
            for (candidate, stack) in candidates {
                let theirs = candidate
                    .program_package
                    .as_ref()
                    .map(|package| package.package_release_hash.as_str());
                if theirs != ours && noted.insert(stack.as_str()) {
                    notes.push(format!(
                        "note: program {}: the `programs` entry {} replaces the program SDK stack '{stack}' brings ({}).",
                        chosen.spec.payload.idl_snapshot.snapshot.name,
                        chosen.label,
                        program_label(candidate)
                    ));
                }
            }
            continue;
        }
        let mut distinct = BTreeMap::<Option<&str>, (&RegistryProgramInstallResponse, &str)>::new();
        for (candidate, stack) in candidates {
            distinct
                .entry(
                    candidate
                        .program_package
                        .as_ref()
                        .map(|package| package.package_release_hash.as_str()),
                )
                .or_insert((candidate, stack.as_str()));
        }
        match distinct.into_values().collect::<Vec<_>>().as_slice() {
            [] => bail!(
                "live '{alias}' of composed stack '{name}' requires program {program_id} (ProgramSpec {hash}), which no composed stack provides; add it under `programs` as a program package or a ProgramSpec file"
            ),
            [(install, _)] => added.push(ChosenProgram {
                spec: program_spec_artifact_from_registry(install)?,
                descriptor: Some((*install).clone()),
                label: program_label(install),
            }),
            several => bail!(
                "composed stack '{name}' gets program {program_id} from several stacks at different program SDK releases ({}); add a `programs` entry for it to choose one",
                several
                    .iter()
                    .map(|(install, stack)| format!("{} from stack '{stack}'", program_label(install)))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }

    let chosen = explicit.into_iter().chain(added).collect::<Vec<_>>();
    let program_specs = chosen
        .iter()
        .map(|program| program.spec.clone())
        .collect::<Vec<_>>();
    let stack_manifest = arete_artifacts::stack_manifest_v2(
        name,
        &program_specs,
        live_specs
            .iter()
            .map(|(alias, live)| (alias.clone(), live))
            .collect(),
        selected,
    )
    .map_err(|error| anyhow::anyhow!("composed stack '{name}' does not compose: {error}"))?;
    arete_interpreter::public_artifacts::stack_specs_from_artifacts_v2(
        &program_specs,
        &live_specs,
        &stack_manifest,
    )
    .map_err(|error| anyhow::anyhow!("composed stack '{name}' does not compose: {error}"))?;

    let (chain_binding, transaction_binding) = shared_gateway(name, gateways, &mut notes);
    Ok(ComposedStack {
        name: name.to_string(),
        stack_manifest,
        live_specs,
        program_specs,
        programs: chosen
            .into_iter()
            .filter_map(|program| program.descriptor)
            .collect(),
        hosted,
        chain_binding,
        transaction_binding,
        parts,
        notes,
        registry,
    })
}

/// The views a composed alias selects: the ones asked for, which must exist
/// in the LiveSpec and, for a hosted source, be among the views its
/// deployment serves; otherwise everything the source serves (or, with no
/// source selection, every view of the LiveSpec).
fn select_views(
    name: &str,
    alias: &str,
    live: &LiveSpecArtifactV2,
    requested: Option<Vec<String>>,
    served: Option<&[String]>,
    hosted_by: Option<&str>,
) -> Result<Vec<String>> {
    let defined = arete_artifacts::selected_views(alias, &live.payload)
        .into_iter()
        .map(|view| view.view_id)
        .collect::<Vec<_>>();
    let Some(requested) = requested else {
        return Ok(served.map(<[String]>::to_vec).unwrap_or(defined));
    };
    for view in &requested {
        if !defined.contains(view) {
            bail!(
                "live '{alias}' of composed stack '{name}' selects view '{view}', which its LiveSpec does not define; it has [{}]",
                defined.join(", ")
            );
        }
        if let (Some(stack), Some(served)) = (hosted_by, served) {
            if !served.contains(view) {
                bail!(
                    "live '{alias}' of composed stack '{name}' selects view '{view}', which stack '{stack}' does not serve; it serves [{}]",
                    served.join(", ")
                );
            }
        }
    }
    Ok(requested)
}

type Gateway = (
    String,
    Option<RegistryCapabilityInstallBinding>,
    Option<RegistryCapabilityInstallBinding>,
);

/// The managed Solana gateway the composed stack's session uses: the one
/// every hosted source stack uses. Sources with different gateways leave the
/// session without one (each program SDK keeps its own) and say so.
fn shared_gateway(
    name: &str,
    gateways: Vec<Gateway>,
    notes: &mut Vec<String>,
) -> (
    Option<RegistryCapabilityInstallBinding>,
    Option<RegistryCapabilityInstallBinding>,
) {
    let Some((_, chain, transactions)) = gateways.first().cloned() else {
        return (None, None);
    };
    if gateways.iter().all(|(_, other_chain, other_transactions)| {
        other_chain == &chain && other_transactions == &transactions
    }) {
        return (chain, transactions);
    }
    notes.push(format!(
        "note: the hosted live views of composed stack '{name}' ({}) use different managed Solana gateways, so its session has none; pass `chain` and `transactions` to the session, or use each program SDK's own.",
        gateways
            .iter()
            .map(|(alias, _, _)| alias.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    ));
    (None, None)
}

/// The composed authoring entry `name`.
pub(crate) fn composed_entry<'a>(
    manifest: &'a ProjectManifest,
    name: &str,
) -> Result<&'a AuthoringStackV1> {
    let entry = manifest
        .document
        .authoring
        .stacks
        .get(name)
        .ok_or_else(|| anyhow::anyhow!("arete.toml has no [authoring.stacks.{name}]"))?;
    if !entry.is_composed() {
        bail!("[authoring.stacks.{name}] names a StackManifest file, not composed parts");
    }
    Ok(entry)
}

/// Loads a composed entry's local LiveSpec and ProgramSpec files, without
/// resolving any registry part.
pub(crate) fn validate_composition_files(manifest: &ProjectManifest, name: &str) -> Result<()> {
    let entry = composed_entry(manifest, name)?;
    let paths = ProjectPaths::new(&manifest.root, false, false)?;
    for (alias, live) in &entry.live {
        if let ComposedLiveSource::Path(path) = live.source() {
            load_live_file(&paths, path)
                .with_context(|| format!("composed stack '{name}' live '{alias}'"))?;
        }
    }
    for program in &entry.programs {
        if let ComposedProgramSource::Path(path) = program.source() {
            load_program_file(&paths, path)
                .with_context(|| format!("composed stack '{name}' program '{path}'"))?;
        }
    }
    Ok(())
}

fn load_live_file(paths: &ProjectPaths, path: &str) -> Result<LiveSpecArtifactV2> {
    let file = paths.input(path, "composed LiveSpec")?;
    let bytes =
        fs::read(&file).with_context(|| format!("Failed to read LiveSpec {}", file.display()))?;
    Ok(arete_artifacts::load_live_spec_v2(&bytes)
        .with_context(|| format!("Invalid LiveSpec {}", file.display()))?
        .artifact)
}

fn load_program_file(paths: &ProjectPaths, path: &str) -> Result<ProgramSpecArtifact> {
    let file = paths.input(path, "composed ProgramSpec")?;
    let bytes = fs::read(&file)
        .with_context(|| format!("Failed to read ProgramSpec {}", file.display()))?;
    Ok(arete_artifacts::load_program_spec(&bytes)
        .with_context(|| format!("Invalid ProgramSpec {}", file.display()))?
        .artifact)
}

/// The resolver request alias of a part: portable, and unique within the
/// composition's batch per kind.
fn request_alias(value: &str) -> String {
    super::alias::derive_local_alias(value)
}

/// Resolves and composes `[authoring.stacks.<name>]` for the dependency
/// `dependency`. `previous` is the dependency's reusable lock entry: its
/// registry parts are requested at exactly their locked releases, and a
/// program SDK that moved under a locked source stack is an integrity
/// failure rather than an update.
pub(crate) fn resolve_composition(
    manifest: &ProjectManifest,
    name: &str,
    dependency: &str,
    previous: Option<&LockedDependency>,
) -> Result<ComposedStack> {
    let entry = composed_entry(manifest, name)?;
    let paths = ProjectPaths::new(&manifest.root, false, false)?;
    let locked_part =
        |kind: DependencyKind, source: &str, live: Option<&str>, requirement: &str| {
            previous
                .into_iter()
                .flat_map(|entry| &entry.parts)
                .find(|part| {
                    part.kind == kind
                        && part.source == source
                        && part.live.as_deref() == live
                        && part.requirement.as_deref() == Some(requirement)
                })
                .and_then(|part| part.package_release_hash.clone())
        };

    let mut requests = Vec::new();
    let mut request_aliases = BTreeSet::new();
    let mut lives = Vec::new();
    for (alias, live) in &entry.live {
        let source = match live.source() {
            ComposedLiveSource::Registry {
                stack,
                requirement,
                live_alias,
            } => {
                let request = request_alias(alias);
                if !request_aliases.insert((DependencyKind::Stack, request.clone())) {
                    bail!(
                        "composed stack '{name}' has live aliases that differ only in case or separators ('{alias}')"
                    );
                }
                let locked = locked_part(
                    DependencyKind::Stack,
                    &format!("registry:{stack}"),
                    Some(alias),
                    requirement,
                );
                requests.push(RegistryDependencyRequest {
                    kind: DependencyKind::Stack,
                    alias: request,
                    package: stack.to_string(),
                    requirement: requirement.to_string(),
                    locked_package_release_hash: locked,
                    locked_programs: Vec::new(),
                });
                Pending::Registry {
                    requirement: requirement.to_string(),
                    live_alias: live_alias.map(str::to_string),
                }
            }
            ComposedLiveSource::Path(path) => Pending::File {
                source: format!("path:{}", normalized(path)),
                artifact: load_live_file(&paths, path)
                    .with_context(|| format!("composed stack '{name}' live '{alias}'"))?,
            },
        };
        lives.push((alias.clone(), live.views.clone(), source));
    }
    let mut programs = Vec::new();
    for program in &entry.programs {
        let source = match program.source() {
            ComposedProgramSource::Registry {
                package,
                requirement,
            } => {
                let request = request_alias(package);
                if !request_aliases.insert((DependencyKind::Program, request.clone())) {
                    bail!(
                        "composed stack '{name}' lists program packages that differ only in case or separators ('{package}')"
                    );
                }
                requests.push(RegistryDependencyRequest {
                    kind: DependencyKind::Program,
                    alias: request,
                    package: package.to_string(),
                    requirement: requirement.to_string(),
                    locked_package_release_hash: locked_part(
                        DependencyKind::Program,
                        &format!("registry:{package}"),
                        None,
                        requirement,
                    ),
                    locked_programs: Vec::new(),
                });
                PendingProgram::Registry {
                    requirement: requirement.to_string(),
                }
            }
            ComposedProgramSource::Path(path) => PendingProgram::File {
                source: format!("path:{}", normalized(path)),
                artifact: Box::new(
                    load_program_file(&paths, path)
                        .with_context(|| format!("composed stack '{name}' program '{path}'"))?,
                ),
            },
        };
        programs.push(source);
    }

    let mut responses = if requests.is_empty() {
        Vec::new()
    } else {
        super::installer::resolve_registry_batch(
            manifest.document.manifest_version,
            &manifest.document.sdk.targets,
            &requests,
            Some(dependency),
        )?
    }
    .into_iter();

    let lives: Vec<LivePart> = lives
        .into_iter()
        .map(|(alias, views, pending)| LivePart {
            alias,
            views,
            source: match pending {
                Pending::Registry {
                    requirement,
                    live_alias,
                } => LivePartSource::Registry {
                    requirement,
                    live_alias,
                    resolved: responses.next().expect("one response per request"),
                },
                Pending::File { source, artifact } => LivePartSource::File { source, artifact },
            },
        })
        .collect();
    let programs = programs
        .into_iter()
        .map(|pending| match pending {
            PendingProgram::Registry { requirement } => ProgramPart::Registry {
                requirement,
                resolved: Box::new(responses.next().expect("one response per request")),
            },
            PendingProgram::File { source, artifact } => ProgramPart::File { source, artifact },
        })
        .collect();
    if let Some(previous) = previous {
        for live in &lives {
            if let LivePartSource::Registry { resolved, .. } = &live.source {
                verify_locked_stack_program_sdks(
                    dependency,
                    &live.alias,
                    resolved,
                    &requests,
                    previous,
                )?;
            }
        }
    }
    compose_parts(name, lives, programs)
}

enum Pending {
    Registry {
        requirement: String,
        live_alias: Option<String>,
    },
    File {
        source: String,
        artifact: LiveSpecArtifactV2,
    },
}

enum PendingProgram {
    Registry {
        requirement: String,
    },
    File {
        source: String,
        artifact: Box<ProgramSpecArtifact>,
    },
}

fn normalized(path: &str) -> String {
    Path::new(path)
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(value) => Some(value.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// A stack part requested at its locked release must bring the program SDK
/// releases arete.lock pins for the programs it provided: one that moved
/// underneath the locked stack release is an integrity failure, as it is for
/// an installed stack. A program the lock pinned from an explicit `programs`
/// part was not this stack's, so editing that part away re-resolves it
/// instead, and a program the lock pins no program SDK release for pins
/// nothing to compare.
fn verify_locked_stack_program_sdks(
    dependency: &str,
    alias: &str,
    resolved: &ResolvedRegistryDependency,
    requests: &[RegistryDependencyRequest],
    previous: &LockedDependency,
) -> Result<()> {
    let ResolvedRegistryDependency::Stack {
        package, programs, ..
    } = resolved
    else {
        return Ok(());
    };
    let locked = requests.iter().any(|request| {
        request.kind == DependencyKind::Stack
            && request.alias == request_alias(alias)
            && request.locked_package_release_hash.is_some()
    });
    if !locked {
        return Ok(());
    }
    let explicit = previous
        .parts
        .iter()
        .filter(|part| part.kind == DependencyKind::Program)
        .map(|part| part.artifact_hash.as_str())
        .collect::<BTreeSet<_>>();
    for program in programs {
        let hash = program.definition.program_spec_hash.as_str();
        if explicit.contains(hash) {
            continue;
        }
        let Some(pinned) = previous
            .programs
            .iter()
            .find(|locked| locked.program_spec_hash == hash)
            .and_then(|locked| locked.package_release_hash.as_deref())
        else {
            continue;
        };
        let resolved = program
            .program_package
            .as_ref()
            .map(|package| package.package_release_hash.as_str());
        if resolved != Some(pinned) {
            bail!(
                "arete.lock integrity failure for composed stack '{dependency}': the locked release of stack '{package}' (live '{alias}') pins program {} to program SDK release {pinned}, but the registry now returns {}. Nothing was changed; run `a4 update stack {dependency}` only if you intend to advance",
                program.definition.program_id,
                resolved.unwrap_or("no program SDK release"),
            );
        }
    }
    Ok(())
}

/// Writes a composed stack's StackManifest, LiveSpecs and ProgramSpecs to
/// `.arete/compositions/<name>/`, replacing what was there, so `a4 up <name>`
/// deploys exactly what arete.lock pins.
pub(crate) fn write_composition_artifacts(
    project_root: &Path,
    composed: &ComposedStack,
) -> Result<()> {
    let root = project_root.join(COMPOSITIONS_DIR);
    fs::create_dir_all(&root).with_context(|| format!("Failed to create {}", root.display()))?;
    let staging = root.join(format!(".{}.{}.tmp", composed.name, uuid::Uuid::new_v4()));
    let result = (|| -> Result<()> {
        fs::create_dir_all(&staging)?;
        write_artifact(
            &staging.join(format!("{}.stack-manifest.json", composed.name)),
            &composed.stack_manifest,
        )?;
        let mut written = BTreeSet::new();
        for (alias, live) in &composed.live_specs {
            // One file per LiveSpec: a second copy would make it ambiguous.
            if written.insert(live.artifact_hash.to_string()) {
                write_artifact(&staging.join(format!("{alias}.live-spec.json")), live)?;
            }
        }
        for program in &composed.program_specs {
            write_artifact(
                &staging.join(format!("{}.program-spec.json", program.payload.program_id)),
                program,
            )?;
        }
        let target = composition_dir(project_root, &composed.name);
        if target.exists() {
            fs::remove_dir_all(&target)
                .with_context(|| format!("Failed to replace {}", target.display()))?;
        }
        fs::rename(&staging, &target)
            .with_context(|| format!("Failed to write {}", target.display()))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

/// `.arete/compositions/<name>`.
pub(crate) fn composition_dir(project_root: &Path, name: &str) -> PathBuf {
    project_root.join(COMPOSITIONS_DIR).join(name)
}

fn write_artifact(path: &Path, value: &impl serde::Serialize) -> Result<()> {
    let bytes = arete_hash::canonicalize_jcs(value)?;
    arete_artifacts::atomic_write(path, &bytes)
        .with_context(|| format!("Failed to write {}", path.display()))
}
