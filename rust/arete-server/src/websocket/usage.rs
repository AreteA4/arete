use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tokio::time::{interval, Instant, MissedTickBehavior};
use tracing::{debug, error, warn};
use uuid::Uuid;

const MAX_IN_MEMORY_RETRIES: u32 = 3;

/// Billing and policy identity copied from one verified session token. The
/// legacy fields remain present during the compatibility window; V2 fields
/// are emitted only as a complete signed tuple.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageIdentity {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metering_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_class: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub consumer_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_version: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WebSocketUsageEvent {
    ConnectionEstablished {
        client_id: String,
        remote_addr: String,
        deployment_id: Option<String>,
        #[serde(flatten)]
        identity: UsageIdentity,
    },
    ConnectionClosed {
        client_id: String,
        deployment_id: Option<String>,
        #[serde(flatten)]
        identity: UsageIdentity,
        duration_secs: Option<f64>,
        subscription_count: u32,
    },
    SubscriptionCreated {
        client_id: String,
        deployment_id: Option<String>,
        #[serde(flatten)]
        identity: UsageIdentity,
        view_id: String,
    },
    SubscriptionRemoved {
        client_id: String,
        deployment_id: Option<String>,
        #[serde(flatten)]
        identity: UsageIdentity,
        view_id: String,
    },
    SnapshotSent {
        client_id: String,
        deployment_id: Option<String>,
        #[serde(flatten)]
        identity: UsageIdentity,
        view_id: String,
        rows: u32,
        messages: u32,
        bytes: u64,
    },
    UpdateSent {
        client_id: String,
        deployment_id: Option<String>,
        #[serde(flatten)]
        identity: UsageIdentity,
        view_id: String,
        messages: u32,
        bytes: u64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSocketUsageEnvelope {
    pub event_id: String,
    pub occurred_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_id: Option<String>,
    pub event: WebSocketUsageEvent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSocketUsageBatch {
    pub events: Vec<WebSocketUsageEnvelope>,
}

#[async_trait]
pub trait WebSocketUsageEmitter: Send + Sync {
    async fn emit(&self, event: WebSocketUsageEvent);

    /// Finish or durably preserve every event accepted before this call.
    async fn shutdown(&self) {}
}

#[derive(Clone)]
pub struct ChannelUsageEmitter {
    sender: mpsc::UnboundedSender<WebSocketUsageEvent>,
}

impl ChannelUsageEmitter {
    pub fn new(sender: mpsc::UnboundedSender<WebSocketUsageEvent>) -> Self {
        Self { sender }
    }
}

#[async_trait]
impl WebSocketUsageEmitter for ChannelUsageEmitter {
    async fn emit(&self, event: WebSocketUsageEvent) {
        let _ = self.sender.send(event);
    }
}

pub struct HttpUsageEmitter {
    sender: mpsc::UnboundedSender<UsageEmitterCommand>,
}

enum UsageEmitterCommand {
    Event(WebSocketUsageEvent),
    Shutdown(oneshot::Sender<()>),
}

#[derive(Debug, thiserror::Error)]
#[error("WebSocket usage build ID must be a positive integer")]
pub struct InvalidUsageBuildId;

#[derive(Debug, Clone)]
struct RetryState {
    batch: WebSocketUsageBatch,
    attempts: u32,
    next_retry_at: Instant,
}

impl HttpUsageEmitter {
    pub fn new(endpoint: String, auth_token: Option<String>) -> Self {
        Self::with_config(endpoint, auth_token, 50, Duration::from_secs(2))
    }

    pub fn new_attributed(
        endpoint: String,
        auth_token: Option<String>,
        build_id: impl Into<String>,
    ) -> Result<Self, InvalidUsageBuildId> {
        let build_id = validate_build_id(build_id.into())?;
        Ok(Self::with_full_config(
            endpoint,
            auth_token,
            50,
            Duration::from_secs(2),
            None,
            Some(build_id),
        ))
    }

    pub fn with_spool_dir(
        endpoint: String,
        auth_token: Option<String>,
        spool_dir: impl Into<PathBuf>,
    ) -> Self {
        Self::with_full_config(
            endpoint,
            auth_token,
            50,
            Duration::from_secs(2),
            Some(spool_dir.into()),
            None,
        )
    }

    pub fn with_attributed_spool_dir(
        endpoint: String,
        auth_token: Option<String>,
        spool_dir: impl Into<PathBuf>,
        build_id: impl Into<String>,
    ) -> Result<Self, InvalidUsageBuildId> {
        let build_id = validate_build_id(build_id.into())?;
        Ok(Self::with_full_config(
            endpoint,
            auth_token,
            50,
            Duration::from_secs(2),
            Some(spool_dir.into()),
            Some(build_id),
        ))
    }

    pub fn with_config(
        endpoint: String,
        auth_token: Option<String>,
        batch_size: usize,
        flush_interval: Duration,
    ) -> Self {
        Self::with_full_config(endpoint, auth_token, batch_size, flush_interval, None, None)
    }

    fn with_full_config(
        endpoint: String,
        auth_token: Option<String>,
        batch_size: usize,
        flush_interval: Duration,
        spool_dir: Option<PathBuf>,
        build_id: Option<String>,
    ) -> Self {
        let (sender, mut receiver) = mpsc::unbounded_channel::<UsageEmitterCommand>();
        let client = reqwest::Client::new();

        tokio::spawn(async move {
            let mut ticker = interval(flush_interval);
            ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
            let mut pending: Vec<WebSocketUsageEnvelope> = Vec::new();
            let mut retry_state: Option<RetryState> = None;

            if let Some(dir) = spool_dir.as_ref() {
                if let Err(error) = ensure_spool_dir(dir) {
                    warn!(error = %error, path = %dir.display(), "failed to initialize websocket usage spool directory");
                }
            }

            loop {
                tokio::select! {
                    maybe_command = receiver.recv() => {
                        match maybe_command {
                            Some(UsageEmitterCommand::Event(event)) => {
                                pending.push(WebSocketUsageEnvelope {
                                    event_id: Uuid::new_v4().to_string(),
                                    occurred_at_ms: current_time_ms(),
                                    build_id: build_id.clone(),
                                    event,
                                });

                                if retry_state.is_none() && pending.len() >= batch_size {
                                    flush_pending_batch(
                                        &client,
                                        &endpoint,
                                        auth_token.as_deref(),
                                        &mut pending,
                                        &mut retry_state,
                                        spool_dir.as_deref(),
                                        batch_size,
                                    ).await;
                                }
                            }
                            Some(UsageEmitterCommand::Shutdown(done)) => {
                                flush_on_shutdown(
                                    &client,
                                    &endpoint,
                                    auth_token.as_deref(),
                                    &mut pending,
                                    &mut retry_state,
                                    spool_dir.as_deref(),
                                    batch_size,
                                ).await;
                                let _ = done.send(());
                                break;
                            }
                            None => {
                                flush_on_shutdown(
                                    &client,
                                    &endpoint,
                                    auth_token.as_deref(),
                                    &mut pending,
                                    &mut retry_state,
                                    spool_dir.as_deref(),
                                    batch_size,
                                ).await;
                                break;
                            }
                        }
                    }
                    _ = ticker.tick() => {
                        if let Some(dir) = spool_dir.as_deref() {
                            if retry_state.is_none() {
                                if let Err(error) = flush_one_spooled_batch(
                                    &client,
                                    &endpoint,
                                    auth_token.as_deref(),
                                    dir,
                                    batch_size,
                                ).await {
                                    warn!(error = %error, path = %dir.display(), "failed to process spooled websocket usage batch");
                                }
                            }
                        }

                        if let Some(state) = retry_state.take() {
                            if Instant::now() >= state.next_retry_at {
                                match flush_existing_batch(
                                    &client,
                                    &endpoint,
                                    auth_token.as_deref(),
                                    state,
                                ).await {
                                    Ok(()) => {
                                        if !pending.is_empty() {
                                            flush_pending_batch(
                                                &client,
                                                &endpoint,
                                                auth_token.as_deref(),
                                                &mut pending,
                                                &mut retry_state,
                                                spool_dir.as_deref(),
                                                batch_size,
                                            ).await;
                                        }
                                    }
                                    Err(state) => {
                                        if state.attempts >= MAX_IN_MEMORY_RETRIES {
                                            if let Some(dir) = spool_dir.as_deref() {
                                                if let Err(error) = spool_retry_state(dir, &state) {
                                                    warn!(error = %error, count = state.batch.events.len(), "failed to spool websocket usage batch after retries");
                                                    retry_state = Some(state);
                                                }
                                            } else {
                                                retry_state = Some(state);
                                            }
                                        } else {
                                            retry_state = Some(state)
                                        }
                                    }
                                }
                            } else {
                                retry_state = Some(state);
                            }
                        } else if !pending.is_empty() {
                            flush_pending_batch(
                                &client,
                                &endpoint,
                                auth_token.as_deref(),
                                &mut pending,
                                &mut retry_state,
                                spool_dir.as_deref(),
                                batch_size,
                            ).await;
                        }
                    }
                }
            }
        });

        Self { sender }
    }
}

fn validate_build_id(value: String) -> Result<String, InvalidUsageBuildId> {
    if value.trim() != value || value.parse::<i32>().ok().is_none_or(|value| value <= 0) {
        return Err(InvalidUsageBuildId);
    }
    Ok(value)
}

#[async_trait]
impl WebSocketUsageEmitter for HttpUsageEmitter {
    async fn emit(&self, event: WebSocketUsageEvent) {
        if let Err(error) = self.sender.send(UsageEmitterCommand::Event(event)) {
            warn!(error = %error, "failed to queue websocket usage event");
        }
    }

    async fn shutdown(&self) {
        let (done, completed) = oneshot::channel();
        if self
            .sender
            .send(UsageEmitterCommand::Shutdown(done))
            .is_ok()
        {
            let _ = completed.await;
        }
    }
}

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

async fn flush_batch(
    client: &reqwest::Client,
    endpoint: &str,
    auth_token: Option<&str>,
    batch: &WebSocketUsageBatch,
) -> bool {
    if batch.events.is_empty() {
        return true;
    }

    let mut request = client.post(endpoint).json(batch);
    if let Some(token) = auth_token {
        request = request.header("Authorization", format!("Bearer {}", token));
    }

    match request.send().await {
        Ok(response) if response.status().is_success() => {
            debug!(count = batch.events.len(), "flushed websocket usage batch");
            true
        }
        Ok(response) => {
            error!(status = %response.status(), count = batch.events.len(), "failed to ingest websocket usage batch");
            false
        }
        Err(error) => {
            error!(error = %error, count = batch.events.len(), "failed to post websocket usage batch");
            false
        }
    }
}

async fn flush_pending_batch(
    client: &reqwest::Client,
    endpoint: &str,
    auth_token: Option<&str>,
    pending: &mut Vec<WebSocketUsageEnvelope>,
    retry_state: &mut Option<RetryState>,
    spool_dir: Option<&Path>,
    batch_size: usize,
) {
    let batch = take_pending_batch(pending, batch_size);

    if !flush_batch(client, endpoint, auth_token, &batch).await {
        let state = RetryState {
            batch,
            attempts: 1,
            next_retry_at: Instant::now() + retry_delay(1),
        };

        if let Some(dir) = spool_dir.filter(|_| MAX_IN_MEMORY_RETRIES <= 1) {
            if let Err(error) = spool_retry_state(dir, &state) {
                warn!(error = %error, count = state.batch.events.len(), "failed to spool websocket usage batch after first failure");
                *retry_state = Some(state);
            }
        } else {
            *retry_state = Some(state);
        }
    }
}

/// Exhaust every bounded in-memory batch before the emitter exits.
///
/// A recovered retry can leave several batches queued behind it. Shutdown must
/// continue after that retry succeeds instead of sending only one more chunk
/// and dropping the rest when no spool directory is configured. On the first
/// failed shutdown retry, preserve the failed and not-yet-attempted batches on
/// disk when possible.
async fn flush_on_shutdown(
    client: &reqwest::Client,
    endpoint: &str,
    auth_token: Option<&str>,
    pending: &mut Vec<WebSocketUsageEnvelope>,
    retry_state: &mut Option<RetryState>,
    spool_dir: Option<&Path>,
    batch_size: usize,
) {
    if let Some(dir) = spool_dir {
        // The runtime explicitly awaits this shutdown path, but process-level
        // termination still has a bounded grace period. Persist first so no
        // accepted event depends on completing a sequence of HTTP requests.
        if let Some(state) = retry_state.take() {
            if let Err(error) = spool_retry_state(dir, &state) {
                warn!(error = %error, count = state.batch.events.len(), "failed to spool websocket usage retry during shutdown");
            }
        }
        while !pending.is_empty() {
            let batch = take_pending_batch(pending, batch_size);
            if let Err(error) = spool_batch(dir, &batch) {
                warn!(error = %error, count = batch.events.len(), "failed to spool pending websocket usage batch during shutdown");
            }
        }
        return;
    }

    loop {
        if let Some(state) = retry_state.take() {
            if let Err(retry_state_failed) =
                flush_existing_batch(client, endpoint, auth_token, state).await
            {
                warn!(
                    count = retry_state_failed.batch.events.len(),
                    attempts = retry_state_failed.attempts,
                    "dropping websocket usage batch during shutdown after failed retry"
                );
                break;
            }
            continue;
        }

        if pending.is_empty() {
            return;
        }

        flush_pending_batch(
            client,
            endpoint,
            auth_token,
            pending,
            retry_state,
            None,
            batch_size,
        )
        .await;
    }

    if !pending.is_empty() {
        warn!(
            count = pending.len(),
            "dropping pending websocket usage events during shutdown without spool directory"
        );
    }
}

/// Remove at most one configured HTTP batch from the pending queue.
///
/// A failed request can leave the emitter retrying while new events continue
/// to arrive. Draining the entire pending queue after recovery would turn that
/// backlog into one unbounded request and can permanently wedge delivery on a
/// `413 Payload Too Large`. Keep the wire-size invariant at every flush, not
/// only on the fast path that first reaches `batch_size`.
fn take_pending_batch(
    pending: &mut Vec<WebSocketUsageEnvelope>,
    batch_size: usize,
) -> WebSocketUsageBatch {
    let batch_size = batch_size.max(1);
    let remainder = if pending.len() > batch_size {
        pending.split_off(batch_size)
    } else {
        Vec::new()
    };
    let events = std::mem::replace(pending, remainder);
    WebSocketUsageBatch { events }
}

async fn flush_existing_batch(
    client: &reqwest::Client,
    endpoint: &str,
    auth_token: Option<&str>,
    mut state: RetryState,
) -> Result<(), RetryState> {
    if flush_batch(client, endpoint, auth_token, &state.batch).await {
        Ok(())
    } else {
        state.attempts += 1;
        state.next_retry_at = Instant::now() + retry_delay(state.attempts);
        Err(state)
    }
}

fn retry_delay(attempt: u32) -> Duration {
    let capped_attempt = attempt.min(6);
    Duration::from_secs(1_u64 << capped_attempt)
}

fn ensure_spool_dir(path: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(path)
}

fn spool_retry_state(path: &Path, state: &RetryState) -> std::io::Result<PathBuf> {
    spool_batch(path, &state.batch)
}

fn spool_batch(path: &Path, batch: &WebSocketUsageBatch) -> std::io::Result<PathBuf> {
    ensure_spool_dir(path)?;

    let file_name = format!(
        "ws-usage-{}-{}.json",
        current_time_ms(),
        Uuid::new_v4().simple()
    );
    let final_path = path.join(file_name);
    let temp_path = final_path.with_extension("tmp");
    let data = serde_json::to_vec(batch).map_err(std::io::Error::other)?;
    std::fs::write(&temp_path, data)?;
    std::fs::rename(&temp_path, &final_path)?;
    Ok(final_path)
}

fn load_batch_from_file(path: &Path) -> std::io::Result<WebSocketUsageBatch> {
    let data = std::fs::read(path)?;
    serde_json::from_slice(&data).map_err(std::io::Error::other)
}

fn oldest_spooled_batch(path: &Path) -> std::io::Result<Option<PathBuf>> {
    if !path.exists() {
        return Ok(None);
    }

    let mut entries: Vec<PathBuf> = std::fs::read_dir(path)?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|entry| entry.extension().and_then(|ext| ext.to_str()) == Some("json"))
        .collect();
    entries.sort();
    Ok(entries.into_iter().next())
}

async fn flush_one_spooled_batch(
    client: &reqwest::Client,
    endpoint: &str,
    auth_token: Option<&str>,
    spool_dir: &Path,
    batch_size: usize,
) -> std::io::Result<()> {
    let Some(path) = oldest_spooled_batch(spool_dir)? else {
        return Ok(());
    };

    let mut batch = load_batch_from_file(&path)?;
    let batch_size = batch_size.max(1);
    let remainder = if batch.events.len() > batch_size {
        Some(WebSocketUsageBatch {
            events: batch.events.split_off(batch_size),
        })
    } else {
        None
    };

    if flush_batch(client, endpoint, auth_token, &batch).await {
        if let Some(remainder) = remainder {
            // Replacing the file after the successful request can repeat the
            // first chunk after a crash, but every envelope keeps its event ID
            // and ingestion is idempotent. Rewriting before the request could
            // lose billable usage instead.
            replace_spooled_batch(&path, &remainder)?;
        } else {
            std::fs::remove_file(path)?;
        }
    }

    Ok(())
}

fn replace_spooled_batch(path: &Path, batch: &WebSocketUsageBatch) -> std::io::Result<()> {
    let temp_path = path.with_extension("tmp");
    let data = serde_json::to_vec(batch).map_err(std::io::Error::other)?;
    std::fs::write(&temp_path, data)?;
    std::fs::rename(temp_path, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn usage_envelope(index: u64) -> WebSocketUsageEnvelope {
        WebSocketUsageEnvelope {
            event_id: format!("evt-{index}"),
            occurred_at_ms: index,
            build_id: None,
            event: WebSocketUsageEvent::ConnectionEstablished {
                client_id: format!("client-{index}"),
                remote_addr: "127.0.0.1:1234".to_string(),
                deployment_id: Some("1".to_string()),
                identity: UsageIdentity::default(),
            },
        }
    }

    async fn successful_batch_server(
        request_count: usize,
    ) -> (String, tokio::task::JoinHandle<Vec<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test server should bind");
        let address = listener.local_addr().expect("test server address");
        let server = tokio::spawn(async move {
            let mut received = Vec::with_capacity(request_count);
            for _ in 0..request_count {
                let (mut stream, _) = listener.accept().await.expect("request should connect");
                let mut bytes = Vec::new();
                let body_start = loop {
                    let mut chunk = [0_u8; 4096];
                    let count = stream.read(&mut chunk).await.expect("request should read");
                    assert!(count > 0, "request ended before its headers");
                    bytes.extend_from_slice(&chunk[..count]);
                    if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                        break index + 4;
                    }
                };
                let headers = std::str::from_utf8(&bytes[..body_start])
                    .expect("request headers should be UTF-8");
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().expect("content length"))
                    })
                    .expect("request should have content length");
                while bytes.len() - body_start < content_length {
                    let mut chunk = [0_u8; 4096];
                    let count = stream.read(&mut chunk).await.expect("body should read");
                    assert!(count > 0, "request ended before its body");
                    bytes.extend_from_slice(&chunk[..count]);
                }
                let batch: WebSocketUsageBatch =
                    serde_json::from_slice(&bytes[body_start..body_start + content_length])
                        .expect("request should contain a usage batch");
                received.push(
                    batch
                        .events
                        .into_iter()
                        .map(|event| event.event_id)
                        .collect(),
                );
                stream
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                    .await
                    .expect("response should write");
            }
            received
        });
        (format!("http://{address}"), server)
    }

    fn temp_spool_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("arete-usage-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("temp dir should be created");
        dir
    }

    #[tokio::test]
    async fn channel_usage_emitter_forwards_events() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let emitter = ChannelUsageEmitter::new(tx);

        emitter
            .emit(WebSocketUsageEvent::SubscriptionCreated {
                client_id: "client-1".to_string(),
                deployment_id: Some("deployment-1".to_string()),
                identity: UsageIdentity {
                    metering_key: Some("meter-1".to_string()),
                    subject: Some("subject-1".to_string()),
                    ..Default::default()
                },
                view_id: "OreRound/latest".to_string(),
            })
            .await;

        let event = rx.recv().await.expect("event should be forwarded");
        match event {
            WebSocketUsageEvent::SubscriptionCreated { view_id, .. } => {
                assert_eq!(view_id, "OreRound/latest");
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn retry_delay_grows_and_caps() {
        assert_eq!(retry_delay(1), Duration::from_secs(2));
        assert_eq!(retry_delay(2), Duration::from_secs(4));
        assert_eq!(retry_delay(6), Duration::from_secs(64));
        assert_eq!(retry_delay(9), Duration::from_secs(64));
    }

    #[test]
    fn spooled_batches_round_trip() {
        let dir = temp_spool_dir();
        let batch = WebSocketUsageBatch {
            events: vec![WebSocketUsageEnvelope {
                event_id: "evt_1".to_string(),
                occurred_at_ms: 123,
                build_id: Some("7".to_string()),
                event: WebSocketUsageEvent::UpdateSent {
                    client_id: "client-1".to_string(),
                    deployment_id: Some("1".to_string()),
                    identity: UsageIdentity {
                        metering_key: Some("api_key:1".to_string()),
                        subject: Some("user:1".to_string()),
                        ..Default::default()
                    },
                    view_id: "OreRound/latest".to_string(),
                    messages: 1,
                    bytes: 42,
                },
            }],
        };

        let path = spool_batch(&dir, &batch).expect("batch should spool");
        let loaded = load_batch_from_file(&path).expect("batch should load");
        assert_eq!(loaded.events.len(), 1);
        assert_eq!(loaded.events[0].build_id.as_deref(), Some("7"));

        fs::remove_dir_all(dir).expect("temp dir should be removed");
    }

    #[test]
    fn pending_backlog_is_drained_in_bounded_batches() {
        let mut pending = (0..1250).map(usage_envelope).collect::<Vec<_>>();

        let first = take_pending_batch(&mut pending, 50);
        assert_eq!(first.events.len(), 50);
        assert_eq!(pending.len(), 1200);
        assert_eq!(first.events[0].event_id, "evt-0");
        assert_eq!(pending[0].event_id, "evt-50");
    }

    #[tokio::test]
    async fn oversized_legacy_spool_is_delivered_in_successive_bounded_chunks() {
        let dir = temp_spool_dir();
        let path = dir.join("ws-usage-100-a.json");
        let batch = WebSocketUsageBatch {
            events: (0..5).map(usage_envelope).collect(),
        };
        fs::write(&path, serde_json::to_vec(&batch).unwrap()).expect("legacy spool should write");
        let (endpoint, server) = successful_batch_server(3).await;
        let client = reqwest::Client::new();

        flush_one_spooled_batch(&client, &endpoint, None, &dir, 2)
            .await
            .expect("first chunk should flush");
        let loaded = load_batch_from_file(&path).expect("remainder should be readable");
        assert_eq!(loaded.events.len(), 3);
        assert_eq!(loaded.events[0].event_id, "evt-2");

        flush_one_spooled_batch(&client, &endpoint, None, &dir, 2)
            .await
            .expect("second chunk should flush");
        let loaded = load_batch_from_file(&path).expect("final remainder should be readable");
        assert_eq!(loaded.events.len(), 1);
        assert_eq!(loaded.events[0].event_id, "evt-4");

        flush_one_spooled_batch(&client, &endpoint, None, &dir, 2)
            .await
            .expect("final chunk should flush");
        assert!(!path.exists());

        let received = server.await.expect("test server should finish");
        assert_eq!(
            received,
            vec![
                vec!["evt-0".to_string(), "evt-1".to_string()],
                vec!["evt-2".to_string(), "evt-3".to_string()],
                vec!["evt-4".to_string()],
            ]
        );

        fs::remove_dir_all(dir).expect("temp dir should be removed");
    }

    #[tokio::test]
    async fn shutdown_drains_every_chunk_after_a_retry_recovers() {
        let (endpoint, server) = successful_batch_server(4).await;
        let client = reqwest::Client::new();
        let mut pending = (1..=5).map(usage_envelope).collect::<Vec<_>>();
        let mut retry_state = Some(RetryState {
            batch: WebSocketUsageBatch {
                events: vec![usage_envelope(0)],
            },
            attempts: 1,
            next_retry_at: Instant::now(),
        });

        flush_on_shutdown(
            &client,
            &endpoint,
            None,
            &mut pending,
            &mut retry_state,
            None,
            2,
        )
        .await;

        assert!(pending.is_empty());
        assert!(retry_state.is_none());
        let received = server.await.expect("test server should finish");
        assert_eq!(
            received,
            vec![
                vec!["evt-0".to_string()],
                vec!["evt-1".to_string(), "evt-2".to_string()],
                vec!["evt-3".to_string(), "evt-4".to_string()],
                vec!["evt-5".to_string()],
            ]
        );
    }

    #[tokio::test]
    async fn shutdown_persists_every_chunk_before_network_drain_when_spooling() {
        let dir = temp_spool_dir();
        let client = reqwest::Client::new();
        let mut pending = (1..=5).map(usage_envelope).collect::<Vec<_>>();
        let mut retry_state = Some(RetryState {
            batch: WebSocketUsageBatch {
                events: vec![usage_envelope(0)],
            },
            attempts: 1,
            next_retry_at: Instant::now(),
        });

        flush_on_shutdown(
            &client,
            "http://127.0.0.1:1/unreachable",
            None,
            &mut pending,
            &mut retry_state,
            Some(&dir),
            2,
        )
        .await;

        assert!(pending.is_empty());
        assert!(retry_state.is_none());
        let mut event_ids = Vec::new();
        let mut batch_sizes = Vec::new();
        for entry in fs::read_dir(&dir).expect("spool directory should be readable") {
            let batch = load_batch_from_file(&entry.unwrap().path()).expect("spool should load");
            batch_sizes.push(batch.events.len());
            event_ids.extend(batch.events.into_iter().map(|event| event.event_id));
        }
        batch_sizes.sort_unstable();
        event_ids.sort();
        assert_eq!(batch_sizes, vec![1, 1, 2, 2]);
        assert_eq!(
            event_ids,
            (0..=5)
                .map(|index| format!("evt-{index}"))
                .collect::<Vec<_>>()
        );

        fs::remove_dir_all(dir).expect("temp dir should be removed");
    }

    #[test]
    fn usage_identity_is_flattened_and_old_spool_json_remains_readable() {
        let legacy: WebSocketUsageBatch = serde_json::from_value(serde_json::json!({
            "events": [{
                "event_id": "evt-old",
                "occurred_at_ms": 123,
                "event": {
                    "type": "update_sent",
                    "client_id": "client-1",
                    "deployment_id": "1",
                    "metering_key": "api_key:1",
                    "subject": "user:1",
                    "view_id": "Round/latest",
                    "messages": 1,
                    "bytes": 42
                }
            }]
        }))
        .expect("old-format spool remains readable");
        let value = serde_json::to_value(&legacy).unwrap();
        assert_eq!(value["events"][0]["event"]["metering_key"], "api_key:1");
        assert!(value["events"][0]["event"].get("actor_key").is_none());

        let event = WebSocketUsageEvent::UpdateSent {
            client_id: "client-2".to_string(),
            deployment_id: Some("2".to_string()),
            identity: UsageIdentity {
                metering_key: Some("account:42".to_string()),
                subject: Some("user:7".to_string()),
                key_class: Some("secret".to_string()),
                actor_key: Some("user:7".to_string()),
                account_key: Some("account:42".to_string()),
                consumer_key: Some("consumer:key-9".to_string()),
                plan_code: Some("agent_trial".to_string()),
                policy_version: Some(3),
            },
            view_id: "Round/latest".to_string(),
            messages: 1,
            bytes: 42,
        };
        let value = serde_json::to_value(event).unwrap();
        assert_eq!(value["account_key"], "account:42");
        assert_eq!(value["plan_code"], "agent_trial");
        assert_eq!(value["policy_version"], 3);
    }

    #[test]
    fn legacy_and_attributed_envelopes_are_compatible() {
        let legacy = serde_json::json!({
            "event_id": "legacy",
            "occurred_at_ms": 123,
            "event": {
                "type": "update_sent",
                "client_id": "client",
                "deployment_id": "1",
                "metering_key": null,
                "subject": null,
                "view_id": "view",
                "messages": 1,
                "bytes": 2
            }
        });
        let decoded: WebSocketUsageEnvelope = serde_json::from_value(legacy).unwrap();
        assert_eq!(decoded.build_id, None);

        let mut attributed = serde_json::to_value(decoded).unwrap();
        assert!(attributed.get("build_id").is_none());
        attributed["build_id"] = serde_json::json!("42");
        let decoded: WebSocketUsageEnvelope = serde_json::from_value(attributed).unwrap();
        assert_eq!(decoded.build_id.as_deref(), Some("42"));
    }

    #[test]
    fn attributed_constructor_validates_build_identity() {
        assert!(
            HttpUsageEmitter::new_attributed("http://localhost".to_string(), None, "0").is_err()
        );
        assert!(
            HttpUsageEmitter::new_attributed("http://localhost".to_string(), None, " 7").is_err()
        );
    }

    #[test]
    fn oldest_spooled_batch_prefers_lexicographically_oldest_file() {
        let dir = temp_spool_dir();
        fs::write(dir.join("ws-usage-100-a.json"), b"{\"events\":[]}").expect("first batch");
        fs::write(dir.join("ws-usage-200-b.json"), b"{\"events\":[]}").expect("second batch");

        let oldest = oldest_spooled_batch(&dir)
            .expect("listing should succeed")
            .expect("batch should exist");
        assert!(oldest.ends_with("ws-usage-100-a.json"));

        fs::remove_dir_all(dir).expect("temp dir should be removed");
    }
}
