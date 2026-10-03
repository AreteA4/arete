use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

#[cfg(not(feature = "local"))]
const DEFAULT_API_URL: &str = "https://api.arete.run";

#[cfg(feature = "local")]
const DEFAULT_API_URL: &str = "http://localhost:3000";

pub const PROJECT_AUTH_PROFILE: &str = "agent";
pub const PROJECT_AUTH_RELATIVE_PATH: &str = ".arete/auth.toml";

pub fn project_root(config_path: &str) -> PathBuf {
    Path::new(config_path)
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf()
}

pub fn project_auth_profile_path(root: &Path) -> PathBuf {
    root.join(PROJECT_AUTH_RELATIVE_PATH)
}

pub fn project_auth_profile_contents() -> String {
    format!("default_profile = \"{PROJECT_AUTH_PROFILE}\"\n")
}

/// Read the source-controlled safe default for this repository. Project files
/// may select only the low-privilege agent profile; choosing a human profile
/// inside the repository always requires the explicit `--profile` flag.
pub fn get_project_auth_profile(config_path: &str) -> Result<Option<String>> {
    let path = project_auth_profile_path(&project_root(config_path));
    let content = match std::fs::read_to_string(&path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| format!("Failed to read {}", path.display()))
        }
    };
    let value: toml::Value =
        toml::from_str(&content).with_context(|| format!("Failed to parse {}", path.display()))?;
    let profile = value
        .get("default_profile")
        .and_then(toml::Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("{} must define default_profile", path.display()))?;
    if profile != PROJECT_AUTH_PROFILE {
        anyhow::bail!(
            "{} may select only the low-privilege `{PROJECT_AUTH_PROFILE}` profile; use --profile human explicitly for human access",
            path.display()
        );
    }
    Ok(Some(profile.to_string()))
}

/// Select a credential profile without letting inherited environment state
/// opt a coding agent up inside an initialized repository.
pub fn resolve_auth_profile(
    explicit_profile: Option<&str>,
    config_path: &str,
    inherited_profile: Option<&str>,
) -> Result<Option<String>> {
    if let Some(profile) = explicit_profile {
        return arete_mcp::credentials::validate_profile_name(profile)
            .map(|profile| Some(profile.to_string()));
    }
    if let Some(profile) = get_project_auth_profile(config_path)? {
        return Ok(Some(profile));
    }
    inherited_profile
        .map(|profile| arete_mcp::credentials::validate_profile_name(profile).map(str::to_string))
        .transpose()
}

/// Get the API URL from CLI override, environment variable, or use default.
pub fn get_api_url(override_url: Option<&str>) -> String {
    override_url
        .map(str::to_string)
        .or_else(|| std::env::var("ARETE_API_URL").ok())
        .unwrap_or_else(|| DEFAULT_API_URL.to_string())
}

pub fn to_kebab_case(value: &str) -> String {
    let mut result = String::new();
    for (index, character) in value.chars().enumerate() {
        if character.is_uppercase() {
            if index > 0 {
                result.push('-');
            }
            result.extend(character.to_lowercase());
        } else {
            result.push(character);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_to_kebab_case() {
        assert_eq!(to_kebab_case("SettlementGame"), "settlement-game");
        assert_eq!(to_kebab_case("OreRound"), "ore-round");
        assert_eq!(to_kebab_case("PumpfunToken"), "pumpfun-token");
        assert_eq!(to_kebab_case("simple"), "simple");
        assert_eq!(to_kebab_case("ABC"), "a-b-c");
    }

    #[test]
    fn project_auth_file_selects_only_agent_profile() {
        let directory = tempfile::tempdir().unwrap();
        let auth_path = project_auth_profile_path(directory.path());
        fs::create_dir_all(auth_path.parent().unwrap()).unwrap();
        fs::write(&auth_path, project_auth_profile_contents()).unwrap();

        let config_path = directory.path().join("arete.toml");
        assert_eq!(
            get_project_auth_profile(config_path.to_str().unwrap()).unwrap(),
            Some(PROJECT_AUTH_PROFILE.to_string())
        );

        fs::write(&auth_path, "default_profile = \"human\"\n").unwrap();
        let error = get_project_auth_profile(config_path.to_str().unwrap()).unwrap_err();
        assert!(error.to_string().contains("may select only"));
    }

    #[test]
    fn explicit_then_project_then_environment_profile_precedence() {
        let directory = tempfile::tempdir().unwrap();
        let config_path = directory.path().join("arete.toml");
        let config_path = config_path.to_str().unwrap();

        assert_eq!(
            resolve_auth_profile(None, config_path, Some("human")).unwrap(),
            Some("human".to_string())
        );

        let auth_path = project_auth_profile_path(directory.path());
        fs::create_dir_all(auth_path.parent().unwrap()).unwrap();
        fs::write(&auth_path, project_auth_profile_contents()).unwrap();
        assert_eq!(
            resolve_auth_profile(None, config_path, Some("human")).unwrap(),
            Some("agent".to_string())
        );
        assert_eq!(
            resolve_auth_profile(Some("human"), config_path, Some("agent")).unwrap(),
            Some("human".to_string())
        );
    }
}
