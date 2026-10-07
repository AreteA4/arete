//! The WebSocket session behind `a4 stream`: opening the socket, subscribing,
//! and reading until the stream ends. The plain and TUI outputs share it and
//! differ only in what they do with each message.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use anyhow::{Context, Result};
use arete_sdk::{parse_server_message, ClientMessage, ServerMessage, Subscription};
use futures_util::{Sink, SinkExt, Stream, StreamExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

use super::token;

pub type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// How often the client says it is still there.
const PING_PERIOD: Duration = Duration::from_secs(30);

/// What the stream receives, as the output side needs it.
pub trait SessionHandler {
    /// A message from the server. `Ok(true)` ends the stream.
    fn on_message(&mut self, message: ServerMessage) -> Result<bool>;

    /// Something the user should know that does not end the stream.
    fn on_notice(&mut self, notice: Notice<'_>);
}

pub enum Notice<'a> {
    /// A frame that could not be parsed.
    Unparsed {
        binary: bool,
        error: &'a dyn fmt::Display,
    },
    /// The server refused a refreshed session token, with its reason.
    RefreshRefused(Option<&'a str>),
    /// Minting the next session token failed; it is tried again shortly.
    RefreshFailed(&'a anyhow::Error),
}

/// How a stream ended.
#[derive(Debug)]
pub enum StreamEnd {
    /// The caller's stop signal fired (Ctrl+C, `--duration`, quitting the
    /// TUI) and the socket was closed.
    Stopped,
    /// The handler asked to stop.
    Finished,
    /// The connection ended.
    Lost(Loss),
    /// The stream failed and the command should exit with an error.
    Failed(StreamFailure),
}

/// How a connection ended without either side choosing to stop.
#[derive(Debug)]
pub enum Loss {
    /// The server sent a close frame other than a policy close.
    Closed(Option<CloseFrame<'static>>),
    /// Reading the socket failed, for example because the connection was reset
    /// without a closing handshake.
    Failed(WsError),
    /// The socket ended without a close frame.
    Ended,
}

impl fmt::Display for Loss {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Closed(Some(frame)) if !frame.reason.is_empty() => {
                write!(f, "Connection closed by server: {}", frame.reason)
            }
            Self::Closed(_) => f.write_str("Connection closed by server."),
            Self::Failed(error) => write!(f, "WebSocket error: {error}"),
            Self::Ended => f.write_str("Connection closed."),
        }
    }
}

/// Why a stream ended in failure.
#[derive(Debug)]
pub enum StreamFailure {
    /// The server closed the socket on a policy, such as an expired or
    /// refused session.
    Refused(String),
}

impl StreamFailure {
    /// What a status line shows.
    // Only the TUI has a status line.
    #[cfg_attr(not(feature = "tui"), allow(dead_code))]
    pub fn status(&self) -> String {
        match self {
            Self::Refused(reason) => reason.clone(),
        }
    }
}

impl fmt::Display for StreamFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused(reason) => write!(f, "the server closed the stream: {reason}"),
        }
    }
}

/// What a stream connects to and subscribes with.
pub struct StreamSession {
    url: String,
    refresh: Option<token::SessionRefresh>,
    /// The serialized `subscribe` message.
    subscribe: String,
}

impl StreamSession {
    pub fn new(
        url: String,
        refresh: Option<token::SessionRefresh>,
        subscription: Subscription,
    ) -> Result<Self> {
        let subscribe = serde_json::to_string(&ClientMessage::Subscribe(subscription))
            .context("Failed to serialize subscribe message")?;
        Ok(Self {
            url,
            refresh,
            subscribe,
        })
    }

    /// The URL with any session token redacted.
    pub fn display_url(&self) -> String {
        token::redact_hs_token_for_display(&self.url)
    }

    /// Open the socket and subscribe.
    pub async fn connect(&self) -> Result<Socket> {
        let (mut socket, _) = connect_async(&self.url).await.map_err(|err| {
            let hint = if token::is_hosted_arete_cloud_url(&self.url) {
                "\nHint: hosted stacks need a valid `hs_token` (the CLI adds one after `a4 auth login`). \
                 On some systems, TLS uses the OS trust store — if this persists, report the error above."
            } else {
                ""
            };
            anyhow::anyhow!(
                "Failed to connect to {}: {}{}",
                self.display_url(),
                err,
                hint
            )
        })?;
        socket
            .send(Message::Text(self.subscribe.clone()))
            .await
            .context("Failed to send subscribe message")?;
        Ok(socket)
    }

    /// Read `socket` until the stream ends or `stop` fires, keeping the
    /// session token fresh on the way.
    pub async fn run<H, F>(
        &mut self,
        socket: Socket,
        handler: &mut H,
        mut stop: Pin<&mut F>,
    ) -> Result<StreamEnd>
    where
        H: SessionHandler,
        F: Future<Output = ()>,
    {
        let (mut sink, mut source) = socket.split();
        let mut refresher = token::SessionRefresher::start(self.refresh.clone());
        read_connection(
            &mut sink,
            &mut source,
            &mut refresher,
            handler,
            stop.as_mut(),
        )
        .await
    }
}

/// Read one connection until it ends or `stop` fires.
pub(crate) async fn read_connection<S, R, H, F>(
    sink: &mut S,
    source: &mut R,
    refresher: &mut token::SessionRefresher,
    handler: &mut H,
    mut stop: Pin<&mut F>,
) -> Result<StreamEnd>
where
    S: Sink<Message> + Unpin,
    R: Stream<Item = Result<Message, WsError>> + Unpin,
    H: SessionHandler,
    F: Future<Output = ()>,
{
    let mut ping_interval =
        tokio::time::interval_at(tokio::time::Instant::now() + PING_PERIOD, PING_PERIOD);
    loop {
        tokio::select! {
            () = stop.as_mut() => {
                let _ = sink.close().await;
                return Ok(StreamEnd::Stopped);
            }
            message = source.next() => match message {
                Some(Ok(Message::Binary(bytes))) => {
                    if handle_payload(&bytes, true, handler)? {
                        return Ok(StreamEnd::Finished);
                    }
                }
                Some(Ok(Message::Text(text))) => {
                    if let Some(response) = token::parse_refresh_response(&text) {
                        if !response.success {
                            handler.on_notice(Notice::RefreshRefused(response.error.as_deref()));
                        }
                        continue;
                    }
                    if handle_payload(text.as_bytes(), false, handler)? {
                        return Ok(StreamEnd::Finished);
                    }
                }
                Some(Ok(Message::Ping(payload))) => {
                    let _ = sink.send(Message::Pong(payload)).await;
                }
                Some(Ok(Message::Close(Some(frame)))) if frame.code == CloseCode::Policy => {
                    return Ok(StreamEnd::Failed(StreamFailure::Refused(
                        frame.reason.into_owned(),
                    )));
                }
                Some(Ok(Message::Close(frame))) => {
                    return Ok(StreamEnd::Lost(Loss::Closed(frame.map(CloseFrame::into_owned))));
                }
                Some(Err(error)) => return Ok(StreamEnd::Lost(Loss::Failed(error))),
                None => return Ok(StreamEnd::Lost(Loss::Ended)),
                Some(Ok(_)) => {}
            },
            _ = ping_interval.tick() => {
                if let Ok(message) = serde_json::to_string(&ClientMessage::Ping) {
                    let _ = sink.send(Message::Text(message)).await;
                }
            }
            event = refresher.next() => match event {
                token::RefreshEvent::Token(token) => {
                    if let Ok(message) = serde_json::to_string(&ClientMessage::RefreshAuth { token }) {
                        let _ = sink.send(Message::Text(message)).await;
                    }
                }
                token::RefreshEvent::Failed(error) => {
                    handler.on_notice(Notice::RefreshFailed(&error));
                }
            },
        }
    }
}

fn handle_payload<H: SessionHandler>(bytes: &[u8], binary: bool, handler: &mut H) -> Result<bool> {
    match parse_server_message(bytes) {
        Ok(message) => handler.on_message(message),
        Err(error) => {
            handler.on_notice(Notice::Unparsed {
                binary,
                error: &error,
            });
            Ok(false)
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::api_client::test_support::MockServer;
    use arete_sdk::{Frame, Mode};
    use futures_util::{sink, stream};
    use tokio::sync::oneshot;

    /// An `upsert` for `key` on the `Ore/list` view, as the server sends it.
    pub(crate) fn upsert(subscription_id: &str, key: &str) -> Message {
        let frame = Frame::Upsert {
            protocol_version: 2,
            subscription_id: subscription_id.to_string(),
            mode: Mode::List,
            entity: "Ore/list".to_string(),
            key: key.to_string(),
            data: serde_json::json!({ "id": key }),
            append: Vec::new(),
            seq: None,
            offset: None,
        };
        Message::Text(serde_json::to_string(&frame).expect("frame serializes"))
    }

    /// Records what the session hands the output side.
    #[derive(Default)]
    struct Recorder {
        frames: Vec<Frame>,
        notices: Vec<String>,
        /// Fired by the first notice.
        noticed: Option<oneshot::Sender<()>>,
    }

    impl SessionHandler for Recorder {
        fn on_message(&mut self, message: ServerMessage) -> Result<bool> {
            if let ServerMessage::Frame(frame) = message {
                self.frames.push(frame);
            }
            Ok(false)
        }

        fn on_notice(&mut self, notice: Notice<'_>) {
            self.notices.push(match notice {
                Notice::Unparsed { binary, error } => format!("unparsed {binary}: {error}"),
                Notice::RefreshRefused(reason) => format!("refused: {reason:?}"),
                Notice::RefreshFailed(error) => format!("refresh failed: {error:#}"),
            });
            if let Some(noticed) = self.noticed.take() {
                let _ = noticed.send(());
            }
        }
    }

    async fn read(
        messages: Vec<Result<Message, WsError>>,
        handler: &mut Recorder,
    ) -> Result<StreamEnd> {
        let stop = std::future::pending::<()>();
        tokio::pin!(stop);
        read_connection(
            &mut sink::drain(),
            &mut stream::iter(messages),
            &mut token::SessionRefresher::start(None),
            handler,
            stop,
        )
        .await
    }

    #[tokio::test]
    async fn a_refused_refresh_is_reported_and_a_policy_close_fails_the_stream() {
        let mut handler = Recorder::default();
        let end = read(
            vec![
                Ok(Message::Text(
                    r#"{"success":false,"error":"token-invalid"}"#.to_string(),
                )),
                Ok(Message::Close(Some(CloseFrame {
                    code: CloseCode::Policy,
                    reason: "token-expired: Authentication token expired".into(),
                }))),
            ],
            &mut handler,
        )
        .await
        .unwrap();

        assert_eq!(handler.notices, vec![r#"refused: Some("token-invalid")"#]);
        let StreamEnd::Failed(failure) = end else {
            panic!("a policy close fails the stream: {end:?}");
        };
        assert_eq!(
            failure.status(),
            "token-expired: Authentication token expired"
        );
    }

    #[tokio::test]
    async fn a_plain_close_is_a_loss_not_a_failure() {
        let mut handler = Recorder::default();
        let end = read(vec![Ok(Message::Close(None))], &mut handler)
            .await
            .unwrap();

        assert!(
            matches!(end, StreamEnd::Lost(Loss::Closed(None))),
            "{end:?}"
        );
    }

    #[tokio::test]
    async fn frames_reach_the_handler_until_the_socket_ends() {
        let mut handler = Recorder::default();
        let end = read(
            vec![
                Ok(upsert("cli:test", "1")),
                Ok(Message::Text("not json".to_string())),
            ],
            &mut handler,
        )
        .await
        .unwrap();

        assert!(matches!(end, StreamEnd::Lost(Loss::Ended)), "{end:?}");
        assert!(
            matches!(handler.frames.as_slice(), [Frame::Upsert { key, .. }] if key == "1"),
            "{:?}",
            handler.frames
        );
        assert_eq!(handler.notices.len(), 1);
        assert!(
            handler.notices[0].starts_with("unparsed false"),
            "{:?}",
            handler.notices
        );
    }

    #[tokio::test]
    async fn a_failed_refresh_is_reported() {
        let mint = MockServer::json(500, r#"{"error":"unavailable"}"#);
        let expires_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 2;
        let mut refresher = token::SessionRefresher::start(Some(token::SessionRefresh::for_test(
            format!("{}/ws/sessions", mint.base_url()),
            "wss://ore.stack.arete.run",
            expires_at,
        )));
        let (noticed, notice) = oneshot::channel();
        let mut handler = Recorder {
            noticed: Some(noticed),
            ..Recorder::default()
        };
        // Stop once the failure is reported, or give up after a while.
        let stop = async {
            let _ = tokio::time::timeout(Duration::from_secs(10), notice).await;
        };
        tokio::pin!(stop);

        let end = read_connection(
            &mut sink::drain(),
            &mut stream::pending::<Result<Message, WsError>>(),
            &mut refresher,
            &mut handler,
            stop,
        )
        .await
        .unwrap();

        assert!(matches!(end, StreamEnd::Stopped), "{end:?}");
        assert!(
            handler.notices[0].contains("refresh failed"),
            "{:?}",
            handler.notices
        );
        assert!(handler.notices[0].contains("500"), "{:?}", handler.notices);
    }
}
