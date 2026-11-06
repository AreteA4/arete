//! Template fetching, caching, and extraction for `a4 create`.
//!
//! Templates are downloaded from the CLI's own GitHub release and cached
//! locally in `~/.arete/templates/{version}/`. The tarball must match its
//! entry in the release's `checksums.txt`, whose minisign signature (for the
//! tag `a4-cli-v{version}`) is verified with the release key, and it is
//! extracted only as plain files and directories under the template
//! directories.

use anyhow::{bail, Context, Result};
use flate2::read::GzDecoder;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};
use tar::Archive;

/// Available project templates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Template {
    /// `a4 protocol init`'s scaffold: a tip jar and its automations.
    Protocol,
    /// The protocol scaffold plus a React app and an automations server.
    ProtocolApp,
    React,
    Rust,
    Typescript,
    Python,
}

impl Template {
    /// All available templates.
    pub const ALL: &'static [Template] = &[
        Template::Protocol,
        Template::ProtocolApp,
        Template::React,
        Template::Rust,
        Template::Typescript,
        Template::Python,
    ];

    /// Template directory name (as stored in tarball).
    pub fn dir_name(&self) -> &'static str {
        match self {
            Template::Protocol => "protocol",
            Template::ProtocolApp => "protocol-app",
            Template::React => "ore-react",
            Template::Rust => "ore-rust",
            Template::Typescript => "ore-typescript",
            Template::Python => "ore-python",
        }
    }

    /// Human-readable display name.
    pub fn display_name(&self) -> &'static str {
        match self {
            Template::Protocol => "protocol",
            Template::ProtocolApp => "protocol-app",
            Template::React => "react-ore",
            Template::Rust => "rust-ore",
            Template::Typescript => "typescript-ore",
            Template::Python => "python-ore",
        }
    }

    /// Description for interactive selection.
    pub fn description(&self) -> &'static str {
        match self {
            Template::Protocol => "A protocol: a tip jar with automations (a4 protocol init)",
            Template::ProtocolApp => {
                "A protocol with a React app and an automations server (Privy, dev-only tools)"
            }
            Template::React => "ORE mining rounds viewer (React + Vite)",
            Template::Rust => "ORE mining rounds client (Rust + Tokio)",
            Template::Typescript => "ORE mining rounds client (TypeScript CLI)",
            Template::Python => "ORE mining rounds client (Python + asyncio)",
        }
    }

    /// Parse from string (CLI argument).
    pub fn from_str(s: &str) -> Option<Template> {
        match s.to_lowercase().as_str() {
            "protocol" => Some(Template::Protocol),
            "protocol-app" => Some(Template::ProtocolApp),
            "react-ore" | "ore-react" => Some(Template::React),
            "rust-ore" | "ore-rust" => Some(Template::Rust),
            "typescript-ore" | "ore-typescript" | "ts-ore" | "ore-ts" => Some(Template::Typescript),
            "python-ore" | "ore-python" | "py-ore" | "ore-py" => Some(Template::Python),
            _ => None,
        }
    }

    /// Built into the CLI, not fetched with the example templates.
    pub fn is_protocol(&self) -> bool {
        matches!(self, Template::Protocol | Template::ProtocolApp)
    }

    pub fn is_rust(&self) -> bool {
        matches!(self, Template::Rust)
    }

    pub fn is_python(&self) -> bool {
        matches!(self, Template::Python)
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
        let cache_dir = crate::arete_home::arete_home()?
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

    /// Fetch templates from the CLI release, verify them and cache them.
    pub fn fetch_templates(&self) -> Result<()> {
        let tag = format!("a4-cli-v{}", self.version);
        let base = format!("https://github.com/AreteA4/arete/releases/download/{tag}");
        let client = reqwest::blocking::Client::new();
        let download = |name: &str| -> Result<Vec<u8>> {
            let url = format!("{base}/{name}");
            let response = client
                .get(&url)
                .header("User-Agent", format!("a4-cli/{}", self.version))
                .send()
                .with_context(|| format!("Failed to download templates from {url}"))?;
            if !response.status().is_success() {
                bail!(
                    "Failed to download {url}: HTTP {} - {}",
                    response.status(),
                    response
                        .status()
                        .canonical_reason()
                        .unwrap_or("Unknown error")
                );
            }
            Ok(response
                .bytes()
                .with_context(|| format!("Failed to read {url}"))?
                .to_vec())
        };
        let tarball = download(&self.tarball_name())?;
        let checksums = download("checksums.txt")?;
        let signature = String::from_utf8(download("checksums.txt.minisig")?)
            .context("checksums.txt.minisig is not UTF-8")?;
        crate::arete_home::ensure_arete_home()?;
        self.install_verified(
            &tarball,
            checksums,
            &signature,
            crate::selfhost::keys::MINISIGN_PUBLIC_KEY,
        )
    }

    fn tarball_name(&self) -> String {
        format!("arete-templates-v{}.tar.gz", self.version)
    }

    /// Verify `tarball` against the release's signed checksums, then extract
    /// it into the cache. Nothing is extracted unless both checks pass; a
    /// failed extraction leaves no cache behind.
    fn install_verified(
        &self,
        tarball: &[u8],
        checksums: Vec<u8>,
        signature: &str,
        public_key: &str,
    ) -> Result<()> {
        use crate::selfhost::verify;
        let tag = format!("a4-cli-v{}", self.version);
        let checksums = verify::verify_release_checksums(checksums, signature, public_key, &tag)
            .context("The template download failed verification; nothing was extracted")?;
        verify::verify_sha256_bytes(&checksums, tarball, &self.tarball_name())
            .context("The template download failed verification; nothing was extracted")?;

        if self.cache_dir.exists() {
            fs::remove_dir_all(&self.cache_dir).with_context(|| {
                format!("Failed to remove cache directory: {:?}", self.cache_dir)
            })?;
        }
        fs::create_dir_all(&self.cache_dir)
            .with_context(|| format!("Failed to create cache directory: {:?}", self.cache_dir))?;
        let extracted = self
            .extract_tarball(tarball)
            .context("Failed to extract templates")
            .and_then(|()| {
                let version_file = self.cache_dir.join(".version");
                fs::write(&version_file, &self.version)
                    .with_context(|| format!("Failed to write version file: {:?}", version_file))
            });
        if extracted.is_err() {
            let _ = fs::remove_dir_all(&self.cache_dir);
        }
        extracted
    }

    /// Extract a gzipped tarball to the cache directory. Only regular files
    /// and directories at relative paths without `..` are accepted; any
    /// other entry (a link, a device, an absolute or escaping path) fails
    /// the extraction.
    fn extract_tarball<R: Read>(&self, reader: R) -> Result<()> {
        let decoder = GzDecoder::new(reader);
        let mut archive = Archive::new(decoder);

        for entry in archive
            .entries()
            .context("Failed to read tarball entries")?
        {
            let mut entry = entry.context("Failed to read tarball entry")?;
            let path = entry
                .path()
                .context("Failed to get entry path")?
                .into_owned();
            let entry_type = entry.header().entry_type();

            let mut components = path.components().peekable();
            while components.peek() == Some(&Component::CurDir) {
                components.next();
            }
            let relative: PathBuf = components
                .map(|component| match component {
                    Component::Normal(part) => Ok(part),
                    _ => Err(anyhow::anyhow!(
                        "template archive entry {} is not a relative path inside the archive",
                        path.display()
                    )),
                })
                .collect::<Result<_>>()?;
            if !(entry_type.is_file() || entry_type.is_dir()) {
                bail!(
                    "template archive entry {} is a {:?}, not a regular file or directory",
                    path.display(),
                    entry_type
                );
            }

            let Some(Component::Normal(top)) = relative.components().next() else {
                continue;
            };
            let is_template = Template::ALL
                .iter()
                .filter(|t| !t.is_protocol())
                .any(|t| top == t.dir_name());
            if !is_template {
                continue;
            }

            let dest = self.cache_dir.join(&relative);

            if entry_type.is_dir() {
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

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_PUBLIC_KEY: &str = "RWSD+pKuMle9GDuXt7KwcW0cF6M+NhhAH31FkxTMNh72Ldc2jkOL0XB0";

    fn fixture(name: &str) -> Vec<u8> {
        fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/templates")
                .join(name),
        )
        .unwrap()
    }

    /// A manager for the fixture release `a4-cli-v0.0.0-test`, caching in a
    /// temporary directory.
    fn manager(version: &str) -> (tempfile::TempDir, TemplateManager) {
        let dir = tempfile::tempdir().unwrap();
        let manager = TemplateManager {
            cache_dir: dir.path().join("templates").join(version),
            version: version.to_string(),
        };
        (dir, manager)
    }

    fn install(manager: &TemplateManager, tarball: &[u8], public_key: &str) -> Result<()> {
        let signature = String::from_utf8(fixture("checksums.txt.minisig")).unwrap();
        manager.install_verified(tarball, fixture("checksums.txt"), &signature, public_key)
    }

    #[test]
    fn a_signed_template_tarball_is_extracted() {
        let (_dir, manager) = manager("0.0.0-test");
        install(
            &manager,
            &fixture("arete-templates-v0.0.0-test.tar.gz"),
            TEST_PUBLIC_KEY,
        )
        .unwrap();
        assert!(manager.is_cached());
        assert_eq!(
            fs::read_to_string(manager.template_path(Template::React).join("src/index.ts"))
                .unwrap(),
            "export const ok = true;\n"
        );
        assert!(!manager.cache_dir.join("unrelated").exists());
    }

    #[test]
    fn a_tampered_or_unsigned_template_tarball_is_rejected() {
        let mut tampered = fixture("arete-templates-v0.0.0-test.tar.gz");
        let last = tampered.len() - 1;
        tampered[last] ^= 1;
        let (_dir, manager) = manager("0.0.0-test");
        let error = install(&manager, &tampered, TEST_PUBLIC_KEY).unwrap_err();
        assert!(
            format!("{error:#}").contains("Checksum check failed"),
            "{error:#}"
        );
        assert!(!manager.cache_dir.exists());

        // Not signed by the release key.
        let error = install(
            &manager,
            &fixture("arete-templates-v0.0.0-test.tar.gz"),
            crate::selfhost::keys::MINISIGN_PUBLIC_KEY,
        )
        .unwrap_err();
        assert!(
            format!("{error:#}").contains("Signature check failed"),
            "{error:#}"
        );
        assert!(!manager.cache_dir.exists());
    }

    #[test]
    fn templates_signed_for_another_release_are_rejected() {
        let (_dir, manager) = manager("0.0.1");
        let error = install(
            &manager,
            &fixture("arete-templates-v0.0.0-test.tar.gz"),
            TEST_PUBLIC_KEY,
        )
        .unwrap_err();
        assert!(
            format!("{error:#}")
                .contains("checksums.txt was signed for 'a4-cli-v0.0.0-test', not 'a4-cli-v0.0.1'"),
            "{error:#}"
        );
        assert!(!manager.cache_dir.exists());
    }

    /// A gzipped tarball of one entry with a raw (unvalidated) name.
    fn crafted(name: &str, entry_type: tar::EntryType, link: &str) -> Vec<u8> {
        let mut header = tar::Header::new_gnu();
        let raw = header.as_old_mut();
        raw.name[..name.len()].copy_from_slice(name.as_bytes());
        raw.linkname[..link.len()].copy_from_slice(link.as_bytes());
        let contents: &[u8] = if entry_type.is_file() {
            b"owned\n"
        } else {
            b""
        };
        header.set_entry_type(entry_type);
        header.set_size(contents.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        let mut builder = tar::Builder::new(Vec::new());
        builder.append(&header, contents).unwrap();
        let tar = builder.into_inner().unwrap();
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        std::io::Write::write_all(&mut gz, &tar).unwrap();
        gz.finish().unwrap()
    }

    #[test]
    fn archive_entries_outside_the_cache_or_links_are_rejected() {
        let (dir, manager) = manager("0.0.0-test");
        fs::create_dir_all(&manager.cache_dir).unwrap();
        let escaped = dir.path().join("escaped");
        let cases = [
            crafted("ore-react/../../../escaped", tar::EntryType::Regular, ""),
            crafted(
                &format!("{}", escaped.display()),
                tar::EntryType::Regular,
                "",
            ),
            crafted("ore-react/link", tar::EntryType::Symlink, "/"),
            crafted("ore-react/hard", tar::EntryType::Link, "/etc/hosts"),
            crafted("ore-react/fifo", tar::EntryType::Fifo, ""),
        ];
        for archive in cases {
            let error = manager.extract_tarball(&archive[..]).unwrap_err();
            assert!(
                error.to_string().contains("template archive entry"),
                "{error:#}"
            );
        }
        assert!(!escaped.exists());
        assert!(!manager.template_path(Template::React).join("link").exists());
        assert!(!manager.template_path(Template::React).join("hard").exists());
    }
}
