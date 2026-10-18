//! Tool checks for `a4 dev up`: Node.js and npx, which build the protocol
//! (`arete-protocol build`). Everything else runs in the cloud environment.

use std::process::Command;

use anyhow::{bail, Result};

/// What a tool check found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolReport {
    pub name: &'static str,
    pub detail: String,
    pub warning: Option<String>,
}

fn run_version(program: &str, arg: &str) -> Option<String> {
    let output = Command::new(program).arg(arg).output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub fn check_node() -> Result<ToolReport> {
    let Some(node) = run_version("node", "--version") else {
        bail!("node is not installed. Install Node.js 22.13 or newer (https://nodejs.org or nvm).");
    };
    if run_version("npx", "--version").is_none() {
        bail!("npx is not available. It ships with npm; reinstall Node.js with npm.");
    }
    Ok(node_report(node))
}

fn node_report(node: String) -> ToolReport {
    let major = node
        .trim_start_matches('v')
        .split('.')
        .next()
        .and_then(|major| major.parse::<u32>().ok());
    ToolReport {
        name: "node",
        warning: match major {
            Some(major) if major < 22 => Some(format!(
                "node {node} is older than 22; @usearete/protocol needs Node.js 22.13 or newer"
            )),
            _ => None,
        },
        detail: node,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warns_about_old_node() {
        assert!(node_report("v20.11.0".into()).warning.is_some());
        assert!(node_report("v22.13.1".into()).warning.is_none());
        assert!(node_report("weird".into()).warning.is_none());
    }
}
