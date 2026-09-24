mod app;
mod ui;

use anyhow::Result;
use arete_sdk::{parse_server_message, ClientMessage, Frame, ServerMessage};
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures_util::{Sink, SinkExt, Stream, StreamExt};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::Error as WsError;
use tokio_tungstenite::{connect_async, tungstenite::Message};

use self::app::{App, TuiAction, ViewMode};
use super::token;
use super::StreamArgs;

pub async fn run_tui(
    url: String,
    refresh: Option<token::SessionRefresh>,
    view: &str,
    args: &StreamArgs,
) -> Result<()> {
    // Connect WebSocket
    let (ws, _) = connect_async(&url).await.map_err(|err| {
        let redacted = token::redact_hs_token_for_display(&url);
        let hint = if token::is_hosted_arete_cloud_url(&url) {
            "\nHint: hosted stacks need a valid `hs_token` (the CLI adds one after `a4 auth login`). \
             On some systems, TLS uses the OS trust store — if this persists, report the error above."
        } else {
            ""
        };
        anyhow::anyhow!("Failed to connect to {}: {}{}", redacted, err, hint)
    })?;

    let (mut ws_tx, ws_rx) = ws.split();

    // Subscribe
    let sub = crate::commands::stream::build_subscription(view, args);
    let msg = serde_json::to_string(&ClientMessage::Subscribe(sub))?;
    ws_tx.send(Message::Text(msg)).await?;

    // Channel for frames from WS task
    // 10k buffer accommodates large snapshot batches during pause. Overflow
    // frames are dropped and counted in the "Dropped: N" header indicator.
    let (frame_tx, mut frame_rx) = mpsc::channel::<Frame>(10_000);

    // Shutdown signal for graceful WebSocket close
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    // Dropped frame counter (shared with WS task)
    let dropped_frames = Arc::new(AtomicU64::new(0));
    let dropped_frames_ws = Arc::clone(&dropped_frames);

    let refresher = token::SessionRefresher::start(refresh);
    // Warnings for the status bar; the TUI owns the terminal, so nothing is
    // printed.
    let (notice_tx, mut notice_rx) = mpsc::unbounded_channel::<String>();
    // Set when the server closes the socket on a policy (an expired or
    // refused session): the stream failed rather than ended.
    let policy_close = Arc::new(OnceLock::<String>::new());
    let policy_close_ws = Arc::clone(&policy_close);

    // Spawn WS reader task
    let ws_handle = tokio::spawn(pump_socket(
        ws_tx,
        ws_rx,
        shutdown_rx,
        refresher,
        SocketOutputs {
            frames: frame_tx,
            dropped_frames: dropped_frames_ws,
            notices: notice_tx,
            policy_close: policy_close_ws,
        },
    ));

    // Setup terminal with panic hook to restore on crash.
    // We store the original hook in a Mutex so we can reclaim it on normal exit.
    let original_hook = Arc::new(std::sync::Mutex::new(Some(std::panic::take_hook())));
    let hook_clone = Arc::clone(&original_hook);
    std::panic::set_hook(Box::new(move |panic_info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        if let Ok(guard) = hook_clone.lock() {
            if let Some(ref orig) = *guard {
                orig(panic_info);
            }
        }
    }));

    enable_raw_mode()?;
    let terminal_setup = || -> Result<Terminal<CrosstermBackend<io::Stdout>>> {
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        Ok(Terminal::new(backend)?)
    };
    let mut terminal = match terminal_setup() {
        Ok(t) => t,
        Err(e) => {
            let _ = disable_raw_mode();
            let _ = execute!(io::stdout(), LeaveAlternateScreen);
            return Err(e);
        }
    };

    let mut app = App::new(
        view.to_string(),
        token::redact_hs_token_for_display(&url),
        Arc::clone(&dropped_frames),
    );

    // Main loop: poll terminal events + receive frames
    let tick_rate = std::time::Duration::from_millis(50);
    let result = run_loop(
        &mut terminal,
        &mut app,
        &mut frame_rx,
        &mut notice_rx,
        &policy_close,
        tick_rate,
    )
    .await;

    // Restore terminal (always attempt all steps)
    let _ = disable_raw_mode();
    let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen,);
    let _ = terminal.show_cursor();

    // Signal graceful shutdown, then wait briefly for the task to close
    let _ = shutdown_tx.send(());
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), ws_handle).await;

    // Restore original panic hook (ours is only needed while TUI is active).
    // Note: if run_loop panics, this block is unreachable and the TUI hook stays
    // installed. This is acceptable since the process terminates on panic anyway.
    let _ = std::panic::take_hook(); // drop our TUI hook
    if let Ok(mut guard) = original_hook.lock() {
        if let Some(hook) = guard.take() {
            std::panic::set_hook(hook);
        }
    }

    result?;
    if let Some(reason) = policy_close.get() {
        anyhow::bail!("the server closed the stream: {reason}");
    }
    Ok(())
}

/// Where the socket task hands what it reads to the UI.
struct SocketOutputs {
    frames: mpsc::Sender<Frame>,
    dropped_frames: Arc<AtomicU64>,
    notices: mpsc::UnboundedSender<String>,
    policy_close: Arc<OnceLock<String>>,
}

/// Read the socket until it closes or `shutdown` fires, keeping the session
/// token fresh on the way.
async fn pump_socket<S, R>(
    mut ws_tx: S,
    mut ws_rx: R,
    mut shutdown: tokio::sync::oneshot::Receiver<()>,
    mut refresher: token::SessionRefresher,
    out: SocketOutputs,
) where
    S: Sink<Message> + Unpin,
    R: Stream<Item = Result<Message, WsError>> + Unpin,
{
    let ping_period = std::time::Duration::from_secs(30);
    let mut ping_interval =
        tokio::time::interval_at(tokio::time::Instant::now() + ping_period, ping_period);
    loop {
        tokio::select! {
            _ = &mut shutdown => {
                let _ = ws_tx.close().await;
                break;
            }
            msg = ws_rx.next() => {
                match msg {
                    Some(Ok(Message::Binary(bytes))) => {
                        match parse_server_message(&bytes) {
                            Ok(ServerMessage::Frame(frame)) => {
                                if out.frames.try_send(frame).is_err() {
                                    out.dropped_frames.fetch_add(1, Ordering::Relaxed);
                                }
                            }
                            Ok(ServerMessage::Error(_)) | Err(_) => {}
                        }
                    }
                    Some(Ok(Message::Text(text))) => {
                        if let Some(response) = token::parse_refresh_response(&text) {
                            if !response.success {
                                let _ = out.notices.send(format!(
                                    "Session refresh refused ({}); the stream ends when the current token expires",
                                    response.error.as_deref().unwrap_or("no reason given")
                                ));
                            }
                            continue;
                        }
                        if let Ok(ServerMessage::Frame(frame)) =
                            parse_server_message(text.as_bytes())
                        {
                            if out.frames.try_send(frame).is_err() {
                                out.dropped_frames.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        let _ = ws_tx.send(Message::Pong(payload)).await;
                    }
                    Some(Ok(Message::Close(Some(frame)))) if frame.code == CloseCode::Policy => {
                        let _ = out.policy_close.set(frame.reason.into_owned());
                        break;
                    }
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                    _ => {}
                }
            }
            _ = ping_interval.tick() => {
                if let Ok(msg) = serde_json::to_string(&ClientMessage::Ping) {
                    let _ = ws_tx.send(Message::Text(msg)).await;
                }
            }
            event = refresher.next() => match event {
                token::RefreshEvent::Token(token) => {
                    if let Ok(msg) = serde_json::to_string(&ClientMessage::RefreshAuth { token }) {
                        let _ = ws_tx.send(Message::Text(msg)).await;
                    }
                }
                token::RefreshEvent::Failed(error) => {
                    let _ = out.notices.send(format!(
                        "Could not refresh the session token, retrying: {error:#}"
                    ));
                }
            },
        }
    }
}

async fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    frame_rx: &mut mpsc::Receiver<Frame>,
    notice_rx: &mut mpsc::UnboundedReceiver<String>,
    policy_close: &OnceLock<String>,
    tick_rate: std::time::Duration,
) -> Result<()> {
    loop {
        // Update visible rows from terminal size (minus header/timeline/status/borders)
        let term_size = terminal.size()?;
        // 3 fixed rows (header + timeline + status) + 2 border rows = 5
        app.visible_rows = term_size.height.saturating_sub(5) as usize;
        app.terminal_width = term_size.width;

        terminal.draw(|f| ui::draw(f, app))?;

        // Drain available frames (non-blocking). When paused, leave
        // frames in the channel so they're applied on resume.
        if !app.paused {
            loop {
                match frame_rx.try_recv() {
                    Ok(frame) => app.apply_frame(frame),
                    Err(mpsc::error::TryRecvError::Disconnected) => {
                        // The socket task records a policy close before it
                        // exits and drops the frame sender.
                        app.set_disconnected(policy_close.get().map(String::as_str));
                        break;
                    }
                    Err(mpsc::error::TryRecvError::Empty) => break,
                }
            }
        }
        while let Ok(notice) = notice_rx.try_recv() {
            app.set_status(&notice);
        }

        // Poll for terminal events with timeout
        if event::poll(tick_rate)? {
            if let Event::Key(key) = event::read()? {
                // When filter input is active, capture all keys for typing
                let action = if app.filter_input_active {
                    match key.code {
                        KeyCode::Esc => TuiAction::BackToList,
                        KeyCode::Enter => TuiAction::BackToList,
                        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            TuiAction::Quit
                        }
                        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            TuiAction::FilterClear
                        }
                        KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            TuiAction::FilterDeleteWord
                        }
                        // Ignore other control/alt combos — don't insert them as text
                        KeyCode::Char(_)
                            if key
                                .modifiers
                                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                        {
                            continue
                        }
                        KeyCode::Char(c) => TuiAction::FilterChar(c),
                        KeyCode::Backspace => TuiAction::FilterBackspace,
                        _ => continue,
                    }
                } else {
                    // Number prefix accumulation (vim count)
                    if let KeyCode::Char(c @ '0'..='9') = key.code {
                        // Don't treat '0' as count start (could be "go to beginning" in future)
                        if c != '0' || app.pending_count.is_some() {
                            let digit = c as usize - '0' as usize;
                            let current = app.pending_count.unwrap_or(0);
                            app.pending_count = Some(
                                (current.saturating_mul(10).saturating_add(digit)).min(99_999),
                            );
                            app.pending_g = false;
                            continue;
                        }
                    }

                    match key.code {
                        KeyCode::Char('q') => TuiAction::Quit,
                        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            TuiAction::Quit
                        }
                        // In Detail mode: j/k scroll the JSON pane; arrows still navigate entities
                        KeyCode::Char('j') => {
                            if app.view_mode == ViewMode::Detail {
                                TuiAction::ScrollDetailDown
                            } else {
                                TuiAction::NextEntity
                            }
                        }
                        KeyCode::Char('k') => {
                            if app.view_mode == ViewMode::Detail {
                                TuiAction::ScrollDetailUp
                            } else {
                                TuiAction::PrevEntity
                            }
                        }
                        KeyCode::Down => TuiAction::NextEntity,
                        KeyCode::Up => TuiAction::PrevEntity,
                        KeyCode::Char('G') => {
                            if app.view_mode == ViewMode::Detail {
                                TuiAction::ScrollDetailBottom
                            } else {
                                TuiAction::GotoBottom
                            }
                        }
                        KeyCode::Char('g') => {
                            if app.pending_g {
                                // gg = go to top (of list or detail pane)
                                if app.view_mode == ViewMode::Detail {
                                    TuiAction::ScrollDetailTop
                                } else {
                                    TuiAction::GotoTop
                                }
                            } else {
                                app.pending_g = true;
                                app.pending_count = None;
                                continue;
                            }
                        }
                        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            if app.view_mode == ViewMode::Detail {
                                TuiAction::ScrollDetailHalfDown
                            } else {
                                TuiAction::HalfPageDown
                            }
                        }
                        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            if app.view_mode == ViewMode::Detail {
                                TuiAction::ScrollDetailHalfUp
                            } else {
                                TuiAction::HalfPageUp
                            }
                        }
                        KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            TuiAction::ScrollDetailDown
                        }
                        KeyCode::Char('y') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            TuiAction::ScrollDetailUp
                        }
                        KeyCode::PageDown => TuiAction::ScrollDetailDown,
                        KeyCode::PageUp => TuiAction::ScrollDetailUp,
                        KeyCode::Char('n') => TuiAction::NextMatch,
                        KeyCode::Enter => TuiAction::FocusDetail,
                        KeyCode::Esc => {
                            app.pending_count = None;
                            app.pending_g = false;
                            TuiAction::BackToList
                        }
                        KeyCode::Right | KeyCode::Char('l') => TuiAction::HistoryForward,
                        KeyCode::Left | KeyCode::Char('h') => {
                            if app.pending_g {
                                app.pending_g = false;
                                continue;
                            }
                            TuiAction::HistoryBack
                        }
                        KeyCode::Home => TuiAction::HistoryOldest,
                        KeyCode::End => TuiAction::HistoryNewest,
                        KeyCode::Char('d') => TuiAction::ToggleDiff,
                        KeyCode::Char('r') => TuiAction::ToggleRaw,
                        KeyCode::Char('p') => TuiAction::TogglePause,
                        KeyCode::Char('/') => TuiAction::StartFilter,
                        KeyCode::Char('s') => TuiAction::CycleSortMode,
                        KeyCode::Char('o') => TuiAction::ToggleSortDirection,
                        KeyCode::Char('S') => TuiAction::SaveSnapshot,
                        _ => {
                            app.pending_count = None;
                            app.pending_g = false;
                            continue;
                        }
                    }
                };

                if let TuiAction::Quit = action {
                    break;
                }
                app.handle_action(action);
            }
            // Resize and other events are handled implicitly:
            // layout is recalculated from terminal.size() at the top of each loop iteration
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api_client::test_support::MockServer;
    use futures_util::{sink, stream};
    use std::time::Duration;
    use tokio_tungstenite::tungstenite::protocol::CloseFrame;

    fn outputs() -> (
        SocketOutputs,
        mpsc::UnboundedReceiver<String>,
        Arc<OnceLock<String>>,
    ) {
        let (frames, _) = mpsc::channel(16);
        let (notices, notice_rx) = mpsc::unbounded_channel();
        let policy_close = Arc::new(OnceLock::new());
        let out = SocketOutputs {
            frames,
            dropped_frames: Arc::new(AtomicU64::new(0)),
            notices,
            policy_close: Arc::clone(&policy_close),
        };
        (out, notice_rx, policy_close)
    }

    #[tokio::test]
    async fn a_refused_refresh_and_a_policy_close_reach_the_ui() {
        let (out, mut notices, policy_close) = outputs();
        let (_shutdown_tx, shutdown) = tokio::sync::oneshot::channel();
        let socket = stream::iter(vec![
            Ok(Message::Text(
                r#"{"success":false,"error":"token-invalid"}"#.to_string(),
            )),
            Ok(Message::Close(Some(CloseFrame {
                code: CloseCode::Policy,
                reason: "token-expired: Authentication token expired".into(),
            }))),
        ]);

        pump_socket(
            sink::drain(),
            socket,
            shutdown,
            token::SessionRefresher::start(None),
            out,
        )
        .await;

        let notice = notices.try_recv().expect("the refused refresh is reported");
        assert!(notice.contains("token-invalid"), "{notice}");
        assert_eq!(
            policy_close.get().map(String::as_str),
            Some("token-expired: Authentication token expired")
        );
    }

    #[tokio::test]
    async fn a_plain_close_is_not_a_failure() {
        let (out, _notices, policy_close) = outputs();
        let (_shutdown_tx, shutdown) = tokio::sync::oneshot::channel();

        pump_socket(
            sink::drain(),
            stream::iter(vec![Ok(Message::Close(None))]),
            shutdown,
            token::SessionRefresher::start(None),
            out,
        )
        .await;

        assert!(policy_close.get().is_none());
    }

    #[tokio::test]
    async fn a_failed_refresh_is_reported() {
        let mint = MockServer::json(500, r#"{"error":"unavailable"}"#);
        let expires_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 2;
        let refresher = token::SessionRefresher::start(Some(token::SessionRefresh::for_test(
            format!("{}/ws/sessions", mint.base_url()),
            "wss://ore.stack.arete.run",
            expires_at,
        )));
        let (out, mut notices, _) = outputs();
        let (shutdown_tx, shutdown) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(pump_socket(
            sink::drain(),
            stream::pending::<Result<Message, WsError>>(),
            shutdown,
            refresher,
            out,
        ));

        let notice = tokio::time::timeout(Duration::from_secs(10), notices.recv())
            .await
            .expect("a refresh attempt within the timeout")
            .expect("the socket task is still running");
        assert!(notice.contains("Could not refresh"), "{notice}");

        let _ = shutdown_tx.send(());
        task.await.unwrap();
    }
}
