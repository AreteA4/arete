use crate::chain::{ChainClient, HttpChainClient};
use crate::http::HttpAuthClient;
use crate::managed_solana::*;
use crate::program_read_transport::{
    test_support::{test_release, CannedResponse, TestServer},
    ProgramReadTransport,
};
use crate::read::{AccountBatchItem, AccountReader};
use crate::transactions::{
    HttpTransactionTransport, TransactionInspectOptions, TransactionTransport,
};
use serde_json::Value;
use std::sync::Arc;

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
fn auth() -> Arc<HttpAuthClient> {
    Arc::new(HttpAuthClient::new(None, None, reqwest::Client::new()))
}

#[tokio::test]
async fn managed_owner_pages_and_raw_context_round_trip() {
    let fixtures = fixture("owner-token-accounts");
    let cases = fixtures["cases"].as_array().unwrap();
    let server = TestServer::spawn(
        cases
            .iter()
            .take(3)
            .map(|c| CannedResponse::json(c["response"].to_string()))
            .collect(),
    )
    .await;
    let chain = HttpChainClient::new(&server.base_url, auth());
    for case in cases.iter().take(3) {
        let input: OwnerTokenAccountsRequest =
            serde_json::from_value(case["request"].clone()).unwrap();
        let page = chain.owner_token_accounts(&input).await.unwrap();
        assert_eq!(serde_json::to_value(page).unwrap(), case["response"]);
    }
    let contextual = fixture("contextual-accounts");
    let case = &contextual["cases"][0];
    let server = TestServer::spawn(vec![CannedResponse::json(case["response"].to_string())]).await;
    let chain = HttpChainClient::new(&server.base_url, auth());
    let options: ReadOptions = serde_json::from_value(case["request"]["options"].clone()).unwrap();
    let read = chain
        .account_with_context(case["request"]["address"].as_str().unwrap(), options)
        .await
        .unwrap();
    assert_eq!(read.context.unwrap().slot, 9_007_199_254_740_994);
    assert_eq!(read.value.unwrap().lamports, 9_007_199_254_740_993);
    assert_eq!(
        serde_json::from_str::<Value>(&server.requests()[0].body).unwrap(),
        case["request"]
    );
}

#[tokio::test]
async fn managed_typed_batch_keeps_missing_and_per_item_errors() {
    let fixtures = fixture("contextual-accounts");
    let case = &fixtures["cases"][4];
    let server = TestServer::spawn(vec![CannedResponse::json(case["response"].to_string())]).await;
    let transport = Arc::new(ProgramReadTransport::local_http(
        &server.base_url,
        test_release(),
        reqwest::Client::new(),
    ));
    let reader = AccountReader::<Value>::new("Position", transport);
    let addresses: Vec<_> = case["request"]["addresses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap())
        .collect();
    let read = reader
        .fetch_many_with_context(
            &addresses,
            ReadOptions {
                commitment: Some(Commitment::Confirmed),
                min_context_slot: Some(40),
            },
        )
        .await
        .unwrap();
    assert_eq!(read.context.unwrap().slot, 42);
    assert!(matches!(
        read.value.items[1],
        AccountBatchItem::Missing { .. }
    ));
    assert!(
        matches!(read.value.items[2], AccountBatchItem::Error { ref code, .. } if code == "ACCOUNT_DECODE_FAILED")
    );
    assert!(server.requests()[0].path.ends_with("/Position/context"));
    assert!(reader
        .fetch_many_with_context(&vec![addresses[0]; 101], ReadOptions::default())
        .await
        .is_err());
}

#[tokio::test]
async fn managed_transactions_keep_execution_data_and_absent_metadata() {
    let fixture = fixture("transactions");
    let cases = fixture["cases"].as_array().unwrap();
    let server = TestServer::spawn(
        cases
            .iter()
            .map(|c| CannedResponse::json(c["response"].to_string()))
            .collect(),
    )
    .await;
    let transport = HttpTransactionTransport::new(&server.base_url, auth());
    for case in cases {
        let result = transport
            .transaction("fixture-signature", TransactionInspectOptions::default())
            .await
            .unwrap();
        if case["response"]["transaction"].is_null() {
            assert!(result.is_none());
            continue;
        }
        let tx = result.unwrap();
        let expected = &case["response"]["transaction"];
        assert_eq!(
            tx.meta.as_ref(),
            expected.get("meta").filter(|v| !v.is_null())
        );
        assert_eq!(tx.transaction.as_ref(), expected.get("transaction"));
        assert_eq!(
            tx.metadata_available,
            expected["metadataAvailable"].as_bool()
        );
        assert_eq!(tx.version.as_ref(), expected.get("version"));
    }
}

#[tokio::test]
async fn managed_native_query_uses_pinned_release_and_surfaces_capability_errors() {
    let fixture = fixture("native-position-query");
    let first = &fixture["cases"][0];
    let server = TestServer::spawn(vec![
        CannedResponse::json(first["response"].to_string()),
        CannedResponse::json(fixture["cases"][3]["response"].to_string()).with_status(501),
    ])
    .await;
    let reader = AccountReader::<Value>::new(
        "Position",
        Arc::new(ProgramReadTransport::local_http(
            &server.base_url,
            test_release(),
            reqwest::Client::new(),
        )),
    );
    let input: NativePositionQuery = serde_json::from_value(first["request"].clone()).unwrap();
    let page = reader.query_positions(&input).await.unwrap();
    assert_eq!(serde_json::to_value(page).unwrap(), first["response"]);
    assert!(server.requests()[0].path.ends_with("/Position/query"));
    assert!(
        matches!(reader.query_positions(&input).await, Err(crate::read::ReadError::Request(ref error)) if error.status == 501 && error.server_error_code.as_deref() == Some("unsupported_capability"))
    );
}
