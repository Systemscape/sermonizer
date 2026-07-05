use ratatui::widgets::ListState;
use std::collections::VecDeque;

use super::line_assembler::LineAssembler;

const MAX_OUTPUT_LINES: usize = 1000;

pub struct AppState {
    pub input_line: String,
    pub input_cursor: usize, // Cursor position as char index into input_line
    pub history: Vec<String>,
    pub history_pos: Option<usize>,
    pub draft: String,         // Unsent input stashed while browsing history
    pub pending_literal: bool, // Next key is sent as a raw control byte
    pub output_lines: VecDeque<String>,
    pub assembler: LineAssembler,
    pub list_state: ListState,
    pub auto_scroll_state: ListState,
    pub should_quit: bool,
    pub auto_scroll: bool,
    pub needs_render: bool, // Optimization: only render when needed
}

impl AppState {
    pub fn new(hex: bool, timestamps: bool) -> Self {
        Self {
            input_line: String::new(),
            input_cursor: 0,
            history: Vec::new(),
            history_pos: None,
            draft: String::new(),
            pending_literal: false,
            output_lines: VecDeque::with_capacity(MAX_OUTPUT_LINES),
            assembler: LineAssembler::new(hex, timestamps),
            list_state: ListState::default(),
            auto_scroll_state: ListState::default(),
            should_quit: false,
            auto_scroll: true,
            needs_render: true,
        }
    }

    pub fn add_data(&mut self, bytes: &[u8]) {
        self.output_lines.extend(self.assembler.push(bytes));
        self.trim_output();
        // The partial line is displayed too, so any data changes the view
        self.needs_render = true;
    }

    /// Push a complete status line (bypasses line assembly).
    pub fn add_notice(&mut self, message: String) {
        self.output_lines.push_back(message);
        self.trim_output();
        self.needs_render = true;
    }

    fn trim_output(&mut self) {
        let overflow = self.output_lines.len().saturating_sub(MAX_OUTPUT_LINES);
        if overflow == 0 {
            return;
        }
        self.output_lines.drain(..overflow);
        // Keep the manual scroll position anchored to the same line while
        // old lines are pruned from the front
        if let Some(selected) = self.list_state.selected() {
            self.list_state
                .select(Some(selected.saturating_sub(overflow)));
        }
    }

    pub fn scroll_up(&mut self) {
        if self.output_lines.is_empty() {
            return;
        }
        // Disable auto-scroll when manually scrolling
        self.auto_scroll = false;

        let selected = self
            .list_state
            .selected()
            .unwrap_or(self.output_lines.len() - 1);
        if selected > 0 {
            self.list_state.select(Some(selected - 1));
            self.needs_render = true;
        }
    }

    pub fn scroll_down(&mut self) {
        if self.output_lines.is_empty() {
            return;
        }

        let selected = self.list_state.selected().unwrap_or(0);
        if selected < self.output_lines.len() - 1 {
            self.auto_scroll = false;
            self.list_state.select(Some(selected + 1));
            self.needs_render = true;
        } else {
            // Scrolling past the last line resumes following new data
            self.enable_auto_scroll();
        }
    }

    pub fn scroll_to_bottom(&mut self) {
        self.enable_auto_scroll();
    }

    pub fn enable_auto_scroll(&mut self) {
        self.auto_scroll = true;
        self.list_state.select(None); // Clear selection when re-enabling auto-scroll
        self.needs_render = true;
    }

    pub fn scroll_to_home(&mut self) {
        if !self.output_lines.is_empty() {
            // Disable auto-scroll when manually scrolling to top
            self.auto_scroll = false;
            self.list_state.select(Some(0));
            self.needs_render = true;
        }
    }

    pub fn scroll_page_up(&mut self, page_size: usize) {
        if self.output_lines.is_empty() {
            return;
        }
        self.auto_scroll = false;
        let current = self
            .list_state
            .selected()
            .unwrap_or(self.output_lines.len().saturating_sub(1));
        let new_selected = current.saturating_sub(page_size);
        self.list_state.select(Some(new_selected));
        self.needs_render = true;
    }

    pub fn scroll_page_down(&mut self, page_size: usize) {
        if self.output_lines.is_empty() {
            return;
        }
        let current = self.list_state.selected().unwrap_or(0);
        let new_selected = (current + page_size).min(self.output_lines.len().saturating_sub(1));
        if new_selected == self.output_lines.len().saturating_sub(1) {
            self.enable_auto_scroll();
        } else {
            self.auto_scroll = false;
            self.list_state.select(Some(new_selected));
            self.needs_render = true;
        }
    }

    pub fn clear_output(&mut self) {
        self.output_lines.clear();
        self.assembler.clear();
        self.list_state.select(None);
        self.needs_render = true;
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
