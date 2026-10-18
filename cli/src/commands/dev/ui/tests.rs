//! The dev UI server over real HTTP on an ephemeral loopback port.

use super::*;
use base64::Engine as _;

fn serve() -> (DevUi, String, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let ui = start(
        listener,
        UiOptions {
            project: DevProject::new(dir.path()),
            ui_dev_url: None,
            api_key: Some("a4_sk_test".into()),
        },
    )
    .unwrap();
    let base = ui.url.as_deref().unwrap().trim_end_matches('/').to_string();
    (ui, base, dir)
}

async fn get_json(url: &str) -> (u16, Value) {
    let response = reqwest::Client::new()
        .get(url)
        .header(header::ACCEPT, "application/json")
        .send()
        .await
        .unwrap();
    let status = response.status().as_u16();
    (status, response.json().await.unwrap_or(Value::Null))
}

#[tokio::test]
async fn serves_the_protocol_without_keys() {
    let (ui, base, dir) = serve();
    std::fs::create_dir_all(dir.path().join(".arete/dev/wallets")).unwrap();
    let mut keypair = vec![7u8; 32];
    keypair.extend_from_slice(&[9u8; 32]);
    std::fs::write(
        dir.path().join(".arete/dev/wallets/alice.json"),
        serde_json::to_vec(&keypair).unwrap(),
    )
    .unwrap();
    let build = dir.path().join("build");
    std::fs::create_dir_all(&build).unwrap();
    std::fs::write(
        build.join("protocol-spec.json"),
        json!({ "name": "jar", "version": "0.1.0", "steps": {}, "machines": {} }).to_string(),
    )
    .unwrap();
    ui.build_succeeded(&build);
    ui.set_urls(json!({ "api": "http://127.0.0.1:9/api" }));

    let (status, body) = get_json(&format!("{base}/dev/v1/protocol")).await;
    assert_eq!(status, 200);
    assert_eq!(body["spec"]["name"], "jar");
    assert_eq!(body["build"]["state"], "ok");
    assert_eq!(body["build"]["diff"]["initial"], true);
    assert_eq!(body["wallets"][0]["name"], "alice");
    assert_eq!(
        body["wallets"][0]["pubkey"],
        bs58::encode([9u8; 32]).into_string()
    );
    assert_eq!(body["urls"]["ui"], format!("{base}/"));
    let text = body.to_string();
    assert!(
        !text.contains("a4_sk_test") && !text.contains("[7,7"),
        "{text}"
    );
}

#[tokio::test]
async fn refuses_foreign_hosts_and_origins() {
    let (_ui, base, _dir) = serve();
    let client = reqwest::Client::new();
    let rebound = client
        .get(format!("{base}/dev/v1/protocol"))
        .header(header::HOST, "evil.example")
        .send()
        .await
        .unwrap();
    assert_eq!(rebound.status().as_u16(), 421);
    let body: Value = rebound.json().await.unwrap();
    assert_eq!(body["code"], "dev-ui-host-refused");

    let cross = client
        .get(format!("{base}/dev/v1/events"))
        .header(header::ORIGIN, "https://evil.example")
        .send()
        .await
        .unwrap();
    assert_eq!(cross.status().as_u16(), 403);
    assert_eq!(cross.headers()[header::X_FRAME_OPTIONS], "DENY");
    assert!(cross.headers().get("access-control-allow-origin").is_none());
}

#[tokio::test]
async fn pages_events_after_a_cursor_and_waits_for_new_ones() {
    let (ui, base, _dir) = serve();
    ui.state
        .hub
        .publish("build.started", json!({ "changed": [] }));
    ui.state.hub.publish("build.succeeded", json!({}));

    let (status, page) = get_json(&format!("{base}/dev/v1/events")).await;
    assert_eq!(status, 200);
    assert_eq!(page["events"].as_array().unwrap().len(), 2);
    assert_eq!(page["events"][0]["type"], "build.started");
    let cursor = page["cursor"].as_str().unwrap().to_string();
    assert!(cursor.ends_with(":2"), "{cursor}");

    let (_, empty) = get_json(&format!("{base}/dev/v1/events?after={cursor}")).await;
    assert!(empty["events"].as_array().unwrap().is_empty());
    assert_eq!(empty["cursor"], cursor.as_str());

    let hub = ui.state.hub.clone();
    let publisher = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        hub.publish_ephemeral("chain.clock", json!({ "slot": 1 }));
        hub.publish("step.confirmed", json!({ "step": "open" }));
    });
    let (_, waited) = get_json(&format!("{base}/dev/v1/events?after={cursor}&wait=5000")).await;
    publisher.await.unwrap();
    assert_eq!(waited["events"].as_array().unwrap().len(), 1);
    assert_eq!(waited["events"][0]["type"], "step.confirmed");

    let (status, _) = get_json(&format!("{base}/dev/v1/events?after=nonsense")).await;
    assert_eq!(status, 400);
}

/// Read SSE events until `until` names one, or a timeout.
async fn read_sse(
    mut response: reqwest::Response,
    until: &str,
) -> Vec<(String, Option<String>, Value)> {
    let mut buffer = String::new();
    let mut out = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while let Ok(Ok(Some(chunk))) = tokio::time::timeout_at(deadline, response.chunk()).await {
        buffer.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(end) = buffer.find("\n\n") {
            let block: String = buffer.drain(..end + 2).collect();
            let (mut kind, mut id, mut data) = (String::new(), None, String::new());
            for line in block.lines() {
                if let Some(value) = line.strip_prefix("event:") {
                    kind = value.trim().to_string();
                } else if let Some(value) = line.strip_prefix("id:") {
                    id = Some(value.trim().to_string());
                } else if let Some(value) = line.strip_prefix("data:") {
                    data.push_str(value.trim());
                }
            }
            if kind.is_empty() {
                continue;
            }
            let done = kind == until;
            out.push((kind, id, serde_json::from_str(&data).unwrap_or(Value::Null)));
            if done {
                return out;
            }
        }
    }
    out
}

#[tokio::test]
async fn streams_replays_and_resumes_with_last_event_id() {
    let (ui, base, _dir) = serve();
    let hub = ui.state.hub.clone();
    hub.publish("build.started", json!({}));
    hub.publish("build.succeeded", json!({}));

    let client = reqwest::Client::new();
    let response = client
        .get(format!("{base}/dev/v1/events"))
        .header(header::ACCEPT, "text/event-stream")
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "text/event-stream"
    );
    let publisher = {
        let hub = hub.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            hub.publish("deploy.activated", json!({ "deploymentId": "d1" }));
        })
    };
    let events = read_sse(response, "deploy.activated").await;
    publisher.await.unwrap();
    let kinds: Vec<&str> = events.iter().map(|(kind, _, _)| kind.as_str()).collect();
    assert_eq!(
        kinds,
        [
            "build.started",
            "build.succeeded",
            "stream.live",
            "deploy.activated"
        ]
    );
    let second_id = events[1].1.clone().unwrap();
    assert_eq!(events[3].2["data"]["deploymentId"], "d1");

    // Resume after the second event: only the third comes back.
    let resumed = client
        .get(format!("{base}/dev/v1/events"))
        .header("last-event-id", &second_id)
        .send()
        .await
        .unwrap();
    let events = read_sse(resumed, "stream.live").await;
    let kinds: Vec<&str> = events.iter().map(|(kind, _, _)| kind.as_str()).collect();
    assert_eq!(kinds, ["deploy.activated", "stream.live"]);

    // A cursor from another run starts over with a reset.
    let other = client
        .get(format!("{base}/dev/v1/events"))
        .header("last-event-id", "0000:2")
        .send()
        .await
        .unwrap();
    let events = read_sse(other, "stream.live").await;
    assert_eq!(events[0].0, "stream.reset");
    assert_eq!(events.len(), 5);
}

#[tokio::test]
async fn serves_the_ui_or_the_placeholder_at_the_root() {
    let (_ui, base, _dir) = serve();
    let client = reqwest::Client::new();
    let root = client.get(format!("{base}/")).send().await.unwrap();
    assert_eq!(root.status().as_u16(), 200);
    assert!(root.headers()[header::CONTENT_TYPE]
        .to_str()
        .unwrap()
        .starts_with("text/html"));
    let page = root.text().await.unwrap();
    assert!(page.contains("a4 dev"), "{page}");
    // Unknown API routes are JSON 404s, not the UI.
    let missing = client
        .get(format!("{base}/dev/v1/nope"))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status().as_u16(), 404);
    if assets::built() {
        // A client route gets index.html; a missing file is a 404.
        assert_eq!(
            assets::lookup("/instances/x").unwrap().0,
            "text/html; charset=utf-8"
        );
        assert!(assets::lookup("/assets/missing.js").is_none());
        let gzipped = client
            .get(format!("{base}/"))
            .header(header::ACCEPT_ENCODING, "gzip")
            .send()
            .await
            .unwrap();
        assert_eq!(gzipped.headers()[header::CONTENT_ENCODING], "gzip");
    } else {
        assert!(assets::lookup("/").is_none());
    }
}

#[tokio::test]
async fn run_routes_refuse_cross_site_writes_and_wait_for_a_session() {
    let (_ui, base, _dir) = serve();
    let client = reqwest::Client::new();
    let body = json!({ "step": "open", "args": {}, "wallet": "alice" });
    // Another origin can't run steps with the dev wallets.
    let cross = client
        .post(format!("{base}/dev/v1/steps/run"))
        .header(header::ORIGIN, "https://evil.example")
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(cross.status().as_u16(), 403);
    // A form post (no JSON content type) is refused before any handler.
    let form = client
        .post(format!("{base}/dev/v1/time-travel"))
        .header(header::CONTENT_TYPE, "text/plain")
        .body("{\"to\":\"+1h\"}")
        .send()
        .await
        .unwrap();
    assert_eq!(form.status().as_u16(), 415);
    // The UI's own request reaches the handler, which has no session yet.
    let own = client
        .post(format!("{base}/dev/v1/steps/run"))
        .header(header::ORIGIN, base.as_str())
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(own.status().as_u16(), 503);
    let error: Value = own.json().await.unwrap();
    assert_eq!(error["code"], "dev-ui-session-unavailable");
    // Bad input is a 400 before the session is needed.
    let bad = client
        .post(format!("{base}/dev/v1/airdrop"))
        .json(&json!({ "pubkey": "nope" }))
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status().as_u16(), 400);
    let submit = client
        .post(format!("{base}/dev/v1/steps/submit"))
        .json(&json!({ "executionId": "missing" }))
        .send()
        .await
        .unwrap();
    assert_eq!(submit.status().as_u16(), 404);
}

/// The browser wallet proxy prepares and submits with the account's key,
/// so only the UI's own page may reach it.
#[tokio::test]
async fn the_wallet_proxy_only_answers_the_ui_origin() {
    let (_ui, base, _dir) = serve();
    let port = base.rsplit(':').next().unwrap().to_string();
    let client = reqwest::Client::new();
    let prepare =
        json!({ "step": "open", "args": {}, "pubkey": "11111111111111111111111111111111" });
    let submit = json!({ "executionId": "e1", "signedTransactions": [] });
    let post = |path: &'static str, body: &Value| client.post(format!("{base}{path}")).json(body);
    let code = |response: reqwest::Response| async move {
        let status = response.status().as_u16();
        let body: Value = response.json().await.unwrap_or(Value::Null);
        (
            status,
            body["code"].as_str().unwrap_or_default().to_string(),
        )
    };
    for (path, body) in [
        ("/dev/v1/wallet/prepare", &prepare),
        ("/dev/v1/wallet/submit", &submit),
    ] {
        // A page on another site.
        let (status, refused) = code(
            post(path, body)
                .header(header::ORIGIN, "https://evil.example")
                .send()
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(
            (status, refused.as_str()),
            (403, "dev-ui-origin-refused"),
            "{path}"
        );
        // A page on another local port (same site, different origin).
        let (status, _) = code(
            post(path, body)
                .header(header::ORIGIN, "http://localhost:3000")
                .send()
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, 403, "{path}");
        // DNS rebinding: the right port under another name.
        let (status, refused) = code(
            post(path, body)
                .header(header::HOST, format!("rebound.example:{port}"))
                .send()
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(
            (status, refused.as_str()),
            (421, "dev-ui-host-refused"),
            "{path}"
        );
        // A no-cors form post.
        let (status, _) = code(
            client
                .post(format!("{base}{path}"))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body("step=open")
                .send()
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, 415, "{path}");
        // No CORS preflight succeeds.
        let preflight = client
            .request(reqwest::Method::OPTIONS, format!("{base}{path}"))
            .header(header::ORIGIN, "https://evil.example")
            .header("access-control-request-method", "POST")
            .send()
            .await
            .unwrap();
        assert!(preflight.status().is_client_error());
        assert!(preflight
            .headers()
            .get("access-control-allow-origin")
            .is_none());
    }
    // The UI's own page gets through to the handlers.
    let (status, refused) = code(
        post(
            "/dev/v1/wallet/prepare",
            &json!({ "step": "open", "pubkey": "nope" }),
        )
        .header(header::ORIGIN, base.as_str())
        .send()
        .await
        .unwrap(),
    )
    .await;
    assert_eq!((status, refused.as_str()), (400, "dev-ui-bad-request"));
    let (status, refused) = code(
        post("/dev/v1/wallet/submit", &submit)
            .header(header::ORIGIN, base.as_str())
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!((status, refused.as_str()), (404, "dev-ui-not-prepared"));
}

#[test]
fn an_empty_page_resumes_from_the_head_taken_with_it() {
    let hub = EventHub::new();
    hub.publish("build.started", json!({}));
    let head = hub.since(None, 10);
    assert_eq!(head.events.len(), 1);
    let cursor = Cursor::parse(&hub.event_id(&head.events[0])).unwrap();

    // The replay is empty; an event lands before the page is written.
    let replay = hub.since(Some(&cursor), 10);
    assert!(replay.events.is_empty());
    hub.publish("build.succeeded", json!({}));
    let page = page_from(&hub, replay, Some(&cursor));
    assert_eq!(page["cursor"], format!("{}:1", hub.boot()));

    // The same with no cursor at all: the next read still gets seq 2.
    let replay = hub.since(None, 0);
    hub.publish("deploy.activated", json!({}));
    let page = page_from(&hub, replay, None);
    let next = Cursor::parse(page["cursor"].as_str().unwrap()).unwrap();
    assert_eq!(next.seq, 2);
    let after = hub.since(Some(&next), 10);
    assert_eq!(after.events.len(), 1);
    assert_eq!(after.events[0].kind, "deploy.activated");
}

#[tokio::test]
async fn a_build_without_a_spec_fails_visibly_and_the_next_build_recovers() {
    let (ui, base, dir) = serve();
    let build = dir.path().join("build");
    std::fs::create_dir_all(&build).unwrap();
    ui.build_started(&[]);
    ui.build_succeeded(&build);

    let (status, body) = get_json(&format!("{base}/dev/v1/protocol")).await;
    assert_eq!(status, 200);
    assert_eq!(body["build"]["state"], "failed", "{body:#}");
    let error = body["build"]["error"].as_str().unwrap();
    assert!(error.contains("protocol-spec.json"), "{error}");
    assert!(!body["build"]["diagnostics"].as_array().unwrap().is_empty());
    let kinds: Vec<String> = ui
        .state
        .hub
        .since(None, 10)
        .events
        .into_iter()
        .map(|event| event.kind)
        .collect();
    assert_eq!(kinds, ["build.started", "build.failed"]);

    // Unparsable JSON is the same failure.
    std::fs::write(build.join("protocol-spec.json"), "{ not json").unwrap();
    ui.build_succeeded(&build);
    let (_, body) = get_json(&format!("{base}/dev/v1/protocol")).await;
    assert_eq!(body["build"]["state"], "failed");
    assert!(body["build"]["error"]
        .as_str()
        .unwrap()
        .contains("not valid JSON"));

    std::fs::write(
        build.join("protocol-spec.json"),
        json!({ "name": "jar", "version": "0.1.0", "steps": {}, "machines": {} }).to_string(),
    )
    .unwrap();
    ui.build_succeeded(&build);
    let (_, body) = get_json(&format!("{base}/dev/v1/protocol")).await;
    assert_eq!(body["build"]["state"], "ok");
    assert_eq!(body["spec"]["name"], "jar");
}

/// The Arete API for the browser wallet path: the session endpoint, a
/// prepare that returns `transaction`, and a submit that fails with 503
/// `fail_submits` times before it completes.
struct FakeRuntime {
    transaction: std::sync::Mutex<Vec<u8>>,
    fail_submits: std::sync::atomic::AtomicUsize,
    submitted: std::sync::Mutex<Vec<Vec<String>>>,
}

async fn fake_runtime(transaction: Vec<u8>, fail_submits: usize) -> (Arc<FakeRuntime>, String) {
    use axum::routing::post;
    let fake = Arc::new(FakeRuntime {
        transaction: std::sync::Mutex::new(transaction),
        fail_submits: std::sync::atomic::AtomicUsize::new(fail_submits),
        submitted: std::sync::Mutex::new(Vec::new()),
    });
    let app = Router::new()
        .route(
            "/ws/sessions",
            post(|| async { Json(json!({ "token": "t" })) }),
        )
        .route(
            "/api/protocol-deployments/:deployment/steps/:step/prepare",
            post(|State(fake): State<Arc<FakeRuntime>>| async move {
                let bytes = fake.transaction.lock().unwrap().clone();
                Json(json!({
                    "intent": { "executionId": "e1", "instanceId": null, "state": "prepared" },
                    "transactions": [{ "index": 0, "transactionBase64":
                        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes) }],
                }))
            }),
        )
        .route(
            "/api/execution-intents/:id/submit",
            post(
                |State(fake): State<Arc<FakeRuntime>>, Json(body): Json<Value>| async move {
                    use std::sync::atomic::Ordering;
                    if fake
                        .fail_submits
                        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |left| left.checked_sub(1))
                        .is_ok()
                    {
                        return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "error": "busy" })))
                            .into_response();
                    }
                    let signed: Vec<String> =
                        serde_json::from_value(body["signedTransactions"].clone()).unwrap();
                    fake.submitted.lock().unwrap().push(signed);
                    Json(json!({ "executionId": "e1", "state": "completed",
                        "transactions": [{ "index": 0, "state": "confirmed" }] }))
                    .into_response()
                },
            ),
        )
        .with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (fake, url)
}

/// The project's environment on `api_url`, and a running session with
/// deployment `d1`.
fn write_session(dir: &std::path::Path, api_url: &str) {
    let project = DevProject::new(dir);
    let selection =
        arete_mcp::dev_env::EnvSelection::new("denv_01hzx3k8m9q2r4s5t6v7w8x9yz", "app", api_url);
    selection.save(&project.environment_path()).unwrap();
    let mut session = arete_mcp::dev::DevSession::offline(&selection);
    session.rpc_url = "http://127.0.0.1:9".into();
    session.rpc_ws_url = "ws://127.0.0.1:10".into();
    session.deployment_id = Some("d1".into());
    arete_mcp::dev::write_private(
        &project.session_path(),
        &serde_json::to_vec(&session).unwrap(),
    )
    .unwrap();
}

async fn post_json(base: &str, path: &str, body: Value) -> (u16, Value) {
    let response = reqwest::Client::new()
        .post(format!("{base}{path}"))
        .header(header::ORIGIN, base)
        .json(&body)
        .send()
        .await
        .unwrap();
    let status = response.status().as_u16();
    (status, response.json().await.unwrap_or(Value::Null))
}

#[tokio::test]
async fn a_browser_wallet_signs_and_submits_a_prepared_step() {
    use ed25519_dalek::SigningKey;
    let platform = SigningKey::from_bytes(&[1; 32]);
    let wallet_key = SigningKey::from_bytes(&[2; 32]);
    let pubkey = bs58::encode(wallet_key.verifying_key().as_bytes()).into_string();
    let prepared = wallet::tests::sign_slot(
        &wallet::tests::transaction(&[&platform, &wallet_key]),
        &platform,
        0,
    );
    let (fake, api_url) = fake_runtime(prepared.clone(), 1).await;
    let (ui, base, dir) = serve();
    write_session(dir.path(), &api_url);

    let (status, preview) = post_json(
        &base,
        "/dev/v1/wallet/prepare",
        json!({ "step": "open", "args": {}, "pubkey": pubkey }),
    )
    .await;
    assert_eq!(status, 200, "{preview:#}");
    assert_eq!(preview["executionId"], "e1");
    let unsigned = preview["transactions"][0]["transactionBase64"]
        .as_str()
        .unwrap();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(unsigned)
        .unwrap();
    assert_eq!(bytes, prepared);
    let signed = base64::engine::general_purpose::STANDARD.encode(wallet::tests::sign_slot(
        &bytes,
        &wallet_key,
        1,
    ));

    // Bad input and a wrong signature keep the step prepared.
    let (status, _) = post_json(
        &base,
        "/dev/v1/wallet/submit",
        json!({ "executionId": "e1", "signedTransactions": ["%%%"] }),
    )
    .await;
    assert_eq!(status, 400);
    assert!(ui.state.pending.contains("e1"));
    let (status, refused) = post_json(
        &base,
        "/dev/v1/wallet/submit",
        json!({ "executionId": "e1", "signedTransactions": [unsigned] }),
    )
    .await;
    assert_eq!(status, 400);
    assert!(refused["error"]
        .as_str()
        .unwrap()
        .contains("does not verify"));
    assert!(ui.state.pending.contains("e1"));

    // The runtime is busy: the submit never happened, so the step stays.
    let (status, _) = post_json(
        &base,
        "/dev/v1/wallet/submit",
        json!({ "executionId": "e1", "signedTransactions": [signed] }),
    )
    .await;
    assert_eq!(status, 502);
    assert!(ui.state.pending.contains("e1"));

    let (status, result) = post_json(
        &base,
        "/dev/v1/wallet/submit",
        json!({ "executionId": "e1", "signedTransactions": [signed] }),
    )
    .await;
    assert_eq!(status, 200, "{result:#}");
    assert_eq!(result["state"], "completed");
    assert_eq!(result["wallet"]["pubkey"], pubkey);
    assert_eq!(fake.submitted.lock().unwrap().as_slice(), [vec![signed]]);
    // Settled: it is gone.
    assert!(!ui.state.pending.contains("e1"));
}

#[tokio::test]
async fn a_transaction_that_could_land_on_mainnet_never_reaches_a_browser_wallet() {
    use ed25519_dalek::SigningKey;
    let wallet_key = SigningKey::from_bytes(&[2; 32]);
    let pubkey = bs58::encode(wallet_key.verifying_key().as_bytes()).into_string();
    let mut transaction = wallet::tests::transaction(&[&wallet_key]);
    // Overwrite the blockhash (the last 32 bytes before the instruction
    // count) with a mainnet-looking one.
    let at = transaction.len() - 1 - 32;
    transaction[at..at + 32].copy_from_slice(&[7u8; 32]);
    let (_fake, api_url) = fake_runtime(transaction, 0).await;
    let (ui, base, dir) = serve();
    write_session(dir.path(), &api_url);
    let (status, refused) = post_json(
        &base,
        "/dev/v1/wallet/prepare",
        json!({ "step": "open", "args": {}, "pubkey": pubkey }),
    )
    .await;
    assert_eq!(status, 409, "{refused:#}");
    assert_eq!(refused["code"], "dev-ui-not-local");
    assert!(!ui.state.pending.contains("e1"));
}

#[tokio::test]
async fn a_dev_wallet_step_survives_a_submit_before_the_session_opens() {
    let (ui, base, _dir) = serve();
    ui.state.pending.insert(run::PendingStep {
        request: arete_mcp::dev::StepRequest {
            step: "open".into(),
            instance: None,
            args: json!({}),
            wallet: "alice".into(),
            result: None,
        },
        signer: run::Signer::Dev("alice".into()),
        prepared: arete_mcp::dev::Prepared {
            execution_id: "e9".into(),
            instance_id: None,
            token: "t".into(),
            transactions: vec![],
            response: json!({}),
        },
        at: std::time::Instant::now(),
    });
    let (status, body) = post_json(
        &base,
        "/dev/v1/steps/submit",
        json!({ "executionId": "e9" }),
    )
    .await;
    assert_eq!(status, 503, "{body:#}");
    assert!(ui.state.pending.contains("e9"));
}
