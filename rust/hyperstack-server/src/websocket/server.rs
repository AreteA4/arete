use crate::bus::BusManager;
use crate::cache::{cmp_seq, EntityCache, SnapshotBatchConfig};
use crate::compression::maybe_compress;
use crate::view::{ViewIndex, ViewSpec};
use crate::websocket::client_manager::ClientManager;
use crate::websocket::frame::{
    transform_large_u64_to_strings, Frame, Mode, SnapshotEntity, SnapshotFrame, SortConfig,
    SortOrder, SubscribedFrame,
};
use crate::websocket::subscription::{ClientMessage, Subscription};
use anyhow::Result;
use bytes::Bytes;
use futures_util::StreamExt;
use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::Arc;
#[cfg(feature = "otel")]
use std::time::Instant;

use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::accept_async;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, info_span, warn, Instrument};
use uuid::Uuid;

#[cfg(feature = "otel")]
use crate::metrics::Metrics;

struct SubscriptionContext<'a> {
    client_id: Uuid,
    client_manager: &'a ClientManager,
    bus_manager: &'a BusManager,
    entity_cache: &'a EntityCache,
    view_index: &'a ViewIndex,
    #[cfg(feature = "otel")]
    metrics: Option<Arc<Metrics>>,
}

pub struct WebSocketServer {
    bind_addr: SocketAddr,
    client_manager: ClientManager,
    bus_manager: BusManager,
    entity_cache: EntityCache,
    view_index: Arc<ViewIndex>,
    max_clients: usize,
    #[cfg(feature = "otel")]
    metrics: Option<Arc<Metrics>>,
}

impl WebSocketServer {
    #[cfg(feature = "otel")]
    pub fn new(
        bind_addr: SocketAddr,
        bus_manager: BusManager,
        entity_cache: EntityCache,
        view_index: Arc<ViewIndex>,
        metrics: Option<Arc<Metrics>>,
    ) -> Self {
        Self {
            bind_addr,
            client_manager: ClientManager::new(),
            bus_manager,
            entity_cache,
            view_index,
            max_clients: 10000,
            metrics,
        }
    }

    #[cfg(not(feature = "otel"))]
    pub fn new(
        bind_addr: SocketAddr,
        bus_manager: BusManager,
        entity_cache: EntityCache,
        view_index: Arc<ViewIndex>,
    ) -> Self {
        Self {
            bind_addr,
            client_manager: ClientManager::new(),
            bus_manager,
            entity_cache,
            view_index,
            max_clients: 10000,
        }
    }

    pub fn with_max_clients(mut self, max_clients: usize) -> Self {
        self.max_clients = max_clients;
        self
    }

    pub async fn start(self) -> Result<()> {
        info!(
            "Starting WebSocket server on {} (max_clients: {})",
            self.bind_addr, self.max_clients
        );

        let listener = TcpListener::bind(&self.bind_addr).await?;
        info!("WebSocket server listening on {}", self.bind_addr);

        self.client_manager.start_cleanup_task();

        loop {
            match listener.accept().await {
                Ok((stream, addr)) => {
                    let client_count = self.client_manager.client_count();
                    if client_count >= self.max_clients {
                        warn!(
                            "Rejecting connection from {} - max clients ({}) reached",
                            addr, self.max_clients
                        );
                        drop(stream);
                        continue;
                    }

                    #[cfg(feature = "otel")]
                    if let Some(ref metrics) = self.metrics {
                        metrics.record_ws_connection();
                    }

                    info!(
                        "New WebSocket connection from {} ({}/{} clients)",
                        addr,
                        client_count + 1,
                        self.max_clients
                    );
                    let client_manager = self.client_manager.clone();
                    let bus_manager = self.bus_manager.clone();
                    let entity_cache = self.entity_cache.clone();
                    let view_index = self.view_index.clone();
                    #[cfg(feature = "otel")]
                    let metrics = self.metrics.clone();

                    tokio::spawn(
                        async move {
                            #[cfg(feature = "otel")]
                            let result = handle_connection(
                                stream,
                                client_manager,
                                bus_manager,
                                entity_cache,
                                view_index,
                                metrics,
                            )
                            .await;
                            #[cfg(not(feature = "otel"))]
                            let result = handle_connection(
                                stream,
                                client_manager,
                                bus_manager,
                                entity_cache,
                                view_index,
                            )
                            .await;

                            if let Err(e) = result {
                                error!("WebSocket connection error: {}", e);
                            }
                        }
                        .instrument(info_span!("ws.connection", %addr)),
                    );
                }
                Err(e) => {
                    error!("Failed to accept connection: {}", e);
                }
            }
        }
    }
}

#[cfg(feature = "otel")]
async fn handle_connection(
    stream: TcpStream,
    client_manager: ClientManager,
    bus_manager: BusManager,
    entity_cache: EntityCache,
    view_index: Arc<ViewIndex>,
    metrics: Option<Arc<Metrics>>,
) -> Result<()> {
    let ws_stream = accept_async(stream).await?;
    let client_id = Uuid::new_v4();
    let connection_start = Instant::now();

    info!("WebSocket connection established for client {}", client_id);

    let (ws_sender, mut ws_receiver) = ws_stream.split();

    client_manager.add_client(client_id, ws_sender);

    let ctx = SubscriptionContext {
        client_id,
        client_manager: &client_manager,
        bus_manager: &bus_manager,
        entity_cache: &entity_cache,
        view_index: &view_index,
        metrics: metrics.clone(),
    };

    let mut active_subscriptions: Vec<String> = Vec::new();

    loop {
        tokio::select! {
            ws_msg = ws_receiver.next() => {
                match ws_msg {
                    Some(Ok(msg)) => {
                        if msg.is_close() {
                            info!("Client {} requested close", client_id);
                            break;
                        }

                        client_manager.update_client_last_seen(client_id);

                        if msg.is_text() {
                            if let Some(ref m) = metrics {
                                m.record_ws_message_received();
                            }

                            if let Ok(text) = msg.to_text() {
                                debug!("Received text message from client {}: {}", client_id, text);

                                if let Ok(client_msg) = serde_json::from_str::<ClientMessage>(text) {
                                    match client_msg {
                                        ClientMessage::Subscribe(subscription) => {
                                            let view_id = subscription.view.clone();
                                            let sub_key = subscription.sub_key();
                                            client_manager.update_subscription(client_id, subscription.clone());

                                            let cancel_token = CancellationToken::new();
                                            let is_new = client_manager.add_client_subscription(
                                                client_id,
                                                sub_key.clone(),
                                                cancel_token.clone(),
                                            ).await;

                                            if !is_new {
                                                debug!("Client {} already subscribed to {}, ignoring duplicate", client_id, sub_key);
                                                continue;
                                            }

                                            if let Some(ref m) = metrics {
                                                m.record_subscription_created(&view_id);
                                            }
                                            active_subscriptions.push(view_id);

                                            attach_client_to_bus(&ctx, subscription, cancel_token).await;
                                        }
                                        ClientMessage::Unsubscribe(unsub) => {
                                            let sub_key = unsub.sub_key();
                                            let removed = client_manager
                                                .remove_client_subscription(client_id, &sub_key)
                                                .await;

                                            if removed {
                                                info!("Client {} unsubscribed from {}", client_id, sub_key);
                                                if let Some(ref m) = metrics {
                                                    m.record_subscription_removed(&unsub.view);
                                                }
                                            }
                                        }
                                        ClientMessage::Ping => {
                                            debug!("Received ping from client {}", client_id);
                                        }
                                    }
                                } else if let Ok(subscription) = serde_json::from_str::<Subscription>(text) {
                                    let view_id = subscription.view.clone();
                                    let sub_key = subscription.sub_key();
                                    client_manager.update_subscription(client_id, subscription.clone());

                                    let cancel_token = CancellationToken::new();
                                    let is_new = client_manager.add_client_subscription(
                                        client_id,
                                        sub_key.clone(),
                                        cancel_token.clone(),
                                    ).await;

                                    if !is_new {
                                        debug!("Client {} already subscribed to {}, ignoring duplicate", client_id, sub_key);
                                        continue;
                                    }

                                    if let Some(ref m) = metrics {
                                        m.record_subscription_created(&view_id);
                                    }
                                    active_subscriptions.push(view_id);

                                    attach_client_to_bus(&ctx, subscription, cancel_token).await;
                                } else {
                                    debug!("Received non-subscription message from client {}: {}", client_id, text);
                                }
                            }
                        }
                    }
                    Some(Err(e)) => {
                        warn!("WebSocket error for client {}: {}", client_id, e);
                        break;
                    }
                    None => {
                        debug!("WebSocket stream ended for client {}", client_id);
                        break;
                    }
                }
            }
        }
    }

    client_manager
        .cancel_all_client_subscriptions(client_id)
        .await;
    client_manager.remove_client(client_id);

    if let Some(ref m) = metrics {
        let duration_secs = connection_start.elapsed().as_secs_f64();
        m.record_ws_disconnection(duration_secs);

        for view_id in active_subscriptions {
            m.record_subscription_removed(&view_id);
        }
    }

    info!("Client {} disconnected", client_id);

    Ok(())
}

#[cfg(not(feature = "otel"))]
async fn handle_connection(
    stream: TcpStream,
    client_manager: ClientManager,
    bus_manager: BusManager,
    entity_cache: EntityCache,
    view_index: Arc<ViewIndex>,
) -> Result<()> {
    let ws_stream = accept_async(stream).await?;
    let client_id = Uuid::new_v4();

    info!("WebSocket connection established for client {}", client_id);

    let (ws_sender, mut ws_receiver) = ws_stream.split();

    client_manager.add_client(client_id, ws_sender);

    let ctx = SubscriptionContext {
        client_id,
        client_manager: &client_manager,
        bus_manager: &bus_manager,
        entity_cache: &entity_cache,
        view_index: &view_index,
    };

    loop {
        tokio::select! {
            ws_msg = ws_receiver.next() => {
                match ws_msg {
                    Some(Ok(msg)) => {
                        if msg.is_close() {
                            info!("Client {} requested close", client_id);
                            break;
                        }

                        client_manager.update_client_last_seen(client_id);

                        if msg.is_text() {
                            if let Ok(text) = msg.to_text() {
                                debug!("Received text message from client {}: {}", client_id, text);

                                if let Ok(client_msg) = serde_json::from_str::<ClientMessage>(text) {
                                    match client_msg {
                                        ClientMessage::Subscribe(subscription) => {
                                            let sub_key = subscription.sub_key();
                                            client_manager.update_subscription(client_id, subscription.clone());

                                            let cancel_token = CancellationToken::new();
                                            let is_new = client_manager.add_client_subscription(
                                                client_id,
                                                sub_key.clone(),
                                                cancel_token.clone(),
                                            ).await;

                                            if !is_new {
                                                debug!("Client {} already subscribed to {}, ignoring duplicate", client_id, sub_key);
                                                continue;
                                            }

                                            attach_client_to_bus(&ctx, subscription, cancel_token).await;
                                        }
                                        ClientMessage::Unsubscribe(unsub) => {
                                            let sub_key = unsub.sub_key();
                                            let removed = client_manager
                                                .remove_client_subscription(client_id, &sub_key)
                                                .await;

                                            if removed {
                                                info!("Client {} unsubscribed from {}", client_id, sub_key);
                                            }
                                        }
                                        ClientMessage::Ping => {
                                            debug!("Received ping from client {}", client_id);
                                        }
                                    }
                                } else if let Ok(subscription) = serde_json::from_str::<Subscription>(text) {
                                    let sub_key = subscription.sub_key();
                                    client_manager.update_subscription(client_id, subscription.clone());

                                    let cancel_token = CancellationToken::new();
                                    let is_new = client_manager.add_client_subscription(
                                        client_id,
                                        sub_key.clone(),
                                        cancel_token.clone(),
                                    ).await;

                                    if !is_new {
                                        debug!("Client {} already subscribed to {}, ignoring duplicate", client_id, sub_key);
                                        continue;
                                    }

                                    attach_client_to_bus(&ctx, subscription, cancel_token).await;
                                } else {
                                    debug!("Received non-subscription message from client {}: {}", client_id, text);
                                }
                            }
                        }
                    }
                    Some(Err(e)) => {
                        warn!("WebSocket error for client {}: {}", client_id, e);
                        break;
                    }
                    None => {
                        debug!("WebSocket stream ended for client {}", client_id);
                        break;
                    }
                }
            }
        }
    }

    client_manager
        .cancel_all_client_subscriptions(client_id)
        .await;
    client_manager.remove_client(client_id);
    info!("Client {} disconnected", client_id);

    Ok(())
}

async fn send_snapshot_batches(
    client_id: Uuid,
    entities: &[SnapshotEntity],
    mode: Mode,
    view_id: &str,
    client_manager: &ClientManager,
    batch_config: &SnapshotBatchConfig,
    #[cfg(feature = "otel")] metrics: Option<&Arc<Metrics>>,
) -> Result<()> {
    let total = entities.len();
    if total == 0 {
        return Ok(());
    }

    let mut offset = 0;
    let mut batch_num = 0;

    while offset < total {
        let batch_size = if batch_num == 0 {
            batch_config.initial_batch_size
        } else {
            batch_config.subsequent_batch_size
        };

        let end = (offset + batch_size).min(total);
        let batch_data: Vec<SnapshotEntity> = entities[offset..end].to_vec();
        let is_complete = end >= total;

        let snapshot_frame = SnapshotFrame {
            mode,
            export: view_id.to_string(),
            op: "snapshot",
            data: batch_data,
            complete: is_complete,
        };

        if let Ok(json_payload) = serde_json::to_vec(&snapshot_frame) {
            let payload = maybe_compress(&json_payload);
            if client_manager
                .send_compressed_async(client_id, payload)
                .await
                .is_err()
            {
                return Err(anyhow::anyhow!("Failed to send snapshot batch"));
            }
            #[cfg(feature = "otel")]
            if let Some(m) = metrics {
                m.record_ws_message_sent();
            }
        }

        offset = end;
        batch_num += 1;
    }

    debug!(
        "Sent {} snapshot batches ({} entities) for {} to client {}",
        batch_num, total, view_id, client_id
    );

    Ok(())
}

fn extract_sort_config(view_spec: &ViewSpec) -> Option<SortConfig> {
    if let Some(sort) = view_spec.pipeline.as_ref().and_then(|p| p.sort.as_ref()) {
        return Some(SortConfig {
            field: sort.field_path.clone(),
            order: match sort.order {
                crate::materialized_view::SortOrder::Asc => SortOrder::Asc,
                crate::materialized_view::SortOrder::Desc => SortOrder::Desc,
            },
        });
    }

    if view_spec.mode == Mode::List {
        return Some(SortConfig {
            field: vec!["_seq".to_string()],
            order: SortOrder::Desc,
        });
    }

    None
}

fn send_subscribed_frame(
    client_id: Uuid,
    view_id: &str,
    view_spec: &ViewSpec,
    client_manager: &ClientManager,
) -> Result<()> {
    let sort_config = extract_sort_config(view_spec);
    let subscribed_frame = SubscribedFrame::new(view_id.to_string(), view_spec.mode, sort_config);

    let json_payload = serde_json::to_vec(&subscribed_frame)?;
    let payload = Arc::new(Bytes::from(json_payload));
    client_manager
        .send_to_client(client_id, payload)
        .map_err(|e| anyhow::anyhow!("Failed to send subscribed frame: {:?}", e))
}

#[cfg(feature = "otel")]
async fn attach_client_to_bus(
    ctx: &SubscriptionContext<'_>,
    subscription: Subscription,
    cancel_token: CancellationToken,
) {
    let view_id = &subscription.view;

    let view_spec = match ctx.view_index.get_view(view_id) {
        Some(spec) => spec.clone(),
        None => {
            warn!("Unknown view ID: {}", view_id);
            return;
        }
    };

    if let Err(e) = send_subscribed_frame(ctx.client_id, view_id, &view_spec, ctx.client_manager) {
        warn!("Failed to send subscribed frame: {}", e);
        return;
    }

    let is_derived_with_sort = view_spec.is_derived()
        && view_spec
            .pipeline
            .as_ref()
            .map(|p| p.sort.is_some())
            .unwrap_or(false);

    if is_derived_with_sort {
        attach_derived_view_subscription_otel(ctx, subscription, view_spec, cancel_token).await;
        return;
    }

    match view_spec.mode {
        Mode::State => {
            let key = subscription.key.as_deref().unwrap_or("");

            let mut rx = ctx.bus_manager.get_or_create_state_bus(view_id, key).await;

            // Check if we should send snapshot (defaults to true for backward compatibility)
            let should_send_snapshot = subscription.with_snapshot.unwrap_or(true);

            if should_send_snapshot {
                if let Some(mut cached_entity) = ctx.entity_cache.get(view_id, key).await {
                    transform_large_u64_to_strings(&mut cached_entity);
                    let snapshot_entities = vec![SnapshotEntity {
                        key: key.to_string(),
                        data: cached_entity,
                    }];
                    let batch_config = ctx.entity_cache.snapshot_config();
                    let _ = send_snapshot_batches(
                        ctx.client_id,
                        &snapshot_entities,
                        view_spec.mode,
                        view_id,
                        ctx.client_manager,
                        &batch_config,
                        #[cfg(feature = "otel")]
                        ctx.metrics.as_ref(),
                    )
                    .await;
                    rx.borrow_and_update();
                } else if !rx.borrow().is_empty() {
                    let data = rx.borrow_and_update().clone();
                    let _ = ctx.client_manager.send_to_client(ctx.client_id, data);
                }
            } else {
                info!("Client {} subscribed to {} without snapshot", ctx.client_id, view_id);
                rx.borrow_and_update();
            }

            let client_id = ctx.client_id;
            let client_mgr = ctx.client_manager.clone();
            let metrics_clone = ctx.metrics.clone();
            let view_id_clone = view_id.clone();
            let key_clone = key.to_string();
            tokio::spawn(
                async move {
                    loop {
                        tokio::select! {
                            _ = cancel_token.cancelled() => {
                                debug!("State subscription cancelled for client {}", client_id);
                                break;
                            }
                            result = rx.changed() => {
                                if result.is_err() {
                                    break;
                                }
                                let data = rx.borrow().clone();
                                if client_mgr.send_to_client(client_id, data).is_err() {
                                    break;
                                }
                                if let Some(ref m) = metrics_clone {
                                    m.record_ws_message_sent();
                                }
                            }
                        }
                    }
                }
                .instrument(info_span!("ws.subscribe.state", %client_id, view = %view_id_clone, key = %key_clone)),
            );
        }
        Mode::List | Mode::Append => {
            let mut rx = ctx.bus_manager.get_or_create_list_bus(view_id).await;

            // Check if we should send snapshot (defaults to true for backward compatibility)
            let should_send_snapshot = subscription.with_snapshot.unwrap_or(true);

            if should_send_snapshot {
                // Determine which entities to send based on cursor
                let mut snapshots = if let Some(ref cursor) = subscription.after {
                    ctx.entity_cache.get_after(view_id, cursor, subscription.snapshot_limit).await
                } else {
                    ctx.entity_cache.get_all(view_id).await
                };

                // Sort by _seq descending only when there is no cursor (to get most-recent N from full cache)
                if let Some(limit) = subscription.snapshot_limit {
                    if subscription.after.is_none() {
                        snapshots.sort_by(|a, b| {
                            let sa = a.1.get("_seq").and_then(|s| s.as_str()).unwrap_or("");
                            let sb = b.1.get("_seq").and_then(|s| s.as_str()).unwrap_or("");
                            cmp_seq(sb, sa) // descending: most-recent N
                        });
                        snapshots.truncate(limit);
                    }
                }

                let snapshot_entities: Vec<SnapshotEntity> = snapshots
                    .into_iter()
                    .filter(|(key, _)| subscription.matches_key(key))
                    .map(|(key, mut data)| {
                        transform_large_u64_to_strings(&mut data);
                        SnapshotEntity { key, data }
                    })
                    .collect();

                if !snapshot_entities.is_empty() {
                    let batch_config = ctx.entity_cache.snapshot_config();
                    if send_snapshot_batches(
                        ctx.client_id,
                        &snapshot_entities,
                        view_spec.mode,
                        view_id,
                        ctx.client_manager,
                        &batch_config,
                        #[cfg(feature = "otel")]
                        ctx.metrics.as_ref(),
                    )
                    .await
                    .is_err()
                    {
                        return;
                    }
                }
            } else {
                info!("Client {} subscribed to {} without snapshot", ctx.client_id, view_id);
            }

            let client_id = ctx.client_id;
            let client_mgr = ctx.client_manager.clone();
            let sub = subscription.clone();
            let metrics_clone = ctx.metrics.clone();
            let view_id_clone = view_id.clone();
            let mode = view_spec.mode;
            tokio::spawn(
                async move {
                    loop {
                        tokio::select! {
                            _ = cancel_token.cancelled() => {
                                debug!("List subscription cancelled for client {}", client_id);
                                break;
                            }
                            result = rx.recv() => {
                                match result {
                                    Ok(envelope) => {
                                        if sub.matches(&envelope.entity, &envelope.key) {
                                            if client_mgr
                                                .send_to_client(client_id, envelope.payload.clone())
                                                .is_err()
                                            {
                                                break;
                                            }
                                            if let Some(ref m) = metrics_clone {
                                                m.record_ws_message_sent();
                                            }
                                        }
                                    }
                                    Err(_) => break,
                                }
                            }
                        }
                    }
                }
                .instrument(info_span!("ws.subscribe.list", %client_id, view = %view_id_clone, mode = ?mode)),
            );
        }
    }

    info!(
        "Client {} subscribed to {} (mode: {:?})",
        ctx.client_id, view_id, view_spec.mode
    );
}

#[cfg(feature = "otel")]
async fn attach_derived_view_subscription_otel(
    ctx: &SubscriptionContext<'_>,
    subscription: Subscription,
    view_spec: ViewSpec,
    cancel_token: CancellationToken,
) {
    let view_id = &subscription.view;
    let pipeline_limit = view_spec
        .pipeline
        .as_ref()
        .and_then(|p| p.limit)
        .unwrap_or(100);
    let take = subscription.take.unwrap_or(pipeline_limit);
    let skip = subscription.skip.unwrap_or(0);
    let is_single = take == 1;

    let source_view_id = match &view_spec.source_view {
        Some(s) => s.clone(),
        None => {
            warn!("Derived view {} has no source_view", view_id);
            return;
        }
    };

    let sorted_caches = ctx.view_index.sorted_caches();
    let initial_window: Vec<(String, serde_json::Value)> = {
        let mut caches = sorted_caches.write().await;
        if let Some(cache) = caches.get_mut(view_id) {
            cache.get_window(skip, take)
        } else {
            warn!("No sorted cache for derived view {}", view_id);
            vec![]
        }
    };

    let initial_keys: HashSet<String> = initial_window.iter().map(|(k, _)| k.clone()).collect();

    if !initial_window.is_empty() {
        let snapshot_entities: Vec<SnapshotEntity> = initial_window
            .into_iter()
            .map(|(key, mut data)| {
                transform_large_u64_to_strings(&mut data);
                SnapshotEntity { key, data }
            })
            .collect();

        let batch_config = ctx.entity_cache.snapshot_config();
        if send_snapshot_batches(
            ctx.client_id,
            &snapshot_entities,
            view_spec.mode,
            view_id,
            ctx.client_manager,
            &batch_config,
            ctx.metrics.as_ref(),
        )
        .await
        .is_err()
        {
            return;
        }
    }

    let mut rx = ctx
        .bus_manager
        .get_or_create_list_bus(&source_view_id)
        .await;

    let client_id = ctx.client_id;
    let client_mgr = ctx.client_manager.clone();
    let view_id_clone = view_id.clone();
    let view_id_span = view_id.clone();
    let sorted_caches_clone = sorted_caches;
    let metrics_clone = ctx.metrics.clone();
    let frame_mode = view_spec.mode;

    tokio::spawn(
        async move {
            let mut current_window_keys = initial_keys;

            loop {
                tokio::select! {
                    _ = cancel_token.cancelled() => {
                        debug!("Derived view subscription cancelled for client {}", client_id);
                        break;
                    }
                    result = rx.recv() => {
                        match result {
                            Ok(_envelope) => {
                                let new_window: Vec<(String, serde_json::Value)> = {
                                    let mut caches = sorted_caches_clone.write().await;
                                    if let Some(cache) = caches.get_mut(&view_id_clone) {
                                        cache.get_window(skip, take)
                                    } else {
                                        continue;
                                    }
                                };

                                let new_keys: HashSet<String> =
                                    new_window.iter().map(|(k, _)| k.clone()).collect();

                                if is_single {
                                    if let Some((new_key, data)) = new_window.first() {
                                        for old_key in current_window_keys.difference(&new_keys) {
                                            let delete_frame = Frame {
                                            seq: None,
                                                mode: frame_mode,
                                                export: view_id_clone.clone(),
                                                op: "delete",
                                                key: old_key.clone(),
                                                data: serde_json::Value::Null,
                                                append: vec![],
                                            };
                                            if let Ok(json) = serde_json::to_vec(&delete_frame) {
                                                let payload = Arc::new(Bytes::from(json));
                                                if client_mgr.send_to_client(client_id, payload).is_err() {
                                                    return;
                                                }
                                                if let Some(ref m) = metrics_clone {
                                                    m.record_ws_message_sent();
                                                }
                                            }
                                        }

                                        let mut transformed_data = data.clone();
                                        transform_large_u64_to_strings(&mut transformed_data);
                                        let frame = Frame {
                                            seq: None,
                                            mode: frame_mode,
                                            export: view_id_clone.clone(),
                                            op: "upsert",
                                            key: new_key.clone(),
                                            data: transformed_data,
                                            append: vec![],
                                        };
                                        
                                        if let Ok(json) = serde_json::to_vec(&frame) {
                                            let payload = Arc::new(Bytes::from(json));
                                            if client_mgr.send_to_client(client_id, payload).is_err() {
                                                return;
                                            }
                                            if let Some(ref m) = metrics_clone {
                                                m.record_ws_message_sent();
                                            }
                                        }
                                    }
                                } else {
                                    for key in current_window_keys.difference(&new_keys) {
                                        let delete_frame = Frame {
                                            seq: None,
                                            mode: frame_mode,
                                            export: view_id_clone.clone(),
                                            op: "delete",
                                            key: key.clone(),
                                            data: serde_json::Value::Null,
                                            append: vec![],
                                        };
                                        if let Ok(json) = serde_json::to_vec(&delete_frame) {
                                            let payload = Arc::new(Bytes::from(json));
                                            if client_mgr.send_to_client(client_id, payload).is_err() {
                                                return;
                                            }
                                            if let Some(ref m) = metrics_clone {
                                                m.record_ws_message_sent();
                                            }
                                        }
                                    }

                                    for (key, data) in &new_window {
                                        let mut transformed_data = data.clone();
                                        transform_large_u64_to_strings(&mut transformed_data);
                                        let frame = Frame {
                                            seq: None,
                                            mode: frame_mode,
                                            export: view_id_clone.clone(),
                                            op: "upsert",
                                            key: key.clone(),
                                            data: transformed_data,
                                            append: vec![],
                                        };
                                        if let Ok(json) = serde_json::to_vec(&frame) {
                                            let payload = Arc::new(Bytes::from(json));
                                            if client_mgr.send_to_client(client_id, payload).is_err() {
                                                return;
                                            }
                                            if let Some(ref m) = metrics_clone {
                                                m.record_ws_message_sent();
                                            }
                                        }
                                    }
                                }

                                current_window_keys = new_keys;
                            }
                            Err(_) => break,
                        }
                    }
                }
            }
        }
        .instrument(info_span!("ws.subscribe.derived", %client_id, view = %view_id_span)),
    );

    info!(
        "Client {} subscribed to derived view {} (take={}, skip={})",
        ctx.client_id, view_id, take, skip
    );
}

#[cfg(not(feature = "otel"))]
async fn attach_client_to_bus(
    ctx: &SubscriptionContext<'_>,
    subscription: Subscription,
    cancel_token: CancellationToken,
) {
    let view_id = &subscription.view;

    let view_spec = match ctx.view_index.get_view(view_id) {
        Some(spec) => spec.clone(),
        None => {
            warn!("Unknown view ID: {}", view_id);
            return;
        }
    };

    if let Err(e) = send_subscribed_frame(ctx.client_id, view_id, &view_spec, ctx.client_manager) {
        warn!("Failed to send subscribed frame: {}", e);
        return;
    }

    let is_derived_with_sort = view_spec.is_derived()
        && view_spec
            .pipeline
            .as_ref()
            .map(|p| p.sort.is_some())
            .unwrap_or(false);

    if is_derived_with_sort {
        attach_derived_view_subscription(ctx, subscription, view_spec, cancel_token).await;
        return;
    }

    match view_spec.mode {
        Mode::State => {
            let key = subscription.key.as_deref().unwrap_or("");

            let mut rx = ctx.bus_manager.get_or_create_state_bus(view_id, key).await;

            // Check if we should send snapshot (defaults to true for backward compatibility)
            let should_send_snapshot = subscription.with_snapshot.unwrap_or(true);

            if should_send_snapshot {
                if let Some(mut cached_entity) = ctx.entity_cache.get(view_id, key).await {
                    transform_large_u64_to_strings(&mut cached_entity);
                    let snapshot_entities = vec![SnapshotEntity {
                        key: key.to_string(),
                        data: cached_entity,
                    }];
                    let batch_config = ctx.entity_cache.snapshot_config();
                    let _ = send_snapshot_batches(
                        ctx.client_id,
                        &snapshot_entities,
                        view_spec.mode,
                        view_id,
                        ctx.client_manager,
                        &batch_config,
                    )
                    .await;
                    rx.borrow_and_update();
                } else if !rx.borrow().is_empty() {
                    let data = rx.borrow_and_update().clone();
                    let _ = ctx.client_manager.send_to_client(ctx.client_id, data);
                }
            } else {
                info!("Client {} subscribed to {} without snapshot", ctx.client_id, view_id);
                rx.borrow_and_update();
            }

            let client_id = ctx.client_id;
            let client_mgr = ctx.client_manager.clone();
            let view_id_clone = view_id.clone();
            let key_clone = key.to_string();
            tokio::spawn(
                async move {
                    loop {
                        tokio::select! {
                            _ = cancel_token.cancelled() => {
                                debug!("State subscription cancelled for client {}", client_id);
                                break;
                            }
                            result = rx.changed() => {
                                if result.is_err() {
                                    break;
                                }
                                let data = rx.borrow().clone();
                                if client_mgr.send_to_client(client_id, data).is_err() {
                                    break;
                                }
                            }
                        }
                    }
                }
                .instrument(info_span!("ws.subscribe.state", %client_id, view = %view_id_clone, key = %key_clone)),
            );
        }
        Mode::List | Mode::Append => {
            let mut rx = ctx.bus_manager.get_or_create_list_bus(view_id).await;

            // Check if we should send snapshot (defaults to true for backward compatibility)
            let should_send_snapshot = subscription.with_snapshot.unwrap_or(true);

            if should_send_snapshot {
                // Determine which entities to send based on cursor
                let mut snapshots = if let Some(ref cursor) = subscription.after {
                    ctx.entity_cache.get_after(view_id, cursor, subscription.snapshot_limit).await
                } else {
                    ctx.entity_cache.get_all(view_id).await
                };

                // Sort by _seq descending only when there is no cursor (to get most-recent N from full cache)
                if let Some(limit) = subscription.snapshot_limit {
                    if subscription.after.is_none() {
                        snapshots.sort_by(|a, b| {
                            let sa = a.1.get("_seq").and_then(|s| s.as_str()).unwrap_or("");
                            let sb = b.1.get("_seq").and_then(|s| s.as_str()).unwrap_or("");
                            cmp_seq(sb, sa) // descending: most-recent N
                        });
                        snapshots.truncate(limit);
                    }
                }

                let snapshot_entities: Vec<SnapshotEntity> = snapshots
                    .into_iter()
                    .filter(|(key, _)| subscription.matches_key(key))
                    .map(|(key, mut data)| {
                        transform_large_u64_to_strings(&mut data);
                        SnapshotEntity { key, data }
                    })
                    .collect();

                if !snapshot_entities.is_empty() {
                    let batch_config = ctx.entity_cache.snapshot_config();
                    if send_snapshot_batches(
                        ctx.client_id,
                        &snapshot_entities,
                        view_spec.mode,
                        view_id,
                        ctx.client_manager,
                        &batch_config,
                    )
                    .await
                    .is_err()
                    {
                        return;
                    }
                }
            } else {
                info!("Client {} subscribed to {} without snapshot", ctx.client_id, view_id);
            }

            let client_id = ctx.client_id;
            let client_mgr = ctx.client_manager.clone();
            let sub = subscription.clone();
            let view_id_clone = view_id.clone();
            let mode = view_spec.mode;
            tokio::spawn(
                async move {
                    loop {
                        tokio::select! {
                            _ = cancel_token.cancelled() => {
                                debug!("List subscription cancelled for client {}", client_id);
                                break;
                            }
                            result = rx.recv() => {
                                match result {
                                    Ok(envelope) => {
                                        if sub.matches(&envelope.entity, &envelope.key)
                                            && client_mgr
                                                .send_to_client(client_id, envelope.payload.clone())
                                                .is_err()
                                        {
                                            break;
                                        }
                                    }
                                    Err(_) => break,
                                }
                            }
                        }
                    }
                }
                .instrument(info_span!("ws.subscribe.list", %client_id, view = %view_id_clone, mode = ?mode)),
            );
        }
    }

    info!(
        "Client {} subscribed to {} (mode: {:?})",
        ctx.client_id, view_id, view_spec.mode
    );
}

#[cfg(not(feature = "otel"))]
async fn attach_derived_view_subscription(
    ctx: &SubscriptionContext<'_>,
    subscription: Subscription,
    view_spec: ViewSpec,
    cancel_token: CancellationToken,
) {
    let view_id = &subscription.view;
    let pipeline_limit = view_spec
        .pipeline
        .as_ref()
        .and_then(|p| p.limit)
        .unwrap_or(100);
    let take = subscription.take.unwrap_or(pipeline_limit);
    let skip = subscription.skip.unwrap_or(0);
    let is_single = take == 1;

    let source_view_id = match &view_spec.source_view {
        Some(s) => s.clone(),
        None => {
            warn!("Derived view {} has no source_view", view_id);
            return;
        }
    };

    let sorted_caches = ctx.view_index.sorted_caches();
    let initial_window: Vec<(String, serde_json::Value)> = {
        let mut caches = sorted_caches.write().await;
        if let Some(cache) = caches.get_mut(view_id) {
            cache.get_window(skip, take)
        } else {
            warn!("No sorted cache for derived view {}", view_id);
            vec![]
        }
    };

    let initial_keys: HashSet<String> = initial_window.iter().map(|(k, _)| k.clone()).collect();

    if !initial_window.is_empty() {
        let snapshot_entities: Vec<SnapshotEntity> = initial_window
            .into_iter()
            .map(|(key, mut data)| {
                transform_large_u64_to_strings(&mut data);
                SnapshotEntity { key, data }
            })
            .collect();

        let batch_config = ctx.entity_cache.snapshot_config();
        if send_snapshot_batches(
            ctx.client_id,
            &snapshot_entities,
            view_spec.mode,
            view_id,
            ctx.client_manager,
            &batch_config,
        )
        .await
        .is_err()
        {
            return;
        }
    }

    let mut rx = ctx
        .bus_manager
        .get_or_create_list_bus(&source_view_id)
        .await;

    let client_id = ctx.client_id;
    let client_mgr = ctx.client_manager.clone();
    let view_id_clone = view_id.clone();
    let view_id_span = view_id.clone();
    let sorted_caches_clone = sorted_caches;
    let frame_mode = view_spec.mode;

    tokio::spawn(
        async move {
            let mut current_window_keys = initial_keys;

            loop {
                tokio::select! {
                    _ = cancel_token.cancelled() => {
                        debug!("Derived view subscription cancelled for client {}", client_id);
                        break;
                    }
                    result = rx.recv() => {
                        match result {
                            Ok(_envelope) => {
                                let new_window: Vec<(String, serde_json::Value)> = {
                                    let mut caches = sorted_caches_clone.write().await;
                                    if let Some(cache) = caches.get_mut(&view_id_clone) {
                                        cache.get_window(skip, take)
                                    } else {
                                        continue;
                                    }
                                };

                                let new_keys: HashSet<String> =
                                    new_window.iter().map(|(k, _)| k.clone()).collect();

                                if is_single {
                                    if let Some((new_key, data)) = new_window.first() {
                                        for old_key in current_window_keys.difference(&new_keys) {
                                            let delete_frame = Frame {
                                            seq: None,
                                                mode: frame_mode,
                                                export: view_id_clone.clone(),
                                                op: "delete",
                                                key: old_key.clone(),
                                                data: serde_json::Value::Null,
                                                append: vec![],
                                            };
                                            if let Ok(json) = serde_json::to_vec(&delete_frame) {
                                                let payload = Arc::new(Bytes::from(json));
                                                if client_mgr.send_to_client(client_id, payload).is_err() {
                                                    return;
                                                }
                                            }
                                        }

                                        let mut transformed_data = data.clone();
                                        transform_large_u64_to_strings(&mut transformed_data);
                                        let frame = Frame {
                                            seq: None,
                                            mode: frame_mode,
                                            export: view_id_clone.clone(),
                                            op: "upsert",
                                            key: new_key.clone(),
                                            data: transformed_data,
                                            append: vec![],
                                        };
                                        if let Ok(json) = serde_json::to_vec(&frame) {
                                            let payload = Arc::new(Bytes::from(json));
                                            if client_mgr.send_to_client(client_id, payload).is_err() {
                                                return;
                                            }
                                        }
                                    }
                                } else {
                                    for key in current_window_keys.difference(&new_keys) {
                                        let delete_frame = Frame {
                                            seq: None,
                                            mode: frame_mode,
                                            export: view_id_clone.clone(),
                                            op: "delete",
                                            key: key.clone(),
                                            data: serde_json::Value::Null,
                                            append: vec![],
                                        };
                                        if let Ok(json) = serde_json::to_vec(&delete_frame) {
                                            let payload = Arc::new(Bytes::from(json));
                                            if client_mgr.send_to_client(client_id, payload).is_err() {
                                                return;
                                            }
                                        }
                                    }

                                    for (key, data) in &new_window {
                                        let mut transformed_data = data.clone();
                                        transform_large_u64_to_strings(&mut transformed_data);
                                        let frame = Frame {
                                            seq: None,
                                            mode: frame_mode,
                                            export: view_id_clone.clone(),
                                            op: "upsert",
                                            key: key.clone(),
                                            data: transformed_data,
                                            append: vec![],
                                        };
                                        if let Ok(json) = serde_json::to_vec(&frame) {
                                            let payload = Arc::new(Bytes::from(json));
                                            if client_mgr.send_to_client(client_id, payload).is_err() {
                                                return;
                                            }
                                        }
                                    }
                                }

                                current_window_keys = new_keys;
                            }
                            Err(_) => break,
                        }
                    }
                }
            }
        }
        .instrument(info_span!("ws.subscribe.derived", %client_id, view = %view_id_span)),
    );

    info!(
        "Client {} subscribed to derived view {} (take={}, skip={})",
        ctx.client_id, view_id, take, skip
    );
}
