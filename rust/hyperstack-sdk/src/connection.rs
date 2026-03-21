use crate::config::ConnectionConfig;
use crate::frame::{parse_frame, Frame};
use crate::subscription::{ClientMessage, Subscription, SubscriptionRegistry, Unsubscription};
use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tokio::time::{sleep, Duration};
use tokio_tungstenite::{connect_async, tungstenite::Message};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
    Reconnecting { attempt: u32 },
    Error,
}

pub enum ConnectionCommand {
    Subscribe(Subscription),
    Unsubscribe(Unsubscription),
    Disconnect,
}

struct ConnectionManagerInner {
    #[allow(dead_code)]
    url: String,
    state: Arc<RwLock<ConnectionState>>,
    subscriptions: Arc<RwLock<SubscriptionRegistry>>,
    #[allow(dead_code)]
    config: ConnectionConfig,
    command_tx: mpsc::Sender<ConnectionCommand>,
}

#[derive(Clone)]
pub struct ConnectionManager {
    inner: Arc<ConnectionManagerInner>,
}

impl ConnectionManager {
    pub async fn new(url: String, config: ConnectionConfig, frame_tx: mpsc::Sender<Frame>) -> Self {
        let (command_tx, command_rx) = mpsc::channel(100);
        let state = Arc::new(RwLock::new(ConnectionState::Disconnected));
        let subscriptions = Arc::new(RwLock::new(SubscriptionRegistry::new()));

        let inner = ConnectionManagerInner {
            url: url.clone(),
            state: state.clone(),
            subscriptions: subscriptions.clone(),
            config: config.clone(),
            command_tx,
        };

        spawn_connection_loop(url, state, subscriptions, config, frame_tx, command_rx);

        Self {
            inner: Arc::new(inner),
        }
    }

    pub async fn state(&self) -> ConnectionState {
        *self.inner.state.read().await
    }

    pub async fn ensure_subscription(&self, view: &str, key: Option<&str>) {
        self.ensure_subscription_with_opts(view, key, None, None, None, None, None)
            .await
    }

    pub async fn ensure_subscription_with_opts(
        &self,
        view: &str,
        key: Option<&str>,
        take: Option<u32>,
        skip: Option<u32>,
        with_snapshot: Option<bool>,
        after: Option<&str>,
        snapshot_limit: Option<usize>,
    ) {
        let sub = Subscription {
            view: view.to_string(),
            key: key.map(|s| s.to_string()),
            partition: None,
            filters: None,
            take,
            skip,
            with_snapshot,
            after: after.map(|s| s.to_string()),
            snapshot_limit,
        };

        if !self.inner.subscriptions.read().await.contains(&sub) {
            let _ = self
                .inner
                .command_tx
                .send(ConnectionCommand::Subscribe(sub))
                .await;
        }
    }

    pub async fn subscribe(&self, sub: Subscription) {
        let _ = self
            .inner
            .command_tx
            .send(ConnectionCommand::Subscribe(sub))
            .await;
    }

    pub async fn unsubscribe(&self, unsub: Unsubscription) {
        let _ = self
            .inner
            .command_tx
            .send(ConnectionCommand::Unsubscribe(unsub))
            .await;
    }

    pub async fn disconnect(&self) {
        let _ = self
            .inner
            .command_tx
            .send(ConnectionCommand::Disconnect)
            .await;
    }
}

fn spawn_connection_loop(
    url: String,
    state: Arc<RwLock<ConnectionState>>,
    subscriptions: Arc<RwLock<SubscriptionRegistry>>,
    config: ConnectionConfig,
    frame_tx: mpsc::Sender<Frame>,
    mut command_rx: mpsc::Receiver<ConnectionCommand>,
) {
    tokio::spawn(async move {
        let mut reconnect_attempt: u32 = 0;
        let mut should_run = true;

        while should_run {
            *state.write().await = ConnectionState::Connecting;

            match connect_async(&url).await {
                Ok((ws, _)) => {
                    *state.write().await = ConnectionState::Connected;
                    reconnect_attempt = 0;

                    let (mut ws_tx, mut ws_rx) = ws.split();

                    let subs = subscriptions.read().await.all();
                    for sub in subs {
                        let client_msg = ClientMessage::Subscribe(sub);
                        if let Ok(msg) = serde_json::to_string(&client_msg) {
                            let _ = ws_tx.send(Message::Text(msg)).await;
                        }
                    }

                    let ping_interval = config.ping_interval;
                    let mut ping_timer = tokio::time::interval(ping_interval);

                    loop {
                        tokio::select! {
                            msg = ws_rx.next() => {
                                match msg {
                                    Some(Ok(Message::Binary(bytes))) => {
                                        if let Ok(frame) = parse_frame(&bytes) {
                                            let _ = frame_tx.send(frame).await;
                                        }
                                    }
                                    Some(Ok(Message::Text(text))) => {
                                        if let Ok(frame) = serde_json::from_str::<Frame>(&text) {
                                            let _ = frame_tx.send(frame).await;
                                        }
                                    }
                                    Some(Ok(Message::Ping(payload))) => {
                                        let _ = ws_tx.send(Message::Pong(payload)).await;
                                    }
                                    Some(Ok(Message::Close(_))) => {
                                        break;
                                    }
                                    Some(Err(_)) => {
                                        break;
                                    }
                                    None => {
                                        break;
                                    }
                                    _ => {}
                                }
                            }
                            cmd = command_rx.recv() => {
                                match cmd {
                                    Some(ConnectionCommand::Subscribe(sub)) => {
                                        subscriptions.write().await.add(sub.clone());
                                        let client_msg = ClientMessage::Subscribe(sub);
                                        if let Ok(msg) = serde_json::to_string(&client_msg) {
                                            let _ = ws_tx.send(Message::Text(msg)).await;
                                        }
                                    }
                                    Some(ConnectionCommand::Unsubscribe(unsub)) => {
                                        let sub = Subscription {
                                            view: unsub.view.clone(),
                                            key: unsub.key.clone(),
                                            partition: None,
                                            filters: None,
                                            take: None,
                                            skip: None,
                                            with_snapshot: None,
                                            after: None,
                                            snapshot_limit: None,
                                        };
                                        subscriptions.write().await.remove(&sub);
                                        let client_msg = ClientMessage::Unsubscribe(unsub);
                                        if let Ok(msg) = serde_json::to_string(&client_msg) {
                                            let _ = ws_tx.send(Message::Text(msg)).await;
                                        }
                                    }
                                    Some(ConnectionCommand::Disconnect) => {
                                        let _ = ws_tx.close().await;
                                        *state.write().await = ConnectionState::Disconnected;
                                        should_run = false;
                                        break;
                                    }
                                    None => {
                                        should_run = false;
                                        break;
                                    }
                                }
                            }
                            _ = ping_timer.tick() => {
                                if let Ok(msg) = serde_json::to_string(&ClientMessage::Ping) {
                                    let _ = ws_tx.send(Message::Text(msg)).await;
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("Connection failed: {}", e);
                }
            }

            if !should_run {
                break;
            }

            if !config.auto_reconnect {
                *state.write().await = ConnectionState::Error;
                break;
            }

            if reconnect_attempt >= config.max_reconnect_attempts {
                *state.write().await = ConnectionState::Error;
                break;
            }

            let delay = config
                .reconnect_intervals
                .get(reconnect_attempt as usize)
                .copied()
                .unwrap_or_else(|| {
                    config
                        .reconnect_intervals
                        .last()
                        .copied()
                        .unwrap_or(Duration::from_secs(16))
                });

            *state.write().await = ConnectionState::Reconnecting {
                attempt: reconnect_attempt,
            };
            reconnect_attempt += 1;

            tracing::info!(
                "Reconnecting in {:?} (attempt {})",
                delay,
                reconnect_attempt
            );
            sleep(delay).await;
        }
    });
}
