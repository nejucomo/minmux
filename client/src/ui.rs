use crate::app::AppState;
use crossterm::event::KeyEvent;
use minmux_ipc::{SessionInfo, WindowInfo};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

/// Draw the full application UI.
pub fn draw(f: &mut Frame, state: &AppState) {
    let area = f.area();

    // Split: content area + status bar (1 line).
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(area);

    let content_area = chunks[0];
    let status_area = chunks[1];

    // Draw content.
    if let Some(help) = &state.help_text {
        draw_help(f, content_area, help);
    } else if let Some(session) = &state.session {
        draw_session(f, content_area, session);
    } else {
        let loading = Paragraph::new("Connecting…")
            .style(Style::default().fg(Color::DarkGray));
        f.render_widget(loading, content_area);
    }

    // Draw status bar.
    draw_status_bar(f, status_area, state);
}

fn draw_help(f: &mut Frame, area: Rect, help: &str) {
    let para = Paragraph::new(help)
        .block(Block::default().borders(Borders::ALL).title("Help"))
        .style(Style::default().fg(Color::White));
    f.render_widget(para, area);
}

fn draw_session(f: &mut Frame, area: Rect, session: &SessionInfo) {
    // Find the active window.
    let active_window = session
        .windows
        .iter()
        .find(|w| w.id == session.active_window_id);

    if let Some(window) = active_window {
        draw_window(f, area, window);
    }
}

fn draw_window(f: &mut Frame, area: Rect, window: &WindowInfo) {
    let pane_count = window.panes.len().max(1);

    // Divide height equally among panes.
    let pane_height = area.height / pane_count as u16;

    // Build constraints: each pane gets `pane_height` rows, last pane gets the remainder.
    let constraints: Vec<Constraint> = (0..pane_count)
        .map(|i| {
            if i + 1 == pane_count {
                Constraint::Min(1)
            } else {
                Constraint::Length(pane_height)
            }
        })
        .collect();

    let pane_areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    for (i, pane) in window.panes.iter().enumerate() {
        let is_active = pane.id == window.active_pane_id;
        let border_style = if is_active {
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .title(pane.name.as_str())
            .border_style(border_style);

        let para = Paragraph::new(if is_active { "(active)" } else { "" })
            .block(block)
            .style(Style::default().fg(Color::White));

        f.render_widget(para, pane_areas[i]);
    }
}

fn draw_status_bar(f: &mut Frame, area: Rect, state: &AppState) {
    let mut spans: Vec<Span> = Vec::new();

    // Session name.
    if let Some(session) = &state.session {
        spans.push(Span::styled(
            format!(" {} ", session.name),
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::raw(" "));

        // Window list.
        for win in &session.windows {
            let is_active = win.id == session.active_window_id;
            let style = if is_active {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            spans.push(Span::styled(format!(" {} ", win.name), style));
        }

        spans.push(Span::raw(" "));
    }

    // Input mode.
    spans.push(Span::styled(
        format!("[{}]", state.input_mode),
        Style::default().fg(Color::Magenta),
    ));

    // Pending key sequence.
    if !state.pending_sequence.is_empty() {
        let seq: String = state
            .pending_sequence
            .iter()
            .map(key_event_display)
            .collect::<Vec<_>>()
            .join(" ");
        spans.push(Span::raw(format!(" {seq}")));
    }

    // Status / error message.
    if let Some(msg) = &state.status_message {
        spans.push(Span::styled(
            format!("  {msg}"),
            Style::default().fg(Color::Red),
        ));
    }

    let line = Line::from(spans);
    let para = Paragraph::new(line).style(Style::default().bg(Color::DarkGray));
    f.render_widget(para, area);
}

fn key_event_display(key: &KeyEvent) -> String {
    use crossterm::event::{KeyCode, KeyModifiers};
    if key.modifiers == KeyModifiers::CONTROL {
        if let KeyCode::Char(c) = key.code {
            return format!("^{}", c.to_ascii_uppercase());
        }
    }
    match key.code {
        KeyCode::Char(c) => c.to_string(),
        KeyCode::Esc => "<Esc>".to_string(),
        KeyCode::Enter => "<Enter>".to_string(),
        other => format!("{other:?}"),
    }
}
