use serialport::SerialPort;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;
use tokio::sync::mpsc;

use crate::config::PortSettings;
use crate::logging::LogSink;

const READ_BUF_SIZE: usize = 4096;
const RECONNECT_POLL: Duration = Duration::from_millis(100);
const RECONNECT_RETRY_TICKS: u32 = 5;

/// Events sent from the serial threads to the UI.
#[derive(Debug, Clone)]
pub enum SerialEvent {
    Data(Vec<u8>),
    Error(String),
    Disconnected(String),
    Reconnected,
}

/// Messages consumed by the writer thread.
pub enum WriterMsg {
    Data(Vec<u8>),
    NewPort(Box<dyn SerialPort>),
}

/// Supervises the reader thread: when the device disappears it keeps trying
/// to reopen the port and resumes reading once it comes back.
pub fn spawn_supervisor(
    first_port: Box<dyn SerialPort>,
    settings: PortSettings,
    running: Arc<AtomicBool>,
    events: mpsc::UnboundedSender<SerialEvent>,
    writer: std::sync::mpsc::Sender<WriterMsg>,
    mut rx_log: Option<LogSink>,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let mut port = Some(first_port);
        while running.load(Ordering::SeqCst) {
            let Some(p) = port.take() else { break };

            // Hand the writer its own handle to the fresh connection
            match p.try_clone() {
                Ok(w) => {
                    if writer.send(WriterMsg::NewPort(w)).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    let _ = events.send(SerialEvent::Error(format!(
                        "cannot clone port handle, sending disabled: {e}"
                    )));
                }
            }

            let reader = spawn_reader(p, running.clone(), events.clone(), rx_log.take());
            rx_log = reader.join().unwrap_or(None);
            if !running.load(Ordering::SeqCst) {
                break;
            }

            // Reader exited while we are still running: the device is gone.
            // Poll until the port can be reopened.
            let mut ticks = 0u32;
            while running.load(Ordering::SeqCst) && port.is_none() {
                std::thread::sleep(RECONNECT_POLL);
                ticks += 1;
                if !ticks.is_multiple_of(RECONNECT_RETRY_TICKS) {
                    continue;
                }
                if let Ok(p) = settings.open() {
                    let _ = events.send(SerialEvent::Reconnected);
                    port = Some(p);
                }
            }
        }
    })
}

/// Reads from the port until shutdown or a fatal error. Returns the RX log
/// sink so a future connection can keep appending to it.
fn spawn_reader(
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
