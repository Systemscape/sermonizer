use ratatui::widgets::ListState;

pub struct AppState {
    pub input_line: String,
    pub output_lines: Vec<String>,
    pub partial_line: String,
    pub list_state: ListState,
    pub auto_scroll_state: ListState,
    pub should_quit: bool,
    pub auto_scroll: bool,
    pub needs_render: bool, // Optimization: only render when needed
}

impl AppState {
    pub fn new() -> Self {
        Self {
            input_line: String::new(),
            output_lines: Vec::with_capacity(1000), // Pre-allocate capacity
            partial_line: String::with_capacity(256), // Pre-allocate for partial lines
            list_state: ListState::default(),
            auto_scroll_state: ListState::default(),
            should_quit: false,
            auto_scroll: true,
            needs_render: true,
        }
    }

    pub fn add_output(&mut self, data: String) {
        // Append to partial line buffer
        self.partial_line.push_str(&data);

        // Check if we have complete lines (ending with \n or \r\n)
        let mut has_new_lines = false;
        while let Some(newline_pos) = self.partial_line.find('\n') {
            // Extract complete line (without the newline)
            let complete_line = self.partial_line[..newline_pos]
                .trim_end_matches('\r')
                .to_string();
            self.output_lines.push(complete_line);
            has_new_lines = true;

            // Remove processed part from partial_line
            self.partial_line.drain(..=newline_pos);
        }

        // Only trigger expensive operations if we have new complete lines
        if has_new_lines {
            // Keep only the last 1000 lines to prevent memory issues
            if self.output_lines.len() > 1000 {
                self.output_lines.drain(..self.output_lines.len() - 1000);
            }

            // Update auto-scroll state to point to the new bottom
            if !self.output_lines.is_empty() {
                self.auto_scroll_state
                    .select(Some(self.output_lines.len() - 1));
            }

            self.needs_render = true;
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
