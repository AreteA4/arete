use anyhow::{bail, Result};
use colored::Colorize;
use serde::Serialize;
use std::collections::HashMap;
use std::thread;
use std::time::Duration;

use crate::api_client::{
    ApiClient, Build, BuildStatus, CreateBuildRequest, CreateSpecRequest, DeploymentResponse,
    DeploymentStatus, Spec as ApiSpec, DEFAULT_DOMAIN_SUFFIX,
};
use crate::config::{resolve_stacks_to_push, DiscoveredAst, AreteConfig};
use crate::telemetry;

pub fn push(config_path: &str, stack_name: Option<&str>) -> Result<()> {
    let config = AreteConfig::load_optional(config_path)?;

    if config.is_none() && stack_name.is_none() {
        println!(
            "{} No arete.toml found, auto-discovering stack files...",
            "→".blue().bold()
        );
    } else if config.is_some() {
        println!("{} Loading configuration...", "→".blue().bold());
    }

    let stacks_to_push = resolve_stacks_to_push(config.as_ref(), stack_name)?;

    if stacks_to_push.is_empty() {
        println!("{}", "No stacks found to push.".yellow());
        println!("\n{}", "To push stacks, either:".dimmed());
        println!("  1. Build your stack crate to generate .arete/*.stack.json files");
        println!("  2. Create a arete.toml with your stack configuration");
        return Ok(());
    }

    let client = ApiClient::new()?;

    println!("{} Fetching remote stacks...", "→".blue().bold());
    let remote_specs = client.list_specs()?;
    let remote_map: HashMap<String, ApiSpec> = remote_specs
        .into_iter()
        .map(|s| (s.name.clone(), s))
        .collect();

    println!(
        "{} Pushing {} stack(s)...\n",
        "→".blue().bold(),
        stacks_to_push.len()
    );

    let mut created = 0;
    let mut updated = 0;
    let mut unchanged = 0;
    let mut errors = 0;

    for ast in stacks_to_push {
        let remote_spec = match remote_map.get(&ast.stack_name) {
            Some(existing) => {
                if stack_needs_update(&ast, existing) {
                    print!("  {} Updating {}... ", "↑".yellow(), ast.stack_name);
                    match update_remote_stack(&client, existing.id, &ast) {
                        Ok(updated_spec) => {
                            println!("{}", "metadata updated".dimmed());
                            updated_spec
                        }
                        Err(e) => {
                            println!("{} {}", "✗".red(), e);
                            errors += 1;
                            continue;
                        }
                    }
                } else {
                    existing.clone()
                }
            }
            None => {
                print!("  {} Creating {}... ", "+".green(), ast.stack_name);
                match create_remote_stack(&client, &ast) {
                    Ok(new_spec) => {
                        println!("{}", "✓".green());
                        println!(
                            "    {} {}",
                            "URL:".dimmed(),
                            new_spec.websocket_url(DEFAULT_DOMAIN_SUFFIX).cyan()
                        );
                        created += 1;
                        new_spec
                    }
                    Err(e) => {
                        println!("{} {}", "✗".red(), e);
                        errors += 1;
                        continue;
                    }
                }
            }
        };

        print!("  {} {} ", "↑".blue(), ast.stack_name);
        match load_and_upload_ast(&client, remote_spec.id, &ast) {
            Ok(response) => {
                let hash_short = &response.version.content_hash[..12];
                if !response.version_is_new {
                    println!(
                        "{} (v{} up to date)",
                        "=".blue(),
                        response.version.version_number
                    );
                    unchanged += 1;
                } else if !response.content_is_new {
                    println!(
                        "{} v{} (reused {})",
                        "✓".green(),
                        response.version.version_number,
                        hash_short
                    );
                    updated += 1;
                } else {
                    println!(
                        "{} v{} ({})",
                        "✓".green(),
                        response.version.version_number,
                        hash_short
                    );
                    updated += 1;
                }
            }
            Err(e) => {
                println!("{} {}", "✗".red(), e);
                errors += 1;
            }
        }
    }

    println!();
    if errors > 0 {
        println!("{} Push completed with errors", "!".yellow().bold());
    } else {
        println!("{} Push complete!", "✓".green().bold());
    }

    if created > 0 {
        println!("  New stacks: {}", created);
    }
    if updated > 0 {
        println!("  Updated: {}", updated);
    }
    if unchanged > 0 {
        println!("  Unchanged: {}", unchanged);
    }
    if errors > 0 {
        println!("  {} Errors: {}", "✗".red(), errors);
        bail!("Push completed with {} error(s)", errors);
    }

    Ok(())
}

pub fn list(json: bool) -> Result<()> {
    let client = ApiClient::new()?;

    if !json {
        println!("{} Fetching stacks...", "→".blue().bold());
    }

    let specs = client.list_specs()?;
    let deployments = client.list_deployments(100)?;

    let deployment_map: HashMap<i32, _> = deployments.into_iter().map(|d| (d.spec_id, d)).collect();

    if json {
        #[derive(Serialize)]
        struct StackListItem {
            name: String,
            entity_name: String,
            websocket_url: String,
            status: Option<String>,
            current_version: Option<i32>,
        }

        let items: Vec<StackListItem> = specs
            .iter()
            .map(|spec| {
                let deployment = deployment_map.get(&spec.id);
                StackListItem {
                    name: spec.name.clone(),
                    entity_name: spec.entity_name.clone(),
                    websocket_url: spec.websocket_url(DEFAULT_DOMAIN_SUFFIX),
                    status: deployment.map(|d| d.status.to_string()),
                    current_version: deployment.and_then(|d| d.current_version),
                }
            })
            .collect();

        println!("{}", serde_json::to_string_pretty(&items)?);
        return Ok(());
    }

    if specs.is_empty() {
        println!("{}", "No stacks found.".yellow());
        println!("  Run {} to deploy your first stack.", "a4 up".cyan());
        return Ok(());
    }

    println!();
    println!(
        "{:<24} {:<10} {:<8} {}",
        "STACK".bold(),
        "STATUS".bold(),
        "VERSION".bold(),
        "URL".bold()
    );
    println!("{}", "─".repeat(80).dimmed());

    for spec in &specs {
        let deployment = deployment_map.get(&spec.id);

        let status = match deployment {
            Some(d) => match d.status {
                DeploymentStatus::Active => "active".green().to_string(),
                DeploymentStatus::Updating => "updating".yellow().to_string(),
                DeploymentStatus::Stopped => "stopped".dimmed().to_string(),
                DeploymentStatus::Failed => "failed".red().to_string(),
            },
            None => "—".dimmed().to_string(),
        };

        let version = deployment
            .and_then(|d| d.current_version)
            .map(|v| format!("v{}", v))
            .unwrap_or_else(|| "—".dimmed().to_string());

        let url = spec.websocket_url(DEFAULT_DOMAIN_SUFFIX);

        println!(
            "{:<24} {:<10} {:<8} {}",
            spec.name.green(),
            status,
            version,
            url.cyan()
        );
    }

    println!();
    println!("Total: {} stack(s)", specs.len());
    println!("\nTip: Run {} for details", "a4 stack show <name>".cyan());

    Ok(())
}

pub fn show(stack_name: &str, version: Option<i32>, json: bool) -> Result<()> {
    let client = ApiClient::new()?;

    if !json {
        println!("{} Looking up stack '{}'...", "→".blue().bold(), stack_name);
    }

    let spec = client
        .get_spec_by_name(stack_name)?
        .ok_or_else(|| anyhow::anyhow!("Stack '{}' not found", stack_name))?;

    let spec_with_version = client.get_spec_with_latest_version(spec.id)?;
    let deployments = client.list_deployments(100)?;
    let deployment = deployments.iter().find(|d| d.spec_id == spec.id);
    let builds = client.list_builds_filtered(Some(5), None, Some(spec.id))?;

    if json {
        #[derive(Serialize)]
        struct StackShowResponse {
            name: String,
            entity_name: String,
            websocket_url: String,
            description: Option<String>,
            deployment_status: Option<String>,
            current_version: Option<i32>,
            latest_version: Option<i32>,
            recent_builds: Vec<BuildSummary>,
        }

        #[derive(Serialize)]
        struct BuildSummary {
            id: i32,
            status: String,
            version: Option<i32>,
            created_at: String,
        }

        let response = StackShowResponse {
            name: spec.name.clone(),
            entity_name: spec.entity_name.clone(),
            websocket_url: spec.websocket_url(DEFAULT_DOMAIN_SUFFIX),
            description: spec.description.clone(),
            deployment_status: deployment.map(|d| d.status.to_string()),
            current_version: deployment.and_then(|d| d.current_version),
            latest_version: spec_with_version
                .latest_version
                .as_ref()
                .map(|v| v.version_number),
            recent_builds: builds
                .iter()
                .map(|b| BuildSummary {
                    id: b.id,
                    status: b.status.to_string(),
                    version: b.spec_version_id,
                    created_at: b.created_at.clone(),
                })
                .collect(),
        };

        println!("{}", serde_json::to_string_pretty(&response)?);
        return Ok(());
    }

    println!(
        "\n{} Stack: {}\n",
        "→".blue().bold(),
        stack_name.green().bold()
    );

    println!("  Entity: {}", spec.entity_name);
    println!(
        "  URL: {}",
        spec.websocket_url(DEFAULT_DOMAIN_SUFFIX).cyan()
    );
    if let Some(desc) = &spec.description {
        println!("  Description: {}", desc);
    }

    println!();
    println!("  {} Deployment", "•".dimmed());
    if let Some(d) = deployment {
        let status_colored = match d.status {
            DeploymentStatus::Active => "active".green(),
            DeploymentStatus::Updating => "updating".yellow(),
            DeploymentStatus::Stopped => "stopped".dimmed(),
            DeploymentStatus::Failed => "failed".red(),
        };
        println!("    Status: {}", status_colored);
        if let Some(v) = d.current_version {
            println!("    Version: v{}", v);
        }
        if let Some(deployed) = &d.last_deployed_at {
            println!("    Last deployed: {}", deployed);
        }
    } else {
        println!("    {}", "Not deployed".dimmed());
    }

    if let Some(ver) = &spec_with_version.latest_version {
        println!();
        println!("  {} Latest Version", "•".dimmed());
        println!("    v{} ({})", ver.version_number, &ver.content_hash[..12]);
        println!("    State: {}", ver.state_name);
        println!(
            "    Handlers: {}, Sections: {}",
            ver.handler_count, ver.section_count
        );
        if let Some(program_id) = &ver.program_id {
            println!("    Program ID: {}", program_id);
        }
    }

    if !builds.is_empty() {
        println!();
        println!("  {} Recent Builds", "•".dimmed());
        for build in builds.iter().take(5) {
            let status = format_build_status(build.status);
            let version_str = build
                .spec_version_id
                .map(|v| format!("v{}", v))
                .unwrap_or_else(|| "—".to_string());
            println!(
                "    #{:<5} {:<12} {:<6} {}",
                build.id,
                status,
                version_str,
                build.created_at.dimmed()
            );
        }
    }

    if let Some(v) = version {
        println!();
        println!("{} Looking up version {}...", "→".blue().bold(), v);

        let versions = client.list_spec_versions(spec.id)?;
        let ver = versions.iter().find(|ver| ver.version_number == v);

        if let Some(ver) = ver {
            println!();
            println!("  {} Version {}", "•".dimmed(), v);
            println!("    Hash: {}", ver.content_hash);
            println!("    State: {}", ver.state_name);
            println!(
                "    Handlers: {}, Sections: {}",
                ver.handler_count, ver.section_count
            );
            if let Some(program_id) = &ver.program_id {
                println!("    Program ID: {}", program_id);
            }
            println!("    Created: {}", ver.version_created_at);
        } else {
            println!("{}", format!("Version {} not found.", v).yellow());
        }
    }

    Ok(())
}

pub fn versions(stack_name: &str, limit: i64, json: bool) -> Result<()> {
    let client = ApiClient::new()?;

    if !json {
        println!("{} Looking up stack '{}'...", "→".blue().bold(), stack_name);
    }

    let spec = client
        .get_spec_by_name(stack_name)?
        .ok_or_else(|| anyhow::anyhow!("Stack '{}' not found", stack_name))?;

    if !json {
        println!("{} Found stack (id={})", "✓".green().bold(), spec.id);
    }

    let versions = client.list_spec_versions_paginated(spec.id, Some(limit), None)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&versions)?);
        return Ok(());
    }

    if versions.is_empty() {
        println!("\n{}", "No versions found for this stack.".yellow());
        println!(
            "Push a version with: {}",
            format!("a4 stack push {}", stack_name).cyan()
        );
        return Ok(());
    }

    println!(
        "\n{} Version history for '{}':\n",
        "→".blue().bold(),
        stack_name
    );

    for version in &versions {
        let hash_short = &version.content_hash[..12];

        println!(
            "  {} v{}",
            "•".dimmed(),
            version.version_number.to_string().bold()
        );
        println!("    Hash: {}", hash_short);
        println!("    State: {}", version.state_name);
        println!(
            "    Handlers: {}, Sections: {}",
            version.handler_count, version.section_count
        );

        if let Some(program_id) = &version.program_id {
            println!("    Program ID: {}", program_id);
        }

        println!("    Created: {}", version.version_created_at);
        println!();
    }

    println!("Total: {} version(s)", versions.len());

    Ok(())
}

pub fn delete(stack_name: &str, force: bool) -> Result<()> {
    let client = ApiClient::new()?;

    println!("{} Looking up stack '{}'...", "→".blue().bold(), stack_name);

    let spec = client
        .get_spec_by_name(stack_name)?
        .ok_or_else(|| anyhow::anyhow!("Stack '{}' not found", stack_name))?;

    if !force {
        println!();
        println!(
            "{} You are about to delete stack '{}'",
            "!".yellow().bold(),
            stack_name
        );
        println!("  This will delete the stack and ALL its versions.");
        println!("  This action cannot be undone.");
        println!();

        print!("Type the stack name to confirm: ");
        use std::io::{self, Write};
        io::stdout().flush()?;

        let mut confirmation = String::new();
        io::stdin().read_line(&mut confirmation)?;
        let confirmation = confirmation.trim();

        if confirmation != stack_name {
            println!();
            println!("{} Deletion cancelled.", "!".yellow().bold());
            return Ok(());
        }
    }

    println!("{} Deleting stack '{}'...", "→".blue().bold(), stack_name);

    client.delete_spec(spec.id)?;

    println!(
        "{} Stack '{}' deleted successfully.",
        "✓".green().bold(),
        stack_name
    );

    Ok(())
}

pub fn rollback(
    stack_name: &str,
    to_version: Option<i32>,
    build_id: Option<i32>,
    branch: &str,
    _rebuild: bool,
    watch: bool,
) -> Result<()> {
    let client = ApiClient::new()?;

    println!(
        "{} Starting rollback for '{}'...",
        "→".blue().bold(),
        stack_name
    );

    let spec = client.get_spec_by_name(stack_name)?.ok_or_else(|| {
        anyhow::anyhow!(
            "Stack '{}' not found. Use 'a4 stack list' to see available stacks.",
            stack_name
        )
    })?;

    println!("  Found stack (id={})", spec.id);

    let target_version_id = if let Some(bid) = build_id {
        println!("{} Looking up build #{}...", "→".blue().bold(), bid);

        let build_response = client.get_build(bid)?;
        let build = &build_response.build;

        if build.spec_id != Some(spec.id) {
            bail!(
                "Build #{} does not belong to stack '{}'. It belongs to spec_id {:?}",
                bid,
                stack_name,
                build.spec_id
            );
        }

        if build.status != BuildStatus::Completed {
            bail!(
                "Build #{} is not a successful deployment (status: {}). \
                 Cannot rollback to a failed or incomplete build.",
                bid,
                build.status
            );
        }

        build.spec_version_id.ok_or_else(|| {
            anyhow::anyhow!(
                "Build #{} has no spec_version_id. Cannot rollback without a version reference.",
                bid
            )
        })?
    } else if let Some(version) = to_version {
        println!("{} Looking up version {}...", "→".blue().bold(), version);

        let versions = client.list_spec_versions(spec.id)?;
        let ver = versions
            .iter()
            .find(|v| v.version_number == version)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Version {} not found for stack '{}'. Available versions: {:?}",
                    version,
                    stack_name,
                    versions
                        .iter()
                        .map(|v| v.version_number)
                        .collect::<Vec<_>>()
                )
            })?;

        println!(
            "  Found version {} (hash: {})",
            ver.version_number,
            &ver.content_hash[..12]
        );
        ver.id
    } else {
        println!(
            "{} Finding previous successful deployment...",
            "→".blue().bold()
        );

        let builds = client.list_builds_filtered(Some(50), None, Some(spec.id))?;

        let mut successful_builds: Vec<&Build> = builds
            .iter()
            .filter(|b| b.status == BuildStatus::Completed && b.spec_version_id.is_some())
            .collect();

        successful_builds.sort_by(|a, b| b.created_at.cmp(&a.created_at));

        if successful_builds.is_empty() {
            bail!(
                "No successful deployments found for stack '{}'. Nothing to rollback to.",
                stack_name
            );
        }

        if successful_builds.len() < 2 {
            bail!(
                "Only one successful deployment found for stack '{}'. \
                 Need at least two deployments to rollback.",
                stack_name
            );
        }

        let current = successful_builds[0];
        let previous = successful_builds[1];

        println!(
            "  Current deployment: build #{} (version_id: {:?})",
            current.id, current.spec_version_id
        );
        println!(
            "  Rolling back to: build #{} (version_id: {:?})",
            previous.id, previous.spec_version_id
        );

        previous.spec_version_id.unwrap()
    };

    println!();
    println!("{} Creating rollback build...", "→".blue().bold());

    let branch_opt = if branch == "production" {
        None
    } else {
        Some(branch.to_string())
    };

    let req = CreateBuildRequest {
        spec_id: Some(spec.id),
        spec_version_id: Some(target_version_id),
        ast_payload: None,
        branch: branch_opt,
    };

    let response = client.create_build(req)?;

    println!(
        "{} Rollback build created (ID: {})",
        "✓".green().bold(),
        response.build_id
    );
    println!("  Status: {}", format_build_status(response.status));

    if watch {
        println!();
        let result = watch_build(&client, response.build_id);
        telemetry::record_stack_rollback(result.is_ok());
        return result;
    }

    println!();
    println!("Track progress with:");
    println!(
        "  {}",
        format!("a4 build status {} --watch", response.build_id).cyan()
    );

    telemetry::record_stack_rollback(true);

    Ok(())
}

pub fn stop(stack_name: &str, branch: Option<&str>, force: bool) -> Result<()> {
    let client = ApiClient::new()?;

    println!("{} Looking up stack '{}'...", "→".blue().bold(), stack_name);

    let spec = client
        .get_spec_by_name(stack_name)?
        .ok_or_else(|| anyhow::anyhow!("Stack '{}' not found", stack_name))?;

    // Get deployments and find the one for this spec
    let deployments = client.list_deployments(100)?;

    let deployment = find_deployment(&deployments, spec.id, branch).ok_or_else(|| {
        let branch_msg = branch.unwrap_or("production");
        anyhow::anyhow!(
            "No {} deployment found for stack '{}'",
            branch_msg,
            stack_name
        )
    })?;

    // Check if already stopped
    if deployment.status == DeploymentStatus::Stopped {
        println!(
            "{} Deployment for '{}' is already stopped.",
            "!".yellow().bold(),
            stack_name
        );
        return Ok(());
    }

    let branch_display = branch.unwrap_or("production");

    if !force {
        println!();
        println!(
            "{} You are about to stop the deployment for '{}'",
            "!".yellow().bold(),
            stack_name
        );
        println!("  Branch: {}", branch_display);
        println!(
            "  Current status: {}",
            format_deployment_status(deployment.status)
        );
        println!();
        println!("  This will stop the running deployment.");
        println!("  You can restart it later with 'a4 up'.");
        println!();

        print!("Continue? [y/N] ");
        use std::io::{self, Write};
        io::stdout().flush()?;

        let mut confirmation = String::new();
        io::stdin().read_line(&mut confirmation)?;
        let confirmation = confirmation.trim().to_lowercase();

        if confirmation != "y" && confirmation != "yes" {
            println!();
            println!("{} Stop cancelled.", "!".yellow().bold());
            return Ok(());
        }
    }

    println!("{} Stopping deployment...", "→".blue().bold());

    client.stop_deployment(deployment.id)?;

    println!(
        "{} Deployment for '{}' ({}) stopped successfully.",
        "✓".green().bold(),
        stack_name,
        branch_display
    );

    println!();
    println!("To restart, run:");
    println!("  {}", format!("a4 up {}", stack_name).cyan());

    Ok(())
}

fn find_deployment<'a>(
    deployments: &'a [DeploymentResponse],
    spec_id: i32,
    branch: Option<&str>,
) -> Option<&'a DeploymentResponse> {
    deployments.iter().find(|d| {
        d.spec_id == spec_id
            && match branch {
                Some(b) => d.branch.as_deref() == Some(b),
                None => d.branch.is_none(), // production deployment has no branch
            }
    })
}

fn format_deployment_status(status: DeploymentStatus) -> String {
    match status {
        DeploymentStatus::Active => "active".green().to_string(),
        DeploymentStatus::Updating => "updating".yellow().to_string(),
        DeploymentStatus::Stopped => "stopped".dimmed().to_string(),
        DeploymentStatus::Failed => "failed".red().to_string(),
    }
}

fn load_and_upload_ast(
    client: &ApiClient,
    spec_id: i32,
    ast: &DiscoveredAst,
) -> Result<crate::api_client::CreateSpecVersionResponse> {
    let ast_payload = ast.load_ast()?;
    client.create_spec_version(spec_id, ast_payload)
}

fn stack_needs_update(local: &DiscoveredAst, remote: &ApiSpec) -> bool {
    local.stack_id != remote.entity_name
}

fn create_remote_stack(client: &ApiClient, ast: &DiscoveredAst) -> Result<ApiSpec> {
    let req = CreateSpecRequest {
        name: ast.stack_name.clone(),
        entity_name: ast.stack_id.clone(),
        crate_name: String::new(),
        module_path: String::new(),
        description: None,
        package_name: None,
        output_path: None,
    };

    client.create_spec(req)
}

fn update_remote_stack(client: &ApiClient, spec_id: i32, ast: &DiscoveredAst) -> Result<ApiSpec> {
    let req = crate::api_client::UpdateSpecRequest {
        name: Some(ast.stack_name.clone()),
        entity_name: Some(ast.stack_id.clone()),
        crate_name: None,
        module_path: None,
        description: None,
        package_name: None,
        output_path: None,
    };

    client.update_spec(spec_id, req)
}

fn watch_build(client: &ApiClient, build_id: i32) -> Result<()> {
    println!("{} Watching build #{}...\n", "→".blue().bold(), build_id);

    let mut last_status: Option<BuildStatus> = None;
    let mut last_phase: Option<String> = None;

    loop {
        let response = client.get_build(build_id)?;
        let build = &response.build;

        if last_status != Some(build.status) {
            println!(
                "  {} Status: {}",
                chrono_now().dimmed(),
                format_build_status(build.status)
            );
            last_status = Some(build.status);
        }

        if last_phase != build.phase {
            if let Some(phase) = &build.phase {
                println!("  {} Phase: {}", chrono_now().dimmed(), phase);
            }
            last_phase = build.phase.clone();
        }

        if let Some(msg) = &build.status_message {
            if !msg.is_empty() {
                println!("  {} {}", chrono_now().dimmed(), msg.dimmed());
            }
        }

        if build.status.is_terminal() {
            println!();

            match build.status {
                BuildStatus::Completed => {
                    println!("{} Build completed successfully!", "✓".green().bold());

                    if let Some(ws_url) = &build.websocket_url {
                        println!();
                        println!("  WebSocket URL: {}", ws_url.cyan().bold());
                    }
                }
                BuildStatus::Failed => {
                    println!("{} Build failed!", "✗".red().bold());

                    if let Some(msg) = &build.status_message {
                        println!("  {}", msg);
                    }
                }
                BuildStatus::Cancelled => {
                    println!("{} Build was cancelled.", "!".yellow().bold());
                }
                _ => {}
            }

            break;
        }

        thread::sleep(Duration::from_secs(3));
    }

    Ok(())
}

fn format_build_status(status: BuildStatus) -> String {
    match status {
        BuildStatus::Pending => "pending".yellow().to_string(),
        BuildStatus::Uploading => "uploading".yellow().to_string(),
        BuildStatus::Queued => "queued".yellow().to_string(),
        BuildStatus::Building => "building".blue().to_string(),
        BuildStatus::Pushing => "pushing".blue().to_string(),
        BuildStatus::Deploying => "deploying".blue().to_string(),
        BuildStatus::Completed => "completed".green().bold().to_string(),
        BuildStatus::Failed => "failed".red().bold().to_string(),
        BuildStatus::Cancelled => "cancelled".dimmed().to_string(),
    }
}

fn chrono_now() -> String {
    chrono::Local::now().format("%H:%M:%S").to_string()
}
