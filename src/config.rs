use clap::ValueEnum;
use serialport::{ClearBuffer, SerialPort};
use std::sync::{Arc, atomic::AtomicBool};
use std::time::Duration;

use crate::serial_io::WriterMsg;

/// Everything needed to (re)open the serial port.
#[derive(Clone)]
pub struct PortSettings {
    pub name: String,
    pub baud: u32,
}

impl PortSettings {
    pub fn open(&self) -> serialport::Result<Box<dyn SerialPort>> {
        let port = serialport::new(&self.name, self.baud)
            .timeout(Duration::from_millis(100))
            .open()?;

        // Drop any stale data buffered by the OS
        port.clear(ClearBuffer::All)?;
        Ok(port)
    }
}

/// Which line ending to send when you press Enter
#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum LineEnding {
    /// Send nothing extra (no line ending)
    None,
    /// Send '\n' (LF)
    Nl,
    /// Send '\r' (CR)
    Cr,
    /// Send "\r\n" (CRLF)
    Crlf,
}

impl LineEnding {
    pub fn describe(self) -> &'static str {
        match self {
            LineEnding::None => "none",
            LineEnding::Nl => "LF (\\n)",
            LineEnding::Cr => "CR (\\r)",
            LineEnding::Crlf => "CRLF (\\r\\n)",
        }
    }

    pub fn bytes(self) -> &'static [u8] {
        match self {
            LineEnding::None => b"",
            LineEnding::Nl => b"\n",
            LineEnding::Cr => b"\r",
            LineEnding::Crlf => b"\r\n",
        }
    }
}

pub struct UiConfig {
    pub running: Arc<AtomicBool>,
    pub line_ending: LineEnding,
    pub writer: std::sync::mpsc::Sender<WriterMsg>,
}
