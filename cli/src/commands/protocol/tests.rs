use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::*;
use crate::api_client::test_support::{MockServer, ReceivedRequest};

const SECRET_KEY: &str = "a4_sk_0123456789012345678901234567890123456789";
/// `escrow-accounts.server` in `test-vectors/protocol-release/vectors.json`.
const ESCROW_RELEASE: &str =
    "arete:h1:protocol-release:sha256:12669446dd0391e9c1eac87e8640bf5fb19ef9659bbc8e7e81fb833c40398075";
/// `token-raise.server` in the same file.
const TOKEN_RAISE_RELEASE: &str =
    "arete:h1:protocol-release:sha256:43819eb555924faaa5ea9b006b95de74cf72034d64c371bec494f4d4cde66608";
const PROTOCOL_ID: &str = "7f1c1a52-4cf8-4f3e-9d1e-1f0f5d0d0a01";
const DEPLOYMENT_ID: &str = "0b5cb1c2-8c5f-4d0e-8f4d-2c6a7c1f9e10";

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/protocol")
        .join(name)
}

/// A project directory holding a built protocol.
fn project(spec: &[u8], bundle: Option<&[u8]>) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let build = dir.path().join(PROTOCOL_BUILD_DIR);
    fs::create_dir_all(&build).unwrap();
    fs::write(build.join("protocol-spec.json"), spec).unwrap();
    if let Some(bundle) = bundle {
        fs::write(build.join("handlers.mjs"), bundle).unwrap();
    }
    write_build_sources(dir.path(), &[]);
    dir
}

/// Write `sources` into the project and a receipt plus `build-sources.json`
/// recording them, as `arete-protocol build` would.
fn write_build_sources(dir: &Path, sources: &[(&str, &str)]) {
    let files: Vec<BuildSource> = sources
        .iter()
        .map(|(path, contents)| {
            let file = dir.join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(&file, contents).unwrap();
            BuildSource {
                path: path.to_string(),
                sha256: sha256_hex(contents.as_bytes()),
            }
        })
        .collect();
    let tree = source_tree_sha256(&files).unwrap();
    let build = dir.join(PROTOCOL_BUILD_DIR);
    fs::create_dir_all(&build).unwrap();
    fs::write(
        build.join("build-receipt.json"),
        serde_json::to_vec(&json!({ "sourceTreeSha256": tree })).unwrap(),
    )
    .unwrap();
    let files: Vec<Value> = files
        .iter()
        .map(|file| json!({ "path": file.path, "sha256": file.sha256 }))
        .collect();
    fs::write(
        build.join("build-sources.json"),
        serde_json::to_vec(&json!({
            "schema": "arete.protocol-build-sources/v1",
            "sourceTreeSha256": tree,
            "files": files,
        }))
        .unwrap(),
    )
    .unwrap();
}

fn escrow_spec() -> Vec<u8> {
    fs::read(fixture("escrow-accounts.protocol-spec.json")).unwrap()
}

/// The escrow fixture plus one declared resource, so a new deployment has
/// to publish a resource version.
fn escrow_spec_with_resource() -> Vec<u8> {
    let mut spec: Value = serde_json::from_slice(&escrow_spec()).unwrap();
    spec["resources"] = json!({
        "limits": {
            "schema": {
                "type": "object",
                "properties": { "maxDeposits": { "type": "integer", "minimum": 0, "maximum": 100 } },
                "required": ["maxDeposits"],
                "additionalProperties": false
            },
            "description": "Publisher limits."
        }
    });
    serde_json::to_vec(&spec).unwrap()
}

fn token_raise() -> tempfile::TempDir {
    project(
        &fs::read(fixture("token-raise.protocol-spec.json")).unwrap(),
        Some(&fs::read(fixture("token-raise.handlers.mjs")).unwrap()),
    )
}

fn client(server: &MockServer) -> ApiClient {
    ApiClient::with_base_url(server.base_url()).with_api_key(SECRET_KEY.to_string())
}

fn args(dir: &Path) -> DeployArgs {
    DeployArgs {
        dir: dir.to_path_buf(),
        no_build: true,
        ..DeployArgs::default()
    }
}

fn write_json(dir: &Path, name: &str, value: &Value) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, serde_json::to_vec(value).unwrap()).unwrap();
    path
}

fn fee_policy() -> Value {
    json!({
        "schema": "arete.protocol-fee-policy/v1",
        "platformDestination": "9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM",
        "steps": {}
    })
}

/// A hosted fee policy without `platformDestination`: Arete fills in its
/// own destination, so the CLI sends the document as written.
fn hosted_fee_policy() -> Value {
    let mut policy = fee_policy();
    policy
        .as_object_mut()
        .unwrap()
        .remove("platformDestination");
    policy
}

fn protocol_json(slug: &str) -> Value {
    json!({ "id": PROTOCOL_ID, "slug": slug, "name": "escrowResolution", "createdAt": "2026-10-13T00:00:00Z" })
}

fn upload_json(
    release_hash: &str,
    has_handlers: bool,
    template_check: &str,
    created: bool,
) -> Value {
    json!({
        "schema": "arete.protocol-release-upload/v1",
        "protocolId": PROTOCOL_ID,
        "releaseHash": release_hash,
        "specHash": "arete:h1:protocol-spec:sha256:00",
        "stackSourceHash": "arete:h1:protocol-stack-source:sha256:00",
        "handlerBundleHash": null,
        "hasHandlers": has_handlers,
        "templateCheck": template_check,
        "reviewed": false,
        "created": created
    })
}

fn deployment_json(id: &str, status: &str, release_hash: &str, access_mode: &str) -> Value {
    json!({
        "id": id,
        "protocolId": PROTOCOL_ID,
        "shortId": "abcdefghij",
        "status": status,
        "revision": 1,
        "releaseHash": release_hash,
        "accessMode": access_mode,
        "attestationKeyId": "attest-1",
        "trustTier": "unreviewed",
        "runtimeKind": "template-v1"
    })
}

fn resource_json(version: i64, status: &str) -> Value {
    json!({
        "id": "res-1",
        "name": "limits",
        "version": version,
        "status": status,
        "schemaSha256": "00",
        "validatedReleaseHash": ESCROW_RELEASE,
        "resourceHash": "arete:h1:protocol-resource:sha256:00",
        "document": {}
    })
}

fn responses(items: Vec<(u16, Value)>) -> MockServer {
    MockServer::json_sequence(
        items
            .into_iter()
            .map(|(status, body)| (status, body.to_string()))
            .collect(),
    )
}

fn take(server: &MockServer, count: usize) -> Vec<ReceivedRequest> {
    (0..count).map(|_| server.request()).collect()
}

fn line(request: &ReceivedRequest) -> &str {
    request
        .request_line
        .strip_suffix(" HTTP/1.1")
        .unwrap_or(&request.request_line)
}

fn body(request: &ReceivedRequest) -> Value {
    serde_json::from_str(&request.body).unwrap()
}

#[test]
fn the_escrow_fixture_predicts_the_vector_release_hash() {
    let dir = project(&escrow_spec(), None);
    let local = load_local_release(dir.path()).unwrap();
    assert_eq!(local.expected_release_hash, ESCROW_RELEASE);
    assert!(!local.spec.has_handlers());

    let dir = token_raise();
    let local = load_local_release(dir.path()).unwrap();
    assert_eq!(local.expected_release_hash, TOKEN_RAISE_RELEASE);
    assert!(local.bundle.is_some());
}

#[test]
fn template_only_deploy_creates_protocol_release_inputs_and_deployment_then_activates() {
    let dir = project(&escrow_spec_with_resource(), None);
    let expected = load_local_release(dir.path())
        .unwrap()
        .expected_release_hash;
    let fee = write_json(dir.path(), "fee.json", &hosted_fee_policy());
    let limits = write_json(dir.path(), "limits.json", &json!({ "maxDeposits": 5 }));
    let server = responses(vec![
        (200, json!([])),
        (201, protocol_json("escrow-resolution")),
        (201, upload_json(&expected, false, "passed", true)),
        (200, json!([])),
        (
            201,
            json!({ "id": "fee-1", "version": 1, "documentSha256": "00" }),
        ),
        (201, resource_json(1, "draft")),
        (201, resource_json(1, "published")),
        (
            201,
            deployment_json(DEPLOYMENT_ID, "disabled", &expected, "private"),
        ),
        (
            200,
            json!({ "deployment": deployment_json(DEPLOYMENT_ID, "active", &expected, "private") }),
        ),
    ]);
    let mut args = args(dir.path());
    args.fee_policy = Some(fee);
    args.resources = vec![format!("limits={}", limits.display())];

    let outcome = execute(&client(&server), &args, true).unwrap();

    let requests = take(&server, 9);
    let lines: Vec<&str> = requests.iter().map(line).collect();
    let base = format!("/api/protocols/{PROTOCOL_ID}");
    assert_eq!(
        lines,
        vec![
            "GET /api/protocols".to_string(),
            "POST /api/protocols".to_string(),
            format!("POST {base}/releases"),
            format!("GET {base}/deployments"),
            format!("POST {base}/fee-policies"),
            format!("POST {base}/resources/limits/versions"),
            format!("POST {base}/resources/limits/versions/1/publish"),
            format!("POST {base}/deployments"),
            format!("POST /api/protocol-deployments/{DEPLOYMENT_ID}/activate"),
        ]
    );
    assert_eq!(
        requests[0].header("authorization"),
        Some(format!("Bearer {SECRET_KEY}").as_str())
    );
    assert_eq!(
        body(&requests[1]),
        json!({ "slug": "escrow-resolution", "name": "escrowResolution" })
    );
    let upload = body(&requests[2]);
    let spec: Value = serde_json::from_slice(&escrow_spec_with_resource()).unwrap();
    assert_eq!(upload["spec"], spec);
    assert!(upload.get("bundleBase64").is_none());
    assert_eq!(
        body(&requests[4]),
        json!({ "document": hosted_fee_policy() })
    );
    assert_eq!(
        body(&requests[5]),
        json!({ "value": { "maxDeposits": 5 }, "releaseHash": expected })
    );
    assert_eq!(body(&requests[6]), json!({ "releaseHash": expected }));
    assert_eq!(
        body(&requests[7]),
        json!({
            "releaseHash": expected,
            "resources": { "limits": 1 },
            "feePolicyId": "fee-1",
            "accessMode": "private"
        })
    );

    assert_eq!(outcome.status, "active");
    assert_eq!(outcome.protocol.created, Some(true));
    assert_eq!(
        outcome.release.release_hash.as_deref(),
        Some(expected.as_str())
    );
    assert_eq!(outcome.release.created, Some(true));
    let deployment = outcome.deployment.unwrap();
    assert!(deployment.created);
    assert_eq!(deployment.deployment.short_id, "abcdefghij");
    assert!(outcome.warnings.is_empty(), "{:?}", outcome.warnings);
}

#[test]
fn an_existing_deployment_of_the_release_is_reused() {
    let dir = project(&escrow_spec(), None);
    let mut development = deployment_json("cloud-dev", "active", ESCROW_RELEASE, "private");
    development["environmentId"] = json!("denv_test");
    let server = responses(vec![
        (200, json!([protocol_json("escrow-resolution")])),
        (200, upload_json(ESCROW_RELEASE, false, "passed", false)),
        (
            200,
            json!([
                development,
                deployment_json(
                    "other-release",
                    "active",
                    "arete:h1:protocol-release:sha256:ff",
                    "private"
                ),
                deployment_json("public-one", "active", ESCROW_RELEASE, "public"),
                deployment_json("disabled-one", "disabled", ESCROW_RELEASE, "private"),
                deployment_json(DEPLOYMENT_ID, "active", ESCROW_RELEASE, "private"),
            ]),
        ),
    ]);

    let outcome = execute(&client(&server), &args(dir.path()), true).unwrap();

    let lines: Vec<String> = take(&server, 3)
        .iter()
        .map(|r| line(r).to_string())
        .collect();
    assert_eq!(lines[0], "GET /api/protocols");
    assert_eq!(
        lines[1],
        format!("POST /api/protocols/{PROTOCOL_ID}/releases")
    );
    assert_eq!(
        lines[2],
        format!("GET /api/protocols/{PROTOCOL_ID}/deployments")
    );
    assert_eq!(outcome.status, "active");
    assert_eq!(outcome.protocol.created, Some(false));
    assert_eq!(outcome.release.created, Some(false));
    let deployment = outcome.deployment.unwrap();
    assert!(!deployment.created);
    assert_eq!(deployment.deployment.id, DEPLOYMENT_ID);
}

#[test]
fn environment_bound_deployments_are_never_production_reuse_candidates() {
    for environment in [json!("denv_test"), json!("")] {
        let mut value = deployment_json("cloud-dev", "active", ESCROW_RELEASE, "private");
        value["environmentId"] = environment.clone();
        let deployment: ProtocolDeployment = serde_json::from_value(value).unwrap();
        assert_eq!(deployment.environment_id.as_deref(), environment.as_str());
        assert!(select_reusable_deployment(&[deployment], ESCROW_RELEASE).is_none());
    }
    let mut value = deployment_json("production", "disabled", ESCROW_RELEASE, "private");
    value["environmentId"] = Value::Null;
    let deployment: ProtocolDeployment = serde_json::from_value(value).unwrap();
    assert!(select_reusable_deployment(&[deployment], ESCROW_RELEASE).is_some());
}

#[test]
fn a_disabled_reused_deployment_is_activated_and_may_stay_activating() {
    let dir = project(&escrow_spec(), None);
    let server = responses(vec![
        (200, json!([protocol_json("escrow-resolution")])),
        (200, upload_json(ESCROW_RELEASE, false, "pending", false)),
        (
            200,
            json!([deployment_json(
                DEPLOYMENT_ID,
                "disabled",
                ESCROW_RELEASE,
                "private"
            )]),
        ),
        (
            202,
            json!({
                "deployment": deployment_json(DEPLOYMENT_ID, "activating", ESCROW_RELEASE, "private"),
                "pending": { "programSpecs": ["escrowVault"] }
            }),
        ),
    ]);

    let outcome = execute(&client(&server), &args(dir.path()), true).unwrap();

    let last = take(&server, 4).pop().unwrap();
    assert_eq!(
        line(&last),
        format!("POST /api/protocol-deployments/{DEPLOYMENT_ID}/activate")
    );
    assert_eq!(outcome.status, "activating");
    assert_eq!(
        outcome.pending,
        Some(json!({ "programSpecs": ["escrowVault"] }))
    );
    assert!(outcome
        .warnings
        .iter()
        .any(|warning| warning.contains("template check is pending")));
}

#[test]
fn activation_is_retried_while_the_server_prepares_the_deployment() {
    let dir = project(&escrow_spec(), None);
    let activating = json!({
        "deployment": deployment_json(DEPLOYMENT_ID, "activating", ESCROW_RELEASE, "private"),
        "pending": "protocol-stack-deploying"
    });
    let server = responses(vec![
        (200, json!([protocol_json("escrow-resolution")])),
        (200, upload_json(ESCROW_RELEASE, false, "passed", false)),
        (
            200,
            json!([deployment_json(
                DEPLOYMENT_ID,
                "disabled",
                ESCROW_RELEASE,
                "private"
            )]),
        ),
        (202, activating),
        (
            200,
            json!({
                "deployment": deployment_json(DEPLOYMENT_ID, "active", ESCROW_RELEASE, "private")
            }),
        ),
    ]);
    let mut args = args(dir.path());
    args.wait_secs = 30;

    let outcome = execute(&client(&server), &args, true).unwrap();

    let requests = take(&server, 5);
    let activations = requests
        .iter()
        .filter(|request| line(request).ends_with("/activate"))
        .count();
    assert_eq!(activations, 2);
    assert_eq!(outcome.status, "active");
    assert_eq!(outcome.pending, None);
}

#[test]
fn a_handler_release_reports_review_required_and_succeeds() {
    let dir = token_raise();
    fs::write(dir.path().join("fee.json"), fee_policy().to_string()).unwrap();
    // An inline resource value in arete.toml is sent as JSON.
    fs::write(
        dir.path().join("arete.toml"),
        "[protocol]\nslug = \"my-raise\"\nfee_policy = \"fee.json\"\naccess = \"private\"\n\n[protocol.resources]\nlaunchDefaults = { tokenDecimals = 6 }\n",
    )
    .unwrap();
    let server = responses(vec![
        (200, json!([])),
        (201, protocol_json("my-raise")),
        (201, upload_json(TOKEN_RAISE_RELEASE, true, "passed", true)),
        (200, json!([])),
        (
            201,
            json!({ "id": "fee-1", "version": 1, "documentSha256": "00" }),
        ),
        (201, resource_json(3, "draft")),
        (201, resource_json(3, "published")),
        (
            201,
            deployment_json(DEPLOYMENT_ID, "disabled", TOKEN_RAISE_RELEASE, "private"),
        ),
        (
            403,
            json!({
                "schemaVersion": 1,
                "error": "release needs a review",
                "code": "protocol-review-required"
            }),
        ),
    ]);

    let outcome = execute(&client(&server), &args(dir.path()), true).unwrap();

    let requests = take(&server, 9);
    assert_eq!(body(&requests[1])["slug"], "my-raise");
    let bundle = fs::read(fixture("token-raise.handlers.mjs")).unwrap();
    assert_eq!(
        body(&requests[2])["bundleBase64"],
        Value::String(encode_base64(&bundle))
    );
    assert_eq!(
        line(&requests[5]),
        format!("POST /api/protocols/{PROTOCOL_ID}/resources/launchDefaults/versions")
    );
    assert_eq!(body(&requests[5])["value"], json!({ "tokenDecimals": 6 }));
    assert_eq!(
        line(&requests[6]),
        format!("POST /api/protocols/{PROTOCOL_ID}/resources/launchDefaults/versions/3/publish")
    );
    assert_eq!(
        body(&requests[7])["resources"],
        json!({ "launchDefaults": 3 })
    );
    assert_eq!(outcome.status, "review-required");
    assert!(outcome
        .warnings
        .iter()
        .any(|warning| warning.contains("operator reviews it")
            && warning.contains(TOKEN_RAISE_RELEASE)));
    let json = serde_json::to_value(&outcome).unwrap();
    assert_eq!(json["status"], "review-required");
    assert_eq!(json["deployment"]["shortId"], "abcdefghij");
}

#[test]
fn other_activation_failures_are_errors() {
    let dir = project(&escrow_spec(), None);
    let server = responses(vec![
        (200, json!([protocol_json("escrow-resolution")])),
        (200, upload_json(ESCROW_RELEASE, false, "pending", false)),
        (
            200,
            json!([deployment_json(
                DEPLOYMENT_ID,
                "disabled",
                ESCROW_RELEASE,
                "private"
            )]),
        ),
        (
            409,
            json!({ "schemaVersion": 1, "error": "template check pending", "code": "protocol-template-check-pending" }),
        ),
    ]);

    let error = execute(&client(&server), &args(dir.path()), true).unwrap_err();
    assert!(format!("{error:#}").contains("protocol-template-check-pending"));
}

#[test]
fn a_squads_activation_refusal_prints_its_commands() {
    let dir = project(&escrow_spec(), None);
    let add = "a4 dev squads add-proposer --multisig M --proposer P --wallet <a dev wallet that can initiate>";
    let status = format!("a4 protocol squads status {DEPLOYMENT_ID} --dev --multisig M");
    let server = responses(vec![
        (200, json!([protocol_json("escrow-resolution")])),
        (200, upload_json(ESCROW_RELEASE, false, "pending", false)),
        (
            200,
            json!([deployment_json(
                DEPLOYMENT_ID,
                "disabled",
                ESCROW_RELEASE,
                "private"
            )]),
        ),
        (
            409,
            json!({ "schemaVersion": 1, "error": "the Squads proposer is not a member", "code": "protocol-squads-proposer-missing",
                "deploymentId": DEPLOYMENT_ID, "step": "finalize", "multisig": "M", "problem": "not_member",
                "commands": { "addProposer": add, "status": status } }),
        ),
    ]);

    let error = execute(&client(&server), &args(dir.path()), true).unwrap_err();
    let text = format!("{error:#}");
    assert!(
        text.contains(&format!(
            "Add the Squads proposer: `{add}`; then check it: `{status}`."
        )),
        "{text}"
    );
    assert!(
        text.contains("the Squads proposer is not a member"),
        "{text}"
    );
}

#[test]
fn a_failed_template_check_stops_before_any_deployment_call() {
    let dir = project(&escrow_spec(), None);
    let server = responses(vec![
        (200, json!([protocol_json("escrow-resolution")])),
        (201, upload_json(ESCROW_RELEASE, false, "failed", true)),
    ]);

    let error = execute(&client(&server), &args(dir.path()), true).unwrap_err();
    assert!(error.to_string().contains("template check failed"));
}

#[test]
fn a_different_server_release_hash_is_a_warning() {
    let dir = project(&escrow_spec(), None);
    let server_hash = "arete:h1:protocol-release:sha256:abab";
    let server = responses(vec![
        (200, json!([protocol_json("escrow-resolution")])),
        (200, upload_json(server_hash, false, "passed", false)),
        (
            200,
            json!([deployment_json(
                DEPLOYMENT_ID,
                "active",
                server_hash,
                "private"
            )]),
        ),
    ]);

    let outcome = execute(&client(&server), &args(dir.path()), true).unwrap();
    assert_eq!(outcome.status, "active");
    assert_eq!(outcome.release.release_hash.as_deref(), Some(server_hash));
    assert!(outcome.warnings.iter().any(|w| w.contains(ESCROW_RELEASE)));
}

#[test]
fn a_concurrent_protocol_create_is_found_by_listing_again() {
    let dir = project(&escrow_spec(), None);
    let server = responses(vec![
        (200, json!([])),
        (
            409,
            json!({ "schemaVersion": 1, "error": "slug taken", "code": "protocol-slug-taken" }),
        ),
        (200, json!([protocol_json("escrow-resolution")])),
        (200, upload_json(ESCROW_RELEASE, false, "passed", false)),
        (
            200,
            json!([deployment_json(
                DEPLOYMENT_ID,
                "active",
                ESCROW_RELEASE,
                "private"
            )]),
        ),
    ]);

    let outcome = execute(&client(&server), &args(dir.path()), true).unwrap();
    assert_eq!(outcome.protocol.created, Some(false));
    assert_eq!(outcome.protocol.id.as_deref(), Some(PROTOCOL_ID));
}

#[test]
fn a_new_deployment_without_a_fee_policy_explains_the_document() {
    let dir = project(&escrow_spec(), None);
    let server = responses(vec![
        (200, json!([protocol_json("escrow-resolution")])),
        (200, upload_json(ESCROW_RELEASE, false, "passed", false)),
        (200, json!([])),
    ]);

    let error = execute(&client(&server), &args(dir.path()), true).unwrap_err();
    let message = error.to_string();
    assert!(message.contains("--fee-policy"), "{message}");
    assert!(
        message.contains("arete.protocol-fee-policy/v1"),
        "{message}"
    );
    // Arete fills in the platform destination; the CLI never asks for it.
    assert!(!message.contains("<wallet address>"), "{message}");
}

fn check_json(release_hash: &str, template_check: &str) -> Value {
    json!({
        "schema": "arete.protocol-release-check/v1",
        "releaseHash": release_hash,
        "specHash": "arete:h1:protocol-spec:sha256:00",
        "stackSourceHash": "arete:h1:protocol-stack-source:sha256:00",
        "handlerBundleHash": null,
        "hasHandlers": false,
        "templateCheck": template_check
    })
}

#[test]
fn dry_run_with_a_key_checks_the_release_and_otherwise_only_reads() {
    let dir = project(&escrow_spec(), None);
    let server = responses(vec![
        (200, check_json(ESCROW_RELEASE, "passed")),
        (200, json!([protocol_json("escrow-resolution")])),
        (
            200,
            json!([deployment_json(
                DEPLOYMENT_ID,
                "active",
                ESCROW_RELEASE,
                "private"
            )]),
        ),
    ]);
    let mut args = args(dir.path());
    args.dry_run = true;

    let outcome = execute(&client(&server), &args, true).unwrap();

    let requests = take(&server, 3);
    // The check stores nothing; it sends the release body an upload would.
    assert_eq!(line(&requests[0]), "POST /api/protocols/check");
    assert_eq!(
        body(&requests[0])["spec"],
        serde_json::from_slice::<Value>(&escrow_spec()).unwrap()
    );
    assert!(body(&requests[0]).get("bundleBase64").is_none());
    for request in &requests[1..] {
        assert!(
            request.request_line.starts_with("GET "),
            "{}",
            request.request_line
        );
    }
    assert_eq!(outcome.status, "dry-run");
    let plan = outcome.plan.unwrap();
    assert_eq!(
        plan.release_check
            .as_ref()
            .map(|check| check.template_check.as_str()),
        Some("passed")
    );
    assert!(plan.checked_remote);
    assert_eq!(plan.protocol, "exists");
    assert_eq!(plan.deployment, "reuse");
    assert_eq!(plan.reuse_deployment_id.as_deref(), Some(DEPLOYMENT_ID));
    assert_eq!(plan.activation, "already active");
    assert_eq!(outcome.release.expected_release_hash, ESCROW_RELEASE);
}

#[test]
fn a_dry_run_fails_on_a_release_the_server_would_reject() {
    let dir = project(&escrow_spec(), None);
    let server = responses(vec![(
        400,
        json!({
            "schemaVersion": 1,
            "error": "The spec's expressions do not type-check",
            "code": "protocol-spec-check-failed",
            "retryable": false,
            "errors": [{
                "code": "unknown-reference",
                "path": "/steps/open/guards/0/expr",
                "message": "Expression references unknown source field 'args.amout'"
            }]
        }),
    )]);
    let mut args = args(dir.path());
    args.dry_run = true;

    let error = execute(&client(&server), &args, true).unwrap_err();
    let text = format!("{error:#}");
    assert!(text.contains("would reject this release"), "{text}");
    assert!(
        text.contains("/steps/open/guards/0/expr: Expression references unknown source field 'args.amout' (unknown-reference)"),
        "{text}"
    );
    assert_eq!(take(&server, 1).len(), 1);
}

#[test]
fn a_dry_run_against_a_server_without_the_check_still_plans() {
    let dir = project(&escrow_spec(), None);
    let server =
        MockServer::json_sequence(vec![(404, String::new()), (200, json!([]).to_string())]);
    let mut args = args(dir.path());
    args.dry_run = true;

    let outcome = execute(&client(&server), &args, true).unwrap();
    let plan = outcome.plan.unwrap();
    assert!(plan.release_check.is_none());
    assert_eq!(plan.protocol, "create");
    assert!(outcome
        .warnings
        .iter()
        .any(|warning| warning.contains("no release check")));
}

#[test]
fn a_pending_template_check_is_a_dry_run_warning() {
    let dir = project(&escrow_spec(), None);
    let mut pending = check_json(ESCROW_RELEASE, "pending");
    pending["missingProgramSpecs"] =
        json!([{ "alias": "escrowVault", "programSpecHash": "arete:h1:program-spec:sha256:00" }]);
    let server = responses(vec![(200, pending), (200, json!([]))]);
    let mut args = args(dir.path());
    args.dry_run = true;

    let outcome = execute(&client(&server), &args, true).unwrap();
    assert!(outcome
        .warnings
        .iter()
        .any(|warning| warning.contains("'escrowVault'")));
}

#[test]
fn dry_run_without_a_key_works_offline() {
    let dir = token_raise();
    // Nothing listens here; any request would fail the run.
    let client = ApiClient::with_base_url("http://127.0.0.1:9");
    let mut args = args(dir.path());
    args.dry_run = true;

    let outcome = execute(&client, &args, true).unwrap();

    let plan = outcome.plan.unwrap();
    assert!(!plan.checked_remote);
    assert_eq!(plan.deployment, "unknown");
    assert!(plan.activation.contains("operator review"));
    assert_eq!(
        plan.missing_inputs,
        vec![
            "fee policy".to_string(),
            "resource launchDefaults".to_string()
        ]
    );
    assert_eq!(outcome.protocol.slug, "token-raise");
    assert_eq!(outcome.files.len(), 2);
}

#[test]
fn deploy_needs_a_secret_key() {
    let dir = project(&escrow_spec(), None);
    let error = execute(
        &ApiClient::with_base_url("http://127.0.0.1:9"),
        &args(dir.path()),
        true,
    )
    .unwrap_err();
    assert!(error.to_string().contains("secret API key"));

    let publishable =
        ApiClient::with_base_url("http://127.0.0.1:9").with_api_key("a4_pk_browser".into());
    let error = execute(&publishable, &args(dir.path()), true).unwrap_err();
    assert!(error.to_string().contains("publishable"));
}

#[test]
fn a_missing_build_suggests_building() {
    let dir = tempfile::tempdir().unwrap();
    let error = load_local_release(dir.path()).unwrap_err();
    let message = error.to_string();
    assert!(message.contains("arete-protocol build"), "{message}");
    assert!(message.contains("without --no-build"), "{message}");
}

/// The build receipt's source-tree hash, as `arete-protocol build` computes
/// it (`test-vectors`-independent: SHA-256 over the JCS of the pairs).
#[test]
fn the_source_tree_hash_is_sha256_of_the_jcs_pairs() {
    let files = vec![
        BuildSource {
            path: "handlers/a.ts".into(),
            sha256: "00".repeat(32),
        },
        BuildSource {
            path: "protocol.ts".into(),
            sha256: "11".repeat(32),
        },
    ];
    let jcs = format!(
        "[[\"handlers/a.ts\",\"{}\"],[\"protocol.ts\",\"{}\"]]",
        "00".repeat(32),
        "11".repeat(32)
    );
    assert_eq!(
        source_tree_sha256(&files).unwrap(),
        sha256_hex(jcs.as_bytes())
    );
}

#[test]
fn no_build_refuses_a_stale_build_and_names_the_changed_files() {
    let dir = project(&escrow_spec(), None);
    write_build_sources(
        dir.path(),
        &[
            ("protocol.ts", "export default 1;"),
            ("handlers/pay.ts", "export const pay = 1;"),
            ("lib/util.ts", "export const u = 1;"),
        ],
    );
    assert!(changed_sources(dir.path()).unwrap().is_empty());
    check_build_is_current(dir.path()).unwrap();

    fs::write(dir.path().join("protocol.ts"), "export default 2;").unwrap();
    fs::remove_file(dir.path().join("lib/util.ts")).unwrap();
    assert_eq!(
        changed_sources(dir.path()).unwrap(),
        vec![
            ChangedSource {
                path: "protocol.ts".into(),
                change: "changed"
            },
            ChangedSource {
                path: "lib/util.ts".into(),
                change: "deleted"
            },
        ]
    );
    // Refused before anything talks to the server.
    let error = execute(
        &ApiClient::with_base_url("http://127.0.0.1:9").with_api_key(SECRET_KEY.into()),
        &args(dir.path()),
        true,
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("stale"), "{error}");
    assert!(error.contains("protocol.ts (changed)"), "{error}");
    assert!(error.contains("lib/util.ts (deleted)"), "{error}");
    assert!(!error.contains("handlers/pay.ts"), "{error}");
    assert!(error.contains("without --no-build"), "{error}");
}

#[test]
fn no_build_needs_the_source_list_and_a_matching_receipt() {
    let dir = project(&escrow_spec(), None);
    let build = dir.path().join(PROTOCOL_BUILD_DIR);
    fs::remove_file(build.join("build-sources.json")).unwrap();
    let error = changed_sources(dir.path()).unwrap_err().to_string();
    assert!(error.contains("build-sources.json"), "{error}");

    write_build_sources(dir.path(), &[("protocol.ts", "x")]);
    fs::write(
        build.join("build-receipt.json"),
        serde_json::to_vec(&json!({ "sourceTreeSha256": "ab".repeat(32) })).unwrap(),
    )
    .unwrap();
    let error = changed_sources(dir.path()).unwrap_err().to_string();
    assert!(error.contains("does not match"), "{error}");

    fs::remove_file(build.join("build-receipt.json")).unwrap();
    let error = changed_sources(dir.path()).unwrap_err().to_string();
    assert!(error.contains("build-receipt.json"), "{error}");

    write_build_sources(dir.path(), &[]);
    let sources = build.join("build-sources.json");
    let mut value: Value = serde_json::from_slice(&fs::read(&sources).unwrap()).unwrap();
    value["files"] = json!([{ "path": "../outside.ts", "sha256": "00" }]);
    let tree = source_tree_sha256(&[BuildSource {
        path: "../outside.ts".into(),
        sha256: "00".into(),
    }])
    .unwrap();
    value["sourceTreeSha256"] = json!(tree);
    fs::write(&sources, serde_json::to_vec(&value).unwrap()).unwrap();
    fs::write(
        build.join("build-receipt.json"),
        serde_json::to_vec(&json!({ "sourceTreeSha256": tree })).unwrap(),
    )
    .unwrap();
    let error = changed_sources(dir.path()).unwrap_err().to_string();
    assert!(error.contains("outside the project"), "{error}");
}

#[test]
fn the_bundle_must_match_the_handler_steps() {
    let stale = project(&escrow_spec(), Some(b"export const x = 1;"));
    assert!(load_local_release(stale.path())
        .unwrap_err()
        .to_string()
        .contains("no handler steps"));

    let missing = project(
        &fs::read(fixture("token-raise.protocol-spec.json")).unwrap(),
        None,
    );
    assert!(load_local_release(missing.path())
        .unwrap_err()
        .to_string()
        .contains("handlers.mjs is missing"));
}

#[test]
fn slugs_derive_from_spec_names() {
    assert_eq!(
        derive_slug("escrowResolution").unwrap(),
        "escrow-resolution"
    );
    assert_eq!(derive_slug("tokenRaise").unwrap(), "token-raise");
    assert_eq!(derive_slug("My Protocol_v2").unwrap(), "my-protocol-v2");
    assert_eq!(derive_slug("--Weird!!Name--").unwrap(), "weird-name");
    assert_eq!(derive_slug("ABC").unwrap(), "abc");
    assert_eq!(derive_slug(&"a".repeat(80)).unwrap().len(), 63);
    assert!(derive_slug("!!!").is_err());

    assert!(validate_slug("escrow-1").is_ok());
    assert!(validate_slug("-escrow").is_err());
    assert!(validate_slug("Escrow").is_err());
    assert!(validate_slug(&"a".repeat(64)).is_err());
}

#[test]
fn flags_override_the_protocol_config() {
    let config = parse_protocol_config(
        "manifest_version = 1\n[project]\nname = \"p\"\n\n[protocol]\nslug = \"from-config\"\nname = \"Config Name\"\nfee_policy = \"fees/policy.json\"\n\n[protocol.resources]\nlaunchDefaults = \"res/defaults.json\"\n",
    )
    .unwrap();
    assert_eq!(config.slug.as_deref(), Some("from-config"));
    let spec = validate_protocol_spec_value(
        &serde_json::from_slice(&fs::read(fixture("token-raise.protocol-spec.json")).unwrap())
            .unwrap(),
    )
    .unwrap();

    let base = DeployArgs {
        dir: PathBuf::from("proj"),
        ..DeployArgs::default()
    };
    let settings = resolve_settings(&base, &config, &spec).unwrap();
    assert_eq!(settings.slug, "from-config");
    assert_eq!(settings.name, "Config Name");
    assert_eq!(
        settings.fee_policy,
        Some(PathBuf::from("proj/fees/policy.json"))
    );
    assert_eq!(
        settings.resources.get("launchDefaults"),
        Some(&ResourceSource::File(PathBuf::from(
            "proj/res/defaults.json"
        )))
    );

    let flags = DeployArgs {
        slug: Some("from-flag".into()),
        fee_policy: Some(PathBuf::from("other.json")),
        resources: vec!["launchDefaults=cli.json".into()],
        ..base.clone()
    };
    let settings = resolve_settings(&flags, &config, &spec).unwrap();
    assert_eq!(settings.slug, "from-flag");
    assert_eq!(settings.fee_policy, Some(PathBuf::from("other.json")));
    assert_eq!(
        settings.resources.get("launchDefaults"),
        Some(&ResourceSource::File(PathBuf::from("cli.json")))
    );

    let undeclared = DeployArgs {
        resources: vec!["nope=x.json".into()],
        ..base.clone()
    };
    assert!(resolve_settings(&undeclared, &config, &spec)
        .unwrap_err()
        .to_string()
        .contains("not declared"));
    let bad_slug = DeployArgs {
        slug: Some("Bad Slug".into()),
        ..base
    };
    assert!(resolve_settings(&bad_slug, &config, &spec).is_err());
}

#[test]
fn protocol_resources_are_a_file_path_or_an_inline_value() {
    let config = parse_protocol_config(
        "[protocol.resources]\nfile = \"res/defaults.json\"\ntable = { tokenDecimals = 6, tiers = [1, 2], label = \"x\" }\nnumber = 5\n\n[protocol.resources.nested]\nenabled = true\n",
    )
    .unwrap();
    assert_eq!(
        config.resources["file"],
        ResourceSource::File(PathBuf::from("res/defaults.json"))
    );
    let table = json!({ "tokenDecimals": 6, "tiers": [1, 2], "label": "x" });
    assert_eq!(
        config.resources["table"],
        ResourceSource::Inline(table.clone())
    );
    assert_eq!(config.resources["number"], ResourceSource::Inline(json!(5)));
    assert_eq!(
        config.resources["nested"],
        ResourceSource::Inline(json!({ "enabled": true }))
    );

    // File paths resolve against the project; inline values are kept.
    let project = Path::new("proj");
    assert_eq!(
        config.resources["file"].relative_to(project),
        ResourceSource::File(PathBuf::from("proj/res/defaults.json"))
    );
    assert_eq!(
        config.resources["table"]
            .relative_to(project)
            .read("table")
            .unwrap(),
        table
    );

    // A file is read and checked as JSON.
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("ok.json"), r#"{"tokenDecimals":6}"#).unwrap();
    fs::write(dir.path().join("bad.json"), "{ not json").unwrap();
    let file = ResourceSource::File(PathBuf::from("ok.json")).relative_to(dir.path());
    assert_eq!(file.read("d").unwrap(), json!({ "tokenDecimals": 6 }));
    let bad = ResourceSource::File(PathBuf::from("bad.json")).relative_to(dir.path());
    let error = format!("{:#}", bad.read("d").unwrap_err());
    assert!(
        error.contains("Resource d") && error.contains("bad.json"),
        "{error}"
    );
}

#[test]
fn invalid_protocol_config_reports_the_field_and_reason() {
    let error = parse_protocol_config("[protocol.resources]\nwhen = 2026-10-23\n").unwrap_err();
    let error = format!("{error:#}");
    assert!(error.contains("[protocol.resources].when"), "{error}");
    assert!(error.contains("TOML dates"), "{error}");
    let error = parse_protocol_config("[protocol.resources]\nempty = \"\"\n").unwrap_err();
    assert!(
        format!("{error:#}").contains("[protocol.resources].empty"),
        "{error:#}"
    );

    // The top-level handler prints only the outermost message, so it has to
    // carry the cause.
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("arete.toml"),
        "[protocol]\nfee_policy = 5\n",
    )
    .unwrap();
    let error = load_protocol_config(dir.path()).unwrap_err().to_string();
    assert!(error.contains("arete.toml"), "{error}");
    assert!(error.contains("fee_policy"), "{error}");
    assert!(error.contains("invalid type"), "{error}");
    fs::write(
        dir.path().join("arete.toml"),
        "[protocol.resources]\nx = 2026-10-23\n",
    )
    .unwrap();
    let error = load_protocol_config(dir.path()).unwrap_err().to_string();
    assert!(error.contains("[protocol.resources].x"), "{error}");
}

#[test]
fn a_missing_resource_names_both_config_forms() {
    let help = missing_resource_help("launchDefaults");
    assert!(
        help.contains("--resource launchDefaults=<file.json>"),
        "{help}"
    );
    assert!(
        help.contains("launchDefaults = \"resources/launchDefaults.json\""),
        "{help}"
    );
    assert!(
        help.contains("launchDefaults = { someField = 1 }"),
        "{help}"
    );

    let dir = token_raise();
    let local = load_local_release(dir.path()).unwrap();
    let settings = Settings {
        slug: "token-raise".into(),
        name: "Token Raise".into(),
        fee_policy: Some(write_json(dir.path(), "fee.json", &fee_policy())),
        resources: BTreeMap::new(),
    };
    let client = ApiClient::with_base_url("http://127.0.0.1:9");
    let upload: ProtocolReleaseUpload =
        serde_json::from_value(upload_json(TOKEN_RAISE_RELEASE, true, "passed", true)).unwrap();
    let reporter = Reporter {
        json: true,
        warnings: Vec::new(),
    };
    let error = create_deployment(&client, PROTOCOL_ID, &upload, &local, &settings, &reporter)
        .unwrap_err()
        .to_string();
    assert_eq!(error, help);
}

#[test]
fn protocol_config_is_optional_strict_and_private_only() {
    assert_eq!(
        parse_protocol_config("manifest_version = 1\n").unwrap(),
        ProtocolConfig::default()
    );
    assert!(load_protocol_config(tempfile::tempdir().unwrap().path())
        .unwrap()
        .slug
        .is_none());
    let error = parse_protocol_config("[protocol]\nslugg = \"typo\"\n").unwrap_err();
    assert!(format!("{error:#}").contains("slugg"), "{error:#}");
    let error = parse_protocol_config("[protocol]\naccess = \"public\"\n").unwrap_err();
    assert!(error.to_string().contains("operator"), "{error}");
}

#[test]
fn other_commands_accept_a_protocol_table_in_the_manifest() {
    let source = "manifest_version = 1\n[project]\nname = \"p\"\n\n[protocol]\nslug = \"x\"\nfuture_key = 1\n";
    let manifest: crate::project::manifest::ManifestV1 = toml::from_str(source).unwrap();
    manifest.validate().unwrap();
    let plain: crate::project::manifest::ManifestV1 =
        toml::from_str("manifest_version = 1\n[project]\nname = \"p\"\n").unwrap();
    assert_eq!(
        manifest.resolution_hash().unwrap(),
        plain.resolution_hash().unwrap()
    );
}

fn actions_args(target: Option<&str>, wallet: &str) -> ActionsArgs {
    ActionsArgs {
        target: target.map(str::to_string),
        wallet: wallet.to_string(),
        ..ActionsArgs::default()
    }
}

#[test]
fn actions_targets_are_protocols_or_deployments() {
    let target = actions_target(&actions_args(Some("token-raise"), "W")).unwrap();
    assert_eq!(target.protocol.as_deref(), Some("token-raise"));
    assert_eq!(target.deployment, None);
    assert!(!target.dev);
    let target = actions_target(&actions_args(Some(DEPLOYMENT_ID), "W")).unwrap();
    assert_eq!(target.deployment.as_deref(), Some(DEPLOYMENT_ID));
    assert!(actions_target(&actions_args(None, "W")).is_err());
    let target = actions_target(&ActionsArgs {
        dev: true,
        ..actions_args(None, "W")
    })
    .unwrap();
    assert!(target.dev);
    let target = actions_target(&ActionsArgs {
        project: Some("app".into()),
        ..actions_args(None, "W")
    })
    .unwrap();
    assert!(target.dev);
    assert_eq!(target.project.as_deref(), Some("app"));
}

#[test]
fn actions_are_read_and_rendered() {
    let wallet = "9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM";
    let answer = json!({
        "deploymentId": DEPLOYMENT_ID,
        "wallet": wallet,
        "createSteps": [{ "step": "open", "allowed": true, "scope": "create",
            "args": [{ "name": "goal", "type": "string", "format": "u64", "optional": false },
                     { "name": "memo", "type": "string", "optional": true },
                     { "name": "members", "type": "array", "optional": false,
                       "items": { "type": "string", "format": "pubkey" } }] }],
        "instances": [{
            "instanceId": "i-1", "state": "raising", "relation": ["participant", "role:admin"],
            "participantState": "contributed",
            "steps": [
                { "step": "contribute", "allowed": true, "scope": "participant", "args": [],
                  "argsDependent": ["under_cap"] },
                { "step": "refund", "allowed": false, "reason": "raise_not_failed", "scope": "participant", "args": [] }
            ]
        }],
        "nextCursor": "c-2",
    });
    let server = MockServer::json_sequence(vec![
        (200, json!({ "token": "session" }).to_string()),
        (200, answer.to_string()),
    ]);
    let agent =
        arete_mcp::protocol_agent::ProtocolAgent::new(None).with_api(server.base_url(), SECRET_KEY);
    let result = fetch_actions(
        &agent,
        &ActionsArgs {
            instance: Some("i-1".into()),
            ..actions_args(Some(DEPLOYMENT_ID), wallet)
        },
    )
    .unwrap();
    let mint: ReceivedRequest = server.request();
    assert_eq!(mint.request_line, "POST /ws/sessions HTTP/1.1");
    assert_eq!(
        mint.header("authorization"),
        Some(format!("Bearer {SECRET_KEY}").as_str())
    );
    let mint_body: Value = serde_json::from_str(&mint.body).unwrap();
    assert_eq!(mint_body["scopes"], json!(["protocol:read"]));
    assert_eq!(mint_body["targetId"], DEPLOYMENT_ID);
    let read = server.request();
    assert_eq!(
        read.request_line,
        format!("GET /api/protocol-deployments/{DEPLOYMENT_ID}/wallets/{wallet}/actions?instance=i-1 HTTP/1.1")
    );
    assert_eq!(read.header("authorization"), Some("Bearer session"));
    assert_eq!(result["createSteps"][0]["step"], "open");

    colored::control::set_override(false);
    let text = render_actions(&result);
    assert!(
        text.contains(&format!("Wallet {wallet} on deployment {DEPLOYMENT_ID}")),
        "{text}"
    );
    assert!(
        text.contains("open  args { goal: u64, memo?: string, members: pubkey[] }"),
        "{text}"
    );
    assert!(
        text.contains("Instance i-1 [raising] participant, role:admin (participant: contributed)"),
        "{text}"
    );
    assert!(
        text.contains("contribute  decided at prepare: under_cap"),
        "{text}"
    );
    assert!(text.contains("refund (raise_not_failed)"), "{text}");
    assert!(text.contains("--cursor c-2"), "{text}");
}
