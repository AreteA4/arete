//! End-to-end tests for `a4 init` and `a4 doctor` against the built binary.
//!
//! Every test runs in a fresh temp dir with an isolated `HOME`, `ARETE_HOME`
//! and `PATH` (optionally containing a fake `npx`), a local HTTP stub for
//! the API / docs checks and an unreachable `A4_LATEST_URL`. Fixtures under
//! `tests/fixtures/init/` are copied, never mutated.

use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;
use tempfile::TempDir;

const A4: &str = env!("CARGO_BIN_EXE_a4");

struct Sandbox {
    _dir: TempDir,
    root: PathBuf,
    home: PathBuf,
    bin: PathBuf,
    server: String,
}

/// Minimal HTTP stub: 200 for every GET/HEAD.
fn spawn_stub_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            let mut buffer = [0u8; 4096];
            let read = stream.read(&mut buffer).unwrap_or(0);
            let request = String::from_utf8_lossy(&buffer[..read]);
            let head = request.starts_with("HEAD");
            let body = "[]";
            let response = if head {
                "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string()
            } else {
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
            };
            let _ = stream.write_all(response.as_bytes());
        }
    });
    format!("http://{address}")
}

#[cfg(unix)]
fn install_fake_npx(bin: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let script = r#"#!/bin/sh
# fake npx: record args, install fake skills for the universal + claude dirs, write the lock
# (PATH holds only this directory, so use absolute tool paths)
printf '%s\n' "$@" > npx-args.txt
for agent in .agents .claude; do
  for skill in arete arete-streams arete-programs arete-stack-authoring arete-deploy; do
    /bin/mkdir -p "$agent/skills/$skill"
    printf '# %s\n' "$skill" > "$agent/skills/$skill/SKILL.md"
  done
done
printf '{"skills":{"arete":{}}}\n' > skills-lock.json
"#;
    let path = bin.join("npx");
    fs::write(&path, script).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else if entry.file_name() != ".gitkeep" {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn sandbox(fixture: &str, with_npx: bool) -> Sandbox {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("proj");
    let home = dir.path().join("home");
    let bin = dir.path().join("bin");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&bin).unwrap();
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/init");
    copy_dir(&fixtures.join(fixture), &root);
    if with_npx {
        #[cfg(unix)]
        install_fake_npx(&bin);
    }
    Sandbox {
        _dir: dir,
        root,
        home,
        bin,
        server: spawn_stub_server(),
    }
}

fn a4(sb: &Sandbox, args: &[&str]) -> Output {
    let mut command = Command::new(A4);
    command
        .args(args)
        .current_dir(&sb.root)
        .env_clear()
        .env("HOME", &sb.home)
        .env("PATH", &sb.bin)
        .env("ARETE_HOME", sb.home.join(".arete"))
        .env(
            "ARETE_CREDENTIALS_PATH",
            sb.home.join(".arete/credentials.toml"),
        )
        .env("ARETE_API_URL", &sb.server)
        .env("A4_DOCS_MCP_URL", format!("{}/mcp", sb.server))
        .env("A4_LATEST_URL", "http://127.0.0.1:1/latest.json")
        .env("DO_NOT_TRACK", "1");
    command.output().unwrap()
}

fn a4_json(sb: &Sandbox, args: &[&str]) -> (Value, Output) {
    let output = a4(sb, args);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let value: Value = serde_json::from_str(&stdout).unwrap_or_else(|error| {
        panic!(
            "stdout is not exactly one JSON object ({error}):\n--- stdout ---\n{stdout}\n--- stderr ---\n{}",
            String::from_utf8_lossy(&output.stderr)
        )
    });
    (value, output)
}

fn results(report: &Value) -> BTreeMap<String, Value> {
    report["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| (r["item"].as_str().unwrap().to_string(), r.clone()))
        .collect()
}

fn checks(report: &Value) -> BTreeMap<String, Value> {
    report["checks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| (c["id"].as_str().unwrap().to_string(), c.clone()))
        .collect()
}

fn snapshot(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    fn walk(base: &Path, dir: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if entry.file_type().unwrap().is_dir() {
                walk(base, &path, files);
            } else {
                files.insert(
                    path.strip_prefix(base).unwrap().to_path_buf(),
                    fs::read(&path).unwrap(),
                );
            }
        }
    }
    walk(dir, dir, &mut files);
    files
}

fn read(sb: &Sandbox, relative: &str) -> String {
    fs::read_to_string(sb.root.join(relative)).unwrap_or_else(|e| panic!("{relative}: {e}"))
}

#[test]
fn init_in_empty_dir_without_node_then_doctor_warns_on_skills() {
    let sb = sandbox("empty", false);
    let (report, output) = a4_json(&sb, &["init", "-y", "--json"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(report["schemaVersion"], 1);
    assert_eq!(report["dryRun"], false);
    assert_eq!(report["detectedAgents"], serde_json::json!([]));
    assert_eq!(report["selectedAgents"], serde_json::json!([]));
    let items = results(&report);
    assert_eq!(items["arete.toml"]["status"], "created");
    assert_eq!(items["agents-md"]["status"], "created");
    assert_eq!(items["agents-md"]["path"], "AGENTS.md");
    assert_eq!(items["claude-md"]["status"], "created");
    assert_eq!(items["skills"]["status"], "skipped");
    assert_eq!(items["skills"]["reason"], "npx not found");
    assert!(items["skills"]["fix"]
        .as_str()
        .unwrap()
        .starts_with("npx skills add AreteA4/skills"));
    assert_eq!(items["mcp:claude-code"]["status"], "created");
    assert_eq!(items["mcp:claude-code"]["path"], ".mcp.json");
    assert_eq!(
        report["warnings"],
        serde_json::json!(["No coding agent detected; wrote agent-independent files only."])
    );
    assert_eq!(
        report["next"],
        serde_json::json!(["a4 doctor --json", "a4 explore --json"])
    );
    assert_eq!(read(&sb, "CLAUDE.md"), "@AGENTS.md\n");
    assert!(read(&sb, "AGENTS.md").starts_with("<!-- BEGIN:arete v2 -->\n## Arete\n"));
    assert!(read(&sb, "arete.toml").contains("name = \"proj\""));
    let mcp: Value = serde_json::from_str(&read(&sb, ".mcp.json")).unwrap();
    assert_eq!(mcp["mcpServers"]["arete"]["command"], "a4");
    assert_eq!(
        mcp["mcpServers"]["arete"]["args"],
        serde_json::json!(["mcp"])
    );
    assert_eq!(
        mcp["mcpServers"]["arete-docs"]["url"],
        "https://docs.arete.run/mcp"
    );

    // Second run: byte-identical tree, everything unchanged, claude-code now detected from the files.
    let before = snapshot(&sb.root);
    let (report, output) = a4_json(&sb, &["init", "-y", "--json"]);
    assert!(output.status.success());
    assert_eq!(report["selectedAgents"], serde_json::json!(["claude-code"]));
    assert_eq!(report["detectedAgents"][0]["how"], "project");
    for (item, result) in results(&report) {
        let expected = if item == "skills" {
            "skipped"
        } else {
            "unchanged"
        };
        assert_eq!(result["status"], expected, "{item}: {result}");
    }
    assert!(report["warnings"].as_array().unwrap().is_empty());
    assert_eq!(snapshot(&sb.root), before);

    // Doctor: warn (skills missing), everything else ok/info.
    let (doctor, output) = a4_json(&sb, &["doctor", "--json"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(doctor["schemaVersion"], 1);
    assert_eq!(doctor["status"], "warn", "{doctor}");
    let c = checks(&doctor);
    assert_eq!(c["cli.version"]["status"], "info");
    assert_eq!(c["cli.install"]["status"], "info");
    assert_eq!(c["project.manifest"]["status"], "ok");
    assert_eq!(c["project.lock"]["status"], "ok");
    assert_eq!(c["auth.credentials"]["status"], "info");
    assert_eq!(c["auth.whoami"]["status"], "info");
    assert_eq!(c["net.api"]["status"], "ok", "{}", c["net.api"]);
    assert_eq!(c["net.docs-mcp"]["status"], "ok", "{}", c["net.docs-mcp"]);
    assert_eq!(c["tools.node"]["status"], "info");
    assert_eq!(c["agents.detected"]["status"], "info");
    assert_eq!(c["agents.claude-code.mcp"]["status"], "ok");
    assert_eq!(c["agents.claude-code.skills"]["status"], "warn");
    assert_eq!(
        c["agents.claude-code.skills"]["fix"],
        "npx skills add AreteA4/skills --agent claude-code"
    );
    assert_eq!(c["agents.agents-md"]["status"], "ok");
    assert_eq!(c["agents.claude-md"]["status"], "ok");
    assert!(c["cli.version"]["fix"].is_null());
    let warn_ids: Vec<&String> = c
        .iter()
        .filter(|(_, v)| v["status"] == "warn")
        .map(|(k, _)| k)
        .collect();
    assert_eq!(warn_ids, vec!["agents.claude-code.skills"], "{doctor}");
}

#[cfg(unix)]
#[test]
fn init_with_node_and_receipt_makes_doctor_ok() {
    let sb = sandbox("empty", true);
    // Pretend `a4 self install` ran: receipt pointing at this binary, install dir on PATH.
    let exe = PathBuf::from(A4);
    let install_dir = exe.parent().unwrap();
    fs::create_dir_all(sb.home.join(".arete")).unwrap();
    fs::write(
        sb.home.join(".arete/receipt.json"),
        serde_json::json!({
            "schemaVersion": 1, "version": env!("CARGO_PKG_VERSION"),
            "binary": exe, "installDir": install_dir, "platform": "test", "source": "manual",
            "verified": false, "modifyPath": false, "installedAt": "2026-09-03T00:00:00Z"
        })
        .to_string(),
    )
    .unwrap();
    let path = std::env::join_paths([sb.bin.as_path(), install_dir]).unwrap();
    let run = |args: &[&str]| {
        let mut command = Command::new(A4);
        command
            .args(args)
            .current_dir(&sb.root)
            .env_clear()
            .env("HOME", &sb.home)
            .env("PATH", &path)
            .env("ARETE_HOME", sb.home.join(".arete"))
            .env(
                "ARETE_CREDENTIALS_PATH",
                sb.home.join(".arete/credentials.toml"),
            )
            .env("ARETE_API_URL", &sb.server)
            .env("A4_DOCS_MCP_URL", format!("{}/mcp", sb.server))
            .env("A4_LATEST_URL", "http://127.0.0.1:1/latest.json")
            .env("DO_NOT_TRACK", "1");
        let output = command.output().unwrap();
        let value: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
            panic!(
                "not JSON: {}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
        });
        (value, output)
    };

    let (report, output) = run(&["init", "-y", "--json"]);
    assert!(output.status.success());
    let items = results(&report);
    assert_eq!(items["skills"]["status"], "created", "{}", items["skills"]);
    assert_eq!(items["skills"]["path"], "skills-lock.json");
    let args = read(&sb, "npx-args.txt");
    assert_eq!(
        args.lines().collect::<Vec<_>>(),
        vec![
            "-y",
            "skills",
            "add",
            "AreteA4/skills",
            "--skill",
            "*",
            "--agent",
            "claude-code",
            "-y"
        ]
    );
    assert!(sb.root.join(".claude/skills/arete/SKILL.md").exists());
    let mcp: Value = serde_json::from_str(&read(&sb, ".mcp.json")).unwrap();
    assert_eq!(
        mcp["mcpServers"]["arete"]["command"],
        exe.to_str().unwrap(),
        "receipt binary used"
    );

    let (report, _) = run(&["init", "-y", "--json"]);
    assert_eq!(results(&report)["skills"]["status"], "unchanged");

    let (doctor, output) = run(&["doctor", "--json"]);
    assert!(output.status.success());
    assert_eq!(doctor["status"], "ok", "{doctor}");
    let c = checks(&doctor);
    assert_eq!(c["cli.install"]["status"], "ok", "{}", c["cli.install"]);
    assert_eq!(c["cli.path"]["status"], "ok", "{}", c["cli.path"]);
    assert_eq!(c["tools.node"]["status"], "ok");
    assert_eq!(c["agents.claude-code.skills"]["status"], "ok");
    assert_eq!(c["agents.claude-code.mcp"]["status"], "ok");
}

#[test]
fn dry_run_writes_nothing_and_reports_would_statuses() {
    let sb = sandbox("empty", false);
    let before = snapshot(&sb.root);
    let (report, output) = a4_json(&sb, &["init", "-y", "--dry-run", "--json"]);
    assert!(output.status.success());
    assert_eq!(report["dryRun"], true);
    let items = results(&report);
    assert_eq!(items["arete.toml"]["status"], "would-create");
    assert_eq!(items["agents-md"]["status"], "would-create");
    assert_eq!(items["claude-md"]["status"], "would-create");
    assert_eq!(items["mcp:claude-code"]["status"], "would-create");
    assert_eq!(items["skills"]["status"], "skipped");
    assert_eq!(snapshot(&sb.root), before);
    assert!(!sb.root.join("arete.toml").exists());
}

#[test]
fn agents_md_keeps_user_content_and_replaces_stale_blocks() {
    let sb = sandbox("agents-md-user", false);
    let (report, _) = a4_json(&sb, &["init", "-y", "--json", "--no-mcp", "--no-skills"]);
    assert_eq!(results(&report)["agents-md"]["status"], "updated");
    let content = read(&sb, "AGENTS.md");
    assert!(content.starts_with("# My project\n\nNotes the team wrote by hand. Keep me.\n\n<!-- BEGIN:arete v2 -->\n## Arete\n"));
    assert!(content.ends_with("<!-- END:arete -->\n\n## Below the block\n\nAlso kept.\n"));
    assert!(!content.contains("out of date"));
    let (report, _) = a4_json(&sb, &["init", "-y", "--json", "--no-mcp", "--no-skills"]);
    assert_eq!(results(&report)["agents-md"]["status"], "unchanged");

    let sb = sandbox("agents-md-stale", false);
    let (report, _) = a4_json(&sb, &["init", "-y", "--json", "--no-mcp", "--no-skills"]);
    assert_eq!(results(&report)["agents-md"]["status"], "updated");
    let content = read(&sb, "AGENTS.md");
    assert!(content.starts_with("# Stale\n\n<!-- BEGIN:arete v2 -->\n"));
    assert!(!content.contains("v0") && !content.contains("old instructions"));
    assert_eq!(content.matches("BEGIN:arete").count(), 1);
}

#[test]
fn existing_mcp_json_keeps_other_servers_and_order() {
    let sb = sandbox("mcp-json-existing", false);
    let (report, output) = a4_json(&sb, &["init", "-y", "--json", "--no-skills"]);
    assert!(output.status.success());
    assert_eq!(
        report["detectedAgents"],
        serde_json::json!([{"id": "claude-code", "how": "project"}])
    );
    assert_eq!(results(&report)["mcp:claude-code"]["status"], "updated");
    let text = read(&sb, ".mcp.json");
    let mcp: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(mcp["mcpServers"]["zeta"]["env"]["ZETA_TOKEN"], "x");
    assert_eq!(
        mcp["mcpServers"]["alpha"]["url"],
        "https://alpha.example/mcp"
    );
    assert_eq!(mcp["mcpServers"]["arete"]["type"], "stdio");
    assert_eq!(mcp["mcpServers"]["arete-docs"]["type"], "http");
    let zeta = text.find("\"zeta\"").unwrap();
    let alpha = text.find("\"alpha\"").unwrap();
    let arete = text.find("\"arete\"").unwrap();
    assert!(
        zeta < alpha && alpha < arete,
        "existing key order kept:\n{text}"
    );
    assert!(
        text.contains("\n  \"mcpServers\""),
        "two-space indent kept:\n{text}"
    );
    let before = snapshot(&sb.root);
    let (report, _) = a4_json(&sb, &["init", "-y", "--json", "--no-skills"]);
    assert_eq!(results(&report)["mcp:claude-code"]["status"], "unchanged");
    assert_eq!(snapshot(&sb.root), before);
}

#[test]
fn opencode_jsonc_comments_survive() {
    let sb = sandbox("opencode-jsonc", false);
    let (report, output) = a4_json(&sb, &["init", "-y", "--json", "--no-skills"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        report["detectedAgents"],
        serde_json::json!([{"id": "opencode", "how": "project"}])
    );
    let items = results(&report);
    assert_eq!(
        items["mcp:opencode"]["status"], "updated",
        "{}",
        items["mcp:opencode"]
    );
    assert_eq!(items["mcp:opencode"]["path"], "opencode.jsonc");
    assert!(
        !items.contains_key("claude-md"),
        "CLAUDE.md is only for claude-code"
    );
    let text = read(&sb, "opencode.jsonc");
    assert!(text.contains("// OpenCode config with comments and trailing commas."));
    assert!(text.contains("// keep this comment"));
    assert!(text.contains("\"mine\""));
    assert!(text.contains("\"arete\""));
    assert!(text.contains("\"arete-docs\""));
    assert!(!sb.root.join("opencode.json").exists());
    let (report, _) = a4_json(&sb, &["init", "-y", "--json", "--no-skills"]);
    assert_eq!(results(&report)["mcp:opencode"]["status"], "unchanged");
}

#[test]
fn codex_toml_keeps_other_tables_and_warns_about_trust() {
    let sb = sandbox("codex-toml", false);
    let (report, output) = a4_json(&sb, &["init", "-y", "--json", "--no-skills"]);
    assert!(output.status.success());
    assert_eq!(
        report["detectedAgents"],
        serde_json::json!([{"id": "codex", "how": "project"}])
    );
    assert_eq!(results(&report)["mcp:codex"]["status"], "updated");
    assert!(report["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|w| w.as_str().unwrap().contains("trusted projects")));
    let text = read(&sb, ".codex/config.toml");
    for kept in [
        "# Codex project config",
        "model = \"gpt-5-codex\"",
        "[sandbox_workspace_write]",
        "[mcp_servers.other]",
        "[profiles.fast]",
        "[mcp_servers.arete]",
        "[mcp_servers.arete-docs]",
    ] {
        assert!(text.contains(kept), "missing {kept:?} in\n{text}");
    }
    let parsed: toml::Value = toml::from_str(&text).unwrap();
    assert_eq!(
        parsed["mcp_servers"]["arete"]["args"],
        toml::Value::Array(vec!["mcp".into()])
    );
    let (report, _) = a4_json(&sb, &["init", "-y", "--json", "--no-skills"]);
    assert_eq!(results(&report)["mcp:codex"]["status"], "unchanged");
    assert!(report["warnings"].as_array().unwrap().is_empty());

    let (doctor, _) = a4_json(&sb, &["doctor", "--json"]);
    let c = checks(&doctor);
    assert_eq!(c["agents.codex.mcp"]["status"], "ok");
    assert_eq!(c["agents.codex-trust"]["status"], "info");
}

#[test]
fn doctor_on_empty_dir_fails_on_manifest() {
    let sb = sandbox("empty", false);
    let (doctor, output) = a4_json(&sb, &["doctor", "--json"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        output.stderr.is_empty(),
        "no generic Error line: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(doctor["status"], "fail");
    let c = checks(&doctor);
    assert_eq!(c["project.manifest"]["status"], "fail");
    assert_eq!(c["project.manifest"]["fix"], "a4 init");
    assert_eq!(c["project.lock"]["status"], "info");
    assert_eq!(c["agents.agents-md"]["status"], "warn");

    // Human mode also exits 1.
    let output = a4(&sb, &["doctor"]);
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("project.manifest"));
    assert!(stdout.contains("Fixes:"));
}

#[test]
fn doctor_fix_restores_a_removed_agents_md_block() {
    let sb = sandbox("empty", false);
    let (_, output) = a4_json(&sb, &["init", "-y", "--json", "--no-skills"]);
    assert!(output.status.success());
    fs::write(sb.root.join("AGENTS.md"), "# mine\n").unwrap();
    fs::remove_file(sb.root.join(".mcp.json")).unwrap();
    fs::create_dir_all(sb.root.join(".claude")).unwrap();
    let (doctor, _) = a4_json(&sb, &["doctor", "--json"]);
    let c = checks(&doctor);
    assert_eq!(c["agents.agents-md"]["status"], "warn");
    assert_eq!(c["agents.agents-md"]["fix"], "a4 doctor --fix");
    assert_eq!(c["agents.claude-code.mcp"]["status"], "warn");

    let (doctor, output) = a4_json(&sb, &["doctor", "--fix", "--json"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let c = checks(&doctor);
    assert_eq!(c["agents.agents-md"]["status"], "ok");
    assert_eq!(c["agents.claude-code.mcp"]["status"], "ok");
    let content = read(&sb, "AGENTS.md");
    assert!(content.starts_with("# mine\n\n<!-- BEGIN:arete v2 -->"));
    assert!(sb.root.join(".mcp.json").exists());
    assert!(sb.root.join("arete.toml").exists());
}

#[test]
fn human_output_and_unknown_agent_errors() {
    let sb = sandbox("empty", false);
    let output = a4(&sb, &["init", "-y", "--no-skills"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("created"), "{stdout}");
    assert!(stdout.contains("AGENTS.md"));
    assert!(stdout.contains("Next:"));
    assert!(stdout.contains("a4 doctor --json"));

    let output = a4(&sb, &["init", "-y", "--agents", "emacs"]);
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Unknown agent id `emacs`"), "{stderr}");
    assert!(stderr.contains("claude-code"));
}

#[test]
fn explicit_agent_list_and_config_path_root() {
    let sb = sandbox("empty", false);
    let nested = sb.root.join("nested");
    fs::create_dir_all(&nested).unwrap();
    let (report, output) = a4_json(
        &sb,
        &[
            "--config",
            "nested/arete.toml",
            "init",
            "-y",
            "--json",
            "--no-skills",
            "--agents",
            "cursor,windsurf,gemini-cli",
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        report["selectedAgents"],
        serde_json::json!(["cursor", "windsurf", "gemini-cli"])
    );
    let items = results(&report);
    assert_eq!(items["arete.toml"]["status"], "created");
    assert_eq!(items["mcp:cursor"]["status"], "created");
    assert_eq!(items["mcp:cursor"]["path"], ".cursor/mcp.json");
    assert_eq!(items["mcp:windsurf"]["status"], "skipped");
    assert_eq!(items["mcp:windsurf"]["reason"], "global only");
    // gemini-context creates .gemini/settings.json first; the MCP writer then updates it.
    assert_eq!(items["gemini-context"]["status"], "created");
    assert_eq!(items["mcp:gemini-cli"]["status"], "updated");
    assert!(!items.contains_key("claude-md"));
    assert!(nested.join("arete.toml").exists());
    assert!(nested.join("AGENTS.md").exists());
    assert!(nested.join(".cursor/mcp.json").exists());
    assert!(!sb.root.join("AGENTS.md").exists(), "root untouched");
    let gemini: Value =
        serde_json::from_str(&fs::read_to_string(nested.join(".gemini/settings.json")).unwrap())
            .unwrap();
    assert_eq!(
        gemini["context"]["fileName"],
        serde_json::json!(["AGENTS.md", "GEMINI.md"])
    );
    assert_eq!(
        gemini["mcpServers"]["arete-docs"]["httpUrl"],
        "https://docs.arete.run/mcp"
    );
    assert!(nested.join("arete.toml").exists());
    assert!(fs::read_to_string(nested.join("arete.toml"))
        .unwrap()
        .contains("name = \"nested\""));

    let (doctor, _) = a4_json(&sb, &["--config", "nested/arete.toml", "doctor", "--json"]);
    let c = checks(&doctor);
    assert_eq!(c["project.manifest"]["status"], "ok");
    assert_eq!(c["agents.cursor.mcp"]["status"], "ok");
    assert_eq!(c["agents.gemini-cli.mcp"]["status"], "ok");
    assert_eq!(c["agents.gemini-context"]["status"], "ok");
    assert_eq!(c["agents.agents-md"]["status"], "ok");
    assert!(!c.contains_key("agents.claude-md"));
}
