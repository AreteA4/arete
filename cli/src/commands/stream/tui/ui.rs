use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
    Frame,
};

use super::app::{App, ViewMode};

pub fn draw(f: &mut Frame, app: &mut App) {
    app.ensure_filtered_cache();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),  // Header
            Constraint::Min(0),    // Main content
            Constraint::Length(1), // Timeline
            Constraint::Length(1), // Status bar
        ])
        .split(f.area());

    draw_header(f, app, chunks[0]);

    match app.view_mode {
        ViewMode::List => draw_split_view(f, app, chunks[1]),
        ViewMode::Detail => draw_detail_view(f, app, chunks[1]),
    }

    draw_timeline(f, app, chunks[2]);
    draw_status_bar(f, app, chunks[3]);
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let status = if app.disconnected {
        Span::styled(" DISCONNECTED ", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))
    } else if app.paused {
        Span::styled(" PAUSED ", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))
    } else {
        Span::styled(" LIVE ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD))
    };

    let dropped = app.dropped_frames.load(std::sync::atomic::Ordering::Relaxed);
    let mut spans = vec![
        Span::styled("hs stream ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::styled(&app.view, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        status,
        Span::raw("  "),
        Span::styled(
            format!("Updates: {}", app.update_count),
            Style::default().fg(Color::DarkGray),
        ),
    ];
    if dropped > 0 {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            format!("Dropped: {}", dropped),
            Style::default().fg(Color::Red),
        ));
    }
    let header = Line::from(spans);

    f.render_widget(Paragraph::new(header), area);
}

fn draw_split_view(f: &mut Frame, app: &mut App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(30), Constraint::Percentage(70)])
        .split(area);

    draw_entity_list(f, app, chunks[0]);
    draw_entity_detail(f, app, chunks[1]);
}

fn draw_entity_list(f: &mut Frame, app: &mut App, area: Rect) {
    let keys = app.filtered_keys();
    let items: Vec<ListItem> = keys
        .iter()
        .enumerate()
        .map(|(i, key)| {
            let style = if i == app.selected_index {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let prefix = if i == app.selected_index { "> " } else { "  " };
            ListItem::new(format!("{}{}", prefix, truncate_key(key, area.width as usize - 3)))
                .style(style)
        })
        .collect();

    let title = if app.filter_input_active {
        format!("Entities [/{}]", app.filter_text)
    } else if !app.filter_text.is_empty() {
        format!("Entities ({}/{}) [/{}]", keys.len(), app.entity_keys.len(), app.filter_text)
    } else {
        format!("Entities ({})", keys.len())
    };

    let list = List::new(items)
        .block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray)),
        )
        .highlight_style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD));

    f.render_stateful_widget(list, area, &mut app.list_state);
}

fn draw_entity_detail(f: &mut Frame, app: &App, area: Rect) {
    let content = app.selected_entity_data().unwrap_or_else(|| {
        if app.entity_keys.is_empty() {
            "Waiting for data...".to_string()
        } else {
            "Select an entity".to_string()
        }
    });

    let title = match app.selected_key() {
        Some(key) => {
            let mode = if app.show_diff {
                " [diff]"
            } else if app.history_position > 0 {
                " [history]"
            } else {
                ""
            };
            format!("{}{}", truncate_key(&key, area.width as usize - 10), mode)
        }
        None => "Detail".to_string(),
    };

    // Apply simple JSON syntax coloring
    let lines: Vec<Line> = content
        .lines()
        .skip(app.scroll_offset as usize)
        .map(|line| colorize_json_line(line))
        .collect();

    let detail = Paragraph::new(lines)
        .block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan)),
        )
        .wrap(Wrap { trim: false });

    f.render_widget(detail, area);
}

fn draw_detail_view(f: &mut Frame, app: &App, area: Rect) {
    draw_entity_detail(f, app, area);
}

fn draw_timeline(f: &mut Frame, app: &App, area: Rect) {
    let history_len = app.selected_history_len();
    let pos = app.history_position;
    let list_len = app.filtered_keys().len();
    let list_pos = if list_len > 0 { app.selected_index + 1 } else { 0 };

    let mut spans = vec![
        Span::styled(
            format!(" Row {}/{}", list_pos, list_len),
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(" │ ", Style::default().fg(Color::DarkGray)),
    ];

    if history_len == 0 {
        spans.push(Span::styled("Entity history: no data", Style::default().fg(Color::DarkGray)));
    } else {
        spans.push(Span::styled("[|<] ", Style::default().fg(if pos < history_len - 1 { Color::White } else { Color::DarkGray })));
        spans.push(Span::styled("[<] ", Style::default().fg(if pos < history_len - 1 { Color::White } else { Color::DarkGray })));
        spans.push(Span::styled(
            format!("version {}/{} ", history_len - pos, history_len),
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled("[>] ", Style::default().fg(if pos > 0 { Color::White } else { Color::DarkGray })));
        spans.push(Span::styled("[>|]", Style::default().fg(if pos > 0 { Color::White } else { Color::DarkGray })));
        spans.push(Span::raw("  "));
        spans.push(if app.show_diff {
            Span::styled("[d]iff ON", Style::default().fg(Color::Green))
        } else {
            Span::styled("[d]iff", Style::default().fg(Color::DarkGray))
        });
    }

    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_status_bar(f: &mut Frame, app: &App, area: Rect) {
    let status = Line::from(vec![
        Span::styled(
            format!(" {} ", app.status()),
            Style::default().fg(Color::DarkGray),
        ),
        Span::raw(" | "),
        Span::styled("q", Style::default().fg(Color::Yellow)),
        Span::styled("uit ", Style::default().fg(Color::DarkGray)),
        Span::styled("p", Style::default().fg(Color::Yellow)),
        Span::styled("ause ", Style::default().fg(Color::DarkGray)),
        Span::styled("d", Style::default().fg(Color::Yellow)),
        Span::styled("iff ", Style::default().fg(Color::DarkGray)),
        Span::styled("r", Style::default().fg(Color::Yellow)),
        Span::styled("aw ", Style::default().fg(Color::DarkGray)),
        Span::styled("/", Style::default().fg(Color::Yellow)),
        Span::styled("filter ", Style::default().fg(Color::DarkGray)),
        Span::styled("s", Style::default().fg(Color::Yellow)),
        Span::styled("ave ", Style::default().fg(Color::DarkGray)),
        Span::styled("h/l", Style::default().fg(Color::Yellow)),
        Span::styled(" history", Style::default().fg(Color::DarkGray)),
    ]);

    f.render_widget(Paragraph::new(status), area);
}

fn truncate_key(key: &str, max_len: usize) -> String {
    if key.chars().count() <= max_len {
        key.to_string()
    } else if max_len > 3 {
        let end = key
            .char_indices()
            .nth(max_len - 3)
            .map(|(i, _)| i)
            .unwrap_or(key.len());
        format!("{}...", &key[..end])
    } else {
        let end = key
            .char_indices()
            .nth(max_len)
            .map(|(i, _)| i)
            .unwrap_or(key.len());
        key[..end].to_string()
    }
}

/// Simple heuristic JSON syntax coloring for serde_json::to_string_pretty output.
/// Assumes keys are always properly quoted/escaped (guaranteed by serde_json).
fn colorize_json_line(line: &str) -> Line<'_> {
    let trimmed = line.trim();

    // Key-value lines
    if trimmed.starts_with('"') {
        // Use "\": " (with trailing space) to avoid matching colons inside key names.
        // serde_json pretty-print always uses ": " as the key-value separator.
        if let Some(colon_pos) = trimmed.find("\": ") {
            let key_end = colon_pos + 1;
            let indent = &line[..line.len() - trimmed.len()];
            let key = &trimmed[..key_end];
            let rest = &trimmed[key_end..];
            return Line::from(vec![
                Span::raw(indent),
                Span::styled(key, Style::default().fg(Color::Cyan)),
                colorize_value(rest),
            ]);
        }
    }

    // String values (in arrays)
    if (trimmed.starts_with('"') && trimmed.ends_with('"'))
        || (trimmed.starts_with('"') && trimmed.ends_with("\","))
    {
        return Line::from(Span::styled(line, Style::default().fg(Color::Green)));
    }

    // Braces and brackets
    if trimmed == "{" || trimmed == "}" || trimmed == "{}" || trimmed == "},"
        || trimmed == "[" || trimmed == "]" || trimmed == "[]" || trimmed == "],"
    {
        return Line::from(Span::styled(line, Style::default().fg(Color::DarkGray)));
    }

    Line::from(Span::raw(line))
}

fn colorize_value(rest: &str) -> Span<'_> {
    let trimmed = rest.trim().trim_end_matches(',');
    if trimmed.starts_with('"') {
        Span::styled(rest, Style::default().fg(Color::Green))
    } else if trimmed == "true" || trimmed == "false" {
        Span::styled(rest, Style::default().fg(Color::Yellow))
    } else if trimmed == "null" {
        Span::styled(rest, Style::default().fg(Color::DarkGray))
    } else if trimmed.parse::<f64>().is_ok() {
        Span::styled(rest, Style::default().fg(Color::Magenta))
    } else if trimmed.starts_with('[') || trimmed.starts_with("[]") {
        // Inline compact array — render in default color (contains mixed types)
        Span::styled(rest, Style::default().fg(Color::White))
    } else {
        Span::raw(rest)
    }
}
