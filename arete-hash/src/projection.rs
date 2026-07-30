use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    hash_framed_tuple, hash_jcs, Compiler, HashError, HashId, ProgramRelease, ProgramSpec,
    SdkDefinition, TupleField,
};

pub const COMPILER_SCHEMA_V1: &str = "arete.compiler/v1";
pub const SDK_DEFINITION_SCHEMA_V1: &str = "arete.sdk-definition/v1";
pub const SDK_DEFINITION_PROGRAM_SPEC_INPUT_KIND: &str = "program-spec";
pub const OSS_DECODER_ENGINE_ID: &str = "arete-oss-generated-decoder/v1";
pub const PROGRAM_RELEASE_SCHEMA_V1: &str = "arete.program-release/v1";
pub const HOSTED_MANAGED_RELEASE_PROFILE: &str = "hosted-managed";
pub const OSS_GENERATED_RELEASE_PROFILE: &str = "oss-generated";

/// Remove the declared top-level self-hash field and no other field.
///
/// Nested `artifactHash` fields and all other hash-like fields are retained.
pub fn project_without_artifact_hash(value: &Value) -> Result<Value, HashError> {
    let mut projection = value
        .as_object()
        .cloned()
        .ok_or(HashError::InvalidSelfHashProjection)?;
    projection.remove("artifactHash");
    Ok(Value::Object(projection))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompilerSourceV1 {
    pub path: String,
    pub bytes: Vec<u8>,
}

impl CompilerSourceV1 {
    pub fn new(path: impl Into<String>, bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            path: path.into(),
            bytes: bytes.into(),
        }
    }
}

/// Frozen v1 identity projection for the OSS SDK compiler source tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompilerV1 {
    pub schema: String,
    pub sources: Vec<CompilerSourceV1>,
}

impl CompilerV1 {
    pub fn new(sources: impl IntoIterator<Item = CompilerSourceV1>) -> Result<Self, HashError> {
        let mut projection = Self {
            schema: COMPILER_SCHEMA_V1.to_string(),
            sources: sources.into_iter().collect(),
        };
        projection
            .sources
            .sort_by(|left, right| left.path.as_bytes().cmp(right.path.as_bytes()));
        projection.validate()?;
        Ok(projection)
    }

    pub fn hash(&self) -> Result<HashId<Compiler>, HashError> {
        self.validate()?;
        let mut fields = Vec::with_capacity(self.sources.len() + 1);
        fields.push(TupleField::new("schema", self.schema.as_bytes()));
        fields.extend(
            self.sources
                .iter()
                .map(|source| TupleField::new(&source.path, &source.bytes)),
        );
        hash_framed_tuple(&fields)
    }

    fn validate(&self) -> Result<(), HashError> {
        if self.schema != COMPILER_SCHEMA_V1 {
            return Err(HashError::UnknownVersion(self.schema.clone()));
        }
        if self.sources.is_empty() {
            return Err(HashError::InvalidProjection {
                projection: "compiler",
                reason: "sources must not be empty".to_string(),
            });
        }
        let mut previous: Option<&[u8]> = None;
        for source in &self.sources {
            if source.path.is_empty() || source.path == "schema" {
                return Err(HashError::InvalidProjection {
                    projection: "compiler",
                    reason: format!("invalid source path '{}'", source.path),
                });
            }
            if let Some(previous) = previous {
                match previous.cmp(source.path.as_bytes()) {
                    std::cmp::Ordering::Greater => {
                        return Err(HashError::InvalidProjection {
                            projection: "compiler",
                            reason: "sources must be sorted by raw UTF-8 path bytes".to_string(),
                        })
                    }
                    std::cmp::Ordering::Equal => {
                        return Err(HashError::InvalidProjection {
                            projection: "compiler",
                            reason: format!("duplicate source path '{}'", source.path),
                        })
                    }
                    std::cmp::Ordering::Less => {}
                }
            }
            previous = Some(source.path.as_bytes());
        }
        Ok(())
    }
}

/// Frozen v1 identity projection for one generated program SDK definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SdkDefinitionV1 {
    pub schema: String,
    pub input_kind: String,
    pub input_hash: HashId<ProgramSpec>,
    pub compiler_hash: HashId<Compiler>,
}

impl SdkDefinitionV1 {
    pub fn new(input_hash: HashId<ProgramSpec>, compiler_hash: HashId<Compiler>) -> Self {
        Self {
            schema: SDK_DEFINITION_SCHEMA_V1.to_string(),
            input_kind: SDK_DEFINITION_PROGRAM_SPEC_INPUT_KIND.to_string(),
            input_hash,
            compiler_hash,
        }
    }

    pub fn hash(&self) -> Result<HashId<SdkDefinition>, HashError> {
        if self.schema != SDK_DEFINITION_SCHEMA_V1 {
            return Err(HashError::UnknownVersion(self.schema.clone()));
        }
        if self.input_kind != SDK_DEFINITION_PROGRAM_SPEC_INPUT_KIND {
            return Err(HashError::InvalidProjection {
                projection: "SDK definition",
                reason: format!(
                    "inputKind must be '{}', not '{}'",
                    SDK_DEFINITION_PROGRAM_SPEC_INPUT_KIND, self.input_kind
                ),
            });
        }
        hash_jcs(self)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostedManagedProgramReleaseV1 {
    pub schema: String,
    pub release_profile: String,
    pub program_id: String,
    pub program_spec_hash: HashId<ProgramSpec>,
    pub idl_content_hash: HashId<crate::IdlContent>,
    pub normalized_idl_hash: HashId<crate::IdlNormalized>,
    pub decoder_abi_version: String,
    pub decoder_engine_id: String,
    pub decoder_binding_id: String,
}

impl HostedManagedProgramReleaseV1 {
    pub fn new(
        program_id: impl Into<String>,
        program_spec_hash: HashId<ProgramSpec>,
        idl_content_hash: HashId<crate::IdlContent>,
        normalized_idl_hash: HashId<crate::IdlNormalized>,
        decoder_abi_version: impl Into<String>,
        decoder_engine_id: impl Into<String>,
        decoder_binding_id: impl Into<String>,
    ) -> Self {
        Self {
            schema: PROGRAM_RELEASE_SCHEMA_V1.to_string(),
            release_profile: HOSTED_MANAGED_RELEASE_PROFILE.to_string(),
            program_id: program_id.into(),
            program_spec_hash,
            idl_content_hash,
            normalized_idl_hash,
            decoder_abi_version: decoder_abi_version.into(),
            decoder_engine_id: decoder_engine_id.into(),
            decoder_binding_id: decoder_binding_id.into(),
        }
    }

    pub fn hash(&self) -> Result<HashId<ProgramRelease>, HashError> {
        validate_release_projection(
            &self.schema,
            &self.release_profile,
            HOSTED_MANAGED_RELEASE_PROFILE,
            &self.program_id,
            &self.decoder_engine_id,
            Some(&self.decoder_abi_version),
            Some(&self.decoder_binding_id),
        )?;
        hash_jcs(self)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OssGeneratedProgramReleaseV1 {
    pub schema: String,
    pub release_profile: String,
    pub program_id: String,
    pub program_spec_hash: HashId<ProgramSpec>,
    pub idl_content_hash: HashId<crate::IdlContent>,
    pub normalized_idl_hash: HashId<crate::IdlNormalized>,
    pub decoder_engine_id: String,
}

impl OssGeneratedProgramReleaseV1 {
    pub fn new(
        program_id: impl Into<String>,
        program_spec_hash: HashId<ProgramSpec>,
        idl_content_hash: HashId<crate::IdlContent>,
        normalized_idl_hash: HashId<crate::IdlNormalized>,
    ) -> Self {
        Self::with_decoder_engine(
            program_id,
            program_spec_hash,
            idl_content_hash,
            normalized_idl_hash,
            OSS_DECODER_ENGINE_ID,
        )
    }

    pub fn with_decoder_engine(
        program_id: impl Into<String>,
        program_spec_hash: HashId<ProgramSpec>,
        idl_content_hash: HashId<crate::IdlContent>,
        normalized_idl_hash: HashId<crate::IdlNormalized>,
        decoder_engine_id: impl Into<String>,
    ) -> Self {
        Self {
            schema: PROGRAM_RELEASE_SCHEMA_V1.to_string(),
            release_profile: OSS_GENERATED_RELEASE_PROFILE.to_string(),
            program_id: program_id.into(),
            program_spec_hash,
            idl_content_hash,
            normalized_idl_hash,
            decoder_engine_id: decoder_engine_id.into(),
        }
    }

    pub fn hash(&self) -> Result<HashId<ProgramRelease>, HashError> {
        validate_release_projection(
            &self.schema,
            &self.release_profile,
            OSS_GENERATED_RELEASE_PROFILE,
            &self.program_id,
            &self.decoder_engine_id,
            None,
            None,
        )?;
        hash_jcs(self)
    }
}

fn validate_release_projection(
    schema: &str,
    release_profile: &str,
    expected_profile: &'static str,
    program_id: &str,
    decoder_engine_id: &str,
    decoder_abi_version: Option<&str>,
    decoder_binding_id: Option<&str>,
) -> Result<(), HashError> {
    if schema != PROGRAM_RELEASE_SCHEMA_V1 {
        return Err(HashError::UnknownVersion(schema.to_string()));
    }
    if release_profile != expected_profile {
        return Err(HashError::InvalidProjection {
            projection: "program release",
            reason: format!("releaseProfile must be '{expected_profile}', not '{release_profile}'"),
        });
    }
    if program_id.is_empty() {
        return Err(HashError::InvalidProjection {
            projection: "program release",
            reason: "programId must not be empty".to_string(),
        });
    }
    if decoder_engine_id.is_empty() {
        return Err(HashError::InvalidProjection {
            projection: "program release",
            reason: "decoderEngineId must not be empty".to_string(),
        });
    }
    if decoder_abi_version.is_some_and(str::is_empty) {
        return Err(HashError::InvalidProjection {
            projection: "program release",
            reason: "decoderAbiVersion must not be empty".to_string(),
        });
    }
    if decoder_binding_id.is_some_and(str::is_empty) {
        return Err(HashError::InvalidProjection {
            projection: "program release",
            reason: "decoderBindingId must not be empty".to_string(),
        });
    }
    Ok(())
}
