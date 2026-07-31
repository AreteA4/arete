use arete_server::{RuntimePlan, Server, TransactionConfig};

#[test]
fn public_gateway_composition_builds_without_a_spec_or_live_runtime() {
    let runtime = Server::solana_gateway("gateway-us-east-1")
        .bind("127.0.0.1:0".parse::<std::net::SocketAddr>().unwrap())
        .transactions_config(TransactionConfig {
            enabled: true,
            rpc_url: Some("http://127.0.0.1:8899".into()),
            ..TransactionConfig::default()
        })
        .build()
        .unwrap();

    let plan = runtime.plan();
    assert_eq!(plan, RuntimePlan::solana_gateway());
    assert!(plan.health);
    assert!(plan.chain_reads);
    assert!(plan.transactions);
    assert!(!plan.program_reads);
    assert!(!plan.stack_queries);
    assert!(!plan.websocket);
    assert!(!plan.live_runtime_enabled());
}
