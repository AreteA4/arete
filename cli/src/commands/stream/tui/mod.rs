mod app;
mod ui;

use anyhow::Result;
use arete_sdk::{Frame, ServerMessage};
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use tokio::sync::mpsc;

use self::app::{App, TuiAction, ViewMode};
use super::session::{Notice, SessionHandler, StreamEnd, StreamFailure, StreamSession};
use super::token;
use super::StreamArgs;

pub async fn run_tui(
    url: String,
    refresh: Option<token::SessionRefresh>,
    view: &str,
    args: &StreamArgs,
) -> Result<()> {
    // Connect and subscribe
    let mut session = StreamSession::new(
        url,
        refresh,
        crate::commands::stream::build_subscription(view, args),
    )?;
    let socket = session.connect().await?;
    let display_url = session.display_url();

    // Channel for frames from WS task
    // 10k buffer accommodates large snapshot batches during pause. Overflow
    // frames are dropped and counted in the "Dropped: N" header indicator.
    let (frame_tx, mut frame_rx) = mpsc::channel::<Frame>(10_000);

    // Shutdown signal for graceful WebSocket close
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    // Dropped frame counter (shared with WS task)
    let dropped_frames = Arc::new(AtomicU64::new(0));
    let dropped_frames_ws = Arc::clone(&dropped_frames);

    // Warnings for the status bar; the TUI owns the terminal, so nothing is
    // printed.
    let (notice_tx, mut notice_rx) = mpsc::unbounded_channel::<String>();
    // Set when the stream fails rather than ends, such as when the server
    // closes the socket on a policy (an expired or refused session).
    let failure = Arc::new(OnceLock::<StreamFailure>::new());
    let failure_ws = Arc::clone(&failure);

    // Spawn WS reader task
    let ws_handle = tokio::spawn(async move {
        let mut outputs = SocketOutputs {
            frames: frame_tx,
            dropped_frames: dropped_frames_ws,
            notices: notice_tx,
        };
        let stop = async {
            let _ = shutdown_rx.await;
        };
        tokio::pin!(stop);
        if let Ok(StreamEnd::Failed(stream_failure)) = session.run(socket, &mut outputs, stop).await
        {
            let _ = failure_ws.set(stream_failure);
        }
    });

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

    let mut app = App::new(view.to_string(), display_url, Arc::clone(&dropped_frames));

    // Main loop: poll terminal events + receive frames
    let tick_rate = std::time::Duration::from_millis(50);
    let result = run_loop(
        &mut terminal,
        &mut app,
        &mut frame_rx,
        &mut notice_rx,
        &failure,
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
    if let Some(failure) = failure.get() {
        anyhow::bail!("{failure}");
    }
    Ok(())
}

/// Where the socket task hands what it reads to the UI.
struct SocketOutputs {
    frames: mpsc::Sender<Frame>,
    dropped_frames: Arc<AtomicU64>,
    notices: mpsc::UnboundedSender<String>,
}

impl SessionHandler for SocketOutputs {
    fn on_message(&mut self, message: ServerMessage) -> Result<bool> {
        if let ServerMessage::Frame(frame) = message {
            if self.frames.try_send(frame).is_err() {
                self.dropped_frames.fetch_add(1, Ordering::Relaxed);
            }
        }
        Ok(false)
    }

    fn on_notice(&mut self, notice: Notice<'_>) {
        let notice = match notice {
            Notice::Unparsed { .. } => return,
            Notice::RefreshRefused(reason) => format!(
                "Session refresh refused ({}); the stream ends when the current token expires",
                reason.unwrap_or("no reason given")
            ),
            Notice::RefreshFailed(error) => {
                format!("Could not refresh the session token, retrying: {error:#}")
            }
        };
        let _ = self.notices.send(notice);
    }
}

async fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    frame_rx: &mut mpsc::Receiver<Frame>,
    notice_rx: &mut mpsc::UnboundedReceiver<String>,
    failure: &OnceLock<StreamFailure>,
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
                        // The socket task records a failure before it exits
                        // and drops the frame sender.
                        let status = failure.get().map(StreamFailure::status);
                        app.set_disconnected(status.as_deref());
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

    fn outputs() -> (
        SocketOutputs,
        mpsc::Receiver<Frame>,
        mpsc::UnboundedReceiver<String>,
    ) {
        let (frames, frame_rx) = mpsc::channel(1);
        let (notices, notice_rx) = mpsc::unbounded_channel();
        let out = SocketOutputs {
            frames,
            dropped_frames: Arc::new(AtomicU64::new(0)),
            notices,
        };
        (out, frame_rx, notice_rx)
    }

    #[test]
    fn a_refused_refresh_reaches_the_status_bar() {
        let (mut out, _frames, mut notices) = outputs();

        out.on_notice(Notice::RefreshRefused(Some("token-invalid")));

        let notice = notices.try_recv().expect("the refused refresh is reported");
        assert!(notice.contains("token-invalid"), "{notice}");
    }

    #[test]
    fn a_failed_refresh_reaches_the_status_bar() {
        let (mut out, _frames, mut notices) = outputs();

        out.on_notice(Notice::RefreshFailed(&anyhow::anyhow!("mint returned 500")));

        let notice = notices.try_recv().expect("the failed refresh is reported");
        assert!(notice.contains("Could not refresh"), "{notice}");
    }

    #[test]
    fn frames_past_the_buffer_are_counted_as_dropped() {
        let (mut out, mut frames, _notices) = outputs();
        let frame = || {
            ServerMessage::Frame(Frame::Unsubscribed {
                protocol_version: 2,
                subscription_id: "cli:test".to_string(),
            })
        };

        assert!(!out.on_message(frame()).unwrap());
        assert!(!out.on_message(frame()).unwrap());

        assert!(frames.try_recv().is_ok());
        assert_eq!(out.dropped_frames.load(Ordering::Relaxed), 1);
    }
}
