mod app;
mod ui;

use anyhow::{Context, Result};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures_util::{SinkExt, StreamExt};
use hyperstack_sdk::{parse_frame, ClientMessage, Frame, Subscription};
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
    let mut sub = Subscription::new(view);
    if let Some(key) = &args.key {
        sub = sub.with_key(key.clone());
    }
    if let Some(take) = args.take {
        sub = sub.with_take(take);
    }
    if let Some(skip) = args.skip {
        sub = sub.with_skip(skip);
    }
    if args.no_snapshot {
        sub = sub.with_snapshot(false);
    }
    if let Some(after) = &args.after {
        sub = sub.after(after.clone());
    }

    let msg = serde_json::to_string(&ClientMessage::Subscribe(sub))?;
    ws_tx.send(Message::Text(msg)).await?;

    // Channel for frames from WS task
    let (frame_tx, mut frame_rx) = mpsc::channel::<Frame>(1000);

    // Spawn WS reader task
    let ws_handle = tokio::spawn(async move {
        let mut ping_interval = tokio::time::interval(std::time::Duration::from_secs(30));
        loop {
            tokio::select! {
                msg = ws_rx.next() => {
                    match msg {
                        Some(Ok(Message::Binary(bytes))) => {
                            if let Ok(frame) = parse_frame(&bytes) {
                                if frame_tx.send(frame).await.is_err() {
                                    break;
                                }
                            }
                        }
                        Some(Ok(Message::Text(text))) => {
                            if let Ok(frame) = serde_json::from_str::<Frame>(&text) {
                                if frame_tx.send(frame).await.is_err() {
                                    break;
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

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(view.to_string(), url.clone());

    // Main loop: poll terminal events + receive frames
    let tick_rate = std::time::Duration::from_millis(50);
    let result = run_loop(&mut terminal, &mut app, &mut frame_rx, tick_rate).await;

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture,
    )?;
    terminal.show_cursor()?;

    ws_handle.abort();

    result
}

async fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    frame_rx: &mut mpsc::Receiver<Frame>,
    tick_rate: std::time::Duration,
) -> Result<()> {
    loop {
        terminal.draw(|f| ui::draw(f, app))?;

        // Drain available frames (non-blocking)
        while let Ok(frame) = frame_rx.try_recv() {
            if !app.paused {
                app.apply_frame(frame);
            }
        }

        // Poll for terminal events with timeout
        if event::poll(tick_rate)? {
            if let Event::Key(key) = event::read()? {
                let action = match key.code {
                    KeyCode::Char('q') => TuiAction::Quit,
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        TuiAction::Quit
                    }
                    KeyCode::Down | KeyCode::Char('j') => TuiAction::NextEntity,
                    KeyCode::Up | KeyCode::Char('k') => TuiAction::PrevEntity,
                    KeyCode::Enter => TuiAction::FocusDetail,
                    KeyCode::Esc => TuiAction::BackToList,
                    KeyCode::Right | KeyCode::Char('l') => TuiAction::HistoryForward,
                    KeyCode::Left | KeyCode::Char('h') => TuiAction::HistoryBack,
                    KeyCode::Home => TuiAction::HistoryOldest,
                    KeyCode::End => TuiAction::HistoryNewest,
                    KeyCode::Char('d') => TuiAction::ToggleDiff,
                    KeyCode::Char('r') => TuiAction::ToggleRaw,
                    KeyCode::Char('p') => TuiAction::TogglePause,
                    KeyCode::Char('/') => TuiAction::StartFilter,
                    KeyCode::Char('s') => TuiAction::SaveSnapshot,
                    _ => {
                        if app.filter_input_active {
                            match key.code {
                                KeyCode::Char(c) => TuiAction::FilterChar(c),
                                KeyCode::Backspace => TuiAction::FilterBackspace,
                                _ => continue,
                            }
                        } else {
                            continue;
                        }
                    }
                };

                if let TuiAction::Quit = action {
                    break;
                }
                app.handle_action(action);
            }
        }
    }

    Ok(())
}
