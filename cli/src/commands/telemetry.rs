use anyhow::Result;
use colored::Colorize;

use crate::telemetry;
use crate::ui;

pub fn status() -> Result<()> {
    let (enabled, anonymous_id) = telemetry::status();

    if enabled {
        println!(
            "{} {}",
            ui::symbols::SUCCESS.green().bold(),
            "Telemetry is enabled".green().bold()
        );
    } else {
        println!(
            "{} {}",
            ui::symbols::INACTIVE.dimmed(),
            "Telemetry is disabled".dimmed()
        );
    }

    println!();

    if let Some(id) = anonymous_id {
        println!("  Anonymous ID: {}", id.dimmed());
    }

    if std::env::var("DO_NOT_TRACK").is_ok() {
        println!(
            "  {} {}",
            ui::symbols::WARNING.yellow(),
            "DO_NOT_TRACK environment variable is set".yellow()
        );
    }

    if std::env::var("ARETE_TELEMETRY_DISABLED").is_ok() {
        println!(
            "  {} {}",
            ui::symbols::WARNING.yellow(),
            "ARETE_TELEMETRY_DISABLED environment variable is set".yellow()
        );
    }

    println!();
    println!("  Learn more: {}", telemetry::TELEMETRY_DOCS_URL.cyan());

    Ok(())
}

pub fn enable() -> Result<()> {
    telemetry::enable()?;

    println!("{} Telemetry enabled", ui::symbols::SUCCESS.green().bold());
    println!();
    println!("  Thank you for helping improve Arete!");

    Ok(())
}

pub fn disable() -> Result<()> {
    telemetry::disable()?;

    println!("{} Telemetry disabled", ui::symbols::SUCCESS.green().bold());
    println!();
    println!("  No data will be collected.");

    Ok(())
}
