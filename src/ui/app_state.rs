use ratatui::widgets::ListState;

use super::line_assembler::LineAssembler;

const MAX_OUTPUT_LINES: usize = 1000;

pub struct AppState {
    pub input_line: String,
    pub output_lines: Vec<String>,
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
            output_lines: Vec::with_capacity(MAX_OUTPUT_LINES),
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
        self.output_lines.push(message);
        self.trim_output();
        self.needs_render = true;
    }

    fn trim_output(&mut self) {
        if self.output_lines.len() > MAX_OUTPUT_LINES {
            self.output_lines
                .drain(..self.output_lines.len() - MAX_OUTPUT_LINES);
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
        // Disable auto-scroll when manually scrolling
        self.auto_scroll = false;

        let selected = self.list_state.selected().unwrap_or(0);
        if selected < self.output_lines.len() - 1 {
            self.list_state.select(Some(selected + 1));
            self.needs_render = true;
        }
    }

    pub fn scroll_to_bottom(&mut self) {
        if !self.output_lines.is_empty() {
            // Disable auto-scroll when manually scrolling to bottom
            self.auto_scroll = false;
            self.list_state.select(Some(self.output_lines.len() - 1));
            self.needs_render = true;
        }
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
        self.auto_scroll = false;
        let current = self.list_state.selected().unwrap_or(0);
        let new_selected = (current + page_size).min(self.output_lines.len().saturating_sub(1));
        self.list_state.select(Some(new_selected));
        self.needs_render = true;
    }

    pub fn update_input(&mut self, c: char) {
        self.input_line.push(c);
        self.needs_render = true;
    }

    pub fn backspace_input(&mut self) {
        if self.input_line.pop().is_some() {
            self.needs_render = true;
        }
    }

    pub fn clear_input(&mut self) -> String {
        let input = std::mem::take(&mut self.input_line);
        if !input.is_empty() {
            self.needs_render = true;
        }
        input
    }

    pub fn quit(&mut self) {
        self.should_quit = true;
        self.needs_render = true;
    }

    pub fn mark_rendered(&mut self) {
        self.needs_render = false;
    }
}
