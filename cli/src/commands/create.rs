use anyhow::{Context, Result};
use colored::Colorize;
use dialoguer::{theme::ColorfulTheme, Input, Select};
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::telemetry;
use crate::templates::{
    customize_project, detect_package_manager, dev_command, install_command, start_command,
    Template, TemplateManager,
};
use crate::ui;

const TEMPLATE_CHOICES: &str = "react-ore|rust-ore|typescript-ore|python-ore";

/// Human progress goes to stdout in normal mode and is suppressed with
/// `--json`, where stdout carries exactly one JSON object.
macro_rules! say {
    ($json:expr, $($arg:tt)*) => {
        if !$json {
            println!($($arg)*);
        }
    };
}

pub fn create(
    name: Option<String>,
    template: Option<String>,
    offline: bool,
    force_refresh: bool,
    skip_install: bool,
    json: bool,
) -> Result<()> {
    let start = std::time::Instant::now();
    let theme = ColorfulTheme::default();

    if !ui::interactive() && (name.is_none() || template.is_none()) {
        let mut missing = Vec::new();
        if name.is_none() {
            missing.push("<name>");
        }
        if template.is_none() {
            missing.push("--template");
        }
        anyhow::bail!(
            "Missing {} and no terminal to prompt on. Pass: a4 create <name> --template {}",
            missing.join(" and "),
            TEMPLATE_CHOICES
        );
    }

    let project_name = match name {
        Some(n) => n,
        None => Input::with_theme(&theme)
            .with_prompt("Project name")
            .default("my-arete-app".to_string())
            .interact_text()
            .context("Failed to read project name")?,
    };

    let selected_template = match template {
        Some(t) => Template::from_str(&t).ok_or_else(|| {
            anyhow::anyhow!("Unknown template: {}. Available: {}", t, TEMPLATE_CHOICES)
        })?,
        None => {
            let items: Vec<String> = Template::ALL
                .iter()
                .map(|t| format!("{} - {}", t.display_name(), t.description()))
                .collect();

            let selection = Select::with_theme(&theme)
                .with_prompt("Select a template")
                .items(&items)
                .default(0)
                .interact()
                .context("Failed to select template")?;

            Template::ALL[selection]
        }
    };

    telemetry::record_template_selected(selected_template.display_name());

    let project_dir = Path::new(&project_name);

    if project_dir.exists() {
        anyhow::bail!(
            "Directory '{}' already exists. Choose a different name or remove it first.",
            project_name
        );
    }

    let manager = TemplateManager::new()?;

    if force_refresh {
        if !json {
            ui::print_step("Clearing template cache...");
        }
        manager.clear_cache()?;
    }

    if !manager.is_cached() {
        if offline {
            anyhow::bail!(
                "Templates not cached and --offline specified. Run without --offline first."
            );
        }

        if !json {
            ui::print_step("Downloading templates...");
        }
        manager.fetch_templates()?;
        say!(json, "  {} Templates cached", ui::symbols::SUCCESS.green());
    }

    if !json {
        ui::print_step(&format!(
            "Creating {} from {}...",
            project_name.bold(),
            selected_template.display_name().cyan()
        ));
    }

    fs::create_dir_all(project_dir)
        .with_context(|| format!("Failed to create directory: {}", project_name))?;

    manager.copy_template(selected_template, project_dir)?;
    customize_project(project_dir, &project_name)?;

    say!(
        json,
        "  {} Project scaffolded",
        ui::symbols::SUCCESS.green()
    );

    let mut installed_dependencies = false;
    let next: Vec<String> = if selected_template.is_rust() {
        say!(json, "");
        if !json {
            print_rust_next_steps(&project_name);
        }
        vec![format!("cd {project_name}"), "cargo run".to_string()]
    } else if selected_template.is_python() {
        say!(json, "");
        if !json {
            print_python_next_steps(&project_name);
        }
        vec![format!("cd {project_name}"), "python main.py".to_string()]
    } else {
        let pm = detect_package_manager();
        installed_dependencies = if skip_install {
            false
        } else {
            run_npm_install(project_dir, pm, json)?
        };
        say!(json, "");
        let run = if selected_template.is_typescript_cli() {
            if !json {
                print_ts_cli_next_steps(&project_name, pm, installed_dependencies);
            }
            start_command(pm)
        } else {
            if !json {
                print_js_next_steps(&project_name, pm, installed_dependencies);
            }
            dev_command(pm)
        };
        let mut next = vec![format!("cd {project_name}")];
        if !installed_dependencies {
            next.push(install_command(pm).to_string());
        }
        next.push(run.to_string());
        next
    };

    telemetry::record_create_completed(selected_template.display_name(), start.elapsed());

    if json {
        let output = serde_json::json!({
            "schemaVersion": 1,
            "path": project_dir.display().to_string(),
            "template": selected_template.display_name(),
            "installedDependencies": installed_dependencies,
            "next": next,
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    }

    Ok(())
}

fn run_npm_install(project_dir: &Path, pm: &str, json: bool) -> Result<bool> {
    if !json {
        ui::print_step("Installing dependencies...");
    }

    let (cmd, args) = match pm {
        "yarn" => ("yarn", vec!["install"]),
        "pnpm" => ("pnpm", vec!["install"]),
        "bun" => ("bun", vec!["install"]),
        _ => ("npm", vec!["install"]),
    };

    // With --json stdout must carry only our JSON object; package-manager
    // chatter goes to stderr. stdin is never inherited so the child cannot
    // block on a prompt.
    let status = Command::new(cmd)
        .args(&args)
        .current_dir(project_dir)
        .stdin(Stdio::null())
        .stdout(if json {
            Stdio::null()
        } else {
            Stdio::inherit()
        })
        .stderr(Stdio::inherit())
        .status()
        .with_context(|| format!("Failed to run {}", install_command(pm)))?;

    if status.success() {
        say!(
            json,
            "  {} Dependencies installed",
            ui::symbols::SUCCESS.green()
        );
        Ok(true)
    } else {
        if json {
            eprintln!(
                "Install failed (exit code: {}); retry with: {}",
                status.code().unwrap_or(-1),
                install_command(pm)
            );
        } else {
            println!(
                "  {} Install failed (exit code: {})",
                ui::symbols::FAILURE.red(),
                status.code().unwrap_or(-1)
            );
            println!(
                "    You can retry manually with: {}",
                install_command(pm).dimmed()
            );
        }
        Ok(false)
    }
}

fn print_js_next_steps(project_name: &str, pm: &str, install_succeeded: bool) {
    println!(
        "{} {}",
        ui::symbols::SUCCESS.green().bold(),
        "Ready!".bold()
    );
    println!();

    if install_succeeded {
        println!("Start the dev server:");
        println!();
        println!(
            "  {} {} && {}",
            "$".dimmed(),
            format!("cd {}", project_name).cyan(),
            dev_command(pm).cyan()
        );
    } else {
        println!("Install dependencies and start:");
        println!();
        println!(
            "  {} {} && {} && {}",
            "$".dimmed(),
            format!("cd {}", project_name).cyan(),
            install_command(pm).cyan(),
            dev_command(pm).cyan()
        );
    }

    println!();
}

fn print_rust_next_steps(project_name: &str) {
    println!(
        "{} {}",
        ui::symbols::SUCCESS.green().bold(),
        "Ready!".bold()
    );
    println!();
    println!("Build and run:");
    println!();
    println!(
        "  {} {} && {}",
        "$".dimmed(),
        format!("cd {}", project_name).cyan(),
        "cargo run".cyan()
    );
    println!();
}

fn print_python_next_steps(project_name: &str) {
    println!(
        "{} {}",
        ui::symbols::SUCCESS.green().bold(),
        "Ready!".bold()
    );
    println!();
    println!("Run the client:");
    println!();
    println!(
        "  {} {} && {}",
        "$".dimmed(),
        format!("cd {}", project_name).cyan(),
        "python main.py".cyan()
    );
    println!();
}

fn print_ts_cli_next_steps(project_name: &str, pm: &str, install_succeeded: bool) {
    println!(
        "{} {}",
        ui::symbols::SUCCESS.green().bold(),
        "Ready!".bold()
    );
    println!();

    if install_succeeded {
        println!("Run the CLI:");
        println!();
        println!(
            "  {} {} && {}",
            "$".dimmed(),
            format!("cd {}", project_name).cyan(),
            start_command(pm).cyan()
        );
    } else {
        println!("Install dependencies and run:");
        println!();
        println!(
            "  {} {} && {} && {}",
            "$".dimmed(),
            format!("cd {}", project_name).cyan(),
            install_command(pm).cyan(),
            start_command(pm).cyan()
        );
    }

    println!();
}
