use anyhow::Result;
use chrono::Utc;
use serialport::SerialPort;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{Mutex, mpsc};

#[derive(Debug, Clone)]
pub enum SerialData {
    Received(String),
}

pub struct SerialReader {
    port: Arc<Mutex<Box<dyn SerialPort + Send>>>,
    running: Arc<AtomicBool>,
    sender: mpsc::UnboundedSender<SerialData>,
    hex_mode: bool,
    log_ts: bool,
    rx_log_writer: Option<Arc<std::sync::Mutex<std::io::BufWriter<std::fs::File>>>>,
    // No cached timestamp needed with chrono
    buffer: Vec<u8>, // Pre-allocated buffer
}

impl SerialReader {
    pub fn new(
        port: Arc<Mutex<Box<dyn SerialPort + Send>>>,
        running: Arc<AtomicBool>,
        sender: mpsc::UnboundedSender<SerialData>,
        hex_mode: bool,
        log_ts: bool,
        rx_log_writer: Option<Arc<std::sync::Mutex<std::io::BufWriter<std::fs::File>>>>,
    ) -> Self {
        Self {
            port,
            running,
            sender,
            hex_mode,
            log_ts,
            rx_log_writer,
            // No cached timestamp initialization needed
            buffer: vec![0u8; 4096], // Pre-allocate buffer to avoid allocations
        }
    }

    pub async fn run(mut self) {
        while self.running.load(Ordering::SeqCst) {
            let n = {
                let mut guard = self.port.lock().await;
                match guard.read(&mut self.buffer) {
                    Ok(n) => n,
                    Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => 0,
                    Err(_) => break,
                }
            };

            if n > 0 {
                // Make a copy to avoid borrow checker issues
                let bytes = self.buffer[..n].to_vec();
                self.process_received_data(&bytes).await;
            } else {
                // Small async yield to prevent busy waiting
                tokio::task::yield_now().await;
            }
        }
    }

    async fn process_received_data(&mut self, bytes: &[u8]) {
        // Format the data - optimized to avoid multiple allocations
        let display_text = if self.hex_mode {
            self.format_hex_data(bytes)
        } else {
            self.format_text_data(bytes)
        };

        // Send to UI
        let _ = self.sender.send(SerialData::Received(display_text));

        // Write to RX log file if configured
        self.write_to_log(bytes).await;
    }

    fn format_hex_data(&mut self, bytes: &[u8]) -> String {
        let capacity = if self.log_ts { 32 } else { 0 } + bytes.len() * 3; // Estimate capacity
        let mut hex_str = String::with_capacity(capacity);

        if self.log_ts {
            hex_str.push_str("[");
            hex_str.push_str(&Utc::now().format("%Y-%m-%d %H:%M:%S%.3f").to_string());
            hex_str.push_str("] ");
        }

        // Optimize hex formatting with pre-allocated string
        for (i, b) in bytes.iter().enumerate() {
            if i > 0 {
                hex_str.push(' ');
            }
            hex_str.push_str(&format!("{:02X}", b));
        }

        hex_str
    }

    fn format_text_data(&mut self, bytes: &[u8]) -> String {
        let capacity = if self.log_ts { 32 } else { 0 } + bytes.len();
        let mut text = String::with_capacity(capacity);

        if self.log_ts {
            text.push_str("[");
            text.push_str(&Utc::now().format("%Y-%m-%d %H:%M:%S%.3f").to_string());
            text.push_str("] ");
        }

        // Use from_utf8_lossy but avoid extra allocations where possible
        text.push_str(&String::from_utf8_lossy(bytes));
        text
    }

    async fn write_to_log(&mut self, bytes: &[u8]) {
        if let Some(w) = &self.rx_log_writer {
            if let Ok(mut lw) = w.lock() {
                use std::io::Write;

                if self.log_ts {
                    let _ = write!(lw, "[{}] ", Utc::now().format("%Y-%m-%d %H:%M:%S%.3f"));
                }

                if self.hex_mode {
                    for (i, b) in bytes.iter().enumerate() {
                        let separator = if i + 1 == bytes.len() { "" } else { " " };
                        let _ = write!(lw, "{:02X}{}", b, separator);
                    }
                    let _ = writeln!(lw);
                } else {
                    let _ = lw.write_all(bytes);
                }

                let _ = lw.flush();
            }
        }
    }
}

pub async fn write_bytes_async(
    port: &Arc<Mutex<Box<dyn SerialPort + Send>>>,
    bytes: &[u8],
) -> Result<()> {
    let mut guard = port.lock().await;
    guard.write_all(bytes)?;
    guard.flush()?;
    Ok(())
}
