use serde::{Deserialize, Serialize};

use super::manifest::{DependencyKind, InstallTarget};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryResolveRequest {
    pub manifest_version: u32,
    pub dependencies: Vec<RegistryDependencyRequest>,
    pub targets: Vec<InstallTarget>,
    pub generator_contract: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryDependencyRequest {
    pub kind: DependencyKind,
    pub alias: String,
    pub package: String,
    pub requirement: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locked_package_release_hash: Option<String>,
    /// The locked stack's program SDK releases, checked against the response
    /// when the lock is reused. Local state only; never sent.
    #[serde(skip)]
    pub locked_programs: Vec<crate::project::lockfile::LockedProgram>,
    /// The locked protocol's hoisted stacks, checked the same way. Local
    /// state only; never sent.
    #[serde(skip)]
    pub locked_stacks: Vec<crate::project::lockfile::LockedProtocolStack>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegistryResolveResponse {
    pub resolver_contract: String,
    pub dependencies: Vec<ResolvedRegistryDependency>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "lowercase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ResolvedRegistryDependency {
    Stack {
        alias: String,
        package: String,
        version: String,
        package_release_hash: String,
        generator_contract: String,
        stack_manifest_hash: String,
        stack_manifest: serde_json::Value,
        live_specs: Vec<ResolvedLiveSpec>,
        programs: Vec<crate::api_client::RegistryProgramInstallResponse>,
        sdk_extensions: Vec<ResolvedSdkExtension>,
        /// Present only because the request asked for `include=delivery`; an
        /// absent value means the registry ignored the opt-in.
        #[serde(default)]
        delivery: Option<Box<ResolvedStackDelivery>>,
    },
    Program {
        alias: String,
        package: String,
        version: String,
        package_release_hash: String,
        generator_contract: String,
        install: Box<crate::api_client::RegistryProgramInstallResponse>,
        sdk_extensions: Vec<ResolvedSdkExtension>,
    },
    /// A protocol package: the protocol release, the deployment its client
    /// calls, and its hoisted stacks and programs.
    Protocol {
        alias: String,
        package: String,
        version: String,
        package_release_hash: String,
        generator_contract: String,
        protocol: Box<ResolvedProtocol>,
        /// One stack resolution (`kind: "stack"`) per stack the spec
        /// declares, aliased by the spec's stack key and carrying its
        /// programs exactly as a stack dependency does.
        stacks: Vec<ResolvedRegistryDependency>,
        /// One exact program release per `programSpec` program the spec
        /// declares, with the program package (program SDK) it installs as,
        /// exactly as a stack carries its programs.
        programs: Vec<crate::api_client::RegistryProgramInstallResponse>,
        /// `programSpec` programs the package leaves unpinned: no program
        /// package exists for them (a program installed from a local IDL),
        /// so nothing is generated and the app installs them itself. Only
        /// an owner-private package carries any.
        #[serde(default)]
        unpinned_programs: Vec<ResolvedUnpinnedProgram>,
    },
}

/// One program a protocol package names by its exact ProgramSpec only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedUnpinnedProgram {
    /// The spec's key for the program (`programs.<key>`).
    pub key: String,
    pub program_id: String,
    pub program_spec_hash: String,
}

impl ResolvedRegistryDependency {
    pub fn alias(&self) -> &str {
        match self {
            Self::Stack { alias, .. }
            | Self::Program { alias, .. }
            | Self::Protocol { alias, .. } => alias,
        }
    }

    pub fn package(&self) -> &str {
        match self {
            Self::Stack { package, .. }
            | Self::Program { package, .. }
            | Self::Protocol { package, .. } => package,
        }
    }

    /// The immutable release identity the lockfile pins.
    pub fn package_release_hash(&self) -> &str {
        match self {
            Self::Stack {
                package_release_hash,
                ..
            }
            | Self::Program {
                package_release_hash,
                ..
            }
            | Self::Protocol {
                package_release_hash,
                ..
            } => package_release_hash,
        }
    }

    pub fn kind(&self) -> super::manifest::DependencyKind {
        match self {
            Self::Stack { .. } => super::manifest::DependencyKind::Stack,
            Self::Program { .. } => super::manifest::DependencyKind::Program,
            Self::Protocol { .. } => super::manifest::DependencyKind::Protocol,
        }
    }

    pub fn version(&self) -> &str {
        match self {
            Self::Stack { version, .. }
            | Self::Program { version, .. }
            | Self::Protocol { version, .. } => version,
        }
    }
}

/// The protocol half of a protocol resolution.
///
/// `protocolSpec` is the exact `arete.protocol-spec/v1` document. A registry
/// may omit it; the CLI then reads it from
/// `GET /api/registry/artifacts/protocol-spec/{protocolSpecHash}`. Either way
/// the document must hash to `protocolSpecHash`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedProtocol {
    pub protocol_release_hash: String,
    pub protocol_spec_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol_spec: Option<serde_json::Value>,
    pub deployment: ResolvedProtocolDeployment,
    /// Which hoisted stack is the protocol's own generated stack, and the
    /// view whose row carries an instance's live fields. Absent when the
    /// protocol publishes no stack.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live_stack: Option<ResolvedProtocolLiveStack>,
    /// Built by `a4 dev sdk` for a local deployment, not by the registry:
    /// there are no registry stacks or programs, so none are hoisted and the
    /// client carries the protocol module only. Never deserialized.
    #[serde(skip)]
    pub local_dev: bool,
}

/// The deployment a protocol client calls: the publisher's shared
/// deployment for a `public` protocol, the owner's own for `private`. It
/// names where to call and nothing else: no credential is ever part of a
/// resolution, and the app's own key supplies identity at runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedProtocolDeployment {
    pub deployment_id: String,
    pub access: ProtocolDeploymentAccess,
    /// Base URL of the hosted protocol routes (`…/api`).
    pub api_url: String,
    /// The session endpoint (`POST /ws/sessions`) that mints protocol
    /// sessions for this deployment.
    pub session_endpoint: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProtocolDeploymentAccess {
    Private,
    Public,
}

impl ProtocolDeploymentAccess {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Private => "private",
            Self::Public => "public",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedProtocolLiveStack {
    /// Key of the stack in the protocol's `stacks`.
    pub stack: String,
    /// `<Entity>/<view>` keyed by the instance account below.
    pub view: String,
    /// The instance account (`instance.accounts.<keyAccount>`) that keys
    /// the view.
    pub key_account: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedLiveSpec {
    pub alias: String,
    pub artifact_hash: String,
    pub artifact: serde_json::Value,
}

/// How a resolved stack is delivered. Transport state only: it is
/// re-resolved on every install and never written to `arete.lock`.
#[derive(Debug, Clone, Deserialize)]
#[serde(
    tag = "mode",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ResolvedStackDelivery {
    /// A hosted stream served by one exact deployment release.
    Hosted {
        deployment_release_hash: String,
        /// One binding per resolved LiveSpec, in the same order.
        live_bindings: Vec<ResolvedLiveBinding>,
        chain_binding: Option<Box<crate::api_client::RegistryCapabilityInstallBinding>>,
        transaction_binding: Option<Box<crate::api_client::RegistryCapabilityInstallBinding>>,
        /// Set while this version is being retired: it is served until at
        /// least this time (RFC 3339).
        #[serde(default)]
        served_until: Option<String>,
        /// The version served by default, when this one is being retired.
        #[serde(default)]
        replacement: Option<ServedVersionReplacement>,
        /// The command that installs the replacement.
        #[serde(default)]
        upgrade_command: Option<String>,
    },
    /// The user deploys it; nothing is hosted, so endpoints stay placeholders.
    DefinitionOnly {},
    /// This version is no longer served. The definition still installs, and
    /// its bindings are the stack's own endpoints, so a generated SDK keeps
    /// naming this version and its connection is refused with the
    /// replacement.
    Retired {
        retired_at: String,
        #[serde(default)]
        replacement: Option<ServedVersionReplacement>,
        #[serde(default)]
        upgrade_command: Option<String>,
        /// One binding per resolved LiveSpec, in the same order; empty when
        /// the stack is no longer hosted at all.
        live_bindings: Vec<ResolvedLiveBinding>,
        chain_binding: Option<Box<crate::api_client::RegistryCapabilityInstallBinding>>,
        transaction_binding: Option<Box<crate::api_client::RegistryCapabilityInstallBinding>>,
    },
}

/// The served version that replaces one being retired.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ServedVersionReplacement {
    pub stack_manifest_hash: String,
    /// The package version that installs it, when the registry publishes one.
    #[serde(default)]
    pub version: Option<String>,
}

/// The binding for one resolved LiveSpec, joined to it by alias and hash.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedLiveBinding {
    pub alias: String,
    pub live_spec_hash: String,
    pub binding: crate::api_client::RegistryLiveSpecInstallBinding,
}

/// One SDK extension the resolver returns, for a stack or program package
/// (`sdkExtensions`) or for a program a stack references (its program's
/// `sdkExtensions`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedSdkExtension {
    pub target: String,
    pub content_hash: String,
    pub artifact: crate::api_client::RegistrySdkExtensionArtifact,
    /// The extension API contract the extension declared when it was
    /// published. The registry reports it here, beside the served artifact,
    /// whose manifest never carries it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extension_api: Option<std::num::NonZeroU32>,
}

impl ResolvedSdkExtension {
    /// The artifact to generate from, with the declared extension API in
    /// its manifest, where the SDK runtime compatibility check reads it.
    pub fn artifact_with_contract(
        &self,
    ) -> anyhow::Result<crate::api_client::RegistrySdkExtensionArtifact> {
        let mut artifact = self.artifact.clone();
        match (artifact.manifest.extension_api, self.extension_api) {
            (Some(manifest), Some(declared)) if manifest != declared => anyhow::bail!(
                "Registry returned {} SDK extension {} with extension API {declared}, but its manifest declares {manifest}",
                self.target,
                self.content_hash
            ),
            (None, declared) => artifact.manifest.extension_api = declared,
            _ => {}
        }
        Ok(artifact)
    }
}

/// The target a legacy single program extension was authored for: its
/// manifest `language`, TypeScript when absent.
pub(crate) fn legacy_extension_target(
    artifact: &crate::api_client::RegistrySdkExtensionArtifact,
) -> Option<InstallTarget> {
    match artifact.manifest.language.as_deref() {
        None | Some("typescript") => Some(InstallTarget::TypeScript),
        Some("rust") => Some(InstallTarget::Rust),
        Some("python") => Some(InstallTarget::Python),
        Some(_) => None,
    }
}

/// The SDK extensions a stack program carries, as locked: the per-target
/// `sdkExtensions` filtered to `targets` when the registry sent them,
/// otherwise the legacy single `definition.extensions`.
pub fn program_extension_artifacts<'a>(
    program: &'a crate::api_client::RegistryProgramInstallResponse,
    targets: &[InstallTarget],
) -> Vec<&'a crate::api_client::RegistrySdkExtensionArtifact> {
    match &program.sdk_extensions {
        Some(extensions) => extensions
            .iter()
            .filter(|extension| {
                targets
                    .iter()
                    .any(|target| target.as_str() == extension.target)
            })
            .map(|extension| &extension.artifact)
            .collect(),
        None => program.definition.extensions.iter().collect(),
    }
}

/// A stack program's SDK extension for one generation target: the matching
/// per-target `sdkExtensions` entry when the registry sent them, otherwise
/// the legacy single extension when it was authored for `target`.
pub fn program_extension_for_target(
    program: &crate::api_client::RegistryProgramInstallResponse,
    target: InstallTarget,
) -> anyhow::Result<Option<crate::api_client::RegistrySdkExtensionArtifact>> {
    match &program.sdk_extensions {
        Some(extensions) => {
            let mut selected = extensions
                .iter()
                .filter(|extension| extension.target == target.as_str());
            let first = selected.next();
            if selected.next().is_some() {
                anyhow::bail!(
                    "Registry returned more than one {target} SDK extension for program '{}'",
                    program.install_name
                );
            }
            first
                .map(ResolvedSdkExtension::artifact_with_contract)
                .transpose()
        }
        None => Ok(program
            .definition
            .extensions
            .as_ref()
            .filter(|artifact| legacy_extension_target(artifact) == Some(target))
            .cloned()),
    }
}
