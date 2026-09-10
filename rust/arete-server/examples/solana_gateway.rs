//! Standalone local relay using the public gateway API and transaction settings.
use arete_server::{Server, TransactionConfig};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    Server::solana_gateway("local-solana-gateway")
        .bind("127.0.0.1:8081".parse::<std::net::SocketAddr>()?)
        .transactions_config(TransactionConfig::from_env()?)
        .start()
        .await
}
