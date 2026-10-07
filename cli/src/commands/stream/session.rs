//! The WebSocket session behind `a4 stream`: opening the socket, subscribing,
//! reading until the stream ends, and replacing a connection that drops. The
//! plain and TUI outputs share it and differ only in what they do with each
//! message.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, Instant};

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

    /// A new connection to `url` (redacted) has replaced a lost one and
    /// resubscribed. Nothing built from the old connection's frames carries
    /// over: the new subscription starts again with a snapshot.
    fn on_reconnected(&mut self, url: &str) -> Result<()>;
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
    /// The connection is down and the next attempt to replace it starts
    /// after `delay`.
    Reconnecting(Reconnecting<'a>),
}

/// A pending reconnect attempt.
pub struct Reconnecting<'a> {
    /// Why the connection, or the previous attempt to replace it, failed.
    pub reason: &'a str,
    /// Whether `reason` is a failed attempt rather than the lost connection.
    pub retry: bool,
    /// This attempt's number, counting from 1 since the last stable connection.
    pub attempt: u32,
    /// The most attempts that will be made, if limited.
    pub max_attempts: Option<u32>,
    pub delay: Duration,
}

impl fmt::Display for Reconnecting<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (what, next) = if self.retry {
            ("Reconnect failed", "retrying")
        } else {
            ("Connection lost", "reconnecting")
        };
        write!(
            f,
            "{what} ({}); {next} in {:.1}s (attempt {}",
            self.reason,
            self.delay.as_secs_f64(),
            self.attempt
        )?;
        if let Some(max) = self.max_attempts {
            write!(f, " of {max}")?;
        }
        f.write_str(")...")
    }
}

/// How a stream ended.
#[derive(Debug)]
pub enum StreamEnd {
    /// The caller's stop signal fired (Ctrl+C, `--duration`, quitting the
    /// TUI) and the socket was closed.
    Stopped,
    /// The handler asked to stop.
    Finished,
    /// The connection ended and was not replaced, because reconnecting is off
    /// or a new connection would end the same way.
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

impl Loss {
    /// Whether a new connection could do better.
    ///
    /// A server restart, a reset connection or an ordinary close is worth
    /// retrying. A close for a protocol violation is not: the new connection
    /// sends the same subscription and would be closed the same way.
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Closed(Some(frame)) => !matches!(
                frame.code,
                CloseCode::Protocol
                    | CloseCode::Unsupported
                    | CloseCode::Invalid
                    | CloseCode::Size
                    | CloseCode::Extension
            ),
            Self::Closed(None) | Self::Failed(_) | Self::Ended => true,
        }
    }

    /// Why the connection ended, for a reconnect notice.
    fn reason(&self) -> String {
        match self {
            Self::Closed(Some(frame)) if !frame.reason.is_empty() => {
                format!("closed by server: {}", frame.reason)
            }
            Self::Closed(Some(frame)) => {
                format!("closed by server with code {}", u16::from(frame.code))
            }
            Self::Closed(None) => "closed by server".to_string(),
            Self::Failed(error) => error.to_string(),
            Self::Ended => "connection ended".to_string(),
        }
    }
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
    /// refused session. Reconnecting would be refused the same way.
    Refused(String),
    /// The connection dropped and every attempt to replace it failed.
    GaveUp { attempts: u32, reason: String },
}

impl StreamFailure {
    /// What a status line shows.
    // Only the TUI has a status line.
    #[cfg_attr(not(feature = "tui"), allow(dead_code))]
    pub fn status(&self) -> String {
        match self {
            Self::Refused(reason) => reason.clone(),
            Self::GaveUp { .. } => self.to_string(),
        }
    }
}

impl fmt::Display for StreamFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused(reason) => write!(f, "the server closed the stream: {reason}"),
            Self::GaveUp { attempts, reason } => write!(
                f,
                "gave up after {attempts} reconnect attempt{}: {reason}",
                if *attempts == 1 { "" } else { "s" }
            ),
        }
    }
}

/// When and how often a dropped connection is replaced.
#[derive(Debug, Clone)]
pub struct ReconnectPolicy {
    /// Off with `--no-reconnect`: a dropped connection ends the stream.
    pub enabled: bool,
    /// Attempts in a row before giving up; `None` keeps trying.
    pub max_attempts: Option<u32>,
    /// The longest wait before the first attempt. It doubles for each
    /// further attempt.
    pub initial_delay: Duration,
    /// The longest wait before any attempt.
    pub max_delay: Duration,
    /// How long a connection must stay up for the next drop to count attempts
    /// from 1 again.
    pub stable_after: Duration,
    /// How long one attempt may take to connect and resubscribe.
    pub connect_timeout: Duration,
}

impl ReconnectPolicy {
    pub fn new(enabled: bool, max_attempts: Option<u32>) -> Self {
        Self {
            enabled,
            max_attempts,
            initial_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(30),
            stable_after: Duration::from_secs(30),
            connect_timeout: Duration::from_secs(15),
        }
    }

    /// The wait before attempt `attempt` (counting from 1), given a `jitter`
    /// between 0 and 1.
    ///
    /// The ceiling grows exponentially from `initial_delay` up to `max_delay`,
    /// and the wait falls in its upper half, so clients dropped together by a
    /// server restart do not all come back at the same moment.
    pub fn delay(&self, attempt: u32, jitter: f64) -> Duration {
        let doublings = attempt.saturating_sub(1).min(31);
        let ceiling = self
            .initial_delay
            .saturating_mul(1 << doublings)
            .min(self.max_delay);
        ceiling.mul_f64(0.5 + 0.5 * jitter.clamp(0.0, 1.0))
    }
}

/// A number between 0 and 1 that differs from call to call.
fn jitter() -> f64 {
    use std::hash::{BuildHasher, Hasher};
    // Each `RandomState` is keyed differently, which is all the randomness
    // spreading reconnects out needs.
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u8(0);
    (hasher.finish() >> 11) as f64 / (1u64 << 53) as f64
}

/// What a stream connects to and subscribes with.
pub struct StreamSession {
    url: String,
    refresh: Option<token::SessionRefresh>,
    /// The serialized `subscribe` message, sent again on every connection.
    subscribe: String,
    policy: ReconnectPolicy,
}

impl StreamSession {
    pub fn new(
        url: String,
        refresh: Option<token::SessionRefresh>,
        subscription: Subscription,
        policy: ReconnectPolicy,
    ) -> Result<Self> {
        let subscribe = serde_json::to_string(&ClientMessage::Subscribe(subscription))
            .context("Failed to serialize subscribe message")?;
        Ok(Self {
            url,
            refresh,
            subscribe,
            policy,
        })
    }

    /// The URL with any session token redacted.
    pub fn display_url(&self) -> String {
        token::redact_hs_token_for_display(&self.url)
    }

    /// Open the socket and subscribe.
    ///
    /// The first connection is not retried: a stream that never started is
    /// more likely misconfigured than interrupted.
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

    /// Connect again after a drop, with a fresh session token when the CLI
    /// minted the first one: the old token may have expired, and the server
    /// only accepted later ones on the connection that is gone.
    async fn reconnect(&mut self) -> Result<Socket> {
        if let Some(refresh) = &mut self.refresh {
            self.url = refresh
                .renew()
                .await
                .context("could not mint a session token")?;
        }
        // The WebSocket error already names its cause, so it is formatted in
        // rather than chained, which would print the cause twice.
        let connecting = async {
            let (mut socket, _) = connect_async(&self.url)
                .await
                .map_err(|error| anyhow::anyhow!("could not connect: {error}"))?;
            socket
                .send(Message::Text(self.subscribe.clone()))
                .await
                .map_err(|error| anyhow::anyhow!("could not resubscribe: {error}"))?;
            Ok(socket)
        };
        tokio::time::timeout(self.policy.connect_timeout, connecting)
            .await
            .unwrap_or_else(|_| {
                Err(anyhow::anyhow!(
                    "timed out after {}s",
                    self.policy.connect_timeout.as_secs_f64()
                ))
            })
    }

    /// Read `socket`, and each connection that replaces it, until the stream
    /// ends or `stop` fires. `stop` is shared by every connection and by the
    /// waits between them, so a deadline keeps counting across reconnects.
    pub async fn run<H, F>(
        &mut self,
        mut socket: Socket,
        handler: &mut H,
        mut stop: Pin<&mut F>,
    ) -> Result<StreamEnd>
    where
        H: SessionHandler,
        F: Future<Output = ()>,
    {
        let mut attempt: u32 = 0;
        loop {
            let connected_at = Instant::now();
            let loss = match self.read(socket, handler, stop.as_mut()).await? {
                StreamEnd::Lost(loss) if self.policy.enabled && loss.is_retryable() => loss,
                end => return Ok(end),
            };
            if connected_at.elapsed() >= self.policy.stable_after {
                attempt = 0;
            }

            let mut reason = loss.reason();
            let mut retry = false;
            socket = loop {
                attempt = attempt.saturating_add(1);
                if self.policy.max_attempts.is_some_and(|max| attempt > max) {
                    return Ok(StreamEnd::Failed(StreamFailure::GaveUp {
                        attempts: attempt - 1,
                        reason,
                    }));
                }
                let delay = self.policy.delay(attempt, jitter());
                handler.on_notice(Notice::Reconnecting(Reconnecting {
                    reason: &reason,
                    retry,
                    attempt,
                    max_attempts: self.policy.max_attempts,
                    delay,
                }));
                tokio::select! {
                    () = tokio::time::sleep(delay) => {}
                    () = stop.as_mut() => return Ok(StreamEnd::Stopped),
                }
                let connected = tokio::select! {
                    connected = self.reconnect() => connected,
                    () = stop.as_mut() => return Ok(StreamEnd::Stopped),
                };
                match connected {
                    Ok(socket) => break socket,
                    Err(error) => {
                        reason = format!("{error:#}");
                        retry = true;
                    }
                }
            };
            handler.on_reconnected(&self.display_url())?;
        }
    }

    /// Read one connection until it ends or `stop` fires, keeping the
    /// session token fresh on the way.
    async fn read<H, F>(
        &self,
        socket: Socket,
        handler: &mut H,
        stop: Pin<&mut F>,
    ) -> Result<StreamEnd>
    where
        H: SessionHandler,
        F: Future<Output = ()>,
    {
        let (mut sink, mut source) = socket.split();
        let mut refresher = token::SessionRefresher::start(self.refresh.clone());
        read_connection(&mut sink, &mut source, &mut refresher, handler, stop).await
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
                Notice::Reconnecting(reconnecting) => reconnecting.to_string(),
            });
            if let Some(noticed) = self.noticed.take() {
                let _ = noticed.send(());
            }
        }

        fn on_reconnected(&mut self, url: &str) -> Result<()> {
            self.notices.push(format!("reconnected to {url}"));
            Ok(())
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

    fn policy() -> ReconnectPolicy {
        ReconnectPolicy::new(true, None)
    }

    #[test]
    fn the_wait_doubles_from_half_a_second() {
        let policy = policy();
        let ceilings: Vec<_> = (1..=4).map(|attempt| policy.delay(attempt, 1.0)).collect();
        assert_eq!(
            ceilings,
            vec![
                Duration::from_millis(500),
                Duration::from_secs(1),
                Duration::from_secs(2),
                Duration::from_secs(4),
            ]
        );
    }

    #[test]
    fn jitter_keeps_the_wait_in_the_upper_half() {
        let policy = policy();
        assert_eq!(policy.delay(1, 0.0), Duration::from_millis(250));
        assert_eq!(policy.delay(3, 0.5), Duration::from_millis(1_500));
        for _ in 0..100 {
            let delay = policy.delay(2, jitter());
            assert!(
                (Duration::from_millis(500)..=Duration::from_secs(1)).contains(&delay),
                "{delay:?}"
            );
        }
    }

    #[test]
    fn the_wait_stops_growing_at_the_cap() {
        let policy = policy();
        assert_eq!(policy.delay(7, 1.0), Duration::from_secs(30));
        assert_eq!(policy.delay(1_000, 1.0), Duration::from_secs(30));
        assert_eq!(policy.delay(u32::MAX, 1.0), Duration::from_secs(30));
        assert_eq!(policy.delay(u32::MAX, 0.0), Duration::from_secs(15));
    }

    #[test]
    fn jitter_varies_between_calls() {
        let samples: std::collections::HashSet<u64> = (0..20).map(|_| jitter().to_bits()).collect();
        assert!(samples.len() > 1);
        assert!((0..100).map(|_| jitter()).all(|j| (0.0..1.0).contains(&j)));
    }

    fn closed(code: CloseCode) -> Loss {
        Loss::Closed(Some(CloseFrame {
            code,
            reason: "".into(),
        }))
    }

    #[test]
    fn drops_and_restarts_are_retried() {
        assert!(Loss::Ended.is_retryable());
        assert!(Loss::Failed(WsError::Protocol(
            tokio_tungstenite::tungstenite::error::ProtocolError::ResetWithoutClosingHandshake,
        ))
        .is_retryable());
        assert!(Loss::Closed(None).is_retryable());
        for code in [
            CloseCode::Normal,
            CloseCode::Away,
            CloseCode::Restart,
            CloseCode::Again,
            CloseCode::Error,
            CloseCode::Library(4000),
        ] {
            assert!(closed(code).is_retryable(), "{code:?}");
        }
    }

    #[test]
    fn closes_a_new_connection_would_repeat_are_not_retried() {
        for code in [
            CloseCode::Protocol,
            CloseCode::Unsupported,
            CloseCode::Invalid,
            CloseCode::Size,
            CloseCode::Extension,
        ] {
            assert!(!closed(code).is_retryable(), "{code:?}");
        }
    }

    #[test]
    fn a_reconnect_notice_says_why_when_and_which_attempt() {
        let first = Reconnecting {
            reason: "connection ended",
            retry: false,
            attempt: 1,
            max_attempts: None,
            delay: Duration::from_millis(1_234),
        };
        assert_eq!(
            first.to_string(),
            "Connection lost (connection ended); reconnecting in 1.2s (attempt 1)..."
        );
        let later = Reconnecting {
            reason: "could not connect: refused",
            retry: true,
            attempt: 2,
            max_attempts: Some(5),
            delay: Duration::from_secs(1),
        };
        assert_eq!(
            later.to_string(),
            "Reconnect failed (could not connect: refused); retrying in 1.0s (attempt 2 of 5)..."
        );
    }

    #[test]
    fn a_lost_connection_names_its_cause() {
        assert_eq!(Loss::Ended.reason(), "connection ended");
        assert_eq!(Loss::Closed(None).reason(), "closed by server");
        assert_eq!(
            closed(CloseCode::Away).reason(),
            "closed by server with code 1001"
        );
        assert_eq!(
            Loss::Closed(Some(CloseFrame {
                code: CloseCode::Restart,
                reason: "restarting".into(),
            }))
            .reason(),
            "closed by server: restarting"
        );
    }

    #[test]
    fn giving_up_reports_the_attempts_and_the_last_reason() {
        let failure = StreamFailure::GaveUp {
            attempts: 3,
            reason: "could not connect: refused".to_string(),
        };
        assert_eq!(
            failure.to_string(),
            "gave up after 3 reconnect attempts: could not connect: refused"
        );
        assert_eq!(failure.status(), failure.to_string());
    }
}
