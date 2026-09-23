//! Its own test binary: it changes the process working directory.

use arete_server::config::RuntimePlan;
use arete_server::{Commitment, Server};

/// A level set only in `.env` must be what the runtime validates at startup,
/// not something the parser discovers after the runtime has already committed
/// to the default.
#[tokio::test]
async fn a_level_in_dotenv_is_read_before_the_runtime_commits_to_one() {
    let dir = std::env::temp_dir().join(format!("arete-dotenv-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(".env"), format!("{}=bogus\n", Commitment::ENV)).unwrap();
    std::env::set_current_dir(&dir).unwrap();

    let spawned = Server::builder()
        .runtime_plan(RuntimePlan {
            health: false,
            chain_reads: false,
            program_reads: false,
            stack_queries: false,
            transactions: false,
            websocket: false,
            live_runtime: true,
        })
        .build()
        .unwrap()
        .spawn()
        .await;

    let _ = std::fs::remove_dir_all(&dir);
    let error = spawned.err().expect("the .env value must fail startup");
    assert!(error.to_string().contains("bogus"), "{error:#}");
}
