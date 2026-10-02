use arete_solana_contracts::*;
use serde_json::Value;
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

#[test]
fn discovery_fixtures_round_trip_without_precision_or_freshness_loss() {
    for case in fixture("owner-token-accounts")["cases"].as_array().unwrap() {
        if case.get("status").is_some() {
            continue;
        }
        let request: OwnerTokenAccountsRequest =
            serde_json::from_value(case["request"].clone()).unwrap();
        request.validate().unwrap();
        let page: OwnerTokenAccountsPage =
            serde_json::from_value(case["response"].clone()).unwrap();
        page.validate(&request).unwrap();
        assert_eq!(serde_json::to_value(page).unwrap(), case["response"]);
    }
}

#[test]
fn contextual_fixtures_preserve_actual_slots_and_nulls() {
    for case in fixture("contextual-accounts")["cases"].as_array().unwrap() {
        let response: Contextual<Value> = serde_json::from_value(case["response"].clone()).unwrap();
        assert_eq!(serde_json::to_value(response).unwrap(), case["response"]);
    }
    for bad in [
        serde_json::json!({"slot": 42}),
        serde_json::json!({"slot": "18446744073709551616"}),
        serde_json::json!({"slot": "+42"}),
    ] {
        assert!(serde_json::from_value::<ReadContext>(bad).is_err());
    }
    assert!(serde_json::from_value::<Contextual<Option<Value>>>(
        serde_json::json!({ "context": { "slot": "42" } })
    )
    .is_err());
}

#[test]
fn invalid_filters_limits_and_cursors_fail() {
    let mut request: OwnerTokenAccountsRequest =
        serde_json::from_value(fixture("owner-token-accounts")["cases"][0]["request"].clone())
            .unwrap();
    request.limit = 0;
    assert!(request.validate().is_err());
    request.limit = 101;
    assert!(request.validate().is_err());
    request.limit = 1;
    request.cursor = Some(String::new());
    assert!(request.validate().is_err());
    request.cursor = None;
    request.owner = "wrong".into();
    assert!(request.validate().is_err());
    assert!(NativePositionQuery::default().validate().is_err());
}

#[test]
fn discovery_requires_an_actual_timestamp_with_a_timezone() {
    let mut discovery = DiscoveryProvenance {
        source: "fixture-index".into(),
        observed_at: "2026-10-01T12:30:45Z".into(),
        watermark: None,
    };
    discovery.validate().unwrap();
    for invalid in ["yesterday", "2026-10-01", "2026-10-01T12:30:45"] {
        discovery.observed_at = invalid.into();
        assert!(discovery.validate().is_err());
    }
}

#[test]
fn versioned_manifest_and_deletion_contract_are_precise() {
    let manifest = fixture("manifest");
    assert_eq!(manifest["contract"], CONTRACT_VERSION);
    assert_eq!(manifest["bundleVersion"], "1.0.0");
    assert_eq!(manifest["sdkRelease"]["firstPlannedRelease"], "0.29.0");
    let schema = fixture("schema");
    for route in manifest["routes"].as_array().unwrap() {
        assert_eq!(route["method"], "POST");
        for field in ["requestSchema", "responseSchema"] {
            assert!(schema["$defs"]
                .get(route[field].as_str().unwrap())
                .is_some());
        }
    }
    let tombstone: AccountTombstone =
        serde_json::from_value(fixture("account-deletion")["tombstone"].clone()).unwrap();
    tombstone.validate().unwrap();
    assert_eq!(tombstone.slot, 9_007_199_254_740_993);
    assert_eq!(tombstone.write_version, u64::MAX);
    assert_eq!(
        serde_json::to_value(tombstone).unwrap(),
        fixture("account-deletion")["tombstone"]
    );
    for case in fixture("wire-cases")["cases"].as_array().unwrap() {
        let route = manifest["routes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|route| route["id"] == case["route"])
            .unwrap();
        assert_eq!(case["method"], route["method"]);
        if case["status"] == 200 {
            let value: Contextual<Value> =
                serde_json::from_value(case["response"].clone()).unwrap();
            assert_eq!(serde_json::to_value(value).unwrap(), case["response"]);
        }
    }
}
