mod app;
mod ui;

use anyhow::{Context, Result};
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures_util::{SinkExt, StreamExt};
use hyperstack_sdk::{parse_frame, ClientMessage, Frame};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};

use self::app::{App, TuiAction};
use super::StreamArgs;

pub async fn run_tui(url: String, view: &str, args: &StreamArgs) -> Result<()> {
    // Connect WebSocket
    let (ws, _) = connect_async(&url)
        .await
        .with_context(|| format!("Failed to connect to {}", url))?;

    let (mut ws_tx, mut ws_rx) = ws.split();

    // Subscribe
    let sub = crate::commands::stream::build_subscription(view, args);
    let msg = serde_json::to_string(&ClientMessage::Subscribe(sub))?;
    ws_tx.send(Message::Text(msg)).await?;

    // Channel for frames from WS task
    let (frame_tx, mut frame_rx) = mpsc::channel::<Frame>(1000);

    // Shutdown signal for graceful WebSocket close
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    // Spawn WS reader task
    let ws_handle = tokio::spawn(async move {
        let ping_period = std::time::Duration::from_secs(30);
        let mut ping_interval = tokio::time::interval_at(tokio::time::Instant::now() + ping_period, ping_period);
        loop {
            tokio::select! {
                _ = &mut shutdown_rx => {
                    let _ = ws_tx.close().await;
                    break;
                }
                msg = ws_rx.next() => {
                    match msg {
                        Some(Ok(Message::Binary(bytes))) => {
                            if let Ok(frame) = parse_frame(&bytes) {
                                // Non-blocking: drop frames when receiver is paused/full
                                let _ = frame_tx.try_send(frame);
                            }
                        }
                        Some(Ok(Message::Text(text))) => {
                            if let Ok(frame) = serde_json::from_str::<Frame>(&text) {
                                let _ = frame_tx.try_send(frame);
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

    // Setup terminal with panic hook to restore on crash
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        original_hook(panic_info);
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
            return Err(e);
        }
    };

    let mut app = App::new(view.to_string(), url.clone());

    // Main loop: poll terminal events + receive frames
    let tick_rate = std::time::Duration::from_millis(50);
    let result = run_loop(&mut terminal, &mut app, &mut frame_rx, tick_rate).await;

    // Restore terminal (always attempt all steps)
    let _ = disable_raw_mode();
    let _ = execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
    );
    let _ = terminal.show_cursor();

    // Signal graceful shutdown, then wait briefly for the task to close
    let _ = shutdown_tx.send(());
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), ws_handle).await;

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
        app.visible_rows = term_size.height.saturating_sub(6) as usize;

        terminal.draw(|f| ui::draw(f, app))?;

        // Drain available frames (non-blocking). When paused, leave
        // frames in the channel so they're applied on resume.
        if !app.paused {
            while let Ok(frame) = frame_rx.try_recv() {
                app.apply_frame(frame);
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
                            app.pending_count = Some(current * 10 + digit);
                            app.pending_g = false;
                            continue;
                        }
                    }

                    match key.code {
                        KeyCode::Char('q') => TuiAction::Quit,
                        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            TuiAction::Quit
                        }
                        KeyCode::Down | KeyCode::Char('j') => TuiAction::NextEntity,
                        KeyCode::Up | KeyCode::Char('k') => TuiAction::PrevEntity,
                        KeyCode::Char('G') => TuiAction::GotoBottom,
                        KeyCode::Char('g') => {
                            if app.pending_g {
                                // gg = go to top
                                TuiAction::GotoTop
                            } else {
                                app.pending_g = true;
                                app.pending_count = None;
                                continue;
                            }
                        }
                        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            TuiAction::HalfPageDown
                        }
                        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            TuiAction::HalfPageUp
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
                        KeyCode::Char('s') => TuiAction::SaveSnapshot,
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
