use anyhow::{Context, Result};
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
use crate::config::{discover_ast_files, find_ast_file, to_kebab_case, AreteConfig, DiscoveredAst};
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
}

enum ResolvedStackSource {
    Local(DiscoveredAst),
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExtensionsManifest {
    entry: String,
    files: Vec<String>,
    input_kind: Option<ExtensionsInputKind>,
    input_hash: Option<String>,
    sdk_range: Option<String>,
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
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProgramExtensionBinding {
    export_name: String,
    program_key: String,
}

#[derive(Debug, Clone)]
struct HostedProgramExtension {
    program_key: String,
    program_const_name: String,
    import_name: String,
    input_pin: ResolvedExtensionsInputPin,
    artifact: ResolvedExtensionsArtifact,
}

#[derive(Debug, Clone)]
struct TypeScriptLayout {
    output_dir: PathBuf,
    base_name: String,
    entry_path: PathBuf,
    core_path: PathBuf,
}

const SDK_PROVENANCE_FILE: &str = "sdk-provenance.json";

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
    extensions: Option<SdkProvenanceExtensionsV2>,
    artifacts: Vec<String>,
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
    legacy_provenance_sha256: String,
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
    fn stack_id(&self) -> &str {
        match self {
            Self::Local(ast) => ast.stack_id.as_str(),
            Self::LocalArtifacts(stack) => stack.manifest_hash.as_str(),
            Self::Remote(stack) => stack.stack.as_str(),
        }
    }

    fn sdk_name(&self) -> &str {
        match self {
            Self::Local(ast) => ast.stack_name.as_str(),
            Self::LocalArtifacts(stack) => stack.stack_manifest.payload.name.as_str(),
            Self::Remote(stack) => stack.sdk_name.as_str(),
        }
    }

    fn default_websocket_url(&self) -> Option<String> {
        match self {
            Self::Local(_) => None,
            Self::LocalArtifacts(_) => None,
            Self::Remote(stack) => (stack.live_bindings.len() == 1)
                .then(|| stack.live_bindings[0].binding.websocket_endpoint.clone()),
        }
    }

    fn default_http_url(&self) -> Option<String> {
        match self {
            Self::Local(_) => None,
            Self::LocalArtifacts(_) => None,
            Self::Remote(stack) => (stack.live_bindings.len() == 1)
                .then(|| stack.live_bindings[0].binding.query_endpoint.clone()),
        }
    }

    fn print_source_details(&self) {
        match self {
            Self::Local(ast) => {
                println!("  Path: {}", ast.path.display());
                println!(
                    "  {}",
                    "Composite stack input is deprecated; use --manifest with the generated StackManifest."
                        .yellow()
                );
                if !ast.program_ids.is_empty() {
                    println!("  Program IDs: {}", ast.program_ids.join(", "));
                }
            }
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
            Self::Local(ast) => load_stack_spec_from_file(ast, require_entities),
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
            Self::Local(_) | Self::LocalArtifacts(_) => None,
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
            Self::Local(_) | Self::LocalArtifacts(_) => BTreeMap::new(),
        }
    }

    fn typescript_programs(
        &self,
        stack_spec: &arete_interpreter::ast::SerializableStackSpec,
    ) -> Result<Option<Vec<arete_interpreter::typescript::TypeScriptProgramConfig>>> {
        let mut programs = match self {
            Self::Local(_) | Self::LocalArtifacts(_) if stack_spec.program_specs.is_empty() => {
                return Ok(None)
            }
            Self::Local(_) | Self::LocalArtifacts(_) => stack_spec
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
        for program in &mut programs {
            program.definition.sdk_definition_hash = Some(program_definition_hash(
                &program.definition.program_spec_hash,
            )?);
        }
        Ok(Some(programs))
    }
}

pub fn list(config_path: &str) -> Result<()> {
    let config = AreteConfig::load_optional(config_path)?;

    let discovered = discover_ast_files(None)?;

    let has_config_stacks = config
        .as_ref()
        .map(|c| !c.stacks.is_empty())
        .unwrap_or(false);

    if !has_config_stacks && discovered.is_empty() {
        println!("{}", "No stacks found.".yellow());
        println!();
        println!("To add stacks:");
        println!("  1. Build your stack crate to generate .arete/*.stack.json files");
        println!("  2. Run {} to create a configuration", "a4 init".cyan());
        return Ok(());
    }

    println!("{} Available stacks:\n", "→".blue().bold());

    if let Some(ref cfg) = config {
        for stack in &cfg.stacks {
            let name = stack.name.as_deref().unwrap_or(&stack.stack);
            println!("  {}", name.green().bold());
            println!("    Stack: {}", stack.stack);

            if let Some(desc) = &stack.description {
                println!("    Description: {}", desc);
            }

            if let Some(url) = &stack.url {
                println!("    URL: {}", url.cyan());
            }

            let ts_output = cfg.get_typescript_output_path(name, Some(stack), None);
            let rust_output = cfg.get_rust_output_path(name, Some(stack), None);
            println!("    TypeScript: {}", ts_output.display());
            println!("    Rust: {}", rust_output.display());
            println!();
        }
    }

    let config_asts: std::collections::HashSet<_> = config
        .as_ref()
        .map(|c| c.stacks.iter().map(|s| s.stack.clone()).collect())
        .unwrap_or_default();

    for ast in discovered {
        if !config_asts.contains(&ast.stack_id) {
            println!("  {} {}", "•".dimmed(), ast.stack_name.green().bold());
            println!("    Stack: {}", ast.stack_id);
            println!("    Path: {}", ast.path.display());
            if !ast.program_ids.is_empty() {
                println!("    Program IDs: {}", ast.program_ids.join(", "));
            }
            println!("    {}", "(auto-discovered, not in config)".dimmed());
            println!();
        }
    }

    println!(
        "Use {} to generate SDK",
        "a4 sdk create <stack-name> --ts".cyan()
    );

    Ok(())
}

pub fn sync(config_path: &str, ts: bool, rust: bool, stack_filters: Vec<String>) -> Result<()> {
    let config = AreteConfig::load(config_path)?;
    if config.stacks.is_empty() {
        anyhow::bail!("No stacks are configured in {}", config_path);
    }

    let sync_typescript = ts || !rust;
    let sync_rust = rust || !ts;
    let filter_set: BTreeSet<String> = stack_filters.into_iter().collect();
    let selected = resolve_sync_stack_names(&config, &filter_set)?;

    println!(
        "{} Syncing {} configured stack(s)...",
        "→".blue().bold(),
        selected.len()
    );

    for stack_name in selected {
        if sync_typescript {
            create_typescript(
                config_path,
                Some(&stack_name),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Vec::new(),
                Vec::new(),
                Vec::new(),
                false,
            )?;
        }
        if sync_rust {
            create_rust(
                config_path,
                Some(&stack_name),
                None,
                None,
                false,
                None,
                None,
                Vec::new(),
            )?;
        }
    }

    Ok(())
}

fn resolve_sync_stack_names(
    config: &AreteConfig,
    filter_set: &BTreeSet<String>,
) -> Result<Vec<String>> {
    let selected: Vec<String> = config
        .stacks
        .iter()
        .filter_map(|stack| {
            let name = stack.name.as_deref().unwrap_or(&stack.stack);
            if filter_set.is_empty()
                || filter_set.contains(name)
                || filter_set.contains(&stack.stack)
            {
                Some(name.to_string())
            } else {
                None
            }
        })
        .collect();

    if selected.is_empty() {
        anyhow::bail!("No configured stacks matched the requested sync filters");
    }

    Ok(selected)
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

    match select_sdk_target(ts, rust, "Generate which SDK?")? {
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
    let config = AreteConfig::load_optional(config_path)?;
    let live_module_imports = parse_live_module_imports(&live_module_values)?;
    let program_module_imports = parse_program_module_imports(&program_module_values)?;

    // Get the config file's directory for resolving relative paths
    let config_dir = Path::new(config_path)
        .parent()
        .unwrap_or(Path::new("."))
        .to_path_buf();

    if let Some(program_spec_path) = program_spec_override {
        let program_spec_path = PathBuf::from(program_spec_path);
        let bytes = fs::read(&program_spec_path).with_context(|| {
            format!("Failed to read ProgramSpec {}", program_spec_path.display())
        })?;
        let program_spec = arete_artifacts::load_program_spec(&bytes)
            .with_context(|| format!("Invalid ProgramSpec {}", program_spec_path.display()))?
            .artifact;
        let sdk_name = to_kebab_case(&program_spec.payload.idl_snapshot.snapshot.name);
        let output_path = resolve_typescript_output_path_for_idl(
            config.as_ref(),
            &config_dir,
            &sdk_name,
            output_override,
        );
        let package_name = package_name_override
            .or_else(|| {
                config.as_ref().and_then(|cfg| {
                    cfg.sdk
                        .as_ref()
                        .and_then(|sdk| sdk.typescript_package.clone())
                })
            })
            .unwrap_or_else(|| "@usearete/react".to_string());

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
        let output_path = resolve_typescript_output_path_for_idl(
            config.as_ref(),
            &config_dir,
            &sdk_name,
            output_override,
        );
        let package_name = package_name_override
            .or_else(|| {
                config.as_ref().and_then(|cfg| {
                    cfg.sdk
                        .as_ref()
                        .and_then(|sdk| sdk.typescript_package.clone())
                })
            })
            .unwrap_or_else(|| "@usearete/react".to_string());

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
        let package_name = package_name_override
            .or_else(|| {
                config.as_ref().and_then(|cfg| {
                    cfg.sdk
                        .as_ref()
                        .and_then(|sdk| sdk.typescript_package.clone())
                })
            })
            .unwrap_or_else(|| "@usearete/react".to_string());
        (source, output, package_name, url_override, None)
    } else if let Some(ref cfg) = config {
        let stack_name = stack_name.ok_or_else(|| {
            anyhow::anyhow!(
                "stack name is required unless using --idl with --program-only or --manifest"
            )
        })?;
        println!(
            "{} Looking for stack '{}'...",
            "→".blue().bold(),
            stack_name
        );
        if let Some(stack_config) = cfg.find_stack(stack_name) {
            let source = resolve_stack_source(&client, &stack_config.stack)?;

            let name = stack_config.name.as_deref().unwrap_or(&stack_config.stack);
            let raw_output =
                cfg.get_typescript_output_path(name, Some(stack_config), output_override.clone());

            // Resolve relative paths relative to the config file's directory
            let output = if raw_output.is_relative() {
                config_dir.join(&raw_output)
            } else {
                raw_output
            };

            let pkg = package_name_override
                .or_else(|| cfg.sdk.as_ref().and_then(|s| s.typescript_package.clone()))
                .unwrap_or_else(|| "@usearete/react".to_string());

            let websocket_url = url_override
                .or_else(|| stack_config.url.clone())
                .or_else(|| source.default_websocket_url());
            let http_url = source.default_http_url();

            (source, output, pkg, websocket_url, http_url)
        } else {
            let (source, output, pkg) =
                find_stack_by_name(&client, stack_name, output_override, package_name_override)?;
            let websocket_url = url_override.or_else(|| source.default_websocket_url());
            let http_url = source.default_http_url();
            (source, output, pkg, websocket_url, http_url)
        }
    } else {
        let stack_name = stack_name.ok_or_else(|| {
            anyhow::anyhow!(
                "stack name is required unless using --idl with --program-only or --manifest"
            )
        })?;
        println!(
            "{} Looking for stack '{}'...",
            "→".blue().bold(),
            stack_name
        );
        let (source, output, pkg) =
            find_stack_by_name(&client, stack_name, output_override, package_name_override)?;
        let websocket_url = url_override.or_else(|| source.default_websocket_url());
        let http_url = source.default_http_url();
        (source, output, pkg, websocket_url, http_url)
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

#[allow(clippy::too_many_arguments)]
pub fn install_command(
    install_target: &str,
    maybe_install_name: Option<&str>,
    ts: bool,
    rust: bool,
    output_override: Option<String>,
    package_name_override: Option<String>,
    crate_name_override: Option<String>,
    module_flag: bool,
    url_override: Option<String>,
    extensions_override: Option<String>,
) -> Result<()> {
    if let Some(install_name) = maybe_install_name {
        if install_target != "program" {
            return Err(anyhow::anyhow!(
                "Unexpected install subcommand '{}'. Supported subcommand: program",
                install_target
            ));
        }

        return install_program(
            install_name,
            ts,
            rust,
            output_override,
            package_name_override,
            extensions_override,
        );
    }

    install_stack(
        install_target,
        ts,
        rust,
        output_override,
        package_name_override,
        crate_name_override,
        module_flag,
        url_override,
        extensions_override,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn install_stack(
    stack_name: &str,
    ts: bool,
    rust: bool,
    output_override: Option<String>,
    package_name_override: Option<String>,
    crate_name_override: Option<String>,
    module_flag: bool,
    url_override: Option<String>,
    extensions_override: Option<String>,
) -> Result<()> {
    match select_sdk_target(ts, rust, "Install which SDK?")? {
        SdkTarget::TypeScript => install_typescript(
            stack_name,
            output_override,
            package_name_override,
            url_override,
            extensions_override,
        ),
        SdkTarget::Rust => install_rust(
            stack_name,
            output_override,
            crate_name_override,
            module_flag,
            url_override,
        ),
    }
}

fn install_typescript(
    stack_name: &str,
    output_override: Option<String>,
    package_name_override: Option<String>,
    url_override: Option<String>,
    extensions_override: Option<String>,
) -> Result<()> {
    println!(
        "{} Looking up hosted stack '{}'...",
        "→".blue().bold(),
        stack_name
    );

    let client = ApiClient::new()?;
    let source = resolve_remote_stack_source(&client, stack_name)?;
    let output_path = output_override
        .map(PathBuf::from)
        .unwrap_or_else(|| default_typescript_output_dir(source.sdk_name()));
    let package_name = package_name_override.unwrap_or_else(|| "@usearete/react".to_string());
    let websocket_url = url_override.or_else(|| source.default_websocket_url());
    let http_url = source.default_http_url();

    println!(
        "{} Found hosted stack: {}",
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
            "(not provided by hosted stack - placeholder will be generated)".dimmed()
        );
    }
    if let Some(url) = &http_url {
        println!("  HTTP URL: {}", url.cyan());
    } else {
        println!(
            "  HTTP URL: {}",
            "(not provided by hosted stack - placeholder will be generated)".dimmed()
        );
    }

    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create output directory: {}", parent.display()))?;
    }

    println!("\n{} Generating TypeScript SDK...", "→".blue().bold());

    generate_typescript_sdk_from_source(
        &source,
        &output_path,
        &package_name,
        websocket_url,
        http_url,
        extensions_override.as_deref().map(Path::new),
        &BTreeMap::new(),
        &BTreeMap::new(),
        false,
    )?;

    println!(
        "{} Successfully generated TypeScript SDK!",
        "✓".green().bold()
    );
    println!("  Output: {}", output_path.display().to_string().bold());

    telemetry::record_sdk_generated("typescript");

    Ok(())
}

fn install_rust(
    stack_name: &str,
    output_override: Option<String>,
    crate_name_override: Option<String>,
    module_flag: bool,
    url_override: Option<String>,
) -> Result<()> {
    println!(
        "{} Looking up hosted stack '{}'...",
        "→".blue().bold(),
        stack_name
    );

    let client = ApiClient::new()?;
    let source = resolve_remote_stack_source(&client, stack_name)?;
    let crate_name = crate_name_override.unwrap_or_else(|| format!("{}-stack", source.sdk_name()));
    let output_dir = output_override
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("./generated/{}-stack", source.sdk_name())));
    let stack_url = url_override.or_else(|| source.default_websocket_url());

    println!(
        "{} Found hosted stack: {}",
        "✓".green().bold(),
        source.stack_id().bold()
    );
    source.print_source_details();
    println!("  Output: {}", output_dir.display());
    if module_flag {
        println!("  Mode: module (mod.rs)");
    }
    if let Some(url) = &stack_url {
        println!("  URL: {}", url.cyan());
    } else {
        println!(
            "  URL: {}",
            "(not provided by hosted stack - placeholder will be generated)".dimmed()
        );
    }

    println!("\n{} Generating Rust SDK...", "→".blue().bold());

    if let Some(composition) = source.composition_artifacts() {
        if stack_url.is_some() {
            anyhow::bail!(
                "multi-live Rust install requires per-alias URLs; a shared --url is not allowed"
            );
        }
        let live_urls = match &source {
            ResolvedStackSource::Remote(stack) => stack
                .live_bindings
                .iter()
                .map(|live| (live.alias.clone(), live.binding.websocket_endpoint.clone()))
                .collect(),
            ResolvedStackSource::Local(_) | ResolvedStackSource::LocalArtifacts(_) => {
                BTreeMap::new()
            }
        };
        let output = arete_interpreter::rust::compile_composed_public_artifacts_v2(
            composition.program_specs,
            composition.live_specs,
            composition.stack_manifest,
            Some(arete_interpreter::rust::RustCompositionConfig {
                stack: arete_interpreter::rust::RustStackConfig {
                    crate_name: crate_name.clone(),
                    sdk_version: "0.3".to_string(),
                    module_mode: module_flag,
                    url: None,
                },
                live_urls,
            }),
        )
        .map_err(|error| anyhow::anyhow!("Failed to compile Rust composition: {error}"))?;
        if module_flag {
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

    let rust_config = arete_interpreter::rust::RustStackConfig {
        crate_name: crate_name.clone(),
        sdk_version: "0.3".to_string(),
        module_mode: module_flag,
        url: stack_url,
    };

    let output = match &source {
        ResolvedStackSource::Remote(stack) if stack.exact_views => {
            arete_interpreter::rust::compile_stack_spec_with_exact_views(
                stack_spec,
                Some(rust_config),
            )
        }
        _ => arete_interpreter::rust::compile_stack_spec(stack_spec, Some(rust_config)),
    }
    .map_err(|e| anyhow::anyhow!("Failed to compile Rust: {}", e))?;

    if module_flag {
        arete_interpreter::rust::write_rust_module(&output, &output_dir)
            .with_context(|| format!("Failed to write Rust module to {}", output_dir.display()))?;

        println!("{} Successfully generated Rust module!", "✓".green().bold());
        println!("  Module: {}", output_dir.display().to_string().bold());
        println!("\n  Add to your lib.rs:");
        let module_name = output_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("module");
        println!("    pub mod {};", module_name.cyan());
    } else {
        arete_interpreter::rust::write_rust_crate(&output, &output_dir)
            .with_context(|| format!("Failed to write Rust crate to {}", output_dir.display()))?;

        println!("{} Successfully generated Rust SDK!", "✓".green().bold());
        println!("  Crate: {}", output_dir.display().to_string().bold());
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

pub fn install_program(
    program: &str,
    ts: bool,
    rust: bool,
    output_override: Option<String>,
    package_name_override: Option<String>,
    extensions_override: Option<String>,
) -> Result<()> {
    match select_sdk_target(ts, rust, "Install which SDK?")? {
        SdkTarget::TypeScript => install_program_typescript(
            program,
            output_override,
            package_name_override,
            extensions_override,
        ),
        SdkTarget::Rust => Err(anyhow::anyhow!(
            "Published program SDK install currently supports TypeScript only"
        )),
    }
}

fn install_program_typescript(
    program: &str,
    output_override: Option<String>,
    package_name_override: Option<String>,
    extensions_override: Option<String>,
) -> Result<()> {
    println!(
        "{} Looking up hosted program '{}'...",
        "→".blue().bold(),
        program
    );

    let client = ApiClient::new()?;
    let install = client
        .get_registry_program_install(program)
        .with_context(|| {
            format!(
                "No accessible hosted program SDK with identifier '{}' was found.",
                program
            )
        })?;

    let sdk_name = install.install_name.clone();
    let output_path = output_override
        .map(PathBuf::from)
        .unwrap_or_else(|| default_typescript_output_dir(&sdk_name));
    let package_name = package_name_override.unwrap_or_else(|| "@usearete/react".to_string());

    println!(
        "{} Found hosted program: {}",
        "✓".green().bold(),
        install.install_name.as_str().bold()
    );
    println!("  Program ID: {}", install.definition.program_id);
    println!("  Display Name: {}", install.display_name);
    println!("  Output: {}", output_path.display());

    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create output directory: {}", parent.display()))?;
    }

    println!(
        "\n{} Generating TypeScript program SDK...",
        "→".blue().bold()
    );

    let hosted_artifact = install
        .definition
        .extensions
        .as_ref()
        .map(resolved_extensions_artifact_from_registry)
        .transpose()?;
    generate_typescript_program_sdk_from_install(
        &install,
        &sdk_name,
        &output_path,
        &package_name,
        extensions_override.as_deref().map(Path::new),
        hosted_artifact.as_ref(),
    )?;

    println!(
        "{} Successfully generated TypeScript SDK!",
        "✓".green().bold()
    );
    println!("  Output: {}", output_path.display().to_string().bold());

    telemetry::record_sdk_generated("typescript");

    Ok(())
}

fn select_sdk_target(ts: bool, rust: bool, prompt: &str) -> Result<SdkTarget> {
    match (ts, rust) {
        (true, false) => Ok(SdkTarget::TypeScript),
        (false, true) => Ok(SdkTarget::Rust),
        (false, false) => {
            let theme = ColorfulTheme::default();
            let items = ["TypeScript", "Rust"];
            let selection = Select::with_theme(&theme)
                .with_prompt(prompt)
                .items(&items)
                .default(0)
                .interact()
                .context("Failed to select SDK language")?;

            Ok(match selection {
                0 => SdkTarget::TypeScript,
                1 => SdkTarget::Rust,
                _ => unreachable!(),
            })
        }
        (true, true) => Err(anyhow::anyhow!(
            "Cannot specify both --ts and --rust. Choose one."
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
    config: Option<&AreteConfig>,
    config_dir: &Path,
    sdk_name: &str,
    output_override: Option<String>,
) -> PathBuf {
    let raw_output = if let Some(path) = output_override {
        PathBuf::from(path)
    } else if let Some(cfg) = config {
        PathBuf::from(cfg.get_typescript_output_dir()).join(sdk_name)
    } else {
        default_typescript_output_dir(sdk_name)
    };

    if raw_output.is_relative() {
        config_dir.join(raw_output)
    } else {
        raw_output
    }
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
        sdk_extension_hash: None,
        sdk_output_tree_hash: None,
        program_extension_bindings,
    })
}

fn infer_extensions_artifact_from_entry(entry_path: &Path) -> Result<ResolvedExtensionsArtifact> {
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

fn resolve_explicit_extensions_artifact(
    path: &Path,
    layout: &TypeScriptLayout,
) -> Result<ResolvedExtensionsArtifact> {
    if path.is_dir() {
        let manifest_path = path.join("extensions.json");
        if manifest_path.exists() {
            let manifest = read_extensions_manifest(&manifest_path)?;
            return build_extensions_artifact(
                manifest.entry,
                read_extensions_files(path, &manifest.files)?,
                manifest.input_kind,
                manifest.input_hash,
                manifest.sdk_range,
            );
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
        return build_extensions_artifact(
            manifest.entry,
            read_extensions_files(&source_dir, &manifest.files)?,
            manifest.input_kind,
            manifest.input_hash,
            manifest.sdk_range,
        );
    }

    infer_extensions_artifact_from_entry(path)
}

fn resolve_extensions_artifact(
    explicit_path: Option<&Path>,
    layout: &TypeScriptLayout,
    hosted_artifact: Option<&ResolvedExtensionsArtifact>,
) -> Result<Option<ResolvedExtensionsArtifact>> {
    if let Some(path) = explicit_path {
        return resolve_explicit_extensions_artifact(path, layout).map(Some);
    }

    if let Some(artifact) = hosted_artifact {
        return Ok(Some(artifact.clone()));
    }

    let inferred_entry = layout
        .output_dir
        .join(format!("{}.ts", extension_entry_stem(&layout.base_name)));
    if inferred_entry.exists() {
        return infer_extensions_artifact_from_entry(&inferred_entry).map(Some);
    }

    Ok(None)
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

fn program_definition_hash(program_spec_hash: &str) -> Result<String> {
    let program_spec_hash = program_spec_hash
        .parse::<arete_hash::HashId<arete_hash::ProgramSpec>>()
        .context("Invalid ProgramSpec hash for SDK definition")?;
    Ok(
        arete_hash::SdkDefinitionV1::new(program_spec_hash, sdk_compiler_hash()?)
            .hash()
            .context("Failed to hash SDK definition")?
            .to_string(),
    )
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
    let mut artifacts = BTreeSet::from([
        generated_artifact_name(&layout.core_path)?,
        generated_artifact_name(&layout.entry_path)?,
    ]);
    if let Some(artifact) = extensions {
        artifacts.insert("extensions.json".to_string());
        for file in &artifact.files {
            artifacts.insert(normalize_extension_relative_path(&file.path)?);
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
        extensions: extensions.map(|artifact| SdkProvenanceExtensionsV2 {
            legacy_provenance_sha256: extensions_artifact_hash(artifact),
            sdk_extension_hash: artifact.sdk_extension_hash.clone(),
            sdk_output_tree_hash: artifact.sdk_output_tree_hash.clone(),
        }),
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
    let contents = format!(
        "{}\n",
        serde_json::to_string_pretty(&manifest)
            .context("Failed to serialize SDK provenance manifest")?
    );
    let path = layout.output_dir.join(SDK_PROVENANCE_FILE);
    fs::write(&path, contents).with_context(|| {
        format!(
            "Failed to write SDK provenance manifest to {}",
            path.display()
        )
    })
}

fn stack_input_pin(
    source: &ResolvedStackSource,
    stack_spec: &arete_interpreter::ast::SerializableStackSpec,
) -> Result<ResolvedExtensionsInputPin> {
    Ok(match source {
        ResolvedStackSource::Local(_) => {
            let bytes = serde_json::to_vec(stack_spec)
                .context("Failed to serialize local stack for artifact decomposition")?;
            let artifacts = arete_artifacts::decompose_legacy_stack(&bytes)
                .context("Failed to derive StackManifest identity for local stack")?;
            ResolvedExtensionsInputPin {
                kind: ExtensionsInputKind::StackManifest,
                hash: artifacts.stack_manifest.artifact_hash.to_string(),
            }
        }
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
            sdk_definition_hash: Some(program_definition_hash(
                &install.definition.program_spec_hash,
            )?),
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
    })
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

fn hosted_program_extensions(
    source: &ResolvedStackSource,
    stack_spec: &arete_interpreter::ast::SerializableStackSpec,
) -> Result<Vec<HostedProgramExtension>> {
    let ResolvedStackSource::Remote(remote) = source else {
        return Ok(Vec::new());
    };
    if remote.programs.len() != stack_spec.idls.len() {
        return Err(anyhow::anyhow!(
            "Hosted program extension descriptor count mismatch: expected {}, received {}",
            stack_spec.idls.len(),
            remote.programs.len()
        ));
    }

    remote
        .programs
        .iter()
        .zip(&stack_spec.idls)
        .filter_map(|(install, idl)| {
            install.definition.extensions.as_ref().map(|artifact| {
                let program_key = to_camel_case(&idl.name);
                Ok(HostedProgramExtension {
                    import_name: format!("hosted{}ProgramExtensions", to_pascal_case(&program_key)),
                    program_const_name: to_screaming_snake_case(&idl.name),
                    program_key,
                    input_pin: ResolvedExtensionsInputPin {
                        kind: ExtensionsInputKind::ProgramSpec,
                        hash: install.definition.program_spec_hash.clone(),
                    },
                    artifact: resolved_extensions_artifact_from_registry(artifact)?,
                })
            })
        })
        .collect()
}

fn stage_hosted_program_extensions(
    extensions: &[HostedProgramExtension],
    layout: &TypeScriptLayout,
    stack_name: &str,
    program_only: bool,
) -> Result<()> {
    for extension in extensions {
        stage_extensions_artifact_with_manifest(
            &extension.artifact,
            &layout.output_dir,
            &extension.input_pin,
            &format!("{}-extensions.json", extension.program_key),
        )?;
        stage_program_extension_core_proxies(extension, layout, stack_name, program_only)?;
    }
    Ok(())
}

fn stage_program_extension_core_proxies(
    extension: &HostedProgramExtension,
    layout: &TypeScriptLayout,
    stack_name: &str,
    program_only: bool,
) -> Result<()> {
    let import_regex =
        Regex::new(r#"from\s+['\"]\./([^'\"]*?(?:-core|core))(?:\.(?:js|ts))?['\"]"#)
            .expect("program core import regex should compile");
    let mut proxy_paths = BTreeSet::new();
    for file in &extension.artifact.files {
        let source_parent = Path::new(&file.path).parent().unwrap_or(Path::new(""));
        for captures in import_regex.captures_iter(&file.contents) {
            let relative = source_parent.join(format!("{}.ts", &captures[1]));
            proxy_paths.insert(normalize_extension_relative_path(
                &relative.to_string_lossy(),
            )?);
        }
    }
    let actual_core_stem = layout
        .core_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Invalid generated core path: {}",
                layout.core_path.display()
            )
        })?;

    for proxy_relative in proxy_paths {
        let proxy_path = layout.output_dir.join(&proxy_relative);
        if proxy_path == layout.core_path {
            continue;
        }
        if proxy_path.exists() {
            continue;
        }
        if let Some(parent) = proxy_path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "Failed to create hosted program core proxy directory {}",
                    parent.display()
                )
            })?;
        }
        let parent_depth = Path::new(&proxy_relative)
            .parent()
            .map(|parent| parent.components().count())
            .unwrap_or(0);
        let core_import = format!("{}{}.js", "../".repeat(parent_depth), actual_core_stem);
        let contents = if program_only {
            format!("export * from './{core_import}';\n")
        } else {
            let stack_core_export = format!("{}_STACK_CORE", to_screaming_snake_case(stack_name));
            format!(
                "export * from './{core_import}';\nimport {{ {stack_core_export} }} from './{core_import}';\n\nexport const {program_const} = {stack_core_export}.programs.{program_key};\n",
                program_const = extension.program_const_name,
                program_key = extension.program_key,
            )
        };
        fs::write(&proxy_path, contents).with_context(|| {
            format!(
                "Failed to write hosted program core proxy {}",
                proxy_path.display()
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
    extension_files: &[&str],
    program_extension_bindings: &[ProgramExtensionBinding],
    hosted_program_extensions: &[HostedProgramExtension],
) -> String {
    let export_name = format!("{}_STACK", to_screaming_snake_case(stack_name));
    let core_export_name = format!("{}_CORE", export_name);
    let type_name = format!("{}Stack", stack_name);
    let core_import = format!("./{}-core.js", layout.base_name);
    let extension_exports = extension_files
        .iter()
        .filter_map(|path| path.strip_suffix(".ts"))
        .map(|path| format!("export * from './{path}.js';"))
        .collect::<Vec<_>>()
        .join("\n");

    if !hosted_program_extensions.is_empty() {
        let sdk_import = if extension_entry.is_some() {
            "import { extendPrograms, extendStack } from '@usearete/sdk';"
        } else {
            "import { extendPrograms } from '@usearete/sdk';"
        };
        let hosted_imports = hosted_program_extensions
            .iter()
            .map(|extension| {
                let entry = extension
                    .artifact
                    .entry
                    .strip_suffix(".ts")
                    .unwrap_or(&extension.artifact.entry);
                format!("import {} from './{}.js';", extension.import_name, entry)
            })
            .collect::<Vec<_>>()
            .join("\n");
        let stack_program_lines = program_extension_bindings
            .iter()
            .map(|binding| format!("    {}: {},", binding.program_key, binding.export_name))
            .collect::<Vec<_>>();
        let hosted_program_lines = hosted_program_extensions
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
{extension_exports}

const HOSTED_PROGRAMS = extendPrograms({core_export_name}.programs, {{
{hosted_program_lines}
}});{stack_program_layer}

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
{extension_exports}

export const {export_name} = extendStack(
  {core_export_name},
  stackExtensions
);

export type {type_name} = typeof {export_name};

export default {export_name};"#,
                core_export_name = core_export_name,
                core_import = core_import,
                extension_exports = extension_exports,
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
{extension_exports}

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
                extension_exports = extension_exports,
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
            r#"import {{ extendProgram }} from '@usearete/sdk';

import {{ {core_const_name} as {core_import_name}, {core_read_const_name} as {core_read_import_name} }} from '{core_import}';
import programExtensions from './{extension_runtime_import}';

export * from '{core_import}';
export {{ {core_const_name} as {core_import_name} }} from '{core_import}';
export * from './{extension_runtime_import}';

export const {export_name} = extendProgram({core_import_name}, programExtensions);
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
            r#"import {{ {core_const_name} as {core_import_name}, {core_read_const_name} as {core_read_import_name} }} from '{core_import}';

export * from '{core_import}';
export {{ {core_const_name} as {core_import_name} }} from '{core_import}';

export const {export_name} = {core_import_name};
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
    hosted_program_extensions: &[HostedProgramExtension],
) -> String {
    let export_name = format!("{}_PROGRAMS", to_screaming_snake_case(stack_name));
    let core_export_name = format!("{}_CORE", export_name);
    let type_name = format!("{}Programs", stack_name);
    let core_import = format!("./{}-core.js", layout.base_name);

    if !hosted_program_extensions.is_empty() {
        let hosted_imports = hosted_program_extensions
            .iter()
            .map(|extension| {
                let entry = extension
                    .artifact
                    .entry
                    .strip_suffix(".ts")
                    .unwrap_or(&extension.artifact.entry);
                format!("import {} from './{}.js';", extension.import_name, entry)
            })
            .collect::<Vec<_>>()
            .join("\n");
        let hosted_lines = hosted_program_extensions
            .iter()
            .map(|extension| format!("  {}: {},", extension.program_key, extension.import_name))
            .collect::<Vec<_>>()
            .join("\n");
        let (explicit_import, base_expression, extension_export) = extension_entry
            .map(|entry| entry.strip_suffix(".ts").unwrap_or(entry))
            .map(|entry| {
                (
                    format!("import programExtensions from './{entry}.js';"),
                    format!("extendPrograms({core_export_name}, programExtensions)"),
                    format!("export * from './{entry}.js';"),
                )
            })
            .unwrap_or_else(|| (String::new(), core_export_name.clone(), String::new()));

        return finish_typescript_module(format!(
            r#"import {{ extendPrograms }} from '@usearete/sdk';

import {{ {export_name} as {core_export_name} }} from '{core_import}';
{explicit_import}
{hosted_imports}

export * from '{core_import}';
{extension_export}

const BASE_PROGRAMS = {base_expression};

export const {export_name} = extendPrograms(BASE_PROGRAMS, {{
{hosted_lines}
}});

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
export * from './{extension_runtime_import}';

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

fn find_stack_by_name(
    client: &ApiClient,
    stack_name: &str,
    output_override: Option<String>,
    package_name_override: Option<String>,
) -> Result<(ResolvedStackSource, PathBuf, String)> {
    let source = resolve_stack_source(client, stack_name)?;

    let output = output_override
        .map(|p| p.into())
        .unwrap_or_else(|| default_typescript_output_dir(source.sdk_name()));

    let pkg = package_name_override.unwrap_or_else(|| "@usearete/react".to_string());

    Ok((source, output, pkg))
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
    let mut program = arete_interpreter::typescript::TypeScriptProgramConfig::from(&identity);
    program.definition.sdk_definition_hash = Some(program_definition_hash(
        &program.definition.program_spec_hash,
    )?);

    write_typescript_program_sdk(
        &sdk_name,
        &program_name,
        stack_spec,
        output_path,
        package_name,
        TypeScriptProgramSdkExtensions {
            input_pin: &input_pin,
            programs: Some(vec![program]),
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
    let mut program = arete_interpreter::typescript::TypeScriptProgramConfig::from(&identity);
    program.definition.sdk_definition_hash = Some(program_definition_hash(
        &program.definition.program_spec_hash,
    )?);
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
            path: extensions_path,
            hosted_artifact: None,
        },
    )
}

struct TypeScriptProgramSdkExtensions<'a> {
    input_pin: &'a ResolvedExtensionsInputPin,
    programs: Option<Vec<arete_interpreter::typescript::TypeScriptProgramConfig>>,
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

    fs::write(&layout.core_path, output.full_file()).with_context(|| {
        format!(
            "Failed to write TypeScript core module to {}",
            layout.core_path.display()
        )
    })?;

    let artifact =
        resolve_extensions_artifact(extensions.path, &layout, extensions.hosted_artifact)?;
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
    let hosted_program_extensions = hosted_program_extensions(source, &stack_spec)?;

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

        fs::write(&layout.core_path, output.full_file()).with_context(|| {
            format!(
                "Failed to write TypeScript core module to {}",
                layout.core_path.display()
            )
        })?;

        let artifact = resolve_extensions_artifact(extensions_path, &layout, None)?;
        if let Some(ref artifact) = artifact {
            stage_extensions_artifact(artifact, &layout.output_dir, &input_pin)?;
        }
        stage_hosted_program_extensions(&hosted_program_extensions, &layout, &stack_name, true)?;

        let entry_contents = render_typescript_program_collection_entry(
            &layout,
            &stack_name,
            artifact.as_ref().map(|artifact| artifact.entry.as_str()),
            &hosted_program_extensions,
        );

        fs::write(&layout.entry_path, entry_contents).with_context(|| {
            format!(
                "Failed to write TypeScript entry module to {}",
                layout.entry_path.display()
            )
        })?;
        write_sdk_provenance_manifest(&layout, &input_pin, artifact.as_ref())?;
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

        fs::write(&layout.core_path, output.full_file()).with_context(|| {
            format!(
                "Failed to write TypeScript core module to {}",
                layout.core_path.display()
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
        )?;
        if let Some(ref artifact) = artifact {
            stage_extensions_artifact(artifact, &layout.output_dir, &input_pin)?;
        }
        stage_hosted_program_extensions(&hosted_program_extensions, &layout, &stack_name, false)?;
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
            &hosted_program_extensions,
        );
        fs::write(&layout.entry_path, entry_contents).with_context(|| {
            format!(
                "Failed to write TypeScript entry module to {}",
                layout.entry_path.display()
            )
        })?;
        write_sdk_provenance_manifest(&layout, &input_pin, artifact.as_ref())?;
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

fn load_stack_spec_from_file(
    ast: &DiscoveredAst,
    require_entities: bool,
) -> Result<arete_interpreter::ast::SerializableStackSpec> {
    let ast_json = fs::read_to_string(&ast.path)
        .with_context(|| format!("Failed to read stack file: {}", ast.path.display()))?;

    load_stack_spec_from_json(&ast_json, &ast.path.display().to_string(), require_entities)
}

fn load_stack_spec_from_json(
    ast_json: &str,
    source_name: &str,
    require_entities: bool,
) -> Result<arete_interpreter::ast::SerializableStackSpec> {
    // Use versioned loader for automatic version detection and migration
    let stack_spec = arete_interpreter::versioned::load_stack_spec(ast_json)
        .with_context(|| format!("Failed to load stack AST from {}", source_name))?;

    if require_entities && stack_spec.entities.is_empty() {
        return Err(anyhow::anyhow!(
            "Stack AST contains no entities: {}",
            source_name
        ));
    }

    Ok(stack_spec)
}

#[allow(clippy::too_many_arguments)]
pub fn create_rust(
    config_path: &str,
    stack_name: Option<&str>,
    output_override: Option<String>,
    crate_name_override: Option<String>,
    module_flag: bool,
    url_override: Option<String>,
    manifest_override: Option<String>,
    artifact_dirs: Vec<String>,
) -> Result<()> {
    let config = AreteConfig::load_optional(config_path)?;
    let client = ApiClient::new()?;

    let config_dir = Path::new(config_path)
        .parent()
        .unwrap_or(Path::new("."))
        .to_path_buf();

    let stack_config = stack_name.and_then(|name| config.as_ref().and_then(|c| c.find_stack(name)));

    let as_module = module_flag
        || stack_config.and_then(|s| s.rust_module).unwrap_or_else(|| {
            config
                .as_ref()
                .and_then(|c| c.sdk.as_ref())
                .map(|s| s.rust_module_mode)
                .unwrap_or(false)
        });

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
        find_stack_for_rust(
            &client,
            stack_name,
            config.as_ref(),
            output_override,
            crate_name_override,
        )?
    };

    let stack_url = url_override
        .or_else(|| stack_config.and_then(|s| s.url.clone()))
        .or_else(|| source.default_websocket_url());

    let output_dir = if raw_output_dir.is_relative() {
        config_dir.join(&raw_output_dir)
    } else {
        raw_output_dir
    };

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
        let live_urls = match &source {
            ResolvedStackSource::Remote(stack) => stack
                .live_bindings
                .iter()
                .map(|live| (live.alias.clone(), live.binding.websocket_endpoint.clone()))
                .collect(),
            ResolvedStackSource::Local(_) | ResolvedStackSource::LocalArtifacts(_) => {
                BTreeMap::new()
            }
        };
        let output = arete_interpreter::rust::compile_composed_public_artifacts_v2(
            composition.program_specs,
            composition.live_specs,
            composition.stack_manifest,
            Some(arete_interpreter::rust::RustCompositionConfig {
                stack: arete_interpreter::rust::RustStackConfig {
                    crate_name: crate_name.clone(),
                    sdk_version: "0.3".to_string(),
                    module_mode: as_module,
                    url: None,
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

    let rust_config = arete_interpreter::rust::RustStackConfig {
        crate_name: crate_name.clone(),
        sdk_version: "0.3".to_string(),
        module_mode: as_module,
        url: stack_url,
    };

    let output = match &source {
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

    if as_module {
        arete_interpreter::rust::write_rust_module(&output, &output_dir)
            .with_context(|| format!("Failed to write Rust module to {}", output_dir.display()))?;

        println!("{} Successfully generated Rust module!", "✓".green().bold());
        println!("  Module: {}", output_dir.display().to_string().bold());
        println!("\n  Add to your lib.rs:");
        let module_name = output_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("module");
        println!("    pub mod {};", module_name.cyan());
    } else {
        arete_interpreter::rust::write_rust_crate(&output, &output_dir)
            .with_context(|| format!("Failed to write Rust crate to {}", output_dir.display()))?;

        println!("{} Successfully generated Rust SDK!", "✓".green().bold());
        println!("  Crate: {}", output_dir.display().to_string().bold());
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

fn find_stack_for_rust(
    client: &ApiClient,
    stack_name: &str,
    config: Option<&AreteConfig>,
    output_override: Option<String>,
    crate_name_override: Option<String>,
) -> Result<(ResolvedStackSource, PathBuf, String)> {
    let (source, stack_config) = if let Some(cfg) = config {
        if let Some(stack_config) = cfg.find_stack(stack_name) {
            let source = resolve_stack_source(client, &stack_config.stack)?;
            (source, Some(stack_config))
        } else {
            let source = resolve_stack_source(client, stack_name)?;
            (source, None)
        }
    } else {
        let source = resolve_stack_source(client, stack_name)?;
        (source, None)
    };

    let crate_name = crate_name_override.unwrap_or_else(|| format!("{}-stack", source.sdk_name()));

    let crate_dir = if let Some(cfg) = config {
        cfg.get_rust_output_path(source.sdk_name(), stack_config, output_override)
    } else {
        output_override
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(format!("./generated/{}-stack", source.sdk_name())))
    };

    Ok((source, crate_dir, crate_name))
}

fn resolve_stack_source(client: &ApiClient, stack: &str) -> Result<ResolvedStackSource> {
    if let Some(ast) = find_ast_file(stack, None)? {
        return Ok(ResolvedStackSource::Local(ast));
    }

    let remote = client.get_registry_stack_install(stack).with_context(|| {
        format!(
            "Stack '{}' was not found locally and no accessible hosted stack with that identifier was found.",
            stack
        )
    })?;

    Ok(ResolvedStackSource::Remote(Box::new(remote_stack_install(
        remote,
    )?)))
}

fn resolve_remote_stack_source(client: &ApiClient, stack: &str) -> Result<ResolvedStackSource> {
    let remote = client.get_registry_stack_install(stack).with_context(|| {
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
            sdk_extension_hash: None,
            sdk_output_tree_hash: None,
            program_extension_bindings: vec![],
        }
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
        assert_eq!(
            first
                .extensions
                .as_ref()
                .unwrap()
                .legacy_provenance_sha256
                .len(),
            64
        );
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
        assert_eq!(manifest.artifacts, vec!["demo-core.ts", "demo.ts"]);
        assert!(!first.contains(&output_dir.display().to_string()));
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
            ast_content_hash: "ast-hash".to_string(),
            portable_ast_hash: "portable-ast-hash".to_string(),
            ast_payload: serde_json::json!({"stack_name": "ignored-legacy-name"}),
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
        assert!(rendered.contains("export * from './squads-v4-extensions.js';"));
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
        assert!(rendered.contains("export * from './squads-v4-devex.js';"));
        assert!(rendered.contains("export * from './squads-v4-extensions.js';"));
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
        let hosted = HostedProgramExtension {
            program_key: "splToken".to_string(),
            program_const_name: "TOKEN".to_string(),
            import_name: "hostedSplTokenProgramExtensions".to_string(),
            input_pin: ResolvedExtensionsInputPin {
                kind: ExtensionsInputKind::ProgramSpec,
                hash: "program-spec-hash".to_string(),
            },
            artifact,
        };
        let rendered = render_typescript_stack_entry(
            &layout("token-stack"),
            "TokenStack",
            None,
            &[],
            &[],
            &[hosted],
        );

        assert!(rendered
            .contains("import hostedSplTokenProgramExtensions from './spl-token-extensions.js';"));
        assert!(rendered.contains("...TOKEN_STACK_STACK_CORE,"));
        assert!(rendered
            .contains("const HOSTED_PROGRAMS = extendPrograms(TOKEN_STACK_STACK_CORE.programs, {"));
        assert!(rendered.contains("programs: HOSTED_PROGRAMS,"));
        assert!(rendered.contains("splToken: hostedSplTokenProgramExtensions,"));
        assert!(!rendered.contains("programReads:"));
    }

    #[test]
    fn hosted_program_extension_staging_writes_a_stack_core_proxy() {
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
        let mut artifact = test_artifact(ExtensionsInputKind::ProgramSpec, "typed-hash");
        artifact.entry = "spl-token-extensions.ts".to_string();
        artifact.files[0] = ResolvedExtensionsFile {
            path: artifact.entry.clone(),
            contents: "import { TOKEN } from './spl-token-core';\nexport default {};".to_string(),
        };
        let hosted = HostedProgramExtension {
            program_key: "splToken".to_string(),
            program_const_name: "TOKEN".to_string(),
            import_name: "hostedSplTokenProgramExtensions".to_string(),
            input_pin: ResolvedExtensionsInputPin {
                kind: ExtensionsInputKind::ProgramSpec,
                hash: "typed-hash".to_string(),
            },
            artifact,
        };

        stage_hosted_program_extensions(&[hosted], &layout, "OrderedStream", false)
            .expect("hosted program extension should stage");
        let proxy =
            fs::read_to_string(output_dir.join("spl-token-core.ts")).expect("program core proxy");
        let _ = fs::remove_dir_all(&output_dir);

        assert!(proxy.contains("export * from './ordered-stream-core.js';"));
        assert!(proxy.contains("export const TOKEN = ORDERED_STREAM_STACK_CORE.programs.splToken;"));
    }

    #[test]
    fn render_typescript_program_entry_without_extensions_aliases_core() {
        let rendered = render_typescript_program_entry(&layout("spl-token"), "token", None);

        assert!(rendered.contains("import { TOKEN as SPL_TOKEN_PROGRAM_CORE, TOKEN_READ as SPL_TOKEN_PROGRAM_READ_CORE } from './spl-token-core.js';"));
        assert!(rendered
            .contains("export { TOKEN as SPL_TOKEN_PROGRAM_CORE } from './spl-token-core.js';"));
        assert!(rendered.contains("export const SPL_TOKEN_PROGRAM = SPL_TOKEN_PROGRAM_CORE;"));
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

        assert!(rendered.contains("import { extendProgram } from '@usearete/sdk';"));
        assert!(rendered.contains("import { SYSTEM_PROGRAM as SYSTEM_PROGRAM_CORE, SYSTEM_PROGRAM_READ as SYSTEM_PROGRAM_READ_CORE } from './system-program-core.js';"));
        assert!(rendered.contains(
            "export { SYSTEM_PROGRAM as SYSTEM_PROGRAM_CORE } from './system-program-core.js';"
        ));
        assert!(
            rendered.contains("import programExtensions from './system-program-extensions.js';")
        );
        assert!(rendered.contains("export * from './system-program-extensions.js';"));
        assert!(rendered.contains(
            "export const SYSTEM_PROGRAM = extendProgram(SYSTEM_PROGRAM_CORE, programExtensions);"
        ));
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
        assert!(rendered.contains("export * from './ore-program-extensions.js';"));
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
    fn resolve_sync_stack_names_matches_by_alias_and_stack_id() {
        let config = AreteConfig {
            project: crate::config::ProjectConfig {
                name: "demo".to_string(),
            },
            stacks: vec![
                crate::config::StackConfig {
                    name: Some("ore-main".to_string()),
                    stack: "OreStream".to_string(),
                    description: None,
                    typescript_output_file: None,
                    rust_output_crate: None,
                    rust_module: None,
                    url: None,
                },
                crate::config::StackConfig {
                    name: None,
                    stack: "EntropyStream".to_string(),
                    description: None,
                    typescript_output_file: None,
                    rust_output_crate: None,
                    rust_module: None,
                    url: None,
                },
            ],
            sdk: None,
            build: None,
        };

        let filtered = resolve_sync_stack_names(
            &config,
            &BTreeSet::from(["ore-main".to_string(), "EntropyStream".to_string()]),
        )
        .expect("filters should match configured stacks");

        assert_eq!(
            filtered,
            vec!["ore-main".to_string(), "EntropyStream".to_string()]
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
        let definition_hash = program_definition_hash(&identity.program_spec_hash.to_string())
            .expect("definition hash");
        let mut program = arete_interpreter::typescript::TypeScriptProgramConfig::from(&identity);
        program.definition.sdk_definition_hash = Some(definition_hash.clone());
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
            .contains(&format!("sdkDefinitionHash: '{definition_hash}',")));
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
        };
        let stack_spec =
            arete_interpreter::program_sdk::build_program_only_stack_spec_from_identity(
                &identity, "Presale",
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
        let expected_definition_hash =
            program_definition_hash(&identity.program_spec_hash.to_string())
                .expect("definition hash");

        assert!(generated.contains(&format!("sdkDefinitionHash: '{expected_definition_hash}',")));
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
                r#"{{"address":"{}","metadata":{{"name":"{}","version":"1.0.0","spec":"0.1.0"}},"instructions":[],"accounts":[],"types":[],"events":[],"errors":[]}}"#,
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
                ast_content_hash: "public-ast".into(),
                portable_ast_hash: "portable-ast".into(),
                ast_payload: serde_json::json!({}),
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
    fn local_multi_live_generation_writes_namespaced_typescript_and_rust_modules() {
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
        fs::remove_dir_all(directory).unwrap();
    }
}
