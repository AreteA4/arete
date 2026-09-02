use std::collections::{BTreeMap, BTreeSet};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use arete_hash::{InstructionDefinitionV1, LiveSpec, PdaDefinitionV1, ProgramSpec, ProgramSpecV1};
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::{
    ArtifactError, LiveSpecArtifact, LiveSpecArtifactV2, LiveSpecReferenceV2, LiveSpecV2,
    PortableEntity, PortableFieldPath, PortableView, PortableViewOutput, PortableViewSource,
    ProgramAdapterV2, ProgramRequirementV2, ProgramSpecArtifact, ProgramSpecReferenceV2,
    SelectedViewV2, StackManifestArtifact, StackManifestArtifactV2, StackManifestV2,
};

pub const DEFAULT_LIVE_ALIAS: &str = "live";

#[derive(Debug, Clone)]
pub struct StackAuthoringV2 {
    pub name: String,
    pub programs: Vec<ProgramSpecV1>,
    pub entities: Vec<PortableEntity>,
    pub pda_overrides: BTreeMap<String, BTreeMap<String, PdaDefinitionV1>>,
    pub instruction_overrides: Vec<InstructionDefinitionV1>,
    pub live_alias: String,
}

impl StackAuthoringV2 {
    pub fn new(
        name: impl Into<String>,
        programs: Vec<ProgramSpecV1>,
        entities: Vec<PortableEntity>,
    ) -> Self {
        Self {
            name: name.into(),
            programs,
            entities,
            pda_overrides: BTreeMap::new(),
            instruction_overrides: Vec::new(),
            live_alias: DEFAULT_LIVE_ALIAS.to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AuthoredStackV2 {
    pub program_specs: Vec<ProgramSpecArtifact>,
    pub live_spec: Option<LiveSpecArtifactV2>,
    pub stack_manifest: StackManifestArtifactV2,
}

pub fn program_spec_v1(payload: ProgramSpecV1) -> Result<ProgramSpecArtifact, ArtifactError> {
    ProgramSpecArtifact::new(payload)
}

pub fn default_views(entity: &PortableEntity) -> Vec<PortableView> {
    let mut views = Vec::new();
    if let Some(primary_key) = entity.identity.primary_keys.first() {
        views.push(PortableView {
            id: format!("{}/state", entity.state_name),
            source: PortableViewSource::Entity {
                name: entity.state_name.clone(),
            },
            pipeline: Vec::new(),
            output: PortableViewOutput::Keyed {
                key_field: PortableFieldPath::new(primary_key.split('.')),
            },
        });
    }
    views.push(PortableView {
        id: format!("{}/list", entity.state_name),
        source: PortableViewSource::Entity {
            name: entity.state_name.clone(),
        },
        pipeline: Vec::new(),
        output: PortableViewOutput::Collection,
    });
    views
}

pub fn with_default_views(mut entity: PortableEntity) -> Result<PortableEntity, ArtifactError> {
    let mut seen_primary_keys = BTreeSet::new();
    entity
        .identity
        .primary_keys
        .retain(|primary_key| seen_primary_keys.insert(primary_key.clone()));
    for expected in default_views(&entity) {
        match entity.views.iter().find(|view| view.id == expected.id) {
            Some(existing) if existing != &expected => {
                return Err(ArtifactError::InvalidArtifact(format!(
                    "entity '{}' defines conflicting default view '{}'",
                    entity.state_name, expected.id
                )));
            }
            Some(_) => {}
            None => entity.views.push(expected),
        }
    }
    entity.validate()?;
    Ok(entity)
}

pub fn selected_views(alias: &str, live: &LiveSpecV2) -> Vec<SelectedViewV2> {
    live.entities
        .iter()
        .flat_map(|entity| {
            entity.views.iter().map(|view| SelectedViewV2 {
                live_alias: alias.to_string(),
                view_id: view.id.clone(),
            })
        })
        .collect()
}

pub fn live_spec_v2(
    programs: &[ProgramSpecArtifact],
    entities: Vec<PortableEntity>,
    program_adapters: Vec<ProgramAdapterV2>,
) -> Result<LiveSpecArtifactV2, ArtifactError> {
    let entities = entities
        .into_iter()
        .map(with_default_views)
        .collect::<Result<Vec<_>, _>>()?;
    let requirements = programs
        .iter()
        .map(|program| ProgramRequirementV2 {
            program_id: program.payload.program_id.clone(),
            program_spec_hash: program.artifact_hash,
        })
        .collect();
    LiveSpecArtifactV2::new(LiveSpecV2::new(requirements, entities, program_adapters))
}

pub fn stack_manifest_v2(
    name: impl Into<String>,
    programs: &[ProgramSpecArtifact],
    live_specs: Vec<(String, &LiveSpecArtifactV2)>,
    selected_views: Vec<SelectedViewV2>,
) -> Result<StackManifestArtifactV2, ArtifactError> {
    compose_stack_manifest_v2(name, programs, live_specs, selected_views)
}

/// Compose independently authored LiveSpecs without changing the singular
/// semantics of [`author_stack_v2`].
pub fn compose_stack_manifest_v2(
    name: impl Into<String>,
    programs: &[ProgramSpecArtifact],
    live_specs: Vec<(String, &LiveSpecArtifactV2)>,
    selected_views: Vec<SelectedViewV2>,
) -> Result<StackManifestArtifactV2, ArtifactError> {
    let manifest = StackManifestV2::new(
        name,
        programs
            .iter()
            .map(|program| ProgramSpecReferenceV2 {
                program_id: program.payload.program_id.clone(),
                artifact_hash: program.artifact_hash,
            })
            .collect(),
        live_specs
            .iter()
            .map(|(alias, live)| LiveSpecReferenceV2 {
                alias: alias.clone(),
                artifact_hash: live.artifact_hash,
            })
            .collect(),
        selected_views,
    );
    let artifact = StackManifestArtifactV2::new(manifest)?;
    let owned_lives = live_specs
        .iter()
        .map(|(alias, live)| (alias.clone(), (*live).clone()))
        .collect::<Vec<_>>();
    crate::resolve_stack_composition_v2(&artifact, &owned_lives, programs)?;
    Ok(artifact)
}

pub fn author_stack_v2(input: StackAuthoringV2) -> Result<AuthoredStackV2, ArtifactError> {
    if input.name.is_empty() {
        return Err(ArtifactError::InvalidArtifact(
            "stack name must not be empty".to_string(),
        ));
    }
    let program_specs = input
        .programs
        .into_iter()
        .map(program_spec_v1)
        .collect::<Result<Vec<_>, _>>()?;
    let adapters = derive_program_adapters(
        &program_specs,
        &input.pda_overrides,
        &input.instruction_overrides,
    )?;

    if input.entities.is_empty() {
        let stack_manifest = stack_manifest_v2(input.name, &program_specs, Vec::new(), Vec::new())?;
        return Ok(AuthoredStackV2 {
            program_specs,
            live_spec: None,
            stack_manifest,
        });
    }

    let live_spec = live_spec_v2(&program_specs, input.entities, adapters)?;
    let selected = selected_views(&input.live_alias, &live_spec.payload);
    let stack_manifest = stack_manifest_v2(
        input.name,
        &program_specs,
        vec![(input.live_alias, &live_spec)],
        selected,
    )?;
    Ok(AuthoredStackV2 {
        program_specs,
        live_spec: Some(live_spec),
        stack_manifest,
    })
}

pub fn normalize_live_spec_v1(
    live: &LiveSpecArtifact,
    programs: &[ProgramSpecArtifact],
) -> Result<LiveSpecArtifactV2, ArtifactError> {
    live.validate()?;
    let required = live
        .payload
        .programs
        .iter()
        .map(|requirement| {
            (
                requirement.program_spec_hash.to_string(),
                requirement.program_id.as_str(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    if programs.len() != required.len()
        || programs.iter().any(|program| {
            required.get(&program.artifact_hash.to_string()).copied()
                != Some(program.payload.program_id.as_str())
        })
    {
        return Err(ArtifactError::InvalidArtifact(
            "V1 LiveSpec ProgramSpec dependencies do not match the supplied artifacts".to_string(),
        ));
    }

    let entities = live
        .payload
        .entities
        .iter()
        .map(transcode)
        .collect::<Result<Vec<PortableEntity>, _>>()?;
    let (pdas, instructions) = match &live.payload.legacy_program_extensions {
        Some(extensions) => (
            transcode(&extensions.pdas)?,
            transcode(&extensions.instructions)?,
        ),
        None => (BTreeMap::new(), Vec::new()),
    };
    let adapters = derive_program_adapters(programs, &pdas, &instructions)?;
    live_spec_v2(programs, entities, adapters)
}

pub fn normalize_stack_manifest_v1(
    manifest: &StackManifestArtifact,
    programs: &[ProgramSpecArtifact],
    live_specs: &[(arete_hash::HashId<LiveSpec>, String, &LiveSpecArtifactV2)],
) -> Result<StackManifestArtifactV2, ArtifactError> {
    manifest.validate()?;
    let program_refs = manifest
        .payload
        .programs
        .iter()
        .map(|reference| {
            (
                reference.artifact_hash.to_string(),
                reference.program_id.as_str(),
            )
        })
        .collect::<Vec<_>>();
    let supplied_programs = programs
        .iter()
        .map(|program| {
            (
                program.artifact_hash.to_string(),
                program.payload.program_id.as_str(),
            )
        })
        .collect::<Vec<_>>();
    if program_refs != supplied_programs {
        return Err(ArtifactError::InvalidArtifact(
            "V1 StackManifest ProgramSpec order does not match supplied artifacts".to_string(),
        ));
    }

    let normalized_by_source_hash = live_specs
        .iter()
        .map(|(source_hash, alias, live)| (source_hash.to_string(), (alias.as_str(), *live)))
        .collect::<BTreeMap<_, _>>();
    if normalized_by_source_hash.len() != manifest.payload.live_specs.len()
        || manifest.payload.live_specs.iter().any(|reference| {
            !normalized_by_source_hash.contains_key(&reference.artifact_hash.to_string())
        })
    {
        return Err(ArtifactError::InvalidArtifact(
            "V1 StackManifest LiveSpec references do not match supplied normalized artifacts"
                .to_string(),
        ));
    }
    let ordered_lives = manifest
        .payload
        .live_specs
        .iter()
        .map(|reference| {
            let (alias, live) = normalized_by_source_hash[&reference.artifact_hash.to_string()];
            (alias.to_string(), live)
        })
        .collect::<Vec<_>>();
    let selected = manifest
        .payload
        .selected_views
        .iter()
        .map(|selected| {
            normalized_by_source_hash
                .get(&selected.live_spec_hash.to_string())
                .map(|(alias, _)| SelectedViewV2 {
                    live_alias: (*alias).to_string(),
                    view_id: selected.view_id.clone(),
                })
                .ok_or_else(|| {
                    ArtifactError::InvalidArtifact(format!(
                        "selected V1 view '{}' references an unknown LiveSpec",
                        selected.view_id
                    ))
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    stack_manifest_v2(
        manifest.payload.name.clone(),
        programs,
        ordered_lives,
        selected,
    )
}

pub fn write_authored_stack_v2(
    directory: &Path,
    stack_name: &str,
    artifacts: &AuthoredStackV2,
) -> Result<Vec<PathBuf>, ArtifactError> {
    validate_file_stem(stack_name)?;
    let mut files = Vec::new();
    let mut program_file_names = BTreeSet::new();
    for program in &artifacts.program_specs {
        let name = &program.payload.idl_snapshot.snapshot.name;
        validate_file_stem(name)?;
        if !program_file_names.insert(name.as_str()) {
            return Err(ArtifactError::InvalidArtifact(format!(
                "multiple ProgramSpecs would write '{name}.program-spec.json'"
            )));
        }
        files.push((
            directory.join(format!("{name}.program-spec.json")),
            program.canonical_bytes()?,
        ));
    }
    if let Some(live) = &artifacts.live_spec {
        files.push((
            directory.join(format!("{stack_name}.live-spec.json")),
            live.canonical_bytes()?,
        ));
    }
    files.push((
        directory.join(format!("{stack_name}.stack-manifest.json")),
        artifacts.stack_manifest.canonical_bytes()?,
    ));

    std::fs::create_dir_all(directory)?;
    let mut written = Vec::with_capacity(files.len());
    for (path, bytes) in files {
        atomic_write(&path, &bytes)?;
        written.push(path);
    }
    Ok(written)
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), ArtifactError> {
    static NEXT_TEMP_FILE: AtomicU64 = AtomicU64::new(0);

    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            ArtifactError::InvalidArtifact(format!(
                "artifact path '{}' has no UTF-8 filename",
                path.display()
            ))
        })?;
    let temp_path = parent.join(format!(
        ".{file_name}.tmp-{}-{}",
        std::process::id(),
        NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| -> Result<(), std::io::Error> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temp_path, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp_path);
    }
    result.map_err(Into::into)
}

fn derive_program_adapters(
    programs: &[ProgramSpecArtifact],
    pda_overrides: &BTreeMap<String, BTreeMap<String, PdaDefinitionV1>>,
    instruction_overrides: &[InstructionDefinitionV1],
) -> Result<Vec<ProgramAdapterV2>, ArtifactError> {
    let program_names = programs
        .iter()
        .map(|program| program.payload.idl_snapshot.snapshot.name.as_str())
        .collect::<BTreeSet<_>>();
    if let Some(unknown) = pda_overrides
        .keys()
        .find(|name| !program_names.contains(name.as_str()))
    {
        return Err(ArtifactError::InvalidArtifact(format!(
            "PDA overrides reference unknown program '{unknown}'"
        )));
    }

    let mut adapters = Vec::new();
    for program in programs {
        let program_name = &program.payload.idl_snapshot.snapshot.name;
        let pdas = pda_overrides
            .get(program_name)
            .into_iter()
            .flat_map(|overrides| overrides.iter())
            .filter(|(name, value)| program.payload.pdas.get(*name) != Some(*value))
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect::<BTreeMap<_, _>>();

        let mut instruction_resolutions = Vec::new();
        for override_instruction in instruction_overrides.iter().filter(|instruction| {
            instruction.program_id.as_deref() == Some(program.payload.program_id.as_str())
                || (instruction.program_id.is_none() && programs.len() == 1)
        }) {
            let base = program
                .payload
                .instructions
                .iter()
                .find(|instruction| instruction.name == override_instruction.name)
                .ok_or_else(|| {
                    ArtifactError::InvalidArtifact(format!(
                        "instruction override '{}.{}' has no ProgramSpec instruction",
                        program_name, override_instruction.name
                    ))
                })?;
            let mut reconciled = base.clone();
            let mut accounts = BTreeMap::new();
            for override_account in &override_instruction.accounts {
                let base_account = reconciled
                    .accounts
                    .iter_mut()
                    .find(|account| account.name == override_account.name)
                    .ok_or_else(|| {
                        ArtifactError::InvalidArtifact(format!(
                            "instruction override '{}.{}' contains unknown account '{}'",
                            program_name, override_instruction.name, override_account.name
                        ))
                    })?;
                if base_account.resolution != override_account.resolution {
                    accounts.insert(
                        override_account.name.clone(),
                        override_account.resolution.clone(),
                    );
                    base_account.resolution = override_account.resolution.clone();
                }
            }
            if reconciled != *override_instruction {
                return Err(ArtifactError::InvalidArtifact(format!(
                    "instruction override '{}.{}' changes fields other than account resolution",
                    program_name, override_instruction.name
                )));
            }
            if !accounts.is_empty() {
                instruction_resolutions.push(crate::InstructionResolutionAdapterV2 {
                    instruction: override_instruction.name.clone(),
                    accounts,
                });
            }
        }
        instruction_resolutions.sort_by(|left, right| left.instruction.cmp(&right.instruction));
        if !pdas.is_empty() || !instruction_resolutions.is_empty() {
            adapters.push(ProgramAdapterV2 {
                program_spec_hash: program.artifact_hash,
                pdas,
                instruction_resolutions,
            });
        }
    }

    let recognized = programs
        .iter()
        .flat_map(|program| {
            program.payload.instructions.iter().map(move |instruction| {
                (
                    program.payload.program_id.as_str(),
                    instruction.name.as_str(),
                )
            })
        })
        .collect::<BTreeSet<_>>();
    for instruction in instruction_overrides {
        let matches = instruction.program_id.as_deref().map_or_else(
            || {
                programs.len() == 1
                    && recognized.contains(&(
                        programs[0].payload.program_id.as_str(),
                        instruction.name.as_str(),
                    ))
            },
            |program_id| recognized.contains(&(program_id, instruction.name.as_str())),
        );
        if !matches {
            return Err(ArtifactError::InvalidArtifact(format!(
                "instruction override '{}' does not match a ProgramSpec instruction",
                instruction.name
            )));
        }
    }
    Ok(adapters)
}

fn transcode<T: Serialize, U: DeserializeOwned>(value: &T) -> Result<U, ArtifactError> {
    serde_json::from_value(serde_json::to_value(value).map_err(crate::json_error)?)
        .map_err(crate::json_error)
}

fn validate_file_stem(value: &str) -> Result<(), ArtifactError> {
    if value.is_empty()
        || !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err(ArtifactError::InvalidArtifact(format!(
            "'{value}' is not safe for an artifact filename"
        )));
    }
    Ok(())
}

#[allow(dead_code)]
fn _type_markers(_: arete_hash::HashId<ProgramSpec>, _: arete_hash::HashId<LiveSpec>) {}
