//! Its own test binary: it sets `YELLOWSTONE_COMMITMENT` for the whole process.

use arete_server::config::RuntimePlan;
use arete_server::{Commitment, Server};

/// The level is a Yellowstone setting: a server that never ingests (HTTP-only,
/// program reads, the Solana gateway) must start whatever it says.
#[tokio::test]
async fn a_bad_level_does_not_stop_a_server_that_never_ingests() {
    std::env::set_var(Commitment::ENV, "not-a-level");

    let handle = Server::builder()
        .runtime_plan(RuntimePlan {
            health: false,
            chain_reads: false,
            program_reads: true,
            stack_queries: false,
            transactions: false,
            websocket: false,
            live_runtime: false,
        })
        .build()
        .unwrap()
        .spawn()
        .await
        .expect("a non-live server must ignore YELLOWSTONE_COMMITMENT");
    handle.shutdown().await.unwrap();
}
