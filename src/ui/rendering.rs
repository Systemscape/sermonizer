use super::app_state::{AppState, LineKind, OutputLine};
use ratatui::text::Text;
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
    // Lines are clipped at the border unless wrapping is on
    let wrap_width = app_state
        .wrap
        .then(|| chunks[0].width.saturating_sub(2) as usize);
    let mut output_items: Vec<ListItem> = app_state
        .output_lines
        .iter()
        .map(|line| ListItem::new(output_line(line, wrap_width)))
        .collect();

    // Show the line still being received below the completed output
    if let Some(partial) = app_state.assembler.partial_display() {
        let lines: Vec<Line> = wrap_text(&partial, wrap_width)
            .into_iter()
            .map(|s| Line::raw(s.to_string()))
            .collect();
        output_items.push(ListItem::new(Text::from(lines)));
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

fn output_line(line: &OutputLine, wrap_width: Option<usize>) -> Text<'_> {
    let (style, prefix) = match line.kind {
        LineKind::Rx => (Style::default(), ""),
        LineKind::Tx => (Style::default().fg(Color::Cyan), TX_PREFIX),
        LineKind::Notice => (
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::ITALIC),
            "",
        ),
    };
    // The prefix takes room on the first row only
    let first_width = wrap_width.map(|w| w.saturating_sub(prefix.len()).max(1));
    let mut rows = wrap_text(&line.text, first_width).into_iter();
    let mut lines: Vec<Line> = Vec::new();
    if let Some(first) = rows.next() {
        lines.push(Line::from(vec![
            Span::styled(prefix, style),
            Span::styled(first, style),
        ]));
    }
    if let Some(width) = wrap_width {
        let rest: String = rows.collect();
        if !rest.is_empty() {
            lines.extend(
                wrap_text(&rest, Some(width))
                    .into_iter()
                    .map(|s| Line::styled(s.to_string(), style)),
            );
        }
    }
    Text::from(lines)
}

/// Split text into rows no wider than `width` display columns. Without a
/// width the text is returned as a single row. Wide characters that would
/// straddle the edge start the next row.
fn wrap_text(text: &str, width: Option<usize>) -> Vec<&str> {
    let Some(width) = width.filter(|w| *w > 0) else {
        return vec![text];
    };
    let mut rows = Vec::new();
    let mut row_start = 0;
    let mut row_width = 0;
    for (idx, c) in text.char_indices() {
        let w = c.width().unwrap_or(0);
        if row_width + w > width && idx > row_start {
            rows.push(&text[row_start..idx]);
            row_start = idx;
            row_width = 0;
        }
        row_width += w;
    }
    rows.push(&text[row_start..]);
    rows
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
    if app_state.wrap {
        spans.push(Span::raw(" | wrap"));
    }

    if app_state.pending_literal {
        spans.push(Span::styled(
            " | Ctrl+V: next key is sent raw",
            Style::default().fg(Color::Magenta),
        ));
    } else {
        spans.push(Span::styled(
            " | Enter send, Up/Down history, Shift+Up/Down PgUp/PgDn scroll, Shift+End follow, Ctrl+T wrap, Ctrl+L clear, Ctrl+V literal, Ctrl+C quit",
            Style::default().fg(Color::DarkGray),
        ));
    }

    Paragraph::new(Line::from(spans))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_text_splits_at_display_width() {
        assert_eq!(wrap_text("abcdefgh", Some(3)), vec!["abc", "def", "gh"]);
        assert_eq!(wrap_text("abc", Some(3)), vec!["abc"]);
        assert_eq!(wrap_text("", Some(3)), vec![""]);
    }

    #[test]
    fn wrap_text_keeps_wide_characters_whole() {
        // Each CJK character is two columns wide
        assert_eq!(wrap_text("a日本", Some(3)), vec!["a日", "本"]);
    }

    #[test]
    fn wrap_text_without_width_returns_one_row() {
        assert_eq!(wrap_text("anything at all", None), vec!["anything at all"]);
        assert_eq!(wrap_text("x", Some(0)), vec!["x"]);
    }

    fn demo_state() -> AppState {
        let mut state = AppState::new(false, false, true, "ttyUSB0 115200 8N1".to_string(), "LF");
        state.add_rx("[2026-09-10 09:41:02.118] I (312) boot: ESP-IDF v5.2".to_string());
        state.add_rx("[2026-09-10 09:41:02.121] I (318) wifi: connecting to lab-iot".to_string());
        state.add_rx("[2026-09-10 09:41:03.877] I (2074) wifi: got ip 192.168.4.23".to_string());
        state.add_tx("[2026-09-10 09:41:07.402] AT+GMR".to_string());
        state.add_rx("[2026-09-10 09:41:07.410] AT version:2.4.0.0".to_string());
        state.add_rx("[2026-09-10 09:41:07.411] OK".to_string());
        state.add_notice(
            "[sermonizer] device disconnected: Broken pipe - reconnecting (Ctrl+C to quit)"
                .to_string(),
        );
        state.add_notice("[sermonizer] device reconnected".to_string());
        state.add_rx("[2026-09-10 09:41:12.006] I (309) boot: ESP-IDF v5.2".to_string());
        state.add_data(b"[2026-09-10 09:41:12.009] I (315) main: sensor=23.4C hum=41%");
        for c in "AT+CWJAP=\"lab-iot\",\"".chars() {
            state.update_input(c);
        }
        state
    }

    fn render(width: u16, height: u16) -> String {
        use ratatui::{Terminal, backend::TestBackend};
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("test terminal");
        let mut state = demo_state();
        terminal.draw(|f| draw_ui(f, &mut state)).expect("draw");
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn renders_without_panicking_on_tiny_terminals() {
        for (w, h) in [(1, 1), (10, 3), (20, 5), (40, 6), (80, 24)] {
            let screen = render(w, h);
            assert_eq!(screen.lines().count(), usize::from(h), "{w}x{h}");
        }
    }

    #[test]
    fn output_shows_rx_tx_notices_and_the_partial_line() {
        let screen = render(100, 18);
        assert!(
            screen.contains("> [2026-09-10 09:41:07.402] AT+GMR"),
            "{screen}"
        );
        assert!(screen.contains("device reconnected"), "{screen}");
        assert!(screen.contains("sensor=23.4C hum=41%"), "{screen}");
        assert!(
            screen.contains(" ttyUSB0 115200 8N1  follow | LF"),
            "{screen}"
        );
    }

    /// Prints the README screenshot: cargo test readme_screenshot -- --ignored --nocapture
    #[test]
    #[ignore = "prints the README screenshot on demand"]
    fn readme_screenshot() {
        println!("{}", render(96, 18));
    }

    #[test]
    fn wrapped_short_lines_take_a_single_row() {
        for kind in [LineKind::Rx, LineKind::Tx, LineKind::Notice] {
            let line = OutputLine {
                kind,
                text: "short".to_string(),
            };
            assert_eq!(output_line(&line, Some(40)).lines.len(), 1, "{kind:?}");
        }
    }

    #[test]
    fn tx_prefix_takes_room_on_the_first_row_only() {
        let line = OutputLine {
            kind: LineKind::Tx,
            text: "abcdef".to_string(),
        };
        let text = output_line(&line, Some(4));
        let rows: Vec<String> = text.lines.iter().map(ToString::to_string).collect();
        assert_eq!(rows, vec!["> ab", "cdef"]);
    }
}
