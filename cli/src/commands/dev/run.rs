//! `a4 dev run -- <command>`: run a command against the running `a4 dev up`.
//!
//! The command gets `ARETE_ENVIRONMENT=dev` and `ARETE_DEV_URL` (the edge),
//! so every installed SDK in it reads and sends through the environment.
//! Browser bundles read no process variables, so the same two values are
//! also copied under each public prefix the project's framework exposes to
//! the browser (`VITE_ARETE_ENVIRONMENT`, `NEXT_PUBLIC_ARETE_DEV_URL`, ...).
//! The edge URL is not a secret: it only answers on this machine.

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use arete_mcp::dev::{DevProject, DEV_UP_HINT};
use clap::Args;

use super::edge::{EDGE_HEADER, WELL_KNOWN_PATH};

/// Public prefixes copied when `[dev] public-env-prefixes` is not set: the
/// ones Vite, Next.js and Expo expose to browser code.
pub const DEFAULT_PUBLIC_PREFIXES: [&str; 3] = ["VITE_", "NEXT_PUBLIC_", "EXPO_PUBLIC_"];

#[derive(Args, Debug, Clone)]
pub struct RunArgs {
    /// The command and its arguments (after `--`)
    #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
    pub command: Vec<String>,
}

/// The variables a command run against the edge gets.
pub fn variables(edge_url: &str, prefixes: &[String]) -> Vec<(String, String)> {
    let mut variables = vec![
        ("ARETE_ENVIRONMENT".to_string(), "dev".to_string()),
        ("ARETE_DEV_URL".to_string(), edge_url.to_string()),
    ];
    for prefix in prefixes {
        variables.push((format!("{prefix}ARETE_ENVIRONMENT"), "dev".to_string()));
        variables.push((format!("{prefix}ARETE_DEV_URL"), edge_url.to_string()));
    }
    variables
}

/// `[dev] public-env-prefixes` from `<root>/arete.toml`, else the defaults.
pub fn public_prefixes(root: &Path) -> Result<Vec<String>> {
    let defaults = || DEFAULT_PUBLIC_PREFIXES.map(str::to_string).to_vec();
    let Ok(text) = std::fs::read_to_string(root.join("arete.toml")) else {
        return Ok(defaults());
    };
    let manifest: toml::Value = toml::from_str(&text).context("arete.toml is not valid TOML")?;
    let Some(listed) = manifest
        .get("dev")
        .and_then(|dev| dev.get("public-env-prefixes"))
    else {
        return Ok(defaults());
    };
    let Some(listed) = listed.as_array() else {
        bail!("[dev] public-env-prefixes must be a list of prefixes, e.g. [\"VITE_\"]");
    };
    listed
        .iter()
        .map(|prefix| match prefix.as_str() {
            Some(prefix)
                if !prefix.is_empty()
                    && prefix.bytes().all(|byte| {
                        byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_'
                    }) =>
            {
                Ok(prefix.to_string())
            }
            _ => bail!(
                "[dev] public-env-prefixes holds '{prefix}': use upper-case letters, digits and _"
            ),
        })
        .collect()
}

/// Whether `edge_url` answers as the edge.
fn edge_answers(edge_url: &str) -> bool {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .ok()
        .and_then(|client| {
            client
                .get(format!(
                    "{}{WELL_KNOWN_PATH}",
                    edge_url.trim_end_matches('/')
                ))
                .send()
                .ok()
        })
        .is_some_and(|response| {
            response.status().is_success()
                && response
                    .headers()
                    .get(EDGE_HEADER)
                    .is_some_and(|value| value == "1")
        })
}

pub fn run(args: RunArgs) -> Result<()> {
    let project = DevProject::discover(None)?;
    let session = project.load_session().ok();
    let Some(edge_url) = session
        .as_ref()
        .map(|session| session.edge_url.clone())
        .filter(|url| !url.is_empty())
    else {
        bail!("`a4 dev up` is not running for this project. {DEV_UP_HINT}");
    };
    if !edge_answers(&edge_url) {
        bail!("`a4 dev up` is not answering at {edge_url}. {DEV_UP_HINT}");
    }
    let prefixes = public_prefixes(&project.root)?;
    let (program, rest) = args
        .command
        .split_first()
        .context("Name the command to run after `--`")?;
    let status = Command::new(program)
        .args(rest)
        .envs(variables(&edge_url, &prefixes))
        .status()
        .with_context(|| format!("Failed to start `{program}`"))?;
    std::process::exit(status.code().unwrap_or(1));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_get_the_switch_and_its_public_copies() {
        let found = variables("http://127.0.0.1:8911", &["VITE_".to_string()]);
        assert_eq!(
            found,
            [
                ("ARETE_ENVIRONMENT", "dev"),
                ("ARETE_DEV_URL", "http://127.0.0.1:8911"),
                ("VITE_ARETE_ENVIRONMENT", "dev"),
                ("VITE_ARETE_DEV_URL", "http://127.0.0.1:8911"),
            ]
            .map(|(name, value)| (name.to_string(), value.to_string()))
        );
    }

    #[test]
    fn prefixes_come_from_the_dev_section() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            public_prefixes(dir.path()).unwrap(),
            DEFAULT_PUBLIC_PREFIXES.map(str::to_string)
        );
        std::fs::write(
            dir.path().join("arete.toml"),
            "[dev]\npublic-env-prefixes = [\"PUBLIC_\"]\n",
        )
        .unwrap();
        assert_eq!(public_prefixes(dir.path()).unwrap(), ["PUBLIC_"]);
        std::fs::write(
            dir.path().join("arete.toml"),
            "[dev]\npublic-env-prefixes = [\"bad-prefix\"]\n",
        )
        .unwrap();
        assert!(public_prefixes(dir.path()).is_err());
    }
}
