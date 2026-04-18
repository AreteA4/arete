//! Template fetching, caching, and extraction for `a4 create`.
//!
//! Templates are downloaded from GitHub releases and cached locally
//! in `~/.arete/templates/{version}/`.

use anyhow::{Context, Result};
use flate2::read::GzDecoder;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use tar::Archive;

/// Available project templates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Template {
    React,
    Rust,
    Typescript,
}

impl Template {
    /// All available templates.
    pub const ALL: &'static [Template] = &[Template::React, Template::Rust, Template::Typescript];

    /// Template directory name (as stored in tarball).
    pub fn dir_name(&self) -> &'static str {
        match self {
            Template::React => "ore-react",
            Template::Rust => "ore-rust",
            Template::Typescript => "ore-typescript",
        }
    }

    /// Human-readable display name.
    pub fn display_name(&self) -> &'static str {
        match self {
            Template::React => "react-ore",
            Template::Rust => "rust-ore",
            Template::Typescript => "typescript-ore",
        }
    }

    /// Description for interactive selection.
    pub fn description(&self) -> &'static str {
        match self {
            Template::React => "ORE mining rounds viewer (React + Vite)",
            Template::Rust => "ORE mining rounds client (Rust + Tokio)",
            Template::Typescript => "ORE mining rounds client (TypeScript CLI)",
        }
    }

    /// Parse from string (CLI argument).
    pub fn from_str(s: &str) -> Option<Template> {
        match s.to_lowercase().as_str() {
            "react-ore" | "ore-react" => Some(Template::React),
            "rust-ore" | "ore-rust" => Some(Template::Rust),
            "typescript-ore" | "ore-typescript" | "ts-ore" | "ore-ts" => Some(Template::Typescript),
            _ => None,
        }
    }

    pub fn is_rust(&self) -> bool {
        matches!(self, Template::Rust)
    }

    pub fn is_typescript_cli(&self) -> bool {
        matches!(self, Template::Typescript)
    }
}

/// Template manager handles fetching, caching, and extracting templates.
pub struct TemplateManager {
    cache_dir: PathBuf,
    version: String,
}

impl TemplateManager {
    /// Create a new template manager for the current CLI version.
    pub fn new() -> Result<Self> {
        let version = env!("CARGO_PKG_VERSION").to_string();
        let cache_dir = dirs::home_dir()
            .context("Could not determine home directory")?
            .join(".arete")
            .join("templates")
            .join(&version);

        Ok(Self { cache_dir, version })
    }

    /// Check if templates are cached for the current version.
    pub fn is_cached(&self) -> bool {
        self.cache_dir.exists() && self.cache_dir.join(".version").exists()
    }

    /// Get the path to a cached template directory.
    pub fn template_path(&self, template: Template) -> PathBuf {
        self.cache_dir.join(template.dir_name())
    }

    /// Fetch templates from GitHub releases and cache them.
    pub fn fetch_templates(&self) -> Result<()> {
        let url = format!(
            "https://github.com/AreteA4/arete/releases/download/a4-cli-v{}/arete-templates-v{}.tar.gz",
            self.version, self.version
        );

        fs::create_dir_all(&self.cache_dir)
            .with_context(|| format!("Failed to create cache directory: {:?}", self.cache_dir))?;

        let response = reqwest::blocking::Client::new()
            .get(&url)
            .header("User-Agent", format!("a4-cli/{}", self.version))
            .send()
            .with_context(|| format!("Failed to download templates from {}", url))?;

        if !response.status().is_success() {
            anyhow::bail!(
                "Failed to download templates: HTTP {} - {}",
                response.status(),
                response
                    .status()
                    .canonical_reason()
                    .unwrap_or("Unknown error")
            );
        }

        let bytes = response
            .bytes()
            .context("Failed to read template tarball")?;

        self.extract_tarball(&bytes[..])
            .context("Failed to extract templates")?;

        let version_file = self.cache_dir.join(".version");
        fs::write(&version_file, &self.version)
            .with_context(|| format!("Failed to write version file: {:?}", version_file))?;

        Ok(())
    }

    /// Extract a gzipped tarball to the cache directory.
    fn extract_tarball<R: Read>(&self, reader: R) -> Result<()> {
        let decoder = GzDecoder::new(reader);
        let mut archive = Archive::new(decoder);

        for entry in archive
            .entries()
            .context("Failed to read tarball entries")?
        {
            let mut entry = entry.context("Failed to read tarball entry")?;
            let path = entry.path().context("Failed to get entry path")?;

            let path_str = path.to_string_lossy();
            let is_template = Template::ALL
                .iter()
                .any(|t| path_str.starts_with(t.dir_name()));

            if !is_template {
                continue;
            }

            let dest = self.cache_dir.join(&*path);

            if entry.header().entry_type().is_dir() {
                fs::create_dir_all(&dest)
                    .with_context(|| format!("Failed to create directory: {:?}", dest))?;
            } else {
                if let Some(parent) = dest.parent() {
                    fs::create_dir_all(parent).with_context(|| {
                        format!("Failed to create parent directory: {:?}", parent)
                    })?;
                }

                let mut file = File::create(&dest)
                    .with_context(|| format!("Failed to create file: {:?}", dest))?;
                io::copy(&mut entry, &mut file)
                    .with_context(|| format!("Failed to write file: {:?}", dest))?;
            }
        }

        Ok(())
    }

    /// Clear the template cache (for --force-refresh).
    pub fn clear_cache(&self) -> Result<()> {
        if self.cache_dir.exists() {
            fs::remove_dir_all(&self.cache_dir).with_context(|| {
                format!("Failed to remove cache directory: {:?}", self.cache_dir)
            })?;
        }
        Ok(())
    }

    /// Copy a template to the target directory.
    pub fn copy_template(&self, template: Template, target_dir: &Path) -> Result<()> {
        let source = self.template_path(template);

        if !source.exists() {
            anyhow::bail!(
                "Template '{}' not found in cache. Try running without --offline.",
                template.display_name()
            );
        }

        copy_dir_recursive(&source, target_dir)
            .with_context(|| format!("Failed to copy template to {:?}", target_dir))?;

        Ok(())
    }
}

/// Recursively copy a directory.
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst)?;

    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path)?;
        }
    }

    Ok(())
}

/// Customize the scaffolded project.
pub fn customize_project(project_dir: &Path, project_name: &str) -> Result<()> {
    update_package_json_name(project_dir, project_name)?;
    update_cargo_toml_name(project_dir, project_name)?;
    update_html_title(project_dir, project_name)?;
    copy_env_example(project_dir)?;
    Ok(())
}

fn update_package_json_name(project_dir: &Path, project_name: &str) -> Result<()> {
    let path = project_dir.join("package.json");
    if !path.exists() {
        return Ok(());
    }

    let content = fs::read_to_string(&path).context("Failed to read package.json")?;
    let mut json: serde_json::Value =
        serde_json::from_str(&content).context("Failed to parse package.json")?;

    if let Some(obj) = json.as_object_mut() {
        obj.insert(
            "name".to_string(),
            serde_json::Value::String(project_name.to_string()),
        );
    }

    let updated =
        serde_json::to_string_pretty(&json).context("Failed to serialize package.json")?;
    fs::write(&path, updated).context("Failed to write package.json")?;
    Ok(())
}

fn update_cargo_toml_name(project_dir: &Path, project_name: &str) -> Result<()> {
    let path = project_dir.join("Cargo.toml");
    if !path.exists() {
        return Ok(());
    }

    let content = fs::read_to_string(&path).context("Failed to read Cargo.toml")?;

    let updated = content
        .lines()
        .map(|line| {
            if line.starts_with("name = ") {
                format!("name = \"{}\"", project_name)
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    let updated = if content.ends_with('\n') {
        format!("{}\n", updated)
    } else {
        updated
    };

    fs::write(&path, updated).context("Failed to write Cargo.toml")?;
    Ok(())
}

fn update_html_title(project_dir: &Path, project_name: &str) -> Result<()> {
    let path = project_dir.join("index.html");
    if !path.exists() {
        return Ok(());
    }

    let content = fs::read_to_string(&path).context("Failed to read index.html")?;

    let updated = if let Some(start) = content.find("<title>") {
        if let Some(end) = content[start..].find("</title>") {
            let before = &content[..start + 7];
            let after = &content[start + end..];
            format!("{}{}{}", before, project_name, after)
        } else {
            content
        }
    } else {
        content
    };

    fs::write(&path, updated).context("Failed to write index.html")?;
    Ok(())
}

fn copy_env_example(project_dir: &Path) -> Result<()> {
    let env_example = project_dir.join(".env.example");
    let env_file = project_dir.join(".env");
    if env_example.exists() && !env_file.exists() {
        fs::copy(&env_example, &env_file).context("Failed to copy .env.example to .env")?;
    }
    Ok(())
}

/// Detect package manager: first from npm_config_user_agent (when run via npx/pnpm dlx/etc),
/// then by checking which package managers are installed on the system.
pub fn detect_package_manager() -> &'static str {
    // 1. Check if invoked via a package manager (npx, pnpm dlx, yarn dlx, bunx)
    if let Ok(user_agent) = std::env::var("npm_config_user_agent") {
        if user_agent.starts_with("yarn") {
            return "yarn";
        } else if user_agent.starts_with("pnpm") {
            return "pnpm";
        } else if user_agent.starts_with("bun") {
            return "bun";
        } else if user_agent.starts_with("npm") {
            return "npm";
        }
    }

    // 2. Check what's available on the system (prefer faster ones)
    if is_command_available("bun") {
        return "bun";
    }
    if is_command_available("pnpm") {
        return "pnpm";
    }
    if is_command_available("yarn") {
        return "yarn";
    }
    if is_command_available("npm") {
        return "npm";
    }

    // 3. Default to npm (will fail with clear error if not installed)
    "npm"
}

fn is_command_available(cmd: &str) -> bool {
    std::process::Command::new(cmd)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Get the install command for a package manager.
pub fn install_command(pm: &str) -> &'static str {
    match pm {
        "yarn" => "yarn",
        "pnpm" => "pnpm install",
        "bun" => "bun install",
        _ => "npm install",
    }
}

/// Get the dev command for a package manager.
pub fn dev_command(pm: &str) -> &'static str {
    match pm {
        "yarn" => "yarn dev",
        "pnpm" => "pnpm dev",
        "bun" => "bun dev",
        _ => "npm run dev",
    }
}

/// Get the start command for a package manager.
pub fn start_command(pm: &str) -> &'static str {
    match pm {
        "yarn" => "yarn start",
        "pnpm" => "pnpm start",
        "bun" => "bun start",
        _ => "npm start",
    }
}
