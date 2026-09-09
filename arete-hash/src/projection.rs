use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::identifier::DecoderBindingId;
use crate::{
    hash_framed_tuple, hash_jcs, parse_json_bytes_strict, Compiler, HashError, HashId,
    ProgramRelease, ProgramSpec, SdkDefinition, SdkOutputTree, TupleField,
};

pub const COMPILER_SCHEMA_V1: &str = "arete.compiler/v1";
pub const SDK_DEFINITION_SCHEMA_V1: &str = "arete.sdk-definition/v1";
pub const SDK_DEFINITION_PROGRAM_SPEC_INPUT_KIND: &str = "program-spec";
pub const OSS_DECODER_ENGINE_ID: &str = "arete-oss-generated-decoder/v1";
pub const PROGRAM_RELEASE_SCHEMA_V1: &str = "arete.program-release/v1";
pub const PROGRAM_RELEASE_SCHEMA_V2: &str = "arete.program-release/v2";
pub const PROGRAM_RELEASE_SCHEMA_V3: &str = "arete.program-release/v3";
pub const HOSTED_MANAGED_RELEASE_PROFILE: &str = "hosted-managed";
pub const HOSTED_PRIVATE_RELEASE_PROFILE: &str = "hosted-private";
pub const HOSTED_PRIVATE_EXECUTABLE_POLICY: &str = "observed";
pub const OSS_GENERATED_RELEASE_PROFILE: &str = "oss-generated";
pub const SOLANA_EXECUTABLE_IDENTITY_SCHEMA_V1: &str = "arete.solana-executable-identity/v1";
pub const SOLANA_BPF_LOADER_V2_PROGRAM_ID: &str = "BPFLoader2111111111111111111111111111111111";
pub const SOLANA_BPF_UPGRADEABLE_LOADER_PROGRAM_ID: &str =
    "BPFLoaderUpgradeab1e11111111111111111111111";
pub const SOLANA_EXECUTABLE_PAYLOAD_SHA256_PREFIX: &str = "sha256:";

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

/// Identity of generated SDK content, independent of the compiler's source identity.
/// V1 remains frozen and readable. The output tree projection is specified by the
/// named runtime contract and excludes the identity itself and build provenance.
pub const SDK_DEFINITION_SCHEMA_V2: &str = "arete.sdk-definition/v2";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SdkDefinitionV2 {
    pub schema: String,
    pub input_kind: String,
    pub input_hash: HashId<ProgramSpec>,
    pub target: String,
    pub runtime_contract: String,
    pub output_tree_hash: HashId<SdkOutputTree>,
}

impl SdkDefinitionV2 {
    pub fn new(
        input_hash: HashId<ProgramSpec>,
        target: impl Into<String>,
        runtime_contract: impl Into<String>,
        output_tree_hash: HashId<SdkOutputTree>,
    ) -> Self {
        Self {
            schema: SDK_DEFINITION_SCHEMA_V2.to_string(),
            input_kind: SDK_DEFINITION_PROGRAM_SPEC_INPUT_KIND.to_string(),
            input_hash,
            target: target.into(),
            runtime_contract: runtime_contract.into(),
            output_tree_hash,
        }
    }

    pub fn hash(&self) -> Result<HashId<SdkDefinition>, HashError> {
        if self.schema != SDK_DEFINITION_SCHEMA_V2 {
            return Err(HashError::UnknownVersion(self.schema.clone()));
        }
        if self.input_kind != SDK_DEFINITION_PROGRAM_SPEC_INPUT_KIND
            || !matches!(self.target.as_str(), "typescript" | "rust" | "python")
            || self.runtime_contract.is_empty()
            || self.runtime_contract.len() > 128
            || !self
                .runtime_contract
                .bytes()
                .all(|byte| byte.is_ascii_graphic())
        {
            return Err(HashError::InvalidProjection {
                projection: "SDK definition",
                reason: "expected program-spec input, a supported target and a printable ASCII runtime contract (1..128 bytes)".to_string(),
            });
        }
        hash_jcs(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SolanaExecutableIdentityV1 {
    pub schema: String,
    pub genesis_hash: String,
    pub loader: SolanaExecutableLoaderV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum SolanaExecutableLoaderV1 {
    BpfLoaderV2(SolanaBpfLoaderV2IdentityV1),
    BpfUpgradeableLoader(SolanaBpfUpgradeableLoaderIdentityV1),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SolanaBpfLoaderV2IdentityV1 {
    pub loader_program_id: String,
    pub executable_payload_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SolanaBpfUpgradeableLoaderIdentityV1 {
    pub loader_program_id: String,
    pub program_data_address: String,
    pub deployment_slot: String,
    pub upgrade_authority: SolanaUpgradeAuthorityV1,
    pub executable_payload_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum SolanaUpgradeAuthorityV1 {
    None(SolanaNoUpgradeAuthorityV1),
    Address(SolanaUpgradeAuthorityAddressV1),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SolanaNoUpgradeAuthorityV1 {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SolanaUpgradeAuthorityAddressV1 {
    pub address: String,
}

impl SolanaExecutableIdentityV1 {
    pub fn new(
        genesis_hash: impl Into<String>,
        loader: SolanaExecutableLoaderV1,
    ) -> Result<Self, HashError> {
        let identity = Self {
            schema: SOLANA_EXECUTABLE_IDENTITY_SCHEMA_V1.to_string(),
            genesis_hash: genesis_hash.into(),
            loader,
        };
        validate_solana_executable_identity_v1(&identity)?;
        Ok(identity)
    }
}

impl SolanaExecutableLoaderV1 {
    pub fn bpf_loader_v2(executable_payload_sha256: impl Into<String>) -> Result<Self, HashError> {
        let loader = Self::BpfLoaderV2(SolanaBpfLoaderV2IdentityV1 {
            loader_program_id: SOLANA_BPF_LOADER_V2_PROGRAM_ID.to_string(),
            executable_payload_sha256: executable_payload_sha256.into(),
        });
        validate_solana_executable_loader_v1(&loader)?;
        Ok(loader)
    }

    pub fn bpf_upgradeable_loader(
        program_data_address: impl Into<String>,
        deployment_slot: u64,
        upgrade_authority: SolanaUpgradeAuthorityV1,
        executable_payload_sha256: impl Into<String>,
    ) -> Result<Self, HashError> {
        let loader = Self::BpfUpgradeableLoader(SolanaBpfUpgradeableLoaderIdentityV1 {
            loader_program_id: SOLANA_BPF_UPGRADEABLE_LOADER_PROGRAM_ID.to_string(),
            program_data_address: program_data_address.into(),
            deployment_slot: deployment_slot.to_string(),
            upgrade_authority,
            executable_payload_sha256: executable_payload_sha256.into(),
        });
        validate_solana_executable_loader_v1(&loader)?;
        Ok(loader)
    }
}

impl SolanaUpgradeAuthorityV1 {
    pub const fn none() -> Self {
        Self::None(SolanaNoUpgradeAuthorityV1 {})
    }

    pub fn address(address: impl Into<String>) -> Result<Self, HashError> {
        let address = address.into();
        validate_base58_32(&address, "upgradeAuthority.address")?;
        Ok(Self::Address(SolanaUpgradeAuthorityAddressV1 { address }))
    }
}

pub fn parse_solana_executable_identity_v1(
    bytes: &[u8],
) -> Result<SolanaExecutableIdentityV1, HashError> {
    let value = parse_json_bytes_strict(bytes)?;
    let identity: SolanaExecutableIdentityV1 = serde_json::from_value(value)
        .map_err(|error| release_projection_error(error.to_string()))?;
    validate_solana_executable_identity_v1(&identity)?;
    Ok(identity)
}

pub fn validate_solana_executable_identity_v1(
    identity: &SolanaExecutableIdentityV1,
) -> Result<(), HashError> {
    if identity.schema != SOLANA_EXECUTABLE_IDENTITY_SCHEMA_V1 {
        return Err(HashError::UnknownVersion(identity.schema.clone()));
    }
    validate_base58_32(&identity.genesis_hash, "genesisHash")?;
    validate_solana_executable_loader_v1(&identity.loader)
}

fn validate_solana_executable_loader_v1(
    loader: &SolanaExecutableLoaderV1,
) -> Result<(), HashError> {
    match loader {
        SolanaExecutableLoaderV1::BpfLoaderV2(loader) => {
            validate_loader_program_id(
                &loader.loader_program_id,
                SOLANA_BPF_LOADER_V2_PROGRAM_ID,
                "bpf-loader-v2",
            )?;
            validate_sha256_digest(&loader.executable_payload_sha256, "executablePayloadSha256")
        }
        SolanaExecutableLoaderV1::BpfUpgradeableLoader(loader) => {
            validate_loader_program_id(
                &loader.loader_program_id,
                SOLANA_BPF_UPGRADEABLE_LOADER_PROGRAM_ID,
                "bpf-upgradeable-loader",
            )?;
            validate_base58_32(&loader.program_data_address, "programDataAddress")?;
            validate_deployment_slot(&loader.deployment_slot)?;
            if let SolanaUpgradeAuthorityV1::Address(authority) = &loader.upgrade_authority {
                validate_base58_32(&authority.address, "upgradeAuthority.address")?;
            }
            validate_sha256_digest(&loader.executable_payload_sha256, "executablePayloadSha256")
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostedManagedProgramReleaseV2 {
    pub schema: String,
    pub release_profile: String,
    pub program_id: String,
    pub program_spec_hash: HashId<ProgramSpec>,
    pub idl_content_hash: HashId<crate::IdlContent>,
    pub normalized_idl_hash: HashId<crate::IdlNormalized>,
    pub decoder_abi_version: String,
    pub decoder_engine_id: String,
    pub decoder_binding_id: String,
    pub executable_identity: SolanaExecutableIdentityV1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostedManagedProgramReleaseV2Fields {
    pub program_id: String,
    pub program_spec_hash: HashId<ProgramSpec>,
    pub idl_content_hash: HashId<crate::IdlContent>,
    pub normalized_idl_hash: HashId<crate::IdlNormalized>,
    pub decoder_abi_version: String,
    pub decoder_engine_id: String,
    pub decoder_binding_id: String,
    pub executable_identity: SolanaExecutableIdentityV1,
}

impl HostedManagedProgramReleaseV2 {
    pub fn new(fields: HostedManagedProgramReleaseV2Fields) -> Result<Self, HashError> {
        let release = Self {
            schema: PROGRAM_RELEASE_SCHEMA_V2.to_string(),
            release_profile: HOSTED_MANAGED_RELEASE_PROFILE.to_string(),
            program_id: fields.program_id,
            program_spec_hash: fields.program_spec_hash,
            idl_content_hash: fields.idl_content_hash,
            normalized_idl_hash: fields.normalized_idl_hash,
            decoder_abi_version: fields.decoder_abi_version,
            decoder_engine_id: fields.decoder_engine_id,
            decoder_binding_id: fields.decoder_binding_id,
            executable_identity: fields.executable_identity,
        };
        validate_hosted_managed_program_release_v2(&release)?;
        Ok(release)
    }

    pub fn hash(&self) -> Result<HashId<ProgramRelease>, HashError> {
        validate_hosted_managed_program_release_v2(self)?;
        hash_jcs(self)
    }
}

pub fn parse_hosted_managed_program_release_v2(
    bytes: &[u8],
) -> Result<HostedManagedProgramReleaseV2, HashError> {
    let value = parse_json_bytes_strict(bytes)?;
    let release: HostedManagedProgramReleaseV2 = serde_json::from_value(value)
        .map_err(|error| release_projection_error(error.to_string()))?;
    validate_hosted_managed_program_release_v2(&release)?;
    Ok(release)
}

pub fn validate_hosted_managed_program_release_v2(
    release: &HostedManagedProgramReleaseV2,
) -> Result<(), HashError> {
    validate_release_projection(
        (&release.schema, PROGRAM_RELEASE_SCHEMA_V2),
        (&release.release_profile, HOSTED_MANAGED_RELEASE_PROFILE),
        &release.program_id,
        &release.decoder_engine_id,
        Some(&release.decoder_abi_version),
        Some(&release.decoder_binding_id),
    )?;
    validate_base58_32(&release.program_id, "programId")?;
    validate_release_identifier(&release.decoder_abi_version, "decoderAbiVersion", 64)?;
    validate_release_identifier(&release.decoder_engine_id, "decoderEngineId", 128)?;
    validate_release_identifier(&release.decoder_binding_id, "decoderBindingId", 128)?;
    validate_solana_executable_identity_v1(&release.executable_identity)
}

/// Immutable hosted-private release identity.
///
/// Ownership, admission, alias, visibility, and observations deliberately do
/// not participate in this projection. The same exact decoder artifact may be
/// granted to multiple owners without changing its content identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostedPrivateProgramReleaseV3 {
    pub schema: String,
    pub release_profile: String,
    pub program_id: String,
    pub program_spec_hash: HashId<ProgramSpec>,
    pub idl_content_hash: HashId<crate::IdlContent>,
    pub normalized_idl_hash: HashId<crate::IdlNormalized>,
    pub decoder_abi_version: String,
    pub decoder_engine_id: String,
    pub decoder_binding_id: String,
    pub executable_policy: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostedPrivateProgramReleaseV3Fields {
    pub program_id: String,
    pub program_spec_hash: HashId<ProgramSpec>,
    pub idl_content_hash: HashId<crate::IdlContent>,
    pub normalized_idl_hash: HashId<crate::IdlNormalized>,
    pub decoder_abi_version: String,
    pub decoder_engine_id: String,
    pub decoder_binding_id: String,
}

impl HostedPrivateProgramReleaseV3 {
    pub fn new(fields: HostedPrivateProgramReleaseV3Fields) -> Result<Self, HashError> {
        let release = Self {
            schema: PROGRAM_RELEASE_SCHEMA_V3.to_string(),
            release_profile: HOSTED_PRIVATE_RELEASE_PROFILE.to_string(),
            program_id: fields.program_id,
            program_spec_hash: fields.program_spec_hash,
            idl_content_hash: fields.idl_content_hash,
            normalized_idl_hash: fields.normalized_idl_hash,
            decoder_abi_version: fields.decoder_abi_version,
            decoder_engine_id: fields.decoder_engine_id,
            decoder_binding_id: fields.decoder_binding_id,
            executable_policy: HOSTED_PRIVATE_EXECUTABLE_POLICY.to_string(),
        };
        validate_hosted_private_program_release_v3(&release)?;
        Ok(release)
    }

    pub fn hash(&self) -> Result<HashId<ProgramRelease>, HashError> {
        validate_hosted_private_program_release_v3(self)?;
        hash_jcs(self)
    }
}

pub fn parse_hosted_private_program_release_v3(
    bytes: &[u8],
) -> Result<HostedPrivateProgramReleaseV3, HashError> {
    let value = parse_json_bytes_strict(bytes)?;
    let release: HostedPrivateProgramReleaseV3 = serde_json::from_value(value)
        .map_err(|error| release_projection_error(error.to_string()))?;
    validate_hosted_private_program_release_v3(&release)?;
    Ok(release)
}

pub fn validate_hosted_private_program_release_v3(
    release: &HostedPrivateProgramReleaseV3,
) -> Result<(), HashError> {
    validate_release_projection(
        (&release.schema, PROGRAM_RELEASE_SCHEMA_V3),
        (&release.release_profile, HOSTED_PRIVATE_RELEASE_PROFILE),
        &release.program_id,
        &release.decoder_engine_id,
        Some(&release.decoder_abi_version),
        Some(&release.decoder_binding_id),
    )?;
    validate_base58_32(&release.program_id, "programId")?;
    validate_release_identifier(&release.decoder_abi_version, "decoderAbiVersion", 64)?;
    validate_release_identifier(&release.decoder_engine_id, "decoderEngineId", 128)?;
    DecoderBindingId::new(&release.decoder_binding_id)?;
    if release.executable_policy != HOSTED_PRIVATE_EXECUTABLE_POLICY {
        return Err(release_projection_error(format!(
            "executablePolicy must be '{}', not '{}'",
            HOSTED_PRIVATE_EXECUTABLE_POLICY, release.executable_policy
        )));
    }
    Ok(())
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
            (&self.schema, PROGRAM_RELEASE_SCHEMA_V1),
            (&self.release_profile, OSS_GENERATED_RELEASE_PROFILE),
            &self.program_id,
            &self.decoder_engine_id,
            None,
            None,
        )?;
        hash_jcs(self)
    }
}

fn validate_release_projection(
    schema: (&str, &'static str),
    release_profile: (&str, &'static str),
    program_id: &str,
    decoder_engine_id: &str,
    decoder_abi_version: Option<&str>,
    decoder_binding_id: Option<&str>,
) -> Result<(), HashError> {
    let (schema, expected_schema) = schema;
    if schema != expected_schema {
        return Err(HashError::UnknownVersion(schema.to_string()));
    }
    let (release_profile, expected_profile) = release_profile;
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

fn validate_loader_program_id(
    actual: &str,
    expected: &'static str,
    variant: &'static str,
) -> Result<(), HashError> {
    if actual != expected {
        return Err(release_projection_error(format!(
            "loaderProgramId for '{variant}' must be '{expected}', not '{actual}'"
        )));
    }
    Ok(())
}

fn validate_release_identifier(
    value: &str,
    field: &str,
    max_length: usize,
) -> Result<(), HashError> {
    if value.is_empty() || value.trim() != value || value.len() > max_length {
        return Err(release_projection_error(format!(
            "{field} must be a nonempty, trimmed string of at most {max_length} bytes"
        )));
    }
    Ok(())
}

fn validate_deployment_slot(value: &str) -> Result<(), HashError> {
    let canonical = value == "0"
        || value
            .strip_prefix(|character: char| ('1'..='9').contains(&character))
            .is_some_and(|rest| rest.bytes().all(|byte| byte.is_ascii_digit()));
    if !canonical || value.parse::<u64>().is_err() {
        return Err(release_projection_error(
            "deploymentSlot must be a canonical unsigned decimal u64 string".to_string(),
        ));
    }
    Ok(())
}

fn validate_sha256_digest(value: &str, field: &str) -> Result<(), HashError> {
    let Some(digest) = value.strip_prefix(SOLANA_EXECUTABLE_PAYLOAD_SHA256_PREFIX) else {
        return Err(release_projection_error(format!(
            "{field} must use the sha256:<lowercase-hex> format"
        )));
    };
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(release_projection_error(format!(
            "{field} must use the sha256:<lowercase-hex> format"
        )));
    }
    Ok(())
}

fn validate_base58_32(value: &str, field: &str) -> Result<(), HashError> {
    let decoded = bs58::decode(value).into_vec().map_err(|_| {
        release_projection_error(format!("{field} must be a canonical 32-byte base58 value"))
    })?;
    if decoded.len() != 32 || bs58::encode(decoded).into_string() != value {
        return Err(release_projection_error(format!(
            "{field} must be a canonical 32-byte base58 value"
        )));
    }
    Ok(())
}

fn release_projection_error(reason: String) -> HashError {
    HashError::InvalidProjection {
        projection: "program release",
        reason,
    }
}
