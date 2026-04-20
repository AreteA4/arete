use arete_server::Server;
use ore_stack as ore_stream;
use std::net::SocketAddr;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let env_path = manifest_dir.join(".env");
    if env_path.exists() {
        dotenvy::from_path(&env_path)?;
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let spec = ore_stream::spec();

    println!("Starting Ore server on [::]:8878...");

    Server::builder()
        .spec(spec)
        .websocket()
        .bind("[::]:8878".parse::<SocketAddr>()?)
        .health_monitoring()
        .start()
        .await?;

    Ok(())
}
