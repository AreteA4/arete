use std::fs;
use std::process::Command;

use serde_json::{json, Value};

fn invoke(request: &Value) -> Value {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("request.json");
    fs::write(&path, serde_json::to_vec(request).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_a4"))
        .args(["workspace-agents", "--request"])
        .arg(path)
        .env("ARETE_HOME", temp.path().join("receipts"))
        .env("ARETE_DEV_HOME", temp.path().join("independent-workspace"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !temp.path().join("receipts").exists(),
        "render mode must not touch consumer receipts"
    );
    assert!(!temp.path().join("independent-workspace").exists());
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn all_five_adapters_share_public_writers_and_leave_target_untouched() {
    let temp = tempfile::tempdir().unwrap();
    let result = invoke(
        &json!({"schema_version":1,"target":temp.path(),"harnesses":["codex","claude-code","opencode","oh-my-pi","cursor"],"skills":[{"name":"local-skill","source":temp.path().join("source")}],"mcp":{"local":{"transport":"stdio","command":"/explicit/a4","args":["mcp","--fixture"]},"docs":{"transport":"http","url":"https://docs.arete.run/mcp"}},"instructions":"Internal context only.\n"}),
    );
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
    assert_eq!(
        result["copies"].as_array().unwrap().len(),
        4,
        "Codex and OpenCode share one discovery copy"
    );
    for path in [
        ".codex/config.toml",
        ".mcp.json",
        "opencode.json",
        ".omp/mcp.json",
        ".cursor/mcp.json",
    ] {
        let file = result["files"]
            .as_array()
            .unwrap()
            .iter()
            .find(|file| file["path"] == path)
            .unwrap();
        assert!(file["content"].as_str().unwrap().contains("/explicit/a4"));
        assert_eq!(file["owned_entries"].as_object().unwrap().len(), 2);
    }
    assert!(!result.to_string().contains("arete.toml"));
}

#[test]
fn retirement_preserves_jsonc_comments_unrelated_servers_and_toml_preferences() {
    let temp = tempfile::tempdir().unwrap();
    let old = json!({"type":"stdio","command":"/explicit/a4","args":["mcp"]});
    let result = invoke(
        &json!({"schema_version":1,"target":temp.path(),"harnesses":[],"skills":[],"mcp":{},"instructions":"","existing":{
            ".mcp.json":{"content":format!("{{ // user comment\n\"mcpServers\":{{\"local\":{old},\"user\":{{\"command\":\"custom\"}}}}}}"),"owned_entries":{"local":old}},
            ".codex/config.toml":{"content":"model = 'user'\n[mcp_servers.local]\ncommand = '/explicit/a4'\nargs = ['mcp']\n","owned_entries":{"local":{"command":"/explicit/a4","args":["mcp"]}}}
        }}),
    );
    let text = result.to_string();
    assert!(text.contains("user comment") && text.contains("custom"));
    assert!(text.contains("model = 'user'"));
    assert!(!text.contains("/explicit/a4"));
}

#[test]
fn protocol_info_and_invalid_requests_do_not_initialize_consumers() {
    let temp = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_a4"))
        .args(["workspace-agents", "--protocol-info"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(output.status.success());
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["request_schema"], json!([1]));
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
}
