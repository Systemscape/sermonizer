pub mod app_state;
pub mod rendering;

pub use app_state::AppState;
pub use rendering::draw_ui;

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
};
use ratatui::{backend::Backend, Terminal};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

use crate::config::UiConfig;
use crate::serial_io::{write_bytes_async, SerialData};
use crate::time_utils::CachedTimestamp;

#[derive(Debug)]
pub enum UiMessage {
    Quit,
}

pub async fn run_ui<B: Backend>(
    terminal: &mut Terminal<B>,
    mut ui_rx: mpsc::UnboundedReceiver<UiMessage>,
    mut serial_rx: mpsc::UnboundedReceiver<SerialData>,
    port: Arc<tokio::sync::Mutex<Box<dyn serialport::SerialPort + Send>>>,
    ui_config: UiConfig,
) -> Result<()> {
    let mut app_state = AppState::new();
    let mut cached_timestamp = CachedTimestamp::new();

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

            // Serial data
            data = serial_rx.recv() => {
                if let Some(data) = data {
                    match data {
                        SerialData::Received(line) => {
                            app_state.add_output(line);
                        }
                    }
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
                    handle_key_event(k, &mut app_state, &port, &ui_config, &mut cached_timestamp).await?;
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

async fn handle_key_event(
    key: crossterm::event::KeyEvent,
    app_state: &mut AppState,
    port: &Arc<tokio::sync::Mutex<Box<dyn serialport::SerialPort + Send>>>,
    ui_config: &UiConfig,
    cached_timestamp: &mut CachedTimestamp,
) -> Result<()> {
    match key.code {
        KeyCode::Char(c) if key.modifiers.contains(KeyModifiers::CONTROL) && (c == 'c' || c == 'd') => {
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
            handle_enter_key(app_state, port, ui_config, cached_timestamp).await?;
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
    Ok(())
}

async fn handle_enter_key(
    app_state: &mut AppState,
    port: &Arc<tokio::sync::Mutex<Box<dyn serialport::SerialPort + Send>>>,
    ui_config: &UiConfig,
    cached_timestamp: &mut CachedTimestamp,
) -> Result<()> {
    let input = app_state.clear_input();

    // Send the complete line to serial port
    if !input.is_empty() {
        write_bytes_async(port, input.as_bytes()).await?;
        if let Some(w) = &ui_config.tx_log {
            if let Ok(mut lw) = w.lock() {
                use std::io::Write;
                if ui_config.log_ts {
                    let _ = write!(lw, "[{}] ", cached_timestamp.now_rfc3339());
                }
                let _ = lw.write_all(input.as_bytes());
                let _ = lw.flush();
            }
        }
    }

    // Send line ending
    let end = ui_config.line_ending.bytes();
    if !end.is_empty() {
        write_bytes_async(port, end).await?;
        if let Some(w) = &ui_config.tx_log {
            if let Ok(mut lw) = w.lock() {
                use std::io::Write;
                if ui_config.log_ts && input.is_empty() {
                    let _ = write!(lw, "[{}] ", cached_timestamp.now_rfc3339());
                }
                let _ = lw.write_all(end);
                let _ = lw.flush();
            }
        }
    }

    Ok(())
}