use arete_server::websocket::auth::{
    AllowAllAuthPlugin, AuthDecision, AuthDeny, ConnectionAuthRequest, WebSocketAuthPlugin,
};
use arete_server::{http_health::HttpHealthServer, token_discovery::*};
use arete_server::{SolanaGatewayUsageObservation, SolanaGatewayUsageObserver};

struct FixtureAuth;
#[async_trait::async_trait]
impl WebSocketAuthPlugin for FixtureAuth {
    async fn authorize(&self, request: &ConnectionAuthRequest) -> AuthDecision {
        assert!(request.query.is_none(), "HTTP auth is bearer-only");
        let Some(token) = request.bearer_token() else {
            return AuthDecision::Deny(AuthDeny::token_missing());
        };
        let AuthDecision::Allow(mut context) = AllowAllAuthPlugin.authorize(request).await else {
            unreachable!()
        };
        context.audience = arete_auth::SOLANA_GATEWAY_AUDIENCE.into();
        context.target_kind = Some(arete_auth::TargetKind::SolanaGatewayBinding);
        context.target_id = Some(
            if token == "wrong-target" {
                "other-gateway"
            } else {
                "fixture-gateway"
            }
            .into(),
        );
        context.scope = if token == "wrong-scope" {
            "transaction:inspect"
        } else {
            "read"
        }
        .into();
        context.limits.max_http_batch_addresses = Some(1);
        AuthDecision::Allow(context)
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
#[derive(Default)]
struct Usage(Mutex<Vec<SolanaGatewayUsageObservation>>);
#[async_trait::async_trait]
impl SolanaGatewayUsageObserver for Usage {
    async fn observe(&self, event: SolanaGatewayUsageObservation) {
        self.0.lock().unwrap().push(event);
    }
}

use arete_server::{
    IdlContentHash, NormalizedIdlHash, ProgramRuntimeCatalog, ProgramRuntimeDefinition,
    ProgramSpecHash,
};
use http_body_util::{BodyExt, Full};
use hyper::{body::Bytes, service::service_fn, Response};
use hyper_util::rt::TokioIo;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

fn fixture(name: &str) -> Value {
    serde_json::from_str(
        &std::fs::read_to_string(format!(
            "{}/../../tests/fixtures/managed-solana-v1/{name}.json",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap(),
    )
    .unwrap()
}
struct Index;
#[async_trait::async_trait]
impl OwnerTokenAccountsProvider for Index {
    async fn owner_token_accounts(
        &self,
        request: &OwnerTokenAccountsRequest,
    ) -> Result<OwnerTokenAccountsPage, TokenDiscoveryError> {
        let fixtures = fixture("owner-token-accounts");
        let index = if request.cursor.is_some() { 1 } else { 0 };
        if request.owner != fixtures["cases"][0]["request"]["owner"].as_str().unwrap() {
            return Err(TokenDiscoveryError::InvalidCursor(
                "Cursor does not match the owner and filters".into(),
            ));
        }
        if request.cursor.is_some()
            && (request.mint.as_deref() != fixtures["cases"][0]["request"]["mint"].as_str()
                || request.token_program.as_deref()
                    != fixtures["cases"][0]["request"]["tokenProgram"].as_str())
        {
            return Err(TokenDiscoveryError::InvalidCursor(
                "Cursor does not match the owner and filters".into(),
            ));
        }
        Ok(serde_json::from_value(fixtures["cases"][index]["response"].clone()).unwrap())
    }
}

#[tokio::test]
async fn managed_gateway_provider_and_contextual_reads_are_independent_of_live_services() {
    let fixtures = fixture("owner-token-accounts");
    let calls = Arc::new(Mutex::new(Vec::<Value>::new()));
    let rpc = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let rpc_url = format!("http://{}", rpc.local_addr().unwrap());
    let recorded = calls.clone();
    let rpc_task = tokio::spawn(async move {
        loop {
            let (socket, _) = rpc.accept().await.unwrap();
            let recorded = recorded.clone();
            tokio::spawn(async move {
                hyper::server::conn::http1::Builder::new().serve_connection(TokioIo::new(socket), service_fn(move |req: hyper::Request<hyper::body::Incoming>| {
                    let recorded = recorded.clone();
                    async move {
                        let body: Value = serde_json::from_slice(&req.into_body().collect().await.unwrap().to_bytes()).unwrap();
                        recorded.lock().unwrap().push(body.clone());
                        let account = json!({ "owner": "11111111111111111111111111111111", "lamports": 9007199254740993u64, "executable": false, "data": ["AQID", "base64"] });
                        let value = if body["method"] == "getMultipleAccounts" {
                            Value::Array(body["params"][0].as_array().unwrap().iter().enumerate().map(|(index, _)| match index {
                                0 => account.clone(), 1 => Value::Null,
                                _ => { let mut bad = account.clone(); bad["data"] = json!(["BA==", "base64"]); bad },
                            }).collect())
                        } else { account };
                        let result = if body["method"] == "getTransaction" {
                            let index = bs58::decode(body["params"][0].as_str().unwrap()).into_vec().unwrap()[0] as usize - 1;
                            fixture("transactions")["cases"][index]["upstream"].clone()
                        } else { json!({ "context": { "slot": 9007199254740994u64 }, "value": value }) };
                        Ok::<_, std::convert::Infallible>(Response::new(Full::new(Bytes::from(json!({ "jsonrpc": "2.0", "id": 1, "result": result }).to_string()))))
                    }
                })).await.unwrap();
            });
        }
    });
    // This test binary has one test; environment mutation cannot race sibling tests.
    std::env::set_var("ARETE_READ_RPC_URL", &rpc_url);
    let port = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = port.local_addr().unwrap();
    drop(port);
    let shutdown = CancellationToken::new();
    let program_id = "11111111111111111111111111111111";
    let program_spec_hash = ProgramSpecHash::from_digest([1; 32]);
    let idl_content_hash = IdlContentHash::from_digest([2; 32]);
    let normalized_idl_hash = NormalizedIdlHash::from_digest([3; 32]);
    let program_release_hash = arete_hash::OssGeneratedProgramReleaseV1::new(
        program_id,
        program_spec_hash,
        idl_content_hash,
        normalized_idl_hash,
    )
    .hash()
    .unwrap();
    let definition = ProgramRuntimeDefinition {
        program_id: program_id.into(),
        program_spec_hash,
        idl_content_hash,
        normalized_idl_hash,
        program_release_hash,
        account_reader: Arc::new(|account, bytes| {
            anyhow::ensure!(
                account == "Position" && bytes == [1, 2, 3],
                "fixture decoder rejection"
            );
            Ok(json!({ "liquidity": "9007199254740993" }))
        }),
    };
    let usage = Arc::new(Usage::default());
    let server = HttpHealthServer::new(address)
        .with_transaction_config(arete_server::config::TransactionConfig {
            enabled: true,
            allow_unauthenticated: true,
            rpc_url: Some(rpc_url.clone()),
            ..Default::default()
        })
        .with_solana_gateway_target("fixture-gateway")
        .with_solana_gateway_usage_observer(usage.clone())
        .with_owner_token_accounts_provider(Arc::new(Index))
        .with_program_runtime_catalog(ProgramRuntimeCatalog::try_new(vec![definition]).unwrap())
        .with_shutdown(shutdown.clone());
    let task = tokio::spawn(server.start());
    let root = format!("http://{address}");
    let client = reqwest::Client::new();
    for _ in 0..100 {
        if client.get(format!("{root}/health")).send().await.is_ok() {
            break;
        }
        tokio::task::yield_now().await;
    }
    for case in fixtures["cases"].as_array().unwrap().iter().take(2) {
        let response = client
            .post(format!("{root}/chain/v1/owner-token-accounts"))
            .json(&case["request"])
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(response.json::<Value>().await.unwrap(), case["response"]);
    }
    let case = &fixtures["cases"][4];
    let response = client
        .post(format!("{root}/chain/v1/owner-token-accounts"))
        .json(&case["request"])
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
    assert_eq!(response.json::<Value>().await.unwrap(), case["response"]);
    let mut changed_mint = fixtures["cases"][1]["request"].clone();
    changed_mint["mint"] = json!("11111111111111111111111111111111");
    assert_eq!(
        client
            .post(format!("{root}/chain/v1/owner-token-accounts"))
            .json(&changed_mint)
            .send()
            .await
            .unwrap()
            .status(),
        400
    );
    let cases = fixture("contextual-accounts");
    for index in [0usize, 2, 3] {
        let case = &cases["cases"][index];
        let response = client
            .post(format!("{root}{}", case["path"].as_str().unwrap()))
            .json(&case["request"])
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let result: Value = response.json().await.unwrap();
        if index == 0 || index == 3 {
            assert_eq!(result, case["response"]);
        } else {
            assert_eq!(result["context"]["slot"], "9007199254740994");
            assert!(result["value"][1].is_null());
        }
    }
    {
        let calls = calls.lock().unwrap();
        assert_eq!(
            calls.len(),
            2,
            "empty batches and indexed discovery never use RPC"
        );
        assert_eq!(calls[0]["params"][1]["commitment"], "finalized");
        assert_eq!(calls[0]["params"][1]["minContextSlot"], 9007199254740993u64);
    }
    let typed_path = format!("{root}/v1/releases/{program_release_hash}/accounts/Position");
    let options = json!({ "commitment": "finalized", "minContextSlot": "40" });
    let result: Value = client
        .post(format!("{typed_path}/{program_id}/context"))
        .json(&json!({ "options": options }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(result["context"]["slot"], "9007199254740994");
    assert_eq!(result["value"]["liquidity"], "9007199254740993");
    let addresses = &cases["cases"][4]["request"]["addresses"];
    let result: Value = client
        .post(format!("{typed_path}/context"))
        .json(&json!({ "addresses": addresses, "options": options }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let items = result["value"]["items"].as_array().unwrap();
    assert_eq!(items[0]["status"], "ok");
    assert_eq!(items[1]["status"], "missing");
    assert_eq!(items[2]["error"]["code"], "ACCOUNT_DECODE_FAILED");
    for (item, address) in items.iter().zip(addresses.as_array().unwrap()) {
        assert_eq!(&item["address"], address);
    }
    let result: Value = client
        .post(format!("{typed_path}/context"))
        .json(&json!({ "addresses": [] }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(result, json!({ "context": null, "value": { "items": [] } }));
    assert_eq!(
        client
            .post(format!("{typed_path}/query"))
            .json(&json!({ "owner": program_id }))
            .send()
            .await
            .unwrap()
            .status(),
        501
    );
    let unknown = arete_server::ProgramReleaseHash::from_digest([9; 32]);
    let response = client
        .post(format!(
            "{root}/v1/releases/{unknown}/accounts/Position/context"
        ))
        .json(&json!({ "addresses": [] }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 404);
    assert_eq!(
        response.headers()["X-Error-Code"],
        "PROGRAM_RELEASE_NOT_FOUND"
    );
    let mut bad = fixtures["cases"][0]["request"].clone();
    bad["limit"] = json!(101);
    assert_eq!(
        client
            .post(format!("{root}/chain/v1/owner-token-accounts"))
            .json(&bad)
            .send()
            .await
            .unwrap()
            .status(),
        400
    );
    // The same public relay returns the full retained execution response for
    // every wire fixture, including unknown metadata and unseen transactions.
    for (index, case) in fixture("transactions")["cases"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let signature = bs58::encode([index as u8 + 1; 64]).into_string();
        let result: Value = client
            .post(format!("{root}/transactions/v1/get"))
            .json(&json!({"signature": signature}))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        if case["response"]["transaction"].is_null() {
            assert!(result["transaction"].is_null());
        } else {
            for field in [
                "transaction",
                "meta",
                "version",
                "metadataAvailable",
                "slot",
            ] {
                assert_eq!(
                    result["transaction"][field], case["response"]["transaction"][field],
                    "{field}"
                );
            }
        }
    }
    {
        let observations = usage.0.lock().unwrap();
        let owner = observations
            .iter()
            .find(|event| event.operation == "owner_token_accounts")
            .unwrap();
        assert_eq!(owner.address_count, 1);
        assert_eq!(owner.target_id, "fixture-gateway");
        assert!(owner.response_bytes > 0);
        assert!(observations
            .iter()
            .any(|event| event.operation == "get" && event.result == "ok"));
    }
    // Fixture error cases run against the actual canonical routes. Release
    // identity comes from this runtime's registered definition.
    for case in fixture("wire-cases")["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|case| case["status"] != 200 && case["responseCode"] != "unsupported_capability")
    {
        let path = case["path"].as_str().unwrap().replace(
            &format!("arete:h1:program-release:sha256:{}", "a".repeat(64)),
            &program_release_hash.to_string(),
        );
        let response = client
            .post(format!("{root}{path}"))
            .json(&case["request"])
            .send()
            .await
            .unwrap();
        assert_eq!(
            response.status().as_u16(),
            case["status"].as_u64().unwrap() as u16
        );
        let body: Value = response.json().await.unwrap();
        assert_eq!(
            body.get("code")
                .or_else(|| body.pointer("/error/code"))
                .unwrap(),
            &case["responseCode"]
        );
    }
    for path in [
        "/chain/v1/account",
        "/chain/v1/accounts",
        "/chain/v1/owner-token-accounts",
    ] {
        let response = client
            .post(format!("{root}{path}"))
            .body("{invalid")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 400);
        assert_eq!(
            response.json::<Value>().await.unwrap()["code"],
            "invalid_request"
        );
        assert_eq!(
            client
                .get(format!("{root}{path}"))
                .send()
                .await
                .unwrap()
                .status(),
            404
        );
    }
    shutdown.cancel();
    task.await.unwrap().unwrap();
    rpc_task.abort();
    std::env::remove_var("ARETE_READ_RPC_URL");
    verify_authenticated_provider_without_rpc(&fixtures).await;
}

async fn verify_authenticated_provider_without_rpc(fixtures: &Value) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let shutdown = CancellationToken::new();
    let usage = Arc::new(Usage::default());
    let task = tokio::spawn(
        HttpHealthServer::new(address)
            .with_auth_plugin(Arc::new(FixtureAuth))
            .with_solana_gateway_target("fixture-gateway")
            .with_solana_gateway_usage_observer(usage.clone())
            .with_owner_token_accounts_provider(Arc::new(Index))
            .with_shutdown(shutdown.clone())
            .start(),
    );
    let client = reqwest::Client::new();
    let root = format!("http://{address}");
    for _ in 0..100 {
        if client.get(format!("{root}/health")).send().await.is_ok() {
            break;
        }
        tokio::task::yield_now().await;
    }
    let endpoint = format!("{root}/chain/v1/owner-token-accounts");
    let request = &fixtures["cases"][0]["request"];
    for (token, status, code) in [
        (None, 401, "token-missing"),
        (Some("wrong-target"), 403, "invalid_gateway_target"),
        (Some("wrong-scope"), 403, "insufficient_scope"),
    ] {
        let mut builder = client.post(&endpoint).json(request);
        if let Some(token) = token {
            builder = builder.bearer_auth(token);
        }
        let response = builder.send().await.unwrap();
        assert_eq!(response.status(), status);
        assert_eq!(response.json::<Value>().await.unwrap()["code"], code);
    }
    assert!(
        usage.0.lock().unwrap().is_empty(),
        "authorization fails before invoking the provider/meter"
    );
    let response = client
        .post(&endpoint)
        .bearer_auth("read")
        .json(request)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.json::<Value>().await.unwrap(),
        fixtures["cases"][0]["response"]
    );
    // Omitted limit defaults to 100; a one-address session must specify 1.
    let response = client
        .post(&endpoint)
        .bearer_auth("read")
        .json(&json!({"owner":request["owner"]}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
    {
        let observations = usage.0.lock().unwrap();
        assert_eq!(observations.len(), 2);
        assert_eq!(observations[0].address_count, 1);
        assert_eq!(observations[1].address_count, 0);
        assert_eq!(observations[1].result, "error");
        assert_eq!(
            observations[0]
                .auth_context
                .as_ref()
                .unwrap()
                .target_id
                .as_deref(),
            Some("fixture-gateway")
        );
    }
    shutdown.cancel();
    task.await.unwrap().unwrap();
}
