use anyhow::{Context, Result};
use cap_std::{ambient_authority, fs::Dir};
use colored::Colorize;
use dialoguer::{theme::ColorfulTheme, Select};
use regex::Regex;
use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::api_client::{
    ApiClient, RegistryCapabilityInstallBinding, RegistryLiveSpecInstallBinding,
    RegistryLiveSpecInstallDescriptor, RegistryProgramInstallResponse,
    RegistryProgramInstallTransport, RegistrySdkExtensionArtifact, RegistrySdkExtensionInputKind,
    RegistryStackInstallResponse,
};
use crate::commands::public_artifacts::{
    load_local_artifact_stack, load_local_artifact_stack_with_roots, LocalArtifactStack,
};
use crate::config::to_kebab_case;
use crate::telemetry;

type AliasedLiveSpecs = Vec<(String, arete_artifacts::LiveSpecArtifactV2)>;

struct RemoteStackAst {
    name: String,
    stack: String,
    manifest_hash: String,
    program_specs: Vec<arete_artifacts::ProgramSpecArtifact>,
    live_specs: AliasedLiveSpecs,
    live_bindings: Vec<RegistryLiveSpecInstallDescriptor>,
    stack_manifest: arete_artifacts::StackManifestArtifactV2,
    chain_binding: Option<RegistryCapabilityInstallBinding>,
    transaction_binding: Option<RegistryCapabilityInstallBinding>,
    exact_views: bool,
    sdk_name: String,
    hosted_extensions: Option<ResolvedExtensionsArtifact>,
    programs: Vec<RegistryProgramInstallResponse>,
    require_managed_gateway: bool,
}

enum ResolvedStackSource {
    LocalArtifacts(Box<LocalArtifactStack>),
    Remote(Box<RemoteStackAst>),
}

#[derive(Clone, Copy)]
struct CompositionArtifacts<'a> {
    program_specs: &'a [arete_artifacts::ProgramSpecArtifact],
    live_specs: &'a [(String, arete_artifacts::LiveSpecArtifactV2)],
    stack_manifest: &'a arete_artifacts::StackManifestArtifactV2,
}

struct ResolvedRegistryComposition {
    stack_manifest: arete_artifacts::StackManifestArtifactV2,
    live_specs: AliasedLiveSpecs,
    live_bindings: Vec<RegistryLiveSpecInstallDescriptor>,
}

#[derive(Clone, Copy)]
enum SdkTarget {
    TypeScript,
    Rust,
    Python,
}

#[derive(Clone, Copy)]
enum OutputExtensionsFallback {
    Reuse,
    Ignore,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExtensionsManifest {
    entry: String,
    files: Vec<String>,
    input_kind: Option<ExtensionsInputKind>,
    input_hash: Option<String>,
    sdk_range: Option<String>,
    /// Target SDK language of the bundle (`"rust"` for Rust bundles,
    /// `"python"` for Python bundles; absent or `"typescript"` for
    /// TypeScript bundles). Skipped when absent so pre-existing TypeScript
    /// manifests round-trip byte-identically.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    language: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum ExtensionsInputKind {
    StackAst,
    StackManifest,
    ProgramIdl,
    ProgramSpec,
}

impl ExtensionsInputKind {
    fn as_manifest_value(self) -> &'static str {
        match self {
            Self::StackAst => "stack-ast",
            Self::StackManifest => "stack-manifest",
            Self::ProgramIdl => "program-idl",
            Self::ProgramSpec => "program-spec",
        }
    }

    fn from_registry(kind: RegistrySdkExtensionInputKind) -> Self {
        match kind {
            RegistrySdkExtensionInputKind::StackAst => Self::StackAst,
            RegistrySdkExtensionInputKind::StackManifest => Self::StackManifest,
            RegistrySdkExtensionInputKind::ProgramIdl => Self::ProgramIdl,
            RegistrySdkExtensionInputKind::ProgramSpec => Self::ProgramSpec,
        }
    }
}

#[derive(Debug, Clone)]
struct ResolvedExtensionsFile {
    path: String,
    contents: String,
}

#[derive(Debug, Clone)]
struct ResolvedExtensionsArtifact {
    entry: String,
    files: Vec<ResolvedExtensionsFile>,
    input_kind: Option<ExtensionsInputKind>,
    input_hash: Option<String>,
    sdk_range: Option<String>,
    language: Option<String>,
    sdk_extension_hash: Option<String>,
    sdk_output_tree_hash: Option<String>,
    program_extension_bindings: Vec<ProgramExtensionBinding>,
}

impl ResolvedExtensionsArtifact {
    fn manifest(&self) -> ExtensionsManifest {
        ExtensionsManifest {
            entry: self.entry.clone(),
            files: self
                .files
                .iter()
                .map(|file| file.path.clone())
                .collect::<Vec<_>>(),
            input_kind: self.input_kind,
            input_hash: self.input_hash.clone(),
            sdk_range: self.sdk_range.clone(),
            language: self.language.clone(),
        }
    }
}

/// Bundle language declared by an extensions manifest.
const EXTENSIONS_LANGUAGE_TYPESCRIPT: &str = "typescript";
const EXTENSIONS_LANGUAGE_RUST: &str = "rust";
const EXTENSIONS_LANGUAGE_PYTHON: &str = "python";

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProgramExtensionBinding {
    export_name: String,
    program_key: String,
}

#[derive(Debug, Clone)]
struct HostedProgramModule {
    program_key: String,
    program_const_name: String,
    import_name: String,
    input_pin: ResolvedExtensionsInputPin,
    extension: Option<ResolvedExtensionsArtifact>,
    program_spec: arete_artifacts::ProgramSpecArtifact,
    program_config: arete_interpreter::typescript::TypeScriptProgramConfig,
}

#[derive(Debug, Clone)]
struct TypeScriptLayout {
    output_dir: PathBuf,
    base_name: String,
    entry_path: PathBuf,
    core_path: PathBuf,
}

const SDK_PROVENANCE_FILE: &str = "sdk-provenance.json";
const SDK_MANIFEST_FILE: &str = "sdk-manifest.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)] // Retained to validate and migrate legacy provenance manifests.
struct SdkProvenanceManifestV1 {
    schema_version: u32,
    input: SdkProvenanceInputV1,
    generator: SdkProvenanceGeneratorV1,
    extensions: Option<SdkProvenanceExtensionsV1>,
    artifacts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[allow(dead_code)]
struct SdkProvenanceInputV1 {
    kind: ExtensionsInputKind,
    sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[allow(dead_code)]
struct SdkProvenanceGeneratorV1 {
    name: String,
    version: String,
    sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[allow(dead_code)]
struct SdkProvenanceExtensionsV1 {
    sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SdkProvenanceManifestV2 {
    schema_version: u32,
    input: SdkProvenanceInputV2,
    generator: SdkProvenanceGeneratorV2,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sdk_output_tree_hash: Option<String>,
    extensions: Option<SdkProvenanceExtensionsV2>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    program_extensions: BTreeMap<String, SdkProvenanceProgramExtensionV2>,
    artifacts: Vec<String>,
}

/// Stable, committed description of generated content. The adjacent provenance
/// document records the compiler used for each generation and owns this manifest.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SdkContentManifestV1<'a> {
    schema_version: u32,
    input: &'a SdkProvenanceInputV2,
    sdk_output_tree_hash: &'a str,
    extensions: &'a Option<SdkProvenanceExtensionsV2>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    program_extensions: &'a BTreeMap<String, SdkProvenanceProgramExtensionV2>,
    artifacts: &'a [String],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct SdkProvenanceInputV2 {
    kind: ExtensionsInputKind,
    hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SdkProvenanceGeneratorV2 {
    name: String,
    version: String,
    compiler_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SdkProvenanceExtensionsV2 {
    #[serde(alias = "legacyProvenanceSha256")]
    content_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sdk_extension_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sdk_output_tree_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SdkProvenanceProgramExtensionV2 {
    input: SdkProvenanceInputV2,
    #[serde(alias = "legacyProvenanceSha256")]
    content_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sdk_extension_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sdk_output_tree_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
enum SdkProvenanceManifest {
    V1(SdkProvenanceManifestV1),
    V2(SdkProvenanceManifestV2),
}

impl<'de> Deserialize<'de> for SdkProvenanceManifest {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        match value
            .get("schemaVersion")
            .and_then(serde_json::Value::as_u64)
        {
            Some(1) => serde_json::from_value(value)
                .map(Self::V1)
                .map_err(serde::de::Error::custom),
            Some(2) => serde_json::from_value(value)
                .map(Self::V2)
                .map_err(serde::de::Error::custom),
            Some(version) => Err(serde::de::Error::custom(format!(
                "unsupported SDK provenance schema version {version}"
            ))),
            None => Err(serde::de::Error::custom(
                "SDK provenance manifest omitted schemaVersion",
            )),
        }
    }
}

#[derive(Debug, Deserialize)]
struct PackageVersionManifest {
    version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedExtensionsInputPin {
    kind: ExtensionsInputKind,
    hash: String,
}

impl ResolvedStackSource {
    fn output_extensions_fallback(&self) -> OutputExtensionsFallback {
        match self {
            Self::Remote(_) => OutputExtensionsFallback::Ignore,
            Self::LocalArtifacts(_) => OutputExtensionsFallback::Reuse,
        }
    }

    fn stack_id(&self) -> &str {
        match self {
            Self::LocalArtifacts(stack) => stack.manifest_hash.as_str(),
            Self::Remote(stack) => stack.stack.as_str(),
        }
    }

    fn sdk_name(&self) -> &str {
        match self {
            Self::LocalArtifacts(stack) => stack.stack_manifest.payload.name.as_str(),
            Self::Remote(stack) => stack.sdk_name.as_str(),
        }
    }

    fn default_websocket_url(&self) -> Option<String> {
        match self {
            Self::LocalArtifacts(_) => None,
            Self::Remote(stack) => (stack.live_bindings.len() == 1)
                .then(|| stack.live_bindings[0].binding.websocket_endpoint.clone()),
        }
    }

    fn default_http_url(&self) -> Option<String> {
        match self {
            Self::LocalArtifacts(_) => None,
            Self::Remote(stack) => (stack.live_bindings.len() == 1)
                .then(|| stack.live_bindings[0].binding.query_endpoint.clone()),
        }
    }

    fn hosted_gateway(&self) -> Result<Option<serde_json::Value>> {
        match self {
            Self::LocalArtifacts(_) => Ok(None),
            Self::Remote(stack) if stack.require_managed_gateway => {
                Ok(Some(managed_gateway_descriptor(
                    stack.chain_binding.as_ref(),
                    stack.transaction_binding.as_ref(),
                    &format!("hosted stack '{}'", stack.stack),
                )?))
            }
            Self::Remote(stack) => optional_gateway_descriptor(
                stack.chain_binding.as_ref(),
                stack.transaction_binding.as_ref(),
                &format!("resolved project stack '{}'", stack.stack),
            ),
        }
    }

    fn rust_program_reads(&self) -> Result<Vec<arete_interpreter::rust::RustProgramReadConfig>> {
        match self {
            Self::LocalArtifacts(_) => Ok(Vec::new()),
            Self::Remote(stack) => stack.programs.iter().map(program_read_override).collect(),
        }
    }

    fn python_program_reads(
        &self,
    ) -> Result<Vec<arete_interpreter::python::PythonProgramReadConfig>> {
        self.rust_program_reads()?
            .into_iter()
            .map(|read| {
                Ok(arete_interpreter::python::PythonProgramReadConfig {
                    program_id: read.program_id,
                    program_spec_hash: read.program_spec_hash,
                    program_release_hash: read.program_release_hash,
                    descriptor: read.descriptor,
                })
            })
            .collect()
    }

    fn print_source_details(&self) {
        match self {
            Self::LocalArtifacts(stack) => {
                println!("  StackManifest: {}", stack.manifest_path.display());
                println!("  StackManifest Hash: {}", stack.manifest_hash);
                if stack.live_specs.is_empty() {
                    println!("  LiveSpecs: none (program-only manifest)");
                } else {
                    for (alias, live) in &stack.live_specs {
                        println!("  LiveSpec {alias}: {}", live.artifact_hash);
                    }
                }
            }
            Self::Remote(stack) => {
                println!("  Hosted Stack: {}", stack.stack.cyan());
                println!("  Stack Name: {}", stack.name);
                println!("  StackManifest Hash: {}", stack.manifest_hash);
                for live in &stack.live_bindings {
                    println!(
                        "  LiveSpec {}: {} ({}, {})",
                        live.alias,
                        live.live_spec_hash,
                        live.binding.websocket_endpoint,
                        live.binding.query_endpoint
                    );
                }
            }
        }
    }

    fn load_stack_spec(
        &self,
        require_entities: bool,
    ) -> Result<arete_interpreter::ast::SerializableStackSpec> {
        match self {
            Self::LocalArtifacts(stack) => {
                let spec = match stack.live_specs.as_slice() {
                    [(alias, live)] => {
                        debug_assert_eq!(alias, &stack.stack_manifest.payload.live_specs[0].alias);
                        arete_interpreter::public_artifacts::stack_spec_from_artifacts_v2(
                            &stack.program_specs,
                            live,
                            &stack.stack_manifest,
                        )
                    }
                    [] => arete_interpreter::public_artifacts::stack_spec_from_program_artifacts(
                        &stack.stack_manifest.payload.name,
                        &stack.program_specs,
                    ),
                    _ if !require_entities => {
                        arete_interpreter::public_artifacts::stack_spec_from_program_artifacts(
                            &stack.stack_manifest.payload.name,
                            &stack.program_specs,
                        )
                    }
                    _ => anyhow::bail!(
                        "StackManifest {} requires the composition SDK generator",
                        stack.manifest_path.display()
                    ),
                }
                .map_err(anyhow::Error::msg)?;
                if require_entities && spec.entities.is_empty() {
                    anyhow::bail!(
                        "StackManifest {} contains no entities",
                        stack.manifest_path.display()
                    );
                }
                Ok(spec)
            }
            Self::Remote(stack) => {
                let spec = match stack.live_specs.as_slice() {
                    [(_, live)] => {
                        arete_interpreter::public_artifacts::stack_spec_from_artifacts_v2(
                            &stack.program_specs,
                            live,
                            &stack.stack_manifest,
                        )
                    }
                    [] => arete_interpreter::public_artifacts::stack_spec_from_program_artifacts(
                        &stack.stack_manifest.payload.name,
                        &stack.program_specs,
                    ),
                    _ if !require_entities => {
                        arete_interpreter::public_artifacts::stack_spec_from_program_artifacts(
                            &stack.stack_manifest.payload.name,
                            &stack.program_specs,
                        )
                    }
                    _ => Err(format!(
                        "hosted stack '{}' requires the composition SDK generator",
                        stack.stack
                    )),
                }
                .map_err(anyhow::Error::msg)?;
                if require_entities && spec.entities.is_empty() {
                    anyhow::bail!("hosted stack '{}' contains no entities", stack.stack);
                }
                Ok(spec)
            }
        }
    }

    fn hosted_extensions(&self) -> Option<&ResolvedExtensionsArtifact> {
        match self {
            Self::LocalArtifacts(_) => None,
            Self::Remote(stack) => stack.hosted_extensions.as_ref(),
        }
    }

    fn composition_artifacts(&self) -> Option<CompositionArtifacts<'_>> {
        match self {
            Self::LocalArtifacts(stack) if stack.live_specs.len() > 1 => {
                Some(CompositionArtifacts {
                    program_specs: &stack.program_specs,
                    live_specs: &stack.live_specs,
                    stack_manifest: &stack.stack_manifest,
                })
            }
            Self::Remote(stack) if stack.live_specs.len() > 1 => Some(CompositionArtifacts {
                program_specs: &stack.program_specs,
                live_specs: &stack.live_specs,
                stack_manifest: &stack.stack_manifest,
            }),
            _ => None,
        }
    }

    fn composition_live_endpoints(
        &self,
    ) -> BTreeMap<String, arete_interpreter::typescript::TypeScriptLiveEndpoints> {
        match self {
            Self::Remote(stack) => stack
                .live_bindings
                .iter()
                .map(|live| {
                    (
                        live.alias.clone(),
                        arete_interpreter::typescript::TypeScriptLiveEndpoints {
                            websocket_url: Some(live.binding.websocket_endpoint.clone()),
                            http_url: Some(live.binding.query_endpoint.clone()),
                        },
                    )
                })
                .collect(),
            Self::LocalArtifacts(_) => BTreeMap::new(),
        }
    }

    fn typescript_programs(
        &self,
        stack_spec: &arete_interpreter::ast::SerializableStackSpec,
    ) -> Result<Option<Vec<arete_interpreter::typescript::TypeScriptProgramConfig>>> {
        let programs = match self {
            Self::LocalArtifacts(_) if stack_spec.program_specs.is_empty() => return Ok(None),
            Self::LocalArtifacts(_) => stack_spec
                .program_specs
                .iter()
                .map(|program_spec| {
                    arete_hash::OssProgramIdentityV1::new(program_spec.clone())
                        .map(|identity| {
                            arete_interpreter::typescript::TypeScriptProgramConfig::from(&identity)
                        })
                        .map_err(|error| anyhow::anyhow!(error))
                })
                .collect::<Result<Vec<_>>>()?,
            Self::Remote(stack) => stack
                .programs
                .iter()
                .map(typescript_program_config_from_registry)
                .collect::<Result<Vec<_>>>()?,
        };
        Ok(Some(programs))
    }
}

pub fn list(config_path: &str) -> Result<()> {
    let (manifest, plan, lock) = crate::project::installer::validate_project(config_path, false)?;
    if manifest.dependencies().next().is_none() {
        println!("{}", "No project dependencies.".yellow());
        return Ok(());
    }
    let locked = lock
        .as_ref()
        .map(|lock| {
            lock.dependencies
                .iter()
                .map(|dependency| dependency.alias.as_str())
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_default();
    for (kind, alias, dependency) in manifest.dependencies() {
        println!(
            "{} {} ({}) [{}]",
            kind,
            alias,
            dependency.source.stable_description(),
            if locked.contains(alias.as_str()) {
                "locked"
            } else {
                "unlocked"
            }
        );
        for output in plan.for_dependency(kind, alias) {
            println!("  {} -> {}", output.target, output.path.display());
        }
    }
    Ok(())
}

pub fn sync(
    config_path: &str,
    ts: bool,
    rust: bool,
    python: bool,
    stack_filters: Vec<String>,
) -> Result<()> {
    if ts || rust || python || !stack_filters.is_empty() {
        anyhow::bail!(
            "a4 sdk sync is now a project-install compatibility spelling and no longer accepts target or stack filters"
        );
    }
    crate::project::installer::install_project(
        config_path,
        crate::project::installer::InstallOptions::default(),
    )
}

fn load_local_stack_with_roots(
    manifest_path: &str,
    artifact_dirs: &[String],
) -> Result<LocalArtifactStack> {
    if artifact_dirs.is_empty() {
        return load_local_artifact_stack(Path::new(manifest_path));
    }
    let roots = artifact_dirs.iter().map(PathBuf::from).collect::<Vec<_>>();
    load_local_artifact_stack_with_roots(Path::new(manifest_path), &roots)
}

fn parse_module_imports(values: &[String], option: &str) -> Result<BTreeMap<String, String>> {
    let mut imports = BTreeMap::new();
    for value in values {
        let (alias, import) = value.split_once('=').with_context(|| {
            format!("{option} must use alias=./path.js syntax, received '{value}'")
        })?;
        if alias.is_empty()
            || !alias.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '-' | '_')
            })
            || !import.starts_with("./")
            || import.contains("..")
            || !import.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '/' | '.' | '-' | '_')
            })
        {
            anyhow::bail!("{option} must use a portable alias and relative import path");
        }
        if imports
            .insert(alias.to_string(), import.to_string())
            .is_some()
        {
            anyhow::bail!("{option} alias '{alias}' was supplied more than once");
        }
    }
    Ok(imports)
}

fn parse_live_module_imports(values: &[String]) -> Result<BTreeMap<String, String>> {
    parse_module_imports(values, "--live-module")
}

fn parse_program_module_imports(values: &[String]) -> Result<BTreeMap<String, String>> {
    parse_module_imports(values, "--program-module")
}

#[allow(clippy::too_many_arguments)]
pub fn create(
    config_path: &str,
    stack_name: Option<&str>,
    ts: bool,
    rust: bool,
    python: bool,
    output_override: Option<String>,
    package_name_override: Option<String>,
    crate_name_override: Option<String>,
    module_flag: bool,
    url_override: Option<String>,
    extensions_override: Option<String>,
    idl_override: Option<String>,
    program_spec_override: Option<String>,
    manifest_override: Option<String>,
    artifact_dirs: Vec<String>,
    live_module_values: Vec<String>,
    program_module_values: Vec<String>,
    program_only: bool,
) -> Result<()> {
    if idl_override.is_some() && !program_only {
        return Err(anyhow::anyhow!(
            "--idl is only supported together with --program-only"
        ));
    }

    match select_sdk_target(ts, rust, python, "Generate which SDK?")? {
        SdkTarget::TypeScript => create_typescript(
            config_path,
            stack_name,
            output_override,
            package_name_override,
            url_override,
            extensions_override,
            idl_override,
            program_spec_override,
            manifest_override,
            artifact_dirs,
            live_module_values,
            program_module_values,
            program_only,
        ),
        SdkTarget::Rust => {
            if program_only {
                return Err(anyhow::anyhow!(
                    "--program-only is only supported for TypeScript SDKs (--ts)"
                ));
            }
            if !live_module_values.is_empty() || !program_module_values.is_empty() {
                return Err(anyhow::anyhow!(
                    "--live-module and --program-module are only supported for TypeScript composition SDKs"
                ));
            }
            create_rust(
                config_path,
                stack_name,
                output_override,
                crate_name_override,
                module_flag,
                url_override,
                extensions_override,
                manifest_override,
                artifact_dirs,
            )
        }
        SdkTarget::Python => {
            if program_only {
                return Err(anyhow::anyhow!(
                    "--program-only is only supported for TypeScript SDKs (--ts)"
                ));
            }
            if !live_module_values.is_empty() || !program_module_values.is_empty() {
                return Err(anyhow::anyhow!(
                    "--live-module and --program-module are only supported for TypeScript composition SDKs"
                ));
            }
            create_python(
                config_path,
                stack_name,
                output_override,
                package_name_override,
                module_flag,
                url_override,
                extensions_override,
                manifest_override,
                artifact_dirs,
            )
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn create_typescript(
    config_path: &str,
    stack_name: Option<&str>,
    output_override: Option<String>,
    package_name_override: Option<String>,
    url_override: Option<String>,
    extensions_override: Option<String>,
    idl_override: Option<String>,
    program_spec_override: Option<String>,
    manifest_override: Option<String>,
    artifact_dirs: Vec<String>,
    live_module_values: Vec<String>,
    program_module_values: Vec<String>,
    program_only: bool,
) -> Result<()> {
    let _ = config_path;
    let live_module_imports = parse_live_module_imports(&live_module_values)?;
    let program_module_imports = parse_program_module_imports(&program_module_values)?;

    if let Some(program_spec_path) = program_spec_override {
        let program_spec_path = PathBuf::from(program_spec_path);
        let bytes = fs::read(&program_spec_path).with_context(|| {
            format!("Failed to read ProgramSpec {}", program_spec_path.display())
        })?;
        let program_spec = arete_artifacts::load_program_spec(&bytes)
            .with_context(|| format!("Invalid ProgramSpec {}", program_spec_path.display()))?
            .artifact;
        let sdk_name = to_kebab_case(&program_spec.payload.idl_snapshot.snapshot.name);
        let output_path = resolve_typescript_output_path_for_idl(&sdk_name, output_override);
        let package_name = package_name_override.unwrap_or_else(|| "@usearete/react".to_string());

        println!(
            "{} Generating program SDK from ProgramSpec '{}'...",
            "→".blue().bold(),
            program_spec_path.display()
        );
        generate_typescript_program_sdk_from_artifact(
            &program_spec,
            &sdk_name,
            &output_path,
            &package_name,
            extensions_override.as_deref().map(Path::new),
        )?;
        println!(
            "{} Successfully generated TypeScript SDK!",
            "✓".green().bold()
        );
        println!("  Output: {}", output_path.display().to_string().bold());
        telemetry::record_sdk_generated("typescript");
        return Ok(());
    }

    if let Some(idl_path) = idl_override {
        let idl_path = PathBuf::from(idl_path);
        let sdk_name = idl_sdk_name_from_path(&idl_path)?;
        let output_path = resolve_typescript_output_path_for_idl(&sdk_name, output_override);
        let package_name = package_name_override.unwrap_or_else(|| "@usearete/react".to_string());

        println!(
            "{} Generating program SDK from IDL '{}'...",
            "→".blue().bold(),
            idl_path.display()
        );
        println!("  Output: {}", output_path.display());

        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("Failed to create output directory: {}", parent.display())
            })?;
        }

        println!(
            "\n{} Generating TypeScript program SDK (no views)...",
            "→".blue().bold()
        );

        generate_typescript_program_sdk_from_idl(
            &idl_path,
            &output_path,
            &package_name,
            extensions_override.as_deref().map(Path::new),
        )?;

        println!(
            "{} Successfully generated TypeScript SDK!",
            "✓".green().bold()
        );
        println!("  Output: {}", output_path.display().to_string().bold());

        telemetry::record_sdk_generated("typescript");
        return Ok(());
    }

    let client = ApiClient::new()?;

    let (source, output_path, package_name, websocket_url, http_url) = if let Some(manifest_path) =
        manifest_override
    {
        let source = ResolvedStackSource::LocalArtifacts(Box::new(load_local_stack_with_roots(
            &manifest_path,
            &artifact_dirs,
        )?));
        let output = output_override
            .map(PathBuf::from)
            .unwrap_or_else(|| default_typescript_output_dir(source.sdk_name()));
        let package_name = package_name_override.unwrap_or_else(|| "@usearete/react".to_string());
        (source, output, package_name, url_override, None)
    } else {
        let stack_name = stack_name.ok_or_else(|| {
            anyhow::anyhow!("stack name is required unless using --program-spec or --manifest")
        })?;
        println!(
            "{} Looking up hosted stack '{}'...",
            "→".blue().bold(),
            stack_name
        );
        let source = resolve_remote_stack_source(&client, stack_name, None)?;
        let output = output_override
            .map(PathBuf::from)
            .unwrap_or_else(|| default_typescript_output_dir(source.sdk_name()));
        let package_name = package_name_override.unwrap_or_else(|| "@usearete/react".to_string());
        let websocket_url = url_override.or_else(|| source.default_websocket_url());
        let http_url = source.default_http_url();
        (source, output, package_name, websocket_url, http_url)
    };

    println!(
        "{} Found stack: {}",
        "✓".green().bold(),
        source.stack_id().bold()
    );
    source.print_source_details();
    println!("  Output: {}", output_path.display());
    if let Some(url) = &websocket_url {
        println!("  WebSocket URL: {}", url.cyan());
    } else {
        println!(
            "  WebSocket URL: {}",
            "(not configured - placeholder will be generated)".dimmed()
        );
    }
    if let Some(url) = &http_url {
        println!("  HTTP URL: {}", url.cyan());
    } else {
        println!(
            "  HTTP URL: {}",
            "(not configured - placeholder will be generated)".dimmed()
        );
    }

    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create output directory: {}", parent.display()))?;
    }

    if program_only {
        println!(
            "\n{} Generating TypeScript program SDK (no views)...",
            "→".blue().bold()
        );
    } else {
        println!("\n{} Generating TypeScript SDK...", "→".blue().bold());
    }

    generate_typescript_sdk_from_source(
        &source,
        &output_path,
        &package_name,
        websocket_url,
        http_url,
        extensions_override.as_deref().map(Path::new),
        &live_module_imports,
        &program_module_imports,
        program_only,
    )?;

    println!(
        "{} Successfully generated TypeScript SDK!",
        "✓".green().bold()
    );
    println!("  Output: {}", output_path.display().to_string().bold());

    telemetry::record_sdk_generated("typescript");

    Ok(())
}

fn program_read_override(
    install: &RegistryProgramInstallResponse,
) -> Result<arete_interpreter::rust::RustProgramReadConfig> {
    // Reuse the canonical transport validation used by TypeScript before
    // carrying the exact hosted descriptor into Rust/Python codegen.
    let _ = typescript_program_config_from_registry(install)?;
    let RegistryProgramInstallTransport::HostedBinding { binding } = &install.transport;
    Ok(arete_interpreter::rust::RustProgramReadConfig {
        program_id: install.definition.program_id.clone(),
        program_spec_hash: install.release.program_spec_hash.clone(),
        program_release_hash: install.release.program_release_hash.clone(),
        descriptor: Some(serde_json::json!({
            "release": {
                "programReleaseHash": install.release.program_release_hash,
                "programSpecHash": install.release.program_spec_hash,
            },
            "transport": {
                "kind": "hosted-binding",
                "binding": binding,
            },
        })),
    })
}

pub(crate) struct ProjectGenerationOptions<'a> {
    pub alias: &'a str,
    pub target: crate::project::manifest::InstallTarget,
    pub output: &'a Path,
    pub typescript_package: &'a str,
    pub rust_module: bool,
    pub python_module: bool,
}

pub(crate) fn generate_project_local_stack(
    manifest_path: &Path,
    artifact_roots: &[PathBuf],
    options: ProjectGenerationOptions<'_>,
) -> Result<()> {
    let stack = load_local_artifact_stack_with_roots(manifest_path, artifact_roots)?;
    generate_project_stack_source(
        &ResolvedStackSource::LocalArtifacts(Box::new(stack)),
        options,
    )
}

pub(crate) fn generate_project_local_program(
    program_spec_path: &Path,
    options: ProjectGenerationOptions<'_>,
) -> Result<()> {
    let bytes = fs::read(program_spec_path)
        .with_context(|| format!("Failed to read ProgramSpec {}", program_spec_path.display()))?;
    let program_spec = arete_artifacts::load_program_spec(&bytes)
        .with_context(|| format!("Invalid ProgramSpec {}", program_spec_path.display()))?
        .artifact;
    generate_project_program(&program_spec, None, options)
}

pub(crate) fn generate_project_registry_dependency(
    dependency: &crate::project::resolver::ResolvedRegistryDependency,
    options: ProjectGenerationOptions<'_>,
) -> Result<()> {
    use crate::project::resolver::ResolvedRegistryDependency;

    match dependency {
        ResolvedRegistryDependency::Program {
            alias,
            install,
            sdk_extensions,
            ..
        } => {
            if alias != options.alias {
                anyhow::bail!(
                    "Resolved program alias '{}' does not match planned alias '{}'",
                    alias,
                    options.alias
                );
            }
            let mut install = install.clone();
            install.definition.extensions = project_sdk_extension(sdk_extensions, options.target)?;
            let program_spec = program_spec_artifact_from_registry(&install)?;
            generate_project_program(&program_spec, Some(&install), options)
        }
        ResolvedRegistryDependency::Stack {
            alias,
            package,
            stack_manifest_hash,
            stack_manifest,
            live_specs,
            programs,
            sdk_extensions,
            ..
        } => {
            if alias != options.alias {
                anyhow::bail!(
                    "Resolved stack alias '{}' does not match planned alias '{}'",
                    alias,
                    options.alias
                );
            }
            let stack_manifest: arete_artifacts::StackManifestArtifactV2 =
                serde_json::from_value(stack_manifest.clone())
                    .context("Registry resolver returned an invalid V2 StackManifest")?;
            stack_manifest
                .validate()
                .context("Registry resolver returned an invalid V2 StackManifest")?;
            if stack_manifest.artifact_hash.to_string() != *stack_manifest_hash {
                anyhow::bail!("Resolved StackManifest hash does not match its artifact");
            }
            if stack_manifest.payload.live_specs.len() != live_specs.len() {
                anyhow::bail!("Resolved LiveSpec vector does not cover the StackManifest");
            }
            let mut verified_live_specs = Vec::with_capacity(live_specs.len());
            for (position, (reference, resolved)) in stack_manifest
                .payload
                .live_specs
                .iter()
                .zip(live_specs)
                .enumerate()
            {
                if reference.alias != resolved.alias
                    || reference.artifact_hash.to_string() != resolved.artifact_hash
                {
                    anyhow::bail!(
                        "Resolved LiveSpec alias/hash mismatch at position {}",
                        position
                    );
                }
                let artifact: arete_artifacts::LiveSpecArtifactV2 =
                    serde_json::from_value(resolved.artifact.clone()).with_context(|| {
                        format!("Resolved LiveSpec '{}' is invalid", resolved.alias)
                    })?;
                artifact.validate().with_context(|| {
                    format!("Resolved LiveSpec '{}' is invalid", resolved.alias)
                })?;
                if artifact.artifact_hash.to_string() != resolved.artifact_hash {
                    anyhow::bail!(
                        "Resolved LiveSpec '{}' artifact hash is incorrect",
                        resolved.alias
                    );
                }
                verified_live_specs.push((resolved.alias.clone(), artifact));
            }
            let program_specs = programs
                .iter()
                .map(program_spec_artifact_from_registry)
                .collect::<Result<Vec<_>>>()?;
            arete_artifacts::resolve_stack_composition_v2(
                &stack_manifest,
                &verified_live_specs,
                &program_specs,
            )
            .context("Resolved registry stack has an invalid artifact closure")?;
            let hosted_extensions = project_sdk_extension(sdk_extensions, options.target)?
                .as_ref()
                .map(resolved_extensions_artifact_from_registry)
                .transpose()?;
            let source = ResolvedStackSource::Remote(Box::new(RemoteStackAst {
                name: alias.clone(),
                stack: package.clone(),
                manifest_hash: stack_manifest_hash.clone(),
                program_specs,
                live_specs: verified_live_specs,
                // Endpoint descriptors are transport state, not lock identity. The
                // v1 project resolver deliberately permits placeholder endpoints.
                live_bindings: Vec::new(),
                stack_manifest,
                chain_binding: None,
                transaction_binding: None,
                exact_views: true,
                sdk_name: alias.clone(),
                hosted_extensions,
                programs: programs.clone(),
                require_managed_gateway: false,
            }));
            generate_project_stack_source(&source, options)
        }
    }
}

fn project_sdk_extension(
    extensions: &[crate::project::resolver::ResolvedSdkExtension],
    target: crate::project::manifest::InstallTarget,
) -> Result<Option<RegistrySdkExtensionArtifact>> {
    let selected = extensions
        .iter()
        .filter(|extension| extension.target == target.as_str())
        .collect::<Vec<_>>();
    if selected.len() > 1 {
        anyhow::bail!(
            "Registry resolver returned more than one {} extension",
            target
        );
    }
    Ok(selected.first().map(|extension| extension.artifact.clone()))
}

fn generate_project_stack_source(
    source: &ResolvedStackSource,
    options: ProjectGenerationOptions<'_>,
) -> Result<()> {
    use crate::project::manifest::InstallTarget;

    match options.target {
        InstallTarget::TypeScript => {
            generate_typescript_sdk_from_source(
                source,
                options.output,
                options.typescript_package,
                source.default_websocket_url(),
                source.default_http_url(),
                None,
                &BTreeMap::new(),
                &BTreeMap::new(),
                false,
            )?;
            if source.composition_artifacts().is_some() {
                write_composition_provenance(
                    options.output,
                    project_stack_manifest_hash(source)?,
                    ExtensionsInputKind::StackManifest,
                )?;
            }
            Ok(())
        }
        InstallTarget::Rust => {
            if let Some(composition) = source.composition_artifacts() {
                if source.hosted_extensions().is_some() {
                    anyhow::bail!(
                        "multi-live Rust package extensions require a composition-wrapper contract"
                    );
                }
                let output = arete_interpreter::rust::compile_composed_public_artifacts_v2(
                    composition.program_specs,
                    composition.live_specs,
                    composition.stack_manifest,
                    Some(arete_interpreter::rust::RustCompositionConfig {
                        stack: arete_interpreter::rust::RustStackConfig {
                            crate_name: format!("{}-stack", options.alias),
                            sdk_version: arete_interpreter::rust::GENERATED_RUST_SDK_VERSION
                                .to_string(),
                            module_mode: options.rust_module,
                            url: None,
                            http_url: None,
                            extension_modules: Vec::new(),
                            extension_entry: None,
                            program_reads: source.rust_program_reads()?,
                            gateway: source.hosted_gateway()?,
                        },
                        live_urls: BTreeMap::new(),
                    }),
                )
                .map_err(|error| anyhow::anyhow!("Failed to compile Rust composition: {error}"))?;
                if options.rust_module {
                    arete_interpreter::rust::write_rust_composition_module(
                        &output,
                        options.output,
                    )?;
                } else {
                    arete_interpreter::rust::write_rust_composition_crate(&output, options.output)?;
                }
                write_composition_provenance(
                    options.output,
                    project_stack_manifest_hash(source)?,
                    ExtensionsInputKind::StackManifest,
                )
            } else {
                let stack_spec = source.load_stack_spec(true)?;
                generate_rust_stack_sdk(
                    source,
                    stack_spec,
                    options.output,
                    &format!("{}-stack", options.alias),
                    options.rust_module,
                    source.default_websocket_url(),
                    None,
                )
            }
        }
        InstallTarget::Python => {
            if let Some(composition) = source.composition_artifacts() {
                if source.hosted_extensions().is_some() {
                    anyhow::bail!(
                        "multi-live Python package extensions require a composition-wrapper contract"
                    );
                }
                let output = arete_interpreter::python::compile_composed_public_artifacts_v2(
                    composition.program_specs,
                    composition.live_specs,
                    composition.stack_manifest,
                    Some(arete_interpreter::python::PythonCompositionConfig {
                        stack: arete_interpreter::python::PythonStackConfig {
                            package_name: format!("{}-stack", options.alias),
                            sdk_version: arete_interpreter::python::GENERATED_PYTHON_SDK_VERSION
                                .to_string(),
                            module_mode: options.python_module,
                            url: None,
                            http_url: None,
                            extension_modules: Vec::new(),
                            extension_entry: None,
                            program_reads: source.python_program_reads()?,
                            gateway: source.hosted_gateway()?,
                        },
                        live_urls: BTreeMap::new(),
                    }),
                )
                .map_err(|error| {
                    anyhow::anyhow!("Failed to compile Python composition: {error}")
                })?;
                if options.python_module {
                    arete_interpreter::python::write_python_composition_module(
                        &output,
                        options.output,
                    )?;
                } else {
                    arete_interpreter::python::write_python_composition_package(
                        &output,
                        options.output,
                    )?;
                }
                write_composition_provenance(
                    options.output,
                    project_stack_manifest_hash(source)?,
                    ExtensionsInputKind::StackManifest,
                )
            } else {
                let stack_spec = source.load_stack_spec(true)?;
                generate_python_stack_sdk(
                    source,
                    stack_spec,
                    options.output,
                    &format!("{}-stack", options.alias),
                    options.python_module,
                    source.default_websocket_url(),
                    None,
                )
            }
        }
    }
}

fn generate_project_program(
    program_spec: &arete_artifacts::ProgramSpecArtifact,
    install: Option<&RegistryProgramInstallResponse>,
    options: ProjectGenerationOptions<'_>,
) -> Result<()> {
    use crate::project::manifest::InstallTarget;

    match options.target {
        InstallTarget::TypeScript => {
            if let Some(install) = install {
                let hosted_artifact = install
                    .definition
                    .extensions
                    .as_ref()
                    .map(resolved_extensions_artifact_from_registry)
                    .transpose()?;
                generate_typescript_program_sdk_from_install(
                    install,
                    options.alias,
                    options.output,
                    options.typescript_package,
                    None,
                    hosted_artifact.as_ref(),
                )
            } else {
                generate_typescript_program_sdk_from_artifact(
                    program_spec,
                    options.alias,
                    options.output,
                    options.typescript_package,
                    None,
                )
            }
        }
        InstallTarget::Rust => generate_project_rust_program(program_spec, install, options),
        InstallTarget::Python => generate_project_python_program(program_spec, install, options),
    }
}

fn generate_project_rust_program(
    program_spec: &arete_artifacts::ProgramSpecArtifact,
    install: Option<&RegistryProgramInstallResponse>,
    options: ProjectGenerationOptions<'_>,
) -> Result<()> {
    let stack_spec = arete_interpreter::public_artifacts::stack_spec_from_program_artifacts(
        options.alias,
        std::slice::from_ref(program_spec),
    )
    .map_err(anyhow::Error::msg)?;
    let input_pin = ResolvedExtensionsInputPin {
        kind: ExtensionsInputKind::ProgramSpec,
        hash: program_spec.artifact_hash.to_string(),
    };
    let hosted_artifact = install
        .and_then(|install| install.definition.extensions.as_ref())
        .map(resolved_extensions_artifact_from_registry)
        .transpose()?;
    let module_dir = if options.rust_module {
        options.output.to_path_buf()
    } else {
        options.output.join("src")
    };
    let artifact = resolve_rust_extensions_artifact(
        None,
        hosted_artifact.as_ref(),
        &module_dir,
        options.alias,
        OutputExtensionsFallback::Ignore,
    )?;
    let (extension_modules, extension_entry) = match artifact.as_ref() {
        Some(artifact) => {
            let (modules, entry) = rust_extension_wiring(artifact)?;
            (modules, Some(entry))
        }
        None => (Vec::new(), None),
    };
    let rust_config = arete_interpreter::rust::RustStackConfig {
        crate_name: format!("{}-program", options.alias),
        sdk_version: arete_interpreter::rust::GENERATED_RUST_SDK_VERSION.to_string(),
        module_mode: options.rust_module,
        url: None,
        http_url: None,
        extension_modules,
        extension_entry,
        program_reads: install
            .map(program_read_override)
            .transpose()?
            .into_iter()
            .collect(),
        gateway: install
            .map(|install| {
                optional_gateway_descriptor(
                    install.chain_binding.as_ref(),
                    install.transaction_binding.as_ref(),
                    &format!("resolved program '{}'", install.install_name),
                )
            })
            .transpose()?
            .flatten(),
    };
    let output = arete_interpreter::rust::compile_program_modules(stack_spec, Some(rust_config))
        .map_err(|error| anyhow::anyhow!("Failed to compile Rust program SDK: {error}"))?;
    let generated = if options.rust_module {
        arete_interpreter::rust::write_rust_program_module(&output, options.output)?;
        BTreeSet::from([
            "mod.rs".to_string(),
            "types.rs".to_string(),
            "programs.rs".to_string(),
        ])
    } else {
        arete_interpreter::rust::write_rust_program_crate(&output, options.output)?;
        BTreeSet::from([
            "Cargo.toml".to_string(),
            "src/lib.rs".to_string(),
            "src/types.rs".to_string(),
            "src/programs.rs".to_string(),
        ])
    };
    if let Some(artifact) = artifact.as_ref() {
        stage_rust_extensions_artifact(artifact, &module_dir, &input_pin)?;
    }
    write_language_sdk_provenance_manifest(
        options.output,
        generated,
        if options.rust_module { "" } else { "src/" },
        &input_pin,
        artifact.as_ref(),
    )
}

fn generate_project_python_program(
    program_spec: &arete_artifacts::ProgramSpecArtifact,
    install: Option<&RegistryProgramInstallResponse>,
    options: ProjectGenerationOptions<'_>,
) -> Result<()> {
    let package_name = format!("{}-program", options.alias);
    let stack_spec = arete_interpreter::public_artifacts::stack_spec_from_program_artifacts(
        options.alias,
        std::slice::from_ref(program_spec),
    )
    .map_err(anyhow::Error::msg)?;
    let input_pin = ResolvedExtensionsInputPin {
        kind: ExtensionsInputKind::ProgramSpec,
        hash: program_spec.artifact_hash.to_string(),
    };
    let hosted_artifact = install
        .and_then(|install| install.definition.extensions.as_ref())
        .map(resolved_extensions_artifact_from_registry)
        .transpose()?;
    let import_module = arete_interpreter::python::python_module_name(&package_name);
    let module_dir = if options.python_module {
        options.output.to_path_buf()
    } else {
        options.output.join(&import_module)
    };
    let artifact = resolve_python_extensions_artifact(
        None,
        hosted_artifact.as_ref(),
        &module_dir,
        options.alias,
        OutputExtensionsFallback::Ignore,
    )?;
    let (extension_modules, extension_entry) = match artifact.as_ref() {
        Some(artifact) => {
            let (modules, entry) = python_extension_wiring(artifact)?;
            (modules, Some(entry))
        }
        None => (Vec::new(), None),
    };
    let program_reads = install
        .map(program_read_override)
        .transpose()?
        .into_iter()
        .map(|read| arete_interpreter::python::PythonProgramReadConfig {
            program_id: read.program_id,
            program_spec_hash: read.program_spec_hash,
            program_release_hash: read.program_release_hash,
            descriptor: read.descriptor,
        })
        .collect();
    let python_config = arete_interpreter::python::PythonStackConfig {
        package_name,
        sdk_version: arete_interpreter::python::GENERATED_PYTHON_SDK_VERSION.to_string(),
        module_mode: options.python_module,
        url: None,
        http_url: None,
        extension_modules,
        extension_entry,
        program_reads,
        gateway: install
            .map(|install| {
                optional_gateway_descriptor(
                    install.chain_binding.as_ref(),
                    install.transaction_binding.as_ref(),
                    &format!("resolved program '{}'", install.install_name),
                )
            })
            .transpose()?
            .flatten(),
    };
    let output =
        arete_interpreter::python::compile_program_modules(stack_spec, Some(python_config))
            .map_err(|error| anyhow::anyhow!("Failed to compile Python program SDK: {error}"))?;
    let generated = if options.python_module {
        arete_interpreter::python::write_python_program_module(&output, options.output)?;
        BTreeSet::from([
            "__init__.py".to_string(),
            "models.py".to_string(),
            "programs.py".to_string(),
        ])
    } else {
        arete_interpreter::python::write_python_program_package(&output, options.output)?;
        BTreeSet::from([
            "pyproject.toml".to_string(),
            format!("{}/__init__.py", output.module_name),
            format!("{}/models.py", output.module_name),
            format!("{}/programs.py", output.module_name),
        ])
    };
    if let Some(artifact) = artifact.as_ref() {
        stage_python_extensions_artifact(artifact, &module_dir, &input_pin)?;
    }
    write_language_sdk_provenance_manifest(
        options.output,
        generated,
        if options.python_module {
            "".to_string()
        } else {
            format!("{}/", output.module_name)
        }
        .as_str(),
        &input_pin,
        artifact.as_ref(),
    )
}

fn write_composition_provenance(
    output: &Path,
    stack_manifest_hash: &str,
    kind: ExtensionsInputKind,
) -> Result<()> {
    let mut generated = BTreeSet::new();
    collect_relative_files(output, output, &mut generated)?;
    write_language_sdk_provenance_manifest(
        output,
        generated,
        "",
        &ResolvedExtensionsInputPin {
            kind,
            hash: stack_manifest_hash.to_string(),
        },
        None,
    )
}

fn project_stack_manifest_hash(source: &ResolvedStackSource) -> Result<&str> {
    match source {
        ResolvedStackSource::LocalArtifacts(stack) => Ok(&stack.manifest_hash),
        ResolvedStackSource::Remote(stack) => Ok(&stack.manifest_hash),
    }
}

fn collect_relative_files(
    root: &Path,
    directory: &Path,
    files: &mut BTreeSet<String>,
) -> Result<()> {
    for entry in fs::read_dir(directory)
        .with_context(|| format!("Failed to inspect generated output {}", directory.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_relative_files(root, &path, files)?;
        } else if path.file_name().and_then(|name| name.to_str()) != Some(SDK_PROVENANCE_FILE)
            && path != root.join(SDK_MANIFEST_FILE)
        {
            files.insert(
                path.strip_prefix(root)
                    .context("Generated output escaped its staging root")?
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
    }
    Ok(())
}

fn select_sdk_target(ts: bool, rust: bool, python: bool, prompt: &str) -> Result<SdkTarget> {
    match (ts, rust, python) {
        (true, false, false) => Ok(SdkTarget::TypeScript),
        (false, true, false) => Ok(SdkTarget::Rust),
        (false, false, true) => Ok(SdkTarget::Python),
        (false, false, false) => {
            let theme = ColorfulTheme::default();
            let items = ["TypeScript", "Rust", "Python"];
            let selection = Select::with_theme(&theme)
                .with_prompt(prompt)
                .items(&items)
                .default(0)
                .interact()
                .context("Failed to select SDK language")?;

            Ok(match selection {
                0 => SdkTarget::TypeScript,
                1 => SdkTarget::Rust,
                2 => SdkTarget::Python,
                _ => unreachable!(),
            })
        }
        _ => Err(anyhow::anyhow!(
            "Cannot specify more than one of --ts, --rust, and --python. Choose one."
        )),
    }
}

fn default_typescript_dir_name(sdk_name: &str) -> String {
    sdk_name
        .strip_suffix("-stream")
        .unwrap_or(sdk_name)
        .to_string()
}

fn default_typescript_output_dir(sdk_name: &str) -> PathBuf {
    PathBuf::from("./generated").join(default_typescript_dir_name(sdk_name))
}

fn idl_sdk_name_from_path(path: &Path) -> Result<String> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow::anyhow!("Invalid IDL path: {}", path.display()))?;

    if let Some(base) = file_name.strip_suffix(".idl.json") {
        return Ok(base.to_string());
    }
    if let Some(base) = file_name.strip_suffix(".json") {
        return Ok(base.to_string());
    }
    if let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) {
        return Ok(stem.to_string());
    }

    Err(anyhow::anyhow!(
        "Unable to derive SDK name from IDL path: {}",
        path.display()
    ))
}

fn to_pascal_case(input: &str) -> String {
    input
        .split(['-', '_', ' '])
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            let mut chars = segment.chars();
            match chars.next() {
                Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect()
}

fn to_camel_case(input: &str) -> String {
    let pascal = to_pascal_case(input);
    let mut chars = pascal.chars();
    chars
        .next()
        .map(|first| first.to_ascii_lowercase().to_string() + chars.as_str())
        .unwrap_or_default()
}

fn resolve_typescript_output_path_for_idl(
    sdk_name: &str,
    output_override: Option<String>,
) -> PathBuf {
    output_override
        .map(PathBuf::from)
        .unwrap_or_else(|| default_typescript_output_dir(sdk_name))
}

fn extension_entry_stem(base_name: &str) -> String {
    base_name
        .strip_suffix("-stream")
        .map(|base| format!("{}-extensions", base))
        .unwrap_or_else(|| format!("{}-extensions", base_name))
}

fn resolve_typescript_layout(output_path: &Path, default_base_name: &str) -> TypeScriptLayout {
    let is_ts_file = output_path.extension().and_then(|ext| ext.to_str()) == Some("ts");
    if is_ts_file {
        let output_dir = output_path.parent().unwrap_or(Path::new(".")).to_path_buf();
        let base_name = output_path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or(default_base_name)
            .to_string();
        TypeScriptLayout {
            output_dir: output_dir.clone(),
            base_name: base_name.clone(),
            entry_path: output_path.to_path_buf(),
            core_path: output_dir.join(format!("{}-core.ts", base_name)),
        }
    } else {
        let output_dir = output_path.to_path_buf();
        TypeScriptLayout {
            output_dir: output_dir.clone(),
            base_name: default_base_name.to_string(),
            entry_path: output_dir.join(format!("{}.ts", default_base_name)),
            core_path: output_dir.join(format!("{}-core.ts", default_base_name)),
        }
    }
}

fn read_extensions_manifest(manifest_path: &Path) -> Result<ExtensionsManifest> {
    let manifest_json = fs::read_to_string(manifest_path).with_context(|| {
        format!(
            "Failed to read extensions manifest: {}",
            manifest_path.display()
        )
    })?;
    serde_json::from_str(&manifest_json).with_context(|| {
        format!(
            "Failed to parse extensions manifest: {}",
            manifest_path.display()
        )
    })
}

fn normalize_extension_relative_path(path: &str) -> Result<String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(anyhow::anyhow!("Extension file paths cannot be empty"));
    }

    let mut parts = Vec::new();
    for component in Path::new(trimmed).components() {
        match component {
            Component::Normal(part) => parts.push(part.to_string_lossy().to_string()),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(anyhow::anyhow!(
                    "Extension file path '{}' must be a normalized relative path",
                    path
                ));
            }
        }
    }

    if parts.is_empty() {
        return Err(anyhow::anyhow!(
            "Extension file path '{}' must not resolve to the current directory",
            path
        ));
    }

    Ok(parts.join("/"))
}

fn read_extensions_files(
    source_dir: &Path,
    files: &[String],
) -> Result<Vec<ResolvedExtensionsFile>> {
    let mut resolved = Vec::with_capacity(files.len());
    for relative_path in files {
        let normalized = normalize_extension_relative_path(relative_path)?;
        let contents = fs::read_to_string(source_dir.join(relative_path)).with_context(|| {
            format!(
                "Failed to read extensions artifact file: {}",
                source_dir.join(relative_path).display()
            )
        })?;
        resolved.push(ResolvedExtensionsFile {
            path: normalized,
            contents,
        });
    }
    resolved.sort_by(|left, right| left.path.cmp(&right.path));
    resolved.dedup_by(|left, right| left.path == right.path);
    Ok(resolved)
}

fn build_extensions_artifact(
    entry: String,
    files: Vec<ResolvedExtensionsFile>,
    input_kind: Option<ExtensionsInputKind>,
    input_hash: Option<String>,
    sdk_range: Option<String>,
    language: Option<String>,
) -> Result<ResolvedExtensionsArtifact> {
    let entry = normalize_extension_relative_path(&entry)?;
    let entry_source = files
        .iter()
        .find(|file| file.path == entry)
        .map(|file| file.contents.as_str())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Extensions entry '{}' is missing from artifact files",
                entry
            )
        })?;
    let program_extension_bindings = parse_program_extension_bindings(entry_source);

    Ok(ResolvedExtensionsArtifact {
        entry,
        files,
        input_kind,
        input_hash,
        sdk_range,
        language,
        sdk_extension_hash: None,
        sdk_output_tree_hash: None,
        program_extension_bindings,
    })
}

fn infer_extensions_artifact_from_entry(entry_path: &Path) -> Result<ResolvedExtensionsArtifact> {
    infer_extensions_artifact_from_entry_with_language(entry_path, None)
}

fn infer_extensions_artifact_from_entry_with_language(
    entry_path: &Path,
    language: Option<String>,
) -> Result<ResolvedExtensionsArtifact> {
    let source_dir = entry_path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let entry = entry_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow::anyhow!("Invalid extensions entry path: {}", entry_path.display()))?
        .to_string();

    build_extensions_artifact(
        entry.clone(),
        read_extensions_files(&source_dir, &[entry])?,
        None,
        None,
        None,
        language,
    )
}

fn parse_program_extension_bindings(source: &str) -> Vec<ProgramExtensionBinding> {
    let regex = Regex::new(
        r"export\s+const\s+([A-Za-z_][A-Za-z0-9_]*)\s*=\s*defineProgramExtensions\s*<\s*typeof\s+[A-Za-z_][A-Za-z0-9_]*\.programs\.([A-Za-z_][A-Za-z0-9_]*)\s*>\s*\(\s*\)"
    )
    .expect("program extension binding regex should compile");

    let mut bindings = regex
        .captures_iter(source)
        .map(|captures| ProgramExtensionBinding {
            export_name: captures[1].to_string(),
            program_key: captures[2].to_string(),
        })
        .collect::<Vec<_>>();
    bindings.sort_by(|left, right| {
        left.program_key
            .cmp(&right.program_key)
            .then(left.export_name.cmp(&right.export_name))
    });
    bindings.dedup();
    bindings
}

fn build_extensions_artifact_from_manifest(
    manifest: ExtensionsManifest,
    source_dir: &Path,
) -> Result<ResolvedExtensionsArtifact> {
    build_extensions_artifact(
        manifest.entry,
        read_extensions_files(source_dir, &manifest.files)?,
        manifest.input_kind,
        manifest.input_hash,
        manifest.sdk_range,
        manifest.language,
    )
}

/// Reject bundles authored for the other SDK language. Bundles without a
/// declared `language` stay accepted by both pipelines for back-compat.
fn ensure_extensions_language(artifact: &ResolvedExtensionsArtifact, expected: &str) -> Result<()> {
    match artifact.language.as_deref() {
        None => Ok(()),
        Some(language) if language == expected => Ok(()),
        Some(language) => Err(anyhow::anyhow!(
            "Extensions bundle declares language '{}' but this is a {} SDK generation; \
             pass a {} extensions bundle instead",
            language,
            expected,
            expected
        )),
    }
}

fn resolve_explicit_extensions_artifact(
    path: &Path,
    layout: &TypeScriptLayout,
) -> Result<ResolvedExtensionsArtifact> {
    if path.is_dir() {
        let manifest_path = path.join("extensions.json");
        if manifest_path.exists() {
            let manifest = read_extensions_manifest(&manifest_path)?;
            return build_extensions_artifact_from_manifest(manifest, path);
        }

        let explicit_entry = path.join(format!("{}.ts", extension_entry_stem(&layout.base_name)));
        if explicit_entry.exists() {
            return infer_extensions_artifact_from_entry(&explicit_entry);
        }

        let index_entry = path.join("index.ts");
        if index_entry.exists() {
            return infer_extensions_artifact_from_entry(&index_entry);
        }

        return Err(anyhow::anyhow!(
            "No extensions manifest or entry file found in {}",
            path.display()
        ));
    }

    if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
        let manifest = read_extensions_manifest(path)?;
        let source_dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        return build_extensions_artifact_from_manifest(manifest, &source_dir);
    }

    infer_extensions_artifact_from_entry(path)
}

/// Resolve the devex extensions bundle for a TypeScript SDK generation.
///
/// Precedence: explicit `--extensions` path, then a hosted registry
/// artifact, then (when `output_fallback` permits it) an `extensions.json`
/// already staged in the output directory, then bare entry-file inference
/// from the output directory. Local regeneration reuses staged bundles to
/// keep their input pins and helper files intact. Registry installs ignore
/// staged bundles so the hosted response, including the absence of a bundle,
/// is authoritative. Mirroring the Rust rung, a reused staged manifest
/// declaring the other SDK language is a hard error rather than a silent
/// skip.
fn resolve_extensions_artifact(
    explicit_path: Option<&Path>,
    layout: &TypeScriptLayout,
    hosted_artifact: Option<&ResolvedExtensionsArtifact>,
    output_fallback: OutputExtensionsFallback,
) -> Result<Option<ResolvedExtensionsArtifact>> {
    let artifact = if let Some(path) = explicit_path {
        Some(resolve_explicit_extensions_artifact(path, layout)?)
    } else if let Some(artifact) = hosted_artifact {
        Some(artifact.clone())
    } else if matches!(output_fallback, OutputExtensionsFallback::Reuse) {
        let manifest_path = layout.output_dir.join("extensions.json");
        if manifest_path.exists() {
            let manifest = read_extensions_manifest(&manifest_path)?;
            Some(build_extensions_artifact_from_manifest(
                manifest,
                &layout.output_dir,
            )?)
        } else {
            let inferred_entry = layout
                .output_dir
                .join(format!("{}.ts", extension_entry_stem(&layout.base_name)));
            if inferred_entry.exists() {
                Some(infer_extensions_artifact_from_entry(&inferred_entry)?)
            } else {
                None
            }
        }
    } else {
        None
    };

    if let Some(ref artifact) = artifact {
        ensure_extensions_language(artifact, EXTENSIONS_LANGUAGE_TYPESCRIPT)?;
    }
    Ok(artifact)
}

/// Resolve the devex extensions bundle for a Rust SDK generation.
///
/// Precedence: explicit `--extensions` path, then a hosted registry artifact
/// (only when its manifest declares `language: "rust"`), then an
/// `extensions.json` already staged in the output module directory when
/// `output_fallback` permits it. Registry installs ignore staged bundles so
/// the hosted response is authoritative. Unlike the TypeScript pipeline
/// there is no final bare entry-file inference from the output directory.
fn resolve_rust_extensions_artifact(
    explicit_path: Option<&Path>,
    hosted_artifact: Option<&ResolvedExtensionsArtifact>,
    output_module_dir: &Path,
    base_stem: &str,
    output_fallback: OutputExtensionsFallback,
) -> Result<Option<ResolvedExtensionsArtifact>> {
    let artifact = if let Some(path) = explicit_path {
        Some(resolve_explicit_rust_extensions_artifact(path, base_stem)?)
    } else if let Some(artifact) = hosted_artifact
        .filter(|artifact| artifact.language.as_deref() == Some(EXTENSIONS_LANGUAGE_RUST))
    {
        Some(artifact.clone())
    } else if matches!(output_fallback, OutputExtensionsFallback::Reuse) {
        let manifest_path = output_module_dir.join("extensions.json");
        if manifest_path.exists() {
            let manifest = read_extensions_manifest(&manifest_path)?;
            Some(build_extensions_artifact_from_manifest(
                manifest,
                output_module_dir,
            )?)
        } else {
            None
        }
    } else {
        None
    };

    if let Some(ref artifact) = artifact {
        ensure_extensions_language(artifact, EXTENSIONS_LANGUAGE_RUST)?;
    }
    Ok(artifact)
}

/// Compute the `pub mod` wiring stems for a staged Rust bundle: helper files
/// first (manifest order, which is sorted), entry stem last. Duplicate stems
/// and collisions with generated modules are rejected by the compiler.
fn rust_extension_wiring(artifact: &ResolvedExtensionsArtifact) -> Result<(Vec<String>, String)> {
    let entry_stem = rust_extension_module_stem(&artifact.entry)?;
    let mut stems = Vec::new();
    for file in &artifact.files {
        if file.path == artifact.entry {
            continue;
        }
        stems.push(rust_extension_module_stem(&file.path)?);
    }
    stems.push(entry_stem.clone());
    Ok((stems, entry_stem))
}

fn rust_extension_module_stem(path: &str) -> Result<String> {
    let normalized = normalize_extension_relative_path(path)?;
    let stem = normalized.strip_suffix(".rs").ok_or_else(|| {
        anyhow::anyhow!(
            "Rust extensions bundles require .rs files; '{}' is not supported",
            path
        )
    })?;
    if stem.contains('/') {
        return Err(anyhow::anyhow!(
            "Rust extensions bundles require flat .rs files; '{}' is not supported",
            path
        ));
    }
    Ok(arete_interpreter::rust::rust_module_name(stem))
}

fn resolve_explicit_rust_extensions_artifact(
    path: &Path,
    base_stem: &str,
) -> Result<ResolvedExtensionsArtifact> {
    if path.is_dir() {
        let manifest_path = path.join("extensions.json");
        if manifest_path.exists() {
            let manifest = read_extensions_manifest(&manifest_path)?;
            return build_extensions_artifact_from_manifest(manifest, path);
        }

        let entry = path.join("extensions.rs");
        if entry.exists() {
            return infer_extensions_artifact_from_entry_with_language(
                &entry,
                Some(EXTENSIONS_LANGUAGE_RUST.to_string()),
            );
        }

        return Err(anyhow::anyhow!(
            "No extensions.json manifest or extensions.rs entry found in {} for the '{}' Rust SDK",
            path.display(),
            base_stem
        ));
    }

    if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
        let manifest = read_extensions_manifest(path)?;
        let source_dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        return build_extensions_artifact_from_manifest(manifest, &source_dir);
    }

    infer_extensions_artifact_from_entry_with_language(
        path,
        Some(EXTENSIONS_LANGUAGE_RUST.to_string()),
    )
}

/// Resolve the devex extensions bundle for a Python SDK generation.
///
/// Mirror of [`resolve_rust_extensions_artifact`]: explicit `--extensions`
/// path, then a hosted registry artifact (only when its manifest declares
/// `language: "python"`), then an `extensions.json` already staged in the
/// output module directory when `output_fallback` permits it. Registry
/// installs ignore staged bundles so the hosted response is authoritative.
/// There is no final bare entry-file inference from the output directory.
fn resolve_python_extensions_artifact(
    explicit_path: Option<&Path>,
    hosted_artifact: Option<&ResolvedExtensionsArtifact>,
    output_module_dir: &Path,
    base_stem: &str,
    output_fallback: OutputExtensionsFallback,
) -> Result<Option<ResolvedExtensionsArtifact>> {
    let artifact = if let Some(path) = explicit_path {
        Some(resolve_explicit_python_extensions_artifact(
            path, base_stem,
        )?)
    } else if let Some(artifact) = hosted_artifact
        .filter(|artifact| artifact.language.as_deref() == Some(EXTENSIONS_LANGUAGE_PYTHON))
    {
        Some(artifact.clone())
    } else if matches!(output_fallback, OutputExtensionsFallback::Reuse) {
        let manifest_path = output_module_dir.join("extensions.json");
        if manifest_path.exists() {
            let manifest = read_extensions_manifest(&manifest_path)?;
            Some(build_extensions_artifact_from_manifest(
                manifest,
                output_module_dir,
            )?)
        } else {
            None
        }
    } else {
        None
    };

    if let Some(ref artifact) = artifact {
        ensure_extensions_language(artifact, EXTENSIONS_LANGUAGE_PYTHON)?;
    }
    Ok(artifact)
}

/// Compute the `from . import <stem>` wiring stems for a staged Python
/// bundle: helper files first (manifest order, which is sorted), entry stem
/// last. Duplicate stems and collisions with generated modules are rejected
/// by the compiler.
fn python_extension_wiring(artifact: &ResolvedExtensionsArtifact) -> Result<(Vec<String>, String)> {
    let entry_stem = python_extension_module_stem(&artifact.entry)?;
    let mut stems = Vec::new();
    for file in &artifact.files {
        if file.path == artifact.entry {
            continue;
        }
        stems.push(python_extension_module_stem(&file.path)?);
    }
    stems.push(entry_stem.clone());
    Ok((stems, entry_stem))
}

fn python_extension_module_stem(path: &str) -> Result<String> {
    let normalized = normalize_extension_relative_path(path)?;
    let stem = normalized.strip_suffix(".py").ok_or_else(|| {
        anyhow::anyhow!(
            "Python extensions bundles require .py files; '{}' is not supported",
            path
        )
    })?;
    if stem.contains('/') {
        return Err(anyhow::anyhow!(
            "Python extensions bundles require flat .py files; '{}' is not supported",
            path
        ));
    }
    Ok(arete_interpreter::python::python_module_name(stem))
}

fn resolve_explicit_python_extensions_artifact(
    path: &Path,
    base_stem: &str,
) -> Result<ResolvedExtensionsArtifact> {
    if path.is_dir() {
        let manifest_path = path.join("extensions.json");
        if manifest_path.exists() {
            let manifest = read_extensions_manifest(&manifest_path)?;
            return build_extensions_artifact_from_manifest(manifest, path);
        }

        let entry = path.join("extensions.py");
        if entry.exists() {
            return infer_extensions_artifact_from_entry_with_language(
                &entry,
                Some(EXTENSIONS_LANGUAGE_PYTHON.to_string()),
            );
        }

        return Err(anyhow::anyhow!(
            "No extensions.json manifest or extensions.py entry found in {} for the '{}' Python SDK",
            path.display(),
            base_stem
        ));
    }

    if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
        let manifest = read_extensions_manifest(path)?;
        let source_dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        return build_extensions_artifact_from_manifest(manifest, &source_dir);
    }

    infer_extensions_artifact_from_entry_with_language(
        path,
        Some(EXTENSIONS_LANGUAGE_PYTHON.to_string()),
    )
}

fn version_satisfies_range(current: &str, range: &str) -> bool {
    let Ok(current) = Version::parse(current) else {
        return false;
    };
    let requirements = range.split("||").map(str::trim).collect::<Vec<_>>();
    if requirements.is_empty()
        || requirements
            .iter()
            .any(|requirement| requirement.is_empty())
    {
        return false;
    }
    requirements.iter().any(|requirement| {
        VersionReq::parse(requirement)
            .map(|range| range.matches(&current))
            .unwrap_or(false)
    })
}

fn discover_usearete_sdk_version(start_dir: &Path) -> Option<String> {
    for ancestor in start_dir.ancestors() {
        let manifest_path = ancestor.join("node_modules/@usearete/sdk/package.json");
        if !manifest_path.exists() {
            continue;
        }

        let manifest_json = fs::read_to_string(&manifest_path).ok()?;
        let manifest: PackageVersionManifest = serde_json::from_str(&manifest_json).ok()?;
        return Some(manifest.version);
    }

    None
}

/// Best-effort discovery of the `arete-a4-sdk` dependency version declared by
/// a `Cargo.toml` at or above `start_dir`. Only an exact `major.minor.patch`
/// version is returned (dependency *requirements* like `"0"` or `"0.4"` are
/// not comparable against an extensions `sdkRange` and are skipped).
fn discover_arete_sdk_crate_version(start_dir: &Path) -> Option<String> {
    let version_regex = Regex::new(
        r#"(?m)^\s*(?:arete-a4-sdk|arete-sdk)\s*=\s*(?:"([^"]+)"|\{[^}]*version\s*=\s*"([^"]+)"[^}]*\})"#,
    )
    .expect("arete sdk version regex should compile");

    for ancestor in start_dir.ancestors() {
        let manifest_path = ancestor.join("Cargo.toml");
        let Ok(manifest) = fs::read_to_string(&manifest_path) else {
            continue;
        };
        for captures in version_regex.captures_iter(&manifest) {
            let declared = captures
                .get(1)
                .or_else(|| captures.get(2))
                .map(|value| value.as_str());
            if let Some(declared) = declared {
                if Version::parse(declared).is_ok() {
                    return Some(declared.to_string());
                }
            }
        }
    }

    None
}

/// Best-effort discovery of the `arete-sdk` dependency version pinned by a
/// `pyproject.toml` at or above `start_dir`. Only an exact `==major.minor.patch`
/// pin is returned (requirement *ranges* like `>=0.4` are not comparable
/// against an extensions `sdkRange` and are skipped). Mirror of
/// [`discover_arete_sdk_crate_version`].
fn discover_arete_sdk_python_version(start_dir: &Path) -> Option<String> {
    let version_regex = Regex::new(r#"["']arete-sdk\s*==\s*([0-9]+\.[0-9]+\.[0-9]+)["']"#)
        .expect("arete python sdk version regex should compile");

    for ancestor in start_dir.ancestors() {
        let manifest_path = ancestor.join("pyproject.toml");
        let Ok(manifest) = fs::read_to_string(&manifest_path) else {
            continue;
        };
        for captures in version_regex.captures_iter(&manifest) {
            if let Some(declared) = captures.get(1).map(|value| value.as_str()) {
                if Version::parse(declared).is_ok() {
                    return Some(declared.to_string());
                }
            }
        }
    }

    None
}

fn build_pda_degradation_summary(
    degradations: &[arete_interpreter::typescript_instructions::PdaDegradation],
) -> Vec<String> {
    if degradations.is_empty() {
        return Vec::new();
    }

    let instruction_count = degradations
        .iter()
        .map(|degradation| degradation.instruction_name.as_str())
        .collect::<BTreeSet<_>>()
        .len();
    let mut reasons: BTreeMap<&str, usize> = BTreeMap::new();
    for degradation in degradations {
        *reasons.entry(degradation.reason.as_str()).or_insert(0) += 1;
    }

    let mut lines = vec![format!(
        "{} {} PDA account(s) degraded to userProvided across {} instruction(s)",
        "⚠".yellow().bold(),
        degradations.len(),
        instruction_count,
    )];
    for (reason, count) in reasons {
        lines.push(format!("   {}x {}", count, reason));
    }
    lines
}

fn print_pda_degradation_summary(
    degradations: &[arete_interpreter::typescript_instructions::PdaDegradation],
) {
    for line in build_pda_degradation_summary(degradations) {
        println!("{}", line);
    }
}

fn update_hash_part(hasher: &mut Sha256, label: &str, value: &[u8]) {
    hasher.update((label.len() as u64).to_be_bytes());
    hasher.update(label.as_bytes());
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value);
}

fn sdk_compiler_hash() -> Result<arete_hash::HashId<arete_hash::Compiler>> {
    env!("ARETE_SDK_COMPILER_HASH")
        .parse()
        .context("Build embedded an invalid SDK compiler hash")
}

fn extensions_artifact_hash(artifact: &ResolvedExtensionsArtifact) -> String {
    let mut hasher = Sha256::new();
    update_hash_part(&mut hasher, "entry", artifact.entry.as_bytes());
    update_hash_part(
        &mut hasher,
        "input-kind",
        artifact
            .input_kind
            .map(ExtensionsInputKind::as_manifest_value)
            .unwrap_or("")
            .as_bytes(),
    );
    update_hash_part(
        &mut hasher,
        "input-hash",
        artifact.input_hash.as_deref().unwrap_or("").as_bytes(),
    );
    update_hash_part(
        &mut hasher,
        "sdk-range",
        artifact.sdk_range.as_deref().unwrap_or("").as_bytes(),
    );

    let mut files = artifact.files.iter().collect::<Vec<_>>();
    files.sort_by(|left, right| left.path.cmp(&right.path));
    for file in files {
        update_hash_part(&mut hasher, "file-path", file.path.as_bytes());
        update_hash_part(&mut hasher, "file-contents", file.contents.as_bytes());
    }

    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{:02x}", byte))
        .collect()
}

fn generated_artifact_name(path: &Path) -> Result<String> {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("Invalid generated artifact path: {}", path.display()))
}

fn build_sdk_provenance_manifest(
    layout: &TypeScriptLayout,
    input_pin: &ResolvedExtensionsInputPin,
    extensions: Option<&ResolvedExtensionsArtifact>,
) -> Result<SdkProvenanceManifestV2> {
    build_sdk_provenance_manifest_with_program_extensions(layout, input_pin, extensions, &[])
}

fn build_sdk_provenance_manifest_with_program_extensions(
    layout: &TypeScriptLayout,
    input_pin: &ResolvedExtensionsInputPin,
    extensions: Option<&ResolvedExtensionsArtifact>,
    program_modules: &[HostedProgramModule],
) -> Result<SdkProvenanceManifestV2> {
    let mut generated = typescript_core_paths(layout, extensions)?;
    generated.insert(generated_artifact_name(&layout.entry_path)?);
    let mut manifest =
        build_sdk_provenance_manifest_from_artifacts(generated, "", input_pin, extensions)?;
    for program in program_modules {
        let relative_dir = hosted_program_directory(program);
        let prefix = format!("{}/", relative_dir.to_string_lossy());
        manifest
            .artifacts
            .push(format!("{prefix}{HOSTED_PROGRAM_ENTRY}"));
        for path in hosted_program_core_paths(program)? {
            manifest.artifacts.push(format!("{prefix}{path}"));
        }
        if let Some(extension) = &program.extension {
            manifest.artifacts.push(format!("{prefix}extensions.json"));
            for file in &extension.files {
                manifest.artifacts.push(format!(
                    "{prefix}{}",
                    normalize_extension_relative_path(&file.path)?
                ));
            }
            manifest.program_extensions.insert(
                program.program_key.clone(),
                SdkProvenanceProgramExtensionV2 {
                    input: SdkProvenanceInputV2 {
                        kind: program.input_pin.kind,
                        hash: program.input_pin.hash.clone(),
                    },
                    content_sha256: extensions_artifact_hash(extension),
                    sdk_extension_hash: extension.sdk_extension_hash.clone(),
                    sdk_output_tree_hash: extension.sdk_output_tree_hash.clone(),
                },
            );
        }
    }
    manifest.artifacts.sort();
    manifest.artifacts.dedup();
    Ok(manifest)
}

/// Shared provenance builder for TypeScript and Rust outputs. `generated`
/// lists the generated artifact names relative to the provenance manifest;
/// staged extension files (and `extensions.json`) are appended under
/// `extension_file_prefix` (`"src/"` for Rust crate mode, empty otherwise).
fn build_sdk_provenance_manifest_from_artifacts(
    generated: BTreeSet<String>,
    extension_file_prefix: &str,
    input_pin: &ResolvedExtensionsInputPin,
    extensions: Option<&ResolvedExtensionsArtifact>,
) -> Result<SdkProvenanceManifestV2> {
    let mut artifacts = generated;
    if let Some(artifact) = extensions {
        artifacts.insert(format!("{extension_file_prefix}extensions.json"));
        for file in &artifact.files {
            artifacts.insert(format!(
                "{extension_file_prefix}{}",
                normalize_extension_relative_path(&file.path)?
            ));
        }
    }

    validate_provenance_input_pin(input_pin)?;

    Ok(SdkProvenanceManifestV2 {
        schema_version: 2,
        input: SdkProvenanceInputV2 {
            kind: input_pin.kind,
            hash: input_pin.hash.clone(),
        },
        generator: SdkProvenanceGeneratorV2 {
            name: env!("CARGO_PKG_NAME").to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            compiler_hash: sdk_compiler_hash()?.to_string(),
        },
        sdk_output_tree_hash: None,
        extensions: extensions.map(|artifact| SdkProvenanceExtensionsV2 {
            content_sha256: extensions_artifact_hash(artifact),
            sdk_extension_hash: artifact.sdk_extension_hash.clone(),
            sdk_output_tree_hash: artifact.sdk_output_tree_hash.clone(),
        }),
        program_extensions: BTreeMap::new(),
        artifacts: artifacts.into_iter().collect(),
    })
}

fn validate_provenance_input_pin(input_pin: &ResolvedExtensionsInputPin) -> Result<()> {
    let expected = match input_pin.kind {
        ExtensionsInputKind::StackManifest => arete_hash::HashKindName::StackManifest,
        ExtensionsInputKind::ProgramSpec => arete_hash::HashKindName::ProgramSpec,
        ExtensionsInputKind::StackAst | ExtensionsInputKind::ProgramIdl => {
            anyhow::bail!(
                "SDK provenance V2 requires StackManifest or ProgramSpec input, not {}",
                input_pin.kind.as_manifest_value()
            )
        }
    };
    let actual = input_pin
        .hash
        .parse::<arete_hash::AnyHashId>()
        .context("SDK provenance V2 input must be a typed Arete hash")?
        .kind();
    if actual != expected {
        anyhow::bail!(
            "SDK provenance input kind mismatch: expected {}, got {}",
            expected,
            actual
        );
    }
    Ok(())
}

#[allow(dead_code)]
fn parse_sdk_provenance_manifest(contents: &str) -> Result<SdkProvenanceManifest> {
    serde_json::from_str(contents).context("Failed to parse SDK provenance manifest")
}

fn write_sdk_provenance_manifest(
    layout: &TypeScriptLayout,
    input_pin: &ResolvedExtensionsInputPin,
    extensions: Option<&ResolvedExtensionsArtifact>,
) -> Result<()> {
    let manifest = build_sdk_provenance_manifest(layout, input_pin, extensions)?;
    write_sdk_provenance_manifest_file(&layout.output_dir, &manifest)
}

fn write_sdk_provenance_manifest_with_program_extensions(
    layout: &TypeScriptLayout,
    input_pin: &ResolvedExtensionsInputPin,
    extensions: Option<&ResolvedExtensionsArtifact>,
    program_modules: &[HostedProgramModule],
) -> Result<()> {
    let manifest = build_sdk_provenance_manifest_with_program_extensions(
        layout,
        input_pin,
        extensions,
        program_modules,
    )?;
    write_sdk_provenance_manifest_file(&layout.output_dir, &manifest)
}

/// Write `sdk-provenance.json` for a Rust or Python output directory.
/// `generated` lists the generated file names relative to `output_dir` (for
/// example `mod.rs` in Rust module mode, `src/lib.rs` in crate mode, or
/// `<module>/__init__.py` in Python package mode).
fn write_language_sdk_provenance_manifest(
    output_dir: &Path,
    generated: BTreeSet<String>,
    extension_file_prefix: &str,
    input_pin: &ResolvedExtensionsInputPin,
    extensions: Option<&ResolvedExtensionsArtifact>,
) -> Result<()> {
    let manifest = build_sdk_provenance_manifest_from_artifacts(
        generated,
        extension_file_prefix,
        input_pin,
        extensions,
    )?;
    write_sdk_provenance_manifest_file(output_dir, &manifest)
}

fn write_sdk_provenance_manifest_file(
    output_dir: &Path,
    manifest: &SdkProvenanceManifestV2,
) -> Result<()> {
    let output = Dir::open_ambient_dir(output_dir, ambient_authority()).with_context(|| {
        format!(
            "Failed to open SDK output directory {}",
            output_dir.display()
        )
    })?;
    // Validate and hash every declared payload file before pruning or writing metadata.
    // Source fingerprints, absolute checkout paths and root metadata are excluded.
    let mut payload = Vec::new();
    for name in &manifest.artifacts {
        arete_hash::validate_artifact_path(name)?;
        if matches!(name.as_str(), SDK_PROVENANCE_FILE | SDK_MANIFEST_FILE) {
            anyhow::bail!("Reserved SDK metadata path in generated payload: {name}");
        }
        let mut prefix = PathBuf::new();
        for component in Path::new(name).components() {
            prefix.push(component);
            if output.symlink_metadata(&prefix)?.file_type().is_symlink() {
                anyhow::bail!("Symlinks are not allowed in SDK payload: {name}");
            }
        }
        payload.push((name.as_str(), output.read(name)?));
    }
    let entries = payload
        .iter()
        .map(|(name, bytes)| arete_hash::ArtifactTreeEntry::file(name, bytes))
        .collect::<Vec<_>>();
    let tree_hash =
        arete_hash::hash_artifact_tree::<arete_hash::SdkOutputTree>(&entries)?.to_string();
    let content_manifest = SdkContentManifestV1 {
        schema_version: 1,
        input: &manifest.input,
        sdk_output_tree_hash: &tree_hash,
        extensions: &manifest.extensions,
        program_extensions: &manifest.program_extensions,
        artifacts: &manifest.artifacts,
    };
    let content = format!("{}\n", serde_json::to_string_pretty(&content_manifest)?);
    let mut manifest = manifest.clone();
    manifest.sdk_output_tree_hash = Some(tree_hash);
    manifest.artifacts.push(SDK_MANIFEST_FILE.to_string());
    manifest.artifacts.sort();
    prune_stale_sdk_artifacts(&output, output_dir, &manifest.artifacts)?;
    output.write(SDK_MANIFEST_FILE, content)?;
    let contents = format!(
        "{}\n",
        serde_json::to_string_pretty(&manifest)
            .context("Failed to serialize SDK provenance manifest")?
    );
    let path = output_dir.join(SDK_PROVENANCE_FILE);
    output
        .write(SDK_PROVENANCE_FILE, contents)
        .with_context(|| {
            format!(
                "Failed to write SDK provenance manifest to {}",
                path.display()
            )
        })
}

fn is_removable_stale_sdk_artifact(output: &Dir, relative: &Path) -> bool {
    output
        .symlink_metadata(relative)
        .is_ok_and(|metadata| metadata.is_file() || metadata.file_type().is_symlink())
}

fn remove_stale_sdk_artifact(output: &Dir, output_dir: &Path, relative: &Path) -> Result<()> {
    output.remove_file(relative).with_context(|| {
        format!(
            "Failed to remove stale generated artifact {}",
            output_dir.join(relative).display()
        )
    })
}

fn prune_stale_sdk_artifacts(
    output: &Dir,
    output_dir: &Path,
    next_artifacts: &[String],
) -> Result<()> {
    let Ok(contents) = output.read_to_string(SDK_PROVENANCE_FILE) else {
        return Ok(());
    };
    let Ok(previous) = parse_sdk_provenance_manifest(&contents) else {
        return Ok(());
    };
    let previous_artifacts = match &previous {
        SdkProvenanceManifest::V1(manifest) => &manifest.artifacts,
        SdkProvenanceManifest::V2(manifest) => &manifest.artifacts,
    };
    let next = next_artifacts
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();

    for stale in previous_artifacts {
        if next.contains(stale.as_str()) {
            continue;
        }
        let Ok(relative) = normalize_extension_relative_path(stale) else {
            continue;
        };
        let relative = Path::new(&relative);
        if !is_removable_stale_sdk_artifact(output, relative) {
            continue;
        }
        remove_stale_sdk_artifact(output, output_dir, relative)?;

        let mut parent = relative.parent();
        while let Some(directory) = parent {
            if directory.as_os_str().is_empty() {
                break;
            }
            if output.remove_dir(directory).is_err() {
                break;
            }
            parent = directory.parent();
        }
    }
    Ok(())
}

fn stack_input_pin(
    source: &ResolvedStackSource,
    _stack_spec: &arete_interpreter::ast::SerializableStackSpec,
) -> Result<ResolvedExtensionsInputPin> {
    Ok(match source {
        ResolvedStackSource::LocalArtifacts(stack) => ResolvedExtensionsInputPin {
            kind: ExtensionsInputKind::StackManifest,
            hash: stack.manifest_hash.clone(),
        },
        ResolvedStackSource::Remote(stack) => ResolvedExtensionsInputPin {
            kind: ExtensionsInputKind::StackManifest,
            hash: stack.manifest_hash.clone(),
        },
    })
}

fn validate_extensions_input_pin(
    artifact: &ResolvedExtensionsArtifact,
    input_pin: &ResolvedExtensionsInputPin,
) -> Vec<String> {
    let mut errors = Vec::new();

    match (artifact.input_kind, artifact.input_hash.as_deref()) {
        (None, None) => {}
        (Some(_), None) | (None, Some(_)) => errors.push(
            "extensions input pin is incomplete: inputKind and inputHash must be set together"
                .to_string(),
        ),
        (Some(manifest_kind), Some(manifest_hash)) => {
            if manifest_kind != input_pin.kind {
                errors.push(format!(
                    "extensions input kind mismatch: manifest={}, generated={}",
                    manifest_kind.as_manifest_value(),
                    input_pin.kind.as_manifest_value()
                ));
            } else if manifest_hash != input_pin.hash {
                errors.push(format!(
                    "extensions input hash mismatch: manifest={}, generated={}",
                    manifest_hash, input_pin.hash
                ));
            }
        }
    }

    errors
}

fn resolved_extensions_artifact_from_registry(
    artifact: &RegistrySdkExtensionArtifact,
) -> Result<ResolvedExtensionsArtifact> {
    let files = artifact
        .files
        .iter()
        .map(|(path, contents)| {
            Ok(ResolvedExtensionsFile {
                path: normalize_extension_relative_path(path)?,
                contents: contents.clone(),
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let mut resolved = build_extensions_artifact(
        artifact.manifest.entry.clone(),
        files,
        artifact
            .manifest
            .input_kind
            .clone()
            .map(ExtensionsInputKind::from_registry),
        artifact.manifest.input_hash.clone(),
        artifact.manifest.sdk_range.clone(),
        artifact.manifest.language.clone(),
    )?;
    resolved.sdk_extension_hash = artifact.sdk_extension_hash.clone();
    resolved.sdk_output_tree_hash = artifact.sdk_output_tree_hash.clone();
    Ok(resolved)
}

fn typescript_program_config_from_registry(
    install: &RegistryProgramInstallResponse,
) -> Result<arete_interpreter::typescript::TypeScriptProgramConfig> {
    let RegistryProgramInstallTransport::HostedBinding { binding } = &install.transport;
    let target_kind = binding
        .auth
        .get("targetKind")
        .and_then(serde_json::Value::as_str);
    let session_endpoint = binding
        .auth
        .get("sessionEndpoint")
        .and_then(serde_json::Value::as_str);
    let target_id = binding
        .auth
        .get("targetId")
        .and_then(serde_json::Value::as_str);
    let endpoint = url::Url::parse(&binding.endpoint).ok();
    let session_url = session_endpoint.and_then(|value| url::Url::parse(value).ok());
    let secure_or_loopback = |url: &url::Url| {
        url.scheme() == "https"
            || (url.scheme() == "http"
                && matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "::1")))
    };
    if binding.endpoint.trim().is_empty()
        || binding
            .program_read_binding_id
            .parse::<arete_hash::ProgramReadBindingId>()
            .is_err()
        || target_kind != Some("program-read-binding")
        || target_id != Some(binding.program_read_binding_id.as_str())
        || session_endpoint.is_none_or(|value| value.trim().is_empty())
        || endpoint
            .as_ref()
            .is_none_or(|value| !secure_or_loopback(value))
        || session_url
            .as_ref()
            .is_none_or(|value| !secure_or_loopback(value))
    {
        anyhow::bail!(
            "Program {} returned an incomplete hosted-binding transport",
            install.install_name
        );
    }
    Ok(arete_interpreter::typescript::TypeScriptProgramConfig {
        definition: arete_interpreter::typescript::TypeScriptProgramDefinitionMetadata {
            program_id: install.definition.program_id.clone(),
            sdk_definition_hash: None,
            program_spec_hash: install.definition.program_spec_hash.clone(),
            idl_content_hash: install.definition.idl_content_hash.clone(),
            normalized_idl_hash: install.definition.normalized_idl_hash.clone(),
        },
        release: arete_interpreter::typescript::TypeScriptProgramReleaseReference {
            program_release_hash: install.release.program_release_hash.clone(),
            program_spec_hash: install.release.program_spec_hash.clone(),
        },
        transport: arete_interpreter::typescript::TypeScriptProgramReadTransport::HostedBinding(
            arete_interpreter::typescript::TypeScriptProgramReadBinding {
                endpoint: binding.endpoint.clone(),
                program_read_binding_id: binding.program_read_binding_id.clone(),
                auth: binding.auth.clone(),
            },
        ),
        gateway: optional_gateway_descriptor(
            install.chain_binding.as_ref(),
            install.transaction_binding.as_ref(),
            &format!("hosted program '{}'", install.install_name),
        )?,
    })
}

fn managed_gateway_descriptor(
    chain: Option<&RegistryCapabilityInstallBinding>,
    transactions: Option<&RegistryCapabilityInstallBinding>,
    source: &str,
) -> Result<serde_json::Value> {
    optional_gateway_descriptor(chain, transactions, source)?.ok_or_else(|| {
        anyhow::anyhow!(
            "{source} omitted managed Solana gateway bindings; refusing tenant HTTP fallback"
        )
    })
}

fn optional_gateway_descriptor(
    chain: Option<&RegistryCapabilityInstallBinding>,
    transactions: Option<&RegistryCapabilityInstallBinding>,
    source: &str,
) -> Result<Option<serde_json::Value>> {
    match (chain, transactions) {
        (Some(chain), Some(transactions)) => Ok(Some(serde_json::json!({
            "chain": chain,
            "transactions": transactions,
        }))),
        (None, None) => Ok(None),
        _ => anyhow::bail!("{source} returned only one managed Solana gateway capability binding"),
    }
}

fn program_spec_artifact_from_registry(
    install: &RegistryProgramInstallResponse,
) -> Result<arete_artifacts::ProgramSpecArtifact> {
    let artifact: arete_artifacts::ProgramSpecArtifact =
        serde_json::from_value(install.definition.program_spec.clone()).with_context(|| {
            format!(
                "Program {} returned an invalid ProgramSpec",
                install.install_name
            )
        })?;
    artifact.validate().with_context(|| {
        format!(
            "Program {} returned an invalid ProgramSpec",
            install.install_name
        )
    })?;
    if artifact.artifact_hash.to_string() != install.definition.program_spec_hash
        || install.release.program_spec_hash != install.definition.program_spec_hash
        || artifact.payload.program_id != install.definition.program_id
        || artifact.payload.idl_content_hash.to_string() != install.definition.idl_content_hash
        || artifact.payload.normalized_idl_hash.to_string()
            != install.definition.normalized_idl_hash
    {
        anyhow::bail!(
            "Program {} descriptor does not match its ProgramSpec",
            install.install_name
        );
    }
    Ok(artifact)
}

fn hosted_program_modules(
    source: &ResolvedStackSource,
    stack_spec: &arete_interpreter::ast::SerializableStackSpec,
) -> Result<Vec<HostedProgramModule>> {
    let ResolvedStackSource::Remote(remote) = source else {
        return Ok(Vec::new());
    };
    if remote.programs.len() != stack_spec.idls.len() {
        return Err(anyhow::anyhow!(
            "Hosted program descriptor count mismatch: expected {}, received {}",
            stack_spec.idls.len(),
            remote.programs.len()
        ));
    }

    let mut hosted = Vec::new();
    let mut program_keys = BTreeSet::new();
    for install in &remote.programs {
        let index = stack_spec
            .program_ids
            .iter()
            .position(|program_id| program_id == &install.definition.program_id)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Hosted program '{}' is not present in the generated stack",
                    install.install_name
                )
            })?;
        let idl = &stack_spec.idls[index];
        let program_spec_hash = stack_spec.program_specs[index]
            .hash()
            .map_err(anyhow::Error::msg)?
            .to_string();
        if program_spec_hash != install.definition.program_spec_hash {
            anyhow::bail!(
                "Hosted program '{}' does not match the stack ProgramSpec",
                install.install_name
            );
        }
        let program_key = to_camel_case(&idl.name);
        if !program_keys.insert(program_key.clone()) {
            anyhow::bail!(
                "Hosted programs use duplicate generated key '{}'",
                program_key
            );
        }
        hosted.push(HostedProgramModule {
            import_name: format!("hosted{}Program", to_pascal_case(&program_key)),
            program_const_name: to_screaming_snake_case(&idl.name),
            program_key,
            input_pin: ResolvedExtensionsInputPin {
                kind: ExtensionsInputKind::ProgramSpec,
                hash: install.definition.program_spec_hash.clone(),
            },
            extension: install
                .definition
                .extensions
                .as_ref()
                .map(resolved_extensions_artifact_from_registry)
                .transpose()?,
            program_spec: program_spec_artifact_from_registry(install)?,
            program_config: typescript_program_config_from_registry(install)?,
        });
    }
    Ok(hosted)
}

const HOSTED_PROGRAM_ENTRY: &str = "__arete-program.ts";

fn hosted_program_directory(program: &HostedProgramModule) -> PathBuf {
    PathBuf::from("programs").join(to_kebab_case(&program.program_key))
}

fn hosted_program_core_name(program: &HostedProgramModule) -> String {
    format!("{}-core.ts", to_kebab_case(&program.program_key))
}

fn hosted_program_entry_import(program: &HostedProgramModule) -> String {
    format!(
        "./{}/{}",
        hosted_program_directory(program).to_string_lossy(),
        HOSTED_PROGRAM_ENTRY.trim_end_matches(".ts")
    )
}

fn extension_core_import_paths(extension: &ResolvedExtensionsArtifact) -> Result<BTreeSet<String>> {
    let import_regex =
        Regex::new(r#"from\s+['\"]\./([^'\"]*?(?:-core|core))(?:\.(?:js|ts))?['\"]"#)
            .expect("program core import regex should compile");
    let mut core_paths = BTreeSet::new();
    for file in &extension.files {
        let source_parent = Path::new(&file.path).parent().unwrap_or(Path::new(""));
        for captures in import_regex.captures_iter(&file.contents) {
            let relative = source_parent.join(format!("{}.ts", &captures[1]));
            core_paths.insert(normalize_extension_relative_path(
                &relative.to_string_lossy(),
            )?);
        }
    }
    Ok(core_paths)
}

fn hosted_program_core_paths(program: &HostedProgramModule) -> Result<BTreeSet<String>> {
    let mut core_paths = BTreeSet::from([hosted_program_core_name(program)]);
    if let Some(extension) = &program.extension {
        core_paths.extend(extension_core_import_paths(extension)?);
    }
    Ok(core_paths)
}

fn typescript_core_paths(
    layout: &TypeScriptLayout,
    extension: Option<&ResolvedExtensionsArtifact>,
) -> Result<BTreeSet<String>> {
    let mut core_paths = BTreeSet::from([generated_artifact_name(&layout.core_path)?]);
    if let Some(extension) = extension {
        core_paths.extend(extension_core_import_paths(extension)?);
    }
    Ok(core_paths)
}

fn write_typescript_core_modules(
    layout: &TypeScriptLayout,
    contents: &str,
    extension: Option<&ResolvedExtensionsArtifact>,
) -> Result<()> {
    let core_paths = typescript_core_paths(layout, extension)?;
    if let Some(extension) = extension {
        let mut reserved = core_paths.clone();
        reserved.insert(generated_artifact_name(&layout.entry_path)?);
        reserved.insert("extensions.json".to_string());
        reserved.insert(SDK_PROVENANCE_FILE.to_string());
        reserved.insert(SDK_MANIFEST_FILE.to_string());
        if let Some(path) = extension.files.iter().find_map(|file| {
            normalize_extension_relative_path(&file.path)
                .ok()
                .filter(|path| reserved.contains(path))
                .map(|_| file.path.as_str())
        }) {
            anyhow::bail!(
                "TypeScript extension '{}' collides with generated SDK artifact '{}'",
                extension.entry,
                path
            );
        }
    }

    for core_relative in core_paths {
        let core_path = layout.output_dir.join(&core_relative);
        if let Some(parent) = core_path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "Failed to create TypeScript core directory {}",
                    parent.display()
                )
            })?;
        }
        fs::write(&core_path, contents).with_context(|| {
            format!(
                "Failed to write TypeScript core module to {}",
                core_path.display()
            )
        })?;
    }
    Ok(())
}

fn render_hosted_program_entry(program: &HostedProgramModule) -> String {
    let core_stem = hosted_program_core_name(program)
        .trim_end_matches(".ts")
        .to_string();
    let export_name = format!("{}_PROGRAM", program.program_const_name);
    let read_const_name = format!("{}_READ", program.program_const_name);
    if let Some(extension) = &program.extension {
        let extension_entry = extension.entry.trim_end_matches(".ts").to_string();
        return finish_typescript_module(format!(
            r#"import {{ extendProgram, withProgramRead }} from '@usearete/sdk';

import {{ {program_const} as BASE_PROGRAM, {read_const_name} as BASE_PROGRAM_READ }} from './{core_stem}.js';
import programExtensions from './{extension_entry}.js';

export * from './{core_stem}.js';

export const {export_name} = withProgramRead(
  extendProgram(BASE_PROGRAM, programExtensions),
  BASE_PROGRAM_READ,
);

export default {export_name};"#,
            program_const = program.program_const_name,
        ));
    }
    finish_typescript_module(format!(
        r#"import {{ withProgramRead }} from '@usearete/sdk';

import {{ {program_const} as BASE_PROGRAM, {read_const_name} as BASE_PROGRAM_READ }} from './{core_stem}.js';

export * from './{core_stem}.js';

export const {export_name} = withProgramRead(BASE_PROGRAM, BASE_PROGRAM_READ);

export default {export_name};"#,
        program_const = program.program_const_name,
    ))
}

fn stage_hosted_program_modules(
    programs: &[HostedProgramModule],
    layout: &TypeScriptLayout,
    package_name: &str,
) -> Result<()> {
    for program in programs {
        let relative_dir = hosted_program_directory(program);
        let output_dir = layout.output_dir.join(&relative_dir);
        fs::create_dir_all(&output_dir).with_context(|| {
            format!(
                "Failed to create hosted program directory {}",
                output_dir.display()
            )
        })?;
        let core_paths = hosted_program_core_paths(program)?;
        let mut reserved = core_paths
            .iter()
            .cloned()
            .chain(std::iter::once(HOSTED_PROGRAM_ENTRY.to_string()))
            .collect::<BTreeSet<_>>();
        if let Some(extension) = &program.extension {
            reserved.insert("extensions.json".to_string());
            let collision = extension.files.iter().find_map(|file| {
                normalize_extension_relative_path(&file.path)
                    .ok()
                    .filter(|path| reserved.contains(path))
                    .map(|_| file.path.as_str())
            });
            if let Some(path) = collision {
                anyhow::bail!(
                    "Hosted program extension '{}' collides with generated program artifact '{}'",
                    program.program_key,
                    path
                );
            }
            stage_extensions_artifact_with_manifest(
                extension,
                &output_dir,
                &program.input_pin,
                "extensions.json",
            )?;
        }
        let stack_spec = arete_interpreter::public_artifacts::stack_spec_from_program_artifacts(
            to_pascal_case(&program.program_key),
            std::slice::from_ref(&program.program_spec),
        )
        .map_err(anyhow::Error::msg)?;
        let output = arete_interpreter::typescript::compile_program_modules(
            stack_spec,
            Some(arete_interpreter::typescript::TypeScriptStackConfig {
                package_name: package_name.to_string(),
                generate_helpers: false,
                export_const_name: "PROGRAMS".to_string(),
                websocket_url: None,
                http_url: None,
                extension_import: None,
                programs: Some(vec![program.program_config.clone()]),
                gateway: None,
            }),
        )
        .map_err(|error| anyhow::anyhow!("Failed to compile hosted program SDK: {error}"))?;
        let core = output.full_file();
        for core_relative in core_paths {
            let core_path = output_dir.join(&core_relative);
            if let Some(parent) = core_path.parent() {
                fs::create_dir_all(parent).with_context(|| {
                    format!(
                        "Failed to create hosted program core directory {}",
                        parent.display()
                    )
                })?;
            }
            fs::write(&core_path, &core).with_context(|| {
                format!(
                    "Failed to write hosted program core {}",
                    core_path.display()
                )
            })?;
        }
        let entry_path = output_dir.join(HOSTED_PROGRAM_ENTRY);
        fs::write(&entry_path, render_hosted_program_entry(program)).with_context(|| {
            format!(
                "Failed to write hosted program entry {}",
                entry_path.display()
            )
        })?;
    }
    Ok(())
}

fn stage_extensions_artifact(
    artifact: &ResolvedExtensionsArtifact,
    output_dir: &Path,
    input_pin: &ResolvedExtensionsInputPin,
) -> Result<()> {
    stage_extensions_artifact_with_manifest(artifact, output_dir, input_pin, "extensions.json")
}

fn stage_extensions_artifact_with_manifest(
    artifact: &ResolvedExtensionsArtifact,
    output_dir: &Path,
    input_pin: &ResolvedExtensionsInputPin,
    manifest_name: &str,
) -> Result<()> {
    let input_pin_errors = validate_extensions_input_pin(artifact, input_pin);
    if !input_pin_errors.is_empty() {
        return Err(anyhow::anyhow!(
            "Extensions artifact is incompatible with generated input: {}",
            input_pin_errors.join("; ")
        ));
    }

    if let Some(range) = artifact.sdk_range.as_deref() {
        if let Some(current) = discover_usearete_sdk_version(output_dir) {
            if !version_satisfies_range(&current, range) {
                println!(
                    "{} extensions sdkRange mismatch: manifest={}, current={}",
                    "⚠".yellow().bold(),
                    range,
                    current
                );
            }
        }
    }

    write_extensions_artifact_files(artifact, output_dir, manifest_name)
}

/// Stage a Rust devex extensions bundle into the generated module directory.
///
/// Runs the same input-pin validation as the TypeScript pipeline, then
/// requires a flat all-`.rs` file layout (module wiring emits one
/// `pub mod <stem>;` per file, so nested paths cannot be wired). The
/// `sdkRange` check is warning-only best-effort: it compares against the
/// `arete-a4-sdk` dependency version when one is trivially discoverable from
/// a `Cargo.toml` at or above the output directory, and skips silently
/// otherwise.
fn stage_rust_extensions_artifact(
    artifact: &ResolvedExtensionsArtifact,
    output_dir: &Path,
    input_pin: &ResolvedExtensionsInputPin,
) -> Result<()> {
    let input_pin_errors = validate_extensions_input_pin(artifact, input_pin);
    if !input_pin_errors.is_empty() {
        return Err(anyhow::anyhow!(
            "Extensions artifact is incompatible with generated input: {}",
            input_pin_errors.join("; ")
        ));
    }

    for file in &artifact.files {
        let normalized = normalize_extension_relative_path(&file.path)?;
        if !normalized.ends_with(".rs") || normalized.contains('/') {
            return Err(anyhow::anyhow!(
                "Rust extensions bundles require flat .rs files; '{}' is not supported",
                file.path
            ));
        }
    }

    if let Some(range) = artifact.sdk_range.as_deref() {
        if let Some(current) = discover_arete_sdk_crate_version(output_dir) {
            if !version_satisfies_range(&current, range) {
                println!(
                    "{} extensions sdkRange mismatch: manifest={}, current={}",
                    "⚠".yellow().bold(),
                    range,
                    current
                );
            }
        }
    }

    write_extensions_artifact_files(artifact, output_dir, "extensions.json")
}

/// Stage a Python devex extensions bundle into the generated module
/// directory.
///
/// Mirror of [`stage_rust_extensions_artifact`]: same input-pin validation,
/// then a flat all-`.py` file layout requirement (module wiring emits one
/// `from . import <stem>` per file, so nested paths cannot be wired). The
/// `sdkRange` check is warning-only best-effort against an exact `arete-sdk`
/// pin in a `pyproject.toml` at or above the output directory.
fn stage_python_extensions_artifact(
    artifact: &ResolvedExtensionsArtifact,
    output_dir: &Path,
    input_pin: &ResolvedExtensionsInputPin,
) -> Result<()> {
    let input_pin_errors = validate_extensions_input_pin(artifact, input_pin);
    if !input_pin_errors.is_empty() {
        return Err(anyhow::anyhow!(
            "Extensions artifact is incompatible with generated input: {}",
            input_pin_errors.join("; ")
        ));
    }

    for file in &artifact.files {
        let normalized = normalize_extension_relative_path(&file.path)?;
        if !normalized.ends_with(".py") || normalized.contains('/') {
            return Err(anyhow::anyhow!(
                "Python extensions bundles require flat .py files; '{}' is not supported",
                file.path
            ));
        }
    }

    if let Some(range) = artifact.sdk_range.as_deref() {
        if let Some(current) = discover_arete_sdk_python_version(output_dir) {
            if !version_satisfies_range(&current, range) {
                println!(
                    "{} extensions sdkRange mismatch: manifest={}, current={}",
                    "⚠".yellow().bold(),
                    range,
                    current
                );
            }
        }
    }

    write_extensions_artifact_files(artifact, output_dir, "extensions.json")
}

fn write_extensions_artifact_files(
    artifact: &ResolvedExtensionsArtifact,
    output_dir: &Path,
    manifest_name: &str,
) -> Result<()> {
    for file in &artifact.files {
        let relative_path = normalize_extension_relative_path(&file.path)?;
        let destination_path = output_dir.join(&relative_path);
        if let Some(parent) = destination_path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "Failed to create extensions output directory: {}",
                    parent.display()
                )
            })?;
        }

        fs::write(&destination_path, &file.contents).with_context(|| {
            format!(
                "Failed to write extensions artifact file {}",
                destination_path.display()
            )
        })?;
    }

    let manifest_path = output_dir.join(manifest_name);
    let manifest_json = serde_json::to_string_pretty(&artifact.manifest())
        .context("Failed to serialize extensions manifest")?;
    fs::write(&manifest_path, manifest_json).with_context(|| {
        format!(
            "Failed to write extensions manifest to {}",
            manifest_path.display()
        )
    })?;

    Ok(())
}

fn to_screaming_snake_case(input: &str) -> String {
    let mut result = String::new();
    for (index, ch) in input.chars().enumerate() {
        if ch.is_ascii_uppercase() && index > 0 {
            result.push('_');
        }
        result.push(ch.to_ascii_uppercase());
    }
    result
}

fn finish_typescript_module(module: String) -> String {
    if module.ends_with('\n') {
        module
    } else {
        format!("{module}\n")
    }
}

fn render_typescript_stack_entry(
    layout: &TypeScriptLayout,
    stack_name: &str,
    extension_entry: Option<&str>,
    _extension_files: &[&str],
    program_extension_bindings: &[ProgramExtensionBinding],
    hosted_program_modules: &[HostedProgramModule],
) -> String {
    let export_name = format!("{}_STACK", to_screaming_snake_case(stack_name));
    let core_export_name = format!("{}_CORE", export_name);
    let type_name = format!("{}Stack", stack_name);
    let core_import = format!("./{}-core.js", layout.base_name);
    if !hosted_program_modules.is_empty() {
        let mut sdk_imports = Vec::new();
        if !program_extension_bindings.is_empty() {
            sdk_imports.push("extendPrograms");
        }
        if extension_entry.is_some() {
            sdk_imports.push("extendStack");
        }
        let sdk_import = if sdk_imports.is_empty() {
            String::new()
        } else {
            format!(
                "import {{ {} }} from '@usearete/sdk';",
                sdk_imports.join(", ")
            )
        };
        let hosted_imports = hosted_program_modules
            .iter()
            .map(|extension| {
                format!(
                    "import {} from '{}.js';",
                    extension.import_name,
                    hosted_program_entry_import(extension)
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let stack_program_lines = program_extension_bindings
            .iter()
            .map(|binding| format!("    {}: {},", binding.program_key, binding.export_name))
            .collect::<Vec<_>>();
        let hosted_program_lines = hosted_program_modules
            .iter()
            .map(|extension| format!("    {}: {},", extension.program_key, extension.import_name))
            .collect::<Vec<_>>();
        let stack_program_layer = if stack_program_lines.is_empty() {
            String::new()
        } else {
            format!(
                "\nconst EXTENDED_PROGRAMS = extendPrograms(HOSTED_PROGRAMS, {{\n{}\n}});\n",
                stack_program_lines.join("\n")
            )
        };
        let programs_value = if stack_program_lines.is_empty() {
            "HOSTED_PROGRAMS"
        } else {
            "EXTENDED_PROGRAMS"
        };

        let (stack_import, final_value) = if let Some(extension_entry) = extension_entry {
            let extension_import = extension_entry
                .strip_suffix(".ts")
                .unwrap_or(extension_entry);
            let named_imports = program_extension_bindings
                .iter()
                .map(|binding| binding.export_name.as_str())
                .collect::<Vec<_>>();
            let import = if named_imports.is_empty() {
                format!("import stackExtensions from './{extension_import}.js';")
            } else {
                format!(
                    "import stackExtensions, {{ {} }} from './{extension_import}.js';",
                    named_imports.join(", ")
                )
            };
            (import, "extendStack(CORE, stackExtensions)".to_string())
        } else {
            (String::new(), "CORE".to_string())
        };

        return finish_typescript_module(format!(
            r#"{sdk_import}

import {{ {core_export_name} }} from '{core_import}';
{stack_import}
{hosted_imports}

export * from '{core_import}';

const HOSTED_PROGRAMS = {{
  ...{core_export_name}.programs,
{hosted_program_lines}
}} as const;{stack_program_layer}

const CORE = {{
  ...{core_export_name},
  programs: {programs_value},
}} as const;

export const {export_name} = {final_value};

export type {type_name} = typeof {export_name};

export default {export_name};"#,
            hosted_program_lines = hosted_program_lines.join("\n"),
        ));
    }

    if let Some(extension_entry) = extension_entry {
        let extension_import = extension_entry
            .strip_suffix(".ts")
            .unwrap_or(extension_entry);
        let extension_runtime_import = format!("{}.js", extension_import);
        if program_extension_bindings.is_empty() {
            finish_typescript_module(format!(
                r#"import {{ extendStack }} from '@usearete/sdk';

import {{ {core_export_name} }} from '{core_import}';
import stackExtensions from './{extension_runtime_import}';

export * from '{core_import}';

export const {export_name} = extendStack(
  {core_export_name},
  stackExtensions
);

export type {type_name} = typeof {export_name};

export default {export_name};"#,
                core_export_name = core_export_name,
                core_import = core_import,
                extension_runtime_import = extension_runtime_import,
                export_name = export_name,
                type_name = type_name,
            ))
        } else {
            let named_imports = program_extension_bindings
                .iter()
                .map(|binding| binding.export_name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            let program_extension_lines = program_extension_bindings
                .iter()
                .map(|binding| format!("    {}: {},", binding.program_key, binding.export_name))
                .collect::<Vec<_>>()
                .join("\n");

            finish_typescript_module(format!(
                r#"import {{ extendPrograms, extendStack }} from '@usearete/sdk';

import {{ {core_export_name} }} from '{core_import}';
import stackExtensions, {{ {named_imports} }} from './{extension_runtime_import}';

export * from '{core_import}';

const CORE = {{
  ...{core_export_name},
  programs: extendPrograms({core_export_name}.programs, {{
{program_extension_lines}
  }}),
}} as const;

export const {export_name} = extendStack(
  CORE,
  stackExtensions
);

export type {type_name} = typeof {export_name};

export default {export_name};"#,
                core_export_name = core_export_name,
                core_import = core_import,
                named_imports = named_imports,
                extension_runtime_import = extension_runtime_import,
                program_extension_lines = program_extension_lines,
                export_name = export_name,
                type_name = type_name,
            ))
        }
    } else {
        finish_typescript_module(format!(
            r#"import {{ {core_export_name} }} from '{core_import}';

export * from '{core_import}';

export const {export_name} = {core_export_name};

export type {type_name} = typeof {export_name};

export default {export_name};"#,
            core_export_name = core_export_name,
            core_import = core_import,
            export_name = export_name,
            type_name = type_name,
        ))
    }
}

fn public_program_export_name(base_name: &str) -> String {
    let screaming = base_name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .to_string();

    if screaming.ends_with("_PROGRAM") {
        screaming
    } else {
        format!("{}_PROGRAM", screaming)
    }
}

fn render_typescript_program_entry(
    layout: &TypeScriptLayout,
    program_name: &str,
    extension_entry: Option<&str>,
) -> String {
    let core_const_name = to_screaming_snake_case(program_name);
    let export_name = public_program_export_name(&layout.base_name);
    let core_import_name = format!("{}_CORE", export_name);
    let core_read_const_name = format!("{}_READ", core_const_name);
    let read_export_name = format!("{}_READ", export_name);
    let core_read_import_name = format!("{}_CORE", read_export_name);
    let type_name = format!("{}Program", to_pascal_case(&layout.base_name));
    let core_import = format!("./{}-core.js", layout.base_name);

    if let Some(extension_entry) = extension_entry {
        let extension_import = extension_entry
            .strip_suffix(".ts")
            .unwrap_or(extension_entry);
        let extension_runtime_import = format!("{}.js", extension_import);
        finish_typescript_module(format!(
            r#"import {{ extendProgram, withProgramRead }} from '@usearete/sdk';

import {{ {core_const_name} as {core_import_name}, {core_read_const_name} as {core_read_import_name} }} from '{core_import}';
import programExtensions from './{extension_runtime_import}';

export * from '{core_import}';
export {{ {core_const_name} as {core_import_name} }} from '{core_import}';

export const {export_name} = withProgramRead(
  extendProgram({core_import_name}, programExtensions),
  {core_read_import_name},
);
export const {read_export_name} = {core_read_import_name};

export type {type_name} = typeof {export_name};

export default {export_name};"#,
            core_const_name = core_const_name,
            core_import_name = core_import_name,
            core_read_const_name = core_read_const_name,
            core_read_import_name = core_read_import_name,
            read_export_name = read_export_name,
            core_import = core_import,
            extension_runtime_import = extension_runtime_import,
            export_name = export_name,
            type_name = type_name,
        ))
    } else {
        finish_typescript_module(format!(
            r#"import {{ withProgramRead }} from '@usearete/sdk';

import {{ {core_const_name} as {core_import_name}, {core_read_const_name} as {core_read_import_name} }} from '{core_import}';

export * from '{core_import}';
export {{ {core_const_name} as {core_import_name} }} from '{core_import}';

export const {export_name} = withProgramRead({core_import_name}, {core_read_import_name});
export const {read_export_name} = {core_read_import_name};

export type {type_name} = typeof {export_name};

export default {export_name};"#,
            core_const_name = core_const_name,
            core_import_name = core_import_name,
            core_read_const_name = core_read_const_name,
            core_read_import_name = core_read_import_name,
            read_export_name = read_export_name,
            core_import = core_import,
            export_name = export_name,
            type_name = type_name,
        ))
    }
}

fn render_typescript_program_collection_entry(
    layout: &TypeScriptLayout,
    stack_name: &str,
    extension_entry: Option<&str>,
    hosted_program_modules: &[HostedProgramModule],
) -> String {
    let export_name = format!("{}_PROGRAMS", to_screaming_snake_case(stack_name));
    let core_export_name = format!("{}_CORE", export_name);
    let type_name = format!("{}Programs", stack_name);
    let core_import = format!("./{}-core.js", layout.base_name);

    if !hosted_program_modules.is_empty() {
        let sdk_import = if extension_entry.is_some() {
            "import { extendPrograms } from '@usearete/sdk';\n\n"
        } else {
            ""
        };
        let hosted_imports = hosted_program_modules
            .iter()
            .map(|extension| {
                format!(
                    "import {} from '{}.js';",
                    extension.import_name,
                    hosted_program_entry_import(extension)
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let hosted_lines = hosted_program_modules
            .iter()
            .map(|extension| format!("  {}: {},", extension.program_key, extension.import_name))
            .collect::<Vec<_>>()
            .join("\n");
        let (explicit_import, base_expression) = extension_entry
            .map(|entry| entry.strip_suffix(".ts").unwrap_or(entry))
            .map(|entry| {
                (
                    format!("import programExtensions from './{entry}.js';"),
                    format!("extendPrograms({core_export_name}, programExtensions)"),
                )
            })
            .unwrap_or_else(|| (String::new(), core_export_name.clone()));

        return finish_typescript_module(format!(
            r#"{sdk_import}import {{ {export_name} as {core_export_name} }} from '{core_import}';
{explicit_import}
{hosted_imports}

export * from '{core_import}';

const BASE_PROGRAMS = {base_expression};

export const {export_name} = {{
  ...BASE_PROGRAMS,
{hosted_lines}
}} as const;

export type {type_name} = typeof {export_name};

export default {export_name};"#,
        ));
    }

    if let Some(extension_entry) = extension_entry {
        let extension_import = extension_entry
            .strip_suffix(".ts")
            .unwrap_or(extension_entry);
        let extension_runtime_import = format!("{}.js", extension_import);
        finish_typescript_module(format!(
            r#"import {{ extendPrograms }} from '@usearete/sdk';

import {{ {export_name} as {core_export_name} }} from '{core_import}';
import programExtensions from './{extension_runtime_import}';

export * from '{core_import}';

export const {export_name} = extendPrograms({core_export_name}, programExtensions);

export type {type_name} = typeof {export_name};

export default {export_name};"#,
            export_name = export_name,
            core_export_name = core_export_name,
            core_import = core_import,
            extension_runtime_import = extension_runtime_import,
            type_name = type_name,
        ))
    } else {
        finish_typescript_module(format!(
            r#"import {{ {export_name} as {core_export_name} }} from '{core_import}';

export * from '{core_import}';

export const {export_name} = {core_export_name};

export type {type_name} = typeof {export_name};

export default {export_name};"#,
            export_name = export_name,
            core_export_name = core_export_name,
            core_import = core_import,
            type_name = type_name,
        ))
    }
}

fn generate_typescript_program_sdk_from_idl(
    idl_path: &Path,
    output_path: &Path,
    package_name: &str,
    extensions_path: Option<&Path>,
) -> Result<()> {
    let idl_bytes =
        fs::read(idl_path).with_context(|| format!("Failed to read IDL {}", idl_path.display()))?;
    let identity = arete_interpreter::program_sdk::build_oss_program_identity_v1_from_idl_bytes(
        &idl_bytes, None,
    )
    .map_err(|e| anyhow::anyhow!("Failed to parse IDL {}: {}", idl_path.display(), e))?;
    let sdk_name = idl_sdk_name_from_path(idl_path)?;
    let stack_name = to_pascal_case(&sdk_name);
    let program_name = identity.program_spec.idl_snapshot.snapshot.name.clone();
    let input_pin = ResolvedExtensionsInputPin {
        kind: ExtensionsInputKind::ProgramSpec,
        hash: identity.program_spec_hash.to_string(),
    };
    let stack_spec = arete_interpreter::program_sdk::build_program_only_stack_spec_from_identity(
        &identity,
        &stack_name,
    );
    let program = arete_interpreter::typescript::TypeScriptProgramConfig::from(&identity);

    write_typescript_program_sdk(
        &sdk_name,
        &program_name,
        stack_spec,
        output_path,
        package_name,
        TypeScriptProgramSdkExtensions {
            input_pin: &input_pin,
            programs: Some(vec![program]),
            gateway: None,
            path: extensions_path,
            hosted_artifact: None,
        },
    )
}

fn generate_typescript_program_sdk_from_artifact(
    program_spec: &arete_artifacts::ProgramSpecArtifact,
    sdk_name: &str,
    output_path: &Path,
    package_name: &str,
    extensions_path: Option<&Path>,
) -> Result<()> {
    let identity = arete_hash::OssProgramIdentityV1::new(program_spec.payload.clone())
        .map_err(anyhow::Error::msg)?;
    let stack_name = to_pascal_case(sdk_name);
    let program_name = program_spec.payload.idl_snapshot.snapshot.name.clone();
    let stack_spec = arete_interpreter::public_artifacts::stack_spec_from_program_artifacts(
        &stack_name,
        std::slice::from_ref(program_spec),
    )
    .map_err(anyhow::Error::msg)?;
    let program = arete_interpreter::typescript::TypeScriptProgramConfig::from(&identity);
    let input_pin = ResolvedExtensionsInputPin {
        kind: ExtensionsInputKind::ProgramSpec,
        hash: program_spec.artifact_hash.to_string(),
    };

    write_typescript_program_sdk(
        sdk_name,
        &program_name,
        stack_spec,
        output_path,
        package_name,
        TypeScriptProgramSdkExtensions {
            input_pin: &input_pin,
            programs: Some(vec![program]),
            gateway: None,
            path: extensions_path,
            hosted_artifact: None,
        },
    )
}

struct TypeScriptProgramSdkExtensions<'a> {
    input_pin: &'a ResolvedExtensionsInputPin,
    programs: Option<Vec<arete_interpreter::typescript::TypeScriptProgramConfig>>,
    gateway: Option<serde_json::Value>,
    path: Option<&'a Path>,
    hosted_artifact: Option<&'a ResolvedExtensionsArtifact>,
}

fn write_typescript_program_sdk(
    sdk_name: &str,
    program_name: &str,
    stack_spec: arete_interpreter::ast::SerializableStackSpec,
    output_path: &Path,
    package_name: &str,
    extensions: TypeScriptProgramSdkExtensions<'_>,
) -> Result<()> {
    let output = arete_interpreter::typescript::compile_program_modules(
        stack_spec,
        Some(arete_interpreter::typescript::TypeScriptStackConfig {
            package_name: package_name.to_string(),
            generate_helpers: false,
            export_const_name: "PROGRAMS".to_string(),
            websocket_url: None,
            http_url: None,
            extension_import: None,
            programs: extensions.programs,
            gateway: extensions.gateway,
        }),
    )
    .map_err(|e| anyhow::anyhow!("Failed to compile TypeScript: {}", e))?;

    for warning in &output.warnings {
        println!("{} {}", "⚠".yellow().bold(), warning);
    }
    print_pda_degradation_summary(&output.pda_degradations);

    let layout = resolve_typescript_layout(output_path, sdk_name);
    fs::create_dir_all(&layout.output_dir).with_context(|| {
        format!(
            "Failed to create TypeScript output directory: {}",
            layout.output_dir.display()
        )
    })?;

    let artifact = resolve_extensions_artifact(
        extensions.path,
        &layout,
        extensions.hosted_artifact,
        OutputExtensionsFallback::Ignore,
    )?;
    write_typescript_core_modules(&layout, &output.full_file(), artifact.as_ref())?;
    if let Some(ref artifact) = artifact {
        stage_extensions_artifact(artifact, &layout.output_dir, extensions.input_pin)?;
    }

    let entry_contents = render_typescript_program_entry(
        &layout,
        program_name,
        artifact.as_ref().map(|artifact| artifact.entry.as_str()),
    );
    fs::write(&layout.entry_path, entry_contents).with_context(|| {
        format!(
            "Failed to write TypeScript entry module to {}",
            layout.entry_path.display()
        )
    })?;
    write_sdk_provenance_manifest(&layout, extensions.input_pin, artifact.as_ref())?;

    Ok(())
}

fn generate_typescript_program_sdk_from_install(
    install: &RegistryProgramInstallResponse,
    sdk_name: &str,
    output_path: &Path,
    package_name: &str,
    extensions_path: Option<&Path>,
    hosted_artifact: Option<&ResolvedExtensionsArtifact>,
) -> Result<()> {
    let program_spec = program_spec_artifact_from_registry(install)?;
    let program_name = program_spec.payload.idl_snapshot.snapshot.name.clone();
    let stack_name = to_pascal_case(sdk_name);
    let input_pin = ResolvedExtensionsInputPin {
        kind: ExtensionsInputKind::ProgramSpec,
        hash: install.definition.program_spec_hash.clone(),
    };
    let stack_spec = arete_interpreter::public_artifacts::stack_spec_from_program_artifacts(
        &stack_name,
        &[program_spec],
    )
    .map_err(anyhow::Error::msg)?;

    write_typescript_program_sdk(
        sdk_name,
        &program_name,
        stack_spec,
        output_path,
        package_name,
        TypeScriptProgramSdkExtensions {
            input_pin: &input_pin,
            programs: Some(vec![typescript_program_config_from_registry(install)?]),
            gateway: Some(managed_gateway_descriptor(
                install.chain_binding.as_ref(),
                install.transaction_binding.as_ref(),
                &format!("hosted program '{}'", install.install_name),
            )?),
            path: extensions_path,
            hosted_artifact,
        },
    )
}

#[allow(clippy::too_many_arguments)]
fn generate_typescript_sdk_from_source(
    source: &ResolvedStackSource,
    output_path: &Path,
    package_name: &str,
    websocket_url: Option<String>,
    http_url: Option<String>,
    extensions_path: Option<&Path>,
    live_module_imports: &BTreeMap<String, String>,
    program_module_imports: &BTreeMap<String, String>,
    program_only: bool,
) -> Result<()> {
    if let Some(composition) = source.composition_artifacts() {
        if !program_only {
            return generate_typescript_composition_sdk(
                source,
                composition.program_specs,
                composition.live_specs,
                composition.stack_manifest,
                output_path,
                package_name,
                websocket_url,
                http_url,
                extensions_path,
                live_module_imports,
                program_module_imports,
            );
        }
    }
    if !live_module_imports.is_empty() || !program_module_imports.is_empty() {
        anyhow::bail!("--live-module and --program-module require a multi-live StackManifest");
    }
    let stack_spec = source.load_stack_spec(!program_only)?;
    let input_pin = stack_input_pin(source, &stack_spec)?;
    let hosted_program_modules = hosted_program_modules(source, &stack_spec)?;

    if program_only {
        let stack_name = stack_spec.stack_name.clone();
        let program_count = stack_spec.idls.len();
        println!(
            "{} {} program(s), views skipped (--program-only)",
            "→".blue().bold(),
            program_count,
        );
        for idl in &stack_spec.idls {
            println!("   Program: {}", idl.name);
        }

        println!(
            "{} Compiling TypeScript program modules...",
            "→".blue().bold()
        );

        let config = arete_interpreter::typescript::TypeScriptStackConfig {
            package_name: package_name.to_string(),
            generate_helpers: false,
            export_const_name: "PROGRAMS".to_string(),
            websocket_url,
            http_url,
            extension_import: None,
            programs: source.typescript_programs(&stack_spec)?,
            gateway: source.hosted_gateway()?,
        };

        let output = arete_interpreter::typescript::compile_program_modules(
            stack_spec.clone(),
            Some(config),
        )
        .map_err(|e| anyhow::anyhow!("Failed to compile TypeScript: {}", e))?;

        for warning in &output.warnings {
            println!("{} {}", "⚠".yellow().bold(), warning);
        }
        print_pda_degradation_summary(&output.pda_degradations);

        let layout =
            resolve_typescript_layout(output_path, &format!("{}-programs", source.sdk_name()));
        fs::create_dir_all(&layout.output_dir).with_context(|| {
            format!(
                "Failed to create TypeScript output directory: {}",
                layout.output_dir.display()
            )
        })?;

        let artifact = resolve_extensions_artifact(
            extensions_path,
            &layout,
            None,
            source.output_extensions_fallback(),
        )?;
        write_typescript_core_modules(&layout, &output.full_file(), artifact.as_ref())?;
        if let Some(ref artifact) = artifact {
            stage_extensions_artifact(artifact, &layout.output_dir, &input_pin)?;
        }
        stage_hosted_program_modules(&hosted_program_modules, &layout, package_name)?;

        let entry_contents = render_typescript_program_collection_entry(
            &layout,
            &stack_name,
            artifact.as_ref().map(|artifact| artifact.entry.as_str()),
            &hosted_program_modules,
        );

        fs::write(&layout.entry_path, entry_contents).with_context(|| {
            format!(
                "Failed to write TypeScript entry module to {}",
                layout.entry_path.display()
            )
        })?;
        write_sdk_provenance_manifest_with_program_extensions(
            &layout,
            &input_pin,
            artifact.as_ref(),
            &hosted_program_modules,
        )?;
    } else {
        let entity_count = stack_spec.entities.len();
        let total_views: usize = stack_spec.entities.iter().map(|e| e.views.len()).sum();

        println!(
            "{} {} entities, {} views total",
            "→".blue().bold(),
            entity_count,
            total_views,
        );
        for entity in &stack_spec.entities {
            let view_ids: Vec<&str> = entity.views.iter().map(|v| v.id.as_str()).collect();
            println!(
                "   Entity: {} (views: {})",
                entity.state_name,
                view_ids.join(", ")
            );
        }

        println!("{} Compiling TypeScript from stack...", "→".blue().bold());

        let stack_name = stack_spec.stack_name.clone();
        let config = arete_interpreter::typescript::TypeScriptStackConfig {
            package_name: package_name.to_string(),
            generate_helpers: true,
            export_const_name: "STACK".to_string(),
            websocket_url,
            http_url,
            extension_import: None,
            programs: source.typescript_programs(&stack_spec)?,
            gateway: source.hosted_gateway()?,
        };

        let output = match source {
            ResolvedStackSource::LocalArtifacts(_) => {
                arete_interpreter::typescript::compile_stack_spec_with_exact_views(
                    stack_spec.clone(),
                    Some(config),
                )
            }
            ResolvedStackSource::Remote(stack) if stack.exact_views => {
                arete_interpreter::typescript::compile_stack_spec_with_exact_views(
                    stack_spec.clone(),
                    Some(config),
                )
            }
            _ => {
                arete_interpreter::typescript::compile_stack_spec(stack_spec.clone(), Some(config))
            }
        }
        .map_err(|e| anyhow::anyhow!("Failed to compile TypeScript: {}", e))?;

        for warning in &output.warnings {
            println!("{} {}", "⚠".yellow().bold(), warning);
        }
        print_pda_degradation_summary(&output.pda_degradations);

        let layout = resolve_typescript_layout(output_path, source.sdk_name());
        fs::create_dir_all(&layout.output_dir).with_context(|| {
            format!(
                "Failed to create TypeScript output directory: {}",
                layout.output_dir.display()
            )
        })?;

        let artifact = resolve_extensions_artifact(
            extensions_path,
            &layout,
            if program_only {
                None
            } else {
                source.hosted_extensions()
            },
            source.output_extensions_fallback(),
        )?;
        write_typescript_core_modules(&layout, &output.full_file(), artifact.as_ref())?;
        if let Some(ref artifact) = artifact {
            stage_extensions_artifact(artifact, &layout.output_dir, &input_pin)?;
        }
        stage_hosted_program_modules(&hosted_program_modules, &layout, package_name)?;
        let extension_files = artifact
            .as_ref()
            .map(|artifact| {
                artifact
                    .files
                    .iter()
                    .map(|file| file.path.as_str())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let entry_contents = render_typescript_stack_entry(
            &layout,
            &stack_name,
            artifact.as_ref().map(|artifact| artifact.entry.as_str()),
            &extension_files,
            artifact
                .as_ref()
                .map(|artifact| artifact.program_extension_bindings.as_slice())
                .unwrap_or(&[]),
            &hosted_program_modules,
        );
        fs::write(&layout.entry_path, entry_contents).with_context(|| {
            format!(
                "Failed to write TypeScript entry module to {}",
                layout.entry_path.display()
            )
        })?;
        write_sdk_provenance_manifest_with_program_extensions(
            &layout,
            &input_pin,
            artifact.as_ref(),
            &hosted_program_modules,
        )?;
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn generate_typescript_composition_sdk(
    source: &ResolvedStackSource,
    program_specs: &[arete_artifacts::ProgramSpecArtifact],
    live_specs: &[(String, arete_artifacts::LiveSpecArtifactV2)],
    stack_manifest: &arete_artifacts::StackManifestArtifactV2,
    output_path: &Path,
    package_name: &str,
    websocket_url: Option<String>,
    http_url: Option<String>,
    extensions_path: Option<&Path>,
    live_module_imports: &BTreeMap<String, String>,
    program_module_imports: &BTreeMap<String, String>,
) -> Result<()> {
    if websocket_url.is_some() || http_url.is_some() {
        anyhow::bail!(
            "multi-live generation requires per-alias endpoint configuration; a shared --url is not allowed"
        );
    }
    if extensions_path.is_some() || source.hosted_extensions().is_some() {
        anyhow::bail!(
            "multi-live extensions require a composition-wrapper extension contract; shared stack extensions are not supported"
        );
    }
    let program_stack = arete_interpreter::public_artifacts::stack_spec_from_program_artifacts(
        &stack_manifest.payload.name,
        program_specs,
    )
    .map_err(anyhow::Error::msg)?;
    let config = arete_interpreter::typescript::TypeScriptCompositionConfig {
        stack: arete_interpreter::typescript::TypeScriptStackConfig {
            package_name: package_name.to_string(),
            generate_helpers: true,
            export_const_name: "STACK".to_string(),
            websocket_url: None,
            http_url: None,
            extension_import: None,
            programs: source.typescript_programs(&program_stack)?,
            gateway: source.hosted_gateway()?,
        },
        live_endpoints: source.composition_live_endpoints(),
        live_module_imports: live_module_imports.clone(),
        program_module_imports: program_module_imports.clone(),
    };
    let output = arete_interpreter::typescript::compile_composed_public_artifacts_v2(
        program_specs,
        live_specs,
        stack_manifest,
        Some(config),
    )
    .map_err(|error| anyhow::anyhow!("Failed to compile TypeScript composition: {error}"))?;
    let layout = resolve_typescript_layout(output_path, source.sdk_name());
    fs::create_dir_all(&layout.output_dir).with_context(|| {
        format!(
            "Failed to create TypeScript output directory: {}",
            layout.output_dir.display()
        )
    })?;
    if let Some(programs) = &output.program_collection {
        let path = layout
            .output_dir
            .join(format!("{}.ts", programs.module_name));
        fs::write(&path, programs.output.full_file())
            .with_context(|| format!("Failed to write program module {}", path.display()))?;
    }
    for live in &output.live_stacks {
        let path = layout.output_dir.join(format!("{}.ts", live.module_name));
        fs::write(&path, live.output.full_file())
            .with_context(|| format!("Failed to write live module {}", path.display()))?;
    }
    let mut session_definition = output.session_definition.clone();
    if let Some(bindings) = render_hosted_composition_bindings(source, &output.name)? {
        session_definition.push('\n');
        session_definition.push_str(&bindings);
    }
    fs::write(&layout.entry_path, session_definition).with_context(|| {
        format!(
            "Failed to write composition session {}",
            layout.entry_path.display()
        )
    })?;
    for warning in &output.warnings {
        println!("{} {}", "⚠".yellow().bold(), warning);
    }
    print_pda_degradation_summary(&output.pda_degradations);
    println!(
        "{} Generated {} aliased stack modules and session {}",
        "✓".green().bold(),
        output.live_stacks.len(),
        layout.entry_path.display()
    );
    Ok(())
}

fn render_hosted_composition_bindings(
    source: &ResolvedStackSource,
    manifest_name: &str,
) -> Result<Option<String>> {
    let ResolvedStackSource::Remote(stack) = source else {
        return Ok(None);
    };
    let live_specs = stack
        .live_bindings
        .iter()
        .map(|live| {
            serde_json::json!({
                "alias": live.alias,
                "liveSpecHash": live.live_spec_hash,
                "deploymentId": live.binding.deployment_id,
                "websocketEndpoint": live.binding.websocket_endpoint,
                "queryEndpoint": live.binding.query_endpoint,
                "websocketAuthPolicy": live.binding.websocket_auth_policy,
                "queryAuthPolicy": live.binding.query_auth_policy,
                "observedGeneration": live.binding.observed_generation,
            })
        })
        .collect::<Vec<_>>();
    let value = serde_json::json!({
        "stackManifestHash": stack.manifest_hash,
        "liveSpecs": live_specs,
        "chain": stack.chain_binding,
        "transactions": stack.transaction_binding,
    });
    let manifest_pascal = to_pascal_case(manifest_name);
    let bindings_name = format!("{}_HOSTED_BINDINGS", to_screaming_snake_case(manifest_name));
    let mut rendered = format!(
        "export const {bindings_name} = {} as const;\n",
        serde_json::to_string_pretty(&value)
            .context("Failed to serialize hosted composition bindings")?
    );
    if stack.chain_binding.is_some() && stack.transaction_binding.is_some() {
        rendered.push_str(&format!(
            r#"
import {{ createHostedSolanaGatewayTransports }} from '@usearete/sdk';

export type {manifest_pascal}HostedSessionOptions = Omit<
  CompositionSessionOptions<{manifest_pascal}SessionDefinition>,
  'chain' | 'transactions'
>;

export function create{manifest_pascal}HostedSession(
  options: {manifest_pascal}HostedSessionOptions = {{}}
) {{
  const transports = createHostedSolanaGatewayTransports(
    {{
      chain: {bindings_name}.chain,
      transactions: {bindings_name}.transactions,
    }},
    {{ auth: options.auth, fetch: options.fetch }}
  );
  return create{manifest_pascal}Session({{ ...options, ...transports }});
}}
"#
        ));
    }
    Ok(Some(rendered))
}

#[allow(clippy::too_many_arguments)]
pub fn create_rust(
    config_path: &str,
    stack_name: Option<&str>,
    output_override: Option<String>,
    crate_name_override: Option<String>,
    module_flag: bool,
    url_override: Option<String>,
    extensions_override: Option<String>,
    manifest_override: Option<String>,
    artifact_dirs: Vec<String>,
) -> Result<()> {
    let _ = config_path;
    let client = ApiClient::new()?;
    let as_module = module_flag;

    let (source, raw_output_dir, crate_name) = if let Some(manifest_path) = manifest_override {
        let source = ResolvedStackSource::LocalArtifacts(Box::new(load_local_stack_with_roots(
            &manifest_path,
            &artifact_dirs,
        )?));
        let crate_name =
            crate_name_override.unwrap_or_else(|| format!("{}-stack", source.sdk_name()));
        let output = output_override
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(format!("./generated/{}-stack", source.sdk_name())));
        (source, output, crate_name)
    } else {
        let stack_name = stack_name
            .ok_or_else(|| anyhow::anyhow!("stack name is required unless using --manifest"))?;
        println!(
            "{} Looking for stack '{}'...",
            "→".blue().bold(),
            stack_name
        );
        let source =
            resolve_remote_stack_source(&client, stack_name, Some(EXTENSIONS_LANGUAGE_RUST))?;
        let crate_name =
            crate_name_override.unwrap_or_else(|| format!("{}-stack", source.sdk_name()));
        let output = output_override
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(format!("./generated/{}-stack", source.sdk_name())));
        (source, output, crate_name)
    };

    let stack_url = url_override.or_else(|| source.default_websocket_url());
    let output_dir = raw_output_dir;

    println!(
        "{} Found stack: {}",
        "✓".green().bold(),
        source.stack_id().bold()
    );
    source.print_source_details();
    println!("  Output: {}", output_dir.display());
    if as_module {
        println!("  Mode: module (mod.rs)");
    }
    if let Some(url) = &stack_url {
        println!("  URL: {}", url.cyan());
    } else {
        println!(
            "  URL: {}",
            "(not configured - placeholder will be generated)".dimmed()
        );
    }

    println!("\n{} Generating Rust SDK...", "→".blue().bold());

    if let Some(composition) = source.composition_artifacts() {
        if stack_url.is_some() {
            anyhow::bail!(
                "multi-live Rust generation requires per-alias URLs; a shared --url is not allowed"
            );
        }
        if extensions_override.is_some() || source.hosted_extensions().is_some() {
            anyhow::bail!(
                "multi-live extensions require a composition-wrapper extension contract; shared stack extensions are not supported"
            );
        }
        let live_urls = match &source {
            ResolvedStackSource::Remote(stack) => stack
                .live_bindings
                .iter()
                .map(|live| (live.alias.clone(), live.binding.websocket_endpoint.clone()))
                .collect(),
            ResolvedStackSource::LocalArtifacts(_) => BTreeMap::new(),
        };
        let output = arete_interpreter::rust::compile_composed_public_artifacts_v2(
            composition.program_specs,
            composition.live_specs,
            composition.stack_manifest,
            Some(arete_interpreter::rust::RustCompositionConfig {
                stack: arete_interpreter::rust::RustStackConfig {
                    crate_name: crate_name.clone(),
                    sdk_version: arete_interpreter::rust::GENERATED_RUST_SDK_VERSION.to_string(),
                    module_mode: as_module,
                    url: None,
                    http_url: None,
                    extension_modules: Vec::new(),
                    extension_entry: None,
                    program_reads: source.rust_program_reads()?,
                    gateway: source.hosted_gateway()?,
                },
                live_urls,
            }),
        )
        .map_err(|error| anyhow::anyhow!("Failed to compile Rust composition: {error}"))?;
        if as_module {
            arete_interpreter::rust::write_rust_composition_module(&output, &output_dir)
                .with_context(|| {
                    format!(
                        "Failed to write Rust composition to {}",
                        output_dir.display()
                    )
                })?;
        } else {
            arete_interpreter::rust::write_rust_composition_crate(&output, &output_dir)
                .with_context(|| {
                    format!(
                        "Failed to write Rust composition to {}",
                        output_dir.display()
                    )
                })?;
        }
        println!(
            "{} Generated {} aliased Rust stack modules in {}",
            "✓".green().bold(),
            output.live_stacks.len(),
            output_dir.display()
        );
        telemetry::record_sdk_generated("rust");
        return Ok(());
    }

    let stack_spec = source.load_stack_spec(true)?;

    println!(
        "{} {} entities in stack",
        "→".blue().bold(),
        stack_spec.entities.len()
    );

    generate_rust_stack_sdk(
        &source,
        stack_spec,
        &output_dir,
        &crate_name,
        as_module,
        stack_url,
        extensions_override.as_deref().map(Path::new),
    )
}

/// Shared single-live Rust generation: compile the stack, wire and stage the
/// optional devex extensions bundle, and record provenance.
fn generate_rust_stack_sdk(
    source: &ResolvedStackSource,
    stack_spec: arete_interpreter::ast::SerializableStackSpec,
    output_dir: &Path,
    crate_name: &str,
    as_module: bool,
    stack_url: Option<String>,
    extensions_path: Option<&Path>,
) -> Result<()> {
    let input_pin = stack_input_pin(source, &stack_spec)?;
    let module_dir = if as_module {
        output_dir.to_path_buf()
    } else {
        output_dir.join("src")
    };
    let artifact = resolve_rust_extensions_artifact(
        extensions_path,
        source.hosted_extensions(),
        &module_dir,
        source.sdk_name(),
        source.output_extensions_fallback(),
    )?;
    let (extension_modules, extension_entry) = match artifact.as_ref() {
        Some(artifact) => {
            let (modules, entry) = rust_extension_wiring(artifact)?;
            (modules, Some(entry))
        }
        None => (Vec::new(), None),
    };

    let rust_config = arete_interpreter::rust::RustStackConfig {
        crate_name: crate_name.to_string(),
        sdk_version: arete_interpreter::rust::GENERATED_RUST_SDK_VERSION.to_string(),
        module_mode: as_module,
        url: stack_url,
        http_url: source.default_http_url(),
        extension_modules,
        extension_entry,
        program_reads: source.rust_program_reads()?,
        gateway: source.hosted_gateway()?,
    };

    let output = match source {
        ResolvedStackSource::LocalArtifacts(_) => {
            arete_interpreter::rust::compile_stack_spec_with_exact_views(
                stack_spec,
                Some(rust_config),
            )
        }
        ResolvedStackSource::Remote(stack) if stack.exact_views => {
            arete_interpreter::rust::compile_stack_spec_with_exact_views(
                stack_spec,
                Some(rust_config),
            )
        }
        _ => arete_interpreter::rust::compile_stack_spec(stack_spec, Some(rust_config)),
    }
    .map_err(|e| anyhow::anyhow!("Failed to compile Rust: {}", e))?;

    let mut generated = BTreeSet::new();
    if as_module {
        arete_interpreter::rust::write_rust_module(&output, output_dir)
            .with_context(|| format!("Failed to write Rust module to {}", output_dir.display()))?;
        generated.extend([
            "mod.rs".to_string(),
            "types.rs".to_string(),
            "entity.rs".to_string(),
        ]);
        if output.programs_rs.is_some() {
            generated.insert("programs.rs".to_string());
        }
    } else {
        arete_interpreter::rust::write_rust_crate(&output, output_dir)
            .with_context(|| format!("Failed to write Rust crate to {}", output_dir.display()))?;
        generated.extend([
            "Cargo.toml".to_string(),
            "src/lib.rs".to_string(),
            "src/types.rs".to_string(),
            "src/entity.rs".to_string(),
        ]);
        if output.programs_rs.is_some() {
            generated.insert("src/programs.rs".to_string());
        }
    }

    if let Some(ref artifact) = artifact {
        stage_rust_extensions_artifact(artifact, &module_dir, &input_pin)?;
    }
    let extension_file_prefix = if as_module { "" } else { "src/" };
    write_language_sdk_provenance_manifest(
        output_dir,
        generated,
        extension_file_prefix,
        &input_pin,
        artifact.as_ref(),
    )?;

    if as_module {
        println!("{} Successfully generated Rust module!", "✓".green().bold());
        println!("  Module: {}", output_dir.display().to_string().bold());
        if let Some(ref artifact) = artifact {
            println!(
                "  Extensions: {} file(s), entry {}",
                artifact.files.len(),
                artifact.entry
            );
        }
        println!("\n  Add to your lib.rs:");
        let module_name = output_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("module");
        println!("    pub mod {};", module_name.cyan());
    } else {
        println!("{} Successfully generated Rust SDK!", "✓".green().bold());
        println!("  Crate: {}", output_dir.display().to_string().bold());
        if let Some(ref artifact) = artifact {
            println!(
                "  Extensions: {} file(s), entry {}",
                artifact.files.len(),
                artifact.entry
            );
        }
        println!("\n  Add to your Cargo.toml:");
        println!(
            "    {} = {{ path = \"{}\" }}",
            crate_name.cyan(),
            output_dir.display()
        );
    }

    telemetry::record_sdk_generated("rust");

    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn create_python(
    config_path: &str,
    stack_name: Option<&str>,
    output_override: Option<String>,
    package_name_override: Option<String>,
    module_flag: bool,
    url_override: Option<String>,
    extensions_override: Option<String>,
    manifest_override: Option<String>,
    artifact_dirs: Vec<String>,
) -> Result<()> {
    let _ = config_path;
    let client = ApiClient::new()?;
    let as_module = module_flag;

    let (source, raw_output_dir, package_name) = if let Some(manifest_path) = manifest_override {
        let source = ResolvedStackSource::LocalArtifacts(Box::new(load_local_stack_with_roots(
            &manifest_path,
            &artifact_dirs,
        )?));
        let package_name =
            package_name_override.unwrap_or_else(|| format!("{}-stack", source.sdk_name()));
        let output = output_override
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(format!("./generated/{}-py", source.sdk_name())));
        (source, output, package_name)
    } else {
        let stack_name = stack_name
            .ok_or_else(|| anyhow::anyhow!("stack name is required unless using --manifest"))?;
        println!(
            "{} Looking for stack '{}'...",
            "→".blue().bold(),
            stack_name
        );
        let source =
            resolve_remote_stack_source(&client, stack_name, Some(EXTENSIONS_LANGUAGE_PYTHON))?;
        let package_name =
            package_name_override.unwrap_or_else(|| format!("{}-stack", source.sdk_name()));
        let output = output_override
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(format!("./generated/{}-py", source.sdk_name())));
        (source, output, package_name)
    };

    let stack_url = url_override.or_else(|| source.default_websocket_url());
    let output_dir = raw_output_dir;

    println!(
        "{} Found stack: {}",
        "✓".green().bold(),
        source.stack_id().bold()
    );
    source.print_source_details();
    println!("  Output: {}", output_dir.display());
    if as_module {
        println!("  Mode: module (plain package directory)");
    }
    if let Some(url) = &stack_url {
        println!("  URL: {}", url.cyan());
    } else {
        println!(
            "  URL: {}",
            "(not configured - placeholder will be generated)".dimmed()
        );
    }

    println!("\n{} Generating Python SDK...", "→".blue().bold());

    if let Some(composition) = source.composition_artifacts() {
        if stack_url.is_some() {
            anyhow::bail!(
                "multi-live Python generation requires per-alias URLs; a shared --url is not allowed"
            );
        }
        if extensions_override.is_some() || source.hosted_extensions().is_some() {
            anyhow::bail!(
                "multi-live extensions require a composition-wrapper extension contract; shared stack extensions are not supported"
            );
        }
        let live_urls = match &source {
            ResolvedStackSource::Remote(stack) => stack
                .live_bindings
                .iter()
                .map(|live| (live.alias.clone(), live.binding.websocket_endpoint.clone()))
                .collect(),
            ResolvedStackSource::LocalArtifacts(_) => BTreeMap::new(),
        };
        let output = arete_interpreter::python::compile_composed_public_artifacts_v2(
            composition.program_specs,
            composition.live_specs,
            composition.stack_manifest,
            Some(arete_interpreter::python::PythonCompositionConfig {
                stack: arete_interpreter::python::PythonStackConfig {
                    package_name: package_name.clone(),
                    sdk_version: arete_interpreter::python::GENERATED_PYTHON_SDK_VERSION
                        .to_string(),
                    module_mode: as_module,
                    url: None,
                    http_url: None,
                    extension_modules: Vec::new(),
                    extension_entry: None,
                    program_reads: source.python_program_reads()?,
                    gateway: source.hosted_gateway()?,
                },
                live_urls,
            }),
        )
        .map_err(|error| anyhow::anyhow!("Failed to compile Python composition: {error}"))?;
        if as_module {
            arete_interpreter::python::write_python_composition_module(&output, &output_dir)
                .with_context(|| {
                    format!(
                        "Failed to write Python composition to {}",
                        output_dir.display()
                    )
                })?;
        } else {
            arete_interpreter::python::write_python_composition_package(&output, &output_dir)
                .with_context(|| {
                    format!(
                        "Failed to write Python composition to {}",
                        output_dir.display()
                    )
                })?;
        }
        println!(
            "{} Generated {} aliased Python stack modules in {}",
            "✓".green().bold(),
            output.live_stacks.len(),
            output_dir.display()
        );
        telemetry::record_sdk_generated("python");
        return Ok(());
    }

    let stack_spec = source.load_stack_spec(true)?;

    println!(
        "{} {} entities in stack",
        "→".blue().bold(),
        stack_spec.entities.len()
    );

    generate_python_stack_sdk(
        &source,
        stack_spec,
        &output_dir,
        &package_name,
        as_module,
        stack_url,
        extensions_override.as_deref().map(Path::new),
    )
}

/// Shared single-live Python generation: compile the stack, wire and stage
/// the optional devex extensions bundle, and record provenance. Mirror of
/// [`generate_rust_stack_sdk`].
fn generate_python_stack_sdk(
    source: &ResolvedStackSource,
    stack_spec: arete_interpreter::ast::SerializableStackSpec,
    output_dir: &Path,
    package_name: &str,
    as_module: bool,
    stack_url: Option<String>,
    extensions_path: Option<&Path>,
) -> Result<()> {
    let input_pin = stack_input_pin(source, &stack_spec)?;
    let import_module_name = arete_interpreter::python::python_module_name(package_name);
    let module_dir = if as_module {
        output_dir.to_path_buf()
    } else {
        output_dir.join(&import_module_name)
    };
    let artifact = resolve_python_extensions_artifact(
        extensions_path,
        source.hosted_extensions(),
        &module_dir,
        source.sdk_name(),
        source.output_extensions_fallback(),
    )?;
    let (extension_modules, extension_entry) = match artifact.as_ref() {
        Some(artifact) => {
            let (modules, entry) = python_extension_wiring(artifact)?;
            (modules, Some(entry))
        }
        None => (Vec::new(), None),
    };

    let python_config = arete_interpreter::python::PythonStackConfig {
        package_name: package_name.to_string(),
        sdk_version: arete_interpreter::python::GENERATED_PYTHON_SDK_VERSION.to_string(),
        module_mode: as_module,
        url: stack_url,
        http_url: source.default_http_url(),
        extension_modules,
        extension_entry,
        program_reads: source.python_program_reads()?,
        gateway: source.hosted_gateway()?,
    };

    let output = match source {
        ResolvedStackSource::LocalArtifacts(_) => {
            arete_interpreter::python::compile_stack_spec_with_exact_views(
                stack_spec,
                Some(python_config),
            )
        }
        ResolvedStackSource::Remote(stack) if stack.exact_views => {
            arete_interpreter::python::compile_stack_spec_with_exact_views(
                stack_spec,
                Some(python_config),
            )
        }
        _ => arete_interpreter::python::compile_stack_spec(stack_spec, Some(python_config)),
    }
    .map_err(|e| anyhow::anyhow!("Failed to compile Python: {}", e))?;

    let mut generated = BTreeSet::new();
    if as_module {
        arete_interpreter::python::write_python_module(&output, output_dir).with_context(|| {
            format!("Failed to write Python module to {}", output_dir.display())
        })?;
        generated.extend([
            "__init__.py".to_string(),
            "models.py".to_string(),
            "views.py".to_string(),
        ]);
        if output.programs_py.is_some() {
            generated.insert("programs.py".to_string());
        }
    } else {
        arete_interpreter::python::write_python_package(&output, output_dir).with_context(
            || format!("Failed to write Python package to {}", output_dir.display()),
        )?;
        generated.extend([
            "pyproject.toml".to_string(),
            format!("{}/__init__.py", output.module_name),
            format!("{}/models.py", output.module_name),
            format!("{}/views.py", output.module_name),
        ]);
        if output.programs_py.is_some() {
            generated.insert(format!("{}/programs.py", output.module_name));
        }
    }

    if let Some(ref artifact) = artifact {
        stage_python_extensions_artifact(artifact, &module_dir, &input_pin)?;
    }
    let extension_file_prefix = if as_module {
        String::new()
    } else {
        format!("{}/", output.module_name)
    };
    write_language_sdk_provenance_manifest(
        output_dir,
        generated,
        &extension_file_prefix,
        &input_pin,
        artifact.as_ref(),
    )?;

    if as_module {
        println!(
            "{} Successfully generated Python module!",
            "✓".green().bold()
        );
        println!("  Module: {}", output_dir.display().to_string().bold());
        if let Some(ref artifact) = artifact {
            println!(
                "  Extensions: {} file(s), entry {}",
                artifact.files.len(),
                artifact.entry
            );
        }
        println!("\n  Import from your application:");
        let module_name = output_dir
            .file_name()
            .and_then(|n| n.to_str())
            .map(arete_interpreter::python::python_module_name)
            .unwrap_or_else(|| "module".to_string());
        println!("    import {}", module_name.cyan());
    } else {
        println!("{} Successfully generated Python SDK!", "✓".green().bold());
        println!("  Package: {}", output_dir.display().to_string().bold());
        if let Some(ref artifact) = artifact {
            println!(
                "  Extensions: {} file(s), entry {}",
                artifact.files.len(),
                artifact.entry
            );
        }
        println!("\n  Install into your environment:");
        println!(
            "    pip install -e {}",
            output_dir.display().to_string().cyan()
        );
    }

    telemetry::record_sdk_generated("python");

    Ok(())
}

fn resolve_remote_stack_source(
    client: &ApiClient,
    stack: &str,
    language: Option<&str>,
) -> Result<ResolvedStackSource> {
    let remote = client
        .get_registry_stack_install(stack, language)
        .with_context(|| {
            format!(
                "No accessible hosted stack with identifier '{}' was found.",
                stack
            )
        })?;

    Ok(ResolvedStackSource::Remote(Box::new(remote_stack_install(
        remote,
    )?)))
}

fn remote_stack_install(remote: RegistryStackInstallResponse) -> Result<RemoteStackAst> {
    let exact_views = !remote.live_specs.is_empty()
        || remote.stack_manifest["payload"]["schema"].as_str()
            == Some(arete_artifacts::STACK_MANIFEST_SCHEMA_V2);
    let program_specs = remote
        .programs
        .iter()
        .map(program_spec_artifact_from_registry)
        .collect::<Result<Vec<_>>>()?;
    let composition = if remote.live_specs.is_empty() {
        normalize_singular_registry_install(&remote, &program_specs)?
    } else {
        resolve_v2_registry_composition(&remote, &program_specs)?
    };
    let sdk_name = to_kebab_case(&composition.stack_manifest.payload.name);

    Ok(RemoteStackAst {
        sdk_name,
        name: remote.name,
        stack: remote.stack,
        manifest_hash: remote.stack_manifest_hash,
        program_specs,
        live_specs: composition.live_specs,
        live_bindings: composition.live_bindings,
        stack_manifest: composition.stack_manifest,
        chain_binding: remote.chain_binding,
        transaction_binding: remote.transaction_binding,
        exact_views,
        hosted_extensions: remote
            .extensions
            .as_ref()
            .map(resolved_extensions_artifact_from_registry)
            .transpose()?,
        programs: remote.programs,
        require_managed_gateway: true,
    })
}

fn resolve_v2_registry_composition(
    remote: &RegistryStackInstallResponse,
    program_specs: &[arete_artifacts::ProgramSpecArtifact],
) -> Result<ResolvedRegistryComposition> {
    let stack_manifest: arete_artifacts::StackManifestArtifactV2 =
        serde_json::from_value(remote.stack_manifest.clone())
            .context("Hosted stack returned an invalid V2 StackManifest artifact")?;
    stack_manifest
        .validate()
        .context("Hosted stack returned an invalid V2 StackManifest artifact")?;
    if stack_manifest.artifact_hash.to_string() != remote.stack_manifest_hash {
        anyhow::bail!("Hosted StackManifest hash does not match its envelope");
    }
    if remote.live_specs.len() != stack_manifest.payload.live_specs.len() {
        anyhow::bail!("Hosted liveSpecs do not exactly cover the StackManifest");
    }

    let mut live_specs = Vec::with_capacity(remote.live_specs.len());
    let mut deployment_ids = BTreeSet::new();
    for (position, (reference, descriptor)) in stack_manifest
        .payload
        .live_specs
        .iter()
        .zip(&remote.live_specs)
        .enumerate()
    {
        if descriptor.alias != reference.alias {
            anyhow::bail!(
                "Hosted liveSpecs alias/order mismatch at position {}",
                position
            );
        }
        if descriptor.live_spec_hash != reference.artifact_hash.to_string() {
            anyhow::bail!(
                "Hosted LiveSpec hash mismatch for alias '{}'",
                reference.alias
            );
        }
        let artifact: arete_artifacts::LiveSpecArtifactV2 =
            serde_json::from_value(descriptor.artifact.clone()).with_context(|| {
                format!(
                    "Hosted stack returned an invalid V2 LiveSpec artifact for alias '{}'",
                    reference.alias
                )
            })?;
        artifact.validate().with_context(|| {
            format!(
                "Hosted stack returned an invalid V2 LiveSpec artifact for alias '{}'",
                reference.alias
            )
        })?;
        if artifact.artifact_hash.to_string() != descriptor.live_spec_hash {
            anyhow::bail!(
                "Hosted LiveSpec artifact hash mismatch for alias '{}'",
                reference.alias
            );
        }
        if descriptor.binding.deployment_id <= 0
            || !deployment_ids.insert(descriptor.binding.deployment_id)
            || descriptor.binding.observed_generation <= 0
            || descriptor.binding.websocket_endpoint.trim().is_empty()
            || descriptor.binding.query_endpoint.trim().is_empty()
            || descriptor.binding.websocket_auth_policy.trim().is_empty()
            || descriptor.binding.query_auth_policy.trim().is_empty()
        {
            anyhow::bail!(
                "Hosted LiveSpec binding is incomplete or non-independent for alias '{}'",
                reference.alias
            );
        }
        live_specs.push((reference.alias.clone(), artifact));
    }
    validate_singular_plural_identity(remote, &remote.live_specs)?;
    arete_artifacts::resolve_stack_composition_v2(&stack_manifest, &live_specs, program_specs)
        .context("Hosted stack returned an invalid V2 artifact composition")?;
    Ok(ResolvedRegistryComposition {
        stack_manifest,
        live_specs,
        live_bindings: remote.live_specs.clone(),
    })
}

fn validate_singular_plural_identity(
    remote: &RegistryStackInstallResponse,
    live_specs: &[RegistryLiveSpecInstallDescriptor],
) -> Result<()> {
    let has_singular = remote.live_spec_hash.is_some()
        || remote.live_spec.is_some()
        || remote.websocket_url.is_some()
        || remote.http_url.is_some()
        || remote.websocket_auth.is_some()
        || remote.http_auth.is_some();
    if live_specs.len() != 1 {
        if has_singular {
            anyhow::bail!("Hosted multi-live manifest must not include singular live fields");
        }
        return Ok(());
    }
    let descriptor = &live_specs[0];
    if remote
        .live_spec_hash
        .as_deref()
        .is_some_and(|hash| hash != descriptor.live_spec_hash)
    {
        anyhow::bail!("Hosted singular/plural LiveSpec hash mismatch");
    }
    if remote
        .live_spec
        .as_ref()
        .is_some_and(|artifact| artifact != &descriptor.artifact)
    {
        anyhow::bail!("Hosted singular/plural LiveSpec artifact mismatch");
    }
    if remote
        .websocket_url
        .as_deref()
        .is_some_and(|endpoint| endpoint != descriptor.binding.websocket_endpoint)
    {
        anyhow::bail!("Hosted singular/plural WebSocket endpoint mismatch");
    }
    if remote
        .http_url
        .as_deref()
        .is_some_and(|endpoint| endpoint != descriptor.binding.query_endpoint)
    {
        anyhow::bail!("Hosted singular/plural query endpoint mismatch");
    }
    validate_singular_auth_policy(
        remote.websocket_auth.as_ref(),
        &descriptor.binding.websocket_auth_policy,
        "WebSocket",
    )?;
    validate_singular_auth_policy(
        remote.http_auth.as_ref(),
        &descriptor.binding.query_auth_policy,
        "query",
    )?;
    Ok(())
}

fn validate_singular_auth_policy(
    auth: Option<&serde_json::Value>,
    policy: &str,
    capability: &str,
) -> Result<()> {
    if let Some(auth) = auth {
        let mode = auth.get("mode").and_then(serde_json::Value::as_str);
        if mode != Some(policy) {
            anyhow::bail!("Hosted singular/plural {} auth policy mismatch", capability);
        }
    }
    Ok(())
}

fn normalize_singular_registry_install(
    remote: &RegistryStackInstallResponse,
    program_specs: &[arete_artifacts::ProgramSpecArtifact],
) -> Result<ResolvedRegistryComposition> {
    let live_value = remote
        .live_spec
        .as_ref()
        .context("Hosted stack omitted both liveSpecs and compatibility liveSpec")?;
    let live_hash = remote
        .live_spec_hash
        .as_deref()
        .context("Hosted compatibility liveSpec omitted liveSpecHash")?;
    let manifest_schema = remote.stack_manifest["payload"]["schema"]
        .as_str()
        .unwrap_or_default();

    let (stack_manifest, alias, live_spec) = if manifest_schema
        == arete_artifacts::STACK_MANIFEST_SCHEMA_V2
    {
        let manifest: arete_artifacts::StackManifestArtifactV2 =
            serde_json::from_value(remote.stack_manifest.clone())
                .context("Hosted stack returned an invalid V2 StackManifest artifact")?;
        manifest
            .validate()
            .context("Hosted stack returned an invalid V2 StackManifest artifact")?;
        if manifest.artifact_hash.to_string() != remote.stack_manifest_hash {
            anyhow::bail!("Hosted StackManifest hash does not match its envelope");
        }
        if manifest.payload.live_specs.len() != 1 {
            anyhow::bail!("Hosted multi-live StackManifest requires ordered liveSpecs descriptors");
        }
        let live: arete_artifacts::LiveSpecArtifactV2 = serde_json::from_value(live_value.clone())
            .context("Hosted stack returned an invalid V2 compatibility LiveSpec")?;
        live.validate()
            .context("Hosted stack returned an invalid V2 compatibility LiveSpec")?;
        if live.artifact_hash.to_string() != live_hash
            || manifest.payload.live_specs[0].artifact_hash != live.artifact_hash
        {
            anyhow::bail!("Hosted compatibility artifact hashes do not match their envelopes");
        }
        let alias = manifest.payload.live_specs[0].alias.clone();
        (manifest, alias, live)
    } else {
        let manifest: arete_artifacts::StackManifestArtifact =
            serde_json::from_value(remote.stack_manifest.clone())
                .context("Hosted stack returned an invalid compatibility StackManifest")?;
        let live: arete_artifacts::LiveSpecArtifact = serde_json::from_value(live_value.clone())
            .context("Hosted stack returned an invalid compatibility LiveSpec")?;
        manifest
            .validate()
            .context("Hosted stack returned an invalid compatibility StackManifest")?;
        live.validate()
            .context("Hosted stack returned an invalid compatibility LiveSpec")?;
        if manifest.artifact_hash.to_string() != remote.stack_manifest_hash
            || live.artifact_hash.to_string() != live_hash
        {
            anyhow::bail!("Hosted compatibility artifact hashes do not match their envelopes");
        }
        if manifest.payload.live_specs.len() != 1
            || manifest.payload.live_specs[0].artifact_hash != live.artifact_hash
        {
            anyhow::bail!("Hosted compatibility manifest must reference one exact LiveSpec");
        }
        let normalized_live = arete_artifacts::normalize_live_spec_v1(&live, program_specs)
            .context("Hosted compatibility LiveSpec could not normalize to V2")?;
        let alias = arete_artifacts::DEFAULT_LIVE_ALIAS.to_string();
        let normalized_manifest = arete_artifacts::normalize_stack_manifest_v1(
            &manifest,
            program_specs,
            &[(live.artifact_hash, alias.clone(), &normalized_live)],
        )
        .context("Hosted compatibility StackManifest could not normalize to V2")?;
        (normalized_manifest, alias, normalized_live)
    };

    let websocket_endpoint = remote
        .websocket_url
        .clone()
        .context("Hosted compatibility response omitted websocketUrl")?;
    let query_endpoint = remote
        .http_url
        .clone()
        .context("Hosted compatibility response omitted httpUrl")?;
    let websocket_auth_policy = compatibility_auth_policy(remote.websocket_auth.as_ref())?;
    let query_auth_policy = compatibility_auth_policy(remote.http_auth.as_ref())?;
    let descriptor = RegistryLiveSpecInstallDescriptor {
        alias: alias.clone(),
        live_spec_hash: live_spec.artifact_hash.to_string(),
        artifact: serde_json::to_value(&live_spec)
            .context("Failed to preserve normalized compatibility LiveSpec")?,
        binding: RegistryLiveSpecInstallBinding {
            deployment_id: 0,
            websocket_endpoint,
            query_endpoint,
            websocket_auth_policy,
            query_auth_policy,
            observed_generation: 0,
        },
    };
    let live_specs = vec![(alias, live_spec)];
    arete_artifacts::resolve_stack_composition_v2(&stack_manifest, &live_specs, program_specs)
        .context("Hosted compatibility artifacts do not form a valid V2 composition")?;
    Ok(ResolvedRegistryComposition {
        stack_manifest,
        live_specs,
        live_bindings: vec![descriptor],
    })
}

fn compatibility_auth_policy(auth: Option<&serde_json::Value>) -> Result<String> {
    auth.and_then(|auth| auth.get("mode"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .context("Hosted compatibility auth metadata omitted mode")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn layout(base_name: &str) -> TypeScriptLayout {
        let output_dir = PathBuf::from("/tmp/generated");
        TypeScriptLayout {
            entry_path: output_dir.join(format!("{}.ts", base_name)),
            core_path: output_dir.join(format!("{}-core.ts", base_name)),
            output_dir,
            base_name: base_name.to_string(),
        }
    }

    fn test_artifact(kind: ExtensionsInputKind, hash: &str) -> ResolvedExtensionsArtifact {
        ResolvedExtensionsArtifact {
            entry: "index.ts".to_string(),
            files: vec![ResolvedExtensionsFile {
                path: "index.ts".to_string(),
                contents: "export default {};".to_string(),
            }],
            input_kind: Some(kind),
            input_hash: Some(hash.to_string()),
            sdk_range: None,
            language: None,
            sdk_extension_hash: None,
            sdk_output_tree_hash: None,
            program_extension_bindings: vec![],
        }
    }

    fn test_hosted_program_module_for(
        program_key: &str,
        program_name: &str,
        address: &str,
        extension: Option<ResolvedExtensionsArtifact>,
    ) -> HostedProgramModule {
        let source = format!(
            r#"{{
              "address":"{address}",
              "metadata":{{"name":"{program_name}","version":"1.0.0","spec":"0.1.0"}},
              "instructions":[],"accounts":[],"types":[],"events":[],"errors":[]
            }}"#
        );
        let identity =
            arete_interpreter::program_sdk::build_oss_program_identity_v1_from_idl_bytes(
                source.as_bytes(),
                None,
            )
            .expect("test program identity");
        let program_spec = arete_artifacts::ProgramSpecArtifact::new(identity.program_spec.clone())
            .expect("test ProgramSpec artifact");
        let input_hash = extension
            .as_ref()
            .and_then(|artifact| artifact.input_hash.clone())
            .unwrap_or_else(|| program_spec.artifact_hash.to_string());
        HostedProgramModule {
            program_key: program_key.to_string(),
            program_const_name: to_screaming_snake_case(program_name),
            import_name: format!("hosted{}Program", to_pascal_case(program_key)),
            input_pin: ResolvedExtensionsInputPin {
                kind: ExtensionsInputKind::ProgramSpec,
                hash: input_hash,
            },
            extension,
            program_spec,
            program_config: arete_interpreter::typescript::TypeScriptProgramConfig::from(&identity),
        }
    }

    fn test_hosted_program_module(artifact: ResolvedExtensionsArtifact) -> HostedProgramModule {
        test_hosted_program_module_for(
            "splToken",
            "token",
            "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
            Some(artifact),
        )
    }

    #[test]
    fn sdk_provenance_is_deterministic_and_contains_only_relative_artifact_names() {
        let input_pin = ResolvedExtensionsInputPin {
            kind: ExtensionsInputKind::StackManifest,
            hash: format!("arete:h1:stack-manifest:sha256:{}", "11".repeat(32)),
        };
        let artifact = test_artifact(ExtensionsInputKind::StackManifest, &input_pin.hash);
        let first_layout = layout("ore");
        let second_output = PathBuf::from("/another/checkout/generated");
        let second_layout = TypeScriptLayout {
            entry_path: second_output.join("ore.ts"),
            core_path: second_output.join("ore-core.ts"),
            output_dir: second_output,
            base_name: "ore".to_string(),
        };

        let first = build_sdk_provenance_manifest(&first_layout, &input_pin, Some(&artifact))
            .expect("provenance should build");
        let second = build_sdk_provenance_manifest(&second_layout, &input_pin, Some(&artifact))
            .expect("provenance should be path-independent");
        let json = serde_json::to_string_pretty(&first).expect("provenance should serialize");

        assert_eq!(first, second);
        assert_eq!(first.schema_version, 2);
        assert_eq!(first.input.hash, input_pin.hash);
        assert!(first
            .generator
            .compiler_hash
            .starts_with("arete:h1:compiler:sha256:"));
        assert_eq!(first.extensions.as_ref().unwrap().content_sha256.len(), 64);
        assert_eq!(
            first.artifacts,
            vec!["extensions.json", "index.ts", "ore-core.ts", "ore.ts"]
        );
        assert!(!json.contains("/tmp/"));
        assert!(!json.contains("/another/"));
        assert!(!json.to_ascii_lowercase().contains("timestamp"));
        assert!(!json.contains("createdAt"));
    }

    #[test]
    fn typescript_core_paths_cover_default_program_stack_and_custom_output_names() {
        let cases = [
            ("jurassic-launchpad", "jurassic-fi-token-sale-core"),
            ("ore-stream", "ore-stack-core"),
            ("custom-output", "ore-core"),
        ];

        for (base_name, imported_core) in cases {
            let mut artifact = test_artifact(
                ExtensionsInputKind::ProgramSpec,
                &format!("arete:h1:program-spec:sha256:{}", "22".repeat(32)),
            );
            artifact.files[0].contents =
                format!("import {{ CORE }} from './{imported_core}.js';\nexport default CORE;");
            let paths = typescript_core_paths(&layout(base_name), Some(&artifact))
                .expect("core aliases should resolve");

            assert_eq!(
                paths,
                BTreeSet::from([
                    format!("{base_name}-core.ts"),
                    format!("{imported_core}.ts"),
                ])
            );
        }
    }

    #[test]
    fn typescript_core_staging_preserves_extensions_and_records_aliases() {
        let output_dir =
            std::env::temp_dir().join(format!("a4-typescript-core-aliases-{}", std::process::id()));
        let _ = fs::remove_dir_all(&output_dir);
        fs::create_dir_all(&output_dir).expect("output directory");
        let layout = TypeScriptLayout {
            output_dir: output_dir.clone(),
            base_name: "jurassic-launchpad".to_string(),
            entry_path: output_dir.join("jurassic-launchpad.ts"),
            core_path: output_dir.join("jurassic-launchpad-core.ts"),
        };
        let input_pin = ResolvedExtensionsInputPin {
            kind: ExtensionsInputKind::ProgramSpec,
            hash: format!("arete:h1:program-spec:sha256:{}", "22".repeat(32)),
        };
        let mut artifact = test_artifact(ExtensionsInputKind::ProgramSpec, &input_pin.hash);
        let extension_source =
            "import { CORE } from './jurassic-fi-token-sale-core.js';\nexport default CORE;";
        artifact.files[0].contents = extension_source.to_string();

        write_typescript_core_modules(&layout, "export const CORE = {};\n", Some(&artifact))
            .expect("core and alias should stage");
        stage_extensions_artifact(&artifact, &output_dir, &input_pin)
            .expect("extension should stage unchanged");
        let provenance = build_sdk_provenance_manifest(&layout, &input_pin, Some(&artifact))
            .expect("provenance should include aliases");

        assert_eq!(
            fs::read_to_string(output_dir.join("jurassic-launchpad-core.ts")).expect("layout core"),
            "export const CORE = {};\n"
        );
        assert_eq!(
            fs::read_to_string(output_dir.join("jurassic-fi-token-sale-core.ts"))
                .expect("extension core alias"),
            "export const CORE = {};\n"
        );
        assert_eq!(
            fs::read_to_string(output_dir.join("index.ts")).expect("staged extension"),
            extension_source
        );
        assert!(provenance
            .artifacts
            .contains(&"jurassic-launchpad-core.ts".to_string()));
        assert!(provenance
            .artifacts
            .contains(&"jurassic-fi-token-sale-core.ts".to_string()));
        let _ = fs::remove_dir_all(&output_dir);
    }

    #[test]
    fn typescript_core_staging_rejects_extension_alias_collisions() {
        let output_dir = std::env::temp_dir().join(format!(
            "a4-typescript-core-collision-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&output_dir);
        fs::create_dir_all(&output_dir).expect("output directory");
        let layout = TypeScriptLayout {
            output_dir: output_dir.clone(),
            base_name: "ore-stream".to_string(),
            entry_path: output_dir.join("ore-stream.ts"),
            core_path: output_dir.join("ore-stream-core.ts"),
        };
        let mut artifact = test_artifact(
            ExtensionsInputKind::StackManifest,
            &format!("arete:h1:stack-manifest:sha256:{}", "11".repeat(32)),
        );
        artifact.files[0].contents =
            "import { CORE } from './ore-stack-core.js';\nexport default CORE;".to_string();
        artifact.files.push(ResolvedExtensionsFile {
            path: "ore-stack-core.ts".to_string(),
            contents: "export const userOwned = true;".to_string(),
        });

        let error =
            write_typescript_core_modules(&layout, "export const CORE = {};\n", Some(&artifact))
                .expect_err("extension-owned alias path must be rejected");

        assert!(error
            .to_string()
            .contains("collides with generated SDK artifact"));
        assert!(!output_dir.join("ore-stream-core.ts").exists());
        let _ = fs::remove_dir_all(&output_dir);
    }

    #[test]
    fn sdk_provenance_generation_writes_stable_manifest() {
        let output_dir =
            std::env::temp_dir().join(format!("a4-sdk-provenance-{}", std::process::id()));
        let _ = fs::remove_dir_all(&output_dir);
        fs::create_dir_all(&output_dir).expect("temp output directory should be created");
        let layout = TypeScriptLayout {
            entry_path: output_dir.join("demo.ts"),
            core_path: output_dir.join("demo-core.ts"),
            output_dir: output_dir.clone(),
            base_name: "demo".to_string(),
        };
        let input_pin = ResolvedExtensionsInputPin {
            kind: ExtensionsInputKind::ProgramSpec,
            hash: format!("arete:h1:program-spec:sha256:{}", "22".repeat(32)),
        };

        fs::write(&layout.core_path, "export const answer = 42;").unwrap();
        fs::write(&layout.entry_path, "export * from './demo-core';").unwrap();
        write_sdk_provenance_manifest(&layout, &input_pin, None)
            .expect("provenance should be written");
        let first = fs::read_to_string(output_dir.join(SDK_PROVENANCE_FILE))
            .expect("provenance should be readable");
        write_sdk_provenance_manifest(&layout, &input_pin, None)
            .expect("provenance should be reproducible");
        let second = fs::read_to_string(output_dir.join(SDK_PROVENANCE_FILE))
            .expect("provenance should still be readable");
        let manifest = parse_sdk_provenance_manifest(&first).expect("provenance should parse");
        let _ = fs::remove_dir_all(&output_dir);

        assert_eq!(first, second);
        let SdkProvenanceManifest::V2(manifest) = manifest else {
            panic!("writer must emit provenance V2")
        };
        assert_eq!(manifest.input.kind, ExtensionsInputKind::ProgramSpec);
        assert_eq!(manifest.extensions, None);
        assert_eq!(
            manifest.artifacts,
            vec!["demo-core.ts", "demo.ts", SDK_MANIFEST_FILE]
        );
        assert!(!first.contains(&output_dir.display().to_string()));
    }

    #[test]
    fn sdk_content_manifest_is_stable_across_compilers_and_tracks_payload() {
        let temp = tempfile::tempdir().unwrap();
        let output = temp.path();
        let input = ResolvedExtensionsInputPin {
            kind: ExtensionsInputKind::ProgramSpec,
            hash: format!("arete:h1:program-spec:sha256:{}", "22".repeat(32)),
        };
        let mut manifest = build_sdk_provenance_manifest_from_artifacts(
            BTreeSet::from(["sdk.ts".to_string()]),
            "",
            &input,
            None,
        )
        .unwrap();
        fs::write(output.join("sdk.ts"), "export const value = 1;").unwrap();
        write_sdk_provenance_manifest_file(output, &manifest).unwrap();
        let first = fs::read(output.join(SDK_MANIFEST_FILE)).unwrap();
        let provenance = fs::read(output.join(SDK_PROVENANCE_FILE)).unwrap();
        manifest.generator.compiler_hash = format!("arete:h1:compiler:sha256:{}", "99".repeat(32));
        manifest.generator.version = "999.0.0".into();
        write_sdk_provenance_manifest_file(output, &manifest).unwrap();
        assert_eq!(first, fs::read(output.join(SDK_MANIFEST_FILE)).unwrap());
        assert_ne!(
            provenance,
            fs::read(output.join(SDK_PROVENANCE_FILE)).unwrap()
        );
        fs::write(output.join("sdk.ts"), "export const value = 2;").unwrap();
        write_sdk_provenance_manifest_file(output, &manifest).unwrap();
        assert_ne!(first, fs::read(output.join(SDK_MANIFEST_FILE)).unwrap());
        fs::remove_file(output.join("sdk.ts")).unwrap();
        assert!(write_sdk_provenance_manifest_file(output, &manifest).is_err());
    }

    #[test]
    fn sdk_provenance_prunes_only_previously_owned_stale_artifacts() {
        let output_dir =
            std::env::temp_dir().join(format!("a4-sdk-pruning-{}", std::process::id()));
        let _ = fs::remove_dir_all(&output_dir);
        fs::create_dir_all(output_dir.join("programs/old")).expect("nested output directory");
        fs::write(output_dir.join("programs/old/stale.ts"), "stale")
            .expect("stale generated artifact");
        fs::write(output_dir.join("keep.ts"), "keep").expect("retained generated artifact");
        fs::write(output_dir.join("user.ts"), "user").expect("unowned user artifact");
        let input_pin = ResolvedExtensionsInputPin {
            kind: ExtensionsInputKind::StackManifest,
            hash: format!("arete:h1:stack-manifest:sha256:{}", "44".repeat(32)),
        };
        let mut previous =
            build_sdk_provenance_manifest_from_artifacts(BTreeSet::new(), "", &input_pin, None)
                .expect("previous provenance");
        previous.artifacts = vec!["keep.ts".to_string(), "programs/old/stale.ts".to_string()];
        write_sdk_provenance_manifest_file(&output_dir, &previous)
            .expect("previous provenance should be written");

        let mut next = previous.clone();
        next.artifacts = vec!["keep.ts".to_string(), "new.ts".to_string()];
        fs::write(output_dir.join("new.ts"), "new").unwrap();
        write_sdk_provenance_manifest_file(&output_dir, &next)
            .expect("next provenance should be written");

        assert!(!output_dir.join("programs/old/stale.ts").exists());
        assert!(!output_dir.join("programs/old").exists());
        assert!(output_dir.join("keep.ts").is_file());
        assert!(output_dir.join("user.ts").is_file());
        let _ = fs::remove_dir_all(&output_dir);
    }

    #[cfg(unix)]
    #[test]
    fn sdk_provenance_pruning_does_not_follow_intermediate_symlinks() {
        use std::os::unix::fs::symlink;

        let temp_dir = std::env::temp_dir();
        let output_dir = temp_dir.join(format!("a4-sdk-pruning-symlink-{}", std::process::id()));
        let external_dir = temp_dir.join(format!(
            "a4-sdk-pruning-symlink-target-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&output_dir);
        let _ = fs::remove_dir_all(&external_dir);
        fs::create_dir_all(output_dir.join("programs")).expect("nested output directory");
        fs::create_dir_all(&external_dir).expect("external directory");
        let external_artifact = external_dir.join("stale.ts");
        fs::write(&external_artifact, "external").expect("external artifact");
        symlink(&external_dir, output_dir.join("programs/escaped"))
            .expect("intermediate directory symlink");

        let input_pin = ResolvedExtensionsInputPin {
            kind: ExtensionsInputKind::StackManifest,
            hash: format!("arete:h1:stack-manifest:sha256:{}", "55".repeat(32)),
        };
        let mut previous =
            build_sdk_provenance_manifest_from_artifacts(BTreeSet::new(), "", &input_pin, None)
                .expect("previous provenance");
        previous.artifacts = vec!["programs/escaped/stale.ts".to_string()];
        assert!(write_sdk_provenance_manifest_file(&output_dir, &previous).is_err());
        // An old manifest may reference a path replaced by a symlink since generation.
        fs::write(
            output_dir.join(SDK_PROVENANCE_FILE),
            serde_json::to_vec(&previous).unwrap(),
        )
        .unwrap();

        let mut next = previous.clone();
        next.artifacts.clear();
        write_sdk_provenance_manifest_file(&output_dir, &next)
            .expect("next provenance should be written");

        assert!(external_artifact.is_file());
        fs::remove_file(output_dir.join("programs/escaped")).expect("remove test symlink");
        let _ = fs::remove_dir_all(&output_dir);
        let _ = fs::remove_dir_all(&external_dir);
    }

    #[cfg(unix)]
    #[test]
    fn sdk_provenance_removal_resists_symlink_swap_after_validation() {
        use std::os::unix::fs::symlink;

        let temp_dir = std::env::temp_dir();
        let output_dir = temp_dir.join(format!(
            "a4-sdk-pruning-symlink-swap-{}",
            std::process::id()
        ));
        let external_dir = temp_dir.join(format!(
            "a4-sdk-pruning-symlink-swap-target-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&output_dir);
        let _ = fs::remove_dir_all(&external_dir);
        fs::create_dir_all(output_dir.join("programs/old")).expect("nested output directory");
        fs::create_dir_all(&external_dir).expect("external directory");
        fs::write(output_dir.join("programs/old/stale.ts"), "generated")
            .expect("generated artifact");
        let external_artifact = external_dir.join("stale.ts");
        fs::write(&external_artifact, "external").expect("external artifact");

        let output = Dir::open_ambient_dir(&output_dir, ambient_authority())
            .expect("open output capability");
        let relative = Path::new("programs/old/stale.ts");
        assert!(is_removable_stale_sdk_artifact(&output, relative));

        fs::rename(
            output_dir.join("programs/old"),
            output_dir.join("programs/old-original"),
        )
        .expect("move validated directory");
        symlink(&external_dir, output_dir.join("programs/old"))
            .expect("replace validated directory with symlink");
        assert!(
            remove_stale_sdk_artifact(&output, &output_dir, relative).is_err(),
            "capability-scoped removal must reject the escaped path"
        );

        assert_eq!(
            fs::read_to_string(&external_artifact).expect("external artifact survives"),
            "external"
        );
        assert!(output_dir.join("programs/old-original/stale.ts").is_file());
        fs::remove_file(output_dir.join("programs/old")).expect("remove test symlink");
        let _ = fs::remove_dir_all(&output_dir);
        let _ = fs::remove_dir_all(&external_dir);
    }

    #[test]
    fn sdk_provenance_v1_remains_readable_without_relabeling_legacy_hashes() {
        let contents = r#"{
          "schemaVersion": 1,
          "input": {"kind": "program-spec", "sha256": "legacy-input"},
          "generator": {"name": "a4-cli", "version": "0.3.0", "sha256": "legacy-generator"},
          "extensions": {"sha256": "legacy-extension"},
          "artifacts": ["demo.ts"]
        }"#;

        let manifest = parse_sdk_provenance_manifest(contents).expect("V1 provenance should parse");
        let SdkProvenanceManifest::V1(manifest) = manifest else {
            panic!("V1 provenance must retain its legacy schema")
        };
        assert_eq!(manifest.input.sha256, "legacy-input");
        assert_eq!(manifest.generator.sha256, "legacy-generator");
        assert_eq!(manifest.extensions.unwrap().sha256, "legacy-extension");
    }

    #[test]
    fn extension_hash_is_independent_of_file_order() {
        let mut first = test_artifact(
            ExtensionsInputKind::StackManifest,
            &format!("arete:h1:stack-manifest:sha256:{}", "33".repeat(32)),
        );
        first.files.push(ResolvedExtensionsFile {
            path: "helpers.ts".to_string(),
            contents: "export const helper = true;".to_string(),
        });
        let mut second = first.clone();
        second.files.reverse();

        assert_eq!(
            extensions_artifact_hash(&first),
            extensions_artifact_hash(&second)
        );
    }

    fn registry_stack_install(name: &str, manifest_name: &str) -> RegistryStackInstallResponse {
        let live_spec = arete_artifacts::LiveSpecArtifact::new(arete_artifacts::LiveSpecV1 {
            schema: arete_artifacts::LIVE_SPEC_SCHEMA_V1.to_string(),
            compiler_contract_version: "compiler/v1".into(),
            wire_contract_version: "wire/v1".into(),
            programs: vec![],
            entities: vec![],
            legacy_program_extensions: None,
        })
        .unwrap();
        let stack_manifest =
            arete_artifacts::StackManifestArtifact::new(arete_artifacts::StackManifestV1 {
                schema: arete_artifacts::STACK_MANIFEST_SCHEMA_V1.to_string(),
                name: manifest_name.to_string(),
                programs: vec![],
                live_specs: vec![arete_artifacts::LiveSpecReferenceV1 {
                    artifact_hash: live_spec.artifact_hash,
                }],
                selected_views: vec![],
                queries: vec![],
                extensions: BTreeMap::new(),
                metadata: BTreeMap::new(),
            })
            .unwrap();
        RegistryStackInstallResponse {
            name: name.to_string(),
            stack: "stack-id".to_string(),
            websocket_url: Some("wss://stream.example.test/ws/v2?tenant=stack-id".to_string()),
            http_url: Some("https://reads.unrelated.test/api/arete/v3".to_string()),
            websocket_auth: Some(serde_json::json!({"mode": "signed_session"})),
            http_auth: Some(serde_json::json!({"mode": "signed_session"})),
            description: None,
            visibility: "public".to_string(),
            spec_version_id: Some(1),
            live_spec_hash: Some(live_spec.artifact_hash.to_string()),
            live_spec: Some(serde_json::to_value(live_spec).unwrap()),
            live_specs: Vec::new(),
            stack_manifest_hash: stack_manifest.artifact_hash.to_string(),
            stack_manifest: serde_json::to_value(stack_manifest).unwrap(),
            chain_binding: None,
            transaction_binding: None,
            extensions: None,
            programs: vec![],
        }
    }

    #[test]
    fn remote_stack_uses_manifest_name_for_typescript_basename() {
        let remote = remote_stack_install(registry_stack_install("squads-v4", "SquadsV4Stream"))
            .expect("remote stack should resolve");
        let output_dir = default_typescript_output_dir(&remote.sdk_name);
        let layout = resolve_typescript_layout(&output_dir, &remote.sdk_name);

        assert_eq!(remote.sdk_name, "squads-v4-stream");
        assert_eq!(output_dir, PathBuf::from("./generated/squads-v4"));
        assert_eq!(
            layout.core_path,
            PathBuf::from("./generated/squads-v4/squads-v4-stream-core.ts")
        );
    }

    #[test]
    fn remote_stack_retains_independent_registry_endpoints() {
        let remote =
            remote_stack_install(registry_stack_install("endpoint-stack", "EndpointStream"))
                .expect("remote stack should resolve");
        let source = ResolvedStackSource::Remote(Box::new(remote));

        assert_eq!(
            source.default_websocket_url().as_deref(),
            Some("wss://stream.example.test/ws/v2?tenant=stack-id")
        );
        assert_eq!(
            source.default_http_url().as_deref(),
            Some("https://reads.unrelated.test/api/arete/v3")
        );
    }

    #[test]
    fn remote_stack_ignores_legacy_ast_name() {
        let remote = remote_stack_install(registry_stack_install("squads-v4", "ManifestStream"))
            .expect("remote stack should resolve");

        assert_eq!(remote.sdk_name, "manifest-stream");
    }

    /// A hosted bundle fetched with `?language=rust` carries
    /// `language: "rust"` in its manifest; the resolved artifact must keep
    /// that marker so it reaches the Rust extensions rung — and so the
    /// TypeScript rung keeps rejecting it if it ever leaks there.
    #[test]
    fn remote_stack_routes_rust_language_hosted_extensions_to_the_rust_rung() {
        let mut install = registry_stack_install("ore", "OreStream");
        install.extensions = Some(RegistrySdkExtensionArtifact {
            artifact_hash: "rust-extension-hash".to_string(),
            sdk_extension_hash: None,
            sdk_output_tree_hash: None,
            manifest: crate::api_client::RegistrySdkExtensionManifest {
                entry: "extensions.rs".to_string(),
                files: vec!["extensions.rs".to_string()],
                input_kind: Some(RegistrySdkExtensionInputKind::StackManifest),
                input_hash: Some(install.stack_manifest_hash.clone()),
                sdk_range: None,
                language: Some(EXTENSIONS_LANGUAGE_RUST.to_string()),
            },
            files: BTreeMap::from([(
                "extensions.rs".to_string(),
                "pub trait Devex {}\n".to_string(),
            )]),
            created_at: "2026-08-04T00:00:00Z".to_string(),
        });

        let remote = remote_stack_install(install).expect("remote stack should resolve");
        let hosted = remote
            .hosted_extensions
            .as_ref()
            .expect("hosted extensions should be resolved");
        assert_eq!(hosted.language.as_deref(), Some(EXTENSIONS_LANGUAGE_RUST));

        // Rust rung accepts the hosted artifact...
        let output_dir =
            std::env::temp_dir().join(format!("a4-rust-hosted-route-{}", std::process::id()));
        let _ = fs::remove_dir_all(&output_dir);
        let resolved = resolve_rust_extensions_artifact(
            None,
            Some(hosted),
            &output_dir,
            "ore",
            OutputExtensionsFallback::Ignore,
        )
        .expect("hosted Rust bundle should resolve")
        .expect("hosted Rust bundle should be present");
        assert_eq!(resolved.entry, "extensions.rs");

        // ...while the TypeScript rung rejects it outright.
        let error = resolve_extensions_artifact(
            None,
            &layout("ore"),
            Some(hosted),
            OutputExtensionsFallback::Ignore,
        )
        .expect_err("TypeScript generation must reject a Rust hosted bundle");
        assert!(error.to_string().contains("declares language 'rust'"));
    }

    #[test]
    fn render_typescript_stack_entry_without_extensions_aliases_core() {
        let rendered = render_typescript_stack_entry(
            &layout("ore-augmented-stream"),
            "OreAugmentedStream",
            None,
            &[],
            &[],
            &[],
        );

        assert!(rendered.contains(
            "import { ORE_AUGMENTED_STREAM_STACK_CORE } from './ore-augmented-stream-core.js';"
        ));
        assert!(rendered.contains(
            "export const ORE_AUGMENTED_STREAM_STACK = ORE_AUGMENTED_STREAM_STACK_CORE;"
        ));
        assert!(rendered.contains("export default ORE_AUGMENTED_STREAM_STACK;"));
        assert!(!rendered.contains("extendStack"));
    }

    #[test]
    fn render_typescript_stack_entry_with_extensions_wires_extend_stack() {
        let rendered = render_typescript_stack_entry(
            &layout("squads-v4-stream"),
            "SquadsV4Stream",
            Some("squads-v4-extensions.ts"),
            &["squads-v4-extensions.ts"],
            &[],
            &[],
        );

        assert!(rendered.contains("import { extendStack } from '@usearete/sdk';"));
        assert!(rendered
            .contains("import { SQUADS_V4_STREAM_STACK_CORE } from './squads-v4-stream-core.js';"));
        assert!(rendered.contains("import stackExtensions from './squads-v4-extensions.js';"));
        assert!(!rendered.contains("export * from './squads-v4-extensions.js';"));
        assert!(rendered.contains("export const SQUADS_V4_STREAM_STACK = extendStack("));
        assert!(rendered.contains("export default SQUADS_V4_STREAM_STACK;"));
    }

    #[test]
    fn render_typescript_stack_entry_with_program_extensions_wraps_core_programs() {
        let rendered = render_typescript_stack_entry(
            &layout("squads-v4-stream"),
            "SquadsV4Stream",
            Some("squads-v4-extensions.ts"),
            &["squads-v4-devex.ts", "squads-v4-extensions.ts"],
            &[ProgramExtensionBinding {
                export_name: "squadsProgramExtensions".to_string(),
                program_key: "squadsMultisigProgram".to_string(),
            }],
            &[],
        );

        assert!(rendered.contains("import { extendPrograms, extendStack } from '@usearete/sdk';"));
        assert!(rendered.contains(
            "import stackExtensions, { squadsProgramExtensions } from './squads-v4-extensions.js';"
        ));
        assert!(!rendered.contains("export * from './squads-v4-devex.js';"));
        assert!(!rendered.contains("export * from './squads-v4-extensions.js';"));
        assert!(rendered.contains("const CORE = {"));
        assert!(
            rendered.contains("programs: extendPrograms(SQUADS_V4_STREAM_STACK_CORE.programs, {")
        );
        assert!(rendered.contains("squadsMultisigProgram: squadsProgramExtensions,"));
        assert!(rendered.contains("export const SQUADS_V4_STREAM_STACK = extendStack("));
        assert!(rendered.contains("  CORE,"));
    }

    #[test]
    fn hosted_program_extensions_only_replace_portable_programs() {
        let mut artifact = test_artifact(ExtensionsInputKind::ProgramSpec, "program-spec-hash");
        artifact.entry = "spl-token-extensions.ts".to_string();
        artifact.files[0].path = artifact.entry.clone();
        let hosted = test_hosted_program_module(artifact);
        let rendered = render_typescript_stack_entry(
            &layout("token-stack"),
            "TokenStack",
            None,
            &[],
            &[],
            &[hosted],
        );

        assert!(rendered.contains(
            "import hostedSplTokenProgram from './programs/spl-token/__arete-program.js';"
        ));
        assert!(rendered.contains("...TOKEN_STACK_STACK_CORE,"));
        assert!(rendered.contains("const HOSTED_PROGRAMS = {"));
        assert!(rendered.contains("...TOKEN_STACK_STACK_CORE.programs,"));
        assert!(rendered.contains("programs: HOSTED_PROGRAMS,"));
        assert!(rendered.contains("splToken: hostedSplTokenProgram,"));
        assert!(!rendered.contains("programReads:"));
    }

    #[test]
    fn hosted_program_without_extensions_still_bundles_its_read_descriptor() {
        let hosted = test_hosted_program_module_for(
            "entropy",
            "entropy",
            "Vote111111111111111111111111111111111111111",
            None,
        );
        let rendered = render_hosted_program_entry(&hosted);

        assert!(rendered.contains("import { withProgramRead } from '@usearete/sdk';"));
        assert!(rendered.contains("ENTROPY_READ as BASE_PROGRAM_READ"));
        assert!(rendered.contains(
            "export const ENTROPY_PROGRAM = withProgramRead(BASE_PROGRAM, BASE_PROGRAM_READ);"
        ));
        assert!(!rendered.contains("programExtensions"));
    }

    #[test]
    fn hosted_program_extension_staging_writes_an_isolated_program_sdk() {
        let output_dir = std::env::temp_dir().join(format!(
            "a4-hosted-program-extension-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&output_dir);
        fs::create_dir_all(&output_dir).expect("output directory");
        let layout = TypeScriptLayout {
            output_dir: output_dir.clone(),
            base_name: "ordered-stream".to_string(),
            entry_path: output_dir.join("ordered-stream.ts"),
            core_path: output_dir.join("ordered-stream-core.ts"),
        };
        let program_hash = format!("arete:h1:program-spec:sha256:{}", "22".repeat(32));
        let mut artifact = test_artifact(ExtensionsInputKind::ProgramSpec, &program_hash);
        artifact.entry = "spl-token-extensions.ts".to_string();
        artifact.files[0] = ResolvedExtensionsFile {
            path: artifact.entry.clone(),
            contents: "import { TOKEN } from './spl-token-core';\nexport default {};".to_string(),
        };
        let hosted = test_hosted_program_module(artifact);

        stage_hosted_program_modules(std::slice::from_ref(&hosted), &layout, "@usearete/sdk")
            .expect("hosted program extension should stage");
        let stack_pin = ResolvedExtensionsInputPin {
            kind: ExtensionsInputKind::StackManifest,
            hash: format!("arete:h1:stack-manifest:sha256:{}", "11".repeat(32)),
        };
        let provenance = build_sdk_provenance_manifest_with_program_extensions(
            &layout,
            &stack_pin,
            None,
            std::slice::from_ref(&hosted),
        )
        .expect("hosted program provenance should build");
        let program_dir = output_dir.join("programs/spl-token");
        let core = fs::read_to_string(program_dir.join("spl-token-core.ts"))
            .expect("isolated program core");
        let entry = fs::read_to_string(program_dir.join(HOSTED_PROGRAM_ENTRY))
            .expect("isolated program entry");
        let extension_exists = program_dir.join("spl-token-extensions.ts").is_file();
        let _ = fs::remove_dir_all(&output_dir);

        assert!(core.contains("export const TOKEN = {"));
        assert!(!core.contains("ORDERED_STREAM_STACK_CORE"));
        assert!(entry.contains("import { extendProgram, withProgramRead }"));
        assert!(entry.contains("extendProgram(BASE_PROGRAM, programExtensions)"));
        assert!(entry.contains("BASE_PROGRAM_READ"));
        assert!(!entry.contains("export * from './spl-token-extensions.js';"));
        assert!(extension_exists);
        assert!(provenance.program_extensions.contains_key("splToken"));
        assert!(provenance
            .artifacts
            .contains(&"programs/spl-token/__arete-program.ts".to_string()));
        assert!(provenance
            .artifacts
            .contains(&"programs/spl-token/spl-token-core.ts".to_string()));
    }

    #[test]
    fn hosted_program_staging_namespaces_identical_extension_paths() {
        let output_dir = std::env::temp_dir().join(format!(
            "a4-hosted-program-namespaces-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&output_dir);
        fs::create_dir_all(&output_dir).expect("output directory");
        let layout = TypeScriptLayout {
            output_dir: output_dir.clone(),
            base_name: "collision-stack".to_string(),
            entry_path: output_dir.join("collision-stack.ts"),
            core_path: output_dir.join("collision-stack-core.ts"),
        };
        let extension = |name: &str, hash: &str| {
            ResolvedExtensionsArtifact {
                entry: "extensions.ts".to_string(),
                files: vec![
                    ResolvedExtensionsFile {
                        path: "shared-devex.ts".to_string(),
                        contents: format!(
                            "import * as low from './{name}-core.js';\nexport const marker = '{}';\nexport {{ low }};",
                            name
                        ),
                    },
                    ResolvedExtensionsFile {
                        path: "extensions.ts".to_string(),
                        contents:
                            "export { marker } from './shared-devex.js';\nexport default {};"
                                .to_string(),
                    },
                ],
                input_kind: Some(ExtensionsInputKind::ProgramSpec),
                input_hash: Some(hash.to_string()),
                sdk_range: None,
                language: None,
                sdk_extension_hash: None,
                sdk_output_tree_hash: None,
                program_extension_bindings: vec![],
            }
        };
        let alpha = test_hosted_program_module_for(
            "alpha",
            "alpha",
            "11111111111111111111111111111111",
            Some(extension("alpha", "alpha-hash")),
        );
        let beta = test_hosted_program_module_for(
            "beta",
            "beta",
            "Vote111111111111111111111111111111111111111",
            Some(extension("beta", "beta-hash")),
        );

        stage_hosted_program_modules(&[alpha, beta], &layout, "@usearete/sdk")
            .expect("both hosted programs should stage");
        let alpha_devex = fs::read_to_string(output_dir.join("programs/alpha/shared-devex.ts"))
            .expect("alpha devex");
        let beta_devex = fs::read_to_string(output_dir.join("programs/beta/shared-devex.ts"))
            .expect("beta devex");
        let root_collision = output_dir.join("shared-devex.ts").exists();
        let _ = fs::remove_dir_all(&output_dir);

        assert!(alpha_devex.contains("marker = 'alpha'"));
        assert!(beta_devex.contains("marker = 'beta'"));
        assert!(!root_collision);
    }

    #[test]
    fn render_typescript_program_entry_without_extensions_aliases_core() {
        let rendered = render_typescript_program_entry(&layout("spl-token"), "token", None);

        assert!(rendered.contains("import { TOKEN as SPL_TOKEN_PROGRAM_CORE, TOKEN_READ as SPL_TOKEN_PROGRAM_READ_CORE } from './spl-token-core.js';"));
        assert!(rendered
            .contains("export { TOKEN as SPL_TOKEN_PROGRAM_CORE } from './spl-token-core.js';"));
        assert!(rendered.contains("import { withProgramRead } from '@usearete/sdk';"));
        assert!(rendered.contains(
            "export const SPL_TOKEN_PROGRAM = withProgramRead(SPL_TOKEN_PROGRAM_CORE, SPL_TOKEN_PROGRAM_READ_CORE);"
        ));
        assert!(
            rendered.contains("export const SPL_TOKEN_PROGRAM_READ = SPL_TOKEN_PROGRAM_READ_CORE;")
        );
        assert!(rendered.contains("export default SPL_TOKEN_PROGRAM;"));
    }

    #[test]
    fn render_typescript_program_entry_with_extensions_merges_core_and_extensions() {
        let rendered = render_typescript_program_entry(
            &layout("system-program"),
            "system_program",
            Some("system-program-extensions.ts"),
        );

        assert!(
            rendered.contains("import { extendProgram, withProgramRead } from '@usearete/sdk';")
        );
        assert!(rendered.contains("import { SYSTEM_PROGRAM as SYSTEM_PROGRAM_CORE, SYSTEM_PROGRAM_READ as SYSTEM_PROGRAM_READ_CORE } from './system-program-core.js';"));
        assert!(rendered.contains(
            "export { SYSTEM_PROGRAM as SYSTEM_PROGRAM_CORE } from './system-program-core.js';"
        ));
        assert!(
            rendered.contains("import programExtensions from './system-program-extensions.js';")
        );
        assert!(!rendered.contains("export * from './system-program-extensions.js';"));
        assert!(rendered.contains("export const SYSTEM_PROGRAM = withProgramRead("));
        assert!(rendered.contains("extendProgram(SYSTEM_PROGRAM_CORE, programExtensions),"));
        assert!(rendered.contains("SYSTEM_PROGRAM_READ_CORE,"));
        assert!(rendered.contains("export default SYSTEM_PROGRAM;"));
    }

    #[test]
    fn render_typescript_program_collection_entry_aliases_core() {
        let rendered = render_typescript_program_collection_entry(
            &layout("ore-stream-programs"),
            "OreStream",
            None,
            &[],
        );

        assert!(rendered.contains("import { ORE_STREAM_PROGRAMS as ORE_STREAM_PROGRAMS_CORE } from './ore-stream-programs-core.js';"));
        assert!(rendered.contains("export const ORE_STREAM_PROGRAMS = ORE_STREAM_PROGRAMS_CORE;"));
        assert!(rendered.contains("export default ORE_STREAM_PROGRAMS;"));
    }

    #[test]
    fn render_typescript_program_collection_entry_with_extensions_uses_extend_programs() {
        let rendered = render_typescript_program_collection_entry(
            &layout("ore-stream-programs"),
            "OreStream",
            Some("ore-program-extensions.ts"),
            &[],
        );

        assert!(rendered.contains("import { extendPrograms } from '@usearete/sdk';"));
        assert!(rendered.contains("import programExtensions from './ore-program-extensions.js';"));
        assert!(!rendered.contains("export * from './ore-program-extensions.js';"));
        assert!(rendered.contains("export const ORE_STREAM_PROGRAMS = extendPrograms(ORE_STREAM_PROGRAMS_CORE, programExtensions);"));
    }

    #[test]
    fn build_pda_degradation_summary_groups_by_reason() {
        let lines = build_pda_degradation_summary(&[
            arete_interpreter::typescript_instructions::PdaDegradation {
                instruction_name: "deposit".to_string(),
                account_name: "vault".to_string(),
                pda_name: Some("vault".to_string()),
                source: arete_interpreter::typescript_instructions::PdaDegradationSource::Registry,
                reason: "seed references account 'authority' not present in this instruction"
                    .to_string(),
            },
            arete_interpreter::typescript_instructions::PdaDegradation {
                instruction_name: "withdraw".to_string(),
                account_name: "vault".to_string(),
                pda_name: Some("vault".to_string()),
                source: arete_interpreter::typescript_instructions::PdaDegradationSource::Registry,
                reason: "seed references account 'authority' not present in this instruction"
                    .to_string(),
            },
        ]);

        assert_eq!(lines.len(), 2);
        assert!(
            lines[0].contains("2 PDA account(s) degraded to userProvided across 2 instruction(s)")
        );
        assert_eq!(
            lines[1],
            "   2x seed references account 'authority' not present in this instruction"
        );
    }

    #[test]
    fn validate_extensions_input_pin_accepts_matching_stack_manifest_pin() {
        let artifact = test_artifact(ExtensionsInputKind::StackManifest, "hash-1");
        let input_pin = ResolvedExtensionsInputPin {
            kind: ExtensionsInputKind::StackManifest,
            hash: "hash-1".to_string(),
        };

        assert!(validate_extensions_input_pin(&artifact, &input_pin).is_empty());
    }

    #[test]
    fn validate_extensions_input_pin_reports_kind_mismatch() {
        let artifact = test_artifact(ExtensionsInputKind::ProgramSpec, "hash-1");
        let input_pin = ResolvedExtensionsInputPin {
            kind: ExtensionsInputKind::StackManifest,
            hash: "hash-1".to_string(),
        };

        assert_eq!(
            validate_extensions_input_pin(&artifact, &input_pin),
            vec![
                "extensions input kind mismatch: manifest=program-spec, generated=stack-manifest"
                    .to_string()
            ]
        );
    }

    #[test]
    fn validate_extensions_input_pin_reports_hash_mismatch() {
        let artifact = test_artifact(ExtensionsInputKind::StackManifest, "hash-1");
        let input_pin = ResolvedExtensionsInputPin {
            kind: ExtensionsInputKind::StackManifest,
            hash: "hash-2".to_string(),
        };

        assert_eq!(
            validate_extensions_input_pin(&artifact, &input_pin),
            vec!["extensions input hash mismatch: manifest=hash-1, generated=hash-2".to_string()]
        );
    }

    #[test]
    fn stage_extensions_artifact_rejects_input_pin_mismatch() {
        let artifact = test_artifact(ExtensionsInputKind::ProgramSpec, "hash-1");
        let input_pin = ResolvedExtensionsInputPin {
            kind: ExtensionsInputKind::StackManifest,
            hash: "hash-1".to_string(),
        };
        let output_dir =
            std::env::temp_dir().join(format!("a4-mismatched-extensions-{}", std::process::id()));

        let error = stage_extensions_artifact(&artifact, &output_dir, &input_pin)
            .expect_err("mismatched extension artifact should be rejected");

        assert!(error.to_string().contains("extensions input kind mismatch"));
        assert!(!output_dir.join("index.ts").exists());
    }

    #[test]
    fn version_satisfies_range_supports_standard_semver_requirements() {
        assert!(version_satisfies_range("0.1.5", "^0.1.5"));
        assert!(version_satisfies_range("0.1.8", ">=0.1.5, <0.2.0"));
        assert!(!version_satisfies_range("0.2.0", ">=0.1.5, <0.2.0"));
        assert!(version_satisfies_range("0.2.0", "^0.2.0 || ^0.3.0"));
        assert!(version_satisfies_range("0.3.4", "^0.2.0 || ^0.3.0"));
        assert!(!version_satisfies_range("0.4.0", "^0.2.0 || ^0.3.0"));
        assert!(!version_satisfies_range("0.2.0", "^0.2.0 ||"));
    }

    #[test]
    fn parse_program_extension_bindings_finds_named_exports() {
        let bindings = parse_program_extension_bindings(
            r#"
            export const presaleProgramExtensions = defineProgramExtensions<
              typeof METEORA_PRESALE_STREAM_STACK_CORE.programs.presale
            >()({
              createInstructions() {
                return {};
              },
            });
            "#,
        );

        assert_eq!(
            bindings,
            vec![ProgramExtensionBinding {
                export_name: "presaleProgramExtensions".to_string(),
                program_key: "presale".to_string(),
            }],
        );
    }

    #[test]
    fn live_module_imports_are_exact_and_portable() {
        assert_eq!(
            parse_live_module_imports(&[
                "squads=./squads-v4/squads-v4-stream.js".to_string(),
                "damm=./meteora-damm/meteora-damm-stream.js".to_string(),
            ])
            .unwrap(),
            BTreeMap::from([
                (
                    "damm".to_string(),
                    "./meteora-damm/meteora-damm-stream.js".to_string(),
                ),
                (
                    "squads".to_string(),
                    "./squads-v4/squads-v4-stream.js".to_string(),
                ),
            ])
        );
        assert!(parse_live_module_imports(&["live=../escape.js".to_string()]).is_err());
        assert!(parse_live_module_imports(&[
            "live=./first.js".to_string(),
            "live=./second.js".to_string(),
        ])
        .is_err());
    }

    fn rust_test_artifact(hash: &str) -> ResolvedExtensionsArtifact {
        ResolvedExtensionsArtifact {
            entry: "extensions.rs".to_string(),
            files: vec![
                ResolvedExtensionsFile {
                    path: "devex.rs".to_string(),
                    contents: "pub fn helper() {}\n".to_string(),
                },
                ResolvedExtensionsFile {
                    path: "extensions.rs".to_string(),
                    contents: "pub use super::devex::*;\n".to_string(),
                },
            ],
            input_kind: Some(ExtensionsInputKind::StackManifest),
            input_hash: Some(hash.to_string()),
            sdk_range: None,
            language: Some(EXTENSIONS_LANGUAGE_RUST.to_string()),
            sdk_extension_hash: None,
            sdk_output_tree_hash: None,
            program_extension_bindings: vec![],
        }
    }

    fn write_bundle_dir(dir: &Path, artifact: &ResolvedExtensionsArtifact) {
        fs::create_dir_all(dir).expect("bundle directory");
        for file in &artifact.files {
            fs::write(dir.join(&file.path), &file.contents).expect("bundle file");
        }
        fs::write(
            dir.join("extensions.json"),
            serde_json::to_string_pretty(&artifact.manifest()).unwrap(),
        )
        .expect("bundle manifest");
    }

    fn ts_bundle_artifact(hash: &str) -> ResolvedExtensionsArtifact {
        ResolvedExtensionsArtifact {
            entry: "ore-extensions.ts".to_string(),
            files: vec![
                ResolvedExtensionsFile {
                    path: "ore-devex.ts".to_string(),
                    contents: "export const helper = 1;\n".to_string(),
                },
                ResolvedExtensionsFile {
                    path: "ore-extensions.ts".to_string(),
                    contents: "export * from './ore-devex.js';\nexport default {};\n".to_string(),
                },
            ],
            input_kind: Some(ExtensionsInputKind::StackManifest),
            input_hash: Some(hash.to_string()),
            sdk_range: Some("^0.2.0 || ^0.3.0".to_string()),
            language: Some(EXTENSIONS_LANGUAGE_TYPESCRIPT.to_string()),
            sdk_extension_hash: None,
            sdk_output_tree_hash: None,
            program_extension_bindings: vec![],
        }
    }

    fn layout_in(output_dir: &Path, base_name: &str) -> TypeScriptLayout {
        TypeScriptLayout {
            entry_path: output_dir.join(format!("{}.ts", base_name)),
            core_path: output_dir.join(format!("{}-core.ts", base_name)),
            output_dir: output_dir.to_path_buf(),
            base_name: base_name.to_string(),
        }
    }

    #[test]
    fn typescript_extensions_manifest_without_language_roundtrips_byte_identically() {
        let staged = r#"{
  "entry": "ore-stack-extensions.ts",
  "files": [
    "ore-devex.ts",
    "ore-stack-extensions.ts"
  ],
  "inputKind": "stack-manifest",
  "inputHash": "arete:h1:stack-manifest:sha256:edd1ffe8ef2c26232c1440f20625b8834b8c4d4e63250136ce62bcc38609f84a",
  "sdkRange": "^0.2.0 || ^0.3.0"
}"#;

        let manifest: ExtensionsManifest =
            serde_json::from_str(staged).expect("TS manifest without language should parse");
        assert_eq!(manifest.language, None);
        let reserialized = serde_json::to_string_pretty(&manifest).unwrap();
        assert_eq!(reserialized, staged);
        assert!(!reserialized.contains("language"));
    }

    #[test]
    fn rust_resolution_prefers_explicit_bundle_over_output_dir_manifest() {
        let root =
            std::env::temp_dir().join(format!("a4-rust-ext-explicit-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let bundle_dir = root.join("bundle");
        let output_dir = root.join("out");
        let explicit = rust_test_artifact("explicit-hash");
        let staged = rust_test_artifact("staged-hash");
        write_bundle_dir(&bundle_dir, &explicit);
        write_bundle_dir(&output_dir, &staged);

        let resolved = resolve_rust_extensions_artifact(
            Some(&bundle_dir),
            None,
            &output_dir,
            "ore",
            OutputExtensionsFallback::Ignore,
        )
        .expect("explicit bundle should resolve")
        .expect("explicit bundle should be present");
        let _ = fs::remove_dir_all(&root);

        assert_eq!(resolved.input_hash.as_deref(), Some("explicit-hash"));
        assert_eq!(resolved.language.as_deref(), Some(EXTENSIONS_LANGUAGE_RUST));
    }

    #[test]
    fn rust_resolution_accepts_single_entry_file() {
        let root = std::env::temp_dir().join(format!("a4-rust-ext-single-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("bundle root");
        fs::write(root.join("extensions.rs"), "pub trait Devex {}\n").expect("entry file");

        let resolved = resolve_rust_extensions_artifact(
            Some(&root.join("extensions.rs")),
            None,
            &root.join("missing-output"),
            "ore",
            OutputExtensionsFallback::Ignore,
        )
        .expect("single-entry bundle should resolve")
        .expect("single-entry bundle should be present");
        let _ = fs::remove_dir_all(&root);

        assert_eq!(resolved.entry, "extensions.rs");
        assert_eq!(
            resolved
                .files
                .iter()
                .map(|file| file.path.as_str())
                .collect::<Vec<_>>(),
            vec!["extensions.rs"]
        );
        assert_eq!(resolved.input_hash, None);
        assert_eq!(resolved.language.as_deref(), Some(EXTENSIONS_LANGUAGE_RUST));
    }

    #[test]
    fn rust_resolution_reuses_output_dir_manifest_and_preserves_pins() {
        let output_dir =
            std::env::temp_dir().join(format!("a4-rust-ext-outdir-{}", std::process::id()));
        let _ = fs::remove_dir_all(&output_dir);
        let staged = rust_test_artifact("staged-pin-hash");
        write_bundle_dir(&output_dir, &staged);

        let resolved = resolve_rust_extensions_artifact(
            None,
            None,
            &output_dir,
            "ore",
            OutputExtensionsFallback::Reuse,
        )
        .expect("staged manifest should resolve")
        .expect("staged manifest should be present");
        let _ = fs::remove_dir_all(&output_dir);

        assert_eq!(
            resolved.input_kind,
            Some(ExtensionsInputKind::StackManifest)
        );
        assert_eq!(resolved.input_hash.as_deref(), Some("staged-pin-hash"));
        assert_eq!(resolved.entry, "extensions.rs");
    }

    #[test]
    fn rust_hosted_resolution_ignores_stale_output_dir_manifest() {
        let output_dir = std::env::temp_dir().join(format!(
            "a4-rust-ext-hosted-authoritative-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&output_dir);
        write_bundle_dir(&output_dir, &rust_test_artifact("stale-rust-hash"));

        let resolved = resolve_rust_extensions_artifact(
            None,
            None,
            &output_dir,
            "ore",
            OutputExtensionsFallback::Ignore,
        )
        .expect("hosted resolution should succeed");
        let _ = fs::remove_dir_all(&output_dir);

        assert!(resolved.is_none());
    }

    #[test]
    fn rust_resolution_returns_none_without_sources() {
        let output_dir =
            std::env::temp_dir().join(format!("a4-rust-ext-none-{}", std::process::id()));
        let _ = fs::remove_dir_all(&output_dir);

        let resolved = resolve_rust_extensions_artifact(
            None,
            None,
            &output_dir,
            "ore",
            OutputExtensionsFallback::Reuse,
        )
        .expect("empty resolution should succeed");

        assert!(resolved.is_none());
    }

    #[test]
    fn rust_resolution_rejects_typescript_bundles() {
        let bundle_dir =
            std::env::temp_dir().join(format!("a4-rust-ext-ts-reject-{}", std::process::id()));
        let _ = fs::remove_dir_all(&bundle_dir);
        let mut typescript = rust_test_artifact("hash-1");
        typescript.language = Some(EXTENSIONS_LANGUAGE_TYPESCRIPT.to_string());
        write_bundle_dir(&bundle_dir, &typescript);

        let error = resolve_rust_extensions_artifact(
            Some(&bundle_dir),
            None,
            &bundle_dir.join("missing-output"),
            "ore",
            OutputExtensionsFallback::Ignore,
        )
        .expect_err("TypeScript bundle must be rejected for Rust generation");
        let _ = fs::remove_dir_all(&bundle_dir);

        assert!(error.to_string().contains("declares language 'typescript'"));
    }

    #[test]
    fn typescript_resolution_rejects_rust_bundles() {
        let bundle_dir =
            std::env::temp_dir().join(format!("a4-ts-ext-rust-reject-{}", std::process::id()));
        let _ = fs::remove_dir_all(&bundle_dir);
        write_bundle_dir(&bundle_dir, &rust_test_artifact("hash-1"));

        let error = resolve_extensions_artifact(
            Some(&bundle_dir),
            &layout("ore"),
            None,
            OutputExtensionsFallback::Ignore,
        )
        .expect_err("Rust bundle must be rejected for TypeScript generation");
        let _ = fs::remove_dir_all(&bundle_dir);

        assert!(error.to_string().contains("declares language 'rust'"));
    }

    /// Regression: `a4 sdk sync` (no `--extensions`) against an output dir
    /// holding a full staged bundle must reuse the manifest with its input
    /// pins and helper files intact, instead of re-inferring a pinless
    /// entry-only artifact.
    #[test]
    fn typescript_resolution_reuses_output_dir_manifest_and_preserves_pins() {
        let output_dir =
            std::env::temp_dir().join(format!("a4-ts-ext-outdir-{}", std::process::id()));
        let _ = fs::remove_dir_all(&output_dir);
        write_bundle_dir(&output_dir, &ts_bundle_artifact("ts-staged-pin-hash"));

        let resolved = resolve_extensions_artifact(
            None,
            &layout_in(&output_dir, "ore"),
            None,
            OutputExtensionsFallback::Reuse,
        )
        .expect("staged manifest should resolve")
        .expect("staged manifest should be present");
        let _ = fs::remove_dir_all(&output_dir);

        assert_eq!(resolved.entry, "ore-extensions.ts");
        assert_eq!(
            resolved
                .files
                .iter()
                .map(|file| file.path.as_str())
                .collect::<Vec<_>>(),
            vec!["ore-devex.ts", "ore-extensions.ts"]
        );
        assert_eq!(
            resolved.input_kind,
            Some(ExtensionsInputKind::StackManifest)
        );
        assert_eq!(resolved.input_hash.as_deref(), Some("ts-staged-pin-hash"));
        assert_eq!(resolved.sdk_range.as_deref(), Some("^0.2.0 || ^0.3.0"));
    }

    #[test]
    fn typescript_hosted_resolution_ignores_stale_output_dir_manifest() {
        let output_dir = std::env::temp_dir().join(format!(
            "a4-ts-ext-hosted-authoritative-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&output_dir);
        write_bundle_dir(&output_dir, &ts_bundle_artifact("stale-hosted-hash"));

        let resolved = resolve_extensions_artifact(
            None,
            &layout_in(&output_dir, "ore"),
            None,
            OutputExtensionsFallback::Ignore,
        )
        .expect("hosted resolution should succeed");
        let _ = fs::remove_dir_all(&output_dir);

        assert!(resolved.is_none());
    }

    /// Mirrors `rust_resolution_rejects_typescript_bundles` for the staged
    /// output-dir rung: the Rust path hard-errors on a wrong-language
    /// manifest found in the output dir, so the TypeScript path does the
    /// symmetric thing for a Rust manifest — error, not silent skip. Once
    /// the manifest is gone, entry-file inference is reachable again.
    #[test]
    fn typescript_resolution_rejects_rust_output_dir_manifest_then_falls_back_to_entry() {
        let output_dir =
            std::env::temp_dir().join(format!("a4-ts-ext-rust-outdir-{}", std::process::id()));
        let _ = fs::remove_dir_all(&output_dir);
        write_bundle_dir(&output_dir, &rust_test_artifact("hash-1"));
        fs::write(output_dir.join("ore-extensions.ts"), "export default {};").expect("entry file");

        let error = resolve_extensions_artifact(
            None,
            &layout_in(&output_dir, "ore"),
            None,
            OutputExtensionsFallback::Reuse,
        )
        .expect_err("Rust manifest in output dir must be rejected for TypeScript generation");
        assert!(error.to_string().contains("declares language 'rust'"));

        fs::remove_file(output_dir.join("extensions.json")).expect("remove manifest");
        let resolved = resolve_extensions_artifact(
            None,
            &layout_in(&output_dir, "ore"),
            None,
            OutputExtensionsFallback::Reuse,
        )
        .expect("entry inference should resolve")
        .expect("entry inference should be present");
        let _ = fs::remove_dir_all(&output_dir);

        assert_eq!(resolved.entry, "ore-extensions.ts");
        assert_eq!(resolved.input_hash, None);
    }

    #[test]
    fn typescript_resolution_infers_entry_file_when_no_manifest_present() {
        let output_dir =
            std::env::temp_dir().join(format!("a4-ts-ext-infer-{}", std::process::id()));
        let _ = fs::remove_dir_all(&output_dir);
        fs::create_dir_all(&output_dir).expect("output directory");
        fs::write(output_dir.join("ore-extensions.ts"), "export default {};").expect("entry file");

        let resolved = resolve_extensions_artifact(
            None,
            &layout_in(&output_dir, "ore"),
            None,
            OutputExtensionsFallback::Reuse,
        )
        .expect("entry inference should resolve")
        .expect("entry inference should be present");
        let _ = fs::remove_dir_all(&output_dir);

        assert_eq!(resolved.entry, "ore-extensions.ts");
        assert_eq!(
            resolved
                .files
                .iter()
                .map(|file| file.path.as_str())
                .collect::<Vec<_>>(),
            vec!["ore-extensions.ts"]
        );
        assert_eq!(resolved.input_kind, None);
        assert_eq!(resolved.input_hash, None);
        assert_eq!(resolved.sdk_range, None);
    }

    #[test]
    fn typescript_resolution_prefers_hosted_artifact_over_output_dir_manifest() {
        let output_dir =
            std::env::temp_dir().join(format!("a4-ts-ext-hosted-wins-{}", std::process::id()));
        let _ = fs::remove_dir_all(&output_dir);
        write_bundle_dir(&output_dir, &ts_bundle_artifact("staged-hash"));
        let hosted = test_artifact(ExtensionsInputKind::StackManifest, "hosted-hash");

        let resolved = resolve_extensions_artifact(
            None,
            &layout_in(&output_dir, "ore"),
            Some(&hosted),
            OutputExtensionsFallback::Ignore,
        )
        .expect("hosted bundle should resolve")
        .expect("hosted bundle should be present");
        let _ = fs::remove_dir_all(&output_dir);

        assert_eq!(resolved.input_hash.as_deref(), Some("hosted-hash"));
        assert_eq!(resolved.entry, "index.ts");
    }

    /// The `examples/ore-typescript/src/generated` dir shape (manifest
    /// without `language`, entry + helper files) survives a no-flag
    /// regeneration: resolution reuses the staged manifest and restaging
    /// rewrites `extensions.json` byte-identically with pins intact.
    #[test]
    fn typescript_output_dir_manifest_survives_no_flag_regeneration_byte_stable() {
        let staged_manifest = r#"{
  "entry": "ore-stack-extensions.ts",
  "files": [
    "ore-devex.ts",
    "ore-stack-extensions.ts"
  ],
  "inputKind": "stack-manifest",
  "inputHash": "arete:h1:stack-manifest:sha256:edd1ffe8ef2c26232c1440f20625b8834b8c4d4e63250136ce62bcc38609f84a",
  "sdkRange": "^0.2.0 || ^0.3.0"
}"#;
        let output_dir =
            std::env::temp_dir().join(format!("a4-ts-ext-roundtrip-{}", std::process::id()));
        let _ = fs::remove_dir_all(&output_dir);
        fs::create_dir_all(&output_dir).expect("output directory");
        fs::write(output_dir.join("extensions.json"), staged_manifest).expect("manifest");
        fs::write(
            output_dir.join("ore-devex.ts"),
            "export const helper = 1;\n",
        )
        .expect("helper file");
        fs::write(
            output_dir.join("ore-stack-extensions.ts"),
            "export * from './ore-devex.js';\nexport default {};\n",
        )
        .expect("entry file");

        let resolved = resolve_extensions_artifact(
            None,
            &layout_in(&output_dir, "ore-stack"),
            None,
            OutputExtensionsFallback::Reuse,
        )
        .expect("staged manifest should resolve")
        .expect("staged manifest should be present");
        assert_eq!(
            serde_json::to_string_pretty(&resolved.manifest()).unwrap(),
            staged_manifest
        );

        let input_pin = ResolvedExtensionsInputPin {
            kind: ExtensionsInputKind::StackManifest,
            hash: "arete:h1:stack-manifest:sha256:edd1ffe8ef2c26232c1440f20625b8834b8c4d4e63250136ce62bcc38609f84a"
                .to_string(),
        };
        stage_extensions_artifact(&resolved, &output_dir, &input_pin)
            .expect("restaging the reused bundle should succeed");
        let restaged =
            fs::read_to_string(output_dir.join("extensions.json")).expect("restaged manifest");
        let _ = fs::remove_dir_all(&output_dir);
        assert_eq!(restaged, staged_manifest);
    }

    #[test]
    fn rust_resolution_ignores_hosted_bundles_without_rust_language() {
        let output_dir =
            std::env::temp_dir().join(format!("a4-rust-ext-hosted-{}", std::process::id()));
        let _ = fs::remove_dir_all(&output_dir);
        let hosted_typescript = test_artifact(ExtensionsInputKind::StackManifest, "hash-1");

        let resolved = resolve_rust_extensions_artifact(
            None,
            Some(&hosted_typescript),
            &output_dir,
            "ore",
            OutputExtensionsFallback::Ignore,
        )
        .expect("hosted TypeScript bundle should be skipped, not fatal");
        assert!(resolved.is_none());

        let hosted_rust = rust_test_artifact("hash-2");
        let resolved = resolve_rust_extensions_artifact(
            None,
            Some(&hosted_rust),
            &output_dir,
            "ore",
            OutputExtensionsFallback::Ignore,
        )
        .expect("hosted Rust bundle should resolve")
        .expect("hosted Rust bundle should be present");
        assert_eq!(resolved.input_hash.as_deref(), Some("hash-2"));
    }

    #[test]
    fn rust_extension_wiring_orders_entry_last_and_requires_flat_rs_files() {
        let artifact = rust_test_artifact("hash-1");
        let (modules, entry) = rust_extension_wiring(&artifact).expect("wiring should resolve");
        assert_eq!(modules, vec!["devex".to_string(), "extensions".to_string()]);
        assert_eq!(entry, "extensions");

        let mut nested = rust_test_artifact("hash-1");
        nested.files[0].path = "nested/devex.rs".to_string();
        assert!(rust_extension_wiring(&nested)
            .unwrap_err()
            .to_string()
            .contains("flat .rs files"));

        let mut non_rs = rust_test_artifact("hash-1");
        non_rs.files[0].path = "devex.ts".to_string();
        assert!(rust_extension_wiring(&non_rs)
            .unwrap_err()
            .to_string()
            .contains(".rs files"));
    }

    #[test]
    fn stage_rust_extensions_artifact_writes_bundle_and_rejects_pin_mismatch() {
        let output_dir =
            std::env::temp_dir().join(format!("a4-rust-ext-stage-{}", std::process::id()));
        let _ = fs::remove_dir_all(&output_dir);
        fs::create_dir_all(&output_dir).expect("output directory");
        let hash = format!("arete:h1:stack-manifest:sha256:{}", "44".repeat(32));
        let artifact = rust_test_artifact(&hash);
        let input_pin = ResolvedExtensionsInputPin {
            kind: ExtensionsInputKind::StackManifest,
            hash: hash.clone(),
        };

        stage_rust_extensions_artifact(&artifact, &output_dir, &input_pin)
            .expect("matching pin should stage");
        let manifest_json =
            fs::read_to_string(output_dir.join("extensions.json")).expect("staged manifest");
        assert!(output_dir.join("devex.rs").exists());
        assert!(output_dir.join("extensions.rs").exists());
        assert!(manifest_json.contains("\"language\": \"rust\""));

        let mismatched_pin = ResolvedExtensionsInputPin {
            kind: ExtensionsInputKind::StackManifest,
            hash: format!("arete:h1:stack-manifest:sha256:{}", "55".repeat(32)),
        };
        let error = stage_rust_extensions_artifact(&artifact, &output_dir, &mismatched_pin)
            .expect_err("pin mismatch must be a hard error");
        assert!(error.to_string().contains("extensions input hash mismatch"));

        let mut non_rs = artifact.clone();
        non_rs.files[0].path = "devex.ts".to_string();
        assert!(stage_rust_extensions_artifact(&non_rs, &output_dir, &input_pin).is_err());

        let _ = fs::remove_dir_all(&output_dir);
    }

    #[test]
    fn rust_provenance_lists_generated_and_staged_artifacts_with_prefix() {
        let hash = format!("arete:h1:stack-manifest:sha256:{}", "66".repeat(32));
        let input_pin = ResolvedExtensionsInputPin {
            kind: ExtensionsInputKind::StackManifest,
            hash: hash.clone(),
        };
        let artifact = rust_test_artifact(&hash);

        let manifest = build_sdk_provenance_manifest_from_artifacts(
            BTreeSet::from([
                "Cargo.toml".to_string(),
                "src/lib.rs".to_string(),
                "src/types.rs".to_string(),
                "src/entity.rs".to_string(),
            ]),
            "src/",
            &input_pin,
            Some(&artifact),
        )
        .expect("rust provenance should build");

        assert_eq!(manifest.schema_version, 2);
        assert_eq!(manifest.input.hash, hash);
        assert_eq!(
            manifest.extensions.as_ref().unwrap().content_sha256,
            extensions_artifact_hash(&artifact)
        );
        assert_eq!(
            manifest.artifacts,
            vec![
                "Cargo.toml",
                "src/devex.rs",
                "src/entity.rs",
                "src/extensions.json",
                "src/extensions.rs",
                "src/lib.rs",
                "src/types.rs",
            ]
        );
    }

    #[test]
    fn discover_arete_sdk_crate_version_only_returns_exact_versions() {
        let root = std::env::temp_dir().join(format!("a4-rust-sdk-version-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let nested = root.join("generated/ore");
        fs::create_dir_all(&nested).expect("nested output directory");

        fs::write(
            root.join("Cargo.toml"),
            "[dependencies]\narete-sdk = { package = \"arete-a4-sdk\", version = \"0\" }\n",
        )
        .unwrap();
        assert_eq!(discover_arete_sdk_crate_version(&nested), None);

        fs::write(
            root.join("Cargo.toml"),
            "[dependencies]\narete-a4-sdk = \"0.4.1\"\n",
        )
        .unwrap();
        assert_eq!(
            discover_arete_sdk_crate_version(&nested).as_deref(),
            Some("0.4.1")
        );
        let _ = fs::remove_dir_all(&root);
    }

    fn python_test_artifact(hash: &str) -> ResolvedExtensionsArtifact {
        ResolvedExtensionsArtifact {
            entry: "extensions.py".to_string(),
            files: vec![
                ResolvedExtensionsFile {
                    path: "devex.py".to_string(),
                    contents: "def helper():\n    return None\n".to_string(),
                },
                ResolvedExtensionsFile {
                    path: "extensions.py".to_string(),
                    contents: "from .devex import *  # noqa: F401,F403\n".to_string(),
                },
            ],
            input_kind: Some(ExtensionsInputKind::StackManifest),
            input_hash: Some(hash.to_string()),
            sdk_range: None,
            language: Some(EXTENSIONS_LANGUAGE_PYTHON.to_string()),
            sdk_extension_hash: None,
            sdk_output_tree_hash: None,
            program_extension_bindings: vec![],
        }
    }

    #[test]
    fn python_extensions_manifest_language_roundtrips() {
        let staged = r#"{
  "entry": "extensions.py",
  "files": [
    "devex.py",
    "extensions.py"
  ],
  "inputKind": "stack-manifest",
  "inputHash": "arete:h1:stack-manifest:sha256:edd1ffe8ef2c26232c1440f20625b8834b8c4d4e63250136ce62bcc38609f84a",
  "sdkRange": "^0.2.0 || ^0.3.0",
  "language": "python"
}"#;

        let manifest: ExtensionsManifest =
            serde_json::from_str(staged).expect("Python manifest should parse");
        assert_eq!(
            manifest.language.as_deref(),
            Some(EXTENSIONS_LANGUAGE_PYTHON)
        );
        let reserialized = serde_json::to_string_pretty(&manifest).unwrap();
        assert_eq!(reserialized, staged);
    }

    /// A hosted bundle fetched with `?language=python` carries
    /// `language: "python"` in its manifest; the resolved artifact must keep
    /// that marker so it reaches the Python extensions rung — and so the
    /// TypeScript rung keeps rejecting it if it ever leaks there.
    #[test]
    fn remote_stack_routes_python_language_hosted_extensions_to_the_python_rung() {
        let mut install = registry_stack_install("ore", "OreStream");
        install.extensions = Some(RegistrySdkExtensionArtifact {
            artifact_hash: "python-extension-hash".to_string(),
            sdk_extension_hash: None,
            sdk_output_tree_hash: None,
            manifest: crate::api_client::RegistrySdkExtensionManifest {
                entry: "extensions.py".to_string(),
                files: vec!["extensions.py".to_string()],
                input_kind: Some(RegistrySdkExtensionInputKind::StackManifest),
                input_hash: Some(install.stack_manifest_hash.clone()),
                sdk_range: None,
                language: Some(EXTENSIONS_LANGUAGE_PYTHON.to_string()),
            },
            files: BTreeMap::from([(
                "extensions.py".to_string(),
                "def devex():\n    return None\n".to_string(),
            )]),
            created_at: "2026-08-04T00:00:00Z".to_string(),
        });

        let remote = remote_stack_install(install).expect("remote stack should resolve");
        let hosted = remote
            .hosted_extensions
            .as_ref()
            .expect("hosted extensions should be resolved");
        assert_eq!(hosted.language.as_deref(), Some(EXTENSIONS_LANGUAGE_PYTHON));

        // Python rung accepts the hosted artifact...
        let output_dir =
            std::env::temp_dir().join(format!("a4-python-hosted-route-{}", std::process::id()));
        let _ = fs::remove_dir_all(&output_dir);
        let resolved = resolve_python_extensions_artifact(
            None,
            Some(hosted),
            &output_dir,
            "ore",
            OutputExtensionsFallback::Ignore,
        )
        .expect("hosted Python bundle should resolve")
        .expect("hosted Python bundle should be present");
        assert_eq!(resolved.entry, "extensions.py");

        // ...the Rust rung skips it (hosted wrong-language bundles are
        // non-fatal there, mirroring the TypeScript-bundle behaviour)...
        let skipped = resolve_rust_extensions_artifact(
            None,
            Some(hosted),
            &output_dir,
            "ore",
            OutputExtensionsFallback::Ignore,
        )
        .expect("hosted Python bundle should be skipped by the Rust rung");
        assert!(skipped.is_none());

        // ...while the TypeScript rung rejects it outright.
        let error = resolve_extensions_artifact(
            None,
            &layout("ore"),
            Some(hosted),
            OutputExtensionsFallback::Ignore,
        )
        .expect_err("TypeScript generation must reject a Python hosted bundle");
        assert!(error.to_string().contains("declares language 'python'"));
    }

    #[test]
    fn python_resolution_prefers_explicit_bundle_over_output_dir_manifest() {
        let root =
            std::env::temp_dir().join(format!("a4-python-ext-explicit-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let bundle_dir = root.join("bundle");
        let output_dir = root.join("out");
        let explicit = python_test_artifact("explicit-hash");
        let staged = python_test_artifact("staged-hash");
        write_bundle_dir(&bundle_dir, &explicit);
        write_bundle_dir(&output_dir, &staged);

        let resolved = resolve_python_extensions_artifact(
            Some(&bundle_dir),
            None,
            &output_dir,
            "ore",
            OutputExtensionsFallback::Ignore,
        )
        .expect("explicit bundle should resolve")
        .expect("explicit bundle should be present");
        let _ = fs::remove_dir_all(&root);

        assert_eq!(resolved.input_hash.as_deref(), Some("explicit-hash"));
        assert_eq!(
            resolved.language.as_deref(),
            Some(EXTENSIONS_LANGUAGE_PYTHON)
        );
    }

    #[test]
    fn python_resolution_accepts_single_entry_file() {
        let root =
            std::env::temp_dir().join(format!("a4-python-ext-single-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("bundle root");
        fs::write(
            root.join("extensions.py"),
            "def devex():\n    return None\n",
        )
        .expect("entry file");

        let resolved = resolve_python_extensions_artifact(
            Some(&root.join("extensions.py")),
            None,
            &root.join("missing-output"),
            "ore",
            OutputExtensionsFallback::Ignore,
        )
        .expect("single-entry bundle should resolve")
        .expect("single-entry bundle should be present");
        let _ = fs::remove_dir_all(&root);

        assert_eq!(resolved.entry, "extensions.py");
        assert_eq!(
            resolved
                .files
                .iter()
                .map(|file| file.path.as_str())
                .collect::<Vec<_>>(),
            vec!["extensions.py"]
        );
        assert_eq!(resolved.input_hash, None);
        assert_eq!(
            resolved.language.as_deref(),
            Some(EXTENSIONS_LANGUAGE_PYTHON)
        );
    }

    /// Regression guard for the sync sharp edge: `a4 sdk sync --python`
    /// against an output dir holding a full staged bundle must reuse the
    /// manifest with its input pins and helper files intact, instead of
    /// silently unpinning them.
    #[test]
    fn python_resolution_reuses_output_dir_manifest_and_preserves_pins() {
        let output_dir =
            std::env::temp_dir().join(format!("a4-python-ext-outdir-{}", std::process::id()));
        let _ = fs::remove_dir_all(&output_dir);
        let staged = python_test_artifact("staged-pin-hash");
        write_bundle_dir(&output_dir, &staged);

        let resolved = resolve_python_extensions_artifact(
            None,
            None,
            &output_dir,
            "ore",
            OutputExtensionsFallback::Reuse,
        )
        .expect("staged manifest should resolve")
        .expect("staged manifest should be present");
        let _ = fs::remove_dir_all(&output_dir);

        assert_eq!(
            resolved.input_kind,
            Some(ExtensionsInputKind::StackManifest)
        );
        assert_eq!(resolved.input_hash.as_deref(), Some("staged-pin-hash"));
        assert_eq!(resolved.entry, "extensions.py");
    }

    #[test]
    fn python_hosted_resolution_ignores_stale_output_dir_manifest() {
        let output_dir = std::env::temp_dir().join(format!(
            "a4-python-ext-hosted-authoritative-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&output_dir);
        write_bundle_dir(&output_dir, &python_test_artifact("stale-python-hash"));

        let resolved = resolve_python_extensions_artifact(
            None,
            None,
            &output_dir,
            "ore",
            OutputExtensionsFallback::Ignore,
        )
        .expect("hosted resolution should succeed");
        let _ = fs::remove_dir_all(&output_dir);

        assert!(resolved.is_none());
    }

    #[test]
    fn python_resolution_returns_none_without_sources() {
        let output_dir =
            std::env::temp_dir().join(format!("a4-python-ext-none-{}", std::process::id()));
        let _ = fs::remove_dir_all(&output_dir);

        let resolved = resolve_python_extensions_artifact(
            None,
            None,
            &output_dir,
            "ore",
            OutputExtensionsFallback::Reuse,
        )
        .expect("empty resolution should succeed");

        assert!(resolved.is_none());
    }

    #[test]
    fn python_resolution_rejects_typescript_and_rust_bundles() {
        let root =
            std::env::temp_dir().join(format!("a4-python-ext-reject-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let typescript_dir = root.join("typescript");
        let mut typescript = python_test_artifact("hash-1");
        typescript.language = Some(EXTENSIONS_LANGUAGE_TYPESCRIPT.to_string());
        write_bundle_dir(&typescript_dir, &typescript);

        let error = resolve_python_extensions_artifact(
            Some(&typescript_dir),
            None,
            &root.join("missing-output"),
            "ore",
            OutputExtensionsFallback::Ignore,
        )
        .expect_err("TypeScript bundle must be rejected for Python generation");
        assert!(error.to_string().contains("declares language 'typescript'"));

        let rust_dir = root.join("rust");
        write_bundle_dir(&rust_dir, &rust_test_artifact("hash-2"));
        let error = resolve_python_extensions_artifact(
            Some(&rust_dir),
            None,
            &root.join("missing-output"),
            "ore",
            OutputExtensionsFallback::Ignore,
        )
        .expect_err("Rust bundle must be rejected for Python generation");
        let _ = fs::remove_dir_all(&root);

        assert!(error.to_string().contains("declares language 'rust'"));
    }

    #[test]
    fn typescript_resolution_rejects_python_bundles() {
        let bundle_dir =
            std::env::temp_dir().join(format!("a4-ts-ext-python-reject-{}", std::process::id()));
        let _ = fs::remove_dir_all(&bundle_dir);
        write_bundle_dir(&bundle_dir, &python_test_artifact("hash-1"));

        let error = resolve_extensions_artifact(
            Some(&bundle_dir),
            &layout("ore"),
            None,
            OutputExtensionsFallback::Ignore,
        )
        .expect_err("Python bundle must be rejected for TypeScript generation");
        let _ = fs::remove_dir_all(&bundle_dir);

        assert!(error.to_string().contains("declares language 'python'"));
    }

    #[test]
    fn python_resolution_ignores_hosted_bundles_without_python_language() {
        let output_dir =
            std::env::temp_dir().join(format!("a4-python-ext-hosted-{}", std::process::id()));
        let _ = fs::remove_dir_all(&output_dir);
        let hosted_typescript = test_artifact(ExtensionsInputKind::StackManifest, "hash-1");

        let resolved = resolve_python_extensions_artifact(
            None,
            Some(&hosted_typescript),
            &output_dir,
            "ore",
            OutputExtensionsFallback::Ignore,
        )
        .expect("hosted TypeScript bundle should be skipped, not fatal");
        assert!(resolved.is_none());

        let hosted_rust = rust_test_artifact("hash-2");
        let resolved = resolve_python_extensions_artifact(
            None,
            Some(&hosted_rust),
            &output_dir,
            "ore",
            OutputExtensionsFallback::Ignore,
        )
        .expect("hosted Rust bundle should be skipped, not fatal");
        assert!(resolved.is_none());

        let hosted_python = python_test_artifact("hash-3");
        let resolved = resolve_python_extensions_artifact(
            None,
            Some(&hosted_python),
            &output_dir,
            "ore",
            OutputExtensionsFallback::Ignore,
        )
        .expect("hosted Python bundle should resolve")
        .expect("hosted Python bundle should be present");
        assert_eq!(resolved.input_hash.as_deref(), Some("hash-3"));
    }

    #[test]
    fn python_extension_wiring_orders_entry_last_and_requires_flat_py_files() {
        let artifact = python_test_artifact("hash-1");
        let (modules, entry) = python_extension_wiring(&artifact).expect("wiring should resolve");
        assert_eq!(modules, vec!["devex".to_string(), "extensions".to_string()]);
        assert_eq!(entry, "extensions");

        // File stems are normalized through `python_module_name` so hyphens
        // become underscores in the generated `from . import <stem>` wiring.
        let mut hyphenated = python_test_artifact("hash-1");
        hyphenated.files[0].path = "ore-devex.py".to_string();
        let (modules, _) = python_extension_wiring(&hyphenated).expect("wiring should resolve");
        assert_eq!(
            modules,
            vec!["ore_devex".to_string(), "extensions".to_string()]
        );

        let mut nested = python_test_artifact("hash-1");
        nested.files[0].path = "nested/devex.py".to_string();
        assert!(python_extension_wiring(&nested)
            .unwrap_err()
            .to_string()
            .contains("flat .py files"));

        let mut non_py = python_test_artifact("hash-1");
        non_py.files[0].path = "devex.rs".to_string();
        assert!(python_extension_wiring(&non_py)
            .unwrap_err()
            .to_string()
            .contains(".py files"));
    }

    #[test]
    fn stage_python_extensions_artifact_writes_bundle_and_rejects_pin_mismatch() {
        let output_dir =
            std::env::temp_dir().join(format!("a4-python-ext-stage-{}", std::process::id()));
        let _ = fs::remove_dir_all(&output_dir);
        fs::create_dir_all(&output_dir).expect("output directory");
        let hash = format!("arete:h1:stack-manifest:sha256:{}", "44".repeat(32));
        let artifact = python_test_artifact(&hash);
        let input_pin = ResolvedExtensionsInputPin {
            kind: ExtensionsInputKind::StackManifest,
            hash: hash.clone(),
        };

        stage_python_extensions_artifact(&artifact, &output_dir, &input_pin)
            .expect("matching pin should stage");
        let manifest_json =
            fs::read_to_string(output_dir.join("extensions.json")).expect("staged manifest");
        assert!(output_dir.join("devex.py").exists());
        assert!(output_dir.join("extensions.py").exists());
        assert!(manifest_json.contains("\"language\": \"python\""));

        let mismatched_pin = ResolvedExtensionsInputPin {
            kind: ExtensionsInputKind::StackManifest,
            hash: format!("arete:h1:stack-manifest:sha256:{}", "55".repeat(32)),
        };
        let error = stage_python_extensions_artifact(&artifact, &output_dir, &mismatched_pin)
            .expect_err("pin mismatch must be a hard error");
        assert!(error.to_string().contains("extensions input hash mismatch"));

        let mut non_py = artifact.clone();
        non_py.files[0].path = "devex.rs".to_string();
        assert!(stage_python_extensions_artifact(&non_py, &output_dir, &input_pin).is_err());

        let _ = fs::remove_dir_all(&output_dir);
    }

    #[test]
    fn python_provenance_lists_generated_and_staged_artifacts_with_prefix() {
        let hash = format!("arete:h1:stack-manifest:sha256:{}", "66".repeat(32));
        let input_pin = ResolvedExtensionsInputPin {
            kind: ExtensionsInputKind::StackManifest,
            hash: hash.clone(),
        };
        let artifact = python_test_artifact(&hash);

        let manifest = build_sdk_provenance_manifest_from_artifacts(
            BTreeSet::from([
                "pyproject.toml".to_string(),
                "ore_stack/__init__.py".to_string(),
                "ore_stack/models.py".to_string(),
                "ore_stack/views.py".to_string(),
            ]),
            "ore_stack/",
            &input_pin,
            Some(&artifact),
        )
        .expect("python provenance should build");

        assert_eq!(manifest.schema_version, 2);
        assert_eq!(manifest.input.hash, hash);
        assert_eq!(
            manifest.extensions.as_ref().unwrap().content_sha256,
            extensions_artifact_hash(&artifact)
        );
        assert_eq!(
            manifest.artifacts,
            vec![
                "ore_stack/__init__.py",
                "ore_stack/devex.py",
                "ore_stack/extensions.json",
                "ore_stack/extensions.py",
                "ore_stack/models.py",
                "ore_stack/views.py",
                "pyproject.toml",
            ]
        );
    }

    /// End-to-end single-live Python generation twin of the Rust pipeline:
    /// compile from local V2 artifacts, stage an explicit devex bundle,
    /// wire it through the generated `__init__.py`, and record provenance
    /// with the package-mode `<module>/` prefix.
    #[test]
    fn local_single_live_python_generation_stages_extensions_and_provenance() {
        use arete_hash::{CanonicalIdlDocument, ProgramSpecV1};

        let directory =
            std::env::temp_dir().join(format!("a4-python-single-live-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        let document = CanonicalIdlDocument::parse(
            br#"{"address":"11111111111111111111111111111111","metadata":{"name":"system","version":"1.0.0","spec":"0.1.0"},"instructions":[],"accounts":[],"types":[],"events":[],"errors":[]}"#,
            None,
        )
        .unwrap();
        let program =
            arete_artifacts::ProgramSpecArtifact::new(ProgramSpecV1::from_document(&document))
                .unwrap();
        let live = arete_artifacts::live_spec_v2(
            std::slice::from_ref(&program),
            vec![arete_artifacts::PortableEntity::new(
                "OreState",
                "id.address",
            )],
            Vec::new(),
        )
        .unwrap();
        let live_specs = vec![("live".to_string(), live)];
        let stack_manifest = arete_artifacts::compose_stack_manifest_v2(
            "OreStream",
            std::slice::from_ref(&program),
            live_specs
                .iter()
                .map(|(alias, live)| (alias.clone(), live))
                .collect(),
            vec![arete_artifacts::SelectedViewV2 {
                live_alias: "live".to_string(),
                view_id: "OreState/list".to_string(),
            }],
        )
        .unwrap();
        let manifest_hash = stack_manifest.artifact_hash.to_string();
        let source = ResolvedStackSource::LocalArtifacts(Box::new(LocalArtifactStack {
            manifest_path: directory.join("OreStream.stack-manifest.json"),
            manifest_hash: manifest_hash.clone(),
            program_specs: vec![program],
            live_specs,
            stack_manifest,
        }));

        let bundle_dir = directory.join("bundle");
        write_bundle_dir(&bundle_dir, &python_test_artifact(&manifest_hash));

        let output_dir = directory.join("generated/ore-py");
        let stack_spec = source.load_stack_spec(true).unwrap();
        generate_python_stack_sdk(
            &source,
            stack_spec,
            &output_dir,
            "ore-stack",
            false,
            Some("wss://ore.example.test/ws".to_string()),
            Some(&bundle_dir),
        )
        .expect("python generation should succeed");

        assert!(output_dir.join("pyproject.toml").is_file());
        let init_py = fs::read_to_string(output_dir.join("ore_stack/__init__.py")).unwrap();
        assert!(init_py.contains("from . import devex"));
        assert!(init_py.contains("from .extensions import *"));
        assert!(output_dir.join("ore_stack/models.py").is_file());
        assert!(output_dir.join("ore_stack/views.py").is_file());
        assert!(output_dir.join("ore_stack/devex.py").is_file());
        assert!(output_dir.join("ore_stack/extensions.py").is_file());
        assert!(output_dir.join("ore_stack/extensions.json").is_file());

        let provenance =
            fs::read_to_string(output_dir.join(SDK_PROVENANCE_FILE)).expect("provenance");
        let SdkProvenanceManifest::V2(manifest) =
            parse_sdk_provenance_manifest(&provenance).expect("provenance should parse")
        else {
            panic!("generation must write provenance V2");
        };
        assert_eq!(manifest.input.kind, ExtensionsInputKind::StackManifest);
        assert_eq!(manifest.input.hash, manifest_hash);
        assert!(manifest
            .artifacts
            .contains(&"ore_stack/extensions.py".to_string()));
        assert!(manifest
            .artifacts
            .contains(&"ore_stack/extensions.json".to_string()));
        assert!(manifest.artifacts.contains(&"pyproject.toml".to_string()));

        // A pinless re-read of the staged output keeps the manifest intact
        // (the sync sharp edge): resolution against the module dir reuses
        // the staged bundle with its pins.
        let resolved = resolve_python_extensions_artifact(
            None,
            None,
            &output_dir.join("ore_stack"),
            "ore",
            OutputExtensionsFallback::Reuse,
        )
        .expect("staged manifest should resolve")
        .expect("staged manifest should be present");
        assert_eq!(resolved.input_hash.as_deref(), Some(manifest_hash.as_str()));

        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn discover_arete_sdk_python_version_only_returns_exact_pins() {
        let root =
            std::env::temp_dir().join(format!("a4-python-sdk-version-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let nested = root.join("generated/ore-py");
        fs::create_dir_all(&nested).expect("nested output directory");

        fs::write(
            root.join("pyproject.toml"),
            "[project]\ndependencies = [\"arete-sdk>=0.4\"]\n",
        )
        .unwrap();
        assert_eq!(discover_arete_sdk_python_version(&nested), None);

        fs::write(
            root.join("pyproject.toml"),
            "[project]\ndependencies = [\"arete-sdk==0.4.1\"]\n",
        )
        .unwrap();
        assert_eq!(
            discover_arete_sdk_python_version(&nested).as_deref(),
            Some("0.4.1")
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn normalize_extension_relative_path_rejects_parent_segments() {
        let error = normalize_extension_relative_path("../secrets.ts").unwrap_err();
        assert!(error
            .to_string()
            .contains("must be a normalized relative path"));
    }

    #[test]
    fn index_extension_fallback_does_not_capture_sibling_typescript_files() {
        let source_dir =
            std::env::temp_dir().join(format!("a4-index-extensions-{}", std::process::id()));
        let _ = fs::remove_dir_all(&source_dir);
        fs::create_dir_all(&source_dir).expect("temp extensions directory should be created");
        fs::write(source_dir.join("index.ts"), "export default {};")
            .expect("index extension should be written");
        fs::write(source_dir.join("stale.ts"), "this is not valid TypeScript")
            .expect("stale extension should be written");

        let artifact = infer_extensions_artifact_from_entry(&source_dir.join("index.ts"))
            .expect("index extension should resolve");

        let _ = fs::remove_dir_all(&source_dir);
        assert_eq!(
            artifact
                .files
                .iter()
                .map(|file| file.path.as_str())
                .collect::<Vec<_>>(),
            vec!["index.ts"]
        );
    }

    #[test]
    fn named_extension_fallback_does_not_capture_prefixed_sibling_files() {
        let source_dir =
            std::env::temp_dir().join(format!("a4-named-extensions-{}", std::process::id()));
        let _ = fs::remove_dir_all(&source_dir);
        fs::create_dir_all(&source_dir).expect("temp extensions directory should be created");
        fs::write(source_dir.join("ore-extensions.ts"), "export default {};")
            .expect("named extension should be written");
        fs::write(
            source_dir.join("ore-stale.ts"),
            "this is not valid TypeScript",
        )
        .expect("stale extension should be written");

        let artifact = infer_extensions_artifact_from_entry(&source_dir.join("ore-extensions.ts"))
            .expect("named extension should resolve");

        let _ = fs::remove_dir_all(&source_dir);
        assert_eq!(
            artifact
                .files
                .iter()
                .map(|file| file.path.as_str())
                .collect::<Vec<_>>(),
            vec!["ore-extensions.ts"]
        );
    }

    #[test]
    fn resolved_extensions_artifact_from_registry_preserves_manifest_and_bindings() {
        let artifact = resolved_extensions_artifact_from_registry(&RegistrySdkExtensionArtifact {
            artifact_hash: "artifact-1".to_string(),
            sdk_extension_hash: Some("sdk-extension-1".to_string()),
            sdk_output_tree_hash: Some("sdk-output-tree-1".to_string()),
            manifest: crate::api_client::RegistrySdkExtensionManifest {
                entry: "./index.ts".to_string(),
                files: vec!["index.ts".to_string()],
                input_kind: Some(RegistrySdkExtensionInputKind::ProgramIdl),
                input_hash: Some("idl-hash".to_string()),
                sdk_range: Some("^0.1.5".to_string()),
                language: None,
            },
            files: BTreeMap::from([(
                "index.ts".to_string(),
                "export const foo = defineProgramExtensions<typeof CORE.programs.bar>()({});"
                    .to_string(),
            )]),
            created_at: "2026-07-08T00:00:00Z".to_string(),
        })
        .expect("registry artifact should resolve");

        assert_eq!(artifact.entry, "index.ts");
        assert_eq!(artifact.manifest().input_hash.as_deref(), Some("idl-hash"));
        assert_eq!(
            artifact.sdk_extension_hash.as_deref(),
            Some("sdk-extension-1")
        );
        assert_eq!(
            artifact.sdk_output_tree_hash.as_deref(),
            Some("sdk-output-tree-1")
        );
        assert_eq!(artifact.program_extension_bindings.len(), 1);
        assert_eq!(artifact.program_extension_bindings[0].program_key, "bar");
    }

    #[test]
    fn direct_idl_codegen_matches_shared_release_vector_and_keeps_definition_hash() {
        let corpus: serde_json::Value =
            serde_json::from_str(include_str!("../../../test-vectors/hash-v1.json"))
                .expect("vector corpus");
        let vector = corpus["idlVectors"]
            .as_array()
            .unwrap()
            .iter()
            .find(|vector| vector["id"] == "idl-primary")
            .expect("primary IDL vector");
        let source = vector["input"]["data"].as_str().unwrap().as_bytes();
        let identity =
            arete_interpreter::program_sdk::build_oss_program_identity_v1_from_idl_bytes(
                source, None,
            )
            .expect("CLI identity");
        let program = arete_interpreter::typescript::TypeScriptProgramConfig::from(&identity);
        let stack_spec =
            arete_interpreter::program_sdk::build_program_only_stack_spec_from_identity(
                &identity, "Demo",
            );
        let output = arete_interpreter::typescript::compile_program_modules(
            stack_spec,
            Some(arete_interpreter::typescript::TypeScriptStackConfig {
                programs: Some(vec![program]),
                ..Default::default()
            }),
        )
        .expect("program SDK generation");

        assert_eq!(
            identity.release_hash.to_string(),
            vector["expected"]["ossReleaseIdentity"]["hashId"]
        );
        assert!(output
            .stack_definition
            .contains("sdkDefinitionHash: 'arete:h1:sdk-definition:sha256:"));
        assert!(output.stack_definition.contains(&format!(
            "programReleaseHash: \"{}\"",
            identity.release_hash
        )));
        assert!(output
            .stack_definition
            .contains("transport: { kind: 'local-http', endpointSource: 'connect-http-url' }"));
        assert!(!output.stack_definition.contains("path:"));
    }

    #[test]
    fn hosted_program_codegen_preserves_registry_release_binding_and_auth() {
        let source =
            include_bytes!("../../../arete-macros/tests/fixtures/nested-computed.idl.json");
        let identity =
            arete_interpreter::program_sdk::build_oss_program_identity_v1_from_idl_bytes(
                source, None,
            )
            .expect("fixture identity");
        let hosted_release = "arete:h1:program-release:sha256:hosted-not-oss";
        let binding_id = "prb_00000000000000000000000000000001";
        assert_ne!(hosted_release, identity.release_hash.to_string());
        let install = RegistryProgramInstallResponse {
            install_name: "meteora-presale".to_string(),
            display_name: "Meteora Presale".to_string(),
            definition: crate::api_client::RegistryProgramInstallDefinition {
                program_id: identity.program_spec.program_id.clone(),
                program_spec_hash: identity.program_spec_hash.to_string(),
                idl_content_hash: identity.program_spec.idl_content_hash.to_string(),
                normalized_idl_hash: identity.program_spec.normalized_idl_hash.to_string(),
                idl_payload: serde_json::from_slice(source).expect("IDL payload"),
                program_spec: serde_json::to_value(
                    arete_artifacts::ProgramSpecArtifact::new(identity.program_spec.clone())
                        .unwrap(),
                )
                .unwrap(),
                extensions: None,
            },
            release: crate::api_client::RegistryProgramInstallRelease {
                program_release_hash: hosted_release.to_string(),
                program_spec_hash: identity.program_spec_hash.to_string(),
            },
            transport: RegistryProgramInstallTransport::HostedBinding {
                binding: crate::api_client::RegistryProgramInstallBinding {
                    endpoint: "https://reads.example.test/exact/prefix/".to_string(),
                    program_read_binding_id: binding_id.to_string(),
                    auth: serde_json::json!({
                        "sessionEndpoint": "https://api.example.test/exact/ws/sessions",
                        "targetKind": "program-read-binding",
                        "targetId": binding_id
                    }),
                },
            },
            chain_binding: None,
            transaction_binding: None,
        };
        let stack_spec =
            arete_interpreter::program_sdk::build_program_only_stack_spec_from_identity(
                &identity, "Presale",
            );
        let portable_read = program_read_override(&install).expect("portable read config");
        let descriptor = portable_read
            .descriptor
            .as_ref()
            .expect("published programs carry their hosted descriptor");
        assert_eq!(
            descriptor
                .pointer("/transport/kind")
                .and_then(|value| value.as_str()),
            Some("hosted-binding")
        );
        assert_eq!(
            descriptor
                .pointer("/transport/binding/endpoint")
                .and_then(|value| value.as_str()),
            Some("https://reads.example.test/exact/prefix/")
        );
        let output = arete_interpreter::typescript::compile_program_modules(
            stack_spec,
            Some(arete_interpreter::typescript::TypeScriptStackConfig {
                programs: Some(vec![typescript_program_config_from_registry(&install)
                    .expect("registry program config")]),
                ..Default::default()
            }),
        )
        .expect("hosted descriptor should compile");
        let generated = output.stack_definition;
        assert!(generated.contains("sdkDefinitionHash: 'arete:h1:sdk-definition:sha256:"));
        assert!(generated.contains(&format!("programReleaseHash: \"{hosted_release}\"")));
        assert!(!generated.contains(&identity.release_hash.to_string()));
        assert!(generated.contains("kind: 'hosted-binding'"));
        assert!(generated.contains("endpoint: \"https://reads.example.test/exact/prefix/\""));
        assert!(generated.contains(&format!("programReadBindingId: \"{binding_id}\"")));
        assert!(generated.contains("sessionEndpoint"));
        assert!(generated.contains(&format!("\"targetId\":\"{binding_id}\"")));
        assert!(!generated.contains("decoderEngineId"));
    }

    #[test]
    fn hosted_program_transport_validation_rejects_invalid_metadata_before_codegen() {
        let binding_id = "prb_00000000000000000000000000000001";
        let program_spec_hash = format!("arete:h1:program-spec:sha256:{}", "00".repeat(32));
        let install = RegistryProgramInstallResponse {
            install_name: "fixture".to_string(),
            display_name: "Fixture".to_string(),
            definition: crate::api_client::RegistryProgramInstallDefinition {
                program_id: "Program111".to_string(),
                program_spec_hash: program_spec_hash.clone(),
                idl_content_hash: "content-1".to_string(),
                normalized_idl_hash: "normalized-1".to_string(),
                idl_payload: serde_json::json!({}),
                program_spec: serde_json::json!({}),
                extensions: None,
            },
            release: crate::api_client::RegistryProgramInstallRelease {
                program_release_hash: "release-1".to_string(),
                program_spec_hash,
            },
            transport: RegistryProgramInstallTransport::HostedBinding {
                binding: crate::api_client::RegistryProgramInstallBinding {
                    endpoint: "https://reads.example.test".to_string(),
                    program_read_binding_id: binding_id.to_string(),
                    auth: serde_json::json!({
                        "sessionEndpoint": "https://auth.example.test/session",
                        "targetKind": "program-read-binding",
                        "targetId": binding_id
                    }),
                },
            },
            chain_binding: None,
            transaction_binding: None,
        };
        assert!(typescript_program_config_from_registry(&install).is_ok());

        let mut loopback = install.clone();
        let RegistryProgramInstallTransport::HostedBinding { binding } = &mut loopback.transport;
        binding.endpoint = "http://127.0.0.1:8879".to_string();
        binding.auth["sessionEndpoint"] = serde_json::json!("http://localhost:3000/session");
        assert!(typescript_program_config_from_registry(&loopback).is_ok());

        let mut malformed_scheme = install.clone();
        let RegistryProgramInstallTransport::HostedBinding { binding } =
            &mut malformed_scheme.transport;
        binding.endpoint = "ftp://reads.example.test".to_string();
        assert!(typescript_program_config_from_registry(&malformed_scheme).is_err());

        let mut insecure_session = install.clone();
        let RegistryProgramInstallTransport::HostedBinding { binding } =
            &mut insecure_session.transport;
        binding.auth["sessionEndpoint"] = serde_json::json!("http://auth.example.test/session");
        assert!(typescript_program_config_from_registry(&insecure_session).is_err());

        let mut malformed_id = install.clone();
        let RegistryProgramInstallTransport::HostedBinding { binding } =
            &mut malformed_id.transport;
        binding.program_read_binding_id = "prb_too-short".to_string();
        binding.auth["targetId"] = serde_json::json!("prb_too-short");
        assert!(typescript_program_config_from_registry(&malformed_id).is_err());

        let mut wrong_kind = install.clone();
        let RegistryProgramInstallTransport::HostedBinding { binding } = &mut wrong_kind.transport;
        binding.auth["targetKind"] = serde_json::json!("deployment");
        assert!(typescript_program_config_from_registry(&wrong_kind).is_err());

        let mut mismatched_target = install;
        let RegistryProgramInstallTransport::HostedBinding { binding } =
            &mut mismatched_target.transport;
        binding.auth["targetId"] = serde_json::json!("prb_00000000000000000000000000000002");
        assert!(typescript_program_config_from_registry(&mismatched_target).is_err());
    }

    fn hosted_v2_install(
        aliases: &[&str],
    ) -> (RegistryStackInstallResponse, Vec<String>, Vec<String>) {
        use arete_hash::{CanonicalIdlDocument, ProgramSpecV1};

        let addresses = [
            "11111111111111111111111111111111",
            "Vote111111111111111111111111111111111111111",
            "Stake11111111111111111111111111111111111111",
        ];
        let mut program_specs = Vec::new();
        let mut programs = Vec::new();
        let mut live_specs = Vec::new();
        let mut program_endpoints = Vec::new();
        let mut query_endpoints = Vec::new();
        for (index, alias) in aliases.iter().enumerate() {
            let name = format!("program_{alias}");
            let idl = format!(
                r#"{{"address":"{}","metadata":{{"name":"{}","version":"1.0.0","spec":"0.1.0"}},"instructions":[{{"name":"doThing","discriminator":[1,2,3,4,5,6,7,8],"accounts":[],"args":[]}}],"accounts":[{{"name":"Counter","discriminator":[8,7,6,5,4,3,2,1]}}],"types":[{{"name":"Counter","type":{{"kind":"struct","fields":[{{"name":"count","type":"u64"}}]}}}}],"events":[],"errors":[]}}"#,
                addresses[index], name
            );
            let document = CanonicalIdlDocument::parse(idl.as_bytes(), None).unwrap();
            let program =
                arete_artifacts::ProgramSpecArtifact::new(ProgramSpecV1::from_document(&document))
                    .unwrap();
            let live = arete_artifacts::live_spec_v2(
                std::slice::from_ref(&program),
                vec![arete_artifacts::PortableEntity::new(
                    format!("{}State", to_pascal_case(alias)),
                    "id.address",
                )],
                Vec::new(),
            )
            .unwrap();
            let program_endpoint = format!("https://program-{alias}.example.test/read/v1/");
            let query_endpoint = format!("https://query-{alias}.example.test/v1/");
            let binding_id = format!("prb_{:032}", index + 1);
            program_endpoints.push(program_endpoint.clone());
            query_endpoints.push(query_endpoint.clone());
            programs.push(RegistryProgramInstallResponse {
                install_name: name.clone(),
                display_name: name.clone(),
                definition: crate::api_client::RegistryProgramInstallDefinition {
                    program_id: program.payload.program_id.clone(),
                    program_spec_hash: program.artifact_hash.to_string(),
                    idl_content_hash: program.payload.idl_content_hash.to_string(),
                    normalized_idl_hash: program.payload.normalized_idl_hash.to_string(),
                    idl_payload: serde_json::json!({"name": name}),
                    program_spec: serde_json::to_value(&program).unwrap(),
                    extensions: None,
                },
                release: crate::api_client::RegistryProgramInstallRelease {
                    program_release_hash: format!("hosted-release-{alias}"),
                    program_spec_hash: program.artifact_hash.to_string(),
                },
                transport: RegistryProgramInstallTransport::HostedBinding {
                    binding: crate::api_client::RegistryProgramInstallBinding {
                        endpoint: program_endpoint,
                        program_read_binding_id: binding_id.clone(),
                        auth: serde_json::json!({
                            "required": true,
                            "mode": "signed_session",
                            "sessionEndpoint": format!("https://auth.example.test/{alias}"),
                            "targetKind": "program-read-binding",
                            "targetId": binding_id
                        }),
                    },
                },
                chain_binding: None,
                transaction_binding: None,
            });
            live_specs.push(((*alias).to_string(), live));
            program_specs.push(program);
        }
        let stack_manifest = arete_artifacts::compose_stack_manifest_v2(
            "HostedThree",
            &program_specs,
            live_specs
                .iter()
                .map(|(alias, live)| (alias.clone(), live))
                .collect(),
            Vec::new(),
        )
        .unwrap();
        let descriptors = live_specs
            .iter()
            .enumerate()
            .map(|(index, (alias, live))| RegistryLiveSpecInstallDescriptor {
                alias: alias.clone(),
                live_spec_hash: live.artifact_hash.to_string(),
                artifact: serde_json::to_value(live).unwrap(),
                binding: RegistryLiveSpecInstallBinding {
                    deployment_id: 100 + index as i32,
                    websocket_endpoint: format!("wss://stream-{alias}.example.test/ws"),
                    query_endpoint: query_endpoints[index].clone(),
                    websocket_auth_policy: "signed_session".into(),
                    query_auth_policy: "signed_session".into(),
                    observed_generation: 7,
                },
            })
            .collect();
        let gateway_auth = |scopes: &[&str], transaction_entitlement_required| {
            crate::api_client::RegistrySolanaGatewayAuthMetadata {
                required: true,
                mode: "signed_session".into(),
                session_endpoint: "https://api.example.test/ws/sessions".into(),
                jwks_url: "https://api.example.test/.well-known/jwks.json".into(),
                token_transport: "bearer".into(),
                audience: "arete:solana-gateway".into(),
                target_kind: "solana-gateway-binding".into(),
                target_id: "sgb_00000000000000000000000000000001".into(),
                scopes: scopes.iter().map(|scope| (*scope).into()).collect(),
                accepted_key_classes: if transaction_entitlement_required {
                    vec!["publishable".into(), "secret".into()]
                } else {
                    vec!["anonymous".into(), "publishable".into(), "secret".into()]
                },
                transaction_entitlement_required,
            }
        };
        (
            RegistryStackInstallResponse {
                name: "HostedThree".into(),
                stack: "hosted-three-stack".into(),
                websocket_url: None,
                http_url: None,
                websocket_auth: None,
                http_auth: None,
                description: None,
                visibility: "public".into(),
                spec_version_id: Some(1),
                live_spec_hash: None,
                live_spec: None,
                live_specs: descriptors,
                stack_manifest_hash: stack_manifest.artifact_hash.to_string(),
                stack_manifest: serde_json::to_value(stack_manifest).unwrap(),
                chain_binding: Some(RegistryCapabilityInstallBinding {
                    endpoint: "https://solana.example.test/gateway/".into(),
                    auth_policy: "signed_session".into(),
                    solana_gateway_binding_id: "sgb_00000000000000000000000000000001".into(),
                    cluster: "mainnet-beta".into(),
                    region: "us-west-1".into(),
                    auth: gateway_auth(&["read"], false),
                }),
                transaction_binding: Some(RegistryCapabilityInstallBinding {
                    endpoint: "https://solana.example.test/gateway/".into(),
                    auth_policy: "signed_session".into(),
                    solana_gateway_binding_id: "sgb_00000000000000000000000000000001".into(),
                    cluster: "mainnet-beta".into(),
                    region: "us-west-1".into(),
                    auth: gateway_auth(&["transaction:inspect", "transaction:send"], true),
                }),
                extensions: None,
                programs,
            },
            program_endpoints,
            query_endpoints,
        )
    }

    #[test]
    fn remote_three_live_codegen_preserves_live_program_and_capability_bindings() {
        use std::sync::atomic::{AtomicU64, Ordering};

        static NEXT: AtomicU64 = AtomicU64::new(0);
        let aliases = ["alpha", "beta", "gamma"];
        let (install, program_endpoints, query_endpoints) = hosted_v2_install(&aliases);
        let remote = remote_stack_install(install).unwrap();
        assert_eq!(
            remote
                .live_specs
                .iter()
                .map(|(alias, _)| alias.as_str())
                .collect::<Vec<_>>(),
            aliases
        );
        let source = ResolvedStackSource::Remote(Box::new(remote));
        assert!(source.default_websocket_url().is_none());
        assert!(source.default_http_url().is_none());
        let directory = std::env::temp_dir().join(format!(
            "a4-hosted-composition-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));

        generate_typescript_sdk_from_source(
            &source,
            &directory,
            "@usearete/react",
            None,
            None,
            None,
            &BTreeMap::new(),
            &BTreeMap::new(),
            false,
        )
        .unwrap();

        for (index, alias) in aliases.iter().enumerate() {
            let generated = fs::read_to_string(directory.join(format!("{alias}-stack.ts")))
                .expect("aliased hosted module");
            assert!(generated.contains(&format!("ws: 'wss://stream-{alias}.example.test/ws'")));
            assert!(generated.contains(&format!("http: '{}'", query_endpoints[index])));
            assert!(generated.contains(&format!("endpoint: \"{}\"", program_endpoints[index])));
            assert!(!generated.contains(&format!("endpoint: \"{}\"", query_endpoints[index])));
            assert!(!generated.contains("programReadFallback"));
        }
        let session = fs::read_to_string(directory.join("hosted-three.ts")).unwrap();
        assert!(session.contains("createHostedThreeSession"));
        assert!(session.contains("mode: 'composition',\n  gateway: {"));
        assert!(session.contains("HOSTED_THREE_HOSTED_BINDINGS"));
        assert!(session.contains("https://solana.example.test/gateway/"));
        assert!(session.contains("solanaGatewayBindingId"));
        assert!(session.contains("transactionEntitlementRequired"));
        assert!(session.contains("createHostedSolanaGatewayTransports"));
        assert!(session.contains("createHostedThreeHostedSession"));
        assert!(session.contains("chain: HOSTED_THREE_HOSTED_BINDINGS.chain"));
        assert!(session.contains("transactions: HOSTED_THREE_HOSTED_BINDINGS.transactions"));
        assert!(session.contains("return createHostedThreeSession({ ...options, ...transports })"));
        assert!(!session.contains("query-alpha.example.test/v1\",\n  \"transactions"));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn remote_single_live_codegen_embeds_managed_routes_in_all_languages() {
        let (install, program_endpoints, _) = hosted_v2_install(&["alpha"]);
        let source = ResolvedStackSource::Remote(Box::new(remote_stack_install(install).unwrap()));
        let stack_spec = source.load_stack_spec(true).unwrap();
        let gateway = source.hosted_gateway().unwrap();

        let typescript = arete_interpreter::typescript::compile_stack_spec_with_exact_views(
            stack_spec.clone(),
            Some(arete_interpreter::typescript::TypeScriptStackConfig {
                websocket_url: source.default_websocket_url(),
                http_url: source.default_http_url(),
                programs: source.typescript_programs(&stack_spec).unwrap(),
                gateway: gateway.clone(),
                ..Default::default()
            }),
        )
        .unwrap();
        assert!(typescript.stack_definition.contains("gateway:"));
        assert!(typescript
            .stack_definition
            .contains("https://solana.example.test/gateway/"));
        assert!(typescript
            .stack_definition
            .contains(program_endpoints[0].as_str()));

        let rust = arete_interpreter::rust::compile_stack_spec_with_exact_views(
            stack_spec.clone(),
            Some(arete_interpreter::rust::RustStackConfig {
                url: source.default_websocket_url(),
                http_url: source.default_http_url(),
                program_reads: source.rust_program_reads().unwrap(),
                gateway: gateway.clone(),
                ..Default::default()
            }),
        )
        .unwrap();
        assert!(rust.entity_rs.contains("fn gateway()"));
        assert!(rust
            .entity_rs
            .contains("https://solana.example.test/gateway/"));
        assert!(rust
            .programs_rs
            .as_deref()
            .unwrap()
            .contains(program_endpoints[0].as_str()));

        let python = arete_interpreter::python::compile_stack_spec_with_exact_views(
            stack_spec,
            Some(arete_interpreter::python::PythonStackConfig {
                url: source.default_websocket_url(),
                http_url: source.default_http_url(),
                program_reads: source.python_program_reads().unwrap(),
                gateway,
                ..Default::default()
            }),
        )
        .unwrap();
        assert!(python
            .init_py
            .contains("HostedSolanaGatewayBindings.from_dict"));
        assert!(python
            .init_py
            .contains("https://solana.example.test/gateway/"));
        assert!(python
            .programs_py
            .as_deref()
            .unwrap()
            .contains(program_endpoints[0].as_str()));
    }

    #[test]
    fn remote_v2_install_rejects_alias_hash_order_and_binding_mismatches() {
        let aliases = ["alpha", "beta", "gamma"];

        let (mut alias, _, _) = hosted_v2_install(&aliases);
        alias.live_specs[0].alias = "other".into();
        assert!(remote_stack_install(alias).is_err());

        let (mut hash, _, _) = hosted_v2_install(&aliases);
        hash.live_specs[1].live_spec_hash = "other-hash".into();
        assert!(remote_stack_install(hash).is_err());

        let (mut order, _, _) = hosted_v2_install(&aliases);
        order.live_specs.swap(0, 1);
        assert!(remote_stack_install(order).is_err());

        let (mut binding, _, _) = hosted_v2_install(&aliases);
        binding.live_specs[1].binding.deployment_id = binding.live_specs[0].binding.deployment_id;
        assert!(remote_stack_install(binding).is_err());

        let (mut singular, _, _) = hosted_v2_install(&aliases);
        singular.websocket_url = Some("wss://singular.example.test".into());
        assert!(remote_stack_install(singular).is_err());
    }

    #[test]
    fn remote_single_live_accepts_consistent_singular_plural_compatibility_fields() {
        let (mut install, _, _) = hosted_v2_install(&["alpha"]);
        let live = install.live_specs[0].clone();
        install.websocket_url = Some(live.binding.websocket_endpoint.clone());
        install.http_url = Some(live.binding.query_endpoint.clone());
        install.websocket_auth =
            Some(serde_json::json!({"mode": live.binding.websocket_auth_policy.clone()}));
        install.http_auth =
            Some(serde_json::json!({"mode": live.binding.query_auth_policy.clone()}));
        install.live_spec_hash = Some(live.live_spec_hash.clone());
        install.live_spec = Some(live.artifact.clone());

        let remote = remote_stack_install(install.clone()).unwrap();
        assert_eq!(remote.live_specs.len(), 1);
        assert_eq!(remote.live_bindings[0], live);

        install.http_url = Some("https://mismatch.example.test".into());
        assert!(remote_stack_install(install).is_err());
    }

    #[test]
    fn local_multi_live_generation_writes_namespaced_typescript_rust_and_python_modules() {
        use arete_hash::{CanonicalIdlDocument, ProgramSpecV1};
        use std::sync::atomic::{AtomicU64, Ordering};

        static NEXT: AtomicU64 = AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!(
            "a4-multi-sdk-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&directory).unwrap();
        let document = CanonicalIdlDocument::parse(
            br#"{"address":"11111111111111111111111111111111","metadata":{"name":"system","version":"1.0.0","spec":"0.1.0"},"instructions":[],"accounts":[],"types":[],"events":[],"errors":[]}"#,
            None,
        )
        .unwrap();
        let program =
            arete_artifacts::ProgramSpecArtifact::new(ProgramSpecV1::from_document(&document))
                .unwrap();
        let live = arete_artifacts::live_spec_v2(
            std::slice::from_ref(&program),
            vec![arete_artifacts::PortableEntity::new(
                "SharedState",
                "id.address",
            )],
            Vec::new(),
        )
        .unwrap();
        let live_specs = vec![
            ("alpha".to_string(), live.clone()),
            ("beta".to_string(), live),
        ];
        let stack_manifest = arete_artifacts::compose_stack_manifest_v2(
            "Composed",
            std::slice::from_ref(&program),
            live_specs
                .iter()
                .map(|(alias, live)| (alias.clone(), live))
                .collect(),
            vec![
                arete_artifacts::SelectedViewV2 {
                    live_alias: "alpha".to_string(),
                    view_id: "SharedState/list".to_string(),
                },
                arete_artifacts::SelectedViewV2 {
                    live_alias: "beta".to_string(),
                    view_id: "SharedState/list".to_string(),
                },
            ],
        )
        .unwrap();
        let stack = LocalArtifactStack {
            manifest_path: directory.join("Composed.stack-manifest.json"),
            manifest_hash: stack_manifest.artifact_hash.to_string(),
            program_specs: vec![program],
            live_specs,
            stack_manifest,
        };
        let source = ResolvedStackSource::LocalArtifacts(Box::new(stack.clone()));
        let typescript_dir = directory.join("typescript");
        generate_typescript_composition_sdk(
            &source,
            &stack.program_specs,
            &stack.live_specs,
            &stack.stack_manifest,
            &typescript_dir,
            "@usearete/react",
            None,
            None,
            None,
            &BTreeMap::from([("alpha".to_string(), "./existing/alpha.js".to_string())]),
            &BTreeMap::new(),
        )
        .unwrap();
        assert!(typescript_dir.join("alpha-stack.ts").is_file());
        assert!(typescript_dir.join("beta-stack.ts").is_file());
        let session = fs::read_to_string(typescript_dir.join("Composed.ts")).unwrap();
        assert!(session.contains("createComposedSession"));
        assert!(session.contains("import AlphaStack from './existing/alpha.js'"));
        assert!(session.contains("alpha: AlphaStack"));
        assert!(session.contains("beta: BetaStack"));

        let rust = arete_interpreter::rust::compile_composed_public_artifacts_v2(
            &stack.program_specs,
            &stack.live_specs,
            &stack.stack_manifest,
            Some(arete_interpreter::rust::RustCompositionConfig {
                stack: arete_interpreter::rust::RustStackConfig {
                    crate_name: "composed".to_string(),
                    ..Default::default()
                },
                live_urls: BTreeMap::new(),
            }),
        )
        .unwrap();
        let rust_dir = directory.join("rust");
        arete_interpreter::rust::write_rust_composition_crate(&rust, &rust_dir).unwrap();
        assert!(rust_dir.join("src/alpha/mod.rs").is_file());
        assert!(rust_dir.join("src/beta/mod.rs").is_file());
        assert_eq!(
            fs::read_to_string(rust_dir.join("src/lib.rs")).unwrap(),
            "pub mod alpha;\npub mod beta;\n"
        );

        let python = arete_interpreter::python::compile_composed_public_artifacts_v2(
            &stack.program_specs,
            &stack.live_specs,
            &stack.stack_manifest,
            Some(arete_interpreter::python::PythonCompositionConfig {
                stack: arete_interpreter::python::PythonStackConfig {
                    package_name: "composed".to_string(),
                    ..Default::default()
                },
                live_urls: BTreeMap::new(),
            }),
        )
        .unwrap();
        let python_dir = directory.join("python");
        arete_interpreter::python::write_python_composition_package(&python, &python_dir).unwrap();
        assert!(python_dir.join("pyproject.toml").is_file());
        assert!(python_dir.join("composed/alpha/__init__.py").is_file());
        assert!(python_dir.join("composed/beta/__init__.py").is_file());
        let root_init = fs::read_to_string(python_dir.join("composed/__init__.py")).unwrap();
        assert!(root_init.contains("from . import alpha"));
        assert!(root_init.contains("from . import beta"));
        fs::remove_dir_all(directory).unwrap();
    }

    fn ore_fixture_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("cli crate lives in the repo root")
            .join("stacks/ore/.arete")
    }

    fn assert_linked_sdk_dependency(manifest_path: &Path) {
        let manifest = fs::read_to_string(manifest_path).expect("generated Cargo.toml");
        let expected = format!(
            "arete-sdk = {{ package = \"arete-a4-sdk\", version = {:?} }}",
            arete_interpreter::rust::GENERATED_RUST_SDK_VERSION
        );
        assert!(
            manifest.contains(&expected),
            "{} must depend on the linked SDK release:\n{manifest}",
            manifest_path.display()
        );
        assert!(
            !manifest.contains("\"0.4\""),
            "{} must not pin the obsolete 0.4 SDK:\n{manifest}",
            manifest_path.display()
        );
        assert!(
            !manifest.contains("path =") && !manifest.contains("[patch"),
            "{} must not carry local path or patch dependencies:\n{manifest}",
            manifest_path.display()
        );
    }

    #[test]
    fn generated_rust_program_and_stack_crates_pin_the_linked_sdk_release() {
        let directory = tempfile::tempdir().unwrap();
        let fixtures = ore_fixture_root();

        let program_dir = directory.path().join("ore-program");
        generate_project_local_program(
            &fixtures.join("ore.program-spec.json"),
            ProjectGenerationOptions {
                alias: "ore",
                target: crate::project::manifest::InstallTarget::Rust,
                output: &program_dir,
                typescript_package: "@usearete/react",
                rust_module: false,
                python_module: false,
            },
        )
        .expect("local program generation should succeed");
        assert_linked_sdk_dependency(&program_dir.join("Cargo.toml"));

        let stack_dir = directory.path().join("ore-stack");
        generate_project_local_stack(
            &fixtures.join("OreStream.stack-manifest.json"),
            std::slice::from_ref(&fixtures),
            ProjectGenerationOptions {
                alias: "ore",
                target: crate::project::manifest::InstallTarget::Rust,
                output: &stack_dir,
                typescript_package: "@usearete/react",
                rust_module: false,
                python_module: false,
            },
        )
        .expect("local stack generation should succeed");
        assert_linked_sdk_dependency(&stack_dir.join("Cargo.toml"));

        let python_dir = directory.path().join("ore-py");
        generate_project_local_program(
            &fixtures.join("ore.program-spec.json"),
            ProjectGenerationOptions {
                alias: "ore",
                target: crate::project::manifest::InstallTarget::Python,
                output: &python_dir,
                typescript_package: "@usearete/react",
                rust_module: false,
                python_module: false,
            },
        )
        .expect("local python generation should succeed");
        let pyproject = fs::read_to_string(python_dir.join("pyproject.toml")).unwrap();
        assert!(
            pyproject.contains(&format!(
                "dependencies = [\"arete-sdk>={}\"]",
                arete_interpreter::python::GENERATED_PYTHON_SDK_VERSION
            )),
            "{pyproject}"
        );
        assert!(!pyproject.contains(">=0.4\""), "{pyproject}");
    }
}
