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
const DISCONNECT_ACK_TIMEOUT: Duration = Duration::from_secs(1);

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
    /// Release the current handle and acknowledge once it is dropped: a stale
    /// descriptor on a vanished device can keep the OS from handing the same
    /// name to the re-plugged board, so the reopen must wait for the drop
    Disconnected(std::sync::mpsc::SyncSender<()>),
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
        // A missing write half on the first connection degrades to read-only
        let write_half = first_port
            .try_clone()
            .map_err(|e| {
                let _ = events.send(SerialEvent::Error(format!(
                    "cannot clone port handle, sending disabled: {e}"
                )));
            })
            .ok();
        let mut next = Some((first_port, write_half));
        let mut reconnected = false;
        while running.load(Ordering::SeqCst) {
            let Some((port, write_half)) = next.take() else {
                break;
            };
            if let Some(w) = write_half
                && writer.send(WriterMsg::NewPort(w)).is_err()
            {
                break;
            }
            // Announce only once the writer can use the new connection
            if reconnected {
                let _ = events.send(SerialEvent::Reconnected);
            }

            let reader = spawn_reader(port, running.clone(), events.clone(), rx_log.take());
            rx_log = reader.join().unwrap_or(None);
            if !running.load(Ordering::SeqCst) {
                break;
            }

            // Reader exited while we are still running: the device is gone.
            // Make the writer drop its handle, then poll until the port is back.
            let (ack_tx, ack_rx) = std::sync::mpsc::sync_channel(1);
            if writer.send(WriterMsg::Disconnected(ack_tx)).is_err() {
                break;
            }
            let _ = ack_rx.recv_timeout(DISCONNECT_ACK_TIMEOUT);
            next = wait_for_port(&settings, &running, &events).map(|(p, w)| (p, Some(w)));
            reconnected = true;
        }
    })
}

/// Poll until the port reopens with a usable write half, or until shutdown.
/// A port that reopens but cannot be cloned is dropped and retried: the
/// device is most likely still enumerating.
fn wait_for_port(
    settings: &PortSettings,
    running: &AtomicBool,
    events: &mpsc::UnboundedSender<SerialEvent>,
) -> Option<(Box<dyn SerialPort>, Box<dyn SerialPort>)> {
    let mut ticks = 0u32;
    let mut clone_warned = false;
    while running.load(Ordering::SeqCst) {
        std::thread::sleep(RECONNECT_POLL);
        ticks += 1;
        if !ticks.is_multiple_of(RECONNECT_RETRY_TICKS) {
            continue;
        }
        let Ok(port) = settings.open() else {
            continue;
        };
        match port.try_clone() {
            Ok(write_half) => return Some((port, write_half)),
            Err(e) if !clone_warned => {
                clone_warned = true;
                let _ = events.send(SerialEvent::Error(format!(
                    "port reopened but handle cannot be cloned, retrying: {e}"
                )));
            }
            Err(_) => {}
        }
    }
    None
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
                WriterMsg::Disconnected(ack) => {
                    port = None;
                    let _ = ack.send(());
                }
                WriterMsg::Data(bytes) => {
                    let Some(p) = port.as_mut() else {
                        let _ =
                            events.send(SerialEvent::Error("not connected, input dropped".into()));
                        continue;
                    };
                    if let Err(e) = p.write_all(&bytes) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use serialport::{DataBits, FlowControl, Parity, StopBits};

    fn settings_for(name: String) -> PortSettings {
        PortSettings {
            name,
            baud: 115_200,
            data_bits: DataBits::Eight,
            parity: Parity::None,
            stop_bits: StopBits::One,
            flow_control: FlowControl::None,
            dtr: None,
            rts: None,
        }
    }

    #[test]
    fn wait_for_port_gives_up_on_shutdown() {
        let settings = settings_for("/nonexistent/port".to_string());
        let (events, _rx) = mpsc::unbounded_channel();
        assert!(wait_for_port(&settings, &AtomicBool::new(false), &events).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn wait_for_port_returns_port_with_write_half() {
        let (master, slave) = serialport::TTYPort::pair().expect("pty pair");
        let settings = settings_for(slave.name().expect("pty slave has a path"));
        drop(slave);
        let (events, _rx) = mpsc::unbounded_channel();

        let reopened = wait_for_port(&settings, &AtomicBool::new(true), &events);
        assert!(reopened.is_some(), "pty slave must be reopenable");
        drop(master);
    }

    #[test]
    fn writer_reports_not_connected_after_disconnect() {
        let (writer_tx, writer_rx) = std::sync::mpsc::channel();
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        let handle = spawn_writer(writer_rx, event_tx, None);

        let (ack_tx, ack_rx) = std::sync::mpsc::sync_channel(1);
        writer_tx
            .send(WriterMsg::Disconnected(ack_tx))
            .expect("writer alive");
        ack_rx
            .recv_timeout(DISCONNECT_ACK_TIMEOUT)
            .expect("writer acknowledges the drop");
        writer_tx
            .send(WriterMsg::Data(b"hi".to_vec()))
            .expect("writer alive");
        drop(writer_tx);
        handle.join().expect("writer thread exits cleanly");

        match event_rx.try_recv() {
            Ok(SerialEvent::Error(msg)) => assert!(msg.contains("not connected"), "{msg}"),
            other => panic!("unexpected event: {other:?}"),
        }
    }
}
