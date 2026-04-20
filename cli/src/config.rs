use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

#[cfg(not(feature = "local"))]
const DEFAULT_API_URL: &str = "https://api.arete.run";

#[cfg(feature = "local")]
const DEFAULT_API_URL: &str = "http://localhost:3000";

/// Get the API URL from CLI override, environment variable, or use default
pub fn get_api_url(override_url: Option<&str>) -> String {
    override_url
        .map(|s| s.to_string())
        .or_else(|| std::env::var("ARETE_API_URL").ok())
        .unwrap_or_else(|| DEFAULT_API_URL.to_string())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AreteConfig {
    pub project: ProjectConfig,

    #[serde(default)]
    pub stacks: Vec<StackConfig>,

    #[serde(default)]
    pub sdk: Option<SdkConfig>,

    #[serde(default)]
    pub build: Option<BuildConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectConfig {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SdkConfig {
    #[serde(default = "default_output_dir")]
    pub output_dir: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub typescript_output_dir: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub rust_output_dir: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub typescript_package: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub rust_crate_prefix: Option<String>,

    #[serde(default)]
    pub rust_module_mode: bool,
}

fn default_output_dir() -> String {
    "./generated".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BuildConfig {
    #[serde(default = "default_watch")]
    pub watch_by_default: bool,
}

fn default_watch() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StackConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    pub stack: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub typescript_output_file: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub rust_output_crate: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub rust_module: Option<bool>,

    /// WebSocket URL for the deployed stack (e.g., wss://ore-round-abc123.stack.arete.run)
    /// This is typically set after first deployment and used for SDK generation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

impl AreteConfig {
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();
        let contents = fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file: {}", path.display()))?;

        let config: AreteConfig = toml::from_str(&contents)
            .with_context(|| format!("Failed to parse config file: {}", path.display()))?;

        config.validate()?;
        Ok(config)
    }

    pub fn load_optional<P: AsRef<Path>>(path: P) -> Result<Option<Self>> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(None);
        }
        Self::load(path).map(Some)
    }

    pub fn validate(&self) -> Result<()> {
        if self.project.name.is_empty() {
            anyhow::bail!("Project name cannot be empty");
        }

        let mut names = HashSet::new();
        for stack in &self.stacks {
            if let Some(name) = &stack.name {
                if !names.insert(name.clone()) {
                    anyhow::bail!("Duplicate stack name: {}", name);
                }
            }
        }

        Ok(())
    }

    pub fn find_stack(&self, name: &str) -> Option<&StackConfig> {
        let name_lower = name.to_lowercase();
        let name_kebab = to_kebab_case(name);

        self.stacks.iter().find(|s| {
            s.name.as_deref() == Some(name)
                || s.stack == name
                || s.name.as_deref().map(|n| n.to_lowercase()) == Some(name_lower.clone())
                || s.stack.to_lowercase() == name_lower
                || to_kebab_case(&s.stack) == name_kebab
                || to_kebab_case(&s.stack) == name_lower
        })
    }

    pub fn get_output_dir(&self) -> &str {
        self.sdk
            .as_ref()
            .map(|s| s.output_dir.as_str())
            .unwrap_or("./generated")
    }

    pub fn get_typescript_output_dir(&self) -> &str {
        self.sdk
            .as_ref()
            .and_then(|s| s.typescript_output_dir.as_deref())
            .unwrap_or_else(|| self.get_output_dir())
    }

    pub fn get_rust_output_dir(&self) -> &str {
        self.sdk
            .as_ref()
            .and_then(|s| s.rust_output_dir.as_deref())
            .unwrap_or_else(|| self.get_output_dir())
    }

    pub fn get_typescript_output_path(
        &self,
        stack_name: &str,
        stack_config: Option<&StackConfig>,
        override_path: Option<String>,
    ) -> PathBuf {
        if let Some(path) = override_path {
            return PathBuf::from(path);
        }

        if let Some(stack) = stack_config {
            if let Some(ref file_path) = stack.typescript_output_file {
                return PathBuf::from(file_path);
            }
        }

        PathBuf::from(self.get_typescript_output_dir()).join(format!("{}-stack.ts", stack_name))
    }

    pub fn get_rust_output_path(
        &self,
        stack_name: &str,
        stack_config: Option<&StackConfig>,
        override_path: Option<String>,
    ) -> PathBuf {
        if let Some(path) = override_path {
            return PathBuf::from(path);
        }

        if let Some(stack) = stack_config {
            if let Some(ref crate_path) = stack.rust_output_crate {
                return PathBuf::from(crate_path);
            }
        }

        PathBuf::from(self.get_rust_output_dir()).join(format!("{}-stack", stack_name))
    }
}

#[derive(Debug, Clone)]
pub struct DiscoveredAst {
    pub path: PathBuf,
    pub stack_id: String,
    pub program_ids: Vec<String>,
    pub stack_name: String,
}

impl DiscoveredAst {
    pub fn from_path(path: PathBuf) -> Result<Self> {
        let contents = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read stack file: {}", path.display()))?;

        let ast: serde_json::Value = serde_json::from_str(&contents)
            .with_context(|| format!("Failed to parse stack JSON: {}", path.display()))?;

        let stack_name = ast
            .get("stack_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                anyhow::anyhow!("Stack file missing 'stack_name' field: {}", path.display())
            })?;

        let program_ids = ast
            .get("program_ids")
            .and_then(|v| v.as_array())
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| id.as_str().map(|s| s.to_string()))
                    .collect::<Vec<String>>()
            })
            .filter(|ids| !ids.is_empty())
            .or_else(|| {
                ast.get("program_id")
                    .and_then(|v| v.as_str())
                    .map(|s| vec![s.to_string()])
            })
            .unwrap_or_default();

        Ok(Self {
            path,
            stack_id: stack_name.to_string(),
            program_ids,
            stack_name: to_kebab_case(stack_name),
        })
    }

    pub fn load_ast(&self) -> Result<serde_json::Value> {
        let contents = fs::read_to_string(&self.path)
            .with_context(|| format!("Failed to read stack file: {}", self.path.display()))?;

        serde_json::from_str(&contents)
            .with_context(|| format!("Failed to parse stack JSON: {}", self.path.display()))
    }
}

pub fn to_kebab_case(s: &str) -> String {
    let mut result = String::new();
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() {
            if i > 0 {
                result.push('-');
            }
            result.push(c.to_lowercase().next().unwrap());
        } else {
            result.push(c);
        }
    }
    result
}

pub fn discover_ast_files(base_path: Option<&Path>) -> Result<Vec<DiscoveredAst>> {
    let base = base_path.unwrap_or_else(|| Path::new("."));
    let mut discovered = Vec::new();

    let local_arete = base.join(".arete");
    if local_arete.is_dir() {
        discover_in_dir(&local_arete, &mut discovered)?;
    }

    discover_recursive(base, &mut discovered, 0, 3)?;

    let mut seen_entities = HashSet::new();
    discovered.retain(|ast| seen_entities.insert(ast.stack_id.clone()));

    Ok(discovered)
}

fn discover_in_dir(dir: &Path, discovered: &mut Vec<DiscoveredAst>) -> Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }

    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_file() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.ends_with(".stack.json") {
                    match DiscoveredAst::from_path(path.clone()) {
                        Ok(ast) => discovered.push(ast),
                        Err(e) => {
                            eprintln!("Warning: Failed to parse {}: {}", path.display(), e);
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

fn discover_recursive(
    dir: &Path,
    discovered: &mut Vec<DiscoveredAst>,
    depth: usize,
    max_depth: usize,
) -> Result<()> {
    if depth >= max_depth || !dir.is_dir() {
        return Ok(());
    }

    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name == ".arete" {
                    discover_in_dir(&path, discovered)?;
                } else if !name.starts_with('.') && name != "node_modules" && name != "target" {
                    discover_recursive(&path, discovered, depth + 1, max_depth)?;
                }
            }
        }
    }

    Ok(())
}

pub fn find_ast_file(name: &str, base_path: Option<&Path>) -> Result<Option<DiscoveredAst>> {
    let as_path = Path::new(name);
    if as_path.exists() && as_path.is_file() {
        return DiscoveredAst::from_path(as_path.to_path_buf()).map(Some);
    }

    let discovered = discover_ast_files(base_path)?;

    let name_lower = name.to_lowercase();
    let name_kebab = to_kebab_case(name);

    Ok(discovered.into_iter().find(|ast| {
        ast.stack_id.to_lowercase() == name_lower
            || ast.stack_name == name_kebab
            || ast.stack_name == name_lower
    }))
}

pub fn resolve_stacks_to_push(
    config: Option<&AreteConfig>,
    stack_name: Option<&str>,
) -> Result<Vec<DiscoveredAst>> {
    if let Some(name) = stack_name {
        let ast = find_ast_file(name, None)?.ok_or_else(|| {
            anyhow::anyhow!(
                "Stack file not found for '{}'\n\nSearched in .arete/ directories.\n\
                 Make sure your stack is compiled (cargo build) and the stack file exists.",
                name
            )
        })?;
        return Ok(vec![ast]);
    }

    if let Some(config) = config {
        if !config.stacks.is_empty() {
            let mut result = Vec::new();
            for stack in &config.stacks {
                let ast = find_ast_file(&stack.stack, None)?.ok_or_else(|| {
                    anyhow::anyhow!(
                        "Stack file not found for stack '{}' (stack: '{}')\n\
                         Make sure your stack is compiled and the stack file exists.",
                        stack.name.as_deref().unwrap_or(&stack.stack),
                        stack.stack
                    )
                })?;

                let mut ast = ast;
                if let Some(name) = &stack.name {
                    ast.stack_name = name.clone();
                }
                result.push(ast);
            }
            return Ok(result);
        }
    }

    let discovered = discover_ast_files(None)?;

    if discovered.is_empty() {
        anyhow::bail!(
            "No stack files found.\n\n\
             Make sure you have compiled your stack (cargo build) and have a\n\
            .arete/*.stack.json file in your project.\n\n\
             Alternatively, create a arete.toml to configure your stacks."
        );
    }

    Ok(discovered)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_kebab_case() {
        assert_eq!(to_kebab_case("SettlementGame"), "settlement-game");
        assert_eq!(to_kebab_case("OreRound"), "ore-round");
        assert_eq!(to_kebab_case("PumpfunToken"), "pumpfun-token");
        assert_eq!(to_kebab_case("simple"), "simple");
        assert_eq!(to_kebab_case("ABC"), "a-b-c");
    }
}
