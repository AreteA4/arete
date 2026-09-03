//! Tests for `a4 self update` / `a4 upgrade` against a local HTTP server
//! (`A4_LATEST_URL`, `A4_MANIFEST_BASE_URL`). The full swap cannot be
//! exercised here because release assets must be signed by the production
//! key; the tests cover receipt gating, `--check` exit codes, the
//! "still publishing" 404 path and signature rejection (binary untouched).

use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::{Arc, Mutex};

const A4: &str = env!("CARGO_BIN_EXE_a4");
const VERSION: &str = env!("CARGO_PKG_VERSION");

type Routes = Arc<Mutex<HashMap<String, (u16, Vec<u8>)>>>;

/// Minimal HTTP/1.1 server: path -> (status, body). Unknown paths get 404.
struct Server {
    base_url: String,
    routes: Routes,
}

impl Server {
    fn start() -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let routes: Routes = Arc::default();
        let thread_routes = Arc::clone(&routes);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut buffer = [0u8; 4096];
                let read = stream.read(&mut buffer).unwrap_or(0);
                let request = String::from_utf8_lossy(&buffer[..read]).to_string();
                let path = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/")
                    .to_string();
                let (status, body) = thread_routes
                    .lock()
                    .unwrap()
                    .get(&path)
                    .cloned()
                    .unwrap_or((404, b"not found".to_vec()));
                let reason = match status {
                    200 => "OK",
                    404 => "Not Found",
                    _ => "Error",
                };
                let header = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(&body);
                let _ = stream.flush();
            }
        });
        Server {
            base_url: format!("http://{addr}"),
            routes,
        }
    }

    fn route(&self, path: &str, status: u16, body: impl Into<Vec<u8>>) {
        self.routes
            .lock()
            .unwrap()
            .insert(path.to_string(), (status, body.into()));
    }

    fn latest_url(&self) -> String {
        format!("{}/a4/latest.json", self.base_url)
    }

    /// Base to which the CLI appends `/a4-cli-v<version>`.
    fn release_base(&self) -> String {
        format!("{}/releases", self.base_url)
    }
}

struct Sandbox {
    _dir: tempfile::TempDir,
    home: PathBuf,
    arete_home: PathBuf,
    binary: PathBuf,
}

impl Sandbox {
    /// A sandbox with a receipt pointing at a private copy of the test binary.
    fn with_receipt() -> Sandbox {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let install_dir = home.join(".local").join("bin");
        fs::create_dir_all(&install_dir).unwrap();
        let binary = install_dir.join(if cfg!(windows) { "a4.exe" } else { "a4" });
        fs::copy(A4, &binary).unwrap();
        let arete_home = home.join(".arete");
        fs::create_dir_all(&arete_home).unwrap();
        let receipt = serde_json::json!({
            "schemaVersion": 1,
            "version": VERSION,
            "binary": binary,
            "installDir": install_dir,
            "platform": "test",
            "source": "sh",
            "verified": true,
            "modifyPath": false,
            "installedAt": "2026-09-10T12:00:00Z",
        });
        fs::write(
            arete_home.join("receipt.json"),
            serde_json::to_string_pretty(&receipt).unwrap(),
        )
        .unwrap();
        Sandbox {
            home,
            arete_home,
            binary,
            _dir: dir,
        }
    }

    fn without_receipt() -> Sandbox {
        let sandbox = Sandbox::with_receipt();
        fs::remove_file(sandbox.arete_home.join("receipt.json")).unwrap();
        sandbox
    }

    fn a4(&self, server: &Server) -> Command {
        let mut cmd = Command::new(A4);
        cmd.env_remove("CI")
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("ARETE_HOME", &self.arete_home)
            .env("A4_NO_UPDATE_CHECK", "1")
            .env("A4_LATEST_URL", server.latest_url())
            .env("A4_MANIFEST_BASE_URL", server.release_base())
            .env("DO_NOT_TRACK", "1")
            .env("NO_COLOR", "1");
        cmd
    }
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/selfhost")
        .join(name)
}

fn latest_body(version: &str) -> String {
    format!("{{ \"schemaVersion\": 1, \"version\": \"{version}\" }}")
}

#[test]
fn without_receipt_update_refuses_with_reinstall_hint() {
    let server = Server::start();
    let sandbox = Sandbox::without_receipt();
    for args in [vec!["self", "update", "--check"], vec!["upgrade"]] {
        let output = sandbox.a4(&server).args(&args).output().unwrap();
        assert_eq!(output.status.code(), Some(1), "{args:?}");
        let err = stderr(&output);
        assert!(
            err.contains("a4 was not installed by the Arete installer. Reinstall with: curl -fsSL https://arete.run/install.sh | sh"),
            "{err}"
        );
        // The test binary is not under ~/.cargo/bin, so no cargo hint.
        assert!(!err.contains("cargo install"), "{err}");
    }
}

#[test]
fn check_exits_10_when_newer_and_0_when_current() {
    let server = Server::start();
    let sandbox = Sandbox::with_receipt();

    server.route("/a4/latest.json", 200, latest_body("99.0.0"));
    let output = sandbox
        .a4(&server)
        .args(["self", "update", "--check", "--json"])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(10),
        "stderr: {}",
        stderr(&output)
    );
    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(json["schemaVersion"], 1);
    assert_eq!(json["current"], VERSION);
    assert_eq!(json["latest"], "99.0.0");
    assert_eq!(json["updateAvailable"], true);
    // No "Error:" line for exit 10, and nothing downloaded.
    assert!(!stderr(&output).contains("Error:"));
    assert!(!sandbox.arete_home.join("downloads").exists());

    // Human mode: the message is on stderr, stdout is empty.
    let human = sandbox
        .a4(&server)
        .args(["upgrade", "--check"])
        .output()
        .unwrap();
    assert_eq!(human.status.code(), Some(10));
    assert!(stdout(&human).is_empty());
    assert_eq!(
        stderr(&human).trim(),
        format!("a4 99.0.0 is available (you have {VERSION}). Run: a4 self update")
    );

    server.route("/a4/latest.json", 200, latest_body(VERSION));
    let output = sandbox
        .a4(&server)
        .args(["self", "update", "--check", "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "stderr: {}", stderr(&output));
    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(json["updateAvailable"], false);
    assert_eq!(json["latest"], VERSION);

    // An older pointer is not an update.
    server.route("/a4/latest.json", 200, latest_body("0.0.1"));
    let output = sandbox
        .a4(&server)
        .args(["self", "update", "--check"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert!(stderr(&output).contains("up to date"));

    // An explicit downgrade is "available" (allowed when explicit).
    let output = sandbox
        .a4(&server)
        .args(["self", "update", "0.0.1", "--check", "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(10));
    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(json["latest"], "0.0.1");
    assert_eq!(json["updateAvailable"], true);
}

#[test]
fn check_fails_cleanly_when_latest_is_unreachable() {
    let server = Server::start();
    let sandbox = Sandbox::with_receipt();
    // No /a4/latest.json route: 404.
    let output = sandbox
        .a4(&server)
        .args(["self", "update", "--check"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("latest"), "{}", stderr(&output));
}

#[test]
fn update_reports_still_publishing_on_404() {
    let server = Server::start();
    let sandbox = Sandbox::with_receipt();
    server.route("/a4/latest.json", 200, latest_body("99.0.0"));
    let output = sandbox
        .a4(&server)
        .args(["self", "update", "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let err = stderr(&output);
    assert!(
        err.contains("Release 99.0.0 is still publishing; retry in a few minutes"),
        "{err}"
    );
    assert!(!sandbox.arete_home.join("downloads").join("99.0.0").exists());
    assert_eq!(fs::read(&sandbox.binary).unwrap(), fs::read(A4).unwrap());
}

#[test]
fn update_rejects_signature_from_another_key_and_keeps_binary() {
    let server = Server::start();
    let sandbox = Sandbox::with_receipt();
    let version = "99.0.0";
    server.route("/a4/latest.json", 200, latest_body(version));

    let asset = platform_asset_name();
    let manifest = serde_json::json!({
        "schemaVersion": 1, "name": "a4", "version": version, "tag": format!("a4-cli-v{version}"),
        "releasedAt": "2026-09-10T12:00:00Z", "assets": {},
        "checksums": "checksums.txt", "signature": "checksums.txt.minisig", "minimumVersion": null
    });
    let base = format!("/releases/a4-cli-v{version}");
    server.route(&format!("{base}/manifest.json"), 200, manifest.to_string());
    server.route(
        &format!("{base}/{asset}"),
        200,
        fs::read(fixture("a4-linux-x64")).unwrap(),
    );
    server.route(
        &format!("{base}/checksums.txt"),
        200,
        fs::read(fixture("checksums.txt")).unwrap(),
    );
    // Valid for the test key, but the CLI trusts only the production key.
    server.route(
        &format!("{base}/checksums.txt.minisig"),
        200,
        fs::read(fixture("checksums.txt.minisig")).unwrap(),
    );

    for extra in [vec![], vec!["--dry-run"]] {
        let output = sandbox
            .a4(&server)
            .args(["self", "update"])
            .args(&extra)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{extra:?}");
        let err = stderr(&output);
        assert!(err.contains("Signature check failed"), "{err}");
        assert_eq!(fs::read(&sandbox.binary).unwrap(), fs::read(A4).unwrap());
        assert!(!sandbox.arete_home.join("downloads").join(version).exists());
        let receipt: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(sandbox.arete_home.join("receipt.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(receipt["version"], VERSION);
    }
}

#[test]
fn explicit_version_must_be_semver() {
    let server = Server::start();
    let sandbox = Sandbox::with_receipt();
    let output = sandbox
        .a4(&server)
        .args(["self", "update", "latest"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("semver"), "{}", stderr(&output));
}

/// Mirrors `platform::asset_name(platform_key())` for the host.
fn platform_asset_name() -> String {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "win32",
        other => other,
    };
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "x64",
        other => other,
    };
    if os == "win32" {
        format!("a4-{os}-{arch}.exe")
    } else {
        format!("a4-{os}-{arch}")
    }
}
