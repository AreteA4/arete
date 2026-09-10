//! Minimal live ingestion fixture for the built-in System program. No deployment.
use arete::interpreter::{VmDebugEvent, VmDebugger};
use arete::prelude::*;
use arete::runtime::{
    anyhow,
    serde_json::{self, Value},
    shipstern, tokio,
};
use std::sync::{Arc, Mutex};

#[arete(idl = "examples/transaction-v1/system.json")]
mod smoke {
    #[entity(name = "Transfer")]
    struct Transfer {
        #[map(system_sdk::instructions::Transfer::to, primary_key, strategy = SetOnce)]
        to: String,
        #[map(system_sdk::instructions::Transfer::lamports, strategy = LastWrite)]
        lamports: u64,
        #[map(system_sdk::instructions::Transfer::__signature, strategy = LastWrite)]
        signature: String,
    }
}

// Expose the actual VM input context and emitted state to the local smoke.
// This is stdout only; no checked-in reports or separate observation service.
#[derive(Default)]
struct Observe(Mutex<Option<Value>>);
impl VmDebugger for Observe {
    fn record(&self, event: VmDebugEvent) {
        match event {
            VmDebugEvent::ProcessEventStart { context, .. } => *self.0.lock().unwrap() = context,
            VmDebugEvent::EmitMutation {
                entity_name,
                event_type,
                emitted: true,
                patch,
                ..
            } if entity_name == "Transfer" => {
                println!(
                    "{}",
                    serde_json::json!({
                        "event": event_type,
                        "context": *self.0.lock().unwrap(),
                        "state": patch,
                    })
                );
            }
            _ => {}
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    use arete::interpreter::{runtime_resolvers_factory, scheduler::SlotScheduler, vm::VmContext};
    use arete::runtime::shipstern_yellowstone_grpc_source::{
        YellowstoneGrpcConfig, YellowstoneGrpcSource,
    };
    use shipstern::config::{BufferConfig, ShipsternConfig};

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_writer(std::io::stderr)
        .init();
    let endpoint = std::env::var("YELLOWSTONE_ENDPOINT").map_err(|_| {
        anyhow::anyhow!("Set YELLOWSTONE_ENDPOINT to the already-running local Geyser service")
    })?;
    let vm = Arc::new(Mutex::new(VmContext::new()));
    vm.lock()
        .unwrap()
        .set_debugger(Arc::new(Observe::default()));
    let (tx, mut rx) = tokio::sync::mpsc::channel(128);
    // The normal generated handler requires a projector consumer. The smoke
    // observes its emitted state directly through the public VM debugger.
    tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let handler = smoke::VmHandler::new(
        vm,
        Arc::new(smoke::create_multi_entity_bytecode()),
        tx,
        None,
        arete::server::SlotTracker::new(),
        None,
        runtime_resolvers_factory::build_resolver().map_err(|error| anyhow::anyhow!("{error}"))?,
        Arc::new(Mutex::new(SlotScheduler::new())),
        Arc::new(tokio::sync::Semaphore::new(1)),
    );
    shipstern::Runtime::<YellowstoneGrpcSource>::builder()
        .instruction(shipstern::Pipeline::new(
            smoke::parsers::InstructionParser,
            [handler],
        ))
        .build(ShipsternConfig {
            source: YellowstoneGrpcConfig {
                endpoint,
                x_token: std::env::var("YELLOWSTONE_X_TOKEN").ok(),
                timeout: 60,
                // Surfpool exposes transaction notifications before the full
                // Agave slot lifecycle used by Yellowstone's reconstruction.
                // The smoke independently requires confirmation through Arete.
                commitment_level: Some(arete::runtime::shipstern_core::CommitmentLevel::Processed),
                from_slot: None,
                accept_compression: None,
                max_decoding_message_size: None,
                accounts_data_slice: vec![],
                auto_reconnect: false,
                reconnect_max_retries: None,
                reconnect_slot_retention: None,
            },
            buffer: BufferConfig::default(),
        })
        .try_run_async()
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))
}
