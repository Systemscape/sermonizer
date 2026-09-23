use ratatui::widgets::ListState;
use std::collections::VecDeque;

use super::line_assembler::{LineAssembler, timestamp};

const MAX_OUTPUT_LINES: usize = 1000;

/// Origin of an output line, used to style it
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Rx,
    Tx,
    Notice,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputLine {
    pub kind: LineKind,
    pub text: String,
}

pub struct AppState {
    pub input_line: String,
    pub input_cursor: usize, // Cursor position as char index into input_line
    pub history: Vec<String>,
    pub history_pos: Option<usize>,
    pub draft: String,         // Unsent input stashed while browsing history
    pub pending_literal: bool, // Next key is sent as a raw control byte
    pub output_lines: VecDeque<OutputLine>,
    pub assembler: LineAssembler,
    pub auto_scroll_state: ListState,
    pub scroll_top: usize,  // First visible row while scrolled manually
    pub follow_top: usize,  // First visible row of the last frame while following
    pub view_height: usize, // Rows available to the output list in the last frame
    pub should_quit: bool,
    pub auto_scroll: bool,
    pub connected: bool,
    pub unseen_lines: usize, // Lines received while not following the output
    pub show_ts: bool,
    pub wrap: bool, // Wrap long output lines instead of clipping them
    pub port_label: String,
    pub line_ending_label: &'static str,
    pub needs_render: bool, // Optimization: only render when needed
}

impl AppState {
    pub fn new(
        hex: bool,
        timestamps: bool,
        strip_ansi: bool,
        port_label: String,
        line_ending_label: &'static str,
    ) -> Self {
        Self {
            input_line: String::new(),
            input_cursor: 0,
            history: Vec::new(),
            history_pos: None,
            draft: String::new(),
            pending_literal: false,
            output_lines: VecDeque::with_capacity(MAX_OUTPUT_LINES),
            assembler: LineAssembler::new(hex, timestamps, strip_ansi),
            auto_scroll_state: ListState::default(),
            scroll_top: 0,
            follow_top: 0,
            view_height: 0,
            should_quit: false,
            auto_scroll: true,
            connected: true,
            unseen_lines: 0,
            show_ts: timestamps,
            wrap: false,
            port_label,
            line_ending_label,
            needs_render: true,
        }
    }

    pub fn add_data(&mut self, bytes: &[u8]) {
        let completed = self.assembler.push(bytes);
        if !self.auto_scroll {
            self.unseen_lines += completed.len();
        }
        self.output_lines
            .extend(completed.into_iter().map(|text| OutputLine {
                kind: LineKind::Rx,
                text,
            }));
        self.trim_output();
        // The partial line is displayed too, so any data changes the view
        self.needs_render = true;
    }

    /// Push a complete status line (bypasses line assembly).
    pub fn add_notice(&mut self, message: String) {
        self.push_line(LineKind::Notice, message);
    }

    /// Push a complete received line that bypassed line assembly.
    pub fn add_rx(&mut self, text: String) {
        self.push_line(LineKind::Rx, text);
    }

    /// Push a line describing data that was just transmitted.
    pub fn add_tx(&mut self, text: String) {
        let text = if self.show_ts {
            format!("{}{text}", timestamp())
        } else {
            text
        };
        self.push_line(LineKind::Tx, text);
    }

    fn push_line(&mut self, kind: LineKind, text: String) {
        if !self.auto_scroll {
            self.unseen_lines += 1;
        }
        self.output_lines.push_back(OutputLine { kind, text });
        self.trim_output();
        self.needs_render = true;
    }

    pub fn set_connected(&mut self, connected: bool) {
        self.connected = connected;
        self.needs_render = true;
    }

    fn trim_output(&mut self) {
        let overflow = self.output_lines.len().saturating_sub(MAX_OUTPUT_LINES);
        if overflow == 0 {
            return;
        }
        self.output_lines.drain(..overflow);
        // Keep the scroll window anchored to the same lines while old lines
        // are pruned from the front
        self.scroll_top = self.scroll_top.saturating_sub(overflow);
        self.follow_top = self.follow_top.saturating_sub(overflow);
    }

    /// Rows the output list can show; a sane page before the first frame
    fn page_size(&self) -> usize {
        self.view_height.max(1)
    }

    fn total_rows(&self) -> usize {
        self.output_lines.len() + usize::from(self.assembler.has_partial())
    }

    /// Highest top row at which the newest line is still visible
    fn max_top(&self) -> usize {
        self.total_rows().saturating_sub(self.page_size())
    }

    /// Top row of the current view, whether following or scrolled
    fn current_top(&self) -> usize {
        if self.auto_scroll {
            self.follow_top.min(self.max_top())
        } else {
            self.scroll_top
        }
    }

    /// Scroll so that `top` is the first visible row; reaching the newest
    /// line resumes following new data
    fn set_top(&mut self, top: usize) {
        if top >= self.max_top() {
            self.enable_auto_scroll();
            return;
        }
        self.auto_scroll = false;
        self.scroll_top = top;
        self.needs_render = true;
    }

    pub fn scroll_up_by(&mut self, rows: usize) {
        self.set_top(self.current_top().saturating_sub(rows));
    }

    pub fn scroll_down_by(&mut self, rows: usize) {
        self.set_top(self.current_top().saturating_add(rows));
    }

    pub fn scroll_up(&mut self) {
        self.scroll_up_by(1);
    }

    pub fn scroll_down(&mut self) {
        self.scroll_down_by(1);
    }

    pub fn scroll_page_up(&mut self) {
        self.scroll_up_by(self.page_size());
    }

    pub fn scroll_page_down(&mut self) {
        self.scroll_down_by(self.page_size());
    }

    pub fn scroll_to_home(&mut self) {
        self.set_top(0);
    }

    pub fn scroll_to_bottom(&mut self) {
        self.enable_auto_scroll();
    }

    pub fn enable_auto_scroll(&mut self) {
        self.auto_scroll = true;
        self.unseen_lines = 0;
        self.needs_render = true;
    }

    pub fn toggle_wrap(&mut self) {
        self.wrap = !self.wrap;
        self.needs_render = true;
    }

    pub fn clear_output(&mut self) {
        self.output_lines.clear();
        self.assembler.clear();
        self.scroll_top = 0;
        self.follow_top = 0;
        self.enable_auto_scroll();
    }

    pub fn push_history(&mut self, line: String) {
        if !line.is_empty() && self.history.last() != Some(&line) {
            self.history.push(line);
        }
        self.history_pos = None;
    }

    pub fn history_prev(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let pos = match self.history_pos {
            None => {
                self.draft = std::mem::take(&mut self.input_line);
                self.history.len() - 1
            }
            Some(p) => p.saturating_sub(1),
        };
        self.history_pos = Some(pos);
        self.set_input(self.history[pos].clone());
    }

    pub fn history_next(&mut self) {
        let Some(pos) = self.history_pos else {
            return;
        };
        if pos + 1 < self.history.len() {
            self.history_pos = Some(pos + 1);
            self.set_input(self.history[pos + 1].clone());
        } else {
            self.history_pos = None;
            let draft = std::mem::take(&mut self.draft);
            self.set_input(draft);
        }
    }

    fn set_input(&mut self, text: String) {
        self.input_cursor = text.chars().count();
        self.input_line = text;
        self.needs_render = true;
    }

    pub fn update_input(&mut self, c: char) {
        let byte_idx = self.input_byte_index(self.input_cursor);
        self.input_line.insert(byte_idx, c);
        self.input_cursor += 1;
        self.needs_render = true;
    }

    pub fn backspace_input(&mut self) {
        if self.input_cursor == 0 {
            return;
        }
        self.input_cursor -= 1;
        let byte_idx = self.input_byte_index(self.input_cursor);
        self.input_line.remove(byte_idx);
        self.needs_render = true;
    }

    pub fn delete_input(&mut self) {
        let byte_idx = self.input_byte_index(self.input_cursor);
        if byte_idx < self.input_line.len() {
            self.input_line.remove(byte_idx);
            self.needs_render = true;
        }
    }

    pub fn move_cursor_left(&mut self) {
        if self.input_cursor > 0 {
            self.input_cursor -= 1;
            self.needs_render = true;
        }
    }

    pub fn move_cursor_right(&mut self) {
        if self.input_cursor < self.input_line.chars().count() {
            self.input_cursor += 1;
            self.needs_render = true;
        }
    }

    pub fn input_home(&mut self) {
        if self.input_cursor != 0 {
            self.input_cursor = 0;
            self.needs_render = true;
        }
    }

    pub fn input_end(&mut self) {
        let end = self.input_line.chars().count();
        if self.input_cursor != end {
            self.input_cursor = end;
            self.needs_render = true;
        }
    }

    pub fn kill_to_start(&mut self) {
        let byte_idx = self.input_byte_index(self.input_cursor);
        if byte_idx > 0 {
            self.input_line.drain(..byte_idx);
            self.input_cursor = 0;
            self.needs_render = true;
        }
    }

    pub fn kill_to_end(&mut self) {
        let byte_idx = self.input_byte_index(self.input_cursor);
        if byte_idx < self.input_line.len() {
            self.input_line.truncate(byte_idx);
            self.needs_render = true;
        }
    }

    /// Delete back to the start of the previous word, like readline's Ctrl+W
    pub fn delete_word_back(&mut self) {
        let end = self.input_byte_index(self.input_cursor);
        let head = &self.input_line[..end];
        let trimmed = head.trim_end_matches(char::is_whitespace);
        let start = trimmed
            .char_indices()
            .rev()
            .find(|(_, c)| c.is_whitespace())
            .map_or(0, |(i, c)| i + c.len_utf8());
        if start < end {
            let removed = self.input_line[start..end].chars().count();
            self.input_line.drain(start..end);
            self.input_cursor -= removed;
            self.needs_render = true;
        }
    }

    pub fn clear_input(&mut self) -> String {
        self.input_cursor = 0;
        let input = std::mem::take(&mut self.input_line);
        if !input.is_empty() {
            self.needs_render = true;
        }
        input
    }

    fn input_byte_index(&self, char_idx: usize) -> usize {
        self.input_line
            .char_indices()
            .nth(char_idx)
            .map(|(i, _)| i)
            .unwrap_or(self.input_line.len())
    }

    pub fn quit(&mut self) {
        self.should_quit = true;
        self.needs_render = true;
    }

    pub fn mark_rendered(&mut self) {
        self.needs_render = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state_with_lines(n: usize) -> AppState {
        let mut state = AppState::new(false, false, true, String::new(), "LF");
        for i in 0..n {
            state.add_notice(format!("line {i}"));
        }
        state
    }

    /// A state as it looks after one frame was drawn while following
    fn rendered_state(lines: usize, view_height: usize) -> AppState {
        let mut state = state_with_lines(lines);
        state.view_height = view_height;
        state.follow_top = lines.saturating_sub(view_height);
        state
    }

    #[test]
    fn scroll_down_and_page_down_while_following_keep_following() {
        let mut state = rendered_state(50, 18);
        state.scroll_down();
        assert!(state.auto_scroll);
        state.scroll_page_down();
        assert!(state.auto_scroll);
    }

    #[test]
    fn scroll_up_moves_the_view_one_row_and_down_resumes_following() {
        let mut state = rendered_state(50, 18);
        state.scroll_up();
        assert!(!state.auto_scroll);
        assert_eq!(state.scroll_top, 31);
        state.scroll_down();
        assert!(state.auto_scroll);
    }

    #[test]
    fn pages_move_by_the_visible_height() {
        let mut state = rendered_state(50, 18);
        state.scroll_page_up();
        assert_eq!(state.scroll_top, 14);
        state.scroll_page_up();
        assert_eq!(state.scroll_top, 0);
        state.scroll_page_down();
        assert_eq!(state.scroll_top, 18);
        state.scroll_page_down();
        assert!(state.auto_scroll, "past the newest line resumes following");
    }

    #[test]
    fn home_jumps_to_the_top_and_bottom_resumes_following() {
        let mut state = rendered_state(50, 18);
        state.scroll_to_home();
        assert!(!state.auto_scroll);
        assert_eq!(state.scroll_top, 0);
        state.scroll_to_bottom();
        assert!(state.auto_scroll);
    }

    #[test]
    fn scrolling_is_a_noop_when_everything_fits() {
        let mut state = rendered_state(5, 18);
        state.scroll_up();
        state.scroll_page_up();
        state.scroll_to_home();
        assert!(state.auto_scroll);
    }

    #[test]
    fn partial_row_counts_toward_the_scroll_range() {
        let mut state = rendered_state(50, 18);
        state.add_data(b"partial");
        state.auto_scroll = false;
        state.scroll_top = 31;
        state.scroll_down();
        assert!(!state.auto_scroll, "row 32 still hides the partial line");
        state.scroll_down();
        assert!(state.auto_scroll);
    }

    #[test]
    fn trimming_keeps_manual_scroll_window_anchored() {
        let mut state = rendered_state(MAX_OUTPUT_LINES, 18);
        state.scroll_up();
        state.scroll_top = 500;
        for _ in 0..10 {
            state.add_notice("new".to_string());
        }
        assert_eq!(state.scroll_top, 490);
    }

    #[test]
    fn clearing_output_resumes_following() {
        let mut state = rendered_state(50, 18);
        state.scroll_to_home();
        state.clear_output();
        assert!(state.auto_scroll);
        assert!(state.output_lines.is_empty());
    }

    fn state_with_input(text: &str) -> AppState {
        let mut state = AppState::new(false, false, true, String::new(), "LF");
        for c in text.chars() {
            state.update_input(c);
        }
        state
    }

    #[test]
    fn home_and_end_move_the_input_cursor() {
        let mut state = state_with_input("grün ok");
        state.input_home();
        assert_eq!(state.input_cursor, 0);
        state.update_input('>');
        assert_eq!(state.input_line, ">grün ok");
        state.input_end();
        state.update_input('<');
        assert_eq!(state.input_line, ">grün ok<");
    }

    #[test]
    fn kill_to_start_and_end_split_at_the_cursor() {
        let mut state = state_with_input("abcdef");
        state.move_cursor_left();
        state.move_cursor_left();
        state.kill_to_end();
        assert_eq!(state.input_line, "abcd");
        assert_eq!(state.input_cursor, 4);
        state.move_cursor_left();
        state.kill_to_start();
        assert_eq!(state.input_line, "d");
        assert_eq!(state.input_cursor, 0);
    }

    #[test]
    fn delete_word_back_removes_trailing_spaces_and_one_word() {
        let mut state = state_with_input("AT+CWJAP  ssid   ");
        state.delete_word_back();
        assert_eq!(state.input_line, "AT+CWJAP  ");
        state.delete_word_back();
        assert_eq!(state.input_line, "");
        state.delete_word_back();
        assert_eq!(state.input_line, "");
    }
}
