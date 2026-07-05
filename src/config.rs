use clap::ValueEnum;
use serialport::{ClearBuffer, DataBits, FlowControl, Parity, SerialPort, StopBits};
use std::sync::{Arc, atomic::AtomicBool};
use std::time::Duration;

use crate::serial_io::WriterMsg;

/// Everything needed to (re)open the serial port.
#[derive(Clone)]
pub struct PortSettings {
    pub name: String,
    pub baud: u32,
    pub data_bits: DataBits,
    pub parity: Parity,
    pub stop_bits: StopBits,
    pub flow_control: FlowControl,
}

impl PortSettings {
    pub fn open(&self) -> serialport::Result<Box<dyn SerialPort>> {
        let port = serialport::new(&self.name, self.baud)
            .data_bits(self.data_bits)
            .parity(self.parity)
            .stop_bits(self.stop_bits)
            .flow_control(self.flow_control)
            .timeout(Duration::from_millis(100))
            .open()?;

        // Drop any stale data buffered by the OS
        port.clear(ClearBuffer::All)?;
        Ok(port)
    }
}

/// Number of data bits per character
#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum DataBitsArg {
    #[value(name = "5")]
    Five,
    #[value(name = "6")]
    Six,
    #[value(name = "7")]
    Seven,
    #[value(name = "8")]
    Eight,
}

impl DataBitsArg {
    pub fn label(self) -> &'static str {
        match self {
            DataBitsArg::Five => "5",
            DataBitsArg::Six => "6",
            DataBitsArg::Seven => "7",
            DataBitsArg::Eight => "8",
        }
    }
}

impl From<DataBitsArg> for DataBits {
    fn from(value: DataBitsArg) -> Self {
        match value {
            DataBitsArg::Five => DataBits::Five,
            DataBitsArg::Six => DataBits::Six,
            DataBitsArg::Seven => DataBits::Seven,
            DataBitsArg::Eight => DataBits::Eight,
        }
    }
}

/// Parity checking mode
#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum ParityArg {
    None,
    Odd,
    Even,
}

impl ParityArg {
    pub fn label(self) -> &'static str {
        match self {
            ParityArg::None => "N",
            ParityArg::Odd => "O",
            ParityArg::Even => "E",
        }
    }
}

impl From<ParityArg> for Parity {
    fn from(value: ParityArg) -> Self {
        match value {
            ParityArg::None => Parity::None,
            ParityArg::Odd => Parity::Odd,
            ParityArg::Even => Parity::Even,
        }
    }
}

/// Number of stop bits
#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum StopBitsArg {
    #[value(name = "1")]
    One,
    #[value(name = "2")]
    Two,
}

impl StopBitsArg {
    pub fn label(self) -> &'static str {
        match self {
            StopBitsArg::One => "1",
            StopBitsArg::Two => "2",
        }
    }
}

impl From<StopBitsArg> for StopBits {
    fn from(value: StopBitsArg) -> Self {
        match value {
            StopBitsArg::One => StopBits::One,
            StopBitsArg::Two => StopBits::Two,
        }
    }
}

/// Flow control mode
#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum FlowControlArg {
    None,
    Software,
    Hardware,
}

impl FlowControlArg {
    pub fn label(self) -> &'static str {
        match self {
            FlowControlArg::None => "none",
            FlowControlArg::Software => "software",
            FlowControlArg::Hardware => "hardware",
        }
    }
}

impl From<FlowControlArg> for FlowControl {
    fn from(value: FlowControlArg) -> Self {
        match value {
            FlowControlArg::None => FlowControl::None,
            FlowControlArg::Software => FlowControl::Software,
            FlowControlArg::Hardware => FlowControl::Hardware,
        }
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
    pub hex: bool,
    pub show_ts: bool,
    pub port_label: String,
}
