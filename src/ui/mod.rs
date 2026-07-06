pub mod app_state;
pub mod rendering;

pub use app_state::AppState;
pub use rendering::draw_ui;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{Terminal, backend::Backend};
use std::sync::atomic::Ordering;
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
    let mut app_state = AppState::new();

    while ui_config.running.load(Ordering::SeqCst) && !app_state.should_quit {
        tokio::select! {
            // UI messages (like quit from Ctrl-C)
            msg = ui_rx.recv() => {
                if let Some(msg) = msg {
                    match msg {
                        UiMessage::Quit => {
                            app_state.quit();
                            break;
                        }
                    }
                }
            }

            // Serial events
            event = serial_rx.recv() => {
                if let Some(event) = event {
                    handle_serial_event(event, &mut app_state);
                }
            }

            // Keyboard input - async wrapper for crossterm events
            key_result = async {
                if event::poll(Duration::from_millis(0)).unwrap_or(false) {
                    event::read()
                } else {
                    tokio::time::sleep(Duration::from_millis(1)).await;
                    Err(std::io::Error::new(std::io::ErrorKind::WouldBlock, "no input"))
                }
            } => {
                if let Ok(Event::Key(k)) = key_result
                    && k.kind == KeyEventKind::Press {
                    handle_key_event(k, &mut app_state, &ui_config);
                }
            }
        }

        // Only render if state changed - major performance optimization
        if app_state.needs_render {
            terminal.draw(|f| draw_ui(f, &mut app_state))?;
            app_state.mark_rendered();
        }
    }

    ui_config.running.store(false, Ordering::SeqCst);
    Ok(())
}

fn handle_serial_event(event: SerialEvent, app_state: &mut AppState) {
    match event {
        SerialEvent::Data(bytes) => {
            app_state.add_output(String::from_utf8_lossy(&bytes).into_owned());
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
    match key.code {
        KeyCode::Char(c)
            if key.modifiers.contains(KeyModifiers::CONTROL) && (c == 'c' || c == 'd') =>
        {
            app_state.quit();
        }
        KeyCode::Esc => {
            app_state.quit();
        }
        KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            // Ctrl+A to re-enable auto-scroll
            app_state.enable_auto_scroll();
        }
        KeyCode::Char(c) => {
            app_state.update_input(c);
        }
        KeyCode::Enter => {
            handle_enter_key(app_state, ui_config);
        }
        KeyCode::Backspace => {
            app_state.backspace_input();
        }
        KeyCode::Up => {
            app_state.scroll_up();
        }
        KeyCode::Down => {
            app_state.scroll_down();
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

fn handle_enter_key(app_state: &mut AppState, ui_config: &UiConfig) {
    let input = app_state.clear_input();

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
