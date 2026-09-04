use anyhow::{bail, Result};
use colored::Colorize;
use serde::Serialize;
use std::collections::HashMap;
use std::thread;
use std::time::{Duration, Instant};

use crate::api_client::{
    ApiClient, BuildStatus, DeploymentPhase, DeploymentResponse, DeploymentStatus,
    StackDestroyResponse, StackDestroyStatus, DEFAULT_DOMAIN_SUFFIX,
};

const STACK_DESTROY_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const STACK_DESTROY_INITIAL_POLL_INTERVAL: Duration = Duration::from_millis(500);
const STACK_DESTROY_MAX_POLL_INTERVAL: Duration = Duration::from_secs(5);
const STACK_DESTROY_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
pub fn list(json: bool) -> Result<()> {
    let client = ApiClient::new()?;

    if !json {
        println!("{} Fetching stacks...", "→".blue().bold());
    }

    let specs = client.list_specs()?;
    let deployments = client.list_deployments(100)?;

    let deployment_map: HashMap<i32, _> = specs
        .iter()
        .filter_map(|spec| {
            find_deployment(&deployments, spec.id, None).map(|deployment| (spec.id, deployment))
        })
        .collect();

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
    let deployment = find_deployment(&deployments, spec.id, None);
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
        println!("    v{} ({})", ver.version_number, ver.short_hash());
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
            println!("    Portable AST: {}", ver.portable_hash());
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
        let hash_short = version.short_hash();

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
        println!("  This removes its stack-owned runtime resources and mutable stack metadata.");
        println!("  Retired deployment identity and immutable build, composition, and usage history are retained.");
        println!();

        print!("Type the stack name to confirm: ");
        use std::io::{self, Write};
        io::stdout().flush()?;

        let mut confirmation = String::new();
        io::stdin().read_line(&mut confirmation)?;
        let confirmation = confirmation.trim();

        if !delete_confirmation_matches(stack_name, confirmation) {
            println!();
            println!("{} Deletion cancelled.", "!".yellow().bold());
            return Ok(());
        }
    }

    println!(
        "{} Destroying stack-owned runtime resources for '{}'...",
        "→".blue().bold(),
        stack_name
    );

    destroy_stack_with_options(
        &client,
        spec.id,
        stack_name,
        STACK_DESTROY_TIMEOUT,
        STACK_DESTROY_INITIAL_POLL_INTERVAL,
    )?;

    println!(
        "{} Stack '{}' deleted successfully.",
        "✓".green().bold(),
        stack_name
    );

    Ok(())
}

fn delete_confirmation_matches(stack_name: &str, confirmation: &str) -> bool {
    confirmation == stack_name
}

fn shell_quote(value: &str) -> String {
    if !value.is_empty()
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'_' | b'@' | b'%' | b'+' | b'=' | b':' | b',' | b'.' | b'/' | b'-'
                )
        })
    {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "'\"'\"'"))
    }
}

fn retry_stack_delete_command(stack_name: &str) -> String {
    format!("a4 stack delete --force -- {}", shell_quote(stack_name))
}

fn destroy_stack_with_options(
    client: &ApiClient,
    spec_id: i32,
    stack_name: &str,
    timeout: Duration,
    initial_poll_interval: Duration,
) -> Result<StackDestroyResponse> {
    let started = Instant::now();
    let mut poll_interval = initial_poll_interval;
    let mut operation = client.request_stack_destroy(spec_id)?;
    let retry_command = retry_stack_delete_command(stack_name);
    println!("  Durable operation: {}", operation.operation_id.cyan());

    loop {
        match operation.status {
            StackDestroyStatus::Succeeded => return Ok(operation),
            StackDestroyStatus::Failed => {
                let code = operation
                    .error_code
                    .as_deref()
                    .unwrap_or("stack-destroy-failed");
                let message = operation
                    .error_message
                    .as_deref()
                    .unwrap_or("Stack-owned runtime resources could not be confirmed absent");
                bail!(
                    "Stack destroy operation {} failed ({code}): {message}. Safe retry: {retry_command}",
                    operation.operation_id,
                );
            }
            StackDestroyStatus::Pending | StackDestroyStatus::Running => {}
        }

        if started.elapsed() >= timeout {
            bail!(
                "Timed out waiting for stack destroy operation {}. The durable server operation continues; inspect or safely retry with: {retry_command}",
                operation.operation_id,
            );
        }
        let remaining = timeout.saturating_sub(started.elapsed());
        thread::sleep(poll_interval.min(remaining));
        let remaining = timeout.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            bail!(
                "Timed out waiting for stack destroy operation {}. The durable server operation continues; inspect or safely retry with: {retry_command}",
                operation.operation_id,
            );
        }
        operation = match client.get_stack_destroy_with_timeout(
            spec_id,
            &operation.operation_id,
            remaining.min(STACK_DESTROY_REQUEST_TIMEOUT),
        ) {
            Ok(operation) => operation,
            Err(error) => bail!(
                "Failed to inspect stack destroy operation {}: {error:#}. The durable server operation continues; inspect or safely retry with: {retry_command}",
                operation.operation_id,
            ),
        };
        poll_interval = poll_interval
            .checked_mul(2)
            .unwrap_or(STACK_DESTROY_MAX_POLL_INTERVAL)
            .min(STACK_DESTROY_MAX_POLL_INTERVAL);
    }
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

    let response = client.stop_deployment(deployment.id)?;

    if response.operation_id != 0 {
        println!(
            "{} {} (operation #{})",
            "→".blue().bold(),
            response.message,
            response.operation_id
        );
        wait_for_deployment_to_stop(&client, deployment.id)?;
    }

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

fn wait_for_deployment_to_stop(client: &ApiClient, deployment_id: i32) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let deployment = client.get_deployment(deployment_id)?;
        if deployment.live_status.phase == DeploymentPhase::ScaledDown {
            return Ok(());
        }
        if Instant::now() >= deadline {
            bail!(
                "Timed out waiting for deployment operation to scale deployment {} to zero",
                deployment_id
            );
        }
        thread::sleep(Duration::from_secs(2));
    }
}

fn find_deployment<'a>(
    deployments: &'a [DeploymentResponse],
    spec_id: i32,
    branch: Option<&str>,
) -> Option<&'a DeploymentResponse> {
    deployments
        .iter()
        .filter(|deployment| {
            deployment.spec_id == spec_id
                && match branch {
                    Some(branch) => deployment.branch.as_deref() == Some(branch),
                    None => deployment.branch.is_none(), // production deployment has no branch
                }
        })
        // Deployment history can contain multiple records for the same spec and
        // branch. Prefer the record that still owns a serving workload; only use
        // timestamps and IDs to break ties between equivalent lifecycle states.
        .max_by_key(|deployment| deployment_selection_key(deployment))
}

pub(crate) fn deployment_selection_key(
    deployment: &DeploymentResponse,
) -> (bool, u8, u8, Option<&str>, i32) {
    let serving = matches!(
        (deployment.status, deployment.live_status.phase),
        (
            DeploymentStatus::Active | DeploymentStatus::Updating,
            DeploymentPhase::Running | DeploymentPhase::Updating
        )
    );
    let status_priority = match deployment.status {
        DeploymentStatus::Active => 4,
        DeploymentStatus::Updating => 3,
        DeploymentStatus::Failed => 2,
        DeploymentStatus::Stopped => 1,
    };
    let phase_priority = match deployment.live_status.phase {
        DeploymentPhase::Running => 5,
        DeploymentPhase::Updating => 4,
        DeploymentPhase::Degraded => 3,
        DeploymentPhase::ScaledDown => 2,
        DeploymentPhase::Missing => 1,
        DeploymentPhase::Unknown => 0,
    };
    (
        serving,
        status_priority,
        phase_priority,
        deployment.last_deployed_at.as_deref(),
        deployment.id,
    )
}

fn format_deployment_status(status: DeploymentStatus) -> String {
    match status {
        DeploymentStatus::Active => "active".green().to_string(),
        DeploymentStatus::Updating => "updating".yellow().to_string(),
        DeploymentStatus::Stopped => "stopped".dimmed().to_string(),
        DeploymentStatus::Failed => "failed".red().to_string(),
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api_client::{test_support::MockServer, DeploymentLiveStatus};

    const OPERATION_ID: &str = "11111111-1111-4111-8111-111111111111";

    fn destroy_response(status: &str) -> String {
        let (pending, running, succeeded, failed, completed_at, error_code, error_message) =
            match status {
                "pending" => (1, 0, 0, 0, None, None, None),
                "running" => (0, 1, 0, 0, None, None, None),
                "succeeded" => (0, 0, 1, 0, Some("2026-09-04T12:00:02Z"), None, None),
                "failed" => (
                    0,
                    0,
                    1,
                    1,
                    Some("2026-09-04T12:00:02Z"),
                    Some("kubernetes-destroy-failed"),
                    Some("one or more targets failed"),
                ),
                other => panic!("unsupported test status {other}"),
            };
        let target_count = pending + running + succeeded + failed;
        serde_json::json!({
            "schema": "arete.stack-destroy/v1",
            "operationId": OPERATION_ID,
            "specId": 42,
            "status": status,
            "targetCount": target_count,
            "pendingTargets": pending,
            "runningTargets": running,
            "succeededTargets": succeeded,
            "failedTargets": failed,
            "errorCode": error_code,
            "errorMessage": error_message,
            "createdAt": "2026-09-04T12:00:00Z",
            "startedAt": if status == "pending" { None } else { Some("2026-09-04T12:00:01Z") },
            "completedAt": completed_at,
        })
        .to_string()
    }

    fn destroy_client(server: &MockServer) -> ApiClient {
        ApiClient::with_base_url(server.base_url()).with_api_key("a4_ak_test".to_string())
    }

    fn deployment(id: i32, spec_id: i32, branch: Option<&str>) -> DeploymentResponse {
        DeploymentResponse {
            id,
            spec_id,
            spec_name: format!("spec-{spec_id}"),
            atom_name: format!("atom-{id}"),
            branch: branch.map(str::to_string),
            current_build_id: None,
            current_spec_version_id: None,
            current_version: None,
            portable_ast_hash: None,
            deployment_release_hash: None,
            current_idl_program_ids: Vec::new(),
            current_image_tag: None,
            websocket_url: format!("wss://atom-{id}.example.test"),
            http_url: format!("https://atom-{id}.example.test"),
            websocket_auth: serde_json::json!({}),
            http_auth: serde_json::json!({}),
            transaction_relay_enabled: false,
            status: DeploymentStatus::Active,
            status_message: None,
            first_deployed_at: None,
            last_deployed_at: None,
            live_status: DeploymentLiveStatus {
                phase: DeploymentPhase::Running,
                desired_replicas: Some(1),
                ready_replicas: Some(1),
                available_replicas: Some(1),
                updated_replicas: Some(1),
                last_transition_time: None,
                source: "test".to_string(),
                error_category: None,
            },
            latest_operation: None,
        }
    }

    #[test]
    fn find_deployment_selects_newest_record_for_spec_and_branch() {
        let deployments = vec![
            deployment(9, 42, None),
            deployment(12, 42, Some("preview")),
            deployment(15, 7, None),
            deployment(18, 42, None),
        ];

        assert_eq!(
            find_deployment(&deployments, 42, None).map(|item| item.id),
            Some(18)
        );
        assert_eq!(
            find_deployment(&deployments, 42, Some("preview")).map(|item| item.id),
            Some(12)
        );
    }

    #[test]
    fn find_deployment_prefers_serving_record_over_newer_stopped_history() {
        let mut active = deployment(19, 29, None);
        active.last_deployed_at = Some("2026-08-16T23:46:43Z".into());

        let mut stopped = deployment(24, 29, None);
        stopped.status = DeploymentStatus::Stopped;
        stopped.live_status.phase = DeploymentPhase::Missing;
        stopped.last_deployed_at = Some("2026-07-19T23:07:30Z".into());

        assert_eq!(
            find_deployment(&[active, stopped], 29, None).map(|item| item.id),
            Some(19)
        );
    }

    #[test]
    fn find_deployment_ignores_branch_records_for_production_lookup() {
        let production = deployment(19, 29, None);
        let preview = deployment(25, 29, Some("preview"));

        assert_eq!(
            find_deployment(&[production, preview], 29, None).map(|item| item.id),
            Some(19)
        );
    }

    #[test]
    fn delete_confirmation_requires_the_exact_stack_name() {
        assert!(delete_confirmation_matches(
            "settlement-game",
            "settlement-game"
        ));
        assert!(!delete_confirmation_matches(
            "settlement-game",
            "Settlement-Game"
        ));
        assert!(!delete_confirmation_matches("settlement-game", "yes"));
    }

    #[test]
    fn retry_command_shell_quotes_untrusted_stack_names() {
        assert_eq!(
            retry_stack_delete_command("settlement game; echo 'oops'"),
            "a4 stack delete --force -- 'settlement game; echo '\"'\"'oops'\"'\"''"
        );
        assert_eq!(
            retry_stack_delete_command("settlement-game"),
            "a4 stack delete --force -- settlement-game"
        );
    }

    #[test]
    fn destroy_returns_only_after_immediate_terminal_success() {
        let server = MockServer::json(200, &destroy_response("succeeded"));
        let response = destroy_stack_with_options(
            &destroy_client(&server),
            42,
            "settlement-game",
            Duration::from_secs(1),
            Duration::ZERO,
        )
        .expect("destroy succeeds");

        assert_eq!(response.status, StackDestroyStatus::Succeeded);
        assert_eq!(
            server.request().request_line,
            "POST /api/specs/42/destroy HTTP/1.1"
        );
    }

    #[test]
    fn destroy_reuses_the_durable_operation_while_polling() {
        let server = MockServer::json_sequence(vec![
            (202, destroy_response("pending")),
            (200, destroy_response("running")),
            (200, destroy_response("succeeded")),
        ]);
        let response = destroy_stack_with_options(
            &destroy_client(&server),
            42,
            "settlement-game",
            Duration::from_secs(1),
            Duration::ZERO,
        )
        .expect("destroy succeeds after polling");

        assert_eq!(response.operation_id, OPERATION_ID);
        assert_eq!(
            server.request().request_line,
            "POST /api/specs/42/destroy HTTP/1.1"
        );
        for _ in 0..2 {
            assert_eq!(
                server.request().request_line,
                format!("GET /api/specs/42/destroy/{OPERATION_ID} HTTP/1.1")
            );
        }
    }

    #[test]
    fn destroy_surfaces_terminal_failure_with_safe_retry() {
        let server = MockServer::json(200, &destroy_response("failed"));
        let error = destroy_stack_with_options(
            &destroy_client(&server),
            42,
            "settlement-game",
            Duration::from_secs(1),
            Duration::ZERO,
        )
        .expect_err("destroy fails");

        let message = error.to_string();
        assert!(message.contains(OPERATION_ID));
        assert!(message.contains("kubernetes-destroy-failed"));
        assert!(message.contains("a4 stack delete --force -- settlement-game"));
    }

    #[test]
    fn destroy_poll_errors_preserve_operation_identity_and_retry_guidance() {
        let server = MockServer::json_sequence(vec![
            (202, destroy_response("pending")),
            (500, r#"{"error":"server exploded"}"#.to_string()),
        ]);
        let error = destroy_stack_with_options(
            &destroy_client(&server),
            42,
            "settlement game",
            Duration::from_secs(1),
            Duration::ZERO,
        )
        .expect_err("poll failure is surfaced");

        let message = format!("{error:#}");
        assert!(message.contains(OPERATION_ID));
        assert!(message.contains("server exploded"));
        assert!(message.contains("a4 stack delete --force -- 'settlement game'"));
    }

    #[test]
    fn destroy_timeout_explains_that_the_operation_continues() {
        let server = MockServer::json(202, &destroy_response("pending"));
        let error = destroy_stack_with_options(
            &destroy_client(&server),
            42,
            "settlement-game",
            Duration::ZERO,
            Duration::ZERO,
        )
        .expect_err("destroy times out");

        let message = error.to_string();
        assert!(message.contains(OPERATION_ID));
        assert!(message.contains("durable server operation continues"));
        assert!(message.contains("a4 stack delete --force -- settlement-game"));
    }
}
