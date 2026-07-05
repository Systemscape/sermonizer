use super::app_state::AppState;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    widgets::{Block, Borders, List, ListItem, Paragraph},
};
use unicode_width::UnicodeWidthChar;

pub fn draw_ui(f: &mut Frame, app_state: &mut AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),    // Output area (takes most space)
            Constraint::Length(3), // Input area (fixed height)
        ])
        .split(f.area());

    // Serial monitor output - optimize by avoiding allocations where possible
    let mut output_items: Vec<ListItem> = app_state
        .output_lines
        .iter()
        .map(|line| ListItem::new(line.as_str()))
        .collect();

    // Show the line still being received below the completed output
    if let Some(partial) = app_state.assembler.partial_display() {
        output_items.push(ListItem::new(partial));
    }

    let title = if app_state.auto_scroll {
        "Serial Monitor (Auto-scroll ON - ↑↓/PgUp/PgDn to scroll, Ctrl+A to re-enable auto-scroll)"
    } else {
        "Serial Monitor (Auto-scroll OFF - ↑↓/PgUp/PgDn to scroll, Ctrl+A to re-enable auto-scroll)"
    };

    let item_count = output_items.len();
    let output_list = List::new(output_items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .style(Style::default().fg(Color::White))
        .highlight_style(Style::default().fg(Color::Black).bg(Color::White));

    // Handle auto-scrolling vs manual scrolling
    if app_state.auto_scroll {
        // Keep the selection pinned to the bottom so the list follows new data
        app_state
            .auto_scroll_state
            .select(item_count.checked_sub(1));
        f.render_stateful_widget(output_list, chunks[0], &mut app_state.auto_scroll_state);
    } else {
        // Manual scrolling mode - use the user's scroll position
        f.render_stateful_widget(output_list, chunks[0], &mut app_state.list_state);
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
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Input (Press Enter to send, Ctrl+C or Esc to exit)"),
        )
        .style(Style::default().fg(Color::Yellow));

    f.render_widget(input_paragraph, chunks[1]);

    // Set cursor position in input field
    f.set_cursor_position((
        chunks[1].x + 1 + (width_before_cursor - h_scroll) as u16,
        chunks[1].y + 1,
    ));
}
