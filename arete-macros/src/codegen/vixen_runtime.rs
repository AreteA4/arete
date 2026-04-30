//! Unified Vixen runtime generation.
//!
//! This module consolidates VmHandler and runtime loop generation that was previously
//! duplicated across `vm_handler.rs`, `spec_fn.rs`, and `idl_vixen_gen.rs`.
//!
//! Key unification:
//! - Single VmHandler definition with MutationBatch + SlotContext
//! - Single runtime loop with configurable logging verbosity
//! - Config-driven generation for different code paths

use proc_macro2::TokenStream;
use quote::{format_ident, quote};

/// Configuration for runtime code generation.
#[derive(Debug, Clone, Default)]
pub struct RuntimeGenConfig {
    /// Include verbose bytecode logging at startup
    pub verbose_bytecode_logging: bool,
    /// Include parser registration logging
    pub verbose_parser_logging: bool,
    /// Include views in spec() function
    pub include_views: bool,
}

impl RuntimeGenConfig {
    /// Configuration for IDL-based generation (more verbose, includes views)
    pub fn for_idl() -> Self {
        Self {
            verbose_bytecode_logging: true,
            verbose_parser_logging: true,
            include_views: true,
        }
    }

    /// Configuration for generate_all path (minimal logging)
    pub fn for_generate_all() -> Self {
        Self {
            verbose_bytecode_logging: false,
            verbose_parser_logging: false,
            include_views: true,
        }
    }
}

/// Generate the `tokio::spawn` block for the slot scheduler background task.
///
/// This is used by both `generate_spec_function` and `generate_multi_pipeline_spec_function`
/// to avoid duplicating the ~160-line scheduler loop.
fn generate_slot_scheduler_task() -> TokenStream {
    quote! {
        {
            let scheduler = slot_scheduler.clone();
            let vm = vm.clone();
            let bytecode = bytecode_arc.clone();
            let runtime_resolver = runtime_resolver.clone();
            let slot_tracker = slot_tracker.clone();
            let mutations_tx = mutations_tx.clone();

            arete::runtime::tokio::spawn(async move {
                arete::runtime::tracing::info!(
                    "SlotScheduler: started (in-memory only, pending callbacks will not survive restarts)"
                );
                loop {
                    // Wait for a slot advance notification, or fall back to polling
                    // every 5s in case notifications are missed.
                    arete::runtime::tokio::select! {
                        _ = slot_tracker.notified() => {},
                        _ = arete::runtime::tokio::time::sleep(std::time::Duration::from_secs(5)) => {},
                    }

                    let current_slot = slot_tracker.get();
                    if current_slot == 0 {
                        continue;
                    }

                    use arete::runtime::futures::FutureExt;

                    let tick_future = std::panic::AssertUnwindSafe(async {
                        let due = {
                            let mut sched = scheduler.lock().unwrap_or_else(|e| e.into_inner());
                            sched.take_due(current_slot)
                        };

                        const MAX_RETRIES: u32 = arete::runtime::arete_interpreter::scheduler::MAX_RETRIES;

                        if !due.is_empty() {
                            arete::runtime::tracing::info!(
                                current_slot = current_slot,
                                due_count = due.len(),
                                "[SCHEDULER] Processing due callbacks"
                            );
                        }

                        for mut callback in due {
                            let state = {
                                let vm_guard = vm.lock().unwrap_or_else(|e| e.into_inner());
                                vm_guard.get_entity_state(callback.state_id, &callback.primary_key)
                            };

                            let state = match state {
                                Some(s) => s,
                                None => {
                                    if callback.retry_count < MAX_RETRIES {
                                        callback.retry_count += 1;
                                        let mut sched = scheduler.lock().unwrap_or_else(|e| e.into_inner());
                                        sched.re_register(callback, current_slot + 1);
                                    } else {
                                        arete::runtime::tracing::warn!(
                                            entity = %callback.entity_name,
                                            key = ?callback.primary_key,
                                            "SlotScheduler: entity state not found, discarding after max retries"
                                        );
                                    }
                                    continue;
                                }
                            };

                            if let Some(ref condition) = callback.condition {
                                let condition_met = arete::runtime::arete_interpreter::scheduler::evaluate_condition(condition, &state);
                                let field_val = arete::runtime::arete_interpreter::scheduler::get_value_at_path(&state, &condition.field_path);
                                arete::runtime::tracing::info!(
                                    entity = %callback.entity_name,
                                    key = ?callback.primary_key,
                                    condition_field = %condition.field_path,
                                    condition_met = condition_met,
                                    field_value = ?field_val,
                                    "[SCHEDULER] Re-evaluating condition at callback fire time"
                                );
                                if !condition_met {
                                    continue;
                                }
                            }

                            if callback.strategy == arete::runtime::arete_interpreter::ast::ResolveStrategy::SetOnce {
                                // Use all(): fire resolver if any target is still null.
                                // Already-set fields are protected from overwrite by the
                                // SetOnce guard in VmContext::set_value_at_path.
                                let already_resolved = callback.extracts.iter().all(|ext| {
                                    let val = arete::runtime::arete_interpreter::scheduler::get_value_at_path(&state, &ext.target_path);
                                    val.map(|v| !v.is_null()).unwrap_or(false)
                                });
                                if already_resolved {
                                    arete::runtime::tracing::info!(
                                        entity = %callback.entity_name,
                                        key = ?callback.primary_key,
                                        targets = ?callback.extracts.iter().map(|e| &e.target_path).collect::<Vec<_>>(),
                                        "[SCHEDULER] SetOnce guard: all targets already populated, skipping"
                                    );
                                    continue;
                                }
                            }

                            let url = if let Some(ref template) = callback.url_template {
                                match arete::runtime::arete_interpreter::scheduler::build_url_from_template(template, &state) {
                                    Some(u) => u,
                                    None => {
                                        if callback.retry_count < MAX_RETRIES {
                                            callback.retry_count += 1;
                                            let mut sched = scheduler.lock().unwrap_or_else(|e| e.into_inner());
                                            sched.re_register(callback, current_slot + 1);
                                        } else {
                                            arete::runtime::tracing::warn!(
                                                entity = %callback.entity_name,
                                                key = ?callback.primary_key,
                                                "SlotScheduler: URL template unresolvable, discarding after max retries"
                                            );
                                        }
                                        continue;
                                    }
                                }
                            } else if let Some(ref val) = callback.input_value {
                                match val.as_str() {
                                    Some(s) => s.to_string(),
                                    None => val.to_string().trim_matches('"').to_string(),
                                }
                            } else if let Some(ref path) = callback.input_path {
                                match arete::runtime::arete_interpreter::scheduler::get_value_at_path(&state, path) {
                                    Some(v) if !v.is_null() => match v.as_str() {
                                        Some(s) => s.to_string(),
                                        None => v.to_string().trim_matches('"').to_string(),
                                    },
                                    _ => {
                                        if callback.retry_count < MAX_RETRIES {
                                            callback.retry_count += 1;
                                            let mut sched = scheduler.lock().unwrap_or_else(|e| e.into_inner());
                                            sched.re_register(callback, current_slot + 1);
                                        }
                                        continue;
                                    }
                                }
                            } else {
                                continue;
                            };

                            let cache_key = arete::runtime::arete_interpreter::runtime_resolvers::runtime_resolver_cache_key(
                                &callback.resolver,
                                &arete::runtime::serde_json::Value::String(url.clone()),
                            );

                            // IMPORTANT: enqueue + take must stay inside the same lock guard.
                            // Splitting them risks lost or duplicated requests during reconnects.
                            let requests = {
                                let mut vm_guard = vm.lock().unwrap_or_else(|e| e.into_inner());
                                let target = arete::runtime::arete_interpreter::vm::ResolverTarget {
                                    state_id: callback.state_id,
                                    entity_name: callback.entity_name.clone(),
                                    primary_key: callback.primary_key.clone(),
                                    extracts: callback.extracts.clone(),
                                };
                                vm_guard.enqueue_resolver_request(
                                    cache_key.clone(),
                                    callback.resolver.clone(),
                                    arete::runtime::serde_json::Value::String(url.clone()),
                                    target,
                                );
                                vm_guard.take_resolver_requests()
                            };

                            // Scheduled resolver application also mutates VM state, so reserve
                            // projector capacity before applying it.
                            let projector_permit = reserve_projector_batch_slot(
                                mutations_tx.clone(),
                                "scheduled resolver callback",
                            )
                            .await;

                            let url_mutations = runtime_resolver
                                .resolve_and_apply(&vm, bytecode.as_ref(), requests)
                                .await;

                            if url_mutations.is_empty() {
                                if callback.retry_count < MAX_RETRIES {
                                    callback.retry_count += 1;
                                    let mut sched = scheduler.lock().unwrap_or_else(|e| e.into_inner());
                                    sched.re_register(callback, current_slot + 1);
                                } else {
                                    arete::runtime::tracing::warn!(
                                        entity = %callback.entity_name,
                                        key = ?callback.primary_key,
                                        "SlotScheduler: resolver returned no data, discarding after max retries"
                                    );
                                }
                            } else {
                                let slot_context = arete::runtime::arete_server::SlotContext::new(current_slot, 0);
                                let batch = arete::runtime::arete_server::MutationBatch::with_slot_context(
                                    arete::runtime::smallvec::SmallVec::from_vec(url_mutations),
                                    slot_context,
                                );
                                projector_permit.send(batch);
                            }
                        }
                    });

                    if let Err(panic_info) = tick_future.catch_unwind().await {
                        let msg = panic_info
                            .downcast_ref::<&str>().map(|s| s.to_string())
                            .or_else(|| panic_info.downcast_ref::<String>().cloned())
                            .unwrap_or_else(|| "unknown panic".to_string());
                        arete::runtime::tracing::error!(
                            error = %msg,
                            "SlotScheduler: tick panicked, continuing"
                        );
                    }
                }
            });
        }
    }
}

/// Generate the `tokio::spawn` block for the gRPC slot subscription.
///
/// Opens a dedicated gRPC connection to stream slot updates, updating the
/// `SlotTracker` on each new slot. This drives the scheduler to fire callbacks
/// immediately when the target slot arrives, rather than waiting for the next
/// account/instruction event.
fn generate_slot_subscription_task() -> TokenStream {
    quote! {
        // Helper function to parse SlotHashes sysvar data
        fn parse_and_cache_slot_hashes(current_slot: u64, data: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
            if data.len() < 8 {
                return Err("Data too short".into());
            }

            let len = u64::from_le_bytes([
                data[0], data[1], data[2], data[3],
                data[4], data[5], data[6], data[7],
            ]) as usize;

            let entry_size: usize = 40;
            let expected_size = 8_usize
                .checked_add(len.checked_mul(entry_size).ok_or("len * entry_size overflow")?)
                .ok_or("expected_size overflow")?;

            if data.len() < expected_size {
                return Err(format!("Data too short: expected {}, got {}", expected_size, data.len()).into());
            }

            for i in 0..len {
                let offset = 8 + (i * entry_size);
                let slot = u64::from_le_bytes([
                    data[offset], data[offset + 1], data[offset + 2], data[offset + 3],
                    data[offset + 4], data[offset + 5], data[offset + 6], data[offset + 7],
                ]);
                let hash_bytes = &data[offset + 8..offset + 40];
                let hash = arete::runtime::bs58::encode(hash_bytes).into_string();
                arete::runtime::arete_interpreter::record_slot_hash(slot, hash);
                arete::runtime::tracing::debug!(slot = slot, current_slot = current_slot, "[SLOT_SUB] Cached slot hash");
            }
            Ok(())
        }

        {
            let slot_tracker = slot_tracker.clone();
            let endpoint = endpoint.clone();
            let x_token = x_token.clone();

            arete::runtime::tokio::spawn(async move {
                arete::runtime::tracing::info!("[SLOT_SUB] Starting dedicated gRPC slot subscription");

                loop {
                    let result: Result<(), Box<dyn std::error::Error + Send + Sync>> = async {
                        use arete::runtime::yellowstone_grpc_proto::geyser::{
                            SubscribeRequest, SubscribeRequestFilterSlots, SubscribeRequestFilterAccounts,
                            subscribe_update::UpdateOneof,
                        };
                        use arete::runtime::futures::StreamExt;

                        let mut builder = arete::runtime::yellowstone_grpc_client::GeyserGrpcClient
                            ::build_from_shared(endpoint.clone())?
                            .x_token(x_token.clone())?
                            .max_decoding_message_size(usize::MAX)
                            .accept_compressed(
                                arete::runtime::yellowstone_grpc_proto::tonic::codec::CompressionEncoding::Zstd
                            )
                            .connect_timeout(std::time::Duration::from_secs(30))
                            .timeout(std::time::Duration::from_secs(60));

                        if endpoint.starts_with("https://") || endpoint.starts_with("grpcs://") {
                            builder = builder.tls_config(
                                arete::runtime::yellowstone_grpc_proto::tonic::transport::ClientTlsConfig::new()
                                    .with_native_roots()
                            )?;
                        }

                        let mut client = builder.connect().await?;

                        // Solana SlotHashes sysvar address
                        let slot_hashes_sysvar = "SysvarS1otHashes111111111111111111111111111".to_string();

                        let subscribe_request = SubscribeRequest {
                            slots: std::collections::HashMap::from([(
                                "slot_sub".to_string(),
                                SubscribeRequestFilterSlots {
                                    filter_by_commitment: Some(true),
                                    interslot_updates: None,
                                },
                            )]),
                            // Subscribe to SlotHashes sysvar to capture slot hashes
                            accounts: std::collections::HashMap::from([(
                                "slot_hashes_sysvar".to_string(),
                                SubscribeRequestFilterAccounts {
                                    account: vec![slot_hashes_sysvar.clone()],
                                    owner: vec![],
                                    filters: vec![],
                                    nonempty_txn_signature: None,
                                },
                            )]),
                            transactions: std::collections::HashMap::new(),
                            transactions_status: std::collections::HashMap::new(),
                            blocks: std::collections::HashMap::new(),
                            blocks_meta: std::collections::HashMap::new(),
                            entry: std::collections::HashMap::new(),
                            commitment: Some(
                                arete::runtime::yellowstone_grpc_proto::geyser::CommitmentLevel::Processed as i32
                            ),
                            accounts_data_slice: vec![],
                            ping: None,
                            from_slot: None,
                        };

                        let (sub_tx, mut stream) = client
                            .subscribe_with_request(Some(subscribe_request))
                            .await?;
                        // Keep sender alive for the duration of the stream
                        let _keep_alive = sub_tx;

                        arete::runtime::tracing::info!("[SLOT_SUB] Connected and subscribed to slot and SlotHashes updates");

                        while let Some(msg) = stream.next().await {
                            match msg {
                                Ok(update) => {
                                    match update.update_oneof {
                                        Some(UpdateOneof::Slot(slot_update)) => {
                                            slot_tracker.record(slot_update.slot);
                                            if let Some(ref health) = health_monitor {
                                                health.record_event().await;
                                            }
                                        }
                                        Some(UpdateOneof::Account(account_update)) => {
                                            if let Some(ref health) = health_monitor {
                                                health.record_event().await;
                                            }
                                            // Process SlotHashes sysvar update
                                            if let Some(account) = account_update.account {
                                                if arete::runtime::bs58::encode(&account.pubkey).into_string() == slot_hashes_sysvar {
                                                    arete::runtime::tracing::debug!(
                                                        slot = account_update.slot,
                                                        "[SLOT_SUB] Received SlotHashes sysvar update"
                                                    );
                                                    // Parse slot hashes from account data
                                                    // The SlotHashes sysvar contains a vector of (slot, hash) pairs
                                                    if let Err(e) = parse_and_cache_slot_hashes(
                                                        account_update.slot,
                                                        &account.data,
                                                    ) {
                                                        arete::runtime::tracing::warn!(
                                                            error = %e,
                                                            "[SLOT_SUB] Failed to parse SlotHashes"
                                                        );
                                                    }
                                                }
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                                Err(e) => {
                                    arete::runtime::tracing::warn!(
                                        error = %e,
                                        "[SLOT_SUB] Stream error, will reconnect"
                                    );
                                    break;
                                }
                            }
                        }

                        Ok(())
                    }.await;

                    if let Err(e) = result {
                        arete::runtime::tracing::warn!(
                            error = %e,
                            "[SLOT_SUB] Connection failed, reconnecting in 2s"
                        );
                    }

                    arete::runtime::tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                }
            });
        }
    }
}

/// Generate the VmHandler struct and its Handler trait implementations.
///
/// This is the single source of truth for VmHandler generation.
/// Uses MutationBatch with SlotContext for proper slot tracking.
pub fn generate_vm_handler(
    state_enum_name: &str,
    instruction_enum_name: &str,
    entity_name: &str,
) -> TokenStream {
    let state_enum = format_ident!("{}", state_enum_name);
    let instruction_enum = format_ident!("{}", instruction_enum_name);
    let entity_name_lit = entity_name;

    quote! {
        #[allow(dead_code)]
        const DEFAULT_DAS_BATCH_SIZE: usize = 100;
        #[allow(dead_code)]
        const DEFAULT_DAS_TIMEOUT_SECS: u64 = 10;

        #[allow(dead_code)]
        struct ResolverClient {
            endpoint: String,
            client: arete::runtime::reqwest::Client,
            batch_size: usize,
        }

        #[allow(dead_code)]
        impl ResolverClient {
            fn new(endpoint: String, batch_size: usize) -> arete::runtime::anyhow::Result<Self> {
                let client = arete::runtime::reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(DEFAULT_DAS_TIMEOUT_SECS))
                    .build()
                    .map_err(|err| {
                        arete::runtime::anyhow::anyhow!(
                            "Failed to build resolver HTTP client: {}",
                            err
                        )
                    })?;

                Ok(Self {
                    endpoint,
                    client,
                    batch_size: batch_size.max(1),
                })
            }

            async fn resolve_token_metadata(
                &self,
                mints: &[String],
            ) -> arete::runtime::anyhow::Result<
                std::collections::HashMap<String, arete::runtime::serde_json::Value>,
            > {
                let mut unique = std::collections::HashSet::new();
                let mut deduped = Vec::new();

                for mint in mints {
                    if mint.is_empty() {
                        continue;
                    }
                    if unique.insert(mint.clone()) {
                        deduped.push(mint.clone());
                    }
                }

                let mut results = std::collections::HashMap::new();
                if deduped.is_empty() {
                    return Ok(results);
                }

                for chunk in deduped.chunks(self.batch_size) {
                    let assets = self.fetch_assets(chunk).await?;
                    for asset in assets {
                        if let Some((mint, value)) = Self::build_token_metadata(&asset) {
                            results.insert(mint, value);
                        }
                    }
                }

                Ok(results)
            }

            async fn fetch_assets(
                &self,
                ids: &[String],
            ) -> arete::runtime::anyhow::Result<Vec<arete::runtime::serde_json::Value>> {
                let payload = arete::runtime::serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": "1",
                    "method": "getAssetBatch",
                    "params": {
                        "ids": ids,
                        "options": {
                            "showFungible": true,
                        },
                    },
                });

                let response = self
                    .client
                    .post(&self.endpoint)
                    .json(&payload)
                    .send()
                    .await
                    .map_err(|err| {
                        arete::runtime::anyhow::anyhow!(
                            "Resolver request failed: {}",
                            err
                        )
                    })?;

                let response = response.error_for_status().map_err(|err| {
                    arete::runtime::anyhow::anyhow!("Resolver request failed: {}", err)
                })?;

                let value = response
                    .json::<arete::runtime::serde_json::Value>()
                    .await
                    .map_err(|err| {
                        arete::runtime::anyhow::anyhow!(
                            "Resolver response parse failed: {}",
                            err
                        )
                    })?;

                if let Some(error) = value.get("error") {
                    return Err(arete::runtime::anyhow::anyhow!(
                        "Resolver response error: {}",
                        error
                    ));
                }

                let assets = value
                    .get("result")
                    .and_then(|result| match result {
                        arete::runtime::serde_json::Value::Array(items) => Some(items.clone()),
                        arete::runtime::serde_json::Value::Object(obj) => obj
                            .get("items")
                            .and_then(|items| items.as_array())
                            .map(|items| items.clone()),
                        _ => None,
                    })
                    .ok_or_else(|| {
                        arete::runtime::anyhow::anyhow!("Resolver response missing result")
                    })?;

                // Filter out null entries (DAS returns null for assets not in the index)
                let assets = assets.into_iter().filter(|a| !a.is_null()).collect();

                Ok(assets)
            }

            fn build_token_metadata(
                asset: &arete::runtime::serde_json::Value,
            ) -> Option<(String, arete::runtime::serde_json::Value)> {
                let mint = asset.get("id").and_then(|value| value.as_str())?.to_string();

                let name = asset
                    .pointer("/content/metadata/name")
                    .and_then(|value| value.as_str());

                let symbol = asset
                    .pointer("/content/metadata/symbol")
                    .and_then(|value| value.as_str());

                let token_info = asset
                    .get("token_info")
                    .or_else(|| asset.pointer("/content/token_info"));

                let decimals = token_info
                    .and_then(|info| info.get("decimals"))
                    .and_then(|value| value.as_u64());

                let logo_uri = asset
                    .pointer("/content/links/image")
                    .and_then(|value| value.as_str())
                    .or_else(|| {
                        asset
                            .pointer("/content/links/image_uri")
                            .and_then(|value| value.as_str())
                    });

                let mut obj = arete::runtime::serde_json::Map::new();
                obj.insert(
                    "mint".to_string(),
                    arete::runtime::serde_json::json!(mint),
                );
                obj.insert(
                    "name".to_string(),
                    name.map(|value| arete::runtime::serde_json::json!(value))
                        .unwrap_or(arete::runtime::serde_json::Value::Null),
                );
                obj.insert(
                    "symbol".to_string(),
                    symbol.map(|value| arete::runtime::serde_json::json!(value))
                        .unwrap_or(arete::runtime::serde_json::Value::Null),
                );
                obj.insert(
                    "decimals".to_string(),
                    decimals
                        .map(|value| arete::runtime::serde_json::json!(value))
                        .unwrap_or(arete::runtime::serde_json::Value::Null),
                );
                obj.insert(
                    "logo_uri".to_string(),
                    logo_uri
                        .map(|value| arete::runtime::serde_json::json!(value))
                        .unwrap_or(arete::runtime::serde_json::Value::Null),
                );

                Some((mint, arete::runtime::serde_json::Value::Object(obj)))
            }
        }

        const PROJECTOR_ENQUEUE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

        async fn reserve_projector_batch_slot(
            mutations_tx: arete::runtime::tokio::sync::mpsc::Sender<arete::runtime::arete_server::MutationBatch>,
            operation: &str,
        ) -> arete::runtime::tokio::sync::mpsc::OwnedPermit<arete::runtime::arete_server::MutationBatch> {
            match arete::runtime::tokio::time::timeout(PROJECTOR_ENQUEUE_TIMEOUT, mutations_tx.reserve_owned()).await {
                Ok(Ok(permit)) => permit,
                Ok(Err(_)) => {
                    arete::runtime::tracing::error!(
                        operation = %operation,
                        "Projector queue closed while reserving mutation capacity; exiting to avoid inconsistent VM state"
                    );
                    std::process::exit(1);
                }
                Err(_) => {
                    arete::runtime::tracing::error!(
                        operation = %operation,
                        timeout = ?PROJECTOR_ENQUEUE_TIMEOUT,
                        "Timed out waiting for projector queue capacity; exiting to avoid inconsistent VM state"
                    );
                    std::process::exit(1);
                }
            }
        }

        #[derive(Clone)]
        pub struct VmHandler {
            vm: std::sync::Arc<std::sync::Mutex<arete::runtime::arete_interpreter::vm::VmContext>>,
            bytecode: std::sync::Arc<arete::runtime::arete_interpreter::compiler::MultiEntityBytecode>,
            mutations_tx: arete::runtime::tokio::sync::mpsc::Sender<arete::runtime::arete_server::MutationBatch>,
            health_monitor: Option<arete::runtime::arete_server::HealthMonitor>,
            slot_tracker: arete::runtime::arete_server::SlotTracker,
            runtime_resolver: arete::runtime::arete_interpreter::runtime_resolvers::SharedRuntimeResolver,
            slot_scheduler: std::sync::Arc<std::sync::Mutex<arete::runtime::arete_interpreter::scheduler::SlotScheduler>>,
        }

        impl std::fmt::Debug for VmHandler {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_struct("VmHandler")
                    .field("vm", &"<VmContext>")
                    .field("bytecode", &"<MultiEntityBytecode>")
                    .finish()
            }
        }

        impl VmHandler {
            pub fn new(
                vm: std::sync::Arc<std::sync::Mutex<arete::runtime::arete_interpreter::vm::VmContext>>,
                bytecode: std::sync::Arc<arete::runtime::arete_interpreter::compiler::MultiEntityBytecode>,
                mutations_tx: arete::runtime::tokio::sync::mpsc::Sender<arete::runtime::arete_server::MutationBatch>,
                health_monitor: Option<arete::runtime::arete_server::HealthMonitor>,
                slot_tracker: arete::runtime::arete_server::SlotTracker,
                runtime_resolver: arete::runtime::arete_interpreter::runtime_resolvers::SharedRuntimeResolver,
                slot_scheduler: std::sync::Arc<std::sync::Mutex<arete::runtime::arete_interpreter::scheduler::SlotScheduler>>,
            ) -> Self {
                Self {
                    vm,
                    bytecode,
                    mutations_tx,
                    health_monitor,
                    slot_tracker,
                    runtime_resolver,
                    slot_scheduler,
                }
            }

            #[inline]
            async fn send_mutations_with_context(
                &self,
                mutations: Vec<arete::runtime::arete_interpreter::Mutation>,
                slot: u64,
                ordering: u64,
                event_context: Option<arete::runtime::arete_server::EventContext>,
                projector_permit: arete::runtime::tokio::sync::mpsc::OwnedPermit<arete::runtime::arete_server::MutationBatch>,
            ) {
                if !mutations.is_empty() {
                    let slot_context = arete::runtime::arete_server::SlotContext::new(slot, ordering);
                    let mut batch = arete::runtime::arete_server::MutationBatch::with_slot_context(
                        arete::runtime::smallvec::SmallVec::from_vec(mutations),
                        slot_context,
                    );
                    if let Some(ctx) = event_context {
                        batch = batch.with_event_context(ctx);
                    }
                    projector_permit.send(batch);
                }
            }

            async fn resolve_and_apply_resolvers(
                &self,
                requests: Vec<arete::runtime::arete_interpreter::vm::ResolverRequest>,
            ) -> Vec<arete::runtime::arete_interpreter::Mutation> {
                self.runtime_resolver
                    .resolve_and_apply(&self.vm, self.bytecode.as_ref(), requests)
                    .await
            }

            async fn reserve_mutation_batch_slot(
                &self,
                operation: &str,
            ) -> arete::runtime::tokio::sync::mpsc::OwnedPermit<arete::runtime::arete_server::MutationBatch> {
                reserve_projector_batch_slot(self.mutations_tx.clone(), operation).await
            }
        }

        impl arete::runtime::yellowstone_vixen::Handler<parsers::#state_enum, arete::runtime::yellowstone_vixen_core::AccountUpdate> for VmHandler {
            async fn handle(
                &self,
                value: &parsers::#state_enum,
                raw_update: &arete::runtime::yellowstone_vixen_core::AccountUpdate,
            ) -> arete::runtime::yellowstone_vixen::HandlerResult<()> {
                let slot = raw_update.slot;
                let account = raw_update.account.as_ref().unwrap();
                let write_version = account.write_version;
                let signature = arete::runtime::bs58::encode(account.txn_signature.as_ref().unwrap()).into_string();

                if let Some(ref health) = self.health_monitor {
                    health.record_event().await;
                }

                let account_address = arete::runtime::bs58::encode(&account.pubkey).into_string();

                let event_type = value.event_type();
                // Reserve downstream capacity before mutating VM state so a wedged
                // projector cannot leave the parser ahead of published batches.
                let projector_permit = self.reserve_mutation_batch_slot(event_type).await;

                let mut log = arete::runtime::arete_interpreter::CanonicalLog::new();
                log.set("phase", "vixen")
                    .set("event_kind", "account")
                    .set("event_type", event_type)
                    .set("slot", slot)
                    .set("program", #entity_name_lit)
                    .set("account", account_address);
                let mut event_value = value.to_value();

                if let Some(obj) = event_value.as_object_mut() {
                    obj.insert("__account_address".to_string(), arete::runtime::serde_json::json!(account_address));
                }

                let resolver_result = {
                    let mut vm = self.vm.lock().unwrap_or_else(|e| e.into_inner());

                    if let Some(state_table) = vm.get_state_table_mut(0) {
                        let mut ctx = arete::runtime::arete_interpreter::resolvers::ResolveContext::new(
                            0,
                            slot,
                            signature.clone(),
                            &mut state_table.pda_reverse_lookups,
                        );

                        if let Some(resolver_fn) = get_resolver_for_account_type(event_type) {
                            resolver_fn(&account_address, &event_value, &mut ctx)
                        } else {
                            arete::runtime::arete_interpreter::resolvers::KeyResolution::Found(String::new())
                        }
                    } else {
                        arete::runtime::arete_interpreter::resolvers::KeyResolution::Found(String::new())
                    }
                };

                match resolver_result {
                    arete::runtime::arete_interpreter::resolvers::KeyResolution::Found(resolved_key) => {
                        arete::runtime::tracing::info!(
                            event_type = %event_type,
                            account = %account_address,
                            resolved_key = %resolved_key,
                            slot = slot,
                            "[PDA] Account key resolution: Found"
                        );
                        if !resolved_key.is_empty() {
                            if let Some(obj) = event_value.as_object_mut() {
                                obj.insert("__resolved_primary_key".to_string(), arete::runtime::serde_json::json!(resolved_key));
                            }
                        }
                    }
                    arete::runtime::arete_interpreter::resolvers::KeyResolution::QueueUntil(_discriminators) => {
                        let mut vm = self.vm.lock().unwrap_or_else(|e| e.into_inner());
                        arete::runtime::tracing::info!(
                            event_type = %event_type,
                            pda = %account_address,
                            slot = slot,
                            "QueueUntil: queueing account update for later flush"
                        );

                        let _ = vm.queue_account_update(
                            0,
                            arete::runtime::arete_interpreter::QueuedAccountUpdate {
                                pda_address: account_address.clone(),
                                account_type: event_type.to_string(),
                                account_data: event_value,
                                slot,
                                write_version,
                                signature,
                            },
                        );
                        return Ok(());
                    }
                    arete::runtime::arete_interpreter::resolvers::KeyResolution::Skip => {
                        return Ok(());
                    }
                }

                let (mutations_result, resolver_requests, scheduled_callbacks) = {
                    let mut vm = self.vm.lock().unwrap_or_else(|e| e.into_inner());

                    let context = arete::runtime::arete_interpreter::UpdateContext::new_account(slot, signature.clone(), write_version);

                    // Clone event data before process_event so we can cache it
                    // for reprocessing when a PDA mapping changes at round boundaries.
                    let event_value_for_cache = event_value.clone();

                    let result = vm.process_event(&self.bytecode, event_value, event_type, Some(&context), Some(&mut log))
                        .map_err(|e| e.to_string());

                    // Cache the last account data per PDA address.  When a PDA
                    // mapping later changes (same PDA, different seed) the cached
                    // data is returned for reprocessing with the corrected mapping.
                    if result.is_ok() {
                        // Cache under every state_id that routes this event_type so that
                        // register_pda_reverse_lookup finds data for all participating entities.
                        let state_ids: std::collections::HashSet<u32> = self.bytecode.event_routing
                            .get(event_type)
                            .map(|entities| entities.iter()
                                .filter_map(|name| self.bytecode.entities.get(name).map(|eb| eb.state_id))
                                .collect())
                            .unwrap_or_default();
                        let pending = arete::runtime::arete_interpreter::PendingAccountUpdate {
                            account_type: event_type.to_string(),
                            pda_address: account_address.clone(),
                            account_data: event_value_for_cache,
                            slot,
                            write_version,
                            signature: signature.clone(),
                            queued_at: std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs() as i64,
                            is_stale_reprocess: false,
                        };
                        for state_id in state_ids {
                            vm.cache_last_account_data(state_id, &account_address, pending.clone());
                        }
                    }

                    let requests = if result.is_ok() {
                        vm.take_resolver_requests()
                    } else {
                        Vec::new()
                    };

                    let scheduled = if result.is_ok() {
                        vm.take_scheduled_callbacks()
                    } else {
                        Vec::new()
                    };

                    (result, requests, scheduled)
                };

                if !scheduled_callbacks.is_empty() {
                    let mut scheduler = self.slot_scheduler.lock().unwrap_or_else(|e| e.into_inner());
                    for (target_slot, callback) in scheduled_callbacks {
                        scheduler.register(target_slot, callback);
                    }
                }

                let resolver_mutations = if mutations_result.is_ok() {
                    self.resolve_and_apply_resolvers(resolver_requests).await
                } else {
                    Vec::new()
                };

                match mutations_result {
                    Ok(mut mutations) => {
                        // Combine primary mutations with resolver mutations into a single batch
                        // to avoid duplicate frames for the same entity key
                        mutations.extend(resolver_mutations);
                        let event_context = arete::runtime::arete_server::EventContext {
                            program: #entity_name_lit.to_string(),
                            event_kind: "account".to_string(),
                            event_type: event_type.to_string(),
                            account: Some(account_address),
                            accounts_count: None,
                        };
                        self.send_mutations_with_context(
                            mutations,
                            slot,
                            write_version,
                            Some(event_context),
                            projector_permit,
                        )
                        .await;
                        self.slot_tracker.record(slot);
                        Ok(())
                    }
                    Err(e) => {
                        if let Some(ref health) = self.health_monitor {
                            health.record_error(format!("VM error for {}: {}", event_type, e)).await;
                        }
                        Ok(())
                    }
                }
            }
        }

        impl arete::runtime::yellowstone_vixen::Handler<parsers::#instruction_enum, arete::runtime::yellowstone_vixen_core::instruction::InstructionUpdate> for VmHandler {
            async fn handle(
                &self,
                value: &parsers::#instruction_enum,
                raw_update: &arete::runtime::yellowstone_vixen_core::instruction::InstructionUpdate,
            ) -> arete::runtime::yellowstone_vixen::HandlerResult<()> {
                let slot = raw_update.shared.slot;
                let txn_index = raw_update.shared.txn_index;
                let signature = arete::runtime::bs58::encode(&raw_update.shared.signature).into_string();

                if let Some(ref health) = self.health_monitor {
                    health.record_event().await;
                }

                let static_keys_vec = &raw_update.accounts;
                let event_type = value.event_type();
                // Reserve downstream capacity before mutating VM state so a wedged
                // projector cannot leave the parser ahead of published batches.
                let projector_permit = self.reserve_mutation_batch_slot(event_type).await;

                let account_keys: Vec<String> = static_keys_vec
                    .iter()
                    .map(|key| {
                        let key_bytes: &[u8] = AsRef::<[u8]>::as_ref(key);
                        arete::runtime::bs58::encode(key_bytes).into_string()
                    })
                    .collect();
                let mut log = arete::runtime::arete_interpreter::CanonicalLog::new();
                log.set("phase", "vixen")
                    .set("event_kind", "instruction")
                    .set("event_type", event_type)
                    .set("slot", slot)
                    .set("txn_index", txn_index)
                    .set("program", #entity_name_lit)
                    .set("accounts", account_keys);
                let event_value = value.to_value_with_accounts(static_keys_vec);

                let bytecode = self.bytecode.clone();
                let (mutations_result, resolver_requests, scheduled_callbacks) = {
                    let mut vm = self.vm.lock().unwrap_or_else(|e| e.into_inner());

                    let context = arete::runtime::arete_interpreter::UpdateContext::new_instruction(slot, signature.clone(), txn_index);

                    let mut result = vm.process_event(&bytecode, event_value.clone(), event_type, Some(&context), Some(&mut log))
                        .map_err(|e| e.to_string());

                    if result.is_ok() {
                        let hooks = get_instruction_hooks(event_type);
                        if !hooks.is_empty() {
                            let accounts = event_value.get("accounts")
                                .and_then(|a| a.as_object())
                                .map(|obj| {
                                    obj.iter()
                                        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                                        .collect::<std::collections::HashMap<String, String>>()
                                })
                                .unwrap_or_default();

                            let instruction_data = event_value.get("data").unwrap_or(&arete::runtime::serde_json::Value::Null);

                            let timestamp = vm.current_context()
                                .map(|ctx| ctx.timestamp())
                                .unwrap_or_else(|| std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap()
                                    .as_secs() as i64);

                            // SAFETY: Carefully splitting mutable borrow into disjoint parts
                            let vm_ptr: *mut arete::runtime::arete_interpreter::vm::VmContext = &mut *vm as *mut arete::runtime::arete_interpreter::vm::VmContext;

                            let mut ctx = arete::runtime::arete_interpreter::resolvers::InstructionContext::with_metrics(
                                accounts,
                                0,
                                &mut *vm,
                                unsafe { (*vm_ptr).registers_mut() },
                                2,
                                unsafe { (*vm_ptr).path_cache() },
                                instruction_data,
                                Some(context.slot.unwrap_or(0)),
                                context.signature.clone(),
                                timestamp,
                            );

                            for hook_fn in hooks.iter() {
                                hook_fn(&mut ctx);
                            }

                            let pending_updates = ctx.take_pending_updates();

                            drop(ctx);

                            // Process pending account updates from instruction hooks
                            if !pending_updates.is_empty() {
                                arete::runtime::tracing::info!(
                                    count = pending_updates.len(),
                                    event_type = %event_type,
                                    "[PDA] Flushing pending account updates from instruction hooks"
                                );
                                for update in pending_updates {
                                    arete::runtime::tracing::info!(
                                        account_type = %update.account_type,
                                        pda = %update.pda_address,
                                        update_slot = update.slot,
                                        current_instruction_slot = slot,
                                        "[PDA] Reprocessing flushed update"
                                    );
                                    let resolved_key = vm.try_chained_pda_lookup(0, "default_pda_lookup", &update.pda_address);

                                    let mut account_data = update.account_data;
                                    if let Some(ref key) = resolved_key {
                                        arete::runtime::tracing::info!(
                                            pda = %update.pda_address,
                                            resolved_key = %key,
                                            "[PDA] Chained PDA lookup resolved for reprocessed update"
                                        );
                                        if let Some(obj) = account_data.as_object_mut() {
                                            obj.insert("__resolved_primary_key".to_string(), arete::runtime::serde_json::json!(key));
                                        }
                                    } else {
                                        arete::runtime::tracing::warn!(
                                            pda = %update.pda_address,
                                            "[PDA] Chained PDA lookup returned None for reprocessed update"
                                        );
                                    }

                                    let update_context = if update.is_stale_reprocess {
                                        arete::runtime::tracing::info!(
                                            pda = %update.pda_address,
                                            "[PDA] Using reprocessed context (empty sig, skip resolvers)"
                                        );
                                        arete::runtime::arete_interpreter::UpdateContext::new_reprocessed(
                                            update.slot,
                                            update.write_version,
                                        )
                                    } else {
                                        arete::runtime::arete_interpreter::UpdateContext::new_account(
                                            update.slot,
                                            update.signature.clone(),
                                            update.write_version,
                                        )
                                    };

                                    match vm.process_event(&bytecode, account_data, &update.account_type, Some(&update_context), None) {
                                        Ok(pending_mutations) => {
                                            arete::runtime::tracing::info!(
                                                account_type = %update.account_type,
                                                pda = %update.pda_address,
                                                mutations = pending_mutations.len(),
                                                is_stale = update.is_stale_reprocess,
                                                "[PDA] Reprocessed flushed account update"
                                            );
                                            if let Ok(ref mut mutations) = result {
                                                mutations.extend(pending_mutations);
                                            }
                                        }
                                        Err(e) => {
                                            arete::runtime::tracing::warn!(
                                                account_type = %update.account_type,
                                                error = %e,
                                                "[PDA] Failed to reprocess flushed account update"
                                            );
                                        }
                                    }
                                }
                            }
                        }

                        // Periodic cleanup
                        if vm.instructions_executed % 1000 == 0 {
                            let _ = vm.cleanup_all_expired(0);
                            let stats = vm.get_memory_stats(0);
                            arete::runtime::arete_interpreter::vm_metrics::record_memory_stats(&stats, #entity_name_lit);
                        }
                    }

                    let requests = if result.is_ok() {
                        vm.take_resolver_requests()
                    } else {
                        Vec::new()
                    };

                    let scheduled = if result.is_ok() {
                        vm.take_scheduled_callbacks()
                    } else {
                        Vec::new()
                    };

                    (result, requests, scheduled)
                };

                if !scheduled_callbacks.is_empty() {
                    let mut scheduler = self.slot_scheduler.lock().unwrap_or_else(|e| e.into_inner());
                    for (target_slot, callback) in scheduled_callbacks {
                        scheduler.register(target_slot, callback);
                    }
                }

                let resolver_mutations = if mutations_result.is_ok() {
                    self.resolve_and_apply_resolvers(resolver_requests).await
                } else {
                    Vec::new()
                };

                match mutations_result {
                    Ok(mut mutations) => {
                        // Combine primary mutations with resolver mutations into a single batch
                        // to avoid duplicate frames for the same entity key
                        mutations.extend(resolver_mutations);
                        let event_context = arete::runtime::arete_server::EventContext {
                            program: #entity_name_lit.to_string(),
                            event_kind: "instruction".to_string(),
                            event_type: event_type.to_string(),
                            account: None,
                            accounts_count: Some(static_keys_vec.len()),
                        };
                        self.send_mutations_with_context(
                            mutations,
                            slot,
                            txn_index as u64,
                            Some(event_context),
                            projector_permit,
                        )
                        .await;
                        self.slot_tracker.record(slot);
                        Ok(())
                    }
                    Err(e) => {
                        if let Some(ref health) = self.health_monitor {
                            health.record_error(format!("VM error for {}: {}", event_type, e)).await;
                        }
                        Ok(())
                    }
                }
            }
        }
    }
}

/// Generate the complete spec() function with runtime setup.
///
/// This consolidates the runtime loop generation that was previously duplicated
/// in `spec_fn.rs` and `idl_vixen_gen.rs`.
pub fn generate_spec_function(
    state_enum_name: &str,
    instruction_enum_name: &str,
    program_name: &str,
    config: &RuntimeGenConfig,
) -> TokenStream {
    let _state_enum = format_ident!("{}", state_enum_name);
    let _instruction_enum = format_ident!("{}", instruction_enum_name);

    let views_call = if config.include_views {
        quote! { .with_views(get_view_definitions()) }
    } else {
        quote! {}
    };

    let bytecode_logging = if config.verbose_bytecode_logging {
        quote! {
            arete::runtime::tracing::info!("Bytecode Handler Details:");
            for (entity_name, entity_bytecode) in &bytecode.entities {
                arete::runtime::tracing::info!("   Entity: {}", entity_name);
                for (event_type, handler_opcodes) in &entity_bytecode.handlers {
                    arete::runtime::tracing::info!("      {} -> {} opcodes", event_type, handler_opcodes.len());
                }
            }
        }
    } else {
        quote! {}
    };

    let parser_logging = if config.verbose_parser_logging {
        quote! {
            arete::runtime::tracing::info!("Registering parsers:");
            arete::runtime::tracing::info!("   - Account Parser ID: {}", arete::runtime::yellowstone_vixen_core::Parser::id(&account_parser));
            arete::runtime::tracing::info!("   - Instruction Parser ID: {}", arete::runtime::yellowstone_vixen_core::Parser::id(&instruction_parser));
        }
    } else {
        quote! {}
    };

    let slot_scheduler_task = generate_slot_scheduler_task();
    let slot_subscription_task = generate_slot_subscription_task();

    quote! {
        pub fn spec() -> arete::runtime::arete_server::Spec {
            let bytecode = create_multi_entity_bytecode();
            let program_id = parsers::PROGRAM_ID_STR.to_string();

            arete::runtime::arete_server::Spec::new(bytecode, program_id)
                .with_parser_setup(create_parser_setup())
                #views_call
        }

        fn create_parser_setup() -> arete::runtime::arete_server::ParserSetupFn {
            use std::sync::Arc;

            Arc::new(|mutations_tx, health_monitor, reconnection_config| {
                Box::pin(async move {
                    run_vixen_runtime_with_channel(mutations_tx, health_monitor, reconnection_config).await
                })
            })
        }

        async fn run_vixen_runtime_with_channel(
            mutations_tx: arete::runtime::tokio::sync::mpsc::Sender<arete::runtime::arete_server::MutationBatch>,
            health_monitor: Option<arete::runtime::arete_server::HealthMonitor>,
            reconnection_config: arete::runtime::arete_server::ReconnectionConfig,
        ) -> arete::runtime::anyhow::Result<()> {
            use arete::runtime::yellowstone_vixen::config::{BufferConfig, VixenConfig};
            use arete::runtime::yellowstone_vixen_yellowstone_grpc_source::YellowstoneGrpcConfig;
            use arete::runtime::yellowstone_vixen_yellowstone_grpc_source::YellowstoneGrpcSource;
            use arete::runtime::yellowstone_vixen::Pipeline;
            use std::sync::{Arc, Mutex};

            // Load environment variables
            let env_loaded = arete::runtime::dotenvy::from_filename(".env.local").is_ok()
                || arete::runtime::dotenvy::from_filename(".env").is_ok()
                || arete::runtime::dotenvy::dotenv().is_ok();

            if !env_loaded {
                arete::runtime::tracing::warn!("No .env file found. Make sure environment variables are set.");
            }

            let endpoint = std::env::var("YELLOWSTONE_ENDPOINT")
                .map_err(|_| arete::runtime::anyhow::anyhow!(
                    "YELLOWSTONE_ENDPOINT environment variable must be set.\n\
                     Example: export YELLOWSTONE_ENDPOINT=http://localhost:10000"
                ))?;
            let x_token = std::env::var("YELLOWSTONE_X_TOKEN").ok();

            let runtime_resolver: arete::runtime::arete_interpreter::runtime_resolvers::SharedRuntimeResolver =
                arete::runtime::arete_interpreter::runtime_resolvers_factory::build_resolver()
                    .map_err(|err| {
                        arete::runtime::anyhow::anyhow!(
                            "Failed to build runtime resolver: {}",
                            err
                        )
                    })?;

            let slot_tracker = arete::runtime::arete_server::SlotTracker::new();
            let slot_scheduler = Arc::new(Mutex::new(arete::runtime::arete_interpreter::scheduler::SlotScheduler::new()));
            let mut attempt = 0u32;
            let mut backoff = reconnection_config.initial_delay;

            let bytecode = create_multi_entity_bytecode();

            #bytecode_logging

            let vm = Arc::new(Mutex::new(arete::runtime::arete_interpreter::vm::VmContext::new()));
            let bytecode_arc = Arc::new(bytecode);

            // Spawn slot scheduler background task
            #slot_scheduler_task

            // Spawn dedicated gRPC slot subscription to drive the scheduler in real-time
            #slot_subscription_task

            loop {
                let from_slot = {
                    let last = slot_tracker.get();
                    if last > 0 { Some(last) } else { None }
                };

                if from_slot.is_some() {
                    arete::runtime::tracing::info!("Resuming from slot {}", from_slot.unwrap());
                }

                let vixen_config = VixenConfig {
                    source: YellowstoneGrpcConfig {
                        endpoint: endpoint.clone(),
                        x_token: x_token.clone(),
                        timeout: 60,
                        commitment_level: None,
                        from_slot,
                        accept_compression: None,
                        max_decoding_message_size: None,
                    },
                    buffer: BufferConfig::default(),
                };

                let handler = VmHandler::new(
                    vm.clone(),
                    bytecode_arc.clone(),
                    mutations_tx.clone(),
                    health_monitor.clone(),
                    slot_tracker.clone(),
                    runtime_resolver.clone(),
                    slot_scheduler.clone(),
                );

                let account_parser = parsers::AccountParser;
                let instruction_parser = parsers::InstructionParser;

                if attempt == 0 {
                    arete::runtime::tracing::info!("Starting yellowstone-vixen runtime for {} program", #program_name);
                    arete::runtime::tracing::info!("Program ID: {}", parsers::PROGRAM_ID_STR);
                    #parser_logging
                }

                if let Some(ref health) = health_monitor {
                    health.record_reconnecting().await;
                }

                let account_pipeline = Pipeline::new(account_parser, [handler.clone()]);
                let instruction_pipeline = Pipeline::new(instruction_parser, [handler]);

                if let Some(ref health) = health_monitor {
                    health.record_connection().await;
                }

                let result = arete::runtime::yellowstone_vixen::Runtime::<YellowstoneGrpcSource>::builder()
                    .account(account_pipeline)
                    .instruction(instruction_pipeline)
                    .build(vixen_config)
                    .try_run_async()
                    .await;

                if let Err(e) = result {
                    arete::runtime::tracing::error!("Vixen runtime error: {:?}", e);
                }

                attempt += 1;

                if let Some(max) = reconnection_config.max_attempts {
                    if attempt >= max {
                        arete::runtime::tracing::error!("Max reconnection attempts ({}) reached, giving up", max);
                        if let Some(ref health) = health_monitor {
                            health.record_error("Max reconnection attempts reached".into()).await;
                        }
                        return Err(arete::runtime::anyhow::anyhow!("Max reconnection attempts reached"));
                    }
                }

                arete::runtime::tracing::warn!(
                    "gRPC stream disconnected. Reconnecting in {:?} (attempt {})",
                    backoff,
                    attempt
                );

                if let Some(ref health) = health_monitor {
                    health.record_disconnection().await;
                }

                arete::runtime::tokio::time::sleep(backoff).await;

                backoff = reconnection_config.next_backoff(backoff);
            }
        }
    }
}

pub fn generate_vm_handler_struct() -> TokenStream {
    quote! {
        #[allow(dead_code)]
        const DEFAULT_DAS_BATCH_SIZE: usize = 100;
        #[allow(dead_code)]
        const DEFAULT_DAS_TIMEOUT_SECS: u64 = 10;

        #[allow(dead_code)]
        struct ResolverClient {
            endpoint: String,
            client: arete::runtime::reqwest::Client,
            batch_size: usize,
        }

        #[allow(dead_code)]
        impl ResolverClient {
            fn new(endpoint: String, batch_size: usize) -> arete::runtime::anyhow::Result<Self> {
                let client = arete::runtime::reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(DEFAULT_DAS_TIMEOUT_SECS))
                    .build()
                    .map_err(|err| {
                        arete::runtime::anyhow::anyhow!(
                            "Failed to build resolver HTTP client: {}",
                            err
                        )
                    })?;

                Ok(Self {
                    endpoint,
                    client,
                    batch_size: batch_size.max(1),
                })
            }

            async fn resolve_token_metadata(
                &self,
                mints: &[String],
            ) -> arete::runtime::anyhow::Result<
                std::collections::HashMap<String, arete::runtime::serde_json::Value>,
            > {
                let mut unique = std::collections::HashSet::new();
                let mut deduped = Vec::new();

                for mint in mints {
                    if mint.is_empty() {
                        continue;
                    }
                    if unique.insert(mint.clone()) {
                        deduped.push(mint.clone());
                    }
                }

                let mut results = std::collections::HashMap::new();
                if deduped.is_empty() {
                    return Ok(results);
                }

                for chunk in deduped.chunks(self.batch_size) {
                    let assets = self.fetch_assets(chunk).await?;
                    for asset in assets {
                        if let Some((mint, value)) = Self::build_token_metadata(&asset) {
                            results.insert(mint, value);
                        }
                    }
                }

                Ok(results)
            }

            async fn fetch_assets(
                &self,
                ids: &[String],
            ) -> arete::runtime::anyhow::Result<Vec<arete::runtime::serde_json::Value>> {
                let payload = arete::runtime::serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": "1",
                    "method": "getAssetBatch",
                    "params": {
                        "ids": ids,
                        "options": {
                            "showFungible": true,
                        },
                    },
                });

                let response = self
                    .client
                    .post(&self.endpoint)
                    .json(&payload)
                    .send()
                    .await
                    .map_err(|err| {
                        arete::runtime::anyhow::anyhow!(
                            "Resolver request failed: {}",
                            err
                        )
                    })?;

                let response = response.error_for_status().map_err(|err| {
                    arete::runtime::anyhow::anyhow!("Resolver request failed: {}", err)
                })?;

                let value = response
                    .json::<arete::runtime::serde_json::Value>()
                    .await
                    .map_err(|err| {
                        arete::runtime::anyhow::anyhow!(
                            "Resolver response parse failed: {}",
                            err
                        )
                    })?;

                if let Some(error) = value.get("error") {
                    return Err(arete::runtime::anyhow::anyhow!(
                        "Resolver response error: {}",
                        error
                    ));
                }

                let assets = value
                    .get("result")
                    .and_then(|result| match result {
                        arete::runtime::serde_json::Value::Array(items) => Some(items.clone()),
                        arete::runtime::serde_json::Value::Object(obj) => obj
                            .get("items")
                            .and_then(|items| items.as_array())
                            .map(|items| items.clone()),
                        _ => None,
                    })
                    .ok_or_else(|| {
                        arete::runtime::anyhow::anyhow!("Resolver response missing result")
                    })?;

                // Filter out null entries (DAS returns null for assets not in the index)
                let assets = assets.into_iter().filter(|a| !a.is_null()).collect();

                Ok(assets)
            }

            fn build_token_metadata(
                asset: &arete::runtime::serde_json::Value,
            ) -> Option<(String, arete::runtime::serde_json::Value)> {
                let mint = asset.get("id").and_then(|value| value.as_str())?.to_string();

                let name = asset
                    .pointer("/content/metadata/name")
                    .and_then(|value| value.as_str());

                let symbol = asset
                    .pointer("/content/metadata/symbol")
                    .and_then(|value| value.as_str());

                let token_info = asset
                    .get("token_info")
                    .or_else(|| asset.pointer("/content/token_info"));

                let decimals = token_info
                    .and_then(|info| info.get("decimals"))
                    .and_then(|value| value.as_u64());

                let logo_uri = asset
                    .pointer("/content/links/image")
                    .and_then(|value| value.as_str())
                    .or_else(|| {
                        asset
                            .pointer("/content/links/image_uri")
                            .and_then(|value| value.as_str())
                    });

                let mut obj = arete::runtime::serde_json::Map::new();
                obj.insert(
                    "mint".to_string(),
                    arete::runtime::serde_json::json!(mint),
                );
                obj.insert(
                    "name".to_string(),
                    name.map(|value| arete::runtime::serde_json::json!(value))
                        .unwrap_or(arete::runtime::serde_json::Value::Null),
                );
                obj.insert(
                    "symbol".to_string(),
                    symbol.map(|value| arete::runtime::serde_json::json!(value))
                        .unwrap_or(arete::runtime::serde_json::Value::Null),
                );
                obj.insert(
                    "decimals".to_string(),
                    decimals
                        .map(|value| arete::runtime::serde_json::json!(value))
                        .unwrap_or(arete::runtime::serde_json::Value::Null),
                );
                obj.insert(
                    "logo_uri".to_string(),
                    logo_uri
                        .map(|value| arete::runtime::serde_json::json!(value))
                        .unwrap_or(arete::runtime::serde_json::Value::Null),
                );

                Some((mint, arete::runtime::serde_json::Value::Object(obj)))
            }
        }

        const PROJECTOR_ENQUEUE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

        async fn reserve_projector_batch_slot(
            mutations_tx: arete::runtime::tokio::sync::mpsc::Sender<arete::runtime::arete_server::MutationBatch>,
            operation: &str,
        ) -> arete::runtime::tokio::sync::mpsc::OwnedPermit<arete::runtime::arete_server::MutationBatch> {
            match arete::runtime::tokio::time::timeout(PROJECTOR_ENQUEUE_TIMEOUT, mutations_tx.reserve_owned()).await {
                Ok(Ok(permit)) => permit,
                Ok(Err(_)) => {
                    arete::runtime::tracing::error!(
                        operation = %operation,
                        "Projector queue closed while reserving mutation capacity; exiting to avoid inconsistent VM state"
                    );
                    std::process::exit(1);
                }
                Err(_) => {
                    arete::runtime::tracing::error!(
                        operation = %operation,
                        timeout = ?PROJECTOR_ENQUEUE_TIMEOUT,
                        "Timed out waiting for projector queue capacity; exiting to avoid inconsistent VM state"
                    );
                    std::process::exit(1);
                }
            }
        }

        #[derive(Clone)]
        pub struct VmHandler {
            vm: std::sync::Arc<std::sync::Mutex<arete::runtime::arete_interpreter::vm::VmContext>>,
            bytecode: std::sync::Arc<arete::runtime::arete_interpreter::compiler::MultiEntityBytecode>,
            mutations_tx: arete::runtime::tokio::sync::mpsc::Sender<arete::runtime::arete_server::MutationBatch>,
            health_monitor: Option<arete::runtime::arete_server::HealthMonitor>,
            slot_tracker: arete::runtime::arete_server::SlotTracker,
            runtime_resolver: arete::runtime::arete_interpreter::runtime_resolvers::SharedRuntimeResolver,
            slot_scheduler: std::sync::Arc<std::sync::Mutex<arete::runtime::arete_interpreter::scheduler::SlotScheduler>>,
        }

        impl std::fmt::Debug for VmHandler {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_struct("VmHandler")
                    .field("vm", &"<VmContext>")
                    .field("bytecode", &"<MultiEntityBytecode>")
                    .finish()
            }
        }

        impl VmHandler {
            pub fn new(
                vm: std::sync::Arc<std::sync::Mutex<arete::runtime::arete_interpreter::vm::VmContext>>,
                bytecode: std::sync::Arc<arete::runtime::arete_interpreter::compiler::MultiEntityBytecode>,
                mutations_tx: arete::runtime::tokio::sync::mpsc::Sender<arete::runtime::arete_server::MutationBatch>,
                health_monitor: Option<arete::runtime::arete_server::HealthMonitor>,
                slot_tracker: arete::runtime::arete_server::SlotTracker,
                runtime_resolver: arete::runtime::arete_interpreter::runtime_resolvers::SharedRuntimeResolver,
                slot_scheduler: std::sync::Arc<std::sync::Mutex<arete::runtime::arete_interpreter::scheduler::SlotScheduler>>,
            ) -> Self {
                Self {
                    vm,
                    bytecode,
                    mutations_tx,
                    health_monitor,
                    slot_tracker,
                    runtime_resolver,
                    slot_scheduler,
                }
            }

            #[inline]
            async fn send_mutations_with_context(
                &self,
                mutations: Vec<arete::runtime::arete_interpreter::Mutation>,
                slot: u64,
                ordering: u64,
                event_context: Option<arete::runtime::arete_server::EventContext>,
                projector_permit: arete::runtime::tokio::sync::mpsc::OwnedPermit<arete::runtime::arete_server::MutationBatch>,
            ) {
                if !mutations.is_empty() {
                    let slot_context = arete::runtime::arete_server::SlotContext::new(slot, ordering);
                    let mut batch = arete::runtime::arete_server::MutationBatch::with_slot_context(
                        arete::runtime::smallvec::SmallVec::from_vec(mutations),
                        slot_context,
                    );
                    if let Some(ctx) = event_context {
                        batch = batch.with_event_context(ctx);
                    }
                    projector_permit.send(batch);
                }
            }

            async fn resolve_and_apply_resolvers(
                &self,
                requests: Vec<arete::runtime::arete_interpreter::vm::ResolverRequest>,
            ) -> Vec<arete::runtime::arete_interpreter::Mutation> {
                self.runtime_resolver
                    .resolve_and_apply(&self.vm, self.bytecode.as_ref(), requests)
                    .await
            }

            async fn reserve_mutation_batch_slot(
                &self,
                operation: &str,
            ) -> arete::runtime::tokio::sync::mpsc::OwnedPermit<arete::runtime::arete_server::MutationBatch> {
                reserve_projector_batch_slot(self.mutations_tx.clone(), operation).await
            }
        }
    }
}

pub fn generate_account_handler_impl(
    parser_module_name: &str,
    state_enum_name: &str,
) -> TokenStream {
    let parser_mod = format_ident!("{}", parser_module_name);
    let state_enum = format_ident!("{}", state_enum_name);
    let program_name_lit = parser_module_name;

    quote! {
        impl arete::runtime::yellowstone_vixen::Handler<#parser_mod::#state_enum, arete::runtime::yellowstone_vixen_core::AccountUpdate> for VmHandler {
            async fn handle(
                &self,
                value: &#parser_mod::#state_enum,
                raw_update: &arete::runtime::yellowstone_vixen_core::AccountUpdate,
            ) -> arete::runtime::yellowstone_vixen::HandlerResult<()> {
                let slot = raw_update.slot;
                let account = raw_update.account.as_ref().unwrap();
                let write_version = account.write_version;
                let signature = arete::runtime::bs58::encode(account.txn_signature.as_ref().unwrap()).into_string();

                if let Some(ref health) = self.health_monitor {
                    health.record_event().await;
                }

                let account_address = arete::runtime::bs58::encode(&account.pubkey).into_string();

                let event_type = value.event_type();
                // Reserve downstream capacity before mutating VM state so a wedged
                // projector cannot leave the parser ahead of published batches.
                let projector_permit = self.reserve_mutation_batch_slot(event_type).await;

                let mut log = arete::runtime::arete_interpreter::CanonicalLog::new();
                log.set("phase", "vixen")
                    .set("event_kind", "account")
                    .set("event_type", event_type)
                    .set("slot", slot)
                    .set("program", #program_name_lit)
                    .set("account", &account_address);
                let mut event_value = value.to_value();

                if let Some(obj) = event_value.as_object_mut() {
                    obj.insert("__account_address".to_string(), arete::runtime::serde_json::json!(account_address));
                }

                let resolver_result = {
                    let mut vm = self.vm.lock().unwrap_or_else(|e| e.into_inner());

                    if let Some(state_table) = vm.get_state_table_mut(0) {
                        let mut ctx = arete::runtime::arete_interpreter::resolvers::ResolveContext::new(
                            0,
                            slot,
                            signature.clone(),
                            &mut state_table.pda_reverse_lookups,
                        );

                        if let Some(resolver_fn) = get_resolver_for_account_type(event_type) {
                            resolver_fn(&account_address, &event_value, &mut ctx)
                        } else {
                            arete::runtime::arete_interpreter::resolvers::KeyResolution::Found(String::new())
                        }
                    } else {
                        arete::runtime::arete_interpreter::resolvers::KeyResolution::Found(String::new())
                    }
                };

                match resolver_result {
                    arete::runtime::arete_interpreter::resolvers::KeyResolution::Found(resolved_key) => {
                        arete::runtime::tracing::info!(
                            event_type = %event_type,
                            account = %account_address,
                            resolved_key = %resolved_key,
                            slot = slot,
                            "[PDA] Account key resolution: Found"
                        );
                        if !resolved_key.is_empty() {
                            if let Some(obj) = event_value.as_object_mut() {
                                obj.insert("__resolved_primary_key".to_string(), arete::runtime::serde_json::json!(resolved_key));
                            }
                        }
                    }
                    arete::runtime::arete_interpreter::resolvers::KeyResolution::QueueUntil(_discriminators) => {
                        let mut vm = self.vm.lock().unwrap_or_else(|e| e.into_inner());
                        let _ = vm.queue_account_update(
                            0,
                            arete::runtime::arete_interpreter::QueuedAccountUpdate {
                                pda_address: account_address.clone(),
                                account_type: event_type.to_string(),
                                account_data: event_value,
                                slot,
                                write_version,
                                signature,
                            },
                        );
                        return Ok(());
                    }
                    arete::runtime::arete_interpreter::resolvers::KeyResolution::Skip => {
                        return Ok(());
                    }
                }

                let (mutations_result, resolver_requests, scheduled_callbacks) = {
                    let mut vm = self.vm.lock().unwrap_or_else(|e| e.into_inner());

                    let context = arete::runtime::arete_interpreter::UpdateContext::new_account(slot, signature.clone(), write_version);

                    let event_value_for_cache = event_value.clone();

                    let result = vm.process_event(&self.bytecode, event_value, event_type, Some(&context), Some(&mut log))
                        .map_err(|e| e.to_string());

                    if result.is_ok() {
                        // Cache under every state_id that routes this event_type so that
                        // register_pda_reverse_lookup finds data for all participating entities.
                        let state_ids: std::collections::HashSet<u32> = self.bytecode.event_routing
                            .get(event_type)
                            .map(|entities| entities.iter()
                                .filter_map(|name| self.bytecode.entities.get(name).map(|eb| eb.state_id))
                                .collect())
                            .unwrap_or_default();
                        let pending = arete::runtime::arete_interpreter::PendingAccountUpdate {
                            account_type: event_type.to_string(),
                            pda_address: account_address.clone(),
                            account_data: event_value_for_cache,
                            slot,
                            write_version,
                            signature: signature.clone(),
                            queued_at: std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs() as i64,
                            is_stale_reprocess: false,
                        };
                        for state_id in state_ids {
                            vm.cache_last_account_data(state_id, &account_address, pending.clone());
                        }
                    }

                    let requests = if result.is_ok() {
                        vm.take_resolver_requests()
                    } else {
                        Vec::new()
                    };

                    let scheduled = if result.is_ok() {
                        vm.take_scheduled_callbacks()
                    } else {
                        Vec::new()
                    };

                    (result, requests, scheduled)
                };

                if !scheduled_callbacks.is_empty() {
                    let mut scheduler = self.slot_scheduler.lock().unwrap_or_else(|e| e.into_inner());
                    for (target_slot, callback) in scheduled_callbacks {
                        scheduler.register(target_slot, callback);
                    }
                }

                let resolver_mutations = if mutations_result.is_ok() {
                    self.resolve_and_apply_resolvers(resolver_requests).await
                } else {
                    Vec::new()
                };

                match mutations_result {
                    Ok(mut mutations) => {
                        // Combine primary mutations with resolver mutations into a single batch
                        // to avoid duplicate frames for the same entity key
                        mutations.extend(resolver_mutations);
                        let event_context = arete::runtime::arete_server::EventContext {
                            program: #program_name_lit.to_string(),
                            event_kind: "account".to_string(),
                            event_type: event_type.to_string(),
                            account: Some(account_address),
                            accounts_count: None,
                        };
                        self.send_mutations_with_context(
                            mutations,
                            slot,
                            write_version,
                            Some(event_context),
                            projector_permit,
                        )
                        .await;
                        self.slot_tracker.record(slot);
                        Ok(())
                    }
                    Err(e) => {
                        if let Some(ref health) = self.health_monitor {
                            health.record_error(format!("VM error for {}: {}", event_type, e)).await;
                        }
                        Ok(())
                    }
                }
            }
        }
    }
}

pub fn generate_instruction_handler_impl(
    parser_module_name: &str,
    instruction_enum_name: &str,
    entity_name: &str,
) -> TokenStream {
    let parser_mod = format_ident!("{}", parser_module_name);
    let instruction_enum = format_ident!("{}", instruction_enum_name);
    let entity_name_lit = entity_name;

    quote! {
        impl arete::runtime::yellowstone_vixen::Handler<#parser_mod::#instruction_enum, arete::runtime::yellowstone_vixen_core::instruction::InstructionUpdate> for VmHandler {
            async fn handle(
                &self,
                value: &#parser_mod::#instruction_enum,
                raw_update: &arete::runtime::yellowstone_vixen_core::instruction::InstructionUpdate,
            ) -> arete::runtime::yellowstone_vixen::HandlerResult<()> {
                let slot = raw_update.shared.slot;
                let txn_index = raw_update.shared.txn_index;
                let signature = arete::runtime::bs58::encode(&raw_update.shared.signature).into_string();

                if let Some(ref health) = self.health_monitor {
                    health.record_event().await;
                }

                let static_keys_vec = &raw_update.accounts;
                let event_type = value.event_type();
                // Reserve downstream capacity before mutating VM state so a wedged
                // projector cannot leave the parser ahead of published batches.
                let projector_permit = self.reserve_mutation_batch_slot(event_type).await;

                let mut log = arete::runtime::arete_interpreter::CanonicalLog::new();
                log.set("phase", "vixen")
                    .set("event_kind", "instruction")
                    .set("event_type", event_type)
                    .set("slot", slot)
                    .set("txn_index", txn_index)
                    .set("program", #entity_name_lit)
                    .set("accounts_count", static_keys_vec.len());
                let event_value = value.to_value_with_accounts(static_keys_vec);

                let bytecode = self.bytecode.clone();
                let (mutations_result, resolver_requests, scheduled_callbacks) = {
                    let mut vm = self.vm.lock().unwrap_or_else(|e| e.into_inner());

                    let context = arete::runtime::arete_interpreter::UpdateContext::new_instruction(slot, signature.clone(), txn_index);

                    let mut result = vm.process_event(&bytecode, event_value.clone(), event_type, Some(&context), Some(&mut log))
                        .map_err(|e| e.to_string());

                    if result.is_ok() {
                        let hooks = get_instruction_hooks(event_type);
                        if !hooks.is_empty() {
                            let accounts = event_value.get("accounts")
                                .and_then(|a| a.as_object())
                                .map(|obj| {
                                    obj.iter()
                                        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                                        .collect::<std::collections::HashMap<String, String>>()
                                })
                                .unwrap_or_default();

                            let instruction_data = event_value.get("data").unwrap_or(&arete::runtime::serde_json::Value::Null);

                            let timestamp = vm.current_context()
                                .map(|ctx| ctx.timestamp())
                                .unwrap_or_else(|| std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap()
                                    .as_secs() as i64);

                            let vm_ptr: *mut arete::runtime::arete_interpreter::vm::VmContext = &mut *vm as *mut arete::runtime::arete_interpreter::vm::VmContext;

                            let mut ctx = arete::runtime::arete_interpreter::resolvers::InstructionContext::with_metrics(
                                accounts,
                                0,
                                &mut *vm,
                                unsafe { (*vm_ptr).registers_mut() },
                                2,
                                unsafe { (*vm_ptr).path_cache() },
                                instruction_data,
                                Some(context.slot.unwrap_or(0)),
                                context.signature.clone(),
                                timestamp,
                            );

                            for hook_fn in hooks.iter() {
                                hook_fn(&mut ctx);
                            }

                            let pending_updates = ctx.take_pending_updates();

                            drop(ctx);

                            if !pending_updates.is_empty() {
                                arete::runtime::tracing::info!(
                                    count = pending_updates.len(),
                                    event_type = %event_type,
                                    "[PDA] Flushing pending account updates from instruction hooks"
                                );
                                for update in pending_updates {
                                    arete::runtime::tracing::info!(
                                        account_type = %update.account_type,
                                        pda = %update.pda_address,
                                        update_slot = update.slot,
                                        current_instruction_slot = slot,
                                        "[PDA] Reprocessing flushed update"
                                    );
                                    let resolved_key = vm.try_chained_pda_lookup(0, "default_pda_lookup", &update.pda_address);

                                    let mut account_data = update.account_data;
                                    if let Some(ref key) = resolved_key {
                                        arete::runtime::tracing::info!(
                                            pda = %update.pda_address,
                                            resolved_key = %key,
                                            "[PDA] Chained PDA lookup resolved for reprocessed update"
                                        );
                                        if let Some(obj) = account_data.as_object_mut() {
                                            obj.insert("__resolved_primary_key".to_string(), arete::runtime::serde_json::json!(key));
                                        }
                                    } else {
                                        arete::runtime::tracing::warn!(
                                            pda = %update.pda_address,
                                            "[PDA] Chained PDA lookup returned None for reprocessed update"
                                        );
                                    }

                                    let update_context = if update.is_stale_reprocess {
                                        arete::runtime::tracing::info!(
                                            pda = %update.pda_address,
                                            "[PDA] Using reprocessed context (empty sig, skip resolvers)"
                                        );
                                        arete::runtime::arete_interpreter::UpdateContext::new_reprocessed(
                                            update.slot,
                                            update.write_version,
                                        )
                                    } else {
                                        arete::runtime::arete_interpreter::UpdateContext::new_account(
                                            update.slot,
                                            update.signature.clone(),
                                            update.write_version,
                                        )
                                    };

                                    match vm.process_event(&bytecode, account_data, &update.account_type, Some(&update_context), None) {
                                        Ok(pending_mutations) => {
                                            arete::runtime::tracing::info!(
                                                account_type = %update.account_type,
                                                pda = %update.pda_address,
                                                mutations = pending_mutations.len(),
                                                is_stale = update.is_stale_reprocess,
                                                "[PDA] Reprocessed flushed account update"
                                            );
                                            if let Ok(ref mut mutations) = result {
                                                mutations.extend(pending_mutations);
                                            }
                                        }
                                        Err(e) => {
                                            arete::runtime::tracing::warn!(
                                                account_type = %update.account_type,
                                                error = %e,
                                                "[PDA] Flushed account reprocessing failed"
                                            );
                                        }
                                    }
                                }
                            }
                        }

                        if vm.instructions_executed % 1000 == 0 {
                            let _ = vm.cleanup_all_expired(0);
                            let stats = vm.get_memory_stats(0);
                            arete::runtime::arete_interpreter::vm_metrics::record_memory_stats(&stats, #entity_name_lit);
                        }
                    }

                    let requests = if result.is_ok() {
                        vm.take_resolver_requests()
                    } else {
                        Vec::new()
                    };

                    let scheduled = if result.is_ok() {
                        vm.take_scheduled_callbacks()
                    } else {
                        Vec::new()
                    };

                    (result, requests, scheduled)
                };

                if !scheduled_callbacks.is_empty() {
                    let mut scheduler = self.slot_scheduler.lock().unwrap_or_else(|e| e.into_inner());
                    for (target_slot, callback) in scheduled_callbacks {
                        scheduler.register(target_slot, callback);
                    }
                }

                let resolver_mutations = if mutations_result.is_ok() {
                    self.resolve_and_apply_resolvers(resolver_requests).await
                } else {
                    Vec::new()
                };

                match mutations_result {
                    Ok(mut mutations) => {
                        // Combine primary mutations with resolver mutations into a single batch
                        // to avoid duplicate frames for the same entity key
                        mutations.extend(resolver_mutations);
                        let event_context = arete::runtime::arete_server::EventContext {
                            program: #entity_name_lit.to_string(),
                            event_kind: "instruction".to_string(),
                            event_type: event_type.to_string(),
                            account: None,
                            accounts_count: Some(static_keys_vec.len()),
                        };
                        self.send_mutations_with_context(
                            mutations,
                            slot,
                            txn_index as u64,
                            Some(event_context),
                            projector_permit,
                        )
                        .await;
                        self.slot_tracker.record(slot);
                        Ok(())
                    }
                    Err(e) => {
                        if let Some(ref health) = self.health_monitor {
                            health.record_error(format!("VM error for {}: {}", event_type, e)).await;
                        }
                        Ok(())
                    }
                }
            }
        }
    }
}

#[allow(dead_code)]
pub struct PipelineInfo {
    pub parser_module_name: String,
    pub program_name: String,
    pub program_id: String,
    pub state_enum_name: String,
    pub instruction_enum_name: String,
}

pub fn generate_multi_pipeline_spec_function(
    pipelines: &[PipelineInfo],
    config: &RuntimeGenConfig,
) -> TokenStream {
    let primary = &pipelines[0];

    let views_call = if config.include_views {
        quote! { .with_views(get_view_definitions()) }
    } else {
        quote! {}
    };

    let bytecode_logging = if config.verbose_bytecode_logging {
        quote! {
            arete::runtime::tracing::info!("Bytecode Handler Details:");
            for (entity_name, entity_bytecode) in &bytecode.entities {
                arete::runtime::tracing::info!("   Entity: {}", entity_name);
                for (event_type, handler_opcodes) in &entity_bytecode.handlers {
                    arete::runtime::tracing::info!("      {} -> {} opcodes", event_type, handler_opcodes.len());
                }
            }
        }
    } else {
        quote! {}
    };

    let primary_parser_mod = format_ident!("{}", primary.parser_module_name);
    let primary_program_name_lit = &primary.program_name;

    let pipeline_creations: Vec<TokenStream> = pipelines
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let parser_mod = format_ident!("{}", p.parser_module_name);
            let acct_var = format_ident!("account_pipeline_{}", i);
            let ix_var = format_ident!("instruction_pipeline_{}", i);
            let is_last = i == pipelines.len() - 1;
            if is_last {
                quote! {
                    let #acct_var = Pipeline::new(#parser_mod::AccountParser, [handler.clone()]);
                    let #ix_var = Pipeline::new(#parser_mod::InstructionParser, [handler]);
                }
            } else {
                quote! {
                    let #acct_var = Pipeline::new(#parser_mod::AccountParser, [handler.clone()]);
                    let #ix_var = Pipeline::new(#parser_mod::InstructionParser, [handler.clone()]);
                }
            }
        })
        .collect();

    let pipeline_registrations: Vec<TokenStream> = pipelines
        .iter()
        .enumerate()
        .map(|(i, _)| {
            let acct_var = format_ident!("account_pipeline_{}", i);
            let ix_var = format_ident!("instruction_pipeline_{}", i);
            quote! {
                .account(#acct_var)
                .instruction(#ix_var)
            }
        })
        .collect();

    let parser_logging = if config.verbose_parser_logging {
        let log_stmts: Vec<TokenStream> = pipelines.iter().map(|p| {
            let parser_mod = format_ident!("{}", p.parser_module_name);
            let prog_name = &p.program_name;
            quote! {
                arete::runtime::tracing::info!("   - {} Account Parser ID: {}", #prog_name, arete::runtime::yellowstone_vixen_core::Parser::id(&#parser_mod::AccountParser));
                arete::runtime::tracing::info!("   - {} Instruction Parser ID: {}", #prog_name, arete::runtime::yellowstone_vixen_core::Parser::id(&#parser_mod::InstructionParser));
            }
        }).collect();

        quote! {
            arete::runtime::tracing::info!("Registering parsers:");
            #(#log_stmts)*
        }
    } else {
        quote! {}
    };

    let program_id_stmts: Vec<TokenStream> = pipelines.iter().map(|p| {
        let parser_mod = format_ident!("{}", p.parser_module_name);
        let prog_name = &p.program_name;
        quote! {
            arete::runtime::tracing::info!("   {} Program ID: {}", #prog_name, #parser_mod::PROGRAM_ID_STR);
        }
    }).collect();

    let slot_scheduler_task = generate_slot_scheduler_task();
    let slot_subscription_task = generate_slot_subscription_task();

    quote! {
        pub fn spec() -> arete::runtime::arete_server::Spec {
            let bytecode = create_multi_entity_bytecode();
            let program_id = #primary_parser_mod::PROGRAM_ID_STR.to_string();

            arete::runtime::arete_server::Spec::new(bytecode, program_id)
                .with_parser_setup(create_parser_setup())
                #views_call
        }

        fn create_parser_setup() -> arete::runtime::arete_server::ParserSetupFn {
            use std::sync::Arc;

            Arc::new(|mutations_tx, health_monitor, reconnection_config| {
                Box::pin(async move {
                    run_vixen_runtime_with_channel(mutations_tx, health_monitor, reconnection_config).await
                })
            })
        }

        async fn run_vixen_runtime_with_channel(
            mutations_tx: arete::runtime::tokio::sync::mpsc::Sender<arete::runtime::arete_server::MutationBatch>,
            health_monitor: Option<arete::runtime::arete_server::HealthMonitor>,
            reconnection_config: arete::runtime::arete_server::ReconnectionConfig,
        ) -> arete::runtime::anyhow::Result<()> {
            use arete::runtime::yellowstone_vixen::config::{BufferConfig, VixenConfig};
            use arete::runtime::yellowstone_vixen_yellowstone_grpc_source::YellowstoneGrpcConfig;
            use arete::runtime::yellowstone_vixen_yellowstone_grpc_source::YellowstoneGrpcSource;
            use arete::runtime::yellowstone_vixen::Pipeline;
            use std::sync::{Arc, Mutex};

            let env_loaded = arete::runtime::dotenvy::from_filename(".env.local").is_ok()
                || arete::runtime::dotenvy::from_filename(".env").is_ok()
                || arete::runtime::dotenvy::dotenv().is_ok();

            if !env_loaded {
                arete::runtime::tracing::warn!("No .env file found. Make sure environment variables are set.");
            }

            let endpoint = std::env::var("YELLOWSTONE_ENDPOINT")
                .map_err(|_| arete::runtime::anyhow::anyhow!(
                    "YELLOWSTONE_ENDPOINT environment variable must be set.\n\
                     Example: export YELLOWSTONE_ENDPOINT=http://localhost:10000"
                ))?;
            let x_token = std::env::var("YELLOWSTONE_X_TOKEN").ok();

            let runtime_resolver: arete::runtime::arete_interpreter::runtime_resolvers::SharedRuntimeResolver =
                arete::runtime::arete_interpreter::runtime_resolvers_factory::build_resolver()
                    .map_err(|err| {
                        arete::runtime::anyhow::anyhow!(
                            "Failed to build runtime resolver: {}",
                            err
                        )
                    })?;

            let slot_tracker = arete::runtime::arete_server::SlotTracker::new();
            let slot_scheduler = Arc::new(Mutex::new(arete::runtime::arete_interpreter::scheduler::SlotScheduler::new()));
            let mut attempt = 0u32;
            let mut backoff = reconnection_config.initial_delay;

            let bytecode = create_multi_entity_bytecode();

            #bytecode_logging

            let vm = Arc::new(Mutex::new(arete::runtime::arete_interpreter::vm::VmContext::new()));
            let bytecode_arc = Arc::new(bytecode);

            // Spawn slot scheduler background task
            #slot_scheduler_task

            // Spawn dedicated gRPC slot subscription to drive the scheduler in real-time
            #slot_subscription_task

            loop {
                let from_slot = {
                    let last = slot_tracker.get();
                    if last > 0 { Some(last) } else { None }
                };

                if from_slot.is_some() {
                    arete::runtime::tracing::info!("Resuming from slot {}", from_slot.unwrap());
                }

                let vixen_config = VixenConfig {
                    source: YellowstoneGrpcConfig {
                        endpoint: endpoint.clone(),
                        x_token: x_token.clone(),
                        timeout: 60,
                        commitment_level: None,
                        from_slot,
                        accept_compression: None,
                        max_decoding_message_size: None,
                    },
                    buffer: BufferConfig::default(),
                };

                let handler = VmHandler::new(
                    vm.clone(),
                    bytecode_arc.clone(),
                    mutations_tx.clone(),
                    health_monitor.clone(),
                    slot_tracker.clone(),
                    runtime_resolver.clone(),
                    slot_scheduler.clone(),
                );

                if attempt == 0 {
                    arete::runtime::tracing::info!("Starting yellowstone-vixen runtime for {} program", #primary_program_name_lit);
                    #(#program_id_stmts)*
                    #parser_logging
                }

                if let Some(ref health) = health_monitor {
                    health.record_reconnecting().await;
                }

                #(#pipeline_creations)*

                if let Some(ref health) = health_monitor {
                    health.record_connection().await;
                }

                let result = arete::runtime::yellowstone_vixen::Runtime::<YellowstoneGrpcSource>::builder()
                    #(#pipeline_registrations)*
                    .build(vixen_config)
                    .try_run_async()
                    .await;

                if let Err(e) = result {
                    arete::runtime::tracing::error!("Vixen runtime error: {:?}", e);
                }

                attempt += 1;

                if let Some(max) = reconnection_config.max_attempts {
                    if attempt >= max {
                        arete::runtime::tracing::error!("Max reconnection attempts ({}) reached, giving up", max);
                        if let Some(ref health) = health_monitor {
                            health.record_error("Max reconnection attempts reached".into()).await;
                        }
                        return Err(arete::runtime::anyhow::anyhow!("Max reconnection attempts reached"));
                    }
                }

                arete::runtime::tracing::warn!(
                    "gRPC stream disconnected. Reconnecting in {:?} (attempt {})",
                    backoff,
                    attempt
                );

                if let Some(ref health) = health_monitor {
                    health.record_disconnection().await;
                }

                arete::runtime::tokio::time::sleep(backoff).await;

                backoff = reconnection_config.next_backoff(backoff);
            }
        }
    }
}

#[allow(dead_code)]
pub fn generate_runtime(
    state_enum_name: &str,
    instruction_enum_name: &str,
    entity_name: &str,
    config: &RuntimeGenConfig,
) -> TokenStream {
    let vm_handler = generate_vm_handler(state_enum_name, instruction_enum_name, entity_name);
    let spec_fn =
        generate_spec_function(state_enum_name, instruction_enum_name, entity_name, config);

    quote! {
        #vm_handler
        #spec_fn
    }
}
