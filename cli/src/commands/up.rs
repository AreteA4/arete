use anyhow::Result;
use colored::Colorize;
use indicatif::{ProgressBar, ProgressStyle};
use std::time::Duration;

use crate::api_client::{ApiClient, BuildStatus, CreateBuildRequest, DEFAULT_DOMAIN_SUFFIX};
use crate::config::{resolve_stacks_to_push, AreteConfig};
use crate::telemetry;
use crate::ui;

fn generate_short_uuid() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("{:08x}", (timestamp & 0xFFFFFFFF) as u32)
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

    let hash_short = &version_response.version.content_hash[..12];
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

fn watch_build_progress(client: &ApiClient, build_id: i32) -> Result<()> {
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
                }
                BuildStatus::Failed => {
                    ui::print_error("Build failed!");

                    if let Some(msg) = &build.status_message {
                        println!("  {}", msg);
                    }

                    anyhow::bail!("Deployment failed");
                }
                BuildStatus::Cancelled => {
                    ui::print_warning("Build was cancelled.");
                    anyhow::bail!("Deployment cancelled");
                }
                _ => {}
            }

            break;
        }

        std::thread::sleep(Duration::from_millis(ui::DEFAULT_POLL_INTERVAL_MS));
    }

    Ok(())
}
