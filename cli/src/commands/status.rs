use anyhow::Result;
use colored::Colorize;
use serde::Serialize;

use crate::api_client::{
    ApiClient, Build, BuildStatus, DeploymentResponse, DeploymentStatus, Spec,
};

#[derive(Serialize)]
struct StatusOutput {
    deployments: Vec<DeploymentInfo>,
    in_progress: Vec<BuildInfo>,
    failed: Vec<BuildInfo>,
    undeployed_stacks: Vec<String>,
}

#[derive(Serialize)]
struct DeploymentInfo {
    id: i32,
    stack_name: String,
    branch: Option<String>,
    websocket_url: String,
    current_version: Option<i32>,
    status: String,
    deployed_at: Option<String>,
}

#[derive(Serialize)]
struct BuildInfo {
    build_id: i32,
    stack_name: String,
    status: String,
    current_phase: Option<String>,
    phase_progress: Option<i32>,
    status_message: Option<String>,
}

pub fn status(json: bool) -> Result<()> {
    let client = ApiClient::new()?;

    let specs = client.list_specs()?;
    let builds = client.list_builds(Some(50), None)?;
    let deployments = client.list_deployments(50)?;

    let active_deployments: Vec<_> = deployments
        .iter()
        .filter(|d| d.status == DeploymentStatus::Active || d.status == DeploymentStatus::Updating)
        .collect();

    let in_progress: Vec<&Build> = builds.iter().filter(|b| !b.status.is_terminal()).collect();

    let failed_recent: Vec<&Build> = builds
        .iter()
        .filter(|b| b.status == BuildStatus::Failed)
        .take(3)
        .collect();

    println!();
    println!("{}", "Arete Status".bold());
    println!("{}", "─".repeat(50).dimmed());

    println!();
    if active_deployments.is_empty() {
        println!("{} No active deployments", "○".dimmed());
    } else {
        println!(
            "{} {} active deployment{}",
            "●".green(),
            active_deployments.len(),
            if active_deployments.len() == 1 {
                ""
            } else {
                "s"
            }
        );

        for deployment in &active_deployments {
            print_deployment(deployment);
        }
    }

    if !in_progress.is_empty() {
        println!();
        println!(
            "{} {} build{} in progress",
            "◐".blue(),
            in_progress.len(),
            if in_progress.len() == 1 { "" } else { "s" }
        );

        for build in &in_progress {
            print_in_progress_build(build, &specs);
        }
    }

    if !failed_recent.is_empty() {
        println!();
        println!(
            "{} {} recent failed build{}",
            "✗".red(),
            failed_recent.len(),
            if failed_recent.len() == 1 { "" } else { "s" }
        );

        for build in &failed_recent {
            print_failed_build(build, &specs);
        }
    }

    let deployed_spec_ids: std::collections::HashSet<_> =
        active_deployments.iter().map(|d| d.spec_id).collect();

    let undeployed_stacks: Vec<_> = specs
        .iter()
        .filter(|s| !deployed_spec_ids.contains(&s.id))
        .collect();

    if json {
        let output = StatusOutput {
            deployments: active_deployments
                .iter()
                .map(|d| DeploymentInfo {
                    id: d.id,
                    stack_name: d.spec_name.clone(),
                    branch: d.branch.clone(),
                    websocket_url: d.websocket_url.clone(),
                    current_version: d.current_version,
                    status: d.status.to_string(),
                    deployed_at: d.last_deployed_at.clone(),
                })
                .collect(),
            in_progress: in_progress
                .iter()
                .map(|b| {
                    let stack_name = b
                        .spec_id
                        .and_then(|id| specs.iter().find(|s| s.id == id))
                        .map(|s| s.name.clone())
                        .unwrap_or_else(|| "unknown".to_string());
                    BuildInfo {
                        build_id: b.id,
                        stack_name,
                        status: b.status.to_string(),
                        current_phase: b.phase.clone(),
                        phase_progress: b.progress,
                        status_message: None,
                    }
                })
                .collect(),
            failed: failed_recent
                .iter()
                .map(|b| {
                    let stack_name = b
                        .spec_id
                        .and_then(|id| specs.iter().find(|s| s.id == id))
                        .map(|s| s.name.clone())
                        .unwrap_or_else(|| "unknown".to_string());
                    BuildInfo {
                        build_id: b.id,
                        stack_name,
                        status: b.status.to_string(),
                        current_phase: b.phase.clone(),
                        phase_progress: b.progress,
                        status_message: b.status_message.clone(),
                    }
                })
                .collect(),
            undeployed_stacks: undeployed_stacks.iter().map(|s| s.name.clone()).collect(),
        };
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    if !undeployed_stacks.is_empty() {
        println!();
        println!(
            "{} {} stack{} not deployed",
            "○".dimmed(),
            undeployed_stacks.len(),
            if undeployed_stacks.len() == 1 {
                ""
            } else {
                "s"
            }
        );

        for stack in undeployed_stacks {
            println!("    {} {}", "·".dimmed(), stack.name.dimmed());
        }
    }

    println!();
    println!("{}", "─".repeat(50).dimmed());
    println!("{}", "Quick commands:".dimmed());
    println!("  {}  Deploy a stack", "a4 up".cyan());
    println!("  {}  List all stacks", "a4 stack list".cyan());
    println!("  {}  Show stack details", "a4 stack show <name>".cyan());

    println!();

    Ok(())
}

fn print_deployment(deployment: &DeploymentResponse) {
    let name_with_branch = if let Some(branch) = &deployment.branch {
        format!("{} ({})", deployment.spec_name, branch)
    } else {
        deployment.spec_name.clone()
    };

    let status_indicator = match deployment.status {
        DeploymentStatus::Active => "→".green(),
        DeploymentStatus::Updating => "↻".yellow(),
        DeploymentStatus::Stopped => "○".dimmed(),
        DeploymentStatus::Failed => "✗".red(),
    };

    println!();
    println!("    {} {}", status_indicator, name_with_branch.bold());
    println!("      {}", deployment.websocket_url.cyan());

    if let Some(version) = deployment.current_version {
        print!("      v{}", version);
    }

    if let Some(deployed) = &deployment.last_deployed_at {
        if let Some(age) = format_relative_time(deployed) {
            println!(" - deployed {}", age.dimmed());
        } else {
            println!();
        }
    } else {
        println!();
    }
}

fn print_in_progress_build(build: &Build, specs: &[Spec]) {
    let stack_name = build
        .spec_id
        .and_then(|id| specs.iter().find(|s| s.id == id))
        .map(|s| s.name.as_str())
        .unwrap_or("unknown");

    let status = format_status(build.status);
    let phase = build
        .phase
        .as_ref()
        .map(|p| humanize_phase(p))
        .unwrap_or("");

    println!(
        "    {} #{} {} - {} {}",
        "→".blue(),
        build.id,
        stack_name,
        status,
        phase.dimmed()
    );

    if let Some(progress) = build.progress {
        println!("      {}%", progress);
    }
}

fn print_failed_build(build: &Build, specs: &[Spec]) {
    let stack_name = build
        .spec_id
        .and_then(|id| specs.iter().find(|s| s.id == id))
        .map(|s| s.name.as_str())
        .unwrap_or("unknown");

    println!("    {} #{} {}", "→".red(), build.id, stack_name);

    if let Some(msg) = &build.status_message {
        let display_msg = if msg.len() > 60 {
            format!("{}...", &msg[..57])
        } else {
            msg.clone()
        };
        println!("      {}", display_msg.dimmed());
    }
}

fn format_status(status: BuildStatus) -> String {
    match status {
        BuildStatus::Pending => "pending".yellow().to_string(),
        BuildStatus::Uploading => "uploading".yellow().to_string(),
        BuildStatus::Queued => "queued".yellow().to_string(),
        BuildStatus::Building => "building".blue().to_string(),
        BuildStatus::Pushing => "pushing".blue().to_string(),
        BuildStatus::Deploying => "deploying".blue().to_string(),
        BuildStatus::Completed => "completed".green().to_string(),
        BuildStatus::Failed => "failed".red().to_string(),
        BuildStatus::Cancelled => "cancelled".dimmed().to_string(),
    }
}

fn humanize_phase(phase: &str) -> &str {
    match phase.to_uppercase().as_str() {
        "SUBMITTED" => "queued",
        "PROVISIONING" => "starting",
        "DOWNLOAD_SOURCE" => "preparing",
        "INSTALL" => "installing",
        "PRE_BUILD" => "preparing",
        "BUILD" => "compiling",
        "POST_BUILD" => "finalizing",
        "UPLOAD_ARTIFACTS" => "publishing",
        "FINALIZING" => "deploying",
        _ => phase,
    }
}

fn format_relative_time(timestamp: &str) -> Option<String> {
    let parsed = chrono::DateTime::parse_from_rfc3339(timestamp).ok()?;
    let now = chrono::Utc::now();
    let duration = now.signed_duration_since(parsed);

    let seconds = duration.num_seconds();
    if seconds < 0 {
        return Some("just now".to_string());
    }

    if seconds < 60 {
        return Some("just now".to_string());
    }

    let minutes = duration.num_minutes();
    if minutes < 60 {
        return Some(format!(
            "{} minute{} ago",
            minutes,
            if minutes == 1 { "" } else { "s" }
        ));
    }

    let hours = duration.num_hours();
    if hours < 24 {
        return Some(format!(
            "{} hour{} ago",
            hours,
            if hours == 1 { "" } else { "s" }
        ));
    }

    let days = duration.num_days();
    if days < 30 {
        return Some(format!(
            "{} day{} ago",
            days,
            if days == 1 { "" } else { "s" }
        ));
    }

    Some(parsed.format("%Y-%m-%d").to_string())
}
