//! Exercise the real child-process protocol and recovery from a saved pin
//! whose client regeneration failed. No hosted API or dev wallet is used.
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::{json, Value};

const RELEASE: &str = "arete:h1:program-release:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const MARKER: &str = ".arete/dev/local-program-admission.pending";

fn scaffold(root: &Path) {
    let output = Command::new(env!("CARGO_BIN_EXE_a4"))
        .current_dir(root.parent().unwrap())
        .args(["protocol", "init", "--local-idl"])
        .arg(root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn api(root: &Path, remove_idl: bool) -> (String, std::thread::JoinHandle<()>) {
    let idl_path = root.join("idl/system.idl.json");
    let spec =
        arete_hash::build_program_spec_v1_from_bytes(&std::fs::read(&idl_path).unwrap(), None)
            .unwrap();
    let owned = json!({"schema":"arete.user-program-list/v1","items":[{
        "schema":"arete.user-program/v1","userProgramId":format!("upr_{}","a".repeat(32)),
        "programId":"11111111111111111111111111111111","programSpecHash":spec.hash().unwrap().to_string(),
        "alias":"tip-jar-system","lifecycleState":"active","admissionState":"ready","visibility":"private",
        "programReleaseHash":RELEASE,"programReadBindingId":format!("prb_{}","b".repeat(32)),"operationalStatus":"unverified",
        "health":{"status":"unverified","assessedAt":null,"schemaRelevantAttempts":0,"schemaFailureRateBasisPoints":0},
        "eventCursor":"uev_AAAAAAAAAAE","diagnosticCodes":[],"idempotent":false
    }],"nextCursor":null});
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();
    let thread = std::thread::spawn(move || {
        for (index, body) in [owned.to_string(), "[]".into()].into_iter().enumerate() {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            let (mut stream, _) = loop {
                match listener.accept() {
                    Ok(connection) => break connection,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && std::time::Instant::now() < deadline =>
                    {
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                    Err(error) => panic!("admission mock did not receive request {index}: {error}"),
                }
            };
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(10)))
                .unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                stream.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
            }
            if index == 1 && remove_idl {
                std::fs::remove_file(&idl_path).unwrap();
            }
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
        }
    });
    (origin, thread)
}

fn worker(root: &Path, origin: &str, repair_only: bool) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_a4"))
        .args(["dev", "__admit-local-programs"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(json!({
        "root":root,"origin":origin,"key":"a4_sk_test","slug":"tip-jar","repair_only":repair_only,
    }).to_string().as_bytes()).unwrap();
    child.wait_with_output().unwrap()
}

fn frames(output: &Output) -> Vec<Value> {
    String::from_utf8(output.stdout.clone())
        .unwrap()
        .lines()
        .map(|line| {
            serde_json::from_str(line)
                .unwrap_or_else(|error| panic!("non-JSON worker stdout: {line:?}: {error}"))
        })
        .collect()
}

fn generated(root: &Path) -> String {
    fn visit(path: PathBuf, text: &mut String) {
        for file in std::fs::read_dir(path).unwrap() {
            let path = file.unwrap().path();
            if path.is_dir() {
                visit(path, text);
            } else {
                text.push_str(&std::fs::read_to_string(path).unwrap());
            }
        }
    }
    let mut text = String::new();
    visit(root.join("generated"), &mut text);
    text
}

#[test]
fn first_pin_regeneration_keeps_worker_stdout_json_only() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("tip-jar");
    scaffold(&root);
    let (origin, server) = api(&root, false);
    let output = worker(&root, &origin, false);
    server.join().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(frames(&output).last().unwrap()["lockChanged"], true);
    assert!(String::from_utf8_lossy(&output.stderr).contains("Generating"));
    assert!(generated(&root).contains(RELEASE));
    assert!(!root.join(MARKER).exists());
}

#[test]
fn interrupted_pin_update_can_be_repaired_before_building() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("tip-jar");
    scaffold(&root);
    let idl = std::fs::read(root.join("idl/system.idl.json")).unwrap();
    let (origin, server) = api(&root, true);
    let failed = worker(&root, &origin, false);
    server.join().unwrap();
    assert!(!failed.status.success());
    frames(&failed);
    assert!(root.join(MARKER).exists());
    assert!(std::fs::read_to_string(root.join("arete.lock"))
        .unwrap()
        .contains(RELEASE));
    assert!(!generated(&root).contains(RELEASE));
    std::fs::write(root.join("idl/system.idl.json"), idl).unwrap();
    // Repair requires no API lookup or protocol build: the new lock is authoritative.
    let repaired = worker(&root, "", true);
    assert!(
        repaired.status.success(),
        "{}",
        String::from_utf8_lossy(&repaired.stderr)
    );
    assert_eq!(frames(&repaired).last().unwrap()["lockChanged"], true);
    assert!(generated(&root).contains(RELEASE));
    assert!(!root.join(MARKER).exists());
}
