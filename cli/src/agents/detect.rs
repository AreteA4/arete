//! Coding-agent detection (spec WP7 table).
//!
//! Signals per agent, in precedence order: an environment variable set by
//! the agent that is driving this process (`env`), project files (`project`),
//! then the agent's home directory (`home`). Every env signal carries a
//! source comment in `signals` (verified 2026-09-03); anything that could
//! not be cited was dropped. The rest of the table has no env signal.

use std::path::PathBuf;

use serde::Serialize;

use super::{Env, AGENT_IDS};

/// How an agent was detected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum How {
    Project,
    Home,
    Env,
}

impl std::fmt::Display for How {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            How::Project => "project",
            How::Home => "home",
            How::Env => "env",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DetectedAgent {
    pub id: String,
    pub how: How,
}

/// Detection result: agents in table order plus the `universal` marker
/// (`.agents/` present in the project).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Detection {
    pub agents: Vec<DetectedAgent>,
    pub universal: bool,
}

impl Detection {
    pub fn ids(&self) -> Vec<String> {
        self.agents.iter().map(|agent| agent.id.clone()).collect()
    }

    pub fn contains(&self, id: &str) -> bool {
        self.agents.iter().any(|agent| agent.id == id)
    }
}

/// Signals for one agent.
struct Signals {
    project: &'static [&'static str],
    env: &'static [&'static str],
}

fn signals(id: &str) -> Signals {
    let (project, env): (&'static [&'static str], &'static [&'static str]) = match id {
        // CLAUDECODE=1: spec WP7 table (confirmed before implementation).
        "claude-code" => (&[".claude", "CLAUDE.md", ".mcp.json"], &["CLAUDECODE"]),
        // CURSOR_AGENT: https://cursor.com/docs/agent/tools/terminal ("use the
        // CURSOR_AGENT environment variable in your shell config to detect when
        // Cursor is running").
        "cursor" => (&[".cursor"], &["CURSOR_AGENT"]),
        // openai/codex: CODEX_THREAD_ID is injected into every shell-tool env
        // (codex-rs/protocol/src/shell_environment.rs, create_env step 6);
        // CODEX_SANDBOX=seatbelt under the macOS sandbox and
        // CODEX_SANDBOX_NETWORK_DISABLED=1 when network is restricted
        // (codex-rs/core/src/spawn.rs, codex-rs/core/src/sandboxing/mod.rs).
        "codex" => (
            &[".codex"],
            &[
                "CODEX_THREAD_ID",
                "CODEX_SANDBOX",
                "CODEX_SANDBOX_NETWORK_DISABLED",
            ],
        ),
        // anomalyco/opencode: the shell tool inherits process.env unchanged
        // (packages/opencode/src/tool/shell.ts, shellEnv) and OPENCODE_CLIENT is
        // only set under the ACP host (packages/opencode/src/cli/cmd/acp.ts,
        // `process.env.OPENCODE_CLIENT = "acp"`) and the desktop app; there is
        // no bare `OPENCODE` variable, so that signal was removed.
        "opencode" => (
            &["opencode.json", "opencode.jsonc", ".opencode"],
            &["OPENCODE_CLIENT"],
        ),
        // GEMINI_CLI=1: https://geminicli.com/docs/tools/shell/ ("When
        // run_shell_command executes a command, it sets the GEMINI_CLI=1
        // environment variable in the subprocess's environment").
        "gemini-cli" => (&[".gemini"], &["GEMINI_CLI"]),
        "vscode" => (&[".vscode"], &[]),
        "copilot-cli" => (&[".github/copilot-instructions.md"], &[]),
        "windsurf" => (&[".windsurf"], &[]),
        "cline" => (&[".clinerules"], &[]),
        "zed" => (&[".zed"], &[]),
        "amp" => (&[".amp"], &[]),
        "kiro" => (&[".kiro"], &[]),
        "roo" => (&[".roo"], &[]),
        "goose" => (&[".goose"], &[]),
        _ => (&[], &[]),
    };
    Signals { project, env }
}

/// Home-directory signal for one agent (None = no home signal).
pub fn home_signal(env: &Env, id: &str) -> Option<PathBuf> {
    let home = env.home.as_ref();
    match id {
        "claude-code" => home.map(|h| h.join(".claude")),
        "cursor" => home.map(|h| h.join(".cursor")),
        "codex" => env.codex_home(),
        "opencode" => env.xdg_config_home().map(|c| c.join("opencode")),
        "gemini-cli" => home.map(|h| h.join(".gemini")),
        "vscode" => None,
        "copilot-cli" => home.map(|h| h.join(".copilot")),
        "windsurf" => home.map(|h| h.join(".codeium").join("windsurf")),
        "cline" => home.map(|h| h.join(".cline")),
        "zed" => home.map(|h| h.join(".config").join("zed")),
        "amp" => home.map(|h| h.join(".config").join("amp")),
        "kiro" => home.map(|h| h.join(".kiro")),
        "roo" => home.map(|h| h.join(".roo")),
        "goose" => home.map(|h| h.join(".config").join("goose")),
        _ => None,
    }
}

fn detect_one(env: &Env, id: &str) -> Option<How> {
    let signals = signals(id);
    if signals.env.iter().any(|key| env.var(key).is_some()) {
        return Some(How::Env);
    }
    if signals
        .project
        .iter()
        .any(|relative| env.root.join(relative).exists())
    {
        return Some(How::Project);
    }
    if home_signal(env, id).is_some_and(|path: PathBuf| path.exists()) {
        return Some(How::Home);
    }
    None
}

/// Detect every agent from the WP7 table.
pub fn detect(env: &Env) -> Detection {
    let agents = AGENT_IDS
        .iter()
        .filter_map(|id| {
            detect_one(env, id).map(|how| DetectedAgent {
                id: (*id).to_string(),
                how,
            })
        })
        .collect();
    Detection {
        agents,
        universal: env.root.join(".agents").is_dir(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    fn env(root: &Path, home: &Path, vars: &[(&str, &str)]) -> Env {
        Env::new(root, Some(home.to_path_buf()), vars)
    }

    #[test]
    fn nothing_detected_in_empty_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("proj");
        let home = dir.path().join("home");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&home).unwrap();
        let detection = detect(&env(&root, &home, &[]));
        assert_eq!(detection, Detection::default());
    }

    #[test]
    fn env_wins_over_project_over_home() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("proj");
        let home = dir.path().join("home");
        fs::create_dir_all(root.join(".claude")).unwrap();
        fs::create_dir_all(home.join(".claude")).unwrap();
        fs::create_dir_all(home.join(".cursor")).unwrap();
        fs::create_dir_all(root.join(".agents")).unwrap();
        fs::write(root.join("opencode.jsonc"), "{}").unwrap();

        let detection = detect(&env(
            &root,
            &home,
            &[("CLAUDECODE", "1"), ("GEMINI_CLI", "1")],
        ));
        assert!(detection.universal);
        assert_eq!(
            detection.agents,
            vec![
                DetectedAgent {
                    id: "claude-code".into(),
                    how: How::Env
                },
                DetectedAgent {
                    id: "cursor".into(),
                    how: How::Home
                },
                DetectedAgent {
                    id: "opencode".into(),
                    how: How::Project
                },
                DetectedAgent {
                    id: "gemini-cli".into(),
                    how: How::Env
                },
            ]
        );

        let detection = detect(&env(&root, &home, &[]));
        assert_eq!(detection.agents[0].how, How::Project);
    }

    #[test]
    fn codex_and_opencode_honour_their_home_overrides() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("proj");
        let home = dir.path().join("home");
        let codex_home = dir.path().join("codex-home");
        let xdg = dir.path().join("xdg");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&codex_home).unwrap();
        fs::create_dir_all(xdg.join("opencode")).unwrap();
        let detection = detect(&env(
            &root,
            &home,
            &[
                ("CODEX_HOME", codex_home.to_str().unwrap()),
                ("XDG_CONFIG_HOME", xdg.to_str().unwrap()),
            ],
        ));
        assert_eq!(detection.ids(), vec!["codex", "opencode"]);
        assert!(detection.agents.iter().all(|a| a.how == How::Home));
    }

    #[test]
    fn env_signals_are_the_cited_set_only() {
        assert_eq!(signals("claude-code").env, ["CLAUDECODE"]);
        assert_eq!(signals("cursor").env, ["CURSOR_AGENT"]);
        assert_eq!(
            signals("codex").env,
            [
                "CODEX_THREAD_ID",
                "CODEX_SANDBOX",
                "CODEX_SANDBOX_NETWORK_DISABLED"
            ]
        );
        assert_eq!(signals("opencode").env, ["OPENCODE_CLIENT"]);
        assert_eq!(signals("gemini-cli").env, ["GEMINI_CLI"]);
        for id in AGENT_IDS.iter().skip(5) {
            assert!(signals(id).env.is_empty(), "{id}");
        }

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("proj");
        let home = dir.path().join("home");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&home).unwrap();
        // The unverified bare `OPENCODE` var no longer counts; the cited ones do.
        assert!(detect(&env(&root, &home, &[("OPENCODE", "1")]))
            .agents
            .is_empty());
        let detection = detect(&env(
            &root,
            &home,
            &[
                ("OPENCODE_CLIENT", "acp"),
                ("CODEX_THREAD_ID", "t-1"),
                ("CURSOR_AGENT", "1"),
            ],
        ));
        assert_eq!(detection.ids(), vec!["cursor", "codex", "opencode"]);
        assert!(detection.agents.iter().all(|a| a.how == How::Env));
    }

    #[test]
    fn every_agent_has_a_project_signal_and_json_shape_is_flat() {
        for id in AGENT_IDS {
            assert!(!signals(id).project.is_empty(), "{id}");
        }
        let agent = DetectedAgent {
            id: "cursor".into(),
            how: How::Home,
        };
        assert_eq!(
            serde_json::to_string(&agent).unwrap(),
            r#"{"id":"cursor","how":"home"}"#
        );
    }
}
