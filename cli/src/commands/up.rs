use anyhow::Result;
use colored::Colorize;
use indicatif::{ProgressBar, ProgressStyle};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::path::Path;
use std::time::Duration;

use crate::api_client::{
    ApiClient, BindStackCompositionRequest, BindStackCompositionResponse, BuildStatus,
    BuildStatusResponse, CreateAliasedLiveSpecArtifact, CreateArtifactBuildRequest,
    CreateBuildRequest, DeploymentPhase, DeploymentResponse, DeploymentStatus,
    DEFAULT_DOMAIN_SUFFIX,
};
use crate::commands::public_artifacts::{load_local_artifact_stack, LocalArtifactStack};
use crate::config::{resolve_stacks_to_push, AreteConfig};
use crate::telemetry;
use crate::ui;

fn generate_short_uuid() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let digest = Sha256::digest(format!("{timestamp}:{}", std::process::id()).as_bytes());
    digest[..6]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HostedLiveTarget {
    alias: String,
    live_spec_hash: String,
    spec_name: String,
    entity_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HostedDeploymentPlan {
    stack_name: String,
    stack_manifest_hash: String,
    branch: Option<String>,
    targets: Vec<HostedLiveTarget>,
}

impl HostedDeploymentPlan {
    fn from_stack(stack: &LocalArtifactStack, branch: Option<&str>) -> Result<Self> {
        let stack_name = stack.stack_manifest.payload.name.clone();
        if stack.live_specs.is_empty() {
            anyhow::bail!(
                "Hosted deployment requires at least one LiveSpec; StackManifest '{}' is program-only. Install its programs through Program Read instead of `a4 up`.",
                stack_name
            );
        }
        let targets = stack
            .live_specs
            .iter()
            .enumerate()
            .map(|(index, (alias, live))| HostedLiveTarget {
                alias: alias.clone(),
                live_spec_hash: live.artifact_hash.to_string(),
                spec_name: child_spec_name(&stack_name, alias, index),
                entity_name: live
                    .payload
                    .entities
                    .first()
                    .map(|entity| entity.state_name.clone())
                    .unwrap_or_else(|| alias.clone()),
            })
            .collect::<Vec<_>>();
        if targets
            .iter()
            .map(|target| target.spec_name.as_str())
            .collect::<BTreeSet<_>>()
            .len()
            != targets.len()
        {
            anyhow::bail!("Hosted child spec naming produced a collision");
        }
        Ok(Self {
            stack_name,
            stack_manifest_hash: stack.stack_manifest.artifact_hash.to_string(),
            branch: branch.map(str::to_string),
            targets,
        })
    }
}

fn child_spec_name(stack_name: &str, alias: &str, position: usize) -> String {
    if position == 0 {
        return stack_name.to_string();
    }
    let mut slug = alias
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    while slug.contains("--") {
        slug = slug.replace("--", "-");
    }
    let slug = slug.trim_matches('-');
    let slug = if slug.is_empty() { "live" } else { slug };
    let slug = slug.chars().take(24).collect::<String>();
    let digest = Sha256::digest(alias.as_bytes());
    let alias_hash = digest[..6]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let suffix = format!("--live-{}-{slug}-{alias_hash}", position + 1);
    let prefix = stack_name
        .chars()
        .take(63usize.saturating_sub(suffix.chars().count()))
        .collect::<String>();
    let mut name = format!("{prefix}{suffix}");
    if name == stack_name {
        let first_len = name.chars().next().map(char::len_utf8).unwrap_or(0);
        let replacement = if stack_name.starts_with('a') {
            "b"
        } else {
            "a"
        };
        name.replace_range(..first_len, replacement);
    }
    name
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CompletedHostedTarget {
    alias: String,
    build_id: i32,
    deployment_id: i32,
}

#[derive(Debug, Clone)]
struct HostedOrchestration {
    plan: HostedDeploymentPlan,
    completed: Vec<CompletedHostedTarget>,
    failed: bool,
}

impl HostedOrchestration {
    fn new(plan: HostedDeploymentPlan) -> Self {
        Self {
            plan,
            completed: Vec::new(),
            failed: false,
        }
    }

    fn next_target(&self) -> Option<&HostedLiveTarget> {
        (!self.failed)
            .then(|| self.plan.targets.get(self.completed.len()))
            .flatten()
    }

    fn record_success(&mut self, alias: &str, build_id: i32, deployment_id: i32) -> Result<()> {
        let expected = self
            .next_target()
            .ok_or_else(|| anyhow::anyhow!("No hosted target is awaiting completion"))?;
        if expected.alias != alias {
            anyhow::bail!(
                "Hosted target completed out of order: expected '{}', received '{}'",
                expected.alias,
                alias
            );
        }
        if self
            .completed
            .iter()
            .any(|completed| completed.deployment_id == deployment_id)
        {
            anyhow::bail!("Hosted aliases must use independent deployment IDs");
        }
        self.completed.push(CompletedHostedTarget {
            alias: alias.to_string(),
            build_id,
            deployment_id,
        });
        Ok(())
    }

    fn record_failure(&mut self, alias: &str) -> Result<()> {
        let expected = self
            .next_target()
            .ok_or_else(|| anyhow::anyhow!("No hosted target is awaiting completion"))?;
        if expected.alias != alias {
            anyhow::bail!(
                "Hosted target failed out of order: expected '{}', received '{}'",
                expected.alias,
                alias
            );
        }
        self.failed = true;
        Ok(())
    }

    fn composition_request(&self) -> Option<BindStackCompositionRequest> {
        if self.failed || self.completed.len() != self.plan.targets.len() {
            return None;
        }
        Some(BindStackCompositionRequest {
            stack_manifest_hash: self.plan.stack_manifest_hash.clone(),
            deployments: self
                .completed
                .iter()
                .map(|completed| (completed.alias.clone(), completed.deployment_id))
                .collect(),
            branch: self.plan.branch.clone(),
        })
    }
}

fn artifact_build_request(
    stack: &LocalArtifactStack,
    target: &HostedLiveTarget,
    spec_id: i32,
    branch: Option<&str>,
) -> CreateArtifactBuildRequest {
    CreateArtifactBuildRequest {
        spec_id,
        program_specs: stack.program_specs.clone(),
        live_specs: stack
            .live_specs
            .iter()
            .map(|(alias, artifact)| CreateAliasedLiveSpecArtifact {
                alias: alias.clone(),
                artifact: artifact.clone(),
            })
            .collect(),
        stack_manifest: stack.stack_manifest.clone(),
        target_live_alias: target.alias.clone(),
        branch: branch.map(str::to_string),
    }
}

fn validate_composition_response(
    orchestration: &HostedOrchestration,
    response: &BindStackCompositionResponse,
) -> Result<()> {
    let request = orchestration.composition_request().ok_or_else(|| {
        anyhow::anyhow!("Composition cannot be bound before every target succeeds")
    })?;
    if response.stack_manifest_hash != request.stack_manifest_hash {
        anyhow::bail!("Composition response StackManifest hash mismatch");
    }
    if response.branch != request.branch {
        anyhow::bail!("Composition response branch mismatch");
    }
    if response.live_specs.len() != orchestration.plan.targets.len() {
        anyhow::bail!("Composition response does not cover every manifest alias");
    }
    for (target, binding) in orchestration.plan.targets.iter().zip(&response.live_specs) {
        if binding.alias != target.alias {
            anyhow::bail!("Composition response alias order mismatch");
        }
        if binding.live_spec_hash != target.live_spec_hash {
            anyhow::bail!(
                "Composition response LiveSpec hash mismatch for alias '{}'",
                target.alias
            );
        }
        if request.deployments.get(&target.alias) != Some(&binding.deployment_id) {
            anyhow::bail!(
                "Composition response deployment mismatch for alias '{}'",
                target.alias
            );
        }
    }
    Ok(())
}

pub fn up(
    config_path: &str,
    stack_name: Option<&str>,
    branch: Option<String>,
    preview: bool,
    dry_run: bool,
) -> Result<()> {
    let start = std::time::Instant::now();
    let config = AreteConfig::load_optional(config_path)?;

    let branch = if preview {
        Some(format!("preview-{}", generate_short_uuid()))
    } else {
        branch
    };

    if let Some(target) = stack_name.filter(|target| target.ends_with(".stack-manifest.json")) {
        let stack = load_local_artifact_stack(Path::new(target))?;
        if dry_run {
            return show_artifact_dry_run(&stack, branch.as_deref());
        }
        let client = ApiClient::new()?;
        deploy_artifact_stack(&client, stack, branch.as_deref())?;
        telemetry::record_stack_deployed(target, start.elapsed());
        return Ok(());
    }

    ui::print_warning(
        "Deploying a composite .stack.json is deprecated and supported only through August 31, 2026. Deploy the generated .stack-manifest.json instead.",
    );

    let stacks = resolve_stacks_to_push(config.as_ref(), stack_name)?;

    if stacks.is_empty() {
        anyhow::bail!("No stacks found to deploy");
    }

    if dry_run {
        return show_dry_run(&stacks, branch.as_deref());
    }

    let client = ApiClient::new()?;

    if stacks.len() > 1 && stack_name.is_none() {
        println!(
            "{} Found {} stacks. Deploying all...\n",
            ui::symbols::ARROW.blue().bold(),
            stacks.len()
        );
    }

    for ast in &stacks {
        deploy_single_stack(&client, ast, branch.as_deref())?;
        println!();
    }

    telemetry::record_stack_deployed(stack_name.unwrap_or(""), start.elapsed());

    Ok(())
}

fn show_artifact_dry_run(stack: &LocalArtifactStack, branch: Option<&str>) -> Result<()> {
    let plan = HostedDeploymentPlan::from_stack(stack, branch)?;
    ui::print_section("Dry Run - No changes will be made");
    println!();
    println!(
        "  {} {}",
        ui::symbols::BULLET.dimmed(),
        stack.stack_manifest.payload.name.green().bold()
    );
    println!("    StackManifest: {}", stack.manifest_path.display());
    println!("    StackManifest hash: {}", plan.stack_manifest_hash);
    if stack.manifest_hash != plan.stack_manifest_hash {
        println!("    Source compatibility hash: {}", stack.manifest_hash);
    }
    println!("    ProgramSpecs: {}", stack.program_specs.len());
    println!("    Child targets: {}", plan.targets.len());
    for (index, target) in plan.targets.iter().enumerate() {
        println!(
            "      {}. {} -> spec '{}' (LiveSpec {})",
            index + 1,
            target.alias,
            target.spec_name,
            target.live_spec_hash
        );
        println!("         targetLiveAlias: {}", target.alias);
    }
    println!(
        "    Final bind: POST /api/deployments/compositions ({})",
        plan.targets
            .iter()
            .map(|target| format!("{}=<deployment-id>", target.alias))
            .collect::<Vec<_>>()
            .join(", ")
    );
    if let Some(branch_name) = branch {
        println!("    Branch: {}", branch_name.cyan());
    }
    println!();
    println!("{}", "Run without --dry-run to deploy.".dimmed());
    Ok(())
}

fn deploy_artifact_stack(
    client: &ApiClient,
    stack: LocalArtifactStack,
    branch: Option<&str>,
) -> Result<()> {
    let plan = HostedDeploymentPlan::from_stack(&stack, branch)?;
    let mut orchestration = HostedOrchestration::new(plan.clone());
    ui::print_divider();
    println!(
        "{} Deploying {} from StackManifest",
        ui::symbols::ARROW.blue().bold(),
        plan.stack_name.bold()
    );
    ui::print_divider();
    println!("  StackManifest: {}", plan.stack_manifest_hash);
    println!("  ProgramSpecs: {}", stack.program_specs.len());
    if let Some(branch_name) = branch {
        println!("  Branch: {}", branch_name.cyan());
    }

    for (index, target) in plan.targets.iter().enumerate() {
        ui::print_numbered_step(
            (index + 1) as u32,
            &format!("Deploying live alias '{}'...", target.alias),
        );
        let spec_id = if let Some(spec) = client.get_spec_by_name(&target.spec_name)? {
            println!(
                "  {} Reusing exact spec '{}' (id={})",
                ui::symbols::SUCCESS.green(),
                target.spec_name,
                spec.id
            );
            spec.id
        } else {
            let spinner = ui::create_spinner(&format!("Creating spec '{}'...", target.spec_name));
            let request = crate::api_client::CreateSpecRequest {
                name: target.spec_name.clone(),
                entity_name: target.entity_name.clone(),
                crate_name: String::new(),
                module_path: String::new(),
                description: None,
                package_name: None,
                output_path: None,
            };
            match client.create_spec(request) {
                Ok(spec) => {
                    spinner.finish_with_message(format!(
                        "{} Spec '{}' created",
                        ui::symbols::SUCCESS.green(),
                        target.spec_name
                    ));
                    spec.id
                }
                Err(create_error) => {
                    let Some(spec) = client.get_spec_by_name(&target.spec_name)? else {
                        spinner.finish_and_clear();
                        return Err(create_error);
                    };
                    spinner.finish_with_message(format!(
                        "{} Reusing concurrently created spec '{}'",
                        ui::symbols::SUCCESS.green(),
                        target.spec_name
                    ));
                    spec.id
                }
            }
        };

        let response = client
            .create_artifact_build(artifact_build_request(&stack, target, spec_id, branch))?;
        println!("  Alias: {}", target.alias);
        println!("  LiveSpec: {}", target.live_spec_hash);
        println!("  Build ID: {}", response.build_id.to_string().bold());
        println!();
        let build = match watch_build_progress(client, response.build_id) {
            Ok(build) => build,
            Err(error) => {
                orchestration.record_failure(&target.alias)?;
                return Err(anyhow::anyhow!(
                    "Hosted alias '{}' did not deploy: {}",
                    target.alias,
                    error
                ));
            }
        };
        let deployment = wait_for_healthy_current_deployment(
            client,
            spec_id,
            response.build_id,
            branch,
            build.related_deployment_id,
        )?;
        orchestration.record_success(&target.alias, response.build_id, deployment.id)?;
        println!(
            "  {} Alias '{}' is healthy on deployment {}",
            ui::symbols::SUCCESS.green(),
            target.alias,
            deployment.id
        );
        println!();
    }

    ui::print_numbered_step(
        (plan.targets.len() + 1) as u32,
        "Binding stack composition...",
    );
    let request = orchestration.composition_request().ok_or_else(|| {
        anyhow::anyhow!("Not every hosted target completed; composition not bound")
    })?;
    let response = client.bind_stack_composition(request)?;
    validate_composition_response(&orchestration, &response)?;
    ui::print_success("Stack composition bound successfully!");
    println!("  Composition ID: {}", response.composition_id);
    for binding in response.live_specs {
        println!(
            "  {} {} -> deployment {}",
            ui::symbols::SUCCESS.green(),
            binding.alias,
            binding.deployment_id
        );
        println!("    WebSocket: {}", binding.websocket_endpoint.cyan());
        println!("    Query: {}", binding.query_endpoint.cyan());
    }
    Ok(())
}

fn show_dry_run(stacks: &[crate::config::DiscoveredAst], branch: Option<&str>) -> Result<()> {
    ui::print_section("Dry Run - No changes will be made");
    println!();

    println!(
        "{} Would deploy {} stack(s):",
        ui::symbols::ARROW.blue().bold(),
        stacks.len()
    );
    println!();

    let client = ApiClient::new().ok();

    for ast in stacks {
        println!(
            "  {} {}",
            ui::symbols::BULLET.dimmed(),
            ast.stack_name.green().bold()
        );
        println!("    Stack: {}", ast.stack_id);
        println!("    Stack: {}", ast.path.display());
        if !ast.program_ids.is_empty() {
            println!("    Program IDs: {}", ast.program_ids.join(", "));
        }

        let url = get_expected_url(&client, &ast.stack_name, branch);
        println!("    URL: {}", url.cyan());
        println!();
    }

    if let Some(branch_name) = branch {
        println!("  Branch: {}", branch_name.cyan());
    }

    println!();
    println!("{}", "Run without --dry-run to deploy.".dimmed());

    Ok(())
}

fn get_expected_url(client: &Option<ApiClient>, stack_name: &str, branch: Option<&str>) -> String {
    let existing_slug = client
        .as_ref()
        .and_then(|c| c.get_spec_by_name(stack_name).ok())
        .flatten()
        .map(|spec| spec.url_slug);

    let name_lower = stack_name.to_lowercase();

    match (existing_slug, branch) {
        (Some(slug), Some(b)) => {
            format!(
                "wss://{}-{}-{}.{}",
                name_lower, slug, b, DEFAULT_DOMAIN_SUFFIX
            )
        }
        (Some(slug), None) => {
            format!("wss://{}-{}.{}", name_lower, slug, DEFAULT_DOMAIN_SUFFIX)
        }
        (None, Some(b)) => {
            format!(
                "wss://{}-<slug>-{}.{} (slug assigned on first deploy)",
                name_lower, b, DEFAULT_DOMAIN_SUFFIX
            )
        }
        (None, None) => {
            format!(
                "wss://{}-<slug>.{} (slug assigned on first deploy)",
                name_lower, DEFAULT_DOMAIN_SUFFIX
            )
        }
    }
}

fn deploy_single_stack(
    client: &ApiClient,
    ast: &crate::config::DiscoveredAst,
    branch: Option<&str>,
) -> Result<()> {
    ui::print_divider();
    if let Some(branch_name) = branch {
        println!(
            "{} Deploying {} (branch: {})",
            ui::symbols::ARROW.blue().bold(),
            ast.stack_name.bold(),
            branch_name.cyan()
        );
    } else {
        println!(
            "{} Deploying {}",
            ui::symbols::ARROW.blue().bold(),
            ast.stack_name.bold()
        );
    }
    ui::print_divider();

    ui::print_numbered_step(1, "Pushing stack...");

    let remote_spec = client.get_spec_by_name(&ast.stack_name)?;

    let spec_id = if let Some(spec) = remote_spec {
        println!(
            "  {} Stack exists (id={})",
            ui::symbols::SUCCESS.green(),
            spec.id
        );
        spec.id
    } else {
        let spinner = ui::create_spinner("Creating stack...");
        let req = crate::api_client::CreateSpecRequest {
            name: ast.stack_name.clone(),
            entity_name: ast.stack_id.clone(),
            crate_name: String::new(),
            module_path: String::new(),
            description: None,
            package_name: None,
            output_path: None,
        };
        let new_spec = client.create_spec(req)?;
        spinner.finish_with_message(format!("{} Stack created", ui::symbols::SUCCESS.green()));
        new_spec.id
    };

    let spinner = ui::create_spinner("Uploading stack...");
    let ast_payload = ast.load_ast()?;
    let version_response = client.create_spec_version(spec_id, ast_payload)?;

    let hash_short = version_response.version.short_hash();
    if version_response.version_is_new {
        spinner.finish_with_message(format!(
            "{} v{} ({})",
            ui::symbols::SUCCESS.green(),
            version_response.version.version_number,
            hash_short
        ));
    } else {
        spinner.finish_with_message(format!(
            "{} v{} (up to date)",
            ui::symbols::EQUALS.blue(),
            version_response.version.version_number
        ));
    }

    ui::print_numbered_step(2, "Creating build...");

    let req = CreateBuildRequest {
        spec_id: Some(spec_id),
        spec_version_id: Some(version_response.version.id),
        ast_payload: None,
        branch: branch.map(|s| s.to_string()),
    };

    let build_response = client.create_build(req)?;
    println!("  Build ID: {}", build_response.build_id.to_string().bold());
    if let Some(branch_name) = branch {
        println!("  Branch: {}", branch_name.cyan());
    }

    ui::print_numbered_step(3, "Building & deploying...");
    println!();

    watch_build_progress(client, build_response.build_id)?;

    Ok(())
}

fn wait_for_healthy_current_deployment(
    client: &ApiClient,
    spec_id: i32,
    build_id: i32,
    branch: Option<&str>,
    mut deployment_id: Option<i32>,
) -> Result<DeploymentResponse> {
    let spinner = ui::create_spinner("Waiting for healthy current deployment...");
    let start_time = std::time::Instant::now();
    let timeout = Duration::from_secs(ui::DEFAULT_POLL_TIMEOUT_SECS);

    loop {
        if start_time.elapsed() > timeout {
            spinner.finish_and_clear();
            anyhow::bail!(
                "Deployment for build {} did not become healthy/current within {} minutes",
                build_id,
                timeout.as_secs() / 60
            );
        }

        let deployment = if let Some(id) = deployment_id {
            Some(client.get_deployment(id)?)
        } else {
            let candidate = find_deployment(client, spec_id, branch)?;
            if let Some(candidate) = &candidate {
                deployment_id = Some(candidate.id);
            }
            candidate
        };

        if let Some(deployment) = deployment {
            if deployment.spec_id != spec_id || deployment.branch.as_deref() != branch {
                spinner.finish_and_clear();
                anyhow::bail!("Build resolved to an unexpected deployment target");
            }
            if deployment.status == DeploymentStatus::Failed
                || deployment.live_status.phase == DeploymentPhase::Degraded
            {
                spinner.finish_and_clear();
                anyhow::bail!("Deployment {} became unhealthy", deployment.id);
            }
            if deployment.current_build_id == Some(build_id)
                && deployment.status == DeploymentStatus::Active
                && deployment.live_status.phase == DeploymentPhase::Running
            {
                spinner.finish_and_clear();
                return Ok(deployment);
            }
        }

        std::thread::sleep(Duration::from_millis(ui::DEFAULT_POLL_INTERVAL_MS));
    }
}

fn find_deployment(
    client: &ApiClient,
    spec_id: i32,
    branch: Option<&str>,
) -> Result<Option<DeploymentResponse>> {
    const PAGE_SIZE: i64 = 100;
    const MAX_PAGES: i64 = 100;
    for page in 0..MAX_PAGES {
        let deployments = client.list_deployments_page(PAGE_SIZE, page * PAGE_SIZE)?;
        if let Some(deployment) = deployments.iter().find(|deployment| {
            deployment.spec_id == spec_id && deployment.branch.as_deref() == branch
        }) {
            return Ok(Some(deployment.clone()));
        }
        if deployments.len() < PAGE_SIZE as usize {
            return Ok(None);
        }
    }
    anyhow::bail!("Deployment lookup exceeded the bounded pagination limit")
}

fn watch_build_progress(client: &ApiClient, build_id: i32) -> Result<BuildStatusResponse> {
    let mut last_phase: Option<String> = None;
    let progress_bar = ProgressBar::new(100);
    progress_bar.set_style(
        ProgressStyle::default_bar()
            .template("  {spinner:.blue} [{bar:30.green/dim}] {pos}% {msg}")
            .expect("Invalid progress bar template")
            .progress_chars("█░░"),
    );
    progress_bar.enable_steady_tick(Duration::from_millis(80));

    let start_time = std::time::Instant::now();
    let timeout = Duration::from_secs(ui::DEFAULT_POLL_TIMEOUT_SECS);

    loop {
        if start_time.elapsed() > timeout {
            progress_bar.finish_and_clear();
            anyhow::bail!(
                "Build timed out after {} minutes. Check build status with: a4 build status {}",
                timeout.as_secs() / 60,
                build_id
            );
        }

        let response = client.get_build(build_id)?;
        let build = &response.build;

        if last_phase != build.phase {
            if let Some(phase) = &build.phase {
                let phase_display = ui::humanize_phase(phase);
                progress_bar.set_message(phase_display.to_string());
            }
            last_phase = build.phase.clone();
        }

        if let Some(progress) = build.progress {
            progress_bar.set_position(progress as u64);
        }

        if build.status.is_terminal() {
            progress_bar.finish_and_clear();
            println!();

            match build.status {
                BuildStatus::Completed => {
                    ui::print_success("Deployed successfully!");

                    if let Some(ws_url) = &build.websocket_url {
                        println!();
                        println!("  {} {}", "WebSocket:".bold(), ws_url.cyan().bold());
                    }
                    return Ok(response);
                }
                BuildStatus::Failed => {
                    ui::print_error("Build failed!");

                    if let Some(msg) = &build.status_message {
                        println!("  {}", msg);
                    } else if let Some(category) = &build.error_category {
                        println!("  Error category: {}", category);
                    }

                    anyhow::bail!("Deployment failed");
                }
                BuildStatus::Cancelled => {
                    ui::print_warning("Build was cancelled.");
                    anyhow::bail!("Deployment cancelled");
                }
                _ => {}
            }
        }

        std::thread::sleep(Duration::from_millis(ui::DEFAULT_POLL_INTERVAL_MS));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn local_stack(aliases: &[&str]) -> LocalArtifactStack {
        let live = arete_artifacts::LiveSpecArtifactV2::new(arete_artifacts::LiveSpecV2::new(
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ))
        .unwrap();
        let live_specs = aliases
            .iter()
            .map(|alias| ((*alias).to_string(), live.clone()))
            .collect::<Vec<_>>();
        let stack_manifest = arete_artifacts::compose_stack_manifest_v2(
            "HostedComposition",
            &[],
            live_specs
                .iter()
                .map(|(alias, live)| (alias.clone(), live))
                .collect(),
            Vec::new(),
        )
        .unwrap();
        LocalArtifactStack {
            manifest_path: PathBuf::from("HostedComposition.stack-manifest.json"),
            manifest_hash: stack_manifest.artifact_hash.to_string(),
            program_specs: Vec::new(),
            live_specs,
            stack_manifest,
        }
    }

    fn completed_orchestration(stack: &LocalArtifactStack) -> HostedOrchestration {
        let plan = HostedDeploymentPlan::from_stack(stack, Some("preview")).unwrap();
        let mut orchestration = HostedOrchestration::new(plan);
        for index in 0..orchestration.plan.targets.len() {
            let alias = orchestration.plan.targets[index].alias.clone();
            orchestration
                .record_success(&alias, 200 + index as i32, 100 + index as i32)
                .unwrap();
        }
        orchestration
    }

    fn bind_response(orchestration: &HostedOrchestration) -> BindStackCompositionResponse {
        BindStackCompositionResponse {
            composition_id: 77,
            stack_manifest_hash: orchestration.plan.stack_manifest_hash.clone(),
            branch: orchestration.plan.branch.clone(),
            live_specs: orchestration
                .plan
                .targets
                .iter()
                .enumerate()
                .map(
                    |(index, target)| crate::api_client::CompositionLiveBindingResponse {
                        alias: target.alias.clone(),
                        live_spec_hash: target.live_spec_hash.clone(),
                        deployment_id: 100 + index as i32,
                        websocket_endpoint: format!("wss://{}.example.test", target.alias),
                        query_endpoint: format!("https://{}.example.test", target.alias),
                        websocket_auth_policy: "signed_session".into(),
                        query_auth_policy: "signed_session".into(),
                        observed_generation: 3,
                    },
                )
                .collect(),
        }
    }

    #[test]
    fn three_alias_plan_has_stable_independent_targets_and_full_build_requests() {
        let stack = local_stack(&["first", "second-value", "third_value"]);
        let first = HostedDeploymentPlan::from_stack(&stack, None).unwrap();
        let second = HostedDeploymentPlan::from_stack(&stack, None).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.targets[0].spec_name, "HostedComposition");
        assert_ne!(first.targets[1].spec_name, first.targets[2].spec_name);
        assert_eq!(
            first
                .targets
                .iter()
                .map(|target| target.spec_name.as_str())
                .collect::<BTreeSet<_>>()
                .len(),
            3
        );

        for (index, target) in first.targets.iter().enumerate() {
            let request = artifact_build_request(&stack, target, 10 + index as i32, None);
            assert_eq!(request.target_live_alias, target.alias);
            assert_eq!(request.live_specs.len(), 3);
            assert_eq!(
                request
                    .live_specs
                    .iter()
                    .map(|live| live.alias.as_str())
                    .collect::<Vec<_>>(),
                vec!["first", "second-value", "third_value"]
            );
        }
    }

    #[test]
    fn partial_failure_never_produces_a_bind_request() {
        let stack = local_stack(&["first", "second", "third"]);
        let plan = HostedDeploymentPlan::from_stack(&stack, None).unwrap();
        let mut orchestration = HostedOrchestration::new(plan);
        orchestration.record_success("first", 1, 11).unwrap();
        orchestration.record_failure("second").unwrap();
        assert!(orchestration.composition_request().is_none());
        assert!(orchestration.next_target().is_none());
    }

    #[test]
    fn composition_response_rejects_alias_hash_deployment_and_order_mismatches() {
        let stack = local_stack(&["first", "second", "third"]);
        let orchestration = completed_orchestration(&stack);
        let valid = bind_response(&orchestration);
        validate_composition_response(&orchestration, &valid).unwrap();

        let mut alias = valid.clone();
        alias.live_specs[0].alias = "other".into();
        assert!(validate_composition_response(&orchestration, &alias).is_err());

        let mut hash = valid.clone();
        hash.live_specs[1].live_spec_hash = "other-hash".into();
        assert!(validate_composition_response(&orchestration, &hash).is_err());

        let mut deployment = valid.clone();
        deployment.live_specs[2].deployment_id = 999;
        assert!(validate_composition_response(&orchestration, &deployment).is_err());

        let mut order = valid;
        order.live_specs.swap(0, 1);
        assert!(validate_composition_response(&orchestration, &order).is_err());
    }

    #[test]
    fn repeated_live_hashes_still_get_independent_alias_targets() {
        let stack = local_stack(&["primary", "replica", "archive"]);
        let plan = HostedDeploymentPlan::from_stack(&stack, None).unwrap();
        assert!(plan
            .targets
            .iter()
            .all(|target| target.live_spec_hash == plan.targets[0].live_spec_hash));
        assert_eq!(
            plan.targets
                .iter()
                .map(|target| target.spec_name.as_str())
                .collect::<BTreeSet<_>>()
                .len(),
            3
        );
        let orchestration = completed_orchestration(&stack);
        assert_eq!(
            orchestration
                .composition_request()
                .unwrap()
                .deployments
                .len(),
            3
        );
    }

    #[test]
    fn single_live_keeps_manifest_name_anchor_and_compatibility_shape() {
        let stack = local_stack(&[arete_artifacts::DEFAULT_LIVE_ALIAS]);
        let plan = HostedDeploymentPlan::from_stack(&stack, None).unwrap();
        assert_eq!(plan.targets.len(), 1);
        assert_eq!(plan.targets[0].spec_name, "HostedComposition");
        let request = artifact_build_request(&stack, &plan.targets[0], 9, None);
        assert_eq!(request.live_specs.len(), 1);
        assert_eq!(
            request.target_live_alias,
            arete_artifacts::DEFAULT_LIVE_ALIAS
        );
    }

    #[test]
    fn program_only_hosted_plan_is_rejected() {
        let mut stack = local_stack(&["live"]);
        stack.live_specs.clear();
        let error = HostedDeploymentPlan::from_stack(&stack, None).unwrap_err();
        assert!(error.to_string().contains("program-only"));
        assert!(error.to_string().contains("Program Read"));
    }
}
