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
    pub dtr: Option<bool>,
    pub rts: Option<bool>,
}

impl PortSettings {
    pub fn open(&self) -> serialport::Result<Box<dyn SerialPort>> {
        let mut port = serialport::new(&self.name, self.baud)
            .data_bits(self.data_bits)
            .parity(self.parity)
            .stop_bits(self.stop_bits)
            .flow_control(self.flow_control)
            .timeout(Duration::from_millis(100))
            .open()?;

        // Control lines matter for boards that wire DTR/RTS to reset or boot
        // pins (e.g. ESP32, Arduino); leave them untouched unless requested
        if let Some(dtr) = self.dtr {
            port.write_data_terminal_ready(dtr)?;
        }
        if let Some(rts) = self.rts {
            port.write_request_to_send(rts)?;
        }

        // Drop any stale data buffered by the OS
        port.clear(ClearBuffer::All)?;
        Ok(port)
    }
}

/// Explicit level for a control line
#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum Toggle {
    On,
    Off,
}

impl Toggle {
    pub fn as_bool(self) -> bool {
        matches!(self, Toggle::On)
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
    #[value(alias = "lf")]
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

/// Longest port name shown in the status bar before it is cut from the left
const PORT_LABEL_MAX: usize = 28;

/// Compact status-bar label: the port's basename (by-id paths and Windows
/// COM names both survive), cut from the left when still too long, followed
/// by baud and framing.
pub fn port_label(port_name: &str, baud: u32, framing: &str) -> String {
    let name = std::path::Path::new(port_name)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(port_name);
    let chars = name.chars().count();
    let name = if chars > PORT_LABEL_MAX {
        let tail: String = name.chars().skip(chars - (PORT_LABEL_MAX - 3)).collect();
        format!("...{tail}")
    } else {
        name.to_string()
    };
    format!("{name} {baud} {framing}")
}

pub struct UiConfig {
    pub running: Arc<AtomicBool>,
    pub line_ending: LineEnding,
    pub writer: std::sync::mpsc::Sender<WriterMsg>,
    pub hex: bool,
    pub show_ts: bool,
    /// Keep ANSI escape sequences in the display instead of stripping them
    pub raw: bool,
    /// Show transmitted lines in the output
    pub echo: bool,
    /// Start with long lines wrapped instead of clipped
    pub wrap: bool,
    pub port_label: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_label_uses_the_basename_with_baud_and_framing() {
        assert_eq!(
            port_label("/dev/ttyUSB0", 115_200, "8N1"),
            "ttyUSB0 115200 8N1"
        );
        assert_eq!(port_label("COM3", 9600, "7E1"), "COM3 9600 7E1");
    }

    #[test]
    fn port_label_cuts_long_names_from_the_left() {
        let name = "/dev/serial/by-id/usb-Silicon_Labs_CP2102_USB_to_UART_Bridge_Controller_0001-if00-port0";
        let label = port_label(name, 115_200, "8N1");
        assert_eq!(label, "...ontroller_0001-if00-port0 115200 8N1");
        assert!(label.len() <= PORT_LABEL_MAX + " 115200 8N1".len());
    }
}
