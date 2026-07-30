use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

use crate::HashError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CanonicalizationProfile {
    RawBytesV1,
    AreteJcsV1,
    FramedTupleV1,
    ArtifactTreeV1,
}

impl CanonicalizationProfile {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RawBytesV1 => "raw-bytes-v1",
            Self::AreteJcsV1 => "arete-jcs-v1",
            Self::FramedTupleV1 => "framed-tuple-v1",
            Self::ArtifactTreeV1 => "artifact-tree-v1",
        }
    }
}

impl fmt::Display for CanonicalizationProfile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for CanonicalizationProfile {
    type Err = HashError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "raw-bytes-v1" => Ok(Self::RawBytesV1),
            "arete-jcs-v1" => Ok(Self::AreteJcsV1),
            "framed-tuple-v1" => Ok(Self::FramedTupleV1),
            "artifact-tree-v1" => Ok(Self::ArtifactTreeV1),
            _ => Err(HashError::InvalidHashId("unknown canonicalization profile")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HashKindName {
    IdlSource,
    IdlContent,
    IdlPortable,
    IdlNormalized,
    ProgramSpec,
    AstPortable,
    RuntimeArtifact,
    ArtifactFile,
    DecoderContent,
    SdkDefinition,
    SdkExtension,
    SdkOutputTree,
    Compiler,
    ProgramRelease,
    LiveSpec,
    StackManifest,
    DeploymentRelease,
    DecoderFixtureSet,
}

impl HashKindName {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::IdlSource => "idl-source",
            Self::IdlContent => "idl-content",
            Self::IdlPortable => "idl-portable",
            Self::IdlNormalized => "idl-normalized",
            Self::ProgramSpec => "program-spec",
            Self::AstPortable => "ast-portable",
            Self::RuntimeArtifact => "runtime-artifact",
            Self::ArtifactFile => "artifact-file",
            Self::DecoderContent => "decoder-content",
            Self::SdkDefinition => "sdk-definition",
            Self::SdkExtension => "sdk-extension",
            Self::SdkOutputTree => "sdk-output-tree",
            Self::Compiler => "compiler",
            Self::ProgramRelease => "program-release",
            Self::LiveSpec => "live-spec",
            Self::StackManifest => "stack-manifest",
            Self::DeploymentRelease => "deployment-release",
            Self::DecoderFixtureSet => "decoder-fixture-set",
        }
    }
}

impl fmt::Display for HashKindName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for HashKindName {
    type Err = HashError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "idl-source" => Ok(Self::IdlSource),
            "idl-content" => Ok(Self::IdlContent),
            "idl-portable" => Ok(Self::IdlPortable),
            "idl-normalized" => Ok(Self::IdlNormalized),
            "program-spec" => Ok(Self::ProgramSpec),
            "ast-portable" => Ok(Self::AstPortable),
            "runtime-artifact" => Ok(Self::RuntimeArtifact),
            "artifact-file" => Ok(Self::ArtifactFile),
            "decoder-content" => Ok(Self::DecoderContent),
            "sdk-definition" => Ok(Self::SdkDefinition),
            "sdk-extension" => Ok(Self::SdkExtension),
            "sdk-output-tree" => Ok(Self::SdkOutputTree),
            "compiler" => Ok(Self::Compiler),
            "program-release" => Ok(Self::ProgramRelease),
            "live-spec" => Ok(Self::LiveSpec),
            "stack-manifest" => Ok(Self::StackManifest),
            "deployment-release" => Ok(Self::DeploymentRelease),
            "decoder-fixture-set" => Ok(Self::DecoderFixtureSet),
            _ => Err(HashError::UnknownKind(value.to_string())),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Visibility {
    Public,
    AuthenticatedOwner,
    InternalOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum IdentityClass {
    ExactSource,
    CanonicalContent,
    PortableContent,
    NormalizedContent,
    Composite,
    ArtifactTree,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityMetadata {
    pub kind: HashKindName,
    pub profile: CanonicalizationProfile,
    pub visibility: Visibility,
    pub identity_class: IdentityClass,
    pub api_field: &'static str,
    pub rust_type: &'static str,
    pub typescript_type: &'static str,
    pub projection: &'static str,
    pub allowed_dto_audiences: &'static [Visibility],
    pub database_mappings: &'static [&'static str],
    pub legacy_aliases: &'static [&'static str],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NonHashIdentityMetadata {
    pub api_field: &'static str,
    pub rust_type: &'static str,
    pub typescript_type: &'static str,
    pub projection: &'static str,
    pub visibility: Visibility,
    pub allowed_dto_audiences: &'static [Visibility],
    pub database_mappings: &'static [&'static str],
    pub legacy_aliases: &'static [&'static str],
}

const PUBLIC_DTO_AUDIENCES: &[Visibility] = &[
    Visibility::Public,
    Visibility::AuthenticatedOwner,
    Visibility::InternalOnly,
];
const OWNER_DTO_AUDIENCES: &[Visibility] =
    &[Visibility::AuthenticatedOwner, Visibility::InternalOnly];
const INTERNAL_DTO_AUDIENCES: &[Visibility] = &[Visibility::InternalOnly];

const fn allowed_dto_audiences(visibility: Visibility) -> &'static [Visibility] {
    match visibility {
        Visibility::Public => PUBLIC_DTO_AUDIENCES,
        Visibility::AuthenticatedOwner => OWNER_DTO_AUDIENCES,
        Visibility::InternalOnly => INTERNAL_DTO_AUDIENCES,
    }
}

const fn api_field(kind: HashKindName) -> &'static str {
    match kind {
        HashKindName::IdlSource => "sourceIdlHash",
        HashKindName::IdlContent => "idlContentHash",
        HashKindName::IdlPortable => "portableIdlHash",
        HashKindName::IdlNormalized => "normalizedIdlHash",
        HashKindName::ProgramSpec => "programSpecHash",
        HashKindName::AstPortable => "portableAstHash",
        HashKindName::RuntimeArtifact => "runtimeArtifactHash",
        HashKindName::ArtifactFile => "artifactFileHash",
        HashKindName::DecoderContent => "decoderContentHash",
        HashKindName::SdkDefinition => "sdkDefinitionHash",
        HashKindName::SdkExtension => "sdkExtensionHash",
        HashKindName::SdkOutputTree => "sdkOutputTreeHash",
        HashKindName::Compiler => "compilerHash",
        HashKindName::ProgramRelease => "programReleaseHash",
        HashKindName::LiveSpec => "liveSpecHash",
        HashKindName::StackManifest => "stackManifestHash",
        HashKindName::DeploymentRelease => "deploymentReleaseHash",
        HashKindName::DecoderFixtureSet => "decoderFixtureSetHash",
    }
}

const fn rust_type(kind: HashKindName) -> &'static str {
    match kind {
        HashKindName::IdlSource => "HashId<IdlSource>",
        HashKindName::IdlContent => "HashId<IdlContent>",
        HashKindName::IdlPortable => "HashId<IdlPortable>",
        HashKindName::IdlNormalized => "HashId<IdlNormalized>",
        HashKindName::ProgramSpec => "HashId<ProgramSpec>",
        HashKindName::AstPortable => "HashId<AstPortable>",
        HashKindName::RuntimeArtifact => "HashId<RuntimeArtifact>",
        HashKindName::ArtifactFile => "HashId<ArtifactFile>",
        HashKindName::DecoderContent => "HashId<DecoderContent>",
        HashKindName::SdkDefinition => "HashId<SdkDefinition>",
        HashKindName::SdkExtension => "HashId<SdkExtension>",
        HashKindName::SdkOutputTree => "HashId<SdkOutputTree>",
        HashKindName::Compiler => "HashId<Compiler>",
        HashKindName::ProgramRelease => "HashId<ProgramRelease>",
        HashKindName::LiveSpec => "HashId<LiveSpec>",
        HashKindName::StackManifest => "HashId<StackManifest>",
        HashKindName::DeploymentRelease => "HashId<DeploymentRelease>",
        HashKindName::DecoderFixtureSet => "HashId<DecoderFixtureSet>",
    }
}

const fn typescript_type(kind: HashKindName) -> &'static str {
    match kind {
        HashKindName::IdlSource => "IdlSourceHash",
        HashKindName::IdlContent => "IdlContentHash",
        HashKindName::IdlPortable => "IdlPortableHash",
        HashKindName::IdlNormalized => "IdlNormalizedHash",
        HashKindName::ProgramSpec => "ProgramSpecHash",
        HashKindName::AstPortable => "AstPortableHash",
        HashKindName::RuntimeArtifact => "RuntimeArtifactHash",
        HashKindName::ArtifactFile => "ArtifactFileHash",
        HashKindName::DecoderContent => "DecoderContentHash",
        HashKindName::SdkDefinition => "SdkDefinitionHash",
        HashKindName::SdkExtension => "SdkExtensionHash",
        HashKindName::SdkOutputTree => "SdkOutputTreeHash",
        HashKindName::Compiler => "CompilerHash",
        HashKindName::ProgramRelease => "ProgramReleaseHash",
        HashKindName::LiveSpec => "LiveSpecHash",
        HashKindName::StackManifest => "StackManifestHash",
        HashKindName::DeploymentRelease => "DeploymentReleaseHash",
        HashKindName::DecoderFixtureSet => "DecoderFixtureSetHash",
    }
}

const fn projection(kind: HashKindName) -> &'static str {
    match kind {
        HashKindName::IdlSource => "arete.idl-source/exact-bytes-v1",
        HashKindName::IdlContent => "arete.idl-content/source-json-v1",
        HashKindName::IdlPortable => "arete.idl-portable/source-json-v1",
        HashKindName::IdlNormalized => "arete.idl-normalized/v1",
        HashKindName::ProgramSpec => "arete.program-spec/v1",
        HashKindName::AstPortable => "arete.ast-portable/self-hash-v1",
        HashKindName::RuntimeArtifact => "arete.runtime-artifact/v1",
        HashKindName::ArtifactFile => "arete.artifact-file/exact-bytes-v1",
        HashKindName::DecoderContent => "arete.decoder-content/exact-bytes-v1",
        HashKindName::SdkDefinition => "arete.sdk-definition/v1",
        HashKindName::SdkExtension => "arete.sdk-extension/v1",
        HashKindName::SdkOutputTree => "arete.sdk-output-tree/artifact-tree-v1",
        HashKindName::Compiler => "arete.compiler/v1",
        HashKindName::ProgramRelease => "arete.program-release/v1",
        HashKindName::LiveSpec => "arete.artifact-envelope/live-spec-v1",
        HashKindName::StackManifest => "arete.artifact-envelope/stack-manifest-v1",
        HashKindName::DeploymentRelease => "arete.deployment-release/v1",
        HashKindName::DecoderFixtureSet => "arete.decoder-fixtures/v1",
    }
}

const fn database_mappings(kind: HashKindName) -> &'static [&'static str] {
    match kind {
        HashKindName::IdlSource | HashKindName::ArtifactFile => &[],
        HashKindName::IdlContent => &[
            "idl_contents.idl_content_hash",
            "program_releases.idl_content_hash",
        ],
        HashKindName::IdlPortable => &["idl_contents.idl_portable_hash"],
        HashKindName::IdlNormalized => &[
            "idl_contents.idl_normalized_hash",
            "decoder_bindings.normalized_idl_hash",
            "program_releases.normalized_idl_hash",
            "decoder_fixture_sets.normalized_idl_hash",
        ],
        HashKindName::ProgramSpec => &[
            "idl_contents.program_spec_hash",
            "program_spec_artifacts.program_spec_hash",
            "program_releases.program_spec_hash",
        ],
        HashKindName::AstPortable => &[
            "ast_contents.ast_portable_hash",
            "builds.ast_portable_hash",
            "deployments.current_ast_portable_hash",
        ],
        HashKindName::RuntimeArtifact => &[
            "runtime_artifacts.runtime_artifact_hash",
            "builds.runtime_artifact_hash",
        ],
        HashKindName::DecoderContent => &[
            "decoder_contents.content_hash",
            "decoder_executions.decoder_content_hash",
        ],
        HashKindName::SdkDefinition | HashKindName::Compiler => &[],
        HashKindName::SdkExtension => &["sdk_extension_contents.sdk_extension_hash"],
        HashKindName::SdkOutputTree => &["sdk_extension_contents.sdk_output_tree_hash"],
        HashKindName::ProgramRelease => &["program_releases.release_hash"],
        HashKindName::LiveSpec => &["live_spec_artifacts.live_spec_hash"],
        HashKindName::StackManifest => &["stack_manifest_artifacts.stack_manifest_hash"],
        HashKindName::DeploymentRelease => &[
            "deployment_releases.deployment_release_hash",
            "builds.deployment_release_hash",
            "deployments.deployment_release_hash",
        ],
        HashKindName::DecoderFixtureSet => &["decoder_fixture_sets.fixture_set_hash"],
    }
}

const fn legacy_aliases(kind: HashKindName) -> &'static [&'static str] {
    match kind {
        HashKindName::IdlContent => &["legacy_idl_json_sha256"],
        HashKindName::IdlPortable => &["legacy_idl_json_no_program_sha256"],
        HashKindName::IdlNormalized => &["legacy_normalized_idl_sha256"],
        HashKindName::AstPortable => &["legacy_portable_ast_sha256"],
        HashKindName::RuntimeArtifact => &["legacy_platform_ast_sha256"],
        HashKindName::DecoderContent => &["legacy_decoder_content_sha256"],
        HashKindName::SdkExtension => &["legacy_sdk_extension_sha256"],
        _ => &[],
    }
}

mod sealed {
    pub trait Sealed {}
}

pub trait Kind: sealed::Sealed + 'static {
    const NAME: HashKindName;
    const PROFILE: CanonicalizationProfile;
    const VISIBILITY: Visibility;
    const IDENTITY_CLASS: IdentityClass;
}

macro_rules! define_kinds {
    ($(($type:ident, $name:ident, $profile:ident, $visibility:ident, $class:ident)),+ $(,)?) => {
        $(
            #[derive(Debug)]
            pub struct $type;

            impl sealed::Sealed for $type {}

            impl Kind for $type {
                const NAME: HashKindName = HashKindName::$name;
                const PROFILE: CanonicalizationProfile = CanonicalizationProfile::$profile;
                const VISIBILITY: Visibility = Visibility::$visibility;
                const IDENTITY_CLASS: IdentityClass = IdentityClass::$class;
            }
        )+

        pub const IDENTITY_REGISTRY: &[IdentityMetadata] = &[
            $(IdentityMetadata {
                kind: HashKindName::$name,
                profile: CanonicalizationProfile::$profile,
                visibility: Visibility::$visibility,
                identity_class: IdentityClass::$class,
                api_field: api_field(HashKindName::$name),
                rust_type: rust_type(HashKindName::$name),
                typescript_type: typescript_type(HashKindName::$name),
                projection: projection(HashKindName::$name),
                allowed_dto_audiences: allowed_dto_audiences(Visibility::$visibility),
                database_mappings: database_mappings(HashKindName::$name),
                legacy_aliases: legacy_aliases(HashKindName::$name),
            }),+
        ];
    };
}

define_kinds!(
    (IdlSource, IdlSource, RawBytesV1, Public, ExactSource),
    (IdlContent, IdlContent, AreteJcsV1, Public, CanonicalContent),
    (
        IdlPortable,
        IdlPortable,
        AreteJcsV1,
        Public,
        PortableContent
    ),
    (
        IdlNormalized,
        IdlNormalized,
        AreteJcsV1,
        Public,
        NormalizedContent
    ),
    (ProgramSpec, ProgramSpec, AreteJcsV1, Public, Composite),
    (
        AstPortable,
        AstPortable,
        AreteJcsV1,
        Public,
        PortableContent
    ),
    (
        RuntimeArtifact,
        RuntimeArtifact,
        AreteJcsV1,
        InternalOnly,
        Composite
    ),
    (
        ArtifactFile,
        ArtifactFile,
        RawBytesV1,
        Public,
        CanonicalContent
    ),
    (
        DecoderContent,
        DecoderContent,
        RawBytesV1,
        InternalOnly,
        CanonicalContent
    ),
    (SdkDefinition, SdkDefinition, AreteJcsV1, Public, Composite),
    (SdkExtension, SdkExtension, AreteJcsV1, Public, Composite),
    (
        SdkOutputTree,
        SdkOutputTree,
        ArtifactTreeV1,
        Public,
        ArtifactTree
    ),
    (Compiler, Compiler, FramedTupleV1, Public, Composite),
    (
        ProgramRelease,
        ProgramRelease,
        AreteJcsV1,
        Public,
        Composite
    ),
    (LiveSpec, LiveSpec, AreteJcsV1, Public, Composite),
    (StackManifest, StackManifest, AreteJcsV1, Public, Composite),
    (
        DeploymentRelease,
        DeploymentRelease,
        AreteJcsV1,
        AuthenticatedOwner,
        Composite
    ),
    (
        DecoderFixtureSet,
        DecoderFixtureSet,
        AreteJcsV1,
        InternalOnly,
        Composite
    ),
);

pub const NON_HASH_IDENTITY_REGISTRY: &[NonHashIdentityMetadata] = &[
    NonHashIdentityMetadata {
        api_field: "programReadBindingId",
        rust_type: "ProgramReadBindingId",
        typescript_type: "ProgramReadBindingId",
        projection: "arete.program-read-binding/v1",
        visibility: Visibility::Public,
        allowed_dto_audiences: PUBLIC_DTO_AUDIENCES,
        database_mappings: &[
            "program_read_bindings.id",
            "program_read_routes.program_read_binding_id",
            "program_read_usage_events.program_read_binding_id",
        ],
        legacy_aliases: &[],
    },
    NonHashIdentityMetadata {
        api_field: "decoderBindingId",
        rust_type: "internal::DecoderBindingId",
        typescript_type: "DecoderBindingId",
        projection: "arete.decoder-binding/v1",
        visibility: Visibility::InternalOnly,
        allowed_dto_audiences: INTERNAL_DTO_AUDIENCES,
        database_mappings: &["decoder_bindings.id", "program_releases.decoder_binding_id"],
        legacy_aliases: &[],
    },
    NonHashIdentityMetadata {
        api_field: "decoderEngineId",
        rust_type: "internal::DecoderEngineId",
        typescript_type: "DecoderEngineId",
        projection: "arete.decoder-engine/v1",
        visibility: Visibility::InternalOnly,
        allowed_dto_audiences: INTERNAL_DTO_AUDIENCES,
        database_mappings: &[
            "decoder_executions.decoder_engine_id",
            "program_releases.decoder_engine_id",
            "decoder_fixture_sets.decoder_engine_id",
        ],
        legacy_aliases: &[],
    },
];

pub fn identity_metadata(kind: HashKindName) -> &'static IdentityMetadata {
    IDENTITY_REGISTRY
        .iter()
        .find(|metadata| metadata.kind == kind)
        .expect("closed hash kind registry is exhaustive")
}
