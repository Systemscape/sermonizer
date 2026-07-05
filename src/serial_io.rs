use serialport::SerialPort;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use tokio::sync::mpsc;

use crate::logging::LogSink;

const READ_BUF_SIZE: usize = 4096;

/// Events sent from the serial threads to the UI.
#[derive(Debug, Clone)]
pub enum SerialEvent {
    Data(Vec<u8>),
    Error(String),
    Disconnected(String),
}

/// Messages consumed by the writer thread.
pub enum WriterMsg {
    Data(Vec<u8>),
    NewPort(Box<dyn SerialPort>),
}

/// Reads from the port until shutdown or a fatal error. Returns the RX log
/// sink so a future connection can keep appending to it.
pub fn spawn_reader(
    mut port: Box<dyn SerialPort>,
    running: Arc<AtomicBool>,
    events: mpsc::UnboundedSender<SerialEvent>,
    mut rx_log: Option<LogSink>,
) -> JoinHandle<Option<LogSink>> {
    std::thread::spawn(move || {
        let mut buf = [0u8; READ_BUF_SIZE];
        while running.load(Ordering::SeqCst) {
            match port.read(&mut buf) {
                Ok(0) => {
                    let _ = events.send(SerialEvent::Disconnected("port returned EOF".into()));
                    break;
                }
                Ok(n) => {
                    if let Some(log) = rx_log.as_mut()
                        && let Err(e) = log.write_chunk(&buf[..n])
                    {
                        let _ = events.send(SerialEvent::Error(format!(
                            "RX log write failed, logging disabled: {e}"
                        )));
                        rx_log = None;
                    }
                    if events.send(SerialEvent::Data(buf[..n].to_vec())).is_err() {
                        break;
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => {}
                Err(e) => {
                    let _ = events.send(SerialEvent::Disconnected(e.to_string()));
                    break;
                }
            }
        }
        rx_log
    })
}

/// Owns the write half of the port. Write errors are reported to the UI
/// instead of terminating the application.
pub fn spawn_writer(
    messages: std::sync::mpsc::Receiver<WriterMsg>,
    events: mpsc::UnboundedSender<SerialEvent>,
    mut tx_log: Option<LogSink>,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let mut port: Option<Box<dyn SerialPort>> = None;
        while let Ok(msg) = messages.recv() {
            match msg {
                WriterMsg::NewPort(p) => port = Some(p),
                WriterMsg::Data(bytes) => {
                    let Some(p) = port.as_mut() else {
                        let _ =
                            events.send(SerialEvent::Error("not connected, input dropped".into()));
                        continue;
                    };
                    if let Err(e) = p.write_all(&bytes).and_then(|_| p.flush()) {
                        let _ = events.send(SerialEvent::Error(format!("write failed: {e}")));
                        continue;
                    }
                    if let Some(log) = tx_log.as_mut()
                        && let Err(e) = log.write_chunk(&bytes)
                    {
                        let _ = events.send(SerialEvent::Error(format!(
                            "TX log write failed, logging disabled: {e}"
                        )));
                        tx_log = None;
                    }
                }
            }
        }
    })
}
