//! End-to-end tests for `a4 self install` / `a4 self uninstall` using the
//! built `a4` binary. Every process gets its own HOME / ARETE_HOME /
//! A4_INSTALL_DIR in a tempdir; environment is set on the child only, since
//! tests run in parallel.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const A4: &str = env!("CARGO_BIN_EXE_a4");
const VERSION: &str = env!("CARGO_PKG_VERSION");

struct Sandbox {
    _dir: tempfile::TempDir,
    home: PathBuf,
    arete_home: PathBuf,
    install_dir: PathBuf,
}

impl Sandbox {
    fn new() -> Sandbox {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        fs::create_dir_all(&home).unwrap();
        Sandbox {
            arete_home: home.join(".arete"),
            install_dir: home.join(".local").join("bin"),
            home,
            _dir: dir,
        }
    }

    /// A command for `exe` (the test binary or the installed copy) with the
    /// sandbox environment and PATH edits disabled.
    fn cmd(&self, exe: &Path) -> Command {
        let mut cmd = Command::new(exe);
        cmd.env_remove("CI")
            .env_remove("GITHUB_PATH")
            .env_remove("XDG_BIN_HOME")
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("ARETE_HOME", &self.arete_home)
            .env("A4_INSTALL_DIR", &self.install_dir)
            .env("A4_NO_MODIFY_PATH", "1")
            .env("A4_NO_UPDATE_CHECK", "1")
            .env("DO_NOT_TRACK", "1")
            .env("NO_COLOR", "1")
            .env("PATH", self.home.join("empty-path"));
        cmd
    }

    fn a4(&self) -> Command {
        self.cmd(Path::new(A4))
    }

    fn binary(&self) -> PathBuf {
        let name = if cfg!(windows) { "a4.exe" } else { "a4" };
        self.install_dir.join(name)
    }

    fn receipt(&self) -> serde_json::Value {
        let text = fs::read_to_string(self.arete_home.join("receipt.json")).unwrap();
        serde_json::from_str(&text).unwrap()
    }
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

/// Split install stdout into (JSON object, `A4_BIN=` value, export line).
fn parse_install_stdout(text: &str) -> (serde_json::Value, String, String) {
    let lines: Vec<&str> = text.trim_end().lines().collect();
    assert!(lines.len() >= 2, "stdout too short: {text:?}");
    let export = lines[lines.len() - 1].to_string();
    let a4_bin = lines[lines.len() - 2]
        .strip_prefix("A4_BIN=")
        .unwrap_or_else(|| panic!("second-to-last line must be A4_BIN=: {text:?}"))
        .to_string();
    let json_text = lines[..lines.len() - 2].join("\n");
    let json = serde_json::from_str(&json_text).unwrap_or_else(|error| {
        panic!("stdout before the final lines is not JSON ({error}): {json_text}")
    });
    (json, a4_bin, export)
}

#[test]
fn install_writes_receipt_prints_a4_bin_and_rerun_is_noop() {
    let sandbox = Sandbox::new();
    let output = sandbox
        .a4()
        .args(["self", "install", "--source", "sh", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success(), "stderr: {}", stderr(&output));

    let (json, a4_bin, export) = parse_install_stdout(&stdout(&output));
    assert_eq!(json["schemaVersion"], 1);
    assert_eq!(json["status"], "installed");
    assert_eq!(json["version"], VERSION);
    assert_eq!(json["source"], "sh");
    assert_eq!(json["verified"], false);
    assert_eq!(json["modifyPath"], false);
    assert_eq!(json["pathModified"], serde_json::json!([]));
    assert_eq!(json["shadowedBy"], serde_json::Value::Null);
    assert_eq!(Path::new(&a4_bin), sandbox.binary());
    if cfg!(windows) {
        assert!(export.starts_with("$env:Path = "), "{export}");
    } else {
        assert_eq!(export, "export PATH=\"$HOME/.local/bin:$PATH\"");
    }

    let receipt = sandbox.receipt();
    assert_eq!(receipt["schemaVersion"], 1);
    assert_eq!(receipt["version"], VERSION);
    assert_eq!(
        Path::new(receipt["binary"].as_str().unwrap()),
        sandbox.binary()
    );
    assert_eq!(
        Path::new(receipt["installDir"].as_str().unwrap()),
        sandbox.install_dir
    );
    assert_eq!(receipt["verified"], false);
    assert!(receipt["installedAt"].as_str().unwrap().ends_with('Z'));
    assert!(!sandbox
        .install_dir
        .join(format!("a4.tmp-{}", std::process::id()))
        .exists());

    // The installed copy runs and reports the same version.
    let version = sandbox
        .cmd(&sandbox.binary())
        .arg("--version")
        .output()
        .unwrap();
    assert!(version.status.success());
    assert!(stdout(&version).contains(VERSION));

    // Re-running from the installed binary is a no-op.
    let before = fs::metadata(sandbox.binary()).unwrap().modified().unwrap();
    let rerun = sandbox
        .cmd(&sandbox.binary())
        .args(["self", "install", "--source", "sh", "--json"])
        .output()
        .unwrap();
    assert!(rerun.status.success(), "stderr: {}", stderr(&rerun));
    let (json, a4_bin, _) = parse_install_stdout(&stdout(&rerun));
    assert_eq!(json["status"], "unchanged");
    assert_eq!(Path::new(&a4_bin), sandbox.binary());
    assert_eq!(
        fs::metadata(sandbox.binary()).unwrap().modified().unwrap(),
        before
    );
    assert!(stderr(&rerun).contains("already installed"));
}

#[test]
fn human_output_goes_to_stderr_and_final_lines_to_stdout() {
    let sandbox = Sandbox::new();
    let output = sandbox.a4().args(["self", "install"]).output().unwrap();
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let out = stdout(&output);
    let lines: Vec<&str> = out.trim_end().lines().collect();
    assert_eq!(
        lines.len(),
        2,
        "stdout must be exactly the two final lines: {out:?}"
    );
    assert!(lines[0].starts_with("A4_BIN="));
    assert!(stderr(&output).contains("Installed a4"));
    assert!(stderr(&output).contains("Not verified"));
}

#[test]
fn unknown_source_is_rejected() {
    let sandbox = Sandbox::new();
    let output = sandbox
        .a4()
        .args(["self", "install", "--source", "brew"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("--source"));
    assert!(!sandbox.binary().exists());
}

#[test]
fn signature_from_another_key_fails_and_installs_nothing() {
    let sandbox = Sandbox::new();
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/selfhost");
    let output = sandbox
        .a4()
        .args(["self", "install", "--source", "sh", "--json"])
        .arg("--checksums")
        .arg(fixtures.join("checksums.txt"))
        .arg("--signature")
        .arg(fixtures.join("checksums.txt.minisig"))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("Signature check failed"),
        "{}",
        stderr(&output)
    );
    assert!(!sandbox.binary().exists());
    assert!(!sandbox.arete_home.join("receipt.json").exists());
    assert!(stdout(&output).is_empty());
}

#[test]
fn missing_signature_flag_is_a_usage_error() {
    let sandbox = Sandbox::new();
    let output = sandbox
        .a4()
        .args(["self", "install", "--checksums", "x.txt"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(!sandbox.binary().exists());
}

#[cfg(unix)]
#[test]
fn path_edits_are_idempotent_and_removed_by_uninstall() {
    let sandbox = Sandbox::new();
    let zshrc = sandbox.home.join(".zshrc");
    fs::write(&zshrc, "alias ll='ls -l'\n").unwrap();
    let profile = sandbox.home.join(".profile");

    let run = || {
        let mut cmd = sandbox.a4();
        cmd.env_remove("A4_NO_MODIFY_PATH")
            .env("SHELL", "/bin/zsh")
            .args(["self", "install", "--source", "manual", "--json"]);
        cmd.output().unwrap()
    };

    let first = run();
    assert!(first.status.success(), "stderr: {}", stderr(&first));
    let (json, _, _) = parse_install_stdout(&stdout(&first));
    assert_eq!(json["modifyPath"], true);
    let modified: Vec<String> = json["pathModified"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        modified,
        vec![profile.display().to_string(), zshrc.display().to_string()]
    );
    let expected_line = "export PATH=\"$HOME/.local/bin:$PATH\" # added by a4 self install\n";
    assert_eq!(fs::read_to_string(&profile).unwrap(), expected_line);
    assert_eq!(
        fs::read_to_string(&zshrc).unwrap(),
        format!("alias ll='ls -l'\n{expected_line}")
    );
    assert!(!sandbox.home.join(".bashrc").exists());
    assert!(!sandbox.home.join(".bash_profile").exists());

    let second = run();
    assert!(second.status.success());
    let (json, _, _) = parse_install_stdout(&stdout(&second));
    assert_eq!(json["pathModified"], serde_json::json!([]));
    assert_eq!(fs::read_to_string(&profile).unwrap(), expected_line);
    assert_eq!(
        fs::read_to_string(&zshrc).unwrap(),
        format!("alias ll='ls -l'\n{expected_line}")
    );

    // Uninstall removes the binary, the receipt and exactly the added lines.
    fs::write(sandbox.arete_home.join("credentials.toml"), "[keys]\n").unwrap();
    let uninstall = sandbox
        .a4()
        .args(["self", "uninstall", "--json"])
        .output()
        .unwrap();
    assert!(uninstall.status.success(), "stderr: {}", stderr(&uninstall));
    let json: serde_json::Value = serde_json::from_str(&stdout(&uninstall)).unwrap();
    assert_eq!(json["schemaVersion"], 1);
    let removed: Vec<String> = json["removed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert!(removed.contains(&sandbox.binary().display().to_string()));
    assert!(removed.contains(
        &sandbox
            .arete_home
            .join("receipt.json")
            .display()
            .to_string()
    ));
    assert!(!sandbox.binary().exists());
    assert!(!sandbox.arete_home.join("receipt.json").exists());
    assert_eq!(fs::read_to_string(&zshrc).unwrap(), "alias ll='ls -l'\n");
    assert_eq!(fs::read_to_string(&profile).unwrap(), "");
    let left: Vec<String> = json["leftBehind"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert!(left.contains(
        &sandbox
            .arete_home
            .join("credentials.toml")
            .display()
            .to_string()
    ));
    assert!(sandbox.arete_home.join("credentials.toml").exists());
    assert!(stderr(&uninstall).contains("Left behind"));
}

#[cfg(unix)]
#[test]
fn ci_env_skips_rc_edits_but_github_path_is_honoured() {
    let sandbox = Sandbox::new();
    let github_path = sandbox.home.join("github_path");
    let output = sandbox
        .a4()
        .env_remove("A4_NO_MODIFY_PATH")
        .env("CI", "true")
        .env("GITHUB_PATH", &github_path)
        .env("SHELL", "/bin/bash")
        .args(["self", "install", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let (json, _, _) = parse_install_stdout(&stdout(&output));
    assert_eq!(json["modifyPath"], false);
    assert_eq!(
        json["pathModified"],
        serde_json::json!([github_path.display().to_string()])
    );
    assert!(!sandbox.home.join(".profile").exists());
    assert_eq!(
        fs::read_to_string(&github_path).unwrap(),
        format!("{}\n", sandbox.install_dir.display())
    );
}

#[cfg(unix)]
#[test]
fn shadowing_binary_is_reported_never_deleted() {
    use std::os::unix::fs::PermissionsExt;
    let sandbox = Sandbox::new();
    let cargo_bin = sandbox.home.join(".cargo").join("bin");
    fs::create_dir_all(&cargo_bin).unwrap();
    let other = cargo_bin.join("a4");
    fs::write(&other, "#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(&other, fs::Permissions::from_mode(0o755)).unwrap();

    let path = std::env::join_paths([&cargo_bin, &sandbox.install_dir]).unwrap();
    let output = sandbox
        .a4()
        .env("PATH", path)
        .args(["self", "install", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let (json, _, _) = parse_install_stdout(&stdout(&output));
    assert_eq!(
        json["shadowedBy"],
        serde_json::json!(other.display().to_string())
    );
    let err = stderr(&output);
    assert!(err.contains("cargo uninstall a4-cli"), "{err}");
    assert!(other.exists());
}

#[test]
fn uninstall_without_install_reports_nothing_to_remove() {
    let sandbox = Sandbox::new();
    let output = sandbox
        .a4()
        .args(["self", "uninstall", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(json["removed"], serde_json::json!([]));
    assert!(stderr(&output).contains("Nothing to remove"));
}

#[test]
fn developer_workspace_never_redirects_default_or_overridden_public_receipts() {
    for custom_receipts in [false, true] {
        let mut sandbox = Sandbox::new();
        let workspace = sandbox._dir.path().join("developer workspace");
        fs::create_dir_all(workspace.join(".arete-workspace")).unwrap();
        fs::write(
            workspace.join(".arete-workspace/workspace.json"),
            "private sentinel",
        )
        .unwrap();
        fs::write(
            workspace.join("arete-dev.toml"),
            "deliberately invalid private declaration",
        )
        .unwrap();
        if custom_receipts {
            sandbox.arete_home = sandbox._dir.path().join("independent public receipts");
        }
        let mut install = sandbox.a4();
        install
            .env("ARETE_DEV_HOME", &workspace)
            .current_dir(&workspace);
        if !custom_receipts {
            install.env_remove("ARETE_HOME");
        }
        let output = install
            .args(["self", "install", "--source", "sh", "--json"])
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", stderr(&output));
        assert_eq!(sandbox.receipt()["version"], VERSION);
        assert!(!workspace.join("receipt.json").exists());
        assert!(!workspace.join(".arete/receipt.json").exists());
        if custom_receipts {
            assert!(!sandbox.home.join(".arete/receipt.json").exists());
        }
        let mut uninstall = sandbox.a4();
        uninstall
            .env("ARETE_DEV_HOME", &workspace)
            .current_dir(&workspace);
        if !custom_receipts {
            uninstall.env_remove("ARETE_HOME");
        }
        let output = uninstall
            .args(["self", "uninstall", "--json"])
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", stderr(&output));
        assert!(!sandbox.arete_home.join("receipt.json").exists());
        assert_eq!(
            fs::read_to_string(workspace.join(".arete-workspace/workspace.json")).unwrap(),
            "private sentinel"
        );
        assert_eq!(
            fs::read_to_string(workspace.join("arete-dev.toml")).unwrap(),
            "deliberately invalid private declaration"
        );
    }
}
