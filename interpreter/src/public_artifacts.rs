//! Compatibility bridge between explicit public artifacts and the current
//! interpreter generators. The composite AST is constructed only in memory and
//! remains a legacy input shape, not a published artifact.

use std::collections::{BTreeMap, BTreeSet};

use arete_artifacts::{
    decompose_legacy_stack, resolve_stack_composition_v2, LegacyDecomposition, LiveSpecArtifact,
    LiveSpecArtifactV2, ProgramSpecArtifact, ResolvedLiveSpecV2, StackManifestArtifact,
    StackManifestArtifactV2,
};
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::ast::{InstructionDef, PdaDefinition, SerializableStackSpec, CURRENT_AST_VERSION};

#[derive(Debug, Clone)]
pub struct AliasedStackSpecV2 {
    pub alias: String,
    pub live_spec_hash: String,
    pub stack_spec: SerializableStackSpec,
}

#[derive(Debug, Clone)]
pub struct ComposedStackSpecsV2 {
    pub name: String,
    pub live_specs: Vec<AliasedStackSpecV2>,
}

pub fn decompose_stack_spec(
    stack_spec: &SerializableStackSpec,
) -> Result<LegacyDecomposition, String> {
    let bytes = serde_json::to_vec(stack_spec).map_err(|error| error.to_string())?;
    decompose_legacy_stack(&bytes).map_err(|error| error.to_string())
}

pub fn stack_spec_from_program_artifacts(
    name: impl Into<String>,
    programs: &[ProgramSpecArtifact],
) -> Result<SerializableStackSpec, String> {
    for program in programs {
        program.validate().map_err(|error| error.to_string())?;
    }
    let (program_ids, idls, program_specs, pdas, instructions) = program_inputs(programs)?;
    Ok(SerializableStackSpec {
        ast_version: CURRENT_AST_VERSION.to_string(),
        stack_name: name.into(),
        program_ids,
        idls,
        program_specs,
        entities: Vec::new(),
        pdas,
        instructions,
        content_hash: None,
    })
}

pub fn stack_spec_from_artifacts(
    programs: &[ProgramSpecArtifact],
    live_spec: &LiveSpecArtifact,
    manifest: &StackManifestArtifact,
) -> Result<SerializableStackSpec, String> {
    live_spec.validate().map_err(|error| error.to_string())?;
    manifest.validate().map_err(|error| error.to_string())?;
    let mut stack = stack_spec_from_program_artifacts(&manifest.payload.name, programs)?;

    let program_hashes = programs
        .iter()
        .map(|program| program.artifact_hash)
        .collect::<Vec<_>>();
    let live_program_hashes = live_spec
        .payload
        .programs
        .iter()
        .map(|program| program.program_spec_hash)
        .collect::<Vec<_>>();
    let manifest_program_hashes = manifest
        .payload
        .programs
        .iter()
        .map(|program| program.artifact_hash)
        .collect::<Vec<_>>();
    if program_hashes != live_program_hashes || program_hashes != manifest_program_hashes {
        return Err(
            "ProgramSpec order must match LiveSpec and StackManifest requirements".to_string(),
        );
    }
    if manifest.payload.live_specs.len() != 1
        || manifest.payload.live_specs[0].artifact_hash != live_spec.artifact_hash
    {
        return Err("StackManifest must reference the supplied LiveSpec exactly once".to_string());
    }

    stack.entities = transcode(&live_spec.payload.entities)?;
    if let Some(extensions) = &live_spec.payload.legacy_program_extensions {
        stack.pdas = transcode(&extensions.pdas)?;
        stack.instructions = transcode(&extensions.instructions)?;
    }
    Ok(stack)
}

/// Reconstruct the single-live generator input from typed V2 artifacts.
/// This compatibility wrapper deliberately rejects zero- and multi-live inputs.
pub fn stack_spec_from_artifacts_v2(
    programs: &[ProgramSpecArtifact],
    live_spec: &LiveSpecArtifactV2,
    manifest: &StackManifestArtifactV2,
) -> Result<SerializableStackSpec, String> {
    if manifest.payload.live_specs.len() != 1 {
        return Err("single-live compatibility requires exactly one aliased LiveSpec".to_string());
    }
    let alias = manifest.payload.live_specs[0].alias.clone();
    let lives = vec![(alias, live_spec.clone())];
    let mut composed = stack_specs_from_artifacts_v2(programs, &lives, manifest)?;
    Ok(composed.live_specs.remove(0).stack_spec)
}

/// Build one fresh generator model per manifest alias. ProgramSpec lookup is
/// hash-keyed, adapters are applied only to that live's dependency subset, and
/// selected views are projected before language generation.
pub fn stack_specs_from_artifacts_v2(
    programs: &[ProgramSpecArtifact],
    live_specs: &[(String, LiveSpecArtifactV2)],
    manifest: &StackManifestArtifactV2,
) -> Result<ComposedStackSpecsV2, String> {
    let resolved = resolve_stack_composition_v2(manifest, live_specs, programs)
        .map_err(|error| error.to_string())?;
    let multiple = resolved.live_specs.len() > 1;
    let live_specs = resolved
        .live_specs
        .into_iter()
        .map(|live| {
            let stack_name = if multiple {
                format!(
                    "{}{}",
                    identifier_pascal_case(&manifest.payload.name),
                    identifier_pascal_case(&live.alias)
                )
            } else {
                manifest.payload.name.clone()
            };
            stack_spec_for_live(stack_name, live)
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(ComposedStackSpecsV2 {
        name: manifest.payload.name.clone(),
        live_specs,
    })
}

fn stack_spec_for_live(
    stack_name: String,
    live: ResolvedLiveSpecV2<'_>,
) -> Result<AliasedStackSpecV2, String> {
    let programs = live
        .program_specs
        .iter()
        .map(|program| (*program).clone())
        .collect::<Vec<_>>();
    let mut stack = stack_spec_from_program_artifacts(stack_name, &programs)?;
    stack.entities = transcode(&live.artifact.payload.entities)?;
    let selected = live.selected_views.iter().collect::<BTreeSet<_>>();
    for entity in &mut stack.entities {
        entity.views.retain(|view| selected.contains(&view.id));
    }

    for adapter in &live.artifact.payload.program_adapters {
        let program = programs
            .iter()
            .find(|program| program.artifact_hash == adapter.program_spec_hash)
            .ok_or_else(|| "program adapter references an unknown ProgramSpec".to_string())?;
        let program_name = &program.payload.idl_snapshot.snapshot.name;
        let pdas = stack.pdas.entry(program_name.clone()).or_default();
        for (name, pda) in &adapter.pdas {
            pdas.insert(name.clone(), transcode(pda)?);
        }
        for resolution in &adapter.instruction_resolutions {
            let instruction = stack
                .instructions
                .iter_mut()
                .find(|instruction| {
                    instruction.name == resolution.instruction
                        && instruction.program_id.as_deref()
                            == Some(program.payload.program_id.as_str())
                })
                .ok_or_else(|| {
                    format!(
                        "program adapter references unknown instruction '{}.{}'",
                        program_name, resolution.instruction
                    )
                })?;
            for (account_name, account_resolution) in &resolution.accounts {
                let account = instruction
                    .accounts
                    .iter_mut()
                    .find(|account| account.name == *account_name)
                    .ok_or_else(|| {
                        format!(
                            "program adapter references unknown account '{}.{}.{}'",
                            program_name, resolution.instruction, account_name
                        )
                    })?;
                account.resolution = transcode(account_resolution)?;
            }
        }
    }
    Ok(AliasedStackSpecV2 {
        alias: live.alias,
        live_spec_hash: live.artifact.artifact_hash.to_string(),
        stack_spec: stack,
    })
}

type ProgramInputs = (
    Vec<String>,
    Vec<arete_idl::snapshot::IdlSnapshot>,
    Vec<arete_hash::ProgramSpecV1>,
    BTreeMap<String, BTreeMap<String, PdaDefinition>>,
    Vec<InstructionDef>,
);

fn program_inputs(programs: &[ProgramSpecArtifact]) -> Result<ProgramInputs, String> {
    let mut program_ids = Vec::with_capacity(programs.len());
    let mut idls = Vec::with_capacity(programs.len());
    let mut program_specs = Vec::with_capacity(programs.len());
    let mut pdas = BTreeMap::new();
    let mut instructions = Vec::new();
    for program in programs {
        let payload = &program.payload;
        program_ids.push(payload.program_id.clone());
        idls.push(payload.idl_snapshot.snapshot.clone());
        program_specs.push(payload.clone());
        let program_pdas = transcode(&payload.pdas)?;
        pdas.insert(payload.idl_snapshot.snapshot.name.clone(), program_pdas);
        instructions.extend(transcode::<_, Vec<InstructionDef>>(&payload.instructions)?);
    }
    Ok((program_ids, idls, program_specs, pdas, instructions))
}

fn transcode<T: Serialize, U: DeserializeOwned>(value: &T) -> Result<U, String> {
    serde_json::from_value(serde_json::to_value(value).map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())
}

fn identifier_pascal_case(value: &str) -> String {
    let mut output = value
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            let mut characters = segment.chars();
            characters
                .next()
                .map(|first| first.to_ascii_uppercase().to_string() + characters.as_str())
                .unwrap_or_default()
        })
        .collect::<String>();
    if output
        .chars()
        .next()
        .is_some_and(|character| character.is_ascii_digit())
    {
        output.insert(0, 'A');
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use arete_hash::{CanonicalIdlDocument, ProgramSpecV1};

    fn program() -> ProgramSpecArtifact {
        let idl = br#"{
          "address":"11111111111111111111111111111111",
          "metadata":{"name":"system","version":"1.0.0","spec":"0.1.0"},
          "instructions":[],"accounts":[],"types":[],"events":[],"errors":[]
        }"#;
        let document = CanonicalIdlDocument::parse(idl, None).unwrap();
        ProgramSpecArtifact::new(ProgramSpecV1::from_document(&document)).unwrap()
    }

    #[test]
    fn program_artifacts_reconstruct_generator_input_without_hosted_release_state() {
        let program = program();
        let stack = stack_spec_from_program_artifacts("SystemProgram", &[program.clone()]).unwrap();
        assert_eq!(stack.program_ids, vec![program.payload.program_id]);
        assert_eq!(
            stack.program_specs[0].hash().unwrap(),
            program.artifact_hash
        );
        assert!(stack.entities.is_empty());
    }
}
