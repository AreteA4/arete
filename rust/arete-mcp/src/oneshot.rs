//! One-shot view reads: connect, subscribe, wait for the snapshot, read the
//! entities, disconnect.
//!
//! This backs the `read_view` tool, for an agent that wants the current
//! value of a view ("the current ORE round") without managing a connection
//! and a subscription across several calls. Nothing it opens outlives the
//! call.

use std::time::Duration;

use arete_sdk::{
    AreteError, AuthConfig, ConnectionConfig, ConnectionManager, SharedStore, SocketIssue,
    Subscription, SubscriptionQuery,
};
use serde_json::Value;
use tokio::sync::broadcast;
use tokio::time::Instant;

/// Why a one-shot read failed.
#[derive(Debug)]
pub enum ReadError {
    /// Opening the connection or subscribing failed, or the server refused
    /// the subscription (an [`AreteError::SocketIssue`]).
    Sdk(AreteError),
    /// The connection or the snapshot did not arrive in time.
    TimedOut { waiting_for: &'static str },
}

/// Read the entities of `view` (at `key`, when given; at most `take`, when
/// given) on the stack at `url`, in the view's order, within `timeout`.
pub async fn read_view(
    url: String,
    api_key: Option<String>,
    view: &str,
    key: Option<String>,
    take: Option<usize>,
    timeout: Duration,
) -> Result<Vec<(String, Value)>, ReadError> {
    let deadline = Instant::now() + timeout;
    let mut config = ConnectionConfig {
        // A one-shot read reports a dropped connection instead of retrying
        // past its deadline.
        auto_reconnect: false,
        ..ConnectionConfig::default()
    };
    if let Some(key) = api_key {
        config.auth = Some(AuthConfig::default().with_api_key(key));
    }
    let store = SharedStore::new();
    let manager =
        tokio::time::timeout_at(deadline, ConnectionManager::new(url, config, store.clone()))
            .await
            .map_err(|_| ReadError::TimedOut {
                waiting_for: "the connection",
            })?
            .map_err(ReadError::Sdk)?;

    let result = read_on(&manager, &store, view, key, take, deadline).await;
    manager.disconnect().await;
    result
}

async fn read_on(
    manager: &ConnectionManager,
    store: &SharedStore,
    view: &str,
    key: Option<String>,
    take: Option<usize>,
    deadline: Instant,
) -> Result<Vec<(String, Value)>, ReadError> {
    let mut issues = manager.subscribe_socket_issues();
    let mut query = SubscriptionQuery::new(view);
    query.key = key;
    if let Some(take) = take {
        query = query.with_take(take);
    }
    let id = format!("read-{}", uuid::Uuid::new_v4().simple());
    let lease = manager
        .subscribe(Subscription::new(&id, query))
        .await
        .map_err(ReadError::Sdk)?;
    let wire_id = lease.subscription_id().to_string();

    let remaining = deadline.saturating_duration_since(Instant::now());
    tokio::select! {
        ready = store.wait_for_subscription_ready(&wire_id, remaining) => {
            if !ready {
                return Err(ReadError::TimedOut { waiting_for: "the snapshot" });
            }
        }
        issue = refusal(&mut issues, &wire_id) => return Err(ReadError::Sdk(AreteError::SocketIssue(Box::new(issue)))),
    }

    let mut entities = Vec::new();
    for key in store.keys_for_subscription(&wire_id).await {
        if let Some(value) = store.get_for_subscription::<Value>(&wire_id, &key).await {
            entities.push((key, value));
        }
    }
    drop(lease);
    Ok(entities)
}

/// The first socket issue that ends the read: a fatal one, or one about
/// this subscription. Never resolves when the channel closes, so the
/// deadline decides.
async fn refusal(
    issues: &mut broadcast::Receiver<SocketIssue>,
    subscription_id: &str,
) -> SocketIssue {
    loop {
        match issues.recv().await {
            Ok(issue)
                if issue.fatal || issue.subscription_id.as_deref() == Some(subscription_id) =>
            {
                return issue;
            }
            Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => continue,
            Err(broadcast::error::RecvError::Closed) => std::future::pending().await,
        }
    }
}
