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
}

impl ResolvedRegistryDependency {
    pub fn alias(&self) -> &str {
        match self {
            Self::Stack { alias, .. } | Self::Program { alias, .. } => alias,
        }
    }

    pub fn package(&self) -> &str {
        match self {
            Self::Stack { package, .. } | Self::Program { package, .. } => package,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedLiveSpec {
    pub alias: String,
    pub artifact_hash: String,
    pub artifact: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedSdkExtension {
    pub target: String,
    pub content_hash: String,
    pub artifact: crate::api_client::RegistrySdkExtensionArtifact,
}
