//! Live instance handles: an instance's detail kept current from its event
//! stream, plus the protocol's live stack view row when one is wired.
//!
//! Mirror of `client.instances.use(id)` in `@usearete/sdk`.

use std::fmt;
use std::pin::Pin;
use std::sync::Arc;

use futures_util::future::FutureExt as _;
use futures_util::{Stream, StreamExt as _};
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::sync::{mpsc, oneshot, watch};

use super::client::{ProtocolClient, ProtocolError, ProtocolEventStream, ProtocolEventsOptions};
use super::types::{ProtocolInstanceDetail, ProtocolLiveStackDescriptor, ProtocolStreamEvent};
use crate::client::Arete;
use crate::entity::Stack;
use crate::view::StateView;

/// A stream of a live view row's values.
pub type ProtocolLiveStream = Pin<Box<dyn Stream<Item = Value> + Send>>;

/// Where a [`ProtocolClient`] finds the stack views its instance handles
/// follow. [`ProtocolStackViews`] and [`crate::Session::protocol_client`]
/// provide one; implement it to supply rows from elsewhere.
pub trait ProtocolLiveViews: Send + Sync {
    /// Follow the row keyed `key` of the view `view` (`<Entity>/<mode>`) of
    /// the stack registered as `stack`; `None` when there is no such stack.
    fn listen(&self, stack: &str, view: &str, key: &str) -> Option<ProtocolLiveStream>;
}

type StackListener = Arc<dyn Fn(&str, &str) -> ProtocolLiveStream + Send + Sync>;

/// Connected stack clients keyed like the protocol's hoisted stacks
/// (`ProtocolLiveStackDescriptor::stack`).
#[derive(Clone, Default)]
pub struct ProtocolStackViews {
    stacks: Vec<(String, StackListener)>,
}

impl ProtocolStackViews {
    pub fn new() -> Self {
        Self::default()
    }

    /// Serve the views of `client` under `key`.
    pub fn with_stack<S: Stack>(self, key: impl Into<String>, client: &Arete<S>) -> Self {
        let views = client.state_view_source();
        self.with_listener(key, move |view, row| Box::pin(views(view).listen(row)))
    }

    /// Serve `key` from a function of `(view, row key)`.
    pub fn with_listener(
        mut self,
        key: impl Into<String>,
        listen: impl Fn(&str, &str) -> ProtocolLiveStream + Send + Sync + 'static,
    ) -> Self {
        let key = key.into();
        self.stacks.retain(|(existing, _)| *existing != key);
        self.stacks.push((key, Arc::new(listen)));
        self
    }

    /// The stack keys served.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.stacks.iter().map(|(key, _)| key.as_str())
    }
}

impl fmt::Debug for ProtocolStackViews {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProtocolStackViews")
            .field("stacks", &self.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl ProtocolLiveViews for ProtocolStackViews {
    fn listen(&self, stack: &str, view: &str, key: &str) -> Option<ProtocolLiveStream> {
        self.stacks
            .iter()
            .find(|(candidate, _)| candidate == stack)
            .map(|(_, listen)| listen(view, key))
    }
}

/// Builds untyped state views of one connected stack client.
pub(crate) type StateViewSource = Arc<dyn Fn(&str) -> StateView<Value> + Send + Sync>;

/// One reading of an instance handle.
#[derive(Debug, Clone, PartialEq)]
pub struct ProtocolInstanceSnapshot {
    /// The instance as last read from the hosted API.
    pub instance: ProtocolInstanceDetail,
    /// The live stack view row, once the view has sent it. Always `None`
    /// when the definition has no live stack or the client no live views.
    pub live: Option<Value>,
}

impl ProtocolInstanceSnapshot {
    /// The live row as `T`, e.g. the generated entity of the live stack.
    pub fn live_as<T: DeserializeOwned>(&self) -> Option<Result<T, serde_json::Error>> {
        self.live
            .as_ref()
            .map(|live| serde_json::from_value(live.clone()))
    }
}

/// What an instance handle holds: the latest snapshot and the latest error.
#[derive(Debug, Clone)]
pub struct ProtocolInstanceState {
    pub snapshot: ProtocolInstanceSnapshot,
    /// The last refresh or stream failure; cleared by the next successful
    /// read.
    pub error: Option<Arc<ProtocolError>>,
}

/// Options of [`ProtocolClient::instance_handle_with`].
#[derive(Debug, Clone, Default)]
pub struct ProtocolInstanceHandleOptions {
    /// Evaluate `allowed_steps` for this wallet on every read.
    pub wallet: Option<String>,
    /// Options of the underlying event stream.
    pub events: ProtocolEventsOptions,
}

impl ProtocolInstanceHandleOptions {
    pub fn with_wallet(mut self, wallet: impl Into<String>) -> Self {
        self.wallet = Some(wallet.into());
        self
    }

    pub fn with_events(mut self, events: ProtocolEventsOptions) -> Self {
        self.events = events;
        self
    }
}

type RefreshResult = Result<ProtocolInstanceSnapshot, Arc<ProtocolError>>;
type RefreshReply = oneshot::Sender<RefreshResult>;

/// A live instance from [`ProtocolClient::instance_handle`].
///
/// Holds the instance detail and re-reads it after every event of the
/// instance's stream (events arriving together cause one read). When the
/// protocol definition names a live stack and the client has live views
/// ([`super::ProtocolClientOptions::with_live_views`], or a client from
/// [`crate::Session::protocol_client`]), the row of that view keyed by the
/// instance's key account is followed too, as
/// [`ProtocolInstanceSnapshot::live`]. Dropping or [closing](Self::close)
/// the handle stops both.
pub struct ProtocolInstanceHandle {
    instance_id: String,
    state: watch::Receiver<ProtocolInstanceState>,
    refresh: mpsc::Sender<RefreshReply>,
    task: tokio::task::JoinHandle<()>,
}

impl fmt::Debug for ProtocolInstanceHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProtocolInstanceHandle")
            .field("instance_id", &self.instance_id)
            .field("closed", &self.is_closed())
            .finish_non_exhaustive()
    }
}

impl ProtocolInstanceHandle {
    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }

    /// The latest snapshot.
    pub fn current(&self) -> ProtocolInstanceSnapshot {
        self.state.borrow().snapshot.clone()
    }

    /// The latest failure, if the last read or the stream failed.
    pub fn error(&self) -> Option<Arc<ProtocolError>> {
        self.state.borrow().error.clone()
    }

    /// A receiver of every state change.
    pub fn subscribe(&self) -> watch::Receiver<ProtocolInstanceState> {
        self.state.clone()
    }

    /// Every state change from now on (only the latest when several pile up).
    /// Ends when the handle closes.
    pub fn updates(&self) -> impl Stream<Item = ProtocolInstanceState> + Send + 'static {
        tokio_stream::wrappers::WatchStream::from_changes(self.state.clone())
    }

    /// Re-read the instance now. A failure is also the handle's
    /// [`error`](Self::error) until the next successful read.
    pub async fn refresh(&self) -> RefreshResult {
        let (reply, response) = oneshot::channel();
        let closed = || {
            Arc::new(ProtocolError::InvalidInput(
                "the instance handle is closed".into(),
            ))
        };
        self.refresh.send(reply).await.map_err(|_| closed())?;
        response.await.map_err(|_| closed())?
    }

    pub fn is_closed(&self) -> bool {
        self.task.is_finished()
    }

    /// Stop following the instance and its live row.
    pub fn close(&mut self) {
        self.task.abort();
    }
}

impl Drop for ProtocolInstanceHandle {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl ProtocolClient {
    /// Load instance `id` and keep it current; see [`ProtocolInstanceHandle`].
    /// Fails when the first read fails. Needs a Tokio runtime.
    pub async fn instance_handle(&self, id: &str) -> Result<ProtocolInstanceHandle, ProtocolError> {
        self.instance_handle_with(id, ProtocolInstanceHandleOptions::default())
            .await
    }

    /// [`ProtocolClient::instance_handle`] with a wallet and stream options.
    pub async fn instance_handle_with(
        &self,
        id: &str,
        options: ProtocolInstanceHandleOptions,
    ) -> Result<ProtocolInstanceHandle, ProtocolError> {
        let instance = self.instance(id, options.wallet.as_deref()).await?;
        let (sender, state) = watch::channel(ProtocolInstanceState {
            snapshot: ProtocolInstanceSnapshot {
                instance,
                live: None,
            },
            error: None,
        });
        let (refresh, commands) = mpsc::channel(8);
        let events = self.events(id, options.events.clone());
        let live = self
            .definition()
            .live_stack
            .and_then(|descriptor| Some((descriptor, self.live_views()?)));
        let runner = Runner {
            client: self.clone(),
            id: id.to_string(),
            wallet: options.wallet,
            sender,
            live,
            live_key: None,
            live_rows: None,
        };
        let task = tokio::spawn(runner.run(events, commands));
        Ok(ProtocolInstanceHandle {
            instance_id: id.to_string(),
            state,
            refresh,
            task,
        })
    }
}

struct Runner {
    client: ProtocolClient,
    id: String,
    wallet: Option<String>,
    sender: watch::Sender<ProtocolInstanceState>,
    live: Option<(ProtocolLiveStackDescriptor, Arc<dyn ProtocolLiveViews>)>,
    live_key: Option<String>,
    live_rows: Option<ProtocolLiveStream>,
}

type EventItem = Result<ProtocolStreamEvent, ProtocolError>;

async fn next_event(events: &mut Option<ProtocolEventStream>) -> Option<EventItem> {
    match events {
        Some(events) => events.next().await,
        None => std::future::pending().await,
    }
}

async fn next_row(rows: &mut Option<ProtocolLiveStream>) -> Option<Value> {
    match rows {
        Some(rows) => rows.next().await,
        None => std::future::pending().await,
    }
}

impl Runner {
    async fn run(
        mut self,
        events: ProtocolEventStream,
        mut commands: mpsc::Receiver<RefreshReply>,
    ) {
        let mut events = Some(events);
        let first = self.sender.borrow().snapshot.instance.clone();
        self.follow_live(&first);
        loop {
            tokio::select! {
                item = next_event(&mut events) => match item {
                    Some(item) => {
                        let mut refresh = self.note_event(item);
                        // Events that arrived together cost one read.
                        while let Some(Some(item)) = events
                            .as_mut()
                            .map(|events| events.next().now_or_never())
                            .unwrap_or(None)
                        {
                            refresh |= self.note_event(item);
                        }
                        if refresh {
                            let _ = self.reload().await;
                        }
                    }
                    // The hosted API refused the stream for good; the last
                    // error says why.
                    None => events = None,
                },
                command = commands.recv() => match command {
                    Some(reply) => {
                        let _ = reply.send(self.reload().await);
                    }
                    None => return,
                },
                row = next_row(&mut self.live_rows) => match row {
                    Some(row) => self.sender.send_modify(|state| state.snapshot.live = Some(row)),
                    None => self.live_rows = None,
                },
            }
        }
    }

    /// Record a stream error; `true` when the item is an event.
    fn note_event(&self, item: EventItem) -> bool {
        match item {
            Ok(_) => true,
            Err(error) => {
                self.sender
                    .send_modify(|state| state.error = Some(Arc::new(error)));
                false
            }
        }
    }

    async fn reload(&mut self) -> RefreshResult {
        match self.client.instance(&self.id, self.wallet.as_deref()).await {
            Ok(instance) => {
                self.follow_live(&instance);
                self.sender.send_modify(|state| {
                    state.snapshot.instance = instance;
                    state.error = None;
                });
                Ok(self.sender.borrow().snapshot.clone())
            }
            Err(error) => {
                let error = Arc::new(error);
                self.sender
                    .send_modify(|state| state.error = Some(error.clone()));
                Err(error)
            }
        }
    }

    /// Follow the live row keyed by the instance's key account, switching
    /// rows when that account changes.
    fn follow_live(&mut self, instance: &ProtocolInstanceDetail) {
        let Some((descriptor, views)) = &self.live else {
            return;
        };
        let Some(key) = instance.accounts.get(descriptor.key_account) else {
            return;
        };
        if self.live_key.as_deref() == Some(key.as_str()) {
            return;
        }
        self.live_key = Some(key.clone());
        self.live_rows = views.listen(descriptor.stack, descriptor.view, key);
        if self.live_rows.is_none() {
            tracing::debug!(
                stack = descriptor.stack,
                "protocol live stack is not among the client's live views"
            );
        }
    }
}
