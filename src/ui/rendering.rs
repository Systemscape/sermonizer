use super::app_state::{AppState, LineKind, OutputLine};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
};
use unicode_width::UnicodeWidthChar;

const TX_PREFIX: &str = "> ";

pub fn draw_ui(f: &mut Frame, app_state: &mut AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),    // Output area (takes most space)
            Constraint::Length(3), // Input area (fixed height)
            Constraint::Length(1), // Status bar
        ])
        .split(f.area());

    // Serial monitor output - optimize by avoiding allocations where possible
    let mut output_items: Vec<ListItem> = app_state
        .output_lines
        .iter()
        .map(|line| ListItem::new(output_line(line)))
        .collect();

    // Show the line still being received below the completed output
    if let Some(partial) = app_state.assembler.partial_display() {
        output_items.push(ListItem::new(partial));
    }

    let item_count = output_items.len();
    let output_list = List::new(output_items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Serial Monitor"),
        )
        .style(Style::default().fg(Color::White));

    app_state.view_height = chunks[0].height.saturating_sub(2) as usize;
    if app_state.auto_scroll {
        // Keep the selection pinned to the bottom so the list follows new
        // data; no highlight, the selection is not user-visible state here
        app_state
            .auto_scroll_state
            .select(item_count.checked_sub(1));
        f.render_stateful_widget(output_list, chunks[0], &mut app_state.auto_scroll_state);
        // Remember where the view starts so manual scrolling continues from it
        app_state.follow_top = app_state.auto_scroll_state.offset();
    } else {
        let mut state = ListState::default().with_offset(app_state.scroll_top);
        f.render_stateful_widget(output_list, chunks[0], &mut state);
    }

    // Input line: keep the cursor visible by scrolling horizontally once the
    // text is wider than the input area
    let inner_width = chunks[1].width.saturating_sub(2) as usize;
    let width_before_cursor: usize = app_state
        .input_line
        .chars()
        .take(app_state.input_cursor)
        .map(|c| c.width().unwrap_or(0))
        .sum();
    let h_scroll = width_before_cursor.saturating_sub(inner_width.saturating_sub(1));

    let input_paragraph = Paragraph::new(app_state.input_line.as_str())
        .scroll((0, h_scroll.try_into().unwrap_or(u16::MAX)))
        .block(Block::default().borders(Borders::ALL).title("Input"))
        .style(Style::default().fg(Color::Yellow));

    f.render_widget(input_paragraph, chunks[1]);

    // Set cursor position in input field
    f.set_cursor_position((
        chunks[1].x + 1 + (width_before_cursor - h_scroll) as u16,
        chunks[1].y + 1,
    ));

    f.render_widget(status_line(app_state), chunks[2]);
}

fn output_line(line: &OutputLine) -> Line<'_> {
    match line.kind {
        LineKind::Rx => Line::raw(line.text.as_str()),
        LineKind::Tx => Line::from(vec![
            Span::styled(TX_PREFIX, Style::default().fg(Color::Cyan)),
            Span::styled(line.text.as_str(), Style::default().fg(Color::Cyan)),
        ]),
        LineKind::Notice => Line::styled(
            line.text.as_str(),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::ITALIC),
        ),
    }
}

fn status_line(app_state: &AppState) -> Paragraph<'_> {
    let mut spans: Vec<Span> = Vec::new();

    // Segments are ordered by importance: the bar is clipped from the right
    // on narrow terminals, so key hints go last
    if app_state.connected {
        spans.push(Span::styled(
            format!(" {} ", app_state.port_label),
            Style::default().fg(Color::Black).bg(Color::Green),
        ));
    } else {
        spans.push(Span::styled(
            " DISCONNECTED - reconnecting... ",
            Style::default().fg(Color::White).bg(Color::Red),
        ));
    }

    if app_state.auto_scroll {
        spans.push(Span::raw(" follow"));
    } else {
        spans.push(Span::styled(
            format!(" scroll ({} new)", app_state.unseen_lines),
            Style::default().fg(Color::Yellow),
        ));
    }

    spans.push(Span::raw(format!(" | {}", app_state.line_ending_label)));

    if app_state.pending_literal {
        spans.push(Span::styled(
            " | Ctrl+V: next key is sent raw",
            Style::default().fg(Color::Magenta),
        ));
    } else {
        spans.push(Span::styled(
            " | Enter send, Up/Down history, Shift+Up/Down PgUp/PgDn scroll, Shift+End follow, Ctrl+L clear, Ctrl+V literal, Ctrl+C quit",
            Style::default().fg(Color::DarkGray),
        ));
    }

    Paragraph::new(Line::from(spans))
}
