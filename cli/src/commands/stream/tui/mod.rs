mod app;
mod ui;

use anyhow::Result;
use arete_sdk::{parse_frame, try_parse_subscribed_frame, ClientMessage, Frame};
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures_util::{SinkExt, StreamExt};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};

use self::app::{App, TuiAction, ViewMode};
use super::token;
use super::StreamArgs;

pub async fn run_tui(url: String, view: &str, args: &StreamArgs) -> Result<()> {
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

    let (mut ws_tx, mut ws_rx) = ws.split();

    // Subscribe
    let sub = crate::commands::stream::build_subscription(view, args);
    let msg = serde_json::to_string(&ClientMessage::Subscribe(sub))?;
    ws_tx.send(Message::Text(msg)).await?;

    // Channel for frames from WS task
    // 10k buffer accommodates large snapshot batches during pause. Overflow
    // frames are dropped and counted in the "Dropped: N" header indicator.
    let (frame_tx, mut frame_rx) = mpsc::channel::<Frame>(10_000);

    // Shutdown signal for graceful WebSocket close
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    // Dropped frame counter (shared with WS task)
    let dropped_frames = Arc::new(AtomicU64::new(0));
    let dropped_frames_ws = Arc::clone(&dropped_frames);

    // Spawn WS reader task
    let ws_handle = tokio::spawn(async move {
        let ping_period = std::time::Duration::from_secs(30);
        let mut ping_interval =
            tokio::time::interval_at(tokio::time::Instant::now() + ping_period, ping_period);
        loop {
            tokio::select! {
                _ = &mut shutdown_rx => {
                    let _ = ws_tx.close().await;
                    break;
                }
                msg = ws_rx.next() => {
                    match msg {
                        Some(Ok(Message::Binary(bytes))) => {
                            match parse_frame(&bytes) {
                                Ok(frame) => {
                                    if frame_tx.try_send(frame).is_err() {
                                        dropped_frames_ws.fetch_add(1, Ordering::Relaxed);
                                    }
                                }
                                Err(_) => {
                                    // Subscribed frames have a different shape (no `entity` field)
                                    if try_parse_subscribed_frame(&bytes).is_some() {
                                        let subscribed = Frame {
                                            mode: arete_sdk::Mode::List,
                                            entity: String::new(),
                                            op: "subscribed".to_string(),
                                            key: String::new(),
                                            data: serde_json::Value::Null,
                                            append: Vec::new(),
                                            seq: None,
                                        };
                                        let _ = frame_tx.try_send(subscribed);
                                    }
                                }
                            }
                        }
                        Some(Ok(Message::Text(text))) => {
                            if let Ok(frame) = serde_json::from_str::<Frame>(&text) {
                                if frame_tx.try_send(frame).is_err() {
                                    dropped_frames_ws.fetch_add(1, Ordering::Relaxed);
                                }
                            }
                        }
                        Some(Ok(Message::Ping(payload))) => {
                            let _ = ws_tx.send(Message::Pong(payload)).await;
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
            }
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

    let mut app = App::new(
        view.to_string(),
        token::redact_hs_token_for_display(&url),
        Arc::clone(&dropped_frames),
    );

    // Main loop: poll terminal events + receive frames
    let tick_rate = std::time::Duration::from_millis(50);
    let result = run_loop(&mut terminal, &mut app, &mut frame_rx, tick_rate).await;

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

    result
}

async fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    frame_rx: &mut mpsc::Receiver<Frame>,
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
                        app.set_disconnected();
                        break;
                    }
                    Err(mpsc::error::TryRecvError::Empty) => break,
                }
            }
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
