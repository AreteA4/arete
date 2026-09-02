//! Versioned public artifacts shared by Arete generators, clients, and the
//! hosted control plane.
//!
//! Authoring constructs typed V2 artifacts with [`author_stack_v2`].

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
use serde_json::Value;
use thiserror::Error;

pub const ARTIFACT_VERSION_V1: &str = "1.0.0";
pub const PROGRAM_SPEC_KIND: &str = "program-spec";
pub const LIVE_SPEC_KIND: &str = "live-spec";
pub const STACK_MANIFEST_KIND: &str = "stack-manifest";
pub const LIVE_SPEC_SCHEMA_V1: &str = "arete.live-spec/v1";
pub const STACK_MANIFEST_SCHEMA_V1: &str = "arete.stack-manifest/v1";
pub const LIVE_SPEC_SCHEMA_V2: &str = "arete.live-spec/v2";
pub const STACK_MANIFEST_SCHEMA_V2: &str = "arete.stack-manifest/v2";
pub const LIVE_COMPILER_CONTRACT_V1: &str = "arete-live-compiler/v1";
pub const LIVE_WIRE_CONTRACT_V1: &str = "arete-live-wire/v1";

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
            return Err(ArtifactError::InvalidArtifact(
                "live compiler and wire contract versions must not be empty".to_string(),
            ));
        }
        let mut program_hashes = BTreeSet::new();
        for program in &self.programs {
            if program.program_id.is_empty()
                || !program_hashes.insert(program.program_spec_hash.to_string())
            {
                return Err(ArtifactError::InvalidArtifact(
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
            return Err(ArtifactError::InvalidArtifact(
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
    use arete_hash::CanonicalIdlDocument;

    fn program_spec() -> ProgramSpecV1 {
        let idl = br#"{
          "address":"11111111111111111111111111111111",
          "metadata":{"name":"system","version":"1.0.0","spec":"0.1.0"},
          "instructions":[],"accounts":[],"types":[],"events":[],"errors":[]
        }"#;
        let document = CanonicalIdlDocument::parse(idl, None).expect("canonical IDL");
        ProgramSpecV1::from_document(&document)
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
    fn exact_artifact_input_bytes_are_preserved_for_audit() {
        let bytes = ProgramSpecArtifact::new(program_spec())
            .expect("artifact")
            .canonical_bytes()
            .expect("canonical bytes");
        let loaded = load_program_spec(&bytes).expect("program artifact");
        assert_eq!(loaded.original_bytes, bytes);
        assert_eq!(
            loaded.source_hash,
            hash_raw_bytes::<ArtifactFile>(&loaded.original_bytes).unwrap()
        );
    }
}
