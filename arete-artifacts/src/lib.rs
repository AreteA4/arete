//! Versioned public artifacts shared by Arete generators, clients, and the
//! hosted control plane.
//!
//! Composite stack ASTs are accepted only through [`decompose_legacy_stack`]
//! and [`normalize_legacy_stack_v2`]. New authoring should construct typed V2
//! artifacts with [`author_stack_v2`] instead.

mod authoring;
mod live;
mod manifest;

pub use authoring::*;
pub use live::*;
pub use manifest::*;

use std::collections::{BTreeMap, BTreeSet};

use arete_hash::{
    canonicalize_jcs, hash_jcs, hash_raw_bytes, parse_json_bytes_strict, ArtifactFile, HashError,
    HashId, LiveSpec, ProgramSpec, ProgramSpecV1, StackManifest,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Map, Value};
use thiserror::Error;

pub const ARTIFACT_VERSION_V1: &str = "1.0.0";
pub const PROGRAM_SPEC_KIND: &str = "program-spec";
pub const LIVE_SPEC_KIND: &str = "live-spec";
pub const STACK_MANIFEST_KIND: &str = "stack-manifest";
pub const LIVE_SPEC_SCHEMA_V1: &str = "arete.live-spec/v1";
pub const STACK_MANIFEST_SCHEMA_V1: &str = "arete.stack-manifest/v1";
pub const LIVE_SPEC_SCHEMA_V2: &str = "arete.live-spec/v2";
pub const STACK_MANIFEST_SCHEMA_V2: &str = "arete.stack-manifest/v2";
pub const LEGACY_NORMALIZER_CONTRACT_V1: &str = "arete.legacy-stack-normalizer/v1";
pub const LIVE_COMPILER_CONTRACT_V1: &str = "arete-live-compiler/v1";
pub const LIVE_WIRE_CONTRACT_V1: &str = "arete-live-wire/v1";

pub const CURRENT_AST_VERSION: &str = "0.0.5";
pub const COMPATIBLE_AST_VERSIONS: &[&str] = &["0.0.1", "0.0.2", "0.0.3", "0.0.4"];

#[derive(Debug, Error)]
pub enum ArtifactError {
    #[error(transparent)]
    Hash(#[from] HashError),
    #[error("invalid artifact JSON: {0}")]
    InvalidJson(String),
    #[error("unsupported {artifact} version '{version}'")]
    UnsupportedVersion {
        artifact: &'static str,
        version: String,
    },
    #[error("artifact kind must be '{expected}', not '{actual}'")]
    WrongKind {
        expected: &'static str,
        actual: String,
    },
    #[error("artifact hash does not match its payload")]
    HashMismatch,
    #[error("invalid legacy stack: {0}")]
    InvalidLegacyStack(String),
    #[error("invalid artifact: {0}")]
    InvalidArtifact(String),
    #[error("public artifact contains private field '{0}'")]
    PrivateField(String),
    #[error("artifact I/O failed: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProgramSpecArtifact {
    pub artifact_version: String,
    pub kind: String,
    pub artifact_hash: HashId<ProgramSpec>,
    pub payload: ProgramSpecV1,
}

impl ProgramSpecArtifact {
    pub fn new(payload: ProgramSpecV1) -> Result<Self, ArtifactError> {
        let artifact_hash = payload.hash()?;
        Ok(Self {
            artifact_version: ARTIFACT_VERSION_V1.to_string(),
            kind: PROGRAM_SPEC_KIND.to_string(),
            artifact_hash,
            payload,
        })
    }

    pub fn validate(&self) -> Result<(), ArtifactError> {
        validate_envelope_version(&self.artifact_version, PROGRAM_SPEC_KIND)?;
        validate_kind(&self.kind, PROGRAM_SPEC_KIND)?;
        reject_private_fields(&serde_json::to_value(self).map_err(json_error)?)?;
        if self.payload.hash()? != self.artifact_hash {
            return Err(ArtifactError::HashMismatch);
        }
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, ArtifactError> {
        self.validate()?;
        canonicalize_jcs(self).map_err(Into::into)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProgramRequirementV1 {
    pub program_id: String,
    pub program_spec_hash: HashId<ProgramSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LegacyProgramExtensionsV1 {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub pdas: BTreeMap<String, Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub instructions: Vec<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LiveSpecV1 {
    pub schema: String,
    pub compiler_contract_version: String,
    pub wire_contract_version: String,
    pub programs: Vec<ProgramRequirementV1>,
    pub entities: Vec<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy_program_extensions: Option<LegacyProgramExtensionsV1>,
}

impl LiveSpecV1 {
    pub fn validate(&self) -> Result<(), ArtifactError> {
        if self.schema != LIVE_SPEC_SCHEMA_V1 {
            return Err(ArtifactError::UnsupportedVersion {
                artifact: LIVE_SPEC_KIND,
                version: self.schema.clone(),
            });
        }
        if self.compiler_contract_version.is_empty() || self.wire_contract_version.is_empty() {
            return Err(ArtifactError::InvalidLegacyStack(
                "live compiler and wire contract versions must not be empty".to_string(),
            ));
        }
        let mut program_hashes = BTreeSet::new();
        for program in &self.programs {
            if program.program_id.is_empty()
                || !program_hashes.insert(program.program_spec_hash.to_string())
            {
                return Err(ArtifactError::InvalidLegacyStack(
                    "live program requirements must have unique hashes and non-empty program IDs"
                        .to_string(),
                ));
            }
        }
        reject_private_fields(&serde_json::to_value(self).map_err(json_error)?)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LiveSpecArtifact {
    pub artifact_version: String,
    pub kind: String,
    pub artifact_hash: HashId<LiveSpec>,
    pub payload: LiveSpecV1,
}

impl LiveSpecArtifact {
    pub fn new(payload: LiveSpecV1) -> Result<Self, ArtifactError> {
        payload.validate()?;
        let artifact_hash = hash_live_projection(ARTIFACT_VERSION_V1, &payload)?;
        Ok(Self {
            artifact_version: ARTIFACT_VERSION_V1.to_string(),
            kind: LIVE_SPEC_KIND.to_string(),
            artifact_hash,
            payload,
        })
    }

    pub fn validate(&self) -> Result<(), ArtifactError> {
        validate_envelope_version(&self.artifact_version, LIVE_SPEC_KIND)?;
        validate_kind(&self.kind, LIVE_SPEC_KIND)?;
        self.payload.validate()?;
        reject_private_fields(&serde_json::to_value(self).map_err(json_error)?)?;
        let expected = hash_live_projection(&self.artifact_version, &self.payload)?;
        if expected != self.artifact_hash {
            return Err(ArtifactError::HashMismatch);
        }
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, ArtifactError> {
        self.validate()?;
        canonicalize_jcs(self).map_err(Into::into)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProgramSpecReferenceV1 {
    pub program_id: String,
    pub artifact_hash: HashId<ProgramSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LiveSpecReferenceV1 {
    pub artifact_hash: HashId<LiveSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SelectedViewV1 {
    pub live_spec_hash: HashId<LiveSpec>,
    pub view_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StackManifestV1 {
    pub schema: String,
    pub name: String,
    pub programs: Vec<ProgramSpecReferenceV1>,
    pub live_specs: Vec<LiveSpecReferenceV1>,
    pub selected_views: Vec<SelectedViewV1>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub queries: Vec<Value>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extensions: BTreeMap<String, Value>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, Value>,
}

impl StackManifestV1 {
    pub fn validate(&self) -> Result<(), ArtifactError> {
        if self.schema != STACK_MANIFEST_SCHEMA_V1 {
            return Err(ArtifactError::UnsupportedVersion {
                artifact: STACK_MANIFEST_KIND,
                version: self.schema.clone(),
            });
        }
        if self.name.is_empty() {
            return Err(ArtifactError::InvalidLegacyStack(
                "stack manifest name must not be empty".to_string(),
            ));
        }
        reject_private_fields(&serde_json::to_value(self).map_err(json_error)?)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StackManifestArtifact {
    pub artifact_version: String,
    pub kind: String,
    pub artifact_hash: HashId<StackManifest>,
    pub payload: StackManifestV1,
}

impl StackManifestArtifact {
    pub fn new(payload: StackManifestV1) -> Result<Self, ArtifactError> {
        payload.validate()?;
        let artifact_hash = hash_manifest_projection(ARTIFACT_VERSION_V1, &payload)?;
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
        reject_private_fields(&serde_json::to_value(self).map_err(json_error)?)?;
        let expected = hash_manifest_projection(&self.artifact_version, &self.payload)?;
        if expected != self.artifact_hash {
            return Err(ArtifactError::HashMismatch);
        }
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, ArtifactError> {
        self.validate()?;
        canonicalize_jcs(self).map_err(Into::into)
    }
}

#[derive(Debug, Clone)]
pub struct LoadedArtifact<A> {
    pub artifact: A,
    pub original_bytes: Vec<u8>,
    pub source_hash: HashId<ArtifactFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacySourceProvenance {
    pub source_hash: HashId<ArtifactFile>,
    pub declared_ast_version: Option<String>,
    pub normalized_ast_version: String,
    pub legacy_content_hash: Option<String>,
    pub normalizer_contract_version: String,
}

#[derive(Debug, Clone)]
pub struct LegacyDecomposition {
    pub source: LegacySourceProvenance,
    pub program_specs: Vec<ProgramSpecArtifact>,
    pub live_spec: LiveSpecArtifact,
    pub stack_manifest: StackManifestArtifact,
}

pub fn load_program_spec(
    bytes: &[u8],
) -> Result<LoadedArtifact<ProgramSpecArtifact>, ArtifactError> {
    load_artifact(bytes, ProgramSpecArtifact::validate)
}

pub fn load_live_spec(bytes: &[u8]) -> Result<LoadedArtifact<LiveSpecArtifact>, ArtifactError> {
    load_artifact(bytes, LiveSpecArtifact::validate)
}

pub fn load_stack_manifest(
    bytes: &[u8],
) -> Result<LoadedArtifact<StackManifestArtifact>, ArtifactError> {
    load_artifact(bytes, StackManifestArtifact::validate)
}

pub fn load_legacy_stack_value(bytes: &[u8]) -> Result<LoadedArtifact<Value>, ArtifactError> {
    let mut value = parse_json_bytes_strict(bytes)?;
    reject_private_fields(&value)?;
    let object = value.as_object_mut().ok_or_else(|| {
        ArtifactError::InvalidLegacyStack("top-level value must be an object".to_string())
    })?;
    let declared_ast_version = object
        .get("ast_version")
        .and_then(Value::as_str)
        .unwrap_or("0.0.1");
    if declared_ast_version != CURRENT_AST_VERSION
        && !COMPATIBLE_AST_VERSIONS.contains(&declared_ast_version)
    {
        return Err(ArtifactError::UnsupportedVersion {
            artifact: "legacy-stack",
            version: declared_ast_version.to_string(),
        });
    }
    object.insert(
        "ast_version".to_string(),
        Value::String(CURRENT_AST_VERSION.to_string()),
    );
    Ok(LoadedArtifact {
        artifact: value,
        original_bytes: bytes.to_vec(),
        source_hash: hash_raw_bytes::<ArtifactFile>(bytes)?,
    })
}

/// Deterministically decompose the supported composite stack AST into public
/// artifacts. Historical ASTs that predate embedded `ProgramSpecV1` values must
/// first be enriched from retained IDL provenance; this adapter never fabricates
/// modern IDL hashes from a lossy snapshot.
pub fn decompose_legacy_stack(bytes: &[u8]) -> Result<LegacyDecomposition, ArtifactError> {
    let loaded = load_legacy_stack_value(bytes)?;
    let original: Value = parse_json_bytes_strict(bytes)?;
    let declared_ast_version = original
        .get("ast_version")
        .and_then(Value::as_str)
        .map(str::to_string);
    let legacy_content_hash = original
        .get("content_hash")
        .and_then(Value::as_str)
        .map(str::to_string);
    let stack = loaded.artifact.as_object().ok_or_else(|| {
        ArtifactError::InvalidLegacyStack("top-level value must be an object".to_string())
    })?;
    let stack_name = required_string(stack, "stack_name")?;
    let program_ids = string_array(stack.get("program_ids"), "program_ids")?;
    let program_specs_value = stack
        .get("program_specs")
        .cloned()
        .unwrap_or_else(|| Value::Array(Vec::new()));
    let program_spec_payloads: Vec<ProgramSpecV1> =
        serde_json::from_value(program_specs_value).map_err(json_error)?;
    if program_ids.len() != program_spec_payloads.len() {
        return Err(ArtifactError::InvalidLegacyStack(format!(
            "program_ids has {} entries but program_specs has {}; historical inputs require exact ProgramSpec resolution from retained IDLs",
            program_ids.len(),
            program_spec_payloads.len()
        )));
    }

    let mut program_specs = Vec::with_capacity(program_spec_payloads.len());
    for (program_id, payload) in program_ids.iter().zip(program_spec_payloads) {
        if payload.program_id != *program_id {
            return Err(ArtifactError::InvalidLegacyStack(format!(
                "ProgramSpec program ID '{}' does not match ordered program ID '{}'",
                payload.program_id, program_id
            )));
        }
        program_specs.push(ProgramSpecArtifact::new(payload)?);
    }

    let mut entities = stack
        .get("entities")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| {
            ArtifactError::InvalidLegacyStack("entities must be an array".to_string())
        })?;
    let mut selected_view_ids = Vec::new();
    for entity in &mut entities {
        normalize_entity(entity, &mut selected_view_ids)?;
    }

    let programs = program_specs
        .iter()
        .map(|artifact| ProgramRequirementV1 {
            program_id: artifact.payload.program_id.clone(),
            program_spec_hash: artifact.artifact_hash,
        })
        .collect();
    let pdas = value_map(stack.get("pdas"), "pdas")?;
    let instructions = stack
        .get("instructions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let legacy_program_extensions = if pdas.is_empty() && instructions.is_empty() {
        None
    } else {
        Some(LegacyProgramExtensionsV1 { pdas, instructions })
    };
    let live_spec = LiveSpecArtifact::new(LiveSpecV1 {
        schema: LIVE_SPEC_SCHEMA_V1.to_string(),
        compiler_contract_version: LIVE_COMPILER_CONTRACT_V1.to_string(),
        wire_contract_version: LIVE_WIRE_CONTRACT_V1.to_string(),
        programs,
        entities,
        legacy_program_extensions,
    })?;

    let stack_manifest = StackManifestArtifact::new(StackManifestV1 {
        schema: STACK_MANIFEST_SCHEMA_V1.to_string(),
        name: stack_name,
        programs: program_specs
            .iter()
            .map(|artifact| ProgramSpecReferenceV1 {
                program_id: artifact.payload.program_id.clone(),
                artifact_hash: artifact.artifact_hash,
            })
            .collect(),
        live_specs: vec![LiveSpecReferenceV1 {
            artifact_hash: live_spec.artifact_hash,
        }],
        selected_views: selected_view_ids
            .into_iter()
            .map(|view_id| SelectedViewV1 {
                live_spec_hash: live_spec.artifact_hash,
                view_id,
            })
            .collect(),
        queries: Vec::new(),
        extensions: BTreeMap::new(),
        metadata: BTreeMap::new(),
    })?;

    Ok(LegacyDecomposition {
        source: LegacySourceProvenance {
            source_hash: loaded.source_hash,
            declared_ast_version,
            normalized_ast_version: CURRENT_AST_VERSION.to_string(),
            legacy_content_hash,
            normalizer_contract_version: LEGACY_NORMALIZER_CONTRACT_V1.to_string(),
        },
        program_specs,
        live_spec,
        stack_manifest,
    })
}

fn load_artifact<A: DeserializeOwned>(
    bytes: &[u8],
    validate: impl FnOnce(&A) -> Result<(), ArtifactError>,
) -> Result<LoadedArtifact<A>, ArtifactError> {
    let value = parse_json_bytes_strict(bytes)?;
    let artifact = serde_json::from_value(value).map_err(json_error)?;
    validate(&artifact)?;
    Ok(LoadedArtifact {
        artifact,
        original_bytes: bytes.to_vec(),
        source_hash: hash_raw_bytes::<ArtifactFile>(bytes)?,
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ArtifactProjection<'a, P> {
    artifact_version: &'a str,
    kind: &'static str,
    payload: &'a P,
}

fn hash_live_projection(
    artifact_version: &str,
    payload: &LiveSpecV1,
) -> Result<HashId<LiveSpec>, ArtifactError> {
    hash_jcs(&ArtifactProjection {
        artifact_version,
        kind: LIVE_SPEC_KIND,
        payload,
    })
    .map_err(Into::into)
}

fn hash_manifest_projection(
    artifact_version: &str,
    payload: &StackManifestV1,
) -> Result<HashId<StackManifest>, ArtifactError> {
    hash_jcs(&ArtifactProjection {
        artifact_version,
        kind: STACK_MANIFEST_KIND,
        payload,
    })
    .map_err(Into::into)
}

pub(crate) fn validate_envelope_version(
    version: &str,
    artifact: &'static str,
) -> Result<(), ArtifactError> {
    let components = version.split('.').collect::<Vec<_>>();
    let valid = components.len() == 3
        && components.iter().all(|component| {
            !component.is_empty() && component.chars().all(|c| c.is_ascii_digit())
        });
    if !valid || components[0] != "1" {
        return Err(ArtifactError::UnsupportedVersion {
            artifact,
            version: version.to_string(),
        });
    }
    Ok(())
}

pub(crate) fn validate_kind(actual: &str, expected: &'static str) -> Result<(), ArtifactError> {
    if actual != expected {
        return Err(ArtifactError::WrongKind {
            expected,
            actual: actual.to_string(),
        });
    }
    Ok(())
}

fn normalize_entity(
    entity: &mut Value,
    selected_view_ids: &mut Vec<String>,
) -> Result<(), ArtifactError> {
    let object = entity.as_object_mut().ok_or_else(|| {
        ArtifactError::InvalidLegacyStack("every entity must be an object".to_string())
    })?;
    object.remove("ast_version");
    object.remove("idl");
    object.remove("content_hash");
    let state_name = required_string(object, "state_name")?;
    let primary_keys = object
        .get("identity")
        .and_then(Value::as_object)
        .and_then(|identity| identity.get("primary_keys"))
        .and_then(Value::as_array)
        .ok_or_else(|| {
            ArtifactError::InvalidLegacyStack(format!(
                "entity '{state_name}' must declare identity.primary_keys"
            ))
        })?;
    let primary_key = primary_keys
        .first()
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| {
            ArtifactError::InvalidLegacyStack(format!(
                "entity '{state_name}' must declare a string primary key"
            ))
        })?;
    let views = object
        .entry("views".to_string())
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or_else(|| {
            ArtifactError::InvalidLegacyStack(format!(
                "entity '{state_name}' views must be an array"
            ))
        })?;
    let default_views = [
        json!({
            "id": format!("{state_name}/state"),
            "source": { "Entity": { "name": state_name } },
            "pipeline": [],
            "output": { "Keyed": { "key_field": {
                "segments": primary_key.split('.').collect::<Vec<_>>(),
                "offsets": null
            } } }
        }),
        json!({
            "id": format!("{state_name}/list"),
            "source": { "Entity": { "name": state_name } },
            "pipeline": [],
            "output": "Collection"
        }),
    ];
    for expected in default_views {
        let expected_id = expected["id"].as_str().expect("view ID");
        match views
            .iter()
            .find(|view| view.get("id").and_then(Value::as_str) == Some(expected_id))
        {
            Some(existing) if existing != &expected => {
                return Err(ArtifactError::InvalidLegacyStack(format!(
                    "entity '{state_name}' defines conflicting default view '{expected_id}'"
                )));
            }
            Some(_) => {}
            None => views.push(expected),
        }
    }
    for view in views {
        let view_id = view.get("id").and_then(Value::as_str).ok_or_else(|| {
            ArtifactError::InvalidLegacyStack(format!(
                "entity '{state_name}' contains a view without an ID"
            ))
        })?;
        selected_view_ids.push(view_id.to_string());
    }
    Ok(())
}

fn required_string(object: &Map<String, Value>, field: &str) -> Result<String, ArtifactError> {
    object
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            ArtifactError::InvalidLegacyStack(format!("{field} must be a non-empty string"))
        })
}

fn string_array(value: Option<&Value>, field: &str) -> Result<Vec<String>, ArtifactError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    value
        .as_array()
        .ok_or_else(|| ArtifactError::InvalidLegacyStack(format!("{field} must be an array")))?
        .iter()
        .map(|entry| {
            entry.as_str().map(str::to_string).ok_or_else(|| {
                ArtifactError::InvalidLegacyStack(format!("{field} must contain only strings"))
            })
        })
        .collect()
}

fn value_map(value: Option<&Value>, field: &str) -> Result<BTreeMap<String, Value>, ArtifactError> {
    let Some(value) = value else {
        return Ok(BTreeMap::new());
    };
    let object = value
        .as_object()
        .ok_or_else(|| ArtifactError::InvalidLegacyStack(format!("{field} must be an object")))?;
    Ok(object
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect())
}

pub(crate) fn reject_private_fields(value: &Value) -> Result<(), ArtifactError> {
    const FORBIDDEN: &[&str] = &[
        "platform_parser",
        "platformParser",
        "platform_decoder_bundle",
        "platformDecoderBundle",
        "augmented_specs",
        "augmentedSpecs",
        "decoder_binding_id",
        "decoderBindingId",
        "decoder_content_hash",
        "decoderContentHash",
        "artifact_ref",
        "artifactRef",
    ];
    match value {
        Value::Object(object) => {
            for (key, nested) in object {
                if FORBIDDEN.contains(&key.as_str()) {
                    return Err(ArtifactError::PrivateField(key.clone()));
                }
                reject_private_fields(nested)?;
            }
        }
        Value::Array(values) => {
            for nested in values {
                reject_private_fields(nested)?;
            }
        }
        _ => {}
    }
    Ok(())
}

pub(crate) fn json_error(error: serde_json::Error) -> ArtifactError {
    ArtifactError::InvalidJson(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use arete_hash::{CanonicalIdlDocument, HashId};

    fn program_spec() -> ProgramSpecV1 {
        let idl = br#"{
          "address":"11111111111111111111111111111111",
          "metadata":{"name":"system","version":"1.0.0","spec":"0.1.0"},
          "instructions":[],"accounts":[],"types":[],"events":[],"errors":[]
        }"#;
        let document = CanonicalIdlDocument::parse(idl, None).expect("canonical IDL");
        ProgramSpecV1::from_document(&document)
    }

    fn legacy_stack() -> Vec<u8> {
        serde_json::to_vec(&json!({
            "ast_version": "0.0.5",
            "stack_name": "SystemStack",
            "program_ids": ["11111111111111111111111111111111"],
            "idls": [],
            "program_specs": [program_spec()],
            "entities": [{
                "ast_version": "0.0.5",
                "state_name": "SystemState",
                "program_id": "11111111111111111111111111111111",
                "idl": null,
                "identity": {"primary_keys": ["id.address"], "lookup_indexes": []},
                "handlers": [],
                "sections": [],
                "field_mappings": {},
                "resolver_hooks": [],
                "instruction_hooks": [],
                "resolver_specs": [],
                "computed_fields": [],
                "computed_field_specs": [],
                "content_hash": "legacy-entity-hash",
                "views": []
            }],
            "pdas": {},
            "instructions": [],
            "content_hash": "legacy-stack-hash"
        }))
        .expect("legacy JSON")
    }

    #[test]
    fn program_spec_envelope_preserves_frozen_program_hash() {
        let payload = program_spec();
        let expected = payload.hash().expect("ProgramSpec hash");
        let artifact = ProgramSpecArtifact::new(payload).expect("artifact");
        assert_eq!(artifact.artifact_hash, expected);
        artifact.validate().expect("valid artifact");
    }

    #[test]
    fn decomposition_is_deterministic_and_adds_explicit_default_views() {
        let bytes = legacy_stack();
        let first = decompose_legacy_stack(&bytes).expect("first decomposition");
        let second = decompose_legacy_stack(&bytes).expect("second decomposition");
        assert_eq!(
            first.live_spec.artifact_hash,
            second.live_spec.artifact_hash
        );
        assert_eq!(
            first.stack_manifest.artifact_hash,
            second.stack_manifest.artifact_hash
        );
        assert_eq!(
            first.program_specs[0].artifact_hash,
            program_spec().hash().unwrap()
        );
        assert_eq!(first.stack_manifest.payload.selected_views.len(), 2);
        assert!(first.live_spec.payload.entities[0].get("idl").is_none());
        assert!(first.live_spec.payload.entities[0]
            .get("content_hash")
            .is_none());
    }

    #[test]
    fn loaders_reject_unknown_major_wrong_hash_and_private_fields() {
        let decomposition = decompose_legacy_stack(&legacy_stack()).expect("decomposition");
        let mut value = serde_json::to_value(&decomposition.live_spec).unwrap();
        value["artifactVersion"] = Value::String("2.0.0".to_string());
        assert!(matches!(
            load_live_spec(&serde_json::to_vec(&value).unwrap()),
            Err(ArtifactError::UnsupportedVersion { .. })
        ));

        value["artifactVersion"] = Value::String("1.0.0".to_string());
        value["artifactHash"] = Value::String(HashId::<LiveSpec>::from_digest([7; 32]).to_string());
        assert!(matches!(
            load_live_spec(&serde_json::to_vec(&value).unwrap()),
            Err(ArtifactError::HashMismatch)
        ));

        let mut private_stack: Value = serde_json::from_slice(&legacy_stack()).unwrap();
        private_stack["entities"][0]["decoderBindingId"] = Value::String("private".into());
        assert!(matches!(
            decompose_legacy_stack(&serde_json::to_vec(&private_stack).unwrap()),
            Err(ArtifactError::PrivateField(_))
        ));
    }

    #[test]
    fn exact_input_bytes_are_preserved_for_audit() {
        let bytes = legacy_stack();
        let loaded = load_legacy_stack_value(&bytes).expect("legacy source");
        assert_eq!(loaded.original_bytes, bytes);
        assert_eq!(
            loaded.source_hash,
            hash_raw_bytes::<ArtifactFile>(&loaded.original_bytes).unwrap()
        );
    }
}
