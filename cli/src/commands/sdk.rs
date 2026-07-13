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
    ApiClient, RegistryProgramInstallResponse, RegistrySdkExtensionArtifact,
    RegistrySdkExtensionInputKind, RegistryStackInstallResponse,
};
use crate::config::{discover_ast_files, find_ast_file, to_kebab_case, AreteConfig, DiscoveredAst};
use crate::telemetry;

struct RemoteStackAst {
    name: String,
    stack: String,
    websocket_url: String,
    ast_payload: serde_json::Value,
    sdk_name: String,
    hosted_extensions: Option<ResolvedExtensionsArtifact>,
}

enum ResolvedStackSource {
    Local(DiscoveredAst),
    Remote(RemoteStackAst),
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
    ProgramIdl,
}

impl ExtensionsInputKind {
    fn as_manifest_value(self) -> &'static str {
        match self {
            Self::StackAst => "stack-ast",
            Self::ProgramIdl => "program-idl",
        }
    }

    fn from_registry(kind: RegistrySdkExtensionInputKind) -> Self {
        match kind {
            RegistrySdkExtensionInputKind::StackAst => Self::StackAst,
            RegistrySdkExtensionInputKind::ProgramIdl => Self::ProgramIdl,
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
struct TypeScriptLayout {
    output_dir: PathBuf,
    base_name: String,
    entry_path: PathBuf,
    core_path: PathBuf,
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
            Self::Remote(stack) => stack.stack.as_str(),
        }
    }

    fn sdk_name(&self) -> &str {
        match self {
            Self::Local(ast) => ast.stack_name.as_str(),
            Self::Remote(stack) => stack.sdk_name.as_str(),
        }
    }

    fn default_url(&self) -> Option<String> {
        match self {
            Self::Local(_) => None,
            Self::Remote(stack) => Some(stack.websocket_url.clone()),
        }
    }

    fn print_source_details(&self) {
        match self {
            Self::Local(ast) => {
                println!("  Path: {}", ast.path.display());
                if !ast.program_ids.is_empty() {
                    println!("  Program IDs: {}", ast.program_ids.join(", "));
                }
            }
            Self::Remote(stack) => {
                println!("  Hosted Stack: {}", stack.stack.cyan());
                println!("  Stack Name: {}", stack.name);
            }
        }
    }

    fn load_stack_spec(
        &self,
        require_entities: bool,
    ) -> Result<arete_interpreter::ast::SerializableStackSpec> {
        match self {
            Self::Local(ast) => load_stack_spec_from_file(ast, require_entities),
            Self::Remote(stack) => load_stack_spec_from_value(
                &stack.ast_payload,
                &format!("hosted stack '{}'", stack.stack),
                require_entities,
            ),
        }
    }

    fn hosted_extensions(&self) -> Option<&ResolvedExtensionsArtifact> {
        match self {
            Self::Local(_) => None,
            Self::Remote(stack) => stack.hosted_extensions.as_ref(),
        }
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
                false,
            )?;
        }
        if sync_rust {
            create_rust(config_path, &stack_name, None, None, false, None)?;
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
            program_only,
        ),
        SdkTarget::Rust => {
            if program_only {
                return Err(anyhow::anyhow!(
                    "--program-only is only supported for TypeScript SDKs (--ts)"
                ));
            }
            let stack_name = stack_name.ok_or_else(|| {
                anyhow::anyhow!("stack name is required unless using --idl with --program-only")
            })?;
            create_rust(
                config_path,
                stack_name,
                output_override,
                crate_name_override,
                module_flag,
                url_override,
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
    program_only: bool,
) -> Result<()> {
    let config = AreteConfig::load_optional(config_path)?;

    // Get the config file's directory for resolving relative paths
    let config_dir = Path::new(config_path)
        .parent()
        .unwrap_or(Path::new("."))
        .to_path_buf();

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

    let stack_name = stack_name.ok_or_else(|| {
        anyhow::anyhow!("stack name is required unless using --idl with --program-only")
    })?;

    println!(
        "{} Looking for stack '{}'...",
        "→".blue().bold(),
        stack_name
    );

    let client = ApiClient::new()?;

    let (source, output_path, package_name, stack_url) = if let Some(ref cfg) = config {
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

            let url = url_override
                .or_else(|| stack_config.url.clone())
                .or_else(|| source.default_url());

            (source, output, pkg, url)
        } else {
            let (source, output, pkg) =
                find_stack_by_name(&client, stack_name, output_override, package_name_override)?;
            let url = url_override.or_else(|| source.default_url());
            (source, output, pkg, url)
        }
    } else {
        let (source, output, pkg) =
            find_stack_by_name(&client, stack_name, output_override, package_name_override)?;
        let url = url_override.or_else(|| source.default_url());
        (source, output, pkg, url)
    };

    println!(
        "{} Found stack: {}",
        "✓".green().bold(),
        source.stack_id().bold()
    );
    source.print_source_details();
    println!("  Output: {}", output_path.display());
    if let Some(url) = &stack_url {
        println!("  URL: {}", url.cyan());
    } else {
        println!(
            "  URL: {}",
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
        stack_url,
        extensions_override.as_deref().map(Path::new),
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
    let stack_url = url_override.or_else(|| source.default_url());

    println!(
        "{} Found hosted stack: {}",
        "✓".green().bold(),
        source.stack_id().bold()
    );
    source.print_source_details();
    println!("  Output: {}", output_path.display());
    if let Some(url) = &stack_url {
        println!("  URL: {}", url.cyan());
    } else {
        println!(
            "  URL: {}",
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
        stack_url,
        extensions_override.as_deref().map(Path::new),
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
    let stack_url = url_override.or_else(|| source.default_url());

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

    let stack_spec = source.load_stack_spec(true)?;

    println!(
        "{} {} entities in stack",
        "→".blue().bold(),
        stack_spec.entities.len()
    );

    let rust_config = arete_interpreter::rust::RustStackConfig {
        crate_name: crate_name.clone(),
        sdk_version: "0.2".to_string(),
        module_mode: module_flag,
        url: stack_url,
    };

    let output = arete_interpreter::rust::compile_stack_spec(stack_spec, Some(rust_config))
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

    let sdk_name = install
        .install_name
        .clone()
        .unwrap_or_else(|| to_kebab_case(&install.name));
    let output_path = output_override
        .map(PathBuf::from)
        .unwrap_or_else(|| default_typescript_output_dir(&sdk_name));
    let package_name = package_name_override.unwrap_or_else(|| "@usearete/react".to_string());

    println!(
        "{} Found hosted program: {}",
        "✓".green().bold(),
        install
            .install_name
            .as_deref()
            .unwrap_or(&install.program_id)
            .bold()
    );
    println!("  Program ID: {}", install.program_id);
    println!("  IDL Name: {}", install.name);
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
    let Ok(range) = VersionReq::parse(range) else {
        return false;
    };
    range.matches(&current)
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

fn compute_idl_content_hash_from_value(idl_payload: &serde_json::Value) -> Result<String> {
    let json = serde_json::to_string(idl_payload)
        .context("Failed to serialize IDL payload for hashing")?;
    Ok(Sha256::digest(json.as_bytes())
        .iter()
        .map(|byte| format!("{:02x}", byte))
        .collect())
}

fn stack_ast_input_pin(
    stack_spec: &arete_interpreter::ast::SerializableStackSpec,
) -> ResolvedExtensionsInputPin {
    ResolvedExtensionsInputPin {
        kind: ExtensionsInputKind::StackAst,
        hash: stack_spec
            .content_hash
            .clone()
            .unwrap_or_else(|| stack_spec.compute_content_hash()),
    }
}

fn program_idl_input_pin(idl_path: &Path) -> Result<ResolvedExtensionsInputPin> {
    let idl_json = fs::read_to_string(idl_path).with_context(|| {
        format!(
            "Failed to read IDL file for hashing: {}",
            idl_path.display()
        )
    })?;
    let idl_payload: serde_json::Value = serde_json::from_str(&idl_json)
        .with_context(|| format!("Failed to parse IDL JSON from {}", idl_path.display()))?;

    Ok(ResolvedExtensionsInputPin {
        kind: ExtensionsInputKind::ProgramIdl,
        hash: compute_idl_content_hash_from_value(&idl_payload)?,
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

    build_extensions_artifact(
        artifact.manifest.entry.clone(),
        files,
        artifact
            .manifest
            .input_kind
            .clone()
            .map(ExtensionsInputKind::from_registry),
        artifact.manifest.input_hash.clone(),
        artifact.manifest.sdk_range.clone(),
    )
}

fn stage_extensions_artifact(
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

    let manifest_path = output_dir.join("extensions.json");
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
    program_extension_bindings: &[ProgramExtensionBinding],
) -> String {
    let export_name = format!("{}_STACK", to_screaming_snake_case(stack_name));
    let core_export_name = format!("{}_CORE", export_name);
    let type_name = format!("{}Stack", stack_name);
    let core_import = format!("./{}-core.js", layout.base_name);

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
    let type_name = format!("{}Program", to_pascal_case(&layout.base_name));
    let core_import = format!("./{}-core.js", layout.base_name);

    if let Some(extension_entry) = extension_entry {
        let extension_import = extension_entry
            .strip_suffix(".ts")
            .unwrap_or(extension_entry);
        let extension_runtime_import = format!("{}.js", extension_import);
        finish_typescript_module(format!(
            r#"import {{ extendProgram }} from '@usearete/sdk';

import {{ {core_const_name} as {core_import_name} }} from '{core_import}';
import programExtensions from './{extension_runtime_import}';

export * from '{core_import}';
export {{ {core_const_name} as {core_import_name} }} from '{core_import}';
export * from './{extension_runtime_import}';

export const {export_name} = extendProgram({core_import_name}, programExtensions);

export type {type_name} = typeof {export_name};

export default {export_name};"#,
            core_const_name = core_const_name,
            core_import_name = core_import_name,
            core_import = core_import,
            extension_runtime_import = extension_runtime_import,
            export_name = export_name,
            type_name = type_name,
        ))
    } else {
        finish_typescript_module(format!(
            r#"import {{ {core_const_name} as {core_import_name} }} from '{core_import}';

export * from '{core_import}';
export {{ {core_const_name} as {core_import_name} }} from '{core_import}';

export const {export_name} = {core_import_name};

export type {type_name} = typeof {export_name};

export default {export_name};"#,
            core_const_name = core_const_name,
            core_import_name = core_import_name,
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
) -> String {
    let export_name = format!("{}_PROGRAMS", to_screaming_snake_case(stack_name));
    let core_export_name = format!("{}_CORE", export_name);
    let type_name = format!("{}Programs", stack_name);
    let core_import = format!("./{}-core.js", layout.base_name);

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
    let idl = arete_idl::parse::parse_idl_file(idl_path)
        .map_err(|e| anyhow::anyhow!("Failed to parse IDL {}: {}", idl_path.display(), e))?;
    let sdk_name = idl_sdk_name_from_path(idl_path)?;
    let stack_name = to_pascal_case(&sdk_name);
    let program_name = idl.get_name().to_string();
    let input_pin = program_idl_input_pin(idl_path)?;
    let stack_spec =
        arete_interpreter::program_sdk::build_program_only_stack_spec_from_idl(&idl, &stack_name);

    write_typescript_program_sdk(
        &sdk_name,
        &program_name,
        stack_spec,
        output_path,
        package_name,
        TypeScriptProgramSdkExtensions {
            input_pin: &input_pin,
            path: extensions_path,
            hosted_artifact: None,
        },
    )
}

struct TypeScriptProgramSdkExtensions<'a> {
    input_pin: &'a ResolvedExtensionsInputPin,
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
            url: None,
            extension_import: None,
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
    let idl_json = serde_json::to_string(&install.idl_payload)
        .context("Failed to serialize hosted program IDL")?;
    let idl = arete_idl::parse::parse_idl_content(&idl_json).map_err(|e| {
        anyhow::anyhow!(
            "Failed to parse hosted program IDL for '{}': {}",
            install.program_id,
            e
        )
    })?;
    let stack_name = to_pascal_case(sdk_name);
    let input_pin = ResolvedExtensionsInputPin {
        kind: ExtensionsInputKind::ProgramIdl,
        hash: install.idl_content_hash.clone(),
    };
    let stack_spec =
        arete_interpreter::program_sdk::build_program_only_stack_spec_from_idl(&idl, &stack_name);

    write_typescript_program_sdk(
        sdk_name,
        idl.get_name(),
        stack_spec,
        output_path,
        package_name,
        TypeScriptProgramSdkExtensions {
            input_pin: &input_pin,
            path: extensions_path,
            hosted_artifact,
        },
    )
}

fn generate_typescript_sdk_from_source(
    source: &ResolvedStackSource,
    output_path: &Path,
    package_name: &str,
    url: Option<String>,
    extensions_path: Option<&Path>,
    program_only: bool,
) -> Result<()> {
    let stack_spec = source.load_stack_spec(!program_only)?;
    let input_pin = stack_ast_input_pin(&stack_spec);

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
            url,
            extension_import: None,
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

        let entry_contents = render_typescript_program_collection_entry(
            &layout,
            &stack_name,
            artifact.as_ref().map(|artifact| artifact.entry.as_str()),
        );

        fs::write(&layout.entry_path, entry_contents).with_context(|| {
            format!(
                "Failed to write TypeScript entry module to {}",
                layout.entry_path.display()
            )
        })?;
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
            url,
            extension_import: None,
        };

        let output =
            arete_interpreter::typescript::compile_stack_spec(stack_spec.clone(), Some(config))
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

        let entry_contents = render_typescript_stack_entry(
            &layout,
            &stack_name,
            artifact.as_ref().map(|artifact| artifact.entry.as_str()),
            artifact
                .as_ref()
                .map(|artifact| artifact.program_extension_bindings.as_slice())
                .unwrap_or(&[]),
        );
        fs::write(&layout.entry_path, entry_contents).with_context(|| {
            format!(
                "Failed to write TypeScript entry module to {}",
                layout.entry_path.display()
            )
        })?;
    }

    Ok(())
}

fn load_stack_spec_from_file(
    ast: &DiscoveredAst,
    require_entities: bool,
) -> Result<arete_interpreter::ast::SerializableStackSpec> {
    let ast_json = fs::read_to_string(&ast.path)
        .with_context(|| format!("Failed to read stack file: {}", ast.path.display()))?;

    load_stack_spec_from_json(&ast_json, &ast.path.display().to_string(), require_entities)
}

fn load_stack_spec_from_value(
    ast: &serde_json::Value,
    source_name: &str,
    require_entities: bool,
) -> Result<arete_interpreter::ast::SerializableStackSpec> {
    let ast_json = serde_json::to_string(ast)
        .with_context(|| format!("Failed to serialize stack AST from {}", source_name))?;

    load_stack_spec_from_json(&ast_json, source_name, require_entities)
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

pub fn create_rust(
    config_path: &str,
    stack_name: &str,
    output_override: Option<String>,
    crate_name_override: Option<String>,
    module_flag: bool,
    url_override: Option<String>,
) -> Result<()> {
    println!(
        "{} Looking for stack '{}'...",
        "→".blue().bold(),
        stack_name
    );

    let config = AreteConfig::load_optional(config_path)?;
    let client = ApiClient::new()?;

    let config_dir = Path::new(config_path)
        .parent()
        .unwrap_or(Path::new("."))
        .to_path_buf();

    let stack_config = config.as_ref().and_then(|c| c.find_stack(stack_name));

    let as_module = module_flag
        || stack_config.and_then(|s| s.rust_module).unwrap_or_else(|| {
            config
                .as_ref()
                .and_then(|c| c.sdk.as_ref())
                .map(|s| s.rust_module_mode)
                .unwrap_or(false)
        });

    let (source, raw_output_dir, crate_name) = find_stack_for_rust(
        &client,
        stack_name,
        config.as_ref(),
        output_override,
        crate_name_override,
    )?;

    let stack_url = url_override
        .or_else(|| stack_config.and_then(|s| s.url.clone()))
        .or_else(|| source.default_url());

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

    let stack_spec = source.load_stack_spec(true)?;

    println!(
        "{} {} entities in stack",
        "→".blue().bold(),
        stack_spec.entities.len()
    );

    let rust_config = arete_interpreter::rust::RustStackConfig {
        crate_name: crate_name.clone(),
        sdk_version: "0.2".to_string(),
        module_mode: as_module,
        url: stack_url,
    };

    let output = arete_interpreter::rust::compile_stack_spec(stack_spec, Some(rust_config))
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

    Ok(ResolvedStackSource::Remote(remote_stack_install(remote)?))
}

fn resolve_remote_stack_source(client: &ApiClient, stack: &str) -> Result<ResolvedStackSource> {
    let remote = client.get_registry_stack_install(stack).with_context(|| {
        format!(
            "No accessible hosted stack with identifier '{}' was found.",
            stack
        )
    })?;

    Ok(ResolvedStackSource::Remote(remote_stack_install(remote)?))
}

fn remote_stack_sdk_name(ast_payload: &serde_json::Value, registry_name: &str) -> String {
    ast_payload
        .get("stack_name")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|stack_name| !stack_name.is_empty())
        .map(to_kebab_case)
        .filter(|sdk_name| {
            sdk_name.split('-').all(|segment| {
                !segment.is_empty() && segment.chars().all(|ch| ch.is_ascii_alphanumeric())
            })
        })
        .unwrap_or_else(|| to_kebab_case(registry_name))
}

fn remote_stack_install(remote: RegistryStackInstallResponse) -> Result<RemoteStackAst> {
    let sdk_name = remote_stack_sdk_name(&remote.ast_payload, &remote.name);

    Ok(RemoteStackAst {
        sdk_name,
        name: remote.name,
        stack: remote.stack,
        websocket_url: remote.websocket_url,
        ast_payload: remote.ast_payload,
        hosted_extensions: remote
            .extensions
            .as_ref()
            .map(resolved_extensions_artifact_from_registry)
            .transpose()?,
    })
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
            program_extension_bindings: vec![],
        }
    }

    fn registry_stack_install(
        name: &str,
        ast_payload: serde_json::Value,
    ) -> RegistryStackInstallResponse {
        RegistryStackInstallResponse {
            name: name.to_string(),
            stack: "stack-id".to_string(),
            websocket_url: "wss://stack.example.com".to_string(),
            http_url: "https://stack.example.com".to_string(),
            websocket_auth: serde_json::json!({}),
            http_auth: serde_json::json!({}),
            description: None,
            visibility: "public".to_string(),
            spec_version_id: Some(1),
            ast_content_hash: "ast-hash".to_string(),
            ast_payload,
            extensions: None,
        }
    }

    #[test]
    fn remote_stack_uses_ast_stack_name_for_typescript_basename() {
        let remote = remote_stack_install(registry_stack_install(
            "squads-v4",
            serde_json::json!({ "stack_name": "SquadsV4Stream" }),
        ))
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
    fn remote_stack_falls_back_to_registry_name_without_valid_ast_stack_name() {
        for ast_payload in [
            serde_json::json!({}),
            serde_json::json!({ "stack_name": "not/a/stack" }),
        ] {
            let remote = remote_stack_install(registry_stack_install("squads-v4", ast_payload))
                .expect("remote stack should resolve");

            assert_eq!(remote.sdk_name, "squads-v4");
        }
    }

    #[test]
    fn render_typescript_stack_entry_without_extensions_aliases_core() {
        let rendered = render_typescript_stack_entry(
            &layout("ore-augmented-stream"),
            "OreAugmentedStream",
            None,
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
            &[],
        );

        assert!(rendered.contains("import { extendStack } from '@usearete/sdk';"));
        assert!(rendered
            .contains("import { SQUADS_V4_STREAM_STACK_CORE } from './squads-v4-stream-core.js';"));
        assert!(rendered.contains("import stackExtensions from './squads-v4-extensions.js';"));
        assert!(rendered.contains("export const SQUADS_V4_STREAM_STACK = extendStack("));
        assert!(rendered.contains("export default SQUADS_V4_STREAM_STACK;"));
    }

    #[test]
    fn render_typescript_stack_entry_with_program_extensions_wraps_core_programs() {
        let rendered = render_typescript_stack_entry(
            &layout("squads-v4-stream"),
            "SquadsV4Stream",
            Some("squads-v4-extensions.ts"),
            &[ProgramExtensionBinding {
                export_name: "squadsProgramExtensions".to_string(),
                program_key: "squadsMultisigProgram".to_string(),
            }],
        );

        assert!(rendered.contains("import { extendPrograms, extendStack } from '@usearete/sdk';"));
        assert!(rendered.contains(
            "import stackExtensions, { squadsProgramExtensions } from './squads-v4-extensions.js';"
        ));
        assert!(rendered.contains("const CORE = {"));
        assert!(
            rendered.contains("programs: extendPrograms(SQUADS_V4_STREAM_STACK_CORE.programs, {")
        );
        assert!(rendered.contains("squadsMultisigProgram: squadsProgramExtensions,"));
        assert!(rendered.contains("export const SQUADS_V4_STREAM_STACK = extendStack("));
        assert!(rendered.contains("  CORE,"));
    }

    #[test]
    fn render_typescript_program_entry_without_extensions_aliases_core() {
        let rendered = render_typescript_program_entry(&layout("spl-token"), "token", None);

        assert!(rendered
            .contains("import { TOKEN as SPL_TOKEN_PROGRAM_CORE } from './spl-token-core.js';"));
        assert!(rendered
            .contains("export { TOKEN as SPL_TOKEN_PROGRAM_CORE } from './spl-token-core.js';"));
        assert!(rendered.contains("export const SPL_TOKEN_PROGRAM = SPL_TOKEN_PROGRAM_CORE;"));
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
        assert!(rendered.contains(
            "import { SYSTEM_PROGRAM as SYSTEM_PROGRAM_CORE } from './system-program-core.js';"
        ));
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
    fn validate_extensions_input_pin_accepts_matching_stack_ast_pin() {
        let artifact = test_artifact(ExtensionsInputKind::StackAst, "hash-1");
        let input_pin = ResolvedExtensionsInputPin {
            kind: ExtensionsInputKind::StackAst,
            hash: "hash-1".to_string(),
        };

        assert!(validate_extensions_input_pin(&artifact, &input_pin).is_empty());
    }

    #[test]
    fn validate_extensions_input_pin_reports_kind_mismatch() {
        let artifact = test_artifact(ExtensionsInputKind::ProgramIdl, "hash-1");
        let input_pin = ResolvedExtensionsInputPin {
            kind: ExtensionsInputKind::StackAst,
            hash: "hash-1".to_string(),
        };

        assert_eq!(
            validate_extensions_input_pin(&artifact, &input_pin),
            vec![
                "extensions input kind mismatch: manifest=program-idl, generated=stack-ast"
                    .to_string()
            ]
        );
    }

    #[test]
    fn validate_extensions_input_pin_reports_hash_mismatch() {
        let artifact = test_artifact(ExtensionsInputKind::StackAst, "hash-1");
        let input_pin = ResolvedExtensionsInputPin {
            kind: ExtensionsInputKind::StackAst,
            hash: "hash-2".to_string(),
        };

        assert_eq!(
            validate_extensions_input_pin(&artifact, &input_pin),
            vec!["extensions input hash mismatch: manifest=hash-1, generated=hash-2".to_string()]
        );
    }

    #[test]
    fn stage_extensions_artifact_rejects_input_pin_mismatch() {
        let artifact = test_artifact(ExtensionsInputKind::ProgramIdl, "hash-1");
        let input_pin = ResolvedExtensionsInputPin {
            kind: ExtensionsInputKind::StackAst,
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
        assert_eq!(artifact.program_extension_bindings.len(), 1);
        assert_eq!(artifact.program_extension_bindings[0].program_key, "bar");
    }

    #[test]
    fn program_idl_input_pin_hashes_canonical_json_not_raw_bytes() {
        let temp_path =
            std::env::temp_dir().join(format!("a4-program-idl-pin-{}.json", std::process::id()));
        let raw_json = "{\n  \"name\": \"Demo\",\n  \"metadata\": { \"address\": \"Demo111111111111111111111111111111111111111\" },\n  \"version\": \"0.0.1\",\n  \"instructions\": []\n}";
        fs::write(&temp_path, raw_json).expect("temp idl should be written");

        let value: serde_json::Value = serde_json::from_str(raw_json).expect("json should parse");
        let expected = compute_idl_content_hash_from_value(&value).expect("hash should compute");
        let pin = program_idl_input_pin(&temp_path).expect("pin should compute");

        let _ = fs::remove_file(&temp_path);

        assert_eq!(pin.kind, ExtensionsInputKind::ProgramIdl);
        assert_eq!(pin.hash, expected);
    }
}
