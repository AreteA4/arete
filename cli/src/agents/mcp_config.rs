//! MCP config writers (spec WP7 §6): two servers, `arete` (stdio `a4 mcp`)
//! and `arete-docs` (remote), upserted into each agent's config file.
//! JSON/JSONC files go through the comment-preserving CST, Codex TOML through
//! `toml_edit`, Goose YAML through `serde_yaml`.

use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};

use super::jsonc::JsonDoc;
use super::{display_path, read_optional, upsert_file, Env, ItemResult, Outcome};

pub const DOCS_MCP_URL: &str = "https://docs.arete.run/mcp";
pub const ARETE_SERVER: &str = "arete";
pub const DOCS_SERVER: &str = "arete-docs";
pub const CODEX_TRUST_WARNING: &str =
    "Codex only loads .codex/config.toml for trusted projects: run `codex` in this directory once and accept the trust prompt (or add it under [projects] in ~/.codex/config.toml).";

/// Command used for the `arete` server: the absolute installed binary
/// when a receipt exists (GUI hosts do not inherit shell PATH), else `a4`.
pub fn command_from_receipt() -> String {
    match crate::selfhost::receipt::Receipt::load() {
        Ok(Some(receipt)) if receipt.binary.is_absolute() => {
            receipt.binary.to_string_lossy().into_owned()
        }
        _ => "a4".to_string(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Project,
    Global,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Json,
    Toml,
    Yaml,
}

/// Where an agent's MCP config lives for a scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Placement {
    File {
        path: PathBuf,
        format: Format,
    },
    /// Not applicable for this scope (`global only` / `project only`).
    Skipped(&'static str),
}

fn json(path: PathBuf) -> Placement {
    Placement::File {
        path,
        format: Format::Json,
    }
}

/// Resolve the config file for `id` in `scope`.
pub fn placement(env: &Env, id: &str, scope: Scope) -> Result<Placement> {
    let root = &env.root;
    let home = || env.home();
    Ok(match (id, scope) {
        ("claude-code", Scope::Project) => json(root.join(".mcp.json")),
        ("claude-code", Scope::Global) => json(home()?.join(".claude.json")),
        ("cursor", Scope::Project) => json(root.join(".cursor/mcp.json")),
        ("oh-my-pi", Scope::Project) => json(root.join(".omp/mcp.json")),
        ("oh-my-pi", Scope::Global) => json(home()?.join(".omp/agent/mcp.json")),
        ("cursor", Scope::Global) => json(home()?.join(".cursor/mcp.json")),
        ("vscode", Scope::Project) => json(root.join(".vscode/mcp.json")),
        ("vscode", Scope::Global) => Placement::Skipped("project only"),
        ("copilot-cli", Scope::Project) => json(root.join(".mcp.json")),
        ("copilot-cli", Scope::Global) => json(home()?.join(".copilot/mcp-config.json")),
        ("codex", Scope::Project) => Placement::File {
            path: root.join(".codex/config.toml"),
            format: Format::Toml,
        },
        ("codex", Scope::Global) => Placement::File {
            path: env
                .codex_home()
                .ok_or_else(|| anyhow!("Could not find home directory (set HOME or CODEX_HOME)"))?
                .join("config.toml"),
            format: Format::Toml,
        },
        ("opencode", Scope::Project) => {
            let jsonc = root.join("opencode.jsonc");
            json(if jsonc.exists() {
                jsonc
            } else {
                root.join("opencode.json")
            })
        }
        ("opencode", Scope::Global) => json(
            env.xdg_config_home()
                .ok_or_else(|| {
                    anyhow!("Could not find home directory (set HOME or XDG_CONFIG_HOME)")
                })?
                .join("opencode/opencode.json"),
        ),
        ("gemini-cli", Scope::Project) => json(root.join(".gemini/settings.json")),
        ("gemini-cli", Scope::Global) => json(home()?.join(".gemini/settings.json")),
        ("windsurf", Scope::Project) => Placement::Skipped("global only"),
        ("windsurf", Scope::Global) => json(home()?.join(".codeium/windsurf/mcp_config.json")),
        ("cline", Scope::Project) => Placement::Skipped("global only"),
        ("cline", Scope::Global) => json(home()?.join(".cline/mcp.json")),
        ("zed", Scope::Project) => json(root.join(".zed/settings.json")),
        ("zed", Scope::Global) => json(
            env.xdg_config_home()
                .ok_or_else(|| anyhow!("Could not find home directory"))?
                .join("zed/settings.json"),
        ),
        ("amp", Scope::Project) => json(root.join(".amp/settings.json")),
        ("amp", Scope::Global) => json(
            env.xdg_config_home()
                .ok_or_else(|| anyhow!("Could not find home directory"))?
                .join("amp/settings.json"),
        ),
        ("kiro", Scope::Project) => json(root.join(".kiro/settings/mcp.json")),
        ("kiro", Scope::Global) => json(home()?.join(".kiro/settings/mcp.json")),
        ("roo", Scope::Project) => json(root.join(".roo/mcp.json")),
        ("roo", Scope::Global) => Placement::Skipped("project only"),
        ("goose", Scope::Project) => Placement::File {
            path: root.join(".goose/config.yaml"),
            format: Format::Yaml,
        },
        ("goose", Scope::Global) => Placement::File {
            path: env
                .xdg_config_home()
                .ok_or_else(|| anyhow!("Could not find home directory"))?
                .join("goose/config.yaml"),
            format: Format::Yaml,
        },
        _ => anyhow::bail!("Unknown agent id: {id}"),
    })
}

/// Desired entries for one agent: top-level key, `arete`, `arete-docs`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shape {
    pub top_key: &'static str,
    pub arete: Value,
    pub docs: Value,
}

fn plain(command: &str) -> Value {
    json!({"command": command, "args": ["mcp"]})
}

/// Shape for `id`. `copilot_owned` selects the Copilot-CLI shape (with
/// `tools`) for the shared `.mcp.json`; when the file already existed for
/// Claude Code the Claude shape is used instead.
pub fn shape(id: &str, command: &str, copilot_owned: bool) -> Shape {
    let url = DOCS_MCP_URL;
    let (top_key, arete, docs) = match id {
        "claude-code" | "oh-my-pi" => (
            "mcpServers",
            json!({"type": "stdio", "command": command, "args": ["mcp"]}),
            json!({"type": "http", "url": url}),
        ),
        "copilot-cli" if copilot_owned => (
            "mcpServers",
            json!({"type": "local", "command": command, "args": ["mcp"], "tools": ["*"]}),
            json!({"type": "http", "url": url, "tools": ["*"]}),
        ),
        "copilot-cli" => (
            "mcpServers",
            json!({"type": "stdio", "command": command, "args": ["mcp"]}),
            json!({"type": "http", "url": url}),
        ),
        "cursor" | "kiro" => ("mcpServers", plain(command), json!({"url": url})),
        "vscode" => (
            "servers",
            json!({"type": "stdio", "command": command, "args": ["mcp"]}),
            json!({"type": "http", "url": url}),
        ),
        "codex" => ("mcp_servers", plain(command), json!({"url": url})),
        "opencode" => (
            "mcp",
            json!({"type": "local", "command": [command, "mcp"], "enabled": true}),
            json!({"type": "remote", "url": url, "enabled": true}),
        ),
        "gemini-cli" => ("mcpServers", plain(command), json!({"httpUrl": url})),
        "windsurf" => ("mcpServers", plain(command), json!({"serverUrl": url})),
        "cline" => (
            "mcpServers",
            plain(command),
            json!({"type": "streamableHttp", "url": url}),
        ),
        "zed" => ("context_servers", plain(command), json!({"url": url})),
        "amp" => ("amp.mcpServers", plain(command), json!({"url": url})),
        "roo" => (
            "mcpServers",
            plain(command),
            json!({"type": "streamable-http", "url": url}),
        ),
        "goose" => (
            "extensions",
            json!({"type": "stdio", "cmd": command, "args": ["mcp"], "enabled": true}),
            json!({"type": "streamable_http", "uri": url, "enabled": true}),
        ),
        _ => ("mcpServers", plain(command), json!({"url": url})),
    };
    Shape {
        top_key,
        arete,
        docs,
    }
}

/// Current `arete` / `arete-docs` entries in a config file's content.
fn current_entries(
    format: Format,
    content: &str,
    top_key: &str,
) -> Result<(Option<Value>, Option<Value>)> {
    Ok(match format {
        Format::Json => {
            let doc = JsonDoc::parse(content)?;
            (
                doc.get(&[top_key, ARETE_SERVER]),
                doc.get(&[top_key, DOCS_SERVER]),
            )
        }
        Format::Toml => {
            let table: toml::Value = toml::from_str(content).context("invalid TOML")?;
            let servers = table.get(top_key);
            let get = |name: &str| -> Option<Value> {
                servers
                    .and_then(|servers| servers.get(name))
                    .and_then(|entry| serde_json::to_value(entry).ok())
            };
            (get(ARETE_SERVER), get(DOCS_SERVER))
        }
        Format::Yaml => {
            let root: serde_yaml::Value = if content.trim().is_empty() {
                serde_yaml::Value::Null
            } else {
                serde_yaml::from_str(content).context("invalid YAML")?
            };
            let servers = root.get(top_key);
            let get = |name: &str| -> Option<Value> {
                servers
                    .and_then(|servers| servers.get(name))
                    .and_then(|entry| serde_json::to_value(entry).ok())
            };
            (get(ARETE_SERVER), get(DOCS_SERVER))
        }
    })
}

/// New file content with both entries set (everything else preserved).
fn render_with_entries(
    format: Format,
    existing: Option<&str>,
    shape: &Shape,
    opencode_schema: bool,
) -> Result<String> {
    render_entries(
        format,
        existing,
        shape.top_key,
        &[
            (ARETE_SERVER.to_string(), shape.arete.clone()),
            (DOCS_SERVER.to_string(), shape.docs.clone()),
        ]
        .into(),
        &[],
        opencode_schema,
    )
}

/// Shared renderer for public init and neutral workspace configuration. Only
/// named entries change; JSONC/TOML comments and unrelated fields survive.
pub fn render_entries(
    format: Format,
    existing: Option<&str>,
    top_key: &str,
    entries: &std::collections::BTreeMap<String, Value>,
    remove: &[String],
    opencode_schema: bool,
) -> Result<String> {
    match format {
        Format::Json => {
            let doc = JsonDoc::parse(existing.unwrap_or(""))?;
            if !doc.root_is_object() {
                anyhow::bail!("root value is not an object");
            }
            if opencode_schema && existing.is_none() {
                doc.set(&["$schema"], &json!("https://opencode.ai/config.json"));
            }
            for name in remove {
                doc.remove(&[top_key, name]);
            }
            for (name, value) in entries {
                doc.set(&[top_key, name], value);
            }
            Ok(doc.render())
        }
        Format::Toml => {
            let mut doc: toml_edit::DocumentMut =
                existing.unwrap_or("").parse().context("invalid TOML")?;
            let servers = doc
                .entry(top_key)
                .or_insert(toml_edit::Item::Table(toml_edit::Table::new()));
            let servers_table = servers
                .as_table_mut()
                .ok_or_else(|| anyhow!("`{}` is not a table", top_key))?;
            servers_table.set_implicit(true);
            for name in remove {
                servers_table.remove(name);
            }
            for (name, value) in entries {
                // Replace the selected entry as a whole: stale managed fields must disappear.
                servers_table.remove(name);
                let entry = servers_table
                    .entry(name)
                    .or_insert(toml_edit::Item::Table(toml_edit::Table::new()));
                let entry_table = entry
                    .as_table_mut()
                    .ok_or_else(|| anyhow!("`{}.{name}` is not a table", top_key))?;
                let Value::Object(fields) = value else {
                    unreachable!("server shapes are objects");
                };
                for (key, field) in fields {
                    entry_table.insert(key, toml_edit::Item::Value(json_to_toml_value(field)?));
                }
            }
            Ok(doc.to_string())
        }
        Format::Yaml => {
            let mut root: serde_yaml::Value = match existing {
                Some(text) if !text.trim().is_empty() => {
                    serde_yaml::from_str(text).context("invalid YAML")?
                }
                _ => serde_yaml::Value::Mapping(Default::default()),
            };
            let serde_yaml::Value::Mapping(map) = &mut root else {
                anyhow::bail!("root value is not a mapping");
            };
            let key = serde_yaml::Value::String(top_key.to_string());
            let servers = map
                .entry(key)
                .or_insert_with(|| serde_yaml::Value::Mapping(Default::default()));
            let serde_yaml::Value::Mapping(servers) = servers else {
                anyhow::bail!("`{}` is not a mapping", top_key);
            };
            for name in remove {
                servers.remove(serde_yaml::Value::String(name.clone()));
            }
            for (name, value) in entries {
                servers.insert(
                    serde_yaml::Value::String(name.to_string()),
                    serde_yaml::to_value(value)?,
                );
            }
            Ok(serde_yaml::to_string(&root)?)
        }
    }
}

fn json_to_toml_value(value: &Value) -> Result<toml_edit::Value> {
    Ok(match value {
        Value::String(s) => toml_edit::Value::from(s.as_str()),
        Value::Bool(b) => toml_edit::Value::from(*b),
        Value::Number(n) if n.is_i64() => toml_edit::Value::from(n.as_i64().unwrap_or_default()),
        Value::Number(n) => toml_edit::Value::from(n.as_f64().unwrap_or_default()),
        Value::Array(items) => {
            let mut array = toml_edit::Array::new();
            for item in items {
                array.push(json_to_toml_value(item)?);
            }
            toml_edit::Value::Array(array)
        }
        Value::Object(map) => {
            let mut table = toml_edit::InlineTable::new();
            for (key, item) in map {
                table.insert(key, json_to_toml_value(item)?);
            }
            toml_edit::Value::InlineTable(table)
        }
        Value::Null => anyhow::bail!("null is not representable in TOML"),
    })
}

/// State of an agent's MCP config, for `a4 doctor`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpState {
    Ok,
    /// File missing or entries absent/different.
    Missing(String),
    Skipped(&'static str),
    Error(String),
}

/// Whether `existing` equals a desired entry; the shared `.mcp.json` accepts
/// either the Claude Code or the Copilot CLI shape.
fn entry_matches(existing: Option<&Value>, candidates: &[&Value]) -> bool {
    existing.is_some_and(|value| candidates.contains(&value))
}

fn acceptable_shapes(id: &str, command: &str, scope: Scope, file_exists: bool) -> Vec<Shape> {
    if id == "copilot-cli" && scope == Scope::Project {
        vec![
            shape(id, command, false),
            shape(id, command, true),
            shape("claude-code", command, false),
        ]
    } else if id == "copilot-cli" {
        vec![shape(id, command, true)]
    } else if id == "claude-code" && scope == Scope::Project && file_exists {
        vec![
            shape(id, command, false),
            shape("copilot-cli", command, true),
        ]
    } else {
        vec![shape(id, command, false)]
    }
}

/// Check an agent's MCP config against the desired shape.
pub fn check(env: &Env, id: &str, scope: Scope, command: &str) -> McpState {
    let (path, format) = match placement(env, id, scope) {
        Ok(Placement::File { path, format }) => (path, format),
        Ok(Placement::Skipped(reason)) => return McpState::Skipped(reason),
        Err(error) => return McpState::Error(format!("{error:#}")),
    };
    let shown = display_path(env, &path);
    let content = match read_optional(&path) {
        Ok(Some(content)) => content,
        Ok(None) => return McpState::Missing(format!("{shown} missing")),
        Err(error) => return McpState::Error(format!("{error:#}")),
    };
    let shapes = acceptable_shapes(id, command, scope, true);
    let top_key = shapes[0].top_key;
    match current_entries(format, &content, top_key) {
        Ok((arete, docs)) => {
            let arete_ok = entry_matches(
                arete.as_ref(),
                &shapes.iter().map(|s| &s.arete).collect::<Vec<_>>(),
            );
            let docs_ok = entry_matches(
                docs.as_ref(),
                &shapes.iter().map(|s| &s.docs).collect::<Vec<_>>(),
            );
            match (arete_ok, docs_ok) {
                (true, true) => McpState::Ok,
                (false, true) => McpState::Missing(format!(
                    "{shown}: `{ARETE_SERVER}` server missing or different"
                )),
                (true, false) => McpState::Missing(format!(
                    "{shown}: `{DOCS_SERVER}` server missing or different"
                )),
                (false, false) => McpState::Missing(format!(
                    "{shown}: `{ARETE_SERVER}` and `{DOCS_SERVER}` servers missing"
                )),
            }
        }
        Err(error) => McpState::Error(format!("{shown}: {error:#}")),
    }
}

/// Writer: upsert both servers for `id`. Returns the item result and an
/// optional warning (Codex trust).
pub fn write(
    env: &Env,
    id: &str,
    scope: Scope,
    command: &str,
    dry_run: bool,
) -> (ItemResult, Option<String>) {
    let item = format!("mcp:{id}");
    let (path, format) = match placement(env, id, scope) {
        Ok(Placement::File { path, format }) => (path, format),
        Ok(Placement::Skipped(reason)) => {
            let fix = match reason {
                "global only" => Some(format!(
                    "a4 init --global --agents {id} --no-manifest --no-agents-md --no-skills"
                )),
                _ => Some(format!(
                    "a4 init --agents {id} --no-manifest --no-agents-md --no-skills"
                )),
            };
            return (
                ItemResult::new(item, Outcome::skipped(reason, fix), None),
                None,
            );
        }
        Err(error) => {
            return (
                ItemResult::new(item, Outcome::error(format!("{error:#}"), None), None),
                None,
            )
        }
    };
    let shown = display_path(env, &path);
    let existing = match read_optional(&path) {
        Ok(existing) => existing,
        Err(error) => {
            return (
                ItemResult::new(
                    item,
                    Outcome::error(format!("{error:#}"), None),
                    Some(shown),
                ),
                None,
            )
        }
    };
    let file_exists = existing.is_some();
    // Already correct in any acceptable shape: leave the file alone.
    if file_exists && check(env, id, scope, command) == McpState::Ok {
        return (ItemResult::new(item, Outcome::Unchanged, Some(shown)), None);
    }
    let copilot_owned = id == "copilot-cli" && (scope == Scope::Global || !file_exists);
    let desired = shape(id, command, copilot_owned);
    let content = match render_with_entries(format, existing.as_deref(), &desired, id == "opencode")
    {
        Ok(content) => content,
        Err(error) => {
            let fix = match id {
                "opencode" => Some(format!(
                    "add manually to {shown}: \"mcp\": {{ \"arete\": {}, \"arete-docs\": {} }}",
                    desired.arete, desired.docs
                )),
                _ => Some(format!("fix {shown} by hand, then re-run a4 init")),
            };
            return (
                ItemResult::new(
                    item,
                    Outcome::error(format!("{shown}: {error:#}"), fix),
                    Some(shown),
                ),
                None,
            );
        }
    };
    let outcome = upsert_file(&path, &content, dry_run);
    let warning = match (id, scope, &outcome) {
        ("codex", Scope::Project, Outcome::Created | Outcome::Updated) => {
            Some(CODEX_TRUST_WARNING.to_string())
        }
        _ => None,
    };
    (ItemResult::new(item, outcome, Some(shown)), warning)
}

/// Whether the project is trusted in the user's Codex config
/// (`[projects."<root>"] trust_level = "trusted"`). `None` = no user config.
pub fn codex_project_trusted(env: &Env) -> Option<bool> {
    let path = env.codex_home()?.join("config.toml");
    let content = read_optional(&path).ok()??;
    let table: toml::Value = toml::from_str(&content).ok()?;
    let Some(projects) = table.get("projects").and_then(|p| p.as_table()) else {
        return Some(false);
    };
    let root = std::fs::canonicalize(&env.root).unwrap_or_else(|_| env.root.clone());
    let root_text = root.to_string_lossy();
    let trusted = projects.iter().any(|(key, value)| {
        let same = key == root_text.as_ref()
            || std::fs::canonicalize(key)
                .map(|k| k == root)
                .unwrap_or(false);
        same && value.get("trust_level").and_then(|v| v.as_str()) == Some("trusted")
    });
    Some(trusted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn env(dir: &std::path::Path) -> Env {
        let root = dir.join("proj");
        let home = dir.join("home");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&home).unwrap();
        Env::new(&root, Some(home), &[])
    }

    #[test]
    fn placements_follow_the_table() {
        let dir = tempfile::tempdir().unwrap();
        let env = env(dir.path());
        let root = &env.root;
        let home = env.home.clone().unwrap();
        let file = |p: Placement| match p {
            Placement::File { path, .. } => path,
            Placement::Skipped(r) => panic!("skipped: {r}"),
        };
        assert_eq!(
            file(placement(&env, "claude-code", Scope::Project).unwrap()),
            root.join(".mcp.json")
        );
        assert_eq!(
            file(placement(&env, "claude-code", Scope::Global).unwrap()),
            home.join(".claude.json")
        );
        assert_eq!(
            file(placement(&env, "opencode", Scope::Project).unwrap()),
            root.join("opencode.json")
        );
        fs::write(root.join("opencode.jsonc"), "{}").unwrap();
        assert_eq!(
            file(placement(&env, "opencode", Scope::Project).unwrap()),
            root.join("opencode.jsonc")
        );
        assert_eq!(
            file(placement(&env, "opencode", Scope::Global).unwrap()),
            home.join(".config/opencode/opencode.json")
        );
        assert_eq!(
            file(placement(&env, "codex", Scope::Global).unwrap()),
            home.join(".codex/config.toml")
        );
        assert_eq!(
            placement(&env, "windsurf", Scope::Project).unwrap(),
            Placement::Skipped("global only")
        );
        assert_eq!(
            placement(&env, "cline", Scope::Project).unwrap(),
            Placement::Skipped("global only")
        );
        assert_eq!(
            placement(&env, "vscode", Scope::Global).unwrap(),
            Placement::Skipped("project only")
        );
        assert_eq!(
            placement(&env, "roo", Scope::Global).unwrap(),
            Placement::Skipped("project only")
        );
        assert_eq!(
            file(placement(&env, "goose", Scope::Global).unwrap()),
            home.join(".config/goose/config.yaml")
        );
        for id in super::super::AGENT_IDS {
            placement(&env, id, Scope::Project).unwrap();
            placement(&env, id, Scope::Global).unwrap();
        }
    }

    #[test]
    fn json_writer_preserves_other_servers_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let env = env(dir.path());
        let path = env.root.join(".mcp.json");
        fs::write(
            &path,
            "{\n  \"mcpServers\": {\n    \"other\": { \"command\": \"x\" }\n  }\n}\n",
        )
        .unwrap();
        let (result, warning) = write(&env, "claude-code", Scope::Project, "/opt/a4", false);
        assert_eq!(result.outcome, Outcome::Updated);
        assert_eq!(result.path.as_deref(), Some(".mcp.json"));
        assert!(warning.is_none());
        let parsed: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(parsed["mcpServers"]["other"]["command"], "x");
        assert_eq!(
            parsed["mcpServers"]["arete"],
            json!({"type": "stdio", "command": "/opt/a4", "args": ["mcp"]})
        );
        assert_eq!(
            parsed["mcpServers"]["arete-docs"],
            json!({"type": "http", "url": DOCS_MCP_URL})
        );
        assert_eq!(
            check(&env, "claude-code", Scope::Project, "/opt/a4"),
            McpState::Ok
        );
        let before = fs::read_to_string(&path).unwrap();
        assert_eq!(
            write(&env, "claude-code", Scope::Project, "/opt/a4", false)
                .0
                .outcome,
            Outcome::Unchanged
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), before);
        // Different command => stale.
        assert!(matches!(
            check(&env, "claude-code", Scope::Project, "a4"),
            McpState::Missing(_)
        ));
        // Copilot shares the file and accepts the Claude shape.
        assert_eq!(
            check(&env, "copilot-cli", Scope::Project, "/opt/a4"),
            McpState::Ok
        );
        assert_eq!(
            write(&env, "copilot-cli", Scope::Project, "/opt/a4", false)
                .0
                .outcome,
            Outcome::Unchanged
        );
    }

    #[test]
    fn copilot_creates_its_own_shape_when_file_is_new() {
        let dir = tempfile::tempdir().unwrap();
        let env = env(dir.path());
        let (result, _) = write(&env, "copilot-cli", Scope::Project, "a4", false);
        assert_eq!(result.outcome, Outcome::Created);
        let parsed: Value =
            serde_json::from_str(&fs::read_to_string(env.root.join(".mcp.json")).unwrap()).unwrap();
        assert_eq!(parsed["mcpServers"]["arete"]["tools"], json!(["*"]));
        assert_eq!(parsed["mcpServers"]["arete"]["type"], "local");
        // Claude Code then accepts the Copilot-owned file as-is.
        assert_eq!(
            check(&env, "claude-code", Scope::Project, "a4"),
            McpState::Ok
        );
    }

    #[test]
    fn dry_run_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let env = env(dir.path());
        let (result, _) = write(&env, "cursor", Scope::Project, "a4", true);
        assert_eq!(result.outcome, Outcome::Created);
        assert!(!env.root.join(".cursor/mcp.json").exists());
        assert!(matches!(
            check(&env, "cursor", Scope::Project, "a4"),
            McpState::Missing(_)
        ));
    }

    #[test]
    fn opencode_jsonc_keeps_comments_and_adds_schema_on_create() {
        let dir = tempfile::tempdir().unwrap();
        let env = env(dir.path());
        let (result, _) = write(&env, "opencode", Scope::Project, "a4", false);
        assert_eq!(result.outcome, Outcome::Created);
        let created = fs::read_to_string(env.root.join("opencode.json")).unwrap();
        let parsed: Value = serde_json::from_str(&created).unwrap();
        assert_eq!(parsed["$schema"], "https://opencode.ai/config.json");
        assert_eq!(
            parsed["mcp"]["arete"],
            json!({"type": "local", "command": ["a4", "mcp"], "enabled": true})
        );
        assert_eq!(
            parsed["mcp"]["arete-docs"],
            json!({"type": "remote", "url": DOCS_MCP_URL, "enabled": true})
        );
        fs::remove_file(env.root.join("opencode.json")).unwrap();

        let jsonc = env.root.join("opencode.jsonc");
        fs::write(&jsonc, "{\n  // my config\n  \"theme\": \"dark\",\n  \"mcp\": {\n    \"mine\": { \"type\": \"local\", \"command\": [\"me\"] }, // keep\n  },\n}\n").unwrap();
        let (result, _) = write(&env, "opencode", Scope::Project, "a4", false);
        assert_eq!(result.outcome, Outcome::Updated, "{:?}", result.outcome);
        assert_eq!(result.path.as_deref(), Some("opencode.jsonc"));
        let text = fs::read_to_string(&jsonc).unwrap();
        assert!(text.contains("// my config"));
        assert!(text.contains("// keep"));
        assert!(text.contains("\"mine\""));
        assert!(!text.contains("$schema"), "schema only on create");
        assert_eq!(check(&env, "opencode", Scope::Project, "a4"), McpState::Ok);
        assert_eq!(
            write(&env, "opencode", Scope::Project, "a4", false)
                .0
                .outcome,
            Outcome::Unchanged
        );
    }

    #[test]
    fn codex_toml_preserves_other_tables_and_warns() {
        let dir = tempfile::tempdir().unwrap();
        let env = env(dir.path());
        let path = env.root.join(".codex/config.toml");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "# top comment\nmodel = \"o3\"\n\n[sandbox]\nmode = \"workspace-write\"\n\n[mcp_servers.other]\ncommand = \"x\"\n").unwrap();
        let (result, warning) = write(&env, "codex", Scope::Project, "a4", false);
        assert_eq!(result.outcome, Outcome::Updated);
        assert_eq!(warning.as_deref(), Some(CODEX_TRUST_WARNING));
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("# top comment"));
        assert!(text.contains("model = \"o3\""));
        assert!(text.contains("[sandbox]"));
        assert!(text.contains("[mcp_servers.other]"));
        assert!(text.contains("[mcp_servers.arete]"));
        assert!(text.contains("[mcp_servers.arete-docs]"));
        assert!(
            !text.contains("[mcp_servers]\n"),
            "no empty header:\n{text}"
        );
        let parsed: toml::Value = toml::from_str(&text).unwrap();
        assert_eq!(
            parsed["mcp_servers"]["arete"]["command"].as_str(),
            Some("a4")
        );
        assert_eq!(
            parsed["mcp_servers"]["arete-docs"]["url"].as_str(),
            Some(DOCS_MCP_URL)
        );
        assert_eq!(check(&env, "codex", Scope::Project, "a4"), McpState::Ok);
        let (result, warning) = write(&env, "codex", Scope::Project, "a4", false);
        assert_eq!(result.outcome, Outcome::Unchanged);
        assert!(warning.is_none());

        // Fresh file.
        fs::remove_file(&path).unwrap();
        assert_eq!(
            write(&env, "codex", Scope::Project, "a4", false).0.outcome,
            Outcome::Created
        );
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("[mcp_servers.arete]\n"), "{text}");
    }

    #[test]
    fn goose_yaml_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let env = env(dir.path());
        let path = env.root.join(".goose/config.yaml");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            "GOOSE_MODEL: gpt\nextensions:\n  developer:\n    enabled: true\n    type: builtin\n",
        )
        .unwrap();
        assert_eq!(
            write(&env, "goose", Scope::Project, "a4", false).0.outcome,
            Outcome::Updated
        );
        let text = fs::read_to_string(&path).unwrap();
        let parsed: serde_yaml::Value = serde_yaml::from_str(&text).unwrap();
        assert_eq!(parsed["GOOSE_MODEL"].as_str(), Some("gpt"));
        assert_eq!(
            parsed["extensions"]["developer"]["type"].as_str(),
            Some("builtin")
        );
        assert_eq!(parsed["extensions"]["arete"]["cmd"].as_str(), Some("a4"));
        assert_eq!(
            parsed["extensions"]["arete-docs"]["uri"].as_str(),
            Some(DOCS_MCP_URL)
        );
        assert_eq!(check(&env, "goose", Scope::Project, "a4"), McpState::Ok);
        assert_eq!(
            write(&env, "goose", Scope::Project, "a4", false).0.outcome,
            Outcome::Unchanged
        );
    }

    #[test]
    fn every_agent_writes_then_checks_ok_in_its_supported_scope() {
        let dir = tempfile::tempdir().unwrap();
        let env = env(dir.path());
        for id in super::super::AGENT_IDS {
            for scope in [Scope::Project, Scope::Global] {
                let (result, _) = write(&env, id, scope, "a4", false);
                match result.outcome {
                    // copilot-cli shares .mcp.json with claude-code (written just before).
                    Outcome::Created | Outcome::Unchanged => {
                        assert_eq!(check(&env, id, scope, "a4"), McpState::Ok, "{id} {scope:?}")
                    }
                    Outcome::Skipped { .. } => {
                        assert!(matches!(check(&env, id, scope, "a4"), McpState::Skipped(_)))
                    }
                    other => panic!("{id} {scope:?}: {other:?}"),
                }
            }
        }
    }

    #[test]
    fn codex_trust_reads_projects_table() {
        let dir = tempfile::tempdir().unwrap();
        let env = env(dir.path());
        assert_eq!(codex_project_trusted(&env), None);
        let config = env.home.clone().unwrap().join(".codex/config.toml");
        fs::create_dir_all(config.parent().unwrap()).unwrap();
        fs::write(&config, "model = \"x\"\n").unwrap();
        assert_eq!(codex_project_trusted(&env), Some(false));
        let root = fs::canonicalize(&env.root).unwrap();
        fs::write(
            &config,
            format!(
                "[projects.\"{}\"]\ntrust_level = \"trusted\"\n",
                root.display()
            ),
        )
        .unwrap();
        assert_eq!(codex_project_trusted(&env), Some(true));
    }
}
