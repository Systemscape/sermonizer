pub mod app_state;
pub mod line_assembler;
pub mod rendering;

pub use app_state::AppState;
pub use rendering::draw_ui;

use anyhow::Result;
use ratatui::crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEventKind,
};
use ratatui::{Terminal, backend::Backend};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::mpsc;

use crate::config::UiConfig;
use crate::serial_io::{SerialEvent, WriterMsg};

const MOUSE_SCROLL_ROWS: usize = 3;

#[derive(Debug)]
pub enum UiMessage {
    Quit,
}

pub async fn run_ui<B: Backend>(
    terminal: &mut Terminal<B>,
    mut ui_rx: mpsc::UnboundedReceiver<UiMessage>,
    mut serial_rx: mpsc::UnboundedReceiver<SerialEvent>,
    ui_config: UiConfig,
) -> Result<()> {
    let mut app_state = AppState::new(
        ui_config.hex,
        ui_config.show_ts,
        !ui_config.raw,
        ui_config.port_label.clone(),
        ui_config.line_ending.describe(),
    );
    app_state.wrap = ui_config.wrap;
    let (mut input_rx, input_handle) = spawn_input_thread(ui_config.running.clone());

    // Run the loop in a block so the input thread is stopped and joined on
    // every exit path, including a failed draw
    let result: Result<()> = async {
        loop {
            // Only render if state changed - major performance optimization
            if app_state.needs_render {
                terminal.draw(|f| draw_ui(f, &mut app_state))?;
                app_state.mark_rendered();
            }

            if !ui_config.running.load(Ordering::SeqCst) || app_state.should_quit {
                break;
            }

            tokio::select! {
                // UI messages (like quit from Ctrl-C)
                msg = ui_rx.recv() => {
                    match msg {
                        Some(UiMessage::Quit) | None => app_state.quit(),
                    }
                }

                // Serial events
                event = serial_rx.recv() => {
                    match event {
                        Some(event) => handle_serial_event(event, &mut app_state),
                        None => app_state.quit(),
                    }
                }

                // Terminal events from the blocking input thread
                input = input_rx.recv() => {
                    match input {
                        Some(ev) => handle_input_event(ev, &mut app_state, &ui_config),
                        None => app_state.quit(),
                    }
                }
            }

            // Fold everything already queued into the same frame: a fast serial
            // stream arrives in many small reads and must not cost a draw each
            while let Ok(event) = serial_rx.try_recv() {
                handle_serial_event(event, &mut app_state);
            }
            while let Ok(ev) = input_rx.try_recv() {
                handle_input_event(ev, &mut app_state, &ui_config);
            }
        }
        Ok(())
    }
    .await;

    ui_config.running.store(false, Ordering::SeqCst);
    let _ = input_handle.join();
    result
}

/// Reads terminal events on a dedicated thread so the UI loop can await them
/// instead of busy-polling.
fn spawn_input_thread(
    running: Arc<AtomicBool>,
) -> (mpsc::UnboundedReceiver<Event>, std::thread::JoinHandle<()>) {
    let (tx, rx) = mpsc::unbounded_channel();
    let handle = std::thread::spawn(move || {
        while running.load(Ordering::SeqCst) {
            if event::poll(Duration::from_millis(100)).unwrap_or(false)
                && let Ok(ev) = event::read()
                && tx.send(ev).is_err()
            {
                break;
            }
        }
    });
    (rx, handle)
}

fn handle_input_event(event: Event, app_state: &mut AppState, ui_config: &UiConfig) {
    match event {
        Event::Key(k) if k.kind == KeyEventKind::Press => {
            handle_key_event(k, app_state, ui_config);
        }
        Event::Paste(text) => handle_paste(&text, app_state, ui_config),
        // Only delivered when --mouse enabled capture
        Event::Mouse(m) => match m.kind {
            MouseEventKind::ScrollUp => app_state.scroll_up_by(MOUSE_SCROLL_ROWS),
            MouseEventKind::ScrollDown => app_state.scroll_down_by(MOUSE_SCROLL_ROWS),
            _ => {}
        },
        Event::Resize(_, _) => app_state.needs_render = true,
        _ => {}
    }
}

/// Pasted text is sent line by line; an unterminated last line stays in the
/// input box so the user can finish it.
fn handle_paste(text: &str, app_state: &mut AppState, ui_config: &UiConfig) {
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' | '\n' => {
                if c == '\r' {
                    chars.next_if_eq(&'\n');
                }
                // Stop at the first line that cannot be sent so the paste
                // does not run together into one input line
                if !handle_enter_key(app_state, ui_config) {
                    break;
                }
            }
            c if c.is_control() => {}
            c => app_state.update_input(c),
        }
    }
}

fn handle_serial_event(event: SerialEvent, app_state: &mut AppState) {
    match event {
        SerialEvent::Data(bytes) => {
            app_state.add_data(&bytes);
        }
        SerialEvent::Error(message) => {
            app_state.add_notice(format!("[sermonizer] {message}"));
        }
        SerialEvent::Disconnected(reason) => {
            if let Some(line) = app_state.assembler.finish() {
                app_state.add_rx(line);
            }
            app_state.set_connected(false);
            app_state.add_notice(format!(
                "[sermonizer] device disconnected: {reason} - reconnecting (Ctrl+C to quit)"
            ));
        }
        SerialEvent::Reconnected => {
            app_state.set_connected(true);
            app_state.add_notice("[sermonizer] device reconnected".to_string());
        }
    }
}

fn handle_key_event(key: KeyEvent, app_state: &mut AppState, ui_config: &UiConfig) {
    // Ctrl+V arms literal mode: the next key is sent as a raw control byte
    if app_state.pending_literal {
        app_state.pending_literal = false;
        app_state.needs_render = true;
        if !app_state.connected {
            app_state.add_notice("[sermonizer] not connected, nothing sent".to_string());
            return;
        }
        let Some(byte) = literal_byte(key) else {
            app_state.add_notice(
                "[sermonizer] no literal byte for that key, nothing sent (use Ctrl+A..Z, Esc, Enter or Tab)"
                    .to_string(),
            );
            return;
        };
        if ui_config.writer.send(WriterMsg::Data(vec![byte])).is_err() {
            app_state.add_notice("[sermonizer] writer stopped, input dropped".to_string());
        } else {
            app_state.add_tx(format!("<0x{byte:02X}>"));
        }
        return;
    }

    match key.code {
        KeyCode::Char(c)
            if key.modifiers.contains(KeyModifiers::CONTROL) && (c == 'c' || c == 'd') =>
        {
            app_state.quit();
        }
        KeyCode::Char('l') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app_state.clear_output();
        }
        // A pasted LF arrives as Ctrl+J in raw mode
        KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let _ = handle_enter_key(app_state, ui_config);
        }
        KeyCode::Char('v') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app_state.pending_literal = true;
            app_state.needs_render = true;
        }
        KeyCode::Char('t') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app_state.toggle_wrap();
        }
        KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app_state.input_home();
        }
        KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app_state.input_end();
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app_state.kill_to_start();
        }
        KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app_state.kill_to_end();
        }
        KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app_state.delete_word_back();
        }
        KeyCode::Esc => {
            app_state.clear_input();
        }
        KeyCode::Char(c)
            if !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
        {
            app_state.update_input(c);
        }
        KeyCode::Enter => {
            let _ = handle_enter_key(app_state, ui_config);
        }
        KeyCode::Backspace => {
            app_state.backspace_input();
        }
        KeyCode::Delete => {
            app_state.delete_input();
        }
        KeyCode::Left => {
            app_state.move_cursor_left();
        }
        KeyCode::Right => {
            app_state.move_cursor_right();
        }
        KeyCode::Up if key.modifiers.contains(KeyModifiers::SHIFT) => {
            app_state.scroll_up();
        }
        KeyCode::Down if key.modifiers.contains(KeyModifiers::SHIFT) => {
            app_state.scroll_down();
        }
        KeyCode::Up => {
            app_state.history_prev();
        }
        KeyCode::Down => {
            app_state.history_next();
        }
        KeyCode::PageUp => {
            app_state.scroll_page_up();
        }
        KeyCode::PageDown => {
            app_state.scroll_page_down();
        }
        KeyCode::Home
            if key
                .modifiers
                .intersects(KeyModifiers::SHIFT | KeyModifiers::CONTROL) =>
        {
            app_state.scroll_to_home();
        }
        KeyCode::End
            if key
                .modifiers
                .intersects(KeyModifiers::SHIFT | KeyModifiers::CONTROL) =>
        {
            app_state.scroll_to_bottom();
        }
        KeyCode::Home => {
            app_state.input_home();
        }
        KeyCode::End => {
            app_state.input_end();
        }
        _ => {}
    }
}

/// Map a key pressed after Ctrl+V to the raw byte it should send.
fn literal_byte(key: KeyEvent) -> Option<u8> {
    match key.code {
        KeyCode::Char(c) if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let c = c.to_ascii_uppercase();
            // Ctrl+A..Ctrl+Z map to 0x01..0x1A
            c.is_ascii_uppercase().then(|| c as u8 - b'A' + 1)
        }
        KeyCode::Esc => Some(0x1B),
        KeyCode::Enter => Some(b'\r'),
        KeyCode::Tab => Some(b'\t'),
        _ => None,
    }
}

/// Returns whether the line was handed to the writer.
fn handle_enter_key(app_state: &mut AppState, ui_config: &UiConfig) -> bool {
    if !app_state.connected {
        app_state.add_notice(
            "[sermonizer] not connected, input kept: press Enter again once the device is back"
                .to_string(),
        );
        return false;
    }
    let input = app_state.clear_input();
    app_state.push_history(input.clone());

    // Send input and line ending as a single write
    let mut bytes = input.clone().into_bytes();
    bytes.extend_from_slice(ui_config.line_ending.bytes());
    if bytes.is_empty() {
        return true;
    }

    if ui_config.writer.send(WriterMsg::Data(bytes)).is_err() {
        app_state.add_notice("[sermonizer] writer stopped, input dropped".to_string());
        return false;
    }
    if ui_config.echo {
        app_state.add_tx(input);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::LineEnding;
    use app_state::LineKind;

    fn test_config() -> (UiConfig, std::sync::mpsc::Receiver<WriterMsg>) {
        let (writer, writer_rx) = std::sync::mpsc::channel();
        let config = UiConfig {
            running: Arc::new(AtomicBool::new(true)),
            line_ending: LineEnding::Nl,
            writer,
            hex: false,
            show_ts: false,
            raw: false,
            echo: false,
            wrap: false,
            port_label: String::new(),
        };
        (config, writer_rx)
    }

    #[test]
    fn echo_shows_sent_lines_as_tx() {
        let (mut config, _writer_rx) = test_config();
        config.echo = true;
        let mut state = AppState::new(false, false, true, String::new(), "LF");
        handle_key_event(KeyEvent::from(KeyCode::Char('h')), &mut state, &config);
        handle_key_event(KeyEvent::from(KeyCode::Char('i')), &mut state, &config);
        handle_key_event(KeyEvent::from(KeyCode::Enter), &mut state, &config);
        assert_eq!(state.output_lines[0].kind, LineKind::Tx);
        assert_eq!(state.output_lines[0].text, "hi");
    }

    #[test]
    fn without_echo_sent_lines_are_not_shown() {
        let (config, _writer_rx) = test_config();
        let mut state = AppState::new(false, false, true, String::new(), "LF");
        handle_key_event(KeyEvent::from(KeyCode::Char('h')), &mut state, &config);
        handle_key_event(KeyEvent::from(KeyCode::Enter), &mut state, &config);
        assert!(state.output_lines.is_empty());
    }

    #[test]
    fn enter_while_disconnected_keeps_the_input() {
        let (config, writer_rx) = test_config();
        let mut state = AppState::new(false, false, true, String::new(), "LF");
        state.set_connected(false);
        handle_key_event(KeyEvent::from(KeyCode::Char('x')), &mut state, &config);
        handle_key_event(KeyEvent::from(KeyCode::Enter), &mut state, &config);
        assert_eq!(state.input_line, "x");
        assert!(writer_rx.try_recv().is_err());
        assert!(state.output_lines[0].text.contains("not connected"));

        state.set_connected(true);
        handle_key_event(KeyEvent::from(KeyCode::Enter), &mut state, &config);
        assert!(state.input_line.is_empty());
        assert!(matches!(writer_rx.try_recv(), Ok(WriterMsg::Data(b)) if b == b"x\n"));
    }

    #[test]
    fn ctrl_j_sends_like_enter() {
        let (config, writer_rx) = test_config();
        let mut state = AppState::new(false, false, true, String::new(), "LF");
        handle_key_event(KeyEvent::from(KeyCode::Char('a')), &mut state, &config);
        handle_key_event(
            KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL),
            &mut state,
            &config,
        );
        match writer_rx.try_recv() {
            Ok(WriterMsg::Data(bytes)) => assert_eq!(bytes, b"a\n"),
            _ => panic!("expected the line to be sent"),
        }
        assert!(state.input_line.is_empty());
    }

    #[test]
    fn paste_sends_complete_lines_and_keeps_the_rest() {
        let (config, writer_rx) = test_config();
        let mut state = AppState::new(false, false, true, String::new(), "LF");
        handle_input_event(
            Event::Paste("first\r\nsecond\nthird".to_string()),
            &mut state,
            &config,
        );
        let mut sent = Vec::new();
        while let Ok(WriterMsg::Data(bytes)) = writer_rx.try_recv() {
            sent.push(bytes);
        }
        assert_eq!(sent, vec![b"first\n".to_vec(), b"second\n".to_vec()]);
        assert_eq!(state.input_line, "third");
    }

    #[test]
    fn literal_mode_reports_keys_without_a_mapping() {
        let (config, writer_rx) = test_config();
        let mut state = AppState::new(false, false, true, String::new(), "LF");
        handle_key_event(
            KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL),
            &mut state,
            &config,
        );
        assert!(state.pending_literal);
        handle_key_event(KeyEvent::from(KeyCode::Char('x')), &mut state, &config);
        assert!(!state.pending_literal);
        assert!(writer_rx.try_recv().is_err(), "nothing must be sent");
        assert!(
            state.output_lines[0].text.contains("nothing sent"),
            "{:?}",
            state.output_lines
        );
    }

    #[test]
    fn disconnect_finishes_partial_output_before_notices() {
        for hex in [false, true] {
            let mut state = AppState::new(hex, false, true, String::new(), "LF");
            handle_serial_event(SerialEvent::Data(b"before".to_vec()), &mut state);
            handle_serial_event(SerialEvent::Disconnected("EOF".into()), &mut state);
            assert!(!state.connected);
            assert_eq!(state.assembler.partial_display(), None);
            if hex {
                assert!(state.output_lines[0].text.starts_with("62 65 66 6F 72 65"));
                assert!(state.output_lines[0].text.ends_with("|before|"));
            } else {
                assert_eq!(state.output_lines[0].text, "before");
            }
            assert_eq!(state.output_lines[0].kind, LineKind::Rx);
            assert!(state.output_lines[1].text.contains("device disconnected"));
            assert_eq!(state.output_lines[1].kind, LineKind::Notice);

            handle_serial_event(SerialEvent::Reconnected, &mut state);
            handle_serial_event(SerialEvent::Data(b"after\n".to_vec()), &mut state);
            assert!(state.connected);
            assert!(state.output_lines[2].text.contains("device reconnected"));
            if hex {
                let partial = state.assembler.partial_display().unwrap_or_default();
                assert!(partial.starts_with("61 66 74 65 72 0A"), "{partial}");
                assert!(partial.ends_with("|after.|"), "{partial}");
            } else {
                assert_eq!(state.output_lines[3].text, "after");
            }
        }
    }
}
