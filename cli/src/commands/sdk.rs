use anyhow::{Context, Result};
use colored::Colorize;
use std::fs;
use std::path::{Path, PathBuf};

use crate::api_client::{ApiClient, RegistryAstResponse};
use crate::config::{discover_ast_files, find_ast_file, to_kebab_case, AreteConfig, DiscoveredAst};
use crate::telemetry;

struct RemoteStackAst {
    name: String,
    stack: String,
    websocket_url: String,
    ast_payload: serde_json::Value,
    sdk_name: String,
}

enum ResolvedStackSource {
    Local(DiscoveredAst),
    Remote(RemoteStackAst),
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

    fn load_stack_spec(&self) -> Result<arete_interpreter::ast::SerializableStackSpec> {
        match self {
            Self::Local(ast) => load_stack_spec_from_file(ast),
            Self::Remote(stack) => load_stack_spec_from_value(
                &stack.ast_payload,
                &format!("hosted stack '{}'", stack.stack),
            ),
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
        "a4 sdk create typescript <stack-name>".cyan()
    );

    Ok(())
}

pub fn create_typescript(
    config_path: &str,
    stack_name: &str,
    output_override: Option<String>,
    package_name_override: Option<String>,
    url_override: Option<String>,
) -> Result<()> {
    println!(
        "{} Looking for stack '{}'...",
        "→".blue().bold(),
        stack_name
    );

    let config = AreteConfig::load_optional(config_path)?;
    let client = ApiClient::new()?;

    // Get the config file's directory for resolving relative paths
    let config_dir = Path::new(config_path)
        .parent()
        .unwrap_or(Path::new("."))
        .to_path_buf();

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

            let url = url_override.or_else(|| match &source {
                ResolvedStackSource::Local(_) => stack_config.url.clone(),
                ResolvedStackSource::Remote(_) => source.default_url(),
            });

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

    println!("\n{} Generating TypeScript SDK...", "→".blue().bold());

    generate_typescript_sdk_from_source(&source, &output_path, &package_name, stack_url)?;

    println!(
        "{} Successfully generated TypeScript SDK!",
        "✓".green().bold()
    );
    println!("  File: {}", output_path.display().to_string().bold());

    telemetry::record_sdk_generated("typescript");

    Ok(())
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
        .unwrap_or_else(|| PathBuf::from(format!("./generated/{}-stack.ts", source.sdk_name())));

    let pkg = package_name_override.unwrap_or_else(|| "@usearete/react".to_string());

    Ok((source, output, pkg))
}

fn generate_typescript_sdk_from_source(
    source: &ResolvedStackSource,
    output_path: &Path,
    package_name: &str,
    url: Option<String>,
) -> Result<()> {
    let stack_spec = source.load_stack_spec()?;

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

    let config = arete_interpreter::typescript::TypeScriptStackConfig {
        package_name: package_name.to_string(),
        generate_helpers: true,
        export_const_name: "STACK".to_string(),
        url,
    };

    let output = arete_interpreter::typescript::compile_stack_spec(stack_spec, Some(config))
        .map_err(|e| anyhow::anyhow!("Failed to compile TypeScript: {}", e))?;

    arete_interpreter::typescript::write_stack_typescript_to_file(&output, output_path)
        .with_context(|| format!("Failed to write TypeScript to {}", output_path.display()))?;

    Ok(())
}

fn load_stack_spec_from_file(
    ast: &DiscoveredAst,
) -> Result<arete_interpreter::ast::SerializableStackSpec> {
    let ast_json = fs::read_to_string(&ast.path)
        .with_context(|| format!("Failed to read stack file: {}", ast.path.display()))?;

    load_stack_spec_from_json(&ast_json, &ast.path.display().to_string())
}

fn load_stack_spec_from_value(
    ast: &serde_json::Value,
    source_name: &str,
) -> Result<arete_interpreter::ast::SerializableStackSpec> {
    let ast_json = serde_json::to_string(ast)
        .with_context(|| format!("Failed to serialize stack AST from {}", source_name))?;

    load_stack_spec_from_json(&ast_json, source_name)
}

fn load_stack_spec_from_json(
    ast_json: &str,
    source_name: &str,
) -> Result<arete_interpreter::ast::SerializableStackSpec> {
    // Use versioned loader for automatic version detection and migration
    let stack_spec = arete_interpreter::versioned::load_stack_spec(ast_json)
        .with_context(|| format!("Failed to load stack AST from {}", source_name))?;

    if stack_spec.entities.is_empty() {
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

    let stack_url = url_override.or_else(|| match &source {
        ResolvedStackSource::Local(_) => stack_config.and_then(|s| s.url.clone()),
        ResolvedStackSource::Remote(_) => source.default_url(),
    });

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

    let stack_spec = source.load_stack_spec()?;

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

    let remote = client.get_registry_ast_by_stack(stack).with_context(|| {
        format!(
            "Stack '{}' was not found locally and no accessible hosted stack with that identifier was found.",
            stack
        )
    })?;

    Ok(ResolvedStackSource::Remote(remote_stack_ast(remote)))
}

fn remote_stack_ast(remote: RegistryAstResponse) -> RemoteStackAst {
    RemoteStackAst {
        sdk_name: to_kebab_case(&remote.name),
        name: remote.name,
        stack: remote.stack,
        websocket_url: remote.websocket_url,
        ast_payload: remote.ast_payload,
    }
}
