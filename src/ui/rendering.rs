use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    widgets::{Block, Borders, List, ListItem, Paragraph},
};
use super::app_state::AppState;

pub fn draw_ui(f: &mut Frame, app_state: &mut AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),    // Output area (takes most space)
            Constraint::Length(3), // Input area (fixed height)
        ])
        .split(f.area());

    // Serial monitor output - optimize by avoiding allocations where possible
    let output_items: Vec<ListItem> = app_state
        .output_lines
        .iter()
        .map(|line| ListItem::new(line.as_str()))
        .collect();

    let title = if app_state.auto_scroll {
        "Serial Monitor (Auto-scroll ON - ↑↓/PgUp/PgDn to scroll, Ctrl+A to re-enable auto-scroll)"
    } else {
        "Serial Monitor (Auto-scroll OFF - ↑↓/PgUp/PgDn to scroll, Ctrl+A to re-enable auto-scroll)"
    };

    let output_list = List::new(output_items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .style(Style::default().fg(Color::White))
        .highlight_style(Style::default().fg(Color::Black).bg(Color::White));

    // Handle auto-scrolling vs manual scrolling
    if app_state.auto_scroll {
        // Use the persistent auto-scroll state that stays positioned at bottom
        f.render_stateful_widget(output_list, chunks[0], &mut app_state.auto_scroll_state);
    } else {
        // Manual scrolling mode - use the user's scroll position
        f.render_stateful_widget(output_list, chunks[0], &mut app_state.list_state);
    }

    // Input line
    let input_paragraph = Paragraph::new(app_state.input_line.as_str())
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Input (Press Enter to send, Ctrl+C or Esc to exit)"),
        )
        .style(Style::default().fg(Color::Yellow));

    f.render_widget(input_paragraph, chunks[1]);

    // Set cursor position in input field
    f.set_cursor_position((
        chunks[1].x + app_state.input_line.len() as u16 + 1,
        chunks[1].y + 1,
    ));
}