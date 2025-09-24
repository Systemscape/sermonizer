use clap::ValueEnum;
use std::io::BufWriter;
use std::sync::{Arc, Mutex as StdMutex, atomic::AtomicBool};

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
    pub tx_log: Option<Arc<StdMutex<BufWriter<std::fs::File>>>>,
    pub log_ts: bool,
}
