pub mod app_state;
pub mod line_assembler;
pub mod rendering;

pub use app_state::AppState;
pub use rendering::draw_ui;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{Terminal, backend::Backend};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::mpsc;

use crate::config::UiConfig;
use crate::serial_io::{SerialEvent, WriterMsg};

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
    let mut app_state = AppState::new(ui_config.hex, ui_config.show_ts);
    let (mut input_rx, input_handle) = spawn_input_thread(ui_config.running.clone());

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
                    Some(Event::Key(k)) if k.kind == KeyEventKind::Press => {
                        handle_key_event(k, &mut app_state, &ui_config);
                    }
                    Some(Event::Resize(_, _)) => app_state.needs_render = true,
                    Some(_) => {}
                    None => app_state.quit(),
                }
            }
        }
    }

    ui_config.running.store(false, Ordering::SeqCst);
    let _ = input_handle.join();
    Ok(())
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

fn handle_serial_event(event: SerialEvent, app_state: &mut AppState) {
    match event {
        SerialEvent::Data(bytes) => {
            app_state.add_data(&bytes);
        }
        SerialEvent::Error(message) => {
            app_state.add_notice(format!("[sermonizer] {message}"));
        }
        SerialEvent::Disconnected(reason) => {
            app_state.add_notice(format!(
                "[sermonizer] device disconnected: {reason} — reconnecting (Ctrl+C to quit)"
            ));
        }
        SerialEvent::Reconnected => {
            app_state.add_notice("[sermonizer] device reconnected".to_string());
        }
    }
}

fn handle_key_event(
    key: crossterm::event::KeyEvent,
    app_state: &mut AppState,
    ui_config: &UiConfig,
) {
    // Ctrl+V arms literal mode: the next key is sent as a raw control byte
    if app_state.pending_literal {
        app_state.pending_literal = false;
        app_state.needs_render = true;
        if let Some(byte) = literal_byte(key) {
            if ui_config.writer.send(WriterMsg::Data(vec![byte])).is_err() {
                app_state.add_notice("[sermonizer] writer stopped, input dropped".to_string());
            } else {
                app_state.add_notice(format!("[sermonizer] sent control byte 0x{byte:02X}"));
            }
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
        KeyCode::Char('v') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app_state.pending_literal = true;
            app_state.needs_render = true;
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
            handle_enter_key(app_state, ui_config);
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
            app_state.scroll_page_up(10);
        }
        KeyCode::PageDown => {
            app_state.scroll_page_down(10);
        }
        KeyCode::Home => {
            app_state.scroll_to_home();
        }
        KeyCode::End => {
            app_state.scroll_to_bottom();
        }
        _ => {}
    }
}

/// Map a key pressed after Ctrl+V to the raw byte it should send.
fn literal_byte(key: crossterm::event::KeyEvent) -> Option<u8> {
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

fn handle_enter_key(app_state: &mut AppState, ui_config: &UiConfig) {
    let input = app_state.clear_input();
    app_state.push_history(input.clone());

    // Send input and line ending as a single write
    let mut bytes = input.into_bytes();
    bytes.extend_from_slice(ui_config.line_ending.bytes());
    if bytes.is_empty() {
        return;
    }

    if ui_config.writer.send(WriterMsg::Data(bytes)).is_err() {
        app_state.add_notice("[sermonizer] writer stopped, input dropped".to_string());
    }
}
