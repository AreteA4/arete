use std::collections::{BTreeMap, BTreeSet};

use arete_hash::{hash_jcs, HashId, LiveSpec, ProgramSpec, StackManifest};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    json_error, reject_private_fields, validate_envelope_version, validate_kind, ArtifactError,
    LiveSpecArtifactV2, ProgramSpecArtifact, ARTIFACT_VERSION_V1, STACK_MANIFEST_KIND,
    STACK_MANIFEST_SCHEMA_V2,
};

#[derive(Debug, Clone)]
pub struct ResolvedLiveSpecV2<'a> {
    pub alias: String,
    pub artifact: &'a LiveSpecArtifactV2,
    pub program_specs: Vec<&'a ProgramSpecArtifact>,
    pub selected_views: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ResolvedStackCompositionV2<'a> {
    /// ProgramSpecs in portable manifest order.
    pub program_specs: Vec<&'a ProgramSpecArtifact>,
    /// LiveSpecs in portable manifest alias order.
    pub live_specs: Vec<ResolvedLiveSpecV2<'a>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProgramSpecReferenceV2 {
    pub program_id: String,
    pub artifact_hash: HashId<ProgramSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LiveSpecReferenceV2 {
    pub alias: String,
    pub artifact_hash: HashId<LiveSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SelectedViewV2 {
    pub live_alias: String,
    pub view_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StackManifestV2 {
    pub schema: String,
    pub name: String,
    pub programs: Vec<ProgramSpecReferenceV2>,
    pub live_specs: Vec<LiveSpecReferenceV2>,
    pub selected_views: Vec<SelectedViewV2>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub queries: Vec<Value>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extensions: BTreeMap<String, Value>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, Value>,
}

impl StackManifestV2 {
    pub fn new(
        name: impl Into<String>,
        programs: Vec<ProgramSpecReferenceV2>,
        live_specs: Vec<LiveSpecReferenceV2>,
        selected_views: Vec<SelectedViewV2>,
    ) -> Self {
        Self {
            schema: STACK_MANIFEST_SCHEMA_V2.to_string(),
            name: name.into(),
            programs,
            live_specs,
            selected_views,
            queries: Vec::new(),
            extensions: BTreeMap::new(),
            metadata: BTreeMap::new(),
        }
    }

    pub fn validate(&self) -> Result<(), ArtifactError> {
        if self.schema != STACK_MANIFEST_SCHEMA_V2 {
            return Err(ArtifactError::UnsupportedVersion {
                artifact: STACK_MANIFEST_KIND,
                version: self.schema.clone(),
            });
        }
        if self.name.is_empty() {
            return Err(ArtifactError::InvalidArtifact(
                "stack manifest name must not be empty".to_string(),
            ));
        }
        let mut program_hashes = BTreeSet::new();
        for program in &self.programs {
            if program.program_id.is_empty()
                || !program_hashes.insert(program.artifact_hash.to_string())
            {
                return Err(ArtifactError::InvalidArtifact(
                    "manifest ProgramSpec references must have unique hashes and non-empty program IDs"
                        .to_string(),
                ));
            }
        }
        let mut live_aliases = BTreeSet::new();
        for live in &self.live_specs {
            if !is_portable_alias(&live.alias) || !live_aliases.insert(live.alias.as_str()) {
                return Err(ArtifactError::InvalidArtifact(
                    "LiveSpec references must have unique portable aliases".to_string(),
                ));
            }
        }
        let mut selected = BTreeSet::new();
        for view in &self.selected_views {
            if !live_aliases.contains(view.live_alias.as_str())
                || view.view_id.is_empty()
                || !selected.insert((view.live_alias.as_str(), view.view_id.as_str()))
            {
                return Err(ArtifactError::InvalidArtifact(
                    "selected views must uniquely reference a declared LiveSpec alias".to_string(),
                ));
            }
        }
        reject_private_fields(&serde_json::to_value(self).map_err(json_error)?)
    }

    /// Validate the selected-view allowlist against exact aliased LiveSpec inputs.
    pub fn validate_selected_views<'a>(
        &self,
        live_specs: impl IntoIterator<Item = (&'a str, &'a LiveSpecArtifactV2)>,
    ) -> Result<(), ArtifactError> {
        self.validate()?;
        let mut supplied = BTreeMap::new();
        for (alias, live) in live_specs {
            if supplied.insert(alias, live).is_some() {
                return Err(ArtifactError::InvalidArtifact(format!(
                    "LiveSpec alias '{alias}' was supplied more than once"
                )));
            }
        }
        if supplied.len() != self.live_specs.len() {
            return Err(ArtifactError::InvalidArtifact(
                "exactly the manifest's aliased LiveSpecs must be supplied".to_string(),
            ));
        }
        for reference in &self.live_specs {
            let live = supplied.get(reference.alias.as_str()).ok_or_else(|| {
                ArtifactError::InvalidArtifact(format!(
                    "missing LiveSpec alias '{}'",
                    reference.alias
                ))
            })?;
            live.validate()?;
            if live.artifact_hash != reference.artifact_hash {
                return Err(ArtifactError::InvalidArtifact(format!(
                    "LiveSpec alias '{}' has the wrong artifact hash",
                    reference.alias
                )));
            }
        }
        for selected in &self.selected_views {
            let live = supplied[selected.live_alias.as_str()];
            if selected_view_owner(live, &selected.view_id).is_none() {
                return Err(ArtifactError::InvalidArtifact(format!(
                    "selected view '{}:{}' does not exist",
                    selected.live_alias, selected.view_id
                )));
            }
        }
        validate_client_names(
            self.live_specs
                .iter()
                .map(|reference| (reference.alias.as_str(), supplied[reference.alias.as_str()])),
            &self.selected_views,
        )
    }
}

/// Validate and resolve one portable V2 composition. Supplied LiveSpecs are
/// positional because alias order is part of the manifest contract; supplied
/// ProgramSpecs are resolved by typed hash and may be in any filesystem order.
pub fn resolve_stack_composition_v2<'a>(
    manifest: &StackManifestArtifactV2,
    live_specs: &'a [(String, LiveSpecArtifactV2)],
    program_specs: &'a [ProgramSpecArtifact],
) -> Result<ResolvedStackCompositionV2<'a>, ArtifactError> {
    manifest.validate()?;
    if live_specs.len() != manifest.payload.live_specs.len() {
        return Err(ArtifactError::InvalidArtifact(
            "exactly the manifest's ordered aliased LiveSpecs must be supplied".to_string(),
        ));
    }

    for ((alias, live), reference) in live_specs.iter().zip(&manifest.payload.live_specs) {
        live.validate()?;
        if alias != &reference.alias || live.artifact_hash != reference.artifact_hash {
            return Err(ArtifactError::InvalidArtifact(format!(
                "LiveSpec at manifest position for alias '{}' must have exact alias and hash",
                reference.alias
            )));
        }
    }

    manifest.payload.validate_selected_views(
        live_specs
            .iter()
            .map(|(alias, live)| (alias.as_str(), live)),
    )?;

    let mut supplied_programs = BTreeMap::new();
    for program in program_specs {
        program.validate()?;
        if supplied_programs
            .insert(program.artifact_hash.to_string(), program)
            .is_some()
        {
            return Err(ArtifactError::InvalidArtifact(format!(
                "ProgramSpec {} was supplied more than once",
                program.artifact_hash
            )));
        }
    }

    let mut required_programs = BTreeMap::<String, String>::new();
    for (_, live) in live_specs {
        for requirement in &live.payload.programs {
            let hash = requirement.program_spec_hash.to_string();
            if let Some(existing) =
                required_programs.insert(hash.clone(), requirement.program_id.clone())
            {
                if existing != requirement.program_id {
                    return Err(ArtifactError::InvalidArtifact(format!(
                        "ProgramSpec {hash} is required with conflicting program IDs '{existing}' and '{}'",
                        requirement.program_id
                    )));
                }
            }
        }
    }

    let manifest_programs = manifest
        .payload
        .programs
        .iter()
        .map(|reference| (reference.artifact_hash.to_string(), reference))
        .collect::<BTreeMap<_, _>>();
    for (hash, required_id) in &required_programs {
        let reference = manifest_programs.get(hash).ok_or_else(|| {
            ArtifactError::InvalidArtifact(format!(
                "StackManifest is missing LiveSpec-required ProgramSpec {hash}"
            ))
        })?;
        if required_id != &reference.program_id {
            return Err(ArtifactError::InvalidArtifact(format!(
                "StackManifest ProgramSpec {hash} has program ID '{}', not '{required_id}'",
                reference.program_id
            )));
        }
    }

    if supplied_programs.len() != manifest.payload.programs.len() {
        return Err(ArtifactError::InvalidArtifact(
            "supplied ProgramSpecs must exactly match the StackManifest".to_string(),
        ));
    }
    let ordered_programs = manifest
        .payload
        .programs
        .iter()
        .map(|reference| {
            let hash = reference.artifact_hash.to_string();
            let program = supplied_programs.get(&hash).copied().ok_or_else(|| {
                ArtifactError::InvalidArtifact(format!("missing required ProgramSpec {hash}"))
            })?;
            if program.payload.program_id != reference.program_id {
                return Err(ArtifactError::InvalidArtifact(format!(
                    "ProgramSpec {hash} has program ID '{}', not '{}'",
                    program.payload.program_id, reference.program_id
                )));
            }
            Ok(program)
        })
        .collect::<Result<Vec<_>, ArtifactError>>()?;

    let resolved_lives = live_specs
        .iter()
        .map(|(alias, live)| {
            let programs = live
                .payload
                .programs
                .iter()
                .map(|requirement| {
                    supplied_programs
                        .get(&requirement.program_spec_hash.to_string())
                        .copied()
                        .ok_or_else(|| {
                            ArtifactError::InvalidArtifact(format!(
                                "LiveSpec alias '{alias}' requires missing ProgramSpec {}",
                                requirement.program_spec_hash
                            ))
                        })
                })
                .collect::<Result<Vec<_>, _>>()?;
            let selected_views = manifest
                .payload
                .selected_views
                .iter()
                .filter(|selected| selected.live_alias == *alias)
                .map(|selected| selected.view_id.clone())
                .collect();
            Ok(ResolvedLiveSpecV2 {
                alias: alias.clone(),
                artifact: live,
                program_specs: programs,
                selected_views,
            })
        })
        .collect::<Result<Vec<_>, ArtifactError>>()?;

    Ok(ResolvedStackCompositionV2 {
        program_specs: ordered_programs,
        live_specs: resolved_lives,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StackManifestArtifactV2 {
    pub artifact_version: String,
    pub kind: String,
    pub artifact_hash: HashId<StackManifest>,
    pub payload: StackManifestV2,
}

impl StackManifestArtifactV2 {
    pub fn new(payload: StackManifestV2) -> Result<Self, ArtifactError> {
        payload.validate()?;
        let artifact_hash = hash_jcs(&ManifestProjection {
            artifact_version: ARTIFACT_VERSION_V1,
            kind: STACK_MANIFEST_KIND,
            payload: &payload,
        })?;
        Ok(Self {
            artifact_version: ARTIFACT_VERSION_V1.to_string(),
            kind: STACK_MANIFEST_KIND.to_string(),
            artifact_hash,
            payload,
        })
    }

    pub fn validate(&self) -> Result<(), ArtifactError> {
        validate_envelope_version(&self.artifact_version, STACK_MANIFEST_KIND)?;
        validate_kind(&self.kind, STACK_MANIFEST_KIND)?;
        self.payload.validate()?;
        let expected = hash_jcs(&ManifestProjection {
            artifact_version: &self.artifact_version,
            kind: STACK_MANIFEST_KIND,
            payload: &self.payload,
        })?;
        if expected != self.artifact_hash {
            return Err(ArtifactError::HashMismatch);
        }
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, ArtifactError> {
        self.validate()?;
        arete_hash::canonicalize_jcs(self).map_err(Into::into)
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ManifestProjection<'a> {
    artifact_version: &'a str,
    kind: &'static str,
    payload: &'a StackManifestV2,
}

pub fn load_stack_manifest_v2(
    bytes: &[u8],
) -> Result<crate::LoadedArtifact<StackManifestArtifactV2>, ArtifactError> {
    let value = arete_hash::parse_json_bytes_strict(bytes)?;
    let artifact: StackManifestArtifactV2 = serde_json::from_value(value).map_err(json_error)?;
    artifact.validate()?;
    Ok(crate::LoadedArtifact {
        artifact,
        original_bytes: bytes.to_vec(),
        source_hash: arete_hash::hash_raw_bytes(bytes)?,
    })
}

fn is_portable_alias(alias: &str) -> bool {
    !alias.is_empty()
        && alias.len() <= 64
        && alias
            .chars()
            .any(|character| character.is_ascii_alphanumeric())
        && alias
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
}

/// Comparison key shared by TypeScript property and Rust identifier generation.
/// It intentionally erases case and separators so collisions fail before codegen.
pub fn normalized_client_name(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .map(|character| character.to_ascii_lowercase())
        .collect()
}

fn selected_view_owner<'a>(
    live: &'a LiveSpecArtifactV2,
    view_id: &str,
) -> Option<(&'a crate::PortableEntity, &'a crate::PortableView)> {
    live.payload.entities.iter().find_map(|entity| {
        entity
            .views
            .iter()
            .find(|view| view.id == view_id)
            .map(|view| (entity, view))
    })
}

fn validate_client_names<'a>(
    live_specs: impl IntoIterator<Item = (&'a str, &'a LiveSpecArtifactV2)>,
    selected_views: &[SelectedViewV2],
) -> Result<(), ArtifactError> {
    let lives = live_specs.into_iter().collect::<Vec<_>>();
    reject_normalized_collisions("LiveSpec alias", lives.iter().map(|(alias, _)| *alias))?;

    for (alias, live) in lives {
        reject_normalized_collisions(
            &format!("entity in LiveSpec alias '{alias}'"),
            live.payload
                .entities
                .iter()
                .map(|entity| entity.state_name.as_str()),
        )?;

        let selected = selected_views
            .iter()
            .filter(|selected| selected.live_alias == alias)
            .collect::<Vec<_>>();
        for entity in &live.payload.entities {
            let prefix = format!("{}/", entity.state_name);
            let members = selected
                .iter()
                .filter_map(|selected| {
                    selected_view_owner(live, &selected.view_id)
                        .filter(|(owner, _)| owner.state_name == entity.state_name)
                        .map(|_| selected.view_id.as_str())
                })
                .map(|view_id| {
                    let member = view_id.strip_prefix(&prefix).unwrap_or_default();
                    if member.is_empty() || member.contains('/') {
                        Err(ArtifactError::InvalidArtifact(format!(
                            "selected view '{alias}:{view_id}' cannot be represented as a client member"
                        )))
                    } else {
                        Ok(member)
                    }
                })
                .collect::<Result<Vec<_>, _>>()?;
            reject_normalized_collisions(
                &format!("view member for '{}:{}'", alias, entity.state_name),
                members,
            )?;
        }
    }
    Ok(())
}

fn reject_normalized_collisions<'a>(
    kind: &str,
    names: impl IntoIterator<Item = &'a str>,
) -> Result<(), ArtifactError> {
    let mut normalized = BTreeMap::<String, &str>::new();
    for name in names {
        let key = normalized_client_name(name);
        if key.is_empty() {
            return Err(ArtifactError::InvalidArtifact(format!(
                "{kind} '{name}' has no language-safe identifier"
            )));
        }
        if let Some(existing) = normalized.insert(key.clone(), name) {
            return Err(ArtifactError::InvalidArtifact(format!(
                "{kind} names '{existing}' and '{name}' collide after language-safe normalization as '{key}'"
            )));
        }
    }
    Ok(())
}
