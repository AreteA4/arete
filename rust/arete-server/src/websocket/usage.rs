use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::{interval, Instant, MissedTickBehavior};
use tracing::{debug, error, warn};
use uuid::Uuid;

const MAX_IN_MEMORY_RETRIES: u32 = 3;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WebSocketUsageEvent {
    ConnectionEstablished {
        client_id: String,
        remote_addr: String,
        deployment_id: Option<String>,
        metering_key: Option<String>,
        subject: Option<String>,
        key_class: Option<String>,
    },
    ConnectionClosed {
        client_id: String,
        deployment_id: Option<String>,
        metering_key: Option<String>,
        subject: Option<String>,
        duration_secs: Option<f64>,
        subscription_count: u32,
    },
    SubscriptionCreated {
        client_id: String,
        deployment_id: Option<String>,
        metering_key: Option<String>,
        subject: Option<String>,
        view_id: String,
    },
    SubscriptionRemoved {
        client_id: String,
        deployment_id: Option<String>,
        metering_key: Option<String>,
        subject: Option<String>,
        view_id: String,
    },
    SnapshotSent {
        client_id: String,
        deployment_id: Option<String>,
        metering_key: Option<String>,
        subject: Option<String>,
        view_id: String,
        rows: u32,
        messages: u32,
        bytes: u64,
    },
    UpdateSent {
        client_id: String,
        deployment_id: Option<String>,
        metering_key: Option<String>,
        subject: Option<String>,
        view_id: String,
        messages: u32,
        bytes: u64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSocketUsageEnvelope {
    pub event_id: String,
    pub occurred_at_ms: u64,
    pub event: WebSocketUsageEvent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSocketUsageBatch {
    pub events: Vec<WebSocketUsageEnvelope>,
}

#[async_trait]
pub trait WebSocketUsageEmitter: Send + Sync {
    async fn emit(&self, event: WebSocketUsageEvent);
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
    sender: mpsc::UnboundedSender<WebSocketUsageEvent>,
}

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
        )
    }

    pub fn with_config(
        endpoint: String,
        auth_token: Option<String>,
        batch_size: usize,
        flush_interval: Duration,
    ) -> Self {
        Self::with_full_config(endpoint, auth_token, batch_size, flush_interval, None)
    }

    fn with_full_config(
        endpoint: String,
        auth_token: Option<String>,
        batch_size: usize,
        flush_interval: Duration,
        spool_dir: Option<PathBuf>,
    ) -> Self {
        let (sender, mut receiver) = mpsc::unbounded_channel::<WebSocketUsageEvent>();
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
                    maybe_event = receiver.recv() => {
                        match maybe_event {
                            Some(event) => {
                                pending.push(WebSocketUsageEnvelope {
                                    event_id: Uuid::new_v4().to_string(),
                                    occurred_at_ms: current_time_ms(),
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
                                    ).await;
                                }
                            }
                            None => {
                                if retry_state.is_none() && !pending.is_empty() {
                                    flush_pending_batch(
                                        &client,
                                        &endpoint,
                                        auth_token.as_deref(),
                                        &mut pending,
                                        &mut retry_state,
                                        spool_dir.as_deref(),
                                    ).await;
                                }

                                if let Some(state) = retry_state.take() {
                                    if let Err(retry_state_failed) = flush_existing_batch(
                                        &client,
                                        &endpoint,
                                        auth_token.as_deref(),
                                        state,
                                    ).await {
                                        if let Some(dir) = spool_dir.as_deref() {
                                            if let Err(error) = spool_retry_state(dir, &retry_state_failed) {
                                                warn!(error = %error, count = retry_state_failed.batch.events.len(), "failed to spool websocket usage batch during shutdown");
                                            }
                                        } else {
                                            warn!(
                                                count = retry_state_failed.batch.events.len(),
                                                attempts = retry_state_failed.attempts,
                                                "dropping websocket usage batch during shutdown after failed retry"
                                            );
                                        }
                                    }
                                }

                                if !pending.is_empty() {
                                    if let Some(dir) = spool_dir.as_deref() {
                                        let batch = WebSocketUsageBatch { events: std::mem::take(&mut pending) };
                                        if let Err(error) = spool_batch(dir, &batch) {
                                            warn!(error = %error, count = batch.events.len(), "failed to spool pending websocket usage batch during shutdown");
                                        }
                                    } else {
                                        warn!(count = pending.len(), "dropping pending websocket usage events during shutdown without spool directory");
                                    }
                                }
                                break;
                            }
                        }
                    }
                    _ = ticker.tick() => {
                        if let Some(dir) = spool_dir.as_deref() {
                            if retry_state.is_none() {
                                if let Err(error) = flush_one_spooled_batch(&client, &endpoint, auth_token.as_deref(), dir).await {
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
                            ).await;
                        }
                    }
                }
            }
        });

        Self { sender }
    }
}

#[async_trait]
impl WebSocketUsageEmitter for HttpUsageEmitter {
    async fn emit(&self, event: WebSocketUsageEvent) {
        if let Err(error) = self.sender.send(event) {
            warn!(error = %error, "failed to queue websocket usage event");
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
) {
    let batch = WebSocketUsageBatch {
        events: std::mem::take(pending),
    };

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
) -> std::io::Result<()> {
    let Some(path) = oldest_spooled_batch(spool_dir)? else {
        return Ok(());
    };

    let batch = load_batch_from_file(&path)?;
    if flush_batch(client, endpoint, auth_token, &batch).await {
        std::fs::remove_file(path)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

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
                metering_key: Some("meter-1".to_string()),
                subject: Some("subject-1".to_string()),
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
                event: WebSocketUsageEvent::UpdateSent {
                    client_id: "client-1".to_string(),
                    deployment_id: Some("1".to_string()),
                    metering_key: Some("api_key:1".to_string()),
                    subject: Some("user:1".to_string()),
                    view_id: "OreRound/latest".to_string(),
                    messages: 1,
                    bytes: 42,
                },
            }],
        };

        let path = spool_batch(&dir, &batch).expect("batch should spool");
        let loaded = load_batch_from_file(&path).expect("batch should load");
        assert_eq!(loaded.events.len(), 1);

        fs::remove_dir_all(dir).expect("temp dir should be removed");
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
